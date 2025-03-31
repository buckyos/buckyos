use std::collections::HashMap;
use std::net::IpAddr;
use std::net::SocketAddr;
use std::path::PathBuf;

use jsonwebtoken::jwk::Jwk;
use jsonwebtoken::{encode, Algorithm, DecodingKey, EncodingKey, Header};
use log::*;
use rand::seq::SliceRandom;
use serde::{Serialize,Deserialize};
use serde_json::json;
use buckyos_kit::*;
use once_cell::sync::OnceCell;
use crate::get_x_from_jwk;
use crate::DID;
use crate::DeviceInfo;

use crate::{DIDDocumentTrait,EncodedDocument};
use crate::{NSResult,NSError};
use crate::{decode_json_from_jwt_with_pk,decode_jwt_claim_without_verify,decode_json_from_jwt_with_default_pk};

#[derive(Clone, Serialize, Deserialize,Debug,PartialEq)]
pub(crate) struct VerificationMethodNode {
    #[serde(rename = "type")]
    pub key_type: String,
    #[serde(rename = "id")]
    pub key_id: String,
    #[serde(rename = "controller")]
    pub key_controller: String,
    #[serde(rename = "publicKeyJwk")]
    pub public_key: Jwk,
    #[serde(flatten)]
    pub extra_info: HashMap<String, serde_json::Value>,
}

#[derive(Clone, Serialize, Deserialize,Debug,PartialEq)]
pub(crate) struct ServiceNode {
    pub id: String,
    #[serde(rename = "type")]
    pub service_type: String,
    #[serde(rename = "serviceEndpoint")]
    pub service_endpoint: String,
}

fn default_context() -> String {
    "https://www.w3.org/ns/did/v1".to_string()
}

//this config is store at DNS TXT record,and can be used to boot up the zone
#[derive(Clone, Serialize, Deserialize,Debug,PartialEq)]
pub struct ZoneBootConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id:Option<DID>,
    pub oods: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sn:Option<String>,
    pub exp:u64,
    pub nonce:u32,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner:Option<DID>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_key:Option<Jwk>,//PKX=0:xxxxxxx;
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub gateway_devs:Vec<DID>,

    #[serde(flatten)]
    pub extra_info: HashMap<String, serde_json::Value>,
}

impl DIDDocumentTrait for ZoneBootConfig {
    fn get_id(&self) -> DID {
        if self.id.is_some() {
            return self.id.clone().unwrap();
        }
        return DID::undefined();
    }
    
    fn get_auth_key(&self,kid:Option<&str>) -> Option<DecodingKey> {
        if kid.is_none() {
            if self.owner_key.is_none() {
                return None;
            }
            let result_key = DecodingKey::from_jwk(&self.owner_key.as_ref().unwrap());
            if result_key.is_err() {
                error!("Failed to decode owner key: {:?}",result_key.err().unwrap());
                return None;
            }
            return Some(result_key.unwrap());
        }
        return None;
    }

    fn get_iss(&self) -> Option<String> {
        return None;
    }

    fn get_exp(&self) -> Option<u64> {
        return Some(self.exp);
    }

    fn get_iat(&self) -> Option<u64> {
        return None;
    }

    fn encode(&self,key:Option<&EncodingKey>) -> NSResult<EncodedDocument> {
        if key.is_none() {
            return Err(NSError::Failed("No key provided".to_string()));
        }
        let key = key.unwrap();
        let mut header = Header::new(Algorithm::EdDSA);
        header.typ = None; // 默认为 JWT，设置为None以节约空间
        let token = encode(&header, self,key).map_err(|error| {
            NSError::Failed(format!("Failed to encode zone boot config:{}",error))
        })?;
        return Ok(EncodedDocument::Jwt(token));
    }

    fn decode(doc: &EncodedDocument,key:Option<&DecodingKey>) -> NSResult<Self> where Self: Sized {
        match doc {
            EncodedDocument::Jwt(jwt_str) => {
                if key.is_none() {
                    return Err(NSError::Failed("No key provided".to_string()));
                }
                let json_result = decode_json_from_jwt_with_pk(jwt_str,key.unwrap())?;
                let result:ZoneBootConfig = serde_json::from_value(json_result).map_err(|error| {
                    NSError::Failed(format!("Failed to decode zone boot config:{}",error))
                })?;
                return Ok(result);
            },
            EncodedDocument::JsonLd(json_value) => {
                let result:ZoneBootConfig = serde_json::from_value(json_value.clone()).map_err(|error| {
                    NSError::Failed(format!("Failed to decode zone boot config:{}",error))
                })?;
                return Ok(result);
            },
        }
    }

}


#[derive(Clone, Serialize, Deserialize,Debug,PartialEq)]
pub struct VerifyHubInfo {
    pub port:u16,
    pub node_name:String,
    pub public_key:Jwk,
}


#[derive(Clone, Serialize, Deserialize,Debug,PartialEq)]
pub struct ZoneConfig {
    #[serde(rename = "@context",default = "default_context")]
    pub context: String,
    pub id: DID,
    #[serde(rename = "verificationMethod")]
    verification_method: Vec<VerificationMethodNode>,
    authentication: Vec<String>,
    #[serde(rename = "assertionMethod")]
    assertion_method: Vec<String>,
    service:Vec<ServiceNode>,
    pub exp:u64,
    pub iat:u64,
    #[serde(flatten)]
    pub extra_info: HashMap<String, serde_json::Value>,

    //--------------------------------
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<DID>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,//zone short name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_list: Option<Vec<String>>,//device did list
    //ood server endpoints,can be ["ood1","ood2@192.168.32.1","ood3#vlan1]
    pub oods: Vec<String>,    
    //因为所有的Node上的Gateway都是同质的，所以这里可以不用配置？DNS记录解析到哪个Node，哪个Node的Gateway就是ZoneGateway
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sn:Option<String>,//
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verify_hub_info:Option<VerifyHubInfo>,
    pub nonce:u32,

}

impl ZoneConfig {

    pub fn new(id:DID,owner_did:DID,public_key:Jwk) -> Self {
        ZoneConfig {
            context: default_context(),
            id: id,
            verification_method: vec![VerificationMethodNode { 
                key_type: "Ed25519VerificationKey2020".to_string(),
                key_id: "#main_key".to_string(),
                key_controller: owner_did.to_string(),
                public_key: public_key,
                extra_info: HashMap::new(),
            }],
            authentication: vec!["#main_key".to_string()],
            assertion_method: vec!["#main_key".to_string()],
            service: vec![],
            exp: 0,
            iat: 0,
            extra_info: HashMap::new(),
            owner: Some(owner_did),
            name: None,
            device_list: None,
            oods: vec![],
            sn: None,
            verify_hub_info: None,
            nonce: 0,
        }
    }

    pub fn load_zone_config(file_path: &PathBuf) -> NSResult<ZoneConfig> {
        let contents = std::fs::read_to_string(file_path.clone()).map_err(|err| {
            error!("read {} failed! {}", file_path.to_string_lossy(), err);
            return NSError::ReadLocalFileError(format!("read {} failed! {}", file_path.to_string_lossy(), err));
        })?;
        let config: ZoneConfig = serde_json::from_str(&contents).map_err(|err| {
            error!("parse {} failed! {}", file_path.to_string_lossy(), err);
            return NSError::ReadLocalFileError(format!(
                "Failed to parse ZoneConfig json: {}",
                err
            ));
        })?;    
        Ok(config)
    }

    pub fn init_by_boot_config(&mut self,boot_config:&ZoneBootConfig) {
        self.oods = boot_config.oods.clone();
        self.sn = boot_config.sn.clone();
        self.nonce = boot_config.nonce;
        self.extra_info.extend(boot_config.extra_info.clone());
    }

    pub fn get_zone_short_name(&self) -> String {        
        if self.name.is_some() {
            return self.name.clone().unwrap();
        }
        let host_name = self.id.to_host_name();
        let short_name = host_name.split('.').next().unwrap();
        return short_name.to_string();
    }

    pub fn get_node_host_name(&self,node_name:&str) -> String {
        let zone_short_name = self.get_zone_short_name();
        let host_name = format!("{}-{}",zone_short_name,node_name);
        return host_name;
    }

    //ood需要通用这个信息，来与zone内的其它ood建立连接
    pub fn get_ood_desc_string(&self,node_name:&str) -> Option<String> {
        for ood in self.oods.iter() {
            if ood.starts_with(node_name) {
                return Some(ood.clone());
            }
        }
        return None;
    }
    
    pub fn select_same_subnet_ood(&self,device_info:&DeviceInfo) -> Option<String> {
        let mut ood_list = self.oods.clone();
        ood_list.shuffle(&mut rand::thread_rng());

        for ood in ood_list.iter() {
            let (device_name,net_id,ip) = DeviceInfo::get_net_info_from_ood_desc_string(ood);
            if net_id == device_info.net_id {
                return Some(ood.clone());
            }
        }

        return None;
    }

    pub fn select_wan_ood(&self) -> Option<String> {
        let mut ood_list = self.oods.clone();
        ood_list.shuffle(&mut rand::thread_rng());
        for ood in self.oods.iter() {
            let (device_name,net_id,ip) = DeviceInfo::get_net_info_from_ood_desc_string(ood);   
            if net_id.is_some() {
                if net_id.as_ref().unwrap().starts_with("wan") {
                    return Some(ood.clone());
                }
            }
        }
        return None;
    }

    pub fn get_sn_url(&self) -> Option<String> {
        if self.sn.is_some() {
            return Some(format!("https://{}/kapi/sn",self.sn.as_ref().unwrap()));
        }
        return None;
    }

    fn get_default_service_port(&self,service_name: &str) -> Option<u16> {
        if service_name.starts_with("http") {
            return Some(80);
        } else if service_name.starts_with("https") {
            return Some(443);
        }
        return None;
    }   

    pub fn get_default_key(&self) -> Option<Jwk> {
        for method in self.verification_method.iter() {
            if method.key_id == "#main_key" {
                return Some(method.public_key.clone());
            }
        }
        return None;
    } 
}



impl DIDDocumentTrait for ZoneConfig {
    
    fn get_id(&self) -> DID {    
        return self.id.clone();
    }

    fn get_auth_key(&self,kid:Option<&str>) -> Option<DecodingKey> {
        if self.verification_method.is_empty() {
            return None;
        }
        if kid.is_none() {
            let decoding_key = DecodingKey::from_jwk(&self.verification_method[0].public_key);
            if decoding_key.is_err() {
                error!("Failed to decode auth key: {:?}",decoding_key.err().unwrap());
                return None;
            }
            return Some(decoding_key.unwrap());
        }
        let kid = kid.unwrap();
        for method in self.verification_method.iter() {
            if method.key_id == kid {
                let decoding_key = DecodingKey::from_jwk(&method.public_key);
                if decoding_key.is_err() {
                    error!("Failed to decode auth key: {:?}",decoding_key.err().unwrap());
                    return None;
                }
                return Some(decoding_key.unwrap());
            }
        }
        return None;
    }

    fn get_iss(&self) -> Option<String> {
        if self.owner.is_some() {
            return Some(self.owner.as_ref().unwrap().to_string());
        }
        return None;
    }
    fn get_exp(&self) -> Option<u64> {
        return Some(self.exp)
    }

    fn get_iat(&self) -> Option<u64> {
        return Some(self.iat);
    }

    fn encode(&self,key:Option<&EncodingKey>) -> NSResult<EncodedDocument> {
        if key.is_none() {
            return Err(NSError::Failed("No key provided".to_string()));
        }
        let key = key.unwrap();
        let mut header = Header::new(Algorithm::EdDSA);
        header.typ = None; // 默认为 JWT，设置为None以节约空间
        let token = encode(&header, self,key).map_err(|error| {
            NSError::Failed(format!("Failed to encode zone config:{}",error))
        })?;
        return Ok(EncodedDocument::Jwt(token));
    }

    fn decode(doc: &EncodedDocument,key:Option<&DecodingKey>) -> NSResult<Self> where Self: Sized {
        match doc {
            EncodedDocument::Jwt(jwt_str) => {
                let decoded_payload = decode_jwt_claim_without_verify(jwt_str)?;
                let result:ZoneConfig = serde_json::from_value(decoded_payload).map_err(|error| {
                    NSError::Failed(format!("Failed to decode OwnerConfig from json:{}",error))
                })?;
                let default_key = result.get_default_key();
                if default_key.is_none() {
                    return Err(NSError::Failed("No default key found".to_string()));
                }
                let default_key = default_key.unwrap();
                let json_result = decode_json_from_jwt_with_default_pk(jwt_str,&default_key)?;
                return Ok(result);
            },
            _ => {
                return Err(NSError::Failed("Invalid document type".to_string()));
            }
        }
    }
    // async fn decode_with_load_key<'a, F, Fut>(doc: &'a EncodedDocument,loader:F) -> NSResult<Self> 
    //     where Self: Sized,
    //           F: Fn(&'a str) -> Fut,
    //           Fut: std::future::Future<Output = NSResult<DecodingKey>> {
    //     unimplemented!()
    // }
}


pub enum DeviceType {
    OOD, //run system config service
    Server,//run other service
    Sensor,
}


#[derive(Clone, Serialize, Deserialize,Debug,PartialEq)]
pub struct DeviceConfig {
    #[serde(rename = "@context",default = "default_context")]
    pub context: String,
    pub id: DID,
    #[serde(rename = "verificationMethod")]
    verification_method: Vec<VerificationMethodNode>,
    authentication: Vec<String>,
    #[serde(rename = "assertionMethod")]
    assertion_method: Vec<String>,
    service:Vec<ServiceNode>,
    pub exp:u64,
    pub iat:u64,
    #[serde(flatten)]
    pub extra_info: HashMap<String, serde_json::Value>,

    //--------------------------------
    pub device_type: String,//[ood,node,sensor
    pub name: String,//short name,like ood1
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip:Option<IpAddr>,//main_ip
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net_id:Option<String>,// lan1 | wan ，为None时表示为 lan0
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ddns_sn_url:Option<String>,
    #[serde(skip_serializing_if = "is_true", default = "bool_default_true")]
    pub support_container:bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zone_did: Option<DID>,//Device 所在的zone did
    pub iss:String,
}

impl DeviceConfig {
    pub fn new_by_jwk(name:&str,pk:Jwk) -> Self {
        let x = get_x_from_jwk(&pk).unwrap();
        return DeviceConfig::new(name,x);
    }

    pub fn new(name:&str,pkx:String) -> Self {
        let did = format!("did:dev:{}",pkx);
        let jwk = json!(
            {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": pkx
            }
        );

        let public_key_jwk : jsonwebtoken::jwk::Jwk = serde_json::from_value(jwk).unwrap();
        DeviceConfig {
            context: default_context(),
            id: DID::from_str(&did).unwrap(),
            name: name.to_string(),
            arch: None,
            device_type: "node".to_string(),
            ip: None,
            net_id: None,
            ddns_sn_url: None,
            verification_method: vec![VerificationMethodNode {
                key_type: "Ed25519VerificationKey2020".to_string(),
                key_id: "#main_key".to_string(),
                key_controller: did.clone(),
                public_key: public_key_jwk,
                extra_info: HashMap::new(),
            }],
            authentication: vec!["#main_key".to_string()],
            assertion_method: vec!["#main_key".to_string()],
            service: vec![],
            support_container: true,
            zone_did: None,
            iss: "".to_string(),
            exp: buckyos_get_unix_timestamp() + 3600*24*365*10,
            iat: buckyos_get_unix_timestamp() as u64,
            extra_info: HashMap::new(),
        }
    }

    pub fn get_default_key(&self) -> Option<Jwk> {
        for method in self.verification_method.iter() {
            if method.key_id == "#main_key" {
                return Some(method.public_key.clone());
            }
        }
        return None;
    }
}

impl DIDDocumentTrait for DeviceConfig {
    fn get_id(&self) -> DID {
        return self.id.clone();
    }

    fn get_auth_key(&self,kid:Option<&str>) -> Option<DecodingKey> {
        if self.verification_method.is_empty() {
            return None;
        }
        if kid.is_none() {
            let decoding_key = DecodingKey::from_jwk(&self.verification_method[0].public_key);
            if decoding_key.is_err() {
                error!("Failed to decode auth key: {:?}",decoding_key.err().unwrap());
                return None;
            }
            return Some(decoding_key.unwrap());
        }
        let kid = kid.unwrap();
        for method in self.verification_method.iter() {
            if method.key_id == kid {
                let decoding_key = DecodingKey::from_jwk(&method.public_key);
                if decoding_key.is_err() {
                    error!("Failed to decode auth key: {:?}",decoding_key.err().unwrap());
                    return None;
                }
                return Some(decoding_key.unwrap());
            }
        }
        return None;
    }

    fn get_iss(&self) -> Option<String> {
        return Some(self.iss.clone());
    }
    fn get_exp(&self) -> Option<u64> {
        return Some(self.exp)
    }
    fn get_iat(&self) -> Option<u64> {
        return Some(self.iat)
    }

    fn encode(&self,key:Option<&EncodingKey>) -> NSResult<EncodedDocument> {
        if key.is_none() {
            return Err(NSError::Failed("No key provided".to_string()));
        }
        let key = key.unwrap();
        let mut header = Header::new(Algorithm::EdDSA);
        header.typ = None; // 默认为 JWT，设置为None以节约空间
        let token = encode(&header, self,key).map_err(|error| {
            NSError::Failed(format!("Failed to encode OwnerConfig :{}",error))
        })?;
        return Ok(EncodedDocument::Jwt(token));
    }
    fn decode(doc: &EncodedDocument,key:Option<&DecodingKey>) -> NSResult<Self> where Self: Sized {
        match doc {
            EncodedDocument::Jwt(jwt_str) => {
                let decoded_payload = decode_jwt_claim_without_verify(jwt_str)?;
                let result:DeviceConfig = serde_json::from_value(decoded_payload).map_err(|error| {
                    NSError::Failed(format!("Failed to decode OwnerConfig from json:{}",error))
                })?;
                let default_key = result.get_default_key();
                if default_key.is_none() {
                    return Err(NSError::Failed("No default key found".to_string()));
                }
                let default_key = default_key.unwrap();
                let json_result = decode_json_from_jwt_with_default_pk(jwt_str,&default_key)?;
                return Ok(result);
            },
            _ => {
                return Err(NSError::Failed("Invalid document type".to_string()));
            }
        }
    }
    // async fn decode_with_load_key<'a, F, Fut>(doc: &'a EncodedDocument,loader:F) -> NSResult<Self> 
    //     where Self: Sized,
    //           F: Fn(&'a str) -> Fut,
    //           Fut: std::future::Future<Output = NSResult<DecodingKey>> {
    //     unimplemented!()
    // }
}



#[derive(Clone, Serialize, Deserialize,Debug,PartialEq)]
pub struct OwnerConfig {
    #[serde(rename = "@context",default = "default_context")]
    pub context: String,
    pub id: DID,
    #[serde(rename = "verificationMethod")]
    verification_method: Vec<VerificationMethodNode>,
    authentication: Vec<String>,
    #[serde(rename = "assertionMethod")]
    assertion_method: Vec<String>,
    service:Vec<ServiceNode>,
    pub exp:u64,
    pub iat:u64,
    #[serde(flatten)]
    pub extra_info: HashMap<String, serde_json::Value>,

    //--------------------------------
    pub name: String,
    pub full_name : String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_zone_did: Option<DID>,

}

impl OwnerConfig {
    pub fn new(id:DID,name:String,full_name:String,public_key:Jwk) -> Self {
        let verification_method = vec![VerificationMethodNode {
            key_type: "Ed25519VerificationKey2020".to_string(),
            key_id: "#main_key".to_string(),
            key_controller: id.to_string(),
            public_key: public_key, 
            extra_info: HashMap::new(),
        }];

        OwnerConfig {
            context: default_context(),
            id: id,
            name: name,
            full_name: full_name,
            verification_method: verification_method,
            authentication: vec!["#main_key".to_string()],
            assertion_method: vec!["#main_key".to_string()],
            default_zone_did: None,
            exp: buckyos_get_unix_timestamp() + 3600*24*365*10,
            iat: buckyos_get_unix_timestamp(),
            extra_info: HashMap::new(),
            service: vec![],
        }
    }

    pub fn set_default_zone_did(&mut self,default_zone_did:DID) {
        self.default_zone_did = Some(default_zone_did.clone());
        self.service.push(ServiceNode { 
            id: format!("{}#lastDoc",self.id.to_string()),
            service_type: "DIDDoc".to_string(),
            service_endpoint: format!("https://{}/resolve/{}",default_zone_did.to_host_name(),self.id.to_string()),
        });
    }

    pub fn load_owner_config(file_path: &PathBuf) -> NSResult<OwnerConfig> {
        let contents = std::fs::read_to_string(file_path.clone()).map_err(|err| {
            error!("read {} failed! {}", file_path.to_string_lossy(), err);
            return NSError::ReadLocalFileError(format!("read {} failed! {}", file_path.to_string_lossy(), err));
        })?;
        let config: OwnerConfig = serde_json::from_str(&contents).map_err(|err| {
            error!("parse {} failed! {}", file_path.to_string_lossy(), err);
            return NSError::ReadLocalFileError(format!(
                "Failed to parse OwnerConfig json: {}",
                err
            ));
        })?;
        Ok(config)
    }

    pub fn get_default_zone_did(&self) -> Option<DID> {
        return self.default_zone_did.clone();
    }

    pub fn get_default_key(&self) -> Option<Jwk> {
        for method in self.verification_method.iter() {
            if method.key_id == "#main_key" {
                return Some(method.public_key.clone());
            }
        }
        return None;
    }
}

impl DIDDocumentTrait for OwnerConfig {
    fn get_id(&self) -> DID {
        return self.id.clone();
    }
    fn get_auth_key(&self,kid:Option<&str>) -> Option<DecodingKey> {
        if self.verification_method.is_empty() {
            return None;
        }
        if kid.is_none() {
            let decoding_key = DecodingKey::from_jwk(&self.verification_method[0].public_key);
            if decoding_key.is_err() {
                error!("Failed to decode auth key: {:?}",decoding_key.err().unwrap());
                return None;
            }
            return Some(decoding_key.unwrap());
        }
        let kid = kid.unwrap();
        for method in self.verification_method.iter() {
            if method.key_id == kid {
                let decoding_key = DecodingKey::from_jwk(&method.public_key);
                if decoding_key.is_err() {
                    error!("Failed to decode auth key: {:?}",decoding_key.err().unwrap());
                    return None;
                }
                return Some(decoding_key.unwrap());
            }
        }
        return None;
    }

    fn get_iss(&self) -> Option<String> {
        return None;
    }
    fn get_exp(&self) -> Option<u64> {
        return Some(self.exp)
    }
    fn get_iat(&self) -> Option<u64> {
        return Some(self.iat)
    }

    fn encode(&self,key:Option<&EncodingKey>) -> NSResult<EncodedDocument> {
        if key.is_none() {
            return Err(NSError::Failed("No key provided".to_string()));
        }
        let key = key.unwrap();
        let mut header = Header::new(Algorithm::EdDSA);
        header.typ = None; // 默认为 JWT，设置为None以节约空间
        let token = encode(&header, self,key).map_err(|error| {
            NSError::Failed(format!("Failed to encode OwnerConfig :{}",error))
        })?;
        return Ok(EncodedDocument::Jwt(token));
    }

    fn decode(doc: &EncodedDocument,key:Option<&DecodingKey>) -> NSResult<Self> where Self: Sized {
        match doc {
            EncodedDocument::Jwt(jwt_str) => {
                let decoded_payload = decode_jwt_claim_without_verify(jwt_str)?;
                let result:OwnerConfig = serde_json::from_value(decoded_payload).map_err(|error| {
                    NSError::Failed(format!("Failed to decode OwnerConfig from json:{}",error))
                })?;
                let default_key = result.get_default_key();
                if default_key.is_none() {
                    return Err(NSError::Failed("No default key found".to_string()));
                }
                let default_key = default_key.unwrap();
                let json_result = decode_json_from_jwt_with_default_pk(jwt_str,&default_key)?;
                return Ok(result);
            },
            _ => {
                return Err(NSError::Failed("Invalid document type".to_string()));
            }
        }
    }
    // async fn decode_with_load_key<'a, F, Fut>(doc: &'a EncodedDocument,loader:F) -> NSResult<Self> 
    //     where Self: Sized,
    //           F: Fn(&'a str) -> Fut,
    //           Fut: std::future::Future<Output = NSResult<DecodingKey>> {
    //     unimplemented!()
    // }
}

//NodeIdentity from ood active progress
#[derive(Deserialize, Debug)]
pub struct NodeIdentityConfig {
    pub zone_did: DID,// $name.buckyos.org or did:ens:$name
    pub owner_public_key: jsonwebtoken::jwk::Jwk, //owner is zone_owner, must same as zone_config.default_auth_key
    pub owner_did:DID,//owner's did
    pub device_doc_jwt:String,//device document,jwt string,siged by owner
    pub zone_nonce:String,// random string, use to identify the zone
    //device_private_key: ,storage in partical file
}

impl NodeIdentityConfig {
    pub fn load_node_identity_config(file_path: &PathBuf) -> NSResult<(NodeIdentityConfig)> {
        let contents = std::fs::read_to_string(file_path.clone()).map_err(|err| {
            error!("read {} failed! {}", file_path.to_string_lossy(), err);
            return NSError::ReadLocalFileError(format!("read {} failed! {}", file_path.to_string_lossy(), err));
        })?;
    
        let config: NodeIdentityConfig = toml::from_str(&contents).map_err(|err| {
            error!("parse {} failed! {}", file_path.to_string_lossy(), err);
            return NSError::ReadLocalFileError(format!(
                "Failed to parse NodeIdentityConfig TOML: {}",
                err
            ));
        })?;
    
        Ok(config)
    }
}


//unit test
#[cfg(test)]
mod tests {
    use std::{alloc::System, time::{SystemTime, UNIX_EPOCH}};

    use super::*;
    use super::DeviceInfo;
    use serde::de;
    use serde_json::json;
    #[test]
    fn test_zone_config() {
        let private_key_pem = r#"
        -----BEGIN PRIVATE KEY-----
        MC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr
        -----END PRIVATE KEY-----
        "#;
        let jwk = json!(
            {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8"
            }
        );
        let public_key_jwk : jsonwebtoken::jwk::Jwk = serde_json::from_value(jwk).unwrap();
        let private_key: EncodingKey = EncodingKey::from_ed_pem(private_key_pem.as_bytes()).unwrap();
        let public_key = DecodingKey::from_jwk(&public_key_jwk).unwrap();

        let zone_config = ZoneConfig {
            id: DID::new("bns","dev_test"),
            name: None,
            owner: None,
            gateway: None,
            auth_key: None, 
            oods: vec!["ood1".to_string()],
            services: None,
            sn: None,
            vlan: None,
            verify_hub_info: None,
            device_list: None,
            iat:None,
            exp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as u64 + 3600*24*365*10, 
            extra_info: HashMap::new(),
        };

        let json_str = serde_json::to_string(&zone_config).unwrap();
        println!("json_str: {:?}",json_str);

        let encoded = zone_config.encode(Some(&private_key)).unwrap();
        println!("encoded: {:?}",encoded);

        let decoded = ZoneConfig::decode(&encoded,Some(&public_key)).unwrap();
        println!("decoded: {:?}",serde_json::to_string(&decoded).unwrap());
        let token2 = decoded.encode(Some(&private_key)).unwrap();

        assert_eq!(zone_config,decoded);
        assert_eq!(encoded,token2);
    }

    #[tokio::test]
    async fn test_device_config() {
        let owner_private_key_pem = r#"
        -----BEGIN PRIVATE KEY-----
        MC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr
        -----END PRIVATE KEY-----
        "#;
        let owner_jwk = json!(
            {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8"
            }
        );
        let public_key_jwk : jsonwebtoken::jwk::Jwk = serde_json::from_value(owner_jwk).unwrap();
        let owner_private_key: EncodingKey = EncodingKey::from_ed_pem(owner_private_key_pem.as_bytes()).unwrap();
        let public_key = DecodingKey::from_jwk(&public_key_jwk).unwrap();
        
        //ood1 privete key:

        
        let ood_public_key = json!(
            {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": "gubVIszw-u_d5PVTh-oc8CKAhM9C-ne5G_yUK5BDaXc"
            }
        );
        let ood_key_jwk : jsonwebtoken::jwk::Jwk = serde_json::from_value(ood_public_key).unwrap();
        let device_config = DeviceConfig {
            id: DID::new("dev","gubVIszw-u_d5PVTh-oc8CKAhM9C-ne5G_yUK5BDaXc"),
            name: "ood1".to_string(),
            device_type: "ood".to_string(),
            auth_key: ood_key_jwk,
            iss: "lzc".to_string(),
            ip:None,
            net_id:None,
            arch: None,
            ddns_sn_url: None,
            support_container: true,
            exp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as u64 + 3600*24*365*10, 
            iat: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as u64,
            extra_info: HashMap::new(),
        };

        let json_str = serde_json::to_string(&device_config).unwrap();
        println!("ood json_str: {:?}",json_str);

        let encoded = device_config.encode(Some(&owner_private_key)).unwrap();
        println!("ood encoded: {:?}",encoded);

        let decoded = DeviceConfig::decode(&encoded,Some(&public_key)).unwrap();
        println!("ood decoded: {:?}",serde_json::to_string(&decoded).unwrap());
        let token2 = decoded.encode(Some(&owner_private_key)).unwrap();

        let mut device_info_ood = DeviceInfo::from_device_doc(&decoded);
        device_info_ood.auto_fill_by_system_info().await;
        let device_info_str = serde_json::to_string(&device_info_ood).unwrap();
        println!("ood device_info: {}",device_info_str);

        assert_eq!(device_config,decoded);
        assert_eq!(encoded,token2); 

    
        // Public Key (JWK base64URL): 
        //  M3-pAdhs0uFkWmmjdHLBfs494R91QmQeXzCEhEHP-tI
        // Private Key (DER): 
        //-----BEGIN PRIVATE KEY-----
        // MC4CAQAwBQYDK2VwBCIEIGdfBOWv07OemQY4BGe7LYqDOVY+qvwpcbAeI1d1VRBo
        // -----END PRIVATE KEY-----
        let gateway_public_key = json!(
            {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": "M3-pAdhs0uFkWmmjdHLBfs494R91QmQeXzCEhEHP-tI"
            }
        );
        let gateway_key_jwk : jsonwebtoken::jwk::Jwk = serde_json::from_value(gateway_public_key).unwrap();
        let device_config = DeviceConfig {
            id: DID::new("dev","M3-pAdhs0uFkWmmjdHLBfs494R91QmQeXzCEhEHP-tI"),
            name: "gateway".to_string(),
            device_type: "node".to_string(),
            auth_key: gateway_key_jwk,
            iss: "waterfllier".to_string(),
            ip:Some("23.239.23.54".parse().unwrap()),
            net_id:Some("wan".to_string()),
            arch: None,
            ddns_sn_url: None,
            support_container: true,
            exp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as u64 + 3600*24*365*10, 
            iat: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as u64,
            extra_info: HashMap::new(),
        };

        let json_str = serde_json::to_string(&device_config).unwrap();
        println!("gateway json_str: {:?}",json_str);

        let encoded = device_config.encode(Some(&owner_private_key)).unwrap();
        println!("gateway encoded: {:?}",encoded);

        let decoded = DeviceConfig::decode(&encoded,Some(&public_key)).unwrap();
        println!("gateway decoded: {:?}",serde_json::to_string(&decoded).unwrap());
        let token2 = decoded.encode(Some(&owner_private_key)).unwrap();

        assert_eq!(device_config,decoded);
        assert_eq!(encoded,token2); 

        //Public Key (JWK base64URL): LBgzvFCD4VqQxTsO2LCZjs9FPVaQV2Dt0Q5W_lr4mr0
        //Private Key (DER): 
        //-----BEGIN PRIVATE KEY-----
        //MC4CAQAwBQYDK2VwBCIEIHb18syrSj0BELLwDLJKugmj+63JUzDPIay6gZqUaBeM
        //-----END PRIVATE KEY-----
        let server_public_key = json!(
            {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": "LBgzvFCD4VqQxTsO2LCZjs9FPVaQV2Dt0Q5W_lr4mr0"
            }
        );
        let server_key_jwk : jsonwebtoken::jwk::Jwk = serde_json::from_value(server_public_key).unwrap();
        let device_config = DeviceConfig {
            id: DID::new("dev","LBgzvFCD4VqQxTsO2LCZjs9FPVaQV2Dt0Q5W_lr4mr0"),
            name: "server1".to_string(),
            device_type: "node".to_string(),
            auth_key: server_key_jwk,
            iss: "waterfllier".to_string(),
            ip:None,
            net_id:None,
            arch: None,
            ddns_sn_url: None,
            support_container: true,
            exp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as u64 + 3600*24*365*10, 
            iat: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as u64,
            extra_info: HashMap::new(),
        };
        let json_str = serde_json::to_string(&device_config).unwrap();
        println!("server json_str: {:?}",json_str);

        let encoded = device_config.encode(Some(&owner_private_key)).unwrap();
        println!("server encoded: {:?}",encoded);

        let decoded = DeviceConfig::decode(&encoded,Some(&public_key)).unwrap();
        println!("server decoded: {:?}",serde_json::to_string(&decoded).unwrap());
        let token2 = decoded.encode(Some(&owner_private_key)).unwrap();

        assert_eq!(device_config,decoded);
        assert_eq!(encoded,token2); 
    }

    #[test]
    fn test_owner_config() {
        let private_key_pem = r#"
        -----BEGIN PRIVATE KEY-----
        MC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr
        -----END PRIVATE KEY-----
        "#;
        let jwk = json!(
            {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8"
            }
        );
        let public_key_jwk : jsonwebtoken::jwk::Jwk = serde_json::from_value(jwk).unwrap();
        let private_key: EncodingKey = EncodingKey::from_ed_pem(private_key_pem.as_bytes()).unwrap();
        let public_key = DecodingKey::from_jwk(&public_key_jwk).unwrap();

        let mut owner_config = OwnerConfig::new(DID::new("bns","lzc"),
        "lzc".to_string(),"zhicong liu".to_string(),public_key_jwk);

        owner_config.set_default_zone_did(DID::new("bns","waterflier"));
   
        let json_str = serde_json::to_string_pretty(&owner_config).unwrap();
        println!("json_str: {}",json_str.as_str());

        let encoded = owner_config.encode(Some(&private_key)).unwrap();
        println!("encoded: {:?}",encoded);

        let decoded = OwnerConfig::decode(&encoded,None).unwrap();
        println!("decoded: {}",serde_json::to_string_pretty(&decoded).unwrap());
        let token2 = decoded.encode(Some(&private_key)).unwrap();

        assert_eq!(owner_config,decoded);
        assert_eq!(encoded,token2); 
    }
}