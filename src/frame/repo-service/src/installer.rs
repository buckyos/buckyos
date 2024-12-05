use crate::def::*;
use crate::error::*;
use crate::source_manager::SourceManager;
use crate::verifier::Verifier;
use buckyos_kit::buckyos_get_unix_timestamp;
use log::*;
use package_lib::PackageId;
use package_lib::Parser;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::task;

#[derive(Debug, Clone, PartialEq)]
pub enum InstallStatus {
    Pending,
    Running,
    Finished,
    Error,
}

#[derive(Debug, Clone)]
pub struct InstallTask {
    pub id: String,
    pub package_id: Option<PackageId>,
    pub status: InstallStatus,
    pub status_msg: Option<String>,
    pub error: Option<String>,
    pub deps: Vec<PackageMeta>,
    pub start_time: u64,  //任务开始时间,用来计算超时
    pub finish_time: u64, //任务完成时间,0表示未完成,定期会清理已完成的任务
}

#[derive(Debug, Clone)]
pub struct Installer {
    pub source_mgr: SourceManager,
    pub tasks: Arc<Mutex<HashMap<String, InstallTask>>>,
}

impl Installer {
    pub async fn new() -> RepoResult<Self> {
        Ok(Self {
            source_mgr: SourceManager::new().await?,
            tasks: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub async fn start_install_task(&self, package_desc: &str) -> RepoResult<String> {
        let now_time = buckyos_get_unix_timestamp();
        let task_id = format!("{}-install-{}", package_desc, now_time);
        let package_id = Parser::parse(package_desc).map_err(|e| {
            error!("parse package desc failed: {:?}", e);
            RepoError::ParseError(package_desc.to_string(), e.to_string())
        })?;
        let task = InstallTask {
            id: task_id.clone(),
            package_id: Some(package_id.clone()),
            status: InstallStatus::Pending,
            status_msg: None,
            error: None,
            deps: vec![],
            start_time: now_time,
            finish_time: 0,
        };
        self.tasks.lock().unwrap().insert(task_id.clone(), task);

        //启动一个异步任务来安装包的所有依赖
        let task_id_tmp = task_id.clone();
        let installer = self.clone();
        task::spawn(async move {
            if let Err(e) = installer.do_install(package_id, &task_id_tmp).await {
                error!("do_install failed: {:?}", e);
                installer
                    .set_task_status(&task_id_tmp, InstallStatus::Error, &e.to_string())
                    .unwrap();
            }
        });
        Ok(task_id)
    }

    pub async fn do_install(&self, package_id: PackageId, task_id: &str) -> RepoResult<()> {
        self.set_task_status(task_id, InstallStatus::Running, "Resolving dependencies")?;
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
        self.source_mgr
            .resolve_dependencies(&package_id.name, &version_desc, 0, &mut dependencies)
            .await?;
        self.set_task_deps(task_id, dependencies.clone());
        for dep in dependencies {
            let dep_id = format!("{}#{}", dep.name, dep.version);
            self.set_task_status(
                task_id,
                InstallStatus::Running,
                &format!("Downloading {}", dep_id),
            )?;
            self.pull_pkg(&dep).await?;
        }
        self.set_task_status(task_id, InstallStatus::Finished, "Finished")?;
        Ok(())
    }

    fn set_task_status(
        &self,
        task_id: &str,
        status: InstallStatus,
        status_msg: &str,
    ) -> RepoResult<()> {
        let mut tasks = self.tasks.lock().unwrap();
        if let Some(task) = tasks.get_mut(task_id) {
            if status == InstallStatus::Finished || status == InstallStatus::Error {
                task.finish_time = buckyos_get_unix_timestamp();
            }
            task.status = status;
            task.status_msg = Some(status_msg.to_string());
            Ok(())
        } else {
            Err(RepoError::NotFound(format!("task {} not found", task_id)))
        }
    }

    fn set_task_deps(&self, task_id: &str, deps: Vec<PackageMeta>) {
        let mut tasks = self.tasks.lock().unwrap();
        if let Some(task) = tasks.get_mut(task_id) {
            task.deps = deps;
        }
    }

    pub async fn get_task(&self, task_id: String) -> RepoResult<Option<InstallTask>> {
        let tasks = self.tasks.lock().unwrap();
        Ok(tasks.get(&task_id).cloned())
    }

    pub async fn get_all_tasks(&self) -> RepoResult<Vec<InstallTask>> {
        let tasks = self.tasks.lock().unwrap();
        Ok(tasks.values().cloned().collect())
    }

    pub async fn remove_task(&self, task_id: String) -> RepoResult<()> {
        let mut tasks = self.tasks.lock().unwrap();
        tasks.remove(&task_id);
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
