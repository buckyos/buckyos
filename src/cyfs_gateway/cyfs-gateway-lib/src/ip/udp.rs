use crate::tunnel::{DatagramClient, DatagramServer, TunnelEndpoint};
use crate::{TunnelError, TunnelResult};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use url::Url;

#[derive(Clone)]
pub struct UdpClient {
    client: Arc<UdpSocket>,
    dest_port: u16,
    dest_addr: String,
}

impl UdpClient {
    pub async fn new(dest_addr: String, dest_port: u16) -> Result<UdpClient, std::io::Error> {
        let client = UdpSocket::bind("0.0.0.0:0").await?;
        Ok(UdpClient {
            client: Arc::new(client),
            dest_port,
            dest_addr,
        })
    }
}

#[async_trait::async_trait]
impl DatagramClient for UdpClient {
    async fn recv_datagram(&self, buffer: &mut [u8]) -> Result<usize, std::io::Error> {
        let (size, _) = self.client.recv_from(buffer).await?;
        Ok(size)
    }

    async fn send_datagram(&self, buffer: &[u8]) -> Result<usize, std::io::Error> {
        let server_addr = format!("{}:{}", self.dest_addr, self.dest_port);
        let size = self.client.send_to(buffer, server_addr.clone()).await?;
        trace!("udpclient send datagram to {} size:{}", server_addr, size);
        Ok(size)
    }
}

#[derive(Clone)]
pub struct UdpDatagramServer {
    server_socket: Option<Arc<UdpSocket>>,
}

impl UdpDatagramServer {
    pub fn new() -> UdpDatagramServer {
        UdpDatagramServer {
            server_socket: None,
        }
    }

    pub async fn bind(&mut self, bind_url: &Url) -> TunnelResult<()> {
        let host = bind_url.host_str().unwrap();
        let port = bind_url.port().unwrap();
        let bind_str = format!("{}:{}", host, port);
        let server_socket = tokio::net::UdpSocket::bind(bind_str)
            .await
            .map_err(|e| TunnelError::BindError(e.to_string()))?;
        self.server_socket = Some(Arc::new(server_socket));
        Ok(())
    }
}

#[async_trait::async_trait]
impl DatagramServer for UdpDatagramServer {
    async fn recv_datagram(
        &self,
        buffer: &mut [u8],
    ) -> Result<(usize, TunnelEndpoint), std::io::Error> {
        if self.server_socket.is_none() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "server socket not initialized",
            ));
        }

        let server_socket = self.server_socket.as_ref().unwrap();
        let (size, addr) = server_socket.recv_from(buffer).await?;
        Ok((
            size,
            TunnelEndpoint {
                device_id: addr.ip().to_string(),
                port: addr.port(),
            },
        ))
    }

    async fn send_datagram(
        &self,
        ep: &TunnelEndpoint,
        buffer: &[u8],
    ) -> Result<usize, std::io::Error> {
        if self.server_socket.is_none() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "server socket not initialized",
            ));
        }

        let server_socket = self.server_socket.as_ref().unwrap();
        let addr = SocketAddr::new(ep.device_id.parse().unwrap(), ep.port);
        let size = server_socket.send_to(buffer, addr).await?;
        trace!("UdpDatagramServer send datagram to {} size:{}", addr, size);
        Ok(size)
    }
}
