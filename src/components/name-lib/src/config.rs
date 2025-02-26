use std::net::IpAddr;
use std::net::SocketAddr;

use jsonwebtoken::jwk::Jwk;
use jsonwebtoken::{encode, Algorithm, DecodingKey, EncodingKey, Header};
use log::*;
use rand::seq::SliceRandom;
use serde::{Serialize,Deserialize};
use serde_json::json;
use buckyos_kit::*;
use once_cell::sync::OnceCell;
use crate::DID;
use crate::DeviceInfo;

use crate::{DIDDocumentTrait,EncodedDocument};
use crate::{NSResult,NSError};
use crate::{decode_json_from_jwt_with_pk,decode_jwt_claim_without_verify,decode_json_from_jwt_with_default_pk};


#[derive(Clone, Serialize, Deserialize,Debug,PartialEq)]
pub struct VerifyHubInfo {
    pub port:u16,
    pub node_name:String,
    pub public_key:Jwk,
}


#[derive(Clone, Serialize, Deserialize,Debug,PartialEq)]
pub struct ZoneConfig {
    pub did: String,//full did
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,//domain name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_key : Option<Jwk>,//owner's public key
    
    //ood server endpoints,can be ["ood1","ood2@192.168.32.1","ood3#vlan1]
    pub oods: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub services:Option<Vec<String>>,//like ["http:0","https:443","smb"],0 means disable
    
    //因为所有的Node上的Gateway都是同质的，所以这里可以不用配置？DNS记录解析到哪个Node，哪个Node的Gateway就是ZoneGateway
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway:Option<String>,//default gateway node name for this zone,like gate@210.22.12.3#wan
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sn:Option<String>,//
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vlan:Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verify_hub_info:Option<VerifyHubInfo>,
    pub exp:u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iat:Option<u64>,
}

impl ZoneConfig {
    pub fn get_zone_short_name(&self) -> String {
        let did = DID::from_str(self.did.as_str());
        if did.is_some() {
            let did = did.unwrap();
            return did.id.clone();
        }
        
        if self.name.is_some() {
            let name = self.name.as_ref().unwrap();
            let short_name = name.split('.').next().unwrap_or(name);
            return short_name.to_string();
        }

        return self.did.clone();
    }

    pub fn get_test_config() -> ZoneConfig {
        let jwk = json!(
            {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": "gubVIszw-u_d5PVTh-oc8CKAhM9C-ne5G_yUK5BDaXc"
            }
        );
        let public_key_jwk : jsonwebtoken::jwk::Jwk = serde_json::from_value(jwk).unwrap();
        return ZoneConfig {
            did: "did:ens:example".to_string(),
            name: Some("waterflier.web3.buckyos.io".to_string()),
            owner_name: None,
            auth_key: Some(public_key_jwk),
            oods: vec!["ood01".to_string()],
            gateway: None,
            sn: None,
            vlan: None,
            services: None,
            verify_hub_info: None,
            iat:None,
            exp: buckyos_get_unix_timestamp() + 3600*24*365,
        }
    }

    pub fn get_node_host_name(&self,node_name:&str) -> String {
        let zone_short_name = self.get_zone_short_name();
        let host_name = format!("{}-{}",zone_short_name,node_name);
        return host_name;
    }

    pub fn get_ood_string(&self,node_name:&str) -> Option<String> {
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
            let ood_device_info = DeviceInfo::new(ood,None);
            if ood_device_info.net_id == device_info.net_id {
                return Some(ood.clone());
            }
        }

        return None;
    }

    pub fn select_wan_ood(&self) -> Option<String> {
        let mut ood_list = self.oods.clone();
        ood_list.shuffle(&mut rand::thread_rng());
        for ood in self.oods.iter() {
            let device_info = DeviceInfo::new(ood,None);
            if device_info.is_wan_device() {
                return Some(ood.clone());
            }
        }
        return None;
    }

    pub fn get_sn_url(&self) -> Option<String> {
        let sn_port = self.get_service_port("http");
        if sn_port.is_none() {
            return None;
        }

        let sn_port = sn_port.unwrap();
        if self.sn.is_some() {
            if sn_port == 80 {
                return Some(format!("http://{}/kapi/sn",self.sn.as_ref().unwrap()));
            } else {
                return Some(format!("http://{}:{}/kapi/sn",self.sn.as_ref().unwrap(),sn_port));
            }
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

    pub fn get_service_port(&self,service_name: &str) -> Option<u16> {
        if self.services.is_none() {
            return self.get_default_service_port(service_name);
        }
        let services = self.services.as_ref().unwrap();
        if services.is_empty() {
            return self.get_default_service_port(service_name);
        }

        for service in services.iter() {
            if service.starts_with(service_name) {
                let port_str = service.split(':').nth(1).unwrap_or("");
                if port_str.is_empty() {
                    return self.get_default_service_port(service_name);
                }
                if port_str == "0" {
                    return None;
                }
                return Some(port_str.parse::<u16>().unwrap());
            }
        }

        return None;
    }
    
}



impl DIDDocumentTrait for ZoneConfig {
    
    fn get_did(&self) -> &str {    
        return self.did.as_str();
    }
    fn get_auth_key(&self) -> Option<DecodingKey> {
        if self.auth_key.is_some() {
            let result_key = DecodingKey::from_jwk(&self.auth_key.as_ref().unwrap());
            if result_key.is_err() {
                error!("Failed to decode auth key: {:?}",result_key.err().unwrap());
                return None;
            }
            return Some(result_key.unwrap());
        } else {
            return None;
        }
    }
    fn is_proof(self) -> bool{
        return true;
    }
    fn get_prover_kid(&self) -> Option<String> {
        if self.owner_name.is_some() {
            return Some(format!("{}#auth_key",self.owner_name.as_ref().unwrap()));
        }
        return None;
    }
    fn get_iss(&self) -> Option<String> {
        return self.owner_name.clone();
    }
    fn get_exp(&self) -> Option<u64> {
        return Some(self.exp)
    }
    fn get_iat(&self) -> Option<u64> {
        return self.iat;
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
                if key.is_none() {
                    return Err(NSError::Failed("No key provided".to_string()));
                }
                let json_result = decode_json_from_jwt_with_pk(jwt_str,key.unwrap())?;
                let result:ZoneConfig = serde_json::from_value(json_result).map_err(|error| {
                    NSError::Failed(format!("Failed to decode zone config:{}",error))
                })?;
                return Ok(result);
            },
            EncodedDocument::JsonLd(json_value) => {
                let result:ZoneConfig = serde_json::from_value(json_value.clone()).map_err(|error| {
                    NSError::Failed(format!("Failed to decode zone config:{}",error))
                })?;
                return Ok(result);
            },
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
    pub did: String,

    pub name: String,//host name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    pub device_type: String,//[ood,node,sensor
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip:Option<IpAddr>,//main_ip
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net_id:Option<String>,// lan1 | wan ，为None时表示为 lan0
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ddns_sn_url:Option<String>,
    pub auth_key : Jwk,
    #[serde(skip_serializing_if = "is_true", default = "bool_default_true")]
    pub support_container:bool,
    pub iss:String,
    pub exp:u64,
    pub iat:u64,
}

impl DeviceConfig {
    pub fn new(name:&str,pkx:Option<String>) -> Self {
        if pkx.is_some() {
            let did = format!("did:dev:{}",pkx.as_ref().unwrap());
            let jwk = json!(
                {
                    "kty": "OKP",
                    "crv": "Ed25519",
                    "x": pkx
                }
            );
            let public_key_jwk : jsonwebtoken::jwk::Jwk = serde_json::from_value(jwk).unwrap();
            DeviceConfig {
                did: did,
                name: name.to_string(),
                arch: None,
                device_type: "node".to_string(),
                ip: None,
                net_id: None,
                ddns_sn_url: None,
                auth_key: public_key_jwk,
                support_container: true,
                iss: "".to_string(),
                exp: 0,
                iat: 0,
            }
        } else {
            let jwk = json!(
                {
                    "kty": "OKP",
                    "crv": "Ed25519",
                    "x": "gubVIszw-u_d5PVTh-oc8CKAhM9C-ne5G_yUK5BDaXc"
                }
            );
            let public_key_jwk : jsonwebtoken::jwk::Jwk = serde_json::from_value(jwk).unwrap();
            DeviceConfig {
                did: name.to_string(),
                name: name.to_string(),
                arch: None,
                device_type: "node".to_string(),
                ip: None,
                net_id: None,
                ddns_sn_url: None,
                auth_key: public_key_jwk,
                support_container: true,
                iss: "".to_string(),
                exp: 0,
                iat: 0,
            } 
        }
    }

    pub fn get_test_config() -> DeviceConfig {
        let jwk = json!(
            {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": "gubVIszw-u_d5PVTh-oc8CKAhM9C-ne5G_yUK5BDaXc"
            }
        );
        let public_key_jwk : jsonwebtoken::jwk::Jwk = serde_json::from_value(jwk).unwrap();
        return DeviceConfig {
            did: "did:dev:gubVIszw-u_d5PVTh-oc8CKAhM9C-ne5G_yUK5BDaXc".to_string(),
            name: "ood1".to_string(),
            arch: None,
            device_type: "ood".to_string(),
            ip:None,
            net_id:None,
            ddns_sn_url: None,
            auth_key: public_key_jwk,
            support_container: true,
            iss: "waterfllier".to_string(),
            exp: buckyos_get_unix_timestamp() + 3600*24*365, 
            iat: buckyos_get_unix_timestamp(),
        }
    }
}

impl DIDDocumentTrait for DeviceConfig {
    fn get_did(&self) -> &str {
        return self.did.as_str()
    }
    fn get_auth_key(&self) -> Option<DecodingKey> {
        let result_key = DecodingKey::from_jwk(&self.auth_key);
        if result_key.is_err() {
            error!("Failed to decode auth key: {:?}",result_key.err().unwrap());
            return None;
        }
        return Some(result_key.unwrap());
    }
    fn is_proof(self) -> bool{
        return true;
    }
    fn get_prover_kid(&self) -> Option<String> {
        return Some(format!("{}#auth_key",self.iss));
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
                if key.is_none() {
                    return Err(NSError::Failed("No key provided".to_string()));
                }
                let json_result = decode_json_from_jwt_with_pk(jwt_str,key.unwrap())?;
                let result:DeviceConfig = serde_json::from_value(json_result).map_err(|error| {
                    NSError::Failed(format!("Failed to decode zone config:{}",error))
                })?;
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
    pub did: String,

    pub name: String,
    pub nickname : String,
    pub auth_key : Jwk,

    pub exp:u64,
    pub iat:u64,
}

impl DIDDocumentTrait for OwnerConfig {
    fn get_did(&self) -> &str {
        return self.did.as_str()
    }
    fn get_auth_key(&self) -> Option<DecodingKey> {
        let result_key = DecodingKey::from_jwk(&self.auth_key);
        if result_key.is_err() {
            error!("Failed to decode auth key: {:?}",result_key.err().unwrap());
            return None;
        }
        return Some(result_key.unwrap());
    }
    fn is_proof(self) -> bool{
        return false;
    }
    fn get_prover_kid(&self) -> Option<String> {
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
                let json_result = decode_json_from_jwt_with_default_pk(jwt_str,&result.auth_key)?;
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
            did: "did:bns:dev_test".to_string(),
            name: None,
            owner_name: None,
            gateway: None,
            auth_key: None, 
            oods: vec!["ood1".to_string()],
            services: None,
            sn: None,
            vlan: None,
            verify_hub_info: None,
            iat:None,
            exp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as u64 + 3600*24*365*10, 
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
            did: "did:dev:gubVIszw-u_d5PVTh-oc8CKAhM9C-ne5G_yUK5BDaXc".to_string(),
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
            did: "did:dev:M3-pAdhs0uFkWmmjdHLBfs494R91QmQeXzCEhEHP-tI".to_string(),
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
            did: "did:dev:LBgzvFCD4VqQxTsO2LCZjs9FPVaQV2Dt0Q5W_lr4mr0".to_string(),
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

        let owner_config = OwnerConfig {
            did: "did:ens:waterfllier".to_string(),
            name: "waterflier".to_string(),
            nickname: "zhicong liu".to_string(),
            auth_key: public_key_jwk,
            exp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as u64 + 3600*24*365, 
            iat: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as u64,
        };   
        let json_str = serde_json::to_string(&owner_config).unwrap();
        println!("json_str: {:?}",json_str);

        let encoded = owner_config.encode(Some(&private_key)).unwrap();
        println!("encoded: {:?}",encoded);

        let decoded = OwnerConfig::decode(&encoded,None).unwrap();
        println!("decoded: {:?}",serde_json::to_string(&decoded).unwrap());
        let token2 = decoded.encode(Some(&private_key)).unwrap();

        assert_eq!(owner_config,decoded);
        assert_eq!(encoded,token2); 
    }
}