use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use buckyos_api::msg_queue::Message;
use buckyos_api::{
    get_buckyos_api_runtime, MsgRecord, MsgRecordWithObject, OpenDanAgentSessionRecord,
    OpenDanSessionLink,
};
use log::{debug, info, warn};
use name_lib::DID;
use ndn_lib::MsgObject;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value as Json};
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, Notify, RwLock};

use crate::agent_tool::{AgentTool, AgentToolError, ToolSpec, TOOL_GET_SESSION};
use crate::behavior::SessionRuntimeContext;

const DEFAULT_SESSION_FILE: &str = "session.json";
const DEFAULT_MSG_RECORD_FILE: &str = "msg_record.jsonl";
const MAX_SESSION_ID_LEN: usize = 180;
const WORK_SESSION_PREFIX: &str = "work-";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SessionState {
    Wait,         //运行中，等待任意触发
    WaitForMsg,   //运行中，等待特定触发
    WaitForEvent, //运行中，等待特定触发

    Ready,   //已经触发，等待调度成Running
    Running, //正在运行中

    End, //结束，再次触发会从Default behavior中唤醒
}

impl ToString for SessionState {
    fn to_string(&self) -> String {
        match self {
            SessionState::Wait => "wait".to_string(),
            SessionState::WaitForMsg => "wait_for_msg".to_string(),
            SessionState::WaitForEvent => "wait_for_event".to_string(),
            SessionState::Ready => "ready".to_string(),
            SessionState::Running => "running".to_string(),
            SessionState::End => "end".to_string(),
        }
    }
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SessionInputItem {
    pub msg: Option<MsgRecord>,
    pub event_id: Option<String>,
}

impl Default for SessionInputItem {
    fn default() -> Self {
        Self {
            msg: None,
            event_id: None,
        }
    }
}

//下面结构定义了会被序列化的状态
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
struct SessionRuntimeState {
    state: SessionState,
    is_paused: bool,

    //TODO 简化成wait特定的object_id + event_id （比如等待授权任务的task)
    wait_details: Option<SessionWaitDetails>,
    current_behavior: Option<String>,
    //reply的默认对象
    default_remote: Option<String>,

    step_index: u32,
    last_step_summary: Option<Json>,

    workspace_info: Option<Json>,
    local_workspace_id: Option<String>,
    worklog: Vec<Json>,

    loaded_skills: Vec<String>,
    allow_tools: Vec<String>,
    cost_trace: Json,
}

impl Default for SessionRuntimeState {
    fn default() -> Self {
        Self {
            state: SessionState::Wait,
            is_paused: false,
            wait_details: None,
            current_behavior: None,
            default_remote: None,
            step_index: 0,
            last_step_summary: None,
            workspace_info: None,
            local_workspace_id: None,
            worklog: vec![],
            loaded_skills: vec![],
            allow_tools: vec![],
            cost_trace: json!({}),
        }
    }
}

#[derive(Clone, Debug)]
pub struct AgentSession {
    pub session_id: String,
    pub owner_agent: String,
    pub title: String,
    pub summary: String,
    pub links: Vec<OpenDanSessionLink>,
    pub tags: Vec<String>,
    pub meta: Json,

    pub last_step_summary: Option<String>,
    pub is_paused: bool,
    pub state: SessionState,
    pub wait_details: Option<SessionWaitDetails>,
    pub current_behavior: String,
    pub default_remote: Option<String>,
    pub step_index: u32,

    pub msg_kmsgqueue_curosr: u64,
    pub event_kmsgqueue_curosr: u64,
    //这个不会被序列化
    pub just_readed_input_msg: Vec<Vec<u8>>,

    pub cwd: PathBuf,
    pub session_root_dir: PathBuf,
    pub workspace_info: Option<Json>,
    pub local_workspace_id: Option<String>,
    pub worklog: Vec<Json>,

    pub loaded_skills: Vec<String>,
    pub loaded_tools: Vec<String>,

    pub cost_trace: Json,
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
        let current_behavior =
            normalize_optional_string(default_behavior.map(str::to_string)).unwrap_or_default();

        Self {
            title: format!("Session {}", session_id),
            summary: String::new(),
            session_id,
            owner_agent: owner_agent.into(),
            is_paused: false,
            state: SessionState::Wait,
            wait_details: None,
            current_behavior,
            default_remote: None,
            step_index: 0,
            last_step_summary: None,
            msg_kmsgqueue_curosr: 0,
            event_kmsgqueue_curosr: 0,
            just_readed_input_msg: vec![],
            cwd: PathBuf::new(),
            session_root_dir: PathBuf::new(),
            workspace_info: None,
            local_workspace_id: None,
            worklog: vec![],
            loaded_skills: vec![],
            loaded_tools: vec![],
            cost_trace: json!({}),
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
        let state = runtime_meta.state;
        let is_paused = runtime_meta.is_paused;

        let mut meta = record.meta.clone();
        if let Some(map) = meta.as_object_mut() {
            map.remove("runtime_state");
        }
        let mut summary = record.summary;
        let runtime_last_step_summary = runtime_meta
            .last_step_summary
            .as_ref()
            .and_then(extract_step_summary_text);
        if summary.trim().is_empty() {
            summary = runtime_last_step_summary.clone().unwrap_or_default();
        }

        Self {
            session_id: record.session_id,
            cwd: PathBuf::new(),
            session_root_dir: PathBuf::new(),
            owner_agent: record.owner_agent,
            title: if record.title.trim().is_empty() {
                "Untitled Session".to_string()
            } else {
                record.title
            },
            summary,
            is_paused,
            state,
            wait_details: runtime_meta.wait_details,
            current_behavior: normalize_optional_string(runtime_meta.current_behavior)
                .unwrap_or_default(),
            default_remote: normalize_optional_string(runtime_meta.default_remote),
            step_index: runtime_meta.step_index,
            last_step_summary: runtime_last_step_summary,
            msg_kmsgqueue_curosr: 0,
            event_kmsgqueue_curosr: 0,
            just_readed_input_msg: vec![],
            workspace_info: runtime_meta.workspace_info,
            local_workspace_id: normalize_optional_string(runtime_meta.local_workspace_id),
            worklog: runtime_meta.worklog,
            loaded_skills: runtime_meta.loaded_skills,
            loaded_tools: runtime_meta.allow_tools,
            cost_trace: normalize_json_object(runtime_meta.cost_trace),
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
            summary: self
                .summary
                .trim()
                .to_string()
                .if_empty_then(|| self.last_step_summary.clone().unwrap_or_default()),
            status: self.state.to_string(),
            created_at_ms: self.created_at_ms,
            updated_at_ms,
            last_activity_ms,
            links: self.links.clone(),
            tags: self.tags.clone(),
            meta: Json::Object(meta),
        }
    }

    fn runtime_meta(&self) -> SessionRuntimeState {
        SessionRuntimeState {
            state: self.state,
            is_paused: self.is_paused,
            wait_details: self.wait_details.clone(),
            current_behavior: normalize_optional_string(Some(self.current_behavior.clone())),
            default_remote: self.default_remote.clone(),
            step_index: self.step_index,
            last_step_summary: self.last_step_summary.clone().map(Json::String),
            workspace_info: self.workspace_info.clone(),
            local_workspace_id: self.local_workspace_id.clone(),
            worklog: self.worklog.clone(),
            loaded_skills: self.loaded_skills.clone(),
            allow_tools: self.loaded_tools.clone(),
            cost_trace: normalize_json_object(self.cost_trace.clone()),
        }
    }

    pub fn mark_msg_arrived(&mut self, item: &SessionInputItem) {
        self.update_state_on_input_arrived(item);
    }

    pub fn mark_event_arrived(&mut self, item: &SessionInputItem) {
        self.update_state_on_input_arrived(item);
    }

    pub fn mark_input_links_used(&mut self, link_ids: &[String]) {
        let _ = link_ids;
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
        self.last_step_summary = normalize_optional_string(Some(self.summary.clone()));
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
            "status": self.state.to_string(),
            "state": format!("{:?}", self.state).to_uppercase(),
            "title": self.title,
            "summary": self.summary,
            "current_behavior": self.current_behavior,
            "default_remote": self.default_remote.clone(),
            "step_index": self.step_index,
            "updated_at_ms": self.updated_at_ms,
            "last_activity_ms": self.last_activity_ms,
            "new_msg_count": 0,
            "new_event_count": 0,
            "history_msg_count": 0,
            "history_event_count": 0,
            "new_link_count": 0,
            "workspace_info": self.workspace_info,
            "local_workspace_id": self.local_workspace_id,
            "meta": self.meta,
        })
    }

    fn update_state_on_input_arrived(&mut self, item: &SessionInputItem) {
        if self.state == SessionState::Running {
            return;
        }

        if self.state == SessionState::WaitForMsg && item.msg.is_some() {
            self.updated_at_ms = now_ms();
            self.state = SessionState::Ready;
            info!(
                "{} will wakeup session:{} from WaitForMsg",
                self.owner_agent, self.session_id
            );
            return;
        }
        if self.state == SessionState::WaitForEvent && item.event_id.is_some() {
            self.updated_at_ms = now_ms();
            self.state = SessionState::Ready;
            info!(
                "{} will wakeup session:{} from WaitForEvent",
                self.owner_agent, self.session_id
            );
            return;
        }
        if self.state == SessionState::Wait || self.state == SessionState::End {
            self.updated_at_ms = now_ms();
            self.state = SessionState::Ready;
            debug!(
                "{} will wakeup session:{} by input",
                self.owner_agent, self.session_id
            );
            return;
        }
        return;
    }

    pub async fn pull_new_msg_from_kmsgqueue(
        kmsg_queue_id: &str,
        max_length: u32,
    ) -> Result<Vec<Message>, kRPC::RPCErrors> {
        let sub_id = kmsg_queue_id.trim();
        if sub_id.is_empty() {
            return Err(kRPC::RPCErrors::ReasonError(
                "kmsg_queue_id cannot be empty".to_string(),
            ));
        }
        if max_length == 0 {
            return Ok(vec![]);
        }

        let buckyos_runtime = get_buckyos_api_runtime()?;
        let kmsg_client = buckyos_runtime.get_msg_queue_client().await?;
        let length = (max_length as usize).min(4096);
        kmsg_client.fetch_messages(sub_id, length, false).await
    }

    pub async fn append_msg_record(
        session_id: &str,
        msg_record: MsgRecord,
        msg_obj: MsgObject,
    ) -> Result<(), AgentToolError> {
        let raw_session = session_id.trim();
        if raw_session.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "session_id cannot be empty".to_string(),
            ));
        }

        let session_dir = if raw_session.contains('/') || raw_session.contains('\\') {
            PathBuf::from(raw_session)
        } else {
            let session_id = sanitize_session_id(raw_session)?;
            let default_root = PathBuf::from("session");
            if fs::metadata(&default_root)
                .await
                .map(|meta| meta.is_dir())
                .unwrap_or(false)
            {
                default_root.join(session_id)
            } else {
                PathBuf::from(session_id)
            }
        };

        if !is_existing_dir(&session_dir).await {
            info!(
                "agent.persist_entity_prepare: kind=session_msg_record_dir session_id={} path={}",
                raw_session,
                session_dir.display()
            );
        }
        fs::create_dir_all(&session_dir).await.map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "create session dir `{}` failed: {err}",
                session_dir.display()
            ))
        })?;

        let msg_record_with_obj = MsgRecordWithObject {
            record: msg_record,
            msg: Some(msg_obj),
        };
        let json_str = serde_json::to_string(&msg_record_with_obj).map_err(|err| {
            AgentToolError::ExecFailed(format!("serialize msg record failed: {err}"))
        })?;

        let msg_record_path = session_dir.join(DEFAULT_MSG_RECORD_FILE);
        if !is_existing_file(&msg_record_path).await {
            info!(
                "agent.persist_entity_prepare: kind=session_msg_record_file session_id={} path={}",
                raw_session,
                msg_record_path.display()
            );
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&msg_record_path)
            .await
            .map_err(|err| {
                AgentToolError::ExecFailed(format!(
                    "open msg record file `{}` for append failed: {err}",
                    msg_record_path.display()
                ))
            })?;

        file.write_all(json_str.as_bytes()).await.map_err(|err| {
            AgentToolError::ExecFailed(format!("append msg record line failed: {err}"))
        })?;
        file.write_all(b"\n").await.map_err(|err| {
            AgentToolError::ExecFailed(format!("append msg record newline failed: {err}"))
        })?;
        file.flush().await.map_err(|err| {
            AgentToolError::ExecFailed(format!("flush msg record file failed: {err}"))
        })?;
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct AgentSessionMgr {
    owner_agent: String,
    sessions_root: PathBuf,
    sessions: Arc<RwLock<HashMap<String, Arc<Mutex<AgentSession>>>>>,
    scheduler_lock: Arc<Mutex<()>>,
    ready_notify: Arc<Notify>,
}

impl AgentSessionMgr {
    pub async fn new(
        owner_agent: impl Into<String>,
        sessions_root: impl Into<PathBuf>,
        _default_behavior: String,
    ) -> Result<Self, AgentToolError> {
        let owner_agent = owner_agent.into();
        let sessions_root = sessions_root.into();

        if !is_existing_dir(&sessions_root).await {
            info!(
                "agent.persist_entity_prepare: kind=sessions_root owner_agent={} path={}",
                owner_agent,
                sessions_root.display()
            );
        }
        fs::create_dir_all(&sessions_root).await.map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "create sessions dir `{}` failed: {err}",
                sessions_root.display()
            ))
        })?;

        let store = Self {
            owner_agent,
            sessions_root,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            scheduler_lock: Arc::new(Mutex::new(())),
            ready_notify: Arc::new(Notify::new()),
        };
        store.load_existing().await?;
        Ok(store)
    }

    pub fn sessions_root(&self) -> &Path {
        &self.sessions_root
    }

    fn workspace_root(&self) -> PathBuf {
        self.sessions_root
            .parent()
            .map(|path| path.to_path_buf())
            .unwrap_or_else(|| self.sessions_root.clone())
    }

    fn hydrate_session_runtime_context(&self, session: &mut AgentSession) {
        if session.cwd.as_os_str().is_empty() {
            session.cwd = self.workspace_root();
        }
        if session.session_root_dir.as_os_str().is_empty() {
            session.session_root_dir = self.sessions_root.clone();
        }
    }

    pub fn is_ui_session(session_id: &str) -> bool {
        !session_id.starts_with(WORK_SESSION_PREFIX)
    }

    pub fn get_ui_session_id(&self, target: &DID, ui_msg_tunnel_id: &str) -> String {
        format!(
            "ui-{}-{}-{}",
            self.owner_agent.as_str(),
            target.to_raw_host_name(),
            ui_msg_tunnel_id
        )
    }

    pub async fn ensure_session(
        &self,
        session_id: &str,
        title: Option<String>,
        behavior: Option<&str>,
        default_remote: Option<&str>,
    ) -> Result<Arc<Mutex<AgentSession>>, AgentToolError> {
        let session_id = sanitize_session_id(session_id)?;
        if let Some(existing) = self.get_session(session_id.as_str()).await {
            let mut guard = existing.lock().await;
            self.hydrate_session_runtime_context(&mut guard);
            drop(guard);
            return Ok(existing);
        }
        info!(
            "agent.persist_entity_prepare: kind=session_entity owner_agent={} session_id={}",
            self.owner_agent, session_id
        );

        let behavior = behavior.map(str::trim).filter(|value| !value.is_empty());

        let mut session = AgentSession::new(session_id.clone(), self.owner_agent.clone(), behavior);
        self.hydrate_session_runtime_context(&mut session);
        if let Some(title) = normalize_optional_string(title) {
            session.title = title;
        }
        session.default_remote = normalize_optional_string(default_remote.map(str::to_string));
        let record = session.to_record(true);
        self.write_session_record(&record).await?;

        let mut session_runtime = AgentSession::from_record(record);
        self.hydrate_session_runtime_context(&mut session_runtime);
        let session = Arc::new(Mutex::new(session_runtime));
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

    pub async fn try_wakeup_session_by_input_item(
        &self,
        session_id: &str,
        input_item: &SessionInputItem,
    ) -> Result<(), AgentToolError> {
        let default_remote = input_item.msg.as_ref().map(|msg| msg.from.to_string());
        let session = self
            .ensure_session(session_id, None, None, default_remote.as_deref())
            .await?;

        let mut guard = session.lock().await;
        guard.update_state_on_input_arrived(input_item);
        info!(
            "agent.session_try_wakeup: session_id={} state={:?}",
            guard.session_id, guard.state
        );

        self.ready_notify.notify_one();
        Ok(())
    }

    pub async fn save_session(&self, session_id: &str) -> Result<(), AgentToolError> {
        let Some(session) = self.get_session(session_id).await else {
            return Err(AgentToolError::InvalidArgs(format!(
                "session not found: {session_id}"
            )));
        };
        let record = {
            let guard = session.lock().await;
            guard.to_record(true)
        };
        self.write_session_record(&record).await
    }

    pub async fn save_session_locked(&self, session: &AgentSession) -> Result<(), AgentToolError> {
        let record = session.to_record(true);
        self.write_session_record(&record).await
    }

    pub async fn schedule_wait_timeouts(&self, now_ms: u64) -> Result<(), AgentToolError> {
        let sessions = {
            let guard = self.sessions.read().await;
            guard.values().cloned().collect::<Vec<_>>()
        };
        let mut woke_any = false;

        for session in sessions {
            let record = {
                let mut guard = session.lock().await;
                if guard.should_ready_by_wait_timeout(now_ms) {
                    guard.wait_details = None;
                    guard.state = SessionState::Ready;
                    Some(guard.to_record(true))
                } else {
                    None
                }
            };
            if let Some(record) = record {
                self.write_session_record(&record).await?;
                woke_any = true;
            }
        }
        if woke_any {
            self.ready_notify.notify_one();
        }
        Ok(())
    }

    pub async fn refresh_all_statuses_from_disk(&self) -> Result<(), AgentToolError> {
        let session_ids = {
            let guard = self.sessions.read().await;
            guard.keys().cloned().collect::<Vec<_>>()
        };
        for session_id in session_ids {
            self.refresh_status_from_disk(session_id.as_str()).await?;
        }
        Ok(())
    }

    pub async fn refresh_status_from_disk(&self, session_id: &str) -> Result<(), AgentToolError> {
        let session_id = sanitize_session_id(session_id)?;
        let path = self.session_file_path(session_id.as_str());
        if !is_existing_file(&path).await {
            return Ok(());
        }

        let raw = fs::read_to_string(&path).await.map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "read session file `{}` failed: {err}",
                path.display()
            ))
        })?;
        let record = serde_json::from_str::<OpenDanAgentSessionRecord>(&raw).map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "parse session file `{}` failed: {err}",
                path.display()
            ))
        })?;

        let Some(_session) = self.get_session(session_id.as_str()).await else {
            let mut runtime = AgentSession::from_record(record);
            self.hydrate_session_runtime_context(&mut runtime);
            self.sessions
                .write()
                .await
                .insert(session_id.clone(), Arc::new(Mutex::new(runtime)));
            return Ok(());
        };

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
                guard.state = SessionState::Running;
                return Some(session.clone());
            }
        }
        None
    }

    pub async fn wait_for_ready_or_timeout(&self, timeout: std::time::Duration) -> bool {
        tokio::time::timeout(timeout, self.ready_notify.notified())
            .await
            .is_ok()
    }

    pub async fn session_view(&self, session_id: &str) -> Result<Json, AgentToolError> {
        let session_id = sanitize_session_id(session_id)?;
        self.refresh_status_from_disk(session_id.as_str()).await?;
        let Some(session) = self.get_session(session_id.as_str()).await else {
            return Err(AgentToolError::InvalidArgs(format!(
                "session not found: {session_id}"
            )));
        };
        let guard = session.lock().await;
        Ok(guard.summary_view_json())
    }

    async fn load_existing(&self) -> Result<(), AgentToolError> {
        let mut read_dir = fs::read_dir(&self.sessions_root).await.map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "read sessions dir `{}` failed: {err}",
                self.sessions_root.display()
            ))
        })?;

        while let Some(entry) = read_dir.next_entry().await.map_err(|err| {
            AgentToolError::ExecFailed(format!(
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
            } else if record.owner_agent != self.owner_agent {
                warn!(
                    "agent_session.load normalize owner_agent: session={} owner_agent={} normalized={}",
                    session_id, record.owner_agent, self.owner_agent
                );
                record.owner_agent = self.owner_agent.clone();
            }

            self.sessions.write().await.insert(session_id, {
                let mut runtime = AgentSession::from_record(record);
                self.hydrate_session_runtime_context(&mut runtime);
                Arc::new(Mutex::new(runtime))
            });
        }

        Ok(())
    }

    async fn write_session_record(
        &self,
        record: &OpenDanAgentSessionRecord,
    ) -> Result<(), AgentToolError> {
        let session_id = sanitize_session_id(record.session_id.as_str())?;
        let session_dir = self.sessions_root.join(session_id.as_str());
        if !is_existing_dir(&session_dir).await {
            info!(
                "agent.persist_entity_prepare: kind=session_dir session_id={} path={}",
                session_id,
                session_dir.display()
            );
        }
        fs::create_dir_all(&session_dir).await.map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "create session dir `{}` failed: {err}",
                session_dir.display()
            ))
        })?;

        let session_file = session_dir.join(DEFAULT_SESSION_FILE);
        if !is_existing_file(&session_file).await {
            info!(
                "agent.persist_entity_prepare: kind=session_file session_id={} path={}",
                session_id,
                session_file.display()
            );
        }
        let bytes = serde_json::to_vec_pretty(record).map_err(|err| {
            AgentToolError::ExecFailed(format!("serialize session record failed: {err}"))
        })?;
        fs::write(&session_file, bytes).await.map_err(|err| {
            AgentToolError::ExecFailed(format!(
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
            usage: None,
        }
    }

    fn support_bash(&self) -> bool {
        true
    }
    fn support_action(&self) -> bool {
        false
    }
    fn support_llm_tool_call(&self) -> bool {
        false
    }

    async fn call(&self, _ctx: &SessionRuntimeContext, args: Json) -> Result<Json, AgentToolError> {
        let session_id = require_string(&args, "session_id")?;
        let session = self.store.session_view(&session_id).await?;
        Ok(json!({
            "ok": true,
            "session": session
        }))
    }
}

fn extract_step_summary_text(summary: &Json) -> Option<String> {
    if let Some(text) = summary.as_str() {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

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

fn parse_runtime_meta(meta: &Json) -> SessionRuntimeState {
    meta.get("runtime_state")
        .cloned()
        .and_then(|value| serde_json::from_value::<SessionRuntimeState>(value).ok())
        .unwrap_or_default()
}

fn sanitize_session_id(input: &str) -> Result<String, AgentToolError> {
    let session_id = input.trim();
    if session_id.is_empty() {
        return Err(AgentToolError::InvalidArgs(
            "session_id cannot be empty".to_string(),
        ));
    }
    if session_id.len() > MAX_SESSION_ID_LEN {
        return Err(AgentToolError::InvalidArgs(format!(
            "session_id too long (>{MAX_SESSION_ID_LEN})"
        )));
    }
    if session_id == "." || session_id == ".." {
        return Err(AgentToolError::InvalidArgs(
            "session_id cannot be `.` or `..`".to_string(),
        ));
    }
    if session_id.contains('/') || session_id.contains('\\') {
        return Err(AgentToolError::InvalidArgs(
            "session_id cannot contain path separators".to_string(),
        ));
    }
    if session_id.chars().any(|ch| ch.is_control()) {
        return Err(AgentToolError::InvalidArgs(
            "session_id cannot contain control characters".to_string(),
        ));
    }
    Ok(session_id.to_string())
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

async fn is_existing_dir(path: &Path) -> bool {
    fs::metadata(path)
        .await
        .map(|meta| meta.is_dir())
        .unwrap_or(false)
}

fn require_string(args: &Json, key: &str) -> Result<String, AgentToolError> {
    args.get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| AgentToolError::InvalidArgs(format!("missing `{key}`")))
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
    use super::*;

    #[test]
    fn summary_view_has_zero_queue_counters() {
        let mut session = AgentSession::new("s1", "did:opendan:test", Some("on_wakeup"));
        session.current_behavior = "on_wakeup".to_string();

        let view = session.summary_view_json();
        assert_eq!(view["new_msg_count"], Json::from(0));
        assert_eq!(view["new_event_count"], Json::from(0));
        assert_eq!(view["history_msg_count"], Json::from(0));
        assert_eq!(view["history_event_count"], Json::from(0));
    }
}
