//! aiworkspace-core: pure logic of the AI Workspace kernel.
//!
//! No tokio, filesystem, SQLite, network, clock or randomness — callers inject
//! them — so the same code runs in the backend and (as WASM) in the browser replica.

pub mod access;
pub mod anchor;
pub mod canonical;
pub mod error;
pub mod filter;
pub mod id;
pub mod materialize;
pub mod model;
pub mod order_key;
pub mod plan;
pub mod plan_richtext;
pub mod plan_table;
pub mod read;
pub mod replica;
pub mod richtext;
pub mod testkit;
pub mod types;
pub mod value;

pub use error::{Code, WsError, WsResult};
