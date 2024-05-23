use super::protocol::*;
use super::tunnel::{TunnelReader, TunnelSide, TunnelWriter};
use crate::error::{GatewayResult, GatewayError};

use std::sync::Arc;
use tokio::sync::{Mutex, OnceCell};

#[async_trait::async_trait]
pub trait ControlTunnelEvents: Send + Sync {
    async fn on_req_data_tunnel(&self, port: u16, seq: u32) -> GatewayResult<()>;
}

pub type ControlTunnelEventsRef = Arc<Box<dyn ControlTunnelEvents>>;

const CONTROL_TUNNEL_PING_INTERVAL: u64 = 60;
const CONTROL_TUNNEL_PING_TIMEOUT: u64 = 60 * 5;

#[derive(Clone)]
pub struct ControlTunnel {
    tunnel_side: TunnelSide,

    device_id: String,
    remote_device_id: String,

    tunnel_reader: Arc<Mutex<Box<dyn TunnelReader>>>,
    tunnel_writer: Arc<Mutex<Box<dyn TunnelWriter>>>,

    events: OnceCell<ControlTunnelEventsRef>,
}

impl std::fmt::Debug for ControlTunnel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlTunnel")
            .field("tunnel_side", &self.tunnel_side)
            .field("device_id", &self.device_id)
            .field("remote_device_id", &self.remote_device_id)
            .finish()
    }
}

impl ControlTunnel {
    pub fn new(
        tunnel_side: TunnelSide,
        device_id: String,
        remote_device_id: String,
        tunnel_reader: Box<dyn TunnelReader>,
        tunnel_writer: Box<dyn TunnelWriter>,
    ) -> Self {
        Self {
            tunnel_side,
            device_id,
            remote_device_id,
            tunnel_reader: Arc::new(Mutex::new(tunnel_reader)),
            tunnel_writer: Arc::new(Mutex::new(tunnel_writer)),
            events: OnceCell::new(),
        }
    }

    pub fn bind_events(&self, events: ControlTunnelEventsRef) {
        if let Err(e) = self.events.set(events) {
            unreachable!("Error binding control tunnel events: {}", e);
        }
    }

    pub fn tunnel_side(&self) -> TunnelSide {
        self.tunnel_side
    }

    pub async fn run(&self) -> GatewayResult<()> {
        let mut reader = self.tunnel_reader.lock().await;

        let mut last_active = std::time::Instant::now();

        let result;
        loop {
            match tokio::time::timeout(
                tokio::time::Duration::from_secs(CONTROL_TUNNEL_PING_INTERVAL),
                ControlPackageTransceiver::read_package(&mut reader),
            )
            .await
            {
                Ok(Ok(pkg)) => {
                    match pkg.cmd {
                        ControlCmd::Ping => {
                            info!("Recv ping via control tunnel: {} -> {}", self.remote_device_id, self.device_id);
                            assert!(self.tunnel_side == TunnelSide::Passive);
                            last_active = std::time::Instant::now();
                        }
                        ControlCmd::ReqBuild => {
                            let events = self.events.get().unwrap();
                            let ret = events
                                .on_req_data_tunnel(pkg.port.unwrap_or(0), pkg.seq)
                                .await;

                            if let Err(e) = ret {
                                error!("Error on new data tunnel: {}", e);
                                result = Err(e);
                                break;
                            }
                        }
                        _ => {
                            error!("Invalid control command: {:?}", pkg.cmd);
                        }
                    }
                }
                Ok(Err(e)) => {
                    error!("Error reading control package: {}", e);
                    result = Err(e);
                    break;
                }
                Err(_) => {
                    match self.tunnel_side {
                        TunnelSide::Active => {
                            if let Err(e) = self.ping().await {
                                error!("Error sending ping package via control tunnel: {}", e);
                                result = Err(e);
                                break;
                            }
                        }
                        TunnelSide::Passive => {
                            // check if ping timeout with 5min
                            if last_active.elapsed().as_secs() > CONTROL_TUNNEL_PING_TIMEOUT {
                                error!("Control tunnel ping timeout");
                                result = Err(GatewayError::Timeout("Control tunnel ping timeout".to_string()));
                                break;
                            }
                        }
                    }
                }
            }
        }

        result
    }

    pub async fn req_new_data_tunnel(&self, seq: u32, port: u16) -> GatewayResult<()> {
        info!(
            "Requesting new data tunnel via control: remote={}, port={}, seq={}",
            self.remote_device_id, port, seq
        );

        let build_pkg = ControlPackage::new(
            ControlCmd::ReqBuild,
            TunnelUsage::Data,
            Some(self.device_id.clone()),
            Some(port),
            seq,
        );
        self.write_pkg(build_pkg).await
    }

    async fn ping(&self) -> GatewayResult<()> {
        let ping_pkg = ControlPackage::new(
            ControlCmd::Ping,
            TunnelUsage::Control,
            None,
            None,
            0,
        );
        self.write_pkg(ping_pkg).await
    }

    async fn write_pkg(&self, pkg: ControlPackage) -> GatewayResult<()> {
        let mut writer = self.tunnel_writer.lock().await;
        ControlPackageTransceiver::write_package(&mut writer, pkg).await
    }
}
