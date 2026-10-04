//! Every on-disk / on-queue structure of the Agent Session protocol.
//!
//! These types are the source for the reverse-written Spec and JSON Schemas
//! (L6, `doc/opendan/protocol/`). Within one schema version keep field names
//! stable and add fields only with `#[serde(default)]`; a rename or a change
//! of meaning bumps the schema version (beta 2.2: older versions are refused,
//! no aliases).

pub mod agent_state;
pub mod behavior;
pub mod config;
pub mod input;
pub mod misc;
pub mod runtime;
pub mod state;
pub mod summary;
pub mod worklog;

pub use agent_state::*;
pub use behavior::*;
pub use config::*;
pub use input::*;
pub use misc::*;
pub use runtime::*;
pub use state::*;
pub use summary::*;
pub use worklog::*;

/// Directory holding the session state inside a session directory.
pub const STATE_DIR: &str = ".opendan_agent_session";
pub const RUNTIME_DIR: &str = ".runtime";
pub const SESSION_CONFIG_FILE: &str = "session_config.json";
pub const STATE_FILE: &str = "state.json";
pub const SUMMARY_FILE: &str = "summary.json";
pub const WORKLOG_FILE: &str = "worklog.jsonl";
pub const STATIC_FILE: &str = "static.json";
/// Per-Round usage by model (append only, may be missing).
pub const USAGE_FILE: &str = "usage.jsonl";
pub const LEASE_FILE: &str = "lease.json";
/// Serializes "check pending inputs + append" of producers.
pub const POST_LOCK_FILE: &str = "post.lock";
pub const BINDING_FILE: &str = "binding.json";
pub const RUNS_DIR: &str = "runs";
pub const README_FILE: &str = "readme.md";
pub const REPORT_FILE: &str = "report.md";

/// Export the JSON Schemas of the protocol files (L6).
pub fn json_schemas() -> Vec<(&'static str, serde_json::Value)> {
    fn s<T: schemars::JsonSchema>() -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(T)).unwrap_or(serde_json::Value::Null)
    }
    vec![
        ("session_config", s::<config::SessionConfig>()),
        ("behavior_entry", s::<config::BehaviorEntry>()),
        ("behavior_config", s::<behavior::BehaviorConfig>()),
        ("session_state", s::<state::SessionState>()),
        ("session_summary", s::<summary::SessionSummary>()),
        ("worklog_entry", s::<worklog::WorklogEntry>()),
        ("lock_info", s::<misc::LockInfo>()),
        ("binding", s::<misc::Binding>()),
        ("session_static", s::<misc::SessionStatic>()),
        ("usage_record", s::<misc::UsageRecord>()),
        ("registry_entry", s::<agent_state::RegistryEntry>()),
        ("perception_record", s::<agent_state::PerceptionRecord>()),
        ("perception_cursor", s::<agent_state::PerceptionCursor>()),
        ("artifact_head", s::<agent_state::ArtifactHead>()),
        ("artifact_version", s::<agent_state::ArtifactVersion>()),
        ("session_input", s::<input::PostedInput>()),
        ("session_msg", s::<input::SessionMsg>()),
        ("agent_event", s::<input::AgentEvent>()),
        ("control_command", s::<input::ControlCommand>()),
        ("input_receipt", s::<input::InputReceipt>()),
        ("host_meta", s::<input::HostMeta>()),
        ("runtime_descriptor", s::<runtime::RuntimeDescriptor>()),
    ]
}
