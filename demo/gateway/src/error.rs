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
}

pub type GatewayResult<T> = Result<T, GatewayError>;