use crate::{CheckPointVersion, ChunkServerType, TaskKey};

pub enum FileServerType {}

pub trait FileMgrServer {}

#[async_trait::async_trait]
pub trait FileMgrSelector {
    async fn select(
        &self,
        task_key: &TaskKey,
        check_point_version: CheckPointVersion,
        file_hash: &str,
    ) -> Result<Box<dyn FileMgrClient>, Box<dyn std::error::Error>>;

    async fn select_by_name(
        &self,
        file_server_type: FileServerType,
        server_name: &str,
    ) -> Result<Box<dyn FileMgrClient>, Box<dyn std::error::Error>>;
}

#[async_trait::async_trait]
pub trait FileMgrClient {
    fn server_type(&self) -> FileServerType;
    fn server_name(&self) -> &str;
    async fn add_chunk(
        &self,
        file_hash: &str,
        chunk_seq: u64,
        chunk_hash: &str,
    ) -> Result<(ChunkServerType, String), Box<dyn std::error::Error>>;
}
