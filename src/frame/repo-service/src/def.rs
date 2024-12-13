use package_lib::PackageId;
use serde_json::Value;
use sqlx::FromRow;

pub const SERVICE_NAME: &str = "repo_service";
pub const REPO_CHUNK_MGR_ID: &str = "repo_chunk_mgr";
pub const INDEX_DIR_NAME: &str = "index";
pub const LOCAL_INDEX_DB: &str = "local.db";
pub const REPO_SOURCE_CONFIG_DB: &str = "source_config.db";
pub const TASK_EXPIRE_TIME: u64 = 30 * 60; //任务超时时间,单位秒

#[derive(Clone, Debug, FromRow)]
pub struct PackageMeta {
    pub name: String,
    pub version: String,
    pub author: String, //author did
    pub chunk_id: String,
    pub dependencies: Value,
    pub sign: String, //sign of the chunk_id
    pub pub_time: i64,
}

pub struct SourceMeta {
    pub version: String,
    pub author: String,
    pub chunk_id: String,
    pub sign: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct SourceNodeConfig {
    pub id: i32,
    pub name: String,
    pub url: String,
    pub author: String,
    pub chunk_id: String,
    pub sign: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TaskStatus {
    Pending,
    Running(String), //Running status desc
    Finished,
    Error(String), //Error reason
}

#[derive(Debug, Clone)]
pub enum Task {
    InstallTask {
        id: String,
        package_id: PackageId,
        status: TaskStatus,
        deps: Vec<PackageMeta>,
        start_time: u64,  //任务开始时间,用来计算超时
        finish_time: u64, //任务完成时间,0表示未完成,定期会清理已完成的任务
    },
    IndexUpdateTask {
        id: String,
        status: TaskStatus,
        start_time: u64,
        finish_time: u64,
    },
}
