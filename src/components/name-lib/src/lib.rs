#![allow(dead_code)]
#![allow(unused)]

mod utility;
mod did;
mod config;
mod device_info;

pub use did::*;
pub use config::*;
pub use utility::*;
pub use device_info::*;

use std::net::IpAddr;
use once_cell::sync::Lazy;
use tokio::sync::Mutex;
use once_cell::sync::OnceCell;
use std::env;
use log::*;

pub static CURRENT_DEVICE_CONFIG: OnceCell<DeviceConfig> = OnceCell::new();

pub fn try_load_current_device_config_from_env() -> NSResult<()> {
    let device_doc = env::var("BUCKYOS_THIS_DEVICE");
    if device_doc.is_err() {
        return Err(NSError::NotFound("BUCKY_DEVICE_DOC not set".to_string()));
    }
    let device_doc = device_doc.unwrap();
    
    let device_config= serde_json::from_str(device_doc.as_str());
    if device_config.is_err() {
        warn!("parse device_doc format error");
        return Err(NSError::Failed("device_doc format error".to_string()));
    }
    let device_config:DeviceConfig = device_config.unwrap();
    let set_result = CURRENT_DEVICE_CONFIG.set(device_config);
    if set_result.is_err() {
        warn!("Failed to set CURRENT_DEVICE_CONFIG");
        return Err(NSError::Failed("Failed to set CURRENT_DEVICE_CONFIG".to_string()));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_utility() {
        assert_eq!(is_did("did:example:123456789abcdefghi"), true);
        assert_eq!(is_did("www.buckyos.org"), false);
    }

    #[tokio::test]
    async fn test_get_device_info() {
        let mut device_info = DeviceInfo::new("ood1",DID::new("bns","ood1"));
        device_info.auto_fill_by_system_info().await.unwrap();
        println!("device_info: {:?}",device_info);
    }

}
