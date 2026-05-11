//! Core LLMContext driver — the "process" object that runs one bounded
//! LLM execution to completion (or to a cooperative yield).
//!
//! First version: traditional AiMessage-accumulating loop.
//! - one LLM inference per round
//! - if `tool_calls` are produced, policy-gate them, run them through
//!   `ToolManager`, append the assistant tool-call message + tool result
//!   messages to `accumulated`, and loop
//! - terminate with `Done` once the LLM stops requesting tools
//! - terminate with `BudgetExhausted` on token / wallclock / round caps
//! - terminate with `Error` on Fatal errors; Recoverable errors are handled
//!   per `ErrorPolicy`
//!
//! Suspension outcomes (`WaitInput`, `PendingTool`, `ContextLimitReached`)
//! are *defined* but not actively produced in this first version — they
//! require deferred tools and explicit human-input requests to be wired up.

use std::time::{SystemTime, UNIX_EPOCH};

use buckyos_api::{AiMessage, AiResponseSummary, AiToolCall, AiUsage};
use serde_json::Value;

use crate::deps::{
    resolve_tool_specs, LLMContextDeps, LlmInferenceRequest, WorkEvent,
};
use crate::error::LLMComputeError;
use crate::observation::{Observation, ToolExecRecord};
use crate::outcome::{
    BudgetKind, ContextOutput, ContextRunTrace, LLMContextOutcome, ResumeFill,
};
use crate::request::{
    ErrorClass, ErrorMode, LLMContextRequest, OutputSpec, ToolMode,
};
use crate::state::{LLMContextSnapshot, LLMContextState};

pub struct LLMContext {
    request: LLMContextRequest,
    state: LLMContextState,
    deps: LLMContextDeps,
    /// Per-run tool audit, flushed into the final `Done` trace.
    tool_trace: Vec<ToolExecRecord>,
    /// Last raw provider response. Carried so we can populate
    /// `Outcome::Done.response`.
    last_response: AiResponseSummary,
}

impl LLMContext {
    pub fn new(request: LLMContextRequest, deps: LLMContextDeps) -> Self {
        let started = now_ms();
        let state = LLMContextState::from_request(&request, started);
        Self {
            request,
            state,
            deps,
            tool_trace: Vec::new(),
            last_response: AiResponseSummary::default(),
        }
    }

    /// Resume a previously-yielded context with the data the scheduler
    /// gathered while it was suspended.
    pub fn resume(
        snapshot: LLMContextSnapshot,
        fill: ResumeFill,
        deps: LLMContextDeps,
    ) -> Result<Self, LLMComputeError> {
        let LLMContextSnapshot { request, mut state } = snapshot;

        match fill {
            ResumeFill::ToolResults { results } => {
                if state.pending_tool_calls.is_empty() {
                    return Err(LLMComputeError::SnapshotCorrupted(
                        "ToolResults fill but no pending calls".to_string(),
                    ));
                }
                // Append tool messages back to accumulated history. We require
                // the caller to provide one observation per pending call.
                let pending = std::mem::take(&mut state.pending_tool_calls);
                if results.len() != pending.len() {
                    return Err(LLMComputeError::SnapshotCorrupted(format!(
                        "ToolResults length {} != pending {}",
                        results.len(),
                        pending.len()
                    )));
                }
                for (call, (call_id, obs)) in pending.iter().zip(results.into_iter()) {
                    if call.call.call_id != call_id {
                        return Err(LLMComputeError::SnapshotCorrupted(format!(
                            "call_id mismatch: expected {}, got {}",
                            call.call.call_id, call_id
                        )));
                    }
                    state
                        .accumulated
                        .push(tool_observation_message(&call.call.name, &obs));
                }
            }
            ResumeFill::HumanInput { message } => {
                state.accumulated.push(message);
            }
            ResumeFill::RewrittenHistory { history } => {
                state.accumulated = history;
            }
            ResumeFill::ResumeFromMidRun => {
                // §3.1 / §6.6 nail this down: a mid-run recovery is only valid
                // when the snapshot is **not** in any suspended state.
                // Suspended snapshots carry data the caller must feed back
                // through the matching ResumeFill variant; silently treating
                // them as mid-run would drop unanswered tool_use entries from
                // the accumulated transcript.
                if !state.pending_tool_calls.is_empty() {
                    return Err(LLMComputeError::SnapshotCorrupted(
                        "ResumeFromMidRun fill but snapshot has pending tool calls".to_string(),
                    ));
                }
            }
        }

        Ok(Self {
            request,
            state,
            deps,
            tool_trace: Vec::new(),
            last_response: AiResponseSummary::default(),
        })
    }

    pub fn snapshot(&self) -> LLMContextSnapshot {
        LLMContextSnapshot {
            request: self.request.clone(),
            state: self.state.clone(),
        }
    }

    /// Run the loop until an outcome is produced.
    pub async fn run(&mut self) -> LLMContextOutcome {
        self.deps
            .worklog
            .emit(WorkEvent::LLMStarted {
                trace_id: self.request.trace.clone(),
                model: self.request.model_policy.preferred.clone(),
            })
            .await;

        let outcome = self.run_inner().await;

        self.deps
            .worklog
            .emit(WorkEvent::LLMFinished {
                trace_id: self.request.trace.clone(),
                ok: matches!(outcome, LLMContextOutcome::Done { .. }),
            })
            .await;

        outcome
    }

    async fn run_inner(&mut self) -> LLMContextOutcome {
        loop {
            if let Some(budget_outcome) = self.check_wallclock_budget() {
                return budget_outcome;
            }

            // 1. Inference — fire TurnHook (§3.12) before the call so the L4
            // persistence layer can checkpoint "right before we pay for the
            // next inference".
            if let Some(hook) = &self.deps.turn_hook {
                let snap = self.snapshot();
                hook.before_inference(&snap);
            }
            let infer_req = self.build_inference_request();
            let response = match self.deps.llm.infer(infer_req).await {
                Ok(resp) => resp,
                Err(err) => {
                    self.deps
                        .worklog
                        .emit(WorkEvent::LLMInferenceFailed {
                            trace_id: self.request.trace.clone(),
                            error: err.to_string(),
                        })
                        .await;
                    if let Some(outcome) = self.handle_error(classify(err)).await {
                        return outcome;
                    } else {
                        continue;
                    }
                }
            };

            self.account_response(&response);
            self.last_response = response.clone();

            if let Some(provider_task) = &response.provider_task_ref {
                if !provider_task.trim().is_empty() {
                    self.state.llm_task_ids.push(provider_task.clone());
                }
            }

            if let Some(budget_outcome) = self.check_token_budget(&response) {
                return budget_outcome;
            }

            let tool_calls = response.tool_calls.clone();
            let assistant_text = response.text.clone().unwrap_or_default();

            // 2. No tool calls ⇒ finish
            if tool_calls.is_empty() || self.request.tool_policy.mode == ToolMode::None {
                return self.finish_done(response).await;
            }

            // 3. Tool loop bookkeeping
            if self.state.rounds_left == 0 {
                return LLMContextOutcome::BudgetExhausted {
                    which: BudgetKind::ToolRounds,
                    partial: Some(ContextOutput::Text {
                        content: assistant_text.clone(),
                    }),
                    usage: self.state.usage.clone(),
                };
            }

            if tool_calls.len() as u32 > self.request.tool_policy.max_calls_per_round {
                let err = LLMComputeError::Internal(format!(
                    "tool calls {} exceed max_calls_per_round {}",
                    tool_calls.len(),
                    self.request.tool_policy.max_calls_per_round
                ));
                if let Some(outcome) = self.handle_error(ErrorClass::Fatal(err)).await {
                    return outcome;
                }
                continue;
            }

            // 4. Policy gate
            let gated = match self
                .deps
                .policy
                .gate_tool_calls(&self.request, tool_calls.clone())
                .await
            {
                Ok(calls) => calls,
                Err(msg) => {
                    let err = LLMComputeError::PolicyRejected(msg);
                    if let Some(outcome) = self.handle_error(classify(err)).await {
                        return outcome;
                    } else {
                        continue;
                    }
                }
            };

            // 5. Push assistant message describing the tool_calls into history.
            // We persist the raw text (if any) plus a JSON envelope describing
            // the calls so the LLM stays self-consistent on the next round.
            self.state.accumulated.push(assistant_tool_call_message(
                &assistant_text,
                &gated,
            ));

            // 6. Execute calls (serial in v1).
            let mut had_error = false;
            for call in gated {
                let started = now_ms();
                self.deps
                    .worklog
                    .emit(WorkEvent::ToolCallPlanned {
                        trace_id: self.request.trace.clone(),
                        tool: call.name.clone(),
                        call_id: call.call_id.clone(),
                    })
                    .await;

                let observation = self.deps.tools.call_tool(call.clone()).await;
                let duration_ms = now_ms().saturating_sub(started);

                match &observation {
                    Observation::Pending { .. } => {
                        if !self.request.tool_policy.allow_deferred {
                            let err = LLMComputeError::Internal(
                                "tool returned Pending but allow_deferred=false".to_string(),
                            );
                            self.tool_trace.push(ToolExecRecord {
                                tool_name: call.name.clone(),
                                call_id: call.call_id.clone(),
                                ok: false,
                                duration_ms,
                                error: Some(err.to_string()),
                            });
                            if let Some(outcome) = self
                                .handle_error(ErrorClass::Fatal(err))
                                .await
                            {
                                return outcome;
                            }
                            had_error = true;
                            break;
                        }
                        // Deferred path is declared but not yet exercised in
                        // v1; we still record it so future iterations can
                        // turn this into Outcome::PendingTool.
                        self.tool_trace.push(ToolExecRecord {
                            tool_name: call.name.clone(),
                            call_id: call.call_id.clone(),
                            ok: false,
                            duration_ms,
                            error: Some("pending (deferred)".to_string()),
                        });
                        let err = LLMComputeError::Internal(
                            "deferred tool path not yet implemented".to_string(),
                        );
                        if let Some(outcome) =
                            self.handle_error(ErrorClass::Fatal(err)).await
                        {
                            return outcome;
                        }
                        had_error = true;
                        break;
                    }
                    Observation::Success { .. } => {
                        self.state
                            .accumulated
                            .push(tool_observation_message(&call.name, &observation));
                        self.tool_trace.push(ToolExecRecord {
                            tool_name: call.name.clone(),
                            call_id: call.call_id.clone(),
                            ok: true,
                            duration_ms,
                            error: None,
                        });
                        self.deps
                            .worklog
                            .emit(WorkEvent::ToolCallFinished {
                                trace_id: self.request.trace.clone(),
                                tool: call.name.clone(),
                                call_id: call.call_id.clone(),
                                ok: true,
                                duration_ms,
                            })
                            .await;
                    }
                    Observation::Error { message, .. } => {
                        // Feed the error back so the LLM can self-correct
                        // (when ErrorMode == FeedAsObservation).
                        self.tool_trace.push(ToolExecRecord {
                            tool_name: call.name.clone(),
                            call_id: call.call_id.clone(),
                            ok: false,
                            duration_ms,
                            error: Some(message.clone()),
                        });
                        self.deps
                            .worklog
                            .emit(WorkEvent::ToolCallFailed {
                                trace_id: self.request.trace.clone(),
                                tool: call.name.clone(),
                                call_id: call.call_id.clone(),
                                message: message.clone(),
                            })
                            .await;

                        // Always push observation message into accumulated —
                        // FeedAsObservation uses it directly; Suspend mode
                        // still benefits from a coherent transcript.
                        self.state
                            .accumulated
                            .push(tool_observation_message(&call.name, &observation));

                        let err = LLMComputeError::ToolFailed {
                            tool: call.name.clone(),
                            call_id: call.call_id.clone(),
                            message: message.clone(),
                        };
                        if let Some(outcome) =
                            self.handle_error(ErrorClass::Recoverable(err)).await
                        {
                            return outcome;
                        }
                        had_error = true;
                    }
                }
            }

            if !had_error {
                self.state.consecutive_errors = 0;
            }

            // 7. Round consumed.
            self.state.rounds_left = self.state.rounds_left.saturating_sub(1);
        }
    }

    fn build_inference_request(&self) -> LlmInferenceRequest {
        let tool_specs =
            resolve_tool_specs(&self.request.tool_policy, self.deps.tools.as_ref());
        let allow_tool_calls = self.request.tool_policy.mode != ToolMode::None
            && self.state.rounds_left > 0;

        let (force_json, json_schema) = match &self.request.output {
            OutputSpec::Text => (false, None),
            OutputSpec::Json { schema, .. } => (true, schema.clone()),
        };

        LlmInferenceRequest {
            messages: self.state.accumulated.clone(),
            model_alias: self.request.model_policy.preferred.clone(),
            fallbacks: self.request.model_policy.fallbacks.clone(),
            temperature: self.request.model_policy.temperature,
            max_completion_tokens: self.request.model_policy.max_completion_tokens,
            force_json,
            json_schema,
            provider_options: self.request.model_policy.provider_options.clone(),
            tool_specs,
            allow_tool_calls,
        }
    }

    fn account_response(&mut self, response: &AiResponseSummary) {
        if let Some(usage) = &response.usage {
            self.state.usage = merge_usage(&self.state.usage, usage);
        }
    }

    fn check_wallclock_budget(&self) -> Option<LLMContextOutcome> {
        let max = self.request.budget.max_wallclock_ms?;
        let elapsed = now_ms().saturating_sub(self.state.started_at_ms);
        if elapsed > max {
            return Some(LLMContextOutcome::BudgetExhausted {
                which: BudgetKind::Wallclock,
                partial: None,
                usage: self.state.usage.clone(),
            });
        }
        None
    }

    fn check_token_budget(
        &self,
        _response: &AiResponseSummary,
    ) -> Option<LLMContextOutcome> {
        let max = self.request.budget.max_total_tokens?;
        let total = self.state.usage.total_tokens.unwrap_or(0);
        if total > max as u64 {
            return Some(LLMContextOutcome::BudgetExhausted {
                which: BudgetKind::Tokens,
                partial: None,
                usage: self.state.usage.clone(),
            });
        }
        None
    }

    async fn finish_done(&mut self, response: AiResponseSummary) -> LLMContextOutcome {
        let text = response.text.clone().unwrap_or_default();
        let output = match &self.request.output {
            OutputSpec::Text => ContextOutput::Text {
                content: text.clone(),
            },
            OutputSpec::Json { strict, .. } => match serde_json::from_str::<Value>(&text) {
                Ok(value) => ContextOutput::Json { content: value },
                Err(err) => {
                    if *strict {
                        self.deps
                            .worklog
                            .emit(WorkEvent::OutputParseFailed {
                                trace_id: self.request.trace.clone(),
                                error: err.to_string(),
                            })
                            .await;
                        return LLMContextOutcome::Error {
                            error: LLMComputeError::OutputParse(err.to_string()),
                            usage: self.state.usage.clone(),
                        };
                    }
                    // Non-strict: pass the raw text through so the caller can
                    // recover. We still wrap it in `Text` so callers know it
                    // failed to parse.
                    ContextOutput::Text { content: text }
                }
            },
        };

        let trace = ContextRunTrace {
            trace_id: self
                .request
                .trace
                .clone()
                .unwrap_or_default(),
            latency_ms: now_ms().saturating_sub(self.state.started_at_ms),
            tool_trace: std::mem::take(&mut self.tool_trace),
            llm_task_ids: std::mem::take(&mut self.state.llm_task_ids),
        };

        LLMContextOutcome::Done {
            reason: None,
            output,
            usage: self.state.usage.clone(),
            response,
            trace,
        }
    }

    /// Decide what to do with a (already-emitted, already-logged) error.
    /// Returns `Some(outcome)` when the loop should terminate; `None` when
    /// the loop should continue (FeedAsObservation path).
    async fn handle_error(&mut self, class: ErrorClass) -> Option<LLMContextOutcome> {
        match class {
            ErrorClass::Fatal(err) => Some(LLMContextOutcome::Error {
                error: err,
                usage: self.state.usage.clone(),
            }),
            ErrorClass::Recoverable(err) => match self.request.error_policy.mode {
                ErrorMode::Suspend => Some(LLMContextOutcome::WaitInput {
                    reason: format!("recoverable error: {err}"),
                    prompt_to_human: Some(err.to_string()),
                    snapshot: self.snapshot(),
                    deadline_ms: self.request.human_policy.wait_timeout_ms,
                }),
                ErrorMode::FeedAsObservation => {
                    self.state.consecutive_errors =
                        self.state.consecutive_errors.saturating_add(1);
                    let cap = self.request.error_policy.max_consecutive_errors;
                    if cap > 0 && self.state.consecutive_errors > cap {
                        return Some(LLMContextOutcome::WaitInput {
                            reason: "too many consecutive errors".to_string(),
                            prompt_to_human: Some(err.to_string()),
                            snapshot: self.snapshot(),
                            deadline_ms: self.request.human_policy.wait_timeout_ms,
                        });
                    }
                    // The observation message has already been appended to
                    // accumulated by the caller (tool path); for non-tool
                    // recoverable errors we push a system message describing
                    // the error so the next inference can see it.
                    if !matches!(&err, LLMComputeError::ToolFailed { .. }) {
                        self.state.accumulated.push(AiMessage::new(
                            "system".to_string(),
                            format!("error: {err}"),
                        ));
                    }
                    None
                }
            },
        }
    }
}

/// Map a raw `LLMComputeError` to the default `ErrorClass`.
fn classify(err: LLMComputeError) -> ErrorClass {
    match err {
        // Fatal — caller cannot fix mid-flight.
        LLMComputeError::SnapshotCorrupted(_) | LLMComputeError::Cancelled => {
            ErrorClass::Fatal(err)
        }
        // Recoverable by default. Provider-level retry has already run in the
        // adapter; surfacing here means we want the LLM (or scheduler) to
        // decide what to do next.
        _ => ErrorClass::Recoverable(err),
    }
}

fn merge_usage(left: &AiUsage, right: &AiUsage) -> AiUsage {
    fn add(a: Option<u64>, b: Option<u64>) -> Option<u64> {
        match (a, b) {
            (Some(x), Some(y)) => Some(x.saturating_add(y)),
            (Some(x), None) | (None, Some(x)) => Some(x),
            (None, None) => None,
        }
    }
    AiUsage {
        input_tokens: add(left.input_tokens, right.input_tokens),
        output_tokens: add(left.output_tokens, right.output_tokens),
        total_tokens: add(left.total_tokens, right.total_tokens),
    }
}

/// Build the assistant message that records the tool_calls about to be
/// dispatched. We carry both the textual portion of the LLM's reply and a
/// JSON envelope listing the calls — this keeps the transcript readable on
/// re-feed even when the provider drops native tool_call slots.
fn assistant_tool_call_message(text: &str, calls: &[AiToolCall]) -> AiMessage {
    let envelope = serde_json::json!({
        "text": text,
        "tool_calls": calls,
    });
    let content =
        serde_json::to_string(&envelope).unwrap_or_else(|_| text.to_string());
    AiMessage::new("assistant".to_string(), content)
}

/// Build the tool-role message that carries one observation back to the LLM.
fn tool_observation_message(tool_name: &str, observation: &Observation) -> AiMessage {
    let content = serde_json::to_string(&serde_json::json!({
        "tool": tool_name,
        "observation": observation,
    }))
    .unwrap_or_else(|_| match observation {
        Observation::Success { content, .. } => content.to_string(),
        Observation::Error { message, .. } => message.clone(),
        Observation::Pending { call_id } => format!("pending:{call_id}"),
    });
    AiMessage::new("tool".to_string(), content)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
