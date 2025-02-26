use super::config_loader::GatewayConfig;
use super::dispatcher::ServiceDispatcher;
use cyfs_dns::start_cyfs_dns_server;
use cyfs_dns::DNSServer;
use cyfs_gateway_lib::ServerConfig;
use cyfs_gateway_lib::{GatewayDevice, GatewayDeviceRef, TunnelManager};
use cyfs_socks::Socks5Proxy;
use cyfs_warp::start_cyfs_warp_server;
use cyfs_warp::CyfsWarpServer;
use name_client::*;
use name_lib::*;
use once_cell::sync::OnceCell;
use tokio::sync::Mutex;
use url::Url;

pub struct GatewayParams {
    pub disable_buckyos: bool,
    pub keep_tunnel: Vec<String>,
}

pub struct Gateway {
    config: GatewayConfig,
    tunnel_manager: OnceCell<TunnelManager>,

    device_config: OnceCell<DeviceConfig>,

    // servers
    warp_servers: Mutex<Vec<CyfsWarpServer>>,
    dns_servers: Mutex<Vec<DNSServer>>,
    socks_servers: Mutex<Vec<Socks5Proxy>>,
}

impl Gateway {
    pub fn new(config: GatewayConfig) -> Self {
        Self {
            config,
            tunnel_manager: OnceCell::new(),
            device_config: OnceCell::new(),
            warp_servers: Mutex::new(Vec::new()),
            dns_servers: Mutex::new(Vec::new()),
            socks_servers: Mutex::new(Vec::new()),
        }
    }

    pub fn tunnel_manager(&self) -> &TunnelManager {
        self.tunnel_manager.get().unwrap()
    }

    pub async fn start(&self, params: GatewayParams) {
        if params.disable_buckyos {
            self.init_without_buckyos().await;
        } else {
            self.init_with_buckyos().await;
        }

        // Init tunnel manager
        self.init_tunnel_manager().await;

        if !params.keep_tunnel.is_empty() {
            self.keep_tunnels(params.keep_tunnel).await;
        }

        // Start servers
        self.start_servers().await;

        // Start dispatchers
        self.start_dispatcher().await;
    }

    async fn init_with_buckyos(&self) {
        init_global_buckyos_value_by_env("GATEWAY");
        let this_device = CURRENT_DEVICE_CONFIG.get();
        let this_device = this_device.unwrap();

        let set_result = self.device_config.set(this_device.clone());
        if set_result.is_err() {
            error!("device_config can only be set once");
        }

        let this_device_info = DeviceInfo::from_device_doc(this_device);
        let session_token = CURRENT_APP_SESSION_TOKEN.get();
        let _ = enable_zone_provider(Some(&this_device_info), session_token, true).await;
    }

    async fn init_without_buckyos(&self) {
        info!("TODO:disable buckyos,set device config for test");
        init_default_name_client().await.unwrap();

        let pk = if let Some(sk) = &self.config.device_private_key {
            let pk_value = encode_ed25519_pkcs8_sk_to_pk(sk);
            info!("Will use device pk: {}", pk_value);
            pk_value
        } else {
            // TODO use default pk or set it to none?
            let pk_value = "8vlobDX73HQj-w5TUjC_ynr_ljsWcDAgVOzsqXCw7no".to_string();
            info!("Will use default device pk: {}", pk_value);
            pk_value
        };

        let mut device_name = "web3.buckyos.io".to_string();
        if let Some(device_did) = &self.config.device_did {
            device_name = device_did.clone();
        }

        let this_device_config = DeviceConfig::new(device_name.as_str(), Some(pk));
        let set_result = self.device_config.set(this_device_config.clone());
        if set_result.is_err() {
            error!("device_config can only be set once");
        }

        // Also set it to global for now..
        let set_result = CURRENT_DEVICE_CONFIG.set(this_device_config);
        if set_result.is_err() {
            error!("Failed to set CURRENT_DEVICE_CONFIG");
        }
    }

    async fn init_tunnel_manager(&self) {
        let gateway_device = GatewayDevice {
            config: self.device_config.get().unwrap().clone(),
            private_key: self.config.device_private_key.clone().unwrap(),
        };
        let gateway_device = GatewayDeviceRef::new(gateway_device);
        let tunnel_manager = TunnelManager::new(gateway_device.clone());
        let set_result = self.tunnel_manager.set(tunnel_manager.clone());
        if set_result.is_err() {
            error!("tunnel_manager can only be set once");
        }

        if let Err(_) = cyfs_gateway_lib::CURRENT_GATEWAY_DEVICE.set(gateway_device) {
            unreachable!("CURRENT_GATEWAY_DEVICE can only be set once");
        }

        if let Err(_) = cyfs_gateway_lib::GATEWAY_TUNNEL_MANAGER.set(tunnel_manager) {
            unreachable!("GATEWAY_TUNNEL_MANAGER can only be set once");
        }
    }

    async fn keep_tunnels(&self, keep_tunnel: Vec<String>) {
        for tunnel in keep_tunnel {
            self.keep_tunnel(tunnel.as_str()).await;
        }
    }

    async fn keep_tunnel(&self, tunnel: &str) {
        let tunnel_url = format!("rtcp://{}", tunnel);
        info!("Will keep tunnel: {}", tunnel_url);
        let tunnel_url = Url::parse(tunnel_url.as_str());
        if tunnel_url.is_err() {
            warn!("Invalid tunnel url: {}", tunnel_url.err().unwrap());
            return;
        }

        let tunnel_manager = self.tunnel_manager().clone();
        tokio::task::spawn(async move {
            let tunnel_url = tunnel_url.unwrap();
            loop {
                let last_ok;
                let tunnel = tunnel_manager.get_tunnel(&tunnel_url, None).await;
                if tunnel.is_err() {
                    warn!("Error getting tunnel: {}", tunnel.err().unwrap());
                    last_ok = false;
                } else {
                    let tunnel = tunnel.unwrap();
                    let ping_result = tunnel.ping().await;
                    if ping_result.is_err() {
                        warn!("Error pinging tunnel: {}", ping_result.err().unwrap());
                        last_ok = false;
                    } else {
                        last_ok = true;
                    }
                }

                if last_ok {
                    tokio::time::sleep(std::time::Duration::from_secs(60 * 2)).await;
                } else {
                    tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                }
            }
        });
    }

    async fn start_servers(&self) {
        for (server_id, server_config) in self.config.servers.iter() {
            info!("Will start server: {}, {:?}", server_id, server_config);

            match server_config {
                ServerConfig::Warp(warp_config) => {
                    let warp_config = warp_config.clone();
                    match cyfs_warp::start_cyfs_warp_server(warp_config).await {
                        Ok(warp_server) => {
                            let mut warp_servers = self.warp_servers.lock().await;
                            warp_servers.push(warp_server);
                        }
                        Err(e) => {
                            // FIXME: should we return error here? or just ignore it?
                            error!("Error starting warp server: {}", e);
                        }
                    }
                }
                ServerConfig::DNS(dns_config) => {
                    let dns_config = dns_config.clone();

                    let ret = cyfs_dns::start_cyfs_dns_server(dns_config).await;
                    match ret {
                        Ok(dns_server) => {
                            let mut dns_servers = self.dns_servers.lock().await;
                            dns_servers.push(dns_server);
                        }
                        Err(e) => {
                            // FIXME: should we return error here? or just ignore it?
                            error!("Error starting dns server: {}", e);
                        }
                    }
                }
                ServerConfig::Socks(socks_config) => {
                    let tunnel_provider =
                        crate::socks::SocksTunnelBuilder::new_ref(self.tunnel_manager().clone());

                    let socks_config = socks_config.clone();
                    let ret =
                        cyfs_socks::start_cyfs_socks_server(socks_config, tunnel_provider).await;

                    match ret {
                        Ok(socks_server) => {
                            let mut socks_servers = self.socks_servers.lock().await;
                            socks_servers.push(socks_server);
                        }
                        Err(e) => {
                            // FIXME: should we return error here? or just ignore it?
                            error!("Error starting socks server: {}", e);
                        }
                    }
                }
            }
        }
    }

    async fn start_dispatcher(&self) {
        let dispatcher = ServiceDispatcher::new(
            self.tunnel_manager().clone(),
            self.config.dispatcher.clone(),
        );
        dispatcher.start().await;
    }
}
