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
    /// The effect layer handed the work to a task (`task_id`, opaque to the
    /// waist). With `tool_policy.allow_deferred` the waist yields
    /// `Outcome::PendingTool` and the host waits for the task; `until_ms`
    /// (epoch) bounds that wait — past it the call is filled with the
    /// task's state at that moment. Without `allow_deferred` a tool manager
    /// must not return this (it waits inside the call instead).
    Pending {
        call_id: String,
        task_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        until_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_result: Option<ToolResultView>,
    },
    /// The call was cancelled before it ran to completion: by an interrupt,
    /// a graceful finish or the run deadline (inline, see
    /// `ToolManager::call_tool`), or by the host through
    /// `ResumeFill::ToolResults`. Distinct from `Error`: not a failure the
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
    /// The dispatcher produced no result for this call. `effect_unknown =
    /// true`: the infrastructure failed while the call may have been running,
    /// so its side effects cannot be confirmed. `effect_unknown = false`: the
    /// batch / step was aborted before this call started. Never produced by a
    /// `ToolManager`; the waist records it when a batch is cut short so the
    /// transcript stays paired and auditable. Not an LLM-correctable error.
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
            Observation::Pending { call_id, .. } => call_id,
            Observation::Cancelled { call_id, .. } => call_id,
            Observation::Unresolved { call_id, .. } => call_id,
        }
    }
}

/// One suspended call carried in `Outcome::PendingTool.pending` and in the
/// snapshot: what the host waits for (`task_id`) and until when
/// (`until_ms`, epoch; `None` = until the task ends).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PendingToolCall {
    /// The original call (name, args, call_id) exactly as dispatched.
    pub call: AiToolCall,
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until_ms: Option<u64>,
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
    /// Dispatched and deferred (`Observation::Pending`); the result is filled
    /// by the scheduler through `ResumeFill::ToolResults`.
    Pending,
    /// Cancelled: inline after an interrupt / finish / deadline, or by the
    /// scheduler through `ResumeFill::ToolResults`.
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
