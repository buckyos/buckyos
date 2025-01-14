use crate::ip::IPTunnelBuilder;
use crate::rtcp::RTCP_STACK_MANAGER;
use crate::{
    DatagramServerBox, StreamListener, TunnelBox, TunnelBuilder, TunnelError, TunnelResult,
    StreamProbe, StreamSelector,
};

use lazy_static::lazy_static;
use log::*;

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use url::Url;
use buckyos_kit::AsyncStream;
use crate::DatagramClientBox;
use crate::socks::SocksTunnelBuilder;

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
        "socks" => Ok(ProtocolCategory::Stream),
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
        "socks" => {
            let builder = SocksTunnelBuilder::new();
            Ok(Box::new(builder))
        }
        _ => {
            let msg = format!("Unknown protocol: {}", protocol);
            error!("{}", msg);
            Err(TunnelError::UnknownProtocol(msg))
        }
    }
}

pub fn get_stream_probe(_probe_id:&str) -> TunnelResult<Box<dyn StreamProbe + Send>> {
    unimplemented!()
}

pub fn get_stream_selector(_selector_id:&str) -> TunnelResult<Box<dyn StreamSelector + Send>> {
    unimplemented!()
}

lazy_static! {
    static ref TUNNEL_MAP: Arc<Mutex<HashMap<String, Box<dyn TunnelBox>>>> =
        Arc::new(Mutex::new(HashMap::new()));
}

pub async fn get_tunnel(
    target_url: &Url,
    _enable_tunnel: Option<Vec<String>>,
) -> TunnelResult<Box<dyn TunnelBox>> {

    let builder = get_tunnel_builder_by_protocol(target_url.scheme()).await?;
    let tunnel = builder.create_tunnel(target_url.host_str()).await?;

    info!("Get tunnel for {} success", target_url);
    return Ok(tunnel);
}

pub async fn create_listener_by_url(bind_url: &Url) -> TunnelResult<Box<dyn StreamListener>> {
    let builder = get_tunnel_builder_by_protocol(bind_url.scheme()).await?;
    let listener = builder.create_stream_listener(bind_url).await?;
    return Ok(listener);
}

pub async fn create_datagram_server_by_url(
    bind_url: &Url,
) -> TunnelResult<Box<dyn DatagramServerBox>> {
    let builder = get_tunnel_builder_by_protocol(bind_url.scheme()).await?;
    let server = builder.create_datagram_server(bind_url).await?;
    return Ok(server);
}
//$tunnel_schema://$tunnel_stack_id/$target_stream_id
pub async fn open_stream_by_url(url: &Url) -> TunnelResult<Box<dyn AsyncStream>> {
    let builder = get_tunnel_builder_by_protocol(url.scheme()).await?;
    let auth_str = url.authority();
    let tunnel;
    if auth_str.is_empty() {
        tunnel = builder.create_tunnel(None).await?;
    } else {
        tunnel = builder.create_tunnel(Some(auth_str)).await?;
    }

    let stream = tunnel.open_stream(url.path()).await
        .map_err(|e| {
            error!("Open stream by url failed: {}", e);
            TunnelError::ConnectError(format!("Open stream by url failed: {}", e))
        })?;
    
    return Ok(stream);
}

pub async fn create_datagram_client_by_url(url: &Url) -> TunnelResult<Box<dyn DatagramClientBox>> {
    let builder = get_tunnel_builder_by_protocol(url.scheme()).await?;
    let tunnel = builder.create_tunnel(url.host_str()).await?;
    let client = tunnel.create_datagram_client(url.path()).await
        .map_err(|e| {
            error!("Create datagram client by url failed: {}", e);
            TunnelError::ConnectError(format!("Create datagram client by url failed: {}", e))
        })?;
    return Ok(client);
}