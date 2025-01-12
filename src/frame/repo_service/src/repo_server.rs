use crate::def::*;
use crate::downloader::Downloader;
use crate::source_manager::SourceManager;
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
        let pkg_version = ReqHelper::get_str_param_from_req(&req, "version")?;
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
            .unwrap_or(false);
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
        let author_did = ReqHelper::get_str_param_from_req(&req, "author")?;
        let chunk_id = ReqHelper::get_str_param_from_req(&req, "chunk_id")?;
        let dependencies = ReqHelper::get_str_param_from_req(&req, "dependencies")?;
        let sign = ReqHelper::get_str_param_from_req(&req, "sign")?;
        let pub_time = buckyos_get_unix_timestamp() as i64;

        let zone_did = ZoneInfoHelper::get_zone_did().map_err(|e| {
            RPCErrors::ParseRequestError(format!("Failed to get zone did, err:{}", e))
        })?;
        let zone_name = ZoneInfoHelper::get_zone_name().map_err(|e| {
            RPCErrors::ParseRequestError(format!("Failed to get zone name, err:{}", e))
        })?;

        //TODO did应该和author一致？存不存在二次打包的情况？

        let pkg_meta = PackageMeta {
            pkg_name,
            version,
            author_did,
            author_name: zone_name,
            chunk_id,
            dependencies: serde_json::Value::from_str(&dependencies).map_err(|e| {
                RPCErrors::ParseRequestError(format!("Failed to parse dependencies, err:{}", e))
            })?,
            sign: sign.to_string(),
            pub_time,
        };
        match self.source_mgr.pub_pkg(&pkg_meta).await {
            Ok(_) => Ok(RPCResponse::new(RPCResult::Success(Value::Null), req.seq)),
            Err(e) => Err(RPCErrors::ReasonError(e.to_string())),
        }
    }

    async fn handle_pub_index(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let pem_file = ReqHelper::get_str_param_from_req(&req, "pem_file")?;
        let version = ReqHelper::get_str_param_from_req(&req, "version")?;
        match self
            .source_mgr
            .pub_index(&PathBuf::from(pem_file), &version)
            .await
        {
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
}

#[async_trait]
impl kRPCHandler for RepoServer {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        ip_from: IpAddr,
    ) -> Result<RPCResponse, RPCErrors> {
        match req.method.as_str() {
            "install_pkg" => self.handle_install_pkg(req).await,
            "update_index" => self.handle_update_index(req).await,
            "pub_pkg" => self.handle_pub_pkg(req).await,
            "pub_index" => self.handle_pub_index(req).await,
            "query_index_meta" => self.handle_query_index_meta(req).await,
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
