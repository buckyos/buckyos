use jsonwebtoken::jwk::Jwk;
use jsonwebtoken::{EncodingKey,DecodingKey};
use log::*;
use serde::{Serialize,Deserialize};

use crate::{DIDDocumentTrait,EncodedDocument};
use crate::{NSResult,NSError};
use crate::{decode_json_from_jwt_with_pk,decode_jwt_claim_without_verify,decode_json_from_jwt_with_default_pk};


#[derive(Clone, Serialize, Deserialize,Debug)]
pub struct VerifyHubInfo {
    pub node_name:String,
    pub public_key:Jwk,
}


#[derive(Clone, Serialize, Deserialize,Debug)]
pub struct ZoneConfig {
    pub did: String,

    pub name: String,
    pub owner_name: String,
    pub auth_key : Option<Jwk>,
    pub oods: Vec<String>, //etcd server endpoints
    pub backup_server_info:Option<String>,
    pub verify_hub_info:Option<VerifyHubInfo>,

    pub iss:String,
    pub exp:u64,
    pub iat:u64,
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
        return Some(format!("{}#auth_key",self.owner_name));
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
        unimplemented!()
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


#[derive(Clone, Serialize, Deserialize,Debug)]
pub struct DeviceConfig {
    pub did: String,

    pub name: String,
    pub device_type: String,
    pub auth_key : Jwk,
    pub owner_name:String,

    pub iss:String,
    pub exp:u64,
    pub iat:u64,
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
        return Some(format!("{}#auth_key",self.owner_name));
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
        unimplemented!()
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

#[derive(Clone, Serialize, Deserialize,Debug)]
pub struct OwnerConfig {
    pub did: String,

    pub name: String,
    pub nickname : String,
    pub auth_key : Jwk,

    pub iss:String,
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
        return Some(self.iss.clone());
    }
    fn get_exp(&self) -> Option<u64> {
        return Some(self.exp)
    }
    fn get_iat(&self) -> Option<u64> {
        return Some(self.iat)
    }

    fn encode(&self,key:Option<&EncodingKey>) -> NSResult<EncodedDocument> {
        unimplemented!()
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
    use super::*;

    #[test]
    fn test_zone_config() {
        
    }
}