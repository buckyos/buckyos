use async_trait::async_trait;
use jsonwebtoken::{DecodingKey, EncodingKey};
use log::*;
use name_lib::DeviceConfig;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::hash::Hash;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use tokio::sync::RwLock;

use crate::run_item::*;
use buckyos_kit::*;
use package_lib::*;

use sys_config::AppServiceConfig;


pub struct AppRunItem {
    pub app_id: String,
    pub app_service_config: AppServiceConfig,
    pub app_loader: RwLock<Option<ServicePkg>>,
    device_doc: DeviceConfig,
    device_private_key: EncodingKey,
}

impl AppRunItem {
    pub fn new(
        app_id: &String,
        app_service_config: AppServiceConfig,
        device_doc: &DeviceConfig,
        device_private_key: &EncodingKey,
    ) -> Self {
        AppRunItem {
            app_id: app_id.clone(),
            app_service_config: app_service_config,
            app_loader: RwLock::new(None),
            device_doc: device_doc.clone(),
            device_private_key: device_private_key.clone(),
        }
    }
}

#[async_trait]
impl RunItemControl for AppRunItem {
    fn get_item_name(&self) -> Result<String> {
        //appid#userid
        Ok(self.app_id.clone())
    }

    async fn deploy(&self, params: Option<&Vec<String>>) -> Result<()> {
        //check already have deploy task ?
        //create deploy task
        //install  or upgrade pkg
        //call pkg.deploy() scrpit 不要调用，由pkg在自己的start脚本里管理？
        unimplemented!();
    }

    async fn start(&self, control_key: &EncodingKey, params: Option<&Vec<String>>) -> Result<()> {
        let app_loader = self.app_loader.read().await;
        if app_loader.is_some() {
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
            let full_appid = format!("{}#{}", self.app_service_config.user_id, self.app_id);
            let env_key = format!("{}.token", full_appid.as_str());
            std::env::set_var(env_key.as_str(), device_session_token_jwt);
            let app_config_str = serde_json::to_string(&self.app_service_config).unwrap();
            std::env::set_var(format!("{}.config", full_appid.as_str()), app_config_str);

            let real_param = vec![self.app_id.clone(), self.app_service_config.user_id.clone()];
            let result = app_loader
                .as_ref()
                .unwrap()
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
        return Err(ControlRuntItemErrors::ExecuteError(
            "start".to_string(),
            "failed".to_string(),
        ));
    }
    async fn stop(&self, params: Option<&Vec<String>>) -> Result<()> {
        let app_loader = self.app_loader.read().await;
        if app_loader.is_some() {
            let real_param = vec![self.app_id.clone(), self.app_service_config.user_id.clone()];
            let result = app_loader
                .as_ref()
                .unwrap()
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
        return Err(ControlRuntItemErrors::ExecuteError(
            "stop".to_string(),
            "failed".to_string(),
        ));
    }

    async fn get_state(&self, params: Option<&Vec<String>>) -> Result<ServiceState> {
        let mut need_load_pkg = false;
        let real_param = vec![self.app_id.clone(), self.app_service_config.user_id.clone()];
        {
            let app_loader = self.app_loader.read().await;
            if app_loader.is_none() {
                need_load_pkg = true;
            } else {
                let result_state = app_loader
                    .as_ref()
                    .unwrap()
                    .status(Some(&real_param))
                    .await
                    .map_err(|err| {
                        return ControlRuntItemErrors::ExecuteError(
                            "get_state".to_string(),
                            err.to_string(),
                        );
                    })?;
                return Ok(result_state);
            }
        }

        if need_load_pkg {
            let mut app_loader =
                ServicePkg::new("app_loader".to_string(), get_buckyos_system_bin_dir());
            let load_result = app_loader.load().await;
            if load_result.is_ok() {
                let mut new_app_loader = self.app_loader.write().await;
                let result = app_loader.status(Some(&real_param)).await.map_err(|err| {
                    return ControlRuntItemErrors::ExecuteError(
                        "get_state".to_string(),
                        err.to_string(),
                    );
                })?;
                *new_app_loader = Some(app_loader);
                return Ok(result);
            } else {
                return Ok(ServiceState::NotExist);
            }
        } else {
            //deead path
            warn!("DEAD PATH,never enter here");
            return Err(ControlRuntItemErrors::ExecuteError(
                "get_state".to_string(),
                "dead path".to_string(),
            ));
        }
    }
}
