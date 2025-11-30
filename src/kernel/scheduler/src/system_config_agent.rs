use std::collections::HashMap;

use anyhow::Result;
use log::*;
use rbac::DEFAULT_POLICY;
use serde_json::json;

use crate::app::*;
use crate::scheduler::*;
use crate::service::*;
use buckyos_api::{
    get_buckyos_api_runtime, AppServiceSpec, KernelServiceSpec, NodeConfig,
    ServiceInstanceReportInfo, UserSettings, UserType as ApiUserType,
};
use buckyos_kit::*;
use name_client::*;
use name_lib::{DeviceInfo, ZoneConfig};

fn map_api_user_type(user_type: &ApiUserType) -> UserType {
    match user_type {
        ApiUserType::Admin | ApiUserType::Root => UserType::Admin,
        ApiUserType::Limited => UserType::Limited,
        _ => UserType::User,
    }
}

fn craete_node_item_by_device_info(device_name: &str, device_info: &DeviceInfo) -> NodeItem {
    let node_state = crate::scheduler::NodeState::from(
        device_info.state.clone().unwrap_or("Ready".to_string()),
    );
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
    let mut need_container = true;
    if app_config
        .app_doc
        .pkg_list
        .iter()
        .into_iter()
        .any(|(_, pkg)| pkg.docker_image_name.is_none())
        &&
        (app_config.app_doc.author == "did:web:buckyos.ai"
            || app_config.app_doc.author == "did:web:buckyos.io"
            || app_config.app_doc.author == "did:web:buckyos.org")
    {
        need_container = false;
    }

    ServiceSpec {
        id: full_app_id.to_string(),
        app_id: app_config.app_id().to_string(),
        owner_id: owner_user_id.to_string(),
        spec_type: ServiceSpecType::App,
        default_service_port: 0,
        state: spec_state,
        need_container,
        best_instance_count: app_config.expected_instance_count,
        required_cpu_mhz: 200,
        required_memory: 1024 * 1024 * 256,
        required_gpu_tflops: 0.0,
        required_gpu_mem: 0,
        node_affinity: None,
        network_affinity: None,
    }
}

fn create_service_spec_by_service_config(
    service_name: &str,
    service_config: &KernelServiceSpec,
) -> ServiceSpec {
    let spec_state = ServiceSpecState::from(service_config.state.clone());
    let default_service_port = service_config
        .install_config
        .service_ports
        .get("main")
        .copied()
        .or_else(|| service_config.install_config.service_ports.values().next().copied())
        .unwrap_or(0);
    ServiceSpec {
        id: service_name.to_string(),
        app_id: service_name.to_string(),
        owner_id: "root".to_string(),
        spec_type: ServiceSpecType::Kernel,
        state: spec_state,
        default_service_port,
        need_container: false,
        best_instance_count: service_config.expected_instance_count,
        required_cpu_mhz: 300,
        required_memory: 1024 * 1024 * 256,
        required_gpu_tflops: 0.0,
        required_gpu_mem: 0,
        node_affinity: None,
        network_affinity: None,
    }
}

fn create_scheduler_by_system_config(
    input_config: &HashMap<String, String>,
) -> Result<(NodeScheduler, HashMap<String, DeviceInfo>)> {
    let mut scheduler_ctx = NodeScheduler::new_empty(1, buckyos_get_unix_timestamp());
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
            if key.ends_with("/config") {
                let parts: Vec<&str> = key.split('/').collect();
                if parts.len() >= 4 && parts[2] == "apps" {
                    let user_id = parts[1];
                    let app_id = parts[3];
                    let full_appid = format!("{}@{}", app_id, user_id);
                    let app_config: AppServiceSpec = serde_json::from_str(value.as_str()).map_err(|e| {
                        error!(
                            "AppConfig serde_json::from_str failed: {:?} {}",
                            e,
                            value.as_str()
                        );
                        e
                    })?;
                    let service_spec =
                        create_service_spec_by_app_config(full_appid.as_str(), user_id, &app_config);
                    scheduler_ctx.add_service_spec(service_spec);
                }
            } else if key.ends_with("/settings") {
                let parts: Vec<&str> = key.split('/').collect();
                if parts.len() >= 3 {
                    let user_id = parts[1];
                    let user_settings: UserSettings =
                        serde_json::from_str(value.as_str()).map_err(|e| {
                            error!("UserSettings serde_json::from_str failed: {:?}", e);
                            e
                        })?;
                    let user_item = UserItem {
                        userid: user_id.to_string(),
                        res_pool_id: None,
                        user_type: map_api_user_type(&user_settings.user_type),
                    };
                    scheduler_ctx.add_user(user_item);
                }
            }
        }

        //add service service_spec
        if key.starts_with("services/") && key.ends_with("/config") {
            let service_name = key.split('/').nth(1).unwrap();
            let service_config: KernelServiceSpec =
                serde_json::from_str(value.as_str()).map_err(|e| {
                    error!("KernelServiceConfig serde_json::from_str failed: {:?}", e);
                    e
                })?;
            let service_spec =
                create_service_spec_by_service_config(service_name, &service_config);
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
                    service_port: app_config
                        .get_host_service_port("www")
                        .or_else(|| {
                            app_config
                                .node_install_config
                                .as_ref()
                                .and_then(|cfg| cfg.service_ports.values().next().copied())
                        })
                        .unwrap_or(0),
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
            let instance_info: ServiceInstanceReportInfo =
                serde_json::from_str(value.as_str()).map_err(|e| {
                    error!("ServiceInstanceInfo serde_json::from_str failed: {:?}", e);
                    e
                })?;
            let reported_port = instance_info
                .service_ports
                .get("main")
                .copied()
                .or_else(|| instance_info.service_ports.values().next().copied())
                .unwrap_or(0);
            let instance = ReplicaInstance {
                spec_id: service_name.to_string(),
                node_id: instance_node_id.to_string(),
                res_limits: HashMap::new(),
                instance_id: instance_info.instance_id.clone(),
                last_update_time: instance_info.last_update_time,
                state: InstanceState::from(instance_info.state.clone()),
                service_port: reported_port,
            };
            scheduler_ctx.add_replica_instance(instance);
        }
    }

    Ok((scheduler_ctx, device_list))
}

fn schedule_action_to_tx_actions(
    action: &SchedulerAction,
    scheduler_ctx: &NodeScheduler,
    device_list: &HashMap<String, DeviceInfo>,
    input_config: &HashMap<String, String>,
) -> Result<HashMap<String, KVAction>> {
    let mut result = HashMap::new();
    let zone_config = input_config.get("boot/config");
    if zone_config.is_none() {
        return Err(anyhow::anyhow!("zone_config not found"));
    }
    let zone_config = zone_config.unwrap();
    let zone_config: ZoneConfig = serde_json::from_str(zone_config.as_str())?;
    let zone_gateway = zone_config.zone_gateway;
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
                    let set_state_action = set_app_service_state(spec_id.as_str(), spec_status)?;
                    info!("will change app service status: {} -> {}", spec_id, spec_status);
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
            match service_spec.spec_type {
                ServiceSpecType::App => {
                    let instance_action =
                        instance_app_service(new_instance, &device_list, &input_config)?;
                    info!("will instance app pod: {}", new_instance.spec_id);
                    result.extend(instance_action);
                }
                ServiceSpecType::Service | ServiceSpecType::Kernel => {
                    let service_config = input_config
                        .get(format!("services/{}/config", service_spec.id.as_str()).as_str());
                    if service_config.is_none() {
                        return Err(anyhow::anyhow!(
                            "service_config {} not found",
                            service_spec.id.as_str()
                        ));
                    }
                    let service_config = service_config.unwrap();
                    let service_config: KernelServiceSpec =
                        serde_json::from_str(service_config.as_str())?;
                    let is_zone_gateway = zone_gateway.contains(&new_instance.node_id);
                    let instance_action =
                        instance_service(new_instance, &service_config, is_zone_gateway)?;
                    info!("will instance service pod: {}", new_instance.spec_id);
                    result.extend(instance_action);
                }
            }
        }
        SchedulerAction::RemoveInstance(spec_id, instance_id, _node_id) => {
            let service_spec = scheduler_ctx.get_service_spec(spec_id.as_str());
            if service_spec.is_none() {
                return Err(anyhow::anyhow!("service_spec not found"));
            }
            let service_spec = service_spec.unwrap();
            let instance = scheduler_ctx.get_replica_instance(instance_id.as_str());
            if instance.is_none() {
                return Err(anyhow::anyhow!("instance not found"));
            }
            let instance = instance.unwrap();
            match service_spec.spec_type {
                ServiceSpecType::App => {
                    info!("will uninstance app service: {}", instance.spec_id);
                    let uninstance_action = uninstance_app_service(instance)?;
                    result.extend(uninstance_action);
                }
                ServiceSpecType::Service | ServiceSpecType::Kernel => {
                    info!("will uninstance service: {}", instance.spec_id);
                    let uninstance_action = uninstance_service(instance)?;
                    result.extend(uninstance_action);
                }
            }
        }
        SchedulerAction::UpdateInstance(instance_id, instance) => {
            //相对比较复杂的操作:需要根据service_spec的类型,来执行更新实例化操作
            let (spec_id, _owner_id, _node_id) = parse_instance_id(instance_id.as_str())?;
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
            let update_action = update_service_info(spec_id.as_str(), service_info, device_list)?;
            info!("will update service info: {}", spec_id);
            result.extend(update_action);
        }
    }
    Ok(result)
}

async fn update_rbac(
    input_config: &HashMap<String, String>,
    scheduler_ctx: &NodeScheduler,
) -> Result<HashMap<String, KVAction>> {
    let basic_rbac_policy = input_config.get("system/rbac/basic_policy");
    let current_rbac_policy = input_config.get("system/rbac/policy");
    let mut rbac_policy = String::new();
    if basic_rbac_policy.is_none() {
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
                rbac_policy.push_str(&format!("\ng, {}, service", spec_id));
            }
            // ServiceSpecType::Kernel => {
            //     kernel service already set in basic_policy
            //     rbac_policy.push_str(&format!("\ng, {}, kernel", pod_id));
            // }
            _ => {
                continue;
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
    result.insert("system/rbac/policy".to_string(), KVAction::Update(rbac_policy));

    Ok(result)
}


pub async fn schedule_loop(is_boot: bool) -> Result<()> {
    let mut loop_step = 0;
    let is_running = true;
    let mut need_update_rbac = false;
    //info!("schedule loop start...");
    loop {
        if !is_running {
            break;
        }
        need_update_rbac = false;
        if is_boot || loop_step % 10 == 0 {
            need_update_rbac = true;
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
        let input_config = system_config_client.dump_configs_for_scheduler().await;
        if input_config.is_err() {
            error!(
                "dump_configs_for_scheduler failed: {:?}",
                input_config.err().unwrap()
            );
            continue;
        }
        let input_config = input_config.unwrap();
        //cover value to hashmap
        let input_config = serde_json::from_value(input_config);
        if input_config.is_err() {
            error!(
                "serde_json::from_value failed: {:?}",
                input_config.err().unwrap()
            );
            continue;
        }
        let input_config = input_config.unwrap();

        //init scheduler
        let (mut scheduler_ctx, device_list) = create_scheduler_by_system_config(&input_config)?;

        //schedule
        let action_list = scheduler_ctx.schedule();
        if action_list.is_err() {
            error!(
                "scheduler.schedule failed: {:?}",
                action_list.err().unwrap()
            );
            return Err(anyhow::anyhow!("scheduler.schedule failed"));
        }

        let action_list = action_list.unwrap();
        let mut tx_actions = HashMap::new();
        for action in action_list {
            let new_tx_actions = schedule_action_to_tx_actions(
                &action,
                &scheduler_ctx,
                &device_list,
                &input_config,
            )?;
            extend_kv_action_map(&mut tx_actions, &new_tx_actions);
        }

        if need_update_rbac {
            let rbac_actions = update_rbac(&input_config, &scheduler_ctx).await?;
            extend_kv_action_map(&mut tx_actions, &rbac_actions);
        }

        //执行调度动作
        let ret = system_config_client.exec_tx(tx_actions, None).await;
        if ret.is_err() {
            error!("exec_tx failed: {:?}", ret.err().unwrap());
        }
        if is_boot {
            break;
        }
    }
    Ok(())
}
