#![allow(unused)]

use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;

use hickory_resolver::{config::*, Resolver};
use hickory_resolver::proto::rr::record_type;
use hickory_resolver::TokioAsyncResolver;

use crate::{NSProvider, NameInfo, NameProof};
use name_lib::*;
pub struct DNSProvider {
    dns_server: Option<String>,
}

impl DNSProvider {
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
impl NSProvider for DNSProvider {
    fn get_id(&self) -> String {
        return  "dns provider".to_string();
    }
    async fn query(&self, name: &str,record_type:Option<&str>) -> NSResult<NameInfo> {
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
        let record_type = record_type.unwrap_or("A");
        match record_type {
            "TXT" => {
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
                    proof_type: NameProof::None,
                    create_time: 0,
                    ttl: None,
                };
                return Ok(name_info);
            },
            "A" | "AAAA" => {
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
                    proof_type: NameProof::None,
                    create_time: 0,
                    ttl: None,
                };
                return Ok(name_info);
            },
            "DID"=>{
                let response = resolver.txt_lookup(name).await;
                if response.is_err() {
                    return Err(NSError::Failed(format!("lookup txt failed! {}",response.err().unwrap())));
                }
                let response = response.unwrap();
                //let mut did_tx:String;
                //let mut did_doc = DIDSimpleDocument::new();

                for record in response.iter() {
                    let txt = record.txt_data().iter().map(|s| -> String {
                        let byte_slice: &[u8] = &s;
                        return String::from_utf8_lossy(byte_slice).to_string();
                    }).collect::<Vec<String>>().join("");

                    if txt.starts_with("DID=") {
                        let did_payload = txt.trim_start_matches("DID=").trim_end_matches(";");
                        println!("did_payload: {}",did_payload);

                        let did_doc = EncodedDocument::Jwt(did_payload.to_string());
                        let name_info = NameInfo {
                            name: name.to_string(),
                            address:Vec::new(),
                            cname: None,
                            txt: None,
                            did_document: Some(did_doc),
                            proof_type: NameProof::None,
                            create_time: 0,
                            ttl: None,
                        }; 
                        return Ok(name_info);
                    }
                }
                return Err(NSError::Failed("DID not found".to_string()));
            },
            _ => {
                //resolver.lookup(name, record_type).await;
                return Err(NSError::Failed(format!("Invalid record type: {}", record_type)));
            }
        }
        
    }

    async fn query_did(&self, did: &str,fragment:Option<&str>) -> NSResult<EncodedDocument> {
        return Err(NSError::Failed("Not implemented".to_string()));
    }
}

