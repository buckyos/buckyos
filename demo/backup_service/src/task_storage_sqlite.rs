use backup_lib::{TaskStorage, TaskStorageQuerier, Transaction};

pub struct TaskStorageSqlite {}

impl TaskStorageSqlite {
    pub(crate) fn new(db_path: &Path) -> Self {
        Self {}
    }
}

impl Transaction for TaskStorageSqlite {}

impl TaskStorageQuerier for TaskStorageSqlite {}

impl TaskStorage for TaskStorageSqlite {}

impl TaskStorageClient for TaskStorageSqlite {}
