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
use buckyos_api::*;
use crate::service_pkg::*;
//use package_installer::*;

use crate::run_item::*;

pub struct KernelServiceRunItem {
    pub target_state: RunItemTargetState,
    pub pkg_id: String,
    service_pkg: ServicePkg,
    device_doc: DeviceConfig,
    device_private_key: EncodingKey,
}

impl KernelServiceRunItem {
    pub fn new(
        kernel_config: &KernelServiceInstanceConfig,
        device_doc: &DeviceConfig,
        device_private_key: &EncodingKey,
    ) -> Self {
        let service_pkg = ServicePkg::new(kernel_config.pkg_id.clone(), 
        get_buckyos_system_bin_dir());
        Self {
            target_state: RunItemTargetState::from_str(&kernel_config.target_state.as_str()).unwrap(),
            pkg_id: kernel_config.pkg_id.clone(),
            service_pkg: service_pkg,
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
        //这个逻辑是不区分新装和升级的
        let mut pkg_env = PackageEnv::new(get_buckyos_system_bin_dir());
        pkg_env.install_pkg(&self.pkg_id, true,false).await
            .map_err(|e| {
                error!("KernelServiceRunItem install pkg {} failed! {}", self.pkg_id, e);
                return ControlRuntItemErrors::ExecuteError(
                    "deploy".to_string(),
                    e.to_string(),
                );
            })?;

        warn!("install kernel service {} success",self.pkg_id);
        Ok(())
        
    }

    async fn start(&self, control_key: &EncodingKey, params: Option<&Vec<String>>) -> Result<()> {
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
            .generate_jwt(Some(self.device_doc.name.clone()), &self.device_private_key)
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

        let result = self.service_pkg
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

    
    async fn stop(&self, params: Option<&Vec<String>>) -> Result<()> {
        let result = self.service_pkg
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

    async fn get_state(&self, params: Option<&Vec<String>>) -> Result<ServiceState> {
        let result = self.service_pkg
            .status(None)
            .await
            .map_err(|err| {
                return ControlRuntItemErrors::ExecuteError(
                    "get_state".to_string(),
                    err.to_string(),
                );
            })?;
        Ok(result)
    }
}
