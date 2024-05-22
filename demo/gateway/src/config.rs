use crate::error::{GatewayError, GatewayResult};
use crate::peer::NameManagerRef;
use crate::proxy::ProxyManagerRef;
use crate::service::UpstreamManagerRef;

use std::sync::Arc;

/*
"config": {
    "device-id": "client1",
},
"known_device": [{
    "id": "gateway",
    "addr": "1.2.3.4:8000",
    "addr_type": "wan"
}],
"service":
[{
    "block": "upstream",
    "addr": "127.0.0.1",
    "port": 2000,
    "type": "tcp"
}, {
    "block": "upstream",
    "addr": "127.0.0.1",
    "port": 2001,
    "type": "http",
}]
*/

pub struct GlobalConfig {
    device_id: String,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            device_id: "".to_owned(),
        }
    }
}

impl GlobalConfig {
    pub fn device_id(&self) -> &str {
        &self.device_id
    }
}

pub type GlobalConfigRef = Arc<GlobalConfig>;


pub struct ConfigLoader {
    name_manager: NameManagerRef,
    upstream_manager: UpstreamManagerRef,
    proxy_manager: ProxyManagerRef,
}

impl ConfigLoader {
    pub fn new(
        name_manager: NameManagerRef,
        upstream_manager: UpstreamManagerRef,
        proxy_manager: ProxyManagerRef,
    ) -> Self {
        Self {
            name_manager,
            upstream_manager,
            proxy_manager,
        }
    }

    pub fn load_config_node(json: &serde_json::Value) -> GatewayResult<GlobalConfigRef> {

        let value = json
            .get("config")
            .ok_or(GatewayError::InvalidConfig("config".to_owned()))?;

        if !value.is_object() {
            return Err(GatewayError::InvalidConfig("Invalid config node type".to_owned()));
        }

        let device_id = value["device-id"]
            .as_str()
            .ok_or(GatewayError::InvalidConfig("device-id".to_owned()))?;

        let mut config = GlobalConfig::default();
        config.device_id = device_id.to_owned();

    
        Ok(Arc::new(config))
    }

    fn load_known_device(&self, value: &serde_json::Value) -> GatewayResult<()> {
        self.name_manager.load(value)
    }

    fn load_service_list(&self, value: &serde_json::Value) -> GatewayResult<()> {
        if !value.is_array() {
            return Err(GatewayError::InvalidConfig("service".to_owned()));
        }

        for item in value.as_array().unwrap() {
            let block = item["block"]
                .as_str()
                .ok_or(GatewayError::InvalidConfig("block".to_owned()))?;

            match block {
                "upstream" => {
                    self.upstream_manager.load_block(item)?;
                }
                "proxy" => {
                    self.proxy_manager.load_proxy(item)?;
                }
                _ => {
                    warn!("Unknown block: {}", block);
                }
            }
        }

        Ok(())
    }

    pub fn load_str(&self, json_str: &str) -> GatewayResult<()> {
        let json = serde_json::from_str(json_str)
            .map_err(|_| GatewayError::InvalidConfig(json_str.to_owned()))?;

        self.load(&json)
    }

    pub fn load(&self, json: &serde_json::Value) -> GatewayResult<()> {
        

        // load known device if exists
        let value = json.get("known_device");
        if value.is_some() {
            self.load_known_device(value.unwrap())?;
        }

        // load service list if exists
        let value = json.get("service");
        if value.is_some() {
            self.load_service_list(value.unwrap())?;
        }

        Ok(())
    }
}
