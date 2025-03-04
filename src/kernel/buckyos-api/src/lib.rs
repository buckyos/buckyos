#![allow(dead_code)]
#![allow(unused)]
mod system_config;
mod sn_client;
mod app_list;
mod node_list;
mod gateway;
mod task_mgr;

pub use system_config::*;
pub use sn_client::*;
pub use app_list::*;
pub use node_list::*;
pub use gateway::*;
pub use task_mgr::*;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use serde_json::Value;
use lazy_static::lazy_static;
use buckyos_kit::*;

//本库以后可能改名叫buckyos-sdk, 
// 通过syc_config_client与buckyos的各种服务交互，与传统OS的system_call类似


//TODO:改成每个线程一个client?
lazy_static!{
    static ref buckyos_api: Arc<Mutex<SystemConfigClient>> = {
        print!("init SystemConfigClient");
        Arc::new(Mutex::new(SystemConfigClient::new(None,None)))
    };
}

pub fn buckyos_api_get_device_path(device_id: &str) -> String {
    format!("devices/{}", device_id)
}


pub async fn buckyos_api_get(key: &str) -> SytemConfigResult<(String, u64)> {
    let mut client = buckyos_api.lock().unwrap(); 
    client.get(key).await
}

pub async fn buckyos_api_set(key: &str, value: &str) -> SytemConfigResult<u64> {
    let mut client = buckyos_api.lock().unwrap();
    client.set(key, value).await
}

//if http_only is false, return the url with tunnel protocol
pub fn get_zone_service_url(service_name: &str,http_only: bool) -> String {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_utility() {
        ()
    }
}
