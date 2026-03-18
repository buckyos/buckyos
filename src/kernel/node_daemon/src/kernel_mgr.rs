use crate::app_loader::{expected_env_pkg_name, parse_package_meta_from_store};
use crate::run_item::*;
use crate::service_pkg::*;
use async_trait::async_trait;
use buckyos_api::*;
use buckyos_kit::*;
use jsonwebtoken::{DecodingKey, EncodingKey};
use log::*;
use name_lib::DeviceConfig;
use ndn_lib::ObjId;
use package_lib::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::hash::Hash;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use tokio::sync::RwLock;
//use package_installer::*;

use crate::run_item::*;

pub struct KernelServiceRunItem {
    pub service_name: String,
    pub pkg_desc: SubPkgDesc,
    service_pkg: ServicePkg,
}

impl KernelServiceRunItem {
    pub fn new(app_id: &str, kernel_config: &KernelServiceInstanceConfig) -> Self {
        let service_doc = &kernel_config.service_sepc.service_doc;
        let pkg_desc = current_platform_service_pkg_desc(service_doc)
            .unwrap_or_else(|| SubPkgDesc::new(service_doc.get_package_id().to_string()));
        let pkg_id = pkg_desc
            .get_pkg_id_with_objid()
            .unwrap_or_else(|| pkg_desc.pkg_id.clone());
        let service_pkg = ServicePkg::new(pkg_id.clone(), get_buckyos_system_bin_dir());
        Self {
            service_name: app_id.to_string(),
            pkg_desc,
            service_pkg: service_pkg,
        }
    }

    fn resolved_pkg_id(&self) -> String {
        self.pkg_desc
            .get_pkg_id_with_objid()
            .unwrap_or_else(|| self.pkg_desc.pkg_id.clone())
    }

    async fn ensure_exact_pkg_meta_indexed(&self) -> Result<()> {
        let pkg_id = self.resolved_pkg_id();
        let package_id = PackageId::parse(pkg_id.as_str()).map_err(|error| {
            ControlRuntItemErrors::ExecuteError(
                "index_pkg_meta".to_string(),
                format!("parse pkg id {} failed: {}", pkg_id, error),
            )
        })?;
        let Some(meta_obj_id_str) = package_id.objid.as_deref() else {
            return Ok(());
        };

        let env = new_system_package_env();
        if env.get_pkg_meta(pkg_id.as_str()).await.is_ok() {
            return Ok(());
        }

        let runtime = get_buckyos_api_runtime().map_err(|error| {
            ControlRuntItemErrors::ExecuteError(
                "index_pkg_meta".to_string(),
                format!(
                    "buckyos runtime unavailable when indexing kernel service {}: {}",
                    pkg_id, error
                ),
            )
        })?;
        let store_mgr = runtime.get_named_store().await.map_err(|error| {
            ControlRuntItemErrors::ExecuteError(
                "index_pkg_meta".to_string(),
                format!(
                    "get named store for kernel service {} failed: {}",
                    pkg_id, error
                ),
            )
        })?;
        let meta_obj_id = ObjId::new(meta_obj_id_str).map_err(|error| {
            ControlRuntItemErrors::ExecuteError(
                "index_pkg_meta".to_string(),
                format!("parse pkg objid {} failed: {}", meta_obj_id_str, error),
            )
        })?;
        let pkg_meta_str = store_mgr.get_object(&meta_obj_id).await.map_err(|error| {
            ControlRuntItemErrors::ExecuteError(
                "index_pkg_meta".to_string(),
                format!(
                    "load kernel service pkg meta {} from named store failed: {}",
                    meta_obj_id, error
                ),
            )
        })?;

        let meta_obj_id_string = meta_obj_id.to_string();
        let mut pkg_meta =
            parse_package_meta_from_store(meta_obj_id_string.as_str(), &pkg_meta_str)?;
        let expected_pkg_name = expected_env_pkg_name(&env, &package_id);
        if pkg_meta.name != expected_pkg_name {
            pkg_meta.name = expected_pkg_name;
        }
        env.set_pkg_meta_to_index_db(meta_obj_id_string.as_str(), &pkg_meta)
            .await
            .map_err(|error| {
                ControlRuntItemErrors::ExecuteError(
                    "index_pkg_meta".to_string(),
                    format!(
                        "insert kernel service pkg meta {} into env db failed: {}",
                        meta_obj_id, error
                    ),
                )
            })?;

        info!(
            "indexed exact pkg meta {} for kernel service {}",
            pkg_id, self.service_name
        );
        Ok(())
    }
}

fn current_platform_service_pkg_desc(service_doc: &AppDoc) -> Option<SubPkgDesc> {
    let pkg_list = &service_doc.pkg_list;

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        return pkg_list
            .amd64_linux_app
            .clone()
            .or(pkg_list.aarch64_linux_app.clone());
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        return pkg_list
            .aarch64_linux_app
            .clone()
            .or(pkg_list.amd64_linux_app.clone());
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        return pkg_list
            .amd64_win_app
            .clone()
            .or(pkg_list.aarch64_win_app.clone());
    }
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        return pkg_list
            .aarch64_win_app
            .clone()
            .or(pkg_list.amd64_win_app.clone());
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        return pkg_list
            .amd64_apple_app
            .clone()
            .or(pkg_list.aarch64_apple_app.clone());
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        return pkg_list
            .aarch64_apple_app
            .clone()
            .or(pkg_list.amd64_apple_app.clone());
    }

    #[allow(unreachable_code)]
    None
}

#[async_trait]
impl RunItemControl for KernelServiceRunItem {
    fn get_item_name(&self) -> Result<String> {
        Ok(self.resolved_pkg_id())
    }

    async fn deploy(&self, params: Option<&Vec<String>>) -> Result<()> {
        //这个逻辑是不区分新装和升级的
        let pkg_id = self.resolved_pkg_id();
        self.ensure_exact_pkg_meta_indexed().await?;
        let mut pkg_env = new_system_package_env();
        pkg_env
            .install_pkg(&pkg_id, true, false)
            .await
            .map_err(|e| {
                error!("KernelServiceRunItem install pkg {} failed! {}", pkg_id, e);
                return ControlRuntItemErrors::ExecuteError("deploy".to_string(), e.to_string());
            })?;

        warn!("install kernel service {} success", pkg_id);
        Ok(())
    }

    async fn start(&self, params: Option<&Vec<String>>) -> Result<()> {
        self.ensure_exact_pkg_meta_indexed().await?;
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
            exp: Some(timestamp + VERIFY_HUB_TOKEN_EXPIRE_TIME * 2),
            iss: Some(device_doc.name.clone()),
            token: None,
            extra: HashMap::new(),
        };

        let device_session_token_jwt = device_session_token
            .generate_jwt(None, device_private_key)
            .map_err(|err| {
                error!(
                    "generate session token for {} failed! {}",
                    self.resolved_pkg_id(),
                    err
                );
                return ControlRuntItemErrors::ExecuteError("start".to_string(), err.to_string());
            })?;

        let env_key = get_session_token_env_key(&self.service_name, false);
        unsafe {
            std::env::set_var(env_key.as_str(), device_session_token_jwt);
        }

        let result = self.service_pkg.start(params).await.map_err(|err| {
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
        let result = self.service_pkg.stop(None).await.map_err(|err| {
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
        self.ensure_exact_pkg_meta_indexed().await?;
        let result = self.service_pkg.status(None).await.map_err(|err| {
            return ControlRuntItemErrors::ExecuteError("get_state".to_string(), err.to_string());
        })?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::KernelServiceRunItem;
    use buckyos_api::{
        AppDoc, AppType, KernelServiceInstanceConfig, KernelServiceSpec, ServiceInstallConfig,
        ServiceInstanceState, ServiceState, SubPkgDesc,
    };
    use name_lib::DID;
    use ndn_lib::ObjId;

    #[test]
    fn kernel_service_run_item_uses_current_platform_pkg_desc() {
        let owner = DID::from_str("did:bns:test").unwrap();
        let mut service_doc = AppDoc::builder(
            AppType::Service,
            "verify-hub",
            "0.1.0",
            "did:bns:test",
            &owner,
        )
        .build()
        .unwrap();
        let mut desc = SubPkgDesc::new("verify-hub#0.1.0");
        desc.pkg_objid = Some(ObjId::new("pkg:1234567890").unwrap());

        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            service_doc.pkg_list.amd64_linux_app = Some(desc);
        }
        #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
        {
            service_doc.pkg_list.aarch64_linux_app = Some(desc);
        }
        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        {
            service_doc.pkg_list.amd64_win_app = Some(desc);
        }
        #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
        {
            service_doc.pkg_list.aarch64_win_app = Some(desc);
        }
        #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
        {
            service_doc.pkg_list.amd64_apple_app = Some(desc);
        }
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            service_doc.pkg_list.aarch64_apple_app = Some(desc);
        }

        let item = KernelServiceRunItem::new(
            "verify-hub",
            &KernelServiceInstanceConfig {
                target_state: ServiceInstanceState::Started,
                node_id: "ood1".to_string(),
                service_sepc: KernelServiceSpec {
                    service_doc,
                    enable: true,
                    app_index: 0,
                    expected_instance_count: 1,
                    state: ServiceState::default(),
                    install_config: ServiceInstallConfig::default(),
                },
            },
        );

        assert_eq!(
            item.resolved_pkg_id(),
            item.pkg_desc
                .get_pkg_id_with_objid()
                .expect("resolved pkg id should include objid")
        );
    }
}
