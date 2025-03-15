use std::env;
use std::fmt::format;
use std::process::exit;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use time::macros::format_description;
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



// fn load_identity_config(node_id: &str) -> Result<(NodeIdentityConfig)> {
//     //load ./node_identity.toml for debug
//     //load from /opt/buckyos/etc/node_identity.toml
//     let mut file_path = PathBuf::from(format!("{}_identity.toml",node_id));
//     let path = Path::new(&file_path);
//     if path.exists() {
//         warn!("debug load node identity config from ./node_identity.toml");
//     } else {
//         let etc_dir = get_buckyos_system_etc_dir();

//         file_path = etc_dir.join(format!("{}_identity.toml",node_id));
//     }

//     let contents = std::fs::read_to_string(file_path.clone()).map_err(|err| {
//         error!("read node identity config failed! {}", err);
//         return NodeDaemonErrors::ReadConfigError(file_path.to_string_lossy().to_string());
//     })?;

//     let config: NodeIdentityConfig = toml::from_str(&contents).map_err(|err| {
//         error!("parse node identity config failed! {}", err);
//         return NodeDaemonErrors::ParserConfigError(format!(
//             "Failed to parse NodeIdentityConfig TOML: {}",
//             err
//         ));
//     })?;

//     info!("load node identity config from {} success!",file_path.to_string_lossy());
//     Ok(config)
// }

// fn load_device_private_key(node_id: &str) -> Result<(EncodingKey)> {
//     let mut file_path = format!("{}_private_key.pem",node_id);
//     let path = Path::new(file_path.as_str());
//     if path.exists() {
//         warn!("debug load device private_key from ./device_private_key.pem");
//     } else {
//         let etc_dir = get_buckyos_system_etc_dir();
//         file_path = format!("{}/{}_private_key.pem",etc_dir.to_string_lossy(),node_id);
//     }
//     let private_key = std::fs::read_to_string(file_path.clone()).map_err(|err| {
//         error!("read device private key failed! {}", err);
//         return NodeDaemonErrors::ParserConfigError("read device private key failed!".to_string());
//     })?;

//     let private_key: EncodingKey = EncodingKey::from_ed_pem(private_key.as_bytes()).map_err(|err| {
//         error!("parse device private key failed! {}", err);
//         return NodeDaemonErrors::ParserConfigError("parse device private key failed!".to_string());
//     })?;

//     info!("load device private key from {} success!",file_path);
//     Ok(private_key)
// }

async fn looking_zone_config(node_identity: &NodeIdentityConfig) -> Result<ZoneConfig> {
    //If local files exist, priority loads local files
    let etc_dir = get_buckyos_system_etc_dir();
    let json_config_path = etc_dir.join(format!("{}_zone.toml",node_identity.zone_name)).to_string_lossy().to_string();
    info!("try load zone config from {} for debug", json_config_path);
    let json_config = std::fs::read_to_string(json_config_path.clone());
    if json_config.is_ok() {
        let zone_config = serde_json::from_str(&json_config.unwrap());
        if zone_config.is_ok() {
            warn!("debug load zone config from {} success!", json_config_path);
            return Ok(zone_config.unwrap());
        } else {
            error!("parse debug zone config {} failed! {}", json_config_path, zone_config.err().unwrap());
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
        let owner_from_resolve = zone_jwt.get_owner_pk();
        if owner_from_resolve.is_some() {
            let owner_from_resolve = owner_from_resolve.unwrap();
            //if owner_public_key != owner_from_resolve {
            //    error!("owner public key from resolve is not match!");
            //    return Err(NodeDaemonErrors::ReasonError("owner public key from resolve is not match!".to_string()));
            //}
        }

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

async fn load_app_info(app_id: &str,username: &str,buckyos_api_client: &SystemConfigClient) -> Result<AppDoc> {
    let app_key = format!("users/{}/apps/{}/config", username,app_id);
    let (app_cfg_result,rversion) = buckyos_api_client.get(app_key.as_str()).await
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
    let (node_cfg_result,rversion) = buckyos_api_client.get(node_key.as_str()).await
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
    let (node_cfg_result,rversion) = buckyos_api_client.get(node_key.as_str()).await
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
    for pkg_id in pkg_list {
        let pkg_env = PackageEnv::new(get_buckyos_system_bin_dir());
        let media_info = pkg_env.load(&pkg_id).await;
        if media_info.is_err() {
            info!("check_and_update_system_pkgs: pkg {} not exist, deploy it", pkg_id);
            let result = pkg_env.install_pkg(&pkg_id, false).await;
            if result.is_err() {
                error!("check_and_update_system_pkgs: deploy pkg {} failed! {}", pkg_id, result.err().unwrap());
            }
            info!("check_and_update_system_pkgs: deploy pkg {} success", pkg_id);
        }
    }
    Ok(true)
}

async fn check_and_update_root_pkg_index_db(session_token: Option<String>) -> std::result::Result<bool, String>  {
    let zone_repo_index_db_url = "http://127.0.0.1:8080/repo/meta_index.db";
    let root_env_path = BuckyOSRuntime::get_root_pkg_env_path();
    let meta_db_file_patgh = root_env_path.join(".pkgs").join("meta_index.db");
    let ndn_client = NdnClient::new("http://127.0.0.1:8080/".to_string(), session_token,None);
    let is_same = ndn_client.verify_remote_is_same_as_local_file(zone_repo_index_db_url,&meta_db_file_patgh).await
        .map_err(|err| {
            error!("verify remote index db  to root pkg env's meta-Index db failed! {}", err);
            return String::from("verify remote index db  to root pkg env's meta-Index db failed!");
        })?;

    if is_same {
        info!("remote index db is same as local index db, no need to update!");
        return Ok(false);
    }

    info!("remote index db is not same as local index db, start update node's root_pkg_env.meta_index.db");
    let download_path = root_env_path.join(".pkgs").join("meta_index.downloading");
    ndn_client.download_fileobj_to_local(zone_repo_index_db_url, &download_path, None).await
        .map_err(|err| {
            error!("download remote index db to root pkg env's meta-Index db failed! {}", err);
            return String::from("download remote index db to root pkg env's meta-Index db failed!");
        })?;
    info!("download new meta-index.db success,update root env's meta-index.db..");
    let mut root_env = PackageEnv::new(root_env_path);
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

async fn check_and_update_sys_pkgs(is_ood: bool,buckyos_api_client: &SystemConfigClient) {
    let mut will_check_update_pkg_list = vec![
        "cyfs-gateway".to_string(),
        "app_loader".to_string(),
        "node-active".to_string(),
        "bucky-cli".to_string(),
        "control_panel".to_string(),
        "repo_service".to_string(),
        "scheduler".to_string(),
        "smb_service".to_string(),
        "verify_hub".to_string(),
    ];

    if is_ood {
        will_check_update_pkg_list.push("system_config".to_string());
    }

    let env = PackageEnv::new(get_buckyos_system_etc_dir());
    for pkg_id in will_check_update_pkg_list {
        let mut need_update = true;
        let pkg_meta = env.get_pkg_meta(pkg_id.as_str()).await;
        if pkg_meta.is_ok() {
            let (meta_obj_id,pkg_meta) = pkg_meta.unwrap();
        }

        if need_update {
            //通过repo_service安装pkg
            //call env.install_pkg_from_repo(pkg_id,local_repo_url);
            //env安装新版本完成后，停止进程，等待自动重启。从=
            // create ServicePkg
            // call ServicePkg.stop()
        }
    }    
}

async fn update_device_info(device_doc: &DeviceConfig,syste_config_client: &SystemConfigClient) {
    let mut device_info = DeviceInfo::from_device_doc(device_doc);
    let fill_result = device_info.auto_fill_by_system_info().await;
    if fill_result.is_err() {
        error!("auto fill device info failed! {}", fill_result.err().unwrap());
        return;
    }
    let device_info_str = serde_json::to_string(&device_info).unwrap();

    let device_key = format!("devices/{}/info", device_doc.name.as_str());
    let put_result = syste_config_client.set(device_key.as_str(),device_info_str.as_str()).await;
    if put_result.is_err() {
        error!("update device info to system_config failed! {}", put_result.err().unwrap());
    } else {
        info!("update device info to system_config success!");
    }
}

async fn register_device_doc(device_doc:&DeviceConfig,syste_config_client: &SystemConfigClient) {
    let device_key = format!("devices/{}/doc", device_doc.name.as_str());
    let device_doc_str = serde_json::to_string(&device_doc).unwrap();
    let put_result = syste_config_client.create(device_key.as_str(),device_doc_str.as_str()).await;
    if put_result.is_err() {
        error!("register device doc to system_config failed! {}", put_result.err().unwrap());
    } else {
        info!("register device doc to system_config success!");
    }
}



//if register OK then return sn's URL
async fn report_ood_info_to_sn(device_doc: &DeviceConfig, device_token_jwt: &str,zone_config: &ZoneConfig) -> std::result::Result<String,String> {
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

async fn keep_system_config_service(node_id: &str,device_doc: &DeviceConfig, device_private_key: &EncodingKey,zone_config: &ZoneConfig,is_restart:bool) -> std::result::Result<(),String> {
    let mut system_config_service_pkg = ServicePkg::new("system_config".to_string(),get_buckyos_system_bin_dir());

    if !system_config_service_pkg.try_load().await {
        error!("load system_config_service pkg failed!");
        let env = PackageEnv::new(get_buckyos_system_bin_dir());
        let result = env.install_pkg("system_config", false).await;
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

async fn keep_cyfs_gateway_service(node_id: &str,device_doc: &DeviceConfig, device_private_key: &EncodingKey,zone_config: &ZoneConfig,is_restart:bool) -> std::result::Result<(),String> {
    let mut cyfs_gateway_service_pkg = ServicePkg::new("cyfs_gateway".to_string(),get_buckyos_system_bin_dir());
    if !cyfs_gateway_service_pkg.try_load().await {
        let env = PackageEnv::new(get_buckyos_system_bin_dir());
        let result = env.install_pkg("cyfs_gateway", false).await;
        if result.is_err() {
            error!("install cyfs_gateway service pkg failed! {}", result.err().unwrap());
            return Err(String::from("install cyfs_gateway service pkg failed!"));
        } else {
            info!("install cyfs_gateway service pkg success");
        }
    }

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
        let mut need_keep_tunnel_to_sn = false;
        if zone_config.sn.is_some() {
            need_keep_tunnel_to_sn = true;
            if device_doc.net_id.is_some() {
                let net_id = device_doc.net_id.as_ref().unwrap();
                if net_id == "wan" {
                    need_keep_tunnel_to_sn = false;
                }
            }
        }

        if need_keep_tunnel_to_sn {
            let sn_url = zone_config.sn.as_ref().unwrap();
            params = vec!["--node_id".to_string(),node_id.to_string(),"--keep_tunnel".to_string(),sn_url.clone()];
        } else {
            params = vec!["--node_id".to_string(),node_id.to_string()];
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

fn get_node_daemon_version() -> u32 {
    let build_number = 99999999;
    return build_number;
}

async fn stop_all_system_services() -> std::result::Result<(), String> {
    Ok(())
}

async fn node_main(node_host_name: &str,
                   is_ood: bool,
                   buckyos_api_client: &SystemConfigClient,
                   device_doc:&DeviceConfig,device_private_key: &EncodingKey) -> Result<bool> {
    let need_check_update:bool;
    let root_env_need_upgrade = check_and_update_root_pkg_index_db(buckyos_api_client.get_session_token()).await;
    if root_env_need_upgrade.is_err() {
        error!("check and update root pkg index db failed! {}", root_env_need_upgrade.err().unwrap());
        need_check_update = false 
    } else {
        need_check_update = root_env_need_upgrade.unwrap();
    }
    

    let node_daemon_version = get_node_daemon_version();
    if node_daemon_version < 99999999 {
        //TODO:check and update node_daemon

    }
    //2.check and upgrade some system pkgs not in app_stream or kernel_stream
    let system_pkgs = vec![
        "app_loader".to_string(),
        "node_active".to_string(),
        "bucky-cli".to_string(),
        "control_panel".to_string(),
    ];
    check_and_update_system_pkgs(system_pkgs,buckyos_api_client.get_session_token()).await;
    
    //3. control pod instance to target state
    let node_config = load_node_config(node_host_name, buckyos_api_client).await
        .map_err(|err| {
            error!("load node config failed! {}", err);
            return NodeDaemonErrors::SystemConfigError("cann't load node config!".to_string());
        })?;

    if !node_config.is_running {
        return Ok(false);
    }
    
    let kernel_stream = stream::iter(node_config.kernel);
    let kernel_task = kernel_stream.for_each_concurrent(4, |(kernel_service_name, kernel_cfg)| async move {
        let kernel_run_item = KernelServiceRunItem::new(
            &kernel_cfg,
            &device_doc,
            &device_private_key
        );

        let target_state = RunItemTargetState::from_str(&kernel_cfg.target_state.as_str()).unwrap();
        
        let _ = control_run_item_to_target_state(&kernel_run_item, target_state, device_private_key)
            .await
            .map_err(|_err| {
                error!("control kernel service item {} to target state failed!",kernel_service_name.clone());
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
  
        let app_run_item = AppRunItem::new(&app_cfg.app_id,app_cfg.clone(),
            app_loader,device_doc,device_private_key);

        let target_state = RunItemTargetState::from_str(&app_cfg.target_state).unwrap();
        let _ = control_run_item_to_target_state(&app_run_item, target_state, device_private_key)
            .await
            .map_err(|_err| {
                error!("control app service item {} to target state failed!",app_cfg.app_id.clone());
                return NodeDaemonErrors::SystemConfigError(app_cfg.app_id.clone());
            });
    });

    tokio::join!(kernel_task,app_task);
    Ok(true)
}

async fn node_daemon_main_loop(
    node_id:&str,
    node_host_name:&str,
    buckyos_api_client: &SystemConfigClient,
    device_doc:&DeviceConfig,
    device_session_token_jwt: &str,
    device_private_key: &EncodingKey,
    zone_config: &ZoneConfig,
    is_ood: bool
) -> Result<()> {
    let mut loop_step = 0;
    let mut is_running = true;
    let mut last_register_time = 0;

    //TODO: check and upgrade self use repo_service
    //try regsiter device doc
    report_ood_info_to_sn(&device_doc, device_session_token_jwt,&zone_config).await;

    let mut node_gateway_config = None;
    loop {
        if !is_running {
            break;
        }

        loop_step += 1;
        info!("node daemon main loop step:{}", loop_step);
        report_ood_info_to_sn(&device_doc, device_session_token_jwt,&zone_config).await;
        let now = buckyos_get_unix_timestamp();
        if now - last_register_time > 30 {
            update_device_info(&device_doc, buckyos_api_client).await;
            last_register_time = now;
        }

        if(is_ood) {
            keep_system_config_service(node_id,device_doc, device_private_key,zone_config,false).await.map_err(|err| {
                error!("start system_config_service failed! {}", err);
                return NodeDaemonErrors::SystemConfigError("start system_config_service failed!".to_string());
            })?;
        }

        let main_result = node_main(node_host_name,is_ood, buckyos_api_client, device_doc, device_private_key).await;
        
        if main_result.is_err() {
            error!("node_main failed! {}", main_result.err().unwrap());
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        } else {
            is_running = main_result.unwrap();
            let new_node_gateway_config = load_node_gateway_config(node_host_name, buckyos_api_client).await;
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
                    keep_cyfs_gateway_service(node_id,&device_doc, &device_private_key,&zone_config,true).await.map_err(|err| {
                        error!("start cyfs_gateway service failed! {}", err);
                    });
                } else {
                    keep_cyfs_gateway_service(node_id,&device_doc, &device_private_key,&zone_config,false).await.map_err(|err| {
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


async fn async_main(matches: ArgMatches) -> std::result::Result<(), String> {
    let node_id = matches.get_one::<String>("id");
    let enable_active = matches.get_flag("enable_active");
    let default_node_id = "node".to_string();
    let node_id = node_id.unwrap_or(&default_node_id);

    info!("node_daemon start...");

    init_default_name_client().await.map_err(|err| {
        error!("init default name client failed! {}", err);
        return String::from("init default name client failed!");
    })?;
    info!("init default name client OK!");

    //load node identity config
    let node_identity_file = get_buckyos_system_etc_dir().join("node_identity.toml");
    let mut node_identity = NodeIdentityConfig::load_node_identity_config(&node_identity_file);
    if node_identity.is_err() {
        if enable_active {
            //befor start node_active_service ,try check and upgrade self
            info!("node identity config not found, start node active service...");
            start_node_active_service().await;
            info!("node active service returned,exit node_daemon.");
            exit(0);
            //restart_program();
        } else {
            error!("load node identity config failed! {}", node_identity.err().unwrap());
            warn!("Would you like to enable activation mode? (Use `--enable_active` to proceed)");
            return Err(String::from("load node identity config failed!"));
        }
    }
    let node_identity = node_identity.unwrap();
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
    let device_private_key_file = get_buckyos_system_etc_dir().join("device_private_key.pem");
    let device_private_key = load_private_key(&device_private_key_file).map_err(|error| {
        error!("load device private key failed! {}", error);
        return String::from("load device private key failed!");
    })?;

    //lookup zone config
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
    CURRENT_ZONE_CONFIG.set(zone_config).unwrap();
    if is_ood {
        info!("Booting OOD {} ...",node_host_name);
    } else {
        info!("Booting Node {} ...",node_host_name);
    }

    let zone_config = CURRENT_ZONE_CONFIG.get().unwrap();
    std::env::set_var("BUCKY_ZONE_OWNER", serde_json::to_string(&node_identity.owner_public_key).unwrap());
    std::env::set_var("BUCKYOS_ZONE_CONFIG", serde_json::to_string(&zone_config).unwrap());
    std::env::set_var("BUCKYOS_THIS_DEVICE", serde_json::to_string(&device_doc).unwrap());

    info!("set var BUCKY_ZONE_OWNER to {}", env::var("BUCKY_ZONE_OWNER").unwrap());
    info!("set var BUCKYOS_ZONE_CONFIG to {}", env::var("BUCKYOS_ZONE_CONFIG").unwrap());
    info!("set var BUCKYOS_THIS_DEVICE to {}", env::var("BUCKYOS_THIS_DEVICE").unwrap());

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
    //enable_zone_provider(Some(&device_info),Some(&device_session_token_jwt),false);
    //init kernel_service:cyfs-gateway service
    std::env::set_var("GATEWAY_SESSIONT_TOKEN",device_session_token_jwt.clone());
    info!("set var GATEWAY_SESSIONT_TOKEN to {}", device_session_token_jwt);
    keep_cyfs_gateway_service(node_id.as_str(),&device_doc, &device_private_key,&zone_config,false).await.map_err(|err| {
        error!("init cyfs_gateway service failed! {}", err);
        return String::from("init cyfs_gateway service failed!");
    })?;

    //init kernel_service:system_config service
    let mut syc_cfg_client: SystemConfigClient;
    let boot_config: serde_json::Value;
    if is_ood {
        keep_system_config_service(node_id.as_str(),&device_doc, &device_private_key,&zone_config,false).await.map_err(|err| {
            error!("start system_config_service failed! {}", err);
            return String::from("start system_config_service failed!");
        })?;
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        syc_cfg_client = SystemConfigClient::new(None, Some(device_session_token_jwt.as_str()));
        update_device_info(&device_doc, &syc_cfg_client).await;
        //register_device_doc(&device_doc, &syc_cfg_client).await;
        let boot_config_result = syc_cfg_client.get("boot/config").await;
        match boot_config_result {
            buckyos_api::SytemConfigResult::Err(SystemConfigError::KeyNotFound(_)) => {
                warn!("BuckyOS is started for the first time, enter the BOOT_INIT process...");
                warn!("Check and upgrade BuckyOS to latest version...");
                //repo-server is not running, so we need to check and upgrade system by SN.
                //Here is the only chance to upgrade system to latest version use SN.
                do_boot_upgreade().await?;

                warn!("Do boot schedule to generate all system configs...");
                std::env::set_var("SCHEDULER_SESSION_TOKEN", device_session_token_jwt.clone());
                debug!("set var SCHEDULER_SESSION_TOKEN {}", device_session_token_jwt);
                do_boot_schedule().await.map_err(|err| {
                    error!("do boot scheduler failed! {}", err);
                    return String::from("do boot scheduler failed!");
                })?;
                warn!("Restart all first-time-run services ...");
                warn!("BuckyOS BOOT_INIT OK, enter system after 2 secs.");
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            },
            buckyos_api::SytemConfigResult::Ok(r) => {
                boot_config = serde_json::from_str(r.0.as_str()).map_err(|err| {
                    error!("parse boot config failed! {}", err);
                    return String::from("parse boot config failed!");
                })?;
                info!("Load boot config OK, {}", boot_config);
            },
            _ => {
                error!("get boot config failed! {}", boot_config_result.err().unwrap());
                return Err("get boot config failed!".to_string());
            }
        }

    } else {
        //this node is not ood: try connect to system_config_service
        let this_device = DeviceInfo::from_device_doc(&device_doc);
        let runtime = get_buckyos_api_runtime().unwrap();
        syc_cfg_client = runtime.get_system_config_client().await.unwrap();
        loop {
            //syc_cfg_client = SystemConfigClient::new(Some(system_config_url.as_str()), Some(device_session_token_jwt.as_str()));
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
    register_device_doc(&device_doc, &syc_cfg_client).await;
    //use boot config to init name-lib.. etc kernel libs.
    //let gateway_ip = resolve_ip("gateway").await;
    //info!("gateway ip: {:?}", gateway_ip);

    info!("{}@{} boot OK, enter node daemon main loop!", device_doc.name, node_identity.zone_name);
    node_daemon_main_loop(node_id,&device_doc.name.as_str(), &syc_cfg_client, 
        &device_doc, &device_session_token_jwt,&device_private_key, &zone_config, is_ood)
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