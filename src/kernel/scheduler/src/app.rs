use scheduler::parse_app_pod_id;
use scheduler::PodInstance;
use scheduler::PodItemState;
use sys_config::*;
use name_lib::*;
use std::collections::HashMap;
use serde_json::json;
use serde_json::Value;
use log::{info, warn};
use buckyos_kit::*;
use crate::*;

use anyhow::Result;

fn build_app_service_config(user_id:&str,app_config:&AppConfigNode,node_info:&DeviceInfo) -> Result<AppServiceConfig> {
    let arch_name = node_info.arch.clone().unwrap_or("amd64".to_string());
    let docker_pkg_name = format!("{}_docker_image",arch_name.as_str());
    let docker_pkg_info =  app_config.app_info.pkg_list.get(&docker_pkg_name);
    if docker_pkg_info.is_none() {
        return Err(anyhow::anyhow!("docker_pkg_name: {} not found",docker_pkg_name));
    }
    let docker_pkg_info = docker_pkg_info.unwrap();

    let mut result_config = AppServiceConfig::new(user_id,app_config);
    result_config.docker_image_name = Some(docker_pkg_info.docker_image_name.clone().unwrap());
    if  app_config.app_index  > 400 {
        warn!("app_index: {} is too large,skip",app_config.app_index);
        return Err(anyhow::anyhow!("app_index: {} is too large",app_config.app_index));
    }

    let mut real_port:u16 = 20080 + app_config.app_index * 100;
    for (port_desc,inner_port) in app_config.tcp_ports.iter() {
        result_config.tcp_ports.insert(real_port, inner_port.clone());
        real_port += 1;
    }
    return Ok(result_config)    
}


fn select_node_for_app(device_list:&HashMap<String,DeviceInfo>,app_config:&AppConfigNode) -> Result<String> {
    for (node_name,node_info) in device_list.iter() {
        if node_info.device_type == "ood" || node_info.device_type == "server" {
            return Ok(node_name.clone());
        }
    }
    return Err(anyhow::anyhow!("no node found"));
}

//return action_map and http_port
pub async fn deploy_app_service(user_id:&str,app_id:&str,device_list:&HashMap<String,DeviceInfo>,input_config:&HashMap<String,String>) 
    -> Result<(HashMap<String, JsonValueAction>,Option<u16>)> {
    let mut result_port = None;
    let app_config_path = format!("users/{}/apps/{}/config",user_id,app_id);
    let app_config = input_config.get(&app_config_path);
    if app_config.is_none() {
        return Err(anyhow::anyhow!("app_config: {} not found",app_config_path));
    }
    let app_config = app_config.unwrap();
    println!("{}",app_config);
    let app_config = serde_json::from_str(&app_config);
    if app_config.is_err() {
        println!("{:?}",app_config.err());
        return Err(anyhow::anyhow!("app_config: {} is not a valid json",app_config_path));
    }
    let app_config : AppConfigNode = app_config.unwrap();
    //if app_config.deployed {
    //    info!("app: {} is already deployed,skip",app_id);
    //    return Ok((HashMap::new(),None));
    // }

    warn!("app: {} is not deployed, start deploy...",app_id);
    //首先根据instace的数量选择ood, 
    let mut result_config = HashMap::new();
    let node_id = select_node_for_app(device_list,&app_config)?;
    let node_info = device_list.get(&node_id).unwrap();
    //根据ood的info(硬件配置) 构造node_config里的apps配置
    let app_service_config = build_app_service_config(user_id,&app_config,&node_info)?;
    let app_index = app_config.app_index;
    let mut set_action = HashMap::new();
    set_action.insert(format!("/apps/{}",app_index),serde_json::to_value(&app_service_config).unwrap());
    let app_service_config_set_action = JsonValueAction::SetByPath(set_action);
    result_config.insert(format!("nodes/{}/config",node_id),app_service_config_set_action);

    //如果是http服务,则需要挂到默认的sub host上
    let http_port = app_service_config.get_http_port();
    if http_port.is_some() {
        let http_port = http_port.unwrap();
        let app_prefix;
        if user_id == "root" {
            app_prefix = format!("{}.*",app_id);
        } else {
            app_prefix = format!("{}.{}.*",app_id,user_id);
        }
        //创建默认的appid-userid的短域名给node-gateway.json
        let gateway_path = format!("/servers/main_http_server/hosts/{}",app_prefix);
        let app_gateway_config = json!(
            {
                "routes":{
                    "/":{
                        "upstream":format!("http://127.0.0.1:{}",http_port)
                    }
                }  
            }
        );    
        let mut set_action = HashMap::new();
        set_action.insert(gateway_path,app_gateway_config);
        let node_gateway_set_action = JsonValueAction::SetByPath(set_action);
        result_config.insert(format!("nodes/{}/gateway",node_id),node_gateway_set_action);  
        result_port = Some(http_port);
    }

    //修改用户组不在调度器里做,而是在安装的时候做

    //修改deployed为true
    let mut set_action = HashMap::new();
    set_action.insert(format!("/deployed"),Value::Bool(true));
    let set_deployed_action = JsonValueAction::SetByPath(set_action);
    result_config.insert(format!("users/{}/apps/{}/config",user_id,app_id),set_deployed_action);
    return Ok((result_config,result_port));
}


pub fn instance_app_service(new_instance:&PodInstance,device_list:&HashMap<String,DeviceInfo>,input_config:&HashMap<String,String>)->Result<HashMap<String,KVAction>> {
    let mut result = HashMap::new();
    
    let (app_id,user_id) = parse_app_pod_id(new_instance.pod_id.as_str())?;
    let app_config_path = format!("users/{}/apps/{}/config",user_id,app_id);
    let app_config = input_config.get(&app_config_path);
    if app_config.is_none() {
        return Err(anyhow::anyhow!("app_config: {} not found",app_config_path));
    }
    let app_config = app_config.unwrap();
    debug!("will instance_app_service app_config: {}",app_config);
    let app_config = serde_json::from_str(&app_config);
    if app_config.is_err() {
        println!("{:?}",app_config.err());
        return Err(anyhow::anyhow!("app_config: {} is not a valid json",app_config_path));
    }
    let app_config : AppConfigNode = app_config.unwrap();

    let node_info = device_list.get(&new_instance.node_id);
    if node_info.is_none() {
        return Err(anyhow::anyhow!("node_info: {} not found",new_instance.node_id));
    }
    let node_info = node_info.unwrap();

    //write to node_config
    let app_service_config = build_app_service_config(user_id.as_str(),&app_config,&node_info)?;
    let mut set_action = HashMap::new();
    set_action.insert(format!("/apps/{}",new_instance.instance_id.as_str()),serde_json::to_value(&app_service_config).unwrap());
    let app_service_config_set_action =  KVAction::SetByJsonPath(set_action);
    result.insert(format!("nodes/{}/config",new_instance.node_id),app_service_config_set_action);

    //write to gateway_config
    let http_port = app_service_config.get_http_port();
    if http_port.is_some() {
        let http_port = http_port.unwrap();
        let app_prefix;
        if user_id == "root" {
            app_prefix = format!("{}.*",app_id);
        } else {
            app_prefix = format!("{}_{}.*",app_id,user_id);
        }
        //创建默认的appid-userid的短域名给node-gateway.json
        let gateway_path = format!("/servers/main_http_server/hosts/{}",app_prefix);
        let app_gateway_config = json!(
            {
                "routes":{
                    "/":{
                        "upstream":format!("http://127.0.0.1:{}",http_port)
                    }
                }  
            }
        );    
        let mut set_action = HashMap::new();
        set_action.insert(gateway_path,app_gateway_config);
        let node_gateway_set_action = KVAction::SetByJsonPath(set_action);
        result.insert(format!("nodes/{}/gateway",new_instance.node_id.as_str()),node_gateway_set_action);  
    }

    // if http_port.is_some() {
    //     all_app_http_port.insert(format!("{}.{}",app_id,user_name),http_port.unwrap());
    // }

    //调度器不处理权限,权限是在安装应用的时候就完成的配置
    Ok(result)
}

pub fn uninstance_app_service(instance:&PodInstance)->Result<HashMap<String,KVAction>> {
    unimplemented!();
}

pub fn update_app_service_instance(instance:&PodInstance)->Result<HashMap<String,KVAction>> {
    unimplemented!();
}

pub fn set_app_service_state(pod_id:&str,state:&PodItemState)->Result<HashMap<String,KVAction>> {
    //pod_id 是app_id@user_id
    let (app_id,user_id) = parse_app_pod_id(pod_id)?;
    let key = format!("users/{}/apps/{}/config",user_id,app_id);
    let mut set_paths = HashMap::new();
    set_paths.insert("state".to_string(),json!(state.to_string()));
    let mut result = HashMap::new();
    result.insert(key,KVAction::SetByJsonPath(set_paths));
    Ok(result)
}
