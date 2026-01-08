
//system control panel client

use log::warn;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use ::kRPC::*;
use package_lib::PackageMeta;
use name_lib::DID;
use std::ops::Deref;
use crate::AppDoc;
use crate::SelectorType;

pub const SERVICE_INSTANCE_INFO_UPDATE_INTERVAL: u64 = 30;

pub const KNOWN_SERVICE_WWW: (&str, u16) = ("www",80);
pub const KNOWN_SERVICE_HTTP: (&str, u16) = ("http",80);
pub const KNOWN_SERVICE_HTTPS: (&str, u16) = ("https",443);


#[derive(Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ServiceState {
    New,
    Running,
    Stopped,
    Stopping,
    Restarting,
    Updating,
    Deleted,
}

impl Default for ServiceState {
    fn default() -> Self {
        ServiceState::New
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq,Debug)]
#[serde(rename_all = "lowercase")]
pub enum ServiceInstanceState {
    //InstllDeps,Updating... any maintanence state
    Deploying,
    NotExist,
    Exited,
    Started,
    Stopped,
}


//用于上报给调度器的实例信息
#[derive(Serialize, Deserialize, Clone)]
pub struct ServiceInstanceReportInfo {
    pub instance_id:String,
    pub node_id:String,
    pub node_did:DID,
    pub state: ServiceInstanceState,
    pub service_ports:  HashMap<String,u16>,
    pub last_update_time: u64,
    pub start_time: u64,
    pub pid: u32,
}

#[derive(Serialize, Deserialize,Clone)]
pub struct ServiceNode {
    pub node_did:DID,//device id of node,
    pub node_net_id:Option<String>,
    pub state: ServiceInstanceState,
    pub weight: u32,
    //pub service_port:  HashMap<String,u16>,
}

//有调度器定期更新的ServiceInfo, 是selector的输入信息
#[derive(Serialize, Deserialize)]
pub struct ServiceInfo {
    //TODO:后续要提供类似nginx的cluster的支持
    pub selector_type:String,//random ONLY
    //node_name -> ServiceNodeInfo
    pub node_list: HashMap<String, ServiceNode>,
}



#[derive(Serialize, Deserialize,Clone)]
pub struct ServiceExposeConfig {
    #[serde(default)]
    pub sub_hostname: Vec<String>,//for app's www service
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expose_uri: Option<String>,//for service's www service, not used now
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expose_port: Option<u16>,//for other service's port,
}

impl Default for ServiceExposeConfig {
    fn default() -> Self {
        Self {
            sub_hostname: Vec::new(),
            expose_uri: None,
            expose_port: None,
        }
    }
}


#[derive(Serialize, Deserialize,Clone)]
pub struct ServiceInstallConfig {
    //mount pint
    // folder in docker -> real folder in host
    pub data_mount_point: HashMap<String,String>,
    pub cache_mount_point: Vec<String>,
    pub local_cache_mount_point: Vec<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub bind_address: Option<String>,//为None绑定到127.0.0.1，只能通过rtcp转发访问

    #[serde(default)]
    pub expose_config: HashMap<String,ServiceExposeConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_param:Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_param:Option<String>,
    pub res_pool_id:String,
}

impl Default for ServiceInstallConfig {
    fn default() -> Self {
        Self {
            data_mount_point: HashMap::new(),
            cache_mount_point: Vec::new(),
            local_cache_mount_point: Vec::new(),
            bind_address: None,
            expose_config: HashMap::new(),
            container_param: None,
            start_param: None,
            res_pool_id: "default".to_string(),
        }
    }
}

impl ServiceInstallConfig {
    pub fn to_service_ports_config(&self) -> HashMap<String,u16> {
        let mut service_ports_config = HashMap::new();
        for (service_name, expose_config) in self.expose_config.iter() {
            if expose_config.expose_port.is_some() {
                service_ports_config.insert(service_name.clone(), expose_config.expose_port.unwrap());
            } else {
                if service_name == "www" {
                    service_ports_config.insert(service_name.clone(), 80);
                } 
                warn!("service_name: {} is not exposed", service_name);
            }
        }
        service_ports_config
    }
}

#[derive(Serialize, Deserialize,Clone)]
pub struct AppServiceSpec {
    pub app_doc: AppDoc,
    pub app_index: u16, //app index in user's app list
    pub user_id: String,

    //与调度器相关的关键参数
    pub enable: bool,
    pub expected_instance_count: u32,//期望的instance数量
    pub state: ServiceState,

    //App的active统计数据，应该使用另一个数据保存
    // pub install_time: u64,//安装时间
    // pub last_start_time: u64,//最后一次启动时间

    pub install_config:ServiceInstallConfig,
}

impl AppServiceSpec {
    pub fn app_id(&self) -> &str {
        self.app_doc.name.as_str()
    }
}


#[derive(Serialize, Deserialize,Clone)]
pub struct AppServiceInstanceConfig {
    pub target_state: ServiceInstanceState,
    pub node_id: String,
    pub app_spec : AppServiceSpec,
    //service_name -> service instance port ,use instance port can access the service
    pub service_ports_config : HashMap<String,u16>,
    //#[serde(skip_serializing_if = "Option::is_none")]
    //pub node_install_config: Option<ServiceInstallConfig>,//当存在的时候，覆盖app_spec.install_config,目前只是占位，并未使用
}
impl AppServiceInstanceConfig {
    pub fn new(node_id:&str,app_config:&AppServiceSpec) -> AppServiceInstanceConfig {
        AppServiceInstanceConfig {
            target_state: ServiceInstanceState::Started,
            node_id: node_id.to_string(),
            app_spec: app_config.clone(),
            service_ports_config: HashMap::new(),
        }
    }

    pub fn to_string(&self) -> String {
        serde_json::to_string(self).unwrap()
    }
}  
#[derive(Serialize, Deserialize, Clone)]
pub struct KernelServiceSpec {
    pub service_doc:AppDoc,
    pub enable: bool,
    pub app_index: u16,
    pub expected_instance_count: u32, 
    pub state: ServiceState,
    pub install_config:ServiceInstallConfig,
}

#[derive(Serialize, Deserialize)]
pub struct KernelServiceInstanceConfig {
    pub target_state: ServiceInstanceState,
    pub node_id: String,
    pub service_sepc:KernelServiceSpec,
}

impl KernelServiceInstanceConfig {
    pub fn new(service_sepc:KernelServiceSpec,node_id:String,)->Self {
        Self {
            target_state: ServiceInstanceState::Started,
            node_id,
            service_sepc,
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct LocalAppInstanceConfig {
    pub target_state: ServiceInstanceState,
    pub enable: bool,

    pub app_doc: AppDoc,
    pub user_id: String,

    pub install_config:ServiceInstallConfig,
}


//frame service是运行在容器中的Service，与app service的不同之处在于frame service允许被其它人依赖
//目前系统里还没有frame service
#[derive(Serialize, Deserialize)]
pub struct FrameServiceInstanceConfig {
    pub target_state: String,
    pub pkg_id: String,
}

impl FrameServiceInstanceConfig {
    pub fn new(_pkg_id:String)->Self {
        unimplemented!()
    }
}

