use async_trait::async_trait;
use log::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::env;
use std::collections::HashMap;
use std::net::IpAddr;
use std::path::PathBuf;
use std::result::Result;
use std::str::FromStr;
use ::kRPC::*;
use name_lib::{DeviceConfig, ZoneConfig};
use package_lib::*;
use buckyos_kit::buckyos_get_unix_timestamp;
use sys_config::*;

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


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoServerSetting {
    remote_source: HashMap<String, String>,
    enable_dev_mode: bool,
}

impl Default for RepoServerSetting {
    fn default() -> Self {
        Self {
            remote_source: HashMap::new(),
            enable_dev_mode: false,
        }
    }
}

#[derive(Debug,Clone)]
pub struct RepoServer {
    settng: RepoServerSetting,
    session_token : Option<String>,
}

impl RepoServer {
    pub async fn new(config: RepoServerSetting,session_token: Option<String>) -> PkgResult<Self> {
        Ok(RepoServer { 
            settng:config,
            session_token:session_token,
        })
    }

    pub async fn init(&self) -> PkgResult<()> {
        Ok(())
    }

    // async fn handle_install_pkg(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
    //     let pkg_name = ReqHelper::get_str_param_from_req(&req, "pkg_name")?;
    //     //获取version参数，如果未传入，则默认为*
    //     let pkg_version = req
    //         .params
    //         .get("version")
    //         .and_then(|version| version.as_str())
    //         .unwrap_or("*");
    //     let pkg_id = format!("{}#{}", pkg_name, pkg_version);
    //     let pkg_id: PackageId = PackageId::from_str(pkg_id.as_str()).map_err(|e| {
    //         RPCErrors::ParseRequestError(format!("Failed to parse package id, err:{}", e))
    //     })?;
    //     match self.source_mgr.install_pkg(pkg_id).await {
    //         Ok(task_id) => Ok(RPCResponse::new(
    //             RPCResult::Success(json!({
    //                 "task_id": task_id,
    //             })),
    //             req.seq,
    //         )),
    //         Err(e) => Err(RPCErrors::ReasonError(e.to_string())),
    //     }
    // }
    
    //成功返回需要下载的chunkid列表
    async fn check_new_remote_index(&self) -> Result<Vec<String>, RPCErrors> {
        Ok(vec![])
    }
    
    // 将source-meta-index更新到最新版本
    async fn handle_sync_from_remote_source(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        /*
        1.先下载并验证远程版本到临时db
        2.根据业务逻辑检查pkg-meta,下载必要的chunk
        3.下载并验证chunk
        4.全部成功后，将临时db覆盖当前的source-meta-index
        */
        let will_update_source_list = self.settng.remote_source.keys().cloned();
        for source in will_update_source_list {
            let source_url = self.settng.remote_source.get(&source);
            if source_url.is_none() {
                error!("source {} download url not found", source);
                return Err(RPCErrors::ReasonError(format!("source {} not found", source)));
            }
            let source_url = source_url.unwrap();
            info!("update meta-index-db:source {}, download url:{}", source, source_url);
            //let client = NdnClient::new(source_url,self.session_token.clone(),None);
        }

        
        
        unimplemented!();
    }

    // pub_pkg(pkg_list) 将pkg_list发布到zone内，是发布操作的第一步
    async fn handle_pub_pkg(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        /*
        将pkg_list发布到zone内，是发布操作的第一步
        Zone内在调用接口前已经将chunk写入repo-servere可访问的named_mgr了
        检查完所有pkg_list都ready后（尤其是其依赖都ready后），通过SQL事务插入一批pkg到 local-wait-meta

         */
        unimplemented!();
    }

    //将local-wait-meta发布（发布后只读），发布后会计算index-db的chunk_id并构造fileobj,更新R路径的信息
    async fn handle_pub_index(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        /*
        将pkg_list发布到zone内，是发布操作的第一步
        Zone内在调用接口前已经将chunk写入repo-servere可访问的named_mgr了
        检查完所有pkg_list都ready后（尤其是其依赖都ready后），通过SQL事务插入一批pkg到 local-wait-meta
         */
        unimplemented!();
    }
    //源处理 发布pkg到源
    async fn handle_pub_pkg_to_source(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        /*
        因为是处理zone外来的Pkg，所以流程上要稍微复杂一点
        1. 验证身份是合法的
        2. 在pub_pkg_db库里创建发布任务，写入pkg_list，初始状态为已经收到
        2. 检查pkg_list的各种deps已经存在了,失败在发布任务中写入错误信息
        3. 尝试下载chunkid到本地，失败在发布任务中写入错误信息，下载成功的chunk会关联到正确的path,防止被删除
        4. 所有的chunk都准备好了，本次发布成功（业务逻辑也可以加入审核流程，手工将发布任务的状态设置为成功）
        */
        unimplemented!();
    }

    async fn handle_merge_wait_pub_to_source_pkg(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        /*
        合并 `未合并但准备好的` 发布任务里包含的pkg_list到local-wait-meta
        注意merge完后，也要调用pub_index_db发布
        */
        unimplemented!();
    }

    // 查询index-meta
    // async fn handle_query_index_meta(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
    // 查询index-meta
    // async fn handle_query_index_meta(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
    //     let version = match ReqHelper::get_str_param_from_req(&req, "version") {
    //         Ok(version) => Some(version),
    //         Err(e) => None,
    //     };

    //     match self.source_mgr.get_index_meta(version.as_deref()).await {
    //         Ok(meta) => {
    //             info!("query_index_meta, version:{:?}, meta:{:?}", version, meta);
    //             match meta {
    //                 Some(meta) => {
    //                     let meta = serde_json::to_value(meta).map_err(|e| {
    //                         RPCErrors::ReasonError(format!("Failed to serialize meta, err:{}", e))
    //                     })?;
    //                     Ok(RPCResponse::new(RPCResult::Success(meta), req.seq))
    //                 }
    //                 None => Err(RPCErrors::ReasonError(format!(
    //                     "No meta found for version: {:?}",
    //                     version
    //                 ))),
    //             }
    //         }
    //         Err(e) => {
    //             error!("query_index_meta failed, version: {:?}, err:{}", version, e);
    //             Err(RPCErrors::ReasonError(e.to_string()))
    //         }
    //     }
    // }

    // async fn handle_query_task(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
    //     let task_id = ReqHelper::get_str_param_from_req(&req, "task_id")?;
    //     match REPO_TASK_MANAGER.get_task(&task_id).await {
    //         Ok(task) => {
    //             let task = serde_json::to_value(task).map_err(|e| {
    //                 RPCErrors::ReasonError(format!("Failed to serialize task, err:{}", e))
    //             })?;
    //             Ok(RPCResponse::new(RPCResult::Success(task), req.seq))
    //         }
    //         Err(e) => Err(RPCErrors::ReasonError(e.to_string())),
    //     }
    // }

    // async fn handle_query_all_latest_pkg(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
    //     let category = match req.params.get("category") {
    //         Some(category) => category.as_str().map(|s| s),
    //         None => None,
    //     };
    //     match self.source_mgr.query_all_latest_pkg(category).await {
    //         Ok(pkgs) => {
    //             let pkgs = serde_json::to_value(pkgs).map_err(|e| {
    //                 RPCErrors::ReasonError(format!("Failed to serialize pkgs, err:{}", e))
    //             })?;
    //             Ok(RPCResponse::new(RPCResult::Success(pkgs), req.seq))
    //         }
    //         Err(e) => Err(RPCErrors::ReasonError(e.to_string())),
    //     }
    // }
}

#[async_trait]
impl kRPCHandler for RepoServer {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        ip_from: IpAddr,
    ) -> Result<RPCResponse, RPCErrors> {
        match req.method.as_str() {
            //"query_all_latest_pkg" => self.handle_query_all_latest_pkg(req).await,
            //"install_pkg" => self.handle_install_pkg(req).await,
            //"update_index" => self.handle_update_index(req).await,
            "pub_pkg" => self.handle_pub_pkg(req).await,
            "pub_index" => self.handle_pub_index(req).await,
            "pub_pkg_to_source" => self.handle_pub_pkg_to_source(req).await,
            "merge_wait_pub_to_source_pkg" => self.handle_merge_wait_pub_to_source_pkg(req).await,
            "sync_from_remote_source" => self.handle_sync_from_remote_source(req).await,
            //"query_index_meta" => self.handle_query_index_meta(req).await,
            //"query_task" => self.handle_query_task(req).await,
            _ => {
                error!("Unknown method:{}", req.method);
                Err(RPCErrors::UnknownMethod(req.method))
            }
        }
    }
}

