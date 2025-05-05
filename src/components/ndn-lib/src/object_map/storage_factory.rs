use super::db_storage::ObjectMapSqliteStorage;
use super::memory_storage::MemoryStorage;
use super::storage::{self, ObjectMapInnerStorage, ObjectMapInnerStorageType};
use crate::{NdnError, NdnResult, ObjId};
use once_cell::sync::OnceCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Once;

pub struct ObjectMapStorageFactory {
    data_dir: PathBuf,
    default_storage_type: ObjectMapInnerStorageType,

    // Use to create a new file name for the storage, randomly generated and should be unique.
    temp_file_index: AtomicU64,
}

impl ObjectMapStorageFactory {
    pub fn new(data_dir: &Path, default_storage_type: Option<ObjectMapInnerStorageType>) -> Self {
        Self {
            data_dir: data_dir.to_path_buf(),
            default_storage_type: default_storage_type.unwrap_or(ObjectMapInnerStorageType::default()),
            temp_file_index: AtomicU64::new(0),
        }
    }

    pub async fn open(
        &self,
        container_id: Option<&ObjId>,
        read_only: bool,
        storage_type: Option<ObjectMapInnerStorageType>,
    ) -> NdnResult<Box<dyn ObjectMapInnerStorage>> {
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

        let file_name = if let Some(id) = container_id {
            format!("{}.sqlite", id.to_base32())
        } else {
            let temp_file_name = self.get_temp_file_name();
            format!("{}.sqlite", temp_file_name)
        };

        let storage_type = storage_type.unwrap_or(self.default_storage_type);
        match storage_type {
            ObjectMapInnerStorageType::Memory => {
                let msg = "Memory storage is not supported for open operation".to_string();
                error!("{}", msg);
                Err(NdnError::PermissionDenied(msg))
            }
            ObjectMapInnerStorageType::SQLite => {
                let file = self.data_dir.join(&file_name);
                let storage = ObjectMapSqliteStorage::new(&file, read_only)?;
                Ok(Box::new(storage))
            }
        }
    }

    pub async fn save(&self, container_id: &ObjId, storage: &mut dyn ObjectMapInnerStorage) -> NdnResult<()> {
        let file_name = format!("{}.sqlite", container_id.to_base32());
        let file = self.data_dir.join(&file_name);
        
        storage.save(&file).await
    }

    pub async fn clone(
        &self,
        container_id: &ObjId,
        storage: &dyn ObjectMapInnerStorage,
        read_only: bool,
    ) -> NdnResult<Box<dyn ObjectMapInnerStorage>> {
        let file = if read_only {
            self.data_dir.join(format!("{}.sqlite", container_id.to_base32()))
        } else {
            let index = self.temp_file_index.fetch_add(1, Ordering::SeqCst);
            let file_name = format!("clone_{}_{}_{}.sqlite", container_id.to_base32(), index, chrono::Utc::now().timestamp());
            self.data_dir.join(&file_name)
        };
        
        storage.clone(&file, read_only).await
    }

    fn get_temp_file_name(&self) -> String {
        // Use index and time tick to create a unique file name.
        let index = self.temp_file_index.fetch_add(1, Ordering::SeqCst);
        format!("temp_{}_{}.sqlite", chrono::Utc::now().timestamp(), index)
    }
}

pub static GLOBAL_OBJECT_MAP_STORAGE_FACTORY: OnceCell<ObjectMapStorageFactory> = OnceCell::new();
