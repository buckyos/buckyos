//! 安装任务 runner：dispatch 与恢复（P5.2）。
//!
//! 三条路径职责分明，全部收敛到同一个 `InstallEngine::run_task`（幂等 +
//! 进程内去重），且每次执行前都重新读取 TaskManager 真相，不信任何消息 /
//! 事件 payload 的完整性：
//! - MsgQueue：持久 dispatch 记录（不丢）。RPC 创建任务后 post `task_id`
//!   调度引用；consumer fetch(auto_commit=false) -> run -> 到达安全边界
//!   （任务离开 Pending/Running 空转态）后 commit_ack；
//! - task-ready KEvent：低延迟唤醒（只是加速通道，事件丢了也不影响正确性）；
//! - 启动扫描 + 低频 sweep：修复 queue/event 的一切遗漏。

use crate::app_install_engine::InstallEngine;
use buckyos_api::msg_queue::{Message, SubPosition};
use buckyos_api::{get_buckyos_api_runtime, TaskStatus, APP_INSTALL_RUNNER};
use log::{info, warn};
use std::sync::Arc;
use std::time::Duration;

const DISPATCH_QUEUE_NAME: &str = "app_install_dispatch";
const DISPATCH_SUB_ID: &str = "app_install_runner_main";
const SWEEP_INTERVAL_SECS: u64 = 60;
const KEVENT_PULL_TIMEOUT_MS: u64 = 30_000;

pub struct InstallRunner {
    engine: Arc<InstallEngine>,
}

impl InstallRunner {
    pub fn new(engine: Arc<InstallEngine>) -> Arc<Self> {
        Arc::new(Self { engine })
    }

    /// 启动全部 runner 循环（服务启动时调用一次）。
    pub fn start(self: &Arc<Self>) {
        let runner = self.clone();
        tokio::spawn(async move {
            runner.startup_scan().await;
        });

        let runner = self.clone();
        tokio::spawn(async move {
            runner.msg_queue_loop().await;
        });

        let runner = self.clone();
        tokio::spawn(async move {
            runner.kevent_loop().await;
        });

        let runner = self.clone();
        tokio::spawn(async move {
            runner.sweep_loop().await;
        });
    }

    /// RPC 侧调度入口：持久化 dispatch 引用 + 立即本地执行。
    pub async fn dispatch(self: &Arc<Self>, task_id: i64) {
        if let Err(err) = self.post_dispatch_message(task_id).await {
            // 消息投递失败不影响正确性（扫描兜底），但要留痕。
            warn!("post install dispatch for task {task_id} failed: {err}");
        }
        self.spawn_run(task_id);
    }

    pub fn spawn_run(self: &Arc<Self>, task_id: i64) {
        let runner = self.clone();
        tokio::spawn(async move {
            let _ = runner.engine.run_task(task_id).await;
        });
    }

    async fn post_dispatch_message(&self, task_id: i64) -> Result<(), String> {
        let runtime = get_buckyos_api_runtime().map_err(|err| err.to_string())?;
        let client = runtime
            .get_msg_queue_client()
            .await
            .map_err(|err| err.to_string())?;
        let queue_urn = self.ensure_queue().await?;
        let payload = serde_json::json!({ "task_id": task_id });
        client
            .post_message(
                queue_urn.as_str(),
                Message::new(payload.to_string().into_bytes()),
            )
            .await
            .map_err(|err| err.to_string())?;
        Ok(())
    }

    async fn ensure_queue(&self) -> Result<String, String> {
        let runtime = get_buckyos_api_runtime().map_err(|err| err.to_string())?;
        let client = runtime
            .get_msg_queue_client()
            .await
            .map_err(|err| err.to_string())?;
        let (appid, owner) = runner_identity(&runtime);
        let urn = buckyos_api::msg_queue::calc_queue_urn(
            appid.as_str(),
            owner.as_str(),
            DISPATCH_QUEUE_NAME,
        );
        // create 幂等失败可容忍（已存在）。
        let _ = client
            .create_queue(
                Some(DISPATCH_QUEUE_NAME),
                appid.as_str(),
                owner.as_str(),
                Default::default(),
            )
            .await;
        Ok(urn)
    }

    /// 启动恢复：TaskManager 真相扫描，Pending/Running 恢复执行；
    /// WaitingForApproval 等确认、Paused 等 retry，都不动。
    async fn startup_scan(self: &Arc<Self>) {
        match self.engine.store().list_active().await {
            Ok(tasks) => {
                for task in tasks {
                    match task.status {
                        TaskStatus::Pending | TaskStatus::Running => {
                            info!("recover install task {} ({:?})", task.id, task.status);
                            self.spawn_run(task.id);
                        }
                        _ => {}
                    }
                }
            }
            Err(err) => warn!("startup install task scan failed: {err}"),
        }
    }

    /// MsgQueue consumer：持久 dispatch 不丢；ack 只在任务离开待执行态后。
    async fn msg_queue_loop(self: &Arc<Self>) {
        loop {
            if let Err(err) = self.msg_queue_round().await {
                warn!("install dispatch queue round failed: {err}");
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        }
    }

    async fn msg_queue_round(self: &Arc<Self>) -> Result<(), String> {
        let runtime = get_buckyos_api_runtime().map_err(|err| err.to_string())?;
        let client = runtime
            .get_msg_queue_client()
            .await
            .map_err(|err| err.to_string())?;
        let queue_urn = self.ensure_queue().await?;
        let (_appid, owner) = runner_identity(&runtime);

        let sub_id = client
            .subscribe(
                queue_urn.as_str(),
                owner.as_str(),
                "control-panel",
                Some(DISPATCH_SUB_ID.to_string()),
                SubPosition::Earliest,
            )
            .await
            .map_err(|err| err.to_string())?;

        loop {
            let messages = client
                .fetch_messages(sub_id.as_str(), 16, false)
                .await
                .map_err(|err| err.to_string())?;
            if messages.is_empty() {
                // 空队列：慢轮询（kevent/sweep 负责低延迟与兜底）。
                tokio::time::sleep(Duration::from_secs(15)).await;
                continue;
            }
            for message in messages {
                let task_id = serde_json::from_slice::<serde_json::Value>(&message.payload)
                    .ok()
                    .and_then(|value| value.get("task_id").and_then(|id| id.as_i64()));
                if let Some(task_id) = task_id {
                    // 每次执行都以 TaskManager 为真相（payload 只是引用）。
                    let _ = self.engine.run_task(task_id).await;
                }
                // 安全边界：run_task 返回时任务必然处于
                // 终态 / WaitingForApproval / Paused / 其它执行体持有，
                // 后续推进由 confirm/retry/sweep 负责，可以 ack。
                client
                    .commit_ack(sub_id.as_str(), message.index)
                    .await
                    .map_err(|err| err.to_string())?;
            }
        }
    }

    /// task-ready KEvent：低延迟唤醒；唤醒后从权威源刷新（不解析 payload 细节）。
    async fn kevent_loop(self: &Arc<Self>) {
        loop {
            match self.kevent_round().await {
                Ok(()) => {}
                Err(err) => {
                    warn!("install runner kevent loop degraded: {err}");
                    tokio::time::sleep(Duration::from_secs(30)).await;
                }
            }
        }
    }

    async fn kevent_round(self: &Arc<Self>) -> Result<(), String> {
        let runtime = get_buckyos_api_runtime().map_err(|err| err.to_string())?;
        let kevent = runtime
            .get_kevent_client()
            .await
            .map_err(|err| err.to_string())?;
        let topic = format!("/task_mgr/runner/{APP_INSTALL_RUNNER}/task_ready");
        let reader = kevent
            .create_event_reader(vec![topic])
            .await
            .map_err(|err| err.to_string())?;
        // 订阅后立即扫一次，关闭订阅前事件已发布的竞态窗口。
        self.scan_pending().await;
        loop {
            match reader.pull_event(Some(KEVENT_PULL_TIMEOUT_MS)).await {
                Ok(Some(_event)) => {
                    // 事件只当加速信号：回权威源取 Pending 列表。
                    self.scan_pending().await;
                }
                Ok(None) => {
                    // timeout sweep：kevent 只是加速通道，超时同样回源。
                    self.scan_pending().await;
                }
                Err(err) => return Err(err.to_string()),
            }
        }
    }

    async fn scan_pending(self: &Arc<Self>) {
        match self.engine.store().list_active().await {
            Ok(tasks) => {
                for task in tasks {
                    if matches!(task.status, TaskStatus::Pending) {
                        self.spawn_run(task.id);
                    }
                }
            }
            Err(err) => warn!("scan pending install tasks failed: {err}"),
        }
    }

    /// 低频 sweep：修复一切遗漏（含 Running 态僵尸——进程内守卫保证不会
    /// 与在跑的执行体打架；真正跑着的任务 run_task 会因守卫直接返回）。
    async fn sweep_loop(self: &Arc<Self>) {
        loop {
            tokio::time::sleep(Duration::from_secs(SWEEP_INTERVAL_SECS)).await;
            match self.engine.store().list_active().await {
                Ok(tasks) => {
                    for task in tasks {
                        if matches!(task.status, TaskStatus::Pending | TaskStatus::Running) {
                            self.spawn_run(task.id);
                        }
                    }
                }
                Err(err) => warn!("install sweep failed: {err}"),
            }
        }
    }
}

fn runner_identity(runtime: &buckyos_api::BuckyOSRuntime) -> (String, String) {
    let owner = runtime
        .get_owner_user_id()
        .unwrap_or_else(|| "root".to_string());
    ("control-panel".to_string(), owner)
}
