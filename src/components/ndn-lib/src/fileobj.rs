use serde::{Serialize,Deserialize};
use crate::{ChunkId, LinkData};
use std::collections::HashMap;
use crate::{OBJ_TYPE_FILE,build_named_object_by_json,ObjId};

//TODO：NDN如何提供一种通用机制，检查FileObject在本地是 完全存在的 ？ 在这里的逻辑是FileObject的Content(存在)
// 思路：Object如果引用了另一个Object,要区分这个引用是强引用(依赖）还是弱引用，
#[derive(Serialize,Deserialize,Clone)]
pub struct FileObject {
    pub name:String,
    pub size:u64,
    pub content:String,//chunkid
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
}

impl FileObject {
    pub fn new(name:String,size:u64,content:String)->Self {
        Self {name,size,content,meta:None,mime:None,owner:None,create_time:None,chunk_list:None,links:None}
    }

    pub fn gen_obj_id(&self)->(ObjId, String) {
        let json_value = serde_json::to_value(self).unwrap();
        build_named_object_by_json(OBJ_TYPE_FILE, &json_value)
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
}