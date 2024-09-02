

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

#[derive(Serialize, Deserialize, Debug)]
pub struct ServiceConfig {
    pub target_state : RunItemTargetState,
    pub version : String,
    //pub name : String, // service name
    pub pkg_id : String,
    pub operations : HashMap<String, RunItemControlOperation>,

}

//service include kernel_service and frame_service,not include app_service

#[async_trait]
impl RunItemControl for ServiceConfig {
    fn get_item_name(&self) -> Result<String> {
        return Ok(self.pkg_id.clone());
    }


    async fn deploy(&self, params: &Option<RunItemParams>) -> Result<()> {
        // 部署文件系统时需要机器名，以root权限运行脚本，默认程序本身应该是root权限
        let env = PackageEnv::new(PathBuf::from("/opt/buckyos/cache/service"));
        let media_info = env.load(&self.pkg_id).await;
        if media_info.is_ok() {
            self.execute_operation(&media_info.unwrap(),"deploy").await?;
            return Ok(());
        } else {
            //TODO: 补充从env中安装pkg的流程
            error!("deploy service {} error: env.install({}) error.", self.pkg_id.as_str(),self.pkg_id);
            return Err(ControlRuntItemErrors::ExecuteError(
                format!("deploy service {} error", self.pkg_id.as_str()),
                "install package error".to_string(),
            ));
        }
    }

    // async fn remove(&self, params: Option<&RunItemParams>) -> Result<()> {
    //     let env = PackageEnv::new(PathBuf::from("/buckyos/service"));
    //     let media_info = env.load_pkg(&self.pkg_id).await;
    //     if media_info.is_ok() {
    //         self.execute_operation(&media_info.unwrap(),"remove").await?;
    //     }
    //     Ok(())
    // }

    async fn update(&self, params: &Option<RunItemParams>) -> Result<String> {

        unimplemented!();
        //self.execute_operation("update").await?;
    }

    async fn start(&self, control_key:&EncodingKey, params: &Option<RunItemParams>) -> Result<()> {
        let env = PackageEnv::new(PathBuf::from("/opt/buckyos/cache/service"));
        let media_info = env.load(&self.pkg_id).await;
        if media_info.is_ok() {
            self.execute_operation(&media_info.unwrap(),"start").await?;
        }
        Ok(())
    }

    async fn stop(&self, params: &Option<RunItemParams>) -> Result<()> {
        let env = PackageEnv::new(PathBuf::from("/opt/buckyos/cache/service"));
        let media_info = env.load(&self.pkg_id).await;
        if media_info.is_ok() {
            self.execute_operation(&media_info.unwrap(),"stop").await?;
        }
        Ok(())
    }

    async fn get_state(&self, params: &Option<RunItemParams>) -> Result<RunItemState> {
        let env = PackageEnv::new(PathBuf::from("/opt/buckyos/cache/service"));
        let media_info = env.load(&self.pkg_id).await;
        if media_info.is_err() {
            return Ok(RunItemState::NotExist);
        }
        let ret_code = self.execute_operation(&media_info.unwrap(),"status").await?;
        if ret_code == 0 {
            Ok(RunItemState::Started)
        } else if ret_code > 0 {
            Ok(RunItemState::Stopped("".to_string()))
        } else {
            Ok(RunItemState::NotExist)
        }
    }
}

impl ServiceConfig {
    async fn execute_operation(&self, media_info: &MediaInfo, op_name: &str) -> Result<i32> {
        let op = self.operations.get(op_name);
        if op.is_none() {
            warn!("{} service execuite op {} error:  operation not found", self.pkg_id.as_str(),op_name);
            return Err(ControlRuntItemErrors::ExecuteError(
                format!("{} service execuite op {} error", self.pkg_id.as_str(), op_name),
                "deploy operation not found".to_string(),
            ));
        }

        let op: &RunItemControlOperation = op.unwrap();
        let op_sh_file = media_info.full_path.join(op.command.as_str());
        //run_cmd(deploy_sh_file)
        let ret = buckyos_kit::run_script_with_args(
            op_sh_file.to_str().unwrap(),
            5,  
            &op.params
        ).await.map_err(|error| {
            ControlRuntItemErrors::ExecuteError(
                format!("{} service execuite op {} error", self.pkg_id.as_str(), op_name),
                format!("{}", error),
            )
        })?;
        return Ok(ret);
    }

    
}
