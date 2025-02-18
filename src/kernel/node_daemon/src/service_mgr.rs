use async_trait::async_trait;
use jsonwebtoken::{DecodingKey, EncodingKey};
use log::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::hash::Hash;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use crate::run_item::*;
use buckyos_kit::*;
use package_lib::*;

#[derive(Serialize, Deserialize, Debug)]
pub struct FrameServiceConfig {
    pub target_state: RunItemTargetState,
    //pub name : String, // service name
    pub pkg_id: String,
    pub operations: HashMap<String, RunItemControlOperation>,

    //不支持serizalize
    #[serde(skip)]
    service_pkg: Option<MediaInfo>,
}

//service include kernel_service and frame_service,not include app_service

#[async_trait]
impl RunItemControl for FrameServiceConfig {
    fn get_item_name(&self) -> Result<String> {
        Ok(self.pkg_id.clone())
    }

    async fn deploy(&self, params: Option<&Vec<String>>) -> Result<()> {
        // 部署文件系统时需要机器名，以root权限运行脚本，默认程序本身应该是root权限
        let env = PackageEnv::new(PathBuf::from("/opt/buckyos/cache/service"));
        let media_info = env.load(&self.pkg_id);
        if media_info.is_ok() {
            self.execute_operation(&media_info.unwrap(), "deploy")
                .await?;
            Ok(())
        } else {
            //TODO: 补充从env中安装pkg的流程
            error!(
                "deploy service {} error: env.install({}) error.",
                self.pkg_id.as_str(),
                self.pkg_id
            );
            Err(ControlRuntItemErrors::ExecuteError(
                format!("deploy service {} error", self.pkg_id.as_str()),
                "install package error".to_string(),
            ))
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

    async fn start(&self, control_key: &EncodingKey, params: Option<&Vec<String>>) -> Result<()> {
        let env = PackageEnv::new(PathBuf::from("/opt/buckyos/cache/service"));
        let media_info = env.load(&self.pkg_id);
        if media_info.is_ok() {
            self.execute_operation(&media_info.unwrap(), "start")
                .await?;
        }
        Ok(())
    }

    async fn stop(&self, params: Option<&Vec<String>>) -> Result<()> {
        let env = PackageEnv::new(PathBuf::from("/opt/buckyos/cache/service"));
        let media_info = env.load(&self.pkg_id);
        if media_info.is_ok() {
            self.execute_operation(&media_info.unwrap(), "stop").await?;
        }
        Ok(())
    }

    async fn get_state(&self, params: Option<&Vec<String>>) -> Result<ServiceState> {
        let env = PackageEnv::new(PathBuf::from("/opt/buckyos/cache/service"));
        let media_info = env.load(&self.pkg_id);
        if media_info.is_err() {
            return Ok(ServiceState::NotExist);
        }
        let ret_code = self
            .execute_operation(&media_info.unwrap(), "status")
            .await?;
        if ret_code == 0 {
            Ok(ServiceState::Started)
        } else if ret_code > 0 {
            Ok(ServiceState::Stopped)
        } else {
            Ok(ServiceState::NotExist)
        }
    }
}

impl FrameServiceConfig {
    async fn execute_operation(&self, media_info: &MediaInfo, op_name: &str) -> Result<i32> {
        let op = self.operations.get(op_name);
        if op.is_none() {
            warn!(
                "{} service execuite op {} error:  operation not found",
                self.pkg_id.as_str(),
                op_name
            );
            return Err(ControlRuntItemErrors::ExecuteError(
                format!(
                    "{} service execuite op {} error",
                    self.pkg_id.as_str(),
                    op_name
                ),
                "deploy operation not found".to_string(),
            ));
        }

        let op: &RunItemControlOperation = op.unwrap();
        let op_sh_file = media_info.full_path.join(op.command.as_str());
        //run_cmd(deploy_sh_file)
        let ret = execute(
            &op_sh_file,
            5,
            None, //TODO: 补充op的params
            None,
            None,
        )
        .await
        .map_err(|error| {
            ControlRuntItemErrors::ExecuteError(
                format!(
                    "{} service execuite op {} error",
                    self.pkg_id.as_str(),
                    op_name
                ),
                format!("{}", error),
            )
        })?;
        return Ok(ret.0);
    }
}
