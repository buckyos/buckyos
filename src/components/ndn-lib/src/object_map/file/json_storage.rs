use super::super::storage::{
    ObjectMapInnerStorage, ObjectMapInnerStorageStat, ObjectMapStorageType,
};
use crate::{NdnError, NdnResult, ObjId};
use http_types::content;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use super::memory_storage::{ObjectMapMemoryStorage, JSONStorageData};


pub struct ObjectMapJSONStorage {
    read_only: bool,
    file: PathBuf,

    storage: ObjectMapMemoryStorage,
}

impl ObjectMapJSONStorage {
    pub fn new(file: PathBuf, read_only: bool) -> NdnResult<Self> {
        let data = if file.exists() {
            let data = Self::load(&file)?;
            Some(data)
        } else {
            None
        };

        Ok(Self {
            read_only,
            file,
            storage: ObjectMapMemoryStorage::new_raw(data, read_only),
        })
    }

    fn load(file_path: &Path) -> NdnResult<JSONStorageData> {
        if !file_path.exists() {
            let msg = format!("File does not exist: {:?}", file_path);
            error!("{}", msg);
            return Err(NdnError::IoError(msg));
        }

        let file = std::fs::File::open(&file_path).map_err(|e| {
            let msg = format!("Failed to open file: {:?}, {}", file_path, e);
            error!("{}", msg);
            NdnError::IoError(msg)
        })?;

        // Deserialize the data from JSON
        let data: JSONStorageData = serde_json::from_reader(file).map_err(|e| {
            let msg = format!("Failed to read data from file: {:?}, {}", file_path, e);
            error!("{}", msg);
            NdnError::IoError(msg)
        })?;

        Ok(data)
    }

    async fn clone_for_modify(&self, target: &Path) -> NdnResult<Box<dyn ObjectMapInnerStorage>> {
        // First check if target is same as current file
        if target == self.file {
            let msg = format!("Target file is same as current file: {}", target.display());
            error!("{}", msg);
            return Err(NdnError::AlreadyExists(msg));
        }

        if self.storage.is_dirty() {
            // Clone the current storage to a new file
            let mut new_storage = Self {
                read_only: false,
                file: target.to_path_buf(),
                storage: self.storage.clone_for_modify(),
            };

            new_storage.save(&target).await?;
            Ok(Box::new(new_storage))
        } else {
            // If the storage is not dirty, just copy the file
            tokio::fs::copy(&self.file, target).await.map_err(|e| {
                let msg = format!("Failed to copy file: {:?}, {}", target, e);
                error!("{}", msg);
                NdnError::IoError(msg)
            })?;

            // Create a new storage instance
            let new_storage = Self {
                read_only: false,
                file: target.to_path_buf(),
                storage: self.storage.clone_for_modify(),
            };

            // Return the new storage
            Ok(Box::new(new_storage))
        }
    }
}

#[async_trait::async_trait]
impl ObjectMapInnerStorage for ObjectMapJSONStorage {
    fn get_type(&self) -> ObjectMapStorageType {
        ObjectMapStorageType::JSONFile
    }

    fn is_readonly(&self) -> bool {
        self.read_only
    }

    fn put(&mut self, key: &str, value: &ObjId) -> NdnResult<()> {
        self.storage.put(key, value)
    }

    fn put_with_index(&mut self, key: &str, value: &ObjId, index: Option<u64>) -> NdnResult<()> {
        self.storage.put_with_index(key, value, index)
    }

    fn get(&self, key: &str) -> NdnResult<Option<(ObjId, Option<u64>)>> {
        self.storage.get(key)
    }

    fn remove(&mut self, key: &str) -> NdnResult<Option<ObjId>> {
        self.storage.remove(key)
    }

    fn is_exist(&self, key: &str) -> NdnResult<bool> {
        self.storage.is_exist(key)
    }

    fn list(&self, page_index: usize, page_size: usize) -> NdnResult<Vec<String>> {
        self.storage.list(page_index, page_size)
    }

    fn stat(&self) -> NdnResult<ObjectMapInnerStorageStat> {
        self.storage.stat()
    }

    fn iter<'a>(&'a self) -> Box<dyn Iterator<Item = (String, ObjId, Option<u64>)> + 'a> {
        self.storage.iter()
    }

    // Use to store the index of the mtree node
    fn update_mtree_index(&mut self, key: &str, index: u64) -> NdnResult<()> {
        self.storage.update_mtree_index(key, index)
    }

    fn get_mtree_index(&self, key: &str) -> NdnResult<Option<u64>> {
        self.storage.get_mtree_index(key)
    }

    fn put_mtree_data(&mut self, value: &[u8]) -> NdnResult<()> {
        self.storage.put_mtree_data(value)
    }

    fn load_mtree_data(&self) -> NdnResult<Option<Vec<u8>>> {
        self.storage.load_mtree_data()
    }

    // Clone the storage to a new file.
    // If the target file exists, it will be failed.
    async fn clone(
        &self,
        target: &Path,
        read_only: bool,
    ) -> NdnResult<Box<dyn ObjectMapInnerStorage>> {
        if read_only {
            let ret = Self {
                read_only,
                file: target.to_path_buf(),
                storage: self.storage.clone_for_read(),
            };

            Ok(Box::new(ret))
        } else {
            self.clone_for_modify(target).await
        }
    }

    // If file is diff from the current one, it will be saved to the file.
    async fn save(&mut self, file: &Path) -> NdnResult<()> {
        // Check if the storage is read-only
        self.storage.check_read_only()?;

        // Check if the file is the same as the current one
        if file != self.file {
            if file.exists() {
                warn!(
                    "Target object map storage file already exists: {}, now will overwrite it",
                    file.display()
                );
            }

            if self.file.exists() {
                if file.exists() {
                    // If the target file exists, we need to rename the current file
                    std::fs::remove_file(file).map_err(|e| {
                        let msg = format!("Failed to remove file: {:?}, {}", file, e);
                        error!("{}", msg);
                        NdnError::IoError(msg)
                    })?;
                }
                
                tokio::fs::rename(&self.file, file).await.map_err(|e| {
                    let msg = format!(
                        "Failed to rename json file: {:?} -> {:?}, {}",
                        self.file, file, e
                    );
                    error!("{}", msg);
                    NdnError::IoError(msg)
                })?;
            } else {
                // We hadn't save the file yet, so we can just create a new one
                self.storage.set_dirty(true);
            }

            // Update the file path
            self.file = file.to_path_buf();
        } else {
            if !self.file.exists() {
                // We hadn't save the file yet, so we can just create a new one
                self.storage.set_dirty(true);
            }
        }

        if !self.storage.is_dirty() {
            // No changes to save
            return Ok(());
        }

        // Serialize the data to JSON and write it to the file
        let f = std::fs::File::create(&file).map_err(|e| {
            let msg = format!("Failed to create file: {:?}, {}", file, e);
            error!("{}", msg);
            NdnError::IoError(msg)
        })?;

        serde_json::to_writer(f, &self.storage.get_data()).map_err(|e| {
            let msg = format!("Failed to write data to file: {:?}, {}", file, e);
            error!("{}", msg);
            NdnError::IoError(msg)
        })?;

        info!("Saved JSON storage to file: {:?}", file);

        // Mark the storage as clean
        self.storage.set_dirty(false);

        Ok(())
    }

    async fn dump(&self) -> NdnResult<Option<serde_json::Value>> {
        self.storage.dump().await
    }
}
