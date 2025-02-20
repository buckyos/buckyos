use crate::def::*;
use crate::downloader::Downloader;
use crate::source_manager::SourceManager;
use crate::task_manager::REPO_TASK_MANAGER;
use crate::zone_info_helper::ZoneInfoHelper;
use ::kRPC::*;
use async_trait::async_trait;
use buckyos_kit::buckyos_get_unix_timestamp;
use log::*;
use name_lib::{DeviceConfig, ZoneConfig};
use package_lib::PackageId;
use serde_json::{json, Value};
use std::env;
use std::net::IpAddr;
use std::path::PathBuf;
use std::result::Result;
use std::str::FromStr;

#[derive(Debug, Clone)]
pub struct RepoServer {
    source_mgr: SourceManager,
}

impl RepoServer {
    pub async fn new() -> RepoResult<Self> {
        let source_mgr = SourceManager::new().await?;
        Downloader::init_repo_chunk_mgr().await?;
        Ok(RepoServer { source_mgr })
    }

    async fn handle_install_pkg(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let pkg_name = ReqHelper::get_str_param_from_req(&req, "pkg_name")?;
        //获取version参数，如果未传入，则默认为*
        let pkg_version = req
            .params
            .get("version")
            .and_then(|version| version.as_str())
            .unwrap_or("*");
        let pkg_id = format!("{}#{}", pkg_name, pkg_version);
        let pkg_id: PackageId = PackageId::from_str(pkg_id.as_str()).map_err(|e| {
            RPCErrors::ParseRequestError(format!("Failed to parse package id, err:{}", e))
        })?;
        match self.source_mgr.install_pkg(pkg_id).await {
            Ok(task_id) => Ok(RPCResponse::new(
                RPCResult::Success(json!({
                    "task_id": task_id,
                })),
                req.seq,
            )),
            Err(e) => Err(RPCErrors::ReasonError(e.to_string())),
        }
    }

    async fn handle_update_index(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        info!("handle_update_index, params:{:?}", req.params);
        let update = req
            .params
            .get("update")
            .and_then(|update| update.as_bool())
            .unwrap_or(true);
        match self.source_mgr.update_index(update).await {
            Ok(task_id) => Ok(RPCResponse::new(
                RPCResult::Success(json!({
                    "task_id": task_id,
                })),
                req.seq,
            )),
            Err(e) => Err(RPCErrors::ReasonError(e.to_string())),
        }
    }

    async fn handle_pub_pkg(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let pkg_name = ReqHelper::get_str_param_from_req(&req, "pkg_name")?;
        let version = ReqHelper::get_str_param_from_req(&req, "version")?;
        let hostname = ReqHelper::get_str_param_from_req(&req, "hostname")?;
        let chunk_id = ReqHelper::get_str_param_from_req(&req, "chunk_id")?;
        let dependencies = ReqHelper::get_str_param_from_req(&req, "dependencies")?;
        let jwt = ReqHelper::get_str_param_from_req(&req, "jwt")?;
        let pub_time = buckyos_get_unix_timestamp() as i64;

        let pkg_meta = PackageMeta {
            pkg_name,
            version,
            hostname,
            chunk_id: Some(chunk_id),
            dependencies,
            jwt,
            pub_time,
        };

        log::info!("recv pub_pkg request, pkg_meta:{:?}", pkg_meta);

        match self.source_mgr.pub_pkg(&pkg_meta).await {
            Ok(_) => Ok(RPCResponse::new(RPCResult::Success(Value::Null), req.seq)),
            Err(e) => Err(RPCErrors::ReasonError(e.to_string())),
        }
    }

    async fn handle_pub_index(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let version = ReqHelper::get_str_param_from_req(&req, "version")?;
        let hostname = ReqHelper::get_str_param_from_req(&req, "hostname")?;
        let jwt = ReqHelper::get_str_param_from_req(&req, "jwt")?;

        log::info!(
            "recv pub_index request, version:{}, hostname:{}, jwt:{}",
            version,
            hostname,
            jwt
        );

        //TODO: did 和 hostname 传入的和从zone获取的是否要一致？
        match self.source_mgr.pub_index(&version, &hostname, &jwt).await {
            Ok(_) => Ok(RPCResponse::new(RPCResult::Success(Value::Null), req.seq)),
            Err(e) => Err(RPCErrors::ReasonError(e.to_string())),
        }
    }

    async fn handle_query_index_meta(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let version = match ReqHelper::get_str_param_from_req(&req, "version") {
            Ok(version) => Some(version),
            Err(e) => None,
        };

        match self.source_mgr.get_index_meta(version.as_deref()).await {
            Ok(meta) => {
                info!("query_index_meta, version:{:?}, meta:{:?}", version, meta);
                match meta {
                    Some(meta) => {
                        let meta = serde_json::to_value(meta).map_err(|e| {
                            RPCErrors::ReasonError(format!("Failed to serialize meta, err:{}", e))
                        })?;
                        Ok(RPCResponse::new(RPCResult::Success(meta), req.seq))
                    }
                    None => Err(RPCErrors::ReasonError(format!(
                        "No meta found for version: {:?}",
                        version
                    ))),
                }
            }
            Err(e) => {
                error!("query_index_meta failed, version: {:?}, err:{}", version, e);
                Err(RPCErrors::ReasonError(e.to_string()))
            }
        }
    }

    async fn handle_query_task(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let task_id = ReqHelper::get_str_param_from_req(&req, "task_id")?;
        match REPO_TASK_MANAGER.get_task(&task_id).await {
            Ok(task) => {
                let task = serde_json::to_value(task).map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to serialize task, err:{}", e))
                })?;
                Ok(RPCResponse::new(RPCResult::Success(task), req.seq))
            }
            Err(e) => Err(RPCErrors::ReasonError(e.to_string())),
        }
    }

    async fn handle_query_all_latest_pkg(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        match self.source_mgr.query_all_latest_pkg().await {
            Ok(pkgs) => {
                let pkgs = serde_json::to_value(pkgs).map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to serialize pkgs, err:{}", e))
                })?;
                Ok(RPCResponse::new(RPCResult::Success(pkgs), req.seq))
            }
            Err(e) => Err(RPCErrors::ReasonError(e.to_string())),
        }
    }

    pub async fn init(&self) -> RepoResult<()> {
        self.source_mgr.init().await
    }
}

#[async_trait]
impl kRPCHandler for RepoServer {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        ip_from: IpAddr,
    ) -> Result<RPCResponse, RPCErrors> {
        match req.method.as_str() {
            "query_all_latest_pkg" => self.handle_query_all_latest_pkg(req).await,
            "install_pkg" => self.handle_install_pkg(req).await,
            "update_index" => self.handle_update_index(req).await,
            "pub_pkg" => self.handle_pub_pkg(req).await,
            "pub_index" => self.handle_pub_index(req).await,
            "query_index_meta" => self.handle_query_index_meta(req).await,
            "query_task" => self.handle_query_task(req).await,
            _ => {
                error!("Unknown method:{}", req.method);
                Err(RPCErrors::UnknownMethod(req.method))
            }
        }
    }
}

struct ReqHelper;

impl ReqHelper {
    fn get_str_param_from_req(req: &RPCRequest, key: &str) -> Result<String, RPCErrors> {
        req.params
            .get(key)
            .and_then(|value| value.as_str())
            .map(|value| value.to_string())
            .ok_or(RPCErrors::ParseRequestError(format!(
                "Failed to get {} from params",
                key
            )))
    }

    fn get_session_token(req: &RPCRequest) -> Result<RPCSessionToken, RPCErrors> {
        req.token
            .as_ref()
            .map(|token| RPCSessionToken::from_string(token.as_str()))
            .unwrap_or(Err(RPCErrors::ParseRequestError(
                "Invalid params, session_token is none".to_string(),
            )))
    }

    fn get_user_id(req: &RPCRequest) -> Result<String, RPCErrors> {
        let session_token = Self::get_session_token(req)?;
        session_token
            .userid
            .as_ref()
            .map(|userid| userid.to_string())
            .ok_or(RPCErrors::ParseRequestError(
                "Invalid params, user_id is none".to_string(),
            ))
    }
}
