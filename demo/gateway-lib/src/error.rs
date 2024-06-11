use std::str::FromStr;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum GatewayError {
    #[error("Unknown: {0}")]
    Unknown(String),

    #[error("Io: {0}")]
    Io(#[from] std::io::Error),

    #[error("Socks: {0}")]
    Socks(#[from] fast_socks5::SocksError),

    #[error("InvalidConfig: {0}")]
    InvalidConfig(String),

    #[error("InvalidParam: {0}")]
    InvalidParam(String),

    #[error("InvalidFormat: {0}")]
    InvalidFormat(String),

    #[error("NotSupported: {0}")]
    NotSupported(String),

    #[error("UpstreamNotFound: {0}")]
    UpstreamNotFound(String),

    #[error("PeerNotFound: {0}")]
    PeerNotFound(String),

    #[error("TunnelError: {0}")]
    TunnelError(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("InvalidState: {0}")]
    InvalidState(String),

    #[error("AlreadyExists: {0}")]
    AlreadyExists(String),

    #[error("NotFound: {0}")]
    NotFound(String),

    #[error("HttpError: {0}")]
    HttpError(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
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

// Trim the prefix "InvalidConfig: " then got the error message
fn extract_error_msg(s: &str) -> String {
    let pos = s.find(": ").unwrap_or(0);
    s[pos + 2..].to_owned()
}

impl FromStr for GatewayError {
    type Err = GatewayError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Io" => Ok(GatewayError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                extract_error_msg(s),
            ))),
            "Socks" => Ok(GatewayError::Socks(fast_socks5::SocksError::Other(
                anyhow::anyhow!(extract_error_msg(s)),
            ))),

            "InvalidConfig" => Ok(GatewayError::InvalidConfig(extract_error_msg(s))),
            "InvalidParam" => Ok(GatewayError::InvalidParam(extract_error_msg(s))),
            "InvalidFormat" => Ok(GatewayError::InvalidFormat(extract_error_msg(s))),
            "NotSupported" => Ok(GatewayError::NotSupported(extract_error_msg(s))),
            "UpstreamNotFound" => Ok(GatewayError::UpstreamNotFound(extract_error_msg(s))),
            "PeerNotFound" => Ok(GatewayError::PeerNotFound(extract_error_msg(s))),
            "TunnelError" => Ok(GatewayError::TunnelError(extract_error_msg(s))),
            "Timeout" => Ok(GatewayError::Timeout(extract_error_msg(s))),
            "InvalidState" => Ok(GatewayError::InvalidState(extract_error_msg(s))),
            "AlreadyExists" => Ok(GatewayError::AlreadyExists(extract_error_msg(s))),
            "NotFound" => Ok(GatewayError::NotFound(extract_error_msg(s))),
            "HttpError" => Ok(GatewayError::HttpError(extract_error_msg(s))),
            _ => Ok(GatewayError::Unknown(s.to_owned())),
        }
    }
}
