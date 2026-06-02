//! §9.6 of NewOpenDANRuntime — top-level `AIAgent` runtime.
//!
//! MVP control flow:
//!
//! ```text
//!   AIAgent::open(root, runtime) -> Self        // load AgentConfig
//!   AIAgent::run()                              // spawns the dispatcher loop
//!     ├── restore_session_routes()              // restore routing indexes only
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
use buckyos_api::{
    get_buckyos_api_runtime, parse_typed_task_data, AgentDelegateProgress, AgentDelegateTaskData,
    AgentDelegateTaskRequest, AiMessage, AiRole, CreateTaskOptions, Task, TimerOptions,
    TypedTaskData,
};
use chrono::{Local, Utc};
use log::{info, warn};
use tokio::sync::{mpsc, Mutex, Notify};

use crate::agent_bash::{build_session_tools, SessionBinLayout, SessionToolsBuild};
use crate::agent_config::AgentConfig;
use crate::agent_session::{AgentSession, AgentSessionBuild, SessionReply};
use crate::agent_task_executor::TASK_TYPE_AGENT_DELEGATE;
use crate::ai_runtime::AgentRuntime;
use crate::contact::ContactLookup;
use crate::dispatch::{
    DispatchEvaluator, EnumSessionIdStrategy, FixedRulesDispatch, SessionIdEvaluator,
    SessionIdInput,
};
use crate::local_workspace::LocalWorkspaceManager;
use crate::msg_center_pump::{self, PumpConfig};
use crate::paths;
use crate::session_event_pump::SessionEventPump;
use crate::session_model::{
    AgentTaskBinding, PendingInput, SessionKind, SessionMeta, SessionStatus, SessionSummary,
    TimerEventKind, TimerReason, TimerTargetType, TimerTriggerType, UI_CLOCK_TIMER_EVENT_ID,
    UI_CLOCK_TIMER_INTERVAL_MS,
};
use crate::tool_plan::{self, ResolvedToolPlan, SessionBinRenderer, ToolPlanToml};

/// Reason string we tag msg-center ack updates with so audit logs can tell
/// "the opendan agent picked this up" apart from other consumers.
const MSG_ROUTED_REASON: &str = "routed_by_opendan_runtime";
const SELF_CHECK_HARD_BARRIER_INTERVAL_MS: u64 = 60_000;

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
        /// Display name for the sender — populated either by msg-center
        /// (when its contact-mgr already knows the peer) or by the pump
        /// via [`ContactLookup`](crate::contact::ContactLookup) when the
        /// record lands without one. Used in prompts so the LLM sees a
        /// human-readable name instead of a raw DID.
        from_name: Option<String>,
        /// Tunnel DID from the msg-center route hint. Retained for diagnostics
        /// only: replies now target the inbound sender's determined endpoint DID
        /// (`MsgObject.to`), which carries its own routing, so this is no longer
        /// forwarded to `post_send`.
        tunnel_did: Option<String>,
        /// Optional explicit target. `None` ⇒ resolve via `from`.
        session_id: Option<String>,
        /// Group DID for group messages. `None` for one-to-one chat.
        group_id: Option<String>,
        text: String,
        ai_message: AiMessage,
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
    /// §3 — slash-command intercepted before LLM dispatch. Carries the
    /// parsed `command`/`args` plus the same routing fields as `Msg` so
    /// the agent can ack the msg-center record and post the command's
    /// system reply through the same outbound path as a normal turn.
    Command {
        record_id: String,
        from: String,
        from_did: Option<String>,
        tunnel_did: Option<String>,
        command: String,
        args: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwardRouteStrategy {
    ExplicitTargetOnly,
    MostRecentWaitingInput,
    NewWorkSessionOnInterrupt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForwardRouteDecision {
    Forward(String),
    CreateNewWorkSession,
    KeepInUi,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardRouteCandidate {
    pub session_id: String,
    pub kind: SessionKind,
    pub status: SessionStatus,
    pub status_changed_at_ms: u64,
}

/// forwardMessage routing decision table:
/// - explicit target: forward only when it is a non-Ended Work session
/// - no explicit target: at most one message is forwarded
/// - multiple WaitForInput Work sessions: choose the most recently entered
///   WaitingInput state
/// - Running Work sessions are not implicit targets
/// - user interrupt without explicit target creates a new Work session
pub fn decide_forward_route(
    strategy: ForwardRouteStrategy,
    explicit_target: Option<&str>,
    user_interrupt: bool,
    candidates: &[ForwardRouteCandidate],
) -> ForwardRouteDecision {
    if let Some(target) = explicit_target {
        return candidates
            .iter()
            .find(|candidate| {
                candidate.session_id == target
                    && matches!(candidate.kind, SessionKind::Work)
                    && !matches!(candidate.status, SessionStatus::Ended)
            })
            .map(|candidate| ForwardRouteDecision::Forward(candidate.session_id.clone()))
            .unwrap_or(ForwardRouteDecision::KeepInUi);
    }

    if user_interrupt && matches!(strategy, ForwardRouteStrategy::NewWorkSessionOnInterrupt) {
        return ForwardRouteDecision::CreateNewWorkSession;
    }

    if matches!(strategy, ForwardRouteStrategy::MostRecentWaitingInput) {
        return candidates
            .iter()
            .filter(|candidate| {
                matches!(candidate.kind, SessionKind::Work)
                    && matches!(candidate.status, SessionStatus::WaitingInput)
            })
            .max_by_key(|candidate| candidate.status_changed_at_ms)
            .map(|candidate| ForwardRouteDecision::Forward(candidate.session_id.clone()))
            .unwrap_or(ForwardRouteDecision::KeepInUi);
    }

    ForwardRouteDecision::KeepInUi
}

async fn resolve_appclient_session_token() -> Result<String> {
    let runtime = match get_buckyos_api_runtime() {
        Ok(runtime) => runtime,
        Err(err) => {
            #[cfg(test)]
            {
                warn!("opendan.agent: buckyos runtime unavailable before session tools: {err}");
                return Ok(String::new());
            }
            #[cfg(not(test))]
            {
                return Err(anyhow!("load buckyos runtime before session tools: {err}"));
            }
        }
    };
    let token = runtime.get_session_token().await;
    if token.trim().is_empty() {
        #[cfg(test)]
        {
            warn!("opendan.agent: buckyos runtime returned empty session token for session tools");
            return Ok(String::new());
        }
        #[cfg(not(test))]
        {
            return Err(anyhow!(
                "buckyos runtime returned empty session token before session tools"
            ));
        }
    }
    Ok(token)
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
    session_locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
    inbox_tx: mpsc::Sender<Inbound>,
    inbox_rx: Arc<Mutex<Option<mpsc::Receiver<Inbound>>>>,
    shutdown_tx: ShutdownTx,
    shutdown_rx: Arc<Mutex<Option<ShutdownRx>>>,
    /// Signalled when `run()` is exiting, so the msg-center pump task can
    /// drop its kevent reader and return promptly.
    pub(crate) pump_shutdown: Arc<Notify>,
    /// Per-session kevent subscription pump. `None` when the runtime has
    /// no `kevent_client` (CLI / test). Cheap to keep around: idle pump
    /// just parks on its `refresh` Notify when no session subscribes.
    event_pump: Option<Arc<SessionEventPump>>,
    /// Owns the on-disk workspace records under `<agent_root>/workspace/`.
    /// Stateless — cloning is just a `PathBuf`.
    workspaces: LocalWorkspaceManager,
    /// v0 dispatcher (fixed rule table). Trait-object so a future
    /// script-engine impl can drop-in.
    dispatcher: Arc<dyn DispatchEvaluator>,
    /// v0 session-id evaluator (4-strategy enum). Same trait-object seam.
    session_id_eval: Arc<dyn SessionIdEvaluator>,
}

impl AIAgent {
    pub fn open(root: PathBuf, runtime: Arc<AgentRuntime>) -> Result<Arc<Self>> {
        let config = AgentConfig::open(root).map_err(|err| anyhow!("open agent config: {err}"))?;
        let agent_name = if !config.toml.identity.display_name.trim().is_empty() {
            config.toml.identity.display_name.clone()
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
        let dispatcher: Arc<dyn DispatchEvaluator> =
            Arc::new(FixedRulesDispatch::new(&config.toml.dispatch));
        let session_id_eval: Arc<dyn SessionIdEvaluator> = Arc::new(EnumSessionIdStrategy);
        Ok(Arc::new(Self {
            config: Arc::new(config),
            runtime,
            agent_name,
            tunnel_to_ui_session: Arc::new(Mutex::new(HashMap::new())),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            session_locks: Arc::new(Mutex::new(HashMap::new())),
            inbox_tx,
            inbox_rx: Arc::new(Mutex::new(Some(inbox_rx))),
            shutdown_tx,
            shutdown_rx: Arc::new(Mutex::new(Some(shutdown_rx))),
            pump_shutdown,
            event_pump,
            workspaces,
            dispatcher,
            session_id_eval,
        }))
    }

    /// Public accessor for the agent-owned workspace manager. Tools that
    /// need to enumerate / pick workspaces (e.g. `try_create_worksession`)
    /// hold this handle.
    pub fn workspaces(&self) -> &LocalWorkspaceManager {
        &self.workspaces
    }

    /// Filesystem-safe identifier used wherever we splice the agent into a
    /// path (§9.2 4-layer overlay's `<agent_id>` segment). Derived from
    /// `agent_did` when present (canonical, stable across renames) and
    /// otherwise from the human-friendly `agent_name`.
    pub fn agent_id(&self) -> String {
        let raw = if !self.config.toml.identity.agent_did.trim().is_empty() {
            self.config.toml.identity.agent_did.as_str()
        } else {
            self.agent_name.as_str()
        };
        paths::sanitize_path_segment(raw)
    }

    fn agent_tool_mount_id(&self) -> String {
        std::env::var("BUCKYOS_APP_ID")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(|value| paths::sanitize_path_segment(&value))
            .unwrap_or_else(|| self.agent_id())
    }

    /// Build a Session Exec Bin renderer for `behavior_name`. Returns
    /// `Some(renderer)` whenever we have *any* lower-layer state to manage
    /// (Agent tools to sync or a tool plan to enforce). The renderer is
    /// then consulted by `TmuxBashRunner` on every `exec_bash` call.
    ///
    /// Missing behavior config / missing tool plan files are downgraded
    /// to warnings + an empty-plan renderer so a misconfigured behavior
    /// still gets Agent tools sync and doesn't refuse to start.
    fn build_session_bin_renderer(
        &self,
        agent_id: &str,
        session_id: &str,
        behavior_name: &str,
    ) -> Option<Arc<SessionBinRenderer>> {
        let layout = SessionBinLayout::compute(agent_id, session_id, &self.config.layout.root);

        // Look up the behavior's tool_plan; tolerate a missing behavior
        // config because `builtin_ui_default()` is used as a fallback in
        // the session worker.
        let plan_name = match self.config.load_behavior(behavior_name) {
            Ok(cfg) => cfg.capabilities.tool_plan,
            Err(err) => {
                warn!(
                    "opendan.agent[{}]: load behavior `{}` for tool plan failed (using empty plan): {err}",
                    self.agent_name, behavior_name
                );
                String::new()
            }
        };

        let (plan_name, plan_toml) = if plan_name.trim().is_empty() {
            (String::new(), ToolPlanToml::default())
        } else {
            let path = self.config.layout.tool_plan_path(&plan_name);
            match ToolPlanToml::load_from_file(&path) {
                Ok(p) => (plan_name, p),
                Err(err) => {
                    warn!(
                        "opendan.agent[{}]: load tool plan `{}` failed (using empty plan): {err}",
                        self.agent_name, plan_name
                    );
                    (plan_name, ToolPlanToml::default())
                }
            }
        };

        let universe = tool_plan::scan_bin_universe([
            &layout.system_bin,
            &layout.runtime_bin,
            &layout.agent_bin,
        ]);
        let resolved = ResolvedToolPlan::resolve(
            if plan_name.is_empty() {
                "(none)"
            } else {
                &plan_name
            },
            &plan_toml,
            &universe,
        );
        Some(Arc::new(SessionBinRenderer::new(
            layout.session_bin.clone(),
            layout.agent_bin.clone(),
            if plan_name.is_empty() {
                "(none)".to_string()
            } else {
                plan_name
            },
            resolved,
        )))
    }

    /// Producer-end clone of the inbox. Multiple callers may keep clones.
    pub fn inbox(&self) -> mpsc::Sender<Inbound> {
        self.inbox_tx.clone()
    }

    /// Look up a live session by id. Returns `None` when no session with
    /// that id is currently mounted (never existed, archived, or removed
    /// after `NextAction::End`).
    ///
    /// Session-aware tools (`try_create_worksession`, `forward_msg`, ...)
    /// use this to reach into the calling session for fork primitives /
    /// pending-input injection.
    pub async fn get_session(&self, session_id: &str) -> Option<Arc<AgentSession>> {
        self.sessions.lock().await.get(session_id).cloned()
    }

    /// Snapshot every live session as a [`SessionSummary`]. Used by the
    /// `try_create_worksession` fork sub-context to give its LLM enough
    /// inventory to decide "reuse an existing worksession vs create a new
    /// one" — see §8.2 of `notepads/NewOpenDANRuntime.md`.
    ///
    /// Ordering is alphabetical by `session_id` for determinism. Excludes
    /// the calling session (when `exclude_id` matches), since "reuse the
    /// session that just called this tool" is never the right answer.
    pub async fn list_session_summaries(&self, exclude_id: Option<&str>) -> Vec<SessionSummary> {
        let sessions: Vec<Arc<AgentSession>> = {
            let map = self.sessions.lock().await;
            let mut entries: Vec<_> = map
                .iter()
                .filter(|(id, _)| exclude_id != Some(id.as_str()))
                .map(|(_, s)| s.clone())
                .collect();
            entries.sort_by(|a, b| a.session_id.cmp(&b.session_id));
            entries
        };
        let mut out = Vec::with_capacity(sessions.len());
        for s in sessions {
            out.push(s.summary().await);
        }
        out
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
        self.clone().restore_session_routes().await;
        self.clone().ensure_self_check_hard_barrier_timer().await;
        self.clone().ensure_ui_clock_timer().await;

        let pump_handle = self.clone().spawn_msg_center_pump();
        let task_inbox_handle = self.clone().spawn_task_inbox();
        let event_pump_handle = self.event_pump.as_ref().map(|p| {
            let p = p.clone();
            tokio::spawn(async move { p.run().await })
        });

        if std::env::var("AGENT_MAIN_LOOP").ok().as_deref() == Some("1") {
            info!(
                "opendan.agent[{}]: AGENT_MAIN_LOOP=1 enabled",
                self.agent_name
            );
        }
        self.clone().main_loop().await?;
        self.pump_shutdown.notify_waiters();
        if let Some(handle) = pump_handle {
            // Best-effort: pump task observes `pump_shutdown` and exits on its
            // own; we just wait so the kevent reader is fully closed before
            // the agent drops.
            let _ = handle.await;
        }
        if let Some(handle) = task_inbox_handle {
            let _ = handle.await;
        }
        if let Some(handle) = event_pump_handle {
            let _ = handle.await;
        }
        self.stop_all_sessions().await;
        Ok(())
    }

    pub async fn main_loop(self: Arc<Self>) -> Result<()> {
        let mut inbox_rx = self
            .inbox_rx
            .lock()
            .await
            .take()
            .ok_or_else(|| anyhow!("AIAgent::main_loop called twice (inbox already taken)"))?;
        let mut shutdown_rx =
            self.shutdown_rx.lock().await.take().ok_or_else(|| {
                anyhow!("AIAgent::main_loop called twice (shutdown already taken)")
            })?;

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
        Ok(())
    }

    /// Spawn the msg-center / kevent inbound pump if the runtime wired both
    /// dependencies and the agent has a parseable owner DID. Returns `None`
    /// when any of those is missing — the agent then runs in
    /// inbox()-only mode, which is the right behavior for tests and CLI.
    fn spawn_msg_center_pump(self: Arc<Self>) -> Option<tokio::task::JoinHandle<()>> {
        let msg_center = self.runtime.msg_center.clone()?;
        let kevent_client = self.runtime.kevent_client.clone()?;
        let owner_did = msg_center_pump::parse_owner_did(&self.config.toml.identity.agent_did)?;
        let contact_lookup = Some(Arc::new(ContactLookup::new(
            msg_center.clone(),
            Some(owner_did.clone()),
        )));
        let cfg = PumpConfig {
            agent_name: self.agent_name.clone(),
            owner_did,
            msg_center,
            kevent_client,
            inbox_tx: self.inbox_tx.clone(),
            shutdown: self.pump_shutdown.clone(),
            contact_lookup,
        };
        Some(tokio::spawn(msg_center_pump::run(cfg)))
    }

    /// Restore lightweight dispatch indexes for non-Ended sessions without
    /// starting their workers. Workers are mounted on demand when the main loop
    /// is about to write a new pending input.
    async fn restore_session_routes(self: Arc<Self>) {
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
            let mut meta = meta;
            if matches!(meta.status, SessionStatus::Ended) {
                continue;
            }
            if meta.ensure_default_event_subscriptions(current_unix_ms()) {
                info!(
                    "opendan.agent[{}]: backfill default ui clock subscription session_id={} event_id={} mode=background_only",
                    self.agent_name, meta.session_id, UI_CLOCK_TIMER_EVENT_ID
                );
                match serde_json::to_vec_pretty(&meta) {
                    Ok(bytes) => {
                        if let Err(err) = std::fs::write(&meta_path, bytes) {
                            warn!(
                                "opendan.agent[{}]: backfill default subscriptions for {} failed: {err}",
                                self.agent_name,
                                meta_path.display()
                            );
                        }
                    }
                    Err(err) => {
                        warn!(
                            "opendan.agent[{}]: encode default subscriptions for {} failed: {err}",
                            self.agent_name,
                            meta_path.display()
                        );
                    }
                }
            }
            if matches!(meta.kind, SessionKind::Ui) && !meta.owner.is_empty() {
                self.tunnel_to_ui_session
                    .lock()
                    .await
                    .insert(meta.owner.clone(), meta.session_id.clone());
            }
            if let Some(pump) = self.event_pump.as_ref() {
                let patterns = meta
                    .event_subscriptions
                    .iter()
                    .map(|sub| sub.pattern.clone())
                    .collect::<Vec<_>>();
                pump.set_session_subscriptions(&meta.session_id, patterns)
                    .await;
            }
        }
    }

    /// Decide which session class an inbound item lands in. Walks the
    /// `[dispatch]` rule table for the matching event-type string and
    /// falls back to `default_class` when nothing fires.
    fn route_to_class(&self, event_type: &str) -> String {
        self.dispatcher
            .route(event_type)
            .filter(|class| self.config.session_class_enabled(class))
            .or_else(|| {
                let default = self.config.toml.dispatch.default_class.clone();
                if self.config.session_class_enabled(&default) {
                    Some(default)
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "ui".to_string())
    }

    fn route_msg(&self, event_type: &str) -> String {
        self.route_to_class(event_type)
    }

    fn route_event_target(&self, data: &serde_json::Value) -> Option<String> {
        data.get("target_session_id")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .or_else(|| {
                data.get("reason")
                    .and_then(|value| {
                        serde_json::from_value::<crate::session_model::TimerReason>(value.clone())
                            .ok()
                    })
                    .and_then(|reason| {
                        let target_id = reason.target_id.trim();
                        if target_id.is_empty() {
                            None
                        } else {
                            Some(target_id.to_string())
                        }
                    })
            })
    }

    pub async fn schedule_precise_timer(
        &self,
        session_id: &str,
        reason: TimerReason,
    ) -> Result<String> {
        let Some(client) = self.runtime.kevent_client.as_ref() else {
            return Err(anyhow!("schedule_precise_timer: kevent client unavailable"));
        };
        let now = Utc::now();
        let expected = reason.expected_trigger_time.with_timezone(&Utc);
        let delay_ms = expected
            .signed_duration_since(now)
            .num_milliseconds()
            .max(1) as u64;
        let event_kind = match reason.target_type {
            TimerTargetType::Reminder => TimerEventKind::ReminderCheck,
            TimerTargetType::ScheduledTask => TimerEventKind::ScheduledTaskCheck,
            TimerTargetType::Other | TimerTargetType::Named(_) => TimerEventKind::ReminderCheck,
        };
        let timer_id = client
            .create_timer(
                event_kind.event_id(),
                TimerOptions {
                    interval_ms: delay_ms,
                    repeat: false,
                    initial_delay_ms: Some(delay_ms),
                    data: Some(serde_json::json!({
                        "target_session_id": session_id,
                        "reason": reason,
                    })),
                },
            )
            .await
            .map_err(|err| anyhow!("schedule_precise_timer: create_timer failed: {err:?}"))?;
        Ok(timer_id)
    }

    async fn ensure_self_check_hard_barrier_timer(self: Arc<Self>) {
        let Some(client) = self.runtime.kevent_client.as_ref() else {
            return;
        };
        let mut classes = self
            .config
            .toml
            .session
            .iter()
            .filter_map(|(name, cfg)| {
                if cfg.enabled && matches!(cfg.kind, SessionKind::SelfCheck) {
                    Some(name.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        let has_configured_self_check = self
            .config
            .toml
            .session
            .values()
            .any(|cfg| matches!(cfg.kind, SessionKind::SelfCheck));
        if classes.is_empty() && !has_configured_self_check {
            classes.push(self.config.class_name_for_kind(SessionKind::SelfCheck));
        }
        classes.sort();
        classes.dedup();

        for class in classes {
            let session_id = class.clone();
            let session = match self
                .clone()
                .get_or_create_session(
                    session_id.clone(),
                    "system".to_string(),
                    SessionKind::SelfCheck,
                    &class,
                )
                .await
            {
                Ok(session) => session,
                Err(err) => {
                    warn!(
                        "opendan.agent[{}]: create self_check session {} for hard barrier failed: {err:#}",
                        self.agent_name, session_id
                    );
                    continue;
                }
            };
            for event_kind in [
                TimerEventKind::ReminderCheck,
                TimerEventKind::HardBarrier,
                TimerEventKind::ScheduledTaskCheck,
            ] {
                let event_id = event_kind.event_id();
                if let Err(err) = session.subscribe_event(event_id).await {
                    warn!(
                        "opendan.agent[{}]: subscribe self_check {} failed: {err:#}",
                        self.agent_name, event_id
                    );
                }
            }
            let reason = TimerReason {
                trigger_type: TimerTriggerType::HardBarrier,
                target_type: TimerTargetType::Other,
                target_id: session_id.clone(),
                expected_trigger_time: Utc::now().fixed_offset(),
                reason: "periodic hard barrier self-check".to_string(),
            };
            if let Err(err) = client
                .create_timer(
                    TimerEventKind::HardBarrier.event_id(),
                    TimerOptions {
                        interval_ms: SELF_CHECK_HARD_BARRIER_INTERVAL_MS,
                        repeat: true,
                        initial_delay_ms: Some(1),
                        data: Some(serde_json::json!({
                            "target_session_id": session_id,
                            "reason": reason,
                        })),
                    },
                )
                .await
            {
                warn!(
                    "opendan.agent[{}]: create self_check hard barrier timer failed: {err:?}",
                    self.agent_name
                );
            }
        }
    }

    async fn ensure_ui_clock_timer(self: Arc<Self>) {
        let Some(client) = self.runtime.kevent_client.as_ref() else {
            return;
        };
        match client
            .create_timer(
                UI_CLOCK_TIMER_EVENT_ID,
                TimerOptions {
                    interval_ms: UI_CLOCK_TIMER_INTERVAL_MS,
                    repeat: true,
                    initial_delay_ms: Some(UI_CLOCK_TIMER_INTERVAL_MS),
                    data: Some(serde_json::json!({
                        "purpose": "current_clock",
                    })),
                },
            )
            .await
        {
            Ok(timer_id) => {
                info!(
                    "opendan.agent[{}]: ui clock timer started timer_id={} event_id={} interval_ms={}",
                    self.agent_name,
                    timer_id,
                    UI_CLOCK_TIMER_EVENT_ID,
                    UI_CLOCK_TIMER_INTERVAL_MS
                );
            }
            Err(err) => {
                warn!(
                    "opendan.agent[{}]: create ui clock timer failed: {err:?}",
                    self.agent_name
                );
            }
        }
    }

    pub async fn snapshot_global_state(
        &self,
        exclude_session_id: Option<&str>,
    ) -> serde_json::Value {
        let summaries = self.list_session_summaries(exclude_session_id).await;
        serde_json::json!({
            "agent_name": self.agent_name,
            "agent_id": self.agent_id(),
            "session_count": summaries.len(),
            "sessions": summaries.into_iter().map(|s| {
                serde_json::json!({
                    "session_id": s.session_id,
                    "kind": s.kind.as_str(),
                    "title": s.title,
                    "objective": s.objective,
                    "status": format!("{:?}", s.status).to_ascii_lowercase(),
                    "one_line_status": s.one_line_status,
                    "workspace_id": s.workspace_id,
                    "current_behavior": s.current_behavior,
                })
            }).collect::<Vec<_>>(),
        })
    }

    fn route_event_class(&self, event_id: &str) -> Option<String> {
        if event_id.starts_with("timer.") || event_id.starts_with("/timer/") {
            let class = self.config.class_name_for_kind(SessionKind::SelfCheck);
            self.config.session_class_enabled(&class).then_some(class)
        } else {
            None
        }
    }

    async fn session_accepts_pending(&self, session_id: &str) -> Result<bool> {
        if let Some(session) = self.sessions.lock().await.get(session_id).cloned() {
            return Ok(!matches!(
                session.meta.lock().await.status,
                SessionStatus::Ended
            ));
        }
        let meta_path = self
            .config
            .layout
            .session_dir(session_id)
            .join(".meta")
            .join("session.json");
        if !meta_path.exists() {
            return Ok(true);
        }
        let bytes = std::fs::read(&meta_path)
            .map_err(|err| anyhow!("read {} failed: {err}", meta_path.display()))?;
        let meta: SessionMeta = serde_json::from_slice(&bytes)
            .map_err(|err| anyhow!("parse {} failed: {err}", meta_path.display()))?;
        Ok(!matches!(meta.status, SessionStatus::Ended))
    }

    /// Mint or look up the session id this inbound goes to, given its
    /// resolved session class. Returns `None` when the strategy's required
    /// fields aren't present in the inbound (e.g. PerPeer with no `from`).
    fn evaluate_session_id(&self, class: &str, inbound: &Inbound) -> Option<String> {
        let cfg = self.config.session_class(class)?;
        let input = SessionIdInput::from_inbound(class, inbound)?;
        self.session_id_eval
            .compute(cfg.session_id_strategy, &input)
    }

    async fn dispatch_inbound(self: Arc<Self>, item: Inbound) -> Result<()> {
        match item {
            Inbound::Msg {
                record_id,
                from,
                from_did,
                from_name,
                tunnel_did,
                session_id,
                group_id,
                text,
                ai_message,
            } => {
                let event_type = if group_id.as_deref().is_some_and(|s| !s.is_empty()) {
                    "msg.group"
                } else {
                    "msg.chat"
                };
                let class = self.route_msg(event_type);
                let kind = self
                    .config
                    .session_class(&class)
                    .map(|c| c.kind)
                    .unwrap_or(SessionKind::Ui);
                let resolved_id = if let Some(sid) = session_id {
                    sid
                } else {
                    // Build a tiny shim Inbound so the evaluator can read
                    // from/from_did without re-borrowing the moved fields.
                    let probe = Inbound::Msg {
                        record_id: record_id.clone(),
                        from: from.clone(),
                        from_did: from_did.clone(),
                        from_name: from_name.clone(),
                        tunnel_did: tunnel_did.clone(),
                        session_id: None,
                        group_id: group_id.clone(),
                        text: String::new(),
                        ai_message: ai_message.clone(),
                    };
                    match self.evaluate_session_id(&class, &probe) {
                        Some(id) => {
                            // For UI-style classes we keep the tunnel binding
                            // so /switch and follow-up messages route to the
                            // same session.
                            if matches!(kind, SessionKind::Ui) {
                                self.tunnel_to_ui_session
                                    .lock()
                                    .await
                                    .insert(from.clone(), id.clone());
                            }
                            id
                        }
                        None => self.clone().resolve_ui_session(&from).await?,
                    }
                };
                if !self.session_accepts_pending(&resolved_id).await? {
                    warn!(
                        "opendan.agent[{}]: reject msg record_id={} for ended session {}",
                        self.agent_name, record_id, resolved_id
                    );
                    self.ack_msg_record(record_id).await;
                    return Ok(());
                }
                let session = self
                    .clone()
                    .get_or_create_session(resolved_id.clone(), from.clone(), kind, &class)
                    .await?;
                // enqueue_pending durably parks the input on the session
                // and only returns once `.meta/session.json` is on disk.
                // Once it returns we're safe to ack upstream — a crash from
                // here on leaves the input owned by the session, not lost.
                session
                    .push_msg(PendingInput::Msg {
                        record_id: record_id.clone(),
                        from,
                        from_did,
                        from_name,
                        tunnel_did,
                        text,
                        ai_message,
                    })
                    .await?;
                self.ack_msg_record(record_id).await;
                Ok(())
            }
            Inbound::Command {
                record_id,
                from,
                from_did,
                tunnel_did,
                command,
                args,
            } => {
                let invocation = crate::command_dispatcher::CommandInvocation {
                    record_id: record_id.clone(),
                    from: from.clone(),
                    from_did: from_did.clone(),
                    tunnel_did: tunnel_did.clone(),
                    command: command.clone(),
                    args,
                };
                let outcome = match crate::command_dispatcher::run_command(&self, &invocation).await
                {
                    Ok(outcome) => outcome,
                    Err(err) => crate::command_dispatcher::CommandOutcome {
                        reply: format!("/{command} failed: {err:#}"),
                    },
                };
                // Send the reply back through the tunnel that originated
                // the command. If there's no live session yet (e.g. brand
                // new tunnel that immediately typed `/help`), fall through
                // to the dispatch_command_reply helper which constructs a
                // standalone outbound message rather than parking it on a
                // session.
                self.dispatch_command_reply(
                    &from,
                    from_did.as_deref(),
                    tunnel_did.as_deref(),
                    &outcome.reply,
                )
                .await;
                self.ack_msg_record(record_id).await;
                Ok(())
            }
            Inbound::Event {
                event_id,
                target_session_id,
                data,
            } => {
                info!(
                    "opendan.agent[{}]: main_loop received event event_id={} target_session_id={:?}",
                    self.agent_name, event_id, target_session_id
                );
                // Event routing is intentionally narrow in MVP: only
                // pre-routed events (carrier sets `target_session_id`) are
                // delivered. Broadcast / pattern-matched event delivery
                // lands with `session_sub_kevent`.
                let explicit_target = target_session_id.or_else(|| self.route_event_target(&data));
                let session = if let Some(sid) = explicit_target {
                    if !self.session_accepts_pending(&sid).await? {
                        warn!(
                            "opendan.agent[{}]: reject event {} for ended session {}",
                            self.agent_name, event_id, sid
                        );
                        return Ok(());
                    }
                    match self.clone().ensure_session(&sid).await {
                        Ok(session) => session,
                        Err(err) => {
                            warn!(
                                "opendan.agent[{}]: event {} target session {} unavailable: {err:#}",
                                self.agent_name, event_id, sid
                            );
                            return Ok(());
                        }
                    }
                } else if let Some(class) = self.route_event_class(&event_id) {
                    let kind = self
                        .config
                        .session_class(&class)
                        .map(|cfg| cfg.kind)
                        .unwrap_or(SessionKind::SelfCheck);
                    if !self.session_accepts_pending(&class).await? {
                        warn!(
                            "opendan.agent[{}]: reject event {} for ended session {}",
                            self.agent_name, event_id, class
                        );
                        return Ok(());
                    }
                    self.clone()
                        .get_or_create_session(class.clone(), "system".to_string(), kind, &class)
                        .await?
                } else {
                    warn!(
                        "opendan.agent[{}]: event {} dropped — no target_session_id and broadcast routing not yet wired",
                        self.agent_name, event_id
                    );
                    return Ok(());
                };
                let full_delivery = session.notify_event(event_id.clone(), data).await?;
                info!(
                    "opendan.agent[{}]: event dispatched event_id={} session_id={} delivery={}",
                    self.agent_name,
                    event_id,
                    session.session_id,
                    if full_delivery {
                        "pending_input"
                    } else {
                        "background_only"
                    }
                );
                Ok(())
            }
        }
    }

    /// §3.3 — send a system-style reply to a slash command. Used by the
    /// command dispatcher so `/help`, `/list`, etc. ride the same tunnel
    /// the user sent the command on without parking anything on a
    /// session. Quietly skips when prerequisites (msg-center, peer DID,
    /// agent DID) are missing — the same conservative pattern
    /// `AgentSession::post_outbound_message` uses.
    async fn dispatch_command_reply(
        &self,
        _from: &str,
        from_did: Option<&str>,
        tunnel_did: Option<&str>,
        reply: &str,
    ) {
        let Some(msg_center) = self.runtime.msg_center.as_ref().cloned() else {
            return;
        };
        let Some(peer_did_str) = from_did else { return };
        let Ok(peer_did) = name_lib::DID::from_str(peer_did_str) else {
            return;
        };
        let agent_did_raw = self.config.toml.identity.agent_did.trim();
        if agent_did_raw.is_empty() {
            return;
        }
        let Ok(agent_did) = name_lib::DID::from_str(agent_did_raw) else {
            return;
        };
        if agent_did == peer_did {
            return;
        }
        // `peer_did` is the inbound sender DID, already a determined target
        // (a second-level `did:msgtunnel:*` endpoint DID for tunnel traffic),
        // so it carries its own routing — no SendContext / preferred_tunnel.
        let _ = tunnel_did;

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let mut msg = ndn_lib::MsgObject {
            from: agent_did.clone(),
            to: vec![peer_did.clone()],
            kind: ndn_lib::MsgObjKind::Chat,
            created_at_ms: now_ms,
            content: ndn_lib::MsgContent {
                format: Some(ndn_lib::MsgContentFormat::TextPlain),
                content: reply.trim().to_string(),
                ..ndn_lib::MsgContent::default()
            },
            ..Default::default()
        };
        msg.meta.insert(
            "llm_role".to_string(),
            serde_json::Value::String("system".to_string()),
        );
        msg.meta.insert(
            "parse_mode".to_string(),
            serde_json::Value::String("Plain".to_string()),
        );

        if let Err(err) = msg_center.post_send(msg, None).await {
            warn!(
                "opendan.agent[{}]: command reply post_send failed: {err}",
                self.agent_name
            );
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

    /// Resolve the UI session associated with a tunnel `from` for a
    /// slash-command invocation. Unlike `resolve_ui_session` this does
    /// **not** mint a new session id — commands operate on an existing
    /// session or fail clean. Returns the bound session id or an Err the
    /// command handler can turn into a user-visible reply.
    pub async fn resolve_session_for_command(&self, from: &str) -> Result<String> {
        if let Some(sid) = self.tunnel_to_ui_session.lock().await.get(from) {
            return Ok(sid.clone());
        }
        Err(anyhow!(
            "no session is bound to tunnel `{from}` yet — send a message first to mint one"
        ))
    }

    pub async fn create_ui_session_for_tunnel(
        self: Arc<Self>,
        from: &str,
        from_did: Option<&str>,
        tunnel_did: Option<&str>,
    ) -> Result<String> {
        let session_id = mint_session_id("ui");
        let class = self.config.class_name_for_kind(SessionKind::Ui);
        let behavior = self.config.default_behavior_for_class(&class);
        let mut seed = SessionMeta::new(
            session_id.clone(),
            SessionKind::Ui,
            behavior.clone(),
            from.to_string(),
        );
        seed.peer_did = from_did
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        seed.peer_tunnel_did = tunnel_did
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        self.clone()
            .ensure_session_inner(
                session_id.clone(),
                SessionKind::Ui,
                from.to_string(),
                Some(behavior),
                Some(seed),
            )
            .await?;
        self.bind_tunnel_to_session(from, &session_id).await;
        Ok(session_id)
    }

    pub async fn delete_session_physical(&self, session_id: &str) -> Result<bool> {
        let session = self.sessions.lock().await.remove(session_id);
        let Some(session) = session else {
            return Ok(false);
        };
        if let Some(pump) = self.event_pump.as_ref() {
            pump.remove_session(session_id).await;
        }
        let workspace_id = session.meta.lock().await.workspace_id.clone();
        session.abort_worker().await;

        if let Some(workspace_id) = workspace_id.filter(|s| !s.trim().is_empty()) {
            if workspace_id == session_id {
                remove_dir_all_if_exists(self.workspaces.workspace_dir(&workspace_id)).await?;
            } else {
                match self.workspaces.load_record(&workspace_id).await {
                    Ok(record) if record.current_session.as_deref() == Some(session_id) => {
                        if let Err(err) = self
                            .workspaces
                            .set_current_session(&workspace_id, None)
                            .await
                        {
                            warn!(
                                "opendan.agent[{}]: workspace `{}` unbind session {} failed: {err}",
                                self.agent_name, workspace_id, session_id
                            );
                        }
                    }
                    Ok(_) => {}
                    Err(err) => {
                        warn!(
                            "opendan.agent[{}]: load workspace `{}` before deleting session {} failed: {err}",
                            self.agent_name, workspace_id, session_id
                        );
                    }
                }
            }
        }

        remove_dir_all_if_exists(self.config.layout.session_dir(session_id)).await?;
        Ok(true)
    }

    pub async fn unbind_tunnel_if_session(&self, from: &str, session_id: &str) {
        let mut guard = self.tunnel_to_ui_session.lock().await;
        if guard.get(from).map(|sid| sid.as_str()) == Some(session_id) {
            guard.remove(from);
        }
    }

    /// Replace the tunnel→session binding so a `/switch <id>` command
    /// reroutes subsequent inbound messages to a different session.
    /// Does not modify the target session's state.
    pub async fn bind_tunnel_to_session(&self, from: &str, session_id: &str) {
        self.tunnel_to_ui_session
            .lock()
            .await
            .insert(from.to_string(), session_id.to_string());
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
        kind: SessionKind,
        class: &str,
    ) -> Result<Arc<AgentSession>> {
        // Note: existing session lookup is in a separate scope so we can drop
        // the lock before doing the (potentially expensive) tool manager
        // bootstrap on a miss.
        if let Some(s) = self.sessions.lock().await.get(&session_id).cloned() {
            return Ok(s);
        }
        let behavior_hint = Some(self.config.default_behavior_for_class(class));
        self.ensure_session_inner(session_id, kind, owner, behavior_hint, None)
            .await
    }

    async fn session_build_lock(&self, session_id: &str) -> Arc<Mutex<()>> {
        let mut locks = self.session_locks.lock().await;
        locks
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    pub async fn ensure_session(self: Arc<Self>, session_id: &str) -> Result<Arc<AgentSession>> {
        if let Some(s) = self.sessions.lock().await.get(session_id).cloned() {
            return Ok(s);
        }
        let meta_path = self
            .config
            .layout
            .session_dir(session_id)
            .join(".meta")
            .join("session.json");
        let bytes = std::fs::read(&meta_path)
            .map_err(|err| anyhow!("ensure_session: read {} failed: {err}", meta_path.display()))?;
        let meta: SessionMeta = serde_json::from_slice(&bytes).map_err(|err| {
            anyhow!(
                "ensure_session: parse {} failed: {err}",
                meta_path.display()
            )
        })?;
        let kind = meta.kind;
        let owner = meta.owner.clone();
        let behavior = meta.current_behavior.clone();
        self.ensure_session_inner(
            session_id.to_string(),
            kind,
            owner,
            Some(behavior),
            Some(meta),
        )
        .await
    }

    pub async fn retire_idle_session(&self, session_id: &str) {
        self.sessions.lock().await.remove(session_id);
    }

    pub async fn retire_ended_session(&self, session_id: &str) {
        let removed = self.sessions.lock().await.remove(session_id);
        if removed.is_some() {
            if let Some(pump) = self.event_pump.as_ref() {
                pump.remove_session(session_id).await;
            }
        }
    }

    pub(crate) async fn ensure_session_inner(
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
        let build_lock = self.session_build_lock(&session_id).await;
        let _guard = build_lock.lock().await;
        {
            let map = self.sessions.lock().await;
            if let Some(s) = map.get(&session_id) {
                return Ok(s.clone());
            }
        }
        let session_dir = self.config.layout.session_dir(&session_id);
        let _ = std::fs::create_dir_all(&session_dir);
        let mut existing_meta = existing_meta;
        let workspace_rec = if kind.is_work_family() {
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
            Some(workspace_rec)
        } else {
            None
        };
        let tool_root = workspace_rec
            .as_ref()
            .map(|rec| self.workspaces.workspace_dir(&rec.workspace_id))
            .unwrap_or_else(|| session_dir.clone());

        // Resolve the behavior name up-front so we can pull its tool plan
        // before building the tool manager. (`behavior_hint` wins so a
        // restoring session keeps the same behavior; otherwise look up
        // the session class default via the on-disk `[session.<class>]`
        // table — falling back to the canonical `<class>_default` name.)
        let behavior_name = behavior_hint.unwrap_or_else(|| {
            let class = self.config.class_name_for_kind(kind);
            self.config.default_behavior_for_class(&class)
        });
        if let Some(meta) = existing_meta.as_mut() {
            if meta.status_changed_at_ms == 0 {
                meta.status_changed_at_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
            }
        }
        let agent_id = self.agent_tool_mount_id();
        let bin_renderer = self.build_session_bin_renderer(&agent_id, &session_id, &behavior_name);
        let appclient_session_token = resolve_appclient_session_token().await?;

        let tools = build_session_tools(SessionToolsBuild {
            workspace_root: tool_root.clone(),
            session_dir: session_dir.clone(),
            agent_root: self.config.layout.root.clone(),
            agent_id: agent_id.clone(),
            session_id: session_id.clone(),
            appclient_session_token,
            filesystem_policy: self.config.toml.runtime.filesystem_policy,
            bin_renderer,
        })
        .map_err(|err| anyhow!("build session tools: {err}"))?;
        // Worksession control tools (create_worksession / forward_msg) are
        // registered on every session — visibility is gated by the
        // behavior whitelist downstream.
        crate::worksession_tools::register_worksession_tools(
            &tools,
            Arc::downgrade(&self),
            &session_id,
        );
        crate::buildin_tool::register_event_subscription_tools(
            &tools,
            Arc::downgrade(&self),
            &session_id,
        );
        crate::buildin_tool::register_session_history_tools(&tools, Arc::downgrade(&self));

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
            event_pump: self.event_pump.clone(),
            parent_agent: Arc::downgrade(&self),
        });
        let session = Arc::new(session);
        {
            let mut meta = session.meta.lock().await;
            if meta.status_changed_at_ms == 0 {
                meta.status_changed_at_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
            }
        }
        if let Some(workspace_rec) = workspace_rec.as_ref() {
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
        }
        if let Err(err) = session.flush_meta().await {
            warn!(
                "opendan.agent[{}]: initial flush_meta for session {} failed: {err:#}",
                self.agent_name, session_id
            );
        }
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
                    SessionReply::Error { message } => {
                        warn!("opendan.agent[{agent_name}]: session={log_sid} error: {message}");
                    }
                    SessionReply::Ended => {
                        info!("opendan.agent[{agent_name}]: session={log_sid} ended");
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
        session.clone().start(inbox_rx).await;
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

    /// Create a Work session bound to a workspace and start its worker.
    /// Used by the `create_worksession` tool. Returns enough info for the
    /// caller (typically the sub-LLM context from `try_create_worksession`)
    /// to report back to the parent UI session.
    pub async fn create_work_session(
        self: Arc<Self>,
        params: CreateWorkSessionParams,
    ) -> Result<CreateWorkSessionOutcome> {
        let CreateWorkSessionParams {
            mut title,
            mut objective,
            mut workspace_id,
            behavior,
            created_by_session_id,
            mut reason_messages,
            mut task_binding,
            task_id,
            auto_start,
            bind_task,
        } = params;
        if let Some(task_id) = task_id {
            let Some(client) = self.runtime.task_mgr.as_ref().cloned() else {
                return Err(anyhow!(
                    "create_work_session: task_id was specified but task manager is unavailable"
                ));
            };
            let task = client
                .get_task(task_id)
                .await
                .map_err(|err| anyhow!("create_work_session: load task {task_id}: {err}"))?;
            let defaults = work_session_defaults_from_task(&task);
            if title.trim().is_empty() {
                title = defaults.title;
            }
            if objective.trim().is_empty() {
                objective = defaults.objective;
            }
            if workspace_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none()
            {
                workspace_id = defaults.workspace_id;
            }
            reason_messages.push(format!(
                "worksession created from task {} ({})",
                task.id, task.task_type
            ));
            task_binding = Some(agent_task_binding_from_task(&task));
        }
        if objective.trim().is_empty() {
            return Err(anyhow!("create_work_session: objective must not be empty"));
        }
        let behavior = behavior.and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });
        let new_session_id = self.allocate_worksession_id(title.trim()).await;
        // Workspace resolution: explicit id reuses; absence mints a readable
        // id from the task title/name, with a numeric suffix only on conflict.
        let (workspace_id, workspace_status) = match workspace_id {
            Some(id) if !id.trim().is_empty() => match self.workspaces.load_record(&id).await {
                Ok(_) => (id, "reused".to_string()),
                Err(_) => {
                    // Either NotFound or unreadable — treat as create-with-id
                    // request so the LLM's intent is honored.
                    self.workspaces
                        .create_or_open(&id, &id, Some(&new_session_id))
                        .await
                        .map_err(|err| anyhow!("workspace create_or_open: {err}"))?;
                    (id, "created".to_string())
                }
            },
            _ => {
                let name = meaningful_workspace_name(title.trim(), objective.trim());
                let new_ws_id = self.allocate_workspace_id(&name).await;
                self.workspaces
                    .create_or_open(&new_ws_id, &name, Some(&new_session_id))
                    .await
                    .map_err(|err| anyhow!("workspace create_or_open: {err}"))?;
                (new_ws_id, "created".to_string())
            }
        };
        if bind_task && task_binding.is_none() {
            match self
                .create_task_for_work_session(
                    &new_session_id,
                    title.trim(),
                    objective.trim(),
                    &workspace_id,
                    &created_by_session_id,
                )
                .await
            {
                Ok(binding) => task_binding = Some(binding),
                Err(err) => warn!(
                    "opendan.agent[{}]: create task for worksession {} failed: {err:#}",
                    self.agent_name, new_session_id
                ),
            }
        }

        // Seed an existing_meta so the session is created with the right
        // title / objective / kind / workspace binding before its first
        // worker tick.
        let mut seed = SessionMeta::new(
            new_session_id.clone(),
            SessionKind::Work,
            behavior.clone().unwrap_or_else(|| {
                let class = self.config.class_name_for_kind(SessionKind::Work);
                self.config.default_behavior_for_class(&class)
            }),
            created_by_session_id.clone(),
        );
        seed.title = title.trim().to_string();
        seed.objective = objective.trim().to_string();
        seed.workspace_id = Some(workspace_id.clone());
        seed.task_binding = task_binding;
        let task_binding_for_update = seed.task_binding.clone();

        // Write a readme.md capturing the origin context so a later
        // human / debugger can see why this work session exists.
        let session_dir = self.config.layout.session_dir(&new_session_id);
        let _ = std::fs::create_dir_all(&session_dir);
        write_worksession_readme(
            &session_dir,
            &seed.title,
            &seed.objective,
            &created_by_session_id,
            &reason_messages,
        );

        let behavior_name = seed.current_behavior.clone();
        let session = self
            .clone()
            .ensure_session_inner(
                new_session_id.clone(),
                SessionKind::Work,
                created_by_session_id.clone(),
                Some(behavior_name.clone()),
                Some(seed),
            )
            .await?;
        if auto_start {
            // Wake the worker so the bootstrap turn runs even before the
            // first external input — `needs_bootstrap_turn` will fire on the
            // freshly-objective-bearing meta.
            session.wake().await;
        }
        if let Some(binding) = task_binding_for_update.as_ref() {
            self.update_work_session_task_started(
                binding,
                &new_session_id,
                &workspace_id,
                &workspace_status,
                &behavior_name,
                auto_start,
            )
            .await;
        }

        Ok(CreateWorkSessionOutcome {
            session_id: new_session_id,
            title: title.trim().to_string(),
            workspace_id,
            workspace_status,
            behavior: behavior_name,
            auto_started: auto_start,
            task_id: task_binding_for_update
                .as_ref()
                .map(|binding| binding.task_id),
        })
    }

    async fn create_task_for_work_session(
        &self,
        session_id: &str,
        title: &str,
        objective: &str,
        workspace_id: &str,
        created_by_session_id: &str,
    ) -> Result<AgentTaskBinding> {
        let Some(task_mgr) = self.runtime.task_mgr.as_ref().cloned() else {
            return Err(anyhow!("task manager unavailable"));
        };
        let (user_id, app_id) = self.default_task_identity(created_by_session_id);
        let task_name = if title.trim().is_empty() {
            meaningful_workspace_name(title, objective)
        } else {
            title.trim().to_string()
        };
        let runner = self.task_executor_runner_id()?;
        let task = task_mgr
            .create_task(
                &task_name,
                TASK_TYPE_AGENT_DELEGATE,
                Some(serde_json::to_value(AgentDelegateTaskData {
                    request: AgentDelegateTaskRequest {
                        version: 1,
                        purpose: Some(objective.to_string()),
                        title: Some(title.to_string()),
                        requester_agent_id: Some(self.agent_id()),
                        owner_session_id: Some(created_by_session_id.to_string()),
                        input: Some(serde_json::json!({
                            "text": objective
                        })),
                        workspace_hints: vec![serde_json::json!({
                            "workspace_id": workspace_id
                        })],
                        ..Default::default()
                    },
                    progress: Some(AgentDelegateProgress {
                        execution: Some(serde_json::json!({
                            "session_id": session_id,
                            "workspace_id": workspace_id,
                            "runner": runner.clone(),
                            "status": "creating"
                        })),
                        one_line_status: None,
                        updated_at_ms: Some(Utc::now().timestamp_millis()),
                    }),
                    result: None,
                    route: None,
                    blocker: None,
                    human_input: None,
                    error: None,
                })?),
                &user_id,
                &app_id,
                Some(CreateTaskOptions {
                    session_id: Some(session_id.to_string()),
                    runner: Some(runner),
                    ..Default::default()
                }),
            )
            .await
            .map_err(|err| anyhow!("create task: {err}"))?;
        Ok(agent_task_binding_from_task(&task))
    }

    async fn update_work_session_task_started(
        &self,
        binding: &AgentTaskBinding,
        session_id: &str,
        workspace_id: &str,
        workspace_status: &str,
        behavior: &str,
        auto_started: bool,
    ) {
        let Some(task_mgr) = self.runtime.task_mgr.as_ref().cloned() else {
            return;
        };
        let status = if auto_started {
            Some(buckyos_api::TaskStatus::Running)
        } else {
            None
        };
        let progress = if auto_started { Some(10.0) } else { None };
        let message = if auto_started {
            "Agent session started"
        } else {
            "Agent session created"
        };
        let runner = match self.task_executor_runner_id() {
            Ok(runner) => runner,
            Err(err) => {
                warn!(
                    "opendan.agent[{}]: cannot resolve task executor runner for task {} update: {err:#}",
                    self.agent_name, binding.task_id
                );
                return;
            }
        };
        let execution_status = if auto_started { "running" } else { "idle" };
        let data = match task_mgr.get_task(binding.task_id).await {
            Ok(task) => {
                let mut data = match agent_delegate_task_data(&task) {
                    Some(data) => data,
                    None => {
                        warn!(
                            "opendan.agent[{}]: worksession task {} has invalid agent.delegate data",
                            self.agent_name, binding.task_id
                        );
                        return;
                    }
                };
                data.progress
                    .get_or_insert_with(AgentDelegateProgress::default)
                    .execution = Some(serde_json::json!({
                    "session_id": session_id,
                    "workspace_id": workspace_id,
                    "workspace_status": workspace_status,
                    "behavior": behavior,
                    "runner": runner,
                    "status": execution_status
                }));
                if let Some(progress) = data.progress.as_mut() {
                    progress.updated_at_ms = Some(Utc::now().timestamp_millis());
                }
                match serde_json::to_value(data) {
                    Ok(value) => value,
                    Err(err) => {
                        warn!(
                            "opendan.agent[{}]: serialize worksession task {} data failed: {err:#}",
                            self.agent_name, binding.task_id
                        );
                        return;
                    }
                }
            }
            Err(err) => {
                warn!(
                    "opendan.agent[{}]: load worksession task {} for update failed: {err:#}",
                    self.agent_name, binding.task_id
                );
                return;
            }
        };
        if let Err(err) = task_mgr
            .update_task(
                binding.task_id,
                status,
                progress,
                Some(message.to_string()),
                Some(data),
            )
            .await
        {
            warn!(
                "opendan.agent[{}]: update worksession task {} started failed: {err:#}",
                self.agent_name, binding.task_id
            );
        }
    }

    fn default_task_identity(&self, created_by_session_id: &str) -> (String, String) {
        if let Ok(runtime) = get_buckyos_api_runtime() {
            let user_id = runtime
                .get_owner_user_id()
                .or_else(|| runtime.user_id.clone())
                .unwrap_or_else(|| created_by_session_id.to_string());
            return (user_id, runtime.get_app_id());
        }
        let fallback_user = if created_by_session_id.trim().is_empty() {
            self.agent_id()
        } else {
            created_by_session_id.to_string()
        };
        (fallback_user, self.agent_id())
    }

    pub(crate) async fn allocate_workspace_id(&self, name: &str) -> String {
        let base = sanitize_worksession_title(name);
        let mut suffix = 1usize;
        loop {
            let candidate = if suffix == 1 {
                base.clone()
            } else {
                format!("{base} {suffix}")
            };
            if !self.workspaces.workspace_dir(&candidate).exists() {
                return candidate;
            }
            suffix += 1;
        }
    }

    async fn allocate_worksession_id(&self, title: &str) -> String {
        let date = Local::now().format("%Y-%m-%d").to_string();
        let base = build_worksession_id(&date, title);
        let mut suffix = 1usize;
        loop {
            let candidate = if suffix == 1 {
                base.clone()
            } else {
                format!("{base} {suffix}")
            };
            let in_memory = self.sessions.lock().await.contains_key(&candidate);
            if !in_memory && !self.config.layout.session_dir(&candidate).exists() {
                return candidate;
            }
            suffix += 1;
        }
    }

    /// Forward a chat message from one session to another's pending queue.
    /// Returns the synthetic `record_id` used for the forwarded entry so
    /// the caller can include it in its tool response. Errors when:
    ///   - target session doesn't exist or isn't a Work session
    ///   - target session has already Ended
    pub async fn forward_message(
        &self,
        target_session_id: &str,
        source_session_id: &str,
        text: &str,
    ) -> Result<String> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err(anyhow!("forward_message: text is empty"));
        }
        let target = {
            let map = self.sessions.lock().await;
            map.get(target_session_id).cloned()
        };
        let Some(target) = target else {
            return Err(anyhow!(
                "forward_message: target session `{target_session_id}` not found"
            ));
        };
        if !matches!(target.kind, SessionKind::Work) {
            return Err(anyhow!(
                "forward_message: target session `{target_session_id}` is not a work session"
            ));
        }
        let status = target.meta.lock().await.status;
        if matches!(status, SessionStatus::Ended) {
            let summary = target.summary().await;
            return Err(anyhow!("{}", ended_forward_message_guidance(&summary)));
        }
        let record_id = format!("forward:{source_session_id}:{}", mint_session_id("fwd"));
        target
            .enqueue_pending(PendingInput::Msg {
                record_id: record_id.clone(),
                from: source_session_id.to_string(),
                from_did: None,
                from_name: None,
                tunnel_did: None,
                text: trimmed.to_string(),
                ai_message: AiMessage::text(AiRole::User, trimmed.to_string()),
            })
            .await?;
        Ok(record_id)
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

fn ended_forward_message_guidance(summary: &SessionSummary) -> String {
    let mut context = Vec::new();
    if !summary.title.trim().is_empty() {
        context.push(format!("title `{}`", summary.title.trim()));
    }
    if !summary.objective.trim().is_empty() {
        context.push(format!("objective `{}`", summary.objective.trim()));
    }
    if let Some(workspace_id) = summary
        .workspace_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        context.push(format!("workspace `{workspace_id}`"));
    }
    let context = if context.is_empty() {
        String::new()
    } else {
        format!(" Previous context: {}.", context.join(", "))
    };
    format!(
        "forward_message: target session `{}` has ended and cannot accept forwarded messages.{} Do not retry `forward_msg` for this session. Fork/select a replacement with `try_create_worksession` for the current user follow-up, then forward to the new session id returned in `followup_routing.target_worksession_id`.",
        summary.session_id, context
    )
}

/// Parameters for [`AIAgent::create_work_session`]. Mirrors the §8.1
/// `create_worksession` tool args, but in Rust-native form so non-LLM
/// callers (CLI, tests) can build it directly.
#[derive(Debug, Clone)]
pub struct CreateWorkSessionParams {
    pub title: String,
    pub objective: String,
    pub workspace_id: Option<String>,
    pub behavior: Option<String>,
    pub created_by_session_id: String,
    pub reason_messages: Vec<String>,
    pub task_binding: Option<AgentTaskBinding>,
    pub task_id: Option<i64>,
    pub auto_start: bool,
    pub bind_task: bool,
}

/// Result of [`AIAgent::create_work_session`].
#[derive(Debug, Clone, serde::Serialize)]
pub struct CreateWorkSessionOutcome {
    pub session_id: String,
    pub title: String,
    pub workspace_id: String,
    /// Always `"created"` or `"reused"` so downstream tooling can branch
    /// without parsing free-form text.
    pub workspace_status: String,
    pub behavior: String,
    pub auto_started: bool,
    pub task_id: Option<i64>,
}

struct WorkSessionTaskDefaults {
    title: String,
    objective: String,
    workspace_id: Option<String>,
}

fn work_session_defaults_from_task(task: &Task) -> WorkSessionTaskDefaults {
    let data = agent_delegate_task_data(task);
    let title = data
        .as_ref()
        .and_then(|data| data.request.title.as_deref())
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| task.name.clone());
    let objective = data
        .as_ref()
        .and_then(|data| data.request.purpose.as_deref())
        .or_else(|| {
            data.as_ref()
                .and_then(|data| data.request.input.as_ref())
                .and_then(|input| input.get("text"))
                .and_then(serde_json::Value::as_str)
        })
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| task.name.clone());
    let workspace_id = data
        .as_ref()
        .and_then(|data| data.route.as_ref())
        .and_then(|route| route.get("workspace_id"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            data.as_ref()
                .and_then(|data| data.progress.as_ref())
                .and_then(|progress| progress.execution.as_ref())
                .and_then(|execution| execution.get("workspace_id"))
                .and_then(serde_json::Value::as_str)
        })
        .or_else(|| {
            data.as_ref()
                .and_then(|data| data.request.input.as_ref())
                .and_then(|input| input.get("workspace_id"))
                .and_then(serde_json::Value::as_str)
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            data.as_ref().and_then(|data| {
                if data.request.workspace_hints.len() == 1 {
                    data.request
                        .workspace_hints
                        .first()
                        .and_then(workspace_id_from_task_hint)
                } else {
                    None
                }
            })
        });
    WorkSessionTaskDefaults {
        title,
        objective,
        workspace_id,
    }
}

fn agent_delegate_task_data(task: &Task) -> Option<AgentDelegateTaskData> {
    match parse_typed_task_data(task.task_type.as_str(), task.data.clone()).ok()? {
        TypedTaskData::AgentDelegate(data) => Some(data),
        _ => None,
    }
}

fn workspace_id_from_task_hint(value: &serde_json::Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_string)
        .or_else(|| {
            value
                .get("workspace_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| {
            value
                .get("id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
}

pub(crate) fn agent_task_binding_from_task(task: &Task) -> AgentTaskBinding {
    AgentTaskBinding {
        task_id: task.id,
        root_task_id: task.root_id.parse::<i64>().unwrap_or(task.id),
        root_id: task.root_id.clone(),
        task_type: task.task_type.clone(),
        runner: task.runner.clone(),
        task_name: task.name.clone(),
        user_id: task.user_id.clone(),
        app_id: task.app_id.clone(),
        parent_id: task.parent_id,
    }
}

/// Mint a stable, short id with a prefix. Uses uuid v4 so concurrent
/// callers don't collide. The prefix is informational — it makes
/// debugging easier (`ws-...`, `fwd-...`).
pub fn mint_session_id(prefix: &str) -> String {
    let short = uuid::Uuid::new_v4().simple().to_string();
    // Keep only the first 12 hex chars — full uuid is 32, which is
    // visually noisy in logs and meta files.
    let short = short.get(..12).unwrap_or(&short).to_string();
    format!("{prefix}-{short}")
}

fn build_worksession_id(date: &str, title: &str) -> String {
    let readable_title = sanitize_worksession_title(title);
    format!("{date} {readable_title}")
}

fn meaningful_workspace_name(title: &str, objective: &str) -> String {
    let source = if title.trim().is_empty() {
        objective
    } else {
        title
    };
    sanitize_worksession_title(source)
}

fn sanitize_worksession_title(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len().min(80));
    let mut prev_space = false;
    for ch in raw.trim().chars() {
        let replacement = if ch.is_control()
            || matches!(ch, '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*')
        {
            ' '
        } else {
            ch
        };
        if replacement.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
            continue;
        }
        out.push(replacement);
        prev_space = false;
    }
    let trimmed = out.trim_matches([' ', '.']).to_string();
    if trimmed.is_empty() {
        "未命名任务".to_string()
    } else {
        trimmed
    }
}

/// Render the work-session readme. Captures title / objective / origin
/// session / reason messages so a later reader can reconstruct context
/// without grovelling through agent logs.
fn write_worksession_readme(
    session_dir: &std::path::Path,
    title: &str,
    objective: &str,
    created_by: &str,
    reason_messages: &[String],
) {
    let mut buf = String::new();
    if !title.is_empty() {
        buf.push_str(&format!("# {title}\n\n"));
    } else {
        buf.push_str("# Work session\n\n");
    }
    if !objective.is_empty() {
        buf.push_str("## Objective\n");
        buf.push_str(objective);
        buf.push_str("\n\n");
    }
    buf.push_str(&format!("## Origin\nCreated by session `{created_by}`.\n"));
    if !reason_messages.is_empty() {
        buf.push_str("\n## Reason messages\n");
        for (i, m) in reason_messages.iter().enumerate() {
            buf.push_str(&format!("{}. {}\n", i + 1, m.trim()));
        }
    }
    let path = session_dir.join("readme.md");
    if let Err(err) = std::fs::write(&path, buf) {
        warn!("opendan.agent: write {} failed: {err}", path.display());
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

async fn remove_dir_all_if_exists(path: PathBuf) -> Result<()> {
    match tokio::fs::remove_dir_all(&path).await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(anyhow!("remove {} failed: {err}", path.display())),
    }
}

fn current_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
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
    use crate::agent_session::WorksessionReportPhase;
    use crate::worklog::{WorklogService, WorklogToolConfig};
    use async_trait::async_trait;
    use buckyos_api::{
        AiMethodRequest, AiMethodResponse, AiccClient, AiccHandler, CancelResponse, TaskFilter,
        TaskManagerClient, TaskManagerHandler, TaskNote, TaskPermissions, TaskStatus,
    };
    use kRPC::{RPCContext, RPCErrors};
    use std::ops::Range;
    use std::sync::atomic::{AtomicI64, Ordering};

    #[test]
    fn sanitizes_session_segment() {
        assert_eq!(sanitize_session_segment("did:dev:alice"), "did_dev_alice");
        assert_eq!(sanitize_session_segment(""), "anon");
    }

    #[test]
    fn builds_human_readable_worksession_id() {
        assert_eq!(
            build_worksession_id("2026-05-20", "开发网页版连连看小游戏"),
            "2026-05-20 开发网页版连连看小游戏"
        );
    }

    #[test]
    fn sanitizes_worksession_title_for_directory_name() {
        assert_eq!(
            build_worksession_id("2026-05-20", "  a/b\\c:d*e? \n f  "),
            "2026-05-20 a b c d e f"
        );
        assert_eq!(
            build_worksession_id("2026-05-20", "../"),
            "2026-05-20 未命名任务"
        );
    }

    #[test]
    fn meaningful_workspace_name_prefers_title_then_objective() {
        assert_eq!(
            meaningful_workspace_name("Frontend Snake Game", "ignored"),
            "Frontend Snake Game"
        );
        assert_eq!(
            meaningful_workspace_name("", "Build a pure front-end Snake mini-game"),
            "Build a pure front-end Snake mini-game"
        );
    }

    #[test]
    fn truncate_short_string() {
        assert_eq!(truncate("hi", 10), "hi");
    }

    #[test]
    fn truncate_long_string() {
        assert_eq!(truncate("abcdefghij", 4), "abcd…");
    }

    #[test]
    fn forward_route_selects_latest_waiting_work_session() {
        let candidates = vec![
            ForwardRouteCandidate {
                session_id: "work-old".to_string(),
                kind: SessionKind::Work,
                status: SessionStatus::WaitingInput,
                status_changed_at_ms: 10,
            },
            ForwardRouteCandidate {
                session_id: "work-new".to_string(),
                kind: SessionKind::Work,
                status: SessionStatus::WaitingInput,
                status_changed_at_ms: 20,
            },
            ForwardRouteCandidate {
                session_id: "ui".to_string(),
                kind: SessionKind::Ui,
                status: SessionStatus::WaitingInput,
                status_changed_at_ms: 30,
            },
        ];
        assert_eq!(
            decide_forward_route(
                ForwardRouteStrategy::MostRecentWaitingInput,
                None,
                false,
                &candidates
            ),
            ForwardRouteDecision::Forward("work-new".to_string())
        );
    }

    #[test]
    fn forward_route_does_not_implicit_forward_to_running_work() {
        let candidates = vec![ForwardRouteCandidate {
            session_id: "work-running".to_string(),
            kind: SessionKind::Work,
            status: SessionStatus::Running,
            status_changed_at_ms: 20,
        }];
        assert_eq!(
            decide_forward_route(
                ForwardRouteStrategy::MostRecentWaitingInput,
                None,
                false,
                &candidates
            ),
            ForwardRouteDecision::KeepInUi
        );
    }

    #[test]
    fn forward_route_interrupt_creates_new_work_session() {
        assert_eq!(
            decide_forward_route(
                ForwardRouteStrategy::NewWorkSessionOnInterrupt,
                None,
                true,
                &[]
            ),
            ForwardRouteDecision::CreateNewWorkSession
        );
    }

    struct NoopAicc;

    #[async_trait]
    impl AiccHandler for NoopAicc {
        async fn handle_method(
            &self,
            method: &str,
            _request: AiMethodRequest,
            _ctx: RPCContext,
        ) -> std::result::Result<AiMethodResponse, RPCErrors> {
            Err(RPCErrors::UnknownMethod(method.to_string()))
        }

        async fn handle_cancel(
            &self,
            task_id: &str,
            _ctx: RPCContext,
        ) -> std::result::Result<CancelResponse, RPCErrors> {
            Ok(CancelResponse::new(task_id.to_string(), false))
        }
    }

    fn ensure_test_task_runner_config(root: &PathBuf) {
        std::fs::create_dir_all(root).expect("mkdir agent root");
        let path = root.join("agent.toml");
        if path.exists() {
            return;
        }
        std::fs::write(
            path,
            r#"
[runtime.task_executor]
runner_id = "agent"
"#,
        )
        .expect("write test agent.toml");
    }

    fn test_agent(root: PathBuf) -> Arc<AIAgent> {
        ensure_test_task_runner_config(&root);
        let worklog = WorklogService::new(WorklogToolConfig::with_db_path(root.join("worklog.db")))
            .expect("create worklog");
        let runtime = AgentRuntime::new(
            Arc::new(AiccClient::new_in_process(Box::new(NoopAicc))),
            Arc::new(worklog),
        );
        AIAgent::open(root, Arc::new(runtime)).expect("open test agent")
    }

    fn test_agent_with_task_mgr(root: PathBuf, task_mgr: MemoryTaskMgr) -> Arc<AIAgent> {
        ensure_test_task_runner_config(&root);
        let worklog = WorklogService::new(WorklogToolConfig::with_db_path(root.join("worklog.db")))
            .expect("create worklog");
        let runtime = AgentRuntime::new(
            Arc::new(AiccClient::new_in_process(Box::new(NoopAicc))),
            Arc::new(worklog),
        )
        .with_task_mgr(Arc::new(TaskManagerClient::new_in_process(Box::new(
            task_mgr,
        ))));
        AIAgent::open(root, Arc::new(runtime)).expect("open test agent")
    }

    #[derive(Clone, Default)]
    struct MemoryTaskMgr {
        inner: Arc<MemoryTaskMgrInner>,
    }

    #[derive(Default)]
    struct MemoryTaskMgrInner {
        next_id: AtomicI64,
        next_note_id: AtomicI64,
        tasks: std::sync::Mutex<Vec<Task>>,
        notes: std::sync::Mutex<Vec<TaskNote>>,
    }

    impl MemoryTaskMgr {
        fn insert(&self, mut task: Task) -> Task {
            if task.id == 0 {
                task.id = self.inner.next_id.fetch_add(1, Ordering::Relaxed) + 1;
            }
            if task.root_id.is_empty() {
                task.root_id = task.id.to_string();
            }
            self.inner.tasks.lock().unwrap().push(task.clone());
            task
        }

        fn task(&self, id: i64) -> Task {
            self.inner
                .tasks
                .lock()
                .unwrap()
                .iter()
                .find(|task| task.id == id)
                .cloned()
                .expect("task")
        }
    }

    #[async_trait]
    impl TaskManagerHandler for MemoryTaskMgr {
        async fn handle_create_task(
            &self,
            name: &str,
            task_type: &str,
            data: Option<serde_json::Value>,
            opts: CreateTaskOptions,
            user_id: &str,
            app_id: &str,
            _ctx: RPCContext,
        ) -> std::result::Result<Task, RPCErrors> {
            let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed) + 1;
            let task = Task {
                id,
                user_id: user_id.to_string(),
                app_id: app_id.to_string(),
                session_id: opts.session_id.unwrap_or_default(),
                parent_id: opts.parent_id,
                root_id: opts.root_id.unwrap_or_else(|| id.to_string()),
                name: name.to_string(),
                task_type: task_type.to_string(),
                runner: opts.runner.unwrap_or_default(),
                status: TaskStatus::Pending,
                progress: 0.0,
                message: None,
                data: data.unwrap_or_else(|| serde_json::json!({})),
                permissions: opts.permissions.unwrap_or_default(),
                created_at: 1,
                updated_at: 1,
            };
            self.inner.tasks.lock().unwrap().push(task.clone());
            Ok(task)
        }

        async fn handle_get_task(
            &self,
            id: i64,
            _ctx: RPCContext,
        ) -> std::result::Result<Task, RPCErrors> {
            self.inner
                .tasks
                .lock()
                .unwrap()
                .iter()
                .find(|task| task.id == id)
                .cloned()
                .ok_or_else(|| RPCErrors::ReasonError(format!("task {id} not found")))
        }

        async fn handle_add_task_note(
            &self,
            task_id: i64,
            note_type: Option<&str>,
            content: &str,
            data: Option<serde_json::Value>,
            source_user_id: Option<&str>,
            source_app_id: Option<&str>,
            _ctx: RPCContext,
        ) -> std::result::Result<TaskNote, RPCErrors> {
            let task = self
                .inner
                .tasks
                .lock()
                .unwrap()
                .iter()
                .find(|task| task.id == task_id)
                .cloned()
                .ok_or_else(|| RPCErrors::ReasonError(format!("task {task_id} not found")))?;
            let id = self.inner.next_note_id.fetch_add(1, Ordering::Relaxed) + 1;
            let note = TaskNote {
                id,
                task_id,
                note_type: note_type.unwrap_or("human").to_string(),
                content: content.to_string(),
                data: data.unwrap_or_else(|| serde_json::json!({})),
                author_user_id: source_user_id.unwrap_or(&task.user_id).to_string(),
                author_app_id: source_app_id.unwrap_or(&task.app_id).to_string(),
                created_at: 1,
                updated_at: 1,
            };
            self.inner.notes.lock().unwrap().push(note.clone());
            Ok(note)
        }

        async fn handle_list_task_notes(
            &self,
            task_id: i64,
            _source_user_id: Option<&str>,
            _source_app_id: Option<&str>,
            _ctx: RPCContext,
        ) -> std::result::Result<Vec<TaskNote>, RPCErrors> {
            Ok(self
                .inner
                .notes
                .lock()
                .unwrap()
                .iter()
                .filter(|note| note.task_id == task_id)
                .cloned()
                .collect())
        }

        async fn handle_list_tasks(
            &self,
            filter: TaskFilter,
            _source_user_id: Option<&str>,
            _source_app_id: Option<&str>,
            _ctx: RPCContext,
        ) -> std::result::Result<Vec<Task>, RPCErrors> {
            Ok(self
                .inner
                .tasks
                .lock()
                .unwrap()
                .iter()
                .filter(|task| {
                    filter
                        .task_type
                        .as_ref()
                        .map(|value| task.task_type == *value)
                        .unwrap_or(true)
                        && filter
                            .runner
                            .as_ref()
                            .map(|value| task.runner == *value)
                            .unwrap_or(true)
                        && filter
                            .status
                            .map(|value| task.status == value)
                            .unwrap_or(true)
                })
                .cloned()
                .collect())
        }

        async fn handle_list_tasks_by_time_range(
            &self,
            _app_id: Option<&str>,
            _session_id: Option<&str>,
            _task_type: Option<&str>,
            _source_user_id: Option<&str>,
            _source_app_id: Option<&str>,
            _time_range: Range<u64>,
            _ctx: RPCContext,
        ) -> std::result::Result<Vec<Task>, RPCErrors> {
            Ok(Vec::new())
        }

        async fn handle_get_subtasks(
            &self,
            parent_id: i64,
            _ctx: RPCContext,
        ) -> std::result::Result<Vec<Task>, RPCErrors> {
            Ok(self
                .inner
                .tasks
                .lock()
                .unwrap()
                .iter()
                .filter(|task| task.parent_id == Some(parent_id))
                .cloned()
                .collect())
        }

        async fn handle_update_task(
            &self,
            id: i64,
            status: Option<TaskStatus>,
            progress: Option<f32>,
            message: Option<String>,
            data: Option<serde_json::Value>,
            _ctx: RPCContext,
        ) -> std::result::Result<(), RPCErrors> {
            let mut tasks = self.inner.tasks.lock().unwrap();
            let task = tasks
                .iter_mut()
                .find(|task| task.id == id)
                .ok_or_else(|| RPCErrors::ReasonError(format!("task {id} not found")))?;
            if let Some(status) = status {
                task.status = status;
            }
            if let Some(progress) = progress {
                task.progress = progress;
            }
            if let Some(message) = message {
                task.message = Some(message);
            }
            if let Some(data) = data {
                merge_json_test(&mut task.data, &data);
            }
            Ok(())
        }

        async fn handle_update_task_progress(
            &self,
            id: i64,
            completed_items: u64,
            total_items: u64,
            ctx: RPCContext,
        ) -> std::result::Result<(), RPCErrors> {
            let progress = if total_items == 0 {
                0.0
            } else {
                completed_items as f32 / total_items as f32
            };
            self.handle_update_task(id, None, Some(progress), None, None, ctx)
                .await
        }

        async fn handle_update_task_status(
            &self,
            id: i64,
            status: TaskStatus,
            ctx: RPCContext,
        ) -> std::result::Result<(), RPCErrors> {
            self.handle_update_task(id, Some(status), None, None, None, ctx)
                .await
        }

        async fn handle_update_task_error(
            &self,
            id: i64,
            error_message: &str,
            ctx: RPCContext,
        ) -> std::result::Result<(), RPCErrors> {
            self.handle_update_task(
                id,
                Some(TaskStatus::Failed),
                None,
                Some(error_message.to_string()),
                None,
                ctx,
            )
            .await
        }

        async fn handle_update_task_data(
            &self,
            id: i64,
            data: serde_json::Value,
            _ctx: RPCContext,
        ) -> std::result::Result<(), RPCErrors> {
            let mut tasks = self.inner.tasks.lock().unwrap();
            let task = tasks
                .iter_mut()
                .find(|task| task.id == id)
                .ok_or_else(|| RPCErrors::ReasonError(format!("task {id} not found")))?;
            task.data = data;
            Ok(())
        }

        async fn handle_cancel_task(
            &self,
            id: i64,
            _recursive: bool,
            ctx: RPCContext,
        ) -> std::result::Result<(), RPCErrors> {
            self.handle_update_task_status(id, TaskStatus::Canceled, ctx)
                .await
        }

        async fn handle_delete_task(
            &self,
            id: i64,
            _ctx: RPCContext,
        ) -> std::result::Result<(), RPCErrors> {
            self.inner
                .tasks
                .lock()
                .unwrap()
                .retain(|task| task.id != id);
            Ok(())
        }
    }

    fn merge_json_test(dst: &mut serde_json::Value, patch: &serde_json::Value) {
        match (dst, patch) {
            (serde_json::Value::Object(dst), serde_json::Value::Object(patch)) => {
                for (key, value) in patch {
                    merge_json_test(
                        dst.entry(key.clone()).or_insert(serde_json::Value::Null),
                        value,
                    );
                }
            }
            (dst, patch) => *dst = patch.clone(),
        }
    }

    fn delegate_task(id: i64, data: serde_json::Value) -> Task {
        Task {
            id,
            user_id: "user".to_string(),
            app_id: "opendan".to_string(),
            session_id: String::new(),
            parent_id: None,
            root_id: id.to_string(),
            name: "delegate task".to_string(),
            task_type: TASK_TYPE_AGENT_DELEGATE.to_string(),
            runner: "agent".to_string(),
            status: TaskStatus::Pending,
            progress: 0.0,
            message: None,
            data,
            permissions: TaskPermissions::default(),
            created_at: 1,
            updated_at: 1,
        }
    }

    fn write_session_meta(agent: &AIAgent, mut meta: SessionMeta) {
        let dir = agent
            .config
            .layout
            .session_dir(&meta.session_id)
            .join(".meta");
        std::fs::create_dir_all(&dir).expect("mkdir meta");
        if meta.status_changed_at_ms == 0 {
            meta.status_changed_at_ms = 1;
        }
        std::fs::write(
            dir.join("session.json"),
            serde_json::to_vec_pretty(&meta).expect("encode meta"),
        )
        .expect("write meta");
    }

    #[tokio::test]
    async fn session_accepts_pending_rejects_ended_disk_meta() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agent = test_agent(dir.path().to_path_buf());
        let mut meta = SessionMeta::new(
            "work-ended".to_string(),
            SessionKind::Work,
            "plan".to_string(),
            "ui-1".to_string(),
        );
        meta.status = SessionStatus::Ended;
        write_session_meta(&agent, meta);

        assert!(!agent
            .session_accepts_pending("work-ended")
            .await
            .expect("status check"));
        assert!(agent
            .session_accepts_pending("new-session")
            .await
            .expect("missing meta is acceptable"));
    }

    #[tokio::test]
    async fn forward_message_ended_session_guides_new_worksession() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agent = test_agent(dir.path().to_path_buf());
        let mut meta = SessionMeta::new(
            "work-ended".to_string(),
            SessionKind::Work,
            "work_default".to_string(),
            "ui-1".to_string(),
        );
        meta.status = SessionStatus::Ended;
        meta.title = "Old task".to_string();
        meta.objective = "Finish the previous work".to_string();
        meta.workspace_id = Some("old-workspace".to_string());
        meta.bootstrap_done = true;
        let session = agent
            .clone()
            .ensure_session_inner(
                meta.session_id.clone(),
                meta.kind,
                meta.owner.clone(),
                Some(meta.current_behavior.clone()),
                Some(meta),
            )
            .await
            .expect("mount ended session");
        session.meta.lock().await.status = SessionStatus::Ended;

        let err = agent
            .forward_message("work-ended", "ui-1", "follow-up")
            .await
            .expect_err("ended session should reject forward");
        let msg = format!("{err:#}");
        assert!(msg.contains("target session `work-ended` has ended"));
        assert!(msg.contains("Do not retry `forward_msg` for this session"));
        assert!(msg.contains("try_create_worksession"));
        assert!(msg.contains("followup_routing.target_worksession_id"));
        assert!(msg.contains("title `Old task`"));
        assert!(msg.contains("workspace `old-workspace`"));
    }

    #[tokio::test]
    async fn restore_session_routes_does_not_mount_workers() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agent = test_agent(dir.path().to_path_buf());
        let mut meta = SessionMeta::new(
            "ui-did_dev_alice".to_string(),
            SessionKind::Ui,
            "chat_route".to_string(),
            "did:dev:alice".to_string(),
        );
        meta.status = SessionStatus::Idle;
        write_session_meta(&agent, meta);

        agent.clone().restore_session_routes().await;

        assert!(agent.get_session("ui-did_dev_alice").await.is_none());
        assert_eq!(
            agent
                .resolve_session_for_command("did:dev:alice")
                .await
                .unwrap(),
            "ui-did_dev_alice"
        );
    }

    #[tokio::test]
    async fn compress_command_mounts_restored_ui_session() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agent = test_agent(dir.path().to_path_buf());
        let mut meta = SessionMeta::new(
            "ui-did_dev_alice".to_string(),
            SessionKind::Ui,
            "chat_route".to_string(),
            "did:dev:alice".to_string(),
        );
        meta.status = SessionStatus::Idle;
        write_session_meta(&agent, meta);

        agent.clone().restore_session_routes().await;
        assert!(agent.get_session("ui-did_dev_alice").await.is_none());

        let outcome = crate::command_dispatcher::run_command(
            &agent,
            &crate::command_dispatcher::CommandInvocation {
                record_id: "local-command".to_string(),
                from: "did:dev:alice".to_string(),
                from_did: None,
                tunnel_did: None,
                command: "compress".to_string(),
                args: String::new(),
            },
        )
        .await
        .expect("run compress command");

        assert!(outcome
            .reply
            .contains("session `ui-did_dev_alice` has no saved context to compress"));
        assert!(!outcome.reply.contains("no active session"));
        let session = agent
            .get_session("ui-did_dev_alice")
            .await
            .expect("session mounted by command");
        session.abort_worker().await;
    }

    #[tokio::test]
    async fn ui_session_creation_does_not_create_or_bind_workspace() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agent = test_agent(dir.path().to_path_buf());

        let session_id = agent
            .clone()
            .create_ui_session_for_tunnel("did:dev:alice", None, None)
            .await
            .expect("create ui session");
        let session = agent.get_session(&session_id).await.expect("session");
        assert_eq!(session.summary().await.workspace_id, None);
        let meta = session.meta.lock().await;
        let clock_sub = meta
            .event_subscriptions
            .iter()
            .find(|sub| sub.pattern == UI_CLOCK_TIMER_EVENT_ID)
            .expect("default ui clock subscription");
        assert_eq!(
            clock_sub.mode,
            crate::session_model::EventSubscriptionMode::BackgroundOnly
        );
        drop(meta);
        assert!(agent
            .workspaces()
            .list()
            .await
            .expect("list workspaces")
            .is_empty());
        assert!(!agent.workspaces().workspace_dir(&session_id).exists());

        agent
            .delete_session_physical(&session_id)
            .await
            .expect("delete session");
    }

    #[tokio::test]
    async fn restore_session_routes_backfills_ui_clock_subscription() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agent = test_agent(dir.path().to_path_buf());
        let mut meta = SessionMeta::new(
            "ui-did_dev_alice".to_string(),
            SessionKind::Ui,
            "chat_route".to_string(),
            "did:dev:alice".to_string(),
        );
        meta.event_subscriptions.clear();
        write_session_meta(&agent, meta);

        agent.clone().restore_session_routes().await;

        let meta_path = agent
            .config
            .layout
            .session_dir("ui-did_dev_alice")
            .join(".meta")
            .join("session.json");
        let restored: SessionMeta =
            serde_json::from_slice(&std::fs::read(meta_path).expect("read restored session meta"))
                .expect("decode restored session meta");
        let clock_sub = restored
            .event_subscriptions
            .iter()
            .find(|sub| sub.pattern == UI_CLOCK_TIMER_EVENT_ID)
            .expect("default ui clock subscription");
        assert_eq!(
            clock_sub.mode,
            crate::session_model::EventSubscriptionMode::BackgroundOnly
        );
    }

    #[tokio::test]
    async fn work_session_still_binds_workspace() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agent = test_agent(dir.path().to_path_buf());
        let mut meta = SessionMeta::new(
            "work-1".to_string(),
            SessionKind::Work,
            "work_default".to_string(),
            "ui-1".to_string(),
        );
        meta.workspace_id = Some("ws-1".to_string());

        agent
            .clone()
            .ensure_session_inner(
                "work-1".to_string(),
                SessionKind::Work,
                "ui-1".to_string(),
                Some("work_default".to_string()),
                Some(meta),
            )
            .await
            .expect("create work session");
        let session = agent.get_session("work-1").await.expect("session");
        assert_eq!(
            session.summary().await.workspace_id.as_deref(),
            Some("ws-1")
        );
        let record = agent
            .workspaces()
            .load_record("ws-1")
            .await
            .expect("load workspace");
        assert_eq!(record.current_session.as_deref(), Some("work-1"));

        agent
            .delete_session_physical("work-1")
            .await
            .expect("delete session");
    }

    #[tokio::test]
    async fn work_session_final_report_does_not_enqueue_ui_pending_input() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agent = test_agent(dir.path().to_path_buf());
        let ui_session_id = agent
            .clone()
            .create_ui_session_for_tunnel("did:dev:alice", None, None)
            .await
            .expect("create ui session");
        let ui = agent.get_session(&ui_session_id).await.expect("ui session");
        ui.abort_worker().await;
        agent.sessions.lock().await.remove(&ui_session_id);
        assert!(agent.get_session(&ui_session_id).await.is_none());
        let mut meta = SessionMeta::new(
            "work-report".to_string(),
            SessionKind::Work,
            "work_default".to_string(),
            ui_session_id.clone(),
        );
        meta.title = "draft report".to_string();
        meta.objective = "finish the report".to_string();
        meta.workspace_id = Some("ws-report".to_string());
        agent
            .clone()
            .ensure_session_inner(
                "work-report".to_string(),
                SessionKind::Work,
                ui_session_id.clone(),
                Some("work_default".to_string()),
                Some(meta),
            )
            .await
            .expect("create work session");
        let work = agent
            .get_session("work-report")
            .await
            .expect("work session");
        work.abort_worker().await;
        let request = llm_context::request::LLMContextRequest {
            owner: llm_context::request::ContextOwnerRef::Agent {
                session_id: "work-report".to_string(),
            },
            trace: Some("trace-1".to_string()),
            objective: "finish the report".to_string(),
            behavior_name: "work_default".to_string(),
            input: vec![],
            model_policy: Default::default(),
            tool_policy: Default::default(),
            output: Default::default(),
            budget: Default::default(),
            human_policy: Default::default(),
            error_policy: Default::default(),
            forbid_next_behavior: false,
        };
        let mut state = llm_context::state::LLMContextState::from_request(&request, 1);
        state.last_report = Some("final answer".to_string());
        let snapshot = llm_context::state::LLMContextSnapshot { request, state };

        work.maybe_publish_worksession_report(
            &snapshot,
            WorksessionReportPhase::Final,
            Some("END"),
            "trace-1",
        )
        .await
        .expect("publish report");
        work.maybe_publish_worksession_report(
            &snapshot,
            WorksessionReportPhase::Final,
            Some("END"),
            "trace-1",
        )
        .await
        .expect("second publish remains a no-op without msg-center");

        let pending = ui.meta.lock().await.pending_inputs.clone();
        assert!(pending.is_empty());
        assert!(work.meta.lock().await.last_report_delivery.is_none());
        assert!(agent.get_session(&ui_session_id).await.is_none());
        let report_md = std::fs::read_to_string(work.session_dir.join("report.md"))
            .expect("report.md should be written even when outbound is unavailable");
        assert!(report_md.contains("final answer"));

        agent
            .delete_session_physical("work-report")
            .await
            .expect("delete work session");
        agent
            .delete_session_physical(&ui_session_id)
            .await
            .expect("delete ui session");
    }

    #[tokio::test]
    async fn create_work_session_without_workspace_id_uses_meaningful_workspace_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agent = test_agent(dir.path().to_path_buf());

        let outcome = agent
            .clone()
            .create_work_session(CreateWorkSessionParams {
                title: "Frontend Snake Game".to_string(),
                objective: "Build the first pure front-end version".to_string(),
                workspace_id: None,
                behavior: None,
                created_by_session_id: "ui-1".to_string(),
                reason_messages: Vec::new(),
                task_binding: None,
                task_id: None,
                auto_start: true,
                bind_task: true,
            })
            .await
            .expect("create work session");
        assert_eq!(outcome.workspace_id, "Frontend Snake Game");
        assert!(agent
            .workspaces()
            .workspace_dir("Frontend Snake Game")
            .exists());

        agent
            .delete_session_physical(&outcome.session_id)
            .await
            .expect("delete session");
    }

    #[tokio::test]
    async fn create_work_session_treats_empty_behavior_as_default() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agent = test_agent(dir.path().to_path_buf());

        let outcome = agent
            .clone()
            .create_work_session(CreateWorkSessionParams {
                title: "Frontend Snake Game".to_string(),
                objective: "Build the first pure front-end version".to_string(),
                workspace_id: None,
                behavior: Some("  ".to_string()),
                created_by_session_id: "ui-1".to_string(),
                reason_messages: Vec::new(),
                task_binding: None,
                task_id: None,
                auto_start: true,
                bind_task: true,
            })
            .await
            .expect("create work session");
        assert_eq!(outcome.behavior, "work_default");
        let session = agent
            .get_session(&outcome.session_id)
            .await
            .expect("session");
        assert_eq!(session.summary().await.current_behavior, "work_default");

        agent
            .delete_session_physical(&outcome.session_id)
            .await
            .expect("delete session");
    }

    #[tokio::test]
    async fn create_work_session_can_skip_auto_start() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agent = test_agent(dir.path().to_path_buf());

        let outcome = agent
            .clone()
            .create_work_session(CreateWorkSessionParams {
                title: "Deferred Work".to_string(),
                objective: "Create the session without running it".to_string(),
                workspace_id: None,
                behavior: None,
                created_by_session_id: "ui-1".to_string(),
                reason_messages: Vec::new(),
                task_binding: None,
                task_id: None,
                auto_start: false,
                bind_task: true,
            })
            .await
            .expect("create work session");
        assert!(!outcome.auto_started);
        let session = agent
            .get_session(&outcome.session_id)
            .await
            .expect("session");
        let meta = session.meta.lock().await;
        assert_eq!(meta.status, SessionStatus::Idle);
        assert!(!meta.bootstrap_done);
        drop(meta);

        agent
            .delete_session_physical(&outcome.session_id)
            .await
            .expect("delete session");
    }

    #[tokio::test]
    async fn create_work_session_by_task_id_uses_task_data_and_binds_task() {
        let dir = tempfile::tempdir().expect("tempdir");
        let task_mgr = MemoryTaskMgr::default();
        let task = task_mgr.insert(delegate_task(
            41,
            serde_json::json!({
                "agent_delegate": {
                    "title": "Task title",
                    "purpose": "Complete the task objective",
                    "execution": {
                        "workspace_id": "existing-workspace"
                    }
                }
            }),
        ));
        let agent = test_agent_with_task_mgr(dir.path().to_path_buf(), task_mgr.clone());

        let outcome = agent
            .clone()
            .create_work_session(CreateWorkSessionParams {
                title: String::new(),
                objective: String::new(),
                workspace_id: None,
                behavior: None,
                created_by_session_id: "ui-1".to_string(),
                reason_messages: Vec::new(),
                task_binding: None,
                task_id: Some(task.id),
                auto_start: false,
                bind_task: true,
            })
            .await
            .expect("create work session by task");

        assert_eq!(outcome.title, "Task title");
        assert_eq!(outcome.workspace_id, "existing-workspace");
        assert_eq!(outcome.task_id, Some(task.id));
        let session = agent
            .get_session(&outcome.session_id)
            .await
            .expect("session");
        let meta = session.meta.lock().await;
        assert_eq!(meta.objective, "Complete the task objective");
        assert_eq!(
            meta.task_binding.as_ref().map(|binding| binding.task_id),
            Some(task.id)
        );
        drop(meta);

        let updated = task_mgr.task(task.id);
        let updated_data = agent_delegate_task_data(&updated).expect("agent.delegate task data");
        assert_eq!(updated.status, TaskStatus::Pending);
        assert_eq!(
            updated_data
                .progress
                .as_ref()
                .and_then(|progress| progress.execution.as_ref())
                .and_then(|execution| execution.get("session_id"))
                .and_then(serde_json::Value::as_str),
            Some(outcome.session_id.as_str())
        );
        assert_eq!(
            updated_data
                .progress
                .as_ref()
                .and_then(|progress| progress.execution.as_ref())
                .and_then(|execution| execution.get("status"))
                .and_then(serde_json::Value::as_str),
            Some("idle")
        );
    }

    #[tokio::test]
    async fn create_work_session_without_task_id_creates_bound_task() {
        let dir = tempfile::tempdir().expect("tempdir");
        let task_mgr = MemoryTaskMgr::default();
        let agent = test_agent_with_task_mgr(dir.path().to_path_buf(), task_mgr.clone());

        let outcome = agent
            .clone()
            .create_work_session(CreateWorkSessionParams {
                title: "Generated task".to_string(),
                objective: "Do generated work".to_string(),
                workspace_id: None,
                behavior: None,
                created_by_session_id: "ui-1".to_string(),
                reason_messages: Vec::new(),
                task_binding: None,
                task_id: None,
                auto_start: true,
                bind_task: true,
            })
            .await
            .expect("create work session");

        let task_id = outcome.task_id.expect("task id");
        let task = task_mgr.task(task_id);
        assert_eq!(task.task_type, TASK_TYPE_AGENT_DELEGATE);
        assert_eq!(task.status, TaskStatus::Running);
        assert_eq!(task.session_id, outcome.session_id);
        let data = agent_delegate_task_data(&task).expect("agent.delegate task data");
        assert_eq!(data.request.purpose.as_deref(), Some("Do generated work"));
        assert_eq!(
            data.progress
                .as_ref()
                .and_then(|progress| progress.execution.as_ref())
                .and_then(|execution| execution.get("session_id"))
                .and_then(serde_json::Value::as_str),
            Some(outcome.session_id.as_str())
        );
    }

    #[tokio::test]
    async fn allocate_workspace_id_adds_suffix_on_conflict() {
        let dir = tempfile::tempdir().expect("tempdir");
        let agent = test_agent(dir.path().to_path_buf());
        agent
            .workspaces()
            .create_or_open("Frontend Snake Game", "Frontend Snake Game", None)
            .await
            .expect("create workspace");

        let workspace_id = agent.allocate_workspace_id("Frontend Snake Game").await;
        assert_eq!(workspace_id, "Frontend Snake Game 2");
    }
}
