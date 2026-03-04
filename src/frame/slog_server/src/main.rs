#![allow(dead_code)]

mod server;
mod storage;

#[macro_use]
extern crate log;

use crate::server::LogHttpServer;
use crate::storage::{LogStorageType, create_log_storage_with_dir};
use std::path::PathBuf;

pub const SERVICE_NAME: &str = "slog_server";
const SLOG_SERVER_BIND_ENV_KEY: &str = "SLOG_SERVER_BIND";
const SLOG_STORAGE_DIR_ENV_KEY: &str = "SLOG_STORAGE_DIR";
const DEFAULT_SERVER_BIND: &str = "127.0.0.1:8089";

fn read_env_nonempty(env_key: &str) -> Option<String> {
    match std::env::var(env_key) {
        Ok(v) if !v.trim().is_empty() => Some(v.trim().to_string()),
        _ => None,
    }
}

fn resolve_bind_addr() -> String {
    read_env_nonempty(SLOG_SERVER_BIND_ENV_KEY).unwrap_or_else(|| DEFAULT_SERVER_BIND.to_string())
}

fn resolve_storage_dir() -> PathBuf {
    match read_env_nonempty(SLOG_STORAGE_DIR_ENV_KEY) {
        Some(v) => PathBuf::from(v),
        None => slog::get_buckyos_root_dir().join("slog_server"),
    }
}

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

    let bind_addr = resolve_bind_addr();
    let storage_dir = resolve_storage_dir();

    info!(
        "slog_server config: bind_addr={}, storage_dir={}",
        bind_addr,
        storage_dir.display()
    );

    let storage = match create_log_storage_with_dir(LogStorageType::Sqlite, &storage_dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to create log storage: {}", e);
            return;
        }
    };

    let server = LogHttpServer::new(storage);
    info!("Starting slog server at http://{}", bind_addr);
    if let Err(e) = server.run(&bind_addr).await {
        error!("Log server exited with error: {}", e);
    }
}
