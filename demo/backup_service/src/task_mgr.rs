use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

use backup_lib::{
    CheckPointVersion, CheckPointVersionStrategy, ChunkMgrSelector, FileMgrSelector, ListOffset,
    TaskId, TaskKey, TaskMgrSelector, TaskStorageQuerier,
};
use rusqlite::Result;
use tokio::sync::Mutex;

use crate::{
    backup_task::{BackupTask, BackupTaskEvent, Task, TaskInfo, TaskInner},
    chunk_transfer::{self, ChunkTransfer},
    restore_task::RestoreTask,
    task_storage::{ChunkStorageClient, FileStorageClient, FilesReadyState, TaskStorageClient},
};

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

    async fn try_run(&mut self, backup_task: BackupTask) {
        let task_count = self.tasks.len();
        if task_count >= MAX_RUNNING_TASK_COUNT {
            let low_priority = self
                .tasks
                .iter()
                .max_by(|t1, t2| Self::priority_cmp(&t1.1.task_info(), &t2.1.task_info()))
                .unwrap();

            if let std::cmp::Ordering::Greater =
                Self::priority_cmp(&low_priority.1.task_info(), &backup_task.task_info())
            {
                let _todo_ = low_priority.1.stop().await;
            }
        } else {
            let task_key = backup_task.task_key();
            let task_id = backup_task.task_id();
            let version = backup_task.check_point_version();
            match self.tasks.entry(task_id) {
                std::collections::hash_map::Entry::Occupied(_) => {
                    return;
                }
                std::collections::hash_map::Entry::Vacant(et) => {
                    et.insert(backup_task.clone());
                }
            }
            self.task_ids
                .entry(task_key)
                .or_insert_with(HashMap::new)
                .insert(version, task_id);

            let _todo_ = backup_task.start().await;
        }
    }

    fn remove_task(&mut self, backup_task: &BackupTask) {
        let task_key = backup_task.task_key();
        let task_id = backup_task.task_id();
        let version = backup_task.check_point_version();
        self.tasks.remove(&task_id);
        self.task_ids.entry(task_key).and_modify(|v| {
            v.remove(&version);
        });
    }

    fn priority_cmp(t1: &TaskInfo, t2: &TaskInfo) -> std::cmp::Ordering {
        let retry1 = Self::test_retry_duration(t1);
        let retry2 = Self::test_retry_duration(t2);
        if !retry1 {
            std::cmp::Ordering::Greater
        } else if !retry2 {
            std::cmp::Ordering::Less
        } else {
            t1.priority.cmp(&t2.priority)
        }
    }

    fn test_retry_duration(t: &TaskInfo) -> bool {
        match t.last_fail_at {
            Some(t) => {
                let now = std::time::SystemTime::now();
                match now.duration_since(t) {
                    Ok(d) if d.as_secs() < 300 => false,
                    _ => true,
                }
            }
            None => true,
        }
    }
}

enum BackupState {
    Running(tokio::sync::mpsc::Sender<BackupTaskEvent>),
    Stopping(
        tokio::sync::mpsc::Sender<()>,
        tokio::sync::mpsc::Sender<BackupTaskEvent>,
    ),
    Stopped,
}

pub(crate) struct BackupTaskMgrInner {
    zone_id: String,
    task_storage: Arc<dyn TaskStorageClient>,
    file_storage: Arc<dyn FileStorageClient>,
    chunk_storage: Arc<dyn ChunkStorageClient>,
    task_mgr_selector: Arc<Box<dyn TaskMgrSelector>>,
    file_mgr_selector: Arc<Box<dyn FileMgrSelector>>,
    chunk_mgr_selector: Arc<Box<dyn ChunkMgrSelector>>,
    running_tasks: Arc<Mutex<BackupTaskMap>>,
    state: Arc<Mutex<BackupState>>,
    chunk_transfer: ChunkTransfer,
    timeout_per_kb: std::time::Duration,
}

impl BackupTaskMgrInner {
    pub(crate) fn zone_id(&self) -> &str {
        self.zone_id.as_str()
    }

    pub(crate) fn task_storage(&self) -> Arc<dyn TaskStorageClient> {
        self.task_storage.clone()
    }

    pub(crate) fn file_storage(&self) -> Arc<dyn FileStorageClient> {
        self.file_storage.clone()
    }

    pub(crate) fn chunk_storage(&self) -> Arc<dyn ChunkStorageClient> {
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

    pub(crate) fn timeout_per_kb(&self) -> std::time::Duration {
        self.timeout_per_kb
    }

    pub(crate) async fn task_event_sender(
        &self,
    ) -> Option<tokio::sync::mpsc::Sender<BackupTaskEvent>> {
        let state = self.state.lock().await;
        match &*state {
            BackupState::Running(sender) => Some(sender.clone()),
            BackupState::Stopping(_, sender) => Some(sender.clone()),
            _ => None,
        }
    }

    async fn try_run(&self, backup_task: BackupTask) -> usize {
        let mut running_tasks = self.running_tasks.lock().await;
        running_tasks.try_run(backup_task).await;
        running_tasks.tasks.len()
    }

    async fn remove_task(&self, backup_task: &BackupTask) -> usize {
        let mut running_tasks = self.running_tasks.lock().await;
        running_tasks.remove_task(backup_task);
        running_tasks.tasks.len()
    }
}

#[derive(Clone)]
pub struct BackupTaskMgr(Arc<BackupTaskMgrInner>);

impl BackupTaskMgr {
    pub fn new(
        zone_id: String,
        task_storage: Arc<dyn TaskStorageClient>,
        file_storage: Arc<dyn FileStorageClient>,
        chunk_storage: Arc<dyn ChunkStorageClient>,
        task_mgr_selector: Box<dyn TaskMgrSelector>,
        file_mgr_selector: Box<dyn FileMgrSelector>,
        chunk_mgr_selector: Box<dyn ChunkMgrSelector>,
    ) -> Self {
        // let (task_event_sender, task_event_receiver) = tokio::sync::mpsc::channel(1024);
        let task_mgr = Arc::new(BackupTaskMgrInner {
            task_storage,
            file_storage,
            chunk_storage,
            task_mgr_selector: Arc::new(task_mgr_selector),
            file_mgr_selector: Arc::new(file_mgr_selector),
            chunk_mgr_selector: Arc::new(chunk_mgr_selector),
            running_tasks: Arc::new(Mutex::new(BackupTaskMap::new())),
            state: Arc::new(Mutex::new(BackupState::Stopped)),
            zone_id,
            chunk_transfer: ChunkTransfer::new(chunk_transfer::Config { limit: 8 }),
            timeout_per_kb: std::time::Duration::from_secs(4),
        });

        Self(task_mgr)
    }

    pub async fn start(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let (task_event_sender, mut task_event_receiver) = tokio::sync::mpsc::channel(1024);

        {
            let mut state = self.0.state.lock().await;
            match &*state {
                BackupState::Running(_) => return Err("BackupTaskMgr is already started".into()),
                BackupState::Stopping(_, _) => {
                    return Err("BackupTaskMgr is stopping, you should wait it finish.".into())
                }
                BackupState::Stopped => *state = BackupState::Running(task_event_sender),
            }
        }

        let task_mgr = self.clone();
        let task_mgr_inner = self.0.clone();
        // listen events from tasks
        tokio::task::spawn(async move {
            loop {
                task_mgr
                    .makeup_tasks()
                    .await
                    .expect("todo: you can retry later");
                if let Some(event) = task_event_receiver.recv().await {
                    match event {
                        BackupTaskEvent::New(backup_task) => {
                            log::info!("new task: {:?}, try run it.", backup_task.task_id());
                            task_mgr_inner.try_run(backup_task).await;
                        }
                        BackupTaskEvent::Idle(backup_task) => {
                            task_mgr_inner.remove_task(&backup_task).await;
                        }
                        BackupTaskEvent::ErrorAndWillRetry(backup_task, _todo_err) => {
                            task_mgr_inner.remove_task(&backup_task).await;
                        }
                        BackupTaskEvent::Fail(backup_task, _todo_err) => {
                            task_mgr_inner.remove_task(&backup_task).await;
                        }
                        BackupTaskEvent::Successed(backup_task) => {
                            task_mgr_inner.remove_task(&backup_task).await;
                        }
                        BackupTaskEvent::Stop(backup_task) => {
                            let task_count = task_mgr_inner.remove_task(&backup_task).await;
                            let mut state = task_mgr_inner.state.lock().await;
                            match &*state {
                                BackupState::Running(_) => {}
                                BackupState::Stopping(stop_notifier, _) => {
                                    if task_count == 0 {
                                        let stop_notifier = stop_notifier.clone();
                                        *state = BackupState::Stopped;
                                        let _todo_ = stop_notifier.send(()).await;
                                        break;
                                    }
                                }
                                BackupState::Stopped => {
                                    unreachable!("should no task running when stopped")
                                }
                            }
                        }
                    }
                }
            }
        });

        Ok(())
    }

    pub async fn stop(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        {
            let mut state = self.0.state.lock().await;
            match &*state {
                BackupState::Running(task_event_sender) => {
                    *state = BackupState::Stopping(sender, task_event_sender.clone())
                }
                BackupState::Stopping(_, _) => {
                    return Err("BackupTaskMgr is stopping, you should wait it finish.".into())
                }
                BackupState::Stopped => return Err("BackupTaskMgr is already stopped".into()),
            }
        }
        receiver.recv().await;
        Ok(())
    }

    pub async fn backup(
        &self,
        task_key: TaskKey,
        check_point_version: CheckPointVersion,
        prev_check_point_version: Option<CheckPointVersion>,
        meta: Option<String>,
        dir_path: PathBuf,
        files: Vec<(PathBuf, Option<(String, u64)>)>, // (relative_file_path, (file-hash, file-size))
        _todo_is_discard_incomplete_versions: bool,
        priority: u32,
        is_manual: bool,
    ) -> Result<BackupTask, Box<dyn std::error::Error + Send + Sync>> {
        let mgr = Arc::downgrade(&self.0);
        let backup_task = BackupTask::create_new(
            mgr,
            task_key.clone(),
            check_point_version,
            prev_check_point_version,
            meta,
            dir_path,
            files,
            priority,
            is_manual,
            self.0.chunk_transfer.clone(),
        )
        .await?;

        if let Some(task_event_sender) = self.0.task_event_sender().await {
            task_event_sender
                .send(BackupTaskEvent::New(backup_task.clone()))
                .await?;
        }

        if let Ok(remote_task_server) = self.0.task_mgr_selector.select(&task_key, None).await {
            if let Ok(strategy) = remote_task_server
                .get_check_point_strategy(self.0.zone_id.as_str(), &task_key)
                .await
            {
                if let Ok(removed_tasks) = self
                    .0
                    .task_storage
                    .prepare_clear_task_in_strategy(
                        &task_key,
                        check_point_version,
                        prev_check_point_version,
                        &strategy,
                    )
                    .await
                {
                    // TODO: stop and remove the removed tasks.

                    let _todo_ = self
                        .0
                        .task_storage
                        .delete_tasks_by_id(removed_tasks.as_slice())
                        .await;
                }
            }
        }

        Ok(backup_task)
    }

    pub async fn all_files_has_prepare_ready(
        &self,
        task_id: TaskId,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.0
            .task_storage
            .set_files_prepare_ready(task_id, FilesReadyState::Ready)
            .await
    }

    pub async fn get_tasks(
        &self,
        task_key: &TaskKey,
        check_point_version: Option<CheckPointVersion>,
    ) -> Result<Vec<TaskInfo>, Box<dyn std::error::Error + Send + Sync>> {
        unimplemented!()
    }

    pub async fn get_last_check_point_version(
        &self,
        task_key: &TaskKey,
    ) -> Result<Option<TaskInfo>, Box<dyn std::error::Error + Send + Sync>> {
        let mut task_infos = self
            .0
            .task_mgr_selector
            .select(task_key, None)
            .await?
            .get_check_point_version_list(
                self.0.zone_id.as_str(),
                task_key,
                ListOffset::FromLast(0),
                1,
                false,
            )
            .await?;

        let task_info_server = task_infos.get(0);
        let server_version =
            task_info_server.map_or(0, |info| Into::<u128>::into(info.check_point_version));

        let task_info_local = TaskStorageClient::get_last_check_point_version(
            self.0.task_storage.as_ref(),
            task_key,
            false,
        )
        .await?;

        let local_version = task_info_local
            .as_ref()
            .map_or(0, |info| Into::<u128>::into(info.check_point_version));
        if local_version >= server_version {
            if local_version == 0 {
                Ok(None)
            } else {
                Ok(task_info_local)
            }
        } else {
            let info_server = task_infos.remove(0);
            Ok(Some(TaskInfo {
                task_id: info_server.task_id,
                task_key: info_server.task_key,
                check_point_version: info_server.check_point_version,
                prev_check_point_version: info_server.prev_check_point_version,
                meta: info_server.meta,
                dir_path: info_server.dir_path,
                priority: 0,
                is_manual: false,
                is_all_files_ready: if info_server.is_all_files_ready {
                    FilesReadyState::RemoteReady
                } else {
                    FilesReadyState::NotReady
                },
                complete_file_count: info_server.complete_file_count,
                file_count: info_server.file_count,
                last_fail_at: None,
                create_time: info_server.create_time,
            }))
        }
    }

    pub async fn get_check_point_version_list(
        &self,
        task_key: &TaskKey,
        offset: ListOffset,
        limit: u32,
    ) -> Result<Vec<TaskInfo>, Box<dyn std::error::Error + Send + Sync>> {
        // TODO: 到task-mgr服务器上获取

        TaskStorageClient::get_check_point_version_list(
            self.0.task_storage.as_ref(),
            task_key,
            offset,
            limit,
            false,
        )
        .await
    }

    pub async fn get_check_point_strategy(
        &self,
        task_key: &TaskKey,
    ) -> Result<CheckPointVersionStrategy, Box<dyn std::error::Error + Send + Sync>> {
        self.0
            .task_mgr_selector
            .select(task_key, None)
            .await?
            .get_check_point_strategy(self.0.zone_id.as_str(), task_key)
            .await
    }

    pub async fn update_check_point_strategy(
        &self,
        task_key: &TaskKey,
        strategy: CheckPointVersionStrategy,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.0
            .task_mgr_selector
            .select(task_key, None)
            .await?
            .update_check_point_strategy(self.0.zone_id.as_str(), task_key, strategy)
            .await
    }

    async fn makeup_tasks(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut is_more = true;
        let mut incomplete_tasks = vec![];
        let mut offset = 0;
        while is_more {
            let tasks = self.0.task_storage.get_incomplete_tasks(offset, 10).await?;

            is_more = tasks.len() == 10;
            offset += tasks.len() as u32;

            log::info!(
                "get incomplete tasks: {:?}",
                tasks.iter().map(|t| t.task_id).collect::<Vec<_>>()
            );

            let tasks = tasks
                .into_iter()
                .filter(BackupTaskMap::test_retry_duration)
                .collect::<Vec<_>>();
            incomplete_tasks.extend(tasks);
            incomplete_tasks.sort_by(BackupTaskMap::priority_cmp);
            incomplete_tasks.truncate(10);
        }

        for task in incomplete_tasks {
            let backup_task = BackupTask::from_storage(
                Arc::downgrade(&self.0),
                task,
                self.0.chunk_transfer.clone(),
            );
            self.0.try_run(backup_task).await;
        }

        Ok(())
    }
}

pub struct RestoreTaskMgrInner {
    zone_id: String,
    task_mgr_selector: Arc<Box<dyn TaskMgrSelector>>,
    file_mgr_selector: Arc<Box<dyn FileMgrSelector>>,
    chunk_mgr_selector: Arc<Box<dyn ChunkMgrSelector>>,
    timeout_per_kb: std::time::Duration,
}

impl RestoreTaskMgrInner {
    pub(crate) fn zone_id(&self) -> &str {
        self.zone_id.as_str()
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

    pub(crate) fn timeout_per_kb(&self) -> std::time::Duration {
        self.timeout_per_kb
    }
}

pub struct RestoreTaskMgr(Arc<RestoreTaskMgrInner>);

impl RestoreTaskMgr {
    pub fn new(
        zone_id: String,
        task_mgr_selector: Box<dyn TaskMgrSelector>,
        file_mgr_selector: Box<dyn FileMgrSelector>,
        chunk_mgr_selector: Box<dyn ChunkMgrSelector>,
    ) -> Self {
        RestoreTaskMgr(Arc::new(RestoreTaskMgrInner {
            task_mgr_selector: Arc::new(task_mgr_selector),
            file_mgr_selector: Arc::new(file_mgr_selector),
            chunk_mgr_selector: Arc::new(chunk_mgr_selector),
            zone_id,
            timeout_per_kb: std::time::Duration::from_secs(4),
        }))
    }

    pub async fn restore(
        &self,
        task_key: TaskKey,
        check_point_version: CheckPointVersion,
        dir_path: &Path,
    ) -> Result<Vec<std::path::PathBuf>, Box<dyn std::error::Error + Send + Sync>> {
        let task_mgr_server = self
            .0
            .task_mgr_selector
            .select(&task_key, Some(check_point_version))
            .await?;
        let task_info = task_mgr_server
            .get_check_point_version(self.0.zone_id.as_str(), &task_key, check_point_version)
            .await?;

        match task_info {
            Some(task_info) => {
                let restore_task = RestoreTask::create_new(
                    Arc::downgrade(&self.0),
                    task_mgr_server,
                    task_info,
                    dir_path.to_path_buf(),
                )
                .await?;
                let files = restore_task.start().await?;
                Ok(files)
            }
            None => Err("task not found".into()),
        }
    }

    pub async fn get_last_check_point_version(
        &self,
        task_key: &TaskKey,
    ) -> Result<Option<TaskInfo>, Box<dyn std::error::Error + Send + Sync>> {
        let mut task_infos = self
            .0
            .task_mgr_selector
            .select(task_key, None)
            .await?
            .get_check_point_version_list(
                self.0.zone_id.as_str(),
                task_key,
                ListOffset::FromLast(0),
                1,
                true,
            )
            .await?;

        if task_infos.len() > 0 {
            let info_server = task_infos.remove(0);
            Ok(Some(TaskInfo {
                task_id: info_server.task_id,
                task_key: info_server.task_key,
                check_point_version: info_server.check_point_version,
                prev_check_point_version: info_server.prev_check_point_version,
                meta: info_server.meta,
                dir_path: info_server.dir_path,
                priority: 0,
                is_manual: false,
                is_all_files_ready: if info_server.is_all_files_ready {
                    FilesReadyState::RemoteReady
                } else {
                    FilesReadyState::NotReady
                },
                complete_file_count: info_server.complete_file_count,
                file_count: info_server.file_count,
                last_fail_at: None,
                create_time: info_server.create_time,
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn get_check_point_version_list(
        &self,
        task_key: &TaskKey,
        offset: ListOffset,
        limit: u32,
    ) -> Result<Vec<TaskInfo>, Box<dyn std::error::Error + Send + Sync>> {
        // TODO: 到task-mgr服务器上获取
        let task_infos = self
            .0
            .task_mgr_selector
            .select(task_key, None)
            .await?
            .get_check_point_version_list(self.0.zone_id.as_str(), task_key, offset, limit, true)
            .await?;
        Ok(task_infos
            .into_iter()
            .map(|info| TaskInfo {
                task_id: info.task_id,
                task_key: info.task_key,
                check_point_version: info.check_point_version,
                prev_check_point_version: info.prev_check_point_version,
                meta: info.meta,
                dir_path: info.dir_path,
                priority: 0,
                is_manual: false,
                is_all_files_ready: if info.is_all_files_ready {
                    FilesReadyState::RemoteReady
                } else {
                    FilesReadyState::NotReady
                },
                complete_file_count: info.complete_file_count,
                file_count: info.file_count,
                last_fail_at: None,
                create_time: info.create_time,
            })
            .collect())
    }
}
