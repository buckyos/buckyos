use super::storage::{
    ObjectMapInnerStorage, ObjectMapInnerStorageType,
};
use super::db_storage::ObjectMapSqliteStorage;
use super::memory_storage::MemoryStorage;
use std::path::{Path, PathBuf};
use crate::{NdnResult, ObjId, NdnError};
use std::sync::atomic::{AtomicU64, Ordering};

pub struct ObjectMapStorageFactory {
    data_dir: PathBuf,
    storage_type: ObjectMapInnerStorageType,

    // Use to create a new file name for the storage, randomly generated and should be unique.
    temp_file_index: AtomicU64,
}

impl ObjectMapStorageFactory {
    pub fn new(data_dir: &Path, storage_type: Option<ObjectMapInnerStorageType>) -> Self {
        Self {
            data_dir: data_dir.to_path_buf(),
            storage_type: storage_type.unwrap_or(ObjectMapInnerStorageType::default()),
            temp_file_index: AtomicU64::new(0),
        }
    }

    pub async fn open_storage(&self, file_name: Option<&str>) -> NdnResult<Box<dyn ObjectMapInnerStorage>> {
        if !self.data_dir.exists() {
            std::fs::create_dir_all(&self.data_dir).map_err(|e| {
                let msg = format!(
                    "Error creating directory {}: {}",
                    self.data_dir.display(),
                    e
                );
                error!("{}", msg);
                NdnError::IoError(msg)
            })?;
        }

        match self.storage_type {
            ObjectMapInnerStorageType::Memory => Ok(Box::new(MemoryStorage::new())),
            ObjectMapInnerStorageType::SQLite => {
                let file = if let Some(file_name) = file_name {
                    self.data_dir.join(file_name)
                } else {
                    // Create a new file name for the storage
                    let temp_file_name = self.get_temp_file_name();
                    self.data_dir.join(&temp_file_name)
                };

                let storage = ObjectMapSqliteStorage::new(&file)?;
                Ok(Box::new(storage))
            }
        }
    }

    fn get_temp_file_name(&self) -> String {
        // Use index and time tick to create a unique file name.
        let index = self.temp_file_index.fetch_add(1, Ordering::SeqCst);
        format!("temp_{}_{}.sqlite", chrono::Utc::now().timestamp(), index)
    }
}