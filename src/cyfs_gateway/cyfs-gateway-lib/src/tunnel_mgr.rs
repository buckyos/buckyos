
use crate::{DatagramServer, DatagramServerBox,StreamListener,TunnelBuilder, Tunnel,TunnelBox,TunnelError, TunnelResult};
use serde_json::Value;
use url::Url;
use std::collections::HashMap;
use std::sync::{Arc};
use tokio::sync::Mutex;
use lazy_static::lazy_static;
use log::*;

#[derive(Debug,PartialEq, Eq)]
pub enum ProtocolCategory {
    Stream,
    Datagram,
    //Named Object
}

pub fn get_protocol_category(str_protocol:&str) -> TunnelResult<ProtocolCategory> {
    //lowercase
    let str_protocol = str_protocol.to_lowercase();
    match str_protocol.as_str() {
        "tcp" => Ok(ProtocolCategory::Stream),
        "rtcp" => Ok(ProtocolCategory::Stream),
        "udp" => Ok(ProtocolCategory::Datagram),
        _ => Err(TunnelError::UnknowProtocol(str_protocol)),
    }
}

pub fn get_tunnel_builder_by_protocol(protocol:&str) -> TunnelResult<Box<dyn TunnelBuilder>> {
    match protocol {
        "tcp" => {
            return Ok(Box::new(crate::IPTunnelBuilder::new()))
        },
        "udp" => {
            return Ok(Box::new(crate::IPTunnelBuilder::new()))
        },
        "rtcp" => {
            return Ok(Box::new(crate::RTcpTunnelBuilder::new("cyfs_gateway".to_string(),2980)))
        }
        _ => return Err(TunnelError::UnknowProtocol(protocol.to_string()))
    }
}

lazy_static!{
    static ref TUNNEL_MAP:Arc<Mutex<HashMap<String,Box<dyn TunnelBox>>>> = {
        Arc::new(Mutex::new(HashMap::new()))
    };
}

pub async fn get_tunnel(target_url:&Url,enable_tunnel:Option<Vec<String>>) 
    -> TunnelResult<Box<dyn TunnelBox>> 
{
    let mut all_tunnel = TUNNEL_MAP.lock().await;
    let tunnel = all_tunnel.get(target_url.to_string().as_str());
    if tunnel.is_some() {
        return Ok(tunnel.unwrap().clone());
    }
    info!("try create tunnel for {}", target_url);
    //url like tcp://deviceid 
    let builder = get_tunnel_builder_by_protocol(target_url.scheme())?;
    let tunnel = builder.create_tunnel(target_url).await?;
    all_tunnel.insert(target_url.to_string(),tunnel.clone());
    info!("create tunnel for {} success,add to tunnel cache", target_url);
    return Ok(tunnel);
}


pub async fn create_listner_by_url(bind_url:&Url) -> TunnelResult<Box<dyn StreamListener>> {
    let builder = get_tunnel_builder_by_protocol(bind_url.scheme())?;
    let listener = builder.create_listener(bind_url).await?;
    return Ok(listener);
}

pub async fn create_datagram_server_by_url(bind_url:&Url) -> TunnelResult<Box<dyn DatagramServerBox>> {
    let builder = get_tunnel_builder_by_protocol(bind_url.scheme())?;
    let server = builder.create_datagram_server(bind_url).await?;
    return Ok(server);
}
