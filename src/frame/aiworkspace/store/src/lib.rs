//! aiworkspace-store: SQLite persistence, NamedObject/asset storage, packages.

pub mod acl;
pub mod docdb;
pub mod entries;
pub mod export;
pub mod ext;
pub mod home;
pub mod library;
pub mod objects;
pub mod pack;
pub mod presence;
pub mod proc;
pub mod reads;
pub mod schema;
pub mod service;
pub mod session;
pub mod show;
pub mod source;
pub mod sql;
pub mod template;
pub mod urlsource;
pub mod users;
pub mod wish;
pub mod worklog;
pub mod workspace;

pub use service::{Service, WsHandle};
pub use workspace::{Caller, CallerClass, CommitOpts, FailAt, FailPoint, Workspace};
