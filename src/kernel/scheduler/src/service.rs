use std::collections::HashMap;
use serde_json::{json,Value};
use buckyos_api::*;
use buckyos_kit::*;
use crate::scheduler::*;
use anyhow::Result;
use name_lib::DeviceInfo;
use log::warn;

pub fn instance_service(new_instance:&PodInstance,server_config:&KernelServiceConfig,is_zone_gateway:bool)->Result<HashMap<String,KVAction>> {
    let mut result = HashMap::new();
    //目前所有的service都是kernel service (no docker) ,有标准的frame service也是应该运行在docker中的.
    //add instance to node config
    let kernel_service_config = KernelServiceInstanceConfig::new(server_config.pkg_id.clone());
    let key_path = format!("nodes/{}/config",new_instance.node_id.as_str());
    let json_path = format!("kernel/{}",new_instance.pod_id.as_str());
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
        let json_path = format!("servers/zone_gateway/hosts/*/routes/\"/kapi/{}\"",new_instance.pod_id.as_str());
        let set_value = json!({
            "upstream":format!("http://127.0.0.1:{}",server_config.port),
        });
        set_actions.insert(json_path,Some(set_value));
    }

    let json_path = format!("servers/node_gateway/hosts/*/routes/\"/kapi/{}\"",new_instance.pod_id.as_str());
    let set_value = json!({
        "upstream":format!("http://127.0.0.1:{}",server_config.port),
    });
    set_actions.insert(json_path,Some(set_value));

    
   
    let set_action = KVAction::SetByJsonPath(set_actions);
    result.insert(key_path,set_action);
    Ok(result)
}

pub fn uninstance_service(instance:&PodInstance)->Result<HashMap<String,KVAction>> {
    let mut result = HashMap::new();
    let key_path = format!("nodes/{}/config",instance.node_id.as_str());
    let json_path = format!("kernel/{}",instance.pod_id.as_str());
    let mut set_actions = HashMap::new();
    set_actions.insert(json_path,None);
    result.insert(key_path,KVAction::SetByJsonPath(set_actions));

    let key_path = format!("nodes/{}/gateway_config",instance.node_id.as_str());
    let json_path = format!("servers/zone_gateway/hosts/*/routes/\"/kapi/{}\"",instance.pod_id.as_str());
    let mut set_actions:HashMap<String,Option<Value>> = HashMap::new();
    set_actions.insert(json_path,None);
    let set_action = KVAction::SetByJsonPath(set_actions);
    result.insert(key_path,set_action);

    Ok(result)
}

pub fn update_service_instance(instance:&PodInstance)->Result<HashMap<String,KVAction>> {
    unimplemented!();
}

pub fn update_service_info(pod_id: &str, pod_info: &PodInfo,device_list:&HashMap<String, DeviceInfo>) -> Result<HashMap<String, KVAction>> {
    //update service info
    let mut result = HashMap::new();

    let key = format!("services/{}/info",pod_id);
    let mut info_map:HashMap<String, ServiceNodeInfo> = HashMap::new();
    match pod_info {
        PodInfo::RandomCluster(cluster) => {
            for (node_id, (weight,instance)) in cluster.iter() {
                let device_info = device_list.get(node_id.as_str());
                if device_info.is_some() {
                    let device_info = device_info.unwrap();
                    let node_net_id = device_info.device_doc.net_id.clone();

                    info_map.insert(node_id.clone(), ServiceNodeInfo {
                        weight: weight.clone(),
                        state: "Running".to_string(), 
                        port: instance.service_port,
                        node_did: instance.node_id.clone(),
                        node_net_id:node_net_id,
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

    //TODO: update pod_item's cyfs_gateway upstream cluster info here?
    Ok(result)
}

pub fn set_service_state(pod_id:&str,state:&PodItemState)->Result<HashMap<String,KVAction>> {
    let key = format!("services/{}/config",pod_id);
    let mut set_paths = HashMap::new();
    set_paths.insert("state".to_string(),Some(json!(state.to_string())));
    let mut result = HashMap::new();
    result.insert(key,KVAction::SetByJsonPath(set_paths));
    Ok(result)
}