//! Observation boundaries (§8.4) and the checkpoint hook (§8.3 / §8.5).
//!
//! The hook runs before every inference with the outer snapshot:
//! - no pending batch: publish the snapshot with the tool results it now
//!   contains (clears the in-flight markers it covers), apply control inputs
//!   (stop / activity / subscriptions / perception), refresh the activity
//!   heartbeat, and look for changes to inject;
//! - changes found: hand the waist an injection whose host metadata already
//!   carries the batch receipt (message and receipt land in one snapshot);
//! - called again with the injected state: ① snapshot fsync ② run.json with
//!   the host commit gate ③ state.json ④ clear the gate ⑤ confirm inputs —
//!   only then may the inference start.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use buckyos_api::{AiMessage, AiRole};
use llm_context::deps::{CheckpointHook, Injection};
use llm_context::state::LLMContextSnapshot;
use serde_json::{json, Value};

use crate::error::Result;
use crate::protocol::*;
use crate::session::runs::RunHandle;
use crate::state::{declared_refs, render_active_sessions, AgentStateClient};

use super::assembler::{ChangeItem, TurnMaterial};
use super::drive::{apply_controls, confirm_inputs, fetch_inputs, report, Shared};
use super::receipts::{apply_receipt, predict_position, snapshot_host_meta, with_host_meta};

/// Cursor key of the active-session set subscription.
pub const ACTIVE_CURSOR: &str = "_active_sessions";

/// Result of a change check.
#[derive(Debug, Default)]
pub struct Changes {
    pub items: Vec<ChangeItem>,
    pub receipts: Vec<ChangeReceipt>,
    /// Queue change inputs rendered into the message.
    pub injected_inputs: Vec<InputRef>,
    /// Queue change inputs consumed without rendering (coalesced).
    pub consumed_only: Vec<InputRef>,
    pub dropped: Vec<(String, String)>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty() && self.consumed_only.is_empty()
    }
}

fn watched_view(status: &SessionStatus, watch: &[String]) -> Value {
    let full = json!({
        "run_state": status.run_state,
        "outcome": status.outcome,
        "acceptance": status.acceptance,
        "one_line_status": status.one_line_status,
    });
    let mut out = serde_json::Map::new();
    let fields: Vec<String> = if watch.is_empty() {
        vec![
            "run_state".into(),
            "outcome".into(),
            "acceptance".into(),
            "one_line_status".into(),
        ]
    } else {
        watch.to_vec()
    };
    for f in fields {
        out.insert(f.clone(), full.get(&f).cloned().unwrap_or(Value::Null));
    }
    Value::Object(out)
}

/// Pull-mode changes of subscribed sessions, push-mode `change` inputs
/// (coalesced by key; terminal keys separate) and the active session set.
pub async fn check_changes(
    agent: &dyn AgentStateClient,
    cfg: &SessionConfig,
    state: &SessionState,
    me: Option<&RegistryEntry>,
    change_inputs: &[InputMessage],
    budget: usize,
    active_limit: usize,
    include_active: bool,
) -> Result<Changes> {
    let mut out = Changes::default();
    let mut candidates: Vec<(ChangeItem, Option<ChangeReceipt>, Option<InputRef>)> = Vec::new();
    // 1. subscribed sessions (compare registry rev, not event delivery).
    for sub in &cfg.subscriptions {
        let SubscriptionSource::Session { session_ref } = &sub.source else {
            continue;
        };
        let Some(e) = agent.sessions().lookup(session_ref).await? else {
            continue;
        };
        let cur = state.subscription_cursors.get(&sub.id);
        let cur_rev = cur.and_then(|c| c.get("rev")).and_then(Value::as_u64).unwrap_or(0);
        if e.status.rev <= cur_rev {
            continue;
        }
        let view = watched_view(&e.status, &sub.watch);
        let changed = cur.and_then(|c| c.get("view")) != Some(&view);
        if !changed {
            continue;
        }
        let terminal = e.status.run_state == RunState::Finished;
        let mut text = format!(
            "session {} is {}",
            e.session_id,
            e.status.run_state.as_str()
        );
        if let Some(o) = e.status.outcome {
            text.push_str(&format!(" ({:?})", o).to_lowercase());
        }
        if !e.status.one_line_status.is_empty() {
            text.push_str(&format!(": {}", e.status.one_line_status));
        }
        if terminal && !e.status.report_brief.is_empty() {
            text.push_str(&format!("\nreport: {}", e.status.report_brief));
        }
        let id = format!("{}@{}", sub.id, e.status.rev);
        candidates.push((
            ChangeItem {
                id: id.clone(),
                text,
                terminal,
            },
            Some(ChangeReceipt {
                id,
                subscription: sub.id.clone(),
                cursor: json!({ "rev": e.status.rev, "view": view }),
            }),
            None,
        ));
    }
    // 2. queue changes: the newest per key wins; `#terminal` keys never merge
    //    with progress keys.
    let mut latest: BTreeMap<String, &InputMessage> = BTreeMap::new();
    for m in change_inputs {
        match latest.get(&m.key) {
            Some(prev) if prev.index > m.index => {
                out.consumed_only.push(m.input_ref());
                out.dropped
                    .push((m.input_ref().id(), "superseded by a newer change".into()));
            }
            Some(prev) => {
                out.consumed_only.push(prev.input_ref());
                out.dropped
                    .push((prev.input_ref().id(), "superseded by a newer change".into()));
                latest.insert(m.key.clone(), m);
            }
            None => {
                latest.insert(m.key.clone(), m);
            }
        }
    }
    for (key, m) in latest {
        let terminal = key.ends_with("#terminal")
            || m.payload.get("terminal").and_then(Value::as_bool) == Some(true);
        let sub = m
            .payload
            .get("subscription")
            .and_then(Value::as_str)
            .map(str::to_string);
        let receipt = sub.map(|s| ChangeReceipt {
            id: m.input_ref().id(),
            subscription: s,
            cursor: json!({ "key": key, "version": m.payload.get("version").cloned().unwrap_or(Value::Null) }),
        });
        candidates.push((
            ChangeItem {
                id: m.input_ref().id(),
                text: m.text(),
                terminal,
            },
            receipt,
            Some(m.input_ref()),
        ));
    }
    // 3. active session set (default semi-subscription, §6.7). A turn
    //    message renders the full list itself; boundaries inject changes.
    let active = if include_active {
        agent.activity().active(me, active_limit).await?
    } else {
        Vec::new()
    };
    let view = active_view(&active);
    let cur = state.subscription_cursors.get(ACTIVE_CURSOR);
    if include_active && state.bootstrap_done && cur != Some(&view) {
        let text = render_active_sessions(&active)
            .unwrap_or_else(|| "no other active sessions".to_string());
        let id = format!("{ACTIVE_CURSOR}@{}", crate::ids::h(&[&view.to_string()])[..8].to_string());
        candidates.push((
            ChangeItem {
                id: id.clone(),
                text,
                terminal: false,
            },
            Some(ChangeReceipt {
                id,
                subscription: ACTIVE_CURSOR.to_string(),
                cursor: view,
            }),
            None,
        ));
    }
    // Terminal first; trim to budget (trimmed queue inputs stay unconsumed).
    candidates.sort_by_key(|(c, _, _)| !c.terminal);
    for (i, (item, receipt, input)) in candidates.into_iter().enumerate() {
        if i >= budget {
            out.dropped
                .push((item.id.clone(), "change budget exceeded; deferred".into()));
            continue;
        }
        if let Some(r) = receipt {
            out.receipts.push(r);
        }
        if let Some(inp) = input {
            out.injected_inputs.push(inp);
        }
        out.items.push(item);
    }
    Ok(out)
}

/// Cursor value of an active-session list (what the agent has seen).
pub fn active_view(active: &[crate::state::ActiveSession]) -> Value {
    json!(active
        .iter()
        .map(|a| json!({"id": a.session_id, "state": a.run_state, "overlap": a.overlap}))
        .collect::<Vec<_>>())
}

/// The per-run checkpoint hook.
pub struct SessionCheckpointHook {
    shared: Arc<Shared>,
    run: RunHandle,
    behavior: bool,
    pending: Mutex<Option<(InputReceipt, Vec<(String, String)>)>>,
}

impl SessionCheckpointHook {
    pub fn new(shared: Arc<Shared>, run: RunHandle, behavior: bool) -> Self {
        Self {
            shared,
            run,
            behavior,
            pending: Mutex::new(None),
        }
    }

    async fn commit_pending(
        &self,
        snapshot: &LLMContextSnapshot,
        receipt: InputReceipt,
        dropped: Vec<(String, String)>,
    ) -> Result<()> {
        let sh = &self.shared;
        // The receipt must be the one inside the snapshot.
        let meta = snapshot_host_meta(snapshot);
        if !meta
            .input_receipts
            .iter()
            .any(|r| r.input_seq == receipt.input_seq)
        {
            return Err(crate::error::OpenDanError::Other(
                "injected receipt missing from the snapshot".into(),
            ));
        }
        self.run
            .publish_input_checkpoint(snapshot, receipt.input_seq)?; // ① ②
        crate::fault::point("hook:after_input_checkpoint");
        {
            let mut s = sh.session.lock().await;
            apply_receipt(&mut s.state, &receipt)?;
            let bodies: Vec<WorklogBody> = dropped
                .into_iter()
                .map(|(change, reason)| WorklogBody::ChangeDropped { change, reason })
                .collect();
            s.append_worklog(&sh.lease, bodies)?;
            s.commit_state(&sh.lease)?; // ③
            self.run.complete_host_commit()?; // ④
            confirm_inputs(&sh.sources, &s.state).await; // ⑤
        }
        Ok(())
    }

    async fn boundary(&self, snapshot: &LLMContextSnapshot) -> Result<Option<Injection>> {
        let sh = &self.shared;
        sh.lease.check()?;
        // Tool results first: publish and clear covered in-flight actions.
        self.run.checkpoint_with_results(snapshot, None)?;
        // Controls / perception / malformed inputs.
        let mut inputs = fetch_inputs(sh).await?;
        apply_controls(sh, &mut inputs, true).await?;
        let (stop, state_snapshot, cfg) = {
            let s = sh.session.lock().await;
            (s.state.stop_requested, s.state.clone(), s.config.clone())
        };
        if stop {
            if let Some(h) = sh.interrupt.lock().expect("interrupt lock").as_ref() {
                h.interrupt("session stop requested");
            }
            return Ok(None);
        }
        self.heartbeat().await?;
        // Changes to inject at this boundary.
        let change_inputs: Vec<InputMessage> = inputs
            .items
            .iter()
            .filter(|m| m.kind == InputKind::Change)
            .cloned()
            .collect();
        let me = sh.agent().sessions().lookup(&cfg.session.session_id).await?;
        let changes = check_changes(
            sh.agent(),
            &cfg,
            &state_snapshot,
            me.as_ref(),
            &change_inputs,
            sh.deps.options.change_budget,
            sh.deps.options.active_sessions_limit,
            true,
        )
        .await?;
        if changes.is_empty() {
            return Ok(None);
        }
        let material = TurnMaterial {
            hook: "observation".into(),
            changes: changes.items.clone(),
            now_ms: crate::now_ms(),
            ..Default::default()
        };
        let text = sh.deps.assembler.render_observation(&material).await?;
        let seq = state_snapshot
            .live_run
            .as_ref()
            .filter(|l| l.run_id == self.run.run_id())
            .map(|l| l.applied_input_seq)
            .unwrap_or(0)
            + 1;
        let receipt = InputReceipt {
            run_id: self.run.run_id().to_string(),
            input_seq: seq,
            round: state_snapshot.round,
            opens_round: false,
            hook: Some("observation".into()),
            inputs: changes.injected_inputs.clone(),
            changes: changes.receipts.clone(),
            consumed_only: changes.consumed_only.clone(),
            message_pos: if text.is_some() {
                predict_position(snapshot, self.behavior)
            } else {
                MessagePos::None
            },
            content: text.clone().unwrap_or_default(),
            bootstrap: false,
            after_step: snapshot.state.next_step_index,
            extra: Default::default(),
            at_ms: crate::now_ms(),
        };
        let mut meta = snapshot_host_meta(snapshot);
        meta.input_receipts.push(receipt.clone());
        let host = with_host_meta(snapshot.state.host.as_ref(), &meta);
        *self.pending.lock().expect("pending lock") = Some((receipt, changes.dropped));
        Ok(Some(Injection {
            messages: text
                .map(|t| vec![AiMessage::text(AiRole::User, t)])
                .unwrap_or_default(),
            host: Some(host),
        }))
    }

    /// Merge inferred touching, refresh the heartbeat (throttled commit).
    async fn heartbeat(&self) -> Result<()> {
        let sh = &self.shared;
        let touched: Vec<Touching> = std::mem::take(&mut *sh.touched.lock().expect("touched"));
        let now = crate::now_ms();
        let mut s = sh.session.lock().await;
        let before = s.state.activity.touching.clone();
        for t in touched {
            s.state.activity.touch(t);
        }
        let changed = s.state.activity.touching != before;
        let due = now.saturating_sub(s.state.activity.heartbeat_ms)
            >= sh.deps.options.heartbeat_interval.as_millis() as u64;
        if changed || due {
            s.state.activity.heartbeat_ms = now;
            s.commit_state(&sh.lease)?;
            let status = s.status(sh.lease.epoch());
            drop(s);
            report(sh, status).await;
        }
        Ok(())
    }
}

#[async_trait]
impl CheckpointHook for SessionCheckpointHook {
    async fn before_inference(
        &self,
        snapshot: &LLMContextSnapshot,
    ) -> std::result::Result<Option<Injection>, String> {
        let pending = self.pending.lock().expect("pending lock").take();
        if let Some((receipt, dropped)) = pending {
            self.commit_pending(snapshot, receipt, dropped)
                .await
                .map_err(|e| e.to_string())?;
            return Ok(None);
        }
        self.boundary(snapshot).await.map_err(|e| e.to_string())
    }
}

/// Refs a session declared at creation (for activity initialization).
pub fn scope_touching(cfg: &SessionConfig, entry: Option<&RegistryEntry>) -> Vec<Touching> {
    let mut out = Vec::new();
    if let Some(e) = entry {
        for r in declared_refs(e) {
            let kind = if r.starts_with("artifact:") {
                "artifact"
            } else if r.starts_with("ws:") || r.starts_with('/') {
                "path"
            } else {
                "object"
            };
            out.push(Touching {
                kind: kind.into(),
                target: r,
                mode: "write".into(),
                since_ms: crate::now_ms(),
            });
        }
    } else if let Some(scope) = &cfg.session.scope {
        for p in &scope.paths {
            out.push(Touching {
                kind: "path".into(),
                target: p.clone(),
                mode: "write".into(),
                since_ms: crate::now_ms(),
            });
        }
    }
    out
}
