mod database_manager;
mod server;
mod task;

use rusqlite::{params, Connection, Result};
// use serde::{Deserialize, Serialize};
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use database_manager::{DatabaseManager, DB_MANAGER};
use server::start_task_manager_service;
use std::str::FromStr;
use std::sync::Arc;
use task::{Task, TaskStatus};
use tokio::sync::Mutex;
use warp::{reply::Json, Filter};

// #[derive(Debug)]
// struct CustomReject {
//     message: String,
// }
// impl warp::reject::Reject for CustomReject {}

// async fn create_task_handler(
//     new_task: Task,
//     db_manager: Arc<Mutex<DatabaseManager>>,
// ) -> Result<Json, warp::Rejection> {
//     let mut db_manager = db_manager.lock().await;
//     match db_manager.create_task(&new_task).await {
//         Ok(_) => Ok(warp::reply::json(&new_task)),
//         Err(_) => Err(warp::reject::not_found()),
//     }
// }

// async fn list_tasks_handler(
//     db_manager: Arc<Mutex<DatabaseManager>>,
// ) -> Result<impl warp::Reply, warp::Rejection> {
//     let db_manager = db_manager.lock().await;
//     match db_manager.list_tasks().await {
//         Ok(tasks) => Ok(warp::reply::json(&tasks)),
//         Err(e) => {
//             eprintln!("Error listing tasks: {}", e);
//             let reject = CustomReject {
//                 message: format!("Error listing tasks: {}", e),
//             };
//             Err(warp::reject::custom(reject))
//         }
//     }
// }
// async fn get_task_info_handler(id: String) -> Result<impl warp::Reply, warp::Rejection> {
//     Ok(format!("Get task info: {}", id))
// }

// async fn resume_backup_task_handler(id: String) -> Result<impl warp::Reply, warp::Rejection> {
//     Ok(format!("Resume backup task: {}", id))
// }

// async fn pause_backup_task_handler(id: String) -> Result<impl warp::Reply, warp::Rejection> {
//     Ok(format!("Pause backup task: {}", id))
// }

// async fn validate_path_handler() -> Result<impl warp::Reply, warp::Rejection> {
//     Ok("Validate path")
// }

// async fn paused_handler() -> Result<impl warp::Reply, warp::Rejection> {
//     Ok("Paused")
// }

// async fn running_handler() -> Result<impl warp::Reply, warp::Rejection> {
//     Ok("Running")
// }

#[tokio::main]
async fn main() {
    let db_path = "tasks.db";
    let mut db_manager = DB_MANAGER.lock().await;
    db_manager.connect(db_path).unwrap();
    match db_manager.init_db().await {
        Ok(_) => println!("Database initialized successfully."),
        Err(e) => println!("Failed to initialize database: {}", e),
    }
    // let get_task_info = warp::path("get_task_info")
    //     .and(warp::path::param())
    //     .and_then(get_task_info_handler);
    // let resume_backup_task = warp::path("resume_backup_task")
    //     .and(warp::path::param())
    //     .and(warp::post())
    //     .and_then(resume_backup_task_handler);
    // let pause_backup_task = warp::path("pause_backup_task")
    //     .and(warp::path::param())
    //     .and(warp::post())
    //     .and_then(pause_backup_task_handler);
    // let validate_path = warp::path("validate_path")
    //     .and(warp::post())
    //     .and_then(validate_path_handler);
    // let paused = warp::path("paused").and_then(paused_handler);
    // let running = warp::path("running").and_then(running_handler);
    // // let routes = list_backup_task
    // //     .or(get_task_info)
    // //     .or(resume_backup_task)
    // //     .or(pause_backup_task)
    // //     .or(validate_path)
    // //     .or(paused)
    // //     .or(running)
    // //     .or(create_task);
    // let db_manager_for_create_task = db_manager.clone();
    // let create_task = warp::path!("task")
    //     .and(warp::post())
    //     .and(warp::body::json())
    //     .and(warp::any().map(move || db_manager_for_create_task.clone()))
    //     .and_then(create_task_handler);
    // let db_manager_for_list_tasks = db_manager.clone();
    // let list_tasks_route = warp::path!("tasks")
    //     .and(warp::get())
    //     .and(warp::any().map(move || db_manager_for_list_tasks.clone()))
    //     .and_then(list_tasks_handler);
    // let routes = create_task.or(list_tasks_route);
    // warp::serve(routes).run(([127, 0, 0, 1], 3033)).await;

    start_task_manager_service().await;
}
