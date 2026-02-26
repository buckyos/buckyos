use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use buckyos_api::{MsgRecord, MsgRecordWithObject, OpenDanAgentSessionRecord, OpenDanSessionLink};
use log::warn;
use name_lib::DID;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value as Json};
use tokio::fs;
use tokio::sync::{Mutex, RwLock};

use crate::agent_tool::{AgentTool, ToolError, ToolSpec};
use crate::behavior::{BehaviorConfig, TraceCtx};

pub const TOOL_GET_SESSION: &str = "get_session";

const DEFAULT_SESSION_FILE: &str = "session.json";
const MAX_SESSION_ID_LEN: usize = 180;
const SESSION_STATUS_PAUSE: &str = "pause";
const SESSION_STATUS_NORMAL: &str = "normal";

static SESSION_ITEM_SEQ: AtomicU64 = AtomicU64::new(0);
static SESSION_LINK_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SessionState {
    Pause,
    Wait,
    WaitForMsg,
    WaitForEvent,
    Ready,
    Running,
    Sleep,
}

impl Default for SessionState {
    fn default() -> Self {
        Self::Wait
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SessionWaitDetails {
    pub filter: Json,
    pub deadline_ms: Option<u64>,
    pub note: Option<String>,
}

impl Default for SessionWaitDetails {
    fn default() -> Self {
        Self {
            filter: Json::Null,
            deadline_ms: None,
            note: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct ConsumableIds {
    pub msg_ids: Vec<String>,
    pub event_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SessionInputItem {
    pub id: String,
    pub ts_ms: u64,
    pub payload: Json,
}

impl Default for SessionInputItem {
    fn default() -> Self {
        Self {
            id: String::new(),
            ts_ms: 0,
            payload: Json::Null,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SessionExecInput {
    pub session_id: String,
    pub behavior: String,
    pub step_index: u32,
    pub payload: Json,
    pub link_ids: Vec<String>,
    pub consumable_ids: ConsumableIds,
}

impl Default for SessionExecInput {
    fn default() -> Self {
        Self {
            session_id: String::new(),
            behavior: String::new(),
            step_index: 0,
            payload: Json::Null,
            link_ids: vec![],
            consumable_ids: ConsumableIds::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SessionMessageLinkState {
    New,
    Used,
    Acked,
}

impl Default for SessionMessageLinkState {
    fn default() -> Self {
        Self::New
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct SessionMessageLink {
    pub link_id: String,
    pub session_id: String,
    pub msg_tunnle_message_id: String,
    pub msg_record_id: String,
    pub state: SessionMessageLinkState,
    pub reason: String,
    pub created_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
struct SessionRuntimeMeta {
    state: SessionState,
    wait_details: Option<SessionWaitDetails>,
    current_behavior: Option<String>,
    step_index: u32,
    last_step_summary: Option<Json>,
    workspace_info: Option<Json>,
    local_workspace_id: Option<String>,
    worklog: Vec<Json>,
    cost_trace: Json,
    message_links: Vec<SessionMessageLink>,
}

impl Default for SessionRuntimeMeta {
    fn default() -> Self {
        Self {
            state: SessionState::Wait,
            wait_details: None,
            current_behavior: None,
            step_index: 0,
            last_step_summary: None,
            workspace_info: None,
            local_workspace_id: None,
            worklog: vec![],
            cost_trace: json!({}),
            message_links: vec![],
        }
    }
}

#[derive(Clone, Debug)]
pub struct AgentSession {
    pub session_id: String,
    pub owner_agent: String,
    pub title: String,
    pub summary: String,
    pub state: SessionState,
    pub wait_details: Option<SessionWaitDetails>,
    pub current_behavior: Option<String>,
    pub step_index: u32,
    pub last_step_summary: Option<Json>,
    pub new_msgs: Vec<SessionInputItem>,
    pub new_events: Vec<SessionInputItem>,
    pub history_msgs: Vec<SessionInputItem>,
    pub history_events: Vec<SessionInputItem>,

    pub workspace_info: Option<Json>,
    pub local_workspace_id: Option<String>,
    pub worklog: Vec<Json>,
    pub cost_trace: Json,
    pub message_links: Vec<SessionMessageLink>,
    pub links: Vec<OpenDanSessionLink>,
    pub tags: Vec<String>,
    pub meta: Json,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub last_activity_ms: u64,
}

impl AgentSession {
    pub fn new(
        session_id: impl Into<String>,
        owner_agent: impl Into<String>,
        default_behavior: Option<&str>,
    ) -> Self {
        let ts = now_ms();
        let session_id = session_id.into();
        let current_behavior = normalize_optional_string(default_behavior.map(|v| v.to_string()));
        Self {
            title: format!("Session {}", session_id),
            summary: String::new(),
            session_id,
            owner_agent: owner_agent.into(),
            state: SessionState::Wait,
            wait_details: None,
            current_behavior,
            step_index: 0,
            last_step_summary: None,
            new_msgs: vec![],
            new_events: vec![],
            history_msgs: vec![],
            history_events: vec![],
            workspace_info: None,
            local_workspace_id: None,
            worklog: vec![],
            cost_trace: json!({}),
            message_links: vec![],
            links: vec![],
            tags: vec![],
            meta: json!({}),
            created_at_ms: ts,
            updated_at_ms: ts,
            last_activity_ms: ts,
        }
    }

    pub fn from_record(record: OpenDanAgentSessionRecord) -> Self {
        let runtime_meta = parse_runtime_meta(&record.meta);
        let mut state = runtime_meta.state;
        if matches!(
            record.status.trim().to_ascii_lowercase().as_str(),
            SESSION_STATUS_PAUSE
        ) {
            state = SessionState::Pause;
        }
        let mut meta = record.meta.clone();
        if let Some(map) = meta.as_object_mut() {
            map.remove("runtime_state");
        }
        let mut summary = record.summary;
        if summary.trim().is_empty() {
            summary = runtime_meta
                .last_step_summary
                .as_ref()
                .and_then(extract_step_summary_text)
                .unwrap_or_default();
        }

        Self {
            session_id: record.session_id,
            owner_agent: record.owner_agent,
            title: if record.title.trim().is_empty() {
                "Untitled Session".to_string()
            } else {
                record.title
            },
            summary,
            state,
            wait_details: runtime_meta.wait_details,
            current_behavior: normalize_optional_string(runtime_meta.current_behavior),
            step_index: runtime_meta.step_index,
            last_step_summary: runtime_meta.last_step_summary,
            new_msgs: vec![],
            new_events: vec![],
            history_msgs: vec![],
            history_events: vec![],
            workspace_info: runtime_meta.workspace_info,
            local_workspace_id: normalize_optional_string(runtime_meta.local_workspace_id),
            worklog: runtime_meta.worklog,
            cost_trace: normalize_json_object(runtime_meta.cost_trace),
            message_links: runtime_meta.message_links,
            links: record.links,
            tags: record.tags,
            meta: normalize_json_object(meta),
            created_at_ms: record.created_at_ms,
            updated_at_ms: record.updated_at_ms,
            last_activity_ms: record.last_activity_ms,
        }
    }

    pub fn to_record(&self, touch_ts: bool) -> OpenDanAgentSessionRecord {
        let now = now_ms();
        let updated_at_ms = if touch_ts {
            now
        } else {
            self.updated_at_ms.max(self.created_at_ms)
        };
        let last_activity_ms = if touch_ts {
            now
        } else {
            self.last_activity_ms.max(self.created_at_ms)
        };

        let mut meta = match self.meta.clone() {
            Json::Object(map) => map,
            _ => Map::new(),
        };
        meta.insert(
            "runtime_state".to_string(),
            serde_json::to_value(self.runtime_meta()).unwrap_or_else(|_| json!({})),
        );

        OpenDanAgentSessionRecord {
            session_id: self.session_id.clone(),
            owner_agent: self.owner_agent.clone(),
            title: self.title.clone(),
            summary: self.summary.trim().to_string().if_empty_then(|| {
                self.last_step_summary
                    .as_ref()
                    .and_then(extract_step_summary_text)
                    .unwrap_or_default()
            }),
            status: self.record_status().to_string(),
            created_at_ms: self.created_at_ms,
            updated_at_ms,
            last_activity_ms,
            links: self.links.clone(),
            tags: self.tags.clone(),
            meta: Json::Object(meta),
        }
    }

    fn runtime_meta(&self) -> SessionRuntimeMeta {
        SessionRuntimeMeta {
            state: self.state,
            wait_details: self.wait_details.clone(),
            current_behavior: self.current_behavior.clone(),
            step_index: self.step_index,
            last_step_summary: self.last_step_summary.clone(),
            workspace_info: self.workspace_info.clone(),
            local_workspace_id: self.local_workspace_id.clone(),
            worklog: self.worklog.clone(),
            cost_trace: normalize_json_object(self.cost_trace.clone()),
            message_links: self.message_links.clone(),
        }
    }

    pub fn record_status(&self) -> &'static str {
        if self.state == SessionState::Pause {
            SESSION_STATUS_PAUSE
        } else {
            SESSION_STATUS_NORMAL
        }
    }

    pub fn update_state(&mut self, new_state: SessionState) {
        self.state = new_state;
        self.updated_at_ms = now_ms();
        if new_state == SessionState::Running {
            self.last_activity_ms = self.updated_at_ms;
        }
    }

    pub fn set_wait_state(
        &mut self,
        state: SessionState,
        wait_details: Option<SessionWaitDetails>,
    ) {
        self.state = state;
        self.wait_details = wait_details;
        self.updated_at_ms = now_ms();
    }

    pub fn append_msg(&mut self, payload: Json) -> String {
        let id = extract_input_id(&payload, "msg");
        let item = SessionInputItem {
            id: id.clone(),
            ts_ms: now_ms(),
            payload,
        };
        self.new_msgs.push(item);
        self.updated_at_ms = now_ms();
        id
    }

    pub fn append_event(&mut self, payload: Json) -> String {
        let id = extract_input_id(&payload, "event");
        let item = SessionInputItem {
            id: id.clone(),
            ts_ms: now_ms(),
            payload,
        };
        self.new_events.push(item);
        self.updated_at_ms = now_ms();
        id
    }

    pub fn append_msg_link(
        &mut self,
        msg_tunnle_message_id: impl Into<String>,
        msg_record_id: impl Into<String>,
        reason: impl Into<String>,
    ) -> String {
        let now = now_ms();
        let reason = reason.into();
        let link_id = new_link_id();
        self.message_links.push(SessionMessageLink {
            link_id: link_id.clone(),
            session_id: self.session_id.clone(),
            msg_tunnle_message_id: msg_tunnle_message_id.into(),
            msg_record_id: msg_record_id.into(),
            state: SessionMessageLinkState::New,
            reason: normalize_nonempty_reason(reason),
            created_at_ms: now,
        });
        self.updated_at_ms = now;
        self.last_activity_ms = now;
        link_id
    }

    pub fn append_msg_link_from_record(
        &mut self,
        record: &MsgRecordWithObject,
        reason: impl Into<String>,
    ) -> String {
        self.append_msg_link(
            record.record.msg_id.to_string(),
            record.record.record_id.clone(),
            reason.into(),
        )
    }

    pub fn mark_msg_arrived(&mut self, item: &SessionInputItem) {
        self.update_state_on_input_arrived(item, SessionState::WaitForMsg);
    }

    pub fn mark_event_arrived(&mut self, item: &SessionInputItem) {
        self.update_state_on_input_arrived(item, SessionState::WaitForEvent);
    }

    pub fn generate_input(&self, behavior_cfg: &BehaviorConfig) -> Option<SessionExecInput> {
        let slots = required_input_slots(behavior_cfg);
        let mut payload = Map::<String, Json>::new();
        let mut consumable = ConsumableIds::default();
        let mut link_ids = Vec::<String>::new();

        if slots.contains("new_msg") {
            let selected_links =
                select_new_links(&self.message_links, self.state, SessionState::WaitForMsg);
            if !selected_links.is_empty() {
                link_ids = selected_links
                    .iter()
                    .map(|item| item.link_id.clone())
                    .collect();
                payload.insert("new_msg".to_string(), select_link_payload(selected_links));
            } else {
                let selected = select_new_items(
                    &self.new_msgs,
                    self.state,
                    self.wait_details.as_ref(),
                    SessionState::WaitForMsg,
                );
                consumable.msg_ids = selected.iter().map(|item| item.id.clone()).collect();
                payload.insert("new_msg".to_string(), select_item_payload(selected));
            }
        }

        if slots.contains("new_event") {
            let selected = select_new_items(
                &self.new_events,
                self.state,
                self.wait_details.as_ref(),
                SessionState::WaitForEvent,
            );
            consumable.event_ids = selected.iter().map(|item| item.id.clone()).collect();
            payload.insert("new_event".to_string(), select_item_payload(selected));
        }

        if slots.contains("current_todo") {
            let current_todo = self
                .workspace_info
                .as_ref()
                .and_then(|ws| ws.get("current_todo"))
                .cloned()
                .unwrap_or(Json::Null);
            payload.insert("current_todo".to_string(), current_todo);
        }

        if slots.contains("last_step_summary") {
            payload.insert(
                "last_step_summary".to_string(),
                self.last_step_summary.clone().unwrap_or(Json::Null),
            );
        }

        if payload.values().all(is_null_like) {
            return None;
        }

        Some(SessionExecInput {
            session_id: self.session_id.clone(),
            behavior: self.current_behavior.clone().unwrap_or_default(),
            step_index: self.step_index,
            payload: Json::Object(payload),
            link_ids,
            consumable_ids: consumable,
        })
    }

    pub fn mark_input_links_used(&mut self, link_ids: &[String]) {
        if link_ids.is_empty() {
            return;
        }
        let consumed = link_ids
            .iter()
            .map(|item| item.as_str())
            .collect::<HashSet<_>>();
        let mut touched = false;
        for link in &mut self.message_links {
            if consumed.contains(link.link_id.as_str())
                && link.state == SessionMessageLinkState::New
            {
                link.state = SessionMessageLinkState::Used;
                touched = true;
            }
        }
        if touched {
            self.updated_at_ms = now_ms();
        }
    }

    pub fn update_input_used(&mut self, exec_input: &SessionExecInput) {
        self.mark_input_links_used(exec_input.link_ids.as_slice());

        if !exec_input.consumable_ids.msg_ids.is_empty() {
            let consumed = exec_input
                .consumable_ids
                .msg_ids
                .iter()
                .cloned()
                .collect::<HashSet<_>>();
            self.new_msgs
                .retain(|item| !consumed.contains(item.id.as_str()));
        }

        if !exec_input.consumable_ids.event_ids.is_empty() {
            let consumed = exec_input
                .consumable_ids
                .event_ids
                .iter()
                .cloned()
                .collect::<HashSet<_>>();
            self.new_events
                .retain(|item| !consumed.contains(item.id.as_str()));
        }
        self.updated_at_ms = now_ms();
    }

    pub fn append_worklog(&mut self, item: Json) {
        self.worklog.push(item);
        if self.worklog.len() > 256 {
            let start = self.worklog.len().saturating_sub(256);
            self.worklog = self.worklog.split_off(start);
        }
        self.updated_at_ms = now_ms();
    }

    pub fn set_last_step_summary(&mut self, summary: Json) {
        self.summary = extract_step_summary_text(&summary).unwrap_or_default();
        self.last_step_summary = Some(summary);
        self.updated_at_ms = now_ms();
        self.last_activity_ms = self.updated_at_ms;
    }

    pub fn should_ready_by_wait_timeout(&self, now_ms: u64) -> bool {
        if self.state != SessionState::WaitForMsg && self.state != SessionState::WaitForEvent {
            return false;
        }
        self.wait_details
            .as_ref()
            .and_then(|details| details.deadline_ms)
            .map(|deadline| now_ms >= deadline)
            .unwrap_or(false)
    }

    pub fn summary_view_json(&self) -> Json {
        json!({
            "session_id": self.session_id,
            "status": self.record_status(),
            "state": format!("{:?}", self.state).to_uppercase(),
            "title": self.title,
            "summary": self.summary,
            "current_behavior": self.current_behavior,
            "step_index": self.step_index,
            "updated_at_ms": self.updated_at_ms,
            "last_activity_ms": self.last_activity_ms,
            "new_msg_count": self.new_msgs.len(),
            "new_event_count": self.new_events.len(),
            "history_msg_count": self.history_msgs.len(),
            "history_event_count": self.history_events.len(),
            "new_link_count": self
                .message_links
                .iter()
                .filter(|item| item.state == SessionMessageLinkState::New)
                .count(),
            "workspace_info": self.workspace_info,
            "local_workspace_id": self.local_workspace_id,
            "meta": self.meta,
        })
    }

    fn update_state_on_input_arrived(&mut self, item: &SessionInputItem, wait_state: SessionState) {
        self.updated_at_ms = now_ms();
        if self.state == SessionState::Wait || self.state == SessionState::Sleep {
            self.state = SessionState::Ready;
            return;
        }
        if self.state == wait_state && match_wait_filter(item, self.wait_details.as_ref()) {
            self.state = SessionState::Ready;
        }
    }
}

#[derive(Clone, Debug)]
pub struct AgentSessionMgr {
    owner_agent: String,
    sessions_root: PathBuf,
    default_behavior: Option<String>,
    sessions: Arc<RwLock<HashMap<String, Arc<Mutex<AgentSession>>>>>,
    scheduler_lock: Arc<Mutex<()>>,
}

impl AgentSessionMgr {
    pub async fn new(
        owner_agent: impl Into<String>,
        sessions_root: impl Into<PathBuf>,
        default_behavior: Option<String>,
    ) -> Result<Self, ToolError> {
        let owner_agent = owner_agent.into();
        let sessions_root = sessions_root.into();

        fs::create_dir_all(&sessions_root).await.map_err(|err| {
            ToolError::ExecFailed(format!(
                "create sessions dir `{}` failed: {err}",
                sessions_root.display()
            ))
        })?;

        let store = Self {
            owner_agent,
            sessions_root,
            default_behavior,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            scheduler_lock: Arc::new(Mutex::new(())),
        };
        store.load_existing().await?;
        Ok(store)
    }

    pub fn sessions_root(&self) -> &Path {
        &self.sessions_root
    }

    pub fn get_default_session_id(target:&DID,tunnel_did:&DID) -> String {
        format!("session-{}-{}",target.to_raw_host_name(),tunnel_did.to_raw_host_name())
    }

    pub async fn ensure_default_session(&self) -> Result<Arc<Mutex<AgentSession>>, ToolError> {
        self.ensure_session("default", Some("Default Session".to_string()))
            .await
    }

    pub async fn ensure_session(
        &self,
        session_id: &str,
        title: Option<String>,
    ) -> Result<Arc<Mutex<AgentSession>>, ToolError> {
        let session_id = sanitize_session_id(session_id)?;
        if let Some(existing) = self.get_session(session_id.as_str()).await {
            return Ok(existing);
        }

        let mut session = AgentSession::new(
            session_id.clone(),
            self.owner_agent.clone(),
            self.default_behavior.as_deref(),
        );
        if let Some(title) = normalize_optional_string(title) {
            session.title = title;
        }
        let record = session.to_record(true);
        self.write_session_record(&record).await?;

        let session = Arc::new(Mutex::new(AgentSession::from_record(record)));
        self.sessions
            .write()
            .await
            .insert(session_id, session.clone());
        Ok(session)
    }

    pub async fn get_session(&self, session_id: &str) -> Option<Arc<Mutex<AgentSession>>> {
        let Ok(session_id) = sanitize_session_id(session_id) else {
            return None;
        };
        self.sessions.read().await.get(session_id.as_str()).cloned()
    }

    pub async fn append_msg(&self, session_id: &str, payload: Json) -> Result<String, ToolError> {
        let session = self.ensure_session(session_id, None).await?;
        let mut guard = session.lock().await;
        let id = guard.append_msg(payload);
        self.save_session_locked(&guard).await?;
        Ok(id)
    }

    pub async fn append_event(&self, session_id: &str, payload: Json) -> Result<String, ToolError> {
        let session = self.ensure_session(session_id, None).await?;
        let mut guard = session.lock().await;
        let id = guard.append_event(payload);
        self.save_session_locked(&guard).await?;
        Ok(id)
    }

    pub async fn link_msg_record(
        &self,
        session_id: &str,
        record: &MsgRecordWithObject,
        reason: &str,
    ) -> Result<String, ToolError> {
        let session = self.ensure_session(session_id, None).await?;
        let mut guard = session.lock().await;
        let link_id = guard.append_msg_link_from_record(record, reason.to_string());
        self.save_session_locked(&guard).await?;
        Ok(link_id)
    }

    pub async fn mark_msg_arrived(
        &self,
        session_id: &str,
        item: &MsgRecord,
    ) -> Result<(), ToolError> {
        let session = self.ensure_session(session_id, None).await?;
        let mut guard = session.lock().await;
        let payload = serde_json::to_value(item).map_err(|err| {
            ToolError::ExecFailed(format!(
                "serialize msg record to session input failed: {err}"
            ))
        })?;
        let input = SessionInputItem {
            id: extract_input_id(&payload, "msg"),
            ts_ms: now_ms(),
            payload,
        };
        guard.update_state_on_input_arrived(&input, SessionState::WaitForMsg);
        self.save_session_locked(&guard).await
    }

    pub async fn save_session(&self, session_id: &str) -> Result<(), ToolError> {
        let Some(session) = self.get_session(session_id).await else {
            return Err(ToolError::InvalidArgs(format!(
                "session not found: {session_id}"
            )));
        };
        let guard = session.lock().await;
        self.save_session_locked(&guard).await
    }

    pub async fn save_session_locked(&self, session: &AgentSession) -> Result<(), ToolError> {
        let record = session.to_record(true);
        self.write_session_record(&record).await
    }

    pub async fn schedule_wait_timeouts(&self, now_ms: u64) -> Result<(), ToolError> {
        let sessions = {
            let guard = self.sessions.read().await;
            guard.values().cloned().collect::<Vec<_>>()
        };

        for session in sessions {
            let mut guard = session.lock().await;
            if guard.should_ready_by_wait_timeout(now_ms) {
                guard.wait_details = None;
                guard.update_state(SessionState::Ready);
                self.save_session_locked(&guard).await?;
            }
        }
        Ok(())
    }

    pub async fn refresh_all_statuses_from_disk(&self) -> Result<(), ToolError> {
        let session_ids = {
            let guard = self.sessions.read().await;
            guard.keys().cloned().collect::<Vec<_>>()
        };
        for session_id in session_ids {
            self.refresh_status_from_disk(session_id.as_str()).await?;
        }
        Ok(())
    }

    pub async fn refresh_status_from_disk(&self, session_id: &str) -> Result<(), ToolError> {
        let session_id = sanitize_session_id(session_id)?;
        let path = self.session_file_path(session_id.as_str());
        if !is_existing_file(&path).await {
            return Ok(());
        }

        let raw = fs::read_to_string(&path).await.map_err(|err| {
            ToolError::ExecFailed(format!(
                "read session file `{}` failed: {err}",
                path.display()
            ))
        })?;
        let record = serde_json::from_str::<OpenDanAgentSessionRecord>(&raw).map_err(|err| {
            ToolError::ExecFailed(format!(
                "parse session file `{}` failed: {err}",
                path.display()
            ))
        })?;

        let Some(session) = self.get_session(session_id.as_str()).await else {
            self.sessions.write().await.insert(
                session_id.clone(),
                Arc::new(Mutex::new(AgentSession::from_record(record))),
            );
            return Ok(());
        };

        let mut guard = session.lock().await;
        let paused = matches!(
            record.status.trim().to_ascii_lowercase().as_str(),
            SESSION_STATUS_PAUSE
        );
        if paused {
            guard.state = SessionState::Pause;
        } else if guard.state == SessionState::Pause {
            guard.state = if !guard.new_msgs.is_empty() || !guard.new_events.is_empty() {
                SessionState::Ready
            } else {
                SessionState::Wait
            };
        }
        Ok(())
    }

    pub async fn get_next_ready_session(&self) -> Option<Arc<Mutex<AgentSession>>> {
        let _scheduler_guard = self.scheduler_lock.lock().await;
        let sessions = {
            let guard = self.sessions.read().await;
            guard.values().cloned().collect::<Vec<_>>()
        };

        let mut occupied_local_workspaces = HashSet::<String>::new();
        for session in &sessions {
            let guard = session.lock().await;
            if guard.state == SessionState::Running {
                if let Some(local_workspace_id) = guard
                    .local_workspace_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    occupied_local_workspaces.insert(local_workspace_id.to_string());
                }
            }
        }

        for session in sessions {
            let mut guard = session.lock().await;
            if guard.state == SessionState::Ready {
                if let Some(local_workspace_id) = guard
                    .local_workspace_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    if occupied_local_workspaces.contains(local_workspace_id) {
                        continue;
                    }
                    occupied_local_workspaces.insert(local_workspace_id.to_string());
                }
                guard.update_state(SessionState::Running);
                return Some(session.clone());
            }
        }
        None
    }

    pub async fn session_view(&self, session_id: &str) -> Result<Json, ToolError> {
        let session_id = sanitize_session_id(session_id)?;
        self.refresh_status_from_disk(session_id.as_str()).await?;
        let Some(session) = self.get_session(session_id.as_str()).await else {
            return Err(ToolError::InvalidArgs(format!(
                "session not found: {session_id}"
            )));
        };
        let guard = session.lock().await;
        Ok(guard.summary_view_json())
    }

    async fn load_existing(&self) -> Result<(), ToolError> {
        let mut read_dir = fs::read_dir(&self.sessions_root).await.map_err(|err| {
            ToolError::ExecFailed(format!(
                "read sessions dir `{}` failed: {err}",
                self.sessions_root.display()
            ))
        })?;

        while let Some(entry) = read_dir.next_entry().await.map_err(|err| {
            ToolError::ExecFailed(format!(
                "iterate sessions dir `{}` failed: {err}",
                self.sessions_root.display()
            ))
        })? {
            let path = entry.path();
            let Ok(file_type) = entry.file_type().await else {
                continue;
            };
            if !file_type.is_dir() {
                continue;
            }

            let Some(session_id) = entry
                .file_name()
                .to_str()
                .and_then(|name| sanitize_session_id(name).ok())
            else {
                continue;
            };

            let session_file = path.join(DEFAULT_SESSION_FILE);
            if !is_existing_file(&session_file).await {
                continue;
            }

            let raw = match fs::read_to_string(&session_file).await {
                Ok(raw) => raw,
                Err(err) => {
                    warn!(
                        "agent_session.load skip unreadable file: path={} err={}",
                        session_file.display(),
                        err
                    );
                    continue;
                }
            };

            let mut record = match serde_json::from_str::<OpenDanAgentSessionRecord>(&raw) {
                Ok(record) => record,
                Err(err) => {
                    warn!(
                        "agent_session.load skip invalid file: path={} err={}",
                        session_file.display(),
                        err
                    );
                    continue;
                }
            };
            record.session_id = session_id.clone();
            if record.owner_agent.trim().is_empty() {
                record.owner_agent = self.owner_agent.clone();
            }

            self.sessions.write().await.insert(
                session_id,
                Arc::new(Mutex::new(AgentSession::from_record(record))),
            );
        }

        if self.sessions.read().await.is_empty() {
            let _ = self.ensure_default_session().await?;
        }
        Ok(())
    }

    async fn write_session_record(
        &self,
        record: &OpenDanAgentSessionRecord,
    ) -> Result<(), ToolError> {
        let session_id = sanitize_session_id(record.session_id.as_str())?;
        let session_dir = self.sessions_root.join(session_id.as_str());
        fs::create_dir_all(&session_dir).await.map_err(|err| {
            ToolError::ExecFailed(format!(
                "create session dir `{}` failed: {err}",
                session_dir.display()
            ))
        })?;

        let session_file = session_dir.join(DEFAULT_SESSION_FILE);
        let bytes = serde_json::to_vec_pretty(record).map_err(|err| {
            ToolError::ExecFailed(format!("serialize session record failed: {err}"))
        })?;
        fs::write(&session_file, bytes).await.map_err(|err| {
            ToolError::ExecFailed(format!(
                "write session file `{}` failed: {err}",
                session_file.display()
            ))
        })
    }

    fn session_file_path(&self, session_id: &str) -> PathBuf {
        self.sessions_root
            .join(session_id)
            .join(DEFAULT_SESSION_FILE)
    }
}

#[derive(Clone)]
pub struct GetSessionTool {
    store: Arc<AgentSessionMgr>,
}

impl GetSessionTool {
    pub fn new(store: Arc<AgentSessionMgr>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl AgentTool for GetSessionTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_GET_SESSION.to_string(),
            description:
                "Read current session state and status. Used by runtime before each LLM round."
                    .to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" }
                },
                "required": ["session_id"],
                "additionalProperties": false
            }),
            output_schema: json!({
                "type": "object",
                "properties": {
                    "ok": { "type": "boolean" },
                    "session": { "type": "object" }
                }
            }),
        }
    }

    async fn call(&self, _ctx: &TraceCtx, args: Json) -> Result<Json, ToolError> {
        let session_id = require_string(&args, "session_id")?;
        let session = self.store.session_view(&session_id).await?;
        Ok(json!({
            "ok": true,
            "session": session
        }))
    }
}

fn required_input_slots(cfg: &BehaviorConfig) -> HashSet<&'static str> {
    let mut slots = HashSet::<&'static str>::new();
    for placeholder in extract_placeholders(cfg.input.as_str()) {
        match placeholder.as_str() {
            "new_msg" => {
                slots.insert("new_msg");
            }
            "new_event" => {
                slots.insert("new_event");
            }
            "current_todo" | "current_todo_details" => {
                slots.insert("current_todo");
            }
            "last_step_summary" | "session_summary" => {
                slots.insert("last_step_summary");
            }
            _ => {}
        }
    }

    if slots.is_empty() {
        slots.insert("new_msg");
        slots.insert("new_event");
    }
    slots
}

fn extract_placeholders(template: &str) -> Vec<String> {
    let mut out = Vec::<String>::new();
    let mut cursor = 0usize;
    while let Some(open) = template[cursor..].find("{{").map(|idx| idx + cursor) {
        let start = open + 2;
        let Some(close) = template[start..].find("}}").map(|idx| idx + start) else {
            break;
        };
        let placeholder = template[start..close].trim();
        if !placeholder.is_empty() {
            out.push(placeholder.to_string());
        }
        cursor = close + 2;
    }
    out
}

fn select_new_items(
    input: &[SessionInputItem],
    current_state: SessionState,
    wait_details: Option<&SessionWaitDetails>,
    wait_state: SessionState,
) -> Vec<SessionInputItem> {
    if current_state != wait_state {
        return input.to_vec();
    }
    input
        .iter()
        .filter(|item| match_wait_filter(item, wait_details))
        .cloned()
        .collect()
}

fn select_new_links(
    input: &[SessionMessageLink],
    current_state: SessionState,
    wait_state: SessionState,
) -> Vec<SessionMessageLink> {
    if current_state != wait_state {
        return input
            .iter()
            .filter(|item| item.state == SessionMessageLinkState::New)
            .cloned()
            .collect();
    }

    input
        .iter()
        .filter(|item| item.state == SessionMessageLinkState::New)
        .cloned()
        .collect()
}

fn select_item_payload(items: Vec<SessionInputItem>) -> Json {
    if items.is_empty() {
        return Json::Null;
    }
    Json::Array(
        items
            .into_iter()
            .map(|item| {
                json!({
                    "id": item.id,
                    "ts_ms": item.ts_ms,
                    "payload": item.payload,
                })
            })
            .collect::<Vec<_>>(),
    )
}

fn select_link_payload(items: Vec<SessionMessageLink>) -> Json {
    if items.is_empty() {
        return Json::Null;
    }
    Json::Array(
        items
            .into_iter()
            .map(|item| {
                json!({
                    "link_id": item.link_id,
                    "session_id": item.session_id,
                    "msg_tunnle_message_id": item.msg_tunnle_message_id,
                    "msg_record_id": item.msg_record_id,
                    "state": format!("{:?}", item.state).to_uppercase(),
                    "reason": item.reason,
                    "created_at_ms": item.created_at_ms,
                })
            })
            .collect::<Vec<_>>(),
    )
}

fn match_wait_filter(item: &SessionInputItem, wait_details: Option<&SessionWaitDetails>) -> bool {
    let Some(wait_details) = wait_details else {
        return true;
    };
    let Some(filter_obj) = wait_details.filter.as_object() else {
        return true;
    };

    for (key, expected) in filter_obj {
        if key == "id" {
            if Json::String(item.id.clone()) != *expected {
                return false;
            }
            continue;
        }

        if key.starts_with('/') {
            let actual = item.payload.pointer(key).cloned().unwrap_or(Json::Null);
            if actual != *expected {
                return false;
            }
            continue;
        }

        let actual = item.payload.get(key).cloned().unwrap_or(Json::Null);
        if actual != *expected {
            return false;
        }
    }
    true
}

fn is_null_like(value: &Json) -> bool {
    match value {
        Json::Null => true,
        Json::String(v) => v.trim().is_empty(),
        Json::Array(v) => v.is_empty(),
        Json::Object(v) => v.is_empty(),
        _ => false,
    }
}

fn extract_step_summary_text(summary: &Json) -> Option<String> {
    if let Some(text) = summary
        .get("summary")
        .or_else(|| summary.get("message"))
        .and_then(|value| value.as_str())
    {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    if let Some(text) = summary
        .get("llm")
        .and_then(|value| value.get("next_behavior"))
        .and_then(|value| value.as_str())
    {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            return Some(format!("next_behavior={trimmed}"));
        }
    }
    None
}

fn parse_runtime_meta(meta: &Json) -> SessionRuntimeMeta {
    meta.get("runtime_state")
        .cloned()
        .and_then(|value| serde_json::from_value::<SessionRuntimeMeta>(value).ok())
        .unwrap_or_default()
}

fn sanitize_session_id(input: &str) -> Result<String, ToolError> {
    let session_id = input.trim();
    if session_id.is_empty() {
        return Err(ToolError::InvalidArgs(
            "session_id cannot be empty".to_string(),
        ));
    }
    if session_id.len() > MAX_SESSION_ID_LEN {
        return Err(ToolError::InvalidArgs(format!(
            "session_id too long (>{MAX_SESSION_ID_LEN})"
        )));
    }
    if session_id == "." || session_id == ".." {
        return Err(ToolError::InvalidArgs(
            "session_id cannot be `.` or `..`".to_string(),
        ));
    }
    if session_id.contains('/') || session_id.contains('\\') {
        return Err(ToolError::InvalidArgs(
            "session_id cannot contain path separators".to_string(),
        ));
    }
    if session_id.chars().any(|ch| ch.is_control()) {
        return Err(ToolError::InvalidArgs(
            "session_id cannot contain control characters".to_string(),
        ));
    }
    Ok(session_id.to_string())
}

fn extract_input_id(payload: &Json, prefix: &str) -> String {
    for key in ["id", "msg_id", "event_id", "record_id"] {
        if let Some(id) = payload
            .get(key)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return id.to_string();
        }
    }

    let seq = SESSION_ITEM_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{}-{seq}", now_ms())
}

fn new_link_id() -> String {
    let seq = SESSION_LINK_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("link-{}-{seq}", now_ms())
}

fn normalize_nonempty_reason(reason: String) -> String {
    let trimmed = reason.trim();
    if trimmed.is_empty() {
        "INBOX_FALLBACK".to_string()
    } else {
        trimmed.to_string()
    }
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
}

fn normalize_json_object(value: Json) -> Json {
    if value.is_object() {
        return value;
    }
    json!({})
}

async fn is_existing_file(path: &Path) -> bool {
    fs::metadata(path)
        .await
        .map(|meta| meta.is_file())
        .unwrap_or(false)
}

fn require_string(args: &Json, key: &str) -> Result<String, ToolError> {
    args.get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| ToolError::InvalidArgs(format!("missing `{key}`")))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

trait EmptyFallback {
    fn if_empty_then<F>(self, fallback: F) -> String
    where
        F: FnOnce() -> String;
}

impl EmptyFallback for String {
    fn if_empty_then<F>(self, fallback: F) -> String
    where
        F: FnOnce() -> String,
    {
        if self.trim().is_empty() {
            fallback()
        } else {
            self
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn behavior_with_input(input: &str) -> BehaviorConfig {
        BehaviorConfig {
            name: "test".to_string(),
            process_rule: "rule".to_string(),
            input: input.to_string(),
            ..BehaviorConfig::default()
        }
    }

    #[test]
    fn generate_input_returns_none_when_all_slots_empty() {
        let mut session = AgentSession::new("s1", "did:opendan:test", Some("on_wakeup"));
        session.last_step_summary = None;
        let cfg = behavior_with_input("{{new_msg}}\n{{new_event}}\n{{last_step_summary}}");

        let got = session.generate_input(&cfg);
        assert!(got.is_none());
    }

    #[test]
    fn generate_input_includes_new_msg_and_consumable_ids() {
        let mut session = AgentSession::new("s1", "did:opendan:test", Some("on_wakeup"));
        session.append_msg(json!({"id":"msg-1","text":"hello"}));
        let cfg = behavior_with_input("{{new_msg}}");

        let got = session.generate_input(&cfg).expect("input exists");
        assert_eq!(got.consumable_ids.msg_ids, vec!["msg-1".to_string()]);
        assert!(got.payload.get("new_msg").is_some());
    }

    #[test]
    fn generate_input_prefers_new_link_and_sets_link_ids() {
        let mut session = AgentSession::new("s1", "did:opendan:test", Some("on_wakeup"));
        let link_id = session.append_msg_link("msg-obj-1", "record-1", "INBOX_FALLBACK");
        let cfg = behavior_with_input("{{new_msg}}");

        let got = session.generate_input(&cfg).expect("input exists");
        assert_eq!(got.link_ids, vec![link_id]);
        assert!(got.consumable_ids.msg_ids.is_empty());
        assert!(got.payload.get("new_msg").is_some());
    }

    #[test]
    fn update_input_used_drops_consumed_items() {
        let mut session = AgentSession::new("s1", "did:opendan:test", Some("on_wakeup"));
        session.append_msg(json!({"id":"msg-1","text":"hello"}));
        let cfg = behavior_with_input("{{new_msg}}");
        let exec_input = session.generate_input(&cfg).expect("input exists");

        session.update_input_used(&exec_input);
        assert!(session.new_msgs.is_empty());
        assert!(session.history_msgs.is_empty());
    }

    #[test]
    fn mark_input_links_used_updates_state_to_used() {
        let mut session = AgentSession::new("s1", "did:opendan:test", Some("on_wakeup"));
        let link_id = session.append_msg_link("msg-obj-1", "record-1", "INBOX_FALLBACK");

        session.mark_input_links_used(&[link_id]);
        assert_eq!(session.message_links.len(), 1);
        assert_eq!(
            session.message_links[0].state,
            SessionMessageLinkState::Used
        );
    }

    #[test]
    fn wait_for_msg_filters_non_matching_inputs() {
        let mut session = AgentSession::new("s1", "did:opendan:test", Some("on_wakeup"));
        session.state = SessionState::WaitForMsg;
        session.wait_details = Some(SessionWaitDetails {
            filter: json!({"source":"owner"}),
            deadline_ms: None,
            note: None,
        });
        session.append_msg(json!({"id":"msg-1","source":"user"}));
        let cfg = behavior_with_input("{{new_msg}}");

        assert!(session.generate_input(&cfg).is_none());
    }
}
