#![allow(unused_must_use)]

use std::collections::HashMap;
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ::kRPC::*;
use buckyos_kit::*;
use jsonwebtoken::{decode, DecodingKey, EncodingKey, Validation};
use log::*;
use std::env;
use tokio::sync::RwLock;

use name_client::*;
use name_lib::*;
use rand::Rng;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuckyOSRuntimeType {
    AppClient,     //运行在所有设备上，通常不在容器里（唯一可能加载user private key的类型)
    AppService,    //R3 运行在Node上，指定用户，可能在容器里
    FrameService,  //R2 运行在Node上，通常在容器里
    KernelService, //R1 由node-daemon启动的的系统基础服务
    Kernel,        //R0,node-daemon和cyfs-gateway使用，可以单独启动的组件
}

pub struct BuckyOSRuntime {
    pub app_owner_id: Option<String>,
    pub app_id: String,
    pub app_host_perfix: String,
    pub runtime_type: BuckyOSRuntimeType,
    pub main_service_port: RwLock<u16>,

    pub user_id: Option<String>,
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

    pub force_https: bool,
    pub buckyos_root_dir: PathBuf,
    pub web3_bridges: HashMap<String, String>,
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

        let runtime = BuckyOSRuntime {
            app_id: app_id.to_string(),
            app_owner_id: app_owner_user_id,
            app_host_perfix: app_host_perfix,
            main_service_port: RwLock::new(0),
            user_id: None,
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
            web3_bridges: HashMap::new(),
            force_https: true,
        };
        runtime
    }

    pub async fn set_main_service_port(&self, port: u16) {
        let mut main_service_port = self.main_service_port.write().await;
        *main_service_port = port;
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

        if CURRENT_DEVICE_CONFIG.get().is_none() {
            let device_doc = env::var("BUCKYOS_THIS_DEVICE");
            if device_doc.is_ok() {
                let device_doc = device_doc.unwrap();
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
                let set_result = CURRENT_DEVICE_CONFIG.set(device_config.clone());
                if set_result.is_err() {
                    warn!("Failed to set CURRENT_DEVICE_CONFIG by env var");
                    return Err(RPCErrors::ReasonError(
                        "Failed to set CURRENT_DEVICE_CONFIG by env var".to_string(),
                    ));
                }
            }
        }

        let mut session_token_key = "".to_string();
        match self.runtime_type {
            BuckyOSRuntimeType::KernelService | BuckyOSRuntimeType::FrameService => {
                session_token_key = get_session_token_env_key(&self.get_full_appid(), false);
            }
            BuckyOSRuntimeType::AppService => {
                session_token_key = get_session_token_env_key(&self.get_full_appid(), true);
            }
            _ => {
                info!(
                    "will not load session_token from env var for runtime_type: {:?}",
                    self.runtime_type
                );
            }
        }

        if session_token_key.len() > 1 {
            let session_token = env::var(session_token_key.as_str());
            if session_token.is_ok() {
                info!("load session_token from env var success");
                let mut this_session_token = self.session_token.write().await;
                *this_session_token = session_token.unwrap();
            } else {
                info!("load session_token from env var failed");
                return Err(RPCErrors::ReasonError(
                    "load session_token from env var failed".to_string(),
                ));
            }
        }

        Ok(())
    }

    pub async fn fill_policy_by_load_config(&mut self) -> Result<()> {
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
            if CURRENT_DEVICE_CONFIG.get().is_none() {
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
                self.device_config = Some(device_config.clone());
                self.user_id = Some(device_config.name.clone());
                let set_result = CURRENT_DEVICE_CONFIG.set(device_config.clone());
                if set_result.is_err() {
                    warn!("Failed to set CURRENT_DEVICE_CONFIG");
                    return Err(RPCErrors::ReasonError(
                        "Failed to set CURRENT_DEVICE_CONFIG".to_string(),
                    ));
                }
                let zone_did = node_identity_config.zone_did.clone();
                self.zone_id = zone_did.clone();
            }
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
                self.user_id = Some(owner_config.name.clone());
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
                    self.user_id = Some("root".to_string());
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

    pub async fn update_service_instance_info(&self) -> Result<()> {
        if !self.is_service() {
            return Ok(());
        }

        let mut last_update_service_info_time = self.last_update_service_info_time.write().await;
        let now = buckyos_get_unix_timestamp();
        if now - *last_update_service_info_time
            < crate::app_mgr::SERVICE_INSTANCE_INFO_UPDATE_INTERVAL
        {
            return Ok(());
        }
        *last_update_service_info_time = now;
        drop(last_update_service_info_time);

        let node_did = self.device_config.as_ref().unwrap().id.clone();
        let node_id = self.device_config.as_ref().unwrap().name.clone();
        let instance_id = format!(
            "{}-{}",
            self.app_id,
            self.device_config
                .as_ref()
                .map(|cfg| cfg.name.clone())
                .unwrap_or_default()
        );
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
        let _ = control_panel_client
            .update_service_instance_info(&self.app_id, &node_id, &service_instance_info)
            .await?;
        info!("update service instance info,app_id:{}", self.app_id);
        Ok(())
    }

    pub async fn renew_token_from_verify_hub(&self) -> Result<()> {
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

    async fn keep_alive() -> Result<()> {
        //info!("buckyos-api-runtime::keep_alive start");
        let buckyos_api_runtime = match get_buckyos_api_runtime() {
            Ok(runtime) => runtime,
            Err(error) => {
                warn!(
                    "buckyos-api-runtime is not initialized, skip keep_alive tick: {}",
                    error
                );
                return Ok(());
            }
        };
        let refresh_result = buckyos_api_runtime.renew_token_from_verify_hub().await;
        if refresh_result.is_err() {
            warn!(
                "buckyos-api-runtime::renew_token_from_verify_hub failed {:?}",
                refresh_result.err().unwrap()
            );
        }

        let _ = buckyos_api_runtime.update_service_instance_info().await;

        if buckyos_api_runtime.is_service() {
            // RBAC is initialized at login(). Avoid high-frequency remote RBAC reload here;
            // keepalive should prioritize stability and token/service liveness.
            buckyos_api_runtime.refresh_trust_keys().await?;
        }
        Ok(())
    }
    //if login by jwt failed, exit current process is the best choose
    pub async fn login(&mut self) -> Result<()> {
        if !self.zone_id.is_valid() {
            return Err(RPCErrors::ReasonError(
                "Zone id is not valid,api-runtime.login failed".to_string(),
            ));
        }

        let real_session_token;
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

        {
            let mut session_token = self.session_token.write().await;
            if session_token.is_empty() {
                //info!("api-runtime: session token is empty,runtime_type:{:?},try to create session token by known private key",self.runtime_type);
                if self.runtime_type == BuckyOSRuntimeType::AppClient {
                    if self.user_private_key.is_some() && self.user_config.is_some() {
                        info!("api-runtime: session token is empty,runtime_type:{:?},try to create session token by user_private_key",self.runtime_type);
                        let (session_token_str, _real_session_token) =
                            RPCSessionToken::generate_jwt_token(
                                self.user_id.as_ref().unwrap(),
                                self.app_id.as_str(),
                                None,
                                self.user_private_key.as_ref().unwrap(),
                            )?;
                        *session_token = session_token_str;
                    }
                }

                if self.device_private_key.is_some()
                    && self.device_config.is_some()
                    && session_token.is_empty()
                {
                    info!("buckyos-api-runtime: session token is empty,runtime_type:{:?},try to create session token by device_private_key",self.runtime_type);
                    let device_name = &self.device_config.as_ref().unwrap().name;
                    let (session_token_str, _real_session_token) =
                        RPCSessionToken::generate_jwt_token(
                            device_name.as_str(),
                            self.app_id.as_str(),
                            None,
                            self.device_private_key.as_ref().unwrap(),
                        )?;
                    *session_token = session_token_str;
                }

                if session_token.is_empty() {
                    return Err(RPCErrors::ReasonError(
                        "session_token is empty!".to_string(),
                    ));
                }
                drop(session_token);
            } else {
                info!(
                    "buckyos-api-runtime: session token is set,runtime_type:{:?}",
                    self.runtime_type
                );
                real_session_token = RPCSessionToken::from_string(session_token.as_str())?;
                drop(session_token);

                info!("real_session_token: {:?}", real_session_token);
                let appid = real_session_token
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
                //login by jwt
                // let verify_hub_client = self.get_verify_hub_client().await?;
                // let login_result = verify_hub_client.login_by_jwt(real_session_token.to_string(),None).await?;
                // info!("verify_hub_client login by jwt success,login_result: {:?}",login_result);
            }
        }

        info!("all config is checked,try to connect to control-panel and get zone config");
        //session token already set, try to connect to control-panel and get zone config
        let control_panel_client = self.get_control_panel_client().await?;
        let zone_config = control_panel_client.load_zone_config().await?;
        self.zone_config = Some(zone_config);
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
            self.refresh_trust_keys().await?;
            info!("refresh trust keys OK");
        }

        //start keep-alive timer
        tokio::task::spawn(async move {
            let start = tokio::time::Instant::now() + Duration::from_secs(5);
            let mut timer = tokio::time::interval_at(start, Duration::from_secs(5));
            loop {
                timer.tick().await;
                let result = BuckyOSRuntime::keep_alive().await;
                if result.is_err() {
                    warn!(
                        "buckyos-api-runtime::keep_alive failed {:?}",
                        result.err().unwrap()
                    );
                }
            }
        });

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
                if self.device_private_key.is_some() {
                    let device_private_key = self.device_private_key.as_ref().unwrap();
                    let device_uid = self.device_config.as_ref().unwrap().name.clone();
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

    pub fn get_data_folder(&self) -> PathBuf {
        match self.runtime_type {
            BuckyOSRuntimeType::AppClient => {
                //返回
                panic!("AppClient not support get_data_folder");
            }
            BuckyOSRuntimeType::AppService => {
                //返回
                return self
                    .buckyos_root_dir
                    .join("data")
                    .join(self.user_id.clone().unwrap())
                    .join(self.app_id.clone());
            }
            BuckyOSRuntimeType::FrameService
            | BuckyOSRuntimeType::KernelService
            | BuckyOSRuntimeType::Kernel => {
                return self.buckyos_root_dir.join("data").join(self.app_id.clone());
                //返回
            }
        }
    }

    pub fn get_cache_folder(&self) -> PathBuf {
        match self.runtime_type {
            BuckyOSRuntimeType::AppClient => {
                //返回
                panic!("AppClient not support get_cache_folder");
            }
            BuckyOSRuntimeType::AppService => {
                //返回
                return self
                    .buckyos_root_dir
                    .join("cache")
                    .join(self.user_id.clone().unwrap())
                    .join(self.app_id.clone());
            }
            BuckyOSRuntimeType::FrameService
            | BuckyOSRuntimeType::KernelService
            | BuckyOSRuntimeType::Kernel => {
                return self
                    .buckyos_root_dir
                    .join("cache")
                    .join(self.app_id.clone()); //返回
            }
        }
    }

    pub fn get_local_cache_folder(&self) -> PathBuf {
        match self.runtime_type {
            BuckyOSRuntimeType::AppClient => {
                //返回
                panic!("AppClient not support get_local_cache_folder");
            }
            BuckyOSRuntimeType::AppService => {
                return self
                    .buckyos_root_dir
                    .join("tmp")
                    .join(self.user_id.clone().unwrap())
                    .join(self.app_id.clone());
            }
            BuckyOSRuntimeType::FrameService
            | BuckyOSRuntimeType::KernelService
            | BuckyOSRuntimeType::Kernel => {
                return self.buckyos_root_dir.join("tmp").join(self.app_id.clone());
            }
        }
    }

    // 获得与物理逻辑磁盘绑定的本地存储目录，存储的可靠性和特性由物理磁盘决定
    //目录原理上是  disk_id/service_instance_id/
    pub fn get_lcoal_storage_folder(&self, disk_id: Option<String>) -> PathBuf {
        if self.runtime_type == BuckyOSRuntimeType::KernelService
            || self.runtime_type == BuckyOSRuntimeType::FrameService
        {
            if disk_id.is_some() {
                let disk_id = disk_id.unwrap();
                return self
                    .buckyos_root_dir
                    .join("local")
                    .join(disk_id)
                    .join(self.app_id.clone());
            } else {
                return self
                    .buckyos_root_dir
                    .join("local")
                    .join(self.app_id.clone());
            }
        } else {
            panic!("This runtime type not support get_lcoal_storage_folder");
        }
    }

    pub fn get_root_pkg_env_path() -> PathBuf {
        get_buckyos_service_local_data_dir("node_daemon", None).join("root_pkg_env")
    }

    fn get_my_settings_path(&self) -> String {
        match self.runtime_type {
            BuckyOSRuntimeType::AppClient => {
                panic!("AppClient not support get_my_settings_path");
            }
            BuckyOSRuntimeType::AppService => {
                format!(
                    "users/{}/apps/{}/settings",
                    self.user_id.as_ref().unwrap(),
                    self.app_id.as_str()
                )
            }
            BuckyOSRuntimeType::FrameService
            | BuckyOSRuntimeType::KernelService
            | BuckyOSRuntimeType::Kernel => {
                format!("services/{}/settings", self.app_id.as_str())
            }
        }
    }

    pub async fn get_my_settings(&self) -> Result<serde_json::Value> {
        let system_config_client = self.get_system_config_client().await?;
        let settiing_path = self.get_my_settings_path();
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
        let settiing_path = self.get_my_settings_path();
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
        let settiing_path = self.get_my_settings_path();
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
            BuckyOSRuntimeType::AppClient => {
                format!(
                    "users/{}/apps/{}/{}",
                    self.user_id.as_ref().unwrap(),
                    self.app_id.as_str(),
                    config_name
                )
            }
            BuckyOSRuntimeType::AppService => {
                format!(
                    "users/{}/apps/{}/{}",
                    self.user_id.as_ref().unwrap(),
                    self.app_id.as_str(),
                    config_name
                )
            }
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
        let client = ControlPanelClient::new(system_config_client);
        Ok(client)
    }

    pub async fn get_system_config_client(&self) -> Result<SystemConfigClient> {
        let mut url = "http://127.0.0.1:3200/kapi/system_config".to_string();
        let mut schema = "http";
        if self.force_https {
            schema = "https";
        }
        let zone_host = self.zone_id.to_host_name();
        if !self.is_ood() {
            match self.runtime_type {
                BuckyOSRuntimeType::AppClient => {
                    url = format!("{}://{}/kapi/system_config", schema, zone_host);
                }
                BuckyOSRuntimeType::AppService => {
                    url = format!(
                        "http://127.0.0.1:{}/kapi/system_config",
                        DEFAULT_NODE_GATEWAY_PORT
                    );
                }
                _ => {
                    //do nothing
                }
            }
        }

        //let url = self.get_zone_service_url("system_config",self.force_https)?;
        let session_token = self.get_session_token().await;
        let client = SystemConfigClient::new(Some(url.as_str()), Some(session_token.as_str()));
        debug!("get system config client OK,url:{}", url);
        Ok(client)
    }

    pub async fn get_task_mgr_client(&self) -> Result<TaskManagerClient> {
        let krpc_client = self.get_zone_service_krpc_client("task-manager").await?;
        let client = TaskManagerClient::new(krpc_client);
        Ok(client)
    }

    pub async fn get_aicc_client(&self) -> Result<AiccClient> {
        let krpc_client = self
            .get_zone_service_krpc_client(AICC_SERVICE_SERVICE_NAME)
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
        let client = ControlPanelClient::new(system_config_client);
        Ok(client)
    }

    pub async fn get_verify_hub_client(&self) -> Result<VerifyHubClient> {
        let krpc_client = self.get_zone_service_krpc_client("verify-hub").await?;
        let client = VerifyHubClient::new(krpc_client);
        Ok(client)
    }

    pub async fn get_repo_client(&self) -> Result<RepoClient> {
        let krpc_client = self.get_zone_service_krpc_client("repo-service").await?;
        let client = RepoClient::new(krpc_client);
        Ok(client)
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
            return Err(RPCErrors::ReasonError(
                "access kernel service need set device_config".to_string(),
            ));
        }
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
                        return Ok((
                            format!("http://127.0.0.1:{}/kapi/{}", port, service_name),
                            true,
                        ));
                    }
                }
            }
        }

        if self.runtime_type != BuckyOSRuntimeType::Kernel {
            return Ok((
                format!(
                    "http://127.0.0.1:{}/kapi/{}",
                    DEFAULT_NODE_GATEWAY_PORT, service_name
                ),
                false,
            ));
        }

        let mut total_weight = 0;
        for (_node_name, node_info) in service_info.node_list.iter() {
            if node_info.state == ServiceInstanceState::Started {
                total_weight += node_info.weight;
            }
        }

        let mut rng = rand::thread_rng();
        let random_num = rng.gen_range(0..total_weight);
        let mut current_weight = 0;
        let mut last_best_same_lan_node_url = String::new();
        let mut last_best_wan_node_url = String::new();
        for (_node_name, node_info) in service_info.node_list.iter() {
            if node_info.state == ServiceInstanceState::Started {
                let maybe_port = Self::resolve_service_port(node_info, service_name);
                if maybe_port.is_none() {
                    continue;
                }
                let port = maybe_port.unwrap();
                if node_info.node_net_id == self.device_config.as_ref().unwrap().net_id {
                    last_best_same_lan_node_url = format!(
                        "rtcp://{}/127.0.0.1:{}",
                        node_info.node_did.to_string(),
                        port
                    );
                }
                if node_info.node_net_id == Some("wan".to_string()) {
                    last_best_wan_node_url = format!(
                        "rtcp://{}/127.0.0.1:{}",
                        node_info.node_did.to_string(),
                        port
                    );
                }
                current_weight += node_info.weight;
                if current_weight >= random_num {
                    if last_best_same_lan_node_url.len() > 0 {
                        return Ok((last_best_same_lan_node_url, false));
                    }
                    if last_best_wan_node_url.len() > 0 {
                        return Ok((last_best_wan_node_url, false));
                    }
                }
            }
        }
        //todo: use wan_node to get the
        return Err(RPCErrors::ReasonError(
            "no running instance found".to_string(),
        ));
    }

    fn resolve_service_port(_node_info: &ServiceNode, _service_name: &str) -> Option<u16> {
        return _node_info.service_port.get(_service_name).cloned();
    }

    //if http_only is false, return the url with tunnel protocol
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
                //TODO: 基于appid对system service的访问进行控制，以消除对跨域的依赖，需要依赖新版本的cyfs-gateway
                //let host_name = format!("{}.{}",self.app_host_perfix,self.zone_id.to_host_name());
                let host_name = self.zone_id.to_host_name();
                return Ok(format!("{}://{}/kapi/{}", schema, host_name, service_name));
            }
            BuckyOSRuntimeType::AppService | BuckyOSRuntimeType::FrameService => {
                let (result_url, is_local) = self.get_kernel_service_url(service_name).await?;
                if is_local {
                    return Ok(result_url);
                }
                return Ok(format!(
                    "http://127.0.0.1:{}/kapi/{}",
                    self.node_gateway_port, service_name
                ));
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
        let url = self
            .get_zone_service_url(service_name, self.force_https)
            .await?;
        let session_token = self.session_token.read().await;
        let client = kRPC::new(&url, Some(session_token.clone()));
        Ok(client)
    }
}
