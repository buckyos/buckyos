#![allow(dead_code)]

mod client;
mod read_manager;
mod reader;
mod upload;

#[macro_use]
extern crate log;

use crate::client::LogDaemonClient;
use std::path::PathBuf;

const SERVICE_NAME: &str = "slog_daemon";
const SLOG_NODE_ID_ENV_KEY: &str = "SLOG_NODE_ID";
const SLOG_SERVER_ENDPOINT_ENV_KEY: &str = "SLOG_SERVER_ENDPOINT";
const SLOG_LOG_DIR_ENV_KEY: &str = "SLOG_LOG_DIR";
const DEFAULT_NODE_ID: &str = "node-001";
const DEFAULT_SERVER_ENDPOINT: &str = "http://127.0.0.1:8089/logs";

fn read_env_or_default(env_key: &str, default_value: &str) -> String {
    match std::env::var(env_key) {
        Ok(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => default_value.to_string(),
    }
}

fn resolve_log_dir() -> PathBuf {
    match std::env::var(SLOG_LOG_DIR_ENV_KEY) {
        Ok(v) if !v.trim().is_empty() => PathBuf::from(v.trim()),
        _ => slog::get_buckyos_log_root_dir(),
    }
}

#[tokio::main]
async fn main() {
    let node_id = read_env_or_default(SLOG_NODE_ID_ENV_KEY, DEFAULT_NODE_ID);
    let service_endpoint =
        read_env_or_default(SLOG_SERVER_ENDPOINT_ENV_KEY, DEFAULT_SERVER_ENDPOINT);
    let log_dir = resolve_log_dir();

    // Init slog daemon own logs, output to file and console
    let daemon_log_dir = log_dir.join(SERVICE_NAME);
    std::fs::create_dir_all(&daemon_log_dir).unwrap();
    slog::SystemLoggerBuilder::new(
        &daemon_log_dir,
        SERVICE_NAME,
        slog::SystemLoggerCategory::Service,
    )
    .level("debug")
    .console("debug")
    .file(true)
    .build()
    .unwrap()
    .start();

    info!(
        "slog_daemon config: node_id={}, endpoint={}, log_dir={}",
        node_id,
        service_endpoint,
        log_dir.display()
    );

    // Specify excluded services
    // We should not upload logs from slog_daemon itself and slog_server
    let excluded = vec![SERVICE_NAME.to_string(), "slog_server".to_string()];
    let _client = match LogDaemonClient::new(node_id, service_endpoint, &log_dir, excluded) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to create LogDaemonClient: {}", e);
            return;
        }
    };

    // Sleep indefinitely (or implement actual logic)
    tokio::signal::ctrl_c().await.unwrap();

    info!("Log daemon client exiting.");
}
