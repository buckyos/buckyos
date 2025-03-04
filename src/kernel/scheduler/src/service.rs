use std::collections::HashMap;
use serde_json::{json,Value};
use buckyos_api::*;
use buckyos_kit::*;
use crate::scheduler::*;
use anyhow::Result;

pub fn instance_service(new_instance:&PodInstance,server_info:&KernelServiceConfig)->Result<HashMap<String,KVAction>> {
    let mut result = HashMap::new();
    //目前所有的service都是kernel service (no docker) ,有标准的frame service也是应该运行在docker中的.
    //add instance to node config
    let kernel_service_config = KernelServiceInstanceConfig::new(server_info.pkg_id.clone());
    let key_path = format!("nodes/{}/config",new_instance.node_id.as_str());
    let json_path = format!("kernel/{}",new_instance.pod_id.as_str());
    let set_value = serde_json::to_value(kernel_service_config)?;
    let mut set_actions = HashMap::new();
    set_actions.insert(json_path,Some(set_value));
    let set_action = KVAction::SetByJsonPath(set_actions);
    result.insert(key_path,set_action);

    //add to node gateway config
    let key_path = format!("nodes/{}/gateway_config",new_instance.node_id.as_str());
    //TODO: fix bug
    let json_path = format!("servers/main_http_server/hosts/*/routes/\"/kapi/{}\"",new_instance.pod_id.as_str());
    let set_value = json!({
        "upstream":format!("http://127.0.0.1:{}",server_info.port),
    });
    let mut set_actions = HashMap::new();
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
    let json_path = format!("servers/main_http_server/hosts/*/routes/\"/kapi/{}\"",instance.pod_id.as_str());
    let mut set_actions:HashMap<String,Option<Value>> = HashMap::new();
    set_actions.insert(json_path,None);
    let set_action = KVAction::SetByJsonPath(set_actions);
    result.insert(key_path,set_action);

    Ok(result)
}

pub fn update_service_instance(instance:&PodInstance)->Result<HashMap<String,KVAction>> {
    unimplemented!();
}

pub fn set_service_state(pod_id:&str,state:&PodItemState)->Result<HashMap<String,KVAction>> {
    let key = format!("services/{}/config",pod_id);
    let mut set_paths = HashMap::new();
    set_paths.insert("state".to_string(),Some(json!(state.to_string())));
    let mut result = HashMap::new();
    result.insert(key,KVAction::SetByJsonPath(set_paths));
    Ok(result)
}