use super::proxy::{GatewayProxy, ProxyAuth, ProxyConfig};
use crate::error::GatewayError;

use fast_socks5::{
    server::{Authentication, Config, SimpleUserPassword, Socks5Socket},
    Result,
};
use std::sync::Arc;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpListener,
    task,
};

struct Socks5Proxy {
    config: ProxyConfig,
}

#[async_trait::async_trait]
impl GatewayProxy for Socks5Proxy {
    async fn start(&self) -> Result<(), GatewayError> {
        let config = Config::default();

        let config = match self.config.auth {
            ProxyAuth::None => config,
            ProxyAuth::Password(ref username, ref password) => {
                config.with_authentication(SimpleUserPassword {
                    username: username.clone(),
                    password: password.clone(),
                })
            }
        };

        let config = Arc::new(config);

        let listener = TcpListener::bind(&self.config.addr).await?;

        info!("Listen for socks connections @{}", &self.config.addr);

        // Standard TCP loop
        loop {
            match listener.accept().await {
                Ok((socket, _addr)) => {
                    info!("Connection from {}", socket.peer_addr()?);
                    let socket = Socks5Socket::new(socket, config.clone());

                    task::spawn(async move {
                        match socket.upgrade_to_socks5().await {
                            Ok(_) => {
                                info!("Connection closed");
                            }
                            Err(err) => {
                                error!("Error: {}", err);
                            }
                        }
                    });
                }
                Err(err) => {
                    error!("Error accepting connection: {}", err);
                }
            }
        }
    }

    async fn stop(&self) -> Result<(), GatewayError> {
        unimplemented!("stop not implemented for Socks5Proxy");
    }
}
