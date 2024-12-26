mod server;
mod task;

use rusqlite::{params, Connection, Result};
// use serde::{Deserialize, Serialize};
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use std::str::FromStr;
use std::sync::Arc;
use task::{Task, TaskStatus};
use tokio::sync::Mutex;
use warp::{reply::Json, Filter};

#[derive(Debug)]
struct CustomReject {
    message: String,
}

impl warp::reject::Reject for CustomReject {}

pub struct DatabaseManager {
    conn: Arc<Mutex<Connection>>,
}

impl DatabaseManager {
    pub fn new(db_path: &str) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        Ok(DatabaseManager {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub async fn init_db(&self) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock().await;
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
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO task (id, name, status, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![task.id, task.name, task.status.to_string(), task.created_at.to_string(), task.updated_at.to_string()],
        )?;
        Ok(())
    }

    pub async fn list_tasks(&self) -> rusqlite::Result<Vec<Task>> {
        let conn = self.conn.lock().await;
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

async fn create_task_handler(
    new_task: Task,
    db_manager: Arc<Mutex<DatabaseManager>>,
) -> Result<Json, warp::Rejection> {
    let mut db_manager = db_manager.lock().await;
    match db_manager.create_task(&new_task).await {
        Ok(_) => Ok(warp::reply::json(&new_task)),
        Err(_) => Err(warp::reject::not_found()),
    }
}

async fn list_tasks_handler(
    db_manager: Arc<Mutex<DatabaseManager>>,
) -> Result<impl warp::Reply, warp::Rejection> {
    let db_manager = db_manager.lock().await;
    match db_manager.list_tasks().await {
        Ok(tasks) => Ok(warp::reply::json(&tasks)),
        Err(e) => {
            eprintln!("Error listing tasks: {}", e);
            let reject = CustomReject {
                message: format!("Error listing tasks: {}", e),
            };
            Err(warp::reject::custom(reject))
        }
    }
}

async fn get_task_info_handler(id: String) -> Result<impl warp::Reply, warp::Rejection> {
    Ok(format!("Get task info: {}", id))
}

async fn resume_backup_task_handler(id: String) -> Result<impl warp::Reply, warp::Rejection> {
    Ok(format!("Resume backup task: {}", id))
}

async fn pause_backup_task_handler(id: String) -> Result<impl warp::Reply, warp::Rejection> {
    Ok(format!("Pause backup task: {}", id))
}

async fn validate_path_handler() -> Result<impl warp::Reply, warp::Rejection> {
    Ok("Validate path")
}

async fn paused_handler() -> Result<impl warp::Reply, warp::Rejection> {
    Ok("Paused")
}

async fn running_handler() -> Result<impl warp::Reply, warp::Rejection> {
    Ok("Running")
}

#[tokio::main]
async fn main() {
    let db_path = "tasks.db";

    let db_manager = match DatabaseManager::new(db_path) {
        Ok(manager) => manager,
        Err(e) => {
            println!("Failed to create database manager: {}", e);
            return;
        }
    };
    match db_manager.init_db().await {
        Ok(_) => println!("Database initialized successfully."),
        Err(e) => println!("Failed to initialize database: {}", e),
    }
    let db_manager = Arc::new(Mutex::new(db_manager));

    let get_task_info = warp::path("get_task_info")
        .and(warp::path::param())
        .and_then(get_task_info_handler);

    let resume_backup_task = warp::path("resume_backup_task")
        .and(warp::path::param())
        .and(warp::post())
        .and_then(resume_backup_task_handler);

    let pause_backup_task = warp::path("pause_backup_task")
        .and(warp::path::param())
        .and(warp::post())
        .and_then(pause_backup_task_handler);

    let validate_path = warp::path("validate_path")
        .and(warp::post())
        .and_then(validate_path_handler);

    let paused = warp::path("paused").and_then(paused_handler);

    let running = warp::path("running").and_then(running_handler);

    // let routes = list_backup_task
    //     .or(get_task_info)
    //     .or(resume_backup_task)
    //     .or(pause_backup_task)
    //     .or(validate_path)
    //     .or(paused)
    //     .or(running)
    //     .or(create_task);

    let db_manager_for_create_task = db_manager.clone();
    let create_task = warp::path!("task")
        .and(warp::post())
        .and(warp::body::json())
        .and(warp::any().map(move || db_manager_for_create_task.clone()))
        .and_then(create_task_handler);

    let db_manager_for_list_tasks = db_manager.clone();
    let list_tasks_route = warp::path!("tasks")
        .and(warp::get())
        .and(warp::any().map(move || db_manager_for_list_tasks.clone()))
        .and_then(list_tasks_handler);
    let routes = create_task.or(list_tasks_route);

    warp::serve(routes).run(([127, 0, 0, 1], 3033)).await;
}
