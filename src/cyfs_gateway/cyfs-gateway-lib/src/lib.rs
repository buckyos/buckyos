#![allow(dead_code)]

mod aes_stream;
mod config;
mod ip;
mod rtcp;
mod tunnel;
mod tunnel_connector;
mod tunnel_mgr;
mod socks;
mod selector;
mod acme_client;
mod cert_mgr;


pub use aes_stream::*;
pub use config::*;
pub use rtcp::*;
pub use tunnel::*;
pub use tunnel_connector::*;
pub use tunnel_mgr::*;
pub use socks::*;
pub use selector::*;
pub use cert_mgr::*;
pub use acme_client::*;

use once_cell::sync::OnceCell;
use thiserror::Error;

#[macro_use]
extern crate log;

#[derive(Error, Debug)]
pub enum TunnelError {
    #[error("parse url {0} error : {1}")]
    UrlParseError(String, String),
    #[error("Unknown Protocol: {0}")]
    UnknownProtocol(String),
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
pub static CURRENT_DEVICE_PRIVATE_KEY: OnceCell<[u8; 48]> = OnceCell::new();
