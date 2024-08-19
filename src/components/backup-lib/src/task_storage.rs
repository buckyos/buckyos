use serde::{Deserialize, Serialize};
use std::ops::{Add, Sub};
use std::path::PathBuf;

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

impl Sub<CheckPointVersion> for CheckPointVersion {
    type Output = u128;

    fn sub(self, rhs: CheckPointVersion) -> Self::Output {
        self.0 - rhs.0
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
    pub create_time: std::time::SystemTime,
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
        // 1. reserved_complete_tasks: get reserved complete tasks
        // 2. reserved_incomplete_tasks: get reserved incomplete tasks
        // 3. all_tasks = get all tasks
        // 4. reserved_depend_tasks = get all tasks depend by reserved_complete_tasks and reserved_incomplete_tasks
        // 5. discard_tasks = all_tasks - reserved_complete_tasks - reserved_incomplete_tasks - reserved_depend_tasks

        let now = std::time::SystemTime::now();

        let reserved_complete_tasks = self
            .get_check_point_version_list(
                task_key,
                ListOffset::FromLast(0),
                strategy.reserve_history_limit,
                true,
            )
            .await?;

        let mut reserved_incomplete_task = None;
        let last_complete_task = reserved_complete_tasks.last();
        let should_reserve_incomplete_task = match last_complete_task {
            Some(task_info) => {
                if (check_point_version - task_info.check_point_version) as u32
                    > strategy.continuous_abort_incomplete_limit
                {
                    true
                } else {
                    match now.duration_since(task_info.create_time) {
                        Ok(d) => {
                            d > std::time::Duration::from_secs(
                                strategy.continuous_abort_seconds_limit as u64,
                            )
                        }
                        _ => true,
                    }
                }
            }
            None => true,
        };

        let mut all_tasks = vec![];
        let mut offset_from_last = 0;
        let mut is_reserve_incomplete_task_fixed = false;
        loop {
            let append_tasks = self
                .get_check_point_version_list(
                    task_key,
                    ListOffset::FromLast(offset_from_last),
                    100,
                    false,
                )
                .await?;
            if append_tasks.len() == 0 {
                break;
            }
            offset_from_last += append_tasks.len() as u32;

            if should_reserve_incomplete_task && !is_reserve_incomplete_task_fixed {
                if let Some(task_info) = last_complete_task {
                    match append_tasks
                        .iter()
                        .rposition(|info| info.task_id == task_info.task_id)
                    {
                        Some(index) => {
                            if index + 1 < append_tasks.len() {
                                reserved_incomplete_task = Some(append_tasks[index + 1].clone());
                            }
                            is_reserve_incomplete_task_fixed = true;
                        }
                        None => {
                            reserved_incomplete_task = Some(append_tasks.first().unwrap().clone());
                        }
                    }
                }
            }

            all_tasks.push(append_tasks);
        }

        let all_tasks = all_tasks.into_iter().rev().flatten().collect::<Vec<_>>();

        if should_reserve_incomplete_task && !is_reserve_incomplete_task_fixed {
            reserved_incomplete_task = all_tasks.first().map(|info| info.clone());
        }

        let mut reserve_versions = vec![check_point_version];
        if let Some(prev_check_point_version) = prev_check_point_version {
            reserve_versions.push(prev_check_point_version);
        }
        if let Some(task_info) = reserved_incomplete_task {
            reserve_versions.push(task_info.check_point_version);
        }
        for task_info in reserved_complete_tasks {
            reserve_versions.push(task_info.check_point_version);
        }

        let mut remove_tasks = vec![];
        for task_info in all_tasks.into_iter().rev() {
            if reserve_versions.contains(&task_info.check_point_version) {
                reserve_versions.retain(|&version| version != task_info.check_point_version);
                if let Some(prev_check_point_version) = task_info.prev_check_point_version {
                    reserve_versions = reserve_versions
                        .iter()
                        .map(|&version| {
                            if version == task_info.check_point_version {
                                prev_check_point_version
                            } else {
                                version
                            }
                        })
                        .collect();
                }
            } else {
                remove_tasks.push(task_info.task_id);
            }
        }

        remove_tasks.reverse();

        Ok(remove_tasks)
    }
}
