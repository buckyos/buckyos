use super::manager::{ProxyAuth, ProxyConfig};
use crate::{
    error::{GatewayError, GatewayResult},
    peer::{NameManagerRef, PeerManagerRef},
    tunnel::TunnelCombiner,
};

use fast_socks5::{
    server::{Config, SimpleUserPassword, Socks5Socket},
    util::target_addr::TargetAddr,
};
use std::{net::SocketAddr, sync::Arc};
use tokio::{
    net::{TcpListener, TcpStream},
    task,
};

#[derive(Clone)]
pub struct Socks5Proxy {
    name_manager: NameManagerRef,
    peer_manager: PeerManagerRef,
    config: ProxyConfig,
    socks5_config: Arc<Config<SimpleUserPassword>>,
}

impl Socks5Proxy {
    pub fn new(
        config: ProxyConfig,
        name_manager: NameManagerRef,
        peer_manager: PeerManagerRef,
    ) -> Self {
        let socks5_config = Config::default();

        let socks5_config = match config.auth {
            ProxyAuth::None => socks5_config,
            ProxyAuth::Password(ref username, ref password) => {
                socks5_config.with_authentication(SimpleUserPassword {
                    username: username.clone(),
                    password: password.clone(),
                })
            }
        };

        Socks5Proxy {
            name_manager,
            peer_manager,
            config,
            socks5_config: Arc::new(socks5_config),
        }
    }

    pub async fn start(&self) -> GatewayResult<()> {
        let listener = TcpListener::bind(&self.config.addr).await.map_err(|e| {
            let msg = format!("Error binding to {}: {}", self.config.addr, e);
            error!("{}", msg);
            GatewayError::Io(e)
        })?;

        info!("Listen for socks connections at {}", &self.config.addr);

        let this = self.clone();
        task::spawn(async move {
            if let Err(e) = this.run(listener).await {
                error!("Error running socks5 proxy: {}", e);
            }
        });

        Ok(())
    }

    pub async fn stop(&self) -> GatewayResult<()> {
        todo!("stop socks5 proxy");
    }

    async fn run(&self, listener: TcpListener) -> GatewayResult<()> {
        // Standard TCP loop
        loop {
            match listener.accept().await {
                Ok((socket, addr)) => {
                    info!("Connection from {}", addr);

                    if let Err(e) = self.on_new_connection(socket, addr).await {
                        error!("Error processing socks5 connection: {}", e);
                    }
                }
                Err(err) => {
                    error!("Error accepting connection: {}", err);
                }
            }
        }
    }

    async fn on_new_connection(&self, conn: TcpStream, addr: SocketAddr) -> GatewayResult<()> {
        info!("Socks5 connection from {}", addr);
        let socket = Socks5Socket::new(conn, self.socks5_config.clone());

        match socket.upgrade_to_socks5().await {
            Ok(socket) => {
                let target = match socket.target_addr() {
                    Some(target) => {
                        info!("Recv socks5 connection from {} to {}", addr, target);
                        target.to_owned()
                    }
                    None => {
                        let msg =
                            format!("Error getting socks5 connection target address: {},", addr,);
                        error!("{}", msg);
                        return Err(GatewayError::InvalidParam(msg));
                    }
                };

                self.process_socket(socket, target).await
            }
            Err(err) => {
                let msg = format!("Upgrade to socks5 error: {}", err);
                error!("{}", msg);
                Err(GatewayError::Socks(err))
            }
        }
    }

    async fn process_socket(
        &self,
        mut socket: fast_socks5::server::Socks5Socket<TcpStream, SimpleUserPassword>,
        target: TargetAddr,
    ) -> GatewayResult<()> {
        let (device_id, port) = match target {
            TargetAddr::Ip(addr) => match self.name_manager.get_device_id(&addr.ip()) {
                Some(device_id) => (device_id, addr.port()),
                None => {
                    let msg = format!("Device not found for address: {}", addr);
                    error!("{}", msg);
                    return Err(GatewayError::PeerNotFound(msg));
                }
            },
            TargetAddr::Domain(domain, port) => (domain, port),
        };

        let peer = self.peer_manager.get_or_init_peer(&device_id, true).await?;

        let (reader, writer) = peer.build_data_tunnel(port).await?;

        let mut tunnel = TunnelCombiner::new(reader, writer);

        let (read, write) = tokio::io::copy_bidirectional(&mut tunnel, &mut socket)
            .await
            .map_err(|e| {
                let msg = format!(
                    "Error copying data on socks connection: {}:{}, {}",
                    device_id, port, e
                );
                error!("{}", msg);
                GatewayError::Io(e)
            })?;

        info!(
            "socks5 connection to {}:{} closed, {} bytes read, {} bytes written",
            device_id, port, read, write
        );

        Ok(())
    }
}
