use std::ops::Deref;
//system control panel client
use std::sync::Arc;
use name_lib::DeviceConfig;
use name_lib::DeviceInfo;
use name_lib::ZoneConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use serde_json::Value;
use serde_json::json;
use log::*;
use ::kRPC::*;
use crate::system_config::*;
use package_lib::PackageMeta;

#[derive(Serialize, Deserialize)]
pub struct InstallConfig {
    pub data_mount_point: Vec<String>,
    pub cache_mount_point: Vec<String>,
    pub local_cache_mount_point: Vec<String>,
    pub tcp_ports: HashMap<String,u16>,
    pub udp_ports: HashMap<String,u16>,
} 

impl Default for InstallConfig {
    fn default() -> Self {
        Self {
            data_mount_point: vec![],
            cache_mount_point: vec![],
            local_cache_mount_point: vec![],
            tcp_ports: HashMap::new(),
            udp_ports: HashMap::new(),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct SubPkgDesc {
    pub pkg_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docker_image_name:Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docker_image_hash:Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_url:Option<String>,
    #[serde(flatten)]
    pub configs:HashMap<String,String>,

}
//App info is store at Index-db, publish to bucky store
#[derive(Serialize, Deserialize)]
pub struct AppDoc {
    #[serde(flatten)]    
    pub meta: PackageMeta,
    pub app_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_icon_url: Option<String>,
    pub install_config:InstallConfig,

    //service name -> full image url
    // 命名逻辑:<arch>_<runtimne_type>_<media_type>, 
    //"amd64_docker_image" 
    //"aarch64_docker_image"
    //"amd64_win_app"
    //"aarch64_apple_app"
    //"amd64_apple_app"
    pub pkg_list: HashMap<String, SubPkgDesc>,
}

impl AppDoc {
    pub fn from_pkg_meta(pkg_meta: &PackageMeta) -> Result<Self> {
        let pkg_json = serde_json::to_value(pkg_meta).unwrap();
        let result_self  = serde_json::from_value(pkg_json)
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        Ok(result_self) 
    }

    pub fn to_pkg_meta(&self) -> Result<PackageMeta> {
        let pkg_json = serde_json::to_value(self).unwrap();
        let result_pkg_meta = serde_json::from_value(pkg_json)
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        Ok(result_pkg_meta)
    }
}

impl Deref for AppDoc {
    type Target = PackageMeta;
    
    fn deref(&self) -> &Self::Target {
        &self.meta
    }
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
    pub data_mount_point: Vec<String>,
    pub cache_mount_point: Vec<String>,
    pub local_cache_mount_point: Vec<String>,
    //extra mount pint, real_path:docker_inner_path
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_cpu_num: Option<u32>,
    // 0 - 100
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_cpu_percent: Option<u32>,
    
    // memory quota in bytes
    pub memory_quota: Option<u64>,
    
    //network resource, name:docker_inner_port
    pub tcp_ports: HashMap<String,u16>,
    pub udp_ports: HashMap<String,u16>
}




#[derive(Serialize, Deserialize,Clone)]
pub struct AppServiceInstanceConfig {
    pub target_state: String,
    pub app_id: String,
    pub user_id: String,

    pub app_pkg_id: Option<String>,
    pub docker_image_pkg_id: Option<String>,
    pub docker_image_name : Option<String>,//TODO:能否从pkg_id中推断出docker_image_name?
    pub docker_image_hash: Option<String>,
    pub service_pkg_id: Option<String>,         
    pub data_mount_point: Vec<String>,
    pub cache_mount_point: Vec<String>,
    pub local_cache_mount_point: Vec<String>,
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_cpu_num: Option<u32>,
    // 0 - 100
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_cpu_percent: Option<u32>,
    // memory quota in bytes
    pub memory_quota: Option<u64>,

    // target port ==> real port in docker
    pub tcp_ports: HashMap<u16,u16>,
    pub udp_ports: HashMap<u16,u16>,
    //pub service_image_name : String, // support mutil platform image name (arm/x86...)
}



impl AppServiceInstanceConfig {
    pub fn new(owner_user_id:&str,app_config:&AppConfig) -> AppServiceInstanceConfig {
        AppServiceInstanceConfig {
            target_state: "Running".to_string(),
            app_id: app_config.app_id.clone(),
            user_id:owner_user_id.to_string(),
            app_pkg_id: None,
            docker_image_pkg_id: None,
            docker_image_name: None,
            docker_image_hash: None,
            service_pkg_id: None,
            data_mount_point: app_config.data_mount_point.clone(),
            cache_mount_point: app_config.cache_mount_point.clone(),
            local_cache_mount_point: app_config.local_cache_mount_point.clone(),
            max_cpu_num: app_config.max_cpu_num.clone(),
            max_cpu_percent: app_config.max_cpu_percent.clone(),
            memory_quota: app_config.memory_quota.clone(),
            tcp_ports: HashMap::new(),//TODO
            udp_ports: HashMap::new(),
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

    pub async fn load_zone_config(&self) -> Result<ZoneConfig> {
        let zone_config_path = "boot/config";
        let zone_config_result = self.system_config_client.get(zone_config_path).await;
        if zone_config_result.is_err() {
            return Err(RPCErrors::ReasonError("boot config(Zone config) not found".to_string()));
        }
        let (zone_config_str,_version) = zone_config_result.unwrap();
        let zone_config:ZoneConfig = serde_json::from_str(&zone_config_str)
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        Ok(zone_config)
    }

    pub async fn get_device_info(&self,device_id:&str) -> Result<DeviceInfo> {
        let device_info_path = format!("devices/{}/info",device_id);
        let get_result = self.system_config_client.get(device_info_path.as_str()).await;
        if get_result.is_err() {
            return Err(RPCErrors::ReasonError("Device info not found".to_string()));
        }
        let (device_info,_version) = get_result.unwrap();
        let device_info:DeviceInfo= serde_json::from_str(&device_info)
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        Ok(device_info)
    }
    
    pub async fn get_device_config(&self,device_id:&str) -> Result<DeviceConfig> {
        let device_doc_path = format!("devices/{}/doc",device_id);
        let get_result = self.system_config_client.get(device_doc_path.as_str()).await;
        if get_result.is_err() {
            return Err(RPCErrors::ReasonError("Trust key  not found".to_string()));
        }
        let (device_config,_version) = get_result.unwrap();
        let device_config:DeviceConfig= serde_json::from_str(&device_config)
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        Ok(device_config)
    }

    //TODO: help app installer dev easy to generate right app-index
    pub async fn install_app_service(&self,user_id:&str,app_config:&AppConfig,shortcut:Option<String>) -> Result<u64> {
        // TODO: if you want install a web-client-app, use another function
        //1. create users/{user_id}/apps/{appid}/config
        let app_id = app_config.app_id.as_str();
        let app_config_str = serde_json::to_string(app_config).unwrap();
        self.system_config_client.create(format!("users/{}/apps/{}/config",user_id,app_id).as_str(),app_config_str.as_str()).await
            .map_err(|e| RPCErrors::ReasonError(format!("install app service failed, err:{}", e)))?;
        //2. update rbac
        self.system_config_client.append("system/rbac/policy",format!("\ng, {}, app",app_id).as_str()).await
            .map_err(|e| RPCErrors::ReasonError(format!("install app service failed, err:{}", e)))?;
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
                short_json_path.as_str(),short_json_value_str.as_str()).await
                .map_err(|e| RPCErrors::ReasonError(format!("install app service failed, err:{}", e)))?;

            info!("set shortcut {} for user {}'s app {} success!",short_name,user_id,app_id);
        }

        info!("install app service {} for user {} success!",app_id,user_id);
        Ok(0)
    }

    pub async fn get_user_list(&self) -> Result<Vec<String>> {
        let user_list = self.system_config_client.list("users").await;
        if user_list.is_err() {
            return Err(RPCErrors::ReasonError("user list not found".to_string()));
        }
        Ok(user_list.unwrap())
    }

    pub async fn get_app_list(&self) -> Result<Vec<AppConfig>> {
        let user_list = self.get_user_list().await;
        if user_list.is_err() {
            return Err(RPCErrors::ReasonError("user list not found".to_string()));
        }
        let user_list = user_list.unwrap();
        let mut result_app_list = Vec::new();
        for user_id in user_list {
            let app_list = self.system_config_client.list(format!("users/{}/apps",user_id).as_str()).await;
            if app_list.is_err() {
                return Err(RPCErrors::ReasonError("app list not found".to_string()));
            }
            for app_id in app_list.unwrap() {
                let app_config = self.system_config_client.get(format!("users/{}/apps/{}/config",user_id,app_id).as_str()).await;
                if app_config.is_err() {
                    return Err(RPCErrors::ReasonError("app config not found".to_string()));
                }
                let (app_config_str,_version) = app_config.unwrap();
                let app_config:AppConfig = serde_json::from_str(&app_config_str)
                    .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
                result_app_list.push(app_config);
            }
        }
        Ok(result_app_list)
    }

    pub async fn get_services_list(&self) -> Result<Vec<KernelServiceConfig>> {
        Ok(vec![])
    }

    pub async fn get_valid_app_index(&self,user_id:&str) -> Result<u64> {
        unimplemented!();
    }

    pub async fn remove_app(&self,appid:&str) -> Result<u64> {
        unimplemented!();
    }


    pub async fn disable_app(&self,appid:&str) -> Result<u64> {
        unimplemented!();
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::system_config::*;
    use crate::control_panel::*;
    use serde_json::json;
    #[tokio::test]
    async fn test_get_parse_app_doc() {
        let app_doc = json!({
            "pkg_name":"buckyos.home-station",
            "version":"1.0.0",
            "tag":"latest",
            "app_name" : "Home Station",
            "description" : "Home Station",
            "author" : "did:bns:buckyos",
            "pub_time":1715760000,
            "pkg_list" : {
                "amd64_docker_image" : {
                    "pkg_id":"home-station-x86-img",
                    "docker_image_name":"filebrowser/filebrowser:s6"
                },
                "aarch64_docker_image" : {
                    "pkg_id":"home-station-arm64-img",
                    "docker_image_name":"filebrowser/filebrowser:s6"
                },
                "web_pages" :{
                    "pkg_id" : "home-station-web-page"
                },
                "amd64_direct_image" :{
                    "pkg_id" : "home-station-web-page",
                    "package_url": "https://web3.buckyos.io/static/home-station-win.zip"
                },
                "amd64_win_app" :{
                    "pkg_id" : "home-station-win-app"
                },
                "amd64_linux_app" :{
                    "pkg_id" : "home-station-linux-app"
                }
            }
        });
        let app_doc:AppDoc = serde_json::from_value(app_doc).unwrap();
        println!("{}#{}", app_doc.pkg_name, app_doc.version);
        let app_doc_str = serde_json::to_string_pretty(&app_doc).unwrap();
        println!("{}", app_doc_str);
    }
}
