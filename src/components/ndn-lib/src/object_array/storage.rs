use crate::NdnResult;

#[async_trait::async_trait]
pub trait ObjectArrayInnerStorage: Send + Sync {
    async fn append(&mut self, value: &[u8]) -> NdnResult<()>;
    async fn insert(&mut self, index: usize, value: &[u8]) -> NdnResult<()>;

    async fn get(&self, index: &usize) -> NdnResult<Option<Vec<u8>>>;
    async fn remove(&mut self, index: usize) -> NdnResult<Option<Vec<u8>>>;
    async fn pop(&mut self) -> NdnResult<Option<Vec<u8>>>;

    async fn len(&self) -> NdnResult<usize>;
}