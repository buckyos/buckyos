
use std::collections::HashMap;
use reqwest::header::HeaderMap;

use crate::{ObjId, NdnResult, NdnError};

enum CYFSUrlMode {
    PathMode,//objid at url path
    HostnameMode,//objid at url hostname
}

#[derive(Debug,Clone)]
pub struct CYFSHttpRespHeaders {
    pub obj_id:Option<ObjId>,//cyfs-obj-id
    pub chunk_size:Option<u64>,//cyfs-data-size
    pub obj_path:Option<String>,//cyfs-obj-path
    pub embed_objs:Option<HashMap<ObjId,String>>,//cyfs-$objid : $obj_json_str
}

pub fn cyfs_get_obj_id_from_url(cyfs_url:&str)->NdnResult<ObjId> {
    unimplemented!()
}

pub fn cyfs_get_obj_path_from_url(cyfs_url:&str)->NdnResult<String> {
    unimplemented!()
}

pub fn gen_cyfs_obj_url(obj_id:&ObjId,url_mode:CYFSUrlMode)->String {
    unimplemented!()
}

pub fn get_cyfs_resp_headers(headers:&HeaderMap)->NdnResult<CYFSHttpRespHeaders> {
    let mut real_obj_id = None;
    let obj_id = headers.get("cyfs-obj-id");
    if obj_id.is_some() {
        let obj_id = obj_id.unwrap().to_str().unwrap();
        real_obj_id = Some(ObjId::from_str(obj_id)?);
    }

    let mut real_chunk_size = None;
    let chunk_size = headers.get("cyfs-data-size");
    if chunk_size.is_some() {
        let chunk_size = chunk_size.unwrap().to_str().unwrap();
        let chunk_size = chunk_size.parse::<u64>().map_err(|e| {
            NdnError::DecodeError(format!("get chunk size from headers failed:{}",e.to_string()))
        })?;
        real_chunk_size = Some(chunk_size);
    }

    let mut real_obj_path = None;
    let obj_path = headers.get("cyfs-obj-path");
    if obj_path.is_some() {
        let obj_path = obj_path.unwrap().to_str().unwrap();
        real_obj_path = Some(obj_path.to_string());
    }

    //TODO: get embed objs

    return Ok(CYFSHttpRespHeaders {
        obj_id:real_obj_id,
        chunk_size:real_chunk_size,
        obj_path:real_obj_path,
        embed_objs:None,
    });
}
