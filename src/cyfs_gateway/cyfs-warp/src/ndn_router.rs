use buckyos_kit::get_by_json_path;
use log::*;
use crate::{RouterResult,RouterError};
use hyper::{Request,Response,Body,StatusCode};

use std::{io::SeekFrom, sync::Arc};
use std::net::IpAddr;
use ndn_lib::*;
use cyfs_gateway_lib::{NamedDataMgrRouteConfig};
use serde_json::Value;
use crate::parse_range;

//1. get objid and inner path
//2. if enable, try use relative path to get objid and inner path
//3. if inneer path is not null, use get_json_by_path to get Value
//4. if Value is objid, return 
//5. return : Value | Reader | Text Record


enum LoadedObjBody {
    NamedObj(Value), //value, embeded obj_string
    Reader(ChunkReader,u64),//reader, chunk_size, embeded obj_string
    TextRecord(String),//text_record, verify_obj path  
}


struct LoadedObj {
    pub real_obj_id:Option<ObjId>,
    pub real_body:LoadedObjBody,
    pub path_obj_jwt:Option<String>,
}



impl LoadedObj {
    pub fn new_chunk_result(real_obj_id:ObjId,real_body:ChunkReader,chunk_size:u64)->Self {
        let body = LoadedObjBody::Reader(real_body,chunk_size);
        Self { real_obj_id:Some(real_obj_id), real_body:body, path_obj_jwt:None }
    }

    pub fn new_named_obj_result(real_obj_id:ObjId,real_body:Value)->Self {
        let body = LoadedObjBody::NamedObj(real_body);
        Self { real_obj_id:Some(real_obj_id), real_body:body, path_obj_jwt:None }
    }

    pub fn new_value_result(real_obj_id:Option<ObjId>,real_body:Value)->Self {
        let body_str = serde_json::to_string(&real_body).unwrap();
        let body = LoadedObjBody::TextRecord(body_str);
       
        Self { real_obj_id:real_obj_id, real_body:body, path_obj_jwt:None }
    }
}

async fn load_obj(mgr:Arc<tokio::sync::Mutex<NamedDataMgr>>,obj_id:&ObjId,offset:u64)->RouterResult<LoadedObj> {
    let real_mgr = mgr.lock().await;
    if obj_id.is_chunk() {
        let chunk_id = ChunkId::from_obj_id(&obj_id);
        let seek_from = SeekFrom::Start(offset);
        let (chunk_reader,chunk_size) = real_mgr.open_chunk_reader_impl(&chunk_id, seek_from, true).await
            .map_err(|e| {
                warn!("get chunk reader by objid failed: {}", e);
                match e {
                    NdnError::NotFound(e2) => RouterError::NotFound(e2),
                    _ => RouterError::Internal(format!("get chunk reader by objid failed: {}", e))
                }
            })?;

        debug!("ndn route -> chunk: {}, chunk_size: {}, offset: {}", obj_id.to_base32(), chunk_size, offset);
        return Ok(LoadedObj::new_chunk_result(obj_id.clone(),chunk_reader,chunk_size));
    } else  {
        //TODO: Add chunklist support
        let obj_body = real_mgr.get_object_impl(&obj_id,None).await.map_err(|e| {
            warn!("get object by objid failed: {}", e);
            match e {
                NdnError::NotFound(e2) => RouterError::NotFound(e2),
                _ => RouterError::Internal(format!("get object by objid failed: {}", e))
            }
        })?;
        debug!("ndn route -> obj {}", obj_body.to_string());
        return Ok(LoadedObj::new_named_obj_result(obj_id.clone(),obj_body));
    }
}

pub struct InnerPathInfo {
    pub root_obj_id:ObjId,
    pub inner_obj_path:String,
    pub inner_proof:Option<String>,
}

async fn build_response_by_obj_get_result(obj_load_result:LoadedObj,start:u64,inner_path_info:Option<InnerPathInfo>)->RouterResult<Response<Body>> {
    let body_result;
    let mut result = Response::builder();
    debug!("ndn_router:build_response_by_obj_get_result: obj_load_result: {:?}", obj_load_result.real_obj_id);

    if obj_load_result.real_obj_id.is_some() {
        result = result.header("cyfs-obj-id", obj_load_result.real_obj_id.unwrap().to_base32());
    }

    if inner_path_info.is_some() {
        let inner_path_info = inner_path_info.unwrap();
        result = result.header("cyfs-root-obj-id", inner_path_info.root_obj_id.to_base32());

        if inner_path_info.inner_proof.is_some() {
            result = result.header("cyfs-proof", inner_path_info.inner_proof.unwrap());
        }
    }

    if obj_load_result.path_obj_jwt.is_some() {
        result = result.header("cyfs-path-obj", obj_load_result.path_obj_jwt.unwrap());
    }

    match obj_load_result.real_body {
        LoadedObjBody::NamedObj(json_value) => {
            result = result.header("Content-Type", "application/json")
            .status(StatusCode::OK);
            body_result = result.body(Body::from(serde_json::to_string(&json_value).map_err(|e| {
                RouterError::Internal(format!("Failed to convert json value to string: {}", e))
            })?)).unwrap();
        }
        LoadedObjBody::Reader(chunk_reader,chunk_size) => {
            let stream = tokio_util::io::ReaderStream::new(chunk_reader);
            result = result.header("Accept-Ranges", "bytes")
                .header("Content-Type", "application/octet-stream")
                .header("Cache-Control", "public,max-age=31536000")
                .header("cyfs-obj-size", chunk_size.to_string());

            if start > 0 {
                debug!("ndn_router:build_response_by_obj_get_result: Content-Range: bytes {}-{}/{}", start, chunk_size - 1, chunk_size);
                result = result.header("Content-Range", format!("bytes {}-{}/{}", start, chunk_size - 1, chunk_size))
                .header("Content-Length", chunk_size - start)
                .status(StatusCode::PARTIAL_CONTENT);
            } else { 
                debug!("ndn_router:build_response_by_obj_get_result: Content-Length: {}", chunk_size);
                result = result.header("Content-Length", chunk_size)
                .status(StatusCode::OK);
            }
            body_result = result.body(Body::wrap_stream(stream)).unwrap();
        }
        LoadedObjBody::TextRecord(text_record) => {
            result = result.header("Content-Type", "plain/text")
                .status(StatusCode::OK);
            body_result = result.body(Body::from(text_record)).unwrap();
        }
    }
    Ok(body_result)
}

pub async fn handle_chunk_put(mgr_config: &NamedDataMgrRouteConfig, req: Request<Body>, _host: &str, _client_ip:IpAddr,_route_path: &str) -> RouterResult<Response<Body>> {
    if mgr_config.read_only {
        error!("Named manager is read only,cann't process put");
        return Err(RouterError::Forbidden("Named manager is read only".to_string()));
    }
    
    if !mgr_config.enable_zone_put_chunk {
        error!("Named manager is not enable zone put chunk");
        return Err(RouterError::Forbidden("Named manager is not enable zone put chunk".to_string()));
    }

    let path = req.uri().path();
    let obj_id = match ObjId::from_path(path) {
        Ok((id, _)) => id,
        Err(_) => return Err(RouterError::BadRequest("Invalid object ID in path".to_string()))
    };
    
    let named_mgr_id = mgr_config.named_data_mgr_id.clone();
    let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(Some(named_mgr_id.as_str())).await
        .ok_or_else(|| RouterError::NotFound(format!("Named manager not found: {}", named_mgr_id)))?;
    
    let named_mgr_lock = named_mgr.lock().await;
    let chunk_id = ChunkId::from_obj_id(&obj_id);

    // 获取总大小
    let total_size = req.headers()
        .get("cyfs-chunk-size")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
        // 如果是最后一块数据，完成写入

    // 打开写入器
    let (chunk_writer, _) = named_mgr_lock.open_chunk_writer_impl(&chunk_id, total_size, 0).await.map_err(|e| {
        warn!("Failed to open chunk writer: {}", e);
        match e {
            NdnError::NotFound(e2) => RouterError::NotFound(e2),
            _ => RouterError::Internal(format!("Failed to open chunk writer: {}", e))
        }
    })?;
    drop(named_mgr_lock);
    
    // 读取整个请求体到内存
    let body_bytes = hyper::body::to_bytes(req.into_body()).await
        .map_err(|e| RouterError::BadRequest(format!("Failed to read request body: {}", e)))?;
    
    // 创建一个内存读取器
    let chunk_reader = std::io::Cursor::new(body_bytes);
    
    // 使用 copy_chunk 函数
    let write_result = ndn_lib::copy_chunk(
        chunk_id.clone(), 
        chunk_reader, 
        chunk_writer, 
        None, 
        None::<fn(ChunkId, u64, &Option<ChunkHasher>) -> _>
    ).await
        .map_err(|e| {
            warn!("Failed to copy chunk: {}", e);
            match e {
                NdnError::NotFound(e2) => RouterError::NotFound(e2),
                _ => RouterError::Internal(format!("Failed to copy chunk: {}", e))
            }
        })?;
    
    if write_result == total_size {
        let named_mgr_lock = named_mgr.lock().await;
        named_mgr_lock.complete_chunk_writer_impl(&chunk_id).await.map_err(|e| {
            warn!("Failed to complete chunk: {}", e);
            RouterError::Internal(format!("Failed to complete chunk: {}", e))
        })?;
    } else {
        warn!("Failed to complete chunk: {}", write_result);
        return Err(RouterError::Internal(format!("Failed to complete chunk: {}", write_result)));
    }
    
    return Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::empty()).unwrap());
}

pub async fn handle_chunk_status(mgr_config: &NamedDataMgrRouteConfig, req: Request<Body>, _host: &str, _client_ip:IpAddr,_route_path: &str) -> RouterResult<Response<Body>> {
    let path = req.uri().path();
    let obj_id = match ObjId::from_path(path) {
        Ok((id, _)) => id,
        Err(_) => return Err(RouterError::BadRequest("Invalid object ID in path".to_string()))
    };
    

    let chunk_id = ChunkId::from_obj_id(&obj_id);

    let (chunk_state,chunk_size,progress) = NamedDataMgr::query_chunk_state(
        Some(mgr_config.named_data_mgr_id.as_str()),&chunk_id).await.map_err(|e| {
        warn!("Failed to query chunk state: {}", e);
        match e {
            NdnError::NotFound(e2) => RouterError::NotFound(e2),
            _ => RouterError::Internal(format!("Failed to query chunk state: {}", e))
        }
    })?;
    let status_code;
    match chunk_state {
        ChunkState::New => {
            status_code = StatusCode::CREATED;
        }
        ChunkState::Completed => {
            status_code = StatusCode::OK;
        }
        ChunkState::Incompleted => {
            status_code = StatusCode::PARTIAL_CONTENT;
        }
        ChunkState::Disabled => {
            status_code = StatusCode::FORBIDDEN;
        }
        ChunkState::NotExist => {
            status_code = StatusCode::NOT_FOUND;
        }
        ChunkState::Link(_) => {
            status_code = StatusCode::MOVED_PERMANENTLY;
        }
    }
    return Ok(Response::builder()
        .status(status_code)
        .header("Content-Length", chunk_size.to_string())
        .header("cyfs-chunk-status", chunk_state.to_str())
        .header("cyfs-chunk-progress", progress)
        .body(Body::empty()).unwrap());
}


pub async fn handle_ndn_get(mgr_config: &NamedDataMgrRouteConfig, req: Request<Body>, host: &str, _client_ip:IpAddr,route_path: &str) -> RouterResult<Response<Body>> {
    let named_mgr_id = mgr_config.named_data_mgr_id.clone();
    let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(Some(named_mgr_id.as_str())).await;
   
    if named_mgr.is_none() {
        warn!("Named manager not found: {}", named_mgr_id);
        return Err(RouterError::NotFound(format!("Named manager not found: {}", named_mgr_id)));
    }
    debug!("named manager founded!");
    let named_mgr = named_mgr.unwrap();
    let named_mgr2 = named_mgr.clone();

    let range_str = req.headers().get(hyper::header::RANGE);
    let mut start = 0;
    if range_str.is_some() {
        let range_str = range_str.unwrap().to_str().unwrap();
        (start,_) = parse_range(range_str,u64::MAX)
            .map_err(|e| {
                warn!("parse range failed: {}", e);
                RouterError::BadRequest(format!("parse range failed: {}", e))
            })?;
    }

    let req_path = req.uri().path();
    let mut obj_id:Option<ObjId> = None;
    let mut path_obj_jwt:Option<String> = None;

    let mut root_obj_id:Option<ObjId> = None;
    let mut _inner_obj_path:Option<String> = None;
    let mut inner_path_info:Option<InnerPathInfo> = None;


    let _user_id = "guest";//TODO: session_token from cookie
    let _app_id = "unknown";

    //get objid by hostname
    let obj_id_result = ObjId::from_hostname(host);
    if obj_id_result.is_ok() {
        obj_id = Some(obj_id_result.unwrap());
    }

    if obj_id.is_none() && mgr_config.is_object_id_in_path {
        let obj_id_result = ObjId::from_path(req_path);
        if obj_id_result.is_ok() {
            let (the_obj_id,the_obj_path) = obj_id_result.unwrap();
            if the_obj_path.is_some() {
                debug!("get root object_id and inner_path from url");
                _inner_obj_path = the_obj_path;
                root_obj_id = Some(the_obj_id);
            } else {
                debug!("get object id from url");
                obj_id = Some(the_obj_id);
            }
        }
    } 



    if obj_id.is_none() && mgr_config.enable_mgr_file_path {
        let sub_path = buckyos_kit::get_relative_path(route_path, req_path);
        let real_named_mgr = named_mgr.lock().await;
        let target_obj_result = real_named_mgr.get_obj_id_by_path_impl(&sub_path).await;
        if target_obj_result.is_ok() {
            info!("ndn_router:get_obj_id_by_path success,ndn_path:{}",sub_path);
            // will return (obj_id,obj_json_str)
            let (target_obj_id,the_path_obj_jwt) = target_obj_result.unwrap();
            path_obj_jwt = the_path_obj_jwt;
            obj_id = Some(target_obj_id);
        } else {
            //root_obj/inner_path = obj_id,
            let root_obj_id_result = real_named_mgr.select_obj_id_by_path_impl(&sub_path).await;
            if root_obj_id_result.is_ok() {
                let (the_root_obj_id,_the_path_obj_jwt,the_inner_path) = root_obj_id_result.unwrap();
                if the_inner_path.is_none() {
                    return Err(RouterError::NotFound("ndn_router:cann't found target object,inner_obj_path is not found".to_string()));
                }
                _inner_obj_path = the_inner_path.clone();
                info!("ndn_router:select_obj_id_by_path success,ndn_path:{},obj_inner_path:{} ",sub_path,the_inner_path.clone().unwrap_or("None".to_string()));
                if the_root_obj_id.is_chunk() {
                    return Err(RouterError::BadRequest("ndn_router:chunk is not supported to be root obj".to_string()));
                }
                if the_root_obj_id.is_big_container() {
                    //TODO: not support now
                    warn!("ndn_router:big container is not supported to be root obj");
                    return Err(RouterError::BadRequest("ndn_router:big container is not supported to be root obj".to_string()));
                } 
                root_obj_id = Some(the_root_obj_id);
            }
        }
    } 

    if obj_id.is_none() && root_obj_id.is_none() {
        warn!("ndn_router:cann't get obj id from request!,request.uri():{}",req.uri());
        return Err(RouterError::NotFound(format!("NotFound! failed to get obj id from request!,request.uri():{}",req.uri())));
    }

    debug!("ndn_router will load object, obj_id: {:?},root_obj_id: {:?}",obj_id,root_obj_id);
    //load obj
    if _inner_obj_path.is_some() {
        let root_obj_id = root_obj_id.unwrap();
        let inner_obj_path = _inner_obj_path.unwrap();
        let real_named_mgr = named_mgr.lock().await;
        let root_obj_json = real_named_mgr.get_object_impl(&root_obj_id, None).await.map_err(|e| {
            warn!("Failed to get object: {}", e);
            match e {
                NdnError::NotFound(e2) => RouterError::NotFound(e2),
                _ => RouterError::Internal(format!("Failed to get object: {}", e))
            }
        })?;

        let obj_filed = get_by_json_path(&root_obj_json, &inner_obj_path);
        if obj_filed.is_none() {
            warn!("ndn_router:cann't found target object,inner_obj_path {} is not valid",&inner_obj_path);
            return Err(RouterError::BadRequest("ndn_router:cann't found target object,inner_obj_path is not valid".to_string()));
        } 

        //this is the target content or target obj_id
        inner_path_info = Some(InnerPathInfo {
            root_obj_id: root_obj_id,
            inner_obj_path: inner_obj_path,
            inner_proof: None,
        });   

        let obj_filed = obj_filed.unwrap();
        if obj_filed.is_string() {
            let obj_id_str = obj_filed.as_str().unwrap();
            let p_obj_id = ObjId::new(obj_id_str);
            if p_obj_id.is_ok() {
                obj_id = Some(p_obj_id.unwrap());
            } 
        }

        if obj_id.is_none() {
            //return root_obj's field
            let mut load_result = LoadedObj::new_value_result(None,obj_filed);
            load_result.path_obj_jwt = path_obj_jwt;
            let response = build_response_by_obj_get_result(load_result, start,inner_path_info).await?;
            return Ok(response);
        }
    } 
    debug!("ndn_router:obj_id: {:?}",obj_id);
    let obj_id = obj_id.unwrap();
    debug!("ndn_router:before load obj");
    let mut load_result:LoadedObj = load_obj(named_mgr2, &obj_id, start).await?;
    load_result.path_obj_jwt = path_obj_jwt;
    let response = build_response_by_obj_get_result(load_result, start,inner_path_info).await.map_err(
        |e| {
            warn!("ndn_router:build_response_by_obj_get_result failed: {}", e);
            e
        }
    )?;
    debug!("ndn_router:build_response_by_obj_get_result success");
    return Ok(response);

}


pub async fn handle_ndn(mgr_config: &NamedDataMgrRouteConfig, req: Request<Body>, host: &str, _client_ip:IpAddr,route_path: &str) -> RouterResult<Response<Body>> {
    if req.method() == hyper::Method::PUT || req.method() == hyper::Method::PATCH{
        return handle_chunk_put(mgr_config, req, host, _client_ip, route_path).await;
    }

    if req.method() == hyper::Method::HEAD {
        return handle_chunk_status(mgr_config, req, host, _client_ip, route_path).await;
    }

    if req.method() == hyper::Method::GET {
        return handle_ndn_get(mgr_config, req, host, _client_ip, route_path).await;
    }
    
    return Err(RouterError::BadRequest(format!("Invalid method: {}", req.method())));
}


#[cfg(test)] 
mod tests {
    use super::*;
    use buckyos_kit::*;
    use rand::RngCore;
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
    async fn test_ndn_basic_op() {
        std::env::set_var("BUCKY_LOG", "debug");
        init_logging("ndn_client_test",false);
        let test_server_config = json!({
            "tls_port":3243,
            "http_port":3280,
            "hosts": {
              "*": {
                "enable_cors":true,
                "routes": {
                  "/ndn/": {
                    "named_mgr": {
                        "named_data_mgr_id":"test_pub",
                        "read_only":false,
                        "guest_access":true,
                        "is_object_id_in_path":true,
                        "enable_mgr_file_path":true,
                        "enable_zone_put_chunk":true
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
        
        let pub_named_mgr = NamedDataMgr::from_config(
            Some("test_pub".to_string()),
            temp_dir.path().to_path_buf(),
            config
        ).await.unwrap();
        let chunk_a_size:u64 = 1024*1024 + 321;
        let chunk_a = generate_random_bytes(chunk_a_size);
        let mut hasher = ChunkHasher::new(None).unwrap();
        let hash_a = hasher.calc_from_bytes(&chunk_a);
        let chunk_id_a = ChunkId::from_sha256_result(&hash_a);
        info!("chunk_id_a:{}",chunk_id_a.to_string());
        let (mut chunk_writer,_) = pub_named_mgr.open_chunk_writer_impl(&chunk_id_a, chunk_a_size, 0).await.unwrap();
        chunk_writer.write_all(&chunk_a).await.unwrap();
        drop(chunk_writer);
        pub_named_mgr.complete_chunk_writer_impl(&chunk_id_a).await.unwrap();
        info!("put chunk_id_a {} to test_pub named mgr OK!",chunk_id_a.to_string());


        let chunk_b_size:u64 = 1024*1024*3 + 321*71;
        let chunk_b = generate_random_bytes(chunk_b_size);
        let mut hasher = ChunkHasher::new(None).unwrap();
        let hash_b = hasher.calc_from_bytes(&chunk_b);
        let chunk_id_b = ChunkId::from_sha256_result(&hash_b);
        info!("chunk_id_b:{}",chunk_id_b.to_string());
        let (mut chunk_writer,_) = pub_named_mgr.open_chunk_writer_impl(&chunk_id_b, chunk_b_size, 0).await.unwrap();
        chunk_writer.write_all(&chunk_b).await.unwrap();
        drop(chunk_writer);
        pub_named_mgr.complete_chunk_writer_impl(&chunk_id_b).await.unwrap();
        info!("put chunk_id_b {} to test_pub named mgr OK!",chunk_id_b.to_string());

        
        //http://localhost:3280/ndn/test/chunk_a -> chunk_id_a
        let test_path = "/test/chunk_a".to_string();
        // Bind chunk to path
        pub_named_mgr.create_file_impl(
            test_path.as_str(),
            &chunk_id_a.to_obj_id(),
            "test_app",
            "test_user"
        ).await.unwrap();
        
        //http://localhost:3280/ndn/test/fileb/content -> chunk_id_b
        let path2 = "/test/fileb".to_string();
        let file_obj = FileObject::new("fileb".to_string(),chunk_b_size,chunk_id_b.to_string());
        let (file_obj_id,file_obj_str) = file_obj.gen_obj_id();
        info!("file_obj_id -> chunk_id:{}",file_obj_id.to_string());
        pub_named_mgr.put_object_impl(&file_obj_id, &file_obj_str).await.unwrap();
        pub_named_mgr.create_file_impl(
            path2.as_str(),
            &file_obj_id,
            "test_app",
            "test_user"
        ).await.unwrap();

        info!("named_mgr [test_pub] init OK!");
        NamedDataMgr::set_mgr_by_id(Some("test_pub"),pub_named_mgr).await.unwrap();
        //===================================================================
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
        drop(named_mgr_test);
    
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        // // Step 3: Configure the ndn-client and set the cyfs-warp address (obj_id in path)
        info!("ndn_client will pull chunk_id_a");
        let mut client = NdnClient::new("http://localhost:3280/ndn/".to_string(),None,Some("test_client".to_string()));
        client.force_trust_remote = true;
        client.pull_chunk(chunk_id_a.clone(),Some("test_client")).await.unwrap();

        let named_mgr_client = NamedDataMgr::get_named_data_mgr_by_id(Some("test_client")).await.unwrap();
        let real_named_mgr_client = named_mgr_client.lock().await;
        let (mut reader,len) = real_named_mgr_client.open_chunk_reader_impl(&chunk_id_a,SeekFrom::Start(0),false).await.unwrap();
        assert_eq!(len,chunk_a_size);
        drop(real_named_mgr_client);
        let mut buffer = vec![0u8;chunk_a_size as usize];
        reader.read_exact(&mut buffer).await.unwrap();
        assert_eq!(buffer,chunk_a);


        //Step 4.1: Use the ndn-client's get_obj_by_url interface to get the fileb object
        info!("ndn_client will get obj fileb");
        let obj_result = client.get_obj_by_url("http://localhost:3280/ndn/test/fileb",None).await;
        info!("obj_result:{:?}",obj_result);
        assert!(obj_result.is_ok(), "Failed to get object by URL");

        info!("ndn_client will open chunk reader for fileb.content");
        let (mut reader,cyfs_resp) = client.open_chunk_reader_by_url("http://localhost:3280/ndn/test/fileb/content",None,None).await.unwrap();
        let mut buffer = vec![0u8;chunk_b_size as usize];
        reader.read_exact(&mut buffer).await.unwrap();
        assert_eq!(cyfs_resp.obj_size.unwrap(),chunk_b_size);
        assert_eq!(buffer,chunk_b);

        // Step 5: Test put chunk functionality
      
        // Put the chunk using the client
        let named_mgr_client = NamedDataMgr::get_named_data_mgr_by_id(Some("test_client")).await.unwrap();
        let real_named_mgr_client = named_mgr_client.lock().await;

        let chunk_c_size:u64 = 1024*1024*3 + 321*71;
        let chunk_c = generate_random_bytes(chunk_c_size);
        let mut hasher = ChunkHasher::new(None).unwrap();
        let hash_c = hasher.calc_from_bytes(&chunk_c);
        let chunk_id_c = ChunkId::from_sha256_result(&hash_c);
        info!("chunk_id_c:{}",chunk_id_c.to_string());
        let (mut chunk_writer,progress_info) = real_named_mgr_client.open_chunk_writer_impl(&chunk_id_c, chunk_c_size, 0).await.unwrap();
        chunk_writer.write_all(&chunk_c).await.unwrap();
        drop(chunk_writer);
        real_named_mgr_client.complete_chunk_writer_impl(&chunk_id_c).await.unwrap();
        drop(real_named_mgr_client);

        info!("ndn_client will push a new chunk");
        let put_result = client.push_chunk(chunk_id_c.clone(), None).await;
        assert!(put_result.is_ok(), "Failed to put chunk: {:?}", put_result.err());

        let named_mgr_client = NamedDataMgr::get_named_data_mgr_by_id(Some("test_pub")).await.unwrap();
        let real_named_mgr_client = named_mgr_client.lock().await;
        let (mut _reader,len) = real_named_mgr_client.open_chunk_reader_impl(&chunk_id_c,SeekFrom::Start(0),false).await.unwrap();
        assert_eq!(len,chunk_c_size);
        
    }


}



