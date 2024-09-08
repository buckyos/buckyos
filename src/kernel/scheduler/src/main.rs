use std::collections::HashMap;
use std::process::exit;
use std::{fs::File};
use log::*;
use simplelog::*;
use serde_json::{Value, json};

use name_lib::*;
use buckyos_kit::*;
use ::kRPC::*;
use rbac::*;
use sys_config::SystemConfigClient;


type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

async fn generate_ood_config(ood_name:&str) -> Result<HashMap<String,Value>> {
    let mut init_list : HashMap<String,Value> = HashMap::new();

    //init ood config
    init_list.insert(format!("nodes/{}/config",ood_name),json!({
        "kernel" : {
            "verify_hub" : {
                "revision" : 0,
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
                "revision" : 0,
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

        }
    }));
    
    Ok(init_list)
}

async fn do_boot_scheduler() -> Result<()> {
    let mut init_list : HashMap<String,Value> = HashMap::new();
    let zone_config_str = std::env::var("BUCKY_ZONE_CONFIG");
    if zone_config_str.is_ok() {
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
                let ood_name = zone_config.oods[0].clone();
                let ood_config = generate_ood_config(&ood_name).await?;
                init_list.extend(ood_config);

                //generate verify_hub service config
                //generate verify_hub key pairs
                let (private_key_pem, public_key_jwk) = generate_ed25519_key_pair();
                let verify_hub_info = VerifyHubInfo {
                    node_name: ood_name.clone(),
                    public_key: serde_json::from_value(public_key_jwk).unwrap(),
                };
                zone_config.verify_hub_info = Some(verify_hub_info);
                init_list.insert("system/verify_hub/key".to_string(),Value::String(private_key_pem));
                init_list.insert("services/verify_hub/info".to_string(),json!(
                    {
                        "endpoints" :[
                            format!("{}:10032",ood_name)
                        ]
                    }
                ));
                init_list.insert("services/verify_hub/setting".to_string(),json!(
                    {
                        "trust_keys" : []
                    }
                ));

                //scheduer
                init_list.insert("services/scheduler/info".to_string(),json!(
                    {
                        "endpoints" :[
                            format!("{}:10034",ood_name)
                        ]
                    }
                ));


                //add default user
                if zone_config.owner_name.is_some() {
                    let mut owner_name = zone_config.owner_name.clone().unwrap();
                    if is_did(&owner_name) {
                        let owner_did = DID::from_str(&owner_name);
                        if owner_did.is_some() {
                            owner_name = owner_did.unwrap().id.to_string();
                        }
                    }
                    init_list.insert(format!("users/{}",owner_name),json!(
                        {
                            "type":"admin"
                        }
                    ));
                }

                //write zone config 
                init_list.insert("boot/config".to_string(),serde_json::to_value(zone_config.clone()).unwrap());
                init_list.insert("system/rbac/model".to_string(),Value::String(rbac::DEFAULT_MODEL.to_string()));
                init_list.insert("system/rbac/policy".to_string(),Value::String(rbac::DEFAULT_POLICY.to_string()));

                //write to system_config
                for (key,value) in init_list.iter() {
                    system_config_client.create(key,serde_json::to_string(value).unwrap().as_str()).await?;
                }
                
            } else {
                error!("only one ood in zone config is supported");
                return Err("only one ood in zone config is supported".into());
            }
        }
    }

    Err("boot scheduler failed".into())
}

async fn schedule_loop() -> Result<()> {
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

mod test {
    use tokio::test;
    use super::*;
    #[tokio::test]
    async fn test_schedule_loop() {
        service_main(true).await;
    }
}