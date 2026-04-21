#![allow(unused_must_use)]

use std::collections::HashMap;
use std::fs::File;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use ::kRPC::*;
use buckyos_kit::*;
use jsonwebtoken::{decode, DecodingKey, EncodingKey, Validation};
use log::*;
use ndn_lib::{load_named_object_from_obj_str, ChunkId, ObjId};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use std::net::{IpAddr, Ipv4Addr, ToSocketAddrs};
use tokio::sync::{OnceCell, RwLock};

use name_client::*;
use name_lib::*;
use named_store::NamedDataMgr;

use crate::aicc_client::*;
use crate::app_mgr::*;
use crate::control_panel::*;
use crate::msg_center_client::*;
use crate::msg_queue::*;
use crate::opendan_client::*;
use crate::repo_client::*;
use crate::scheduler_client::*;
use crate::system_config::*;
use crate::task_mgr::*;
use crate::verify_hub_client::*;
use crate::{
    get_buckyos_api_runtime, get_full_appid, get_session_token_env_key, OPENDAN_SERVICE_NAME,
};

const DEFAULT_NODE_GATEWAY_PORT: u16 = 3180;
const DEFAULT_AICC_KRPC_TIMEOUT_SECS: u64 = 120;
const DEFAULT_KRPC_TIMEOUT_SECS: u64 = 15;
const BUCKYOS_KRPC_TIMEOUT_SECS_ENV: &str = "BUCKYOS_KRPC_TIMEOUT_SECS";
const BUCKYOS_KRPC_TIMEOUT_SECS_PREFIX: &str = "BUCKYOS_KRPC_TIMEOUT_SECS_";
const BUCKYOS_HOST_GATEWAY_ENV: &str = "BUCKYOS_HOST_GATEWAY";
const DEFAULT_DOCKER_HOST_GATEWAY: &str = "host.docker.internal";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuckyOSRuntimeType {
    AppClient,     //运行在所有设备上，通常不在容器里（唯一可能加载user private key的类型)
    AppService,    //R3 运行在Node上，指定用户，可能在容器里
    FrameService,  //R2 运行在Node上，通常在容器里
    KernelService, //R1 由node-daemon启动的的系统基础服务
    Kernel,        //R0,node-daemon和cyfs-gateway使用，可以单独启动的组件
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeErrorClass {
    Retryable,
    NonRetryable,
    Degraded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeBackgroundTaskKind {
    RenewToken,
    UpdateServiceInstanceInfo,
    RefreshTrustKeys,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeBackgroundTaskStatus {
    pub task: RuntimeBackgroundTaskKind,
    #[serde(default = "background_task_enabled_default")]
    pub enabled: bool,
    pub error_class: Option<RuntimeErrorClass>,
    pub consecutive_failures: u32,
    pub last_error: Option<String>,
    pub last_failure_time: Option<u64>,
    pub last_success_time: Option<u64>,
    pub next_retry_time: Option<u64>,
    pub degraded_since: Option<u64>,
}

fn background_task_enabled_default() -> bool {
    true
}

impl RuntimeBackgroundTaskStatus {
    fn new(task: RuntimeBackgroundTaskKind) -> Self {
        Self {
            task,
            enabled: true,
            error_class: None,
            consecutive_failures: 0,
            last_error: None,
            last_failure_time: None,
            last_success_time: None,
            next_retry_time: None,
            degraded_since: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeHealthSnapshot {
    pub tasks: Vec<RuntimeBackgroundTaskStatus>,
}

#[derive(Debug, Clone, Copy)]
struct RetryPolicy {
    max_attempts: u32,
    base_delay_ms: u64,
    max_delay_ms: u64,
    jitter_ms: u64,
}

impl RetryPolicy {
    fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let exp = attempt.saturating_sub(1).min(16);
        let scale = 1u64 << exp;
        let delay_ms = self
            .base_delay_ms
            .saturating_mul(scale)
            .min(self.max_delay_ms);
        let jitter_ms = if self.jitter_ms == 0 {
            0
        } else {
            rand::thread_rng().gen_range(0..=self.jitter_ms)
        };
        Duration::from_millis(delay_ms.saturating_add(jitter_ms))
    }
}

#[derive(Debug, Clone, Copy)]
struct BackgroundTaskPolicy {
    degraded_after_failures: u32,
    base_retry_secs: u64,
    max_retry_secs: u64,
}

impl BackgroundTaskPolicy {
    fn next_retry_time(&self, consecutive_failures: u32, now: u64) -> u64 {
        let exp = consecutive_failures.saturating_sub(1).min(16);
        let scale = 1u64 << exp;
        let delay = self
            .base_retry_secs
            .saturating_mul(scale)
            .min(self.max_retry_secs);
        now.saturating_add(delay)
    }
}

pub struct BuckyOSRuntime {
    pub app_owner_id: Option<String>,
    pub app_id: String,
    pub app_host_perfix: String,
    pub runtime_type: BuckyOSRuntimeType,
    pub main_service_port: RwLock<u16>,

    pub user_id: Option<String>,
    pub authenticated_user_id: Option<String>,
    pub user_config: Option<OwnerConfig>,
    pub user_private_key: Option<EncodingKey>,

    pub device_config: Option<DeviceConfig>,
    pub device_private_key: Option<EncodingKey>,
    pub device_info: Option<DeviceInfo>,

    pub zone_id: DID,
    pub node_gateway_port: u16,

    //pub is_token_iss_by_self:bool,
    pub zone_config: Option<ZoneConfig>,
    pub session_token: Arc<RwLock<String>>,
    pub refresh_token: Arc<RwLock<String>>,
    trust_keys: Arc<RwLock<HashMap<String, DecodingKey>>>,
    last_update_service_info_time: RwLock<u64>,
    named_store_mgr: OnceCell<NamedDataMgr>,
    system_config_client: OnceCell<Arc<SystemConfigClient>>,
    background_task_status:
        Arc<RwLock<HashMap<RuntimeBackgroundTaskKind, RuntimeBackgroundTaskStatus>>>,

    pub force_https: bool,
    pub buckyos_root_dir: PathBuf,
    pub web3_bridges: HashMap<String, String>,
    registered_tasks_started: AtomicBool,
}

impl BuckyOSRuntime {
    pub fn new(
        app_id: &str,
        app_owner_user_id: Option<String>,
        runtime_type: BuckyOSRuntimeType,
    ) -> Self {
        let app_host_perfix;
        if app_owner_user_id.is_none() {
            app_host_perfix = format!("{}", app_id);
        } else {
            app_host_perfix = format!("{}-{}", app_id, app_owner_user_id.clone().unwrap());
        }
        let user_id = if matches!(
            runtime_type,
            BuckyOSRuntimeType::AppService | BuckyOSRuntimeType::AppClient
        ) {
            app_owner_user_id.clone()
        } else {
            None
        };

        let runtime = BuckyOSRuntime {
            app_id: app_id.to_string(),
            app_owner_id: app_owner_user_id,
            app_host_perfix: app_host_perfix,
            main_service_port: RwLock::new(0),
            user_id,
            authenticated_user_id: None,
            runtime_type,
            session_token: Arc::new(RwLock::new("".to_string())),
            refresh_token: Arc::new(RwLock::new("".to_string())),
            buckyos_root_dir: get_buckyos_root_dir(),
            zone_config: None,
            device_config: None,
            device_private_key: None,
            device_info: None,
            user_private_key: None,
            user_config: None,
            zone_id: DID::undefined(),
            node_gateway_port: DEFAULT_NODE_GATEWAY_PORT,
            trust_keys: Arc::new(RwLock::new(HashMap::new())),
            last_update_service_info_time: RwLock::new(0),
            named_store_mgr: OnceCell::new(),
            system_config_client: OnceCell::new(),
            background_task_status: Arc::new(RwLock::new(HashMap::from([
                (
                    RuntimeBackgroundTaskKind::RenewToken,
                    RuntimeBackgroundTaskStatus::new(RuntimeBackgroundTaskKind::RenewToken),
                ),
                (
                    RuntimeBackgroundTaskKind::UpdateServiceInstanceInfo,
                    RuntimeBackgroundTaskStatus::new(
                        RuntimeBackgroundTaskKind::UpdateServiceInstanceInfo,
                    ),
                ),
                (
                    RuntimeBackgroundTaskKind::RefreshTrustKeys,
                    RuntimeBackgroundTaskStatus::new(RuntimeBackgroundTaskKind::RefreshTrustKeys),
                ),
            ]))),
            web3_bridges: HashMap::new(),
            force_https: true,
            registered_tasks_started: AtomicBool::new(false),
        };
        runtime
    }

    pub async fn set_main_service_port(&self, port: u16) {
        let mut main_service_port = self.main_service_port.write().await;
        *main_service_port = port;
    }

    pub async fn get_named_store(&self) -> Result<NamedDataMgr> {
        let store_mgr: &NamedDataMgr = self
            .named_store_mgr
            .get_or_try_init(|| async {
                let config_path = get_buckyos_root_dir()
                    .join("storage")
                    .join("named_store.json");
                let http_backend_links = HashMap::new();
                NamedDataMgr::get_store_mgr(config_path.as_path(), &http_backend_links)
                    .await
                    .map_err(|e| {
                        RPCErrors::ReasonError(format!(
                            "Failed to initialize named store manager: {}",
                            e
                        ))
                    })
            })
            .await?;
        Ok(store_mgr.clone())
    }

    pub async fn fill_by_env_var(&mut self) -> Result<()> {
        // let zone_boot_config = env::var("BUCKYOS_ZONE_BOOT_CONFIG");
        // if zone_boot_config.is_ok() {
        //     let zone_boot_config:ZoneBootConfig = serde_json::from_str(zone_boot_config.unwrap().as_str())
        //         .map_err(|e| {
        //             error!("Failed to parse zone boot config: {}", e);
        //             RPCErrors::ReasonError(format!("Failed to parse zone boot config: {}", e))
        //         })?;
        //     if zone_boot_config.id.is_none() {
        //         return Err(RPCErrors::ReasonError("zone_boot_config id is not set".to_string()));
        //     }

        //     self.zone_id = zone_boot_config.id.clone().unwrap();
        // } e
        let zone_config_str = env::var("BUCKYOS_ZONE_CONFIG");
        if zone_config_str.is_ok() {
            let zone_config_str = zone_config_str.unwrap();
            debug!("zone_config_str:{}", zone_config_str);
            let zone_config = serde_json::from_str(zone_config_str.as_str());
            if zone_config.is_err() {
                warn!("zone_config_str format error");
                return Err(RPCErrors::ReasonError(
                    "zone_config_str format error".to_string(),
                ));
            }
            let zone_config: ZoneConfig = zone_config.unwrap();
            self.zone_id = zone_config.id.clone();
            self.zone_config = Some(zone_config);
        }

        let device_info_str = env::var("BUCKYOS_THIS_DEVICE_INFO");
        if device_info_str.is_ok() {
            let device_info_str = device_info_str.unwrap();
            let device_info = serde_json::from_str(device_info_str.as_str());
            if device_info.is_err() {
                warn!("device_info_str format error");
                return Err(RPCErrors::ReasonError(
                    "device_info_str format error".to_string(),
                ));
            }
            let device_info = device_info.unwrap();
            self.device_info = Some(device_info);
        }

        if let Ok(device_doc) = env::var("BUCKYOS_THIS_DEVICE") {
            info!("device_doc:{}", device_doc);
            let device_config = serde_json::from_str(device_doc.as_str());
            if device_config.is_err() {
                warn!("device_doc format error");
                return Err(RPCErrors::ReasonError(
                    "device_doc format error".to_string(),
                ));
            }
            let device_config: DeviceConfig = device_config.unwrap();
            self.device_config = Some(device_config.clone());
        }

        if self.runtime_type == BuckyOSRuntimeType::AppService && self.user_id.is_none() {
            self.user_id = self.app_owner_id.clone();
        }

        let mut session_token_keys = Vec::new();
        match self.runtime_type {
            BuckyOSRuntimeType::KernelService => {
                session_token_keys.push(get_session_token_env_key(&self.get_full_appid(), false));
            }
            BuckyOSRuntimeType::FrameService => {
                if let Some(owner_id) = self.app_owner_id.as_deref() {
                    session_token_keys.push(get_session_token_env_key(
                        &get_full_appid(self.app_id.as_str(), owner_id),
                        true,
                    ));
                }
                session_token_keys.push(get_session_token_env_key(&self.get_full_appid(), false));
            }
            BuckyOSRuntimeType::AppService => {
                if let Some(owner_id) = self.app_owner_id.as_deref() {
                    session_token_keys.push(get_session_token_env_key(
                        &get_full_appid(self.app_id.as_str(), owner_id),
                        true,
                    ));
                }
                session_token_keys.push(get_session_token_env_key(self.app_id.as_str(), true));
            }
            _ => {
                info!(
                    "will not load session_token from env var for runtime_type: {:?}",
                    self.runtime_type
                );
            }
        }

        session_token_keys.dedup();
        if !session_token_keys.is_empty() {
            let mut loaded_session_token = None;
            for session_token_key in &session_token_keys {
                if let Ok(session_token) = env::var(session_token_key.as_str()) {
                    info!(
                        "load session_token from env var success: {}",
                        session_token_key
                    );
                    loaded_session_token = Some(session_token);
                    break;
                }
            }

            if let Some(session_token) = loaded_session_token {
                let mut this_session_token = self.session_token.write().await;
                *this_session_token = session_token;
            } else {
                info!(
                    "load session_token from env var failed, tried keys: {:?}",
                    session_token_keys
                );
                return Err(RPCErrors::ReasonError(format!(
                    "load session_token from env var failed, tried keys: {:?}",
                    session_token_keys
                )));
            }
        }

        Ok(())
    }

    pub async fn fill_policy_by_load_config(&mut self) -> Result<()> {
        if self.runtime_type == BuckyOSRuntimeType::AppService {
            info!("AppService runtime skips system config root bootstrap");
            return Ok(());
        }

        let mut config_root_dir = None;
        if self.runtime_type == BuckyOSRuntimeType::AppClient {
            let bucky_dev_user_home_dir = get_buckyos_dev_user_home();
            if bucky_dev_user_home_dir.exists() {
                info!(
                    "dev folder exists: {}",
                    bucky_dev_user_home_dir.to_string_lossy()
                );
                config_root_dir = Some(bucky_dev_user_home_dir);
            } else {
                info!(
                    "dev folder {} not exists,try to use $BUCKYOS_ROOT/etc folder",
                    bucky_dev_user_home_dir.to_string_lossy()
                );
            }
        }

        if config_root_dir.is_none() {
            let etc_dir = get_buckyos_system_etc_dir();
            config_root_dir = Some(etc_dir);
        }

        if config_root_dir.is_none() {
            return Err(RPCErrors::ReasonError(
                "config_root_dir is not set".to_string(),
            ));
        }
        let config_root_dir = config_root_dir.unwrap();
        if !config_root_dir.exists() {
            error!(
                "config_root_dir not exists: {}, init buckyos runtime config would failed!",
                config_root_dir.to_string_lossy()
            );
            return Err(RPCErrors::ReasonError(
                "config_root_dir not exists".to_string(),
            ));
        }
        info!(
            "will use config_root_dir: {} to load buckyos runtime config",
            config_root_dir.to_string_lossy()
        );

        let machine_config_file = config_root_dir.join("machine.json");

        let mut machine_config = BuckyOSMachineConfig::default();
        if machine_config_file.exists() {
            let machine_config_file = File::open(machine_config_file);
            if machine_config_file.is_ok() {
                let machine_config_json = serde_json::from_reader(machine_config_file.unwrap());
                if machine_config_json.is_ok() {
                    machine_config = machine_config_json.unwrap();
                } else {
                    error!(
                        "Failed to parse machine_config: {}",
                        machine_config_json.err().unwrap()
                    );
                    return Err(RPCErrors::ReasonError(format!(
                        "Failed to parse machine_config "
                    )));
                }
            }
        }
        self.web3_bridges = machine_config.web3_bridge;
        self.force_https = machine_config.force_https;

        Ok(())
    }

    pub async fn fill_by_load_config(&mut self) -> Result<()> {
        let mut config_root_dir = None;
        if self.runtime_type == BuckyOSRuntimeType::AppClient {
            let bucky_dev_user_home_dir = get_buckyos_dev_user_home();
            if bucky_dev_user_home_dir.exists() {
                info!(
                    "dev folder exists: {}",
                    bucky_dev_user_home_dir.to_string_lossy()
                );
                config_root_dir = Some(bucky_dev_user_home_dir);
            } else {
                info!(
                    "dev folder {} not exists,try to use $BUCKYOS_ROOT/etc folder",
                    bucky_dev_user_home_dir.to_string_lossy()
                );
            }
        }

        if config_root_dir.is_none() {
            let etc_dir = get_buckyos_system_etc_dir();
            config_root_dir = Some(etc_dir);
        }

        if config_root_dir.is_none() {
            return Err(RPCErrors::ReasonError(
                "config_root_dir is not set".to_string(),
            ));
        }

        let config_root_dir = config_root_dir.unwrap();
        if !config_root_dir.exists() {
            error!(
                "config_root_dir not exists: {}, fill_by_load_config would failed!",
                config_root_dir.to_string_lossy()
            );
            return Err(RPCErrors::ReasonError(
                "config_root_dir not exists".to_string(),
            ));
        }
        info!(
            "will load config for buckyos-runtime from {} ",
            config_root_dir.to_string_lossy()
        );

        let node_identity_file = config_root_dir.join("node_identity.json");
        let device_private_key_file = config_root_dir.join("node_private_key.pem");

        let node_identity_config =
            NodeIdentityConfig::load_node_identity_config(&node_identity_file).map_err(|e| {
                error!("Failed to load node identity config: {}", e);
                RPCErrors::ReasonError(format!("Failed to load node identity config: {}", e))
            });

        if node_identity_config.is_ok() {
            let node_identity_config = node_identity_config.unwrap();
            let device_config =
                decode_jwt_claim_without_verify(node_identity_config.device_doc_jwt.as_str())
                    .map_err(|e| {
                        error!("Failed to decode device config: {}", e);
                        RPCErrors::ReasonError(format!("Failed to decode device config: {}", e))
                    })?;

            let devcie_config = serde_json::from_value::<DeviceConfig>(device_config);
            if devcie_config.is_err() {
                error!(
                    "Failed to parse device config: {}",
                    devcie_config.err().unwrap()
                );
                return Err(RPCErrors::ReasonError(format!(
                    "Failed to parse device config from jwt: {}",
                    node_identity_config.device_doc_jwt.as_str()
                )));
            }
            let device_config = devcie_config.unwrap();
            self.device_config = Some(device_config);
            let zone_did = node_identity_config.zone_did.clone();
            self.zone_id = zone_did.clone();
        } else {
            if self.runtime_type != BuckyOSRuntimeType::AppClient {
                return Err(RPCErrors::ReasonError(
                    "node_identity_config is not set".to_string(),
                ));
            }
        }

        if self.runtime_type == BuckyOSRuntimeType::AppClient
            || self.runtime_type == BuckyOSRuntimeType::Kernel
        {
            let private_key = load_private_key(&device_private_key_file);
            if private_key.is_ok() {
                self.device_private_key = Some(private_key.unwrap());
            } else {
                self.device_private_key = None;
            }
        }

        //let zone_config_file;
        if self.runtime_type == BuckyOSRuntimeType::AppClient {
            let user_config_file = config_root_dir.join("user_config.json");
            let user_private_key_file = config_root_dir.join("user_private_key.pem");
            if user_config_file.exists() {
                let owner_config =
                    OwnerConfig::load_owner_config(&user_config_file).map_err(|e| {
                        error!("Failed to load owner config: {}", e);
                        RPCErrors::ReasonError(format!("Failed to load owner config: {}", e))
                    })?;
                if self.user_id.is_none() {
                    self.user_id = Some(owner_config.name.clone());
                }
                if self.app_owner_id.is_none() {
                    self.app_owner_id = Some(owner_config.name.clone());
                }
                if !self.zone_id.is_valid() {
                    if owner_config.default_zone_did.is_some() {
                        let zone_did = owner_config.default_zone_did.clone().unwrap();
                        self.zone_id = zone_did.clone();
                    } else {
                        return Err(RPCErrors::ReasonError(
                            "default_zone_did is not set".to_string(),
                        ));
                    }
                }

                self.user_config = Some(owner_config);

                let private_key = load_private_key(&user_private_key_file);
                if private_key.is_ok() {
                    info!("!!!! Make sure your development machine is secured,user_private_key_file {} load success, ",user_private_key_file.to_string_lossy());
                    self.user_private_key = Some(private_key.unwrap());
                } else {
                    info!(
                        "user_private_key_file {} load failed!:{:?}",
                        user_private_key_file.to_string_lossy(),
                        private_key.err().unwrap()
                    );
                }
            }
        }

        Ok(())
    }

    pub fn is_service(&self) -> bool {
        self.runtime_type == BuckyOSRuntimeType::KernelService
            || self.runtime_type == BuckyOSRuntimeType::FrameService
            || self.runtime_type == BuckyOSRuntimeType::AppService
    }

    fn resolve_authenticated_user_id(session_token: &RPCSessionToken) -> Option<String> {
        if let Some(sub) = session_token
            .sub
            .clone()
            .filter(|value| !value.trim().is_empty())
        {
            return Some(sub);
        }

        let raw_jwt = session_token.token.as_deref()?;
        let claims = decode_jwt_claim_without_verify(raw_jwt).ok()?;
        claims
            .get("userid")
            .or_else(|| claims.get("sub"))
            .and_then(|value| value.as_str())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    pub fn get_authenticated_user_id(&self) -> Option<String> {
        self.authenticated_user_id.clone()
    }

    pub fn classify_error(error: &RPCErrors) -> RuntimeErrorClass {
        match error {
            RPCErrors::InvalidToken(_)
            | RPCErrors::ParseRequestError(_)
            | RPCErrors::ParserResponseError(_)
            | RPCErrors::NoPermission(_)
            | RPCErrors::InvalidPassword
            | RPCErrors::UserNotFound(_)
            | RPCErrors::KeyNotExist(_)
            | RPCErrors::UnknownMethod(_) => RuntimeErrorClass::NonRetryable,
            RPCErrors::TokenExpired(_) | RPCErrors::ServiceNotValid(_) => {
                RuntimeErrorClass::Retryable
            }
            RPCErrors::ReasonError(message) => {
                let message = message.to_ascii_lowercase();
                if [
                    "timeout",
                    "timed out",
                    "connection refused",
                    "connection reset",
                    "temporarily unavailable",
                    "503",
                    "502",
                    "504",
                    "leader",
                    "unreachable",
                    "broken pipe",
                    "eof",
                    "dns",
                ]
                .iter()
                .any(|pattern| message.contains(pattern))
                {
                    RuntimeErrorClass::Retryable
                } else if [
                    "notsupported",
                    "not supported",
                    "notimplemented",
                    "not implemented",
                    "invalid",
                    "parse",
                    "permission",
                    "forbidden",
                    "unauthorized",
                    "401",
                    "403",
                    "contract",
                    "schema",
                    "format",
                ]
                .iter()
                .any(|pattern| message.contains(pattern))
                {
                    RuntimeErrorClass::NonRetryable
                } else {
                    RuntimeErrorClass::Retryable
                }
            }
        }
    }

    pub fn is_retryable_error(error: &RPCErrors) -> bool {
        matches!(Self::classify_error(error), RuntimeErrorClass::Retryable)
    }

    pub async fn get_runtime_health_snapshot(&self) -> RuntimeHealthSnapshot {
        let tasks = self
            .background_task_status
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        RuntimeHealthSnapshot { tasks }
    }

    pub async fn set_background_task_enabled(
        &self,
        task: RuntimeBackgroundTaskKind,
        enabled: bool,
    ) {
        let mut status_map = self.background_task_status.write().await;
        let status = status_map
            .entry(task)
            .or_insert_with(|| RuntimeBackgroundTaskStatus::new(task));
        status.enabled = enabled;
        if enabled {
            status.next_retry_time = None;
        }
    }

    pub async fn set_all_background_tasks_enabled(&self, enabled: bool) {
        self.set_background_task_enabled(RuntimeBackgroundTaskKind::RenewToken, enabled)
            .await;
        self.set_background_task_enabled(
            RuntimeBackgroundTaskKind::UpdateServiceInstanceInfo,
            enabled,
        )
        .await;
        self.set_background_task_enabled(RuntimeBackgroundTaskKind::RefreshTrustKeys, enabled)
            .await;
    }

    fn login_retry_policy() -> RetryPolicy {
        RetryPolicy {
            max_attempts: 3,
            base_delay_ms: 250,
            max_delay_ms: 2_000,
            jitter_ms: 250,
        }
    }

    fn renew_token_retry_policy() -> RetryPolicy {
        RetryPolicy {
            max_attempts: 3,
            base_delay_ms: 500,
            max_delay_ms: 5_000,
            jitter_ms: 300,
        }
    }

    fn background_task_policy(task: RuntimeBackgroundTaskKind) -> BackgroundTaskPolicy {
        match task {
            RuntimeBackgroundTaskKind::RenewToken => BackgroundTaskPolicy {
                degraded_after_failures: 3,
                base_retry_secs: 5,
                max_retry_secs: 60,
            },
            RuntimeBackgroundTaskKind::UpdateServiceInstanceInfo => BackgroundTaskPolicy {
                degraded_after_failures: 3,
                base_retry_secs: 5,
                max_retry_secs: 60,
            },
            RuntimeBackgroundTaskKind::RefreshTrustKeys => BackgroundTaskPolicy {
                degraded_after_failures: 2,
                base_retry_secs: 10,
                max_retry_secs: 300,
            },
        }
    }

    fn unsupported_error(&self, capability: &str) -> RPCErrors {
        RPCErrors::ReasonError(format!(
            "NotSupported: runtime_type {:?} does not support {}",
            self.runtime_type, capability
        ))
    }

    fn not_implemented_error(operation: &str) -> RPCErrors {
        RPCErrors::ReasonError(format!("NotImplemented: {}", operation))
    }

    async fn should_run_background_task(&self, task: RuntimeBackgroundTaskKind, now: u64) -> bool {
        let status_map = self.background_task_status.read().await;
        status_map
            .get(&task)
            .map(|status| {
                status.enabled
                    && status
                        .next_retry_time
                        .map(|retry_at| retry_at <= now)
                        .unwrap_or(true)
            })
            .unwrap_or(true)
    }

    async fn record_background_task_success(&self, task: RuntimeBackgroundTaskKind, now: u64) {
        let mut status_map = self.background_task_status.write().await;
        let status = status_map
            .entry(task)
            .or_insert_with(|| RuntimeBackgroundTaskStatus::new(task));
        let had_failures = status.consecutive_failures > 0 || status.degraded_since.is_some();
        status.error_class = None;
        status.consecutive_failures = 0;
        status.last_error = None;
        status.last_success_time = Some(now);
        status.next_retry_time = None;
        status.degraded_since = None;
        if had_failures {
            info!("runtime background task {:?} recovered", task);
        }
    }

    async fn record_background_task_failure(
        &self,
        task: RuntimeBackgroundTaskKind,
        now: u64,
        error: &RPCErrors,
        forced_class: Option<RuntimeErrorClass>,
    ) {
        let policy = Self::background_task_policy(task);
        let mut status_map = self.background_task_status.write().await;
        let status = status_map
            .entry(task)
            .or_insert_with(|| RuntimeBackgroundTaskStatus::new(task));
        status.consecutive_failures = status.consecutive_failures.saturating_add(1);
        status.last_failure_time = Some(now);
        status.last_error = Some(error.to_string());
        let error_class = forced_class.unwrap_or_else(|| Self::classify_error(error));
        let should_degrade = matches!(error_class, RuntimeErrorClass::Degraded)
            || status.consecutive_failures >= policy.degraded_after_failures;
        status.error_class = Some(if should_degrade {
            RuntimeErrorClass::Degraded
        } else {
            error_class
        });
        status.next_retry_time = Some(policy.next_retry_time(status.consecutive_failures, now));
        if should_degrade && status.degraded_since.is_none() {
            status.degraded_since = Some(now);
        }

        let should_log = status.consecutive_failures == 1
            || should_degrade
            || status.consecutive_failures % 10 == 0;
        if should_log {
            warn!(
                "runtime background task {:?} failed: class={:?} failures={} next_retry={} err={}",
                task,
                status.error_class.unwrap_or(error_class),
                status.consecutive_failures,
                status.next_retry_time.unwrap_or(now),
                error
            );
        }
    }

    async fn execute_retryable_operation<F, Fut>(
        &self,
        operation_name: &str,
        policy: RetryPolicy,
        mut operation: F,
    ) -> Result<()>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<()>>,
    {
        let mut last_error = None;
        for attempt in 1..=policy.max_attempts {
            match operation().await {
                Ok(()) => return Ok(()),
                Err(error) => {
                    let retryable = Self::is_retryable_error(&error);
                    if !retryable || attempt == policy.max_attempts {
                        return Err(error);
                    }

                    let delay = policy.delay_for_attempt(attempt);
                    warn!(
                        "{} failed on attempt {}/{} with retryable error: {}. retry in {:?}",
                        operation_name, attempt, policy.max_attempts, error, delay
                    );
                    last_error = Some(error);
                    tokio::time::sleep(delay).await;
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            RPCErrors::ReasonError(format!(
                "{} failed without returning a concrete error",
                operation_name
            ))
        }))
    }

    pub async fn update_service_instance_info(&self) -> Result<()> {
        if !self.is_service() {
            return Ok(());
        }

        let now = buckyos_get_unix_timestamp();
        let last_update_service_info_time = self.last_update_service_info_time.read().await;
        if now - *last_update_service_info_time
            < crate::app_mgr::SERVICE_INSTANCE_INFO_UPDATE_INTERVAL
        {
            return Ok(());
        }
        drop(last_update_service_info_time);

        let device_config = self.device_config.as_ref().ok_or_else(|| {
            RPCErrors::ReasonError(
                "update_service_instance_info requires device_config for service runtime"
                    .to_string(),
            )
        })?;
        let node_did = device_config.id.clone();
        let node_id = device_config.name.clone();
        let instance_id = format!("{}-{}", self.app_id, device_config.name);
        let main_port = *self.main_service_port.read().await;
        let mut service_ports = HashMap::new();
        if main_port > 0 {
            service_ports.insert("www".to_string(), main_port);
        }
        let service_instance_info = ServiceInstanceReportInfo {
            instance_id,
            node_id: node_id.clone(),
            node_did,
            state: ServiceInstanceState::Started,
            service_ports: service_ports,
            last_update_time: buckyos_get_unix_timestamp(),
            start_time: 0,
            pid: std::process::id(),
        };

        let control_panel_client = self.get_control_panel_client().await?;
        control_panel_client
            .update_service_instance_info(&self.app_id, &node_id, &service_instance_info)
            .await?;
        let mut last_update_service_info_time = self.last_update_service_info_time.write().await;
        *last_update_service_info_time = now;
        info!("update service instance info,app_id:{}", self.app_id);
        Ok(())
    }

    pub async fn renew_token_from_verify_hub(&self) -> Result<()> {
        self.execute_retryable_operation(
            "renew_token_from_verify_hub",
            Self::renew_token_retry_policy(),
            || self.renew_token_from_verify_hub_once(),
        )
        .await
    }

    async fn renew_token_from_verify_hub_once(&self) -> Result<()> {
        let session_token = self.session_token.write().await;
        if session_token.is_empty() {
            debug!("session_token is empty,skip refresh token");
            return Ok(());
        }
        let session_token_str = session_token.clone();
        let real_session_token = RPCSessionToken::from_string(session_token_str.as_str())?;
        drop(session_token);
        if real_session_token.exp.is_none() {
            info!("session_token is none,skip refresh token");
            return Ok(());
        }

        let mut need_refresh = false;
        if let Some(iss) = real_session_token.iss.as_deref() {
            if iss != "verify-hub" {
                need_refresh = true;
            }
        }

        if !need_refresh {
            let expired_time = real_session_token.exp.unwrap();
            let now = buckyos_get_unix_timestamp();
            if now < expired_time.saturating_sub(30) {
                debug!("session_token is not expired,skip renew token");
                return Ok(());
            }
        }

        info!("session_token is close to expired,try to renew token");
        let verify_hub_client = self.get_verify_hub_client().await?;

        // Prefer refresh-token rotation when current session is issued by verify-hub.
        let token_pair = if real_session_token.iss.as_deref() == Some("verify-hub") {
            info!("use refresh-token to renew session-token from verify-hub...");
            let refresh_token = self.refresh_token.read().await;
            if !refresh_token.is_empty() {
                verify_hub_client
                    .refresh_token(refresh_token.as_str())
                    .await?
            } else {
                info!("refresh-token is empty, re-login by a locally generated device/user JWT...");
                // Fallback: if refresh token is missing, re-login by a locally generated device/user JWT.
                // This keeps the runtime functional but is less standard than refresh rotation.
                drop(refresh_token);
                let (fallback_jwt, _) = self.create_local_login_jwt().await?;
                verify_hub_client
                    .login_by_jwt(fallback_jwt.as_str(), None)
                    .await?
            }
        } else {
            // First login / exchange: accept trusted JWT (device/root/etc)
            info!("use session-token to renew session-token from verify-hub...");
            verify_hub_client
                .login_by_jwt(session_token_str.as_str(), None)
                .await?
        };

        info!("renew session-token success, token_pair: {:?}", token_pair);
        {
            let mut session_token = self.session_token.write().await;
            *session_token = token_pair.session_token.clone();
        }
        {
            let mut refresh_token = self.refresh_token.write().await;
            *refresh_token = token_pair.refresh_token.clone();
        }
        Ok(())
    }

    async fn create_local_login_jwt(&self) -> Result<(String, RPCSessionToken)> {
        if self.app_id.is_empty() {
            return Err(RPCErrors::ReasonError("App id is not set".to_string()));
        }

        if self.runtime_type == BuckyOSRuntimeType::AppClient {
            if self.user_private_key.is_some() && self.user_id.is_some() {
                let (jwt, token) = RPCSessionToken::generate_jwt_token(
                    self.user_id.as_ref().unwrap(),
                    self.app_id.as_str(),
                    None,
                    self.user_private_key.as_ref().unwrap(),
                )?;
                return Ok((jwt, token));
            }
        }

        if self.device_private_key.is_some() && self.device_config.is_some() {
            let device_private_key = self.device_private_key.as_ref().unwrap();
            let device_uid = self.device_config.as_ref().unwrap().name.clone();
            let (jwt, token) = RPCSessionToken::generate_jwt_token(
                device_uid.as_str(),
                self.app_id.as_str(),
                None,
                device_private_key,
            )?;
            return Ok((jwt, token));
        }

        Err(RPCErrors::ReasonError(
            "Missing local private key for login".to_string(),
        ))
    }

    async fn keep_alive() {
        let buckyos_api_runtime = match get_buckyos_api_runtime() {
            Ok(runtime) => runtime,
            Err(error) => {
                warn!(
                    "buckyos-api-runtime is not initialized, skip keep_alive tick: {}",
                    error
                );
                return;
            }
        };
        let now = buckyos_get_unix_timestamp();

        if buckyos_api_runtime
            .should_run_background_task(RuntimeBackgroundTaskKind::RenewToken, now)
            .await
        {
            match buckyos_api_runtime.renew_token_from_verify_hub().await {
                Ok(()) => {
                    buckyos_api_runtime
                        .record_background_task_success(RuntimeBackgroundTaskKind::RenewToken, now)
                        .await;
                }
                Err(error) => {
                    buckyos_api_runtime
                        .record_background_task_failure(
                            RuntimeBackgroundTaskKind::RenewToken,
                            now,
                            &error,
                            None,
                        )
                        .await;
                }
            }
        }

        if buckyos_api_runtime
            .should_run_background_task(RuntimeBackgroundTaskKind::UpdateServiceInstanceInfo, now)
            .await
        {
            match buckyos_api_runtime.update_service_instance_info().await {
                Ok(()) => {
                    buckyos_api_runtime
                        .record_background_task_success(
                            RuntimeBackgroundTaskKind::UpdateServiceInstanceInfo,
                            now,
                        )
                        .await;
                }
                Err(error) => {
                    buckyos_api_runtime
                        .record_background_task_failure(
                            RuntimeBackgroundTaskKind::UpdateServiceInstanceInfo,
                            now,
                            &error,
                            Some(RuntimeErrorClass::Degraded),
                        )
                        .await;
                }
            }
        }

        if buckyos_api_runtime.is_service()
            && buckyos_api_runtime
                .should_run_background_task(RuntimeBackgroundTaskKind::RefreshTrustKeys, now)
                .await
        {
            // RBAC is initialized at login(). Avoid high-frequency remote RBAC reload here;
            // keepalive should prioritize stability and token/service liveness.
            match buckyos_api_runtime.refresh_trust_keys().await {
                Ok(()) => {
                    buckyos_api_runtime
                        .record_background_task_success(
                            RuntimeBackgroundTaskKind::RefreshTrustKeys,
                            now,
                        )
                        .await;
                }
                Err(error) => {
                    buckyos_api_runtime
                        .record_background_task_failure(
                            RuntimeBackgroundTaskKind::RefreshTrustKeys,
                            now,
                            &error,
                            Some(RuntimeErrorClass::Degraded),
                        )
                        .await;
                }
            }
        }
    }

    pub fn start_registered_tasks_if_needed(&'static self) {
        if !self.zone_id.is_valid() {
            info!("skip starting registered runtime tasks because zone_id is invalid");
            return;
        }

        if self.registered_tasks_started.swap(true, Ordering::SeqCst) {
            debug!("registered runtime tasks already started");
            return;
        }

        tokio::task::spawn(async move {
            let start = tokio::time::Instant::now() + Duration::from_secs(5);
            let mut timer = tokio::time::interval_at(start, Duration::from_secs(5));
            loop {
                timer.tick().await;
                BuckyOSRuntime::keep_alive().await;
            }
        });
    }

    pub async fn login(&mut self) -> Result<()> {
        let policy = Self::login_retry_policy();
        let mut last_error = None;
        for attempt in 1..=policy.max_attempts {
            match self.login_once().await {
                Ok(()) => return Ok(()),
                Err(error) => {
                    if !Self::is_retryable_error(&error) || attempt == policy.max_attempts {
                        return Err(error);
                    }
                    let delay = policy.delay_for_attempt(attempt);
                    warn!(
                        "login failed on attempt {}/{} with retryable error: {}. retry in {:?}",
                        attempt, policy.max_attempts, error, delay
                    );
                    last_error = Some(error);
                    tokio::time::sleep(delay).await;
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            RPCErrors::ReasonError("login failed without concrete error".to_string())
        }))
    }

    async fn login_once(&mut self) -> Result<()> {
        if !self.zone_id.is_valid() {
            return Err(RPCErrors::ReasonError(
                "Zone id is not valid,api-runtime.login failed".to_string(),
            ));
        }

        let authenticated_session_token;
        if self.app_id.is_empty() {
            return Err(RPCErrors::ReasonError("App id is not set".to_string()));
        }

        if self.runtime_type == BuckyOSRuntimeType::FrameService
            || self.runtime_type == BuckyOSRuntimeType::KernelService
            || self.runtime_type == BuckyOSRuntimeType::Kernel
        {
            if self.device_config.is_none() {
                return Err(RPCErrors::ReasonError(
                    "Device config is not set!".to_string(),
                ));
            }
        }

        init_name_lib(&self.web3_bridges).await;

        authenticated_session_token = {
            let mut session_token = self.session_token.write().await;
            if session_token.is_empty() {
                let mut generated_session_token: Option<RPCSessionToken> = None;
                //info!("api-runtime: session token is empty,runtime_type:{:?},try to create session token by known private key",self.runtime_type);
                if self.runtime_type == BuckyOSRuntimeType::AppClient {
                    if self.user_private_key.is_some() && self.user_config.is_some() {
                        info!("api-runtime: session token is empty,runtime_type:{:?},try to create session token by user_private_key",self.runtime_type);
                        let (session_token_str, real_session_token) =
                            RPCSessionToken::generate_jwt_token(
                                self.user_id.as_ref().unwrap(),
                                self.app_id.as_str(),
                                None,
                                self.user_private_key.as_ref().unwrap(),
                            )?;
                        *session_token = session_token_str;
                        generated_session_token = Some(real_session_token);
                    }
                }

                if generated_session_token.is_none()
                    && self.device_private_key.is_some()
                    && self.device_config.is_some()
                    && session_token.is_empty()
                {
                    info!("buckyos-api-runtime: session token is empty,runtime_type:{:?},try to create session token by device_private_key",self.runtime_type);
                    let device_name = &self.device_config.as_ref().unwrap().name;
                    let (session_token_str, real_session_token) =
                        RPCSessionToken::generate_jwt_token(
                            device_name.as_str(),
                            self.app_id.as_str(),
                            None,
                            self.device_private_key.as_ref().unwrap(),
                        )?;
                    *session_token = session_token_str;
                    generated_session_token = Some(real_session_token);
                }

                if session_token.is_empty() {
                    return Err(RPCErrors::ReasonError(
                        "session_token is empty!".to_string(),
                    ));
                }
                drop(session_token);
                generated_session_token.ok_or(RPCErrors::ReasonError(
                    "session_token generated but parsed token is missing".to_string(),
                ))?
            } else {
                info!(
                    "buckyos-api-runtime: session token is set,runtime_type:{:?}",
                    self.runtime_type
                );
                let authenticated_session_token =
                    RPCSessionToken::from_string(session_token.as_str())?;
                drop(session_token);

                // info!("real_session_token: {:?}", authenticated_session_token);
                let appid = authenticated_session_token
                    .appid
                    .clone()
                    .unwrap_or("kernel".to_string());
                if appid != self.app_id {
                    warn!(
                        "Session token is not valid,aud(appid):{} != self.app_id:{}",
                        appid, self.app_id
                    );
                    return Err(RPCErrors::ReasonError(
                        "Session token is not valid".to_string(),
                    ));
                }
                authenticated_session_token
            }
        };

        info!("all config is checked,try to connect to control-panel and get zone config");
        //session token already set, try to connect to control-panel and get zone config
        let control_panel_client = self.get_control_panel_client().await?;
        let zone_config = control_panel_client.load_zone_config().await?;
        self.zone_config = Some(zone_config);
        self.authenticated_user_id =
            BuckyOSRuntime::resolve_authenticated_user_id(&authenticated_session_token);
        info!("get zone config OK ,api-runtime: login success");

        if self.runtime_type == BuckyOSRuntimeType::KernelService
            || self.runtime_type == BuckyOSRuntimeType::FrameService
        {
            let (rbac_model, rbac_policy) = control_panel_client.load_rbac_config().await?;
            rbac::create_enforcer(Some(rbac_model.as_str()), Some(rbac_policy.as_str()))
                .await
                .map_err(|error| {
                    RPCErrors::ReasonError(format!("init rbac enforcer failed: {}", error))
                })?;
            match self.refresh_trust_keys().await {
                Ok(()) => {
                    self.record_background_task_success(
                        RuntimeBackgroundTaskKind::RefreshTrustKeys,
                        buckyos_get_unix_timestamp(),
                    )
                    .await;
                    info!("refresh trust keys OK");
                }
                Err(error) => {
                    self.record_background_task_failure(
                        RuntimeBackgroundTaskKind::RefreshTrustKeys,
                        buckyos_get_unix_timestamp(),
                        &error,
                        Some(RuntimeErrorClass::Degraded),
                    )
                    .await;
                    warn!(
                        "refresh trust keys failed during login, runtime enters degraded mode: {}",
                        error
                    );
                }
            }
        }

        Ok(())
    }

    // pub async fn generate_session_token(&self) -> Result<String> {
    //     if self.session_token.read().await.is_empty() {
    //         if self.user_private_key.is_some() {
    //             let session_token = RPCSessionToken::generate_jwt_token(
    //                 self.user_id.as_ref().unwrap(),
    //                 self.app_id.as_str(),
    //                 self.user_id.clone(),
    //                 self.user_private_key.as_ref().unwrap()
    //             )?;
    //             let mut session_token_guard = self.session_token.write().await;
    //             *session_token_guard = session_token.clone();
    //             return Ok(session_token);
    //         } else if self.device_private_key.is_some() {
    //             if self.deivce_config.is_none() {
    //                 return Err(RPCErrors::ReasonError("Device config not set".to_string()));
    //             }
    //             if self.device_private_key.is_none() {
    //                 return Err(RPCErrors::ReasonError("Device private key not set".to_string()));
    //             }
    //             let device_uid= self.deivce_config.as_ref().unwrap().name.clone();
    //             let session_token = RPCSessionToken::generate_jwt_token(
    //                 device_uid.as_str(),
    //                 self.app_id.as_str(),
    //                 Some(device_uid.clone()),
    //                 self.device_private_key.as_ref().unwrap()
    //             )?;
    //             let mut session_token_guard = self.session_token.write().await;
    //             *session_token_guard = session_token.clone();
    //             return Ok(session_token);
    //         } else {
    //             return Err(RPCErrors::ReasonError("No private key found".to_string()));
    //         }
    //     } else {
    //         return Ok(self.session_token.read().await.clone());
    //     }
    // }

    pub async fn remove_trust_key(&self, kid: &str) -> Result<()> {
        let mut key_map = self.trust_keys.write().await;
        let remove_result = key_map.remove(kid);
        if remove_result.is_none() {
            return Err(RPCErrors::ReasonError(format!("kid {} not found", kid)));
        }
        Ok(())
    }

    async fn refresh_trust_keys(&self) -> Result<()> {
        if self.device_config.is_some() {
            let device_config = self.device_config.as_ref().unwrap();
            let device_key = device_config.get_auth_key(None);
            if device_key.is_some() {
                let kid = device_config.get_id().to_string();
                let key = device_key.as_ref().unwrap().0.clone();
                self.set_trust_key(kid.as_str(), &key).await;
                debug!("set trust key - device_config.did: {}", kid);
                let kid = device_config.name.clone();
                self.set_trust_key(kid.as_str(), &key).await;
                debug!("set trust key - device_config.name: {}", kid);
            }
        }

        //zone_config 中包含trust_keys
        if self.zone_config.is_some() {
            let zone_config = self.zone_config.as_ref().unwrap();

            if zone_config.verify_hub_info.is_some() {
                let verify_hub_info = zone_config.verify_hub_info.as_ref().unwrap();
                let kid = "verify-hub".to_string();
                let key = DecodingKey::from_jwk(&verify_hub_info.public_key);
                if key.is_ok() {
                    self.set_trust_key(kid.as_str(), &key.unwrap()).await;
                    debug!("set trust key - verify-hub");
                }
            } else {
                warn!("NO verfiy-hub publick key, system init with errors!");
            }

            if zone_config.owner.is_valid() {
                let owner_did = zone_config.owner.clone();
                if let Some(owner_key) = zone_config.get_default_key() {
                    let owner_public_key = DecodingKey::from_jwk(&owner_key).map_err(|err| {
                        error!("Failed to parse owner_public_key from zone_config: {}", err);
                        RPCErrors::ReasonError(err.to_string())
                    })?;
                    let _ = self.set_trust_key("root", &owner_public_key).await;
                    debug!("set trust key - root");
                    let _ = self
                        .set_trust_key(owner_did.to_string().as_str(), &owner_public_key)
                        .await;
                    let _ = self
                        .set_trust_key(owner_did.id.clone().as_str(), &owner_public_key)
                        .await;
                    let _ = self.set_trust_key("$default", &owner_public_key).await;
                    debug!(
                        "update owner_public_key [{}],[{}] to trust keys",
                        owner_did.to_string(),
                        owner_did.id
                    );
                }
            }
        }

        Ok(())
    }

    pub async fn set_trust_key(&self, kid: &str, key: &DecodingKey) -> Result<()> {
        let mut key_map = self.trust_keys.write().await;
        key_map.insert(kid.to_string(), key.clone());
        Ok(())
    }

    pub async fn verify_trusted_session_token(&self, token_str: &str) -> Result<RPCSessionToken> {
        let mut rpc_token = RPCSessionToken::from_string(token_str).map_err(|error| {
            RPCErrors::InvalidToken(format!("Invalid session token: {}", error))
        })?;
        if !rpc_token.is_self_verify() {
            return Err(RPCErrors::InvalidToken(
                "Session token is not valid".to_string(),
            ));
        }

        let raw_jwt = rpc_token.token.as_deref().ok_or(RPCErrors::InvalidToken(
            "Session token is missing raw JWT".to_string(),
        ))?;
        let header = jsonwebtoken::decode_header(raw_jwt).map_err(|error| {
            RPCErrors::InvalidToken(format!("JWT decode header error: {}", error))
        })?;
        let claims = decode_jwt_claim_without_verify(raw_jwt).ok();

        let mut kid = header
            .kid
            .clone()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                claims.as_ref().and_then(|value| {
                    value
                        .get("iss")
                        .and_then(|iss| iss.as_str())
                        .and_then(|iss| {
                            let trimmed = iss.trim();
                            if trimmed.is_empty() {
                                None
                            } else {
                                Some(trimmed.to_string())
                            }
                        })
                })
            })
            .unwrap_or_else(|| "root".to_string());

        let mut decoding_key = {
            let key_map = self.trust_keys.read().await;
            key_map.get(&kid).cloned()
        };

        if decoding_key.is_none() {
            self.refresh_trust_keys().await?;
            decoding_key = {
                let key_map = self.trust_keys.read().await;
                key_map.get(&kid).cloned()
            };
        }

        if decoding_key.is_none() && kid != "root" {
            kid = "root".to_string();
            decoding_key = {
                let key_map = self.trust_keys.read().await;
                key_map.get(&kid).cloned()
            };
        }

        let decoding_key =
            decoding_key.ok_or(RPCErrors::NoPermission(format!("kid {} not found", kid)))?;

        rpc_token
            .verify_by_key(&decoding_key)
            .map_err(|error| RPCErrors::InvalidToken(format!("JWT decode error: {}", error)))?;

        Ok(rpc_token)
    }

    //success return (userid,appid)
    pub async fn enforce(
        &self,
        req: &RPCRequest,
        action: &str,
        resource_path: &str,
    ) -> Result<(String, String)> {
        let token_str = req.token.as_deref().ok_or(RPCErrors::ParseRequestError(
            "Invalid params, session_token is none".to_string(),
        ))?;
        let token = RPCSessionToken::from_string(token_str).map_err(|error| {
            RPCErrors::InvalidToken(format!("Invalid session token: {}", error))
        })?;
        if !token.is_self_verify() {
            return Err(RPCErrors::InvalidToken(
                "Session token is not valid".to_string(),
            ));
        }
        let token_str = token.token.as_deref().ok_or(RPCErrors::InvalidToken(
            "Session token is missing raw JWT".to_string(),
        ))?;
        let header: jsonwebtoken::Header =
            jsonwebtoken::decode_header(token_str).map_err(|error| {
                RPCErrors::InvalidToken(format!("JWT decode header error : {}", error))
            })?;

        let key_map = self.trust_keys.read().await;
        let kid = header.kid.unwrap_or("root".to_string());
        let decoding_key = key_map.get(&kid);
        if decoding_key.is_none() {
            warn!("kid {} not found,Session token is not valid", kid.as_str());
            return Err(RPCErrors::NoPermission(format!(
                "kid {} not found",
                kid.as_str()
            )));
        }
        let decoding_key = decoding_key.unwrap();

        let validation = Validation::new(header.alg);
        //exp always checked
        let decoded_token = decode::<serde_json::Value>(token_str, &decoding_key, &validation)
            .map_err(|error| RPCErrors::InvalidToken(format!("JWT decode error:{}", error)))?;
        let decoded_json = decoded_token
            .claims
            .as_object()
            .ok_or(RPCErrors::InvalidToken("Invalid token".to_string()))?;

        let userid = decoded_json
            .get("userid")
            .or_else(|| decoded_json.get("sub"))
            .ok_or(RPCErrors::InvalidToken("Missing userid".to_string()))?;
        let userid = userid
            .as_str()
            .ok_or(RPCErrors::InvalidToken("Invalid userid".to_string()))?;
        let appid = decoded_json
            .get("appid")
            .and_then(|appid| appid.as_str())
            .or_else(|| decoded_json.get("aud").and_then(|aud| aud.as_str()))
            .unwrap_or("kernel");

        let system_config_client = self.get_system_config_client().await?;
        let rbac_policy = system_config_client.get("system/rbac/policy").await;
        if rbac_policy.is_ok() {
            let rbac_policy = rbac_policy.unwrap();
            if rbac_policy.is_changed {
                rbac::update_enforcer(Some(rbac_policy.value.as_str())).await;
            }
        }

        let result = rbac::enforce(userid, Some(appid), resource_path, action).await;
        if !result {
            return Err(RPCErrors::NoPermission(format!(
                "enforce failed,userid:{},appid:{},resource:{},action:{}",
                userid, appid, resource_path, action
            )));
        }
        Ok((userid.to_string(), appid.to_string()))
    }

    //     pub async fn enable_zone_provider (_is_gateway: bool) -> Result<()> {
    //         let client = GLOBAL_NAME_CLIENT.get();
    //         if client.is_none() {
    //             let client = NameClient::new(NameClientConfig::default());
    //             client.add_provider(Box::new(ZONE_PROVIDER.clone()),None).await;
    //             let set_result = GLOBAL_NAME_CLIENT.set(client);
    //             if set_result.is_err() {
    //                 error!("Failed to set GLOBAL_NAME_CLIENT");
    //             }
    //         } else {
    //             let client = client.unwrap();
    //             client.add_provider(Box::new(ZONE_PROVIDER.clone()),None).await;
    //         }

    //         Ok(())
    //    }

    pub fn get_app_id(&self) -> String {
        self.app_id.clone()
    }

    pub fn get_owner_user_id(&self) -> Option<String> {
        self.app_owner_id.clone()
    }

    pub fn get_zone_config(&self) -> Option<&ZoneConfig> {
        self.zone_config.as_ref()
    }

    //use https://full_appid.zonehost/ to access the app
    pub fn get_full_appid(&self) -> String {
        if self.runtime_type == BuckyOSRuntimeType::AppClient {
            let root_id = "root".to_string();
            let owner_id = self.app_owner_id.as_ref().unwrap_or(&root_id);
            return get_full_appid(self.app_id.as_str(), owner_id);
        } else {
            return self.app_id.clone();
        }
    }

    pub async fn get_session_token(&self) -> String {
        let session_token = self.session_token.read().await;
        let session_token_str = session_token.clone();
        drop(session_token);

        let session_token = match RPCSessionToken::from_string(&session_token_str) {
            Ok(token) => token,
            Err(error) => {
                warn!(
                    "session token parse failed, fallback to raw token string: {}",
                    error
                );
                return session_token_str;
            }
        };
        if session_token.exp.is_some() {
            let exp = session_token.exp.unwrap();
            let now = buckyos_get_unix_timestamp();
            if now < exp.saturating_sub(10) {
                return session_token_str;
            } else {
                if let (Some(device_private_key), Some(device_config)) = (
                    self.device_private_key.as_ref(),
                    self.device_config.as_ref(),
                ) {
                    let device_uid = device_config.name.clone();
                    let jwt_result = RPCSessionToken::generate_jwt_token(
                        device_uid.as_str(),
                        self.app_id.as_str(),
                        None,
                        device_private_key,
                    )
                    .map_err(|e| {
                        error!("generate session token failed! {}", e);
                    });
                    if jwt_result.is_ok() {
                        let (new_session_token_str, _new_session_token) = jwt_result.unwrap();
                        let mut session_token_guard = self.session_token.write().await;
                        *session_token_guard = new_session_token_str.clone();
                        return new_session_token_str;
                    }
                }
                warn!("session-token expired!");
                return "".to_string();
            }
        }
        warn!("use a session-token without expiration, it will be a BUG?");
        return session_token_str;
    }

    pub fn get_data_folder(&self) -> Result<PathBuf> {
        match self.runtime_type {
            BuckyOSRuntimeType::AppClient => Err(self.unsupported_error("get_data_folder")),
            BuckyOSRuntimeType::AppService => Ok(self
                .buckyos_root_dir
                .join("data")
                .join(self.user_id.clone().ok_or_else(|| {
                    RPCErrors::ReasonError(
                        "AppService get_data_folder requires resolved user_id".to_string(),
                    )
                })?)
                .join(self.app_id.clone())),
            BuckyOSRuntimeType::FrameService
            | BuckyOSRuntimeType::KernelService
            | BuckyOSRuntimeType::Kernel => {
                Ok(self.buckyos_root_dir.join("data").join(self.app_id.clone()))
            }
        }
    }

    pub fn get_cache_folder(&self) -> Result<PathBuf> {
        match self.runtime_type {
            BuckyOSRuntimeType::AppClient => Err(self.unsupported_error("get_cache_folder")),
            BuckyOSRuntimeType::AppService => Ok(self
                .buckyos_root_dir
                .join("cache")
                .join(self.user_id.clone().ok_or_else(|| {
                    RPCErrors::ReasonError(
                        "AppService get_cache_folder requires resolved user_id".to_string(),
                    )
                })?)
                .join(self.app_id.clone())),
            BuckyOSRuntimeType::FrameService
            | BuckyOSRuntimeType::KernelService
            | BuckyOSRuntimeType::Kernel => Ok(self
                .buckyos_root_dir
                .join("cache")
                .join(self.app_id.clone())),
        }
    }

    pub fn get_local_cache_folder(&self) -> Result<PathBuf> {
        match self.runtime_type {
            BuckyOSRuntimeType::AppClient => Err(self.unsupported_error("get_local_cache_folder")),
            BuckyOSRuntimeType::AppService => Ok(self
                .buckyos_root_dir
                .join("tmp")
                .join(self.user_id.clone().ok_or_else(|| {
                    RPCErrors::ReasonError(
                        "AppService get_local_cache_folder requires resolved user_id".to_string(),
                    )
                })?)
                .join(self.app_id.clone())),
            BuckyOSRuntimeType::FrameService
            | BuckyOSRuntimeType::KernelService
            | BuckyOSRuntimeType::Kernel => {
                Ok(self.buckyos_root_dir.join("tmp").join(self.app_id.clone()))
            }
        }
    }

    // 获得与物理逻辑磁盘绑定的本地存储目录，存储的可靠性和特性由物理磁盘决定
    //目录原理上是  disk_id/service_instance_id/
    pub fn get_lcoal_storage_folder(&self, disk_id: Option<String>) -> Result<PathBuf> {
        if self.runtime_type == BuckyOSRuntimeType::KernelService
            || self.runtime_type == BuckyOSRuntimeType::FrameService
        {
            if disk_id.is_some() {
                let disk_id = disk_id.unwrap();
                return Ok(self
                    .buckyos_root_dir
                    .join("local")
                    .join(disk_id)
                    .join(self.app_id.clone()));
            } else {
                return Ok(self
                    .buckyos_root_dir
                    .join("local")
                    .join(self.app_id.clone()));
            }
        } else {
            Err(self.unsupported_error("get_lcoal_storage_folder"))
        }
    }

    pub fn get_root_pkg_env_path() -> PathBuf {
        get_buckyos_service_local_data_dir("node_daemon").join("root_pkg_env")
    }

    fn get_my_settings_path(&self) -> Result<String> {
        match self.runtime_type {
            BuckyOSRuntimeType::AppClient => Err(self.unsupported_error("get_my_settings_path")),
            BuckyOSRuntimeType::AppService => Ok(format!(
                "users/{}/apps/{}/settings",
                self.user_id.as_ref().ok_or_else(|| {
                    RPCErrors::ReasonError(
                        "AppService get_my_settings_path requires resolved user_id".to_string(),
                    )
                })?,
                self.app_id.as_str()
            )),
            BuckyOSRuntimeType::FrameService
            | BuckyOSRuntimeType::KernelService
            | BuckyOSRuntimeType::Kernel => {
                Ok(format!("services/{}/settings", self.app_id.as_str()))
            }
        }
    }

    pub async fn get_my_settings(&self) -> Result<serde_json::Value> {
        let system_config_client = self.get_system_config_client().await?;
        let settiing_path = self.get_my_settings_path()?;
        let result_value = system_config_client
            .get(settiing_path.as_str())
            .await
            .map_err(|e| {
                error!("get settings failed! err:{}", e);
                RPCErrors::ReasonError(format!("get settings failed! err:{}", e))
            })?;
        let settings: serde_json::Value = serde_json::from_str(result_value.value.as_str())
            .map_err(|e| {
                error!("parse settings failed! err:{}", e);
                RPCErrors::ReasonError(format!("parse settings failed! err:{}", e))
            })?;
        Ok(settings)
    }

    pub async fn update_my_settings(
        &self,
        json_path: &str,
        settings: serde_json::Value,
    ) -> Result<()> {
        let system_config_client = self.get_system_config_client().await?;
        let settiing_path = self.get_my_settings_path()?;
        let settings_str = serde_json::to_string(&settings).map_err(|e| {
            error!("serialize settings failed! err:{}", e);
            RPCErrors::ReasonError(format!("serialize settings failed! err:{}", e))
        })?;

        system_config_client
            .set_by_json_path(settiing_path.as_str(), json_path, settings_str.as_str())
            .await
            .map_err(|e| {
                error!("update settings failed! err:{}", e);
                RPCErrors::ReasonError(format!("update settings failed! err:{}", e))
            })?;

        Ok(())
    }

    pub async fn update_all_my_settings(&self, settings: serde_json::Value) -> Result<()> {
        let system_config_client = self.get_system_config_client().await?;
        let settiing_path = self.get_my_settings_path()?;
        let settings_str = serde_json::to_string(&settings).map_err(|e| {
            error!("serialize settings failed! err:{}", e);
            RPCErrors::ReasonError(format!("serialize settings failed! err:{}", e))
        })?;
        system_config_client
            .set(settiing_path.as_str(), settings_str.as_str())
            .await
            .map_err(|e| {
                error!("update settings failed! err:{}", e);
                RPCErrors::ReasonError(format!("update settings failed! err:{}", e))
            })?;
        Ok(())
    }

    pub fn get_my_sys_config_path(&self, config_name: &str) -> String {
        match self.runtime_type {
            BuckyOSRuntimeType::AppClient | BuckyOSRuntimeType::AppService => format!(
                "users/{}/apps/{}/{}",
                self.user_id.as_deref().unwrap_or(""),
                self.app_id.as_str(),
                config_name
            ),
            BuckyOSRuntimeType::FrameService
            | BuckyOSRuntimeType::KernelService
            | BuckyOSRuntimeType::Kernel => {
                format!("services/{}/{}", self.app_id.as_str(), config_name)
            }
        }
    }

    pub fn is_ood(&self) -> bool {
        if self.device_config.is_some() {
            let device_config = self.device_config.as_ref().unwrap();
            if device_config.device_type == "ood" {
                return true;
            }
        }

        return false;
    }

    pub async fn get_system_control_panel_client(&self) -> Result<ControlPanelClient> {
        let system_config_client = self.get_system_config_client().await?;
        let client = ControlPanelClient::from_shared(system_config_client);
        Ok(client)
    }

    fn resolve_local_service_host(&self) -> String {
        match self.runtime_type {
            BuckyOSRuntimeType::AppService | BuckyOSRuntimeType::FrameService => {
                let configured_host = env::var(BUCKYOS_HOST_GATEWAY_ENV).ok();
                resolve_container_gateway_host(configured_host.as_deref())
            }
            _ => "127.0.0.1".to_string(),
        }
    }

    /// Compute the URL used to reach the zone's `system_config` service for
    /// the current runtime type. Exposed so that callers (e.g. control_panel
    /// handlers that want to forward the caller's RPC session token) can
    /// construct a fresh `SystemConfigClient` with their own auth token.
    pub fn get_system_config_url(&self) -> String {
        let mut url = format!(
            "http://{}:3200/kapi/system_config",
            self.resolve_local_service_host()
        );
        let mut schema = "http";
        if self.force_https {
            schema = "https";
        }
        let zone_host = self.zone_id.to_host_name();
        match self.runtime_type {
            BuckyOSRuntimeType::AppClient => {
                if !self.is_ood() {
                    url = format!("{}://{}/kapi/system_config", schema, zone_host);
                }
            }
            BuckyOSRuntimeType::AppService | BuckyOSRuntimeType::FrameService => {
                url = format!(
                    "http://{}:{}/kapi/system_config",
                    self.resolve_local_service_host(),
                    DEFAULT_NODE_GATEWAY_PORT
                );
            }
            _ => {
                // keep local direct system_config url
            }
        }
        url
    }

    pub async fn get_system_config_client(&self) -> Result<Arc<SystemConfigClient>> {
        let url = self.get_system_config_url();

        //let url = self.get_zone_service_url("system_config",self.force_https)?;
        let session_token = self.get_session_token().await;
        let client = self
            .system_config_client
            .get_or_try_init(|| async {
                Ok(Arc::new(SystemConfigClient::new(
                    Some(url.as_str()),
                    Some(session_token.as_str()),
                )))
            })
            .await?;
        client
            .sync_session_token(Some(session_token.as_str()))
            .await
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        debug!("get system config client OK,url:{}", url);
        Ok(client.clone())
    }

    pub async fn get_task_mgr_client(&self) -> Result<TaskManagerClient> {
        let krpc_client = self.get_zone_service_krpc_client("task-manager").await?;
        let client = TaskManagerClient::new(krpc_client);
        Ok(client)
    }

    pub async fn get_aicc_client(&self) -> Result<AiccClient> {
        let krpc_client = self
            .get_zone_service_krpc_client_with_default_timeout(
                AICC_SERVICE_SERVICE_NAME,
                Some(DEFAULT_AICC_KRPC_TIMEOUT_SECS),
            )
            .await?;
        let client = AiccClient::new(krpc_client);
        Ok(client)
    }

    pub async fn get_msg_center_client(&self) -> Result<MsgCenterClient> {
        let krpc_client = self
            .get_zone_service_krpc_client(MSG_CENTER_SERVICE_NAME)
            .await?;
        let client = MsgCenterClient::new(krpc_client);
        Ok(client)
    }

    pub async fn get_msg_queue_client(&self) -> Result<MsgQueueClient> {
        let krpc_client = self.get_zone_service_krpc_client(KMSG_SERVICE_NAME).await?;
        Ok(MsgQueueClient::new_krpc(Box::new(krpc_client)))
    }

    pub async fn get_scheduler_client(&self) -> Result<SchedulerClient> {
        let krpc_client = self.get_zone_service_krpc_client("scheduler").await?;
        let client = SchedulerClient::new(krpc_client);
        Ok(client)
    }

    pub async fn get_control_panel_client(&self) -> Result<ControlPanelClient> {
        let system_config_client = self.get_system_config_client().await?;
        let client = ControlPanelClient::from_shared(system_config_client);
        Ok(client)
    }

    pub async fn get_verify_hub_client(&self) -> Result<VerifyHubClient> {
        let krpc_client = self.get_zone_service_krpc_client("verify-hub").await?;
        let client = VerifyHubClient::new(krpc_client);
        Ok(client)
    }

    pub async fn get_repo_client(&self) -> Result<RepoClient> {
        let krpc_client = self
            .get_zone_service_krpc_client(REPO_SERVICE_SERVICE_NAME)
            .await?;
        Ok(RepoClient::new(krpc_client))
    }

    pub async fn get_opendan_client(&self) -> Result<OpenDanClient> {
        let krpc_client = self
            .get_zone_service_krpc_client(OPENDAN_SERVICE_NAME)
            .await?;
        Ok(OpenDanClient::new(krpc_client))
    }

    pub fn get_zone_ndn_base_url(&self) -> String {
        let mut schema = "http";
        if self.force_https {
            schema = "https";
        }
        format!("{}://{}/ndn/", schema, self.zone_id.to_host_name())
    }

    //return (url,is_local)
    pub async fn get_kernel_service_url(&self, service_name: &str) -> Result<(String, bool)> {
        // get service info from system_config
        if self.device_config.is_none() {
            warn!("access kernel service need set device_config");
            return Err(RPCErrors::ReasonError(
                "access kernel service need set device_config".to_string(),
            ));
        }
        let local_service_host = self.resolve_local_service_host();
        let control_panel_client = self.get_system_control_panel_client().await?;
        let service_info = control_panel_client.get_services_info(service_name).await?;
        // select best instance
        let local_node = service_info
            .node_list
            .get(self.device_config.as_ref().unwrap().name.as_str());
        if local_node.is_some() {
            let local_node = local_node.unwrap();
            if local_node.node_did == self.device_config.as_ref().unwrap().id {
                if local_node.state == ServiceInstanceState::Started {
                    if let Some(port) = Self::resolve_service_port(local_node, "www") {
                        //短路：直接连在本机的Service 服务的 实际端口，绕过node-gateway转发
                        return Ok((
                            format!(
                                "http://{}:{}/kapi/{}",
                                local_service_host, port, service_name
                            ),
                            true,
                        ));
                    }
                } else {
                    warn!(
                        "local node {} is not started",
                        local_node.node_did.to_string()
                    );
                }
            }
        } else {
            warn!(
                "local node {} is not found",
                self.device_config.as_ref().unwrap().name.as_str()
            );
        }

        //通过本机的node-gatewa 转发，局域网的即使可以直连也要通过rtcp,局域网的明文流量也并不安全。
        return Ok((
            format!(
                "http://{}:{}/kapi/{}",
                local_service_host, DEFAULT_NODE_GATEWAY_PORT, service_name
            ),
            false,
        ));

        //下面的实现其实永远不会进入，因为cyfs-gateway并不加载buckyos-sdk,而是通过process-chain规则完成下面逻辑
        //  得到service的provider node列表，并随机选择一个
        //  根据选择的deivce_id,查表得到forward url

        // let mut total_weight = 0;
        // for (_node_name, node_info) in service_info.node_list.iter() {
        //     if node_info.state == ServiceInstanceState::Started {
        //         total_weight += node_info.weight;
        //     }
        // }

        // let mut rng = rand::thread_rng();
        // let random_num = rng.gen_range(0..total_weight);
        // let mut current_weight = 0;
        // let mut last_best_same_lan_node_url = String::new();
        // let mut last_best_wan_node_url = String::new();
        // for (_node_name, node_info) in service_info.node_list.iter() {
        //     if node_info.state == ServiceInstanceState::Started {
        //         let maybe_port = Self::resolve_service_port(node_info, service_name);
        //         if maybe_port.is_none() {
        //             continue;
        //         }
        //         let port = maybe_port.unwrap();
        //         if node_info.node_net_id == self.device_config.as_ref().unwrap().net_id {
        //             last_best_same_lan_node_url = format!(
        //                 "rtcp://{}/127.0.0.1:{}",
        //                 node_info.node_did.to_string(),
        //                 port
        //             );
        //         }
        //         if node_info.node_net_id == Some("wan".to_string()) {
        //             last_best_wan_node_url = format!(
        //                 "rtcp://{}/127.0.0.1:{}",
        //                 node_info.node_did.to_string(),
        //                 port
        //             );
        //         }
        //         current_weight += node_info.weight;
        //         if current_weight >= random_num {
        //             if last_best_same_lan_node_url.len() > 0 {
        //                 return Ok((last_best_same_lan_node_url, false));
        //             }
        //             if last_best_wan_node_url.len() > 0 {
        //                 return Ok((last_best_wan_node_url, false));
        //             }
        //         }
        //     }
        // }
        // //todo: use wan_node to get the
        // return Err(RPCErrors::ReasonError(
        //     "no running instance found".to_string(),
        // ));
    }

    fn resolve_service_port(_node_info: &ServiceNode, _service_name: &str) -> Option<u16> {
        return _node_info.service_port.get(_service_name).cloned();
    }

    //if http_only is false, return the url with tunnel protocol
    //这里有一个隐含的假设：所有的Service通过http path就能区分
    //因为调整了hostname,所以通过二级域名区分appid在这里就看不到了
    pub async fn get_zone_service_url(
        &self,
        service_name: &str,
        https_only: bool,
    ) -> Result<String> {
        let mut schema = "http";
        if https_only {
            schema = "https";
        }

        match self.runtime_type {
            BuckyOSRuntimeType::AppClient => {
                //通过Zone Host Name 访问Service总是可以成功的，理论上有SDK的环境不应该使用这种方式。
                //TODO：如果约束为有SDK的环境，必然有node_gateway,那么这个分支就不必要存在
                let host_name = self.zone_id.to_host_name();
                if self.app_host_perfix.len() > 0 {
                    return Ok(format!(
                        "{}://{}.{}/kapi/{}",
                        schema, self.app_host_perfix, host_name, service_name
                    ));
                } else {
                    return Ok(format!("{}://{}/kapi/{}", schema, host_name, service_name));
                }
            }
            BuckyOSRuntimeType::AppService | BuckyOSRuntimeType::FrameService => {
                let (result_url, _is_local) = self.get_kernel_service_url(service_name).await?;
                return Ok(result_url);

                // if is_local {
                //     return Ok(result_url);
                // }
                // return Ok(format!(
                //     "http://127.0.0.1:{}/kapi/{}",
                //     self.node_gateway_port, service_name
                // ));
            }
            BuckyOSRuntimeType::KernelService => {
                let (result_url, _is_local) = self.get_kernel_service_url(service_name).await?;
                return Ok(result_url);
            }
            BuckyOSRuntimeType::Kernel => {
                let (result_url, _is_local) = self.get_kernel_service_url(service_name).await?;
                return Ok(result_url);
            }
        }
    }

    pub async fn get_zone_service_krpc_client(&self, service_name: &str) -> Result<kRPC> {
        self.get_zone_service_krpc_client_with_default_timeout(service_name, None)
            .await
    }

    pub async fn get_zone_service_krpc_client_with_default_timeout(
        &self,
        service_name: &str,
        default_timeout_secs: Option<u64>,
    ) -> Result<kRPC> {
        let url = self
            .get_zone_service_url(service_name, self.force_https)
            .await?;
        let session_token = self.session_token.read().await;
        let timeout_secs = default_timeout_secs.unwrap_or(DEFAULT_KRPC_TIMEOUT_SECS);

        let client = kRPC::new_with_timeout_secs(&url, Some(session_token.clone()), timeout_secs);
        Ok(client)
    }

    pub async fn get_chunklist_from_known_named_object(
        &self,
        obj_id: &ObjId,
        named_object: &Value,
    ) -> Result<Vec<ChunkId>> {
        //如果objid指向的是一个pkg_meta,则尝试解析是否为AppDoc,并根据其sub_pkg_list中的pkg_objid，逐个调用ndn-toolkit的get_named_object_from_known_named_object函数
        //先调用ndn-toolkit的get_named_object_from_known_named_object函数
        let store_mgr = self.get_named_store().await?;
        let mut chunk_ids = ndn_toolkit::tools::get_chunklist_from_known_named_object(
            &store_mgr,
            obj_id,
            named_object,
        )
        .await
        .map_err(|e| {
            RPCErrors::ReasonError(format!(
                "Failed to get chunklist from object {}: {}",
                obj_id, e
            ))
        })?;

        let app_doc = match serde_json::from_value::<crate::AppDoc>(named_object.clone()) {
            Ok(app_doc) => app_doc,
            Err(_) => return Ok(chunk_ids),
        };

        for (_, sub_pkg) in app_doc.pkg_list.iter() {
            let Some(sub_pkg_obj_id) = sub_pkg.pkg_objid.clone() else {
                continue;
            };

            let sub_pkg_json = if sub_pkg_obj_id.is_chunk() {
                Value::Null
            } else {
                let obj_str = store_mgr.get_object(&sub_pkg_obj_id).await.map_err(|e| {
                    RPCErrors::ReasonError(format!(
                        "Failed to load sub package object {} referenced by {}: {}",
                        sub_pkg_obj_id, obj_id, e
                    ))
                })?;
                serde_json::from_str::<Value>(obj_str.as_str())
                    .or_else(|_| load_named_object_from_obj_str(obj_str.as_str()))
                    .map_err(|e| {
                        RPCErrors::ReasonError(format!(
                            "Failed to parse sub package object {} referenced by {}: {}",
                            sub_pkg_obj_id, obj_id, e
                        ))
                    })?
            };

            chunk_ids.extend(
                ndn_toolkit::tools::get_chunklist_from_known_named_object(
                    &store_mgr,
                    &sub_pkg_obj_id,
                    &sub_pkg_json,
                )
                .await
                .map_err(|e| {
                    RPCErrors::ReasonError(format!(
                        "Failed to get chunklist from sub package {} referenced by {}: {}",
                        sub_pkg_obj_id, obj_id, e
                    ))
                })?,
            );
        }

        Ok(chunk_ids)
    }
}

fn resolve_container_gateway_host(configured_host: Option<&str>) -> String {
    let host = configured_host
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_DOCKER_HOST_GATEWAY);
    resolve_host_to_ipv4_literal(host).unwrap_or_else(|| host.to_string())
}

fn resolve_host_to_ipv4_literal(host: &str) -> Option<String> {
    let host = host.trim();
    if host.is_empty() {
        return None;
    }
    if let Ok(ipv4) = host.parse::<Ipv4Addr>() {
        return Some(ipv4.to_string());
    }

    (host, 0)
        .to_socket_addrs()
        .ok()?
        .find_map(|addr| match addr.ip() {
            IpAddr::V4(ipv4) => Some(ipv4.to_string()),
            IpAddr::V6(_) => None,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reuses_shared_system_config_client_and_syncs_token() {
        let runtime = BuckyOSRuntime::new("system_config_test", None, BuckyOSRuntimeType::Kernel);

        {
            let mut session_token = runtime.session_token.write().await;
            *session_token = "token-a".to_string();
        }
        let client_a = runtime
            .get_system_config_client()
            .await
            .expect("get shared client");

        {
            let mut session_token = runtime.session_token.write().await;
            *session_token = "token-b".to_string();
        }
        let client_b = runtime
            .get_system_config_client()
            .await
            .expect("reuse shared client");

        assert!(Arc::ptr_eq(&client_a, &client_b));
        assert_eq!(
            client_b.get_session_token().await,
            Some("token-b".to_string())
        );
    }

    #[tokio::test]
    async fn background_tasks_can_be_disabled_and_reenabled() {
        let runtime = BuckyOSRuntime::new("demo", None, BuckyOSRuntimeType::KernelService);
        let now = buckyos_get_unix_timestamp();

        assert!(
            runtime
                .should_run_background_task(
                    RuntimeBackgroundTaskKind::UpdateServiceInstanceInfo,
                    now
                )
                .await
        );

        runtime
            .set_background_task_enabled(
                RuntimeBackgroundTaskKind::UpdateServiceInstanceInfo,
                false,
            )
            .await;
        assert!(
            !runtime
                .should_run_background_task(
                    RuntimeBackgroundTaskKind::UpdateServiceInstanceInfo,
                    now
                )
                .await
        );

        runtime
            .set_background_task_enabled(RuntimeBackgroundTaskKind::UpdateServiceInstanceInfo, true)
            .await;
        assert!(
            runtime
                .should_run_background_task(
                    RuntimeBackgroundTaskKind::UpdateServiceInstanceInfo,
                    now
                )
                .await
        );
    }

    #[test]
    fn classify_errors_for_retry_policy() {
        assert_eq!(
            BuckyOSRuntime::classify_error(&RPCErrors::ReasonError(
                "connection refused".to_string()
            )),
            RuntimeErrorClass::Retryable
        );
        assert_eq!(
            BuckyOSRuntime::classify_error(&RPCErrors::ParseRequestError(
                "bad payload".to_string()
            )),
            RuntimeErrorClass::NonRetryable
        );
        assert_eq!(
            BuckyOSRuntime::classify_error(&RPCErrors::ReasonError(
                "NotImplemented: demo".to_string()
            )),
            RuntimeErrorClass::NonRetryable
        );
    }

    #[test]
    fn unsupported_folder_apis_return_error_instead_of_panicking() {
        let runtime = BuckyOSRuntime::new("demo", None, BuckyOSRuntimeType::AppClient);
        let data_dir = runtime.get_data_folder();
        let cache_dir = runtime.get_cache_folder();
        let local_cache_dir = runtime.get_local_cache_folder();

        assert!(data_dir.is_err());
        assert!(cache_dir.is_err());
        assert!(local_cache_dir.is_err());
    }

    #[test]
    fn container_gateway_host_prefers_ipv4_literals() {
        assert_eq!(
            resolve_container_gateway_host(Some("127.0.0.1")),
            "127.0.0.1"
        );
        assert_eq!(
            resolve_container_gateway_host(Some("localhost")),
            "127.0.0.1"
        );
    }
}
