

use std::collections::HashMap;

use jsonwebtoken::{jwk::Jwk, DecodingKey, EncodingKey};
use serde::{Deserialize, Serialize,Serializer, Deserializer};
use serde_json::{Value, json};
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, engine::general_purpose::STANDARD,Engine as _};
use crate::{decode_jwt_claim_without_verify, NSError, NSResult};
use crate::config::{OwnerConfig, DeviceConfig,ZoneConfig,ZoneBootConfig};

#[derive(Clone,Debug,PartialEq,Hash,Eq,PartialOrd,Ord)]
pub struct DID {
    pub method: String,
    pub id: String,
}

pub const DID_DOC_AUTHKEY: &str = "#auth-key";

impl DID {
    pub fn new(method: &str, id: &str) -> Self {
        DID {
            method: method.to_string(),
            id: id.to_string(),
        }
    }

    pub fn undefined() -> Self {
        DID {
            method: "undefined".to_string(),
            id: "undefined".to_string(),
        }
    }

    pub fn is_valid(&self) -> bool {
        self.method != "undefined"
    }

    pub fn get_ed25519_auth_key(&self) -> Option<[u8; 32]> {
        if self.method == "dev" {
            let auth_key = URL_SAFE_NO_PAD.decode(self.id.as_str()).unwrap();
            return Some(auth_key.try_into().unwrap());
        }
        None
    }

    pub fn get_auth_key(&self) -> Option<(DecodingKey,Jwk)> {
        if self.method == "dev" {
           let jwk = json!({
            "kty": "OKP",
            "crv": "Ed25519",
            "x": self.id,
           });
           let jwk = serde_json::from_value(jwk);
           if jwk.is_err() {
            return None;
           }
           let jwk:Jwk = jwk.unwrap();
           return Some((DecodingKey::from_jwk(&jwk).unwrap(),jwk));
        }
        None
    }

    pub fn is_self_auth(&self) -> bool {
        self.method == "dev"
    }
    
    pub fn from_str(did: &str) -> NSResult<Self> {
        let parts: Vec<&str> = did.split(':').collect();
        if parts[0] != "did" {
            //this is a host name
            let result = Self::from_host_name(did);
            if result.is_some() {
                return Ok(result.unwrap());
            }
            return Err(NSError::InvalidDID(format!("invalid did {}",did)));
        }
        let id = parts[2..].join(":");
        Ok(DID {
            method: parts[1].to_string(),
            id,
        })
    }

    pub fn to_string(&self) -> String {
        format!("did:{}:{}", self.method, self.id)
    }

    pub fn to_host_name(&self) -> String {
        if self.method == "web" {
            return self.id.clone();
        }

        let web3_bridge_config = KNOWN_WEB3_BRIDGE_CONFIG.get();
        if web3_bridge_config.is_some() {
            let web3_bridge_config = web3_bridge_config.unwrap();
            let bridge_base_hostname = web3_bridge_config.get(self.method.as_str());
            if bridge_base_hostname.is_some() {
                return format!("{}.{}",self.id,bridge_base_hostname.unwrap());
            }
        }
        //todo: find web3 bridge config
        format!("{}.{}.did",self.id,self.method)
    }

    fn from_host_name(host_name: &str) -> Option<Self> {
        if host_name.ends_with(".did") {
            let parts: Vec<&str> = host_name.split('.').collect();
            if parts.len() == 3 {
                return Some(DID::new(parts[1].to_string().as_str(), parts[0]));
            }
        }

        let web3_bridge_config = KNOWN_WEB3_BRIDGE_CONFIG.get();
        if web3_bridge_config.is_some() {
            let web3_bridge_config = web3_bridge_config.unwrap();
            for (method,bridge_base_hostname) in web3_bridge_config.iter() {
                if host_name.ends_with(bridge_base_hostname) {
                    let id = host_name[..host_name.len()-bridge_base_hostname.len()-1].to_string();
                    return Some(DID::new(method, &id));
                }
            }
        }

        return Some(DID::new("web", host_name.to_string().as_str()));
    }

    pub fn is_did(did: &str) -> bool {
        did.starts_with("did:")
    }
}

impl Serialize for DID {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for DID {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let result = Self::from_str(&s);
        if result.is_err() {
            return Err(serde::de::Error::custom(format!("invalid did: {}", s)));
        }
        Ok(result.unwrap())
    }
}

#[derive(Clone, Serialize, Deserialize,Debug,PartialEq)]
pub enum EncodedDocument {
    JsonLd(Value),
    Jwt(String),
}

impl EncodedDocument {
    pub fn is_proof(&self) -> bool {
        match self {
            EncodedDocument::Jwt(_jwt) => true,
            _ => false,
        }
    }

    // pub fn get_prover_kid(&self) -> Option<String> {
    //     match self {
    //         EncodedDocument::Jwt(jwt) => {
    //             //return jwt header kid
    //             let header = decode_jwt_header_without_verify(jwt.as_str()).unwrap();
    //             header.kid
    //         }
    //         _ => None,
    //     }
    // }

    pub fn to_string(&self) -> String {
        match self {
            EncodedDocument::Jwt(jwt) => jwt.clone(),
            EncodedDocument::JsonLd(value) => serde_json::to_string(value).unwrap(),
        }
    }

    pub fn from_str(doc_str: String) -> NSResult<Self> {
        if doc_str.starts_with("{") {
            let real_value = serde_json::from_str(&doc_str)
                .map_err(|e| NSError::DecodeJWTError(e.to_string()))?;
            return Ok(EncodedDocument::JsonLd(real_value));
        }
        return Ok(EncodedDocument::Jwt(doc_str));
    }

    pub fn to_json_value(self)->NSResult<Value> {
        match self {
            EncodedDocument::Jwt(jwt_str) => {
                let claims = decode_jwt_claim_without_verify(jwt_str.as_str())
                    .map_err(|e| NSError::DecodeJWTError(e.to_string()))?;
                Ok(claims)
            },
            EncodedDocument::JsonLd(value) => Ok(value),
        }
    }
}

#[async_trait]
pub trait DIDDocumentTrait {
    fn get_id(&self) -> DID;
    //key id is none means the default key
    fn get_auth_key(&self,kid:Option<&str>) -> Option<(DecodingKey,Jwk)>;
    fn get_exchange_key(&self,kid:Option<&str>) -> Option<(DecodingKey,Jwk)>;

    fn get_iss(&self) -> Option<String>;
    fn get_exp(&self) -> Option<u64>;
    fn get_iat(&self) -> Option<u64>;

    fn encode(&self,key:Option<&EncodingKey>) -> NSResult<EncodedDocument>;
    fn decode(doc: &EncodedDocument,key:Option<&DecodingKey>) -> NSResult<Self> where Self: Sized;
    // async fn decode_with_load_key<'a, F, Fut>(doc: &'a EncodedDocument,loader:F) -> NSResult<Self> 
    //     where Self: Sized,
    //           F: Fn(&'a str) -> Fut,
    //           Fut: std::future::Future<Output = NSResult<DecodingKey>>;

    //JSON-LD
    //fn to_json_value(&self) -> Value;
    //fn from_json_value(value: &Value) -> Self;
}


pub static KNOWN_WEB3_BRIDGE_CONFIG:OnceCell<HashMap<String,String>> = OnceCell::new();

pub fn parse_did_doc(doc: EncodedDocument) -> NSResult<Box<dyn DIDDocumentTrait>> {
    let doc_value = doc.to_json_value()?;
    if doc_value.get("verificationMethod").is_none() {
        let zone_boot_config = serde_json::from_value::<ZoneBootConfig>(doc_value).map_err(|e| NSError::Failed(format!("parse zone boot config failed: {}",e)))?;
        return Ok(Box::new(zone_boot_config));
    }

    if doc_value.get("full_name").is_some() {
        let owner_config = serde_json::from_value::<OwnerConfig>(doc_value).map_err(|e| NSError::Failed(format!("parse owner config failed: {}",e)))?;
        return Ok(Box::new(owner_config));
    }
    if doc_value.get("device_type").is_some() {
        let device_config = serde_json::from_value::<DeviceConfig>(doc_value).map_err(|e| NSError::Failed(format!("parse device config failed: {}",e)))?;
        return Ok(Box::new(device_config));
    }

    if doc_value.get("oods").is_some() {
        let zone_config = serde_json::from_value::<ZoneConfig>(doc_value).map_err(|e| NSError::Failed(format!("parse zone config failed: {}",e)))?;
        return Ok(Box::new(zone_config));
    }

    Err(NSError::Failed("unknown did document".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_did_from_str() {
        let did = DID::from_str("did:bns:waterflier").unwrap();
        assert_eq!(did.method, "bns");
        assert_eq!(did.id, "waterflier");

        let did = DID::from_str("did:bns:waterflier:sssn.did").unwrap();
        assert_eq!(did.method, "bns");
        assert_eq!(did.id, "waterflier:sssn.did");

        let mut web3_bridge_config = HashMap::new();
        web3_bridge_config.insert("bns".to_string(), "web3.buckyos.io".to_string());
        let _ = KNOWN_WEB3_BRIDGE_CONFIG.set(web3_bridge_config);

        let did = DID::from_host_name("waterflier.web3.buckyos.io").unwrap();
        assert_eq!(did.method, "bns");
        assert_eq!(did.id, "waterflier");
        let host_name = did.to_host_name();
        assert_eq!(host_name, "waterflier.web3.buckyos.io");

        let did = DID::from_host_name("zhicong.me").unwrap();
        assert_eq!(did.method, "web");
        assert_eq!(did.id, "zhicong.me");

        let did = DID::from_str("buckyos.ai").unwrap();
        assert_eq!(did.method, "web");
        assert_eq!(did.id, "buckyos.ai");
        let host_name = did.to_host_name();
        assert_eq!(host_name, "buckyos.ai");
        let did_str = did.to_string();
        assert_eq!(did_str, "did:web:buckyos.ai");


        let did = DID::from_str("abcdef.dev.did").unwrap();
        assert_eq!(did.method, "dev");
        assert_eq!(did.id, "abcdef");
        let did_str = did.to_string();
        assert_eq!(did_str, "did:dev:abcdef");
    }
    
}

