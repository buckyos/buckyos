use std::io::SeekFrom;
use std::ops::{Deref, DerefMut};

use crate::ChunkId;
use crate::ObjId;
use crate::ObjectArray;
use crate::NdnResult;

pub struct ChunkList {
    chunk_list_imp:ObjectArray
}


impl Deref for ChunkList {
    type Target = ObjectArray;
    fn deref(&self) -> &Self::Target {
        &self.chunk_list_imp
    }
}


impl DerefMut for ChunkList {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.chunk_list_imp
    }
}

impl ChunkList {
    pub fn new() -> Self {
        unimplemented!()
    }

    pub fn new_by_obj_data(obj_data:serde_json::Value) -> Self {
        unimplemented!()
    }

    pub fn cacl_chunk_list_id(&self) -> ObjId {
        unimplemented!()
    }

    pub fn is_chunklist(obj_id:&ObjId) -> bool {
        unimplemented!()
    }

    pub fn is_simple_chunklist(obj_id:&ObjId) -> bool {
        unimplemented!()
    }

    pub fn is_fixed_size_chunklist(obj_id:&ObjId) -> bool {
        unimplemented!()
    }

    //return (chunk_index,chunk_offset)
    pub fn get_chunk_index_by_offset(&self, offset:SeekFrom) -> NdnResult<(u64,u64)> {
        unimplemented!()
    }

    pub fn get_chunk_offset_by_index(&self, index:u64) -> u64 {
        unimplemented!()
    }

    pub fn get_total_size(&self) -> u64 {
        unimplemented!()
    }
}

