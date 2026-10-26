//! nfs_server: NFSP v0 server over a local filesystem entity tree + filedb
//! virtual plane (see buckyos/product/bucky_file/nfs_server.md).
//!
//! Runs either as a buckyos system service (node_daemon launch, buckyos-api
//! login/heartbeat, cyfs:/// backed by $BUCKYOS_ROOT/data) or standalone via
//! CLI flags for independent testing. Every request is authenticated (auth.rs)
//! and authorized against path ACLs (acl.rs, access.rs). The share module
//! (share/) snapshots files into the named store and serves share links on
//! `share.<zone>`. The NDM gateway (ndm/) serves the Zone's upload and proxy
//! protocols over the node named store. See README.md for scope, honest degradations, and how to
//! run it.

pub mod access;
pub mod acl;
pub mod auth;
pub mod bucket;
pub mod config;
pub mod copy;
pub mod containers;
pub mod error;
pub mod filedb;
pub mod fsutil;
pub mod handle;
pub mod mutate;
pub mod namespace;
pub mod ndm;
pub mod reconciler;
pub mod search;
pub mod server;
pub mod session;
pub mod share;
pub mod state;
pub mod types;
pub mod uploads;
pub mod watch;

pub use config::{ExportRoot, ServerConfig};
pub use server::build_router;
pub use state::{AppState, SharedState};
