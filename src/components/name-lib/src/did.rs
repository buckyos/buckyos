

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
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
    
    pub fn from_str(did: &str) -> Option<Self> {
        let parts: Vec<&str> = did.split(':').collect();
        Some(DID {
            method: parts[1].to_string(),
            id: parts[2].to_string(),
        })
    }

    pub fn to_string(&self) -> String {
        format!("did:{}:{}", self.method, self.id)
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct DIDSimpleDocument {
    pub did: String,
    pub payload: Option<String>, 
    pub public_key: Option<String>,
    pub signature: Option<String>,
    pub last_modified: Option<u64>,
}

impl DIDSimpleDocument {
    pub fn new() -> Self {
        DIDSimpleDocument {
            did: String::new(),
            payload: None,
            public_key: None,
            signature: None,
            last_modified: None,
        }
    }
}

#[derive(Clone, Serialize, Deserialize,Debug)]
pub struct ZoneConfig {
    pub zone_name: String,
    pub did: String,
    pub oods: Vec<String>, //etcd server endpoints
    pub backup_server_info:Option<String>
}


#[derive(Clone, Serialize, Deserialize,Debug)]
pub struct DeviceConfig {
    pub did: String,
    pub hostname: String,
    pub device_type: String,
}
