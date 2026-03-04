#![allow(dead_code)]

mod client;
mod constants;
mod read_manager;
mod reader;
mod upload;

#[macro_use]
extern crate log;

use crate::client::LogDaemonClient;
use crate::constants::*;
use std::path::PathBuf;

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

fn parse_env_positive_u64(env_key: &str, default_value: u64) -> u64 {
    match std::env::var(env_key) {
        Ok(v) if !v.trim().is_empty() => match v.trim().parse::<u64>() {
            Ok(v) if v > 0 => v,
            _ => {
                warn!(
                    "invalid {} value '{}', fallback to default {}",
                    env_key, v, default_value
                );
                default_value
            }
        },
        _ => default_value,
    }
}

#[tokio::main]
async fn main() {
    let node_id = read_env_or_default(SLOG_NODE_ID_ENV_KEY, DEFAULT_NODE_ID);
    let service_endpoint =
        read_env_or_default(SLOG_SERVER_ENDPOINT_ENV_KEY, DEFAULT_SERVER_ENDPOINT);
    let upload_timeout_secs =
        parse_env_positive_u64(SLOG_UPLOAD_TIMEOUT_SECS_ENV_KEY, DEFAULT_UPLOAD_TIMEOUT_SECS);
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
        "slog_daemon config: node_id={}, endpoint={}, log_dir={}, upload_timeout_secs={}",
        node_id,
        service_endpoint,
        log_dir.display(),
        upload_timeout_secs
    );

    // Specify excluded services
    // We should not upload logs from slog_daemon itself and slog_server
    let excluded = vec![SERVICE_NAME.to_string(), "slog_server".to_string()];
    let _client = match LogDaemonClient::new(
        node_id,
        service_endpoint,
        upload_timeout_secs,
        &log_dir,
        excluded,
    ) {
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
