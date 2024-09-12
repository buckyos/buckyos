use std::collections::HashMap;
use std::process::exit;
use std::{fs::File};
use log::*;
use simplelog::*;
use serde_json::{json};

use name_lib::*;
use buckyos_kit::*;
use sys_config::SystemConfigClient;


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
            "x86_server" : "buckyos/home_station:latest"
        },
        "data_mount_point" : "/home/data",
        "cache_mount_point" : "/home/cache",
        "local_cache_mount_point" : "/home/local_cache",
        "max_cpu_num" : Some(4),
        "max_cpu_percent" : Some(80),
        "memory_quota" : 1024*1024*1024*1, //1GB
        "host_name" : Some("home".to_string()),
        "port" : 20080
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

async fn do_boot_scheduler() -> Result<()> {
    let mut init_list : HashMap<String,String> = HashMap::new();
    let zone_config_str = std::env::var("BUCKY_ZONE_CONFIG");
    
    if zone_config_str.is_ok() {
        info!("zone_config_str:{}",zone_config_str.as_ref().unwrap());
        let mut zone_config:ZoneConfig = serde_json::from_str(&zone_config_str.unwrap()).unwrap();
        let rpc_session_token_str = std::env::var("SCHEDULER_SESSION_TOKEN"); 
        if rpc_session_token_str.is_ok() {
            let rpc_session_token = rpc_session_token_str.unwrap();
            let vec_oods = vec![];
            let system_config_client = SystemConfigClient::new(&vec_oods,&Some(rpc_session_token));
            let boot_config = system_config_client.get("boot/config").await;
            if boot_config.is_ok() {
                return Err("boot/config already exists, boot scheduler failed".into());
            }

            //generate ood config
            if zone_config.oods.len() ==1 {
                //add default user
                let mut owner_name = zone_config.owner_name.clone().unwrap();

                if is_did(&owner_name) {
                    let owner_did = DID::from_str(&owner_name);
                    if owner_did.is_some() {
                        owner_name = owner_did.unwrap().id.to_string();
                    }
                }
                let owner_str = serde_json::to_string(&json!(   
                    {
                        "type":"admin"
                    }
                )).unwrap();
                
                init_list.insert(format!("users/{}/info",owner_name),owner_str);
                let app_config = generate_app_config(&owner_name).await?;
                init_list.extend(app_config);
       

                let ood_name = zone_config.oods[0].clone();
                let ood_config = generate_ood_config(&ood_name,&owner_name).await?;
                init_list.extend(ood_config);

                //generate verify_hub service config
                //generate verify_hub key pairs
                let (private_key_pem, public_key_jwk) = generate_ed25519_key_pair();
                let verify_hub_info = VerifyHubInfo {
                    node_name: ood_name.clone(),
                    public_key: serde_json::from_value(public_key_jwk).unwrap(),
                };
                zone_config.verify_hub_info = Some(verify_hub_info);
                init_list.insert("system/verify_hub/key".to_string(),private_key_pem);
                let verify_hub_info_str = serde_json::to_string(&json!(
                    {
                        "endpoints" :[
                            format!("{}:10032",ood_name)
                        ]
                    }
                )).unwrap();
                init_list.insert("services/verify_hub/info".to_string(),verify_hub_info_str);
                let verify_hub_setting_str = serde_json::to_string(&json!(
                    {
                        "trust_keys" : []
                    }
                )).unwrap();
                init_list.insert("services/verify_hub/setting".to_string(),verify_hub_setting_str);

                //scheduer
                let scheduler_info_str = serde_json::to_string(&json!(
                    {
                        "endpoints" :[
                            format!("{}:10034",ood_name)
                        ]
                    }
                )).unwrap();
                init_list.insert("services/scheduler/info".to_string(),scheduler_info_str);


                //write zone config 
                init_list.insert("boot/config".to_string(),serde_json::to_string(&zone_config).unwrap());
                init_list.insert("system/rbac/model".to_string(),rbac::DEFAULT_MODEL.to_string());
                init_list.insert("system/rbac/policy".to_string(),rbac::DEFAULT_POLICY.to_string());

                //write to system_config
                for (key,value) in init_list.iter() {
                    system_config_client.create(key,value).await?;
                }
                info!("boot scheduler success");
                return Ok(());
                
            } else {
                error!("only one ood in zone config is supported");
                return Err("only one ood in zone config is supported".into());
            }
        }
    }

    Err("boot scheduler failed".into())
}

async fn schedule_loop() -> Result<()> {
    let mut loop_step = 0;
    let is_running = true;
    info!("schedule loop start...");
    loop {
        if !is_running {
            break;
        }
        
        loop_step += 1;
        info!("schedule loop step:{}.", loop_step);
        tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
    }
    Ok(())

}

fn init_log_config() {
    let config = ConfigBuilder::new()
        .set_time_format_custom(format_description!("[year]-[month]-[day] [hour]:[minute]:[second].[subsecond digits:3]"))
        .build();
       
    let log_path = get_buckyos_root_dir().join("logs").join("scheduler.log");

    CombinedLogger::init(vec![
        // 将日志输出到标准输出，例如终端
        TermLogger::new(
            LevelFilter::Info,
            config.clone(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ),
        WriteLogger::new(
            LevelFilter::Info,
            config,
            File::create(log_path).unwrap(),
        ),
    ])
    .unwrap();
}

async fn service_main(is_boot:bool) -> Result<i32> {
    init_log_config();
    info!("Starting scheduler service............................");
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
}