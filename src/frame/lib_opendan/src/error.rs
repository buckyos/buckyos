//! Error type shared by every libOpenDAN module.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Why a recovery was refused. The session keeps its live run, run directory,
/// committed input positions and execution evidence untouched (§8.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryBlocked {
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refs: Vec<String>,
}

impl std::fmt::Display for RecoveryBlocked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.run_id {
            Some(run) => write!(f, "recovery blocked (run {run}): {}", self.reason),
            None => write!(f, "recovery blocked: {}", self.reason),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OpenDanError {
    #[error("io error at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid json in {path}: {reason}")]
    Json { path: String, reason: String },
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("session id conflict: {0}")]
    SessionIdConflict(String),
    #[error("session {0} is not registered at this location")]
    Unregistered(String),
    #[error("principal {principal} is not the driver of session {session_id} (driver: {driver})")]
    NotDriver {
        session_id: String,
        principal: String,
        driver: String,
    },
    #[error("lock {resource} is held by another holder")]
    Busy {
        resource: String,
        holder: Option<Value>,
    },
    #[error("run {run_id} is busy (another executor holds its lock)")]
    RunBusy { run_id: String },
    #[error("lease lost for {0}")]
    LeaseLost(String),
    #[error("session {0} is finished")]
    SessionFinished(String),
    /// The session's bus holds the maximum of pending inputs; retry later.
    #[error("session {session_id} already holds {pending} pending inputs (input_full); retry later")]
    InputFull { session_id: String, pending: usize },
    /// A session of an older protocol version: read-only until migrated.
    #[error("session {session_id} is read-only: {reason}")]
    SessionReadonly { session_id: String, reason: String },
    #[error("runtime mismatch: session is bound to {bound}, runner provides {provided}")]
    RuntimeMismatch { bound: String, provided: String },
    #[error("bind error: {0}")]
    Bind(String),
    #[error("{0}")]
    RecoveryBlocked(RecoveryBlocked),
    #[error("input channel error: {0}")]
    Channel(String),
    #[error("llm context error: {0}")]
    Llm(String),
    #[error("xllm error: {0}")]
    Xllm(String),
    #[error("tool error: {0}")]
    Tool(String),
    #[error("artifact error: {0}")]
    Artifact(String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, OpenDanError>;

impl OpenDanError {
    pub fn io(path: impl AsRef<std::path::Path>, source: std::io::Error) -> Self {
        OpenDanError::Io {
            path: path.as_ref().display().to_string(),
            source,
        }
    }

    pub fn json(path: impl AsRef<std::path::Path>, reason: impl std::fmt::Display) -> Self {
        OpenDanError::Json {
            path: path.as_ref().display().to_string(),
            reason: reason.to_string(),
        }
    }

    pub fn blocked(reason: impl Into<String>, run_id: Option<&str>) -> Self {
        OpenDanError::RecoveryBlocked(RecoveryBlocked {
            reason: reason.into(),
            run_id: run_id.map(str::to_string),
            refs: Vec::new(),
        })
    }

    /// Machine readable error body for `state.last_error` / registry reports.
    pub fn to_json(&self) -> Value {
        let kind = match self {
            OpenDanError::Io { .. } => "io",
            OpenDanError::Json { .. } => "json",
            OpenDanError::InvalidArgument(_) => "invalid_argument",
            OpenDanError::NotFound(_) => "not_found",
            OpenDanError::SessionIdConflict(_) => "session_id_conflict",
            OpenDanError::Unregistered(_) => "unregistered",
            OpenDanError::NotDriver { .. } => "not_driver",
            OpenDanError::Busy { .. } => "busy",
            OpenDanError::RunBusy { .. } => "run_busy",
            OpenDanError::LeaseLost(_) => "lease_lost",
            OpenDanError::SessionFinished(_) => "session_finished",
            OpenDanError::InputFull { .. } => "input_full",
            OpenDanError::SessionReadonly { .. } => "session_readonly",
            OpenDanError::RuntimeMismatch { .. } => "runtime_mismatch",
            OpenDanError::Bind(_) => "bind_failed",
            OpenDanError::RecoveryBlocked(_) => "recovery_blocked",
            OpenDanError::Channel(_) => "channel",
            OpenDanError::Llm(_) => "llm",
            OpenDanError::Xllm(_) => "xllm",
            OpenDanError::Tool(_) => "tool",
            OpenDanError::Artifact(_) => "artifact",
            OpenDanError::Other(_) => "other",
        };
        let mut v = serde_json::json!({
            "kind": kind,
            "message": self.to_string(),
            "at_ms": crate::now_ms(),
        });
        if let OpenDanError::RecoveryBlocked(b) = self {
            v["recovery"] = serde_json::to_value(b).unwrap_or(Value::Null);
        }
        v
    }
}

impl From<agent_tool::xllm::XllmError> for OpenDanError {
    fn from(e: agent_tool::xllm::XllmError) -> Self {
        OpenDanError::Xllm(e.to_string())
    }
}

impl From<::kRPC::RPCErrors> for OpenDanError {
    fn from(e: ::kRPC::RPCErrors) -> Self {
        OpenDanError::Channel(e.to_string())
    }
}
