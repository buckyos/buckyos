use std::io;
use thiserror::Error;

#[allow(dead_code)]
#[derive(Error, Debug)]
pub enum PkgError {
    #[error("Download {0} error: {1}")]
    DownloadError(String, String),
    #[error("Install {0} error: {1}")]
    InstallError(String, String),
    #[error("Load {0} error: {1}")]
    LoadError(String, String),
    #[error("Parse {0} error: {1}")]
    ParseError(String, String),
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
    #[error("Json Error: {0}")]
    JsonError(#[from] serde_json::Error),
    #[error("Sql Error: {0}")]
    SqlError(String),
    #[error("Access denied: {0}")]
    AccessDeniedError(String),
    #[error("Lock Error: {0}")]
    LockError(String),
}

pub type PkgResult<T> = std::result::Result<T, PkgError>;
