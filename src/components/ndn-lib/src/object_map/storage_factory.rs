use super::file::{ObjectMapJSONStorage, ObjectMapSqliteStorage};
use super::memory_storage::MemoryStorage;
use super::storage::{self, ObjectMapInnerStorage, ObjectMapStorageType};
use crate::{NdnError, NdnResult, ObjId};
use once_cell::sync::OnceCell;
use serde_json::de;
use std::clone;
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
        obj_id: Option<&ObjId>,
        storage_type: ObjectMapStorageType,
    ) -> PathBuf {
        let file_name = match storage_type {
            ObjectMapStorageType::Memory => {
                unreachable!("Memory storage does not have a file path");
            }
            ObjectMapStorageType::SQLite => {
                if let Some(obj_id) = obj_id {
                    obj_id.to_base32()
                } else {
                    self.get_temp_file_name(storage_type)
                }
            }
            ObjectMapStorageType::JSONFile => {
                if let Some(obj_id) = obj_id {
                    obj_id.to_base32()
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

    fn get_clone_file_name(&self, obj_id: &ObjId, storage_type: ObjectMapStorageType) -> String {
        let index = self.temp_file_index.fetch_add(1, Ordering::SeqCst);
        format!(
            "clone_{}_{}_{}.{}",
            obj_id.to_base32(),
            index,
            chrono::Utc::now().timestamp(),
            Self::get_file_ext(storage_type),
        )
    }

    pub async fn open(
        &self,
        obj_id: Option<&ObjId>,
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

        let mut file = self.get_file_path_by_id(obj_id, storage_type);
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

                if !read_only {
                    // If we are not in read-only mode, we need to clone the file to a temporary file
                    let clone_file_name =
                        self.get_clone_file_name(obj_id.expect("obj_id is None"), storage_type);
                    let clone_file_path = self.get_file_path(&clone_file_name, storage_type);

                    if clone_file_path.exists() {
                        let msg = format!(
                            "Clone file {} already exists, cannot create new storage",
                            clone_file_path.display()
                        );
                        error!("{}", msg);
                        return Err(NdnError::AlreadyExists(msg));
                    }

                    // Clone the file to a temporary file
                    tokio::fs::copy(&file, &clone_file_path)
                        .await
                        .map_err(|e| {
                            let msg = format!(
                                "Error copying file {} to {}: {}",
                                file.display(),
                                clone_file_path.display(),
                                e
                            );
                            error!("{}", msg);
                            NdnError::IoError(msg)
                        })?;

                    file = clone_file_path;
                    info!(
                        "Cloned file to {} for modify {}",
                        file.display(),
                        obj_id.unwrap().to_base32()
                    );
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
        obj_id: &ObjId,
        storage: &mut dyn ObjectMapInnerStorage,
    ) -> NdnResult<()> {
        let file = self.get_file_path_by_id(Some(obj_id), storage.get_type());

        storage.save(&file).await
    }

    pub async fn clone(
        &self,
        obj_id: &ObjId,
        storage: &dyn ObjectMapInnerStorage,
        read_only: bool,
    ) -> NdnResult<Box<dyn ObjectMapInnerStorage>> {
        let file_name = if read_only {
            obj_id.to_base32()
        } else {
            self.get_clone_file_name(obj_id, storage.get_type())
        };

        let file = self.get_file_path(&file_name, storage.get_type());
        storage.clone(&file, read_only).await
    }

    pub async fn switch_storage(
        &self,
        obj_id: &ObjId,
        storage: Box<dyn ObjectMapInnerStorage>,
        new_storage_type: ObjectMapStorageType,
    ) -> NdnResult<Box<dyn ObjectMapInnerStorage>> {
        let old_storage_type = storage.get_type();
        assert_ne!(
            old_storage_type, new_storage_type,
            "Cannot switch to the same storage type"
        );

        let mut new_storage = self
            .open(
                Some(obj_id),
                false,
                Some(new_storage_type),
                ObjectMapStorageOpenMode::CreateNew,
            )
            .await?;

        for item in storage.iter() {
            new_storage.put_with_index(&item.0, &item.1, item.2).await?;
        }

        // Save the new storage to the file
        self.save(obj_id, &mut *new_storage).await?;

        drop(storage);

        // Remove the old storage file if it exists
        let old_file = self.get_file_path_by_id(Some(obj_id), old_storage_type);
        if old_file.exists() {
            let ret = std::fs::remove_file(&old_file);
            if let Err(e) = ret {
                let msg = format!(
                    "Error removing old storage file {}: {}",
                    old_file.display(),
                    e
                );
                warn!("{}", msg);
                // FIXME: Should we return an error here? or we can remove the file later in GC?
            }
        }

        info!(
            "Switched object map storage for {} from {:?} to {:?}",
            obj_id.to_base32(),
            old_storage_type,
            new_storage_type,
        );

        Ok(new_storage)
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
