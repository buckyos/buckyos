use crate::peer::{NameManager, PeerManager};
use crate::tunnel::TunnelCombiner;
use gateway_lib::*;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

#[derive(Debug, Clone)]
pub struct ForwardProxyConfig {
    pub id: String,
    pub protocol: ForwardProxyProtocol,
    pub addr: SocketAddr,
    pub target_device: String,
    pub target_port: u16,

    pub source: ConfigSource,
}

impl ForwardProxyConfig {
    pub fn load(config: &serde_json::Value, source: Option<ConfigSource>) -> GatewayResult<Self> {
        if !config.is_object() {
            return Err(GatewayError::InvalidConfig("proxy".to_owned()));
        }

        let id = config["id"]
            .as_str()
            .ok_or(GatewayError::InvalidConfig(
                "Invalid proxy block config: id".to_owned(),
            ))?
            .to_owned();
        if id.is_empty() {
            return Err(GatewayError::InvalidConfig(
                "Invalid proxy block config: id".to_owned(),
            ));
        }

        let protocol = config["protocol"]
            .as_str()
            .ok_or(GatewayError::InvalidConfig(
                "Invalid proxy block config: protocol".to_owned(),
            ))?
            .parse::<ForwardProxyProtocol>()?;

        let addr = config["addr"]
            .as_str()
            .ok_or(GatewayError::InvalidConfig("addr".to_owned()))?;
        let port = config["port"]
            .as_u64()
            .ok_or(GatewayError::InvalidConfig("port".to_owned()))? as u16;
        let addr = format!("{}:{}", addr, port);
        let addr = addr.parse().map_err(|e| {
            let msg = format!("Error parsing addr: {}, {}", addr, e);
            error!("{}", msg);
            GatewayError::InvalidConfig(msg)
        })?;

        let target_device = config["target_device"]
            .as_str()
            .ok_or(GatewayError::InvalidConfig(
                "Invalid proxy block config: target_device".to_owned(),
            ))?
            .to_owned();

        let target_port = config["target_port"]
            .as_u64()
            .ok_or(GatewayError::InvalidConfig(
                "Invalid proxy block config: target_port".to_owned(),
            ))? as u16;

        Ok(Self {
            id,
            protocol,
            addr,
            target_device,
            target_port,

            source: source.unwrap_or(ConfigSource::Config),
        })
    }

    pub fn dump(&self) -> serde_json::Value {
        let mut config = serde_json::Map::new();
        config.insert("block".to_owned(), "proxy".into());
        config.insert("type".to_owned(), "forward".into());
        config.insert("id".to_owned(), self.id.clone().into());
        config.insert("protocol".to_owned(), self.protocol.to_string().into());
        config.insert("addr".to_owned(), self.addr.to_string().into());
        config.insert("target_device".to_owned(), self.target_device.clone().into());
        config.insert("target_port".to_owned(), self.target_port.into());
        config.into()
    }

}

#[derive(Clone)]
pub struct TcpForwardProxy {
    name_manager: Arc<NameManager>,
    peer_manager: Arc<PeerManager>,
    config: Arc<ForwardProxyConfig>,

    // Use to stop the proxy running task
    task: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl TcpForwardProxy {
    pub fn new(
        config: ForwardProxyConfig,
        name_manager: Arc<NameManager>,
        peer_manager: Arc<PeerManager>,
    ) -> Self {
        assert!(config.protocol == ForwardProxyProtocol::Tcp);

        Self {
            name_manager,
            peer_manager,
            config: Arc::new(config),
            task: Arc::new(Mutex::new(None)),
        }
    }

    pub fn id(&self) -> &str {
        &self.config.id
    }

    pub fn config(&self) -> &ForwardProxyConfig {
        &self.config
    }

    pub fn source(&self) -> ConfigSource {
        self.config.source
    }

    pub fn dump(&self) -> serde_json::Value {
        self.config.dump()
    }
    
    pub async fn start(&self) -> GatewayResult<()> {
        let listener = TcpListener::bind(&self.config.addr).await.map_err(|e| {
            let msg = format!("Error binding to {}: {}", self.config.addr, e);
            error!("{}", msg);
            GatewayError::Io(e)
        })?;

        info!(
            "Listen for tcp forward connections at {}, {}",
            &self.config.addr, self.config.id
        );

        let this = self.clone();
        let task = tokio::task::spawn(async move {
            if let Err(e) = this.run(listener).await {
                error!("Error running socks5 proxy: {}", e);
            }
        });

        let prev;
        {
            let mut slot = self.task.lock().unwrap();
            prev = slot.take();
            *slot = Some(task);
        }

        if let Some(prev) = prev {
            warn!(
                "Previous tcp forward proxy task still running, aborting now: {}",
                self.config.id
            );
            prev.abort();
        }

        Ok(())
    }

    pub fn stop(&self) {
        let task = {
            let mut slot = self.task.lock().unwrap();
            slot.take()
        };

        if let Some(task) = task {
            task.abort();
            info!("Tcp forward proxy task stopped: {}", self.config.id);
        } else {
            warn!("Tcp forward proxy task not running: {}", self.config.id);
        }
    }

    async fn run(&self, listener: TcpListener) -> GatewayResult<()> {
        // Standard TCP loop
        loop {
            match listener.accept().await {
                Ok((socket, addr)) => {
                    let this = self.clone();
                    tokio::task::spawn(async move {
                        if let Err(e) = this.on_new_connection(socket, addr).await {
                            error!("Error processing socks5 connection: {}", e);
                        }
                    });
                }
                Err(err) => {
                    error!("Error accepting connection: {}", err);
                }
            }
        }
    }

    async fn on_new_connection(&self, mut conn: TcpStream, addr: SocketAddr) -> GatewayResult<()> {
        info!(
            "Recv tcp forward connection from {} to {}:{}",
            addr, self.config.target_device, self.config.target_port
        );

        let mut tunnel = match self.build_data_tunnel().await {
            Ok(tunnel) => tunnel,
            Err(e) => {
                error!(
                    "Error building data tunnel: {}:{}, {}",
                    self.config.target_device, self.config.target_port, e
                );
                return Err(e);
            }
        };

        let (read, write) = tokio::io::copy_bidirectional(&mut conn, &mut tunnel)
            .await
            .map_err(|e| {
                let msg = format!(
                    "Error copying data on tcp forward connection: {}:{}, {}",
                    self.config.target_device, self.config.target_port, e
                );
                error!("{}", msg);
                GatewayError::Io(e)
            })?;

        info!(
            "Tcp forward connection to {}:{} closed, {} bytes read, {} bytes written",
            self.config.target_device, self.config.target_port, read, write
        );

        Ok(())
    }

    async fn build_data_tunnel(&self) -> GatewayResult<TunnelCombiner> {
        let peer = self
            .peer_manager
            .get_or_init_peer(&self.config.target_device, true)
            .await?;

        let (reader, writer) = peer.build_data_tunnel(self.config.target_port).await?;

        let tunnel = TunnelCombiner::new(reader, writer);

        Ok(tunnel)
    }
}
