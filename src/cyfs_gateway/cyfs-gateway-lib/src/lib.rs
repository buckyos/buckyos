mod config;
mod tunnel;
mod tunnel_mgr;
mod log_util;

pub use config::*;
pub use tunnel::*;
pub use tunnel_mgr::*;
pub use log_util::*;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum TunnelError {
    #[error("parse url {0} error : {1}")]
    UrlParseError(String,String),
    #[error("Unknow Protocl: {0}")]
    UnknowProtocol(String),
    #[error("Bind Error: {0}")]
    BindError(String),

}

pub type TunnelResult<T> = std::result::Result<T, TunnelError>;