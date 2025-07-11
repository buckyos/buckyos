use rusqlite::{params, Connection, Result};
use crate::task::{Task, TaskStatus};
use chrono::{NaiveDateTime, TimeZone, Utc};
use log::*;
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
                title           TEXT NOT NULL,
                task_type       TEXT NOT NULL,
                app_name        TEXT NOT NULL,
                status          TEXT NOT NULL,
                progress        REAL NOT NULL,
                total_items     INTEGER NOT NULL,
                completed_items INTEGER NOT NULL,
                error_message   TEXT,
                data            TEXT,
                created_at      TEXT NOT NULL,
                updated_at      TEXT NOT NULL
            )",
            params![],
        )?;
        Ok(())
    }

    pub async fn create_task(&self, task: &Task) -> Result<i64> {
        let conn = self.conn.as_ref().unwrap();
        let conn = conn.lock().await;
        conn.execute(
            "INSERT INTO task (
                name, title, task_type, app_name, status, progress, 
                total_items, completed_items, error_message, data, 
                created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                task.name, 
                task.title,
                task.task_type,
                task.app_name,
                task.status.to_string(), 
                task.progress,
                task.total_items,
                task.completed_items,
                task.error_message,
                task.data,
                task.created_at.to_string(), 
                task.updated_at.to_string(), 
            ],
        )?;
        
        // 获取最后插入的ID
        let id = conn.last_insert_rowid();
        Ok(id)
    }

    pub async fn get_task(&self, id: i64) -> rusqlite::Result<Option<Task>> {
        let conn = self.conn.as_ref().unwrap();
        let conn = conn.lock().await;
        let mut stmt = conn.prepare("SELECT * FROM task WHERE id = ?1")?;
        
        let task_iter = stmt.query_map(params![id], |row| {
            let format = "%Y-%m-%d %H:%M:%S%.f UTC";

            let id = row.get(0)?;
            let name: String = row.get(1)?;
            let title: String = row.get(2)?;
            let task_type: String = row.get(3)?;
            let app_name: String = row.get(4)?;
            let status: String = row.get(5)?;
            let progress: f32 = row.get(6)?;
            let total_items: i32 = row.get(7)?;
            let completed_items: i32 = row.get(8)?;
            let error_message: Option<String> = row.get(9)?;
            let data: Option<String> = row.get(10)?;
            let created_at: String = row.get(11)?;
            let created_at = NaiveDateTime::parse_from_str(&created_at, format).unwrap();
            let updated_at: String = row.get(12)?;
            let updated_at = NaiveDateTime::parse_from_str(&updated_at, format).unwrap();
            
            Ok(Task {
                id,
                name,
                title,
                task_type,
                app_name,
                status: TaskStatus::from_str(status.as_str()).unwrap(),
                progress,
                total_items,
                completed_items,
                error_message,
                data,
                created_at: Utc.from_utc_datetime(&created_at),
                updated_at: Utc.from_utc_datetime(&updated_at),
            })
        })?;

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
        
        let task_iter = stmt.query_map([], |row| {
            let format = "%Y-%m-%d %H:%M:%S%.f UTC";

            let id = row.get(0)?;
            let name: String = row.get(1)?;
            let title: String = row.get(2)?;
            let task_type: String = row.get(3)?;
            let app_name: String = row.get(4)?;
            let status: String = row.get(5)?;
            let progress: f32 = row.get(6)?;
            let total_items: i32 = row.get(7)?;
            let completed_items: i32 = row.get(8)?;
            let error_message: Option<String> = row.get(9)?;
            let data: Option<String> = row.get(10)?;
            let created_at: String = row.get(11)?;
            let created_at = NaiveDateTime::parse_from_str(&created_at, format).unwrap();
            let updated_at: String = row.get(12)?;
            let updated_at = NaiveDateTime::parse_from_str(&updated_at, format).unwrap();
            
            Ok(Task {
                id,
                name,
                title,
                task_type,
                app_name,
                status: TaskStatus::from_str(status.as_str()).unwrap(),
                progress,
                total_items,
                completed_items,
                error_message,
                data,
                created_at: Utc.from_utc_datetime(&created_at),
                updated_at: Utc.from_utc_datetime(&updated_at),
            })
        })?;

        let mut tasks = Vec::new();
        for task in task_iter {
            tasks.push(task?);
        }
        Ok(tasks)
    }

    pub async fn list_tasks_by_app(&self, app_name: &str) -> rusqlite::Result<Vec<Task>> {
        let conn = self.conn.as_ref().unwrap();
        let conn = conn.lock().await;
        let mut stmt = conn.prepare("SELECT * FROM task WHERE app_name = ?1 ORDER BY created_at DESC")?;
        
        let task_iter = stmt.query_map(params![app_name], |row| {
            let format = "%Y-%m-%d %H:%M:%S%.f UTC";

            let id = row.get(0)?;
            let name: String = row.get(1)?;
            let title: String = row.get(2)?;
            let task_type: String = row.get(3)?;
            let app_name: String = row.get(4)?;
            let status: String = row.get(5)?;
            let progress: f32 = row.get(6)?;
            let total_items: i32 = row.get(7)?;
            let completed_items: i32 = row.get(8)?;
            let error_message: Option<String> = row.get(9)?;
            let data: Option<String> = row.get(10)?;
            let created_at: String = row.get(11)?;
            let created_at = NaiveDateTime::parse_from_str(&created_at, format).unwrap();
            let updated_at: String = row.get(12)?;
            let updated_at = NaiveDateTime::parse_from_str(&updated_at, format).unwrap();
            
            Ok(Task {
                id,
                name,
                title,
                task_type,
                app_name,
                status: TaskStatus::from_str(status.as_str()).unwrap(),
                progress,
                total_items,
                completed_items,
                error_message,
                data,
                created_at: Utc.from_utc_datetime(&created_at),
                updated_at: Utc.from_utc_datetime(&updated_at),
            })
        })?;

        let mut tasks = Vec::new();
        for task in task_iter {
            tasks.push(task?);
        }
        Ok(tasks)
    }

    pub async fn list_tasks_by_type(&self, task_type: &str) -> rusqlite::Result<Vec<Task>> {
        let conn = self.conn.as_ref().unwrap();
        let conn = conn.lock().await;
        let mut stmt = conn.prepare("SELECT * FROM task WHERE task_type = ?1 ORDER BY created_at DESC")?;
        
        let task_iter = stmt.query_map(params![task_type], |row| {
            let format = "%Y-%m-%d %H:%M:%S%.f UTC";

            let id = row.get(0)?;
            let name: String = row.get(1)?;
            let title: String = row.get(2)?;
            let task_type: String = row.get(3)?;
            let app_name: String = row.get(4)?;
            let status: String = row.get(5)?;
            let progress: f32 = row.get(6)?;
            let total_items: i32 = row.get(7)?;
            let completed_items: i32 = row.get(8)?;
            let error_message: Option<String> = row.get(9)?;
            let data: Option<String> = row.get(10)?;
            let created_at: String = row.get(11)?;
            let created_at = NaiveDateTime::parse_from_str(&created_at, format).unwrap();
            let updated_at: String = row.get(12)?;
            let updated_at = NaiveDateTime::parse_from_str(&updated_at, format).unwrap();
            
            Ok(Task {
                id,
                name,
                title,
                task_type,
                app_name,
                status: TaskStatus::from_str(status.as_str()).unwrap(),
                progress,
                total_items,
                completed_items,
                error_message,
                data,
                created_at: Utc.from_utc_datetime(&created_at),
                updated_at: Utc.from_utc_datetime(&updated_at),
            })
        })?;

        let mut tasks = Vec::new();
        for task in task_iter {
            tasks.push(task?);
        }
        Ok(tasks)
    }

    pub async fn list_tasks_by_status(&self, status: TaskStatus) -> rusqlite::Result<Vec<Task>> {
        let conn = self.conn.as_ref().unwrap();
        let conn = conn.lock().await;
        let mut stmt = conn.prepare("SELECT * FROM task WHERE status = ?1 ORDER BY created_at DESC")?;
        
        let task_iter = stmt.query_map(params![status.to_string()], |row| {
            let format = "%Y-%m-%d %H:%M:%S%.f UTC";

            let id = row.get(0)?;
            let name: String = row.get(1)?;
            let title: String = row.get(2)?;
            let task_type: String = row.get(3)?;
            let app_name: String = row.get(4)?;
            let status: String = row.get(5)?;
            let progress: f32 = row.get(6)?;
            let total_items: i32 = row.get(7)?;
            let completed_items: i32 = row.get(8)?;
            let error_message: Option<String> = row.get(9)?;
            let data: Option<String> = row.get(10)?;
            let created_at: String = row.get(11)?;
            let created_at = NaiveDateTime::parse_from_str(&created_at, format).unwrap();
            let updated_at: String = row.get(12)?;
            let updated_at = NaiveDateTime::parse_from_str(&updated_at, format).unwrap();
            
            Ok(Task {
                id,
                name,
                title,
                task_type,
                app_name,
                status: TaskStatus::from_str(status.as_str()).unwrap(),
                progress,
                total_items,
                completed_items,
                error_message,
                data,
                created_at: Utc.from_utc_datetime(&created_at),
                updated_at: Utc.from_utc_datetime(&updated_at),
            })
        })?;

        let mut tasks = Vec::new();
        for task in task_iter {
            tasks.push(task?);
        }
        Ok(tasks)
    }

    pub async fn update_task_status(&self, id: i64, status: TaskStatus) -> Result<()> {
        let conn = self.conn.as_ref().unwrap();
        let conn = conn.lock().await;
        let updated_at = Utc::now();
        conn.execute(
            "UPDATE task SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![
                status.to_string(),
                updated_at.to_string(),
                id,
            ],
        )?;
        Ok(())
    }

    pub async fn update_task_progress(&self, id: i64, progress: f32, completed_items: i32, total_items: i32) -> Result<()> {
        let conn = self.conn.as_ref().unwrap();
        let conn = conn.lock().await;
        let updated_at = Utc::now();
        conn.execute(
            "UPDATE task SET progress = ?1, completed_items = ?2, total_items = ?3, updated_at = ?4 WHERE id = ?5",
            params![
                progress,
                completed_items,
                total_items,
                updated_at.to_string(),
                id,
            ],
        )?;
        Ok(())
    }

    pub async fn update_task_error(&self, id: i64, error_message: &str) -> Result<()> {
        let conn = self.conn.as_ref().unwrap();
        let conn = conn.lock().await;
        let updated_at = Utc::now();
        conn.execute(
            "UPDATE task SET status = ?1, error_message = ?2, updated_at = ?3 WHERE id = ?4",
            params![
                TaskStatus::Failed.to_string(),
                error_message,
                updated_at.to_string(),
                id,
            ],
        )?;
        Ok(())
    }

    pub async fn update_task_data(&self, id: i64, data: &str) -> Result<()> {
        let conn = self.conn.as_ref().unwrap();
        let conn = conn.lock().await;
        let updated_at = Utc::now();
        conn.execute(
            "UPDATE task SET data = ?1, updated_at = ?2 WHERE id = ?3",
            params![
                data,
                updated_at.to_string(),
                id,
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

pub async fn init_db(db_path: &str) {
    let mut db_manager = DB_MANAGER.lock().await;
    db_manager.connect(db_path).unwrap();
    match db_manager.init_db().await {
        Ok(_) => info!("Database initialized successfully."),
        Err(e) => info!("Failed to initialize database: {}", e),
    }
}

lazy_static::lazy_static! {
    pub static ref DB_MANAGER: Mutex<TaskDb> = Mutex::new(TaskDb::new());
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use tempfile::tempdir;

    // 创建测试任务的辅助函数
    fn create_test_task(name: &str) -> Task {
        Task {
            id: 0,
            name: name.to_string(),
            title: format!("Test Task {}", name),
            task_type: "test_type".to_string(),
            app_name: "test_app".to_string(),
            status: TaskStatus::Pending,
            progress: 0.0,
            total_items: 10,
            completed_items: 0,
            error_message: None,
            data: Some("{}".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    // 设置测试数据库的辅助函数
    async fn setup_test_db() -> (TaskDb, tempfile::TempDir) {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let db_path_str = db_path.to_str().unwrap();
        
        let mut db = TaskDb::new();
        db.connect(db_path_str).unwrap();
        db.init_db().await.unwrap();
        
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
        
        // 验证数据库文件已创建
        assert!(db_path.exists());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_create_and_get_task() {
        let (db, _temp_dir) = setup_test_db().await;
        
        let task = create_test_task("task1");
        let id = db.create_task(&task).await.unwrap();
        
        // 验证ID是否大于0
        assert!(id > 0);

        let task_1_again = create_test_task("task1");
        let id2 = db.create_task(&task_1_again).await;
        assert!(id2.is_err());
        
        // 获取并验证任务
        let retrieved_task = db.get_task(id).await.unwrap().unwrap();
        assert_eq!(retrieved_task.id, id as i32);
        assert_eq!(retrieved_task.name, "task1");
        assert_eq!(retrieved_task.title, "Test Task task1");
        assert_eq!(retrieved_task.status, TaskStatus::Pending);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_list_tasks() {
        let (db, _temp_dir) = setup_test_db().await;
        
        // 创建多个任务
        let task1 = create_test_task("task1");
        let task2 = create_test_task("task2");
        let task3 = create_test_task("task3");
        
        db.create_task(&task1).await.unwrap();
        db.create_task(&task2).await.unwrap();
        db.create_task(&task3).await.unwrap();
        
        // 列出所有任务
        let tasks = db.list_tasks().await.unwrap();
        assert_eq!(tasks.len(), 3);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_list_tasks_by_app() {
        let (db, _temp_dir) = setup_test_db().await;
        
        // 创建不同app的任务
        let mut task1 = create_test_task("task1");
        let mut task2 = create_test_task("task2");
        task1.app_name = "app1".to_string();
        task2.app_name = "app2".to_string();
        
        db.create_task(&task1).await.unwrap();
        db.create_task(&task2).await.unwrap();
        
        // 按app筛选
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
        
        // 创建不同类型的任务
        let mut task1 = create_test_task("task1");
        let mut task2 = create_test_task("task2");
        task1.task_type = "type1".to_string();
        task2.task_type = "type2".to_string();
        
        db.create_task(&task1).await.unwrap();
        db.create_task(&task2).await.unwrap();
        
        // 按类型筛选
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
        
        // 创建不同状态的任务
        let mut task1 = create_test_task("task1");
        let mut task2 = create_test_task("task2");
        task1.status = TaskStatus::Running;
        task2.status = TaskStatus::Completed;
        
        db.create_task(&task1).await.unwrap();
        db.create_task(&task2).await.unwrap();
        
        // 按状态筛选
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
        
        // 更新状态
        db.update_task_status(id, TaskStatus::Running).await.unwrap();
        
        // 验证状态已更新
        let updated_task = db.get_task(id).await.unwrap().unwrap();
        assert_eq!(updated_task.status, TaskStatus::Running);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_update_task_progress() {
        let (db, _temp_dir) = setup_test_db().await;
        
        let task = create_test_task("progress_test");
        let id = db.create_task(&task).await.unwrap();
        
        // 更新进度
        db.update_task_progress(id, 0.5, 5, 10).await.unwrap();
        
        // 验证进度已更新
        let updated_task = db.get_task(id).await.unwrap().unwrap();
        assert_eq!(updated_task.progress, 0.5);
        assert_eq!(updated_task.completed_items, 5);
        assert_eq!(updated_task.total_items, 10);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_update_task_error() {
        let (db, _temp_dir) = setup_test_db().await;
        
        let task = create_test_task("error_test");
        let id = db.create_task(&task).await.unwrap();
        
        // 更新错误信息
        db.update_task_error(id, "Test error message").await.unwrap();
        
        // 验证错误信息已更新
        let updated_task = db.get_task(id).await.unwrap().unwrap();
        assert_eq!(updated_task.status, TaskStatus::Failed);
        assert_eq!(updated_task.error_message, Some("Test error message".to_string()));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_update_task_data() {
        let (db, _temp_dir) = setup_test_db().await;
        
        let task = create_test_task("data_test");
        let id = db.create_task(&task).await.unwrap();
        
        // 更新数据
        let new_data = r#"{"key": "value"}"#;
        db.update_task_data(id, new_data).await.unwrap();
        
        // 验证数据已更新
        let updated_task = db.get_task(id).await.unwrap().unwrap();
        assert_eq!(updated_task.data, Some(new_data.to_string()));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_delete_task() {
        let (db, _temp_dir) = setup_test_db().await;
        
        let task = create_test_task("delete_test");
        let id = db.create_task(&task).await.unwrap();
        
        // 删除任务
        db.delete_task(id).await.unwrap();
        
        // 验证任务已删除
        let result = db.get_task(id).await.unwrap();
        assert!(result.is_none());
    }
}
