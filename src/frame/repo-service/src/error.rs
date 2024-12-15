use std::io;
use thiserror::Error;

#[allow(dead_code)]
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
    #[error("File not found: {0}")]
    FileNotFoundError(String),
    #[error("Unknown Error: {0}")]
    UnknownError(String),
    #[error("IO Error: {0}")]
    IOError(#[from] io::Error),
    #[error("DB Error: {0}")]
    DbError(#[from] sqlx::Error),
    #[error("Json Error: {0}")]
    JsonError(#[from] serde_json::Error),
    #[error("Ndn Error: {0}")]
    NdnError(String),
    #[error("Permission Error: {0}")]
    PermissionError(String),
}

pub type RepoResult<T> = std::result::Result<T, RepoError>;
