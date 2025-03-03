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

    pub async fn create_task(&self, task: &Task) -> Result<i32> {
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
        let id = conn.last_insert_rowid() as i32;
        Ok(id)
    }

    pub async fn get_task(&self, id: i32) -> rusqlite::Result<Option<Task>> {
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

    pub async fn update_task_status(&self, id: i32, status: TaskStatus) -> Result<()> {
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

    pub async fn update_task_progress(&self, id: i32, progress: f32, completed_items: i32, total_items: i32) -> Result<()> {
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

    pub async fn update_task_error(&self, id: i32, error_message: &str) -> Result<()> {
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

    pub async fn update_task_data(&self, id: i32, data: &str) -> Result<()> {
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

    pub async fn delete_task(&self, id: i32) -> Result<()> {
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
