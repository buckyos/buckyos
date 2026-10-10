//! Bridges between other services and the Session Input Bus. A bridge only
//! converts sources and delivers reliably (post first, acknowledge the
//! upstream afterwards; re-posts are deduplicated by `key`): it never
//! renders, never decides whether an event is an Input or an Observe, and
//! never writes session state — the driver does, under the lease.

pub mod msg;
pub mod task;

pub use msg::{
    context_msg_record, outbound_base, outbound_key, record_sender, route_msg_record,
    MsgBridgeCtx, MsgBridgeOutput, MsgRecord, OutboundRecord, SlashCommand,
};
pub use task::{dispatch_idempotency_key, task_event, task_event_key};

pub mod kevent;
pub mod timer;

use async_trait::async_trait;

use crate::error::Result;
use crate::protocol::SessionConfig;

/// A producer living in the host process: turns a system source (kevent,
/// timers …) into `AgentEvent`s posted to a session through the registry.
/// It only describes events; Input or Observe is decided by the receiving
/// session's subscriptions. A bridge runs from the moment its session is
/// served until the host stops it; it is restarted when `config_rev` moves.
#[async_trait]
pub trait EventBridge: Send + Sync {
    /// Whether the session has subscriptions this bridge produces for.
    fn wants(&self, cfg: &SessionConfig) -> bool;
    /// Produce until cancelled (or the session finished).
    async fn run(&self, cfg: SessionConfig) -> Result<()>;
}

pub use kevent::KEventBridge;
pub use timer::TimerBridge;

