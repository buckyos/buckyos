use rusqlite::{params, Connection, Result};
use crate::task::{Task, TaskStatus};
use chrono::{NaiveDateTime, TimeZone, Utc};
use log::*;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Mutex;



pub struct DatabaseManager {
    conn: Option<Arc<Mutex<Connection>>>,
}

impl DatabaseManager {
    pub fn new() -> Self {
        DatabaseManager { conn: None }
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
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                name        TEXT NOT NULL,
                app_name    TEXT NOT NULL,
                status      TEXT NOT NULL,
                created_at  TEXT NOT NULL,
                updated_at  TEXT NOT NULL
            )",
            params![],
        )?;
        Ok(())
    }

    // TODO: Add other database operation methods here, such as add_task, get_task, etc.
    pub async fn create_task(&self, task: &Task) -> Result<()> {
        let conn = self.conn.as_ref().unwrap();
        let conn = conn.lock().await;
        conn.execute(
            "INSERT INTO task (name, app_name, status, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                task.name, 
                task.app_name,
                task.status.to_string(), 
                task.created_at.to_string(), 
                task.updated_at.to_string(), 
                ],

        )?;
        Ok(())
    }

    pub async fn list_tasks(&self) -> rusqlite::Result<Vec<Task>> {
        let conn = self.conn.as_ref().unwrap();
        let conn = conn.lock().await;
        let mut stmt = conn.prepare("SELECT * FROM task")?;
        let task_iter = stmt.query_map([], |row| {
            let format = "%Y-%m-%d %H:%M:%S%.f UTC";

            let id = row.get(0)?;
            let name: String = row.get(1)?;
            let app_name: String = row.get(2)?;
            let status: String = row.get(3)?;
            let created_at: String = row.get(4)?;
            let created_at = NaiveDateTime::parse_from_str(&created_at, format).unwrap();
            let updated_at: String = row.get(5)?;
            let updated_at = NaiveDateTime::parse_from_str(&updated_at, format).unwrap();
            Ok(Task {
                id:id,
                name: name,
                app_name: app_name,
                status: TaskStatus::from_str(status.as_str()).unwrap(),
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

    // update task status
    pub async fn update_task(&self, id: u64, status: String) -> Result<()> {
        let conn = self.conn.as_ref().unwrap();
        let conn = conn.lock().await;
        let updated_at = chrono::Utc::now();
        conn.execute(
            "UPDATE task SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![
                status,
                updated_at.to_string(),
                id,
            ],
        )?;
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
    pub static ref DB_MANAGER: Mutex<DatabaseManager> = Mutex::new(DatabaseManager::new());
}
