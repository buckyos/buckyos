//! `worklog.jsonl` — strictly append-only session history (§4.4).
//!
//! One JSON object per line, `seq` strictly increasing. The file is never
//! rewritten; the only permitted modification is truncating the uncommitted
//! tail after `state.worklog.committed_bytes` during recovery.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::state::TurnStatus;

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

/// Worklog entry bodies (format of `opendan.session_state/2` sessions).
///
/// Identities: an input batch is `(run_id, input_seq)`, a Turn is `turn`
/// (session-wide), a behavior Step is `(run_id, step_index)`, a tool call /
/// action is its `call_id`. Entries of one run carry the Turn they belong
/// to.
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
    /// The input batch that opened a logical Turn.
    TurnStarted {
        run_id: String,
        turn: u64,
        input_seq: u64,
        #[serde(default)]
        inputs: Vec<InputRef>,
        #[serde(default)]
        changes: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hook: Option<String>,
        at_ms: u64,
    },
    /// An input batch that joined the open Turn (behavior hand-over, fork
    /// return, input consumed while the Turn was in progress). Observation
    /// injections are only recorded as their `user_message`.
    InputBatch {
        run_id: String,
        turn: u64,
        input_seq: u64,
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
        turn: u64,
        content: String,
    },
    /// Function call runs: one assistant response (one Round), with the
    /// native tool calls it requested. Not a behavior Step.
    AssistantMessage {
        run_id: String,
        turn: u64,
        #[serde(default)]
        assistant: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<ActionEntry>,
    },
    /// Behavior runs: one `StepRecord` (decision + actions; results follow
    /// as `action_result`).
    Step {
        run_id: String,
        turn: u64,
        step_index: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        behavior: Option<String>,
        #[serde(default)]
        assistant: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        actions: Vec<ActionEntry>,
        /// A synthetic step recording a rejected response (parse failure /
        /// policy rejection) fed back for self-correction, not a decision.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        correction: bool,
    },
    /// Result of a native tool call or a behavior action (`call_id`).
    ActionResult {
        run_id: String,
        turn: u64,
        call_id: String,
        /// `ok | error | unresolved | cancelled | pending`.
        status: String,
        #[serde(default)]
        result: String,
    },
    /// How the session ended or set aside a run (written with the run's
    /// history). Outcomes that keep the run open (pending tool, context
    /// limit pause, interrupt, retryable error) write nothing here.
    Outcome {
        run_id: String,
        turn: u64,
        /// Run ended: `done | wait | process_done | budget | error |
        /// stopped`; process suspended into `process_stack`: `suspended`;
        /// `context_rewritten`: the run hit the context limit, its history
        /// so far is above this entry and it continues from the compacted
        /// session history. Not a Turn end: see `turn_ended`.
        kind: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next_behavior: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        report: Option<String>,
    },
    /// The session closed a logical Turn.
    TurnEnded {
        #[serde(default)]
        run_id: String,
        turn: u64,
        status: TurnStatus,
        at_ms: u64,
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
            WorklogBody::TurnStarted { .. } => "turn_started",
            WorklogBody::InputBatch { .. } => "input_batch",
            WorklogBody::UserMessage { .. } => "user_message",
            WorklogBody::AssistantMessage { .. } => "assistant_message",
            WorklogBody::Step { .. } => "step",
            WorklogBody::ActionResult { .. } => "action_result",
            WorklogBody::Outcome { .. } => "outcome",
            WorklogBody::TurnEnded { .. } => "turn_ended",
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
