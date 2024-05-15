use thiserror::Error;

#[derive(Error, Debug)]
pub enum GatewayError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Config format error: {0}")]
    InvalidConfig(String),

    #[error("Invalid data format: {0}")]
    InvalidFormat(String),
}

pub type GatewayResult<T> = Result<T, GatewayError>;