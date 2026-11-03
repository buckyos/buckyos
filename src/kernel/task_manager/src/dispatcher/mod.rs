//! Task Dispatch Center 2.0 — the TaskMgr subsystem's internal durable
//! delivery module (own store, own RPC path, own authorization).
//! Design: `doc/task_mgr/task-mgr 2.0.md` §11.

mod dispatch_db;
mod service;

pub use service::{start_task_dispatcher, TaskDispatcherHttpServer};

#[cfg(test)]
mod tests;
