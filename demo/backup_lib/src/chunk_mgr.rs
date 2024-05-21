use crate::{CheckPointVersion, TaskKey};

pub enum ChunkServerType {}

pub trait ChunkMgrServer {}

pub trait ChunkMgrSelector {
    async fn select(
        &self,
        task_key: &TaskKey,
        check_point_version: CheckPointVersion,
        file_hash: &str,
        chunk_seq: u32,
        chunk_hash: &str,
    ) -> Result<Box<dyn ChunkMgrClient>, Box<dyn std::error::Error>>;

    async fn select_by_name(
        &self,
        chunk_server_type: ChunkServerType,
        server_name: &str,
    ) -> Result<Box<dyn ChunkMgrClient>, Box<dyn std::error::Error>>;
}

pub trait ChunkMgrClient {
    fn server_type(&self) -> ChunkServerType;
    fn server_name(&self) -> &str;
    fn upload(&self, chunk_hash: &str, chunk: &[u8]) -> Result<(), Box<dyn std::error::Error>>;
}
