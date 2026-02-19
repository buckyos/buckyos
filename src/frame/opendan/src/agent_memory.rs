use std::path::{Component, Path, PathBuf};

use async_trait::async_trait;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value as Json};
use tokio::fs;
use tokio::task;

use crate::agent_tool::{AgentTool, ToolCallContext, ToolError, ToolManager, ToolSpec};

pub const TOOL_LIST: &str = "list";
pub const TOOL_LOAD_MEMORY: &str = "load_memory";
pub const TOOL_LOAD_THINGS: &str = "load_things";
pub const TOOL_DELETE_BY_SOURCE_SESSION: &str = "delete_by_source_session";

const DEFAULT_MEMORY_DIR_REL_PATH: &str = "memory";
const DEFAULT_MEMORY_MD_REL_PATH: &str = "memory/memory.md";
const DEFAULT_THINGS_DB_REL_PATH: &str = "memory/things.db";
const DEFAULT_MEMORY_TOKEN_LIMIT: u32 = 2_000;
const DEFAULT_LIST_MAX_ENTRIES: usize = 128;
const DEFAULT_TABLE_LIMIT: usize = 32;
const THING_TYPE_FACT: &str = "fact";
const THING_TYPE_EVENT: &str = "event";

#[derive(Clone, Debug)]
pub struct AgentMemoryConfig {
    pub agent_root: PathBuf,
    pub memory_dir_rel_path: PathBuf,
    pub memory_md_rel_path: PathBuf,
    pub things_db_rel_path: PathBuf,
    pub default_memory_token_limit: u32,
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
}

impl AgentMemory {
    pub async fn new(mut cfg: AgentMemoryConfig) -> Result<Self, ToolError> {
        if cfg.default_memory_token_limit == 0 {
            cfg.default_memory_token_limit = DEFAULT_MEMORY_TOKEN_LIMIT;
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
        tool_mgr.register_tool(DeleteBySourceSessionTool {
            memory: self.clone(),
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

    async fn delete_by_source_session(&self, source_session: String) -> Result<Json, ToolError> {
        let source_session = source_session.trim().to_string();
        if source_session.is_empty() {
            return Err(ToolError::InvalidArgs(
                "arg `source_session` cannot be empty".to_string(),
            ));
        }
        let db_path = self.things_db_path.clone();
        let source_session_for_query = source_session.clone();

        let deleted = task::spawn_blocking(move || -> Result<DeletedThingsSummary, ToolError> {
            let conn = Connection::open(&db_path).map_err(|err| {
                ToolError::ExecFailed(format!(
                    "open things db `{}` failed: {err}",
                    db_path.display()
                ))
            })?;
            ensure_things_db_schema(&conn)?;

            let deleted_facts = conn
                .query_row(
                    "SELECT COUNT(*) FROM things WHERE source_session = ?1 AND thing_type = ?2",
                    params![source_session_for_query, THING_TYPE_FACT],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|err| ToolError::ExecFailed(format!("count fact things failed: {err}")))?;

            let deleted_events = conn
                .query_row(
                    "SELECT COUNT(*) FROM things WHERE source_session = ?1 AND thing_type = ?2",
                    params![source_session_for_query, THING_TYPE_EVENT],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|err| {
                    ToolError::ExecFailed(format!("count event things failed: {err}"))
                })?;

            let deleted_things = conn
                .execute(
                    "DELETE FROM things WHERE source_session = ?1",
                    params![source_session_for_query],
                )
                .map_err(|err| {
                    ToolError::ExecFailed(format!("delete things by session failed: {err}"))
                })?;

            Ok(DeletedThingsSummary {
                deleted_things: i64::try_from(deleted_things)
                    .map_err(|_| ToolError::ExecFailed("deleted rows overflow i64".to_string()))?,
                deleted_facts,
                deleted_events,
            })
        })
        .await
        .map_err(|err| ToolError::ExecFailed(format!("delete things db join error: {err}")))??;

        Ok(json!({
            "path": self.things_db_path.to_string_lossy().to_string(),
            "source_session": source_session,
            "deleted_things": deleted.deleted_things,
            "deleted_facts": deleted.deleted_facts,
            "deleted_events": deleted.deleted_events,
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
struct DeleteBySourceSessionTool {
    memory: AgentMemory,
}

#[async_trait]
impl AgentTool for DeleteBySourceSessionTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_DELETE_BY_SOURCE_SESSION.to_string(),
            description: "Delete things rows by source_session.".to_string(),
            args_schema: json!({
                "type":"object",
                "properties": {
                    "source_session": {"type":"string"}
                },
                "required": ["source_session"],
                "additionalProperties": true
            }),
            output_schema: json!({
                "type":"object",
                "properties": {
                    "path": {"type":"string"},
                    "source_session": {"type":"string"},
                    "deleted_things": {"type":"integer"},
                    "deleted_facts": {"type":"integer"},
                    "deleted_events": {"type":"integer"}
                }
            }),
        }
    }

    async fn call(&self, _ctx: &ToolCallContext, args: Json) -> Result<Json, ToolError> {
        let source_session = required_string_arg(&args, "source_session")?;
        self.memory.delete_by_source_session(source_session).await
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
    source_session: Option<String>,
}

#[derive(Debug, Serialize)]
struct EventEntry {
    id: String,
    event_type: String,
    payload: String,
    ts: i64,
    source: Option<String>,
    source_session: Option<String>,
}

#[derive(Debug, Serialize)]
struct ThingsSnapshot {
    kv: Vec<KvEntry>,
    facts: Vec<FactEntry>,
    events: Vec<EventEntry>,
}

#[derive(Debug)]
struct DeletedThingsSummary {
    deleted_things: i64,
    deleted_facts: i64,
    deleted_events: i64,
}

fn required_string_arg(args: &Json, key: &str) -> Result<String, ToolError> {
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
CREATE TABLE IF NOT EXISTS things (
    id TEXT PRIMARY KEY,
    thing_type TEXT NOT NULL,
    subject TEXT NOT NULL,
    predicate TEXT NOT NULL,
    object TEXT NOT NULL,
    updated_at INTEGER NOT NULL DEFAULT 0,
    source TEXT,
    source_session TEXT
);
CREATE INDEX IF NOT EXISTS idx_things_type_updated_at
    ON things(thing_type, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_things_source_session
    ON things(source_session);
"#,
    )
    .map_err(|err| ToolError::ExecFailed(format!("ensure things schema failed: {err}")))?;

    migrate_legacy_things_tables(conn)?;
    rebuild_legacy_things_aliases(conn)?;
    Ok(())
}

fn migrate_legacy_things_tables(conn: &Connection) -> Result<(), ToolError> {
    if is_table(conn, "facts")? {
        conn.execute(
            "INSERT INTO things(
                 id, thing_type, subject, predicate, object, updated_at, source, source_session
             )
             SELECT id, ?1, subject, predicate, object, updated_at, source, NULL
             FROM facts
             ON CONFLICT(id) DO UPDATE SET
                 thing_type = excluded.thing_type,
                 subject = excluded.subject,
                 predicate = excluded.predicate,
                 object = excluded.object,
                 updated_at = excluded.updated_at,
                 source = excluded.source,
                 source_session = excluded.source_session",
            params![THING_TYPE_FACT],
        )
        .map_err(|err| ToolError::ExecFailed(format!("migrate facts into things failed: {err}")))?;
        conn.execute("DROP TABLE facts", []).map_err(|err| {
            ToolError::ExecFailed(format!("drop legacy facts table failed: {err}"))
        })?;
    }

    if is_table(conn, "events")? {
        conn.execute(
            "INSERT INTO things(
                 id, thing_type, subject, predicate, object, updated_at, source, source_session
             )
             SELECT id, ?1, type, type, payload, ts, NULL, NULL
             FROM events
             ON CONFLICT(id) DO UPDATE SET
                 thing_type = excluded.thing_type,
                 subject = excluded.subject,
                 predicate = excluded.predicate,
                 object = excluded.object,
                 updated_at = excluded.updated_at,
                 source = excluded.source,
                 source_session = excluded.source_session",
            params![THING_TYPE_EVENT],
        )
        .map_err(|err| {
            ToolError::ExecFailed(format!("migrate events into things failed: {err}"))
        })?;
        conn.execute("DROP TABLE events", []).map_err(|err| {
            ToolError::ExecFailed(format!("drop legacy events table failed: {err}"))
        })?;
    }

    Ok(())
}

fn rebuild_legacy_things_aliases(conn: &Connection) -> Result<(), ToolError> {
    conn.execute_batch(
        r#"
CREATE VIEW IF NOT EXISTS facts AS
SELECT
    id,
    subject,
    predicate,
    object,
    updated_at,
    source,
    source_session
FROM things
WHERE thing_type = 'fact';

CREATE VIEW IF NOT EXISTS events AS
SELECT
    id,
    predicate AS type,
    object AS payload,
    updated_at AS ts,
    source,
    source_session
FROM things
WHERE thing_type = 'event';

CREATE TRIGGER IF NOT EXISTS facts_insert
INSTEAD OF INSERT ON facts
BEGIN
    INSERT INTO things(
        id, thing_type, subject, predicate, object, updated_at, source, source_session
    )
    VALUES (
        NEW.id,
        'fact',
        COALESCE(NEW.subject, ''),
        COALESCE(NEW.predicate, ''),
        COALESCE(NEW.object, ''),
        COALESCE(NEW.updated_at, 0),
        NEW.source,
        NEW.source_session
    )
    ON CONFLICT(id) DO UPDATE SET
        thing_type = excluded.thing_type,
        subject = excluded.subject,
        predicate = excluded.predicate,
        object = excluded.object,
        updated_at = excluded.updated_at,
        source = excluded.source,
        source_session = excluded.source_session;
END;

CREATE TRIGGER IF NOT EXISTS facts_update
INSTEAD OF UPDATE ON facts
BEGIN
    DELETE FROM things WHERE id = OLD.id AND thing_type = 'fact';
    INSERT INTO things(
        id, thing_type, subject, predicate, object, updated_at, source, source_session
    )
    VALUES (
        COALESCE(NEW.id, OLD.id),
        'fact',
        COALESCE(NEW.subject, ''),
        COALESCE(NEW.predicate, ''),
        COALESCE(NEW.object, ''),
        COALESCE(NEW.updated_at, 0),
        NEW.source,
        NEW.source_session
    )
    ON CONFLICT(id) DO UPDATE SET
        thing_type = excluded.thing_type,
        subject = excluded.subject,
        predicate = excluded.predicate,
        object = excluded.object,
        updated_at = excluded.updated_at,
        source = excluded.source,
        source_session = excluded.source_session;
END;

CREATE TRIGGER IF NOT EXISTS facts_delete
INSTEAD OF DELETE ON facts
BEGIN
    DELETE FROM things WHERE id = OLD.id AND thing_type = 'fact';
END;

CREATE TRIGGER IF NOT EXISTS events_insert
INSTEAD OF INSERT ON events
BEGIN
    INSERT INTO things(
        id, thing_type, subject, predicate, object, updated_at, source, source_session
    )
    VALUES (
        NEW.id,
        'event',
        COALESCE(NEW.type, ''),
        COALESCE(NEW.type, ''),
        COALESCE(NEW.payload, ''),
        COALESCE(NEW.ts, 0),
        NEW.source,
        NEW.source_session
    )
    ON CONFLICT(id) DO UPDATE SET
        thing_type = excluded.thing_type,
        subject = excluded.subject,
        predicate = excluded.predicate,
        object = excluded.object,
        updated_at = excluded.updated_at,
        source = excluded.source,
        source_session = excluded.source_session;
END;

CREATE TRIGGER IF NOT EXISTS events_update
INSTEAD OF UPDATE ON events
BEGIN
    DELETE FROM things WHERE id = OLD.id AND thing_type = 'event';
    INSERT INTO things(
        id, thing_type, subject, predicate, object, updated_at, source, source_session
    )
    VALUES (
        COALESCE(NEW.id, OLD.id),
        'event',
        COALESCE(NEW.type, ''),
        COALESCE(NEW.type, ''),
        COALESCE(NEW.payload, ''),
        COALESCE(NEW.ts, 0),
        NEW.source,
        NEW.source_session
    )
    ON CONFLICT(id) DO UPDATE SET
        thing_type = excluded.thing_type,
        subject = excluded.subject,
        predicate = excluded.predicate,
        object = excluded.object,
        updated_at = excluded.updated_at,
        source = excluded.source,
        source_session = excluded.source_session;
END;

CREATE TRIGGER IF NOT EXISTS events_delete
INSTEAD OF DELETE ON events
BEGIN
    DELETE FROM things WHERE id = OLD.id AND thing_type = 'event';
END;
"#,
    )
    .map_err(|err| ToolError::ExecFailed(format!("ensure things alias schema failed: {err}")))
}

fn schema_object_type(conn: &Connection, name: &str) -> Result<Option<String>, ToolError> {
    conn.query_row(
        "SELECT type FROM sqlite_master WHERE name = ?1 LIMIT 1",
        params![name],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(|err| ToolError::ExecFailed(format!("query sqlite_master failed: {err}")))
}

fn is_table(conn: &Connection, name: &str) -> Result<bool, ToolError> {
    Ok(matches!(
        schema_object_type(conn, name)?.as_deref(),
        Some("table")
    ))
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
                "SELECT id, subject, predicate, object, updated_at, source, source_session
                 FROM things
                 WHERE thing_type = ?1
                   AND (subject LIKE ?2 OR predicate LIKE ?2 OR object LIKE ?2
                        OR source LIKE ?2 OR source_session LIKE ?2)
                 ORDER BY updated_at DESC
                 LIMIT ?3",
            )
            .map_err(|err| {
                ToolError::ExecFailed(format!("prepare things(fact) query failed: {err}"))
            })?;
        let rows = stmt
            .query_map(params![THING_TYPE_FACT, pattern, limit], |row| {
                Ok(FactEntry {
                    id: row.get(0)?,
                    subject: row.get(1)?,
                    predicate: row.get(2)?,
                    object: row.get(3)?,
                    updated_at: row.get(4)?,
                    source: row.get(5).ok(),
                    source_session: row.get(6).ok(),
                })
            })
            .map_err(|err| ToolError::ExecFailed(format!("query things(fact) failed: {err}")))?;
        for row in rows {
            out.push(
                row.map_err(|err| ToolError::ExecFailed(format!("read facts row failed: {err}")))?,
            );
        }
    } else {
        let mut stmt = conn
            .prepare(
                "SELECT id, subject, predicate, object, updated_at, source, source_session
                 FROM things
                 WHERE thing_type = ?1
                 ORDER BY updated_at DESC
                 LIMIT ?2",
            )
            .map_err(|err| {
                ToolError::ExecFailed(format!("prepare things(fact) query failed: {err}"))
            })?;
        let rows = stmt
            .query_map(params![THING_TYPE_FACT, limit], |row| {
                Ok(FactEntry {
                    id: row.get(0)?,
                    subject: row.get(1)?,
                    predicate: row.get(2)?,
                    object: row.get(3)?,
                    updated_at: row.get(4)?,
                    source: row.get(5).ok(),
                    source_session: row.get(6).ok(),
                })
            })
            .map_err(|err| ToolError::ExecFailed(format!("query things(fact) failed: {err}")))?;
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
                "SELECT id, subject, predicate, object, updated_at, source, source_session
                 FROM things
                 WHERE thing_type = ?1
                   AND (subject LIKE ?2 OR predicate LIKE ?2 OR object LIKE ?2
                        OR source LIKE ?2 OR source_session LIKE ?2)
                 ORDER BY updated_at DESC
                 LIMIT ?3",
            )
            .map_err(|err| {
                ToolError::ExecFailed(format!("prepare things(event) query failed: {err}"))
            })?;
        let rows = stmt
            .query_map(params![THING_TYPE_EVENT, pattern, limit], |row| {
                Ok(EventEntry {
                    id: row.get(0)?,
                    event_type: row.get(2)?,
                    payload: row.get(3)?,
                    ts: row.get(4)?,
                    source: row.get(5).ok(),
                    source_session: row.get(6).ok(),
                })
            })
            .map_err(|err| ToolError::ExecFailed(format!("query things(event) failed: {err}")))?;
        for row in rows {
            out.push(
                row.map_err(|err| ToolError::ExecFailed(format!("read events row failed: {err}")))?,
            );
        }
    } else {
        let mut stmt = conn
            .prepare(
                "SELECT id, subject, predicate, object, updated_at, source, source_session
                 FROM things
                 WHERE thing_type = ?1
                 ORDER BY updated_at DESC
                 LIMIT ?2",
            )
            .map_err(|err| {
                ToolError::ExecFailed(format!("prepare things(event) query failed: {err}"))
            })?;
        let rows = stmt
            .query_map(params![THING_TYPE_EVENT, limit], |row| {
                Ok(EventEntry {
                    id: row.get(0)?,
                    event_type: row.get(2)?,
                    payload: row.get(3)?,
                    ts: row.get(4)?,
                    source: row.get(5).ok(),
                    source_session: row.get(6).ok(),
                })
            })
            .map_err(|err| ToolError::ExecFailed(format!("query things(event) failed: {err}")))?;
        for row in rows {
            out.push(
                row.map_err(|err| ToolError::ExecFailed(format!("read events row failed: {err}")))?,
            );
        }
    }

    Ok(out)
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
            current_session_id: None,
        }
    }

    #[tokio::test]
    async fn list_and_load_memory_work() {
        let tmp = tempdir().expect("create tempdir");
        let memory = AgentMemory::new(AgentMemoryConfig::new(tmp.path()))
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
        let memory = AgentMemory::new(AgentMemoryConfig::new(tmp.path()))
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
                "INSERT OR REPLACE INTO things(
                     id, thing_type, subject, predicate, object, updated_at, source, source_session
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    "fact-1",
                    THING_TYPE_FACT,
                    "user",
                    "prefers",
                    "concise response",
                    101_i64,
                    "unit-test",
                    "session-1"
                ],
            )
            .expect("insert fact");
            conn.execute(
                "INSERT OR REPLACE INTO things(
                     id, thing_type, subject, predicate, object, updated_at, source, source_session
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    "event-1",
                    THING_TYPE_EVENT,
                    "conversation",
                    "chat",
                    "{\"topic\":\"language\"}",
                    102_i64,
                    "unit-test",
                    "session-1"
                ],
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
    async fn load_things_supports_legacy_fact_event_aliases() {
        let tmp = tempdir().expect("create tempdir");
        let memory = AgentMemory::new(AgentMemoryConfig::new(tmp.path()))
            .await
            .expect("create agent memory");

        let db_path = memory.things_db_path().to_path_buf();
        let counts = task::spawn_blocking(move || {
            let conn = Connection::open(&db_path).expect("open db");
            ensure_things_db_schema(&conn).expect("ensure schema");
            conn.execute(
                "INSERT OR REPLACE INTO facts(
                     id, subject, predicate, object, updated_at, source, source_session
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    "legacy-fact-1",
                    "project",
                    "status",
                    "legacy path still works",
                    200_i64,
                    "legacy-test",
                    "session-legacy"
                ],
            )
            .expect("insert legacy fact");
            conn.execute(
                "INSERT OR REPLACE INTO events(id, type, payload, ts)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    "legacy-event-1",
                    "legacy",
                    "{\"topic\":\"legacy\"}",
                    201_i64
                ],
            )
            .expect("insert legacy event");

            let fact_count = conn
                .query_row(
                    "SELECT COUNT(*) FROM things WHERE thing_type = ?1",
                    params![THING_TYPE_FACT],
                    |row| row.get::<_, i64>(0),
                )
                .expect("count fact things");
            let event_count = conn
                .query_row(
                    "SELECT COUNT(*) FROM things WHERE thing_type = ?1",
                    params![THING_TYPE_EVENT],
                    |row| row.get::<_, i64>(0),
                )
                .expect("count event things");
            (fact_count, event_count)
        })
        .await
        .expect("join insert");

        assert_eq!(counts.0, 1);
        assert_eq!(counts.1, 1);

        let tools = ToolManager::new();
        memory.register_tools(&tools).expect("register tools");
        let loaded = tools
            .call_tool(
                &test_ctx(),
                ToolCall {
                    name: TOOL_LOAD_THINGS.to_string(),
                    args: json!({"name":"legacy", "limit": 8}),
                    call_id: "load-things-legacy-1".to_string(),
                },
            )
            .await
            .expect("call load_things");

        assert_eq!(loaded["facts"].as_array().map(|v| v.len()), Some(1));
        assert_eq!(loaded["events"].as_array().map(|v| v.len()), Some(1));
    }

    #[tokio::test]
    async fn delete_by_source_session_removes_target_only() {
        let tmp = tempdir().expect("create tempdir");
        let memory = AgentMemory::new(AgentMemoryConfig::new(tmp.path()))
            .await
            .expect("create agent memory");
        let db_path = memory.things_db_path().to_path_buf();

        task::spawn_blocking(move || {
            let conn = Connection::open(&db_path).expect("open db");
            ensure_things_db_schema(&conn).expect("ensure schema");

            conn.execute(
                "INSERT OR REPLACE INTO things(
                     id, thing_type, subject, predicate, object, updated_at, source, source_session
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    "fact-session-a",
                    THING_TYPE_FACT,
                    "agent",
                    "preference",
                    "concise",
                    101_i64,
                    "unit-test",
                    "session-a"
                ],
            )
            .expect("insert fact-session-a");
            conn.execute(
                "INSERT OR REPLACE INTO things(
                     id, thing_type, subject, predicate, object, updated_at, source, source_session
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    "event-session-a",
                    THING_TYPE_EVENT,
                    "conversation",
                    "chat",
                    "{\"topic\":\"cleanup\"}",
                    102_i64,
                    "unit-test",
                    "session-a"
                ],
            )
            .expect("insert event-session-a");
            conn.execute(
                "INSERT OR REPLACE INTO things(
                     id, thing_type, subject, predicate, object, updated_at, source, source_session
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    "fact-session-b",
                    THING_TYPE_FACT,
                    "agent",
                    "status",
                    "active",
                    103_i64,
                    "unit-test",
                    "session-b"
                ],
            )
            .expect("insert fact-session-b");
            conn.execute(
                "INSERT OR REPLACE INTO things(
                     id, thing_type, subject, predicate, object, updated_at, source, source_session
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    "event-no-session",
                    THING_TYPE_EVENT,
                    "conversation",
                    "chat",
                    "{\"topic\":\"global\"}",
                    104_i64,
                    "unit-test",
                    Option::<String>::None
                ],
            )
            .expect("insert event-no-session");
        })
        .await
        .expect("join insert");

        let tools = ToolManager::new();
        memory.register_tools(&tools).expect("register tools");
        let deleted = tools
            .call_tool(
                &test_ctx(),
                ToolCall {
                    name: TOOL_DELETE_BY_SOURCE_SESSION.to_string(),
                    args: json!({"source_session":"session-a"}),
                    call_id: "delete-by-session-1".to_string(),
                },
            )
            .await
            .expect("delete by source_session");

        assert_eq!(deleted["deleted_things"].as_i64(), Some(2));
        assert_eq!(deleted["deleted_facts"].as_i64(), Some(1));
        assert_eq!(deleted["deleted_events"].as_i64(), Some(1));

        let db_path = memory.things_db_path().to_path_buf();
        let remained = task::spawn_blocking(move || {
            let conn = Connection::open(&db_path).expect("open db");
            let things_total = conn
                .query_row("SELECT COUNT(*) FROM things", [], |row| {
                    row.get::<_, i64>(0)
                })
                .expect("count things");
            let facts_session_a = conn
                .query_row(
                    "SELECT COUNT(*) FROM facts WHERE source_session = 'session-a'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .expect("count facts session-a");
            let facts_session_b = conn
                .query_row(
                    "SELECT COUNT(*) FROM facts WHERE source_session = 'session-b'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .expect("count facts session-b");
            let events_session_a = conn
                .query_row(
                    "SELECT COUNT(*) FROM events WHERE source_session = 'session-a'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .expect("count events session-a");
            let events_without_session = conn
                .query_row(
                    "SELECT COUNT(*) FROM events WHERE source_session IS NULL",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .expect("count events without session");

            (
                things_total,
                facts_session_a,
                facts_session_b,
                events_session_a,
                events_without_session,
            )
        })
        .await
        .expect("join readback");

        assert_eq!(remained.0, 2);
        assert_eq!(remained.1, 0);
        assert_eq!(remained.2, 1);
        assert_eq!(remained.3, 0);
        assert_eq!(remained.4, 1);
    }

    #[tokio::test]
    async fn delete_by_source_session_requires_non_empty_value() {
        let tmp = tempdir().expect("create tempdir");
        let memory = AgentMemory::new(AgentMemoryConfig::new(tmp.path()))
            .await
            .expect("create agent memory");
        let tools = ToolManager::new();
        memory.register_tools(&tools).expect("register tools");

        let err = tools
            .call_tool(
                &test_ctx(),
                ToolCall {
                    name: TOOL_DELETE_BY_SOURCE_SESSION.to_string(),
                    args: json!({"source_session":"   "}),
                    call_id: "delete-by-session-invalid-1".to_string(),
                },
            )
            .await
            .expect_err("empty source_session should fail");

        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }
}
