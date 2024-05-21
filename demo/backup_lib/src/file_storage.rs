use std::path::{Path, PathBuf};

use crate::{task_storage::TaskId, CheckPointVersion, ChunkServerType, TaskKey};

pub struct FileInfo {
    pub file_seq: Option<u32>,
    pub task_id: TaskId,
    pub file_path: PathBuf,
    pub hash: String,
    pub file_size: u64,
}

pub trait FileStorageQuerier {}

pub trait FileStorage: FileStorageQuerier {}

pub trait FileStorageClient: FileStorage {
    // Ok((chunk-server-type, chunk-server-name, chunk-hash))
    async fn is_chunk_info_pushed(
        &self,
        file_hash: &str,
        chunk_seq: u64,
    ) -> Result<Option<(ChunkServerType, String, String)>, Box<dyn std::error::Error>>;

    async fn chunk_info_pushed(
        &self,
        file_hash: &str,
        chunk_seq: u64,
        chunk_server_type: ChunkServerType,
        server_name: &str,
        chunk_hash: &str,
    ) -> Result<(), Box<dyn std::error::Error>>;
}
