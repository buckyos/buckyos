use crate::task::{Task, TaskPermissions, TaskStatus};
use chrono::NaiveDateTime;
use log::*;
use rusqlite::{params, Connection, Result, Row};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct TaskDb {
    conn: Option<Arc<Mutex<Connection>>>,
}

impl TaskDb {
    pub fn new() -> Self {
        TaskDb { conn: None }
    }

    pub fn connect(&mut self, db_path: &str) -> Result<()> {
        let conn: Connection = Connection::open(db_path)?;
        self.conn = Some(Arc::new(Mutex::new(conn)));
        Ok(())
    }

    pub async fn init_db(&self) -> Result<(), rusqlite::Error> {
        let conn = self.conn.as_ref().unwrap();
        let conn = conn.lock().await;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS task (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                name            TEXT NOT NULL UNIQUE,
                title           TEXT,
                task_type       TEXT NOT NULL,
                app_name        TEXT,
                status          TEXT NOT NULL,
                progress        REAL NOT NULL,
                total_items     INTEGER NOT NULL,
                completed_items INTEGER NOT NULL,
                error_message   TEXT,
                data            TEXT,
                created_at      TEXT NOT NULL,
                updated_at      TEXT NOT NULL,
                user_id         TEXT,
                app_id          TEXT,
                parent_id       INTEGER,
                root_id         INTEGER,
                permissions     TEXT,
                message         TEXT
            )",
            params![],
        )?;
        Ok(())
    }

    pub async fn ensure_columns(&self) -> Result<()> {
        let conn = self.conn.as_ref().unwrap();
        let conn = conn.lock().await;
        let mut stmt = conn.prepare("PRAGMA table_info(task)")?;
        let column_iter = stmt.query_map([], |row| row.get::<_, String>(1))?;
        let mut columns: HashSet<String> = HashSet::new();
        for column in column_iter {
            columns.insert(column?);
        }

        let add_columns = [
            "user_id TEXT",
            "app_id TEXT",
            "parent_id INTEGER",
            "root_id INTEGER",
            "permissions TEXT",
            "message TEXT",
        ];

        for column in add_columns {
            let name = column.split_whitespace().next().unwrap_or("");
            if !columns.contains(name) {
                let sql = format!("ALTER TABLE task ADD COLUMN {}", column);
                conn.execute(sql.as_str(), params![])?;
            }
        }

        Ok(())
    }

    pub async fn create_task(&self, task: &Task) -> Result<i64> {
        let conn = self.conn.as_ref().unwrap();
        let conn = conn.lock().await;
        let data_str = serde_json::to_string(&task.data).unwrap_or_else(|_| "{}".to_string());
        let permissions_str =
            serde_json::to_string(&task.permissions).unwrap_or_else(|_| "{}".to_string());
        let created_at = task.created_at.to_string();
        let updated_at = task.updated_at.to_string();
        let app_name = if task.app_id.is_empty() {
            None
        } else {
            Some(task.app_id.clone())
        };

        conn.execute(
            "INSERT INTO task (
                name, title, task_type, app_name, status, progress,
                total_items, completed_items, error_message, data,
                created_at, updated_at, user_id, app_id, parent_id,
                root_id, permissions, message
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
            params![
                task.name,
                task.name,
                task.task_type,
                app_name,
                task.status.to_string(),
                task.progress,
                0,
                0,
                Option::<String>::None,
                data_str,
                created_at,
                updated_at,
                task.user_id,
                task.app_id,
                task.parent_id,
                task.root_id,
                permissions_str,
                task.message,
            ],
        )?;

        let id = conn.last_insert_rowid();
        Ok(id)
    }

    pub async fn set_root_id(&self, id: i64, root_id: i64) -> Result<()> {
        let conn = self.conn.as_ref().unwrap();
        let conn = conn.lock().await;
        conn.execute(
            "UPDATE task SET root_id = ?1 WHERE id = ?2",
            params![root_id, id],
        )?;
        Ok(())
    }

    pub async fn get_task(&self, id: i64) -> rusqlite::Result<Option<Task>> {
        let conn = self.conn.as_ref().unwrap();
        let conn = conn.lock().await;
        let mut stmt = conn.prepare("SELECT * FROM task WHERE id = ?1")?;
        let task_iter = stmt.query_map(params![id], |row| task_from_row(row))?;

        let mut tasks: Vec<Task> = Vec::new();
        for task in task_iter {
            tasks.push(task?);
        }

        if tasks.is_empty() {
            Ok(None)
        } else {
            Ok(Some(tasks[0].clone()))
        }
    }

    pub async fn list_tasks(&self) -> rusqlite::Result<Vec<Task>> {
        let conn = self.conn.as_ref().unwrap();
        let conn = conn.lock().await;
        let mut stmt = conn.prepare("SELECT * FROM task ORDER BY created_at DESC")?;
        let task_iter = stmt.query_map([], |row| task_from_row(row))?;

        let mut tasks = Vec::new();
        for task in task_iter {
            tasks.push(task?);
        }
        Ok(tasks)
    }

    pub async fn list_tasks_filtered(
        &self,
        app_id: Option<&str>,
        task_type: Option<&str>,
        status: Option<TaskStatus>,
        parent_id: Option<i64>,
        root_id: Option<i64>,
        user_id: Option<&str>,
    ) -> rusqlite::Result<Vec<Task>> {
        let mut sql = "SELECT * FROM task".to_string();
        let mut conditions: Vec<String> = Vec::new();
        let mut params_vec: Vec<rusqlite::types::Value> = Vec::new();

        if let Some(user_id) = user_id {
            conditions.push("user_id = ?".to_string());
            params_vec.push(rusqlite::types::Value::Text(user_id.to_string()));
        }
        if let Some(app_id) = app_id {
            conditions.push("(app_id = ? OR app_name = ?)".to_string());
            params_vec.push(rusqlite::types::Value::Text(app_id.to_string()));
            params_vec.push(rusqlite::types::Value::Text(app_id.to_string()));
        }
        if let Some(task_type) = task_type {
            conditions.push("task_type = ?".to_string());
            params_vec.push(rusqlite::types::Value::Text(task_type.to_string()));
        }
        if let Some(status) = status {
            conditions.push("status = ?".to_string());
            params_vec.push(status.to_string().into());
        }
        if let Some(parent_id) = parent_id {
            conditions.push("parent_id = ?".to_string());
            params_vec.push(parent_id.into());
        }
        if let Some(root_id) = root_id {
            conditions.push("root_id = ?".to_string());
            params_vec.push(root_id.into());
        }

        if !conditions.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&conditions.join(" AND "));
        }
        sql.push_str(" ORDER BY created_at DESC");

        let conn = self.conn.as_ref().unwrap();
        let conn = conn.lock().await;
        let mut stmt = conn.prepare(sql.as_str())?;
        let task_iter = stmt.query_map(rusqlite::params_from_iter(params_vec), |row| {
            task_from_row(row)
        })?;

        let mut tasks = Vec::new();
        for task in task_iter {
            tasks.push(task?);
        }
        Ok(tasks)
    }

    pub async fn list_tasks_by_app(&self, app_name: &str) -> rusqlite::Result<Vec<Task>> {
        self.list_tasks_filtered(Some(app_name), None, None, None, None, None)
            .await
    }

    pub async fn list_tasks_by_type(&self, task_type: &str) -> rusqlite::Result<Vec<Task>> {
        self.list_tasks_filtered(None, Some(task_type), None, None, None, None)
            .await
    }

    pub async fn list_tasks_by_status(&self, status: TaskStatus) -> rusqlite::Result<Vec<Task>> {
        self.list_tasks_filtered(None, None, Some(status), None, None, None)
            .await
    }

    pub async fn update_task_status(&self, id: i64, status: TaskStatus) -> Result<()> {
        let conn = self.conn.as_ref().unwrap();
        let conn = conn.lock().await;
        let updated_at = now_ts_string();
        conn.execute(
            "UPDATE task SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![status.to_string(), updated_at, id],
        )?;
        Ok(())
    }

    pub async fn update_task_status_by_root_id(
        &self,
        root_id: i64,
        status: TaskStatus,
    ) -> Result<()> {
        let conn = self.conn.as_ref().unwrap();
        let conn = conn.lock().await;
        let updated_at = now_ts_string();
        conn.execute(
            "UPDATE task SET status = ?1, updated_at = ?2 WHERE root_id = ?3",
            params![status.to_string(), updated_at, root_id],
        )?;
        Ok(())
    }

    pub async fn update_task_progress(
        &self,
        id: i64,
        progress: f32,
        completed_items: i32,
        total_items: i32,
    ) -> Result<()> {
        let conn = self.conn.as_ref().unwrap();
        let conn = conn.lock().await;
        let updated_at = now_ts_string();
        conn.execute(
            "UPDATE task SET progress = ?1, completed_items = ?2, total_items = ?3, updated_at = ?4 WHERE id = ?5",
            params![progress, completed_items, total_items, updated_at, id],
        )?;
        Ok(())
    }

    pub async fn update_task_error(&self, id: i64, error_message: &str) -> Result<()> {
        let conn = self.conn.as_ref().unwrap();
        let conn = conn.lock().await;
        let updated_at = now_ts_string();
        conn.execute(
            "UPDATE task SET status = ?1, error_message = ?2, message = ?3, updated_at = ?4 WHERE id = ?5",
            params![
                TaskStatus::Failed.to_string(),
                error_message,
                error_message,
                updated_at,
                id,
            ],
        )?;
        Ok(())
    }

    pub async fn update_task_data(&self, id: i64, data: &str) -> Result<()> {
        let conn = self.conn.as_ref().unwrap();
        let conn = conn.lock().await;
        let updated_at = now_ts_string();
        conn.execute(
            "UPDATE task SET data = ?1, updated_at = ?2 WHERE id = ?3",
            params![data, updated_at, id],
        )?;
        Ok(())
    }

    pub async fn update_task(
        &self,
        id: i64,
        status: Option<TaskStatus>,
        progress: Option<f32>,
        message: Option<String>,
        data_patch: Option<Value>,
    ) -> Result<()> {
        let mut data_str: Option<String> = None;
        if let Some(data_patch) = data_patch {
            if let Ok(existing) = self.get_task(id).await {
                let mut current = existing
                    .map(|task| task.data)
                    .unwrap_or_else(|| Value::Object(Default::default()));
                merge_json(&mut current, &data_patch);
                data_str = Some(serde_json::to_string(&current).unwrap_or_else(|_| "{}".to_string()));
            }
        }

        let conn = self.conn.as_ref().unwrap();
        let conn = conn.lock().await;
        let updated_at = now_ts_string();
        conn.execute(
            "UPDATE task SET status = COALESCE(?1, status), progress = COALESCE(?2, progress), message = COALESCE(?3, message), data = COALESCE(?4, data), updated_at = ?5 WHERE id = ?6",
            params![
                status.map(|s| s.to_string()),
                progress,
                message,
                data_str,
                updated_at,
                id
            ],
        )?;
        Ok(())
    }

    pub async fn delete_task(&self, id: i64) -> Result<()> {
        let conn = self.conn.as_ref().unwrap();
        let conn = conn.lock().await;
        conn.execute("DELETE FROM task WHERE id = ?1", params![id])?;
        Ok(())
    }
}

fn parse_timestamp(value: Option<String>) -> u64 {
    if let Some(value) = value {
        if let Ok(ts) = value.parse::<i64>() {
            return ts.max(0) as u64;
        }
        let format = "%Y-%m-%d %H:%M:%S%.f UTC";
        if let Ok(dt) = NaiveDateTime::parse_from_str(&value, format) {
            return dt.and_utc().timestamp().max(0) as u64;
        }
    }
    0
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

fn task_from_row(row: &Row) -> rusqlite::Result<Task> {
    let id: i64 = row.get("id")?;
    let name: String = row.get("name")?;
    let task_type: String = row.get("task_type")?;
    let status: String = row.get("status")?;
    let progress: f32 = row.get("progress")?;
    let message: Option<String> = row.get("message")?;
    let data_str: Option<String> = row.get("data")?;
    let permissions_str: Option<String> = row.get("permissions")?;
    let parent_id: Option<i64> = row.get("parent_id")?;
    let root_id: Option<i64> = row.get("root_id")?;
    let user_id: Option<String> = row.get("user_id")?;
    let app_id: Option<String> = row.get("app_id")?;
    let app_name: Option<String> = row.get("app_name")?;
    let created_at: Option<String> = row.get("created_at")?;
    let updated_at: Option<String> = row.get("updated_at")?;

    let mut resolved_root_id = root_id;
    if resolved_root_id.is_none() {
        resolved_root_id = Some(id);
    }

    let resolved_app_id = app_id.or(app_name).unwrap_or_else(|| "".to_string());
    let resolved_user_id = user_id.unwrap_or_else(|| "".to_string());

    Ok(Task {
        id,
        user_id: resolved_user_id,
        app_id: resolved_app_id,
        parent_id,
        root_id: resolved_root_id,
        name,
        task_type,
        status: TaskStatus::from_str(status.as_str()).unwrap_or(TaskStatus::Pending),
        progress,
        message,
        data: parse_data(data_str),
        permissions: parse_permissions(permissions_str),
        created_at: parse_timestamp(created_at),
        updated_at: parse_timestamp(updated_at),
    })
}

fn now_ts_string() -> String {
    chrono::Utc::now().timestamp().to_string()
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

pub async fn init_db(db_path: &str) {
    let mut db_manager = DB_MANAGER.lock().await;
    db_manager.connect(db_path).unwrap();
    match db_manager.init_db().await {
        Ok(_) => info!("Database initialized successfully."),
        Err(e) => info!("Failed to initialize database: {}", e),
    }

    if let Err(err) = db_manager.ensure_columns().await {
        warn!("Failed to ensure task_manager columns: {}", err);
    }
}

lazy_static::lazy_static! {
    pub static ref DB_MANAGER: Mutex<TaskDb> = Mutex::new(TaskDb::new());
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
            parent_id: None,
            root_id: None,
            status: TaskStatus::Pending,
            progress: 0.0,
            message: None,
            data: json!({}),
            permissions: TaskPermissions::default(),
            created_at: 1,
            updated_at: 1,
        }
    }

    async fn setup_test_db() -> (TaskDb, tempfile::TempDir) {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let db_path_str = db_path.to_str().unwrap();

        let mut db = TaskDb::new();
        db.connect(db_path_str).unwrap();
        db.init_db().await.unwrap();
        db.ensure_columns().await.unwrap();

        (db, temp_dir)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_connect_and_init() {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let db_path_str = db_path.to_str().unwrap();

        let mut db = TaskDb::new();
        assert!(db.connect(db_path_str).is_ok());
        assert!(db.init_db().await.is_ok());
        assert!(db.ensure_columns().await.is_ok());
        assert!(db_path.exists());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_create_and_get_task() {
        let (db, _temp_dir) = setup_test_db().await;

        let task = create_test_task("task1");
        let id = db.create_task(&task).await.unwrap();
        assert!(id > 0);

        let task_1_again = create_test_task("task1");
        let id2 = db.create_task(&task_1_again).await;
        assert!(id2.is_err());

        let retrieved_task = db.get_task(id).await.unwrap().unwrap();
        assert_eq!(retrieved_task.id, id);
        assert_eq!(retrieved_task.name, "task1");
        assert_eq!(retrieved_task.status, TaskStatus::Pending);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_list_tasks() {
        let (db, _temp_dir) = setup_test_db().await;

        let task1 = create_test_task("task1");
        let task2 = create_test_task("task2");
        let task3 = create_test_task("task3");

        db.create_task(&task1).await.unwrap();
        db.create_task(&task2).await.unwrap();
        db.create_task(&task3).await.unwrap();

        let tasks = db.list_tasks().await.unwrap();
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

        let app1_tasks = db.list_tasks_by_app("app1").await.unwrap();
        let app2_tasks = db.list_tasks_by_app("app2").await.unwrap();

        assert_eq!(app1_tasks.len(), 1);
        assert_eq!(app2_tasks.len(), 1);
        assert_eq!(app1_tasks[0].name, "task1");
        assert_eq!(app2_tasks[0].name, "task2");
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

        let type1_tasks = db.list_tasks_by_type("type1").await.unwrap();
        let type2_tasks = db.list_tasks_by_type("type2").await.unwrap();

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

        let running_tasks = db.list_tasks_by_status(TaskStatus::Running).await.unwrap();
        let completed_tasks = db.list_tasks_by_status(TaskStatus::Completed).await.unwrap();

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

        db.update_task_status(id, TaskStatus::Running).await.unwrap();

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
        assert_eq!(updated_task.progress, 0.5);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_update_task_error() {
        let (db, _temp_dir) = setup_test_db().await;

        let task = create_test_task("error_test");
        let id = db.create_task(&task).await.unwrap();

        db.update_task_error(id, "Test error message").await.unwrap();
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
}
