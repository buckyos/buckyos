//! Outputs / outcomes of one LLMContext run.
//!
//! Outcomes split into two structural classes (see `LLM Context 设计.md` §3.10):
//! - **Terminal**: `Done` / `Error` / `BudgetExhausted` — object is consumed.
//! - **Suspended**: `PendingTool` / `ContextLimitReached` / `Interrupted` —
//!   a `LLMContextSnapshot` is produced and the run is resumable.
//!
//! "Waiting for the next human message" is **not** a waist concept — it is a
//! session-layer state. The behavior loop signals it via
//! `Done.behavior_result.next_behavior == "WAIT_USER_MSG"` (sentinel
//! interpreted by opendan/session, not by the waist); the waist only sees a
//! Done outcome and is done with it.

use buckyos_api::{AiMessage, AiResponse, AiUsage};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::behavior_loop::{HistorySummaryRecord, LLMBehaviorResult, StepRecord};
use crate::error::LLMComputeError;
use crate::interrupt::InferenceAbortTrace;
use crate::observation::{Observation, PendingToolCall, ToolExecRecord};
use crate::state::LLMContextSnapshot;

/// What the scheduler feeds back when resuming. The variant must match the
/// snapshot's `state.suspended`; any mismatch is rejected by
/// `LLMContext::resume` with `SnapshotCorrupted` before any inference or
/// tool call.
///
/// Serialised as a tagged enum (`{"kind": "...", ...}`) so snapshot + fill can
/// travel over JSON across processes (L4 OneShot crash recovery relies on this).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResumeFill {
    /// Paired with `PendingTool`. Every pending call is answered exactly once
    /// with a terminal observation (`Success` / `Error` / `Cancelled` /
    /// `Unresolved`, never `Pending`) whose own call id matches; order is
    /// free, results are written back in the original call order. Calls of
    /// the same batch / step that were never dispatched may additionally be
    /// answered with `Cancelled`: they are then recorded as not executed
    /// and never dispatched. Missing, duplicate or unknown ids are rejected.
    ToolResults { results: Vec<(String, Observation)> },
    /// Paired with `ContextLimitReached` in function call mode. The history
    /// becomes the new stable base: it replaces both `request.input` and
    /// `accumulated`. It must keep every tool call paired with its result;
    /// thinking blocks are dropped (they are only valid under the prefix
    /// they were produced with).
    RewrittenHistory { history: Vec<AiMessage> },
    /// Paired with `ContextLimitReached` in behavior mode, where the prompt is
    /// materialized from `request.input`, the step history and the hot step.
    /// Replaces `request.input`, `history_summaries`, `steps` and `last_step`;
    /// `steps` / `last_step` may only keep (possibly compacted) steps of the
    /// snapshot, identified by `step_index`. Step / action numbering,
    /// `history_inputs` and the in-progress turn (messages after
    /// `request.input` in `accumulated`) are kept. Thinking blocks are
    /// dropped as for `RewrittenHistory`. Folding the whole materialized
    /// history into `input` (empty steps, no hot step) is valid.
    RewrittenSteps {
        input: Vec<AiMessage>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        history_summaries: Vec<HistorySummaryRecord>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        steps: Vec<StepRecord>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_step: Option<StepRecord>,
    },
    /// Recovery of a snapshot that is *not* suspended: persisted at an
    /// outcome boundary, by a checkpoint hook before an inference, after an
    /// `Interrupted` outcome, or after a fill was applied (a tool batch /
    /// behavior step may still be in progress and continues first).
    ResumeFromMidRun,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContextOutput {
    Text { content: String },
    Json { content: Value },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ContextRunTrace {
    pub trace_id: String,
    pub latency_ms: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_trace: Vec<ToolExecRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub llm_task_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BudgetKind {
    Tokens,
    Wallclock,
    CostUnits,
    ToolRounds,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextLimitKind {
    /// The estimated request reached `BudgetSpec.context_yield_threshold`.
    ApproachingWindow,
    /// The estimated request plus the completion reserve exceeds
    /// `BudgetSpec.context_window_tokens`; it was not sent.
    HardLimit,
    /// The provider refused the request with a structured context-length
    /// code (`ProviderFailure::ContextLimit`).
    ProviderRefused,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LLMContextOutcome {
    Done {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        output: ContextOutput,
        usage: AiUsage,
        response: AiResponse,
        trace: ContextRunTrace,
        /// Behavior Loop payload. `None` for traditional Agent Loop runs.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        behavior_result: Option<LLMBehaviorResult>,
    },

    /// Suspended: a tool / action returned `Observation::Pending` (only with
    /// `tool_policy.allow_deferred`). Dispatch stopped at that call; the
    /// calls after it stay in `snapshot.state.tool_batch` /
    /// `action_step` and run after the fill. The scheduler persists the
    /// snapshot before handing the call to its async executor, then resumes
    /// with `ResumeFill::ToolResults`. `trace` audits this run segment.
    PendingTool {
        pending: Vec<PendingToolCall>,
        snapshot: LLMContextSnapshot,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        deadline_ms: Option<u64>,
        #[serde(default)]
        trace: ContextRunTrace,
    },

    BudgetExhausted {
        which: BudgetKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        partial: Option<ContextOutput>,
        usage: AiUsage,
    },

    /// Terminal failure. `error.source()` tells the runtime who owns the
    /// recovery: `Runtime` errors (`Checkpoint`, `ToolRuntime`) leave the
    /// in-memory snapshot valid and resumable via `ResumeFromMidRun`, every
    /// other source means the run is spent. `trace` keeps the tool audit of
    /// the aborted run, including calls that were never dispatched.
    Error {
        error: LLMComputeError,
        usage: AiUsage,
        #[serde(default)]
        trace: ContextRunTrace,
    },

    /// Suspended at an inference boundary (first round, after tool results,
    /// after injections) because the request about to be sent does not fit;
    /// nothing was sent. The snapshot is the pre-inference state.
    /// `accumulated` is the history the scheduler may rewrite: function call
    /// mode `state.accumulated`; behavior mode the materialized prompt
    /// (input, rendered step history, hot step) without the in-progress turn,
    /// which the waist keeps verbatim. Resume with `RewrittenHistory` /
    /// `RewrittenSteps`; a history that still does not fit yields again
    /// without inference.
    ContextLimitReached {
        which: ContextLimitKind,
        usage: AiUsage,
        accumulated: Vec<AiMessage>,
        snapshot: LLMContextSnapshot,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        deadline_ms: Option<u64>,
        #[serde(default)]
        trace: ContextRunTrace,
    },

    /// Suspended: `run()` was preempted by an external interrupt handle while
    /// an inference was in flight. The snapshot is the state captured **before**
    /// the aborted inference started — no partial assistant tokens / tool calls
    /// enter `accumulated`. Resume by feeding this snapshot back with
    /// `ResumeFill::ResumeFromMidRun`; the next `run()` will retry the inference
    /// from that point.
    Interrupted {
        reason: String,
        usage: AiUsage,
        snapshot: LLMContextSnapshot,
        abort: InferenceAbortTrace,
    },
}

impl LLMContextOutcome {
    /// True for `Done` / `Error` / `BudgetExhausted` — the object is consumed
    /// and cannot be resumed.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            LLMContextOutcome::Done { .. }
                | LLMContextOutcome::Error { .. }
                | LLMContextOutcome::BudgetExhausted { .. }
        )
    }
}
