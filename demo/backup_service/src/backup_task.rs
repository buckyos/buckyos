use std::{
    collections::HashMap, path::PathBuf, sync::{Arc, Weak}, time::SystemTime
};

use base58::ToBase58;
use sha2::Digest;
use tokio::{io::{AsyncReadExt, AsyncSeekExt}, sync::Mutex};

use backup_lib::{CheckPointVersion, ChunkInfo, ChunkMgr, FileId, FileInfo, FileMgr, TaskId, TaskKey, TaskMgr};

use crate::{chunk_transfer::ChunkTransfer, task_mgr::BackupTaskMgrInner, task_storage::{ChunkStorageClient, FilesReadyState}};
use tokio::select;


#[derive(Clone)]
pub struct TaskInfo {
    pub task_id: TaskId,
    pub task_key: TaskKey,
    pub check_point_version: CheckPointVersion,
    pub prev_check_point_version: Option<CheckPointVersion>,
    pub meta: Option<String>,
    pub dir_path: PathBuf,
    pub is_all_files_ready: FilesReadyState,
    pub complete_file_count: usize,
    pub file_count: usize,
    pub priority: u32,
    pub is_manual: bool,
    pub last_fail_at: Option<SystemTime>,
    pub create_time: SystemTime,
}

#[derive(Clone)]
pub(crate) enum BackupTaskEvent {
    New(BackupTask),
    Idle(BackupTask),
    ErrorAndWillRetry(BackupTask, Arc<Box<dyn std::error::Error + Send + Sync>>),
    Fail(BackupTask, Arc<Box<dyn std::error::Error + Send + Sync>>),
    Stop(BackupTask),
    Successed(BackupTask),
}

pub trait Task {
    fn task_key(&self) -> TaskKey;
    fn task_id(&self) -> TaskId;
    fn check_point_version(&self) -> CheckPointVersion;
    fn prev_check_point_version(&self) -> Option<CheckPointVersion>;
    fn meta(&self) -> Option<String>;
    fn dir_path(&self) -> PathBuf;
    fn is_all_files_ready(&self) -> FilesReadyState;
    fn is_all_files_done(&self) -> bool;
    fn file_count(&self) -> usize;
}

#[async_trait::async_trait]
pub(crate) trait TaskInner {
    async fn start(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    async fn stop(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

#[derive(Clone, PartialEq, Eq, Debug)]
enum TaskState {
    Running,
    Stopping,
    Stopped,
}

#[derive(Clone)]
struct PendingChunkInfo {
    chunk_size: u32,
    chunk_seq: u64,
    file_info: FileInfo,
}

struct UploadChunkParam {
    chunk: Vec<u8>,
    hash: String,
    target_server: Box<dyn ChunkMgr>,
}

async fn upload_chunk(param: UploadChunkParam) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    param.target_server.upload(param.hash.as_str(), param.chunk.as_slice()).await
}

#[derive(Clone)]
pub struct BackupTask {
    mgr: Weak<BackupTaskMgrInner>,
    info: Arc<std::sync::Mutex<TaskInfo>>,
    state: (
        state_waiter::Waiter<TaskState>,
        state_waiter::State<TaskState>,
    ),
    transfer: ChunkTransfer,
    pending_waiters: Arc<Mutex<Vec<(state_waiter::Waiter<Option<Result<(), Arc<Box<dyn std::error::Error + Send + Sync>>>>>, Option<PendingChunkInfo>)>>>,
    pending_files: Arc<Mutex<HashMap<PathBuf, (FileInfo, u64)>>>, // <file-path, (FileInfo, complete_size)>
}

impl BackupTask {
    pub(crate) fn from_storage(mgr: Weak<BackupTaskMgrInner>, info: TaskInfo, transfer: ChunkTransfer) -> Self {
        let (state, waiter) = state_waiter::StateWaiter::new(TaskState::Running);
        // pending
        let (_, pending_waiter) = state_waiter::StateWaiter::new(None);
        Self {
            mgr,
            info: Arc::new(std::sync::Mutex::new(info)),
            pending_waiters: Arc::new(Mutex::new(vec![(pending_waiter, None)])),
            state: (waiter, state),
            transfer,
            pending_files: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub(crate) async fn create_new(
        mgr: Weak<BackupTaskMgrInner>,
        task_key: TaskKey,
        check_point_version: CheckPointVersion,
        prev_check_point_version: Option<CheckPointVersion>,
        meta: Option<String>,
        dir_path: PathBuf,
        files: Vec<(PathBuf, Option<(String, u64)>)>,
        priority: u32,
        is_manual: bool,
        transfer: ChunkTransfer
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let chunk_file_info_rets = futures::future::join_all(files.into_iter().enumerate().map(
            |(seq, (chunk_relative_path, hash_and_size))| {
                let dir_path = dir_path.clone();
                async move {
                    match hash_and_size {
                        Some((hash, file_size)) => Ok(FileInfo {
                            task_id: TaskId::from(0),
                            file_seq: seq as u64,
                            file_path: chunk_relative_path,
                            hash,
                            file_size,
                            file_server: None,
                        }),
                        None => {
                            let chunk_full_path = dir_path.join(&chunk_relative_path);
                            log::debug!("will read chunk file: {:?}, dir-path: {:?}, relative_path: {:?}", chunk_full_path, dir_path, chunk_relative_path);
                            let mut file = tokio::fs::File::open(chunk_full_path).await?;
                            let file_size = file.metadata().await?.len();
                            let mut buf = vec![];
                            file.read_to_end(&mut buf).await?;

                            let mut hasher = sha2::Sha256::new();
                            hasher.update(buf.as_slice());
                            let hash = hasher.finalize();
                            let hash = hash.as_slice().to_base58();

                            Ok(FileInfo {
                                task_id: TaskId::from(0),
                                file_seq: seq as u64,
                                file_path: chunk_relative_path,
                                hash,
                                file_size,
                                file_server: None,
                            })
                        }
                    }
                }
            },
        ))
        .await;

        let mut files = vec![];
        for info in chunk_file_info_rets {
            match info {
                Err(err) => {
                    log::error!("read chunk files failed: {:?}", err);
                    return Err(err);
                }
                Ok(r) => files.push(r),
            }
        }

        let task_storage = mgr
            .upgrade()
            .map_or(
                Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "maybe the system has stopped.",
                ))),
                |t| Ok(t),
            )?
            .task_storage();

        log::debug!("will create task with files: {:?}", files.len());

        let task_id = task_storage
            .create_task_with_files(
                &task_key,
                check_point_version,
                prev_check_point_version,
                meta.as_ref().map(|p| p.as_str()),
                dir_path.as_path(),
                files.as_slice(),
                priority,
                is_manual,
            )
            .await?;

        log::debug!("task created: {:?}", task_id);

        let (state, waiter) = state_waiter::StateWaiter::new(TaskState::Running);
        let (_, pending_waiter) = state_waiter::StateWaiter::new(None);
        Ok(Self {
            mgr,
            info: Arc::new(std::sync::Mutex::new(TaskInfo {
                task_id,
                task_key,
                check_point_version,
                prev_check_point_version,
                meta,
                dir_path: dir_path,
                is_all_files_ready: FilesReadyState::NotReady,
                file_count: files.len(),
                priority,
                is_manual,
                last_fail_at: None,
                complete_file_count: 0,
                create_time: SystemTime::now(),
            })),
            pending_waiters: Arc::new(Mutex::new(vec![(pending_waiter, None)])),
            state: (waiter, state),
            transfer,
            pending_files: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn task_info(&self) -> TaskInfo {
        self.info.lock().unwrap().clone()
    }

    async fn run_once(&self) -> BackupTaskEvent {
        log::debug!("try to run task: {:?}", self.task_id());

        let task_mgr = match self.mgr.upgrade() {
            Some(mgr) => mgr,
            None => {
                log::error!("task manager has been dropped.");
                return BackupTaskEvent::ErrorAndWillRetry(self.clone(), Arc::new("task manager has been dropped.".into()));
            }
        };

        let task_storage = task_mgr.task_storage();
        let task_info = self.info.lock().unwrap().clone();

        // push task info
        let (remote_task_mgr, remote_task_id) = match task_storage
            .is_task_info_pushed(&task_info.task_key, task_info.check_point_version)
            .await
        {
            Ok(remote_task_id) => {
                let remote_task_mgr = match task_mgr
                    .task_mgr_selector()
                    .select(&task_info.task_key, Some(task_info.check_point_version))
                    .await
                {
                    Ok(remote_task_mgr) => remote_task_mgr,
                    Err(err) => {return BackupTaskEvent::ErrorAndWillRetry(self.clone(), Arc::new(err));},
                };

                let remote_task_id = match remote_task_id {
                    Some(remote_task_id) => remote_task_id,
                    None => {
                        match remote_task_mgr
                            .push_task_info(
                                task_mgr.zone_id(),
                                &task_info.task_key,
                                task_info.check_point_version,
                                task_info.prev_check_point_version,
                                task_info.meta.as_ref().map(|s| s.as_str()),
                                task_info.dir_path.as_path(),
                            )
                            .await
                        {
                            Ok(remote_task_id) => {
                                if let Err(err) = task_mgr
                                    .task_storage()
                                    .set_task_info_pushed(
                                        &task_info.task_key,
                                        task_info.check_point_version,
                                        remote_task_id,
                                    )
                                    .await
                                {
                                    return BackupTaskEvent::ErrorAndWillRetry(self.clone(), Arc::new(err));
                                }
                                remote_task_id
                            }
                            Err(err) => return BackupTaskEvent::ErrorAndWillRetry(self.clone(), Arc::new(err)),
                        }
                    }
                };

                (remote_task_mgr, remote_task_id)
            }
            Err(err) => return BackupTaskEvent::ErrorAndWillRetry(self.clone(), Arc::new(err)),
        };

        log::info!("task info pushed: {:?} => {:?}", task_info.task_id, remote_task_id);

        // push files
        let mut file_seq = 0;
        loop {
            let upload_files = match task_storage.get_incomplete_files(&task_info.task_key, task_info.check_point_version, file_seq, 1).await {
                Ok(files) => {
                    if files.len() == 0 {
                        match self.is_all_files_ready() {
                            FilesReadyState::NotReady => return BackupTaskEvent::Idle(self.clone()),
                            FilesReadyState::Ready => {
                                loop {
                                    // only reserved pending
                                    if self.pending_waiters.lock().await.len() == 1 {
                                        log::info!("all chunk pending waiters are waked. task: {:?}", task_info.task_id);
                                        break;
                                    }

                                    let pending_waiters_fut = {
                                        let pending_waiters = self.pending_waiters.lock().await;
                                        futures::future::select_all(pending_waiters.iter().map(|(waiter, _)| waiter.wait(|s| s.is_some())))
                                    };
                                    let task_state_fut = self.state.0.wait(|s| *s != TaskState::Running);
                                
                                    // pin_mut!(task_state_fut);
                                    // pin_mut!(pending_waiters_fut);

                                    select! {
                                        _ = task_state_fut => {
                                            // Handle control event
                                            return BackupTaskEvent::Stop(self.clone())
                                        },
                                        complete_result = pending_waiters_fut => {
                                            // Handle pending event
                                            let (result, index, _) = complete_result;
                                            if let Some(result) = result {
                                                let pending_chunk_info = self.pending_waiters.lock().await.remove(index).1;
                                                let pending_chunk_info = pending_chunk_info.expect("there is a long pending waiter, should be not wake.");
                                                log::info!("pending chunk waited: {:?}, seq: {}, result: {:?}", pending_chunk_info.file_info.file_path, pending_chunk_info.chunk_seq, result);
                                                match result {
                                                    Ok(_) => {
                                                        if let Err(err) = self.post_chunk_uploaded(&pending_chunk_info.file_info, pending_chunk_info.chunk_seq, pending_chunk_info.chunk_size, task_mgr.as_ref(), &task_info, remote_task_id, true).await {
                                                            return BackupTaskEvent::ErrorAndWillRetry(self.clone(), Arc::new(err));
                                                        }
                                                    },
                                                    Err(err) => {
                                                        log::error!("upload chunk failed: {:?}", err);
                                                        return BackupTaskEvent::ErrorAndWillRetry(self.clone(), err);
                                                    }
                                                }
                                            }
                                        },
                                    }
                                }

                                match remote_task_mgr.set_files_prepare_ready(remote_task_id).await {
                                    Ok(_) => {
                                        match task_storage.set_files_prepare_ready(task_info.task_id, FilesReadyState::RemoteReady).await  {
                                            Ok(_) => {
                                                // TODO: remove task from storage
                                                return BackupTaskEvent::Successed(self.clone());
                                            }
                                            Err(err) => return BackupTaskEvent::ErrorAndWillRetry(self.clone(), Arc::new(err))
                                        }
                                    }
                                    Err(err) => return BackupTaskEvent::ErrorAndWillRetry(self.clone(), Arc::new(err))
                                }
                            },
                            FilesReadyState::RemoteReady => return BackupTaskEvent::Successed(self.clone()),
                        }
                    }
                    files
                }
                Err(err) => return BackupTaskEvent::ErrorAndWillRetry(self.clone(), Arc::new(err)),
            };

            for mut file in upload_files {
                file_seq = std::cmp::max(file.file_seq as usize + 1, file_seq);
                self.pending_files.lock().await.insert(file.file_path.clone(), (file.clone(), 0));
                let (file_server_type, file_server_name, remote_file_id, chunk_size) = match task_storage
                    .is_file_info_pushed(
                        &task_info.task_key,
                        task_info.check_point_version,
                        file.file_path.as_path(),
                    )
                    .await
                {
                    Ok(file_server_name) => match file_server_name {
                        Some(file_server_name) => file_server_name,
                        None => {
                            match remote_task_mgr
                                .add_file(
                                    remote_task_id,
                                    file.file_seq,
                                    file.file_path.as_path(),
                                    file.hash.as_str(),
                                    file.file_size,
                                )
                                .await
                            {
                                Ok((file_server_type, file_server_name, remote_file_id, chunk_size)) => {
                                    match task_storage
                                        .set_file_info_pushed(
                                            &task_info.task_key,
                                            task_info.check_point_version,
                                            file.file_path.as_path(),
                                            file_server_type,
                                            file_server_name.as_str(),
                                            remote_file_id,
                                            chunk_size,
                                        )
                                        .await
                                    {
                                        Ok(_) => (file_server_type, file_server_name, remote_file_id, chunk_size),
                                        Err(err) => {
                                            log::error!("set file info pushed failed: {:?}", err);
                                            return BackupTaskEvent::ErrorAndWillRetry(
                                                self.clone(), Arc::new(err)
                                            )
                                        }
                                    }
                                }
                                Err(err) => {
                                    log::error!("add file to remote server failed: {:?}", err);
                                    return BackupTaskEvent::ErrorAndWillRetry(self.clone(), Arc::new(err))
                                }
                            }
                        }
                    },
                    Err(err) => {
                        log::error!("is file info pushed failed: {:?}", err);
                        return BackupTaskEvent::ErrorAndWillRetry(self.clone(), Arc::new(err));
                    },
                };

                file.file_server = Some((file_server_type, file_server_name, Some((remote_file_id, chunk_size))));
                let file_server = file.file_server.as_ref().unwrap();
                let remote_file_server = match task_mgr
                    .file_mgr_selector()
                    .select_by_name(file_server.0, file_server.1.as_str())
                    .await
                {
                    Ok(remote_file_server) => remote_file_server,
                    Err(err) => {
                        log::error!("select file server failed: {:?}", err);
                        return BackupTaskEvent::ErrorAndWillRetry(self.clone(), Arc::new(err));
                    },
                };

                // push chunks
                let file_storage = task_mgr.file_storage();
                let chunk_size = chunk_size as u64;
                let chunk_count = (file.file_size + chunk_size - 1) / chunk_size;
                let file_path = task_info.dir_path.join(file.file_path.as_path());

                let mut chunk_seq = 0;
                while chunk_seq < chunk_count {
                    let offset = chunk_seq * chunk_size;
                    let chunk_size = std::cmp::min(chunk_size, file.file_size - offset);
                    let (chunk_server_type, chunk_server_name, chunk_hash, chunk, _todo_remote_chunk_id) =
                        match file_storage
                            .is_chunk_info_pushed(&task_info.task_key, task_info.check_point_version, file.file_path.as_path(), chunk_seq)
                            .await
                        {
                            Ok(chunk_server) => match chunk_server {
                                Some((chunk_server_type, chunk_server_name, chunk_hash, remote_chunk_id)) => {
                                    (chunk_server_type, chunk_server_name, chunk_hash, None, remote_chunk_id)
                                }
                                None => {
                                    match read_file_from(file_path.as_path(), offset, chunk_size)
                                        .await
                                    {
                                        Ok(chunk) => {
                                            let mut hasher = sha2::Sha256::new();
                                            hasher.update(chunk.as_slice());
                                            let hash = hasher.finalize();
                                            let hash = hash.as_slice().to_base58();
                                            match remote_file_server
                                                .add_chunk(
                                                    remote_file_id,
                                                    chunk_seq,
                                                    hash.as_str(),
                                                    chunk_size as u32,
                                                )
                                                .await
                                            {
                                                Ok((chunk_server_type, chunk_server_name, remote_chunk_id)) => {
                                                    match file_storage
                                                        .set_chunk_info_pushed(
                                                            &task_info.task_key,
                                                            task_info.check_point_version,
                                                            file.file_path.as_path(),
                                                            chunk_seq,
                                                            chunk_server_type,
                                                            chunk_server_name.as_str(),
                                                            hash.as_str(),
                                                            remote_chunk_id,
                                                        )
                                                        .await
                                                    {
                                                        Ok(_) => (
                                                            chunk_server_type,
                                                            chunk_server_name,
                                                            hash,
                                                            Some(chunk),
                                                            remote_chunk_id,
                                                        ),
                                                        Err(err) => {
                                                            log::error!("set chunk info pushed failed: {:?}", err);
                                                            return 
                                                                BackupTaskEvent::ErrorAndWillRetry(
                                                                    self.clone(), Arc::new(err)
                                                                );
                                                            
                                                        }
                                                    }
                                                }
                                                Err(err) => {
                                                    log::error!("add chunk to remote server failed: {:?}", err);
                                                    return BackupTaskEvent::ErrorAndWillRetry(
                                                        self.clone(), Arc::new(err)
                                                    )
                                                }
                                            }
                                        }
                                        Err(err) => {
                                            log::error!("read chunk file failed: {:?}, path: {:?}, chunk: {}, offset: {}, len: {}", err, file_path, chunk_seq, offset, chunk_size);
                                            return BackupTaskEvent::ErrorAndWillRetry(
                                                self.clone(), Arc::new(err)
                                            )
                                        }
                                    }
                                }
                            },
                            Err(err) => {
                                log::error!("is chunk info pushed failed: {:?}", err);
                                return BackupTaskEvent::ErrorAndWillRetry(self.clone(), Arc::new(err))
                            }
                        };

                    let chunk_storage = task_mgr.chunk_storage();
                    match chunk_storage.is_chunk_uploaded(&task_info.task_key, task_info.check_point_version, file.file_path.as_path(), chunk_seq).await {
                        Ok(is_upload) => {
                            if is_upload {
                                if let Err(err) = self.post_chunk_uploaded(&file, chunk_seq, chunk_size as u32, task_mgr.as_ref(), &task_info, remote_task_id, false).await {
                                    return BackupTaskEvent::ErrorAndWillRetry(self.clone(), Arc::new(err));
                                }
                                chunk_seq += 1;
                                continue;
                            }
                        }

                        Err(err) => {
                            log::error!("is chunk uploaded failed: {:?}", err);
                            return BackupTaskEvent::ErrorAndWillRetry(self.clone(), Arc::new(err));
                        },
                    }

                    let remote_chunk_server = match task_mgr
                        .chunk_mgr_selector()
                        .select_by_name(chunk_server_type, chunk_server_name.as_str())
                        .await
                    {
                        Ok(remote_chunk_server) => remote_chunk_server,
                        Err(err) => {
                            log::error!("select chunk server failed: {:?}", err);
                            return BackupTaskEvent::ErrorAndWillRetry(self.clone(), Arc::new(err));
                        },
                    };

                    let chunk = match chunk {
                        Some(chunk) => chunk,
                        None => match read_file_from(file_path.as_path(), offset, chunk_size).await
                        {
                            Ok(chunk) => {
                                log::info!("read chunk file success: path: {:?}, chunk: {}, offset: {}, len: {}", file_path, chunk_seq, offset, chunk_size);
                                chunk
                            },
                            Err(err) => {
                                log::error!("read chunk file failed: {:?}, path: {:?}, chunk: {}, offset: {}, len: {}", err, file_path, chunk_seq, offset, chunk_size);
                                return BackupTaskEvent::ErrorAndWillRetry(self.clone(), Arc::new(err))
                            }
                        },
                    };

                    match self.transfer_chunk(chunk, chunk_hash, task_info.priority, remote_chunk_server, &file, chunk_seq, task_mgr.as_ref(), &task_info, remote_task_id).await {
                        Ok(is_pending) => {
                            if is_pending {
                                chunk_seq += 1;
                            }
                            continue;
                        }
                        Err(e) => return e
                    }
                }
            }
        }
    }

    async fn transfer_chunk(&self, chunk: Vec<u8>, hash: String, priority: u32, target_server: Box<dyn ChunkMgr>, file_info: &FileInfo, chunk_seq: u64, task_mgr: &BackupTaskMgrInner, task_info: &TaskInfo, remote_task_id: TaskId) -> Result<bool, BackupTaskEvent> {
        let chunk_size = chunk.len() as u32;
        let chunk_size_kb = std::cmp::max(3, chunk_size >> 10); // Convert chunk size to kilobytes
        let timeout = task_mgr.timeout_per_kb() * chunk_size_kb as u32;
        match self.transfer.push(upload_chunk, UploadChunkParam {hash, chunk, target_server}, priority, timeout).await {
            Ok(pending_waiter) => {
                self.pending_waiters.lock().await.push((pending_waiter, Some(PendingChunkInfo {
                    chunk_size,
                    chunk_seq,
                    file_info: file_info.clone(),
                })));
                Ok(true)
            },
            Err((wait_event, _)) => {
                // 1. wait event to continue; wait_event.recv().await
                // 2. wait control event to stop; self.control.1.recv().await
                // 3. wait pending event to check result; self.pending_event.recv().await

                loop {
                    let task_state_fut = self.state.0.wait(|s| *s != TaskState::Running);
                    let wait_event_fut = wait_event.wait(|s| s.is_some());    //     let complete_result = {
                    let pending_waiters_fut = {
                        let pending_waiters = self.pending_waiters.lock().await;
                        futures::future::select_all(pending_waiters.iter().map(|(waiter, _)| waiter.wait(|s| s.is_some())))
                    };
                
                    // pin_mut!(task_state_fut);
                    // pin_mut!(wait_event_fut);
                    // pin_mut!(pending_waiters_fut);

                    select! {
                        _ = wait_event_fut => {
                            // Handle wait event
                            return Ok(false)
                        },
                        _ = task_state_fut => {
                            // Handle control event
                            return Err(BackupTaskEvent::Stop(self.clone()))
                        },
                        complete_result = pending_waiters_fut => {
                            // Handle pending event
                            let (result, index, _) = complete_result;
                            if let Some(result) = result {
                                let pending_chunk_info = self.pending_waiters.lock().await.remove(index).1;
                                let pending_chunk_info = pending_chunk_info.expect("there is a long pending waiter, should be not wake.");
                                log::info!("pending chunk waited: {:?}, seq: {}, result: {:?}", pending_chunk_info.file_info.file_path, pending_chunk_info.chunk_seq, result);
                                match result {
                                    Ok(_) => {
                                        if let Err(err) = self.post_chunk_uploaded(&pending_chunk_info.file_info, pending_chunk_info.chunk_seq, pending_chunk_info.chunk_size, task_mgr, &task_info, remote_task_id, true).await {
                                            return Err(BackupTaskEvent::ErrorAndWillRetry(self.clone(), Arc::new(err)));
                                        }
                                    },
                                    Err(err) => {
                                        log::error!("upload chunk failed: {:?}", err);
                                        return Err(BackupTaskEvent::ErrorAndWillRetry(self.clone(), err));
                                    }
                                }
                            }
                        },
                    }
                }
            },
        }
    }

    async fn post_chunk_uploaded(&self, file_info: &FileInfo, chunk_seq: u64, chunk_size: u32, task_mgr: &BackupTaskMgrInner, task_info: &TaskInfo, remote_task_id: TaskId, is_transfer: bool) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let remote_task_mgr = task_mgr.task_mgr_selector().select(&task_info.task_key, Some(task_info.check_point_version)).await?;
        let chunk_storage = task_mgr.chunk_storage();

        let mut pending_files = self.pending_files.lock().await;
        let pending_file_info = pending_files.get_mut(&file_info.file_path).unwrap();
        pending_file_info.1 += chunk_size as u64;


        log::info!("chunk uploaded: task: {:?}, file: {:?}, seq: {:?}, is_transefer: {}", task_info.task_id, file_info.file_path, chunk_seq, is_transfer);

        if is_transfer {
            let remove_file_server_info = file_info.file_server.as_ref().unwrap();
            let remote_file_server = task_mgr.file_mgr_selector().select_by_name(remove_file_server_info.0, remove_file_server_info.1.as_str()).await?;
            log::info!("set chunk uploaded(remote): task: {:?}, file: {:?}, seq: {:?}", task_info.task_id, file_info.file_path, chunk_seq);
            if let Err(err) = remote_file_server.set_chunk_uploaded(remove_file_server_info.2.unwrap().0, chunk_seq).await {
                log::info!("set chunk uploaded(remote) failed: {:?}, task: {:?}, file: {:?}, seq: {:?}", err, task_info.task_id, file_info.file_path, chunk_seq);

                return Err(err);
            }
            log::info!("set chunk uploaded(remote) success: task: {:?}, file: {:?}, seq: {:?}", task_info.task_id, file_info.file_path, chunk_seq);
        }

        if pending_file_info.1 >= pending_file_info.0.file_size {
            log::info!("set file uploaded: task: {:?}, file: {:?}, seq: {}", task_info.task_id, pending_file_info.0.file_path, chunk_seq);
            if let Err(err) = remote_task_mgr.set_file_uploaded(
                remote_task_id,
                pending_file_info.0.file_path.as_path()
            ).await {
                log::error!("set file uploaded failed: task: {:?}, file: {:?}, seq: {}, {:?}", task_info.task_id, file_info.file_path, chunk_seq, err);
                return Err(err);
            }
            let _todo_ = tokio::fs::remove_file(pending_file_info.0.file_path.as_path()).await;
        }

        if is_transfer {
            log::info!("set chunk uploaded(local): task: {:?}, file: {:?}, seq: {:?}", task_info.task_id, file_info.file_path, chunk_seq);
            if let Err(err) = chunk_storage.set_chunk_uploaded(&task_info.task_key, task_info.check_point_version, pending_file_info.0.file_path.as_path(), chunk_seq).await {
                log::info!("set chunk uploaded(local) failed: {:?}, task: {:?}, file: {:?}, seq: {:?}", err, task_info.task_id, file_info.file_path, chunk_seq);
                return Err(err);
            }
            log::info!("set chunk uploaded(local) success: task: {:?}, file: {:?}, seq: {:?}", task_info.task_id, file_info.file_path, chunk_seq);
        }
        Ok(())
    }

    // async fn wait_pending_chunk(&self, task_mgr: &BackupTaskMgrInner, task_info: &TaskInfo, remote_task_id: TaskId) -> Result<Option<()>, BackupTaskEvent> {
    //     let complete_result = {
    //         let pending_waiters = self.pending_waiters.lock().await;
    //         futures::future::select_all(pending_waiters.iter().map(|(waiter, _)| waiter.wait(|s| s.is_some()))).await
    //     };

    //     let (result, index, _) = complete_result;
    //     match result {
    //         Some(result) => {
    //             let pending_chunk_info = self.pending_waiters.lock().await.get(index).unwrap().1.clone();
    //             let pending_chunk_info = pending_chunk_info.expect("there is a long pending waiter, should be not wake.");
    //             log::info!("pending chunk waited: {:?}, seq: {}, result: {:?}", pending_chunk_info.file_info.file_path, pending_chunk_info.chunk_seq, result.map(|_| "..."));
    //             let result = match result {
    //                 Ok(_) => {
    //                     if let Err(err) = self.post_chunk_uploaded(&pending_chunk_info.file_info, pending_chunk_info.chunk_seq, pending_chunk_info.chunk_size, task_mgr, &task_info, remote_task_id, true).await {
    //                         Err(BackupTaskEvent::ErrorAndWillRetry(self.clone(), Arc::new(err)))
    //                     } else {
    //                         Ok(Some(()))
    //                     }
    //                 },
    //                 Err(err) => {
    //                     log::error!("upload chunk failed: {:?}", err);
    //                     Err(BackupTaskEvent::ErrorAndWillRetry(self.clone(), err))
    //                 }
    //             };

    //             self.pending_waiters.lock().await.remove(index);
    //             result
    //         },
    //         None => Ok(None),
    //     }
    // }

    // [path, Option<(hash, file-size)>]
    pub(crate) async fn add_files(&self, _todo_files: Vec<(PathBuf, Option<(String, u64)>)>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        unimplemented!()
    }
}

async fn read_file_from(
    file_path: &std::path::Path,
    offset: u64,
    len: u64,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let mut file = tokio::fs::File::open(file_path).await?;
    file.seek(std::io::SeekFrom::Start(offset)).await?;

    let mut buf = Vec::with_capacity(len as usize);
    unsafe {
        buf.set_len(len as usize);
    }
    file.read_exact(buf.as_mut_slice()).await?;

    Ok(buf)
}

impl Task for BackupTask {
    fn task_key(&self) -> TaskKey {
        self.info.lock().unwrap().task_key.clone()
    }

    fn task_id(&self) -> TaskId {
        self.info.lock().unwrap().task_id.clone()
    }

    fn check_point_version(&self) -> CheckPointVersion {
        self.info.lock().unwrap().check_point_version
    }

    fn prev_check_point_version(&self) -> Option<CheckPointVersion> {
        self.info.lock().unwrap().prev_check_point_version
    }

    fn meta(&self) -> Option<String> {
        self.info.lock().unwrap().meta.clone()
    }

    fn dir_path(&self) -> PathBuf {
        self.info.lock().unwrap().dir_path.clone()
    }

    fn is_all_files_ready(&self) -> FilesReadyState {
        self.info.lock().unwrap().is_all_files_ready
    }

    fn is_all_files_done(&self) -> bool {
        let info = self.info.lock().unwrap();
        info.file_count == info.complete_file_count as usize
    }

    fn file_count(&self) -> usize {
        self.info.lock().unwrap().file_count
    }
}

#[async_trait::async_trait]
impl TaskInner for BackupTask {
    async fn start(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let backup_task = self.clone();
        tokio::task::spawn(async move {
            let task_mgr = backup_task.mgr.upgrade();
            let task_mgr = match task_mgr {
                Some(task_mgr) => task_mgr,
                None => {
                    log::error!("task manager has been dropped.");
                    return;
                }
            };

            // // run once
            let state = backup_task.run_once().await;
            log::info!("task run once done: {:?}", backup_task.task_id());
            
            match state {
                BackupTaskEvent::New(_) => unreachable!(),
                BackupTaskEvent::Idle(_) => {},
                BackupTaskEvent::ErrorAndWillRetry(_, _) => {
                    let (task_key, check_point_version) = {
                        let task_info = backup_task.info.lock().unwrap();
                        (task_info.task_key.clone(), task_info.check_point_version)
                    };
                    
                    let _ = task_mgr.task_storage().set_task_last_try_fail_time(&task_key, check_point_version).await;
                }
                BackupTaskEvent::Fail(_, _) => {
                    let (task_key, check_point_version) = {
                        let task_info = backup_task.info.lock().unwrap();
                        (task_info.task_key.clone(), task_info.check_point_version)
                    };
                    
                    let _ = task_mgr.task_storage().set_task_last_try_fail_time(&task_key, check_point_version).await;
                },
                BackupTaskEvent::Successed(_) => {},
                BackupTaskEvent::Stop(_) => {
                    backup_task.state.1.set(TaskState::Stopped);
                },
            }
            
            if let Some(task_event_sender) = task_mgr.task_event_sender().await {
                task_event_sender.send(state.clone())
                .await
                .expect("todo: channel overflow");
            } else {
                log::error!("task manager has stopped.");
                return;
            }
        });

        Ok(())
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.state.1.set(TaskState::Stopping);
        Ok(())
    }
}
