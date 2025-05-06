
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ChunkClientError {
    #[error("Failed due to reason: {0}")]
    ReasonError(String),
    #[error("File not found: {0}")]
    FileNotFoundError(String),
    // 其他错误类型
}


type ChunkId = u128;
type Result<T> = std::result::Result<T, ChunkClientError>;

pub struct ChunkClient {
    local_cache : Option<String>, // local cache path
    device_id : String,
}

impl ChunkClient {
    pub fn new() -> ChunkClient {
        ChunkClient {}
    }

    //
    pub fn start_sync_disk_map() -> Result<()> {

    }  

    async fn get_chunk(&self, chunk_id : ChunkId) -> Result<Vec<u8>> {
        // 1) lookup the chunk in the local cache (include reading), if found return it
        // 2) use disk_map locate the chunk:remote or local
        // 3)   try load chunk from disk or chunk_server ,if all success, write the chunk to cache
        // 4)   use old disk_map to locate the chunk:remote or local
    }

    async fn put_chunk(&self, chunk_id : ChunkId, data : Vec<u8>) -> Result<()> {
        // 1) if writeing cache is enabled, write the chunk to local cache ,return write to cache ok 
        // 2) use disk_map locate the chunk:remote or local
        // 3) try write chunk to disk or chunk_server ,if all success, remove the chunk from cache
        // 4) if write to disk or chunk_server enough, return write OK
    }
}