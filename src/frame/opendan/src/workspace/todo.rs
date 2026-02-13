use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use rusqlite::{params, params_from_iter, types::Value as SqlValue, Connection};
use serde::Serialize;
use serde_json::{json, Value as Json};
use tokio::task;

use crate::agent_tool::{AgentTool, ToolCallContext, ToolError, ToolSpec};

pub const TOOL_TODO_MANAGE: &str = "todo_manage";

const DEFAULT_LIST_LIMIT: usize = 32;
const DEFAULT_MAX_LIST_LIMIT: usize = 128;
const MAX_TITLE_LEN: usize = 256;
const MAX_DESCRIPTION_LEN: usize = 4096;
const MAX_TAGS: usize = 16;

static TODO_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

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

#[derive(Clone, Debug)]
pub struct TodoTool {
    cfg: TodoToolConfig,
}

impl TodoTool {
    pub fn new(mut cfg: TodoToolConfig) -> Result<Self, ToolError> {
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
                    "create todo db parent dir `{}` failed: {err}",
                    parent.display()
                ))
            })?;
        }

        let conn = Connection::open(&cfg.db_path).map_err(|err| {
            ToolError::ExecFailed(format!(
                "open todo db `{}` failed: {err}",
                cfg.db_path.display()
            ))
        })?;
        ensure_todo_schema(&conn)?;

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
                    "open todo db `{}` failed: {err}",
                    db_path.display()
                ))
            })?;
            ensure_todo_schema(&conn)?;
            op(&conn)
        })
        .await
        .map_err(|err| ToolError::ExecFailed(format!("{op_name} join error: {err}")))?
    }
}

#[async_trait]
impl AgentTool for TodoTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_TODO_MANAGE.to_string(),
            description: "Manage workspace todo items backed by todo/todo.db.".to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["create", "list", "get", "update", "delete"]
                    },
                    "id": { "type": "string" },
                    "title": { "type": "string" },
                    "description": { "type": "string" },
                    "status": { "type": "string", "enum": ["todo","in_progress","blocked","done","cancelled"] },
                    "priority": { "type": "string", "enum": ["low","normal","high","urgent"] },
                    "tags": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "due_at": { "type": "integer", "minimum": 1 },
                    "clear_due_at": { "type": "boolean", "default": false },
                    "task_id": { "type": "integer" },
                    "task_status": { "type": "string" },
                    "clear_task_id": { "type": "boolean", "default": false },
                    "include_closed": { "type": "boolean", "default": true },
                    "query": { "type": "string" },
                    "limit": { "type": "integer", "minimum": 1 },
                    "offset": { "type": "integer", "minimum": 0 }
                },
                "required": ["action"],
                "additionalProperties": true
            }),
            output_schema: json!({
                "type": "object",
                "properties": {
                    "ok": { "type": "boolean" },
                    "action": { "type": "string" },
                    "todo": { "type": "object" },
                    "todos": { "type": "array", "items": { "type": "object" } },
                    "deleted": { "type": "boolean" },
                    "total": { "type": "integer" }
                }
            }),
        }
    }

    async fn call(&self, _ctx: &ToolCallContext, args: Json) -> Result<Json, ToolError> {
        let action = require_action(&args)?;
        match action.as_str() {
            "create" => self.call_create(args).await,
            "list" => self.call_list(args).await,
            "get" => self.call_get(args).await,
            "update" => self.call_update(args).await,
            "delete" => self.call_delete(args).await,
            _ => Err(ToolError::InvalidArgs(format!(
                "unsupported action `{action}`, expected create/list/get/update/delete"
            ))),
        }
    }
}

impl TodoTool {
    async fn call_create(&self, args: Json) -> Result<Json, ToolError> {
        let input = TodoCreateInput::from_args(&args)?;
        let item = self
            .run_db("create todo", move |conn| create_todo(conn, input))
            .await?;
        Ok(json!({
            "ok": true,
            "action": "create",
            "todo": item
        }))
    }

    async fn call_list(&self, args: Json) -> Result<Json, ToolError> {
        let status = optional_string(&args, "status")?
            .map(normalize_status)
            .transpose()?;
        let include_closed = optional_bool(&args, "include_closed")?.unwrap_or(true);
        let query = optional_string(&args, "query")?.map(|s| s.trim().to_string());
        let limit = optional_u64(&args, "limit")?
            .map(|v| u64_to_usize(v, "limit"))
            .transpose()?
            .unwrap_or(self.cfg.default_list_limit);
        let offset = optional_u64(&args, "offset")?
            .map(|v| u64_to_usize(v, "offset"))
            .transpose()?
            .unwrap_or(0);
        let limit = limit.clamp(1, self.cfg.max_list_limit);
        let query = query.filter(|v| !v.is_empty());

        let rows = self
            .run_db("list todo", move |conn| {
                list_todos(
                    conn,
                    status.as_deref(),
                    include_closed,
                    query.as_deref(),
                    limit,
                    offset,
                )
            })
            .await?;

        Ok(json!({
            "ok": true,
            "action": "list",
            "todos": rows,
            "total": rows.len()
        }))
    }

    async fn call_get(&self, args: Json) -> Result<Json, ToolError> {
        let id = require_string(&args, "id")?;
        let lookup_id = id.clone();
        let item = self
            .run_db("get todo", move |conn| get_todo_by_id(conn, &lookup_id))
            .await?;
        let Some(item) = item else {
            return Err(ToolError::InvalidArgs(format!("todo `{id}` not found")));
        };
        Ok(json!({
            "ok": true,
            "action": "get",
            "todo": item
        }))
    }

    async fn call_update(&self, args: Json) -> Result<Json, ToolError> {
        let id = require_string(&args, "id")?;
        let patch = TodoPatch::from_args(&args)?;
        let item = self
            .run_db("update todo", move |conn| update_todo(conn, &id, patch))
            .await?;
        Ok(json!({
            "ok": true,
            "action": "update",
            "todo": item
        }))
    }

    async fn call_delete(&self, args: Json) -> Result<Json, ToolError> {
        let id = require_string(&args, "id")?;
        let deleted = self
            .run_db("delete todo", move |conn| delete_todo(conn, &id))
            .await?;
        Ok(json!({
            "ok": true,
            "action": "delete",
            "deleted": deleted
        }))
    }
}

#[derive(Clone, Debug)]
struct TodoCreateInput {
    id: String,
    title: String,
    description: String,
    status: String,
    priority: String,
    tags: Vec<String>,
    due_at: Option<u64>,
    task_id: Option<i64>,
    task_status: Option<String>,
}

impl TodoCreateInput {
    fn from_args(args: &Json) -> Result<Self, ToolError> {
        let id = optional_string(args, "id")?.unwrap_or_else(generate_todo_id);
        let title = require_string(args, "title")?;
        let description = optional_string(args, "description")?.unwrap_or_default();
        let status = optional_string(args, "status")?
            .map(normalize_status)
            .transpose()?
            .unwrap_or_else(|| "todo".to_string());
        let priority = optional_string(args, "priority")?
            .map(normalize_priority)
            .transpose()?
            .unwrap_or_else(|| "normal".to_string());
        let tags = parse_tags(args.get("tags"))?;
        let due_at = optional_u64(args, "due_at")?;
        let task_id = optional_i64(args, "task_id")?;
        let task_status = optional_string(args, "task_status")?;

        validate_todo_text_fields(&id, &title, &description)?;
        Ok(Self {
            id,
            title,
            description,
            status,
            priority,
            tags,
            due_at,
            task_id,
            task_status,
        })
    }
}

#[derive(Clone, Debug, Default)]
struct TodoPatch {
    title: Option<String>,
    description: Option<String>,
    status: Option<String>,
    priority: Option<String>,
    tags: Option<Vec<String>>,
    due_at: Option<Option<u64>>,
    task_id: Option<Option<i64>>,
    task_status: Option<Option<String>>,
}

impl TodoPatch {
    fn from_args(args: &Json) -> Result<Self, ToolError> {
        let title = optional_string(args, "title")?;
        let description = optional_string(args, "description")?;
        let status = optional_string(args, "status")?
            .map(normalize_status)
            .transpose()?;
        let priority = optional_string(args, "priority")?
            .map(normalize_priority)
            .transpose()?;
        let tags = if args.get("tags").is_some() {
            Some(parse_tags(args.get("tags"))?)
        } else {
            None
        };

        let clear_due_at = optional_bool(args, "clear_due_at")?.unwrap_or(false);
        let due_at = if clear_due_at {
            Some(None)
        } else if args.get("due_at").is_some() {
            Some(optional_u64(args, "due_at")?)
        } else {
            None
        };

        let clear_task_id = optional_bool(args, "clear_task_id")?.unwrap_or(false);
        let task_id = if clear_task_id {
            Some(None)
        } else if args.get("task_id").is_some() {
            Some(optional_i64(args, "task_id")?)
        } else {
            None
        };

        let task_status = if args.get("task_status").is_some() {
            Some(optional_string(args, "task_status")?)
        } else {
            None
        };

        if let Some(title) = &title {
            validate_todo_title(title)?;
        }
        if let Some(description) = &description {
            validate_todo_description(description)?;
        }

        let has_change = title.is_some()
            || description.is_some()
            || status.is_some()
            || priority.is_some()
            || tags.is_some()
            || due_at.is_some()
            || task_id.is_some()
            || task_status.is_some();
        if !has_change {
            return Err(ToolError::InvalidArgs(
                "update requires at least one mutable field".to_string(),
            ));
        }

        Ok(Self {
            title,
            description,
            status,
            priority,
            tags,
            due_at,
            task_id,
            task_status,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
struct TodoItem {
    id: String,
    title: String,
    description: String,
    status: String,
    priority: String,
    tags: Vec<String>,
    due_at: Option<u64>,
    task_id: Option<i64>,
    task_status: Option<String>,
    created_at: u64,
    updated_at: u64,
}

fn ensure_todo_schema(conn: &Connection) -> Result<(), ToolError> {
    conn.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS todos (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'todo',
    priority TEXT NOT NULL DEFAULT 'normal',
    tags_json TEXT NOT NULL DEFAULT '[]',
    due_at INTEGER,
    task_id INTEGER,
    task_status TEXT,
    created_at INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_todos_status_updated ON todos(status, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_todos_task_id ON todos(task_id);
"#,
    )
    .map_err(|err| ToolError::ExecFailed(format!("ensure todo schema failed: {err}")))
}

fn create_todo(conn: &Connection, input: TodoCreateInput) -> Result<TodoItem, ToolError> {
    let now = now_ms();
    let tags_json = serde_json::to_string(&input.tags)
        .map_err(|err| ToolError::ExecFailed(format!("serialize tags failed: {err}")))?;
    conn.execute(
        "INSERT INTO todos (
            id, title, description, status, priority, tags_json, due_at, task_id, task_status, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            input.id,
            input.title,
            input.description,
            input.status,
            input.priority,
            tags_json,
            input.due_at.map(u64_to_i64),
            input.task_id,
            input.task_status,
            u64_to_i64(now),
            u64_to_i64(now),
        ],
    )
    .map_err(|err| ToolError::ExecFailed(format!("insert todo failed: {err}")))?;

    let item = get_todo_by_id(conn, &input.id)?
        .ok_or_else(|| ToolError::ExecFailed("read created todo failed".to_string()))?;
    Ok(item)
}

fn list_todos(
    conn: &Connection,
    status: Option<&str>,
    include_closed: bool,
    query: Option<&str>,
    limit: usize,
    offset: usize,
) -> Result<Vec<TodoItem>, ToolError> {
    let mut sql = String::from(
        "SELECT id, title, description, status, priority, tags_json, due_at, task_id, task_status, created_at, updated_at
         FROM todos
         WHERE 1 = 1",
    );
    let mut params_vec: Vec<SqlValue> = Vec::new();

    if let Some(status) = status {
        sql.push_str(" AND status = ?");
        params_vec.push(SqlValue::Text(status.to_string()));
    }
    if !include_closed {
        sql.push_str(" AND status NOT IN ('done', 'cancelled')");
    }
    if let Some(keyword) = query {
        let pattern = format!("%{}%", keyword);
        sql.push_str(" AND (title LIKE ? OR description LIKE ?)");
        params_vec.push(SqlValue::Text(pattern.clone()));
        params_vec.push(SqlValue::Text(pattern));
    }

    sql.push_str(" ORDER BY updated_at DESC, created_at DESC LIMIT ? OFFSET ?");
    params_vec.push(SqlValue::Integer(usize_to_i64(limit, "limit")?));
    params_vec.push(SqlValue::Integer(usize_to_i64(offset, "offset")?));

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|err| ToolError::ExecFailed(format!("prepare list todos failed: {err}")))?;
    let rows = stmt
        .query_map(params_from_iter(params_vec), map_todo_row)
        .map_err(|err| ToolError::ExecFailed(format!("query list todos failed: {err}")))?;

    let mut out = Vec::new();
    for row in rows {
        out.push(
            row.map_err(|err| ToolError::ExecFailed(format!("decode todo row failed: {err}")))?,
        );
    }
    Ok(out)
}

fn get_todo_by_id(conn: &Connection, id: &str) -> Result<Option<TodoItem>, ToolError> {
    let mut stmt = conn
        .prepare(
            "SELECT id, title, description, status, priority, tags_json, due_at, task_id, task_status, created_at, updated_at
             FROM todos
             WHERE id = ?1
             LIMIT 1",
        )
        .map_err(|err| ToolError::ExecFailed(format!("prepare get todo failed: {err}")))?;

    let mut rows = stmt
        .query(params![id])
        .map_err(|err| ToolError::ExecFailed(format!("query get todo failed: {err}")))?;

    if let Some(row) = rows
        .next()
        .map_err(|err| ToolError::ExecFailed(format!("read get todo row failed: {err}")))?
    {
        return map_todo_row(row)
            .map(Some)
            .map_err(|err| ToolError::ExecFailed(format!("decode todo row failed: {err}")));
    }
    Ok(None)
}

fn update_todo(conn: &Connection, id: &str, patch: TodoPatch) -> Result<TodoItem, ToolError> {
    let mut current = get_todo_by_id(conn, id)?
        .ok_or_else(|| ToolError::InvalidArgs(format!("todo `{id}` not found")))?;

    if let Some(title) = patch.title {
        current.title = title;
    }
    if let Some(description) = patch.description {
        current.description = description;
    }
    if let Some(status) = patch.status {
        current.status = status;
    }
    if let Some(priority) = patch.priority {
        current.priority = priority;
    }
    if let Some(tags) = patch.tags {
        current.tags = tags;
    }
    if let Some(due_at) = patch.due_at {
        current.due_at = due_at;
    }
    if let Some(task_id) = patch.task_id {
        current.task_id = task_id;
    }
    if let Some(task_status) = patch.task_status {
        current.task_status = task_status;
    }
    current.updated_at = now_ms();

    let tags_json = serde_json::to_string(&current.tags)
        .map_err(|err| ToolError::ExecFailed(format!("serialize tags failed: {err}")))?;
    conn.execute(
        "UPDATE todos SET
            title = ?2,
            description = ?3,
            status = ?4,
            priority = ?5,
            tags_json = ?6,
            due_at = ?7,
            task_id = ?8,
            task_status = ?9,
            updated_at = ?10
         WHERE id = ?1",
        params![
            current.id,
            current.title,
            current.description,
            current.status,
            current.priority,
            tags_json,
            current.due_at.map(u64_to_i64),
            current.task_id,
            current.task_status,
            u64_to_i64(current.updated_at),
        ],
    )
    .map_err(|err| ToolError::ExecFailed(format!("update todo failed: {err}")))?;

    get_todo_by_id(conn, id)?
        .ok_or_else(|| ToolError::ExecFailed(format!("read updated todo `{id}` failed")))
}

fn delete_todo(conn: &Connection, id: &str) -> Result<bool, ToolError> {
    let changed = conn
        .execute("DELETE FROM todos WHERE id = ?1", params![id])
        .map_err(|err| ToolError::ExecFailed(format!("delete todo failed: {err}")))?;
    Ok(changed > 0)
}

fn map_todo_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TodoItem> {
    let tags_json: String = row.get(5)?;
    let tags = serde_json::from_str::<Vec<String>>(&tags_json).unwrap_or_default();
    Ok(TodoItem {
        id: row.get(0)?,
        title: row.get(1)?,
        description: row.get(2)?,
        status: row.get(3)?,
        priority: row.get(4)?,
        tags,
        due_at: row.get::<_, Option<i64>>(6)?.and_then(i64_to_u64),
        task_id: row.get(7)?,
        task_status: row.get(8)?,
        created_at: row.get::<_, i64>(9).map_or(0, |v| v.max(0) as u64),
        updated_at: row.get::<_, i64>(10).map_or(0, |v| v.max(0) as u64),
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
    Ok(Some(raw.trim().to_string()))
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

fn optional_i64(args: &Json, key: &str) -> Result<Option<i64>, ToolError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    value
        .as_i64()
        .map(Some)
        .ok_or_else(|| ToolError::InvalidArgs(format!("`{key}` must be an integer")))
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
        if !out.contains(&tag) {
            out.push(tag);
        }
    }
    Ok(out)
}

fn normalize_status(raw: String) -> Result<String, ToolError> {
    let status = normalize_enum(&raw);
    let allowed = ["todo", "in_progress", "blocked", "done", "cancelled"];
    if allowed.contains(&status.as_str()) {
        return Ok(status);
    }
    Err(ToolError::InvalidArgs(format!(
        "invalid todo status `{raw}`; allowed: {}",
        allowed.join(", ")
    )))
}

fn normalize_priority(raw: String) -> Result<String, ToolError> {
    let priority = normalize_enum(&raw);
    let allowed = ["low", "normal", "high", "urgent"];
    if allowed.contains(&priority.as_str()) {
        return Ok(priority);
    }
    Err(ToolError::InvalidArgs(format!(
        "invalid todo priority `{raw}`; allowed: {}",
        allowed.join(", ")
    )))
}

fn normalize_enum(raw: &str) -> String {
    raw.trim()
        .to_lowercase()
        .replace([' ', '-'], "_")
        .to_string()
}

fn validate_todo_text_fields(id: &str, title: &str, description: &str) -> Result<(), ToolError> {
    if id.is_empty() || id.len() > MAX_TITLE_LEN {
        return Err(ToolError::InvalidArgs(
            "`id` is empty or too long".to_string(),
        ));
    }
    validate_todo_title(title)?;
    validate_todo_description(description)?;
    Ok(())
}

fn validate_todo_title(title: &str) -> Result<(), ToolError> {
    if title.trim().is_empty() {
        return Err(ToolError::InvalidArgs(
            "`title` cannot be empty".to_string(),
        ));
    }
    if title.chars().count() > MAX_TITLE_LEN {
        return Err(ToolError::InvalidArgs(format!(
            "`title` length exceeds max {}",
            MAX_TITLE_LEN
        )));
    }
    Ok(())
}

fn validate_todo_description(description: &str) -> Result<(), ToolError> {
    if description.chars().count() > MAX_DESCRIPTION_LEN {
        return Err(ToolError::InvalidArgs(format!(
            "`description` length exceeds max {}",
            MAX_DESCRIPTION_LEN
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

fn generate_todo_id() -> String {
    let counter = TODO_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("todo-{}-{counter}", now_ms())
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

    fn test_ctx() -> ToolCallContext {
        ToolCallContext {
            trace_id: "trace-test".to_string(),
            agent_did: "did:example:agent".to_string(),
            behavior: "on_wakeup".to_string(),
            step_idx: 0,
            wakeup_id: "wakeup-test".to_string(),
        }
    }

    async fn call(tool: &TodoTool, args: Json) -> Result<Json, ToolError> {
        tool.call(&test_ctx(), args).await
    }

    #[tokio::test]
    async fn todo_tool_crud_flow_works() {
        let tmp = tempdir().expect("create tempdir");
        let db_path = tmp.path().join("todo").join("todo.db");
        let tool = TodoTool::new(TodoToolConfig::with_db_path(db_path)).expect("create todo tool");

        let created = call(
            &tool,
            json!({
                "action": "create",
                "id": "todo-crud-1",
                "title": "Implement todo tool test",
                "description": "validate create/list/get/update/delete",
                "status": "todo",
                "priority": "high",
                "tags": ["runtime", "runtime", "test"]
            }),
        )
        .await
        .expect("create todo");
        assert_eq!(created["todo"]["id"], "todo-crud-1");
        assert_eq!(created["todo"]["tags"], json!(["runtime", "test"]));

        let listed = call(
            &tool,
            json!({
                "action": "list",
                "status": "todo",
                "include_closed": false
            }),
        )
        .await
        .expect("list todos");
        let todos = listed["todos"].as_array().expect("todos array");
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0]["id"], "todo-crud-1");

        let updated = call(
            &tool,
            json!({
                "action": "update",
                "id": "todo-crud-1",
                "status": "in_progress",
                "task_id": 99,
                "task_status": "running"
            }),
        )
        .await
        .expect("update todo");
        assert_eq!(updated["todo"]["status"], "in_progress");
        assert_eq!(updated["todo"]["task_id"], 99);
        assert_eq!(updated["todo"]["task_status"], "running");

        let got = call(
            &tool,
            json!({
                "action": "get",
                "id": "todo-crud-1"
            }),
        )
        .await
        .expect("get todo");
        assert_eq!(got["todo"]["status"], "in_progress");

        let deleted = call(
            &tool,
            json!({
                "action": "delete",
                "id": "todo-crud-1"
            }),
        )
        .await
        .expect("delete todo");
        assert_eq!(deleted["deleted"], true);
    }

    #[tokio::test]
    async fn todo_tool_list_can_filter_closed_items() {
        let tmp = tempdir().expect("create tempdir");
        let db_path = tmp.path().join("todo").join("todo.db");
        let tool = TodoTool::new(TodoToolConfig::with_db_path(db_path)).expect("create todo tool");

        call(
            &tool,
            json!({
                "action": "create",
                "id": "todo-open",
                "title": "open item"
            }),
        )
        .await
        .expect("create open todo");
        call(
            &tool,
            json!({
                "action": "create",
                "id": "todo-done",
                "title": "done item",
                "status": "done"
            }),
        )
        .await
        .expect("create done todo");

        let active_only = call(
            &tool,
            json!({
                "action": "list",
                "include_closed": false
            }),
        )
        .await
        .expect("list active todos");
        let active = active_only["todos"].as_array().expect("todos array");
        assert_eq!(active.len(), 1);
        assert_eq!(active[0]["id"], "todo-open");
    }

    #[tokio::test]
    async fn todo_tool_validates_invalid_args() {
        let tmp = tempdir().expect("create tempdir");
        let db_path = tmp.path().join("todo").join("todo.db");
        let tool = TodoTool::new(TodoToolConfig::with_db_path(db_path)).expect("create todo tool");

        let err = call(
            &tool,
            json!({
                "action": "create",
                "title": "bad status",
                "status": "unknown_status"
            }),
        )
        .await
        .expect_err("invalid status should fail");
        assert!(matches!(err, ToolError::InvalidArgs(_)));

        call(
            &tool,
            json!({
                "action": "create",
                "id": "todo-update-noop",
                "title": "noop"
            }),
        )
        .await
        .expect("create todo");

        let err = call(
            &tool,
            json!({
                "action": "update",
                "id": "todo-update-noop"
            }),
        )
        .await
        .expect_err("update without patch should fail");
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }
}
