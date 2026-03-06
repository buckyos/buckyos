mod sqlite;
mod sqlite_partitioned;
mod storage;

pub use sqlite_partitioned::{PartitionBucket, SqlitePartitionedConfig};
pub use storage::*;
// pub use sqlite::*;

use std::path::Path;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogStorageType {
    Sqlite,
    SqlitePartitioned(SqlitePartitionedConfig),
}

pub fn create_log_storage(storage_type: LogStorageType) -> Result<LogStorageRef, String> {
    let storage_dir = slog::get_buckyos_root_dir().join("slog_server");
    create_log_storage_with_dir(storage_type, &storage_dir)
}

pub fn create_log_storage_with_dir(
    storage_type: LogStorageType,
    storage_dir: &Path,
) -> Result<LogStorageRef, String> {
    std::fs::create_dir_all(storage_dir).map_err(|e| {
        let msg = format!(
            "Failed to create storage directory {:?}: {}",
            storage_dir, e
        );
        error!("{}", msg);
        msg
    })?;

    match storage_type {
        LogStorageType::Sqlite => {
            let logs_file = storage_dir.join("logs.db");

            let storage = sqlite::SqliteLogStorage::open(&logs_file)?;
            Ok(Arc::new(Box::new(storage)))
        }
        LogStorageType::SqlitePartitioned(config) => {
            let storage =
                sqlite_partitioned::SqlitePartitionedLogStorage::open(storage_dir, config)?;
            Ok(Arc::new(Box::new(storage)))
        }
    }
}
