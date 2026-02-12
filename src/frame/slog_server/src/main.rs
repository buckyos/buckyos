#![allow(dead_code)]

mod server;
mod storage;

#[macro_use]
extern crate log;

use crate::server::LogHttpServer;
use crate::storage::{LogStorageType, create_log_storage};

pub const SERVICE_NAME: &str = "slog_server";

#[tokio::main]
async fn main() {
    // First init logs output
    let log_root_dir = slog::get_buckyos_log_root_dir();
    let log_dir = log_root_dir.join(SERVICE_NAME);
    std::fs::create_dir_all(&log_dir).unwrap();

    let logger =
        slog::SystemLoggerBuilder::new(&log_dir, SERVICE_NAME, slog::SystemLoggerCategory::Service)
            .level("info")
            .console("debug")
            .file(true)
            .build()
            .unwrap();
    logger.start();

    let storage = match create_log_storage(LogStorageType::Sqlite) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to create log storage: {}", e);
            return;
        }
    };

    let server = LogHttpServer::new(storage);
    let addr = "127.0.0.1:8089";

    info!("Starting slog server at http://{}", addr);
    if let Err(e) = server.run(addr).await {
        error!("Log server exited with error: {}", e);
    }
}
