mod database_manager;
mod server;
mod task;

use buckyos_kit::init_logging;
use database_manager::init_db;
use server::start_task_manager_service;

#[tokio::main]
async fn main() {
    init_logging("task_manager");

    let db_path = "tasks.db";
    init_db(&db_path).await;
    start_task_manager_service().await;
}
