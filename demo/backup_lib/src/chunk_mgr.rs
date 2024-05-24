use crate::{CheckPointVersion, TaskKey};

#[derive(Copy, Clone)]
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
