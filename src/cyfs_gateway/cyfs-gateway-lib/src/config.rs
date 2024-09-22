
use serde::Deserialize;
use serde_json::Value;
use url::Url;

use std::collections::HashMap;

use tokio::sync::RwLock;
use std::sync::Arc;



pub struct WarpServerConfig {

}

pub struct DNSServerConfig {

}

pub struct SocksProxyConfig {

}

pub enum ServerConfig {
    WarpServerConfig(WarpServerConfig),
    DNSServerConfig(DNSServerConfig),
    SocksProxyConfig(SocksProxyConfig),
}

#[derive(Clone,Debug)]
pub enum DispatcherTarget {
    Forward(Url),
    Server(String),
}

#[derive(Clone,Debug)]
pub struct DispatcherConfig {
    pub incoming: Url,
    pub target: DispatcherTarget,
    pub enable_tunnels:Option<Vec<String>>,
}


impl DispatcherConfig {
    pub fn new_forward(incoming: Url, target: Url, enable_tunnels:Option<Vec<String>>) -> Self {
        DispatcherConfig {
            incoming,
            target : DispatcherTarget::Forward(target),
            enable_tunnels,
        }
    }

    pub fn new_server(incoming: Url, server_id: String, enable_tunnels:Option<Vec<String>>) -> Self {
        DispatcherConfig {
            incoming,
            target : DispatcherTarget::Server(server_id),
            enable_tunnels,
        }
    }
}


