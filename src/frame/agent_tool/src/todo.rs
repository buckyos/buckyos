use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use buckyos_api::KEventClient;
use log::info;
use rusqlite::{params, params_from_iter, types::Value as SqlValue, Connection};
use serde::Serialize;
use serde_json::{json, Value as Json};
use tokio::task;

use crate::{
    now_ms, optional_trimmed_string_arg, require_trimmed_string_arg, AgentToolError,
    SessionRuntimeContext,
};

pub(crate) use crate::optional_trimmed_string_arg as optional_string;
pub(crate) use crate::optional_u64_arg as optional_u64;
pub(crate) use crate::require_trimmed_string_arg as require_string;
pub(crate) use crate::u64_to_usize_arg as u64_to_usize;

const DEFAULT_LIST_LIMIT: usize = 32;
const DEFAULT_MAX_LIST_LIMIT: usize = 128;
const MAX_TEXT_256: usize = 256;
const MAX_TEXT_1024: usize = 1024;
const MAX_TEXT_4096: usize = 4096;
const MAX_LABELS: usize = 32;
const MAX_SKILLS: usize = 32;
const MAX_DEPS: usize = 128;
const MAX_NOTES_FETCH: usize = 100;
const RENDER_ITEM_LIMIT: usize = 64;
const DEFAULT_TOKEN_BUDGET: usize = 1600;
const DEFAULT_PRIORITY_GAP: i64 = 10;

static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
pub struct TodoToolConfig {
    pub db_path: PathBuf,
    pub default_list_limit: usize,
    pub max_list_limit: usize,
}

impl TodoToolConfig {
    pub fn with_db_path(db_path: PathBuf) -> Self {
        Self {
            db_path,
            default_list_limit: DEFAULT_LIST_LIMIT,
            max_list_limit: DEFAULT_MAX_LIST_LIMIT,
        }
    }
}

#[derive(Clone)]
pub struct TodoTool {
    cfg: TodoToolConfig,
    oplog_path: PathBuf,
    kevent_client: KEventClient,
}

impl TodoTool {
    pub fn new(mut cfg: TodoToolConfig) -> Result<Self, AgentToolError> {
        if cfg.default_list_limit == 0 {
            cfg.default_list_limit = DEFAULT_LIST_LIMIT;
        }
        if cfg.max_list_limit == 0 {
            cfg.max_list_limit = DEFAULT_MAX_LIST_LIMIT;
        }
        if cfg.default_list_limit > cfg.max_list_limit {
            cfg.default_list_limit = cfg.max_list_limit;
        }

        if let Some(parent) = cfg.db_path.parent() {
            if !parent.exists() {
                info!(
                    "opendan.persist_entity_prepare: kind=todo_db_parent_dir path={}",
                    parent.display()
                );
            }
            std::fs::create_dir_all(parent).map_err(|err| {
                AgentToolError::ExecFailed(format!(
                    "create todo db parent dir `{}` failed: {err}",
                    parent.display()
                ))
            })?;
        }
        if !cfg.db_path.exists() {
            info!(
                "opendan.persist_entity_prepare: kind=todo_db_file path={}",
                cfg.db_path.display()
            );
        }

        let conn = Connection::open(&cfg.db_path).map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "open todo db `{}` failed: {err}",
                cfg.db_path.display()
            ))
        })?;
        ensure_todo_schema(&conn)?;

        let oplog_path = cfg
            .db_path
            .parent()
            .map(|v| v.join("oplog.jsonl"))
            .unwrap_or_else(|| PathBuf::from("oplog.jsonl"));

        Ok(Self {
            cfg,
            oplog_path,
            kevent_client: KEventClient::new_full("opendan-todo", None),
        })
    }

    async fn run_db<F, T>(&self, op_name: &str, op: F) -> Result<T, AgentToolError>
    where
        F: FnOnce(&mut Connection) -> Result<T, AgentToolError> + Send + 'static,
        T: Send + 'static,
    {
        let db_path = self.cfg.db_path.clone();
        task::spawn_blocking(move || {
            let mut conn = Connection::open(&db_path).map_err(|err| {
                AgentToolError::ExecFailed(format!(
                    "open todo db `{}` failed: {err}",
                    db_path.display()
                ))
            })?;
            ensure_todo_schema(&conn)?;
            op(&mut conn)
        })
        .await
        .map_err(|err| AgentToolError::ExecFailed(format!("{op_name} join error: {err}")))?
    }
}

#[path = "agent_todo_tool.rs"]
mod agent_todo_tool;

#[derive(Clone, Debug, PartialEq, Eq)]
enum TodoType {
    Task,
    Bench,
}

impl TodoType {
    fn parse(raw: &str) -> Result<Self, AgentToolError> {
        let value = normalize_enum(raw);
        match value.as_str() {
            "task" => Ok(Self::Task),
            "bench" => Ok(Self::Bench),
            _ => Err(AgentToolError::InvalidArgs(format!(
                "invalid todo type `{raw}`; allowed: Task|Bench"
            ))),
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            Self::Task => "Task",
            Self::Bench => "Bench",
        }
    }

    fn from_db(raw: &str) -> Result<Self, AgentToolError> {
        Self::parse(raw)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum TodoStatus {
    Wait,
    InProgress,
    Complete,
    Failed,
    Done,
    CheckFailed,
}

impl TodoStatus {
    fn parse(raw: &str) -> Result<Self, AgentToolError> {
        let value = normalize_enum(raw);
        match value.as_str() {
            "wait" => Ok(Self::Wait),
            "in_progress" => Ok(Self::InProgress),
            "complete" => Ok(Self::Complete),
            "failed" => Ok(Self::Failed),
            "done" => Ok(Self::Done),
            "check_failed" => Ok(Self::CheckFailed),
            _ => Err(AgentToolError::InvalidArgs(format!(
                "invalid todo status `{raw}`; allowed: WAIT|IN_PROGRESS|COMPLETE|FAILED|DONE|CHECK_FAILED"
            ))),
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            Self::Wait => "WAIT",
            Self::InProgress => "IN_PROGRESS",
            Self::Complete => "COMPLETE",
            Self::Failed => "FAILED",
            Self::Done => "DONE",
            Self::CheckFailed => "CHECK_FAILED",
        }
    }

    fn from_db(raw: &str) -> Result<Self, AgentToolError> {
        Self::parse(raw)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ActorKind {
    RootAgent,
    SubAgent,
    User,
    System,
}

impl ActorKind {
    fn parse(raw: &str) -> Result<Self, AgentToolError> {
        let value = normalize_enum(raw);
        match value.as_str() {
            "root_agent" => Ok(Self::RootAgent),
            "sub_agent" => Ok(Self::SubAgent),
            "user" => Ok(Self::User),
            "system" => Ok(Self::System),
            _ => Err(AgentToolError::InvalidArgs(format!(
                "invalid actor kind `{raw}`; allowed: root_agent|sub_agent|user|system"
            ))),
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            Self::RootAgent => "root_agent",
            Self::SubAgent => "sub_agent",
            Self::User => "user",
            Self::System => "system",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct ActorRefOut {
    kind: String,
    did: String,
}

#[derive(Clone, Debug)]
struct ActorCtx {
    kind: ActorKind,
    did: String,
    session_id: Option<String>,
    trace_id: Option<String>,
}

impl ActorCtx {
    fn from_args(ctx: &SessionRuntimeContext, args: &Json) -> Result<Self, AgentToolError> {
        let actor_raw = args.get("actor_ctx").and_then(|v| v.as_object());
        let kind = actor_raw
            .and_then(|m| m.get("kind"))
            .and_then(|v| v.as_str())
            .map(ActorKind::parse)
            .transpose()?
            .unwrap_or(ActorKind::RootAgent);

        let did = actor_raw
            .and_then(|m| m.get("did"))
            .and_then(|v| v.as_str())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| {
                let v = ctx.agent_name.trim();
                if v.is_empty() {
                    "did:opendan:unknown".to_string()
                } else {
                    v.to_string()
                }
            });

        let session_id = actor_raw
            .and_then(|m| m.get("session_id"))
            .and_then(|v| v.as_str())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .or_else(|| {
                optional_trimmed_string_arg(args, "session_id")
                    .ok()
                    .flatten()
            });

        let trace_id = actor_raw
            .and_then(|m| m.get("trace_id"))
            .and_then(|v| v.as_str())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .or_else(|| {
                let v = ctx.trace_id.trim();
                if v.is_empty() {
                    None
                } else {
                    Some(v.to_string())
                }
            });

        Ok(Self {
            kind,
            did,
            session_id,
            trace_id,
        })
    }

    fn out(&self) -> ActorRefOut {
        ActorRefOut {
            kind: self.kind.as_str().to_string(),
            did: self.did.clone(),
        }
    }
}

#[derive(Clone, Debug)]
struct TodoListFilters {
    statuses: Vec<TodoStatus>,
    todo_type: Option<TodoType>,
    assignee: Option<String>,
    label: Option<String>,
    query: Option<String>,
    sort_by: Option<String>,
    asc: bool,
}

impl TodoListFilters {
    fn from_args(args: &Json) -> Result<Self, AgentToolError> {
        let mut statuses = Vec::new();
        let mut todo_type = None;
        let mut assignee = None;
        let mut label = None;
        let mut query = None;
        let mut sort_by = None;
        let mut asc = false;

        if let Some(filters) = args.get("filters") {
            let map = filters.as_object().ok_or_else(|| {
                AgentToolError::InvalidArgs("`filters` must be a json object".to_string())
            })?;
            if let Some(statuses_raw) = map.get("status") {
                statuses = parse_status_list(Some(statuses_raw))?;
            }
            if let Some(type_raw) = map.get("type").and_then(|v| v.as_str()) {
                todo_type = Some(TodoType::parse(type_raw)?);
            }
            assignee = map
                .get("assignee")
                .and_then(|v| v.as_str())
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty());
            label = map
                .get("label")
                .and_then(|v| v.as_str())
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty());
            query = map
                .get("query")
                .and_then(|v| v.as_str())
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty());
            sort_by = map
                .get("sort_by")
                .and_then(|v| v.as_str())
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty());
            asc = map.get("asc").and_then(|v| v.as_bool()).unwrap_or(false);
        }

        if statuses.is_empty() {
            statuses = parse_status_list(args.get("status"))?;
        }
        if todo_type.is_none() {
            todo_type = optional_trimmed_string_arg(args, "type")?
                .map(|v| TodoType::parse(&v))
                .transpose()?;
        }
        if assignee.is_none() {
            assignee = optional_trimmed_string_arg(args, "assignee")?;
        }
        if label.is_none() {
            label = optional_trimmed_string_arg(args, "label")?;
        }
        if query.is_none() {
            query = optional_trimmed_string_arg(args, "query")?;
        }
        if sort_by.is_none() {
            sort_by = optional_trimmed_string_arg(args, "sort_by")?;
        }
        if !asc {
            asc = optional_bool(args, "asc")?.unwrap_or(false);
        }

        Ok(Self {
            statuses,
            todo_type,
            assignee,
            label,
            query,
            sort_by,
            asc,
        })
    }
}

#[derive(Clone, Debug)]
struct ApplyDeltaInput {
    workspace_id: String,
    op_id: String,
    actor: ActorCtx,
    ops: Vec<DeltaOp>,
}

impl ApplyDeltaInput {
    fn from_args(ctx: &SessionRuntimeContext, args: &Json) -> Result<Self, AgentToolError> {
        let workspace_id = require_workspace_id(args)?;
        let actor = ActorCtx::from_args(ctx, args)?;

        let delta_obj = args
            .get("delta")
            .or_else(|| args.get("todo_delta"))
            .unwrap_or(args);
        let delta = delta_obj.as_object().ok_or_else(|| {
            AgentToolError::InvalidArgs("`delta` must be a json object".to_string())
        })?;

        let op_id = delta
            .get("op_id")
            .and_then(|v| v.as_str())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .or_else(|| {
                args.get("op_id")
                    .and_then(|v| v.as_str())
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
            })
            .unwrap_or_else(|| generate_id("op"));

        let ops_json = delta
            .get("ops")
            .or_else(|| args.get("ops"))
            .ok_or_else(|| AgentToolError::InvalidArgs("missing `delta.ops`".to_string()))?;
        let ops_arr = ops_json.as_array().ok_or_else(|| {
            AgentToolError::InvalidArgs("`delta.ops` must be an array".to_string())
        })?;
        if ops_arr.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "`delta.ops` cannot be empty".to_string(),
            ));
        }

        let mut ops = Vec::with_capacity(ops_arr.len());
        for op in ops_arr {
            ops.push(DeltaOp::parse(op)?);
        }

        Ok(Self {
            workspace_id,
            op_id,
            actor,
            ops,
        })
    }
}

#[derive(Clone, Debug)]
enum DeltaOp {
    Init {
        mode: InitMode,
        items: Vec<InitTodoItem>,
        raw: Json,
    },
    Update {
        todo_code: String,
        to_status: TodoStatus,
        reason: String,
        last_error: Option<Json>,
        raw: Json,
    },
    Note {
        todo_code: String,
        kind: String,
        content: String,
        raw: Json,
    },
}

impl DeltaOp {
    fn parse(value: &Json) -> Result<Self, AgentToolError> {
        let map = value.as_object().ok_or_else(|| {
            AgentToolError::InvalidArgs("each op in delta.ops must be a json object".to_string())
        })?;
        let op = map
            .get("op")
            .and_then(|v| v.as_str())
            .map(|v| v.trim().to_string())
            .ok_or_else(|| AgentToolError::InvalidArgs("delta op missing `op`".to_string()))?;

        if op == "init" {
            let mode = map
                .get("mode")
                .and_then(|v| v.as_str())
                .map(InitMode::parse)
                .transpose()?
                .unwrap_or(InitMode::Replace);
            let items_raw = map.get("items").and_then(|v| v.as_array()).ok_or_else(|| {
                AgentToolError::InvalidArgs("init op missing `items[]`".to_string())
            })?;
            if items_raw.is_empty() && !matches!(mode, InitMode::Replace) {
                return Err(AgentToolError::InvalidArgs(
                    "init op `items` cannot be empty unless mode=replace".to_string(),
                ));
            }
            let mut items = Vec::with_capacity(items_raw.len());
            for item in items_raw {
                items.push(InitTodoItem::parse(item)?);
            }
            return Ok(Self::Init {
                mode,
                items,
                raw: value.clone(),
            });
        }

        if let Some(todo_code) = op.strip_prefix("update:") {
            let code = normalize_todo_code(todo_code)?;
            let to_status = map
                .get("to_status")
                .and_then(|v| v.as_str())
                .map(TodoStatus::parse)
                .transpose()?
                .ok_or_else(|| {
                    AgentToolError::InvalidArgs("update op missing `to_status`".to_string())
                })?;
            let reason = map
                .get("reason")
                .and_then(|v| v.as_str())
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .ok_or_else(|| {
                    AgentToolError::InvalidArgs("update op missing `reason`".to_string())
                })?;
            if reason.chars().count() > MAX_TEXT_1024 {
                return Err(AgentToolError::InvalidArgs(format!(
                    "update reason exceeds max {} chars",
                    MAX_TEXT_1024
                )));
            }
            let last_error = if let Some(err_obj) = map.get("last_error") {
                let bytes = serde_json::to_vec(err_obj)
                    .map_err(|err| {
                        AgentToolError::InvalidArgs(format!("serialize last_error failed: {err}"))
                    })?
                    .len();
                if bytes > 16 * 1024 {
                    return Err(AgentToolError::InvalidArgs(
                        "`last_error` too large (max 16KB)".to_string(),
                    ));
                }
                Some(err_obj.clone())
            } else {
                None
            };
            return Ok(Self::Update {
                todo_code: code,
                to_status,
                reason,
                last_error,
                raw: value.clone(),
            });
        }

        if let Some(todo_code) = op.strip_prefix("note:") {
            let code = normalize_todo_code(todo_code)?;
            let kind = map
                .get("kind")
                .and_then(|v| v.as_str())
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| "note".to_string());
            if kind.chars().count() > 32 {
                return Err(AgentToolError::InvalidArgs(
                    "note kind too long (max 32 chars)".to_string(),
                ));
            }
            let content = map
                .get("content")
                .and_then(|v| v.as_str())
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .ok_or_else(|| {
                    AgentToolError::InvalidArgs("note op missing `content`".to_string())
                })?;
            if content.chars().count() > MAX_TEXT_4096 {
                return Err(AgentToolError::InvalidArgs(format!(
                    "note content exceeds max {} chars",
                    MAX_TEXT_4096
                )));
            }
            return Ok(Self::Note {
                todo_code: code,
                kind,
                content,
                raw: value.clone(),
            });
        }

        Err(AgentToolError::InvalidArgs(format!(
            "unsupported delta op `{op}`; expected init/update:Txxx/note:Txxx"
        )))
    }

    fn raw(&self) -> &Json {
        match self {
            Self::Init { raw, .. } => raw,
            Self::Update { raw, .. } => raw,
            Self::Note { raw, .. } => raw,
        }
    }
}

#[derive(Clone, Debug)]
enum InitMode {
    Replace,
    Merge,
}

impl InitMode {
    fn parse(raw: &str) -> Result<Self, AgentToolError> {
        let value = normalize_enum(raw);
        match value.as_str() {
            "replace" => Ok(Self::Replace),
            "merge" => Ok(Self::Merge),
            _ => Err(AgentToolError::InvalidArgs(format!(
                "invalid init mode `{raw}`; allowed: replace|merge"
            ))),
        }
    }
}

#[derive(Clone, Debug)]
struct InitTodoItem {
    title: String,
    description: Option<String>,
    todo_type: TodoType,
    labels: Vec<String>,
    skills: Vec<String>,
    assignee: Option<String>,
    priority: Option<i64>,
    deps: Option<Vec<String>>,
    estimate: Option<Json>,
}

impl InitTodoItem {
    fn parse(value: &Json) -> Result<Self, AgentToolError> {
        let map = value.as_object().ok_or_else(|| {
            AgentToolError::InvalidArgs("init item must be a json object".to_string())
        })?;
        let title = map
            .get("title")
            .and_then(|v| v.as_str())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .ok_or_else(|| AgentToolError::InvalidArgs("init item missing `title`".to_string()))?;
        if title.chars().count() > MAX_TEXT_256 {
            return Err(AgentToolError::InvalidArgs(format!(
                "title exceeds max {} chars",
                MAX_TEXT_256
            )));
        }

        let description = map
            .get("description")
            .and_then(|v| v.as_str())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        if let Some(v) = description.as_ref() {
            if v.chars().count() > MAX_TEXT_4096 {
                return Err(AgentToolError::InvalidArgs(format!(
                    "description exceeds max {} chars",
                    MAX_TEXT_4096
                )));
            }
        }

        let todo_type = map
            .get("type")
            .and_then(|v| v.as_str())
            .map(TodoType::parse)
            .transpose()?
            .unwrap_or(TodoType::Task);

        let labels = parse_string_array(map.get("labels"), "labels", MAX_LABELS, 128)?;
        let skills = parse_string_array(map.get("skills"), "skills", MAX_SKILLS, 128)?;
        let assignee = map
            .get("assignee")
            .and_then(|v| v.as_str())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let priority = map.get("priority").and_then(|v| v.as_i64());

        let deps = match map.get("deps") {
            None => None,
            Some(v) => {
                let items = parse_string_array(Some(v), "deps", MAX_DEPS, 64)?;
                Some(items)
            }
        };

        let estimate = map.get("estimate").cloned();
        if let Some(ref v) = estimate {
            let size = serde_json::to_vec(v)
                .map_err(|err| {
                    AgentToolError::InvalidArgs(format!("serialize estimate failed: {err}"))
                })?
                .len();
            if size > 16 * 1024 {
                return Err(AgentToolError::InvalidArgs(
                    "estimate payload too large (max 16KB)".to_string(),
                ));
            }
        }

        Ok(Self {
            title,
            description,
            todo_type,
            labels,
            skills,
            assignee,
            priority,
            deps,
            estimate,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
struct TodoListItem {
    id: String,
    todo_code: String,
    workspace_id: String,
    session_id: Option<String>,
    title: String,
    description: Option<String>,
    #[serde(rename = "type")]
    todo_type: String,
    status: String,
    labels: Vec<String>,
    skills: Vec<String>,
    assignee: Option<String>,
    priority: Option<i64>,
    estimate: Option<Json>,
    attempts: i64,
    last_error: Option<Json>,
    created_at: u64,
    updated_at: u64,
    created_by: ActorRefOut,
    order_pos: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
struct TodoNoteItem {
    note_id: String,
    author: String,
    kind: String,
    content: String,
    created_at: u64,
    session_id: Option<String>,
    trace_id: Option<String>,
}

#[derive(Clone, Debug)]
struct TodoDetail {
    item: TodoListItem,
    notes: Vec<TodoNoteItem>,
    dep_codes: Vec<String>,
}

#[derive(Clone, Debug)]
struct TodoRowForUpdate {
    id: String,
    todo_code: String,
    status: TodoStatus,
    assignee: Option<String>,
    attempts: i64,
}

#[derive(Clone, Debug)]
struct OrderedTodoBrief {
    id: String,
    todo_code: String,
    todo_type: TodoType,
}

#[derive(Clone, Debug, Serialize)]
struct ApplyDeltaError {
    code: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    op: Option<Json>,
}

#[derive(Clone, Debug)]
struct ApplyDeltaResponse {
    ok: bool,
    workspace_id: String,
    op_id: String,
    before_version: i64,
    new_version: i64,
    idempotent: bool,
    errors: Vec<ApplyDeltaError>,
    applied_count: usize,
    cleared_count: usize,
    created_items: Vec<ApplyDeltaCreatedItem>,
    status_events: Vec<TodoStatusChangedEvent>,
}

#[derive(Clone, Debug, Serialize)]
struct ApplyDeltaCreatedItem {
    id: String,
    todo_code: String,
    title: String,
}

#[derive(Clone, Debug, Default)]
struct ApplySingleOpEffect {
    status_event: Option<TodoStatusChangedEvent>,
    cleared_count: usize,
    created_items: Vec<ApplyDeltaCreatedItem>,
}

#[derive(Clone, Debug, Default)]
struct InitApplyEffect {
    cleared_count: usize,
    created_items: Vec<ApplyDeltaCreatedItem>,
}

#[derive(Clone, Debug)]
struct TodoStatusChangedEvent {
    workspace_id: String,
    todo_id: String,
    todo_code: String,
    from_status: String,
    to_status: String,
    updated_at: u64,
    op_id: String,
    actor_kind: String,
    actor_did: String,
    session_id: Option<String>,
    trace_id: Option<String>,
}

#[derive(Clone, Debug)]
struct DomainError {
    code: &'static str,
    message: String,
    op: Option<Json>,
}

impl DomainError {
    fn not_found(message: impl Into<String>, op: Option<&DeltaOp>) -> Self {
        Self {
            code: "NOT_FOUND",
            message: message.into(),
            op: op.map(|v| v.raw().clone()),
        }
    }

    fn forbidden(message: impl Into<String>, op: Option<&DeltaOp>) -> Self {
        Self {
            code: "FORBIDDEN",
            message: message.into(),
            op: op.map(|v| v.raw().clone()),
        }
    }

    fn invalid_args(message: impl Into<String>, op: Option<&DeltaOp>) -> Self {
        Self {
            code: "INVALID_ARGS",
            message: message.into(),
            op: op.map(|v| v.raw().clone()),
        }
    }

    fn to_output(&self) -> ApplyDeltaError {
        ApplyDeltaError {
            code: self.code.to_string(),
            message: self.message.clone(),
            op: self.op.clone(),
        }
    }
}

fn ensure_todo_schema(conn: &Connection) -> Result<(), AgentToolError> {
    conn.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS todo_meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS todo_items (
  id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL,
  session_id TEXT,
  todo_code TEXT NOT NULL,
  title TEXT NOT NULL,
  description TEXT,
  type TEXT NOT NULL,
  status TEXT NOT NULL,
  priority INTEGER,
  labels_json TEXT,
  skills_json TEXT,
  assignee_did TEXT,
  estimate_json TEXT,
  attempts INTEGER NOT NULL DEFAULT 0,
  last_error_json TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  created_by_kind TEXT NOT NULL,
  created_by_did TEXT,
  UNIQUE(workspace_id, todo_code)
);

CREATE INDEX IF NOT EXISTS idx_todo_items_ws_status
  ON todo_items(workspace_id, status);

CREATE INDEX IF NOT EXISTS idx_todo_items_ws_priority
  ON todo_items(workspace_id, priority, updated_at);

CREATE INDEX IF NOT EXISTS idx_todo_items_ws_assignee
  ON todo_items(workspace_id, assignee_did);

CREATE INDEX IF NOT EXISTS idx_todo_items_ws_updated
  ON todo_items(workspace_id, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_todo_items_ws_session_assignee_status_created
  ON todo_items(workspace_id, session_id, assignee_did, status, created_at DESC);

CREATE TABLE IF NOT EXISTS todo_deps (
  workspace_id TEXT NOT NULL,
  todo_id TEXT NOT NULL,
  dep_todo_id TEXT NOT NULL,
  PRIMARY KEY (workspace_id, todo_id, dep_todo_id)
);

CREATE INDEX IF NOT EXISTS idx_todo_deps_ws_todo
  ON todo_deps(workspace_id, todo_id);

CREATE TABLE IF NOT EXISTS todo_notes (
  note_id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL,
  todo_id TEXT NOT NULL,
  author_did TEXT NOT NULL,
  kind TEXT NOT NULL DEFAULT 'note',
  content TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  session_id TEXT,
  trace_id TEXT
);

CREATE INDEX IF NOT EXISTS idx_todo_notes_ws_todo_time
  ON todo_notes(workspace_id, todo_id, created_at DESC);

CREATE TABLE IF NOT EXISTS todo_order (
  workspace_id TEXT NOT NULL,
  pos INTEGER NOT NULL,
  todo_id TEXT NOT NULL,
  PRIMARY KEY (workspace_id, pos),
  UNIQUE (workspace_id, todo_id)
);

CREATE TABLE IF NOT EXISTS todo_applied_ops (
  op_id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL,
  session_id TEXT,
  actor_did TEXT,
  applied_at INTEGER NOT NULL,
  ops_json TEXT NOT NULL
);
"#,
    )
    .map_err(|err| AgentToolError::ExecFailed(format!("ensure todo schema failed: {err}")))?;

    Ok(())
}

fn apply_todo_delta(
    conn: &mut Connection,
    oplog_path: &PathBuf,
    input: ApplyDeltaInput,
) -> Result<ApplyDeltaResponse, AgentToolError> {
    let before_version = read_workspace_version(conn, &input.workspace_id)?;

    if has_applied_op(conn, &input.op_id)? {
        let entry = build_oplog_entry(
            &input,
            before_version,
            before_version,
            "idempotent",
            Some(json!([])),
        );
        append_oplog(oplog_path, &entry)?;
        return Ok(ApplyDeltaResponse {
            ok: true,
            workspace_id: input.workspace_id,
            op_id: input.op_id,
            before_version,
            new_version: before_version,
            idempotent: true,
            errors: Vec::new(),
            applied_count: 0,
            cleared_count: 0,
            created_items: Vec::new(),
            status_events: Vec::new(),
        });
    }

    let tx = conn
        .transaction()
        .map_err(|err| AgentToolError::ExecFailed(format!("start todo tx failed: {err}")))?;

    let mut applied_count = 0usize;
    let mut cleared_count = 0usize;
    let mut created_items = Vec::<ApplyDeltaCreatedItem>::new();
    let mut status_events = Vec::new();
    for op in &input.ops {
        match apply_single_op(
            &tx,
            &input.workspace_id,
            &input.actor,
            input.op_id.as_str(),
            op,
        ) {
            Ok(effect) => {
                if let Some(event) = effect.status_event {
                    status_events.push(event);
                }
                cleared_count = cleared_count.saturating_add(effect.cleared_count);
                created_items.extend(effect.created_items);
                applied_count = applied_count.saturating_add(1);
            }
            Err(err) => {
                tx.rollback().map_err(|rollback_err| {
                    AgentToolError::ExecFailed(format!(
                        "rollback todo tx failed after domain error: {rollback_err}"
                    ))
                })?;
                let err_output = err.to_output();
                let entry = build_oplog_entry(
                    &input,
                    before_version,
                    before_version,
                    "rejected",
                    Some(json!([err_output])),
                );
                append_oplog(oplog_path, &entry)?;
                return Ok(ApplyDeltaResponse {
                    ok: false,
                    workspace_id: input.workspace_id,
                    op_id: input.op_id,
                    before_version,
                    new_version: before_version,
                    idempotent: false,
                    errors: vec![err_output],
                    applied_count,
                    cleared_count: 0,
                    created_items: Vec::new(),
                    status_events: Vec::new(),
                });
            }
        }
    }

    let new_version = before_version.saturating_add(1);
    write_workspace_version(&tx, &input.workspace_id, new_version)?;

    let ops_json = serde_json::to_string(
        &input
            .ops
            .iter()
            .map(|op| op.raw().clone())
            .collect::<Vec<Json>>(),
    )
    .map_err(|err| AgentToolError::ExecFailed(format!("serialize applied ops failed: {err}")))?;

    tx.execute(
        "INSERT INTO todo_applied_ops(op_id, workspace_id, session_id, actor_did, applied_at, ops_json)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            input.op_id,
            input.workspace_id,
            input.actor.session_id,
            input.actor.did,
            u64_to_i64(now_ms()),
            ops_json,
        ],
    )
    .map_err(|err| AgentToolError::ExecFailed(format!("insert todo_applied_ops failed: {err}")))?;

    tx.commit()
        .map_err(|err| AgentToolError::ExecFailed(format!("commit todo tx failed: {err}")))?;

    let entry = build_oplog_entry(
        &input,
        before_version,
        new_version,
        "applied",
        Some(json!([])),
    );
    append_oplog(oplog_path, &entry)?;

    Ok(ApplyDeltaResponse {
        ok: true,
        workspace_id: input.workspace_id,
        op_id: input.op_id,
        before_version,
        new_version,
        idempotent: false,
        errors: Vec::new(),
        applied_count,
        cleared_count,
        created_items,
        status_events,
    })
}

fn apply_single_op(
    tx: &rusqlite::Transaction<'_>,
    workspace_id: &str,
    actor: &ActorCtx,
    op_id: &str,
    op: &DeltaOp,
) -> Result<ApplySingleOpEffect, DomainError> {
    match op {
        DeltaOp::Init { mode, items, .. } => {
            let init_effect = apply_init_op(tx, workspace_id, actor, mode, items, op)?;
            Ok(ApplySingleOpEffect {
                status_event: None,
                cleared_count: init_effect.cleared_count,
                created_items: init_effect.created_items,
            })
        }
        DeltaOp::Update {
            todo_code,
            to_status,
            reason,
            last_error,
            ..
        } => apply_update_op(
            tx,
            workspace_id,
            actor,
            op_id,
            todo_code,
            to_status,
            reason,
            last_error,
            op,
        )
        .map(|status_event| ApplySingleOpEffect {
            status_event,
            ..Default::default()
        }),
        DeltaOp::Note {
            todo_code,
            kind,
            content,
            ..
        } => {
            apply_note_op(tx, workspace_id, actor, todo_code, kind, content, op)?;
            Ok(ApplySingleOpEffect::default())
        }
    }
}

fn apply_init_op(
    tx: &rusqlite::Transaction<'_>,
    workspace_id: &str,
    actor: &ActorCtx,
    mode: &InitMode,
    items: &[InitTodoItem],
    op: &DeltaOp,
) -> Result<InitApplyEffect, DomainError> {
    if actor.kind == ActorKind::SubAgent {
        return Err(DomainError::forbidden(
            "sub_agent cannot run init operation",
            Some(op),
        ));
    }

    let mut cleared_count = 0usize;
    if matches!(mode, InitMode::Replace) {
        let existing_count: i64 = tx
            .query_row(
                "SELECT COUNT(1) FROM todo_items WHERE workspace_id = ?1",
                params![workspace_id],
                |row| row.get(0),
            )
            .map_err(|err| {
                DomainError::invalid_args(
                    format!("count items for replace mode failed: {err}"),
                    Some(op),
                )
            })?;
        cleared_count = existing_count.max(0) as usize;

        tx.execute(
            "DELETE FROM todo_deps WHERE workspace_id = ?1",
            params![workspace_id],
        )
        .map_err(|err| {
            DomainError::invalid_args(
                format!("clear deps for replace mode failed: {err}"),
                Some(op),
            )
        })?;
        tx.execute(
            "DELETE FROM todo_notes WHERE workspace_id = ?1",
            params![workspace_id],
        )
        .map_err(|err| {
            DomainError::invalid_args(
                format!("clear notes for replace mode failed: {err}"),
                Some(op),
            )
        })?;
        tx.execute(
            "DELETE FROM todo_order WHERE workspace_id = ?1",
            params![workspace_id],
        )
        .map_err(|err| {
            DomainError::invalid_args(
                format!("clear order for replace mode failed: {err}"),
                Some(op),
            )
        })?;
        tx.execute(
            "DELETE FROM todo_items WHERE workspace_id = ?1",
            params![workspace_id],
        )
        .map_err(|err| {
            DomainError::invalid_args(
                format!("clear items for replace mode failed: {err}"),
                Some(op),
            )
        })?;
    }

    let existing = load_ordered_todos(tx, workspace_id).map_err(|err| {
        DomainError::invalid_args(
            format!("load existing todo order failed: {}", err.message),
            Some(op),
        )
    })?;

    let mut code_to_id: HashMap<String, String> = existing
        .iter()
        .map(|item| (item.todo_code.clone(), item.id.clone()))
        .collect();

    let mut non_bench_before: Vec<String> = existing
        .iter()
        .filter(|item| item.todo_type == TodoType::Task)
        .map(|item| item.id.clone())
        .collect();

    let mut next_code = if matches!(mode, InitMode::Replace) {
        1
    } else {
        next_todo_code_seq(tx, workspace_id).map_err(|err| {
            DomainError::invalid_args(
                format!("read next todo code failed: {}", err.message),
                Some(op),
            )
        })?
    };

    let mut next_pos = if matches!(mode, InitMode::Replace) {
        0
    } else {
        next_order_pos(tx, workspace_id).map_err(|err| {
            DomainError::invalid_args(
                format!("read next order position failed: {}", err.message),
                Some(op),
            )
        })?
    };

    let mut previous_todo_id = existing.last().map(|v| v.id.clone());
    let mut created_items = Vec::<ApplyDeltaCreatedItem>::new();

    for item in items {
        let todo_id = generate_id("todo");
        let todo_code = format!("T{:03}", next_code);
        next_code += 1;

        let assignee = item.assignee.clone().unwrap_or_else(|| actor.did.clone());

        let labels_json = to_json_string(&item.labels).map_err(|err| {
            DomainError::invalid_args(
                format!("serialize labels failed: {}", err.message),
                Some(op),
            )
        })?;
        let skills_json = to_json_string(&item.skills).map_err(|err| {
            DomainError::invalid_args(
                format!("serialize skills failed: {}", err.message),
                Some(op),
            )
        })?;
        let estimate_json = if let Some(ref estimate) = item.estimate {
            Some(to_json_string(estimate).map_err(|err| {
                DomainError::invalid_args(
                    format!("serialize estimate failed: {}", err.message),
                    Some(op),
                )
            })?)
        } else {
            None
        };

        let now = now_ms();
        tx.execute(
            "INSERT INTO todo_items (
                id, workspace_id, session_id, todo_code, title, description, type, status,
                priority, labels_json, skills_json, assignee_did, estimate_json, attempts,
                last_error_json, created_at, updated_at, created_by_kind, created_by_did
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                ?9, ?10, ?11, ?12, ?13, 0,
                NULL, ?14, ?15, ?16, ?17
            )",
            params![
                todo_id,
                workspace_id,
                actor.session_id,
                todo_code,
                item.title,
                item.description,
                item.todo_type.as_str(),
                TodoStatus::Wait.as_str(),
                item.priority,
                labels_json,
                skills_json,
                assignee,
                estimate_json,
                u64_to_i64(now),
                u64_to_i64(now),
                actor.kind.as_str(),
                actor.did,
            ],
        )
        .map_err(|err| {
            DomainError::invalid_args(format!("insert init todo failed: {err}"), Some(op))
        })?;

        tx.execute(
            "INSERT INTO todo_order(workspace_id, pos, todo_id) VALUES(?1, ?2, ?3)",
            params![workspace_id, next_pos, todo_id],
        )
        .map_err(|err| {
            DomainError::invalid_args(format!("insert todo order failed: {err}"), Some(op))
        })?;
        next_pos += 1;

        let dep_ids =
            resolve_init_deps(item, &code_to_id, &previous_todo_id, &non_bench_before, op)?;
        for dep_id in dep_ids {
            tx.execute(
                "INSERT OR IGNORE INTO todo_deps(workspace_id, todo_id, dep_todo_id) VALUES(?1, ?2, ?3)",
                params![workspace_id, todo_id, dep_id],
            )
            .map_err(|err| {
                DomainError::invalid_args(format!("insert todo deps failed: {err}"), Some(op))
            })?;
        }

        created_items.push(ApplyDeltaCreatedItem {
            id: todo_id.clone(),
            todo_code: todo_code.clone(),
            title: item.title.clone(),
        });
        code_to_id.insert(todo_code, todo_id.clone());
        if item.todo_type == TodoType::Task {
            non_bench_before.push(todo_id.clone());
        }
        previous_todo_id = Some(todo_id);
    }

    Ok(InitApplyEffect {
        cleared_count,
        created_items,
    })
}

fn resolve_init_deps(
    item: &InitTodoItem,
    code_to_id: &HashMap<String, String>,
    previous_todo_id: &Option<String>,
    non_bench_before: &[String],
    op: &DeltaOp,
) -> Result<Vec<String>, DomainError> {
    match item.deps.as_ref() {
        Some(deps) if deps.is_empty() => Ok(Vec::new()),
        Some(deps) => {
            let mut out = Vec::new();
            for dep in deps {
                if dep == "@prev" {
                    let prev = previous_todo_id.as_ref().ok_or_else(|| {
                        DomainError::invalid_args(
                            "`@prev` used but no previous todo exists",
                            Some(op),
                        )
                    })?;
                    push_unique(&mut out, prev.clone());
                    continue;
                }

                let dep_code = normalize_todo_code(dep).map_err(|_| {
                    DomainError::invalid_args(
                        format!("invalid dep reference `{dep}`, expected Txxx or @prev"),
                        Some(op),
                    )
                })?;
                let dep_id = code_to_id.get(&dep_code).ok_or_else(|| {
                    DomainError::not_found(format!("dep todo `{dep_code}` not found"), Some(op))
                })?;
                push_unique(&mut out, dep_id.clone());
            }
            Ok(out)
        }
        None => {
            if item.todo_type == TodoType::Bench {
                Ok(non_bench_before.to_vec())
            } else {
                Ok(Vec::new())
            }
        }
    }
}

fn apply_update_op(
    tx: &rusqlite::Transaction<'_>,
    workspace_id: &str,
    actor: &ActorCtx,
    op_id: &str,
    todo_code: &str,
    to_status: &TodoStatus,
    _reason: &str,
    last_error: &Option<Json>,
    op: &DeltaOp,
) -> Result<Option<TodoStatusChangedEvent>, DomainError> {
    let mut todo = load_todo_for_update(tx, workspace_id, todo_code).map_err(|err| {
        DomainError::not_found(
            format!("todo `{todo_code}` not found: {}", err.message),
            Some(op),
        )
    })?;

    assert_subagent_permission(actor, &todo, op)?;

    let now = now_ms();
    let mut attempts = todo.attempts;
    if todo.status != TodoStatus::Failed && *to_status == TodoStatus::Failed {
        attempts = attempts.saturating_add(1);
    }

    let last_error_json = if let Some(v) = last_error {
        Some(to_json_string(v).map_err(|err| {
            DomainError::invalid_args(
                format!("serialize last_error failed: {}", err.message),
                Some(op),
            )
        })?)
    } else {
        None
    };

    tx.execute(
        "UPDATE todo_items
         SET status = ?3,
             attempts = ?4,
             last_error_json = COALESCE(?5, last_error_json),
             updated_at = ?6
         WHERE workspace_id = ?1 AND id = ?2",
        params![
            workspace_id,
            todo.id,
            to_status.as_str(),
            attempts,
            last_error_json,
            u64_to_i64(now),
        ],
    )
    .map_err(|err| {
        DomainError::invalid_args(format!("update todo status failed: {err}"), Some(op))
    })?;

    let from_status = todo.status.as_str().to_string();
    let to_status_text = to_status.as_str().to_string();
    todo.status = to_status.clone();
    if from_status == to_status_text {
        return Ok(None);
    }

    Ok(Some(TodoStatusChangedEvent {
        workspace_id: workspace_id.to_string(),
        todo_id: todo.id.clone(),
        todo_code: todo.todo_code.clone(),
        from_status,
        to_status: to_status_text,
        updated_at: now,
        op_id: op_id.to_string(),
        actor_kind: actor.kind.as_str().to_string(),
        actor_did: actor.did.clone(),
        session_id: actor.session_id.clone(),
        trace_id: actor.trace_id.clone(),
    }))
}

fn apply_note_op(
    tx: &rusqlite::Transaction<'_>,
    workspace_id: &str,
    actor: &ActorCtx,
    todo_code: &str,
    kind: &str,
    content: &str,
    op: &DeltaOp,
) -> Result<(), DomainError> {
    let todo = load_todo_for_update(tx, workspace_id, todo_code).map_err(|err| {
        DomainError::not_found(
            format!("todo `{todo_code}` not found: {}", err.message),
            Some(op),
        )
    })?;

    assert_subagent_permission(actor, &todo, op)?;

    let note_id = generate_id("note");
    let now = now_ms();
    tx.execute(
        "INSERT INTO todo_notes(
            note_id, workspace_id, todo_id, author_did, kind, content, created_at, session_id, trace_id
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            note_id,
            workspace_id,
            todo.id,
            actor.did,
            kind,
            content,
            u64_to_i64(now),
            actor.session_id,
            actor.trace_id,
        ],
    )
    .map_err(|err| DomainError::invalid_args(format!("insert todo note failed: {err}"), Some(op)))?;

    tx.execute(
        "UPDATE todo_items SET updated_at = ?3 WHERE workspace_id = ?1 AND id = ?2",
        params![workspace_id, todo.id, u64_to_i64(now)],
    )
    .map_err(|err| {
        DomainError::invalid_args(format!("update todo updated_at failed: {err}"), Some(op))
    })?;

    Ok(())
}

fn assert_subagent_permission(
    actor: &ActorCtx,
    todo: &TodoRowForUpdate,
    op: &DeltaOp,
) -> Result<(), DomainError> {
    if actor.kind != ActorKind::SubAgent {
        return Ok(());
    }

    let Some(assignee) = todo.assignee.as_ref() else {
        return Err(DomainError::forbidden(
            format!(
                "sub_agent `{}` cannot update unassigned todo `{}`",
                actor.did, todo.todo_code
            ),
            Some(op),
        ));
    };

    if assignee != &actor.did {
        return Err(DomainError::forbidden(
            format!(
                "sub_agent `{}` cannot update todo `{}` assigned to `{}`",
                actor.did, todo.todo_code, assignee
            ),
            Some(op),
        ));
    }

    Ok(())
}

fn load_ordered_todos(
    tx: &rusqlite::Transaction<'_>,
    workspace_id: &str,
) -> Result<Vec<OrderedTodoBrief>, DomainError> {
    let mut stmt = tx
        .prepare(
            "SELECT i.id, i.todo_code, i.type, o.pos
             FROM todo_order o
             JOIN todo_items i ON i.workspace_id = o.workspace_id AND i.id = o.todo_id
             WHERE o.workspace_id = ?1
             ORDER BY o.pos ASC",
        )
        .map_err(|err| {
            DomainError::invalid_args(format!("prepare load order failed: {err}"), None)
        })?;

    let rows = stmt
        .query_map(params![workspace_id], |row| {
            let todo_type_raw: String = row.get(2)?;
            Ok(OrderedTodoBrief {
                id: row.get(0)?,
                todo_code: row.get(1)?,
                todo_type: TodoType::from_db(&todo_type_raw).map_err(to_sql_err)?,
            })
        })
        .map_err(|err| {
            DomainError::invalid_args(format!("query load order failed: {err}"), None)
        })?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|err| {
            DomainError::invalid_args(format!("decode ordered todo failed: {err}"), None)
        })?);
    }
    Ok(out)
}

fn next_todo_code_seq(
    tx: &rusqlite::Transaction<'_>,
    workspace_id: &str,
) -> Result<i64, DomainError> {
    let mut stmt = tx
        .prepare("SELECT todo_code FROM todo_items WHERE workspace_id = ?1")
        .map_err(|err| {
            DomainError::invalid_args(format!("prepare next code failed: {err}"), None)
        })?;
    let rows = stmt
        .query_map(params![workspace_id], |row| row.get::<_, String>(0))
        .map_err(|err| DomainError::invalid_args(format!("query next code failed: {err}"), None))?;

    let mut max_seq = 0i64;
    for row in rows {
        let code = row.map_err(|err| {
            DomainError::invalid_args(format!("decode todo_code failed: {err}"), None)
        })?;
        if let Some(seq) = parse_todo_seq(&code) {
            max_seq = max_seq.max(seq);
        }
    }
    Ok(max_seq.saturating_add(1))
}

fn next_order_pos(tx: &rusqlite::Transaction<'_>, workspace_id: &str) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(pos), -1) + 1 FROM todo_order WHERE workspace_id = ?1",
        params![workspace_id],
        |row| row.get::<_, i64>(0),
    )
    .map_err(|err| DomainError::invalid_args(format!("query next order pos failed: {err}"), None))
}

fn read_next_default_priority(
    conn: &Connection,
    workspace_id: &str,
) -> Result<i64, AgentToolError> {
    let max_priority = conn
        .query_row(
            "SELECT MAX(priority) FROM todo_items WHERE workspace_id = ?1",
            params![workspace_id],
            |row| row.get::<_, Option<i64>>(0),
        )
        .map_err(|err| {
            AgentToolError::ExecFailed(format!("query next default priority failed: {err}"))
        })?
        .unwrap_or(0);

    let next = if max_priority <= 0 {
        DEFAULT_PRIORITY_GAP
    } else {
        max_priority.saturating_add(DEFAULT_PRIORITY_GAP)
    };
    Ok(next)
}

fn workspace_has_todos(conn: &Connection, workspace_id: &str) -> Result<bool, AgentToolError> {
    let count = conn
        .query_row(
            "SELECT COUNT(1) FROM todo_items WHERE workspace_id = ?1",
            params![workspace_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|err| AgentToolError::ExecFailed(format!("query todo count failed: {err}")))?;
    Ok(count > 0)
}

fn load_todo_for_update(
    tx: &rusqlite::Transaction<'_>,
    workspace_id: &str,
    todo_code: &str,
) -> Result<TodoRowForUpdate, DomainError> {
    tx.query_row(
        "SELECT id, todo_code, status, assignee_did, attempts
         FROM todo_items
         WHERE workspace_id = ?1 AND todo_code = ?2
         LIMIT 1",
        params![workspace_id, todo_code],
        |row| {
            let raw_status: String = row.get(2)?;
            Ok(TodoRowForUpdate {
                id: row.get(0)?,
                todo_code: row.get(1)?,
                status: TodoStatus::from_db(&raw_status).map_err(to_sql_err)?,
                assignee: row.get(3)?,
                attempts: row.get(4)?,
            })
        },
    )
    .map_err(|err| DomainError::not_found(format!("query todo `{todo_code}` failed: {err}"), None))
}

fn list_todo_items(
    conn: &Connection,
    workspace_id: &str,
    filters: &TodoListFilters,
    limit: usize,
    offset: usize,
) -> Result<Vec<TodoListItem>, AgentToolError> {
    let mut sql = String::from(
        "SELECT
            i.id,
            i.todo_code,
            i.workspace_id,
            i.session_id,
            i.title,
            i.description,
            i.type,
            i.status,
            i.labels_json,
            i.skills_json,
            i.assignee_did,
            i.priority,
            i.estimate_json,
            i.attempts,
            i.last_error_json,
            i.created_at,
            i.updated_at,
            i.created_by_kind,
            i.created_by_did,
            o.pos
         FROM todo_items i
         LEFT JOIN todo_order o
           ON o.workspace_id = i.workspace_id AND o.todo_id = i.id
         WHERE i.workspace_id = ?",
    );

    let mut params_vec = vec![SqlValue::Text(workspace_id.to_string())];

    if !filters.statuses.is_empty() {
        sql.push_str(" AND i.status IN (");
        for idx in 0..filters.statuses.len() {
            if idx > 0 {
                sql.push(',');
            }
            sql.push('?');
            params_vec.push(SqlValue::Text(filters.statuses[idx].as_str().to_string()));
        }
        sql.push(')');
    }

    if let Some(todo_type) = filters.todo_type.as_ref() {
        sql.push_str(" AND i.type = ?");
        params_vec.push(SqlValue::Text(todo_type.as_str().to_string()));
    }

    if let Some(assignee) = filters.assignee.as_ref() {
        sql.push_str(" AND i.assignee_did = ?");
        params_vec.push(SqlValue::Text(assignee.to_string()));
    }

    if let Some(label) = filters.label.as_ref() {
        sql.push_str(" AND i.labels_json LIKE ?");
        params_vec.push(SqlValue::Text(format!("%\"{}\"%", escape_like(label))));
    }

    if let Some(query) = filters.query.as_ref() {
        let like = format!("%{}%", escape_like(query));
        sql.push_str(" AND (i.title LIKE ? ESCAPE '\\\\' OR i.description LIKE ? ESCAPE '\\\\')");
        params_vec.push(SqlValue::Text(like.clone()));
        params_vec.push(SqlValue::Text(like));
    }

    let sort_by = filters.sort_by.as_deref().unwrap_or("updated_at");
    match sort_by {
        "priority" => {
            sql.push_str(" ORDER BY i.priority IS NULL ASC, i.priority");
            if filters.asc {
                sql.push_str(" ASC");
            } else {
                sql.push_str(" ASC");
            }
            sql.push_str(", i.updated_at DESC, o.pos ASC");
        }
        "order" => {
            sql.push_str(" ORDER BY o.pos ");
            if filters.asc {
                sql.push_str("ASC");
            } else {
                sql.push_str("DESC");
            }
            sql.push_str(", i.updated_at DESC");
        }
        _ => {
            sql.push_str(" ORDER BY i.updated_at ");
            if filters.asc {
                sql.push_str("ASC");
            } else {
                sql.push_str("DESC");
            }
            sql.push_str(", o.pos ASC");
        }
    }

    sql.push_str(" LIMIT ? OFFSET ?");
    params_vec.push(SqlValue::Integer(usize_to_i64(limit, "limit")?));
    params_vec.push(SqlValue::Integer(usize_to_i64(offset, "offset")?));

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|err| AgentToolError::ExecFailed(format!("prepare todo list failed: {err}")))?;
    let rows = stmt
        .query_map(params_from_iter(params_vec), map_todo_list_row)
        .map_err(|err| AgentToolError::ExecFailed(format!("query todo list failed: {err}")))?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|err| {
            AgentToolError::ExecFailed(format!("decode todo list row failed: {err}"))
        })?);
    }
    Ok(out)
}

fn get_todo_detail(
    conn: &Connection,
    workspace_id: &str,
    todo_ref: &str,
    max_notes: usize,
) -> Result<Option<TodoDetail>, AgentToolError> {
    let id_or_code = resolve_todo_id(conn, workspace_id, todo_ref)?;
    let Some(todo_id) = id_or_code else {
        return Ok(None);
    };

    let mut stmt = conn
        .prepare(
            "SELECT
                i.id,
                i.todo_code,
                i.workspace_id,
                i.session_id,
                i.title,
                i.description,
                i.type,
                i.status,
                i.labels_json,
                i.skills_json,
                i.assignee_did,
                i.priority,
                i.estimate_json,
                i.attempts,
                i.last_error_json,
                i.created_at,
                i.updated_at,
                i.created_by_kind,
                i.created_by_did,
                o.pos
             FROM todo_items i
             LEFT JOIN todo_order o
               ON o.workspace_id = i.workspace_id AND o.todo_id = i.id
             WHERE i.workspace_id = ?1 AND i.id = ?2
             LIMIT 1",
        )
        .map_err(|err| AgentToolError::ExecFailed(format!("prepare todo get failed: {err}")))?;

    let item = stmt
        .query_row(params![workspace_id, todo_id], map_todo_list_row)
        .map_err(|err| AgentToolError::ExecFailed(format!("query todo get failed: {err}")))?;

    let notes = list_todo_notes(conn, workspace_id, &todo_id, max_notes)?;
    let dep_codes = list_todo_dep_codes(conn, workspace_id, &todo_id)?;

    Ok(Some(TodoDetail {
        item,
        notes,
        dep_codes,
    }))
}

fn query_pending_counts(
    conn: &Connection,
    workspace_id: &str,
) -> Result<BTreeMap<String, u64>, AgentToolError> {
    let mut stmt = conn
        .prepare(
            "SELECT status, COUNT(1)
             FROM todo_items
             WHERE workspace_id = ?1
             GROUP BY status",
        )
        .map_err(|err| {
            AgentToolError::ExecFailed(format!("prepare pending query failed: {err}"))
        })?;

    let rows = stmt
        .query_map(params![workspace_id], |row| {
            let status: String = row.get(0)?;
            let count: i64 = row.get(1)?;
            Ok((status, count.max(0) as u64))
        })
        .map_err(|err| AgentToolError::ExecFailed(format!("query pending counts failed: {err}")))?;

    let mut out = BTreeMap::new();
    for row in rows {
        let (status, count) = row.map_err(|err| {
            AgentToolError::ExecFailed(format!("decode pending row failed: {err}"))
        })?;
        out.insert(status, count);
    }

    for status in [
        TodoStatus::Wait,
        TodoStatus::InProgress,
        TodoStatus::Complete,
        TodoStatus::Failed,
        TodoStatus::Done,
        TodoStatus::CheckFailed,
    ] {
        out.entry(status.as_str().to_string()).or_insert(0);
    }

    Ok(out)
}

fn list_for_prompt(
    conn: &Connection,
    workspace_id: &str,
    limit: usize,
) -> Result<Vec<TodoListItem>, AgentToolError> {
    let mut stmt = conn
        .prepare(
            "SELECT
                i.id,
                i.todo_code,
                i.workspace_id,
                i.session_id,
                i.title,
                i.description,
                i.type,
                i.status,
                i.labels_json,
                i.skills_json,
                i.assignee_did,
                i.priority,
                i.estimate_json,
                i.attempts,
                i.last_error_json,
                i.created_at,
                i.updated_at,
                i.created_by_kind,
                i.created_by_did,
                o.pos
             FROM todo_items i
             LEFT JOIN todo_order o ON o.workspace_id = i.workspace_id AND o.todo_id = i.id
             WHERE i.workspace_id = ?1
             ORDER BY
                CASE i.status
                    WHEN 'IN_PROGRESS' THEN 0
                    WHEN 'WAIT' THEN 1
                    WHEN 'COMPLETE' THEN 2
                    WHEN 'CHECK_FAILED' THEN 3
                    WHEN 'FAILED' THEN 4
                    WHEN 'DONE' THEN 5
                    ELSE 6
                END,
                i.priority IS NULL ASC,
                i.priority ASC,
                o.pos ASC,
                i.updated_at DESC
             LIMIT ?2",
        )
        .map_err(|err| AgentToolError::ExecFailed(format!("prepare prompt list failed: {err}")))?;

    let rows = stmt
        .query_map(
            params![workspace_id, usize_to_i64(limit, "limit")?],
            map_todo_list_row,
        )
        .map_err(|err| AgentToolError::ExecFailed(format!("query prompt list failed: {err}")))?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|err| {
            AgentToolError::ExecFailed(format!("decode prompt row failed: {err}"))
        })?);
    }
    Ok(out)
}

fn select_current_todo_details(
    conn: &Connection,
    workspace_id: &str,
    session_id: Option<&str>,
    todo_ref: Option<&str>,
) -> Result<Option<TodoDetail>, AgentToolError> {
    if let Some(todo_ref) = todo_ref {
        return get_todo_detail(conn, workspace_id, todo_ref, 12);
    }

    let selected_todo_id = if let Some(session_id) = session_id {
        let mut stmt = conn
            .prepare(
                "SELECT id
                 FROM todo_items
                 WHERE workspace_id = ?1 AND session_id = ?2
                 ORDER BY
                    CASE status
                        WHEN 'IN_PROGRESS' THEN 0
                        WHEN 'WAIT' THEN 1
                        WHEN 'COMPLETE' THEN 2
                        WHEN 'CHECK_FAILED' THEN 3
                        ELSE 9
                    END,
                    priority IS NULL ASC,
                    priority ASC,
                    updated_at DESC
                 LIMIT 1",
            )
            .map_err(|err| {
                AgentToolError::ExecFailed(format!("prepare select by session failed: {err}"))
            })?;

        stmt.query_row(params![workspace_id, session_id], |row| {
            row.get::<_, String>(0)
        })
        .ok()
    } else {
        None
    };

    if let Some(todo_id) = selected_todo_id {
        return get_todo_detail(conn, workspace_id, &todo_id, 12);
    }

    let mut stmt = conn
        .prepare(
            "SELECT id
             FROM todo_items
             WHERE workspace_id = ?1
             ORDER BY
                CASE status
                    WHEN 'IN_PROGRESS' THEN 0
                    WHEN 'WAIT' THEN 1
                    WHEN 'COMPLETE' THEN 2
                    WHEN 'CHECK_FAILED' THEN 3
                    WHEN 'FAILED' THEN 4
                    ELSE 9
                END,
                priority IS NULL ASC,
                priority ASC,
                updated_at DESC
             LIMIT 1",
        )
        .map_err(|err| {
            AgentToolError::ExecFailed(format!("prepare select fallback todo failed: {err}"))
        })?;

    let fallback_todo_id = stmt
        .query_row(params![workspace_id], |row| row.get::<_, String>(0))
        .ok();

    if let Some(todo_id) = fallback_todo_id {
        get_todo_detail(conn, workspace_id, &todo_id, 12)
    } else {
        Ok(None)
    }
}

fn get_next_ready_todo(
    conn: &Connection,
    workspace_id: &str,
    session_id: &str,
    agent_id: &str,
) -> Result<Option<TodoDetail>, AgentToolError> {
    let mut stmt = conn
        .prepare(
            "SELECT i.id
             FROM todo_items i
             LEFT JOIN todo_order o
               ON o.workspace_id = i.workspace_id AND o.todo_id = i.id
             WHERE i.workspace_id = ?1
               AND i.session_id = ?2
               AND i.assignee_did = ?3
               AND i.status = 'WAIT'
               AND NOT EXISTS (
                    SELECT 1
                    FROM todo_deps d
                    JOIN todo_items dep
                      ON dep.workspace_id = d.workspace_id AND dep.id = d.dep_todo_id
                    WHERE d.workspace_id = i.workspace_id
                      AND d.todo_id = i.id
                      AND dep.status NOT IN ('COMPLETE', 'DONE')
               )
             ORDER BY i.created_at DESC, o.pos DESC
             LIMIT 1",
        )
        .map_err(|err| {
            AgentToolError::ExecFailed(format!("prepare next ready todo query failed: {err}"))
        })?;

    let todo_id = stmt
        .query_row(params![workspace_id, session_id, agent_id], |row| {
            row.get::<_, String>(0)
        })
        .ok();

    if let Some(todo_id) = todo_id {
        get_todo_detail(conn, workspace_id, &todo_id, 12)
    } else {
        Ok(None)
    }
}

pub fn get_next_ready_todo_code(
    conn: &Connection,
    workspace_id: &str,
    session_id: &str,
    agent_id: &str,
) -> Result<Option<String>, AgentToolError> {
    Ok(
        get_next_ready_todo(conn, workspace_id, session_id, agent_id)?
            .map(|detail| detail.item.todo_code),
    )
}

pub fn get_next_ready_todo_text(
    conn: &Connection,
    workspace_id: &str,
    session_id: &str,
    agent_id: &str,
) -> Result<Option<String>, AgentToolError> {
    Ok(
        get_next_ready_todo(conn, workspace_id, session_id, agent_id)?
            .as_ref()
            .map(render_current_todo_text),
    )
}

pub fn get_session_todo_text_by_ref(
    conn: &Connection,
    workspace_id: &str,
    session_id: &str,
    todo_ref: &str,
) -> Result<Option<String>, AgentToolError> {
    let detail = get_todo_detail(conn, workspace_id, todo_ref, 12)?;
    let Some(detail) = detail else {
        return Ok(None);
    };

    if detail.item.session_id.as_deref() != Some(session_id) {
        return Ok(None);
    }

    Ok(Some(render_current_todo_text(&detail)))
}

fn resolve_todo_id(
    conn: &Connection,
    workspace_id: &str,
    todo_ref: &str,
) -> Result<Option<String>, AgentToolError> {
    let trimmed = todo_ref.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    if looks_like_todo_code(trimmed) {
        let mut stmt = conn
            .prepare("SELECT id FROM todo_items WHERE workspace_id = ?1 AND todo_code = ?2 LIMIT 1")
            .map_err(|err| {
                AgentToolError::ExecFailed(format!("prepare resolve by code failed: {err}"))
            })?;
        let id = stmt
            .query_row(params![workspace_id, trimmed], |row| {
                row.get::<_, String>(0)
            })
            .ok();
        return Ok(id);
    }

    let mut stmt = conn
        .prepare("SELECT id FROM todo_items WHERE workspace_id = ?1 AND id = ?2 LIMIT 1")
        .map_err(|err| {
            AgentToolError::ExecFailed(format!("prepare resolve by id failed: {err}"))
        })?;
    let id = stmt
        .query_row(params![workspace_id, trimmed], |row| {
            row.get::<_, String>(0)
        })
        .ok();
    Ok(id)
}

fn list_todo_notes(
    conn: &Connection,
    workspace_id: &str,
    todo_id: &str,
    limit: usize,
) -> Result<Vec<TodoNoteItem>, AgentToolError> {
    let mut stmt = conn
        .prepare(
            "SELECT note_id, author_did, kind, content, created_at, session_id, trace_id
             FROM todo_notes
             WHERE workspace_id = ?1 AND todo_id = ?2
             ORDER BY created_at DESC
             LIMIT ?3",
        )
        .map_err(|err| AgentToolError::ExecFailed(format!("prepare note list failed: {err}")))?;

    let rows = stmt
        .query_map(
            params![workspace_id, todo_id, usize_to_i64(limit, "limit")?],
            |row| {
                let created_at: i64 = row.get(4)?;
                Ok(TodoNoteItem {
                    note_id: row.get(0)?,
                    author: row.get(1)?,
                    kind: row.get(2)?,
                    content: row.get(3)?,
                    created_at: i64_to_u64(created_at).unwrap_or(0),
                    session_id: row.get(5)?,
                    trace_id: row.get(6)?,
                })
            },
        )
        .map_err(|err| AgentToolError::ExecFailed(format!("query note list failed: {err}")))?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|err| {
            AgentToolError::ExecFailed(format!("decode note list row failed: {err}"))
        })?);
    }
    Ok(out)
}

fn list_todo_dep_codes(
    conn: &Connection,
    workspace_id: &str,
    todo_id: &str,
) -> Result<Vec<String>, AgentToolError> {
    let mut stmt = conn
        .prepare(
            "SELECT i.todo_code
             FROM todo_deps d
             JOIN todo_items i
               ON i.workspace_id = d.workspace_id AND i.id = d.dep_todo_id
             WHERE d.workspace_id = ?1 AND d.todo_id = ?2
             ORDER BY i.todo_code ASC",
        )
        .map_err(|err| AgentToolError::ExecFailed(format!("prepare dep list failed: {err}")))?;

    let rows = stmt
        .query_map(params![workspace_id, todo_id], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|err| AgentToolError::ExecFailed(format!("query dep list failed: {err}")))?;

    let mut out = Vec::new();
    for row in rows {
        out.push(
            row.map_err(|err| AgentToolError::ExecFailed(format!("decode dep row failed: {err}")))?,
        );
    }
    Ok(out)
}

fn map_todo_list_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TodoListItem> {
    let labels_raw: Option<String> = row.get(8)?;
    let skills_raw: Option<String> = row.get(9)?;
    let estimate_raw: Option<String> = row.get(12)?;
    let last_error_raw: Option<String> = row.get(14)?;
    let created_by_kind: String = row.get(17)?;
    let created_by_did: Option<String> = row.get(18)?;
    let created_at: i64 = row.get(15)?;
    let updated_at: i64 = row.get(16)?;

    Ok(TodoListItem {
        id: row.get(0)?,
        todo_code: row.get(1)?,
        workspace_id: row.get(2)?,
        session_id: row.get(3)?,
        title: row.get(4)?,
        description: row.get(5)?,
        todo_type: row.get(6)?,
        status: row.get(7)?,
        labels: parse_json_vec(labels_raw.as_deref()),
        skills: parse_json_vec(skills_raw.as_deref()),
        assignee: row.get(10)?,
        priority: row.get(11)?,
        estimate: parse_json_obj(estimate_raw.as_deref()),
        attempts: row.get(13)?,
        last_error: parse_json_obj(last_error_raw.as_deref()),
        created_at: i64_to_u64(created_at).unwrap_or(0),
        updated_at: i64_to_u64(updated_at).unwrap_or(0),
        created_by: ActorRefOut {
            kind: created_by_kind,
            did: created_by_did.unwrap_or_default(),
        },
        order_pos: row.get(19)?,
    })
}

fn build_oplog_entry(
    input: &ApplyDeltaInput,
    before_version: i64,
    after_version: i64,
    result: &str,
    errors: Option<Json>,
) -> Json {
    json!({
        "ts": now_ms(),
        "op_id": input.op_id,
        "workspace_id": input.workspace_id,
        "session_id": input.actor.session_id,
        "actor": input.actor.out(),
        "ops": input.ops.iter().map(|op| op.raw().clone()).collect::<Vec<Json>>(),
        "before_version": before_version,
        "after_version": after_version,
        "result": result,
        "errors": errors
    })
}

fn append_oplog(oplog_path: &PathBuf, entry: &Json) -> Result<(), AgentToolError> {
    if let Some(parent) = oplog_path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "create todo oplog dir `{}` failed: {err}",
                parent.display()
            ))
        })?;
    }

    let line = serde_json::to_string(entry).map_err(|err| {
        AgentToolError::ExecFailed(format!("serialize oplog entry failed: {err}"))
    })?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(oplog_path)
        .map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "open oplog `{}` failed: {err}",
                oplog_path.display()
            ))
        })?;

    file.write_all(line.as_bytes()).map_err(|err| {
        AgentToolError::ExecFailed(format!(
            "write oplog `{}` failed: {err}",
            oplog_path.display()
        ))
    })?;
    file.write_all(b"\n").map_err(|err| {
        AgentToolError::ExecFailed(format!(
            "write oplog newline `{}` failed: {err}",
            oplog_path.display()
        ))
    })?;
    Ok(())
}

fn has_applied_op(conn: &Connection, op_id: &str) -> Result<bool, AgentToolError> {
    let count = conn
        .query_row(
            "SELECT COUNT(1) FROM todo_applied_ops WHERE op_id = ?1",
            params![op_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|err| AgentToolError::ExecFailed(format!("query applied op failed: {err}")))?;
    Ok(count > 0)
}

fn read_workspace_version(conn: &Connection, workspace_id: &str) -> Result<i64, AgentToolError> {
    let key = version_key(workspace_id);
    let value = conn
        .query_row(
            "SELECT value FROM todo_meta WHERE key = ?1",
            params![key],
            |row| row.get::<_, String>(0),
        )
        .ok();

    match value {
        Some(raw) => Ok(raw.parse::<i64>().unwrap_or(0).max(0)),
        None => Ok(0),
    }
}

fn write_workspace_version(
    tx: &rusqlite::Transaction<'_>,
    workspace_id: &str,
    version: i64,
) -> Result<(), AgentToolError> {
    let key = version_key(workspace_id);
    tx.execute(
        "INSERT INTO todo_meta(key, value) VALUES(?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, version.to_string()],
    )
    .map_err(|err| AgentToolError::ExecFailed(format!("write workspace version failed: {err}")))?;
    Ok(())
}

fn version_key(workspace_id: &str) -> String {
    format!("version:{workspace_id}")
}

fn render_workspace_todo_text(
    workspace_id: &str,
    version: i64,
    items: &[TodoListItem],
    token_budget: usize,
) -> String {
    if items.is_empty() {
        return format!("Workspace Todo ({workspace_id}, v{version})\n- No todo items available.");
    }

    let mut out = String::new();
    out.push_str(&format!("Workspace Todo ({workspace_id}, v{version})\n"));

    let char_budget = token_budget.saturating_mul(4).max(256);
    for item in items {
        let line = format!(
            "- {} [{}] assignee={} p={} {}\n",
            item.todo_code,
            item.status,
            item.assignee.clone().unwrap_or_else(|| "-".to_string()),
            item.priority
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".to_string()),
            item.title
        );
        if out.len().saturating_add(line.len()) > char_budget {
            out.push_str("- ...truncated by token budget\n");
            break;
        }
        out.push_str(&line);
    }

    out
}

fn render_current_todo_text(detail: &TodoDetail) -> String {
    let item = &detail.item;
    let mut lines = Vec::new();
    lines.push(format!("Current Todo {} [{}]", item.todo_code, item.status));
    lines.push(format!("Title: {}", item.title));
    if let Some(desc) = item.description.as_deref() {
        if !desc.trim().is_empty() {
            lines.push(format!("Description: {}", desc));
        }
    }
    lines.push(format!(
        "Type: {} | Assignee: {} | Priority: {}",
        item.todo_type,
        item.assignee.clone().unwrap_or_else(|| "-".to_string()),
        item.priority
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".to_string())
    ));

    if detail.dep_codes.is_empty() {
        lines.push("Deps: (none)".to_string());
    } else {
        lines.push(format!("Deps: {}", detail.dep_codes.join(", ")));
    }

    if let Some(last_error) = item.last_error.as_ref() {
        lines.push(format!("LastError: {}", compact_json(last_error, 300)));
    }

    if detail.notes.is_empty() {
        lines.push("Recent Notes: (none)".to_string());
    } else {
        lines.push("Recent Notes:".to_string());
        for note in detail.notes.iter().take(5) {
            lines.push(format!(
                "- [{}] {}: {}",
                note.kind,
                note.author,
                truncate_chars(&note.content, 120)
            ));
        }
    }

    lines.join("\n")
}

fn parse_status_set(value: Option<&Json>) -> Result<HashSet<TodoStatus>, AgentToolError> {
    let statuses = parse_status_list(value)?;
    Ok(statuses.into_iter().collect())
}

fn parse_status_list(value: Option<&Json>) -> Result<Vec<TodoStatus>, AgentToolError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };

    if let Some(single) = value.as_str() {
        return Ok(vec![TodoStatus::parse(single)?]);
    }

    let arr = value.as_array().ok_or_else(|| {
        AgentToolError::InvalidArgs("status filter must be string or array".to_string())
    })?;

    let mut out = Vec::new();
    for item in arr {
        let raw = item.as_str().ok_or_else(|| {
            AgentToolError::InvalidArgs("status filter array must contain strings".to_string())
        })?;
        let status = TodoStatus::parse(raw)?;
        if !out.contains(&status) {
            out.push(status);
        }
    }
    Ok(out)
}

fn parse_string_array(
    value: Option<&Json>,
    field_name: &str,
    max_items: usize,
    max_each: usize,
) -> Result<Vec<String>, AgentToolError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let arr = value.as_array().ok_or_else(|| {
        AgentToolError::InvalidArgs(format!("`{field_name}` must be an array of strings"))
    })?;
    if arr.len() > max_items {
        return Err(AgentToolError::InvalidArgs(format!(
            "`{field_name}` exceeds max {max_items} items"
        )));
    }

    let mut out = Vec::new();
    for item in arr {
        let text = item.as_str().ok_or_else(|| {
            AgentToolError::InvalidArgs(format!("`{field_name}` must be an array of strings"))
        })?;
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.chars().count() > max_each {
            return Err(AgentToolError::InvalidArgs(format!(
                "`{field_name}` contains entry that exceeds max {max_each} chars"
            )));
        }
        if !out.iter().any(|v| v == trimmed) {
            out.push(trimmed.to_string());
        }
    }

    Ok(out)
}

fn require_workspace_id(args: &Json) -> Result<String, AgentToolError> {
    let workspace_id = require_trimmed_string_arg(args, "workspace_id")?;
    if workspace_id.chars().count() > MAX_TEXT_256 {
        return Err(AgentToolError::InvalidArgs(
            "`workspace_id` too long (max 256 chars)".to_string(),
        ));
    }
    Ok(workspace_id)
}

fn optional_bool(args: &Json, key: &str) -> Result<Option<bool>, AgentToolError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    value
        .as_bool()
        .map(Some)
        .ok_or_else(|| AgentToolError::InvalidArgs(format!("`{key}` must be a boolean")))
}

fn normalize_todo_code(raw: &str) -> Result<String, AgentToolError> {
    let normalized = raw.trim().to_uppercase();
    if looks_like_todo_code(&normalized) {
        return Ok(normalized);
    }
    Err(AgentToolError::InvalidArgs(format!(
        "invalid todo code `{raw}`, expected format T001"
    )))
}

fn looks_like_todo_code(raw: &str) -> bool {
    raw.len() >= 2 && raw.starts_with('T') && raw[1..].chars().all(|c| c.is_ascii_digit())
}

fn parse_todo_seq(todo_code: &str) -> Option<i64> {
    if !looks_like_todo_code(todo_code) {
        return None;
    }
    todo_code[1..].parse::<i64>().ok()
}

fn sanitize_kevent_token(raw: &str) -> String {
    let mut output = String::with_capacity(raw.len());
    let mut prev_dash = false;
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            output.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            output.push('-');
            prev_dash = true;
        }
    }
    let trimmed = output.trim_matches('-');
    if trimmed.is_empty() {
        "default".to_string()
    } else {
        trimmed.chars().take(80).collect()
    }
}

fn build_todo_status_eventid(workspace_id: &str, todo_code: &str) -> String {
    format!(
        "/agent/{}/{}/status_changed",
        sanitize_kevent_token(workspace_id),
        sanitize_kevent_token(todo_code)
    )
}

fn normalize_enum(raw: &str) -> String {
    raw.trim()
        .to_lowercase()
        .replace([' ', '-'], "_")
        .to_string()
}

fn push_unique<T: PartialEq>(target: &mut Vec<T>, value: T) {
    if !target.contains(&value) {
        target.push(value);
    }
}

fn escape_like(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn parse_json_vec(raw: Option<&str>) -> Vec<String> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_default()
}

fn parse_json_obj(raw: Option<&str>) -> Option<Json> {
    let Some(raw) = raw else {
        return None;
    };
    serde_json::from_str::<Json>(raw).ok()
}

fn to_json_string<T: Serialize>(value: &T) -> Result<String, DomainError> {
    serde_json::to_string(value)
        .map_err(|err| DomainError::invalid_args(format!("serialize json failed: {err}"), None))
}

fn to_sql_err(err: AgentToolError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            err.to_string(),
        )),
    )
}

fn compact_json(value: &Json, max_len: usize) -> String {
    let raw = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string());
    truncate_chars(&raw, max_len)
}

fn truncate_chars(raw: &str, max_len: usize) -> String {
    let count = raw.chars().count();
    if count <= max_len {
        return raw.to_string();
    }
    raw.chars().take(max_len).collect::<String>() + "..."
}

fn generate_id(prefix: &str) -> String {
    let seq = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{}-{seq}", now_ms())
}

fn u64_to_i64(v: u64) -> i64 {
    if v > i64::MAX as u64 {
        i64::MAX
    } else {
        v as i64
    }
}

fn i64_to_u64(v: i64) -> Option<u64> {
    if v < 0 {
        None
    } else {
        Some(v as u64)
    }
}

fn usize_to_i64(v: usize, field: &str) -> Result<i64, AgentToolError> {
    i64::try_from(v).map_err(|_| AgentToolError::InvalidArgs(format!("`{field}` too large")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_todo_status_eventid_sanitizes_segments() {
        assert_eq!(
            build_todo_status_eventid("ws:Alpha/1", "T001"),
            "/agent/ws-alpha-1/t001/status_changed"
        );
    }

    #[test]
    fn render_workspace_todo_text_formats_and_truncates_by_budget() {
        let long_title = "A".repeat(120);
        let make_item = |todo_code: &str, title: &str| TodoListItem {
            id: format!("id-{todo_code}"),
            todo_code: todo_code.to_string(),
            workspace_id: "ws-demo".to_string(),
            session_id: Some("sess-demo".to_string()),
            title: title.to_string(),
            description: None,
            todo_type: "Task".to_string(),
            status: "WAIT".to_string(),
            labels: vec![],
            skills: vec![],
            assignee: Some("did:od:alice".to_string()),
            priority: Some(1),
            estimate: None,
            attempts: 0,
            last_error: None,
            created_at: 1,
            updated_at: 1,
            created_by: ActorRefOut {
                kind: "root_agent".to_string(),
                did: "did:od:jarvis".to_string(),
            },
            order_pos: Some(1),
        };

        let items = vec![
            make_item("T001", &long_title),
            make_item("T002", "second task should be truncated"),
        ];
        let rendered = render_workspace_todo_text("ws-demo", 7, &items, 1);
        assert!(rendered.starts_with("Workspace Todo (ws-demo, v7)\n"));
        assert!(rendered.contains("- T001 [WAIT] assignee=did:od:alice p=1 "));
        assert!(rendered.contains("- ...truncated by token budget"));
        assert!(!rendered.contains("T002"));
    }
    #[test]
    fn get_next_ready_todo_direct_function_call() {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        ensure_todo_schema(&conn).expect("ensure schema");

        let workspace_id = "ws-ready-func";
        let session_id = "sess-ready";
        let assignee = "did:od:alice";

        conn.execute(
            "INSERT INTO todo_items(
                id, workspace_id, session_id, todo_code, title, type, status,
                assignee_did, created_at, updated_at, created_by_kind, created_by_did
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                "todo-1",
                workspace_id,
                session_id,
                "T001",
                "base task",
                "Task",
                "WAIT",
                assignee,
                1000_i64,
                1000_i64,
                "root_agent",
                "did:od:jarvis"
            ],
        )
        .expect("insert T001");

        conn.execute(
            "INSERT INTO todo_items(
                id, workspace_id, session_id, todo_code, title, type, status,
                assignee_did, created_at, updated_at, created_by_kind, created_by_did
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                "todo-2",
                workspace_id,
                session_id,
                "T002",
                "dep task",
                "Task",
                "WAIT",
                assignee,
                2000_i64,
                2000_i64,
                "root_agent",
                "did:od:jarvis"
            ],
        )
        .expect("insert T002");

        conn.execute(
            "INSERT INTO todo_items(
                id, workspace_id, session_id, todo_code, title, type, status,
                assignee_did, created_at, updated_at, created_by_kind, created_by_did
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                "todo-3",
                workspace_id,
                session_id,
                "T003",
                "newest task",
                "Task",
                "WAIT",
                assignee,
                3000_i64,
                3000_i64,
                "root_agent",
                "did:od:jarvis"
            ],
        )
        .expect("insert T003");

        conn.execute(
            "INSERT INTO todo_items(
                id, workspace_id, session_id, todo_code, title, type, status,
                assignee_did, created_at, updated_at, created_by_kind, created_by_did
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                "todo-4",
                workspace_id,
                "sess-other",
                "T004",
                "other session task",
                "Task",
                "WAIT",
                assignee,
                4000_i64,
                4000_i64,
                "root_agent",
                "did:od:jarvis"
            ],
        )
        .expect("insert T004");

        conn.execute(
            "INSERT INTO todo_deps(workspace_id, todo_id, dep_todo_id) VALUES(?1, ?2, ?3)",
            rusqlite::params![workspace_id, "todo-2", "todo-1"],
        )
        .expect("insert dep T002->T001");

        let first = get_next_ready_todo(&conn, workspace_id, session_id, assignee)
            .expect("query first")
            .expect("first todo");
        assert_eq!(first.item.todo_code, "T003");
        assert_eq!(
            get_next_ready_todo_code(&conn, workspace_id, session_id, assignee)
                .expect("query first code")
                .as_deref(),
            Some("T003")
        );
        assert!(
            get_next_ready_todo_text(&conn, workspace_id, session_id, assignee)
                .expect("query first text")
                .unwrap_or_default()
                .contains("Current Todo T003 [WAIT]")
        );

        conn.execute(
            "UPDATE todo_items SET status = 'IN_PROGRESS' WHERE workspace_id = ?1 AND todo_code = ?2",
            rusqlite::params![workspace_id, "T003"],
        )
        .expect("update T003");

        let second = get_next_ready_todo(&conn, workspace_id, session_id, assignee)
            .expect("query second")
            .expect("second todo");
        assert_eq!(second.item.todo_code, "T001");

        conn.execute(
            "UPDATE todo_items SET status = 'COMPLETE' WHERE workspace_id = ?1 AND todo_code = ?2",
            rusqlite::params![workspace_id, "T001"],
        )
        .expect("update T001");

        let third = get_next_ready_todo(&conn, workspace_id, session_id, assignee)
            .expect("query third")
            .expect("third todo");
        assert_eq!(third.item.todo_code, "T002");
        assert_eq!(third.dep_codes, vec!["T001".to_string()]);
    }
}
