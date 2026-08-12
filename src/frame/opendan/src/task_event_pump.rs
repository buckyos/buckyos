//! Bound-task kevent subscription pump.
//!
//! Subscribes to the TaskMgr change channels of exactly the `agent.delegate`
//! tasks this agent currently runs, so external control (cancel/pause), an
//! answered `human.input` child, or any other task mutation wakes the
//! executor immediately instead of waiting for the next owner sweep.
//!
//! KEvent discipline (the project-wide rule): this channel is acceleration
//! ONLY. An event never carries truth — on every hit the pump re-reads the
//! task from TaskMgr and drives it through the same idempotent
//! `process_accepted_dispatch_task` path the sweep uses. The minute-level
//! owner sweep stays on as the lost-event backstop.
//!
//! Never a discovery channel: the pump only ever watches tasks that reached
//! the executor through the dispatch adapter or the owner sweep. There is
//! no global `/task_mgr/**` subscription (removed in beta2.2 by design).
//!
//! Per bound task two patterns are held:
//! - `/task_mgr/{task_id}` — the task's own mutations
//! - `/task_mgr/tree/{root_id}` — child-task events (e.g. the `human.input`
//!   child completing publishes on the tree channel of the shared root)

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use buckyos_api::{
    task_mgr_task_event_path, task_mgr_tree_event_path, EventReader, KEventClient, KEventError,
};
use log::{debug, info, warn};
use tokio::sync::{Mutex, Notify};
use tokio::time::sleep;

use crate::agent::AIAgent;

/// Same cadence as the other opendan pumps: short enough for prompt
/// shutdown/refresh, long enough to avoid RPC churn.
const EVENT_PULL_TIMEOUT_MS: u64 = 1_000;

pub struct TaskEventPump {
    agent_name: String,
    kevent_client: Arc<KEventClient>,
    /// `task_id -> root_id` of every bound task currently watched.
    watched: Mutex<HashMap<String, String>>,
    refresh: Arc<Notify>,
    shutdown: Arc<Notify>,
}

impl TaskEventPump {
    pub fn new(
        agent_name: String,
        kevent_client: Arc<KEventClient>,
        shutdown: Arc<Notify>,
    ) -> Arc<Self> {
        Arc::new(Self {
            agent_name,
            kevent_client,
            watched: Mutex::new(HashMap::new()),
            refresh: Arc::new(Notify::new()),
            shutdown,
        })
    }

    /// Start (or keep) watching a bound task. Idempotent.
    pub async fn watch(&self, task_id: &str, root_id: &str) {
        let mut guard = self.watched.lock().await;
        let changed = guard
            .insert(task_id.to_string(), root_id.to_string())
            .map(|prev| prev != root_id)
            .unwrap_or(true);
        drop(guard);
        if changed {
            self.refresh.notify_waiters();
        }
    }

    /// Stop watching a task (it reached a terminal phase or left this
    /// agent). Idempotent.
    pub async fn unwatch(&self, task_id: &str) {
        let mut guard = self.watched.lock().await;
        if guard.remove(task_id).is_some() {
            drop(guard);
            self.refresh.notify_waiters();
        }
    }

    /// Sorted, deduped union of the kevent patterns for all watched tasks.
    async fn union_patterns(&self) -> Vec<String> {
        let guard = self.watched.lock().await;
        let mut acc: Vec<String> = Vec::with_capacity(guard.len() * 2);
        for (task_id, root_id) in guard.iter() {
            acc.push(task_mgr_task_event_path(task_id));
            acc.push(task_mgr_tree_event_path(root_id));
        }
        acc.sort();
        acc.dedup();
        acc
    }

    /// Which watched tasks does this event concern? Tree events fan out to
    /// every watched task sharing the root (a tree may hold unrelated
    /// siblings — the re-read makes the extra drives harmless no-ops).
    async fn tasks_for_event(&self, eventid: &str) -> Vec<String> {
        let guard = self.watched.lock().await;
        if let Some(root) = eventid.strip_prefix("/task_mgr/tree/") {
            guard
                .iter()
                .filter(|(_, task_root)| task_root.as_str() == root)
                .map(|(task_id, _)| task_id.clone())
                .collect()
        } else if let Some(task_id) = eventid.strip_prefix("/task_mgr/") {
            guard
                .contains_key(task_id)
                .then(|| vec![task_id.to_string()])
                .unwrap_or_default()
        } else {
            Vec::new()
        }
    }

    /// Run until shutdown. Reader lifecycle mirrors `SessionEventPump`:
    /// one reader over the pattern union, rebuilt on watch-set changes and
    /// on reader loss.
    pub async fn run(self: Arc<Self>, agent: Arc<AIAgent>) {
        info!("opendan.task_event_pump[{}]: starting", self.agent_name);
        let runner = match agent.task_executor_runner_id() {
            Ok(runner) => runner,
            Err(err) => {
                warn!(
                    "opendan.task_event_pump[{}]: no runner id, pump disabled: {err:#}",
                    self.agent_name
                );
                return;
            }
        };
        let mut reader: Option<Arc<EventReader>> = None;
        let mut last_union: Vec<String> = Vec::new();

        loop {
            let current_union = self.union_patterns().await;
            let needs_rebuild = current_union != last_union || reader.is_none();
            if needs_rebuild {
                if let Some(prev) = reader.take() {
                    let _ = prev.close().await;
                }
                if !current_union.is_empty() {
                    match self
                        .kevent_client
                        .create_event_reader(current_union.clone())
                        .await
                    {
                        Ok(r) => {
                            debug!(
                                "opendan.task_event_pump[{}]: reader {} patterns={:?}",
                                self.agent_name,
                                r.reader_id(),
                                current_union
                            );
                            reader = Some(Arc::new(r));
                        }
                        Err(err) => {
                            warn!(
                                "opendan.task_event_pump[{}]: create_event_reader failed: {err:?}",
                                self.agent_name
                            );
                        }
                    }
                }
                last_union = current_union;
            }

            if let Some(r) = reader.as_ref().cloned() {
                tokio::select! {
                    _ = self.shutdown.notified() => {
                        let _ = r.close().await;
                        return;
                    }
                    _ = self.refresh.notified() => continue,
                    res = r.pull_event(Some(EVENT_PULL_TIMEOUT_MS)) => match res {
                        Ok(Some(event)) => {
                            self.drive_for_event(&agent, &runner, &event.eventid).await;
                        }
                        Ok(None) => {}
                        Err(KEventError::ReaderClosed(_)) => {
                            warn!(
                                "opendan.task_event_pump[{}]: reader closed — recreating",
                                self.agent_name
                            );
                            reader = None;
                            last_union.clear();
                        }
                        Err(err) => {
                            warn!(
                                "opendan.task_event_pump[{}]: pull_event error: {err:?}",
                                self.agent_name
                            );
                        }
                    }
                }
            } else {
                tokio::select! {
                    _ = self.shutdown.notified() => return,
                    _ = self.refresh.notified() => {}
                    _ = sleep(Duration::from_millis(EVENT_PULL_TIMEOUT_MS * 5)) => {
                        // Periodic wake so a swallowed refresh cannot strand
                        // a freshly-watched task (Notify is one-shot).
                    }
                }
            }
        }
    }

    /// Acceleration-path apply: re-read from the authoritative store and
    /// drive the same idempotent executor entry the sweep uses. The drive
    /// itself unwatches terminal tasks.
    async fn drive_for_event(&self, agent: &Arc<AIAgent>, runner: &str, eventid: &str) {
        for task_id in self.tasks_for_event(eventid).await {
            let task_mgr = match agent.runtime.task_mgr_client().await {
                Ok(client) => client,
                Err(err) => {
                    warn!(
                        "opendan.task_event_pump[{}]: task manager unavailable: {err}",
                        self.agent_name
                    );
                    return;
                }
            };
            let task = match task_mgr.get_task(&task_id).await {
                Ok(task) => task,
                Err(err) => {
                    warn!(
                        "opendan.task_event_pump[{}]: refresh task {} failed: {err}",
                        self.agent_name, task_id
                    );
                    continue;
                }
            };
            debug!(
                "opendan.task_event_pump[{}]: event {} → drive task {} (phase {:?})",
                self.agent_name, eventid, task_id, task.phase
            );
            if let Err(err) = agent
                .clone()
                .process_agent_delegate_task_from_event(task, runner)
                .await
            {
                warn!(
                    "opendan.task_event_pump[{}]: drive task {} failed: {err:#}",
                    self.agent_name, task_id
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pump() -> Arc<TaskEventPump> {
        TaskEventPump::new(
            "test".to_string(),
            Arc::new(KEventClient::new_local("test-task-pump")),
            Arc::new(Notify::new()),
        )
    }

    #[tokio::test]
    async fn union_covers_task_and_tree_patterns() {
        let pump = pump();
        pump.watch("t-1", "t-1").await;
        pump.watch("t-2", "t-root").await;
        let union = pump.union_patterns().await;
        assert_eq!(
            union,
            vec![
                "/task_mgr/t-1".to_string(),
                "/task_mgr/t-2".to_string(),
                "/task_mgr/tree/t-1".to_string(),
                "/task_mgr/tree/t-root".to_string(),
            ]
        );
        pump.unwatch("t-2").await;
        let union = pump.union_patterns().await;
        assert_eq!(
            union,
            vec![
                "/task_mgr/t-1".to_string(),
                "/task_mgr/tree/t-1".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn event_routing_matches_task_and_tree() {
        let pump = pump();
        pump.watch("t-1", "t-1").await;
        pump.watch("t-2", "t-root").await;
        assert_eq!(pump.tasks_for_event("/task_mgr/t-1").await, vec!["t-1"]);
        // A child event on the shared tree drives the watched task with
        // that root, not the child id itself.
        assert_eq!(
            pump.tasks_for_event("/task_mgr/tree/t-root").await,
            vec!["t-2"]
        );
        assert!(pump.tasks_for_event("/task_mgr/t-unknown").await.is_empty());
        assert!(pump
            .tasks_for_event("/task_mgr/tree/t-other")
            .await
            .is_empty());
        assert!(pump.tasks_for_event("/other_channel/x").await.is_empty());
    }
}
