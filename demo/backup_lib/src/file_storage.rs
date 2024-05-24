use std::path::{Path, PathBuf};

use crate::{task_storage::TaskId, CheckPointVersion, ChunkServerType, TaskKey};

pub struct FileInfo {
    pub file_seq: Option<u32>,
    pub task_id: TaskId,
    pub file_path: PathBuf,
    pub hash: String,
    pub file_size: u64,
}

pub trait FileStorageQuerier: Send + Sync {}
