#![allow(unused, dead_code)]

mod chunk;
mod object;
mod link_obj;
mod named_data;
mod cyfs_http;
mod ndn_client;
mod fileobj;
mod mtree;
mod hash;
mod object_map;
mod trie_object_map;
mod object_array;
mod coll;

pub use object::*;
pub use chunk::*;
pub use link_obj::*;
pub use named_data::*;
pub use cyfs_http::*;
pub use ndn_client::*;
pub use fileobj::*;
pub use hash::*;
pub use mtree::*;
pub use object_map::*;
pub use trie_object_map::*;
pub use object_array::*;
pub use coll::*;

use reqwest::StatusCode;
use thiserror::Error;

#[macro_use]
extern crate log;

#[derive(Error, Debug)]
pub enum NdnError {
    #[error("internal error: {0}")]
    Internal(String),
    #[error("invalid object id format: {0}")]
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
    #[error("remote error: {0}")]
    RemoteError(String),
    #[error("decode error: {0}")]
    DecodeError(String),
    #[error("offset too large: {0}")]
    OffsetTooLarge(String),
    #[error("invalid obj type: {0}")]
    InvalidObjType(String),

    #[error("invalid data: {0}")]
    InvalidData(String),

    #[error("invalid param: {0}")]
    InvalidParam(String),

    #[error("invalid state: {0}")]
    InvalidState(String),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Unsupported operation: {0}")]
    Unsupported(String),
}

impl NdnError {
    pub fn from_http_status(code: StatusCode,info:String) -> Self {
        match code {
            StatusCode::NOT_FOUND => NdnError::NotFound(info),
            StatusCode::INTERNAL_SERVER_ERROR => NdnError::Internal(info),
            _ => NdnError::RemoteError(format!("HTTP error: {} for {}", code, info)),
        }
    }
}


pub type NdnResult<T> = std::result::Result<T, NdnError>;


pub const OBJ_TYPE_FILE: &str = "cyfile";
pub const OBJ_TYPE_DIR: &str = "cydir";
pub const OBJ_TYPE_PATH: &str = "cypath";
pub const OBJ_TYPE_MTREE: &str = "cytree";
pub const OBJ_TYPE_OBJMAP: &str = "cymap"; // object map
pub const OBJ_TYPE_TRIE: &str = "cytrie"; // trie object map
pub const OBJ_TYPE_PACK: &str = "cypack"; // object set
pub const OBJ_TYPE_LIST: &str = "cylist"; // object list

pub const OBJ_TYPE_CHUNK_LIST: &str = "cl"; // normal chunk list with variable size
pub const OBJ_TYPE_CHUNK_LIST_SIMPLE: &str = "cl-s"; // simple chunk list with variable size
pub const OBJ_TYPE_CHUNK_LIST_FIX_SIZE: &str = "cl-f"; // normal chunk list with fixed size
pub const OBJ_TYPE_CHUNK_LIST_SIMPLE_FIX_SIZE: &str = "cl-sf"; // simple chunk list with fixed size

pub const OBJ_TYPE_PKG: &str = "pkg"; // package
// mod http;
// pub use http::*;

