#![allow(unused)]

use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;

use hickory_resolver::{config::*, Resolver};
use hickory_resolver::proto::rr::record_type;
use hickory_resolver::TokioAsyncResolver;

use crate::{NsProvider, NameInfo, NameProof, RecordType};
use name_lib::*;
pub struct DnsProvider {
    dns_server: Option<String>,
}

impl DnsProvider {
    pub fn new(dns_server: Option<String>) -> Self {
        Self {
            dns_server,
        }
    }

    // fn parse_dns_response(resp: DnsResponse) -> NSResult<NameInfo> {
    //     let mut txt_list = Vec::new();
    //     for record in resp.answers() {
    //         if record.record_type() == RecordType::TXT {
    //             let data = record.data();
    //             if data.is_some() {
    //                 let data = data.unwrap();
    //                 if let RData::TXT(txt) = data {
    //                     for txt in txt.txt_data() {
    //                         let txt = String::from_utf8_lossy(txt).to_string();
    //                         txt_list.push(txt);
    //                     }
    //                 }

    //             }
    //         }
    //     }
    //     if txt_list.len() == 0 {
    //         return Err(ns_err!(NSErrorCode::NotFound, "txt data is empty"));
    //     }

    //     let txt = DnsTxtCodec::decode(txt_list)?;
    //     return Ok(serde_json::from_str(txt.as_str()).map_err(into_ns_err!(NSErrorCode::InvalidData, "Failed to parse txt {}", txt))?);
    // }
   
}
#[async_trait::async_trait]
impl NsProvider for DnsProvider {
    fn get_id(&self) -> String {
        return "dns provider".to_string();
    }

    async fn query(&self, name: &str, record_type: Option<RecordType>, from_ip: Option<IpAddr>) -> NSResult<NameInfo> {
        let mut server_config = ResolverConfig::default();
        if self.dns_server.is_some() {
            let dns_server = self.dns_server.clone().unwrap();
            let name_server_configs = vec![NameServerConfig::new(
                SocketAddr::new(IpAddr::from_str(&dns_server).unwrap(), 53),
                Protocol::Udp,
            )];
            server_config = ResolverConfig::from_parts(
                None, 
                vec![], 
                name_server_configs,
            );
        }
        //let resolver2 = Resolver::new();
        let resolver = TokioAsyncResolver::tokio(server_config, ResolverOpts::default());
        //resolver.lookup(name, record_type)
        //for dns proivder,default record type is A.
        let record_type_str = record_type
            .map(|rt| rt.to_string())
            .unwrap_or_else(|| "A".to_string());

        match record_type.unwrap_or(RecordType::A) {
            RecordType::TXT => {
                let response = resolver.txt_lookup(name).await;
                if response.is_err() {
                    return Err(NSError::Failed(format!("lookup txt failed! {}",response.err().unwrap())));
                }
                let response = response.unwrap();
                let mut whole_txt = String::new();
                for record in response.iter() {
                    let txt = record.txt_data().iter().map(|s| -> String {
                        let byte_slice: &[u8] = &s;
                        return String::from_utf8_lossy(byte_slice).to_string();
                    }).collect::<Vec<String>>().join("");
                    whole_txt.push_str(&txt);
                }

                let name_info = NameInfo {
                    name: name.to_string(),
                    address: Vec::new(),
                    cname: None,
                    txt: Some(whole_txt),
                    did_document: None,
                    pk_x_list: None,
                    proof_type: NameProof::None,
                    create_time: 0,
                    ttl: None,
                };
                return Ok(name_info);
            },
            RecordType::A | RecordType::AAAA => {
                let response = resolver.lookup_ip(name).await;
                if response.is_err() {
                    return Err(NSError::Failed(format!("lookup ip failed! {}",response.err().unwrap())));
                }
                let response = response.unwrap();
                let mut addrs = Vec::new();
                for ip in response.iter() {
                    addrs.push(ip);
                }
                let name_info = NameInfo {
                    name: name.to_string(),
                    address:addrs,
                    cname: None,
                    txt: None,
                    did_document: None,
                    pk_x_list: None,
                    proof_type: NameProof::None,
                    create_time: 0,
                    ttl: None,
                };
                return Ok(name_info);
            },
            RecordType::DID => {
                let response = resolver.txt_lookup(name).await;
                if response.is_err() {
                    return Err(NSError::Failed(format!("lookup txt failed! {}",response.err().unwrap())));
                }
                let response = response.unwrap();
                //let mut did_tx:String;
                //let mut did_doc = DIDSimpleDocument::new();
                let mut pkx_list = Vec::new();
                let mut name_info = NameInfo {
                    name: name.to_string(),
                    address:Vec::new(),
                    cname: None,
                    txt: None,
                    did_document: None,
                    pk_x_list: None,
                    proof_type: NameProof::None,
                    create_time: 0,
                    ttl: None,
                }; 
                for record in response.iter() {
                    let txt = record.txt_data().iter().map(|s| -> String {
                        let byte_slice: &[u8] = &s;
                        return String::from_utf8_lossy(byte_slice).to_string();
                    }).collect::<Vec<String>>().join("");

                    if txt.starts_with("DID=") {
                        let did_payload = txt.trim_start_matches("DID=").trim_end_matches(";");
                        debug!("did_payload: {}",did_payload);

                        let did_doc = EncodedDocument::Jwt(did_payload.to_string());
                        name_info.did_document = Some(did_doc);
                    }
                   
                    if txt.starts_with("PKX=") {
                        let pkx = txt.trim_start_matches("PKX=").trim_end_matches(";");
                        pkx_list.push(pkx.to_string());
                    }
                }

                if name_info.did_document.is_none() {
                    return Err(NSError::Failed("DID Document not found".to_string()));
                }
                if pkx_list.len() > 0 {
                    debug!("pkx_list: {:?}",pkx_list);
                    name_info.pk_x_list = Some(pkx_list);  
                
                    //verify did_document by pkx_list
                    let jwt_str = name_info.did_document.as_ref().unwrap();
                    let owner_public_key = name_info.get_owner_pk();
                    if owner_public_key.is_none() {
                        return Err(NSError::Failed("Owner public key not found".to_string()));
                    }
                    let owner_public_key = owner_public_key.unwrap();
                    let zone_config = ZoneConfig::decode(&jwt_str, Some(&owner_public_key));
                    if zone_config.is_err() {
                        return Err(NSError::Failed("parse zone config failed!".to_string()));
                    }
                    info!("resolve & verify zone_config from {} TXT record OK.",name);
                }
                return Ok(name_info);
            },
            _ => {
                return Err(NSError::Failed(format!("Invalid record type: {:?}", record_type)));
            }
        }
        
    }

    async fn query_did(&self, did: &str, fragment: Option<&str>, from_ip: Option<IpAddr>) -> NSResult<EncodedDocument> {
        return Err(NSError::Failed("Not implemented".to_string()));
    }
}

