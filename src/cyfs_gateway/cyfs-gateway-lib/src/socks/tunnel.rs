use crate::tunnel::*;
use crate::{TunnelError, TunnelResult};
use buckyos_kit::AsyncStream;
use std::sync::Arc;
use tokio_socks::tcp::Socks5Stream;
use url::Url;
use super::udp::SocksUdpClient;
use crate::ip::UdpClient;

enum SocksAuth {
    None,
    UsernamePassword(String, String),
}

struct SocksServerInfo {
    host: String,
    port: u16,
    auth: SocksAuth,
}

impl SocksServerInfo {
    pub fn server(&self) -> (&str, u16) {
        (self.host.as_str(), self.port)
    }

    pub fn from_target(target_id: &str) -> TunnelResult<Self> {
        let target = format!("socks://{}", target_id);
        debug!("socks target: {}", target);
        let url = Url::parse(target.as_str()).map_err(|e| {
            let msg = format!("Invalid socks target url: {}", e);
            error!("{}", msg);
            TunnelError::UrlParseError(target.to_owned(), msg)
        })?;

        let host = url.host_str().ok_or_else(|| {
            let msg = format!("Invalid socks target host");
            error!("{}", msg);
            TunnelError::UrlParseError(target.to_owned(), msg)
        })?;

        let port = url.port().unwrap_or(1080);

        // Parse auth
        let username = url.username().to_string();
        let password = url.password().map(|p| p.to_string());
        let auth = if username.is_empty() {
            SocksAuth::None
        } else {
            SocksAuth::UsernamePassword(username, password.unwrap_or_default())
        };

        Ok(Self {
            host: host.to_string(),
            port,
            auth,
        })
    }
}

#[derive(Clone)]
pub struct SocksTunnel {
    socks_server: Option<Arc<SocksServerInfo>>,
}

impl SocksTunnel {
    pub async fn new(target_id: Option<&str>) -> TunnelResult<Self> {
        let socks_server = match target_id {
            Some(target) => {
                let socks_server = SocksServerInfo::from_target(target)?;
                Some(socks_server)
            }
            None => None,
        };

        Ok(Self {
            socks_server: socks_server.map(|s| Arc::new(s)),
        })
    }
}

#[async_trait::async_trait]
impl Tunnel for SocksTunnel {
    async fn ping(&self) -> Result<(), std::io::Error> {
        warn!("Socks tunnel's ping not implemented");
        Ok(())
    }

    async fn open_stream_by_dest(
        &self,
        dest_port: u16,
        dest_host: Option<String>,
    ) -> Result<Box<dyn AsyncStream>, std::io::Error> {
        debug!("socks_tunnel open_stream_by_dest: {:?}:{}", dest_host, dest_port);
        // FIXME what should we do if dest_host is None or the port is 0?
        let dest_host = dest_host.unwrap_or("0.0.0.0".to_string());
        let dest_port = if dest_port == 0 { 80 } else { dest_port };

        match self.socks_server {
            Some(ref socks_server) => {
                // Establish a SOCKS5 tunnel with optional username and password
                let ret = match socks_server.auth {
                    SocksAuth::UsernamePassword(ref username, ref password) => {
                        Socks5Stream::connect_with_password(
                            (socks_server.host.as_str(), socks_server.port),
                            (dest_host.as_str(), dest_port),
                            &username,
                            &password,
                        )
                        .await
                    }
                    SocksAuth::None => {
                        Socks5Stream::connect(
                            (socks_server.host.as_str(), socks_server.port),
                            (dest_host.as_str(), dest_port),
                        )
                        .await
                    }
                };

                ret.as_ref().map_err(|e| {
                    let msg = format!(
                        "Failed to establish SOCKS5 tunnel: {:?}, {}",
                        socks_server.server(),
                        e
                    );
                    error!("{}", msg);
                    std::io::Error::new(std::io::ErrorKind::Other, msg)
                })?;

                let stream = ret.unwrap();
                Ok(Box::new(stream))
            }
            None => {
                let dest_addr = format!("{}:{}", dest_host, dest_port);
                let stream = tokio::net::TcpStream::connect(&dest_addr)
                    .await
                    .map_err(|e| {
                        let msg = format!("Failed to connect to target: {}, {}", dest_addr, e);
                        error!("{}", msg);
                        std::io::Error::new(std::io::ErrorKind::Other, msg)
                    })?;

                Ok(Box::new(stream))
            }
        }
    }

    async fn open_stream(&self, stream_id: &str) -> Result<Box<dyn AsyncStream>, std::io::Error> {
        debug!("socks_tunnel open_stream: {}", stream_id);
        let (dest_host, dest_port) = get_dest_info_from_url_path(stream_id)?;
        self.open_stream_by_dest(dest_port, dest_host).await
    }

    async fn create_datagram_client_by_dest(
        &self,
        dest_port: u16,
        dest_host: Option<String>,
    ) -> Result<Box<dyn DatagramClientBox>, std::io::Error> {
        // FIXME what should we do if dest_host is None or the port is 0?
        let dest_host = dest_host.unwrap_or("0.0.0.0".to_string());
        let dest_port = if dest_port == 0 { 80 } else { dest_port };

        match self.socks_server {
            Some(ref socks_server) => {
                let client =
                    libsocks_client::SocksClientBuilder::new(&socks_server.host, socks_server.port)
                        .socks5();
                let client = match socks_server.auth {
                    SocksAuth::UsernamePassword(ref username, ref password) => {
                        client.username(username).password(password)
                    }
                    SocksAuth::None => client,
                };

                let mut client = client.build_udp_client();
                client.udp_associate("0.0.0.0", 0).await.map_err(|e| {
                    let msg = format!(
                        "Failed to establish SOCKS5 UDP tunnel: {:?}, {:?}, {}",
                        socks_server.server(), (&dest_host, dest_port),
                        e
                    );
                    error!("{}", msg);
                    std::io::Error::new(std::io::ErrorKind::Other, msg)
                })?;

                let socket: libsocks_client::SocksUdpSocket = client.get_udp_socket("0.0.0.0:0").await.map_err(|e| {
                    let msg = format!(
                        "Failed to get UDP socket for SOCKS5 UDP tunnel: {:?}, {:?}, {}",
                        socks_server.server(), (&dest_host, dest_port),
                        e
                    );
                    error!("{}", msg);
                    std::io::Error::new(std::io::ErrorKind::Other, msg)
                })?;

                let client = SocksUdpClient::new(socket, dest_host, dest_port);
                Ok(Box::new(client))
            }
            None => {
                let client = UdpClient::new(dest_host, dest_port, None).await?;
                Ok(Box::new(client))
            }
        }
    }

    async fn create_datagram_client(
        &self,
        session_id: &str,
    ) -> Result<Box<dyn DatagramClientBox>, std::io::Error> {
        let (dest_host, dest_port) = get_dest_info_from_url_path(session_id)?;
        self.create_datagram_client_by_dest(dest_port, dest_host)
            .await
    }
}

pub struct SocksTunnelBuilder {}

impl SocksTunnelBuilder {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait::async_trait]
impl TunnelBuilder for SocksTunnelBuilder {
    async fn create_tunnel(
        &self,
        tunnel_stack_id: Option<&str>,
    ) -> TunnelResult<Box<dyn TunnelBox>> {
        debug!("socks_tunnel_builder create_tunnel: {}", tunnel_stack_id.unwrap_or(""));
        let tunnel = SocksTunnel::new(tunnel_stack_id).await?;
        Ok(Box::new(tunnel))
    }

    async fn create_stream_listener(
        &self,
        _bind_stream_id: &Url,
    ) -> TunnelResult<Box<dyn StreamListener>> {
        unimplemented!("SocksTunnelBuilder create_stream_listener not implemented");
    }

    async fn create_datagram_server(
        &self,
        _bind_session_id: &Url,
    ) -> TunnelResult<Box<dyn DatagramServerBox>> {
        unimplemented!("SocksTunnelBuilder create_datagram_server not implemented");
    }
}
