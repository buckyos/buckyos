use tokio::io::{AsyncRead,AsyncWrite,AsyncWriteExt,AsyncReadExt};
use url::Url;
use log::*;
use reqwest::Client;
use std::io::SeekFrom;
use std::ops::Range;
use std::path::PathBuf;
use tokio_util::io::StreamReader;
use futures_util::StreamExt;
use std::pin::Pin;
use tokio::io::{BufReader,BufWriter};
use std::collections::HashMap;
use futures::Future;

use crate::{copy_chunk, cyfs_get_obj_id_from_url, get_cyfs_resp_headers, CYFSHttpRespHeaders, ChunkState};


pub enum ChunkWorkState {
    Idle,
    Downloading(u64,u64),//complete size / total size
    DownloadError(String),//error message
}

pub struct NdnGetChunkResult {
    pub chunk_id : ChunkId,
    pub chunk_size : u64,
    pub reader : ChunkReader,
}

use crate::{chunk, named_data_mgr, ChunkReader, ObjId, MAX_CHUNK_SIZE};
use crate::{ChunkId,NdnResult,NamedDataMgr,NdnError,ChunkReadSeek,ChunkHasher};
pub struct NdnClient {
    default_ndn_mgr_id:Option<String>,
    session_token:Option<String>,
    default_remote_url:Option<String>,
    enable_mutil_remote:bool,
    enable_remote_pull:bool,
    enable_zone_pull:bool,
    chunk_work_state:HashMap<ChunkId,ChunkWorkState>,//
    pub obj_id_in_host:bool,
}


pub enum ChunkWriterOpenMode {
    AlwaysNew,
    AutoResume,
    SpecifiedOffset(u64,SeekFrom),
} 

//ndn client的核心类似传统http的reqwest库，但增加了chunk的语义
impl NdnClient {
    pub fn new(default_remote_url:String,session_token:Option<String>,named_mgr_id:Option<String>)->Self {
        Self {
            default_ndn_mgr_id:named_mgr_id,
            session_token,
            default_remote_url:Some(default_remote_url),
            enable_mutil_remote:false,
            enable_remote_pull:false,
            enable_zone_pull:false,
            chunk_work_state:HashMap::new(),
            obj_id_in_host:false,
        }
    }

    pub fn gen_chunk_url(&self,chunk_id:&ChunkId,base_url:Option<String>)->String {
        let real_base_url;
        if base_url.is_some() {
            real_base_url = base_url.unwrap();
        } else {
            real_base_url = self.default_remote_url.as_ref().unwrap().clone();
        }

        if self.obj_id_in_host {
            format!("{}.{}",chunk_id.to_base32(),real_base_url)
        } else {
            format!("{}/{}",real_base_url,chunk_id.to_base32())
        }
    }

    //返回成功下载的chunk_id和chunk_size
    pub async fn download_chunk_to_local(&self,chunk_url:&str,local_path:&PathBuf,no_verify:Option<bool>) -> NdnResult<(ChunkId,u64)> {
        unimplemented!()
    }

    pub async fn get_obj_by_url(&self,url:&str,no_verify:Option<bool>) -> NdnResult<(ObjId,String)> {
        unimplemented!()
    }


    //helper function 1
    pub async fn open_chunk_reader_by_url(&self,chunk_url:&str,expect_chunk_id:Option<ChunkId>,range:Option<Range<u64>>)
        ->NdnResult<(ChunkReader,CYFSHttpRespHeaders)> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| NdnError::Internal(format!("Failed to create client: {}", e)))?;
        let res;
        if range.is_some() {
            let range = range.unwrap();
            res = client.get(chunk_url)
                .header("Range", format!("bytes={}-{}", range.start, range.end - 1))
                .send()
                .await
                .map_err(|e| NdnError::GetFromRemoteError(format!("Request failed: {}", e)))?;
        } else {
            res = client.get(chunk_url)
            .send()
            .await
            .map_err(|e| NdnError::GetFromRemoteError(format!("Request failed: {}", e)))?;
        }

        if !res.status().is_success() {
            return Err(NdnError::GetFromRemoteError(
                format!("HTTP error: {} for {}", res.status(), chunk_url)
            ));
        }
        let must_have_obj_id = expect_chunk_id.is_none();

        let mut chunk_id;
        let cyfs_resp_headers = get_cyfs_resp_headers(&res.headers())?;
        if cyfs_resp_headers.obj_id.is_some() {
            debug!("remote return with cyfs-extension headers!:{:?}",cyfs_resp_headers);
            let obj_id = cyfs_resp_headers.obj_id.clone().unwrap();
            if obj_id.is_chunk() {
                chunk_id = ChunkId::from_obj_id(&obj_id);
            } else {
                warn!("remote return with cyfs-extension headers, but obj_id is not a chunk:{}",obj_id.to_string());
                return Err(NdnError::InvalidId(format!("remote return with cyfs-extension headers, but obj_id is not a chunk:{}",
                    obj_id.to_string())));
            }
        } else {
            let get_obj_result = cyfs_get_obj_id_from_url(chunk_url);
            if get_obj_result.is_ok() {
                let (obj_id,obj_inner_path) = get_obj_result.unwrap();
                if obj_id.is_chunk() {
                    chunk_id = ChunkId::from_obj_id(&obj_id);
                } else {
                    warn!("remote return with cyfs-extension headers, but obj_id is not a chunk:{}",obj_id.to_string());
                    return Err(NdnError::InvalidId(format!("remote return with cyfs-extension headers, but obj_id is not a chunk:{}",
                        obj_id.to_string())));
                }
            } else {
                if must_have_obj_id {
                    warn!("no chunkid found in url:{}",chunk_url);
                    return Err(NdnError::InvalidId("no chunkid found in url".to_string()));
                } else {
                    chunk_id = expect_chunk_id.clone().unwrap();
                }
            }
        }

        if expect_chunk_id.is_some() {
            let expect_chunk_id = expect_chunk_id.unwrap();
            if expect_chunk_id != chunk_id {
                warn!("get_chunk_from_url: chunk-id not match for {}, expect:{} actual:{}", chunk_url, expect_chunk_id.to_string(), chunk_id.to_string());
                return Err(NdnError::GetFromRemoteError(format!("chunk-id not match for {}", chunk_url)));
            }
        }

        let content_length = res.content_length();
        if content_length.is_none() {
            return Err(NdnError::GetFromRemoteError(format!("content length not found for {}", chunk_url)));
        }
        let content_length = content_length.unwrap();

        let stream = res.bytes_stream().map(|r| {
            r.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("Stream error: {}", e)))
        });
        let reader = StreamReader::new(stream);
        let reader = Box::pin(reader);

        Ok((reader,cyfs_resp_headers))

    }


    //async fn open_chunk_writer_by_url(&self,chunk_url:String,open_mode:ChunkWriterOpenMode)->NdnResult<(ChunkWriter,Option<ChunkHasher>)> {
    //    unimplemented!()
    //}

    //auto_add 为false时，本次获取不会进行任何的磁盘操作
    // pub async fn get_chunk(&self, chunk_id:ChunkId,auto_add:bool)->NdnResult<Pin<Box<dyn ChunkReadSeek + Send + Sync + Unpin>>> {
    //     if !auto_add {
    //         warn!("get_chunk: auto_add is false, will not download to local");
    //         return Err(NdnError::Internal("auto_add is false".to_string()));
    //     }
 
    //     if self.default_chunk_mgr_id.is_some() {
    //         let chunk_mgr_id = self.default_chunk_mgr_id.clone().unwrap();
    //         let chunk_mgr = NamedDataMgr::get_named_data_mgr_by_id(Some(chunk_mgr_id.as_str())).await;
    //         if chunk_mgr.is_none() {
    //             return Err(NdnError::Internal("no chunk mgr".to_string()));
    //         }
    //         let chunk_mgr = chunk_mgr.unwrap();
    //         let real_chunk_mgr = chunk_mgr.lock().await;
    //         let reader = real_chunk_mgr.get_chunk_reader(&chunk_id,auto_add).await;
    //         if reader.is_ok() {
    //             let (reader,len) = reader.unwrap();
    //             return Ok(reader);
    //         }
    //     }

    //     //本机机没有，开始尝试从remote中读取 （串行尝试，这里没必要并发，但有断点续传）
    //     //   从Zone的Chunk Mgr中读取（别的设备上的Chunk Mgr）
    //     //   从默认的Remote Zone中读取
        
    //     //根据本地缓存进行断点续传
    //     let mut offset:u64 = 0;
    //     let mut chunk_size:u64 = 0;
    //     let mut writer: Option<_> = None;
    //     let mut download_buffer:Vec<u8> = vec![];
    //     let is_downlaod_to_mgr;

    //     if auto_add && self.default_chunk_mgr_id.is_some() {
    //         let chunk_mgr_id = self.default_chunk_mgr_id.clone().unwrap();
    //         let chunk_mgr = NamedDataMgr::get_named_data_mgr_by_id(Some(chunk_mgr_id.as_str())).await;
    //         if chunk_mgr.is_none() {
    //             return Err(NdnError::Internal("no chunk mgr".to_string()));
    //         }
    //         let chunk_mgr = chunk_mgr.unwrap();
    //         let real_chunk_mgr = chunk_mgr.lock().await;
    //         let complete_size = 0;//TODO restore from hasher state
    //         let chunk_writer = real_chunk_mgr.open_chunk_writer(&chunk_id,0,true).await
    //             .map_err(|e| {
    //                 warn!("get_chunk: open chunk writer failed:{}",e.to_string());
    //                 e
    //             })?;
    //         offset = complete_size;
    //         writer = Some(chunk_writer);
    //         is_downlaod_to_mgr = true;
    //     }  else {
    //         is_downlaod_to_mgr = false;
    //     }

    //     let range:Option<Range<u64>> = if offset > 0 {
    //         Some(offset..MAX_CHUNK_SIZE)
    //     } else {
    //         None
    //     };
    //     let mut remote_reader = None;
    //     if self.enable_zone_pull {
    //         //TODO:从Zone的Chunk Mgr中读取（别的设备上的Chunk Mgr）
    //         unimplemented!()
    //     }

    //     if self.enable_remote_pull {
    //         //TODO:从指定的Remote Zone中读取
    //         if self.default_remote_url.is_some() {
    //             let remote_url = self.default_remote_url.as_ref().unwrap();
    //             let chunk_url = Self::gen_chunk_url(&chunk_id,Some(remote_url.clone()));
    //             let reader_result = self.get_chunk_from_url(chunk_url,range).await;
    //             if reader_result.is_ok() {
    //                 let (reader,len) = reader_result.unwrap();
    //                 remote_reader = Some(reader);
    //                 chunk_size = len;
    //             }
    //         }
    //     }

    //     if remote_reader.is_none() {
    //         warn!("get_chunk: no remote reader for chunk:{}",chunk_id.to_string());
    //         return Err(NdnError::NotFound(chunk_id.to_string()));
    //     }
    //     let mut remote_reader = remote_reader.unwrap();
    //     let mut writer = writer.unwrap();
        
        
    //     info!("start download chunk {} from remote",chunk_id.to_string());
    //     //边下载边计算hash，注意断点续传也需要保存hash的计算进度
    //     //let chunk_hasher = ChunkHasher::new(Some(chunk_id.hash_type.as_str()));
    //     tokio::io::copy(&mut remote_reader,&mut writer).await
    //         .map_err(|e| {
    //             warn!("download chunk {} from remote failed:{}",chunk_id.to_string(),e.to_string());
    //             NdnError::IoError(e.to_string())
    //         })?;
    //     info!("download chunk {} from remote success and verifyed",chunk_id.to_string());
        
    //     if is_downlaod_to_mgr {
    //         let chunk_mgr_id = self.default_chunk_mgr_id.clone().unwrap();
    //         let chunk_mgr = NamedDataMgr::get_named_data_mgr_by_id(Some(chunk_mgr_id.as_str())).await.unwrap();
    //         let real_chunk_mgr = chunk_mgr.lock().await;
    //         real_chunk_mgr.close_chunk_writer(&chunk_id).await?;
    //         let (reader,len) = real_chunk_mgr.get_chunk_reader(&chunk_id,false).await
    //             .map_err(|e| {
    //                 warn!("get_chunk: get chunk reader failed:{}",e.to_string());
    //                 e
    //             })?;
    //         return Ok(reader);
    //     } 

    //     Err(NdnError::Internal("no chunk mgr".to_string()))
    // }

    pub async fn pull_chunk_by_url(&self, chunk_url:String,chunk_id:ChunkId,mgr_id:Option<&str>)->NdnResult<u64> {
        let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(mgr_id).await;
        if named_mgr.is_none() {
            return Err(NdnError::Internal("no named data mgr".to_string()));
        }
        let named_mgr = named_mgr.unwrap();
        let mut real_named_mgr = named_mgr.lock().await;

        let mut chunk_size:u64 = 0;
        // query chunk state from named_mgr (if chunk is completed, return already exists)
        let (chunk_state,_chunk_size,progress) = real_named_mgr.query_chunk_state(&chunk_id).await?;
        drop(real_named_mgr);

        let mut real_hash_state = None;
        let mut download_pos = 0;
        let mut reader = None;
        match chunk_state {
            ChunkState::Completed => {
                warn!("pull_chunk: chunk {} already exists at named_mgr:{}",chunk_id.to_string(),mgr_id.unwrap());
                return Ok(0);
            },
            ChunkState::NotExist => {
                //no progess info
                let open_result = self.open_chunk_reader_by_url(chunk_url.as_str(),Some(chunk_id.clone()),None).await;
                if open_result.is_err() {
                    warn!("pull_chunk: open chunk reader failed:{}",open_result.err().unwrap().to_string());
                    return Err(NdnError::NotFound(chunk_id.to_string()));
                }
                let (mut _reader,resp_headers) = open_result.unwrap();
                chunk_size = resp_headers.chunk_size.unwrap();
                reader = Some(_reader);
            },
            _ => {
                chunk_size = _chunk_size;
                // use progress info to open reader send request with range to remote
                if progress.len() > 2 {
                    let json_value = serde_json::from_str::<serde_json::Value>(&progress);
                    if json_value.is_err() {
                        warn!("pull_chunk: invalid progress info:{}",progress);
                    } else {
                        let json_value = json_value.unwrap();
                        let hash_state = ChunkHasher::restore_from_state(json_value);
                        if hash_state.is_err() {
                            warn!("pull_chunk: invalid progress info:{}",progress);
                        } else {
                            
                            let hash_state = hash_state.unwrap();
                            download_pos = hash_state.pos;
                            real_hash_state  = Some(hash_state);
                            info!("pull_chunk load progress sucess!,pos:{}",download_pos);
                        }
                    }
                }
                let mut download_range = None;
                if real_hash_state.is_some() {
                    download_range = Some(download_pos.._chunk_size);
                }
                let open_result = self.open_chunk_reader_by_url(chunk_url.as_str(),Some(chunk_id.clone()),download_range).await;
                if open_result.is_err() {
                    warn!("pull_chunk: open chunk reader failed:{},url:{}",open_result.err().unwrap().to_string(),chunk_url);
                    return Err(NdnError::NotFound(chunk_id.to_string()));
                }
                let (mut _reader,resp_headers) = open_result.unwrap();
                reader = Some(_reader);
                info!("pull_chunk: open chunk reader success,chunk_id:{},chunk_size:{},download_pos:{}",
                    chunk_id.to_string(),chunk_size,download_pos);
            },
        }
        // open chunk writer with progress info
        let real_named_mgr = named_mgr.lock().await;
        let (mut chunk_writer,progress_info) = real_named_mgr.open_chunk_writer(&chunk_id,chunk_size,download_pos).await?;
        drop(real_named_mgr);
        let named_mgr2 = named_mgr.clone();
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1));
        let progress_callback = {
            Some(move |chunk_id: ChunkId, pos: u64, hasher: &Option<ChunkHasher>| {
                let this_chunk_id = chunk_id.clone();
                let mut json_progress_str = String::new();
                if let Some(hasher) = hasher {
                    let state = hasher.save_state();
                    json_progress_str = serde_json::to_string(&state).unwrap(); 
                }
                let counter = counter.clone();
                let named_mgr2 = named_mgr2.clone();
                
                Box::pin(async move {
                    let count = counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if count % 16 == 0 {
                        if !json_progress_str.is_empty() {
                            let mut real_named_mgr = named_mgr2.lock().await;
                            real_named_mgr.update_chunk_progress(&this_chunk_id,json_progress_str).await?;
                        }
                    }
                    Ok(())
                }) as Pin<Box<dyn Future<Output = NdnResult<()>> + Send>>
            })
        };

        let reader = reader.unwrap();
        let copy_result = copy_chunk(chunk_id.clone(), reader, chunk_writer, real_hash_state, progress_callback).await?;
        named_mgr.lock().await.complete_chunk_writer(&chunk_id).await?;
        return Ok(copy_result);
    }

    pub async fn pull_chunk(&self, chunk_id:ChunkId,mgr_id:Option<&str>)->NdnResult<u64> {
        let chunk_url = self.gen_chunk_url(&chunk_id,None);
        info!("will pull chunk {} by url:{}",chunk_id.to_string(),chunk_url);
        self.pull_chunk_by_url(chunk_url,chunk_id,mgr_id).await
    }
}


#[cfg(test)] 
mod tests {
    use super::*;
    use buckyos_kit::*;
    use crate::*;
    use serde_json::json;
    use cyfs_gateway_lib::*;
    use cyfs_warp::*;
    use rand::{thread_rng, RngCore};

    fn generate_random_bytes(size: u64) -> Vec<u8> {
        let mut rng = thread_rng();
        let mut buffer = vec![0u8; size as usize];
        rng.fill_bytes(&mut buffer);
        buffer
    }

    #[tokio::test]
    async fn test_pull_chunk() {
        init_logging("ndn_client_test");
        let test_server_config = json!({
            "tls_port":3243,
            "http_port":3280,
            "hosts": {
              "*": {
                "enable_cors":true,
                "routes": {
                  "/ndn/": {
                    "named_mgr": {
                        "named_data_mgr_id":"test",
                        "read_only":true,
                        "guest_access":true,
                        "is_chunk_id_in_path":true,
                        "enable_mgr_file_path":true
                    }
                  }
                } 
              }
            }
          });  

        let test_server_config:WarpServerConfig = serde_json::from_value(test_server_config).unwrap();

        tokio::spawn(async move {
            info!("start test ndn server(powered by cyfs-warp)...");
            start_cyfs_warp_server(test_server_config).await;
        });
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        // Step 1: Initialize a new NamedDataMgr in a temporary directory and create a test object
        let temp_dir = tempfile::tempdir().unwrap();
        let config = NamedDataMgrConfig {
            local_stores: vec![temp_dir.path().to_str().unwrap().to_string()],
            local_cache: None,
            mmap_cache_dir: None,
        };
        
        let named_mgr = NamedDataMgr::from_config(
            Some("test".to_string()),
            temp_dir.path().to_path_buf(),
            config
        ).await.unwrap();
        let chunk_a_size:u64 = 1024*1024 + 321;
        let chunk_a = generate_random_bytes(chunk_a_size);
        let mut hasher = ChunkHasher::new(None).unwrap();
        let hash_a = hasher.calc_from_bytes(&chunk_a);
        let chunk_id_a = ChunkId::from_sha256_result(&hash_a);
        info!("chunk_id_a:{}",chunk_id_a.to_string());
        let (mut chunk_writer,progress_info) = named_mgr.open_chunk_writer(&chunk_id_a, chunk_a_size, 0).await.unwrap();
        chunk_writer.write_all(&chunk_a).await.unwrap();
        drop(chunk_writer);
        named_mgr.complete_chunk_writer(&chunk_id_a).await.unwrap();


        let chunk_b_size:u64 = 1024*1024*3 + 321*71;
        let chunk_b = generate_random_bytes(chunk_b_size);
        let mut hasher = ChunkHasher::new(None).unwrap();
        let hash_b = hasher.calc_from_bytes(&chunk_b);
        let chunk_id_b = ChunkId::from_sha256_result(&hash_b);
        info!("chunk_id_b:{}",chunk_id_b.to_string());
        let (mut chunk_writer,progress_info) = named_mgr.open_chunk_writer(&chunk_id_b, chunk_b_size, 0).await.unwrap();
        chunk_writer.write_all(&chunk_b).await.unwrap();
        drop(chunk_writer);
        named_mgr.complete_chunk_writer(&chunk_id_b).await.unwrap();
        
        
        info!("named_mgr [test] init OK!");
        NamedDataMgr::set_mgr_by_id(Some("test"),named_mgr).await.unwrap();

        let temp_dir = tempfile::tempdir().unwrap();
        let config = NamedDataMgrConfig {
            local_stores: vec![temp_dir.path().to_str().unwrap().to_string()],
            local_cache: None,
            mmap_cache_dir: None,
        };    
        let named_mgr2 = NamedDataMgr::from_config(
            Some("test_client".to_string()),
            temp_dir.path().to_path_buf(),
            config
        ).await.unwrap();
        info!("named_mgr [test_client] init OK!");
        NamedDataMgr::set_mgr_by_id(Some("test_client"),named_mgr2).await.unwrap();
        // Step 2: Start a cyfs-warp server based on the named_mgr and configure the ndn-router
        let named_mgr_test = NamedDataMgr::get_named_data_mgr_by_id(Some("test_client")).await.unwrap();
        info!("test get test_client named mgr  OK!");
        drop(named_mgr_test);
    
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        // // Step 3: Configure the ndn-client and set the cyfs-warp address (obj_id in path)
        let client = NdnClient::new("http://localhost:3280/ndn/".to_string(),None,Some("test_client".to_string()));
        client.pull_chunk(chunk_id_a.clone(),Some("test_client")).await.unwrap();

        let named_mgr_client = NamedDataMgr::get_named_data_mgr_by_id(Some("test_client")).await.unwrap();
        let real_named_mgr_client = named_mgr_client.lock().await;
        let (mut reader,len) = real_named_mgr_client.open_chunk_reader(&chunk_id_a,SeekFrom::Start(0),false).await.unwrap();
        assert_eq!(len,chunk_a_size);
        drop(real_named_mgr_client);
        let mut buffer = vec![0u8;chunk_a_size as usize];
        reader.read_exact(&mut buffer).await.unwrap();
        assert_eq!(buffer,chunk_a);


        //client.set_remote_url(format!("http://{}/{}", warp_addr, test_obj_id.to_base32()));

        // // Step 4.1: Use the ndn-client's pull_chunk interface to retrieve the chunk
        // let chunk_id = ChunkId::from_obj_id(&test_obj_id);
        // let pull_result = client.pull_chunk(chunk_id.clone()).await;
        // assert!(pull_result.is_ok(), "Failed to pull chunk");

        // // Step 4.2: Use the ndn-client's get_obj_by_url interface to get the chunk/object
        // let obj_result = client.get_obj_by_url(&format!("http://{}/{}", warp_addr, test_obj_id.to_base32())).await;
        // assert!(obj_result.is_ok(), "Failed to get object by URL");

        // // Step 4.3: Use the ndn-client's get_obj_by_url with a URL containing obj_json_path to get the corresponding value
        // let json_path = "some_json_path";
        // let json_result = client.get_obj_by_url(&format!("http://{}/{}/{}", warp_addr, test_obj_id.to_base32(), json_path)).await;
        // assert!(json_result.is_ok(), "Failed to get JSON value by URL");

        // http://remote_zone_id/ndn/repo/meta_index_db
        // // Step 4.4: Use the ndn-client's get_obj_by_url to get a typical file_obj.content
        // let file_content_result = client.get_obj_by_url(&format!("http://{}/file_obj.content", warp_addr)).await;
        // assert!(file_content_result.is_ok(), "Failed to get file object content");

        // // Clean up
        // warp_server.stop().await.unwrap();
        // temp_dir.close().unwrap();
    }
}



