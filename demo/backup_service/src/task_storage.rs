use std::path::Path;
use backup_lib::{CheckPointVersion, ChunkId, ChunkServerType, ChunkStorage, FileId, FileInfo, FileServerType, FileStorageQuerier, ListOffset, TaskId, TaskKey, TaskStorageDelete, TaskStorageInStrategy, Transaction};

use crate::backup_task::TaskInfo;

#[derive(Copy, Clone)]
pub enum FilesReadyState {
    NotReady = 0,
    Ready = 1,
    RemoteReady = 2,
}

impl std::convert::TryFrom<u32> for FilesReadyState {
    type Error = ();

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(FilesReadyState::NotReady),
            1 => Ok(FilesReadyState::Ready),
            2 => Ok(FilesReadyState::RemoteReady),
            _ => Err(()),
        }
    }
}

impl Into<u32> for FilesReadyState {
    fn into(self) -> u32 {
        match self {
            FilesReadyState::NotReady => 0,
            FilesReadyState::Ready => 1,
            FilesReadyState::RemoteReady => 2,
        }
    }
}

#[async_trait::async_trait]
pub trait TaskStorageClient: TaskStorageInStrategy + TaskStorageDelete + Transaction {
    async fn create_task(
        &self,
        task_key: &TaskKey,
        check_point_version: CheckPointVersion,
        prev_check_point_version: Option<CheckPointVersion>,
        meta: Option<&str>,
        dir_path: &Path,
        priority: u32,
        is_manual: bool,
    ) -> Result<TaskId, Box<dyn std::error::Error + Send + Sync>>;

    async fn add_file(
        &self,
        task_id: TaskId,
        file_path: &Path,
        hash: &str,
        file_size: u64,
        file_seq: u32
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    async fn set_files_prepare_ready(&self, task_id: TaskId, state: FilesReadyState) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    async fn create_task_with_files(
        &self,
        task_key: &TaskKey,
        check_point_version: CheckPointVersion,
        prev_check_point_version: Option<CheckPointVersion>,
        meta: Option<&str>,
        dir_path: &Path,
        files: &[FileInfo],
        priority: u32,
        is_manual: bool,
    ) -> Result<TaskId, Box<dyn std::error::Error + Send + Sync>> {
        self.begin_transaction().await?;

        let task_id = self
            .create_task(
                &task_key,
                check_point_version,
                prev_check_point_version,
                meta,
                dir_path,
                priority,
                is_manual,
            )
            .await?;

        for (seq, file_info) in files.iter().enumerate() {
            self.add_file(
                task_id,
                file_info.file_path.as_path(),
                file_info.hash.as_str(),
                file_info.file_size,
                seq as u32,
            )
            .await?;
        }

        self.commit_transaction().await?;

        Ok(task_id)
    }

    async fn get_incomplete_tasks(
        &self,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<TaskInfo>, Box<dyn std::error::Error + Send + Sync>>;

    async fn get_incomplete_files(
        &self,
        task_key: &TaskKey,
        version: CheckPointVersion,
        min_file_seq: usize,
        limit: usize,
    ) -> Result<Vec<FileInfo>, Box<dyn std::error::Error + Send + Sync>>;

    async fn is_task_info_pushed(
        &self,
        task_key: &TaskKey,
        check_point_version: CheckPointVersion,
    ) -> Result<Option<TaskId>, Box<dyn std::error::Error + Send + Sync>>;

    async fn set_task_info_pushed(
        &self,
        task_key: &TaskKey,
        check_point_version: CheckPointVersion,
        task_id: TaskId,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    // Ok(file-server-name)
    async fn is_file_info_pushed(
        &self,
        task_key: &TaskKey,
        check_point_version: CheckPointVersion,
        file_path: &Path,
    ) -> Result<Option<(FileServerType, String, FileId, u32)>, Box<dyn std::error::Error + Send + Sync>>;

    async fn set_file_info_pushed(
        &self,
        task_key: &TaskKey,
        check_point_version: CheckPointVersion,
        file_path: &Path,
        server_type: FileServerType,
        server_name: &str,
        remote_file_id: FileId,
        chunk_size: u32,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    async fn get_last_check_point_version(
        &self,
        task_key: &TaskKey,
        is_restorable_only: bool,
    ) -> Result<Option<TaskInfo>, Box<dyn std::error::Error + Send + Sync>>;

    async fn get_check_point_version_list(
        &self,
        task_key: &TaskKey,
        offset: ListOffset,
        limit: u32,
        is_restorable_only: bool,
    ) -> Result<Vec<TaskInfo>, Box<dyn std::error::Error + Send + Sync>>;

    async fn get_check_point_version_list_in_range(
        &self,
        task_key: &TaskKey,
        min_version: Option<CheckPointVersion>,
        max_version: Option<CheckPointVersion>,
        limit: u32,
        is_restorable_only: bool,
    ) -> Result<Vec<TaskInfo>, Box<dyn std::error::Error + Send + Sync>>;
}

#[async_trait::async_trait]
pub trait FileStorageClient: FileStorageQuerier {
    // Ok((chunk-server-type, chunk-server-name, chunk-hash))
    async fn is_chunk_info_pushed(
        &self,
        task_key: &TaskKey,
        version: CheckPointVersion,
        file_path: &Path,
        chunk_seq: u64,
    ) -> Result<Option<(ChunkServerType, String, String, ChunkId)>, Box<dyn std::error::Error + Send + Sync>>;

    async fn set_chunk_info_pushed(
        &self,
        task_key: &TaskKey,
        version: CheckPointVersion,
        file_path: &Path,
        chunk_seq: u64,
        chunk_server_type: ChunkServerType,
        server_name: &str,
        chunk_hash: &str,
        remote_chunk_id: ChunkId,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

#[async_trait::async_trait]
pub trait ChunkStorageClient: ChunkStorage {
    // Ok(is_uploaded)
    async fn is_chunk_uploaded(
        &self,
        task_key: &TaskKey,
        version: CheckPointVersion,
        file_path: &Path,
        chunk_seq: u64,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>;
    async fn set_chunk_uploaded(
        &self,
        task_key: &TaskKey,
        version: CheckPointVersion,
        file_path: &Path,
        chunk_seq: u64,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}
