use crate::constants::TUNNEL_SERVER_DEFAULT_PORT;
use crate::peer::PeerAddrType;

use std::sync::Arc;

/*
"config": {
    "device-id": "client1",
    "addr_type": "wan/lan",
    "tunnel_server_port": 23558
},
"known_device": [{
    "id": "gateway",
    "addr": "1.2.3.4:8000",
    "addr_type": "wan"
}],
"service":
[{
    "block": "upstream",
    "id": "local_service",
    "addr": "127.0.0.1",
    "port": 2000,
    "type": "tcp"
}, {
    "block": "upstream",
    "id": "local_service2",
    "addr": "127.0.0.1",
    "port": 2001,
    "type": "http",
}]
*/

pub struct GlobalConfig {
    pub device_id: String,
    pub addr_type: PeerAddrType,
    pub tunnel_server_port: u16,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            device_id: "".to_owned(),
            addr_type: PeerAddrType::WAN,
            tunnel_server_port: TUNNEL_SERVER_DEFAULT_PORT,
        }
    }
}

impl GlobalConfig {
    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    pub fn addr_type(&self) -> PeerAddrType {
        self.addr_type
    }
    
    pub fn tunnel_server_port(&self) -> u16 {
        self.tunnel_server_port
    }
}

pub type GlobalConfigRef = Arc<GlobalConfig>;
