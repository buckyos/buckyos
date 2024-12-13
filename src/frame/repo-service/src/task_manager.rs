use crate::def::*;
use crate::error::*;
use crate::source_manager::SourceManager;
use crate::verifier::Verifier;
use buckyos_kit::buckyos_get_unix_timestamp;
use log::*;
use package_lib::PackageId;
use package_lib::Parser;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::task;

pub struct TaskManager {
    pub tasks: Arc<Mutex<HashMap<String, Task>>>,
}

impl TaskManager {
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn start_install_task(&self, package_id: PackageId) -> RepoResult<String> {
        let now_time = buckyos_get_unix_timestamp();
        let task_id = format!("{}-install-{}", package_id.to_string(), now_time);
        let task = Task::InstallTask {
            id: task_id.clone(),
            package_id,
            status: TaskStatus::Pending,
            deps: vec![],
            start_time: now_time,
            finish_time: 0,
        };
        self.tasks.lock().unwrap().insert(task_id.clone(), task);
        Ok(task_id)
    }

    pub fn start_index_update_task(&self) -> RepoResult<String> {
        let now_time = buckyos_get_unix_timestamp();
        let task_id = format!("index-update-{}", now_time);
        let task = Task::IndexUpdateTask {
            id: task_id.clone(),
            status: TaskStatus::Pending,
            start_time: now_time,
            finish_time: 0,
        };
        self.tasks.lock().unwrap().insert(task_id.clone(), task);
        Ok(task_id)
    }

    pub fn set_task_status(&self, task_id: &str, status: TaskStatus) -> RepoResult<()> {
        let mut tasks = self.tasks.lock().unwrap();
        if let Some(task) = tasks.get_mut(task_id) {
            match task {
                Task::InstallTask {
                    finish_time,
                    status: task_status,
                    ..
                }
                | Task::IndexUpdateTask {
                    finish_time,
                    status: task_status,
                    ..
                } => {
                    //如果status是Finished或者Error，设置finish_time
                    if let TaskStatus::Finished | TaskStatus::Error(_) = status {
                        *finish_time = buckyos_get_unix_timestamp();
                    }
                    *task_status = status;
                }
            }
            Ok(())
        } else {
            Err(RepoError::NotFound(format!("task {} not found", task_id)))
        }
    }

    pub fn set_task_deps(&self, task_id: &str, deps: Vec<PackageMeta>) -> RepoResult<()> {
        let mut tasks = self.tasks.lock().unwrap();
        if let Some(task) = tasks.get_mut(task_id) {
            match task {
                Task::InstallTask { deps: old_deps, .. } => {
                    *old_deps = deps;
                    return Ok(());
                }
                _ => {
                    let err_msg = format!("task {} is not an InstallTask", task_id);
                    warn!("{}", err_msg);
                    return Err(RepoError::ParamError(err_msg));
                }
            }
        }
        let err_msg = format!("task {} not found", task_id);
        warn!("{}", err_msg);
        Err(RepoError::NotFound(err_msg))
    }

    pub async fn get_task(&self, task_id: String) -> RepoResult<Option<Task>> {
        let tasks = self.tasks.lock().unwrap();
        Ok(tasks.get(&task_id).cloned())
    }

    pub async fn get_all_tasks(&self) -> RepoResult<Vec<Task>> {
        let tasks = self.tasks.lock().unwrap();
        Ok(tasks.values().cloned().collect())
    }

    pub async fn remove_task(&self, task_id: String) -> RepoResult<()> {
        let mut tasks = self.tasks.lock().unwrap();
        tasks.remove(&task_id);
        Ok(())
    }
}

lazy_static::lazy_static! {
    pub static ref REPO_TASK_MANAGER: TaskManager = TaskManager::new();
}
