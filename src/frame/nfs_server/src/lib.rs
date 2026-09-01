//! nfs_server: NFSP v0 server over a local filesystem entity tree + filedb
//! virtual plane (see buckyos/product/bucky_file/nfs_server.md).
//!
//! v1 runs standalone (no buckyos-api / auth / NamedStore integration yet);
//! the focus is the protocol surface and filedb. See README.md for scope,
//! honest degradations, and how to run it.

pub mod config;
pub mod containers;
pub mod error;
pub mod filedb;
pub mod fsutil;
pub mod handle;
pub mod mutate;
pub mod namespace;
pub mod reconciler;
pub mod search;
pub mod server;
pub mod session;
pub mod state;
pub mod types;
pub mod uploads;
pub mod watch;

pub use config::{ExportRoot, ServerConfig};
pub use server::build_router;
pub use state::{AppState, SharedState};
