use crate::{NdnResult, NdnError};
use super::storage::ObjectArrayInnerStorage;

pub struct ObjectArrayMemoryStorage {
    storage: Vec<Vec<u8>>,
}

impl ObjectArrayMemoryStorage {
    pub fn new() -> Self {
        Self {
            storage: Vec::new(),
        }
    }
}

#[async_trait::async_trait]
impl ObjectArrayInnerStorage for ObjectArrayMemoryStorage {
    async fn append(&mut self, value: &[u8]) -> NdnResult<()> {
        self.storage.push(value.to_vec());
        Ok(())
    }

    async fn insert(&mut self, index: usize, value: &[u8]) -> NdnResult<()> {
        if index > self.storage.len() {
            let msg = format!(
                "Index out of bounds: {} > {}",
                index,
                self.storage.len()
            );
            error!("{}", msg);
            return Err(NdnError::OffsetTooLarge(msg));
        }

        self.storage.insert(index, value.to_vec());
        Ok(())
    }

    async fn get(&self, index: &usize) -> NdnResult<Option<Vec<u8>>> {
        if *index < self.storage.len() {
            Ok(Some(self.storage[*index].clone()))
        } else {
            Ok(None)
        }
    }

    async fn remove(&mut self, index: usize) -> NdnResult<Option<Vec<u8>>> {
        if index < self.storage.len() {
            Ok(Some(self.storage.remove(index)))
        } else {
            Ok(None)
        }
    }

    async fn pop(&mut self) -> NdnResult<Option<Vec<u8>>> {
        if self.storage.is_empty() {
            Ok(None)
        } else {
            Ok(Some(self.storage.pop().unwrap()))
        }
    }

    async fn len(&self) -> NdnResult<usize> {
        Ok(self.storage.len())
    }
}