use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use crate::{DIDDocumentTrait, EncodedDocument , NSError, NSResult};


#[derive(Clone, Serialize, Deserialize)]
pub struct EndPointInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    protocol: Option<String>,
    addr: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
}


#[derive(Clone, Serialize, Deserialize,Debug)]
pub enum NameProof {
    None, 
    ServerProof,
    OwnerProof,
}

#[derive(Clone, Serialize, Deserialize,Debug)]
pub struct NameInfo {
    pub name: String,
    pub address: Vec<IpAddr>,
    pub cname:Option<String>,
    pub txt:Option<String>,
    pub did_document:Option<EncodedDocument>,
    pub proof_type:NameProof,
    pub create_time: u64,
    pub ttl: Option<u32>,
}


#[async_trait::async_trait]
pub trait NSProvider: 'static + Send + Sync {
    fn get_id(&self) -> String;
    async fn query(&self, name: &str,record_type:Option<&str>) -> NSResult<NameInfo>;
    async fn query_did(&self, did: &str,fragment:Option<&str>) -> NSResult<EncodedDocument>;
}
