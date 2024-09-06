use jsonwebtoken::jwk::Jwk;
use jsonwebtoken::{encode, Algorithm, DecodingKey, EncodingKey, Header};
use log::*;
use serde::{Serialize,Deserialize};
use serde_json::json;
use buckyos_kit::*;

use crate::{DIDDocumentTrait,EncodedDocument};
use crate::{NSResult,NSError};
use crate::{decode_json_from_jwt_with_pk,decode_jwt_claim_without_verify,decode_json_from_jwt_with_default_pk};


#[derive(Clone, Serialize, Deserialize,Debug,PartialEq)]
pub struct VerifyHubInfo {
    pub node_name:String,
    pub public_key:Jwk,
}

#[derive(Clone, Serialize, Deserialize,Debug,PartialEq)]
pub struct ZoneConfig {
    pub did: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_key : Option<Jwk>,
    pub oods: Vec<String>, //etcd server endpoints
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_server_info:Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verify_hub_info:Option<VerifyHubInfo>,
    pub exp:u64,
    pub iat:u64,
}

impl ZoneConfig {
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
            name: Some("www.example.com".to_string()),
            owner_name: None,
            auth_key: Some(public_key_jwk),
            oods: vec!["ood01".to_string()],
            backup_server_info: None,
            verify_hub_info: None,
            exp: buckyos_get_unix_timestamp() + 3600*24*365,
            iat: buckyos_get_unix_timestamp(),
        }
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
        if self.owner_name.is_none() {
            return None;
        }   
        return Some(format!("{}#auth_key",self.owner_name.as_ref().unwrap()));
    }
    fn get_iss(&self) -> Option<String> {
        return self.owner_name.clone();
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
    OOD,
    Server,
    Sensor,
}

#[derive(Clone, Serialize, Deserialize,Debug,PartialEq)]
pub struct DeviceConfig {
    pub did: String,

    pub name: String,
    pub device_type: String,
    pub auth_key : Jwk,

    pub iss:String,
    pub exp:u64,
    pub iat:u64,
}

impl DeviceConfig {
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
            device_type: "ood".to_string(),
            auth_key: public_key_jwk,
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
            did: "did:ens:buckyos".to_string(),
            name: None,
            owner_name: None,
            auth_key: None,
            oods: vec!["ood01".to_string()],
            backup_server_info: Some("http://abcd@backup.example.com".to_string()),
            verify_hub_info: None,
            
            exp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as u64 + 3600, 
            iat: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as u64,
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

    #[test]
    fn test_device_config() {
        let owner_private_key_pem = r#"
        -----BEGIN PRIVATE KEY-----
        MC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr
        -----END PRIVATE KEY-----
        "#;
        let device_jwk = json!(
            {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": "gubVIszw-u_d5PVTh-oc8CKAhM9C-ne5G_yUK5BDaXc"
            }
        );
        let public_key_jwk : jsonwebtoken::jwk::Jwk = serde_json::from_value(device_jwk).unwrap();
        let private_key: EncodingKey = EncodingKey::from_ed_pem(owner_private_key_pem.as_bytes()).unwrap();
        let public_key = DecodingKey::from_jwk(&public_key_jwk).unwrap();

        let device_config = DeviceConfig {
            did: "did:dev:gubVIszw-u_d5PVTh-oc8CKAhM9C-ne5G_yUK5BDaXc".to_string(),
            name: "ood01".to_string(),
            device_type: "ood".to_string(),
            auth_key: public_key_jwk,
            iss: "waterfllier".to_string(),
            exp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as u64 + 3600*24*365, 
            iat: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as u64,
        };

        let json_str = serde_json::to_string(&device_config).unwrap();
        println!("json_str: {:?}",json_str);

        let encoded = device_config.encode(Some(&private_key)).unwrap();
        println!("encoded: {:?}",encoded);

        let decoded = DeviceConfig::decode(&encoded,Some(&public_key)).unwrap();
        println!("decoded: {:?}",serde_json::to_string(&decoded).unwrap());
        let token2 = decoded.encode(Some(&private_key)).unwrap();

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