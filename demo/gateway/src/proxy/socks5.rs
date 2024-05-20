use super::proxy::{GatewayProxy, ProxyAuth, ProxyConfig};
use crate::{
    error::{GatewayError, GatewayResult},
    peer::{NAME_MANAGER, PEER_MANAGER},
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
struct Socks5Proxy {
    config: ProxyConfig,
    socks5_config: Arc<Config<SimpleUserPassword>>,
}

impl Socks5Proxy {
    pub fn new(config: ProxyConfig) -> Self {
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
            config,
            socks5_config: Arc::new(socks5_config),
        }
    }

    pub async fn run(&self) -> GatewayResult<()> {
        let listener = TcpListener::bind(&self.config.addr).await.map_err(|e| {
            let msg = format!("Error binding to {}: {}", self.config.addr, e);
            error!("{}", msg);
            GatewayError::Io(e)
        })?;

        info!("Listen for socks connections at {}", &self.config.addr);

        // Standard TCP loop
        loop {
            match listener.accept().await {
                Ok((socket, addr)) => {
                    info!("Connection from {}", addr);
                    let config = self.socks5_config.clone();
                    if let Err(e) = Self::on_new_connection(socket, addr, config).await {
                        error!("Error processing socks5 connection: {}", e);
                    }
                }
                Err(err) => {
                    error!("Error accepting connection: {}", err);
                }
            }
        }

    }

    async fn on_new_connection(
        conn: TcpStream,
        addr: SocketAddr,
        config: Arc<Config<SimpleUserPassword>>,
    ) -> GatewayResult<()> {
        info!("Socks5 connection from {}", addr);
        let socket = Socks5Socket::new(conn, config);

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

                task::spawn(Self::process_socket(socket, target));
            }
            Err(err) => {
                let msg = format!("Upgrade to socks5 error: {}", err);
                error!("{}", msg);
                return Err(GatewayError::Socks(err));
            }
        }

        Ok(())
    }

    async fn process_socket(
        mut socket: fast_socks5::server::Socks5Socket<TcpStream, SimpleUserPassword>,
        target: TargetAddr,
    ) -> GatewayResult<()> {
        let (device_id, port) = match target {
            TargetAddr::Ip(addr) => match NAME_MANAGER.get_device_id(&addr.ip()) {
                Some(device_id) => (device_id, addr.port()),
                None => {
                    let msg = format!("Device not found for address: {}", addr);
                    error!("{}", msg);
                    return Err(GatewayError::PeerNotFound(msg));
                }
            },
            TargetAddr::Domain(domain, port) => (domain, port),
        };

        let peer = PEER_MANAGER.get_or_init_peer(&device_id).await?;

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
