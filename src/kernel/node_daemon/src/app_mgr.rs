use async_trait::async_trait;
use jsonwebtoken::{DecodingKey, EncodingKey};
use log::*;
use name_lib::DeviceConfig;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::hash::Hash;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use tokio::sync::RwLock;
use buckyos_kit::*;
use package_lib::*;
use crate::run_item::*;
use crate::service_pkg::*;

//use package_installer::*;
use sys_config::AppServiceInstanceConfig;


pub struct AppRunItem {
    pub app_id: String,
    pub app_service_config: AppServiceInstanceConfig,
    pub app_loader: ServicePkg,
    device_doc: DeviceConfig,
    device_private_key: EncodingKey,
}

impl AppRunItem {
    pub fn new(
        app_id: &String,
        app_service_config: AppServiceInstanceConfig,
        app_loader: ServicePkg,
        device_doc: &DeviceConfig,
        device_private_key: &EncodingKey,
    ) -> Self {
        AppRunItem {
            app_id: app_id.clone(),
            app_service_config: app_service_config,
            app_loader: app_loader,
            device_doc: device_doc.clone(),
            device_private_key: device_private_key.clone(),
        }
    }

    fn get_app_pkg_id(&self) -> Result<String> {
        if self.app_service_config.image_pkg_id.is_some() {
            return Ok(self.app_service_config.image_pkg_id.as_ref().unwrap().clone());
        }
        Err(ControlRuntItemErrors::PkgNotExist(
            self.app_loader.pkg_id.clone(),
        ))
    }

    async fn set_env_var(&self) -> Result<()> {
        let app_pkg_id = self.get_app_pkg_id()?;
        let env = PackageEnv::new(get_buckyos_system_bin_dir());
        let app_pkg = env.load(app_pkg_id.as_str()).await;
        if app_pkg.is_err() {
            return Err(ControlRuntItemErrors::PkgNotExist(app_pkg_id));
        }
        let app_pkg = app_pkg.unwrap();
        let media_info_json = json!({
            "pkg_id": app_pkg_id,
            "full_path": app_pkg.full_path.to_string_lossy(),
        });
        let media_info_json_str = media_info_json.to_string();
        std::env::set_var("app_media_info", media_info_json_str);

        let app_config_str = serde_json::to_string(&self.app_service_config).unwrap();
        std::env::set_var("app_instance_config",app_config_str);
        
        let timestamp = buckyos_get_unix_timestamp();
        let device_session_token = kRPC::RPCSessionToken {
            token_type: kRPC::RPCSessionTokenType::JWT,
            nonce: None,
            userid: Some(self.app_service_config.user_id.clone()),
            appid: Some(self.app_id.clone()),
            exp: Some(timestamp + 3600 * 24 * 7),
            iss: Some(self.device_doc.name.clone()),
            token: None,
        };

        let device_session_token_jwt = device_session_token
            .generate_jwt(Some(self.device_doc.did.clone()), &self.device_private_key)
            .map_err(|err| {
                error!("generate session token for {} failed! {}", self.app_id, err);
                return ControlRuntItemErrors::ExecuteError(
                    "start".to_string(),
                    err.to_string(),
                );
            })?;
        let full_appid = format!("{}#{}", self.app_id,self.app_service_config.user_id);
        let env_key = format!("{}_token", full_appid.as_str());
        std::env::set_var(env_key.as_str(), device_session_token_jwt);

        Ok(())
    }
}

#[async_trait]
impl RunItemControl for AppRunItem {
    fn get_item_name(&self) -> Result<String> {
        //appid#userid
        let full_appid = format!("{}#{}", self.app_service_config.user_id, self.app_id);
        Ok(full_appid)
    }

    async fn deploy(&self, params: Option<&Vec<String>>) -> Result<()> {
        let app_pkg_id = self.get_app_pkg_id()?;
        let env = PackageEnv::new(get_buckyos_system_bin_dir());
        let pkg_meta = env.get_pkg_meta(app_pkg_id.as_str()).await;
        if pkg_meta.is_err() {
            return Err(ControlRuntItemErrors::PkgNotExist(app_pkg_id));
        }
        let (meta_obj_id,pkg_meta) = pkg_meta.unwrap();

        warn!("deploy app {}",app_pkg_id);
        let repo_url = "http://127.0.0.1:8080/repo";
        //TODO:由install流程管理去重和断点续传,这个去重通常是跨进程的
        // let deps = Installer::install(app_pkg_id.as_str(), &PathBuf::from(get_buckyos_system_bin_dir()), repo_url, None)
        //     .await
        //     .map_err(|e| {
        //         error!("Failed to call install package, err:{:?}", e);
        //         return ControlRuntItemErrors::ExecuteError(
        //             "deploy".to_string(),
        //             e.to_string(),
        //         );
        //     })?;
        warn!("install app {} success",app_pkg_id);
        Ok(())
    }

    async fn start(&self, control_key: &EncodingKey, params: Option<&Vec<String>>) -> Result<()> {
        self.set_env_var().await?;
        let real_param = vec![self.app_id.clone(), self.app_service_config.user_id.clone()];

        let result = self.app_loader
            .start(Some(&real_param))
            .await
            .map_err(|err| {
                return ControlRuntItemErrors::ExecuteError(
                    "start".to_string(),
                    err.to_string(),
                );
            })?;

        if result == 0 {
            return Ok(());
        } else {
            return Err(ControlRuntItemErrors::ExecuteError(
                "start".to_string(),
                "failed".to_string(),
            ));
        }
    }

    
    async fn stop(&self, params: Option<&Vec<String>>) -> Result<()> {
        self.set_env_var().await?;
        let real_param = vec![self.app_id.clone(), self.app_service_config.user_id.clone()];
        
        let result = self.app_loader
            .stop(Some(&real_param))
            .await
            .map_err(|err| {
                return ControlRuntItemErrors::ExecuteError(
                    "stop".to_string(),
                    err.to_string(),
                );
            })?;
        if result == 0 {
            return Ok(());
        } else {
            return Err(ControlRuntItemErrors::ExecuteError(
                "stop".to_string(),
                "failed".to_string(),
            ));
        }
    }

    async fn get_state(&self, params: Option<&Vec<String>>) -> Result<ServiceState> {
        let app_pkg_id = self.get_app_pkg_id()?;
        let env = PackageEnv::new(get_buckyos_system_bin_dir());
        let app_pkg = env.load(app_pkg_id.as_str()).await;
        if app_pkg.is_err() {
            return Ok(ServiceState::NotExist);
        }
        
        self.set_env_var().await?;
        let real_param = vec![self.app_id.clone(), self.app_service_config.user_id.clone()];

        let result = self.app_loader.status(Some(&real_param)).await.map_err(|err| {
            return ControlRuntItemErrors::ExecuteError(
                "get_state".to_string(),
                err.to_string(),
            );
        })?;

        Ok(result)
    }


}
