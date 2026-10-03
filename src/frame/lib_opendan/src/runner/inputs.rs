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
//! | `event` of a suspended call's task | consumed; only wakes the wait |
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
    /// A notification for a task a suspended call waits for arrived: query
    /// the task now instead of at the next poll.
    pub task_notified: bool,
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
/// boundary). `in_run`: a run is executing. `pending_tasks`: task ids the
/// suspended calls of the live run wait for.
pub async fn route_inputs(sh: &Shared, in_run: bool, pending_tasks: &[String]) -> Result<Routed> {
    // A large future (controls, decide, perception): kept off the stack.
    Box::pin(route_inputs_inner(sh, in_run, pending_tasks)).await
}

async fn route_inputs_inner(
    sh: &Shared,
    in_run: bool,
    pending_tasks: &[String],
) -> Result<Routed> {
    let fetched = fetch_inputs(sh).await?;
    let mut routed = Routed::default();
    let mut s = sh.session.lock().await;
    let mut bodies = Vec::new();
    let mut changed = false;
    let mut seen: BTreeSet<String> = BTreeSet::new();
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
            SessionInput::Control(cmd) => {
                if Box::pin(apply_control(sh, &mut s, &m, &cmd, in_run, &mut bodies)).await? {
                    s.state.source_mut(&m.src).mark(m.index);
                    s.state.recent_keys.push(&m.key);
                    changed = true;
                }
            }
            SessionInput::Msg(_) => {
                if s.config.session.input_policy == InputPolicy::None {
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
                // The result dependency of a suspended call is matched by
                // task id, separately from ordinary subscriptions: the
                // notification only triggers a query, the result is filled
                // as the call's tool result.
                if ev.source.kind == "task" && pending_tasks.contains(&ev.source.id) {
                    s.state.source_mut(&m.src).mark(m.index);
                    s.state.recent_keys.push(&m.key);
                    bodies.push(WorklogBody::EventDropped {
                        input: m.input_ref(),
                        reason: "pending_call".into(),
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
        ControlCommand::Stop { reason } => {
            s.state.stop_requested = true;
            bodies.push(WorklogBody::ControlApplied {
                input: m.input_ref(),
                command: "stop".into(),
                detail: json!({ "reason": reason, "from": m.from }),
            });
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
        } => {
            let sid = sh.dir.sid().to_string();
            let seq = s.state.perception_seq + 1;
            let rec = PerceptionRecord {
                seq,
                at_ms: m.at_ms.max(1),
                session_id: sid.clone(),
                kind: kind.clone(),
                source: "session".into(),
                tags: tags.clone(),
                objects: objects.clone(),
                summary: summary.clone(),
                payload: serde_json::to_value(cmd).unwrap_or(Value::Null),
                refs: json!({ "input": m.input_ref().id() }),
            };
            // Appended (idempotent by seq) before the input is consumed.
            sh.agent().perception().append(&sh.lease, &sid, vec![rec]).await?;
            s.state.perception_seq = seq;
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
    if decision == "discard" {
        // Appended after the commit by the caller's commit; perception after.
        let seq = s.state.perception_seq + 1;
        s.state.perception_seq = seq;
        let rec = PerceptionRecord {
            seq,
            at_ms: crate::now_ms(),
            session_id: sid.clone(),
            kind: "task_discarded".into(),
            source: "session".into(),
            tags: s.state.topic.tags.clone(),
            objects: s
                .config
                .artifact_id
                .iter()
                .map(|a| format!("artifact:{a}"))
                .collect(),
            summary: format!("work session {sid} was discarded"),
            payload: json!({ "session": sid, "artifact_version": version.as_ref().map(|v| v.ver.clone()) }),
            refs: Value::Null,
        };
        if let Err(e) = sh.agent().perception().append(&sh.lease, &sid, vec![rec]).await {
            log::warn!("task_discarded perception of {sid}: {e}");
        }
    }
    Ok(Some(body))
}

/// Side-effect actions of this session, read backwards from the worklog.
pub(super) fn side_effects_from_worklog(s: &Session) -> Result<Vec<SideEffectRef>> {
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

