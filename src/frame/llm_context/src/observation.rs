//! Tool observation types (effect-side product paired with `AiToolCall`).

use buckyos_api::AiToolCall;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Protocol-level rendering view of an AgentToolResult.
///
/// This lives in `llm_context` instead of `agent_tool` so StepRecord renderers
/// can consume structured tool results without depending on the concrete tool
/// crate.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolResultStatusView {
    Success,
    Error,
    Pending,
}

impl Default for ToolResultStatusView {
    fn default() -> Self {
        Self::Success
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ToolResultView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_tool_protocol: Option<String>,
    #[serde(default)]
    pub status: ToolResultStatusView,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmd_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmd_args: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partial_output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check_after: Option<u64>,
}

impl ToolResultView {
    pub fn command_line_text(&self) -> Option<String> {
        self.cmd_name.as_ref().map(|cmd_name| {
            match self
                .cmd_args
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                Some(cmd_args) => format!("{cmd_name} {cmd_args}"),
                None => cmd_name.clone(),
            }
        })
    }
}

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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_result: Option<ToolResultView>,
    },
    Error {
        call_id: String,
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_result: Option<ToolResultView>,
    },
    /// Effect layer declared this call is async — its result will arrive via
    /// an external callback. waist then yields `Outcome::PendingTool`.
    Pending {
        call_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_result: Option<ToolResultView>,
    },
    /// The call was cancelled (typically by an upper-layer interrupt) before
    /// it ran to completion. Distinct from `Error` so renderers / the LLM
    /// can treat it as "not a failure" — the side effects, if any, are
    /// still external to this call's observation, but the *resolution* of
    /// the call is "user / session cancelled, please move on".
    Cancelled { call_id: String, reason: String },
}

impl Observation {
    pub fn call_id(&self) -> &str {
        match self {
            Observation::Success { call_id, .. } => call_id,
            Observation::Error { call_id, .. } => call_id,
            Observation::Pending { call_id, .. } => call_id,
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
