//! Bridges between other services and the Session Input Bus. A bridge only
//! converts sources and delivers reliably (post first, acknowledge the
//! upstream afterwards; re-posts are deduplicated by `key`): it never
//! renders, never decides whether an event is an Input or an Observe, and
//! never writes session state — the driver does, under the lease.

pub mod msg;
pub mod task;

pub use msg::{
    outbound_base, outbound_key, route_msg_record, MsgBridgeCtx, MsgBridgeOutput, MsgRecord,
    OutboundRecord, SlashCommand,
};
pub use task::{dispatch_idempotency_key, task_event, task_event_key};
