use super::protocol::*;
use super::tcp::TcpTunnelServer;
use super::tunnel::{Tunnel, TunnelReader, TunnelWriter};
use crate::error::GatewayResult;

use std::net::SocketAddr;
use std::sync::Arc;

#[async_trait::async_trait]
pub trait TunnelServerEvents: Send + Sync {
    async fn on_new_tunnel(&self, tunnel: Box<dyn Tunnel>) -> GatewayResult<()>;
}

pub type TunnelServerEventsRef = Arc<Box<dyn TunnelServerEvents>>;

// tunnel server used to accept tunnel connections from clients
#[async_trait::async_trait]
pub trait TunnelServer: Send + Sync {
    fn bind_events(&self, events: TunnelServerEventsRef);
    async fn start(&self) -> GatewayResult<()>;
    async fn stop(&self) -> GatewayResult<()>;
}

pub type TunnelServerRef = Arc<Box<dyn TunnelServer>>;

pub struct TunnelServerCreator {}

impl TunnelServerCreator {
    pub fn new() -> Self {
        Self {}
    }

    pub fn create_tcp_tunnel_server(addr: SocketAddr) -> TunnelServerRef {
        let server = TcpTunnelServer::new(addr);
        Arc::new(Box::new(server))
    }

    pub fn create_default_tcp_tunnel_server() -> TunnelServerRef {
        let addr = "0.0.0.0:23558";
        let addr = addr.parse().unwrap();
        Self::create_tcp_tunnel_server(addr)
    }
}

pub struct TunnelInitInfo {
    pub pkg: ControlPackage,

    pub tunnel_reader: Box<dyn TunnelReader>,
    pub tunnel_writer: Box<dyn TunnelWriter>,
}
