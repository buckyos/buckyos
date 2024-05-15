use super::tunnel::Tunnel;
use crate::error::*;
use tokio::io;

pub struct TunnelManager {
    tunnels: Vec<Box<dyn Tunnel>>,
}

impl TunnelManager {
    pub fn new() -> Self {
        Self {
            tunnels: Vec::new(),
        }
    }

    pub fn load_tunnels(&mut self, tunnels: Vec<Tunnel>) -> GatewayResult<()> {
        for tunnel in tunnels {
            let tunnel = match tunnel.tunnel_type() {
                TunnelType::Tcp => {
                    let tunnel = TcpTunnel::build(tunnel.server()).await?;
                    Box::new(tunnel) as Box<dyn Tunnel>
                }
                TunnelType::Udp => {
                    unimplemented!()
                }
            };
            self.tunnels.push(tunnel);
        }
        Ok(())
    }
}
