use super::config::{SocksProxyAuth, SocksProxyConfig};
use super::util::Socks5Util;
use crate::error::{SocksError, SocksResult};
use crate::rule::{RuleAction, RuleInput};
use buckyos_kit::AsyncStream;
use fast_socks5::{
    server::{Config, SimpleUserPassword, Socks5Socket},
    util::target_addr::TargetAddr,
    Socks5Command,
};
use once_cell::sync::OnceCell;
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use tokio::{
    net::{TcpListener, TcpStream},
    task,
    task::JoinHandle,
};
use url::Url;

#[async_trait::async_trait]
pub trait SocksDataTunnelProvider: Send + Sync {
    async fn build(
        &self,
        target: &TargetAddr,
        proxy_target: &Url,
        enable_tunnel: &Option<Vec<String>>,
    ) -> SocksResult<Box<dyn AsyncStream>>;
}

pub type SocksDataTunnelProviderRef = Arc<Box<dyn SocksDataTunnelProvider>>;

#[derive(Clone)]
pub struct Socks5Proxy {
    config: Arc<SocksProxyConfig>,
    socks5_config: Arc<Config<SimpleUserPassword>>,

    // Use to stop the proxy
    task: Arc<Mutex<Option<JoinHandle<()>>>>,

    // The data tunnel provider
    data_tunnel_provider: Arc<OnceCell<SocksDataTunnelProviderRef>>,
}

impl Socks5Proxy {
    pub fn new(config: SocksProxyConfig) -> Self {
        let mut socks5_config = Config::default();

        // We should process the command and dns resolve by ourselves
        socks5_config.set_dns_resolve(false);
        socks5_config.set_execute_command(false);

        let socks5_config = match config.auth {
            SocksProxyAuth::None => socks5_config,
            SocksProxyAuth::Password(ref username, ref password) => socks5_config
                .with_authentication(SimpleUserPassword {
                    username: username.clone(),
                    password: password.clone(),
                }),
        };

        Self {
            config: Arc::new(config),
            socks5_config: Arc::new(socks5_config),
            task: Arc::new(Mutex::new(None)),
            data_tunnel_provider: Arc::new(OnceCell::new()),
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

    // Should only call once
    pub fn set_data_tunnel_provider(&self, provider: SocksDataTunnelProviderRef) {
        if let Err(_) = self.data_tunnel_provider.set(provider) {
            unreachable!(
                "Data tunnel provider already set for socks5 proxy: {}",
                self.config.id
            );
        }
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
                    let this = self.clone();
                    tokio::spawn(async move {
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
                    Socks5Command::TCPConnect => {
                        self.process_socket(socket, addr, target.clone()).await
                    }
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

    async fn build_data_tunnel(&self, target: &TargetAddr) -> SocksResult<Box<dyn AsyncStream>> {
        info!("Will build tunnel to {}", target);

        if let Some(builder) = self.data_tunnel_provider.get() {
            builder
                .build(target, &self.config.target, &self.config.enable_tunnel)
                .await
        } else {
            let msg = format!(
                "Data tunnel provider not set for socks5 proxy: {}",
                self.config.id
            );
            error!("{}", msg);
            Err(SocksError::InvalidState(msg))
        }
    }

    async fn process_socket(
        &self,
        mut socket: fast_socks5::server::Socks5Socket<TcpStream, SimpleUserPassword>,
        addr: SocketAddr,
        target: TargetAddr,
    ) -> SocksResult<()> {
        // Select by rule engine
        if let Some(ref rule_engine) = self.config.rule_engine {
            let input = RuleInput::new_socks_request(&addr, &target);
            match rule_engine.select(input).await {
                Ok(action) => match action {
                    RuleAction::Direct | RuleAction::Pass => {
                        info!("Will process socks5 connection to {} directly", target);
                        self.process_socket_direct(socket, target).await
                    }
                    RuleAction::Proxy(proxy_target) => {
                        info!(
                            "Will process socks5 connection to {} via proxy {}",
                            target, proxy_target
                        );
                        self.process_socket_via_proxy(socket, target).await
                    }
                    RuleAction::Reject => {
                        let msg = format!("Rule engine blocked connection to {}", target);
                        error!("{}", msg);
                        Socks5Util::reply_error(
                            &mut socket,
                            fast_socks5::ReplyError::HostUnreachable,
                        )
                        .await
                    }
                },
                Err(e) => {
                    let msg = format!("Error selecting rule: {}", e);
                    error!("{}", msg);
                    Socks5Util::reply_error(&mut socket, fast_socks5::ReplyError::GeneralFailure)
                        .await
                }
            }
        } else {
            warn!(
                "Rule engine is not set, now Will process socks5 connection to {} directly",
                target
            );
            self.process_socket_direct(socket, target).await
        }
    }

    async fn process_socket_direct(
        &self,
        mut socket: fast_socks5::server::Socks5Socket<TcpStream, SimpleUserPassword>,
        target: TargetAddr,
    ) -> SocksResult<()> {
        // Connect to target directly
        let mut stream = match &target {
            TargetAddr::Ip(ip) => TcpStream::connect(ip).await.map_err(|e| {
                let msg = format!("Error connecting to target with ip: {}, {}", ip, e);
                error!("{}", msg);
                SocksError::IoError(msg)
            })?,
            TargetAddr::Domain(domain, port) => {
                // Resolve domain

                let addr = format!("{}:{}", domain, port);
                TcpStream::connect(&addr).await.map_err(|e| {
                    let msg = format!("Error connecting to target with domain: {}, {}", addr, e);
                    error!("{}", msg);
                    SocksError::IoError(msg)
                })?
            }
        };

        // Reply success after connected
        Socks5Util::reply_error(&mut socket, fast_socks5::ReplyError::Succeeded).await?;

        let (read, write) = tokio::io::copy_bidirectional(&mut stream, &mut socket)
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
    
    async fn process_socket_via_proxy(
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

        // Reply success after data tunnel connected
        Socks5Util::reply_error(&mut socket, fast_socks5::ReplyError::Succeeded).await?;

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
