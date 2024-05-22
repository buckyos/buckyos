use crate::{CheckPointVersion, TaskKey};

pub struct ChunkInfo {}

pub trait ChunkStorageQuerier {}
pub trait ChunkStorage: ChunkStorageQuerier {}

pub trait ChunkStorageClient: ChunkStorage {
    // Ok(is_uploaded)
    async fn is_chunk_uploaded(
        &self,
        file_hash: &str,
        chunk_seq: u64,
    ) -> Result<bool, Box<dyn std::error::Error>>;
    async fn set_chunk_uploaded(
        &self,
        file_hash: &str,
        chunk_seq: u64,
    ) -> Result<(), Box<dyn std::error::Error>>;
}
