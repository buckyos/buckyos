use lazy_static::lazy_static;
use std::env;
use std::fmt::format;
use std::hash::Hash;
use std::path::{Path, PathBuf};
use std::process::exit;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use std::{collections::HashMap, fs::File};
use time::macros::format_description;
use tokio::sync::RwLock;

use clap::{Arg, ArgMatches, Command};
use futures::prelude::*;
use log::*;
use named_store::NamedStoreMgr;
use serde::{Deserialize, Serialize};
use serde_json::{from_value, json, Value};
use simplelog::*;
use toml;

use buckyos_api::*;
use buckyos_kit::*;
use jsonwebtoken::jwk::Jwk;
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use name_client::*;
use name_lib::*;
use ndn_lib::*;
use package_lib::*;

use crate::active_server::*;
use crate::app_mgr::*;
use crate::frame_service_mgr::*;
use crate::kernel_mgr::*;
use crate::local_app_mgr::LocalAppRunItem;
use crate::run_item::*;
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
    let json_config_path = etc_dir.join(format!(
        "{}.zone.json",
        node_identity.zone_did.to_raw_host_name()
    ));
    info!(
        "check  {} is exist for debug ...",
        json_config_path.display()
    );
    let mut zone_boot_config: ZoneBootConfig;
    //在离线环境中，可以利用下面机制来绕开DNS查询
    if json_config_path.exists() {
        info!(
            "try load zone boot config from {} for debug",
            json_config_path.display()
        );
        let json_config = std::fs::read_to_string(json_config_path.clone());
        if json_config.is_ok() {
            let zone_boot_config_result = serde_json::from_str(&json_config.unwrap());
            if zone_boot_config_result.is_ok() {
                warn!(
                    "debug load zone boot config from {} success!",
                    json_config_path.display()
                );
                zone_boot_config = zone_boot_config_result.unwrap();
            } else {
                error!(
                    "parse debug zone boot config {} failed! {}",
                    json_config_path.display(),
                    zone_boot_config_result.err().unwrap()
                );
                return Err(NodeDaemonErrors::ReasonError(
                    "parse debug zone boot config from local file failed!".to_string(),
                ));
            }
        } else {
            return Err(NodeDaemonErrors::ReasonError(
                "parse debug zone boot config from local file failed!".to_string(),
            ));
        }
    } else {
        let mut zone_did = node_identity.zone_did.clone();
        info!(
            "node_identity.owner_public_key: {:?}",
            node_identity.owner_public_key
        );
        let owner_public_key =
            DecodingKey::from_jwk(&node_identity.owner_public_key).map_err(|err| {
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

        let zone_doc = resolve_did(&node_identity.zone_did, Some("boot"))
            .await
            .map_err(|err| {
                error!("resolve zone did failed! {}", err);
                return NodeDaemonErrors::ReasonError("resolve zone did failed!".to_string());
            })?;

        zone_boot_config =
            ZoneBootConfig::decode(&zone_doc, Some(&owner_public_key)).map_err(|err| {
                error!("parse zone config failed! {}", err);
                return NodeDaemonErrors::ReasonError("parse zone config failed!".to_string());
            })?;
    }

    zone_boot_config.id = Some(node_identity.zone_did.clone());
    if zone_boot_config.owner.is_some() {
        if zone_boot_config.owner.as_ref().unwrap() != &node_identity.owner_did {
            error!("zone boot config's owner is not match node_identity's owner_did!");
            return Err(NodeDaemonErrors::ReasonError(
                "zone owner is not match!".to_string(),
            ));
        }
    } else {
        zone_boot_config.owner = Some(node_identity.owner_did.clone());
    }
    zone_boot_config.owner_key = Some(node_identity.owner_public_key.clone());

    return Ok(zone_boot_config);
}

async fn load_app_info(
    app_id: &str,
    username: &str,
    buckyos_api_client: &SystemConfigClient,
) -> Result<AppDoc> {
    let app_key = format!("users/{}/apps/{}/config", username, app_id);
    let get_result = buckyos_api_client
        .get(app_key.as_str())
        .await
        .map_err(|error| {
            let err_str = format!("get app config failed from system_config! {}", error);
            warn!("{}", err_str.as_str());
            return NodeDaemonErrors::SystemConfigError(err_str);
        })?;

    let app_info = serde_json::from_str(&get_result.value);
    if app_info.is_ok() {
        return Ok(app_info.unwrap());
    }
    let err_str = format!("parse app info failed! {}", app_info.err().unwrap());
    warn!("{}", err_str.as_str());
    return Err(NodeDaemonErrors::SystemConfigError(err_str));
}

async fn load_node_gateway_config(
    node_host_name: &str,
    buckyos_api_client: &SystemConfigClient,
) -> Result<Value> {
    let json_config_path = format!("{}_node_gateway.json", node_host_name);
    let json_config = std::fs::read_to_string(json_config_path);
    if json_config.is_ok() {
        let json_config = json_config.unwrap();
        let gateway_config = serde_json::from_str(json_config.as_str()).map_err(|err| {
            error!("parse DEBUG node gateway_config  failed! {}", err);
            return NodeDaemonErrors::SystemConfigError(
                "parse DEBUG node gateway_config failed!".to_string(),
            );
        })?;

        warn!(
            "Debug load node gateway_config from ./{}_node_gateway.json success!",
            node_host_name
        );
        return Ok(gateway_config);
    }

    let node_key = format!("nodes/{}/gateway_config", node_host_name);
    let get_result = buckyos_api_client
        .get(node_key.as_str())
        .await
        .map_err(|error| {
            error!(
                "get node gateway_config failed from system_config_service! {}",
                error
            );
            return NodeDaemonErrors::SystemConfigError(
                "get node gateway_config failed from system_config_service!".to_string(),
            );
        })?;

    let gateway_config = serde_json::from_str(&get_result.value).map_err(|err| {
        error!("parse node gateway_config failed! {}", err);
        return NodeDaemonErrors::SystemConfigError("parse gateway_config failed!".to_string());
    })?;

    Ok(gateway_config)
}

async fn load_node_gateway_info(
    node_host_name: &str,
    buckyos_api_client: &SystemConfigClient,
) -> Result<Value> {
    let json_config_path = format!("{}_node_gateway_info.json", node_host_name);
    let json_config = std::fs::read_to_string(json_config_path);
    if json_config.is_ok() {
        let json_config = json_config.unwrap();
        let gateway_info = serde_json::from_str(json_config.as_str()).map_err(|err| {
            error!("parse DEBUG node gateway_info failed! {}", err);
            NodeDaemonErrors::SystemConfigError("parse DEBUG node gateway_info failed!".to_string())
        })?;

        warn!(
            "Debug load node gateway_info from ./{}_node_gateway_info.json success!",
            node_host_name
        );
        return Ok(gateway_info);
    }

    let node_key = format!("nodes/{}/gateway_info", node_host_name);
    let get_result = buckyos_api_client
        .get(node_key.as_str())
        .await
        .map_err(|error| {
            error!(
                "get node gateway_info failed from system_config_service! {}",
                error
            );
            NodeDaemonErrors::SystemConfigError(
                "get node gateway_info failed from system_config_service!".to_string(),
            )
        })?;

    let gateway_info = serde_json::from_str(&get_result.value).map_err(|err| {
        error!("parse node gateway_info failed! {}", err);
        NodeDaemonErrors::SystemConfigError("parse gateway_info failed!".to_string())
    })?;

    Ok(gateway_info)
}

fn resolve_static_dir_pkg_id(app_info: &serde_json::Map<String, Value>) -> Result<Option<String>> {
    let Some(dir_pkg_id) = app_info.get("dir_pkg_id").and_then(Value::as_str) else {
        return Ok(None);
    };

    let resolved_pkg_id = match app_info.get("dir_pkg_objid").and_then(Value::as_str) {
        Some(dir_pkg_objid) => {
            let obj_id = ObjId::new(dir_pkg_objid).map_err(|err| {
                NodeDaemonErrors::ReasonError(format!(
                    "invalid dir_pkg_objid {} for dir pkg {}: {}",
                    dir_pkg_objid, dir_pkg_id, err
                ))
            })?;
            PackageId::get_pkgid_with_objid(dir_pkg_id, Some(obj_id)).map_err(|err| {
                NodeDaemonErrors::ReasonError(format!(
                    "build exact pkg id for {} with objid {} failed: {}",
                    dir_pkg_id, dir_pkg_objid, err
                ))
            })?
        }
        None => {
            warn!(
                "static web app dir package {} missing dir_pkg_objid, fallback to pkg id only",
                dir_pkg_id
            );
            dir_pkg_id.to_string()
        }
    };

    Ok(Some(resolved_pkg_id))
}

async fn index_static_dir_pkg_meta_from_named_store(
    pkg_env_path: &Path,
    resolved_pkg_id: &str,
    store_mgr: &NamedStoreMgr,
) -> Result<bool> {
    let package_id = PackageId::parse(resolved_pkg_id).map_err(|err| {
        NodeDaemonErrors::ReasonError(format!(
            "parse static dir pkg id {} failed: {}",
            resolved_pkg_id, err
        ))
    })?;
    let Some(meta_obj_id_str) = package_id.objid.as_deref() else {
        return Ok(false);
    };

    let meta_obj_id = ObjId::new(meta_obj_id_str).map_err(|err| {
        NodeDaemonErrors::ReasonError(format!(
            "parse static dir pkg objid {} failed: {}",
            meta_obj_id_str, err
        ))
    })?;
    let pkg_meta_str = store_mgr.get_object(&meta_obj_id).await.map_err(|err| {
        NodeDaemonErrors::ReasonError(format!(
            "load static dir pkg meta {} from named store failed: {}",
            meta_obj_id, err
        ))
    })?;
    let pkg_meta = PackageMeta::from_str(pkg_meta_str.as_str()).or_else(|_| {
        let pkg_meta_json = serde_json::from_str::<Value>(pkg_meta_str.as_str())
            .or_else(|_| load_named_object_from_obj_str(pkg_meta_str.as_str()))
            .map_err(|err| {
                NodeDaemonErrors::ReasonError(format!(
                    "parse static dir pkg meta {} from named store failed: {}",
                    meta_obj_id, err
                ))
            })?;
        serde_json::from_value::<PackageMeta>(pkg_meta_json).map_err(|err| {
            NodeDaemonErrors::ReasonError(format!(
                "decode static dir pkg meta {} failed: {}",
                meta_obj_id, err
            ))
        })
    })?;

    let expected_pkg_name = if package_id.name.contains('.') {
        package_id.name.clone()
    } else {
        format!(
            "{}.{}",
            new_package_env(pkg_env_path.to_path_buf()).get_prefix(),
            package_id.name
        )
    };
    let meta_obj_id_string = meta_obj_id.to_string();
    let mut indexed_pkg_meta = pkg_meta.clone();
    if indexed_pkg_meta.name != expected_pkg_name {
        indexed_pkg_meta.name = expected_pkg_name;
    }
    let indexed_pkg_meta_str = serde_json::to_string(&indexed_pkg_meta).map_err(|err| {
        NodeDaemonErrors::ReasonError(format!(
            "serialize indexed static dir pkg meta {} failed: {}",
            meta_obj_id, err
        ))
    })?;

    std::fs::create_dir_all(pkg_env_path.join("pkgs")).map_err(|err| {
        NodeDaemonErrors::ReasonError(format!(
            "create pkg env meta db dir {} failed: {}",
            pkg_env_path.join("pkgs").display(),
            err
        ))
    })?;

    let meta_db =
        MetaIndexDb::new(pkg_env_path.join("pkgs/meta_index.db"), false).map_err(|err| {
            NodeDaemonErrors::ReasonError(format!(
                "open pkg env meta db for {} failed: {}",
                resolved_pkg_id, err
            ))
        })?;
    meta_db
        .add_pkg_meta(
            meta_obj_id_string.as_str(),
            indexed_pkg_meta_str.as_str(),
            indexed_pkg_meta.author.as_str(),
            None,
        )
        .map_err(|err| {
            NodeDaemonErrors::ReasonError(format!(
                "insert static dir pkg meta {} into env db failed: {}",
                meta_obj_id, err
            ))
        })?;
    meta_db
        .set_pkg_version(
            indexed_pkg_meta.name.as_str(),
            indexed_pkg_meta.author.as_str(),
            indexed_pkg_meta.version.as_str(),
            meta_obj_id_string.as_str(),
            indexed_pkg_meta.version_tag.as_deref(),
        )
        .map_err(|err| {
            NodeDaemonErrors::ReasonError(format!(
                "set static dir pkg version for {} in env db failed: {}",
                resolved_pkg_id, err
            ))
        })?;

    Ok(true)
}

async fn index_static_dir_pkg_meta_from_current_named_store(
    pkg_env_path: &Path,
    resolved_pkg_id: &str,
) -> Result<bool> {
    let package_id = PackageId::parse(resolved_pkg_id).map_err(|err| {
        NodeDaemonErrors::ReasonError(format!(
            "parse static dir pkg id {} failed: {}",
            resolved_pkg_id, err
        ))
    })?;
    if package_id.objid.is_none() {
        return Ok(false);
    }

    let runtime = get_buckyos_api_runtime().map_err(|err| {
        NodeDaemonErrors::ReasonError(format!(
            "buckyos runtime is not initialized when indexing {}: {}",
            resolved_pkg_id, err
        ))
    })?;
    let store_mgr = runtime.get_named_store().await.map_err(|err| {
        NodeDaemonErrors::ReasonError(format!(
            "get named store for static dir pkg {} failed: {}",
            resolved_pkg_id, err
        ))
    })?;

    index_static_dir_pkg_meta_from_named_store(pkg_env_path, resolved_pkg_id, &store_mgr).await
}

async fn ensure_node_gateway_dir_pkgs_installed_in_env(
    pkg_env_path: &Path,
    node_gateway_info: &Value,
) -> Result<()> {
    let Some(app_info_map) = node_gateway_info.get("app_info").and_then(Value::as_object) else {
        return Ok(());
    };

    let mut pkg_env = new_package_env(pkg_env_path.to_path_buf());
    let mut checked_pkg_ids = std::collections::HashSet::new();
    let mut failures = Vec::new();

    for (host, app_entry) in app_info_map {
        let Some(app_info) = app_entry.as_object() else {
            continue;
        };
        let app_id = app_info
            .get("app_id")
            .and_then(Value::as_str)
            .unwrap_or(host.as_str());
        let resolved_pkg_id = match resolve_static_dir_pkg_id(app_info) {
            Ok(Some(pkg_id)) => pkg_id,
            Ok(None) => continue,
            Err(err) => {
                failures.push(format!(
                    "resolve static web dir pkg for app {} failed: {}",
                    app_id, err
                ));
                continue;
            }
        };

        if !checked_pkg_ids.insert(resolved_pkg_id.clone()) {
            continue;
        };

        match pkg_env.load(resolved_pkg_id.as_str()).await {
            Ok(media_info) => {
                info!(
                    "static web dir pkg {} for app {} already installed at {}",
                    resolved_pkg_id,
                    app_id,
                    media_info.full_path.display()
                );
                continue;
            }
            Err(err) => {
                info!(
                    "static web dir pkg {} for app {} is missing or stale, will install: {}",
                    resolved_pkg_id, app_id, err
                );
            }
        }

        match index_static_dir_pkg_meta_from_current_named_store(
            pkg_env_path,
            resolved_pkg_id.as_str(),
        )
        .await
        {
            Ok(true) => {
                info!(
                    "indexed static web dir pkg meta {} for app {} from named store",
                    resolved_pkg_id, app_id
                );
            }
            Ok(false) => {}
            Err(err) => {
                warn!(
                    "index static web dir pkg meta {} for app {} from named store failed, will continue install: {}",
                    resolved_pkg_id, app_id, err
                );
            }
        }

        match pkg_env
            .install_pkg(resolved_pkg_id.as_str(), true, false)
            .await
        {
            Ok(meta_obj_id) => {
                info!(
                    "installed static web dir pkg {} for app {} with meta obj id {}",
                    resolved_pkg_id, app_id, meta_obj_id
                );
            }
            Err(PkgError::PackageAlreadyInstalled(_)) => {
                info!(
                    "static web dir pkg {} for app {} is already installed",
                    resolved_pkg_id, app_id
                );
            }
            Err(err) => {
                failures.push(format!(
                    "install static web dir pkg {} for app {} failed: {}",
                    resolved_pkg_id, app_id, err
                ));
                continue;
            }
        }

        if let Err(err) = pkg_env.load(resolved_pkg_id.as_str()).await {
            failures.push(format!(
                "load static web dir pkg {} for app {} after install failed: {}",
                resolved_pkg_id, app_id, err
            ));
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(NodeDaemonErrors::ReasonError(failures.join("; ")))
    }
}

async fn ensure_node_gateway_dir_pkgs_installed(node_gateway_info: &Value) -> Result<()> {
    ensure_node_gateway_dir_pkgs_installed_in_env(
        get_buckyos_system_bin_dir().as_path(),
        node_gateway_info,
    )
    .await
}

async fn load_node_config(
    node_host_name: &str,
    buckyos_api_client: &SystemConfigClient,
) -> Result<NodeConfig> {
    let json_config_path = format!("{}_node_config.json", node_host_name);
    let json_config = std::fs::read_to_string(json_config_path);
    if json_config.is_ok() {
        let json_config = json_config.unwrap();
        let node_config = serde_json::from_str(json_config.as_str()).map_err(|err| {
            error!("parse DEBUG node config failed! {}", err);
            return NodeDaemonErrors::SystemConfigError(
                "parse DEBUG node config failed!".to_string(),
            );
        })?;

        warn!(
            "Debug load node config from ./{}_node_config.json success!",
            node_host_name
        );
        return Ok(node_config);
    }

    let node_key = format!("nodes/{}/config", node_host_name);
    let get_result = buckyos_api_client
        .get(node_key.as_str())
        .await
        .map_err(|error| {
            error!("get node config failed from etcd! {}", error);
            return NodeDaemonErrors::SystemConfigError(
                "get node config failed from system_config_service!".to_string(),
            );
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
async fn do_boot_upgreade() -> std::result::Result<(), String> {
    //TODO:
    Ok(())
}

async fn update_device_info(device_info: &DeviceInfo, syste_config_client: &SystemConfigClient) {
    let device_key = format!("devices/{}/info", device_info.name.as_str());
    let device_info_str = serde_json::to_string(&device_info).unwrap();
    let put_result = syste_config_client
        .set(device_key.as_str(), device_info_str.as_str())
        .await;
    if put_result.is_err() {
        error!(
            "update device info to system_config failed! {}",
            put_result.err().unwrap()
        );
    } else {
        info!(
            "update {} info to system_config success!",
            device_info.name.as_str()
        );
    }
}

//if register OK then return sn's real URL for this user
async fn report_ood_info_to_sn(
    device_info: &DeviceInfo,
    device_token_jwt: &str,
    zone_config: &ZoneConfig,
) -> std::result::Result<(), String> {
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

    if zone_config
        .oods
        .iter()
        .find(|ood| ood.name == device_info.name)
        .is_none()
    {
        error!("this device is not in zone's ood list!");
        return Err(String::from("this device is not in zone's ood list!"));
    }

    let owner_did = zone_config.owner.clone();
    if !owner_did.is_valid() {
        error!("zone config owner_did is not set!");
        return Err(String::from("zone config owner_did is not set!"));
    }

    sn_update_device_info(
        sn_url.as_str(),
        Some(device_token_jwt.to_string()),
        &owner_did.id,
        device_info.name.as_str(),
        &device_info,
    )
    .await;

    info!(
        "update {}'s info to sn {} success!",
        device_info.name.as_str(),
        sn_url.as_str()
    );
    Ok(())
}

async fn do_boot_schedule() -> std::result::Result<(), String> {
    let mut scheduler_pkg = ServicePkg::new("scheduler".to_string(), get_buckyos_system_bin_dir());
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

async fn wait_sysmte_config_sync() -> std::result::Result<(), String> {
    return Ok(());
}

async fn keep_system_config_service(
    node_id: &str,
    device_doc: &DeviceConfig,
    device_private_key: &EncodingKey,
    is_restart: bool,
) -> std::result::Result<(), String> {
    let mut system_config_service_pkg =
        ServicePkg::new("system-config".to_string(), get_buckyos_system_bin_dir());

    if !system_config_service_pkg.try_load().await {
        error!("load system_config_service pkg failed!");
        let mut env = new_system_package_env();
        let result = env.install_pkg("system-config", false, false).await;
        if result.is_err() {
            error!(
                "install system_config_service pkg failed! {}",
                result.err().unwrap()
            );
            return Err(String::from("install system_config_service pkg failed!"));
        } else {
            info!("install system_config_service pkg success");
        }
    }

    let mut running_state = system_config_service_pkg
        .status(None)
        .await
        .map_err(|err| {
            error!("check system_config_service running failed! {}", err);
            return String::from("check system_config_service running failed!");
        })?;

    if is_restart {
        system_config_service_pkg.stop(None).await.map_err(|err| {
            error!("stop system_config_service failed! {}", err);
            return String::from("stop system_config_service failed!");
        })?;
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        running_state = ServiceInstanceState::Stopped;
    }

    if running_state == ServiceInstanceState::Stopped {
        warn!("check system_config_service is stopped,try to start system_config_service");
        let start_result = system_config_service_pkg.start(None).await.map_err(|err| {
            error!("start system_config_service failed! {}", err);
            return String::from("start system_config_service failed!");
        })?;
        info!("start system_config_service OK!,result:{}", start_result);
    }
    Ok(())
}

async fn keep_cyfs_gateway_service(
    node_id: &str,
    device_doc: &DeviceConfig,
    node_private_key: &EncodingKey,
    sn: Option<String>,
    is_reload: bool,
) -> std::result::Result<(), String> {
    //TODO: 需要区分boot模式和正常模式
    let mut cyfs_gateway_service_pkg =
        ServicePkg::new("cyfs-gateway".to_string(), get_buckyos_system_bin_dir());

    let mut running_state = cyfs_gateway_service_pkg.status(None).await.map_err(|err| {
        error!("check cyfs_gateway running failed! {}", err);
        return String::from("check cyfs_gateway running failed!");
    })?;
    debug!("cyfs_gateway service pkg status: {:?}", running_state);

    if running_state == ServiceInstanceState::NotExist {
        //当版本更新后,上述检查会返回NotExist, 并触发更新操作
        warn!("cyfs_gateway service pkg not exist, try install it...");
        let mut env = new_system_package_env();
        let result = env.install_pkg("cyfs-gateway", false, false).await;
        if result.is_err() {
            error!(
                "install cyfs_gateway service pkg failed! {}",
                result.err().unwrap()
            );
            return Err(String::from("install cyfs_gateway service pkg failed!"));
        } else {
            info!("install cyfs_gateway service pkg success");
        }
    }

    if is_reload && running_state == ServiceInstanceState::Started {
        info!("cyfs_gateway service pkg loaded, will do reload ...");
        let params = vec!["--reload".to_string()];
        cyfs_gateway_service_pkg
            .start(Some(&params))
            .await
            .map_err(|err| {
                error!("start cyfs_gateway service failed! {}", err);
                return String::from("start cyfs_gateway service failed!");
            })?;
        info!("call cyfs_gateway reload success!");
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        running_state = ServiceInstanceState::Started;
    }

    if running_state == ServiceInstanceState::Stopped {
        warn!("check cyfs_gateway is stopped,try to start cyfs_gateway");
        //params: boot cyfs-gateway configs, identiy_etc folder, keep_tunnel list
        //  ood: keep tunnel to other ood, keep tunnel to gateway
        //  gateway_config: port_forward for system_config service
        let params: Vec<String>;
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
            let sn_host_name = get_real_sn_host_name(sn.as_ref().unwrap(), device_did.as_str())
                .await
                .map_err(|err| {
                    error!("get sn host name failed! {}", err);
                    return String::from("get sn host name failed!");
                })?;
            params = vec!["--keep_tunnel".to_string(), sn_host_name.clone()];
        } else {
            params = Vec::new();
        }

        let start_result = cyfs_gateway_service_pkg
            .start(Some(&params))
            .await
            .map_err(|err| {
                error!("start cyfs_gateway failed! {}", err);
                return String::from("start cyfs_gateway failed!");
            })?;

        info!(
            "start cyfs_gateway OK!,result:{}. wait 2 seconds...",
            start_result
        );
        //wait 5 seconds
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    }

    Ok(())
}

async fn desktop_daemon_main(skip_app_ids: &Vec<String>) -> Result<()> {
    info!("desktop_daemon_main skip_app_ids: {:?}", skip_app_ids);
    let app_list_path = get_buckyos_system_bin_dir().join("applist.json");
    let app_list_str = std::fs::read_to_string(app_list_path.clone());
    if app_list_str.is_err() {
        error!("read app list failed! {}", app_list_str.err().unwrap());
        return Err(NodeDaemonErrors::ReadConfigError(String::from(
            "read local app list failed!",
        )));
    }
    let app_list_str = app_list_str.unwrap();
    let app_list = serde_json::from_str(app_list_str.as_str());
    if app_list.is_err() {
        error!("parse app list failed! {}", app_list.err().unwrap());
        return Err(NodeDaemonErrors::ParserConfigError(String::from(
            "parse local app list failed!",
        )));
    }
    let app_list: HashMap<String, LocalAppInstanceConfig> = app_list.unwrap();

    let app_stream = stream::iter(app_list);
    let app_task = app_stream.for_each_concurrent(2, |(app_id_with_name, app_cfg)| async move {
        if skip_app_ids.contains(&app_id_with_name) {
            info!(
                "skip app {} because it is in skip_app_ids",
                app_id_with_name.clone()
            );
            return;
        }
        let local_app_run_item = LocalAppRunItem::new(&app_id_with_name, app_cfg.clone());

        let target_state = RunItemTargetState::from_instance_state(&app_cfg.target_state);
        let _ = ensure_run_item_state(&local_app_run_item, target_state)
            .await
            .map_err(|_err| {
                error!(
                    "ensure app {} to target state failed!",
                    app_id_with_name.clone()
                );
            });
    });
    return Ok(());
}

async fn desktop_daemon() -> Result<()> {
    //为了防止desktop_daemon和node_daemon用不同的参数启动同一个app,使用node_daemon优先原则:
    //即使优先执行node_daemon的检查，让node_daemon有机会先启动app
    //read local app config frorm bin/app_list.json
    let app_list_path = get_buckyos_system_bin_dir().join("applist.json");
    let mut loop_step = 0;
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        loop_step += 1;
        info!(
            "desktop_daemon step:{},app_list_path:{}",
            loop_step,
            app_list_path.display()
        );
        let _ = desktop_daemon_main(&Vec::new()).await.map_err(|err| {
            warn!("desktop_daemon failed! {}", err);
            return NodeDaemonErrors::SystemConfigError("desktop_daemon failed!".to_string());
        });
    }

    Ok(())
}

async fn node_main(
    node_host_name: &str,
    is_ood: bool,
    is_desktop: bool,
    buckyos_api_client: &SystemConfigClient,
    device_doc: &DeviceConfig,
) -> Result<bool> {
    //control replica instance to target state
    let node_config = load_node_config(node_host_name, buckyos_api_client)
        .await
        .map_err(|err| {
            error!("load node config failed! {}", err);
            return NodeDaemonErrors::SystemConfigError("cann't load node config!".to_string());
        })?;

    if !node_config.is_running() {
        warn!("node_config.is_running is set to false manually, will restart node_daemon...");
        return Ok(false);
    }

    let kernel_stream = stream::iter(node_config.kernel);
    let kernel_task =
        kernel_stream.for_each_concurrent(4, |(kernel_service_name, kernel_cfg)| async move {
            let kernel_run_item =
                KernelServiceRunItem::new(kernel_service_name.as_str(), &kernel_cfg);

            let target_state = RunItemTargetState::from_instance_state(&kernel_cfg.target_state);

            let _ = ensure_run_item_state(&kernel_run_item, target_state)
                .await
                .map_err(|_err| {
                    error!(
                        "ensure {} to target state failed!",
                        kernel_service_name.clone()
                    );
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
    let app_ids = node_config.apps.keys().cloned().collect::<Vec<String>>();

    let app_stream = stream::iter(node_config.apps);
    let app_task = app_stream.for_each_concurrent(4, |(app_id_with_name, app_cfg)| async move {
        let app_run_item = AppRunItem::new(&app_id_with_name, app_cfg.clone());

        let target_state = RunItemTargetState::from_instance_state(&app_cfg.target_state);
        let _ = ensure_run_item_state(&app_run_item, target_state)
            .await
            .map_err(|_err| {
                error!(
                    "ensure app {} to target state failed!",
                    app_id_with_name.clone()
                );
                return NodeDaemonErrors::SystemConfigError(app_id_with_name.clone());
            });
    });

    if is_desktop {
        let mut skip_app_ids = app_ids;
        skip_app_ids.push("cyfs-gateway".to_string());
        let _ = desktop_daemon_main(&skip_app_ids).await.map_err(|err| {
            warn!("desktop_daemon failed! {}", err);
        });
    }

    tokio::join!(kernel_task, app_task);
    Ok(true)
}

async fn get_system_config_client(
    is_ood: bool,
    session_token: String,
) -> std::result::Result<SystemConfigClient, String> {
    if is_ood {
        let system_config_client = SystemConfigClient::new(None, Some(session_token.as_str()));
        return Ok(system_config_client);
    } else {
        unimplemented!();
    }
}

async fn node_daemon_main_loop(
    node_id: &str,
    node_host_name: &str,
    is_ood: bool,
    is_desktop: bool,
) -> Result<()> {
    let mut loop_step = 0;
    let mut is_running = true;
    let mut last_register_time = 0;
    let mut node_gateway_config_id: Option<ObjId> = None;
    let mut node_gateway_info_id: Option<ObjId> = None;

    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        loop_step += 1;
        info!("node daemon main loop step:{}", loop_step);

        let buckyos_runtime = get_buckyos_api_runtime();
        if buckyos_runtime.is_err() {
            error!("buckyos_runtime is none, will restart node_daemon...");
            return Err(NodeDaemonErrors::SystemConfigError(
                "buckyos_runtime is none, will restart node_daemon!".to_string(),
            ));
        }
        let buckyos_runtime = buckyos_runtime.unwrap();
        let system_config_client = buckyos_runtime.get_system_config_client().await;
        if system_config_client.is_err() {
            error!(
                "get system_config_client failed! {}",
                system_config_client.err().unwrap()
            );
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
                error!(
                    "auto fill device info failed! {}",
                    fill_result.err().unwrap()
                );
                break;
            }
            let device_info_str = serde_json::to_string(&device_info).unwrap();
            debug!("update device info: {:?}", device_info);
            unsafe {
                std::env::set_var("BUCKYOS_THIS_DEVICE_INFO", device_info_str);
            }
            update_device_info(&device_info, &system_config_client).await;
            //TODO：SN的上报频率不用那么快
            let device_session_token_jwt = buckyos_runtime.get_session_token().await;
            report_ood_info_to_sn(&device_info, device_session_token_jwt.as_str(), zone_config)
                .await;
            last_register_time = now;
        }

        if (is_ood) {
            keep_system_config_service(node_id, device_doc, device_private_key, false)
                .await
                .map_err(|err| {
                    error!("start system_config_service failed! {}", err);
                    return NodeDaemonErrors::SystemConfigError(
                        "start system_config_service failed!".to_string(),
                    );
                })?;
        }

        let main_result = node_main(
            node_host_name,
            is_ood,
            is_desktop,
            &system_config_client,
            device_doc,
        )
        .await;

        if main_result.is_err() {
            error!("node_main failed! {}", main_result.err().unwrap());
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        } else {
            is_running = main_result.unwrap();
            if !is_running {
                break;
            }
            let new_node_gateway_info =
                load_node_gateway_info(node_host_name, &system_config_client).await;
            if let Ok(new_node_gateway_info) = new_node_gateway_info {
                let (new_node_gateway_info_id_value, new_node_gateway_info_str) =
                    build_named_object_by_json("node_gateway_info", &new_node_gateway_info);
                let need_write = match node_gateway_info_id.as_ref() {
                    None => true,
                    Some(old_id) => old_id != &new_node_gateway_info_id_value,
                };
                let ensure_result =
                    ensure_node_gateway_dir_pkgs_installed(&new_node_gateway_info).await;
                if let Err(err) = ensure_result {
                    error!("ensure static web dir pkg installed failed! {}", err);
                } else if need_write {
                    node_gateway_info_id = Some(new_node_gateway_info_id_value);
                    info!("node gateway_info changed, will write to node_gateway_info.json");
                    let gateway_info_path =
                        buckyos_kit::get_buckyos_system_etc_dir().join("node_gateway_info.json");
                    std::fs::write(gateway_info_path, new_node_gateway_info_str.as_bytes())
                        .unwrap();
                }
            } else {
                error!("load node gateway_info from system_config failed!");
            }

            info!("node_main OK, load node gateway config ...");
            let new_node_gateway_config =
                load_node_gateway_config(node_host_name, &system_config_client).await;

            if new_node_gateway_config.is_ok() {
                let mut need_reload = false;
                let new_node_gateway_config = new_node_gateway_config.unwrap();
                let (new_node_gateway_config_id, new_node_gateway_config_str) =
                    build_named_object_by_json("nodeconfig", &new_node_gateway_config);
                if node_gateway_config_id.is_none() {
                    need_reload = true;
                    node_gateway_config_id = Some(new_node_gateway_config_id);
                } else {
                    if node_gateway_config_id.as_ref().unwrap() == &new_node_gateway_config_id {
                        need_reload = false;
                    } else {
                        need_reload = true;
                        node_gateway_config_id = Some(new_node_gateway_config_id);
                    }
                }

                if need_reload {
                    info!(
                        "node gateway_config changed, will write to node_gateway.json and reload"
                    );

                    let gateway_config_path =
                        buckyos_kit::get_buckyos_system_etc_dir().join("node_gateway.json");
                    std::fs::write(gateway_config_path, new_node_gateway_config_str.as_bytes())
                        .unwrap();
                }

                let sn = buckyos_runtime.zone_config.as_ref().unwrap().sn.clone();
                info!("*** keep cyfs-gateway service with sn: {:?}", sn);
                keep_cyfs_gateway_service(node_id, device_doc, device_private_key, sn, need_reload)
                    .await
                    .map_err(|err| {
                        error!("keep cyfs_gateway service failed! {}", err);
                    });
            } else {
                error!("load node gateway_cconfig from system_config failed!");
            }
        }
    }
    Ok(())
}

async fn generate_device_session_token(
    device_doc: &DeviceConfig,
    device_private_key: &EncodingKey,
    is_boot: bool,
) -> std::result::Result<String, String> {
    let now = SystemTime::now();
    let since_the_epoch = now.duration_since(UNIX_EPOCH).expect("Time went backwards");
    let timestamp = since_the_epoch.as_secs();
    let login_jti = timestamp.to_string();
    let mut userid = "kernel".to_string();
    if !is_boot {
        userid = device_doc.name.clone();
    }

    let device_session_token = kRPC::RPCSessionToken {
        token_type: kRPC::RPCSessionTokenType::Normal,
        appid: Some("node-daemon".to_string()),
        jti: Some(login_jti),
        session: Some(timestamp),
        sub: Some(userid),
        aud: None,
        exp: Some(timestamp + 60 * 15),
        iss: Some(device_doc.name.clone()),
        token: None,
        extra: HashMap::new(),
    };

    let device_session_token_jwt = device_session_token
        .generate_jwt(Some(device_doc.name.clone()), &device_private_key)
        .map_err(|err| {
            error!("generate device session token failed! {}", err);
            return String::from("generate device session token failed!");
        })?;

    return Ok(device_session_token_jwt);
}

async fn async_main(matches: ArgMatches) -> std::result::Result<(), String> {
    let node_id = matches.get_one::<String>("id");
    let enable_active = matches.get_flag("enable_active");
    let is_desktop = matches.get_flag("desktop_daemon");
    let default_node_id = "node".to_string();
    let node_id = node_id.unwrap_or(&default_node_id);

    info!("node_daemon start...");
    let mut real_machine_config = BuckyOSMachineConfig::default();
    let machine_config = BuckyOSMachineConfig::load_machine_config();
    if machine_config.is_some() {
        real_machine_config = machine_config.unwrap();
    }
    info!("machine_config: {:?}", &real_machine_config);

    init_name_lib(&real_machine_config.web3_bridge)
        .await
        .map_err(|err| {
            error!("init default name client failed! {}", err);
            return String::from("init default name client failed!");
        })?;
    info!("init default name client OK!");

    //load node identity config
    let node_identity_file = get_buckyos_system_etc_dir().join("node_identity.json");
    let mut node_identity = NodeIdentityConfig::load_node_identity_config(&node_identity_file);
    if node_identity.is_err() {
        let desktop_task = if is_desktop {
            info!("start desktop daemon...");
            let task_handle = tokio::task::spawn(desktop_daemon());
            Some(task_handle)
        } else {
            None
        };

        if enable_active {
            //befor start node_active_service ,try check and upgrade self
            info!("node_identity.json load error or not found, start node active service...");
            start_node_active_service().await;
            info!("node active service returned,restart node_daemon ...");
            exit(0);
        } else {
            error!(
                "load node identity config failed! {}",
                node_identity.err().unwrap()
            );
            warn!("Would you like to enable activation mode? (Use `--enable_active` to proceed)");
        }

        if desktop_task.is_some() {
            desktop_task.unwrap().await.unwrap();
            info!("desktop daemon returned,exit node_daemon...");
            exit(0);
        } else {
            return Err(String::from("load node identity config failed!"));
        }
    }
    let node_identity = node_identity.unwrap();
    info!(
        "node_identity.json load OK,  zone_did:{}, owner_did:{}.",
        node_identity.zone_did.to_string(),
        node_identity.owner_did.to_string()
    );
    //verify device_doc by owner_public_key (skip now)
    // {
    //     //let owner_name = node_identity.owner_did.to_string();
    //     let owner_config = resolve_did(&node_identity.owner_did,None).await;
    //     match owner_config {
    //         Ok(owner_config) => {
    //             let owner_config = OwnerConfig::decode(&owner_config,None);
    //             if owner_config.is_ok() {
    //                 let owner_config = owner_config.unwrap();
    //                 let default_key = owner_config.get_default_key();
    //                 if default_key.is_none() {
    //                     warn!("owner public key not defined in owner_config! ");
    //                     return Err("owner public key not defined in owner_config!".to_string());
    //                 }

    //                 let default_key = default_key.unwrap();
    //                 if default_key != node_identity.owner_public_key {
    //                     warn!("owner_config's default key not match to node_identity's owner_public_key! ");
    //                     return Err("owner_config's default key not match to node_identity's owner_public_key!".to_string());
    //                 }
    //             }
    //         }
    //         Err(err) => {
    //             info!("skip resolve owner_config. {} ", err);
    //         }
    //     }
    // }

    let device_doc_json = decode_json_from_jwt_with_default_pk(
        &node_identity.device_doc_jwt,
        &node_identity.owner_public_key,
    )
    .map_err(|err| {
        error!("decode device doc failed! {}", err);
        return String::from("decode device doc from jwt failed!");
    })?;
    let device_doc: DeviceConfig = serde_json::from_value(device_doc_json).map_err(|err| {
        error!("parse device doc failed! {}", err);
        return String::from("parse device doc failed!");
    })?;
    info!("current node's device doc: {:?}", device_doc);

    //load device private key
    let device_private_key_file = get_buckyos_system_etc_dir().join("node_private_key.pem");
    let device_private_key = load_private_key(&device_private_key_file).map_err(|error| {
        error!(
            "load device private key from node_private_key.pem failed! {}",
            error
        );
        return String::from("load device private key failed!");
    })?;

    //lookup zone config
    info!(
        "looking {} zone_boot_config...",
        node_identity.zone_did.to_host_name()
    );
    let zone_boot_config = looking_zone_boot_config(&node_identity)
        .await
        .map_err(|err| {
            error!("looking zone config failed! {}", err);
            String::from("looking zone config failed!")
        })?;
    let zone_config_json_str = serde_json::to_string_pretty(&zone_boot_config).unwrap();
    info!("Load zone_boot_config OK, {}", zone_config_json_str);

    //verify node_name is this device's hostname
    let is_ood = zone_boot_config.device_is_ood(&device_doc.name);
    let device_name = device_doc.name.clone();
    //CURRENT_ZONE_CONFIG.set(zone_config).unwrap();
    if is_ood {
        info!("Booting OOD {} ...", device_doc.name.as_str());
    } else {
        info!("Booting Node {} ...", device_doc.name.as_str());
    }

    unsafe {
        std::env::set_var(
            "BUCKY_ZONE_OWNER",
            serde_json::to_string(&node_identity.owner_public_key).unwrap(),
        );
        std::env::set_var(
            "BUCKYOS_ZONE_BOOT_CONFIG",
            serde_json::to_string(&zone_boot_config).unwrap(),
        );
        std::env::set_var(
            "BUCKYOS_THIS_DEVICE",
            serde_json::to_string(&device_doc).unwrap(),
        );
    }

    info!("set env var BUCKY_ZONE_OWNER,BUCKYOS_ZONE_BOOT_CONFIG,BUCKYOS_THIS_DEVICE OK!");

    // uncomment this when cyfs-gateway support etcd
    // keep_cyfs_gateway_service(node_id.as_str(),&device_doc, &device_private_key,
    //     zone_boot_config.sn.clone(),false).await.map_err(|err| {
    //         error!("init cyfs_gateway service failed! {}", err);
    //         return String::from("init cyfs_gateway service failed!");
    // })?;

    let device_session_token_jwt =
        generate_device_session_token(&device_doc, &device_private_key, true)
            .await
            .map_err(|err| {
                error!("generate device session token failed! {}", err);
                return String::from("generate device session token failed!");
            })?;

    //init kernel_service:system_config service
    let mut syc_cfg_client: SystemConfigClient;
    let boot_config: serde_json::Value;
    let mut boot_config_result_str = "".to_string();
    if is_ood {
        keep_system_config_service(node_id.as_str(), &device_doc, &device_private_key, false)
            .await
            .map_err(|err| {
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
            error!(
                "auto fill device info failed! {}",
                fill_result.err().unwrap()
            );
            return Err(String::from("auto fill device info failed!"));
        }
        update_device_info(&device_info, &syc_cfg_client).await;

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
                    unsafe {
                        std::env::set_var(
                            "SCHEDULER_SESSION_TOKEN",
                            device_session_token_jwt.clone(),
                        );
                    }
                    debug!(
                        "set var SCHEDULER_SESSION_TOKEN {}",
                        device_session_token_jwt
                    );
                    let boot_result = do_boot_schedule().await;
                    if boot_result.is_ok() {
                        warn!("BuckyOS BOOT_INIT OK, will enter system after 2 secs.");
                        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                    } else {
                        warn!(
                            "do boot schedule failed! {}, will retry after 5 secs...",
                            boot_result.err().unwrap()
                        );
                        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                    }
                }
                buckyos_api::SytemConfigResult::Ok(r) => {
                    boot_config_result_str = r.value.clone();
                    info!("Load boot config OK, {}", boot_config_result_str.as_str());
                }
                _ => {
                    error!(
                        "get boot config failed! {},wait 5 sec to retry...",
                        get_result.err().unwrap()
                    );
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                }
            }
        }

        let zone_config: ZoneConfig = serde_json::from_str(boot_config_result_str.as_str())
            .map_err(|err| {
                error!("parse zone config from boot/config failed! {}", err);
                return String::from("parse zone config from boot/config failed!");
            })?;
        unsafe {
            std::env::set_var("BUCKYOS_ZONE_CONFIG", boot_config_result_str);
        }
        info!("--------------------------------");

        let mut runtime = BuckyOSRuntime::new("node-daemon", None, BuckyOSRuntimeType::Kernel);
        runtime.fill_policy_by_load_config().await.map_err(|err| {
            error!("fill policy by load config failed! {}", err);
            return String::from("fill policy by load config failed!");
        })?;

        runtime.device_config = Some(device_doc);
        runtime.device_private_key = Some(device_private_key);
        runtime.device_info = Some(device_info);
        runtime.zone_id = node_identity.zone_did.clone();
        runtime.zone_config = Some(zone_config);
        runtime.session_token = Arc::new(RwLock::new(device_session_token_jwt.clone()));
        runtime.force_https = false;
        set_buckyos_api_runtime(runtime);
    } else {
        //this node is not ood: try connect to system_config_service
        let mut runtime = init_buckyos_api_runtime("node-daemon", None, BuckyOSRuntimeType::Kernel)
            .await
            .map_err(|e| {
                error!("init_buckyos_api_runtime failed: {:?}", e);
                return String::from("init_buckyos_api_runtime failed!");
            })?;

        loop {
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

    info!(
        "{}@{} boot OK, enter node daemon main loop.",
        &device_name,
        node_identity.zone_did.to_host_name()
    );
    node_daemon_main_loop(node_id, &device_name, is_ood, is_desktop)
        .await
        .map_err(|err| {
            error!("node daemon main loop failed! {}", err);
            return String::from("node daemon main loop failed!");
        })?;

    Ok(())
}

pub(crate) fn run(matches: ArgMatches) -> std::result::Result<(), String> {
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    if num_cpus::get() < 2 {
        builder.worker_threads(2);
    }
    let runtime = builder.enable_all().build().map_err(|err| {
        error!("create tokio runtime failed: {err}");
        String::from("create tokio runtime failed!")
    })?;

    runtime.block_on(async_main(matches))
}

#[cfg(test)]
mod tests {
    use super::*;
    use named_store::{NamedLocalStore, StoreLayout, StoreTarget};
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn create_temp_pkg_env_path(test_name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("node-daemon-{test_name}-{unique}"));
        std::fs::create_dir_all(path.join("pkgs")).unwrap();
        path
    }

    fn cleanup_temp_pkg_env_path(path: &Path) {
        let _ = std::fs::remove_dir_all(path);
    }

    fn index_test_pkg(pkg_env_path: &Path, unique_name: &str, version: &str) -> (String, String) {
        let owner = DID::from_str("did:bns:test").unwrap();
        let pkg_name = format!("{}.{}", PackageEnvConfig::get_default_prefix(), unique_name);
        let pkg_meta = PackageMeta::new(pkg_name.as_str(), version, "test", &owner, None);
        let pkg_meta_str = serde_json::to_string(&pkg_meta).unwrap();
        let (meta_obj_id, _) = pkg_meta.gen_obj_id();
        let meta_obj_id_str = meta_obj_id.to_string();
        let meta_db = MetaIndexDb::new(pkg_env_path.join("pkgs/meta_index.db"), false).unwrap();
        let package_id = pkg_meta.get_package_id();

        meta_db
            .add_pkg_meta(
                meta_obj_id_str.as_str(),
                pkg_meta_str.as_str(),
                pkg_meta.author.as_str(),
                None,
            )
            .unwrap();
        meta_db
            .set_pkg_version(
                package_id.name.as_str(),
                pkg_meta.author.as_str(),
                pkg_meta.version.as_str(),
                meta_obj_id_str.as_str(),
                pkg_meta.version_tag.as_deref(),
            )
            .unwrap();

        (pkg_name, meta_obj_id_str)
    }

    fn build_static_web_gateway_info(dir_pkg_id: &str, dir_pkg_objid: &str) -> Value {
        json!({
            "app_info": {
                "portal": {
                    "app_id": "portal",
                    "dir_pkg_id": dir_pkg_id,
                    "dir_pkg_objid": dir_pkg_objid
                }
            }
        })
    }

    async fn create_test_store_mgr(base_dir: &Path) -> NamedStoreMgr {
        let store = NamedLocalStore::get_named_store_by_path(base_dir.join("named_store"))
            .await
            .unwrap();
        let store_id = store.store_id().to_string();
        let store_ref = Arc::new(tokio::sync::Mutex::new(store));

        let store_mgr = NamedStoreMgr::new();
        store_mgr.register_store(store_ref).await;
        store_mgr
            .add_layout(StoreLayout::new(
                1,
                vec![StoreTarget {
                    store_id,
                    device_did: None,
                    capacity: None,
                    used: None,
                    readonly: false,
                    enabled: true,
                    weight: 1,
                }],
                0,
                0,
            ))
            .await;

        store_mgr
    }

    #[tokio::test]
    async fn test_index_static_dir_pkg_meta_from_named_store_populates_env_meta_db() {
        let pkg_env_path = create_temp_pkg_env_path("index_static_dir_pkg_meta_from_named_store");
        let store_mgr = create_test_store_mgr(pkg_env_path.as_path()).await;
        let owner = DID::from_str("did:bns:test").unwrap();
        let pkg_name = "portal-web".to_string();
        let pkg_meta = PackageMeta::new(pkg_name.as_str(), "1.0.0", "test", &owner, None);
        let (meta_obj_id, pkg_meta_str) = pkg_meta.gen_obj_id();
        store_mgr
            .put_object(&meta_obj_id, pkg_meta_str.as_str())
            .await
            .unwrap();

        let resolved_pkg_id = format!("portal-web#{}", meta_obj_id);
        index_static_dir_pkg_meta_from_named_store(
            pkg_env_path.as_path(),
            resolved_pkg_id.as_str(),
            &store_mgr,
        )
        .await
        .unwrap();

        let pkg_env = PackageEnv::new(pkg_env_path.clone());
        let (indexed_meta_obj_id, indexed_meta) = pkg_env
            .get_pkg_meta(resolved_pkg_id.as_str())
            .await
            .unwrap();

        cleanup_temp_pkg_env_path(&pkg_env_path);
        assert_eq!(indexed_meta_obj_id, meta_obj_id.to_string());
        assert_eq!(
            indexed_meta.name,
            format!(
                "{}.{}",
                PackageEnvConfig::get_default_prefix(),
                "portal-web"
            )
        );
        assert_eq!(indexed_meta.version, pkg_meta.version);
    }

    #[tokio::test]
    async fn test_ensure_node_gateway_dir_pkgs_installed_accepts_exact_strict_pkg() {
        let pkg_env_path =
            create_temp_pkg_env_path("ensure_node_gateway_dir_pkgs_installed_accepts_exact");
        let (pkg_name, meta_obj_id) = index_test_pkg(&pkg_env_path, "portal-web", "1.0.0");
        let strict_dir = pkg_env_path
            .join("pkgs")
            .join(pkg_name)
            .join(ObjId::new(meta_obj_id.as_str()).unwrap().to_filename());
        std::fs::create_dir_all(&strict_dir).unwrap();

        let gateway_info = build_static_web_gateway_info("portal-web", meta_obj_id.as_str());
        let result =
            ensure_node_gateway_dir_pkgs_installed_in_env(pkg_env_path.as_path(), &gateway_info)
                .await;

        cleanup_temp_pkg_env_path(&pkg_env_path);
        result.unwrap();
    }

    #[tokio::test]
    async fn test_ensure_node_gateway_dir_pkgs_installed_does_not_accept_friendly_dir_for_exact_pkg(
    ) {
        let pkg_env_path =
            create_temp_pkg_env_path("ensure_node_gateway_dir_pkgs_installed_requires_exact");
        let (_, meta_obj_id) = index_test_pkg(&pkg_env_path, "portal-web", "1.0.0");
        let friendly_dir = pkg_env_path.join("portal-web");
        std::fs::create_dir_all(&friendly_dir).unwrap();

        let gateway_info = build_static_web_gateway_info("portal-web", meta_obj_id.as_str());
        let result =
            ensure_node_gateway_dir_pkgs_installed_in_env(pkg_env_path.as_path(), &gateway_info)
                .await;

        cleanup_temp_pkg_env_path(&pkg_env_path);
        let err = result.expect_err("friendly dir should not satisfy exact pkg objid");
        match err {
            NodeDaemonErrors::ReasonError(msg) => {
                assert!(msg.contains("portal-web"));
                assert!(msg.contains("after install failed") || msg.contains("install static web"));
            }
            other => panic!("unexpected error: {}", other),
        }
    }
}
