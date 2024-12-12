use crate::def::*;
use crate::error::*;
use crate::source_manager::SourceManager;
use ::kRPC::*;
use async_trait::async_trait;
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
            _ => Err(RPCErrors::UnknownMethod(req.method)),
        }
    }
}
