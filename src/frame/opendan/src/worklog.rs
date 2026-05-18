use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use chrono::{DateTime, Utc};
use log::info;
use rusqlite::{params, params_from_iter, types::Value as SqlValue, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use tokio::task;

use ::agent_tool::{
    now_ms, optional_trimmed_string_arg as optional_string, optional_u64_arg as optional_u64,
    u64_to_usize_arg as u64_to_usize, AgentToolError, AgentToolResult,
};

/// Per-append context carried by `OpenDanWorklogSink` and other producers.
/// Replaces the old `SessionRuntimeContext` dependency now that the AgentTool
/// wrapper layer has moved out of this module.
#[derive(Clone, Debug, Default)]
pub struct WorklogAppendCtx {
    pub trace_id: String,
    pub agent_name: String,
    pub behavior: String,
    pub session_id: String,
}

const DEFAULT_LIST_LIMIT: usize = 64;
const DEFAULT_MAX_LIST_LIMIT: usize = 256;

static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Open string-based event type. Common values include `GetMessage`,
/// `ReplyMessage`, `FunctionRecord`, `ActionRecord`, `CreateSubAgent`, etc.,
/// but new event names can be introduced without code changes.
pub type WorklogRecordType = String;

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
pub struct WorklogService {
    cfg: WorklogToolConfig,
}

impl WorklogService {
    pub fn new(mut cfg: WorklogToolConfig) -> Result<Self, AgentToolError> {
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
                    "opendan.persist_entity_prepare: kind=worklog_db_parent_dir path={}",
                    parent.display()
                );
            }
            std::fs::create_dir_all(parent).map_err(|err| {
                AgentToolError::ExecFailed(format!(
                    "create worklog db parent dir `{}` failed: {err}",
                    parent.display()
                ))
            })?;
        }
        if !cfg.db_path.exists() {
            info!(
                "opendan.persist_entity_prepare: kind=worklog_db_file path={}",
                cfg.db_path.display()
            );
        }

        let conn = Connection::open(&cfg.db_path).map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "open worklog db `{}` failed: {err}",
                cfg.db_path.display()
            ))
        })?;
        ensure_worklog_schema(&conn)?;

        Ok(Self { cfg })
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
                    "open worklog db `{}` failed: {err}",
                    db_path.display()
                ))
            })?;
            ensure_worklog_schema(&conn)?;
            op(&mut conn)
        })
        .await
        .map_err(|err| AgentToolError::ExecFailed(format!("{op_name} join error: {err}")))?
    }

    pub async fn list_worklog_records(
        &self,
        options: WorklogListOptions,
    ) -> Result<Vec<WorklogRecord>, AgentToolError> {
        Ok(self.list_worklog_page(options).await?.records)
    }

    pub async fn list_worklog_page(
        &self,
        options: WorklogListOptions,
    ) -> Result<WorklogListPage, AgentToolError> {
        let filters = options.into_filters(self.cfg.default_list_limit, self.cfg.max_list_limit);
        let listed = self
            .run_db("worklog list records", move |conn| {
                list_records(conn, &filters)
            })
            .await?;
        Ok(WorklogListPage {
            records: listed.records,
            total: listed.total,
        })
    }

    pub async fn append_record_for_session(
        &self,
        session_id: &str,
        agent_name: &str,
        behavior: &str,
        step_idx: u32,
        record: Json,
    ) -> Result<WorklogRecord, AgentToolError> {
        let sid = session_id.trim();
        if sid.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "session_id cannot be empty".to_string(),
            ));
        }
        let ctx = WorklogAppendCtx {
            trace_id: "session-worklog".to_string(),
            agent_name: agent_name.trim().to_string(),
            behavior: behavior.trim().to_string(),
            session_id: sid.to_string(),
        };
        let _ = step_idx; // step_idx is carried via the record payload's step_id
        let args = json!({
            "record": record,
            "session_id": sid,
        });
        let input = AppendRecordInput::parse(&ctx, &args)?;
        self.run_db("worklog append", move |conn| insert_record(conn, input))
            .await
    }

    /// Direct append entry used by `OpenDanWorklogSink`. Bypasses the legacy
    /// AgentTool action dispatch in favor of a typed context + raw record JSON.
    pub async fn append_record(
        &self,
        ctx: &WorklogAppendCtx,
        record: Json,
    ) -> Result<WorklogRecord, AgentToolError> {
        let args = json!({ "record": record });
        let input = AppendRecordInput::parse(ctx, &args)?;
        self.run_db("worklog append", move |conn| insert_record(conn, input))
            .await
    }
}

/// Producer-side payload schema for `ActionRecord` events. Values are stored
/// verbatim — no truncation or digesting — so that worklog can serve as a
/// full audit trail.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct WorklogActionPayload {
    #[serde(default)]
    pub action_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exec_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_result: Option<AgentToolResult>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorklogRecord {
    pub id: String,
    pub ts: String,
    pub timestamp: u64,
    pub seq: u64,
    #[serde(rename = "type")]
    pub event_type: WorklogRecordType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_did: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behavior: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_index: Option<u32>,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default)]
    pub payload: Json,
}

/// Backwards-compatible alias used by older call sites that read
/// `record.record_type`. Prefer `event_type` directly.
impl WorklogRecord {
    pub fn record_type(&self) -> &str {
        self.event_type.as_str()
    }
}

#[derive(Clone, Debug)]
struct AppendRecordInput {
    now_ms: u64,
    ts: String,
    timestamp: u64,
    event_type: WorklogRecordType,
    agent_did: Option<String>,
    session_id: Option<String>,
    workspace_id: Option<String>,
    behavior: Option<String>,
    step_id: Option<String>,
    step_index: Option<u32>,
    status: String,
    trace_id: Option<String>,
    task_id: Option<String>,
    payload: Json,
}

impl AppendRecordInput {
    fn parse(ctx: &WorklogAppendCtx, args: &Json) -> Result<Self, AgentToolError> {
        let raw = args.get("record").unwrap_or(args);
        let map = raw.as_object().ok_or_else(|| {
            AgentToolError::InvalidArgs("`record` must be a json object".to_string())
        })?;

        let now_ms = now_ms();
        let timestamp = map
            .get("timestamp")
            .and_then(|v| v.as_u64())
            .or_else(|| {
                map.get("ts")
                    .and_then(|v| v.as_str())
                    .and_then(parse_rfc3339_to_ms)
            })
            .or_else(|| args.get("timestamp").and_then(|v| v.as_u64()))
            .unwrap_or(now_ms);
        let ts = map
            .get("ts")
            .and_then(|v| v.as_str())
            .map(|v| v.to_string())
            .unwrap_or_else(|| to_rfc3339(timestamp));

        let event_type_raw = map
            .get("type")
            .and_then(|v| v.as_str())
            .or_else(|| map.get("log_type").and_then(|v| v.as_str()))
            .or_else(|| args.get("type").and_then(|v| v.as_str()))
            .or_else(|| args.get("log_type").and_then(|v| v.as_str()))
            .ok_or_else(|| AgentToolError::InvalidArgs("missing `type`".to_string()))?;
        let event_type = normalize_event_type(event_type_raw)?;

        let session_id = map
            .get("session_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string())
            .or_else(|| {
                map.get("owner_session_id")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(|v| v.to_string())
            })
            .or_else(|| optional_string(args, "session_id").ok().flatten())
            .or_else(|| optional_string(args, "owner_session_id").ok().flatten());

        let workspace_id = map
            .get("workspace_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string())
            .or_else(|| optional_string(args, "workspace_id").ok().flatten());

        let agent_did = map
            .get("agent_did")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string())
            .or_else(|| {
                map.get("agent_id")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(|v| v.to_string())
            })
            .or_else(|| {
                let v = ctx.agent_name.trim();
                if v.is_empty() {
                    None
                } else {
                    Some(v.to_string())
                }
            });

        let behavior = map
            .get("behavior")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string())
            .or_else(|| {
                let v = ctx.behavior.trim();
                if v.is_empty() {
                    None
                } else {
                    Some(v.to_string())
                }
            });

        let step_id = map
            .get("step_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string())
            .or_else(|| optional_string(args, "step_id").ok().flatten());
        let step_index = map
            .get("step_index")
            .and_then(|v| v.as_u64())
            .and_then(u64_to_u32)
            .or_else(|| step_id.as_deref().and_then(parse_step_index_from_id));

        let status = normalize_status(
            map.get("status")
                .and_then(|v| v.as_str())
                .or_else(|| args.get("status").and_then(|v| v.as_str()))
                .unwrap_or("OK"),
        );

        let trace_id = map
            .get("trace")
            .and_then(|v| v.get("taskmgr_id"))
            .and_then(|v| v.as_str())
            .map(|v| v.to_string())
            .or_else(|| {
                map.get("trace_id")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(|v| v.to_string())
            })
            .or_else(|| {
                let v = ctx.trace_id.trim();
                if v.is_empty() {
                    None
                } else {
                    Some(v.to_string())
                }
            });
        let task_id = map
            .get("task_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string())
            .or_else(|| optional_string(args, "task_id").ok().flatten());

        let payload = map
            .get("payload")
            .cloned()
            .or_else(|| args.get("payload").cloned())
            .unwrap_or_else(|| Json::Object(serde_json::Map::new()));

        Ok(Self {
            now_ms,
            ts,
            timestamp,
            event_type,
            agent_did,
            session_id,
            workspace_id,
            behavior,
            step_id,
            step_index,
            status,
            trace_id,
            task_id,
            payload,
        })
    }
}

#[derive(Clone, Debug)]
struct ListFilters {
    owner_session_id: Option<String>,
    workspace_id: Option<String>,
    step_id: Option<String>,
    event_type: Option<String>,
    status: Option<String>,
    keyword: Option<String>,
    limit: usize,
    offset: usize,
}

impl ListFilters {
    #[allow(dead_code)] // retained for upcoming ai_runtime list-API wiring
    fn parse(args: &Json, default_limit: usize, max_limit: usize) -> Result<Self, AgentToolError> {
        let owner_session_id = optional_string(args, "owner_session_id")?;
        let workspace_id = optional_string(args, "workspace_id")?;
        let step_id = optional_string(args, "step_id")?;
        let event_type = optional_string(args, "type")?
            .map(|v| normalize_event_type(&v))
            .transpose()?;
        let status = optional_string(args, "status")?.map(|v| normalize_status(&v));
        let keyword = optional_string(args, "keyword")?;

        let limit = optional_u64(args, "limit")?
            .map(|v| u64_to_usize(v, "limit"))
            .transpose()?
            .unwrap_or(default_limit)
            .clamp(1, max_limit);
        let offset = optional_u64(args, "offset")?
            .map(|v| u64_to_usize(v, "offset"))
            .transpose()?
            .unwrap_or(0);
        Ok(Self {
            owner_session_id,
            workspace_id,
            step_id,
            event_type,
            status,
            keyword,
            limit,
            offset,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct WorklogListOptions {
    pub owner_session_id: Option<String>,
    pub workspace_id: Option<String>,
    pub step_id: Option<String>,
    pub event_type: Option<String>,
    pub status: Option<String>,
    pub keyword: Option<String>,
    pub limit: Option<usize>,
    pub offset: usize,
}

impl WorklogListOptions {
    fn into_filters(self, default_limit: usize, max_limit: usize) -> ListFilters {
        let limit = self.limit.unwrap_or(default_limit).clamp(1, max_limit);
        ListFilters {
            owner_session_id: self
                .owner_session_id
                .and_then(|v| optional_non_empty(v.as_str())),
            workspace_id: self
                .workspace_id
                .and_then(|v| optional_non_empty(v.as_str())),
            step_id: self.step_id.and_then(|v| optional_non_empty(v.as_str())),
            event_type: self.event_type.and_then(|v| optional_non_empty(v.as_str())),
            status: self
                .status
                .and_then(|v| optional_non_empty(v.as_str()))
                .map(|v| normalize_status(v.as_str())),
            keyword: self.keyword.and_then(|v| optional_non_empty(v.as_str())),
            limit,
            offset: self.offset,
        }
    }
}

#[derive(Clone, Debug)]
pub struct WorklogListPage {
    pub records: Vec<WorklogRecord>,
    pub total: u64,
}

#[derive(Clone, Debug)]
struct ListResult {
    records: Vec<WorklogRecord>,
    total: u64,
}

fn ensure_worklog_schema(conn: &Connection) -> Result<(), AgentToolError> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS worklogs (
            log_id TEXT PRIMARY KEY,
            seq INTEGER NOT NULL,
            ts TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            event_type TEXT NOT NULL,
            agent_id TEXT,
            owner_session_id TEXT,
            workspace_id TEXT,
            behavior TEXT,
            step_id TEXT,
            step_index INTEGER,
            status TEXT NOT NULL,
            trace_id TEXT,
            task_id TEXT,
            record_json TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_worklogs_timestamp ON worklogs(timestamp DESC, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_worklogs_session ON worklogs(owner_session_id, timestamp DESC);
        CREATE INDEX IF NOT EXISTS idx_worklogs_workspace ON worklogs(workspace_id, timestamp DESC);
        CREATE INDEX IF NOT EXISTS idx_worklogs_step ON worklogs(step_id, timestamp DESC);
        CREATE INDEX IF NOT EXISTS idx_worklogs_type ON worklogs(event_type, timestamp DESC);
        CREATE INDEX IF NOT EXISTS idx_worklogs_status ON worklogs(status, timestamp DESC);
        "#,
    )
    .map_err(|err| AgentToolError::ExecFailed(format!("init worklog schema failed: {err}")))?;
    Ok(())
}

fn insert_record(
    conn: &mut Connection,
    input: AppendRecordInput,
) -> Result<WorklogRecord, AgentToolError> {
    let tx = conn
        .transaction()
        .map_err(|err| AgentToolError::ExecFailed(format!("start worklog tx failed: {err}")))?;
    let seq = next_session_seq(&tx, input.session_id.as_deref())?;
    let id = generate_worklog_id(input.timestamp);

    let record = WorklogRecord {
        id: id.clone(),
        ts: input.ts.clone(),
        timestamp: input.timestamp,
        seq,
        event_type: input.event_type.clone(),
        agent_did: input.agent_did.clone(),
        session_id: input.session_id.clone(),
        workspace_id: input.workspace_id.clone(),
        behavior: input.behavior.clone(),
        step_id: input.step_id.clone(),
        step_index: input.step_index,
        status: input.status.clone(),
        trace_id: input.trace_id.clone(),
        task_id: input.task_id.clone(),
        payload: input.payload.clone(),
    };

    let record_json = serde_json::to_string(&record)
        .map_err(|err| AgentToolError::ExecFailed(format!("serialize record failed: {err}")))?;

    tx.execute(
        r#"
        INSERT INTO worklogs (
            log_id, seq, ts, timestamp, event_type, agent_id,
            owner_session_id, workspace_id, behavior, step_id, step_index,
            status, trace_id, task_id, record_json, created_at
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6,
            ?7, ?8, ?9, ?10, ?11,
            ?12, ?13, ?14, ?15, ?16
        )
        "#,
        params![
            &record.id,
            record.seq as i64,
            &record.ts,
            record.timestamp as i64,
            record.event_type.as_str(),
            record.agent_did.as_deref(),
            record.session_id.as_deref(),
            record.workspace_id.as_deref(),
            record.behavior.as_deref(),
            record.step_id.as_deref(),
            record.step_index.map(|v| v as i64),
            &record.status,
            record.trace_id.as_deref(),
            record.task_id.as_deref(),
            &record_json,
            input.now_ms as i64,
        ],
    )
    .map_err(|err| AgentToolError::ExecFailed(format!("insert worklog failed: {err}")))?;

    tx.commit()
        .map_err(|err| AgentToolError::ExecFailed(format!("commit worklog tx failed: {err}")))?;

    Ok(record)
}

fn list_records(conn: &Connection, filters: &ListFilters) -> Result<ListResult, AgentToolError> {
    let mut where_sql = String::from(" WHERE 1=1");
    let mut where_params = Vec::<SqlValue>::new();

    if let Some(v) = filters.owner_session_id.as_deref() {
        where_sql.push_str(" AND owner_session_id = ?");
        where_params.push(SqlValue::Text(v.to_string()));
    }
    if let Some(v) = filters.workspace_id.as_deref() {
        where_sql.push_str(" AND workspace_id = ?");
        where_params.push(SqlValue::Text(v.to_string()));
    }
    if let Some(v) = filters.step_id.as_deref() {
        where_sql.push_str(" AND step_id = ?");
        where_params.push(SqlValue::Text(v.to_string()));
    }
    if let Some(v) = filters.event_type.as_deref() {
        where_sql.push_str(" AND event_type = ?");
        where_params.push(SqlValue::Text(v.to_string()));
    }
    if let Some(v) = filters.status.as_deref() {
        where_sql.push_str(" AND status = ?");
        where_params.push(SqlValue::Text(v.to_string()));
    }
    if let Some(v) = filters.keyword.as_deref() {
        let pattern = format!("%{v}%");
        where_sql.push_str(" AND record_json LIKE ?");
        where_params.push(SqlValue::Text(pattern));
    }

    let count_sql = format!("SELECT COUNT(1) FROM worklogs{}", where_sql);
    let mut count_stmt = conn.prepare(&count_sql).map_err(|err| {
        AgentToolError::ExecFailed(format!("prepare worklog count failed: {err}"))
    })?;
    let total = count_stmt
        .query_row(params_from_iter(where_params.clone()), |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|err| AgentToolError::ExecFailed(format!("query worklog count failed: {err}")))?
        .max(0) as u64;

    let mut list_sql = format!("SELECT record_json FROM worklogs{}", where_sql);
    list_sql.push_str(" ORDER BY timestamp DESC, created_at DESC LIMIT ? OFFSET ?");
    let mut list_params = where_params;
    list_params.push(SqlValue::Integer(filters.limit as i64));
    list_params.push(SqlValue::Integer(filters.offset as i64));

    let mut stmt = conn
        .prepare(&list_sql)
        .map_err(|err| AgentToolError::ExecFailed(format!("prepare worklog list failed: {err}")))?;
    let mut rows = stmt
        .query(params_from_iter(list_params))
        .map_err(|err| AgentToolError::ExecFailed(format!("query worklog list failed: {err}")))?;

    let mut records = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|err| AgentToolError::ExecFailed(format!("read worklog row failed: {err}")))?
    {
        let record_json: String = row.get(0).unwrap_or_else(|_| "{}".to_string());
        if let Ok(record) = serde_json::from_str::<WorklogRecord>(&record_json) {
            records.push(record);
        }
    }

    Ok(ListResult { records, total })
}

#[allow(dead_code)] // retained for upcoming ai_runtime get-by-id wiring
fn get_record(conn: &Connection, id: &str) -> Result<Option<WorklogRecord>, AgentToolError> {
    let mut stmt = conn
        .prepare("SELECT record_json FROM worklogs WHERE log_id = ? LIMIT 1")
        .map_err(|err| AgentToolError::ExecFailed(format!("prepare worklog get failed: {err}")))?;
    let value: Result<String, rusqlite::Error> = stmt.query_row(params![id], |row| row.get(0));
    match value {
        Ok(raw) => {
            let parsed = serde_json::from_str::<WorklogRecord>(&raw).map_err(|err| {
                AgentToolError::ExecFailed(format!("decode worklog `{id}` failed: {err}"))
            })?;
            Ok(Some(parsed))
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(err) => Err(AgentToolError::ExecFailed(format!(
            "query worklog `{id}` failed: {err}"
        ))),
    }
}

fn normalize_event_type(raw: &str) -> Result<String, AgentToolError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AgentToolError::InvalidArgs(
            "worklog `type` cannot be empty".to_string(),
        ));
    }
    Ok(trimmed.to_string())
}

fn normalize_status(raw: &str) -> String {
    match raw.trim().to_lowercase().as_str() {
        "ok" | "success" | "succeeded" | "done" | "info" => "OK".to_string(),
        "failed" | "fail" | "error" => "FAILED".to_string(),
        "pending" | "running" => "PENDING".to_string(),
        other => {
            if other.is_empty() {
                "OK".to_string()
            } else {
                other.to_uppercase()
            }
        }
    }
}

fn next_session_seq(conn: &Connection, session_id: Option<&str>) -> Result<u64, AgentToolError> {
    let seq = if let Some(session_id) = session_id.filter(|v| !v.trim().is_empty()) {
        conn.query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM worklogs WHERE owner_session_id = ?1",
            params![session_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|err| AgentToolError::ExecFailed(format!("query session seq failed: {err}")))?
    } else {
        conn.query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM worklogs",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|err| AgentToolError::ExecFailed(format!("query global seq failed: {err}")))?
    };
    Ok(seq.max(1) as u64)
}

fn generate_worklog_id(timestamp_ms: u64) -> String {
    let seq = ID_COUNTER.fetch_add(1, Ordering::Relaxed) + 1;
    format!("wlrec_{}_{}", timestamp_ms, seq)
}

fn to_rfc3339(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    let nanos = ((ms % 1000) * 1_000_000) as u32;
    let dt = DateTime::<Utc>::from_timestamp(secs, nanos).unwrap_or_else(Utc::now);
    dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn parse_rfc3339_to_ms(raw: &str) -> Option<u64> {
    let parsed = DateTime::parse_from_rfc3339(raw).ok()?;
    Some(parsed.timestamp_millis().max(0) as u64)
}

fn parse_step_index_from_id(step_id: &str) -> Option<u32> {
    let digits = step_id
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u32>().ok()
}

fn optional_non_empty(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

fn u64_to_u32(value: u64) -> Option<u32> {
    u32::try_from(value).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn test_ctx(session: &str) -> WorklogAppendCtx {
        WorklogAppendCtx {
            trace_id: format!("trace-{session}"),
            agent_name: "did:opendan:test".to_string(),
            behavior: "DO".to_string(),
            session_id: session.to_string(),
        }
    }

    #[tokio::test]
    async fn worklog_service_supports_append_and_list() {
        let dir = tempdir().expect("temp dir");
        let db = dir.path().join("worklog.db");
        let svc = WorklogService::new(WorklogToolConfig::with_db_path(db)).expect("create svc");
        let ctx = test_ctx("sess-1");

        let inserted = svc
            .append_record(
                &ctx,
                json!({
                    "type": "FunctionRecord",
                    "owner_session_id": "sess-1",
                    "step_id": "step-1",
                    "status": "OK",
                    "payload": {
                        "tool_name": "todo_manage",
                        "result_text": "ok"
                    }
                }),
            )
            .await
            .expect("append");
        assert!(!inserted.id.is_empty());

        let page = svc
            .list_worklog_page(WorklogListOptions {
                owner_session_id: Some("sess-1".to_string()),
                ..Default::default()
            })
            .await
            .expect("list");
        assert_eq!(page.total, 1);
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].event_type, "FunctionRecord");
    }

    #[tokio::test]
    async fn worklog_open_event_type_accepts_custom_names() {
        let dir = tempdir().expect("temp dir");
        let db = dir.path().join("worklog.db");
        let svc = WorklogService::new(WorklogToolConfig::with_db_path(db)).expect("create svc");
        let ctx = test_ctx("sess-2");

        let _ = svc
            .append_record(
                &ctx,
                json!({
                    "type": "agent.file.write",
                    "owner_session_id": "sess-2",
                    "status": "OK",
                    "payload": { "path": "/tmp/x" }
                }),
            )
            .await
            .expect("append");

        let records = svc
            .list_worklog_records(WorklogListOptions {
                owner_session_id: Some("sess-2".to_string()),
                ..Default::default()
            })
            .await
            .expect("list");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].event_type, "agent.file.write");
    }
}
