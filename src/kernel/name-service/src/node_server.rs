use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use serde::{Deserialize, Serialize};
use sfo_serde_result::SerdeResult;
use crate::{NSError, NSErrorCode, NSResult};

pub struct NSCmdRequest {
    cmd_data: Vec<u8>,
}

impl NSCmdRequest {
    pub fn get<T: for<'a> Deserialize<'a>>(&self) -> NSResult<T> {
        serde_json::from_slice(&self.cmd_data).map_err(|e| {
            NSError::new(NSErrorCode::InvalidData, format!("Failed to deserialize cmd data: {}", e))
        })
    }
}

impl From<Vec<u8>> for NSCmdRequest {
    fn from(value: Vec<u8>) -> Self {
        Self {
            cmd_data: value
        }
    }
}

pub struct NSCmdResponse {
    cmd_data: Vec<u8>,
}

impl NSCmdResponse {
    pub fn new(cmd_data: Vec<u8>) -> Self {
        Self {
            cmd_data,
        }
    }

    pub fn get<T: for<'a> Deserialize<'a>>(&self) -> NSResult<T> {
        serde_json::from_slice(&self.cmd_data).map_err(|e| {
            NSError::new(NSErrorCode::InvalidData, format!("Failed to deserialize cmd data: {}", e))
        })
    }
}

impl Into<Vec<u8>> for NSCmdResponse {
    fn into(self) -> Vec<u8> {
        self.cmd_data
    }
}

impl<T: Serialize> From<T> for NSCmdResponse {
    fn from(value: T) -> Self {
        serde_json::to_vec(&value).map(|data| {
            Self {
                cmd_data: data
            }
        }).unwrap()
    }
}

#[callback_trait::callback_trait]
pub(super) trait NSCmdRawHandler: 'static + Sync + Send {
    async fn handle(&self, request: NSCmdRequest) -> NSCmdResponse;
}

#[async_trait::async_trait]
pub trait NSCmdHandler<P, T>: 'static + Sync + Send where T: for<'a> Deserialize<'a> + Serialize, P: for<'a> Deserialize<'a> + Send + Sync {
    async fn handle(&self, cmd: P) -> NSResult<T>;
}

pub struct NSCmdRegister {
    handlers: Mutex<HashMap<String, Pin<Arc<dyn NSCmdRawHandler>>>>,
}

impl NSCmdRegister {
    pub (super) fn new() -> Self {
        Self {
            handlers: Mutex::new(HashMap::new())
        }
    }
    pub fn register_cmd<P: for<'a> Deserialize<'a> + Send + Sync, T: for<'a> Deserialize<'a> + Serialize>(&self, cmd_name: &str, handler: impl NSCmdHandler<P, T>) {
        let handler = Arc::pin(handler);
        self.handlers.lock().unwrap().insert(cmd_name.to_string(), Arc::pin(move |request: NSCmdRequest| {
            let tmp = handler.clone();
            async move {
                let resp: NSResult<T> = async move {
                    tmp.handle(request.get()?).await
                }.await;
                let result = SerdeResult::<_, NSError>::from(resp);
                NSCmdResponse::from(result)
            }
        }));
    }

    pub(super) fn get_cmd_handler(&self, cmd_name: &str) -> Option<Pin<Arc<dyn NSCmdRawHandler>>> {
        self.handlers.lock().unwrap().get(cmd_name).map(|v| v.clone())
    }
}
