#![allow(unused, dead_code)]
mod downloader;
mod env;
mod error;
mod index_store;
mod parser;
mod verifier;
mod version_util;

use serde_json::Value;

pub use env::*;
pub use error::*;
pub use index_store::*;
pub use parser::*;
pub use verifier::*;

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
