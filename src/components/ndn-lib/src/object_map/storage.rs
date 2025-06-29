use crate::coll::CollectionStorageMode;
use crate::{NdnResult, ObjId};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::AtomicU64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

impl ObjectMapStorageType {
    pub fn is_memory(&self) -> bool {
        matches!(self, ObjectMapStorageType::Memory)
    }

    pub fn is_sqlite(&self) -> bool {
        matches!(self, ObjectMapStorageType::SQLite)
    }

    pub fn is_json_file(&self) -> bool {
        matches!(self, ObjectMapStorageType::JSONFile)
    }

    pub fn select_storage_type(coll_mode: Option<CollectionStorageMode>) -> Self {
        match coll_mode {
            Some(CollectionStorageMode::Simple) => Self::JSONFile,
            Some(CollectionStorageMode::Normal) => Self::SQLite,
            None => Self::SQLite,
        }
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
    async fn put_with_index(&mut self, key: &str, value: &ObjId, index: Option<u64>) -> NdnResult<()>;
    async fn get(&self, key: &str) -> NdnResult<Option<(ObjId, Option<u64>)>>;
    async fn remove(&mut self, key: &str) -> NdnResult<Option<ObjId>>;
    async fn is_exist(&self, key: &str) -> NdnResult<bool>;

    async fn list(&self, page_index: usize, page_size: usize) -> NdnResult<Vec<String>>;
    async fn stat(&self) -> NdnResult<ObjectMapInnerStorageStat>;

    fn iter<'a>(&'a self) -> Box<dyn Iterator<Item = (String, ObjId, Option<u64>)> + 'a>;

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
    async fn clone(
        &self,
        target: &Path,
        read_only: bool,
    ) -> NdnResult<Box<dyn ObjectMapInnerStorage>>;

    // If file is diff from the current one, it will be saved to the file.
    async fn save(&mut self, file: &Path) -> NdnResult<()>;
}
