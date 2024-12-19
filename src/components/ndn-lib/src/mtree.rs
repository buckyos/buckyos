//对mix-hash提供原生支持
use std::io::{Read, Seek, SeekFrom};
use std::pin::Pin;
use futures::io::{AsyncRead, AsyncWrite,AsyncSeek};
use std::collections::HashMap;
use crate::{ObjId,NdnResult,OBJ_TYPE_MTREE};



pub trait MtreeReadSeek: AsyncRead + AsyncSeek {}
// Blanket implementation for any type that implements both traits
impl<T: AsyncRead + AsyncSeek> MtreeReadSeek for T {}

pub trait MtreeWriteSeek: AsyncWrite + AsyncSeek {}
// Blanket implementation for any type that implements both traits
impl<T: AsyncWrite + AsyncSeek> MtreeWriteSeek for T {}

pub struct MerkleTreeObject {
    data_size:u64,
    leaf_size:u64,
    hash_type:Option<String>, //none means sha256
    body_reader:Option<Box<dyn MtreeReadSeek>>,
    body_writer:Option<Box<dyn MtreeWriteSeek>>,
    root_hash:Option<Vec<u8>>,
}

impl MerkleTreeObject {
    pub fn new(data_size:u64,leaf_size:u64,body_reader:Box<dyn MtreeWriteSeek>,body_writer:Box<dyn MtreeWriteSeek>,hash_type:Option<String>)->Self {
        Self {
            data_size,
            leaf_size,
            hash_type,
            body_reader:None,
            body_writer:Some(body_writer),
            root_hash:None
        }
    }

    //init from reader, reader is a reader of the body of the mtree
    pub fn load_from_reader(body_reader:Box<dyn MtreeReadSeek>)->NdnResult<Self> {
        unimplemented!()
    }

    pub fn append_leaf_hashs(&self,leaf_hashs:Vec<Vec<u8>>)->NdnResult<()> {
        unimplemented!()
    }

    pub fn cacl_root_hash(&self)->NdnResult<Vec<u8>> {
        unimplemented!()
    }

    //result is a map, key is the index of the leaf, value is the hash of the leaf
    pub fn get_verify_path_by_leaf_index(&self,leaf_index:u64)->NdnResult<HashMap<u64,Vec<u8>>> {
        unimplemented!()
    }

    pub fn get_leaf_count(&self)->u64 {
        unimplemented!()
    }

    pub fn get_leaf_size(&self)->u64 {
        unimplemented!()
    }

    pub fn get_root_hash(&self)->Option<Vec<u8>> {
        self.root_hash.clone()
    }

    pub fn get_data_size(&self)->u64 {
        self.data_size
    }

    pub fn get_obj_id(&self)->ObjId {
        return ObjId::new_by_raw(OBJ_TYPE_MTREE.to_string(), self.root_hash.as_ref().unwrap().clone());
    }

}