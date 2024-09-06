#![allow(dead_code)]
#![allow(unused)]

// mod backup;
// mod backup_task_mgr;
// mod backup_task_storage;
//mod etcd_mgr;
//mod pkg_mgr;
mod run_item;
mod service_mgr;
use std::env;
use std::fmt::format;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{collections::HashMap, fs::File};
use std::path::Path;
// use tokio::*;

use buckyos_kit::{ServicePkg,ServiceState,get_buckyos_system_bin_dir};
//use etcd_client::*;
use futures::prelude::*;
use jsonwebtoken::jwk::Jwk;
use log::*;
use serde::{Deserialize, Serialize};
// use serde_json::error;
use simplelog::*;
use jsonwebtoken::{encode,decode,Header, Algorithm, Validation, EncodingKey, DecodingKey};
use sys_config::*;
use toml;
use serde_json::{from_value, json};
use name_lib::*;
// use crate::backup::*;
// use crate::backup_task_mgr::*;

use crate::run_item::*;
use crate::service_mgr::*;

use thiserror::Error;
use tide::log::start;

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
    owner_name:Option<String>,//owner's name
    device_doc_jwt:String,//device document,jwt string,siged by owner
    zone_nonce:String,// random string, is default password of some service
    //device_private_key: ,storage in partical file
}

//load from SystemConfig,node的配置分为下面几个部分
// 固定的硬件配置，一般只有硬件改变或损坏才会修改
// 系统资源情况，（比如可用内存等），改变密度很大。这一块暂时不用etcd实现，而是用专门的监控服务保存
// RunItem的配置。这块由调度器改变，一旦改变,node_daemon就会产生相应的控制命令
// Task(Cmd)配置，暂时不实现
#[derive(Serialize, Deserialize, Debug)]
struct NodeConfig {
    revision: u64,
    services: HashMap<String, ServiceConfig>,
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
    let config = ConfigBuilder::new().build();

    // 初始化日志器
    CombinedLogger::init(vec![
        // 将日志输出到标准输出，例如终端
        TermLogger::new(
            LevelFilter::Info,
            config.clone(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ),
        // 同时将日志输出到文件
        WriteLogger::new(
            LevelFilter::Info,
            config,
            File::create("/tmp/node_daemon.log").unwrap(),
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

fn load_identity_config() -> Result<(NodeIdentityConfig)> {
    //load ./node_identity.toml for debug
    //load from /opt/buckyos/etc/node_identity.toml
    let mut file_path = "node_identity.toml";
    let path = Path::new(file_path);
    if path.exists() {
        warn!("debug load node identity config from ./node_identity.toml");
    } else {
        file_path = "/opt/buckyos/etc/node_identity.toml";
    }   

    let contents = std::fs::read_to_string(file_path).map_err(|err| {
        error!("read node identity config failed! {}", err);
        return NodeDaemonErrors::ReadConfigError(String::from(file_path));
    })?;

    let config: NodeIdentityConfig = toml::from_str(&contents).map_err(|err| {
        error!("parse node identity config failed! {}", err);
        return NodeDaemonErrors::ParserConfigError(format!(
            "Failed to parse NodeIdentityConfig TOML: {}",
            err
        ));
    })?;

    info!("load node identity config from {} success!",file_path);
    Ok(config)
}

fn load_device_private_key() -> Result<(EncodingKey)> {
    let mut file_path = "device_private_key.pem";
    let path = Path::new(file_path);
    if path.exists() {
        warn!("debug load device private_key from ./device_private_key.pem");
    } else {
        file_path = "/opt/buckyos/etc/device_private_key.pem";
    }   
    let private_key = std::fs::read_to_string(file_path).map_err(|err| {
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
    let json_config_path = format!("{}.zconfig.json", node_identity.zone_name);
    let json_config = std::fs::read_to_string(json_config_path.clone());
    if json_config.is_ok() {
        let zone_config = serde_json::from_str(&json_config.unwrap());
        if zone_config.is_ok() {
            warn!("debug load zone config from {} success!",json_config_path.as_str());
            return Ok(zone_config.unwrap());
        } else {
            error!("parse debug zone config from local file failed! {}", json_config_path.as_str());
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

        let zone_jwt = name_lib::resolve(node_identity.zone_name.as_str(),Some("DID")).await
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

        
        let zone_config = ZoneConfig::decode(&zone_jwt, Some(&owner_public_key))
            .map_err(|err| {
                error!("parse zone config failed! {}", err);
                return NodeDaemonErrors::ReasonError("parse zone config failed!".to_string());
            })?;
    
        zone_did = zone_config.did;
        add_did_cache(zone_did.as_str(),zone_jwt.clone()).await.unwrap();
        info!("add zone did {} -> {:?} to cache success!",zone_did,zone_jwt);
    }  

    //try load lasted document from name_lib 
    let zone_doc: EncodedDocument = name_lib::resolve_did(zone_did.as_str(),None).await.map_err(|err| {
        error!("resolve zone did failed! {}", err);
        return NodeDaemonErrors::ReasonError("resolve zone did failed!".to_string());
    })?;

    let zone_config:ZoneConfig = ZoneConfig::decode(&zone_doc,Some(&owner_public_key)).map_err(|err| {
        error!("parse zone config failed! {}", err);
        return NodeDaemonErrors::ReasonError("parse zone config failed!".to_string());
    })?;

    return Ok(zone_config);
}

async fn generate_boot_config()->Result<serde_json::Value> {
    let mut boot_config = json!({
        "node_id": "node_id",
        "zone_name": "zone_name",
        "owner_name": "owner_name",
        "device_doc_jwt": "device_doc_jwt",
    });

    Ok(boot_config)
}


async fn get_node_config(node_host_name: &str,sys_config_client: &SystemConfigClient) -> Result<NodeConfig> {
    let json_config_path = format!("{}_node_config.json", node_host_name);
    let json_config = std::fs::read_to_string(json_config_path);
    if json_config.is_ok() {
        let node_config = NodeConfig::from_json_str(&json_config.unwrap());
        if node_config.is_ok() {
            warn!("Debug load node config from ./{}_node_config.json success!",node_host_name);
            return node_config;
        }
    }

    let sys_node_key = format!("nodes/{}/config", node_host_name);
    let (sys_cfg_result,rversion) = sys_config_client.get(sys_node_key.as_str()).await
        .map_err(|error| {
            error!("get node config failed from etcd! {}", error);
            return NodeDaemonErrors::SystemConfigError("get node config failed from etcd!".to_string());
        })?;

    let node_config = serde_json::from_value(sys_cfg_result).map_err(|err| {
        error!("parse node config failed! {}", err);
        return NodeDaemonErrors::SystemConfigError("parse node config failed!".to_string());
    })?;

    info!("load node config from system_config success!",);

    Ok(node_config)
}

async fn node_main(node_host_name: &str,
    sys_config_client: &SystemConfigClient,
    device_private_key: &EncodingKey) -> Result<bool> {

    let node_config= get_node_config(node_host_name, sys_config_client).await
        .map_err(|err| {
            error!("load node config failed! {}", err);
            return NodeDaemonErrors::SystemConfigError("cann't load node config!".to_string());
        })?;
    
    if !node_config.is_running {
        return Ok(false);
    }

    let service_stream = stream::iter(node_config.services);
    service_stream.for_each_concurrent(1, |(service_name, service_cfg)| async move {
            let target_state = service_cfg.target_state.clone();
            let _ = control_run_item_to_target_state(&service_cfg, target_state, device_private_key)
                .await
                .map_err(|_err| {
                    error!("control service item to target state failed!");
                    return NodeDaemonErrors::SystemConfigError(service_name.clone());
                });
        })
        .await;

    //service_config = system_config.get("")
    //execute_service(service_config)
    //vm_config = system_config.get("")
    //execute_vm(vm_config)
    //docker_config = system_config.get("")
    //execute_docker(docker_config)
    Ok(true)
}


async fn node_daemon_main_loop(
    node_host_name:&str,
    sys_config_client: &SystemConfigClient,
    device_private_key: &EncodingKey,
) -> Result<()> {
    let mut loop_step = 0;
    let mut is_running = true;

    loop {
        if !is_running {
            break;
        }

        loop_step += 1;
        info!("node daemon main loop step:{}", loop_step);

        let main_result = node_main(node_host_name, sys_config_client, device_private_key).await;
        if main_result.is_err() {
            error!("node_main failed! {}", main_result.err().unwrap());
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        } else {
            is_running = main_result.unwrap();
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }
    }
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

async fn async_main() -> std::result::Result<(), String> {
    init_log_config();
    info!("node_dameon start...");
    //load node identity config
    let node_identity = load_identity_config().map_err(|err| {
        error!("load node identity config failed! {}", err);
        return String::from("load node identity config failed!")
    })?;

    //verify device_doc by owner_public_key
    if node_identity.owner_name.is_some() {
        let owner_name = node_identity.owner_name.as_ref().unwrap();
        let owner_config = name_lib::resolve_did(owner_name.as_str(),None).await;
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
                error!("resolve owner public key from resolve_did failed! {}", err);
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
    
    //verify node_name is this device's hostname
    if let Ok(hostname) = std::fs::read_to_string("/etc/hostname") {
        let hostname = hostname.trim().to_string();
        if device_doc.name != hostname {
            warn!("device.hostname not match node's hostname {}!",hostname);
            //return Err("device.hostname not match node's hostname".to_string());
        }     
    }

    //load device private key
    let device_private_key = load_device_private_key().map_err(|error| {
        error!("load device private key failed! {}", error);
        return String::from("load device private key failed!");
    })?;
    
    info!("start looking zone [{}] 's config...", node_identity.zone_name.as_str());
    let zone_config = looking_zone_config(&node_identity).await.map_err(|err| {
        error!("looking zone config failed! {}", err);
        String::from("looking zone config failed!")
    })?;
    info!("Load zone document OK, {:?}", zone_config);
    info!("Booting...");


    //init system config (client)
    //if init system config failed, try to init system config service at this machine ,then try to init system config client again
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

    let device_session_token_jwt = device_session_token.generate_jwt(Some(device_doc.did.clone()),&device_private_key).map_err(|err| {
        error!("generate device session token failed! {}", err);
        return String::from("generate device session token failed!");
    })?;

    //check this is node is in ood list
    let syc_cfg_client: SystemConfigClient;
    let boot_config: serde_json::Value; 
    if zone_config.oods.contains(&device_doc.name) {
        let mut sys_config_service_pkg = ServicePkg::new("system_config_service".to_string(),get_buckyos_system_bin_dir());
        let _ = sys_config_service_pkg.load().await.map_err(|err| {
            error!("load system_config_service pkg failed! {}", err);
            return String::from("load system_config_service failed!");
        })?;
        //If this node is ood: try run / recover  system_config_service
        //  init system_config client, if kv://boot is not exist, create it and register new ood. 
        let running_state = sys_config_service_pkg.status().await.map_err(|err| {
            error!("check system_config_service running failed! {}", err);
            return String::from("check system_config_service running failed!");
        })?;

        if running_state == ServiceState::Stopped {
            warn!("check system_config_service running failed!,try to start system_config_service");
            let start_result = sys_config_service_pkg.start().await.map_err(|err| {
                error!("start system_config_service failed! {}", err);
                return String::from("start system_config_service failed!");
            })?;
        }

        let ood = vec![device_doc.name.clone()];
        syc_cfg_client = SystemConfigClient::new(&ood, &Some(device_session_token_jwt));
        let boot_config_result = syc_cfg_client.get("boot").await;
        match boot_config_result {
            sys_config::Result::Err(SystemConfigError::KeyNotFound(_)) => {
                warn!("boot config is not exist, create it!");

                boot_config = generate_boot_config().await.map_err(|err| {
                    error!("generate boot config failed! {}", err);
                    return String::from("generate boot config failed!");
                })?;


                syc_cfg_client.register_device(node_identity.device_doc_jwt.as_str(),&Some(boot_config)).await
                    .map_err(|err| {
                        error!("reigster device and set boot config failed! {}", err);
                        return String::from("set boot config failed!");
                    })?;
                info!("Register Device and Init boot config OK, wat 2 secs.");
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            },
            sys_config::Result::Ok(r) => {
                boot_config = r.0;
                info!("OOD device load boot config OK, {}", boot_config);
            },
            _ => {
                error!("get boot config failed!");
                return Err(String::from("get boot config failed!"));
            }
        }

    } else {
        //this node is not ood: try connect to system_config_service
        syc_cfg_client = SystemConfigClient::new(&zone_config.oods, &Some(device_session_token_jwt));
        let (boot_config,_) = syc_cfg_client.get("boot").await
            .map_err(|error| {
                error!("get boot config failed! {}", error);
                return String::from("get boot config failed!");
            })?;
        info!("Load boot config OK, {:?}", boot_config);
    }

    //use boot config to init name-lib.. etc kernel libs.
    info!("{}@{} boot OK, enter node daemon main loop!", device_doc.name, node_identity.zone_name);
    node_daemon_main_loop(&device_doc.name.as_str(), &syc_cfg_client, &device_private_key)
        .await
        .map_err(|err| {
            error!("node daemon main loop failed! {}", err);
            return String::from("node daemon main loop failed!");
        })?;

    Ok(())
}
