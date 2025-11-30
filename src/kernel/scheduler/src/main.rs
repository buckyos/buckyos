#![allow(unused_mut, unused, dead_code)]
mod app;
mod scheduler;
mod service;
mod scheduler_server;
mod system_config_agent;
mod system_config_builder;

#[cfg(test)]
mod scheduler_test;

use jsonwebtoken::jwk::Jwk;
use log::*;
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
use scheduler_server::*;
use service::*;
use system_config_agent::schedule_loop;
use system_config_builder::{StartConfigSummary, SystemConfigBuilder};
use server_runner::*;
use std::sync::Arc;
use anyhow::Result;

async fn create_init_list_by_template(zone_boot_config: &ZoneBootConfig) -> Result<HashMap<String, String>> {
    //load start_parms from active_service.
    let start_params_file_path = get_buckyos_system_etc_dir().join("start_config.json");
    info!(
        "load start_params from :{}",
        start_params_file_path.to_string_lossy()
    );
    let start_params_str = tokio::fs::read_to_string(start_params_file_path).await?;
    let start_params: serde_json::Value = serde_json::from_str(&start_params_str)?;
    let start_config = StartConfigSummary::from_value(&start_params)?;

    //generate dynamic params
    let (private_key_pem, public_key_jwk) = generate_ed25519_key_pair();
    let verify_hub_public_key: Jwk = serde_json::from_value(public_key_jwk).map_err(|e| anyhow::anyhow!("invalid jwk: {}", e))?;

    // let verify_hub_public_key: Value = serde_json::from_str(&public_key_jwk.to_string()).map_err(|err| {
    //     anyhow::anyhow!("parse verify hub public key failed: {:?}", err)
    // })?;

    let mut builder = SystemConfigBuilder::new();
    builder
        .add_default_accounts(&start_config)?
        .add_user_doc(&start_config)?
        .add_default_apps(&start_config)?
        .add_device_doc(&start_config)?
        .add_system_defaults()?
        .add_verify_hub_entries(&private_key_pem)?
        .add_scheduler_service()?
        .add_gateway_settings(&start_config)?
        .add_repo_service_entries()?
        .add_smb_service()?
        .add_node_defaults()?
        .add_boot_config(&start_config, &verify_hub_public_key, zone_boot_config)?;

    let mut config = builder.build();
    if !config.contains_key("system/rbac/base_policy") {
        config.insert("system/rbac/base_policy".to_string(), rbac::DEFAULT_POLICY.to_string());
    }
    if !config.contains_key("system/rbac/model") {
        config.insert("system/rbac/model".to_string(), rbac::DEFAULT_MODEL.to_string());
    }

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

    let mut init_list = create_init_list_by_template(&zone_boot_config).await.map_err(|e| {
        error!("create_init_list_by_template failed: {:?}", e);
        e
    })?;

    let boot_config_str = init_list.get("boot/config");
    if boot_config_str.is_none() {
        return Err(anyhow::anyhow!("boot/config not found in init list"));
    }
    let boot_config_str = boot_config_str.unwrap();
    let mut zone_config: ZoneConfig = serde_json::from_str(boot_config_str.as_str()).map_err(|e| {
        error!("serde_json::from_str failed: {:?}", e);
        e
    })?;
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
        error!("boot schedule_loop failed: {:?}", boot_result.err().unwrap());
        return Err(anyhow::anyhow!("schedule_loop failed"));
    }
    system_config_client.refresh_trust_keys().await?;
    info!("system_config_service refresh trust keys success");
    
    info!("boot scheduler success");
    return Ok(());
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
                error!("buckyos-api-runtime::login failed: {:?}", e);
                e
        })?;
        set_buckyos_api_runtime(runtime);

        let scheduler_server = SchedulerServer::new();
        
        //start!
        info!("Start Scheduler Server...");
        let runner = Runner::new(SCHEDULER_SERVICE_MAIN_PORT);
        runner.add_http_server("/kapi/scheduler".to_string(), Arc::new(scheduler_server));
        runner.run().await;

        schedule_loop(false).await.map_err(|e| {
            error!("schedule_loop failed: {:?}", e);
            e
        });
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

    //#[tokio::test]
    async fn test_template() {
        let zone_boot_config = ZoneBootConfig {
            id: Some(DID::new("bns", "test")),
            oods: vec!["ood1".to_string().parse().unwrap()],
            sn: None,
            exp: 0,
            owner: None,
            extra_info: HashMap::new(),
            owner_key: None,
            gateway_devs: vec![],
            devices: HashMap::new(),
        };
        let start_params = create_init_list_by_template(&zone_boot_config).await.unwrap();
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
}
