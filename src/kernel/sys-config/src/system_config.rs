// use etcd_client::*;

use log::*;
use serde_json::{Value, json};
use ::kRPC::kRPC;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SystemConfigError {
    #[error("Failed due to reason: {0}")]
    ReasonError(String),
    #[error("key {0} not found")]
    KeyNotFound(String),
}

pub type Result<T> = std::result::Result<T, SystemConfigError>;
pub struct SystemConfigClient {
    client: kRPC,
}

impl SystemConfigClient {
    pub fn new(server_url:&str) -> Self {
       unimplemented!()
    }
    
    pub async fn get(&mut self, key: &str) -> Result<(String,u64)> {
        let result = self.client.call("sys_config_get", json!({"key": key}))
            .await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))?;

        unimplemented!()
    }

    pub async fn set(&mut self, key: &str, value: &str) -> Result<u64> {
        let result = self.client.call("sys_config_set", json!({"key": key, "value": value}))
            .await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))?;

        unimplemented!()
    }
}
