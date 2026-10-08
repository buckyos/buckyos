//! aiworkspace-store: SQLite persistence, NamedObject/asset storage, packages.

pub mod docdb;
pub mod export;
pub mod objects;
pub mod proc;
pub mod reads;
pub mod schema;
pub mod service;
pub mod show;
pub mod urlsource;
pub mod wish;
pub mod workspace;

pub use service::{Service, WsHandle};
pub use workspace::{Caller, CommitOpts, FailAt, FailPoint, Workspace};
