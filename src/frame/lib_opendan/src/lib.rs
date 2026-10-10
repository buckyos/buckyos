//! libOpenDAN — advance a Session as an Agent, outside the OpenDAN process.
//!
//! See `doc/opendan/Agent Session SDK 实现计划.md`. The **protocol** is the
//! session directory layout, the commit order, the lock semantics, the input
//! message format and xllm's run directory; everything else here (Agent State
//! client, runtime, runner) is the Rust reference implementation.
//!
//! - [`session`]: the self-contained, location independent session directory
//!   (`.opendan_agent_session/`), created with [`api::create_session`].
//! - [`lock`]: long-held exclusive file locks (`lease.json`, run `.lock`).
//! - [`state`]: Agent State through [`state::AgentStateClient`] (registry,
//!   active session view, perception, cognition facade, artifact list).
//! - [`channel`]: kmsg inputs + kevent wake-ups.
//! - [`bridge`]: msg-center / task state → Session Input Bus records.
//! - [`runtime`]: execution environments for `exec` (native, tmux).
//! - [`runner`]: drives a session to its end condition.

pub mod api;
pub mod bridge;
pub mod channel;
pub mod error;
pub mod fault;
pub mod fsutil;
pub mod host;
pub mod ids;
pub mod lock;
pub mod memory;
pub mod protocol;
pub mod runner;
pub mod runtime;
pub mod session;
pub mod state;
pub mod template;

pub use api::{
    create_self_improve_session, create_session, post_input, read_session, InputChannel,
    SessionSpec, SessionView,
};
pub use error::{OpenDanError, RecoveryBlocked, Result};
pub use session::{Session, SessionDir};
pub use state::{AgentStateClient, FsAgentStateClient};
pub use template::SessionTemplate;

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
