use crate::file_storage::{FileStorage, FileStorageQuerier};

pub struct FileStorageSqlite {
    connection: rusqlite::Connection,
}

impl FileStorageQuerier for FileStorageSqlite {}

impl FileStorage for FileStorageSqlite {}
