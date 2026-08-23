//system control panel client

use crate::{
    AppDoc, AppId, AppInstanceId, DeploymentPackage, InstanceVolumeConfig, NodeExecutionSpec,
    PermissionItem, RdbInstanceConfig, ServiceProtocol, SubPkgDesc, TaskId,
    NODE_EXECUTION_SPEC_SCHEMA_VERSION,
};
use ::kRPC::*;
use name_lib::DID;
use ndn_lib::ObjId;
use package_lib::PackageId;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
};

pub const SERVICE_INSTANCE_INFO_UPDATE_INTERVAL: u64 = 30;

pub const KNOWN_SERVICE_WWW: (&str, u16) = ("www", 80);
pub const KNOWN_SERVICE_HTTP: (&str, u16) = ("http", 80);
pub const KNOWN_SERVICE_HTTPS: (&str, u16) = ("https", 443);

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DeploymentIdentity {
    pub app_instance_id: AppInstanceId,
    pub task_id: TaskId,
    pub app_doc_object_id: ObjId,
    pub spec_generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pikg_digest: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentHealth {
    Unknown,
    Healthy,
    Unhealthy,
    Materialized,
    GatewayReady,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DeploymentError {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StaticWebDeploymentEvidence {
    pub deployment: DeploymentIdentity,
    pub node_id: String,
    pub content_id: String,
    pub gateway_config_generation: String,
    pub materialized_at: u64,
    pub gateway_ready_at: u64,
    pub observed_at: u64,
    pub expires_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployment_error: Option<DeploymentError>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ServiceState {
    #[serde(alias = "New")]
    New,
    #[serde(alias = "Running", alias = "Deployed", alias = "deployed")]
    Running,
    #[serde(alias = "Stopped", alias = "Disable")]
    Stopped,
    #[serde(alias = "Stopping")]
    Stopping,
    #[serde(alias = "Restarting")]
    Restarting,
    #[serde(alias = "Updating")]
    Updating,
    #[serde(alias = "Deleted")]
    Deleted,
}

impl Default for ServiceState {
    fn default() -> Self {
        ServiceState::New
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
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
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ServiceInstanceReportInfo {
    pub node_id: String,
    pub node_did: DID,
    pub state: ServiceInstanceState,
    pub service_ports: HashMap<String, u16>,
    pub last_update_time: u64,
    pub start_time: u64,
    pub pid: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployment: Option<DeploymentIdentity>,
    pub instance_epoch: String,
    pub node_session_id: String,
    pub observed_at: u64,
    pub expires_at: u64,
    pub health: DeploymentHealth,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployment_error: Option<DeploymentError>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ServiceNode {
    pub node_did: DID, //device id of node,
    pub node_net_id: Option<String>,
    pub state: ServiceInstanceState,
    pub weight: u32,
    pub service_port: HashMap<String, u16>,
}

//有调度器定期更新的ServiceInfo, 是selector的输入信息
#[derive(Serialize, Deserialize)]
pub struct ServiceInfo {
    //TODO:后续要提供类似nginx的cluster的支持
    pub selector_type: String, //random ONLY
    //node_name -> ServiceNodeInfo
    pub node_list: HashMap<String, ServiceNode>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ServiceEndpointConfig {
    pub protocol: ServiceProtocol,
    pub inner_port: u16,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServiceExposeRouteConfig {
    Web {
        #[serde(default)]
        sub_hostname: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expose_uri: Option<String>,
    },
    Port {
        expose_port: u16,
    },
}

impl Default for ServiceExposeRouteConfig {
    fn default() -> Self {
        Self::Web {
            sub_hostname: Vec::new(),
            expose_uri: None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ServiceExposeSetting {
    pub route: ServiceExposeRouteConfig,
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub allow_guest: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ServiceSetting {
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expose: Option<ServiceExposeSetting>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ServiceSettings {
    #[serde(default)]
    pub services: HashMap<String, ServiceSetting>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ServiceExposeConfig {
    pub route: ServiceExposeRouteConfig,
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub allow_guest: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bind_address: Option<String>, //为None绑定到127.0.0.1，只能通过rtcp转发访问
}

impl Default for ServiceExposeConfig {
    fn default() -> Self {
        Self {
            route: ServiceExposeRouteConfig::default(),
            scope: String::new(),
            allow_guest: false,
            bind_address: None,
        }
    }
}

impl ServiceExposeConfig {
    pub fn web(sub_hostname: Vec<String>, scope: String, allow_guest: bool) -> Self {
        Self {
            route: ServiceExposeRouteConfig::Web {
                sub_hostname,
                expose_uri: None,
            },
            scope,
            allow_guest,
            bind_address: None,
        }
    }

    pub fn port(expose_port: u16, scope: String, allow_guest: bool) -> Self {
        Self {
            route: ServiceExposeRouteConfig::Port { expose_port },
            scope,
            allow_guest,
            bind_address: None,
        }
    }

    pub fn expose_port(&self) -> Option<u16> {
        match &self.route {
            ServiceExposeRouteConfig::Port { expose_port } => Some(*expose_port),
            ServiceExposeRouteConfig::Web { .. } => None,
        }
    }

    pub fn sub_hostname(&self) -> &[String] {
        match &self.route {
            ServiceExposeRouteConfig::Web { sub_hostname, .. } => sub_hostname,
            ServiceExposeRouteConfig::Port { .. } => &[],
        }
    }

    pub fn set_sub_hostname(&mut self, value: Vec<String>) -> bool {
        match &mut self.route {
            ServiceExposeRouteConfig::Web { sub_hostname, .. } => {
                *sub_hostname = value;
                true
            }
            ServiceExposeRouteConfig::Port { .. } => false,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MountPointConfig {
    pub target_path: PathBuf,
    pub access: String, //read_only, read_write, read_write_append
}

//ServiceConfigTips + Installer UI = ServiceSpecConfig
// 调度器和AppLoader不关心ServiceConfigTips，只关心ServiceSpecConfig
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ServiceSpecConfig {
    #[serde(default)]
    pub service_config: HashMap<String, ServiceEndpointConfig>,
    #[serde(default)]
    pub expose_config: HashMap<String, ServiceExposeConfig>,

    //mount pint
    // folder in docker -> real folder in host
    #[serde(default)]
    pub data_mount_point: HashMap<PathBuf, MountPointConfig>,
    // folder in docker -> local cache folder in host
    #[serde(default)]
    pub local_cache_mount_point: HashMap<PathBuf, MountPointConfig>,
    // folder in docker -> external mount point in host
    #[serde(default)]
    pub external_mount_point: HashMap<PathBuf, MountPointConfig>,
    #[serde(default)]
    pub instance_volume: InstanceVolumeConfig,

    #[serde(default)]
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub rdb_instances: HashMap<String, RdbInstanceConfig>,
    #[serde(default)]
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub bash_envs: HashMap<String, String>,

    #[serde(default = "default_res_pool_id")]
    pub res_pool_id: String,

    #[serde(default)]
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub runtime_caps: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_param: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_param: Option<String>,
}

fn default_res_pool_id() -> String {
    "default".to_string()
}

impl Default for ServiceSpecConfig {
    fn default() -> Self {
        Self {
            data_mount_point: HashMap::new(),
            local_cache_mount_point: HashMap::new(),
            external_mount_point: HashMap::new(),
            service_config: HashMap::new(),
            expose_config: HashMap::new(),
            container_param: None,
            start_param: None,
            res_pool_id: default_res_pool_id(),
            rdb_instances: HashMap::new(),
            instance_volume: InstanceVolumeConfig::default(),
            bash_envs: HashMap::new(),
            runtime_caps: HashMap::new(),
        }
    }
}

impl ServiceSpecConfig {
    pub fn to_service_ports_config(&self) -> HashMap<String, u16> {
        let mut service_ports_config = HashMap::new();
        for (service_name, service_config) in &self.service_config {
            service_ports_config.insert(service_name.clone(), service_config.inner_port);
        }
        service_ports_config
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AppServiceSpec {
    pub app_instance_id: AppInstanceId,
    pub app_did: DID,
    pub deployment: DeploymentIdentity,
    pub app_doc: AppDoc,
    /// scheduler-only projection from `system/app_registry`.
    pub app_name: String,
    /// scheduler-only projection from `system/app_registry`.
    pub app_host_name: String,
    /// scheduler-only projection from `system/app_registry`.
    pub app_index: u16,
    pub owner_user_id: String,
    pub permission: Vec<PermissionItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub selected_components: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub packages: Vec<DeploymentPackage>,

    //与调度器相关的关键参数
    pub enable: bool,
    pub expected_instance_count: u32, //期望的instance数量
    pub state: ServiceState,

    //App的active统计数据，应该使用另一个数据保存
    // pub install_time: u64,//安装时间
    // pub last_start_time: u64,//最后一次启动时间
    pub spec_config: ServiceSpecConfig,
}

impl AppServiceSpec {
    pub fn app_id(&self) -> &AppId {
        self.app_instance_id.app_id()
    }

    pub fn app_instance_id(&self) -> &AppInstanceId {
        &self.app_instance_id
    }

    pub fn to_node_execution_spec(&self) -> Result<NodeExecutionSpec> {
        if self.app_instance_id != self.deployment.app_instance_id
            || self.owner_user_id != self.app_instance_id.owner_user_id()
            || AppId::from_app_did(&self.app_did).as_ref() != Ok(self.app_id())
        {
            return Err(RPCErrors::ReasonError(
                "AppServiceSpec contains inconsistent identity fields".to_string(),
            ));
        }
        let mut packages = BTreeMap::new();
        for package in &self.packages {
            let parsed = PackageId::parse(&package.pkg_id).map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "deployment package `{}` has invalid PackageId: {error}",
                    package.sub_pkg_name
                ))
            })?;
            if parsed.objid.as_deref() != Some(package.package_meta_object_id.to_string().as_str())
            {
                return Err(RPCErrors::ReasonError(format!(
                    "deployment package `{}` does not pin its Package Meta ObjectId",
                    package.sub_pkg_name
                )));
            }
            let previous = packages.insert(
                package.sub_pkg_name.clone(),
                SubPkgDesc {
                    pkg_id: package.pkg_id.clone(),
                    pkg_objid: Some(package.package_meta_object_id.clone()),
                    docker_image_name: package.docker_image_name.clone(),
                    docker_image_digest: package.docker_image_digest.clone(),
                    source_url: None,
                    selector: None,
                    required: Some(true),
                },
            );
            if previous.is_some() {
                return Err(RPCErrors::ReasonError(format!(
                    "duplicate deployment package `{}`",
                    package.sub_pkg_name
                )));
            }
        }
        Ok(NodeExecutionSpec {
            schema_version: NODE_EXECUTION_SPEC_SCHEMA_VERSION,
            app_instance_id: self.app_instance_id.clone(),
            app_did: self.app_did.clone(),
            app_doc_object_id: self.deployment.app_doc_object_id.clone(),
            spec_generation: self.deployment.spec_generation,
            app_type: self.app_doc.get_app_type(),
            packages,
            permission: self.permission.clone(),
            service_spec_config: self.spec_config.clone(),
            app_name: self.app_name.clone(),
            app_host_name: self.app_host_name.clone(),
            app_index: self.app_index,
        })
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AppServiceInstanceConfig {
    pub target_state: ServiceInstanceState,
    pub node_id: String,
    pub node_execution_spec: NodeExecutionSpec,
    //service_name -> service instance port ,use instance port can access the service
    pub service_ports_config: HashMap<String, u16>,
    pub deployment: DeploymentIdentity,
}
impl AppServiceInstanceConfig {
    pub fn new(node_id: &str, app_config: &AppServiceSpec) -> Result<AppServiceInstanceConfig> {
        let node_execution_spec = app_config.to_node_execution_spec()?;
        node_execution_spec
            .validate_against(&app_config.deployment)
            .map_err(RPCErrors::ReasonError)?;
        Ok(AppServiceInstanceConfig {
            target_state: ServiceInstanceState::Started,
            node_id: node_id.to_string(),
            node_execution_spec,
            service_ports_config: HashMap::new(),
            deployment: app_config.deployment.clone(),
        })
    }

    pub fn to_string(&self) -> String {
        serde_json::to_string(self).unwrap()
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct KernelServiceSpec {
    pub service_doc: AppDoc,
    pub enable: bool,
    pub app_index: u16,
    //系统服务使用系统的内置的RBAC权限配置，不做个性化配置
    //pub permission: Vec<PermissionItem>,
    pub expected_instance_count: u32,
    pub state: ServiceState,
    pub spec_config: ServiceSpecConfig,
}

#[derive(Serialize, Deserialize)]
pub struct KernelServiceInstanceConfig {
    pub target_state: ServiceInstanceState,
    pub node_id: String,
    pub service_sepc: KernelServiceSpec,
}

impl KernelServiceInstanceConfig {
    pub fn new(service_sepc: KernelServiceSpec, node_id: String) -> Self {
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

    pub install_config: ServiceSpecConfig,
}

//frame service是运行在容器中的Service，与app service的不同之处在于frame service允许被其它人依赖
//目前系统里还没有frame service
#[derive(Serialize, Deserialize)]
pub struct FrameServiceInstanceConfig {
    pub target_state: String,
    pub pkg_id: String,
}

impl FrameServiceInstanceConfig {
    pub fn new(_pkg_id: String) -> Result<Self> {
        Err(RPCErrors::ReasonError(
            "NotImplemented: FrameServiceInstanceConfig::new".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RdbBackend;

    #[test]
    fn service_runtime_and_exposure_are_independent() {
        let setting = ServiceSetting {
            enabled: true,
            expose: None,
        };
        let mut spec = ServiceSpecConfig::default();
        spec.service_config.insert(
            "smb".to_string(),
            ServiceEndpointConfig {
                protocol: ServiceProtocol::Tcp,
                inner_port: 445,
            },
        );

        assert!(setting.expose.is_none());
        assert!(spec.service_config.contains_key("smb"));
        assert!(!spec.expose_config.contains_key("smb"));
        assert_eq!(spec.to_service_ports_config().get("smb"), Some(&445));
    }

    #[test]
    fn expose_route_serialization_distinguishes_web_and_port() {
        let web = serde_json::to_value(ServiceExposeRouteConfig::Web {
            sub_hostname: vec!["files".to_string()],
            expose_uri: Some("/kapi/files".to_string()),
        })
        .unwrap();
        let port =
            serde_json::to_value(ServiceExposeRouteConfig::Port { expose_port: 445 }).unwrap();

        assert_eq!(web["type"], "web");
        assert_eq!(web["sub_hostname"][0], "files");
        assert_eq!(
            port,
            serde_json::json!({"type": "port", "expose_port": 445})
        );
    }

    #[test]
    fn service_spec_preserves_full_rdb_config() {
        let rdb = RdbInstanceConfig {
            backend: RdbBackend::Postgres,
            version: 3,
            schema: HashMap::from([(
                RdbBackend::Postgres,
                "create table demo(id int)".to_string(),
            )]),
            connection: "postgres://scheduler-assigned".to_string(),
        };
        let mut spec = ServiceSpecConfig::default();
        spec.rdb_instances.insert("main".to_string(), rdb.clone());

        let value = serde_json::to_value(&spec).unwrap();
        let decoded: ServiceSpecConfig = serde_json::from_value(value).unwrap();
        assert_eq!(decoded.rdb_instances.get("main"), Some(&rdb));
    }
}
