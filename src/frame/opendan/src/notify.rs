//! System Notify of a hosted agent: msg-center's `notify.*` as the
//! sessions' [`NotifyHost`], answers committed to TaskMgr, and a watcher
//! that wakes the sessions whose questions were answered (kevent on the
//! notification's task; the Supervisor's and GoalHost's rechecks back it
//! up).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use buckyos_api::{
    commit_notify_answer, task_mgr_task_event_path, KEventClient, MsgCenterClient,
    NotifyCommit, TaskManagerClient,
};
use libopendan::host::Supervisor;
use libopendan::notify::{
    closed_conflict, NotifyAnswer, NotifyHost, NotifyKind, NotifyListReq, NotifySendReq,
    NotifyStatus, NotifyVia, NotifyView,
};
use libopendan::{OpenDanError, Result};

fn err(e: impl std::fmt::Display) -> OpenDanError {
    OpenDanError::Channel(format!("notify: {e}"))
}

/// msg-center / TaskMgr of the zone, as the agent's owner's notification
/// service. Clients are taken from the runtime for every call (the
/// process's session token is renewed).
pub struct ZoneNotify {
    /// Zone user id of the agent's owner (`to_user` of every notification).
    pub owner_user: String,
}

impl ZoneNotify {
    async fn msg_center() -> Result<MsgCenterClient> {
        buckyos_api::get_buckyos_api_runtime()
            .map_err(err)?
            .get_msg_center_client()
            .await
            .map_err(err)
    }

    async fn task_mgr() -> Result<TaskManagerClient> {
        buckyos_api::get_buckyos_api_runtime()
            .map_err(err)?
            .get_task_mgr_client()
            .await
            .map_err(err)
    }
}

#[async_trait]
impl NotifyHost for ZoneNotify {
    async fn send(&self, mut req: NotifySendReq) -> Result<NotifyView> {
        req.to_user = self.owner_user.clone();
        Self::msg_center().await?.notify_send(req).await.map_err(err)
    }

    async fn state(&self, notify_id: &str) -> Result<NotifyView> {
        Self::msg_center().await?.notify_get(notify_id).await.map_err(err)
    }

    async fn list(&self, req: NotifyListReq) -> Result<Vec<NotifyView>> {
        Ok(Self::msg_center()
            .await?
            .notify_list(req)
            .await
            .map_err(err)?
            .items)
    }

    async fn cancel(&self, notify_id: &str, reason: &str) -> Result<NotifyView> {
        Self::msg_center()
            .await?
            .notify_cancel(notify_id, Some(reason.to_string()))
            .await
            .map_err(err)
    }

    async fn answer(
        &self,
        notify_id: &str,
        answer: NotifyAnswer,
        via: Option<NotifyVia>,
    ) -> Result<NotifyView> {
        let mc = Self::msg_center().await?;
        let view = mc.notify_get(notify_id).await.map_err(err)?;
        if view.status.is_closed() {
            return Err(closed_conflict(&view));
        }
        match view.kind {
            NotifyKind::Notice if answer.dismiss => mc.notify_dismiss(notify_id).await.map_err(err),
            NotifyKind::Notice => mc
                .notify_ack(notify_id, via.as_ref().map(NotifyVia::note))
                .await
                .map_err(err),
            NotifyKind::Input => {
                answer
                    .validate(&view.prompt)
                    .map_err(OpenDanError::InvalidArgument)?;
                let task_id = view
                    .task_id
                    .clone()
                    .ok_or_else(|| err(format!("notification {notify_id} has no task")))?;
                let tasks = Self::task_mgr().await?;
                let committed = commit_notify_answer(
                    &tasks,
                    &task_id,
                    &answer,
                    &self.owner_user,
                    libopendan::now_ms(),
                )
                .await
                .map_err(err)?;
                let now = mc.notify_get(notify_id).await.map_err(err)?;
                match committed {
                    NotifyCommit::Committed(_) => {
                        if let Some(v) = &via {
                            if let Err(e) = tasks
                                .add_task_note(&task_id, Some("notify_proxy"), &v.note(), None)
                                .await
                            {
                                log::warn!("notify {notify_id}: task note not written: {e}");
                            }
                        }
                        Ok(now)
                    }
                    NotifyCommit::AlreadyClosed(_) => Err(closed_conflict(&now)),
                }
            }
        }
    }
}

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
