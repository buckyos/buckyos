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
        Arc::new(Mutex::new(SystemConfigClient::new("http://wwww.xxxxx.com")))
    };
}

pub async fn sys_config_get(key: &str) -> Result<(String, u64)> {
    let mut client = SYS_CONFIG.lock().unwrap(); 
    return client.get(key).await;
}

pub async fn sys_config_set(key: &str, value: &str) -> Result<u64> {
    let mut client = SYS_CONFIG.lock().unwrap();
    return client.set(key, value).await;
}


#[cfg(test)]
mod tests {
    #[test]
    fn test_utility() {
        ()
    }
}
