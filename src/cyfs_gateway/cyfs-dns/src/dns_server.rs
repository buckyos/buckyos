use std::f32::consts::E;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;

use async_trait::async_trait;

use log::trace;
use log::{debug, error, info, warn};
use rdata::{CNAME,TXT,A,AAAA};
use tokio::net::UdpSocket;
use hickory_server::proto::op::*;
use hickory_server::proto::rr::*;
use hickory_server::server::{Request, RequestHandler, ResponseHandler, ResponseInfo};
use hickory_server::ServerFuture;
use hickory_server::authority::{Catalog, MessageRequest, MessageResponse, MessageResponseBuilder};
use hickory_proto::serialize::binary::{BinEncodable,BinDecodable};

use anyhow::Result;
use name_client::{DnsProvider, NsProvider, NameInfo, RecordType};
use cyfs_gateway_lib::*;
use tokio::time::timeout;
use url::Url;
use std::time::Duration;
use cyfs_sn::get_sn_server_by_id;
use futures::stream::{self, StreamExt};
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Name not found: {0:}")]
    NameNotFound(String),
    #[error("Invalid OpCode {0:}")]
    InvalidOpCode(OpCode),
    #[error("Invalid MessageType {0:}")]
    InvalidMessageType(MessageType),
    #[error("Invalid Zone {0:}")]
    InvalidZone(LowerName),
    #[error("Invalid RecordType {0:}")]
    InvalidRecordType(String),
    #[error("IO error: {0:}")]
    Io(#[from] std::io::Error),
}

pub struct DnsServer {
    config : DNSServerConfig,
    resolver_chain: Vec<Box<dyn NsProvider>>,
}

pub async fn create_ns_provider(provider_config: &DNSProviderConfig) -> Result<Box<dyn NsProvider>> {
    match provider_config.provider_type {
        DNSProviderType::DNS => {
            let dns_provider = DnsProvider::new(None);
            Ok(Box::new(dns_provider))
        },
        DNSProviderType::SN => {
            let sn_server_id = provider_config.config.get("server_id");
            if sn_server_id.is_none() {
                error!("server_id is none");
                return Err(anyhow::anyhow!("server_id is none"));
            }
            let sn_server_id = sn_server_id.unwrap();
            let sn_server_id = sn_server_id.as_str();
            if sn_server_id.is_none() {
                error!("server_id is none");
                return Err(anyhow::anyhow!("server_id is none"));
            }
            let sn_server_id = sn_server_id.unwrap();
            let sn_server = get_sn_server_by_id(sn_server_id).await;
            //let sn_server = SNServer::new(sn_server_id);
            if sn_server.is_none() {
                error!("sn_server not found:{}", sn_server_id);
                return Err(anyhow::anyhow!("sn_server not found:{}", sn_server_id));
            }
            let sn_server = sn_server.unwrap();
            Ok(Box::new(sn_server))
        },
        _ => {
            Err(anyhow::anyhow!("Unknown provider type: {:?}", provider_config.provider_type))
        }
    }
}

//TODO: dns_provider is realy a demo implementation, must refactor before  used in a offical server.
fn nameinfo_to_rdata(record_type:&str, name_info: &NameInfo) -> Result<Vec<RData>> {
    match record_type {
        "A"=> {
            if name_info.address.is_empty() {
                return Err(anyhow::anyhow!("Address is none"));
            }
            
            let mut records = Vec::new();
            // Convert all IPv4 addresses to A records
            for addr in name_info.address.iter() {
                match addr {
                    IpAddr::V4(addr) => {
                        records.push(RData::A(A::from(*addr)));
                    },
                    _ => {
                        debug!("Skipping non-IPv4 address");
                        continue;
                    }
                }
            }
            
            if records.is_empty() {
                return Err(anyhow::anyhow!("No valid IPv4 addresses found"));
            }
            Ok(records)
        },
        "AAAA"=> {
            if name_info.address.is_empty() {
                return Err(anyhow::anyhow!("Address is none"));
            }
            let mut records = Vec::new();
            // Convert all IPv6 addresses to AAAA records
            for addr in name_info.address.iter() {
                match addr {
                    IpAddr::V6(addr) => {
                        records.push(RData::AAAA(AAAA::from(*addr)));
                    },
                    _ => {
                        debug!("Skipping non-IPv6 address");
                        continue;
                    }
                }
            }
            
            if records.is_empty() {
                return Err(anyhow::anyhow!("No valid IPv6 addresses found"));
            }
            Ok(records)
        },
        "CNAME"=> {
            if name_info.cname.is_none() {
                return Err(anyhow::anyhow!("CNAME is none")); 
            }
            let cname = name_info.cname.clone().unwrap();
            let mut records = Vec::new();
            records.push(RData::CNAME(CNAME(Name::from_str(cname.as_str()).unwrap())));
            return Ok(records);

        },
        "TXT"=> {
            if name_info.txt.is_none() {
                warn!("TXT is none");
                return Err(anyhow::anyhow!("TXT is none"));
            }
            let txt = name_info.txt.clone().unwrap();
            let mut records = Vec::new();
            if txt.len() > 255 {
                warn!("TXT is too long, split it");
                let s1 = txt[0..254].to_string();
                let s2 = txt[254..].to_string();
                
                records.push(RData::TXT(TXT::new(vec![s1,s2])));
            } else {
                records.push(RData::TXT(TXT::new(vec![txt])));
            }    
            return Ok(records);        
        },
        _ => {
            return Err(anyhow::anyhow!("Unknown record type:{}", record_type));
        }
    }
}

impl DnsServer {
    pub async fn new(config: DNSServerConfig) -> Result<Self> {
        let mut resolver_chain : Vec<Box<dyn NsProvider>> = Vec::new();

        for provider_config in config.resolver_chain.iter() {
            let provider = create_ns_provider(provider_config).await;
            if provider.is_err() {
                error!("Failed to create provider: {}", provider_config.config);
            } else {
                resolver_chain.push(provider.unwrap());
            }
        }

        Ok(DnsServer {
            config,
            resolver_chain,
        })
    }

    async fn handle_fallback<R: ResponseHandler> (
        &self, request: &Request,server_name:&str,
        mut response: R
    ) -> Result<Message, Error> {
        let message = request.to_bytes();
        let message = message.unwrap();
        let socket = UdpSocket::bind("0.0.0.0:0").await?;
        let target_url = Url::parse(server_name);
        if target_url.is_err() {
            return Err(Error::NameNotFound("".to_string()));
        }
        let target_url = target_url.unwrap();
        let host = target_url.host_str().unwrap(); 
        let port = target_url.port().unwrap_or(53);
        let target_addr = SocketAddr::new(IpAddr::from_str(host).unwrap(), port);
        socket.send_to(&message, target_addr).await?;
        let mut buf = [0u8; 2048]; 
        let mut resp_len = 512;
        let proxy_result = timeout(Duration::from_secs(5), 
            socket.recv_from(&mut buf)).await;
        //let resp_vec =buf[0..resp_len].to_vec();
        let resp_message = Message::from_vec(&buf[0..resp_len]);
        if resp_message.is_err() {
            return Err(Error::NameNotFound("".to_string()));
        }
        let resp_message = resp_message.unwrap();
        let resp_info = resp_message.into();
        return Ok(resp_info);
        //unimplemented!("handle_fallback");
    }

    async fn do_handle_request<R: ResponseHandler>(
        &self,
        request: &Request,
        mut response: R
    ) -> Result<ResponseInfo, Error> {

        // make sure the request is a query
        if request.op_code() != OpCode::Query {
            return Err(Error::InvalidOpCode(request.op_code()));
        }

        // make sure the message type is a query
        if request.message_type() != MessageType::Query {
            return Err(Error::InvalidMessageType(request.message_type()));
        }

        let from_ip = request.src().ip();
    
        // WARN!!! 
        // Be careful to handle the request that may be delivered to the DNS-Server again to avoid the dead cycle
        
        let name = request.query().name().to_string();
        let record_type_str = request.query().query_type().to_string();
        let record_type = RecordType::from_str(&record_type_str)
            .ok_or_else(|| Error::InvalidRecordType(record_type_str))?;

        info!("|==>DNS query name:{}, record_type:{:?}", name, record_type);

        for provider in self.resolver_chain.iter() {
            let name_info = provider.query(
                name.as_str(),
                Some(record_type.clone()),
                Some(from_ip)
            ).await;
            if name_info.is_err() {
                trace!("Provider {} can't resolve name:{}", provider.get_id(), name);
                continue;
            }

            let name_info = name_info.unwrap();
            let rdata_vec = nameinfo_to_rdata(record_type.to_string().as_str(),&name_info);
            if rdata_vec.is_err() {
                error!("Failed to convert nameinfo to rdata:{}", rdata_vec.err().unwrap());
                continue;
            }

            let rdata_vec = rdata_vec.unwrap();
            let mut builder = MessageResponseBuilder::from_message_request(request);
            let mut header = Header::response_from_request(request.header());
            header.set_response_code(ResponseCode::NoError);

            let mut ttl = name_info.ttl.unwrap_or(600);
            let records = rdata_vec.into_iter().map(|rdata| Record::from_rdata(request.query().name().into(), ttl, rdata)).collect::<Vec<_>>();
            let mut message = builder.build(header,records.iter(),&[],&[],&[]);
            response.send_response(message).await;
            info!("<==|name:{} {} resolved by provider:{}", name, record_type.to_string(),provider.get_id());
            //let mut response = message.into();
            return Ok(header.into());
        }

        
        if let Some(server_name) = self.config.this_name.as_ref() {
            if !name.ends_with(server_name.as_str()) {
                info!("All providers can't resolve name:{} enter fallback", name);
                // for server_name in self.config.fallback.iter() {
                //     let resp_message = self.handle_fallback(request,server_name,response.clone()).await;
                //     if resp_message.is_ok() {
                        
                //         return resp_info;
                //     }
                // }
            }
        }


        warn!("All providers can't resolve name:{}", name);
        return Err(Error::NameNotFound("".to_string()));
    }
}

#[async_trait]
impl RequestHandler for DnsServer {
    async fn handle_request<R: ResponseHandler>(
        &self,
        request: &Request,
        response: R,
    ) -> ResponseInfo {
        // try to handle request
        let mut resp2 = response.clone();
        match self.do_handle_request(request, response).await {
            Ok(info) => {
                info
            },
            Err(error) => {
                error!("Error in RequestHandler: {error}");
                let mut builder = MessageResponseBuilder::from_message_request(request);
                let mut header = Header::response_from_request(request.header());
                header.set_response_code(ResponseCode::NXDomain);
                let records = vec![];
                let mut message = builder.build(header,records.iter(),&[],&[],&[]);
                resp2.send_response(message).await;
                header.into()
            }
        }  
    }
}

pub async fn start_cyfs_dns_server(config:DNSServerConfig) -> anyhow::Result<()> {
    let bind_addr = config.bind.clone().unwrap_or("0.0.0.0".to_string());
    let addr = format!("{}:{}", bind_addr,config.port);
    info!("cyfs-dns-server try bind at:{}", addr);
    let udp_socket = UdpSocket::bind(addr.clone()).await?;
    let handler = DnsServer::new(config).await?;
    let mut server = ServerFuture::new(handler);
    server.register_socket(udp_socket);

    info!("cyfs-dns-server run at:{}", addr);
    server.block_until_done().await?;

    Ok(())
}
