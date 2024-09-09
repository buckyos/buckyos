
use async_trait::async_trait;
use jsonwebtoken::{DecodingKey, EncodingKey};
use log::*;
use serde_json::Value;
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use std::hash::Hash;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use crate::run_item::*;
use package_manager::*;
use buckyos_kit::*;

#[derive(Serialize, Deserialize)]
pub struct AppServiceConfig {
    pub target_state : RunItemTargetState,
    pub pkg_id : String,

    #[serde(skip)]
    docker_pkg : Option<ServicePkg>,
}


#[async_trait]
impl RunItemControl for AppServiceConfig {
    fn get_item_name(&self) -> Result<String> {
        Ok(self.pkg_id.clone())
    }

    async fn deploy(&self, params: Option<&Vec<String>>) -> Result<()> {
        //check already have deploy task ?
        //create deploy task
            //install  or upgrade pkg
            //call pkg.deploy() scrpit 不要调用，由pkg在自己的start脚本里管理？
        unimplemented!();
    }

    async fn start(&self, control_key:&EncodingKey,params: Option<&Vec<String>>) -> Result<()> {
        unimplemented!();
    }
    async fn stop(&self, params: Option<&Vec<String>>) -> Result<()> {
        unimplemented!();
    }

    async fn get_state(&self, params: Option<&Vec<String>>) -> Result<ServiceState> {
        //加载一个确定的，容器管理的service pkg(用户无法扩展)
        //将pkg id作为容器名，交给该脚本管理
        unimplemented!();
    }
}
