//! System Notify of a hosted agent: the watcher that wakes the sessions
//! whose questions were answered (kevent on the notification's task; the
//! Supervisor's and GoalHost's rechecks back it up) and tells the
//! conversations that showed them. The notification service itself is
//! [`ZoneNotify`] (libopendan).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use buckyos_api::{task_mgr_task_event_path, KEventClient};
use libopendan::host::Supervisor;
pub use libopendan::notify::ZoneNotify;
use libopendan::notify::{
    conversations_of, notify_closed_event, NotifyHost, NotifyKind, NotifyListReq, NotifyStatus,
    NotifyView,
};
use libopendan::protocol::PostedInput;
use libopendan::state::AgentStateClient;

/// Questions closed while this process did not run: told at its start when
/// they are at most this old.
const CATCH_UP_MS: u64 = 24 * 3600 * 1000;
/// How far back a later look goes, for questions opened and closed between
/// two looks.
const OVERLAP_MS: u64 = 120_000;

/// Tell the conversations that showed `view` that it is closed, as a
/// background event (it opens no Turn). Told again, it is a duplicate.
async fn tell_conversations(agent: &dyn AgentStateClient, supervisor: &Supervisor, who: &str, view: &NotifyView) {
    for entry in conversations_of(agent, view).await {
        let sid = entry.session_id;
        let (key, event) = notify_closed_event(view, &sid);
        match agent.sessions().post_input(&sid, &PostedInput::event(who, key, event)).await {
            Ok(_) => {
                if let Err(e) = supervisor.ensure_task(&sid, "notification closed").await {
                    log::debug!("notify watcher: session {sid} not hosted: {e}");
                }
            }
            Err(e) => log::warn!("notify watcher: session {sid} not told about {}: {e}", view.notify_id),
        }
    }
}

/// Wakes what waits for a notification of this agent once it is closed:
/// the session that asked (an `ask_user` question, `source.session_id`) is
/// hosted again and its kept run reads the task's result, the Goals look at
/// their questions at once, the conversations that showed it are told.
/// Notifications are listed every `every`; the kevent of their tasks
/// shortens the wait.
pub async fn watch_answers(
    notify: Arc<dyn NotifyHost>,
    kevent: Option<Arc<KEventClient>>,
    supervisor: Arc<Supervisor>,
    agent: Arc<dyn AgentStateClient>,
    who: String,
    every: Duration,
) {
    let agent_did = agent.agent_did().to_string();
    // notify_id -> (task_id, session_id)
    let mut open: BTreeMap<String, (String, Option<String>)> = BTreeMap::new();
    let mut told: BTreeSet<String> = BTreeSet::new();
    let mut since = libopendan::now_ms().saturating_sub(CATCH_UP_MS);
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
        // What was opened and closed unseen.
        let now = libopendan::now_ms();
        match notify
            .list(NotifyListReq {
                agent_did: Some(agent_did.clone()),
                kind: Some(NotifyKind::Input),
                since_ms: Some(since),
                limit: Some(200),
                ..Default::default()
            })
            .await
        {
            Ok(views) => {
                since = now.saturating_sub(OVERLAP_MS);
                for v in views {
                    if v.status.is_closed() && !open.contains_key(&v.notify_id) && told.insert(v.notify_id.clone()) {
                        tell_conversations(agent.as_ref(), &supervisor, &who, &v).await;
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
                Ok(v) if v.status.is_closed() => closed.push((v, sid.clone())),
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
        for (view, sid) in closed {
            open.remove(&view.notify_id);
            if let Some(sid) = sid {
                if let Err(e) = supervisor.ensure_task(&sid, "notification answered").await {
                    log::debug!("notify watcher: session {sid} not hosted: {e}");
                }
            }
            if told.insert(view.notify_id.clone()) {
                tell_conversations(agent.as_ref(), &supervisor, &who, &view).await;
            }
        }
    }
}
