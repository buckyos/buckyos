use gateway_lib::*;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::{Arc, Mutex};


#[derive(Debug, Clone)]
pub struct NameInfo {
    pub device_id: String,
    pub ip: Option<IpAddr>,
    pub port: Option<u16>,
    pub addr_type: Option<PeerAddrType>,
}


impl NameInfo {
    pub fn new_empty(device_id: String) -> Self {
        Self {
            device_id,
            ip: None,
            port: None,
            addr_type: None,
        }
    }

    pub fn addr(&self) -> Option<SocketAddr> {
        match self.ip {
            Some(ip) => {
                assert!(self.port.is_some());
                Some(SocketAddr::new(ip, self.port.unwrap()))
            },
            None => None,
        }
    }

    pub fn set_ip(&mut self, ip: IpAddr) {
        self.ip = Some(ip);
        if self.port.is_none() || self.port.unwrap() == 0 {
            self.port = Some(TUNNEL_SERVER_DEFAULT_PORT);
        }

        if self.addr_type.is_none() {
            self.addr_type = Some(NameManager::get_addr_type(&self.addr().unwrap()));
        }
    }
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
            addr_type: "wan/lan"
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

            // If addr is not provided, use resolve(device_id) va dns to get it
            let ip = match item["addr"]
                .as_str() {
                Some(ip) => {
                    let ip = IpAddr::from_str(ip).map_err(|e| {
                        let msg = format!("Error parsing device ip: {}, {}", ip, e);
                        error!("{}", msg);
                        GatewayError::InvalidConfig(msg)
                    })?;
                    Some(ip)
                }
                None => None,
            };

            let port = item["port"]
                .as_u64()
                .unwrap_or(TUNNEL_SERVER_DEFAULT_PORT as u64) as u16;

            // Parse addr_type as optional
            let addr_type = item["addr_type"]
                .as_str()
                .map(|s| PeerAddrType::from_str(s))
                .transpose()
                .map_err(|e| {
                    let msg = format!("Error parsing addr_type: {}", e);
                    GatewayError::InvalidConfig(msg)
                })?;

            
            let info = NameInfo {
                device_id: id.to_string(),
                ip,
                port: Some(port),
                addr_type,
            };

            info!("Load known name: {:?}", info);
            names.insert(id.to_string(), info);
        }

        Ok(())
    }

    pub async fn resolve(&self, device_id: &str) -> Option<NameInfo> {
        {
            let mut names = self.names.lock().unwrap();
            let ret = names.get(device_id).cloned();
            if ret.is_some() {
                let item = ret.unwrap();
                if item.ip.is_some() {
                    return Some(item);
                }
            } else {
                info!("Name {} not found, now will init as empty", device_id);
                names.insert(device_id.to_owned(), NameInfo::new_empty(device_id.to_owned()));
            }
        }

        match self.resolver.resolve(device_id).await {
            Ok(addr) => {
                if let Some(addr) = addr {
                    let mut names = self.names.lock().unwrap();
                    let item = names.get_mut(device_id).unwrap();
                    item.set_ip(addr.ip());
                    Some(item.to_owned())
                } else {
                    None
                }
            }
            Err(_) => None,
        }
    }

    pub fn add(&self, device_id: String, addr: SocketAddr, addr_type: Option<PeerAddrType>) {
        let addr_type  = match addr_type {
            Some(addr_type) => addr_type,
            None => Self::get_addr_type(&addr),
        };
        let port = match addr.port() {
            0 => TUNNEL_SERVER_DEFAULT_PORT,
            port => port,
        };

        let mut names = self.names.lock().unwrap();
        if let Some(prev) = names.insert(
            device_id.clone(),
            NameInfo {
                device_id: device_id.clone(),
                ip: Some(addr.ip()),
                port: Some(port),
                addr_type: Some(addr_type),
            },
        ) {
            warn!("Device id {} already registered, update to {} -> {}", device_id, prev.addr().as_ref().unwrap(), addr);
        } else {
            info!("Register device id {} to addr {}", device_id, addr);
        }
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
            match info.addr() {
                Some(addr) => {
                    if &addr.ip() == addr_ip {
                        return Some(device_id.clone());
                    }
                }
                None => {
                    continue;
                }
            }
        }

        None
    }

    pub fn select_peers_by_type(&self, addr_type: PeerAddrType) -> Vec<NameInfo> {
        let names = self.names.lock().unwrap();
        names
            .values()
            .filter(|info| info.addr_type.is_some() && *info.addr_type.as_ref().unwrap() == addr_type)
            .cloned()
            .collect()
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
