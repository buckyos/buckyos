use serde::{Serialize, Deserialize};
use crate::NdnResult;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InnerStorageStat {
    pub total_count: u64,
}

#[async_trait::async_trait]
pub trait InnerStorage: Send + Sync {
    // Use to store object data
    async fn put(&mut self, key: &str, value: &[u8]) -> NdnResult<()>;
    async fn get(&self, key: &str) -> NdnResult<Option<(Vec<u8>, Option<u64>)>>;
    async fn remove(&mut self, key: &str) -> NdnResult<Option<Vec<u8>>>;
    async fn is_exist(&self, key: &str) -> NdnResult<bool>;

    async fn list(&self, page_index: usize, page_size: usize) -> NdnResult<Vec<String>>;
    async fn stat(&self) -> NdnResult<InnerStorageStat>;

    // Use to store meta data
    async fn put_meta(&mut self,value: &[u8]) -> NdnResult<()>;
    async fn get_meta(&self) -> NdnResult<Option<Vec<u8>>>;

    // Use to store the index of the mtree node
    async fn update_mtree_index(&mut self, key: &str, index: u64) -> NdnResult<()>;
    async fn get_mtree_index(&self, key: &str) -> NdnResult<Option<u64>>;
    async fn put_mtree_data(&mut self, value: &[u8]) -> NdnResult<()>;
    async fn load_mtree_data(&self) -> NdnResult<Option<Vec<u8>>>;
}


// Use to map key to path, first hash(key) -> base32 ->
