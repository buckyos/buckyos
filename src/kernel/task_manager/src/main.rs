mod server;
mod task;
mod task_db;

use buckyos_kit::init_logging;
use log::error;
use server::start_task_manager_service;
use task_db::init_db;

#[tokio::main]
async fn main() {
    init_logging("task_manager", true);

    let db_path = "tasks.db";
    init_db(&db_path).await;
    if let Err(err) = start_task_manager_service().await {
        error!("task manager service exited with error: {:?}", err);
    }
}
