use super::tcp::TcpStreamListener;
use super::udp::UdpClient;
use super::udp::UdpDatagramServer;
use crate::tunnel::*;
use crate::{TunnelError, TunnelResult};
use async_trait::async_trait;
use buckyos_kit::AsyncStream;
use url::Url;
use std::net::SocketAddr;

#[derive(Clone)]
pub struct IPTunnel {
    pub ip_stack_id: Option<String>,
}

impl IPTunnel {
    pub fn new(ip_stack_id: Option<&str>) -> IPTunnel {
        IPTunnel {
            ip_stack_id: ip_stack_id.map(|s| s.to_string()),
        }
    }
}

#[async_trait]
impl Tunnel for IPTunnel {
    async fn ping(&self) -> Result<(), std::io::Error> {
        warn!("IP tunnel's ping not implemented");
        Ok(())
    }

    async fn open_stream_by_dest(
        &self,
        dest_port: u16,
        dest_host: Option<String>,
    ) -> Result<Box<dyn AsyncStream>, std::io::Error> {
        let dest_addr = match dest_host {
            Some(host) => format!("{}:{}", host, dest_port),
            None => {
                if self.ip_stack_id.is_none() {
                    format!("0.0.0.0:{}", dest_port)
                } else {
                    format!("{}:{}", self.ip_stack_id.as_ref().unwrap(), dest_port)
                }
            }
        };
        
        let stream;
        if self.ip_stack_id.is_none() {
            debug!("use any tcp client addr for open_stream : {}", dest_addr);
            stream = tokio::net::TcpStream::connect(dest_addr).await?;
        } else {
            let bind_addr = self.ip_stack_id.as_ref().unwrap();
            let is_ipv4 = is_ipv4_addr_str(bind_addr)?;
            let socket;
            if is_ipv4 {
                socket = tokio::net::TcpSocket::new_v4().unwrap();
            } else {
                socket = tokio::net::TcpSocket::new_v6().unwrap();
            }
            let local_bind_addr:SocketAddr = format!("{}:0", bind_addr).parse()
                .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "invalid bind addr"))?;
            socket.bind(local_bind_addr)?;
            debug!("use {:?} tcp client addr for open_stream : {}", local_bind_addr, dest_addr);
            let dest_addr:SocketAddr = dest_addr.parse()
                .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "invalid dest addr"))?;
            stream = socket.connect(dest_addr).await?;
        }

        Ok(Box::new(stream))
    }

    async fn open_stream(&self, stream_id: &str) -> Result<Box<dyn AsyncStream>, std::io::Error> {
        debug!("ip_tunnel open_stream: {}", stream_id);
        let (dest_host, dest_port) = get_dest_info_from_url_path(stream_id)?;
        self.open_stream_by_dest(dest_port, dest_host).await
    }

    async fn create_datagram_client_by_dest(
        &self,
        dest_port: u16,
        dest_host: Option<String>,
    ) -> Result<Box<dyn DatagramClientBox>, std::io::Error> {
        let real_dest_host;
        if dest_host.is_none() {
            if self.ip_stack_id.is_none() {
                real_dest_host = "0.0.0.0".to_string();
            } else {
                real_dest_host = self.ip_stack_id.as_ref().unwrap().to_string();
            }
        } else {
            real_dest_host = dest_host.unwrap();
        }
        
        let client = UdpClient::new(real_dest_host, dest_port,self.ip_stack_id.clone()).await?;
        Ok(Box::new(client))
    }

    async fn create_datagram_client(&self, session_id: &str) -> Result<Box<dyn DatagramClientBox>, std::io::Error> {
        let (dest_host, dest_port) = get_dest_info_from_url_path(session_id)?;
        self.create_datagram_client_by_dest(dest_port, dest_host).await
    }
}

pub struct IPTunnelBuilder {}

impl IPTunnelBuilder {
    pub fn new() -> IPTunnelBuilder {
        IPTunnelBuilder {}
    }
}

#[async_trait]
impl TunnelBuilder for IPTunnelBuilder {
    async fn create_tunnel(&self, target_id: Option<&str>) -> TunnelResult<Box<dyn TunnelBox>> {
        Ok(Box::new(IPTunnel::new(target_id)))
    }

    async fn create_stream_listener(&self, bind_stream_id: &Url) -> TunnelResult<Box<dyn StreamListener>> {
        let mut result = TcpStreamListener::new(bind_stream_id);
        result.bind().await?;
        Ok(Box::new(result))
    }

    async fn create_datagram_server(
        &self,
        bind_session_id: &Url,
    ) -> TunnelResult<Box<dyn DatagramServerBox>> {
        let mut result = UdpDatagramServer::new();
        result.bind(bind_session_id).await?;
        Ok(Box::new(result))
    }
}
