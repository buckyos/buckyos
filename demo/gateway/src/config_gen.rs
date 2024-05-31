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
    pub addr: Option<String>,
    pub port: Option<u16>,
    pub addr_type: Option<PeerAddrType>,

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

    pub fn add_device(&mut self, id: impl Into<String>, addr: Option<String>, port: Option<u16>, addr_type: Option<PeerAddrType>) {
        let addr = addr.map(|a| {
            assert!(IpAddr::from_str(&a).is_ok());
            a
        });
    

        let id = id.into();
        assert!(self.known_device.iter().find(|d| d.id == id).is_none());

        self.known_device.push(KnownDevice {
            id,
            addr,
            port,
            addr_type,
        })
    }

    pub fn add_socks5_proxy(&mut self, addr: impl Into<String>, port: u16) {
        let addr = addr.into();
        assert!(IpAddr::from_str(&addr).is_ok());
        assert!(port > 0);

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
        
        let mut config = serde_json::Map::new();
        config.insert("device_id".to_owned(), serde_json::Value::String(self.config.device_id.clone()));
        config.insert("addr_type".to_owned(), serde_json::Value::String(self.config.addr_type.as_str().to_owned()));
        config.insert("tunnel_server_port".to_owned(), serde_json::Value::Number(serde_json::Number::from(self.config.tunnel_server_port)));

        let mut known_device = Vec::new();
        for d in &self.known_device {
            let mut item: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
            item.insert("id".to_owned(), serde_json::Value::String(d.id.clone()));
            if let Some(addr) = &d.addr {
                item.insert("addr".to_owned(), serde_json::Value::String(addr.clone()));
            }
            if let Some(port) = d.port {
                item.insert("port".to_owned(), serde_json::Value::Number(serde_json::Number::from(port)));
            }
            if let Some(addr_type) = &d.addr_type {
                item.insert("addr_type".to_owned(), serde_json::Value::String(addr_type.as_str().to_owned()));
            }

            known_device.push(serde_json::Value::Object(item));
        }

        let mut service = Vec::new();
        for s in &self.upstream_service {
            let mut item = serde_json::Map::new();
            item.insert("block".to_owned(), serde_json::Value::String("upstream".to_owned()));
            item.insert("protocol".to_owned(), serde_json::Value::String(s.protocol.clone()));
            item.insert("addr".to_owned(), serde_json::Value::String(s.addr.clone()));
            item.insert("port".to_owned(), serde_json::Value::Number(serde_json::Number::from(s.port)));

            service.push(serde_json::Value::Object(item));
        }

        for s in &self.socks5_proxy {
            let mut item = serde_json::Map::new();
            item.insert("block".to_owned(), serde_json::Value::String("proxy".to_owned()));
            item.insert("addr".to_owned(), serde_json::Value::String(s.addr.clone()));
            item.insert("port".to_owned(), serde_json::Value::Number(serde_json::Number::from(s.port)));
            item.insert("type".to_owned(), serde_json::Value::String("socks5".to_owned()));

            service.push(serde_json::Value::Object(item));
        }

        for s in &self.forward_proxy {
            let mut item = serde_json::Map::new();
            item.insert("block".to_owned(), serde_json::Value::String("proxy".to_owned()));
            item.insert("type".to_owned(), serde_json::Value::String("forward".to_owned()));
            item.insert("protocol".to_owned(), serde_json::Value::String(s.protocol.clone()));
            item.insert("addr".to_owned(), serde_json::Value::String(s.addr.clone()));
            item.insert("port".to_owned(), serde_json::Value::Number(serde_json::Number::from(s.port)));
            item.insert("target_device".to_owned(), serde_json::Value::String(s.target_device.clone()));
            item.insert("target_port".to_owned(), serde_json::Value::Number(serde_json::Number::from(s.target_port)));

            service.push(serde_json::Value::Object(item));
        }

        let config = serde_json::json!({
            "config": config,
            "known_device": known_device,
            "service": service,
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
        config_gen.add_device("etcd1", Some("192.169.100.110".to_owned()), Some(0), Some(PeerAddrType::WAN));
        config_gen.add_device("etcd2", None, Some(1000), None);
        config_gen.add_device("etcd3", None, None, Some(PeerAddrType::LAN));

        config_gen.add_socks5_proxy("127.0.0.1", 1080);
        config_gen.add_forward_proxy("tcp", "127.0.0.1", 1088, "etcd1", 1008);
        config_gen.add_upstream_service("tcp", "127.0.0.1", 1009);

        let config = config_gen.gen();
        info!("{}", config);
    }
}