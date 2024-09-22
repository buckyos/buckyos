use tokio::io::{AsyncRead, AsyncWrite};
use url::Url;
use std::net::SocketAddr;
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
    async fn recv_datagram(&self,buffer:&mut [u8]) -> Result<(usize),std::io::Error>;
    fn send_datagram(&self,buffer:&[u8]) -> Result<usize,std::io::Error>;
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
    fn send_datagram(&self,ep:&TunnelEndpoint,buffer:&[u8]) -> Result<usize,std::io::Error>;
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
        unimplemented!()
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

pub struct UdpDatagramServer {

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
        unimplemented!()
    }
}