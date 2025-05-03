use crate::{NdnResult, ObjId};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectArrayStorageType {
    Arrow,
    SQLite,
    SimpleFile,
}

impl Default for ObjectArrayStorageType {
    fn default() -> Self {
        ObjectArrayStorageType::Arrow
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectArrayCacheType {
    // Only used for memory storage
    Memory,
    Arrow,
}

impl Default for ObjectArrayCacheType {
    fn default() -> Self {
        ObjectArrayCacheType::Memory
    }
}
#[async_trait::async_trait]
pub trait ObjectArrayInnerCache: Send + Sync {
    fn get_type(&self) -> ObjectArrayCacheType;
    fn len(&self) -> usize;
    fn is_readonly(&self) -> bool;

    fn get(&self, index: usize) -> NdnResult<Option<ObjId>>;
    fn get_range(&self, start: usize, end: usize) -> NdnResult<Vec<ObjId>>;

    fn clone_cache(&self, read_only: bool) -> NdnResult<Box<dyn ObjectArrayInnerCache>>;

    // Modify methods, can not be used in readonly mode
    fn append(&mut self, value: &ObjId) -> NdnResult<()>;
    fn insert(&mut self, index: usize, value: &ObjId) -> NdnResult<()>;
    fn remove(&mut self, index: usize) -> NdnResult<Option<ObjId>>;
    fn clear(&mut self) -> NdnResult<()>;
    fn pop(&mut self) -> NdnResult<Option<ObjId>>;
}

/* 
#[async_trait::async_trait]
pub trait ObjectArrayStorageReader: Send + Sync {
    fn into_cache(self) -> NdnResult<Box<dyn ObjectArrayInnerCache>>;

    async fn get(&self, index: usize) -> NdnResult<Option<ObjId>>;
    async fn get_range(&self, start: usize, end: usize) -> NdnResult<Vec<ObjId>>;
    async fn len(&self) -> NdnResult<usize>;
}
*/

#[async_trait::async_trait]
pub trait ObjectArrayStorageWriter: Send + Sync {
    async fn file_path(&self) -> NdnResult<PathBuf>;

    async fn append(&mut self, value: &ObjId) -> NdnResult<()>;
    async fn len(&self) -> NdnResult<usize>;

    async fn flush(&mut self) -> NdnResult<()>;
}