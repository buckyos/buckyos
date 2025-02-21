use crate::crypto_utils::*;
use crate::def::*;
use crate::downloader::*;
use crate::index_publisher::*;
use crate::source_node::*;
use crate::task_manager::REPO_TASK_MANAGER;
use crate::zone_info_helper::ZoneInfoHelper;
use async_recursion::async_recursion;
use buckyos_kit::get_buckyos_service_data_dir;
use core::hash;
use kRPC::kRPC;
use log::*;
use ndn_lib::{ChunkId, NamedDataMgr};
use package_lib::PackageId;
use serde::ser;
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{Executor, Pool, Sqlite, SqlitePool};
use std::collections::HashMap;
use std::fmt::format;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use sys_config::{SystemConfigClient, SystemConfigError};
use tokio::sync::oneshot::error;
use tokio::{sync::RwLock, task};

const REPO_SERVICE_CONFIG_KEY: &str = "services/repo/settings";

#[derive(PartialEq, Debug, Clone, Copy)]
enum RepoStatus {
    Idle,
    UpdatingIndex,
    Installing(u32), // 表示正在进行的安装计数
}

#[derive(Debug, Clone)]
pub struct SourceManager {
    source_list: Arc<RwLock<Vec<SourceNode>>>,
    status_flag: Arc<Mutex<RepoStatus>>,
}

impl SourceManager {
    pub async fn new() -> RepoResult<Self> {
        let repo_dir = get_buckyos_service_data_dir(SERVICE_NAME);
        if !repo_dir.exists() {
            std::fs::create_dir_all(&repo_dir)?;
        }

        Ok(Self {
            source_list: Arc::new(RwLock::new(Vec::new())),
            status_flag: Arc::new(Mutex::new(RepoStatus::Idle)),
        })
    }

    pub async fn init(&self) -> RepoResult<()> {
        let _ = self.update_index(false).await?;
        Ok(())
    }

    async fn load_index_source_list(&self) -> RepoResult<Vec<SourceNodeConfig>> {
        let rpc_session_token = std::env::var("REPO_SERVICE_SESSION_TOKEN").map_err(|e| {
            error!("repo service session token not found! err:{}", e);
            RepoError::NotReadyError("repo service session token not found!".to_string())
        })?;

        let sys_config_client = SystemConfigClient::new(None, Some(rpc_session_token.as_str()));

        let repo_config = sys_config_client
            .get(REPO_SERVICE_CONFIG_KEY)
            .await
            .map_err(|e| {
                error!("get index source config failed! err:{}", e);
                RepoError::LoadError("index_source".to_string(), e.to_string())
            })?;

        let repo_config = repo_config.0;

        info!("load repo config: {:?}", repo_config);

        let repo_config: serde_json::Value =
            serde_json::from_str(repo_config.as_str()).map_err(|e| {
                error!("parse index_source failed: {:?}", e);
                RepoError::ParseError("index_source".to_string(), e.to_string())
            })?;

        let source_config_list = if repo_config["source_list"].is_array() {
            serde_json::from_value(repo_config["source_list"].clone()).map_err(|e| {
                error!("parse source_list failed: {:?}", e);
                RepoError::ParseError("source_list".to_string(), e.to_string())
            })?
        } else {
            error!("source_list not found in index_source");
            return Err(RepoError::LoadError(
                "source_list".to_string(),
                "source_list not found in index_source".to_string(),
            ));
        };

        Ok(source_config_list)
    }

    async fn save_source_config_list(
        &self,
        source_config_list: &Vec<SourceNodeConfig>,
    ) -> RepoResult<()> {
        let source_config = json!({
            "source_list": source_config_list
        });
        let source_config_str = serde_json::to_string(&source_config).map_err(|e| {
            error!("to_string source_config failed: {:?}", e);
            RepoError::ParseError("source_config_list".to_string(), e.to_string())
        })?;
        info!("save source config list: {}", source_config_str);

        let rpc_session_token = std::env::var("REPO_SERVICE_SESSION_TOKEN").map_err(|e| {
            error!("Repo service session token not found! err:{}", e);
            RepoError::NotReadyError("Repo service session token not found!".to_string())
        })?;

        let sys_config_client = SystemConfigClient::new(None, Some(rpc_session_token.as_str()));

        sys_config_client
            .set(REPO_SERVICE_CONFIG_KEY, &source_config_str)
            .await
            .map_err(|e| {
                error!("Set index source config failed! err:{}", e);
                RepoError::NotReadyError("Set index source config failed!".to_string())
            })?;

        Ok(())
    }

    async fn make_sure_source_file_exists(
        source: &SourceNodeConfig,
        local_file: &PathBuf,
    ) -> RepoResult<()> {
        if !local_file.exists() {
            if source.hostname.is_empty() || source.chunk_id.is_empty() {
                error!("source_config is invalid: {:?}", source);
                return Err(RepoError::ParamError(format!(
                    "source_config is invalid: {:?}",
                    source
                )));
            }
            //TODO 构建chunk的url
            let url = format!("http://{}", source.hostname);
            Downloader::pull_remote_chunk(&url, &source.hostname, &source.jwt, &source.chunk_id)
                .await?;
            Downloader::chunk_to_local_file(&source.chunk_id, None, &local_file).await?;
        }
        Ok(())
    }

    fn local_node_config() -> RepoResult<SourceNodeConfig> {
        //TODO fix 从系统配置中获取
        //let self_did = ZoneInfoHelper::get_zone_did()?;
        let self_name = ZoneInfoHelper::get_zone_name()?;
        Ok(SourceNodeConfig {
            hostname: self_name,
            chunk_id: "".to_string(),
            jwt: "".to_string(),
            version: "".to_string(),
        })
    }

    fn source_db_file(source_config: &SourceNodeConfig, dir: &PathBuf) -> PathBuf {
        //去掉chunkid中冒号之前的部分
        let hex = source_config.chunk_id.split(':').last().unwrap();
        let fix_name = source_config.hostname.replace(":", "-");
        dir.join(format!("index_{}_{}.db", fix_name, hex))
    }

    //todo: 改成标准的NDN FileObject获取逻辑（带验证）
    async fn get_remote_source_meta(source_config: &SourceNodeConfig) -> RepoResult<SourceMeta> {
        //TODO 拼接meta url，要修改成正式url
        let url = format!("http://{}/kapi/repo", source_config.hostname);
        //let url = format!("http://{}/kapi/repo", "127.0.0.1:4000");
        info!("get_remote_source_meta url: {}", url);
        let session_token = std::env::var("REPO_SERVICE_SESSION_TOKEN").map_err(|e| {
            error!("repo service session token not found! err:{}", e);
            RepoError::NotReadyError("repo service session token not found!".to_string())
        })?;
        let rpc_client = kRPC::new(&url, Some(session_token));
        let resp = rpc_client
            .call("query_index_meta", json!({}))
            .await
            .map_err(|e| {
                error!("get_remote_source_meta failed: {:?}", e);
                RepoError::RpcError(format!("get_remote_source_meta failed: {:?}", e))
            })?;

        let source_meta: SourceMeta = serde_json::from_value(resp).map_err(|e| {
            error!("parse source_meta failed: {:?}", e);
            RepoError::ParseError("source_meta".to_string(), e.to_string())
        })?;

        info!(
            "get remote source meta from {:?} success: {:?}",
            source_config.hostname, source_meta
        );

        Ok(source_meta)
    }

    async fn build_source_list(&self, update: bool, task_id: &str) -> RepoResult<()> {
        let mut need_update_config_list = Vec::new();
        let remote_source_db_dir =
            get_buckyos_service_data_dir(SERVICE_NAME).join(REMOTE_INDEX_DIR_NAME);
        if !remote_source_db_dir.exists() {
            std::fs::create_dir_all(&remote_source_db_dir)?;
        }
        let source_config_list = self.load_index_source_list().await?;

        let mut new_source_list = Vec::new();
        // record the source file that need to keep, others will be deleted
        let mut keep_source_file_list = Vec::new();

        for mut source_config in source_config_list {
            if source_config.hostname.is_empty() {
                warn!(
                    "source_config invalid, hostname is empty, {:?}",
                    source_config
                );
                continue;
            }
            if !source_config.chunk_id.is_empty() {
                let source_db_file = Self::source_db_file(&source_config, &remote_source_db_dir);
                if source_db_file.exists() && !update {
                    keep_source_file_list.push(source_db_file.clone());
                    let source_node =
                        SourceNode::new(source_config, source_db_file.clone(), false).await?;
                    new_source_list.push(source_node);
                    continue;
                }
            }
            //通过url请求最新的source_meta
            if update || source_config.chunk_id.is_empty() || source_config.jwt.is_empty() {
                info!("update source meta info from {}", source_config.hostname);
                REPO_TASK_MANAGER.set_task_status(
                    task_id,
                    TaskStatus::Running(format!(
                        "Updating source meta info from {}",
                        source_config.hostname
                    )),
                )?;
                let source_meta = Self::get_remote_source_meta(&source_config).await?;
                if source_meta.chunk_id != source_config.chunk_id {
                    source_config.chunk_id = source_meta.chunk_id;
                    source_config.jwt = source_meta.jwt;
                    source_config.version = source_meta.version;
                    need_update_config_list.push(source_config.clone());
                }
            }
            let source_db_file = Self::source_db_file(&source_config, &remote_source_db_dir);
            if source_db_file.exists() {
                //也许以前下载过?
                info!(
                    "source index file {} exists",
                    source_db_file.to_string_lossy()
                );
                let source_node =
                    SourceNode::new(source_config, source_db_file.clone(), false).await?;
                new_source_list.push(source_node);
                keep_source_file_list.push(source_db_file.clone());
                continue;
            } else {
                info!(
                    "will download source index from {} to {}",
                    source_config.hostname,
                    source_db_file.display()
                );
                REPO_TASK_MANAGER.set_task_status(
                    task_id,
                    TaskStatus::Running(format!(
                        "Downloading source index from {}",
                        source_config.hostname
                    )),
                )?;
                Self::make_sure_source_file_exists(&source_config, &source_db_file).await?;
                let source_node =
                    SourceNode::new(source_config, source_db_file.clone(), false).await?;
                new_source_list.push(source_node);
                keep_source_file_list.push(source_db_file.clone());
            }
        }

        //先更新配置，即使更新完配置异常退出，下次启动时也会根据最新的配置重新下载
        if !need_update_config_list.is_empty() {
            self.save_source_config_list(&need_update_config_list)
                .await?;
        }

        {
            let mut source_list = self.source_list.write().await;
            *source_list = new_source_list;
        }

        //TODO 删除不需要的source文件
        // let source_db_files = std::fs::read_dir(&source_db_dir).map_err(|e| {
        //     error!("read_dir failed: {:?}", e);
        //     RepoError::IoError(format!("read_dir failed: {:?}", e))
        // })?;
        // for entry in source_db_files {
        //     let entry = entry.map_err(|e| {
        //         error!("read_dir entry failed: {:?}", e);
        //         RepoError::IoError(format!("read_dir entry failed: {:?}", e))
        //     })?;
        //     let path = entry.path();
        //     if !path.is_file() || path.file_name().is_none(){
        //         continue;
        //     }
        //     let file_name = path.file_name().unwrap().to_str().unwrap();
        //     if !file_name.starts_with("index_") || !file_name.ends_with(".db") {
        //         continue;
        //     }
        //     if !keep_source_file_list.contains(&path) {
        //         std::fs::remove_file(&path).map_err(|e| {
        //             error!("remove_file failed: {:?}", e);
        //             RepoError::IoError(format!("remove_file failed: {:?}", e))
        //         })?;
        //     }
        // }
        info!("build source list success");

        Ok(())
    }

    //start_source_index 从哪个source开始查找，默认从0开始
    //return (meta_info, source_index), meta_info和在哪个source里找到的， 只有meta_info不为None时，source_index才有意义
    pub async fn get_package_meta(
        &self,
        name: &str,
        version_desc: &str,
        start_source_index: u32,
    ) -> RepoResult<(Option<PackageMeta>, u32)> {
        let source_list = self.source_list.read().await;
        for (index, source) in source_list.iter().enumerate() {
            if index < start_source_index as usize {
                continue;
            }
            let meta_info = source.get_pkg_meta(name, version_desc).await?;
            if meta_info.is_some() {
                return Ok((meta_info, index as u32));
            }
        }
        Ok((None, 0))
    }

    //查找一个包的所有依赖，返回的是一个包的列表
    //查找规则是，按照source_list的顺序，从第一个source开始查找
    //如果一个包的meta信息在某一层里找到，那么可以继续在这一层或者之后的source里找这个包的依赖
    //不能返回到上层去继续找依赖
    //只到所有的依赖都找到算成功
    #[async_recursion]
    pub async fn resolve_dependencies(
        &self,
        name: &str,
        version_desc: &str,
        start_source_index: u32,
        dependencies: &mut Vec<PackageMeta>,
    ) -> RepoResult<()> {
        let (meta_info, source_index) = self
            .get_package_meta(name, version_desc, start_source_index)
            .await?;
        if meta_info.is_none() {
            warn!(
                "package {}-{} not found, start source index: {}",
                name, version_desc, start_source_index
            );
            return Err(RepoError::NotFound(format!(
                "package {}-{}",
                name, version_desc
            )));
        }
        info!(
            "find package {}-{} in source {}",
            name, version_desc, source_index
        );
        let meta_info = meta_info.unwrap();
        let deps = meta_info.dependencies.clone();
        let deps: HashMap<String, String> = serde_json::from_str(&deps).map_err(|e| {
            error!("dependencies from_value failed: {:?}", e);
            RepoError::ParamError(format!(
                "dependencies from_value failed, deps:{} err:{:?}",
                deps, e
            ))
        })?;
        dependencies.push(meta_info);
        for (dep_name, dep_version) in deps.iter() {
            self.resolve_dependencies(dep_name, dep_version, source_index, dependencies)
                .await?;
        }
        Ok(())
    }

    fn try_enter_install_status(&self) -> RepoResult<()> {
        let mut status_flag = self.status_flag.lock().unwrap();
        match *status_flag {
            RepoStatus::Idle => {
                *status_flag = RepoStatus::Installing(1);
                info!("change repo status from idle to installing");
                Ok(())
            }
            RepoStatus::UpdatingIndex => {
                info!("repo status is updating index, can not enter installing status");
                Err(RepoError::StatusError(
                    "Updating index, please try later".to_string(),
                ))
            }
            RepoStatus::Installing(v) => {
                *status_flag = RepoStatus::Installing(v + 1);
                info!(
                    "repo status is installing, increase installing count to {}",
                    v + 1
                );
                Ok(())
            }
        }
    }

    fn leave_install_status(&self) {
        let mut status_flag = self.status_flag.lock().unwrap();
        match *status_flag {
            RepoStatus::Installing(v) => {
                if v == 1 {
                    *status_flag = RepoStatus::Idle;
                    info!("change repo status from installing to idle");
                } else {
                    *status_flag = RepoStatus::Installing(v - 1);
                    info!(
                        "repo status is installing, decrease installing count to {}",
                        v - 1
                    );
                }
            }
            _ => {
                error!(
                    "status_flag is not installing, current status: {:?}",
                    *status_flag
                );
            }
        }
    }

    fn try_enter_index_update_status(&self) -> RepoResult<()> {
        let mut status_flag = self.status_flag.lock().unwrap();
        if *status_flag != RepoStatus::Idle {
            return Err(RepoError::StatusError(format!(
                "Status is {:?}, can not update index",
                *status_flag
            )));
        }
        *status_flag = RepoStatus::UpdatingIndex;
        info!("change repo status from idle to updating index");
        Ok(())
    }

    fn leave_index_update_status(&self) {
        let mut status_flag = self.status_flag.lock().unwrap();
        match *status_flag {
            RepoStatus::UpdatingIndex => {
                *status_flag = RepoStatus::Idle;
                info!("change repo status from updating index to idle");
            }
            _ => {
                error!(
                    "status_flag is not updating index, current status: {:?}",
                    *status_flag
                );
            }
        }
    }

    fn set_task_status(task_id: &str, status: TaskStatus) -> RepoResult<()> {
        let ret = REPO_TASK_MANAGER.set_task_status(task_id, status);
        if let Err(ref e) = ret {
            error!("set_task_status failed: {:?}", e);
        }
        ret
    }

    pub async fn install_pkg(&self, package_id: PackageId) -> RepoResult<String> {
        //如果source_list为空，说明还没有初始化成功，不作多余动作，直接返回错误
        if self.source_list.read().await.is_empty() {
            return Err(RepoError::NotReadyError(
                "index list is not ready".to_string(),
            ));
        }

        self.try_enter_install_status()?;

        let task_id = REPO_TASK_MANAGER.start_install_task(package_id.clone())?;
        let task_id_tmp = task_id.clone();
        let self_clone = self.clone();
        task::spawn(async move {
            match self_clone.do_install(package_id, &task_id_tmp).await {
                Ok(_) => {
                    let _ = Self::set_task_status(&task_id_tmp, TaskStatus::Finished);
                }
                Err(e) => {
                    error!("do_install failed: {:?}", e);
                    let _ = Self::set_task_status(&task_id_tmp, TaskStatus::Error(e.to_string()));
                }
            }
            self_clone.leave_install_status();
        });
        Ok(task_id)
    }

    pub async fn update_index(&self, update: bool) -> RepoResult<String> {
        self.try_enter_index_update_status()?;

        let task_id = REPO_TASK_MANAGER.start_index_update_task()?;
        let task_id_tmp = task_id.clone();
        let self_clone = self.clone();
        task::spawn(async move {
            match self_clone.build_source_list(update, &task_id_tmp).await {
                Ok(_) => {
                    let _ = Self::set_task_status(&task_id_tmp, TaskStatus::Finished);
                }
                Err(e) => {
                    error!("update_index failed: {:?}", e);
                    let _ = Self::set_task_status(&task_id_tmp, TaskStatus::Error(e.to_string()));
                }
            }
            self_clone.leave_index_update_status();
        });
        Ok(task_id)
    }

    pub async fn do_install(&self, package_id: PackageId, task_id: &str) -> RepoResult<()> {
        Self::set_task_status(
            task_id,
            TaskStatus::Running("Resolving dependencies".to_string()),
        )?;
        let version_desc = if let Some(version) = &package_id.version {
            version.clone()
        } else {
            if let Some(sha256) = &package_id.sha256 {
                sha256.clone()
            } else {
                "*".to_string()
            }
        };
        let mut dependencies = vec![];
        self.resolve_dependencies(&package_id.name, &version_desc, 0, &mut dependencies)
            .await?;
        REPO_TASK_MANAGER.set_task_deps(task_id, dependencies.clone())?;
        for dep in dependencies {
            let dep_id = format!("{}#{}", dep.pkg_name, dep.version);
            Self::set_task_status(
                task_id,
                TaskStatus::Running(format!("Downloading {}", dep_id)),
            )?;
            self.pull_pkg(&dep).await?;
        }
        Ok(())
    }

    pub async fn pull_pkg(&self, pkg_meta: &PackageMeta) -> RepoResult<()> {
        if pkg_meta.chunk_id.is_none() {
            return Ok(());
        }

        if self.check_chunk_exist(pkg_meta).await? {
            return Ok(());
        }
        // TODO: fix this url
        let url = format!("http://{}", pkg_meta.hostname);
        Downloader::pull_remote_chunk(
            &url,
            &pkg_meta.hostname,
            &pkg_meta.jwt,
            pkg_meta.chunk_id.as_ref().unwrap(),
        )
        .await
    }

    pub async fn check_chunk_exist(&self, pkg_meta: &PackageMeta) -> RepoResult<bool> {
        let chunk_mgr_id = None;
        debug!("check chunk exist: {:?}", pkg_meta.chunk_id);
        if pkg_meta.chunk_id.is_none() {
            return Ok(true);
        }

        let meta_chunk_id = pkg_meta.chunk_id.as_ref().unwrap();

        let chunk_id = ChunkId::new(meta_chunk_id).map_err(|e| {
            error!("Parse chunk id failed: {:?}", e);
            RepoError::ParseError(meta_chunk_id.clone(), e.to_string())
        })?;
        let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(chunk_mgr_id)
            .await
            .ok_or_else(|| RepoError::NdnError("no chunk mgr".to_string()))?;
        let mut named_mgr = named_mgr.lock().await;
        let ret = named_mgr.is_chunk_exist(&chunk_id).await.map_err(|e| {
            error!("is_chunk_exist failed: {:?}", e);
            RepoError::NdnError(format!("is_chunk_exist failed: {:?}", e))
        })?;
        info!(
            "check chunk {:?} in {:?} exist: {}",
            pkg_meta.chunk_id, chunk_mgr_id, ret
        );
        Ok(ret)
    }

    async fn get_local_index_node(&self) -> RepoResult<SourceNode> {
        let local_dir = get_buckyos_service_data_dir(SERVICE_NAME).join(LOCAL_INDEX_DATA);
        if !local_dir.exists() {
            std::fs::create_dir_all(&local_dir)?;
        }
        //打开local_index.db，如果不存在就创建
        let local_file = local_dir.join(LOCAL_INDEX_DB);
        let source_config = Self::local_node_config()?;
        SourceNode::new(source_config, local_file, true).await
    }

    pub async fn pub_pkg(&self, pkg_meta: &PackageMeta) -> RepoResult<()> {
        //需要确认chunk_id是否已经存在
        if pkg_meta.chunk_id.is_some() {
            if !self.check_chunk_exist(pkg_meta).await? {
                return Err(RepoError::NotFound(format!(
                    "Pub pkg chunk {:?} not exists",
                    pkg_meta.chunk_id
                )));
            }
        }

        let local_index_node = self.get_local_index_node().await.map_err(|e| {
            error!("get_local_index_node failed: {:?}", e);
            RepoError::NdnError(format!("get_local_index_node failed: {:?}", e))
        })?;
        local_index_node.insert_pkg_meta(pkg_meta).await
    }

    pub async fn pub_index(&self, version: &str, hostname: &str, jwt: &str) -> RepoResult<()> {
        IndexPublisher::pub_index(version, hostname, jwt).await
    }

    pub async fn get_index_meta(&self, version: Option<&str>) -> RepoResult<Option<SourceMeta>> {
        IndexPublisher::get_meta(version).await
    }

    pub async fn query_all_latest_pkg(
        &self,
        category: Option<&str>,
    ) -> RepoResult<Vec<PackageMeta>> {
        let source_list = self.source_list.read().await;
        let mut all_latest_pkg = Vec::new();
        for source in source_list.iter() {
            let latest_pkg = source.get_all_latest_pkg(category).await?;
            all_latest_pkg.extend(latest_pkg);
        }
        Ok(all_latest_pkg)
    }
}
