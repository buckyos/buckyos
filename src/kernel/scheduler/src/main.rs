mod app;

use std::collections::HashMap;
use std::process::exit;
use log::*;
use serde_json::json;
use serde_json::Value;
//use upon::Engine;

use name_lib::*;
use name_client::*;
use buckyos_kit::*;
use sys_config::SystemConfigClient;
use app::*;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

async fn generate_app_config(user_name:&str) -> Result<HashMap<String,String>> {
    let mut init_list : HashMap<String,String> = HashMap::new();
    let config_str = serde_json::to_string(&json!({
        "app_id":"buckyos.home_station",
        "app_name" : "Home Station",
        "app_description" : "Home Station",
        "vendor_id" : "buckyos",
        "pkg_id" : "home_station",
        "username" : user_name.to_string(),
        "service_docker_images" : {
            "x86_server" : "filebrowser/filebrowser:s6"
        },
        "data_mount_point" : "/srv",
        "cache_mount_point" : "/database/",
        "local_cache_mount_point" : "/config/",
        "max_cpu_num" : Some(4),
        "max_cpu_percent" : Some(80),
        "memory_quota" : 1024*1024*1024*1, //1GB
        "host_name" : Some("home".to_string()),
        "port" : 20080,
        "org_port" : 80
    })).unwrap();
    init_list.insert(format!("users/{}/apps/{}/config",user_name.to_string(),"buckyos.home_station"),config_str);
    Ok(init_list)
}

async fn generate_ood_config(ood_name:&str,owner_name:&str) -> Result<HashMap<String,String>> {
    let mut init_list : HashMap<String,String> = HashMap::new();
    let config_str = serde_json::to_string(&json!({
        "is_running":true,
        "revision" : 0,
        "kernel" : {
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
                    },
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
                    },
                }
            }
        },
        "services":{
        },
        "apps":{
            format!("{}#buckyos.home_station",owner_name):{
                "target_state":"Running",
                "app_id":"buckyos.home_station",
                "username":owner_name,
            }
        }
    })).unwrap();
    //init ood config
    init_list.insert(format!("nodes/{}/config",ood_name),config_str);
    
    Ok(init_list)
}

async fn create_init_list_by_template() -> Result<HashMap<String,String>> {
    //load start_parms from active_service.
    let start_params_file_path = get_buckyos_system_etc_dir().join("start_config.json");
    println!("start_params_file_path:{}",start_params_file_path.to_string_lossy());
    info!("try load start_params from :{}",start_params_file_path.to_string_lossy());
    let start_params_str = tokio::fs::read_to_string(start_params_file_path).await?;
    let mut start_params:serde_json::Value = serde_json::from_str(&start_params_str)?;

    let mut template_type_str = "nat_ood_and_sn".to_string();
    //load template by zone_type from start params.
    let template_type = start_params.get("zone_type");
    if template_type.is_some() {
        template_type_str = template_type.unwrap().as_str().unwrap().to_string();
    }

    let template_file_path = get_buckyos_system_etc_dir().join("scheduler").join(format!("{}.template.toml",template_type_str));
    let template_str = tokio::fs::read_to_string(template_file_path).await?;

    //generate dynamic params 
    let (private_key_pem, public_key_jwk) = generate_ed25519_key_pair();
    start_params["verify_hub_key"] = json!(private_key_pem);
    start_params["verify_hub_public_key"] = json!(public_key_jwk.to_string());

    let mut engine = upon::Engine::new();
    engine.add_template("config", &template_str)?;
    let result = engine
        .template("config")
        .render(&start_params)
        .to_string()?;

    if result.find("{{").is_some() {
        return Err("template contains unescaped double curly braces".into());
    }

    //wwrite result to file
    //let result_file_path = get_buckyos_system_etc_dir().join("scheduler_boot.toml");
    //tokio::fs::write(result_file_path, result.clone()).await?;

    let config: HashMap<String, String> = toml::from_str(&result)?;
    
    Ok(config)
}


async fn do_boot_scheduler() -> Result<()> {
    let mut init_list : HashMap<String,String> = HashMap::new();
    let zone_config_str = std::env::var("BUCKY_ZONE_CONFIG");

    if zone_config_str.is_err() {
        warn!("BUCKY_ZONE_CONFIG is not set, use default zone config");
        return Err("BUCKY_ZONE_CONFIG is not set".into());
    }    

    info!("zone_config_str:{}",zone_config_str.as_ref().unwrap());
    let mut zone_config:ZoneConfig = serde_json::from_str(&zone_config_str.unwrap()).unwrap();
    let rpc_session_token_str = std::env::var("SCHEDULER_SESSION_TOKEN"); 

    if rpc_session_token_str.is_err() {
        return Err("SCHEDULER_SESSION_TOKEN is not set".into());
    }

    let rpc_session_token = rpc_session_token_str.unwrap();
    let system_config_client = SystemConfigClient::new(None,Some(rpc_session_token.as_str()));
    let boot_config = system_config_client.get("boot/config").await;
    if boot_config.is_ok() {
        return Err("boot/config already exists, boot scheduler failed".into());
    }

    let init_list = create_init_list_by_template().await
        .map_err(|e| {
            error!("create_init_list_by_template failed: {:?}", e);
            e
        })?;
    
    info!("use init list from template to do boot scheduler");
    //write to system_config
    for (key,value) in init_list.iter() {
        system_config_client.create(key,value).await?;
    }
    info!("boot scheduler success");
    return Ok(());
}



async fn do_one_ood_schedule(input_config: &HashMap<String, String>) -> Result<HashMap<String, JsonValueAction>> {
    let mut result_config: HashMap<String, JsonValueAction> = HashMap::new();
    return Ok(result_config);
    let mut device_list: HashMap<String, DeviceInfo> = HashMap::new();
    for (key, value) in input_config.iter() {
        if key.starts_with("devices/") && key.ends_with("/info") {
            let device_name = key.split('/').nth(1).unwrap();
            let device_info:DeviceInfo = serde_json::from_str(value)
                .map_err(|e| {
                    error!("serde_json::from_str failed: {:?}", e);
                    e
                })?;
            device_list.insert(device_name.to_string(), device_info);
        }
    }

    
    // Process user app configurations
    for (key, value) in input_config.iter() {
        if key.starts_with("users/") && key.ends_with("/config") {
            let parts: Vec<&str> = key.split('/').collect();
            if parts.len() >= 4 && parts[2] == "apps" {
                let user_name = parts[1];
                let app_id = parts[3];
                
                // Install app for user
                let app_config = deploy_app_service(user_name, app_id,&device_list, &input_config).await;
                if app_config.is_err() {
                    error!("do_one_ood_schedule Failed to install app {} for user {}: {:?}", app_id, user_name, app_config.err().unwrap());
                    return Err("do_one_ood_schedule Failed to install app".into());
                }
                let app_config:HashMap<String, JsonValueAction> = app_config.unwrap();
                //TODO 修改
                result_config.extend(app_config);
            }
        }
    }

    //结合系统的快捷方式配置,设置nodes/gateway 配置

    Ok(result_config)
}

async fn schedule_loop() -> Result<()> {
    let mut loop_step = 0;
    let is_running = true;
    info!("schedule loop start...");
    loop {
        if !is_running {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(15)).await;
        loop_step += 1;
        info!("schedule loop step:{}.", loop_step);
        let rpc_session_token_str = std::env::var("SCHEDULER_SESSION_TOKEN"); 
        if rpc_session_token_str.is_err() {
            return Err("SCHEDULER_SESSION_TOKEN is not set".into());
        }
    
        let rpc_session_token = rpc_session_token_str.unwrap();
        let system_config_client = SystemConfigClient::new(None,Some(rpc_session_token.as_str()));
        let input_config = system_config_client.dump_configs_for_scheduler().await;
        if input_config.is_err() {
            error!("dump_configs_for_scheduler failed: {:?}", input_config.err().unwrap());
            continue;
        }
        let input_config = input_config.unwrap();
        //cover value to hashmap
        let input_config = serde_json::from_value(input_config);
        if input_config.is_err() {
            error!("serde_json::from_value failed: {:?}", input_config.err().unwrap());
            continue;
        }
        let input_config = input_config.unwrap();
        let schedule_result = do_one_ood_schedule(&input_config).await;
        if schedule_result.is_err() {
            error!("do_one_ood_schedule failed: {:?}", schedule_result.err().unwrap());
            continue;
        }
        let schedule_result = schedule_result.unwrap();

        //write to system_config
        for (path,value) in schedule_result.iter() {
            match value {
                JsonValueAction::Update(value) => {
                    system_config_client.set(path,value).await?;
                }
                JsonValueAction::Set(value) => {
                    let old_value = input_config.get(path).unwrap();
                    let mut old_value:Value = serde_json::from_str(old_value).unwrap();
                    for (sub_path,sub_value) in value.iter() {
                        set_json_by_path(&mut old_value,sub_path,Some(sub_value));
                    }
                    system_config_client.set(path,old_value.to_string().as_str()).await?;
                }
                JsonValueAction::Remove => {
                    system_config_client.delete(path).await?;
                }
            }
        }
    }
    Ok(())

}


async fn service_main(is_boot:bool) -> Result<i32> {
    init_logging("scheduler");
    info!("Starting scheduler service............................");
    init_global_buckyos_value_by_env("SCHEDULER");
    let _ =init_default_name_client().await;
    if is_boot {
        info!("do_boot_scheduler,scheduler run once");
        do_boot_scheduler().await.map_err(|e| {
            error!("do_boot_scheduler failed: {:?}", e);
            e
        })?;
        return Ok(0);
    }

    info!("Enter schedule loop.");
    schedule_loop().await.map_err(|e| {
        error!("schedule_loop failed: {:?}", e);
        e
    })?;
    return Ok(0);
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
    use tokio::test;
    use super::*;
    #[tokio::test]
    async fn test_schedule_loop() {
        service_main(true).await;
    }

    #[tokio::test]
    async fn test_template() {
        let start_params = create_init_list_by_template().await.unwrap();
        for (key,value) in start_params.iter() {
            let json_value= serde_json::from_str(value);
            if json_value.is_ok() {
                let json_value:serde_json::Value = json_value.unwrap();
                println!("{}:\t{:?}",key,json_value);
            } else {
                println!("{}:\t{}",key,value);
            }
        }
        //println!("start_params:{}",serde_json::to_string(&start_params).unwrap());
    }
}