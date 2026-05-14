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
use tokio::sync::{mpsc, Mutex};

use crate::agent_bash::build_session_tools;
use crate::agent_config::AgentConfig;
use crate::agent_session::{
    AgentSession, AgentSessionBuild, SessionKind, SessionMeta, SessionReply, SessionStatus,
};
use crate::ai_runtime::AgentRuntime;

/// One inbound user/tunnel message. MVP is text-only; multi-modal arrives in
/// later steps once contact_mgr is wired.
#[derive(Debug, Clone)]
pub struct InboundMsg {
    /// Originating tunnel / DID. Used to map to (or create) a UI session.
    pub from: String,
    /// Optional explicit session_id. When `None`, routing falls back to the
    /// per-tunnel UI session.
    pub session_id: Option<String>,
    pub text: String,
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
    inbox_tx: mpsc::Sender<InboundMsg>,
    inbox_rx: Arc<Mutex<Option<mpsc::Receiver<InboundMsg>>>>,
    shutdown_tx: ShutdownTx,
    shutdown_rx: Arc<Mutex<Option<ShutdownRx>>>,
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
        }))
    }

    /// Producer-end clone of the inbox. Multiple callers may keep clones.
    pub fn inbox(&self) -> mpsc::Sender<InboundMsg> {
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

        loop {
            tokio::select! {
                msg = inbox_rx.recv() => {
                    let Some(msg) = msg else {
                        info!("opendan.agent[{}]: inbox closed, shutting down", self.agent_name);
                        break;
                    };
                    if let Err(err) = self.clone().dispatch_msg(msg).await {
                        warn!("opendan.agent[{}]: dispatch_msg failed: {err:#}", self.agent_name);
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("opendan.agent[{}]: shutdown signal received", self.agent_name);
                    break;
                }
            }
        }
        self.stop_all_sessions().await;
        Ok(())
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
        let _session = self
            .clone()
            .ensure_session_inner(
                meta.session_id.clone(),
                meta.kind,
                meta.owner.clone(),
                Some(meta.current_behavior.clone()),
            )
            .await?;
        if matches!(meta.kind, SessionKind::Ui) && !meta.owner.is_empty() {
            self.tunnel_to_ui_session
                .lock()
                .await
                .insert(meta.owner, meta.session_id);
        }
        Ok(())
    }

    async fn dispatch_msg(self: Arc<Self>, msg: InboundMsg) -> Result<()> {
        let session_id = if let Some(sid) = msg.session_id.clone() {
            sid
        } else {
            self.clone().resolve_ui_session(&msg.from).await?
        };
        let session = self.clone().get_or_create_session(session_id, msg.from).await?;
        session.submit_text(msg.text).await?;
        Ok(())
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
        self.ensure_session_inner(session_id, SessionKind::Ui, owner, None)
            .await
    }

    async fn ensure_session_inner(
        self: Arc<Self>,
        session_id: String,
        kind: SessionKind,
        owner: String,
        behavior_hint: Option<String>,
    ) -> Result<Arc<AgentSession>> {
        {
            let map = self.sessions.lock().await;
            if let Some(s) = map.get(&session_id) {
                return Ok(s.clone());
            }
        }
        let session_dir = self.config.layout.session_dir(&session_id);
        let workspace_root = self.config.layout.workspaces_dir.join(&session_id);
        // Workspace dir is auto-created for MVP — the proper §8 worksession
        // creation flow will pick / bind an existing workspace instead.
        let _ = std::fs::create_dir_all(&workspace_root);
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
        });
        let session = Arc::new(session);
        session.flush_meta().await;
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
        Ok(session)
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
