use thiserror::Error;

#[derive(Error, Debug)]
pub enum DCFSClientError {
    #[error("Failed due to reason: {0}")]
    ReasonError(String),
    #[error("File not found: {0}")]
    FileNotFoundError(String),
    // 其他错误类型
}

type Result<T> = std::result::Result<T, DCFSClientError>;
pub struct DCFSClient {
    chunk_client : ChunkClient,
    meta_client: MetaClient,
    bucket_id : String,
}

// a smb:// server can base on this client to implement
// there is MUST be only ONE DCFSClient in a machine
impl DCFSClient {
    pub fn new() -> DCFSClient {
        DCFSClient {
            chunk_client : ChunkClient::new(),
            meta_client : MetaClient::new(),
            
        }
    }

    pub async fn mkdir(&self, path : String) -> Result<()> {
        // 1) use meta_client to create meta info
    }

    //notice: directory have lots of files cloud be slow
    pub async fn list(&self, path : String) -> Result<Vec<String>> {
        // 1) use meta_client to get meta info
    }

    pub async fn remove_dir(&self, path : String) -> Result<()> {
        // 1) use meta_client to delete meta info
    }

    pub async fn remove_file(&self, path : String) -> Result<()> {
        // 1) use meta_client to delete meta info
    }

    pub async fn move_file(&self, src_path : String, dst_path : String) -> Result<()> {
        // 1) use meta_client to move meta info
    }

    pub async fn copy(&self, src_path : String, dst_path : String) -> Result<()> {
        // 1) use meta_client to copy meta info
    }


    pub async fn link(&self, src_path : String, dst_path : String) -> Result<()> {
        // 1) use meta_client to link meta info
    }
    

    pub async fn open(&self, path : String,open_flags:u32) -> Result<()> {
        // 1) find opened file in the opened file list, if found return
        // 2) select a meta_server (some times , it is close to the device and file)
        // 2) use meta_client to create meta 
  
    }
    
    pub fn seek(&self, file_id : u128, offset:u64) -> Result<()> {
        // 1) find opened file in the opened file list, if not found return error
        // 2) change position of the file
    }

    //写入必然是独占的，默认写入会在主写入完成后返回 基本成功，然后等待local_cache写入足够的chunk 
    //       应用可以配置成高可靠cache写和高可靠写
    //       高可靠cache写：会连接至少一个cache server，写入成功后返回
    //       高可靠写：写完cache后立刻计算hash并写入chunk server，写入成功后返回
    pub async fn write(&self, file_id : u128, data:Vec<u8>) -> Result<()> {
        // 1) find opened file in the opened file list by file_id, if not found return error
        // 2) write data to local cache
        // 3) update meta info at local if needed
        // 4) notify local cache start working 
    }

    pub async fn read(&self, file_id : u128, offset:u64, size:u64) -> Result<Vec<u8>> {
        // 1) find opened file in the opened file list, if not found return error
        // 2) read data from local cache 
        // 3) if not found in local cache, try read data from other Cache Server
        // 4) if not found in other Cache Server, try read data from chunk_server
    }

    pub async fn flush(&self, file_id : u128) -> Result<()> {
        // 1) find opened file in the opened file list, if not found return error
        // 2) flush data to local cache
        // 3) flush data to all Cache Server (at least 1)
        // 4) if needed, flush data to chunk_server
    }

    //pub async fn stat(&self, path:String) -> Result<()> {
    //    // 1) use meta_client to get meta info
    //}





    pub async fn close(&self, file_id : u128) -> Result<()> {
        // 1) find opened file in the opened file list, if not found return error
        // 2) use meta_client to close the file
    }
}

