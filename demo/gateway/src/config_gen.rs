use crate::config::GlobalConfig;
use crate::peer::PeerAddrType;
use crate::constants::TUNNEL_SERVER_DEFAULT_PORT;

use std::net::IpAddr;
use std::str::FromStr;

/*
{
    "config": {
        "device_id": "gateway",
        "addr_type": "wan"
    },
    "known_device": [
        {
            "id": "etcd1",
            "addr": "192.168.100.110",
            "port": 23559,
            "addr_type": "wan"
        }
    ],
    "service": [{
        "block": "upstream",
        "protocol": "tcp",
        "addr": "127.0.0.1",
        "port": 1009
    }, {
        "block": "proxy",
        "addr": "127.0.0.1",
        "port": 1080,
        "type": "socks5"
    }, {
        "block": "proxy",
        "type": "forward",
		"protocol": "tcp",
        "addr": "127.0.0.1",
        "port": 1088,
        "target_device": "etcd1",
        "target_port": 1008
    }]
}
*/
pub struct KnownDevice {
    pub id: String,
    pub addr: String,
    pub port: u16,
    pub addr_type: PeerAddrType,

}

pub struct UpstreamService {
    pub addr: String,
    pub protocol: String,
    pub port: u16,
}

pub struct Socks5Proxy {
    pub addr: String,
    pub port: u16,
}

pub struct ForwardProxy {
    pub protocol: String,
    pub addr: String,
    pub port: u16,
    pub target_device: String,
    pub target_port: u16,
}

pub struct ConfigGen {
    pub config: GlobalConfig,
    pub known_device: Vec<KnownDevice>,
    pub socks5_proxy: Vec<Socks5Proxy>,
    pub forward_proxy: Vec<ForwardProxy>,
    pub upstream_service: Vec<UpstreamService>,
}

impl ConfigGen {
    pub fn new(device_id: impl Into<String>, addr_type: PeerAddrType, mut tunnel_server_port: u16) -> Self {
        if tunnel_server_port == 0 {
            tunnel_server_port = TUNNEL_SERVER_DEFAULT_PORT;
        }

        Self {
            config: GlobalConfig {
                device_id: device_id.into(),
                addr_type,
                tunnel_server_port,
            },
            known_device: Vec::new(),
            socks5_proxy: Vec::new(),
            forward_proxy: Vec::new(),
            upstream_service: Vec::new(),
        }
    }

    pub fn add_device(&mut self, id: impl Into<String>, addr: impl Into<String>, mut port: u16, addr_type: PeerAddrType) {
        let addr = addr.into();
        assert!(IpAddr::from_str(&addr).is_ok());

        if port == 0 {
            port = TUNNEL_SERVER_DEFAULT_PORT;
        }

        let id = id.into();
        assert!(self.known_device.iter().find(|d| d.id == id).is_none());

        self.known_device.push(KnownDevice {
            id,
            addr: addr.into(),
            port,
            addr_type,
        })
    }

    pub fn add_socks5_proxy(&mut self, addr: impl Into<String>, port: u16) {
        let addr = addr.into();
        assert!(IpAddr::from_str(&addr).is_ok());
        assert!(port > 0);
        assert!(self.known_device.iter().find(|d| d.addr == addr && d.port == port).is_none());

        self.socks5_proxy.push(Socks5Proxy {
            addr,
            port,
        })
    }

    pub fn add_forward_proxy(&mut self, protocol: impl Into<String>, addr: impl Into<String>, port: u16, target_device: impl Into<String>, target_port: u16) {
        let addr = addr.into();
        assert!(IpAddr::from_str(&addr).is_ok());
        assert!(port > 0);
        assert!(target_port > 0);
        assert!(self.known_device.iter().find(|d| d.addr == addr && d.port == port).is_none());

        self.forward_proxy.push(ForwardProxy {
            protocol: protocol.into(),
            addr,
            port,
            target_device: target_device.into(),
            target_port,
        })
    }

    pub fn add_upstream_service(&mut self, protocol: impl Into<String>, addr: impl Into<String>, port: u16) {
        let addr = addr.into();
        assert!(IpAddr::from_str(&addr).is_ok());
        assert!(port > 0);

        self.upstream_service.push(UpstreamService {
            addr,
            protocol: protocol.into(),
            port,
        })
    }

    pub fn gen(&self) -> String {
        let config = serde_json::json!({
            "config": {
                "device_id": self.config.device_id,
                "addr_type": self.config.addr_type.as_str(),
                "tunnel_server_port": self.config.tunnel_server_port,
            },
            "known_device": self.known_device.iter().map(|d| {
                serde_json::json!({
                    "id": d.id,
                    "addr": d.addr,
                    "port": d.port,
                    "addr_type": d.addr_type.as_str(),
                })
            }).collect::<Vec<_>>(),
            "service": self.upstream_service.iter().map(|s| {
                serde_json::json!({
                    "block": "upstream",
                    "protocol": s.protocol,
                    "addr": s.addr,
                    "port": s.port,
                })
            }).chain(self.socks5_proxy.iter().map(|s| {
                serde_json::json!({
                    "block": "proxy",
                    "addr": s.addr,
                    "port": s.port,
                    "type": "socks5",
                })
            })).chain(self.forward_proxy.iter().map(|s| {
                serde_json::json!({
                    "block": "proxy",
                    "type": "forward",
                    "protocol": s.protocol,
                    "addr": s.addr,
                    "port": s.port,
                    "target_device": s.target_device,
                    "target_port": s.target_port,
                })
            })).collect::<Vec<_>>(),
        });

        config.to_string()
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    pub fn test_config_gen() {
        std::env::set_var("RUST_LOG", "info");
        env_logger::init();

        let mut config_gen = ConfigGen::new("gateway", PeerAddrType::WAN, 0);
        config_gen.add_device("etcd1", "192.169.100.110", 0, PeerAddrType::WAN);

        config_gen.add_socks5_proxy("127.0.0.1", 1080);
        config_gen.add_forward_proxy("tcp", "127.0.0.1", 1088, "etcd1", 1008);
        config_gen.add_upstream_service("tcp", "127.0.0.1", 1009);

        let config = config_gen.gen();
        info!("{}", config);
    }
}