#![allow(unused, dead_code)]

mod chunk;
mod chunk_list;
mod object;
mod link_obj;
mod local_store;
mod named_data_mgr;
mod cyfs_http;
mod ndn_client;
mod fileobj;
mod mtree;
mod hash;
mod object_map;
mod trie_object_map;
mod object_array;

pub use object::*;
pub use chunk::*;
pub use local_store::*;
pub use link_obj::*;
pub use named_data_mgr::*;
pub use cyfs_http::*;
pub use ndn_client::*;
pub use fileobj::*;
pub use hash::*;
pub use mtree::*;
pub use object_map::*;
pub use trie_object_map::*;
pub use object_array::*;
pub use chunk_list::*;

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
}


pub type NdnResult<T> = std::result::Result<T, NdnError>;


pub const OBJ_TYPE_FILE: &str = "cyfile";
pub const OBJ_TYPE_DIR: &str = "cydir";
pub const OBJ_TYPE_PATH: &str = "cypath";
pub const OBJ_TYPE_MTREE: &str = "cytree";
pub const OBJ_TYPE_OBJMAPT: &str = "cymap"; // object map
pub const OBJ_TYPE_PACK: &str = "cypack"; // object set
pub const OBJ_TYPE_LIST: &str = "cylist"; // object list

pub const OBJ_TYPE_PKG: &str = "pkg"; // package
// mod http;
// pub use http::*;

