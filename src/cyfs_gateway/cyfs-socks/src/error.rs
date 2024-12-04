use thiserror::Error;

#[derive(Error, Debug)]
pub enum RuleError {
    #[error("Invalid format: {0}")]
    InvalidFormat(String),

    #[error("Invalid script: {0}")]
    InvalidScript(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("IO error: {0}")]
    IoError(String),

    #[error("Http request error: {0}")]
    HttpError(String),

    #[error("Not support: {0}")]
    NotSupport(String),
}

pub type RuleResult<T> = std::result::Result<T, RuleError>;