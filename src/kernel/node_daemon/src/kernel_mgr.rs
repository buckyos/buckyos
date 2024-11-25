use crate::run_item::*;
use async_trait::async_trait;
use buckyos_kit::*;
use jsonwebtoken::{DecodingKey, EncodingKey};
use log::*;
use name_lib::DeviceConfig;
use package_lib::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::hash::Hash;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use tokio::sync::RwLock;

#[derive(Serialize, Deserialize)]
pub struct KernelServiceConfig {
    pub target_state: RunItemTargetState,
    pub pkg_id: String,
    pub operations: HashMap<String, RunItemControlOperation>,
}

pub struct KernelServiceRunItem {
    pub target_state: RunItemTargetState,
    pub pkg_id: String,
    service_pkg: RwLock<Option<ServicePkg>>,
    device_doc: DeviceConfig,
    device_private_key: EncodingKey,
}

impl KernelServiceRunItem {
    pub fn new(
        kernel_config: &KernelServiceConfig,
        device_doc: &DeviceConfig,
        device_private_key: &EncodingKey,
    ) -> Self {
        Self {
            target_state: kernel_config.target_state.clone(),
            pkg_id: kernel_config.pkg_id.clone(),
            service_pkg: RwLock::new(None),
            device_doc: device_doc.clone(),
            device_private_key: device_private_key.clone(),
        }
    }
}

#[async_trait]
impl RunItemControl for KernelServiceRunItem {
    fn get_item_name(&self) -> Result<String> {
        Ok(self.pkg_id.clone())
    }

    async fn deploy(&self, params: Option<&Vec<String>>) -> Result<()> {
        //check already have deploy task ?
        //create deploy task
        //install  or upgrade pkg
        //call pkg.deploy() scrpit or 由pkg在自己的start脚本里管理？
        unimplemented!();
    }

    async fn start(&self, control_key: &EncodingKey, params: Option<&Vec<String>>) -> Result<()> {
        let service_pkg = self.service_pkg.read().await;
        if service_pkg.is_some() {
            let timestamp = buckyos_get_unix_timestamp();
            let device_session_token = kRPC::RPCSessionToken {
                token_type: kRPC::RPCSessionTokenType::JWT,
                nonce: None,
                userid: Some(self.device_doc.name.clone()),
                appid: Some("kernel".to_string()),
                exp: Some(timestamp + 3600 * 24 * 7),
                iss: Some(self.device_doc.name.clone()),
                token: None,
            };

            let device_session_token_jwt = device_session_token
                .generate_jwt(Some(self.device_doc.did.clone()), &self.device_private_key)
                .map_err(|err| {
                    error!("generate session token for {} failed! {}", self.pkg_id, err);
                    return ControlRuntItemErrors::ExecuteError(
                        "start".to_string(),
                        err.to_string(),
                    );
                })?;

            let upper_name = self.pkg_id.to_uppercase();
            let env_key = format!("{}_SESSION_TOKEN", upper_name);
            std::env::set_var(env_key.as_str(), device_session_token_jwt);

            let result = service_pkg
                .as_ref()
                .unwrap()
                .start(params)
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
        let service_pkg = self.service_pkg.read().await;
        if service_pkg.is_some() {
            let result = service_pkg
                .as_ref()
                .unwrap()
                .stop(None)
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

        {
            let service_pkg = self.service_pkg.read().await;
            if service_pkg.is_none() {
                need_load_pkg = true;
            } else {
                //TODO:还要比较准确版本是否符合现在的“pkg_id要求”
                let result_state =
                    service_pkg
                        .as_ref()
                        .unwrap()
                        .status(None)
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
            let mut service_pkg =
                ServicePkg::new(self.pkg_id.clone(), get_buckyos_system_bin_dir());
            let load_result = service_pkg.load().await;
            if load_result.is_ok() {
                let mut new_service_pkg = self.service_pkg.write().await;
                let result = service_pkg.status(None).await.map_err(|err| {
                    return ControlRuntItemErrors::ExecuteError(
                        "get_state".to_string(),
                        err.to_string(),
                    );
                })?;
                *new_service_pkg = Some(service_pkg);
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
