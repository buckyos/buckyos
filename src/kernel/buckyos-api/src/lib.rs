#![allow(dead_code)]
#![allow(unused)]
mod system_config;
mod sn_client;
mod zone_gateway;
mod task_mgr;
mod control_panel;
mod scheduler_client;
mod verify_hub_client;

use name_lib::{DeviceConfig, DeviceInfo, ZoneConfig};
pub use system_config::*;
pub use sn_client::*;
use tokio::sync::RwLock;
pub use zone_gateway::*;
pub use task_mgr::*;
pub use control_panel::*;
pub use scheduler_client::*;
pub use verify_hub_client::*;
use std::path::PathBuf;
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

//本库以后可能改名叫buckyos-sdk, 
// 通过syc_config_client与buckyos的各种服务交互，与传统OS的system_call类似
#[derive(Debug, Clone)]
pub enum BuckyOSRuntimeType {
    AppClient,    //R3
    AppService,   //R2
    FrameService, //R1 
    KernelService,//R0
}

#[derive(Debug, Clone)]
pub struct BuckyOSRuntime {
    pub appid:String,
    pub runtime_type:BuckyOSRuntimeType,
    pub session_token:Arc<RwLock<String>>,
}

pub struct SystemInfo {

}


static CURRENT_BUCKYOS_RUNTIME:OnceCell<BuckyOSRuntime> = OnceCell::new();
pub static CURRENT_ZONE_CONFIG: OnceCell<ZoneConfig> = OnceCell::new();

pub static INIT_APP_SESSION_TOKEN: OnceCell<String> = OnceCell::new();

pub fn init_global_buckyos_value_by_env(app_id: &str) -> Result<()> {
    let zone_config_str = env::var("BUCKYOS_ZONE_CONFIG");
    if zone_config_str.is_err() {
        warn!("BUCKYOS_ZONE_CONFIG not set");
        return Err(RPCErrors::ReasonError("BUCKYOS_ZONE_CONFIG not set".to_string()));
    }
    let zone_config_str = zone_config_str.unwrap();
    info!("zone_config_str:{}",zone_config_str);    
    let zone_config = serde_json::from_str(zone_config_str.as_str());
    if zone_config.is_err() {
        warn!("zone_config_str format error");
        return Err(RPCErrors::ReasonError("zone_config_str format error".to_string()));
    }
    let zone_config = zone_config.unwrap();
    let set_result = CURRENT_ZONE_CONFIG.set(zone_config);
    if set_result.is_err() {
        warn!("Failed to set GLOBAL_ZONE_CONFIG");
        return Err(RPCErrors::ReasonError("Failed to set GLOBAL_ZONE_CONFIG".to_string()));
    }

    let device_doc = env::var("BUCKYOS_THIS_DEVICE");
    if device_doc.is_err() {
        warn!("BUCKY_DEVICE_DOC not set");
        return Err(RPCErrors::ReasonError("BUCKY_DEVICE_DOC not set".to_string()));
    }
    let device_doc = device_doc.unwrap();
    info!("device_doc:{}",device_doc);
    let device_config= serde_json::from_str(device_doc.as_str());
    if device_config.is_err() {
        warn!("device_doc format error");
        return Err(RPCErrors::ReasonError("device_doc format error".to_string()));
    }
    let device_config:DeviceConfig = device_config.unwrap();
    let set_result = CURRENT_DEVICE_CONFIG.set(device_config);
    if set_result.is_err() {
        warn!("Failed to set CURRENT_DEVICE_CONFIG");
        return Err(RPCErrors::ReasonError("Failed to set CURRENT_DEVICE_CONFIG".to_string()));
    }

    let session_token_key = format!("{}_SESSION_TOKEN",app_id);
    let session_token = env::var(session_token_key.as_str());
    if session_token.is_err() {
        warn!("{} not set",session_token_key);
        return Err(RPCErrors::ReasonError("Failed to set CURRENT_SESSION_TOKEN".to_string()));
    }
    let session_token = session_token.unwrap();
    let set_result = INIT_APP_SESSION_TOKEN.set(session_token);
    if set_result.is_err() {
        warn!("Failed to set CURRENT_APP_SESSION_TOKEN");
        return Err(RPCErrors::ReasonError("Failed to set CURRENT_SESSION_TOKEN".to_string()));
    }

    Ok(())
}


pub async fn init_buckyos_api_runtime(appid:&str,runtime_type:BuckyOSRuntimeType) -> Result<()> {
    if CURRENT_BUCKYOS_RUNTIME.get().is_some() {
        return Err(RPCErrors::ReasonError("BuckyOSRuntime already initialized".to_string()));
    }

    init_global_buckyos_value_by_env(appid)?;
    let runtime = BuckyOSRuntime {
        appid: appid.to_string(),
        runtime_type,
        session_token: Arc::new(RwLock::new(INIT_APP_SESSION_TOKEN.get().unwrap().clone())),
    };
    CURRENT_BUCKYOS_RUNTIME.set(runtime);
    Ok(())
}

pub async fn get_buckyos_api_runtime() -> Result<BuckyOSRuntime> {
    let runtime = CURRENT_BUCKYOS_RUNTIME.get().unwrap();
    Ok(runtime.clone())
}

pub fn enable_zone_provider (
     this_device: Option<&DeviceInfo>,
     session_token: Option<&String>,
     is_gateway: bool) {
    // self.name_query.add_provider(Box::new(ZoneProvider::new(
    //     this_device,
    //     session_token,
    //     is_gateway,
    // )));
    unimplemented!()
}
impl BuckyOSRuntime {
    //login to verify hub. 
    pub async fn login(&self, login_params:Option<Value>,login_config:Option<Value>) -> Result<RPCSessionToken> {
        unimplemented!()
    }

    pub fn get_app_id(&self) -> String {
        self.appid.clone()
    }

    pub async fn get_session_token(&self) -> String {
        let session_token = self.session_token.read().await;
        session_token.clone()
    }

    pub fn get_my_data_folder(&self) -> PathBuf {
        let app_id = self.appid.clone();
        let data_folder = PathBuf::from(format!("data/{}",app_id));
        data_folder
    }

    pub fn get_my_cache_folder(&self) -> PathBuf {
        let app_id = self.appid.clone();
        let cache_folder = PathBuf::from(format!("cache/{}",app_id));
        cache_folder
    }

    pub fn get_my_local_cache_folder(&self) -> PathBuf {
        let app_id = self.appid.clone();
        let cache_folder = PathBuf::from(format!("cache/{}",app_id));
        cache_folder
    }

    pub async fn get_system_info(&self) -> Result<SystemInfo> {
        unimplemented!()
    }

    pub async fn get_my_settings(&self) -> Result<serde_json::Value> {
        unimplemented!()
    }

    pub async fn update_my_settings(&self,json_path: &str,settings:serde_json::Value) -> Result<()> {
        unimplemented!()
    }

    pub async fn update_all_my_settings(&self,settings:serde_json::Value) -> Result<()> {
        unimplemented!()
    }

    pub async fn get_system_config_client(&self) -> Result<SystemConfigClient> {
        let url = self.get_zone_service_url("system_config",true)?;
        let session_token = self.session_token.read().await;
        let client = SystemConfigClient::new(Some(url.as_str()),Some(session_token.as_str()));
        Ok(client)
    }

    pub async fn get_task_mgr_client(&self) -> Result<TaskManagerClient> {
        let krpc_client = self.get_zone_service_krpc_client("task_manager").await?;
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
        let krpc_client = self.get_zone_service_krpc_client("verify_hub").await?;
        let client = VerifyHubClient::new(krpc_client);
        Ok(client)
    }

    //if http_only is false, return the url with tunnel protocol
    pub fn get_zone_service_url(&self,service_name: &str,http_only: bool) -> Result<String> {
        unimplemented!()
    }

    pub async fn get_zone_service_krpc_client(&self,service_name: &str) -> Result<kRPC> {
        let url = self.get_zone_service_url(service_name,true)?;
        let session_token = self.session_token.read().await;
        let client = kRPC::new(&url,Some(session_token.clone()));
        Ok(client)
    }   
}

