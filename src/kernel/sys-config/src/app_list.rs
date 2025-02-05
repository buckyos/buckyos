use name_lib::DeviceInfo;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;



#[derive(Serialize, Deserialize)]
pub struct SubPkgDesc {
    pub pkg_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docker_image_name:Option<String>,
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
    pub docker_image_name : Option<String>,
    
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



