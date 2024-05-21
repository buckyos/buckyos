use crate::chunk_storage::{ChunkStorage, ChunkStorageQuerier};

pub struct ChunkStorageSqlite {
    connection: rusqlite::Connection,
}

impl ChunkStorageQuerier for ChunkStorageSqlite {}

impl ChunkStorage for ChunkStorageSqlite {}
