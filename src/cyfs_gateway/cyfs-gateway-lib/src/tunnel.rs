
use tokio::{io::{AsyncRead, AsyncWrite}, net::UdpSocket};
use url::Url;
use std::{net::SocketAddr, sync::Arc};
use async_trait::async_trait;
use log::*;

use crate::{TunnelError, TunnelResult};


#[derive(Hash, Eq, PartialEq, Debug,Clone)]
pub struct TunnelEndpoint {
    pub device_id: String,
    pub port: u16,
}

pub trait AsyncStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncStream for T {}

#[async_trait]
pub trait StreamListener : Send
{
    async fn accept(&self) -> Result<(Box<dyn AsyncStream>, TunnelEndpoint),std::io::Error>;
}

#[async_trait]
pub trait DatagramClient : Send  + Sync
{
    async fn recv_datagram(&self,buffer:&mut [u8]) -> Result<usize,std::io::Error>;
    async fn send_datagram(&self,buffer:&[u8]) -> Result<usize,std::io::Error>;
}
pub trait DatagramClientBox : DatagramClient {
    fn clone_box(&self) -> Box<dyn DatagramClientBox>;
}

impl<T> DatagramClientBox for T
where
    T: 'static + Clone + Send + DatagramClient,
{
    fn clone_box(&self) -> Box<dyn DatagramClientBox> {
        Box::new(self.clone())
    }
}

impl Clone for Box<dyn DatagramClientBox> {
    fn clone(&self) -> Box<dyn DatagramClientBox> {
        self.clone_box()
    }
}

#[async_trait]
pub trait DatagramServer  : Send
{
    async fn recv_datagram(&self,buffer:&mut [u8]) -> Result<(usize,TunnelEndpoint),std::io::Error>;
    async fn send_datagram(&self,ep:&TunnelEndpoint,buffer:&[u8]) -> Result<usize,std::io::Error>;
}

pub trait DatagramServerBox : DatagramServer {
    fn clone_box(&self) -> Box<dyn DatagramServerBox>;
}

impl<T> DatagramServerBox for T
where
    T: 'static + Clone + Send + DatagramServer,
{
    fn clone_box(&self) -> Box<dyn DatagramServerBox> {
        Box::new(self.clone())
    }
}

impl Clone for Box<dyn DatagramServerBox> {
    fn clone(&self) -> Box<dyn DatagramServerBox> {
        self.clone_box()
    }
}

// one Tunnel to device
#[async_trait]
pub trait Tunnel : Send + Sync
{
    async fn ping(&self)->Result<(),std::io::Error>;
    async fn open_stream(&self,dest_port:u16) -> Result<Box<dyn AsyncStream>, std::io::Error>;
    async fn create_datagram_client(&self, dest_port:u16) -> Result<Box<dyn DatagramClientBox>,std::io::Error>;

    //async fn create_listener();
    //async fn create_datagram_server(&self, bind_port:u16) -> Result<Box<dyn DatagramTunnel>,std::io::Error>;
}

pub trait TunnelBox : Tunnel {
    fn clone_box(&self) -> Box<dyn TunnelBox>;
}
impl<T> TunnelBox for T
where
    T: 'static + Clone + Send + Tunnel,
{
    fn clone_box(&self) -> Box<dyn TunnelBox> {
        Box::new(self.clone())
    }
}
impl Clone for Box<dyn TunnelBox> {
    fn clone(&self) -> Box<dyn TunnelBox> {
        self.clone_box()
    }
}

#[async_trait]
pub trait TunnelBuilder : Send
{
    async fn create_tunnel(&self,target:&Url) -> TunnelResult<Box<dyn TunnelBox>>;
    async fn create_listener(&self,bind_url:&Url) -> TunnelResult<Box<dyn StreamListener>>;
    async fn create_datagram_server(&self,bind_url:&Url) -> TunnelResult<Box<dyn DatagramServerBox>>;
}

// ***************** Implementations of IP Tunnel *****************

#[derive(Clone)]
pub struct IPTunnel {
    pub target: Url,
}

impl IPTunnel {
    pub fn new(target:&Url) -> IPTunnel {
        IPTunnel {
            target: target.clone(),
        }
    }
}

#[derive(Clone)]
pub struct UdpClient {
    client: Arc<UdpSocket>,
    dest_port: u16,
    dest_addr: String,
}

impl UdpClient {
    pub async fn new(dest_addr:String,dest_port:u16) -> Result<UdpClient, std::io::Error> {
        let client = UdpSocket::bind("0.0.0.0:0").await?;
        Ok(UdpClient {
            client:Arc::new(client),
            dest_port,
            dest_addr,
        })
    }
}



#[async_trait]
impl DatagramClient for UdpClient {
    async fn recv_datagram(&self, buffer: &mut [u8]) -> Result<usize, std::io::Error> {
        let (size, _) = self.client.recv_from(buffer).await?;
        Ok(size)
    }

    async fn send_datagram(&self, buffer: &[u8]) -> Result<usize, std::io::Error> {
        let server_addr = format!("{}:{}", self.dest_addr, self.dest_port); 
        let size = self.client.send_to(buffer,server_addr.clone()).await?;
        trace!("udpclient send datagram to {} size:{}", server_addr, size);
        Ok(size)
    }
}

#[async_trait]
impl Tunnel for IPTunnel {
    async fn ping(&self) -> Result<(), std::io::Error> {
        warn!("IP tunnel's ping not implemented");
        Ok(())
    }

    async fn open_stream(&self, dest_port: u16) -> Result<Box<dyn AsyncStream>, std::io::Error> {
        let dest_host = self.target.host_str().unwrap();
        let dest_addr = format!("{}:{}", dest_host, dest_port);
        let stream = tokio::net::TcpStream::connect(dest_addr).await?;
        Ok(Box::new(stream))
    }

    async fn create_datagram_client(&self, dest_port: u16) -> Result<Box<dyn DatagramClientBox>, std::io::Error> {
        let dest_host = self.target.host_str().unwrap();
        let client = UdpClient::new(dest_host.to_string(),dest_port).await?;
        Ok(Box::new(client))
    }
}



pub struct TcpStreamListener {
    bind_addr: Url,
    listener: Option<tokio::net::TcpListener>,
}

impl TcpStreamListener {
    pub fn new(bind_addr:&Url) -> TcpStreamListener {
        TcpStreamListener {
            bind_addr: bind_addr.clone(),
            listener: None,
        }
    }

    pub async fn bind(&mut self) -> TunnelResult<()> {
        let host = self.bind_addr.host_str().unwrap();
        let port = self.bind_addr.port().unwrap();
        let bind_str = format!("{}:{}", host, port);
        info!("TcpStreamListener try bind to {}", bind_str);
        let listener = tokio::net::TcpListener::bind(bind_str.as_str())
            .await.map_err(|e| {
                TunnelError::BindError(e.to_string())
            })?;
        info!("TcpStreamListener bind to {} OK", bind_str);
        self.listener = Some(listener);
        Ok(())
    }
}

#[async_trait]
impl StreamListener for TcpStreamListener {
    async fn accept(&self) -> Result<(Box<dyn AsyncStream>, TunnelEndpoint), std::io::Error> {
        let listener = self.listener.as_ref().unwrap();
        let (stream, addr) = listener.accept().await?;
        Ok((Box::new(stream), TunnelEndpoint {
            device_id: addr.ip().to_string(),
            port: addr.port(),
        }))
    }
}

#[derive(Clone)]
pub struct UdpDatagramServer {
    server_socket : Option<Arc<UdpSocket>>,
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
            .await.map_err(|e| {
                TunnelError::BindError(e.to_string())
            })?;
        self.server_socket = Some(Arc::new(server_socket));
        Ok(())
    }
}

#[async_trait]
impl DatagramServer for UdpDatagramServer {
    async fn recv_datagram(&self, buffer: &mut [u8]) -> Result<(usize, TunnelEndpoint), std::io::Error> {
        if self.server_socket.is_none() {
            return Err(std::io::Error::new(std::io::ErrorKind::Other, "server socket not initialized"));
        }

        let server_socket = self.server_socket.as_ref().unwrap();
        let (size, addr) = server_socket.recv_from(buffer).await?;
        Ok((size, TunnelEndpoint {
            device_id: addr.ip().to_string(),
            port: addr.port(),
        }))
    }

    async fn send_datagram(&self, ep: &TunnelEndpoint, buffer: &[u8]) -> Result<usize, std::io::Error> {
        if self.server_socket.is_none() {
            return Err(std::io::Error::new(std::io::ErrorKind::Other, "server socket not initialized"));
        }

        let server_socket = self.server_socket.as_ref().unwrap();
        let addr = SocketAddr::new(ep.device_id.parse().unwrap(), ep.port);
        let size = server_socket.send_to(buffer, addr).await?;
        trace!("UdpDatagramServer send datagram to {} size:{}", addr, size);
        Ok(size)    
    }
}

pub struct IPTunnelBuilder {

}

impl IPTunnelBuilder {
    pub fn new() -> IPTunnelBuilder {
        IPTunnelBuilder {}
    }
}

#[async_trait]
impl TunnelBuilder for IPTunnelBuilder {
    async fn create_tunnel(&self,target:&Url) -> TunnelResult<Box<dyn TunnelBox>> {
        if target.scheme() != "tcp" && target.scheme() != "udp" {
            return Err(TunnelError::UrlParseError(target.scheme().to_string(), "tcp or udp".to_string()));
        }
        Ok(Box::new(IPTunnel::new(target)))
    }

    async fn create_listener(&self, bind_url: &Url) -> TunnelResult<Box<dyn StreamListener>> {
        let mut result = TcpStreamListener::new(bind_url);
        result.bind().await?;
        Ok(Box::new(result))
    }

    async fn create_datagram_server(&self, bind_url: &Url) -> TunnelResult<Box<dyn DatagramServerBox>> {
        let mut result = UdpDatagramServer::new();
        result.bind(bind_url).await?;
        Ok(Box::new(result))
    }
}