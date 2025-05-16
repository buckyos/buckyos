// use etcd_client::*;

use log::*;
use rand::random;
use serde::{Serialize,Deserialize};
use serde_json::{Value, json, Map};
use ::kRPC::kRPC;
use thiserror::Error;

use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::OnceCell;

use crate::KVAction;
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

pub type SytemConfigResult<T> = std::result::Result<T, SystemConfigError>;
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
        //let default_sys_config_url = 
        let client = kRPC::new(service_url.unwrap_or("http://127.0.0.1:3200/kapi/system_config"), real_session_token.clone());
        let client = Arc::new(client);
        info!("system config client is created,service_url:{},session_token:{}",service_url.unwrap_or("http://127.0.0.1:3200/kapi/system_config"),real_session_token.clone().unwrap_or("None".to_string()));

        SystemConfigClient {
            client:OnceCell::new_with(Some(client)),
            session_token: real_session_token,
        }
    }

    pub fn get_session_token(&self) -> Option<String> {
        self.session_token.clone()
    }

    fn get_krpc_client(&self) -> SytemConfigResult<Arc<kRPC>> {
        let client = self.client.get();
        if client.is_none() {
            return Err(SystemConfigError::ReasonError("krpc client not found!".to_string()));
        }
        Ok(client.unwrap().clone())
    }

    pub async fn get(&self, key: &str) -> SytemConfigResult<(String,u64)> {
        let client = self.get_krpc_client()?;
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

    pub async fn set(&self, key: &str, value: &str) -> SytemConfigResult<u64> {
        if key.is_empty() || value.is_empty() {
            return Err(SystemConfigError::ReasonError("key or value is empty".to_string()));
        }
        //TODO:define a rule for KEY
        if key.contains(":") {
            return Err(SystemConfigError::ReasonError("key can not contain ':'".to_string()));
        }

        let client = self.get_krpc_client()?;
        let result = client.call("sys_config_set", json!({"key": key, "value": value}))
            .await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))?;

        Ok(0)
    }

    pub async fn set_by_json_path(&self,key:&str,json_path:&str,value:&str) -> SytemConfigResult<u64> {
        let client = self.get_krpc_client()?;
        client.call("sys_config_set_by_json_path", json!({"key": key, "json_path": json_path, "value": value})).await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))?;
        Ok(0)
    }

    pub async fn create(&self,key:&str,value:&str) -> SytemConfigResult<u64> {
        let client = self.get_krpc_client()?;
        let result = client.call("sys_config_create", json!({"key": key, "value": value}))
            .await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))?;

        Ok(0)
    }

    pub async fn delete(&self,key:&str) -> SytemConfigResult<u64> {
        let client = self.get_krpc_client()?;
        let result = client.call("sys_config_delete", json!({"key": key}))
            .await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))?;
        Ok(0)
    }

    pub async fn append(&self,key:&str,value:&str) -> SytemConfigResult<u64> {
        let client = self.get_krpc_client()?;
        client.call("sys_config_append", json!({"key": key, "append_value": value})).await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))?;
        Ok(0)
    }

    //list direct children
    pub async fn list(&self,key:&str) -> SytemConfigResult<Vec<String>> {
        let client = self.get_krpc_client()?;
        client.call("sys_config_list", json!({"key": key})).await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))
            .map(|result| {
                let mut list = Vec::new();
                for item in result.as_array().unwrap() {
                    list.push(item.as_str().unwrap().to_string());
                }
                list
            })
    }

    pub async fn exec_tx(&self, tx_actions: HashMap<String, KVAction>, main_key: Option<(String, u64)>) -> SytemConfigResult<u64> {
        if tx_actions.is_empty() {
            return Ok(0);
        }
        let mut tx_json = Map::new();

        for (key, action) in tx_actions.iter() {
            match action {
                KVAction::Create(value) => {
                    tx_json.insert(key.to_string(), json!({
                        "action": "create",
                        "value": value
                    }));
                }
                KVAction::Update(value) => {
                    tx_json.insert(key.to_string(), json!({
                        "action": "update",
                        "value": value
                    }));
                }
                KVAction::Append(value) => {
                    tx_json.insert(key.to_string(), json!({
                        "action": "append",
                        "value": value
                    }));
                }
                KVAction::SetByJsonPath(value) => {
                    tx_json.insert(key.to_string(), json!({
                        "action": "set_by_path",
                        "all_set": value
                    }));
                }
                KVAction::Remove => {
                    tx_json.insert(key.to_string(), json!({
                        "action": "remove"
                    }));
                }
            }
        }

        let mut req_params = Map::new();
        req_params.insert("actions".to_string(), Value::Object(tx_json));

        if let Some((key, revision)) = main_key {
            req_params.insert("main_key".to_string(), Value::String(format!("{}:{}",key,revision)));
        }

        let client = self.get_krpc_client()?;
        client.call("sys_config_exec_tx", Value::Object(req_params)).await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))?;
        Ok(0)
    }

    pub async fn dump_configs_for_scheduler(&self) -> SytemConfigResult<Value> {
        let client = self.get_krpc_client()?;
        let result = client.call("dump_configs_for_scheduler", json!({}))
            .await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))?;
        Ok(result)
    }

  

}
