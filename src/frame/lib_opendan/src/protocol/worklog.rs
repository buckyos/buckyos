//! `worklog.jsonl` — strictly append-only session history (§4.4).
//!
//! One JSON object per line, `seq` strictly increasing. The file is never
//! rewritten; the only permitted modification is truncating the uncommitted
//! tail after `state.worklog.committed_bytes` during recovery.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Stable reference to one consumed input instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InputRef {
    pub src: String,
    pub index: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub key: String,
    /// `msg | event | change | control | perception`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
}

impl InputRef {
    pub fn id(&self) -> String {
        format!("{}#{}", self.src, self.index)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ActionEntry {
    pub call_id: String,
    pub tool: String,
    #[serde(default)]
    pub args: Value,
    /// `read_only | idempotent | side_effect | unknown`.
    #[serde(default = "unknown")]
    pub effect: String,
}

fn unknown() -> String {
    "unknown".to_string()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum WorklogBody {
    /// First entry of every session.
    Created {
        session_id: String,
        kind: String,
        by: String,
        #[serde(default)]
        objective: String,
        at_ms: u64,
    },
    RoundStarted {
        run_id: String,
        round: u64,
        #[serde(default)]
        inputs: Vec<InputRef>,
        #[serde(default)]
        changes: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hook: Option<String>,
        at_ms: u64,
    },
    UserMessage {
        run_id: String,
        round: u64,
        content: String,
    },
    Step {
        run_id: String,
        round: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        behavior: Option<String>,
        #[serde(default)]
        assistant: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        actions: Vec<ActionEntry>,
    },
    ActionResult {
        run_id: String,
        round: u64,
        call_id: String,
        /// `ok | error | unresolved | cancelled | pending`.
        status: String,
        #[serde(default)]
        result: String,
    },
    Outcome {
        run_id: String,
        round: u64,
        /// `done | wait | pending_tool | budget | error | interrupted |
        /// context_limit | stopped`.
        kind: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next_behavior: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        report: Option<String>,
    },
    /// A new summary.json was produced (audit only).
    Compaction {
        summary_start_seq: u64,
        #[serde(default)]
        made_by: String,
    },
    Decide {
        decision: String,
        by: String,
        #[serde(default, skip_serializing_if = "Value::is_null")]
        report: Value,
    },
    /// An input marked consumed without being processed.
    InputRejected {
        input: InputRef,
        reason: String,
    },
    /// A subscription change dropped by coalescing / trimming.
    ChangeDropped {
        change: String,
        reason: String,
    },
    /// Control inputs applied outside a run (stop / subscribe / activity).
    ControlApplied {
        input: InputRef,
        command: String,
        #[serde(default, skip_serializing_if = "Value::is_null")]
        detail: Value,
    },
}

impl WorklogBody {
    pub fn kind(&self) -> &'static str {
        match self {
            WorklogBody::Created { .. } => "created",
            WorklogBody::RoundStarted { .. } => "round_started",
            WorklogBody::UserMessage { .. } => "user_message",
            WorklogBody::Step { .. } => "step",
            WorklogBody::ActionResult { .. } => "action_result",
            WorklogBody::Outcome { .. } => "outcome",
            WorklogBody::Compaction { .. } => "compaction",
            WorklogBody::Decide { .. } => "decide",
            WorklogBody::InputRejected { .. } => "input_rejected",
            WorklogBody::ChangeDropped { .. } => "change_dropped",
            WorklogBody::ControlApplied { .. } => "control_applied",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WorklogEntry {
    pub seq: u64,
    #[serde(flatten)]
    pub body: WorklogBody,
}
