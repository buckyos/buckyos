use super::protocol::*;
use super::tunnel::{TunnelReader, TunnelSide, TunnelWriter};
use crate::error::GatewayResult;

use std::sync::Arc;
use tokio::sync::{Mutex, OnceCell};

#[async_trait::async_trait]
pub trait ControlTunnelEvents: Send + Sync {
    async fn on_req_data_tunnel(&self, port: u16, seq: u32) -> GatewayResult<()>;
}

pub type ControlTunnelEventsRef = Arc<Box<dyn ControlTunnelEvents>>;

#[derive(Clone)]
pub struct ControlTunnel {
    tunnel_side: TunnelSide,

    device_id: String,
    remote_device_id: String,

    tunnel_reader: Arc<Mutex<Box<dyn TunnelReader>>>,
    tunnel_writer: Arc<Mutex<Box<dyn TunnelWriter>>>,

    events: OnceCell<ControlTunnelEventsRef>,
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

        let result ;
        loop {
            match ControlPackageTransceiver::read_package(&mut reader).await {
                Ok(pkg) => {
                    match pkg.cmd {
                        ControlCmd::Ping => {
                            // TODO: handle ping
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
                Err(e) => {
                    error!("Error reading control package: {}", e);
                    result = Err(e);
                    break;
                }
            }
        }

        result
    }

    pub async fn req_new_data_tunnel(&self, seq: u32, port: u16) -> GatewayResult<()> {
        let build_pkg = ControlPackage::new(
            ControlCmd::ReqBuild,
            TunnelUsage::Data,
            Some(self.remote_device_id.clone()),
            Some(port),
            seq,
        );
        self.write_pkg(build_pkg).await
    }

    async fn write_pkg(&self, pkg: ControlPackage) -> GatewayResult<()> {
        let mut writer = self.tunnel_writer.lock().await;
        ControlPackageTransceiver::write_package(&mut writer, pkg).await
    }
}
