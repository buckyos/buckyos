use async_trait::async_trait;
use log::*;
use ndn_lib::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use core::error;
use std::sync::Arc;
use std::{env, hash::Hash};
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::path::PathBuf;
use std::result::Result;
use std::str::FromStr;
use ::kRPC::*;
use name_lib::{DeviceConfig, ZoneConfig};
use package_lib::*;
use buckyos_kit::buckyos_get_unix_timestamp;
use buckyos_api::*;
use tokio::io::AsyncSeekExt;
use name_lib::*;
use tokio::sync::Mutex as TokioMutex;
use std::sync::LazyLock;

use crate::pkg_task_data::*;
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
}

struct WillDownloadPkgInfo {
    pkg_name:String,
    pkg_version:String,
    chunk_id:String,
    chunk_size:u64,
}

/*
repo-server持有的几个meta-index-db

default meta-index-db (zone内)
 = meta-index-db1 + pub meta-index-db

source-mgr (优先级从高到低)，
    source_name meta-index-db1 (zone内read-only)，或则叫default index-db
    source_name meta-index-db2 (zone内read-only)
    是否应该同步成一个meta-index-db? 按优先级先下载meta-indes-db2,然后下载meta-index-db1后合并到meta-index-db2中（会覆盖相同的版本）

如果打开了开发者模式，则有：
    pub meta-index-db (zone内，zone外read-only) 是下面的local-wait-pub 发布签名后的版本
    local-wait-pub meta-index-db (zone内，zone外可写)
        zone内写:handle_pub_pkg
        zone外写:handle_pub_pkg_to_source
        发布：handle_pub_index，发布完成后local-wait-pub会合并到pub meta-index-db，然后清空


每个node上，默认的pkg-env配置：
    机器级别的root_env (/opt/buckyos/data/repo_services/root/meta-index.db),会更新 default index-db，这个不会实际安装pkg,只是提供index-db
    工作目录的pkg-env,会更新pub meta-index-db ，并继承default index-db,会实际安装pkg

在但OOD模式下
    root_env的default index-db与repo的default index-db是共同的
        

流程的核心理念与git类似
发布pkg到repo: 先commit到当前repo.然后再Merge到remote-repo，merge流程允许手工介入
repo 获得更新: git pull后，根据当前已经安装的app列表下载chunk,全部成功后再切换当前repo的版本
env更新：更新env的index-db到zone内的默认index-db 

install_pkg: 
    会讲

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
    
    //session_token : Option<String>,
}

// 添加一个静态的互斥锁
static DEFAULT_META_INDEX_DB_LOCK: LazyLock<TokioMutex<()>> = LazyLock::new(|| TokioMutex::new(()));

impl RepoServer {
    pub async fn new(config: RepoServerSetting) -> PkgResult<Self> {
        Ok(RepoServer { 
            settng:config
        })
    }
    
    pub async fn init_check(&self) -> Result<(),RPCErrors> {
        let get_result = NamedDataMgr::get_obj_id_by_path(None, "/repo/meta_index.db").await;
        if get_result.is_ok() {
            let default_meta_index_db_path = self.get_my_default_meta_index_db_path();
            if !default_meta_index_db_path.exists() {
                info!("default meta-index-db found, but not set to NDN, bind it");
                let mut file_object = FileObject::new("meta_index.db".to_string(),0,String::new());
                NamedDataMgr::pub_local_file_as_fileobj(None,&default_meta_index_db_path,
                    "/repo/meta_index.db","/repo/meta_index.db/content",
                    &mut file_object,"kernel","repo_service").await.map_err(|e| {
                        error!("pub default meta-index-db to named-mgr failed, err:{}", e);
                        RPCErrors::ReasonError(format!("pub_index failed, err:{}", e))
                    })?;
                info!("pub default meta-index-db to named-mgr success");
            }
        }

        Ok(())
    }

    //成功返回需要下载的chunkid列表
    async fn get_need_chunk_in_remote_index_db(&self,new_meta_index_db_path: &PathBuf) -> Result<HashMap<String,WillDownloadPkgInfo>, RPCErrors> {
        let meta_index_db = MetaIndexDb::new(new_meta_index_db_path.clone(),true).map_err(|e| {
            error!("open new meta-index-db {} failed, err:{}", new_meta_index_db_path.to_string_lossy(), e);
            RPCErrors::ReasonError(format!("open new meta-index-db failed, err:{}", e))
        })?;

        let pin_pkg_list = self.get_pin_pkg_list().await?;
        let mut chunk_list:HashMap<String,WillDownloadPkgInfo> = HashMap::new();
        for (pkg_id,_chunk_id) in pin_pkg_list.iter() {
            let pkg_meta = meta_index_db.get_pkg_meta(pkg_id.as_str());
            if pkg_meta.is_err() {
                let err_string = pkg_meta.err().unwrap().to_string();
                error!("get pkg_meta failed from zone meta-index-db, err:{}", err_string.as_str());
                return Err(RPCErrors::ReasonError(format!("get pkg_meta failed from zone meta-index-db, err:{}", err_string.as_str())));
            }
            let pkg_meta = pkg_meta.unwrap();
            if pkg_meta.is_none() {
                error!("pkg_meta not found, pkg_id:{}", pkg_id);
                return Err(RPCErrors::ReasonError(format!("pkg_meta not found, pkg_id:{}", pkg_id)));
            }
            let (pkg_meta_obj_id,pkg_meta) = pkg_meta.unwrap();
            if pkg_meta.chunk_id.is_some() {
                chunk_list.insert(pkg_meta.chunk_id.clone().unwrap(),WillDownloadPkgInfo {
                    pkg_name:pkg_meta.pkg_name.clone(),
                    pkg_version:pkg_meta.version.clone(),
                    chunk_id:pkg_meta.chunk_id.clone().unwrap(),
                    chunk_size:pkg_meta.chunk_size.clone().unwrap(),
                });
            }
        }
        
        return Ok(chunk_list);
    }

    fn get_source_meta_index_db_path(&self,source: &str) -> PathBuf {
        if source == "root" {
            return self.get_my_default_meta_index_db_path();
        }

        let runtime = get_buckyos_api_runtime().unwrap();
        let path = runtime.get_my_data_folder().join(source).join("meta_index.db");
        return path;
    }

    fn get_my_pub_meta_index_db_path(&self) -> PathBuf {
        let runtime = get_buckyos_api_runtime().unwrap();
        let path = runtime.get_my_data_folder().join("pub_meta_index.db");
        return path;
    }

    fn get_my_wait_pub_meta_index_db_path(&self) -> PathBuf {
        let runtime = get_buckyos_api_runtime().unwrap();
        let path = runtime.get_my_data_folder().join("wait_pub_meta_index.db");
        return path;
    }

    fn get_my_default_meta_index_db_path(&self) -> PathBuf {
        let runtime = get_buckyos_api_runtime().unwrap();
        let path = runtime.get_my_data_folder().join("default_meta_index.db");
        return path;
    }

    async fn try_create_wait_pub_meta_index_db(&self) -> Result<(),RPCErrors> {
        let wait_pub_meta_index_db_path = self.get_my_wait_pub_meta_index_db_path();
        
        // 检查文件是否存在
        if !wait_pub_meta_index_db_path.exists() {
            info!("Wait pub meta index db not exists, creating it by copying from pub meta index db");
            let the_db = MetaIndexDb::new(wait_pub_meta_index_db_path,false).map_err(|e| {
                error!("Failed to create wait pub meta index db: {}", e);
                RPCErrors::ReasonError(format!("Failed to create wait pub meta index db: {}", e))
            })?;
    
        }
        Ok(())
    }

    async fn copy_file(target_file_path:&PathBuf,file_path:&PathBuf) -> Result<(),RPCErrors> {
        tokio::fs::copy(file_path, target_file_path).await.map_err(|e| {
            error!("Failed to copy file, err: {}", e);
            RPCErrors::ReasonError(format!("Failed to copy file, err: {}", e))
        })?;
        Ok(())
    }
    
    
    async fn replace_file(target_file_path:&PathBuf,file_path:&PathBuf) -> Result<(),RPCErrors> {
        // 原子地替换文件，类似于 move 操作
        let target_dir = target_file_path.parent().ok_or_else(|| {
            error!("Cannot get target file's parent directory");
            RPCErrors::ReasonError("Cannot get target file's parent directory".to_string())
        })?;

        // 确保目标目录存在
        if !target_dir.exists() {
            tokio::fs::create_dir_all(target_dir).await.map_err(|e| {
                error!("Failed to create target directory, err: {}", e);
                RPCErrors::ReasonError(format!("Failed to create target directory, err: {}", e))
            })?;
        }

        // 在类 Unix 系统上，rename 操作是原子的
        // 在 Windows 上，如果目标文件已存在，rename 会失败，所以需要先删除目标文件
        if target_file_path.exists() {
            tokio::fs::remove_file(target_file_path).await.map_err(|e| {
                error!("Failed to remove target file, err: {}", e);
                RPCErrors::ReasonError(format!("Failed to remove target file, err: {}", e))
            })?;
        }

        // 执行重命名操作（相当于 move）
        tokio::fs::rename(file_path, target_file_path).await.map_err(|e| {
            error!("Failed to replace file, err: {}", e);
            RPCErrors::ReasonError(format!("Failed to replace file, err: {}", e))
        })?;

        Ok(())
    }

    async fn pin_pkg_list(&self,pkg_list: &HashMap<String,String>) -> Result<(),RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        let sys_config_client = runtime.get_system_config_client().await?;
        let sys_config_path = runtime.get_my_sys_config_path("pkg_list");
        let mut current_pkg_list:HashMap<String,String> = HashMap::new();
        let current_pkg_list_str = sys_config_client.get(&sys_config_path).await;
        if current_pkg_list_str.is_ok() {
            let (current_pkg_list_str,_version) = current_pkg_list_str.unwrap();
            current_pkg_list = serde_json::from_str(&current_pkg_list_str).map_err(|e| {
                error!("Failed to parse current pkg list, err: {}", e);
                RPCErrors::ReasonError(format!("Failed to parse current pkg list, err: {}", e))
            })?;
        }

        for (pkg_id,chunk_id) in pkg_list.iter() {
            current_pkg_list.insert(pkg_id.clone(),chunk_id.clone());
        }

        let current_pkg_list_str = serde_json::to_string(&current_pkg_list).map_err(|e| {
            error!("Failed to serialize current pkg list, err: {}", e);
            RPCErrors::ReasonError(format!("Failed to serialize current pkg list, err: {}", e))
        })?;
        sys_config_client.set(&sys_config_path,&current_pkg_list_str).await
        .map_err(|e| {
            error!("set sys_config failed, err:{}", e);
            RPCErrors::ReasonError(format!("set sys_config failed, err:{}", e))
        })?;
        //let pkg_list_path = sys_config_client.get_pkg_list_path().await?;

        let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(None).await.unwrap();
        let named_mgr = named_mgr.lock().await;
        for (pkg_id,chunk_id) in pkg_list.iter() {
            let chunk_obj_id = ObjId::new(chunk_id.as_str()).map_err(|e| {
                error!("parse chunk_id failed, err:{}", e);
                RPCErrors::ReasonError(format!("parse chunk_id failed, err:{}", e))
            })?;
            named_mgr.set_file_impl(format!("/repo/install_pkg/{}/{}",pkg_id,chunk_id).as_str(),&chunk_obj_id,
            "repo_service","root").await
            .map_err(|e| {
                error!("set NDN file failed, err:{}", e);
                RPCErrors::ReasonError(format!("set NDN file failed, err:{}", e))
            })?;
        }
        Ok(())
    }

    async fn get_pin_pkg_list(&self) -> Result<HashMap<String,String>,RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        let sys_config_client = runtime.get_system_config_client().await?;
        let sys_config_path = runtime.get_my_sys_config_path("pkg_list");
        let current_pkg_list_str = sys_config_client.get(&sys_config_path).await;
        if current_pkg_list_str.is_err() {
            error!("get pin pkg list failed, err:{}", current_pkg_list_str.err().unwrap());
            return Err(RPCErrors::ReasonError("get pin pkg list failed".to_string()));
        }
        let (current_pkg_list_str,_version) = current_pkg_list_str.unwrap();
        let current_pkg_list = serde_json::from_str(&current_pkg_list_str).map_err(|e| {
            error!("Failed to parse current pkg list, err: {}", e);
            RPCErrors::ReasonError(format!("Failed to parse current pkg list, err: {}", e))
        })?;

        return Ok(current_pkg_list);
    }

    //在repo里安装pkg(pkg_name已经在index-db中了)
    async fn handle_install_pkg(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let pkg_list = req.params.get("pkg_list");
        let install_task_name = ReqHelper::get_str_param_from_req(&req,"task_name")?;
        let session_token = ReqHelper::get_session_token(&req)?;
        if pkg_list.is_none() {
            return Err(RPCErrors::ReasonError("pkg_list is none".to_string()));
        }
        let pkg_list = pkg_list.unwrap();
        //pkg_list: {"pkg_id":"chunkid"}
        let pkg_list : HashMap<String,String> = serde_json::from_value(pkg_list.clone()).map_err(|e| {
            error!("parse pkg_list failed, err:{}", e);
            RPCErrors::ReasonError(format!("parse pkg_list failed, err:{}", e))
        })?;

        let root_meta_db = self.get_source_meta_index_db_path("root");
        let root_meta_db = MetaIndexDb::new(root_meta_db,true).map_err(|e| {
            error!("open root meta-index-db failed, err:{}", e);
            RPCErrors::ReasonError(format!("open root meta-index-db failed, err:{}", e))
        })?;

        let source_url = self.settng.remote_source.get("root");
        if source_url.is_none() {
            error!("handle_install_pkg error:root source not found");
            return Err(RPCErrors::ReasonError("root source not found".to_string()));
        }
        let source_url = source_url.unwrap();

        let mut total_size = 0;
        for (pkg_id,will_install_chunk_id) in pkg_list.iter() {
            let pkg_meta = root_meta_db.get_pkg_meta(pkg_id.as_str());
            if pkg_meta.is_err() {
                error!("get pkg_meta failed from zone meta-index-db, err:{}", pkg_meta.err().unwrap());
                return Err(RPCErrors::ReasonError("get pkg_meta failed".to_string()));
            }
            let pkg_meta = pkg_meta.unwrap();
            if pkg_meta.is_none() {
                error!("pkg_meta not found, pkg_id:{}", pkg_id);
                return Err(RPCErrors::ReasonError(format!("pkg_meta not found, pkg_id:{}", pkg_id)));
            }
            let (pkg_meta_obj_id,pkg_meta) = pkg_meta.unwrap();
            if pkg_meta.chunk_id.is_none() {
                error!("pkg_meta not found, pkg_id:{}", pkg_id);
                return Err(RPCErrors::ReasonError(format!("pkg_meta not found, pkg_id:{}", pkg_id)));
            }
            let chunk_id = pkg_meta.chunk_id.unwrap();
            total_size += pkg_meta.chunk_size.unwrap();
            if chunk_id.as_str() != will_install_chunk_id {
                error!("chunk_id not match, pkg_id:{}", pkg_id);
                return Err(RPCErrors::ReasonError(format!("chunk_id not match, pkg_id:{}", pkg_id)));
            }
        }
        info!("will install pkg_list, total size:{}", total_size);

        let runtime = get_buckyos_api_runtime()?;
        let task_mgr_client = runtime.get_task_mgr_client().await?;
        let install_pkg_task_data = InstallPkgTaskData {
            pkg_list: pkg_list.clone(),
        };
        let install_pkg_task_data = serde_json::to_value(&install_pkg_task_data).unwrap();
        //todo user_id,app_name应该是来自req的app_name
        let task_id = task_mgr_client.create_task(install_task_name.as_str(),
        "install_pkg",
        "repo_service",
        Some(install_pkg_task_data)).await.map_err(|e| {
            error!("create install_pkg task failed, err:{}", e);
            RPCErrors::ReasonError(format!("create install_pkg task failed, err:{}", e))
        })?;

        //save pkg_list to repo's pin pkg list
        let pin_result = self.pin_pkg_list(&pkg_list).await;
        if pin_result.is_err() {
            let err_string = pin_result.err().unwrap().to_string();
            error!("pin pkg_list failed, err:{}", err_string.as_str());
            task_mgr_client.update_task_error(task_id,err_string.as_str()).await.map_err(|e| {
                error!("update task error failed, err:{}", e);
                return;
            });
        }

        let ndn_client = NdnClient::new(source_url.clone(),None,None);
        let mut completed_size = 0;
        tokio::spawn(async move {
            for (pkg_id,will_install_chunk_id) in pkg_list.iter() {
                //let named_mgr = named_mgr.lock().await;
                let chunk_id = ChunkId::new(will_install_chunk_id.as_str()).unwrap();
                //TODO chunk_url? 
                let pull_result = ndn_client.pull_chunk(chunk_id.clone(),None).await;
    
                if pull_result.is_ok() {
                    let chunk_size = pull_result.unwrap();
                    completed_size += chunk_size;
                    info!("install pkg task pull chunk {} success, completed_size:{}", chunk_id.to_string(), completed_size);
                    task_mgr_client.update_task_progress(task_id,completed_size,total_size).await.map_err(|e| {
                        error!("update task progress failed, err:{}", e);
                        return;
                    });
                } else {
                    let err_string = pull_result.err().unwrap().to_string();
                    error!("pull chunk failed, err:{}", err_string.as_str());
                    task_mgr_client.update_task_error(task_id,err_string.as_str()).await.map_err(|e| {
                        error!("update task error failed, err:{}", e);
                        return;
                    });
                    return;
                }
            }

            info!("install pkg task completed, task_id:{}", task_id);
            task_mgr_client.update_task_status(task_id,TaskStatus::Completed).await.map_err(|e| {
                error!("update task success failed, err:{}", e);
                return;
            });
        });

        Ok(RPCResponse::new(RPCResult::Success(json!({
            "task_id": task_id,
        })), req.seq))
    }
    // 将source-meta-index更新到最新版本
    async fn handle_sync_from_remote_source(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        // 该操作可能会修改default_meta_index_db_path，需要加锁
        let _lock = DEFAULT_META_INDEX_DB_LOCK.lock().await;
        //尝试拿到sync操作的锁，拿不到则说明已经在处理了
        //1.先下载并验证远程版本到临时db
        //let will_update_source_list = self.settng.remote_source.keys().cloned();

        let root_source_url = self.settng.remote_source.get("root");
        if root_source_url.is_none() {
            error!("handle_sync_from_remote_source error:root source not found");
            return Err(RPCErrors::ReasonError("root source not found".to_string()));
        }
        let root_source_url = root_source_url.unwrap();

        info!("update meta-index-db:source {}, download url:{}", "root", root_source_url);
        let new_meta_index_db_path = self.get_my_default_meta_index_db_path().with_extension("download");


        let runtime = get_buckyos_api_runtime()?;
        let session_token = runtime.get_session_token().await;
        let ndn_client = NdnClient::new(root_source_url.clone(),Some(session_token),None);

        let is_better = ndn_client.local_is_better(root_source_url.as_str(),&self.get_my_default_meta_index_db_path()).await;
        if is_better.is_ok() && is_better.unwrap() {
            info!("local meta-index-db is better than remote, will not download");
            return Ok(RPCResponse::new(RPCResult::Success(json!({
                "success": true,
            })), req.seq));
        }
        
        ndn_client.download_fileobj_to_local(root_source_url.as_str(),&new_meta_index_db_path, None).await.map_err(|e| {
            error!("download remote meta-index-db by {} failed, err:{}", root_source_url, e);
            RPCErrors::ReasonError(format!("download remote meta-index-db by {} failed, err:{}", root_source_url, e))
        })?;

        let need_check_chunk_list = self.get_need_chunk_in_remote_index_db(&new_meta_index_db_path).await;
        if need_check_chunk_list.is_err() {
            error!("check_new_remote_index failed, source:{}", "root");
            return Err(RPCErrors::ReasonError("check_new_remote_index failed".to_string()));
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
            let pull_chunk_result = ndn_client.pull_chunk(chunk_id.clone(),None).await;
            if pull_chunk_result.is_err() {
                error!("pull chunk:{} failed, err:{}", chunk_id.to_string(), pull_chunk_result.err().unwrap());
                continue;
            }
            let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(None).await.unwrap();
            let named_mgr = named_mgr.lock().await;
            //下面的操作并不会让旧版本失效，后续需要通过一个完整的重建 /repo/install_pkg的操作来释放
            named_mgr.set_file_impl(format!("/repo/install_pkg/{}/{}/chunk",will_download_pkg_info.pkg_name,will_download_pkg_info.pkg_version).as_str(),&chunk_id.to_obj_id(),
            "repo_service","root").await.map_err(|e| {
                error!("set file failed, err:{}", e);
                RPCErrors::ReasonError(format!("set file failed, err:{}", e))
            })?;
        }

        let current_meta_index_db_path = self.get_my_default_meta_index_db_path();
        RepoServer::replace_file(&current_meta_index_db_path,&new_meta_index_db_path).await?;

        self.create_new_default_meta_index_db("root").await?;

        Ok(RPCResponse::new(RPCResult::Success(json!({
            "success": true,
        })), req.seq))  
    }

    async fn create_new_default_meta_index_db(&self,user_id: &str) -> Result<(),RPCErrors> {
        // 获取锁，确保只有一个调用可以进入临界区
        //et _lock = DEFAULT_META_INDEX_DB_LOCK.lock().await;
        
        let default_meta_index_db_path = self.get_my_default_meta_index_db_path();
        let default_source_meta_index_db_path = self.get_source_meta_index_db_path("root");
        let pub_meta_index_db_path = self.get_my_pub_meta_index_db_path();
        info!("will create_new_default_meta_index_db, default_meta_index_db_path:{}", default_meta_index_db_path.display());
        let mut have_result = false;
        let mut need_merge_source_meta_index_db = false;

        if default_source_meta_index_db_path.exists() {
            info!("default_source_meta_index_db_path exists, will replace default_meta_index_db_path with default_source_meta_index_db_path");
            RepoServer::copy_file(&default_meta_index_db_path,&default_source_meta_index_db_path).await?;
            have_result = true;
            need_merge_source_meta_index_db = true;
        }

        if pub_meta_index_db_path.exists() {
            info!("pub_meta_index_db_path exists, will merge pub_meta_index_db_path with default_meta_index_db_path");
            have_result = true;
            if need_merge_source_meta_index_db {
                info!("need_merge_source_meta_index_db, will merge pub_meta_index_db_path with default_meta_index_db_path");
                let default_meta_index_db = MetaIndexDb::new(default_meta_index_db_path,false).map_err(|e| {
                    error!("open default meta-index-db failed, err:{}", e);
                    RPCErrors::ReasonError(format!("open default meta-index-db failed, err:{}", e))
                })?;
                default_meta_index_db.merge_meta_index_db(&pub_meta_index_db_path).await
                    .map_err(|e| {
                        error!("merge meta-index-db failed, err:{}", e);
                        RPCErrors::ReasonError(format!("merge meta-index-db failed, err:{}", e))
                    })?;   
            } else {
                info!("no need_merge_source_meta_index_db, will replace default_meta_index_db_path with pub_meta_index_db_path");
                RepoServer::copy_file(&default_meta_index_db_path,&pub_meta_index_db_path).await?;
            }
        }

        if !have_result {
            error!("no source-meta-index and no pub-meta-index, there is no record can write to meta-index-db found");
            return Err(RPCErrors::ReasonError("no record can write to meta-index-db ".to_string()));
        }

        //TODO: 将meta-index-db发布到named-mgr
        //info!("will pub new default meta-index-db to named-mgr");
        let mut file_object = FileObject::new("meta_index.db".to_string(),0,String::new());
        let default_meta_index_db_path = self.get_my_default_meta_index_db_path();
        NamedDataMgr::pub_local_file_as_fileobj(None,&default_meta_index_db_path,
            "/repo/meta_index.db","/repo/meta_index.db/content",
            &mut file_object,user_id,"repo_service").await.map_err(|e| {
                error!("pub default meta-index-db to named-mgr failed, err:{}", e);
                RPCErrors::ReasonError(format!("pub_index failed, err:{}", e))
            })?;
        info!("pub new default meta-index-db to named-mgr success");

        Ok(())
    }

    // pub_pkg(pkg_list) 将pkg_list发布到zone内，是发布操作的第一步
    async fn handle_pub_pkg(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        /*
        将pkg_list发布到zone内，是发布操作的第一步
        Zone内在调用接口前已经将chunk写入repo-servere可访问的named_mgr了
        检查完所有pkg_list都ready后（尤其是其依赖都ready后），通过SQL事务插入一批pkg到 local-wait-meta
        */
        let user_id = "root".to_string();
        if !self.settng.enable_dev_mode {
            return Err(RPCErrors::ReasonError("repo_server dev mode is not enabled".to_string()));
        }
        //1）检验参数,得到meta_id:pkg-meta的map
        let pkg_list = req.params.get("pkg_list");
        if pkg_list.is_none() {
            return Err(RPCErrors::ReasonError("pkg_list is none".to_string()));
        }
        let pkg_list = pkg_list.unwrap();
        let owner_pk = CURRENT_ZONE_CONFIG.get().unwrap().get_default_key().unwrap();
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
                let is_exist = NamedDataMgr::have_chunk(&chunk_id,None).await;
                if !is_exist {
                    error!("handle_pub_pkg: {} 's chunk:{} not found", pkg_meta.pkg_name.as_str(),chunk_id.to_string());
                    return Err(RPCErrors::ReasonError(format!("{} 's chunk:{} not found", pkg_meta.pkg_name.as_str(),chunk_id.to_string())));
                }

                //这个路径保存的是自己发布的pkg
                NamedDataMgr::set_file(None,format!("/repo/pkg/{}/{}/chunk",pkg_meta.pkg_name.as_str(),pkg_meta.version.as_str()).as_str(),
                &chunk_id.to_obj_id(),"repo_service",user_id.as_str())
                .await.map_err(|e| {
                    error!("handle_pub_pkg: {} 's chunk:{} not found", pkg_meta.pkg_name.as_str(),chunk_id.to_string());
                    RPCErrors::ReasonError(format!("{} 's chunk:{} not found", pkg_meta.pkg_name.as_str(),chunk_id.to_string()))
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
        info!("pkg_list check success, will pub pkg_list to local-wait-pub-meta-index-db");
        //self.try_create_wait_pub_meta_index_db().await?;
        let wait_meta_db_path = self.get_my_wait_pub_meta_index_db_path();
        let wait_meta_db = MetaIndexDb::new(wait_meta_db_path,false).map_err(|e| {
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
        // 该操作可能会修改default_meta_index_db_path，需要加锁
        let _lock = DEFAULT_META_INDEX_DB_LOCK.lock().await;
        let mut user_id = "".to_string();
        if !self.settng.enable_dev_mode {
            return Err(RPCErrors::ReasonError("repo_server dev mode is not enabled".to_string()));
        }
        let wait_meta_db_path = self.get_my_wait_pub_meta_index_db_path();
        let pub_meta_db_path = self.get_my_pub_meta_index_db_path();
        //info!("start replace pub_meta_db with wait_meta_db, pub_meta_db_path:{}", pub_meta_db_path.display());
        RepoServer::copy_file(&pub_meta_db_path,&wait_meta_db_path).await?;
        info!("copy wait_meta_db to pub_meta_db success");
        let mut file_object = FileObject::new("pub_meta_index.db".to_string(),0,String::new());
        //info!("will pub pub_meta_index to named-mgr");
        NamedDataMgr::pub_local_file_as_fileobj(None,&pub_meta_db_path,
        "/repo/pub_meta_index.db","/repo/pub_meta_index.db/content",
        &mut file_object,user_id.as_str(),"repo_service").await.map_err(|e| {
            error!("pub pub-meta-index-db to named-mgr failed, err:{}", e);
            RPCErrors::ReasonError(format!("pub_index failed, err:{}", e))
        })?;
        info!("pub pub_meta_index.db to named-mgr success");
        self.create_new_default_meta_index_db("root").await?;


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

        let (index_obj_id,_path_obj_jwt) = NamedDataMgr::get_obj_id_by_path(None,"/repo/pub_meta_index.db").await.map_err(|e| {
            error!("get obj_id from ndn://repo/pub_meta_index.db failed, err:{}", e);
            RPCErrors::ReasonError(format!("get meta_index_db obj_id failed, err:{}", e))
        })?;
        //TODO: 验证meta_index_jwt?是对index_obj_id的签名

        NamedDataMgr::sign_obj(None, index_obj_id, meta_index_jwt, user_id.as_str(), "repo_service").await.map_err(|e| {
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
        let user_id = "root".to_string();
        let task_name = ReqHelper::get_str_param_from_req(&req, "task_name")?;
        let real_task_name = format!("pub_pkg_to_source_{}_{}",user_id,task_name);
        let runtime = get_buckyos_api_runtime()?;
        let task_mgr = runtime.get_task_mgr_client().await?;

        let task_data = req.params.get("task_data");
        if task_data.is_none() {
            return Err(RPCErrors::ReasonError("task_data is none".to_string()));
        }
        let task_data = task_data.unwrap();

        let pub_task_data:PubPkgTaskData = serde_json::from_value(task_data.clone()).map_err(|e| {
            error!("parse task_data failed, err:{}", e);
            RPCErrors::ReasonError(format!("parse task_data failed, err:{}", e))
        })?;

        let task_id = task_mgr.create_task(&real_task_name, "pub_pkg_to_source", "repo_service", Some(task_data.clone())).await.map_err(|e| {
            error!("create task failed, err:{}", e);
            RPCErrors::ReasonError(format!("create task failed, err:{}", e))
        })?;

        let session_token = runtime.get_session_token().await;
        let ndn_client = NdnClient::new(pub_task_data.author_repo_url.clone(),Some(session_token),None);
        let author_pk = pub_task_data.author_pk.clone();
        //TODO verify author_pk, 源服务应该由类似SN的账号体系

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
            let pull_chunk_result = ndn_client.pull_chunk(chunk_id.clone(),None).await;
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

    async fn handle_merge_wait_pub(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        /*
        合并 `未合并但准备好的` 发布任务里包含的pkg_list到local-wait-meta
        注意merge完后，还要调用pub_index_db发布
        */
        let runtime = get_buckyos_api_runtime()?;
        let task_mgr = runtime.get_task_mgr_client().await?;

        let filter = TaskFilter {
            app_name: Some("repo_service".to_string()),
            task_type: Some("pub_pkg_to_source".to_string()),
            status: Some(TaskStatus::WaitingForApproval),
        };
        let tasks = task_mgr.list_tasks(Some(filter)).await.map_err(|e| {
            error!("list tasks failed, err:{}", e);
            RPCErrors::ReasonError(format!("list tasks failed, err:{}", e))
        })?;

        let wait_meta_db_path = self.get_my_wait_pub_meta_index_db_path();
        let wait_meta_db = MetaIndexDb::new(wait_meta_db_path,false).map_err(|e| {
            error!("new wait-meta-db failed, err:{}", e);
            RPCErrors::ReasonError(format!("new wait-meta-db failed, err:{}", e))
        })?;

        for task in tasks.iter() {
            let task_data = task.data.as_ref().unwrap();
            let pub_task_data:PubPkgTaskData = serde_json::from_str(task_data.as_str()).map_err(|e| {
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
impl InnerServiceHandler for RepoServer {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        ip_from: IpAddr,
    ) -> Result<RPCResponse, RPCErrors> {
        match req.method.as_str() {
            //"query_all_latest_pkg" => self.handle_query_all_latest_pkg(req).await,
            "install_pkg" => self.handle_install_pkg(req).await,
            "pub_pkg" => self.handle_pub_pkg(req).await,
            "pub_index" => self.handle_pub_index(req).await,
            "pub_pkg_to_source" => self.handle_pub_pkg_to_source(req).await,
            "merge_wait_pub" => self.handle_merge_wait_pub(req).await,
            "sync_from_remote_source" => self.handle_sync_from_remote_source(req).await,
            //"query_index_meta" => self.handle_query_index_meta(req).await,
            //"query_task" => self.handle_query_task(req).await,
            _ => {
                error!("Unknown method:{}", req.method);
                Err(RPCErrors::UnknownMethod(req.method))
            }
        }
    }

    async fn handle_http_get(&self, req_path:&str,ip_from:IpAddr) -> Result<String,RPCErrors> {
        return Err(RPCErrors::UnknownMethod(req_path.to_string()));
    }
}

//TODO:需要补充设计一些单元测试
