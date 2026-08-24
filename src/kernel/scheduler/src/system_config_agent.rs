use std::collections::{BTreeSet, HashMap, HashSet};

use anyhow::Result;
use buckyos_api::{AgentSpec, AppType, SelectorType};
use log::*;
use package_lib::PackageId;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::app::*;
use crate::scheduler::*;
use crate::service::*;
use crate::system_config_builder::{
    derive_sn_ai_provider_endpoints, reconcile_managed_sn_ai_provider,
};
use crate::zone_route_builder::{build_forward_plan, DidIpHint, NodeGatewayRouteCandidate};
use buckyos_api::{
    app_availability_policy_key, get_buckyos_api_runtime, AppAvailabilityGroupRule,
    AppAvailabilityPolicy, AppId, AppInstanceId, AppRegistry, AppServiceSpec, AvailabilityEffect,
    KernelServiceSpec, NodeConfig, SchedulerRefreshRbacResponse, ServiceInstanceReportInfo,
    ServiceState, SystemConfigClient, UserSettings, UserState, UserType as ApiUserType, ZoneConfig,
    ZoneGatewaySettings, APP_REGISTRY_KEY, CONTROL_PANEL_SERVICE_PORT,
};
use buckyos_kit::*;
use name_client::*;
use name_lib::{get_x_from_jwk, DeviceInfo, ZoneDocument};

const SYSTEM_CONFIG_SERVICE_PORT: u16 = 3200;
const FIXED_SERVICE_WEIGHT: u32 = 100;
const DEFAULT_REQUIRED_CPU_MHZ: u32 = 50;
const DEFAULT_REQUIRED_MEMORY: u64 = 32 * 1024 * 1024;

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
    let is_agent = app_config.app_doc.get_app_type() == AppType::Agent;

    let mut need_container = is_agent;
    if !need_container {
        need_container = true;
        if app_config
            .app_doc
            .pkg_list
            .iter()
            .into_iter()
            .any(|(_, pkg)| pkg.docker_image_name.is_none())
            && matches!(
                app_config.app_doc.author.to_string().as_str(),
                "did:web:buckyos.ai" | "did:web:buckyos.io" | "did:web:buckyos.org"
            )
        {
            need_container = false;
        }
    }

    let service_ports_config = app_config.spec_config.to_service_ports_config();
    ServiceSpec {
        id: full_app_id.to_string(),
        app_index: app_config.app_index,
        app_id: app_config.app_id().to_string(),
        owner_id: owner_user_id.to_string(),
        spec_type: ServiceSpecType::App,
        state: spec_state,
        need_container,
        best_instance_count: app_config.expected_instance_count,
        required_cpu_mhz: DEFAULT_REQUIRED_CPU_MHZ,
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
    let service_ports_config = service_config.spec_config.to_service_ports_config();
    ServiceSpec {
        id: service_name.to_string(),
        app_id: service_name.to_string(),
        app_index: 0,
        owner_id: "root".to_string(),
        spec_type: ServiceSpecType::Kernel,
        state: spec_state,
        need_container: false,
        best_instance_count: service_config.expected_instance_count,
        required_cpu_mhz: DEFAULT_REQUIRED_CPU_MHZ,
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
                if parts.len() == 5 && parts[2] == "apps" && parts[4] == "spec" {
                    let user_id = parts[1];
                    let app_id = parts[3];
                    let app_config: AppServiceSpec =
                        serde_json::from_str(value.as_str()).map_err(|e| {
                            error!(
                                "AppConfig serde_json::from_str failed: {:?} {}",
                                e,
                                value.as_str()
                            );
                            e
                        })?;
                    if app_config.owner_user_id != user_id
                        || app_config.app_id().as_str() != app_id
                        || app_config.app_instance_id.owner_user_id() != user_id
                    {
                        return Err(anyhow::anyhow!("invalid user app spec at {key}"));
                    }
                    let app_instance_id = app_config.app_instance_id();
                    if app_config.app_doc.selector_type != SelectorType::Static {
                        let service_spec = create_service_spec_by_app_config(
                            &app_instance_id.to_string(),
                            user_id,
                            &app_config,
                        );
                        scheduler_ctx.add_service_spec(service_spec);
                    }
                }
            } else if key.ends_with("/settings") {
                let parts: Vec<&str> = key.split('/').collect();
                if parts.len() == 3 {
                    let user_id = parts[1];
                    let user_settings: UserSettings = serde_json::from_str(value.as_str())
                        .map_err(|e| {
                            error!("UserSettings serde_json::from_str failed: {:?}", e);
                            e
                        })?;
                    if !matches!(user_settings.state, UserState::Active) {
                        continue;
                    }
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
                    spec_id: app_config.node_execution_spec.app_instance_id.to_string(),
                    node_id: node_id.to_string(),
                    res_limits: HashMap::new(),
                    replica_key: ReplicaKey {
                        service: ReplicaServiceIdentity::App {
                            app_instance_id: app_instance_id.clone(),
                        },
                        node_id: node_id.to_string(),
                    },
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

            let service = match service_name.parse::<buckyos_api::AppInstanceId>() {
                Ok(app_instance_id) => ReplicaServiceIdentity::App { app_instance_id },
                Err(_) => ReplicaServiceIdentity::System {
                    service_id: buckyos_api::SystemServiceId::parse(service_name.to_string())
                        .map_err(anyhow::Error::msg)?,
                },
            };
            let instance = ReplicaInstance {
                spec_id: service_name.to_string(),
                node_id: instance_node_id.to_string(),
                res_limits: HashMap::new(),
                replica_key: ReplicaKey {
                    service,
                    node_id: instance_node_id.to_string(),
                },
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
    let zone_document = zone_config.zone_document()?;
    let zone_gateway = zone_document.get_default_zone_gateway();
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
        SchedulerAction::RemoveInstance(spec_id, replica_key) => {
            let service_spec = scheduler_ctx.get_service_spec(spec_id.as_str());
            if service_spec.is_none() {
                return Err(anyhow::anyhow!("service_spec not found"));
            }
            let service_spec = service_spec.unwrap();
            need_update_gateway_node_list.insert(replica_key.node_id.clone());
            let instance = scheduler_ctx
                .get_replica_instance(replica_key)
                .cloned()
                .unwrap_or_else(|| {
                    warn!(
                        "remove instance {} missing from scheduler snapshot, using action payload",
                        replica_key
                    );
                    ReplicaInstance {
                        spec_id: spec_id.clone(),
                        replica_key: replica_key.clone(),
                        node_id: replica_key.node_id.clone(),
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
        SchedulerAction::UpdateInstance(replica_key, instance) => {
            //相对比较复杂的操作:需要根据service_spec的类型,来执行更新实例化操作
            let spec_id = replica_key.spec_id();
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
                    "skip deleting service info for non-SDK container app-service: {}",
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
    let Some((spec_id, service_name)) = service_info_id.rsplit_once(':') else {
        return (service_info_id.to_string(), "www".to_string());
    };
    if service_name.contains('@') {
        return (service_info_id.to_string(), "www".to_string());
    }
    (spec_id.to_string(), service_name.to_string())
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
    let keys = vec![format!("users/{user_id}/apps/{app_id}/spec")];
    for key in keys {
        if let Some(app_spec) = input_system_config.get(&key) {
            let app_spec: AppServiceSpec = serde_json::from_str(app_spec.as_str())?;
            if app_spec.owner_user_id != user_id || app_spec.app_id().as_str() != app_id {
                return Err(anyhow::anyhow!("invalid app spec at {key}"));
            }
            return Ok(app_spec);
        }
    }
    warn!(
        "app_spec not found at users/{}/apps/{}/spec",
        user_id, app_id
    );
    Err(anyhow::anyhow!("app_spec not found"))
}

fn reconcile_app_instance_specs(
    input_system_config: &HashMap<String, String>,
) -> Result<(HashMap<String, KVAction>, HashSet<String>)> {
    let mut actions = HashMap::new();
    let mut updated_nodes = HashSet::new();

    for (key, value) in input_system_config {
        if !key.starts_with("nodes/") || !key.ends_with("/config") {
            continue;
        }

        let Some(node_id) = key.split('/').nth(1) else {
            continue;
        };
        let node_config: NodeConfig = serde_json::from_str(value)?;
        let mut set_paths = HashMap::new();

        for (instance_id, instance_config) in &node_config.apps {
            let spec_id = instance_config
                .node_execution_spec
                .app_instance_id
                .to_string();
            let desired_spec = match get_app_spec_by_spec_id(&spec_id, input_system_config) {
                Ok(spec) => spec,
                Err(err) => {
                    warn!(
                        "skip app instance spec reconciliation for {} on {}: {}",
                        instance_id, node_id, err
                    );
                    continue;
                }
            };

            let desired_execution_spec = desired_spec
                .to_node_execution_spec()
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            if instance_config.node_execution_spec == desired_execution_spec
                && instance_config.deployment == desired_spec.deployment
            {
                continue;
            }
            set_paths.insert(
                format!("/apps/{instance_id}/node_execution_spec"),
                Some(serde_json::to_value(&desired_execution_spec)?),
            );
            set_paths.insert(
                format!("/apps/{instance_id}/deployment"),
                Some(serde_json::to_value(&desired_spec.deployment)?),
            );

            info!(
                "will refresh app instance spec in place: instance={} node={} generation={}->{}",
                instance_id,
                node_id,
                instance_config.deployment.spec_generation,
                desired_spec.deployment.spec_generation
            );
        }

        if !set_paths.is_empty() {
            actions.insert(key.clone(), KVAction::SetByJsonPath(set_paths));
            updated_nodes.insert(node_id.to_string());
        }
    }

    Ok((actions, updated_nodes))
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

    Ok(is_non_sdk_container_app_service(&app_spec) && is_service_info_empty)
}

pub fn get_boot_config(input_system_config: &HashMap<String, String>) -> Result<ZoneConfig> {
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

pub fn get_zone_config(input_system_config: &HashMap<String, String>) -> Result<ZoneDocument> {
    Ok(get_boot_config(input_system_config)?.zone_document()?)
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
                let app_config: AppServiceSpec =
                    serde_json::from_str(value.as_str()).map_err(|e| {
                        error!(
                            "AppConfig serde_json::from_str failed: {:?} {}",
                            e,
                            value.as_str()
                        );
                        e
                    })?;
                if app_config.owner_user_id != user_id || app_config.app_id().as_str() != app_id {
                    return Err(anyhow::anyhow!("invalid user app spec at {key}"));
                }
                let app_instance_id = app_config.app_instance_id();
                let is_web_app = app_config.app_doc.selector_type == SelectorType::Static;
                let is_gateway_visible = app_config.enable
                    && !matches!(
                        app_config.state,
                        ServiceState::Deleted | ServiceState::Stopped | ServiceState::Stopping
                    );
                if is_web_app && is_gateway_visible {
                    info!("found web app: {}", app_instance_id);
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
        if !web_app.spec_config.expose_config.contains_key("www") {
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
    app_id: AppId,
    app_instance_id: AppInstanceId,
    app_owner_user_id: String,
    sdk_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    deployment: Option<buckyos_api::DeploymentIdentity>,
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
    #[serde(default)]
    routes: HashMap<String, Vec<NodeGatewayRouteCandidate>>,
    #[serde(default)]
    did_ip_hints: HashMap<String, Vec<DidIpHint>>,
    trust_key: HashMap<String, String>,
}

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

fn is_non_sdk_container_app_service(app_spec: &AppServiceSpec) -> bool {
    app_spec.app_doc.selector_type != SelectorType::Static
        && app_spec.app_doc.get_app_type() != AppType::Agent
        && app_spec.app_doc.sdk_version.is_none()
}

fn build_app_host_entry(
    app_spec: &AppServiceSpec,
    service_info: &ServiceInfo,
    service_name: &str,
    guest_allowed: bool,
) -> Option<NodeGatewayAppInfoEntry> {
    let pick_instance = match service_info {
        ServiceInfo::SingleInstance(instance) => Some(instance),
        ServiceInfo::RandomCluster(cluster) => cluster
            .values()
            .map(|(_, instance)| instance)
            .filter(|instance| select_gateway_port(&instance.service_ports, service_name).is_some())
            .min_by(|left, right| left.replica_key.node_id.cmp(&right.replica_key.node_id)),
    }?;

    let port = select_gateway_port(&pick_instance.service_ports, service_name)?;
    let access_mode = if guest_allowed {
        NodeGatewayAccessMode::Public
    } else {
        NodeGatewayAccessMode::Private
    };
    Some(NodeGatewayAppInfoEntry {
        app_id: app_spec.app_id().clone(),
        app_instance_id: app_spec.app_instance_id().clone(),
        app_owner_user_id: app_spec.owner_user_id.clone(),
        sdk_version: parse_sdk_version(app_spec),
        deployment: Some(app_spec.deployment.clone()),
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
    guest_allowed: bool,
) -> Option<NodeGatewayAppInfoEntry> {
    let (node_id, node_info) = service_info
        .node_list
        .iter()
        .filter(|(_, node)| node.state == buckyos_api::ServiceInstanceState::Started)
        .filter(|(_, node)| select_gateway_port(&node.service_port, service_name).is_some())
        .min_by(|left, right| left.0.cmp(right.0))?;

    let port = select_gateway_port(&node_info.service_port, service_name)?;
    let access_mode = if guest_allowed {
        NodeGatewayAccessMode::Public
    } else {
        NodeGatewayAccessMode::Private
    };
    Some(NodeGatewayAppInfoEntry {
        app_id: app_spec.app_id().clone(),
        app_instance_id: app_spec.app_instance_id().clone(),
        app_owner_user_id: app_spec.owner_user_id.clone(),
        sdk_version: parse_sdk_version(app_spec),
        deployment: Some(app_spec.deployment.clone()),
        access_mode,
        node_id: Some(node_id.clone()),
        port: Some(port),
        dir_pkg_id: None,
        dir_pkg_objid: None,
        block_services: vec![],
    })
}

fn build_static_web_app_host_entry(
    app_spec: &AppServiceSpec,
    guest_allowed: bool,
) -> Option<NodeGatewayAppInfoEntry> {
    let web_pkg = app_spec.app_doc.pkg_list.web.as_ref()?;
    let dir_pkg_id = PackageId::get_pkg_id_unique_name(web_pkg.pkg_id.as_str());
    let dir_pkg_objid = web_pkg.pkg_objid.as_ref().map(|objid| objid.to_string());

    Some(NodeGatewayAppInfoEntry {
        app_id: app_spec.app_id().clone(),
        app_instance_id: app_spec.app_instance_id().clone(),
        app_owner_user_id: app_spec.owner_user_id.clone(),
        sdk_version: parse_sdk_version(app_spec),
        deployment: Some(app_spec.deployment.clone()),
        access_mode: if guest_allowed {
            NodeGatewayAccessMode::Public
        } else {
            NodeGatewayAccessMode::Private
        },
        node_id: None,
        port: None,
        dir_pkg_id: Some(dir_pkg_id),
        dir_pkg_objid,
        block_services: vec![],
    })
}

fn app_guest_allowed(
    app_instance_id: &AppInstanceId,
    input_system_config: &HashMap<String, String>,
) -> bool {
    input_system_config
        .get(&app_availability_policy_key(app_instance_id))
        .and_then(|raw| serde_json::from_str::<AppAvailabilityPolicy>(raw).ok())
        .filter(|policy| policy.app_instance_id == *app_instance_id)
        .is_some_and(|policy| {
            policy
                .group_rules
                .iter()
                .any(|rule| rule.group_id == "guest" && rule.effect == AvailabilityEffect::Allow)
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
    boot_config: &ZoneConfig,
    zone_config: &ZoneDocument,
    device_list: &HashMap<String, DeviceInfo>,
) -> HashMap<String, String> {
    let mut trust_key = HashMap::new();

    if let Some(verify_hub_info) = boot_config.verify_hub_info.as_ref() {
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
    zone_config: &ZoneDocument,
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

pub(crate) async fn update_node_gateway_info(
    node_id: &str,
    scheduler_ctx: &NodeScheduler,
    input_system_config: &HashMap<String, String>,
) -> Result<HashMap<String, KVAction>> {
    let boot_config = get_boot_config(input_system_config)?;
    let zone_config = boot_config.zone_document()?;
    let zone_gateway_settings = get_zone_gateway_settings(input_system_config)?;
    let device_list = get_device_list(input_system_config)?;
    let zone_host = zone_config.id.to_host_name();
    let forward_plan = build_forward_plan(node_id, &zone_config, &zone_host, &device_list);

    let mut node_gateway_info = NodeGatewayInfo {
        node_info: NodeGatewayNodeInfo {
            this_node_id: node_id.to_string(),
            this_zone_host: zone_host.clone(),
        },
        app_info: HashMap::new(),
        service_info: HashMap::new(),
        node_route_map: build_node_route_map(node_id, &zone_host, &device_list),
        routes: forward_plan.routes,
        did_ip_hints: forward_plan.did_ip_hints,
        trust_key: build_trust_keys(node_id, &boot_config, &zone_config, &device_list),
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
                    if let Some(expose_config) = app_spec.spec_config.expose_config.get("www") {
                        let guest_allowed =
                            app_guest_allowed(app_spec.app_instance_id(), input_system_config);
                        let app_entry = build_app_host_entry(
                            &app_spec,
                            service_info,
                            service_name.as_str(),
                            guest_allowed,
                        )
                        .or_else(|| {
                            if !is_non_sdk_container_app_service(&app_spec) {
                                return None;
                            }
                            let persisted_service_info =
                                load_persisted_service_info(spec_id.as_str(), input_system_config)?;
                            build_app_host_entry_from_persisted_service_info(
                                &app_spec,
                                &persisted_service_info,
                                service_name.as_str(),
                                guest_allowed,
                            )
                        });

                        if let Some(app_entry) = app_entry {
                            for host in
                                zone_gateway_settings.get_shortcut(app_spec.app_instance_id())
                            {
                                node_gateway_info
                                    .app_info
                                    .insert(host, NodeGatewayAppEntry::App(app_entry.clone()));
                            }
                            for host in expose_config.sub_hostname() {
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
        let full_app_id = web_app.app_instance_id();
        let Some(expose_config) = web_app.spec_config.expose_config.get("www") else {
            continue;
        };
        let guest_allowed = app_guest_allowed(full_app_id, input_system_config);
        let Some(app_entry) = build_static_web_app_host_entry(&web_app, guest_allowed) else {
            continue;
        };

        for host in zone_gateway_settings.get_shortcut(full_app_id) {
            node_gateway_info
                .app_info
                .insert(host, NodeGatewayAppEntry::App(app_entry.clone()));
        }
        for host in expose_config.sub_hostname() {
            node_gateway_info
                .app_info
                .insert(host.clone(), NodeGatewayAppEntry::App(app_entry.clone()));
        }
    }

    for (path, value) in input_system_config {
        if !path.starts_with("users/") || !path.contains("/agents/") || !path.ends_with("/spec") {
            continue;
        }
        let agent_spec: AgentSpec = serde_json::from_str(value)?;
        agent_spec
            .validate()
            .map_err(|error| anyhow::anyhow!("invalid AgentSpec at {path}: {error}"))?;
        let target_id = agent_spec.binding.target_app_instance_id.to_string();
        let service_name = agent_spec.binding.service_name.as_str();
        let service_info_id = if service_name == "www" {
            target_id.clone()
        } else {
            format!("{target_id}:{service_name}")
        };
        let Some(service_info) = scheduler_ctx.service_infos.get(&service_info_id) else {
            continue;
        };
        let target_spec = get_app_spec_by_spec_id(&target_id, input_system_config)?;
        let guest_allowed = app_guest_allowed(target_spec.app_instance_id(), input_system_config);
        if let Some(entry) =
            build_app_host_entry(&target_spec, service_info, service_name, guest_allowed)
        {
            node_gateway_info.app_info.insert(
                agent_spec.agent_id.to_string(),
                NodeGatewayAppEntry::App(entry),
            );
        }
        if let Some(selector) = build_service_selector(service_info, service_name) {
            node_gateway_info.service_info.insert(
                agent_spec.agent_id.to_string(),
                NodeGatewayServiceInfoEntry { selector },
            );
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
    let device_list = get_device_list(input_system_config)?;
    let mut result = HashMap::new();

    for node_id in need_update_gateway_node_list.iter() {
        let mut node_gateway_json = json!({});

        if let Some(sn_host) = zone_config.sn.as_ref() {
            info!("SN enabled, add acme/tls stack for node {}", node_id);
            let device_info = device_list
                .get(node_id)
                .ok_or_else(|| anyhow::anyhow!("device info {} not found", node_id))?;
            let identity_paths = buckyos_api::device_identity_paths(&device_info.device_doc.id)
                .map_err(|err| anyhow::anyhow!("build device identity paths failed: {}", err))?;
            let sn_url = format!("https://{}/kapi/sn", sn_host);
            let zone_hostname = zone_config.id.to_host_name();
            let wildcard_zone_domain = format!("*.{}", zone_hostname);
            node_gateway_json = json!({
                "acme": {
                    "dns_providers": {
                        "sn-dns": {
                            "sn": sn_url,
                            "key_path": identity_paths.authentication_private_key.display().to_string(),
                            "device_config_path": identity_paths.did_json.display().to_string()
                        }
                    },
                    "hosts": [
                        {
                            "host": wildcard_zone_domain.clone(),
                            "challenge_type": "dns-01",
                            "dns_provider": "sn-dns"
                        },
                        {
                            "host": zone_hostname.clone(),
                            "challenge_type": "dns-01",
                            "dns_provider": "sn-dns"
                        }
                    ]
                },
                "stacks": {
                    "zone_tls": {
                        "bind": "[::]:443",
                        "protocol": "tls",
                        "hosts": [
                            wildcard_zone_domain,
                            zone_hostname
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
    let current_rbac_policy = input_config.get("system/rbac/policy");
    let mut rbac_policy = String::new();
    fn push_policy_line(policy: &mut String, line: String) {
        if !policy.is_empty() {
            policy.push('\n');
        }
        policy.push_str(line.as_str());
    }

    for (user_id, user_item) in scheduler_ctx.users.iter() {
        if user_id == "root" {
            continue;
        }
        match user_item.user_type {
            crate::scheduler::UserType::Admin => {
                info!("add admin rbac policy for user {}", user_id);
                push_policy_line(&mut rbac_policy, format!("g, {}, admin", user_id));
                push_policy_line(&mut rbac_policy, format!("g, su_{}, su_admin", user_id));
            }
            crate::scheduler::UserType::User => {
                info!("add users rbac policy for user {}", user_id);
                push_policy_line(&mut rbac_policy, format!("g, {}, users", user_id));
                let sudo_user_id = format!("su_{}", user_id);
                push_policy_line(&mut rbac_policy, format!("g, {}, su_users", sudo_user_id));
                push_policy_line(
                    &mut rbac_policy,
                    format!(
                        "p, {}, obj://config/users/{}/settings, read|write,allow",
                        sudo_user_id, user_id
                    ),
                );
                push_policy_line(
                    &mut rbac_policy,
                    format!(
                        "p, {}, obj://config/users/{}/doc, read|write,allow",
                        sudo_user_id, user_id
                    ),
                );
            }
            crate::scheduler::UserType::Limited => {
                info!("add limited rbac policy for user {}", user_id);
                push_policy_line(&mut rbac_policy, format!("g, {}, limited", user_id));
            }
            _ => {
                continue;
            }
        }
    }

    for (node_id, node_item) in scheduler_ctx.nodes.iter() {
        match node_item.node_type {
            NodeType::OOD => {
                push_policy_line(&mut rbac_policy, format!("g, {}, ood", node_id));
            }
            NodeType::Server => {
                push_policy_line(&mut rbac_policy, format!("g, {}, node", node_id));
            }
            _ => {
                continue;
            }
        }
    }

    for (spec_id, service_spec) in scheduler_ctx.specs.iter() {
        match service_spec.spec_type {
            ServiceSpecType::App => {
                push_policy_line(
                    &mut rbac_policy,
                    format!("g, app:{}, app", service_spec.app_id),
                );
            }
            ServiceSpecType::Service => {
                push_policy_line(
                    &mut rbac_policy,
                    format!("g, system:{}, system", service_spec.app_id),
                );
            }
            ServiceSpecType::Kernel => {
                push_policy_line(
                    &mut rbac_policy,
                    format!("g, system:{}, kernel", service_spec.app_id),
                );
            }
        }
    }

    for (path, value) in input_config {
        if !path.starts_with("users/") || !path.contains("/agents/") || !path.ends_with("/spec") {
            continue;
        }
        let agent_spec: AgentSpec = serde_json::from_str(value)?;
        agent_spec
            .validate()
            .map_err(|error| anyhow::anyhow!("invalid AgentSpec at {path}: {error}"))?;
        push_policy_line(
            &mut rbac_policy,
            format!("g, {}, agent", agent_spec.agent_did.to_string()),
        );
        push_policy_line(
            &mut rbac_policy,
            format!(
                "g, app:{}, agent_runtime",
                agent_spec.binding.target_app_instance_id.app_id()
            ),
        );
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

async fn load_scheduler_input_config(
    system_config_client: &SystemConfigClient,
) -> Result<HashMap<String, String>> {
    let input_system_config = system_config_client.dump_configs_for_scheduler().await?;
    let input_system_config = serde_json::from_value(input_system_config)?;
    validate_beta22_app_state(&input_system_config)?;
    Ok(input_system_config)
}

fn validate_beta22_app_state(input: &HashMap<String, String>) -> Result<()> {
    let registry_raw = input.get(APP_REGISTRY_KEY).ok_or_else(|| {
        anyhow::anyhow!(
            "beta 2.2 requires an initialized system/app_registry; rebuild SystemConfig from empty state"
        )
    })?;
    let registry: AppRegistry = serde_json::from_str(registry_raw)?;
    let mut reserved = BTreeSet::from(["_".to_string(), "www".to_string(), "sys".to_string()]);
    if let Some(raw) = input.get("services/gateway/settings") {
        let settings: ZoneGatewaySettings = serde_json::from_str(raw)?;
        reserved.extend(settings.shortcuts.keys().cloned());
    }
    registry.validate(&reserved).map_err(anyhow::Error::msg)?;

    for (path, raw) in input {
        let parts = path.split('/').collect::<Vec<_>>();
        if matches!(parts.as_slice(), ["users", _, "apps", _, "spec"]) {
            let spec: AppServiceSpec = serde_json::from_str(raw)?;
            let owner = parts[1];
            let app_id = AppId::parse(parts[3]).map_err(anyhow::Error::msg)?;
            if spec.owner_user_id != owner
                || spec.app_id() != &app_id
                || spec.app_instance_id.owner_user_id() != owner
            {
                return Err(anyhow::anyhow!("invalid AppSpec identity at `{path}`"));
            }
            let app_allocation = registry.apps.get(&app_id).ok_or_else(|| {
                anyhow::anyhow!("AppSpec `{path}` has no AppRegistry app allocation")
            })?;
            let instance_allocation =
                registry
                    .instances
                    .get(&spec.app_instance_id)
                    .ok_or_else(|| {
                        anyhow::anyhow!("AppSpec `{path}` has no AppRegistry instance allocation")
                    })?;
            if app_allocation.app_did != spec.app_did
                || app_allocation.app_name != spec.app_name
                || instance_allocation.app_host_name != spec.app_host_name
                || instance_allocation.app_index != spec.app_index
            {
                return Err(anyhow::anyhow!(
                    "AppSpec `{path}` contains stale scheduler allocation projections"
                ));
            }
        } else if matches!(parts.as_slice(), ["users", _, "agents", _, "spec"]) {
            let spec: AgentSpec = serde_json::from_str(raw)?;
            spec.validate().map_err(anyhow::Error::msg)?;
            if spec.agent_id.as_str() != parts[3] {
                return Err(anyhow::anyhow!("invalid AgentSpec key at `{path}`"));
            }
        } else if matches!(parts.as_slice(), ["nodes", _, "config"]) {
            let node_config: NodeConfig = serde_json::from_str(raw)?;
            for (app_instance_id, config) in &node_config.apps {
                if app_instance_id != &config.node_execution_spec.app_instance_id {
                    return Err(anyhow::anyhow!(
                        "NodeConfig `{path}` App map key does not equal AppInstanceId"
                    ));
                }
                config
                    .node_execution_spec
                    .validate_against(&config.deployment)
                    .map_err(anyhow::Error::msg)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod beta22_state_tests {
    use super::*;

    #[test]
    fn missing_registry_fails_closed() {
        let error = validate_beta22_app_state(&HashMap::new()).unwrap_err();
        assert!(error
            .to_string()
            .contains("rebuild SystemConfig from empty state"));
    }

    #[test]
    fn unknown_registry_schema_fails_closed() {
        let mut registry = AppRegistry::default();
        registry.schema_version += 1;
        let input = HashMap::from([(
            APP_REGISTRY_KEY.to_string(),
            serde_json::to_string(&registry).unwrap(),
        )]);
        assert!(validate_beta22_app_state(&input).is_err());
    }

    #[test]
    fn guest_gateway_access_comes_only_from_matching_availability_policy() {
        let app_instance_id =
            AppInstanceId::new(AppId::parse("notes.example").unwrap(), "alice").unwrap();
        let mut policy = AppAvailabilityPolicy::owner_default(app_instance_id.clone());
        policy.group_rules.push(AppAvailabilityGroupRule {
            group_id: "guest".to_string(),
            effect: AvailabilityEffect::Allow,
        });
        let policy_key = app_availability_policy_key(&app_instance_id);
        let input = HashMap::from([(policy_key.clone(), serde_json::to_string(&policy).unwrap())]);
        assert!(app_guest_allowed(&app_instance_id, &input));

        let other_instance =
            AppInstanceId::new(AppId::parse("notes.example").unwrap(), "bob").unwrap();
        assert!(!app_guest_allowed(&other_instance, &input));

        let invalid = HashMap::from([(policy_key, "not-json".to_string())]);
        assert!(!app_guest_allowed(&app_instance_id, &invalid));
    }
}

fn update_managed_sn_ai_provider(
    input_system_config: &HashMap<String, String>,
) -> Result<HashMap<String, KVAction>> {
    const AICC_SETTINGS_KEY: &str = "services/aicc/settings";
    let Some(current_settings) = input_system_config.get(AICC_SETTINGS_KEY) else {
        return Ok(HashMap::new());
    };
    let current_settings: Value = serde_json::from_str(current_settings)?;
    if current_settings.get("sn-ai-provider").is_none() {
        return Ok(HashMap::new());
    }

    let endpoints = (|| -> Result<_> {
        let boot_config = input_system_config
            .get("boot/config")
            .ok_or_else(|| anyhow::anyhow!("boot/config is missing"))?;
        let zone_config: ZoneConfig = serde_json::from_str(boot_config)?;
        let zone_document = zone_config
            .zone_document()
            .map_err(|err| anyhow::anyhow!("decode ZoneDocument from boot/config failed: {err}"))?;
        derive_sn_ai_provider_endpoints(zone_document.sn.as_deref())
    })();

    let reconciled = match &endpoints {
        Ok(endpoints) => reconcile_managed_sn_ai_provider(&current_settings, Ok(endpoints))?,
        Err(err) => {
            warn!(
                "disable managed SN AI provider because Zone SN endpoint is invalid: {}",
                err
            );
            reconcile_managed_sn_ai_provider(&current_settings, Err(err))?
        }
    };
    let Some(reconciled) = reconciled else {
        return Ok(HashMap::new());
    };

    let mut actions = HashMap::new();
    actions.insert(
        AICC_SETTINGS_KEY.to_string(),
        KVAction::Update(serde_json::to_string_pretty(&reconciled)?),
    );
    Ok(actions)
}

pub(crate) async fn refresh_rbac() -> Result<SchedulerRefreshRbacResponse> {
    let buckyos_api_runtime = get_buckyos_api_runtime()?;
    let system_config_client = buckyos_api_runtime.get_system_config_client().await?;
    let input_system_config = load_scheduler_input_config(&system_config_client).await?;
    let (scheduler_ctx, _) = create_scheduler_by_system_config(&input_system_config)?;
    let tx_actions = update_rbac(&input_system_config, &scheduler_ctx).await?;
    let tx_action_count = tx_actions.len();

    if tx_action_count > 0 {
        let tx_action_keys = collect_tx_action_keys(&tx_actions);
        system_config_client.exec_tx(tx_actions, None).await?;
        info!(
            "refresh_rbac applied {} actions, keys={:?}",
            tx_action_count, tx_action_keys
        );
    } else {
        info!("refresh_rbac skipped, RBAC policy is already current");
    }

    Ok(SchedulerRefreshRbacResponse {
        updated: tx_action_count > 0,
        tx_action_count,
    })
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

    let (app_instance_spec_actions, refreshed_app_nodes) =
        reconcile_app_instance_specs(input_system_config)?;
    extend_kv_action_map(&mut tx_actions, &app_instance_spec_actions);
    need_update_gateway_node_list.extend(refreshed_app_nodes);

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

    let aicc_actions = update_managed_sn_ai_provider(input_system_config)?;
    extend_kv_action_map(&mut tx_actions, &aicc_actions);

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

pub async fn schedule_loop(is_boot: bool, run_once: bool) -> Result<()> {
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
        let input_system_config = load_scheduler_input_config(&system_config_client).await;
        if input_system_config.is_err() {
            error!(
                "load_scheduler_input_config failed: {:?}",
                input_system_config.err().unwrap()
            );
            continue;
        }
        let input_system_config = input_system_config.unwrap();
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
        if run_once {
            break;
        }
    }
    Ok(())
}
