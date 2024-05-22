use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use crate::error::*;
use crate::constants::TUNNEL_SERVER_DEFAULT_PORT;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerAddrType {
    WAN,
    LAN,
}

impl PeerAddrType {
    pub fn as_str(&self) -> &'static str {
        match self {
            PeerAddrType::WAN => "wan",
            PeerAddrType::LAN => "lan",
        }
    }
}

impl FromStr for PeerAddrType {
    type Err = GatewayError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "wan" => Ok(PeerAddrType::WAN),
            "lan" => Ok(PeerAddrType::LAN),
            _ => Err(GatewayError::InvalidParam("type".to_owned())),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NameInfo {
    pub device_id: String,
    pub addr: SocketAddr,
    pub addr_type: PeerAddrType,
}

#[derive(Debug, Clone)]
pub struct NameManager {
    resolver: NameResolver,
    names: Arc<Mutex<HashMap<String, NameInfo>>>,
}

impl NameManager {
    pub fn new() -> Self {
        Self {
            resolver: NameResolver::new(),
            names: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /*
    load names from json config as follows:
    [
        {
            id: "device_id",
            addr: "ip",
            port: 1234,
            type: "wan/lan"
        }
    ]
     */
    pub fn load(&self, value: &serde_json::Value) -> GatewayResult<()> {
        if !value.is_array() {
            return Err(GatewayError::InvalidConfig("names".to_owned()));
        }

        let mut names = self.names.lock().unwrap();
        for item in value.as_array().unwrap() {
            let id = item["id"]
                .as_str()
                .ok_or(GatewayError::InvalidConfig("id".to_owned()))?;
            let ip = item["addr"]
                .as_str()
                .ok_or(GatewayError::InvalidConfig("addr".to_owned()))?;

            let port = item["port"]
                .as_u64()
                .unwrap_or(TUNNEL_SERVER_DEFAULT_PORT as u64) as u16;

            // parse addr_type as optional
            let addr_type = item["type"]
                .as_str()
                .map(|s| PeerAddrType::from_str(s))
                .transpose()
                .map_err(|e| {
                    let msg = format!("Error parsing addr type: {}", e);
                    GatewayError::InvalidConfig(msg)
                })?;


            // parse addr
            let addr = format!("{}:{}", ip, port);
            let addr = addr.parse().map_err(|e| {
                let msg = format!("Error parsing addr {}, {}", addr, e);
                GatewayError::InvalidConfig(msg)
            })?;

            let addr_type = 
            match addr_type {
                Some(addr_type) => addr_type,
                None => Self::get_addr_type(&addr),
            };
          
            let info = NameInfo {
                device_id: id.to_string(),
                addr,
                addr_type,
            };

            info!("Load known name: {:?}", info);
            names.insert(id.to_string(), info);
        }

        Ok(())
    }

    pub async fn resolve(&self, device_id: &str) -> Option<NameInfo> {
        {
            let names = self.names.lock().unwrap();
            let ret = names.get(device_id).cloned();
            if ret.is_some() {
                return ret;
            }
        }

        match self.resolver.resolve(device_id).await {
            Ok(addr) => {
                if let Some(addr) = addr {
                    let addr_type = Self::get_addr_type(&addr);
                    let mut names = self.names.lock().unwrap();
                    let info = NameInfo {
                        device_id: device_id.to_string(),
                        addr,
                        addr_type,
                    };
                    names.insert(device_id.to_string(), info.clone());
                    Some(info)
                } else {
                    None
                }
            }
            Err(_) => None,
        }
    }

    pub async fn register(&self, device_id: String, addr: SocketAddr) {
        let addr_type = Self::get_addr_type(&addr);
        let mut names = self.names.lock().unwrap();
        names.insert(
            device_id.clone(),
            NameInfo {
                device_id,
                addr,
                addr_type,
            },
        );
    }

    fn get_addr_type(addr: &SocketAddr) -> PeerAddrType {
        match addr {
            SocketAddr::V4(addr) => {
                if addr.ip().is_private() {
                    PeerAddrType::LAN
                } else {
                    PeerAddrType::WAN
                }
            }
            SocketAddr::V6(_addr) => {
                PeerAddrType::WAN
            }
        }
    }

    pub fn get_device_id(&self, addr_ip: &IpAddr) -> Option<String> {
        let names = self.names.lock().unwrap();
        for (device_id, info) in names.iter() {
            if &info.addr.ip() == addr_ip {
                return Some(device_id.clone());
            }
        }

        None
    }
}

#[derive(Debug, Clone)]
pub struct NameResolver {}

impl NameResolver {
    pub fn new() -> Self {
        Self {}
    }

    pub async fn resolve(&self, device_id: &str) -> GatewayResult<Option<SocketAddr>> {
        // resolve name use DNS protocol
        let result = tokio::net::lookup_host(device_id).await.map_err(|e| {
            error!("Error resolving device id {}: {}", device_id, e);
            e
        })?;

        for addr in result {
            info!("Resolved device id {} to addr {}", device_id, addr);

            return Ok(Some(addr));
        }

        warn!("Device id {} not found", device_id);
        Ok(None)
    }
}

pub type NameManagerRef = Arc<NameManager>;
