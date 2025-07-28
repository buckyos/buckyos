
//TODO:
//  add WATCH,and load cached value automatically when the value is changed.

use buckyos_kit::buckyos_get_unix_timestamp;
use log::*;
use serde_json::{Value, json, Map};
use ::kRPC::kRPC;
use thiserror::Error;

use std::sync::Arc;
use std::collections::HashMap;
use std::sync::LazyLock;
use tokio::sync::{OnceCell, RwLock};

use crate::KVAction;

const CONFIG_CACHE_TIME:u64 = 10; //10s

//key -> (value,version)
static CONFIG_CACHE: LazyLock<RwLock<HashMap<String,(String,u64)>>> = LazyLock::new(|| RwLock::new(HashMap::new()));

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
    cache_key_control:OnceCell<Vec<String>>,
    current_version:RwLock<u64>,
}

pub struct SystemConfigValue {
    pub value:String,
    pub version:u64,
    pub is_changed:bool,
}

impl SystemConfigValue {
    pub fn new(value:String,version:u64,is_changed:bool) -> Self {
        Self { value, version, is_changed }
    }
}


impl SystemConfigClient {

    fn need_cache(&self,key:&str) -> bool {
        let cache_key_control = self.cache_key_control.get();
        if cache_key_control.is_none() {
            return false;
        }
        for k in cache_key_control.unwrap().iter() {
            if key.starts_with(k) {
                return true;
            }
        }
        false
    }

    async fn get_config_cache(&self,key:&str) -> Option<String> {
        let config_cache = &CONFIG_CACHE;
        let current_version = self.current_version.read().await;
        let real_current_version = *current_version;
        drop(current_version);

        let cache_guard = config_cache.read().await;
        let v = cache_guard.get(key);
        if v.is_none() {
            return None;
        }
        let (value,version) = v.unwrap();
        if version + CONFIG_CACHE_TIME < real_current_version {
            // 缓存过期，删除缓存项
            drop(cache_guard); // 释放读锁
            let mut cache_guard = config_cache.write().await;
            cache_guard.remove(key);
            return None;
        }
        debug!("get system_config from CONFIG_CACHE {}=>{}",&key,&value);
        Some(value.clone())
    }
    
    async fn set_config_cache(&self,key:&str,value:&str,version:u64) -> bool {
        if !self.need_cache(key) {
            return true;
        }

        let mut current_version = self.current_version.write().await;
        if *current_version < version {
            *current_version = version;
        }
        drop(current_version);

        let config_cache = &CONFIG_CACHE;
        let mut cache_guard = config_cache.write().await;
        let old_value = cache_guard.insert(key.to_string(), (value.to_string(), version));
        if old_value.is_none() {
            return true;
        }
        let old_value = old_value.unwrap();
        if old_value.0 == value {
            return false;
        }
        true
    }

    async fn remove_config_cache(&self,key:&str) {
        let config_cache = &CONFIG_CACHE;
        let mut cache_guard = config_cache.write().await;
        cache_guard.remove(key);
    }

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
        let key_control = vec![
            "services/".to_string(),
            "system/rbac/".to_string(),
        ];
        let cache_key_control = OnceCell::new_with(Some(key_control));

        SystemConfigClient {
            client:OnceCell::new_with(Some(client)),
            session_token: real_session_token,
            cache_key_control: cache_key_control,
            current_version: RwLock::new(0),
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

    //return (value,version,is_changed)
    pub async fn get(&self, key: &str) -> SytemConfigResult<SystemConfigValue> {
        // 首先尝试从缓存获取
        if let Some(cached_value) = self.get_config_cache(key).await {
            return Ok(SystemConfigValue::new(cached_value, 0, false));
        }

        // 缓存中没有，从服务器获取
        let client = self.get_krpc_client()?;
        let result = client.call("sys_config_get", json!({"key": key}))
            .await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))?;

        if result.is_null() {
            return Err(SystemConfigError::KeyNotFound(key.to_string()));
        }
        let value = result.as_str().unwrap_or("");
        let revision = buckyos_get_unix_timestamp();
        
        // 将结果存入缓存
        let is_changed = self.set_config_cache(key, &value, revision).await;
        
        Ok(SystemConfigValue::new(value.to_string(),revision,is_changed))
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
        let _result = client.call("sys_config_set", json!({"key": key, "value": value}))
            .await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))?;

        let revision = buckyos_get_unix_timestamp();
        let _is_changed = self.set_config_cache(key, value, revision).await;
        
        Ok(0)
    }

    pub async fn set_by_json_path(&self,key:&str,json_path:&str,value:&str) -> SytemConfigResult<u64> {
        let client = self.get_krpc_client()?;
        client.call("sys_config_set_by_json_path", json!({"key": key, "json_path": json_path, "value": value})).await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))?;

        let revision = buckyos_get_unix_timestamp();
        let _is_changed = self.set_config_cache(key, value, revision).await;

        Ok(0)
    }

    pub async fn create(&self,key:&str,value:&str) -> SytemConfigResult<u64> {
        let client = self.get_krpc_client()?;
        let _result = client.call("sys_config_create", json!({"key": key, "value": value}))
            .await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))?;

        Ok(0)
    }

    pub async fn delete(&self,key:&str) -> SytemConfigResult<u64> {
        let client = self.get_krpc_client()?;
        let _result = client.call("sys_config_delete", json!({"key": key}))
            .await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))?;
        self.remove_config_cache(key).await;
        Ok(0)
    }

    pub async fn append(&self,key:&str,value:&str) -> SytemConfigResult<u64> {
        let client = self.get_krpc_client()?;
        client.call("sys_config_append", json!({"key": key, "append_value": value})).await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))?;

        let revision = buckyos_get_unix_timestamp();
        let _is_changed = self.set_config_cache(key, value, revision).await;

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

        for (key, _action) in tx_actions.iter() {
            self.remove_config_cache(key).await;
        }
        Ok(0)
    }

    pub async fn dump_configs_for_scheduler(&self) -> SytemConfigResult<Value> {
        let client = self.get_krpc_client()?;
        let result = client.call("dump_configs_for_scheduler", json!({}))
            .await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))?;
        Ok(result)
    }

    pub async fn refresh_trust_keys(&self) -> SytemConfigResult<()> {
        let client = self.get_krpc_client()?;
        client.call("sys_refresh_trust_keys", json!({})).await
            .map_err(|error| SystemConfigError::ReasonError(error.to_string()))?;
        Ok(())
    }

  

}
