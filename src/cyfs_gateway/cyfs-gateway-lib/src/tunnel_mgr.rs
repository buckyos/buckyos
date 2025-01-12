use crate::ip::IPTunnelBuilder;
use crate::rtcp::RTCP_STACK_MANAGER;
use crate::{
    DatagramServerBox, StreamListener, TunnelBox, TunnelBuilder, TunnelError, TunnelResult,
};

use lazy_static::lazy_static;
use log::*;

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use url::Url;

#[derive(Debug, PartialEq, Eq)]
pub enum ProtocolCategory {
    Stream,
    Datagram,
    //Named Object
}

pub fn get_protocol_category(str_protocol: &str) -> TunnelResult<ProtocolCategory> {
    //lowercase
    let str_protocol = str_protocol.to_lowercase();
    match str_protocol.as_str() {
        "tcp" => Ok(ProtocolCategory::Stream),
        "rtcp" => Ok(ProtocolCategory::Stream),
        "udp" => Ok(ProtocolCategory::Datagram),
        "rudp" => Ok(ProtocolCategory::Datagram),
        _ => {
            let msg = format!("Unknow protocol: {}", str_protocol);
            error!("{}", msg);
            Err(TunnelError::UnknownProtocol(msg))
        }
    }
}

pub async fn get_tunnel_builder_by_protocol(
    protocol: &str,
) -> TunnelResult<Box<dyn TunnelBuilder>> {
    match protocol {
        "tcp" => return Ok(Box::new(IPTunnelBuilder::new())),
        "udp" => return Ok(Box::new(IPTunnelBuilder::new())),
        "rtcp" => {
            let stack = RTCP_STACK_MANAGER.get_current_device_stack().await?;
            Ok(Box::new(stack))
        }
        "rudp" => {
            let stack = RTCP_STACK_MANAGER.get_current_device_stack().await?;
            Ok(Box::new(stack))
        }
        _ => {
            let msg = format!("Unknow protocol: {}", protocol);
            error!("{}", msg);
            Err(TunnelError::UnknownProtocol(msg))
        }
    }
}

lazy_static! {
    static ref TUNNEL_MAP: Arc<Mutex<HashMap<String, Box<dyn TunnelBox>>>> =
        Arc::new(Mutex::new(HashMap::new()));
}

pub async fn get_tunnel(
    target_url: &Url,
    _enable_tunnel: Option<Vec<String>>,
) -> TunnelResult<Box<dyn TunnelBox>> {
    //url like tcp://deviceid
    let builder = get_tunnel_builder_by_protocol(target_url.scheme()).await?;
    let tunnel = builder.create_tunnel(target_url).await?;

    info!("Get tunnel for {} success", target_url);
    return Ok(tunnel);
}

pub async fn create_listener_by_url(bind_url: &Url) -> TunnelResult<Box<dyn StreamListener>> {
    let builder = get_tunnel_builder_by_protocol(bind_url.scheme()).await?;
    let listener = builder.create_listener(bind_url).await?;
    return Ok(listener);
}

pub async fn create_datagram_server_by_url(
    bind_url: &Url,
) -> TunnelResult<Box<dyn DatagramServerBox>> {
    let builder = get_tunnel_builder_by_protocol(bind_url.scheme()).await?;
    let server = builder.create_datagram_server(bind_url).await?;
    return Ok(server);
}
