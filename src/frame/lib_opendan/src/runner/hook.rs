//! The checkpoint hook (§8.3 / §8.5).
//!
//! The hook runs before every inference with the outer snapshot: it
//! publishes the snapshot with the tool results it now contains (clears the
//! in-flight markers it covers), routes what the bus delivered meanwhile
//! (controls are applied, Observe events are merged into `pending_events`
//! and saved) and refreshes the activity heartbeat.
//!
//! Nothing is injected here: reaching a checkpoint, finishing a tool call
//! or receiving an Observe event never puts a message into the context.
//! Saved semi-subscription state waits for the next controlled input
//! (`on_init / on_input / on_context_switch`), which renders it as the
//! snapshot message in front of its own.

use std::sync::Arc;

use async_trait::async_trait;
use llm_context::deps::{CheckpointHook, Injection};
use llm_context::state::LLMContextSnapshot;
use serde_json::{json, Value};

use crate::error::Result;
use crate::protocol::*;
use crate::session::runs::RunHandle;
use crate::state::declared_refs;

use super::inputs::route_inputs;
use super::shared::{report, Shared};

pub(super) fn watched_view(status: &SessionStatus, watch: &[String]) -> Value {
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

/// The per-run checkpoint hook.
pub struct SessionCheckpointHook {
    shared: Arc<Shared>,
    run: RunHandle,
}

impl SessionCheckpointHook {
    pub fn new(shared: Arc<Shared>, run: RunHandle) -> Self {
        Self { shared, run }
    }

    async fn boundary(&self, snapshot: &LLMContextSnapshot) -> Result<()> {
        let sh = &self.shared;
        sh.lease.check()?;
        // Tool results first: publish and clear covered in-flight actions.
        self.run.checkpoint_with_results(snapshot, None)?;
        // Controls, rejected records, Observe events. msg / Input events
        // stay queued for the next controlled input.
        route_inputs(sh, true, &[]).await?;
        if sh.session.lock().await.state.stop_requested {
            if let Some(h) = sh.interrupt.lock().expect("interrupt lock").as_ref() {
                h.interrupt("session stop requested");
            }
            return Ok(());
        }
        // A Turn that reached a tool call tells the other side it is being
        // worked on; its task follows what the session does.
        let tool_called = sh.current_tool.lock().expect("current tool").is_some();
        super::outbound::maybe_placeholder(sh, tool_called).await;
        super::turn_task::sync_turn_task(sh).await;
        self.heartbeat().await
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
        self.boundary(snapshot).await.map_err(|e| e.to_string())?;
        Ok(None)
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
