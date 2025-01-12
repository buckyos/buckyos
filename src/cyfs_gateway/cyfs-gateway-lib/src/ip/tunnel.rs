use super::tcp::TcpStreamListener;
use super::udp::UdpClient;
use super::udp::UdpDatagramServer;
use crate::tunnel::{
    DatagramClientBox, DatagramServerBox, StreamListener, Tunnel, TunnelBox, TunnelBuilder,
};
use crate::{TunnelError, TunnelResult};
use async_trait::async_trait;
use buckyos_kit::AsyncStream;
use url::Url;

// ***************** Implementations of IP Tunnel *****************

#[derive(Clone)]
pub struct IPTunnel {
    pub target: Url,
}

impl IPTunnel {
    pub fn new(target: &Url) -> IPTunnel {
        IPTunnel {
            target: target.clone(),
        }
    }
}

#[async_trait]
impl Tunnel for IPTunnel {
    async fn ping(&self) -> Result<(), std::io::Error> {
        warn!("IP tunnel's ping not implemented");
        Ok(())
    }

    async fn open_stream(
        &self,
        dest_port: u16,
        dest_host: Option<String>,
    ) -> Result<Box<dyn AsyncStream>, std::io::Error> {
        let dest_addr = match dest_host {
            Some(host) => format!("{}:{}", host, dest_port),
            None => {
                let dest_host = self.target.host_str().unwrap();
                format!("{}:{}", dest_host, dest_port)
            }
        };

        let stream = tokio::net::TcpStream::connect(dest_addr).await?;
        Ok(Box::new(stream))
    }

    async fn create_datagram_client(
        &self,
        dest_port: u16,
        dest_host: Option<String>,
    ) -> Result<Box<dyn DatagramClientBox>, std::io::Error> {
        let dest_host = match dest_host {
            Some(host) => host,
            None => self.target.host_str().unwrap().to_string(),
        };
        
        let client = UdpClient::new(dest_host.to_string(), dest_port).await?;
        Ok(Box::new(client))
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
    async fn create_tunnel(&self, target: &Url) -> TunnelResult<Box<dyn TunnelBox>> {
        if target.scheme() != "tcp" && target.scheme() != "udp" {
            return Err(TunnelError::UrlParseError(
                target.scheme().to_string(),
                "tcp or udp".to_string(),
            ));
        }
        Ok(Box::new(IPTunnel::new(target)))
    }

    async fn create_listener(&self, bind_url: &Url) -> TunnelResult<Box<dyn StreamListener>> {
        let mut result = TcpStreamListener::new(bind_url);
        result.bind().await?;
        Ok(Box::new(result))
    }

    async fn create_datagram_server(
        &self,
        bind_url: &Url,
    ) -> TunnelResult<Box<dyn DatagramServerBox>> {
        let mut result = UdpDatagramServer::new();
        result.bind(bind_url).await?;
        Ok(Box::new(result))
    }
}
