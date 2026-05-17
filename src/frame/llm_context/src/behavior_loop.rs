//! Behavior Loop — the outer slim-waist scheduler that sits on top of
//! `run_inner` (traditional Agent Loop).
//!
//! See `notepads/llm_context_behavior_loop.md` for the design rationale.
//! This module only ships the **types and trait signatures**. The wiring
//! into the loop driver lives in `context_loop.rs::run_behavior`.

use async_trait::async_trait;
use buckyos_api::{AiMessage, AiResponse, AiToolCall};
use serde::{Deserialize, Serialize};

use crate::observation::Observation;

/// One Behavior step. Carries both the LLM-emitted intent (`assistant_text`
/// + the parsed slots) and the dispatcher-side echo (`action_results`).
///
/// Persisted into `LLMContextState::steps` once sedimented; the freshest
/// (still-hot) step lives in `LLMContextState::last_step` and is rendered
/// verbatim into the next inference.
///
/// v2 of the Behavior protocol (`doc/opendan/Agent Actions.md`) allows
/// multiple actions per step via the `<actions>` container. `actions` is
/// `Vec<_>` and `action_results` is index-aligned with it.
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
    /// "Actions" slot — zero or more actions per step. Empty on
    /// pure-thought / terminal-only steps.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<AiToolCall>,
    /// "Next behavior" slot — when `Some`, this step is terminal and the
    /// actions (if any) are still dispatched first, then the loop returns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_behavior: Option<String>,
    /// Self Report (`<report>` without `target`) — overwrites
    /// `LLMContextState.last_report` at dispatch time. Kept on the step too
    /// so the rendered history preserves the report-emit event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub self_report: Option<String>,
    /// SendMessage-form reports (`<report target=...>`) emitted in this step.
    /// Stub in v2 first cut: parser captures them, executor only emits a
    /// worklog event. Real delivery moves to a standard `send_message`
    /// agent_tool in a later phase.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages_sent: Vec<SendMessageRecord>,

    // —— Filled by executor (after action dispatch) ——
    /// Per-action observation, index-aligned with `actions`. Empty on
    /// steps with no actions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub action_results: Vec<Observation>,
}

/// One `<report target=...>` emit (SendMessage form). v2 stub: recorded on
/// the step for transcript / audit; not actually delivered yet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendMessageRecord {
    pub target: String,
    pub body: String,
}

impl StepRecord {
    /// Build a step from a parser result. `action_results` is left empty —
    /// the dispatcher fills it after running the actions.
    pub fn from_result(result: LLMBehaviorResult) -> Self {
        let LLMBehaviorResult {
            assistant_text,
            observation,
            thought,
            do_actions,
            next_behavior,
            self_report,
            messages_to_send,
        } = result;
        Self {
            assistant_text,
            observation,
            thought,
            actions: do_actions,
            next_behavior,
            self_report,
            messages_sent: messages_to_send,
            action_results: Vec::new(),
        }
    }

    /// Synthetic step describing a parser failure. The error text is exposed
    /// to the LLM as a synthetic error observation so the next inference can
    /// self-correct (FeedAsObservation style).
    pub fn from_parse_error(error: &str) -> Self {
        Self::synthetic_error(format!("parse failed: {error}"))
    }

    /// Synthetic step describing a policy rejection (e.g. an invocation
    /// outside the behavior's whitelist). Same FeedAsObservation shape as
    /// [`Self::from_parse_error`] but tagged so the LLM can tell why the
    /// step was discarded.
    pub fn from_policy_rejection(error: &str) -> Self {
        Self::synthetic_error(format!("policy rejected: {error}"))
    }

    fn synthetic_error(message: String) -> Self {
        Self {
            assistant_text: String::new(),
            observation: None,
            thought: None,
            actions: Vec::new(),
            next_behavior: None,
            self_report: None,
            messages_sent: Vec::new(),
            action_results: vec![Observation::Error {
                call_id: String::new(),
                message,
            }],
        }
    }
}

/// Structured product of one LLM inference. Produced by the parser, consumed
/// by the loop (reads `do_actions` / `next_behavior` to decide what to do)
/// and by `StepRecord::from_result`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LLMBehaviorResult {
    /// Dispatchable actions extracted from the `<actions>` container,
    /// excluding `<report>` (which is captured separately below).
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

    /// Self Report (`<report>` without `target`) — at most one per step;
    /// last occurrence wins. Overwrites `LLMContextState.last_report`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub self_report: Option<String>,
    /// SendMessage-form reports (`<report target=...>`); recorded in order of
    /// appearance. Stub-delivered in v2 first cut.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages_to_send: Vec<SendMessageRecord>,
}

impl LLMBehaviorResult {
    /// Reconstruct a behavior result from a finished step. Used at terminal
    /// time so `Done.behavior_result` carries the same payload that the
    /// parser produced.
    pub fn from_step(step: &StepRecord) -> Self {
        Self {
            do_actions: step.actions.clone(),
            next_behavior: step.next_behavior.clone(),
            assistant_text: step.assistant_text.clone(),
            observation: step.observation.clone(),
            thought: step.thought.clone(),
            self_report: step.self_report.clone(),
            messages_to_send: step.messages_sent.clone(),
        }
    }
}

/// Parses one raw LLM response into the structured behavior result. The
/// concrete protocol (JSON schema, ReAct-style markdown, ...) lives in the
/// worksession-injected implementation; the waist only sees the trait.
pub trait LLMResultParser: Send + Sync {
    fn parse(&self, response: &AiResponse) -> Result<LLMBehaviorResult, String>;
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
