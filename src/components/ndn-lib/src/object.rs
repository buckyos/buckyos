use std::{collections::{BTreeMap, HashMap}, ops::Range};
use name_lib::EncodedDocument;
use sha2::{Sha256, Digest};
//objid link to a did::EncodedDocument
pub struct ObjId {
    pub obj_type : String,
    pub obj_id : String,
}

impl ObjId {
    pub fn new(obj_type:String,obj_id:String)->Self {
        Self { obj_type: obj_type, obj_id: obj_id }
    }

    pub fn is_chunk(&self)->bool {
        if self.obj_type.starts_with("mix") {
            return true;
        }

        match self.obj_type.as_str() {
            "sha256" => true,
            "qcid" => true,
            _ => false,
        }
    }

    pub fn to_string(&self)->String {
        format!("{}:{}",self.obj_type,self.obj_id)
    }

    pub fn get_known_obj_type(&self)->u8 {
        0
    }
}

pub fn build_obj_id(obj_type:&str,obj_json_str:&str)->ObjId {
    let vec_u8 = obj_json_str.as_bytes().to_vec();
    let hash_value = Sha256::digest(&vec_u8);
    let obj_id = base32::encode(base32::Alphabet::Crockford, &hash_value);
    ObjId::new(obj_type.to_string(),obj_id)
}

pub fn build_named_object_by_json(obj_type:&str,json_value:&serde_json::Value)->(ObjId,String) {
        // 递归地处理 JSON 值，确保所有层级的对象都是有序的
        fn stabilize_json(value: &serde_json::Value) -> serde_json::Value {
            match value {
                serde_json::Value::Object(map) => {
                    let ordered: BTreeMap<String, serde_json::Value> = map.iter()
                        .map(|(k, v)| (k.clone(), stabilize_json(v)))
                        .collect();
                    serde_json::Value::Object(serde_json::Map::from_iter(ordered))
                }
                serde_json::Value::Array(arr) => {
                    // 递归处理数组中的每个元素
                    serde_json::Value::Array(
                        arr.iter()
                            .map(stabilize_json)
                            .collect()
                    )
                }
                // 其他类型直接克隆
                _ => value.clone(),
            }
        }

        let stable_value = stabilize_json(json_value);
        let json_str = serde_json::to_string(&stable_value)
            .unwrap_or_else(|_| "{}".to_string());
        let obj_id = build_obj_id(obj_type,&json_str);
        (obj_id,json_str)
}


pub struct ObjectMap {
    pub obj_map:HashMap<String,ObjId>,
}




