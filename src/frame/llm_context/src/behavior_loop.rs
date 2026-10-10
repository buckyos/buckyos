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
use crate::state::LLMContextSnapshot;

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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HistoryInputRecord {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    pub text: String,
    pub at_ms: u64,
}

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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub report_artifacts: Vec<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_report_result"
    )]
    pub report_result: Option<serde_json::Value>,
    /// SendMessage actions (`<sendmsg target=...>`) emitted in this step.
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
    /// Optional user message to render after this step's assistant message.
    /// When unset, the configured [`StepRenderer`] renders the default
    /// action-result observation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_user_message: Option<AiMessage>,
}

/// One `<sendmsg target=...>` emit. v2 stub: recorded on
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
            report_end,
            report_artifacts,
            report_result,
            messages_to_send,
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
            report_artifacts,
            report_result,
            messages_sent: messages_to_send,
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
            report_artifacts: Vec::new(),
            report_result: None,
            messages_sent: Vec::new(),
            action_results: vec![Observation::Error {
                call_id: String::new(),
                message,
                tool_result: None,
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
    /// Dispatchable actions extracted from the `<actions>` container,
    /// excluding parser-side tags like `<sendmsg>` and `<report>`.
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub report_artifacts: Vec<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_report_result"
    )]
    pub report_result: Option<serde_json::Value>,
    /// SendMessage actions (`<sendmsg target=...>`); recorded in order of
    /// appearance. Stub-delivered in v2 first cut.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages_to_send: Vec<SendMessageRecord>,
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
        if self.self_report.is_none()
            && (!self.report_artifacts.is_empty() || self.report_result.is_some())
        {
            return Err("artifacts/result require a report in the same response".into());
        }
        if self.report_end {
            if self
                .self_report
                .as_deref()
                .is_none_or(|report| report.trim().is_empty())
            {
                return Err("an ending report must contain non-empty text".into());
            }
            if !self.do_actions.is_empty() || !self.messages_to_send.is_empty() {
                return Err(
                    "an ending report cannot contain actions, sendmsg, or native tool_calls".into(),
                );
            }
            if self
                .next_behavior
                .as_deref()
                .is_some_and(|next| !next.trim().is_empty())
            {
                return Err("an ending report cannot contain next_behavior".into());
            }
        }
        if self.self_report.is_none()
            && self.do_actions.is_empty()
            && self.messages_to_send.is_empty()
            && self
                .next_behavior
                .as_deref()
                .is_none_or(|next| next.trim().is_empty())
        {
            return Err("response requires actions, report, or next_behavior; submit <report end=\"true\"> to finish".into());
        }
        Ok(())
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
            report_artifacts: step.report_artifacts.clone(),
            report_result: step.report_result.clone(),
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
        inputs: Vec<HistoryInputRecord>,
    ) -> Vec<AiMessage> {
        let mut out = Vec::with_capacity(steps.len() * 2 + summaries.len());
        for summary in &summaries {
            out.push(self.render_summary(summary));
        }
        for input in inputs {
            out.push(AiMessage::text(
                buckyos_api::AiRole::User,
                format!(
                    "history input:\nsource: {}\nat_ms: {}\n{}",
                    input.source, input.at_ms, input.text
                ),
            ));
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

#[derive(Debug, Clone, Default)]
pub struct StepResultHookOutput {
    /// Override the user message rendered immediately after this step on the
    /// next behavior inference. `None` keeps the renderer default.
    pub user_message: Option<AiMessage>,
    /// Extra history inputs to append before the next behavior inference.
    pub history_inputs: Vec<HistoryInputRecord>,
    /// Stop the behavior loop instead of starting the next inference.
    pub skip_next_inference: bool,
}

/// Post-step extension point. Failure contract: an `Err` is always treated
/// as a degradable failure — the loop logs it and renders the default
/// observation. Implementations therefore must not commit state (consume
/// inputs, advance cursors) before they can return `Ok`; anything that must
/// block progress on failure belongs in `InferenceHook::before_inference`.
#[async_trait]
pub trait StepResultHook: Send + Sync {
    async fn on_behavior_step_ob(
        &self,
        snapshot: &LLMContextSnapshot,
        step: &StepRecord,
    ) -> Result<StepResultHookOutput, String>;
}
