use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use buckyos_api::{BoxKind, MsgCenterClient};
use name_lib::DID;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use tokio::fs;

use crate::agent_tool::{AgentTool, ToolError, ToolManager, ToolSpec};
use crate::behavior::TraceCtx;

pub const TOOL_LIST_SESSIONS: &str = "list_sessions";
pub const TOOL_CREATE_SESSION: &str = "create_session";
pub const TOOL_GET_SESSION: &str = "get_session";
pub const TOOL_UPDATE_SESSION: &str = "update_session";
pub const TOOL_LINK_SESSIONS: &str = "link_sessions";
pub const TOOL_LOAD_CHAT_HISTORY: &str = "load_chat_history";
pub const TOOL_LOAC_CHAT_HISTORY_ALIAS: &str = "loac_chat_history";

const DEFAULT_SESSIONS_DIR_REL_PATH: &str = "session";
const DEFAULT_SESSION_FILE_NAME: &str = "session.json";
const DEFAULT_LIST_LIMIT: usize = 20;
const DEFAULT_MAX_LIST_LIMIT: usize = 200;
const DEFAULT_CHAT_LIMIT: usize = 32;
const DEFAULT_CHAT_TOKEN_LIMIT: u32 = 2_000;
const DEFAULT_SESSION_STATUS: &str = "active";
const MAX_SESSION_ID_LEN: usize = 180;
const MAX_TITLE_LEN: usize = 200;
const MAX_SUMMARY_LEN: usize = 4_000;

static SESSION_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
pub struct AgentSessionConfig {
    pub agent_root: PathBuf,
    pub sessions_dir_rel_path: PathBuf,
    pub session_file_name: String,
    pub default_list_limit: usize,
    pub max_list_limit: usize,
    pub default_chat_limit: usize,
    pub default_chat_token_limit: u32,
}

impl AgentSessionConfig {
    pub fn new(agent_root: impl Into<PathBuf>) -> Self {
        Self {
            agent_root: agent_root.into(),
            sessions_dir_rel_path: PathBuf::from(DEFAULT_SESSIONS_DIR_REL_PATH),
            session_file_name: DEFAULT_SESSION_FILE_NAME.to_string(),
            default_list_limit: DEFAULT_LIST_LIMIT,
            max_list_limit: DEFAULT_MAX_LIST_LIMIT,
            default_chat_limit: DEFAULT_CHAT_LIMIT,
            default_chat_token_limit: DEFAULT_CHAT_TOKEN_LIMIT,
        }
    }
}

#[derive(Clone)]
pub struct AgentSession {
    cfg: AgentSessionConfig,
    sessions_dir: PathBuf,
    msg_center: Option<Arc<MsgCenterClient>>,
}

impl AgentSession {
    pub async fn new(
        mut cfg: AgentSessionConfig,
        msg_center: Option<Arc<MsgCenterClient>>,
    ) -> Result<Self, ToolError> {
        if cfg.default_list_limit == 0 {
            cfg.default_list_limit = DEFAULT_LIST_LIMIT;
        }
        if cfg.max_list_limit == 0 {
            cfg.max_list_limit = DEFAULT_MAX_LIST_LIMIT;
        }
        if cfg.default_list_limit > cfg.max_list_limit {
            cfg.default_list_limit = cfg.max_list_limit;
        }
        if cfg.default_chat_limit == 0 {
            cfg.default_chat_limit = DEFAULT_CHAT_LIMIT;
        }
        if cfg.default_chat_token_limit == 0 {
            cfg.default_chat_token_limit = DEFAULT_CHAT_TOKEN_LIMIT;
        }
        if cfg.session_file_name.trim().is_empty() {
            cfg.session_file_name = DEFAULT_SESSION_FILE_NAME.to_string();
        }
        ensure_safe_file_name(cfg.session_file_name.as_str())?;

        let agent_root = normalize_root(&cfg.agent_root).await?;
        cfg.agent_root = agent_root.clone();
        let sessions_dir = resolve_relative_path(&agent_root, &cfg.sessions_dir_rel_path)?;
        fs::create_dir_all(&sessions_dir)
            .await
            .map_err(|err| ToolError::ExecFailed(format!("create session dir failed: {err}")))?;

        Ok(Self {
            cfg,
            sessions_dir,
            msg_center,
        })
    }

    pub fn sessions_dir(&self) -> &Path {
        &self.sessions_dir
    }

    pub fn register_tools(&self, tool_mgr: &ToolManager) -> Result<(), ToolError> {
        tool_mgr.register_tool(ListSessionsTool {
            session: self.clone(),
        })?;
        tool_mgr.register_tool(CreateSessionTool {
            session: self.clone(),
        })?;
        tool_mgr.register_tool(GetSessionTool {
            session: self.clone(),
        })?;
        tool_mgr.register_tool(UpdateSessionTool {
            session: self.clone(),
        })?;
        tool_mgr.register_tool(LinkSessionsTool {
            session: self.clone(),
        })?;

        if !tool_mgr.has_tool(TOOL_LOAD_CHAT_HISTORY) {
            tool_mgr.register_tool(LoadChatHistoryTool {
                session: self.clone(),
                tool_name: TOOL_LOAD_CHAT_HISTORY.to_string(),
            })?;
        }
        if !tool_mgr.has_tool(TOOL_LOAC_CHAT_HISTORY_ALIAS) {
            tool_mgr.register_tool(LoadChatHistoryTool {
                session: self.clone(),
                tool_name: TOOL_LOAC_CHAT_HISTORY_ALIAS.to_string(),
            })?;
        }
        Ok(())
    }

    pub fn generate_session_id(prefix: Option<&str>) -> String {
        let prefix = prefix
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or("session");
        let ts = now_ms();
        let ctr = SESSION_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        format!("{prefix}-{ts:013x}-{pid:08x}-{ctr:06x}")
    }

    pub async fn create_session(
        &self,
        req: CreateSessionRequest,
    ) -> Result<AgentSessionRecord, ToolError> {
        let session_id = if let Some(session_id) = req.session_id {
            sanitize_session_id(session_id.as_str())?
        } else {
            Self::generate_session_id(req.id_prefix.as_deref())
        };
        let owner_agent = sanitize_non_empty(req.owner_agent.as_str(), "owner_agent")?;

        let session_file_path = self.session_file_path(session_id.as_str())?;
        if fs::try_exists(&session_file_path).await.unwrap_or(false) {
            return Err(ToolError::AlreadyExists(format!(
                "session `{session_id}` already exists"
            )));
        }
        ensure_parent_dir(&session_file_path).await?;

        let ts = now_ms();
        let mut links = Vec::<SessionLink>::new();
        for link in req.links {
            links.push(normalize_session_link(link)?);
        }

        let title = sanitize_title(req.title.unwrap_or_else(|| "Untitled Session".to_string()))?;
        let summary = sanitize_summary(req.summary.unwrap_or_default())?;
        let tags = normalize_tags(req.tags);
        let status = normalize_status(req.status.as_deref().unwrap_or(DEFAULT_SESSION_STATUS))?;
        let meta = normalize_meta(req.meta);

        let record = AgentSessionRecord {
            session_id,
            owner_agent,
            title,
            summary,
            status,
            created_at_ms: ts,
            updated_at_ms: ts,
            last_activity_ms: ts,
            links,
            tags,
            meta,
        };
        self.save_session(&record).await?;
        Ok(record)
    }

    pub async fn load_session(
        &self,
        session_id: impl AsRef<str>,
    ) -> Result<Option<AgentSessionRecord>, ToolError> {
        let session_id = sanitize_session_id(session_id.as_ref())?;
        let path = self.session_file_path(session_id.as_str())?;
        if !fs::try_exists(&path).await.unwrap_or(false) {
            return Ok(None);
        }
        let raw = fs::read_to_string(&path).await.map_err(|err| {
            ToolError::ExecFailed(format!("read session `{}` failed: {err}", path.display()))
        })?;
        let mut session: AgentSessionRecord = serde_json::from_str(&raw).map_err(|err| {
            ToolError::ExecFailed(format!("parse session `{}` failed: {err}", path.display()))
        })?;
        session.session_id = session_id;
        session.owner_agent = sanitize_non_empty(session.owner_agent.as_str(), "owner_agent")
            .unwrap_or_else(|_| "unknown".to_string());
        session.title =
            sanitize_title(session.title).unwrap_or_else(|_| "Untitled Session".to_string());
        session.summary = sanitize_summary(session.summary).unwrap_or_default();
        session.status = normalize_status(session.status.as_str())
            .unwrap_or_else(|_| DEFAULT_SESSION_STATUS.to_string());
        session.tags = normalize_tags(session.tags);
        session.links = session
            .links
            .into_iter()
            .filter_map(|link| normalize_session_link(link).ok())
            .collect();
        session.meta = normalize_meta(session.meta);
        if session.created_at_ms == 0 {
            session.created_at_ms = now_ms();
        }
        if session.updated_at_ms == 0 {
            session.updated_at_ms = session.created_at_ms;
        }
        if session.last_activity_ms == 0 {
            session.last_activity_ms = session.updated_at_ms;
        }
        Ok(Some(session))
    }

    pub async fn list_sessions(
        &self,
        limit: usize,
        include_deleted: bool,
    ) -> Result<Vec<AgentSessionRecord>, ToolError> {
        let limit = limit.clamp(1, self.cfg.max_list_limit);
        let mut sessions = Vec::<AgentSessionRecord>::new();

        let mut read_dir = fs::read_dir(&self.sessions_dir).await.map_err(|err| {
            ToolError::ExecFailed(format!(
                "read sessions dir `{}` failed: {err}",
                self.sessions_dir.display()
            ))
        })?;
        while let Some(entry) = read_dir
            .next_entry()
            .await
            .map_err(|err| ToolError::ExecFailed(format!("iterate sessions dir failed: {err}")))?
        {
            let file_type = entry.file_type().await.map_err(|err| {
                ToolError::ExecFailed(format!(
                    "read entry type `{}` failed: {err}",
                    entry.path().display()
                ))
            })?;
            if !file_type.is_dir() {
                continue;
            }
            let Some(raw_id) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            let Ok(session_id) = sanitize_session_id(raw_id.as_str()) else {
                continue;
            };
            if let Some(session) = self.load_session(session_id.as_str()).await? {
                if !include_deleted && session.status == "deleted" {
                    continue;
                }
                sessions.push(session);
            }
        }

        sessions.sort_by(|a, b| {
            b.last_activity_ms
                .cmp(&a.last_activity_ms)
                .then_with(|| b.updated_at_ms.cmp(&a.updated_at_ms))
                .then_with(|| a.session_id.cmp(&b.session_id))
        });
        sessions.truncate(limit);
        Ok(sessions)
    }

    pub async fn update_session(
        &self,
        patch: UpdateSessionPatch,
    ) -> Result<AgentSessionRecord, ToolError> {
        let mut session = self
            .load_session(patch.session_id.as_str())
            .await?
            .ok_or_else(|| {
                ToolError::InvalidArgs(format!("session `{}` not found", patch.session_id))
            })?;

        let mut changed = false;
        if let Some(title) = patch.title {
            session.title = sanitize_title(title)?;
            changed = true;
        }
        if let Some(summary) = patch.summary {
            session.summary = sanitize_summary(summary)?;
            changed = true;
        }
        if let Some(status) = patch.status {
            session.status = normalize_status(status.as_str())?;
            changed = true;
        }
        if !patch.tags_add.is_empty() {
            session.tags.extend(patch.tags_add);
            session.tags = normalize_tags(session.tags);
            changed = true;
        }
        if !patch.tags_remove.is_empty() {
            let remove_set: std::collections::HashSet<String> = patch
                .tags_remove
                .into_iter()
                .map(|item| item.trim().to_ascii_lowercase())
                .collect();
            session.tags = session
                .tags
                .into_iter()
                .filter(|item| !remove_set.contains(item.trim().to_ascii_lowercase().as_str()))
                .collect();
            changed = true;
        }
        if let Some(meta) = patch.meta {
            session.meta = normalize_meta(meta);
            changed = true;
        }
        if !changed && !patch.touch_activity {
            return Err(ToolError::InvalidArgs("update patch is empty".to_string()));
        }

        let ts = now_ms();
        if changed {
            session.updated_at_ms = ts;
        }
        if changed || patch.touch_activity {
            session.last_activity_ms = ts;
        }
        self.save_session(&session).await?;
        Ok(session)
    }

    pub async fn add_link(
        &self,
        session_id: impl AsRef<str>,
        link: SessionLink,
        bidirectional: bool,
    ) -> Result<AgentSessionRecord, ToolError> {
        let mut current = self
            .load_session(session_id.as_ref())
            .await?
            .ok_or_else(|| {
                ToolError::InvalidArgs(format!("session `{}` not found", session_id.as_ref()))
            })?;

        let link = normalize_session_link(link)?;
        if !contains_link(&current.links, &link) {
            current.links.push(link.clone());
            let ts = now_ms();
            current.updated_at_ms = ts;
            current.last_activity_ms = ts;
            self.save_session(&current).await?;
        }

        if bidirectional {
            if let Some(mut target) = self.load_session(link.session_id.as_str()).await? {
                let reverse = SessionLink {
                    relation: reverse_relation(link.relation.as_str()).to_string(),
                    session_id: current.session_id.clone(),
                    agent_did: Some(current.owner_agent.clone()),
                    note: None,
                };
                if !contains_link(&target.links, &reverse) {
                    target.links.push(reverse);
                    let ts = now_ms();
                    target.updated_at_ms = ts;
                    target.last_activity_ms = ts;
                    self.save_session(&target).await?;
                }
            }
        }

        Ok(current)
    }

    pub async fn set_session_title(
        &self,
        session_id: impl AsRef<str>,
        title: impl AsRef<str>,
    ) -> Result<AgentSessionRecord, ToolError> {
        self.update_session(UpdateSessionPatch {
            session_id: sanitize_session_id(session_id.as_ref())?,
            title: Some(title.as_ref().to_string()),
            summary: None,
            status: None,
            tags_add: Vec::new(),
            tags_remove: Vec::new(),
            meta: None,
            touch_activity: true,
        })
        .await
    }

    pub async fn set_session_summary(
        &self,
        session_id: impl AsRef<str>,
        summary: impl AsRef<str>,
    ) -> Result<AgentSessionRecord, ToolError> {
        self.update_session(UpdateSessionPatch {
            session_id: sanitize_session_id(session_id.as_ref())?,
            title: None,
            summary: Some(summary.as_ref().to_string()),
            status: None,
            tags_add: Vec::new(),
            tags_remove: Vec::new(),
            meta: None,
            touch_activity: true,
        })
        .await
    }

    async fn save_session(&self, session: &AgentSessionRecord) -> Result<(), ToolError> {
        let path = self.session_file_path(session.session_id.as_str())?;
        ensure_parent_dir(&path).await?;
        let serialized = serde_json::to_vec_pretty(session)
            .map_err(|err| ToolError::ExecFailed(format!("serialize session failed: {err}")))?;
        fs::write(&path, serialized).await.map_err(|err| {
            ToolError::ExecFailed(format!("write session `{}` failed: {err}", path.display()))
        })?;
        Ok(())
    }

    fn session_file_path(&self, session_id: &str) -> Result<PathBuf, ToolError> {
        let session_id = sanitize_session_id(session_id)?;
        Ok(self
            .sessions_dir
            .join(session_id)
            .join(self.cfg.session_file_name.as_str()))
    }

    async fn load_chat_history(
        &self,
        owner_did: String,
        box_kind: BoxKind,
        session_id: Option<String>,
        limit: usize,
        token_limit: u32,
        cursor_sort_key: Option<u64>,
        cursor_record_id: Option<String>,
        descending: bool,
    ) -> Result<Json, ToolError> {
        let Some(msg_center) = self.msg_center.as_ref() else {
            return Err(ToolError::ExecFailed(
                "msg_center client is not configured".to_string(),
            ));
        };

        let owner = DID::from_str(owner_did.trim())
            .map_err(|err| ToolError::InvalidArgs(format!("invalid `owner_did`: {err}")))?;

        let page = msg_center
            .list_box_by_time(
                owner,
                box_kind,
                None,
                Some(limit.max(1)),
                cursor_sort_key,
                cursor_record_id,
                Some(descending),
            )
            .await
            .map_err(|err| ToolError::ExecFailed(format!("load chat history failed: {err}")))?;

        let filter_session_id = session_id
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());

        let mut messages = Vec::new();
        for item in page.items {
            let payload = serde_json::to_value(&item.msg.payload).unwrap_or(Json::Null);
            let meta = serde_json::to_value(&item.msg.meta).unwrap_or(Json::Null);
            let msg_session_id =
                json_extract_session_id(&payload).or_else(|| json_extract_session_id(&meta));

            if let Some(expect_session) = filter_session_id.as_ref() {
                let mut matched = msg_session_id.as_deref() == Some(expect_session.as_str());
                if !matched {
                    matched = json_matches_session_id(&payload, expect_session.as_str())
                        || json_matches_session_id(&meta, expect_session.as_str());
                }
                if !matched {
                    continue;
                }
            }

            messages.push(json!({
                "record_id": item.record.record_id,
                "session_id": msg_session_id,
                "box_kind": format!("{:?}", item.record.box_kind),
                "state": format!("{:?}", item.record.state),
                "from": item.msg.from.to_string(),
                "to": item.msg.to.iter().map(|did| did.to_string()).collect::<Vec<_>>(),
                "created_at_ms": item.msg.created_at_ms,
                "payload": item.msg.payload,
                "meta": item.msg.meta,
            }));
        }

        let (messages, used_tokens, truncated_by_budget) =
            apply_token_budget(messages, token_limit.max(1));

        Ok(json!({
            "session_id": filter_session_id,
            "limit": limit.max(1),
            "token_limit": token_limit.max(1),
            "used_tokens": used_tokens,
            "truncated_by_token_limit": truncated_by_budget,
            "items": messages,
            "next_cursor_sort_key": page.next_cursor_sort_key,
            "next_cursor_record_id": page.next_cursor_record_id,
        }))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentSessionRecord {
    pub session_id: String,
    pub owner_agent: String,
    pub title: String,
    pub summary: String,
    pub status: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub last_activity_ms: u64,
    pub links: Vec<SessionLink>,
    pub tags: Vec<String>,
    pub meta: Json,
}

impl Default for AgentSessionRecord {
    fn default() -> Self {
        Self {
            session_id: String::new(),
            owner_agent: String::new(),
            title: "Untitled Session".to_string(),
            summary: String::new(),
            status: DEFAULT_SESSION_STATUS.to_string(),
            created_at_ms: 0,
            updated_at_ms: 0,
            last_activity_ms: 0,
            links: Vec::new(),
            tags: Vec::new(),
            meta: default_json_object(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SessionLink {
    pub relation: String,
    pub session_id: String,
    pub agent_did: Option<String>,
    pub note: Option<String>,
}

impl Default for SessionLink {
    fn default() -> Self {
        Self {
            relation: "related".to_string(),
            session_id: String::new(),
            agent_did: None,
            note: None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct CreateSessionRequest {
    pub session_id: Option<String>,
    pub id_prefix: Option<String>,
    pub owner_agent: String,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub status: Option<String>,
    pub tags: Vec<String>,
    pub links: Vec<SessionLink>,
    pub meta: Json,
}

#[derive(Clone, Debug, Default)]
pub struct UpdateSessionPatch {
    pub session_id: String,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub status: Option<String>,
    pub tags_add: Vec<String>,
    pub tags_remove: Vec<String>,
    pub meta: Option<Json>,
    pub touch_activity: bool,
}

#[derive(Clone)]
struct ListSessionsTool {
    session: AgentSession,
}

#[async_trait]
impl AgentTool for ListSessionsTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_LIST_SESSIONS.to_string(),
            description: "List session objects under agent/session directory.".to_string(),
            args_schema: json!({
                "type":"object",
                "properties": {
                    "limit": {"type":"integer", "minimum": 1},
                    "include_deleted": {"type":"boolean"}
                },
                "additionalProperties": true
            }),
            output_schema: json!({
                "type":"object",
                "properties": {
                    "sessions_dir": {"type":"string"},
                    "sessions": {"type":"array"}
                }
            }),
        }
    }

    async fn call(&self, _ctx: &TraceCtx, args: Json) -> Result<Json, ToolError> {
        let limit = optional_usize(&args, "limit")?.unwrap_or(self.session.cfg.default_list_limit);
        let include_deleted = optional_bool(&args, "include_deleted")?.unwrap_or(false);
        let sessions = self.session.list_sessions(limit, include_deleted).await?;
        Ok(json!({
            "sessions_dir": self.session.sessions_dir.to_string_lossy().to_string(),
            "sessions": sessions,
        }))
    }
}

#[derive(Clone)]
struct CreateSessionTool {
    session: AgentSession,
}

#[async_trait]
impl AgentTool for CreateSessionTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_CREATE_SESSION.to_string(),
            description: "Create a new agent session object.".to_string(),
            args_schema: json!({
                "type":"object",
                "properties": {
                    "session_id": {"type":"string"},
                    "id_prefix": {"type":"string"},
                    "owner_agent": {"type":"string"},
                    "title": {"type":"string"},
                    "summary": {"type":"string"},
                    "status": {"type":"string"},
                    "tags": {"type":"array", "items":{"type":"string"}},
                    "links": {"type":"array", "items":{"type":"object"}},
                    "meta": {"type":"object"}
                },
                "additionalProperties": true
            }),
            output_schema: json!({
                "type":"object",
                "properties": {
                    "session": {"type":"object"}
                }
            }),
        }
    }

    async fn call(&self, ctx: &TraceCtx, args: Json) -> Result<Json, ToolError> {
        let owner_agent = optional_string(&args, "owner_agent")?.unwrap_or(ctx.agent_did.clone());
        let links = optional_links(&args, "links")?;
        let req = CreateSessionRequest {
            session_id: optional_string(&args, "session_id")?,
            id_prefix: optional_string(&args, "id_prefix")?,
            owner_agent,
            title: optional_string(&args, "title")?,
            summary: optional_string(&args, "summary")?,
            status: optional_string(&args, "status")?,
            tags: optional_string_array(&args, "tags")?.unwrap_or_default(),
            links,
            meta: args.get("meta").cloned().unwrap_or(default_json_object()),
        };
        let session = self.session.create_session(req).await?;
        Ok(json!({ "session": session }))
    }
}

#[derive(Clone)]
struct GetSessionTool {
    session: AgentSession,
}

#[async_trait]
impl AgentTool for GetSessionTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_GET_SESSION.to_string(),
            description: "Load one session object by session_id.".to_string(),
            args_schema: json!({
                "type":"object",
                "properties": {
                    "session_id": {"type":"string"}
                },
                "required": ["session_id"],
                "additionalProperties": true
            }),
            output_schema: json!({
                "type":"object",
                "properties": {
                    "session": {"type":"object"}
                }
            }),
        }
    }

    async fn call(&self, _ctx: &TraceCtx, args: Json) -> Result<Json, ToolError> {
        let session_id = require_string(&args, "session_id")?;
        let Some(session) = self.session.load_session(session_id).await? else {
            return Err(ToolError::InvalidArgs("session not found".to_string()));
        };
        Ok(json!({ "session": session }))
    }
}

#[derive(Clone)]
struct UpdateSessionTool {
    session: AgentSession,
}

#[async_trait]
impl AgentTool for UpdateSessionTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_UPDATE_SESSION.to_string(),
            description: "Update session title/summary/status/tags/meta.".to_string(),
            args_schema: json!({
                "type":"object",
                "properties": {
                    "session_id": {"type":"string"},
                    "title": {"type":"string"},
                    "summary": {"type":"string"},
                    "status": {"type":"string"},
                    "tags_add": {"type":"array", "items":{"type":"string"}},
                    "tags_remove": {"type":"array", "items":{"type":"string"}},
                    "meta": {"type":"object"},
                    "touch_activity": {"type":"boolean"}
                },
                "required": ["session_id"],
                "additionalProperties": true
            }),
            output_schema: json!({
                "type":"object",
                "properties": {
                    "session": {"type":"object"}
                }
            }),
        }
    }

    async fn call(&self, _ctx: &TraceCtx, args: Json) -> Result<Json, ToolError> {
        let patch = UpdateSessionPatch {
            session_id: require_string(&args, "session_id")?,
            title: optional_string(&args, "title")?,
            summary: optional_string(&args, "summary")?,
            status: optional_string(&args, "status")?,
            tags_add: optional_string_array(&args, "tags_add")?.unwrap_or_default(),
            tags_remove: optional_string_array(&args, "tags_remove")?.unwrap_or_default(),
            meta: args.get("meta").cloned(),
            touch_activity: optional_bool(&args, "touch_activity")?.unwrap_or(true),
        };
        let session = self.session.update_session(patch).await?;
        Ok(json!({ "session": session }))
    }
}

#[derive(Clone)]
struct LinkSessionsTool {
    session: AgentSession,
}

#[async_trait]
impl AgentTool for LinkSessionsTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_LINK_SESSIONS.to_string(),
            description: "Create relationship between sessions.".to_string(),
            args_schema: json!({
                "type":"object",
                "properties": {
                    "session_id": {"type":"string"},
                    "target_session_id": {"type":"string"},
                    "relation": {"type":"string"},
                    "target_agent_did": {"type":"string"},
                    "note": {"type":"string"},
                    "bidirectional": {"type":"boolean"}
                },
                "required": ["session_id", "target_session_id"],
                "additionalProperties": true
            }),
            output_schema: json!({
                "type":"object",
                "properties": {
                    "session": {"type":"object"}
                }
            }),
        }
    }

    async fn call(&self, _ctx: &TraceCtx, args: Json) -> Result<Json, ToolError> {
        let session_id = require_string(&args, "session_id")?;
        let target_session_id = require_string(&args, "target_session_id")?;
        let relation = optional_string(&args, "relation")?.unwrap_or_else(|| "related".to_string());
        let target_agent_did = optional_string(&args, "target_agent_did")?;
        let note = optional_string(&args, "note")?;
        let bidirectional = optional_bool(&args, "bidirectional")?.unwrap_or(false);

        let session = self
            .session
            .add_link(
                session_id,
                SessionLink {
                    relation,
                    session_id: target_session_id,
                    agent_did: target_agent_did,
                    note,
                },
                bidirectional,
            )
            .await?;

        Ok(json!({ "session": session }))
    }
}

#[derive(Clone)]
struct LoadChatHistoryTool {
    session: AgentSession,
    tool_name: String,
}

#[async_trait]
impl AgentTool for LoadChatHistoryTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.tool_name.clone(),
            description: "Load chat history via MsgCenter and optionally filter by session_id."
                .to_string(),
            args_schema: json!({
                "type":"object",
                "properties": {
                    "owner_did": {"type":"string"},
                    "box_kind": {"type":"string"},
                    "session_id": {"type":"string"},
                    "limit": {"type":"integer", "minimum": 1},
                    "token_limit": {"type":"integer", "minimum": 1},
                    "cursor_sort_key": {"type":"integer", "minimum": 0},
                    "cursor_record_id": {"type":"string"},
                    "descending": {"type":"boolean"}
                },
                "required": ["owner_did"],
                "additionalProperties": true
            }),
            output_schema: json!({
                "type":"object",
                "properties": {
                    "session_id": {"type":["string","null"]},
                    "limit": {"type":"integer"},
                    "token_limit": {"type":"integer"},
                    "used_tokens": {"type":"integer"},
                    "truncated_by_token_limit": {"type":"boolean"},
                    "items": {"type":"array"},
                    "next_cursor_sort_key": {"type":["integer","null"]},
                    "next_cursor_record_id": {"type":["string","null"]}
                }
            }),
        }
    }

    async fn call(&self, _ctx: &TraceCtx, args: Json) -> Result<Json, ToolError> {
        let owner_did = require_string(&args, "owner_did")?;
        let box_kind = parse_box_kind(optional_string(&args, "box_kind")?)?;
        let session_id = optional_string(&args, "session_id")?;
        let limit = optional_usize(&args, "limit")?.unwrap_or(self.session.cfg.default_chat_limit);
        let token_limit = optional_u32(&args, "token_limit")?
            .unwrap_or(self.session.cfg.default_chat_token_limit);
        let cursor_sort_key = optional_u64(&args, "cursor_sort_key")?;
        let cursor_record_id = optional_string(&args, "cursor_record_id")?;
        let descending = optional_bool(&args, "descending")?.unwrap_or(true);

        self.session
            .load_chat_history(
                owner_did,
                box_kind,
                session_id,
                limit,
                token_limit,
                cursor_sort_key,
                cursor_record_id,
                descending,
            )
            .await
    }
}

fn default_json_object() -> Json {
    Json::Object(serde_json::Map::new())
}

fn normalize_meta(meta: Json) -> Json {
    match meta {
        Json::Object(_) => meta,
        _ => default_json_object(),
    }
}

fn sanitize_non_empty(input: &str, field_name: &str) -> Result<String, ToolError> {
    let value = input.trim();
    if value.is_empty() {
        return Err(ToolError::InvalidArgs(format!(
            "field `{field_name}` cannot be empty"
        )));
    }
    Ok(value.to_string())
}

fn sanitize_session_id(session_id: &str) -> Result<String, ToolError> {
    let session_id = session_id.trim();
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

fn sanitize_title(title: String) -> Result<String, ToolError> {
    let title = title.trim();
    if title.is_empty() {
        return Err(ToolError::InvalidArgs("title cannot be empty".to_string()));
    }
    if title.chars().count() > MAX_TITLE_LEN {
        return Err(ToolError::InvalidArgs(format!(
            "title too long (>{MAX_TITLE_LEN} chars)"
        )));
    }
    Ok(title.to_string())
}

fn sanitize_summary(summary: String) -> Result<String, ToolError> {
    if summary.chars().count() > MAX_SUMMARY_LEN {
        return Err(ToolError::InvalidArgs(format!(
            "summary too long (>{MAX_SUMMARY_LEN} chars)"
        )));
    }
    Ok(summary.trim().to_string())
}

fn normalize_status(status: &str) -> Result<String, ToolError> {
    let normalized = status.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "active" | "archived" | "deleted" => Ok(normalized),
        _ => Err(ToolError::InvalidArgs(format!(
            "unsupported status `{status}`"
        ))),
    }
}

fn normalize_tags(tags: Vec<String>) -> Vec<String> {
    let mut out = Vec::<String>::new();
    for tag in tags {
        let normalized = tag.trim();
        if normalized.is_empty() {
            continue;
        }
        if out.iter().any(|item| item.eq_ignore_ascii_case(normalized)) {
            continue;
        }
        out.push(normalized.to_string());
    }
    out
}

fn normalize_session_link(link: SessionLink) -> Result<SessionLink, ToolError> {
    let relation = link.relation.trim();
    let relation = if relation.is_empty() {
        "related".to_string()
    } else {
        relation.to_ascii_lowercase()
    };
    let session_id = sanitize_session_id(link.session_id.as_str())?;
    let agent_did = link
        .agent_did
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty());
    let note = link
        .note
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty());

    Ok(SessionLink {
        relation,
        session_id,
        agent_did,
        note,
    })
}

fn contains_link(links: &[SessionLink], target: &SessionLink) -> bool {
    links.iter().any(|item| {
        item.relation == target.relation
            && item.session_id == target.session_id
            && item.agent_did == target.agent_did
    })
}

fn reverse_relation(relation: &str) -> &str {
    match relation {
        "parent" => "child",
        "child" => "parent",
        _ => relation,
    }
}

fn parse_box_kind(raw: Option<String>) -> Result<BoxKind, ToolError> {
    let Some(raw) = raw else {
        return Ok(BoxKind::Inbox);
    };
    let normalized = raw.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "inbox" | "in_box" => Ok(BoxKind::Inbox),
        "outbox" | "out_box" => Ok(BoxKind::Outbox),
        "groupinbox" | "group_inbox" => Ok(BoxKind::GroupInbox),
        "tunneloutbox" | "tunnel_outbox" => Ok(BoxKind::TunnelOutbox),
        "requestbox" | "request_box" => Ok(BoxKind::RequestBox),
        _ => Err(ToolError::InvalidArgs(format!(
            "unsupported `box_kind`: {raw}"
        ))),
    }
}

fn json_extract_session_id(value: &Json) -> Option<String> {
    if value.is_null() {
        return None;
    }

    let keys = [
        "/session_id",
        "/session",
        "/meta/session_id",
        "/meta/session",
        "/record/session_id",
    ];
    for key in keys {
        let found = value
            .pointer(key)
            .and_then(|v| v.as_str())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        if found.is_some() {
            return found;
        }
    }
    None
}

fn json_matches_session_id(value: &Json, session_id: &str) -> bool {
    json_extract_session_id(value)
        .as_deref()
        .is_some_and(|v| v == session_id)
}

fn approx_tokens(text: &str) -> u32 {
    let chars = text.chars().count();
    ((chars + 3) / 4) as u32
}

fn apply_token_budget(items: Vec<Json>, token_limit: u32) -> (Vec<Json>, u32, bool) {
    let mut selected = Vec::new();
    let mut used_tokens = 0_u32;
    let mut truncated = false;

    for item in items {
        let raw = serde_json::to_string(&item).unwrap_or_default();
        let item_tokens = approx_tokens(&raw).max(1);
        if used_tokens.saturating_add(item_tokens) > token_limit && !selected.is_empty() {
            truncated = true;
            break;
        }
        used_tokens = used_tokens.saturating_add(item_tokens);
        selected.push(item);
        if used_tokens >= token_limit {
            truncated = true;
            break;
        }
    }

    (selected, used_tokens, truncated)
}

fn ensure_safe_file_name(file_name: &str) -> Result<(), ToolError> {
    let file_name = file_name.trim();
    if file_name.is_empty() {
        return Err(ToolError::InvalidArgs(
            "session_file_name cannot be empty".to_string(),
        ));
    }
    if file_name.contains('/') || file_name.contains('\\') {
        return Err(ToolError::InvalidArgs(
            "session_file_name cannot contain path separators".to_string(),
        ));
    }
    if file_name == "." || file_name == ".." {
        return Err(ToolError::InvalidArgs(
            "session_file_name cannot be `.` or `..`".to_string(),
        ));
    }
    Ok(())
}

async fn normalize_root(root: &Path) -> Result<PathBuf, ToolError> {
    if root.as_os_str().is_empty() {
        return Err(ToolError::InvalidArgs(
            "agent_root cannot be empty".to_string(),
        ));
    }
    fs::create_dir_all(root)
        .await
        .map_err(|err| ToolError::ExecFailed(format!("create agent_root failed: {err}")))?;
    fs::canonicalize(root)
        .await
        .map_err(|err| ToolError::ExecFailed(format!("canonicalize agent_root failed: {err}")))
}

fn resolve_relative_path(root: &Path, rel_path: &Path) -> Result<PathBuf, ToolError> {
    if rel_path.is_absolute() {
        return Err(ToolError::InvalidArgs(format!(
            "path `{}` must be relative",
            rel_path.display()
        )));
    }

    for component in rel_path.components() {
        if matches!(component, Component::ParentDir) {
            return Err(ToolError::InvalidArgs(format!(
                "path `{}` cannot contain `..`",
                rel_path.display()
            )));
        }
    }

    Ok(root.join(rel_path))
}

async fn ensure_parent_dir(path: &Path) -> Result<(), ToolError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    fs::create_dir_all(parent).await.map_err(|err| {
        ToolError::ExecFailed(format!(
            "create parent dir `{}` failed: {err}",
            parent.display()
        ))
    })?;
    Ok(())
}

fn now_ms() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_millis().min(u128::from(u64::MAX)) as u64,
        Err(_) => 0,
    }
}

fn require_string(args: &Json, key: &str) -> Result<String, ToolError> {
    let value = args
        .get(key)
        .ok_or_else(|| ToolError::InvalidArgs(format!("missing required arg `{key}`")))?;
    let value = value
        .as_str()
        .ok_or_else(|| ToolError::InvalidArgs(format!("arg `{key}` must be a string")))?;
    Ok(value.to_string())
}

fn optional_string(args: &Json, key: &str) -> Result<Option<String>, ToolError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let value = value
        .as_str()
        .ok_or_else(|| ToolError::InvalidArgs(format!("arg `{key}` must be a string")))?;
    Ok(Some(value.to_string()))
}

fn optional_bool(args: &Json, key: &str) -> Result<Option<bool>, ToolError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let value = value
        .as_bool()
        .ok_or_else(|| ToolError::InvalidArgs(format!("arg `{key}` must be a boolean")))?;
    Ok(Some(value))
}

fn optional_u32(args: &Json, key: &str) -> Result<Option<u32>, ToolError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let value = value
        .as_u64()
        .ok_or_else(|| ToolError::InvalidArgs(format!("arg `{key}` must be a positive integer")))?;
    u32::try_from(value)
        .map(Some)
        .map_err(|_| ToolError::InvalidArgs(format!("arg `{key}` is too large")))
}

fn optional_u64(args: &Json, key: &str) -> Result<Option<u64>, ToolError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let value = value
        .as_u64()
        .ok_or_else(|| ToolError::InvalidArgs(format!("arg `{key}` must be a positive integer")))?;
    Ok(Some(value))
}

fn optional_usize(args: &Json, key: &str) -> Result<Option<usize>, ToolError> {
    let value = optional_u64(args, key)?;
    value
        .map(|raw| {
            usize::try_from(raw)
                .map_err(|_| ToolError::InvalidArgs(format!("arg `{key}` is too large")))
        })
        .transpose()
}

fn optional_string_array(args: &Json, key: &str) -> Result<Option<Vec<String>>, ToolError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let arr = value
        .as_array()
        .ok_or_else(|| ToolError::InvalidArgs(format!("arg `{key}` must be an array")))?;
    let mut out = Vec::new();
    for item in arr {
        let s = item
            .as_str()
            .ok_or_else(|| ToolError::InvalidArgs(format!("arg `{key}` must contain strings")))?;
        out.push(s.to_string());
    }
    Ok(Some(out))
}

fn optional_links(args: &Json, key: &str) -> Result<Vec<SessionLink>, ToolError> {
    let Some(value) = args.get(key) else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    let arr = value
        .as_array()
        .ok_or_else(|| ToolError::InvalidArgs(format!("arg `{key}` must be an array")))?;
    let mut links = Vec::new();
    for item in arr {
        let link: SessionLink = serde_json::from_value(item.clone())
            .map_err(|err| ToolError::InvalidArgs(format!("invalid link in `{key}`: {err}")))?;
        links.push(link);
    }
    Ok(links)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_tool::ToolCall;
    use crate::behavior::TraceCtx;
    use tempfile::tempdir;

    fn test_ctx() -> TraceCtx {
        TraceCtx {
            trace_id: "trace-session".to_string(),
            agent_did: "did:example:agent".to_string(),
            behavior: "on_wakeup".to_string(),
            step_idx: 0,
            wakeup_id: "wakeup-session".to_string(),
        }
    }

    #[tokio::test]
    async fn session_crud_and_links_work() {
        let tmp = tempdir().expect("create tempdir");
        let store = AgentSession::new(AgentSessionConfig::new(tmp.path()), None)
            .await
            .expect("create session store");

        let s1 = store
            .create_session(CreateSessionRequest {
                session_id: Some("session-main".to_string()),
                owner_agent: "did:example:agent".to_string(),
                title: Some("Main Session".to_string()),
                summary: Some("initial summary".to_string()),
                ..Default::default()
            })
            .await
            .expect("create session 1");
        assert_eq!(s1.session_id, "session-main");
        assert_eq!(s1.title, "Main Session");

        let _ = store
            .create_session(CreateSessionRequest {
                session_id: Some("session-sub".to_string()),
                owner_agent: "did:example:agent".to_string(),
                title: Some("Sub Session".to_string()),
                ..Default::default()
            })
            .await
            .expect("create session 2");

        let updated = store
            .update_session(UpdateSessionPatch {
                session_id: "session-main".to_string(),
                title: Some("Main Session v2".to_string()),
                summary: Some("summary v2".to_string()),
                touch_activity: true,
                ..Default::default()
            })
            .await
            .expect("update session");
        assert_eq!(updated.title, "Main Session v2");
        assert_eq!(updated.summary, "summary v2");

        let linked = store
            .add_link(
                "session-main",
                SessionLink {
                    relation: "child".to_string(),
                    session_id: "session-sub".to_string(),
                    agent_did: Some("did:example:agent".to_string()),
                    note: None,
                },
                true,
            )
            .await
            .expect("link sessions");
        assert_eq!(linked.links.len(), 1);
        assert_eq!(linked.links[0].relation, "child");

        let sub = store
            .load_session("session-sub")
            .await
            .expect("load sub")
            .expect("sub should exist");
        assert_eq!(sub.links.len(), 1);
        assert_eq!(sub.links[0].relation, "parent");

        let list = store.list_sessions(10, false).await.expect("list sessions");
        assert_eq!(list.len(), 2);
    }

    #[tokio::test]
    async fn invalid_session_id_rejected() {
        let tmp = tempdir().expect("create tempdir");
        let store = AgentSession::new(AgentSessionConfig::new(tmp.path()), None)
            .await
            .expect("create session store");

        let err = store
            .create_session(CreateSessionRequest {
                session_id: Some("../bad".to_string()),
                owner_agent: "did:example:agent".to_string(),
                ..Default::default()
            })
            .await
            .expect_err("invalid session id should fail");
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn load_chat_history_requires_msg_center_client() {
        let tmp = tempdir().expect("create tempdir");
        let store = AgentSession::new(AgentSessionConfig::new(tmp.path()), None)
            .await
            .expect("create session store");
        let tools = ToolManager::new();
        store.register_tools(&tools).expect("register tools");

        let err = tools
            .call_tool(
                &test_ctx(),
                ToolCall {
                    name: TOOL_LOAD_CHAT_HISTORY.to_string(),
                    args: json!({"owner_did":"did:bns:alice"}),
                    call_id: "load-chat-1".to_string(),
                },
            )
            .await
            .expect_err("missing msg center should fail");
        assert!(matches!(err, ToolError::ExecFailed(_)));
    }
}
