use std::net::IpAddr;
use jsonwebtoken::{jwk::Jwk, DecodingKey};
use serde::{Deserialize, Serialize};
use name_lib::*;
use serde_json::json;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum RecordType {
    A,      // IPv4 address
    AAAA,   // IPv6 address
    CNAME,  // Alias record
    TXT,    // Text record
    DID,    // DID Document
    SRV,    // Service record
    MX,     // Mail exchange record
    NS,     // Name server record
    PTR,    // Pointer record
    SOA,    // Start of authority record
}


impl Default for RecordType {
    fn default() -> Self {
        RecordType::A
    }
}

impl RecordType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "A" => Some(RecordType::A),
            "AAAA" => Some(RecordType::AAAA), 
            "CNAME" => Some(RecordType::CNAME),
            "TXT" => Some(RecordType::TXT),
            "DID" => Some(RecordType::DID),
            "SRV" => Some(RecordType::SRV),
            "MX" => Some(RecordType::MX),
            "NS" => Some(RecordType::NS),
            "PTR" => Some(RecordType::PTR),
            "SOA" => Some(RecordType::SOA),
            _ => None
        }
    }

    pub fn to_string(&self) -> String {
        match self {
            RecordType::A => "A",
            RecordType::AAAA => "AAAA",
            RecordType::CNAME => "CNAME", 
            RecordType::TXT => "TXT",
            RecordType::DID => "DID",
            RecordType::SRV => "SRV",
            RecordType::MX => "MX",
            RecordType::NS => "NS",
            RecordType::PTR => "PTR",
            RecordType::SOA => "SOA",
        }.to_string()
    }
}

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
    pub pk_x_list:Option<Vec<String>>,
    pub proof_type:NameProof,
    pub create_time: u64,
    pub ttl: Option<u32>,
}


impl NameInfo {
    pub fn from_address(name:&str,address:IpAddr) -> Self {
        let ttl = 5*60;
        Self {name:name.to_string(),address:vec![address],cname:None,txt:None,
            did_document:None,pk_x_list:None,proof_type:NameProof::None,create_time:0,ttl:Some(ttl)}
    }

    pub fn from_address_vec(name:&str,address_vec:Vec<IpAddr>) -> Self {
        let ttl = 5*60;
        Self {name:name.to_string(),address:address_vec,cname:None,txt:None,
            did_document:None,pk_x_list:None,proof_type:NameProof::None,create_time:0,ttl:Some(ttl)}
    }

    pub fn from_zone_config_str(name:&str,zone_config_str:&str) -> Self {
        let txt_string = format!("DID={};",zone_config_str);
        let ttl = 3600;
        Self {name:name.to_string(),address:vec![],cname:None,txt:Some(txt_string),
            did_document:None,pk_x_list:None,proof_type:NameProof::None,create_time:0,ttl:Some(ttl)}
    }

     pub fn get_owner_pk(&self) -> Option<Jwk> {
        if self.pk_x_list.is_some() {
            let pkx_list = self.pk_x_list.as_ref().unwrap();
            for pkx in pkx_list {
                if pkx.starts_with("0:") {
                    let pkx = pkx.split(":").nth(1).unwrap();
                    let public_key_jwk = json!({
                        "kty": "OKP",
                        "crv": "Ed25519",
                        "x": pkx,
                    });
                    let public_key_jwk = serde_json::from_value(public_key_jwk);
                    if public_key_jwk.is_err() {
                        error!("parse public key jwk failed! {}",public_key_jwk.err().unwrap());
                        return None;
                    }
                    let public_key_jwk : jsonwebtoken::jwk::Jwk = public_key_jwk.unwrap();
                    return Some(public_key_jwk);


                }
            }
        }
        None
     }

}


#[async_trait::async_trait]
pub trait NsProvider: 'static + Send + Sync {
    fn get_id(&self) -> String;
    async fn query(&self, name: &str, record_type: Option<RecordType>, from_ip: Option<IpAddr>) -> NSResult<NameInfo>;
    async fn query_did(&self, did: &str, fragment: Option<&str>, from_ip: Option<IpAddr>) -> NSResult<EncodedDocument>;
}

#[async_trait::async_trait]
pub trait NsUpdateProvider: 'static + Send + Sync {
    async fn update(&self, record_type: RecordType, record: NameInfo) -> NSResult<NameInfo>;
    async fn delete(&self, name: &str, record_type: RecordType) -> NSResult<Option<NameInfo>>;
}
