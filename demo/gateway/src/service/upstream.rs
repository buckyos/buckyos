use crate::error::*;
use crate::peer::{OnNewTunnelHandleResult, PeerManagerEvents, PeerManagerEventsRef};
use crate::tunnel::{DataTunnelInfo, TunnelCombiner};

use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use tokio::net::TcpStream;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpstreamServiceProtocol {
    Tcp,
    Udp,
    Http,
}

impl UpstreamServiceProtocol {
    pub fn as_str(&self) -> &'static str {
        match self {
            UpstreamServiceProtocol::Tcp => "tcp",
            UpstreamServiceProtocol::Udp => "udp",
            UpstreamServiceProtocol::Http => "http",
        }
    }
}

impl FromStr for UpstreamServiceProtocol {
    type Err = GatewayError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "tcp" => Ok(UpstreamServiceProtocol::Tcp),
            "udp" => Ok(UpstreamServiceProtocol::Udp),
            "http" => Ok(UpstreamServiceProtocol::Http),
            _ => Err(GatewayError::InvalidParam("type".to_owned())),
        }
    }
}

#[derive(Clone, Debug)]
pub struct UpstreamService {
    id: String,
    addr: SocketAddr,
    protocol: UpstreamServiceProtocol,
}

impl UpstreamService {
    pub fn load(value: &serde_json::Value) -> GatewayResult<Self> {
        if !value.is_object() {
            return Err(GatewayError::InvalidConfig("upstream".to_owned()));
        }

        let id = value["id"]
            .as_str()
            .ok_or(GatewayError::InvalidConfig(
                "Invalid upstream block config: id".to_owned(),
            ))?
            .to_owned();
        if id.is_empty() {
            let msg = format!(
                "Invalid upstream block config: id, {}",
                serde_json::to_string(value).unwrap()
            );
            warn!("{}", msg);

            return Err(GatewayError::InvalidConfig(msg));
        }

        let addr: IpAddr = value["addr"]
            .as_str()
            .ok_or(GatewayError::InvalidConfig(
                "Invalid upstream block config: addr".to_owned(),
            ))?
            .parse()
            .map_err(|e| {
                let msg = format!("Error parsing addr: {}", e);
                GatewayError::InvalidConfig(msg)
            })?;
        let port = value["port"].as_u64().ok_or(GatewayError::InvalidConfig(
            "Invalid upstream block config: port".to_owned(),
        ))? as u16;
        let protocol = value["protocol"]
            .as_str()
            .ok_or(GatewayError::InvalidConfig(
                "Invalid upstream block config: type".to_owned(),
            ))?;

        let protocol = UpstreamServiceProtocol::from_str(protocol)?;

        info!(
            "New upstream service: {}:{} type: {}",
            addr,
            port,
            protocol.as_str()
        );

        Ok(Self {
            id,
            addr: SocketAddr::new(addr, port),
            protocol,
        })
    }
}

#[derive(Clone)]
pub struct UpstreamManager {
    services: Arc<Mutex<Vec<UpstreamService>>>,
}

impl UpstreamManager {
    pub fn new() -> Self {
        Self {
            services: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn clone_as_events(&self) -> PeerManagerEventsRef {
        Arc::new(Box::new(self.clone()) as Box<dyn PeerManagerEvents>)
    }

    pub fn get_service(&self, id: &str) -> Option<UpstreamService> {
        let services = self.services.lock().unwrap();
        for service in services.iter() {
            if service.id == id {
                return Some(service.clone());
            }
        }

        None
    }

    pub fn is_service_exist(&self, id: &str) -> bool {
        self.get_service(id).is_some()
    }

    /*
    {
        "block": "upstream",
        "addr": "127.0.0.1",
        "port": 2000,
        "type": "tcp"
    }
    */
    pub fn load_block(&self, value: &serde_json::Value) -> GatewayResult<()> {
        let service = UpstreamService::load(value)?;

        // check if service already exists
        let mut services = self.services.lock().unwrap();
        for s in services.iter() {
            if s.id == service.id {
                let msg = format!("Upstream service already exists: {}", service.id);
                warn!("{}", msg);
                return Err(GatewayError::InvalidConfig(msg));
            }
        }

        info!("New upstream service: {:?}", service);

        services.push(service);

        Ok(())
    }

    pub fn add(&self, service: UpstreamService) -> GatewayResult<()> {
        let mut services = self.services.lock().unwrap();
        for s in services.iter() {
            if s.id == service.id {
                let msg = format!("Upstream service already exists: {}", service.id);
                warn!("{}", msg);
                return Err(GatewayError::AlreadyExists(msg));
            }
        }

        info!("New upstream service: {:?}", service);

        services.push(service);

        Ok(())
    }

    pub fn remove(&self, id: &str) -> GatewayResult<()> {
        let mut services = self.services.lock().unwrap();
        let mut found = false;
        services.retain(|s| {
            if s.id == id {
                found = true;
                false
            } else {
                true
            }
        });

        if !found {
            let msg = format!("Upstream service not found: {}", id);
            warn!("{}", msg);
            return Err(GatewayError::UpstreamNotFound(msg));
        }

        Ok(())
    }

    fn find_service(&self, port: u16, protocol: UpstreamServiceProtocol) -> Option<UpstreamService> {
        let services = self.services.lock().unwrap();
        for service in services.iter() {
            // info!("Service item: {} {}", service.addr.port(), protocol.as_str());
            if service.addr.port() == port && service.protocol == protocol {
                // info!("Got service: {} {}", service.addr.port(), protocol.as_str());
                return Some(service.clone());
            }
        }

        None
    }

    pub async fn bind_tunnel(&self, tunnel: DataTunnelInfo) -> GatewayResult<()> {
        let service = self.find_service(tunnel.port, UpstreamServiceProtocol::Tcp);
        if service.is_none() {
            let msg = format!("No upstream service found for port {}", tunnel.port);
            return Err(GatewayError::UpstreamNotFound(msg));
        }

        self.bind_tunnel_impl(service.unwrap(), tunnel).await
    }

    async fn bind_tunnel_impl(
        &self,
        service: UpstreamService,
        tunnel: DataTunnelInfo,
    ) -> GatewayResult<()> {
        match service.protocol {
            UpstreamServiceProtocol::Tcp | UpstreamServiceProtocol::Http => {
                tokio::spawn(Self::run_tcp_forward(tunnel, service));
            }
            UpstreamServiceProtocol::Udp => {
                let msg = format!("Udp tunnel not supported yet {}", tunnel.port);
                error!("{}", msg);
                return Err(GatewayError::NotSupported(msg));
            }
        }

        Ok(())
    }

    async fn run_tcp_forward(
        tunnel: DataTunnelInfo,
        service: UpstreamService,
    ) -> GatewayResult<()> {
        // first create tcp stream to upstream service
        let mut stream = TcpStream::connect(&service.addr).await.map_err(|e| {
            let msg = format!(
                "Error connecting to upstream service: {}, {}",
                service.addr, e
            );
            error!("{}", msg);
            GatewayError::Io(e)
        })?;

        info!(
            "Bind tunnel {} to upstream service {}",
            tunnel.port, service.addr
        );

        let mut btunnel = TunnelCombiner::new(tunnel.tunnel_reader, tunnel.tunnel_writer);

        let (read, write) = tokio::io::copy_bidirectional(&mut btunnel, &mut stream)
            .await
            .map_err(|e| {
                let msg = format!(
                    "Error forward tunnel to upstream service: {} {}",
                    service.addr, e
                );
                error!("{}", msg);
                GatewayError::Io(e)
            })?;

        /*
        // bind reader and writer to tunnel.tunnel_writer and tunnel.tunnel_reader
        let stream_to_tunnel = tokio::io::copy(&mut reader, &mut tunnel.tunnel_writer);
        let tunnel_to_stream = tokio::io::copy(&mut tunnel.tunnel_reader, &mut writer);

        tokio::try_join!(stream_to_tunnel, tunnel_to_stream).map_err(|e| {
            let msg = format!(
                "Error forward tunnel to upstream service: {} {}",
                service.addr, e
            );
            error!("{}", msg);
            GatewayError::Io(e)
        })?;
        */

        info!(
            "Tunnel {} bound to upstream service {} finished, {} bytes read, {} bytes written",
            tunnel.port, service.addr, read, write
        );

        Ok(())
    }
}

impl PeerManagerEvents for UpstreamManager {
    fn on_recv_data_tunnel(&self, info: DataTunnelInfo) -> GatewayResult<OnNewTunnelHandleResult> {
        info!(
            "Will handle data tunnel for upstream manager: {}, {}",
            info.device_id, info.port
        );

        let service = self.find_service(info.port, UpstreamServiceProtocol::Tcp);
        if service.is_none() {
            let msg = format!("No upstream service found for port {}", info.port);
            info!("{}", msg);

            let ret = OnNewTunnelHandleResult {
                handled: false,
                info: Some(info),
            };

            return Ok(ret);
        }

        let this = self.clone();
        tokio::spawn(async move {
            let service = service.unwrap();
            let _ = this.bind_tunnel_impl(service, info).await;
        });

        Ok(OnNewTunnelHandleResult {
            handled: true,
            info: None,
        })
    }
}

pub type UpstreamManagerRef = Arc<UpstreamManager>;
