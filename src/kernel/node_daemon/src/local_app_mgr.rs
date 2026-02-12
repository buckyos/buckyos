use crate::run_item::*;
use crate::service_pkg::*;
use async_trait::async_trait;
use buckyos_api::*;
use buckyos_kit::*;
use jsonwebtoken::{DecodingKey, EncodingKey};
use log::*;
use name_lib::DeviceConfig;
use package_lib::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::hash::Hash;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use tokio::sync::RwLock;

//use package_installer::*;
use buckyos_api::{get_full_appid, get_session_token_env_key, AppServiceInstanceConfig};

//  目前本地app不支持docker
pub struct LocalAppRunItem {
    pub app_id: String,
    pub app_instance_config: LocalAppInstanceConfig,
    pub app_loader: ServicePkg,
}

impl LocalAppRunItem {
    pub fn new(
        app_id: &String, // app_id@username@nodeid
        app_instance_config: LocalAppInstanceConfig,
        app_loader: ServicePkg,
    ) -> Self {
        LocalAppRunItem {
            app_id: app_id.clone(),
            app_instance_config,
            app_loader,
        }
    }

    fn get_instance_pkg_id(&self, is_strict: bool) -> Result<String> {
        // let docker_image_pkg_id = self.app_instance_config.app_doc.pkg_list.get_docker_image_pkg_id();
        // if docker_image_pkg_id.is_some() {
        //     let full_pkg_id = docker_image_pkg_id.unwrap();
        //     if is_strict {
        //         return Ok(full_pkg_id);
        //     } else {
        //         return Ok(PackageId::get_pkg_id_unique_name(full_pkg_id.as_str()));
        //     }
        // }
        let app_pkg_id = self.app_instance_config.app_doc.pkg_list.get_app_pkg_id();
        if app_pkg_id.is_some() {
            let full_app_pkg_id = app_pkg_id.unwrap();
            if is_strict {
                return Ok(full_app_pkg_id);
            } else {
                return Ok(PackageId::get_pkg_id_unique_name(full_app_pkg_id.as_str()));
            }
        }

        Err(ControlRuntItemErrors::PkgNotExist(format!(
            "app {} pkg not found",
            self.app_id
        )))
    }

    async fn set_env_var(&self, _is_system_app: bool) -> Result<()> {
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
                unsafe {
                    std::env::set_var("app_media_info", media_info_json_str);
                }
            }
        }

        let app_config_str = serde_json::to_string(&self.app_instance_config).unwrap();
        unsafe {
            std::env::set_var("loca_app_instance_config", app_config_str);
        }
        let full_appid = get_full_appid(&self.app_id, &self.app_instance_config.user_id);

        Ok(())
    }
}

#[async_trait]
impl RunItemControl for LocalAppRunItem {
    fn get_item_name(&self) -> Result<String> {
        //appid#userid
        let full_appid = format!("{}#{}", self.app_instance_config.user_id, self.app_id);
        Ok(full_appid)
    }

    async fn deploy(&self, params: Option<&Vec<String>>) -> Result<()> {
        let is_system_app = self
            .app_instance_config
            .app_doc
            .pkg_list
            .get_app_pkg_id()
            .is_some();

        let mut env = PackageEnv::new(get_buckyos_system_bin_dir());
        let instance_pkg_id = self.get_instance_pkg_id(env.is_strict())?;
        info!("install local app pkg {}", instance_pkg_id);
        let install_result = env
            .install_pkg(&instance_pkg_id, true, false)
            .await
            .map_err(|e| {
                error!("LocalAppRunItem install pkg {} failed! {}", self.app_id, e);
                return ControlRuntItemErrors::ExecuteError("deploy".to_string(), e.to_string());
            });

        if install_result.is_ok() {
            warn!("install local app instance pkg {} success", instance_pkg_id);
        }

        if !is_system_app {
            // self.set_env_var(false).await?;
            // let real_param = vec![self.app_id.clone(), self.app_instance_config.user_id.clone()];
            // let result = self.app_loader.execute_operation("deploy",Some(&real_param)).await.map_err(|err| {
            //     return ControlRuntItemErrors::ExecuteError(
            //         "deploy".to_string(),
            //         err.to_string(),
            //     );
            // });
            // if result.is_ok() {
            //     if result.unwrap() == 0 {
            //         info!("deploy local app (not system) {} by app_loader success",self.app_id);
            //         return Ok(());
            //     }
            // }
            // Ok(())
            return Err(ControlRuntItemErrors::NotSupport(
                "local app not only support system app (not docker)".to_string(),
            ));
        } else {
            if install_result.is_ok() {
                return Ok(());
            } else {
                return Err(install_result.err().unwrap());
            }
        }
    }

    async fn start(&self, params: Option<&Vec<String>>) -> Result<()> {
        let is_system_app = self
            .app_instance_config
            .app_doc
            .pkg_list
            .get_app_pkg_id()
            .is_some();
        if is_system_app {
            self.set_env_var(true).await?;
        } else {
            return Err(ControlRuntItemErrors::NotSupport(
                "local app not only support system app (not docker)".to_string(),
            ));
        }
        let real_param = vec![
            self.app_id.clone(),
            self.app_instance_config.user_id.clone(),
        ];

        let result = self
            .app_loader
            .start(Some(&real_param))
            .await
            .map_err(|err| {
                return ControlRuntItemErrors::ExecuteError("start".to_string(), err.to_string());
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
        let is_system_app = self
            .app_instance_config
            .app_doc
            .pkg_list
            .get_app_pkg_id()
            .is_some();

        if is_system_app {
            self.set_env_var(true).await?;
        } else {
            self.set_env_var(false).await?;
        }
        let real_param = vec![
            self.app_id.clone(),
            self.app_instance_config.user_id.clone(),
        ];
        let result = self
            .app_loader
            .stop(Some(&real_param))
            .await
            .map_err(|err| {
                return ControlRuntItemErrors::ExecuteError("stop".to_string(), err.to_string());
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
        let mut is_system_app = false;
        if self
            .app_instance_config
            .app_doc
            .pkg_list
            .get_app_pkg_id()
            .is_some()
        {
            let env = PackageEnv::new(get_buckyos_system_bin_dir());
            let instance_pkg_id = self.get_instance_pkg_id(env.is_strict())?;
            info!(
                "state system app,will load dapp's app_pkg {}",
                instance_pkg_id.as_str()
            );
            let app_pkg = env.load(instance_pkg_id.as_str()).await;
            if app_pkg.is_err() {
                return Ok(ServiceInstanceState::NotExist);
            }
            is_system_app = true;
        } else {
            is_system_app = false;
            return Err(ControlRuntItemErrors::NotSupport(
                "local app not only support system app (not docker)".to_string(),
            ));
        }

        self.set_env_var(is_system_app).await?;
        let real_param = vec![
            self.app_id.clone(),
            self.app_instance_config.user_id.clone(),
        ];
        let result: ServiceInstanceState = self
            .app_loader
            .status(Some(&real_param))
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
