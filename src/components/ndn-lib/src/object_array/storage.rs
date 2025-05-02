use crate::{NdnResult, ObjId};
use std::path::{Path, PathBuf};

#[async_trait::async_trait]
pub trait ObjectArrayInnerStorage: Send + Sync {
    async fn append(&mut self, value: &[u8]) -> NdnResult<()>;
    async fn insert(&mut self, index: usize, value: &[u8]) -> NdnResult<()>;

    async fn get(&self, index: &usize) -> NdnResult<Option<Vec<u8>>>;
    async fn remove(&mut self, index: usize) -> NdnResult<Option<Vec<u8>>>;
    async fn pop(&mut self) -> NdnResult<Option<Vec<u8>>>;

    async fn len(&self) -> NdnResult<usize>;
}

#[async_trait::async_trait]
pub trait ObjectArrayStorageWriter: Send + Sync {
    async fn append(&mut self, value: &ObjId) -> NdnResult<()>;
    async fn len(&self) -> NdnResult<usize>;

    async fn flush(&mut self) -> NdnResult<()>;
}

#[async_trait::async_trait]
pub trait ObjectArrayStorageReader: Send + Sync {
    async fn get(&self, index: usize) -> NdnResult<Option<ObjId>>;
    async fn get_range(&self, start: usize, end: usize) -> NdnResult<Vec<ObjId>>;
    async fn len(&self) -> NdnResult<usize>;
}

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