use std::{
    path::PathBuf,
    sync::{Arc, Mutex, Weak},
};

use base58::ToBase58;
use sha2::Digest;
use tokio::io::AsyncReadExt;

use crate::{
    chunk_storage::ChunkInfo,
    file_storage::FileInfo,
    task_storage::{CheckPointVersion, TaskId, TaskInfo, TaskKey},
};

pub trait Task {
    fn task_key(&self) -> TaskKey;
    fn task_id(&self) -> TaskId;
    fn check_point_version(&self) -> CheckPointVersion;
    fn prev_check_point_version(&self) -> Option<CheckPointVersion>;
    fn meta(&self) -> Option<String>;
    fn dir_path(&self) -> PathBuf;
    fn is_all_files_ready(&self) -> bool;
    fn is_all_files_done(&self) -> bool;
    fn file_count(&self) -> usize;
}

#[derive(Clone)]
pub struct BackupTask {
    mgr: Weak<BackupTaskMgrInner>,
    info: Arc<Mutex<TaskInfo>>,
    uploading_chunks: Arc<Mutex<Vec<ChunkInfo>>>,
}

impl BackupTask {
    pub(crate) fn from_storage(mgr: Weak<BackupTaskMgrInner>, info: TaskInfo) -> Self {
        Self {
            mgr,
            info: Arc::new(Mutex::new(info)),
            uploading_chunks: Arc::new(Mutex::new(Vec::new())),
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
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let chunk_file_info_rets = futures::future::join_all(files.into_iter().map(
            |(chunk_relative_path, hash_and_size)| {
                let dir_path = dir_path.clone();
                async move {
                    match hash_and_size {
                        Some((hash, file_size)) => Ok(FileInfo {
                            task_id: TaskId::from(0),
                            file_path: chunk_relative_path,
                            hash,
                            file_size,
                        }),
                        None => {
                            // TODO: read by chunks
                            let chunk_full_path = dir_path.join(&chunk_relative_path);
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
                                file_path: chunk_relative_path,
                                hash,
                                file_size,
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

        let storage = mgr
            .upgrade()
            .map_or(
                Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "maybe the system has stopped.",
                ))),
                |t| Ok(t),
            )?
            .storage();

        let task_id = storage
            .create_task_with_files(
                &task_key,
                check_point_version,
                prev_check_point_version,
                meta.as_ref().map(|p| p.as_str()),
                dir_path.as_path(),
                files.as_slice(),
            )
            .await?;

        Ok(Self {
            mgr,
            info: Arc::new(Mutex::new(TaskInfo {
                task_id,
                task_key,
                check_point_version,
                prev_check_point_version,
                meta,
                dir_path: dir_path,
                is_all_files_ready: false,
                is_all_files_done: false,
                file_count: files.len(),
            })),
            uploading_chunks: Arc::new(Mutex::new(vec![])),
        })
    }
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

    fn is_all_files_ready(&self) -> bool {
        self.info.lock().unwrap().is_all_files_ready
    }

    fn is_all_files_done(&self) -> bool {
        self.info.lock().unwrap().is_all_files_done
    }

    fn file_count(&self) -> usize {
        self.info.lock().unwrap().file_count
    }
}

pub struct RestoreTask {}

// impl Task for RestoreTask {}
