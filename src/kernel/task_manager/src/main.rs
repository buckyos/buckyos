mod download_executor;
mod server;
mod task;
mod task_db;

use buckyos_api::TASK_MANAGER_SERVICE_NAME;
use buckyos_kit::{get_buckyos_service_data_dir, init_logging};
use log::error;
use server::start_task_manager_service;

#[tokio::main]
async fn main() {
    init_logging("task_manager", true);

    // Make sure the data dir exists so the sqlite file (the default backend)
    // can be created on first use. The actual db path comes from the service
    // spec and is resolved inside start_task_manager_service.
    let data_dir = get_buckyos_service_data_dir(TASK_MANAGER_SERVICE_NAME);
    if let Err(err) = std::fs::create_dir_all(&data_dir) {
        error!(
            "failed to create task_manager data dir {}: {}",
            data_dir.display(),
            err
        );
        return;
    }

    if let Err(err) = start_task_manager_service().await {
        error!("task manager service exited with error: {:?}", err);
    }
}
