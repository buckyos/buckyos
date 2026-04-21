use std::collections::HashMap;
use std::collections::HashSet;

use anyhow::Result;
use buckyos_api::{AppType, SelectorType};
use log::*;
use package_lib::PackageId;
use rbac::DEFAULT_POLICY;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::app::*;
use crate::scheduler::*;
use crate::service::*;
use buckyos_api::{
    AppServiceSpec, CONTROL_PANEL_SERVICE_PORT, KLOG_CLUSTER_ADMIN_SERVICE_NAME,
    KLOG_CLUSTER_INTER_SERVICE_NAME, KLOG_CLUSTER_RAFT_SERVICE_NAME, KLOG_SERVICE_UNIQUE_ID,
    KernelServiceSpec, NodeConfig, ServiceInstanceReportInfo, ServiceState, UserSettings,
    UserType as ApiUserType, ZoneGatewaySettings, get_buckyos_api_runtime,
};
use buckyos_kit::*;
use name_client::*;
use name_lib::{DeviceInfo, ZoneConfig, get_x_from_jwk};

const SYSTEM_CONFIG_SERVICE_PORT: u16 = 3200;
const FIXED_SERVICE_WEIGHT: u32 = 100;
const DEFAULT_REQUIRED_MEMORY: u64 = 32 * 1024 * 1024;
const DEFAULT_KLOG_CLUSTER_ROUTE_PREFIX: &str = "/.cluster/klog";

fn normalize_klog_cluster_route_prefix(prefix: &str) -> String {
    let trimmed = prefix.trim();
    if trimmed.is_empty() {
        return DEFAULT_KLOG_CLUSTER_ROUTE_PREFIX.to_string();
    }

    let with_leading_slash = if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{}", trimmed)
    };

    let normalized = with_leading_slash.trim_end_matches('/').to_string();
    if normalized.is_empty() {
        return "/".to_string();
    }

    normalized
}

fn map_api_user_type(user_type: &ApiUserType) -> UserType {
    match user_type {
        ApiUserType::Admin | ApiUserType::Root => UserType::Admin,
        ApiUserType::Limited => UserType::Limited,
        _ => UserType::User,
    }
}

fn craete_node_item_by_device_info(device_name: &str, device_info: &DeviceInfo) -> NodeItem {
    let node_state =
        crate::scheduler::NodeState::from(device_info.state.clone().unwrap_or("Ready".to_string()));
    let net_id = device_info.net_id.clone().unwrap_or("".to_string());
    NodeItem {
        id: device_name.to_string(),
        node_type: NodeType::from(device_info.device_doc.device_type.clone()),
        labels: vec![],
        network_zone: net_id,
        state: node_state,
        support_container: device_info.support_container,
        available_cpu_mhz: device_info.cpu_mhz.unwrap_or(2000) as u32,
        total_cpu_mhz: device_info.cpu_mhz.unwrap_or(2000) as u32,
        total_memory: device_info.total_mem.unwrap_or(1024 * 1024 * 1024 * 2) as u64,
        available_memory: device_info.total_mem.unwrap_or(1024 * 1024 * 1024 * 2) as u64
            - device_info.mem_usage.unwrap_or(0) as u64,
        total_gpu_memory: device_info.gpu_total_mem.unwrap_or(0) as u64,
        available_gpu_memory: device_info.gpu_total_mem.unwrap_or(0) as u64
            - device_info.gpu_used_mem.unwrap_or(0) as u64,
        gpu_tflops: device_info.gpu_tflops.unwrap_or(0.0) as f32,
        resources: HashMap::new(),
        op_tasks: vec![],
    }
}

fn create_service_spec_by_app_config(
    full_app_id: &str,
    owner_user_id: &str,
    app_config: &AppServiceSpec,
) -> ServiceSpec {
    let spec_state = ServiceSpecState::from(app_config.state.clone());

    let mut need_container = app_config.app_doc.get_app_type() == AppType::Agent;
    if !need_container {
        need_container = true;
        if app_config
            .app_doc
            .pkg_list
            .iter()
            .into_iter()
            .any(|(_, pkg)| pkg.docker_image_name.is_none())
            && (app_config.app_doc.author == "did:web:buckyos.ai"
                || app_config.app_doc.author == "did:web:buckyos.io"
                || app_config.app_doc.author == "did:web:buckyos.org")
        {
            need_container = false;
        }
    }

    let service_ports_config = app_config.install_config.to_service_ports_config();
    ServiceSpec {
        id: full_app_id.to_string(),
        app_index: app_config.app_index,
        app_id: app_config.app_id().to_string(),
        owner_id: owner_user_id.to_string(),
        spec_type: ServiceSpecType::App,
        state: spec_state,
        need_container,
        best_instance_count: app_config.expected_instance_count,
        required_cpu_mhz: 200,
        required_memory: DEFAULT_REQUIRED_MEMORY,
        required_gpu_tflops: 0.0,
        required_gpu_mem: 0,
        node_affinity: None,
        network_affinity: None,
        service_ports_config: service_ports_config,
    }
}

fn create_service_spec_by_service_config(
    service_name: &str,
    service_config: &KernelServiceSpec,
) -> ServiceSpec {
    let spec_state = ServiceSpecState::from(service_config.state.clone());
    let service_ports_config = service_config.install_config.to_service_ports_config();
    ServiceSpec {
        id: service_name.to_string(),
        app_id: service_name.to_string(),
        app_index: 0,
        owner_id: "root".to_string(),
        spec_type: ServiceSpecType::Kernel,
        state: spec_state,
        need_container: false,
        best_instance_count: service_config.expected_instance_count,
        required_cpu_mhz: 300,
        required_memory: DEFAULT_REQUIRED_MEMORY,
        required_gpu_tflops: 0.0,
        required_gpu_mem: 0,
        node_affinity: None,
        network_affinity: None,
        service_ports_config: service_ports_config,
    }
}

pub fn create_scheduler_by_system_config(
    input_config: &HashMap<String, String>,
) -> Result<(NodeScheduler, HashMap<String, DeviceInfo>)> {
    let mut scheduler_ctx = NodeScheduler::new_empty(1);
    let mut device_list: HashMap<String, DeviceInfo> = HashMap::new();
    for (key, value) in input_config.iter() {
        //add node
        if key.starts_with("devices/") && key.ends_with("/info") {
            let device_name = key.split('/').nth(1).unwrap();
            let device_info: DeviceInfo = serde_json::from_str(value).map_err(|e| {
                error!("serde_json::from_str failed: {:?}", e);
                e
            })?;
            let node_item = craete_node_item_by_device_info(device_name, &device_info);
            let node_item = craete_node_item_by_device_info(device_name, &device_info);
            device_list.insert(device_name.to_string(), device_info);
            scheduler_ctx.add_node(node_item);
        }

        //add app service_spec
        if key.starts_with("users/") {
            if key.ends_with("/spec") {
                let parts: Vec<&str> = key.split('/').collect();
                if parts.len() >= 4 && (parts[2] == "apps" || parts[2] == "agents") {
                    let user_id = parts[1];
                    let app_id = parts[3];
                    let full_appid = format!("{}@{}", app_id, user_id);
                    let app_config: AppServiceSpec =
                        serde_json::from_str(value.as_str()).map_err(|e| {
                            error!(
                                "AppConfig serde_json::from_str failed: {:?} {}",
                                e,
                                value.as_str()
                            );
                            e
                        })?;
                    if app_config.app_doc.selector_type != SelectorType::Static {
                        let service_spec = create_service_spec_by_app_config(
                            full_appid.as_str(),
                            user_id,
                            &app_config,
                        );
                        scheduler_ctx.add_service_spec(service_spec);
                    }
                }
            } else if key.ends_with("/settings") {
                let parts: Vec<&str> = key.split('/').collect();
                if parts.len() >= 3 {
                    let user_id = parts[1];
                    let user_settings: UserSettings = serde_json::from_str(value.as_str())
                        .map_err(|e| {
                            error!("UserSettings serde_json::from_str failed: {:?}", e);
                            e
                        })?;
                    let user_item = UserItem {
                        userid: user_id.to_string(),
                        res_pool_id: None,
                        user_type: map_api_user_type(&user_settings.user_type),
                    };
                    scheduler_ctx.add_user(user_item);
                    if user_id == "root" {
                        scheduler_ctx.default_user_id = user_settings.user_id.clone();
                    }
                }
            }
        }

        //add service service_spec
        if key.starts_with("services/") && key.ends_with("/spec") {
            let service_name = key.split('/').nth(1).unwrap();
            let service_config: KernelServiceSpec =
                serde_json::from_str(value.as_str()).map_err(|e| {
                    error!("KernelServiceConfig serde_json::from_str failed: {:?}", e);
                    e
                })?;
            let service_spec = create_service_spec_by_service_config(service_name, &service_config);
            scheduler_ctx.add_service_spec(service_spec);
        }

        if key.starts_with("nodes/") && key.ends_with("/config") {
            let key_parts = key.split('/').collect::<Vec<&str>>();
            let node_id = key_parts[1];
            let node_config: NodeConfig = serde_json::from_str(value.as_str()).map_err(|e| {
                error!("NodeConfig serde_json::from_str failed: {:?}", e);
                e
            })?;
            for (app_instance_id, app_config) in node_config.apps.iter() {
                let app_config_str = app_config.to_string();
                info!(
                    "add app instance:{},{}",
                    format!("{} @ {}", app_instance_id, node_id),
                    app_config_str.as_str()
                );

                //let node_install_config = app_config.node_install_config.as_ref().unwrap();

                //let service_port = node_install_config.service_ports.get("www").unwrap_or(&80);
                //info!("app_id: {}, service_port: {}", app_config.app_spec.app_id(), service_port);
                let instance = ReplicaInstance {
                    spec_id: format!(
                        "{}@{}",
                        app_config.app_spec.app_id(),
                        app_config.app_spec.user_id.clone()
                    ),
                    node_id: node_id.to_string(),
                    res_limits: HashMap::new(),
                    instance_id: app_instance_id.to_string(),
                    last_update_time: 0,
                    state: InstanceState::from(app_config.target_state.clone()),
                    service_ports: app_config.service_ports_config.clone(),
                };
                scheduler_ctx.add_replica_instance(instance);
            }
        }
        //add instance
        // services/$server_name/instances/$node_id
        let key_parts = key.split('/').collect::<Vec<&str>>();
        if key_parts.len() > 3 && key_parts[0] == "services" && key_parts[2] == "instances" {
            info!("add serviceinstance:{}", key);
            let service_name = key_parts[1];
            let instance_node_id = key_parts[3];
            let instance_info: ServiceInstanceReportInfo = serde_json::from_str(value.as_str())
                .map_err(|e| {
                    error!("ServiceInstanceInfo serde_json::from_str failed: {:?}", e);
                    e
                })?;

            let instance = ReplicaInstance {
                spec_id: service_name.to_string(),
                node_id: instance_node_id.to_string(),
                res_limits: HashMap::new(),
                instance_id: instance_info.instance_id.clone(),
                last_update_time: instance_info.last_update_time,
                state: InstanceState::from(instance_info.state.clone()),
                service_ports: instance_info.service_ports.clone(),
            };
            scheduler_ctx.add_replica_instance(instance);
        }
    }

    info!(
        "scheduler config snapshot loaded: keys={} nodes={} users={} specs={} reported_instances={}",
        input_config.len(),
        scheduler_ctx.nodes.len(),
        scheduler_ctx.users.len(),
        scheduler_ctx.specs.len(),
        scheduler_ctx.replica_instances.len()
    );

    Ok((scheduler_ctx, device_list))
}

pub(crate) fn schedule_action_to_tx_actions(
    action: &SchedulerAction,
    scheduler_ctx: &NodeScheduler,
    device_list: &HashMap<String, DeviceInfo>,
    input_config: &HashMap<String, String>,
    need_update_gateway_node_list: &mut HashSet<String>,
    need_update_rbac: &mut bool,
) -> Result<HashMap<String, KVAction>> {
    let mut result = HashMap::new();
    let zone_config = input_config.get("boot/config");
    if zone_config.is_none() {
        return Err(anyhow::anyhow!("zone_config not found"));
    }
    let zone_config = zone_config.unwrap();
    let zone_config: ZoneConfig = serde_json::from_str(zone_config.as_str())?;
    let zone_gateway = zone_config.get_default_zone_gateway();
    match action {
        SchedulerAction::ChangeNodeStatus(node_id, node_status) => {
            let key = format!("nodes/{}/config", node_id);
            let mut set_paths = HashMap::new();
            set_paths.insert("state".to_string(), Some(json!(node_status.to_string())));
            //TODO:需要将insert替换成合并
            info!("will change node status: {} -> {}", node_id, node_status);
            result.insert(key, KVAction::SetByJsonPath(set_paths));
        }
        SchedulerAction::ChangeServiceStatus(spec_id, spec_status) => {
            let service_spec = scheduler_ctx.get_service_spec(spec_id.as_str());
            if service_spec.is_none() {
                return Err(anyhow::anyhow!("service_spec not found"));
            }
            let service_spec = service_spec.unwrap();
            match service_spec.spec_type {
                ServiceSpecType::App => {
                    let set_state_action =
                        set_app_service_state(spec_id.as_str(), spec_status, input_config)?;
                    info!(
                        "will change app service status: {} -> {}",
                        spec_id, spec_status
                    );
                    result.extend(set_state_action);
                }
                ServiceSpecType::Service | ServiceSpecType::Kernel => {
                    let set_state_action = set_service_state(spec_id.as_str(), spec_status)?;
                    info!("will change service status: {} -> {}", spec_id, spec_status);
                    result.extend(set_state_action);
                }
            }
        }
        SchedulerAction::CreateOPTask(_new_op_task) => {
            //TODO:
            unimplemented!();
        }
        SchedulerAction::InstanceReplica(new_instance) => {
            //最复杂的流程,需要根据pod的类型,来执行实例化操作
            let service_spec = scheduler_ctx.get_service_spec(new_instance.spec_id.as_str());
            if service_spec.is_none() {
                return Err(anyhow::anyhow!("service_spec not found"));
            }
            let service_spec = service_spec.unwrap();
            need_update_gateway_node_list.insert(new_instance.node_id.clone());
            match service_spec.spec_type {
                ServiceSpecType::App => {
                    let instance_action =
                        instance_app_service(new_instance, &device_list, &input_config)?;
                    info!("will instance app pod: {}", new_instance.spec_id);
                    result.extend(instance_action);
                }
                ServiceSpecType::Service | ServiceSpecType::Kernel => {
                    let service_config = input_config
                        .get(format!("services/{}/spec", service_spec.id.as_str()).as_str());
                    if service_config.is_none() {
                        return Err(anyhow::anyhow!(
                            "service_config {} not found",
                            service_spec.id.as_str()
                        ));
                    }
                    let service_config = service_config.unwrap();
                    let service_config: KernelServiceSpec =
                        serde_json::from_str(service_config.as_str())?;
                    let is_zone_gateway = zone_gateway
                        .as_ref()
                        .map(|gw| gw == &new_instance.node_id)
                        .unwrap_or(false);
                    let instance_action =
                        instance_service(new_instance, &service_config, is_zone_gateway)?;
                    info!("will instance service pod: {}", new_instance.spec_id);
                    result.extend(instance_action);
                }
            }
        }
        SchedulerAction::RemoveInstance(spec_id, instance_id, node_id) => {
            let service_spec = scheduler_ctx.get_service_spec(spec_id.as_str());
            if service_spec.is_none() {
                return Err(anyhow::anyhow!("service_spec not found"));
            }
            let service_spec = service_spec.unwrap();
            need_update_gateway_node_list.insert(node_id.clone());
            let instance = scheduler_ctx
                .get_replica_instance(instance_id.as_str())
                .cloned()
                .unwrap_or_else(|| {
                    warn!(
                        "remove instance {} missing from scheduler snapshot, using action payload",
                        instance_id
                    );
                    ReplicaInstance {
                        spec_id: spec_id.clone(),
                        instance_id: instance_id.clone(),
                        node_id: node_id.clone(),
                        res_limits: HashMap::new(),
                        last_update_time: 0,
                        state: InstanceState::Deleted,
                        service_ports: HashMap::new(),
                    }
                });
            match service_spec.spec_type {
                ServiceSpecType::App => {
                    info!("will uninstance app service: {}", instance.spec_id);
                    let uninstance_action = uninstance_app_service(&instance)?;
                    result.extend(uninstance_action);
                }
                ServiceSpecType::Service | ServiceSpecType::Kernel => {
                    info!("will uninstance service: {}", instance.spec_id);
                    let uninstance_action = uninstance_service(&instance)?;
                    result.extend(uninstance_action);
                }
            }
        }
        SchedulerAction::UpdateInstance(instance_id, instance) => {
            //相对比较复杂的操作:需要根据service_spec的类型,来执行更新实例化操作
            let (app_id, owner_id, _node_id) = parse_instance_id(instance_id.as_str())?;
            let spec_id = format!("{}@{}", app_id, owner_id);
            let service_spec_opt = scheduler_ctx.get_service_spec(spec_id.as_str());
            if service_spec_opt.is_none() {
                return Err(anyhow::anyhow!("service_spec not found"));
            }
            let service_spec = service_spec_opt.unwrap();
            match service_spec.spec_type {
                ServiceSpecType::App => {
                    let update_action = update_app_service_instance(instance)?;
                    info!("will update app service instance: {}", instance.spec_id);
                    result.extend(update_action);
                }
                ServiceSpecType::Service | ServiceSpecType::Kernel => {
                    let update_action = update_service_instance(instance)?;
                    info!("will update service instance: {}", instance.spec_id);
                    result.extend(update_action);
                }
            }
        }
        SchedulerAction::UpdateServiceInfo(spec_id, service_info) => {
            if should_skip_app_service_info_deletion(spec_id.as_str(), service_info, input_config)?
            {
                info!(
                    "skip deleting service info for legacy docker app-service: {}",
                    spec_id
                );
                return Ok(result);
            }
            let update_action =
                update_service_info(spec_id.as_str(), service_info, device_list, &input_config)?;
            info!("will update service info: {}", spec_id);
            result.extend(update_action);
        }
    }
    Ok(result)
}

pub fn get_spec_id_from_service_info_id(service_info_id: &str) -> (String, String) {
    let parts = service_info_id.split(":").collect::<Vec<&str>>();
    if parts.len() < 2 {
        return (service_info_id.to_string(), "www".to_string());
    }
    if parts.len() == 2 {
        return (parts[0].to_string(), parts[1].to_string());
    }
    warn!("invalid service_info_id: {}", service_info_id);
    return (parts[0].to_string(), parts[1].to_string());
}

pub fn get_service_spec_by_spec_id(
    spec_id: &str,
    input_system_config: &HashMap<String, String>,
) -> Result<ServiceSpec> {
    let key = format!("services/{}/spec", spec_id);
    let service_spec = input_system_config.get(&key);
    if service_spec.is_none() {
        return Err(anyhow::anyhow!("service_spec not found"));
    }
    let service_spec = service_spec.unwrap();
    let service_spec: ServiceSpec = serde_json::from_str(service_spec.as_str())?;
    Ok(service_spec)
}

pub fn get_appid_and_userid_from_spec_id(spec_id: &str) -> Result<(String, String)> {
    let parts = spec_id.split("@").collect::<Vec<&str>>();
    if parts.len() < 2 {
        return Err(anyhow::anyhow!("invalid spec_id: {}", spec_id));
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

pub fn get_app_spec_by_spec_id(
    spec_id: &str,
    input_system_config: &HashMap<String, String>,
) -> Result<AppServiceSpec> {
    let (app_id, user_id) = get_appid_and_userid_from_spec_id(spec_id)?;
    for key in [
        format!("users/{}/apps/{}/spec", user_id, app_id),
        format!("users/{}/agents/{}/spec", user_id, app_id),
    ] {
        if let Some(app_spec) = input_system_config.get(&key) {
            let app_spec: AppServiceSpec = serde_json::from_str(app_spec.as_str())?;
            return Ok(app_spec);
        }
    }
    warn!(
        "app_spec not found, tried users/{}/apps/{}/spec and users/{}/agents/{}/spec",
        user_id, app_id, user_id, app_id
    );
    Err(anyhow::anyhow!("app_spec not found"))
}

fn should_skip_app_service_info_deletion(
    spec_id: &str,
    service_info: &ServiceInfo,
    input_system_config: &HashMap<String, String>,
) -> Result<bool> {
    if !spec_id.contains('@') {
        return Ok(false);
    }

    let app_spec = get_app_spec_by_spec_id(spec_id, input_system_config)?;
    let is_service_info_empty = match service_info {
        ServiceInfo::SingleInstance(_) => false,
        ServiceInfo::RandomCluster(cluster) => cluster.is_empty(),
    };

    Ok(is_legacy_docker_app_service(&app_spec) && is_service_info_empty)
}

pub fn get_zone_config(input_system_config: &HashMap<String, String>) -> Result<ZoneConfig> {
    let key = "boot/config";
    let zone_config = input_system_config.get(key);
    if zone_config.is_none() {
        return Err(anyhow::anyhow!("zone_config not found"));
    }
    let zone_config = zone_config.unwrap();
    let zone_config: ZoneConfig = serde_json::from_str(zone_config.as_str()).map_err(|e| {
        error!("ZoneConfig::from_str failed: {:?}", e);
        e
    })?;
    Ok(zone_config)
}

pub fn get_zone_gateway_settings(
    input_system_config: &HashMap<String, String>,
) -> Result<ZoneGatewaySettings> {
    let key = "services/gateway/settings";
    let zone_gateway_settings = input_system_config.get(key);
    if zone_gateway_settings.is_none() {
        warn!("zone_gateway_settings not found, use default");
        return Ok(ZoneGatewaySettings::default());
    }
    let zone_gateway_settings = zone_gateway_settings.unwrap();
    info!("zone_gateway_settings: {}", zone_gateway_settings);
    let zone_gateway_settings: ZoneGatewaySettings =
        serde_json::from_str(zone_gateway_settings.as_str()).map_err(|e| {
            error!("serde_json::from_str failed: {:?}", e);
            e
        })?;
    Ok(zone_gateway_settings)
}

pub fn get_web_app_list(
    input_system_config: &HashMap<String, String>,
) -> Result<Vec<AppServiceSpec>> {
    let mut web_app_list: Vec<AppServiceSpec> = Vec::new();
    for (key, value) in input_system_config.iter() {
        if key.starts_with("users/") && key.ends_with("/spec") {
            let parts: Vec<&str> = key.split('/').collect();
            if parts.len() >= 4 && parts[2] == "apps" {
                let user_id = parts[1];
                let app_id = parts[3];
                let full_appid = format!("{}@{}", app_id, user_id);
                let app_config: AppServiceSpec =
                    serde_json::from_str(value.as_str()).map_err(|e| {
                        error!(
                            "AppConfig serde_json::from_str failed: {:?} {}",
                            e,
                            value.as_str()
                        );
                        e
                    })?;
                let is_web_app = app_config.app_doc.selector_type == SelectorType::Static;
                let is_gateway_visible = app_config.enable
                    && !matches!(
                        app_config.state,
                        ServiceState::Deleted | ServiceState::Stopped | ServiceState::Stopping
                    );
                if is_web_app && is_gateway_visible {
                    info!("found web app: {}", full_appid);
                    web_app_list.push(app_config);
                }
            }
        }
    }
    Ok(web_app_list)
}

fn build_web_app_servers(
    input_system_config: &HashMap<String, String>,
) -> Result<HashMap<String, Value>> {
    let mut web_app_servers = HashMap::new();

    for web_app in get_web_app_list(input_system_config)? {
        if !web_app.install_config.expose_config.contains_key("www") {
            continue;
        }

        let Some(web_pkg) = web_app.app_doc.pkg_list.web.as_ref() else {
            continue;
        };

        let web_pkg_id = PackageId::get_pkg_id_unique_name(web_pkg.pkg_id.as_str());
        web_app_servers.insert(
            web_pkg_id.clone(),
            json!({
                "type": "dir",
                "root_path": format!("../bin/{}/", web_pkg_id),
            }),
        );
    }

    Ok(web_app_servers)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum NodeGatewayAccessMode {
    Public,
    Private,
}

impl Default for NodeGatewayAccessMode {
    fn default() -> Self {
        Self::Private
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct NodeGatewayNodeInfo {
    this_node_id: String,
    this_zone_host: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct NodeGatewaySelectorTarget {
    port: u16,
    weight: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct NodeGatewayServiceInfoEntry {
    selector: HashMap<String, NodeGatewaySelectorTarget>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct NodeGatewayAppInfoEntry {
    app_id: String,
    sdk_version: u32,
    access_mode: NodeGatewayAccessMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    node_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dir_pkg_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dir_pkg_objid: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    block_services: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct NodeGatewayAppServiceInfoEntry {
    service_id: String,
    selector: HashMap<String, NodeGatewaySelectorTarget>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
enum NodeGatewayAppEntry {
    App(NodeGatewayAppInfoEntry),
    Service(NodeGatewayAppServiceInfoEntry),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct NodeGatewayInfo {
    node_info: NodeGatewayNodeInfo,
    app_info: HashMap<String, NodeGatewayAppEntry>,
    service_info: HashMap<String, NodeGatewayServiceInfoEntry>,
    node_route_map: HashMap<String, String>,
    trust_key: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    cluster_route_map: HashMap<String, NodeGatewayClusterRouteEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct NodeGatewayClusterRouteEntry {
    route_prefix: String,
    ingress_port: u16,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    nodes: HashMap<String, NodeGatewayClusterRouteNodeEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct NodeGatewayClusterRouteNodeEntry {
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    ports: HashMap<String, u16>,
}

const DEFAULT_NODE_GATEWAY_HTTP_PORT: u16 = 3180;

fn get_device_list(
    input_system_config: &HashMap<String, String>,
) -> Result<HashMap<String, DeviceInfo>> {
    let mut device_list = HashMap::new();
    for (key, value) in input_system_config.iter() {
        if key.starts_with("devices/") && key.ends_with("/info") {
            let node_id = key.split('/').nth(1).unwrap_or_default();
            let device_info: DeviceInfo = serde_json::from_str(value).map_err(|e| {
                error!("DeviceInfo serde_json::from_str failed: {:?}", e);
                e
            })?;
            device_list.insert(node_id.to_string(), device_info);
        }
    }
    Ok(device_list)
}

fn select_gateway_port(service_ports: &HashMap<String, u16>, service_name: &str) -> Option<u16> {
    if let Some(port) = service_ports.get(service_name) {
        return Some(*port);
    }

    for fallback_name in ["www", "http", "https", "main"] {
        if let Some(port) = service_ports.get(fallback_name) {
            return Some(*port);
        }
    }

    if service_ports.len() == 1 {
        return service_ports.values().next().copied();
    }

    let mut ports = service_ports.iter().collect::<Vec<_>>();
    ports.sort_by(|left, right| left.0.cmp(right.0));
    ports.first().map(|(_, port)| **port)
}

fn build_service_selector(
    service_info: &ServiceInfo,
    service_name: &str,
) -> Option<HashMap<String, NodeGatewaySelectorTarget>> {
    let mut selector = HashMap::new();

    match service_info {
        ServiceInfo::SingleInstance(instance) => {
            if let Some(port) = select_gateway_port(&instance.service_ports, service_name) {
                selector.insert(
                    instance.node_id.clone(),
                    NodeGatewaySelectorTarget {
                        port,
                        weight: FIXED_SERVICE_WEIGHT,
                    },
                );
            }
        }
        ServiceInfo::RandomCluster(cluster) => {
            for (_, (weight, instance)) in cluster.iter() {
                if let Some(port) = select_gateway_port(&instance.service_ports, service_name) {
                    selector.insert(
                        instance.node_id.clone(),
                        NodeGatewaySelectorTarget {
                            port,
                            weight: *weight,
                        },
                    );
                }
            }
        }
    }

    if selector.is_empty() {
        None
    } else {
        Some(selector)
    }
}

fn parse_sdk_version(app_spec: &AppServiceSpec) -> u32 {
    app_spec
        .app_doc
        .sdk_version
        .as_deref()
        .and_then(|version| {
            version
                .split(['.', '-'])
                .next()
                .and_then(|major| major.parse::<u32>().ok())
        })
        .unwrap_or(0)
}

fn is_legacy_docker_app_service(app_spec: &AppServiceSpec) -> bool {
    app_spec.app_doc.selector_type != SelectorType::Static
        && app_spec.app_doc.get_app_type() != AppType::Agent
        && app_spec.app_doc.sdk_version.is_none()
}

fn build_app_host_entry(
    app_spec: &AppServiceSpec,
    service_info: &ServiceInfo,
    service_name: &str,
) -> Option<NodeGatewayAppInfoEntry> {
    let pick_instance = match service_info {
        ServiceInfo::SingleInstance(instance) => Some(instance),
        ServiceInfo::RandomCluster(cluster) => cluster
            .values()
            .map(|(_, instance)| instance)
            .filter(|instance| select_gateway_port(&instance.service_ports, service_name).is_some())
            .min_by(|left, right| left.instance_id.cmp(&right.instance_id)),
    }?;

    let port = select_gateway_port(&pick_instance.service_ports, service_name)?;
    let access_mode = if app_spec.install_config.allow_public_access {
        NodeGatewayAccessMode::Public
    } else {
        NodeGatewayAccessMode::Private
    };
    Some(NodeGatewayAppInfoEntry {
        app_id: app_spec.app_id().to_string(),
        sdk_version: parse_sdk_version(app_spec),
        access_mode,
        node_id: Some(pick_instance.node_id.clone()),
        port: Some(port),
        dir_pkg_id: None,
        dir_pkg_objid: None,
        block_services: vec![],
    })
}

fn load_persisted_service_info(
    spec_id: &str,
    input_system_config: &HashMap<String, String>,
) -> Option<buckyos_api::ServiceInfo> {
    let key = format!("services/{}/info", spec_id);
    input_system_config
        .get(&key)
        .and_then(|raw| serde_json::from_str(raw).ok())
}

fn build_app_host_entry_from_persisted_service_info(
    app_spec: &AppServiceSpec,
    service_info: &buckyos_api::ServiceInfo,
    service_name: &str,
) -> Option<NodeGatewayAppInfoEntry> {
    let (node_id, node_info) = service_info
        .node_list
        .iter()
        .filter(|(_, node)| node.state == buckyos_api::ServiceInstanceState::Started)
        .filter(|(_, node)| select_gateway_port(&node.service_port, service_name).is_some())
        .min_by(|left, right| left.0.cmp(right.0))?;

    let port = select_gateway_port(&node_info.service_port, service_name)?;
    let access_mode = if app_spec.install_config.allow_public_access {
        NodeGatewayAccessMode::Public
    } else {
        NodeGatewayAccessMode::Private
    };
    Some(NodeGatewayAppInfoEntry {
        app_id: app_spec.app_id().to_string(),
        sdk_version: parse_sdk_version(app_spec),
        access_mode,
        node_id: Some(node_id.clone()),
        port: Some(port),
        dir_pkg_id: None,
        dir_pkg_objid: None,
        block_services: vec![],
    })
}

fn build_static_web_app_host_entry(app_spec: &AppServiceSpec) -> Option<NodeGatewayAppInfoEntry> {
    let web_pkg = app_spec.app_doc.pkg_list.web.as_ref()?;
    let dir_pkg_id = PackageId::get_pkg_id_unique_name(web_pkg.pkg_id.as_str());
    let dir_pkg_objid = web_pkg.pkg_objid.as_ref().map(|objid| objid.to_string());

    Some(NodeGatewayAppInfoEntry {
        app_id: app_spec.app_id().to_string(),
        sdk_version: parse_sdk_version(app_spec),
        access_mode: NodeGatewayAccessMode::Public,
        node_id: None,
        port: None,
        dir_pkg_id: Some(dir_pkg_id),
        dir_pkg_objid,
        block_services: vec![],
    })
}

fn build_node_route_map(
    this_node_id: &str,
    zone_host: &str,
    device_list: &HashMap<String, DeviceInfo>,
) -> HashMap<String, String> {
    let mut node_route_map = HashMap::new();

    for (node_id, device_info) in device_list.iter() {
        if node_id == this_node_id {
            continue;
        }

        let route = match device_info.device_doc.rtcp_port {
            Some(port) if port != 2980 => format!("rtcp://{}.{}:{}/", node_id, zone_host, port),
            _ => format!("rtcp://{}.{}/", node_id, zone_host),
        };
        node_route_map.insert(node_id.clone(), route);
    }

    node_route_map
}

fn insert_trust_key(
    trust_key: &mut HashMap<String, String>,
    key_id: &str,
    jwk: &jsonwebtoken::jwk::Jwk,
) {
    match get_x_from_jwk(jwk) {
        Ok(x) => {
            trust_key.insert(key_id.to_string(), x);
        }
        Err(err) => {
            warn!("parse trust key {} failed: {:?}", key_id, err);
        }
    }
}

fn build_trust_keys(
    node_id: &str,
    zone_config: &ZoneConfig,
    device_list: &HashMap<String, DeviceInfo>,
) -> HashMap<String, String> {
    let mut trust_key = HashMap::new();

    if let Some(verify_hub_info) = zone_config.verify_hub_info.as_ref() {
        insert_trust_key(&mut trust_key, "verify-hub", &verify_hub_info.public_key);
    }

    if let Some(owner_key) = zone_config.get_default_key() {
        insert_trust_key(&mut trust_key, "root", &owner_key);
        insert_trust_key(&mut trust_key, "$default", &owner_key);
        insert_trust_key(
            &mut trust_key,
            zone_config.owner.to_string().as_str(),
            &owner_key,
        );
        insert_trust_key(&mut trust_key, zone_config.owner.id.as_str(), &owner_key);
    }

    if let Some(device_info) = device_list.get(node_id) {
        if let Some(node_key) = device_info.get_default_key() {
            insert_trust_key(&mut trust_key, node_id, &node_key);
        }
    }

    trust_key
}

fn build_fixed_selector_from_oods(
    zone_config: &ZoneConfig,
    port: u16,
) -> HashMap<String, NodeGatewaySelectorTarget> {
    let mut selector = HashMap::new();
    for ood in zone_config.oods.iter() {
        selector.insert(
            ood.name.clone(),
            NodeGatewaySelectorTarget {
                port,
                weight: FIXED_SERVICE_WEIGHT,
            },
        );
    }
    selector
}

fn extract_klog_cluster_route_prefix(input_system_config: &HashMap<String, String>) -> String {
    let Some(raw_settings) = input_system_config.get("services/klog-service/settings") else {
        return DEFAULT_KLOG_CLUSTER_ROUTE_PREFIX.to_string();
    };

    let Ok(settings) = serde_json::from_str::<Value>(raw_settings) else {
        warn!(
            "parse services/klog-service/settings failed while building klog cluster gateway route prefix"
        );
        return DEFAULT_KLOG_CLUSTER_ROUTE_PREFIX.to_string();
    };

    settings
        .get("cluster_network")
        .and_then(|v| v.get("gateway_route_prefix"))
        .and_then(Value::as_str)
        .map(normalize_klog_cluster_route_prefix)
        .unwrap_or_else(|| DEFAULT_KLOG_CLUSTER_ROUTE_PREFIX.to_string())
}

fn build_klog_cluster_route_entry(
    scheduler_ctx: &NodeScheduler,
    input_system_config: &HashMap<String, String>,
) -> Option<NodeGatewayClusterRouteEntry> {
    let mut node_ports: HashMap<String, HashMap<String, u16>> = HashMap::new();

    for (service_info_id, service_info) in scheduler_ctx.service_infos.iter() {
        let (spec_id, service_name) = get_spec_id_from_service_info_id(service_info_id);
        if spec_id != KLOG_SERVICE_UNIQUE_ID {
            continue;
        }

        let plane_name = match service_name.as_str() {
            KLOG_CLUSTER_RAFT_SERVICE_NAME => "raft",
            KLOG_CLUSTER_INTER_SERVICE_NAME => "inter",
            KLOG_CLUSTER_ADMIN_SERVICE_NAME => "admin",
            _ => continue,
        };

        let Some(selector) = build_service_selector(service_info, service_name.as_str()) else {
            continue;
        };

        for (node_name, target) in selector {
            node_ports
                .entry(node_name)
                .or_default()
                .insert(plane_name.to_string(), target.port);
        }
    }

    let mut nodes = HashMap::new();
    for (node_name, ports) in node_ports {
        if !ports.contains_key("raft") {
            warn!(
                "skip klog cluster gateway entry for node {} because raft port is missing",
                node_name
            );
            continue;
        }
        if !ports.contains_key("inter") {
            warn!(
                "skip klog cluster gateway entry for node {} because inter port is missing",
                node_name
            );
            continue;
        }
        if !ports.contains_key("admin") {
            warn!(
                "skip klog cluster gateway entry for node {} because admin port is missing",
                node_name
            );
            continue;
        }

        nodes.insert(
            node_name.clone(),
            NodeGatewayClusterRouteNodeEntry { ports },
        );
    }

    if nodes.is_empty() {
        return None;
    }

    Some(NodeGatewayClusterRouteEntry {
        route_prefix: extract_klog_cluster_route_prefix(input_system_config),
        ingress_port: DEFAULT_NODE_GATEWAY_HTTP_PORT,
        nodes,
    })
}

pub(crate) async fn update_node_gateway_info(
    node_id: &str,
    scheduler_ctx: &NodeScheduler,
    input_system_config: &HashMap<String, String>,
) -> Result<HashMap<String, KVAction>> {
    let zone_config = get_zone_config(input_system_config)?;
    let zone_gateway_settings = get_zone_gateway_settings(input_system_config)?;
    let device_list = get_device_list(input_system_config)?;
    let zone_host = zone_config.id.to_host_name();
    let mut cluster_route_map = HashMap::new();
    if let Some(klog_cluster_route) =
        build_klog_cluster_route_entry(scheduler_ctx, input_system_config)
    {
        cluster_route_map.insert(KLOG_SERVICE_UNIQUE_ID.to_string(), klog_cluster_route);
    }

    let mut node_gateway_info = NodeGatewayInfo {
        node_info: NodeGatewayNodeInfo {
            this_node_id: node_id.to_string(),
            this_zone_host: zone_host.clone(),
        },
        app_info: HashMap::new(),
        service_info: HashMap::new(),
        node_route_map: build_node_route_map(node_id, &zone_host, &device_list),
        trust_key: build_trust_keys(node_id, &zone_config, &device_list),
        cluster_route_map,
    };

    for (service_info_id, service_info) in scheduler_ctx.service_infos.iter() {
        let (spec_id, service_name) = get_spec_id_from_service_info_id(service_info_id);
        let selector = build_service_selector(service_info, service_name.as_str());

        if let Some(selector) = selector.as_ref() {
            if !spec_id.contains('@') {
                node_gateway_info.service_info.insert(
                    spec_id.clone(),
                    NodeGatewayServiceInfoEntry {
                        selector: selector.clone(),
                    },
                );
            }
        }

        if service_name == "www" {
            if spec_id.contains('@') {
                if let Ok(app_spec) = get_app_spec_by_spec_id(spec_id.as_str(), input_system_config)
                {
                    if let Some(expose_config) = app_spec.install_config.expose_config.get("www") {
                        let app_entry =
                            build_app_host_entry(&app_spec, service_info, service_name.as_str())
                                .or_else(|| {
                                    if !is_legacy_docker_app_service(&app_spec) {
                                        return None;
                                    }
                                    let persisted_service_info = load_persisted_service_info(
                                        spec_id.as_str(),
                                        input_system_config,
                                    )?;
                                    build_app_host_entry_from_persisted_service_info(
                                        &app_spec,
                                        &persisted_service_info,
                                        service_name.as_str(),
                                    )
                                });

                        if let Some(app_entry) = app_entry {
                            for host in zone_gateway_settings.get_shortcut(spec_id.as_str()) {
                                node_gateway_info
                                    .app_info
                                    .insert(host, NodeGatewayAppEntry::App(app_entry.clone()));
                            }
                            for host in expose_config.sub_hostname.iter() {
                                node_gateway_info.app_info.insert(
                                    host.clone(),
                                    NodeGatewayAppEntry::App(app_entry.clone()),
                                );
                            }
                        }
                    }
                }
            } else if spec_id == "control-panel" {
                if let Some(selector) = selector.as_ref() {
                    let service_entry =
                        NodeGatewayAppEntry::Service(NodeGatewayAppServiceInfoEntry {
                            service_id: spec_id.clone(),
                            selector: selector.clone(),
                        });
                    for host in ["_", "www", "sys"] {
                        node_gateway_info
                            .app_info
                            .entry(host.to_string())
                            .or_insert_with(|| service_entry.clone());
                    }
                }
            }
        }
    }

    for web_app in get_web_app_list(input_system_config)? {
        let full_app_id = format!("{}@{}", web_app.app_id(), web_app.user_id);
        let Some(expose_config) = web_app.install_config.expose_config.get("www") else {
            continue;
        };
        let Some(app_entry) = build_static_web_app_host_entry(&web_app) else {
            continue;
        };

        for host in zone_gateway_settings.get_shortcut(full_app_id.as_str()) {
            node_gateway_info
                .app_info
                .insert(host, NodeGatewayAppEntry::App(app_entry.clone()));
        }
        for host in expose_config.sub_hostname.iter() {
            node_gateway_info
                .app_info
                .insert(host.clone(), NodeGatewayAppEntry::App(app_entry.clone()));
        }
    }

    let system_config_selector =
        build_fixed_selector_from_oods(&zone_config, SYSTEM_CONFIG_SERVICE_PORT);
    if !system_config_selector.is_empty() {
        node_gateway_info.service_info.insert(
            "system_config".to_string(),
            NodeGatewayServiceInfoEntry {
                selector: system_config_selector,
            },
        );
    }

    let control_panel_selector = node_gateway_info
        .service_info
        .get("control-panel")
        .map(|entry| entry.selector.clone())
        .filter(|selector| !selector.is_empty())
        .unwrap_or_else(|| {
            build_fixed_selector_from_oods(&zone_config, CONTROL_PANEL_SERVICE_PORT)
        });
    if !control_panel_selector.is_empty() {
        node_gateway_info.service_info.insert(
            "control-panel".to_string(),
            NodeGatewayServiceInfoEntry {
                selector: control_panel_selector.clone(),
            },
        );

        let control_panel_entry = NodeGatewayAppEntry::Service(NodeGatewayAppServiceInfoEntry {
            service_id: "control-panel".to_string(),
            selector: control_panel_selector,
        });
        node_gateway_info
            .app_info
            .insert("sys".to_string(), control_panel_entry.clone());
        node_gateway_info
            .app_info
            .entry("_".to_string())
            .or_insert_with(|| control_panel_entry.clone());
        node_gateway_info
            .app_info
            .entry("www".to_string())
            .or_insert(control_panel_entry);
    }

    let key = format!("nodes/{}/gateway_info", node_id);
    let value = serde_json::to_string_pretty(&node_gateway_info)?;
    info!("will update node {} gateway info: {}", node_id, value);

    let mut result = HashMap::new();
    result.insert(key, KVAction::Update(value));
    Ok(result)
}

pub(crate) async fn update_node_gateway_infos(
    need_update_gateway_node_list: &HashSet<String>,
    scheduler_ctx: &NodeScheduler,
    input_system_config: &HashMap<String, String>,
) -> Result<HashMap<String, KVAction>> {
    let mut result = HashMap::new();
    for node_id in need_update_gateway_node_list.iter() {
        let actions = update_node_gateway_info(node_id, scheduler_ctx, input_system_config).await?;
        extend_kv_action_map(&mut result, &actions);
    }

    Ok(result)
}

pub(crate) async fn update_node_gateway_config(
    need_update_gateway_node_list: &HashSet<String>,
    input_system_config: &HashMap<String, String>,
) -> Result<HashMap<String, KVAction>> {
    let zone_config = get_zone_config(input_system_config)?;
    let web_app_servers = build_web_app_servers(input_system_config)?;
    let mut result = HashMap::new();

    for node_id in need_update_gateway_node_list.iter() {
        let mut node_gateway_json = json!({});

        if let Some(sn_host) = zone_config.sn.as_ref() {
            info!("SN enabled, add acme/tls stack for node {}", node_id);
            let sn_url = format!("https://{}/kapi/sn", sn_host);
            let zone_hostname = zone_config.id.to_host_name();
            let wildcard_zone_domain = format!("*.{}", zone_hostname);
            node_gateway_json = json!({
                "acme": {
                    "dns_providers": {
                        "sn-dns": {
                            "sn": sn_url,
                            "key_path": "./node_private_key.pem",
                            "device_config_path": "./node_device_config.json"
                        }
                    }
                },
                "stacks": {
                    "zone_tls": {
                        "bind": "0.0.0.0:443",
                        "protocol": "tls",
                        "certs": [
                            {
                                "domain": wildcard_zone_domain,
                                "acme_type": "dns-01",
                                "dns_provider": "sn-dns"
                            },
                            {
                                "domain": zone_hostname
                            },
                            {
                                "domain": "*"
                            }
                        ],
                        "hook_point": {
                            "main": {
                                "blocks": {
                                    "default": {
                                        "block": "return \"server node_gateway\";\n"
                                    }
                                }
                            }
                        }
                    }
                }
            });
        }

        if !web_app_servers.is_empty() {
            node_gateway_json["servers"] = json!({});
            for (server_name, server_config) in web_app_servers.iter() {
                node_gateway_json["servers"][server_name] = server_config.clone();
            }
        }

        let node_gateway_config_str = serde_json::to_string_pretty(&node_gateway_json)?;
        info!(
            "will update node {} gateway config: {}",
            node_id, node_gateway_config_str
        );
        let key = format!("nodes/{}/gateway_config", node_id);
        result.insert(key, KVAction::Update(node_gateway_config_str));
    }

    Ok(result)
}

async fn update_rbac(
    input_config: &HashMap<String, String>,
    scheduler_ctx: &NodeScheduler,
) -> Result<HashMap<String, KVAction>> {
    let basic_rbac_policy = input_config.get("system/rbac/base_policy");
    let current_rbac_policy = input_config.get("system/rbac/policy");
    let mut rbac_policy = String::new();
    if basic_rbac_policy.is_none() {
        warn!("basic_rbac_policy is not set, use default policy");
        rbac_policy = DEFAULT_POLICY.to_string();
    } else {
        rbac_policy = basic_rbac_policy.unwrap().clone();
    }

    for (user_id, user_item) in scheduler_ctx.users.iter() {
        if user_id == "root" {
            continue;
        }
        match user_item.user_type {
            crate::scheduler::UserType::Admin => {
                rbac_policy.push_str(&format!("\ng, {}, admin", user_id));
            }
            crate::scheduler::UserType::User => {
                rbac_policy.push_str(&format!("\ng, {}, user", user_id));
            }
            crate::scheduler::UserType::Limited => {
                rbac_policy.push_str(&format!("\ng, {}, limited", user_id));
            }
            _ => {
                continue;
            }
        }
    }

    for (node_id, node_item) in scheduler_ctx.nodes.iter() {
        match node_item.node_type {
            NodeType::OOD => {
                rbac_policy.push_str(&format!("\ng, {}, ood", node_id));
            }
            NodeType::Server => {
                rbac_policy.push_str(&format!("\ng, {}, server", node_id));
            }
            _ => {
                continue;
            }
        }
    }

    for (spec_id, service_spec) in scheduler_ctx.specs.iter() {
        match service_spec.spec_type {
            ServiceSpecType::App => {
                rbac_policy.push_str(&format!("\ng, {}, app", service_spec.app_id));
            }
            ServiceSpecType::Service => {
                rbac_policy.push_str(&format!("\ng, {}, service", service_spec.app_id));
            }
            ServiceSpecType::Kernel => {
                //kernel service already set in basic_policy
                rbac_policy.push_str(&format!("\ng, {}, kernel", service_spec.app_id));
            }
        }
    }

    let mut result = HashMap::new();
    if current_rbac_policy.is_some() {
        let current_rbac_policy = current_rbac_policy.unwrap();
        if *current_rbac_policy == rbac_policy {
            return Ok(HashMap::new());
        }
    }

    info!("will update system/rbac/policy => {}", &rbac_policy);
    result.insert(
        "system/rbac/policy".to_string(),
        KVAction::Update(rbac_policy),
    );

    Ok(result)
}

pub(crate) struct SchedulePlan {
    pub tx_actions: HashMap<String, KVAction>,
    pub schedule_snapshot: NodeScheduler,
    pub need_persist_snapshot: bool,
}

fn collect_tx_action_keys(tx_actions: &HashMap<String, KVAction>) -> Vec<String> {
    let mut keys = tx_actions.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    keys
}

pub(crate) async fn build_schedule_plan(
    input_system_config: &HashMap<String, String>,
    is_boot: bool,
) -> Result<SchedulePlan> {
    let (mut scheduler_ctx, device_list) = create_scheduler_by_system_config(input_system_config)?;
    let last_schedule_snapshot =
        if let Some(snapshot_str) = input_system_config.get("system/scheduler/snapshot") {
            Some(serde_json::from_str::<NodeScheduler>(
                snapshot_str.as_str(),
            )?)
        } else {
            None
        };
    info!(
        "build_schedule_plan: is_boot={} nodes={} specs={} instances={} last_snapshot_present={}",
        is_boot,
        scheduler_ctx.nodes.len(),
        scheduler_ctx.specs.len(),
        scheduler_ctx.replica_instances.len(),
        last_schedule_snapshot.is_some()
    );

    let action_list = scheduler_ctx.schedule(last_schedule_snapshot.as_ref());
    if action_list.is_err() {
        error!(
            "scheduler.schedule failed: {:?}",
            action_list.as_ref().err().unwrap()
        );
        return Err(anyhow::anyhow!("scheduler.schedule failed"));
    }
    let action_list = action_list.unwrap();
    info!("scheduler.schedule produced {} actions", action_list.len());

    let mut tx_actions = HashMap::new();
    let mut need_update_gateway_node_list: HashSet<String> = HashSet::new();
    let mut need_update_rbac = false;
    for action in action_list {
        let new_tx_actions = schedule_action_to_tx_actions(
            &action,
            &scheduler_ctx,
            &device_list,
            input_system_config,
            &mut need_update_gateway_node_list,
            &mut need_update_rbac,
        )?;
        extend_kv_action_map(&mut tx_actions, &new_tx_actions);
    }

    if is_boot || last_schedule_snapshot.is_none() {
        need_update_rbac = true;
    }

    if let Some(last_schedule_snapshot) = last_schedule_snapshot.as_ref() {
        if scheduler_ctx.nodes != last_schedule_snapshot.nodes {
            need_update_rbac = true;
            need_update_gateway_node_list = scheduler_ctx.nodes.keys().cloned().collect();
        } else if scheduler_ctx.specs != last_schedule_snapshot.specs
            || scheduler_ctx.users != last_schedule_snapshot.users
        {
            need_update_rbac = true;
        }
    }

    if need_update_rbac {
        let rbac_actions = update_rbac(input_system_config, &scheduler_ctx).await?;
        extend_kv_action_map(&mut tx_actions, &rbac_actions);
    }

    if !need_update_gateway_node_list.is_empty() {
        let update_gateway_node_list_actions = update_node_gateway_infos(
            &need_update_gateway_node_list,
            &scheduler_ctx,
            input_system_config,
        )
        .await?;
        extend_kv_action_map(&mut tx_actions, &update_gateway_node_list_actions);

        let update_gateway_config_actions =
            update_node_gateway_config(&need_update_gateway_node_list, input_system_config).await?;
        extend_kv_action_map(&mut tx_actions, &update_gateway_config_actions);
    }

    let need_persist_snapshot =
        scheduler_ctx.needs_snapshot_persist(last_schedule_snapshot.as_ref());

    info!(
        "build_schedule_plan result: tx_actions={} need_update_rbac={} gateway_nodes={} need_persist_snapshot={}",
        tx_actions.len(),
        need_update_rbac,
        need_update_gateway_node_list.len(),
        need_persist_snapshot
    );

    Ok(SchedulePlan {
        tx_actions,
        schedule_snapshot: scheduler_ctx,
        need_persist_snapshot,
    })
}

pub async fn schedule_loop(is_boot: bool) -> Result<()> {
    let mut loop_step = 0;
    let is_running = true;

    //info!("schedule loop start...");
    loop {
        if !is_running {
            break;
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        loop_step += 1;
        info!("schedule loop step:{}.", loop_step);
        let buckyos_api_runtime = get_buckyos_api_runtime().unwrap();
        let system_config_client = buckyos_api_runtime
            .get_system_config_client()
            .await
            .map_err(|e| {
                error!("get_system_config_client failed: {:?}", e);
                e
            })?;
        let input_system_config = system_config_client.dump_configs_for_scheduler().await;
        if input_system_config.is_err() {
            error!(
                "dump_configs_for_scheduler failed: {:?}",
                input_system_config.err().unwrap()
            );
            continue;
        }
        let input_system_config = input_system_config.unwrap();
        //cover value to hashmap
        let input_system_config: Result<HashMap<String, String>, _> =
            serde_json::from_value(input_system_config);
        if input_system_config.is_err() {
            error!(
                "serde_json::from_value failed: {:?}",
                input_system_config.err().unwrap()
            );
            continue;
        }
        let input_system_config: HashMap<String, String> = input_system_config.unwrap();
        info!(
            "schedule loop step:{} loaded {} config entries",
            loop_step,
            input_system_config.len()
        );

        let schedule_plan = match build_schedule_plan(&input_system_config, is_boot).await {
            Ok(plan) => plan,
            Err(err) => {
                error!(
                    "build_schedule_plan failed at step {}: {:?}",
                    loop_step, err
                );
                continue;
            }
        };
        let tx_action_count = schedule_plan.tx_actions.len();
        let tx_action_keys = collect_tx_action_keys(&schedule_plan.tx_actions);
        if tx_action_count == 0 {
            info!("schedule loop step:{} no tx actions generated", loop_step);
        }

        //执行调度动作
        let ret = system_config_client
            .exec_tx(schedule_plan.tx_actions, None)
            .await;
        if ret.is_err() {
            error!(
                "schedule loop step:{} exec_tx failed, keys={:?}, err={:?}",
                loop_step,
                tx_action_keys,
                ret.err().unwrap()
            );
            continue;
        }
        info!(
            "schedule loop step:{} exec_tx applied {} actions",
            loop_step, tx_action_count
        );
        //save schedule snapshot to system_config
        if schedule_plan.need_persist_snapshot {
            let schedule_snapshot_str = serde_json::to_string(&schedule_plan.schedule_snapshot)?;
            system_config_client
                .set("system/scheduler/snapshot", &schedule_snapshot_str)
                .await
                .map_err(|err| {
                    error!(
                        "schedule loop step:{} snapshot set failed, key=system/scheduler/snapshot, err={:?}",
                        loop_step, err
                    );
                    err
                })?;
            info!(
                "schedule loop step:{} snapshot saved with nodes={} specs={} instances={}",
                loop_step,
                schedule_plan.schedule_snapshot.nodes.len(),
                schedule_plan.schedule_snapshot.specs.len(),
                schedule_plan.schedule_snapshot.replica_instances.len()
            );
        } else {
            info!(
                "schedule loop step:{} snapshot unchanged, skip persisting",
                loop_step
            );
        }
        if is_boot {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::{
        AppDocBuilder, AppServiceSpec, AppType, ServiceExposeConfig, ServiceInstallConfig,
        ServiceInstanceState, ServiceState, SubPkgDesc,
    };
    use jsonwebtoken::jwk::Jwk;
    use name_lib::generate_ed25519_key_pair;
    use name_lib::{DID, DeviceConfig, DeviceNodeType, OODDescriptionString, VerifyHubInfo};

    fn create_test_replica_instance(
        spec_id: &str,
        instance_id: &str,
        node_id: &str,
        ports: &[(&str, u16)],
    ) -> ReplicaInstance {
        ReplicaInstance {
            spec_id: spec_id.to_string(),
            node_id: node_id.to_string(),
            res_limits: HashMap::new(),
            instance_id: instance_id.to_string(),
            last_update_time: buckyos_get_unix_timestamp(),
            state: InstanceState::Running,
            service_ports: ports
                .iter()
                .map(|(name, port)| ((*name).to_string(), *port))
                .collect(),
        }
    }

    fn create_test_device_info(name: &str, rtcp_port: Option<u32>) -> DeviceInfo {
        let (_, public_key_jwk) = generate_ed25519_key_pair();
        let public_key_jwk: Jwk = serde_json::from_value(public_key_jwk).unwrap();
        let pkx = get_x_from_jwk(&public_key_jwk).unwrap();
        let mut device = DeviceConfig::new(name, pkx);
        device.rtcp_port = rtcp_port;
        device.owner = DID::new("bns", "owner");
        DeviceInfo::from_device_doc(&device)
    }

    fn create_test_zone_config() -> ZoneConfig {
        let (_, owner_key_jwk) = generate_ed25519_key_pair();
        let owner_key_jwk: Jwk = serde_json::from_value(owner_key_jwk).unwrap();
        let (_, verify_hub_key_jwk) = generate_ed25519_key_pair();
        let verify_hub_key_jwk: Jwk = serde_json::from_value(verify_hub_key_jwk).unwrap();

        let mut zone_config = ZoneConfig::new(
            DID::new("web", "test.buckyos.io"),
            DID::new("bns", "owner"),
            owner_key_jwk,
        );
        zone_config.oods = vec![
            OODDescriptionString::new("ood1".to_string(), DeviceNodeType::OOD, None, None),
            OODDescriptionString::new("ood2".to_string(), DeviceNodeType::OOD, None, None),
        ];
        zone_config.verify_hub_info = Some(VerifyHubInfo {
            public_key: verify_hub_key_jwk,
        });
        zone_config
    }

    fn create_test_app_spec() -> AppServiceSpec {
        let owner = DID::new("bns", "owner");
        let app_doc = AppDocBuilder::new(
            AppType::Service,
            "files",
            "0.1.0",
            "did:web:buckyos.ai",
            &owner,
        )
        .sdk_version("10")
        .selector_type(SelectorType::Single)
        .build()
        .unwrap();

        let mut install_config = ServiceInstallConfig::default();
        install_config.expose_config.insert(
            "www".to_string(),
            ServiceExposeConfig {
                sub_hostname: vec!["files".to_string()],
                ..Default::default()
            },
        );

        AppServiceSpec {
            app_doc,
            app_index: 1,
            user_id: "alice".to_string(),
            enable: true,
            expected_instance_count: 1,
            state: ServiceState::Running,
            install_config,
        }
    }

    fn create_test_legacy_docker_app_spec() -> AppServiceSpec {
        let mut app_spec = create_test_app_spec();
        app_spec.app_doc.sdk_version = None;
        app_spec
    }

    fn create_test_agent_spec() -> AppServiceSpec {
        let owner = DID::new("bns", "owner");
        let app_doc = AppDocBuilder::new(
            AppType::Agent,
            "jarvis",
            "0.1.0",
            "did:web:buckyos.ai",
            &owner,
        )
        .sdk_version("11")
        .selector_type(SelectorType::Single)
        .agent_pkg(SubPkgDesc::new("jarvis-agent#0.1.0"))
        .build()
        .unwrap();

        let mut install_config = ServiceInstallConfig::default();
        install_config.expose_config.insert(
            "www".to_string(),
            ServiceExposeConfig {
                sub_hostname: vec!["jarvis".to_string()],
                ..Default::default()
            },
        );

        AppServiceSpec {
            app_doc,
            app_index: 2,
            user_id: "alice".to_string(),
            enable: true,
            expected_instance_count: 1,
            state: ServiceState::Running,
            install_config,
        }
    }

    fn create_test_static_web_app_spec() -> AppServiceSpec {
        let owner = DID::new("bns", "owner");
        let app_doc = AppDocBuilder::new(
            AppType::Web,
            "portal",
            "0.1.0",
            "did:web:buckyos.ai",
            &owner,
        )
        .web_pkg(SubPkgDesc::new("portal-web#0.1.0"))
        .build()
        .unwrap();

        let mut install_config = ServiceInstallConfig::default();
        install_config.expose_config.insert(
            "www".to_string(),
            ServiceExposeConfig {
                sub_hostname: vec!["portal".to_string()],
                ..Default::default()
            },
        );

        AppServiceSpec {
            app_doc,
            app_index: 3,
            user_id: "alice".to_string(),
            enable: true,
            expected_instance_count: 1,
            state: ServiceState::Running,
            install_config,
        }
    }

    #[tokio::test]
    async fn test_build_schedule_plan_skips_snapshot_persist_when_only_schedule_time_changes() {
        let zone_config = create_test_zone_config();
        let device_ood1 = create_test_device_info("ood1", None);
        let mut app_spec = create_test_app_spec();
        app_spec.state = ServiceState::Stopped;

        let mut input_system_config = HashMap::new();
        input_system_config.insert(
            "boot/config".to_string(),
            serde_json::to_string(&zone_config).unwrap(),
        );
        input_system_config.insert(
            "devices/ood1/info".to_string(),
            serde_json::to_string(&device_ood1).unwrap(),
        );
        input_system_config.insert(
            "users/alice/apps/files/spec".to_string(),
            serde_json::to_string(&app_spec).unwrap(),
        );

        let first_plan = build_schedule_plan(&input_system_config, false)
            .await
            .expect("first plan should build");
        assert!(
            first_plan.need_persist_snapshot,
            "initial plan should persist snapshot"
        );

        let mut persisted_snapshot = first_plan.schedule_snapshot.clone();
        persisted_snapshot.schedule_time = 0;
        input_system_config.insert(
            "system/scheduler/snapshot".to_string(),
            serde_json::to_string(&persisted_snapshot).unwrap(),
        );

        let second_plan = build_schedule_plan(&input_system_config, false)
            .await
            .expect("second plan should build");

        assert!(
            second_plan.tx_actions.is_empty(),
            "steady-state plan should not emit tx actions: {:?}",
            second_plan.tx_actions.keys().collect::<Vec<_>>()
        );
        assert!(
            !second_plan.need_persist_snapshot,
            "snapshot persist should be skipped when only schedule_time differs"
        );
        assert_ne!(
            second_plan.schedule_snapshot.schedule_time,
            persisted_snapshot.schedule_time
        );
    }

    #[test]
    fn test_create_scheduler_by_system_config_loads_agent_specs() {
        let zone_config = create_test_zone_config();
        let device_ood1 = create_test_device_info("ood1", None);
        let agent_spec = create_test_agent_spec();

        let mut input_system_config = HashMap::new();
        input_system_config.insert(
            "boot/config".to_string(),
            serde_json::to_string(&zone_config).unwrap(),
        );
        input_system_config.insert(
            "devices/ood1/info".to_string(),
            serde_json::to_string(&device_ood1).unwrap(),
        );
        input_system_config.insert(
            "users/alice/settings".to_string(),
            serde_json::to_string(&json!({
                "type": "admin",
                "user_id": "alice",
                "show_name": "alice",
                "password": "hashed",
                "state": "active",
                "res_pool_id": "default"
            }))
            .unwrap(),
        );
        input_system_config.insert(
            "users/alice/agents/jarvis/spec".to_string(),
            serde_json::to_string(&agent_spec).unwrap(),
        );

        let (scheduler_ctx, _) = create_scheduler_by_system_config(&input_system_config).unwrap();
        let spec = scheduler_ctx
            .get_service_spec("jarvis@alice")
            .expect("agent spec should be loaded");

        assert_eq!(spec.app_id, "jarvis");
        assert_eq!(spec.owner_id, "alice");
        assert_eq!(spec.spec_type, ServiceSpecType::App);
        assert!(spec.need_container);
    }

    #[test]
    fn test_create_scheduler_by_system_config_accepts_uppercase_deleted_app_state() {
        let zone_config = create_test_zone_config();
        let device_ood1 = create_test_device_info("ood1", None);
        let mut app_spec = create_test_app_spec();
        app_spec.state = ServiceState::Deleted;
        let mut app_spec_value = serde_json::to_value(&app_spec).unwrap();
        app_spec_value["state"] = json!("Deleted");

        let mut input_system_config = HashMap::new();
        input_system_config.insert(
            "boot/config".to_string(),
            serde_json::to_string(&zone_config).unwrap(),
        );
        input_system_config.insert(
            "devices/ood1/info".to_string(),
            serde_json::to_string(&device_ood1).unwrap(),
        );
        input_system_config.insert(
            "users/alice/apps/files/spec".to_string(),
            serde_json::to_string(&app_spec_value).unwrap(),
        );

        let (scheduler_ctx, _) = create_scheduler_by_system_config(&input_system_config).unwrap();
        let spec = scheduler_ctx
            .get_service_spec("files@alice")
            .expect("app spec should be loaded");

        assert_eq!(spec.state, ServiceSpecState::Deleted);
    }

    #[test]
    fn test_schedule_action_to_tx_actions_instances_agent_and_marks_gateway_update() {
        let zone_config = create_test_zone_config();
        let device_ood1 = create_test_device_info("ood1", None);
        let agent_spec = create_test_agent_spec();

        let mut input_system_config = HashMap::new();
        input_system_config.insert(
            "boot/config".to_string(),
            serde_json::to_string(&zone_config).unwrap(),
        );
        input_system_config.insert(
            "devices/ood1/info".to_string(),
            serde_json::to_string(&device_ood1).unwrap(),
        );
        input_system_config.insert(
            "users/alice/agents/jarvis/spec".to_string(),
            serde_json::to_string(&agent_spec).unwrap(),
        );

        let mut scheduler_ctx = NodeScheduler::new_empty(1);
        scheduler_ctx.add_service_spec(ServiceSpec {
            id: "jarvis@alice".to_string(),
            app_id: "jarvis".to_string(),
            app_index: 2,
            owner_id: "alice".to_string(),
            spec_type: ServiceSpecType::App,
            state: ServiceSpecState::Deployed,
            need_container: true,
            best_instance_count: 1,
            required_cpu_mhz: 200,
            required_memory: DEFAULT_REQUIRED_MEMORY,
            required_gpu_tflops: 0.0,
            required_gpu_mem: 0,
            node_affinity: None,
            network_affinity: None,
            service_ports_config: HashMap::new(),
        });

        let mut device_list = HashMap::new();
        device_list.insert("ood1".to_string(), device_ood1);

        let action = SchedulerAction::InstanceReplica(create_test_replica_instance(
            "jarvis@alice",
            "jarvis@alice@ood1",
            "ood1",
            &[("www", 11080)],
        ));
        let mut need_update_gateway_node_list = HashSet::new();
        let mut need_update_rbac = false;

        let tx_actions = schedule_action_to_tx_actions(
            &action,
            &scheduler_ctx,
            &device_list,
            &input_system_config,
            &mut need_update_gateway_node_list,
            &mut need_update_rbac,
        )
        .unwrap();

        assert!(need_update_gateway_node_list.contains("ood1"));
        let node_action = tx_actions
            .get("nodes/ood1/config")
            .expect("node config update should exist");
        match node_action {
            KVAction::SetByJsonPath(paths) => {
                let value = paths
                    .get("/apps/jarvis@alice@ood1")
                    .and_then(|value| value.as_ref())
                    .expect("agent instance should be written under node apps");
                let instance: buckyos_api::AppServiceInstanceConfig =
                    serde_json::from_value(value.clone()).expect("parse instance config");
                assert_eq!(instance.app_spec.app_id(), "jarvis");
                assert_eq!(instance.app_spec.user_id, "alice");
            }
            other => panic!("unexpected kv action: {:?}", other),
        }
    }

    #[test]
    fn test_schedule_action_to_tx_actions_remove_instance_keeps_app_entry_and_marks_deleted() {
        let zone_config = create_test_zone_config();

        let mut input_system_config = HashMap::new();
        input_system_config.insert(
            "boot/config".to_string(),
            serde_json::to_string(&zone_config).unwrap(),
        );

        let mut scheduler_ctx = NodeScheduler::new_empty(1);
        scheduler_ctx.add_service_spec(ServiceSpec {
            id: "jarvis@alice".to_string(),
            app_id: "jarvis".to_string(),
            app_index: 2,
            owner_id: "alice".to_string(),
            spec_type: ServiceSpecType::App,
            state: ServiceSpecState::Deleted,
            need_container: true,
            best_instance_count: 1,
            required_cpu_mhz: 200,
            required_memory: DEFAULT_REQUIRED_MEMORY,
            required_gpu_tflops: 0.0,
            required_gpu_mem: 0,
            node_affinity: None,
            network_affinity: None,
            service_ports_config: HashMap::new(),
        });

        let instance = create_test_replica_instance(
            "jarvis@alice",
            "jarvis@alice@ood1",
            "ood1",
            &[("www", 11080)],
        );
        scheduler_ctx.add_replica_instance(instance);

        let mut need_update_gateway_node_list = HashSet::new();
        let mut need_update_rbac = false;

        let tx_actions = schedule_action_to_tx_actions(
            &SchedulerAction::RemoveInstance(
                "jarvis@alice".to_string(),
                "jarvis@alice@ood1".to_string(),
                "ood1".to_string(),
            ),
            &scheduler_ctx,
            &HashMap::new(),
            &input_system_config,
            &mut need_update_gateway_node_list,
            &mut need_update_rbac,
        )
        .unwrap();

        assert!(need_update_gateway_node_list.contains("ood1"));
        let node_action = tx_actions
            .get("nodes/ood1/config")
            .expect("node config update should exist");
        match node_action {
            KVAction::SetByJsonPath(paths) => {
                assert_eq!(
                    paths.get("/apps/jarvis@alice@ood1/target_state"),
                    Some(&Some(json!(ServiceInstanceState::Stopped)))
                );
                assert_eq!(
                    paths.get("/apps/jarvis@alice@ood1/app_spec/state"),
                    Some(&Some(json!(ServiceState::Deleted)))
                );
                assert!(!paths.contains_key("/apps/jarvis@alice@ood1"));
            }
            other => panic!("unexpected kv action: {:?}", other),
        }
    }

    #[test]
    fn test_schedule_action_to_tx_actions_remove_instance_works_without_snapshot_instance() {
        let zone_config = create_test_zone_config();

        let mut input_system_config = HashMap::new();
        input_system_config.insert(
            "boot/config".to_string(),
            serde_json::to_string(&zone_config).unwrap(),
        );

        let mut scheduler_ctx = NodeScheduler::new_empty(1);
        scheduler_ctx.add_service_spec(ServiceSpec {
            id: "jarvis@alice".to_string(),
            app_id: "jarvis".to_string(),
            app_index: 2,
            owner_id: "alice".to_string(),
            spec_type: ServiceSpecType::App,
            state: ServiceSpecState::Deleted,
            need_container: true,
            best_instance_count: 1,
            required_cpu_mhz: 200,
            required_memory: DEFAULT_REQUIRED_MEMORY,
            required_gpu_tflops: 0.0,
            required_gpu_mem: 0,
            node_affinity: None,
            network_affinity: None,
            service_ports_config: HashMap::new(),
        });

        let mut need_update_gateway_node_list = HashSet::new();
        let mut need_update_rbac = false;

        let tx_actions = schedule_action_to_tx_actions(
            &SchedulerAction::RemoveInstance(
                "jarvis@alice".to_string(),
                "jarvis@alice@ood1".to_string(),
                "ood1".to_string(),
            ),
            &scheduler_ctx,
            &HashMap::new(),
            &input_system_config,
            &mut need_update_gateway_node_list,
            &mut need_update_rbac,
        )
        .unwrap();

        assert!(need_update_gateway_node_list.contains("ood1"));
        let node_action = tx_actions
            .get("nodes/ood1/config")
            .expect("node config update should exist");
        match node_action {
            KVAction::SetByJsonPath(paths) => {
                assert_eq!(
                    paths.get("/apps/jarvis@alice@ood1/target_state"),
                    Some(&Some(json!(ServiceInstanceState::Stopped)))
                );
                assert_eq!(
                    paths.get("/apps/jarvis@alice@ood1/app_spec/state"),
                    Some(&Some(json!(ServiceState::Deleted)))
                );
            }
            other => panic!("unexpected kv action: {:?}", other),
        }
    }

    #[test]
    fn test_schedule_action_to_tx_actions_update_instance_keeps_app_entry_and_marks_stopped() {
        let zone_config = create_test_zone_config();

        let mut input_system_config = HashMap::new();
        input_system_config.insert(
            "boot/config".to_string(),
            serde_json::to_string(&zone_config).unwrap(),
        );

        let mut scheduler_ctx = NodeScheduler::new_empty(1);
        scheduler_ctx.add_service_spec(ServiceSpec {
            id: "jarvis@alice".to_string(),
            app_id: "jarvis".to_string(),
            app_index: 2,
            owner_id: "alice".to_string(),
            spec_type: ServiceSpecType::App,
            state: ServiceSpecState::Disable,
            need_container: true,
            best_instance_count: 1,
            required_cpu_mhz: 200,
            required_memory: DEFAULT_REQUIRED_MEMORY,
            required_gpu_tflops: 0.0,
            required_gpu_mem: 0,
            node_affinity: None,
            network_affinity: None,
            service_ports_config: HashMap::new(),
        });

        let mut instance = create_test_replica_instance(
            "jarvis@alice",
            "jarvis@alice@ood1",
            "ood1",
            &[("www", 11080)],
        );
        instance.state = InstanceState::Suspended;
        scheduler_ctx.add_replica_instance(instance.clone());

        let mut need_update_gateway_node_list = HashSet::new();
        let mut need_update_rbac = false;

        let tx_actions = schedule_action_to_tx_actions(
            &SchedulerAction::UpdateInstance(instance.instance_id.clone(), instance),
            &scheduler_ctx,
            &HashMap::new(),
            &input_system_config,
            &mut need_update_gateway_node_list,
            &mut need_update_rbac,
        )
        .unwrap();

        let node_action = tx_actions
            .get("nodes/ood1/config")
            .expect("node config update should exist");
        match node_action {
            KVAction::SetByJsonPath(paths) => {
                assert_eq!(
                    paths.get("/apps/jarvis@alice@ood1/target_state"),
                    Some(&Some(json!(ServiceInstanceState::Stopped)))
                );
                assert_eq!(
                    paths.get("/apps/jarvis@alice@ood1/app_spec/state"),
                    Some(&Some(json!(ServiceState::Stopped)))
                );
                assert!(!paths.contains_key("/apps/jarvis@alice@ood1"));
            }
            other => panic!("unexpected kv action: {:?}", other),
        }
    }

    #[test]
    fn test_schedule_action_to_tx_actions_change_service_status_writes_config_state_values() {
        let zone_config = create_test_zone_config();
        let app_spec = create_test_app_spec();

        let mut input_system_config = HashMap::new();
        input_system_config.insert(
            "boot/config".to_string(),
            serde_json::to_string(&zone_config).unwrap(),
        );
        input_system_config.insert(
            "users/alice/apps/files/spec".to_string(),
            serde_json::to_string(&app_spec).unwrap(),
        );

        let mut scheduler_ctx = NodeScheduler::new_empty(1);
        scheduler_ctx.add_service_spec(ServiceSpec {
            id: "files@alice".to_string(),
            app_id: "files".to_string(),
            app_index: 3,
            owner_id: "alice".to_string(),
            spec_type: ServiceSpecType::App,
            state: ServiceSpecState::Deployed,
            need_container: true,
            best_instance_count: 1,
            required_cpu_mhz: 200,
            required_memory: DEFAULT_REQUIRED_MEMORY,
            required_gpu_tflops: 0.0,
            required_gpu_mem: 0,
            node_affinity: None,
            network_affinity: None,
            service_ports_config: HashMap::new(),
        });

        let mut need_update_gateway_node_list = HashSet::new();
        let mut need_update_rbac = false;

        let deployed_actions = schedule_action_to_tx_actions(
            &SchedulerAction::ChangeServiceStatus(
                "files@alice".to_string(),
                ServiceSpecState::Deployed,
            ),
            &scheduler_ctx,
            &HashMap::new(),
            &input_system_config,
            &mut need_update_gateway_node_list,
            &mut need_update_rbac,
        )
        .unwrap();
        match deployed_actions.get("users/alice/apps/files/spec").unwrap() {
            KVAction::SetByJsonPath(paths) => {
                assert_eq!(paths.get("state"), Some(&Some(json!("running"))));
            }
            other => panic!("unexpected kv action: {:?}", other),
        }

        let deleted_actions = schedule_action_to_tx_actions(
            &SchedulerAction::ChangeServiceStatus(
                "files@alice".to_string(),
                ServiceSpecState::Deleted,
            ),
            &scheduler_ctx,
            &HashMap::new(),
            &input_system_config,
            &mut need_update_gateway_node_list,
            &mut need_update_rbac,
        )
        .unwrap();
        match deleted_actions.get("users/alice/apps/files/spec").unwrap() {
            KVAction::SetByJsonPath(paths) => {
                assert_eq!(paths.get("state"), Some(&Some(json!("deleted"))));
            }
            other => panic!("unexpected kv action: {:?}", other),
        }
    }

    #[test]
    fn test_schedule_action_to_tx_actions_keeps_service_info_creation_for_legacy_docker_app_service()
     {
        let zone_config = create_test_zone_config();
        let app_spec = create_test_legacy_docker_app_spec();
        let device_ood1 = create_test_device_info("ood1", None);

        let mut input_system_config = HashMap::new();
        input_system_config.insert(
            "boot/config".to_string(),
            serde_json::to_string(&zone_config).unwrap(),
        );
        input_system_config.insert(
            "devices/ood1/info".to_string(),
            serde_json::to_string(&device_ood1).unwrap(),
        );
        input_system_config.insert(
            "users/alice/apps/files/spec".to_string(),
            serde_json::to_string(&app_spec).unwrap(),
        );

        let mut device_list = HashMap::new();
        device_list.insert("ood1".to_string(), device_ood1);

        let mut need_update_gateway_node_list = HashSet::new();
        let mut need_update_rbac = false;

        let tx_actions = schedule_action_to_tx_actions(
            &SchedulerAction::UpdateServiceInfo(
                "files@alice".to_string(),
                ServiceInfo::SingleInstance(create_test_replica_instance(
                    "files@alice",
                    "files@alice@ood1",
                    "ood1",
                    &[("www", 10160)],
                )),
            ),
            &NodeScheduler::new_empty(1),
            &device_list,
            &input_system_config,
            &mut need_update_gateway_node_list,
            &mut need_update_rbac,
        )
        .unwrap();

        assert!(tx_actions.contains_key("services/files@alice/info"));
    }

    #[test]
    fn test_schedule_action_to_tx_actions_skips_service_info_deletion_for_legacy_docker_app_service()
     {
        let zone_config = create_test_zone_config();
        let app_spec = create_test_legacy_docker_app_spec();
        let device_ood1 = create_test_device_info("ood1", None);

        let mut input_system_config = HashMap::new();
        input_system_config.insert(
            "boot/config".to_string(),
            serde_json::to_string(&zone_config).unwrap(),
        );
        input_system_config.insert(
            "devices/ood1/info".to_string(),
            serde_json::to_string(&device_ood1).unwrap(),
        );
        input_system_config.insert(
            "users/alice/apps/files/spec".to_string(),
            serde_json::to_string(&app_spec).unwrap(),
        );

        let mut device_list = HashMap::new();
        device_list.insert("ood1".to_string(), device_ood1);

        let mut need_update_gateway_node_list = HashSet::new();
        let mut need_update_rbac = false;

        let tx_actions = schedule_action_to_tx_actions(
            &SchedulerAction::UpdateServiceInfo(
                "files@alice".to_string(),
                ServiceInfo::RandomCluster(HashMap::new()),
            ),
            &NodeScheduler::new_empty(1),
            &device_list,
            &input_system_config,
            &mut need_update_gateway_node_list,
            &mut need_update_rbac,
        )
        .unwrap();

        assert!(tx_actions.is_empty());
    }

    #[test]
    fn test_schedule_action_to_tx_actions_keeps_service_info_for_sdk_app_service() {
        let zone_config = create_test_zone_config();
        let app_spec = create_test_app_spec();
        let device_ood1 = create_test_device_info("ood1", None);

        let mut input_system_config = HashMap::new();
        input_system_config.insert(
            "boot/config".to_string(),
            serde_json::to_string(&zone_config).unwrap(),
        );
        input_system_config.insert(
            "devices/ood1/info".to_string(),
            serde_json::to_string(&device_ood1).unwrap(),
        );
        input_system_config.insert(
            "users/alice/apps/files/spec".to_string(),
            serde_json::to_string(&app_spec).unwrap(),
        );

        let mut device_list = HashMap::new();
        device_list.insert("ood1".to_string(), device_ood1);

        let mut need_update_gateway_node_list = HashSet::new();
        let mut need_update_rbac = false;

        let tx_actions = schedule_action_to_tx_actions(
            &SchedulerAction::UpdateServiceInfo(
                "files@alice".to_string(),
                ServiceInfo::SingleInstance(create_test_replica_instance(
                    "files@alice",
                    "files@alice@ood1",
                    "ood1",
                    &[("www", 10160)],
                )),
            ),
            &NodeScheduler::new_empty(1),
            &device_list,
            &input_system_config,
            &mut need_update_gateway_node_list,
            &mut need_update_rbac,
        )
        .unwrap();

        assert!(tx_actions.contains_key("services/files@alice/info"));
    }

    #[tokio::test]
    async fn test_update_node_gateway_info_builds_expected_payload() {
        let zone_config = create_test_zone_config();
        let app_spec = create_test_app_spec();
        let device_ood1 = create_test_device_info("ood1", None);
        let device_ood2 = create_test_device_info("ood2", Some(2981));

        let mut input_system_config = HashMap::new();
        input_system_config.insert(
            "boot/config".to_string(),
            serde_json::to_string(&zone_config).unwrap(),
        );
        input_system_config.insert(
            "devices/ood1/info".to_string(),
            serde_json::to_string(&device_ood1).unwrap(),
        );
        input_system_config.insert(
            "devices/ood2/info".to_string(),
            serde_json::to_string(&device_ood2).unwrap(),
        );
        input_system_config.insert(
            "users/alice/apps/files/spec".to_string(),
            serde_json::to_string(&app_spec).unwrap(),
        );

        let mut scheduler_ctx = NodeScheduler::new_empty(1);
        scheduler_ctx.service_infos.insert(
            "control-panel".to_string(),
            ServiceInfo::SingleInstance(create_test_replica_instance(
                "control-panel",
                "control-panel@ood1",
                "ood1",
                &[("www", 4020)],
            )),
        );
        scheduler_ctx.service_infos.insert(
            "system_config".to_string(),
            ServiceInfo::SingleInstance(create_test_replica_instance(
                "system_config",
                "system_config@ood1",
                "ood1",
                &[("http", 3200)],
            )),
        );
        scheduler_ctx.service_infos.insert(
            "files@alice".to_string(),
            ServiceInfo::SingleInstance(create_test_replica_instance(
                "files@alice",
                "files@alice@ood2",
                "ood2",
                &[("www", 10160)],
            )),
        );

        let actions = update_node_gateway_info("ood1", &scheduler_ctx, &input_system_config)
            .await
            .unwrap();
        let gateway_info_str = match actions.get("nodes/ood1/gateway_info").unwrap() {
            KVAction::Update(value) => value,
            other => panic!("unexpected kv action: {:?}", other),
        };
        println!("gateway_info_str: {}", gateway_info_str);
        let gateway_info: NodeGatewayInfo = serde_json::from_str(gateway_info_str).unwrap();

        assert_eq!(gateway_info.node_info.this_node_id, "ood1");
        assert_eq!(gateway_info.node_info.this_zone_host, "test.buckyos.io");
        assert_eq!(
            gateway_info.node_route_map.get("ood2").unwrap(),
            "rtcp://ood2.test.buckyos.io:2981/"
        );
        assert!(gateway_info.trust_key.contains_key("verify-hub"));
        assert!(gateway_info.trust_key.contains_key("ood1"));

        let system_config = gateway_info.service_info.get("system_config").unwrap();
        assert_eq!(system_config.selector.get("ood1").unwrap().port, 3200);
        assert_eq!(system_config.selector.get("ood2").unwrap().port, 3200);

        let files = match gateway_info.app_info.get("files").unwrap() {
            NodeGatewayAppEntry::App(entry) => entry,
            _ => panic!("files should resolve to an app entry"),
        };
        assert_eq!(files.app_id, "files");
        assert_eq!(files.node_id.as_deref(), Some("ood2"));
        assert_eq!(files.port, Some(10160));
        assert_eq!(files.dir_pkg_id, None);
        assert_eq!(files.sdk_version, 10);

        let sys = match gateway_info.app_info.get("sys").unwrap() {
            NodeGatewayAppEntry::Service(entry) => entry,
            _ => panic!("sys should resolve to a service entry"),
        };
        assert_eq!(sys.service_id, "control-panel");
        assert_eq!(sys.selector.get("ood1").unwrap().port, 4020);
    }

    #[tokio::test]
    async fn test_update_node_gateway_info_adds_klog_cluster_route_entry() {
        let zone_config = create_test_zone_config();
        let device_ood1 = create_test_device_info("ood1", None);
        let device_ood2 = create_test_device_info("ood2", Some(2981));

        let mut input_system_config = HashMap::new();
        input_system_config.insert(
            "boot/config".to_string(),
            serde_json::to_string(&zone_config).unwrap(),
        );
        input_system_config.insert(
            "devices/ood1/info".to_string(),
            serde_json::to_string(&device_ood1).unwrap(),
        );
        input_system_config.insert(
            "devices/ood2/info".to_string(),
            serde_json::to_string(&device_ood2).unwrap(),
        );
        input_system_config.insert(
            "services/klog-service/settings".to_string(),
            json!({
                "cluster_network": {
                    "gateway_route_prefix": "/.cluster/klog"
                }
            })
            .to_string(),
        );

        let mut scheduler_ctx = NodeScheduler::new_empty(1);
        scheduler_ctx.service_infos.insert(
            "klog-service:raft".to_string(),
            ServiceInfo::RandomCluster(HashMap::from([
                (
                    "ood1".to_string(),
                    (
                        100,
                        create_test_replica_instance(
                            "klog-service",
                            "klog-service@ood1",
                            "ood1",
                            &[(KLOG_CLUSTER_RAFT_SERVICE_NAME, 21001)],
                        ),
                    ),
                ),
                (
                    "ood2".to_string(),
                    (
                        100,
                        create_test_replica_instance(
                            "klog-service",
                            "klog-service@ood2",
                            "ood2",
                            &[(KLOG_CLUSTER_RAFT_SERVICE_NAME, 21011)],
                        ),
                    ),
                ),
            ])),
        );
        scheduler_ctx.service_infos.insert(
            "klog-service:inter".to_string(),
            ServiceInfo::RandomCluster(HashMap::from([
                (
                    "ood1".to_string(),
                    (
                        100,
                        create_test_replica_instance(
                            "klog-service",
                            "klog-service@ood1",
                            "ood1",
                            &[(KLOG_CLUSTER_INTER_SERVICE_NAME, 21002)],
                        ),
                    ),
                ),
                (
                    "ood2".to_string(),
                    (
                        100,
                        create_test_replica_instance(
                            "klog-service",
                            "klog-service@ood2",
                            "ood2",
                            &[(KLOG_CLUSTER_INTER_SERVICE_NAME, 21012)],
                        ),
                    ),
                ),
            ])),
        );
        scheduler_ctx.service_infos.insert(
            "klog-service:admin".to_string(),
            ServiceInfo::RandomCluster(HashMap::from([
                (
                    "ood1".to_string(),
                    (
                        100,
                        create_test_replica_instance(
                            "klog-service",
                            "klog-service@ood1",
                            "ood1",
                            &[(KLOG_CLUSTER_ADMIN_SERVICE_NAME, 21003)],
                        ),
                    ),
                ),
                (
                    "ood2".to_string(),
                    (
                        100,
                        create_test_replica_instance(
                            "klog-service",
                            "klog-service@ood2",
                            "ood2",
                            &[(KLOG_CLUSTER_ADMIN_SERVICE_NAME, 21013)],
                        ),
                    ),
                ),
            ])),
        );

        let actions = update_node_gateway_info("ood1", &scheduler_ctx, &input_system_config)
            .await
            .unwrap();
        let gateway_info_str = match actions.get("nodes/ood1/gateway_info").unwrap() {
            KVAction::Update(value) => value,
            other => panic!("unexpected kv action: {:?}", other),
        };
        let gateway_info: NodeGatewayInfo = serde_json::from_str(gateway_info_str).unwrap();

        let klog_cluster_route = gateway_info
            .cluster_route_map
            .get(KLOG_SERVICE_UNIQUE_ID)
            .unwrap();
        assert_eq!(klog_cluster_route.route_prefix, "/.cluster/klog");
        assert_eq!(klog_cluster_route.ingress_port, DEFAULT_NODE_GATEWAY_HTTP_PORT);

        let local = klog_cluster_route.nodes.get("ood1").unwrap();
        assert_eq!(local.ports.get("raft"), Some(&21001));
        assert_eq!(local.ports.get("inter"), Some(&21002));
        assert_eq!(local.ports.get("admin"), Some(&21003));

        let remote = klog_cluster_route.nodes.get("ood2").unwrap();
        assert_eq!(remote.ports.get("raft"), Some(&21011));
        assert_eq!(remote.ports.get("inter"), Some(&21012));
        assert_eq!(remote.ports.get("admin"), Some(&21013));
    }

    #[test]
    fn test_extract_klog_cluster_route_prefix_normalizes_slashes() {
        let mut input_system_config = HashMap::new();
        input_system_config.insert(
            "services/klog-service/settings".to_string(),
            json!({
                "cluster_network": {
                    "gateway_route_prefix": "cluster/klog-test/"
                }
            })
            .to_string(),
        );

        assert_eq!(
            extract_klog_cluster_route_prefix(&input_system_config),
            "/cluster/klog-test"
        );
    }

    #[tokio::test]
    async fn test_update_node_gateway_info_keeps_legacy_app_entry_from_persisted_service_info() {
        let zone_config = create_test_zone_config();
        let app_spec = create_test_legacy_docker_app_spec();
        let device_ood1 = create_test_device_info("ood1", None);

        let persisted_service_info = buckyos_api::ServiceInfo {
            selector_type: "random".to_string(),
            node_list: HashMap::from([(
                "ood1".to_string(),
                buckyos_api::ServiceNode {
                    node_did: device_ood1.id.clone(),
                    node_net_id: device_ood1.device_doc.net_id.clone(),
                    state: buckyos_api::ServiceInstanceState::Started,
                    weight: 100,
                    service_port: HashMap::from([("www".to_string(), 10160)]),
                },
            )]),
        };

        let mut input_system_config = HashMap::new();
        input_system_config.insert(
            "boot/config".to_string(),
            serde_json::to_string(&zone_config).unwrap(),
        );
        input_system_config.insert(
            "devices/ood1/info".to_string(),
            serde_json::to_string(&device_ood1).unwrap(),
        );
        input_system_config.insert(
            "users/alice/apps/files/spec".to_string(),
            serde_json::to_string(&app_spec).unwrap(),
        );
        input_system_config.insert(
            "services/files@alice/info".to_string(),
            serde_json::to_string(&persisted_service_info).unwrap(),
        );

        let mut scheduler_ctx = NodeScheduler::new_empty(1);
        scheduler_ctx.service_infos.insert(
            "files@alice".to_string(),
            ServiceInfo::RandomCluster(HashMap::new()),
        );

        let actions = update_node_gateway_info("ood1", &scheduler_ctx, &input_system_config)
            .await
            .unwrap();
        let gateway_info_str = match actions.get("nodes/ood1/gateway_info").unwrap() {
            KVAction::Update(value) => value,
            other => panic!("unexpected kv action: {:?}", other),
        };
        let gateway_info: NodeGatewayInfo = serde_json::from_str(gateway_info_str).unwrap();

        let files = match gateway_info.app_info.get("files").unwrap() {
            NodeGatewayAppEntry::App(entry) => entry,
            _ => panic!("files should resolve to an app entry"),
        };
        assert_eq!(files.app_id, "files");
        assert_eq!(files.node_id.as_deref(), Some("ood1"));
        assert_eq!(files.port, Some(10160));
        assert_eq!(files.sdk_version, 0);
    }

    #[tokio::test]
    async fn test_update_node_gateway_config_keeps_acme_and_zone_tls() {
        let mut input_system_config = HashMap::new();
        let mut zone_config = create_test_zone_config();
        zone_config.sn = Some("sn.test.buckyos.io".to_string());
        input_system_config.insert(
            "boot/config".to_string(),
            serde_json::to_string(&zone_config).unwrap(),
        );

        let mut nodes = HashSet::new();
        nodes.insert("ood1".to_string());

        let actions = update_node_gateway_config(&nodes, &input_system_config)
            .await
            .unwrap();
        let gateway_config_str = match actions.get("nodes/ood1/gateway_config").unwrap() {
            KVAction::Update(value) => value,
            other => panic!("unexpected kv action: {:?}", other),
        };
        let gateway_config: serde_json::Value = serde_json::from_str(gateway_config_str).unwrap();

        assert_eq!(
            gateway_config["acme"]["dns_providers"]["sn-dns"]["sn"],
            "https://sn.test.buckyos.io/kapi/sn"
        );
        assert_eq!(gateway_config["stacks"]["zone_tls"]["bind"], "0.0.0.0:443");
        assert_eq!(
            gateway_config["stacks"]["zone_tls"]["hook_point"]["main"]["blocks"]["default"]["block"],
            "return \"server node_gateway\";\n"
        );
    }

    #[tokio::test]
    async fn test_update_node_gateway_config_adds_dir_server_for_static_web_app() {
        let zone_config = create_test_zone_config();
        let web_app_spec = create_test_static_web_app_spec();

        let mut input_system_config = HashMap::new();
        input_system_config.insert(
            "boot/config".to_string(),
            serde_json::to_string(&zone_config).unwrap(),
        );
        input_system_config.insert(
            "users/alice/apps/portal/spec".to_string(),
            serde_json::to_string(&web_app_spec).unwrap(),
        );

        let mut nodes = HashSet::new();
        nodes.insert("ood1".to_string());

        let actions = update_node_gateway_config(&nodes, &input_system_config)
            .await
            .unwrap();
        let gateway_config_str = match actions.get("nodes/ood1/gateway_config").unwrap() {
            KVAction::Update(value) => value,
            other => panic!("unexpected kv action: {:?}", other),
        };
        let gateway_config: serde_json::Value = serde_json::from_str(gateway_config_str).unwrap();

        assert_eq!(gateway_config["servers"]["portal-web"]["type"], "dir");
        assert_eq!(
            gateway_config["servers"]["portal-web"]["root_path"],
            "../bin/portal-web/"
        );
    }

    #[tokio::test]
    async fn test_update_node_gateway_info_adds_static_web_app_entry() {
        let zone_config = create_test_zone_config();
        let mut web_app_spec = create_test_static_web_app_spec();
        web_app_spec
            .app_doc
            .pkg_list
            .web
            .as_mut()
            .unwrap()
            .pkg_objid = Some(serde_json::from_value(json!("pkg:1234567890")).unwrap());
        let device_ood1 = create_test_device_info("ood1", None);

        let mut input_system_config = HashMap::new();
        input_system_config.insert(
            "boot/config".to_string(),
            serde_json::to_string(&zone_config).unwrap(),
        );
        input_system_config.insert(
            "devices/ood1/info".to_string(),
            serde_json::to_string(&device_ood1).unwrap(),
        );
        input_system_config.insert(
            "users/alice/apps/portal/spec".to_string(),
            serde_json::to_string(&web_app_spec).unwrap(),
        );

        let scheduler_ctx = NodeScheduler::new_empty(1);
        let actions = update_node_gateway_info("ood1", &scheduler_ctx, &input_system_config)
            .await
            .unwrap();
        let gateway_info_str = match actions.get("nodes/ood1/gateway_info").unwrap() {
            KVAction::Update(value) => value,
            other => panic!("unexpected kv action: {:?}", other),
        };
        let gateway_info: NodeGatewayInfo = serde_json::from_str(gateway_info_str).unwrap();

        let portal = match gateway_info.app_info.get("portal").unwrap() {
            NodeGatewayAppEntry::App(entry) => entry,
            _ => panic!("portal should resolve to an app entry"),
        };
        assert_eq!(portal.app_id, "portal");
        assert_eq!(portal.sdk_version, 0);
        assert_eq!(portal.access_mode, NodeGatewayAccessMode::Public);
        assert_eq!(portal.node_id, None);
        assert_eq!(portal.port, None);
        assert_eq!(portal.dir_pkg_id.as_deref(), Some("portal-web"));
        assert_eq!(portal.dir_pkg_objid.as_deref(), Some("pkg:1234567890"));
    }

    #[tokio::test]
    async fn test_update_node_gateway_info_skips_deleted_static_web_app_entry() {
        let zone_config = create_test_zone_config();
        let mut web_app_spec = create_test_static_web_app_spec();
        web_app_spec.state = ServiceState::Deleted;
        let device_ood1 = create_test_device_info("ood1", None);

        let mut input_system_config = HashMap::new();
        input_system_config.insert(
            "boot/config".to_string(),
            serde_json::to_string(&zone_config).unwrap(),
        );
        input_system_config.insert(
            "devices/ood1/info".to_string(),
            serde_json::to_string(&device_ood1).unwrap(),
        );
        input_system_config.insert(
            "users/alice/apps/portal/spec".to_string(),
            serde_json::to_string(&web_app_spec).unwrap(),
        );

        let scheduler_ctx = NodeScheduler::new_empty(1);
        let actions = update_node_gateway_info("ood1", &scheduler_ctx, &input_system_config)
            .await
            .unwrap();
        let gateway_info_str = match actions.get("nodes/ood1/gateway_info").unwrap() {
            KVAction::Update(value) => value,
            other => panic!("unexpected kv action: {:?}", other),
        };
        let gateway_info: NodeGatewayInfo = serde_json::from_str(gateway_info_str).unwrap();

        assert!(!gateway_info.app_info.contains_key("portal"));
    }

    #[tokio::test]
    async fn test_update_node_gateway_info_reads_agent_specs() {
        let zone_config = create_test_zone_config();
        let agent_spec = create_test_agent_spec();
        let device_ood1 = create_test_device_info("ood1", None);

        let mut input_system_config = HashMap::new();
        input_system_config.insert(
            "boot/config".to_string(),
            serde_json::to_string(&zone_config).unwrap(),
        );
        input_system_config.insert(
            "devices/ood1/info".to_string(),
            serde_json::to_string(&device_ood1).unwrap(),
        );
        input_system_config.insert(
            "users/alice/agents/jarvis/spec".to_string(),
            serde_json::to_string(&agent_spec).unwrap(),
        );

        let mut scheduler_ctx = NodeScheduler::new_empty(1);
        scheduler_ctx.service_infos.insert(
            "jarvis@alice".to_string(),
            ServiceInfo::SingleInstance(create_test_replica_instance(
                "jarvis@alice",
                "jarvis@alice@ood1",
                "ood1",
                &[("www", 11080)],
            )),
        );

        let actions = update_node_gateway_info("ood1", &scheduler_ctx, &input_system_config)
            .await
            .unwrap();
        let gateway_info_str = match actions.get("nodes/ood1/gateway_info").unwrap() {
            KVAction::Update(value) => value,
            other => panic!("unexpected kv action: {:?}", other),
        };
        let gateway_info: NodeGatewayInfo = serde_json::from_str(gateway_info_str).unwrap();

        let jarvis = match gateway_info.app_info.get("jarvis").unwrap() {
            NodeGatewayAppEntry::App(entry) => entry,
            _ => panic!("jarvis should resolve to an app entry"),
        };
        assert_eq!(jarvis.app_id, "jarvis");
        assert_eq!(jarvis.node_id.as_deref(), Some("ood1"));
        assert_eq!(jarvis.port, Some(11080));
        assert_eq!(jarvis.dir_pkg_id, None);
        assert_eq!(jarvis.sdk_version, 11);
    }
}
