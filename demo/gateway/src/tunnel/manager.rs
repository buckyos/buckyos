use super::builder::TunnelBuilder;
use super::control::{ControlTunnel, ControlTunnelEvents};
use super::server::TunnelInitInfo;
use super::tunnel::*;
use crate::error::*;
use crate::peer::NameManagerRef;

use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc,
};
use tokio::sync::{Mutex, Notify, OnceCell};

pub struct DataTunnelInfo {
    pub device_id: String,
    pub port: u16,
    pub tunnel_reader: Box<dyn TunnelReader>,
    pub tunnel_writer: Box<dyn TunnelWriter>,
}

impl std::fmt::Debug for DataTunnelInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DataTunnelInfo")
            .field("device_id", &self.device_id)
            .field("port", &self.port)
            .finish()
    }
}

#[async_trait::async_trait]
pub trait TunnelManagerEvents: Send + Sync {
    async fn on_recv_data_tunnel(&self, info: DataTunnelInfo) -> GatewayResult<()>;
}

pub type TunnelManagerEventsRef = Arc<Box<dyn TunnelManagerEvents>>;

struct TunnelWaitInfo {
    notify: Arc<Notify>,
    tunnel_reader: Option<Box<dyn TunnelReader>>,
    tunnel_writer: Option<Box<dyn TunnelWriter>>,
}

struct ControlTunnelInfo {
    tunnel: ControlTunnel,
    abort_handle: tokio::task::JoinHandle<()>,
}
// one control tunnel, one or more data tunnel
#[derive(Clone)]
pub struct TunnelManager {
    name_manager: NameManagerRef,

    device_id: String,
    remote_device_id: String,
    control_tunnel: Arc<Mutex<Option<ControlTunnelInfo>>>,
    next_seq: Arc<AtomicU32>,

    waiter: Arc<Mutex<HashMap<u32, TunnelWaitInfo>>>,

    events: Arc<OnceCell<TunnelManagerEventsRef>>,
}

impl TunnelManager {
    pub fn new(name_manager: NameManagerRef, device_id: String, remote_device_id: String) -> Self {
        Self {
            name_manager,
            device_id,
            remote_device_id,
            control_tunnel: Arc::new(Mutex::new(None)),
            next_seq: Arc::new(AtomicU32::new(0)),
            waiter: Arc::new(Mutex::new(HashMap::new())),
            events: Arc::new(OnceCell::new()),
        }
    }

    pub fn bind_events(&self, events: TunnelManagerEventsRef) {
        if let Err(e) = self.events.set(events) {
            unreachable!("Error binding tunnel manager events: {}", e);
        }
    }

    async fn on_control_tunnel_finished(&self) {
        let control_tunnel = self.control_tunnel.lock().await.take();
        assert!(control_tunnel.is_some());

        let control_tunnel = control_tunnel.unwrap();
        if control_tunnel.tunnel.tunnel_side() == TunnelSide::Active {
            self.init_control_tunnel().await.unwrap_or_else(|e| {
                error!("Error on init control tunnel: {}", e);
            });
        } else {
            // wait for new control tunnel build on active side
        }
    }

    fn start_control_tunnel(&self, tunnel: ControlTunnel) -> tokio::task::JoinHandle<()> {
        let this = self.clone();
        let abort_handle = tokio::task::spawn(async move {
            match tunnel.run().await {
                Ok(_) => {
                    info!(
                        "Control tunnel finished: {} -> {}",
                        this.device_id, this.remote_device_id
                    );
                }
                Err(e) => {
                    error!(
                        "Control tunnel error: {} -> {}, {}",
                        this.device_id, this.remote_device_id, e
                    );
                }
            }

            this.on_control_tunnel_finished().await;
        });

        abort_handle
    }

    // bind control tunnel on passive side
    pub async fn bind_tunnel_control(
        &self,
        tunnel_reader: Box<dyn TunnelReader>,
        tunnel_writer: Box<dyn TunnelWriter>,
    ) {
        info!(
            "Bind control tunnel: {} -> {}",
            self.remote_device_id, self.device_id
        );

        let tunnel = ControlTunnel::new(
            TunnelSide::Passive,
            self.device_id.clone(),
            self.remote_device_id.clone(),
            tunnel_reader,
            tunnel_writer,
        );

        tunnel.bind_events(Arc::new(Box::new(self.clone())));

        // run control tunnel async
        let abort_handle = self.start_control_tunnel(tunnel.clone());

        let info = ControlTunnelInfo {
            tunnel,
            abort_handle,
        };

        let prev = self.control_tunnel.lock().await.replace(info);

        if let Some(prev) = prev {
            warn!("Replace control tunnel: {:?}, now will abort", prev.tunnel);
            prev.abort_handle.abort();
        }
    }

    // Init control on active side if needed
    pub async fn init_control_tunnel(&self) -> GatewayResult<()> {
        {
            let control_tunnel = self.control_tunnel.lock().await;
            assert!(control_tunnel.is_none());
        }

        let builder = TunnelBuilder::new(
            self.name_manager.clone(),
            self.device_id.clone(),
            self.remote_device_id.clone(),
        );
        let tunnel = builder.build_control_tunnel().await?;
        tunnel.bind_events(Arc::new(Box::new(self.clone())));

        // run control tunnel async
        let abort_handle = self.start_control_tunnel(tunnel.clone());

        let info = ControlTunnelInfo {
            tunnel,
            abort_handle,
        };

        let prev = self.control_tunnel.lock().await.replace(info);

        if let Some(prev) = prev {
            warn!("Replace control tunnel: {:?}, now will abort", prev.tunnel);
            prev.abort_handle.abort();
        }

        Ok(())
    }

    fn get_seq(&self) -> u32 {
        let seq = self.next_seq.fetch_add(1, Ordering::SeqCst);
        if seq == 0 {
            self.next_seq.fetch_add(1, Ordering::SeqCst)
        } else {
            seq
        }
    }

    pub async fn build_data_tunnel(
        &self,
        port: u16,
    ) -> GatewayResult<(Box<dyn TunnelReader>, Box<dyn TunnelWriter>)> {
        info!(
            "Will build data tunnel: {} -> {}, {}",
            self.device_id, self.remote_device_id, port
        );
        assert!(port > 0);

        let side = {
            match self.control_tunnel.lock().await.as_ref() {
                Some(info) => info.tunnel.tunnel_side(),
                None => TunnelSide::Active,
            }
        };

        match side {
            TunnelSide::Active => {
                let builder = TunnelBuilder::new(
                    self.name_manager.clone(),
                    self.device_id.clone(),
                    self.remote_device_id.clone(),
                );
                builder.build_data_tunnel(port, 0).await
            }
            TunnelSide::Passive => {
                // For the data tunnel we build via control tunnel, seq should be greater than 0
                let seq = self.get_seq();
                assert!(seq > 0);

                // control_tunnel should be ready
                let control_tunnel = {
                    let info = self.control_tunnel.lock().await;
                    if info.is_none() {
                        let msg = format!(
                            "Control tunnel not ready {} -> {}",
                            self.device_id, self.remote_device_id
                        );
                        error!("{}", msg);
                        return Err(GatewayError::InvalidState(msg.to_string()));
                    }

                    info.as_ref().unwrap().tunnel.clone()
                };

                // first create new waiter for incoming tunnel
                let notify = Arc::new(Notify::new());
                {
                    let info = TunnelWaitInfo {
                        notify: notify.clone(),
                        tunnel_reader: None,
                        tunnel_writer: None,
                    };

                    self.waiter.lock().await.insert(seq, info);
                }

                // then req new tunnel
                control_tunnel.req_new_data_tunnel(seq, port).await?;

                // then wait new data tunnel
                notify.notified().await;

                // then get tunnel from waiter
                let mut waiter = self.waiter.lock().await;
                let info = waiter.remove(&seq).unwrap();
                if info.tunnel_reader.is_none() || info.tunnel_writer.is_none() {
                    return Err(GatewayError::TunnelError("Invalid tunnel info".to_string()));
                }

                let (tunnel_reader, tunnel_writer) =
                    (info.tunnel_reader.unwrap(), info.tunnel_writer.unwrap());
                Ok((tunnel_reader, tunnel_writer))
            }
        }
    }

    /*
    pub async fn on_new_tunnel(&self, info: TunnelInitInfo) {
        match info.pkg.cmd {
            ControlCmd::Init => match info.pkg.usage {
                TunnelUsage::Control => {
                    self.on_new_control_tunnel(info.tunnel_reader, info.tunnel_writer)
                        .await;
                }
                TunnelUsage::Data => {
                    self.on_new_data_tunnel(
                        info.tunnel_reader,
                        info.tunnel_writer,
                        info.pkg.seq,
                        info.pkg.port.unwrap_or(0),
                    )
                    .await;
                }
            },
            _ => {
                unreachable!("Invalid control command: {:?}", info.pkg.cmd);
            }
        }
    }

    // use by TunnelServer on receiving new tunnel
    async fn on_new_control_tunnel(
        &self,
        tunnel_reader: Box<dyn TunnelReader>,
        tunnel_writer: Box<dyn TunnelWriter>,
    ) {
        let tunnel = ControlTunnel::new(
            TunnelSide::Passive,
            self.device_id.clone(),
            self.remote_device_id.clone(),
            tunnel_reader,
            tunnel_writer,
        );

        tunnel.bind_events(Arc::new(Box::new(self.clone())));

        // TODO error handle when error happened on connection
        let tunnel_ = tunnel.clone();
        tokio::task::spawn(async move {
            tunnel_.run().await.unwrap_or_else(|e| {
                error!("Control tunnel error: {}", e);
            });
        });

        let mut control_tunnel = self.control_tunnel.lock().await;
        assert!(control_tunnel.is_none());
        *control_tunnel = Some(tunnel);
    }
    */

    pub async fn on_new_data_tunnel(&self, info: TunnelInitInfo) {
        let tunnel_reader = info.tunnel_reader;
        let tunnel_writer = info.tunnel_writer;
        let port = info.pkg.port.unwrap_or(0);
        let seq = info.pkg.seq;

        info!(
            "On new data tunnel: {} -> {}, port={}, seq={}",
            self.remote_device_id, self.device_id, port, seq
        );

        if seq == 0 {
            if let Err(e) = self
                .events
                .get()
                .unwrap()
                .on_recv_data_tunnel(DataTunnelInfo {
                    device_id: self.remote_device_id.clone(),
                    port,
                    tunnel_reader: tunnel_reader,
                    tunnel_writer: tunnel_writer,
                })
                .await
            {
                error!("Error on new data tunnel: {} {}", port, e);
            }
        } else {
            let mut waiter = self.waiter.lock().await;
            let info = waiter.get_mut(&seq).unwrap();
            info.tunnel_reader = Some(tunnel_reader);
            info.tunnel_writer = Some(tunnel_writer);

            info.notify.notify_one();
        }
    }
}

#[async_trait::async_trait]
impl ControlTunnelEvents for TunnelManager {
    async fn on_req_data_tunnel(&self, port: u16, seq: u32) -> GatewayResult<()> {
        info!(
            "Recv req for new data tunnel: {} -> {}, port={}, seq={}",
            self.remote_device_id, self.device_id, port, seq
        );

        let builder = TunnelBuilder::new(
            self.name_manager.clone(),
            self.device_id.clone(),
            self.remote_device_id.clone(),
        );
        let (reader, writer) = builder.build_data_tunnel(port, seq).await?;

        self.events
            .get()
            .unwrap()
            .on_recv_data_tunnel(DataTunnelInfo {
                device_id: self.remote_device_id.clone(),
                port,
                tunnel_reader: reader,
                tunnel_writer: writer,
            })
            .await
    }
}
