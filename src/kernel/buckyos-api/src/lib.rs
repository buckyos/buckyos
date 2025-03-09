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
use name_client::*;
use zone_provider::*;
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
    pub owner_user_id:Option<String>,
    pub buckyos_root_dir:PathBuf,
    pub runtime_type:BuckyOSRuntimeType,
    pub session_token:Arc<RwLock<String>>,
    pub zone_config:Option<ZoneConfig>,
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
    let upper_appid = app_id.to_uppercase();
    let session_token_key = format!("{}_SESSION_TOKEN",upper_appid);
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


pub async fn init_buckyos_api_runtime(appid:&str,owner_user_id:Option<String>,runtime_type:BuckyOSRuntimeType) -> Result<()> {
    if CURRENT_BUCKYOS_RUNTIME.get().is_some() {
        return Err(RPCErrors::ReasonError("BuckyOSRuntime already initialized".to_string()));
    }

    match runtime_type {
        BuckyOSRuntimeType::AppClient | BuckyOSRuntimeType::AppService => {
            if owner_user_id.is_none() {
                return Err(RPCErrors::ReasonError("owner_user_id is required for AppClient or AppService".to_string()));
            }
        }
        _ => {
            //do nothing
        }
    }

    init_global_buckyos_value_by_env(appid)?;
    let zone_config = CURRENT_ZONE_CONFIG.get().unwrap();
    let runtime = BuckyOSRuntime {
        appid: appid.to_string(),
        owner_user_id,
        runtime_type,
        session_token: Arc::new(RwLock::new(INIT_APP_SESSION_TOKEN.get().unwrap().clone())),
        buckyos_root_dir: get_buckyos_root_dir(),
        zone_config: Some(zone_config.clone()),
    };
    CURRENT_BUCKYOS_RUNTIME.set(runtime);
    Ok(())
}

pub fn get_buckyos_api_runtime() -> Result<BuckyOSRuntime> {
    let runtime = CURRENT_BUCKYOS_RUNTIME.get().unwrap();
    Ok(runtime.clone())
}


impl BuckyOSRuntime {
    //login to verify hub. 
    pub async fn login(&mut self, login_params:Option<Value>,login_config:Option<Value>) -> Result<RPCSessionToken> {
        let real_session_token; 
        match self.runtime_type {
            BuckyOSRuntimeType::AppClient => {
                unimplemented!()
            }
            _ => {
                let current_session_token = self.session_token.read().await;
                if current_session_token.is_empty() {
                    return Err(RPCErrors::ReasonError("Session token not exists".to_string()));
                } 
                real_session_token = RPCSessionToken::from_string(current_session_token.clone().as_str())?;
                drop(current_session_token);
            }
        }
       
        let control_panel_client = self.get_control_panel_client().await?;
        let zone_config = control_panel_client.load_zone_config().await?;
        //self.zone_config = Some(zone_config);
        //CURRENT_ZONE_CONFIG.set(self.zone_config.clone().unwrap());

        Ok(real_session_token)
    }

    pub async fn enable_zone_provider (is_gateway: bool) -> Result<()> {
        let client = GLOBAL_NAME_CLIENT.get();
        if client.is_none() {
            let mut client = NameClient::new(NameClientConfig::default());
            client.add_provider(Box::new(ZoneProvider::new(is_gateway))).await;
            let set_result = GLOBAL_NAME_CLIENT.set(client);
            if set_result.is_err() {
                error!("Failed to set GLOBAL_NAME_CLIENT");
            }
        } else {
            let client = client.unwrap();            
            client.add_provider(Box::new(ZoneProvider::new(is_gateway))).await;
        }
        Ok(())
   }

    pub fn get_app_id(&self) -> String {
        self.appid.clone()
    }

    pub fn get_owner_user_id(&self) -> Option<String> {
        self.owner_user_id.clone()
    }

    pub async fn get_session_token(&self) -> String {
        let session_token = self.session_token.read().await;
        session_token.clone()
    }

    pub async fn get_system_info(&self) -> Result<SystemInfo> {
        unimplemented!()
    }


    pub fn get_my_data_folder(&self) -> PathBuf {
        match self.runtime_type {
            BuckyOSRuntimeType::AppClient => {
                //返回 
               unimplemented!()
            }
            BuckyOSRuntimeType::AppService => {
                //返回 
                return self.buckyos_root_dir.join("data").join(self.owner_user_id.clone().unwrap()).join(self.appid.clone());
            }
            BuckyOSRuntimeType::FrameService | BuckyOSRuntimeType::KernelService => {
                return self.buckyos_root_dir.join("data").join(self.appid.clone());     //返回 
            }
        }
    }

    pub fn get_my_cache_folder(&self) -> PathBuf {
        match self.runtime_type {
            BuckyOSRuntimeType::AppClient => {
                //返回 
               unimplemented!()
            }
            BuckyOSRuntimeType::AppService => {
                //返回 
                return self.buckyos_root_dir.join("cache").join(self.owner_user_id.clone().unwrap()).join(self.appid.clone());
            }
            BuckyOSRuntimeType::FrameService | BuckyOSRuntimeType::KernelService => {
                return self.buckyos_root_dir.join("cache").join(self.appid.clone());     //返回 
            }
        }
    }

    pub fn get_my_local_cache_folder(&self) -> PathBuf {
        match self.runtime_type {
            BuckyOSRuntimeType::AppClient => {
                //返回 
               unimplemented!()
            }
            BuckyOSRuntimeType::AppService => {
                //返回 
                return self.buckyos_root_dir.join("tmp").join(self.owner_user_id.clone().unwrap()).join(self.appid.clone());
            }
            BuckyOSRuntimeType::FrameService | BuckyOSRuntimeType::KernelService => {
                return self.buckyos_root_dir.join("tmp").join(self.appid.clone());     //返回 
            }
        }
    }

    // 获得与物理逻辑磁盘绑定的本地存储目录，存储的可靠性和特性由物理磁盘决定
    //目录原理上是  disk_id/service_instance_id/
    pub fn get_lcoal_storage_folder(&self,disk_id: &str) -> PathBuf {
        unimplemented!()
    }

    pub fn get_root_pkg_env_path() -> PathBuf {
        get_buckyos_service_local_data_dir("node_daemon",None).join("root_pkg_env")
    }

    fn get_my_settings_path(&self) -> String {
        match self.runtime_type {
            BuckyOSRuntimeType::AppClient => {
                unimplemented!()
            }
            BuckyOSRuntimeType::AppService => {
                format!("users/{}/apps/{}/settings",self.owner_user_id.as_ref().unwrap(),self.appid.as_str())
            }
            BuckyOSRuntimeType::FrameService | BuckyOSRuntimeType::KernelService => {
                format!("services/{}/settings",self.appid.as_str())
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
                format!("users/{}/apps/{}/{}",self.owner_user_id.as_ref().unwrap(),self.appid.as_str(),config_name)
            }
            BuckyOSRuntimeType::AppService => {
                format!("users/{}/apps/{}/{}",self.owner_user_id.as_ref().unwrap(),self.appid.as_str(),config_name)
            }
            BuckyOSRuntimeType::FrameService | BuckyOSRuntimeType::KernelService => {
                format!("services/{}/{}",self.appid.as_str(),config_name)
            }
        }
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
        match service_name {
            "system_config" => {
                return Ok("http://127.0.0.1:3200/kapi/system_config".to_string());
            }
            "verify_hub" => {
                return Ok("http://127.0.0.1:3300/kapi/verify_hub".to_string());
            }
            "repo_server" | "repo" => {
                return Ok("http://127.0.0.1:4000/kapi/repo".to_string());
            }
            "task_manager" => {
                return Ok("http://127.0.0.1:3380/kapi/task_manager".to_string());
            }
            
            _ => Err(RPCErrors::ServiceNotValid(service_name.to_string())),
        }
    }

    pub async fn get_zone_service_krpc_client(&self,service_name: &str) -> Result<kRPC> {
        let url = self.get_zone_service_url(service_name,true)?;
        let session_token = self.session_token.read().await;
        let client = kRPC::new(&url,Some(session_token.clone()));
        Ok(client)
    }   
}

