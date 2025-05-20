use std::collections::HashMap;
use std::net::IpAddr;
use std::net::SocketAddr;
use std::path::PathBuf;

use crate::get_x_from_jwk;
use crate::DeviceInfo;
use crate::DID;
use buckyos_kit::*;
use jsonwebtoken::jwk::Jwk;
use jsonwebtoken::{encode, Algorithm, DecodingKey, EncodingKey, Header};
use log::*;
use once_cell::sync::OnceCell;
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    decode_json_from_jwt_with_default_pk, decode_json_from_jwt_with_pk,
    decode_jwt_claim_without_verify,
};
use crate::{DIDDocumentTrait, EncodedDocument};
use crate::{NSError, NSResult};

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
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

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
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
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct ZoneBootConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<DID>,
    pub oods: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sn: Option<String>,
    pub exp: u64,
    pub iat: u32,

    //---------------------------------
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<DID>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_key: Option<Jwk>, //PKX=0:xxxxxxx;
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    pub gateway_devs: Vec<DID>,

    #[serde(flatten)]
    pub extra_info: HashMap<String, serde_json::Value>,
}

impl ZoneBootConfig {
    pub fn to_zone_config(&self) -> ZoneConfig {
        let mut result = ZoneConfig::new(
            self.id.clone().unwrap(),
            self.owner.clone().unwrap(),
            self.owner_key.clone().unwrap(),
        );
        result.init_by_boot_config(self);
        return result;
    }
}

impl DIDDocumentTrait for ZoneBootConfig {
    fn get_id(&self) -> DID {
        if self.id.is_some() {
            return self.id.clone().unwrap();
        }
        return DID::undefined();
    }

    fn get_auth_key(&self, kid: Option<&str>) -> Option<(DecodingKey, Jwk)> {
        if kid.is_none() {
            if self.owner_key.is_none() {
                return None;
            }
            let owner_key = self.owner_key.as_ref().unwrap().clone();
            let result_key = DecodingKey::from_jwk(&owner_key);
            if result_key.is_err() {
                error!(
                    "Failed to decode owner key: {:?}",
                    result_key.err().unwrap()
                );
                return None;
            }
            return Some((result_key.unwrap(), owner_key));
        }
        return None;
    }

    fn get_exchange_key(&self, kid: Option<&str>) -> Option<(DecodingKey, Jwk)> {
        if self.gateway_devs.is_empty() {
            return None;
        }
        let did = self.gateway_devs[0].clone();
        let key = did.get_auth_key();
        if key.is_none() {
            return None;
        }
        return Some(key.unwrap());
    }

    fn get_iss(&self) -> Option<String> {
        return None;
    }

    fn get_exp(&self) -> Option<u64> {
        return Some(self.exp);
    }

    fn get_iat(&self) -> Option<u64> {
        return Some(self.iat as u64);
    }

    fn encode(&self, key: Option<&EncodingKey>) -> NSResult<EncodedDocument> {
        if key.is_none() {
            return Err(NSError::Failed("No key provided".to_string()));
        }
        let key = key.unwrap();
        let mut header = Header::new(Algorithm::EdDSA);
        header.typ = None; // 默认为 JWT，设置为None以节约空间
        let token = encode(&header, self, key).map_err(|error| {
            NSError::Failed(format!("Failed to encode zone boot config:{}", error))
        })?;
        return Ok(EncodedDocument::Jwt(token));
    }

    fn decode(doc: &EncodedDocument, key: Option<&DecodingKey>) -> NSResult<Self>
    where
        Self: Sized,
    {
        match doc {
            EncodedDocument::Jwt(jwt_str) => {
                let json_result: serde_json::Value;
                if key.is_none() {
                    json_result = decode_jwt_claim_without_verify(jwt_str)?;
                } else {
                    json_result = decode_json_from_jwt_with_pk(jwt_str, key.unwrap())?;
                }
                let result: ZoneBootConfig =
                    serde_json::from_value(json_result).map_err(|error| {
                        NSError::Failed(format!("Failed to decode device config:{}", error))
                    })?;
                return Ok(result);
            }
            EncodedDocument::JsonLd(json_value) => {
                let result: ZoneBootConfig =
                    serde_json::from_value(json_value.clone()).map_err(|error| {
                        NSError::Failed(format!("Failed to decode zone boot config:{}", error))
                    })?;
                return Ok(result);
            }
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct VerifyHubInfo {
    pub port: u16,
    pub node_name: String,
    pub public_key: Jwk,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct ZoneConfig {
    #[serde(rename = "@context", default = "default_context")]
    pub context: String,
    pub id: DID,
    #[serde(rename = "verificationMethod")]
    verification_method: Vec<VerificationMethodNode>,
    authentication: Vec<String>,
    #[serde(rename = "assertionMethod")]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    assertion_method: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    service: Vec<ServiceNode>,
    pub exp: u64,
    pub iat: u64,
    #[serde(flatten)]
    pub extra_info: HashMap<String, serde_json::Value>,

    //--------------------------------
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<DID>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>, //zone short name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_list: Option<HashMap<String, DID>>, //gateway device did list
    //ood server endpoints,can be ["ood1","ood2@192.168.32.1","ood3#vlan1]
    pub oods: Vec<String>,
    //因为所有的Node上的Gateway都是同质的，所以这里可以不用配置？DNS记录解析到哪个Node，哪个Node的Gateway就是ZoneGateway
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sn: Option<String>, //
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docker_repo_base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verify_hub_info: Option<VerifyHubInfo>,
}

impl ZoneConfig {
    pub fn new(id: DID, owner_did: DID, public_key: Jwk) -> Self {
        let id2 = id.clone();
        ZoneConfig {
            context: default_context(),
            id: id2,
            verification_method: vec![VerificationMethodNode {
                key_type: "Ed25519VerificationKey2020".to_string(),
                key_id: "#main_key".to_string(),
                key_controller: owner_did.to_string(),
                public_key: public_key,
                extra_info: HashMap::new(),
            }],
            authentication: vec!["#main_key".to_string()],
            assertion_method: vec!["#main_key".to_string()],
            service: vec![ServiceNode {
                id: format!("{}#lastDoc", id.to_string()),
                service_type: "DIDDoc".to_string(),
                service_endpoint: format!("https://{}/resolve/this_zone", id.to_host_name()),
            }],
            exp: buckyos_get_unix_timestamp() + 3600 * 24 * 365 * 10,
            iat: buckyos_get_unix_timestamp(),
            extra_info: HashMap::new(),
            owner: Some(owner_did),
            name: None,
            device_list: None,
            oods: vec![],
            sn: None,
            docker_repo_base_url: None,
            verify_hub_info: None,
        }
    }

    pub fn load_zone_config(file_path: &PathBuf) -> NSResult<ZoneConfig> {
        let contents = std::fs::read_to_string(file_path.clone()).map_err(|err| {
            error!("read {} failed! {}", file_path.to_string_lossy(), err);
            return NSError::ReadLocalFileError(format!(
                "read {} failed! {}",
                file_path.to_string_lossy(),
                err
            ));
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

    pub fn init_by_boot_config(&mut self, boot_config: &ZoneBootConfig) {
        self.oods = boot_config.oods.clone();
        self.sn = boot_config.sn.clone();
        self.exp = boot_config.exp;
        self.iat = boot_config.iat as u64;

        if boot_config.owner.is_some() {
            self.owner = Some(boot_config.owner.clone().unwrap());
        }
        if boot_config.owner_key.is_some() {
            self.verification_method[0].public_key = boot_config.owner_key.clone().unwrap();
        }
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

    pub fn get_node_host_name(&self, node_name: &str) -> String {
        let zone_short_name = self.get_zone_short_name();
        let host_name = format!("{}-{}", zone_short_name, node_name);
        return host_name;
    }

    //ood需要通用这个信息，来与zone内的其它ood建立连接
    pub fn get_ood_desc_string(&self, node_name: &str) -> Option<String> {
        for ood in self.oods.iter() {
            if ood.starts_with(node_name) {
                return Some(ood.clone());
            }
        }
        return None;
    }

    pub fn select_same_subnet_ood(&self, device_info: &DeviceInfo) -> Option<String> {
        let mut ood_list = self.oods.clone();
        ood_list.shuffle(&mut rand::thread_rng());

        for ood in ood_list.iter() {
            let (device_name, net_id, ip) = DeviceInfo::get_net_info_from_ood_desc_string(ood);
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
            let (device_name, net_id, ip) = DeviceInfo::get_net_info_from_ood_desc_string(ood);
            if net_id.is_some() {
                if net_id.as_ref().unwrap().starts_with("wan") {
                    return Some(ood.clone());
                }
            }
        }
        return None;
    }

    pub fn get_sn_api_url(&self) -> Option<String> {
        if self.sn.is_some() {
            return Some(format!("https://{}/kapi/sn", self.sn.as_ref().unwrap()));
        }
        return None;
    }

    fn get_default_service_port(&self, service_name: &str) -> Option<u16> {
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

    fn get_auth_key(&self, kid: Option<&str>) -> Option<(DecodingKey, Jwk)> {
        if self.verification_method.is_empty() {
            return None;
        }
        if kid.is_none() {
            let decoding_key = DecodingKey::from_jwk(&self.verification_method[0].public_key);
            if decoding_key.is_err() {
                error!(
                    "Failed to decode auth key: {:?}",
                    decoding_key.err().unwrap()
                );
                return None;
            }
            return Some((
                decoding_key.unwrap(),
                self.verification_method[0].public_key.clone(),
            ));
        }
        let kid = kid.unwrap();
        for method in self.verification_method.iter() {
            if method.key_id == kid {
                let decoding_key = DecodingKey::from_jwk(&method.public_key);
                if decoding_key.is_err() {
                    error!(
                        "Failed to decode auth key: {:?}",
                        decoding_key.err().unwrap()
                    );
                    return None;
                }
                return Some((decoding_key.unwrap(), method.public_key.clone()));
            }
        }
        return None;
    }

    fn get_exchange_key(&self, kid: Option<&str>) -> Option<(DecodingKey, Jwk)> {
        if self.device_list.is_some() {
            let device_list = self.device_list.as_ref().unwrap();
            let did = device_list.get("gateway");
            if did.is_some() {
                let did = did.unwrap();
                let key = did.get_auth_key();
                return key;
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
        return Some(self.exp);
    }

    fn get_iat(&self) -> Option<u64> {
        return Some(self.iat);
    }

    fn encode(&self, key: Option<&EncodingKey>) -> NSResult<EncodedDocument> {
        if key.is_none() {
            return Err(NSError::Failed("No key provided".to_string()));
        }
        let key = key.unwrap();
        let mut header = Header::new(Algorithm::EdDSA);
        header.typ = None; // 默认为 JWT，设置为None以节约空间
        let token = encode(&header, self, key)
            .map_err(|error| NSError::Failed(format!("Failed to encode zone config:{}", error)))?;
        return Ok(EncodedDocument::Jwt(token));
    }

    fn decode(doc: &EncodedDocument, key: Option<&DecodingKey>) -> NSResult<Self>
    where
        Self: Sized,
    {
        match doc {
            EncodedDocument::Jwt(jwt_str) => {
                let json_result: serde_json::Value;
                if key.is_none() {
                    json_result = decode_jwt_claim_without_verify(jwt_str)?;
                } else {
                    json_result = decode_json_from_jwt_with_pk(jwt_str, key.unwrap())?;
                }
                let result: ZoneConfig = serde_json::from_value(json_result).map_err(|error| {
                    NSError::Failed(format!("Failed to decode zone config:{}", error))
                })?;
                return Ok(result);
            }
            EncodedDocument::JsonLd(json_value) => {
                let result: ZoneConfig =
                    serde_json::from_value(json_value.clone()).map_err(|error| {
                        NSError::Failed(format!("Failed to decode zone config:{}", error))
                    })?;
                return Ok(result);
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
    OOD,    //run system config service
    Node,   //run other service
    Device, //client device
    Sensor,
    Browser,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct DeviceConfig {
    #[serde(rename = "@context", default = "default_context")]
    pub context: String,
    pub id: DID,
    #[serde(rename = "verificationMethod")]
    verification_method: Vec<VerificationMethodNode>,
    authentication: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    assertion_method: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    service: Vec<ServiceNode>,
    pub exp: u64,
    pub iat: u64,
    #[serde(flatten)]
    pub extra_info: HashMap<String, serde_json::Value>,

    //--------------------------------
    pub device_type: String, //[ood,node,sensor
    pub name: String,        //short name,like ood1

    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<IpAddr>, //main_ip
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net_id: Option<String>, // lan1 | wan ，为None时表示为 lan0
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ddns_sn_url: Option<String>,
    #[serde(skip_serializing_if = "is_true", default = "bool_default_true")]
    pub support_container: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zone_did: Option<DID>, //Device 所在的zone did
    pub iss: String,
}

impl DeviceConfig {
    pub fn new_by_jwk(name: &str, pk: Jwk) -> Self {
        let x = get_x_from_jwk(&pk).unwrap();
        return DeviceConfig::new(name, x);
    }

    pub fn new(name: &str, pkx: String) -> Self {
        let did = format!("did:dev:{}", pkx);
        let jwk = json!(
            {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": pkx
            }
        );

        let public_key_jwk: jsonwebtoken::jwk::Jwk = serde_json::from_value(jwk).unwrap();
        DeviceConfig {
            context: default_context(),
            id: DID::from_str(&did).unwrap(),
            name: name.to_string(),
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
            exp: buckyos_get_unix_timestamp() + 3600 * 24 * 365 * 10,
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

    pub fn set_zone_did(&mut self, zone_did: DID) {
        self.zone_did = Some(zone_did.clone());
        self.service.push(ServiceNode {
            id: format!("{}#lastDoc", self.id.to_string()),
            service_type: "DIDDoc".to_string(),
            service_endpoint: format!(
                "https://{}/resolve/{}",
                zone_did.to_host_name(),
                self.id.to_string()
            ),
        });
    }
}

impl DIDDocumentTrait for DeviceConfig {
    fn get_id(&self) -> DID {
        return self.id.clone();
    }

    fn get_auth_key(&self, kid: Option<&str>) -> Option<(DecodingKey, Jwk)> {
        if self.verification_method.is_empty() {
            return None;
        }
        if kid.is_none() {
            let decoding_key = DecodingKey::from_jwk(&self.verification_method[0].public_key);
            if decoding_key.is_err() {
                error!(
                    "Failed to decode auth key: {:?}",
                    decoding_key.err().unwrap()
                );
                return None;
            }
            return Some((
                decoding_key.unwrap(),
                self.verification_method[0].public_key.clone(),
            ));
        }
        let kid = kid.unwrap();
        for method in self.verification_method.iter() {
            if method.key_id == kid {
                let decoding_key = DecodingKey::from_jwk(&method.public_key);
                if decoding_key.is_err() {
                    error!(
                        "Failed to decode auth key: {:?}",
                        decoding_key.err().unwrap()
                    );
                    return None;
                }
                return Some((decoding_key.unwrap(), method.public_key.clone()));
            }
        }
        return None;
    }

    fn get_exchange_key(&self, kid: Option<&str>) -> Option<(DecodingKey, Jwk)> {
        return self.get_auth_key(kid);
    }

    fn get_iss(&self) -> Option<String> {
        return Some(self.iss.clone());
    }

    fn get_exp(&self) -> Option<u64> {
        return Some(self.exp);
    }

    fn get_iat(&self) -> Option<u64> {
        return Some(self.iat);
    }

    fn encode(&self, key: Option<&EncodingKey>) -> NSResult<EncodedDocument> {
        if key.is_none() {
            return Err(NSError::Failed("No key provided".to_string()));
        }
        let key = key.unwrap();
        let mut header = Header::new(Algorithm::EdDSA);
        header.typ = None; // 默认为 JWT，设置为None以节约空间
        let token = encode(&header, self, key)
            .map_err(|error| NSError::Failed(format!("Failed to encode OwnerConfig :{}", error)))?;
        return Ok(EncodedDocument::Jwt(token));
    }
    fn decode(doc: &EncodedDocument, key: Option<&DecodingKey>) -> NSResult<Self>
    where
        Self: Sized,
    {
        match doc {
            EncodedDocument::Jwt(jwt_str) => {
                let json_result: serde_json::Value;
                if key.is_none() {
                    json_result = decode_jwt_claim_without_verify(jwt_str)?;
                } else {
                    json_result = decode_json_from_jwt_with_pk(jwt_str, key.unwrap())?;
                }
                let result: DeviceConfig =
                    serde_json::from_value(json_result).map_err(|error| {
                        NSError::Failed(format!("Failed to decode device config:{}", error))
                    })?;
                return Ok(result);
            }
            EncodedDocument::JsonLd(json_value) => {
                let result: DeviceConfig =
                    serde_json::from_value(json_value.clone()).map_err(|error| {
                        NSError::Failed(format!("Failed to decode device config:{}", error))
                    })?;
                return Ok(result);
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

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct OwnerConfig {
    #[serde(rename = "@context", default = "default_context")]
    pub context: String,
    pub id: DID,
    #[serde(rename = "verificationMethod")]
    verification_method: Vec<VerificationMethodNode>,
    authentication: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    assertion_method: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    service: Vec<ServiceNode>,
    pub exp: u64,
    pub iat: u64,
    #[serde(flatten)]
    pub extra_info: HashMap<String, serde_json::Value>,

    //--------------------------------
    pub name: String,
    pub full_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_zone_did: Option<DID>,
}

impl OwnerConfig {
    pub fn new(id: DID, name: String, full_name: String, public_key: Jwk) -> Self {
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
            exp: buckyos_get_unix_timestamp() + 3600 * 24 * 365 * 10,
            iat: buckyos_get_unix_timestamp(),
            extra_info: HashMap::new(),
            service: vec![],
        }
    }

    pub fn set_default_zone_did(&mut self, default_zone_did: DID) {
        self.default_zone_did = Some(default_zone_did.clone());
        self.service.push(ServiceNode {
            id: format!("{}#lastDoc", self.id.to_string()),
            service_type: "DIDDoc".to_string(),
            service_endpoint: format!(
                "https://{}/resolve/{}",
                default_zone_did.to_host_name(),
                self.id.to_string()
            ),
        });
    }

    pub fn load_owner_config(file_path: &PathBuf) -> NSResult<OwnerConfig> {
        let contents = std::fs::read_to_string(file_path.clone()).map_err(|err| {
            error!("read {} failed! {}", file_path.to_string_lossy(), err);
            return NSError::ReadLocalFileError(format!(
                "read {} failed! {}",
                file_path.to_string_lossy(),
                err
            ));
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
    fn get_auth_key(&self, kid: Option<&str>) -> Option<(DecodingKey, Jwk)> {
        if self.verification_method.is_empty() {
            return None;
        }
        if kid.is_none() {
            let decoding_key = DecodingKey::from_jwk(&self.verification_method[0].public_key);
            if decoding_key.is_err() {
                error!(
                    "Failed to decode auth key: {:?}",
                    decoding_key.err().unwrap()
                );
                return None;
            }
            return Some((
                decoding_key.unwrap(),
                self.verification_method[0].public_key.clone(),
            ));
        }
        let kid = kid.unwrap();
        for method in self.verification_method.iter() {
            if method.key_id == kid {
                let decoding_key = DecodingKey::from_jwk(&method.public_key);
                if decoding_key.is_err() {
                    error!(
                        "Failed to decode auth key: {:?}",
                        decoding_key.err().unwrap()
                    );
                    return None;
                }
                return Some((decoding_key.unwrap(), method.public_key.clone()));
            }
        }
        return None;
    }

    fn get_exchange_key(&self, kid: Option<&str>) -> Option<(DecodingKey, Jwk)> {
        //return default zone's exchange key
        return None;
    }

    fn get_iss(&self) -> Option<String> {
        return None;
    }
    fn get_exp(&self) -> Option<u64> {
        return Some(self.exp);
    }
    fn get_iat(&self) -> Option<u64> {
        return Some(self.iat);
    }

    fn encode(&self, key: Option<&EncodingKey>) -> NSResult<EncodedDocument> {
        if key.is_none() {
            return Err(NSError::Failed("No key provided".to_string()));
        }
        let key = key.unwrap();
        let mut header = Header::new(Algorithm::EdDSA);
        header.typ = None; // 默认为 JWT，设置为None以节约空间
        let token = encode(&header, self, key)
            .map_err(|error| NSError::Failed(format!("Failed to encode OwnerConfig :{}", error)))?;
        return Ok(EncodedDocument::Jwt(token));
    }

    fn decode(doc: &EncodedDocument, key: Option<&DecodingKey>) -> NSResult<Self>
    where
        Self: Sized,
    {
        match doc {
            EncodedDocument::Jwt(jwt_str) => {
                let json_result: serde_json::Value;
                if key.is_none() {
                    json_result = decode_jwt_claim_without_verify(jwt_str)?;
                } else {
                    json_result = decode_json_from_jwt_with_pk(jwt_str, key.unwrap())?;
                }
                let result: OwnerConfig = serde_json::from_value(json_result).map_err(|error| {
                    NSError::Failed(format!("Failed to decode owner config:{}", error))
                })?;
                return Ok(result);
            }
            EncodedDocument::JsonLd(json_value) => {
                let result: OwnerConfig =
                    serde_json::from_value(json_value.clone()).map_err(|error| {
                        NSError::Failed(format!("Failed to decode owner config:{}", error))
                    })?;
                return Ok(result);
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
#[derive(Deserialize, Debug, Serialize)]
pub struct NodeIdentityConfig {
    pub zone_did: DID,                            // $name.buckyos.org or did:ens:$name
    pub owner_public_key: jsonwebtoken::jwk::Jwk, //owner is zone_owner, must same as zone_config.default_auth_key
    pub owner_did: DID,                           //owner's did
    pub device_doc_jwt: String,                   //device document,jwt string,siged by owner
    pub zone_iat: u32,
    //device_private_key: ,storage in partical file
}

impl NodeIdentityConfig {
    pub fn load_node_identity_config(file_path: &PathBuf) -> NSResult<(NodeIdentityConfig)> {
        let contents = std::fs::read_to_string(file_path.clone()).map_err(|err| {
            error!("read {} failed! {}", file_path.to_string_lossy(), err);
            return NSError::ReadLocalFileError(format!(
                "read {} failed! {}",
                file_path.to_string_lossy(),
                err
            ));
        })?;

        let config: NodeIdentityConfig = serde_json::from_str(&contents).map_err(|err| {
            error!("parse {} failed! {}", file_path.to_string_lossy(), err);
            return NSError::ReadLocalFileError(format!(
                "Failed to parse NodeIdentityConfig JSON: {}",
                err
            ));
        })?;

        Ok(config)
    }
}

//unit test
#[cfg(test)]
mod tests {
    use super::DeviceInfo;
    use super::*;
    use super::super::*;
    use cyfs_sn::*;
    
    use serde::de;
    use serde_json::json;
    use std::{
        alloc::System, hash::Hash, time::{SystemTime, UNIX_EPOCH}
    };

    #[tokio::test]
    async fn test_all_dev_env_configs() {
        let tmp_dir = std::env::temp_dir().join(".buckycli");
        std::fs::create_dir_all(tmp_dir.clone()).unwrap();
        println!(
            "# all BuckyOS dev test config files will be saved in: {:?}",
            tmp_dir
        );
        //本测试会在tmp目录下构造开环境所有的测试文件，并在控制台输出用于写入DNS的记录信息。
        let now = 1743478939; //2025-04-01
        let exp = now + 3600 * 24 * 365 * 10; //2035-04-01
        let owner_private_key_pem = r#"
-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr
-----END PRIVATE KEY-----
        "#;
        let user_key_path = tmp_dir.join("user_private_key.pem");
        std::fs::write(user_key_path.clone(), owner_private_key_pem).unwrap();
        println!(
            "# user private key write to file: {}",
            user_key_path.to_string_lossy()
        );
        let owner_jwk = json!(
            {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8"
            }
        );
        let owner_jwk: jsonwebtoken::jwk::Jwk = serde_json::from_value(owner_jwk.clone()).unwrap();
        let owner_private_key: EncodingKey =
            EncodingKey::from_ed_pem(owner_private_key_pem.as_bytes()).unwrap();

        let mut owner_config = OwnerConfig::new(
            DID::new("bns", "devtest"),
            "devtest".to_string(),
            "zhicong liu".to_string(),
            owner_jwk.clone(),
        );
        let owner_config_json_str = serde_json::to_string_pretty(&owner_config).unwrap();
        let owner_config_path = tmp_dir.join("user_config.json");
        std::fs::write(owner_config_path.clone(), owner_config_json_str.clone()).unwrap();
        println!("owner config: {}", owner_config_json_str);
        println!(
            "# owner config write to file: {}",
            owner_config_path.to_string_lossy()
        );

        let device_private_key_pem = r#"
-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIMDp9endjUnT2o4ImedpgvhVFyZEunZqG+ca0mka8oRp
-----END PRIVATE KEY-----
        "#;
        let private_key_path = tmp_dir.join("node_private_key.pem");
        std::fs::write(private_key_path.clone(), device_private_key_pem).unwrap();
        println!(
            "# device ood1 private key write to file: {}",
            private_key_path.to_string_lossy()
        );
        let ood1_jwk = json!(
            {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": "gubVIszw-u_d5PVTh-oc8CKAhM9C-ne5G_yUK5BDaXc"
              }
        );
        let ood1_jwk: jsonwebtoken::jwk::Jwk = serde_json::from_value(ood1_jwk.clone()).unwrap();
        let mut ood1_device_config = DeviceConfig::new_by_jwk("ood1", ood1_jwk.clone());

        ood1_device_config.support_container = false;
        #[cfg(all(target_os = "linux"))]
        {
            ood1_device_config.support_container = true;
        }

        ood1_device_config.iss = "did:bns:devtest".to_string();
        let ood1_device_config_json_str =
            serde_json::to_string_pretty(&ood1_device_config).unwrap();
        println!("ood1 device config: {}", ood1_device_config_json_str);
        let device_jwt = ood1_device_config.encode(Some(&owner_private_key)).unwrap();
        println!("ood1 device jwt: {}", device_jwt.to_string());
        let deocode_key = DecodingKey::from_jwk(&owner_jwk).unwrap();
        let decode_ood_config = DeviceConfig::decode(&device_jwt, Some(&deocode_key)).unwrap();
        assert_eq!(ood1_device_config, decode_ood_config);

        let encode_key = EncodingKey::from_ed_pem(device_private_key_pem.as_bytes()).unwrap();
        let decode_key = DecodingKey::from_jwk(&ood1_jwk).unwrap();
        let ood_jwt2 = ood1_device_config.encode(Some(&encode_key)).unwrap();
        let decode_ood_config = DeviceConfig::decode(&ood_jwt2, Some(&decode_key)).unwrap();
        assert_eq!(ood1_device_config, decode_ood_config);

        let zone_boot_config = ZoneBootConfig {
            id: None,
            oods: vec!["ood1".to_string()],
            sn: None,
            exp: exp,
            iat: now as u32,
            owner: None,
            owner_key: None,
            gateway_devs: vec![],
            extra_info: HashMap::new(),
        };
        let zone_boot_config_json_str = serde_json::to_string_pretty(&zone_boot_config).unwrap();
        println!("zone boot config: {}", zone_boot_config_json_str.as_str());

        let zone_boot_config_path = tmp_dir.join(format!(
            "{}.zone.json",
            DID::new("web", "test.buckyos.io").to_host_name()
        ));
        std::fs::write(
            zone_boot_config_path.clone(),
            zone_boot_config_json_str.clone(),
        )
        .unwrap();
        println!(
            "# zone boot config write to file: {}",
            zone_boot_config_path.to_string_lossy()
        );
        let zone_boot_config_jwt = zone_boot_config.encode(Some(&owner_private_key)).unwrap();

        let mut zone_config = ZoneConfig::new(
            DID::new("web", "test.buckyos.io"),
            DID::new("bns", "devtest"),
            owner_jwk.clone(),
        );
        zone_config.init_by_boot_config(&zone_boot_config);
        let zone_config_json_str = serde_json::to_string_pretty(&zone_config).unwrap();
        println!("zone config: {}", zone_config_json_str.as_str());
        let zone_config_path = tmp_dir.join("zone_config.json");
        std::fs::write(zone_config_path.clone(), zone_config_json_str.clone()).unwrap();
        println!(
            " zone config write to file: {}",
            zone_config_path.to_string_lossy()
        );
        println!(
            "# zone config generated by zone boot config will store at {}",
            zone_config_path.to_string_lossy()
        );

        let node_identity_config = NodeIdentityConfig {
            zone_did: DID::new("web", "test.buckyos.io"),
            owner_public_key: owner_jwk.clone(),
            owner_did: DID::new("bns", "devtest"),
            device_doc_jwt: device_jwt.to_string(),
            zone_iat: now as u32,
        };
        let node_identity_config_json_str =
            serde_json::to_string_pretty(&node_identity_config).unwrap();
        println!(
            "node identity config: {}",
            node_identity_config_json_str.as_str()
        );
        let node_identity_config_path = tmp_dir.join("node_identity.json");
        std::fs::write(
            node_identity_config_path.clone(),
            node_identity_config_json_str.clone(),
        )
        .unwrap();
        println!(
            "# node identity config will store at {}",
            node_identity_config_path.to_string_lossy()
        );

        //build start_config.json
        let start_config = json!(
            {
                "admin_password_hash":"o8XyToejrbCYou84h/VkF4Tht0BeQQbuX3XKG+8+GQ4=",//bucky2025
                "device_private_key":device_private_key_pem,
                "device_public_key":ood1_jwk,
                "friend_passcode":"sdfsdfsdf",
                "gateway_type":"PortForward",
                "guest_access":true,
                "private_key":owner_private_key_pem,
                "public_key":owner_jwk,
                "user_name":"devtest",
                "zone_name":"test.buckyos.io",
                "BUCKYOS_ROOT":"/opt/buckyos"
            }
        );
        let start_config_json_str = serde_json::to_string_pretty(&start_config).unwrap();
        println!("start config: {}", start_config_json_str.as_str());
        let start_config_path = tmp_dir.join("start_config.json");
        std::fs::write(start_config_path.clone(), start_config_json_str.clone()).unwrap();
        println!(
            "# start_config will store at {}",
            start_config_path.to_string_lossy()
        );

        println!(
            "# test.buckyos.io TXT Record: DID={};",
            zone_boot_config_jwt.to_string()
        );
        let owner_x = get_x_from_jwk(&owner_jwk).unwrap();
        let ood_x = get_x_from_jwk(&ood1_jwk).unwrap();
        println!(
            "# test.buckyos.io TXT Record: PKX=0:{};",
            owner_x.to_string()
        );
        println!("# test.buckyos.io TXT Record: PKX=1:{};", ood_x.to_string());
    }

    async fn create_test_zone_config(
        user_did: DID,
        username: &str,
        owner_private_key_pem: &str,
        owner_jwk: serde_json::Value,
        zone_did: DID,
        sn_host: Option<String>,
    ) -> String {
        let tmp_dir = std::env::temp_dir()
            .join("buckyos_dev_configs")
            .join(username.to_string());
        std::fs::create_dir_all(tmp_dir.clone()).unwrap();
        println!(
            "# all BuckyOS dev test config files will be saved in: {:?}",
            tmp_dir
        );
        //本测试会在tmp目录下构造开环境所有的测试文件，并在控制台输出用于写入DNS的记录信息。
        let now = 1743478939; //2025-04-01
        let exp = now + 3600 * 24 * 365 * 10; //2035-04-01

        let user_key_path = tmp_dir.join("user_private_key.pem");
        std::fs::write(user_key_path.clone(), owner_private_key_pem).unwrap();
        println!(
            "# user private key write to file: {}",
            user_key_path.to_string_lossy()
        );

        let owner_jwk: jsonwebtoken::jwk::Jwk = serde_json::from_value(owner_jwk.clone()).unwrap();
        let owner_private_key: EncodingKey =
            EncodingKey::from_ed_pem(owner_private_key_pem.as_bytes()).unwrap();

        let mut owner_config = OwnerConfig::new(
            user_did.clone(),
            username.to_string(),
            username.to_string(),
            owner_jwk.clone(),
        );
        let owner_config_json_str = serde_json::to_string_pretty(&owner_config).unwrap();
        let owner_config_path = tmp_dir.join("user_config.json");
        std::fs::write(owner_config_path.clone(), owner_config_json_str.clone()).unwrap();
        println!("{}'s owner config: {}", username, owner_config_json_str);
        println!(
            "# owner config write to file: {}",
            owner_config_path.to_string_lossy()
        );

        let zone_boot_config = ZoneBootConfig {
            id: None,
            oods: vec!["ood1".to_string()],
            sn: sn_host,
            exp: exp,
            iat: now as u32,
            owner: None,
            owner_key: None,
            gateway_devs: vec![],
            extra_info: HashMap::new(),
        };
        let zone_boot_config_json_str = serde_json::to_string_pretty(&zone_boot_config).unwrap();
        println!("zone boot config: {}", zone_boot_config_json_str.as_str());

        let zone_boot_config_path = tmp_dir.join(format!("{}.zone.json", zone_did.to_host_name()));
        std::fs::write(
            zone_boot_config_path.clone(),
            zone_boot_config_json_str.clone(),
        )
        .unwrap();
        println!(
            "# zone boot config write to file: {}",
            zone_boot_config_path.to_string_lossy()
        );
        let zone_boot_config_jwt = zone_boot_config.encode(Some(&owner_private_key)).unwrap();

        let zone_host_name = zone_did.to_host_name();
        println!(
            "# {} TXT Record: DID={};",
            zone_host_name,
            zone_boot_config_jwt.to_string()
        );
        let owner_x = get_x_from_jwk(&owner_jwk).unwrap();
        //let ood_x = get_x_from_jwk(&ood1_jwk).unwrap();
        println!(
            "# {} TXT Record: PKX=0:{};",
            zone_host_name,
            owner_x.to_string()
        );
        return zone_boot_config_jwt.to_string();
        //println!("# {} TXT Record: PKX=1:{};",zone_host_name,ood_x.to_string());
    }

    async fn create_test_node_config(
        user_did: DID,
        username: &str,
        owner_private_key_pem: &str,
        owner_jwk: serde_json::Value,
        zone_did: DID,
        device_name: &str,
        device_private_key_pem: &str,
        device_public_key: serde_json::Value,
        is_wan: bool,
    ) -> String {
        let now = 1743478939; //2025-04-01
        let exp = now + 3600 * 24 * 365 * 10; //2035-04-01
        let owner_private_key: EncodingKey =
            EncodingKey::from_ed_pem(owner_private_key_pem.as_bytes()).unwrap();
        let owner_jwk: jsonwebtoken::jwk::Jwk = serde_json::from_value(owner_jwk.clone()).unwrap();

        let tmp_dir = std::env::temp_dir()
            .join("buckyos_dev_configs")
            .join(username)
            .join(device_name.to_string());
        std::fs::create_dir_all(tmp_dir.clone()).unwrap();
        println!(
            "# all BuckyOS dev test config files will be saved in: {:?}",
            tmp_dir
        );

        let private_key_path = tmp_dir.join("node_private_key.pem");
        std::fs::write(private_key_path.clone(), device_private_key_pem).unwrap();
        println!(
            "# device {} private key write to file: {}",
            device_name,
            private_key_path.to_string_lossy()
        );

        let device_jwk: jsonwebtoken::jwk::Jwk =
            serde_json::from_value(device_public_key.clone()).unwrap();
        let mut device_config = DeviceConfig::new_by_jwk(device_name, device_jwk.clone());

        device_config.support_container = true;
        if is_wan {
            device_config.net_id = Some("wan".to_string());
        }

        device_config.iss = user_did.to_string();
        let device_config_json_str = serde_json::to_string_pretty(&device_config).unwrap();
        println!("device config: {}", device_config_json_str);

        let device_jwt = device_config.encode(Some(&owner_private_key)).unwrap();
        println!(" device {} jwt: {}", device_name, device_jwt.to_string());

        let encode_key = EncodingKey::from_ed_pem(owner_private_key_pem.as_bytes()).unwrap();
        let decode_key = DecodingKey::from_jwk(&owner_jwk).unwrap();
        let device_jwt2 = device_config.encode(Some(&encode_key)).unwrap();
        let decode_device_config = DeviceConfig::decode(&device_jwt2, Some(&decode_key)).unwrap();
        assert_eq!(device_config, decode_device_config);

        let node_identity_config = NodeIdentityConfig {
            zone_did: zone_did.clone(),
            owner_public_key: owner_jwk.clone(),
            owner_did: user_did,
            device_doc_jwt: device_jwt.to_string(),
            zone_iat: now as u32,
        };
        let node_identity_config_json_str =
            serde_json::to_string_pretty(&node_identity_config).unwrap();
        println!(
            "node identity config: {}",
            node_identity_config_json_str.as_str()
        );
        let node_identity_config_path = tmp_dir.join("node_identity.json");
        std::fs::write(
            node_identity_config_path.clone(),
            node_identity_config_json_str.clone(),
        )
        .unwrap();
        println!(
            "# node identity config will store at {}",
            node_identity_config_path.to_string_lossy()
        );

        //build start_config.json
        if device_name.starts_with("ood") {
            let start_config = json!(
                {
                    "admin_password_hash":"o8XyToejrbCYou84h/VkF4Tht0BeQQbuX3XKG+8+GQ4=",//bucky2025
                    "device_private_key":device_private_key_pem,
                    "device_public_key":device_jwk,
                    "friend_passcode":"sdfsdfsdf",
                    "gateway_type":"PortForward",
                    "guest_access":true,
                    "private_key":owner_private_key_pem,
                    "public_key":owner_jwk,
                    "user_name":username,
                    "zone_name":zone_did.to_host_name(),
                    "BUCKYOS_ROOT":"/opt/buckyos"
                }
            );
            let start_config_json_str = serde_json::to_string_pretty(&start_config).unwrap();
            println!("start config: {}", start_config_json_str.as_str());
            let start_config_path = tmp_dir.join("start_config.json");
            std::fs::write(start_config_path.clone(), start_config_json_str.clone()).unwrap();
            println!(
                "# start_config will store at {}",
                start_config_path.to_string_lossy()
            );
        }

        return device_jwt2.to_string();
    }

    async fn create_test_sn_config() {
        let sn_server_ip = "192.168.1.188";
        let sn_server_host = "buckyos.io";
        let now = 1743478939; //2025-04-01
        let exp = now + 3600 * 24 * 365 * 10; //2035-04-01
        let tmp_dir = std::env::temp_dir()
            .join("buckyos_dev_configs")
            .join("sn_server");
        std::fs::create_dir_all(tmp_dir.clone()).unwrap();
        //create test sn zone_boot_config
        let test_sn_zone_owner_private_key = r#"
-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIH3hgzhuE0wuR+OEz0Bx6I+YrJDtS0OIajH1rNkEfxnl
-----END PRIVATE KEY-----
        "#;
        let test_sn_zone_owner_public_key = json!({
            "crv":"Ed25519",
            "kty":"OKP",
            "x":"qJdNEtscIYwTo-I0K7iPEt_UZdBDRd4r16jdBfNR0tM"
        });
        let owner_private_key: EncodingKey =
            EncodingKey::from_ed_pem(test_sn_zone_owner_private_key.as_bytes()).unwrap();
        let x_str = test_sn_zone_owner_public_key.get("x").unwrap().as_str();
        //create test sn device_key_pair

        let test_sn_device_private_key = r#"
-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIBvnIIa1Tx45SjRu9kBZuMgusP5q762SvojXZ4scFxVD
-----END PRIVATE KEY-----
        "#;
        let test_sn_device_public_key = json!({
            "crv":"Ed25519",
            "kty":"OKP",
            "x":"FPvY3WXPxuWPYFuwOY0Qbh0O7-hhKr6ta1jTcX9ORPI"
        });
        let private_key_path = tmp_dir.join("device_key.pem");
        std::fs::write(private_key_path.clone(), test_sn_device_private_key).unwrap();
        println!(
            "# device key write to file: {}",
            private_key_path.to_string_lossy()
        );
        let zone_boot_config = ZoneBootConfig {
            id: None,
            oods: vec!["ood1".to_string()],
            sn: None,
            exp: exp,
            iat: now as u32,
            owner: None,
            owner_key: None,
            gateway_devs: vec![],
            extra_info: HashMap::new(),
        };
        let zone_boot_config_json_str = serde_json::to_string_pretty(&zone_boot_config).unwrap();
        //println!("zone boot config: {}",zone_boot_config_json_str.as_str());

        let zone_boot_config_jwt = zone_boot_config.encode(Some(&owner_private_key)).unwrap();
        let zone_boot_config_jwt_str = zone_boot_config_jwt.to_string();
        let config = json!({
            "device_name":"web3_gateway",
            "device_key_path":"/opt/web3_bridge/device_key.pem",
            "inner_services":{
                "main_sn" : {
                    "type" : "cyfs-sn",
                    "host":format!("web3.{}",sn_server_host),
                    "aliases":vec![format!("sn.{}",sn_server_host)],
                    "ip":sn_server_ip,
                    "zone_config_jwt":zone_boot_config_jwt_str,
                    "zone_config_pkx":x_str

                },
                "zone_provider" : {
                    "type" : "zone-provider"
                }
            },
            "servers":{
                "main_http_server":{
                    "type":"cyfs-warp",
                    "bind":"0.0.0.0",
                    "http_port":80,
                    "tls_port":443,
                    "default_tls_host":format!("*.{}",sn_server_host),
                    "hosts": {
                        format!("web3.{}",sn_server_host): {
                            "tls": {
                                "disable_tls": true,
                                "enable_acme": false
                            },
                            "enable_cors":true,
                            "routes": {
                                "/kapi/sn":{
                                    "inner_service":"main_sn"
                                }
                            }
                        },
                        format!("*.web3.{}",sn_server_host): {
                            "tls": {
                                "disable_tls": true
                            },
                            "routes": {
                                "/":{
                                    "tunnel_selector":"main_sn"
                                }
                            }
                        },
                        "*":{
                            "routes": {
                                "/":{
                                    "tunnel_selector":"main_sn"
                                },
                                "/resolve":{
                                    "inner_service":"zone_provider"
                                }
                            }
                        }
                    }
                },
                "main_dns_server":{
                    "type":"cyfs-dns",
                    "bind":"0.0.0.0",
                    "port":53,
                    "this_name":format!("sn.{}",sn_server_host),
                    "resolver_chain": [
                        {
                          "type": "SN",
                          "server_id": "main_sn"
                        },
                        {
                            "type": "dns",
                            "cache": true
                        }
                    ],
                    "fallback": ["8.8.8.8","6.6.6.6"]
                }
            },

            "dispatcher" : {
                "udp://0.0.0.0:53":{
                    "type":"server",
                    "id":"main_dns_server"
                },
                "tcp://0.0.0.0:80":{
                    "type":"server",
                    "id":"main_http_server"
                },
                "tcp://0.0.0.0:443":{
                    "type":"server",
                    "id":"main_http_server"
                }
            }
        });

        let config_path = tmp_dir.join("web3_gateway.json");
        let config_str = serde_json::to_string_pretty(&config).unwrap();
        println!("# web3 gateway config: {}", config_str.as_str());
        std::fs::write(config_path.clone(), config_str.as_str()).unwrap();
        println!(
            "# web3 gateway config write to file: {}",
            config_path.to_string_lossy()
        );
    }

    #[tokio::test]
    async fn create_test_env_configs() {
        let mut test_web3_bridge = HashMap::new();
        test_web3_bridge.insert("bns".to_string(), "web3.buckyos.io".to_string());
        KNOWN_WEB3_BRIDGE_CONFIG.set(test_web3_bridge.clone());

        let devtest_private_key_pem = r#"
-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr
-----END PRIVATE KEY-----
        "#;

        let devtest_owner_jwk = json!(
            {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8"
            }
        );

        let devtest_node1_private_key = r#"
-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEICwMZt1W7P/9v3Iw/rS2RdziVkF7L+o5mIt/WL6ef/0w
-----END PRIVATE KEY-----"#;
        let devtest_node1_public_key = json!(
            {
                "crv":"Ed25519",
                "kty":"OKP",
                "x":"Bb325f2ed0XSxrPS5sKQaX7ylY9Jh9rfevXiidKA1zc"
            }
        );

        create_test_node_config(
            DID::new("bns", "devtest"),
            "devtest",
            devtest_private_key_pem,
            devtest_owner_jwk.clone(),
            DID::new("bns", "devtest"),
            "node1",
            devtest_node1_private_key,
            devtest_node1_public_key,
            false,
        )
        .await;

        //create bob (nodeB1) config
        let bob_private_key = r#"
-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEILQLoUZt2okCht0UVhsf4UlGAV9h3BoliwZQN5zBO1G+
-----END PRIVATE KEY-----"#;
        let bob_public_key = json!(
            {
                "crv":"Ed25519",
                "kty":"OKP",
                "x":"y-kuJcQ0doFpdNXf4HI8E814lK8MB3-t4XjDRcR_QCU"
            }
        );
        let bob_public_key_str = serde_json::to_string(&bob_public_key).unwrap();

        let bob_zone_jwt = create_test_zone_config(
            DID::new("bns", "bobdev"),
            "bobdev",
            bob_private_key,
            bob_public_key.clone(),
            DID::new("bns", "bob"),
            Some("sn.buckyos.io".to_string()),
        )
        .await;
        let bob_ood1_private_key = r#"
-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIADmO0+u/gcmStDsHZOZCM5gxNYlQmP6jpMo279TQE75
-----END PRIVATE KEY-----"#;
        let bob_ood1_public_key = json!(
            {
                "crv":"Ed25519",
                "kty":"OKP",
                "x":"iSMKakFEGzGAxLTlaB5TkqZ6d4wurObr-BpaQleoE2M"
            }
        );
        let bob_ood1_did = DID::new("dev", "iSMKakFEGzGAxLTlaB5TkqZ6d4wurObr");
        let bob_ood1_device_jwt = create_test_node_config(
            DID::new("bns", "bobdev"),
            "bobdev",
            bob_private_key,
            bob_public_key.clone(),
            DID::new("bns", "bob"),
            "ood1",
            bob_ood1_private_key,
            bob_ood1_public_key,
            false,
        )
        .await;

        //create sn db
        create_test_sn_config().await;

        let tmp_dir = std::env::temp_dir().join("buckyos_dev_configs");

        let sn_db_path = tmp_dir.join("sn_db.sqlite3");
        //delete first
        if sn_db_path.exists() {
            std::fs::remove_file(sn_db_path.clone()).unwrap();
        }

        let conn = get_sn_db_conn_by_path(sn_db_path.to_str().unwrap()).unwrap();
        initialize_database(&conn);
        insert_activation_code(&conn, "test-active-sn-code-bob").unwrap();
        insert_activation_code(&conn, "11111").unwrap();
        insert_activation_code(&conn, "22222").unwrap();
        insert_activation_code(&conn, "33333").unwrap();
        insert_activation_code(&conn, "44444").unwrap();
        insert_activation_code(&conn, "55555").unwrap();
        register_user(
            &conn,
            "test-active-sn-code-bob",
            "bob",
            bob_public_key_str.as_str(),
            bob_zone_jwt.as_str(),
            None,
        )
        .unwrap();

        let mut device_info = DeviceInfo::new("ood1", bob_ood1_did.clone());
        device_info.auto_fill_by_system_info().await.unwrap();
        let device_info_json = serde_json::to_string_pretty(&device_info).unwrap();

        // let device_info_str = r#"{"hostname":"ood1","device_type":"ood","did":"did:dev:iSMKakFEGzGAxLTlaB5TkqZ6d4wurObr","ip":"192.168.1.86","sys_hostname":"nodeB1","base_os_info":"Ubuntu 24.04.2 LTS","cpu_info":"AMD Ryzen 7 5800X 8-Core Processor @ 3800 MHz","cpu_usage":0.0,"total_mem":67392299008,"mem_usage":5.7286677}"#;

        // let device_info_str =
        register_device(
            &conn,
            "bob",
            "ood1",
            bob_ood1_did.to_string().as_str(),
            "192.168.100.100",
            device_info_json.as_str(),
        )
        .unwrap();

        println!("# sn_db already create at {}", sn_db_path.to_string_lossy());
    }

    #[test]
    fn test_zone_boot_config() {
        let private_key_pem = r#"
        -----BEGIN PRIVATE KEY-----
        MC4CAQAwBQYDK2VwBCIEIBwApVoYjauZFuKMBRe02wKlKm2B6a1F0/WIPMqDaw5F
        -----END PRIVATE KEY-----
        "#;
        let jwk = json!(
            {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": "qmtOLLWpZeBMzt97lpfj2MxZGWn3QfuDB7Q4uaP3Eok"
            }
        );
        let private_key: EncodingKey =
            EncodingKey::from_ed_pem(private_key_pem.as_bytes()).unwrap();
        let public_key_jwk: jsonwebtoken::jwk::Jwk = serde_json::from_value(jwk).unwrap();
        let public_key = DecodingKey::from_jwk(&public_key_jwk).unwrap();

        let zone_boot_config = ZoneBootConfig {
            id: None,
            oods: vec!["ood1".to_string()],
            sn: None,
            exp: buckyos_get_unix_timestamp() + 3600 * 24 * 365 * 3,
            iat: buckyos_get_unix_timestamp() as u32,
            owner: None,
            owner_key: None,
            gateway_devs: vec![],
            extra_info: HashMap::new(),
        };

        let zone_boot_config_jwt = zone_boot_config.encode(Some(&private_key)).unwrap();
        println!("zone_boot_config_jwt: {:?}", zone_boot_config_jwt);

        //decode
        let zone_boot_config_decoded =
            ZoneBootConfig::decode(&zone_boot_config_jwt, Some(&public_key)).unwrap();
        println!("zone_boot_config_decoded: {:?}", zone_boot_config_decoded);

        assert_eq!(zone_boot_config, zone_boot_config_decoded);
    }

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
        let public_key_jwk: jsonwebtoken::jwk::Jwk = serde_json::from_value(jwk).unwrap();
        let private_key: EncodingKey =
            EncodingKey::from_ed_pem(private_key_pem.as_bytes()).unwrap();
        let public_key = DecodingKey::from_jwk(&public_key_jwk).unwrap();

        let zone_config = ZoneConfig::new(
            DID::new("web", "test.buckyos.io"),
            DID::new("bns", "devtest"),
            public_key_jwk,
        );

        let json_str = serde_json::to_string(&zone_config).unwrap();
        println!("json_str: {:?}", json_str);

        let encoded = zone_config.encode(Some(&private_key)).unwrap();
        println!("encoded: {:?}", encoded);

        let decoded = ZoneConfig::decode(&encoded, Some(&public_key)).unwrap();
        println!("decoded: {:?}", serde_json::to_string(&decoded).unwrap());
        let token2 = decoded.encode(Some(&private_key)).unwrap();

        assert_eq!(zone_config, decoded);
        assert_eq!(encoded, token2);
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
        let public_key_jwk: jsonwebtoken::jwk::Jwk = serde_json::from_value(owner_jwk).unwrap();
        let owner_private_key: EncodingKey =
            EncodingKey::from_ed_pem(owner_private_key_pem.as_bytes()).unwrap();
        let public_key = DecodingKey::from_jwk(&public_key_jwk).unwrap();

        //ood1 privete key:

        let ood_public_key = json!(
            {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": "gubVIszw-u_d5PVTh-oc8CKAhM9C-ne5G_yUK5BDaXc"
            }
        );
        let ood_key_jwk: jsonwebtoken::jwk::Jwk = serde_json::from_value(ood_public_key).unwrap();
        let mut device_config = DeviceConfig::new(
            "ood1",
            "gubVIszw-u_d5PVTh-oc8CKAhM9C-ne5G_yUK5BDaXc".to_string(),
        );
        device_config.iss = "did:bns:lzc".to_string();

        let json_str = serde_json::to_string(&device_config).unwrap();
        println!("ood json_str: {:?}", json_str);

        let encoded = device_config.encode(Some(&owner_private_key)).unwrap();
        println!("ood encoded: {:?}", encoded);

        let decoded = DeviceConfig::decode(&encoded, Some(&public_key)).unwrap();
        println!(
            "ood decoded: {:?}",
            serde_json::to_string(&decoded).unwrap()
        );
        let token2 = decoded.encode(Some(&owner_private_key)).unwrap();

        let mut device_info_ood = DeviceInfo::from_device_doc(&decoded);
        device_info_ood.auto_fill_by_system_info().await;
        let device_info_str = serde_json::to_string(&device_info_ood).unwrap();
        println!("ood device_info: {}", device_info_str);

        assert_eq!(device_config, decoded);
        assert_eq!(encoded, token2);

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
        let gateway_key_jwk: jsonwebtoken::jwk::Jwk =
            serde_json::from_value(gateway_public_key).unwrap();
        let device_config = DeviceConfig::new(
            "gateway",
            "M3-pAdhs0uFkWmmjdHLBfs494R91QmQeXzCEhEHP-tI".to_string(),
        );

        let json_str = serde_json::to_string(&device_config).unwrap();
        println!("gateway json_str: {:?}", json_str);

        let encoded = device_config.encode(Some(&owner_private_key)).unwrap();
        println!("gateway encoded: {:?}", encoded);

        let decoded = DeviceConfig::decode(&encoded, Some(&public_key)).unwrap();
        println!(
            "gateway decoded: {:?}",
            serde_json::to_string(&decoded).unwrap()
        );
        let token2 = decoded.encode(Some(&owner_private_key)).unwrap();

        assert_eq!(device_config, decoded);
        assert_eq!(encoded, token2);

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
        let server_key_jwk: jsonwebtoken::jwk::Jwk =
            serde_json::from_value(server_public_key).unwrap();
        let mut device_config = DeviceConfig::new(
            "server1",
            "LBgzvFCD4VqQxTsO2LCZjs9FPVaQV2Dt0Q5W_lr4mr0".to_string(),
        );
        device_config.iss = "did:bns:waterflier".to_string();
        device_config.ip = None;
        device_config.net_id = None;

        let json_str = serde_json::to_string(&device_config).unwrap();
        println!("server json_str: {:?}", json_str);

        let encoded = device_config.encode(Some(&owner_private_key)).unwrap();
        println!("server encoded: {:?}", encoded);

        let decoded = DeviceConfig::decode(&encoded, Some(&public_key)).unwrap();
        println!(
            "server decoded: {:?}",
            serde_json::to_string(&decoded).unwrap()
        );
        let token2 = decoded.encode(Some(&owner_private_key)).unwrap();

        assert_eq!(device_config, decoded);
        assert_eq!(encoded, token2);
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
        let public_key_jwk: jsonwebtoken::jwk::Jwk = serde_json::from_value(jwk).unwrap();
        let private_key: EncodingKey =
            EncodingKey::from_ed_pem(private_key_pem.as_bytes()).unwrap();
        let public_key = DecodingKey::from_jwk(&public_key_jwk).unwrap();

        let mut owner_config = OwnerConfig::new(
            DID::new("bns", "lzc"),
            "lzc".to_string(),
            "zhicong liu".to_string(),
            public_key_jwk,
        );

        owner_config.set_default_zone_did(DID::new("bns", "waterflier"));

        let json_str = serde_json::to_string_pretty(&owner_config).unwrap();
        println!("json_str: {}", json_str.as_str());

        let encoded = owner_config.encode(Some(&private_key)).unwrap();
        println!("encoded: {:?}", encoded);

        let decoded = OwnerConfig::decode(&encoded, Some(&public_key)).unwrap();
        println!(
            "decoded: {}",
            serde_json::to_string_pretty(&decoded).unwrap()
        );
        let token2 = decoded.encode(Some(&private_key)).unwrap();

        assert_eq!(owner_config, decoded);
        assert_eq!(encoded, token2);
    }
}
