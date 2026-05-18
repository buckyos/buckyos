//! Tool observation types (effect-side product paired with `AiToolCall`).

use buckyos_api::AiToolCall;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Normalised result of a single tool invocation. `ToolManager` implementations
/// translate whatever native shape they use into one of these variants.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Observation {
    Success {
        call_id: String,
        content: Value,
        bytes: usize,
        #[serde(default)]
        truncated: bool,
    },
    Error {
        call_id: String,
        message: String,
    },
    /// Effect layer declared this call is async — its result will arrive via
    /// an external callback. waist then yields `Outcome::PendingTool`.
    Pending {
        call_id: String,
    },
    /// The call was cancelled (typically by an upper-layer interrupt) before
    /// it ran to completion. Distinct from `Error` so renderers / the LLM
    /// can treat it as "not a failure" — the side effects, if any, are
    /// still external to this call's observation, but the *resolution* of
    /// the call is "user / session cancelled, please move on".
    Cancelled {
        call_id: String,
        reason: String,
    },
}

impl Observation {
    pub fn call_id(&self) -> &str {
        match self {
            Observation::Success { call_id, .. } => call_id,
            Observation::Error { call_id, .. } => call_id,
            Observation::Pending { call_id } => call_id,
            Observation::Cancelled { call_id, .. } => call_id,
        }
    }
}

/// One pending (deferred) tool entry carried in `Outcome::PendingTool.pending`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PendingToolCall {
    pub call: AiToolCall,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eta_ms: Option<u64>,
}

/// Audit record for one tool call attempt. Lives in `ContextRunTrace.tool_trace`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolExecRecord {
    pub tool_name: String,
    pub call_id: String,
    pub ok: bool,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
