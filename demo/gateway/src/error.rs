use thiserror::Error;

#[derive(Error, Debug)]
pub enum GatewayError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Socks error: {0}")]
    Socks(#[from] fast_socks5::SocksError),

    #[error("Config format error: {0}")]
    InvalidConfig(String),

    #[error("Invalid parameter: {0}")]
    InvalidParam(String),

    #[error("Invalid data format: {0}")]
    InvalidFormat(String),

    #[error("Not supported: {0}")]
    NotSupported(String),

    #[error("Upstream not found: {0}")]
    UpstreamNotFound(String),

    #[error("Peer not found: {0}")]
    PeerNotFound(String),

    #[error("Tunnel error: {0}")]
    TunnelError(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("Invalid state: {0}")]
    InvalidState(String),

    #[error("Already exists: {0}")]
    AlreadyExists(String),

    #[error("Not found: {0}")]
    NotFound(String),
}

pub type GatewayResult<T> = Result<T, GatewayError>;

pub fn error_to_status_code(e: &GatewayError) -> http::StatusCode {
    match e {
        GatewayError::InvalidConfig(_) => http::StatusCode::BAD_REQUEST,
        GatewayError::InvalidParam(_) => http::StatusCode::BAD_REQUEST,
        GatewayError::InvalidFormat(_) => http::StatusCode::BAD_REQUEST,
        GatewayError::NotSupported(_) => http::StatusCode::NOT_IMPLEMENTED,
        GatewayError::UpstreamNotFound(_) => http::StatusCode::NOT_FOUND,
        GatewayError::PeerNotFound(_) => http::StatusCode::NOT_FOUND,
        GatewayError::TunnelError(_) => http::StatusCode::INTERNAL_SERVER_ERROR,
        GatewayError::Timeout(_) => http::StatusCode::REQUEST_TIMEOUT,
        GatewayError::InvalidState(_) => http::StatusCode::INTERNAL_SERVER_ERROR,
        GatewayError::AlreadyExists(_) => http::StatusCode::CONFLICT,
        GatewayError::NotFound(_) => http::StatusCode::NOT_FOUND,
        _ => http::StatusCode::INTERNAL_SERVER_ERROR,
    }
}