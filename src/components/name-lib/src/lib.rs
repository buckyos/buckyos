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
