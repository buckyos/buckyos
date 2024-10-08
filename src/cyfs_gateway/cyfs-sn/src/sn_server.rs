#![allow(unused)]
use ::kRPC::*;
use std::result::Result;
use async_trait::async_trait;
use serde_json::{Value,json};
use name_lib::*;
use log::*;
use tokio::sync::Mutex;
use std::sync::Arc;
use std::collections::HashMap;
use serde::{Serialize,Deserialize};

#[derive(Debug,Clone,Copy,Serialize,Deserialize)]
pub struct SNServerConfig {
}

pub struct SNServer {
    all_device_info:Arc<Mutex<HashMap<String,DeviceInfo>>>,
}

impl SNServer {
    pub fn new(server_config:Option<SNServerConfig>) -> Self {
        SNServer {
            all_device_info:Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn register_device(&self, req:RPCRequest) -> Result<RPCResponse,RPCErrors> {
        unimplemented!();
    }

    pub async fn update_device(&self, req:RPCRequest) -> Result<RPCResponse,RPCErrors> {
        let device_info = req.params.get("device_info");
        let owner_id = req.params.get("owner_id");
        if owner_id.is_none() || device_info.is_none() {
            return Err(RPCErrors::ParseRequestError("Invalid params, owner_id or device_info is none".to_string()));
        }
        let owner_id = owner_id.unwrap().as_str();
        if owner_id.is_none() {
            return Err(RPCErrors::ParseRequestError("Invalid params, owner_id is none".to_string()));
        }
        let owner_id = owner_id.unwrap();
        

        let device_info = serde_json::from_value::<DeviceInfo>(device_info.unwrap().clone()).map_err(|e|{
            error!("Failed to parse device info: {:?}",e);
            RPCErrors::ParseRequestError(e.to_string())
        })?;    

        let mut device_info_map = self.all_device_info.lock().await;
        let key = format!("{}_{}",owner_id,device_info.hostname);
        device_info_map.insert(key, device_info);
        let resp = RPCResponse::new(RPCResult::Success(json!({
            "code":0 
        })),req.seq);
        return Ok(resp);
    }
    
    pub async fn get_device(&self, req:RPCRequest) -> Result<RPCResponse,RPCErrors> {
        let device_id = req.params.get("device_id");
        let owner_id = req.params.get("owner_id");
        if owner_id.is_none() || device_id.is_none() {
            return Err(RPCErrors::ParseRequestError("Invalid params, owner_id or device_info is none".to_string()));
        }
        let device_id = device_id.unwrap().as_str();
        let owner_id = owner_id.unwrap().as_str();
        if device_id.is_none() || owner_id.is_none() {
            return Err(RPCErrors::ParseRequestError("Invalid params, device_id or owner_id is none".to_string()));
        }
        let device_id = device_id.unwrap();
        let owner_id = owner_id.unwrap();
        let key = format!("{}_{}",owner_id,device_id);
        let device_info_map = self.all_device_info.lock().await;
        let device_info = device_info_map.get(&key).unwrap();
        let device_json = serde_json::to_value(device_info.clone()).unwrap();
        return Ok(RPCResponse::new(RPCResult::Success(device_json),req.seq));   
    }
}

#[async_trait]
impl kRPCHandler for SNServer {
    async fn handle_rpc_call(&self, req:RPCRequest) -> Result<RPCResponse,RPCErrors> {
        match req.method.as_str() {
            "register" => {
                //register device
                return self.register_device(req).await;
            },
            "update" => {
                //update device info
                return self.update_device(req).await;
            },
            "get" => {
                //get device info
                return self.get_device(req).await;
            }
            _ => Err(RPCErrors::UnknownMethod(req.method)),
        }
    }
}


