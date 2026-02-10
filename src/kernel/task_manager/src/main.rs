mod task_db;
mod server;
mod task;

use buckyos_kit::init_logging;
use log::error;
use task_db::init_db;
use server::start_task_manager_service;

#[tokio::main]
async fn main() {
    init_logging("task_manager",true);

    let db_path = "tasks.db";
    init_db(&db_path).await;
    if let Err(err) = start_task_manager_service().await {
        error!("task manager service exited with error: {:?}", err);
    }
}
