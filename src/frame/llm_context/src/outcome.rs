//! Outputs / outcomes of one LLMContext run.
//!
//! Outcomes split into two structural classes (see `LLM Context 设计.md` §3.10):
//! - **Terminal**: `Done` / `Error` / `BudgetExhausted` — object is consumed.
//! - **Suspended**: `WaitInput` / `PendingTool` / `ContextLimitReached` —
//!   a `LLMContextSnapshot` is produced and the run is resumable.

use buckyos_api::{AiMessage, AiResponseSummary, AiUsage};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::LLMComputeError;
use crate::observation::{Observation, PendingToolCall, ToolExecRecord};
use crate::state::LLMContextSnapshot;

/// What the scheduler feeds back when resuming from a suspended outcome.
/// The variant must match the suspension that produced the snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResumeFill {
    /// Paired with `PendingTool`. `(call_id, observation)` must cover every
    /// pending call from the snapshot.
    ToolResults { results: Vec<(String, Observation)> },
    /// Paired with `WaitInput`.
    HumanInput { message: AiMessage },
    /// Paired with `ContextLimitReached`. The scheduler decides how to
    /// compress — waist just replaces accumulated history with this.
    RewrittenHistory { history: Vec<AiMessage> },
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
        response: AiResponseSummary,
        trace: ContextRunTrace,
    },

    WaitInput {
        reason: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompt_to_human: Option<String>,
        snapshot: LLMContextSnapshot,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        deadline_ms: Option<u64>,
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
