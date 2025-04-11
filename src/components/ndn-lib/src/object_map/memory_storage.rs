use super::storage::{InnerStorage, InnerStorageStat};
use crate::{NdnError, NdnResult};
use std::collections::BTreeMap;

struct MemoryStorageItem {
    value: Vec<u8>,
    mtree_index: Option<u64>,
}

pub struct MemoryStorage {
    storage: BTreeMap<String, MemoryStorageItem>,
    meta: Option<Vec<u8>>,
    mtree_data: Option<Vec<u8>>,
}

impl MemoryStorage {
    pub fn new() -> Self {
        Self {
            storage: BTreeMap::new(),
            meta: None,
            mtree_data: None,
        }
    }
}

#[async_trait::async_trait]
impl InnerStorage for MemoryStorage {
    async fn put(&mut self, key: &str, value: &[u8]) -> NdnResult<()> {
        self.storage.insert(
            key.to_string(),
            MemoryStorageItem {
                value: value.to_vec(),
                mtree_index: None,
            },
        );

        Ok(())
    }

    async fn get(&self, key: &str) -> NdnResult<Option<(Vec<u8>, Option<u64>)>> {
        if let Some(item) = self.storage.get(key) {
            Ok(Some((item.value.clone(), item.mtree_index)))
        } else {
            Ok(None)
        }
    }

    async fn remove(&mut self, key: &str) -> NdnResult<Option<Vec<u8>>> {
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

    async fn stat(&self) -> NdnResult<InnerStorageStat> {
        Ok(InnerStorageStat {
            total_count: self.storage.len() as u64,
        })
    }

    async fn put_meta(&mut self, value: &[u8]) -> NdnResult<()> {
        self.meta = Some(value.to_vec());
        Ok(())
    }

    async fn get_meta(&self) -> NdnResult<Option<Vec<u8>>> {
        Ok(self.meta.clone())
    }

    async fn update_mtree_index(&mut self, key: &str, index: u64) -> NdnResult<()> {
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
        self.mtree_data = Some(value.to_vec());
        Ok(())
    }

    async fn load_mtree_data(&self) -> NdnResult<Option<Vec<u8>>> {
        Ok(self.mtree_data.clone())
    }
}
