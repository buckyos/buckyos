//! §9.6 of NewOpenDANRuntime — top-level `AIAgent` runtime.
//!
//! MVP control flow:
//!
//! ```text
//!   AIAgent::open(root, runtime) -> Self        // load AgentConfig
//!   AIAgent::run()                              // spawns the dispatcher loop
//!     ├── restore_active_sessions()             // boot non-Ended sessions on disk
//!     ├── select loop:
//!     │     - inbound MsgPack  → dispatch_msg_pack → AgentSession::submit_text
//!     │     - inbound EventPack→ dispatch_event_pack (MVP no-op)
//!     │     - shutdown        → graceful stop all sessions
//!     └── reply collector task per session       // logs assistant text / errors
//! ```
//!
//! In MVP the inbound message source is an `mpsc::Sender<InboundMsg>` exposed
//! by `AIAgent::inbox()` — the caller (an RPC handler, a CLI, or a test
//! harness) pushes messages in. Wiring contact_mgr / task_mgr happens once
//! those crates have their consumer surface decided; the seam here is the
//! `InboundMsg` enum + the `inbox()` accessor.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use log::{info, warn};
use tokio::sync::{mpsc, Mutex, Notify};

use crate::agent_bash::build_session_tools;
use crate::agent_config::AgentConfig;
use crate::agent_session::{
    AgentSession, AgentSessionBuild, PendingInput, SessionKind, SessionMeta, SessionReply,
    SessionStatus,
};
use crate::ai_runtime::AgentRuntime;
use crate::local_workspace::LocalWorkspaceManager;
use crate::msg_center_pump::{self, PumpConfig};
use crate::session_event_pump::SessionEventPump;

/// Reason string we tag msg-center ack updates with so audit logs can tell
/// "the opendan agent picked this up" apart from other consumers.
const MSG_ROUTED_REASON: &str = "routed_by_opendan_runtime";

/// One inbound item to route to a session. Tagged so messages and events
/// share the same tokio queue into the dispatcher — keeping with the
/// "external boundary via buckyos-api, internal dispatch via tokio" rule.
///
/// The dispatcher is responsible for:
///   1. mapping each variant to a target session (by explicit id, by
///      `from`-tunnel, or by event-id subscription),
///   2. handing the item to `AgentSession::enqueue_pending` which
///      durably parks it on the session,
///   3. acking back to the source (msg-center `update_record_state` for
///      Msg items; kevent has no per-event ack today).
#[derive(Debug, Clone)]
pub enum Inbound {
    /// A chat-style message — either pulled from msg-center by the pump or
    /// injected locally via [`AIAgent::inbox()`].
    Msg {
        /// Stable id used both as the dedup key inside the session's
        /// pending queue and as the ack handle back to msg-center. Locally
        /// injected items use a synthetic `local-...` id.
        record_id: String,
        /// Originating tunnel / DID host name. Drives the
        /// `tunnel_to_ui_session` lookup when `session_id` is `None`.
        from: String,
        /// Full DID of the sender, used as the reply target when the
        /// session emits an assistant message. `None` for locally-injected
        /// inputs where there is no real peer DID.
        from_did: Option<String>,
        /// Preferred tunnel DID extracted from the msg-center route hint.
        /// Passed through to `msg_center.post_send` as `preferred_tunnel`
        /// so replies ride the same wire whenever possible.
        tunnel_did: Option<String>,
        /// Optional explicit target. `None` ⇒ resolve via `from`.
        session_id: Option<String>,
        text: String,
    },
    /// A subscribed kevent. MVP forwards these to the per-tunnel UI session
    /// as a placeholder — proper per-session kevent subscriptions land
    /// alongside `session_sub_kevent`.
    Event {
        event_id: String,
        /// When the caller already knows which session should consume this
        /// event (e.g. timer events that the session itself scheduled),
        /// they can pre-route by setting this.
        target_session_id: Option<String>,
        data: serde_json::Value,
    },
}

/// Shutdown signal. Owners drop the sender or `send(())` to start a graceful
/// shutdown.
type ShutdownRx = mpsc::Receiver<()>;
type ShutdownTx = mpsc::Sender<()>;

pub struct AIAgent {
    pub config: Arc<AgentConfig>,
    pub runtime: Arc<AgentRuntime>,
    pub agent_name: String,
    /// Map tunnel/from → UI session id.
    tunnel_to_ui_session: Arc<Mutex<HashMap<String, String>>>,
    sessions: Arc<Mutex<HashMap<String, Arc<AgentSession>>>>,
    inbox_tx: mpsc::Sender<Inbound>,
    inbox_rx: Arc<Mutex<Option<mpsc::Receiver<Inbound>>>>,
    shutdown_tx: ShutdownTx,
    shutdown_rx: Arc<Mutex<Option<ShutdownRx>>>,
    /// Signalled when `run()` is exiting, so the msg-center pump task can
    /// drop its kevent reader and return promptly.
    pump_shutdown: Arc<Notify>,
    /// Per-session kevent subscription pump. `None` when the runtime has
    /// no `kevent_client` (CLI / test). Cheap to keep around: idle pump
    /// just parks on its `refresh` Notify when no session subscribes.
    event_pump: Option<Arc<SessionEventPump>>,
    /// Owns the on-disk workspace records under `<agent_root>/workspace/`.
    /// Stateless — cloning is just a `PathBuf`.
    workspaces: LocalWorkspaceManager,
}

impl AIAgent {
    pub fn open(root: PathBuf, runtime: Arc<AgentRuntime>) -> Result<Arc<Self>> {
        let config = AgentConfig::open(root)
            .map_err(|err| anyhow!("open agent config: {err}"))?;
        let agent_name = if !config.toml.display_name.trim().is_empty() {
            config.toml.display_name.clone()
        } else {
            config
                .layout
                .root
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("agent")
                .to_string()
        };
        let (inbox_tx, inbox_rx) = mpsc::channel(256);
        let (shutdown_tx, shutdown_rx) = mpsc::channel(1);
        let pump_shutdown = Arc::new(Notify::new());
        let event_pump = runtime.kevent_client.as_ref().map(|kc| {
            SessionEventPump::new(
                agent_name.clone(),
                kc.clone(),
                inbox_tx.clone(),
                pump_shutdown.clone(),
            )
        });
        let workspaces = LocalWorkspaceManager::new(config.layout.workspaces_dir.clone());
        Ok(Arc::new(Self {
            config: Arc::new(config),
            runtime,
            agent_name,
            tunnel_to_ui_session: Arc::new(Mutex::new(HashMap::new())),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            inbox_tx,
            inbox_rx: Arc::new(Mutex::new(Some(inbox_rx))),
            shutdown_tx,
            shutdown_rx: Arc::new(Mutex::new(Some(shutdown_rx))),
            pump_shutdown,
            event_pump,
            workspaces,
        }))
    }

    /// Public accessor for the agent-owned workspace manager. Tools that
    /// need to enumerate / pick workspaces (e.g. `try_create_worksession`)
    /// hold this handle.
    pub fn workspaces(&self) -> &LocalWorkspaceManager {
        &self.workspaces
    }

    /// Producer-end clone of the inbox. Multiple callers may keep clones.
    pub fn inbox(&self) -> mpsc::Sender<Inbound> {
        self.inbox_tx.clone()
    }

    /// Trigger a graceful shutdown. Returns immediately; `run()` exits its
    /// loop and joins outstanding sessions.
    pub async fn shutdown(&self) {
        let _ = self.shutdown_tx.send(()).await;
    }

    /// Run the dispatcher loop. Consumes the receivers held inside `self`
    /// (single-shot — calling twice panics).
    pub async fn run(self: Arc<Self>) -> Result<()> {
        info!("opendan.agent[{}]: starting AIAgent::run", self.agent_name);
        self.clone().restore_active_sessions().await;

        let mut inbox_rx = self
            .inbox_rx
            .lock()
            .await
            .take()
            .ok_or_else(|| anyhow!("AIAgent::run called twice (inbox already taken)"))?;
        let mut shutdown_rx = self
            .shutdown_rx
            .lock()
            .await
            .take()
            .ok_or_else(|| anyhow!("AIAgent::run called twice (shutdown already taken)"))?;

        let pump_handle = self.clone().spawn_msg_center_pump();
        let event_pump_handle = self.event_pump.as_ref().map(|p| {
            let p = p.clone();
            tokio::spawn(async move { p.run().await })
        });

        loop {
            tokio::select! {
                msg = inbox_rx.recv() => {
                    let Some(msg) = msg else {
                        info!("opendan.agent[{}]: inbox closed, shutting down", self.agent_name);
                        break;
                    };
                    if let Err(err) = self.clone().dispatch_inbound(msg).await {
                        warn!("opendan.agent[{}]: dispatch_inbound failed: {err:#}", self.agent_name);
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("opendan.agent[{}]: shutdown signal received", self.agent_name);
                    break;
                }
            }
        }
        self.pump_shutdown.notify_waiters();
        if let Some(handle) = pump_handle {
            // Best-effort: pump task observes `pump_shutdown` and exits on its
            // own; we just wait so the kevent reader is fully closed before
            // the agent drops.
            let _ = handle.await;
        }
        if let Some(handle) = event_pump_handle {
            let _ = handle.await;
        }
        self.stop_all_sessions().await;
        Ok(())
    }

    /// Spawn the msg-center / kevent inbound pump if the runtime wired both
    /// dependencies and the agent has a parseable owner DID. Returns `None`
    /// when any of those is missing — the agent then runs in
    /// inbox()-only mode, which is the right behavior for tests and CLI.
    fn spawn_msg_center_pump(
        self: Arc<Self>,
    ) -> Option<tokio::task::JoinHandle<()>> {
        let msg_center = self.runtime.msg_center.clone()?;
        let kevent_client = self.runtime.kevent_client.clone()?;
        let owner_did = msg_center_pump::parse_owner_did(&self.config.toml.agent_did)?;
        let cfg = PumpConfig {
            agent_name: self.agent_name.clone(),
            owner_did,
            msg_center,
            kevent_client,
            inbox_tx: self.inbox_tx.clone(),
            shutdown: self.pump_shutdown.clone(),
        };
        Some(tokio::spawn(msg_center_pump::run(cfg)))
    }

    /// Restore non-Ended sessions from disk. MVP scope: for each
    /// `session/<id>/.meta/session.json` whose status is not `Ended`, recreate
    /// the AgentSession in-memory (its worker will resume from `state.snap`
    /// on first input).
    async fn restore_active_sessions(self: Arc<Self>) {
        let sessions_dir = self.config.layout.sessions_dir.clone();
        let Ok(entries) = std::fs::read_dir(&sessions_dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let meta_path = path.join(".meta").join("session.json");
            let Ok(bytes) = std::fs::read(&meta_path) else {
                continue;
            };
            let Ok(meta) = serde_json::from_slice::<SessionMeta>(&bytes) else {
                warn!(
                    "opendan.agent[{}]: cannot decode {} — skipping",
                    self.agent_name,
                    meta_path.display()
                );
                continue;
            };
            if matches!(meta.status, SessionStatus::Ended) {
                continue;
            }
            if let Err(err) = self.clone().restore_session(meta).await {
                warn!(
                    "opendan.agent[{}]: restore session {} failed: {err:#}",
                    self.agent_name,
                    path.display()
                );
            }
        }
    }

    async fn restore_session(self: Arc<Self>, meta: SessionMeta) -> Result<()> {
        info!(
            "opendan.agent[{}]: restoring session {} (kind={:?})",
            self.agent_name, meta.session_id, meta.kind
        );
        let session_id = meta.session_id.clone();
        let kind = meta.kind;
        let owner = meta.owner.clone();
        let behavior = meta.current_behavior.clone();
        let _session = self
            .clone()
            .ensure_session_inner(
                session_id.clone(),
                kind,
                owner.clone(),
                Some(behavior),
                Some(meta),
            )
            .await?;
        if matches!(kind, SessionKind::Ui) && !owner.is_empty() {
            self.tunnel_to_ui_session
                .lock()
                .await
                .insert(owner, session_id);
        }
        Ok(())
    }

    async fn dispatch_inbound(self: Arc<Self>, item: Inbound) -> Result<()> {
        match item {
            Inbound::Msg {
                record_id,
                from,
                from_did,
                tunnel_did,
                session_id,
                text,
            } => {
                let resolved_id = if let Some(sid) = session_id {
                    sid
                } else {
                    self.clone().resolve_ui_session(&from).await?
                };
                let session = self
                    .clone()
                    .get_or_create_session(resolved_id, from.clone())
                    .await?;
                // enqueue_pending durably parks the input on the session
                // and only returns once `.meta/session.json` is on disk.
                // Once it returns we're safe to ack upstream — a crash from
                // here on leaves the input owned by the session, not lost.
                session
                    .enqueue_pending(PendingInput::Msg {
                        record_id: record_id.clone(),
                        from,
                        from_did,
                        tunnel_did,
                        text,
                    })
                    .await?;
                self.ack_msg_record(record_id).await;
                Ok(())
            }
            Inbound::Event {
                event_id,
                target_session_id,
                data,
            } => {
                // Event routing is intentionally narrow in MVP: only
                // pre-routed events (carrier sets `target_session_id`) are
                // delivered. Broadcast / pattern-matched event delivery
                // lands with `session_sub_kevent`.
                let Some(sid) = target_session_id else {
                    warn!(
                        "opendan.agent[{}]: event {} dropped — no target_session_id and broadcast routing not yet wired",
                        self.agent_name, event_id
                    );
                    return Ok(());
                };
                let session = {
                    let map = self.sessions.lock().await;
                    map.get(&sid).cloned()
                };
                let Some(session) = session else {
                    warn!(
                        "opendan.agent[{}]: event {} target session {} unknown, dropping",
                        self.agent_name, event_id, sid
                    );
                    return Ok(());
                };
                session
                    .enqueue_pending(PendingInput::Event { event_id, data })
                    .await?;
                Ok(())
            }
        }
    }

    /// Best-effort ack to msg-center after the record is durably parked on
    /// a session. Failure is logged but not returned — the session already
    /// owns the input, so even a stuck `Reading` record is recoverable
    /// (msg-center's lease will eventually flip it back to `Unread` and we
    /// dedup by `record_id` when re-enqueued).
    async fn ack_msg_record(&self, record_id: String) {
        // Locally-injected records (synthetic id) never hit msg-center.
        if record_id.starts_with("local-") {
            return;
        }
        let Some(msg_center) = self.runtime.msg_center.as_ref() else {
            return;
        };
        if let Err(err) = msg_center
            .update_record_state(
                record_id.clone(),
                buckyos_api::MsgState::Readed,
                Some(MSG_ROUTED_REASON.to_string()),
            )
            .await
        {
            warn!(
                "opendan.agent[{}]: ack record_id={} failed: {err}",
                self.agent_name, record_id
            );
        }
    }

    async fn resolve_ui_session(self: Arc<Self>, from: &str) -> Result<String> {
        if let Some(sid) = self.tunnel_to_ui_session.lock().await.get(from) {
            return Ok(sid.clone());
        }
        // Mint a deterministic UI session id keyed on `from` — survives
        // process restart so the same tunnel always lands on the same session.
        let sid = format!("ui-{}", sanitize_session_segment(from));
        self.tunnel_to_ui_session
            .lock()
            .await
            .insert(from.to_string(), sid.clone());
        Ok(sid)
    }

    async fn get_or_create_session(
        self: Arc<Self>,
        session_id: String,
        owner: String,
    ) -> Result<Arc<AgentSession>> {
        // Note: existing session lookup is in a separate scope so we can drop
        // the lock before doing the (potentially expensive) tool manager
        // bootstrap on a miss.
        if let Some(s) = self.sessions.lock().await.get(&session_id).cloned() {
            return Ok(s);
        }
        self.ensure_session_inner(session_id, SessionKind::Ui, owner, None, None)
            .await
    }

    async fn ensure_session_inner(
        self: Arc<Self>,
        session_id: String,
        kind: SessionKind,
        owner: String,
        behavior_hint: Option<String>,
        existing_meta: Option<SessionMeta>,
    ) -> Result<Arc<AgentSession>> {
        {
            let map = self.sessions.lock().await;
            if let Some(s) = map.get(&session_id) {
                return Ok(s.clone());
            }
        }
        let session_dir = self.config.layout.session_dir(&session_id);
        // Workspace pick:
        //   - existing_meta with a workspace_id → reuse (restore path).
        //   - otherwise → mint a workspace keyed off the session id so the
        //     legacy MVP file layout is preserved.
        // The proper §8 worksession creation flow will replace this with
        // `try_create_worksession` picking an existing workspace.
        let preselected_ws = existing_meta
            .as_ref()
            .and_then(|m| m.workspace_id.clone())
            .filter(|s| !s.trim().is_empty());
        let workspace_id = preselected_ws.unwrap_or_else(|| session_id.clone());
        let workspace_rec = self
            .workspaces
            .create_or_open(&workspace_id, &workspace_id, Some(&session_id))
            .await
            .map_err(|err| anyhow!("open workspace `{workspace_id}`: {err}"))?;
        let workspace_root = self.workspaces.workspace_dir(&workspace_rec.workspace_id);
        let _ = std::fs::create_dir_all(&session_dir);

        let tools = build_session_tools(&workspace_root, &session_dir)
            .map_err(|err| anyhow!("build session tools: {err}"))?;
        let behavior_name = behavior_hint.unwrap_or_else(|| match kind {
            SessionKind::Ui => self.config.default_ui_behavior().to_string(),
            SessionKind::Work => self.config.default_work_behavior().to_string(),
        });

        let (reply_tx, mut reply_rx) = mpsc::channel(64);
        let (session, inbox_rx) = AgentSession::new(AgentSessionBuild {
            session_id: session_id.clone(),
            agent_name: self.agent_name.clone(),
            kind,
            owner: owner.clone(),
            current_behavior: behavior_name,
            runtime: self.runtime.clone(),
            agent_config: self.config.clone(),
            tools,
            reply_tx,
            existing_meta,
        });
        let session = Arc::new(session);
        // Reciprocal binding: session ↔ workspace. Session-side first so
        // its meta is the source of truth; if the workspace-side update
        // fails the session still has the correct binding.
        if let Err(err) = session
            .set_workspace(Some(workspace_rec.workspace_id.clone()))
            .await
        {
            warn!(
                "opendan.agent[{}]: bind workspace `{}` on session {} failed: {err:#}",
                self.agent_name, workspace_rec.workspace_id, session_id
            );
        }
        if let Err(err) = self
            .workspaces
            .set_current_session(&workspace_rec.workspace_id, Some(&session_id))
            .await
        {
            warn!(
                "opendan.agent[{}]: workspace `{}` set_current_session failed: {err}",
                self.agent_name, workspace_rec.workspace_id
            );
        }
        if let Err(err) = session.flush_meta().await {
            warn!(
                "opendan.agent[{}]: initial flush_meta for session {} failed: {err:#}",
                self.agent_name, session_id
            );
        }
        session.clone().start(inbox_rx).await;

        // Reply collector: for MVP just log + (if we had a way) forward to the
        // tunnel. Spawn it under the session id.
        let log_sid = session_id.clone();
        let agent_name = self.agent_name.clone();
        let owner_for_log = owner.clone();
        tokio::spawn(async move {
            while let Some(reply) = reply_rx.recv().await {
                match reply {
                    SessionReply::AssistantText { text } => {
                        info!(
                            "opendan.agent[{agent_name}]: session={log_sid} owner={owner_for_log} assistant: {}",
                            truncate(&text, 240)
                        );
                    }
                    SessionReply::PromptToHuman { text } => {
                        info!(
                            "opendan.agent[{agent_name}]: session={log_sid} prompt-to-human: {}",
                            truncate(&text, 240)
                        );
                    }
                    SessionReply::Error { message } => {
                        warn!(
                            "opendan.agent[{agent_name}]: session={log_sid} error: {message}"
                        );
                    }
                    SessionReply::Ended => {
                        info!(
                            "opendan.agent[{agent_name}]: session={log_sid} ended"
                        );
                        break;
                    }
                }
            }
        });

        self.sessions
            .lock()
            .await
            .insert(session_id.clone(), session.clone());
        // Propagate any persisted subscriptions for this session into the
        // shared event pump. Re-running this for a fresh session is cheap
        // (no subscriptions yet ⇒ empty list ⇒ no reader rebuild).
        if let Some(pump) = self.event_pump.as_ref() {
            let patterns = session.subscription_patterns().await;
            pump.set_session_subscriptions(&session_id, patterns).await;
        }
        Ok(session)
    }

    /// Refresh the event pump's view of a session's subscriptions. Call
    /// this after `AgentSession::subscribe_event` / `unsubscribe_event`
    /// from a tool implementation. No-op when the runtime has no kevent
    /// client (tests, CLI without zone services).
    pub async fn refresh_session_subscriptions(&self, session_id: &str) {
        let Some(pump) = self.event_pump.as_ref() else {
            return;
        };
        let session = self.sessions.lock().await.get(session_id).cloned();
        let patterns = match session {
            Some(s) => s.subscription_patterns().await,
            None => Vec::new(),
        };
        pump.set_session_subscriptions(session_id, patterns).await;
    }

    async fn stop_all_sessions(&self) {
        let sessions = {
            let map = self.sessions.lock().await;
            map.values().cloned().collect::<Vec<_>>()
        };
        for s in sessions {
            s.stop().await;
        }
    }
}

fn sanitize_session_segment(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        out.push_str("anon");
    }
    out
}

fn truncate(s: &str, limit: usize) -> String {
    if s.chars().count() <= limit {
        return s.to_string();
    }
    let mut acc = String::with_capacity(limit + 1);
    for ch in s.chars().take(limit) {
        acc.push(ch);
    }
    acc.push('…');
    acc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_session_segment() {
        assert_eq!(sanitize_session_segment("did:dev:alice"), "did_dev_alice");
        assert_eq!(sanitize_session_segment(""), "anon");
    }

    #[test]
    fn truncate_short_string() {
        assert_eq!(truncate("hi", 10), "hi");
    }

    #[test]
    fn truncate_long_string() {
        assert_eq!(truncate("abcdefghij", 4), "abcd…");
    }
}
