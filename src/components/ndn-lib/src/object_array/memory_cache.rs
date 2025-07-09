use super::storage::{ObjectArrayCacheType, ObjectArrayInnerCache};
use crate::{NdnError, NdnResult, ObjId};

pub struct ObjectArrayMemoryCache {
    /// The storage for the object array, storing ObjId.
    storage: Vec<ObjId>,
}

impl ObjectArrayMemoryCache {
    pub fn new() -> Self {
        Self {
            storage: Vec::new(),
        }
    }

    pub fn new_array(storage: Vec<ObjId>) -> Self {
        Self { storage }
    }
}

#[async_trait::async_trait]
impl ObjectArrayInnerCache for ObjectArrayMemoryCache {
    fn get_type(&self) -> ObjectArrayCacheType {
        ObjectArrayCacheType::Memory
    }

    fn len(&self) -> usize {
        self.storage.len()
    }

    fn is_readonly(&self) -> bool {
        false
    }

    fn clone_cache(&self, _read_only: bool) -> NdnResult<Box<dyn ObjectArrayInnerCache>> {
        let new_cache = ObjectArrayMemoryCache::new_array(self.storage.clone());
        Ok(Box::new(new_cache))
    }

    fn append(&mut self, value: &ObjId) -> NdnResult<()> {
        self.storage.push(value.clone());
        Ok(())
    }

    fn insert(&mut self, index: usize, value: &ObjId) -> NdnResult<()> {
        if index > self.storage.len() {
            let msg = format!("Index out of range: {}", index);
            error!("{}", msg);
            return Err(NdnError::OffsetTooLarge(msg));
        }

        self.storage.insert(index, value.clone());
        Ok(())
    }

    fn remove(&mut self, index: usize) -> NdnResult<Option<ObjId>> {
        if index >= self.storage.len() {
            let msg = format!("Index out of range: {}", index);
            error!("{}", msg);
            return Err(NdnError::OffsetTooLarge(msg));
        }

        let item = self.storage.remove(index);
        Ok(Some(item))
    }

    fn clear(&mut self) -> NdnResult<()> {
        self.storage.clear();
        Ok(())
    }

    fn pop(&mut self) -> NdnResult<Option<ObjId>> {
        if self.storage.is_empty() {
            return Ok(None);
        }

        let obj_id = self.storage.pop().unwrap();
        Ok(Some(obj_id))
    }

    fn get(&self, index: usize) -> NdnResult<Option<ObjId>> {
        if index >= self.storage.len() {
            return Ok(None);
        }

        let obj_id = &self.storage[index];
        let obj_id = ObjId::new_by_raw(obj_id.obj_type.clone(), obj_id.obj_hash.clone());
        Ok(Some(obj_id))
    }

    fn get_range(&self, start: usize, end: usize) -> NdnResult<Vec<ObjId>> {
        if start >= self.storage.len() || end > self.storage.len() || start >= end {
            return Ok(vec![]);
        }

        let mut ret = Vec::with_capacity(end - start);
        for index in start..end {
            let obj_id = &self.storage[index];
            let obj_id = ObjId::new_by_raw(obj_id.obj_type.clone(), obj_id.obj_hash.clone());
            ret.push(obj_id);
        }

        Ok(ret)
    }
}
