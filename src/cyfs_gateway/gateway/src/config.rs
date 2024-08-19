use crate::peer::NameManagerRef;
use crate::proxy::ProxyManagerRef;
use crate::service::UpstreamManagerRef;
use gateway_lib::*;

use std::str::FromStr;
use std::sync::Arc;

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
        let mut config = GlobalConfig::default();

        let value = json
            .get("config")
            .ok_or(GatewayError::InvalidConfig("config".to_owned()))?;

        if !value.is_object() {
            return Err(GatewayError::InvalidConfig(
                "Invalid config node type".to_owned(),
            ));
        }

        let device_id = value["device_id"]
            .as_str()
            .ok_or(GatewayError::InvalidConfig("device_id".to_owned()))?;

        let addr_type = if let Some(v) = value.get("addr_type") {
            let addr_type = v
                .as_str()
                .ok_or(GatewayError::InvalidConfig("addr_type".to_owned()))?;
            PeerAddrType::from_str(addr_type)?
        } else {
            config.addr_type
        };

        let port = if let Some(v) = value.get("tunnel_server_port") {
            let port = v
                .as_u64()
                .ok_or(GatewayError::InvalidConfig("tunnel_server_port".to_owned()))?;
            if port > u16::MAX as u64 {
                return Err(GatewayError::InvalidConfig("tunnel_server_port".to_owned()));
            }

            port as u16
        } else {
            TUNNEL_SERVER_DEFAULT_PORT
        };

        config.device_id = device_id.to_owned();
        config.addr_type = addr_type;
        config.tunnel_server_port = port;

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
