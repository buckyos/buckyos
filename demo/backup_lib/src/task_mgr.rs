use std::path::Path;
use std::sync::Arc;
use crate::{CheckPointVersion, CheckPointVersionStrategy, FileId, FileInfo, FileServerType, ListOffset, TaskId, TaskInfo, TaskKey};
use serde::{Deserialize, Serialize};
use warp::{Filter, Rejection, Reply};

#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
pub enum TaskServerType {
    Http = 1
}

impl TryFrom<u32> for TaskServerType {
    type Error = ();

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(TaskServerType::Http),
            _ => Err(()),
        }
    }
}

impl Into<u32> for TaskServerType {
    fn into(self) -> u32 {
        match self {
            TaskServerType::Http => 1,
        }
    }
}

pub trait TaskMgrServer: TaskMgr + Send + Sync {}

#[async_trait::async_trait]
pub trait TaskMgrSelector: Send + Sync {
    async fn select(
        &self,
        task_key: &TaskKey,
        check_point_version: Option<CheckPointVersion>,
    ) -> Result<Box<dyn TaskMgr>, Box<dyn std::error::Error + Send + Sync>>;
}

#[async_trait::async_trait]
pub trait TaskMgr: Send + Sync {
    fn server_type(&self) -> TaskServerType;
    fn server_name(&self) -> &str;

    async fn update_check_point_strategy(
        &self,
        zone_id: &str,
        task_key: &TaskKey,
        strategy: CheckPointVersionStrategy,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    async fn get_check_point_strategy(
        &self,
        zone_id: &str,
        task_key: &TaskKey,
    ) -> Result<CheckPointVersionStrategy, Box<dyn std::error::Error + Send + Sync>>;

    async fn push_task_info(
        &self,
        zone_id: &str,
        task_key: &TaskKey,
        check_point_version: CheckPointVersion,
        prev_check_point_version: Option<CheckPointVersion>,
        meta: Option<&str>,
        dir_path: &Path,
    ) -> Result<TaskId, Box<dyn std::error::Error + Send + Sync>>;

    // Ok(file-server-type, file-server-name, chunk-size)
    async fn add_file(
        &self,
        task_id: TaskId,
        file_seq: u64,
        file_path: &Path,
        hash: &str,
        file_size: u64,
    ) -> Result<(FileServerType, String, FileId, u32), Box<dyn std::error::Error + Send + Sync>>;

    async fn set_files_prepare_ready(
        &self,
        task_id: TaskId,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    async fn set_file_uploaded(
        &self,
        task_id: TaskId,
        file_path: &Path,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    async fn get_check_point_version_list(
        &self,
        zone_id: &str,
        task_key: &TaskKey,
        offset: ListOffset,
        limit: u32,
        is_restorable_only: bool,
    ) -> Result<Vec<TaskInfo>, Box<dyn std::error::Error + Send + Sync>>;

    async fn get_check_point_version(
        &self,
        zone_id: &str,
        task_key: &TaskKey,
        check_point_version: CheckPointVersion,
    ) -> Result<Option<TaskInfo>, Box<dyn std::error::Error + Send + Sync>>;

    async fn get_file_info(
        &self,
        zone_id: &str,
        task_id: TaskId,
        file_seq: u64,
    ) -> Result<Option<FileInfo>, Box<dyn std::error::Error + Send + Sync>>;
}
