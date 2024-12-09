use std::collections::HashMap;
use std::process::exit;
use log::*;
use serde_json::json;
//use upon::Engine;

use name_lib::*;
use name_client::*;
use buckyos_kit::*;
use sys_config::SystemConfigClient;


type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

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

    info!("parsed template: {}", result);

    //wwrite result to file
    //let result_file_path = get_buckyos_system_etc_dir().join("scheduler_boot.toml");
    //tokio::fs::write(result_file_path, result.clone()).await?;

    let config: HashMap<String, String> = toml::from_str(&result)?;
    
    Ok(config)
}


async fn do_boot_scheduler() -> Result<()> {
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
        info!("on boot, create system config {} => {}", key, value);
    }
    info!("boot scheduler success");
    return Ok(());
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