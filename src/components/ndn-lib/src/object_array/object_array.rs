use crate::ObjId;
use crate::{
    NdnError,
    NdnResult,
};
use super::storage::ObjectArrayInnerStorage;

pub struct ObjectArray {
    pub is_dirty: bool,
    pub storage: Box<dyn ObjectArrayInnerStorage>,
    pub mtree: Option<MerkleTreeObject>,
}


impl ObjectArray {
    pub fn new(mut storage: Box<dyn ObjectArrayInnerStorage>,) -> Self {
        Self {
            is_dirty: false,
            storage,
            mtree: None,
        }
    }

    pub fn append_object(&mut self, obj_id: &ObjId) -> NdnResult<()> {
        self.object_array.extend_from_slice(object);
        Ok(())
    }

    pub fn get_object(&self, index: usize) -> NdnResult<Option<Vec<u8>>> {
        if index < self.object_array.len() {
            Ok(Some(&self.object_array[index..]))
        } else {
            Ok(None)
        }
    }

    pub fn remove_object(&mut self, index: usize) -> NdnResult<()> {
        if index < self.object_array.len() {
            self.object_array.remove(index);
            Ok(())
        } else {
            Err(NdnError::IndexOutOfBounds)
        }
    }
    
}