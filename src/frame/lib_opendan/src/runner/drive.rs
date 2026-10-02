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
use std::sync::{Arc, Mutex};
use std::time::Instant;

use agent_tool::xllm::RunStatus;
use serde_json::Value;

use crate::error::{OpenDanError, Result};
use crate::lock::{Acquire, Lease};
use crate::protocol::*;
use crate::runtime::{bin_plan_for, bind_or_verify, SessionEnv, SessionEnvCtx};
use crate::session::{Session, SessionDir};
use crate::state::run_digest;

use super::assembler::InputMaterial;
use super::hook::{check_changes, ACTIVE_CURSOR};
use super::inputs::{apply_controls, confirm_inputs, fetch_inputs, reject, reject_leftovers};
use super::live::{
    commit_input_batch, new_run_context, open_state_live_run, resume_live_run, run_compacting,
};
use super::outcome::{
    finish_run, handle_context_outcome, perception_window_records, FinishKind, Next,
};
use super::reconcile::{reconcile_runs, Reconciled};
use super::shared::{commit_and_report, report, LiveCtx, Shared};
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
    let result = drive_locked(sd, deps, until, lease.clone()).await;
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
    let mut config = deps.runtime.config();
    let declared = session.config.prompt.llm_context.get("runtime").cloned();
    if let Some(value) = declared {
        let parsed = agent_tool::xllm::parse_llm_context_file(
            &sd.path().join("session_config.json"),
            &serde_json::json!({"runtime":value}).to_string(),
        );
        match parsed {
            Ok(file) => {
                if let Some(r) = file.runtime {
                    let original = config.clone();
                    config.merge_over(&r);
                    if original.kind() != config.kind()
                        || original
                            .id
                            .as_ref()
                            .is_some_and(|id| config.id.as_ref() != Some(id))
                        || original
                            .workdir
                            .as_ref()
                            .is_some_and(|cwd| config.workdir.as_ref() != Some(cwd))
                        || original
                            .remote_ssh
                            .as_ref()
                            .is_some_and(|ssh| config.remote_ssh.as_ref() != Some(ssh))
                        || original
                            .env
                            .iter()
                            .any(|(k, v)| config.env.get(k) != Some(v))
                        || original.tmux.as_ref().is_some_and(|tmux| {
                            config.tmux.as_ref().is_none_or(|new| {
                                tmux.session
                                    .as_ref()
                                    .is_some_and(|v| new.session.as_ref() != Some(v))
                                    || tmux
                                        .socket
                                        .as_ref()
                                        .is_some_and(|v| new.socket.as_ref() != Some(v))
                                    || tmux.mode.is_some_and(|v| new.mode != Some(v))
                            })
                        })
                    {
                        return DriveResult::BindFailed {
                            error: serde_json::json!({"kind":"RuntimeMismatch","reason":"provided runtime differs from prompt.llm_context.runtime"}),
                        };
                    }
                }
            }
            Err(e) => {
                return DriveResult::BindFailed {
                    error: serde_json::json!({"kind":"Config","reason":e.to_string()}),
                }
            }
        }
    }
    if config.workdir.is_none() {
        match crate::runtime::resolve_workdir(sd, &session.config, deps.agent.agent_root()) {
            Ok(p) => config.workdir = Some(p.display().to_string()),
            Err(e) => return DriveResult::BindFailed { error: e.to_json() },
        }
    }
    if config != deps.runtime.config() {
        match agent_tool::runtime::RuntimeRegistry::from_config(&config) {
            Ok(r) => deps.runtime = r,
            Err(e) => {
                return DriveResult::BindFailed {
                    error: serde_json::json!({"reason":e.to_string()}),
                }
            }
        }
    }
    let sources = match deps.inputs.open(&session.config).await {
        Ok(s) => s,
        Err(e) => {
            return DriveResult::Error {
                rev: session.state.rev,
                error: e.to_json(),
            }
        }
    };
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
    });
    let r = drive_inner(&sh, until).await;
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

async fn stop_session(sh: &Arc<Shared>, live: Option<LiveCtx>, env: &SessionEnv) -> Result<()> {
    let next = Next {
        run_ended: true,
        finished: true,
        outcome: Some(Outcome::Stopped),
        kind: FinishKind::Stopped,
        turn_end: Some(TurnStatus::Stopped),
        ..Default::default()
    };
    // A run state still references (e.g. a parent just resumed after its fork
    // child) ends through the normal finish path.
    let live = match live {
        Some(l) => Some(l),
        None if sh.session.lock().await.state.live_run.is_some() => {
            Some(open_state_live_run(sh, env).await?)
        }
        None => None,
    };
    match live {
        Some(lc) => {
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
            }
            s.append_worklog(&sh.lease, bodies)?;
            commit_and_report(sh, &mut s).await?;
            Ok(())
        }
    }
}

async fn drive_inner(sh: &Arc<Shared>, until: StopWhen) -> Result<DriveResult> {
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
    // Recovery: runs, receipts, executions — before reading any new input.
    let mut live: Option<LiveCtx> = None;
    let reconciled = reconcile_runs(sh).await?;
    {
        let s = sh.session.lock().await;
        confirm_inputs(&sh.sources, &s.state).await;
    }
    if let Err(e) = catch_up(sh).await {
        log::warn!("catch-up of {}: {e}", sh.dir.sid());
    }
    let mut inputs = fetch_inputs(sh).await?;
    apply_controls(sh, &mut inputs, false).await?;
    {
        let s = sh.session.lock().await;
        if s.state.is_finished() {
            drop(s);
            reject_leftovers(sh, &mut inputs).await?;
            return Ok(finished_result(&*sh.session.lock().await));
        }
    }
    if let Reconciled::Resume(run, snapshot) = reconciled {
        live = Some(resume_live_run(sh, run, snapshot, &env).await?);
    }
    let started = Instant::now();
    let mut outcomes_handled = 0u64;
    loop {
        sh.lease.check()?;
        if sh.session.lock().await.state.stop_requested {
            stop_session(sh, live.take(), &env).await?;
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
        let (cfg, state) = {
            let s = sh.session.lock().await;
            (s.config.clone(), s.state.clone())
        };
        // Pull policy: msg / event make an input batch (opening a Turn, or
        // joining the open one); changes ride along.
        let mut picked = inputs.take(InputKind::Msg);
        picked.extend(inputs.take(InputKind::Event));
        picked.sort_by_key(|m| (m.src.clone(), m.index));
        if cfg.session.input_policy == InputPolicy::None && !picked.is_empty() {
            let mut s = sh.session.lock().await;
            let bodies: Vec<WorklogBody> = picked
                .iter()
                .map(|m| reject(&mut s, m, "session input_policy is none"))
                .collect();
            s.append_worklog(&sh.lease, bodies)?;
            commit_and_report(sh, &mut s).await?;
            confirm_inputs(&sh.sources, &s.state).await;
            picked.clear();
        }
        let change_inputs = inputs.take(InputKind::Change);
        let me = sh.agent().sessions().lookup(sh.dir.sid()).await?;
        let changes = check_changes(
            sh.agent(),
            &cfg,
            &state,
            me.as_ref(),
            &change_inputs,
            sh.deps.options.change_budget,
            sh.deps.options.active_sessions_limit,
            false,
        )
        .await?;
        let mut changes = changes;
        let hook = if !state.bootstrap_done {
            "on_init"
        } else if state.internal_continuation.is_some() {
            "on_behavior_switch"
        } else {
            "on_wakeup"
        };
        let hints = if sh.deps.options.load_hints && (!state.bootstrap_done || !picked.is_empty()) {
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
        let active = sh
            .agent()
            .activity()
            .active(me.as_ref(), sh.deps.options.active_sessions_limit)
            .await
            .unwrap_or_default();
        // The batch message renders the active list: the agent has seen
        // this view.
        changes.receipts.push(ChangeReceipt {
            id: format!("{}@input", ACTIVE_CURSOR),
            subscription: ACTIVE_CURSOR.to_string(),
            cursor: super::hook::active_view(&active),
        });
        let material = InputMaterial {
            hook: hook.to_string(),
            inputs: picked.clone(),
            changes: changes.items.clone(),
            hints,
            active,
            runtime_status: serde_json::to_value(sh.deps.runtime.info().await?).unwrap_or_default(),
            now_ms: crate::now_ms(),
            perceptions: if !state.bootstrap_done && cfg.session.kind == SessionKind::SelfImprove {
                perception_window_records(sh, &cfg).await?
            } else {
                Vec::new()
            },
        };
        // Only msg / event inputs (active), bootstrap and a behavior
        // hand-over make an input batch; semi changes ride along (S-15,
        // A-07).
        let triggered =
            !picked.is_empty() || !state.bootstrap_done || state.internal_continuation.is_some();
        let mut msg = if triggered {
            sh.deps
                .assembler
                .render_input(&cfg, &state, &material)
                .await?
        } else {
            None
        };
        if msg.is_none() && state.internal_continuation.is_some() {
            msg = Some(format!(
                "<session_input hook=\"on_behavior_switch\">Continue with behavior `{}`.</session_input>",
                state.internal_continuation.clone().unwrap_or_default()
            ));
        }
        let resumable = live.as_ref().map(|l| l.ready).unwrap_or(false);
        if msg.is_none() && !resumable {
            // Nothing to infer on. Coalesced-only changes still get consumed.
            if !changes.consumed_only.is_empty() {
                let mut s = sh.session.lock().await;
                for i in &changes.consumed_only {
                    s.state.source_mut(&i.src).mark(i.index);
                }
                let dropped: Vec<WorklogBody> = changes
                    .dropped
                    .iter()
                    .filter(|(c, _)| changes.consumed_only.iter().any(|i| &i.id() == c))
                    .map(|(c, r)| WorklogBody::ChangeDropped {
                        change: c.clone(),
                        reason: r.clone(),
                    })
                    .collect();
                s.append_worklog(&sh.lease, dropped)?;
                commit_and_report(sh, &mut s).await?;
                confirm_inputs(&sh.sources, &s.state).await;
            }
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
            match until {
                StopWhen::Idle => return Ok(DriveResult::Idle { rev, run_state: rs }),
                StopWhen::MaxOutcomes { .. } => {
                    return Ok(DriveResult::OutcomesHandled { rev, run_state: rs })
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
                    let wake = cfg.channels.wake_event.clone();
                    sh.deps
                        .waker
                        .wait(wake.as_deref(), sh.deps.options.poll_interval)
                        .await;
                    inputs = fetch_inputs(sh).await?;
                    apply_controls(sh, &mut inputs, false).await?;
                    if sh.session.lock().await.state.is_finished() {
                        reject_leftovers(sh, &mut inputs).await?;
                        return Ok(finished_result(&*sh.session.lock().await));
                    }
                    continue;
                }
            }
        }
        let mut lc = match live.take() {
            Some(l) => l,
            None if state.live_run.is_some() => open_state_live_run(sh, &env).await?,
            None => new_run_context(sh, &binding, &env).await?,
        };
        if let Some(text) = msg {
            commit_input_batch(sh, &mut lc, &picked, &changes, text, hook).await?;
        }
        lc.ready = false;
        let outcome = run_compacting(sh, &mut lc).await?;
        let next = handle_context_outcome(sh, &mut lc, outcome).await?;
        outcomes_handled += 1;
        if !next.run_ended && !next.suspended {
            live = Some(lc);
        }
        let (rev, rs) = {
            let s = sh.session.lock().await;
            (s.state.rev, s.state.run_state)
        };
        if next.finished {
            return Ok(finished_result(&*sh.session.lock().await));
        }
        if let Some(err) = next.error.clone() {
            return Ok(DriveResult::Error { rev, error: err });
        }
        if next.waiting && until == StopWhen::Idle {
            return Ok(DriveResult::Idle { rev, run_state: rs });
        }
        inputs = fetch_inputs(sh).await?;
        apply_controls(sh, &mut inputs, false).await?;
    }
}
