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

use crate::behavior_loop::{
    CompressBudget, LLMBehaviorResult, StepRecord,
};
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
        // Behavior-mode invariant: parser without renderer is meaningless —
        // the renderer is how sedimented steps become the next inner prompt.
        // Construction failure here would force every caller to handle a
        // Result; instead we panic, mirroring "missing required dep" semantics
        // in the rest of the crate.
        if deps.result_parser.is_some() && deps.step_renderer.is_none() {
            panic!("LLMContext: result_parser requires step_renderer");
        }
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

    fn is_behavior_mode(&self) -> bool {
        self.deps.result_parser.is_some()
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

    /// Run the loop until an outcome is produced. Dispatches to the Behavior
    /// outer loop when a result parser is configured, otherwise drives the
    /// traditional Agent Loop directly.
    pub async fn run(&mut self) -> LLMContextOutcome {
        self.deps
            .worklog
            .emit(WorkEvent::LLMStarted {
                trace_id: self.request.trace.clone(),
                model: self.request.model_policy.preferred.clone(),
            })
            .await;

        let outcome = if self.is_behavior_mode() {
            self.run_behavior().await
        } else {
            self.run_inner().await
        };

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
            behavior_result: None,
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

    // ===================================================================
    // Behavior Loop (outer slim-waist scheduler)
    //
    // The traditional `run_inner` above is reused as a *subroutine* — one
    // step iteration of `run_behavior` starts a fresh traditional LLMContext
    // (with parser/renderer/compressor stripped), runs it to Done, hands the
    // raw response to the configured parser, and sediments the result as a
    // `StepRecord`. `run_inner` itself is untouched.
    // ===================================================================

    async fn run_behavior(&mut self) -> LLMContextOutcome {
        loop {
            if let Some(outcome) = self.check_wallclock_budget() {
                return outcome;
            }

            // 1. Inner run — get one AiResponseSummary, or bubble up an error
            //    / budget / yield translation as the outer outcome.
            let response = match self.run_inner_for_step().await {
                Ok(resp) => resp,
                Err(outer) => return outer,
            };

            // 2. Parser. Failure is folded back as a synthetic error step so
            //    the next inner-run can self-correct (FeedAsObservation
            //    semantics, scoped to the behavior loop).
            let parser = self
                .deps
                .result_parser
                .as_ref()
                .expect("behavior mode requires result_parser");
            let result = match parser.parse(&response) {
                Ok(r) => r,
                Err(err_msg) => {
                    self.deps
                        .worklog
                        .emit(WorkEvent::OutputParseFailed {
                            trace_id: self.request.trace.clone(),
                            error: err_msg.clone(),
                        })
                        .await;
                    let err_step = StepRecord::from_parse_error(&err_msg);
                    self.sediment(err_step);
                    if let Some(outcome) = self
                        .bump_consecutive_errors(LLMComputeError::OutputParse(err_msg))
                        .await
                    {
                        return outcome;
                    }
                    continue;
                }
            };

            // 3. Wrap into a StepRecord. action_result is filled below if we
            //    actually dispatch.
            let mut new_step = StepRecord::from_result(result);

            // 4. Terminal: next_behavior pinned ⇒ finish immediately, action
            //    (if any) is **not** dispatched.
            if new_step.next_behavior.is_some() {
                return self.finish_done_behavior(new_step, response).await;
            }

            // 5. Dispatch the action (if any). No action = natural ReAct
            //    convergence ⇒ also terminal.
            let action = match new_step.action.clone() {
                Some(a) => a,
                None => {
                    return self.finish_done_behavior(new_step, response).await;
                }
            };

            let started = now_ms();
            self.deps
                .worklog
                .emit(WorkEvent::ToolCallPlanned {
                    trace_id: self.request.trace.clone(),
                    tool: action.name.clone(),
                    call_id: action.call_id.clone(),
                })
                .await;
            let observation = self.deps.tools.call_tool(action.clone()).await;
            let duration_ms = now_ms().saturating_sub(started);

            match &observation {
                Observation::Success { .. } => {
                    self.tool_trace.push(ToolExecRecord {
                        tool_name: action.name.clone(),
                        call_id: action.call_id.clone(),
                        ok: true,
                        duration_ms,
                        error: None,
                    });
                    self.deps
                        .worklog
                        .emit(WorkEvent::ToolCallFinished {
                            trace_id: self.request.trace.clone(),
                            tool: action.name.clone(),
                            call_id: action.call_id.clone(),
                            ok: true,
                            duration_ms,
                        })
                        .await;
                    self.state.consecutive_errors = 0;
                }
                Observation::Error { message, .. } => {
                    self.tool_trace.push(ToolExecRecord {
                        tool_name: action.name.clone(),
                        call_id: action.call_id.clone(),
                        ok: false,
                        duration_ms,
                        error: Some(message.clone()),
                    });
                    self.deps
                        .worklog
                        .emit(WorkEvent::ToolCallFailed {
                            trace_id: self.request.trace.clone(),
                            tool: action.name.clone(),
                            call_id: action.call_id.clone(),
                            message: message.clone(),
                        })
                        .await;
                    // Error feeds back into the next step via action_result;
                    // we still count it against the consecutive-error cap.
                    new_step.action_result = Some(observation.clone());
                    self.sediment(new_step);
                    let err = LLMComputeError::ToolFailed {
                        tool: action.name.clone(),
                        call_id: action.call_id.clone(),
                        message: message.clone(),
                    };
                    if let Some(outcome) = self.bump_consecutive_errors(err).await {
                        return outcome;
                    }
                    self.maybe_compress().await;
                    continue;
                }
                Observation::Pending { .. } => {
                    // D7 — Behavior Loop v1 does not support deferred actions
                    // (would require inner yield). Surface as fatal.
                    return LLMContextOutcome::Error {
                        error: LLMComputeError::Internal(
                            "behavior loop: Pending action not supported in v1"
                                .to_string(),
                        ),
                        usage: self.state.usage.clone(),
                    };
                }
            }

            new_step.action_result = Some(observation);
            self.sediment(new_step);
            self.maybe_compress().await;
        }
    }

    /// Run one inner traditional LLMContext for the current behavior step.
    /// Returns the inner `AiResponseSummary` on success, or the outer outcome
    /// to propagate when the inner ended in a non-Done state.
    async fn run_inner_for_step(
        &mut self,
    ) -> Result<AiResponseSummary, LLMContextOutcome> {
        let inner_request = self.build_inner_request();
        let inner_deps = self.deps.clone().into_traditional();

        let mut inner = LLMContext::new(inner_request, inner_deps);
        let outcome = inner.run_inner().await;

        // Always merge whatever the inner managed to spend, even on error —
        // we paid for those tokens.
        let inner_trace = std::mem::take(&mut inner.tool_trace);
        let inner_task_ids = std::mem::take(&mut inner.state.llm_task_ids);
        self.tool_trace.extend(inner_trace);
        self.state.llm_task_ids.extend(inner_task_ids);

        match outcome {
            LLMContextOutcome::Done { response, usage, .. } => {
                self.state.usage = merge_usage(&self.state.usage, &usage);
                Ok(response)
            }
            // D7 — inner yields are not supported in v1.
            LLMContextOutcome::WaitInput { .. }
            | LLMContextOutcome::PendingTool { .. }
            | LLMContextOutcome::ContextLimitReached { .. } => {
                Err(LLMContextOutcome::Error {
                    error: LLMComputeError::Internal(
                        "behavior loop: inner LLMContext yielded; not supported in v1"
                            .to_string(),
                    ),
                    usage: self.state.usage.clone(),
                })
            }
            LLMContextOutcome::Error { error, usage } => {
                self.state.usage = merge_usage(&self.state.usage, &usage);
                Err(LLMContextOutcome::Error {
                    error,
                    usage: self.state.usage.clone(),
                })
            }
            LLMContextOutcome::BudgetExhausted {
                which,
                partial,
                usage,
            } => {
                self.state.usage = merge_usage(&self.state.usage, &usage);
                Err(LLMContextOutcome::BudgetExhausted {
                    which,
                    partial,
                    usage: self.state.usage.clone(),
                })
            }
        }
    }

    /// Assemble the inner request: system + user_init from the outer request,
    /// followed by the rendered step history and the hot `last_step`.
    fn build_inner_request(&self) -> LLMContextRequest {
        let renderer = self
            .deps
            .step_renderer
            .as_ref()
            .expect("behavior mode requires step_renderer");

        let mut messages = self.request.input.clone();
        messages.extend(renderer.render_history(self.state.steps.clone()));
        if let Some(ref last) = self.state.last_step {
            let (assistant_msg, user_msg) = renderer.render(last);
            messages.push(assistant_msg);
            messages.push(user_msg);
        }

        let mut inner = self.request.clone();
        inner.input = messages;
        inner
    }

    /// Push `prev_last_step` (if any) into `steps`, install `new_step` as the
    /// new hot step.
    fn sediment(&mut self, new_step: StepRecord) {
        if let Some(prev) = self.state.last_step.replace(new_step) {
            self.state.steps.push(prev);
        }
    }

    /// Run the optional history compressor. Compression failures are
    /// non-fatal — we log via worklog and continue with the uncompressed
    /// history.
    async fn maybe_compress(&mut self) {
        let Some(compressor) = self.deps.history_compressor.clone() else {
            return;
        };
        let steps_before = self.state.steps.len();
        if steps_before == 0 {
            return;
        }
        let budget = CompressBudget {
            target_steps: None,
            target_tokens: self.request.budget.max_total_tokens,
        };
        // Clone so a compressor failure leaves the live history intact.
        let snapshot = self.state.steps.clone();
        match compressor.compress(snapshot, budget).await {
            Ok(compressed) => {
                let steps_after = compressed.len();
                self.state.steps = compressed;
                self.deps
                    .worklog
                    .emit(WorkEvent::ContextRewritten {
                        trace_id: self.request.trace.clone(),
                        from_messages: steps_before,
                        to_messages: steps_after,
                    })
                    .await;
            }
            Err(_) => {
                // Compressor refused — keep the uncompressed history. Caller
                // observes the same `steps` it had before the attempt.
            }
        }
    }

    /// Outer-loop counterpart to `handle_error`'s FeedAsObservation cap.
    /// Returns `Some(outcome)` when we should terminate (cap exceeded).
    async fn bump_consecutive_errors(
        &mut self,
        err: LLMComputeError,
    ) -> Option<LLMContextOutcome> {
        self.state.consecutive_errors =
            self.state.consecutive_errors.saturating_add(1);
        let cap = self.request.error_policy.max_consecutive_errors;
        if cap > 0 && self.state.consecutive_errors > cap {
            return Some(LLMContextOutcome::Error {
                error: err,
                usage: self.state.usage.clone(),
            });
        }
        None
    }

    /// Terminal path for the Behavior Loop. Sediments `last_step` and emits
    /// `Done.behavior_result = Some(...)`.
    async fn finish_done_behavior(
        &mut self,
        last_step: StepRecord,
        response: AiResponseSummary,
    ) -> LLMContextOutcome {
        let behavior_result = LLMBehaviorResult::from_step(&last_step);
        self.sediment(last_step);

        let output = ContextOutput::Text {
            content: response.text.clone().unwrap_or_default(),
        };
        let trace = ContextRunTrace {
            trace_id: self.request.trace.clone().unwrap_or_default(),
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
            behavior_result: Some(behavior_result),
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
