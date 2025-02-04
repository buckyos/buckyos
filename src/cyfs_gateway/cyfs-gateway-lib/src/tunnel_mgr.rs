use crate::ip::IPTunnelBuilder;
use crate::socks::SocksTunnelBuilder;
use crate::DatagramClientBox;
use crate::{
    DatagramServerBox, GatewayDeviceRef, RTcpStackManager, StreamListener, StreamProbe,
    StreamSelector, TunnelBox, TunnelBuilder, TunnelError, TunnelResult,
};
use buckyos_kit::AsyncStream;
use log::*;
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
        "socks" => Ok(ProtocolCategory::Stream),
        _ => {
            let msg = format!("Unknown protocol: {}", str_protocol);
            error!("{}", msg);
            Err(TunnelError::UnknownProtocol(msg))
        }
    }
}

#[derive(Clone)]
pub struct TunnelManager {
    device: GatewayDeviceRef,
    rtcp_stack_manager: RTcpStackManager,
}

impl TunnelManager {
    pub fn new(device: GatewayDeviceRef) -> Self {
        Self {
            device: device.clone(),
            rtcp_stack_manager: RTcpStackManager::new(device),
        }
    }

    pub async fn get_tunnel_builder_by_protocol(
        &self,
        protocol: &str,
    ) -> TunnelResult<Box<dyn TunnelBuilder>> {
        match protocol {
            "tcp" => return Ok(Box::new(IPTunnelBuilder::new())),
            "udp" => return Ok(Box::new(IPTunnelBuilder::new())),
            "rtcp" => {
                let stack = self.rtcp_stack_manager.get_current_device_stack().await?;
                Ok(Box::new(stack))
            }
            "rudp" => {
                let stack = self.rtcp_stack_manager.get_current_device_stack().await?;
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

    pub fn get_stream_probe(&self, _probe_id: &str) -> TunnelResult<Box<dyn StreamProbe + Send>> {
        unimplemented!()
    }

    pub fn get_stream_selector(
        &self,
        _selector_id: &str,
    ) -> TunnelResult<Box<dyn StreamSelector + Send>> {
        unimplemented!()
    }

    pub async fn get_tunnel(
        &self,
        target_url: &Url,
        _enable_tunnel: Option<Vec<String>>,
    ) -> TunnelResult<Box<dyn TunnelBox>> {
        let builder = self
            .get_tunnel_builder_by_protocol(target_url.scheme())
            .await?;
        let tunnel = builder.create_tunnel(target_url.host_str()).await?;

        info!("Get tunnel for {} success", target_url);
        return Ok(tunnel);
    }

    pub async fn create_listener_by_url(
        &self,
        bind_url: &Url,
    ) -> TunnelResult<Box<dyn StreamListener>> {
        let builder = self
            .get_tunnel_builder_by_protocol(bind_url.scheme())
            .await?;
        let listener = builder.create_stream_listener(bind_url).await?;
        return Ok(listener);
    }

    pub async fn create_datagram_server_by_url(
        &self,
        bind_url: &Url,
    ) -> TunnelResult<Box<dyn DatagramServerBox>> {
        let builder = self
            .get_tunnel_builder_by_protocol(bind_url.scheme())
            .await?;
        let server = builder.create_datagram_server(bind_url).await?;
        return Ok(server);
    }

    //$tunnel_schema://$tunnel_stack_id/$target_stream_id
    pub async fn open_stream_by_url(&self, url: &Url) -> TunnelResult<Box<dyn AsyncStream>> {
        let builder = self.get_tunnel_builder_by_protocol(url.scheme()).await?;
        let auth_str = url.authority();
        let tunnel;
        if auth_str.is_empty() {
            tunnel = builder.create_tunnel(None).await?;
        } else {
            tunnel = builder.create_tunnel(Some(auth_str)).await?;
        }

        let stream = tunnel.open_stream(url.path()).await.map_err(|e| {
            error!("Open stream by url failed: {}", e);
            TunnelError::ConnectError(format!("Open stream by url failed: {}", e))
        })?;

        return Ok(stream);
    }

    pub async fn create_datagram_client_by_url(
        &self,
        url: &Url,
    ) -> TunnelResult<Box<dyn DatagramClientBox>> {
        let builder = self.get_tunnel_builder_by_protocol(url.scheme()).await?;
        let tunnel = builder.create_tunnel(url.host_str()).await?;
        let client = tunnel
            .create_datagram_client(url.path())
            .await
            .map_err(|e| {
                error!("Create datagram client by url failed: {}", e);
                TunnelError::ConnectError(format!("Create datagram client by url failed: {}", e))
            })?;
        return Ok(client);
    }
}
