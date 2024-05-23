use crate::{CheckPointVersion, TaskKey};

#[derive(Copy, Clone)]
pub enum ChunkServerType {}

pub trait ChunkMgrServer {}

#[async_trait::async_trait]
pub trait ChunkMgrSelector: Send + Sync {
    async fn select(
        &self,
        task_key: &TaskKey,
        check_point_version: CheckPointVersion,
        file_hash: &str,
        chunk_seq: u32,
        chunk_hash: &str,
    ) -> Result<Box<dyn ChunkMgrClient>, Box<dyn std::error::Error + Send + Sync>>;

    async fn select_by_name(
        &self,
        chunk_server_type: ChunkServerType,
        server_name: &str,
    ) -> Result<Box<dyn ChunkMgrClient>, Box<dyn std::error::Error + Send + Sync>>;
}

#[async_trait::async_trait]
pub trait ChunkMgrClient: Send + Sync {
    fn server_type(&self) -> ChunkServerType;
    fn server_name(&self) -> &str;
    async fn upload(&self, chunk_hash: &str, chunk: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}
