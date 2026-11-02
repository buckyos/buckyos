//! Behavior Loop — the outer slim-waist scheduler that sits on top of
//! `run_inner` (traditional Agent Loop).
//!
//! This module only ships the **types and trait signatures**. The wiring
//! into the loop driver lives in `context_loop.rs::run_behavior`; the
//! concrete response protocol (parser) and step rendering belong to the
//! host.

use buckyos_api::{AiMessage, AiResponse, AiToolCall};
use serde::{Deserialize, Serialize};

use crate::observation::Observation;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StepCompressionLevel {
    Full,
    Compact,
    Summary,
}

impl Default for StepCompressionLevel {
    fn default() -> Self {
        Self::Full
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StepMeta {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub behavior_name: String,
    pub step_index: u32,
    pub started_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at_ms: Option<u64>,
    #[serde(default)]
    pub compression_level: StepCompressionLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HistorySummaryRecord {
    pub step_count: u32,
    pub start_step_index: u32,
    pub end_step_index: u32,
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub behavior_names: Vec<String>,
    pub summary: String,
}

/// One Behavior step. Carries both the LLM-emitted intent (`assistant_text`
/// + the parsed slots) and the dispatcher-side echo (`action_results`).
///
/// Persisted into `LLMContextState::steps` once sedimented; the freshest
/// (still-hot) step lives in `LLMContextState::last_step` and is rendered
/// verbatim into the next inference.
///
/// A step carries zero or more actions; `action_results` is index-aligned
/// with `actions`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StepRecord {
    #[serde(default)]
    pub meta: StepMeta,

    // —— Filled by parser (before action dispatch) ——
    /// Raw LLM response text, used verbatim as assistant message content
    /// when this step is rendered back to the LLM.
    pub assistant_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_message: Option<AiMessage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub native_messages: Vec<AiMessage>,

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_behavior: Option<String>,
    /// Self Report (`<report>`) — overwrites
    /// `LLMContextState.last_report` at dispatch time. Kept on the step too
    /// so the rendered history preserves the report-emit event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub self_report: Option<String>,
    #[serde(default)]
    pub report_end: bool,
    /// Structured payload of the report; its shape (including any
    /// attachments) is the host's convention.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_report_result"
    )]
    pub report_result: Option<serde_json::Value>,

    // —— Filled by executor (after action dispatch) ——
    /// Per-action observation, index-aligned with `actions`. Empty on
    /// steps with no actions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub action_results: Vec<Observation>,
    /// Optional user message to render after this step's assistant message.
    /// When unset, the configured [`StepRenderer`] renders the default
    /// action-result observation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_user_message: Option<AiMessage>,
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
            report_end,
            report_result,
        } = result;
        Self {
            meta: StepMeta::default(),
            assistant_text,
            assistant_message: None,
            native_messages: Vec::new(),
            observation,
            thought,
            actions: do_actions,
            next_behavior,
            self_report,
            report_end,
            report_result,
            action_results: Vec::new(),
            next_user_message: None,
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

    /// `true` for a synthetic step built by [`Self::from_parse_error`] /
    /// [`Self::from_policy_rejection`]: it records a rejected response fed
    /// back for self-correction, not a behavior decision. It still takes a
    /// step index (numbering is allocation order).
    pub fn is_correction(&self) -> bool {
        self.actions.is_empty()
            && matches!(
                self.action_results.as_slice(),
                [Observation::Error { call_id, .. }] if call_id.is_empty()
            )
    }

    fn synthetic_error(message: String) -> Self {
        Self {
            meta: StepMeta::default(),
            assistant_text: String::new(),
            assistant_message: None,
            native_messages: Vec::new(),
            observation: None,
            thought: None,
            actions: Vec::new(),
            next_behavior: None,
            self_report: None,
            report_end: false,
            report_result: None,
            action_results: vec![Observation::Error {
                call_id: String::new(),
                message,
                host: None,
            }],
            next_user_message: None,
        }
    }
}

/// Structured product of one LLM inference. Produced by the parser, consumed
/// by the loop (reads `do_actions` / `next_behavior` to decide what to do)
/// and by `StepRecord::from_result`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LLMBehaviorResult {
    /// Dispatchable actions of the step (the report is not an action).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub do_actions: Vec<AiToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_behavior: Option<String>,

    // —— Carried through unchanged ——
    pub assistant_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thought: Option<String>,

    /// Self Report (`<report>`) — at most one per step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub self_report: Option<String>,
    #[serde(default)]
    pub report_end: bool,
    /// Structured payload of the report (host convention).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_report_result"
    )]
    pub report_result: Option<serde_json::Value>,
}

fn deserialize_report_result<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<serde_json::Value>, D::Error> {
    serde_json::Value::deserialize(deserializer).map(Some)
}

impl LLMBehaviorResult {
    pub fn validate(&self) -> Result<(), String> {
        if self.next_behavior.as_deref().is_some_and(|next| {
            next.trim().eq_ignore_ascii_case("END") || next.trim().eq_ignore_ascii_case("done")
        }) {
            return Err(
                "next_behavior END/done is invalid; submit <report end=\"true\"> to finish".into(),
            );
        }
        if self.self_report.is_none() && self.report_result.is_some() {
            return Err("a report result requires a report in the same response".into());
        }
        if self.report_end {
            if self
                .self_report
                .as_deref()
                .is_none_or(|report| report.trim().is_empty())
            {
                return Err("an ending report must contain non-empty text".into());
            }
            if !self.do_actions.is_empty() {
                return Err("an ending report cannot contain actions or native tool_calls".into());
            }
            if self
                .next_behavior
                .as_deref()
                .is_some_and(|next| !next.trim().is_empty())
            {
                return Err("an ending report cannot contain next_behavior".into());
            }
        }
        Ok(())
    }

    /// What this result decides about the control of its context, when it
    /// ends the run: a hand-over (`next_behavior` without actions — with
    /// them the hand-over is ignored anyway) or a yield (no action, not an
    /// end, no hand-over; a progress report is optional). Pure shape check;
    /// the meaning is the host's. `None` for a step with actions or an
    /// ending report.
    pub fn decision(&self) -> Option<StepDecision> {
        if !self.do_actions.is_empty() || self.report_end {
            return None;
        }
        if self
            .next_behavior
            .as_deref()
            .is_some_and(|next| !next.trim().is_empty())
        {
            return Some(StepDecision::Handover);
        }
        Some(StepDecision::Yield)
    }

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
            report_end: step.report_end,
            report_result: step.report_result.clone(),
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

    fn render_inherited(&self, step: &StepRecord) -> AiMessage {
        AiMessage::text(
            buckyos_api::AiRole::User,
            format!(
                "history step record:\nbehavior: {}\nindex: {}\nsummary: {}",
                step.meta.behavior_name,
                step.meta.step_index,
                step.thought
                    .as_deref()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| step.assistant_text.trim())
            ),
        )
    }

    fn render_summary(&self, summary: &HistorySummaryRecord) -> AiMessage {
        AiMessage::text(
            buckyos_api::AiRole::User,
            format!(
                "history summary:\nsteps: {}..{}\ncount: {}\nbehaviors: {}\nsummary: {}",
                summary.start_step_index,
                summary.end_step_index,
                summary.step_count,
                summary.behavior_names.join(", "),
                summary.summary
            ),
        )
    }

    fn render_history(
        &self,
        steps: Vec<StepRecord>,
        current_behavior: &str,
        summaries: Vec<HistorySummaryRecord>,
    ) -> Vec<AiMessage> {
        let mut out = Vec::with_capacity(steps.len() * 2 + summaries.len());
        for summary in &summaries {
            out.push(self.render_summary(summary));
        }
        for step in &steps {
            if !current_behavior.is_empty() && step.meta.behavior_name != current_behavior {
                out.push(self.render_inherited(step));
            } else {
                let (a, u) = self.render(step);
                out.push(a);
                out.push(u);
            }
        }
        out
    }
}

/// A behavior step that changes who acts next, offered to the host for
/// review ([`crate::deps::CheckpointHook::review_decision`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepDecision {
    /// `next_behavior` names another behavior (or a host sentinel).
    Handover,
    /// No action, not an end, no hand-over (a progress report is
    /// optional): the context stops inferring until the host gives it new
    /// input. Ends the run with `Done`; the waist does not judge whether
    /// there is anything to wait for.
    Yield,
}

/// The host's verdict on a [`StepDecision`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionVerdict {
    /// The decision stands: the run ends with `Done`.
    Accept,
    /// The decision is refused: the step becomes a correction step whose
    /// text is this message, and the same run infers again (counts as a
    /// consecutive error, like a refused report).
    Refuse(String),
}

