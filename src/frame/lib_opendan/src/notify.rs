//! System Notify on the agent side (`doc/message_hub/System Notify.md`).
//!
//! A host gives a [`NotifyHost`]: in a zone it is msg-center's `notify.*`
//! (OpenDAN Loader), [`LocalNotify`] keeps notifications as files of the
//! AgentRoot for `--dev` and tests (`xagent notify answer` plays the user).
//! Without one nothing changes: a Work cannot ask, `agent-session notify`
//! says notifications are not available.
//!
//! Notifications never enter a session's history. A UI session only sees
//! the ones shown in its conversation, as `_notify` semi-subscription events
//! ([`notify_event`]).

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::json;

use buckyos_api::{commit_notify_answer, MsgCenterClient, NotifyCommit, TaskManagerClient};
pub use buckyos_api::{
    input_prompt, notify_url, prompt_options, HumanPromptOption, HumanTaskPrompt, NotifyAnswer,
    NotifyCategory, NotifyConversation, NotifyDeliveredData, NotifyKind, NotifyLevel,
    NotifyListReq, NotifyResult, NotifySendReq, NotifySource, NotifyStatus, NotifyView,
    NOTIFY_ACTION_ANSWER, NOTIFY_ACTION_DISMISS,
};

use crate::error::{OpenDanError, Result};
use crate::protocol::*;
use crate::state::AgentStateClient;

/// Who answers on the user's behalf: an Agent in the conversation the user
/// answered in. Kept with the answer (task note / `acked_by`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotifyVia {
    pub agent_did: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// The input (message key) the user's answer came with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
}

impl NotifyVia {
    pub fn note(&self) -> String {
        let mut s = format!("answered by {}", self.agent_did);
        if let Some(sid) = &self.session_id {
            s.push_str(&format!(" in session {sid}"));
        }
        if let Some(input) = &self.input {
            s.push_str(&format!(" after input {input}"));
        }
        s
    }
}

/// The notification service of the host.
#[async_trait]
pub trait NotifyHost: Send + Sync {
    /// Notify the agent's owner (`to_user` is filled by the host).
    /// Idempotent for the same `idempotency_key`.
    async fn send(&self, req: NotifySendReq) -> Result<NotifyView>;
    async fn state(&self, notify_id: &str) -> Result<NotifyView>;
    /// The owner's notifications.
    async fn list(&self, req: NotifyListReq) -> Result<Vec<NotifyView>>;
    async fn cancel(&self, notify_id: &str, reason: &str) -> Result<NotifyView>;
    /// As the user: answer (or dismiss) an `input`, acknowledge (or
    /// dismiss) a `notice`. `via`: on the user's behalf. A notification
    /// that is closed already (someone else was first) is a
    /// [`OpenDanError::NotifyConflict`] carrying its state.
    async fn answer(
        &self,
        notify_id: &str,
        answer: NotifyAnswer,
        via: Option<NotifyVia>,
    ) -> Result<NotifyView>;
}

/// A closed notification as the error of a request that needs it open.
pub fn closed_conflict(view: &NotifyView) -> OpenDanError {
    OpenDanError::NotifyConflict(
        json!({
            "notify_id": view.notify_id,
            "status": view.status,
            "result": view.result,
            "closed_reason": view.closed_reason,
        })
        .to_string(),
    )
}

// ----------------------------------------------------------------------
// ZoneNotify
// ----------------------------------------------------------------------

fn zone_err(e: impl std::fmt::Display) -> OpenDanError {
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
    /// The service a process inside one of the agent's sessions reaches (an
    /// `agent-session` command): the zone's msg-center as the agent's app,
    /// with the session token its host hands the session's tools. `None`
    /// outside a zone.
    pub async fn for_session_process() -> std::result::Result<Self, String> {
        let token = std::env::var(agent_tool::BUCKYOS_APPCLIENT_SESSION_TOKEN_ENV)
            .ok()
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(|| {
                format!(
                    "no ${} (not a session of a zone-hosted agent)",
                    agent_tool::BUCKYOS_APPCLIENT_SESSION_TOKEN_ENV
                )
            })?;
        // An app's session token speaks for its owner (`sub`), as its app.
        let claims = ::kRPC::RPCSessionToken::from_string(&token)
            .map_err(|e| format!("session token: {e}"))?;
        let owner_user = claims
            .sub
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| "the session token has no subject".to_string())?;
        if let Some(app) = claims.appid.filter(|a| !a.trim().is_empty()) {
            // The runtime accepts the token only as the app it names.
            std::env::set_var("BUCKYOS_APP_ID", app);
        }
        agent_tool::xllm::ensure_buckyos_runtime()
            .await
            .map_err(|e| format!("BuckyOS runtime: {e}"))?;
        Ok(Self { owner_user })
    }

    async fn msg_center() -> Result<MsgCenterClient> {
        buckyos_api::get_buckyos_api_runtime()
            .map_err(zone_err)?
            .get_msg_center_client()
            .await
            .map_err(zone_err)
    }

    async fn task_mgr() -> Result<TaskManagerClient> {
        buckyos_api::get_buckyos_api_runtime()
            .map_err(zone_err)?
            .get_task_mgr_client()
            .await
            .map_err(zone_err)
    }
}

#[async_trait]
impl NotifyHost for ZoneNotify {
    async fn send(&self, mut req: NotifySendReq) -> Result<NotifyView> {
        req.to_user = self.owner_user.clone();
        Self::msg_center().await?.notify_send(req).await.map_err(zone_err)
    }

    async fn state(&self, notify_id: &str) -> Result<NotifyView> {
        Self::msg_center().await?.notify_get(notify_id).await.map_err(zone_err)
    }

    async fn list(&self, req: NotifyListReq) -> Result<Vec<NotifyView>> {
        Ok(Self::msg_center()
            .await?
            .notify_list(req)
            .await
            .map_err(zone_err)?
            .items)
    }

    async fn cancel(&self, notify_id: &str, reason: &str) -> Result<NotifyView> {
        Self::msg_center()
            .await?
            .notify_cancel(notify_id, Some(reason.to_string()))
            .await
            .map_err(zone_err)
    }

    async fn answer(
        &self,
        notify_id: &str,
        answer: NotifyAnswer,
        via: Option<NotifyVia>,
    ) -> Result<NotifyView> {
        let mc = Self::msg_center().await?;
        let view = mc.notify_get(notify_id).await.map_err(zone_err)?;
        if view.status.is_closed() {
            return Err(closed_conflict(&view));
        }
        match view.kind {
            NotifyKind::Notice if answer.dismiss => mc.notify_dismiss(notify_id).await.map_err(zone_err),
            NotifyKind::Notice => mc
                .notify_ack(notify_id, via.as_ref().map(NotifyVia::note))
                .await
                .map_err(zone_err),
            NotifyKind::Input => {
                answer
                    .validate(&view.prompt)
                    .map_err(OpenDanError::InvalidArgument)?;
                let task_id = view
                    .task_id
                    .clone()
                    .ok_or_else(|| zone_err(format!("notification {notify_id} has no task")))?;
                let tasks = Self::task_mgr().await?;
                let committed = commit_notify_answer(
                    &tasks,
                    &task_id,
                    &answer,
                    &self.owner_user,
                    crate::now_ms(),
                )
                .await
                .map_err(zone_err)?;
                let now = mc.notify_get(notify_id).await.map_err(zone_err)?;
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

pub fn unavailable() -> OpenDanError {
    OpenDanError::Other("notifications are not available here".to_string())
}

// ----------------------------------------------------------------------
// source
// ----------------------------------------------------------------------

/// Complete `source` from the session it names: its Goal and the
/// conversation of the user it belongs to — a UI session's own, a Work's
/// nearest UI ancestor's, a Goal's (or a Goal Work's) creating UI
/// session's. Nothing is guessed: what cannot be found stays empty.
pub async fn complete_notify_source(agent: &dyn AgentStateClient, source: &mut NotifySource) {
    let Some(sid) = source.session_id.clone() else {
        return;
    };
    let Ok(Some(entry)) = agent.sessions().lookup(&sid).await else {
        return;
    };
    if source.goal_id.is_none() {
        source.goal_id = entry.goal_id.clone();
    }
    if source.conversation.is_none() {
        source.conversation = conversation_of(agent, entry).await;
    }
}

async fn conversation_of(
    agent: &dyn AgentStateClient,
    mut entry: RegistryEntry,
) -> Option<NotifyConversation> {
    for _ in 0..8 {
        if entry.kind == SessionKind::Ui {
            let cfg = crate::session::SessionDir::open(&entry.location).ok()?.config().ok()?;
            let out = cfg.channels.outbound?;
            return Some(NotifyConversation {
                to: out.to,
                to_session: out.to_session,
            });
        }
        let next = match entry.origin.as_ref().and_then(|o| o.parent_session.clone()) {
            Some(parent) if entry.kind != SessionKind::Goal => parent,
            _ => {
                let goal_id = entry.goal_id.clone()?;
                let goal = agent.goals().get(&goal_id).await.ok()??;
                goal.created_by.via.strip_prefix("ui_session:")?.to_string()
            }
        };
        entry = agent.sessions().lookup(&next).await.ok()??;
    }
    None
}

// ----------------------------------------------------------------------
// `_notify` events
// ----------------------------------------------------------------------

fn cut(text: &str, max: usize) -> String {
    let one = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= max {
        return one;
    }
    let mut out: String = one.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// The bus record of a notification shown in conversation `session` (the
/// Agent INBOX session): `(key, event)` of the `_notify` subscription.
pub fn notify_event(data: &NotifyDeliveredData, session: &str) -> (String, AgentEvent) {
    let from = match (&data.source.goal_id, &data.source.session_id) {
        (Some(goal), _) => format!("goal {goal}"),
        (None, Some(sid)) => format!("session {sid}"),
        (None, None) => "this agent".to_string(),
    };
    let at = chrono::DateTime::from_timestamp_millis(data.at_ms as i64)
        .map(|t| t.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_default();
    let mut summary = format!(
        "[notify {}] {}/{} from {from} at {at}: {}",
        data.notify_id,
        data.kind.as_str(),
        data.level.as_str(),
        cut(&data.title, 160)
    );
    if let Some(m) = data.message.as_deref().filter(|m| !m.trim().is_empty()) {
        summary.push_str(&format!(" — {}", cut(m, 300)));
    }
    if !data.options.is_empty() {
        summary.push_str(&format!(
            " (options: {})",
            data.options
                .iter()
                .map(|o| format!("{}={}", o.value, cut(&o.label, 40)))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    summary.push_str(&format!(
        " — check the current state with `agent-session notify show {}`",
        data.notify_id
    ));
    let key = format!("notify:{}:{session}", data.notify_id);
    (
        key,
        AgentEvent {
            subscription_id: Some(NOTIFY_SUBSCRIPTION.to_string()),
            source: EventSource::new("system", format!("{NOTIFY_SOURCE_ID}:{}", data.notify_id)),
            event: NOTIFY_EVENT_DELIVERED.to_string(),
            seq: None,
            summary,
            data_ref: None,
            terminal: false,
        },
    )
}

// ----------------------------------------------------------------------
// LocalNotify
// ----------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LocalRecord {
    view: NotifyView,
    idempotency_key: String,
    #[serde(default)]
    notes: Vec<String>,
}

/// Notifications as files (`<agent_root>/state/notifies/<id>.json`), for
/// `--dev` and tests. Enabled by the directory existing.
#[derive(Debug, Clone)]
pub struct LocalNotify {
    dir: PathBuf,
    agent_did: String,
}

impl LocalNotify {
    pub fn dir_of(agent_root: &Path) -> PathBuf {
        agent_root.join("state").join("notifies")
    }

    /// Turn file notifications on for the agent at `agent_root`.
    pub fn enable(agent_root: &Path, agent_did: &str) -> Result<Self> {
        let dir = Self::dir_of(agent_root);
        std::fs::create_dir_all(&dir).map_err(|e| OpenDanError::io(&dir, e))?;
        Ok(Self {
            dir,
            agent_did: agent_did.to_string(),
        })
    }

    pub fn open_if_enabled(agent_root: &Path, agent_did: &str) -> Option<Self> {
        let dir = Self::dir_of(agent_root);
        dir.is_dir().then(|| Self {
            dir,
            agent_did: agent_did.to_string(),
        })
    }

    fn lock(&self) -> Result<std::fs::File> {
        let path = self.dir.join(".lock");
        let f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|e| OpenDanError::io(&path, e))?;
        f.lock_exclusive().map_err(|e| OpenDanError::io(&path, e))?;
        Ok(f)
    }

    fn path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{}.json", crate::ids::sanitize_segment(id)))
    }

    fn all(&self) -> Result<Vec<LocalRecord>> {
        let mut out = Vec::new();
        let entries = std::fs::read_dir(&self.dir).map_err(|e| OpenDanError::io(&self.dir, e))?;
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "json") {
                if let Some(r) = crate::fsutil::read_json_opt::<LocalRecord>(&p)? {
                    out.push(r);
                }
            }
        }
        Ok(out)
    }

    fn load(&self, id: &str) -> Result<LocalRecord> {
        crate::fsutil::read_json_opt::<LocalRecord>(&self.path(id))?
            .ok_or_else(|| OpenDanError::NotFound(format!("notification {id}")))
    }

    fn save(&self, r: &LocalRecord) -> Result<()> {
        crate::fsutil::atomic_replace_json(&self.path(&r.view.notify_id), r)
    }

    /// A notification past its expiry is expired when it is read.
    fn refresh(&self, mut r: LocalRecord) -> Result<LocalRecord> {
        let now = crate::now_ms();
        if !r.view.status.is_closed() && r.view.expires_at_ms.is_some_and(|at| at <= now) {
            r.view.status = NotifyStatus::Expired;
            r.view.closed_reason = Some("expired".into());
            r.view.closed_at_ms = Some(now);
            self.save(&r)?;
        }
        Ok(r)
    }

    /// Notes kept with an answer given on the user's behalf.
    pub fn notes(&self, notify_id: &str) -> Result<Vec<String>> {
        Ok(self.load(notify_id)?.notes)
    }
}

#[async_trait]
impl NotifyHost for LocalNotify {
    async fn send(&self, req: NotifySendReq) -> Result<NotifyView> {
        let _lock = self.lock()?;
        let key = req.idempotency_key.trim().to_string();
        if key.is_empty() {
            return Err(OpenDanError::InvalidArgument("idempotency_key is required".into()));
        }
        if let Some(r) = self.all()?.into_iter().find(|r| r.idempotency_key == key) {
            return Ok(self.refresh(r)?.view);
        }
        let title = req
            .prompt
            .title
            .clone()
            .or(req.prompt.message.clone())
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(|| OpenDanError::InvalidArgument("a notification needs a title".into()))?;
        let mut prompt = req.prompt;
        prompt.title = Some(title.clone());
        if req.kind == NotifyKind::Notice {
            prompt.controls.clear();
        } else if prompt.controls.is_empty() {
            prompt = input_prompt(title, prompt.message.clone().unwrap_or_default(), Vec::new(), true);
        }
        let id = format!("nt-{}", uuid::Uuid::new_v4().simple());
        let mut source = req.source.unwrap_or_default();
        source.agent_did = Some(self.agent_did.clone());
        source.app_id = Some(crate::ids::agent_id_from_did(&self.agent_did));
        let view = NotifyView {
            url: notify_url(None, &id),
            task_id: (req.kind == NotifyKind::Input).then(|| format!("local-task-{id}")),
            notify_id: id,
            to_user: req.to_user,
            kind: req.kind,
            category: req.category.unwrap_or(NotifyCategory::Agent),
            level: req.level.unwrap_or_default(),
            status: NotifyStatus::Open,
            prompt,
            source,
            created_at_ms: crate::now_ms(),
            expires_at_ms: req.expires_at_ms,
            closed_at_ms: None,
            closed_reason: None,
            acked_by: None,
            result: None,
            home: None,
            deliveries: Vec::new(),
        };
        let r = LocalRecord {
            view,
            idempotency_key: key,
            notes: Vec::new(),
        };
        self.save(&r)?;
        Ok(r.view)
    }

    async fn state(&self, notify_id: &str) -> Result<NotifyView> {
        let _lock = self.lock()?;
        Ok(self.refresh(self.load(notify_id)?)?.view)
    }

    async fn list(&self, req: NotifyListReq) -> Result<Vec<NotifyView>> {
        let _lock = self.lock()?;
        let mut out = Vec::new();
        for r in self.all()? {
            let v = self.refresh(r)?.view;
            let keep = req.status.as_ref().is_none_or(|s| s.contains(&v.status))
                && req.kind.is_none_or(|k| k == v.kind)
                && req.category.is_none_or(|c| c == v.category)
                && req
                    .session_id
                    .as_deref()
                    .is_none_or(|s| v.source.session_id.as_deref() == Some(s))
                && req
                    .goal_id
                    .as_deref()
                    .is_none_or(|g| v.source.goal_id.as_deref() == Some(g))
                && req
                    .agent_did
                    .as_deref()
                    .is_none_or(|a| v.source.agent_did.as_deref() == Some(a))
                && req.since_ms.is_none_or(|t| v.created_at_ms >= t);
            if keep {
                out.push(v);
            }
        }
        out.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms).then(b.notify_id.cmp(&a.notify_id)));
        if let Some(limit) = req.limit {
            out.truncate(limit);
        }
        Ok(out)
    }

    async fn cancel(&self, notify_id: &str, reason: &str) -> Result<NotifyView> {
        let _lock = self.lock()?;
        let mut r = self.refresh(self.load(notify_id)?)?;
        if !r.view.status.is_closed() {
            r.view.status = if reason == "expired" {
                NotifyStatus::Expired
            } else {
                NotifyStatus::Canceled
            };
            r.view.closed_reason = Some(reason.to_string());
            r.view.closed_at_ms = Some(crate::now_ms());
            self.save(&r)?;
        }
        Ok(r.view)
    }

    async fn answer(
        &self,
        notify_id: &str,
        answer: NotifyAnswer,
        via: Option<NotifyVia>,
    ) -> Result<NotifyView> {
        let _lock = self.lock()?;
        let mut r = self.refresh(self.load(notify_id)?)?;
        let now = crate::now_ms();
        match r.view.kind {
            NotifyKind::Notice => {
                if r.view.status.is_closed() {
                    return Err(closed_conflict(&r.view));
                }
                if answer.dismiss {
                    r.view.status = NotifyStatus::Dismissed;
                    r.view.closed_reason = Some("dismissed".into());
                    r.view.closed_at_ms = Some(now);
                } else {
                    r.view.status = NotifyStatus::Read;
                    r.view.acked_by = via.as_ref().map(NotifyVia::note);
                }
            }
            NotifyKind::Input => {
                if r.view.status.is_closed() {
                    return Err(closed_conflict(&r.view));
                }
                answer
                    .validate(&r.view.prompt)
                    .map_err(OpenDanError::InvalidArgument)?;
                let value = answer.to_task_result(&r.view.to_user, now);
                let values = value
                    .get("values")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();
                r.view.result = Some(NotifyResult {
                    action: if answer.dismiss {
                        NOTIFY_ACTION_DISMISS
                    } else {
                        NOTIFY_ACTION_ANSWER
                    }
                    .to_string(),
                    values,
                    submitted_by: Some(r.view.to_user.clone()),
                    submitted_at: Some(now as i64),
                    committed_by: via.as_ref().map(|v| v.agent_did.clone()),
                });
                r.view.status = if answer.dismiss {
                    NotifyStatus::Dismissed
                } else {
                    NotifyStatus::Answered
                };
                r.view.closed_at_ms = Some(now);
            }
        }
        if let Some(v) = &via {
            r.notes.push(v.note());
        }
        self.save(&r)?;
        Ok(r.view)
    }
}

/// The notification service of an Agent State client as a host dependency.
pub struct StateNotify(pub std::sync::Arc<dyn AgentStateClient>);

impl StateNotify {
    /// `None` when `agent` has no notification service.
    pub fn of(agent: std::sync::Arc<dyn AgentStateClient>) -> Option<std::sync::Arc<dyn NotifyHost>> {
        agent
            .notifies()
            .is_some()
            .then(|| std::sync::Arc::new(StateNotify(agent)) as std::sync::Arc<dyn NotifyHost>)
    }

    fn host(&self) -> Result<&dyn NotifyHost> {
        self.0.notifies().ok_or_else(unavailable)
    }
}

#[async_trait]
impl NotifyHost for StateNotify {
    async fn send(&self, req: NotifySendReq) -> Result<NotifyView> {
        self.host()?.send(req).await
    }
    async fn state(&self, notify_id: &str) -> Result<NotifyView> {
        self.host()?.state(notify_id).await
    }
    async fn list(&self, req: NotifyListReq) -> Result<Vec<NotifyView>> {
        self.host()?.list(req).await
    }
    async fn cancel(&self, notify_id: &str, reason: &str) -> Result<NotifyView> {
        self.host()?.cancel(notify_id, reason).await
    }
    async fn answer(
        &self,
        notify_id: &str,
        answer: NotifyAnswer,
        via: Option<NotifyVia>,
    ) -> Result<NotifyView> {
        self.host()?.answer(notify_id, answer, via).await
    }
}
