use crate::{NdnResult, ObjId};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectArrayStorageType {
    Arrow,
    SQLite,
    SimpleFile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectArrayCacheType {
    // Only used for memory storage
    Memory,
    Arrow,
}

#[async_trait::async_trait]
pub trait ObjectArrayInnerCache: Send + Sync {
    fn get_type(&self) -> ObjectArrayCacheType;
    fn len(&self) -> usize;

    fn append(&mut self, value: &ObjId) -> NdnResult<()>;

    fn get(&self, index: usize) -> NdnResult<Option<ObjId>>;
    fn get_range(&self, start: usize, end: usize) -> NdnResult<Vec<ObjId>>;
}

#[async_trait::async_trait]
pub trait ObjectArrayStorageReader: Send + Sync {
    async fn get(&self, index: usize) -> NdnResult<Option<ObjId>>;
    async fn get_range(&self, start: usize, end: usize) -> NdnResult<Vec<ObjId>>;
    async fn len(&self) -> NdnResult<usize>;
}

#[async_trait::async_trait]
pub trait ObjectArrayStorageWriter: Send + Sync {
    async fn append(&mut self, value: &ObjId) -> NdnResult<()>;
    async fn len(&self) -> NdnResult<usize>;

    async fn flush(&mut self) -> NdnResult<()>;
}


impl Default for ObjectArrayStorageType {
    fn default() -> Self {
        ObjectArrayStorageType::Arrow
    }
}