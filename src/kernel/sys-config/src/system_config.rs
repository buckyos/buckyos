// use etcd_client::*;

use log::*;
use rand::random;
use serde::{Serialize,Deserialize};
use serde_json::{Value, json};
use ::kRPC::kRPC;
use thiserror::Error;

use std::sync::Arc;
use tokio::sync::OnceCell;
use name_lib::*;

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
    client: OnceCell<Arc<kRPC>>,
    session_token: Option<String>,
    this_device: Option<DeviceInfo>,
}

impl SystemConfigClient {
    pub fn new(this_device:Option<&DeviceInfo>,session_token:&Option<String>) -> Self {
        //zone_config is none,this is sys_client@ood
        if this_device.is_none() {
            let client = kRPC::new("http://127.0.0.1:3200/kapi/system_config", session_token);
            let client = Arc::new(client);
            SystemConfigClient {
                client:OnceCell::new_with(Some(client)),
                session_token: session_token.clone(),
                this_device: None,
            }
        } else {
            //http://$device_name:3080/systemconfig
            SystemConfigClient {
                client:OnceCell::new(),
                session_token: session_token.clone(),
                this_device: Some(this_device.unwrap().clone()),
            }
        }
    }

    async fn get_krpc_client(&self) -> Result<Arc<kRPC>> {
        if let Some(client) = self.client.get() {
            return Ok(client.clone());
        }

        let zone_config = CURRENT_ZONE_CONFIG.get();
        if zone_config.is_none() {
            return Err(SystemConfigError::ReasonError("zone config is none!".to_string()));
        }
        let zone_config = zone_config.unwrap();
        let this_device : &DeviceInfo = self.this_device.as_ref().unwrap();
        let ood_info_str = zone_config.select_same_subnet_ood(this_device);
        if ood_info_str.is_some() {

            let ood_info = DeviceInfo::new(ood_info_str.unwrap().as_str());
            info!("try connect to same subnet ood: {}",ood_info.hostname);
            let ood_ip = ood_info.resolve_ip().await;
            if ood_ip.is_ok() {
                let ood_ip = ood_ip.unwrap();
                let server_url = format!("http://{}:3200/kapi/system_config",ood_ip);
                let client = kRPC::new(server_url.as_str(), &self.session_token);
                let client = Arc::new(client);
                self.client.set(client.clone()).ok();
                return Ok(client);
            }
        } 

        let ood_info_str = zone_config.select_wan_ood();
        if ood_info_str.is_some() {
            //try connect to wan ood

            let ood_info = DeviceInfo::new(ood_info_str.unwrap().as_str());
            info!("try connect to wan ood: {}",ood_info.hostname);
            let ood_ip = ood_info.resolve_ip().await;
            if ood_ip.is_ok() {
                let ood_ip = ood_ip.unwrap();
                let server_url = format!("http://{}:3200/kapi/system_config",ood_ip);
                let client = kRPC::new(server_url.as_str(), &self.session_token);
                let client = Arc::new(client);
                self.client.set(client.clone()).ok();
                return Ok(client);
            }
        }

        //connect to local cyfs_gateway
        warn!("cann't connect to ood directly, try connect to local cyfs_gateway");
        //TODO 是否需要3200端口？
        let client = kRPC::new("http://127.0.0.1:3180/kapi/system_config", &self.session_token);
        let client = Arc::new(client);
        self.client.set(client.clone());

        return Ok(client);
    }

    pub async fn get(&self, key: &str) -> Result<(String,u64)> {
        let client = self.get_krpc_client().await;
        if client.is_err() {
            return Err(SystemConfigError::ReasonError(format!("get krpc client failed! {}",client.err().unwrap())));
        }
        let client = client.unwrap();
       
        let result = client.call("sys_config_get", json!({"key": key}))
            .await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))?;

        if result.is_null() {
            return Err(SystemConfigError::KeyNotFound(key.to_string()));
        }
        let value = result.as_str().unwrap_or("");
        let revision = 0;
        Ok((value.to_string(),revision))
    }

    pub async fn set(&self, key: &str, value: &str) -> Result<u64> {
        let client = self.get_krpc_client().await;
        if client.is_err() {
            return Err(SystemConfigError::ReasonError(format!("get krpc client failed! {}",client.err().unwrap())));
        }
        let client = client.unwrap();

        let result = client.call("sys_config_set", json!({"key": key, "value": value}))
            .await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))?;

        Ok(0)
    }

    pub async fn create(&self,key:&str,value:&str) -> Result<u64> {
        let client = self.get_krpc_client().await;
        if client.is_err() {
            return Err(SystemConfigError::ReasonError(format!("get krpc client failed! {}",client.err().unwrap())));
        }
        let client = client.unwrap();

        let result = client.call("sys_config_create", json!({"key": key, "value": value}))
            .await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))?;

        Ok(0)
    }

    pub async fn list(&self,key:&str) -> Result<Vec<String>> {
        unimplemented!()
    }
    
    
}
