//! Mutable runtime state of an LLMContext.
//!
//! `LLMContextState` corresponds to "registers + stack" in the process
//! analogy. `LLMContextSnapshot` is the serialisable freeze produced when
//! the context yields (suspended outcomes) — it must be self-contained per
//! §6.2 of the design doc.

use buckyos_api::{AiMessage, AiResponse, AiToolCall, AiUsage};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::behavior_loop::{HistoryInputRecord, HistorySummaryRecord, StepRecord};
use crate::error::LLMComputeError;
use crate::observation::PendingToolCall;
use crate::outcome::ContextLimitKind;
use crate::request::LLMContextRequest;

/// Runtime mutable half of one LLMContext.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMContextState {
    /// Full message list currently visible to the LLM. Starts as a clone of
    /// `request.input`; the loop appends assistant replies and tool messages
    /// after each inference.
    pub accumulated: Vec<AiMessage>,

    /// Aggregate usage across every inference performed so far in this run.
    pub usage: AiUsage,

    /// Tool iterations remaining (see `ToolPolicy.max_tool_iterations`).
    /// Charged once per completed native batch and once per behavior step
    /// with actions, never again on resume.
    pub tool_iterations_left: u32,

    /// Wallclock at which `run()` first started, in ms since epoch.
    pub started_at_ms: u64,

    /// Cumulative cost in scheduler-defined units. We only track the counter
    /// — the meaning is up to the owner.
    pub cost_units: u32,

    /// Consecutive Recoverable errors fed back as observations. Reset on a
    /// successful inference.
    pub consecutive_errors: u32,

    /// Set while the context is suspended by a cooperative yield. `resume`
    /// only accepts the matching fill, `run` refuses to advance until then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suspended: Option<Suspension>,

    /// Provider tool batch in progress (function call loop, or the inner
    /// loop of the current behavior step). `accumulated` already holds the
    /// assistant message and the results of the calls that ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_batch: Option<ToolBatch>,

    /// Behavior loop: the step whose actions are being dispatched. Not yet
    /// sedimented; `step.action_results` holds the results so far.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_step: Option<ActionStep>,

    /// IDs of provider tasks issued by this run. Captured for trace output.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub llm_task_ids: Vec<String>,

    /// Behavior mode: sedimented step history. During one LLMContext run this
    /// history is append-only; any compression or rewrite happens above the
    /// waist before a new stable snapshot is resumed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<StepRecord>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history_summaries: Vec<HistorySummaryRecord>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history_inputs: Vec<HistoryInputRecord>,

    /// Behavior mode: the freshest step still being processed — rendered
    /// verbatim into the next inference. `None` until the first iteration
    /// finishes parsing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_step: Option<StepRecord>,

    /// Behavior mode: latest Self Report (`<report>`) emitted
    /// by the LLM in this run. Overwritten by every new Self Report; carried
    /// in the snapshot so a forked LLMContext that runs to terminal can have
    /// its produced result read out by the parent — see
    /// `doc/opendan/Agent Actions.md` §3.3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_report: Option<String>,

    #[serde(default)]
    pub next_step_index: u32,

    #[serde(default)]
    pub next_action_id: u32,

    /// Snapshot format version. `resume` refuses any version other than
    /// [`SNAPSHOT_FORMAT_VERSION`].
    #[serde(default)]
    pub snapshot_version: u32,

    /// Opaque host metadata carried verbatim through every snapshot, resume
    /// and continuation by another executor (e.g. libOpenDAN input receipts,
    /// keyed by host name). The waist never interprets it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<Value>,
}

/// Current snapshot format version. 2: suspension state (`suspended`,
/// `tool_batch`, `action_step`) replaced `pending_tool_calls`. 3: tool
/// budget renamed (`tool_iterations_left`, `ToolBatch.batch_error`).
/// `resume` accepts only this version.
pub const SNAPSHOT_FORMAT_VERSION: u32 = 3;

/// Why a context is suspended. Every variant records when it yielded:
/// suspended time is not charged to `max_wallclock_ms`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Suspension {
    /// `Outcome::PendingTool`: waiting for `ResumeFill::ToolResults`.
    PendingTool {
        pending: Vec<PendingToolCall>,
        at_ms: u64,
    },
    /// `Outcome::ContextLimitReached`: waiting for a rewritten history.
    /// `estimated_tokens` is the local estimate of the request that was not
    /// sent (absent for a provider refusal).
    ContextLimit {
        which: ContextLimitKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        estimated_tokens: Option<u64>,
        at_ms: u64,
    },
}

impl Suspension {
    pub fn at_ms(&self) -> u64 {
        match self {
            Suspension::PendingTool { at_ms, .. } | Suspension::ContextLimit { at_ms, .. } => {
                *at_ms
            }
        }
    }
}

/// Continuation of a provider tool batch cut by a deferred call.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ToolBatch {
    /// Calls not dispatched yet, in the order the provider returned them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remaining: Vec<AiToolCall>,
    /// First LLM-correctable failure of the batch so far; counted once when
    /// the batch completes (one failed iteration).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_error: Option<LLMComputeError>,
}

/// Continuation of a behavior step cut by a deferred action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionStep {
    /// The step after policy gating; `action_results[i]` answers
    /// `actions[i]` for the actions dispatched so far.
    pub step: StepRecord,
    /// The response the step was parsed from.
    pub response: AiResponse,
}

impl LLMContextState {
    pub fn from_request(req: &LLMContextRequest, started_at_ms: u64) -> Self {
        Self {
            accumulated: req.input.clone(),
            usage: AiUsage::default(),
            tool_iterations_left: req.tool_policy.max_tool_iterations,
            started_at_ms,
            cost_units: 0,
            consecutive_errors: 0,
            suspended: None,
            tool_batch: None,
            action_step: None,
            llm_task_ids: Vec::new(),
            steps: Vec::new(),
            history_summaries: Vec::new(),
            history_inputs: Vec::new(),
            last_step: None,
            last_report: None,
            next_step_index: 0,
            next_action_id: 0,
            snapshot_version: SNAPSHOT_FORMAT_VERSION,
            host: None,
        }
    }

    /// Calls a `PendingTool` suspension waits for (empty otherwise).
    pub fn pending_calls(&self) -> &[PendingToolCall] {
        match &self.suspended {
            Some(Suspension::PendingTool { pending, .. }) => pending,
            _ => &[],
        }
    }

    /// True while a tool batch or a behavior step is only partly dispatched.
    pub fn has_continuation(&self) -> bool {
        self.tool_batch.is_some() || self.action_step.is_some()
    }
}

/// Self-contained, serialisable freeze of a paused LLMContext. Carries both
/// the immutable request and the mutable state, so any scheduler holding
/// equivalent deps can resume it on another node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMContextSnapshot {
    pub request: LLMContextRequest,
    pub state: LLMContextState,
}
