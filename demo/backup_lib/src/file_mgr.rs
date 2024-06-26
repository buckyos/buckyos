use crate::{CheckPointVersion, ChunkId, ChunkInfo, ChunkServerType, FileId, TaskKey, TaskServerType};
use std::convert::TryFrom;
use serde::{Serialize, Deserialize};

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub enum FileServerType {
    Http = 1
}

impl TryFrom<u32> for FileServerType {
    type Error = ();

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(FileServerType::Http),
            _ => Err(()),
        }
    }
}

impl Into<u32> for FileServerType {
    fn into(self) -> u32 {
        match self {
            FileServerType::Http => 1,
        }
    }
}

#[async_trait::async_trait]
pub trait FileMgrServer: FileMgr {
    // Ok((file-id, chunk-size))
    async fn add_file(
        &self,
        task_server_type: TaskServerType,
        task_server_name: &str,
        file_hash: &str,
        file_size: u64,
    ) -> Result<(FileId, u32), Box<dyn std::error::Error + Send + Sync>>;
}

#[async_trait::async_trait]
pub trait FileMgrServerSelector: Send + Sync {
    async fn select(
        &self,
        task_key: &TaskKey,
        check_point_version: CheckPointVersion,
        file_hash: &str,
    ) -> Result<Box<dyn FileMgrServer>, Box<dyn std::error::Error + Send + Sync>>;

    async fn select_by_name(
        &self,
        file_server_type: FileServerType,
        server_name: &str,
    ) -> Result<Box<dyn FileMgrServer>, Box<dyn std::error::Error + Send + Sync>>;
}

#[async_trait::async_trait]
pub trait FileMgr: Send + Sync {
    fn server_type(&self) -> FileServerType;
    fn server_name(&self) -> &str;

    async fn add_chunk(
        &self,
        file_id: FileId,
        chunk_seq: u64,
        chunk_hash: &str,
        chunk_size: u32,
    ) -> Result<(ChunkServerType, String, ChunkId), Box<dyn std::error::Error + Send + Sync>>;

    async fn set_chunk_uploaded(
        &self,
        file_id: FileId,
        chunk_seq: u64,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    async fn get_chunk_info(&self, file_id: FileId, chunk_seq: u64) -> Result<Option<ChunkInfo>, Box<dyn std::error::Error + Send + Sync>>;
}

#[async_trait::async_trait]
pub trait FileMgrSelector: Send + Sync {
    async fn select_by_name(
        &self,
        file_server_type: FileServerType,
        server_name: &str,
    ) -> Result<Box<dyn FileMgr>, Box<dyn std::error::Error + Send + Sync>>;
}
