use std::{collections::{BTreeMap, HashMap}, ops::Range};
use name_lib::EncodedDocument;
use sha2::{Sha256, Digest};
use crate::{NdnResult, NdnError};
//objid link to a did::EncodedDocument
#[derive(Debug, Clone,Eq, PartialEq)]
pub struct ObjId {
    pub obj_type : String,
    pub obj_id_string : String,
}

impl ObjId {
    pub fn new(obj_type:String,obj_id:String)->Self {
        Self { obj_type: obj_type, obj_id_string: obj_id }
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
        format!("{}:{}",self.obj_type,self.obj_id_string)
    }

    pub fn from_string(obj_id_str:&str)->NdnResult<Self> {
        let split = obj_id_str.split(":").collect::<Vec<&str>>();
        if split.len() != 2 {
            return Err(NdnError::InvalidId(obj_id_str.to_string()));
        }
        Ok(Self { obj_type: split[0].to_string(), obj_id_string: split[1].to_string() })
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

pub fn verify_named_object(obj_id:&ObjId,json_value:&serde_json::Value)->bool {
    let (obj_id2,json_str) = build_named_object_by_json(obj_id.obj_type.as_str(),json_value);
    if obj_id2 != *obj_id {
        return false;
    }
   return true;
}

pub fn verify_named_object_from_jwt(obj_id:&ObjId,jwt_str:&str)->NdnResult<bool> {
    let claims = name_lib::decode_jwt_claim_without_verify(jwt_str)
        .map_err(|e|NdnError::DecodeError(format!("decode jwt failed:{}",e.to_string())))?;

    let (obj_id2,json_str) = build_named_object_by_json(obj_id.obj_type.as_str(),&claims);
    if obj_id2 != *obj_id {
        return Ok(false);
    }
   return Ok(true);
}





#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn test_obj_id() {
        let obj_id = ObjId::new("sha256".to_string(),"1234567890".to_string());
        assert_eq!(obj_id.obj_type,"sha256");
        assert_eq!(obj_id.obj_id_string,"1234567890");
        assert_eq!(obj_id.is_chunk(),true);
    }
    #[test]
    fn test_build_obj_id() {
        let json_value = json!({"age":18,"name":"test"});
        let (obj_id,json_str) = build_named_object_by_json("jobj",&json_value);
        assert_eq!(obj_id.obj_type,"jobj");
        //assert_eq!(obj_id.obj_id_string,"02KQC625Y4B1QGSCNPKSK0G0M2E204YBSYF77SYG0QJKEFEXAPBG");
        //assert_eq!(obj_id.to_string(),"jobj:02KQC625Y4B1QGSCNPKSK0G0M2E204YBSYF77SYG0QJKEFEXAPBG");
        let json_value2 = json!({"name":"test","age":18});
        let (obj_id2,json_str2) = build_named_object_by_json("jobj",&json_value2);
        assert_eq!(obj_id,obj_id2);
        println!("obj_id2 : {}",obj_id2.to_string());

        assert_eq!(verify_named_object(&obj_id,&json_value2),true);

    }
}



