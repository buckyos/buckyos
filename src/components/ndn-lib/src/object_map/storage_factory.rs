use super::file::{ObjectMapJSONStorage, ObjectMapSqliteStorage};
use super::memory_storage::MemoryStorage;
use super::storage::{self, ObjectMapInnerStorage, ObjectMapStorageType};
use crate::{NdnError, NdnResult, ObjId};
use once_cell::sync::OnceCell;
use serde_json::de;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectMapStorageOpenMode {
    CreateNew,
    OpenExisting,
}

pub struct ObjectMapStorageFactory {
    data_dir: PathBuf,
    default_storage_type: ObjectMapStorageType,

    // Use to create a new file name for the storage, randomly generated and should be unique.
    temp_file_index: AtomicU64,
}

impl ObjectMapStorageFactory {
    pub fn new(data_dir: &Path, default_storage_type: Option<ObjectMapStorageType>) -> Self {
        Self {
            data_dir: data_dir.to_path_buf(),
            default_storage_type: default_storage_type.unwrap_or(ObjectMapStorageType::default()),
            temp_file_index: AtomicU64::new(0),
        }
    }

    // The storage type must not be Memory, as it does not have a file path.
    pub fn get_file_path_by_id(
        &self,
        root_hash: Option<&str>,
        storage_type: ObjectMapStorageType,
    ) -> PathBuf {
        let file_name = match storage_type {
            ObjectMapStorageType::Memory => {
                unreachable!("Memory storage does not have a file path");
            }
            ObjectMapStorageType::SQLite => {
                if let Some(hash) = root_hash {
                    hash.to_string()
                } else {
                    self.get_temp_file_name(storage_type)
                }
            }
            ObjectMapStorageType::JSONFile => {
                if let Some(hash) = root_hash {
                    hash.to_string()
                } else {
                    self.get_temp_file_name(storage_type)
                }
            }
        };

        self.get_file_path(&file_name, storage_type)
    }

    fn get_file_path(&self, file_name: &str, storage_type: ObjectMapStorageType) -> PathBuf {
        match storage_type {
            ObjectMapStorageType::Memory => {
                unreachable!("Memory storage does not have a file path");
            }
            ObjectMapStorageType::SQLite => {
                let file_name = format!("{}.sqlite", file_name);
                self.data_dir.join(file_name)
            }
            ObjectMapStorageType::JSONFile => {
                let file_name = format!("{}.json", file_name);

                self.data_dir.join(file_name)
            }
        }
    }

    pub async fn open(
        &self,
        root_hash: Option<&str>,
        read_only: bool,
        storage_type: Option<ObjectMapStorageType>,
        mode: ObjectMapStorageOpenMode,
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

        let storage_type = storage_type.unwrap_or(self.default_storage_type);
        if storage_type == ObjectMapStorageType::Memory {
            let msg = "Memory storage is not supported for open operation".to_string();
            error!("{}", msg);
            return Err(NdnError::PermissionDenied(msg));
        }

        let file = self.get_file_path_by_id(root_hash, storage_type);
        match mode {
            ObjectMapStorageOpenMode::CreateNew => {
                if file.exists() {
                    let msg = format!(
                        "File {} already exists, cannot create new storage",
                        file.display()
                    );
                    error!("{}", msg);
                    return Err(NdnError::AlreadyExists(msg));
                }
            }
            ObjectMapStorageOpenMode::OpenExisting => {
                if !file.exists() {
                    let msg = format!(
                        "File {} does not exist, cannot open storage",
                        file.display()
                    );
                    error!("{}", msg);
                    return Err(NdnError::NotFound(msg));
                }
            }
        }

        match storage_type {
            ObjectMapStorageType::Memory => {
                unreachable!("Memory storage does not have a file path");
            }
            ObjectMapStorageType::SQLite => {
                let storage = ObjectMapSqliteStorage::new(file, read_only)?;
                Ok(Box::new(storage))
            }
            ObjectMapStorageType::JSONFile => {
                let storage = ObjectMapJSONStorage::new(file, read_only)?;
                Ok(Box::new(storage))
            }
        }
    }

    pub async fn save(
        &self,
        root_hash: &str,
        storage: &mut dyn ObjectMapInnerStorage,
    ) -> NdnResult<()> {
        let file = self.get_file_path_by_id(Some(root_hash), storage.get_type());

        storage.save(&file).await
    }

    pub async fn clone(
        &self,
        root_hash: &str,
        storage: &dyn ObjectMapInnerStorage,
        read_only: bool,
    ) -> NdnResult<Box<dyn ObjectMapInnerStorage>> {
        let file_name = if read_only {
            root_hash.to_string()
        } else {
            let index = self.temp_file_index.fetch_add(1, Ordering::SeqCst);
            format!(
                "clone_{}_{}_{}.{}",
                root_hash,
                index,
                chrono::Utc::now().timestamp(),
                Self::get_file_ext(storage.get_type()),
            )
        };

        let file = self.get_file_path(&file_name, storage.get_type());
        storage.clone(&file, read_only).await
    }

    fn get_temp_file_name(&self, storage_type: ObjectMapStorageType) -> String {
        // Use index and time tick to create a unique file name.
        let index = self.temp_file_index.fetch_add(1, Ordering::SeqCst);

        let ext = Self::get_file_ext(storage_type);

        format!("temp_{}_{}.{}", chrono::Utc::now().timestamp(), index, ext)
    }

    fn get_file_ext(storage_type: ObjectMapStorageType) -> &'static str {
        match storage_type {
            ObjectMapStorageType::Memory => {
                unreachable!("Memory storage does not have a file extension")
            }
            ObjectMapStorageType::SQLite => "sqlite",
            ObjectMapStorageType::JSONFile => "json",
        }
    }
}

pub static GLOBAL_OBJECT_MAP_STORAGE_FACTORY: OnceCell<ObjectMapStorageFactory> = OnceCell::new();
