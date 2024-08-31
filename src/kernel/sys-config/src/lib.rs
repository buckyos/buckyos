#![allow(dead_code)]
#![allow(unused)]
mod system_config;
mod etcd_control;

use std::sync::{Arc, Mutex};

use lazy_static::lazy_static;
pub use system_config::*;

//TODO:改成每个线程一个client?
lazy_static!{
    static ref SYS_CONFIG: Arc<Mutex<SystemConfigClient>> = {
        print!("init SystemConfigClient");

        Arc::new(Mutex::new(SystemConfigClient::new(&vec!["ood".to_string()],&None)))
    };
}

pub async fn sys_config_get(key: &str) -> Result<(serde_json::Value, u64)> {
    let mut client = SYS_CONFIG.lock().unwrap(); 
    client.get(key).await
}

pub async fn sys_config_set(key: &str, value: &str) -> Result<u64> {
    let mut client = SYS_CONFIG.lock().unwrap();
    client.set(key, value).await
}


pub async fn sys_config_check_running() -> Result<()>{
    //check is system_service running at current device
    unimplemented!();
}

//return pid
pub async fn sys_config_start_service() -> Result<u64>{
    //check is etcd running at current device
    //todo:add recover logic
    unimplemented!();
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_utility() {
        ()
    }
}
