//! Mutable runtime state of an LLMContext.
//!
//! `LLMContextState` corresponds to "registers + stack" in the process
//! analogy. `LLMContextSnapshot` is the serialisable freeze produced when
//! the context yields (suspended outcomes) — it must be self-contained per
//! §6.2 of the design doc.

use buckyos_api::{AiMessage, AiUsage};
use serde::{Deserialize, Serialize};

use crate::behavior_loop::StepRecord;
use crate::observation::PendingToolCall;
use crate::request::LLMContextRequest;

/// Runtime mutable half of one LLMContext.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMContextState {
    /// Full message list currently visible to the LLM. Starts as a clone of
    /// `request.input`; the loop appends assistant replies and tool messages
    /// each round.
    pub accumulated: Vec<AiMessage>,

    /// Aggregate usage across every inference performed so far in this run.
    pub usage: AiUsage,

    /// Tool rounds remaining. Initialised from `tool_policy.max_rounds`.
    pub rounds_left: u32,

    /// Wallclock at which `run()` first started, in ms since epoch.
    pub started_at_ms: u64,

    /// Cumulative cost in scheduler-defined units. We only track the counter
    /// — the meaning is up to the owner.
    pub cost_units: u32,

    /// Consecutive Recoverable errors fed back as observations. Reset on a
    /// successful inference.
    pub consecutive_errors: u32,

    /// Pending tool calls awaiting resume (only set when an outcome of
    /// `PendingTool` was just produced). Resume fills these from
    /// `ResumeFill::ToolResults`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_tool_calls: Vec<PendingToolCall>,

    /// IDs of provider tasks issued by this run. Captured for trace output.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub llm_task_ids: Vec<String>,

    /// Behavior mode: sedimented step history (compression-eligible).
    /// Always empty in traditional mode.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<StepRecord>,

    /// Behavior mode: the freshest step still being processed — rendered
    /// verbatim into the next inference. `None` until the first iteration
    /// finishes parsing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_step: Option<StepRecord>,
}

impl LLMContextState {
    pub fn from_request(req: &LLMContextRequest, started_at_ms: u64) -> Self {
        Self {
            accumulated: req.input.clone(),
            usage: AiUsage {
                input_tokens: None,
                output_tokens: None,
                total_tokens: None,
            },
            rounds_left: req.tool_policy.max_rounds,
            started_at_ms,
            cost_units: 0,
            consecutive_errors: 0,
            pending_tool_calls: Vec::new(),
            llm_task_ids: Vec::new(),
            steps: Vec::new(),
            last_step: None,
        }
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
