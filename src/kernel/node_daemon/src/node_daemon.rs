use std::env;
use std::fmt::format;
use std::process::exit;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use time::macros::format_description;
use tokio::sync::RwLock;
use std::{collections::HashMap, fs::File};
use std::path::{Path, PathBuf};
use lazy_static::lazy_static;

use futures::prelude::*;
use simplelog::*;
use log::*;
use serde::{Deserialize, Serialize};
use serde_json::{from_value, json, Value};
use toml;
use clap::{Arg, ArgMatches, Command};

use jsonwebtoken::{encode,decode,Header, Algorithm, Validation, EncodingKey, DecodingKey};
use jsonwebtoken::jwk::Jwk;
use buckyos_api::*;
use ndn_lib::*;
use name_client::*;
use name_lib::*;
use buckyos_kit::*;
use package_lib::*;

use crate::run_item::*;
use crate::frame_service_mgr::*;
use crate::app_mgr::*;
use crate::kernel_mgr::*;
use crate::active_server::*;
use crate::service_pkg::*;
use buckyos_api::*;
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

async fn looking_zone_boot_config(node_identity: &NodeIdentityConfig) -> Result<ZoneBootConfig> {
    //If local files exist, priority loads local files
    let etc_dir = get_buckyos_system_etc_dir();
    let json_config_path = etc_dir.join(format!("{}.zone.json",node_identity.zone_did.to_host_name()));
    info!("check  {} is exist for debug ...", json_config_path.display());
    let mut zone_boot_config: ZoneBootConfig;;
    //在离线环境中，可以利用下面机制来绕开DNS查询
    if json_config_path.exists() {
        info!("try load zone boot config from {} for debug", json_config_path.display());
        let json_config = std::fs::read_to_string(json_config_path.clone());
        if json_config.is_ok() {
            let zone_boot_config_result = serde_json::from_str(&json_config.unwrap());
            if zone_boot_config_result.is_ok() {
                warn!("debug load zone boot config from {} success!", json_config_path.display());
                zone_boot_config = zone_boot_config_result.unwrap();
            } else {
                error!("parse debug zone boot config {} failed! {}", json_config_path.display(), zone_boot_config_result.err().unwrap());
                return Err(NodeDaemonErrors::ReasonError("parse debug zone boot config from local file failed!".to_string()));
            }
        } else {
            return Err(NodeDaemonErrors::ReasonError("parse debug zone boot config from local file failed!".to_string()));
        }
    } else {
        let mut zone_did = node_identity.zone_did.clone();
        info!("node_identity.owner_public_key: {:?}",node_identity.owner_public_key);
        let owner_public_key = DecodingKey::from_jwk(&node_identity.owner_public_key).map_err(
            |err| {
                error!("parse owner public key failed! {}", err);
                return NodeDaemonErrors::ReasonError("parse owner public key failed!".to_string());
            })?;

        //owner zone is a NAME, need query NameInfo to get DID
        // info!("owner zone is a NAME, try nameclient.query to get did");
        // let zone_jwt = resolve(node_identity.zone_did.as_str(),RecordType::from_str("DID")).await
        //     .map_err(|err| {
        //         error!("query zone config by nameclient failed! {}", err);
        //         return NodeDaemonErrors::ReasonError("query zone config failed!".to_string());
        //     })?;
        // let owner_from_resolve = zone_jwt.get_owner_pk();
        // if owner_from_resolve.is_some() {
        //     let owner_from_resolve = owner_from_resolve.unwrap();
        //     //if owner_public_key != owner_from_resolve {
        //     //    error!("owner public key from resolve is not match!");
        //     //    return Err(NodeDaemonErrors::ReasonError("owner public key from resolve is not match!".to_string()));
        //     //}
        // }
        // if zone_jwt.did_document.is_none() {
        //     error!("get zone jwt failed!");
        //     return Err(NodeDaemonErrors::ReasonError("get zone jwt failed!".to_string()));
        // }
        // let zone_jwt = zone_jwt.did_document.unwrap();
        // info!("zone_jwt: {:?}",zone_jwt);
        

        let zone_doc = resolve_did(&node_identity.zone_did,None).await.map_err(|err| {
            error!("resolve zone did failed! {}", err);
            return NodeDaemonErrors::ReasonError("resolve zone did failed!".to_string());
        })?;


        zone_boot_config = ZoneBootConfig::decode(&zone_doc, Some(&owner_public_key))
            .map_err(|err| {
                error!("parse zone config failed! {}", err);
                return NodeDaemonErrors::ReasonError("parse zone config failed!".to_string());
            })?;
    }
    
    zone_boot_config.id = Some(node_identity.zone_did.clone());
    if node_identity.zone_iat > zone_boot_config.iat {
        error!("zone_boot_config.iat is earlier than node_identity.zone_iat!");
        return Err(NodeDaemonErrors::ReasonError("zone_boot_config.iat is not match!".to_string()));
    }
    
    if zone_boot_config.owner.is_some() {
        if zone_boot_config.owner.as_ref().unwrap() != & node_identity.owner_did {
            error!("zone boot config's owner is not match node_identity's owner_did!");
            return Err(NodeDaemonErrors::ReasonError("zone owner is not match!".to_string()));
        }
    } else {
        zone_boot_config.owner = Some(node_identity.owner_did.clone());
    }
    zone_boot_config.owner_key = Some(node_identity.owner_public_key.clone());


    //zone_config.name = Some(node_identity.zone_did.clone());
    //let zone_config_json = serde_json::to_value(zone_config.clone()).unwrap();
    //let cache_did_doc = EncodedDocument::JsonLd(zone_config_json);
    //add_did_cache(zone_did,cache_did_doc).await.unwrap();
    //info!("add zone did {}  to cache success!",zone_did.to_string());
    //try load lasted document from name_lib
    // let zone_doc: EncodedDocument = resolve_did(zone_did.as_str(),None).await.map_err(|err| {
    //     error!("resolve zone did failed! {}", err);
    //     return NodeDaemonErrors::ReasonError("resolve zone did failed!".to_string());
    // })?;
    // let mut zone_config:ZoneConfig = ZoneConfig::decode(&zone_doc,Some(&owner_public_key)).map_err(|err| {
    //     error!("parse zone config failed! {}", err);
    //     return NodeDaemonErrors::ReasonError("parse zone config failed!".to_string());
    // })?;
    // if zone_config.name.is_none() {
    //     zone_config.name = Some(node_identity.zone_did.clone());
    // }

    return Ok(zone_boot_config);
}

async fn load_app_info(app_id: &str,username: &str,buckyos_api_client: &SystemConfigClient) -> Result<AppDoc> {
    let app_key = format!("users/{}/apps/{}/config", username,app_id);
    let get_result = buckyos_api_client.get(app_key.as_str()).await
        .map_err(|error| {
            let err_str = format!("get app config failed from system_config! {}", error);
            warn!("{}",err_str.as_str());
            return NodeDaemonErrors::SystemConfigError(err_str);
        })?;

    let app_info = serde_json::from_str(&get_result.value);
    if app_info.is_ok() {
        return Ok(app_info.unwrap());
    }
    let err_str = format!("parse app info failed! {}", app_info.err().unwrap());
    warn!("{}",err_str.as_str());
    return Err(NodeDaemonErrors::SystemConfigError(err_str));
}

async fn load_node_gateway_config(node_host_name: &str,buckyos_api_client: &SystemConfigClient) -> Result<Value> {
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

    let node_key = format!("nodes/{}/gateway_config", node_host_name);
    let get_result = buckyos_api_client.get(node_key.as_str()).await
        .map_err(|error| {
            error!("get node gateway_config failed from system_config_service! {}", error);
            return NodeDaemonErrors::SystemConfigError("get node gateway_config failed from system_config_service!".to_string());
        })?;

    let gateway_config = serde_json::from_str(&get_result.value).map_err(|err| {
        error!("parse node gateway_config failed! {}", err);
        return NodeDaemonErrors::SystemConfigError("parse gateway_config failed!".to_string());
    })?;

    Ok(gateway_config)
}

async fn load_node_config(node_host_name: &str,buckyos_api_client: &SystemConfigClient) -> Result<NodeConfig> {
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
    let get_result = buckyos_api_client.get(node_key.as_str()).await
        .map_err(|error| {
            error!("get node config failed from etcd! {}", error);
            return NodeDaemonErrors::SystemConfigError("get node config failed from system_config_service!".to_string());
        })?;

    let node_config = serde_json::from_str(&get_result.value).map_err(|err| {
        error!("parse node config failed! {}", err);
        return NodeDaemonErrors::SystemConfigError("parse node config failed!".to_string());
    })?;

    Ok(node_config)
}


//node_daemon的系统升级流程有2种 （所有的升级都是非强制的）
//1. 在首次使用时，尝试升级到最新版本，比较适合一些硬件用户，在恢复出厂设置后，依旧可以在使用前将系统升级到最新版本，减少后续版本迁移的负担
//2. 常规升级，repo-server先从源获得新版本的index-db和已安装Pkg的chunk,然后任意node上的node_daemon都会触发升级
//    注意升级是zone级别的，这意味着用户对升级和系统的版本控制只会通过repo-server的settings来控制
//    在非开发模式下，各个node上的软件版本需无条件保持与repo-server的设置一致。
async fn do_boot_upgreade() -> std::result::Result<(), String>  {
    //TODO: 
    Ok(())
}


async fn check_and_update_system_pkgs(pkg_list: Vec<String>,session_token: Option<String>) -> std::result::Result<bool, String>  {
    let mut is_self_upgrade = false;
    let mut pkg_env = PackageEnv::new(get_buckyos_system_bin_dir());
    for pkg_id in pkg_list {

        let media_info = pkg_env.load(&pkg_id).await;
        if media_info.is_err() {
            info!("check_and_update_system_pkgs: pkg {} not exist, deploy it", pkg_id);
            let result = pkg_env.install_pkg(&pkg_id, true,false).await;
            if result.is_err() {
                error!("check_and_update_system_pkgs: deploy pkg {} failed! {}", pkg_id, result.err().unwrap());
            }
            if pkg_id == "node_daemon" {
                is_self_upgrade = true;
            }
            info!("check_and_update_system_pkgs: deploy pkg {} success", pkg_id);
        }
    }
    Ok(is_self_upgrade)
}

async fn make_sure_system_pkgs_ready(meta_db_path: &PathBuf,prefix: &str,session_token: Option<String>) -> std::result::Result<(), String> {
    let system_pkgs = vec![
        "app_loader".to_string(),
        "node_active".to_string(),
        "buckycli".to_string(),
        "control_panel".to_string(),
        "repo_service".to_string(),
        "cyfs_gateway".to_string(),
        "system_config".to_string(),
        "verify_hub".to_string(),
    ];
    let mut miss_chunk_list = vec![];
    for pkg_id in system_pkgs {
        let pkg_id = format!("{}.{}",prefix,pkg_id);
        let check_result = PackageEnv::check_pkg_ready(meta_db_path, pkg_id.as_str(), 
        None, &mut miss_chunk_list).await;
        if check_result.is_err() {
            error!("make_sure_system_pkgs_ready: pkg {} is not ready! {}", pkg_id, check_result.err().unwrap());
            return Err(String::from("pkg is not ready!"));  
        }
    }
    
    for chunk_id in miss_chunk_list {
        let ndn_client = NdnClient::new("http://127.0.0.1/ndn/".to_string(), session_token.clone(),None);
        let chunk_result = ndn_client.pull_chunk(chunk_id.clone(),None).await;
        if chunk_result.is_err() {
            error!("make_sure_system_pkgs_ready: pull chunk {} failed! {}", chunk_id.to_string(), chunk_result.err().unwrap());
        }
        info!("make_sure_system_pkgs_ready: pull chunk {} success", chunk_id.to_string());
    }


    Ok(())
}

async fn check_and_update_root_pkg_index_db(session_token: Option<String>) -> std::result::Result<bool, String>  {
    let root_env_path = BuckyOSRuntime::get_root_pkg_env_path();
    let mut root_env = PackageEnv::new(root_env_path.clone());
    let meta_db_file_patgh = root_env_path.join("pkgs").join("meta_index.db");

    let zone_repo_index_db_url = "http://127.0.0.1/ndn/repo/meta_index.db";
    let ndn_client = NdnClient::new("http://127.0.0.1/ndn/".to_string(), session_token.clone(),None);
    
    let local_is_better = ndn_client.local_is_better(zone_repo_index_db_url,&meta_db_file_patgh).await;
    if local_is_better.is_ok() && local_is_better.unwrap() {
        info!("local meta-index.db is better than repo's default meta-index.db, no need to update!");
        return Ok(false);
    }

    info!("remote index db is not same as local index db, start update node's root_pkg_env.meta_index.db");
    let download_path = root_env_path.join("pkgs").join("meta_index.downloading");
    ndn_client.download_fileobj_to_local(zone_repo_index_db_url, &download_path, None).await
        .map_err(|err| {
            error!("download remote index db to root pkg env's meta-Index db failed! {}", err);
            return String::from("download remote index db to root pkg env's meta-Index db failed!");
        })?;
    info!("download new meta-index.db success,update root env's meta-index.db..");

    let prefix = root_env.get_prefix();
    make_sure_system_pkgs_ready(&download_path, &prefix, session_token.clone()).await?;
    root_env.try_update_index_db(&download_path).await
        .map_err(|err| {
            error!("update root pkg env's meta-Index db failed! {}", err);
            return String::from("update root pkg env's meta-Index db failed!");
        })?;
    
    info!("update root pkg env's meta-index.db OK");
    let remove_result = std::fs::remove_file(download_path);
    if remove_result.is_err() {
        warn!("remove meta_index.downloading error!");
    }

    Ok(true)
}

async fn update_device_info(device_info: &DeviceInfo,syste_config_client: &SystemConfigClient) {
    let device_key = format!("devices/{}/info", device_info.name.as_str());
    let device_info_str = serde_json::to_string(&device_info).unwrap();
    let put_result = syste_config_client.set(device_key.as_str(),device_info_str.as_str()).await;
    if put_result.is_err() {
        error!("update device info to system_config failed! {}", put_result.err().unwrap());
    } else {
        info!("update {} info to system_config success!",device_info.name.as_str());
    }
}

//if register OK then return sn's real URL for this user
async fn report_ood_info_to_sn(device_info: &DeviceInfo, device_token_jwt: &str,zone_config: &ZoneConfig) -> std::result::Result<(),String> {
    let mut need_sn = false;
    let mut sn_url = zone_config.get_sn_api_url();
    if sn_url.is_some() {
        need_sn = true;
    } else {
        if device_info.ddns_sn_url.is_some() {
            need_sn = true;
            sn_url = device_info.ddns_sn_url.clone();
        }
    }
    if !need_sn {
        return Ok(());
    }

    let sn_url = sn_url.unwrap();

    let ood_string = zone_config.get_ood_desc_string(device_info.name.as_str());
    if ood_string.is_none() {
        error!("this device is not in zone's ood list!");
        return Err(String::from("this device is not in zone's ood list!"));
    }

    let ood_string = ood_string.unwrap();

    sn_update_device_info(sn_url.as_str(), Some(device_token_jwt.to_string()),
                          &zone_config.get_zone_short_name(),device_info.name.as_str(), &device_info).await;

    info!("update {} 's info to sn {} success!",device_info.name.as_str(),sn_url.as_str());
    Ok(())
}

async fn do_boot_schedule() -> std::result::Result<(),String> {
    let mut scheduler_pkg = ServicePkg::new("scheduler".to_string(),get_buckyos_system_bin_dir());
    if !scheduler_pkg.try_load().await {
        error!("load scheduler pkg failed!");
        return Err(String::from("load scheduler pkg failed!"));
    }

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

async fn wait_sysmte_config_sync() -> std::result::Result<(),String> {
    return Ok(());
}

async fn keep_system_config_service(node_id: &str,device_doc: &DeviceConfig, device_private_key: &EncodingKey,is_restart:bool) -> std::result::Result<(),String> {
    let mut system_config_service_pkg = ServicePkg::new("system_config".to_string(),get_buckyos_system_bin_dir());

    if !system_config_service_pkg.try_load().await {
        error!("load system_config_service pkg failed!");
        let mut env = PackageEnv::new(get_buckyos_system_bin_dir());
        let result = env.install_pkg("system_config", false,false).await;
        if result.is_err() {
            error!("install system_config_service pkg failed! {}", result.err().unwrap());
            return Err(String::from("install system_config_service pkg failed!"));
        } else {
            info!("install system_config_service pkg success");
        }
    } 

    let mut running_state = system_config_service_pkg.status(None).await.map_err(|err| {
        error!("check system_config_service running failed! {}", err);
        return String::from("check system_config_service running failed!");
    })?;

    if is_restart {
        system_config_service_pkg.stop(None).await.map_err(|err| {
            error!("stop system_config_service failed! {}", err);
            return String::from("stop system_config_service failed!");
        })?;
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        running_state = ServiceState::Stopped;
    }

    if running_state == ServiceState::Stopped {
        warn!("check system_config_service is stopped,try to start system_config_service");
        let start_result = system_config_service_pkg.start(None).await.map_err(|err| {
            error!("start system_config_service failed! {}", err);
            return String::from("start system_config_service failed!");
        })?;
        info!("start system_config_service OK!,result:{}",start_result);
    }
    Ok(())
}


async fn keep_cyfs_gateway_service(node_id: &str,device_doc: &DeviceConfig, node_private_key: &EncodingKey,sn: Option<String>,is_restart:bool) -> std::result::Result<(),String> {
    //TODO: 需要区分boot模式和正常模式
    let mut cyfs_gateway_service_pkg = ServicePkg::new("cyfs_gateway".to_string(),get_buckyos_system_bin_dir());
 
    let mut running_state = cyfs_gateway_service_pkg.status(None).await.map_err(|err| {
        error!("check cyfs_gateway running failed! {}", err);
        return String::from("check cyfs_gateway running failed!");
    })?;
    debug!("cyfs_gateway service pkg status: {:?}",running_state);

    if running_state == ServiceState::NotExist {        
        //当版本更新后,上述检查会返回NotExist, 并触发更新操作
        warn!("cyfs_gateway service pkg not exist, try install it...");
        let mut env = PackageEnv::new(get_buckyos_system_bin_dir());
        let result = env.install_pkg("cyfs_gateway", false,false).await;
        if result.is_err() {
            error!("install cyfs_gateway service pkg failed! {}", result.err().unwrap());
            return Err(String::from("install cyfs_gateway service pkg failed!"));
        } else {
            info!("install cyfs_gateway service pkg success");
        }      
    }
    
    if is_restart {
        info!("cyfs_gateway service pkg loaded, will do restart ...");
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
        let mut need_keep_tunnel_to_sn = false;
        if sn.is_some() {
            need_keep_tunnel_to_sn = true;
            if device_doc.net_id.is_some() {
                let net_id = device_doc.net_id.as_ref().unwrap();
                if net_id == "wan" {
                    need_keep_tunnel_to_sn = false;
                }
            }
        }

        if need_keep_tunnel_to_sn {
            let device_did = device_doc.id.to_string();
            let sn_host_name = get_real_sn_host_name(sn.as_ref().unwrap(),device_did.as_str()).await
                .map_err(|err| {
                    error!("get sn host name failed! {}", err);
                    return String::from("get sn host name failed!");
                })?;
            params = vec!["--keep_tunnel".to_string(),sn_host_name.clone()];
        } else {
            params = Vec::new();
        }

        let start_result = cyfs_gateway_service_pkg.start(Some(&params)).await.map_err(|err| {
            error!("start cyfs_gateway failed! {}", err);
            return String::from("start cyfs_gateway failed!");
        })?;

        info!("start cyfs_gateway OK!,result:{}. wait 2 seconds...",start_result);
        //wait 5 seconds
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    }

    Ok(())
}   

async fn node_main(node_host_name: &str,
                   is_ood: bool,
                   buckyos_api_client: &SystemConfigClient,
                   device_doc:&DeviceConfig) -> Result<bool> {

    //check and upgrade some system pkgs not in app_stream or kernel_stream
    let bin_env = PackageEnv::new(get_buckyos_system_bin_dir());
    if !bin_env.is_dev_mode() {
        let root_env_need_upgrade = check_and_update_root_pkg_index_db(buckyos_api_client.get_session_token()).await;
        if root_env_need_upgrade.is_err() {
            warn!("check and update root_pkg_env index db failed! {}", root_env_need_upgrade.err().unwrap());
        }
        let system_pkgs = vec![
            "app_loader".to_string(),
            "node_active".to_string(),
            "buckycli".to_string(),
            "control_panel".to_string(),
            "repo_service".to_string(),
            "cyfs_gateway".to_string(),
            "system_config".to_string(),
            "verify_hub".to_string(),
            "node_daemon".to_string(),
        ];
        let is_self_upgrade = check_and_update_system_pkgs(system_pkgs,buckyos_api_client.get_session_token()).await;
        if is_self_upgrade.is_err() {
            warn!("check and update system pkgs failed! {}", is_self_upgrade.err().unwrap());
        } else {
            if is_self_upgrade.unwrap() {
                warn!("node_daemon self upgrade,will restart!");
                return Ok(false);
            }
        }
    }

    //control pod instance to target state
    let node_config = load_node_config(node_host_name, buckyos_api_client).await
        .map_err(|err| {
            error!("load node config failed! {}", err);
            return NodeDaemonErrors::SystemConfigError("cann't load node config!".to_string());
        })?;
    
    if !node_config.is_running {
        warn!("node_config.is_running is set to false manually, will restart node_daemon...");
        return Ok(false);
    }
    
    let kernel_stream = stream::iter(node_config.kernel);
    let kernel_task = kernel_stream.for_each_concurrent(4, |(kernel_service_name, kernel_cfg)| async move {
        let kernel_run_item = KernelServiceRunItem::new(
            kernel_service_name.as_str(),&kernel_cfg);

        let target_state = RunItemTargetState::from_str(&kernel_cfg.target_state.as_str()).unwrap();
        
        let _ = ensure_run_item_state(&kernel_run_item, target_state)
            .await
            .map_err(|_err| {
                error!("ensure {} to target state failed!",kernel_service_name.clone());
                return NodeDaemonErrors::SystemConfigError(kernel_service_name.clone());
            });
    });

    // TODO: frame_services is "services run in docker container",not support now
    // let service_stream = stream::iter(node_config.frame_services);
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
    

    //app services is "userA-appB-service", run in docker container
    let app_stream = stream::iter(node_config.apps);
    let app_task = app_stream.for_each_concurrent(4, |(app_id_with_name, app_cfg)| async move {
        let app_loader = ServicePkg::new("app_loader".to_string(),get_buckyos_system_bin_dir());
  
        let app_run_item = AppRunItem::new(&app_cfg.app_id,app_cfg.clone(),app_loader);

        let target_state = RunItemTargetState::from_str(&app_cfg.target_state).unwrap();
        let _ = ensure_run_item_state(&app_run_item, target_state)
            .await
            .map_err(|_err| {
                error!("ensure app {} to target state failed!",app_cfg.app_id.clone());
                return NodeDaemonErrors::SystemConfigError(app_cfg.app_id.clone());
            });
    });

    tokio::join!(kernel_task,app_task);
    Ok(true)
}


async fn get_system_config_client(is_ood:bool,session_token:String)->std::result::Result<SystemConfigClient,String> {
    if is_ood {
        let system_config_client = SystemConfigClient::new(None, Some(session_token.as_str()));
        return Ok(system_config_client);
    } else {
        unimplemented!();
    }
}

async fn node_daemon_main_loop(
    node_id:&str,
    node_host_name:&str,
    is_ood: bool
) -> Result<()> {
    let mut loop_step = 0;
    let mut is_running = true;
    let mut last_register_time = 0;
    let mut node_gateway_config = None;

    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        loop_step += 1;
        info!("node daemon main loop step:{}", loop_step);

        let buckyos_runtime = get_buckyos_api_runtime();
        if buckyos_runtime.is_err() {
            error!("buckyos_runtime is none, will restart node_daemon...");
            return Err(NodeDaemonErrors::SystemConfigError("buckyos_runtime is none, will restart node_daemon!".to_string()));
        }
        let buckyos_runtime = buckyos_runtime.unwrap();
        let system_config_client = buckyos_runtime.get_system_config_client().await;
        if system_config_client.is_err() {
            error!("get system_config_client failed! {}", system_config_client.err().unwrap());
            continue;
        }
        let system_config_client = system_config_client.unwrap();
        let device_doc = buckyos_runtime.device_config.as_ref().unwrap();
        let zone_config = buckyos_runtime.zone_config.as_ref().unwrap();
        let device_private_key = buckyos_runtime.device_private_key.as_ref().unwrap();
        let now = buckyos_get_unix_timestamp();
        if now - last_register_time > 30 {
            let mut device_info = DeviceInfo::from_device_doc(device_doc);
            let fill_result = device_info.auto_fill_by_system_info().await;
            if fill_result.is_err() {
                error!("auto fill device info failed! {}", fill_result.err().unwrap());
                break;
            }
            let device_info_str = serde_json::to_string(&device_info).unwrap();
            debug!("update device info: {:?}", device_info);
            std::env::set_var("BUCKYOS_THIS_DEVICE_INFO", device_info_str);
            update_device_info(&device_info, &system_config_client).await;
            //TODO：SN的上报频率不用那么快
            let device_session_token_jwt = buckyos_runtime.get_session_token().await;
            report_ood_info_to_sn(&device_info, device_session_token_jwt.as_str(),zone_config).await;
            last_register_time = now;
        }


        if(is_ood) {
            keep_system_config_service(node_id,device_doc, device_private_key,false).await.map_err(|err| {
                error!("start system_config_service failed! {}", err);
                return NodeDaemonErrors::SystemConfigError("start system_config_service failed!".to_string());
            })?;
        }

        let main_result = node_main(node_host_name,is_ood, &system_config_client, device_doc).await;
        
        if main_result.is_err() {
            error!("node_main failed! {}", main_result.err().unwrap());
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        } else {
            is_running = main_result.unwrap();
            if !is_running {
                break;
            }

            let new_node_gateway_config = load_node_gateway_config(node_host_name, &system_config_client).await;
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
                    info!("node gateway_config changed, will write to node_gateway.json and restart cyfs_gateway service");
                    let gateway_config_path = buckyos_kit::get_buckyos_system_etc_dir().join("node_gateway.json");
                    std::fs::write(gateway_config_path, serde_json::to_string(&node_gateway_config).unwrap()).unwrap();
                }
            
                let sn = buckyos_runtime.zone_config.as_ref().unwrap().sn.clone();
                keep_cyfs_gateway_service(node_id,device_doc, device_private_key, sn,
                need_restart).await.map_err(|err| {
                    error!("keep cyfs_gateway service failed! {}", err);
                });
            
            } else {
                error!("load node gateway_cconfig from system_config failed!");
            }
        }
    }
    Ok(())
}

async fn generate_device_session_token(device_doc: &DeviceConfig, device_private_key: &EncodingKey,is_boot:bool) -> std::result::Result<String,String> {
    let now = SystemTime::now();
    let since_the_epoch = now.duration_since(UNIX_EPOCH)
        .expect("Time went backwards");
    let timestamp = since_the_epoch.as_secs();
    let mut userid = "kernel".to_string();
    if !is_boot {
        userid = device_doc.name.clone();
    }

    let device_session_token = kRPC::RPCSessionToken {
        token_type : kRPC::RPCSessionTokenType::JWT,
        nonce : None,
        session : None,
        userid : Some(userid),
        appid:Some("node-daemon".to_string()),
        exp:Some(timestamp + 60*15),
        iss:Some(device_doc.name.clone()),
        token:None,
    };

    let device_session_token_jwt = device_session_token.generate_jwt(Some(device_doc.name.clone()),&device_private_key)
        .map_err(|err| {
            error!("generate device session token failed! {}", err);
            return String::from("generate device session token failed!");})?;
    
    return Ok(device_session_token_jwt);
}

async fn async_main(matches: ArgMatches) -> std::result::Result<(), String> {
    let node_id = matches.get_one::<String>("id");
    let enable_active = matches.get_flag("enable_active");
    let default_node_id = "node".to_string();
    let node_id = node_id.unwrap_or(&default_node_id);

    info!("node_daemon start...");
    let mut real_machine_config = BuckyOSMachineConfig::default();
    let machine_config = BuckyOSMachineConfig::load_machine_config();
    if machine_config.is_some() {
        real_machine_config = machine_config.unwrap();
    }
    info!("machine_config: {:?}", &real_machine_config);

    init_name_lib(&real_machine_config.web3_bridge).await.map_err(|err| {
        error!("init default name client failed! {}", err);
        return String::from("init default name client failed!");
    })?;
    info!("init default name client OK!");

    //load node identity config
    let node_identity_file = get_buckyos_system_etc_dir().join("node_identity.json");
    let mut node_identity = NodeIdentityConfig::load_node_identity_config(&node_identity_file);
    if node_identity.is_err() {
        if enable_active {
            //befor start node_active_service ,try check and upgrade self
            info!("node_identity.json load error or not found, start node active service...");
            start_node_active_service().await;
            info!("node active service returned,restart node_daemon ...");
            exit(0);
            //restart_program();
        } else {
            error!("load node identity config failed! {}", node_identity.err().unwrap());
            warn!("Would you like to enable activation mode? (Use `--enable_active` to proceed)");
            return Err(String::from("load node identity config failed!"));
        }
    }
    let node_identity = node_identity.unwrap();
    info!("node_identity.json load OK,  zone_did:{}, owner_did:{}.", 
        node_identity.zone_did.to_string(),node_identity.owner_did.to_string());
    //verify device_doc by owner_public_key
    {
        //let owner_name = node_identity.owner_did.to_string();
        let owner_config = resolve_did(&node_identity.owner_did,None).await;
        match owner_config {
            Ok(owner_config) => {
                let owner_config = OwnerConfig::decode(&owner_config,None);
                if owner_config.is_ok() {
                    let owner_config = owner_config.unwrap();
                    let default_key = owner_config.get_default_key();
                    if default_key.is_none() {
                        warn!("owner public key not defined in owner_config! ");
                        return Err("owner public key not defined in owner_config!".to_string());
                    }

                    let default_key = default_key.unwrap();
                    if default_key != node_identity.owner_public_key {
                        warn!("owner_config's default key not match to node_identity's owner_public_key! ");
                        return Err("owner_config's default key not match to node_identity's owner_public_key!".to_string());
                    }
                }
            }
            Err(err) => {
                info!("skip resolve owner_config. {} ", err);
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
    let device_private_key_file = get_buckyos_system_etc_dir().join("node_private_key.pem");
    let device_private_key = load_private_key(&device_private_key_file).map_err(|error| {
        error!("load device private key from node_private_key.pem failed! {}", error);
        return String::from("load device private key failed!");
    })?;

    //lookup zone config
    info!("looking {} zone_boot_config...", node_identity.zone_did.to_host_name());
    let zone_boot_config = looking_zone_boot_config(&node_identity).await.map_err(|err| {
        error!("looking zone config failed! {}", err);
        String::from("looking zone config failed!")
    })?;
    let zone_config_json_str = serde_json::to_string_pretty(&zone_boot_config).unwrap();
    info!("Load zone_boot_config OK, {}", zone_config_json_str);

    //verify node_name is this device's hostname
    let is_ood = zone_boot_config.oods.contains(&device_doc.name);
    let device_name = device_doc.name.clone();
    //CURRENT_ZONE_CONFIG.set(zone_config).unwrap();
    if is_ood {
        info!("Booting OOD {} ...",device_doc.name.as_str());
    } else {
        info!("Booting Node {} ...",device_doc.name.as_str());
    }

    std::env::set_var("BUCKY_ZONE_OWNER", serde_json::to_string(&node_identity.owner_public_key).unwrap());
    std::env::set_var("BUCKYOS_ZONE_BOOT_CONFIG", serde_json::to_string(&zone_boot_config).unwrap());
    std::env::set_var("BUCKYOS_THIS_DEVICE", serde_json::to_string(&device_doc).unwrap());

    info!("set env var BUCKY_ZONE_OWNER,BUCKYOS_ZONE_BOOT_CONFIG,BUCKYOS_THIS_DEVICE OK!");

    keep_cyfs_gateway_service(node_id.as_str(),&device_doc, &device_private_key,   
        zone_boot_config.sn.clone(),false).await.map_err(|err| {
            error!("init cyfs_gateway service failed! {}", err);
            return String::from("init cyfs_gateway service failed!");
    })?;

    let device_session_token_jwt = generate_device_session_token(&device_doc, &device_private_key,true).await.map_err(|err| {
        error!("generate device session token failed! {}", err);
        return String::from("generate device session token failed!");
    })?;
    
    //init kernel_service:system_config service
    let mut syc_cfg_client: SystemConfigClient;
    let boot_config: serde_json::Value;
    let mut boot_config_result_str = "".to_string();
    if is_ood {
        keep_system_config_service(node_id.as_str(),&device_doc, &device_private_key,false).await.map_err(|err| {
            error!("start system_config_service failed! {}", err);
            return String::from("start system_config_service failed!");
        })?;
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        info!("wait system_config_service sync...");
        wait_sysmte_config_sync().await?;
        info!("system_config_service sync OK!,will connect to local system_config_service...");

        //This is ood, so we MUST connect to localhost's system_config_service
        syc_cfg_client = SystemConfigClient::new(None, Some(device_session_token_jwt.as_str()));
        let mut device_info = DeviceInfo::from_device_doc(&device_doc);
        let fill_result = device_info.auto_fill_by_system_info().await;
        if fill_result.is_err() {
            error!("auto fill device info failed! {}", fill_result.err().unwrap());
            return Err(String::from("auto fill device info failed!"));
        }
        update_device_info(&device_info,&syc_cfg_client).await;

        while boot_config_result_str.is_empty() {
            let get_result = syc_cfg_client.get("boot/config").await;
            match get_result {
                buckyos_api::SytemConfigResult::Err(SystemConfigError::KeyNotFound(_)) => {
                    warn!("BuckyOS is started for the first time, enter the BOOT_INIT process...");
                    warn!("Check and upgrade BuckyOS to latest version...");
                    //repo-server is not running, so we need to check and upgrade system by SN.
                    //Here is the only chance to upgrade system to latest version use SN.
                    do_boot_upgreade().await?;

                    warn!("Do boot schedule to generate all system configs...");
                    std::env::set_var("SCHEDULER_SESSION_TOKEN", device_session_token_jwt.clone());
                    debug!("set var SCHEDULER_SESSION_TOKEN {}", device_session_token_jwt);
                    let boot_result = do_boot_schedule().await;
                    if boot_result.is_ok() {
                        warn!("BuckyOS BOOT_INIT OK, will enter system after 2 secs.");
                        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                    } else {
                        warn!("do boot schedule failed! {}, will retry after 5 secs...", boot_result.err().unwrap());
                        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                    }
                },
                buckyos_api::SytemConfigResult::Ok(r) => {
                    boot_config_result_str = r.value.clone();
                    info!("Load boot config OK, {}", boot_config_result_str.as_str());
                },
                _ => {
                    error!("get boot config failed! {},wait 5 sec to retry...", get_result.err().unwrap());
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                }
            }
        }

        let zone_config:ZoneConfig = serde_json::from_str(boot_config_result_str.as_str()).map_err(|err| {
            error!("parse zone config from boot/config failed! {}", err);
            return String::from("parse zone config from boot/config failed!");
        })?;
        std::env::set_var("BUCKYOS_ZONE_CONFIG", boot_config_result_str);
        info!("--------------------------------");

        let mut runtime = BuckyOSRuntime::new("node-daemon", None, BuckyOSRuntimeType::KernelService);
        runtime.fill_policy_by_load_config().await.map_err(|err| {
            error!("fill policy by load config failed! {}", err);
            return String::from("fill policy by load config failed!");
        })?;

        runtime.device_config = Some(device_doc);
        runtime.device_private_key = Some(device_private_key);
        runtime.device_info = Some(device_info);
        runtime.zone_id = node_identity.zone_did.clone();
        runtime.zone_boot_config = Some(zone_boot_config);
        runtime.zone_config = Some(zone_config);
        runtime.session_token = Arc::new(RwLock::new(device_session_token_jwt.clone()));
        runtime.force_https = false;
        set_buckyos_api_runtime(runtime);
    } else {
        //this node is not ood: try connect to system_config_service
        let mut runtime = init_buckyos_api_runtime("node-daemon", None, BuckyOSRuntimeType::KernelService)
                .await
                .map_err(|e| {
                    error!("init_buckyos_api_runtime failed: {:?}", e);
                    return String::from("init_buckyos_api_runtime failed!");
                })?;

        loop {
            //TODO: add searching OOD(system_config_service) logic,search result can generate system_config_url
            // 只有node daemon的这一步需要搜索。搜索完成后会得到一个优先级列表，通过该优先级列表后续的服务都可以直接复用搜索结果
            // 问题： 局域网内的ood重启后，ip发生变化（需要重新搜索）
            
            let login_result = runtime.login().await.map_err(|e| {
                error!("buckyos-api-runtime::login failed: {:?}", e);
                return String::from("buckyos-api-runtime::login failed!");
            });

            if login_result.is_ok() {
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                break;
            } 
        }
        set_buckyos_api_runtime(runtime);
    }
    


    info!("{}@{} boot OK, enter node daemon main loop.", &device_name, node_identity.zone_did.to_host_name());
    node_daemon_main_loop(node_id,&device_name, is_ood)
        .await
        .map_err(|err| {
            error!("node daemon main loop failed! {}", err);
            return String::from("node daemon main loop failed!");
        })?;

    Ok(())
}

pub(crate) fn run(matches: ArgMatches) {
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    if num_cpus::get() < 2 {
        builder.worker_threads(2);
    }
    builder.enable_all().build().unwrap().block_on(async_main(matches));
}