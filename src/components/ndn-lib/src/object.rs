use std::{collections::{BTreeMap, HashMap}, ops::Range};
use name_lib::EncodedDocument;
use sha2::{Sha256, Digest};
use crate::{NdnResult, NdnError};
use crate::{OBJ_TYPE_FILE,OBJ_TYPE_DIR,OBJ_TYPE_MTREE,OBJ_TYPE_OBJMAPT,OBJ_TYPE_PACK,OBJ_TYPE_LIST};
use serde::{Deserialize, Serialize};
use jsonwebtoken::{encode, EncodingKey};
use std::fmt::Display;

//objid link to a did::EncodedDocument
#[derive(Debug, Clone,Eq, PartialEq, Serialize, Deserialize)]
pub struct ObjId {
    pub obj_type : String,
    pub obj_hash : Vec<u8>,//hash result
}

impl ObjId {
    pub fn new(objid_str:&str)->NdnResult<Self> {
        let split = objid_str.split(":").collect::<Vec<&str>>();
        let split_len = split.len();
        match split_len {
            1 => {
                //all encode in base32
                let vec_result = base32::decode(base32::Alphabet::Rfc4648Lower{ padding: false }, split[0]);
                if vec_result.is_none() {
                    return Err(NdnError::InvalidId(format!("decode base32 failed:{}",split[0])));
                }
                let vec_result = vec_result.unwrap();
                
                let pos = vec_result.iter()
                    .position(|&x| x == b':')
                    .ok_or_else(|| NdnError::InvalidId("separator ':' not found".to_string()))?;

                let obj_type = String::from_utf8(vec_result[..pos].to_vec())
                    .map_err(|_| NdnError::InvalidId("invalid utf8 in obj_type".to_string()))?;
                let obj_hash = vec_result[pos + 1..].to_vec();

                Ok(Self { obj_type, obj_hash })
            },
            2 => {
                let obj_type = split[0].to_string();
                let obj_hash = hex::decode(split[1])
                    .map_err(|e|NdnError::InvalidId(format!("decode hex failed:{}",e.to_string())))?;
                
                Ok(Self { obj_type: obj_type, obj_hash: obj_hash })       
            },
            _ => {
                return Err(NdnError::InvalidId(objid_str.to_string()));
            }
        }
    }

    pub fn new_by_raw(obj_type:String,hash_value:Vec<u8>)->Self {
        Self { obj_type: obj_type, obj_hash: hash_value }
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

    pub fn is_json(&self)->bool {
        if self.is_chunk() {
            return false;
        }

        match self.obj_type.as_str() {
            OBJ_TYPE_MTREE => false,
            OBJ_TYPE_OBJMAPT => false,
            OBJ_TYPE_PACK => false,
            OBJ_TYPE_LIST => false,
            _ => true,
        }
    }

    pub fn is_big_container(&self)->bool {
        match self.obj_type.as_str() {
            OBJ_TYPE_MTREE => true,
            OBJ_TYPE_OBJMAPT => true,
            _ => false,
        }
    }

    pub fn to_string(&self)->String {
        let hex_str = hex::encode(self.obj_hash.clone());
        format!("{}:{}",self.obj_type,hex_str)
    }

    pub fn to_base32(&self)->String {
        let mut vec_result:Vec<u8> = Vec::new();
        vec_result.extend_from_slice(self.obj_type.as_bytes());
        vec_result.push(b':');
        vec_result.extend_from_slice(&self.obj_hash);
        
        base32::encode(base32::Alphabet::Rfc4648Lower{ padding: false }, &vec_result)
    }

    pub fn from_hostname(hostname: &str) -> NdnResult<Self> {
        let sub_host = hostname.split(".").collect::<Vec<&str>>();
        let first_part = sub_host[0];
        return Self::new(first_part);
    }

    pub fn from_path(path:&str) -> NdnResult<(Self,Option<String>)> {
        let path_parts = path.split("/").collect::<Vec<&str>>();
        let path_parts2 = path_parts.clone();
        let mut part_index = 0;
        let part_len = path_parts.len();
        for part in path_parts {
            let obj_id = Self::new(part);
            if obj_id.is_ok() {
                if part_index < part_len - 1 {
                    return Ok((obj_id.unwrap(), Some(format!("/{}", path_parts2[part_index + 1..].join("/")))));
                } else {
                    return Ok((obj_id.unwrap(), None));
                }
            }
            part_index += 1;
        }
        return Err(NdnError::InvalidId(format!("no objid found in path:{}",path)));
    }
}

impl Display for ObjId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_base32())
    }
}

pub fn build_obj_id(obj_type:&str,obj_json_str:&str)->ObjId {
    let vec_u8 = obj_json_str.as_bytes().to_vec();
    let hash_value:Vec<u8> = Sha256::digest(&vec_u8).to_vec();
    ObjId::new_by_raw(obj_type.to_string(),hash_value)
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

pub fn named_obj_str_to_jwt(obj_json_str:&String,key:&EncodingKey,kid:Option<String>)->NdnResult<String> {
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::EdDSA);
    header.typ = None; // 默认为 JWT，设置为None以节约空间
    header.kid = kid;
    let obj_json = serde_json::from_str::<serde_json::Value>(&obj_json_str).map_err(|error| {
        NdnError::Internal(format!("Failed to parse json string :{}",error))
    })?;
    let jwt_str = encode(&header, &obj_json,key).map_err(|error| {
        NdnError::Internal(format!("Failed to generate jwt token :{}",error))
    })?;   

    Ok(jwt_str)
}

pub fn named_obj_to_jwt(obj_json:&serde_json::Value,key:&EncodingKey,kid:Option<String>)->NdnResult<String> {
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::EdDSA);
    header.typ = None; // 默认为 JWT，设置为None以节约空间
    header.kid = kid;
    let jwt_str = encode(&header, &obj_json,key).map_err(|error| {
        NdnError::Internal(format!("Failed to generate jwt token :{}",error))
    })?;   

    Ok(jwt_str)
}





#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use crate::cyfs_http::cyfs_get_obj_id_from_url;
    #[test]
    fn test_obj_id() {
        let obj_id = ObjId::new("sha256:0203040506").unwrap();
        //println!("obj_id : {:?}",obj_id);
        assert_eq!(obj_id.to_string(),"sha256:0203040506");
        //println!("obj_id to base32 : {}",obj_id.to_base32());
        assert_eq!(obj_id.to_base32(),"onugcmrvgy5aeayeauda");

        let obj_id2 = ObjId::new("onugcmrvgy5aeayeauda").unwrap();
        assert_eq!(obj_id2.to_string(),"sha256:0203040506");

        let obj_host = "onugcmrvgy5aeayeauda.ndn.cyfs.com";
        let obj_id3 = ObjId::from_hostname(obj_host).unwrap();
        assert_eq!(obj_id3.to_string(),"sha256:0203040506");

        let obj_path = "/sha256:0203040506/test.txt";
        let (obj_id4,obj_path2) = ObjId::from_path(obj_path).unwrap();
        assert_eq!(obj_id4.to_string(),"sha256:0203040506");
        assert_eq!(obj_path2,Some("/test.txt".to_string()));

        let (obj_id5,obj_path3) = cyfs_get_obj_id_from_url("http://www.cyfs.com/abc/sha256:0203040506/def/test.txt").unwrap();
        assert_eq!(obj_id5.to_string(),"sha256:0203040506");
        assert_eq!(obj_path3,Some("/def/test.txt".to_string()));

        let (obj_id6,obj_path4) = cyfs_get_obj_id_from_url("http://onugcmrvgy5aeayeauda.ndn.cyfs.com/abc/sha256:0203040506/def/test.txt").unwrap();
        assert_eq!(obj_id6.to_string(),"sha256:0203040506");
        assert_eq!(obj_path4,Some("/abc/sha256:0203040506/def/test.txt".to_string()));
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

        let json_str = serde_json::to_string_pretty(&json_value2).unwrap();
        let json_value3 = serde_json::from_str::<serde_json::Value>(&json_str).unwrap();
        let (obj_id3,json_str3) = build_named_object_by_json("jobj",&json_value3);
        assert_eq!(obj_id2,obj_id3);
        println!("obj_id2#base32 : {}",obj_id2.to_base32());
        println!("obj_id2#string : {}",obj_id2.to_string());

        assert_eq!(verify_named_object(&obj_id,&json_value2),true);

    }
}



