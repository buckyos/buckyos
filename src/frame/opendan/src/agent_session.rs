use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use buckyos_api::msg_queue::Message;
use buckyos_api::{
    get_buckyos_api_runtime, AccountBinding, Contact, MsgRecord, MsgRecordWithObject,
    OpenDanAgentSessionRecord, OpenDanSessionLink,
};
use log::{debug, info, warn};
use name_lib::DID;
use ndn_lib::MsgObject;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value as Json};
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, Notify, RwLock};

pub use ::agent_tool::GetSessionTool;

use crate::agent_tool::{sanitize_session_id_for_path, session_record_path, AgentToolError};
use crate::behavior::config::BehaviorSkillMode;
use crate::behavior::SessionRuntimeContext;
use crate::step_record::{LLMStepPromptRenderOptions, LLMStepRecord, LLMStepRecordLog};
use crate::worklog::{render_worklog_prompt_line, render_worklog_prompt_line_from_parts};
use crate::workspace::agent_skill::{
    move_skill_ref_to_front, normalize_unique_skill_refs, remove_skill_ref,
};
use crate::workspace::LocalWorkspaceManager;
use crate::workspace_path::{
    resolve_agent_env_root, resolve_bound_workspace_root, WORKSHOP_WORKLOG_DB_REL_PATH,
};

const DEFAULT_SESSION_FILE: &str = "session.json";
const DEFAULT_SESSION_SUMMARY_FILE: &str = "summary.md";
const DEFAULT_MSG_RECORD_FILE: &str = "msg_record.jsonl";
const WORK_SESSION_PREFIX: &str = "work-";
const DEFAULT_UI_BEHAVIOR: &str = "resolve_router";
const DEFAULT_WORK_BEHAVIOR: &str = "plan";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SessionState {
    Wait,         //运行中，等待任意触发
    WaitForMsg,   //运行中，等待用户输入触发
    WaitForEvent, //运行中，等待特定事件触发

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_data: Option<Json>,
}

impl Default for SessionInputItem {
    fn default() -> Self {
        Self {
            msg: None,
            event_id: None,
            event_data: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionSkillScope {
    Behavior,
    Session,
}

impl Default for SessionSkillScope {
    fn default() -> Self {
        Self::Behavior
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

    step_num: u32,
    step_index: u32,

    workspace_info: Option<Json>,
    local_workspace_id: Option<String>,
    worklog: Vec<Json>,

    #[serde(alias = "loaded_skills")]
    session_loaded_skills: Vec<String>,
    behavior_loaded_skills: Vec<String>,
    skill_recent_list: Vec<String>,
    behavior_skill_mode: BehaviorSkillMode,
    behavior_skill_behavior: Option<String>,
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
            step_num: 0,
            step_index: 0,
            workspace_info: None,
            local_workspace_id: None,
            worklog: vec![],
            session_loaded_skills: vec![],
            behavior_loaded_skills: vec![],
            skill_recent_list: vec![],
            behavior_skill_mode: BehaviorSkillMode::Union,
            behavior_skill_behavior: None,
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
    //session 当前step的序号，从0开始递增，不会因为current_behavior改变而重置
    pub step_num: u32,
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
    pub just_readed_input_event: Vec<Vec<u8>>,

    pub pwd: PathBuf,
    pub session_root_dir: PathBuf,
    pub workspace_info: Option<Json>,
    pub local_workspace_id: Option<String>,
    pub worklog: Vec<Json>,

    pub session_loaded_skills: Vec<String>,
    pub behavior_loaded_skills: Vec<String>,
    pub skill_recent_list: Vec<String>,
    pub behavior_skill_mode: BehaviorSkillMode,
    pub behavior_skill_behavior: Option<String>,
    pub loaded_tools: Vec<String>,
    pub llm_step_records: LLMStepRecordLog,

    pub cost_trace: Json,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub last_activity_ms: u64,
}

impl AgentSession {
    pub fn is_work_session_id(session_id: &str) -> bool {
        session_id.trim().starts_with(WORK_SESSION_PREFIX)
    }

    pub fn worklog_step_id(step_idx: u32) -> String {
        format!("step-{step_idx}")
    }

    pub fn build_worklog_record_from_runtime_context(
        trace: &SessionRuntimeContext,
        record_type: &str,
        status: &str,
        payload: Json,
    ) -> Json {
        json!({
            "type": record_type.trim(),
            "owner_session_id": trace.session_id.trim(),
            "agent_did": trace.agent_name.trim(),
            "behavior": trace.behavior.trim(),
            "step_id": Self::worklog_step_id(trace.step_idx),
            "step_index": trace.step_idx,
            "status": status.trim(),
            "trace": {
                "taskmgr_id": trace.trace_id.trim(),
                "span_id": trace.wakeup_id.trim(),
            },
            "payload": payload,
        })
    }

    pub async fn append_worklog_with_runtime_context(
        &mut self,
        trace: &SessionRuntimeContext,
        record_type: &str,
        status: &str,
        payload: Json,
        local_workspace_mgr: Option<&LocalWorkspaceManager>,
    ) -> Result<(), AgentToolError> {
        if !Self::is_work_session_id(trace.session_id.as_str()) {
            return Ok(());
        }
        let item =
            Self::build_worklog_record_from_runtime_context(trace, record_type, status, payload);
        self.append_worklog(item, local_workspace_mgr).await
    }

    fn has_bound_local_workspace(&self) -> bool {
        self.local_workspace_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some()
    }

    pub fn resolve_default_local_workspace_path(&self) -> Option<PathBuf> {
        resolve_default_local_workspace_path(
            self.local_workspace_id.as_deref(),
            self.workspace_info.as_ref(),
            &self.pwd,
        )
    }

    pub fn resolve_workspace_worklog_db_path(&self) -> Option<PathBuf> {
        if let Some(local_workspace_path) = self.resolve_default_local_workspace_path() {
            let worklog_db_path = local_workspace_path.join("worklog").join("worklog.db");
            if worklog_db_path.is_file() {
                return Some(worklog_db_path);
            }
        }
        resolve_workspace_worklog_db_path(self.workspace_info.as_ref(), &self.pwd)
    }

    pub fn effective_loaded_skills(&self) -> Vec<String> {
        match self.behavior_skill_mode {
            BehaviorSkillMode::SessionOnly => {
                normalize_unique_skill_refs(self.session_loaded_skills.clone())
            }
            BehaviorSkillMode::BehaviorOnly => {
                normalize_unique_skill_refs(self.behavior_loaded_skills.clone())
            }
            BehaviorSkillMode::Union => {
                let mut merged = self.session_loaded_skills.clone();
                merged.extend(self.behavior_loaded_skills.clone());
                normalize_unique_skill_refs(merged)
            }
        }
    }

    pub fn sync_behavior_skills(
        &mut self,
        behavior_name: &str,
        mode: BehaviorSkillMode,
        base_skills: &[String],
    ) {
        let behavior_name = behavior_name.trim();
        let behavior_switched = self
            .behavior_skill_behavior
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            != behavior_name;
        if behavior_switched {
            self.behavior_loaded_skills.clear();
        }
        self.behavior_skill_behavior =
            (!behavior_name.is_empty()).then_some(behavior_name.to_string());
        self.behavior_skill_mode = mode;
        let mut merged = base_skills.to_vec();
        merged.extend(self.behavior_loaded_skills.clone());
        self.behavior_loaded_skills = normalize_unique_skill_refs(merged);
        for skill_ref in self.effective_loaded_skills() {
            move_skill_ref_to_front(&mut self.skill_recent_list, skill_ref.as_str());
        }
    }

    pub fn load_skill_ref(&mut self, skill_ref: &str, scope: SessionSkillScope) {
        match scope {
            SessionSkillScope::Session => {
                let mut items = self.session_loaded_skills.clone();
                items.push(skill_ref.to_string());
                self.session_loaded_skills = normalize_unique_skill_refs(items);
            }
            SessionSkillScope::Behavior => {
                let mut items = self.behavior_loaded_skills.clone();
                items.push(skill_ref.to_string());
                self.behavior_loaded_skills = normalize_unique_skill_refs(items);
            }
        }
        move_skill_ref_to_front(&mut self.skill_recent_list, skill_ref);
    }

    pub fn unload_skill_ref(&mut self, skill_ref: &str) {
        remove_skill_ref(&mut self.session_loaded_skills, skill_ref);
        remove_skill_ref(&mut self.behavior_loaded_skills, skill_ref);
    }

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
            step_num: 0,
            step_index: 0,
            msg_kmsgqueue_curosr: 0,
            event_kmsgqueue_curosr: 0,
            just_readed_input_msg: vec![],
            just_readed_input_event: vec![],
            pwd: PathBuf::new(),
            session_root_dir: PathBuf::new(),
            workspace_info: None,
            local_workspace_id: None,
            worklog: vec![],
            session_loaded_skills: vec![],
            behavior_loaded_skills: vec![],
            skill_recent_list: vec![],
            behavior_skill_mode: BehaviorSkillMode::Union,
            behavior_skill_behavior: None,
            loaded_tools: vec![],
            llm_step_records: LLMStepRecordLog::default(),
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
        let mut state = runtime_meta.state;
        if state == SessionState::Running {
            // `RUNNING` is a volatile in-memory state. After process restart there is no
            // active worker bound to this session anymore, so recover as `WAIT`.
            warn!(
                "agent.session_recover_running_state: session_id={} fallback=WAIT",
                record.session_id
            );
            state = SessionState::Wait;
        }
        let is_paused = runtime_meta.is_paused;

        let mut meta = record.meta.clone();
        if let Some(map) = meta.as_object_mut() {
            map.remove("runtime_state");
        }
        let summary = record.summary.trim().to_string();

        let mut session = Self {
            session_id: record.session_id,
            pwd: PathBuf::new(),
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
            step_num: runtime_meta.step_num,
            step_index: runtime_meta.step_index,
            msg_kmsgqueue_curosr: 0,
            event_kmsgqueue_curosr: 0,
            just_readed_input_msg: vec![],
            just_readed_input_event: vec![],
            workspace_info: runtime_meta.workspace_info,
            local_workspace_id: normalize_optional_string(runtime_meta.local_workspace_id),
            worklog: runtime_meta.worklog,
            session_loaded_skills: runtime_meta.session_loaded_skills,
            behavior_loaded_skills: runtime_meta.behavior_loaded_skills,
            skill_recent_list: runtime_meta.skill_recent_list,
            behavior_skill_mode: runtime_meta.behavior_skill_mode,
            behavior_skill_behavior: normalize_optional_string(
                runtime_meta.behavior_skill_behavior,
            ),
            loaded_tools: runtime_meta.allow_tools,
            llm_step_records: LLMStepRecordLog::default(),
            cost_trace: normalize_json_object(runtime_meta.cost_trace),
            links: record.links,
            tags: record.tags,
            meta: normalize_json_object(meta),
            created_at_ms: record.created_at_ms,
            updated_at_ms: record.updated_at_ms,
            last_activity_ms: record.last_activity_ms,
        };
        session.sync_step_record_storage();
        session
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
            summary: self.summary.trim().to_string(),
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
            step_num: self.step_num,
            step_index: self.step_index,
            workspace_info: self.workspace_info.clone(),
            local_workspace_id: self.local_workspace_id.clone(),
            worklog: self.worklog.clone(),
            session_loaded_skills: self.session_loaded_skills.clone(),
            behavior_loaded_skills: self.behavior_loaded_skills.clone(),
            skill_recent_list: self.skill_recent_list.clone(),
            behavior_skill_mode: self.behavior_skill_mode.clone(),
            behavior_skill_behavior: normalize_optional_string(
                self.behavior_skill_behavior.clone(),
            ),
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

    pub fn sync_step_record_storage(&mut self) {
        self.llm_step_records
            .bind_session(self.session_id.as_str(), &self.session_root_dir);
    }

    pub async fn append_llm_step_record(
        &mut self,
        record: LLMStepRecord,
    ) -> Result<(), AgentToolError> {
        self.sync_step_record_storage();
        self.llm_step_records.append(record).await?;
        self.updated_at_ms = now_ms();
        self.last_activity_ms = self.updated_at_ms;
        Ok(())
    }

    pub async fn render_llm_step_records_prompt(
        &mut self,
        options: Option<&LLMStepPromptRenderOptions>,
    ) -> Result<String, AgentToolError> {
        self.sync_step_record_storage();
        self.llm_step_records.render_prompt_text(options).await
    }

    pub async fn render_last_llm_step_record(&mut self) -> Result<Option<String>, AgentToolError> {
        self.sync_step_record_storage();
        self.llm_step_records.render_last_step_text().await
    }

    pub fn llm_step_record_path(&mut self) -> Option<PathBuf> {
        self.sync_step_record_storage();
        self.llm_step_records.record_file_path()
    }

    pub async fn append_worklog(
        &mut self,
        item: Json,
        local_workspace_mgr: Option<&LocalWorkspaceManager>,
    ) -> Result<(), AgentToolError> {
        let has_local_workspace = self.has_bound_local_workspace();
        let mut prompt_line = Self::render_worklog_prompt_line_from_session_item(&item);

        if has_local_workspace {
            let Some(local_workspace_mgr) = local_workspace_mgr else {
                return Err(AgentToolError::InvalidArgs(format!(
                    "session `{}` is bound to local workspace but LocalWorkspaceManager is missing",
                    self.session_id
                )));
            };
            for old_item in self.worklog.drain(..) {
                local_workspace_mgr
                    .append_worklog(
                        self.session_id.as_str(),
                        self.owner_agent.as_str(),
                        self.current_behavior.as_str(),
                        self.step_index,
                        old_item,
                    )
                    .await?;
            }

            let appended = local_workspace_mgr
                .append_worklog(
                    self.session_id.as_str(),
                    self.owner_agent.as_str(),
                    self.current_behavior.as_str(),
                    self.step_index,
                    item,
                )
                .await?;
            prompt_line = render_worklog_prompt_line(&appended);
        } else {
            self.worklog.push(item);
            if self.worklog.len() > 256 {
                let start = self.worklog.len().saturating_sub(256);
                self.worklog = self.worklog.split_off(start);
            }
        }

        self.updated_at_ms = now_ms();
        self.last_activity_ms = self.updated_at_ms;
        info!("{}", prompt_line);
        Ok(())
    }

    fn render_worklog_prompt_line_from_session_item(item: &Json) -> String {
        let record_type = item
            .get("type")
            .and_then(Json::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("Worklog");
        let status = item
            .get("status")
            .and_then(Json::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("UNKNOWN");
        let summary = item
            .get("summary")
            .and_then(Json::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let prompt_digest = item
            .get("prompt_view")
            .and_then(Json::as_object)
            .and_then(|view| view.get("digest"))
            .and_then(Json::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let payload = item.get("payload").cloned().unwrap_or(Json::Null);

        render_worklog_prompt_line_from_parts(record_type, status, prompt_digest, summary, &payload)
    }

    pub fn should_ready_by_wait_timeout(&self, now_ms: u64) -> bool {
        if self.state != SessionState::Wait
            && self.state != SessionState::WaitForMsg
            && self.state != SessionState::WaitForEvent
        {
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
            "loaded_skills": self.effective_loaded_skills(),
            "meta": self.meta,
        })
    }

    fn update_state_on_input_arrived(&mut self, item: &SessionInputItem) {
        if self.state == SessionState::Running {
            return;
        }

        if self.state == SessionState::WaitForMsg && item.msg.is_some() {
            self.updated_at_ms = now_ms();
            self.wait_details = None;
            self.state = SessionState::Ready;
            info!(
                "{} will wakeup session:{} from WaitForMsg",
                self.owner_agent, self.session_id
            );
            return;
        }
        if self.state == SessionState::WaitForEvent && item.event_id.is_some() {
            self.updated_at_ms = now_ms();
            self.wait_details = None;
            self.state = SessionState::Ready;
            info!(
                "{} will wakeup session:{} from WaitForEvent",
                self.owner_agent, self.session_id
            );
            return;
        }
        if self.state == SessionState::Wait || self.state == SessionState::End {
            self.updated_at_ms = now_ms();
            self.wait_details = None;
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
            let default_root = PathBuf::from("sessions");
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

    pub async fn resolve_msg_from_name(
        from_did: &DID,
        session_from_name: Option<&str>,
        contact_mgr_owner: Option<&DID>,
    ) -> Option<String> {
        let session_from_name = normalize_optional_string(session_from_name.map(str::to_string));
        let from_did_text = from_did.to_string();

        let runtime = match get_buckyos_api_runtime() {
            Ok(runtime) => runtime,
            Err(err) => {
                debug!(
                    "agent.resolve_msg_from_name runtime_unavailable: did={} err={}",
                    from_did_text, err
                );
                return session_from_name;
            }
        };

        let msg_center = match runtime.get_msg_center_client().await {
            Ok(client) => client,
            Err(err) => {
                debug!(
                    "agent.resolve_msg_from_name msg_center_unavailable: did={} err={}",
                    from_did_text, err
                );
                return session_from_name;
            }
        };

        let contact = match msg_center
            .get_contact(from_did.clone(), contact_mgr_owner.cloned())
            .await
        {
            Ok(contact) => contact,
            Err(err) => {
                warn!(
                    "agent.resolve_msg_from_name get_contact_failed: did={} owner={:?} err={}",
                    from_did_text, contact_mgr_owner, err
                );
                return session_from_name;
            }
        };

        let Some(contact) = contact else {
            return session_from_name;
        };

        let contact_name = normalize_optional_string(Some(contact.name.clone()))
            .filter(|value| !value.eq_ignore_ascii_case(from_did_text.as_str()));
        let nickname = resolve_contact_binding_name(&contact, &["nickname"]);
        let full_name = resolve_contact_binding_name(&contact, &["full_name"]);

        contact_name
            .or(session_from_name)
            .or(nickname)
            .or(full_name)
    }
}

#[derive(Clone, Debug)]
pub struct AgentSessionMgr {
    owner_agent: String,
    sessions_root: PathBuf,
    default_ui_behavior: String,
    default_work_behavior: String,
    sessions: Arc<RwLock<HashMap<String, Arc<Mutex<AgentSession>>>>>,
    scheduler_lock: Arc<Mutex<()>>,
    ready_notify: Arc<Notify>,
}

impl AgentSessionMgr {
    pub async fn new(
        owner_agent: impl Into<String>,
        sessions_root: impl Into<PathBuf>,
        default_ui_behavior: String,
        default_work_behavior: String,
    ) -> Result<Self, AgentToolError> {
        let owner_agent = owner_agent.into();
        let sessions_root = sessions_root.into();
        let default_ui_behavior = normalize_optional_string(Some(default_ui_behavior))
            .unwrap_or_else(|| DEFAULT_UI_BEHAVIOR.to_string());
        let default_work_behavior = normalize_optional_string(Some(default_work_behavior))
            .unwrap_or_else(|| DEFAULT_WORK_BEHAVIOR.to_string());

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
            default_ui_behavior,
            default_work_behavior,
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

    fn agent_env_root(&self) -> PathBuf {
        self.sessions_root
            .parent()
            .map(|path| path.to_path_buf())
            .unwrap_or_else(|| self.sessions_root.clone())
    }

    fn hydrate_session_runtime_context(&self, session: &mut AgentSession) {
        if session.pwd.as_os_str().is_empty() {
            session.pwd = resolve_bound_workspace_root(session.workspace_info.as_ref())
                .unwrap_or_else(|| self.agent_env_root());
        }
        if session.session_root_dir.as_os_str().is_empty() {
            session.session_root_dir = self.sessions_root.clone();
        }
        session.sync_step_record_storage();
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
        let normalized_default_remote =
            normalize_optional_string(default_remote.map(str::to_string));
        if let Some(existing) = self.get_session(session_id.as_str()).await {
            let mut guard = existing.lock().await;
            self.hydrate_session_runtime_context(&mut guard);
            let mut should_save = self.ensure_default_behavior_if_empty(&mut guard);
            if guard.default_remote.is_none() {
                if let Some(default_remote) = normalized_default_remote.clone() {
                    guard.default_remote = Some(default_remote);
                    should_save = true;
                }
            }
            drop(guard);
            if should_save {
                self.save_session(session_id.as_str()).await?;
            }
            return Ok(existing);
        }
        info!(
            "agent.persist_entity_prepare: kind=session_entity owner_agent={} session_id={}",
            self.owner_agent, session_id
        );

        let behavior = behavior
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                self.default_behavior_for_session_id(session_id.as_str())
                    .to_string()
            });

        let mut session = AgentSession::new(
            session_id.clone(),
            self.owner_agent.clone(),
            Some(behavior.as_str()),
        );
        self.hydrate_session_runtime_context(&mut session);
        self.ensure_default_behavior_if_empty(&mut session);
        if let Some(title) = normalize_optional_string(title) {
            session.title = title;
        }
        session.default_remote = normalized_default_remote;
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
        self.ensure_default_behavior_if_empty(&mut guard);
        let prev_state = guard.state;
        guard.update_state_on_input_arrived(input_item);
        info!(
            "agent.session_try_wakeup: session_id={} prev_state={:?} next_state={:?}",
            guard.session_id, prev_state, guard.state
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
        let path = self.session_file_path(session_id.as_str())?;
        if !is_existing_file(&path).await {
            return Ok(());
        }

        let raw = fs::read_to_string(&path).await.map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "read session file `{}` failed: {err}",
                path.display()
            ))
        })?;
        let mut record =
            serde_json::from_str::<OpenDanAgentSessionRecord>(&raw).map_err(|err| {
                AgentToolError::ExecFailed(format!(
                    "parse session file `{}` failed: {err}",
                    path.display()
                ))
            })?;
        let session_dir = self.sessions_root.join(session_id.as_str());
        self.load_session_summary_from_file(session_dir.as_path(), &mut record)
            .await?;

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
                guard.wait_details = None;
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

    pub fn notify_ready(&self) {
        self.ready_notify.notify_one();
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
            self.load_session_summary_from_file(path.as_path(), &mut record)
                .await?;

            let mut runtime = AgentSession::from_record(record);
            self.hydrate_session_runtime_context(&mut runtime);
            if self.ensure_default_behavior_if_empty(&mut runtime) {
                let normalized = runtime.to_record(true);
                self.write_session_record(&normalized).await?;
            }
            self.sessions
                .write()
                .await
                .insert(session_id, Arc::new(Mutex::new(runtime)));
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
        })?;
        self.write_session_summary_file(session_dir.as_path(), record.summary.as_str())
            .await
    }

    fn session_file_path(&self, session_id: &str) -> Result<PathBuf, AgentToolError> {
        session_record_path(&self.sessions_root, session_id, DEFAULT_SESSION_FILE)
    }

    async fn load_session_summary_from_file(
        &self,
        session_dir: &Path,
        record: &mut OpenDanAgentSessionRecord,
    ) -> Result<(), AgentToolError> {
        let summary_file = session_dir.join(DEFAULT_SESSION_SUMMARY_FILE);
        match fs::read_to_string(&summary_file).await {
            Ok(summary) => {
                record.summary = summary.trim().to_string();
                Ok(())
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                self.write_session_summary_file(session_dir, record.summary.as_str())
                    .await?;
                record.summary = record.summary.trim().to_string();
                Ok(())
            }
            Err(err) => Err(AgentToolError::ExecFailed(format!(
                "read session summary file `{}` failed: {err}",
                summary_file.display()
            ))),
        }
    }

    async fn write_session_summary_file(
        &self,
        session_dir: &Path,
        summary: &str,
    ) -> Result<(), AgentToolError> {
        let summary_file = session_dir.join(DEFAULT_SESSION_SUMMARY_FILE);
        if !is_existing_file(&summary_file).await {
            info!(
                "agent.persist_entity_prepare: kind=session_summary_file path={}",
                summary_file.display()
            );
        }
        fs::write(&summary_file, summary.trim().as_bytes())
            .await
            .map_err(|err| {
                AgentToolError::ExecFailed(format!(
                    "write session summary file `{}` failed: {err}",
                    summary_file.display()
                ))
            })
    }

    fn default_behavior_for_session_id(&self, session_id: &str) -> &str {
        if Self::is_ui_session(session_id) {
            self.default_ui_behavior.as_str()
        } else {
            self.default_work_behavior.as_str()
        }
    }

    fn ensure_default_behavior_if_empty(&self, session: &mut AgentSession) -> bool {
        if !session.current_behavior.trim().is_empty() {
            return false;
        }
        let default_behavior = self
            .default_behavior_for_session_id(session.session_id.as_str())
            .to_string();
        warn!(
            "agent.session_default_behavior_applied: session_id={} behavior={}",
            session.session_id, default_behavior
        );
        session.current_behavior = default_behavior;
        true
    }
}

#[async_trait]
impl ::agent_tool::SessionViewBackend for AgentSessionMgr {
    async fn session_view(&self, session_id: &str) -> Result<Json, AgentToolError> {
        AgentSessionMgr::session_view(self, session_id).await
    }
}

fn resolve_workspace_worklog_db_path(
    workspace_info: Option<&Json>,
    session_cwd: &Path,
) -> Option<PathBuf> {
    resolve_agent_env_root(workspace_info, session_cwd)
        .map(|root| root.join(WORKSHOP_WORKLOG_DB_REL_PATH))
        .filter(|path| path.is_file())
}

fn resolve_default_local_workspace_path(
    local_workspace_id: Option<&str>,
    workspace_info: Option<&Json>,
    session_cwd: &Path,
) -> Option<PathBuf> {
    crate::workspace_path::resolve_default_local_workspace_path(
        local_workspace_id,
        workspace_info,
        session_cwd,
    )
    .filter(|path| path.is_dir())
}

fn parse_runtime_meta(meta: &Json) -> SessionRuntimeState {
    meta.get("runtime_state")
        .cloned()
        .and_then(|value| serde_json::from_value::<SessionRuntimeState>(value).ok())
        .unwrap_or_default()
}

fn sanitize_session_id(input: &str) -> Result<String, AgentToolError> {
    sanitize_session_id_for_path(input)
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
}

fn resolve_contact_binding_name(contact: &Contact, keys: &[&str]) -> Option<String> {
    contact
        .bindings
        .iter()
        .max_by_key(|binding| binding.last_active_at)
        .and_then(|binding| resolve_binding_meta_name(binding, keys))
        .or_else(|| {
            contact
                .bindings
                .iter()
                .find_map(|binding| resolve_binding_meta_name(binding, keys))
        })
}

fn resolve_binding_meta_name(binding: &AccountBinding, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = binding.meta.get(*key) {
            if let Some(value) = normalize_optional_string(Some(value.clone())) {
                return Some(value);
            }
        }
    }

    for (key, value) in &binding.meta {
        if keys.iter().any(|expect| key.eq_ignore_ascii_case(expect)) {
            if let Some(value) = normalize_optional_string(Some(value.clone())) {
                return Some(value);
            }
        }
    }

    None
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

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worklog::{WorklogListOptions, WorklogRecordType, WorklogTool, WorklogToolConfig};
    use crate::workspace::{
        CreateLocalWorkspaceRequest, LocalWorkspaceManagerConfig, WorkspaceOwner,
    };
    use tempfile::tempdir;

    #[tokio::test]
    async fn session_mgr_uses_configured_default_behaviors() {
        let temp = tempdir().expect("create tempdir");
        let mgr = AgentSessionMgr::new(
            "agent.test",
            temp.path().join("sessions"),
            "ui_default".to_string(),
            "work_default".to_string(),
        )
        .await
        .expect("create session manager");

        assert_eq!(mgr.default_behavior_for_session_id("ui-demo"), "ui_default");
        assert_eq!(
            mgr.default_behavior_for_session_id("work-demo"),
            "work_default"
        );
    }

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

    #[tokio::test]
    async fn append_worklog_without_local_workspace_keeps_memory() {
        let mut session = AgentSession::new("sess-memory", "did:opendan:test", Some("DO"));
        session
            .append_worklog(
                json!({
                    "type": "FunctionRecord",
                    "status": "OK",
                    "payload": { "tool_name": "todo_manage" }
                }),
                None,
            )
            .await
            .expect("append in memory");
        assert_eq!(session.worklog.len(), 1);
    }

    #[tokio::test]
    async fn append_worklog_with_local_workspace_delegates_to_workspace_db() {
        let root = tempfile::tempdir().expect("create temp dir");
        let manager = LocalWorkspaceManager::create_workshop(
            "did:opendan:test",
            LocalWorkspaceManagerConfig::new(root.path()),
        )
        .await
        .expect("create local workspace manager");
        let workspace = manager
            .create_local_workspace(CreateLocalWorkspaceRequest {
                name: "devbox".to_string(),
                template: None,
                owner: WorkspaceOwner::AgentCreated,
                created_by_session: Some("sess-db".to_string()),
                policy_profile_id: None,
            })
            .await
            .expect("create workspace");
        manager
            .bind_local_workspace("sess-db", workspace.workspace_id.as_str())
            .await
            .expect("bind local workspace");

        let mut session = AgentSession::new("sess-db", "did:opendan:test", Some("DO"));
        session.local_workspace_id = Some(workspace.workspace_id.clone());

        session
            .append_worklog(
                json!({
                    "type": "FunctionRecord",
                    "status": "OK",
                    "payload": { "tool_name": "todo_manage" }
                }),
                Some(&manager),
            )
            .await
            .expect("append to local workspace");
        assert_eq!(session.worklog.len(), 0);

        let workspace_path = manager
            .get_local_workspace_path(workspace.workspace_id.as_str())
            .await
            .expect("load workspace path");
        let worklog_tool = WorklogTool::new(WorklogToolConfig::with_db_path(
            workspace_path.join("worklog").join("worklog.db"),
        ))
        .expect("create worklog tool");

        let records = worklog_tool
            .list_worklog_records(WorklogListOptions {
                owner_session_id: Some("sess-db".to_string()),
                ..Default::default()
            })
            .await
            .expect("list workspace records");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].record_type, WorklogRecordType::FunctionRecord);
    }

    #[tokio::test]
    async fn ensure_session_backfills_missing_default_remote_for_existing_session() {
        let temp = tempdir().expect("create tempdir");
        let mgr = AgentSessionMgr::new(
            "agent.test",
            temp.path().join("sessions"),
            "ui_default".to_string(),
            "work_default".to_string(),
        )
        .await
        .expect("create session manager");

        mgr.ensure_session("ui-demo", None, None, None)
            .await
            .expect("create session without default remote");
        mgr.ensure_session("ui-demo", None, None, Some("did:bns:alice"))
            .await
            .expect("backfill default remote");

        let session = mgr
            .get_session("ui-demo")
            .await
            .expect("session should exist");
        let guard = session.lock().await;
        assert_eq!(guard.default_remote.as_deref(), Some("did:bns:alice"));
        drop(guard);

        let raw = fs::read_to_string(
            mgr.session_file_path("ui-demo")
                .expect("resolve session file path"),
        )
        .await
        .expect("read session file");
        let record: OpenDanAgentSessionRecord =
            serde_json::from_str(&raw).expect("parse session record");
        assert_eq!(
            record.meta["runtime_state"]["default_remote"].as_str(),
            Some("did:bns:alice")
        );
    }

    #[test]
    fn build_worklog_record_from_runtime_context_infers_core_fields() {
        let trace = SessionRuntimeContext {
            trace_id: "trace-1".to_string(),
            agent_name: "did:opendan:test".to_string(),
            behavior: "DO".to_string(),
            step_idx: 7,
            wakeup_id: "wakeup-1".to_string(),
            session_id: "work-abc".to_string(),
        };
        let record = AgentSession::build_worklog_record_from_runtime_context(
            &trace,
            "ActionRecord",
            "OK",
            json!({"cmd_digest":"echo ok"}),
        );
        assert_eq!(record["type"], Json::String("ActionRecord".to_string()));
        assert_eq!(
            record["owner_session_id"],
            Json::String("work-abc".to_string())
        );
        assert_eq!(record["behavior"], Json::String("DO".to_string()));
        assert_eq!(record["step_id"], Json::String("step-7".to_string()));
        assert_eq!(record["step_index"], Json::from(7));
        assert_eq!(
            record["trace"]["taskmgr_id"],
            Json::String("trace-1".to_string())
        );
    }

    #[tokio::test]
    async fn append_worklog_with_runtime_context_skips_ui_session() {
        let mut session = AgentSession::new("ui-1", "did:opendan:test", Some("DO"));
        let trace = SessionRuntimeContext {
            trace_id: "trace-ui".to_string(),
            agent_name: "did:opendan:test".to_string(),
            behavior: "DO".to_string(),
            step_idx: 0,
            wakeup_id: "wakeup-ui".to_string(),
            session_id: "ui-1".to_string(),
        };

        session
            .append_worklog_with_runtime_context(
                &trace,
                "ActionRecord",
                "OK",
                json!({"cmd_digest":"echo skip"}),
                None,
            )
            .await
            .expect("skip ui worklog");

        assert!(session.worklog.is_empty());
    }

    #[test]
    fn from_record_recovers_running_to_wait() {
        let mut session = AgentSession::new("s-running", "did:opendan:test", Some("DO"));
        session.state = SessionState::Running;

        let record = session.to_record(true);
        let restored = AgentSession::from_record(record);
        assert_eq!(restored.state, SessionState::Wait);
    }

    #[test]
    fn from_record_ignores_legacy_last_step_summary_runtime_meta() {
        let mut session = AgentSession::new("s-summary", "did:opendan:test", Some("DO"));
        session.summary.clear();
        let mut record = session.to_record(true);
        record.meta["runtime_state"]["last_step_summary"] = json!("step summary");
        let restored = AgentSession::from_record(record);
        assert!(restored.summary.is_empty());
        assert_eq!(restored.step_index, 0);
    }

    #[test]
    fn step_num_is_preserved_in_runtime_meta() {
        let mut session = AgentSession::new("s-step-num", "did:opendan:test", Some("DO"));
        session.step_num = 42;
        session.step_index = 7;

        let record = session.to_record(true);
        let restored = AgentSession::from_record(record);
        assert_eq!(restored.step_num, 42);
        assert_eq!(restored.step_index, 7);
    }

    #[test]
    fn wait_timeout_can_wake_generic_wait_state() {
        let mut session = AgentSession::new("s-wait-timeout", "did:opendan:test", Some("DO"));
        session.state = SessionState::Wait;
        session.wait_details = Some(SessionWaitDetails {
            filter: Json::Null,
            deadline_ms: Some(123),
            note: Some("retry later".to_string()),
        });

        assert!(session.should_ready_by_wait_timeout(123));
        assert!(!session.should_ready_by_wait_timeout(122));
    }

    #[tokio::test]
    async fn session_summary_file_is_persisted_and_preferred_over_session_json() {
        let root = tempfile::tempdir().expect("create temp dir");
        let sessions_root = root.path().join("sessions");
        let store = AgentSessionMgr::new(
            "did:opendan:test",
            sessions_root.clone(),
            "resolve_router".to_string(),
            "plan".to_string(),
        )
        .await
        .expect("create session manager");

        let session = store
            .ensure_session("work-summary", None, Some("plan"), None)
            .await
            .expect("ensure session");
        {
            let mut guard = session.lock().await;
            guard.summary = "# Plan\n\n- finish task".to_string();
        }
        store
            .save_session("work-summary")
            .await
            .expect("save session");

        let session_dir = sessions_root.join("work-summary");
        let summary_file = session_dir.join(DEFAULT_SESSION_SUMMARY_FILE);
        let summary_text = fs::read_to_string(&summary_file)
            .await
            .expect("read summary file");
        assert_eq!(summary_text, "# Plan\n\n- finish task");

        let session_file = session_dir.join(DEFAULT_SESSION_FILE);
        let raw = fs::read_to_string(&session_file)
            .await
            .expect("read session file");
        let mut record: OpenDanAgentSessionRecord =
            serde_json::from_str(&raw).expect("parse session json");
        record.summary = "json summary should be ignored".to_string();
        let bytes = serde_json::to_vec_pretty(&record).expect("encode session json");
        fs::write(&session_file, bytes)
            .await
            .expect("rewrite session file");

        let reloaded = AgentSessionMgr::new(
            "did:opendan:test",
            sessions_root.clone(),
            "resolve_router".to_string(),
            "plan".to_string(),
        )
        .await
        .expect("reload session manager");
        let restored = reloaded
            .get_session("work-summary")
            .await
            .expect("session exists");
        let restored_guard = restored.lock().await;
        assert_eq!(restored_guard.summary, "# Plan\n\n- finish task");
    }

    #[tokio::test]
    async fn hydrate_restores_pwd_from_bound_workspace_info() {
        let root = tempfile::tempdir().expect("create temp dir");
        let sessions_root = root.path().join("sessions");
        let store = AgentSessionMgr::new(
            "did:opendan:test",
            sessions_root.clone(),
            "resolve_router".to_string(),
            "plan".to_string(),
        )
        .await
        .expect("create session manager");

        let session = store
            .ensure_session("work-bound", None, Some("plan"), None)
            .await
            .expect("ensure session");
        let bound_workspace_path = root.path().join("workspaces").join("demo");
        {
            let mut guard = session.lock().await;
            guard.local_workspace_id = Some("ws-demo".to_string());
            guard.workspace_info = Some(json!({
                "local_workspace_id": "ws-demo",
                "binding": {
                    "workspace_path": bound_workspace_path.to_string_lossy().to_string()
                }
            }));
        }
        store
            .save_session("work-bound")
            .await
            .expect("save session");

        let reloaded = AgentSessionMgr::new(
            "did:opendan:test",
            sessions_root,
            "resolve_router".to_string(),
            "plan".to_string(),
        )
        .await
        .expect("reload session manager");
        let restored = reloaded
            .get_session("work-bound")
            .await
            .expect("session exists");
        let restored_guard = restored.lock().await;
        assert_eq!(restored_guard.pwd, bound_workspace_path);
    }

    #[tokio::test]
    async fn session_resolve_workspace_worklog_db_path_uses_bound_workspace() {
        let root = tempfile::tempdir().expect("create temp dir");
        let local_workspace_id = "ws-demo";
        let workspace_path = root.path().join("workspaces").join(local_workspace_id);
        let worklog_db_path = workspace_path.join("worklog").join("worklog.db");
        fs::create_dir_all(worklog_db_path.parent().expect("worklog parent"))
            .await
            .expect("create worklog parent");
        fs::write(&worklog_db_path, b"")
            .await
            .expect("create worklog db");

        let mut session = AgentSession::new("work-resolve-db", "did:opendan:test", Some("plan"));
        session.local_workspace_id = Some(local_workspace_id.to_string());
        session.pwd = workspace_path.join("project");
        session.workspace_info = Some(json!({
            "local_workspace_id": local_workspace_id,
            "binding": {
                "workspace_path": workspace_path.to_string_lossy().to_string()
            }
        }));

        assert_eq!(
            session.resolve_default_local_workspace_path(),
            Some(workspace_path.clone())
        );
        assert_eq!(
            session.resolve_workspace_worklog_db_path(),
            Some(worklog_db_path)
        );
    }
}
