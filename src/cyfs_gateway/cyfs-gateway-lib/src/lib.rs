mod config;
mod tunnel;
mod tunnel_connector;
mod tunnel_mgr;
mod rtcp_tunnel;
mod aes_stream;


pub use config::*;
pub use tunnel::*;
pub use tunnel_connector::*;
pub use tunnel_mgr::*;
pub use rtcp_tunnel::*;
pub use aes_stream::*;

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
    #[error("DIDDocument Error: {0}")]
    DocumentError(String),
    #[error("Reason Error: {0}")]
    ReasonError(String),
}

pub type TunnelResult<T> = std::result::Result<T, TunnelError>;