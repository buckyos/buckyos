use futures::FutureExt;
use name_lib::DIDDocumentTrait;
use windows::core::PCWSTR;
use std::collections::{HashMap, HashSet};
use std::ffi::{CString, OsStr};
use std::os::raw::{c_char, c_int, c_void};
use std::os::windows::ffi::OsStrExt;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task;

use buckyos_kit::*;
use serde::{Deserialize, Serialize};
use std::fs;

use sysinfo::System;
use tokio::sync::Mutex;

#[cfg(windows)]
use windows::{
    Win32::UI::Shell::ShellExecuteW,
    Win32::Foundation::{HWND, HINSTANCE},
    Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
};

lazy_static::lazy_static! {
    static ref g_runtime: tokio::runtime::Runtime = tokio::runtime::Runtime::new().unwrap();

    static ref bucky_status_scaner_mgr: Mutex<BuckyStatusScanerMgr> = Mutex::new(BuckyStatusScanerMgr {
        next_seq: 1,
        scaners: HashMap::new()
    });

    static ref buckyos_process: HashSet<&'static str, > = {
        let mut set = HashSet::new();
        set.extend(["node_daemon", "scheduler", "verify_hub", "system_config", "cyfs_gateway"]);
        set
    };

    static ref node_infomation: Arc<Mutex<Option<NodeInfomationObj>>> = Arc::new(Mutex::new(None));
}

struct BuckyStatusScanerMgr {
    next_seq: u32,
    scaners: HashMap<u32, mpsc::Sender<()>>,
}

#[repr(C)]
#[derive(PartialEq, Eq, Copy, Clone)]
enum BuckyStatus {
    Running = 0,
    Stopped = 1,
    NotActive = 2,
    NotInstall = 3,
    Failed = 4,
}

#[repr(C)]
struct BuckyStatusScaner(u32);

struct NodeInfomationObj {
    node_id: String,
    home_page_url: String,
    node_host_name: String,
    sys_cfg_client: sys_config::SystemConfigClient,
}

#[repr(C)]
struct NodeInfomation {
    node_id: *mut c_char,
    home_page_url: *mut c_char,
}

unsafe impl Send for NodeInfomation {}
unsafe impl Sync for NodeInfomation {}

type ScanStatusCallback =
    extern "C" fn(new_status: BuckyStatus, old_status: BuckyStatus, userdata: *const c_void);

#[no_mangle]
extern "C" fn bucky_status_scaner_scan(
    callback: ScanStatusCallback,
    userdata: *const c_void,
    _hwnd: *const c_void,
) -> *mut BuckyStatusScaner {
    let (sender, mut receiver) = mpsc::channel(32);

    // Wrap both callback and userdata in a Send-safe wrapper
    struct CallbackWrapper {
        callback: ScanStatusCallback,
        userdata: *const c_void,
        _hwnd: *const c_void,
    }
    unsafe impl Send for CallbackWrapper {}
    unsafe impl Sync for CallbackWrapper {}
    let callback_wrapper = Arc::new(CallbackWrapper {
        callback,
        userdata,
        _hwnd,
    });

    g_runtime.spawn(async move {
        let mut status = BuckyStatus::Stopped;
        let mut interval = std::time::Duration::from_millis(1);
        loop {
            futures::select! {
                _ = receiver.recv().fuse() => {
                    log::info!("will stop scan status of buckyos!");
                    break;
                },
                _ = tokio::time::sleep(interval).fuse() => {
                    let old_status = status;
                    status = BuckyStatus::Stopped;

                    let bin_dir = get_buckyos_system_bin_dir();

                    log::info!("buckyos has been installed at: {:?}", bin_dir);

                    let is_dir = match fs::metadata(bin_dir) {
                        Ok(meta) if meta.is_dir() => true,
                        _ => false
                    };
                    if !is_dir {
                        status = BuckyStatus::NotInstall;
                        interval = std::time::Duration::from_millis(5000);
                        log::warn!("buckyos status: NotInstall");
                    }

                    if status != BuckyStatus::NotInstall {
                        let mut system = System::new_all();
                        system.refresh_all();
                        let mut exist_process = HashSet::new();
                        
                        #[cfg(windows)]
                        let ext_path = ".exe";

                        let mut not_exist_process = buckyos_process.iter().map(|name| name.to_string() + ext_path).collect::<HashSet<_>>();
                        let node_daemon_process = "node_daemon".to_string() + ext_path;

                        for process in system.processes().values() {
                            let name = process.name().to_ascii_lowercase().into_string().unwrap();

                            if node_daemon_process == name {
                                unsafe {
                                    let info = get_node_info_impl().await;
                                    if info.is_null() || (*info).node_id.is_null() {
                                        status = BuckyStatus::NotActive;
                                        log::warn!("buckyos status: NotActive");
                                    } else {
                                        status = BuckyStatus::Running;
                                        log::info!("buckyos status: Running");
                                    }
                                    free_node_info(info);
                                }
                                interval = std::time::Duration::from_millis(5000);
                                break;
                            }

                            if buckyos_process.contains(name.as_str()) {
                                not_exist_process.remove(name.as_str());
                                exist_process.insert(name);
                            }
                        }

                        if status != BuckyStatus::Running && status != BuckyStatus::NotActive {
                            if !not_exist_process.is_empty() {
                                if !exist_process.is_empty() {
                                    status = BuckyStatus::Failed;
                                    interval = std::time::Duration::from_millis(500);
                                    log::warn!("buckyos status: Failed");
                                } else {
                                    status = BuckyStatus::Stopped;
                                    interval = std::time::Duration::from_millis(5000);
                                    log::warn!("buckyos status: Stopped");
                                }
                            }
                        }
                    }

                    if status != old_status {
                        (callback_wrapper.callback)(status, old_status, callback_wrapper.userdata);
                    }
                }
            }
        }
    });

    g_runtime.block_on(async move {
        let mut scaner_mgr = bucky_status_scaner_mgr.lock().await;
        let seq = scaner_mgr.next_seq;
        scaner_mgr.next_seq = scaner_mgr.next_seq + 1;
        scaner_mgr.scaners.insert(seq, sender);

        Box::into_raw(Box::new(BuckyStatusScaner(seq)))
    })
}

#[no_mangle]
extern "C" fn bucky_status_scaner_stop(scaner: *mut BuckyStatusScaner) {
    if !scaner.is_null() {
        let scaner = unsafe { Box::from_raw(scaner) };

        task::spawn(async move {
            let mut scaner_mgr = bucky_status_scaner_mgr.lock().await;
            let scaner = scaner_mgr.scaners.remove(&scaner.0);
            if let Some(scaner) = scaner {
                let _ = scaner.send(()).await;
            }
        });
    }
}

#[repr(C)]
struct ApplicationInfo {
    id: *mut c_char,
    name: *mut c_char,
    icon_path: *mut c_char,
    home_page_url: *mut c_char,
    is_running: c_char,
}

struct ApplicationInfoRust {
    id: String,
    name: String,
    icon_path: String,
    home_page_url: String,
    is_running: bool,
}

type ListAppCallback = extern "C" fn(
    is_success: c_char,
    apps: *const ApplicationInfo,
    app_count: c_int,
    seq: c_int,
    user_data: *const c_void,
);

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum RunItemTargetState {
    Running,
    Stopped,
}

impl RunItemTargetState {
    pub fn from_str(state: &str) -> Result<Self, String> {
        match state {
            "Running" => Ok(RunItemTargetState::Running),
            "Stopped" => Ok(RunItemTargetState::Stopped),
            _ => Err(format!("invalid target state: {}", state)),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RunItemControlOperation {
    pub command: String,
    pub params: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize)]
pub struct KernelServiceConfig {
    pub target_state: RunItemTargetState,
    pub pkg_id: String,
    pub operations: HashMap<String, RunItemControlOperation>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct AppServiceConfig {
    pub target_state: String,
    pub app_id: String,
    pub user_id: String,

    pub docker_image_name: Option<String>,
    pub data_mount_point: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_mount_point: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_cache_mount_point: Option<String>,
    //extra mount pint, real_path:docker_inner_path
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_mounts: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_cpu_num: Option<u32>,
    // 0 - 100
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_cpu_percent: Option<u32>,

    // memory quota in bytes
    pub memory_quota: Option<u64>,

    // target port ==> real port in docker
    pub tcp_ports: HashMap<u16, u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udp_ports: Option<HashMap<u16, u16>>,
    //pub service_image_name : String, // support mutil platform image name (arm/x86...)
}

#[derive(Serialize, Deserialize, Debug)]
pub struct FrameServiceConfig {
    pub target_state: RunItemTargetState,
    //pub name : String, // service name
    pub pkg_id: String,
    pub operations: HashMap<String, RunItemControlOperation>,

    //不支持serizalize
    #[serde(skip)]
    service_pkg: Option<package_lib::MediaInfo>,
}

#[derive(Serialize, Deserialize)]
struct NodeConfig {
    revision: u64,
    kernel: HashMap<String, KernelServiceConfig>,
    apps: HashMap<String, AppServiceConfig>,
    services: HashMap<String, FrameServiceConfig>,
    is_running: bool,
}

async fn load_node_config(
    node_host_name: &str,
    sys_config_client: &sys_config::SystemConfigClient,
) -> Result<NodeConfig, String> {
    let json_config_path = format!("{}_node_config.json", node_host_name);
    let json_config = std::fs::read_to_string(json_config_path);
    if json_config.is_ok() {
        let json_config = json_config.unwrap();
        let node_config = serde_json::from_str(json_config.as_str()).map_err(|err| {
            log::error!("parse DEBUG node config failed! {}", err);
            "parse DEBUG node config failed!".to_string()
        })?;

        log::warn!(
            "Debug load node config from ./{}_node_config.json success!",
            node_host_name
        );
        return Ok(node_config);
    }

    let node_key = format!("nodes/{}/config", node_host_name);
    let (node_cfg_result, rversion) =
        sys_config_client
            .get(node_key.as_str())
            .await
            .map_err(|error| {
                log::error!("get node config failed from etcd! {}", error);
                "get node config failed from system_config_service!".to_string()
            })?;

    let node_config = serde_json::from_str(&node_cfg_result).map_err(|err| {
        log::error!("parse node config failed! {}", err);
        "parse node config failed!".to_string()
    })?;

    Ok(node_config)
}

async fn set_node_config(
    node_host_name: &str,
    sys_config_client: &sys_config::SystemConfigClient,
    json_path: &str,
    value: &str,
) -> Result<(), String> {
    let node_key = format!("nodes/{}/config", node_host_name);
    let _ = sys_config_client
        .set_by_json_path(node_key.as_str(), json_path, value)
        .await
        .map_err(|error| {
            log::error!("get node config failed from etcd! {}", error);
            "get node config failed from system_config_service!".to_string()
        })?;

    Ok(())
}

async fn list_application_rust(
    node_host_name: &str,
    sys_config_client: &sys_config::SystemConfigClient,
) -> Result<Vec<ApplicationInfoRust>, String> {
    let node_config = load_node_config(node_host_name, sys_config_client)
        .await
        .map_err(|err| {
            log::error!("load node config failed! {}", err);
            "cann't load node config!".to_string()
        })?;

    let apps = node_config
        .apps
        .into_iter()
        .map(|(app_id_with_name, app_cfg)| {
            let target_state = RunItemTargetState::from_str(&app_cfg.target_state).unwrap();
            ApplicationInfoRust {
                id: app_id_with_name.clone(),
                name: app_id_with_name,
                icon_path: "".to_string(),
                home_page_url: "https://www.google.com".to_string(),
                is_running: target_state == RunItemTargetState::Running,
            }
        })
        .collect::<Vec<_>>();

    Ok(apps)
}

#[no_mangle]
extern "C" fn list_application(seq: c_int, callback: ListAppCallback, userdata: *const c_void) {
    struct CallbackWrapper {
        callback: ListAppCallback,
        userdata: *const c_void,
    }
    unsafe impl Send for CallbackWrapper {}
    unsafe impl Sync for CallbackWrapper {}
    let callback_wrapper = Arc::new(CallbackWrapper { callback, userdata });

    g_runtime.spawn(async move {
        let mut apps = vec![];
        {
            let info = node_infomation.lock().await;
            if let Some(node_info) = info.as_ref() {
                match tokio::time::timeout(
                    std::time::Duration::from_millis(500),
                    list_application_rust(
                        node_info.node_host_name.as_str(),
                        &node_info.sys_cfg_client,
                    ),
                )
                .await
                {
                    Ok(result_apps) => match result_apps {
                        Ok(mut result_apps) => {
                            std::mem::swap(&mut apps, &mut result_apps);
                        }
                        Err(err) => {
                            log::error!("{}", err);
                        }
                    },
                    Err(err) => {
                        log::error!("{}", err);
                    }
                }
            }
        }

        let apps = apps
            .into_iter()
            .map(|app| ApplicationInfo {
                id: CString::new(app.id)
                    .expect("no memory for c_app_id")
                    .into_raw(),
                name: CString::new(app.name)
                    .expect("no memory for c_app_name")
                    .into_raw(),
                icon_path: CString::new(app.icon_path)
                    .expect("no memory for c_app_name")
                    .into_raw(),
                home_page_url: CString::new(app.home_page_url)
                    .expect("no memory for c_app_name")
                    .into_raw(),
                is_running: if app.is_running { 1 } else { 0 },
            })
            .collect::<Vec<_>>();
        (callback_wrapper.callback)(
            1,
            apps.as_ptr(),
            apps.len() as i32,
            seq,
            callback_wrapper.userdata,
        );
        apps.into_iter().for_each(|app| unsafe {
            let _ = CString::from_raw(app.id);
            let _ = CString::from_raw(app.name);
            let _ = CString::from_raw(app.icon_path);
            let _ = CString::from_raw(app.home_page_url);
        });
    });
}

//NodeIdentity from ood active progress
#[derive(Deserialize, Debug)]
struct NodeIdentityConfig {
    zone_name: String,                        // $name.buckyos.org or did:ens:$name
    owner_public_key: jsonwebtoken::jwk::Jwk, //owner is zone_owner
    owner_name: String,                       //owner's name
    device_doc_jwt: String,                   //device document,jwt string,siged by owner
    zone_nonce: String,                       // random string, is default password of some service
                                              //device_private_key: ,storage in partical file
}

type NodeId = String;
type StrError = String;

fn list_nodes() -> Result<HashMap<NodeId, NodeIdentityConfig>, StrError> {
    let etc_dir = get_buckyos_system_etc_dir();

    let mut nodes = HashMap::new();

    for entry in fs::read_dir(etc_dir).map_err(|err| err.to_string())? {
        let entry = entry.map_err(|err| err.to_string())?;
        let file_path = entry.path();

        if file_path.is_file() {
            if let Some(file_name) = file_path.file_name().and_then(|name| name.to_str()) {
                if let Some(node_id) = file_name.strip_suffix("_identity.toml") {
                    let contents = std::fs::read_to_string(file_path.as_path())
                        .map_err(|err| err.to_string())?;

                    let config: NodeIdentityConfig =
                        toml::from_str(&contents).map_err(|err| err.to_string())?;

                    nodes.insert(node_id.to_string(), config);
                }
            }
        }
    }

    Ok(nodes)
}

async fn looking_zone_config(
    node_identity: &NodeIdentityConfig,
) -> Result<name_lib::ZoneConfig, String> {
    //If local files exist, priority loads local files
    let etc_dir = get_buckyos_system_etc_dir();
    let json_config_path = format!(
        "{}/{}_zone_config.json",
        etc_dir.to_string_lossy(),
        node_identity.zone_name
    );
    log::info!(
        "try load zone config from {} for debug",
        json_config_path.as_str()
    );
    let json_config = std::fs::read_to_string(json_config_path.clone());
    if json_config.is_ok() {
        let zone_config = serde_json::from_str(&json_config.unwrap());
        if zone_config.is_ok() {
            log::warn!(
                "debug load zone config from {} success!",
                json_config_path.as_str()
            );
            return Ok(zone_config.unwrap());
        } else {
            log::error!(
                "parse debug zone config {} failed! {}",
                json_config_path.as_str(),
                zone_config.err().unwrap()
            );
            return Err("parse debug zone config from local file failed!".to_string());
        }
    }

    let mut zone_did = node_identity.zone_name.clone();
    log::info!(
        "node_identity.owner_public_key: {:?}",
        node_identity.owner_public_key
    );
    let owner_public_key = jsonwebtoken::DecodingKey::from_jwk(&node_identity.owner_public_key)
        .map_err(|err| {
            log::error!("parse owner public key failed! {}", err);
            return "parse owner public key failed!".to_string();
        })?;

    if !name_lib::is_did(node_identity.zone_name.as_str()) {
        //owner zone is a NAME, need query NameInfo to get DID
        log::info!("owner zone is a NAME, try nameclient.query to get did");

        let zone_jwt = name_client::resolve(node_identity.zone_name.as_str(), Some(name_client::RecordType::DID))
            .await
            .map_err(|err| {
                log::error!("query zone config by nameclient failed! {}", err);
                "query zone config failed!".to_string()
            })?;

        if zone_jwt.did_document.is_none() {
            log::error!("get zone jwt failed!");
            return Err("get zone jwt failed!".to_string());
        }
        let zone_jwt = zone_jwt.did_document.unwrap();
        log::info!("zone_jwt: {:?}", zone_jwt);

        let mut zone_config = name_lib::ZoneConfig::decode(&zone_jwt, Some(&owner_public_key))
            .map_err(|err| {
                log::error!("parse zone config failed! {}", err);
                return "parse zone config failed!".to_string();
            })?;

        zone_did = zone_config.did.clone();
        zone_config.owner_name = Some(node_identity.owner_name.clone());
        zone_config.name = Some(node_identity.zone_name.clone());
        let zone_config_json = serde_json::to_value(zone_config).unwrap();
        let cache_did_doc = name_lib::EncodedDocument::JsonLd(zone_config_json);
        name_client::add_did_cache(zone_did.as_str(), cache_did_doc)
            .await
            .unwrap();
        log::info!("add zone did {}  to cache success!", zone_did);
    }

    //try load lasted document from name_lib
    let zone_doc: name_lib::EncodedDocument = name_client::resolve_did(zone_did.as_str(), None)
        .await
        .map_err(|err| {
            log::error!("resolve zone did failed! {}", err);
            "resolve zone did failed!".to_string()
        })?;

    let mut zone_config = name_lib::ZoneConfig::decode(&zone_doc, Some(&owner_public_key))
        .map_err(|err| {
            log::error!("parse zone config failed! {}", err);
            "parse zone config failed!".to_string()
        })?;

    if zone_config.name.is_none() {
        zone_config.name = Some(node_identity.zone_name.clone());
    }

    return Ok(zone_config);
}

fn load_device_private_key(node_id: &str) -> Result<jsonwebtoken::EncodingKey, String> {
    let mut file_path = format!("{}_private_key.pem", node_id);
    let path = std::path::Path::new(file_path.as_str());
    if path.exists() {
        log::warn!("debug load device private_key from ./device_private_key.pem");
    } else {
        let etc_dir = get_buckyos_system_etc_dir();
        file_path = format!("{}/{}_private_key.pem", etc_dir.to_string_lossy(), node_id);
    }
    let private_key = std::fs::read_to_string(file_path.clone()).map_err(|err| {
        log::error!("read device private key failed! {}", err);
        "read device private key failed!".to_string()
    })?;

    let private_key =
        jsonwebtoken::EncodingKey::from_ed_pem(private_key.as_bytes()).map_err(|err| {
            log::error!("parse device private key failed! {}", err);
            "parse device private key failed!".to_string()
        })?;

    log::info!("load device private key from {} success!", file_path);
    Ok(private_key)
}

async fn select_node() -> Result<Option<NodeInfomationObj>, String> {
    let nodes = list_nodes()?;
    if let Some((node_id, cfg)) = nodes.iter().next() {
        let device_doc_json = name_lib::decode_json_from_jwt_with_default_pk(
            &cfg.device_doc_jwt,
            &cfg.owner_public_key,
        )
        .map_err(|err| format!("decode device doc failed! {}", err))?;
        let device_doc = serde_json::from_value::<name_lib::DeviceConfig>(device_doc_json)
            .map_err(|err| format!("parse device doc failed! {}", err))?;

        let zone_config = looking_zone_config(cfg).await.map_err(|err| {
            log::error!("looking zone config failed! {}", err);
            String::from("looking zone config failed!")
        })?;
        let is_ood = zone_config.oods.contains(&device_doc.name);

        let now = std::time::SystemTime::now();
        let since_the_epoch = now
            .duration_since(std::time::UNIX_EPOCH)
            .expect("Time went backwards");
        let timestamp = since_the_epoch.as_secs();
        let device_session_token = kRPC::RPCSessionToken {
            token_type: kRPC::RPCSessionTokenType::JWT,
            nonce: None,
            userid: Some(device_doc.name.clone()),
            appid: Some("kernel".to_string()),
            exp: Some(timestamp + 3600 * 24 * 7),
            iss: Some(device_doc.name.clone()),
            token: None,
        };
        let device_private_key = load_device_private_key(&node_id).map_err(|error| {
            log::error!("load device private key failed! {}", error);
            String::from("load device private key failed!")
        })?;
        let device_session_token_jwt = device_session_token
            .generate_jwt(Some(device_doc.did.clone()), &device_private_key)
            .map_err(|err| {
                log::error!("generate device session token failed! {}", err);
                return String::from("generate device session token failed!");
            })?;

        let sys_cfg_client = if is_ood {
            sys_config::SystemConfigClient::new(None, Some(device_session_token_jwt.as_str()))
        } else {
            let this_device = name_lib::DeviceInfo::from_device_doc(&device_doc);
            let system_config_url =
                name_client::get_system_config_service_url(Some(&this_device), &zone_config, false)
                    .await
                    .map_err(|err| {
                        log::error!("get system_config_url failed! {}", err);
                        String::from("get system_config_url failed!")
                    })?;
            sys_config::SystemConfigClient::new(
                Some(system_config_url.as_str()),
                Some(device_session_token_jwt.as_str()),
            )
        };
        Ok(Some(NodeInfomationObj {
            node_id: node_id.to_owned(),
            home_page_url: format!("http://{}.web3.buckyos.io", cfg.owner_name),
            node_host_name: device_doc.name,
            sys_cfg_client,
        }))
    } else {
        Ok(None)
    }
}


async fn get_node_info_impl() -> *mut NodeInfomation {
    let mut info = node_infomation.lock().await;
    let is_actived = info.is_some();
    if !is_actived {
        if let Ok(node) = select_node().await {
            *info = node;
        }
    }

    let is_actived = info.is_some();
    let c_info = if is_actived {
        let info = info.as_ref().unwrap();
        NodeInfomation {
            node_id: CString::new(info.node_id.clone())
                .expect("no memory for c_node_id")
                .into_raw(),
            home_page_url: CString::new(info.home_page_url.clone())
                .expect("no memory for c_home_page_url")
                .into_raw(),
        }
    } else {
        NodeInfomation {
            node_id: std::ptr::null_mut(),
            home_page_url: CString::new("http://127.0.0.1:3180/index.html")
                .expect("no memory for c_home_page_url")
                .into_raw(),
        }
    };

    Box::into_raw(Box::new(c_info))
}

#[no_mangle]
extern "C" fn get_node_info() -> *mut NodeInfomation {
    g_runtime.block_on(async move {
        get_node_info_impl().await
    })
}

#[no_mangle]
extern "C" fn free_node_info(info: *mut NodeInfomation) {
    if !info.is_null() {
        unsafe {
            let info = Box::from_raw(info);
            if !info.node_id.is_null() {
                let _ = CString::from_raw(info.node_id);
            }
            if !info.home_page_url.is_null() {
                let _ = CString::from_raw(info.home_page_url);
            }
        }
    }
}

fn to_wide_string(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

#[cfg(windows)]
fn run_as_admin(command: &str, parameters: Option<&str>) -> Result<HINSTANCE, ()> {
    let operation = to_wide_string("runas");
    let command_wide = to_wide_string(command);
    let params_wide = parameters.map(|p| to_wide_string(p));

    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(operation.as_ptr() as *const u16),
            PCWSTR(command_wide.as_ptr() as *const u16),
            PCWSTR(params_wide.as_ref().map_or(std::ptr::null(), |v| v.as_ptr()) as *const u16),
            PCWSTR(std::ptr::null() as *const u16),
            SW_SHOWNORMAL,
        )
    };

    if result.0 as isize > 32 {
        Ok(result)
    } else {
        Err(())
    }
}

#[cfg(windows)]
fn start_buckyos_service() -> Result<HINSTANCE, ()> {
    run_as_admin("cmd.exe", Some("/C net start buckyos"))
}

#[cfg(windows)]
fn stop_buckyos_service() -> Result<HINSTANCE, ()> {
    run_as_admin("cmd.exe", Some("/C net stop buckyos"))
}

#[no_mangle]
extern "C" fn start_buckyos() {
    // let deamon_path = get_buckyos_system_bin_dir().join("node_deamon");

    // #[cfg(windows)]
    // let deamon_path = deamon_path.join(".exe");

    // let command = g_runtime.block_on(async move {
    //     let node_info = node_infomation.lock().await;
    //     match node_info.as_ref() {
    //         Some(info) => {
    //             format!(
    //                 "{} --enable_active --node_id {}",
    //                 deamon_path.display(),
    //                 info.node_id
    //             )
    //         }
    //         None => {
    //             format!("{} --enable_active", deamon_path.display())
    //         }
    //     }
    // });

    // let mut command = std::process::Command::new(command);

    // #[cfg(windows)]
    // {
    //     command.creation_flags(0x00000008 | 0x00000010); // DETACHED_PROCESS | CREATE_NO_WINDOW
    // }

    // match command.spawn() {
    //     Ok(_) => println!("Process started successfully"),
    //     Err(e) => eprintln!("Failed to start process: {}", e),
    // }
    
    #[cfg(windows)]
    let _ = start_buckyos_service();
}

#[no_mangle]
extern "C" fn stop_buckyos() {
    #[cfg(windows)]
    let _ = stop_buckyos_service();

    let mut system = System::new_all();
    system.refresh_all();

    let mut kill_count = 0;
    for (_, process) in system.processes() {
        let name = std::path::PathBuf::from(process.name());

        #[cfg(windows)]
        let name = name.with_extension("");

        let name = name.as_os_str().to_ascii_lowercase().into_string().unwrap();

        if buckyos_process.contains(name.as_str()) {
            process.kill();
            kill_count += 1;
            if kill_count >= buckyos_process.len() {
                break;
            }
        }
    }
}

#[no_mangle]
extern "C" fn start_app(app_id: *mut c_char) {
    if app_id.is_null() {
        return;
    }
    let c_app_id = unsafe { CString::from_raw(app_id) };

    g_runtime.block_on(async move {
        let info = node_infomation.lock().await;
        if let Some(node_info) = info.as_ref() {
            let _ = set_node_config(
                node_info.node_host_name.as_str(),
                &node_info.sys_cfg_client,
                format!("apps/{:?}/target_state", c_app_id).as_str(),
                "Running",
            )
            .await;
        }
        let _ = c_app_id.into_raw();
    });
}

#[no_mangle]
extern "C" fn stop_app(app_id: *mut c_char) {
    if app_id.is_null() {
        return;
    }
    let c_app_id = unsafe { CString::from_raw(app_id) };

    g_runtime.block_on(async move {
        let info = node_infomation.lock().await;
        if let Some(node_info) = info.as_ref() {
            let _ = set_node_config(
                node_info.node_host_name.as_str(),
                &node_info.sys_cfg_client,
                format!("apps/{:?}/target_state", c_app_id).as_str(),
                "Stopped",
            )
            .await;
        }
        let _ = c_app_id.into_raw();
    });
}
