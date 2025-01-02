use crate::def::*;
use crate::downloader::Downloader;
use crate::source_manager::SourceManager;
use ::kRPC::*;
use async_trait::async_trait;
use buckyos_kit::buckyos_get_unix_timestamp;
use package_lib::PackageId;
use serde_json::{json, Value};
use std::net::IpAddr;
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
        let pkg_name = req.params.get("pkg_name").unwrap().as_str().unwrap();
        let pkg_version = req.params.get("version").unwrap().as_str().unwrap();
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
        let update = req.params.get("update").unwrap().as_bool().unwrap();
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
        let pkg_name = req.params.get("pkg_name").unwrap().as_str().unwrap();
        let version = req.params.get("version").unwrap().as_str().unwrap();
        let author = req.params.get("author").unwrap().as_str().unwrap();
        let chunk_id = req.params.get("chunk_id").unwrap().as_str().unwrap();
        let dependencies = req.params.get("dependencies").unwrap().as_str().unwrap();
        let sign = req.params.get("sign").unwrap().as_str().unwrap();
        let pub_time = buckyos_get_unix_timestamp() as i64;
        let pkg_meta = PackageMeta {
            name: pkg_name.to_string(),
            version: version.to_string(),
            author: author.to_string(),
            chunk_id: chunk_id.to_string(),
            dependencies: serde_json::Value::from_str(dependencies).map_err(|e| {
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
        unimplemented!()
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
            _ => Err(RPCErrors::UnknownMethod(req.method)),
        }
    }
}
