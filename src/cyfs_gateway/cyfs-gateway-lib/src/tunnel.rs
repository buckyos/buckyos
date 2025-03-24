use crate::TunnelResult;
use async_trait::async_trait;
use buckyos_kit::AsyncStream;
use url::Url;
use std::net::IpAddr;
use std::net::SocketAddr;
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
    async fn open_stream_by_dest(
        &self,
        dest_port: u16,
        dest_host: Option<String>,
    ) -> Result<Box<dyn AsyncStream>, std::io::Error>;

    async fn open_stream(&self,
        stream_id:&str,
    ) -> Result<Box<dyn AsyncStream>, std::io::Error>;

    async fn create_datagram_client_by_dest(
        &self,
        dest_port: u16,
        dest_host: Option<String>,
    ) -> Result<Box<dyn DatagramClientBox>, std::io::Error>;

    async fn create_datagram_client(
        &self,
        session_id:&str,
    ) -> Result<Box<dyn DatagramClientBox>, std::io::Error>;
    
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
    async fn create_tunnel(&self, tunnel_stack_id: Option<&str>) -> TunnelResult<Box<dyn TunnelBox>>;
    async fn create_stream_listener(&self, 
        bind_stream_id: &Url) -> TunnelResult<Box<dyn StreamListener>>;
    async fn create_datagram_server(
        &self,
        bind_session_id: &Url) -> TunnelResult<Box<dyn DatagramServerBox>>;
}

#[async_trait]
pub trait TunnelSelector {
    async fn select_tunnel_for_http_upstream(
        &self,
        req_host: &str,
        req_path: &str,
    ) -> Option<String>;
}

pub fn is_ipv4_addr_str(addr: &str) -> Result<bool, std::io::Error> {
    let ip_addr = addr.parse::<IpAddr>().map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, e)
    })?;
    Ok(ip_addr.is_ipv4())
}

pub fn get_dest_info_from_url_path(path: &str) -> Result<(Option<String>, u16), std::io::Error> {
    let path = path.trim_start_matches('/');
    let path = std::path::Path::new(path);

    let first_component = path.iter().next().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Invalid path: empty path",
        )
    })?;


    let addr_str = first_component.to_str().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Invalid path: contains non-UTF8 characters",
        )
    })?;

    // 处理以冒号开头的情况（如 ":8000"）和 host:port 的情况
    if addr_str.starts_with(':') {

        let dest_port = addr_str[1..].parse::<u16>().map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, e)
        })?;
        Ok((None, dest_port))
    } else {
        if let Ok(sock_addr) = addr_str.parse::<SocketAddr>() {
            let dest_host = sock_addr.ip().to_string();
            let dest_port = sock_addr.port();
            return Ok((Some(dest_host), dest_port))
        }
        
        let parts = addr_str.split(':').collect::<Vec<&str>>();
        if parts.len() != 2 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Invalid address format"));
        }

        let dest_host = parts[0];
        let dest_port = parts[1].parse::<u16>().map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, e)
        })?;

        Ok((Some(dest_host.to_string()), dest_port))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_dest_info_from_url_path() {
        std::env::set_var("BUCKY_LOG", "debug");
        buckyos_kit::init_logging("test_get_dest_info_from_url_path",false);
        let (host, port) = get_dest_info_from_url_path("127.0.0.1:8080").unwrap();
        assert_eq!(host, Some("127.0.0.1".to_string()));
        assert_eq!(port, 8080);

        let (host, port) = get_dest_info_from_url_path("xba.dev.did:8080/krpc/api_test?a=1&b=2").unwrap();
        assert_eq!(host, Some("xba.dev.did".to_string()));
        assert_eq!(port, 8080);

        let (host, port) = get_dest_info_from_url_path("/[2600:1700:1150:9440:f65:adec:9b77:cb2]:8080/krpc/api_test").unwrap();
        assert_eq!(host, Some("2600:1700:1150:9440:f65:adec:9b77:cb2".to_string()));
        assert_eq!(port, 8080);

        let (host, port) = get_dest_info_from_url_path(":8080").unwrap();
        assert_eq!(host, None);
        assert_eq!(port, 8080);

        let ipv4_addr = is_ipv4_addr_str("127.0.0.1").unwrap();
        assert_eq!(ipv4_addr, true);
        let ipv6_addr = is_ipv4_addr_str("::1").unwrap();
        assert_eq!(ipv6_addr, false);
    }
}
