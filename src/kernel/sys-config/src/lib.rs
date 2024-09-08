#![allow(dead_code)]
#![allow(unused)]
mod system_config;
mod etcd_control;
pub use system_config::*;

use std::sync::{Arc, Mutex};
use lazy_static::lazy_static;
use buckyos_kit::*;

//TODO:改成每个线程一个client?
lazy_static!{
    static ref SYS_CONFIG: Arc<Mutex<SystemConfigClient>> = {
        print!("init SystemConfigClient");

        Arc::new(Mutex::new(SystemConfigClient::new(&vec!["ood01".to_string()],&None)))
    };
}

pub fn sys_config_get_device_path(device_id: &str) -> String {
    format!("/device/{}", device_id)
}

pub async fn sys_config_get(key: &str) -> Result<(String, u64)> {
    let mut client = SYS_CONFIG.lock().unwrap(); 
    client.get(key).await
}

pub async fn sys_config_set(key: &str, value: &str) -> Result<u64> {
    let mut client = SYS_CONFIG.lock().unwrap();
    client.set(key, value).await
}


#[cfg(test)]
mod tests {
    #[test]
    fn test_utility() {
        ()
    }
}
