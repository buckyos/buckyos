use super::storage::{ObjectMapInnerStorage, ObjectMapInnerStorageStat, ObjectMapStorageType};
use crate::{NdnError, NdnResult, ObjId};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Clone, Debug)]
struct MemoryStorageItem {
    value: ObjId,
    mtree_index: Option<u64>,
}

pub struct MemoryStorage {
    read_only: bool,
    storage: BTreeMap<String, MemoryStorageItem>,
    meta: Option<Vec<u8>>,
    mtree_data: Option<Vec<u8>>,
}

impl MemoryStorage {
    pub fn new(read_only: bool) -> Self {
        Self {
            read_only,
            storage: BTreeMap::new(),
            meta: None,
            mtree_data: None,
        }
    }

    fn check_read_only(&self) -> NdnResult<()> {
        if self.read_only {
            let msg = "Memory storage is read-only".to_string();
            error!("{}", msg);
            return Err(NdnError::PermissionDenied(msg));
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl ObjectMapInnerStorage for MemoryStorage {
    fn get_type(&self) -> ObjectMapStorageType {
        ObjectMapStorageType::Memory
    }

    fn is_readonly(&self) -> bool {
        self.read_only
    }

    async fn put(&mut self, key: &str, value: &ObjId) -> NdnResult<()> {
        // Check if the storage is read-only
        self.check_read_only()?;

        self.storage.insert(
            key.to_string(),
            MemoryStorageItem {
                value: value.clone(),
                mtree_index: None,
            },
        );

        Ok(())
    }

    async fn get(&self, key: &str) -> NdnResult<Option<(ObjId, Option<u64>)>> {
        if let Some(item) = self.storage.get(key) {
            Ok(Some((item.value.clone(), item.mtree_index)))
        } else {
            Ok(None)
        }
    }

    async fn remove(&mut self, key: &str) -> NdnResult<Option<ObjId>> {
        // Check if the storage is read-only
        self.check_read_only()?;

        if let Some(item) = self.storage.remove(key) {
            Ok(Some(item.value))
        } else {
            Ok(None)
        }
    }

    async fn is_exist(&self, key: &str) -> NdnResult<bool> {
        Ok(self.storage.contains_key(key))
    }

    async fn list(&self, page_index: usize, page_size: usize) -> NdnResult<Vec<String>> {
        let start = page_index * page_size;
        let end = start + page_size;
        let list = self
            .storage
            .iter() // Get an iterator of (&String, &Vec<u8>)
            .map(|(k, _)| k.clone()) // We only need the keys (paths)
            .skip(start) // Skip the first 'start' paths
            .take(page_size) // Take the next 'page_size' paths
            .collect(); // Collect into a Vec<String>

        Ok(list)
    }

    async fn stat(&self) -> NdnResult<ObjectMapInnerStorageStat> {
        Ok(ObjectMapInnerStorageStat {
            total_count: self.storage.len() as u64,
        })
    }

    fn iter<'a>(&'a self) -> Box<dyn Iterator<Item = (String, ObjId, Option<u64>)> + 'a> {
        Box::new(
            self.storage
                .iter()
                .map(|(key, item)| (key.clone(), item.value.clone(), item.mtree_index)),
        )
    }

    async fn put_meta(&mut self, value: &[u8]) -> NdnResult<()> {
        // Check if the storage is read-only
        self.check_read_only()?;

        self.meta = Some(value.to_vec());
        Ok(())
    }

    async fn get_meta(&self) -> NdnResult<Option<Vec<u8>>> {
        Ok(self.meta.clone())
    }

    async fn update_mtree_index(&mut self, key: &str, index: u64) -> NdnResult<()> {
        // Check if the storage is read-only
        self.check_read_only()?;

        if let Some(item) = self.storage.get_mut(key) {
            item.mtree_index = Some(index);
            return Ok(());
        }

        let msg = format!("No such key: {}", key);
        Err(NdnError::NotFound(msg))
    }

    async fn get_mtree_index(&self, key: &str) -> NdnResult<Option<u64>> {
        if let Some(item) = self.storage.get(key) {
            Ok(item.mtree_index)
        } else {
            Ok(None)
        }
    }

    async fn put_mtree_data(&mut self, value: &[u8]) -> NdnResult<()> {
        // Check if the storage is read-only
        self.check_read_only()?;

        self.mtree_data = Some(value.to_vec());
        Ok(())
    }

    async fn load_mtree_data(&self) -> NdnResult<Option<Vec<u8>>> {
        Ok(self.mtree_data.clone())
    }

    async fn clone(
        &self,
        _target: &Path,
        read_only: bool,
    ) -> NdnResult<Box<dyn ObjectMapInnerStorage>> {
        // Create a new MemoryStorage instance
        let mut new_storage = MemoryStorage::new(read_only);

        // Copy meta and mtree data
        new_storage.storage = self.storage.clone();
        new_storage.meta = self.meta.clone();
        new_storage.mtree_data = self.mtree_data.clone();

        Ok(Box::new(new_storage))
    }

    // If file is diff from the current one, it will be saved to the file.
    async fn save(&mut self, _file: &Path) -> NdnResult<()> {
        // Check if the storage is read-only
        self.check_read_only()?;

        // Memory storage does not need to save to file, just return Ok
        Ok(())
    }
}
