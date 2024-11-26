#![allow(unused, dead_code)]

use std::io;
use thiserror::Error;

mod index_store;
mod verifier;

pub use index_store::*;
pub use verifier::*;

use serde_json::Value;

#[derive(Clone, Debug)]
pub struct PackageMeta {
    pub name: String,
    pub version: String,
    pub author: String, //author did
    pub chunk_id: String,
    pub dependencies: Value,
    pub sign: String, //sign of the chunk_id
    pub pub_time: u64,
}

#[derive(Error, Debug)]
pub enum IndexError {
    #[error("Parse {0} error: {1}")]
    ParseError(String, String),
    #[error("Network Error: {0}")]
    NetworkError(String),
    #[error("Version Not Found: {0}")]
    VersionNotFoundError(String),
    #[error("Version Error: {0}")]
    VersionError(String),
    #[error("Update error: {0}")]
    UpdateError(String),
    #[error("Verify error: {0}")]
    VerifyError(String),
    #[error("File not found: {0}")]
    FileNotFoundError(String),
    #[error("Unknown Error: {0}")]
    UnknownError(String),
    #[error("IO Error: {0}")]
    IOError(#[from] io::Error),
    #[error("DB Error: {0}")]
    DbError(#[from] rusqlite::Error),
}

pub type IndexResult<T> = std::result::Result<T, IndexError>;
