use crate::TunnelResult;
use async_trait::async_trait;
use buckyos_kit::AsyncStream;
use url::Url;

#[derive(Hash, Eq, PartialEq, Debug, Clone)]
pub struct TunnelEndpoint {
    pub device_id: String,
    pub port: u16,
}

#[async_trait]
pub trait StreamListener: Send {
    async fn accept(&self) -> Result<(Box<dyn AsyncStream>, TunnelEndpoint), std::io::Error>;
}

#[async_trait]
pub trait DatagramClient: Send + Sync {
    async fn recv_datagram(&self, buffer: &mut [u8]) -> Result<usize, std::io::Error>;
    async fn send_datagram(&self, buffer: &[u8]) -> Result<usize, std::io::Error>;
}
pub trait DatagramClientBox: DatagramClient {
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
pub trait DatagramServer: Send {
    async fn recv_datagram(
        &self,
        buffer: &mut [u8],
    ) -> Result<(usize, TunnelEndpoint), std::io::Error>;
    async fn send_datagram(
        &self,
        ep: &TunnelEndpoint,
        buffer: &[u8],
    ) -> Result<usize, std::io::Error>;
}

pub trait DatagramServerBox: DatagramServer {
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
pub trait Tunnel: Send + Sync {
    async fn ping(&self) -> Result<(), std::io::Error>;
    async fn open_stream(
        &self,
        dest_port: u16,
        dest_host: Option<String>,
    ) -> Result<Box<dyn AsyncStream>, std::io::Error>;
    async fn create_datagram_client(
        &self,
        dest_port: u16,
        dest_host: Option<String>,
    ) -> Result<Box<dyn DatagramClientBox>, std::io::Error>;

    //async fn create_listener();
    //async fn create_datagram_server(&self, bind_port:u16) -> Result<Box<dyn DatagramTunnel>,std::io::Error>;
}

pub trait TunnelBox: Tunnel {
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
pub trait TunnelBuilder: Send {
    async fn create_tunnel(&self, target: &Url) -> TunnelResult<Box<dyn TunnelBox>>;
    async fn create_listener(&self, bind_url: &Url) -> TunnelResult<Box<dyn StreamListener>>;
    async fn create_datagram_server(
        &self,
        bind_url: &Url,
    ) -> TunnelResult<Box<dyn DatagramServerBox>>;
}

#[async_trait]
pub trait TunnelSelector {
    async fn select_tunnel_for_http_upstream(
        &self,
        req_host: &str,
        req_path: &str,
    ) -> Option<String>;
}
