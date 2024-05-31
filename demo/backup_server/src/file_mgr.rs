use std::path::Path;
use std::sync::Arc;
use backup_lib::{ChunkId, ChunkInfo, ChunkServerType, FileId, FileServerType, TaskServerType};
use tokio::sync::Mutex;

use crate::file_mgr_storage::FileStorageSqlite;

pub(crate) struct FileMgr {
    storage: Arc<Mutex<FileStorageSqlite>>,
    chunk_mgr_selector: Arc<dyn backup_lib::ChunkMgrServerSelector>,
}

impl FileMgr {
    pub(crate) fn new(storage: FileStorageSqlite, chunk_mgr_selector: Arc<dyn backup_lib::ChunkMgrServerSelector>) -> Self {
        Self { storage: Arc::new(Mutex::new(storage)), chunk_mgr_selector }
    }

    pub(crate) fn chunk_size(&self) -> u32 {
        // TODO: read from config
        1024 * 1024 * 16
    }
}

#[async_trait::async_trait]
impl backup_lib::FileMgrServer for FileMgr {
    async fn add_file(
        &self,
        task_server_type: TaskServerType,
        task_server_name: &str,
        file_hash: &str,
        file_size: u64,
    ) -> Result<(FileId, u32), Box<dyn std::error::Error + Send + Sync>> {
        self.storage.lock().await.insert_file(task_server_type, task_server_name, file_hash, file_size, self.chunk_size())
    }
}

#[async_trait::async_trait]
impl backup_lib::FileMgr for FileMgr {
    fn server_type(&self) -> FileServerType {
        FileServerType::Http
    }
    fn server_name(&self) -> &str {
        "TODO: demo-file-server-name"
    }
    async fn add_chunk(
        &self,
        file_id: FileId,
        chunk_seq: u64,
        chunk_hash: &str,
        chunk_size: u32,
    ) -> Result<(ChunkServerType, String, ChunkId), Box<dyn std::error::Error + Send + Sync>> {
        let (chunk_server_type, chunk_server_name, remote_chunk_info) = {
            let mut storage = self.storage.lock().await;
            let (task_server_type, task_sever_name, file_hash, file_size, _chunk_size) = storage.get_file_by_id(file_id)?.unwrap();
            let chunk_mgr = self.chunk_mgr_selector.select(file_hash.as_str(), chunk_seq, chunk_hash).await?;
            storage.insert_file_chunk(file_hash.as_str(), chunk_seq, chunk_hash, chunk_size, chunk_mgr.server_type(), chunk_mgr.server_name())?
        };

        match remote_chunk_info {
            Some(remote_chunk_id) => {
                Ok((chunk_server_type, chunk_server_name, remote_chunk_id))
            },
            None => {
                let chunk_mgr = self.chunk_mgr_selector.select_by_name(chunk_server_type, chunk_server_name.as_str()).await?;
                let remote_chunk_id = chunk_mgr.add_chunk(self.server_type(), self.server_name(), chunk_hash, chunk_size).await?;
                
                let mut storage = self.storage.lock().await;
                storage.update_chunk(chunk_hash, remote_chunk_id)?;
                Ok((chunk_server_type, chunk_server_name, remote_chunk_id))
            }
        }
    }

    async fn set_chunk_uploaded(
        &self,
        file_id: FileId,
        chunk_seq: u64,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut storage = self.storage.lock().await;
        storage.set_chunk_uploaded(file_id, chunk_seq)
    }

    async fn get_chunk_info(&self, file_id: FileId, chunk_seq: u64) -> Result<Option<ChunkInfo>, Box<dyn std::error::Error + Send + Sync>> {
        let mut storage = self.storage.lock().await;
        storage.get_chunk_info(file_id, chunk_seq)
    }
}
