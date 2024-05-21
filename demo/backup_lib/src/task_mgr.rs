use std::path::Path;

use crate::{CheckPointVersion, CheckPointVersionStrategy, FileServerType, TaskId, TaskKey};

pub enum TaskServerType {}

pub trait TaskMgrServer {}

#[async_trait::async_trait]
pub trait TaskMgrSelector {
    async fn select(
        &self,
        task_key: &TaskKey,
        check_point_version: CheckPointVersion,
    ) -> Result<Box<dyn TaskMgrClient>, Box<dyn std::error::Error>>;
}

#[async_trait::async_trait]
pub trait TaskMgrClient {
    fn server_type(&self) -> TaskServerType;
    fn server_name(&self) -> &str;

    async fn update_check_point_strategy(
        &self,
        task_key: &TaskKey,
        strategy: CheckPointVersionStrategy,
    ) -> Result<(), Box<dyn std::error::Error>>;

    async fn push_task_info(
        &self,
        task_key: &TaskKey,
        check_point_version: CheckPointVersion,
        prev_check_point_version: Option<CheckPointVersion>,
        meta: Option<&str>,
        dir_path: &Path,
    ) -> Result<TaskId, Box<dyn std::error::Error>>;

    // Ok(file-server-type, file-server-name, chunk-size)
    async fn add_file(
        &self,
        task_id: TaskId,
        file_path: &Path,
        hash: &str,
        file_size: u64,
    ) -> Result<(FileServerType, String, u32), Box<dyn std::error::Error>>;
}
