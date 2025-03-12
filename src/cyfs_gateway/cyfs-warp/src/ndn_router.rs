use buckyos_kit::get_by_json_path;
use log::*;
use anyhow::Result;
use hyper::{Request,Response,Body,StatusCode};
use std::{io::SeekFrom, sync::Arc};
use std::net::IpAddr;
use ndn_lib::*;
use cyfs_gateway_lib::{NamedDataMgrRouteConfig};
use serde_json::Value;
use crate::parse_range;
use rand::RngCore;

//1. get objid and inner path
//2. if enable, try use relative path to get objid and inner path
//3. if inneer path is not null, use get_json_by_path to get Value
//4. if Value is objid, return 
//5. return : Value | Reader | Text Record


enum GetObjResultBody {
    NamedObj(Value), //value, embeded obj_string
    Reader(ChunkReader,u64),//reader, chunk_size, embeded obj_string
    TextRecord(String),//text_record, verify_obj path  
}

struct GetObjResult {
    pub real_obj_id:ObjId,
    pub real_body:GetObjResultBody,
}



impl GetObjResult {
    pub fn new_chunk_result(real_obj_id:ObjId,real_body:ChunkReader,chunk_size:u64)->Self {
        let body = GetObjResultBody::Reader(real_body,chunk_size);
        Self { real_obj_id, real_body:body }
    }

    pub fn new_named_obj_result(real_obj_id:ObjId,real_body:Value)->Self {
        let body = GetObjResultBody::NamedObj(real_body);
        Self { real_obj_id, real_body:body }
    }

    pub fn new_value_result(real_body:Value)->Self {
        let body_str = serde_json::to_string(&real_body).unwrap();
        let body = GetObjResultBody::TextRecord(body_str);
        let fake_obj_id = ObjId::new_by_raw("fake".to_string(),vec![]);
        Self { real_obj_id:fake_obj_id, real_body:body }
    }
}

async fn get_obj_result(mgr:Arc<tokio::sync::Mutex<NamedDataMgr>>,obj_id:&ObjId,offset:u64)->Result<GetObjResult> {
    let real_mgr = mgr.lock().await;
    if obj_id.is_chunk() {
        let chunk_id = ChunkId::from_obj_id(&obj_id);
        let seek_from = SeekFrom::Start(offset);
        let (chunk_reader,chunk_size) = real_mgr.open_chunk_reader(&chunk_id, seek_from, true).await
            .map_err(|e| {
                warn!("get chunk reader by objid failed: {}", e);
                anyhow::anyhow!("get chunk reader by objid failed: {}", e)
            })?;

        debug!("ndn route -> chunk: {}, chunk_size: {}, offset: {}", obj_id.to_base32(), chunk_size, offset);
        return Ok(GetObjResult::new_chunk_result(obj_id.clone(),chunk_reader,chunk_size));
    } else {
        let obj_body = real_mgr.get_object(&obj_id,None).await?;
        debug!("ndn route -> obj {}", obj_body.to_string());
        return Ok(GetObjResult::new_named_obj_result(obj_id.clone(),obj_body));
    }
}

pub struct InnerPathInfo {
    pub root_obj_id:ObjId,
    pub inner_obj_path:String,
    pub path_obj_jwt:Option<String>,
    pub mtree_path:Option<String>,
}

async fn build_response_by_obj_get_result(obj_get_result:GetObjResult,start:u64,inner_path_info:Option<InnerPathInfo>)->Result<Response<Body>> {
    let body_result;
    let mut result = Response::builder()
                    .header("cyfs-obj-id", obj_get_result.real_obj_id.to_base32());

    if inner_path_info.is_some() {
        let inner_path_info = inner_path_info.unwrap();
        result = result.header("cyfs-root-obj-id", inner_path_info.root_obj_id.to_base32());
        if inner_path_info.path_obj_jwt.is_some() {
            result = result.header("cyfs-path-obj", inner_path_info.path_obj_jwt.unwrap());
        }
        if inner_path_info.mtree_path.is_some() {
            result = result.header("cyfs-mtree-path", inner_path_info.mtree_path.unwrap());
        }
    }

    match obj_get_result.real_body {
        GetObjResultBody::NamedObj(json_value) => {
            result = result.header("Content-Type", "application/json")
            .status(StatusCode::OK);
            body_result = result.body(Body::from(serde_json::to_string(&json_value)?))?;
        }
        GetObjResultBody::Reader(chunk_reader,chunk_size) => {

            let stream = tokio_util::io::ReaderStream::new(chunk_reader);
            result = result.header("Accept-Ranges", "bytes")
                .header("Content-Type", "application/octet-stream")
                .header("Cache-Control", "public,max-age=31536000")
                .header("cyfs-obj-size", chunk_size.to_string());
            if start > 0 {
                result = result.header("Content-Range", format!("bytes {}-{}/{}", start, chunk_size - 1, chunk_size))
                .header("Content-Length", chunk_size - start)
                .status(StatusCode::PARTIAL_CONTENT);
            } else {          
                result = result.header("Content-Length", chunk_size)
                .status(StatusCode::OK);
            }
            body_result = result.body(Body::wrap_stream(stream))?;
        }
        GetObjResultBody::TextRecord(text_record) => {
            result = result.header("Content-Type", "plain/text")
                .status(StatusCode::OK);
            body_result = result.body(Body::from(text_record))?;
        }
    }
    Ok(body_result)
}

pub async fn handle_ndn(mgr_config: &NamedDataMgrRouteConfig, req: Request<Body>, host: &str, _client_ip:IpAddr,route_path: &str) -> Result<Response<Body>> {
    if req.method() != hyper::Method::GET {
        return Err(anyhow::anyhow!("Invalid method: {}", req.method()));
    }

    let named_mgr_id = mgr_config.named_data_mgr_id.clone();
    let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(Some(named_mgr_id.as_str())).await;
   
    if named_mgr.is_none() {
        warn!("Named manager not found: {}", named_mgr_id);
        return Err(anyhow::anyhow!("Named manager not found: {}", named_mgr_id));
    }
    let named_mgr = named_mgr.unwrap();
    let named_mgr2 = named_mgr.clone();
    let named_mgr = named_mgr.lock().await;
            
    let range_str = req.headers().get(hyper::header::RANGE);
    let mut start = 0;
    if range_str.is_some() {
        let range_str = range_str.unwrap().to_str().unwrap();
        (start,_) = parse_range(range_str,u64::MAX)
            .map_err(|e| {
                warn!("parse range failed: {}", e);
                anyhow::anyhow!("parse range failed: {}", e)
            })?;
    }

    //let chunk_id_result;
    let mut obj_id:Option<ObjId> = None;
    let mut obj_content:Option<Value> = None;
    let mut inner_obj_path:Option<String> = None;
    let path = req.uri().path();
    let _user_id = "guest";//TODO: session_token from cookie
    let _app_id = "unknown";

    if mgr_config.is_object_id_in_path {
        //let sub_path = path.trim_start_matches(path);
        let obj_id_result = ObjId::from_path(path);
        if obj_id_result.is_ok() {
            let (the_obj_id,the_obj_path) = obj_id_result.unwrap();
            obj_id = Some(the_obj_id);
            inner_obj_path = the_obj_path;
        }
    } else {
        //get chunkid by hostname
        let obj_id_result = ObjId::from_hostname(host);
        if obj_id_result.is_ok() {
            obj_id = Some(obj_id_result.unwrap());
        }
    }

    let mut path_obj_jwt:Option<String> = None;
    //let mut root_obj_id:Option<ObjId> = None;
    let mut inner_path_obj:Option<InnerPathInfo> = None;
    if obj_id.is_none() {
        if mgr_config.enable_mgr_file_path {
            let sub_path = buckyos_kit::get_relative_path(route_path, path);
            let target_obj_id = named_mgr.get_obj_id_by_path(&sub_path).await;
            path_obj_jwt = named_mgr.get_path_obj(&sub_path).await;
            if target_obj_id.is_ok() {
                // will return (obj_id,obj_json_str)
                obj_id = Some(target_obj_id.unwrap());
            } else {
                //root_obj/inner_path = obj_id,
                let root_obj_id_result = named_mgr.select_obj_id_by_path(&sub_path).await;

                if root_obj_id_result.is_ok() {
                    
                    let (the_root_obj_id,the_obj_path) = root_obj_id_result.unwrap();
                    if the_obj_path.is_none() {
                        return Err(anyhow::anyhow!("ndn_router:cann't found target object,inner_obj_path is not found"));
                    }
                    inner_obj_path = the_obj_path.clone();
                    if the_root_obj_id.is_chunk() {
                        return Err(anyhow::anyhow!("ndn_router:chunk is not supported to be root obj"));
                    }
                    if the_root_obj_id.is_big_container() {
                        //TODO: not support now
                    } else {
                        let root_obj_json = named_mgr.get_object(&the_root_obj_id, None).await?;
                        let obj_filed = get_by_json_path(&root_obj_json, &inner_obj_path.clone().unwrap());
                        if obj_filed.is_none() {
                            return Err(anyhow::anyhow!("ndn_router:cann't found target object,inner_obj_path is not valid"));
                        }
                        //this is the target content or target obj_id
                        let obj_filed = obj_filed.unwrap();
                        if obj_filed.is_string() {
                            let obj_id_str = obj_filed.as_str().unwrap();
                            let p_obj_id = ObjId::new(obj_id_str);
                            if p_obj_id.is_ok() {
                                obj_id = Some(p_obj_id.unwrap());
                            } else {
                                obj_content = Some(obj_filed);
                            }
                        } else {
                            obj_content = Some(obj_filed);
                        }
                        inner_path_obj = Some(InnerPathInfo {
                            root_obj_id: the_root_obj_id,
                            inner_obj_path: the_obj_path.unwrap(),
                            path_obj_jwt: path_obj_jwt,
                            mtree_path: None,
                        });
                    }
                }
            }
        } 
    }
    drop(named_mgr);
    let get_result:GetObjResult;
    if obj_content.is_some() {
        //root_obj/inner_path = obj_content
        //get_result = get_obj_result(named_mgr2, &obj_id, start, inner_obj_p).await?;     
        get_result = GetObjResult::new_value_result(obj_content.unwrap());  
    } else {
        if obj_id.is_none() {
            return Err(anyhow::anyhow!("ndn_router:failed to get obj id from request!,request.uri():{}",req.uri()));
        }
        let obj_id = obj_id.unwrap();
        get_result = get_obj_result(named_mgr2, &obj_id, start).await?;
    }
    let response = build_response_by_obj_get_result(get_result, start,inner_path_obj).await?;
    Ok(response)
}


#[cfg(test)] 
mod tests {
    use super::*;
    use buckyos_kit::*;
    use tokio::io::{AsyncReadExt,AsyncWriteExt};
    use crate::*;
    use serde_json::json;
    use cyfs_gateway_lib::*;

    fn generate_random_bytes(size: u64) -> Vec<u8> {
        let mut rng = rand::rng();
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
                        "is_object_id_in_path":true,
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
            let _ =start_cyfs_warp_server(test_server_config).await;
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
        let (mut chunk_writer,_) = named_mgr.open_chunk_writer(&chunk_id_a, chunk_a_size, 0).await.unwrap();
        chunk_writer.write_all(&chunk_a).await.unwrap();
        drop(chunk_writer);
        named_mgr.complete_chunk_writer(&chunk_id_a).await.unwrap();


        let chunk_b_size:u64 = 1024*1024*3 + 321*71;
        let chunk_b = generate_random_bytes(chunk_b_size);
        let mut hasher = ChunkHasher::new(None).unwrap();
        let hash_b = hasher.calc_from_bytes(&chunk_b);
        let chunk_id_b = ChunkId::from_sha256_result(&hash_b);
        info!("chunk_id_b:{}",chunk_id_b.to_string());
        let (mut chunk_writer,_) = named_mgr.open_chunk_writer(&chunk_id_b, chunk_b_size, 0).await.unwrap();
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

        // // Step 4.4: Use the ndn-client's get_obj_by_url to get a typical file_obj.content
        // let file_content_result = client.get_obj_by_url(&format!("http://{}/file_obj.content", warp_addr)).await;
        // assert!(file_content_result.is_ok(), "Failed to get file object content");

        // // Clean up
        // warp_server.stop().await.unwrap();
        // temp_dir.close().unwrap();
    }
}



