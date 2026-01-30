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
    pub service_name: String,
    pub pkg_id: String,
    service_pkg: ServicePkg,
}

impl KernelServiceRunItem {
    pub fn new(
        app_id: &str,
        kernel_config: &KernelServiceInstanceConfig
    ) -> Self {
        let pkg_name = kernel_config
            .service_sepc
            .service_doc
            .name
            .clone();
        let service_pkg = ServicePkg::new(pkg_name.clone(), 
        get_buckyos_system_bin_dir());
        Self {
            service_name: app_id.to_string(),
            pkg_id: pkg_name,
            service_pkg: service_pkg,
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

    async fn start(&self, params: Option<&Vec<String>>) -> Result<()> {
        let timestamp = buckyos_get_unix_timestamp();
        let app_id = self.service_name.clone();
        let runtime = get_buckyos_api_runtime().unwrap();
        let device_doc = runtime.device_config.as_ref().unwrap();
        let device_private_key = runtime.device_private_key.as_ref().unwrap();
        let device_session_token = kRPC::RPCSessionToken {
            token_type: kRPC::RPCSessionTokenType::Normal,
            appid: Some(app_id.clone()),
            jti: Some(timestamp.to_string()),
            session: None,
            sub: Some(device_doc.name.clone()),
            aud: None,
            exp: Some(timestamp + VERIFY_HUB_TOKEN_EXPIRE_TIME*2),
            iss: Some(device_doc.name.clone()),
            token: None,
            extra: HashMap::new(),
        };

        let device_session_token_jwt = device_session_token
            .generate_jwt(None, device_private_key)
            .map_err(|err| {
                error!("generate session token for {} failed! {}", self.pkg_id, err);
                return ControlRuntItemErrors::ExecuteError(
                    "start".to_string(),
                    err.to_string(),
                );
            })?;

        let env_key = get_session_token_env_key(&self.service_name,false);
        unsafe {
            std::env::set_var(env_key.as_str(), device_session_token_jwt);
        }

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

    async fn get_state(&self, params: Option<&Vec<String>>) -> Result<ServiceInstanceState> {
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
