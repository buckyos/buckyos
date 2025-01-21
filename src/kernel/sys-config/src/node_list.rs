
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use serde_json::Value;
use serde_json::json;

use crate::app_list::AppServiceInstanceConfig;

#[derive(Serialize, Deserialize)]
pub struct RunItemControlOperation {
    pub command : String,
    pub params : Option<Vec<String>>,
}

#[derive(Serialize, Deserialize)]
pub struct KernelServiceInstanceConfig {
    pub target_state: String,
    pub pkg_id: String,
    pub operations: HashMap<String, RunItemControlOperation>,
}

impl KernelServiceInstanceConfig {
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

//load from SystemConfig,node的配置分为下面几个部分
// 固定的硬件配置，一般只有硬件改变或损坏才会修改
// 系统资源情况，（比如可用内存等），改变密度很大。这一块暂时不用etcd实现，而是用专门的监控服务保存
// RunItem的配置。这块由调度器改变，一旦改变,node_daemon就会产生相应的控制命令
// Task(Cmd)配置，暂时不实现
#[derive(Serialize, Deserialize)]
pub struct NodeConfig {
    pub pure_version: u64,
    pub kernel: HashMap<String, KernelServiceInstanceConfig>,
    pub apps: HashMap<String, AppServiceInstanceConfig>,
    pub services: HashMap<String, KernelServiceInstanceConfig>,
    pub is_running: bool,
    pub state:Option<String>,
}