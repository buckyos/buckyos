use super::forward::{ForwardProxyConfig, ForwardProxyProtocol, TcpForwardProxy};
use super::socks5::{ProxyAuth, ProxyConfig, Socks5Proxy};
use crate::error::{GatewayError, GatewayResult};
use crate::peer::{NameManagerRef, PeerManagerRef};

use std::str::FromStr;
use std::sync::{Arc, Mutex};

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
                let addr = json["addr"]
                    .as_str()
                    .ok_or(GatewayError::InvalidConfig("addr".to_owned()))?;
                let port = json["port"]
                    .as_u64()
                    .ok_or(GatewayError::InvalidConfig("port".to_owned()))?
                    as u16;
                let addr = format!("{}:{}", addr, port);
                let addr = addr.parse().map_err(|e| {
                    let msg = format!("Error parsing addr: {}, {}", addr, e);
                    error!("{}", msg);
                    GatewayError::InvalidConfig(msg)
                })?;

                let auth = if let Some(auth) = json.get("auth") {
                    if !auth.is_object() {
                        return Err(GatewayError::InvalidConfig("auth".to_owned()));
                    }

                    let auth_type = auth["type"]
                        .as_str()
                        .ok_or(GatewayError::InvalidConfig("auth.type".to_owned()))?;
                    match auth_type {
                        "password" => {
                            let username = json["auth"]["username"].as_str().unwrap();
                            let password = json["auth"]["password"].as_str().unwrap();
                            ProxyAuth::Password(username.to_owned(), password.to_owned())
                        }
                        _ => {
                            let msg = format!("Unknown auth type: {}", auth_type);
                            error!("{}", msg);
                            return Err(GatewayError::InvalidConfig(msg));
                        }
                    }
                } else {
                    ProxyAuth::None
                };

                let config = ProxyConfig { addr, auth };

                self.add_socks5_proxy(config);
            }
            "forward" => {
                let protocol = json["protocol"]
                    .as_str()
                    .ok_or(GatewayError::InvalidConfig("protocol".to_owned()))?;
                let addr = json["addr"]
                    .as_str()
                    .ok_or(GatewayError::InvalidConfig("addr".to_owned()))?;
                let addr = addr.parse().map_err(|e| {
                    let msg = format!("Error parsing addr: {}, {}", addr, e);
                    error!("{}", msg);
                    GatewayError::InvalidConfig(msg)
                })?;

                let target_device = json["target_device"]
                    .as_str()
                    .ok_or(GatewayError::InvalidConfig("target_device".to_owned()))?;
                let target_port = json["target_port"]
                    .as_u64()
                    .ok_or(GatewayError::InvalidConfig("target_port".to_owned()))?
                    as u16;

                let protocol = ForwardProxyProtocol::from_str(protocol)?;
                match protocol {
                    ForwardProxyProtocol::Tcp => {
                        let config = ForwardProxyConfig {
                            protocol,
                            addr,
                            target_device: target_device.to_owned(),
                            target_port,
                        };

                        self.add_tcp_forward_proxy(config);
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

    fn add_socks5_proxy(&self, config: ProxyConfig) {
        let proxy = Socks5Proxy::new(config, self.name_manager.clone(), self.peer_manager.clone());
        self.socks5_proxy.lock().unwrap().push(proxy);
    }

    fn add_tcp_forward_proxy(&self, config: ForwardProxyConfig) {
        let proxy =
            TcpForwardProxy::new(config, self.name_manager.clone(), self.peer_manager.clone());
        self.tcp_forward_proxy.lock().unwrap().push(proxy);
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

    pub async fn stop(&self) -> GatewayResult<()> {
        let proxy_list = self.socks5_proxy.lock().unwrap().clone();
        for proxy in &proxy_list {
            if let Err(e) = proxy.stop().await {
                return Err(e);
            }
        }

        Ok(())
    }
}

pub type ProxyManagerRef = Arc<ProxyManager>;
