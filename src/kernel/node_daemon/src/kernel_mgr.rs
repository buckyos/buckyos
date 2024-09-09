
use async_trait::async_trait;
use jsonwebtoken::{DecodingKey, EncodingKey};
use log::*;
use serde_json::Value;
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use std::hash::Hash;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use tokio::sync::RwLock;
use crate::run_item::*;
use package_manager::*;
use buckyos_kit::*;

#[derive(Serialize, Deserialize)]
pub struct KernelServiceConfig {
    pub target_state : RunItemTargetState,
    pub pkg_id : String,

    #[serde(skip)]
    service_pkg : RwLock<Option<ServicePkg>>,
}

impl KernelServiceConfig {

}

#[async_trait]
impl RunItemControl for KernelServiceConfig {
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

    async fn start(&self, control_key:&EncodingKey,params: Option<&Vec<String>>) -> Result<()> {
        let service_pkg = self.service_pkg.read().await;
        if service_pkg.is_some() {
            let result = service_pkg.as_ref().unwrap().start(params).await.map_err(|err| {
                return ControlRuntItemErrors::ExecuteError("start".to_string(), err.to_string());
            })?;

            if result == 0 {
                return Ok(());
            } else {
                return Err(ControlRuntItemErrors::ExecuteError("start".to_string(), "failed".to_string()));
            }
        }
        return Err(ControlRuntItemErrors::ExecuteError("start".to_string(), "failed".to_string()));
    }
    async fn stop(&self, params:Option<&Vec<String>>) -> Result<()> {
        let service_pkg = self.service_pkg.read().await;
        if service_pkg.is_some() {
            let result = service_pkg.as_ref().unwrap().stop().await.map_err(|err| {
                return ControlRuntItemErrors::ExecuteError("stop".to_string(), err.to_string());
            })?;
            if result == 0 {
                return Ok(());
            } else {
                return Err(ControlRuntItemErrors::ExecuteError("stop".to_string(), "failed".to_string()));
            }
        }
        return Err(ControlRuntItemErrors::ExecuteError("stop".to_string(), "failed".to_string()));
    }

    async fn get_state(&self, params: Option<&Vec<String>>) -> Result<ServiceState> {
        let mut need_load_pkg = false;

        {
            let service_pkg = self.service_pkg.read().await;
            if service_pkg.is_none() {
                need_load_pkg = true;
            } else {
                //TODO:还要比较准确版本是否符合现在的“pkg_id要求”
                let result_state = service_pkg.as_ref().unwrap().status().await.map_err(|err| {
                    return ControlRuntItemErrors::ExecuteError("get_state".to_string(), err.to_string());
                })?;
                return Ok(result_state);
            }
        }

        if need_load_pkg {
            let mut service_pkg = ServicePkg::new(self.pkg_id.clone(),get_buckyos_system_bin_dir());
            let load_result = service_pkg.load().await;
            if load_result.is_ok() {
                let mut new_service_pkg = self.service_pkg.write().await;
                let result = service_pkg.status().await.map_err(|err| {
                    return ControlRuntItemErrors::ExecuteError("get_state".to_string(), err.to_string());
                })?;
                *new_service_pkg = Some(service_pkg);
                return Ok(result);
            } else {
                return Ok(ServiceState::NotExist);
            }
        } else {
            //deead path
            warn!("DEAD PATH,never enter here");
            return Err(ControlRuntItemErrors::ExecuteError("get_state".to_string(), "dead path".to_string()));
        }
    }
}
