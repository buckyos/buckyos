use crate::def::*;
use crate::downloader::*;
use crate::error::*;
use crate::source_node::*;
use crate::task_manager::REPO_TASK_MANAGER;
use crate::verifier::*;
use async_recursion::async_recursion;
use buckyos_kit::get_buckyos_service_data_dir;
use kv::source;
use log::warn;
use log::*;
use ndn_lib::ChunkId;
use package_lib::PackageId;
use rusqlite::{params, Connection};
use serde::ser;
use sqlx::{Executor, Pool, Sqlite, SqlitePool};
use std::collections::HashMap;
use std::fmt::format;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::task;

#[derive(Debug, Clone)]
pub struct SourceManager {
    pub source_list: Arc<RwLock<Vec<SourceNode>>>,
    pub pool: SqlitePool,
}

impl SourceManager {
    pub async fn new() -> RepoResult<Self> {
        let repo_dir = get_buckyos_service_data_dir(SERVICE_NAME);
        if !repo_dir.exists() {
            std::fs::create_dir_all(&repo_dir)?;
        }
        let db_url = format!(
            "sqlite://{}/{}",
            repo_dir.to_str().unwrap(),
            REPO_SOURCE_CONFIG_DB
        );
        let pool = SqlitePool::connect(&db_url).await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS source_node (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL DEFAULT '',
                url TEXT NOT NULL DEFAULT '',
                author TEXT NOT NULL,
                chunk_id TEXT NOT NULL DEFAULT '',
                sign TEXT NOT NULL DEFAULT '',
            )",
        )
        .execute(&pool)
        .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_source_node_name ON source_node (name)")
            .execute(&pool)
            .await?;

        sqlx::query(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_source_node_chunk_id, ON source_node (chunk_id)",
        )
        .execute(&pool)
        .await?;

        Ok(Self {
            source_list: Arc::new(RwLock::new(Vec::new())),
            pool,
        })
    }

    async fn load_source_config_list(&self) -> RepoResult<Vec<SourceNodeConfig>> {
        let source_configs = sqlx::query_as::<_, SourceNodeConfig>(
            "SELECT id, name, url, author, chunk_id, sign FROM source_node",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(source_configs)
    }

    async fn save_source_config_list(
        &self,
        source_config_list: &Vec<SourceNodeConfig>,
    ) -> RepoResult<()> {
        let mut tx = self.pool.begin().await?;
        for source_config in source_config_list {
            sqlx::query(
                "INSERT OR REPLACE INTO source_node (id, name, url, author, chunk_id, sign) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .bind(source_config.id)
            .bind(&source_config.name)
            .bind(&source_config.url)
            .bind(&source_config.author)
            .bind(&source_config.chunk_id)
            .bind(&source_config.sign)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;

        Ok(())
    }

    async fn make_sure_source_file_exists(
        url: &str,
        author: &str,
        chunk_id: &str,
        sign: &str,
        local_file: &PathBuf,
    ) -> RepoResult<()> {
        if !local_file.exists() {
            //从source.url请求对应的chunk_id
            pull_remote_chunk(url, author, sign, chunk_id, REPO_CHUNK_MGR_ID).await?;
            chunk_to_local_file(&chunk_id, REPO_CHUNK_MGR_ID, &local_file).await?;
        }
        Ok(())
    }

    fn local_node_config() -> SourceNodeConfig {
        SourceNodeConfig {
            id: 0,
            name: "local".to_string(),
            url: "".to_string(),
            author: "".to_string(),
            chunk_id: "".to_string(),
            sign: "".to_string(),
        }
    }

    fn source_db_file(source_config: &SourceNodeConfig, dir: &PathBuf) -> PathBuf {
        dir.join(format!(
            "{}_{}.db",
            source_config.name, source_config.chunk_id
        ))
    }

    async fn get_remote_source_meta(url: &str) -> RepoResult<SourceMeta> {
        unimplemented!("get_remote_source_meta")
    }

    async fn build_source_list(&self, update: bool) -> RepoResult<()> {
        let mut need_update_config_list = Vec::new();
        let source_db_dir = get_buckyos_service_data_dir(SERVICE_NAME).join(INDEX_DIR_NAME);
        let source_config_list = self.load_source_config_list().await?;
        let mut new_source_list = Vec::new();
        //先添加一个本地的source，特殊处理
        let local_source_config = Self::local_node_config();
        let local_source_file = source_db_dir.join(LOCAL_INDEX_DB);
        new_source_list.push(SourceNode::new(local_source_config, local_source_file, true).await?);

        for mut source_config in source_config_list {
            if source_config.url.is_empty() || source_config.author.is_empty() {
                warn!("source_config invalid, {:?}", source_config);
                continue;
            }
            let source_db_file = Self::source_db_file(&source_config, &source_db_dir);
            if source_db_file.exists() && !update {
                let source_node =
                    SourceNode::new(source_config, source_db_file.clone(), false).await?;
                new_source_list.push(source_node);
                continue;
            }
            //通过url请求最新的source_meta
            if update || source_config.chunk_id.is_empty() || source_config.sign.is_empty() {
                let source_meta = Self::get_remote_source_meta(&source_config.url).await?;
                if source_meta.chunk_id != source_config.chunk_id {
                    source_config.chunk_id = source_meta.chunk_id;
                    source_config.sign = source_meta.sign;
                    need_update_config_list.push(source_config.clone());
                }
            }
            let source_db_file = Self::source_db_file(&source_config, &source_db_dir);
            if source_db_file.exists() {
                //也许以前下载过?
                let source_node =
                    SourceNode::new(source_config, source_db_file.clone(), false).await?;
                new_source_list.push(source_node);
                continue;
            } else {
                Self::make_sure_source_file_exists(
                    &source_config.url,
                    &source_config.author,
                    &source_config.chunk_id,
                    &source_config.sign,
                    &source_db_file,
                )
                .await?;
                let source_node = SourceNode::new(source_config, source_db_file, false).await?;
                new_source_list.push(source_node);
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
            //TODO:删除旧的db文件
        }

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
        let deps: HashMap<String, String> = serde_json::from_value(deps.clone()).map_err(|e| {
            error!("dependencies from_value failed: {:?}", e);
            RepoError::ParamError(format!(
                "dependencies from_value failed, deps:{:?} err:{:?}",
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

    pub async fn install_pkg(&self, package_id: PackageId) -> RepoResult<String> {
        let task_id = REPO_TASK_MANAGER.start_install_task(package_id.clone())?;
        let task_id_tmp = task_id.clone();
        let self_clone = self.clone();
        task::spawn(async move {
            if let Err(e) = self_clone.do_install(package_id, &task_id_tmp).await {
                error!("do_install failed: {:?}", e);
                REPO_TASK_MANAGER
                    .set_task_status(&task_id_tmp, TaskStatus::Error, &e.to_string())
                    .unwrap();
            }
        });
        Ok(task_id)
    }

    pub async fn do_install(&self, package_id: PackageId, task_id: &str) -> RepoResult<()> {
        REPO_TASK_MANAGER.set_task_status(
            task_id,
            TaskStatus::Running,
            "Resolving dependencies",
        )?;
        let version_desc = if let Some(version) = &package_id.version {
            version.clone()
        } else {
            if let Some(sha256) = &package_id.sha256 {
                format!("sha256:{}", sha256)
            } else {
                "*".to_string()
            }
        };
        let mut dependencies = vec![];
        self.resolve_dependencies(&package_id.name, &version_desc, 0, &mut dependencies)
            .await?;
        REPO_TASK_MANAGER.set_task_deps(task_id, dependencies.clone())?;
        for dep in dependencies {
            let dep_id = format!("{}#{}", dep.name, dep.version);
            REPO_TASK_MANAGER.set_task_status(
                task_id,
                TaskStatus::Running,
                &format!("Downloading {}", dep_id),
            )?;
            self.pull_pkg(&dep).await?;
        }
        REPO_TASK_MANAGER.set_task_status(task_id, TaskStatus::Finished, "Finished")?;
        Ok(())
    }

    pub async fn pull_pkg(&self, meta_info: &PackageMeta) -> RepoResult<()> {
        if self.check_exist(meta_info).await? {
            return Ok(());
        }
        if let Err(e) =
            Verifier::verify(&meta_info.author, &meta_info.chunk_id, &meta_info.sign).await
        {
            return Err(RepoError::VerifyError(format!(
                "verify failed, meta:{:?}, err:{}",
                meta_info, e
            )));
        }
        unimplemented!("pull from other zone")
    }

    pub async fn check_exist(&self, meta_info: &PackageMeta) -> RepoResult<bool> {
        //TODO: 通过chunk manager查询chunk是否存在
        unimplemented!("check_exist")
    }
}
