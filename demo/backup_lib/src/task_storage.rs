use std::path::{Path, PathBuf};

use crate::{file_storage::FileInfo, storage::Transaction, FileServerType};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaskKey(String);

impl<K: ToString> From<K> for TaskKey {
    fn from(value: K) -> Self {
        TaskKey(value.to_string())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, Hash)]
pub struct CheckPointVersion(u128);

impl From<u128> for CheckPointVersion {
    fn from(id: u128) -> Self {
        CheckPointVersion(id)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, Hash)]
pub struct TaskId(u128);

impl From<u128> for TaskId {
    fn from(id: u128) -> Self {
        TaskId(id)
    }
}

pub enum ListOffset {
    FromFirst(u32),
    FromLast(u32),
}

#[derive(Clone)]
pub struct TaskInfo {
    pub task_id: TaskId,
    pub task_key: TaskKey,
    pub check_point_version: CheckPointVersion,
    pub prev_check_point_version: Option<CheckPointVersion>,
    pub meta: Option<String>,
    pub dir_path: PathBuf,
    pub is_all_files_ready: bool,
    pub is_all_files_done: bool,
    pub file_count: usize,
}

pub struct CheckPointVersionStrategy {
    reserve_history_limit: u32,
    continuous_abort_incomplete_limit: u32,
    continuous_abort_seconds_limit: u32,
}

#[async_trait::async_trait]
pub trait TaskStorageQuerier: Sync {
    async fn get_last_check_point_version(
        &self,
        task_key: &TaskKey,
        is_restorable_only: bool,
    ) -> Result<TaskInfo, Box<dyn std::error::Error>>;

    async fn get_check_point_version_list(
        &self,
        task_key: &TaskKey,
        offset: ListOffset,
        limit: u32,
        is_restorable_only: bool,
    ) -> Result<Vec<TaskInfo>, Box<dyn std::error::Error>>;

    async fn get_check_point_version_list_in_range(
        &self,
        task_key: &TaskKey,
        min_version: Option<CheckPointVersion>,
        max_version: Option<CheckPointVersion>,
        limit: u32,
        is_restorable_only: bool,
    ) -> Result<Vec<TaskInfo>, Box<dyn std::error::Error>>;
}

#[async_trait::async_trait]
pub trait TaskStorageDelete {
    async fn delete_tasks_by_id(
        &self,
        task_id: &[TaskId],
    ) -> Result<(), Box<dyn std::error::Error>>;
}

#[async_trait::async_trait]
pub trait TaskStorageInStrategy: TaskStorageQuerier {
    async fn prepare_clear_task_in_strategy(
        &self,
        task_key: &TaskKey,
        check_point_version: CheckPointVersion,
        prev_check_point_version: Option<CheckPointVersion>,
        strategy: &CheckPointVersionStrategy,
    ) -> Result<Vec<TaskId>, Box<dyn std::error::Error>> {
        // TODO: check strategy to clear earlier tasks.
        Ok(())
    }
}

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
    ) -> Result<TaskId, Box<dyn std::error::Error>>;

    async fn add_file(
        &self,
        task_id: TaskId,
        file_path: &Path,
        hash: &str,
        file_size: u64,
    ) -> Result<(), Box<dyn std::error::Error>>;

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
    ) -> Result<TaskId, Box<dyn std::error::Error>> {
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

        for file_info in files.iter() {
            self.add_file(
                task_id,
                file_info.file_path.as_path(),
                file_info.hash.as_str(),
                file_info.file_size,
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
    ) -> Result<Vec<TaskInfo>, Box<dyn std::error::Error>>;

    async fn get_incomplete_files(
        &self,
        min_file_seq: usize,
        limit: usize,
    ) -> Result<Vec<FileInfo>, Box<dyn std::error::Error>>;
    async fn is_task_info_pushed(
        &self,
        task_key: &TaskKey,
        check_point_version: CheckPointVersion,
    ) -> Result<Option<TaskId>, Box<dyn std::error::Error>>;

    async fn set_task_info_pushed(
        &self,
        task_key: &TaskKey,
        check_point_version: CheckPointVersion,
        task_id: TaskId,
    ) -> Result<(), Box<dyn std::error::Error>>;

    // Ok(file-server-name)
    async fn is_file_info_pushed(
        &self,
        task_key: &TaskKey,
        check_point_version: CheckPointVersion,
        file_path: &Path,
    ) -> Result<Option<(FileServerType, String, u32)>, Box<dyn std::error::Error>>;

    async fn set_file_info_pushed(
        &self,
        task_key: &TaskKey,
        check_point_version: CheckPointVersion,
        file_path: &Path,
        server_type: FileServerType,
        server_name: &str,
        chunk_size: u32,
    ) -> Result<(), Box<dyn std::error::Error>>;
}
