#![allow(unused, dead_code)]

mod chunk;
mod object;
mod local_store;
mod chunk_mgr;
mod ndn_client;

pub use chunk::*;
pub use local_store::*;
pub use chunk_mgr::*;
pub use object::*;


use thiserror::Error;

#[derive(Error, Debug)]
pub enum NdnError {
    #[error("internal error: {0}")]
    Internal(String),
    #[error("invalid chunk id format: {0}")]
    InvalidId(String),
    #[error("invalid object link: {0}")]
    InvalidLink(String),
    #[error("object not found: {0}")]
    NotFound(String),
    #[error("already exists: {0}")]
    AlreadyExists(String),
    #[error("verify chunk error: {0}")]
    VerifyError(String),
    #[error("I/O error: {0}")]
    IoError(String),
    #[error("db error: {0}")]
    DbError(String),
    #[error("chunk not completed: {0}")]
    InComplete(String),
    #[error("get from url failed: {0}")]
    GetFromRemoteError(String),
    #[error("decode error: {0}")]
    DecodeError(String),
}


pub type NdnResult<T> = std::result::Result<T, NdnError>;

// mod http;
// pub use http::*;

