use std::collections::HashMap;
use serde_json::{json,Value};
use buckyos_api::*;
use buckyos_kit::*;
use crate::scheduler::{ServiceInfo as SchedulerServiceInfo, ReplicaInstance, ServiceSpecState};
use anyhow::Result;
pub use name_lib::DeviceInfo;
use log::warn;

pub fn instance_service(
    new_instance:&ReplicaInstance,
    server_config:&KernelServiceSpec,
    is_zone_gateway:bool
)->Result<HashMap<String,KVAction>> {
    let mut result = HashMap::new();
    //目前所有的service都是kernel service (no docker) ,有标准的frame service也是应该运行在docker中的.
    //add instance to node config
    let service_port = server_config
        .install_config
        .service_ports
        .get("main")
        .copied()
        .or_else(|| server_config.install_config.service_ports.values().next().copied())
        .unwrap_or(0);
    let kernel_service_config =
        KernelServiceInstanceConfig::new(server_config.clone(), new_instance.node_id.clone());
    let key_path = format!("nodes/{}/config",new_instance.node_id.as_str());
    let json_path = format!("kernel/{}",new_instance.spec_id.as_str());
    let set_value = serde_json::to_value(kernel_service_config)?;
    let mut set_actions = HashMap::new();
    set_actions.insert(json_path,Some(set_value));
    let set_action = KVAction::SetByJsonPath(set_actions);
    result.insert(key_path,set_action);

    //add to node gateway config
    let key_path = format!("nodes/{}/gateway_config",new_instance.node_id.as_str());
    let mut set_actions = HashMap::new();
    //TODO: cyfs-gateway need support router-cluster or router-buckyos_service_selector
    if is_zone_gateway {
        let json_path = format!("servers/zone_gateway/hosts/*/routes/\"/kapi/{}\"",new_instance.spec_id.as_str());
        let set_value = json!({
            "upstream":format!("http://127.0.0.1:{}",service_port),
        });
        set_actions.insert(json_path,Some(set_value));

        let json_path = format!("servers/zone_gateway/hosts/sys*/routes/\"/kapi/{}\"",new_instance.spec_id.as_str());
        let set_value = json!({
            "upstream":format!("http://127.0.0.1:{}",service_port),
        });
        set_actions.insert(json_path,Some(set_value));
    }

    let json_path = format!("servers/node_gateway/hosts/*/routes/\"/kapi/{}\"",new_instance.spec_id.as_str());
    let set_value = json!({
        "upstream":format!("http://127.0.0.1:{}",service_port),
    });
    set_actions.insert(json_path,Some(set_value));
   
    let set_action = KVAction::SetByJsonPath(set_actions);
    result.insert(key_path,set_action);
    Ok(result)
}

pub fn uninstance_service(instance:&ReplicaInstance)->Result<HashMap<String,KVAction>> {
    let mut result = HashMap::new();
    let key_path = format!("nodes/{}/config",instance.node_id.as_str());
    let json_path = format!("kernel/{}",instance.spec_id.as_str());
    let mut set_actions = HashMap::new();
    set_actions.insert(json_path,None);
    result.insert(key_path,KVAction::SetByJsonPath(set_actions));

    let key_path = format!("nodes/{}/gateway_config",instance.node_id.as_str());
    let json_path = format!("servers/zone_gateway/hosts/*/routes/\"/kapi/{}\"",instance.spec_id.as_str());
    let mut set_actions:HashMap<String,Option<Value>> = HashMap::new();
    set_actions.insert(json_path,None);
    let set_action = KVAction::SetByJsonPath(set_actions);
    result.insert(key_path,set_action);

    Ok(result)
}

pub fn update_service_instance(instance:&ReplicaInstance)->Result<HashMap<String,KVAction>> {
    unimplemented!();
}

pub fn update_service_info(spec_id: &str, service_info: &SchedulerServiceInfo,device_list:&HashMap<String, DeviceInfo>) -> Result<HashMap<String, KVAction>> {
    //update service info
    let mut result = HashMap::new();

    let key = format!("services/{}/info",spec_id);
    let mut info_map:HashMap<String, ServiceNode> = HashMap::new();
    match service_info {
        SchedulerServiceInfo::RandomCluster(cluster) => {
            for (node_id, (weight,instance)) in cluster.iter() {
                let device_info = device_list.get(node_id.as_str());
                if device_info.is_some() {
                    let device_info = device_info.unwrap();
                    let node_net_id = device_info.device_doc.net_id.clone();

                    info_map.insert(node_id.clone(), ServiceNode {
                        node_did: instance.node_id.clone(),
                        node_net_id,
                        state: ServiceInstanceState::Started,
                        weight: *weight,
                        service_port: HashMap::from([(
                            "main".to_string(),
                            instance.service_port,
                        )]),
                    });
                } else {
                    warn!("device info not found for node: {}",node_id);
                }
            }
        }
    }
    let service_info = ServiceInfo {
        selector_type: "random".to_string(),
        node_list: info_map,
    };

    result.insert(key,KVAction::Update(serde_json::to_string(&service_info)?));

    //TODO: update service_spec's cyfs_gateway upstream cluster info here?
    Ok(result)
}

pub fn set_service_state(spec_id:&str,state:&ServiceSpecState)->Result<HashMap<String,KVAction>> {
    let key = format!("services/{}/config",spec_id);
    let mut set_paths = HashMap::new();
    set_paths.insert("state".to_string(),Some(json!(state.to_string())));
    let mut result = HashMap::new();
    result.insert(key,KVAction::SetByJsonPath(set_paths));
    Ok(result)
}