use package_lib::PackageId;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::io;
use thiserror::Error;

pub const SERVICE_NAME: &str = "repo_service";
pub const REPO_CHUNK_MGR_ID: &str = "repo_chunk_mgr";
pub const REMOTE_INDEX_DIR_NAME: &str = "remote_index_source";
pub const LOCAL_INDEX_DATA: &str = "local_index_data";
pub const LOCAL_INDEX_DB: &str = "local_index.db";
pub const LOCAL_INDEX_META_DB: &str = "index_meta.db";
pub const REPO_CONFIG_FILE: &str = "repo_config.json";
pub const TASK_EXPIRE_TIME: u64 = 30 * 60; //任务超时时间,单位秒

#[derive(Clone, Debug, FromRow, Serialize, Deserialize)]
pub struct PackageMeta {
    pub pkg_name: String,
    pub version: String,
    pub category: String, //分类,app,pkg,agent等
    pub hostname: String,
    pub chunk_id: Option<String>, //有些pkg不需要下载
    pub dependencies: String,     //json string
    pub jwt: String,              //对meta整体的签名
    pub pub_time: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize, FromRow)]
pub struct SourceMeta {
    pub hostname: String,
    pub chunk_id: String,
    pub jwt: String,
    pub version: String,
    pub pub_time: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SourceNodeConfig {
    pub hostname: String,
    pub chunk_id: String,
    pub jwt: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TaskStatus {
    Pending,
    Running(String), //Running status desc
    Finished,
    Error(String), //Error reason
}

//impl to_string for TaskStatus
impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            TaskStatus::Pending => write!(f, "Pending"),
            TaskStatus::Running(desc) => write!(f, "Running:{}", desc),
            TaskStatus::Finished => write!(f, "Finished"),
            TaskStatus::Error(reason) => write!(f, "Error:{}", reason),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Error, Debug)]
pub enum RepoError {
    #[error("Download {0} error: {1}")]
    DownloadError(String, String),
    #[error("Install {0} error: {1}")]
    InstallError(String, String),
    #[error("Load {0} error: {1}")]
    LoadError(String, String),
    #[error("Not Found: {0}")]
    NotFound(String),
    #[error("Parse {0} error: {1}")]
    ParseError(String, String),
    #[error("Param error: {0}")]
    ParamError(String),
    #[error("Execute cmd {0} error: {1}")]
    ExecuteError(String, String),
    #[error("Config parser error: {0}")]
    ParserConfigError(String),
    #[error("Network Error: {0}")]
    NetworkError(String),
    #[error("Version Not Found: {0}")]
    VersionNotFoundError(String),
    #[error("Version Error: {0}")]
    VersionError(String),
    #[error("Not ready: {0}")]
    NotReadyError(String),
    #[error("Status Error: {0}")]
    StatusError(String),
    #[error("Update error: {0}")]
    UpdateError(String),
    #[error("Verify error: {0}")]
    VerifyError(String),
    #[error("Sign error: {0}")]
    SignError(String),
    #[error("File not found: {0}")]
    FileNotFoundError(String),
    #[error("Unknown Error: {0}")]
    UnknownError(String),
    #[error("IO Error: {0}")]
    IoError(#[from] io::Error),
    #[error("DB Error: {0}")]
    DbError(#[from] sqlx::Error),
    #[error("Json Error: {0}")]
    JsonError(#[from] serde_json::Error),
    #[error("Ndn Error: {0}")]
    NdnError(String),
    #[error("Permission Error: {0}")]
    PermissionError(String),
    #[error("Http Error: {0}")]
    HttpError(String),
    #[error("rpc Error: {0}")]
    RpcError(String),
}

pub type RepoResult<T> = std::result::Result<T, RepoError>;
