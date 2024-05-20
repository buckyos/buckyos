use crate::error::{GatewayError, GatewayResult};
use crate::peer::NAME_MANAGER;
use crate::service::UPSTREAM_MANAGER;

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

pub struct ConfigLoader {}


impl ConfigLoader {
    fn load_config_node(value: &serde_json::Value, block: &str) -> GatewayResult<()> {
        if !value.is_object() {
            return Err(GatewayError::InvalidConfig(block.to_owned()));
        }

        let device_id = value["device-id"]
            .as_str()
            .ok_or(GatewayError::InvalidConfig("device-id".to_owned()))?;

        // TODO
        Ok(())
    }

    fn load_known_device(value: &serde_json::Value) -> GatewayResult<()> {
        NAME_MANAGER.load(value)
    }

    fn load_service_list(value: &serde_json::Value) -> GatewayResult<()> {
        if !value.is_array() {
            return Err(GatewayError::InvalidConfig("service".to_owned()));
        }

        for item in value.as_array().unwrap() {
            let block = item["block"]
                .as_str()
                .ok_or(GatewayError::InvalidConfig("block".to_owned()))?;

            match block {
                "upstream" => {
                    UPSTREAM_MANAGER.load_block(item)?;
                }
                "proxy" => {}

                _ => {
                    warn!("Unknown block: {}", block);
                }
            }
        }

        Ok(())
    }

    pub fn load_str(json_str: &str) -> GatewayResult<()> {
        let json = serde_json::from_str(json_str)
            .map_err(|_| GatewayError::InvalidConfig(json_str.to_owned()))?;

        Self::load(&json)
    }

    pub fn load(json: &serde_json::Value) -> GatewayResult<()> {
        let config = json
            .get("config")
            .ok_or(GatewayError::InvalidConfig("config".to_owned()))?;

        Self::load_config_node(config, "config")?;

        // load known device if exists
        let value = json.get("known_device");
        if value.is_some() {
            Self::load_known_device(value.unwrap())?;
        }

        // load service list if exists
        let value = json.get("service");
        if value.is_some() {
            Self::load_service_list(value.unwrap())?;
        }

        Ok(())
    }
}
