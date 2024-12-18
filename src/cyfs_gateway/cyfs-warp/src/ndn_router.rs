use log::*;
use anyhow::Result;
use hyper::{Request,Response,Body,StatusCode};

use std::{io::SeekFrom, sync::Arc};
use std::net::IpAddr;
use ndn_lib::*;
use cyfs_gateway_lib::{NamedDataMgrRouteConfig};
use serde_json::Value;
use crate::parse_range;

//1) get objid and inner path
//2) if enable, try use relative path to get objid and inner path
//3) if inneer path is not null, use get_json_by_path to get Value
//4) if Value is objid, return 
//4) return : Value | Reader | Text Record


enum GetObjResultBody {
    Value(Value), //value, embeded obj_string
    Reader(ChunkReader,u64),//reader, chunk_size, embeded obj_string
    TextRecord(String),//text_record, verify_obj path  
}

struct GetObjResult {
    pub real_obj_id:ObjId,
    pub real_body:GetObjResultBody,
    pub parent_obj_body_str:Option<String>,
}

impl GetObjResult {
    pub fn new_chunk_result(real_obj_id:ObjId,real_body:ChunkReader,chunk_size:u64,parent_obj_body_str:Option<String>)->Self {
        let body = GetObjResultBody::Reader(real_body,chunk_size);
        Self { real_obj_id, real_body:body, parent_obj_body_str }
    }

    pub fn new_value_result(real_obj_id:ObjId,real_body:Value,parent_obj_body_str:Option<String>)->Self {
        let body = GetObjResultBody::Value(real_body);
        Self { real_obj_id, real_body:body, parent_obj_body_str }
    }

    pub fn new_text_result(real_obj_id:ObjId,real_body:String,parent_obj_body_str:Option<String>)->Self {
        let body = GetObjResultBody::TextRecord(real_body);
        Self { real_obj_id, real_body:body, parent_obj_body_str }
    }
}

async fn get_obj_result(mgr:Arc<tokio::sync::Mutex<NamedDataMgr>>,obj_id:&ObjId,offset:u64,obj_path:Option<String>,parent_obj_str:Option<String>)->Result<GetObjResult> {
    let real_mgr = mgr.lock().await;
    if obj_id.is_chunk() {
        let chunk_id = ChunkId::from_obj_id(&obj_id);
        let seek_from = SeekFrom::Start(offset);
        let (chunk_reader,chunk_size) = real_mgr.open_chunk_reader(&chunk_id, seek_from, true).await
            .map_err(|e| {
                warn!("get chunk reader by objid failed: {}", e);
                anyhow::anyhow!("get chunk reader by objid failed: {}", e)
            })?;
        info!("ndn route -> chunk: {}, chunk_size: {}, offset: {}", obj_id.to_base32(), chunk_size, offset);
        return Ok(GetObjResult::new_chunk_result(obj_id.clone(),chunk_reader,chunk_size,parent_obj_str));
    } else {
        let obj_body = real_mgr.get_object(&obj_id,obj_path).await?;
        if obj_body.is_string() {
            let obj_body_str = obj_body.as_str().unwrap();
            let p_obj_id = ObjId::new(&obj_body_str);
            if p_obj_id.is_err() {
                info!("ndn route -> obj.value: {}", obj_id.to_base32());
                return Ok(GetObjResult::new_value_result(obj_id.clone(),obj_body,parent_obj_str));
            } else {
                let p_obj_id = p_obj_id.unwrap();
                drop(real_mgr);
                return Box::pin(get_obj_result(mgr, &p_obj_id, offset, None,Some(obj_body_str.to_string()))).await;
            }
        } else {
            info!("ndn route -> obj {}", obj_body.to_string());
            return Ok(GetObjResult::new_value_result(obj_id.clone(),obj_body,parent_obj_str));
        }
    }
}

async fn build_response_by_obj_get_result(obj_get_result:GetObjResult,start:u64,obj_id:ObjId)->Result<Response<Body>> {
    let body_result;
    let mut result = Response::builder()
                    .header("cyfs-obj-id", obj_get_result.real_obj_id.to_base32());
    if obj_get_result.parent_obj_body_str.is_some() {
        result = result.header("cyfs-embeded-obj", obj_get_result.parent_obj_body_str.unwrap());
    }
    match obj_get_result.real_body {
        GetObjResultBody::Value(json_value) => {

            result = result.header("Content-Type", "application/json")
            .status(StatusCode::OK);
            body_result = result.body(Body::from(serde_json::to_string(&json_value)?))?;
        }
        GetObjResultBody::Reader(chunk_reader,chunk_size) => {

            let stream = tokio_util::io::ReaderStream::new(chunk_reader);
            result = result.header("Accept-Ranges", "bytes")
                .header("Content-Type", "application/octet-stream")
                .header("Cache-Control", "public,max-age=31536000")
                .header("cyfs-data-size", chunk_size.to_string());
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

pub async fn handle_ndn(mgr_config: &NamedDataMgrRouteConfig, req: Request<Body>, host: &str, client_ip:IpAddr,route_path: &str) -> Result<Response<Body>> {
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
    let mut obj_path:Option<String> = None;
    let path = req.uri().path();
    let user_id = "guest";
    let app_id = "unknown";

    if mgr_config.is_chunk_id_in_path {
        //let sub_path = path.trim_start_matches(path);
        let obj_id_result = ObjId::from_path(path);
        if obj_id_result.is_ok() {
            let (the_obj_id,the_obj_path) = obj_id_result.unwrap();
            obj_id = Some(the_obj_id);
            obj_path = the_obj_path;
        }
    } else {
        //get chunkid by hostname
        let obj_id_result = ObjId::from_hostname(host);
        if obj_id_result.is_ok() {
            obj_id = Some(obj_id_result.unwrap());
        }
    }

    if obj_id.is_none() {
        if mgr_config.enable_mgr_file_path {
            let sub_path = buckyos_kit::get_relative_path(route_path, path);
            let path_obj_id = named_mgr.get_obj_id_by_path(sub_path).await;
            if path_obj_id.is_ok() {
                obj_id = Some(path_obj_id.unwrap());
                //TODO: get obj_path from query?
            }
        } 
    }

    if obj_id.is_none() {
        return Err(anyhow::anyhow!("ndn_router:failed to get obj id from request!,request:{}",req.uri()));
    }
    let obj_id = obj_id.unwrap();
    drop(named_mgr);
    
    let get_result = get_obj_result(named_mgr2, &obj_id, start, obj_path,None).await?;
    let response = build_response_by_obj_get_result(get_result, start, obj_id).await?;
    Ok(response)
}


//unit test move to ndn-lib.ndn-client