use crate::task::{Task, TaskNote, TaskPermissions, TaskStatus};
use buckyos_api::{
    get_rdb_instance, RdbBackend, TASK_MANAGER_RDB_INSTANCE_ID, TASK_MANAGER_RDB_SCHEMA_POSTGRES,
    TASK_MANAGER_RDB_SCHEMA_SQLITE, TASK_MANAGER_SERVICE_NAME,
};
use log::*;
use serde_json::Value;
use sqlx::any::{install_default_drivers, AnyPoolOptions, AnyRow};
use sqlx::{AnyPool, Executor, Row};
use std::sync::Once;

static INSTALL_DRIVERS: Once = Once::new();

fn ensure_any_drivers_installed() {
    INSTALL_DRIVERS.call_once(install_default_drivers);
}

/// Handle to the task-manager rdb. Wraps an `sqlx::AnyPool` — the pool itself
/// is already `Send + Sync + Clone` (internally `Arc`-backed) and manages its
/// own per-connection locking, so a `TaskDb` is safe to share via
/// `Arc<TaskDb>` with no outer Rust-level lock.
///
/// The rdb backend + connection string + DDL come from the service spec
/// (`spec_config.rdb_instances[...]`); the compile-time constants are only
/// a fallback used by tests that don't have a full runtime.
pub struct TaskDb {
    pool: AnyPool,
    backend: RdbBackend,
}

pub type DbResult<T> = Result<T, sqlx::Error>;

impl TaskDb {
    /// Open a pool against `connection`. `schema` is the DDL to apply (usually
    /// what the service spec carried for the chosen backend); an empty / None
    /// value means "use the compile-time default for `backend`".
    pub async fn open(
        connection: &str,
        backend: RdbBackend,
        schema: Option<&str>,
    ) -> Result<Self, String> {
        ensure_any_drivers_installed();
        let mut opts = AnyPoolOptions::new().max_connections(8);
        // Each pooled sqlite connection needs `foreign_keys = ON`; the pragma
        // is per-connection, not per-database, so setting it once on the pool
        // would only stick for the first connection and silently flip off for
        // the rest.
        if backend == RdbBackend::Sqlite {
            opts = opts.after_connect(|conn, _meta| {
                Box::pin(async move {
                    conn.execute("PRAGMA foreign_keys = ON;").await?;
                    Ok(())
                })
            });
        }
        let pool = opts
            .connect(connection)
            .await
            .map_err(|err| format!("open task-manager db at {}: {}", connection, err))?;
        let db = TaskDb { pool, backend };
        db.apply_schema(schema)
            .await
            .map_err(|err| format!("apply task-manager schema: {}", err))?;
        Ok(db)
    }

    /// Resolve the task-manager rdb instance from the service spec and open a
    /// pool against it. This is the production entry point.
    pub async fn open_from_service_spec() -> Result<Self, String> {
        let instance = get_rdb_instance(
            TASK_MANAGER_SERVICE_NAME,
            None,
            TASK_MANAGER_RDB_INSTANCE_ID,
        )
        .await
        .map_err(|err| format!("resolve task-manager rdb instance failed: {}", err))?;
        info!("task_db.open {}", instance.connection);
        Self::open(
            &instance.connection,
            instance.backend,
            instance.schema.as_deref(),
        )
        .await
    }

    async fn apply_schema(&self, override_ddl: Option<&str>) -> DbResult<()> {
        let ddl: &str =
            override_ddl
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(match self.backend {
                    RdbBackend::Sqlite => TASK_MANAGER_RDB_SCHEMA_SQLITE,
                    RdbBackend::Postgres => TASK_MANAGER_RDB_SCHEMA_POSTGRES,
                });
        // sqlx::AnyPool::execute accepts a single statement at a time when the
        // backend driver is strict, so split on ';' and run each non-empty
        // fragment.
        for statement in split_sql_statements(ddl) {
            self.pool.execute(statement.as_str()).await?;
        }
        self.cleanup_removed_task_schema().await?;
        Ok(())
    }

    async fn cleanup_removed_task_schema(&self) -> DbResult<()> {
        self.drop_task_name_scope_index().await?;
        self.drop_task_column("title").await?;
        // beta2.2: runner dispatch semantics removed from TaskMgr.
        self.drop_task_index("idx_task_runner_status").await?;
        self.drop_task_column("runner").await
    }

    async fn drop_task_name_scope_index(&self) -> DbResult<()> {
        self.drop_task_index("idx_task_name_scope").await
    }

    async fn drop_task_index(&self, index_name: &str) -> DbResult<()> {
        let sql = format!("DROP INDEX IF EXISTS {}", index_name);
        self.pool.execute(sql.as_str()).await?;
        Ok(())
    }

    async fn drop_task_column(&self, column: &str) -> DbResult<()> {
        match self.backend {
            RdbBackend::Sqlite => {
                let exists = sqlx::query("SELECT 1 FROM pragma_table_info('task') WHERE name = ?")
                    .bind(column)
                    .fetch_optional(self.pool())
                    .await?
                    .is_some();
                if exists {
                    let sql = format!("ALTER TABLE task DROP COLUMN {}", column);
                    self.pool.execute(sql.as_str()).await?;
                }
            }
            RdbBackend::Postgres => {
                let sql = format!("ALTER TABLE task DROP COLUMN IF EXISTS {}", column);
                self.pool.execute(sql.as_str()).await?;
            }
        }
        Ok(())
    }

    fn pool(&self) -> &AnyPool {
        &self.pool
    }

    pub async fn create_task(&self, task: &Task) -> DbResult<i64> {
        let data_str = serde_json::to_string(&task.data).unwrap_or_else(|_| "{}".to_string());
        let permissions_str =
            serde_json::to_string(&task.permissions).unwrap_or_else(|_| "{}".to_string());
        let created_at = task.created_at as i64;
        let updated_at = task.updated_at as i64;

        let sql = self.render_sql(
            "INSERT INTO task (
                name, task_type, status, progress,
                total_items, completed_items, error_message, data,
                created_at, updated_at, user_id, app_id, session_id, parent_id,
                root_id, permissions, message
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            RETURNING id",
        );

        let row = sqlx::query(&sql)
            .bind(task.name.clone())
            .bind(task.task_type.clone())
            .bind(task.status.to_string())
            .bind(task.progress as f64)
            .bind(0_i64)
            .bind(0_i64)
            .bind(Option::<String>::None)
            .bind(data_str)
            .bind(created_at)
            .bind(updated_at)
            .bind(task.user_id.clone())
            .bind(task.app_id.clone())
            .bind(task.session_id.clone())
            .bind(task.parent_id)
            .bind(task.root_id.clone())
            .bind(permissions_str)
            .bind(task.message.clone())
            .fetch_one(self.pool())
            .await?;

        let id: i64 = row.try_get("id")?;
        info!(
            "task_db.create_task: id={} app_id={} session_id={} user_id={} name={} task_type={} parent_id={:?} status={}",
            id,
            task.app_id,
            task.session_id,
            task.user_id,
            task.name,
            task.task_type,
            task.parent_id,
            task.status
        );
        Ok(id)
    }

    pub async fn set_root_id(&self, id: i64, root_id: &str) -> DbResult<()> {
        let sql = self.render_sql("UPDATE task SET root_id = ? WHERE id = ?");
        sqlx::query(&sql)
            .bind(root_id.to_string())
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn get_task(&self, id: i64) -> DbResult<Option<Task>> {
        let sql = self.render_sql("SELECT * FROM task WHERE id = ?");
        let row = sqlx::query(&sql)
            .bind(id)
            .fetch_optional(self.pool())
            .await?;
        row.map(task_from_row).transpose()
    }

    pub async fn add_task_note(&self, note: &TaskNote) -> DbResult<i64> {
        let data_str = serde_json::to_string(&note.data).unwrap_or_else(|_| "{}".to_string());
        let created_at = note.created_at as i64;
        let updated_at = note.updated_at as i64;

        let sql = self.render_sql(
            "INSERT INTO task_note (
                task_id, note_type, content, data,
                author_user_id, author_app_id, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            RETURNING id",
        );

        let row = sqlx::query(&sql)
            .bind(note.task_id)
            .bind(note.note_type.clone())
            .bind(note.content.clone())
            .bind(data_str)
            .bind(note.author_user_id.clone())
            .bind(note.author_app_id.clone())
            .bind(created_at)
            .bind(updated_at)
            .fetch_one(self.pool())
            .await?;

        let id: i64 = row.try_get("id")?;
        info!(
            "task_db.add_task_note: id={} task_id={} note_type={} author_user_id={} author_app_id={}",
            id,
            note.task_id,
            note.note_type,
            note.author_user_id,
            note.author_app_id
        );
        Ok(id)
    }

    pub async fn list_task_notes(&self, task_id: i64) -> DbResult<Vec<TaskNote>> {
        let sql = self.render_sql(
            "SELECT * FROM task_note WHERE task_id = ? ORDER BY created_at ASC, id ASC",
        );
        let rows = sqlx::query(&sql)
            .bind(task_id)
            .fetch_all(self.pool())
            .await?;
        rows.into_iter().map(task_note_from_row).collect()
    }

    pub async fn list_tasks_filtered(
        &self,
        app_id: Option<&str>,
        session_id: Option<&str>,
        task_type: Option<&str>,
        status: Option<TaskStatus>,
        parent_id: Option<i64>,
        root_id: Option<&str>,
        user_id: Option<&str>,
    ) -> DbResult<Vec<Task>> {
        let mut sql = String::from("SELECT * FROM task");
        let mut conditions: Vec<String> = Vec::new();
        enum Param {
            Text(String),
            Int(i64),
        }
        let mut params: Vec<Param> = Vec::new();

        if let Some(user_id) = user_id {
            conditions.push("user_id = ?".to_string());
            params.push(Param::Text(user_id.to_string()));
        }
        if let Some(app_id) = app_id {
            conditions.push("app_id = ?".to_string());
            params.push(Param::Text(app_id.to_string()));
        }
        if let Some(session_id) = session_id {
            conditions.push("session_id = ?".to_string());
            params.push(Param::Text(session_id.to_string()));
        }
        if let Some(task_type) = task_type {
            conditions.push("task_type = ?".to_string());
            params.push(Param::Text(task_type.to_string()));
        }
        if let Some(status) = status {
            conditions.push("status = ?".to_string());
            params.push(Param::Text(status.to_string()));
        }
        if let Some(parent_id) = parent_id {
            conditions.push("parent_id = ?".to_string());
            params.push(Param::Int(parent_id));
        }
        if let Some(root_id) = root_id {
            conditions.push("root_id = ?".to_string());
            params.push(Param::Text(root_id.to_string()));
        }

        if !conditions.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&conditions.join(" AND "));
        }
        sql.push_str(" ORDER BY created_at DESC");

        let sql = self.render_sql(&sql);
        let mut query = sqlx::query(&sql);
        for param in params {
            query = match param {
                Param::Text(v) => query.bind(v),
                Param::Int(v) => query.bind(v),
            };
        }
        let rows = query.fetch_all(self.pool()).await?;
        rows.into_iter().map(task_from_row).collect()
    }

    pub async fn update_task_status(&self, id: i64, status: TaskStatus) -> DbResult<()> {
        let updated_at = now_ts();
        let sql = self.render_sql("UPDATE task SET status = ?, updated_at = ? WHERE id = ?");
        let result = sqlx::query(&sql)
            .bind(status.to_string())
            .bind(updated_at)
            .bind(id)
            .execute(self.pool())
            .await?;
        info!(
            "task_db.update_task_status: id={} status={} changed={}",
            id,
            status,
            result.rows_affected()
        );
        Ok(())
    }

    pub async fn update_task_status_by_root_id(
        &self,
        root_id: &str,
        status: TaskStatus,
    ) -> DbResult<()> {
        let updated_at = now_ts();
        let sql = self.render_sql("UPDATE task SET status = ?, updated_at = ? WHERE root_id = ?");
        let result = sqlx::query(&sql)
            .bind(status.to_string())
            .bind(updated_at)
            .bind(root_id.to_string())
            .execute(self.pool())
            .await?;
        info!(
            "task_db.update_task_status_by_root_id: root_id={} status={} changed={}",
            root_id,
            status,
            result.rows_affected()
        );
        Ok(())
    }

    pub async fn update_task_progress(
        &self,
        id: i64,
        progress: f32,
        completed_items: i32,
        total_items: i32,
    ) -> DbResult<()> {
        let updated_at = now_ts();
        let sql = self.render_sql(
            "UPDATE task SET progress = ?, completed_items = ?, total_items = ?, updated_at = ? WHERE id = ?",
        );
        let result = sqlx::query(&sql)
            .bind(progress as f64)
            .bind(completed_items as i64)
            .bind(total_items as i64)
            .bind(updated_at)
            .bind(id)
            .execute(self.pool())
            .await?;
        info!(
            "task_db.update_task_progress: id={} progress={} completed_items={} total_items={} changed={}",
            id,
            progress,
            completed_items,
            total_items,
            result.rows_affected()
        );
        Ok(())
    }

    pub async fn update_task_error(&self, id: i64, error_message: &str) -> DbResult<()> {
        let updated_at = now_ts();
        let sql = self.render_sql(
            "UPDATE task SET status = ?, error_message = ?, message = ?, updated_at = ? WHERE id = ?",
        );
        let result = sqlx::query(&sql)
            .bind(TaskStatus::Failed.to_string())
            .bind(error_message.to_string())
            .bind(error_message.to_string())
            .bind(updated_at)
            .bind(id)
            .execute(self.pool())
            .await?;
        info!(
            "task_db.update_task_error: id={} changed={} error_message={}",
            id,
            result.rows_affected(),
            error_message
        );
        Ok(())
    }

    pub async fn update_task_data(&self, id: i64, data: &str) -> DbResult<()> {
        let data_value: Value = serde_json::from_str(data)
            .or_else(|_| {
                let unescaped = data.replace("\\\"", "\"");
                serde_json::from_str(unescaped.as_str())
            })
            .unwrap_or_else(|_| Value::Object(Default::default()));
        let data_str = serde_json::to_string(&data_value).unwrap_or_else(|_| "{}".to_string());
        let updated_at = now_ts();
        let sql = self.render_sql("UPDATE task SET data = ?, updated_at = ? WHERE id = ?");
        let result = sqlx::query(&sql)
            .bind(data_str)
            .bind(updated_at)
            .bind(id)
            .execute(self.pool())
            .await?;
        info!(
            "task_db.update_task_data: id={} changed={}",
            id,
            result.rows_affected()
        );
        Ok(())
    }

    pub async fn update_task(
        &self,
        id: i64,
        status: Option<TaskStatus>,
        progress: Option<f32>,
        message: Option<String>,
        data_patch: Option<Value>,
    ) -> DbResult<()> {
        // Fold a json-patch into the existing `data` column so callers can
        // send partial updates.
        let mut data_str: Option<String> = None;
        if let Some(data_patch) = data_patch {
            if let Ok(existing) = self.get_task(id).await {
                let mut current = existing
                    .map(|task| task.data)
                    .unwrap_or_else(|| Value::Object(Default::default()));
                merge_json(&mut current, &data_patch);
                data_str =
                    Some(serde_json::to_string(&current).unwrap_or_else(|_| "{}".to_string()));
            }
        }

        let updated_at = now_ts();
        let status_str = status.as_ref().map(|s| s.to_string());
        let progress_f64 = progress.map(|p| p as f64);
        let message_present = message.is_some();
        let data_patch_present = data_str.is_some();
        let sql = self.render_sql(
            "UPDATE task SET
                status = COALESCE(?, status),
                progress = COALESCE(?, progress),
                message = COALESCE(?, message),
                data = COALESCE(?, data),
                updated_at = ?
            WHERE id = ?",
        );
        let result = sqlx::query(&sql)
            .bind(status_str)
            .bind(progress_f64)
            .bind(message.clone())
            .bind(data_str)
            .bind(updated_at)
            .bind(id)
            .execute(self.pool())
            .await?;
        info!(
            "task_db.update_task: id={} status={:?} progress={:?} message_present={} data_patch_present={} changed={}",
            id,
            status,
            progress,
            message_present,
            data_patch_present,
            result.rows_affected()
        );
        Ok(())
    }

    pub async fn delete_task(&self, id: i64) -> DbResult<()> {
        let sql = self.render_sql("DELETE FROM task WHERE id = ?");
        sqlx::query(&sql).bind(id).execute(self.pool()).await?;
        Ok(())
    }

    pub async fn delete_tasks_by_session_id(&self, session_id: &str) -> DbResult<u64> {
        let sql = self.render_sql("DELETE FROM task WHERE session_id = ?");
        let result = sqlx::query(&sql)
            .bind(session_id.to_string())
            .execute(self.pool())
            .await?;
        Ok(result.rows_affected())
    }

    /// Translate `?` placeholders into `$N` form for postgres. Other backends
    /// pass through unchanged.
    fn render_sql(&self, sql: &str) -> String {
        match self.backend {
            RdbBackend::Postgres => rewrite_placeholders_to_dollar(sql),
            RdbBackend::Sqlite => sql.to_string(),
        }
    }
}

pub(crate) fn rewrite_placeholders_to_dollar(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut idx = 0u32;
    let mut in_single = false;
    let mut in_double = false;
    for ch in sql.chars() {
        match ch {
            '\'' if !in_double => {
                in_single = !in_single;
                out.push(ch);
            }
            '"' if !in_single => {
                in_double = !in_double;
                out.push(ch);
            }
            '?' if !in_single && !in_double => {
                idx += 1;
                out.push('$');
                out.push_str(&idx.to_string());
            }
            _ => out.push(ch),
        }
    }
    out
}

pub(crate) fn split_sql_statements(ddl: &str) -> Vec<String> {
    let mut stmts = Vec::new();
    let mut buf = String::new();
    let mut in_single = false;
    let mut in_double = false;
    for ch in ddl.chars() {
        match ch {
            '\'' if !in_double => {
                in_single = !in_single;
                buf.push(ch);
            }
            '"' if !in_single => {
                in_double = !in_double;
                buf.push(ch);
            }
            ';' if !in_single && !in_double => {
                let trimmed = buf.trim();
                if !trimmed.is_empty() {
                    stmts.push(trimmed.to_string());
                }
                buf.clear();
            }
            _ => buf.push(ch),
        }
    }
    let trimmed = buf.trim();
    if !trimmed.is_empty() {
        stmts.push(trimmed.to_string());
    }
    stmts
}

fn parse_permissions(value: Option<String>) -> TaskPermissions {
    if let Some(value) = value {
        if let Ok(permissions) = serde_json::from_str::<TaskPermissions>(&value) {
            return permissions;
        }
    }
    TaskPermissions::default()
}

fn parse_data(value: Option<String>) -> Value {
    if let Some(value) = value {
        if let Ok(data) = serde_json::from_str::<Value>(&value) {
            return data;
        }
    }
    Value::Object(Default::default())
}

fn task_from_row(row: AnyRow) -> DbResult<Task> {
    // Column decode failures mean schema drift (column missing, wrong type).
    // Surface them as sqlx errors instead of silently returning a fake task.
    let id: i64 = row.try_get("id")?;
    let name: String = row.try_get("name")?;
    let task_type: String = row.try_get("task_type")?;
    let status: String = row.try_get("status")?;
    let progress: f64 = row.try_get("progress")?;
    let message: Option<String> = row.try_get("message")?;
    let data_str: Option<String> = row.try_get("data")?;
    let permissions_str: Option<String> = row.try_get("permissions")?;
    let parent_id: Option<i64> = row.try_get("parent_id")?;
    let root_id: Option<String> = row.try_get("root_id")?;
    let user_id: Option<String> = row.try_get("user_id")?;
    let app_id: Option<String> = row.try_get("app_id")?;
    let session_id: Option<String> = row.try_get("session_id")?;
    let created_at: i64 = row.try_get("created_at")?;
    let updated_at: i64 = row.try_get("updated_at")?;

    let resolved_root_id = root_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| id.to_string());

    Ok(Task {
        id,
        user_id: user_id.unwrap_or_default(),
        app_id: app_id.unwrap_or_default(),
        session_id: session_id.unwrap_or_default(),
        parent_id,
        root_id: resolved_root_id,
        name,
        task_type,
        // `data` / `permissions` are opaque JSON payloads written by callers
        // with different serde versions over time — tolerate parse failures
        // with an empty default since the column itself decoded fine.
        status: TaskStatus::from_str(status.as_str()).unwrap_or(TaskStatus::Pending),
        progress: progress as f32,
        message,
        data: parse_data(data_str),
        permissions: parse_permissions(permissions_str),
        created_at: created_at.max(0) as u64,
        updated_at: updated_at.max(0) as u64,
    })
}

fn task_note_from_row(row: AnyRow) -> DbResult<TaskNote> {
    let id: i64 = row.try_get("id")?;
    let task_id: i64 = row.try_get("task_id")?;
    let note_type: String = row.try_get("note_type")?;
    let content: String = row.try_get("content")?;
    let data_str: Option<String> = row.try_get("data")?;
    let author_user_id: String = row.try_get("author_user_id")?;
    let author_app_id: String = row.try_get("author_app_id")?;
    let created_at: i64 = row.try_get("created_at")?;
    let updated_at: i64 = row.try_get("updated_at")?;

    Ok(TaskNote {
        id,
        task_id,
        note_type,
        content,
        data: parse_data(data_str),
        author_user_id,
        author_app_id,
        created_at: created_at.max(0) as u64,
        updated_at: updated_at.max(0) as u64,
    })
}

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

fn merge_json(target: &mut Value, patch: &Value) {
    match (target, patch) {
        (Value::Object(target_map), Value::Object(patch_map)) => {
            for (key, value) in patch_map {
                if value.is_null() {
                    target_map.remove(key);
                } else if let Some(existing) = target_map.get_mut(key) {
                    merge_json(existing, value);
                } else {
                    target_map.insert(key.clone(), value.clone());
                }
            }
        }
        (target_value, patch_value) => {
            *target_value = patch_value.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    fn create_test_task(name: &str) -> Task {
        Task {
            id: 0,
            name: name.to_string(),
            task_type: "test_type".to_string(),
            user_id: "user1".to_string(),
            app_id: "app1".to_string(),
            session_id: "session1".to_string(),
            parent_id: None,
            root_id: String::new(),
            status: TaskStatus::Pending,
            progress: 0.0,
            message: None,
            data: json!({}),
            permissions: TaskPermissions::default(),
            created_at: 1,
            updated_at: 1,
        }
    }

    fn create_test_note(task_id: i64, content: &str) -> TaskNote {
        TaskNote {
            id: 0,
            task_id,
            note_type: "human".to_string(),
            content: content.to_string(),
            data: json!({"priority": "high"}),
            author_user_id: "user1".to_string(),
            author_app_id: "task-center".to_string(),
            created_at: 2,
            updated_at: 2,
        }
    }

    async fn setup_test_db() -> (TaskDb, tempfile::TempDir) {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let conn = format!("sqlite://{}?mode=rwc", db_path.to_str().unwrap());

        let db = TaskDb::open(&conn, RdbBackend::Sqlite, None).await.unwrap();
        (db, temp_dir)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_connect_and_init() {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let conn = format!("sqlite://{}?mode=rwc", db_path.to_str().unwrap());

        let _db = TaskDb::open(&conn, RdbBackend::Sqlite, None).await.unwrap();
        assert!(db_path.exists());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_create_and_get_task() {
        let (db, _temp_dir) = setup_test_db().await;

        let task = create_test_task("task1");
        let id = db.create_task(&task).await.unwrap();
        assert!(id > 0);

        let task_1_again = create_test_task("task1");
        let id2 = db.create_task(&task_1_again).await.unwrap();
        assert_ne!(id, id2);

        let retrieved_task = db.get_task(id).await.unwrap().unwrap();
        assert_eq!(retrieved_task.id, id);
        assert_eq!(retrieved_task.name, "task1");
        assert_eq!(retrieved_task.status, TaskStatus::Pending);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_open_cleans_removed_task_schema() {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let conn = format!("sqlite://{}?mode=rwc", db_path.to_str().unwrap());
        let old_schema = r#"
CREATE TABLE IF NOT EXISTS task (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    name            TEXT NOT NULL,
    title           TEXT,
    task_type       TEXT NOT NULL,
    runner          TEXT NOT NULL DEFAULT '',
    status          TEXT NOT NULL,
    progress        REAL NOT NULL,
    total_items     INTEGER NOT NULL DEFAULT 0,
    completed_items INTEGER NOT NULL DEFAULT 0,
    error_message   TEXT,
    data            TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    user_id         TEXT NOT NULL DEFAULT '',
    app_id          TEXT NOT NULL DEFAULT '',
    session_id      TEXT NOT NULL DEFAULT '',
    parent_id       INTEGER,
    root_id         TEXT NOT NULL DEFAULT '',
    permissions     TEXT,
    message         TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_task_name_scope ON task(app_id, user_id, name);
CREATE INDEX IF NOT EXISTS idx_task_runner_status ON task(runner, status);
"#;

        let db = TaskDb::open(&conn, RdbBackend::Sqlite, Some(old_schema))
            .await
            .unwrap();

        for removed_column in ["title", "runner"] {
            let column = sqlx::query("SELECT 1 FROM pragma_table_info('task') WHERE name = ?")
                .bind(removed_column)
                .fetch_optional(db.pool())
                .await
                .unwrap();
            assert!(column.is_none(), "column {} should be dropped", removed_column);
        }

        for removed_index in ["idx_task_name_scope", "idx_task_runner_status"] {
            let index = sqlx::query("SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = ?")
                .bind(removed_index)
                .fetch_optional(db.pool())
                .await
                .unwrap();
            assert!(index.is_none(), "index {} should be dropped", removed_index);
        }

        let id1 = db.create_task(&create_test_task("task1")).await.unwrap();
        let id2 = db.create_task(&create_test_task("task1")).await.unwrap();
        assert_ne!(id1, id2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_add_and_list_task_notes() {
        let (db, _temp_dir) = setup_test_db().await;

        let task_id = db
            .create_task(&create_test_task("note_parent"))
            .await
            .unwrap();

        let first_id = db
            .add_task_note(&create_test_note(task_id, "first note"))
            .await
            .unwrap();
        let second_id = db
            .add_task_note(&create_test_note(task_id, "second note"))
            .await
            .unwrap();

        let notes = db.list_task_notes(task_id).await.unwrap();
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].id, first_id);
        assert_eq!(notes[0].task_id, task_id);
        assert_eq!(notes[0].content, "first note");
        assert_eq!(notes[0].data, json!({"priority": "high"}));
        assert_eq!(notes[1].id, second_id);
        assert_eq!(notes[1].content, "second note");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_list_tasks() {
        let (db, _temp_dir) = setup_test_db().await;

        db.create_task(&create_test_task("task1")).await.unwrap();
        db.create_task(&create_test_task("task2")).await.unwrap();
        db.create_task(&create_test_task("task3")).await.unwrap();

        let tasks = db
            .list_tasks_filtered(None, None, None, None, None, None, None)
            .await
            .unwrap();
        assert_eq!(tasks.len(), 3);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_list_tasks_by_app() {
        let (db, _temp_dir) = setup_test_db().await;

        let mut task1 = create_test_task("task1");
        let mut task2 = create_test_task("task2");
        task1.app_id = "app1".to_string();
        task2.app_id = "app2".to_string();

        db.create_task(&task1).await.unwrap();
        db.create_task(&task2).await.unwrap();

        let app1_tasks = db
            .list_tasks_filtered(Some("app1"), None, None, None, None, None, None)
            .await
            .unwrap();
        let app2_tasks = db
            .list_tasks_filtered(Some("app2"), None, None, None, None, None, None)
            .await
            .unwrap();

        assert_eq!(app1_tasks.len(), 1);
        assert_eq!(app2_tasks.len(), 1);
        assert_eq!(app1_tasks[0].name, "task1");
        assert_eq!(app2_tasks[0].name, "task2");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_list_and_delete_tasks_by_session() {
        let (db, _temp_dir) = setup_test_db().await;

        let mut task1 = create_test_task("session_task_1");
        let mut task2 = create_test_task("session_task_2");
        let mut task3 = create_test_task("other_session_task");
        task1.app_id = "app1".to_string();
        task2.app_id = "app2".to_string();
        task1.session_id = "session-alpha".to_string();
        task2.session_id = "session-alpha".to_string();
        task3.session_id = "session-beta".to_string();

        db.create_task(&task1).await.unwrap();
        db.create_task(&task2).await.unwrap();
        db.create_task(&task3).await.unwrap();

        let session_tasks = db
            .list_tasks_filtered(None, Some("session-alpha"), None, None, None, None, None)
            .await
            .unwrap();
        assert_eq!(session_tasks.len(), 2);

        let deleted_count = db
            .delete_tasks_by_session_id("session-alpha")
            .await
            .unwrap();
        assert_eq!(deleted_count, 2);

        let remaining = db
            .list_tasks_filtered(None, None, None, None, None, None, None)
            .await
            .unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].session_id, "session-beta");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_list_tasks_by_type() {
        let (db, _temp_dir) = setup_test_db().await;

        let mut task1 = create_test_task("task1");
        let mut task2 = create_test_task("task2");
        task1.task_type = "type1".to_string();
        task2.task_type = "type2".to_string();

        db.create_task(&task1).await.unwrap();
        db.create_task(&task2).await.unwrap();

        let type1_tasks = db
            .list_tasks_filtered(None, None, Some("type1"), None, None, None, None)
            .await
            .unwrap();
        let type2_tasks = db
            .list_tasks_filtered(None, None, Some("type2"), None, None, None, None)
            .await
            .unwrap();

        assert_eq!(type1_tasks.len(), 1);
        assert_eq!(type2_tasks.len(), 1);
        assert_eq!(type1_tasks[0].name, "task1");
        assert_eq!(type2_tasks[0].name, "task2");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_list_tasks_by_status() {
        let (db, _temp_dir) = setup_test_db().await;

        let mut task1 = create_test_task("task1");
        let mut task2 = create_test_task("task2");
        task1.status = TaskStatus::Running;
        task2.status = TaskStatus::Completed;

        db.create_task(&task1).await.unwrap();
        db.create_task(&task2).await.unwrap();

        let running_tasks = db
            .list_tasks_filtered(None, None, None, Some(TaskStatus::Running), None, None, None)
            .await
            .unwrap();
        let completed_tasks = db
            .list_tasks_filtered(
                None,
                None,
                None,
                Some(TaskStatus::Completed),
                None,
                None,
                None,
            )
            .await
            .unwrap();

        assert_eq!(running_tasks.len(), 1);
        assert_eq!(completed_tasks.len(), 1);
        assert_eq!(running_tasks[0].name, "task1");
        assert_eq!(completed_tasks[0].name, "task2");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_update_task_status() {
        let (db, _temp_dir) = setup_test_db().await;

        let task = create_test_task("status_test");
        let id = db.create_task(&task).await.unwrap();

        db.update_task_status(id, TaskStatus::Running)
            .await
            .unwrap();

        let updated_task = db.get_task(id).await.unwrap().unwrap();
        assert_eq!(updated_task.status, TaskStatus::Running);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_update_task_progress() {
        let (db, _temp_dir) = setup_test_db().await;

        let task = create_test_task("progress_test");
        let id = db.create_task(&task).await.unwrap();

        db.update_task_progress(id, 0.5, 5, 10).await.unwrap();
        let updated_task = db.get_task(id).await.unwrap().unwrap();
        assert!((updated_task.progress - 0.5).abs() < 1e-6);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_update_task_error() {
        let (db, _temp_dir) = setup_test_db().await;

        let task = create_test_task("error_test");
        let id = db.create_task(&task).await.unwrap();

        db.update_task_error(id, "Test error message")
            .await
            .unwrap();
        let updated_task = db.get_task(id).await.unwrap().unwrap();
        assert_eq!(updated_task.status, TaskStatus::Failed);
        assert_eq!(updated_task.message, Some("Test error message".to_string()));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_update_task_data() {
        let (db, _temp_dir) = setup_test_db().await;

        let task = create_test_task("data_test");
        let id = db.create_task(&task).await.unwrap();

        let new_data = r#"{\"key\": \"value\"}"#;
        db.update_task_data(id, new_data).await.unwrap();

        let updated_task = db.get_task(id).await.unwrap().unwrap();
        assert_eq!(updated_task.data, json!({"key": "value"}));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_delete_task() {
        let (db, _temp_dir) = setup_test_db().await;

        let task = create_test_task("delete_test");
        let id = db.create_task(&task).await.unwrap();

        db.delete_task(id).await.unwrap();
        let result = db.get_task(id).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_foreign_key_cascade_across_connections() {
        // Regression: `PRAGMA foreign_keys = ON` is per-connection; if the
        // after_connect hook is missing the cascade silently stops working
        // on whichever connection sqlx happens to hand out.
        let (db, _temp_dir) = setup_test_db().await;
        let parent_id = db.create_task(&create_test_task("parent")).await.unwrap();

        // Spawn several concurrent queries so the pool opens multiple
        // connections — each one must still have FKs enabled.
        let handles: Vec<_> = (0..6)
            .map(|i| {
                let db_pool = db.pool().clone();
                tokio::spawn(async move {
                    sqlx::query("SELECT id FROM task WHERE id = ?")
                        .bind(parent_id)
                        .fetch_optional(&db_pool)
                        .await
                        .unwrap_or_else(|e| panic!("probe query {} failed: {}", i, e))
                })
            })
            .collect();
        for h in handles {
            h.await.unwrap();
        }

        let mut child = create_test_task("child");
        child.parent_id = Some(parent_id);
        let child_id = db.create_task(&child).await.unwrap();

        db.delete_task(parent_id).await.unwrap();
        assert!(
            db.get_task(child_id).await.unwrap().is_none(),
            "child task should have been cascade-deleted"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_task_delete_cascades_notes() {
        let (db, _temp_dir) = setup_test_db().await;
        let task_id = db
            .create_task(&create_test_task("note_cascade_parent"))
            .await
            .unwrap();
        db.add_task_note(&create_test_note(task_id, "will be deleted"))
            .await
            .unwrap();

        db.delete_task(task_id).await.unwrap();

        let notes = db.list_task_notes(task_id).await.unwrap();
        assert!(notes.is_empty());
    }

    #[test]
    fn rewrite_placeholders_handles_quotes() {
        assert_eq!(
            rewrite_placeholders_to_dollar("SELECT ? FROM t WHERE s = '?' AND x = ?"),
            "SELECT $1 FROM t WHERE s = '?' AND x = $2"
        );
    }
}
