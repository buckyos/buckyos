//! Task Dispatch Center — durable work handoff, co-deployed with the task
//! service but a fully independent abstraction (own store, own RPC path,
//! own authorization). Design: `doc/task_mgr/task_dispatch_center.md`.

mod dispatch_db;
mod service;

pub use dispatch_db::DispatchDb;
pub use service::{start_task_dispatcher, TaskDispatcherHttpServer, TaskDispatcherService};

#[cfg(test)]
mod tests;
