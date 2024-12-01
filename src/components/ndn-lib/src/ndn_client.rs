use tokio::io::{AsyncRead,AsyncWrite};
use url::Url;
use log::*;
use reqwest::Client;
use std::ops::Range;
use tokio_util::io::StreamReader;
use futures_util::StreamExt;
use std::pin::Pin;
use tokio::io::{BufReader,BufWriter};
use std::collections::HashMap;

pub enum ChunkWorkState {
    Idle,
    Downloading(u64,u64),//complete size / total size
    DownloadError(String),//error message
}

use crate::{chunk, chunk_mgr, MAX_CHUNK_SIZE};
use crate::{ChunkId,ChunkResult,ChunkMgr,ChunkError,ChunkReadSeek,ChunkHasher};
pub struct NdnClient {
    default_chunk_mgr_id:Option<String>,
    session_token:Option<String>,
    default_remote_url:Option<String>,
    enable_mutil_remote:bool,
    enable_remote_pull:bool,
    enable_zone_pull:bool,
    chunk_work_state:HashMap<ChunkId,ChunkWorkState>,//
}


//暂时只实现get接口
//NdnClient自己是无持久化状态的，任何状态的保存都依赖于ChunkMgr
impl NdnClient {
    pub fn new(default_remote_url:String,session_token:Option<String>,chunk_mgr_id:Option<String>)->Self {
        Self {
            default_chunk_mgr_id:chunk_mgr_id,
            session_token,
            default_remote_url:Some(default_remote_url),
            enable_mutil_remote:false,
            enable_remote_pull:false,
            enable_zone_pull:false,
            chunk_work_state:HashMap::new(),
        }
    }

    pub fn gen_chunk_url(chunk_id:&ChunkId,base_url:Option<String>)->String {
        if base_url.is_some() {
            let base_url = base_url.unwrap();
            
        }    
        unimplemented!()
    }

    async fn get_chunk_from_url(&self,chunk_url: String,range:Option<Range<u64>>) 
        -> ChunkResult<(Box<dyn AsyncRead + Send + Sync + Unpin>,u64)> {
    
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| ChunkError::Internal(format!("Failed to create client: {}", e)))?;
        let res;
        if range.is_some() {
            let range = range.unwrap();
            res = client.get(&chunk_url)
                .header("Range", format!("bytes={}-{}", range.start, range.end - 1))
                .send()
                .await
                .map_err(|e| ChunkError::GetFromRemoteError(format!("Request failed: {}", e)))?;
        } else {
            res = client.get(&chunk_url)
            .send()
            .await
            .map_err(|e| ChunkError::GetFromRemoteError(format!("Request failed: {}", e)))?;
        }

        if !res.status().is_success() {
            return Err(ChunkError::GetFromRemoteError(
                format!("HTTP error: {} for {}", res.status(), chunk_url)
            ));
        }

        let content_length = res.content_length();
        if content_length.is_none() {
            return Err(ChunkError::GetFromRemoteError(format!("content length not found for {}", chunk_url)));
        }
        let content_length = content_length.unwrap();

        let stream = res.bytes_stream().map(|r| {
            r.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("Stream error: {}", e)))
        });
        let reader = StreamReader::new(stream);

        Ok((Box::new(reader),content_length))
    }

    
    //auto_add 为false时，本次获取不会进行任何的磁盘操作
    pub async fn get_chunk(&self, chunk_id:ChunkId,auto_add:bool)->ChunkResult<Pin<Box<dyn ChunkReadSeek + Send + Sync + Unpin>>> {
        if !auto_add {
            warn!("get_chunk: auto_add is false, will not download to local");
            return Err(ChunkError::Internal("auto_add is false".to_string()));
        }
 
        if self.default_chunk_mgr_id.is_some() {
            let chunk_mgr_id = self.default_chunk_mgr_id.clone().unwrap();
            let chunk_mgr = ChunkMgr::get_chunk_mgr_by_id(Some(chunk_mgr_id.as_str())).await;
            if chunk_mgr.is_none() {
                return Err(ChunkError::Internal("no chunk mgr".to_string()));
            }
            let chunk_mgr = chunk_mgr.unwrap();
            let real_chunk_mgr = chunk_mgr.lock().await;
            let reader = real_chunk_mgr.get_chunk_reader(&chunk_id,auto_add).await;
            if reader.is_ok() {
                let (reader,len) = reader.unwrap();
                return Ok(reader);
            }
        }

        //本机机没有，开始尝试从remote中读取 （串行尝试，这里没必要并发，但有断点续传）
        //   从Zone的Chunk Mgr中读取（别的设备上的Chunk Mgr）
        //   从默认的Remote Zone中读取
        
        //根据本地缓存进行断点续传
        let mut offset:u64 = 0;
        let mut chunk_size:u64 = 0;
        let mut writer: Option<_> = None;
        let mut download_buffer:Vec<u8> = vec![];
        let is_downlaod_to_mgr;

        if auto_add && self.default_chunk_mgr_id.is_some() {
            let chunk_mgr_id = self.default_chunk_mgr_id.clone().unwrap();
            let chunk_mgr = ChunkMgr::get_chunk_mgr_by_id(Some(chunk_mgr_id.as_str())).await;
            if chunk_mgr.is_none() {
                return Err(ChunkError::Internal("no chunk mgr".to_string()));
            }
            let chunk_mgr = chunk_mgr.unwrap();
            let real_chunk_mgr = chunk_mgr.lock().await;
            let complete_size = 0;//TODO restore from hasher state
            let chunk_writer = real_chunk_mgr.open_chunk_writer(&chunk_id,0,true).await
                .map_err(|e| {
                    warn!("get_chunk: open chunk writer failed:{}",e.to_string());
                    e
                })?;
            offset = complete_size;
            writer = Some(chunk_writer);
            is_downlaod_to_mgr = true;
        }  else {
            is_downlaod_to_mgr = false;
        }

        let range:Option<Range<u64>> = if offset > 0 {
            Some(offset..MAX_CHUNK_SIZE)
        } else {
            None
        };
        let mut remote_reader = None;
        if self.enable_zone_pull {
            //TODO:从Zone的Chunk Mgr中读取（别的设备上的Chunk Mgr）
            unimplemented!()
        }

        if self.enable_remote_pull {
            //TODO:从指定的Remote Zone中读取
            if self.default_remote_url.is_some() {
                let remote_url = self.default_remote_url.as_ref().unwrap();
                let chunk_url = Self::gen_chunk_url(&chunk_id,Some(remote_url.clone()));
                let reader_result = self.get_chunk_from_url(chunk_url,range).await;
                if reader_result.is_ok() {
                    let (reader,len) = reader_result.unwrap();
                    remote_reader = Some(reader);
                    chunk_size = len;
                }
            }
        }

        if remote_reader.is_none() {
            warn!("get_chunk: no remote reader for chunk:{}",chunk_id.to_string());
            return Err(ChunkError::ChunkNotFound(chunk_id.to_string()));
        }
        let mut remote_reader = remote_reader.unwrap();
        let mut writer = writer.unwrap();
        
        
        info!("start download chunk {} from remote",chunk_id.to_string());
        //边下载边计算hash，注意断点续传也需要保存hash的计算进度
        //let chunk_hasher = ChunkHasher::new(Some(chunk_id.hash_type.as_str()));
        tokio::io::copy(&mut remote_reader,&mut writer).await
            .map_err(|e| {
                warn!("download chunk {} from remote failed:{}",chunk_id.to_string(),e.to_string());
                ChunkError::IoError(e.to_string())
            })?;
        info!("download chunk {} from remote success and verifyed",chunk_id.to_string());
        
        if is_downlaod_to_mgr {
            let chunk_mgr_id = self.default_chunk_mgr_id.clone().unwrap();
            let chunk_mgr = ChunkMgr::get_chunk_mgr_by_id(Some(chunk_mgr_id.as_str())).await.unwrap();
            let real_chunk_mgr = chunk_mgr.lock().await;
            real_chunk_mgr.close_chunk_writer(&chunk_id).await?;
            let (reader,len) = real_chunk_mgr.get_chunk_reader(&chunk_id,false).await
                .map_err(|e| {
                    warn!("get_chunk: get chunk reader failed:{}",e.to_string());
                    e
                })?;
            return Ok(reader);
        } 

        Err(ChunkError::Internal("no chunk mgr".to_string()))
    }

    pub async fn get_chunk_state(&self,chunk_id:ChunkId)->ChunkResult<ChunkWorkState> {
        unimplemented!()
    }

    //pull的语义是将chunk下载并添加到指定chunk_mgr中，返回的是本次pull传输的字节数
    pub async fn pull_chunk(&self, chunk_id:ChunkId,mgr_id:Option<&str>)->ChunkResult<u64> {
        let chunk_mgr = ChunkMgr::get_chunk_mgr_by_id(mgr_id).await;
        if chunk_mgr.is_none() {
            return Err(ChunkError::Internal("no chunk mgr".to_string()));
        }
        let chunk_mgr = chunk_mgr.unwrap();
        let mut chunk_mgr = chunk_mgr.lock().await;
        let is_exist = chunk_mgr.is_chunk_exist(&chunk_id).await?;
        if is_exist {
            return Err(ChunkError::ChunkExists(chunk_id.to_string()));
        }

        let chunk_url = Self::gen_chunk_url(&chunk_id,None);
        let (mut reader,len) = self.get_chunk_from_url(chunk_url,None).await?;
        let mut writer = chunk_mgr.open_chunk_writer(&chunk_id,len,true).await?;
        tokio::io::copy(&mut reader,&mut writer).await
            .map_err(|e| {
                warn!("pull_chunk: copy chunk {} from remote failed:{}",chunk_id.to_string(),e.to_string());
                ChunkError::IoError(e.to_string())
            })?;
        chunk_mgr.close_chunk_writer(&chunk_id).await?;
        Ok(len)
    }

}



