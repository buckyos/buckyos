//! `drive`: advance one session (§8.2) — lease, recovery, inputs, runtime
//! binding, input batches, outcomes, logical Turns, commit order.
//!
//! Terms: an *input batch* is one receipt `(run_id, input_seq)` committed
//! into the context; a *Turn* is the session's logical Input → result,
//! opened by the first batch committed while none is open and closed only
//! here, when an outcome is interpreted as its result or failure
//! (`Next.turn_end`); an *outcome* ends one run segment of the drive loop.
//!
//! The pieces live next to this loop: [`super::shared`] (state of a drive),
//! [`super::inputs`] (fetch / controls / decide), [`super::reconcile`]
//! (recovery), [`super::live`] (the live `LLMContext`: create, resume,
//! rewrite, suspend, input batch) and [`super::outcome`] (interpreting an
//! outcome, `finish_run`).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agent_tool::xllm::RunStatus;
use llm_context::tasks::{RunningTaskResolver, TaskState};
use serde_json::Value;

use crate::error::{OpenDanError, Result};
use crate::lock::{Acquire, Lease};
use crate::protocol::*;
use crate::runtime::{bin_plan_for, bind_or_verify, SessionEnv, SessionEnvCtx};
use crate::session::{Session, SessionDir};
use crate::state::run_digest;

use super::assembler::InputMaterial;
use super::input_view::{
    event_view, media_blocks, message_view, pending_event_view, unlocated_attachments, EventView,
    InputItem, InputView,
};
use super::inputs::{confirm_inputs, route_inputs, stop_queued};
use super::live::{
    commit_input_batch, new_run_context, open_state_live_run, resume_live_run, run_compacting,
    try_fill, InputBatch,
};
use super::outcome::{
    finish_run, handle_context_outcome, perception_window_records, FinishKind, Next,
};
use super::receipts::internal_task_key;
use super::reconcile::{reconcile_runs, Reconciled};
use super::children::poll_children;
use super::shared::{
    commit_and_report, report, ClosedTurn, LiveCtx, Opened, Shared, WaitingRun,
};
use super::{DriveResult, RunnerDeps, StopWhen};

/// Advance a session until `until` (§8.2).
pub async fn drive(sd: &SessionDir, deps: &RunnerDeps, until: StopWhen) -> DriveResult {
    let lease = match sd.acquire(deps.holder()) {
        Ok(Acquire::Acquired(l)) => Arc::new(l),
        Ok(Acquire::Busy(info)) => {
            return DriveResult::Busy {
                holder: info.and_then(|i| serde_json::to_value(i).ok()),
            }
        }
        Err(OpenDanError::NotDriver { driver, .. }) => return DriveResult::NotDriver { driver },
        Err(e) => {
            return DriveResult::Error {
                rev: 0,
                error: e.to_json(),
            }
        }
    };
    // The drive futures are large: keep them off the caller's stack.
    let result = Box::pin(drive_locked(sd, deps, until, lease.clone())).await;
    match Arc::try_unwrap(lease) {
        Ok(l) => l.release(),
        Err(arc) => drop(arc),
    }
    result
}

async fn drive_locked(
    sd: &SessionDir,
    deps: &RunnerDeps,
    until: StopWhen,
    lease: Arc<Lease>,
) -> DriveResult {
    let mut session = match sd.load(&lease) {
        Ok(s) => s,
        Err(OpenDanError::RecoveryBlocked(b)) => return DriveResult::RecoveryBlocked(b),
        Err(e) => {
            return DriveResult::Error {
                rev: 0,
                error: e.to_json(),
            }
        }
    };
    session.set_writer(WriterInfo {
        runner_id: deps.runner_id.clone(),
        principal: deps.who.clone(),
        host: Some(crate::runtime::native_host_id()),
        pid: std::process::id(),
        lock_epoch: lease.epoch(),
    });
    // Only registered sessions are advanced.
    match deps.agent.sessions().lookup(sd.sid()).await {
        Ok(Some(e)) => {
            let a = std::path::Path::new(&e.location)
                .canonicalize()
                .unwrap_or_else(|_| PathBuf::from(&e.location));
            let b = sd
                .path()
                .canonicalize()
                .unwrap_or_else(|_| sd.path().to_path_buf());
            if a != b {
                return DriveResult::Unregistered;
            }
        }
        _ => return DriveResult::Unregistered,
    }
    let mut deps = deps.clone();
    match crate::runtime::session_runtime(sd, &session.config, &deps.runtime, deps.agent.agent_root()) {
        Ok(r) => deps.runtime = r,
        Err(e) => return DriveResult::BindFailed { error: e.to_json() },
    }
    let mut sources = match deps.inputs.open(&session.config).await {
        Ok(s) => s,
        Err(e) => {
            return DriveResult::Error {
                rev: session.state.rev,
                error: e.to_json(),
            }
        }
    };
    if !session.config.prompt.initial_inputs.is_empty() {
        // Bootstrap material of a session without a queue: a read-only
        // source consumed through the same routing and receipts.
        sources.push(Arc::new(crate::channel::BootstrapSource::new(
            &session.config.prompt.initial_inputs,
        )));
    }
    let sh = Arc::new(Shared {
        deps: deps.clone(),
        lease: lease.clone(),
        session: tokio::sync::Mutex::new(session),
        sources,
        touched: Arc::new(Mutex::new(Vec::new())),
        interrupt: Mutex::new(None),
        agent_root: deps.agent.agent_root().map(|p| p.to_path_buf()),
        dir: sd.clone(),
        kind_lease: Mutex::new(None),
        tasks: Mutex::new(None),
        turn_closed: Mutex::new(None),
    });
    let r = Box::pin(drive_inner(&sh, until)).await;
    let rev = sh.session.lock().await.state.rev;
    match r {
        Ok(res) => res,
        Err(OpenDanError::RecoveryBlocked(b)) => {
            record_error(&sh, &OpenDanError::RecoveryBlocked(b.clone())).await;
            DriveResult::RecoveryBlocked(b)
        }
        Err(OpenDanError::RunBusy { run_id }) => DriveResult::RunBusy { run_id },
        Err(OpenDanError::LeaseLost(_)) => DriveResult::LeaseLost,
        Err(e) => {
            record_error(&sh, &e).await;
            DriveResult::Error {
                rev,
                error: e.to_json(),
            }
        }
    }
}

/// Record `last_error` (keeps live_run, runs and consumption untouched).
async fn record_error(sh: &Arc<Shared>, e: &OpenDanError) {
    if !sh.lease.held() {
        return;
    }
    let mut s = sh.session.lock().await;
    // Only the error is recorded: drop half-applied in-memory changes.
    if let Err(err) = s.reset_to_committed(&sh.lease) {
        log::warn!("cannot reload {} to record an error: {err}", sh.dir.sid());
        return;
    }
    s.state.last_error = Some(e.to_json());
    if let Err(err) = commit_and_report(sh, &mut s).await {
        log::warn!("cannot record error of {}: {err}", sh.dir.sid());
    }
}

/// Freeze what the session still needs from the agent's behavior catalog
/// (the whole closure at the first drive, one behavior on its first use
/// later) and commit the changed configuration (`config_rev + 1`,
/// `control_applied{behavior_frozen}`).
pub(super) async fn ensure_frozen(sh: &Arc<Shared>, behavior: Option<&str>) -> Result<()> {
    let cfg = sh.session.lock().await.config.clone();
    let Some(next) = sh
        .deps
        .assembler
        .ensure_frozen(&cfg, behavior, sh.agent(), &sh.deps.who)
        .await?
    else {
        return Ok(());
    };
    let added: Vec<String> = next
        .prompt
        .frozen
        .iter()
        .flat_map(|f| f.behaviors.keys())
        .filter(|k| {
            !cfg.prompt
                .frozen
                .as_ref()
                .is_some_and(|f| f.behaviors.contains_key(*k))
        })
        .cloned()
        .collect();
    let mut s = sh.session.lock().await;
    s.config = next;
    s.write_config(&sh.lease)?;
    let rev = s.config.config_rev;
    s.append_worklog(
        &sh.lease,
        vec![WorklogBody::ControlApplied {
            input: InputRef {
                src: "_runner".into(),
                index: 0,
                key: format!("behavior_frozen@{rev}"),
                kind: INPUT_TYPE_CONTROL.into(),
            },
            command: "behavior_frozen".into(),
            detail: serde_json::json!({ "behaviors": added, "config_rev": rev }),
        }],
    )?;
    commit_and_report(sh, &mut s).await
}

async fn catch_up(sh: &Arc<Shared>) -> Result<()> {
    let sid = sh.dir.sid().to_string();
    let (status, perception_seq, last_run, turn, topic, summary, wseq, reported) = {
        let s = sh.session.lock().await;
        (
            s.status(sh.lease.epoch()),
            s.state.perception_seq,
            s.state.last_run.clone().unwrap_or_default(),
            s.state.current_turn(),
            s.state.topic.clone(),
            s.state.one_line_status.clone(),
            s.state.worklog.committed_seq,
            s.state.reported_rev,
        )
    };
    if reported < status.rev {
        report(sh, status).await;
    }
    let last = sh.agent().perception().last_seq(&sid).await?;
    if last < perception_seq {
        let mut recs = Vec::new();
        for seq in last + 1..=perception_seq {
            recs.push(run_digest(
                &sid,
                seq,
                &last_run,
                turn,
                None,
                &topic,
                Vec::new(),
                &summary,
                wseq,
            ));
        }
        sh.agent()
            .perception()
            .append(&sh.lease, &sid, recs)
            .await?;
    }
    Ok(())
}

fn finished_result(s: &Session) -> DriveResult {
    DriveResult::Finished {
        rev: s.state.rev,
        outcome: s.state.outcome,
        acceptance: s.state.acceptance,
    }
}

/// Stop the session (`control(stop)`): the live run ends through the
/// normal finish path with a `Stopped` Turn. A run waiting for tasks is
/// answered now — cancellable tasks of the Turn are cancelled, every
/// suspended call gets the state of its task — so the snapshot keeps every
/// call paired with a result.
async fn stop_session(
    sh: &Arc<Shared>,
    live: Option<LiveCtx>,
    waiting: Option<WaitingRun>,
    env: &SessionEnv,
) -> Result<()> {
    let next = Next {
        run_ended: true,
        finished: true,
        outcome: Some(Outcome::Stopped),
        kind: FinishKind::Stopped,
        turn_end: Some(TurnStatus::Stopped),
        ..Default::default()
    };
    let mut live = live;
    let mut waiting = waiting;
    stop_children(sh).await;
    // A run state still references (e.g. a caller just resumed after its sub
    // context) ends through the normal finish path.
    if live.is_none() && waiting.is_none() && sh.session.lock().await.state.live_run.is_some() {
        match open_state_live_run(sh, env).await? {
            Opened::Ctx(l) => live = Some(l),
            Opened::Waiting(w) => waiting = Some(w),
        }
    }
    if let Some(w) = waiting {
        live = match try_fill(sh, w, true).await? {
            Ok(l) => Some(l),
            Err(_) => {
                return Err(OpenDanError::Other(
                    "the waiting run could not be answered for the stop".into(),
                ))
            }
        };
    }
    match live {
        Some(lc) => {
            // Background tasks of the Turn being stopped.
            for t in lc.resolver.active().await {
                if t.status == "running" && t.cancellable {
                    let _ = lc.resolver.cancel(&t.task_id).await;
                }
            }
            let snap = lc.ctx.snapshot();
            lc.run
                .checkpoint_with_results(&snap, Some(RunStatus::Interrupted))?;
            finish_run(sh, &lc.run, &snap, lc.behavior, next).await
        }
        None => {
            let mut s = sh.session.lock().await;
            s.state.run_state = RunState::Finished;
            s.state.outcome = Some(Outcome::Stopped);
            s.state.stop_requested = false;
            s.state.process_stack.clear();
            s.state.internal_continuation = None;
            s.state.watched_tasks.clear();
            s.state.activity = Activity::default();
            if s.config.session.kind == SessionKind::Work {
                s.state.acceptance = Acceptance::Pending;
            }
            s.state.one_line_status = "stopped".into();
            let turn = s.state.current_turn();
            let mut bodies = vec![WorklogBody::Outcome {
                run_id: String::new(),
                turn,
                kind: "stopped".into(),
                next_behavior: None,
                report: None,
            }];
            if s.state.open_turn.take().is_some() {
                bodies.push(WorklogBody::TurnEnded {
                    run_id: String::new(),
                    turn,
                    status: TurnStatus::Stopped,
                    at_ms: crate::now_ms(),
                });
                *sh.turn_closed.lock().expect("turn closed") = Some(ClosedTurn {
                    turn,
                    status: TurnStatus::Stopped,
                    answer: None,
                });
            }
            s.append_worklog(&sh.lease, bodies)?;
            commit_and_report(sh, &mut s).await?;
            Ok(())
        }
    }
}

/// A stopped parent stops its sub sessions that are not finished: those with
/// a queue get a `stop`; one without a queue is stopped by whoever drives it
/// (the host of this process stops the sub sessions it drives when their
/// parent finished as stopped).
async fn stop_children(sh: &Arc<Shared>) {
    let me = sh.dir.sid().to_string();
    let Ok(children) = sh.agent().sessions().children_of(&[me.clone()]).await else {
        return;
    };
    for e in children {
        if e.status.run_state == RunState::Finished || e.input_queue.is_none() {
            continue;
        }
        let input = PostedInput::control(
            &sh.deps.who,
            format!("stop:parent:{me}"),
            ControlCommand::Stop {
                reason: Some(format!("parent session {me} was stopped")),
            },
        );
        if let Err(err) = sh.agent().sessions().post_input(&e.session_id, &input).await {
            log::warn!("cannot stop sub session {}: {err}", e.session_id);
        }
    }
}

/// Task ids the suspended calls of the run state references wait for
/// (matching task notifications to them at the first routing pass).
fn pending_task_ids(snapshot: &llm_context::state::LLMContextSnapshot) -> Vec<String> {
    snapshot
        .state
        .pending_calls()
        .iter()
        .map(|p| p.task_id.clone())
        .collect()
}

fn task_summary(task_id: &str, state: &TaskState) -> (String, String) {
    let cut = |text: &str| {
        let one: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
        one.chars().take(240).collect::<String>()
    };
    match state {
        TaskState::Finished(r) => (
            "finished".into(),
            format!(
                "task {task_id} {}: {}",
                if r.success { "finished" } else { "failed" },
                cut(&r.output)
            ),
        ),
        TaskState::Unknown { reason } => (
            "unknown".into(),
            format!(
                "task {task_id}: state unknown ({}); check its actual result before relying on it",
                cut(reason)
            ),
        ),
        TaskState::Running { brief, .. } => {
            ("updated".into(), format!("task {task_id} is running: {}", cut(brief)))
        }
    }
}

/// Background tasks the session follows after their run ended (§6.2): a
/// task that ended is handled like a subscribed event of `task:<id>` — by
/// its explicit subscription when there is one, else as an Input event (the
/// runner's implicit active subscription). No input queue is involved: the
/// state is always queried by the saved task id, so a lost notification, a
/// restart or an empty inbox change nothing; a task nobody can answer for
/// is reported as unknown, never re-created.
async fn poll_watched_tasks(sh: &Arc<Shared>) -> Result<Vec<FetchedInput>> {
    let (watched, cfg) = {
        let s = sh.session.lock().await;
        (s.state.watched_tasks.clone(), s.config.clone())
    };
    if watched.is_empty() {
        return Ok(Vec::new());
    }
    let resolver: Option<Arc<dyn RunningTaskResolver>> = sh
        .tasks
        .lock()
        .expect("tasks")
        .clone()
        .or_else(|| sh.deps.xllm.buckyos_tasks.clone());
    let mut inputs = Vec::new();
    let mut observed = Vec::new();
    for task_id in watched {
        let state = match &resolver {
            Some(r) if r.can_resolve(&task_id) => r.state(&task_id).await,
            _ => TaskState::Unknown {
                reason: "no task manager of this runner knows the task".into(),
            },
        };
        if matches!(state, TaskState::Running { .. }) {
            continue;
        }
        let (event, summary) = task_summary(&task_id, &state);
        let ev = AgentEvent {
            subscription_id: None,
            source: EventSource::task(task_id.clone()),
            event,
            seq: None,
            summary,
            data_ref: None,
            terminal: true,
        };
        let key = internal_task_key(&task_id);
        match cfg.subscription_for(None, "task", &task_id, &ev.event) {
            Some(sub) if sub.mode == SubscriptionMode::Semi => observed.push((task_id, sub.id, ev, key)),
            _ => inputs.push(FetchedInput {
                src: INTERNAL_TASK_SRC.to_string(),
                index: 0,
                kind: INPUT_TYPE_EVENT.to_string(),
                key,
                from: "runner".to_string(),
                at_ms: crate::now_ms(),
                input: Ok(SessionInput::Event(ev)),
            }),
        }
    }
    if !observed.is_empty() {
        let mut s = sh.session.lock().await;
        for (task_id, sub_id, ev, key) in observed {
            merge_pending_event(
                &mut s.state.pending_events,
                Some(&sub_id),
                &ev,
                &key,
                None,
                crate::now_ms(),
            );
            s.state.watched_tasks.retain(|t| t != &task_id);
        }
        commit_and_report(sh, &mut s).await?;
    }
    Ok(inputs)
}

/// Semi-subscription state versions to show before a controlled input:
/// terminal versions first, at most `budget`; what does not fit stays
/// pending. Selecting has no side effect — only the receipt of a committed
/// batch clears what it injected.
fn select_snapshot(state: &SessionState, budget: usize) -> (Vec<EventView>, Vec<EventReceipt>) {
    let mut picked: Vec<(bool, &PendingEvent, &PendingEventVersion)> = Vec::new();
    for p in &state.pending_events {
        if let Some(v) = &p.terminal {
            picked.push((true, p, v));
        }
        if let Some(v) = &p.latest {
            picked.push((false, p, v));
        }
    }
    picked.sort_by_key(|(terminal, _, _)| !*terminal);
    picked.truncate(budget);
    let views = picked
        .iter()
        .map(|(terminal, p, v)| pending_event_view(p, v, *terminal))
        .collect();
    let receipts = picked
        .iter()
        .map(|(_, p, v)| EventReceipt {
            subscription_id: p.subscription_id.clone(),
            source: p.source.clone(),
            seq: v.seq,
            key: v.key.clone(),
        })
        .collect();
    (views, receipts)
}

/// Watches the control inputs while the context runs (tool calls included):
/// a queued `stop` interrupts the run. It only looks — consuming and
/// committing the control stays with the driver (single writer).
struct StopMonitor {
    seen: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}

impl StopMonitor {
    fn spawn(sh: &Arc<Shared>, wake_event: Option<String>) -> Self {
        let seen = Arc::new(AtomicBool::new(false));
        let flag = seen.clone();
        let sh = sh.clone();
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = sh.deps.waker.wait(wake_event.as_deref(), sh.deps.options.poll_interval) => {}
                    _ = sh.deps.stop.wait() => {}
                }
                if sh.deps.stop.requested() || stop_queued(&sh).await {
                    flag.store(true, Ordering::SeqCst);
                    if let Some(h) = sh.interrupt.lock().expect("interrupt lock").as_ref() {
                        h.interrupt("session stop requested");
                    }
                    return;
                }
            }
        });
        Self { seen, task }
    }

    /// Stop watching; the task (and its hold on the drive's shared state,
    /// the lease included) is gone when this returns.
    async fn finish(self) -> bool {
        self.task.abort();
        let _ = self.task.await;
        self.seen.load(Ordering::SeqCst)
    }
}

/// `StopWhen::TurnClosed`: the Turn a commit of this drive closed.
async fn turn_closed_result(sh: &Arc<Shared>, until: StopWhen) -> Option<DriveResult> {
    if until != StopWhen::TurnClosed {
        return None;
    }
    let closed = sh.turn_closed.lock().expect("turn closed").clone()?;
    let rev = sh.session.lock().await.state.rev;
    Some(DriveResult::TurnClosed {
        rev,
        turn: closed.turn,
        status: closed.status,
        answer: closed.answer,
    })
}

/// `StopWhen::TurnClosed` without a Turn closed by this drive: the open
/// Turn as it is, or `Idle` when there is none (never a made-up Turn).
async fn turn_open_result(sh: &Arc<Shared>) -> DriveResult {
    let s = sh.session.lock().await;
    match &s.state.open_turn {
        Some(t) => DriveResult::TurnOpen {
            rev: s.state.rev,
            turn: t.index,
            waiting_for: s.state.waiting_for.clone(),
        },
        None if s.state.is_finished() => finished_result(&s),
        None => DriveResult::Idle {
            rev: s.state.rev,
            run_state: s.state.run_state,
        },
    }
}

/// A stop requested by the driving process (`RunnerDeps.stop`): recorded
/// like a consumed `stop` control, then handled by the same path.
async fn external_stop(sh: &Arc<Shared>) -> Result<()> {
    if !sh.deps.stop.requested() {
        return Ok(());
    }
    let mut s = sh.session.lock().await;
    if s.state.stop_requested || s.state.is_finished() {
        return Ok(());
    }
    s.state.stop_requested = true;
    let rev = s.state.rev;
    s.append_worklog(
        &sh.lease,
        vec![WorklogBody::ControlApplied {
            input: InputRef {
                src: "_runner".into(),
                index: 0,
                key: format!("stop@{rev}"),
                kind: INPUT_TYPE_CONTROL.into(),
            },
            command: "stop".into(),
            detail: serde_json::json!({ "reason": "the driving process was asked to stop", "from": sh.deps.who }),
        }],
    )?;
    commit_and_report(sh, &mut s).await
}

async fn drive_inner(sh: &Arc<Shared>, until: StopWhen) -> Result<DriveResult> {
    // Behavior entry modes are checked before anything runs: an invalid
    // table never degrades into some default way of switching.
    sh.session
        .lock()
        .await
        .config
        .behaviors()
        .map_err(OpenDanError::InvalidArgument)?;
    // Kind leases: one consolidation at a time for the whole agent.
    let kind = sh.session.lock().await.config.session.kind;
    let _kind_lease = if kind == SessionKind::SelfImprove {
        match sh
            .agent()
            .locks()
            .acquire("self_improve", sh.deps.holder())?
        {
            Acquire::Acquired(l) => Some(Arc::new(l)),
            Acquire::Busy(info) => {
                return Ok(DriveResult::Busy {
                    holder: info.and_then(|i| serde_json::to_value(i).ok()),
                })
            }
        }
    } else {
        None
    };
    *sh.kind_lease.lock().expect("kind lease") = _kind_lease.clone();
    // Runtime binding happens before any inference (Q3).
    let (binding, env) = {
        let cfg = sh.session.lock().await.config.clone();
        let plan = bin_plan_for(
            &cfg,
            sh.agent_root.as_deref(),
            sh.deps.session_cli.clone(),
            &[],
        )?;
        let bound = bind_or_verify(
            &sh.dir,
            &sh.lease,
            sh.deps.runtime.as_ref(),
            &cfg,
            sh.agent_root.as_deref(),
            &sh.deps.app_tools,
            &plan,
            &sh.deps.runner_id,
        )
        .await;
        let binding = match bound {
            Ok(b) => b,
            Err(e @ (OpenDanError::Bind(_) | OpenDanError::RuntimeMismatch { .. })) => {
                let mut s = sh.session.lock().await;
                s.state.last_error = Some(e.to_json());
                commit_and_report(sh, &mut s).await?;
                return Ok(DriveResult::BindFailed { error: e.to_json() });
            }
            Err(e) => return Err(e),
        };
        let ctx = {
            let s = sh.session.lock().await;
            SessionEnvCtx {
                session_id: s.sid().to_string(),
                session_dir: sh.dir.path().to_path_buf(),
                agent_did: s.config.session.agent_did.clone(),
                agent_root: sh.agent_root.clone(),
                input_queue: s.config.channels.kmsg().map(|(_, q, _)| q.to_string()),
                trace_id: s.sid().to_string(),
                extra_env: s
                    .config
                    .runtime
                    .env
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            }
        };
        let env = crate::runtime::open_session_env(&binding, &ctx);
        (binding, env)
    };
    // Behaviors come from the agent's catalog: frozen before any inference
    // (the creator may not have been able to read the catalog).
    ensure_frozen(sh, None).await?;
    sh.session
        .lock()
        .await
        .config
        .behaviors()
        .map_err(OpenDanError::InvalidArgument)?;
    // Recovery: runs, receipts, executions — before reading any new input.
    // The snapshot, the committed state and what the run waits for are
    // aligned first; a task that already ended is collected, only a task
    // still running is waited for. None of it depends on the inbox.
    let mut live: Option<LiveCtx> = None;
    let mut waiting: Option<WaitingRun> = None;
    let reconciled = Box::pin(reconcile_runs(sh)).await?;
    {
        let s = sh.session.lock().await;
        confirm_inputs(&sh.sources, &s.state).await;
    }
    if let Err(e) = catch_up(sh).await {
        log::warn!("catch-up of {}: {e}", sh.dir.sid());
    }
    // Replies committed and not handed over yet (a crash after the commit,
    // a sink that was not reachable).
    super::outbound::flush_outbox(sh).await;
    let pending_now = match &reconciled {
        Reconciled::Resume(_, snapshot) => pending_task_ids(snapshot),
        Reconciled::None => Vec::new(),
    };
    // A Turn closed by the recovery itself is this drive's result.
    if let Some(r) = turn_closed_result(sh, until).await {
        return Ok(r);
    }
    let mut routed = route_inputs(sh, false, &pending_now).await?;
    if sh.session.lock().await.state.is_finished() {
        return Ok(finished_result(&*sh.session.lock().await));
    }
    if let Reconciled::Resume(run, snapshot) = reconciled {
        match Box::pin(resume_live_run(sh, run, snapshot, &env)).await? {
            Opened::Ctx(l) => live = Some(l),
            Opened::Waiting(w) => waiting = Some(w),
        }
    }
    let started = Instant::now();
    let mut outcomes_handled = 0u64;
    loop {
        sh.lease.check()?;
        external_stop(sh).await?;
        if sh.session.lock().await.state.stop_requested {
            Box::pin(stop_session(sh, live.take(), waiting.take(), &env)).await?;
            if let Some(r) = turn_closed_result(sh, until).await {
                return Ok(r);
            }
            return Ok(finished_result(&*sh.session.lock().await));
        }
        if let StopWhen::MaxOutcomes { n } = until {
            if outcomes_handled >= n {
                let s = sh.session.lock().await;
                return Ok(DriveResult::OutcomesHandled {
                    rev: s.state.rev,
                    run_state: s.state.run_state,
                });
            }
        }
        // 串行等待: a run suspended on tool calls. Its tasks are queried
        // (the same path for a notification, the fallback poll and the
        // drive entry); once every wait is over the results are filled and
        // the same run / Turn continues. Meanwhile msg / Input events stay
        // queued — they never stand in for a missing tool result — while
        // controls keep being applied.
        if let Some(w) = waiting.take() {
            match Box::pin(try_fill(sh, w, false)).await? {
                Ok(lc) => live = Some(lc),
                Err(w) => {
                    let (rev, rs) = {
                        let mut s = sh.session.lock().await;
                        let refs = w.task_ids();
                        let wf = WaitingFor {
                            kind: WaitingKind::Tool,
                            refs,
                            deadline_ms: w.deadline_ms(),
                        };
                        if s.state.run_state != RunState::Waiting
                            || s.state.waiting_for.as_ref() != Some(&wf)
                        {
                            s.state.run_state = RunState::Waiting;
                            s.state.waiting_for = Some(wf);
                            commit_and_report(sh, &mut s).await?;
                        }
                        (s.state.rev, s.state.run_state)
                    };
                    if matches!(until, StopWhen::MaxOutcomes { .. }) {
                        return Ok(DriveResult::OutcomesHandled { rev, run_state: rs });
                    }
                    if started.elapsed() >= sh.deps.options.max_wait {
                        if until == StopWhen::TurnClosed {
                            return Ok(turn_open_result(sh).await);
                        }
                        return Ok(DriveResult::Idle { rev, run_state: rs });
                    }
                    if !routed.task_notified {
                        let mut pause = sh.deps.options.poll_interval;
                        if let Some(d) = w.deadline_ms() {
                            let left = d.saturating_sub(crate::now_ms()).max(10);
                            pause = pause.min(Duration::from_millis(left));
                        }
                        let wake = sh.session.lock().await.config.channels.wake_event.clone();
                        tokio::select! {
                            _ = sh.deps.waker.wait(wake.as_deref(), pause) => {}
                            _ = sh.deps.stop.wait() => {}
                        }
                    }
                    routed = route_inputs(sh, false, &w.task_ids()).await?;
                    waiting = Some(w);
                    continue;
                }
            }
        }
        let (cfg, state) = {
            let s = sh.session.lock().await;
            (s.config.clone(), s.state.clone())
        };
        // A caller whose tool-triggered sub context returned: the result is
        // filled as that call's tool result and the run continues its batch
        // by itself. No input batch can be placed before the batch is done,
        // so inputs wait for the next boundary.
        let returning = live.is_none() && state.tool_return_pending();
        if returning {
            match Box::pin(open_state_live_run(sh, &env)).await? {
                Opened::Ctx(l) => live = Some(l),
                Opened::Waiting(w) => {
                    waiting = Some(w);
                    continue;
                }
            }
        }
        // A sub context never consumes its caller's input queue: while one is
        // in progress, msg / event stay queued for the caller.
        // Neither does a run whose suspended calls were just answered: its
        // batch continues first.
        let hold_inputs = returning
            || state.child_call().is_some()
            || live.as_ref().is_some_and(|l| l.filled);
        // The context that receives the batch: its frozen entry decides the
        // consumption policy and the templates (a hand-over reads the
        // target's, after its entry was validated).
        let behavior = state
            .current_behavior
            .clone()
            .or_else(|| cfg.prompt.behavior.clone())
            .unwrap_or_default();
        let entry = if behavior.is_empty() {
            None
        } else {
            // First use of a behavior the freeze did not cover.
            ensure_frozen(sh, Some(&behavior)).await?;
            let cfg = sh.session.lock().await.config.clone();
            Some(sh.deps.assembler.behavior_entry(&cfg, &behavior)?)
        };
        let cfg = sh.session.lock().await.config.clone();
        let (templates, input_cfg) = cfg.input_config(entry.as_ref());
        // Candidates: msg and accepted Input events of the bus, then the
        // completions of watched background tasks.
        let mut picked: Vec<FetchedInput> = Vec::new();
        if !hold_inputs {
            let mut candidates = std::mem::take(&mut routed.candidates);
            candidates.extend(poll_watched_tasks(sh).await?);
            // What the session's sub sessions need from it (xAgent §4.14).
            candidates.extend(poll_children(sh, &[]).await?);
            let take = match input_cfg.mode {
                InputMode::Single => 1,
                InputMode::Batch => sh.deps.options.input_batch_max.max(1),
            };
            picked = candidates.into_iter().take(take).collect();
        }
        let state = sh.session.lock().await.state.clone();
        // One controlled input per batch: `on_init` → `on_context_switch` →
        // `on_input`; external inputs that may be consumed join that batch.
        let hook = if !state.bootstrap_done {
            HOOK_ON_INIT
        } else if state.internal_continuation.is_some() {
            HOOK_ON_CONTEXT_SWITCH
        } else {
            HOOK_ON_INPUT
        };
        let triggered =
            !picked.is_empty() || !state.bootstrap_done || state.internal_continuation.is_some();
        let mut msg = None;
        let mut media = Vec::new();
        if triggered {
            let me = sh.agent().sessions().lookup(sh.dir.sid()).await?;
            let policy = &cfg.session.policy;
            let hints = if sh.deps.options.load_hints
                && policy.load_hints
                && (!state.bootstrap_done || !picked.is_empty())
            {
                sh.agent()
                    .cognition()
                    .recall_hints(&crate::state::RecallQuery {
                        tags: state.topic.tags.clone(),
                        max_hints: 5,
                    })
                    .await
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            let active = if policy.observe == ObserveScope::EventsAndActive {
                sh.agent()
                    .activity()
                    .active(me.as_ref(), sh.deps.options.active_sessions_limit)
                    .await
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            let now_ms = crate::now_ms();
            let items: Vec<InputItem> = picked
                .iter()
                .filter_map(|m| match &m.input {
                    Ok(SessionInput::Msg(sm)) => {
                        let mut v = message_view(&m.key, m.at_ms, sm, &cfg.session.agent_did);
                        for a in &mut v.attachments {
                            a.path = sh.deps.assembler.attachment_path(&cfg, &a.obj_id);
                        }
                        Some(InputItem::Msg(v))
                    }
                    Ok(SessionInput::Event(ev)) => Some(InputItem::Event(event_view(&m.key, ev))),
                    _ => None,
                })
                .collect();
            let material = InputMaterial {
                hook: hook.to_string(),
                input: InputView::new(hook, now_ms, items),
                hints,
                active,
                runtime_status: serde_json::to_value(sh.deps.runtime.info().await?)
                    .unwrap_or_default(),
                now_ms,
                perceptions: if !state.bootstrap_done
                    && cfg.session.kind == SessionKind::SelfImprove
                {
                    perception_window_records(sh, &cfg).await?
                } else {
                    Vec::new()
                },
            };
            msg = sh
                .deps
                .assembler
                .render_input(&cfg, &state, &templates, &material)
                .await?
                .filter(|t| !t.trim().is_empty());
            match &msg {
                Some(text) => {
                    // Text stays self-contained without the media blocks.
                    for a in unlocated_attachments(text, &material.input.messages) {
                        log::warn!(
                            "session {}: the `{hook}` template does not locate attachment {} ({}); a display name is not a readable source",
                            sh.dir.sid(),
                            a.index,
                            a.obj_id
                        );
                    }
                    media = media_blocks(&material.input.messages, input_cfg.media);
                }
                // Selected inputs must never vanish into an empty message:
                // nothing is consumed, the scene is kept.
                None if !picked.is_empty() => {
                    return Err(OpenDanError::InvalidArgument(format!(
                        "the `{hook}` template rendered an empty message for {} selected input(s)",
                        picked.len()
                    )));
                }
                None => {}
            }
        }
        if msg.is_none() && state.internal_continuation.is_some() {
            msg = Some(format!(
                "<session_input hook=\"{HOOK_ON_CONTEXT_SWITCH}\">Continue with behavior `{}`.</session_input>",
                state.internal_continuation.clone().unwrap_or_default()
            ));
        }
        let resumable = live.as_ref().map(|l| l.ready).unwrap_or(false);
        if msg.is_none() && !resumable {
            // Nothing to infer on. Saved semi-subscription state stays where
            // it is until a controlled input uses it.
            {
                let mut s = sh.session.lock().await;
                if s.state.bootstrap_done
                    && !s.state.is_finished()
                    && s.state.run_state != RunState::Waiting
                    && s.state.last_error.is_none()
                {
                    s.state.run_state = RunState::Waiting;
                    s.state.waiting_for = Some(WaitingFor {
                        kind: WaitingKind::Input,
                        refs: Vec::new(),
                        deadline_ms: None,
                    });
                    commit_and_report(sh, &mut s).await?;
                }
            }
            let (rev, rs) = {
                let s = sh.session.lock().await;
                (s.state.rev, s.state.run_state)
            };
            // Sub sessions and followed tasks advance without this
            // session's queue: waiting for them is polling, with or without
            // an input channel.
            let advancing = {
                let s = sh.session.lock().await;
                !s.state.watched_tasks.is_empty()
                    || s.state
                        .waiting_for
                        .as_ref()
                        .is_some_and(|w| w.kind == WaitingKind::Children)
            };
            match until {
                StopWhen::Idle => return Ok(DriveResult::Idle { rev, run_state: rs }),
                StopWhen::MaxOutcomes { .. } => {
                    return Ok(DriveResult::OutcomesHandled { rev, run_state: rs })
                }
                StopWhen::TurnClosed
                    if !advancing || started.elapsed() >= sh.deps.options.max_wait =>
                {
                    return Ok(turn_open_result(sh).await);
                }
                StopWhen::TurnClosed => {
                    let wake = cfg.channels.wake_event.clone();
                    tokio::select! {
                        _ = sh.deps.waker.wait(wake.as_deref(), sh.deps.options.poll_interval) => {}
                        _ = sh.deps.stop.wait() => {}
                    }
                    routed = route_inputs(sh, false, &[]).await?;
                    continue;
                }
                StopWhen::Finished => {
                    if sh.session.lock().await.state.last_error.is_some() {
                        let err = sh.session.lock().await.state.last_error.clone();
                        return Ok(DriveResult::Error {
                            rev,
                            error: err.unwrap_or(Value::Null),
                        });
                    }
                    if started.elapsed() >= sh.deps.options.max_wait {
                        return Ok(DriveResult::Idle { rev, run_state: rs });
                    }
                    // The queue notification only says "look again": losing
                    // or repeating it changes nothing, persisted inputs are
                    // found by the poll.
                    let wake = cfg.channels.wake_event.clone();
                    tokio::select! {
                        _ = sh.deps.waker.wait(wake.as_deref(), sh.deps.options.poll_interval) => {}
                        _ = sh.deps.stop.wait() => {}
                    }
                    routed = route_inputs(sh, false, &[]).await?;
                    if sh.session.lock().await.state.is_finished() {
                        return Ok(finished_result(&*sh.session.lock().await));
                    }
                    continue;
                }
            }
        }
        let mut lc = match live.take() {
            Some(l) => l,
            None if state.live_run.is_some() => match Box::pin(open_state_live_run(sh, &env)).await? {
                Opened::Ctx(l) => l,
                Opened::Waiting(w) => {
                    waiting = Some(w);
                    routed = route_inputs(sh, false, &[]).await?;
                    continue;
                }
            },
            None => Box::pin(new_run_context(sh, &binding, &env)).await?,
        };
        if let Some(text) = msg {
            // The semi-subscription snapshot is assembled right before the
            // controlled input and committed with it as one batch.
            let budget = match cfg.session.policy.observe {
                ObserveScope::Off => 0,
                _ => sh.deps.options.change_budget,
            };
            let (views, shown) = select_snapshot(&state, budget);
            let snapshot = sh
                .deps
                .assembler
                .render_semi_subscription_snapshot(&templates, &views)
                .await?
                .map(|t| (t, shown));
            Box::pin(commit_input_batch(
                sh,
                &mut lc,
                InputBatch {
                    hook,
                    picked: &picked,
                    snapshot,
                    text,
                    media,
                },
            ))
            .await?;
        }
        lc.ready = false;
        lc.filled = false;
        let monitor = StopMonitor::spawn(sh, cfg.channels.wake_event.clone());
        let outcome = Box::pin(run_compacting(sh, &mut lc)).await;
        if monitor.finish().await {
            // The stop is consumed and committed by the driver before the
            // interrupted outcome is interpreted.
            route_inputs(sh, true, &[]).await?;
            external_stop(sh).await?;
        }
        let next = Box::pin(handle_context_outcome(sh, &mut lc, outcome?)).await?;
        outcomes_handled += 1;
        let mut pending_now = Vec::new();
        if next.kind == FinishKind::PendingTool {
            let w = WaitingRun::of(lc);
            pending_now = w.task_ids();
            waiting = Some(w);
        } else if !next.run_ended && !next.suspended {
            live = Some(lc);
        }
        let (rev, rs) = {
            let s = sh.session.lock().await;
            (s.state.rev, s.state.run_state)
        };
        // The Turn this outcome closed is the result asked for, also when
        // the session finished with it.
        if let Some(r) = turn_closed_result(sh, until).await {
            return Ok(r);
        }
        if next.finished {
            return Ok(finished_result(&*sh.session.lock().await));
        }
        if let Some(err) = next.error.clone() {
            return Ok(DriveResult::Error { rev, error: err });
        }
        routed = route_inputs(sh, false, &pending_now).await?;
        // Idle = nothing to do: a Turn that ended waiting for input returns,
        // unless inputs are already queued for the next one.
        if next.waiting
            && until == StopWhen::Idle
            && next.kind != FinishKind::PendingTool
            && routed.candidates.is_empty()
            && poll_children(sh, &pending_now).await?.is_empty()
        {
            return Ok(DriveResult::Idle { rev, run_state: rs });
        }
    }
}
