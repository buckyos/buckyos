use crate::{CheckPointVersion, TaskKey};

pub struct ChunkInfo {}

pub trait ChunkStorageQuerier {}
pub trait ChunkStorage: ChunkStorageQuerier {}

pub trait ChunkStorageClient: ChunkStorage {
    // Ok(is_uploaded)
    async fn is_chunk_uploaded(&self, chunk_hash: u64) -> Result<bool, Box<dyn std::error::Error>>;
    async fn chunk_uploaded(&self, chunk_hash: &str) -> Result<(), Box<dyn std::error::Error>>;
}
