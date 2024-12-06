use super::util::Socks5Util;
use fast_socks5::{
    server::{Config, SimpleUserPassword, Socks5Socket},
    util::target_addr::TargetAddr,
    Socks5Command,
};
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use tokio::{
    net::{TcpListener, TcpStream},
    task,
    task::JoinHandle,
};
use crate::error::{SocksError, SocksResult};
use super::config::{SocksProxyConfig, SocksProxyAuth};
use tokio::io::{AsyncRead, AsyncWrite};

pub trait SocksDataTunnel: AsyncRead + AsyncWrite + Send + Sync + Unpin {}
        

#[derive(Clone)]
pub struct Socks5Proxy {
    config: Arc<SocksProxyConfig>,
    socks5_config: Arc<Config<SimpleUserPassword>>,

    // Use to stop the proxy
    task: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl Socks5Proxy {
    pub fn new(
        config: SocksProxyConfig,
    ) -> Self {
        let mut socks5_config = Config::default();

        // We should process the command and dns resolve by ourselves
        socks5_config.set_dns_resolve(false);
        socks5_config.set_execute_command(false);

        let socks5_config = match config.auth {
            SocksProxyAuth::None => socks5_config,
            SocksProxyAuth::Password(ref username, ref password) => {
                socks5_config.with_authentication(SimpleUserPassword {
                    username: username.clone(),
                    password: password.clone(),
                })
            }
        };

        Self {
            config: Arc::new(config),
            socks5_config: Arc::new(socks5_config),
            task: Arc::new(Mutex::new(None)),
        }
    }

    pub fn id(&self) -> &str {
        &self.config.id
    }

    pub fn addr(&self) -> &SocketAddr {
        &self.config.addr
    }

    pub fn dump(&self) -> serde_json::Value {
        self.config.dump()
    }
    
    pub async fn start(&self) -> SocksResult<()> {
        let listener = TcpListener::bind(&self.config.addr).await.map_err(|e| {
            let msg = format!("Error socks5 binding to {}: {}", self.config.addr, e);
            error!("{}", msg);
            SocksError::IoError(msg)
        })?;

        info!("Listen for socks5 connections at {}", &self.config.addr);

        let this = self.clone();
        let proxy_task = task::spawn(async move {
            if let Err(e) = this.run(listener).await {
                error!("Error running socks5 proxy: {}", e);
            }
        });

        let prev;
        {
            let mut slot = self.task.lock().unwrap();
            prev = slot.replace(proxy_task);
        }

        if let Some(prev) = prev {
            warn!(
                "Previous socks5 proxy task still running, aborting now: {}",
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
            info!("Socks5 proxy task stopped: {}", self.config.id);
        } else {
            warn!("Socks5 proxy task not running: {}", self.config.id);
        }
    }

    async fn run(&self, listener: TcpListener) -> SocksResult<()> {
        // Standard TCP loop
        loop {
            match listener.accept().await {
                Ok((socket, addr)) => {
                    if let Err(e) = self.on_new_connection(socket, addr).await {
                        error!("Error processing socks5 connection: {}", e);
                    }
                }
                Err(err) => {
                    error!("Error accepting connection: {}", err);
                }
            }
        }
    }

    async fn on_new_connection(&self, conn: TcpStream, addr: SocketAddr) -> SocksResult<()> {
        info!("Socks5 connection from {}", addr);
        let socket = Socks5Socket::new(conn, self.socks5_config.clone());

        match socket.upgrade_to_socks5().await {
            Ok(mut socket) => {
                let target = match socket.target_addr() {
                    Some(target) => {
                        info!("Recv socks5 connection from {} to {}", addr, target);
                        target.to_owned()
                    }
                    None => {
                        let msg =
                            format!("Error getting socks5 connection target address: {},", addr,);
                        error!("{}", msg);
                        return Err(SocksError::InvalidParam(msg));
                    }
                };

                let cmd = socket.cmd().as_ref().unwrap();
                match cmd {
                    Socks5Command::TCPConnect => self.process_socket(socket, target.clone()).await,
                    _ => {
                        let msg = format!("Unsupported socks5 command: {:?}", cmd);
                        error!("{}", msg);
                        Socks5Util::reply_error(
                            &mut socket,
                            fast_socks5::ReplyError::CommandNotSupported,
                        )
                        .await
                    }
                }
            }
            Err(err) => {
                let msg = format!("Upgrade to socks5 error: {}", err);
                error!("{}", msg);
                Err(SocksError::SocksError(msg))
            }
        }
    }

    async fn build_data_tunnel(&self, target: &TargetAddr) -> SocksResult<Box<dyn SocksDataTunnel>> {
        
        
        todo!( );
    }

    async fn process_socket(
        &self,
        mut socket: fast_socks5::server::Socks5Socket<TcpStream, SimpleUserPassword>,
        target: TargetAddr,
    ) -> SocksResult<()> {
        let mut tunnel = match self.build_data_tunnel(&target).await {
            Ok(tunnel) => {
                Socks5Util::reply_error(&mut socket, fast_socks5::ReplyError::Succeeded).await?;
                tunnel
            }
            Err(e) => {
                error!("Error building data tunnel: {}", e);
                return Socks5Util::reply_error(
                    &mut socket,
                    fast_socks5::ReplyError::GeneralFailure,
                )
                .await;
            }
        };

        let (read, write) = tokio::io::copy_bidirectional(&mut tunnel, &mut socket)
            .await
            .map_err(|e| {
                let msg = format!("Error copying data on socks connection: {}, {}", target, e);
                error!("{}", msg);
                SocksError::IoError(msg)
            })?;

        info!(
            "socks5 connection to {} closed, {} bytes read, {} bytes written",
            target, read, write
        );

        Ok(())
    }
}
