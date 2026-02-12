mod server;
mod task;
mod task_db;

use buckyos_api::TASK_MANAGER_SERVICE_NAME;
use buckyos_kit::{get_buckyos_service_data_dir, init_logging};
use log::error;
use server::start_task_manager_service;
use task_db::init_db;

#[tokio::main]
async fn main() {
    init_logging("task_manager", true);

    let data_dir = get_buckyos_service_data_dir(TASK_MANAGER_SERVICE_NAME);
    if let Err(err) = std::fs::create_dir_all(&data_dir) {
        error!(
            "failed to create task_manager data dir {}: {}",
            data_dir.display(),
            err
        );
        return;
    }

    let db_path = data_dir.join("tasks.db");
    let db_path_str = db_path.to_string_lossy().to_string();
    init_db(db_path_str.as_str()).await;
    if let Err(err) = start_task_manager_service().await {
        error!("task manager service exited with error: {:?}", err);
    }
}
