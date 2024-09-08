// use etcd_client::*;

use log::*;
use rand::random;
use serde::{Serialize,Deserialize};
use serde_json::{Value, json};
use ::kRPC::kRPC;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SystemConfigError {
    #[error("Failed due to reason: {0}")]
    ReasonError(String),
    #[error("key {0} not found")]
    KeyNotFound(String),
    #[error("NoPermission: {0}")]
    NoPermission(String),
    #[error("Timeout: {0}")]
    Timeout(String),
}

pub type Result<T> = std::result::Result<T, SystemConfigError>;
pub struct SystemConfigClient {
    client: kRPC,
}


impl SystemConfigClient {
    pub fn new(ood_list:&Vec<String>,session_token:&Option<String>) -> Self {
        if ood_list.len() == 0 {
            let client = kRPC::new("http://127.0.0.1:10030/system_config", session_token);
            SystemConfigClient {
                client,
            }
        } else {
            let index = random::<usize>() % ood_list.len();
            let device_name = ood_list[index].clone();
            let server_url = format!("http://{}:10030/system_config",device_name);
            //http://$device_name:3080/systemconfig

            let client = kRPC::new(server_url.as_str(), session_token);
            SystemConfigClient {
                client,
            }
        }
    }

    pub async fn register_device(&self,device_jwt:&str,boot_info:&Option<serde_json::Value>) -> Result<()> {
        let param:serde_json::Value;
        if boot_info.is_some() {
            param = json!({
                "device_doc": device_jwt,
                "boot_info": boot_info,
            });
        } else {
            param = json!({
                "device_doc": device_jwt,
            });
        }
        
        let result = self.client.call("sys_config_register_device",param)
            .await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))?;

        Ok(())
    }
    
    pub async fn get(&self, key: &str) -> Result<(Value,u64)> {
        let result = self.client.call("sys_config_get", json!({"key": key}))
            .await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))?;

        if result.is_null() {
            return Err(SystemConfigError::KeyNotFound(key.to_string()));
        }

        Ok((result,0))
    }

    pub async fn set(&self, key: &str, value: &str) -> Result<u64> {
        let result = self.client.call("sys_config_set", json!({"key": key, "value": value}))
            .await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))?;

        Ok(0)
    }

    pub async fn create(&self,key:&str,value:&str) -> Result<u64> {
        let result = self.client.call("sys_config_create", json!({"key": key, "value": value}))
            .await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))?;

        Ok(0)
    }
    
    
}
