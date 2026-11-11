//! Inputs of a drive: fetching pending deliveries and routing them
//! mechanically — `type + subscription + waiting state` decide what happens
//! to a record, templates never do:
//!
//! | record | handling |
//! |---|---|
//! | rejected / after finish | consumed, `input_rejected` |
//! | duplicate `key` | consumed silently |
//! | `control` | applied by the driver, in delivery order |
//! | `msg` | candidate of a controlled input batch |
//! | `event`, active subscription | candidate (accepted: its routing is kept) |
//! | `event`, semi subscription | merged into `pending_events`, consumed |
//! | `event` of a task the session follows | consumed; its end is queried |
//! | `event` without a valid subscription | consumed, `event_dropped` |
//!
//! Everything consumed here is committed to state.json before the input
//! sources are confirmed.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::channel::InputSource;
use crate::error::{OpenDanError, Result};
use crate::lock::Acquire;
use crate::protocol::*;
use crate::session::Session;

use super::hook::watched_view;
use super::shared::{commit_and_report, Shared};

const FETCH_MAX: usize = 256;

/// Pending-event slot of a retraction the next input reports.
const RETRACT_SUBSCRIPTION: &str = "_retract";

/// Pending deliveries of every source: sources in declared order, each by
/// delivery index (never by `at_ms` / `created_at_ms`).
pub async fn fetch_inputs(sh: &Shared) -> Result<Vec<FetchedInput>> {
    let state = sh.session.lock().await.state.clone();
    let mut out = Vec::new();
    for src in &sh.sources {
        let progress = state.source(src.id());
        let mut batch = src.fetch(&progress, FETCH_MAX).await?;
        batch.sort_by_key(|m| m.index);
        out.extend(batch);
    }
    Ok(out)
}

/// Confirm the committed consumption positions (cumulative ack). Failures are
/// logged; the next drive retries.
pub async fn confirm_inputs(sources: &[Arc<dyn InputSource>], state: &SessionState) {
    for src in sources {
        let p = state.source(src.id());
        if let Err(e) = src.confirm(&p).await {
            log::warn!("confirm input source {}: {e}", src.id());
        }
    }
}

/// Input sources their service lost (a kmsg queue whose node-local data
/// was wiped while the session directory survived) are created again under
/// their fixed names. A new queue numbers deliveries from 1 again, so the
/// source's consumption progress is reset and committed *before* the queue
/// is created: a crash in between finds the queue still missing and resets
/// again; records a producer posts after the creation are never skipped.
/// Runs after the recovery of runs and receipts (which may still mark
/// indices of the lost queue).
pub(super) async fn restore_sources(sh: &Shared) -> Result<()> {
    for src in &sh.sources {
        if src.exists().await? {
            continue;
        }
        {
            let mut s = sh.session.lock().await;
            let lost = s.state.source(src.id());
            if lost != SourceProgress::default() {
                s.state.inputs.remove(src.id());
                let rev = s.state.rev;
                s.append_worklog(
                    &sh.lease,
                    vec![WorklogBody::ControlApplied {
                        input: InputRef {
                            src: "_runner".into(),
                            index: 0,
                            key: format!("input_source_reset@{rev}"),
                            kind: INPUT_TYPE_CONTROL.into(),
                        },
                        command: "input_source_reset".into(),
                        detail: json!({
                            "src": src.id(),
                            "reason": "input queue missing, recreated",
                            "lost_acked_index": lost.acked_index,
                        }),
                    }],
                )?;
                commit_and_report(sh, &mut s).await?;
            }
        }
        src.recreate().await?;
        log::warn!("session {}: input source {} was missing, recreated", sh.dir.sid(), src.id());
    }
    Ok(())
}

/// The session finished and has no decision left to take: its queues are
/// given back (deleted). A failure only leaves the queue behind.
pub(super) async fn release_sources(sh: &Shared) {
    if sh.session.lock().await.state.takes_input() {
        return;
    }
    for src in &sh.sources {
        match src.release().await {
            Ok(()) => log::info!("session {}: input source {} released", sh.dir.sid(), src.id()),
            Err(e) => log::warn!("session {}: release input source {}: {e}", sh.dir.sid(), src.id()),
        }
    }
}

pub(super) fn reject(
    s: &mut Session,
    m: &FetchedInput,
    reason: RejectReason,
    detail: &str,
) -> WorklogBody {
    s.state.source_mut(&m.src).mark(m.index);
    WorklogBody::InputRejected {
        input: m.input_ref(),
        reason: reason.as_str().to_string(),
        detail: detail.to_string(),
    }
}

/// What one routing pass left for the driver.
#[derive(Debug, Default)]
pub struct Routed {
    /// msg and accepted Input events, in consumption order; still queued.
    pub candidates: Vec<FetchedInput>,
    /// A notification for a task the session follows arrived: query the
    /// task now instead of at the next poll.
    pub task_notified: bool,
}

/// The inputs one batch takes from `candidates` (in order): up to `take`
/// inputs, each with the context messages queued right before it. Context
/// messages are material for what follows them and never form a batch of
/// their own; those without a later input stay queued.
pub fn select_batch(candidates: Vec<FetchedInput>, take: usize) -> Vec<FetchedInput> {
    let mut picked = Vec::new();
    let mut context = Vec::new();
    let mut own = 0;
    for m in candidates {
        if m.msg().is_some_and(|s| s.delivery.context) {
            context.push(m);
            continue;
        }
        if own == take {
            break;
        }
        own += 1;
        picked.append(&mut context);
        picked.push(m);
    }
    picked
}

/// A valid, not yet consumed `stop` is queued (monitor task: looks only,
/// never consumes, confirms or writes state).
pub async fn stop_queued(sh: &Shared) -> bool {
    let Ok(fetched) = fetch_inputs(sh).await else {
        return false;
    };
    let s = sh.session.lock().await;
    fetched.iter().any(|m| {
        matches!(m.control(), Some(ControlCommand::Stop { .. }))
            && !s.state.recent_keys.contains(&m.key)
    })
}

/// An update of the Todo this work session executes is queued and not
/// taken in yet.
pub(super) async fn todo_update_queued(sh: &Shared) -> bool {
    let Ok(fetched) = fetch_inputs(sh).await else {
        return false;
    };
    let s = sh.session.lock().await;
    fetched.iter().any(|m| {
        !s.state.source(&m.src).is_consumed(m.index)
            && !s.state.recent_keys.contains(&m.key)
            && matches!(&m.input, Ok(SessionInput::Event(ev)) if ev.source.kind == crate::todos::TODO_EVENT_SOURCE)
    })
}

/// Changes of subscribed sessions (pull mode: compare the registry rev, not
/// event delivery), merged into `pending_events` like any Observe event.
async fn poll_session_subscriptions(sh: &Shared, s: &mut Session) -> Result<bool> {
    let mut changed = false;
    let subs = s.config.subscriptions.clone();
    for sub in &subs {
        let SubscriptionSource::Session { session_ref } = &sub.source else {
            continue;
        };
        let Some(e) = sh.agent().sessions().lookup(session_ref).await? else {
            continue;
        };
        let cur = s.state.subscription_cursors.get(&sub.id);
        let cur_rev = cur
            .and_then(|c| c.get("rev"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if e.status.rev <= cur_rev {
            continue;
        }
        let view = watched_view(&e.status, &sub.watch);
        if cur.and_then(|c| c.get("view")) == Some(&view) {
            continue;
        }
        let terminal = e.status.run_state == RunState::Finished;
        let mut text = format!("session {} is {}", e.session_id, e.status.run_state.as_str());
        if let Some(o) = e.status.outcome {
            text.push_str(&format!(" ({:?})", o).to_lowercase());
        }
        if !e.status.one_line_status.is_empty() {
            text.push_str(&format!(": {}", e.status.one_line_status));
        }
        if terminal && !e.status.report_brief.is_empty() {
            text.push_str(&format!("\nreport: {}", e.status.report_brief));
        }
        if text.len() > MAX_EVENT_SUMMARY_BYTES {
            let mut end = MAX_EVENT_SUMMARY_BYTES;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
        }
        let ev = AgentEvent {
            subscription_id: Some(sub.id.clone()),
            source: EventSource::new("session", session_ref.clone()),
            event: if terminal { "finished" } else { "updated" }.to_string(),
            seq: Some(e.status.rev),
            summary: text,
            data_ref: None,
            terminal,
            call_id: None,
        };
        let key = format!("session:{session_ref}@{}", e.status.rev);
        merge_pending_event(
            &mut s.state.pending_events,
            Some(&sub.id),
            &ev,
            &key,
            None,
            crate::now_ms(),
        );
        s.state
            .subscription_cursors
            .insert(sub.id.clone(), json!({ "rev": e.status.rev, "view": view }));
        changed = true;
    }
    Ok(changed)
}

/// One routing pass (drive start, every loop iteration, every checkpoint
/// boundary). `in_run`: a run is executing.
pub async fn route_inputs(sh: &Shared, in_run: bool) -> Result<Routed> {
    // A large future (controls, decide, perception): kept off the stack.
    Box::pin(route_inputs_inner(sh, in_run)).await
}

async fn route_inputs_inner(sh: &Shared, in_run: bool) -> Result<Routed> {
    let fetched = fetch_inputs(sh).await?;
    let mut routed = Routed::default();
    let mut s = sh.session.lock().await;
    let mut bodies = Vec::new();
    let mut changed = false;
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let all = fetched.clone();
    for m in fetched {
        if s.state.source(&m.src).is_consumed(m.index) {
            continue;
        }
        let input = match &m.input {
            Ok(i) => i.clone(),
            Err(r) => {
                bodies.push(reject(&mut s, &m, r.reason, &r.detail));
                changed = true;
                continue;
            }
        };
        if s.state.is_finished() && !input.allowed_after_finish() {
            bodies.push(reject(&mut s, &m, RejectReason::SessionFinished, ""));
            changed = true;
            continue;
        }
        // A re-delivery of something already consumed, or the same logical
        // input twice in this pass: it never enters the context twice.
        if s.state.recent_keys.contains(&m.key) || !seen.insert(m.key.clone()) {
            s.state.source_mut(&m.src).mark(m.index);
            changed = true;
            continue;
        }
        match input {
            SessionInput::Control(ControlCommand::Retract { key, reason }) => {
                let queued = all.iter().find(|x| {
                    x.key == key
                        && matches!(x.input, Ok(SessionInput::Msg(_)))
                        && !s.state.source(&x.src).is_consumed(x.index)
                        && !s.state.recent_keys.contains(&x.key)
                });
                let outcome = match queued {
                    Some(target) => {
                        routed.candidates.retain(|c| c.key != key);
                        bodies.push(reject(&mut s, target, RejectReason::Retracted, ""));
                        s.state.recent_keys.push(&target.key);
                        "dropped"
                    }
                    None => match open_turn_of(&s, &key).map(Ok).unwrap_or_else(|| consumed_in_turn(&s, &key))? {
                        Some(turn) if s.state.open_turn.as_ref().is_some_and(|t| t.index == turn) => {
                            s.state.turn_stop_requested = true;
                            clear_interruption(&mut s);
                            // The next input tells the model: the Turn's
                            // steps stay in its history, the input is void.
                            let ev = AgentEvent {
                                subscription_id: Some(RETRACT_SUBSCRIPTION.to_string()),
                                source: EventSource::new("system", "retract"),
                                event: "retracted".to_string(),
                                seq: None,
                                summary: format!("the user retracted the message of turn {turn}: do not act on it. What that turn already did is not undone."),
                                data_ref: None,
                                terminal: false,
                                call_id: None,
                            };
                            merge_pending_event(
                                &mut s.state.pending_events,
                                Some(RETRACT_SUBSCRIPTION),
                                &ev,
                                &m.key,
                                None,
                                crate::now_ms(),
                            );
                            "stopped"
                        }
                        Some(_) => "closed",
                        None => "unknown",
                    },
                };
                bodies.push(WorklogBody::ControlApplied {
                    input: m.input_ref(),
                    command: "retract".into(),
                    detail: json!({
                        "key": key,
                        "reason": reason,
                        "from": m.from,
                        "outcome": outcome,
                        "applied": matches!(outcome, "dropped" | "stopped"),
                    }),
                });
                s.state.source_mut(&m.src).mark(m.index);
                s.state.recent_keys.push(&m.key);
                changed = true;
            }
            SessionInput::Control(cmd) => {
                if Box::pin(apply_control(sh, &mut s, &m, &cmd, in_run, &mut bodies)).await? {
                    s.state.source_mut(&m.src).mark(m.index);
                    s.state.recent_keys.push(&m.key);
                    changed = true;
                }
            }
            SessionInput::Msg(_) => {
                if s.config.session.input_policy == InputPolicy::None && m.src != BOOTSTRAP_SRC {
                    bodies.push(reject(&mut s, &m, RejectReason::InputPolicy, ""));
                    changed = true;
                } else {
                    routed.candidates.push(m);
                }
            }
            SessionInput::Event(ev) => {
                if s.state.source(&m.src).accepted.contains(&m.index) {
                    routed.candidates.push(m);
                    continue;
                }
                // What the plan tells the work session of its Todo (an
                // update) is its input: the queue exists for it.
                if ev.source.kind == crate::todos::TODO_EVENT_SOURCE
                    && s.config
                        .session
                        .origin
                        .as_ref()
                        .is_some_and(|o| o.parent_session.is_some())
                {
                    s.state.source_mut(&m.src).accepted.insert(m.index);
                    routed.candidates.push(m);
                    changed = true;
                    continue;
                }
                // A task the session follows is matched by task id,
                // separately from ordinary subscriptions: the notification
                // only triggers a query; its end comes in as a task result.
                if ev.source.kind == "task" && s.state.watched_tasks.contains(&ev.source.id) {
                    s.state.source_mut(&m.src).mark(m.index);
                    s.state.recent_keys.push(&m.key);
                    bodies.push(WorklogBody::EventDropped {
                        input: m.input_ref(),
                        reason: "watched_task".into(),
                    });
                    routed.task_notified = true;
                    changed = true;
                    continue;
                }
                let sub = s.config.subscription_for(
                    ev.subscription_id.as_deref(),
                    &ev.source.kind,
                    &ev.source.id,
                    &ev.event,
                );
                match sub {
                    None => {
                        s.state.source_mut(&m.src).mark(m.index);
                        s.state.recent_keys.push(&m.key);
                        bodies.push(WorklogBody::EventDropped {
                            input: m.input_ref(),
                            reason: "unsubscribed".into(),
                        });
                        changed = true;
                    }
                    Some(sub) if sub.mode == SubscriptionMode::Active => {
                        if s.config.session.input_policy == InputPolicy::None {
                            bodies.push(reject(&mut s, &m, RejectReason::InputPolicy, ""));
                        } else {
                            s.state.source_mut(&m.src).accepted.insert(m.index);
                            routed.candidates.push(m);
                        }
                        changed = true;
                    }
                    Some(sub) => {
                        // Saved first, consumed in the same commit: "received"
                        // is not "seen by the LLM".
                        merge_pending_event(
                            &mut s.state.pending_events,
                            Some(&sub.id),
                            &ev,
                            &m.key,
                            Some(InputPos {
                                src: m.src.clone(),
                                index: m.index,
                            }),
                            crate::now_ms(),
                        );
                        s.state.source_mut(&m.src).mark(m.index);
                        s.state.recent_keys.push(&m.key);
                        changed = true;
                    }
                }
            }
        }
    }
    if !s.state.is_finished() && poll_session_subscriptions(sh, &mut s).await? {
        changed = true;
    }
    if changed {
        s.append_worklog(&sh.lease, bodies)?;
        commit_and_report(sh, &mut s).await?;
        confirm_inputs(&sh.sources, &s.state).await;
    }
    Ok(routed)
}

/// The open Turn, when the msg `key` is among its inputs.
fn open_turn_of(s: &Session, key: &str) -> Option<Option<u64>> {
    s.state
        .open_turn
        .as_ref()
        .filter(|t| t.msg_keys.iter().any(|k| k == key))
        .map(|t| Some(t.index))
}

/// The Turn that consumed the msg `key` (read backwards from the worklog).
fn consumed_in_turn(s: &Session, key: &str) -> Result<Option<u64>> {
    let mut r = s.dir.worklog().reverse(s.state.worklog.committed_bytes, 0)?;
    while let Some((_, e)) = r.next_json::<WorklogEntry>()? {
        if let WorklogBody::TurnStarted { turn, inputs, .. } | WorklogBody::InputBatch { turn, inputs, .. } = &e.body {
            if inputs.iter().any(|i| i.key == key) {
                return Ok(Some(*turn));
            }
        }
    }
    Ok(None)
}

/// Stopping the Turn a restart interrupted ends the interruption too: the
/// session waits for its next input instead of a `resume`.
pub(super) fn clear_interruption(s: &mut Session) {
    if s.state.paused.as_ref().is_some_and(|p| p.source == PauseSource::Interrupted) {
        s.state.paused = None;
        s.state.resume_run = false;
    }
}

/// Reason of the pause a session hands down to its sub sessions.
fn parent_pause_reason(sid: &str) -> String {
    format!("parent session {sid} was paused")
}

/// Why a pause with `reason` reaches this session.
fn pause_source(cfg: &SessionConfig, reason: Option<&str>) -> PauseSource {
    let origin = cfg.session.origin.as_ref();
    let parent = origin.and_then(|o| o.parent_session.as_deref());
    let goal = origin.and_then(|o| o.goal.as_ref());
    match reason {
        Some(r) if parent.is_some_and(|p| r == parent_pause_reason(p)) => PauseSource::Parent,
        Some(r) if goal.is_some_and(|g| r == crate::goal::dormant_reason(&g.goal_id)) => PauseSource::Goal,
        Some(r) if goal.is_some_and(|g| r == crate::goal::budget_pause_reason(&g.goal_id)) => PauseSource::Budget,
        _ => PauseSource::User,
    }
}

/// A pause reaches the sub sessions that are not finished (those with a
/// queue). A resume reaches those this session's pause paused — and, when
/// this session was interrupted by a restart, those the same restart
/// interrupted.
async fn cascade_pause(sh: &Shared, sid: &str, key: &str, pause: bool, was: Option<PauseSource>) {
    let Ok(children) = sh.agent().sessions().children_of(&[sid.to_string()]).await else {
        return;
    };
    let reason = parent_pause_reason(sid);
    for e in children {
        if e.status.run_state == RunState::Finished || e.input_queue.is_none() {
            continue;
        }
        let command = if pause {
            ControlCommand::Pause { reason: Some(reason.clone()) }
        } else {
            let state = crate::session::SessionDir::open(&e.location).and_then(|d| d.state()).ok();
            let mine = match state.and_then(|st| st.paused) {
                Some(p) if p.source == PauseSource::Interrupted => was == Some(PauseSource::Interrupted),
                Some(p) => p.reason.as_deref() == Some(reason.as_str()),
                None => false,
            };
            if !mine {
                continue;
            }
            ControlCommand::Resume { reason: Some(format!("parent session {sid} was resumed")) }
        };
        let input = PostedInput::control(&sh.deps.who, format!("{}:parent:{key}", command.name()), command);
        if let Err(err) = sh.agent().sessions().post_input(&e.session_id, &input).await {
            log::warn!("cannot {} sub session {}: {err}", input.input.type_name(), e.session_id);
        }
    }
}

/// Apply one control command. `true`: consumed; `false`: it stays queued
/// (a `decide` that cannot be applied yet).
async fn apply_control(
    sh: &Shared,
    s: &mut Session,
    m: &FetchedInput,
    cmd: &ControlCommand,
    in_run: bool,
    bodies: &mut Vec<WorklogBody>,
) -> Result<bool> {
    let finished = s.state.is_finished();
    match cmd {
        ControlCommand::Stop { reason, scope } => {
            let kind = s.config.session.kind;
            let mut refused = None;
            let applied = match scope {
                StopScope::Turn => {
                    let open = s.state.open_turn.is_some();
                    if open {
                        s.state.turn_stop_requested = true;
                        clear_interruption(s);
                    }
                    open
                }
                // A UI session is never stopped: it ends when it is archived.
                StopScope::Session if kind == SessionKind::Ui => {
                    refused = Some("a UI session is not stopped: stop the message in progress (scope turn), or start a new conversation to drop its context".to_string());
                    false
                }
                // A Goal Session ends with its Goal: stopping it puts the
                // Goal to sleep.
                StopScope::Session if kind == SessionKind::Goal => {
                    match super::goal::sleep(sh, s, &m.from, reason.as_deref()).await {
                        Ok(()) => true,
                        Err(e) => {
                            refused = Some(e.to_string());
                            false
                        }
                    }
                }
                StopScope::Archive if kind != SessionKind::Ui => {
                    refused = Some("only a UI session is archived".to_string());
                    false
                }
                StopScope::Session | StopScope::Archive => {
                    s.state.stop_requested = true;
                    true
                }
            };
            bodies.push(WorklogBody::ControlApplied {
                input: m.input_ref(),
                command: "stop".into(),
                detail: json!({
                    "reason": reason,
                    "from": m.from,
                    "scope": scope,
                    "applied": applied,
                    "refused": refused,
                }),
            });
        }
        ControlCommand::Pause { reason } => {
            let applied = !finished && s.state.paused.is_none();
            if applied {
                s.state.paused = Some(Paused {
                    source: pause_source(&s.config, reason.as_deref()),
                    at_ms: crate::now_ms(),
                    reason: reason.clone(),
                    by: m.from.clone(),
                    in_run: false,
                });
                s.state.resume_run = false;
                super::turn_task::note_live_line(s, None, None);
            }
            bodies.push(WorklogBody::ControlApplied {
                input: m.input_ref(),
                command: "pause".into(),
                detail: json!({ "reason": reason, "from": m.from, "applied": applied }),
            });
            if applied {
                cascade_pause(sh, s.sid(), &m.key, true, None).await;
            }
        }
        ControlCommand::Resume { reason } => {
            let paused = s.state.paused.take();
            let applied = paused.is_some();
            let was = paused.as_ref().map(|p| p.source);
            if paused.is_some_and(|p| p.in_run) {
                s.state.resume_run = true;
            }
            bodies.push(WorklogBody::ControlApplied {
                input: m.input_ref(),
                command: "resume".into(),
                detail: json!({ "reason": reason, "from": m.from, "applied": applied }),
            });
            if applied {
                super::turn_task::note_live_line(s, None, None);
                cascade_pause(sh, s.sid(), &m.key, false, was).await;
            }
        }
        ControlCommand::Retract { .. } => {}
        ControlCommand::GoalSkills { add, remove } => {
            match super::goal::apply_skills_control(&mut s.state, add, remove) {
                Ok(()) => bodies.push(WorklogBody::ControlApplied {
                    input: m.input_ref(),
                    command: "goal_skills".into(),
                    detail: serde_json::to_value(cmd).unwrap_or(Value::Null),
                }),
                Err(reason) => bodies.push(WorklogBody::InputRejected {
                    input: m.input_ref(),
                    reason: RejectReason::InvalidPayload.as_str().into(),
                    detail: reason,
                }),
            }
        }
        ControlCommand::Activity {
            summary,
            touch,
            clear,
        } => {
            if *clear {
                s.state.activity.touching.clear();
            }
            if let Some(t) = summary {
                s.state.activity.summary = t.clone();
            }
            for t in touch {
                s.state.activity.touch(t.clone());
            }
            s.state.activity.heartbeat_ms = crate::now_ms();
            bodies.push(WorklogBody::ControlApplied {
                input: m.input_ref(),
                command: "activity".into(),
                detail: serde_json::to_value(cmd).unwrap_or(Value::Null),
            });
        }
        ControlCommand::Subscribe { subscription } => {
            s.config.subscriptions.retain(|x| x.id != subscription.id);
            s.config.subscriptions.push(subscription.clone());
            s.write_config(&sh.lease)?;
            bodies.push(WorklogBody::ControlApplied {
                input: m.input_ref(),
                command: "subscribe".into(),
                detail: json!({ "id": subscription.id }),
            });
        }
        ControlCommand::Unsubscribe { id } => {
            s.config.subscriptions.retain(|x| &x.id != id);
            s.state.subscription_cursors.remove(id);
            // State of this subscription that was not injected yet goes with
            // it; events it accepted before stay accepted.
            s.state
                .pending_events
                .retain(|p| p.subscription_id.as_deref() != Some(id.as_str()));
            s.write_config(&sh.lease)?;
            bodies.push(WorklogBody::ControlApplied {
                input: m.input_ref(),
                command: "unsubscribe".into(),
                detail: json!({ "id": id }),
            });
        }
        ControlCommand::Perceive {
            kind,
            summary,
            tags,
            objects,
            subjects,
            cites,
            source,
            occurred_at,
            suggested_kind,
            memory_intent,
            mentions,
        } => {
            // Recorded through the Memory facade (idempotent by the input
            // key) before the input is consumed; a refused observation is a
            // rejected input, never a silent drop.
            let args = super::consolidation::PerceiveArgs {
                kind,
                summary,
                tags,
                objects,
                subjects,
                cites,
                source: source.as_deref(),
                occurred_at: occurred_at.as_deref(),
                suggested_kind: suggested_kind.as_deref(),
                memory_intent: memory_intent.as_deref(),
                mentions,
            };
            let input_id = m.input_ref().id();
            match super::consolidation::perceive(sh, s, &m.key, &input_id, args).await {
                Ok(receipt) => bodies.push(WorklogBody::ControlApplied {
                    input: m.input_ref(),
                    command: "perceive".into(),
                    detail: serde_json::to_value(&receipt).unwrap_or(Value::Null),
                }),
                Err(OpenDanError::InvalidArgument(reason)) => {
                    bodies.push(WorklogBody::InputRejected {
                        input: m.input_ref(),
                        reason: RejectReason::InvalidPayload.as_str().into(),
                        detail: reason,
                    });
                }
                Err(e) => return Err(e),
            }
        }
        ControlCommand::Decide { decision, by, note } => {
            if !finished || in_run {
                // Keep it in the queue; show "waiting for the driver".
                s.state.pending_decision =
                    Some(json!({ "decision": decision, "by": by, "input": m.input_ref().id() }));
                return Ok(false);
            }
            match apply_decide(sh, s, m, decision, by, note.as_deref()).await {
                Ok(Some(body)) => bodies.push(body),
                Ok(None) => return Ok(false), // artifact lock busy: retry later
                Err(OpenDanError::InvalidArgument(reason)) => {
                    bodies.push(WorklogBody::InputRejected {
                        input: m.input_ref(),
                        reason: RejectReason::InvalidPayload.as_str().into(),
                        detail: reason,
                    });
                }
                Err(e) => return Err(e),
            }
        }
    }
    Ok(true)
}

pub(super) async fn apply_decide(
    sh: &Shared,
    s: &mut Session,
    m: &FetchedInput,
    decision: &str,
    by: &str,
    note: Option<&str>,
) -> Result<Option<WorklogBody>> {
    if s.config.session.kind != SessionKind::Work {
        return Err(OpenDanError::InvalidArgument(
            "decide only applies to work sessions".into(),
        ));
    }
    let valid = match decision {
        "accept" => s.state.acceptance == Acceptance::Pending,
        "discard" => matches!(
            s.state.acceptance,
            Acceptance::Pending | Acceptance::Accepted
        ),
        _ => false,
    };
    if !valid {
        return Err(OpenDanError::InvalidArgument(format!(
            "decide `{decision}` is not valid while acceptance is {:?}",
            s.state.acceptance
        )));
    }
    let sid = s.sid().to_string();
    let mut version: Option<ArtifactVersion> = None;
    let mut head_after: Option<Option<String>> = None;
    if let Some(aid) = s.config.artifact_id.clone() {
        let resource = format!("artifact:{aid}");
        let lease = match sh.agent().locks().acquire(&resource, sh.deps.holder())? {
            Acquire::Acquired(l) => l,
            Acquire::Busy(_) => return Ok(None),
        };
        let ver = format!("v-{sid}");
        match sh.agent().artifacts().version(&aid, &ver).await? {
            Some(_) => {
                let r = sh
                    .agent()
                    .artifacts()
                    .decide(&lease, &aid, &ver, decision)
                    .await?;
                head_after = Some(r.head_after.clone());
                version = Some(r.version);
            }
            None => log::warn!("session {sid} has no version of artifact {aid}"),
        }
        lease.release();
    }
    let mut report = Value::Null;
    if decision == "discard" {
        let unsupported = match &version {
            Some(v) => v.side_effects.clone(),
            None => side_effects_from_worklog(s)?,
        };
        let workspace = match &s.config.workspace {
            None => "none",
            // Workspace rollback is designed separately (Q13): nothing is
            // reverted here; every change is reported as not undone.
            Some(_) => "unsupported",
        };
        let r = DiscardReport {
            reverted: Vec::new(),
            workspace: workspace.into(),
            unsupported,
            head_moved_to: head_after.clone(),
            note: note.map(str::to_string),
        };
        report = serde_json::to_value(&r).unwrap_or(Value::Null);
        s.state.acceptance = Acceptance::Discarded;
        let mut result = s.state.result.clone().unwrap_or_else(|| json!({}));
        result["discard_report"] = report.clone();
        s.state.result = Some(result);
    } else {
        s.state.acceptance = Acceptance::Accepted;
    }
    s.state.pending_decision = None;
    let body = WorklogBody::Decide {
        decision: decision.to_string(),
        by: if by.is_empty() { m.from.clone() } else { by.to_string() },
        report,
    };
    // A discarded task leaves no Runtime perception: the decision is in the
    // worklog and the Task object.
    Ok(Some(body))
}

/// Side-effect actions of this session, read backwards from the worklog.
pub(crate) fn side_effects_from_worklog(s: &Session) -> Result<Vec<SideEffectRef>> {
    let mut out = Vec::new();
    let mut r = s
        .dir
        .worklog()
        .reverse(s.state.worklog.committed_bytes, 0)?;
    while let Some((_, e)) = r.next_json::<WorklogEntry>()? {
        let calls = match e.body {
            WorklogBody::Step { actions, .. } => actions,
            WorklogBody::AssistantMessage { tool_calls, .. } => tool_calls,
            _ => continue,
        };
        for a in calls {
            if a.effect != "read_only" {
                out.push(SideEffectRef {
                    call_id: a.call_id,
                    tool: a.tool.clone(),
                    note: format!("{} cannot be undone by the session", a.tool),
                });
            }
        }
    }
    out.reverse();
    Ok(out)
}


#[cfg(test)]
mod tests {
    use super::*;

    fn fetched(index: u64, context: bool) -> FetchedInput {
        let msg = text_msg(
            &parse_did("did:bns:bob").unwrap(),
            &parse_did("did:bns:jarvis").unwrap(),
            format!("message {index}"),
        );
        let posted = PostedInput::msg(
            "app:jarvis@bob",
            msg,
            MsgDelivery {
                context,
                ..Default::default()
            },
        )
        .unwrap();
        FetchedInput {
            src: "q".into(),
            index,
            kind: INPUT_TYPE_MSG.into(),
            key: posted.key,
            from: posted.from,
            at_ms: posted.at_ms,
            input: Ok(posted.input),
        }
    }

    fn indices(picked: &[FetchedInput]) -> Vec<u64> {
        picked.iter().map(|m| m.index).collect()
    }

    #[test]
    fn context_messages_ride_with_the_input_after_them() {
        // Twenty context messages and the request: one batch, whatever the
        // batch budget.
        let mut all: Vec<FetchedInput> = (1..=20).map(|i| fetched(i, true)).collect();
        all.push(fetched(21, false));
        assert_eq!(indices(&select_batch(all.clone(), 16)), (1..=21).collect::<Vec<_>>());
        assert_eq!(indices(&select_batch(all, 1)).len(), 21);
        // Only part of them queued yet: nothing is taken.
        let partial: Vec<FetchedInput> = (1..=7).map(|i| fetched(i, true)).collect();
        assert!(select_batch(partial, 16).is_empty());
        // Context after the last input taken waits for its own request.
        let mixed = vec![fetched(1, false), fetched(2, true), fetched(3, false), fetched(4, true)];
        assert_eq!(indices(&select_batch(mixed.clone(), 16)), vec![1, 2, 3]);
        assert_eq!(indices(&select_batch(mixed, 1)), vec![1]);
    }
}
