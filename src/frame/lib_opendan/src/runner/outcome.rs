//! Outcomes of a run segment (§8.3): interpreting one `LLMContext` outcome
//! for the session, the end-of-run commit (`finish_run`) and what follows
//! it, the report and the artifact version.

use std::sync::Arc;

use agent_tool::xllm::RunStatus;
use buckyos_api::AiUsage;
use llm_context::error::{ErrorSource, LLMComputeError, ProviderFailure};
use llm_context::outcome::{ContextOutput, LLMContextOutcome};
use llm_context::state::LLMContextSnapshot;
use serde_json::{json, Value};

use crate::error::Result;
use crate::protocol::*;
use crate::session::runs::RunHandle;
use crate::session::Session;
use crate::state::run_digest;

use super::flush::{run_history_entries, FlushMarks};
use super::history::maybe_compact;
use super::inputs::side_effects_from_worklog;
use super::live::{live_from_frame, remove_if_safe, suspend_run};
use super::shared::{commit_and_report, report, ClosedTurn, LiveCtx, Shared};
use super::tools::pending_sub_call;

const WAIT_USER_MSG: &str = "WAIT_USER_MSG";

/// How a run segment ended: the `kind` of a [`Next`], persisted in
/// run.json (`host.extra.finish`) and written as the worklog `outcome`
/// entry's kind. Serialized in snake_case (`process_done`, `pending_tool`,
/// `context_limit`, ...).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum FinishKind {
    /// The run delivered its result (`Done`, terminal).
    #[default]
    Done,
    /// A sub context (create-sub-context / fork child) returned to its
    /// caller.
    ProcessDone,
    /// `WAIT_USER_MSG`: the run ended waiting for input.
    Wait,
    /// Hand-over to another behavior: the run is suspended into
    /// `process_stack` (parked, or as the caller of a sub context).
    Switch,
    Budget,
    Error,
    Stopped,
    Interrupted,
    /// Paused on a deferred tool result.
    PendingTool,
    /// Paused at the context limit (compactions exhausted).
    ContextLimit,
}

impl FinishKind {
    /// The snake_case name used in run.json and the worklog.
    pub(super) fn as_str(self) -> &'static str {
        match self {
            FinishKind::Done => "done",
            FinishKind::ProcessDone => "process_done",
            FinishKind::Wait => "wait",
            FinishKind::Switch => "switch",
            FinishKind::Budget => "budget",
            FinishKind::Error => "error",
            FinishKind::Stopped => "stopped",
            FinishKind::Interrupted => "interrupted",
            FinishKind::PendingTool => "pending_tool",
            FinishKind::ContextLimit => "context_limit",
        }
    }
}

/// What `handle_context_outcome` decided. Persisted in run.json
/// (`host.extra.finish`) together with a terminal run status, so a finish
/// redone after a crash reaches the same result.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(super) struct Next {
    pub(super) run_ended: bool,
    pub(super) finished: bool,
    pub(super) outcome: Option<Outcome>,
    pub(super) waiting: bool,
    /// Stop the drive with this error (state carries it).
    pub(super) error: Option<Value>,
    pub(super) kind: FinishKind,
    pub(super) next_behavior: Option<String>,
    pub(super) answer: Option<String>,
    pub(super) report: Option<ReportSubmission>,
    pub(super) usage: Option<AiUsage>,
    /// The run was suspended into `process_stack` (not ended, not kept open).
    pub(super) suspended: bool,
    /// The open Turn ends with this run end (`None`: it continues — a
    /// hand-over, a sub context returning, a resumable suspension).
    pub(super) turn_end: Option<TurnStatus>,
    /// `Switch`: the sub context being called (`None`: SWITCH_CONTEXT).
    pub(super) call: Option<ChildCall>,
    /// `ProcessDone`: `ok | failed | needs_user_input`, handed to the caller
    /// with the result.
    pub(super) child_status: Option<String>,
    /// The session would finish, but sub sessions it must hear from are not
    /// settled: the run ends, the Turn stays open waiting for them.
    pub(super) wait_children: Vec<String>,
}

fn is_retryable_error(e: &LLMComputeError) -> bool {
    match e {
        LLMComputeError::Timeout | LLMComputeError::Cancelled => true,
        LLMComputeError::Provider { failure, .. } => *failure == ProviderFailure::Transient,
        other => other.source() == ErrorSource::Runtime,
    }
}

/// Session end condition after a `Done` that delivers the Turn's result;
/// `completed` counts the completed Turns including this one.
pub(super) fn decide_end(cfg: &SessionConfig, next: &mut Next, completed: u64) {
    next.turn_end = Some(TurnStatus::Completed);
    match cfg.session.end_condition.kind {
        EndConditionType::LlmDeclaresDone | EndConditionType::OutputSchema => {
            next.finished = true;
            next.outcome = Some(Outcome::Succeeded);
        }
        EndConditionType::MaxTurns => {
            let n = cfg
                .session
                .end_condition
                .detail
                .get("n")
                .and_then(Value::as_u64)
                .unwrap_or(1);
            if completed >= n {
                next.finished = true;
                next.outcome = Some(Outcome::Succeeded);
            } else {
                next.waiting = true;
            }
        }
    }
}

/// The run produced a Self Report (`<report>`).
pub(super) fn has_report(snapshot: &LLMContextSnapshot) -> bool {
    snapshot
        .state
        .last_report
        .as_deref()
        .is_some_and(|r| !r.trim().is_empty())
}

/// Where the run being interpreted stands in the session's call structure.
pub(super) struct CallSite<'a> {
    /// The run is a sub context: whatever it ends with returns to its caller.
    pub(super) child: bool,
    /// Sub contexts in progress, this run included.
    pub(super) depth: usize,
    /// Entry configuration of a hand-over target.
    pub(super) entry: &'a (dyn Fn(&str) -> Result<BehaviorEntry> + Send + Sync),
}

/// A sub context returns `result` to its caller.
fn returns(next: &mut Next, status: &str, result: Option<String>) {
    next.kind = FinishKind::ProcessDone;
    next.run_ended = true;
    next.waiting = false;
    next.finished = false;
    next.turn_end = None;
    next.error = None;
    next.next_behavior = None;
    next.child_status = Some(status.to_string());
    if result.is_some() {
        next.answer = result;
    }
}

/// A hand-over that cannot be carried out (target without an entry mode,
/// nesting limit). There is no fallback: a sub context reports it to its
/// caller, any other run ends the Turn as failed.
fn refuse_handover(next: &mut Next, site: &CallSite, message: String) {
    if site.child {
        returns(next, "failed", Some(message));
        return;
    }
    next.kind = FinishKind::Error;
    next.run_ended = true;
    next.turn_end = Some(TurnStatus::Failed);
    next.error = Some(json!({ "kind": "behavior_config", "message": message, "recoverable": false }));
}

/// A Turn that failed for good in a session nobody answers
/// (`wait_user_msg = finish_failed`) has no next input to recover with: the
/// session ends as failed, which is what its parent or creator is told.
fn fail_unattended(cfg: &SessionConfig, next: &mut Next) {
    if cfg.session.policy.wait_user_msg == WaitPolicy::FinishFailed {
        next.finished = true;
        next.outcome = Some(Outcome::Failed);
    }
}

/// Session-level meaning of a `Done` outcome (also used to rebuild the
/// decision of a run another executor finished or left at a hand-over).
/// `completed` = Turns completed before this outcome.
///
/// - `next_behavior = B`: decided by B's entry mode — `switch_context`
///   parks this run and enters B's own context; `create_sub_context` /
///   `fork` call B as a sub context. Both keep the Turn open.
/// - a sub context returns to its caller when it ends with a final report,
///   a hand-over to a `switch_context` target (a sub context does not leave
///   its call), or `WAIT_USER_MSG` (returned as `needs_user_input`: it never
///   consumes the caller's inputs).
/// - otherwise waiting for input completes the Turn only when a reply was
///   delivered (`replied`: a report or a sent message, D2). Completion follows
///   the session policy and end condition.
pub(super) fn classify_done(
    cfg: &SessionConfig,
    behavior: bool,
    next_behavior: Option<String>,
    answer: Option<String>,
    replied: bool,
    site: &CallSite,
    completed: u64,
    explicit_end: bool,
) -> Next {
    let mut next = Next {
        run_ended: true,
        kind: FinishKind::Done,
        answer,
        ..Default::default()
    };
    match next_behavior.as_deref() {
        Some(WAIT_USER_MSG) if site.child => returns(&mut next, "needs_user_input", None),
        Some(WAIT_USER_MSG) => match cfg.session.policy.wait_user_msg {
            WaitPolicy::Allowed => {
                next.kind = FinishKind::Wait;
                next.waiting = true;
                if replied {
                    next.turn_end = Some(TurnStatus::Completed);
                }
            }
            // Nobody answers in this session: the question is the report of
            // a failed Turn, and the session ends.
            WaitPolicy::FinishFailed => {
                next.kind = FinishKind::Wait;
                next.finished = true;
                next.outcome = Some(Outcome::Failed);
                next.turn_end = Some(TurnStatus::Failed);
                next.error = Some(json!({
                    "kind": "needs_user_input",
                    "message": "the session cannot wait for user input (session.policy.wait_user_msg = finish_failed); the question is in the report",
                    "recoverable": false,
                }));
            }
            WaitPolicy::FinishCompleted
                if cfg.session.policy.completion == CompletionPolicy::ExplicitReport =>
            {
                next.finished = true;
                next.outcome = Some(Outcome::Failed);
                next.turn_end = Some(TurnStatus::Failed);
                next.error = Some(
                    json!({"kind":"missing_end_report", "message":"WAIT_USER_MSG is not an explicit final report"}),
                );
            }
            WaitPolicy::FinishCompleted => decide_end(cfg, &mut next, completed + 1),
        },
        Some(b) if behavior && agent_tool::xllm::is_handover_target(b) => match (site.entry)(b) {
            Err(e) => refuse_handover(&mut next, site, e.to_string()),
            Ok(entry) if entry.mode.is_sub_context() => {
                let max_depth = cfg.session.policy.max_process_depth as usize;
                if site.depth >= max_depth {
                    refuse_handover(
                        &mut next,
                        site,
                        format!("sub contexts are nested {max_depth} deep; `{b}` cannot be called"),
                    );
                } else {
                    next.kind = FinishKind::Switch;
                    next.next_behavior = Some(b.to_string());
                    next.run_ended = false;
                    next.call = Some(ChildCall {
                        mode: entry.mode,
                        behavior: b.to_string(),
                        trigger: CallTrigger::Behavior,
                        task: None,
                    });
                }
            }
            Ok(_) if site.child => returns(&mut next, "ok", None),
            Ok(_) => {
                next.kind = FinishKind::Switch;
                next.next_behavior = Some(b.to_string());
                next.run_ended = false;
            }
        },
        _ if site.child => returns(&mut next, "ok", None),
        _ if explicit_end && cfg.session.policy.completion == CompletionPolicy::ExplicitReport => {
            next.finished = true;
            next.outcome = Some(Outcome::Succeeded);
            next.turn_end = Some(TurnStatus::Completed);
        }
        _ if explicit_end => decide_end(cfg, &mut next, completed + 1),
        _ if cfg.session.policy.completion == CompletionPolicy::ExplicitReport => {
            if cfg.channels.inputs.is_empty() {
                next.finished = true;
                next.kind = FinishKind::Error;
                next.outcome = Some(Outcome::Failed);
                next.turn_end = Some(TurnStatus::Failed);
                next.error = Some(
                    json!({"kind":"missing_end_report","message":"an explicit final report is required; this session has no input queue", "recoverable":false}),
                );
            } else {
                next.waiting = true;
                next.turn_end = Some(TurnStatus::Completed);
            }
        }
        _ => decide_end(cfg, &mut next, completed + 1),
    }
    next
}

/// Whether `run_id` is a sub context, and the call depth of the session.
pub(super) async fn call_site_of(sh: &Shared, run_id: &str) -> (bool, usize) {
    let s = sh.session.lock().await;
    let child = s
        .state
        .process_stack
        .last()
        .map(|f| f.role == FrameRole::Caller && f.run_id != run_id)
        .unwrap_or(false);
    (child, s.state.call_depth())
}

/// Carry out a decided hand-over: suspend the run and enter the target.
pub(super) async fn hand_over(
    sh: &Arc<Shared>,
    run: &RunHandle,
    behavior: bool,
    snapshot: &LLMContextSnapshot,
    next: &mut Next,
) -> Result<()> {
    let target = next.next_behavior.clone().unwrap_or_default();
    suspend_run(sh, run, behavior, snapshot, &target, next.call.clone()).await?;
    next.suspended = true;
    Ok(())
}

/// A session that would finish successfully while sub sessions it must hear
/// from are not settled does not finish: the run ends, the Turn stays open
/// (`waiting_for = children`); their end arrives as input of the same Turn
/// and the agent concludes then (xAgent §4.15).
pub(super) async fn hold_for_children(sh: &Shared, next: &mut Next) -> Result<()> {
    if next.report.is_some()
        || !(next.finished && next.run_ended && next.outcome == Some(Outcome::Succeeded))
    {
        return Ok(());
    }
    let pending = super::children::unsettled_children(sh).await?;
    if pending.is_empty() {
        return Ok(());
    }
    next.finished = false;
    next.outcome = None;
    next.turn_end = None;
    next.waiting = true;
    next.wait_children = pending;
    Ok(())
}

/// A hand-over target used for the first time: frozen from the agent's
/// catalog before the transfer is decided. A target that cannot be frozen
/// is left to the decision (no entry mode → refused, never a fallback).
pub(super) async fn freeze_target(sh: &Arc<Shared>, target: Option<&str>) {
    let Some(b) = target.filter(|b| agent_tool::xllm::is_handover_target(b) && *b != WAIT_USER_MSG)
    else {
        return;
    };
    if let Err(e) = super::drive::ensure_frozen(sh, Some(b)).await {
        log::warn!("session {}: behavior `{b}` cannot be frozen: {e}", sh.dir.sid());
    }
}

/// Interpret one `LLMContext` outcome for the session (run status, Turn
/// end, behavior switch, run end) and commit it. Returning an outcome does
/// not by itself complete the logical Turn: `Next.turn_end` says whether it
/// does.
pub(super) async fn handle_context_outcome(
    sh: &Arc<Shared>,
    lc: &mut LiveCtx,
    outcome: LLMContextOutcome,
) -> Result<Next> {
    if let LLMContextOutcome::Done { behavior_result, .. } = &outcome {
        freeze_target(
            sh,
            behavior_result.as_ref().and_then(|b| b.next_behavior.as_deref()),
        )
        .await;
    }
    let (cfg, completed, stop) = {
        let s = sh.session.lock().await;
        (
            s.config.clone(),
            s.state.turns_completed,
            s.state.stop_requested,
        )
    };
    let mut next = Next::default();
    let mut snapshot = lc.ctx.snapshot();
    let (child, depth) = call_site_of(sh, lc.run.run_id()).await;
    let entry_cfg = cfg.clone();
    let assembler = sh.deps.assembler.clone();
    let entry = move |b: &str| assembler.behavior_entry(&entry_cfg, b);
    let site = CallSite {
        child,
        depth,
        entry: &entry,
    };
    let status;
    match outcome {
        LLMContextOutcome::Done {
            output,
            usage,
            behavior_result,
            response,
            ..
        } => {
            let text = match output {
                ContextOutput::Text { content } => content,
                ContextOutput::Json { content } => content.to_string(),
            };
            let nb = behavior_result.as_ref().and_then(|b| b.next_behavior.clone());
            let report = behavior_result.as_ref().and_then(|b| b.self_report.clone());
            let answer = Some(report.unwrap_or_else(|| {
                if text.is_empty() {
                    response.message.text_content()
                } else {
                    text
                }
            }));
            let replied = has_report(&snapshot)
                || behavior_result
                    .as_ref()
                    .is_some_and(|b| !b.messages_to_send.is_empty());
            lc.run.checkpoint_with_results(&snapshot, None)?;
            super::rounds::flush_counts(sh, &lc.run, &lc.rounds).await?;
            super::reports::sync_xml(sh, &lc.run, &snapshot).await?;
            let final_report = super::reports::final_report(&lc.run)?;
            let explicit_end = final_report.is_some();
            let answer = final_report
                .as_ref()
                .map(ReportSubmission::delivery_text)
                .or(answer);
            next = classify_done(
                &cfg,
                lc.behavior,
                nb,
                answer,
                replied,
                &site,
                completed,
                explicit_end,
            );
            next.report = final_report;
            next.usage = Some(usage);
            hold_for_children(sh, &mut next).await?;
            status = match next.kind {
                // Suspended into process_stack: never terminal.
                FinishKind::Switch => RunStatus::Paused,
                FinishKind::Error => RunStatus::Failed,
                _ => RunStatus::Completed,
            };
        }
        LLMContextOutcome::BudgetExhausted { which, usage, .. } => {
            next.usage = Some(usage);
            next.run_ended = true;
            status = RunStatus::LimitReached;
            if site.child {
                // A sub context's failure is a result for its caller.
                returns(
                    &mut next,
                    "failed",
                    Some(format!("the sub context ran out of budget ({which:?})")),
                );
            } else {
                next.kind = FinishKind::Budget;
                next.turn_end = Some(TurnStatus::BudgetExhausted);
                next.error =
                    Some(json!({ "kind": "budget_exhausted", "message": format!("{which:?}") }));
                fail_unattended(&cfg, &mut next);
            }
        }
        LLMContextOutcome::Error { error, usage, .. } => {
            next.usage = Some(usage);
            next.kind = FinishKind::Error;
            let retry = is_retryable_error(&error);
            next.error = Some(json!({
                "kind": format!("{:?}", error.source()).to_lowercase(),
                "message": error.to_string(),
                "recoverable": retry,
            }));
            if retry {
                // Retryable: the run is kept and the Turn stays open.
                status = RunStatus::Paused;
            } else if site.child {
                status = RunStatus::Failed;
                returns(&mut next, "failed", Some(error.to_string()));
            } else {
                status = RunStatus::Failed;
                next.run_ended = true;
                next.turn_end = Some(TurnStatus::Failed);
                fail_unattended(&cfg, &mut next);
            }
        }
        LLMContextOutcome::Interrupted {
            reason,
            usage,
            snapshot: s,
            ..
        }
        | LLMContextOutcome::Settled {
            reason,
            usage,
            snapshot: s,
            ..
        } => {
            next.usage = Some(usage.clone());
            snapshot = s;
            if let Some(report) = super::reports::final_report(&lc.run)? {
                next = classify_done(
                    &cfg,
                    lc.behavior,
                    None,
                    Some(report.delivery_text()),
                    true,
                    &site,
                    completed,
                    true,
                );
                next.usage = Some(usage.clone());
                next.report = Some(report);
                status = RunStatus::Completed;
            } else if stop {
                next.kind = FinishKind::Stopped;
                next.run_ended = true;
                next.finished = true;
                next.outcome = Some(Outcome::Stopped);
                next.turn_end = Some(TurnStatus::Stopped);
                status = RunStatus::Interrupted;
            } else {
                next.kind = FinishKind::Interrupted;
                next.error = Some(json!({ "kind": "interrupted", "message": reason }));
                status = RunStatus::Interrupted;
            }
        }
        LLMContextOutcome::PendingTool { snapshot: s, .. } => {
            snapshot = s;
            status = RunStatus::Paused;
            match pending_sub_call(&snapshot, &entry)? {
                // Suspended on `call_behavior`: the run becomes the caller
                // of that sub context; its batch / step stays with it.
                Some(call) => {
                    next.kind = FinishKind::Switch;
                    next.next_behavior = Some(call.behavior.clone());
                    next.call = Some(call);
                }
                None => {
                    next.kind = FinishKind::PendingTool;
                    next.waiting = true;
                }
            }
        }
        LLMContextOutcome::ContextLimitReached {
            usage, snapshot: s, ..
        } => {
            next.usage = Some(usage);
            snapshot = s;
            next.kind = FinishKind::ContextLimit;
            next.error = Some(json!({ "kind": "context_limit", "message": "context limit reached" }));
            status = RunStatus::Paused;
        }
    }
    if next.report.is_some() {
        lc.run.checkpoint_with_results(&snapshot, None)?;
        super::rounds::flush_counts(sh, &lc.run, &lc.rounds).await?;
        crate::fault::point("report:after_paired_checkpoint");
    }
    // 1. results and snapshot first; only covered in-flight markers clear. A
    //    run that ends records the decision in the same run.json write.
    if next.run_ended {
        let finish = serde_json::to_value(&next).unwrap_or(Value::Null);
        lc.run.checkpoint_finish(&snapshot, status, finish)?;
    } else if next.kind == FinishKind::Switch && snapshot.state.suspended.is_none() {
        // `next_behavior` hand-over: recorded with the snapshot, so recovery
        // commits the transfer instead of inferring again.
        let target = next.next_behavior.clone().unwrap_or_default();
        lc.run.checkpoint_handover(&snapshot, &target)?;
    } else {
        lc.run.checkpoint_with_results(&snapshot, Some(status))?;
    }
    // Rounds of this segment: added to run.json (all executors) and to the
    // session statistics (this runner's attempts).
    let rounds = lc.rounds.take();
    if next.usage.is_some() || rounds.attempts > 0 {
        let _ = lc.run.record_usage(next.usage.as_ref(), rounds.attempts);
    }
    if rounds.attempts > 0 {
        let s = sh.session.lock().await;
        let _ = s.update_static(&sh.lease, |st| {
            st.rounds += rounds.attempts;
            st.rounds_failed += rounds.failed;
            st.rounds_interrupted += rounds.interrupted;
        });
    }
    crate::fault::point("outcome:after_checkpoint");
    if next.kind == FinishKind::Switch {
        hand_over(sh, &lc.run, lc.behavior, &snapshot, &mut next).await?;
        return Ok(next);
    }
    if next.run_ended {
        finish_run(sh, &lc.run, &snapshot, lc.behavior, next.clone()).await?;
        return Ok(next);
    }
    // Run kept (paused / waiting on a tool): state only.
    let mut s = sh.session.lock().await;
    s.state.run_state = if next.waiting {
        RunState::Waiting
    } else {
        RunState::Ready
    };
    if next.waiting {
        // Generated from the snapshot's suspended calls (status display and
        // event matching); the calls and their deadlines live in the
        // snapshot only.
        let pending = snapshot.state.pending_calls();
        s.state.waiting_for = Some(WaitingFor {
            kind: WaitingKind::Tool,
            refs: pending.iter().map(|p| p.task_id.clone()).collect(),
            deadline_ms: pending.iter().filter_map(|p| p.until_ms).min(),
        });
    }
    s.state.last_error = next.error.clone();
    commit_and_report(sh, &mut s).await?;
    Ok(next)
}

/// What the end-of-run commit leaves for the work after it.
struct RunEnded {
    run_id: String,
    sid: String,
    turn: u64,
    /// The Turn closed by this commit, if any.
    turn_closed: Option<TurnStatus>,
    /// Perception seq of the run digest (`+ 1` is the task outcome record).
    digest_seq: u64,
    /// The run this one replaced as `last_run` (cleanup candidate).
    prev: Option<String>,
}

/// End a run (§8.3 `finish_run`; idempotent when redone by reconcile). When
/// `next.turn_end` is set the open Turn is closed in the same commit.
///
/// Two halves around one commit point: [`commit_run_end`] writes everything
/// that must survive a crash in a single state commit (a redo by reconcile
/// reaches the same result); [`after_run_end`] does the best-effort work
/// that follows it (cleanup, compaction, statistics, perception, report).
pub(super) async fn finish_run(
    sh: &Arc<Shared>,
    run: &RunHandle,
    snapshot: &LLMContextSnapshot,
    behavior: bool,
    next: Next,
) -> Result<()> {
    let mut s = sh.session.lock().await;
    let ended = commit_run_end(sh, &mut s, run, snapshot, behavior, &next).await?;
    after_run_end(sh, s, run, &next, ended).await
}

/// The end-of-run commit: flush the run's unwritten history and its
/// `outcome` entry to the worklog, close the Turn, register the artifact
/// version and write the report of a finished session, move the session
/// state (live_run, run_state, result, process_stack hand-back) — all
/// landing in one `state.json` commit.
async fn commit_run_end(
    sh: &Arc<Shared>,
    s: &mut Session,
    run: &RunHandle,
    snapshot: &LLMContextSnapshot,
    behavior: bool,
    next: &Next,
) -> Result<RunEnded> {
    let run_id = run.run_id().to_string();
    let sid = s.sid().to_string();
    // The run's unwritten history (after what a suspension already flushed).
    let marks = s
        .state
        .live_run
        .as_ref()
        .filter(|l| l.run_id == run_id)
        .map(FlushMarks::of)
        .unwrap_or_default();
    let turn = s.state.current_turn();
    let (mut bodies, _) = run_history_entries(&run_id, snapshot, behavior, marks, turn);
    if let Some(report) = &next.report {
        if next.kind != FinishKind::ProcessDone {
            s.state.latest_report = Some(report.clone());
            s.state.final_report = Some(report.clone());
            bodies.push(WorklogBody::ReportDelivery {
                run_id: run_id.clone(),
                turn,
                submission: report.clone(),
                assistant: report.delivery_text(),
            });
        }
    }
    // Close the Turn before the report renders the counters.
    let turn_closed = match (next.turn_end, s.state.open_turn.take()) {
        (Some(status), Some(open)) => {
            super::outbound::queue_reply(
                sh,
                s,
                &run_id,
                super::outbound::TurnReply {
                    turn,
                    status,
                    answer: next.answer.clone(),
                    inputs: open.inputs,
                    has_msg: open.has_msg,
                    error: next.error.clone(),
                    has_placeholder: false,
                },
            )
            .await;
            super::turn_task::close_turn_task(
                s,
                turn,
                status,
                next.answer.as_deref(),
                next.error.as_ref(),
            );
            if status == TurnStatus::Completed {
                s.state.turns_completed += 1;
            }
            *sh.turn_closed.lock().expect("turn closed") = Some(ClosedTurn {
                turn,
                status,
                answer: next.answer.clone(),
            });
            Some(status)
        }
        (_, open) => {
            s.state.open_turn = open;
            None
        }
    };
    let mut artifact_ref = None;
    if next.finished {
        if let Some(aid) = s.config.artifact_id.clone() {
            // New work continues from the version the user accepted (S-12).
            let base = sh
                .agent()
                .artifacts()
                .head(&aid)
                .await?
                .and_then(|h| h.head)
                .filter(|h| h != &format!("v-{}", s.sid()));
            let version = register_outputs(s, run, base, &bodies, next.report.as_ref())?;
            sh.agent()
                .artifacts()
                .register_version(&sh.lease, &aid, s.config.workspace.clone(), version.clone())
                .await?;
            artifact_ref = Some(json!({ "aid": aid, "ver": version.ver }));
        }
        let answer = next.answer.clone().unwrap_or_default();
        s.write_report(&sh.lease, &render_report(s, &answer, next))?;
    }
    if !next.finished && next.kind != FinishKind::ProcessDone && next.report.is_some() {
        s.write_report(
            &sh.lease,
            &render_report(s, next.answer.as_deref().unwrap_or_default(), next),
        )?;
    }
    // Flush the run's unwritten history into the worklog.
    bodies.push(WorklogBody::Outcome {
        run_id: run_id.clone(),
        turn,
        kind: next.kind.as_str().into(),
        next_behavior: next.next_behavior.clone(),
        report: if next.report.is_some() && next.kind != FinishKind::ProcessDone {
            None
        } else {
            next.answer.clone().map(|a| a.chars().take(2000).collect())
        },
    });
    if let Some(status) = turn_closed {
        bodies.push(WorklogBody::TurnEnded {
            run_id: run_id.clone(),
            turn,
            status,
            at_ms: crate::now_ms(),
        });
    }
    s.append_worklog(&sh.lease, bodies)?;
    crate::fault::point("finish_run:after_flush");
    let prev = s.state.last_run.clone();
    let child_behavior = s.state.current_behavior.clone();
    s.state.live_run = None;
    s.state.last_run = Some(run_id.clone());
    s.state.one_line_status = one_line(next);
    s.state.last_error = next.error.clone();
    s.state.waiting_for = None;
    s.state.activity.touching.clear();
    if next.finished {
        s.state.run_state = RunState::Finished;
        s.state.process_stack.clear();
        s.state.outcome = next.outcome;
        s.state.stop_requested = false;
        s.state.activity = Activity::default();
        if s.config.session.kind == SessionKind::Work {
            s.state.acceptance = Acceptance::Pending;
        }
        let mut result = json!({
            "answer": next.answer.clone().map(|a| a.chars().take(2000).collect::<String>()),
            "answer_ref": REPORT_FILE,
        });
        if let Some(a) = artifact_ref {
            result["artifact_ref"] = a;
        }
        if let Some(report) = &next.report {
            result["submission"] = serde_json::to_value(report).unwrap_or(Value::Null);
            result["artifacts"] = json!(report.artifacts);
            if let Some(value) = &report.result {
                result["result"] = value.clone();
            }
        }
        s.state.result = Some(result);
    } else if next.waiting {
        s.state.run_state = RunState::Waiting;
        s.state.waiting_for = Some(if next.wait_children.is_empty() {
            WaitingFor {
                kind: WaitingKind::Input,
                refs: Vec::new(),
                deadline_ms: None,
            }
        } else {
            WaitingFor {
                kind: WaitingKind::Children,
                refs: next.wait_children.clone(),
                deadline_ms: None,
            }
        });
    } else {
        s.state.run_state = RunState::Ready;
    }
    if next.kind == FinishKind::ProcessDone {
        // A sub context ended: its caller becomes live again in this same
        // commit (same Turn), with the child's result — rendered into the
        // caller's hand-over batch (behavior trigger) or filled as the tool
        // result of the call when the caller's run is opened (tool trigger).
        if let Some(f) = s
            .state
            .process_stack
            .pop_if(|f| f.role == FrameRole::Caller)
        {
            let entry = f.entry.clone();
            let trigger = f.call.as_ref().map(|c| c.trigger.clone());
            s.state.live_run = Some(live_from_frame(f));
            let named = Some(entry.clone()).filter(|e| !e.is_empty());
            s.state.process_entry = named.clone();
            s.state.current_behavior = named;
            let mut result = json!({
                "behavior": child_behavior.unwrap_or_default(),
                "result": next.answer.clone().unwrap_or_default(),
                "status": next.child_status.clone().unwrap_or_else(|| "ok".into()),
                // Keep action / step ids unique after the return.
                "next_action_id": snapshot.state.next_action_id,
                "next_step_index": snapshot.state.next_step_index,
            });
            if let Some(report) = &next.report {
                result["submission"] = serde_json::to_value(report).unwrap_or(Value::Null);
            }
            match trigger {
                Some(CallTrigger::Tool { call_id, .. }) => {
                    result["call_id"] = json!(call_id);
                    s.state.internal_continuation = None;
                }
                _ => s.state.internal_continuation = Some(entry),
            }
            s.state.process_result = Some(result);
        }
    }
    // Background tasks the run started and that are still running (their
    // calls already returned): the session keeps following them after the
    // run — until it finishes, which never re-opens for a task.
    if next.finished {
        s.state.watched_tasks.clear();
        s.state.pending_events.clear();
    } else {
        let resolver = sh.tasks.lock().expect("tasks").clone();
        if let Some(r) = resolver {
            for t in r.active().await {
                if t.status == "running" && !s.state.watched_tasks.contains(&t.task_id) {
                    s.state.watched_tasks.push(t.task_id);
                }
            }
        }
        // The same tasks read from the run itself: a finish redone after a
        // crash has no resolver that saw the run, but every task a call
        // returned is in its results. A task that already ended is found
        // (and reported once) by the query that follows.
        for task_id in run.noted_tasks() {
            if !s.state.watched_tasks.contains(&task_id) {
                s.state.watched_tasks.push(task_id);
            }
        }
    }
    let digest_seq = s.state.perception_seq + 1;
    s.state.perception_seq = digest_seq + u64::from(next.finished);
    s.commit_state(&sh.lease)?; // commit point
    crate::fault::point("finish_run:after_commit");
    Ok(RunEnded {
        run_id,
        sid,
        turn,
        turn_closed,
        digest_seq,
        prev,
    })
}

/// After the commit (none of this is redone by reconcile): cleanup of the
/// replaced run and old snapshots, compaction by ratio, statistics,
/// perception records, registry report + notification, and the
/// consolidation of a succeeded self-improve session. The session lock is
/// released before the report.
async fn after_run_end(
    sh: &Arc<Shared>,
    mut s: tokio::sync::MutexGuard<'_, Session>,
    run: &RunHandle,
    next: &Next,
    ended: RunEnded,
) -> Result<()> {
    let RunEnded {
        run_id,
        sid,
        turn,
        turn_closed,
        digest_seq,
        prev,
    } = ended;
    // Cleanup of the replaced run.
    if let Some(p) = prev.filter(|p| p != &run_id && !s.state.references(p)) {
        if let Err(e) = remove_if_safe(sh, &p).await {
            log::warn!("cleanup of previous run {p}: {e}");
        }
    }
    let _ = run.prune(sh.deps.options.keep_snapshots);
    // Compaction by ratio.
    let cfg = s.config.clone();
    let budget = cfg
        .prompt
        .history_budget_tokens
        .unwrap_or(sh.deps.options.history_budget_tokens);
    let ratio = cfg.prompt.compact_ratio.unwrap_or(sh.deps.options.compact_ratio);
    let summarizer = sh.deps.summarizer.clone();
    if let Some(sm) = summarizer {
        if let Err(e) = maybe_compact(&mut s, &sh.lease, Some(sm.as_ref()), budget, ratio).await {
            log::warn!("compaction of {sid}: {e}");
        }
    }
    let usage = next.usage.clone();
    let turns = s.state.turns_completed;
    let _ = s.update_static(&sh.lease, |st| {
        st.runs += 1;
        st.turns = turns;
        if let Some(u) = &usage {
            st.input_tokens += u.input_tokens.unwrap_or(0);
            st.output_tokens += u.output_tokens.unwrap_or(0);
            st.total_tokens += u.total_tokens.unwrap_or(0);
            st.cost += u.reported_cost.unwrap_or(0.0);
        }
    });
    // Registry, perception, notification (catch-up later).
    let mut recs = vec![run_digest(
        &sid,
        digest_seq,
        &run_id,
        turn,
        turn_closed,
        &s.state.topic,
        s.config.artifact_id.iter().map(|a| format!("artifact:{a}")).collect(),
        &s.state.one_line_status,
        s.state.worklog.committed_seq,
    )];
    if next.finished {
        recs.push(PerceptionRecord {
            seq: digest_seq + 1,
            at_ms: crate::now_ms(),
            session_id: sid.clone(),
            kind: "task_outcome".into(),
            source: "session".into(),
            tags: s.state.topic.tags.clone(),
            objects: Vec::new(),
            summary: s.state.one_line_status.clone(),
            payload: json!({ "outcome": next.outcome, "acceptance": s.state.acceptance }),
            refs: Value::Null,
            ..Default::default()
        });
    }
    if let Err(e) = sh.agent().perception().append(&sh.lease, &sid, recs).await {
        log::warn!("perception of {sid}: {e}");
    }
    let consolidate = next.finished
        && next.outcome == Some(Outcome::Succeeded)
        && s.config.session.kind == SessionKind::SelfImprove;
    let window = perception_window(&s.config);
    let status = s.status(sh.lease.epoch());
    drop(s);
    report(sh, status).await;
    super::outbound::flush_outbox(sh).await;
    super::turn_task::flush_turn_tasks(sh).await;
    if consolidate {
        let kind_lease = sh.kind_lease.lock().expect("kind lease").clone();
        if let (Some(w), Some(l)) = (window, kind_lease) {
            let base = sh.agent().perception().cursor().await?;
            let upto = w.advanced(&base);
            let batch = crate::state::ConsolidationBatch {
                session_id: sid.clone(),
                summary: next.answer.clone().unwrap_or_default().chars().take(500).collect(),
                memory_ops: 0,
                notebook_ops: 0,
            };
            sh.agent()
                .cognition()
                .commit_consolidation(&l, &batch, &upto)
                .await?;
        }
    }
    Ok(())
}

fn one_line(next: &Next) -> String {
    let base = match (next.finished, next.kind) {
        (true, FinishKind::Stopped) => "stopped".to_string(),
        (true, _) => "finished".to_string(),
        (false, FinishKind::Wait) => "waiting for input".to_string(),
        (false, FinishKind::Switch) => format!(
            "switching to {}",
            next.next_behavior.clone().unwrap_or_default()
        ),
        (false, k) => k.as_str().to_string(),
    };
    let detail = next
        .answer
        .as_deref()
        .filter(|a| !a.trim().is_empty())
        .or_else(|| next.error.as_ref()?.get("message")?.as_str());
    match detail {
        Some(a) => {
            let first: String = a.trim().lines().next().unwrap_or("").chars().take(160).collect();
            format!("{base}: {first}")
        }
        None => base,
    }
}

fn render_report(s: &Session, answer: &str, next: &Next) -> String {
    format!(
        "# Report — {}\n\n- session: `{}`\n- outcome: {}\n- turns: {}\n\n{}\n",
        s.config
            .session
            .objective
            .lines()
            .next()
            .unwrap_or_default()
            .chars()
            .take(120)
            .collect::<String>(),
        s.sid(),
        next.outcome
            .map(|o| format!("{o:?}").to_lowercase())
            .unwrap_or_else(|| next.kind.as_str().to_string()),
        s.state.turns_completed,
        answer.trim()
    )
}

/// The session's contribution to its artifact (§6.5).
fn register_outputs(
    s: &Session,
    _run: &RunHandle,
    base: Option<String>,
    unflushed: &[WorklogBody],
    report: Option<&ReportSubmission>,
) -> Result<ArtifactVersion> {
    let mut outputs = Vec::new();
    if let Some(report) = report {
        outputs = report
            .artifacts
            .iter()
            .map(|a| a.reference.clone())
            .collect();
    } else if let Ok(rd) = std::fs::read_dir(s.dir.path()) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with('.') || n == README_FILE {
                continue;
            }
            outputs.push(n);
        }
    }
    outputs.sort();
    let mut side_effects = side_effects_from_worklog(s)?;
    // The run being finished is not in the worklog yet.
    for b in unflushed {
        let calls = match b {
            WorklogBody::Step { actions, .. } => actions,
            WorklogBody::AssistantMessage { tool_calls, .. } => tool_calls,
            _ => continue,
        };
        for a in calls {
            if a.effect != "read_only" {
                side_effects.push(SideEffectRef {
                    call_id: a.call_id.clone(),
                    tool: a.tool.clone(),
                    note: format!("{} cannot be undone by the session", a.tool),
                });
            }
        }
    }
    Ok(ArtifactVersion {
        ver: format!("v-{}", s.sid()),
        session: s.sid().to_string(),
        base,
        state: VersionState::Produced,
        outputs,
        workspace_ref: Value::Null,
        side_effects,
        updated_at_ms: crate::now_ms(),
    })
}

/// `extensions.opendan.perception_window` of a self-improve session.
pub fn perception_window(cfg: &SessionConfig) -> Option<crate::state::Backlog> {
    cfg.extensions
        .get("opendan")
        .and_then(|o| o.get("perception_window"))
        .and_then(|w| serde_json::from_value(w.clone()).ok())
}

pub(super) async fn perception_window_records(
    sh: &Shared,
    cfg: &SessionConfig,
) -> Result<Vec<PerceptionRecord>> {
    let mut out = Vec::new();
    if let Some(w) = perception_window(cfg) {
        for item in &w.items {
            out.extend(sh.agent().perception().read(item).await?);
        }
    }
    Ok(out)
}
