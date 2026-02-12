use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use buckyos_api::{BoxKind, MsgCenterClient};
use name_lib::DID;
use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::{json, Value as Json};
use tokio::fs;
use tokio::task;

use crate::agent_tool::{AgentTool, ToolCallContext, ToolError, ToolManager, ToolSpec};

pub const TOOL_LIST: &str = "list";
pub const TOOL_LOAD_MEMORY: &str = "load_memory";
pub const TOOL_LOAD_THINGS: &str = "load_things";
pub const TOOL_LOAD_CHAT_HISTORY: &str = "load_chat_history";
pub const TOOL_LOAC_CHAT_HISTORY_ALIAS: &str = "loac_chat_history";

const DEFAULT_MEMORY_DIR_REL_PATH: &str = "memory";
const DEFAULT_MEMORY_MD_REL_PATH: &str = "memory/memory.md";
const DEFAULT_THINGS_DB_REL_PATH: &str = "memory/things.db";
const DEFAULT_MEMORY_TOKEN_LIMIT: u32 = 2_000;
const DEFAULT_CHAT_TOKEN_LIMIT: u32 = 2_000;
const DEFAULT_CHAT_LIMIT: usize = 32;
const DEFAULT_LIST_MAX_ENTRIES: usize = 128;
const DEFAULT_TABLE_LIMIT: usize = 32;

#[derive(Clone, Debug)]
pub struct AgentMemoryConfig {
    pub agent_root: PathBuf,
    pub memory_dir_rel_path: PathBuf,
    pub memory_md_rel_path: PathBuf,
    pub things_db_rel_path: PathBuf,
    pub default_memory_token_limit: u32,
    pub default_chat_token_limit: u32,
    pub default_chat_limit: usize,
    pub max_list_entries: usize,
    pub default_table_limit: usize,
}

impl AgentMemoryConfig {
    pub fn new(agent_root: impl Into<PathBuf>) -> Self {
        Self {
            agent_root: agent_root.into(),
            memory_dir_rel_path: PathBuf::from(DEFAULT_MEMORY_DIR_REL_PATH),
            memory_md_rel_path: PathBuf::from(DEFAULT_MEMORY_MD_REL_PATH),
            things_db_rel_path: PathBuf::from(DEFAULT_THINGS_DB_REL_PATH),
            default_memory_token_limit: DEFAULT_MEMORY_TOKEN_LIMIT,
            default_chat_token_limit: DEFAULT_CHAT_TOKEN_LIMIT,
            default_chat_limit: DEFAULT_CHAT_LIMIT,
            max_list_entries: DEFAULT_LIST_MAX_ENTRIES,
            default_table_limit: DEFAULT_TABLE_LIMIT,
        }
    }
}

#[derive(Clone)]
pub struct AgentMemory {
    cfg: AgentMemoryConfig,
    memory_dir: PathBuf,
    memory_md_path: PathBuf,
    things_db_path: PathBuf,
    msg_center: Option<Arc<MsgCenterClient>>,
}

impl AgentMemory {
    pub async fn new(
        mut cfg: AgentMemoryConfig,
        msg_center: Option<Arc<MsgCenterClient>>,
    ) -> Result<Self, ToolError> {
        if cfg.default_memory_token_limit == 0 {
            cfg.default_memory_token_limit = DEFAULT_MEMORY_TOKEN_LIMIT;
        }
        if cfg.default_chat_token_limit == 0 {
            cfg.default_chat_token_limit = DEFAULT_CHAT_TOKEN_LIMIT;
        }
        if cfg.default_chat_limit == 0 {
            cfg.default_chat_limit = DEFAULT_CHAT_LIMIT;
        }
        if cfg.max_list_entries == 0 {
            cfg.max_list_entries = DEFAULT_LIST_MAX_ENTRIES;
        }
        if cfg.default_table_limit == 0 {
            cfg.default_table_limit = DEFAULT_TABLE_LIMIT;
        }

        let agent_root = normalize_root(&cfg.agent_root).await?;
        cfg.agent_root = agent_root.clone();

        let memory_dir = resolve_relative_path(&agent_root, &cfg.memory_dir_rel_path)?;
        let memory_md_path = resolve_relative_path(&agent_root, &cfg.memory_md_rel_path)?;
        let things_db_path = resolve_relative_path(&agent_root, &cfg.things_db_rel_path)?;

        fs::create_dir_all(&memory_dir)
            .await
            .map_err(|err| ToolError::ExecFailed(format!("create memory dir failed: {err}")))?;
        ensure_parent_dir(&memory_md_path).await?;
        ensure_parent_dir(&things_db_path).await?;

        if fs::try_exists(&memory_md_path).await.unwrap_or(false) == false {
            fs::write(&memory_md_path, "")
                .await
                .map_err(|err| ToolError::ExecFailed(format!("create memory.md failed: {err}")))?;
        }

        init_things_db(&things_db_path).await?;

        Ok(Self {
            cfg,
            memory_dir,
            memory_md_path,
            things_db_path,
            msg_center,
        })
    }

    pub fn register_tools(&self, tool_mgr: &ToolManager) -> Result<(), ToolError> {
        tool_mgr.register_tool(ListMemoryTool {
            memory: self.clone(),
        })?;
        tool_mgr.register_tool(LoadMemoryTool {
            memory: self.clone(),
        })?;
        tool_mgr.register_tool(LoadThingsTool {
            memory: self.clone(),
        })?;
        tool_mgr.register_tool(LoadChatHistoryTool {
            memory: self.clone(),
            tool_name: TOOL_LOAD_CHAT_HISTORY.to_string(),
        })?;
        tool_mgr.register_tool(LoadChatHistoryTool {
            memory: self.clone(),
            tool_name: TOOL_LOAC_CHAT_HISTORY_ALIAS.to_string(),
        })?;
        Ok(())
    }

    pub fn memory_dir(&self) -> &Path {
        &self.memory_dir
    }

    pub fn memory_md_path(&self) -> &Path {
        &self.memory_md_path
    }

    pub fn things_db_path(&self) -> &Path {
        &self.things_db_path
    }

    async fn list_entries(
        &self,
        recursive: bool,
        max_entries: usize,
    ) -> Result<Vec<Json>, ToolError> {
        let max_entries = max_entries.min(self.cfg.max_list_entries).max(1);
        let mut entries = Vec::new();
        let mut pending_dirs = vec![self.memory_dir.clone()];

        while let Some(dir) = pending_dirs.pop() {
            let mut read_dir = fs::read_dir(&dir).await.map_err(|err| {
                ToolError::ExecFailed(format!("read memory dir `{}` failed: {err}", dir.display()))
            })?;

            while let Some(entry) = read_dir.next_entry().await.map_err(|err| {
                ToolError::ExecFailed(format!("list memory dir entry failed: {err}"))
            })? {
                let path = entry.path();
                let metadata = entry.metadata().await.map_err(|err| {
                    ToolError::ExecFailed(format!(
                        "read metadata failed for `{}`: {err}",
                        path.display()
                    ))
                })?;
                let kind = if metadata.is_dir() { "dir" } else { "file" };
                let rel = path
                    .strip_prefix(&self.memory_dir)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");

                entries.push(json!({
                    "path": rel,
                    "kind": kind,
                    "bytes": if metadata.is_file() { Some(metadata.len()) } else { None },
                }));

                if metadata.is_dir() && recursive {
                    pending_dirs.push(path);
                }

                if entries.len() >= max_entries {
                    entries.sort_by_key(|item| {
                        item.get("path")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string()
                    });
                    return Ok(entries);
                }
            }
        }

        entries.sort_by_key(|item| {
            item.get("path")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        });
        Ok(entries)
    }

    async fn load_memory_md(&self, token_limit: u32) -> Result<Json, ToolError> {
        let token_limit = token_limit.max(1);
        let content = fs::read_to_string(&self.memory_md_path)
            .await
            .unwrap_or_else(|_| "".to_string());
        let full_tokens = approx_tokens(&content);
        let (loaded, truncated) = truncate_by_token_limit(&content, token_limit);

        Ok(json!({
            "path": self.memory_md_path.to_string_lossy().to_string(),
            "token_limit": token_limit,
            "approx_full_tokens": full_tokens,
            "approx_loaded_tokens": approx_tokens(&loaded),
            "truncated": truncated,
            "content": loaded,
        }))
    }

    async fn load_things(&self, name: Option<String>, limit: usize) -> Result<Json, ToolError> {
        let limit = limit.max(1);
        let db_path = self.things_db_path.clone();
        let query_name = name.clone();

        let snapshot = task::spawn_blocking(move || -> Result<ThingsSnapshot, ToolError> {
            let conn = Connection::open(&db_path).map_err(|err| {
                ToolError::ExecFailed(format!(
                    "open things db `{}` failed: {err}",
                    db_path.display()
                ))
            })?;
            ensure_things_db_schema(&conn)?;

            let kv = query_kv(&conn, query_name.as_deref(), limit)?;
            let facts = query_facts(&conn, query_name.as_deref(), limit)?;
            let events = query_events(&conn, query_name.as_deref(), limit)?;

            Ok(ThingsSnapshot { kv, facts, events })
        })
        .await
        .map_err(|err| ToolError::ExecFailed(format!("query things db join error: {err}")))??;

        Ok(json!({
            "path": self.things_db_path.to_string_lossy().to_string(),
            "query": name,
            "limit_per_table": limit,
            "kv": snapshot.kv,
            "facts": snapshot.facts,
            "events": snapshot.events,
        }))
    }

    async fn load_chat_history(
        &self,
        owner_did: String,
        box_kind: BoxKind,
        thread_key: Option<String>,
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
                Some(limit),
                cursor_sort_key,
                cursor_record_id,
                Some(descending),
            )
            .await
            .map_err(|err| ToolError::ExecFailed(format!("load chat history failed: {err}")))?;

        let mut messages = Vec::new();
        for item in page.items {
            let msg_thread_key = item
                .msg
                .thread_key
                .clone()
                .or_else(|| item.record.thread_key.clone());
            if let Some(expect_thread) = thread_key.as_ref() {
                if msg_thread_key.as_deref() != Some(expect_thread.as_str()) {
                    continue;
                }
            }

            messages.push(json!({
                "record_id": item.record.record_id,
                "thread_key": msg_thread_key,
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
            "limit": limit,
            "token_limit": token_limit.max(1),
            "used_tokens": used_tokens,
            "truncated_by_token_limit": truncated_by_budget,
            "thread_key": thread_key,
            "items": messages,
            "next_cursor_sort_key": page.next_cursor_sort_key,
            "next_cursor_record_id": page.next_cursor_record_id,
        }))
    }
}

#[derive(Clone)]
struct ListMemoryTool {
    memory: AgentMemory,
}

#[async_trait]
impl AgentTool for ListMemoryTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_LIST.to_string(),
            description: "List memory directory entries.".to_string(),
            args_schema: json!({
                "type":"object",
                "properties": {
                    "recursive": {"type":"boolean"},
                    "max_entries": {"type":"integer", "minimum": 1}
                },
                "additionalProperties": true
            }),
            output_schema: json!({
                "type":"object",
                "properties": {
                    "memory_dir": {"type":"string"},
                    "entries": {"type":"array"}
                }
            }),
        }
    }

    async fn call(&self, _ctx: &ToolCallContext, args: Json) -> Result<Json, ToolError> {
        let recursive = optional_bool(&args, "recursive")?.unwrap_or(true);
        let max_entries =
            optional_usize(&args, "max_entries")?.unwrap_or(self.memory.cfg.max_list_entries);
        let entries = self.memory.list_entries(recursive, max_entries).await?;

        Ok(json!({
            "memory_dir": self.memory.memory_dir.to_string_lossy().to_string(),
            "entries": entries,
            "chat_history_provider": if self.memory.msg_center.is_some() { "msg_center" } else { "none" },
        }))
    }
}

#[derive(Clone)]
struct LoadMemoryTool {
    memory: AgentMemory,
}

#[async_trait]
impl AgentTool for LoadMemoryTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_LOAD_MEMORY.to_string(),
            description:
                "Load memory.md and keep the most important head section under token limit."
                    .to_string(),
            args_schema: json!({
                "type":"object",
                "properties": {
                    "token_limit": {"type":"integer", "minimum": 1}
                },
                "additionalProperties": true
            }),
            output_schema: json!({
                "type":"object",
                "properties": {
                    "path": {"type":"string"},
                    "token_limit": {"type":"integer"},
                    "approx_full_tokens": {"type":"integer"},
                    "approx_loaded_tokens": {"type":"integer"},
                    "truncated": {"type":"boolean"},
                    "content": {"type":"string"}
                }
            }),
        }
    }

    async fn call(&self, _ctx: &ToolCallContext, args: Json) -> Result<Json, ToolError> {
        let token_limit = optional_u32(&args, "token_limit")?
            .unwrap_or(self.memory.cfg.default_memory_token_limit);
        self.memory.load_memory_md(token_limit).await
    }
}

#[derive(Clone)]
struct LoadThingsTool {
    memory: AgentMemory,
}

#[async_trait]
impl AgentTool for LoadThingsTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_LOAD_THINGS.to_string(),
            description: "Load things from things.db with keyword filter.".to_string(),
            args_schema: json!({
                "type":"object",
                "properties": {
                    "name": {"type":"string"},
                    "limit": {"type":"integer", "minimum": 1}
                },
                "additionalProperties": true
            }),
            output_schema: json!({
                "type":"object",
                "properties": {
                    "path": {"type":"string"},
                    "query": {"type":["string", "null"]},
                    "limit_per_table": {"type":"integer"},
                    "kv": {"type":"array"},
                    "facts": {"type":"array"},
                    "events": {"type":"array"}
                }
            }),
        }
    }

    async fn call(&self, _ctx: &ToolCallContext, args: Json) -> Result<Json, ToolError> {
        let name = optional_string(&args, "name")?;
        let limit = optional_usize(&args, "limit")?.unwrap_or(self.memory.cfg.default_table_limit);
        self.memory.load_things(name, limit).await
    }
}

#[derive(Clone)]
struct LoadChatHistoryTool {
    memory: AgentMemory,
    tool_name: String,
}

#[async_trait]
impl AgentTool for LoadChatHistoryTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.tool_name.clone(),
            description: "Load chat history via MsgCenter by owner/thread.".to_string(),
            args_schema: json!({
                "type":"object",
                "properties": {
                    "owner_did": {"type":"string"},
                    "box_kind": {"type":"string"},
                    "thread_key": {"type":"string"},
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
                    "limit": {"type":"integer"},
                    "token_limit": {"type":"integer"},
                    "used_tokens": {"type":"integer"},
                    "truncated_by_token_limit": {"type":"boolean"},
                    "thread_key": {"type":["string", "null"]},
                    "items": {"type":"array"},
                    "next_cursor_sort_key": {"type":["integer", "null"]},
                    "next_cursor_record_id": {"type":["string", "null"]}
                }
            }),
        }
    }

    async fn call(&self, _ctx: &ToolCallContext, args: Json) -> Result<Json, ToolError> {
        let owner_did = require_string(&args, "owner_did")?;
        let box_kind = parse_box_kind(optional_string(&args, "box_kind")?)?;
        let thread_key = optional_string(&args, "thread_key")?;
        let limit = optional_usize(&args, "limit")?.unwrap_or(self.memory.cfg.default_chat_limit);
        let token_limit =
            optional_u32(&args, "token_limit")?.unwrap_or(self.memory.cfg.default_chat_token_limit);
        let cursor_sort_key = optional_u64(&args, "cursor_sort_key")?;
        let cursor_record_id = optional_string(&args, "cursor_record_id")?;
        let descending = optional_bool(&args, "descending")?.unwrap_or(true);

        self.memory
            .load_chat_history(
                owner_did,
                box_kind,
                thread_key,
                limit,
                token_limit,
                cursor_sort_key,
                cursor_record_id,
                descending,
            )
            .await
    }
}

#[derive(Debug, Serialize)]
struct KvEntry {
    key: String,
    value: String,
    updated_at: i64,
    source: Option<String>,
    confidence: Option<f64>,
}

#[derive(Debug, Serialize)]
struct FactEntry {
    id: String,
    subject: String,
    predicate: String,
    object: String,
    updated_at: i64,
    source: Option<String>,
}

#[derive(Debug, Serialize)]
struct EventEntry {
    id: String,
    event_type: String,
    payload: String,
    ts: i64,
}

#[derive(Debug, Serialize)]
struct ThingsSnapshot {
    kv: Vec<KvEntry>,
    facts: Vec<FactEntry>,
    events: Vec<EventEntry>,
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

async fn init_things_db(db_path: &Path) -> Result<(), ToolError> {
    let db_path = db_path.to_path_buf();
    task::spawn_blocking(move || {
        let conn = Connection::open(&db_path).map_err(|err| {
            ToolError::ExecFailed(format!(
                "open things db `{}` failed: {err}",
                db_path.display()
            ))
        })?;
        ensure_things_db_schema(&conn)
    })
    .await
    .map_err(|err| ToolError::ExecFailed(format!("init things db join error: {err}")))?
}

fn ensure_things_db_schema(conn: &Connection) -> Result<(), ToolError> {
    conn.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS kv (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at INTEGER NOT NULL DEFAULT 0,
    source TEXT,
    confidence REAL
);
CREATE TABLE IF NOT EXISTS facts (
    id TEXT PRIMARY KEY,
    subject TEXT NOT NULL,
    predicate TEXT NOT NULL,
    object TEXT NOT NULL,
    updated_at INTEGER NOT NULL DEFAULT 0,
    source TEXT
);
CREATE TABLE IF NOT EXISTS events (
    id TEXT PRIMARY KEY,
    type TEXT NOT NULL,
    payload TEXT NOT NULL,
    ts INTEGER NOT NULL DEFAULT 0
);
"#,
    )
    .map_err(|err| ToolError::ExecFailed(format!("ensure things schema failed: {err}")))
}

fn query_kv(
    conn: &Connection,
    keyword: Option<&str>,
    limit: usize,
) -> Result<Vec<KvEntry>, ToolError> {
    let limit = i64::try_from(limit)
        .map_err(|_| ToolError::InvalidArgs("limit is too large".to_string()))?;
    let mut out = Vec::new();

    if let Some(keyword) = keyword {
        let pattern = format!("%{}%", keyword.trim());
        let mut stmt = conn
            .prepare(
                "SELECT key, value, updated_at, source, confidence
                 FROM kv
                 WHERE key LIKE ?1 OR value LIKE ?1
                 ORDER BY updated_at DESC
                 LIMIT ?2",
            )
            .map_err(|err| ToolError::ExecFailed(format!("prepare kv query failed: {err}")))?;
        let rows = stmt
            .query_map(params![pattern, limit], |row| {
                Ok(KvEntry {
                    key: row.get(0)?,
                    value: row.get(1)?,
                    updated_at: row.get(2)?,
                    source: row.get(3).ok(),
                    confidence: row.get(4).ok(),
                })
            })
            .map_err(|err| ToolError::ExecFailed(format!("query kv failed: {err}")))?;
        for row in rows {
            out.push(
                row.map_err(|err| ToolError::ExecFailed(format!("read kv row failed: {err}")))?,
            );
        }
    } else {
        let mut stmt = conn
            .prepare(
                "SELECT key, value, updated_at, source, confidence
                 FROM kv
                 ORDER BY updated_at DESC
                 LIMIT ?1",
            )
            .map_err(|err| ToolError::ExecFailed(format!("prepare kv query failed: {err}")))?;
        let rows = stmt
            .query_map(params![limit], |row| {
                Ok(KvEntry {
                    key: row.get(0)?,
                    value: row.get(1)?,
                    updated_at: row.get(2)?,
                    source: row.get(3).ok(),
                    confidence: row.get(4).ok(),
                })
            })
            .map_err(|err| ToolError::ExecFailed(format!("query kv failed: {err}")))?;
        for row in rows {
            out.push(
                row.map_err(|err| ToolError::ExecFailed(format!("read kv row failed: {err}")))?,
            );
        }
    }

    Ok(out)
}

fn query_facts(
    conn: &Connection,
    keyword: Option<&str>,
    limit: usize,
) -> Result<Vec<FactEntry>, ToolError> {
    let limit = i64::try_from(limit)
        .map_err(|_| ToolError::InvalidArgs("limit is too large".to_string()))?;
    let mut out = Vec::new();

    if let Some(keyword) = keyword {
        let pattern = format!("%{}%", keyword.trim());
        let mut stmt = conn
            .prepare(
                "SELECT id, subject, predicate, object, updated_at, source
                 FROM facts
                 WHERE subject LIKE ?1 OR predicate LIKE ?1 OR object LIKE ?1
                 ORDER BY updated_at DESC
                 LIMIT ?2",
            )
            .map_err(|err| ToolError::ExecFailed(format!("prepare facts query failed: {err}")))?;
        let rows = stmt
            .query_map(params![pattern, limit], |row| {
                Ok(FactEntry {
                    id: row.get(0)?,
                    subject: row.get(1)?,
                    predicate: row.get(2)?,
                    object: row.get(3)?,
                    updated_at: row.get(4)?,
                    source: row.get(5).ok(),
                })
            })
            .map_err(|err| ToolError::ExecFailed(format!("query facts failed: {err}")))?;
        for row in rows {
            out.push(
                row.map_err(|err| ToolError::ExecFailed(format!("read facts row failed: {err}")))?,
            );
        }
    } else {
        let mut stmt = conn
            .prepare(
                "SELECT id, subject, predicate, object, updated_at, source
                 FROM facts
                 ORDER BY updated_at DESC
                 LIMIT ?1",
            )
            .map_err(|err| ToolError::ExecFailed(format!("prepare facts query failed: {err}")))?;
        let rows = stmt
            .query_map(params![limit], |row| {
                Ok(FactEntry {
                    id: row.get(0)?,
                    subject: row.get(1)?,
                    predicate: row.get(2)?,
                    object: row.get(3)?,
                    updated_at: row.get(4)?,
                    source: row.get(5).ok(),
                })
            })
            .map_err(|err| ToolError::ExecFailed(format!("query facts failed: {err}")))?;
        for row in rows {
            out.push(
                row.map_err(|err| ToolError::ExecFailed(format!("read facts row failed: {err}")))?,
            );
        }
    }

    Ok(out)
}

fn query_events(
    conn: &Connection,
    keyword: Option<&str>,
    limit: usize,
) -> Result<Vec<EventEntry>, ToolError> {
    let limit = i64::try_from(limit)
        .map_err(|_| ToolError::InvalidArgs("limit is too large".to_string()))?;
    let mut out = Vec::new();

    if let Some(keyword) = keyword {
        let pattern = format!("%{}%", keyword.trim());
        let mut stmt = conn
            .prepare(
                "SELECT id, type, payload, ts
                 FROM events
                 WHERE type LIKE ?1 OR payload LIKE ?1
                 ORDER BY ts DESC
                 LIMIT ?2",
            )
            .map_err(|err| ToolError::ExecFailed(format!("prepare events query failed: {err}")))?;
        let rows = stmt
            .query_map(params![pattern, limit], |row| {
                Ok(EventEntry {
                    id: row.get(0)?,
                    event_type: row.get(1)?,
                    payload: row.get(2)?,
                    ts: row.get(3)?,
                })
            })
            .map_err(|err| ToolError::ExecFailed(format!("query events failed: {err}")))?;
        for row in rows {
            out.push(
                row.map_err(|err| ToolError::ExecFailed(format!("read events row failed: {err}")))?,
            );
        }
    } else {
        let mut stmt = conn
            .prepare(
                "SELECT id, type, payload, ts
                 FROM events
                 ORDER BY ts DESC
                 LIMIT ?1",
            )
            .map_err(|err| ToolError::ExecFailed(format!("prepare events query failed: {err}")))?;
        let rows = stmt
            .query_map(params![limit], |row| {
                Ok(EventEntry {
                    id: row.get(0)?,
                    event_type: row.get(1)?,
                    payload: row.get(2)?,
                    ts: row.get(3)?,
                })
            })
            .map_err(|err| ToolError::ExecFailed(format!("query events failed: {err}")))?;
        for row in rows {
            out.push(
                row.map_err(|err| ToolError::ExecFailed(format!("read events row failed: {err}")))?,
            );
        }
    }

    Ok(out)
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

fn approx_tokens(text: &str) -> u32 {
    let chars = text.chars().count();
    ((chars + 3) / 4) as u32
}

fn truncate_by_token_limit(content: &str, token_limit: u32) -> (String, bool) {
    let max_chars = token_limit.saturating_mul(4) as usize;
    let mut out = String::new();
    for (idx, ch) in content.chars().enumerate() {
        if idx >= max_chars {
            return (out, true);
        }
        out.push(ch);
    }
    (out, false)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_tool::{ToolCall, ToolCallContext};
    use tempfile::tempdir;

    fn test_ctx() -> ToolCallContext {
        ToolCallContext {
            trace_id: "trace-memory".to_string(),
            agent_did: "did:example:agent".to_string(),
            behavior: "on_wakeup".to_string(),
            step_idx: 0,
            wakeup_id: "wakeup-memory".to_string(),
        }
    }

    #[tokio::test]
    async fn list_and_load_memory_work() {
        let tmp = tempdir().expect("create tempdir");
        let memory = AgentMemory::new(AgentMemoryConfig::new(tmp.path()), None)
            .await
            .expect("create agent memory");

        fs::write(
            memory.memory_md_path(),
            "line1: very important facts\nline2: more details\nline3: extra context",
        )
        .await
        .expect("write memory");

        let tools = ToolManager::new();
        memory.register_tools(&tools).expect("register tools");

        let list = tools
            .call_tool(
                &test_ctx(),
                ToolCall {
                    name: TOOL_LIST.to_string(),
                    args: json!({}),
                    call_id: "list-1".to_string(),
                },
            )
            .await
            .expect("call list");
        let entries = list["entries"].as_array().expect("entries array");
        assert!(entries.iter().any(|entry| entry["path"] == "memory.md"));
        assert!(entries.iter().any(|entry| entry["path"] == "things.db"));

        let loaded = tools
            .call_tool(
                &test_ctx(),
                ToolCall {
                    name: TOOL_LOAD_MEMORY.to_string(),
                    args: json!({ "token_limit": 4 }),
                    call_id: "load-memory-1".to_string(),
                },
            )
            .await
            .expect("call load_memory");
        assert_eq!(loaded["truncated"], true);
        assert!(loaded["content"]
            .as_str()
            .expect("memory content string")
            .starts_with("line1: very"));
    }

    #[tokio::test]
    async fn load_things_queries_keyword() {
        let tmp = tempdir().expect("create tempdir");
        let memory = AgentMemory::new(AgentMemoryConfig::new(tmp.path()), None)
            .await
            .expect("create agent memory");

        let db_path = memory.things_db_path().to_path_buf();
        task::spawn_blocking(move || {
            let conn = Connection::open(&db_path).expect("open db");
            ensure_things_db_schema(&conn).expect("ensure schema");
            conn.execute(
                "INSERT OR REPLACE INTO kv(key, value, updated_at, source, confidence)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    "user.preference.language",
                    "zh-CN",
                    100_i64,
                    "unit-test",
                    0.9_f64
                ],
            )
            .expect("insert kv");
            conn.execute(
                "INSERT OR REPLACE INTO facts(id, subject, predicate, object, updated_at, source)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    "fact-1",
                    "user",
                    "prefers",
                    "concise response",
                    101_i64,
                    "unit-test"
                ],
            )
            .expect("insert fact");
            conn.execute(
                "INSERT OR REPLACE INTO events(id, type, payload, ts)
                 VALUES (?1, ?2, ?3, ?4)",
                params!["event-1", "chat", "{\"topic\":\"language\"}", 102_i64],
            )
            .expect("insert event");
        })
        .await
        .expect("join insert");

        let tools = ToolManager::new();
        memory.register_tools(&tools).expect("register tools");

        let loaded = tools
            .call_tool(
                &test_ctx(),
                ToolCall {
                    name: TOOL_LOAD_THINGS.to_string(),
                    args: json!({"name":"language", "limit": 8}),
                    call_id: "load-things-1".to_string(),
                },
            )
            .await
            .expect("call load_things");

        assert_eq!(loaded["kv"].as_array().map(|v| v.len()), Some(1));
        assert_eq!(loaded["facts"].as_array().map(|v| v.len()), Some(0));
        assert_eq!(loaded["events"].as_array().map(|v| v.len()), Some(1));
    }

    #[tokio::test]
    async fn load_chat_history_requires_msg_center_client() {
        let tmp = tempdir().expect("create tempdir");
        let memory = AgentMemory::new(AgentMemoryConfig::new(tmp.path()), None)
            .await
            .expect("create agent memory");
        let tools = ToolManager::new();
        memory.register_tools(&tools).expect("register tools");

        let err = tools
            .call_tool(
                &test_ctx(),
                ToolCall {
                    name: TOOL_LOAC_CHAT_HISTORY_ALIAS.to_string(),
                    args: json!({"owner_did":"did:bns:alice"}),
                    call_id: "load-chat-1".to_string(),
                },
            )
            .await
            .expect_err("missing msg center should fail");

        assert!(matches!(err, ToolError::ExecFailed(_)));
        assert!(err
            .to_string()
            .contains("msg_center client is not configured"));
    }
}
