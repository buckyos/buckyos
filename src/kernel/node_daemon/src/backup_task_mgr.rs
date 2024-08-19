use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Weak},
};

use base58::ToBase58;
use sha2::Digest;
use tokio::{io::AsyncReadExt, sync::Mutex};

use crate::backup_task_storage::{BackupChunkInfo, BackupTaskInfo, BackupTaskStorage};

/**
 * 一般流程：
 *
 * let url = "http://xxx.yyy.zzz:pppp";
 * let storage_dir_path = "/your/storage/dir";
 * let zone_id = "your-zone-id";
 * let task_mgr = TaskManager::new(zone_id, url, storage_dir_path);
 *
 * let app_key = "your-unique-key";
 * let current_version = task_mgr.get_last_version(app_key).await?.version + 1; // 递增的版本号，版本号是不是可以取个类似“COMMIT POINT”的名字？
 * let prev_version = None; // 如果是前一个版本的增量，填写Some(current_version - 1)
 * let mut meta = Some("your-app-attachment-with-the-version"); // 和该版本对应的APP自定义附加信息
 *
 * let backup_task = task_mgr.create_new_backup_task(app_key, current_version, prev_version, meta).await?;
 * let mut last_attribution = "your-app-attachment-with-the-chunk"; // 和该chunk对应的APP自定义附加信息，主要用于APP记录传输状态（比如：用于续传的进度）
 * // 或者是续传任务，从本地数据库里枚举自己的续传任务，从中挑选自己需要的，一般是最新一个版本
 * // let backup_task = task_mgr.continue_tasks(app_key).await?;
 * // let mut last_attribution = backup_task.chunk_attribution(None).await?;
 *
 * while !app.is_all_chunks_ready() {
 *    let chunk_file_path = app.generate_next_chunk_file(last_attribution).await?;
 *    backup_task.append_chunk_files(&[chunk_file_path], last_attribution).await?;
 *    last_attribution = "new-attribution-for-next-chunk";
 * }
 *
 * // 有的任务备份任务要时间比较久，最开始无法完全得到最终的meta信息，所以可以在所有chunk都准备好后再上传meta信息
 * meta = Some("you can upload the metadata when the task is done.");
 * backup_task.all_chunks_ready(meta).await?;
 * backup_task.wait_done().await?;
 *
 * // 如果上传出现错误，以`chunk-file`为单位重传，先不考虑内部分片的精细化逻辑，服务端做好幂等性处理.
 *
 * // 备份中断，有的情况可能无法恢复备份，之前备份的数据没有意义，需要取消任务并删除所有`chunk-file`和已经备份到服务器的任务，暂不实现
 * // 任务完成后，可以删除任务，也可以保留任务，暂时不实现
 * // 备份服务器上的历史版本，很多都是过期的，不再有用，也需要管理和清理，暂不实现
 * // 不管备份成功还是失败，version都应该一直递增，避免重复
 *
 * // 内部实现细节
 * 1. 每个`chunk-file`都先记录到数据库
 * 2. 上传任务按顺序每次从数据库里读取一个`chunk-file`发起上传，同时最多发起5个`chunk-file`上传
*/

pub enum RemoveTaskOption {
    // remove all versions earlier than the specified version.
    RemoveAllEarlierVersions(u32),
    // remove all versions in the prev-versions link.
    RemoveAllPrevVersions(u32),
    // remove the specified version only.
    RemoveSpecificVersions(u32),
}

#[derive(Clone)]
pub struct BackupTask {
    mgr: Weak<TaskManager>,
    info: Arc<Mutex<BackupTaskInfo>>,
    uploading_chunks: Arc<Mutex<Vec<BackupChunkInfo>>>,
}

impl BackupTask {
    fn from_storage(mgr: Weak<TaskManager>, info: BackupTaskInfo) -> Self {
        Self {
            mgr,
            info: Arc::new(Mutex::new(info)),
            uploading_chunks: Arc::new(Mutex::new(Vec::new())),
        }
    }

    async fn create_new(
        mgr: Weak<TaskManager>,
        zone_id: String,
        key: String,
        version: u32,
        prev_version: Option<u32>,
        meta: Option<String>,
        dir_path: PathBuf,
        chunk_files: Vec<PathBuf>,
    ) -> Result<BackupTask, Box<dyn std::error::Error>> {
        let dir_path_str = dir_path.to_str().expect("only utf-8 for dir-path");
        let chunk_file_info_rets =
            futures::future::join_all(chunk_files.into_iter().enumerate().map(
                |(chunk_seq, chunk_relative_path)| {
                    let dir_path = dir_path.clone();
                    async move {
                        let chunk_full_path = dir_path.join(&chunk_relative_path);
                        let mut file = tokio::fs::File::open(chunk_full_path).await?;
                        let mut buf = vec![];
                        file.read_to_end(&mut buf).await?;

                        let mut hasher = sha2::Sha256::new();
                        hasher.update(buf.as_slice());
                        let hash = hasher.finalize();
                        let hash = hash.as_slice().to_base58();

                        Ok(BackupChunkInfo {
                            task_id: 0,
                            chunk_seq: chunk_seq as u32,
                            file_path: chunk_relative_path
                                .to_str()
                                .expect("only utf-8 for chunk-path")
                                .to_string(),
                            hash,
                            chunk_size: buf.len() as u32,
                        })
                    }
                },
            ))
            .await;

        let mut chunk_files = vec![];
        for info in chunk_file_info_rets {
            match info {
                Err(err) => {
                    log::error!("read chunk files failed: {:?}", err);
                    return Err(err);
                }
                Ok(r) => chunk_files.push(r),
            }
        }

        let storage_arc = mgr.upgrade().unwrap().storage();
        let storage = storage_arc.lock().await;
        let task_id = storage.insert_upload_task(
            &zone_id,
            &key,
            version,
            prev_version,
            meta.as_ref().map(|s| s.as_str()),
            dir_path_str,
        )?;

        for chunk_info in chunk_files.iter() {
            storage.add_upload_chunk(
                task_id,
                chunk_info.chunk_seq,
                chunk_info.file_path.as_str(),
                chunk_info.hash.as_str(),
                chunk_info.chunk_size,
            )?;
        }

        let chunk_count = chunk_files.len();
        unsafe {
            chunk_files.set_len(std::cmp::min(chunk_count, 5));
        }

        Ok(Self {
            mgr,
            info: Arc::new(Mutex::new(BackupTaskInfo {
                task_id,
                zone_id,
                key,
                version,
                prev_version,
                meta: meta.unwrap_or("".to_string()),
                dir_path: dir_path_str.to_string(),
                is_all_chunks_ready: false,
                is_all_chunks_backup_done: false,
                chunk_count: chunk_count as u32,
            })),
            uploading_chunks: Arc::new(Mutex::new(chunk_files)),
        })
    }

    // append new chunks to the task in order.
    pub async fn append_chunk_files(
        &self,
        chunk_file_paths: &[&Path],
        attribution: Option<String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        unimplemented!()
    }

    // all chunks has appended to the task, and there will be no chunks any more to append.
    pub async fn all_chunks_ready(
        &self,
        meta: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        unimplemented!()
    }

    // should not append any chunks to the task when all chunks ready.
    pub async fn is_all_chunks_ready(&self) -> bool {
        unimplemented!()
    }

    // all chunks appended to the task has been uploaded to the backup server.
    // but if a new chunk is appended to the task, the task will be not done.
    // is_all_chunks_ready && is_all_chunks_backup_done == true => the task is done.
    pub async fn is_all_chunks_backup_done(&self) -> bool {
        unimplemented!()
    }

    pub async fn wait_done(&self) -> Result<(), Box<dyn std::error::Error>> {
        unimplemented!()
    }

    // remove the task and all chunks files(if is_delete_chunk_files is true).
    // if the task is referenced by any follow tasks, the function will fail.
    pub async fn cancel(
        &self,
        is_delete_chunk_files: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        unimplemented!()
    }

    pub async fn key(&self) -> &str {
        unimplemented!()
    }

    pub async fn version(&self) -> u32 {
        unimplemented!()
    }

    pub async fn prev_version(&self) -> Option<u32> {
        unimplemented!()
    }
    pub async fn metadata(&self) -> Option<String> {
        unimplemented!()
    }
    pub async fn chunk_attribution(&self, chunk_seq: Option<u32>) -> Option<String> {
        unimplemented!()
    }
}

struct BackupTaskMap {
    task_ids: HashMap<String, HashMap<u32, i64>>, // key -> version -> task_id
    tasks: HashMap<i64, BackupTask>,              // task_id -> task
}

pub struct TaskManager {
    url: String,
    zone_id: String,
    storage_dir_path: PathBuf,
    storage: Arc<Mutex<BackupTaskStorage>>,

    tasks: Arc<Mutex<BackupTaskMap>>,
}

impl TaskManager {
    pub fn new(
        zone_id: String,
        url: String,
        storage_dir_path: PathBuf,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let storage = BackupTaskStorage::new_with_path(
            storage_dir_path
                .to_str()
                .expect("only utf-8 for database path."),
        )?;

        Ok(TaskManager {
            tasks: Arc::new(Mutex::new(BackupTaskMap {
                task_ids: HashMap::new(),
                tasks: HashMap::new(),
            })),
            url,
            zone_id,
            storage_dir_path,
            storage: Arc::new(Mutex::new(storage)),
        })
    }

    pub async fn continue_tasks(
        &self,
        key: &str,
    ) -> Result<Vec<BackupTask>, Box<dyn std::error::Error>> {
        unimplemented!("")
    }

    pub async fn get_last_version(&self) -> Result<BackupTaskInfo, Box<dyn std::error::Error>> {
        unimplemented!("")
    }

    pub async fn create_new_backup_task(
        &self,
        key: &str,
        version: u32,
        prev_version: Option<u32>,
        meta: Option<&str>,
    ) -> Result<BackupTask, Box<dyn std::error::Error>> {
        unimplemented!("")
    }

    pub async fn enumerate_backup_tasks(
        &self,
        key: &str,
    ) -> Result<Vec<BackupTask>, Box<dyn std::error::Error>> {
        unimplemented!("")
    }

    // remove the backup versions on the backup server.
    // any version referenced by other versions not in the target versions will not be removed.
    pub async fn remove_remote_backup(
        &self,
        key: &str,
        version_option: RemoveTaskOption,
    ) -> Result<(), Box<dyn std::error::Error>> {
        unimplemented!("")
    }

    // remove the backup versions at local.
    // any version referenced by other versions not in the target versions will not be removed.
    pub async fn remove_local_backup(
        &self,
        key: &str,
        version_option: RemoveTaskOption,
    ) -> Result<(), Box<dyn std::error::Error>> {
        unimplemented!("")
    }

    fn storage(&self) -> Arc<Mutex<BackupTaskStorage>> {
        self.storage.clone()
    }
}
