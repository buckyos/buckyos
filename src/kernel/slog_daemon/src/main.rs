#![allow(dead_code)]

mod client;
mod config;
mod constants;
mod read_manager;
mod reader;
#[cfg(test)]
mod test;
mod upload;

#[macro_use]
extern crate log;

use crate::client::LogDaemonClient;
use crate::config::{DaemonConfig, DaemonEnvOverrides};
use crate::constants::*;
use std::path::PathBuf;

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

fn parse_env_positive_u64(env_key: &str, default_value: u64) -> Option<u64> {
    match std::env::var(env_key) {
        Ok(v) if !v.trim().is_empty() => match v.trim().parse::<u64>() {
            Ok(v) if v > 0 => Some(v),
            _ => {
                warn!(
                    "invalid {} value '{}', fallback to default {}",
                    env_key, v, default_value
                );
                None
            }
        },
        _ => None,
    }
}

fn parse_env_positive_usize(env_key: &str, default_value: usize) -> Option<usize> {
    match std::env::var(env_key) {
        Ok(v) if !v.trim().is_empty() => match v.trim().parse::<usize>() {
            Ok(v) if v > 0 => Some(v),
            _ => {
                warn!(
                    "invalid {} value '{}', fallback to default {}",
                    env_key, v, default_value
                );
                None
            }
        },
        _ => None,
    }
}

#[tokio::main]
async fn main() {
    let mut cfg = DaemonConfig::default();
    let env_overrides = DaemonEnvOverrides {
        node_id: read_env_nonempty(SLOG_NODE_ID_ENV_KEY),
        server_endpoint: read_env_nonempty(SLOG_SERVER_ENDPOINT_ENV_KEY),
        log_dir: read_env_path(SLOG_LOG_DIR_ENV_KEY),
        upload_timeout_secs: parse_env_positive_u64(
            SLOG_UPLOAD_TIMEOUT_SECS_ENV_KEY,
            DEFAULT_UPLOAD_TIMEOUT_SECS,
        ),
        upload_global_concurrency: parse_env_positive_usize(
            SLOG_UPLOAD_GLOBAL_CONCURRENCY_ENV_KEY,
            DEFAULT_UPLOAD_GLOBAL_CONCURRENCY,
        ),
    };
    cfg.apply_env_overrides(env_overrides);

    let node_id = cfg.node.node_id;
    let service_endpoint = cfg.server.endpoint;
    let upload_timeout_secs = cfg.upload.timeout_secs;
    let upload_global_concurrency = cfg.upload.global_concurrency;
    let log_dir = cfg.path.log_dir;

    // Init slog daemon own logs, output to file and console
    let daemon_log_dir = log_dir.join(SERVICE_NAME);
    std::fs::create_dir_all(&daemon_log_dir).unwrap();
    if let Err(e) = slog::SystemLoggerBuilder::new(
        &daemon_log_dir,
        SERVICE_NAME,
        slog::SystemLoggerCategory::Service,
    )
    .level("debug")
    .console("debug")
    .file(true)
    .build()
    .unwrap()
    .start()
    {
        eprintln!("Failed to start slog daemon logger: {}", e);
        return;
    }

    info!(
        "slog_daemon config: node_id={}, endpoint={}, log_dir={}, upload_timeout_secs={}, upload_global_concurrency={}",
        node_id,
        service_endpoint,
        log_dir.display(),
        upload_timeout_secs,
        upload_global_concurrency
    );

    // Specify excluded services
    // We should not upload logs from slog_daemon itself and slog_server
    let excluded = vec![SERVICE_NAME.to_string(), "slog_server".to_string()];
    let client = match LogDaemonClient::new_with_upload_concurrency(
        node_id,
        service_endpoint,
        upload_timeout_secs,
        upload_global_concurrency,
        &log_dir,
        excluded,
    ) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to create LogDaemonClient: {}", e);
            return;
        }
    };

    match tokio::signal::ctrl_c().await {
        Ok(_) => {
            info!("received ctrl-c signal, shutting down slog daemon...");
        }
        Err(e) => {
            error!("failed to listen ctrl-c signal: {}", e);
        }
    }

    if let Err(e) = client.shutdown().await {
        error!("slog daemon client shutdown failed: {}", e);
    }
}
