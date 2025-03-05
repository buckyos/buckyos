//system control panel client
use std::sync::Arc;
use name_lib::DeviceConfig;
use name_lib::DeviceInfo;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use serde_json::Value;
use serde_json::json;
use log::*;
use ::kRPC::*;
use crate::system_config::*;


#[derive(Serialize, Deserialize)]
pub struct SubPkgDesc {
    pub pkg_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docker_image_name:Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_url:Option<String>,
    #[serde(flatten)]
    pub configs:HashMap<String,String>,

}
//App info is store at Index-db, publish to bucky store
#[derive(Serialize, Deserialize)]
pub struct AppDoc {
    pub app_id: String,
    pub name: String,
    pub description: String,
    pub vendor_did: String,
    pub pkg_id: String,
    //service name -> full image url
    // 命名逻辑:<arch>_<runtimne_type>_<media_type>, 
    //"amd64_docker_image" 
    //"aarch64_docker_image"
    //"amd64_win_app"
    //"amd64_linux_app"
    //"aarch64_linux_app"
    //"aarch64_macos_app"
    pub pkg_list: HashMap<String, SubPkgDesc>,
}

#[derive(Serialize, Deserialize)]
pub struct KernelServiceConfig {
    pub name: String,
    pub description: String,
    pub vendor_did: String,
    pub pkg_id: String,
    pub port: u16,
    pub node_list: Vec<String>,
    pub state: String,
    pub service_type: String,
    pub instance: u32
}


#[derive(Serialize, Deserialize)]
pub struct AppConfig {
    pub app_id: String,
    pub app_doc: AppDoc,
    pub app_index: u16, //app index in user's app list
    pub enable: bool,
    pub instance: u32,//期望的instance数量
    pub state: String,
    //mount pint
    pub data_mount_point: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_mount_point: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_cache_mount_point: Option<String>,
    //extra mount pint, real_path:docker_inner_path
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_mounts: Option<HashMap<String,String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_cpu_num: Option<u32>,
    // 0 - 100
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_cpu_percent: Option<u32>,
    
    // memory quota in bytes
    pub memory_quota: Option<u64>,
    
    //network resource, name:docker_inner_port
    pub tcp_ports: HashMap<String,u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udp_ports: Option<HashMap<String,u16>>,
}




#[derive(Serialize, Deserialize,Clone)]
pub struct AppServiceInstanceConfig {
    pub target_state: String,
    pub app_id: String,
    pub user_id: String,

    pub image_pkg_id: Option<String>,
    pub docker_image_name : Option<String>,//TODO:能否从pkg_id中推断出docker_image_name?
    pub docker_image_hash: Option<String>,
    pub direct_image: Option<String>,         // 现在这里只要是Some就可以，以后可以放二进制包的url
    pub data_mount_point: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_mount_point: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_cache_mount_point: Option<String>,
    //extra mount pint, real_path:docker_inner_path
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_mounts: Option<HashMap<String,String>>,
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_cpu_num: Option<u32>,
    // 0 - 100
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_cpu_percent: Option<u32>,
    // memory quota in bytes
    pub memory_quota: Option<u64>,

    // target port ==> real port in docker
    pub tcp_ports: HashMap<u16,u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udp_ports: Option<HashMap<u16,u16>>,
    //pub service_image_name : String, // support mutil platform image name (arm/x86...)
}



impl AppServiceInstanceConfig {
    pub fn new(owner_user_id:&str,app_config:&AppConfig) -> AppServiceInstanceConfig {
        AppServiceInstanceConfig {
            target_state: "Running".to_string(),
            app_id: app_config.app_id.clone(),
            user_id:owner_user_id.to_string(),
            image_pkg_id: None,
            docker_image_name: None,
            docker_image_hash: None,
            direct_image: None,
            data_mount_point: app_config.data_mount_point.clone(),
            cache_mount_point: app_config.cache_mount_point.clone(),
            local_cache_mount_point: app_config.local_cache_mount_point.clone(),
            extra_mounts: app_config.extra_mounts.clone(),
            max_cpu_num: app_config.max_cpu_num.clone(),
            max_cpu_percent: app_config.max_cpu_percent.clone(),
            memory_quota: app_config.memory_quota.clone(),
            tcp_ports: HashMap::new(),//TODO
            udp_ports: None,
        }
    }

    pub fn get_http_port(&self) -> Option<u16> {
        for (real_port,docker_port) in self.tcp_ports.iter() {
            if docker_port == &80 {
                return Some(*real_port);
            }
        }
        None
    }
}  



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
#[derive(Serialize, Deserialize)]
pub struct FrameServiceInstanceConfig {
    pub target_state: String,
    pub pkg_id: String,
    pub operations: HashMap<String, RunItemControlOperation>,
}

impl FrameServiceInstanceConfig {
    pub fn new(pkg_id:String)->Self {
        unimplemented!()
    }
}

//load from SystemConfig,node的配置分为下面几个部分
// 固定的硬件配置，一般只有硬件改变或损坏才会修改
// 系统资源情况，（比如可用内存等），改变密度很大。这一块暂时不用etcd实现，而是用专门的监控服务保存
// RunItem的配置。这块由调度器改变，一旦改变,node_daemon就会产生相应的控制命令
// Task(Cmd)配置，暂时不实现
#[derive(Serialize, Deserialize)]
pub struct NodeConfig {
    //pub pure_version: u64,
    pub kernel: HashMap<String, KernelServiceInstanceConfig>,
    pub apps: HashMap<String, AppServiceInstanceConfig>,
    pub frame_services: HashMap<String, FrameServiceInstanceConfig>,
    pub is_running: bool,
    pub state:Option<String>,
}

pub struct ControlPanelClient {
    system_config_client: SystemConfigClient,
}

impl ControlPanelClient {
    pub fn new(system_config_client: SystemConfigClient) -> Self {
        Self { system_config_client }
    }

    pub async fn get_device_info(&self,device_id:&str) -> Result<DeviceInfo> {
        unimplemented!()
    }

    pub async fn get_device_config(&self,device_id:&str) -> Result<DeviceConfig> {
        let device_doc_path = format!("/devices/{}/doc",device_id);
        let get_result = self.system_config_client.get(device_doc_path.as_str()).await;
        if get_result.is_err() {
            return Err(RPCErrors::ReasonError("Trust key  not found".to_string()));
        }
        let (device_config,_version) = get_result.unwrap();
        let device_config:DeviceConfig= serde_json::from_str(&device_config).map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        Ok(device_config)
    }

    //TODO: help app installer dev easy to generate right app-index
    pub async fn install_app_service(&self,user_id:&str,app_config:&AppConfig,shortcut:Option<String>) -> SytemConfigResult<u64> {
        // TODO: if you want install a web-client-app, use another function
        //1. create users/{user_id}/apps/{appid}/config
        let app_id = app_config.app_id.as_str();
        let app_config_str = serde_json::to_string(app_config).unwrap();
        self.system_config_client.create(format!("users/{}/apps/{}/config",user_id,app_id).as_str(),app_config_str.as_str()).await?;
        //2. update rbac
        self.system_config_client.append("system/rbac/policy",format!("\ng, {}, app",app_id).as_str()).await?;
        //3. update gateway shortcuts
        if shortcut.is_some() {
            let short_name = shortcut.unwrap();
            let short_json_path = format!("/shortcuts/{}",short_name.as_str());
            let short_json_value = json!({
                "type":"app",
                "user_id":user_id,
                "app_id":app_id
            });
            let short_json_value_str = serde_json::to_string(&short_json_value).unwrap();

            self.system_config_client.set_by_json_path("services/gateway/settings",
                short_json_path.as_str(),short_json_value_str.as_str()).await?;

            info!("set shortcut {} for user {}'s app {} success!",short_name,user_id,app_id);
        }

        info!("install app service {} for user {} success!",app_id,user_id);
        Ok(0)
    }

    pub async fn get_valid_app_index(&self,user_id:&str) -> SytemConfigResult<u64> {
        unimplemented!();
    }

    pub async fn remove_app(&self,appid:&str) -> SytemConfigResult<u64> {
        unimplemented!();
    }


    pub async fn disable_app(&self,appid:&str) -> SytemConfigResult<u64> {
        unimplemented!();
    }
}