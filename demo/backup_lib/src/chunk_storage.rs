use crate::ChunkServerType;

#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, Hash)]
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

pub struct ChunkInfo {
    pub hash: String,
    pub chunk_size: u32,
    pub chunk_server: Option<(ChunkServerType, String, ChunkId)>,
}

pub trait ChunkStorageQuerier: Send + Sync {}
pub trait ChunkStorage: ChunkStorageQuerier {}
