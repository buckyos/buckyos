#![allow(unused, dead_code)]

mod chunk;
mod local_store;
mod chunk_mgr;
mod ndn_client;

pub use chunk::*;
pub use local_store::*;
pub use chunk_mgr::*;


use thiserror::Error;

#[derive(Error, Debug)]
pub enum ChunkError {
    #[error("internal error: {0}")]
    Internal(String),
    #[error("invalid chunk id format: {0}")]
    InvalidId(String),
    #[error("chunk not found: {0}")]
    ChunkNotFound(String),
    #[error("chunk already exists: {0}")]
    ChunkExists(String),
    #[error("verify chunk error: {0}")]
    VerifyError(String),
    #[error("I/O error: {0}")]
    IoError(String),
    #[error("db error: {0}")]
    DbError(String),
    #[error("chunk not completed: {0}")]
    InComplete(String),
}


pub type ChunkResult<T> = std::result::Result<T, ChunkError>;

// mod http;
// pub use http::*;

