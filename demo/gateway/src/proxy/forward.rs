use crate::error::*;
use crate::peer::{NameManager, PeerManager};
use crate::tunnel::TunnelCombiner;

use std::sync::Arc;
use std::{net::SocketAddr, str::FromStr};
use tokio::net::{TcpListener, TcpStream};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForwardProxyProtocol {
    Tcp,
    Udp,
}

impl ForwardProxyProtocol {
    pub fn as_str(&self) -> &'static str {
        match self {
            ForwardProxyProtocol::Tcp => "tcp",
            ForwardProxyProtocol::Udp => "udp",
        }
    }
}

impl FromStr for ForwardProxyProtocol {
    type Err = GatewayError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "tcp" => Ok(ForwardProxyProtocol::Tcp),
            "udp" => Ok(ForwardProxyProtocol::Udp),
            _ => Err(GatewayError::InvalidConfig("proxy-type".to_owned())),
        }
    }
}

pub struct ForwardProxyConfig {
    pub protocol: ForwardProxyProtocol,
    pub addr: SocketAddr,
    pub target_device: String,
    pub target_port: u16,
}

#[derive(Clone)]
pub struct TcpForwardProxy {
    name_manager: Arc<NameManager>,
    peer_manager: Arc<PeerManager>,
    config: Arc<ForwardProxyConfig>,
}

impl TcpForwardProxy {
    pub fn new(
        config: ForwardProxyConfig,
        name_manager: Arc<NameManager>,
        peer_manager: Arc<PeerManager>,
    ) -> Self {
        assert!(config.protocol == ForwardProxyProtocol::Tcp);

        Self {
            name_manager,
            peer_manager,
            config: Arc::new(config),
        }
    }

    pub async fn start(&self) -> GatewayResult<()> {
        let listener = TcpListener::bind(&self.config.addr).await.map_err(|e| {
            let msg = format!("Error binding to {}: {}", self.config.addr, e);
            error!("{}", msg);
            GatewayError::Io(e)
        })?;

        info!(
            "Listen for tcp forward connections at {}",
            &self.config.addr
        );

        let this = self.clone();
        tokio::task::spawn(async move {
            if let Err(e) = this.run(listener).await {
                error!("Error running socks5 proxy: {}", e);
            }
        });

        Ok(())
    }

    pub async fn stop(&self) -> GatewayResult<()> {
        todo!("Stop tcp forward proxy");
    }

    async fn run(&self, listener: TcpListener) -> GatewayResult<()> {
        // Standard TCP loop
        loop {
            match listener.accept().await {
                Ok((socket, addr)) => {
                    let this = self.clone();
                    tokio::task::spawn(async move {
                        if let Err(e) = this.on_new_connection(socket, addr).await {
                            error!("Error processing socks5 connection: {}", e);
                        }
                    });
                }
                Err(err) => {
                    error!("Error accepting connection: {}", err);
                }
            }
        }
    }

    async fn on_new_connection(&self, mut conn: TcpStream, addr: SocketAddr) -> GatewayResult<()> {
        info!(
            "Recv tcp forward connection from {} to {}:{}",
            addr, self.config.target_device, self.config.target_port
        );

        let mut tunnel = match self.build_data_tunnel().await {
            Ok(tunnel) => tunnel,
            Err(e) => {
                error!(
                    "Error building data tunnel: {}:{}, {}",
                    self.config.target_device, self.config.target_port, e
                );
                return Err(e);
            }
        };

        let (read, write) = tokio::io::copy_bidirectional(&mut conn, &mut tunnel)
            .await
            .map_err(|e| {
                let msg = format!(
                    "Error copying data on tcp forward connection: {}:{}, {}",
                    self.config.target_device, self.config.target_port, e
                );
                error!("{}", msg);
                GatewayError::Io(e)
            })?;

        info!(
            "Tcp forward connection to {}:{} closed, {} bytes read, {} bytes written",
            self.config.target_device, self.config.target_port, read, write
        );

        Ok(())
    }

    async fn build_data_tunnel(&self) -> GatewayResult<TunnelCombiner> {
        let peer = self
            .peer_manager
            .get_or_init_peer(&self.config.target_device, true)
            .await?;

        let (reader, writer) = peer.build_data_tunnel(self.config.target_port).await?;

        let tunnel = TunnelCombiner::new(reader, writer);

        Ok(tunnel)
    }
}
