//! Outputs / outcomes of one LLMContext run.
//!
//! Outcomes split into two structural classes (see `LLM Context 设计.md` §3.10):
//! - **Terminal**: `Done` / `Error` / `BudgetExhausted` — object is consumed.
//! - **Suspended**: `PendingTool` / `ContextLimitReached` —
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

use crate::behavior_loop::LLMBehaviorResult;
use crate::error::LLMComputeError;
use crate::interrupt::InferenceAbortTrace;
use crate::observation::{Observation, PendingToolCall, ToolExecRecord};
use crate::state::LLMContextSnapshot;

/// What the scheduler feeds back when resuming from a suspended outcome.
/// The variant must match the suspension that produced the snapshot.
///
/// Serialised as a tagged enum (`{"kind": "...", ...}`) so snapshot + fill can
/// travel over JSON across processes (L4 OneShot crash recovery relies on this).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResumeFill {
    /// Paired with `PendingTool`. `(call_id, observation)` must cover every
    /// pending call from the snapshot.
    ToolResults { results: Vec<(String, Observation)> },
    /// Paired with `ContextLimitReached`. The scheduler decides how to
    /// compress — waist just replaces accumulated history with this.
    RewrittenHistory { history: Vec<AiMessage> },
    /// "Crashed-mid-run" recovery (§3.1 / §6.6 of the design doc). The
    /// snapshot was *not* produced by a suspension outcome but was persisted
    /// by an L4 persistence layer at an outcome boundary or via a
    /// [`crate::deps::TurnHook`] before the next inference.
    ///
    /// No payload — there is nothing for the caller to feed back. The waist
    /// validates on resume that the snapshot is *not* in any suspended state
    /// (`pending_tool_calls` must be empty); mid-run snapshots paired with
    /// a different fill variant — or suspended snapshots paired with this
    /// variant — both yield `LLMComputeError::SnapshotCorrupted`.
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
    /// Triggered by `BudgetSpec.context_yield_threshold`.
    ApproachingWindow,
    /// Provider's actual hard window edge was hit.
    HardLimit,
    /// Provider explicitly refused (e.g. OpenAI `context_length_exceeded`).
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

    PendingTool {
        pending: Vec<PendingToolCall>,
        snapshot: LLMContextSnapshot,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        deadline_ms: Option<u64>,
    },

    BudgetExhausted {
        which: BudgetKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        partial: Option<ContextOutput>,
        usage: AiUsage,
    },

    Error {
        error: LLMComputeError,
        usage: AiUsage,
    },

    ContextLimitReached {
        which: ContextLimitKind,
        usage: AiUsage,
        accumulated: Vec<AiMessage>,
        snapshot: LLMContextSnapshot,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        deadline_ms: Option<u64>,
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
