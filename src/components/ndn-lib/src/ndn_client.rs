use crate::{ChunkId,ChunkResult,ChunkMgr};
use tokio::io::{AsyncRead,AsyncWrite};
use url::Url;
pub struct NdnClient {
    default_remote_url:String,
    enable_mutil_remote:bool,
    enable_remote_pull:bool,
    enable_zone_pull:bool,
}

//暂时只实现get接口
impl NdnClient {
    pub fn new(default_remote_url:String)->Self {
        Self {
            default_remote_url,
            enable_mutil_remote:false,
            enable_remote_pull:false,
            enable_zone_pull:false,
        }
    }

    pub fn gen_chunk_url(chunk_id:ChunkId,base_url:Option<String>)->String {
        unimplemented!()
    }

    pub async fn get_chunk_from_url(&self, chunk_url:String,url_only:bool)->ChunkResult<Box<dyn AsyncRead + Send + Sync + Unpin>> {
        unimplemented!()
    }

    pub async fn get_chunk(&self, chunk_id:ChunkId)->ChunkResult<Box<dyn AsyncRead + Send + Sync + Unpin>> {
        unimplemented!()
    }

    //pull的语义是将chunk下载并添加到本地的chunk_mgr中
    pub async fn pull_chunk(&self, chunk_urls:Vec<Url>)->ChunkResult<u64> {
        unimplemented!()
    }

}



