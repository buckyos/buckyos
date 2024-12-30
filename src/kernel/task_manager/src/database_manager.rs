use rusqlite::{params, Connection, Result};
// use serde::{Deserialize, Serialize};
use crate::task::{Task, TaskStatus};
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Mutex;
use warp::{reply::Json, Filter};

#[derive(Debug)]
struct CustomReject {
    message: String,
}

impl warp::reject::Reject for CustomReject {}

pub struct DatabaseManager {
    conn: Option<Arc<Mutex<Connection>>>,
}

impl DatabaseManager {
    pub fn new() -> Self {
        // let conn: Connection = Connection::open(db_path)?;
        // Ok(
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
                id          TEXT PRIMARY KEY,
                name        TEXT NOT NULL,
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
            "INSERT INTO task (id, name, status, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![task.id, task.name, task.status.to_string(), task.created_at.to_string(), task.updated_at.to_string()],
        )?;
        Ok(())
    }

    pub async fn list_tasks(&self) -> rusqlite::Result<Vec<Task>> {
        let conn = self.conn.as_ref().unwrap();
        let conn = conn.lock().await;
        let mut stmt = conn.prepare("SELECT * FROM tasks")?;
        let task_iter = stmt.query_map([], |row| {
            let status: String = row.get(2)?;
            let created_at: String = row.get(3)?;
            let created_at =
                NaiveDateTime::parse_from_str(&created_at, "%Y-%m-%d %H:%M:%S").unwrap();
            let updated_at: String = row.get(4)?;
            let updated_at =
                NaiveDateTime::parse_from_str(&updated_at, "%Y-%m-%d %H:%M:%S").unwrap();
            Ok(Task {
                id: row.get(0)?,
                name: row.get(1)?,
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
}

lazy_static::lazy_static! {
    pub static ref DB_MANAGER: Mutex<DatabaseManager> = Mutex::new(DatabaseManager::new());
}
