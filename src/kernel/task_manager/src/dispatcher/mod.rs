//! Task Dispatch Center 2.0 — the TaskMgr subsystem's internal durable
//! delivery module (own store, own RPC path, own authorization).
//! Design: `doc/task_mgr/task-mgr 2.0.md` §11.

mod dispatch_db;
mod service;

#[allow(unused_imports)]
pub use dispatch_db::{DispatchDb, RecordStateUpdate};
#[allow(unused_imports)]
pub use service::{
    start_task_dispatcher, KrpcRunnerCaller, RunnerCaller, TaskDispatcherHttpServer,
    TaskDispatcherService,
};

#[cfg(test)]
mod tests;
