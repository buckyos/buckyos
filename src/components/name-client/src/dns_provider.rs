#![allow(unused)]

use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;

use hickory_resolver::{config::*, Resolver};
use hickory_resolver::proto::rr::record_type;
use hickory_resolver::TokioAsyncResolver;
use jsonwebtoken::DecodingKey;

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

    pub fn new_with_config(config: serde_json::Value) -> NSResult<Self> {
        let dns_server = config.get("dns_server");
        if dns_server.is_some() {
            let dns_server = dns_server.unwrap().as_str();
            return Ok( Self {
                dns_server : dns_server.map(|s| s.to_string())
            }) 
        }
        Ok(Self {
            dns_server: None,
        })
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
        let resolver;
        if self.dns_server.is_some() {
            let dns_server = self.dns_server.clone().unwrap();
            let dns_ip_addr = IpAddr::from_str(&dns_server)
                .map_err(|e| NSError::ReadLocalFileError(format!("Invalid dns server: {}", e)))?;
            let name_server_configs = vec![NameServerConfig::new(
                SocketAddr::new(dns_ip_addr, 53),
                Protocol::Udp,
            )];
            server_config = ResolverConfig::from_parts(
                None, 
                vec![], 
                name_server_configs,
            );
            resolver = TokioAsyncResolver::tokio(server_config, ResolverOpts::default());
        } else {
            resolver = TokioAsyncResolver::tokio_from_system_conf().unwrap();
        }
        info!("dns query: {}",name);
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
                    warn!("lookup {} txt record failed! {}",name,response.err().unwrap());
                    return Err(NSError::Failed(format!("lookup txt failed! {}",name)));
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
                    let public_key_jwk = owner_public_key.unwrap();
                    let public_key = DecodingKey::from_jwk(&public_key_jwk);
                    if public_key.is_err() {
                        error!("parse public key failed! {}",public_key.err().unwrap());
                        return Err(NSError::Failed("parse public key failed! ".to_string()));
                    }
                    let public_key = public_key.unwrap();

                    let mut zone_boot_config = ZoneBootConfig::decode(&jwt_str, Some(&public_key));
                    if zone_boot_config.is_err() {
                        return Err(NSError::Failed("parse zone boot config failed!".to_string()));
                    }
                    
                    let mut zone_boot_config = zone_boot_config.unwrap();
                    zone_boot_config.owner_key = Some(public_key_jwk);
                    zone_boot_config.id = Some(DID::from_str(name).unwrap());
                    let gateway_devs = name_info.get_gateway_device_list();
                    if gateway_devs.is_some() {
                        zone_boot_config.gateway_devs =  gateway_devs.unwrap();
                    }
         
                    info!("resolve & verify zone_boot_config from {} TXT record OK.",name);
                    let zone_boot_config_value = serde_json::to_value(&zone_boot_config).unwrap();
                    //info!("zone_boot_config_value: {:?}",zone_boot_config_value);
                    name_info.did_document = Some(EncodedDocument::JsonLd(zone_boot_config_value));
                }
                return Ok(name_info);
            },
            _ => {
                return Err(NSError::Failed(format!("Invalid record type: {:?}", record_type)));
            }
        }
        
    }

    async fn query_did(&self, did: &DID, fragment: Option<&str>, from_ip: Option<IpAddr>) -> NSResult<EncodedDocument> {
        info!("query_did: {}",did.to_string());
        let name_info = self.query(&did.to_host_name(), Some(RecordType::DID), None).await?;
        if name_info.did_document.is_some() {
            return Ok(name_info.did_document.unwrap());
        }
        return Err(NSError::Failed("DID Document not found".to_string()));
    }
}

