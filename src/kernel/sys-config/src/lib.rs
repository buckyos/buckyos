#![allow(dead_code)]
#![allow(unused)]
mod system_config;
mod sn_client;
mod app_list;
mod node_list;

pub use system_config::*;
pub use sn_client::*;
pub use app_list::*;
pub use node_list::*;

use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use serde_json::Value;
use lazy_static::lazy_static;
use buckyos_kit::*;


pub enum KVAction {
    Create(String),//创建一个节点并设置值
    Update(String),//完整更新
    SetByJsonPath(HashMap<String,Option<Value>>),//当成json设置其中的一个值,针对一个对象,set可以是一个数组
    Remove,//删除
    //Create(String),
}


//TODO:改成每个线程一个client?
lazy_static!{
    static ref SYS_CONFIG: Arc<Mutex<SystemConfigClient>> = {
        print!("init SystemConfigClient");
        Arc::new(Mutex::new(SystemConfigClient::new(None,None)))
    };
}

pub fn sys_config_get_device_path(device_id: &str) -> String {
    format!("devices/{}", device_id)
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
