//! Outcomes of a run segment (§8.3): interpreting one `LLMContext` outcome
//! for the session, the end-of-run commit (`finish_run`) and what follows
//! it, the report and the artifact version.

use std::sync::Arc;

use agent_tool::xllm::RunStatus;
use buckyos_api::AiUsage;
use llm_context::error::{ErrorSource, LLMComputeError, ProviderFailure};
use llm_context::outcome::{ContextOutput, LLMContextOutcome, ResumeFill};
use llm_context::state::LLMContextSnapshot;
use llm_context::{LLMContext, NEXT_BEHAVIOR_END};
use serde_json::{json, Value};

use crate::error::{OpenDanError, Result};
use crate::protocol::*;
use crate::session::runs::RunHandle;
use crate::session::Session;
use crate::state::run_digest;

use super::flush::{run_history_entries, FlushMarks};
use super::history::maybe_compact;
use super::inputs::side_effects_from_worklog;
use super::live::{live_from_frame, remove_if_safe, stop_executions, suspend_run};
use super::shared::{commit_and_report, report, LiveCtx, Shared};

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
    /// A fork child returned to its caller.
    ProcessDone,
    /// `WAIT_USER_MSG`: the run ended waiting for input.
    Wait,
    /// Hand-over to another behavior (the run continues or is suspended).
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
    pub(super) usage: Option<AiUsage>,
    /// The run was suspended into `process_stack` (not ended, not kept open).
    pub(super) suspended: bool,
    /// The open Turn ends with this run end (`None`: it continues — a
    /// hand-over, a fork child returning, a resumable suspension).
    pub(super) turn_end: Option<TurnStatus>,
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

/// Session-level meaning of a `Done` outcome (also used to rebuild the
/// decision of a run another executor finished). `completed` = Turns
/// completed before this outcome. Hand-overs (switch, fork child return)
/// keep the Turn open; waiting for input completes it only when a reply
/// was delivered (`replied`: a report or a sent message, D2), otherwise the
/// next input joins the same Turn.
pub(super) fn classify_done(
    cfg: &SessionConfig,
    behavior: bool,
    next_behavior: Option<String>,
    answer: Option<String>,
    replied: bool,
    fork_child: bool,
    completed: u64,
) -> Next {
    let mut next = Next {
        run_ended: true,
        kind: FinishKind::Done,
        answer,
        ..Default::default()
    };
    match next_behavior.as_deref() {
        // A fork child returns to its caller whatever it declares.
        _ if fork_child && next_behavior.as_deref() != Some(WAIT_USER_MSG) => {
            next.kind = FinishKind::ProcessDone;
        }
        Some(WAIT_USER_MSG) => {
            next.kind = FinishKind::Wait;
            next.waiting = true;
            if replied {
                next.turn_end = Some(TurnStatus::Completed);
            }
        }
        // `END` (waist) and `done` (xllm: report without actions) are
        // terminal; anything else hands over to that behavior.
        Some(b)
            if behavior
                && !b.eq_ignore_ascii_case(NEXT_BEHAVIOR_END)
                && !b.eq_ignore_ascii_case("done") =>
        {
            next.kind = FinishKind::Switch;
            next.next_behavior = Some(b.to_string());
            next.run_ended = false;
        }
        _ => decide_end(cfg, &mut next, completed + 1),
    }
    next
}

pub(super) async fn is_fork_child(sh: &Shared, run_id: &str) -> bool {
    let s = sh.session.lock().await;
    s.state
        .process_stack
        .last()
        .map(|f| f.mode == ProcessMode::Fork && f.run_id != run_id)
        .unwrap_or(false)
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
            let fork_child = is_fork_child(sh, lc.run.run_id()).await;
            next = classify_done(
                &cfg,
                lc.behavior,
                nb,
                answer,
                replied,
                fork_child,
                completed,
            );
            next.usage = Some(usage);
            status = if next.kind == FinishKind::Switch {
                // The run continues (normal switch) or is suspended into
                // process_stack (fork / independent): never terminal.
                match next
                    .next_behavior
                    .as_deref()
                    .and_then(|b| sh.deps.assembler.process_mode(&cfg, b))
                {
                    None => RunStatus::Running,
                    Some(_) => RunStatus::Paused,
                }
            } else {
                RunStatus::Completed
            };
        }
        LLMContextOutcome::BudgetExhausted { which, usage, .. } => {
            next.usage = Some(usage);
            next.run_ended = true;
            next.kind = FinishKind::Budget;
            next.turn_end = Some(TurnStatus::BudgetExhausted);
            status = RunStatus::LimitReached;
            next.error = Some(json!({ "kind": "budget_exhausted", "message": format!("{which:?}") }));
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
            } else {
                status = RunStatus::Failed;
                next.run_ended = true;
                next.turn_end = Some(TurnStatus::Failed);
            }
        }
        LLMContextOutcome::Interrupted {
            reason,
            usage,
            snapshot: s,
            ..
        } => {
            next.usage = Some(usage);
            snapshot = s;
            if stop {
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
            next.kind = FinishKind::PendingTool;
            next.waiting = true;
            status = RunStatus::Paused;
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
    // 1. results and snapshot first; only covered in-flight markers clear. A
    //    run that ends records the decision in the same run.json write.
    if next.run_ended {
        let finish = serde_json::to_value(&next).unwrap_or(Value::Null);
        lc.run.checkpoint_finish(&snapshot, status, finish)?;
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
    if next.kind == FinishKind::Switch {
        let b = next.next_behavior.clone().unwrap_or_default();
        match sh.deps.assembler.process_mode(&cfg, &b) {
            None => {
                // Normal switch: same context, same run, new behavior.
                let mut snap = snapshot.clone();
                snap.request.behavior_name = b.clone();
                lc.ctx = LLMContext::resume(snap, ResumeFill::ResumeFromMidRun, lc.deps.clone())
                    .map_err(|e| OpenDanError::Llm(format!("behavior switch: {e}")))?;
                *sh.interrupt.lock().expect("interrupt") = Some(lc.ctx.interrupt_handle());
                let mut s = sh.session.lock().await;
                s.state.current_behavior = Some(b.clone());
                s.state.internal_continuation = Some(b);
                s.state.run_state = RunState::Ready;
                commit_and_report(sh, &mut s).await?;
            }
            Some(mode) => {
                suspend_run(sh, lc, &snapshot, mode, &b).await?;
                next.suspended = true;
            }
        }
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
        s.state.waiting_for = Some(WaitingFor {
            kind: WaitingKind::Tool,
            refs: Vec::new(),
            deadline_ms: None,
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
    // Background processes of the run must be confirmed stopped first.
    stop_executions(sh, run).await?;
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
    // Close the Turn before the report renders the counters.
    let turn_closed = match (next.turn_end, s.state.open_turn.is_some()) {
        (Some(status), true) => {
            s.state.open_turn = None;
            if status == TurnStatus::Completed {
                s.state.turns_completed += 1;
            }
            Some(status)
        }
        _ => None,
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
            let version = register_outputs(s, run, base, &bodies)?;
            sh.agent()
                .artifacts()
                .register_version(&sh.lease, &aid, s.config.workspace.clone(), version.clone())
                .await?;
            artifact_ref = Some(json!({ "aid": aid, "ver": version.ver }));
        }
        let answer = next.answer.clone().unwrap_or_default();
        s.write_report(&sh.lease, &render_report(s, &answer, next))?;
    }
    // Flush the run's unwritten history into the worklog.
    bodies.push(WorklogBody::Outcome {
        run_id: run_id.clone(),
        turn,
        kind: next.kind.as_str().into(),
        next_behavior: next.next_behavior.clone(),
        report: next.answer.clone().map(|a| a.chars().take(2000).collect()),
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
        s.state.result = Some(result);
    } else if next.waiting {
        s.state.run_state = RunState::Waiting;
        s.state.waiting_for = Some(WaitingFor {
            kind: WaitingKind::Input,
            refs: Vec::new(),
            deadline_ms: None,
        });
    } else {
        s.state.run_state = RunState::Ready;
    }
    if next.kind == FinishKind::ProcessDone {
        // Fork child ended: its caller becomes live again in this same
        // commit, with the child's result for its hand-over batch (same
        // Turn).
        if let Some(f) = s.state.process_stack.pop() {
            let entry = f.entry.clone();
            s.state.live_run = Some(live_from_frame(f));
            s.state.process_entry = Some(entry.clone());
            s.state.current_behavior = Some(entry.clone());
            s.state.internal_continuation = Some(entry);
            s.state.process_result = Some(json!({
                "behavior": child_behavior.unwrap_or_default(),
                "result": next.answer.clone().unwrap_or_default(),
                // Keep action / step ids unique after the return.
                "next_action_id": snapshot.state.next_action_id,
                "next_step_index": snapshot.state.next_step_index,
            }));
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
    match &next.answer {
        Some(a) if !a.trim().is_empty() => {
            let first: String = a.trim().lines().next().unwrap_or("").chars().take(160).collect();
            format!("{base}: {first}")
        }
        _ => base,
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
) -> Result<ArtifactVersion> {
    let mut outputs = Vec::new();
    if let Ok(rd) = std::fs::read_dir(s.dir.path()) {
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

pub(super) async fn perception_window_records(sh: &Shared, cfg: &SessionConfig) -> Result<Vec<PerceptionRecord>> {
    let mut out = Vec::new();
    if let Some(w) = perception_window(cfg) {
        for item in &w.items {
            out.extend(sh.agent().perception().read(item).await?);
        }
    }
    Ok(out)
}
