use crate::def::*;
use name_lib::{DeviceConfig, ZoneConfig};
use std::env;
pub struct ZoneInfoHelper;

impl ZoneInfoHelper {
    pub fn get_zone_config() -> RepoResult<ZoneConfig> {
        let zone_config_str = env::var("BUCKY_ZONE_CONFIG").map_err(|e| {
            RepoError::NotReadyError(format!("Failed to get BUCKY_ZONE_CONFIG, err:{}", e))
        })?;
        let zone_config: ZoneConfig =
            serde_json::from_str(zone_config_str.as_str()).map_err(|e| {
                RepoError::NotReadyError(format!("Failed to parse zone config, err:{}", e))
            })?;
        Ok(zone_config)
    }

    pub fn get_device_config() -> RepoResult<DeviceConfig> {
        let device_doc = env::var("BUCKY_THIS_DEVICE").map_err(|e| {
            RepoError::NotReadyError(format!("Failed to get BUCKY_THIS_DEVICE, err:{}", e))
        })?;
        let device_config: DeviceConfig =
            serde_json::from_str(device_doc.as_str()).map_err(|e| {
                RepoError::NotReadyError(format!("Failed to parse device config, err:{}", e))
            })?;
        Ok(device_config)
    }

    pub fn get_zone_did() -> RepoResult<String> {
        let zone_config = Self::get_zone_config()?;
        Ok(zone_config.did)
    }

    pub fn get_zone_name() -> RepoResult<String> {
        let zone_config = Self::get_zone_config()?;
        match zone_config.name {
            Some(name) => Ok(name),
            None => Err(RepoError::NotReadyError("Zone name not set".to_string())),
        }
    }
}
