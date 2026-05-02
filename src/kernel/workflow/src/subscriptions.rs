//! 订阅 task_manager 的 `/task_mgr/{run_id}` 子树事件，把用户在 TaskMgr UI 上的
//! `human_action`（写在 TaskData 里）回灌到 orchestrator 的 [`apply_task_data`]。
//!
//! 设计取舍：
//! - **单 EventReader + 动态订阅集**：每个 run 一条 `/task_mgr/{run_id}`
//!   pattern。task_manager 把整棵 task 树（root + descendants）扇出到 root_id
//!   channel（[task_manager/server.rs §publish_task_changed_event]），所以一个
//!   pattern 就能拿到这个 run 下所有 step / shard / thunk 的事件。
//! - **bootstrap pattern**：[`KEventClient::create_event_reader`] 不接受空
//!   pattern 列表，所以服务启动时先用一个永远不会被 publish 的 sentinel
//!   pattern 占位，之后 `watch_run` 再 add_patterns 进去。这样 dispatch loop
//!   可以在第一个 run 之前就跑起来。
//! - **过滤**：只处理 `change_kind == "data"` 的事件。Status / Progress 事件
//!   不携带 `human_action`，绕开节省一次 lock + apply_task_data 的空跑。
//! - **大 data 回拉**：task_manager 的 inline 阈值只有 ~1300 字节
//!   （[task_manager/server.rs §TASK_EVENT_DATA_INLINE_LIMIT_BYTES]），整段
//!   `task_data` 一过阈值就被裁成 `data_omitted=true`。step 输出 / 历史只要
//!   略大就会触发，这时按 `task_id` 调一次 `TaskManagerClient::get_task` 拉
//!   全量 data 再喂给 `apply_task_data`，否则 human_action 会被静默丢。
//! - **回环安全**：workflow 自己 `update_task` 写出的 data 不带 `human_action`，
//!   apply_task_data 会直接返回 `Ok(vec![])`，所以即使触发了反向事件（含回拉
//!   后的）也是空跑。
//! - **终态清理**：apply_task_data 后如果 run 进了终态，就 remove_patterns
//!   把这条订阅摘掉。
//! - **timeout sweep 兜底**：kevent 是加速通道、不是真理来源
//!   （rate-limit、SHM 槽位覆盖、reader 重连、`data_omitted` 回拉失败等都会
//!   静默丢事件），所以 dispatch loop 的 `pull_event` 必须带超时，超时落到
//!   `sweep_watched_runs`：对每个 watched run 把当前还停在 `WaitingHuman` 的
//!   节点对应的 step task 直接拉一遍，再走同一个 `apply_task_data` 入口。
//!   sweep 与事件路径走同一条状态机入口，已经处理过的人工动作再被回放也是
//!   `NodeNotWaitingHuman` 错误（debug 一行了事），保证幂等。
//!
//! [`apply_task_data`]: crate::WorkflowOrchestrator::apply_task_data

use buckyos_api::{
    EventReader, KEventClient, NodeRunState, RunStatus, Task, TaskFilter, TaskManagerClient,
};
use log::{debug, info, warn};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::server::ServiceOrchestrator;
use crate::state::{DefinitionStore, RunStore};

/// 服务启动时占位用的 pattern。永远不会有 publisher 写到这条 channel。
const BOOTSTRAP_PATTERN: &str = "/workflow_service/__bootstrap__";

/// `pull_event` 的等待上限。到了上限就跑一次 sweep 兜底。挑 10s 是为了在
/// kevent 路径正常时几乎察觉不到额外延迟，丢事件时最坏也只多 ~10s 延迟。
const SWEEP_INTERVAL_MS: u64 = 10_000;

/// workflow 服务给每个 step 在 task_manager 里建的 task 类型，sweep 时按这个
/// 过滤减少扫表成本。
const STEP_TASK_TYPE: &str = "workflow/step";

fn run_pattern(run_id: &str) -> String {
    format!("/task_mgr/{}", run_id)
}

fn is_terminal(status: RunStatus) -> bool {
    matches!(
        status,
        RunStatus::Completed
            | RunStatus::Failed
            | RunStatus::Aborted
            | RunStatus::BudgetExhausted
    )
}

pub struct RunSubscriptionManager {
    kevent_client: KEventClient,
    runs: Arc<RunStore>,
    definitions: Arc<DefinitionStore>,
    orchestrator: Arc<ServiceOrchestrator>,
    /// 用来在事件 `data_omitted=true` 时按 task_id 回拉全量 task_data。
    /// 服务启动时拿不到 task_manager 客户端则为 None——此时大 data 事件无法
    /// 被回灌（与"task_manager 不可用整体降级"保持一致）。
    task_mgr_client: Option<Arc<TaskManagerClient>>,
    state: Mutex<ManagerState>,
}

struct ManagerState {
    reader: Option<Arc<EventReader>>,
    /// 当前已经订阅的 run_id（不含 bootstrap）。
    watched_runs: HashSet<String>,
}

impl RunSubscriptionManager {
    pub fn new(
        kevent_client: KEventClient,
        runs: Arc<RunStore>,
        definitions: Arc<DefinitionStore>,
        orchestrator: Arc<ServiceOrchestrator>,
        task_mgr_client: Option<Arc<TaskManagerClient>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            kevent_client,
            runs,
            definitions,
            orchestrator,
            task_mgr_client,
            state: Mutex::new(ManagerState {
                reader: None,
                watched_runs: HashSet::new(),
            }),
        })
    }

    /// 启动 reader + dispatch loop。失败说明 daemon bridge 没就绪——记 warn 但
    /// 不阻塞 service 启动（service 还可以处理 RPC，只是收不到 task_mgr 反馈）。
    pub async fn start(self: &Arc<Self>) {
        let reader = match self
            .kevent_client
            .create_event_reader(vec![BOOTSTRAP_PATTERN.to_string()])
            .await
        {
            Ok(r) => Arc::new(r),
            Err(err) => {
                warn!(
                    "workflow.subscriptions: create_event_reader failed, task_mgr feedback disabled: {:?}",
                    err
                );
                return;
            }
        };
        info!(
            "workflow.subscriptions: event reader created reader_id={}",
            reader.reader_id()
        );
        {
            let mut guard = self.state.lock().await;
            guard.reader = Some(reader.clone());
        }
        let me = self.clone();
        tokio::spawn(async move {
            me.dispatch_loop(reader).await;
        });
    }

    /// 把一个 run 的 `/task_mgr/{run_id}` 加入订阅集。
    pub async fn watch_run(&self, run_id: &str) {
        let pattern = run_pattern(run_id);
        let reader = {
            let mut guard = self.state.lock().await;
            if !guard.watched_runs.insert(run_id.to_string()) {
                return;
            }
            match guard.reader.clone() {
                Some(r) => r,
                None => {
                    debug!(
                        "workflow.subscriptions.watch_run: no reader yet, queued run_id={}",
                        run_id
                    );
                    return;
                }
            }
        };
        if let Err(err) = reader.add_patterns(vec![pattern.clone()]).await {
            warn!(
                "workflow.subscriptions.watch_run: add_patterns failed run_id={} pattern={} err={:?}",
                run_id, pattern, err
            );
        } else {
            debug!(
                "workflow.subscriptions.watch_run: subscribed run_id={} pattern={}",
                run_id, pattern
            );
        }
    }

    /// 把一个 run 的 pattern 摘掉（run 进入终态后调用）。
    pub async fn unwatch_run(&self, run_id: &str) {
        let pattern = run_pattern(run_id);
        let reader = {
            let mut guard = self.state.lock().await;
            if !guard.watched_runs.remove(run_id) {
                return;
            }
            match guard.reader.clone() {
                Some(r) => r,
                None => return,
            }
        };
        if let Err(err) = reader.remove_patterns(vec![pattern.clone()]).await {
            debug!(
                "workflow.subscriptions.unwatch_run: remove_patterns failed run_id={} err={:?}",
                run_id, err
            );
        }
    }

    async fn dispatch_loop(self: Arc<Self>, reader: Arc<EventReader>) {
        loop {
            match reader.pull_event(Some(SWEEP_INTERVAL_MS)).await {
                Ok(Some(event)) => {
                    if let Err(err) = self.handle_event(&event).await {
                        debug!(
                            "workflow.subscriptions.dispatch_loop: handle_event err={} eventid={}",
                            err, event.eventid
                        );
                    }
                }
                // 超时 == 这一窗口里 kevent 没有新东西。kevent 是加速通道、可能
                // 漏推（rate-limit、SHM 槽位覆盖、`data_omitted` 回拉失败等），
                // 这里手工扫一遍权威源把 WaitingHuman 节点的人工动作回灌一次。
                Ok(None) => {
                    self.sweep_watched_runs().await;
                }
                Err(err) => {
                    warn!(
                        "workflow.subscriptions.dispatch_loop: pull_event err={:?}, exiting",
                        err
                    );
                    return;
                }
            }
        }
    }

    async fn handle_event(&self, event: &buckyos_api::Event) -> Result<(), String> {
        let payload = &event.data;

        // change_kind != data 的事件不可能携带新的 human_action，绕开省一次
        // lock + apply_task_data 空跑。
        let change_kind = payload
            .get("change_kind")
            .and_then(Value::as_str)
            .unwrap_or("");
        if change_kind != "data" {
            return Ok(());
        }

        // task_manager 的 inline 阈值很小（~1300 字节，见
        // task_manager/server.rs），整段 task_data 一过阈值就只剩
        // `data_omitted=true`。这里按 task_id 回拉一次拿全量 data。
        let fetched_data: Value;
        let data: &Value = match payload.get("data") {
            Some(d) if !d.is_null() => d,
            _ => {
                if !payload
                    .get("data_omitted")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    return Ok(());
                }
                let task_id = payload
                    .get("task_id")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| "data_omitted event missing task_id".to_string())?;
                let client = self.task_mgr_client.as_ref().ok_or_else(|| {
                    format!(
                        "data_omitted event task_id={} dropped: task_mgr client unavailable",
                        task_id
                    )
                })?;
                let task = client
                    .get_task(task_id)
                    .await
                    .map_err(|err| format!("get_task task_id={} failed: {:?}", task_id, err))?;
                debug!(
                    "workflow.subscriptions: refetched omitted task_data task_id={} size={}",
                    task_id,
                    payload
                        .get("data_size")
                        .and_then(Value::as_u64)
                        .unwrap_or(0)
                );
                fetched_data = task.data;
                &fetched_data
            }
        };

        // root_id 由 task_manager 写入 = workflow 创建 task 时传的 run_id。
        let run_id = payload
            .get("root_id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "event missing root_id".to_string())?;

        if let Err(err) = self.apply_run_data(run_id, data).await {
            debug!(
                "workflow.subscriptions: apply_run_data failed run_id={} err={}",
                run_id, err
            );
        }
        Ok(())
    }

    /// 把一段 task_data 喂给 orchestrator.apply_task_data 并 tick 后继节点。
    /// 事件路径和 sweep 路径共用同一个入口，保证两边幂等性一致——已经应用过
    /// 的 human_action 再被回放只会拿到 `NodeNotWaitingHuman` / 空 events。
    async fn apply_run_data(&self, run_id: &str, data: &Value) -> Result<(), String> {
        let handle = self
            .runs
            .get(run_id)
            .await
            .ok_or_else(|| format!("run `{}` not found in store", run_id))?;
        let definition = self
            .definitions
            .get_by_id(&handle.workflow_id)
            .await
            .ok_or_else(|| format!("definition `{}` not found", handle.workflow_id))?;

        let mut record = handle.state.lock().await;
        let pre_seq = record.run.seq;
        let mut events = self
            .orchestrator
            .apply_task_data(&definition.compiled, &mut record.run, data)
            .await
            .map_err(|err| format!("apply_task_data: {}", err))?;

        // apply_task_data 改了状态（events 非空）才需要 tick 把后继节点推进。
        // 没有 human_action 的事件会走 events 为空的快速路径。
        if !events.is_empty() {
            match self
                .orchestrator
                .tick(&definition.compiled, &mut record.run)
                .await
            {
                Ok(more) => events.extend(more),
                Err(err) => warn!(
                    "workflow.subscriptions: post-apply tick failed run_id={} err={}",
                    run_id, err
                ),
            }
            record.append_events(&events);
            debug!(
                "workflow.subscriptions: applied run_id={} from_seq={} to_seq={}",
                run_id, pre_seq, record.run.seq
            );
        }

        let status = record.run.status;
        drop(record);
        if is_terminal(status) {
            self.unwatch_run(run_id).await;
        }
        Ok(())
    }

    /// dispatch loop 超时兜底：对每个仍在 watched_runs 里的 run 跑一次手工 sweep。
    /// task_mgr_client 不可用时直接 noop（与 §`data_omitted` 回拉对齐：
    /// task_manager 不可用整体降级，单点不补偿）。
    async fn sweep_watched_runs(self: &Arc<Self>) {
        let Some(client) = self.task_mgr_client.clone() else {
            return;
        };
        let runs: Vec<String> = {
            let guard = self.state.lock().await;
            guard.watched_runs.iter().cloned().collect()
        };
        if runs.is_empty() {
            return;
        }
        for run_id in runs {
            if let Err(err) = self.sweep_run(client.as_ref(), &run_id).await {
                debug!(
                    "workflow.subscriptions.sweep: run_id={} err={}",
                    run_id, err
                );
            }
        }
    }

    /// 拉这个 run 下 `workflow/step` 类型的 task 列表，挑还停在 WaitingHuman
    /// 的节点对应的 task，把 task.data 喂回 apply_run_data。其他 task（已经
    /// 通过 event 路径处理过的、还没到 waiting_human 的、根本没人写过
    /// human_action 的）都跳过，避免每个周期重复回放产生噪声。
    async fn sweep_run(
        &self,
        client: &TaskManagerClient,
        run_id: &str,
    ) -> Result<(), String> {
        let waiting_nodes: HashSet<String> = {
            let handle = match self.runs.get(run_id).await {
                Some(h) => h,
                None => return Ok(()),
            };
            let record = handle.state.lock().await;
            record
                .run
                .node_states
                .iter()
                .filter(|(_, state)| **state == NodeRunState::WaitingHuman)
                .map(|(node_id, _)| node_id.clone())
                .collect()
        };
        if waiting_nodes.is_empty() {
            return Ok(());
        }

        let filter = TaskFilter {
            root_id: Some(run_id.to_string()),
            task_type: Some(STEP_TASK_TYPE.to_string()),
            ..Default::default()
        };
        let tasks = client
            .list_tasks(Some(filter), None, None)
            .await
            .map_err(|err| format!("list_tasks: {:?}", err))?;

        for task in tasks {
            let Some(node_id) = step_task_node_id(&task) else {
                continue;
            };
            if !waiting_nodes.contains(node_id) {
                continue;
            }
            if task.data.get("human_action").is_none() {
                continue;
            }
            debug!(
                "workflow.subscriptions.sweep: replay run_id={} node={} task_id={}",
                run_id, node_id, task.id
            );
            if let Err(err) = self.apply_run_data(run_id, &task.data).await {
                debug!(
                    "workflow.subscriptions.sweep: apply run_id={} node={} task_id={} err={}",
                    run_id, node_id, task.id, err
                );
            }
        }
        Ok(())
    }
}

fn step_task_node_id(task: &Task) -> Option<&str> {
    task.data
        .get("workflow")
        .and_then(|w| w.get("node_id"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}
