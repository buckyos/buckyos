use std::{path::PathBuf, sync::Weak};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt}; // Import the AsyncWriteExt trait

use backup_lib::{check_chunk_hash, TaskInfo};

use crate::task_mgr::RestoreTaskMgrInner;

pub struct RestoreTask {
    mgr: Weak<RestoreTaskMgrInner>,
    task_mgr_server: Box<dyn backup_lib::TaskMgr>,
    task_info: TaskInfo,
    dir_path: PathBuf,
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
        })
    }

    pub(crate) async fn start(
        &self,
    ) -> Result<Vec<std::path::PathBuf>, Box<dyn std::error::Error + Send + Sync>> {
        let task_mgr = self.mgr.upgrade().ok_or("mgr is dropped")?;
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

            let (file_server_type, file_server_name, file_index) = file_info.file_server.unwrap();
            if file_index.is_none() {
                return Err("file index not found".into());
            }
            let (remote_file_id, chunk_size) = file_index.unwrap();
            let file_mgr_server = task_mgr
                .file_mgr_selector()
                .select_by_name(file_server_type, file_server_name.as_str())
                .await?;

            let file_path = self.dir_path.join(file_info.file_path.as_path());

            let mut file = tokio::fs::OpenOptions::new()
                .write(true)
                .read(true)
                .create(true)
                .open(file_path.as_path())
                .await
                .map_err(|err| {
                    log::error!("create file failed: {:?}, path: {:?}", err, file_path);
                    err
                })?;

            let mut writen_size = file.metadata().await?.len() as u64;
            let mut pos = 0;

            let chunk_size = chunk_size as u64;
            let chunk_count = (file_info.file_size + chunk_size - 1) / chunk_size;

            for chunk_seq in 0..chunk_count {
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
                        file.read_exact(chunk.as_mut_slice()).await.is_ok()
                            && check_chunk_hash(chunk.as_slice(), chunk_info.hash.as_str())
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
                    let chunk = chunk_mgr_server.download(chunk_id).await?;

                    if chunk.len() as u64 != chunk_info.chunk_size as u64 {
                        return Err("chunk size not match".into());
                    }

                    file.seek(std::io::SeekFrom::Start(pos)).await?;
                    file.write_all(chunk.as_slice()).await?;

                    if !check_chunk_hash(chunk.as_slice(), chunk_info.hash.as_str()) {
                        return Err("chunk hash not match".into());
                    }
                }

                pos += chunk_info.chunk_size as u64;

                if pos > writen_size {
                    writen_size = pos;
                }
            }

            files.push(file_info.file_path);
        }

        Ok(files)
    }
}
