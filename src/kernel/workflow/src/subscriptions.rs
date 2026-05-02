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
//! - **回环安全**：workflow 自己 `update_task` 写出的 data 不带 `human_action`，
//!   apply_task_data 会直接返回 `Ok(vec![])`，所以即使触发了反向事件也是空跑。
//! - **终态清理**：apply_task_data 后如果 run 进了终态，就 remove_patterns
//!   把这条订阅摘掉。
//!
//! [`apply_task_data`]: crate::WorkflowOrchestrator::apply_task_data

use buckyos_api::{EventReader, KEventClient, RunStatus};
use log::{debug, info, warn};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::server::ServiceOrchestrator;
use crate::state::{DefinitionStore, RunStore};

/// 服务启动时占位用的 pattern。永远不会有 publisher 写到这条 channel。
const BOOTSTRAP_PATTERN: &str = "/workflow_service/__bootstrap__";

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
    ) -> Arc<Self> {
        Arc::new(Self {
            kevent_client,
            runs,
            definitions,
            orchestrator,
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
            let event = match reader.pull_event(None).await {
                Ok(Some(event)) => event,
                Ok(None) => continue,
                Err(err) => {
                    warn!(
                        "workflow.subscriptions.dispatch_loop: pull_event err={:?}, exiting",
                        err
                    );
                    return;
                }
            };
            if let Err(err) = self.handle_event(&event).await {
                debug!(
                    "workflow.subscriptions.dispatch_loop: handle_event err={} eventid={}",
                    err, event.eventid
                );
            }
        }
    }

    async fn handle_event(&self, event: &buckyos_api::Event) -> Result<(), String> {
        let payload = &event.data;

        // change_kind != data 的事件不可能携带新的 human_action；data_omitted=true
        // 的事件超过 60KB inline 阈值，human_action 不会那么大，直接丢。
        let change_kind = payload
            .get("change_kind")
            .and_then(Value::as_str)
            .unwrap_or("");
        if change_kind != "data" {
            return Ok(());
        }
        let data = match payload.get("data") {
            Some(d) if !d.is_null() => d,
            _ => return Ok(()),
        };

        // root_id 由 task_manager 写入 = workflow 创建 task 时传的 run_id。
        let run_id = payload
            .get("root_id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "event missing root_id".to_string())?;

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
        let mut events = match self
            .orchestrator
            .apply_task_data(&definition.compiled, &mut record.run, data)
            .await
        {
            Ok(events) => events,
            Err(err) => {
                debug!(
                    "workflow.subscriptions: apply_task_data failed run_id={} err={}",
                    run_id, err
                );
                return Ok(());
            }
        };

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
                "workflow.subscriptions: applied event run_id={} from_seq={} to_seq={}",
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
}
