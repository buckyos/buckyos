//! Behavior Loop — the outer slim-waist scheduler that sits on top of
//! `run_inner` (traditional Agent Loop).
//!
//! See `notepads/llm_context_behavior_loop.md` for the design rationale.
//! This module only ships the **types and trait signatures**. The wiring
//! into the loop driver lives in `context_loop.rs::run_behavior`.

use async_trait::async_trait;
use buckyos_api::{AiMessage, AiResponseSummary, AiToolCall};
use serde::{Deserialize, Serialize};

use crate::observation::Observation;

/// One Behavior step. Carries both the LLM-emitted intent (`assistant_text`
/// + the 4 schema slots) and the dispatcher-side echo (`action_result`).
///
/// Persisted into `LLMContextState::steps` once sedimented; the freshest
/// (still-hot) step lives in `LLMContextState::last_step` and is rendered
/// verbatim into the next inference.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StepRecord {
    // —— Filled by parser (before action dispatch) ——
    /// Raw LLM response text, used verbatim as assistant message content
    /// when this step is rendered back to the LLM.
    pub assistant_text: String,

    /// "Observation" slot — LLM's reading of the previous action's result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation: Option<String>,
    /// "Thought" slot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thought: Option<String>,
    /// "Action" slot — at most one action per step in v1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<AiToolCall>,
    /// "Next behavior" slot — when `Some`, this step is terminal and the
    /// action (if any) is **not** dispatched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_behavior: Option<String>,

    // —— Filled by executor (after action dispatch) ——
    /// Echo of the dispatched action. `None` on terminal steps or when there
    /// was no action (pure-thought step).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_result: Option<Observation>,
}

impl StepRecord {
    /// Build a step from a parser result. `action_result` is left empty —
    /// the dispatcher fills it after running the action.
    pub fn from_result(result: LLMBehaviorResult) -> Self {
        let LLMBehaviorResult {
            assistant_text,
            observation,
            thought,
            do_actions,
            next_behavior,
        } = result;
        Self {
            assistant_text,
            observation,
            thought,
            action: do_actions.into_iter().next(),
            next_behavior,
            action_result: None,
        }
    }

    /// Synthetic step describing a parser failure. The error text is exposed
    /// to the LLM as `assistant_text` and as a synthetic error observation,
    /// so the next inference can self-correct (FeedAsObservation style).
    pub fn from_parse_error(error: &str) -> Self {
        Self {
            assistant_text: String::new(),
            observation: None,
            thought: None,
            action: None,
            next_behavior: None,
            action_result: Some(Observation::Error {
                call_id: String::new(),
                message: format!("parse failed: {error}"),
            }),
        }
    }
}

/// Structured product of one LLM inference. Produced by the parser, consumed
/// by the loop (reads `do_actions` / `next_behavior` to decide what to do)
/// and by `StepRecord::from_result`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LLMBehaviorResult {
    /// Dispatchable actions extracted from the "Action" slot. v1 ships at
    /// most one entry; the field is `Vec` to leave room for parallel actions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub do_actions: Vec<AiToolCall>,
    /// Terminal signal + jump target. `Some(_)` is terminal; the loop does
    /// **not** interpret the string — that belongs to the worksession above.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_behavior: Option<String>,

    // —— Carried through unchanged ——
    pub assistant_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thought: Option<String>,
}

impl LLMBehaviorResult {
    /// Reconstruct a behavior result from a finished step. Used at terminal
    /// time so `Done.behavior_result` carries the same payload that the
    /// parser produced.
    pub fn from_step(step: &StepRecord) -> Self {
        Self {
            do_actions: step.action.clone().into_iter().collect(),
            next_behavior: step.next_behavior.clone(),
            assistant_text: step.assistant_text.clone(),
            observation: step.observation.clone(),
            thought: step.thought.clone(),
        }
    }
}

/// Parses one raw LLM response into the structured behavior result. The
/// concrete protocol (JSON schema, ReAct-style markdown, ...) lives in the
/// worksession-injected implementation; the waist only sees the trait.
pub trait LLMResultParser: Send + Sync {
    fn parse(&self, response: &AiResponseSummary) -> Result<LLMBehaviorResult, String>;
}

/// Renders sedimented history + the hot `last_step` back into AiMessages for
/// the next inference. One full step renders as a pair `(assistant, user)`,
/// which keeps strict role alternation and trains-distribution-friendliness.
pub trait StepRenderer: Send + Sync {
    /// Render one step into an `(assistant, user)` pair.
    fn render(&self, step: &StepRecord) -> (AiMessage, AiMessage);

    /// Render the historical (sedimented) tail. Default implementation just
    /// concatenates `render()` over each step; implementors may override for
    /// summary-style compression artifacts.
    fn render_history(&self, steps: Vec<StepRecord>) -> Vec<AiMessage> {
        let mut out = Vec::with_capacity(steps.len() * 2);
        for step in &steps {
            let (a, u) = self.render(step);
            out.push(a);
            out.push(u);
        }
        out
    }
}

/// Budget hint passed into the compressor. Implementations may ignore it.
#[derive(Debug, Clone, Default)]
pub struct CompressBudget {
    /// Soft target for the number of steps to keep after compression.
    pub target_steps: Option<usize>,
    /// Soft target for the total token estimate of the rendered history.
    pub target_tokens: Option<u32>,
}

/// Compressor error — opaque to the waist; surfaced as `LLMComputeError::Internal`
/// when bubble-up is needed (current design just skips compression on error).
#[derive(Debug, Clone)]
pub struct CompressError(pub String);

impl std::fmt::Display for CompressError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "history compress failed: {}", self.0)
    }
}

impl std::error::Error for CompressError {}

/// Optional history compressor. Same role as `ResumeFill::RewrittenHistory`
/// but triggered from inside the outer loop rather than by an external
/// scheduler. Compression output is still `Vec<StepRecord>` — alternation /
/// renderer contract are preserved.
#[async_trait]
pub trait HistoryCompressor: Send + Sync {
    async fn compress(
        &self,
        steps: Vec<StepRecord>,
        budget: CompressBudget,
    ) -> Result<Vec<StepRecord>, CompressError>;
}
