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
use name_lib::{DNSProvider, NSProvider, NameInfo};
use cyfs_gateway_lib::*;
use tokio::time::timeout;
use url::Url;
use std::time::Duration;

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
    #[error("IO error: {0:}")]
    Io(#[from] std::io::Error),
}

pub struct DnsServer {
    config : DNSServerConfig,
    resolver_chain: Vec<Box<dyn NSProvider>>,
}

pub fn create_ns_provider(provider_config: &DNSProviderConfig) -> Result<Box<dyn NSProvider>> {
    match provider_config.provider_type {
        DNSProviderType::DNS => {
            let dns_provider = DNSProvider::new(None);
            Ok(Box::new(dns_provider))
        },
        _ => {
            Err(anyhow::anyhow!("Unknown provider type: {:?}", provider_config.provider_type))
        }
    }
}

//TODO: dns_provider is realy a demo implementation, must refactor before  used in a offical server.
fn nameinfo_to_rdata(record_type:&str,name_info: &NameInfo) -> Result<RData> {
    match record_type {
        "A"=> {
            if name_info.address.len() < 1 {
                return Err(anyhow::anyhow!("Address is none"));
            }
            let addr = name_info.address[0];
            match addr {
                IpAddr::V4(addr) => {
                    return Ok(RData::A(A::from(addr)));
                },
                _ => {
                    return Err(anyhow::anyhow!("Address is not ipv4"));
                }
            }
        },
        "AAAA"=> {
            if name_info.address.len() < 1 {
                return Err(anyhow::anyhow!("Address is none"));
            }
            let addr = name_info.address[0];
            match addr {
                IpAddr::V6(addr) => {
                    return Ok(RData::AAAA(AAAA::from(addr)));
                },
                _ => {
                    return Err(anyhow::anyhow!("Address is not ipv6"));
                }
            }
        },
        "CNAME"=> {
            if name_info.cname.is_none() {
                return Err(anyhow::anyhow!("CNAME is none")); 
            }
            let cname = name_info.cname.clone().unwrap();
            return Ok(RData::CNAME(CNAME(Name::from_str(cname.as_str()).unwrap())));

        },
        "TXT"=> {
            if name_info.txt.is_none() {
                warn!("TXT is none");
                return Err(anyhow::anyhow!("TXT is none"));
            }
            let txt = name_info.txt.clone().unwrap();
            if txt.len() > 255 {
                warn!("TXT is too long, split it");
                let s1 = txt[0..254].to_string();
                let s2 = txt[254..].to_string();
                return Ok(RData::TXT(TXT::new(vec![s1,s2])));
            } else {
                return Ok(RData::TXT(TXT::new(vec![txt])));
            }            
        },
        _ => {
            return Err(anyhow::anyhow!("Unknown record type:{}", record_type));
        }
    }
}

impl DnsServer {
    pub fn new(config: DNSServerConfig) -> Result<Self> {
        let mut resolver_chain : Vec<Box<dyn NSProvider>> = Vec::new();
        config.resolver_chain.iter().try_for_each(|provider_config| {
            let provider = create_ns_provider(provider_config);
            if provider.is_err() {
                error!("Failed to create provider: {}", provider_config.config);
                Err(provider.err().unwrap())
            } else {
                resolver_chain.push(provider.unwrap());
                Ok(())
            }
        })?;

        Ok(DnsServer {
            config,
            resolver_chain,
        })
    }

    async fn handle_fallback<R: ResponseHandler> (
        &self, request: &Request,server_name:&str,
        mut response: R
    ) -> Result<ResponseInfo, Error> {
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
        let resp_vec =buf[0..resp_len].to_vec();

        unimplemented!("handle_fallback");
    }

    async fn do_handle_request<R: ResponseHandler>(
        &self,
        request: &Request,
        mut response: R,
    ) -> Result<ResponseInfo, Error> {
        // make sure the request is a query
        if request.op_code() != OpCode::Query {
            return Err(Error::InvalidOpCode(request.op_code()));
        }

        // make sure the message type is a query
        if request.message_type() != MessageType::Query {
            return Err(Error::InvalidMessageType(request.message_type()));
        }

        // WARN!!! 
        // Be careful to handle the request that may be delivered to the DNS-Server again to avoid the dead cycle

        let name = request.query().name().to_string();
        let record_type = request.query().query_type().to_string();
        info!("|==>DNS query name:{},record_type:{}", name,record_type);
        //foreach provider in resolver_chain 
        for provider in self.resolver_chain.iter() {
            let name_info = provider.query(name.as_str(),Some(record_type.as_str())).await;
            if name_info.is_err() {
                trace!("Provider {} can't resolve name:{}", provider.get_id(), name);
                continue;
            }
            //cover nameinfo to response
            let name_info = name_info.unwrap(); 
            let rdata = nameinfo_to_rdata(record_type.as_str(),&name_info);
            if rdata.is_err() {
                error!("Failed to convert nameinfo to rdata:{}", rdata.err().unwrap());
                continue;
            }

            let rdata = rdata.unwrap();
            let mut builder = MessageResponseBuilder::from_message_request(request);
            let mut header = Header::response_from_request(request.header());
            header.set_response_code(ResponseCode::NoError);

            let mut ttl = 600;
            if name_info.ttl.is_some() {
                ttl = name_info.ttl.unwrap();
            }
            let records = vec![Record::from_rdata(request.query().name().into(),ttl, rdata)];
            let mut message = builder.build(header,records.iter(),&[],&[],&[]);
            response.send_response(message).await;
            info!("<==|name:{} {} resolved by provider:{}", name, record_type,provider.get_id());
            //let mut response = message.into();
            return Ok(header.into());
        }

        info!("All providers can't resolve name:{} enter fallback", name);
        
        // for server_name in self.config.fallback.iter() {
        //     let resp_info = self.handle_fallback(request,server_name,response.clone()).await;
        //     if resp_info.is_ok() {
        //         return resp_info;
        //     }
        // }

        warn!("All providers can't resolve name:{} and fallback failed", name);
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
        match self.do_handle_request(request, response).await {
            Ok(info) => {
                info
            },
            Err(error) => {
                error!("Error in RequestHandler: {error}");
                let mut header = Header::new();
                header.set_response_code(ResponseCode::ServFail);
                header.into()
            }
        }  
    }
}

pub async fn start_dns_server(config:DNSServerConfig) -> anyhow::Result<()> {
    let bind_addr = config.bind.clone().unwrap_or("0.0.0.0".to_string());
    let addr = format!("{}:{}", bind_addr,config.port);
    info!("cyfs-dns-server bind at:{}", addr);
    let udp_socket = UdpSocket::bind(addr.clone()).await?;
    let handler = DnsServer::new(config)?;
    let mut server = ServerFuture::new(handler);
    server.register_socket(udp_socket);

    info!("cyfs-dns-server run at:{}", addr);
    server.block_until_done().await?;

    Ok(())
}
