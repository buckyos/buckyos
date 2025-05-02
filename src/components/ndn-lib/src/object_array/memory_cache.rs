use super::storage::{ObjectArrayCacheType, ObjectArrayInnerCache};
use crate::{NdnError, NdnResult, ObjId};

pub struct ObjectArrayMemoryCache {
    storage: Vec<ObjId>,
}

impl ObjectArrayMemoryCache {
    pub fn new() -> Self {
        Self {
            storage: Vec::new(),
        }
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

    fn append(&mut self, value: &ObjId) -> NdnResult<()> {
        self.storage.push(value.clone());
        Ok(())
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
