use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use std::ops::{Add, Sub};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TaskKey(String);

impl<K: ToString> From<K> for TaskKey {
    fn from(value: K) -> Self {
        TaskKey(value.to_string())
    }
}

impl TaskKey {
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CheckPointVersion(u128);

impl Into<u128> for CheckPointVersion {
    fn into(self) -> u128 {
        self.0
    }
}

impl From<u128> for CheckPointVersion {
    fn from(id: u128) -> Self {
        CheckPointVersion(id)
    }
}

impl Add<u128> for CheckPointVersion {
    type Output = CheckPointVersion;

    fn add(self, rhs: u128) -> Self::Output {
        CheckPointVersion(self.0 + rhs)
    }
}

impl Sub<u128> for CheckPointVersion {
    type Output = CheckPointVersion;

    fn sub(self, rhs: u128) -> Self::Output {
        CheckPointVersion(self.0 - rhs)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TaskId(u128);

impl From<u128> for TaskId {
    fn from(id: u128) -> Self {
        TaskId(id)
    }
}

impl Into<u128> for TaskId {
    fn into(self) -> u128 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum ListOffset {
    FromFirst(u32),
    FromLast(u32),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskInfo {
    pub task_id: TaskId,
    pub task_key: TaskKey,
    pub check_point_version: CheckPointVersion,
    pub prev_check_point_version: Option<CheckPointVersion>,
    pub meta: Option<String>,
    pub dir_path: PathBuf,
    pub is_all_files_ready: bool,
    pub complete_file_count: usize,
    pub file_count: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CheckPointVersionStrategy {
    reserve_history_limit: u32,
    continuous_abort_incomplete_limit: u32,
    continuous_abort_seconds_limit: u32,
}

impl Default for CheckPointVersionStrategy {
    fn default() -> Self {
        CheckPointVersionStrategy {
            reserve_history_limit: 1,
            continuous_abort_incomplete_limit: 3,
            continuous_abort_seconds_limit: 3600 * 24 * 7, // 1 week
        }
    }
}

#[async_trait::async_trait]
pub trait TaskStorageQuerier: Send + Sync {
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
pub trait TaskStorageDelete {
    async fn delete_tasks_by_id(
        &self,
        task_id: &[TaskId],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

#[async_trait::async_trait]
pub trait TaskStorageInStrategy: TaskStorageQuerier {
    async fn prepare_clear_task_in_strategy(
        &self,
        task_key: &TaskKey,
        check_point_version: CheckPointVersion,
        prev_check_point_version: Option<CheckPointVersion>,
        strategy: &CheckPointVersionStrategy,
    ) -> Result<Vec<TaskId>, Box<dyn std::error::Error + Send + Sync>> {
        // TODO: check strategy to clear earlier tasks.
        Ok(vec![])
    }
}
