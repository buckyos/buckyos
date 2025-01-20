
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


#[derive(Serialize, Deserialize)]
pub struct NodeConfig {
    revision: u64,
    kernel: HashMap<String, KernelServiceConfig>,
    apps: HashMap<String, AppServiceConfig>,
    services: HashMap<String, KernelServiceConfig>,
    is_running: bool,
    state:Option<String>,
}