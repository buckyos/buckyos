#![allow(unused_mut, unused, dead_code)]
mod app;
mod scheduler;
mod service;

use log::*;
use serde_json::json;
use serde_json::Value;
use std::collections::HashMap;
use std::process::exit;
//use upon::Engine;

use app::*;
use buckyos_api::*;
use buckyos_api::*;
use buckyos_kit::*;
use name_client::*;
use name_lib::*;
use scheduler::*;
use service::*;

use anyhow::Result;

async fn create_init_list_by_template() -> Result<HashMap<String, String>> {
    //load start_parms from active_service.
    let start_params_file_path = get_buckyos_system_etc_dir().join("start_config.json");
    info!(
        "load start_params from :{}",
        start_params_file_path.to_string_lossy()
    );
    let start_params_str = tokio::fs::read_to_string(start_params_file_path).await?;
    let mut start_params: serde_json::Value = serde_json::from_str(&start_params_str)?;

    let template_type_str = "boot".to_string();
    let template_file_path = get_buckyos_system_etc_dir()
        .join("scheduler")
        .join(format!("{}.template.toml", template_type_str));
    let template_str = tokio::fs::read_to_string(template_file_path).await?;

    //generate dynamic params
    let (private_key_pem, public_key_jwk) = generate_ed25519_key_pair();
    start_params["verify_hub_key"] = json!(private_key_pem);
    start_params["verify_hub_public_key"] = json!(public_key_jwk.to_string());
    
    // 将Windows路径中的反斜杠转换为正斜杠，避免TOML转义问题
    let buckyos_root = get_buckyos_root_dir().to_string_lossy().to_string().replace('\\', "/");
    start_params["BUCKYOS_ROOT"] = json!(buckyos_root);

    let mut engine = upon::Engine::new();
    engine.add_template("config", &template_str)?;
    let result = engine
        .template("config")
        .render(&start_params)
        .to_string()?;

    if result.find("{{").is_some() {
        return Err(anyhow::anyhow!(
            "template contains unescaped double curly braces"
        ));
    }

    //wwrite result to file
    //let result_file_path = get_buckyos_system_etc_dir().join("scheduler_boot.toml");
    //tokio::fs::write(result_file_path, result.clone()).await?;

    let config: HashMap<String, String> = toml::from_str(&result)?;

    Ok(config)
}

async fn do_boot_scheduler() -> Result<()> {
    let mut init_list: HashMap<String, String> = HashMap::new();
    let zone_boot_config_str = std::env::var("BUCKYOS_ZONE_BOOT_CONFIG");

    if zone_boot_config_str.is_err() {
        warn!("BUCKYOS_ZONE_BOOT_CONFIG is not set, use default zone config");
        return Err(anyhow::anyhow!("BUCKYOS_ZONE_BOOT_CONFIG is not set"));
    }

    info!(
        "zone_boot_config_str:{}",
        zone_boot_config_str.as_ref().unwrap()
    );
    let zone_boot_config: ZoneBootConfig =
        serde_json::from_str(&zone_boot_config_str.unwrap()).unwrap();
    let rpc_session_token_str = std::env::var("SCHEDULER_SESSION_TOKEN");

    if rpc_session_token_str.is_err() {
        return Err(anyhow::anyhow!("SCHEDULER_SESSION_TOKEN is not set"));
    }

    let rpc_session_token = rpc_session_token_str.unwrap();
    let system_config_client = SystemConfigClient::new(None, Some(rpc_session_token.as_str()));
    let boot_config = system_config_client.get("boot/config").await;
    if boot_config.is_ok() {
        return Err(anyhow::anyhow!(
            "boot/config already exists, boot scheduler failed"
        ));
    }

    let mut init_list = create_init_list_by_template().await.map_err(|e| {
        error!("create_init_list_by_template failed: {:?}", e);
        e
    })?;

    let boot_config_str = init_list.get("boot/config");
    if boot_config_str.is_none() {
        return Err(anyhow::anyhow!("boot/config not found in init list"));
    }
    let boot_config_str = boot_config_str.unwrap();
    let mut zone_config: ZoneConfig = zone_boot_config.to_zone_config();
    zone_config.init_by_boot_config(&zone_boot_config);
    init_list.insert(
        "boot/config".to_string(),
        serde_json::to_string_pretty(&zone_config).unwrap(),
    );
    //info!("use init list from template {} to do boot scheduler",template_type_str);
    //write to system_config
    for (key, value) in init_list.iter() {
        system_config_client.create(key, value).await?;
    }

    info!("do first schedule!");
    let boot_result = schedule_loop(true).await;
    if boot_result.is_err() {
        error!("schedule_loop failed: {:?}", boot_result.err().unwrap());
        return Err(anyhow::anyhow!("schedule_loop failed"));
    }
    system_config_client.refresh_trust_keys().await?;
    info!("system_config_service refresh trust keys success");
    
    info!("boot scheduler success");
    return Ok(());
}

fn craete_node_item_by_device_info(device_name: &str, device_info: &DeviceInfo) -> NodeItem {
    let node_state = NodeState::from(device_info.state.clone().unwrap_or("Ready".to_string()));
    let net_id = device_info.net_id.clone().unwrap_or("".to_string());
    NodeItem {
        id: device_name.to_string(),
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

fn create_pod_item_by_app_config(app_id: &str, app_config: &AppConfig) -> PodItem {
    let pod_state = PodItemState::from(app_config.state.clone());
    let mut need_container = true;
    if app_config.app_doc.pkg_list.iter().any(|(_, pkg)| pkg.docker_image_name.is_none()) &&
       //TODO: 需要从配置中获取所有的可信发布商列表
       app_config.app_doc.author == "did:web:buckyos.ai"
        || app_config.app_doc.author == "did:web:buckyos.io"
        || app_config.app_doc.author == "did:web:buckyos.org"
    {
        need_container = false;
    }

    PodItem {
        id: app_id.to_string(),
        pod_type: PodItemType::App,
        state: pod_state,
        need_container: need_container,
        best_instance_count: app_config.instance,
        required_cpu_mhz: 200,
        required_memory: 1024 * 1024 * 256,
        required_gpu_tflops: 0.0,
        required_gpu_mem: 0,
        node_affinity: None,
        network_affinity: None,
    }
}

fn create_pod_item_by_service_config(
    service_name: &str,
    service_config: &KernelServiceConfig,
) -> PodItem {
    let pod_state = PodItemState::from(service_config.state.clone());
    PodItem {
        id: service_name.to_string(),
        pod_type: PodItemType::Service,
        state: pod_state,
        need_container: false,
        best_instance_count: service_config.instance,
        required_cpu_mhz: 300,
        required_memory: 1024 * 1024 * 256,
        required_gpu_tflops: 0.0,
        required_gpu_mem: 0,
        node_affinity: None,
        network_affinity: None,
    }
}

fn create_scheduler_by_input_config(
    input_config: &HashMap<String, String>,
) -> Result<(PodScheduler, HashMap<String, DeviceInfo>)> {
    let mut pod_scheduler = PodScheduler::new_empty(1, buckyos_get_unix_timestamp());
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
            device_list.insert(device_name.to_string(), device_info);
            pod_scheduler.add_node(node_item);
        }

        //add app pod
        if key.starts_with("users/") && key.ends_with("/config") {
            let parts: Vec<&str> = key.split('/').collect();
            if parts.len() >= 4 && parts[2] == "apps" {
                let user_id = parts[1];
                let app_id = parts[3];
                let full_appid = format!("{}@{}", app_id, user_id);
                let app_config: AppConfig = serde_json::from_str(value.as_str()).map_err(|e| {
                    error!(
                        "AppConfig serde_json::from_str failed: {:?} {}",
                        e,
                        value.as_str()
                    );
                    e
                })?;
                let pod_item = create_pod_item_by_app_config(full_appid.as_str(), &app_config);
                pod_scheduler.add_pod(pod_item);
            }
        }

        //add service pod
        if key.starts_with("services/") && key.ends_with("/config") {
            let service_name = key.split('/').nth(1).unwrap();
            let service_config: KernelServiceConfig = serde_json::from_str(value.as_str())
                .map_err(|e| {
                    error!("KernelServiceConfig serde_json::from_str failed: {:?}", e);
                    e
                })?;
            let pod_item = create_pod_item_by_service_config(service_name, &service_config);
            pod_scheduler.add_pod(pod_item);
        }

        //add pod_instance
        //if key.starts_with("nodes/") && key.ends_with("/config") {
        //    let node_id = key.split('/').nth(1).unwrap();
        //}
    }

    Ok((pod_scheduler, device_list))
}

fn schedule_action_to_tx_actions(
    action: &SchedulerAction,
    pod_scheduler: &PodScheduler,
    device_list: &HashMap<String, DeviceInfo>,
    input_config: &HashMap<String, String>,
) -> Result<HashMap<String, KVAction>> {
    let mut result = HashMap::new();
    match action {
        SchedulerAction::ChangeNodeStatus(node_id, node_status) => {
            let key = format!("nodes/{}/config", node_id);
            let mut set_paths = HashMap::new();
            set_paths.insert("state".to_string(), Some(json!(node_status.to_string())));
            //TODO:需要将insert替换成合并
            result.insert(key, KVAction::SetByJsonPath(set_paths));
        }
        SchedulerAction::ChangePodStatus(pod_id, pod_status) => {
            let pod_item = pod_scheduler.get_pod_item(pod_id.as_str());
            if pod_item.is_none() {
                return Err(anyhow::anyhow!("pod_item not found"));
            }
            let pod_item = pod_item.unwrap();
            match pod_item.pod_type {
                PodItemType::App => {
                    let set_state_action = set_app_service_state(pod_id.as_str(), pod_status)?;
                    result.extend(set_state_action);
                }
                PodItemType::Service => {
                    let set_state_action = set_service_state(pod_id.as_str(), pod_status)?;
                    result.extend(set_state_action);
                }
            }
        }
        SchedulerAction::CreateOPTask(new_op_task) => {
            //TODO:
            unimplemented!();
        }
        SchedulerAction::InstancePod(new_instance) => {
            //最复杂的流程,需要根据pod的类型,来执行实例化操作
            let pod_item = pod_scheduler.get_pod_item(new_instance.pod_id.as_str());
            if pod_item.is_none() {
                return Err(anyhow::anyhow!("pod_item not found"));
            }
            let pod_item = pod_item.unwrap();
            match pod_item.pod_type {
                PodItemType::App => {
                    let instance_action =
                        instance_app_service(new_instance, &device_list, &input_config)?;
                    result.extend(instance_action);
                }
                PodItemType::Service => {
                    let service_config = input_config
                        .get(format!("services/{}/config", pod_item.id.as_str()).as_str());
                    if service_config.is_none() {
                        return Err(anyhow::anyhow!(
                            "service_config {} not found",
                            pod_item.id.as_str()
                        ));
                    }
                    let service_config = service_config.unwrap();
                    let service_config: KernelServiceConfig =
                        serde_json::from_str(service_config.as_str())?;
                    let instance_action = instance_service(new_instance, &service_config)?;
                    result.extend(instance_action);
                }
            }
        }
        SchedulerAction::RemovePodInstance(instance_id) => {
            let (pod_id, node_id) = parse_instance_id(instance_id.as_str())?;
            let pod_item = pod_scheduler.get_pod_item(pod_id.as_str());
            if pod_item.is_none() {
                return Err(anyhow::anyhow!("pod_item not found"));
            }
            let pod_item = pod_item.unwrap();
            let pod_instance = pod_scheduler.get_pod_instance(instance_id.as_str());
            if pod_instance.is_none() {
                return Err(anyhow::anyhow!("pod_instance not found"));
            }
            let pod_instance = pod_instance.unwrap();
            match pod_item.pod_type {
                PodItemType::App => {
                    let uninstance_action = uninstance_app_service(&pod_instance)?;
                    result.extend(uninstance_action);
                }
                PodItemType::Service => {
                    let uninstance_action = uninstance_service(&pod_instance)?;
                    result.extend(uninstance_action);
                }
            }
        }
        SchedulerAction::UpdatePodInstance(instance_id, pod_instance) => {
            //相对比较复杂的操作:需要根据pod的类型,来执行更新实例化操作
            let (pod_id, node_id) = parse_instance_id(instance_id.as_str())?;
            let pod_item = pod_scheduler.get_pod_item(pod_id.as_str());
            if pod_item.is_none() {
                return Err(anyhow::anyhow!("pod_item not found"));
            }
            let pod_item = pod_item.unwrap();
            match pod_item.pod_type {
                PodItemType::App => {
                    let update_action = update_app_service_instance(&pod_instance)?;
                    result.extend(update_action);
                }
                PodItemType::Service => {
                    let update_action = update_service_instance(&pod_instance)?;
                    result.extend(update_action);
                }
            }
        }
    }
    Ok(result)
}

async fn schedule_loop(is_boot: bool) -> Result<()> {
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
        let rpc_session_token_str = std::env::var("SCHEDULER_SESSION_TOKEN");
        if rpc_session_token_str.is_err() {
            return Err(anyhow::anyhow!("SCHEDULER_SESSION_TOKEN is not set"));
        }

        let rpc_session_token = rpc_session_token_str.unwrap();
        let system_config_client = SystemConfigClient::new(None, Some(rpc_session_token.as_str()));
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
        let (mut pod_scheduler, device_list) = create_scheduler_by_input_config(&input_config)?;

        //schedule
        let action_list = pod_scheduler.schedule();
        if action_list.is_err() {
            error!(
                "pod_scheduler.schedule failed: {:?}",
                action_list.err().unwrap()
            );
            return Err(anyhow::anyhow!("pod_scheduler.schedule failed"));
        }

        let action_list = action_list.unwrap();
        let mut tx_actions = HashMap::new();
        for action in action_list {
            let new_tx_actions = schedule_action_to_tx_actions(
                &action,
                &pod_scheduler,
                &device_list,
                &input_config,
            )?;
            extend_kv_action_map(&mut tx_actions, &new_tx_actions);
        }
        //TODO 记录"上一次调度成功的信息"

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

async fn service_main(is_boot: bool) -> Result<i32> {
    init_logging("scheduler", true);
    info!("Starting scheduler service............................");

    if is_boot {
        info!("do_boot_scheduler,scheduler run once");
        let runtime =
            init_buckyos_api_runtime("scheduler", None, BuckyOSRuntimeType::KernelService)
                .await
                .map_err(|e| {
                    error!("init_buckyos_api_runtime failed: {:?}", e);
                    e
                })?;
        set_buckyos_api_runtime(runtime);
        do_boot_scheduler().await.map_err(|e| {
            error!("do_boot_scheduler failed: {:?}", e);
            e
        })?;
        return Ok(0);
    } else {
        info!("Enter schedule loop.");
        let mut runtime =
            init_buckyos_api_runtime("scheduler", None, BuckyOSRuntimeType::KernelService)
                .await
                .map_err(|e| {
                    error!("init_buckyos_api_runtime failed: {:?}", e);
                    e
                })?;
        runtime.login().await.map_err(|e| {
            error!("login failed: {:?}", e);
            e
        })?;
        set_buckyos_api_runtime(runtime);
        schedule_loop(false).await.map_err(|e| {
            error!("schedule_loop failed: {:?}", e);
            e
        })?;
        return Ok(0);
    }
}

#[tokio::main]
async fn main() {
    let args = std::env::args().collect::<Vec<String>>();
    let mut is_boot = false;
    if args.len() > 1 {
        if args[1] == "--boot" {
            is_boot = true;
        }
    }

    let ret = service_main(is_boot).await;
    if ret.is_err() {
        println!("service_main failed: {:?}", ret);
        exit(-1);
    }
    exit(ret.unwrap());
}

#[cfg(test)]
mod test {
    use super::*;
    use tokio::test;
    #[tokio::test]
    async fn test_schedule_loop() {
        service_main(true).await;
    }

    #[tokio::test]
    async fn test_template() {
        let start_params = create_init_list_by_template().await.unwrap();
        for (key, value) in start_params.iter() {
            let json_value = serde_json::from_str(value);
            if json_value.is_ok() {
                let json_value: serde_json::Value = json_value.unwrap();
                println!("{}:\t{:?}", key, json_value);
            } else {
                println!("{}:\t{}", key, value);
            }
        }
        //println!("start_params:{}",serde_json::to_string(&start_params).unwrap());
    }
    #[tokio::test]
    async fn test_simple_schedule() {
        let input_config_str = r#"
# users : users/{{user_id}}/info , user_id is never changed, user_name can changed. User root cann't be deleted and always exists
"users/root/info" = '{"type":"root","username":"{{user_name}}","password":"{{admin_password_hash}}"}'

# devices,set & update by register_device_doc@node_daemon
#"devices/ood1/doc" = "ood1_doc"
# devices,set & update by update_device_info@node_daemon
"devices/ood1/info" = """
{
    "hostname":"ood1",
    "device_type":"ood",
    "arch":"aarch64"
}
"""

# system settings

"system/verify_hub/key" = """
{{verify_hub_key}}
"""
# frames & services
"services/verify_hub/info" = """
{
    "port":3300,
    "node_list":["ood1"],
    "type":"kernel"
}
"""
"services/verify_hub/settings" = """
{
    "trust_keys" : []
}
"""
"services/scheduler/info" = """
{
    "port":3400,
    "node_list":["ood1"],
    "type":"kernel"
}
"""
# info for zone-gateway
"services/gateway/info" = """
{
    "port":3100,
    "node_list":["ood1"],
    "type":"kernel"
}
"""
"services/gateway/settings" = """
{
    "shortcuts": {
        "www": {
            "type":"app",
            "user_id":"root",
            "app_id":"home-station"
        },
        "sys": {
            "type":"app",
            "user_id":"root",
            "app_id":"control-panel"
        },
        "test":{
            "type":"app",
            "user_id":"root",
            "app_id":"sys-test"
        }
    }
}
"""

"services/gateway/base_config" = """
{
    "device_key_path":"/opt/buckyos/etc/node_private_key.pem",
    "servers":{
        "main_http_server":{
            "type":"cyfs-warp",
            "bind":"0.0.0.0",
            "http_port":80,
            "tls_port":443,
            "hosts": {
                "*": {
                    "enable_cors":true,
                    "routes": {
                        "/kapi/system_config":{
                            "upstream":"http://127.0.0.1:3200"
                        },
                        "/kapi/verify_hub":{
                            "upstream":"http://127.0.0.1:3300"
                        }
                    }
                },
                "sys.*": {
                    "enable_cors":true,
                    "routes": {
                        "/":{
                            "local_dir":"/opt/buckyos/bin/control_panel"
                        },
                        "/kapi/system_config":{
                            "upstream":"http://127.0.0.1:3200"
                        },
                        "/kapi/verify_hub":{
                            "upstream":"http://127.0.0.1:3300"
                        }
                    }
                }
            }
        }
    },
    "dispatcher" : {
        "tcp://0.0.0.0:80":{
            "type":"server",
            "id":"main_http_server"
        },
        "tcp://0.0.0.0:443":{
            "type":"server",
            "id":"main_http_server"
        }
    }
}
"""

# install apps
"users/root/apps/home-station/config" = """
{
    "app_id":"home-station",
    "app_info" : {
        "name" : "Home Station",
        "description" : "Home Station",
        "vendor_did" : "did:bns:buckyos",
        "pkg_id" : "home-station",
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
            }
        }
    },
    "app_index" : 0,
    "enable" : true,
    "instance" : 1,
    "deployed" : false,

    "data_mount_point" : "/srv",
    "cache_mount_point" : "/database/",
    "local_cache_mount_point" : "/config/",
    "max_cpu_num" : 4,
    "max_cpu_percent" : 80,
    "memory_quota" : 1073741824,
    "tcp_ports" : {
        "www":80
    }
}
"""
# node config
"nodes/ood1/config" = """
{
    "is_running":true,
    "revision" : 0,
    "gateway" : {
    },
    "kernel":{
        "verify_hub" : {
            "target_state":"Running",
            "pkg_id":"verify_hub",
            "operations":{
                "status":{
                    "command":"status",
                    "params":[]
                },
                "start":{
                    "command":"start",
                    "params":[]
                },
                "stop":{
                    "command":"stop",
                    "params":[]
                }
            }
        },
        "scheduler" : {
            "target_state":"Running",
            "pkg_id":"scheduler",
            "operations":{
                "status":{
                    "command":"status",
                    "params":[]
                },
                "start":{
                    "command":"start",
                    "params":[]
                },
                "stop":{
                    "command":"stop",
                    "params":[]
                }
            }
        }
    },
    "services":{

    },
    "apps":{
        "0":{
            "target_state":"Running",
            "app_id":"home-station",
            "username":"root"
            "service_docker_images" : "filebrowser/filebrowser:s6",
            "data_mount_point" : "/srv",
            "cache_mount_point" : "/database/",
            "local_cache_mount_point" : "/config/",
            "extra_mounts" : {
                "/opt/buckyos/data/root/home-station/:/srv",
            },
            "max_cpu_num" : 4,
            "max_cpu_percent" : 80,
            "memory_quota" : 1073741824,
            "tcp_ports" : {
                "20000":80
            }
        }
    }
}
""""
"nodes/ood1/gateway_config" = """
{
    "device_key_path":"/opt/buckyos/etc/node_private_key.pem",
    "servers":{
        "main_http_server":{
            "type":"cyfs-warp",
            "bind":"0.0.0.0",
            "http_port":80,
            "tls_port":443,
            "hosts": {
                "*": {
                    "enable_cors":true,
                    "routes": {
                        "/kapi/system_config":{
                            "upstream":"http://127.0.0.1:3200"
                        },
                        "/kapi/verify_hub":{
                            "upstream":"http://127.0.0.1:3300"
                        },
                        "/":{
                            "upstream":"http://127.0.0.1:20000"
                        }
                    }
                },
                "sys.*":{
                    "routes":{
                        "/":{
                            "local_dir":"{{BUCKYOS_ROOT}}/bin/control_panel"
                        }
                    }
                },
                "test.*":{
                    "routes":{
                        "/":{
                            "local_dir":"{{BUCKYOS_ROOT}}/bin/sys_test"
                        }
                    }
                }
            }
        }
    },
    "dispatcher" : {
        "tcp://0.0.0.0:80":{
            "type":"server",
            "id":"main_http_server"
        },
        "tcp://0.0.0.0:443":{
            "type":"server",
            "id":"main_http_server"
        }
    }
}
"""

"system/rbac/model" = """
[request_definition]
r = sub,obj,act

[policy_definition]
p = sub, obj, act, eft

[role_definition]
g = _, _ # sub, role

[policy_effect]
e = priority(p.eft) || deny

[matchers]
m = (g(r.sub, p.sub) || r.sub == p.sub) && ((r.sub == keyGet3(r.obj, p.obj, p.sub) || keyGet3(r.obj, p.obj, p.sub) =="") && keyMatch3(r.obj,p.obj)) && regexMatch(r.act, p.act)
"""
"system/rbac/policy" = """
p, kernel, kv://*, read|write,allow
p, kernel, dfs://*, read|write,allow

p, owner, kv://*, read|write,allow
p, owner, dfs://*, read|write,allow

p, user, kv://*, read,allow
p, user, dfs://public/*,read|write,allow
p, user, dfs://homes/{user}/*, read|write,allow
p, app,  dfs://homes/*/apps/{app}/*, read|write,allow

p, limit, dfs://public/*, read,allow
p, guest, dfs://public/*, read,allow

g, node_daemon, kernel
g, ood01,ood
g, alice, user
g, bob, user
g, app1, app
g, app2, app
"""

"boot/config" = """
{
    "did":"did:ens:{{user_name}}",
    "oods":["ood1"],
    "sn":"{{sn_host}}",
    "verify_hub_info":{
        "port":3300,
        "node_name":"ood1",
        "public_key":{{verify_hub_public_key}}
    }
}
"""
        "#;
        // buckyos_kit::init_logging("scheduler",false);
        // let input_config: HashMap<String, String> = toml::from_str(input_config_str).unwrap();
        // //let schedule_result = do_one_ood_schedule(&input_config).await;
        // //let schedule_result = schedule_result.unwrap();
        // println!("schedule_result:{}",serde_json::to_string(&schedule_result).unwrap());
    }
}
