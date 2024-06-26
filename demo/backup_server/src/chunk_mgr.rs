use backup_lib::{check_chunk_hash, ChunkId, ChunkServerType, FileServerType};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::chunk_mgr_storage::ChunkStorageSqlite;

pub(crate) struct ChunkMgr {
    storage: Arc<Mutex<ChunkStorageSqlite>>,
    save_dir: PathBuf,
    tmp_dir: PathBuf,
}

impl ChunkMgr {
    pub(crate) fn new(storage: ChunkStorageSqlite, save_dir: PathBuf, tmp_dir: PathBuf) -> Self {
        Self {
            storage: Arc::new(Mutex::new(storage)),
            save_dir,
            tmp_dir,
        }
    }
}

#[async_trait::async_trait]
impl backup_lib::ChunkMgrServer for ChunkMgr {
    async fn add_chunk(
        &self,
        file_server_type: FileServerType,
        file_server_name: &str,
        chunk_hash: &str,
        chunk_size: u32,
    ) -> Result<ChunkId, Box<dyn std::error::Error + Send + Sync>> {
        self.storage.lock().await.insert_chunk(
            file_server_type,
            file_server_name,
            chunk_hash,
            chunk_size,
        )
    }
}

#[async_trait::async_trait]
impl backup_lib::ChunkMgr for ChunkMgr {
    fn server_type(&self) -> ChunkServerType {
        ChunkServerType::Http
    }
    fn server_name(&self) -> &str {
        "TODO: demo-chunk-server-name"
    }
    async fn upload(
        &self,
        chunk_hash: &str,
        chunk: &[u8],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if !check_chunk_hash(chunk, chunk_hash) {
            log::error!("chunk hash not match: {}", chunk_hash);
            return Err("chunk hash not match".into());
        }

        let chunk_info = self.storage.lock().await.query_chunk_by_hash(chunk_hash)?;
        match chunk_info {
            Some((_todo_chunk_id, chunk_size, save_path)) => {
                if chunk.len() as u32 != chunk_size {
                    return Err("chunk size not match".into());
                }

                if let Some(_todo_save_path) = save_path {
                    return Ok(());
                }

                let tmp_path = self.tmp_dir.join(chunk_hash);
                tokio::fs::write(&tmp_path, chunk).await?;
                let save_path = self.save_dir.join(chunk_hash);
                self.storage
                    .lock()
                    .await
                    .update_chunk_save_path(chunk_hash, save_path.as_path())?;
                let _todo = tokio::fs::copy(&tmp_path, &save_path).await?;
                let _todo = tokio::fs::remove_file(&tmp_path).await;

                Ok(())
            }
            None => {
                return Err("chunk not found".into());
            }
        }
    }

    async fn download(
        &self,
        chunk_id: ChunkId,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let info = self.storage.lock().await.get_chunk_by_id(chunk_id)?;
        match info {
            Some((chunk_hash, chunk_size, save_path)) => match save_path {
                Some(save_path) => {
                    let chunk = tokio::fs::read(save_path).await?;
                    if chunk.len() as u32 != chunk_size {
                        return Err("chunk size not match".into());
                    }
                    if !check_chunk_hash(chunk.as_slice(), chunk_hash.as_str()) {
                        return Err("chunk hash not match".into());
                    }
                    Ok(chunk)
                }
                None => Err("chunk not saved".into()),
            },
            None => Err("chunk not found".into()),
        }
    }
}
