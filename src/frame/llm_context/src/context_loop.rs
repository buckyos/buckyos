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
//! - terminate with `Error` on Fatal errors; LLM-correctable errors are
//!   fed back per `ErrorPolicy`
//!
//! Error ownership (see `LLMComputeError::source`):
//! - Provider failures end the run. The adapter has already exhausted its
//!   own tolerance; the waist never re-infers on its own and never feeds
//!   infrastructure errors to the LLM.
//! - Output-protocol violations, tool business failures and policy
//!   rejections are fed back for bounded self-correction.
//! - Runtime failures (checkpoint, tool dispatch) stop further side effects
//!   immediately, keep the results already obtained, and hand the run back
//!   to the runtime with the in-memory snapshot intact.
//!
//! Suspension outcomes (`PendingTool`, `ContextLimitReached`)
//! are *defined* but not actively produced in this first version — they
//! require deferred tools and explicit human-input requests to be wired up.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use buckyos_api::{
    AiContent, AiCost, AiMessage, AiResponse, AiRole, AiToolCall, AiToolResultContent, AiUsage,
};
use serde_json::Value;

use crate::behavior_loop::{is_terminal_next_behavior, LLMBehaviorResult, StepMeta, StepRecord};
use crate::deps::{resolve_tool_specs, LLMContextDeps, LlmInferenceRequest, WorkEvent};
use crate::error::{CheckpointStage, LLMComputeError};
use crate::interrupt::{
    InferenceAbortState, InferenceAbortToken, InferenceAbortTrace, LLMContextInterruptHandle,
};
use crate::observation::{Observation, ToolExecRecord, ToolExecStatus};
use crate::outcome::{BudgetKind, ContextOutput, ContextRunTrace, LLMContextOutcome, ResumeFill};
use crate::request::{ErrorClass, LLMContextRequest, OutputSpec, ToolMode};
use crate::state::{LLMContextSnapshot, LLMContextState};

pub struct LLMContext {
    request: LLMContextRequest,
    state: LLMContextState,
    deps: LLMContextDeps,
    /// Per-run tool audit, flushed into the final `Done` / `Error` trace.
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
    ///
    /// Besides matching `fill` against the suspension state, the accumulated
    /// history must not contain unanswered provider tool calls: a run that
    /// was cut short mid-batch always pairs every `tool_use` with a
    /// `tool_result` (possibly `Observation::Unresolved`), so an unpaired
    /// call means the snapshot was assembled incorrectly.
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

        let unanswered = unanswered_tool_calls(&state.accumulated);
        if !unanswered.is_empty() {
            return Err(LLMComputeError::SnapshotCorrupted(format!(
                "accumulated history has unanswered tool calls: {}",
                unanswered.join(", ")
            )));
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

            // 1. Pre-inference snapshot (s0). It is the critical checkpoint
            // handed to the TurnHook (§3.12) and the state returned verbatim
            // when the inference is preempted (§3.13). Cheap to construct.
            let snapshot_before_inference = self.snapshot();
            if let Some(hook) = &self.deps.turn_hook {
                if let Err(message) = hook.before_inference(&snapshot_before_inference) {
                    self.deps
                        .worklog
                        .emit(WorkEvent::CheckpointFailed {
                            trace_id: self.request.trace.clone(),
                            stage: CheckpointStage::BeforeInference,
                            error: message.clone(),
                        })
                        .await;
                    // Nothing was paid for and nothing executed: the state
                    // is still s0, so the runtime may retry the checkpoint
                    // and resume from here.
                    return self.finish_error(LLMComputeError::Checkpoint {
                        stage: CheckpointStage::BeforeInference,
                        message,
                    });
                }
            }
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
                        // Whatever the adapter returned, the LLM cannot fix
                        // an inference that never produced a response. The
                        // adapter's tolerance is exhausted; the scheduler
                        // decides whether to run again.
                        return self.finish_error(err);
                    }
                }
            };

            if let Err(err) = response.validate() {
                return self.finish_error(LLMComputeError::Internal(format!(
                    "invalid LLM response: {err}"
                )));
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

            // 2. No tool calls ⇒ finish (or self-correct a strict JSON output)
            if tool_calls.is_empty() || self.request.tool_policy.mode == ToolMode::None {
                match self.finish_done(response).await {
                    Some(outcome) => return outcome,
                    None => continue,
                }
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
                let reason = format!(
                    "tool calls {} exceed max_calls_per_round {}",
                    tool_calls.len(),
                    self.request.tool_policy.max_calls_per_round
                );
                self.reject_tool_round(&response.message, &tool_calls, &reason);
                if let Some(outcome) = self
                    .handle_error(LLMComputeError::PolicyRejected(reason).into())
                    .await
                {
                    return outcome;
                }
                continue;
            }

            // 4. Policy gate. A rejection is answered on every call of the
            // round so the transcript stays paired, then fed back.
            let gated = match self
                .deps
                .policy
                .gate_tool_calls(&self.request, tool_calls.clone())
                .await
            {
                Ok(calls) => calls,
                Err(msg) => {
                    self.reject_tool_round(
                        &response.message,
                        &tool_calls,
                        &format!("policy rejected: {msg}"),
                    );
                    if let Some(outcome) = self
                        .handle_error(LLMComputeError::PolicyRejected(msg).into())
                        .await
                    {
                        return outcome;
                    }
                    continue;
                }
            };

            // 5. Push the provider's assistant message into history exactly
            // as returned so content block order and non-text blocks survive.
            self.state.accumulated.push(response.message.clone());

            // 6. Execute calls (serial in v1). Business errors do not stop
            // the batch — the LLM sees every result and corrects the round
            // as a whole. Infrastructure failures stop dispatch immediately;
            // the calls that never ran are answered as `Unresolved` so the
            // transcript and the trace both show what happened.
            let mut round_error: Option<LLMComputeError> = None;
            let mut fatal: Option<LLMComputeError> = None;
            for idx in 0..gated.len() {
                let call = gated[idx].clone();
                let started = now_ms();
                self.deps
                    .worklog
                    .emit(WorkEvent::ToolCallPlanned {
                        trace_id: self.request.trace.clone(),
                        tool: call.name.clone(),
                        call_id: call.call_id.clone(),
                        args: call.args.clone(),
                    })
                    .await;

                let dispatched = self.deps.tools.call_tool(call.clone()).await;
                let duration_ms = now_ms().saturating_sub(started);

                let observation = match dispatched {
                    Ok(observation) => observation,
                    Err(dispatch) => {
                        self.deps
                            .worklog
                            .emit(WorkEvent::ToolDispatchFailed {
                                trace_id: self.request.trace.clone(),
                                tool: call.name.clone(),
                                call_id: call.call_id.clone(),
                                message: dispatch.message.clone(),
                                effect_unknown: dispatch.effect_unknown,
                            })
                            .await;
                        let status = if dispatch.effect_unknown {
                            ToolExecStatus::Unknown
                        } else {
                            ToolExecStatus::NotExecuted
                        };
                        self.record_tool(&call, status, duration_ms, Some(dispatch.message.clone()));
                        let unresolved = Observation::Unresolved {
                            call_id: call.call_id.clone(),
                            reason: dispatch.message.clone(),
                            effect_unknown: dispatch.effect_unknown,
                        };
                        self.state
                            .accumulated
                            .push(tool_observation_message(&call.call_id, &unresolved));
                        self.abort_tool_batch(
                            &gated[idx + 1..],
                            "not executed: dispatch of an earlier call in this round failed",
                        );
                        fatal = Some(LLMComputeError::ToolRuntime {
                            tool: call.name.clone(),
                            call_id: call.call_id.clone(),
                            message: dispatch.message,
                            effect_unknown: dispatch.effect_unknown,
                        });
                        break;
                    }
                };

                match &observation {
                    Observation::Success { .. } => {
                        self.state
                            .accumulated
                            .push(tool_observation_message(&call.call_id, &observation));
                        self.record_tool(&call, ToolExecStatus::Succeeded, duration_ms, None);
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
                        self.state
                            .accumulated
                            .push(tool_observation_message(&call.call_id, &observation));
                        self.record_tool(
                            &call,
                            ToolExecStatus::Failed,
                            duration_ms,
                            Some(message.clone()),
                        );
                        self.deps
                            .worklog
                            .emit(WorkEvent::ToolCallFailed {
                                trace_id: self.request.trace.clone(),
                                tool: call.name.clone(),
                                call_id: call.call_id.clone(),
                                message: message.clone(),
                            })
                            .await;
                        if round_error.is_none() {
                            round_error = Some(LLMComputeError::ToolFailed {
                                tool: call.name.clone(),
                                call_id: call.call_id.clone(),
                                message: message.clone(),
                            });
                        }
                    }
                    Observation::Pending { .. } => {
                        // Deferred tools are declared but not implemented in
                        // the loop; an adapter returning Pending violates the
                        // `allow_deferred=false` contract or hits the
                        // unimplemented path. Either way the call started.
                        let message = if self.request.tool_policy.allow_deferred {
                            "deferred tool path not yet implemented".to_string()
                        } else {
                            "tool returned Pending but allow_deferred=false".to_string()
                        };
                        self.abort_started_call(&gated, idx, &message, duration_ms);
                        fatal = Some(LLMComputeError::Internal(message));
                        break;
                    }
                    Observation::Cancelled { .. } | Observation::Unresolved { .. } => {
                        // Both variants are produced by the session layer /
                        // the waist itself, never by a `ToolManager`.
                        let message =
                            "tool returned Cancelled/Unresolved inline; only valid via ResumeFill::ToolResults"
                                .to_string();
                        self.abort_started_call(&gated, idx, &message, duration_ms);
                        fatal = Some(LLMComputeError::Internal(message));
                        break;
                    }
                }
            }

            if let Some(err) = fatal {
                return self.finish_error(err);
            }
            match round_error {
                Some(err) => {
                    // Observations are already in the transcript; count one
                    // failed round regardless of how many calls failed.
                    if let Some(outcome) = self.bump_consecutive_errors(err) {
                        return outcome;
                    }
                }
                None => self.state.consecutive_errors = 0,
            }

            // 7. Round consumed.
            self.state.rounds_left = self.state.rounds_left.saturating_sub(1);
        }
    }

    fn record_tool(
        &mut self,
        call: &AiToolCall,
        status: ToolExecStatus,
        duration_ms: u64,
        error: Option<String>,
    ) {
        self.tool_trace.push(ToolExecRecord {
            tool_name: call.name.clone(),
            call_id: call.call_id.clone(),
            status,
            duration_ms,
            error,
        });
    }

    /// Answer every call in `remaining` with `Unresolved { effect_unknown:
    /// false }` and record it as `NotExecuted`, keeping the transcript paired
    /// after a batch is cut short.
    fn abort_tool_batch(&mut self, remaining: &[AiToolCall], reason: &str) {
        for call in remaining {
            self.record_tool(
                call,
                ToolExecStatus::NotExecuted,
                0,
                Some(reason.to_string()),
            );
            let unresolved = Observation::Unresolved {
                call_id: call.call_id.clone(),
                reason: reason.to_string(),
                effect_unknown: false,
            };
            self.state
                .accumulated
                .push(tool_observation_message(&call.call_id, &unresolved));
        }
    }

    /// The call at `idx` started but the adapter broke the observation
    /// contract: mark its effect unknown and abort the rest of the batch.
    fn abort_started_call(
        &mut self,
        gated: &[AiToolCall],
        idx: usize,
        message: &str,
        duration_ms: u64,
    ) {
        let call = gated[idx].clone();
        self.record_tool(
            &call,
            ToolExecStatus::Unknown,
            duration_ms,
            Some(message.to_string()),
        );
        let unresolved = Observation::Unresolved {
            call_id: call.call_id.clone(),
            reason: message.to_string(),
            effect_unknown: true,
        };
        self.state
            .accumulated
            .push(tool_observation_message(&call.call_id, &unresolved));
        self.abort_tool_batch(
            &gated[idx + 1..],
            "not executed: an earlier call in this round broke the observation contract",
        );
    }

    /// Push the assistant message that requested `calls` and answer each
    /// call with an error observation carrying `reason`, so the next
    /// inference sees a paired transcript and can correct the round.
    fn reject_tool_round(&mut self, message: &AiMessage, calls: &[AiToolCall], reason: &str) {
        self.state.accumulated.push(message.clone());
        for call in calls {
            self.record_tool(
                call,
                ToolExecStatus::NotExecuted,
                0,
                Some(reason.to_string()),
            );
            let rejected = Observation::Error {
                call_id: call.call_id.clone(),
                message: reason.to_string(),
                tool_result: None,
            };
            self.state
                .accumulated
                .push(tool_observation_message(&call.call_id, &rejected));
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
            trace_id: self.request.trace.clone(),
            messages: self.state.accumulated.clone(),
            model_alias: self.request.model_policy.preferred.clone(),
            fallbacks: self.request.model_policy.fallbacks.clone(),
            temperature: self.request.model_policy.temperature,
            max_completion_tokens: self.request.model_policy.max_completion_tokens,
            force_json,
            json_schema,
            provider_options: self.request.model_policy.provider_options.clone(),
            disable_capabilities: self.request.tool_policy.disable_capabilities.clone(),
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

    fn take_trace(&mut self) -> ContextRunTrace {
        ContextRunTrace {
            trace_id: self.request.trace.clone().unwrap_or_default(),
            latency_ms: now_ms().saturating_sub(self.state.started_at_ms),
            tool_trace: std::mem::take(&mut self.tool_trace),
            llm_task_ids: std::mem::take(&mut self.state.llm_task_ids),
        }
    }

    /// Terminal `Error` outcome carrying the audit trace of this run.
    fn finish_error(&mut self, error: LLMComputeError) -> LLMContextOutcome {
        LLMContextOutcome::Error {
            error,
            usage: self.state.usage.clone(),
            trace: self.take_trace(),
        }
    }

    /// Returns `None` when a strict JSON output failed to parse and the loop
    /// should run another inference with the diagnostic fed back.
    async fn finish_done(&mut self, response: AiResponse) -> Option<LLMContextOutcome> {
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
                        // Keep the failed output in the transcript and add
                        // an actionable diagnostic; bounded by ErrorPolicy.
                        self.state.accumulated.push(response.message.clone());
                        self.state.accumulated.push(AiMessage::text(
                            AiRole::User,
                            format!(
                                "error: llm output parse failed: {err}\nReply again with exactly one valid JSON value and nothing else."
                            ),
                        ));
                        return self.bump_consecutive_errors(LLMComputeError::OutputParse(
                            err.to_string(),
                        ));
                    }
                    // Non-strict: pass the raw text through so the caller can
                    // recover. We still wrap it in `Text` so callers know it
                    // failed to parse.
                    ContextOutput::Text { content: text }
                }
            },
        };

        let trace = self.take_trace();

        self.state.accumulated.push(response.message.clone());

        Some(LLMContextOutcome::Done {
            reason: None,
            output,
            usage: self.state.usage.clone(),
            response,
            trace,
            behavior_result: None,
        })
    }

    /// Decide what to do with a (already-emitted, already-logged) error whose
    /// feedback — if any — has already been appended to the history.
    /// Returns `Some(outcome)` when the loop should terminate; `None` when
    /// the loop should continue with the next inference.
    async fn handle_error(&mut self, class: ErrorClass) -> Option<LLMContextOutcome> {
        match class {
            ErrorClass::Fatal(err) => Some(self.finish_error(err)),
            ErrorClass::Recoverable(err) => self.bump_consecutive_errors(err),
        }
    }

    // ===================================================================
    // Behavior Loop (outer slim-waist scheduler)
    //
    // The traditional `run_inner` above is reused as a *subroutine* — one
    // step iteration of `run_behavior` starts a fresh traditional LLMContext
    // (with parser/renderer stripped), runs it to Done, hands the
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
                    err_step.assistant_text = response.message.text_content();
                    err_step.assistant_message = Some(response.message.clone());
                    self.finish_step(&mut err_step);
                    self.sediment(err_step);
                    if let Some(outcome) =
                        self.bump_consecutive_errors(LLMComputeError::OutputParse(err_msg))
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
            new_step.assistant_message = Some(response.message.clone());

            if !new_step.actions.is_empty() && self.state.rounds_left == 0 {
                return LLMContextOutcome::BudgetExhausted {
                    which: BudgetKind::ToolRounds,
                    partial: Some(ContextOutput::Text {
                        content: response.message.text_content(),
                    }),
                    usage: self.state.usage.clone(),
                };
            }

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
            //    doc/opendan/Agent Actions.md §3.3). `<sendmsg>` records
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

            // 5. Gate parsed XML actions through the action policy. Mirrors
            //    the traditional loop's provider-tool policy gate while
            //    preserving the surface that produced the invocation.
            //    A rejection is folded back as a recoverable error step.
            let had_action_side_effects_before_gate =
                !new_step.actions.is_empty() || !new_step.messages_sent.is_empty();
            let actions = match self
                .deps
                .policy
                .gate_action_calls(&self.request, new_step.actions.clone())
                .await
            {
                Ok(gated) => {
                    new_step.actions = gated.clone();
                    gated
                }
                Err(msg) => {
                    let mut err_step = self
                        .prepare_step(StepRecord::from_policy_rejection(&msg), step_started_at_ms);
                    err_step.assistant_text = response.message.text_content();
                    err_step.assistant_message = Some(response.message.clone());
                    self.finish_step(&mut err_step);
                    self.sediment(err_step);
                    if let Some(outcome) =
                        self.bump_consecutive_errors(LLMComputeError::PolicyRejected(msg))
                    {
                        return outcome;
                    }
                    continue;
                }
            };

            // 5b. `<next_behavior>` on a step that also carries action side
            //     effects. A *jump target* is still scrubbed here: the target
            //     behavior must observe this step's results before the
            //     behavior changes.
            //
            //     A *terminal* directive (`END`) is deliberately kept. It
            //     declares that the current intent is finished, so no later
            //     inference in this behavior would act on those results
            //     anyway. Dropping it was this loop's most expensive bug: a
            //     model that closes every step with
            //     `<actions>…</actions><next_behavior>END</next_behavior>`
            //     never converged and re-declared the same intent until the
            //     Provider rejected the request. The actions are still
            //     dispatched below — the model did ask for them — and `END`
            //     then decides the step's outcome.
            let terminal_declared = new_step
                .next_behavior
                .as_deref()
                .is_some_and(is_terminal_next_behavior);
            if had_action_side_effects_before_gate && !terminal_declared {
                if let Some(violating) = new_step.next_behavior.take() {
                    log::warn!(
                        "behavior_loop: ignoring `<next_behavior>{violating}</next_behavior>` because actions are present; action results must be observed before changing behavior"
                    );
                }
            }

            // 6. Dispatch all actions in document order. v2 allows multiple
            //    actions per step via the `<actions>` container. On the first
            //    business error we stop dispatching (later actions are often
            //    conditional on earlier ones succeeding) and feed the full
            //    result list — including the skipped actions — back to the
            //    LLM. An infrastructure failure also stops dispatch, but the
            //    step is sedimented as-is and the run ends for the runtime
            //    to handle; nothing is fed back for self-correction.
            let mut action_results: Vec<Observation> = Vec::with_capacity(actions.len());
            let mut fatal: Option<LLMComputeError> = None;
            let mut error_to_bump: Option<LLMComputeError> = None;

            if !actions.is_empty() {
                self.state.rounds_left = self.state.rounds_left.saturating_sub(1);
            }
            for idx in 0..actions.len() {
                let action = actions[idx].clone();
                let started = now_ms();
                self.deps
                    .worklog
                    .emit(WorkEvent::ToolCallPlanned {
                        trace_id: self.request.trace.clone(),
                        tool: action.name.clone(),
                        call_id: action.call_id.clone(),
                        args: action.args.clone(),
                    })
                    .await;
                let dispatched = self.deps.tools.call_tool(action.clone()).await;
                let duration_ms = now_ms().saturating_sub(started);

                let observation = match dispatched {
                    Ok(observation) => observation,
                    Err(dispatch) => {
                        self.deps
                            .worklog
                            .emit(WorkEvent::ToolDispatchFailed {
                                trace_id: self.request.trace.clone(),
                                tool: action.name.clone(),
                                call_id: action.call_id.clone(),
                                message: dispatch.message.clone(),
                                effect_unknown: dispatch.effect_unknown,
                            })
                            .await;
                        let status = if dispatch.effect_unknown {
                            ToolExecStatus::Unknown
                        } else {
                            ToolExecStatus::NotExecuted
                        };
                        self.record_tool(&action, status, duration_ms, Some(dispatch.message.clone()));
                        action_results.push(Observation::Unresolved {
                            call_id: action.call_id.clone(),
                            reason: dispatch.message.clone(),
                            effect_unknown: dispatch.effect_unknown,
                        });
                        action_results.extend(self.skip_actions(
                            &actions[idx + 1..],
                            "not executed: dispatch of an earlier action in this step failed",
                        ));
                        fatal = Some(LLMComputeError::ToolRuntime {
                            tool: action.name.clone(),
                            call_id: action.call_id.clone(),
                            message: dispatch.message,
                            effect_unknown: dispatch.effect_unknown,
                        });
                        break;
                    }
                };

                match &observation {
                    Observation::Success { .. } => {
                        self.record_tool(&action, ToolExecStatus::Succeeded, duration_ms, None);
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
                        self.record_tool(
                            &action,
                            ToolExecStatus::Failed,
                            duration_ms,
                            Some(message.clone()),
                        );
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
                        action_results.extend(self.skip_actions(
                            &actions[idx + 1..],
                            "not executed: an earlier action in this step failed",
                        ));
                        error_to_bump = Some(err);
                        break;
                    }
                    Observation::Pending { .. } => {
                        // D7 — Behavior Loop v1 does not support deferred
                        // actions (would require inner yield). The call did
                        // start, so its effect is unknown.
                        let message =
                            "behavior loop: Pending action not supported in v1".to_string();
                        self.record_tool(
                            &action,
                            ToolExecStatus::Unknown,
                            duration_ms,
                            Some(message.clone()),
                        );
                        action_results.push(Observation::Unresolved {
                            call_id: action.call_id.clone(),
                            reason: message.clone(),
                            effect_unknown: true,
                        });
                        action_results.extend(self.skip_actions(
                            &actions[idx + 1..],
                            "not executed: an earlier action in this step broke the observation contract",
                        ));
                        fatal = Some(LLMComputeError::Internal(message));
                        break;
                    }
                    Observation::Cancelled { .. } | Observation::Unresolved { .. } => {
                        // Same rationale as the traditional-loop arm: these
                        // variants must arrive via ResumeFill / the waist,
                        // never inline from a ToolManager.
                        let message =
                            "behavior loop: tool returned Cancelled/Unresolved inline; only valid via ResumeFill::ToolResults"
                                .to_string();
                        self.record_tool(
                            &action,
                            ToolExecStatus::Unknown,
                            duration_ms,
                            Some(message.clone()),
                        );
                        action_results.push(Observation::Unresolved {
                            call_id: action.call_id.clone(),
                            reason: message.clone(),
                            effect_unknown: true,
                        });
                        action_results.extend(self.skip_actions(
                            &actions[idx + 1..],
                            "not executed: an earlier action in this step broke the observation contract",
                        ));
                        fatal = Some(LLMComputeError::Internal(message));
                        break;
                    }
                }
            }

            new_step.action_results = action_results;

            if let Some(err) = fatal {
                // Sediment the truthful partial step so the snapshot keeps
                // what ran, what failed and what never started.
                self.finish_step(&mut new_step);
                self.sediment(new_step);
                return self.finish_error(err);
            }
            if error_to_bump.is_none() {
                self.state.consecutive_errors = 0;
            }

            // 6b. A terminal END that shared its step with actions is honoured
            //     only if every dispatched action succeeded. A failed action
            //     still has to be fed back (the error path below sediments it
            //     and bumps the consecutive-error counter), so the directive is
            //     released for the model to re-declare once it has seen the
            //     failure — released with a log line, never dropped silently.
            if terminal_declared && error_to_bump.is_some() {
                if let Some(deferred) = new_step.next_behavior.take() {
                    log::warn!(
                        "behavior_loop: deferring `<next_behavior>{deferred}</next_behavior>` — a dispatched action failed, its result must be observed before this behavior can end"
                    );
                }
            }

            // 7. Terminal cases:
            //    a) `<next_behavior>` is in force for this step. An action-free
            //       step always ends here. A step that carried actions only
            //       reaches this point with the terminal END (5b suppressed
            //       every jump target, 6b released a failed END), which is
            //       precisely the case the model must not be second-guessed
            //       about: it already ran its actions, nothing later in this
            //       behavior would look at their results, and re-declaring END
            //       would be the only way out of an otherwise endless loop.
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
                if self.apply_step_result_hook(&mut new_step).await {
                    return self.finish_done_behavior(new_step, response).await;
                }
                self.sediment(new_step);
                if let Some(outcome) = self.bump_consecutive_errors(err) {
                    return outcome;
                }
                continue;
            }

            self.finish_step(&mut new_step);
            if self.apply_step_result_hook(&mut new_step).await {
                return self.finish_done_behavior(new_step, response).await;
            }
            self.sediment(new_step);
        }
    }

    /// Record every action in `remaining` as `NotExecuted` and return the
    /// matching `Unresolved` observations to append to the step.
    fn skip_actions(&mut self, remaining: &[AiToolCall], reason: &str) -> Vec<Observation> {
        remaining
            .iter()
            .map(|action| {
                self.record_tool(
                    action,
                    ToolExecStatus::NotExecuted,
                    0,
                    Some(reason.to_string()),
                );
                Observation::Unresolved {
                    call_id: action.call_id.clone(),
                    reason: reason.to_string(),
                    effect_unknown: false,
                }
            })
            .collect()
    }

    /// Run one inner traditional LLMContext for the current behavior step.
    /// Returns the inner `AiResponse` on success, or the outer outcome
    /// to propagate when the inner ended in a non-Done state.
    ///
    /// The inner instance is seeded with the outer usage, start time and
    /// consecutive-error count, and hands them back afterwards, so
    /// recreating the inner context per step cannot bypass the budget or
    /// the self-correction cap.
    async fn run_inner_for_step(&mut self) -> Result<AiResponse, LLMContextOutcome> {
        let inner_request = self.build_inner_request();
        let inner_deps = self.deps.clone().into_traditional();

        let mut inner = LLMContext::new(inner_request, inner_deps);
        // Share the outer abort state so a single `interrupt_handle()` on
        // the outer Behavior LLMContext fires through to the in-flight inner
        // inference. Without this, the inner runs would be unreachable by
        // the scheduler's preemption control plane.
        inner.abort = self.abort.clone();
        inner.state.usage = self.state.usage.clone();
        inner.state.started_at_ms = self.state.started_at_ms;
        inner.state.consecutive_errors = self.state.consecutive_errors;
        inner.state.rounds_left = self.state.rounds_left;
        let outcome = inner.run_inner().await;

        // Always take back whatever the inner spent and recorded, even on
        // error — we paid for those tokens and ran those tools.
        self.tool_trace.append(&mut inner.tool_trace);
        self.state.llm_task_ids.append(&mut inner.state.llm_task_ids);
        self.state.consecutive_errors = inner.state.consecutive_errors;
        self.state.rounds_left = inner.state.rounds_left;
        self.state.usage = inner.state.usage.clone();

        match outcome {
            LLMContextOutcome::Done {
                response, trace, ..
            } => {
                self.absorb_trace(trace);
                self.clear_behavior_turn_tail();
                Ok(response)
            }
            // D7 — inner cooperative yields are not supported in v1.
            LLMContextOutcome::PendingTool { .. }
            | LLMContextOutcome::ContextLimitReached { .. } => {
                Err(self.finish_error(LLMComputeError::Internal(
                    "behavior loop: inner LLMContext yielded; not supported in v1".to_string(),
                )))
            }
            // Inference interrupt propagates straight through — it is a
            // preemptive control-plane event, not an error. The inner
            // snapshot's s0 represents the inner LLMContext's pre-inference
            // state; the outer Behavior Loop returns it verbatim so the
            // scheduler can resume the same way it would for any other
            // Interrupted outcome.
            LLMContextOutcome::Interrupted {
                reason,
                snapshot,
                abort,
                ..
            } => Err(LLMContextOutcome::Interrupted {
                reason,
                usage: self.state.usage.clone(),
                snapshot,
                abort,
            }),
            LLMContextOutcome::Error { error, trace, .. } => {
                self.absorb_trace(trace);
                Err(self.finish_error(error))
            }
            LLMContextOutcome::BudgetExhausted { which, partial, .. } => {
                Err(LLMContextOutcome::BudgetExhausted {
                    which,
                    partial,
                    usage: self.state.usage.clone(),
                })
            }
        }
    }

    fn absorb_trace(&mut self, trace: ContextRunTrace) {
        self.tool_trace.extend(trace.tool_trace);
        self.state.llm_task_ids.extend(trace.llm_task_ids);
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
            self.state.history_inputs.clone(),
        ));
        if let Some(ref last) = self.state.last_step {
            let (assistant_msg, user_msg) = renderer.render(last);
            messages.push(assistant_msg);
            messages.push(user_msg);
        }
        messages.extend(self.behavior_turn_tail());

        let mut inner = self.request.clone();
        inner.input = messages;
        inner
    }

    fn behavior_turn_tail(&self) -> Vec<AiMessage> {
        let prefix_len = self.request.input.len();
        if self.state.accumulated.len() <= prefix_len {
            return Vec::new();
        }
        if !self
            .request
            .input
            .iter()
            .zip(self.state.accumulated.iter())
            .all(|(a, b)| a == b)
        {
            return Vec::new();
        }
        self.state.accumulated[prefix_len..].to_vec()
    }

    fn clear_behavior_turn_tail(&mut self) {
        self.state.history_inputs.clear();
        let prefix_len = self.request.input.len();
        if self.state.accumulated.len() <= prefix_len {
            return;
        }
        if self
            .request
            .input
            .iter()
            .zip(self.state.accumulated.iter())
            .all(|(a, b)| a == b)
        {
            self.state.accumulated.truncate(prefix_len);
        }
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
        for action in &mut step.actions {
            self.state.next_action_id = self.state.next_action_id.saturating_add(1);
            action.call_id = self.state.next_action_id.to_string();
        }
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

    /// Hook failures are always degradable (see `StepResultHook`): the
    /// default rendering is used and the loop continues.
    async fn apply_step_result_hook(&mut self, step: &mut StepRecord) -> bool {
        if step.action_results.is_empty() && step.messages_sent.is_empty() {
            return false;
        }
        let Some(hook) = self.deps.step_result_hook.clone() else {
            return false;
        };
        let snapshot = self.snapshot();
        match hook.on_behavior_step_ob(&snapshot, step).await {
            Ok(output) => {
                if output.skip_next_inference {
                    return true;
                }
                if let Some(message) = output.user_message {
                    step.next_user_message = Some(normalize_user_message(message));
                }
                self.state.history_inputs.extend(output.history_inputs);
            }
            Err(err) => {
                log::warn!(
                    "behavior_loop: on_behavior_step_ob hook failed for behavior `{}` step {}; using default observation: {}",
                    step.meta.behavior_name,
                    step.meta.step_index,
                    err
                );
            }
        }
        false
    }

    /// Count one failed logical round. Returns `Some(outcome)` when the cap
    /// is exceeded and the run must end with `err`.
    fn bump_consecutive_errors(&mut self, err: LLMComputeError) -> Option<LLMContextOutcome> {
        self.state.consecutive_errors = self.state.consecutive_errors.saturating_add(1);
        let cap = self.request.error_policy.max_consecutive_errors;
        if cap > 0 && self.state.consecutive_errors > cap {
            return Some(self.finish_error(err));
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
        let trace = self.take_trace();
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

fn normalize_user_message(message: AiMessage) -> AiMessage {
    if matches!(message.role, AiRole::User) {
        return message;
    }
    AiMessage::text(AiRole::User, message.text_content())
}

/// Provider tool calls in `messages` that have no matching tool result.
fn unanswered_tool_calls(messages: &[AiMessage]) -> Vec<String> {
    let mut open: Vec<String> = Vec::new();
    for message in messages {
        for block in &message.content {
            match block {
                AiContent::ToolUse { call_id, .. } => open.push(call_id.clone()),
                AiContent::ToolResult { call_id, .. } => open.retain(|open_id| open_id != call_id),
                _ => {}
            }
        }
    }
    open
}

fn merge_usage(left: &AiUsage, right: &AiUsage) -> AiUsage {
    fn add_u64(a: Option<u64>, b: Option<u64>) -> Option<u64> {
        match (a, b) {
            (Some(x), Some(y)) => Some(x.saturating_add(y)),
            (Some(x), None) | (None, Some(x)) => Some(x),
            (None, None) => None,
        }
    }
    fn add_f64(a: Option<f64>, b: Option<f64>) -> Option<f64> {
        match (a, b) {
            (Some(x), Some(y)) => Some(x + y),
            (Some(x), None) | (None, Some(x)) => Some(x),
            (None, None) => None,
        }
    }
    fn add_cost(left: &Option<AiCost>, right: &Option<AiCost>) -> Option<AiCost> {
        match (left, right) {
            (Some(left), Some(right)) if left.currency == right.currency => Some(AiCost {
                amount: left.amount + right.amount,
                currency: left.currency.clone(),
            }),
            (Some(cost), None) | (None, Some(cost)) => Some(cost.clone()),
            _ => None,
        }
    }
    AiUsage {
        input_tokens: add_u64(left.input_tokens, right.input_tokens),
        output_tokens: add_u64(left.output_tokens, right.output_tokens),
        total_tokens: add_u64(left.total_tokens, right.total_tokens),
        cache_read_input_tokens: add_u64(
            left.cache_read_input_tokens,
            right.cache_read_input_tokens,
        ),
        cache_write_input_tokens: add_u64(
            left.cache_write_input_tokens,
            right.cache_write_input_tokens,
        ),
        cache_write_1h_input_tokens: add_u64(
            left.cache_write_1h_input_tokens,
            right.cache_write_1h_input_tokens,
        ),
        reasoning_tokens: add_u64(left.reasoning_tokens, right.reasoning_tokens),
        image_units: add_u64(left.image_units, right.image_units),
        audio_seconds: add_f64(left.audio_seconds, right.audio_seconds),
        video_seconds: add_f64(left.video_seconds, right.video_seconds),
        request_units: add_u64(left.request_units, right.request_units),
        characters: add_u64(left.characters, right.characters),
        audio_input_tokens: add_u64(left.audio_input_tokens, right.audio_input_tokens),
        image_input_tokens: add_u64(left.image_input_tokens, right.image_input_tokens),
        audio_output_tokens: add_u64(left.audio_output_tokens, right.audio_output_tokens),
        image_output_tokens: add_u64(left.image_output_tokens, right.image_output_tokens),
        reported_cost: None,
        cost: add_cost(&left.cost, &right.cost),
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
        Observation::Pending { call_id: cid, .. } => (format!("pending:{cid}"), true),
        Observation::Cancelled { reason, .. } => {
            // `is_error=false` — the call did not fail, it was interrupted.
            // The text marker lets a content-aware renderer / the LLM tell
            // cancellations apart from successful outputs.
            (format!("[cancelled] {reason}"), false)
        }
        Observation::Unresolved {
            reason,
            effect_unknown,
            ..
        } => {
            let text = if *effect_unknown {
                format!("[unresolved: result unknown] {reason}")
            } else {
                format!("[not executed] {reason}")
            };
            (text, true)
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
