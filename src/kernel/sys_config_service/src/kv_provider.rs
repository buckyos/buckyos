#![allow(dead_code)]
use std::collections::HashMap;
use async_trait::async_trait;
use thiserror::Error;
use serde_json::Value;
use buckyos_kit::*;

#[derive(Error, Debug)]
pub enum KVStoreErrors {
    #[error("key not found : {0}")]
    KeyNotFound(String),
    #[error("key exist : {0}")]
    KeyExist(String),
    #[error("internal error : {0}")]
    InternalError(String),

}

pub type Result<T> = std::result::Result<T, KVStoreErrors>; 

#[async_trait]
pub trait KVStoreProvider: Send + Sync {
    async fn get(&self, key: String) -> Result<Option<String>>;
    async fn set(&self, key: String, value: String) -> Result<()>;
    async fn set_by_path(&self, key: String, json_path: String, value: &Value) -> Result<()>;
    async fn exec_tx(&self,tx:HashMap<String,KVAction>,main_key:Option<(String,u64)>) -> Result<()>;
    async fn create(&self,key:&str,value:&str) -> Result<()>;
    async fn delete(&self,key:&str) -> Result<()>;
    async fn list_data(&self,key_perfix:&str) -> Result<HashMap<String,String>>;
    async fn list_keys(&self,key_perfix:&str) -> Result<Vec<String>>;
    async fn list_direct_children(&self, prefix: String) -> Result<Vec<String>>;
}
