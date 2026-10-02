//! Inputs of a drive (§8.2): fetching pending deliveries, applying control /
//! perception / malformed inputs, the `decide` command (§6.5) and the
//! cumulative consumption ack.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::channel::{InputSource, Inputs};
use crate::error::{OpenDanError, Result};
use crate::lock::Acquire;
use crate::protocol::*;
use crate::session::Session;

use super::shared::{commit_and_report, Shared};

const FETCH_MAX: usize = 256;

/// Fetch pending inputs of every source, skipping consumed deliveries and
/// duplicate producer keys (`recent_keys`, except coalescing change keys).
pub async fn fetch_inputs(sh: &Shared) -> Result<Inputs> {
    let state = sh.session.lock().await.state.clone();
    let mut out = Inputs::default();
    for src in &sh.sources {
        let progress = state.source(src.id());
        for m in src.fetch(&progress, FETCH_MAX).await? {
            if m.kind != InputKind::Change && !m.key.is_empty() && state.recent_keys.contains(&m.key)
            {
                // A re-delivery of something already consumed: consume the
                // duplicate silently (it never enters the context twice).
                out.items.push(InputMessage {
                    malformed: Some("duplicate key".into()),
                    ..m
                });
                continue;
            }
            out.items.push(m);
        }
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

pub(super) fn reject(s: &mut Session, m: &InputMessage, reason: &str) -> WorklogBody {
    s.state.source_mut(&m.src).mark(m.index);
    WorklogBody::InputRejected {
        input: m.input_ref(),
        reason: reason.to_string(),
    }
}

/// Apply control / perception / malformed inputs (drive start and every
/// observation boundary). `in_run`: a run is executing.
pub async fn apply_controls(sh: &Shared, inputs: &mut Inputs, in_run: bool) -> Result<()> {
    let malformed = inputs.take_malformed();
    let controls = inputs.take(InputKind::Control);
    let perceptions = inputs.take(InputKind::Perception);
    if malformed.is_empty() && controls.is_empty() && perceptions.is_empty() {
        return Ok(());
    }
    let mut s = sh.session.lock().await;
    let mut bodies = Vec::new();
    for m in &malformed {
        let reason = m.malformed.clone().unwrap_or_default();
        if reason == "duplicate key" {
            s.state.source_mut(&m.src).mark(m.index);
        } else {
            bodies.push(reject(&mut s, m, &reason));
        }
    }
    // Perception inputs: append (idempotent by seq) before consuming.
    if !perceptions.is_empty() {
        let sid = sh.dir.sid().to_string();
        let mut recs = Vec::new();
        let mut seq = s.state.perception_seq;
        for p in &perceptions {
            seq += 1;
            recs.push(PerceptionRecord {
                seq,
                at_ms: p.at_ms.max(1),
                session_id: sid.clone(),
                kind: p
                    .payload
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("observation")
                    .to_string(),
                source: "session".into(),
                tags: p
                    .payload
                    .get("tags")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default(),
                objects: p
                    .payload
                    .get("objects")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default(),
                summary: p
                    .payload
                    .get("summary")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| p.text()),
                payload: p.payload.clone(),
                refs: json!({ "input": p.input_ref().id() }),
            });
        }
        sh.agent().perception().append(&sh.lease, &sid, recs).await?;
        s.state.perception_seq = seq;
        for p in &perceptions {
            s.state.source_mut(&p.src).mark(p.index);
            s.state.recent_keys.push(&p.key);
        }
    }
    let finished = s.state.is_finished();
    for m in &controls {
        let Some(cmd) = m.control() else {
            bodies.push(reject(&mut s, m, "unknown control command"));
            continue;
        };
        match &cmd {
            ControlCommand::Stop { reason } => {
                if finished {
                    bodies.push(reject(&mut s, m, "session already finished"));
                    continue;
                }
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
                    detail: serde_json::to_value(&cmd).unwrap_or(Value::Null),
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
                s.write_config(&sh.lease)?;
                bodies.push(WorklogBody::ControlApplied {
                    input: m.input_ref(),
                    command: "unsubscribe".into(),
                    detail: json!({ "id": id }),
                });
            }
            ControlCommand::Decide { decision, by, note } => {
                if !finished || in_run {
                    // Keep it in the queue; show "waiting for the driver".
                    s.state.pending_decision =
                        Some(json!({ "decision": decision, "by": by, "input": m.input_ref().id() }));
                    continue;
                }
                match apply_decide(sh, &mut s, m, decision, by, note.as_deref()).await {
                    Ok(Some(body)) => bodies.push(body),
                    Ok(None) => continue, // artifact lock busy: retry later
                    Err(OpenDanError::InvalidArgument(reason)) => {
                        bodies.push(reject(&mut s, m, &reason))
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        s.state.source_mut(&m.src).mark(m.index);
        s.state.recent_keys.push(&m.key);
    }
    s.append_worklog(&sh.lease, bodies)?;
    commit_and_report(sh, &mut s).await?;
    confirm_inputs(&sh.sources, &s.state).await;
    Ok(())
}

pub(super) async fn apply_decide(
    sh: &Shared,
    s: &mut Session,
    m: &InputMessage,
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

pub(super) async fn reject_leftovers(sh: &Arc<Shared>, inputs: &mut Inputs) -> Result<()> {
    let left: Vec<InputMessage> = std::mem::take(&mut inputs.items);
    if left.is_empty() {
        return Ok(());
    }
    let mut s = sh.session.lock().await;
    let mut bodies = Vec::new();
    for m in &left {
        if m.kind == InputKind::Control {
            continue; // decide inputs wait for a valid state
        }
        bodies.push(reject(&mut s, m, "session is finished"));
    }
    if bodies.is_empty() {
        return Ok(());
    }
    s.append_worklog(&sh.lease, bodies)?;
    commit_and_report(sh, &mut s).await?;
    confirm_inputs(&sh.sources, &s.state).await;
    Ok(())
}
