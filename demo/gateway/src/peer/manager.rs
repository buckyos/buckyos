use super::peer::PeerClient;
use super::name::NameManagerRef;
use crate::config::GlobalConfigRef;
use crate::error::*;
use crate::tunnel::*;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::OnceCell;

pub struct OnNewTunnelHandleResult {
    pub handled: bool,
    pub info: Option<DataTunnelInfo>,
}

pub trait PeerManagerEvents: Send + Sync {
    fn on_recv_data_tunnel(&self, info: DataTunnelInfo) -> GatewayResult<OnNewTunnelHandleResult>;
}

pub type PeerManagerEventsRef = Arc<Box<dyn PeerManagerEvents>>;

#[derive(Clone)]
pub struct PeerManagerEventManager {
    events: Arc<Mutex<Vec<PeerManagerEventsRef>>>,
}

impl PeerManagerEventManager {
    pub fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn bind_events(&self, event: PeerManagerEventsRef) {
        let mut events = self.events.lock().unwrap();
        events.push(event);
    }

    pub fn on_recv_data_tunnel(&self, mut info: DataTunnelInfo) -> GatewayResult<bool> {
        let events = self.events.lock().unwrap();
        for event in events.iter() {
            let ret = event.on_recv_data_tunnel(info)?;
            if ret.handled {
                return Ok(true);
            }

            info = ret.info.unwrap();
        }

        Ok(false)
    }
}

#[derive(Clone)]
pub struct PeerManager {
    config: GlobalConfigRef,
    peers: Arc<Mutex<HashMap<String, OnceCell<Arc<PeerClient>>>>>,

    events: PeerManagerEventManager,
    name_manager: NameManagerRef,
}

impl PeerManager {
    pub fn new(config: GlobalConfigRef, name_manager: NameManagerRef) -> Self {
        Self {
            config,
            peers: Arc::new(Mutex::new(HashMap::new())),
            events: PeerManagerEventManager::new(),
            name_manager,
        }
    }

    pub fn events(&self) -> &PeerManagerEventManager {
        &self.events
    }

    pub fn get_peer(&self, peer_id: &str) -> Option<Arc<PeerClient>> {
        let peers = self.peers.lock().unwrap();
        peers.get(peer_id).map(|peer| peer.get().unwrap().clone())
    }

    pub async fn get_or_init_peer(&self, remote_device_id: &str) -> GatewayResult<Arc<PeerClient>> {
        // first check if peer is already exists
        let peer = {
            let mut peers = self.peers.lock().unwrap();

            match peers.get(remote_device_id) {
                Some(peer) => peer.clone(),
                None => {
                    info!("Create peer client: {}", remote_device_id);
                    let peer = OnceCell::new();
                    peers.insert(remote_device_id.to_owned(), OnceCell::new());
                    peer
                }
            }
        };

        let peer = peer
            .get_or_try_init(|| async {
                let events = Arc::new(Box::new(self.clone()) as Box<dyn TunnelManagerEvents>);
                let peer = PeerClient::new(
                    self.config.device_id().to_owned(),
                    remote_device_id.to_string(),
                    events,
                    self.name_manager.clone(),
                );
                peer.start().await?;

                Ok::<Arc<PeerClient>, GatewayError>(Arc::new(peer))
            })
            .await?;

        Ok(peer.clone())
    }
}

#[async_trait::async_trait]
impl TunnelServerEvents for PeerManager {
    async fn on_new_tunnel(&self, tunnel: Box<dyn Tunnel>) -> GatewayResult<()> {
        let (mut reader, writer) = tunnel.split();
        let build_pkg = ControlPackageTransceiver::read_package(&mut reader).await?;
        match build_pkg.cmd {
            ControlCmd::Init => {
                let device_id = build_pkg.device_id.clone().ok_or({
                    let msg = format!(
                        "Invalid control package, device_id missing: {:?}",
                        build_pkg
                    );
                    error!("{}", msg);
                    GatewayError::InvalidFormat(msg)
                })?;
                let peer = self.get_peer(&device_id).ok_or({
                    let msg = format!("Peer not found: {}", device_id);
                    error!("{}", msg);
                    GatewayError::PeerNotFound(msg)
                })?;

                let info = TunnelInitInfo {
                    pkg: build_pkg,
                    tunnel_reader: Box::new(reader),
                    tunnel_writer: Box::new(writer),
                };

                peer.on_new_tunnel(info).await;

                Ok(())
            }
            _ => {
                let msg = format!("Invalid control command: {:?}", build_pkg.cmd);
                error!("{}", msg);
                Err(GatewayError::InvalidFormat(msg))
            }
        }
    }
}

#[async_trait::async_trait]
impl TunnelManagerEvents for PeerManager {
    async fn on_recv_data_tunnel(&self, info: DataTunnelInfo) -> GatewayResult<()> {
        let device_id = info.device_id.clone();
        let port = info.port;
        let handled = self.events.on_recv_data_tunnel(info)?;
        if !handled {
            let msg = format!("Data tunnel not handled: {} {}", device_id, port);
            error!("{}", msg);
            return Err(GatewayError::InvalidFormat(msg));
        }

        Ok(())
    }
}

pub type PeerManagerRef = Arc<PeerManager>;