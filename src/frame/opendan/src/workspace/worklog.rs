use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use rusqlite::{params, params_from_iter, types::Value as SqlValue, Connection};
use serde::Serialize;
use serde_json::{json, Value as Json};
use tokio::task;

use crate::agent_tool::{AgentTool, ToolError, ToolSpec};
use crate::behavior::TraceCtx;

pub const TOOL_WORKLOG_MANAGE: &str = "worklog_manage";

const DEFAULT_LIST_LIMIT: usize = 64;
const DEFAULT_MAX_LIST_LIMIT: usize = 256;
const MAX_TEXT_FIELD_LEN: usize = 256;
const MAX_SUMMARY_LEN: usize = 2048;
const MAX_TAGS: usize = 32;
const MAX_TAG_LEN: usize = 64;
const MAX_PAYLOAD_BYTES: usize = 64 * 1024;

static WORKLOG_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
pub struct WorklogToolConfig {
    pub db_path: PathBuf,
    pub default_list_limit: usize,
    pub max_list_limit: usize,
}

impl WorklogToolConfig {
    pub fn with_db_path(db_path: PathBuf) -> Self {
        Self {
            db_path,
            default_list_limit: DEFAULT_LIST_LIMIT,
            max_list_limit: DEFAULT_MAX_LIST_LIMIT,
        }
    }
}

#[derive(Clone, Debug)]
pub struct WorklogTool {
    cfg: WorklogToolConfig,
}

impl WorklogTool {
    pub fn new(mut cfg: WorklogToolConfig) -> Result<Self, ToolError> {
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
            std::fs::create_dir_all(parent).map_err(|err| {
                ToolError::ExecFailed(format!(
                    "create worklog db parent dir `{}` failed: {err}",
                    parent.display()
                ))
            })?;
        }

        let conn = Connection::open(&cfg.db_path).map_err(|err| {
            ToolError::ExecFailed(format!(
                "open worklog db `{}` failed: {err}",
                cfg.db_path.display()
            ))
        })?;
        ensure_worklog_schema(&conn)?;

        Ok(Self { cfg })
    }

    async fn run_db<F, T>(&self, op_name: &str, op: F) -> Result<T, ToolError>
    where
        F: FnOnce(&Connection) -> Result<T, ToolError> + Send + 'static,
        T: Send + 'static,
    {
        let db_path = self.cfg.db_path.clone();
        task::spawn_blocking(move || {
            let conn = Connection::open(&db_path).map_err(|err| {
                ToolError::ExecFailed(format!(
                    "open worklog db `{}` failed: {err}",
                    db_path.display()
                ))
            })?;
            ensure_worklog_schema(&conn)?;
            op(&conn)
        })
        .await
        .map_err(|err| ToolError::ExecFailed(format!("{op_name} join error: {err}")))?
    }
}

#[async_trait]
impl AgentTool for WorklogTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_WORKLOG_MANAGE.to_string(),
            description: "Append and query workspace worklog events backed by worklog/worklog.db."
                .to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["append", "list", "get", "delete"]
                    },
                    "log_id": { "type": "string" },
                    "type": { "type": "string" },
                    "status": { "type": "string", "enum": ["info", "success", "failed", "partial"] },
                    "agent_id": { "type": "string" },
                    "owner_session_id": { "type": ["string", "null"] },
                    "related_agent_id": { "type": "string" },
                    "run_id": { "type": "string" },
                    "step_id": { "type": "string" },
                    "task_id": { "type": "string" },
                    "summary": { "type": "string" },
                    "payload": { "type": "object" },
                    "tags": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "timestamp": { "type": "integer", "minimum": 1 },
                    "duration": { "type": "integer", "minimum": 0 },
                    "tag": { "type": "string" },
                    "query": { "type": "string" },
                    "from_ts": { "type": "integer", "minimum": 1 },
                    "to_ts": { "type": "integer", "minimum": 1 },
                    "limit": { "type": "integer", "minimum": 1 },
                    "offset": { "type": "integer", "minimum": 0 },
                    "asc": { "type": "boolean", "default": false }
                },
                "required": ["action"],
                "additionalProperties": true
            }),
            output_schema: json!({
                "type": "object",
                "properties": {
                    "ok": { "type": "boolean" },
                    "action": { "type": "string" },
                    "log": { "type": "object" },
                    "logs": { "type": "array", "items": { "type": "object" } },
                    "total": { "type": "integer" },
                    "deleted": { "type": "boolean" }
                }
            }),
        }
    }

    async fn call(&self, ctx: &TraceCtx, args: Json) -> Result<Json, ToolError> {
        let action = require_action(&args)?;
        match action.as_str() {
            "append" => self.call_append(ctx, args).await,
            "list" => self.call_list(args).await,
            "get" => self.call_get(args).await,
            "delete" => self.call_delete(args).await,
            _ => Err(ToolError::InvalidArgs(format!(
                "unsupported action `{action}`, expected append/list/get/delete"
            ))),
        }
    }
}

impl WorklogTool {
    async fn call_append(&self, ctx: &TraceCtx, args: Json) -> Result<Json, ToolError> {
        let input = WorklogAppendInput::from_args(ctx, &args)?;
        let item = self
            .run_db("append worklog", move |conn| append_worklog(conn, input))
            .await?;
        Ok(json!({
            "ok": true,
            "action": "append",
            "log": item
        }))
    }

    async fn call_list(&self, args: Json) -> Result<Json, ToolError> {
        let filters = WorklogListFilters::from_args(&args, self.cfg.default_list_limit)?;
        let max_limit = self.cfg.max_list_limit;
        let rows = self
            .run_db("list worklogs", move |conn| {
                list_worklogs(conn, filters, max_limit)
            })
            .await?;

        Ok(json!({
            "ok": true,
            "action": "list",
            "logs": rows,
            "total": rows.len()
        }))
    }

    async fn call_get(&self, args: Json) -> Result<Json, ToolError> {
        let log_id = require_string(&args, "log_id")?;
        let lookup_id = log_id.clone();
        let item = self
            .run_db("get worklog", move |conn| {
                get_worklog_by_id(conn, &lookup_id)
            })
            .await?;
        let Some(item) = item else {
            return Err(ToolError::InvalidArgs(format!(
                "worklog `{log_id}` not found"
            )));
        };
        Ok(json!({
            "ok": true,
            "action": "get",
            "log": item
        }))
    }

    async fn call_delete(&self, args: Json) -> Result<Json, ToolError> {
        let log_id = require_string(&args, "log_id")?;
        let deleted = self
            .run_db("delete worklog", move |conn| delete_worklog(conn, &log_id))
            .await?;
        Ok(json!({
            "ok": true,
            "action": "delete",
            "deleted": deleted
        }))
    }
}

#[derive(Clone, Debug)]
struct WorklogAppendInput {
    log_id: String,
    log_type: String,
    status: String,
    agent_id: String,
    owner_session_id: Option<String>,
    related_agent_id: Option<String>,
    run_id: Option<String>,
    step_id: Option<String>,
    task_id: Option<String>,
    summary: String,
    payload: Json,
    tags: Vec<String>,
    timestamp: u64,
    duration: Option<u64>,
}

impl WorklogAppendInput {
    fn from_args(ctx: &TraceCtx, args: &Json) -> Result<Self, ToolError> {
        let log_id = optional_string(args, "log_id")?.unwrap_or_else(generate_worklog_id);
        let log_type = require_string(args, "type")?;
        let status = optional_string(args, "status")?
            .map(normalize_status)
            .transpose()?
            .unwrap_or_else(|| "info".to_string());
        let agent_id = optional_string(args, "agent_id")?.unwrap_or_else(|| ctx.agent_did.clone());
        let owner_session_id = optional_string(args, "owner_session_id")?;
        let related_agent_id = optional_string(args, "related_agent_id")?;
        let run_id = optional_string(args, "run_id")?;
        let step_id = optional_string(args, "step_id")?;
        let task_id = optional_string(args, "task_id")?;
        let summary = require_string(args, "summary")?;
        let payload = parse_payload(args.get("payload"))?;
        let tags = parse_tags(args.get("tags"))?;
        let timestamp = optional_u64(args, "timestamp")?.unwrap_or_else(now_ms);
        let duration = optional_u64(args, "duration")?;

        validate_text_field("log_id", &log_id, MAX_TEXT_FIELD_LEN)?;
        validate_text_field("type", &log_type, MAX_TEXT_FIELD_LEN)?;
        validate_text_field("agent_id", &agent_id, MAX_TEXT_FIELD_LEN)?;
        if let Some(v) = owner_session_id.as_deref() {
            validate_text_field("owner_session_id", v, MAX_TEXT_FIELD_LEN)?;
        }
        if let Some(v) = related_agent_id.as_deref() {
            validate_text_field("related_agent_id", v, MAX_TEXT_FIELD_LEN)?;
        }
        if let Some(v) = run_id.as_deref() {
            validate_text_field("run_id", v, MAX_TEXT_FIELD_LEN)?;
        }
        if let Some(v) = step_id.as_deref() {
            validate_text_field("step_id", v, MAX_TEXT_FIELD_LEN)?;
        }
        if let Some(v) = task_id.as_deref() {
            validate_text_field("task_id", v, MAX_TEXT_FIELD_LEN)?;
        }
        validate_summary(&summary)?;

        Ok(Self {
            log_id,
            log_type,
            status,
            agent_id,
            owner_session_id,
            related_agent_id,
            run_id,
            step_id,
            task_id,
            summary,
            payload,
            tags,
            timestamp,
            duration,
        })
    }
}

#[derive(Clone, Debug)]
struct WorklogListFilters {
    log_type: Option<String>,
    status: Option<String>,
    agent_id: Option<String>,
    owner_session_id: Option<String>,
    related_agent_id: Option<String>,
    run_id: Option<String>,
    step_id: Option<String>,
    task_id: Option<String>,
    tag: Option<String>,
    query: Option<String>,
    from_ts: Option<u64>,
    to_ts: Option<u64>,
    limit: usize,
    offset: usize,
    asc: bool,
}

impl WorklogListFilters {
    fn from_args(args: &Json, default_list_limit: usize) -> Result<Self, ToolError> {
        let log_type = optional_string(args, "type")?;
        let status =
            optional_string(args, "status").and_then(|v| v.map(normalize_status).transpose())?;
        let agent_id = optional_string(args, "agent_id")?;
        let owner_session_id = optional_string(args, "owner_session_id")?;
        let related_agent_id = optional_string(args, "related_agent_id")?;
        let run_id = optional_string(args, "run_id")?;
        let step_id = optional_string(args, "step_id")?;
        let task_id = optional_string(args, "task_id")?;
        let tag = optional_string(args, "tag")?;
        let query = optional_string(args, "query")?;
        let from_ts = optional_u64(args, "from_ts")?;
        let to_ts = optional_u64(args, "to_ts")?;
        let limit = optional_u64(args, "limit")?
            .map(|v| u64_to_usize(v, "limit"))
            .transpose()?
            .unwrap_or(default_list_limit);
        let offset = optional_u64(args, "offset")?
            .map(|v| u64_to_usize(v, "offset"))
            .transpose()?
            .unwrap_or(0);
        let asc = optional_bool(args, "asc")?.unwrap_or(false);

        Ok(Self {
            log_type,
            status,
            agent_id,
            owner_session_id,
            related_agent_id,
            run_id,
            step_id,
            task_id,
            tag,
            query: query.filter(|v| !v.is_empty()),
            from_ts,
            to_ts,
            limit,
            offset,
            asc,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
struct WorklogItem {
    log_id: String,
    #[serde(rename = "type")]
    log_type: String,
    status: String,
    agent_id: String,
    owner_session_id: Option<String>,
    related_agent_id: Option<String>,
    run_id: Option<String>,
    step_id: Option<String>,
    task_id: Option<String>,
    summary: String,
    payload: Json,
    tags: Vec<String>,
    timestamp: u64,
    duration: Option<u64>,
    created_at: u64,
    updated_at: u64,
}

fn ensure_worklog_schema(conn: &Connection) -> Result<(), ToolError> {
    conn.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS worklogs (
    log_id TEXT PRIMARY KEY,
    log_type TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'info',
    agent_id TEXT NOT NULL,
    owner_session_id TEXT,
    related_agent_id TEXT,
    run_id TEXT,
    step_id TEXT,
    task_id TEXT,
    summary TEXT NOT NULL,
    payload_json TEXT NOT NULL DEFAULT '{}',
    tags_json TEXT NOT NULL DEFAULT '[]',
    timestamp INTEGER NOT NULL,
    duration_ms INTEGER,
    created_at INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS worklog_tags (
    log_id TEXT NOT NULL,
    tag TEXT NOT NULL,
    PRIMARY KEY (log_id, tag)
);
CREATE INDEX IF NOT EXISTS idx_worklogs_ts ON worklogs(timestamp DESC, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_worklogs_type_ts ON worklogs(log_type, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_worklogs_status_ts ON worklogs(status, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_worklogs_agent_ts ON worklogs(agent_id, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_worklogs_step_ts ON worklogs(step_id, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_worklog_tags_tag ON worklog_tags(tag, log_id);
"#,
    )
    .map_err(|err| ToolError::ExecFailed(format!("ensure worklog schema failed: {err}")))?;

    ensure_worklog_column_exists(conn, "owner_session_id", "TEXT")?;
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_worklogs_owner_session_ts ON worklogs(owner_session_id, timestamp DESC);",
    )
    .map_err(|err| {
        ToolError::ExecFailed(format!("ensure worklog owner_session index failed: {err}"))
    })?;

    Ok(())
}

fn ensure_worklog_column_exists(
    conn: &Connection,
    column_name: &str,
    column_def_sql: &str,
) -> Result<(), ToolError> {
    if worklog_table_has_column(conn, column_name)? {
        return Ok(());
    }
    let sql = format!("ALTER TABLE worklogs ADD COLUMN {column_name} {column_def_sql}");
    conn.execute(&sql, []).map_err(|err| {
        ToolError::ExecFailed(format!("add worklog column `{column_name}` failed: {err}"))
    })?;
    Ok(())
}

fn worklog_table_has_column(conn: &Connection, column_name: &str) -> Result<bool, ToolError> {
    let mut stmt = conn.prepare("PRAGMA table_info(worklogs)").map_err(|err| {
        ToolError::ExecFailed(format!("prepare worklog table_info failed: {err}"))
    })?;
    let mut rows = stmt
        .query([])
        .map_err(|err| ToolError::ExecFailed(format!("query worklog table_info failed: {err}")))?;
    while let Some(row) = rows
        .next()
        .map_err(|err| ToolError::ExecFailed(format!("read worklog table_info failed: {err}")))?
    {
        let name: String = row.get(1).map_err(|err| {
            ToolError::ExecFailed(format!("decode worklog table_info failed: {err}"))
        })?;
        if name == column_name {
            return Ok(true);
        }
    }
    Ok(false)
}

fn append_worklog(conn: &Connection, input: WorklogAppendInput) -> Result<WorklogItem, ToolError> {
    let now = now_ms();
    let payload_json = serde_json::to_string(&input.payload)
        .map_err(|err| ToolError::ExecFailed(format!("serialize payload failed: {err}")))?;
    if payload_json.len() > MAX_PAYLOAD_BYTES {
        return Err(ToolError::InvalidArgs(format!(
            "`payload` exceeds max {} bytes",
            MAX_PAYLOAD_BYTES
        )));
    }

    let tags_json = serde_json::to_string(&input.tags)
        .map_err(|err| ToolError::ExecFailed(format!("serialize tags failed: {err}")))?;

    let tx = conn
        .unchecked_transaction()
        .map_err(|err| ToolError::ExecFailed(format!("begin transaction failed: {err}")))?;

    tx.execute(
        "INSERT INTO worklogs (
            log_id, log_type, status, agent_id, owner_session_id, related_agent_id, run_id, step_id, task_id,
            summary, payload_json, tags_json, timestamp, duration_ms, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        params![
            input.log_id,
            input.log_type,
            input.status,
            input.agent_id,
            input.owner_session_id,
            input.related_agent_id,
            input.run_id,
            input.step_id,
            input.task_id,
            input.summary,
            payload_json,
            tags_json,
            u64_to_i64(input.timestamp),
            input.duration.map(u64_to_i64),
            u64_to_i64(now),
            u64_to_i64(now),
        ],
    )
    .map_err(|err| ToolError::ExecFailed(format!("insert worklog failed: {err}")))?;

    for tag in input.tags {
        tx.execute(
            "INSERT OR IGNORE INTO worklog_tags (log_id, tag) VALUES (?1, ?2)",
            params![input.log_id, tag],
        )
        .map_err(|err| ToolError::ExecFailed(format!("insert worklog tag failed: {err}")))?;
    }

    tx.commit()
        .map_err(|err| ToolError::ExecFailed(format!("commit transaction failed: {err}")))?;

    get_worklog_by_id(conn, &input.log_id)?
        .ok_or_else(|| ToolError::ExecFailed("read created worklog failed".to_string()))
}

fn list_worklogs(
    conn: &Connection,
    filters: WorklogListFilters,
    max_list_limit: usize,
) -> Result<Vec<WorklogItem>, ToolError> {
    let mut sql = String::from(
        "SELECT DISTINCT w.log_id, w.log_type, w.status, w.agent_id, w.owner_session_id, w.related_agent_id, w.run_id,
            w.step_id, w.task_id, w.summary, w.payload_json, w.tags_json,
            w.timestamp, w.duration_ms, w.created_at, w.updated_at
        FROM worklogs w",
    );

    let mut params_vec: Vec<SqlValue> = Vec::new();
    if filters.tag.is_some() {
        sql.push_str(" INNER JOIN worklog_tags wt ON wt.log_id = w.log_id");
    }

    sql.push_str(" WHERE 1 = 1");

    if let Some(v) = filters.log_type {
        sql.push_str(" AND w.log_type = ?");
        params_vec.push(SqlValue::Text(v));
    }
    if let Some(v) = filters.status {
        sql.push_str(" AND w.status = ?");
        params_vec.push(SqlValue::Text(v));
    }
    if let Some(v) = filters.agent_id {
        sql.push_str(" AND w.agent_id = ?");
        params_vec.push(SqlValue::Text(v));
    }
    if let Some(v) = filters.owner_session_id {
        sql.push_str(" AND w.owner_session_id = ?");
        params_vec.push(SqlValue::Text(v));
    }
    if let Some(v) = filters.related_agent_id {
        sql.push_str(" AND w.related_agent_id = ?");
        params_vec.push(SqlValue::Text(v));
    }
    if let Some(v) = filters.run_id {
        sql.push_str(" AND w.run_id = ?");
        params_vec.push(SqlValue::Text(v));
    }
    if let Some(v) = filters.step_id {
        sql.push_str(" AND w.step_id = ?");
        params_vec.push(SqlValue::Text(v));
    }
    if let Some(v) = filters.task_id {
        sql.push_str(" AND w.task_id = ?");
        params_vec.push(SqlValue::Text(v));
    }
    if let Some(v) = filters.tag {
        sql.push_str(" AND wt.tag = ?");
        params_vec.push(SqlValue::Text(v));
    }
    if let Some(v) = filters.query {
        let pattern = format!("%{v}%");
        sql.push_str(" AND (w.summary LIKE ? OR w.payload_json LIKE ?)");
        params_vec.push(SqlValue::Text(pattern.clone()));
        params_vec.push(SqlValue::Text(pattern));
    }
    if let Some(v) = filters.from_ts {
        sql.push_str(" AND w.timestamp >= ?");
        params_vec.push(SqlValue::Integer(u64_to_i64(v)));
    }
    if let Some(v) = filters.to_ts {
        sql.push_str(" AND w.timestamp <= ?");
        params_vec.push(SqlValue::Integer(u64_to_i64(v)));
    }

    let limit = filters.limit.clamp(1, max_list_limit);
    sql.push_str(if filters.asc {
        " ORDER BY w.timestamp ASC, w.created_at ASC"
    } else {
        " ORDER BY w.timestamp DESC, w.created_at DESC"
    });
    sql.push_str(" LIMIT ? OFFSET ?");
    params_vec.push(SqlValue::Integer(usize_to_i64(limit, "limit")?));
    params_vec.push(SqlValue::Integer(usize_to_i64(filters.offset, "offset")?));

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|err| ToolError::ExecFailed(format!("prepare list worklogs failed: {err}")))?;
    let rows = stmt
        .query_map(params_from_iter(params_vec), map_worklog_row)
        .map_err(|err| ToolError::ExecFailed(format!("query list worklogs failed: {err}")))?;

    let mut out = Vec::new();
    for row in rows {
        out.push(
            row.map_err(|err| ToolError::ExecFailed(format!("decode worklog row failed: {err}")))?,
        );
    }
    Ok(out)
}

fn get_worklog_by_id(conn: &Connection, log_id: &str) -> Result<Option<WorklogItem>, ToolError> {
    let mut stmt = conn
        .prepare(
            "SELECT log_id, log_type, status, agent_id, owner_session_id, related_agent_id, run_id,
                step_id, task_id, summary, payload_json, tags_json,
                timestamp, duration_ms, created_at, updated_at
            FROM worklogs
            WHERE log_id = ?1
            LIMIT 1",
        )
        .map_err(|err| ToolError::ExecFailed(format!("prepare get worklog failed: {err}")))?;

    let mut rows = stmt
        .query(params![log_id])
        .map_err(|err| ToolError::ExecFailed(format!("query get worklog failed: {err}")))?;

    if let Some(row) = rows
        .next()
        .map_err(|err| ToolError::ExecFailed(format!("read get worklog row failed: {err}")))?
    {
        return map_worklog_row(row)
            .map(Some)
            .map_err(|err| ToolError::ExecFailed(format!("decode worklog row failed: {err}")));
    }
    Ok(None)
}

fn delete_worklog(conn: &Connection, log_id: &str) -> Result<bool, ToolError> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|err| ToolError::ExecFailed(format!("begin transaction failed: {err}")))?;

    tx.execute(
        "DELETE FROM worklog_tags WHERE log_id = ?1",
        params![log_id],
    )
    .map_err(|err| ToolError::ExecFailed(format!("delete worklog tags failed: {err}")))?;

    let changed = tx
        .execute("DELETE FROM worklogs WHERE log_id = ?1", params![log_id])
        .map_err(|err| ToolError::ExecFailed(format!("delete worklog failed: {err}")))?;

    tx.commit()
        .map_err(|err| ToolError::ExecFailed(format!("commit transaction failed: {err}")))?;

    Ok(changed > 0)
}

fn map_worklog_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorklogItem> {
    let payload_json: String = row.get(10)?;
    let payload = serde_json::from_str::<Json>(&payload_json).unwrap_or_else(|_| json!({}));
    let tags_json: String = row.get(11)?;
    let tags = serde_json::from_str::<Vec<String>>(&tags_json).unwrap_or_default();

    Ok(WorklogItem {
        log_id: row.get(0)?,
        log_type: row.get(1)?,
        status: row.get(2)?,
        agent_id: row.get(3)?,
        owner_session_id: row.get(4)?,
        related_agent_id: row.get(5)?,
        run_id: row.get(6)?,
        step_id: row.get(7)?,
        task_id: row.get(8)?,
        summary: row.get(9)?,
        payload,
        tags,
        timestamp: row.get::<_, i64>(12).map_or(0, |v| v.max(0) as u64),
        duration: row.get::<_, Option<i64>>(13)?.and_then(i64_to_u64),
        created_at: row.get::<_, i64>(14).map_or(0, |v| v.max(0) as u64),
        updated_at: row.get::<_, i64>(15).map_or(0, |v| v.max(0) as u64),
    })
}

fn require_action(args: &Json) -> Result<String, ToolError> {
    require_string(args, "action")
}

fn require_string(args: &Json, key: &str) -> Result<String, ToolError> {
    let value = args
        .get(key)
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_string())
        .ok_or_else(|| ToolError::InvalidArgs(format!("missing or invalid `{key}`")))?;
    if value.is_empty() {
        return Err(ToolError::InvalidArgs(format!("`{key}` cannot be empty")));
    }
    Ok(value)
}

fn optional_string(args: &Json, key: &str) -> Result<Option<String>, ToolError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let raw = value
        .as_str()
        .ok_or_else(|| ToolError::InvalidArgs(format!("`{key}` must be a string")))?;
    Ok(Some(raw.trim().to_string()).filter(|v| !v.is_empty()))
}

fn optional_bool(args: &Json, key: &str) -> Result<Option<bool>, ToolError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    value
        .as_bool()
        .map(Some)
        .ok_or_else(|| ToolError::InvalidArgs(format!("`{key}` must be a boolean")))
}

fn optional_u64(args: &Json, key: &str) -> Result<Option<u64>, ToolError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    value
        .as_u64()
        .map(Some)
        .ok_or_else(|| ToolError::InvalidArgs(format!("`{key}` must be a positive integer")))
}

fn parse_payload(value: Option<&Json>) -> Result<Json, ToolError> {
    let Some(value) = value else {
        return Ok(Json::Object(serde_json::Map::new()));
    };
    if value.is_null() {
        return Ok(Json::Object(serde_json::Map::new()));
    }
    if !value.is_object() {
        return Err(ToolError::InvalidArgs(
            "`payload` must be a json object".to_string(),
        ));
    }
    Ok(value.clone())
}

fn parse_tags(value: Option<&Json>) -> Result<Vec<String>, ToolError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let arr = value
        .as_array()
        .ok_or_else(|| ToolError::InvalidArgs("`tags` must be an array of strings".to_string()))?;
    if arr.len() > MAX_TAGS {
        return Err(ToolError::InvalidArgs(format!(
            "`tags` length exceeds max {}",
            MAX_TAGS
        )));
    }

    let mut out = Vec::new();
    for item in arr {
        let tag = item
            .as_str()
            .ok_or_else(|| {
                ToolError::InvalidArgs("`tags` must be an array of strings".to_string())
            })?
            .trim()
            .to_string();
        if tag.is_empty() {
            continue;
        }
        if tag.chars().count() > MAX_TAG_LEN {
            return Err(ToolError::InvalidArgs(format!(
                "tag `{}` exceeds max length {}",
                tag, MAX_TAG_LEN
            )));
        }
        if !out.contains(&tag) {
            out.push(tag);
        }
    }
    Ok(out)
}

fn normalize_status(raw: String) -> Result<String, ToolError> {
    let status = normalize_enum(&raw);
    let allowed = ["info", "success", "failed", "partial"];
    if allowed.contains(&status.as_str()) {
        return Ok(status);
    }
    Err(ToolError::InvalidArgs(format!(
        "invalid worklog status `{raw}`; allowed: {}",
        allowed.join(", ")
    )))
}

fn normalize_enum(raw: &str) -> String {
    raw.trim()
        .to_lowercase()
        .replace([' ', '-'], "_")
        .to_string()
}

fn validate_summary(summary: &str) -> Result<(), ToolError> {
    if summary.trim().is_empty() {
        return Err(ToolError::InvalidArgs(
            "`summary` cannot be empty".to_string(),
        ));
    }
    if summary.chars().count() > MAX_SUMMARY_LEN {
        return Err(ToolError::InvalidArgs(format!(
            "`summary` length exceeds max {}",
            MAX_SUMMARY_LEN
        )));
    }
    Ok(())
}

fn validate_text_field(field: &str, value: &str, max_len: usize) -> Result<(), ToolError> {
    if value.trim().is_empty() {
        return Err(ToolError::InvalidArgs(format!("`{field}` cannot be empty")));
    }
    if value.chars().count() > max_len {
        return Err(ToolError::InvalidArgs(format!(
            "`{field}` length exceeds max {max_len}"
        )));
    }
    Ok(())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn generate_worklog_id() -> String {
    let counter = WORKLOG_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("log-{}-{counter}", now_ms())
}

fn usize_to_i64(v: usize, name: &str) -> Result<i64, ToolError> {
    i64::try_from(v).map_err(|_| ToolError::InvalidArgs(format!("`{name}` too large")))
}

fn u64_to_usize(v: u64, name: &str) -> Result<usize, ToolError> {
    usize::try_from(v).map_err(|_| ToolError::InvalidArgs(format!("`{name}` too large")))
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    fn test_ctx() -> TraceCtx {
        TraceCtx {
            trace_id: "trace-test".to_string(),
            agent_did: "did:example:agent".to_string(),
            behavior: "on_wakeup".to_string(),
            step_idx: 0,
            wakeup_id: "wakeup-test".to_string(),
        }
    }

    async fn call(tool: &WorklogTool, args: Json) -> Result<Json, ToolError> {
        tool.call(&test_ctx(), args).await
    }

    async fn call_with_ctx(
        tool: &WorklogTool,
        ctx: &TraceCtx,
        args: Json,
    ) -> Result<Json, ToolError> {
        tool.call(ctx, args).await
    }

    #[tokio::test]
    async fn worklog_tool_append_get_list_and_delete_flow_works() {
        let tmp = tempdir().expect("create tempdir");
        let db_path = tmp.path().join("worklog").join("worklog.db");
        let tool =
            WorklogTool::new(WorklogToolConfig::with_db_path(db_path)).expect("create worklog");

        let first = call(
            &tool,
            json!({
                "action": "append",
                "log_id": "log-1",
                "type": "function_call",
                "status": "success",
                "step_id": "step-001",
                "summary": "Tool execution succeeded",
                "payload": {"tool": "exec_bash", "ok": true},
                "tags": ["tool", "runtime"]
            }),
        )
        .await
        .expect("append first log");
        assert_eq!(first["log"]["log_id"], "log-1");
        assert_eq!(first["log"]["agent_id"], "did:example:agent");
        assert!(first["log"]["owner_session_id"].is_null());

        call(
            &tool,
            json!({
                "action": "append",
                "log_id": "log-2",
                "type": "message_sent",
                "status": "info",
                "summary": "Message sent to sub-agent",
                "payload": {"to": "did:example:web-agent"},
                "tags": ["message"]
            }),
        )
        .await
        .expect("append second log");

        let by_tag = call(
            &tool,
            json!({
                "action": "list",
                "tag": "runtime"
            }),
        )
        .await
        .expect("list by tag");
        let tagged = by_tag["logs"].as_array().expect("logs array");
        assert_eq!(tagged.len(), 1);
        assert_eq!(tagged[0]["log_id"], "log-1");

        let got = call(
            &tool,
            json!({
                "action": "get",
                "log_id": "log-1"
            }),
        )
        .await
        .expect("get worklog");
        assert_eq!(got["log"]["summary"], "Tool execution succeeded");

        let deleted = call(
            &tool,
            json!({
                "action": "delete",
                "log_id": "log-1"
            }),
        )
        .await
        .expect("delete worklog");
        assert_eq!(deleted["deleted"], true);

        let remain = call(
            &tool,
            json!({
                "action": "list"
            }),
        )
        .await
        .expect("list after delete");
        assert_eq!(remain["total"], 1);
    }

    #[tokio::test]
    async fn worklog_tool_validates_invalid_args() {
        let tmp = tempdir().expect("create tempdir");
        let db_path = tmp.path().join("worklog").join("worklog.db");
        let tool =
            WorklogTool::new(WorklogToolConfig::with_db_path(db_path)).expect("create worklog");

        let err = call(
            &tool,
            json!({
                "action": "append",
                "type": "action",
                "status": "bad_status",
                "summary": "invalid"
            }),
        )
        .await
        .expect_err("invalid status should fail");
        assert!(matches!(err, ToolError::InvalidArgs(_)));

        let mut too_many_tags = Vec::new();
        for idx in 0..(MAX_TAGS + 1) {
            too_many_tags.push(format!("tag-{idx}"));
        }

        let err = call(
            &tool,
            json!({
                "action": "append",
                "type": "action",
                "summary": "too many tags",
                "tags": too_many_tags
            }),
        )
        .await
        .expect_err("too many tags should fail");
        assert!(matches!(err, ToolError::InvalidArgs(_)));

        let err = call(
            &tool,
            json!({
                "action": "list",
                "limit": "abc"
            }),
        )
        .await
        .expect_err("invalid limit should fail");
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn worklog_tool_can_save_and_filter_owner_session_id() {
        let tmp = tempdir().expect("create tempdir");
        let db_path = tmp.path().join("worklog").join("worklog.db");
        let tool =
            WorklogTool::new(WorklogToolConfig::with_db_path(db_path)).expect("create worklog");

        let created = call_with_ctx(
            &tool,
            &test_ctx(),
            json!({
                "action": "append",
                "log_id": "log-owner-1",
                "type": "function_call",
                "summary": "owner session log",
                "owner_session_id": "session-001"
            }),
        )
        .await
        .expect("append with owner session");
        assert_eq!(created["log"]["owner_session_id"], "session-001");

        let filtered = call(
            &tool,
            json!({
                "action": "list",
                "owner_session_id": "session-001"
            }),
        )
        .await
        .expect("list by owner session id");
        let logs = filtered["logs"].as_array().expect("logs array");
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0]["log_id"], "log-owner-1");
    }
}
