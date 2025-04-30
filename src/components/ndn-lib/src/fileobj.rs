use buckyos_kit::{buckyos_get_unix_timestamp,is_default};
use serde::{Serialize,Deserialize};

use crate::{ChunkId, LinkData};
use std::collections::HashMap;
use crate::{OBJ_TYPE_FILE,OBJ_TYPE_PATH,build_named_object_by_json,ObjId};
use serde_json::Value;
//TODO：NDN如何提供一种通用机制，检查FileObject在本地是 完全存在的 ？ 在这里的逻辑是FileObject的Content(存在)
// 思路：Object如果引用了另一个Object,要区分这个引用是强引用(依赖）还是弱引用，
#[derive(Serialize,Deserialize,Clone)]
pub struct FileObject {
    pub name:String,
    pub size:u64,
    pub content:String,//chunkid
    #[serde(default)]
    #[serde(skip_serializing_if = "is_default")]
    pub exp:u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta:Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime:Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner:Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create_time:Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_list:Option<HashMap<String,Vec<String>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub links:Option<Vec<LinkData>>,
    #[serde(flatten)]
    pub extra_info: HashMap<String, Value>,
}

impl FileObject {
    pub fn new(name:String,size:u64,content:String)->Self {
        Self {name,size,content,meta:None,mime:None,owner:None,exp:0,
            create_time:None,chunk_list:None,links:None,extra_info:HashMap::new()}
    }

    pub fn gen_obj_id(&self)->(ObjId, String) {
        let json_value = serde_json::to_value(self).unwrap();
        build_named_object_by_json(OBJ_TYPE_FILE, &json_value)
    }
}

#[derive(Serialize,Deserialize,Clone,Eq,PartialEq)]
pub struct PathObject {
    pub path:String,
    pub uptime:u64,
    pub target:ObjId,
    pub exp:u64,
}

impl PathObject {
    pub fn new(path:String,target:ObjId)->Self {
        Self {
            path,
            uptime:buckyos_get_unix_timestamp(),
            target,
            exp:buckyos_get_unix_timestamp() + 3600*24*365*3,
        }
    }

    pub fn gen_obj_id(&self)->(ObjId, String) {
        let json_value = serde_json::to_value(self).unwrap();
        build_named_object_by_json(OBJ_TYPE_PATH, &json_value)
    }
}

#[cfg(test)]
mod tests {
    use crate::build_named_object_by_json;

    use super::*;

    #[test]
    fn test_file_object() {
        let file_object = FileObject::new("test.data".to_string(),100,"sha256:1234567890".to_string());
        let file_object_str = serde_json::to_string(&file_object).unwrap();
        println!("file_object_str {}",file_object_str);

        let (objid,obj_str) = file_object.gen_obj_id();
        println!("fileobj id {}",objid.to_string());
        println!("fileobj str {}",obj_str);
    }

    #[test]
    fn test_path_object() {
        let path_object = PathObject::new("/repo/pub_meta_index.db".to_string(),ObjId::new("sha256:1234567890").unwrap());
        let path_object_str = serde_json::to_string(&path_object).unwrap();
        println!("path_object_str {}",path_object_str);

        let (objid,obj_str) = path_object.gen_obj_id();
        println!("pathobj id {}",objid.to_string());
        println!("pathobj str {}",obj_str);
    }
}