use crate::{ChunkId, FileServerType};
use serde::{Serialize, Deserialize};

#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
pub enum ChunkServerType {
    Http = 1
}

impl TryFrom<u32> for ChunkServerType {
    type Error = ();

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(ChunkServerType::Http),
            _ => Err(()),
        }
    }
}

impl Into<u32> for ChunkServerType {
    fn into(self) -> u32 {
        match self {
            ChunkServerType::Http => 1,
        }
    }
}

#[async_trait::async_trait]
pub trait ChunkMgrServer: ChunkMgr {
    async fn add_chunk(
        &self,
        file_server_type: FileServerType,
        file_server_name: &str,
        chunk_hash: &str,
        chunk_size: u32,
    ) -> Result<ChunkId, Box<dyn std::error::Error + Send + Sync>>;
}

#[async_trait::async_trait]
pub trait ChunkMgrServerSelector: Send + Sync {
    async fn select(
        &self,
        file_hash: &str,
        chunk_seq: u64,
        chunk_hash: &str,
    ) -> Result<Box<dyn ChunkMgrServer>, Box<dyn std::error::Error + Send + Sync>>;

    async fn select_by_name(
        &self,
        chunk_server_type: ChunkServerType,
        server_name: &str,
    ) -> Result<Box<dyn ChunkMgrServer>, Box<dyn std::error::Error + Send + Sync>>;
}

#[async_trait::async_trait]
pub trait ChunkMgr: Send + Sync {
    fn server_type(&self) -> ChunkServerType;
    fn server_name(&self) -> &str;
    async fn upload(&self, chunk_hash: &str, chunk: &[u8]) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    async fn download(&self, chunk_id: ChunkId) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>;
}

#[async_trait::async_trait]
pub trait ChunkMgrSelector: Send + Sync {
    async fn select_by_name(
        &self,
        chunk_server_type: ChunkServerType,
        server_name: &str,
    ) -> Result<Box<dyn ChunkMgr>, Box<dyn std::error::Error + Send + Sync>>;
}
