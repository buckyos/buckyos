//! System Notify of a hosted agent: the watcher that wakes the sessions
//! whose questions were answered (kevent on the notification's task; the
//! Supervisor's and GoalHost's rechecks back it up). The notification
//! service itself is [`ZoneNotify`] (libopendan).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use buckyos_api::{task_mgr_task_event_path, KEventClient};
use libopendan::host::Supervisor;
pub use libopendan::notify::ZoneNotify;
use libopendan::notify::{NotifyHost, NotifyKind, NotifyListReq, NotifyStatus};

/// Wakes what waits for a notification of this agent once it is closed:
/// the session that asked (a Work's `ask-user`) is hosted again, the Goals
/// look at their questions at once. Notifications are listed every
/// `every`; the kevent of their tasks shortens the wait.
pub async fn watch_answers(
    notify: Arc<dyn NotifyHost>,
    kevent: Option<Arc<KEventClient>>,
    supervisor: Arc<Supervisor>,
    agent_did: String,
    every: Duration,
) {
    // notify_id -> (task_id, session_id)
    let mut open: BTreeMap<String, (String, Option<String>)> = BTreeMap::new();
    loop {
        match notify
            .list(NotifyListReq {
                agent_did: Some(agent_did.clone()),
                kind: Some(NotifyKind::Input),
                status: Some(vec![NotifyStatus::Open, NotifyStatus::Read]),
                ..Default::default()
            })
            .await
        {
            Ok(views) => {
                for v in views {
                    if let Some(task) = v.task_id.clone() {
                        open.insert(v.notify_id.clone(), (task, v.source.session_id.clone()));
                    }
                }
            }
            Err(e) => log::debug!("notify watcher: list failed: {e}"),
        }
        let paths: Vec<String> = open
            .values()
            .map(|(task, _)| task_mgr_task_event_path(task))
            .collect();
        let waited = match (&kevent, paths.is_empty()) {
            (Some(k), false) => match k.create_event_reader(paths).await {
                Ok(reader) => {
                    let _ = reader.pull_event(Some(every.as_millis() as u64)).await;
                    true
                }
                Err(e) => {
                    log::debug!("notify watcher: kevent: {e}");
                    false
                }
            },
            _ => false,
        };
        if !waited {
            tokio::time::sleep(every).await;
        }
        let mut closed = Vec::new();
        for (id, (_, sid)) in &open {
            match notify.state(id).await {
                Ok(v) if v.status.is_closed() => closed.push((id.clone(), sid.clone())),
                Ok(_) => {}
                Err(e) => log::debug!("notify watcher: {id}: {e}"),
            }
        }
        if closed.is_empty() {
            continue;
        }
        if let Some(goals) = supervisor.goals() {
            goals.recheck_answers();
        }
        for (id, sid) in closed {
            open.remove(&id);
            if let Some(sid) = sid {
                if let Err(e) = supervisor.ensure_task(&sid, "notification answered").await {
                    log::debug!("notify watcher: session {sid} not hosted: {e}");
                }
            }
        }
    }
}
