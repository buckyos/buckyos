// use etcd_client::*;

use log::*;
use rand::random;
use serde::{Serialize,Deserialize};
use serde_json::{Value, json};
use ::kRPC::kRPC;
use thiserror::Error;

use std::sync::Arc;
use tokio::sync::OnceCell;


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
}

impl SystemConfigClient {
    pub fn new(service_url:Option<&str>,session_token:Option<&str>) -> Self {
        let real_session_token : Option<String>;
        if session_token.is_some() {
            real_session_token = Some(session_token.unwrap().to_string());
        } else {
            real_session_token = None;
        }

        let client = kRPC::new(service_url.unwrap_or("http://127.0.0.1:3200/kapi/system_config"), real_session_token.clone());
        let client = Arc::new(client);

        SystemConfigClient {
            client:OnceCell::new_with(Some(client)),
            session_token: real_session_token,
        }
    }

    async fn get_krpc_client(&self) -> Result<Arc<kRPC>> {
        let client = self.client.get();
        if client.is_none() {
            return Err(SystemConfigError::ReasonError("krpc client not found!".to_string()));
        }
        Ok(client.unwrap().clone())
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
