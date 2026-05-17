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
//! Suspension outcomes (`PendingTool`, `ContextLimitReached`)
//! are *defined* but not actively produced in this first version — they
//! require deferred tools and explicit human-input requests to be wired up.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use buckyos_api::{AiContent, AiMessage, AiResponse, AiRole, AiToolResultContent, AiUsage};
use serde_json::Value;

use crate::behavior_loop::{CompressBudget, LLMBehaviorResult, StepMeta, StepRecord};
use crate::deps::{resolve_tool_specs, LLMContextDeps, LlmInferenceRequest, WorkEvent};
use crate::error::LLMComputeError;
use crate::interrupt::{
    InferenceAbortState, InferenceAbortToken, InferenceAbortTrace, LLMContextInterruptHandle,
};
use crate::observation::{Observation, ToolExecRecord};
use crate::outcome::{BudgetKind, ContextOutput, ContextRunTrace, LLMContextOutcome, ResumeFill};
use crate::request::{ErrorClass, LLMContextRequest, OutputSpec, ToolMode};
use crate::state::{LLMContextSnapshot, LLMContextState};

pub struct LLMContext {
    request: LLMContextRequest,
    state: LLMContextState,
    deps: LLMContextDeps,
    /// Per-run tool audit, flushed into the final `Done` trace.
    tool_trace: Vec<ToolExecRecord>,
    /// Last raw provider response. Carried so we can populate
    /// `Outcome::Done.response`.
    last_response: AiResponse,
    /// Shared abort state (§3.13). `interrupt_handle()` clones it for the
    /// scheduler side; the waist clones a token into every `LlmInferenceRequest`.
    abort: Arc<InferenceAbortState>,
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
            last_response: AiResponse::default(),
            abort: InferenceAbortState::new(),
        }
    }

    /// Hand the scheduler a clonable interrupt handle for this run. Safe to
    /// call before `run()` starts, while it's executing on another task, or
    /// after it returns (the handle just becomes a no-op once the run is
    /// gone). Each `LLMContext` instance has its own abort state — resuming
    /// from a snapshot yields a fresh instance with a fresh handle.
    pub fn interrupt_handle(&self) -> LLMContextInterruptHandle {
        LLMContextInterruptHandle::from_state(self.abort.clone())
    }

    fn is_behavior_mode(&self) -> bool {
        self.deps.result_parser.is_some()
    }

    /// Token to embed in `LlmInferenceRequest`. Cheap to clone.
    fn abort_token(&self) -> InferenceAbortToken {
        InferenceAbortToken::from_state(self.abort.clone())
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
                        .push(tool_observation_message(&call.call.call_id, &obs));
                }
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
            last_response: AiResponse::default(),
            // Fresh abort state on resume: the previous handle is no longer
            // associated with this instance; the scheduler is expected to
            // request a new `interrupt_handle()` from the resumed context if
            // it wants to preempt the next inference.
            abort: InferenceAbortState::new(),
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
            // Capture the pre-inference snapshot (s0): if the inference is
            // preempted by the interrupt handle (§3.13), this is the snapshot
            // we hand back via Outcome::Interrupted. Cheap to construct (it's
            // a clone of request + state, both Cloneable).
            let snapshot_before_inference = self.snapshot();
            let abort_requested_at_ms = if self.abort.is_aborted() {
                Some(now_ms())
            } else {
                None
            };

            // If the scheduler has already requested interrupt before we even
            // entered this round, short-circuit without burning an inference.
            if let Some(requested_at) = abort_requested_at_ms {
                return self.finish_interrupted(
                    snapshot_before_inference,
                    requested_at,
                    /*provider_cancel_supported=*/ true,
                    /*provider_task_ref=*/ None,
                );
            }

            let infer_req = self.build_inference_request();
            let abort_token = infer_req.abort.clone();

            // Race the inference future against the abort signal. Even if the
            // provider adapter ignores `req.abort` and blocks, the
            // `cancelled().await` branch lets the waist drop the inference
            // future and release the scheduler thread immediately.
            let infer_future = self.deps.llm.infer(infer_req);
            let response = tokio::select! {
                biased;
                _ = abort_token.cancelled() => {
                    let requested_at = now_ms();
                    return self.finish_interrupted(
                        snapshot_before_inference,
                        requested_at,
                        /*provider_cancel_supported=*/ true,
                        /*provider_task_ref=*/ None,
                    );
                }
                result = infer_future => match result {
                    Ok(resp) => resp,
                    Err(err) => {
                        // Provider may have observed the abort and surfaced
                        // it as `Cancelled` — that path also funnels into
                        // Interrupted, *not* through ErrorPolicy.
                        if self.abort.is_aborted() {
                            let requested_at = now_ms();
                            return self.finish_interrupted(
                                snapshot_before_inference,
                                requested_at,
                                /*provider_cancel_supported=*/ true,
                                /*provider_task_ref=*/ None,
                            );
                        }
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
                }
            };

            if let Err(err) = response.validate() {
                let err = LLMComputeError::Internal(format!("invalid LLM response: {err}"));
                if let Some(outcome) = self.handle_error(ErrorClass::Fatal(err)).await {
                    return outcome;
                }
                continue;
            }

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

            let tool_calls = response.message.tool_calls();
            let assistant_text = response.message.text_content();

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

            // 5. Push the provider's assistant message into history exactly
            // as returned so content block order and non-text blocks survive.
            self.state.accumulated.push(response.message.clone());

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
                            if let Some(outcome) = self.handle_error(ErrorClass::Fatal(err)).await {
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
                        if let Some(outcome) = self.handle_error(ErrorClass::Fatal(err)).await {
                            return outcome;
                        }
                        had_error = true;
                        break;
                    }
                    Observation::Success { .. } => {
                        self.state
                            .accumulated
                            .push(tool_observation_message(&call.call_id, &observation));
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
                        // Feed the error back so the LLM can self-correct;
                        // the consecutive-error cap in `handle_error` is the
                        // escape valve when self-correction doesn't converge.
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
                            .push(tool_observation_message(&call.call_id, &observation));

                        let err = LLMComputeError::ToolFailed {
                            tool: call.name.clone(),
                            call_id: call.call_id.clone(),
                            message: message.clone(),
                        };
                        if let Some(outcome) = self.handle_error(ErrorClass::Recoverable(err)).await
                        {
                            return outcome;
                        }
                        had_error = true;
                    }
                    Observation::Cancelled { .. } => {
                        // `Cancelled` is only ever produced by the
                        // session-layer interrupt path (injected through
                        // `ResumeFill::ToolResults`); a `ToolManager`
                        // should never emit it from `call_tool`.
                        let err = LLMComputeError::Internal(
                            "tool returned Cancelled inline; only valid via ResumeFill::ToolResults"
                                .to_string(),
                        );
                        if let Some(outcome) = self.handle_error(ErrorClass::Fatal(err)).await {
                            return outcome;
                        }
                        had_error = true;
                        break;
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
        let tool_specs = resolve_tool_specs(&self.request.tool_policy, self.deps.tools.as_ref());
        let allow_tool_calls =
            self.request.tool_policy.mode != ToolMode::None && self.state.rounds_left > 0;

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
            abort: self.abort_token(),
        }
    }

    fn account_response(&mut self, response: &AiResponse) {
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

    fn check_token_budget(&self, _response: &AiResponse) -> Option<LLMContextOutcome> {
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

    /// Build an `Outcome::Interrupted` from the pre-inference snapshot. The
    /// snapshot is `s0` (state *before* the aborted inference), so resume via
    /// `ResumeFill::ResumeFromMidRun` will retry that inference instead of
    /// continuing from half-generated content. `usage` is taken from the
    /// snapshot too — anything spent within this run already shows up there.
    fn finish_interrupted(
        &self,
        snapshot: LLMContextSnapshot,
        requested_at_ms: u64,
        provider_cancel_supported: bool,
        provider_task_ref: Option<String>,
    ) -> LLMContextOutcome {
        let observed_at_ms = now_ms();
        let reason = self
            .abort
            .reason()
            .unwrap_or_else(|| "interrupted".to_string());
        let usage = snapshot.state.usage.clone();
        let trace = InferenceAbortTrace {
            reason: reason.clone(),
            requested_at_ms,
            observed_at_ms,
            provider_cancel_supported,
            provider_task_ref,
        };
        LLMContextOutcome::Interrupted {
            reason,
            usage,
            snapshot,
            abort: trace,
        }
    }

    async fn finish_done(&mut self, response: AiResponse) -> LLMContextOutcome {
        let text = response.message.text_content();
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
            trace_id: self.request.trace.clone().unwrap_or_default(),
            latency_ms: now_ms().saturating_sub(self.state.started_at_ms),
            tool_trace: std::mem::take(&mut self.tool_trace),
            llm_task_ids: std::mem::take(&mut self.state.llm_task_ids),
        };

        self.state.accumulated.push(response.message.clone());

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
            ErrorClass::Recoverable(err) => {
                self.state.consecutive_errors = self.state.consecutive_errors.saturating_add(1);
                let cap = self.request.error_policy.max_consecutive_errors;
                if cap > 0 && self.state.consecutive_errors > cap {
                    // Recoverable but not actually recovering — escalate to
                    // a terminal Error and let the caller decide what to do.
                    return Some(LLMContextOutcome::Error {
                        error: err,
                        usage: self.state.usage.clone(),
                    });
                }
                // The observation message has already been appended to
                // accumulated by the caller (tool path); for non-tool
                // recoverable errors we push a system message describing
                // the error so the next inference can see it.
                if !matches!(&err, LLMComputeError::ToolFailed { .. }) {
                    self.state
                        .accumulated
                        .push(AiMessage::text(AiRole::System, format!("error: {err}")));
                }
                None
            }
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

            let step_started_at_ms = now_ms();

            // 1. Inner run — get one AiResponse, or bubble up an error
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
                    let mut err_step = self
                        .prepare_step(StepRecord::from_parse_error(&err_msg), step_started_at_ms);
                    self.finish_step(&mut err_step);
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
            let mut new_step =
                self.prepare_step(StepRecord::from_result(result), step_started_at_ms);

            // 3a. Honor `forbid_next_behavior`: a fork sub-ctx must terminate
            //     into its own caller, not jump to a sibling behavior. We
            //     scrub the slot before any "terminal: next_behavior pinned"
            //     short-circuit so the run continues toward natural Done.
            //     The original payload is preserved in `assistant_text`, so
            //     the LLM's reasoning isn't lost — only the control-flow
            //     directive is suppressed.
            if self.request.forbid_next_behavior {
                if let Some(violating) = new_step.next_behavior.take() {
                    log::warn!(
                        "behavior_loop: ignoring `<next_behavior>{}</next_behavior>` — forbid_next_behavior flag is set (likely a fork sub-context)",
                        violating
                    );
                }
            }

            // 4. Apply `<report>` side-effects BEFORE actions. self_report
            //    updates LLMContextState.last_report unconditionally (the
            //    snapshot/fork-and-collect contract — see
            //    doc/opendan/Agent Actions.md §3.3). SendMessage-form reports
            //    are stub-emitted via worklog in v2 first cut; real delivery
            //    moves to a standard `send_message` agent_tool later.
            if let Some(report) = new_step.self_report.clone() {
                let chars = report.chars().count();
                self.state.last_report = Some(report);
                self.deps
                    .worklog
                    .emit(WorkEvent::SelfReportSet {
                        trace_id: self.request.trace.clone(),
                        chars,
                    })
                    .await;
            }
            for msg in &new_step.messages_sent {
                self.deps
                    .worklog
                    .emit(WorkEvent::MessageSent {
                        trace_id: self.request.trace.clone(),
                        target: msg.target.clone(),
                        chars: msg.body.chars().count(),
                    })
                    .await;
            }

            // 5. Gate parsed actions through the policy. Mirrors the
            //    traditional loop's `policy.gate_tool_calls` (run_inner step 4)
            //    so that `action_whitelist` / `tool_whitelist` decisions land
            //    on every invocation regardless of which loop dispatched it.
            //    A rejection is folded back as a recoverable error step.
            let actions = match self
                .deps
                .policy
                .gate_tool_calls(&self.request, new_step.actions.clone())
                .await
            {
                Ok(gated) => {
                    new_step.actions = gated.clone();
                    gated
                }
                Err(msg) => {
                    let mut err_step = self
                        .prepare_step(StepRecord::from_policy_rejection(&msg), step_started_at_ms);
                    self.finish_step(&mut err_step);
                    self.sediment(err_step);
                    if let Some(outcome) = self
                        .bump_consecutive_errors(LLMComputeError::PolicyRejected(msg))
                        .await
                    {
                        return outcome;
                    }
                    continue;
                }
            };

            // 6. Dispatch all actions in document order. v2 allows multiple
            //    actions per step via the `<actions>` container. On the first
            //    error we stop dispatching remaining actions (later actions
            //    are often conditional on earlier ones succeeding) and feed
            //    the partial result list back to the LLM via observation.
            let mut action_results: Vec<Observation> = Vec::with_capacity(actions.len());
            let mut error_outcome: Option<LLMContextOutcome> = None;
            let mut error_to_bump: Option<LLMComputeError> = None;

            for action in &actions {
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
                        action_results.push(observation);
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
                        let err = LLMComputeError::ToolFailed {
                            tool: action.name.clone(),
                            call_id: action.call_id.clone(),
                            message: message.clone(),
                        };
                        action_results.push(observation);
                        error_to_bump = Some(err);
                        // Stop the remaining actions — let the LLM see the
                        // error and decide how to proceed.
                        break;
                    }
                    Observation::Pending { .. } => {
                        // D7 — Behavior Loop v1 does not support deferred
                        // actions (would require inner yield). Surface as fatal.
                        error_outcome = Some(LLMContextOutcome::Error {
                            error: LLMComputeError::Internal(
                                "behavior loop: Pending action not supported in v1".to_string(),
                            ),
                            usage: self.state.usage.clone(),
                        });
                        break;
                    }
                    Observation::Cancelled { .. } => {
                        // Same rationale as the traditional-loop arm: Cancelled
                        // must arrive via ResumeFill, never inline.
                        error_outcome = Some(LLMContextOutcome::Error {
                            error: LLMComputeError::Internal(
                                "behavior loop: tool returned Cancelled inline; only valid via ResumeFill::ToolResults"
                                    .to_string(),
                            ),
                            usage: self.state.usage.clone(),
                        });
                        break;
                    }
                }
            }

            if let Some(outcome) = error_outcome {
                return outcome;
            }

            new_step.action_results = action_results;
            if error_to_bump.is_none() && !new_step.actions.is_empty() {
                self.state.consecutive_errors = 0;
            }

            // 7. Terminal cases:
            //    a) `<next_behavior>` was set — finish (after dispatching the
            //       actions above; v2 allows actions + next_behavior in one
            //       step, see doc §2.2).
            //    b) No actions, no report, no message, no next_behavior — a
            //       pure-thought response = natural convergence.
            if new_step.next_behavior.is_some() {
                return self.finish_done_behavior(new_step, response).await;
            }
            let nothing_happened = new_step.actions.is_empty()
                && new_step.self_report.is_none()
                && new_step.messages_sent.is_empty();
            if nothing_happened {
                return self.finish_done_behavior(new_step, response).await;
            }

            // 8. Action error path: sediment the step (so the LLM sees the
            //    failed action_result on the next inference) and bump the
            //    consecutive-error counter.
            if let Some(err) = error_to_bump {
                self.finish_step(&mut new_step);
                self.sediment(new_step);
                if let Some(outcome) = self.bump_consecutive_errors(err).await {
                    return outcome;
                }
                self.maybe_compress().await;
                continue;
            }

            self.finish_step(&mut new_step);
            self.sediment(new_step);
            self.maybe_compress().await;
        }
    }

    /// Run one inner traditional LLMContext for the current behavior step.
    /// Returns the inner `AiResponse` on success, or the outer outcome
    /// to propagate when the inner ended in a non-Done state.
    async fn run_inner_for_step(&mut self) -> Result<AiResponse, LLMContextOutcome> {
        let inner_request = self.build_inner_request();
        let inner_deps = self.deps.clone().into_traditional();

        let mut inner = LLMContext::new(inner_request, inner_deps);
        // Share the outer abort state so a single `interrupt_handle()` on
        // the outer Behavior LLMContext fires through to the in-flight inner
        // inference. Without this, the inner runs would be unreachable by
        // the scheduler's preemption control plane.
        inner.abort = self.abort.clone();
        let outcome = inner.run_inner().await;

        // Always merge whatever the inner managed to spend, even on error —
        // we paid for those tokens.
        let inner_trace = std::mem::take(&mut inner.tool_trace);
        let inner_task_ids = std::mem::take(&mut inner.state.llm_task_ids);
        self.tool_trace.extend(inner_trace);
        self.state.llm_task_ids.extend(inner_task_ids);

        match outcome {
            LLMContextOutcome::Done {
                response, usage, ..
            } => {
                self.state.usage = merge_usage(&self.state.usage, &usage);
                Ok(response)
            }
            // D7 — inner cooperative yields are not supported in v1.
            LLMContextOutcome::PendingTool { .. }
            | LLMContextOutcome::ContextLimitReached { .. } => Err(LLMContextOutcome::Error {
                error: LLMComputeError::Internal(
                    "behavior loop: inner LLMContext yielded; not supported in v1".to_string(),
                ),
                usage: self.state.usage.clone(),
            }),
            // Inference interrupt propagates straight through — it is a
            // preemptive control-plane event, not an error. The inner
            // snapshot's s0 represents the inner LLMContext's pre-inference
            // state; the outer Behavior Loop returns it verbatim so the
            // scheduler can resume the same way it would for any other
            // Interrupted outcome.
            LLMContextOutcome::Interrupted {
                reason,
                usage,
                snapshot,
                abort,
            } => {
                self.state.usage = merge_usage(&self.state.usage, &usage);
                Err(LLMContextOutcome::Interrupted {
                    reason,
                    usage: self.state.usage.clone(),
                    snapshot,
                    abort,
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

        let current_behavior = self.request.behavior_name.as_str();
        let mut messages = self.request.input.clone();
        messages.extend(renderer.render_history(
            self.state.steps.clone(),
            current_behavior,
            self.state.history_summaries.clone(),
        ));
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

    fn prepare_step(&mut self, mut step: StepRecord, started_at_ms: u64) -> StepRecord {
        let step_index = self.state.next_step_index;
        self.state.next_step_index = self.state.next_step_index.saturating_add(1);
        step.meta = StepMeta {
            behavior_name: self.request.behavior_name.clone(),
            step_index,
            started_at_ms,
            ended_at_ms: None,
            compression_level: Default::default(),
        };
        step
    }

    fn finish_step(&self, step: &mut StepRecord) {
        step.meta.ended_at_ms = Some(now_ms());
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
                let steps_after = compressed.steps.len();
                self.state.steps = compressed.steps;
                self.state.history_summaries.extend(compressed.summaries);
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
    async fn bump_consecutive_errors(&mut self, err: LLMComputeError) -> Option<LLMContextOutcome> {
        self.state.consecutive_errors = self.state.consecutive_errors.saturating_add(1);
        let cap = self.request.error_policy.max_consecutive_errors;
        if cap > 0 && self.state.consecutive_errors > cap {
            return Some(LLMContextOutcome::Error {
                error: err,
                usage: self.state.usage.clone(),
            });
        }
        None
    }

    async fn finish_done_behavior(
        &mut self,
        mut last_step: StepRecord,
        response: AiResponse,
    ) -> LLMContextOutcome {
        self.finish_step(&mut last_step);
        let behavior_result = LLMBehaviorResult::from_step(&last_step);
        if let Some(prev) = self.state.last_step.take() {
            self.state.steps.push(prev);
        }
        self.state.steps.push(last_step);

        let output = ContextOutput::Text {
            content: response.message.text_content(),
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

/// Build the tool-role message that carries one observation back to the LLM.
/// Keyed by `call_id` so providers can wire it to the originating ToolUse.
fn tool_observation_message(call_id: &str, observation: &Observation) -> AiMessage {
    let (content_text, is_error) = match observation {
        Observation::Success { content, .. } => {
            let text = if let Some(s) = content.as_str() {
                s.to_string()
            } else {
                serde_json::to_string(content).unwrap_or_else(|_| "{}".to_string())
            };
            (text, false)
        }
        Observation::Error { message, .. } => (message.clone(), true),
        Observation::Pending { call_id: cid } => (format!("pending:{cid}"), true),
        Observation::Cancelled { reason, .. } => {
            // `is_error=false` — the call did not fail, it was interrupted.
            // The text marker lets a content-aware renderer / the LLM tell
            // cancellations apart from successful outputs.
            (format!("[cancelled] {reason}"), false)
        }
    };
    AiMessage::new(
        AiRole::Tool,
        vec![AiContent::ToolResult {
            call_id: call_id.to_string(),
            content: vec![AiToolResultContent::text(content_text)],
            is_error,
        }],
    )
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
