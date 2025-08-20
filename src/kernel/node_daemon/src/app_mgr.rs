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
use buckyos_api::*;
use package_lib::*;
use crate::run_item::*;
use crate::service_pkg::*;

//use package_installer::*;
use buckyos_api::{get_full_appid, get_session_token_env_key, AppServiceInstanceConfig};

// 核心逻辑
// 非docker模式逻辑与标准的service item一致，但脚本调用是由app_loader来完成
// docker模式下
// 1. 通过app_loader的status脚本来判断是否存在（以镜像是否存在未标准）
// 2. 不存在，则要求app_loader安装镜像（可以指定media_info)
// 3. 由app_loader的start脚本来创建容器，创建的过程中可能会导入镜像
pub struct AppRunItem {
    pub app_id: String,
    pub app_service_config: AppServiceInstanceConfig,
    pub app_loader: ServicePkg,

}

impl AppRunItem {
    pub fn new(
        app_id: &String,
        app_service_config: AppServiceInstanceConfig,
        app_loader: ServicePkg
    ) -> Self {
        AppRunItem {
            app_id: app_id.clone(),
            app_service_config: app_service_config,
            app_loader: app_loader,

        }
    }

    fn get_instance_pkg_id(&self,is_strict: bool) -> Result<String> {
        if self.app_service_config.docker_image_pkg_id.is_some() {
            if !is_strict {
                let simple_name = PackageId::get_pkg_id_simple_name(self.app_service_config.docker_image_pkg_id.as_ref().unwrap());
                return Ok(simple_name);
            } else {
                return Ok(self.app_service_config.docker_image_pkg_id.as_ref().unwrap().clone());
            }
        }

        if self.app_service_config.app_pkg_id.is_some() {
            if !is_strict {
                let simple_name = PackageId::get_pkg_id_simple_name(self.app_service_config.app_pkg_id.as_ref().unwrap());
                return Ok(simple_name);
            } else {
                return Ok(self.app_service_config.app_pkg_id.as_ref().unwrap().clone());
            }
        } 

        Err(ControlRuntItemErrors::PkgNotExist(
            self.app_loader.pkg_id.clone(),
        ))
    }

    async fn set_env_var(&self,_is_system_app:bool) -> Result<()> {
        //if self.app_service_config.app_pkg_id.is_some() {
        let env = PackageEnv::new(get_buckyos_system_bin_dir());
        let instance_pkg_id = self.get_instance_pkg_id(env.is_strict());
        if instance_pkg_id.is_ok() {
            let instance_pkg_id = instance_pkg_id.unwrap();
            let app_pkg = env.load(instance_pkg_id.as_str()).await;
            if app_pkg.is_ok() {
                let app_pkg = app_pkg.unwrap();
                let media_info_json = json!({
                    "pkg_id": instance_pkg_id,
                    "full_path": app_pkg.full_path.to_string_lossy(),
                });
                let media_info_json_str = media_info_json.to_string();
                    std::env::set_var("app_media_info", media_info_json_str);
            }
        }

        let app_config_str = serde_json::to_string(&self.app_service_config).unwrap();
        std::env::set_var("app_instance_config",app_config_str);
        
        let timestamp = buckyos_get_unix_timestamp();
        let runtime = get_buckyos_api_runtime().unwrap();
        let device_doc = runtime.device_config.as_ref().unwrap();
        let device_private_key = runtime.device_private_key.as_ref().unwrap();
        let app_service_session_token = kRPC::RPCSessionToken {
            token_type: kRPC::RPCSessionTokenType::JWT,
            nonce: None,
            session: None,
            userid: Some(self.app_service_config.user_id.clone()),
            appid: Some(self.app_id.clone()),
            exp: Some(timestamp + VERIFY_HUB_TOKEN_EXPIRE_TIME*2),
            iss: Some(device_doc.name.clone()),
            token: None,
        };

        let app_service_session_token_jwt = app_service_session_token
            .generate_jwt(Some(device_doc.name.clone()), device_private_key)
            .map_err(|err| {
                error!("generate session token for {} failed! {}", self.app_id, err);
                return ControlRuntItemErrors::ExecuteError(
                    "start".to_string(),
                    err.to_string(),
                );
            })?;
        let full_appid = get_full_appid(&self.app_id, &self.app_service_config.user_id);
        let env_key = get_session_token_env_key(&full_appid,true);
        std::env::set_var(env_key.as_str(), app_service_session_token_jwt);
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
        let is_system_app = self.app_service_config.app_pkg_id.is_some();

        let mut env = PackageEnv::new(get_buckyos_system_bin_dir());
        let instance_pkg_id = self.get_instance_pkg_id(env.is_strict())?;
        info!("install app pkg {}",instance_pkg_id);
        let install_result = env.install_pkg(&instance_pkg_id, true,false).await
            .map_err(|e| {
                error!("AppRunItem install pkg {} failed! {}", self.app_id, e);
                return ControlRuntItemErrors::ExecuteError(
                    "deploy".to_string(),
                    e.to_string(),
                );
            });

        if install_result.is_ok() {
            warn!("install app instance pkg {} success",instance_pkg_id);
        }

        if !is_system_app {
            self.set_env_var(false).await?;
            let real_param = vec![self.app_id.clone(), self.app_service_config.user_id.clone()];
            let result = self.app_loader.execute_operation("deploy",Some(&real_param)).await.map_err(|err| {
                return ControlRuntItemErrors::ExecuteError(
                    "deploy".to_string(),
                    err.to_string(),
                );
            });
            if result.is_ok() {
                if result.unwrap() == 0 {
                    info!("deploy app (not system) {} by app_loader success",self.app_id);
                    return Ok(());
                }
            }
            Ok(())
        } else {
            if install_result.is_ok() {
                return Ok(());
            } else {
                return Err(install_result.err().unwrap());
            }
        }
    }

    async fn start(&self, params: Option<&Vec<String>>) -> Result<()> {
        //TODO
        if self.app_service_config.app_pkg_id.is_some() {
            self.set_env_var(true).await?;
        } else {
            self.set_env_var(false).await?;
        }
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
        if self.app_service_config.app_pkg_id.is_some() {
            self.set_env_var(true).await?;
        } else {
            self.set_env_var(false).await?;
        }
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
        let is_system_app;
        if self.app_service_config.app_pkg_id.is_some() {
            let env = PackageEnv::new(get_buckyos_system_bin_dir());
            let instance_pkg_id = self.get_instance_pkg_id(env.is_strict())?;
            info!("state system app,will load dapp's app_pkg {}",instance_pkg_id.as_str());
            let app_pkg = env.load(instance_pkg_id.as_str()).await;
            if app_pkg.is_err() {
                return Ok(ServiceState::NotExist);
            }
            is_system_app = true;
        } else {

            is_system_app = false;
        }  
        
        self.set_env_var(is_system_app).await?;
        let real_param = vec![self.app_id.clone(), self.app_service_config.user_id.clone()];
        let result: ServiceState = self.app_loader.status(Some(&real_param)).await.map_err(|err| {
            return ControlRuntItemErrors::ExecuteError(
                "get_state".to_string(),
                err.to_string(),
            );
        })?;

        Ok(result)
    }


}
