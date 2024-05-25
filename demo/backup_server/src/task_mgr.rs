use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;
use backup_lib::{CheckPointVersion, CheckPointVersionStrategy, FileId, FileServerType, TaskId, TaskKey, TaskServerType};

use crate::task_mgr_storage::TaskStorageSqlite;

pub(crate) struct TaskMgr {
    storage: Arc<Mutex<TaskStorageSqlite>>,
    file_mgr_selector: Arc<dyn backup_lib::FileMgrSelector>,
}

impl TaskMgr {
    pub(crate) fn new(storage: TaskStorageSqlite, file_mgr_selector: Arc<dyn backup_lib::FileMgrSelector>) -> Self {
        Self { storage: Arc::new(Mutex::new(storage)), file_mgr_selector }
    }    
}

#[async_trait::async_trait]
impl backup_lib::TaskMgr for TaskMgr {
    fn server_type(&self) -> TaskServerType {
        TaskServerType::Http
    }
    fn server_name(&self) -> &str {
        "TODO: demo-task-server-name"
    }

    async fn update_check_point_strategy(
        &self,
        zone_id: &str,
        task_key: &TaskKey,
        strategy: CheckPointVersionStrategy,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let strategy = serde_json::to_string(&strategy)?;
        self.storage.lock().await.insert_or_update_strategy(zone_id, task_key, strategy.as_str())
    }

    async fn get_check_point_strategy(
        &self,
        zone_id: &str,
        task_key: &TaskKey,
    ) -> Result<CheckPointVersionStrategy, Box<dyn std::error::Error + Send + Sync>> {
        let strategy = self.storage.lock().await.query_strategy(zone_id, task_key)?;
        match strategy {
            Some(strategy) => Ok(serde_json::from_str(strategy.as_str())?),
            None => Ok(CheckPointVersionStrategy::default())
        }
    }

    async fn push_task_info(
        &self,
        zone_id: &str,
        task_key: &TaskKey,
        check_point_version: CheckPointVersion,
        prev_check_point_version: Option<CheckPointVersion>,
        meta: Option<&str>,
        dir_path: &Path,
    ) -> Result<TaskId, Box<dyn std::error::Error + Send + Sync>> {
        self.storage.lock().await.insert_task(zone_id, task_key, check_point_version, prev_check_point_version, meta, dir_path)
    }

    // Ok(file-server-type, file-server-name, chunk-size)
    async fn add_file(
        &self,
        task_id: TaskId,
        file_path: &Path,
        hash: &str,
        file_size: u64,
    ) -> Result<(FileServerType, String, FileId, u32), Box<dyn std::error::Error + Send + Sync>> {
        let mut storage = self.storage.lock().await;
        let remote_server = storage.insert_task_file(task_id, file_path, hash, file_size)?;
        let task_info = storage.query_task_info_without_files(task_id)?.unwrap();
        match remote_server {
            Some((file_server_type, file_server_name, file_id, chunk_size)) => {
                Ok((file_server_type, file_server_name, file_id, chunk_size))
            },
            None => {
                let file_mgr = self.file_mgr_selector.select(&task_info.task_key, task_info.check_point_version, hash).await?;
                let (file_server_type, file_server_name, file_id, chunk_size) = file_mgr.add_file(self.server_type(), self.server_name(), hash, file_size).await?;
                
                let mut storage = self.storage.lock().await;
                storage.update_file_info(hash, file_server_type, file_server_name.as_str(), chunk_size, file_id)?;
                Ok((file_server_type, file_server_name, file_id, chunk_size))
            }
        }
    }

    async fn set_files_prepare_ready(
        &self,
        task_id: TaskId,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        unimplemented!()
    }

    async fn set_file_uploaded(
        &self,
        task_id: TaskId,
        file_path: &Path,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        unimplemented!()
    }
}