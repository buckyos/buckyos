use async_trait::async_trait;
use log::*;
use ndn_lib::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use std::{env, hash::Hash};
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::path::PathBuf;
use std::result::Result;
use std::str::FromStr;
use ::kRPC::*;
use name_lib::{DeviceConfig, ZoneConfig, CURRENT_ZONE_CONFIG};
use package_lib::*;
use buckyos_kit::buckyos_get_unix_timestamp;
use sys_config::*;
use tokio::io::AsyncSeekExt;
use name_lib::*;

use crate::pub_task_mgr::*;
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

struct WillDownloadPkgInfo {
    pkg_name:String,
    pkg_version:String,
    chunk_id:String,
    chunk_size:u64,
}

/*
repo-server持有的几个meta-index-db

source-mgr
    source_name meta-index-db1 (read-only)
    source_nmeta-index-db2 (read-ony)

如果打开了开发者模式，则有：
    local-pub meta-index-db (read-only)
    local-wait-pub meta-index-db

流程的核心理念与git类似

发布pkg: 先commit到当前repo.然后再Merge到remote-repo，merge流程允许手工介入
获得更新: git pull后，根据当前已经安装的app列表下载chunk,全部成功后再切换当前repo的版本
*/


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
    async fn check_new_remote_index(&self,new_meta_index_db_path: &PathBuf) -> Result<HashMap<String,WillDownloadPkgInfo>, RPCErrors> {
        unimplemented!();
    }

    fn get_meta_index_db_path(&self,source: &str) -> PathBuf {
        let path = format!("{}/.repo/source-meta-index-db/{}", env::var("HOME").unwrap(), source);
        PathBuf::from(path)
    }
    //
    async fn replace_file(target_file_path:&PathBuf,file_path:&PathBuf) -> Result<(),RPCErrors> {
        unimplemented!();

    }
    // 将source-meta-index更新到最新版本
    async fn handle_sync_from_remote_source(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        //尝试拿到sync操作的锁，拿不到则说明已经在处理了
        //1.先下载并验证远程版本到临时db
        let will_update_source_list = self.settng.remote_source.keys().cloned();
       
        for source in will_update_source_list {
            let source_url = self.settng.remote_source.get(&source);
            if source_url.is_none() {
                error!("source {} download url not found", source);
                return Err(RPCErrors::ReasonError(format!("source {} not found", source)));
            }
            let source_url = source_url.unwrap();
            info!("update meta-index-db:source {}, download url:{}", source, source_url);
            let new_meta_index_db_path = self.get_meta_index_db_path(&source);

            let ndn_client = NdnClient::new(source_url.clone(),self.session_token.clone(),None);
            ndn_client.download_chunk_to_local(source_url.as_str(),&new_meta_index_db_path, None).await.map_err(|e| {
                error!("download remote meta-index-db by {} failed, err:{}", source_url, e);
                RPCErrors::ReasonError(format!("download remote meta-index-db by {} failed, err:{}", source_url, e))
            })?;

            let need_check_chunk_list = self.check_new_remote_index(&new_meta_index_db_path).await;
            if need_check_chunk_list.is_err() {
                error!("check_new_remote_index failed, source:{}", source);
                continue;
            }
            
            let need_check_chunk_list = need_check_chunk_list.unwrap();
            let total_size = need_check_chunk_list.values().map(|info| info.chunk_size).sum::<u64>();
            info!("sync_from_remote_source, start check {} chunks,total size:{}", need_check_chunk_list.len(),total_size);
            for (chunk_id, will_download_pkg_info) in need_check_chunk_list.iter() {
                debug!("sync_from_remote_source, check chunk:{}", chunk_id.as_str());
                //TODO：如何通过防止这些chunk被删除
                let chunk_id = ChunkId::new(chunk_id.as_str()).map_err(|e| {
                    error!("parse chunk_id failed, err:{}", e);
                    RPCErrors::ReasonError(format!("parse chunk_id failed, err:{}", e))
                })?;
                let pull_chunk_result = ndn_client.pull_chunk(chunk_id.clone(),Some("default")).await;
                if pull_chunk_result.is_err() {
                    error!("pull chunk:{} failed, err:{}", chunk_id.to_string(), pull_chunk_result.err().unwrap());
                    continue;
                }
                let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(Some("default")).await.unwrap();
                let named_mgr = named_mgr.lock().await;
                
                named_mgr.set_file(format!("/repo/install_pkg/{}/{}/chunk",will_download_pkg_info.pkg_name,will_download_pkg_info.pkg_version),&chunk_id.to_obj_id(),
                "repo_service","root").await.map_err(|e| {
                    error!("set file failed, err:{}", e);
                    RPCErrors::ReasonError(format!("set file failed, err:{}", e))
                })?;
            }

            let current_meta_index_db_path = self.get_meta_index_db_path(&source);
            RepoServer::replace_file(&current_meta_index_db_path,&new_meta_index_db_path).await?;

        }
        
        Ok(RPCResponse::new(RPCResult::Success(json!({
            "success": true,
        })), req.seq))  
    }

    // pub_pkg(pkg_list) 将pkg_list发布到zone内，是发布操作的第一步
    async fn handle_pub_pkg(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        /*
        将pkg_list发布到zone内，是发布操作的第一步
        Zone内在调用接口前已经将chunk写入repo-servere可访问的named_mgr了
        检查完所有pkg_list都ready后（尤其是其依赖都ready后），通过SQL事务插入一批pkg到 local-wait-meta
        */
        let user_id = ReqHelper::get_user_id(&req)?;
        if !self.settng.enable_dev_mode {
            return Err(RPCErrors::ReasonError("repo_server dev mode is not enabled".to_string()));
        }
        //1）检验参数,得到meta_id:pkg-meta的map
        let pkg_list = req.params.get("pkg_list");
        if pkg_list.is_none() {
            return Err(RPCErrors::ReasonError("pkg_list is none".to_string()));
        }
        
        let owner_pk = CURRENT_ZONE_CONFIG.get().unwrap().auth_key.as_ref().unwrap().clone();
        let pkg_list = pkg_list.unwrap();
        let pkg_meta_jwt_map:HashMap<String,String> = serde_json::from_value(pkg_list.clone()).map_err(|e| {
            error!("parse pkg_list failed, err:{}", e);
            RPCErrors::ReasonError(format!("parse pkg_list failed, err:{}", e))
        })?;
        let mut pkg_meta_map:HashMap<String,PackageMetaNode> = HashMap::new();
        for (meta_obj_id,pkg_meta_jwt) in pkg_meta_jwt_map.iter() {
            let meta_obj_id = ObjId::new(meta_obj_id.as_str()).map_err(|e| {
                error!("parse meta_obj_id failed, err:{}", e);
                RPCErrors::ReasonError(format!("parse meta_obj_id failed, err:{}", e))
            })?;
            let verify_result = verify_named_object_from_jwt(&meta_obj_id,pkg_meta_jwt);
            if verify_result.is_err() {
                error!("verify pkg_meta_jwt failed, err:{}", verify_result.err().unwrap());
                continue;
            }
            let pkg_meta_json = decode_json_from_jwt_with_default_pk(pkg_meta_jwt,&owner_pk).map_err(|e| {
                error!("decode pkg_meta_jwt failed, err:{}", e);
                RPCErrors::ReasonError(format!("decode pkg_meta_jwt failed, err:{}", e))
            })?;
            let pkg_meta:PackageMeta = serde_json::from_value(pkg_meta_json).map_err(|e| {
                error!("parse pkg_meta_jwt failed, err:{}", e);
                RPCErrors::ReasonError(format!("parse pkg_meta_jwt failed, err:{}", e))
            })?;

            if pkg_meta.chunk_id.is_some() {
                let chunk_id = pkg_meta.chunk_id.unwrap();
                let chunk_id = ChunkId::new(chunk_id.as_str()).map_err(|e| {
                    error!("parse chunk_id failed, err:{}", e);
                    RPCErrors::ReasonError(format!("parse chunk_id failed, err:{}", e))
                })?;
                let is_exist = NamedDataMgr::have_chunk(&chunk_id,Some("pub")).await;
                if !is_exist {
                    error!("handle_pub_pkg: {} 's chunk:{:?} not found", pkg_meta.pkg_name.as_str(),chunk_id);
                    return Err(RPCErrors::ReasonError(format!("{} 's chunk:{:?} not found", pkg_meta.pkg_name.as_str(),chunk_id)));
                }
                let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(Some("pub")).await.unwrap();
                let named_mgr = named_mgr.lock().await;
                named_mgr.set_file(format!("/repo/pkg/{}/{}/chunk",pkg_meta.pkg_name,pkg_meta.version),
                &chunk_id.to_obj_id(),"repo_service",user_id.as_str())
                .await.map_err(|e| {
                    error!("handle_pub_pkg: {} 's chunk:{:?} not found", pkg_meta.pkg_name.as_str(),chunk_id);
                    RPCErrors::ReasonError(format!("{} 's chunk:{:?} not found", pkg_meta.pkg_name.as_str(),chunk_id))
                })?;
            }
            let jwk_str = serde_json::to_string(&owner_pk).map_err(|e| {
                error!("serialize owner_pk failed, err:{}", e);
                RPCErrors::ReasonError(format!("serialize owner_pk failed, err:{}", e))
            })?;
            let package_meta_node = PackageMetaNode {
                meta_jwt:pkg_meta_jwt.clone(),
                pkg_name:pkg_meta.pkg_name.clone(),
                version:pkg_meta.version.clone(),
                tag:pkg_meta.tag.clone(),
                author:pkg_meta.author.clone(),
                author_pk:jwk_str,
            };
            pkg_meta_map.insert(meta_obj_id.to_string(),package_meta_node);
        }

        let wait_meta_db_path = self.get_meta_index_db_path("local_wait_pub");
        let wait_meta_db = MetaIndexDb::new(wait_meta_db_path).map_err(|e| {
            error!("new wait-meta-db failed, err:{}", e);
            RPCErrors::ReasonError(format!("new wait-meta-db failed, err:{}", e))
        })?;

        wait_meta_db.add_pkg_meta_batch(&pkg_meta_map).map_err(|e| {
            error!("add pkg_meta_batch failed, err:{}", e);
            RPCErrors::ReasonError(format!("add pkg_meta_batch failed, err:{}", e))
        })?;

        Ok(RPCResponse::new(RPCResult::Success(json!({
            "success": true,
        })), req.seq))
    }

    //将local-wait-meta发布（发布后只读），发布后会计算index-db的chunk_id并构造fileobj,更新R路径的信息
    async fn handle_pub_index(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let mut user_id = "".to_string();
        if !self.settng.enable_dev_mode {
            return Err(RPCErrors::ReasonError("repo_server dev mode is not enabled".to_string()));
        }
        let wait_meta_db_path = self.get_meta_index_db_path("local_wait_pub");
        let local_meta_db_path = self.get_meta_index_db_path("local_pub");

        let mut file_object = FileObject::new("meta_index.db".to_string(),0,String::new());
        NamedDataMgr::pub_local_file_as_fileobj(Some("pub"),&wait_meta_db_path,
        "/repo/meta_index.db","/repo/meta_index.db/content",
        &mut file_object,user_id.as_str(),"repo_service").await.map_err(|e| {
            error!("pub wait-pub-meta-index-db to named-mgr failed, err:{}", e);
            RPCErrors::ReasonError(format!("pub_index failed, err:{}", e))
        })?;

        //对处于NDN中的已发布文件(给外部下载)和本地文件(本地访问) 进行了隔离,未来可以考虑共用
        RepoServer::replace_file(&local_meta_db_path,&wait_meta_db_path).await?;
        Ok(RPCResponse::new(RPCResult::Success(json!({
            "success": true,
        })), req.seq))
    }

    // 更新已发布的local-pub meta-index-db的签名
    async fn handle_sign_index(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let mut user_id = "".to_string();
        if !self.settng.enable_dev_mode {
            return Err(RPCErrors::ReasonError("repo_server dev mode is not enabled".to_string()));
        }

        let meta_index_jwt = ReqHelper::get_str_param_from_req(&req, "meta_index_jwt")?;
        let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(Some("pub")).await;
        if named_mgr.is_none() {
            return Err(RPCErrors::ReasonError("[pub] named-mgr not found".to_string()));
        }
        let named_mgr = named_mgr.unwrap();
        let named_mgr = named_mgr.lock().await;
        let index_obj_id = named_mgr.get_obj_id_by_path("/repo/meta_index.db".to_string()).await.map_err(|e| {
            error!("get obj_id from ndn://repo/meta_index.db failed, err:{}", e);
            RPCErrors::ReasonError(format!("get meta_index_db obj_id failed, err:{}", e))
        })?;
        drop(named_mgr);
        //TODO: 验证meta_index_jwt?是对index_obj_id的签名

        NamedDataMgr::sign_obj(Some("pub"), index_obj_id, meta_index_jwt, user_id.as_str(), "repo_service").await.map_err(|e| {
            error!("sign meta_index_db failed, err:{}", e);
            RPCErrors::ReasonError(format!("sign meta_index_db failed, err:{}", e))
        })?;

        Ok(RPCResponse::new(RPCResult::Success(json!({
            "success": true,
        })), req.seq))
    }

    // 当前repo-service也作为源，有zone外的用户要将pkg发布到meta
    async fn handle_pub_pkg_to_source(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        /*
        因为是处理zone外来的Pkg，所以流程上要稍微复杂一点
        1. 验证身份是合法的
        2. 在pub_pkg_db库里创建发布任务，写入pkg_list，初始状态为已经收到
        2. 检查pkg_list的各种deps已经存在了,失败在发布任务中写入错误信息
       
        */
        let user_id = ReqHelper::get_user_id(&req)?;
        let task_name = ReqHelper::get_str_param_from_req(&req, "task_name")?;
        let real_task_name = format!("pub_pkg_to_source_{}_{}",user_id,task_name);
        let rpc_client = kRPC::new(get_zone_service_url("task_manager",false).as_str(),self.session_token.clone());
        let task_mgr = TaskManager::new(Arc::new(rpc_client));

        let task_data = req.params.get("task_data");
        if task_data.is_none() {
            return Err(RPCErrors::ReasonError("task_data is none".to_string()));
        }
        let task_data = task_data.unwrap();

        let pub_task_data:PubTaskData = serde_json::from_value(task_data.clone()).map_err(|e| {
            error!("parse task_data failed, err:{}", e);
            RPCErrors::ReasonError(format!("parse task_data failed, err:{}", e))
        })?;

        let task_id = task_mgr.create_task(&real_task_name, "pub_pkg_to_source", "repo_service", Some(task_data.clone())).await.map_err(|e| {
            error!("create task failed, err:{}", e);
            RPCErrors::ReasonError(format!("create task failed, err:{}", e))
        })?;

        let ndn_client = NdnClient::new(pub_task_data.author_repo_url.clone(),self.session_token.clone(),None);
        let author_pk = pub_task_data.author_pk.clone();
        //TODO verify author_pk
        // 需要检查和一金认证的author_info是否匹配

        //chunk_id -> (chunk_url,chunk_size)
        let mut total_size = 0;
        let mut will_download_chunk_list:HashMap<String,(String,u64)> = HashMap::new();
        for (meta_obj_id,pkg_meta_jwt) in pub_task_data.pkg_list.iter() {
            let meta_obj_id = ObjId::new(meta_obj_id.as_str()).map_err(|e| {
                error!("parse meta_obj_id failed, err:{}", e);
                RPCErrors::ReasonError(format!("parse meta_obj_id failed, err:{}", e))
            })?;
            let verify_result = verify_named_object_from_jwt(&meta_obj_id,pkg_meta_jwt);
            if verify_result.is_err() {
                error!("verify pkg_meta_jwt failed, err:{}", verify_result.err().unwrap());
                continue;
            }
            let pkg_meta_json = decode_json_from_jwt_with_default_pk(pkg_meta_jwt,&author_pk).map_err(|e| {
                error!("decode pkg_meta_jwt failed, err:{}", e);
                RPCErrors::ReasonError(format!("decode pkg_meta_jwt failed, err:{}", e))
            })?;
            let pkg_meta:PackageMeta = serde_json::from_value(pkg_meta_json).map_err(|e| {
                error!("parse pkg_meta_jwt failed, err:{}", e);
                RPCErrors::ReasonError(format!("parse pkg_meta_jwt failed, err:{}", e))
            })?;
            // TODO:检查pkg_list的各种deps已经存在了,失败在发布任务中写入错误信息
        
            //3. 尝试下载chunkid到本地，失败在发布任务中写入错误信息，下载成功的chunk会关联到正确的path,防止被删除
            //4. 所有的chunk都准备好了，本次发布成功（业务逻辑也可以加入审核流程，手工将发布任务的状态设置为成功）
            if pkg_meta.chunk_id.is_some() {
                let chunk_size = pkg_meta.chunk_size.clone().unwrap_or(0);
                if chunk_size == 0 {
                    error!("chunk_size is 0");
                    continue;
                }
                total_size += chunk_size;
                let chunk_url = pkg_meta.chunk_url.clone().unwrap_or("".to_string());
                will_download_chunk_list.insert(pkg_meta.chunk_id.as_ref().unwrap().clone(),(chunk_url,chunk_size));
            }
        }

        info!("veify pub pkg_list ok, will download {} chunks, total size:{}", will_download_chunk_list.len(), total_size);
        task_mgr.update_task_status(task_id, TaskStatus::Running).await.map_err(|e| {
            error!("update task status failed, err:{}", e);
            RPCErrors::ReasonError(format!("update task status failed, err:{}", e))
        })?;
        task_mgr.update_task_progress(task_id, 0,  total_size).await.map_err(|e| {
            error!("update task progress failed, err:{}", e);
            RPCErrors::ReasonError(format!("update task progress failed, err:{}", e))
        })?;

        let mut completed_size = 0;
        for (chunk_id, (chunk_url, chunk_size)) in will_download_chunk_list.iter() {
            let chunk_id = ChunkId::new(chunk_id.as_str()).map_err(|e| {
                error!("parse chunk_id failed, err:{}", e);
                RPCErrors::ReasonError(format!("parse chunk_id failed, err:{}", e))
            })?;
            let pull_chunk_result = ndn_client.pull_chunk(chunk_id.clone(),Some("pub")).await;
            if pull_chunk_result.is_err() {
                error!("pull chunk:{} failed, err:{}", chunk_id.to_string(), pull_chunk_result.err().unwrap());
                continue;
            }
            completed_size += chunk_size;
            task_mgr.update_task_progress(task_id, completed_size,  total_size).await.map_err(|e| {
                error!("update task progress failed, err:{}", e);
                RPCErrors::ReasonError(format!("update task progress failed, err:{}", e))
            })?;
        }
        
        task_mgr.update_task_status(task_id, TaskStatus::WaitingForApproval).await.map_err(|e| {
            error!("update task status failed, err:{}", e);
            RPCErrors::ReasonError(format!("update task status failed, err:{}", e))
        })?;

        return Ok(RPCResponse::new(RPCResult::Success(json!({
            "task_id": task_id,
        })), req.seq));
    }

    async fn handle_merge_wait_pub_to_source_pkg(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        /*
        合并 `未合并但准备好的` 发布任务里包含的pkg_list到local-wait-meta
        注意merge完后，还要调用pub_index_db发布
        */
        let rpc_client = kRPC::new(get_zone_service_url("task_manager",false).as_str(),self.session_token.clone());
        let task_mgr = TaskManager::new(Arc::new(rpc_client));

        let filter = TaskFilter {
            app_name: Some("repo_service".to_string()),
            task_type: Some("pub_pkg_to_source".to_string()),
            status: Some(TaskStatus::WaitingForApproval),
        };
        let tasks = task_mgr.list_tasks(Some(filter)).await.map_err(|e| {
            error!("list tasks failed, err:{}", e);
            RPCErrors::ReasonError(format!("list tasks failed, err:{}", e))
        })?;

        let wait_meta_db_path = self.get_meta_index_db_path("local_wait_pub");
        let wait_meta_db = MetaIndexDb::new(wait_meta_db_path).map_err(|e| {
            error!("new wait-meta-db failed, err:{}", e);
            RPCErrors::ReasonError(format!("new wait-meta-db failed, err:{}", e))
        })?;

        for task in tasks.iter() {
            let task_data = task.data.as_ref().unwrap();
            let pub_task_data:PubTaskData = serde_json::from_str(task_data.as_str()).map_err(|e| {
                error!("parse task_data failed, err:{}", e);
                RPCErrors::ReasonError(format!("parse task_data failed, err:{}", e))
            })?;

            let mut pkg_meta_map:HashMap<String,PackageMetaNode> = HashMap::new();

            for (meta_obj_id,pkg_meta_jwt) in pub_task_data.pkg_list.iter() {
                let pkg_meta = decode_jwt_claim_without_verify(pkg_meta_jwt).map_err(|e| {
                    error!("decode pkg_meta_jwt failed, err:{}", e);
                    RPCErrors::ReasonError(format!("decode pkg_meta_jwt failed, err:{}", e))
                })?;

                let pkg_meta:PackageMeta = serde_json::from_value(pkg_meta.clone()).map_err(|e| {
                    error!("parse pkg_meta_jwt failed, err:{}", e);
                    RPCErrors::ReasonError(format!("parse pkg_meta_jwt failed, err:{}", e))
                })?;

                let package_meta_node = PackageMetaNode {
                    meta_jwt:pkg_meta_jwt.clone(),
                    pkg_name:pkg_meta.pkg_name.clone(),
                    version:pkg_meta.version.clone(),
                    tag:pkg_meta.tag.clone(),
                    author:pkg_meta.author.clone(),
                    author_pk: serde_json::to_string(&pub_task_data.author_pk).map_err(|e| {
                        error!("serialize author_pk failed, err:{}", e);
                        RPCErrors::ReasonError(format!("serialize author_pk failed, err:{}", e))
                    })?,
                };
                pkg_meta_map.insert(meta_obj_id.to_string(),package_meta_node);
            }

            wait_meta_db.add_pkg_meta_batch(&pkg_meta_map).map_err(|e| {
                error!("add pkg_meta_batch failed, err:{}", e);
                RPCErrors::ReasonError(format!("add pkg_meta_batch failed, err:{}", e))
            })?;

            task_mgr.update_task_status(task.id, TaskStatus::Completed).await.map_err(|e| {
                error!("update task status failed, err:{}", e);
                RPCErrors::ReasonError(format!("update task status failed, err:{}", e))
            })?;
        }

        Ok(RPCResponse::new(RPCResult::Success(json!({
            "success": true,
        })), req.seq))
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

