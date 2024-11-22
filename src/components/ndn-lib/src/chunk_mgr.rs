
//chunk_mgr默认是机器级别的，即多个进程可以共享同一个chunk_mgr
//chunk_mgr使用共享内存/filemap等技术来实现跨进程的读数据共享，比使用127.0.0.1的http协议更高效
//从实现简单的角度考虑，先用http协议实现写数据？
use tokio::{
    fs::{self, File,OpenOptions}, 
    io::{self, AsyncRead,AsyncWrite, AsyncReadExt, AsyncWriteExt, AsyncSeek, AsyncSeekExt}, 
};
use log::*;
use crate::{ChunkStore,ChunkId,ChunkResult,ChunkReadSeek,ChunkError};
use memmap::Mmap;
use std::pin::Pin;



pub struct ChunkMgr {
    local_store_list:Vec<ChunkStore>,
    local_cache:Option<ChunkStore>,
    mgr_id:String,
    mmap_cache_dir:Option<String>,

}

impl ChunkMgr {
    pub async fn get_chunk_mgr_by_id(chunk_mgr_id:&str)->Option<Self> {
        None
    }

    pub fn new()->Self {
        Self {
            local_store_list:vec![],
            local_cache:None,
            mgr_id:"default".to_string(),
            mmap_cache_dir:None,
        }
    }

    fn get_cache_mmap_path(&self, chunk_id:&ChunkId)->Option<String> {
        None
    }

    //得到已经存在chunk的reader
    pub async fn get_chunk_reader(&self, chunk_id:&ChunkId,auto_cache:bool)->ChunkResult<(Pin<Box<dyn ChunkReadSeek + Send + Sync + Unpin>>,u64)> {
        //at first ,do access control

        let mcache_file_path = self.get_cache_mmap_path(chunk_id);
        if mcache_file_path.is_some() {
            let mcache_file_path = mcache_file_path.unwrap();
            let file = File::open(mcache_file_path.clone()).await;
            if file.is_ok() {
                let file = file.unwrap();
                let file_meta = file.metadata().await.unwrap();
                info!("get_chunk_reader:return tmpfs cache file:{}", mcache_file_path);
                return Ok((Box::pin(file),file_meta.len()));
            }
        }

        if self.local_cache.is_some() {
            let local_cache = self.local_cache.as_ref().unwrap();
            let local_reader = local_cache.get_chunk_reader(chunk_id).await;
            if local_reader.is_ok() {
                info!("get_chunk_reader:return local cache file:{}", chunk_id.to_string());
                return local_reader;
            }
        }

        warn!("get_chunk_reader: no cache file:{}", chunk_id.to_string());

        for local_store in self.local_store_list.iter() {
            let local_reader = local_store.get_chunk_reader(chunk_id).await;
            if local_reader.is_ok() {
                //TODO:将结果数据添加到自动cache管理中
                //caceh是完整的，还是可以支持部分？
                return local_reader;
            }
        }

        Err(ChunkError::ChunkNotFound(chunk_id.to_string()))
    }

    pub async fn open_chunk_writer(&self, chunk_id:&ChunkId,chunk_size:u64,append:bool)->ChunkResult<(Pin<Box<dyn AsyncWrite + Send + Sync + Unpin>>,u64)> {
        Err(ChunkError::Internal("no chunk mgr".to_string()))
    }

}


