#![allow(dead_code)]
#![allow(unused)]
mod system_config;
mod etcd_control;
pub use system_config::*;

use std::sync::{Arc, Mutex};
use lazy_static::lazy_static;
use sysinfo::System;
use buckyos_kit::*;

//TODO:改成每个线程一个client?
lazy_static!{
    static ref SYS_CONFIG: Arc<Mutex<SystemConfigClient>> = {
        print!("init SystemConfigClient");

        Arc::new(Mutex::new(SystemConfigClient::new(&vec!["ood".to_string()],&None)))
    };
}

pub fn sys_config_get_device_path(device_id: &str) -> String {
    format!("/device/{}", device_id)
}

pub async fn sys_config_get(key: &str) -> Result<(serde_json::Value, u64)> {
    let mut client = SYS_CONFIG.lock().unwrap(); 
    client.get(key).await
}

pub async fn sys_config_set(key: &str, value: &str) -> Result<u64> {
    let mut client = SYS_CONFIG.lock().unwrap();
    client.set(key, value).await
}


pub async fn sys_config_check_running() -> Result<bool>{
    let mut system = System::new_all();
    system.refresh_all();
    let sys_config_service_path = get_buckyos_system_config_service_path().clone();
    for (_pid, process) in system.processes() {
        if let Some(exe_path) = process.exe() {
            if exe_path == sys_config_service_path  {
                return Ok(true);
            }
        }
    }
    
    return Ok(false);
}

pub async fn sys_config_start_service(nonce:Option<&str>) -> Result<()>{
    //todo:add auto recover logic
    let sys_config_service_path = get_buckyos_system_config_service_path().clone();
    let args:Vec<String>;
    if nonce.is_some() {
        args = vec!["--nonce".to_string(), nonce.unwrap().to_string()];
    } else {
        args = vec![];
    }

    buckyos_kit::run_script_with_args(sys_config_service_path.to_str().unwrap(), 5, &Some(args)).await.map_err(
        |error| SystemConfigError::ReasonError(error.to_string())
    )?;
    
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_utility() {
        ()
    }
}
