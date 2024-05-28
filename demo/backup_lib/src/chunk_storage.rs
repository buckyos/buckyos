use crate::ChunkServerType;
use serde::{Serialize, Deserialize}; // Add this line

#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ChunkId(u128);

impl From<u128> for ChunkId {
    fn from(id: u128) -> Self {
        ChunkId(id)
    }
}

impl Into<u128> for ChunkId {
    fn into(self) -> u128 {
        self.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkInfo {
    pub hash: String,
    pub chunk_size: u32,
    pub chunk_server: Option<(ChunkServerType, String, Option<ChunkId>)>,
}

pub trait ChunkStorageQuerier: Send + Sync {}
pub trait ChunkStorage: ChunkStorageQuerier {}
