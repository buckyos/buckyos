use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use backup_lib::{
    CheckPointVersion, CheckPointVersionStrategy, ChunkMgrSelector, ChunkStorageClient,
    FileMgrSelector, FileStorage, FileStorageClient, ListOffset, TaskId, TaskInfo, TaskKey,
    TaskMgrSelector, TaskStorageClient, TaskStorageQuerier,
};

use crate::task::{BackupTask, BackupTaskEvent, RestoreTask, Task};

// TODO: config
const MAX_RUNNING_TASK_COUNT: usize = 5;

struct BackupTaskMap {
    task_ids: HashMap<TaskKey, HashMap<CheckPointVersion, TaskId>>, // key -> version -> task_id
    tasks: HashMap<TaskId, BackupTask>,                             // task_id -> task
}

impl BackupTaskMap {
    fn new() -> Self {
        Self {
            task_ids: HashMap::new(),
            tasks: HashMap::new(),
        }
    }

    // fn try_run(&mut self, backup_task: BackupTask) {
    //     if self.tasks.len() < MAX_RUNNING_TASK_COUNT {
    //         let task_key = backup_task.task_key();
    //         let task_id = backup_task.task_id();
    //         let version = backup_task.check_point_version();
    //         self.tasks.insert(task_id, backup_task.clone());
    //         self.task_ids
    //             .entry(task_key)
    //             .or_insert_with(HashMap::new)
    //             .insert(version, task_id);

    //         backup_task.start();
    //     }
    // }
}

pub(crate) struct BackupTaskMgrInner {
    task_storage: Arc<Box<dyn TaskStorageClient>>,
    file_storage: Arc<Box<dyn FileStorageClient>>,
    chunk_storage: Arc<Box<dyn ChunkStorageClient>>,
    task_mgr_selector: Arc<Box<dyn TaskMgrSelector>>,
    file_mgr_selector: Arc<Box<dyn FileMgrSelector>>,
    chunk_mgr_selector: Arc<Box<dyn ChunkMgrSelector>>,
    running_tasks: Arc<Mutex<BackupTaskMap>>,
    task_event_sender: tokio::sync::mpsc::Sender<BackupTaskEvent>,
}

impl BackupTaskMgrInner {
    pub(crate) fn task_storage(&self) -> Arc<Box<dyn TaskStorageClient>> {
        self.task_storage.clone()
    }

    pub(crate) fn file_storage(&self) -> Arc<Box<dyn FileStorageClient>> {
        self.file_storage.clone()
    }

    pub(crate) fn chunk_storage(&self) -> Arc<Box<dyn ChunkStorageClient>> {
        self.chunk_storage.clone()
    }

    pub(crate) fn task_mgr_selector(&self) -> Arc<Box<dyn TaskMgrSelector>> {
        self.task_mgr_selector.clone()
    }

    pub(crate) fn file_mgr_selector(&self) -> Arc<Box<dyn FileMgrSelector>> {
        self.file_mgr_selector.clone()
    }

    pub(crate) fn chunk_mgr_selector(&self) -> Arc<Box<dyn ChunkMgrSelector>> {
        self.chunk_mgr_selector.clone()
    }

    pub(crate) fn task_event_sender(&self) -> Arc<Box<dyn TaskEventSender>> {
        self.task_event_sender.clone()
    }
}

pub struct BackupTaskMgr(Arc<BackupTaskMgrInner>);

impl BackupTaskMgr {
    pub fn new(
        task_storage: Box<dyn TaskStorageClient>,
        file_storage: Box<dyn FileStorageClient>,
        chunk_storage: Box<dyn ChunkStorageClient>,
        task_mgr_selector: Box<dyn TaskMgrSelector>,
        file_mgr_selector: Box<dyn FileMgrSelector>,
        chunk_mgr_selector: Box<dyn ChunkMgrSelector>,
    ) -> Self {
        let (task_event_sender, task_event_receiver) = tokio::sync::mpsc::channel(1024);
        // listen events from tasks
        tokio::task::spawn(async move {
            loop {
                if let Some(event) = task_event_receiver.recv().await {
                    match event {
                        TaskEvent::TaskDone(task_id) => {}
                    }
                }
            }
        });

        Self(Arc::new(BackupTaskMgrInner {
            task_storage: Arc::new(task_storage),
            file_storage: Arc::new(file_storage),
            chunk_storage: Arc::new(chunk_storage),
            task_mgr_selector: Arc::new(task_mgr_selector),
            file_mgr_selector: Arc::new(file_mgr_selector),
            chunk_mgr_selector: Arc::new(chunk_mgr_selector),
            running_tasks: Arc::new(Mutex::new(BackupTaskMap::new())),
            task_event_sender,
        }))
    }

    pub async fn backup(
        &self,
        task_key: TaskKey,
        check_point_version: CheckPointVersion,
        prev_check_point_version: Option<CheckPointVersion>,
        meta: Option<String>,
        dir_path: PathBuf,
        chunk_files: Vec<(PathBuf, Option<(String, u64)>)>,
        is_discard_incomplete_versions: bool,
    ) -> Result<BackupTask, Box<dyn std::error::Error>> {
        let mgr = Arc::downgrade(&self.0);
        let backup_task = BackupTask::create_new(
            mgr,
            task_key,
            check_point_version,
            prev_check_point_version,
            meta,
            dir_path,
            chunk_files,
        )
        .await?;

        Ok(backup_task)
    }

    pub async fn continue_tasks(
        &self,
        task_key: &TaskKey,
        check_point_version: Option<CheckPointVersion>,
    ) -> Result<Vec<BackupTask>, Box<dyn std::error::Error>> {
        unimplemented!()
    }

    pub async fn get_last_check_point_version(
        &self,
        task_key: &TaskKey,
    ) -> Result<TaskInfo, Box<dyn std::error::Error>> {
        self.0
            .task_storage
            .get_last_check_point_version(task_key, false)
            .await
    }

    pub async fn get_check_point_version_list(
        &self,
        task_key: &TaskKey,
        offset: ListOffset,
        limit: u32,
    ) -> Result<Vec<TaskInfo>, Box<dyn std::error::Error>> {
        self.0
            .task_storage
            .get_check_point_version_list(task_key, offset, limit, false)
            .await
    }

    pub async fn get_check_point_strategy(
        &self,
        task_key: &TaskKey,
    ) -> Result<CheckPointVersionStrategy, Box<dyn std::error::Error>> {
        self.0.task_storage.get_check_point_strategy(task_key).await
    }

    pub async fn update_check_point_strategy(
        &self,
        task_key: &TaskKey,
        strategy: CheckPointVersionStrategy,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.0
            .task_storage
            .update_check_point_strategy(task_key, strategy)
            .await
    }
}

pub struct RestoreTaskMgr {
    task_storage: Box<dyn TaskStorageQuerier>,
    file_mgr_selector: Box<dyn RestoreFileMgrSelector>,
}

impl RestoreTaskMgr {
    pub fn new(
        task_storage: Box<dyn TaskStorageQuerier>,
        file_mgr_selector: Box<dyn RestoreFileMgrSelector>,
    ) -> Self {
        RestoreTaskMgr {
            task_storage,
            file_mgr_selector,
        }
    }

    pub async fn restore(
        &self,
        task_key: TaskKey,
        check_point_version: CheckPointVersion,
        dir_path: &Path,
    ) -> Result<RestoreTask, Box<dyn std::error::Error>> {
        unimplemented!()
    }

    pub async fn get_last_check_point_version(
        &self,
        task_key: &TaskKey,
    ) -> Result<TaskInfo, Box<dyn std::error::Error>> {
        self.task_storage
            .get_last_check_point_version(task_key, true)
            .await
    }

    pub async fn get_check_point_version_list(
        &self,
        task_key: &TaskKey,
        offset: ListOffset,
        limit: u32,
    ) -> Result<Vec<TaskInfo>, Box<dyn std::error::Error>> {
        self.task_storage
            .get_check_point_version_list(task_key, offset, limit, true)
            .await
    }
}
