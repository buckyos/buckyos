use crate::error::{GatewayError, GatewayResult};
use super::super::server::TunnelServerEventsRef;
use super::tunnel::TcpTunnel;

use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use std::net::SocketAddr;

// tunnel server used to accept tunnel connections from clients
#[derive(Clone)]
struct TcpTunnelServer {
    addr: SocketAddr,
    events: TunnelServerEventsRef,
}

impl TcpTunnelServer {
    pub fn new(addr: SocketAddr, events: TunnelServerEventsRef) -> Self {
        TcpTunnelServer {
            addr,
            events,
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

                    let events = self.events.clone();
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