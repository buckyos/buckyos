use super::forward::{ForwardProxyConfig, ForwardProxyProtocol, TcpForwardProxy};
use super::socks5::{ProxyConfig, Socks5Proxy};
use crate::error::{GatewayError, GatewayResult};
use crate::peer::{NameManagerRef, PeerManagerRef};

use std::sync::{Arc, Mutex};

enum ProxyService {
    Socks5(Socks5Proxy),
    TcpForward(TcpForwardProxy),
}

pub struct ProxyManager {
    name_manager: NameManagerRef,
    peer_manager: PeerManagerRef,
    socks5_proxy: Arc<Mutex<Vec<Socks5Proxy>>>,
    tcp_forward_proxy: Arc<Mutex<Vec<TcpForwardProxy>>>,
}

impl ProxyManager {
    pub fn new(name_manager: NameManagerRef, peer_manager: PeerManagerRef) -> Self {
        Self {
            name_manager,
            peer_manager,
            socks5_proxy: Arc::new(Mutex::new(Vec::new())),
            tcp_forward_proxy: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /*
    load proxy from json config node as follows:
    {
        block: "proxy",
        id: "proxy_id",
        type: "socks5",
        addr: "127.0.0.1",
        port: 8000,
        auth: {
            type: "password",
            username: "user",
            password: "password"
        }
    }

    or

    {
        block: "proxy",
        id: "proxy_id",
        type: "forward",
        protocol: "tcp",
        target_device: "device_id",
        target_port: 8000,
    }
     */
    pub fn load_proxy(&self, json: &serde_json::Value) -> GatewayResult<()> {
        let proxy_type = json["type"].as_str().unwrap();
        match proxy_type {
            "socks5" => {
                let config = ProxyConfig::load(json)?;
                let proxy =
                    Socks5Proxy::new(config, self.name_manager.clone(), self.peer_manager.clone());
                self.add_socks5_proxy(proxy)?;
            }
            "forward" => {
                let config = ForwardProxyConfig::load(json)?;
                match config.protocol {
                    ForwardProxyProtocol::Tcp => {
                        let proxy = TcpForwardProxy::new(
                            config,
                            self.name_manager.clone(),
                            self.peer_manager.clone(),
                        );
                        self.add_tcp_forward_proxy(proxy)?;
                    }
                    ForwardProxyProtocol::Udp => {
                        unimplemented!("UDP forward proxy not implemented yet");
                    }
                }
            }
            _ => {
                warn!("Unknown proxy type: {}", proxy_type);
            }
        }

        Ok(())
    }

    fn get_proxy(&self, id: &str) -> Option<ProxyService> {
        {
            let socks_proxys = self.socks5_proxy.lock().unwrap();
            for p in &*socks_proxys {
                if p.id() == id {
                    return Some(ProxyService::Socks5(p.clone()));
                }
            }
        }

        {
            let forward_proxys = self.tcp_forward_proxy.lock().unwrap();
            for p in &*forward_proxys {
                if p.id() == id {
                    return Some(ProxyService::TcpForward(p.clone()));
                }
            }
        }

        None
    }

    fn check_exist(&self, id: &str) -> bool {
        self.get_proxy(id).is_some()
    }

    pub async fn create_socks5_proxy(&self, config: ProxyConfig) -> GatewayResult<()> {
        let proxy = Socks5Proxy::new(config, self.name_manager.clone(), self.peer_manager.clone());
        proxy.start().await?;

        let mut socks_proxys = self.socks5_proxy.lock().unwrap();

        // Check id duplication
        if socks_proxys.iter().any(|p| p.id() == proxy.id()) {
            proxy.stop();

            let msg = format!("Duplicated socks5 proxy id: {}", proxy.id());
            warn!("{}", msg);
            return Err(GatewayError::AlreadyExists(msg.to_owned()));
        }

        socks_proxys.push(proxy);

        Ok(())
    }

    pub async fn create_tcp_forward_proxy(&self, config: ForwardProxyConfig) -> GatewayResult<()> {
        let proxy =
            TcpForwardProxy::new(config, self.name_manager.clone(), self.peer_manager.clone());
        proxy.start().await?;

        let mut forward_proxys = self.tcp_forward_proxy.lock().unwrap();

        // Check id duplication
        if forward_proxys.iter().any(|p| p.id() == proxy.id()) {
            proxy.stop();

            let msg = format!("Duplicated tcp forward proxy id: {}", proxy.id());
            warn!("{}", msg);
            return Err(GatewayError::AlreadyExists(msg.to_owned()));
        }

        forward_proxys.push(proxy);

        Ok(())
    }

    fn add_socks5_proxy(&self, proxy: Socks5Proxy) -> GatewayResult<()> {
        let mut socks_proxys = self.socks5_proxy.lock().unwrap();

        // Check id duplication
        if socks_proxys.iter().any(|p| p.id() == proxy.id()) {
            let msg = format!("Duplicated socks5 proxy id: {}", proxy.id());
            warn!("{}", msg);
            return Err(GatewayError::AlreadyExists(msg.to_owned()));
        }

        info!("New socks5 proxy: {}, {}", proxy.id(), proxy.addr());

        socks_proxys.push(proxy);

        Ok(())
    }

    fn add_tcp_forward_proxy(&self, proxy: TcpForwardProxy) -> GatewayResult<()> {
        let mut forward_proxys = self.tcp_forward_proxy.lock().unwrap();

        // Check id duplication
        if forward_proxys.iter().any(|p| p.id() == proxy.id()) {
            let msg = format!("Duplicated tcp forward proxy id: {}", proxy.id());
            warn!("{}", msg);
            return Err(GatewayError::AlreadyExists(msg.to_owned()));
        }

        info!("New tcp forward proxy: {:?}", proxy.config());

        forward_proxys.push(proxy);

        Ok(())
    }

    // Stop and remove proxy by id
    pub fn remove_proxy(&self, id: &str) -> GatewayResult<()> {
        let mut found = false;

        {
            let mut socks_proxys = self.socks5_proxy.lock().unwrap();
            socks_proxys.retain(|p| {
                if p.id() == id {
                    p.stop();
                    found = true;
                    false
                } else {
                    true
                }
            });
        }

        if found {
            return Ok(());
        }

        {
            let mut forward_proxys = self.tcp_forward_proxy.lock().unwrap();

            forward_proxys.retain(|p| {
                if p.id() == id {
                    p.stop();
                    found = true;
                    false
                } else {
                    true
                }
            });
        }

        if found {
            return Ok(());
        }

        let msg = format!("Proxy not found: {}", id);
        warn!("{}", msg);
        Err(GatewayError::NotFound(msg.to_owned()))
    }

    pub async fn start(&self) -> GatewayResult<()> {
        let proxy_list = self.socks5_proxy.lock().unwrap().clone();
        for proxy in &proxy_list {
            if let Err(e) = proxy.start().await {
                return Err(e);
            }
        }

        let proxy_list = self.tcp_forward_proxy.lock().unwrap().clone();
        for proxy in &proxy_list {
            if let Err(e) = proxy.start().await {
                return Err(e);
            }
        }

        Ok(())
    }

    pub fn stop(&self) -> GatewayResult<()> {
        let proxy_list = self.socks5_proxy.lock().unwrap().clone();
        for proxy in &proxy_list {
            proxy.stop();
        }

        let proxy_list = self.tcp_forward_proxy.lock().unwrap().clone();
        for proxy in &proxy_list {
            proxy.stop();
        }

        Ok(())
    }
}

pub type ProxyManagerRef = Arc<ProxyManager>;
