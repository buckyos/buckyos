#![allow(dead_code)]
#![allow(unused)]
mod system_config;
mod sn_client;
mod zone_gateway;
mod task_mgr;
mod control_panel;
mod scheduler_client;
mod verify_hub_client;
mod zone_provider;
mod repo_client;

use name_lib::{DeviceConfig, DeviceInfo, ZoneConfig};
pub use system_config::*;
pub use sn_client::*;
use tokio::sync::RwLock;
pub use zone_gateway::*;
pub use task_mgr::*;
pub use control_panel::*;
pub use scheduler_client::*;
pub use verify_hub_client::*;
pub use zone_provider::*;
use std::fmt::format;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use serde_json::Value;
use lazy_static::lazy_static;
use buckyos_kit::*;
use ::kRPC::*;
use std::env;
use once_cell::sync::OnceCell;
use log::*;
use name_lib::*;
use name_client::*;
use zone_provider::*;
use repo_client::*;

use jsonwebtoken::{encode,decode,Header, Algorithm, Validation, EncodingKey, DecodingKey};
use jsonwebtoken::jwk::Jwk;
use std::time::Duration;


#[derive(Debug, Clone,PartialEq,Eq)]
pub enum BuckyOSRuntimeType {
    AppClient,    //R3 可能运行在Node上，指定用户，可能在容器里
    AppService,   //R2 运行在Node上，指定用户，可能在容器里
    FrameService, //R1 运行在Node上，可能在容器里
    KernelService,//R0 运行在Node上
}

#[derive(Clone)]
pub struct BuckyOSRuntime {
    pub app_owner_id:Option<String>,
    pub app_id:String,
    pub runtime_type:BuckyOSRuntimeType,

    pub user_id:Option<String>,
    pub user_config:Option<OwnerConfig>,
    pub user_private_key:Option<EncodingKey>,

    pub device_config:Option<DeviceConfig>, 
    pub device_private_key:Option<EncodingKey>,
    pub device_info:Option<DeviceInfo>,

    pub zone_id:DID,
    pub zone_boot_config:Option<ZoneBootConfig>,

    //pub is_token_iss_by_self:bool,
    pub zone_config:Option<ZoneConfig>,
    pub session_token:Arc<RwLock<String>>,
    trust_keys:Arc<RwLock<HashMap<String,DecodingKey>>>,
    
    pub force_https:bool,
    pub buckyos_root_dir:PathBuf,
    pub web3_bridges:HashMap<String,String>,
}


//pub static CURRENT_ZONE_CONFIG: OnceCell<ZoneConfig> = OnceCell::new();
//pub static INIT_APP_SESSION_TOKEN: OnceCell<String> = OnceCell::new();
pub static CURRENT_DEVICE_CONFIG: OnceCell<DeviceConfig> = OnceCell::new();
pub fn try_load_current_device_config_from_env() -> NSResult<()> {
    let device_doc = env::var("BUCKYOS_THIS_DEVICE");
    if device_doc.is_err() {
        return Err(NSError::NotFound("BUCKY_DEVICE_DOC not set".to_string()));
    }
    let device_doc = device_doc.unwrap();

    let device_config= serde_json::from_str(device_doc.as_str());
    if device_config.is_err() {
        warn!("parse device_doc format error");
        return Err(NSError::Failed("device_doc format error".to_string()));
    }
    let device_config:DeviceConfig = device_config.unwrap();
    let set_result = CURRENT_DEVICE_CONFIG.set(device_config);
    if set_result.is_err() {
        warn!("Failed to set CURRENT_DEVICE_CONFIG");
        return Err(NSError::Failed("Failed to set CURRENT_DEVICE_CONFIG".to_string()));
    }
    Ok(())
}

static CURRENT_BUCKYOS_RUNTIME:OnceCell<BuckyOSRuntime> = OnceCell::new();
pub fn get_buckyos_api_runtime() -> Result<&'static BuckyOSRuntime> {
    let runtime = CURRENT_BUCKYOS_RUNTIME.get().unwrap();
    Ok(runtime)
}

pub fn set_buckyos_api_runtime(runtime: BuckyOSRuntime) {
    CURRENT_BUCKYOS_RUNTIME.set(runtime);
}

pub fn is_buckyos_api_runtime_set() -> bool {
    CURRENT_BUCKYOS_RUNTIME.get().is_some()
}

pub fn get_full_appid(app_id: &str,owner_user_id: &str) -> String {
    format!("{}-{}",owner_user_id,app_id)
}

pub fn get_session_token_env_key(app_full_id: &str,is_app_service:bool) -> String {
    let app_id = app_full_id.to_uppercase();
    let app_id = app_id.replace("-","_");
    if !is_app_service {
        format!("{}_SESSION_TOKEN",app_id)
    } else {
        format!("{}_TOKEN",app_id)
    }
}

pub async fn init_buckyos_api_runtime(app_id:&str,app_owner_id:Option<String>,runtime_type:BuckyOSRuntimeType) -> Result<BuckyOSRuntime> {
    if CURRENT_BUCKYOS_RUNTIME.get().is_some() {
        return Err(RPCErrors::ReasonError("BuckyOSRuntime already initialized".to_string()));
    }

    match runtime_type {
        BuckyOSRuntimeType::AppService => {
            if app_owner_id.is_none() {
                return Err(RPCErrors::ReasonError("owner_user_id is required for AppClient or AppService".to_string()));
            }
        }
        _ => {
            //do nothing
        }
    }

    let mut runtime = BuckyOSRuntime::new(app_id,app_owner_id,runtime_type);
    runtime.fill_policy_by_load_config().await?;
    runtime.fill_by_load_config().await?;
    runtime.fill_by_env_var().await?;
    //CURRENT_BUCKYOS_RUNTIME.set(runtime);
    Ok(runtime)
}


impl BuckyOSRuntime {

    pub fn new(app_id: &str,app_owner_user_id: Option<String>,runtime_type: BuckyOSRuntimeType) -> Self {
        let runtime = BuckyOSRuntime {
            app_id: app_id.to_string(),
            app_owner_id: app_owner_user_id,
            user_id: None,
            runtime_type,
            session_token: Arc::new(RwLock::new("".to_string())),
            buckyos_root_dir: get_buckyos_root_dir(),
            zone_config: None,
            device_config: None,
            device_private_key: None,
            device_info: None,
            user_private_key: None,
            user_config: None,
            zone_id: DID::undefined(),
            zone_boot_config: None,
            trust_keys: Arc::new(RwLock::new(HashMap::new())),
            web3_bridges: HashMap::new(),
            force_https: true,
            
        };
        runtime
    }

    pub async fn fill_by_env_var(&mut self) -> Result<()> {
        let zone_boot_config = env::var("BUCKYOS_ZONE_BOOT_CONFIG");
        if zone_boot_config.is_ok() {
            let zone_boot_config:ZoneBootConfig = serde_json::from_str(zone_boot_config.unwrap().as_str())
                .map_err(|e| {
                    error!("Failed to parse zone boot config: {}", e);
                    RPCErrors::ReasonError(format!("Failed to parse zone boot config: {}", e))
                })?;
            if zone_boot_config.id.is_none() {
                return Err(RPCErrors::ReasonError("zone_boot_config id is not set".to_string()));
            }
            
            self.zone_id = zone_boot_config.id.clone().unwrap();
            self.zone_boot_config = Some(zone_boot_config);
        } else {
            let zone_config_str = env::var("BUCKYOS_ZONE_CONFIG");
            if zone_config_str.is_ok() {
                let zone_config_str = zone_config_str.unwrap();
                debug!("zone_config_str:{}",zone_config_str);    
                let zone_config = serde_json::from_str(zone_config_str.as_str());
                if zone_config.is_err() {
                    warn!("zone_config_str format error");
                    return Err(RPCErrors::ReasonError("zone_config_str format error".to_string()));
                }
                let zone_config:ZoneConfig = zone_config.unwrap();
                self.zone_id = zone_config.id.clone();
                self.zone_config = Some(zone_config);
            }
        }

        let device_info_str = env::var("BUCKYOS_THIS_DEVICE_INFO");
        if device_info_str.is_ok() {
            let device_info_str = device_info_str.unwrap();
            let device_info = serde_json::from_str(device_info_str.as_str());
            if device_info.is_err() {
                warn!("device_info_str format error");  
                return Err(RPCErrors::ReasonError("device_info_str format error".to_string()));
            }
            let device_info = device_info.unwrap();
            self.device_info = Some(device_info);
        }
        

        if CURRENT_DEVICE_CONFIG.get().is_none() {
            let device_doc = env::var("BUCKYOS_THIS_DEVICE");
            if device_doc.is_ok() {
                let device_doc = device_doc.unwrap();
                info!("device_doc:{}",device_doc);
                let device_config= serde_json::from_str(device_doc.as_str());
                if device_config.is_err() {
                    warn!("device_doc format error");
                    return Err(RPCErrors::ReasonError("device_doc format error".to_string()));
                }
                let device_config:DeviceConfig = device_config.unwrap();
                self.device_config = Some(device_config.clone());
                let set_result = CURRENT_DEVICE_CONFIG.set(device_config.clone());
                if set_result.is_err() {
                    warn!("Failed to set CURRENT_DEVICE_CONFIG by env var");
                    return Err(RPCErrors::ReasonError("Failed to set CURRENT_DEVICE_CONFIG by env var".to_string()));
                }
            }
        }

        let session_token_key;
        if self.runtime_type == BuckyOSRuntimeType::KernelService || 
           self.runtime_type == BuckyOSRuntimeType::FrameService {
            session_token_key = get_session_token_env_key(&self.get_full_appid(),false);
        } else {
            session_token_key = get_session_token_env_key(&self.get_full_appid(),true);
            
        }

        let session_token = env::var(session_token_key.as_str());
        if session_token.is_ok() {
            info!("load session_token from env var success");
            let mut this_session_token = self.session_token.write().await;
            *this_session_token = session_token.unwrap();
        } else {
            info!("load session_token from env var failed");
        }

        Ok(())
    }

    pub async fn fill_policy_by_load_config(&mut self) -> Result<()> {
        let mut config_root_dir = None;
        if self.runtime_type == BuckyOSRuntimeType::AppClient {
            let bucky_dev_user_home_dir = get_buckyos_dev_user_home();
            if bucky_dev_user_home_dir.exists() {
                info!("dev folder exists: {}",bucky_dev_user_home_dir.to_string_lossy());
                config_root_dir = Some(bucky_dev_user_home_dir);
            } else {
                info!("dev folder {} not exists,try to use $BUCKYOS_ROOT/etc folder",bucky_dev_user_home_dir.to_string_lossy());
            }
        } 

        if config_root_dir.is_none() {
            let etc_dir = get_buckyos_system_etc_dir();
            config_root_dir = Some(etc_dir);
        }
    
        if config_root_dir.is_none() {
            return Err(RPCErrors::ReasonError("config_root_dir is not set".to_string()));
        }
        let config_root_dir = config_root_dir.unwrap();
        if !config_root_dir.exists() {
            error!("config_root_dir not exists: {}, init buckyos runtime config would failed!",
                config_root_dir.to_string_lossy());
            return Err(RPCErrors::ReasonError("config_root_dir not exists".to_string()));
        }
        info!("will use config_root_dir: {} to load buckyos runtime config", 
            config_root_dir.to_string_lossy());
    
        let machine_config_file = config_root_dir.join("machine.json");

        let mut machine_config = BuckyOSMachineConfig::default();
        if machine_config_file.exists() {
            let machine_config_file = File::open(machine_config_file);
            if machine_config_file.is_ok() {
                let machine_config_json = serde_json::from_reader(machine_config_file.unwrap());
                if machine_config_json.is_ok() {
                    machine_config = machine_config_json.unwrap();
                } else {
                    error!("Failed to parse machine_config: {}", machine_config_json.err().unwrap());
                    return Err(RPCErrors::ReasonError(format!("Failed to parse machine_config ")));
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
                info!("dev folder exists: {}",bucky_dev_user_home_dir.to_string_lossy());
                config_root_dir = Some(bucky_dev_user_home_dir);
            } else {
                info!("dev folder {} not exists,try to use $BUCKYOS_ROOT/etc folder",bucky_dev_user_home_dir.to_string_lossy());
            }
        } 

        if config_root_dir.is_none() {
            let etc_dir = get_buckyos_system_etc_dir();
            config_root_dir = Some(etc_dir);
        }
        
        // //使用当前当前执行文件的目录作为配置根目录主要是为了兼容cyfs-gateway,可以不支持？
        // let exe_path = std::env::current_exe()
        // .map_err(|e| {
        //     error!("cannot get exe path: {}", e);
        //     RPCErrors::ReasonError(format!("cannot get exe path: {}", e))
        // })?;
        // let exe_dir = exe_path.parent()
        //     .ok_or_else(|| {
        //         let err_msg = "cannot get exe path";
        //         error!("{}", err_msg);
        //         RPCErrors::ReasonError(err_msg.to_string())
        //     })?;
        // config_root_dir = Some(exe_dir.to_path_buf());

        if config_root_dir.is_none() {
            return Err(RPCErrors::ReasonError("config_root_dir is not set".to_string()));
        }
        let config_root_dir = config_root_dir.unwrap();
        if !config_root_dir.exists() {
            error!("config_root_dir not exists: {}, init buckyos runtime config would failed!",
                config_root_dir.to_string_lossy());
            return Err(RPCErrors::ReasonError("config_root_dir not exists".to_string()));
        }
        info!("will use config_root_dir: {} to load buckyos runtime config", 
            config_root_dir.to_string_lossy());
    
        let node_identity_file = config_root_dir.join("node_identity.json");
        let device_private_key_file = config_root_dir.join("node_private_key.pem");

        let node_identity_config =  NodeIdentityConfig::load_node_identity_config(&node_identity_file)
            .map_err(|e| {
                error!("Failed to load node identity config: {}", e);
                RPCErrors::ReasonError(format!("Failed to load node identity config: {}", e))
            })?;

    
        if CURRENT_DEVICE_CONFIG.get().is_none() {
            let device_config = decode_jwt_claim_without_verify(node_identity_config.device_doc_jwt.as_str())
                .map_err(|e| {
                    error!("Failed to decode device config: {}", e);
                    RPCErrors::ReasonError(format!("Failed to decode device config: {}", e))
                })?;

            let devcie_config = serde_json::from_value::<DeviceConfig>(device_config);
            if devcie_config.is_err() {
                error!("Failed to parse device config: {}", devcie_config.err().unwrap());
                return Err(RPCErrors::ReasonError(format!("Failed to parse device config from jwt: {}", node_identity_config.device_doc_jwt.as_str())));
            }
            let device_config = devcie_config.unwrap();
            self.device_config = Some(device_config.clone());
            self.user_id = Some(device_config.name.clone());
            let set_result = CURRENT_DEVICE_CONFIG.set(device_config.clone());
            if set_result.is_err() {
                warn!("Failed to set CURRENT_DEVICE_CONFIG");
                    return Err(RPCErrors::ReasonError("Failed to set CURRENT_DEVICE_CONFIG".to_string()));
            }
        }

        let private_key = load_private_key(&device_private_key_file);
        if private_key.is_ok() {
            self.device_private_key = Some(private_key.unwrap());
        } else {
            self.device_private_key = None;
        }

        //let zone_config_file;
        if self.runtime_type == BuckyOSRuntimeType::AppClient {
            let user_config_file = config_root_dir.join("user_config.json");
            let user_private_key_file = config_root_dir.join("user_private_key.pem");
            if user_config_file.exists() {
                let owner_config = OwnerConfig::load_owner_config(&user_config_file)
                    .map_err(|e| {
                        error!("Failed to load owner config: {}", e);
                        RPCErrors::ReasonError(format!("Failed to load owner config: {}", e))
                    })?;
                self.user_id = Some(owner_config.name.clone());
                self.user_config = Some(owner_config);
                
                let private_key = load_private_key(&user_private_key_file);
                if private_key.is_ok() {
                    info!("!!!! Make sure your development machine is secured,user_private_key_file {} load success, ",user_private_key_file.to_string_lossy());
                    self.user_private_key = Some(private_key.unwrap());
                    self.user_id = Some("root".to_string());
                } else {
                    info!("user_private_key_file {} load failed!:{:?}",user_private_key_file.to_string_lossy(),private_key.err().unwrap());
                }
            }
        }
        let zone_did = node_identity_config.zone_did.clone();
        self.zone_id = zone_did.clone();


        Ok(())
    }

    pub async fn renew_token_from_verify_hub(& self) -> Result<()> {
        let mut session_token = self.session_token.write().await;
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
        if real_session_token.iss.is_some() {
            let iss = real_session_token.iss.unwrap();
            if iss != "verify-hub" {
                need_refresh = true;
            }
        }

        if !need_refresh {
            let expired_time = real_session_token.exp.unwrap();
            let now = buckyos_get_unix_timestamp();
            if now < expired_time - 30 {
                debug!("session_token is not expired,skip renew token");
                return Ok(());
            }
        }

        info!("session_token is close to expired,try to renew token");
        let verify_hub_client = self.get_verify_hub_client().await?;
        let login_result = verify_hub_client.login_by_jwt(session_token_str,None).await?;
        info!("verify_hub_client login by jwt success,login_result: {:?}",login_result);
        let mut session_token = self.session_token.write().await;
        *session_token = login_result.token.unwrap();
        Ok(())
    }

    async fn keep_alive() -> Result<()> {
        //info!("buckyos-api-runtime::keep_alive start");
        let buckyos_api_runtime = get_buckyos_api_runtime().unwrap();
        let refresh_result = buckyos_api_runtime.renew_token_from_verify_hub().await;
        if refresh_result.is_err() {
            warn!("buckyos-api-runtime::keep_alive failed {:?}",refresh_result.err().unwrap());
        }
        Ok(())
    }
    //if login by jwt failed, exit current process is the best choose
    pub async fn login(&mut self) -> Result<()> {
        if !self.zone_id.is_valid() {
            return Err(RPCErrors::ReasonError("Zone id is not valid,api-runtime.login failed".to_string()));
        }

        let mut real_session_token; 
        if self.app_id.is_empty() {
            return Err(RPCErrors::ReasonError("App id is not set".to_string()));
        }

        if self.runtime_type == BuckyOSRuntimeType::FrameService || self.runtime_type == BuckyOSRuntimeType::KernelService {
            if self.device_config.is_none() {
                return Err(RPCErrors::ReasonError("Device config is not set!".to_string()));
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
                        let (session_token_str,real_session_token) = RPCSessionToken::generate_jwt_token(
                            self.user_id.as_ref().unwrap(),
                            self.app_id.as_str(),
                            Some(self.user_config.as_ref().unwrap().name.to_string()),
                            self.user_private_key.as_ref().unwrap()
                        )?;
                        *session_token = session_token_str;
                    } else if self.device_private_key.is_some() && self.device_config.is_some() {
                        info!("buckyos-api-runtime: session token is empty,runtime_type:{:?},try to create session token by device_private_key",self.runtime_type);
                        let (session_token_str,real_session_token) = RPCSessionToken::generate_jwt_token(
                            self.user_id.as_ref().unwrap(),
                            self.app_id.as_str(),
                            Some(self.device_config.as_ref().unwrap().name.clone()),
                            self.device_private_key.as_ref().unwrap()
                        )?;
                        *session_token = session_token_str;
                    } else {
                        return Err(RPCErrors::ReasonError("session_token is empty, and No private key found,generate session_token failed!".to_string()));
                    }
                } else {
                    return Err(RPCErrors::ReasonError("session_token is empty!".to_string()));
                }
                drop(session_token);
            } else {
                info!("buckyos-api-runtime: session token is set,runtime_type:{:?}",self.runtime_type);
                real_session_token = RPCSessionToken::from_string(session_token.as_str())?;
                drop(session_token);

                info!("real_session_token: {:?}",real_session_token);
                let appid = real_session_token.appid.clone().unwrap_or("kernel".to_string());
                if appid != self.app_id {
                    warn!("Session token is not valid,appid:{} != self.app_id:{}",appid,self.app_id);
                    return Err(RPCErrors::ReasonError("Session token is not valid".to_string()));
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
        if self.runtime_type == BuckyOSRuntimeType::KernelService || self.runtime_type == BuckyOSRuntimeType::FrameService {
            let (rbac_model,rbac_policy) = control_panel_client.load_rbac_config().await?;
            rbac::create_enforcer(Some(rbac_model.as_str()),Some(rbac_policy.as_str())).await.unwrap();
            self.refresh_trust_keys().await?;
            info!("refresh trust keys OK");

            //start keep-alive timer to
            tokio::task::spawn(async move {
                // 从当前时间+5秒开始，每5秒执行一次
                let start = tokio::time::Instant::now() + Duration::from_secs(5);
                let mut timer = tokio::time::interval_at(start, Duration::from_secs(5));
                loop {
                    timer.tick().await;
                    let result = BuckyOSRuntime::keep_alive().await;
                    if result.is_err() {
                        warn!("buckyos-api-runtime::keep_alive failed {:?}",result.err().unwrap());
                    }
                }
            });
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

    pub async fn remove_trust_key(&self,kid: &str) -> Result<()> {
        let mut key_map = self.trust_keys.write().await;
        let remove_result = key_map.remove(kid);
        if remove_result.is_none() {
            return Err(RPCErrors::ReasonError(format!("kid {} not found",kid)));
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
                self.set_trust_key(kid.as_str(),&key).await;
                info!("set trust key - device_config.did: {}",kid);
                let kid = device_config.name.clone();
                self.set_trust_key(kid.as_str(),&key).await;
                info!("set trust key - device_config.name: {}",kid);
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
                    self.set_trust_key(kid.as_str(),&key.unwrap()).await;
                    info!("set trust key - verify-hub");
                }
            } else {
                warn!("NO verfiy-hub publick key, system init with errors!");
            }

            if zone_config.owner.is_some() {
                let owner_key = zone_config.get_default_key();
                let owner_did = zone_config.owner.as_ref().unwrap().clone();
                if owner_key.is_some() {
                    let owner_key = owner_key.unwrap();
                    let owner_public_key = DecodingKey::from_jwk(&owner_key).map_err(|err| {
                        error!("Failed to parse owner_public_key from zone_config: {}",err);
                        RPCErrors::ReasonError(err.to_string())
                    })?;
                    self.set_trust_key("root",&owner_public_key).await;
                    info!("set trust key - root");
                    self.set_trust_key(owner_did.to_string().as_str(),&owner_public_key).await;
                    self.set_trust_key(owner_did.id.clone().as_str(),&owner_public_key).await;
                    info!("update owner_public_key [{}],[{}] to trust keys",owner_did.to_string(),owner_did.id);
                }
            }
        }

        Ok(())
    }

    pub async fn set_trust_key(&self,kid: &str,key: &DecodingKey) -> Result<()> {
        let mut key_map = self.trust_keys.write().await;
        key_map.insert(kid.to_string(),key.clone());
        Ok(())
    }

    //success return (userid,appid)
    pub async fn enforce(&self,req:&RPCRequest, action: &str,resource_path: &str) -> Result<(String,String)> {
        let token = req.token
            .as_ref()
            .map(|token| RPCSessionToken::from_string(token.as_str()))
            .unwrap_or(Err(RPCErrors::ParseRequestError(
                "Invalid params, session_token is none".to_string(),
            )));
        let token = token.unwrap();
        if !token.is_self_verify() {
            return Err(RPCErrors::InvalidToken("Session token is not valid".to_string()));
        }
        let token_str = token.token.as_ref().unwrap();
        let header: jsonwebtoken::Header = jsonwebtoken::decode_header(token_str).map_err(|error| {
            RPCErrors::InvalidToken(format!("JWT decode header error : {}",error))
        })?;

        let key_map = self.trust_keys.read().await;
        let kid = header.kid.unwrap_or("$default".to_string());
        let decoding_key = key_map.get(&kid);
        if decoding_key.is_none() {
            warn!("kid {} not found,Session token is not valid",kid.as_str());
            return Err(RPCErrors::NoPermission(format!("kid {} not found",kid.as_str())));
        }
        let decoding_key = decoding_key.unwrap();

        let validation = Validation::new(header.alg);
        //exp always checked 
        let decoded_token = decode::<serde_json::Value>(token_str, &decoding_key, &validation).map_err(
            |error| RPCErrors::InvalidToken(format!("JWT decode error:{}",error))
        )?;
        let decoded_json = decoded_token.claims.as_object()
            .ok_or(RPCErrors::InvalidToken("Invalid token".to_string()))?;
 
        let userid = decoded_json.get("userid")
            .ok_or(RPCErrors::InvalidToken("Missing userid".to_string()))?;
        let userid = userid.as_str().ok_or(RPCErrors::InvalidToken("Invalid userid".to_string()))?;
        let appid = decoded_json.get("appid").map(|appid| appid.as_str().unwrap_or("kernel"));
        let appid = appid.unwrap_or("kernel");
        let result = rbac::enforce(userid, Some(appid),resource_path,action).await;
        if !result {
            return Err(RPCErrors::NoPermission(format!("enforce failed,userid:{},appid:{},resource:{},action:{}",userid,appid,resource_path,action)));
        }
        Ok((userid.to_string(),appid.to_string()))
    }

    pub async fn enable_zone_provider (is_gateway: bool) -> Result<()> {
        let client = GLOBAL_NAME_CLIENT.get();
        if client.is_none() {
            let mut client = NameClient::new(NameClientConfig::default());
            client.add_provider(Box::new(ZONE_PROVIDER.clone())).await;
            let set_result = GLOBAL_NAME_CLIENT.set(client);
            if set_result.is_err() {
                error!("Failed to set GLOBAL_NAME_CLIENT");
            }
        } else {
            let client = client.unwrap();            
            client.add_provider(Box::new(ZONE_PROVIDER.clone())).await;
        }

        Ok(())
   }

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
            return get_full_appid(self.app_id.as_str(),owner_id);
        } else {
            return self.app_id.clone();
        }
    }


    pub async fn get_session_token(&self) -> String {
        let session_token = self.session_token.read().await;
        session_token.clone()
    }

    pub fn get_data_folder(&self) -> PathBuf {
        match self.runtime_type {
            BuckyOSRuntimeType::AppClient => {
                //返回 
               panic!("AppClient not support get_data_folder");
            }
            BuckyOSRuntimeType::AppService => {
                //返回 
                return self.buckyos_root_dir.join("data").join(self.user_id.clone().unwrap()).join(self.app_id.clone());
            }
            BuckyOSRuntimeType::FrameService | BuckyOSRuntimeType::KernelService => {
                return self.buckyos_root_dir.join("data").join(self.app_id.clone());     //返回 
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
                return self.buckyos_root_dir.join("cache").join(self.user_id.clone().unwrap()).join(self.app_id.clone());
            }
            BuckyOSRuntimeType::FrameService | BuckyOSRuntimeType::KernelService => {
                return self.buckyos_root_dir.join("cache").join(self.app_id.clone());     //返回 
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
                return self.buckyos_root_dir.join("tmp").join(self.user_id.clone().unwrap()).join(self.app_id.clone());
            }
            BuckyOSRuntimeType::FrameService | BuckyOSRuntimeType::KernelService => {
                return self.buckyos_root_dir.join("tmp").join(self.app_id.clone());     
            }
        }
    }

    // 获得与物理逻辑磁盘绑定的本地存储目录，存储的可靠性和特性由物理磁盘决定
    //目录原理上是  disk_id/service_instance_id/
    pub fn get_lcoal_storage_folder(&self,disk_id: Option<String>) -> PathBuf {
        if self.runtime_type == BuckyOSRuntimeType::KernelService || self.runtime_type == BuckyOSRuntimeType::FrameService {
            if disk_id.is_some() {
                let disk_id = disk_id.unwrap();
                return self.buckyos_root_dir.join("local").join(disk_id).join(self.app_id.clone());
            } else {
                return self.buckyos_root_dir.join("local").join(self.app_id.clone());
            }
        } else {
            panic!("This runtime type not support get_lcoal_storage_folder");
        }
    }

    pub fn get_root_pkg_env_path() -> PathBuf {
        get_buckyos_service_local_data_dir("node_daemon",None).join("root_pkg_env")
    }

    fn get_my_settings_path(&self) -> String {
        match self.runtime_type {
            BuckyOSRuntimeType::AppClient => {
                panic!("AppClient not support get_my_settings_path");
            }
            BuckyOSRuntimeType::AppService => {
                format!("users/{}/apps/{}/settings",self.user_id.as_ref().unwrap(),self.app_id.as_str())
            }
            BuckyOSRuntimeType::FrameService | BuckyOSRuntimeType::KernelService => {
                format!("services/{}/settings",self.app_id.as_str())
            }
        }
    }

    pub async fn get_my_settings(&self) -> Result<serde_json::Value> {
        let system_config_client = self.get_system_config_client().await?;
        let settiing_path = self.get_my_settings_path();
        let (settings_str,_version) = system_config_client.get(settiing_path.as_str()).await
            .map_err(|e| {
                error!("get settings failed! err:{}", e);
                RPCErrors::ReasonError(format!("get settings failed! err:{}", e))
            })?;
        let settings : serde_json::Value = serde_json::from_str(settings_str.as_str()).map_err(|e| {
            error!("parse settings failed! err:{}", e);
            RPCErrors::ReasonError(format!("parse settings failed! err:{}", e))
        })?;
        Ok(settings)
    }

    pub async fn update_my_settings(&self,json_path: &str,settings:serde_json::Value) -> Result<()> {
        let system_config_client = self.get_system_config_client().await?;
        let settiing_path = self.get_my_settings_path();
        let settings_str = serde_json::to_string(&settings).map_err(|e| {
            error!("serialize settings failed! err:{}", e);
            RPCErrors::ReasonError(format!("serialize settings failed! err:{}", e))
        })?;

        system_config_client.set_by_json_path(settiing_path.as_str(),json_path,settings_str.as_str()).await
            .map_err(|e| {
                error!("update settings failed! err:{}", e);
                RPCErrors::ReasonError(format!("update settings failed! err:{}", e))
            })?;

        Ok(())
    }

    pub async fn update_all_my_settings(&self,settings:serde_json::Value) -> Result<()> {
        let system_config_client = self.get_system_config_client().await?;
        let settiing_path = self.get_my_settings_path();
        let settings_str = serde_json::to_string(&settings).map_err(|e| {
            error!("serialize settings failed! err:{}", e);
            RPCErrors::ReasonError(format!("serialize settings failed! err:{}", e))
        })?;
        system_config_client.set(settiing_path.as_str(),settings_str.as_str()).await
            .map_err(|e| {
                error!("update settings failed! err:{}", e);
                RPCErrors::ReasonError(format!("update settings failed! err:{}", e))
            })?;
        Ok(())
    }
    
    pub fn get_my_sys_config_path(&self,config_name: &str) -> String {
        match self.runtime_type {
            BuckyOSRuntimeType::AppClient => {
                format!("users/{}/apps/{}/{}",self.user_id.as_ref().unwrap(),self.app_id.as_str(),config_name)
            }
            BuckyOSRuntimeType::AppService => {
                format!("users/{}/apps/{}/{}",self.user_id.as_ref().unwrap(),self.app_id.as_str(),config_name)
            }
            BuckyOSRuntimeType::FrameService | BuckyOSRuntimeType::KernelService => {
                format!("services/{}/{}",self.app_id.as_str(),config_name)
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

    //pub async fn scan_to_build_zoen_boot_info(&self) -> Result<()> {
    //    unimplemented!()
    //}

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
                    url = format!("{}://{}/kapi/system_config",schema,zone_host);
                },
                BuckyOSRuntimeType::AppService => {
                    url = "http://127.0.0.1/kapi/system_config".to_string();
                }
                _ => {
                    //do nothing
                }
            }
        }

        //let url = self.get_zone_service_url("system_config",self.force_https)?;
        let session_token = self.session_token.read().await;
        let client = SystemConfigClient::new(Some(url.as_str()),Some(session_token.as_str()));
        debug!("get system config client OK,url:{}",url);
        Ok(client)
    }

    pub async fn get_task_mgr_client(&self) -> Result<TaskManagerClient> {
        let krpc_client = self.get_zone_service_krpc_client("task-manager").await?;
        let client = TaskManagerClient::new(krpc_client);
        Ok(client)
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

    pub fn get_zone_ndn_base_url(&self) -> String {
        let mut schema = "http";
        if self.force_https {
            schema = "https";
        }
        format!("{}://{}/ndn/",schema,self.zone_id.to_host_name())
    }

    pub async fn get_zone_boot_info(&self) -> Result<ZoneBootInfo> {
        unimplemented!()
    }

    pub async fn get_system_config_url_list(&self,boot_info: &ZoneBootInfo) -> Result<Vec<String>> {
        unimplemented!()
    }

    //if http_only is false, return the url with tunnel protocol
    pub fn get_zone_service_url(&self,service_name: &str,https_only: bool) -> Result<String> {
        let mut schema = "http";
        if https_only {
            schema = "https";
        }

        match self.runtime_type {
            BuckyOSRuntimeType::AppClient => {
                let host_name = self.zone_id.to_host_name();
                return Ok(format!("{}://{}/kapi/{}",schema,host_name,service_name));
            }
            BuckyOSRuntimeType::AppService => {
                //考虑到应用集成SDK的版本问题，为了乡下兼容，总是通过本地的cyfs-gateway去访问其它的service
                //由cyfs-gateway来执行service selector的逻辑
                return Ok(format!("http://127.0.0.1/kapi/{}",service_name));
            }
            BuckyOSRuntimeType::FrameService | BuckyOSRuntimeType::KernelService => {
                //执行service discover逻辑

                let service_port = match service_name {
                    "system_config" => 3200,
                    "verify-hub" => 3300,
                    "repo-service" => 4000,
                    "task-manager" => 3380,
                    _ => {
                        return Err(RPCErrors::ServiceNotValid(service_name.to_string()));
                    }
                };

                return Ok(format!("http://127.0.0.1:{}/kapi/{}",service_port,service_name));
            }
        }
    }

    pub async fn get_zone_service_krpc_client(&self,service_name: &str) -> Result<kRPC> {
        let url = self.get_zone_service_url(service_name,self.force_https)?;
        let session_token = self.session_token.read().await;
        let client = kRPC::new(&url,Some(session_token.clone()));
        Ok(client)
    }   
}
