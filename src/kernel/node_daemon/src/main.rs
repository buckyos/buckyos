#![allow(dead_code)]
#![allow(unused)]


mod run_item;
mod kernel_mgr; // support manager kernel service (run in native, run for system)
mod service_mgr; // support manager frame service (run in docker,run for all users)
mod app_mgr; // support manager app service (run in docker,run for one user)
mod active_server;

use std::env;
use std::fmt::format;
use std::process::exit;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use clap::{Arg, Command};
use kernel_mgr::KernelServiceConfig;
use time::macros::format_description;
use std::{collections::HashMap, fs::File};
use std::path::{Path, PathBuf};
// use tokio::*;

use buckyos_kit::{*};

use futures::prelude::*;
use jsonwebtoken::jwk::Jwk;
use log::*;
use serde::{Deserialize, Serialize};
use sysinfo::*;

use simplelog::*;
use jsonwebtoken::{encode,decode,Header, Algorithm, Validation, EncodingKey, DecodingKey};
use sys_config::*;
use toml;
use serde_json::{from_value, json,Value};
use name_lib::*;
use name_client::*;


use crate::run_item::*;
use crate::service_mgr::*;
use crate::app_mgr::*;
use crate::kernel_mgr::*;
use crate::active_server::*;
use thiserror::Error;


#[derive(Error, Debug)]
enum NodeDaemonErrors {
    #[error("Failed due to reason: {0}")]
    ReasonError(String),
    #[error("Config file read error: {0}")]
    ReadConfigError(String),
    #[error("Config parser error: {0}")]
    ParserConfigError(String),
    #[error("SystemConfig Error: {0}")]
    SystemConfigError(String), //key
}

type Result<T> = std::result::Result<T, NodeDaemonErrors>;

//NodeIdentity from ood active progress
#[derive(Deserialize, Debug)]
struct NodeIdentityConfig {
    zone_name: String,// $name.buckyos.org or did:ens:$name
    owner_public_key: jsonwebtoken::jwk::Jwk, //owner is zone_owner
    owner_name:String,//owner's name
    device_doc_jwt:String,//device document,jwt string,siged by owner
    zone_nonce:String,// random string, is default password of some service
    //device_private_key: ,storage in partical file
}

//load from SystemConfig,node的配置分为下面几个部分
// 固定的硬件配置，一般只有硬件改变或损坏才会修改
// 系统资源情况，（比如可用内存等），改变密度很大。这一块暂时不用etcd实现，而是用专门的监控服务保存
// RunItem的配置。这块由调度器改变，一旦改变,node_daemon就会产生相应的控制命令
// Task(Cmd)配置，暂时不实现
#[derive(Serialize, Deserialize)]
struct NodeConfig {
    revision: u64,
    kernel: HashMap<String, KernelServiceConfig>,
    apps: HashMap<String, AppServiceConfig>,
    services: HashMap<String, FrameServiceConfig>,
    is_running: bool,
    
}

impl NodeConfig {
    fn from_json_str(jons_str: &str) -> Result<Self> {
        let node_config: std::result::Result<NodeConfig, serde_json::Error> =
            serde_json::from_str(jons_str);
        if node_config.is_ok() {
            return Ok(node_config.unwrap());
        }
        return Err(NodeDaemonErrors::ParserConfigError(
            "Failed to parse NodeConfig JSON".to_string(),
        ));
    }
}

fn init_log_config() {
    // 创建一个日志配置对象
    let config = ConfigBuilder::new()
        .set_time_format_custom(format_description!("[year]-[month]-[day] [hour]:[minute]:[second].[subsecond digits:3]"))
        .build();
       
    let log_path = get_buckyos_root_dir().join("logs").join("node_daemon.log");
    // 初始化日志器
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

fn load_node_private_key() -> Result<EncodingKey> {
    // load from /etc/buckyos/node_private_key.toml
    let file_path = "node_private_key.pem";
    let contents = std::fs::read_to_string(file_path).map_err(|err| {
        error!("read node private key failed! {}", err);
        return NodeDaemonErrors::ReadConfigError(String::from(file_path));
    })?;

    let private_key: EncodingKey = EncodingKey::from_ed_pem(contents.as_bytes()).map_err(|err| {
        error!("parse node private key failed! {}", err);
        return NodeDaemonErrors::ParserConfigError(format!(
            "Failed to parse node private key {}",
            err
        ));
    })?; 

    Ok(private_key)
}

fn load_identity_config(node_id: &str) -> Result<(NodeIdentityConfig)> {
    //load ./node_identity.toml for debug
    //load from /opt/buckyos/etc/node_identity.toml
    let mut file_path = PathBuf::from(format!("{}_identity.toml",node_id));
    let path = Path::new(&file_path);
    if path.exists() {
        warn!("debug load node identity config from ./node_identity.toml");
    } else {
        let etc_dir = get_buckyos_system_etc_dir();
       
        file_path = etc_dir.join(format!("{}_identity.toml",node_id));
    }   

    let contents = std::fs::read_to_string(file_path.clone()).map_err(|err| {
        error!("read node identity config failed! {}", err);
        return NodeDaemonErrors::ReadConfigError(file_path.to_string_lossy().to_string());
    })?;

    let config: NodeIdentityConfig = toml::from_str(&contents).map_err(|err| {
        error!("parse node identity config failed! {}", err);
        return NodeDaemonErrors::ParserConfigError(format!(
            "Failed to parse NodeIdentityConfig TOML: {}",
            err
        ));
    })?;

    info!("load node identity config from {} success!",file_path.to_string_lossy());
    Ok(config)
}

fn load_device_private_key(node_id: &str) -> Result<(EncodingKey)> {
    let mut file_path = format!("{}_private_key.pem",node_id);
    let path = Path::new(file_path.as_str());
    if path.exists() {
        warn!("debug load device private_key from ./device_private_key.pem");
    } else {
        let etc_dir = get_buckyos_system_etc_dir();
        file_path = format!("{}/{}_private_key.pem",etc_dir.to_string_lossy(),node_id);
    }   
    let private_key = std::fs::read_to_string(file_path.clone()).map_err(|err| {
        error!("read device private key failed! {}", err);
        return NodeDaemonErrors::ParserConfigError("read device private key failed!".to_string());
    })?;

    let private_key: EncodingKey = EncodingKey::from_ed_pem(private_key.as_bytes()).map_err(|err| {
        error!("parse device private key failed! {}", err);
        return NodeDaemonErrors::ParserConfigError("parse device private key failed!".to_string());
    })?;

    info!("load device private key from {} success!",file_path);
    Ok(private_key)
}

async fn looking_zone_config(node_identity: &NodeIdentityConfig) -> Result<ZoneConfig> {
    //If local files exist, priority loads local files
    let etc_dir = get_buckyos_system_etc_dir();
    let json_config_path = format!("{}/{}_zone_config.json",etc_dir.to_string_lossy(),node_identity.zone_name);
    info!("try load zone config from {} for debug",json_config_path.as_str());
    let json_config = std::fs::read_to_string(json_config_path.clone());
    if json_config.is_ok() {
        let zone_config = serde_json::from_str(&json_config.unwrap());
        if zone_config.is_ok() {
            warn!("debug load zone config from {} success!",json_config_path.as_str());
            return Ok(zone_config.unwrap());
        } else {
            error!("parse debug zone config {} failed! {}", json_config_path.as_str(),zone_config.err().unwrap());
            return Err(NodeDaemonErrors::ReasonError("parse debug zone config from local file failed!".to_string()));
        }
    }

    let mut zone_did = node_identity.zone_name.clone();
    info!("node_identity.owner_public_key: {:?}",node_identity.owner_public_key);
    let owner_public_key = DecodingKey::from_jwk(&node_identity.owner_public_key).map_err(
        |err| {
            error!("parse owner public key failed! {}", err);
            return NodeDaemonErrors::ReasonError("parse owner public key failed!".to_string());
        })?;

    if !name_lib::is_did(node_identity.zone_name.as_str()) {
        //owner zone is a NAME, need query NameInfo to get DID
        info!("owner zone is a NAME, try nameclient.query to get did");

        let zone_jwt = resolve(node_identity.zone_name.as_str(),RecordType::from_str("DID")).await
            .map_err(|err| {
                error!("query zone config by nameclient failed! {}", err);
                return NodeDaemonErrors::ReasonError("query zone config failed!".to_string());
            })?;

        if zone_jwt.did_document.is_none() {
            error!("get zone jwt failed!");
            return Err(NodeDaemonErrors::ReasonError("get zone jwt failed!".to_string()));
        }
        let zone_jwt = zone_jwt.did_document.unwrap();
        info!("zone_jwt: {:?}",zone_jwt);

        
        let mut zone_config = ZoneConfig::decode(&zone_jwt, Some(&owner_public_key))
            .map_err(|err| {
                error!("parse zone config failed! {}", err);
                return NodeDaemonErrors::ReasonError("parse zone config failed!".to_string());
            })?;
    
        zone_did = zone_config.did.clone();
        zone_config.owner_name = Some(node_identity.owner_name.clone());
        zone_config.name = Some(node_identity.zone_name.clone());
        let zone_config_json = serde_json::to_value(zone_config).unwrap();
        let cache_did_doc = EncodedDocument::JsonLd(zone_config_json);
        add_did_cache(zone_did.as_str(),cache_did_doc).await.unwrap();
        info!("add zone did {}  to cache success!",zone_did);
    }  

    //try load lasted document from name_lib 
    let zone_doc: EncodedDocument = resolve_did(zone_did.as_str(),None).await.map_err(|err| {
        error!("resolve zone did failed! {}", err);
        return NodeDaemonErrors::ReasonError("resolve zone did failed!".to_string());
    })?;

    let mut zone_config:ZoneConfig = ZoneConfig::decode(&zone_doc,Some(&owner_public_key)).map_err(|err| {
        error!("parse zone config failed! {}", err);
        return NodeDaemonErrors::ReasonError("parse zone config failed!".to_string());
    })?;

    if zone_config.name.is_none() {
        zone_config.name = Some(node_identity.zone_name.clone());
    }

    return Ok(zone_config);
}

async fn load_app_info(app_id: &str,username: &str,sys_config_client: &SystemConfigClient) -> Result<AppInfo> {
    let app_key = format!("users/{}/apps/{}/config", username,app_id);
    let (app_cfg_result,rversion) = sys_config_client.get(app_key.as_str()).await
        .map_err(|error| {
            let err_str = format!("get app config failed from system_config! {}", error);
            warn!("{}",err_str.as_str());
            return NodeDaemonErrors::SystemConfigError(err_str);
        })?;
    
    let app_info = serde_json::from_str(&app_cfg_result);
    if app_info.is_ok() {
        return Ok(app_info.unwrap());
    }
    let err_str = format!("parse app info failed! {}", app_info.err().unwrap());
    warn!("{}",err_str.as_str());
    return Err(NodeDaemonErrors::SystemConfigError(err_str));
}

async fn load_node_gateway_config(node_host_name: &str,sys_config_client: &SystemConfigClient) -> Result<Value> {
    let json_config_path = format!("{}_node_gateway.json", node_host_name);
    let json_config = std::fs::read_to_string(json_config_path);
    if json_config.is_ok() {
        let json_config = json_config.unwrap();
        let gateway_config = serde_json::from_str(json_config.as_str()).map_err(|err| {
            error!("parse DEBUG node gateway_config  failed! {}", err);
            return NodeDaemonErrors::SystemConfigError("parse DEBUG node gateway_config failed!".to_string());
        })?;

        warn!("Debug load node gateway_config from ./{}_node_gateway.json success!",node_host_name);
        return Ok(gateway_config);
    }

    let node_key = format!("nodes/{}/gateway", node_host_name);
    let (node_cfg_result,rversion) = sys_config_client.get(node_key.as_str()).await
        .map_err(|error| {
            error!("get node gateway_config failed from system_config_service! {}", error);
            return NodeDaemonErrors::SystemConfigError("get node gateway_config failed from system_config_service!".to_string());
        })?;

    let gateway_config = serde_json::from_str(&node_cfg_result).map_err(|err| {
        error!("parse node gateway_config failed! {}", err);
        return NodeDaemonErrors::SystemConfigError("parse gateway_config failed!".to_string());
    })?;

    Ok(gateway_config)
}

async fn load_node_config(node_host_name: &str,sys_config_client: &SystemConfigClient) -> Result<NodeConfig> {
    let json_config_path = format!("{}_node_config.json", node_host_name);
    let json_config = std::fs::read_to_string(json_config_path);
    if json_config.is_ok() {
        let json_config = json_config.unwrap();
        let node_config = serde_json::from_str(json_config.as_str()).map_err(|err| {
            error!("parse DEBUG node config failed! {}", err);
            return NodeDaemonErrors::SystemConfigError("parse DEBUG node config failed!".to_string());
        })?;

        warn!("Debug load node config from ./{}_node_config.json success!",node_host_name);
        return Ok(node_config);
    }

    let node_key = format!("nodes/{}/config", node_host_name);
    let (node_cfg_result,rversion) = sys_config_client.get(node_key.as_str()).await
        .map_err(|error| {
            error!("get node config failed from etcd! {}", error);
            return NodeDaemonErrors::SystemConfigError("get node config failed from system_config_service!".to_string());
        })?;

    let node_config = serde_json::from_str(&node_cfg_result).map_err(|err| {
        error!("parse node config failed! {}", err);
        return NodeDaemonErrors::SystemConfigError("parse node config failed!".to_string());
    })?;

    Ok(node_config)
}

async fn node_main(node_host_name: &str,
    sys_config_client: &SystemConfigClient,
    device_doc:&DeviceConfig,device_private_key: &EncodingKey) -> Result<bool> {

    //get node_gateway_config


    let target_state = RunItemTargetState::from_str("Running").unwrap();

    let node_config= load_node_config(node_host_name, sys_config_client).await
        .map_err(|err| {
            error!("load node config failed! {}", err);
            return NodeDaemonErrors::SystemConfigError("cann't load node config!".to_string());
        })?;
    
    if !node_config.is_running {
        return Ok(false);
    }

    let kernel_stream = stream::iter(node_config.kernel);
    kernel_stream.for_each_concurrent(1, |(kernel_service_name, kernel_cfg)| async move {
            let kernel_run_item = KernelServiceRunItem::new(
                &kernel_cfg,
                &device_doc,
                &device_private_key
            );
            
            let target_state = kernel_cfg.target_state.clone();

            let _ = control_run_item_to_target_state(&kernel_run_item, target_state, device_private_key)
                .await
                .map_err(|_err| {
                    error!("control kernel service item {} to target state failed!",kernel_service_name.clone());
                    return NodeDaemonErrors::SystemConfigError(kernel_service_name.clone());
                });
    }).await;
    
    // let service_stream = stream::iter(node_config.services);
    // service_stream.for_each_concurrent(1, |(service_name, service_cfg)| async move {
    //         let target_state = service_cfg.target_state.clone();
    //         let _ = control_run_item_to_target_state(&service_cfg, target_state, device_private_key)
    //             .await
    //             .map_err(|_err| {
    //                 error!("control service item to target state failed!");
    //                 return NodeDaemonErrors::SystemConfigError(service_name.clone());
    //             });
    //     })
    //     .await;

    let app_stream = stream::iter(node_config.apps);
    app_stream.for_each_concurrent(1, |(app_id_with_name, app_cfg)| async move {
        
        //let app_info = load_app_info(app_cfg.app_id.as_str(),app_cfg.username.as_str(),sys_config_client).await;
        //if app_info.is_err() {
        //    error!("load {} info failed! ,app not install?", app_cfg.app_id);
        //    return;
        //}
        //let app_info = app_info.unwrap();
        let app_run_item = AppRunItem::new(&app_cfg.app_id,app_cfg.clone(),
            device_doc,device_private_key);
        let target_state = RunItemTargetState::from_str(&app_cfg.target_state).unwrap();
        let _ = control_run_item_to_target_state(&app_run_item, target_state, device_private_key)
            .await
            .map_err(|_err| {
                error!("control app service item {} to target state failed!",app_cfg.app_id.clone());
                return NodeDaemonErrors::SystemConfigError(app_cfg.app_id.clone());
            });
    }).await;
    
    Ok(true)
}

async fn update_device_info(device_doc: &DeviceConfig,sys_config_client: &SystemConfigClient) {
    let mut device_info = DeviceInfo::from_device_doc(device_doc);
    let fill_result = device_info.auto_fill_by_system_info().await;
    if fill_result.is_err() {
        error!("auto fill device info failed! {}", fill_result.err().unwrap());
        return;
    }
    let device_info_str = serde_json::to_string(&device_info).unwrap();
    
    let device_key = format!("{}/info",sys_config_get_device_path(device_doc.name.as_str()));
    let put_result = sys_config_client.set(device_key.as_str(),device_info_str.as_str()).await;
    if put_result.is_err() {
        error!("update device info to system_config failed! {}", put_result.err().unwrap());
    } else {
        info!("update device info to system_config success!");
    }
}

async fn register_device_doc(device_doc:&DeviceConfig,sys_config_client: &SystemConfigClient) {
    let device_key = sys_config_get_device_path(device_doc.name.as_str());
    let device_doc_str = serde_json::to_string(&device_doc).unwrap();
    let device_key = format!("{}/doc",sys_config_get_device_path(device_doc.name.as_str()));
    let put_result = sys_config_client.create(device_key.as_str(),device_doc_str.as_str()).await;
    if put_result.is_err() {
        error!("register device doc to system_config failed! {}", put_result.err().unwrap());
    } else {
        info!("register device doc to system_config success!");
    }
}

async fn node_daemon_main_loop(
    node_id:&str,
    node_host_name:&str,
    sys_config_client: &SystemConfigClient,
    device_doc:&DeviceConfig,
    device_private_key: &EncodingKey,
    zone_config: &ZoneConfig
) -> Result<()> {
    let mut loop_step = 0;
    let mut is_running = true;
    let mut last_register_time = 0;

    //try regsiter device doc
    register_device_doc(device_doc, sys_config_client).await;
    let mut node_gateway_config = None;
    loop {
        if !is_running {
            break;
        }

        loop_step += 1;
        info!("node daemon main loop step:{}", loop_step);
        let now = buckyos_get_unix_timestamp();
        if now - last_register_time > 30 {
            update_device_info(&device_doc, sys_config_client).await;
            last_register_time = now;
        }


        let main_result = node_main(node_host_name, sys_config_client, device_doc, device_private_key).await;
        if main_result.is_err() {
            error!("node_main failed! {}", main_result.err().unwrap());
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        } else {
            is_running = main_result.unwrap();
            let new_node_gateway_config = load_node_gateway_config(node_host_name, sys_config_client).await;
            if new_node_gateway_config.is_ok() {
                let mut need_restart = false;
                let new_node_gateway_config = new_node_gateway_config.unwrap();
                if node_gateway_config.is_none() {
                    node_gateway_config = Some(new_node_gateway_config);
                    need_restart = true;
                } else {
                    if new_node_gateway_config == node_gateway_config.unwrap() {
                        need_restart = false;
                    } else {
                        need_restart = true;
                    }
                    node_gateway_config = Some(new_node_gateway_config);
                }
                
                if need_restart {
                    info!("node gateway_config changed, update node_gateway_config!");
                    let gateway_config_path = buckyos_kit::get_buckyos_system_etc_dir().join("node_gateway.json");
                    std::fs::write(gateway_config_path, serde_json::to_string(&node_gateway_config).unwrap()).unwrap();
                    start_cyfs_gateway_service(node_id,&device_doc, &device_private_key,&zone_config,true).await.map_err(|err| {
                        error!("start cyfs_gateway service failed! {}", err);
                    }); 
                }
            } else {
                error!("load node gateway_config failed!");
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    }
    Ok(())
}


async fn do_boot_scheduler() -> std::result::Result<(),String> {
    let mut scheduler_pkg = ServicePkg::new("scheduler".to_string(),get_buckyos_system_bin_dir());
    let _ = scheduler_pkg.load().await.map_err(|err| {
        error!("load scheduler pkg failed! {}", err);
        return String::from("load scheduler pkg failed!");
    })?;

    let params = vec!["--boot".to_string()];
    let start_result = scheduler_pkg.start(Some(&params)).await.map_err(|err| {
        error!("start scheduler pkg failed! {}", err);
        return String::from("start scheduler pkg failed!");
    })?;

    if start_result == 0 {
        info!("boot run scheduler pkg success!");
        Ok(())
    } else {
        error!("boot run run scheduler pkg failed!");
        Err(String::from("run scheduler pkg failed!"))
    }
    
}

//if register OK then return sn's URL
async fn start_update_ood_info_to_sn(device_doc: &DeviceConfig, device_token_jwt: &str,zone_config: &ZoneConfig) -> std::result::Result<String,String> {
    //try register ood's device_info to sn,
    // TODO: move this logic to cyfs-gateway service?
    let mut need_sn = false;
    let mut sn_url = zone_config.get_sn_url();
    if sn_url.is_some() {
        need_sn = true;
    } else {
        if device_doc.ddns_sn_url.is_some() {
            need_sn = true;
            sn_url = device_doc.ddns_sn_url.clone();
        }
    }
    if !need_sn {
        return Err(String::from("no sn url to update ood info!"));
    }
    
    let sn_url = sn_url.unwrap();

    let ood_string = zone_config.get_ood_string(device_doc.name.as_str());
    if ood_string.is_none() {
        error!("this device is not in zone's ood list!");
        return Err(String::from("this device is not in zone's ood list!"));
    }

    let ood_string = ood_string.unwrap();
    let mut ood_info = DeviceInfo::from_device_doc(&device_doc);
    let fill_result =ood_info.auto_fill_by_system_info().await;
    if fill_result.is_err() {
        error!("auto fill ood info failed! {}", fill_result.err().unwrap());
        return Err(String::from("auto fill ood info failed!"));
    }

    info!("ood info: {:?}",ood_info);

    sn_update_device_info(sn_url.as_str(), Some(device_token_jwt.to_string()), 
    &zone_config.get_zone_short_name(),device_doc.name.as_str(), &ood_info, ).await;

    info!("update ood info to sn {} success!",sn_url.as_str());
    Ok(sn_url)
}

async fn start_cyfs_gateway_service(node_id: &str,device_doc: &DeviceConfig, device_private_key: &EncodingKey,zone_config: &ZoneConfig,is_restart:bool) -> std::result::Result<(),String> {
    let mut cyfs_gateway_service_pkg = ServicePkg::new("cyfs_gateway".to_string(),get_buckyos_system_bin_dir());
    let _ = cyfs_gateway_service_pkg.load().await.map_err(|err| {
        error!("load cyfs_gateway service pkg failed! {}", err);
        return String::from("load cyfs_gateway service pkg failed!");
    })?;

    let mut running_state = cyfs_gateway_service_pkg.status(None).await.map_err(|err| {
        error!("check cyfs_gateway running failed! {}", err);
        return String::from("check cyfs_gateway running failed!");
    })?;    
    if is_restart {
        cyfs_gateway_service_pkg.stop(None).await.map_err(|err| {
            error!("stop cyfs_gateway service failed! {}", err);
            return String::from("stop cyfs_gateway service failed!");
        })?;
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        running_state = ServiceState::Stopped;
    }

    if running_state == ServiceState::Stopped {
        warn!("check cyfs_gateway is stopped,try to start cyfs_gateway");
        //params: boot cyfs-gateway configs, identiy_etc folder, keep_tunnel list 
        //  ood: keep tunnel to other ood, keep tunnel to gateway
        //  gateway_config: port_forward for system_config service 
        let params : Vec<String>;
        if zone_config.sn.is_some() {
            let sn_url = zone_config.sn.as_ref().unwrap();
            params = vec!["--node_id".to_string(),node_id.to_string(),"--keep_tunnel".to_string(),sn_url.clone()];
        } else {
            params = vec!["--node_id".to_string(),node_id.to_string()];
        }
        let start_result = cyfs_gateway_service_pkg.start(Some(&params)).await.map_err(|err| {
            error!("start cyfs_gateway failed! {}", err);
            return String::from("start cyfs_gateway failed!");
        })?;

        info!("start cyfs_gateway OK!,result:{}. wait 5 seconds...",start_result);
        //wait 5 seconds
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    }

    Ok(())
}


async fn async_main() -> std::result::Result<(), String> {
    init_log_config();
    let matches = Command::new("BuckyOS Node Daemon")
    .arg(
        Arg::new("id")
            .long("node_id")
            .help("This node's id")
            .required(false),
    )
    .arg(
        Arg::new("enable_active")
            .long("enable_active")
            .help("Enable node active service")
            .action(clap::ArgAction::SetTrue)
            .required(false),
    )
    .get_matches();

    let node_id = matches.get_one::<String>("id");
    let enable_active = matches.get_flag("enable_active");
    let defualt_node_id = "node".to_string();
    let node_id = node_id.unwrap_or(&defualt_node_id);

    info!("node_daemon start...");
    //load node identity config
    let mut node_identity = load_identity_config(node_id);
    if node_identity.is_err() {
        if enable_active {
            info!("node identity config not found, start node active service...");
            start_node_active_service().await;
            info!("node active service returned,exit node_daemon.");
            exit(0);
            //restart_program();
        } else {   
            error!("load node identity config failed! {}", node_identity.err().unwrap());
            return Err(String::from("load node identity config failed!"));
        }
    }

    let node_identity = node_identity.unwrap();

    init_default_name_client().await.map_err(|err| {
        error!("init default name client failed! {}", err);
        return String::from("init default name client failed!");
    })?;

    //verify device_doc by owner_public_key
    {
        let owner_name = node_identity.owner_name.as_ref();
        let owner_config = resolve_did(owner_name,None).await;
        match owner_config {
            Ok(owner_config) => {
                let owner_config = OwnerConfig::decode(&owner_config,None);
                if owner_config.is_ok() {
                    let owner_config = owner_config.unwrap();
                    if owner_config.auth_key != node_identity.owner_public_key {
                        warn!("owner public key not match! ");
                    }
                }
            }
            Err(err) => {
                error!("resolve owner public key by resolve_did failed! {}", err);
            }
        }
    }

    let device_doc_json = decode_json_from_jwt_with_default_pk(&node_identity.device_doc_jwt, &node_identity.owner_public_key)
        .map_err(|err| {
            error!("decode device doc failed! {}", err);
            return String::from("decode device doc from jwt failed!");
        })?;
    let device_doc : DeviceConfig = serde_json::from_value(device_doc_json).map_err(|err| {
        error!("parse device doc failed! {}", err);
        return String::from("parse device doc failed!");
    })?;
    info!("current node's device doc: {:?}", device_doc);
    
    //load device private key
    let device_private_key = load_device_private_key(&node_id).map_err(|error| {
        error!("load device private key failed! {}", error);
        return String::from("load device private key failed!");
    })?;
    
    info!("start refresh zone [{}] 's config...", node_identity.zone_name.as_str());
    let zone_config = looking_zone_config(&node_identity).await.map_err(|err| {
        error!("looking zone config failed! {}", err);
        String::from("looking zone config failed!")
    })?;
    info!("Load Zone document OK, {:?}", zone_config);

    //verify node_name is this device's hostname 
    let node_host_name = zone_config.get_node_host_name(&device_doc.name);
    let hostname = sysinfo::System::host_name();
    if hostname.is_some() {
        let hostname = hostname.unwrap();
        if node_host_name != hostname {
            warn!("device.hostname:{} not match node's hostname: {}!",device_doc.name,hostname);
        } 
    }

    let is_ood = zone_config.oods.contains(&device_doc.name);
    name_lib::CURRENT_ZONE_CONFIG.set(zone_config).unwrap();
    if is_ood {
        info!("Booting OOD {}......",node_host_name);
    } else {
        info!("Booting Node {}......",node_host_name);
    }

    let zone_config = name_lib::CURRENT_ZONE_CONFIG.get().unwrap();
    std::env::set_var("BUCKY_ZONE_OWNER", serde_json::to_string(&node_identity.owner_public_key).unwrap());
    std::env::set_var("BUCKY_ZONE_CONFIG", serde_json::to_string(&zone_config).unwrap());
    std::env::set_var("BUCKY_THIS_DEVICE", serde_json::to_string(&device_doc).unwrap());

    info!("set var BUCKY_ZONE_OWNER to {}", env::var("BUCKY_ZONE_OWNER").unwrap());
    info!("set var BUCKY_ZONE_CONFIG to {}", env::var("BUCKY_ZONE_CONFIG").unwrap());
    info!("set var BUCKY_THIS_DEVICE to {}", env::var("BUCKY_THIS_DEVICE").unwrap());

    let now = SystemTime::now();
    let since_the_epoch = now.duration_since(UNIX_EPOCH)
        .expect("Time went backwards");
    let timestamp = since_the_epoch.as_secs();

    let device_session_token = kRPC::RPCSessionToken {
        token_type : kRPC::RPCSessionTokenType::JWT,
        nonce : None,
        userid : Some(device_doc.name.clone()),
        appid:Some("kernel".to_string()),
        exp:Some(timestamp + 3600*24*7),
        iss:Some(device_doc.name.clone()),
        token:None,
    };

    let device_session_token_jwt = device_session_token.generate_jwt(Some(device_doc.did.clone()),&device_private_key)
        .map_err(|err| {
        error!("generate device session token failed! {}", err);
        return String::from("generate device session token failed!");})?;

    let device_info = DeviceInfo::from_device_doc(&device_doc);
    enable_zone_provider(Some(&device_info),Some(&device_session_token_jwt),false).await.map_err(|err| {
        error!("enable zone provider failed! {}", err);
        return String::from("enable zone provider failed!");
    })?;
    
    //init kernel_service:cyfs-gateway service
    std::env::set_var("GATEWAY_SESSIONT_TOKEN",device_session_token_jwt.clone());
    info!("set var GATEWAY_SESSIONT_TOKEN to {}", device_session_token_jwt);
    start_cyfs_gateway_service(node_id.as_str(),&device_doc, &device_private_key,&zone_config,false).await.map_err(|err| {
        error!("init cyfs_gateway service failed! {}", err);
        return String::from("init cyfs_gateway service failed!");
    })?;

    //init kernel_service:system_config service
    let mut syc_cfg_client: SystemConfigClient;
    let boot_config: serde_json::Value; 
    if is_ood {
        start_update_ood_info_to_sn(&device_doc, &device_session_token_jwt.as_str(),&zone_config).await;

        let mut sys_config_service_pkg = ServicePkg::new("system_config".to_string(),get_buckyos_system_bin_dir());
        let _ = sys_config_service_pkg.load().await.map_err(|err| {
            error!("load system_config_service pkg failed! {}", err);
            return String::from("load system_config_service failed!");
        })?;
        //If this node is ood: try run / recover  system_config_service
        //  init system_config client, if kv://boot is not exist, create it and register new ood. 
        let running_state = sys_config_service_pkg.status(None).await.map_err(|err| {
            error!("check system_config_service running failed! {}", err);
            return String::from("check system_config_service running failed!");
        })?;

        if running_state == ServiceState::Stopped {
            warn!("check system_config_service is stopped,try to start system_config_service");
            let start_result = sys_config_service_pkg.start(None).await.map_err(|err| {
                error!("start system_config_service failed! {}", err);
                return String::from("start system_config_service failed!");
            })?;
            info!("start system_config_service OK!,result:{},wait 5 seconds...",start_result);
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }

        syc_cfg_client = SystemConfigClient::new(None, Some(device_session_token_jwt.as_str()));
        let boot_config_result = syc_cfg_client.get("boot/config").await;
        match boot_config_result {
            sys_config::Result::Err(SystemConfigError::KeyNotFound(_)) => {
                warn!("boot config is not exist, try first scheduler to generate it!");
                std::env::set_var("SCHEDULER_SESSION_TOKEN", device_session_token_jwt.clone());
                info!("set var SCHEDULER_SESSION_TOKEN {}", device_session_token_jwt);
                do_boot_scheduler().await.map_err(|err| {
                    error!("do boot scheduler failed! {}", err);
                    return String::from("do boot scheduler failed!");
                })?;
                info!("Init boot config OK, wat 2 secs.");
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            },
            sys_config::Result::Ok(r) => {
                boot_config = serde_json::from_str(r.0.as_str()).map_err(|err| {
                    error!("parse boot config failed! {}", err);
                    return String::from("parse boot config failed!");
                })?;
                info!("OOD device load boot config OK, {}", boot_config);
            },
            _ => {
                error!("get boot config failed!");
                return Err(format!("get boot config failed! {}", boot_config_result.err().unwrap()));
            }
        }

    } else {
        //this node is not ood: try connect to system_config_service
        let this_device = DeviceInfo::from_device_doc(&device_doc);
        let system_config_url = get_system_config_service_url(Some(&this_device),&zone_config,false).await.map_err(|err| {
            error!("get system_config_url failed! {}", err);
            return String::from("get system_config_url failed!");
        })?;
        loop {
            syc_cfg_client = SystemConfigClient::new(Some(system_config_url.as_str()), Some(device_session_token_jwt.as_str()));
            let boot_config_result = syc_cfg_client.get("boot").await;
            if boot_config_result.is_ok() {
                info!("Connect to system_config_service and load boot config OK! boot config: {:?}", boot_config_result.unwrap().0.as_str());
                break;
            } else {
                warn!("Connect to system_config_service failed! {}", boot_config_result.err().unwrap());
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }
        }
    }

    //use boot config to init name-lib.. etc kernel libs.
    let gateway_ip = resolve_ip("gateway").await;
    info!("gateway ip: {:?}", gateway_ip);

    info!("{}@{} boot OK, enter node daemon main loop!", device_doc.name, node_identity.zone_name);
    node_daemon_main_loop(node_id,&device_doc.name.as_str(), &syc_cfg_client, &device_doc, &device_private_key, &zone_config)
        .await
        .map_err(|err| {
            error!("node daemon main loop failed! {}", err);
            return String::from("node daemon main loop failed!");
        })?;

    Ok(())
}

fn main() {
    if num_cpus::get() < 2 {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
            .block_on(async_main());
    } else {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async_main());
    }
}
