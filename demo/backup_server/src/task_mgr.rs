use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;
use backup_lib::{CheckPointVersion, CheckPointVersionStrategy, FileId, FileInfo, FileServerType, ListOffset, TaskId, TaskInfo, TaskKey, TaskMgrServer, TaskServerType};

use crate::task_mgr_storage::TaskStorageSqlite;

pub(crate) struct TaskMgr {
    storage: Arc<Mutex<TaskStorageSqlite>>,
    file_mgr_selector: Arc<dyn backup_lib::FileMgrServerSelector>,
}

impl TaskMgr {
    pub(crate) fn new(storage: TaskStorageSqlite, file_mgr_selector: Arc<dyn backup_lib::FileMgrServerSelector>) -> Self {
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
        file_seq: u64,
        file_path: &Path,
        hash: &str,
        file_size: u64,
    ) -> Result<(FileServerType, String, FileId, u32), Box<dyn std::error::Error + Send + Sync>> {
        let (file_server_type, file_server_name, remote_file_info) = {
            let mut storage = self.storage.lock().await;
            let task_info = storage.query_task_info_without_files(task_id)?.unwrap();
            let file_mgr = self.file_mgr_selector.select(&task_info.task_key, task_info.check_point_version, hash).await?;
            storage.insert_task_file(task_id, file_seq, file_path, hash, file_size, file_mgr.server_type(), file_mgr.server_name())?
        };

        match remote_file_info {
            Some((file_id, chunk_size)) => {
                Ok((file_server_type, file_server_name, file_id, chunk_size))
            },
            None => {
                let file_mgr = self.file_mgr_selector.select_by_name(file_server_type, file_server_name.as_str()).await?;
                let (file_id, chunk_size) = file_mgr.add_file(self.server_type(), self.server_name(), hash, file_size).await?;
                
                let mut storage = self.storage.lock().await;
                storage.update_file_info(hash, chunk_size, file_id)?;
                Ok((file_server_type, file_server_name, file_id, chunk_size))
            }
        }
    }

    async fn set_files_prepare_ready(
        &self,
        task_id: TaskId,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut storage = self.storage.lock().await;
        storage.update_all_files_ready(task_id)
    }

    async fn set_file_uploaded(
        &self,
        task_id: TaskId,
        file_path: &Path,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut storage = self.storage.lock().await;
        storage.update_file_uploaded(task_id, file_path)
    }

    async fn get_check_point_version_list(
        &self,
        zone_id: &str,
        task_key: &TaskKey,
        offset: ListOffset,
        limit: u32,
        is_restorable_only: bool,
    ) -> Result<Vec<TaskInfo>, Box<dyn std::error::Error + Send + Sync>> {
        self.storage.lock().await.get_check_point_version_list(zone_id, task_key, offset, limit, is_restorable_only)
    }

    async fn get_check_point_version(
        &self,
        zone_id: &str,
        task_key: &TaskKey,
        check_point_version: CheckPointVersion,
    ) -> Result<Option<TaskInfo>, Box<dyn std::error::Error + Send + Sync>> {
        self.storage.lock().await.get_check_point_version(zone_id, task_key, check_point_version)
    }

    async fn get_file_info(
        &self,
        zone_id: &str,
        task_id: TaskId,
        file_seq: u64,
    ) -> Result<Option<FileInfo>, Box<dyn std::error::Error + Send + Sync>> {
        self.storage.lock().await.get_file_info(zone_id, task_id, file_seq)
    }
}

impl TaskMgrServer for TaskMgr {}