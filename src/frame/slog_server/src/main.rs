#![allow(dead_code)]

mod config;
mod server;
mod storage;

#[macro_use]
extern crate log;

use crate::server::LogHttpServer;
use crate::storage::{LogStorageType, create_log_storage_with_dir};
use config::{
    SLOG_SERVER_BIND_ENV_KEY, SLOG_STORAGE_DIR_ENV_KEY, ServerConfig, ServerEnvOverrides,
};
use std::path::PathBuf;

pub const SERVICE_NAME: &str = "slog_server";

fn read_env_nonempty(env_key: &str) -> Option<String> {
    match std::env::var(env_key) {
        Ok(v) if !v.trim().is_empty() => Some(v.trim().to_string()),
        _ => None,
    }
}

fn read_env_path(env_key: &str) -> Option<PathBuf> {
    match std::env::var(env_key) {
        Ok(v) if !v.trim().is_empty() => Some(PathBuf::from(v.trim())),
        _ => None,
    }
}

#[tokio::main]
async fn main() {
    // First init logs output
    let log_root_dir = slog::get_buckyos_log_root_dir();
    let log_dir = log_root_dir.join(SERVICE_NAME);
    if let Err(e) = std::fs::create_dir_all(&log_dir) {
        eprintln!(
            "Failed to create slog server log directory {}: {}",
            log_dir.display(),
            e
        );
        return;
    }

    let logger =
        slog::SystemLoggerBuilder::new(&log_dir, SERVICE_NAME, slog::SystemLoggerCategory::Service)
            .level("info")
            .console("debug")
            .file(true)
            .build();
    let logger = match logger {
        Ok(logger) => logger,
        Err(e) => {
            eprintln!("Failed to build slog server logger: {}", e);
            return;
        }
    };
    if let Err(e) = logger.start() {
        eprintln!("Failed to start slog server logger: {}", e);
        return;
    }

    let mut cfg = ServerConfig::default();
    let env_overrides = ServerEnvOverrides {
        bind_addr: read_env_nonempty(SLOG_SERVER_BIND_ENV_KEY),
        storage_dir: read_env_path(SLOG_STORAGE_DIR_ENV_KEY),
    };
    cfg.apply_env_overrides(env_overrides);

    let bind_addr = cfg.network.bind_addr;
    let storage_dir = cfg.storage.storage_dir;

    info!(
        "slog_server config: bind_addr={}, storage_dir={}",
        bind_addr,
        storage_dir.display()
    );

    let storage = match create_log_storage_with_dir(LogStorageType::Sqlite, &storage_dir) {
        Ok(s) => s,
        Err(e) => {
            error!("failed to create log storage: {}", e);
            return;
        }
    };

    let server = LogHttpServer::new(storage);
    info!("Starting slog server at http://{}", bind_addr);
    if let Err(e) = server.run(&bind_addr).await {
        error!("Log server exited with error: {}", e);
    }
}
