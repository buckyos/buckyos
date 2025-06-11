
use std::collections::HashMap;
use reqwest::header::HeaderMap;
use url::Url;
use crate::{ObjId, NdnResult, NdnError, PathObject};

enum CYFSUrlMode {
    PathMode,//objid at url path
    HostnameMode,//objid at url hostname
}

#[derive(Debug,Clone)]
pub struct CYFSHttpRespHeaders {
    pub obj_id:Option<ObjId>,//cyfs-obj-id
    pub obj_size:Option<u64>,//Content-Length
    //if use R-Path http mode ,need this
    pub path_obj:Option<String>,//cyfs-path-obj jwt

    pub root_obj_id:Option<ObjId>,//cyfs-root-obj-id
    pub mtree_path:String,//cyfs-mtree-path
    pub embed_objs:Option<HashMap<ObjId,String>>,//cyfs-$objid : $obj_json_str
}

//return (objid,obj_path)
pub fn cyfs_get_obj_id_from_url(cyfs_url:&str)->NdnResult<(ObjId,Option<String>)> {
    let url = Url::parse(cyfs_url).map_err(|e|{
        NdnError::InvalidId(format!("parse cyfs url failed:{}",e.to_string()))
    })?;
    let host = url.host_str();
    if host.is_none() {
        return Err(NdnError::InvalidId(format!("cyfs url host not found:{}",cyfs_url)));
    }
    let host = host.unwrap();
    let obj_id = ObjId::from_hostname(host);
    if obj_id.is_ok() {
        let obj_id = obj_id.unwrap();
        let obj_path = url.path();
        if obj_path.is_empty() {
            return Ok((obj_id,None));
        }
        return Ok((obj_id,Some(obj_path.to_string())));
    } else {
        return ObjId::from_path(url.path());
    }
}


// pub fn gen_cyfs_obj_url(obj_id:&ObjId,url_mode:CYFSUrlMode)->String {
//     unimplemented!()
// }

pub fn get_cyfs_resp_headers(headers:&HeaderMap)->NdnResult<CYFSHttpRespHeaders> {
    debug!("get_cyfs_resp_headers:headers:{:?}",headers);
    let mut real_obj_id = None;
    let obj_id = headers.get("cyfs-obj-id");
    if obj_id.is_some() {
        let obj_id = obj_id.unwrap().to_str().unwrap();
        real_obj_id = Some(ObjId::new(obj_id)?);
    }

    let mut real_chunk_size = None;
    let chunk_size = headers.get("cyfs-obj-size");
    if chunk_size.is_some() {
        let chunk_size = chunk_size.unwrap().to_str().unwrap();
        let chunk_size = chunk_size.parse::<u64>().map_err(|e| {
            NdnError::DecodeError(format!("get chunk size from headers failed:{}",e.to_string()))
        })?;
        real_chunk_size = Some(chunk_size);
    }

    let mut cyfs_root_obj_id = None;
    let root_obj_id = headers.get("cyfs-root-obj-id");
    if root_obj_id.is_some() {
        let root_obj_id = root_obj_id.unwrap().to_str().unwrap();
        cyfs_root_obj_id = Some(ObjId::new(root_obj_id)?);
    }

    let mut real_path_obj_jwt = None;
    let path_obj_jwt = headers.get("cyfs-path-obj");
    if path_obj_jwt.is_some() {
        let path_obj_jwt = path_obj_jwt.unwrap().to_str().unwrap();
        real_path_obj_jwt = Some(path_obj_jwt.to_string());
    }

    //TODO: get embed objs

    return Ok(CYFSHttpRespHeaders {
        obj_id:real_obj_id,
        obj_size:real_chunk_size,
        path_obj:real_path_obj_jwt,
        root_obj_id:cyfs_root_obj_id,
        mtree_path:String::new(),
        embed_objs:None,
    });
}
