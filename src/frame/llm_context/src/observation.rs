//! Tool observation types (effect-side product paired with `AiToolCall`).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Normalised result of a single tool invocation. `ToolManager` implementations
/// translate whatever native shape they use into one of these variants.
///
/// Every observation is terminal: a tool whose work is not done within the
/// runtime's wait budget answers with a `Success` saying it is still running
/// as a task; the waist never waits for anything. `host` carries the
/// runtime's result envelope verbatim (e.g. an AgentToolResult): the waist
/// never reads it, the runtime's renderer does.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Observation {
    Success {
        call_id: String,
        content: Value,
        bytes: usize,
        #[serde(default)]
        truncated: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        host: Option<Value>,
    },
    Error {
        call_id: String,
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        host: Option<Value>,
    },
    /// The call was cancelled before it ran to completion: by an interrupt,
    /// a graceful finish or the run deadline (inline, see
    /// `ToolManager::call_tool`). Distinct from `Error`: not a failure the
    /// LLM should correct. `effect_unknown = false`: the work was stopped
    /// (or never started) and the text says what state it is in;
    /// `effect_unknown = true`: the work could not be cancelled and was
    /// abandoned while running, so its effect cannot be confirmed.
    Cancelled {
        call_id: String,
        reason: String,
        #[serde(default)]
        effect_unknown: bool,
    },
    /// No result for this call. `effect_unknown = true`: the call may have
    /// run but its side effects cannot be confirmed — the infrastructure
    /// failed while it was running, or (from a `ToolManager`, on resume) an
    /// earlier dispatch of the same `call_id` left no record and the tool is
    /// not safe to run again. `effect_unknown = false`: the call never
    /// started (the waist records it when a batch / step is cut short so the
    /// transcript stays paired and auditable). Not an LLM-correctable error.
    Unresolved {
        call_id: String,
        reason: String,
        effect_unknown: bool,
    },
}

impl Observation {
    pub fn call_id(&self) -> &str {
        match self {
            Observation::Success { call_id, .. } => call_id,
            Observation::Error { call_id, .. } => call_id,
            Observation::Cancelled { call_id, .. } => call_id,
            Observation::Unresolved { call_id, .. } => call_id,
        }
    }
}

/// Final state of one tool call attempt as seen by the waist.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolExecStatus {
    /// Ran and returned `Observation::Success`.
    Succeeded,
    /// Ran and returned `Observation::Error` (business failure).
    Failed,
    /// Dispatch infrastructure failed while the call may have been running;
    /// side effects cannot be confirmed.
    Unknown,
    /// The batch / step was aborted before this call was dispatched.
    NotExecuted,
    /// Cancelled: inline after an interrupt / finish / deadline.
    Cancelled,
}

/// Audit record for one tool call attempt. Lives in `ContextRunTrace.tool_trace`
/// and is carried by both `Done` and `Error` outcomes so an aborted batch can
/// still be audited.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolExecRecord {
    pub tool_name: String,
    pub call_id: String,
    pub status: ToolExecStatus,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ToolExecRecord {
    pub fn ok(&self) -> bool {
        self.status == ToolExecStatus::Succeeded
    }
}
