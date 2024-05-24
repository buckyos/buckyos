use std::path::Path;

use crate::{CheckPointVersion, CheckPointVersionStrategy, FileServerType, TaskId, TaskKey};

#[derive(Copy, Clone)]
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

pub trait TaskMgrServer: Send + Sync {}

#[async_trait::async_trait]
pub trait TaskMgrSelector: Send + Sync {
    async fn select(
        &self,
        task_key: &TaskKey,
        check_point_version: Option<CheckPointVersion>,
    ) -> Result<Box<dyn TaskMgrClient>, Box<dyn std::error::Error + Send + Sync>>;
}

#[async_trait::async_trait]
pub trait TaskMgrClient: Send + Sync {
    fn server_type(&self) -> TaskServerType;
    fn server_name(&self) -> &str;

    async fn update_check_point_strategy(
        &self,
        task_key: &TaskKey,
        strategy: CheckPointVersionStrategy,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    async fn get_check_point_strategy(
        &self,
        task_key: &TaskKey,
    ) -> Result<CheckPointVersionStrategy, Box<dyn std::error::Error + Send + Sync>>;

    async fn push_task_info(
        &self,
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
        file_path: &Path,
        hash: &str,
        file_size: u64,
    ) -> Result<(FileServerType, String, u32), Box<dyn std::error::Error + Send + Sync>>;

    async fn set_files_prepare_ready(
        &self,
        task_key: &TaskKey,
        check_point_version: CheckPointVersion,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    async fn set_file_uploaded(
        &self,
        task_key: &TaskKey,
        check_point_version: CheckPointVersion,
        file_path: &Path,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}
