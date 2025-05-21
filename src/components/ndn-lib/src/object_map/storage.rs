use crate::{NdnResult, ObjId};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::AtomicU64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectMapStorageType {
    Memory,
    SQLite,
    JSONFile,
}

impl Default for ObjectMapStorageType {
    fn default() -> Self {
        ObjectMapStorageType::SQLite
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectMapInnerStorageStat {
    pub total_count: u64,
}

#[async_trait::async_trait]
pub trait ObjectMapInnerStorage: Send + Sync {
    fn get_type(&self) -> ObjectMapStorageType;
    fn is_readonly(&self) -> bool;

    // Use to store object data
    async fn put(&mut self, key: &str, value: &ObjId) -> NdnResult<()>;
    async fn get(&self, key: &str) -> NdnResult<Option<(ObjId, Option<u64>)>>;
    async fn remove(&mut self, key: &str) -> NdnResult<Option<ObjId>>;
    async fn is_exist(&self, key: &str) -> NdnResult<bool>;

    async fn list(&self, page_index: usize, page_size: usize) -> NdnResult<Vec<String>>;
    async fn stat(&self) -> NdnResult<ObjectMapInnerStorageStat>;

    // Use to store meta data
    async fn put_meta(&mut self, value: &[u8]) -> NdnResult<()>;
    async fn get_meta(&self) -> NdnResult<Option<Vec<u8>>>;

    // Use to store the index of the mtree node
    async fn update_mtree_index(&mut self, key: &str, index: u64) -> NdnResult<()>;
    async fn get_mtree_index(&self, key: &str) -> NdnResult<Option<u64>>;
    async fn put_mtree_data(&mut self, value: &[u8]) -> NdnResult<()>;
    async fn load_mtree_data(&self) -> NdnResult<Option<Vec<u8>>>;

    // Clone the storage to a new file.
    // If the target file exists, it will be failed.
    async fn clone(&self, target: &Path, read_only: bool) -> NdnResult<Box<dyn ObjectMapInnerStorage>>;

    // If file is diff from the current one, it will be saved to the file.
    async fn save(&mut self, file: &Path) -> NdnResult<()>;
}
