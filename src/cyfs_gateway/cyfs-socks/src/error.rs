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


#[derive(Error, Debug)]
pub enum SocksError {
    #[error("Invalid config: {0}")]
    InvalidConfig(String),

    #[error("Invalid address: {0}")]
    InvalidAddress(String),

    #[error("Invalid auth: {0}")]
    InvalidAuth(String),

    #[error("Invalid param: {0}")]
    InvalidParam(String),

    #[error("IO error: {0}")]
    IoError(String),

    #[error("Http request error: {0}")]
    HttpError(String),

    #[error("Not support: {0}")]
    NotSupport(String),

    #[error("Socks error: {0}")]
    SocksError(String),
}

pub type SocksResult<T> = std::result::Result<T, SocksError>;