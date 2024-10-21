mod config;
mod tunnel;
mod tunnel_connector;
mod tunnel_mgr;
mod rtcp_tunnel;

pub use config::*;
pub use tunnel::*;
pub use tunnel_connector::*;
pub use tunnel_mgr::*;
pub use rtcp_tunnel::*;


use thiserror::Error;

#[derive(Error, Debug)]
pub enum TunnelError {
    #[error("parse url {0} error : {1}")]
    UrlParseError(String,String),
    #[error("Unknow Protocl: {0}")]
    UnknowProtocol(String),
    #[error("Bind Error: {0}")]
    BindError(String),
    #[error("Connect Error: {0}")]
    ConnectError(String),

}

pub type TunnelResult<T> = std::result::Result<T, TunnelError>;