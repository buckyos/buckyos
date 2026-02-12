mod sqlite;
mod storage;

pub use storage::*;
// pub use sqlite::*;

use std::sync::Arc;

#[derive(Clone, Debug, Copy, PartialEq, Eq)]
pub enum LogStorageType {
    Sqlite,
}

pub fn create_log_storage(storage_type: LogStorageType) -> Result<LogStorageRef, String> {
    let root_dir = slog::get_buckyos_root_dir();
    let storage_dir = root_dir.join("slog_server");
    std::fs::create_dir_all(&storage_dir).map_err(|e| {
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
    }
}
