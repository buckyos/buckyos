#![allow(dead_code)]

mod config;
mod tunnel;
mod tunnel_connector;
mod tunnel_mgr;
mod rtcp;
mod aes_stream;
mod ip;

pub use config::*;
pub use tunnel::*;
pub use tunnel_connector::*;
pub use tunnel_mgr::*;
pub use rtcp::*;
pub use aes_stream::*;

use thiserror::Error;
use once_cell::sync::OnceCell;

#[macro_use]
extern crate log;

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
    #[error("Invalid State: {0}")]
    InvalidState(String),
    #[error("Already Exists: {0}")]
    AlreadyExists(String),
    #[error("IO Error: {0}")]
    IoError(String),
}

pub type TunnelResult<T> = std::result::Result<T, TunnelError>;

// Only used in gateway service now
pub static CURRENT_DEVICE_RRIVATE_KEY: OnceCell<[u8;48]> = OnceCell::new();