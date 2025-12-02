use std::ops::Deref;
//system control panel client

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use ::kRPC::*;
use package_lib::PackageMeta;

pub const SERVICE_INSTANCE_INFO_UPDATE_INTERVAL: u64 = 30;

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ServiceState {
    New,
    Running,
    Stopped,
    Starting,
    Stopping,
    Restarting,
    Updating,
}

impl Default for ServiceState {
    fn default() -> Self {
        ServiceState::New
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq,Debug)]
#[serde(rename_all = "lowercase")]
pub enum ServiceInstanceState {
    //InstllDeps,
    Deploying,
    //DeployFailed(String,u32), //error message,failed count
    NotExist,
    Started,
    Stopped,
}


//用于上报给调度器的实例信息
#[derive(Serialize, Deserialize, Clone)]
pub struct ServiceInstanceReportInfo {
    pub instance_id:String,
    pub state: ServiceInstanceState,
    pub service_ports:  HashMap<String,u16>,
    pub last_update_time: u64,
    pub start_time: u64,
    pub pid: u32,
}

#[derive(Serialize, Deserialize,Clone)]
pub struct ServiceNode {
    pub node_did:String,
    pub node_net_id:Option<String>,
    pub state: ServiceInstanceState,
    pub weight: u32,
    pub service_port:  HashMap<String,u16>,
}

//有调度器定期更新的ServiceInfo, 是selector的输入信息
#[derive(Serialize, Deserialize)]
pub struct ServiceInfo {
    //TODO:后续要提供类似nginx的cluster的支持
    pub selector_type:String,//random ONLY
    //node_name -> ServiceNodeInfo
    pub node_list: HashMap<String, ServiceNode>,
}

//AppDoc \ InstallConfig \ ServiceSpec \ InstanceConfig 的基本设计
// App开发者发布的，有签名的Config是 AppDoc （已知应用，其更新应该走did-document的标准机制)
// AppDoc + InstallConfig后，保存在system_config（已安装应用）上的是 [AppServiceSpec],如果应用有更新，必要的时候是需要修改AppServiceSpec来执行更新的
// 调度器基于AppServiceSpec，部署在Node上的是 AppInstanceConfig (这个必然是自动构建的)
//    为了减少多次获取信息的一致性问题，AppInstanceConfig中包含了所有信息（包含AppDoc,InstallConfig)

#[derive(Serialize, Deserialize,Clone)]
pub struct ServiceInstallConfigTips {
    pub data_mount_point: Vec<String>,
    pub local_cache_mount_point: Vec<String>,


    //通过tcp_ports和udp_ports,可以知道该Service实现了哪些服务
    //系统允许多个不同的app实现同一个服务，但有不同的“路由方法”
    //比如 如果系统里app1 有配置 {"smb":445},app2有配置 {"smb":445}，此时系统选择使用app2作为smb服务提供者，则最终按如下流程完成访问
    //   client->zone_gateway:445 --rtcp-> node_gateway:rtcp_stack -> docker_port 127:0.0.1:2190(调度器随机分配给app2) -> app2:445
    //                                                                docker_port 127.0.0.1:2189 -> app1:445
    //   此时基于app1.service_info可以通过 node_gateway:2189访问到app1的smb服务
    //service_name(like,http , smb, dns, etc...) -> real port
    pub service_ports: HashMap<String,u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_param:Option<String>,
    #[serde(flatten)]
    pub custom_config:HashMap<String,serde_json::Value>,
} 

impl Default for ServiceInstallConfigTips {
    fn default() -> Self {
        Self {
            data_mount_point: vec![],
            local_cache_mount_point: vec![],
            service_ports: HashMap::new(),
            container_param: None,
            custom_config: HashMap::new(),
        }
    }
}

#[derive(Serialize, Deserialize,Clone)]
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

#[derive(Serialize, Deserialize,Clone)]
pub struct SubPkgList {
    pub amd64_docker_image: Option<SubPkgDesc>,
    pub aarch64_docker_image: Option<SubPkgDesc>,
    pub amd64_win_app: Option<SubPkgDesc>,
    pub aarch64_apple_app: Option<SubPkgDesc>,
    pub web: Option<SubPkgDesc>,
    #[serde(flatten)]
    pub others: HashMap<String, SubPkgDesc>,
}

impl SubPkgList {

    pub fn get_app_pkg_id(&self) -> Option<String> {
        //根据编译时的目标系统，返回对应的app pkg_id
        if cfg!(target_os = "macos") {
            if let Some(pkg) = &self.aarch64_apple_app {
                return Some(pkg.pkg_id.clone());
            }
        } else if cfg!(target_os = "windows") {
            if let Some(pkg) = &self.amd64_win_app {
                return Some(pkg.pkg_id.clone());
            }
        }

        None
    }

    pub fn get_docker_image_pkg_id(&self) -> Option<String> {
        //根据当前编译期架构，返回对应的docker image pkg_id
        if cfg!(target_arch = "aarch64") {
            if let Some(pkg) = &self.aarch64_docker_image {
                return Some(pkg.pkg_id.clone());
            }
        } else {
            if let Some(pkg) = &self.amd64_docker_image {
                return Some(pkg.pkg_id.clone());
            }
        }

        None
    }
    pub fn get(&self, key: &str) -> Option<&SubPkgDesc> {
        match key {
            "amd64_docker_image" => self.amd64_docker_image.as_ref(),
            "aarch64_docker_image" => self.aarch64_docker_image.as_ref(),
            "amd64_win_app" => self.amd64_win_app.as_ref(),
            "aarch64_apple_app" => self.aarch64_apple_app.as_ref(),
            "web" => self.web.as_ref(),
            _ => self.others.get(key),
        }
    }

    pub fn iter(&self) -> Vec<(String, &SubPkgDesc)> {
        let mut list = Vec::new();
        if let Some(pkg) = &self.amd64_docker_image {
            list.push(("amd64_docker_image".to_string(), pkg));
        }
        if let Some(pkg) = &self.aarch64_docker_image {
            list.push(("aarch64_docker_image".to_string(), pkg));
        }
        if let Some(pkg) = &self.amd64_win_app {
            list.push(("amd64_win_app".to_string(), pkg));
        }
        if let Some(pkg) = &self.aarch64_apple_app {
            list.push(("aarch64_apple_app".to_string(), pkg));
        }
        if let Some(pkg) = &self.web {
            list.push(("web".to_string(), pkg));
        }
        for (k, v) in self.others.iter() {
            list.push((k.clone(), v));
        }
        list
    }
}

#[derive(Serialize, Deserialize,Clone)]
#[serde(try_from = "String", into = "String")]
pub enum SelectorType {
    Single,
    StaticWeb,//no instance, only one static web page
    Random,
    Custom(String),//custom selector type, like "round_robin"
}

impl Default for SelectorType {
    fn default() -> Self {
        Self::Single
    }
}

impl From<SelectorType> for String {
    fn from(value: SelectorType) -> Self {
        match value {
            SelectorType::Single => "single".into(),
            SelectorType::StaticWeb => "static_web".into(),
            SelectorType::Random => "random".into(),
            SelectorType::Custom(s) => s,
        }
    }
}

impl TryFrom<String> for SelectorType {
    type Error = &'static str;

    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        Ok(match value.as_str() {
            "single" => SelectorType::Single,
            "static_web" => SelectorType::StaticWeb,
            "random" => SelectorType::Random,
            other => SelectorType::Custom(other.to_owned()),
        })
    }
}


//App doc is store at Index-db, publish to bucky store
#[derive(Serialize, Deserialize,Clone)]
pub struct AppDoc {
    #[serde(flatten)]    
    pub meta: PackageMeta,
    pub show_name: String, // just for display, app_id is meta.pkg_name (like "buckyos-filebrowser")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_icon_url: Option<String>,
    pub selector_type:SelectorType,
    pub install_config_tips:ServiceInstallConfigTips,
    pub pkg_list: SubPkgList,
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

#[derive(Serialize, Deserialize,Clone)]
pub struct ServiceInstallConfig {
    //mount pint
    // folder in docker -> real folder in host
    pub data_mount_point: HashMap<String,String>,
    pub cache_mount_point: Vec<String>,
    pub local_cache_mount_point: Vec<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub bind_address: Option<String>,//为None绑定到127.0.0.1，只能通过rtcp转发访问
    //network resource, name:docker_inner_port
    pub service_ports: HashMap<String,u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_param:Option<String>,

    pub res_pool_id:String,
}

impl Default for ServiceInstallConfig {
    fn default() -> Self {
        Self {
            data_mount_point: HashMap::new(),
            cache_mount_point: Vec::new(),
            local_cache_mount_point: Vec::new(),
            bind_address: None,
            service_ports: HashMap::new(),
            container_param: None,
            res_pool_id: "default".to_string(),
        }
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
        self.app_doc.pkg_name.as_str()
    }

    pub fn container_service_port(&self, service_name: &str) -> Option<u16> {
        self.install_config.service_ports.get(service_name).copied()
    }
}


#[derive(Serialize, Deserialize,Clone)]
pub struct AppServiceInstanceConfig {
    pub target_state: ServiceInstanceState,
    pub node_id: String,
    pub app_spec : AppServiceSpec,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_install_config: Option<ServiceInstallConfig>,
}
impl AppServiceInstanceConfig {
    pub fn new(node_id:&str,app_config:&AppServiceSpec) -> AppServiceInstanceConfig {
        AppServiceInstanceConfig {
            target_state: ServiceInstanceState::Started,
            node_id: node_id.to_string(),
            app_spec: app_config.clone(),
            node_install_config: None,
        }
    }

    pub fn to_string(&self) -> String {
        serde_json::to_string(self).unwrap()
    }

    pub fn get_host_service_port(&self, service_name: &str) -> Option<u16> {
        if let Some(node_install_config) = &self.node_install_config {
            if let Some(port) = node_install_config.service_ports.get(service_name) {
                return Some(*port);
            }
        }
        self.app_spec
            .install_config
            .service_ports
            .get(service_name)
            .copied()
    }
}  

#[derive(Serialize, Deserialize, Clone)]
pub struct KernelServiceDoc {
    #[serde(flatten)]    
    pub meta: PackageMeta,
    pub show_name: String,//just for display
    pub selector_type:SelectorType,
}

impl Deref for KernelServiceDoc {
    type Target = PackageMeta;
    
    fn deref(&self) -> &Self::Target {
        &self.meta
    }
}
impl KernelServiceDoc {
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

#[derive(Serialize, Deserialize, Clone)]
pub struct KernelServiceSpec {
    pub service_doc:KernelServiceDoc,
    pub enable: bool,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system_config::*;
    use crate::control_panel::*;
    use serde_json::json;
    #[tokio::test]
    async fn test_get_parse_app_doc() {
        let app_doc = json!({
            "pkg_name": "buckyos_filebrowser",
            "version": "0.4.1",
            "tag": "latest",
            "show_name": "BuckyOS File Browser",
            "description": {
                "detail": "BuckyOS File Browser"
            },
            "author": "did:web:buckyos.ai",
            "owner": "did:web:buckyos.ai",
            "pub_time": 1743008063u64,
            "exp": 1837616063u64,
            "selector_type": "single",
            "install_config_tips": {
                "data_mount_point": ["/srv/", "/database/", "/config/"],
                "local_cache_mount_point": [],
                "service_ports": {
                    "www": 80
                }
            },
            "pkg_list": {
                "amd64_docker_image": {
                    "pkg_id": "nightly-linux-amd64.buckyos_filebrowser-img#0.4.1",
                    "docker_image_name": "buckyos/nightly-buckyos-filebrowser:0.4.1-amd64"
                },
                "aarch64_docker_image": {
                    "pkg_id": "nightly-linux-aarch64.buckyos_filebrowser-img#0.4.1",
                    "docker_image_name": "buckyos/nightly-buckyos-filebrowser:0.4.1-aarch64"
                },
                "amd64_win_app": {
                    "pkg_id": "nightly-windows-amd64.buckyos_filebrowser-bin#0.4.1"
                },
                "aarch64_apple_app": {
                    "pkg_id": "nightly-apple-aarch64.buckyos_filebrowser-bin#0.4.1"
                },
                "amd64_apple_app": {
                    "pkg_id": "nightly-apple-amd64.buckyos_filebrowser-bin#0.4.1"
                }
            },
            "deps": {
                "nightly-linux-amd64.buckyos_filebrowser-img": "0.4.1",
                "nightly-linux-aarch64.buckyos_filebrowser-img": "0.4.1",
                "nightly-windows-amd64.buckyos_filebrowser-bin": "0.4.1",
                "nightly-apple-amd64.buckyos_filebrowser-bin": "0.4.1",
                "nightly-apple-aarch64.buckyos_filebrowser-bin": "0.4.1"
            }
        });
        let app_doc:AppDoc = serde_json::from_value(app_doc).unwrap();
        println!("{}#{}", app_doc.pkg_name, app_doc.version);
        let app_doc_str = serde_json::to_string_pretty(&app_doc).unwrap();
        println!("{}", app_doc_str);
    }
}
