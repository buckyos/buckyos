use std::path::{Path, PathBuf};

use crate::{task_storage::TaskId, CheckPointVersion, ChunkServerType, TaskKey};


#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, Hash)]
pub struct FileId(u128);

impl From<u128> for FileId {
    fn from(id: u128) -> Self {
        FileId(id)
    }
}

impl Into<u128> for FileId {
    fn into(self) -> u128 {
        self.0
    }
}


pub struct FileInfo {
    pub file_seq: Option<u32>,
    pub task_id: TaskId,
    pub file_path: PathBuf,
    pub hash: String,
    pub file_size: u64,
}

pub trait FileStorageQuerier: Send + Sync {}
