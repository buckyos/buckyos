use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Weak},
};
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    select,
    sync::Mutex,
}; // Import the AsyncWriteExt trait

use backup_lib::{check_chunk_hash, ChunkId, ChunkMgr, FileInfo, TaskInfo};

use crate::{chunk_transfer, task_mgr::RestoreTaskMgrInner};

#[derive(Clone)]
struct PendingChunkInfo {
    chunk_size: u32,
    chunk_seq: u64,
    file_info: FileInfo,
}

struct DownloadChunkParam {
    chunk_id: ChunkId,
    chunk_size: u32,
    hash: String,
    target_server: Box<dyn ChunkMgr>,
}

async fn download_chunk_proc(
    param: DownloadChunkParam,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let chunk = param.target_server.download(param.chunk_id).await?;
    if param.chunk_size as u64 != chunk.len() as u64 {
        return Err("chunk size not match".into());
    }
    if !check_chunk_hash(chunk.as_slice(), param.hash.as_str()) {
        return Err("chunk hash not match".into());
    }
    Ok(chunk)
}

struct PendingFileInfo {
    file_handle: Arc<tokio::sync::Mutex<tokio::fs::File>>,
    file_info: FileInfo,
    complete_size: u64,
    chunk_cache: Vec<(u64, Vec<u8>, bool)>, // <offset, chunk>
}

impl PendingFileInfo {
    fn push_chunk(&mut self, chunk_offset: u64, chunk: Vec<u8>, is_saved: bool) {
        if self.complete_size <= chunk_offset {
            let pos = self
                .chunk_cache
                .iter()
                .position(|(offset, _, _)| *offset >= chunk_offset);
            if let Some(pos) = pos {
                let target_pos_offset = self.chunk_cache[pos].0;
                if target_pos_offset == chunk_offset {
                    log::warn!("chunk cache conflict: {:?}", chunk_offset);
                } else {
                    self.chunk_cache
                        .insert(pos, (chunk_offset, chunk, is_saved));
                }
            } else {
                self.chunk_cache.push((chunk_offset, chunk, is_saved));
            }
        } else {
            log::warn!(
                "chunk has writen: file: {:?}, offset: {:?}",
                self.file_info.file_path,
                chunk_offset
            );
        }
    }

    async fn flush_in_order(&mut self) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let mut flush_chunk_count = 0;
        let mut handle = self.file_handle.lock().await;
        if let Some((offset, _, _)) = self.chunk_cache.first() {
            if *offset == self.complete_size {
                handle.seek(std::io::SeekFrom::Start(*offset)).await?;
            }
        }

        for (chunk_offset, chunk, is_saved) in self.chunk_cache.iter() {
            match self.complete_size.cmp(chunk_offset) {
                std::cmp::Ordering::Less => break,
                std::cmp::Ordering::Greater => {
                    unreachable!("chunk cache should be ordered")
                }
                std::cmp::Ordering::Equal => {
                    if *is_saved {
                        handle.seek(std::io::SeekFrom::Start(*chunk_offset)).await?;
                    } else {
                        handle.write_all(chunk.as_slice()).await?;
                    }
                    self.complete_size += chunk.len() as u64;
                    flush_chunk_count += 1;
                }
            }
        }
        if flush_chunk_count > 0 {
            self.chunk_cache.drain(0..flush_chunk_count);
        }

        if self.complete_size == self.file_info.file_size {
            handle.sync_all().await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

pub struct RestoreTask {
    mgr: Weak<RestoreTaskMgrInner>,
    task_mgr_server: Box<dyn backup_lib::TaskMgr>,
    task_info: TaskInfo,
    dir_path: PathBuf,
    pending_waiters: Arc<
        Mutex<
            Vec<(
                state_waiter::Waiter<
                    Option<Result<Vec<u8>, Arc<Box<dyn std::error::Error + Send + Sync>>>>,
                >,
                Option<PendingChunkInfo>,
            )>,
        >,
    >,
    pending_files: Arc<Mutex<HashMap<PathBuf, PendingFileInfo>>>, // <file-path, (FileInfo, complete_size)>
}

impl RestoreTask {
    pub(crate) async fn create_new(
        mgr: Weak<RestoreTaskMgrInner>,
        task_mgr_server: Box<dyn backup_lib::TaskMgr>,
        task_info: TaskInfo,
        dir_path: PathBuf,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Self {
            mgr,
            task_mgr_server,
            task_info,
            dir_path,
            pending_waiters: Arc::new(Mutex::new(Vec::new())),
            pending_files: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub(crate) async fn start(
        &self,
    ) -> Result<Vec<std::path::PathBuf>, Box<dyn std::error::Error + Send + Sync>> {
        let task_mgr = self.mgr.upgrade().ok_or("mgr is dropped")?;
        let chunk_transfer =
            chunk_transfer::ChunkTransfer::new(chunk_transfer::Config { limit: 8 });
        let mut files = Vec::new();

        for file_seq in 0..self.task_info.file_count {
            let file_info = self
                .task_mgr_server
                .get_file_info(task_mgr.zone_id(), self.task_info.task_id, file_seq as u64)
                .await?;

            if file_info.is_none() {
                return Err("file info not found".into());
            }

            let file_info = file_info.unwrap();
            if file_info.file_server.is_none() {
                return Err("file server not found".into());
            }

            let (file_server_type, file_server_name, file_index) =
                file_info.file_server.as_ref().unwrap();
            if file_index.is_none() {
                return Err("file index not found".into());
            }
            let (remote_file_id, chunk_size) = file_index.unwrap();
            let file_mgr_server = task_mgr
                .file_mgr_selector()
                .select_by_name(*file_server_type, file_server_name.as_str())
                .await?;

            let file_path = self.dir_path.join(file_info.file_path.as_path());

            let file = tokio::fs::OpenOptions::new()
                .write(true)
                .read(true)
                .create(true)
                .open(file_path.as_path())
                .await
                .map_err(|err| {
                    log::error!("create file failed: {:?}, path: {:?}", err, file_path);
                    err
                })?;

            let file = Arc::new(tokio::sync::Mutex::new(file));
            self.pending_files.lock().await.insert(
                file_info.file_path.clone(),
                PendingFileInfo {
                    file_handle: file.clone(),
                    file_info: file_info.clone(),
                    complete_size: 0,
                    chunk_cache: Vec::new(),
                },
            );

            let handle = file.lock().await;
            let mut writen_size = handle.metadata().await?.len() as u64;
            let mut pos = 0;

            let chunk_size = chunk_size as u64;
            let chunk_count = (file_info.file_size + chunk_size - 1) / chunk_size;
            let mut chunk_seq = 0;

            while chunk_seq < chunk_count {
                let chunk_info = file_mgr_server
                    .get_chunk_info(remote_file_id, chunk_seq)
                    .await?;

                if chunk_info.is_none() {
                    return Err("chunk info not found".into());
                }

                let chunk_info = chunk_info.unwrap();

                if chunk_info.chunk_size as u64 > chunk_size {
                    return Err("chunk size not match".into());
                }

                let is_local =
                    if pos < writen_size && pos + (chunk_info.chunk_size as u64) <= writen_size {
                        let mut chunk = vec![0u8; chunk_info.chunk_size as usize];
                        let mut handle = file.lock().await;
                        handle.seek(std::io::SeekFrom::Start(pos)).await?;
                        if handle.read_exact(chunk.as_mut_slice()).await.is_ok()
                            && check_chunk_hash(chunk.as_slice(), chunk_info.hash.as_str())
                        {
                            self.post_chunk_downloaded(&file_info, pos, chunk, true)
                                .await?;
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                if !is_local {
                    if chunk_info.chunk_server.is_none() {
                        return Err("chunk server not found".into());
                    }

                    let chunk_server = chunk_info.chunk_server.unwrap();
                    if chunk_server.2.is_none() {
                        return Err("chunk id not found".into());
                    }

                    let chunk_mgr_server = task_mgr
                        .chunk_mgr_selector()
                        .select_by_name(chunk_server.0, chunk_server.1.as_str())
                        .await?;

                    let chunk_id = chunk_server.2.unwrap();
                    if !self
                        .download_chunk(
                            chunk_id,
                            chunk_info.chunk_size,
                            chunk_info.hash.clone(),
                            chunk_mgr_server,
                            &file_info,
                            chunk_seq,
                            task_mgr.as_ref(),
                            &chunk_transfer,
                        )
                        .await?
                    {
                        continue;
                    }
                }

                chunk_seq += 1;
                pos += chunk_info.chunk_size as u64;

                if pos > writen_size {
                    writen_size = pos;
                }
            }

            files.push(file_info.file_path);
        }

        // wait all pending chunk
        loop {
            if self.pending_waiters.lock().await.is_empty() {
                break;
            }
            self.wait_pending_chunk().await?;
        }
        Ok(files)
    }

    async fn download_chunk(
        &self,
        chunk_id: ChunkId,
        chunk_size: u32,
        hash: String,
        target_server: Box<dyn ChunkMgr>,
        file_info: &FileInfo,
        chunk_seq: u64,
        task_mgr: &RestoreTaskMgrInner,
        transfer: &chunk_transfer::ChunkTransfer,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let chunk_size_kb = std::cmp::max(3, chunk_size >> 10); // Convert chunk size to kilobytes
        let timeout = task_mgr.timeout_per_kb() * chunk_size_kb as u32;
        match transfer
            .push(
                download_chunk_proc,
                DownloadChunkParam {
                    chunk_id,
                    chunk_size,
                    hash,
                    target_server,
                },
                0,
                timeout,
            )
            .await
        {
            Ok(pending_waiter) => {
                self.pending_waiters.lock().await.push((
                    pending_waiter,
                    Some(PendingChunkInfo {
                        chunk_size,
                        chunk_seq,
                        file_info: file_info.clone(),
                    }),
                ));
                Ok(true)
            }
            Err((wait_event, _)) => {
                // 1. wait event to continue; wait_event.recv().await
                // 2. wait pending event to check result; self.pending_event.recv().await

                loop {
                    let wait_event_fut = wait_event.wait(|s| s.is_some());
                    let pending_waiters_fut = self.wait_pending_chunk();

                    select! {
                        _ = wait_event_fut => {
                            // Handle wait event
                            return Ok(false)
                        },
                        result = pending_waiters_fut => {
                            // Handle pending event
                            result?;
                            continue;
                        },
                    }
                }
            }
        }
    }

    async fn post_chunk_downloaded(
        &self,
        file_info: &FileInfo,
        chunk_offset: u64,
        chunk: Vec<u8>,
        is_saved: bool,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut pending_files = self.pending_files.lock().await;
        let pending_file_info = pending_files.get_mut(&file_info.file_path).unwrap();

        log::info!(
            "chunk download: file: {:?}, offset: {:?}, is_saved: {}",
            file_info.file_path,
            chunk_offset,
            is_saved
        );

        pending_file_info.push_chunk(chunk_offset, chunk, is_saved);
        let is_complete = pending_file_info.flush_in_order().await?;
        if is_complete {
            pending_files.remove(&file_info.file_path);
        }

        Ok(())
    }

    async fn wait_pending_chunk(
        &self,
    ) -> Result<Option<()>, Box<dyn std::error::Error + Send + Sync>> {
        let complete_result = {
            let pending_waiters = self.pending_waiters.lock().await;
            futures::future::select_all(
                pending_waiters
                    .iter()
                    .map(|(waiter, _)| waiter.wait(|s| s.is_some())),
            )
            .await
        };

        let (result, index, _) = complete_result;
        match result {
            Some(result) => {
                let pending_chunk_info = self
                    .pending_waiters
                    .lock()
                    .await
                    .get(index)
                    .unwrap()
                    .1
                    .clone();
                let pending_chunk_info = pending_chunk_info
                    .expect("there is a long pending waiter, should be not wake.");
                log::info!(
                    "pending chunk waited: {:?}, seq: {}, result: {:?}",
                    pending_chunk_info.file_info.file_path,
                    pending_chunk_info.chunk_seq,
                    result
                );
                let result = match result {
                    Ok(chunk) => {
                        if let Err(err) = self
                            .post_chunk_downloaded(
                                &pending_chunk_info.file_info,
                                pending_chunk_info.chunk_seq
                                    * pending_chunk_info
                                        .file_info
                                        .file_server
                                        .as_ref()
                                        .unwrap()
                                        .2
                                        .as_ref()
                                        .unwrap()
                                        .1 as u64,
                                chunk,
                                false,
                            )
                            .await
                        {
                            Err(err)
                        } else {
                            Ok(Some(()))
                        }
                    }
                    Err(err) => {
                        log::error!("upload chunk failed: {:?}", err);
                        Err(Arc::try_unwrap(err).unwrap())
                    }
                };

                self.pending_waiters.lock().await.remove(index);
                result
            }
            None => Ok(None),
        }
    }
}
