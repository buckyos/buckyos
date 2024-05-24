use crate::{CheckPointVersion, TaskKey};

pub struct ChunkInfo {}

pub trait ChunkStorageQuerier: Send + Sync {}
pub trait ChunkStorage: ChunkStorageQuerier {}
