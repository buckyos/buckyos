use buckyos_api::*;
use name_lib::*;
use std::collections::HashMap;
use serde_json::json;
use log::*;

use crate::*;
use crate::scheduler::*;

use anyhow::Result;

const MAX_APP_INDEX:u16 = 2048;
const BASE_APP_PORT:u16 = 10000;

fn build_app_service_config(node_id:&str,app_config:&AppServiceSpec,node_info:&DeviceInfo) -> Result<AppServiceInstanceConfig> {
    let mut result_config = AppServiceInstanceConfig::new(node_id,app_config);
    if node_info.support_container {
        let docker_pkg_name = format!("{}_docker_image",node_info.arch.as_str());
        let docker_pkg_info =  app_config.app_doc.pkg_list.get(&docker_pkg_name);
        if docker_pkg_info.is_none() {
            return Err(anyhow::anyhow!("docker_pkg_name: {} not found",docker_pkg_name));
        }
    } else {
        // 解析是否有<arch>_<os_type>_app字段
        let app_pkg_name = format!("{}_{}_app", node_info.arch.as_str(),node_info.os.as_str());
        if app_config.app_doc.pkg_list.get(&app_pkg_name).is_none() {
            return Err(anyhow::anyhow!("app_pkg_name: {} not found",app_pkg_name));
        }
    }

    if  app_config.app_index  > MAX_APP_INDEX {
        warn!("app_index: {} is too large,skip",app_config.app_index);
        return Err(anyhow::anyhow!("app_index: {} is too large",app_config.app_index));
    }

    let mut real_port:u16 = BASE_APP_PORT + app_config.app_index * 16;
    let mut host_ports = HashMap::new();
    for (service_name, _inner_port) in app_config.install_config.service_ports.iter() {
        host_ports.insert(service_name.clone(), real_port);
        real_port += 1;
    }
    if !host_ports.is_empty() {
        let mut node_install_config = app_config.install_config.clone();
        node_install_config.service_ports = host_ports;
        result_config.node_install_config = Some(node_install_config);
    }
    return Ok(result_config)
}

pub fn instance_app_service(new_instance:&ReplicaInstance,device_list:&HashMap<String,DeviceInfo>,input_config:&HashMap<String,String>)->Result<HashMap<String,KVAction>> {
    let mut result = HashMap::new();
    
    let (app_id,user_id,node_id) = parse_instance_id(new_instance.instance_id.as_str())?;
    //debug!("instance_app_service app_id: {}, user_id: {}, node_id: {},instance_id: {}", app_id, user_id, node_id,new_instance.instance_id);
    let app_config_path = format!("users/{}/apps/{}/spec",user_id,app_id);
    info!("instance_app_service app_config_path: {}",app_config_path);
    let app_config = input_config.get(&app_config_path);
    if app_config.is_none() {
        warn!("app_config: {} not found",app_config_path);
        return Err(anyhow::anyhow!("app_config: {} not found",app_config_path));
    }
    let app_config = app_config.unwrap();
    debug!("will instance_app_service app_config: {}",app_config);
    let app_config = serde_json::from_str(&app_config);
    if app_config.is_err() {
        warn!("app_config: {} is not a valid json",app_config_path);
        return Err(anyhow::anyhow!("app_config: {} is not a valid json",app_config_path));
    }
    let app_config : AppServiceSpec = app_config.unwrap();

    //check app_config.install_config is valid?

    let node_info = device_list.get(&new_instance.node_id);
    if node_info.is_none() {
        return Err(anyhow::anyhow!("node_info: {} not found",new_instance.node_id));
    }
    let node_info = node_info.unwrap();

    //write to node_config
    let app_service_config = build_app_service_config(new_instance.node_id.as_str(),&app_config,&node_info)?;
    let mut set_action = HashMap::new();
    set_action.insert(format!("/apps/{}",new_instance.instance_id.as_str()),
        Some(serde_json::to_value(&app_service_config).unwrap()));
    let app_service_config_set_action =  KVAction::SetByJsonPath(set_action);
    result.insert(format!("nodes/{}/config",new_instance.node_id),app_service_config_set_action);

    // //write to gateway_config
    // let http_port = app_service_config.get_host_service_port("www");
    // if let Some(http_port) = http_port {
    //     let app_prefix;
    //     let mut user_owner_domain:Option<String> = None;
    //     //if user_id is owner,then use app_id as prefix
    //     if user_id == "root" {
    //         app_prefix = format!("{}*",app_id);
    //     } else {
    //         if user_owner_domain.is_none() {
    //             app_prefix = format!("{}-{}*",app_id,user_id);
    //         } else {
    //             app_prefix = format!("{}.{}",app_id,user_owner_domain.as_ref().unwrap());
    //         }
    //     }
        
    //     //创建默认的appid-userid的短域名给node-gateway.json
    //     let gateway_path = format!("/servers/zone_gateway/hosts/{}",app_prefix);
    //     let app_gateway_config = json!(
    //         {
    //             "routes":{
    //                 "/":{
    //                     "upstream":format!("http://127.0.0.1:{}",http_port)
    //                 }
    //             }  
    //         }
    //     );    
    //     let mut set_action = HashMap::new();
    //     set_action.insert(gateway_path,Some(app_gateway_config));

    //     if let Some(gateway_settings) = input_config.get("services/gateway/settings") {
    //         let gateway_settings: GatewaySettings = serde_json::from_str(gateway_settings)?;
    //         for (key,node) in gateway_settings.shortcuts.iter() {
    //             if node.target_type != "app" {
    //                 continue;
    //             }
    //             if node.user_id.is_none() {
    //                 continue;
    //             }
    //             let target_user_id = node.user_id.as_ref().unwrap();
    //             if target_user_id != &user_id {
    //                 continue;
    //             }
    //             let target_app_id = node.app_id.clone();
    //             if target_app_id != app_id {
    //                 continue;
    //             }

    //             info!("will create gatewayshortcut: {} -> {}",key,app_prefix);
    //             // 如果用户设置了独立的域名,则使用 app_id.独立域名
    //             // 使用系统快捷方式会让 appid前缀失效（我们不希望有两个不同的URL都可以访问APP），注意谨慎选择
    //             let short_prefix = match key.as_str() {
    //                 "www" => "*".to_string(),
    //                 _ => format!("{}*",key.as_str()),
    //             };
    //             let gateway_path = format!("/servers/zone_gateway/hosts/{}/routes/\"/\"",short_prefix);
    //             set_action.insert(gateway_path,Some(json!({
    //                 "upstream":format!("http://127.0.0.1:{}",http_port)
    //             })));

    //         }
    //     }
    //     info!("instance_app_service set gateway_config: {:?}",set_action);
    //     result.insert(format!("nodes/{}/gateway_config",new_instance.node_id.as_str()),
    //         KVAction::SetByJsonPath(set_action));  
    
    // }



    //调度器不处理权限,权限是在安装应用的时候就完成的配置
    Ok(result)
}

pub fn uninstance_app_service(instance:&ReplicaInstance)->Result<HashMap<String,KVAction>> {
    let mut result = HashMap::new();
    let (app_id,user_id,node_id) = parse_instance_id(instance.spec_id.as_str())?;

    let key_path = format!("nodes/{}/config",instance.node_id.as_str());
    let mut set_action = HashMap::new();
    set_action.insert(format!("/apps/{}/target_state",instance.instance_id.as_str()), Some(json!("Stopped")));
    result.insert(key_path,KVAction::SetByJsonPath(set_action));

    // let key_path = format!("nodes/{}/gateway_config",instance.node_id.as_str());
    // let mut set_action = HashMap::new();
    // let app_prefix;
    // if user_id == "root" {
    //     app_prefix = format!("{}.*",app_id);
    // } else {
    //     app_prefix = format!("{}_{}.*",app_id,user_id);
    // }
    // set_action.insert(format!("/servers/zone_gateway/hosts/{}",app_prefix.as_str()), None);
    //result.insert(key_path,KVAction::SetByJsonPath(set_action));
    Ok(result)
}

pub fn update_app_service_instance(instance:&ReplicaInstance)->Result<HashMap<String,KVAction>> {
    unimplemented!();
}

pub fn set_app_service_state(spec_id:&str,state:&ServiceSpecState)->Result<HashMap<String,KVAction>> {
    //spec_id 是app_id@user_id
    let (app_id,user_id) = parse_spec_id(spec_id)?;
    let key = format!("users/{}/apps/{}/spec",user_id,app_id);
    debug!("update_app_service sepc key: {}", key);
    let mut set_paths = HashMap::new();
    set_paths.insert("state".to_string(),Some(json!(state.to_string())));
    let mut result = HashMap::new();
    result.insert(key,KVAction::SetByJsonPath(set_paths));
    Ok(result)
}
