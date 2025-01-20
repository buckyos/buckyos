use std::collections::HashMap;
use serde_json::json;
use sys_config::*;
use name_lib::*;
use crate::scheduler::*;

use anyhow::Result;

pub fn instance_service(new_instance:&PodInstance)->Result<HashMap<String,KVAction>> {
    unimplemented!();
}

pub fn uninstance_service(instance:&PodInstance)->Result<HashMap<String,KVAction>> {
    unimplemented!();
}

pub fn update_service_instance(instance:&PodInstance)->Result<HashMap<String,KVAction>> {
    unimplemented!();
}

pub fn set_service_state(pod_id:&str,state:&PodItemState)->Result<HashMap<String,KVAction>> {
    let key = format!("services/{}/info",pod_id);
    let mut set_paths = HashMap::new();
    set_paths.insert("state".to_string(),json!(state.to_string()));
    let mut result = HashMap::new();
    result.insert(key,KVAction::SetByJsonPath(set_paths));
    Ok(result)
}