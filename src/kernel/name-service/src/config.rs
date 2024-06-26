#![allow(non_camel_case_types)]
use serde::{Deserialize, Serialize};
use serde::de::DeserializeOwned;
use crate::error::into_ns_err;
use crate::{NSErrorCode, NSResult};

#[derive(Serialize, Deserialize, Debug)]
pub enum ProviderType {
    #[serde(rename = "dns")]
    DNS,
    #[serde(rename = "etcd")]
    ETCD,
}

#[derive(Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(rename = "type")]
    pub ty: ProviderType,
    #[serde(flatten)]
    config: serde_json::Value,
}

impl ProviderConfig {
    pub fn get<T: DeserializeOwned>(&self) -> NSResult<T> {
        serde_json::from_value(self.config.clone()).map_err(into_ns_err!(NSErrorCode::InvalidData, "Failed to deserialize {:?} provider config", self.ty))
    }

    pub fn set<T: Serialize>(&mut self, config: T) -> NSResult<()> {
        self.config = serde_json::to_value(config).map_err(into_ns_err!(NSErrorCode::InvalidData, "Failed to serialize {:?} provider config", self.ty))?;
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
pub struct ETCDConfig {
    pub(crate) etcd_url: String,
}

#[derive(Serialize, Deserialize)]
pub struct DNSConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) dns_server: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct NSConfig {
    pub(crate) node_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) zone_ca: Option<String>,
    pub(crate) node_cert: String,
    pub(crate) node_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) local_info_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) default_info_path: Option<String>,
    pub(crate) provide_list: Vec<ProviderConfig>,
}

#[cfg(test)]
mod test_config {
    use crate::config::{ProviderConfig, ProviderType};
    use crate::NSConfig;

    #[test]
    fn test_provider_config() {
        let config = NSConfig {
            node_name: "test".to_string(),
            zone_ca: None,
            node_cert: "cert".to_string(),
            node_key: "key".to_string(),
            local_info_path: None,
            default_info_path: None,
            provide_list: vec![
                ProviderConfig {
                    ty: ProviderType::DNS,
                    config: serde_json::json!({
                        "dns_server": "8.8.8.8"
                        })
                },
                ProviderConfig {
                    ty: ProviderType::ETCD,
                    config: serde_json::json!({
                        "etcd_url": "http://127.0.0.1:2890"
                        })
                }],
        };

        let config_str = serde_json::to_string(&config).unwrap();
        println!("{}", config_str);

        let toml_str = toml::to_string(&config).unwrap();
        println!("{}", toml_str);
    }
}
