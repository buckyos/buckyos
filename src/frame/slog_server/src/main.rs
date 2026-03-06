#![allow(dead_code)]

mod config;
mod server;
mod storage;

#[macro_use]
extern crate log;

use crate::server::LogHttpServer;
use crate::storage::create_log_storage_with_dir;
use config::{
    SLOG_SERVER_BIND_ENV_KEY, SLOG_STORAGE_DIR_ENV_KEY, SLOG_STORAGE_PARTITION_BUCKET_ENV_KEY,
    SLOG_STORAGE_PARTITION_MAX_ROWS_ENV_KEY, SLOG_STORAGE_PARTITION_MAX_SIZE_MB_ENV_KEY,
    SLOG_STORAGE_TYPE_ENV_KEY, ServerConfig, ServerEnvOverrides, StorageEngine,
};
use std::path::PathBuf;
use storage::PartitionBucket;

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

fn read_env_u64(env_key: &str) -> Option<u64> {
    let value = read_env_nonempty(env_key)?;
    match value.parse::<u64>() {
        Ok(v) => Some(v),
        Err(e) => {
            warn!(
                "ignore invalid env {}='{}': expected unsigned integer ({})",
                env_key, value, e
            );
            None
        }
    }
}

fn read_env_storage_engine(env_key: &str) -> Option<StorageEngine> {
    let value = read_env_nonempty(env_key)?;
    match StorageEngine::parse(&value) {
        Some(v) => Some(v),
        None => {
            warn!(
                "ignore invalid env {}='{}': expected sqlite or sqlite_partitioned",
                env_key, value
            );
            None
        }
    }
}

fn read_env_partition_bucket(env_key: &str) -> Option<PartitionBucket> {
    let value = read_env_nonempty(env_key)?;
    match PartitionBucket::parse(&value) {
        Some(v) => Some(v),
        None => {
            warn!("ignore invalid env {}='{}': expected day", env_key, value);
            None
        }
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
        storage_engine: read_env_storage_engine(SLOG_STORAGE_TYPE_ENV_KEY),
        partition_bucket: read_env_partition_bucket(SLOG_STORAGE_PARTITION_BUCKET_ENV_KEY),
        partition_max_rows: read_env_u64(SLOG_STORAGE_PARTITION_MAX_ROWS_ENV_KEY),
        partition_max_size_mb: read_env_u64(SLOG_STORAGE_PARTITION_MAX_SIZE_MB_ENV_KEY),
    };
    cfg.apply_env_overrides(env_overrides);

    let bind_addr = cfg.network.bind_addr;
    let storage_type = cfg.storage.to_storage_type();
    let storage_dir = cfg.storage.storage_dir;

    info!(
        "slog_server config: bind_addr={}, storage_dir={}, storage_engine={}, partition_bucket={}, partition_max_rows={}, partition_max_size_mb={}",
        bind_addr,
        storage_dir.display(),
        cfg.storage.storage_engine.as_str(),
        cfg.storage.partition.bucket.as_str(),
        cfg.storage.partition.max_rows_per_partition,
        cfg.storage.partition.max_partition_size_mb
    );

    let storage = match create_log_storage_with_dir(storage_type, &storage_dir) {
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
