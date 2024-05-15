use crate::error::GatewayResult;
use super::tunnel::Tunnel;
use super::control::*;


use std::net::SocketAddr;
use std::sync::Arc;

#[async_trait::async_trait]
pub trait TunnelServerEvents: Send + Sync {
    async fn on_new_tunnel(&self, tunnel: Box<dyn Tunnel>) -> GatewayResult<()>;
}

pub type TunnelServerEventsRef = Arc<Box<dyn TunnelServerEvents>>;


// tunnel server used to accept tunnel connections from clients
#[derive(Clone)]
struct TunnelServer {
    addr: SocketAddr,
}

impl TunnelServer {
    pub fn new(addr: SocketAddr) -> Self {
        Self {
            addr,
        }
    }
}

impl TunnelServer {
   
}

#[async_trait::async_trait]
impl TunnelServerEvents for TunnelServer {
    async fn on_new_tunnel(&self, mut tunnel: Box<dyn Tunnel>) -> GatewayResult<()>  {
        let build_pkg = ControlPackageTransceiver::read_package(&mut tunnel).await?;
        match build_pkg.cmd {
            ControlCmd::Build => {
                todo!("Build tunnel");
            }
            _ => {
                error!("Invalid control command: {:?}", build_pkg.cmd);
            }
        }
        Ok(())
    }
}