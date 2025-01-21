
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use serde_json::Value;
use serde_json::json;

use crate::app_list::AppServiceConfig;

#[derive(Serialize, Deserialize)]
pub struct RunItemControlOperation {
    pub command : String,
    pub params : Option<Vec<String>>,
}

#[derive(Serialize, Deserialize)]
pub struct KernelServiceConfig {
    pub target_state: String,
    pub pkg_id: String,
    pub operations: HashMap<String, RunItemControlOperation>,
}

impl KernelServiceConfig {
    pub fn new(pkg_id:String)->Self {
        let mut operations = HashMap::new();
        operations.insert("status".to_string(),RunItemControlOperation {
            command: "status".to_string(),
            params: None,
        });
        operations.insert("start".to_string(),RunItemControlOperation {
            command: "start".to_string(),
            params: None,
        });
        operations.insert("stop".to_string(),RunItemControlOperation {
            command: "stop".to_string(),
            params: None,
        });
        
        Self {
            target_state: "Running".to_string(),
            pkg_id,
            operations,
        }
    }
}


#[derive(Serialize, Deserialize)]
pub struct NodeConfig {
    revision: u64,
    kernel: HashMap<String, KernelServiceConfig>,
    apps: HashMap<String, AppServiceConfig>,
    services: HashMap<String, KernelServiceConfig>,
    is_running: bool,
    state:Option<String>,
}