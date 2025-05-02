use crate::ObjId;
use crate::{
    NdnError,
    NdnResult,
};
use super::storage::ObjectArrayInnerStorage;

pub struct ObjectArray {
    pub data: Vec<ObjId>,
    pub is_dirty: bool,
}


impl ObjectArray {
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            is_dirty: false,
        }
    }

    pub fn append_object(&mut self, obj_id: &ObjId) -> NdnResult<()> {
        self.data.push(obj_id.clone());
        self.is_dirty = true;
        Ok(())
    }

    pub fn insert_object(&mut self, index: usize, obj_id: &ObjId) -> NdnResult<()> {
        if index > self.data.len() {
            let msg = format!("Index out of bounds: {}", index);
            error!("{}", msg);
            return Err(NdnError::OffsetTooLarge(msg));
        }

        self.data.insert(index, obj_id.clone());
        self.is_dirty = true;
        Ok(())
    }

    pub fn get_object(&self, index: usize) -> NdnResult<Option<&ObjId>> {
        if index >= self.data.len() {
            return Ok(None);
        }

        Ok(Some(&self.data[index]))
    }

    pub fn remove_object(&mut self, index: usize) -> NdnResult<Option<ObjId>> {
        if index >= self.data.len() {
            return Ok(None);
        }

        let obj_id = self.data.remove(index);
        self.is_dirty = true;
        Ok(Some(obj_id))
    }

    pub fn pop_object(&mut self) -> NdnResult<Option<ObjId>> {
        if self.data.is_empty() {
            return Ok(None);
        }

        let obj_id = self.data.pop().unwrap();
        self.is_dirty = true;
        Ok(Some(obj_id))
    }
}