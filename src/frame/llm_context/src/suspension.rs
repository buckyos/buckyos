//! Resuming a context: matching the fill with `state.suspended`, applying
//! it, and the pairing invariants a resumable state keeps.
//!
//! The only suspension is the context-limit yield; every other resumable
//! snapshot is resumed with `ResumeFromMidRun`.
//!
//! Every check here runs inside `LLMContext::resume`, before any inference or
//! tool call; a rejected fill leaves nothing half-applied.

use std::collections::HashSet;

use buckyos_api::{AiContent, AiMessage};
use serde_json::Value;

use crate::behavior_loop::{HistorySummaryRecord, StepRecord};
use crate::error::LLMComputeError;
use crate::outcome::ResumeFill;
use crate::request::LLMContextRequest;
use crate::state::{LLMContextSnapshot, LLMContextState, Suspension};

fn corrupted(message: impl Into<String>) -> LLMComputeError {
    LLMComputeError::SnapshotCorrupted(message.into())
}

/// Apply `fill` to a snapshot's request / state.
pub(crate) fn apply_fill(
    request: &mut LLMContextRequest,
    state: &mut LLMContextState,
    fill: ResumeFill,
    behavior: bool,
    now_ms: u64,
) -> Result<(), LLMComputeError> {
    if state.tool_batch.is_some() && state.action_step.is_some() {
        return Err(corrupted(
            "snapshot has both a tool batch and a behavior step in progress",
        ));
    }
    if state.action_step.is_some() && !behavior {
        return Err(corrupted(
            "snapshot has a behavior step in progress but the context is not in behavior mode",
        ));
    }
    match (state.suspended.take(), fill) {
        (Some(Suspension { at_ms, .. }), ResumeFill::RewrittenHistory { history }) if !behavior => {
            rewrite_history(request, state, history)?;
            resume_clock(state, at_ms, now_ms);
        }
        (
            Some(Suspension { at_ms, .. }),
            ResumeFill::RewrittenSteps {
                input,
                history_summaries,
                steps,
                last_step,
            },
        ) if behavior => {
            rewrite_steps(request, state, input, history_summaries, steps, last_step)?;
            resume_clock(state, at_ms, now_ms);
        }
        (None, ResumeFill::ResumeFromMidRun) => {}
        (suspended, fill) => {
            return Err(corrupted(format!(
                "{} fill does not match {} snapshot",
                fill_name(&fill),
                suspension_name(suspended.as_ref(), behavior)
            )))
        }
    }
    // Only the calls of a batch that were not dispatched yet may be open.
    let mut unanswered = unanswered_tool_calls(&state.accumulated);
    let mut expected: Vec<String> = state
        .tool_batch
        .as_ref()
        .map(|b| b.remaining.iter().map(|c| c.call_id.clone()).collect())
        .unwrap_or_default();
    unanswered.sort();
    expected.sort();
    if unanswered != expected {
        return Err(corrupted(format!(
            "accumulated history has unanswered tool calls: {}",
            unanswered.join(", ")
        )));
    }
    Ok(())
}

fn resume_clock(state: &mut LLMContextState, at_ms: u64, now_ms: u64) {
    // Suspended time is not charged to the wallclock budget.
    state.started_at_ms = state
        .started_at_ms
        .saturating_add(now_ms.saturating_sub(at_ms));
}

fn fill_name(fill: &ResumeFill) -> &'static str {
    match fill {
        ResumeFill::RewrittenHistory { .. } => "rewritten_history",
        ResumeFill::RewrittenSteps { .. } => "rewritten_steps",
        ResumeFill::ResumeFromMidRun => "resume_from_mid_run",
    }
}

fn suspension_name(suspended: Option<&Suspension>, behavior: bool) -> &'static str {
    match (suspended, behavior) {
        (None, _) => "a running (not suspended)",
        (Some(_), false) => "a function call context_limit",
        (Some(_), true) => "a behavior context_limit",
    }
}

fn rewrite_history(
    request: &mut LLMContextRequest,
    state: &mut LLMContextState,
    mut history: Vec<AiMessage>,
) -> Result<(), LLMComputeError> {
    if state.has_continuation() {
        return Err(corrupted(
            "cannot rewrite history while a tool batch is in progress",
        ));
    }
    strip_thinking(&mut history);
    if history.is_empty() {
        return Err(corrupted("rewritten history is empty"));
    }
    check_paired(&history)?;
    request.input = history.clone();
    state.accumulated = history;
    Ok(())
}

fn rewrite_steps(
    request: &mut LLMContextRequest,
    state: &mut LLMContextState,
    mut input: Vec<AiMessage>,
    history_summaries: Vec<HistorySummaryRecord>,
    mut steps: Vec<StepRecord>,
    mut last_step: Option<StepRecord>,
) -> Result<(), LLMComputeError> {
    if state.has_continuation() {
        return Err(corrupted(
            "cannot rewrite history while a tool batch or step is in progress",
        ));
    }
    strip_thinking(&mut input);
    if input.is_empty() {
        return Err(corrupted("rewritten input is empty"));
    }
    check_paired(&input)?;
    let known: HashSet<u32> = state
        .steps
        .iter()
        .chain(state.last_step.iter())
        .map(|s| s.meta.step_index)
        .collect();
    let mut prev: Option<u32> = None;
    for s in &steps {
        let idx = s.meta.step_index;
        if !known.contains(&idx) {
            return Err(corrupted(format!(
                "rewritten steps keep step {idx}, which is not in the snapshot"
            )));
        }
        if prev.is_some_and(|p| idx <= p) {
            return Err(corrupted("rewritten steps are not in step order"));
        }
        prev = Some(idx);
    }
    if let Some(hot) = &last_step {
        let idx = hot.meta.step_index;
        if state.last_step.as_ref().map(|s| s.meta.step_index) != Some(idx) {
            return Err(corrupted(format!(
                "rewritten last_step {idx} is not the snapshot's hot step"
            )));
        }
        if prev.is_some_and(|p| p >= idx) {
            return Err(corrupted("rewritten steps must precede the hot step"));
        }
    }
    for s in steps.iter_mut().chain(last_step.iter_mut()) {
        strip_step_thinking(s);
    }
    let mut tail = inner_transcript_of(request, state).to_vec();
    strip_thinking(&mut tail);
    request.input = input.clone();
    state.accumulated = input;
    state.accumulated.extend(tail);
    state.history_summaries = history_summaries;
    state.steps = steps;
    state.last_step = last_step;
    Ok(())
}

/// Behavior mode: the inner transcript of the step in progress (the native
/// tool loop messages not yet folded into a `StepRecord`), i.e. the messages
/// after `request.input` in `accumulated` (empty when `accumulated` does not
/// extend `request.input`). Not related to an AgentSession Turn.
pub(crate) fn inner_transcript_of<'a>(
    request: &LLMContextRequest,
    state: &'a LLMContextState,
) -> &'a [AiMessage] {
    let prefix = request.input.len();
    if state.accumulated.len() <= prefix || state.accumulated[..prefix] != request.input[..] {
        return &[];
    }
    &state.accumulated[prefix..]
}

/// Provider tool calls in `messages` that have no matching tool result.
pub(crate) fn unanswered_tool_calls(messages: &[AiMessage]) -> Vec<String> {
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

/// Every tool call answered by a later result, every result answering an
/// earlier call.
pub(crate) fn check_paired(messages: &[AiMessage]) -> Result<(), LLMComputeError> {
    let mut open: Vec<&str> = Vec::new();
    for message in messages {
        for block in &message.content {
            match block {
                AiContent::ToolUse { call_id, .. } => open.push(call_id),
                AiContent::ToolResult { call_id, .. } => {
                    match open.iter().position(|o| *o == call_id.as_str()) {
                        Some(i) => {
                            open.remove(i);
                        }
                        None => return Err(corrupted(format!(
                            "rewritten history has a tool result `{call_id}` without its tool call"
                        ))),
                    }
                }
                _ => {}
            }
        }
    }
    if !open.is_empty() {
        return Err(corrupted(format!(
            "rewritten history has unanswered tool calls: {}",
            open.join(", ")
        )));
    }
    Ok(())
}

/// Thinking blocks: `AiContent::Thinking`, and thinking-type provider state
/// (Claude `redacted_thinking`, OpenAI Responses `reasoning`).
pub fn is_thinking(part: &AiContent) -> bool {
    match part {
        AiContent::Thinking { .. } => true,
        AiContent::ProviderState { value, .. } => matches!(
            value.get("type").and_then(Value::as_str),
            Some("redacted_thinking" | "reasoning")
        ),
        _ => false,
    }
}

/// Drop every thinking block. Thinking is only valid under the exact prefix
/// it was produced with, so any rewrite of a prefix must drop the thinking
/// that follows it. A message left empty only because its thinking was
/// removed is dropped; text / tool calls / tool results are kept.
pub fn strip_thinking(messages: &mut Vec<AiMessage>) {
    messages.retain_mut(|m| {
        let before = m.content.len();
        m.content.retain(|p| !is_thinking(p));
        !(before > 0 && m.content.is_empty())
    });
}

/// [`strip_thinking`] over everything a snapshot can send back to the
/// provider: `request.input`, `accumulated` and the assistant message of
/// every step record.
pub fn strip_snapshot_thinking(snapshot: &mut LLMContextSnapshot) {
    strip_thinking(&mut snapshot.request.input);
    let state = &mut snapshot.state;
    strip_thinking(&mut state.accumulated);
    for step in state
        .steps
        .iter_mut()
        .chain(state.last_step.iter_mut())
        .chain(state.action_step.iter_mut().map(|a| &mut a.step))
    {
        strip_step_thinking(step);
    }
}

pub(crate) fn strip_step_thinking(step: &mut StepRecord) {
    if let Some(m) = step.assistant_message.as_mut() {
        let before = m.content.len();
        m.content.retain(|p| !is_thinking(p));
        if before > 0 && m.content.is_empty() {
            step.assistant_message = None;
        }
    }
}
