use super::super::server::TunnelServerEventsRef;
use super::tunnel::TcpTunnel;
use crate::error::*;
use crate::tunnel::TunnelServer;

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::OnceCell;

// tunnel server used to accept tunnel connections from clients
#[derive(Clone)]
pub struct TcpTunnelServer {
    addr: SocketAddr,
    events: Arc<OnceCell<TunnelServerEventsRef>>,
}

impl TcpTunnelServer {
    pub fn new(addr: SocketAddr) -> Self {
        TcpTunnelServer {
            addr,
            events: Arc::new(OnceCell::new()),
        }
    }

    pub async fn start(&self) -> GatewayResult<()> {
        let listener = TcpListener::bind(&self.addr).await.map_err(|e| {
            error!("Error binding tcp tunnel server to {}: {}", self.addr, e);
            e
        })?;

        let this = self.clone();
        tokio::spawn(async move {
            match this.run(listener).await {
                Ok(_) => {
                    info!("Tcp tunnel server closed {}", this.addr);
                }
                Err(e) => {
                    error!("Error running tcp tunnel server: {}, {}", e, this.addr);
                }
            }
        });

        Ok(())
    }

    async fn run(&self, listener: TcpListener) -> GatewayResult<()> {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let remote = stream.peer_addr().unwrap().to_string();
                    info!("Recv tcp tunnel connection from {}", remote);

                    let events = self.events.get().unwrap().clone();
                    tokio::spawn(async move {
                        let tunnel = TcpTunnel::new(remote.clone(), stream);
                        match events.on_new_tunnel(Box::new(tunnel)).await {
                            Ok(_) => {
                                info!("New tunnel connection closed {}", remote);
                            }
                            Err(e) => {
                                error!("Error handling tcp tunnel connection: {} {}", e, remote);
                            }
                        }
                    });
                }
                Err(e) => {
                    error!("Error accepting tcp tunnel connection: {}", e);

                    // sleep 5 seconds and try again
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                }
            }
        }
    }
}

#[async_trait::async_trait]
impl TunnelServer for TcpTunnelServer {
    fn bind_events(&self, events: TunnelServerEventsRef) {
        if let Err(_) = self.events.set(events) {
            unreachable!("Error setting events for TcpTunnelServer");
        }
    }

    async fn start(&self) -> GatewayResult<()> {
        self.start().await
    }

    async fn stop(&self) -> GatewayResult<()> {
        unimplemented!("stop not implemented for TcpTunnelServer")
    }
}
