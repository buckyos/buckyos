//! nfs_server: NFSP v0 server over a local filesystem entity tree + filedb
//! virtual plane (see buckyos/product/bucky_file/nfs_server.md).
//!
//! Runs either as a buckyos system service (node_daemon launch, buckyos-api
//! login/heartbeat, cyfs:/// backed by $BUCKYOS_ROOT/data) or standalone via
//! CLI flags for independent testing. Per-request auth / NamedStore are not
//! integrated yet. See README.md for scope, honest degradations, and how to
//! run it.

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
