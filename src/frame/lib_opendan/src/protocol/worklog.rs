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
    /// `msg | event | control`.
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
    /// Why the call was made, as it stated (`intent` argument).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
}

fn unknown() -> String {
    "unknown".to_string()
}

/// What a tool result's envelope (`Observation.host`) says besides the text
/// the model was shown (prompt mechanics TODO A3): how a rebuilt history
/// shows it shorter (`title`, `summary`) and what it showed in full
/// (`dedup_key`, `hint_once_keys`). `dedup_of`: the text is the dedup hint
/// naming that earlier call.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ResultMeta {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedup_key: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hint_once_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedup_of: Option<String>,
}

const RESULT_META_TITLE_CHARS: usize = 300;
const RESULT_META_SUMMARY_CHARS: usize = 2000;

impl ResultMeta {
    /// The meta of an envelope (`None`: the observation carries none).
    pub fn of_host(host: &Option<Value>) -> Option<Self> {
        let v = agent_tool::ToolResultView::from_host(host)?;
        let clip = |s: &str, n: usize| s.trim().chars().take(n).collect::<String>();
        Some(Self {
            title: clip(&v.title, RESULT_META_TITLE_CHARS),
            summary: clip(&v.summary, RESULT_META_SUMMARY_CHARS),
            dedup_key: v.dedup.as_ref().map(|d| d.key.clone()).filter(|k| !k.is_empty()),
            hint_once_keys: v.once_keys(),
            dedup_of: v
                .detail
                .as_ref()
                .and_then(|d| d.get("dedup_of"))
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }

    /// What a context keeps of a result it shows in full: the keys only.
    pub fn keys(&self) -> Self {
        Self {
            dedup_key: self.dedup_key.clone(),
            hint_once_keys: self.hint_once_keys.clone(),
            ..Default::default()
        }
    }

    pub fn has_keys(&self) -> bool {
        self.dedup_key.is_some() || !self.hint_once_keys.is_empty()
    }
}

/// Source id of inputs the runner synthesizes itself (completion of a
/// watched task): they have no delivery position to consume.
pub const INTERNAL_TASK_SRC: &str = "_task";
/// Source id of the events a runner synthesizes from the registry state of
/// the session's sub sessions (needs attention / finished).
pub const INTERNAL_CHILD_SRC: &str = "_child";
/// Source id of the ticks a runner produces for `timer:` subscriptions
/// while the session waits for the work it started.
pub const INTERNAL_TIMER_SRC: &str = "_timer";
/// Source id of the correction a runner gives a Turn that neither ended
/// the session nor waits for anything (`system:runner`, `nudge`).
pub const INTERNAL_NUDGE_SRC: &str = "_nudge";

/// Worklog entry bodies (format of `opendan.session_state/6` sessions).
///
/// Identities: an input batch is `(run_id, input_seq)`, a Turn is `turn`
/// (session-wide), a behavior Step is `(run_id, step_index)`, a tool call /
/// action is its `call_id`. Entries of one run carry the Turn they belong
/// to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum WorklogBody {
    ReportDelivery {
        run_id: String,
        turn: u64,
        submission: super::ReportSubmission,
        assistant: String,
    },
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
        events: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hook: Option<String>,
        /// The batch's Memory material could not be read: why.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        memory_error: Option<String>,
        at_ms: u64,
    },
    /// An input batch that joined the open Turn (hand-over, sub context
    /// return, input consumed while the Turn was in progress).
    InputBatch {
        run_id: String,
        turn: u64,
        input_seq: u64,
        #[serde(default)]
        inputs: Vec<InputRef>,
        #[serde(default)]
        events: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hook: Option<String>,
        /// The batch's Memory material could not be read: why.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        memory_error: Option<String>,
        at_ms: u64,
    },
    UserMessage {
        run_id: String,
        turn: u64,
        content: String,
        /// One line standing for it in rendered history (a Goal header).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        brief: Option<String>,
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
        /// `ok | error | unresolved | cancelled`.
        status: String,
        /// The text the model was shown.
        #[serde(default)]
        result: String,
        /// What the result's envelope says besides that text.
        #[serde(flatten)]
        meta: ResultMeta,
    },
    /// How the session ended or set aside a run (written with the run's
    /// history). Outcomes that keep the run open without a decision
    /// (context limit pause, interrupt, retryable error) write nothing here.
    Outcome {
        run_id: String,
        turn: u64,
        /// Run ended: `done | process_done | budget | error | stopped`;
        /// `wait`: the run ended waiting for the user's next message
        /// (`WAIT_USER_MSG`), or it is kept while its tasks or sub sessions
        /// run and their ends are injected into it; `message`: it stopped for
        /// a message, which joins its open Turn; process suspended into
        /// `process_stack`: `suspended`;
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
    /// An input marked consumed without being processed. `reason` is a
    /// `RejectReason` name.
    InputRejected {
        input: InputRef,
        reason: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        detail: String,
    },
    /// An event consumed without entering the context or `pending_events`:
    /// no valid subscription matched (`unsubscribed`), or it only says a
    /// task the session follows changed (`watched_task`: its end is queried
    /// and comes in as a task result).
    EventDropped { input: InputRef, reason: String },
    /// Control inputs applied outside a run (stop / subscribe / activity).
    ControlApplied {
        input: InputRef,
        command: String,
        #[serde(default, skip_serializing_if = "Value::is_null")]
        detail: Value,
    },
}

impl WorklogBody {
    pub fn turn(&self) -> Option<u64> {
        match self {
            Self::TurnStarted { turn, .. }
            | Self::InputBatch { turn, .. }
            | Self::UserMessage { turn, .. }
            | Self::AssistantMessage { turn, .. }
            | Self::ReportDelivery { turn, .. }
            | Self::Step { turn, .. }
            | Self::ActionResult { turn, .. }
            | Self::Outcome { turn, .. }
            | Self::TurnEnded { turn, .. } => Some(*turn),
            _ => None,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            WorklogBody::ReportDelivery { .. } => "report_delivery",
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
            WorklogBody::EventDropped { .. } => "event_dropped",
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
