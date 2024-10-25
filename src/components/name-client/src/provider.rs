use std::net::IpAddr;
use serde::{Deserialize, Serialize};
use name_lib::*;

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


impl NameInfo {
    pub fn from_address(name:&str,address:IpAddr) -> Self {
        let ttl = 5*60;
        Self {name:name.to_string(),address:vec![address],cname:None,txt:None,did_document:None,proof_type:NameProof::None,create_time:0,ttl:Some(ttl)}
    }

    pub fn from_zone_config_str(name:&str,zone_config_str:&str) -> Self {
        let txt_string = format!("DID={};",zone_config_str);
        let ttl = 3600;
        Self {name:name.to_string(),address:vec![],cname:None,txt:Some(txt_string),did_document:None,proof_type:NameProof::None,create_time:0,ttl:Some(ttl)}
    }
}


#[async_trait::async_trait]
pub trait NSProvider: 'static + Send + Sync {
    fn get_id(&self) -> String;
    async fn query(&self, name: &str,record_type:Option<&str>) -> NSResult<NameInfo>;
    async fn query_did(&self, did: &str,fragment:Option<&str>) -> NSResult<EncodedDocument>;
}
