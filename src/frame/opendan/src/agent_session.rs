//! §9.4 of NewOpenDANRuntime — Agent session.
//!
//! MVP scope: text-only inbox, single-turn `LLMContext::run` per arrival batch,
//! resume from a `state.snap` when present, simple Outcome handling, normal-mode
//! behavior switch. PendingTool / Fork / Independent / async-tool dispatch are
//! left as `warn!` + idle for now (§9.6 will round these out once contact_mgr /
//! task_mgr wiring lands).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use buckyos_api::{AiContent, AiMessage, AiRole};
use log::{info, warn};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;

use agent_tool::{AgentToolManager, SessionRuntimeContext};
use llm_context::{
    context_loop::LLMContext,
    observation::Observation,
    outcome::{ContextOutput, LLMContextOutcome, ResumeFill},
    request::{ContextOwnerRef, LLMContextRequest},
    state::{LLMContextSnapshot, LLMContextState},
};

use crate::agent_config::AgentConfig;
use crate::ai_runtime::{
    build_session_deps, AgentRuntime, OneLineStatusSink, SessionDepsInput,
};
use crate::behavior_cfg::{BehaviorCfg, SwitchMode};
use crate::llm_context_helper::{apply_overrides_to_snapshot, RequestOverrides};
use crate::session_event_pump::SessionEventPump;
use crate::task_dispatch::TaskDispatch;

/// Internal wake-up signal for the session worker. The worker consumes the
/// actual payload from `SessionMeta::pending_inputs` (which is persisted) —
/// this channel only nudges the worker to check.
#[derive(Debug, Clone)]
pub enum SessionInput {
    /// New item enqueued to `meta.pending_inputs` — worker should re-check.
    Wakeup,
    /// Cooperative cancel (used by `stop()`).
    Cancel,
}

/// How an interrupt should wind down outstanding tool calls. Chosen
/// per-call by the caller of [`AgentSession::interrupt`] — different
/// upper-layer control flows targeting the same session may legitimately
/// want different strategies, so this is not a per-behavior or per-agent
/// default.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InterruptMode {
    /// Inject `Observation::Cancelled` for every pending tool call and
    /// drive the existing LLMContext to a terminal outcome (with the
    /// resumed snapshot's `tool_policy.max_rounds` overridden to 0 so any
    /// further tool attempt is rejected as `BudgetExhausted(ToolRounds)`).
    /// Side effects already dispatched externally stay recorded in
    /// accumulated history — the next turn can reason about them.
    Graceful,
    /// Discard the trailing assistant turn that owns the unresolved
    /// `tool_use` blocks (and everything after it in accumulated), then
    /// continue from the truncated history. The interrupted side effects
    /// vanish from the LLM's view; externally-dispatched task_mgr tasks
    /// may still complete but their results never reach the LLM.
    Discard,
}

/// One inbound item parked on the session until the worker is ready to
/// consume it. Persisted as part of [`SessionMeta`] so that a crash between
/// `enqueue_pending` (which acks the system inbox) and the LLM actually
/// reading the input never loses a message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PendingInput {
    /// A user-facing chat message routed in from msg-center / a UI tunnel /
    /// the local CLI. `record_id` is the msg-center record id when the
    /// source was msg-center; locally-injected messages use a generated id.
    ///
    /// `from` is the raw host-name form used for keying the per-tunnel UI
    /// session. `from_did` (when present) carries the *full* DID string so
    /// the session can address replies back to the original peer through
    /// `msg_center.post_send`. `tunnel_did` is the tunnel route hint pulled
    /// from `MsgRecord.route.tunnel_did` — used as `preferred_tunnel` on the
    /// outbound side so the reply rides the same wire whenever possible.
    Msg {
        record_id: String,
        from: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from_did: Option<String>,
        /// Human-readable display name for the sender, when known. Filled
        /// either by msg-center (existing `record.from_name`) or by the
        /// inbound pump via the contact-mgr lookup. Empty / unknown when
        /// the contact isn't registered.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tunnel_did: Option<String>,
        text: String,
    },
    /// A kevent / system event the session has subscribed to. `event_id` is
    /// the kevent eventid; `data` is the opaque payload.
    Event {
        event_id: String,
        data: serde_json::Value,
    },
    /// A session-layer interrupt request. Enqueued by upper-layer control
    /// flows that want to cut into the worker's stream — e.g. "stop the
    /// long tool, the user just said something more important". `id` is
    /// the enqueue-time tag that makes the dedup key unique (so a re-fired
    /// interrupt does not silently coalesce with a stale one).
    Interrupt { mode: InterruptMode, id: String },
}

impl PendingInput {
    /// Stable dedup key. Two `PendingInput`s with the same key are treated
    /// as the same logical item — the second `enqueue_pending` becomes a
    /// no-op so a msg-center lease replay can't double-feed the LLM.
    pub fn dedup_key(&self) -> String {
        match self {
            PendingInput::Msg { record_id, .. } => format!("msg:{record_id}"),
            PendingInput::Event { event_id, .. } => format!("event:{event_id}"),
            PendingInput::Interrupt { id, .. } => format!("interrupt:{id}"),
        }
    }
}

/// Sentinel emitted by a behavior parser in
/// `LLMBehaviorResult.next_behavior` to mean "current intent ran its course,
/// no autonomous next step — park the session until the next inbound user
/// message". Interpreted only at the session layer; the waist treats it as
/// an opaque jump-target string.
pub const NEXT_BEHAVIOR_WAIT_USER_MSG: &str = "WAIT_USER_MSG";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    Ui,
    Work,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Idle,
    Running,
    WaitingInput,
    WaitingTool,
    Ended,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub session_id: String,
    pub kind: SessionKind,
    pub current_behavior: String,
    pub status: SessionStatus,
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub one_line_status: String,
    /// Inputs that have been received from the system but not yet handed to
    /// the LLM. Persisted so a process crash doesn't lose buffered inputs.
    /// See [`AgentSession::enqueue_pending`] / the worker loop in
    /// `run_worker`.
    #[serde(default)]
    pub pending_inputs: Vec<PendingInput>,
    /// Full DID of the most-recently-seen human peer for this session.
    /// Captured from `PendingInput::Msg.from_did` when present. Used as the
    /// reply target when an outcome produces assistant text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_did: Option<String>,
    /// Preferred tunnel DID for outbound replies, captured from
    /// `PendingInput::Msg.tunnel_did`. Optional — `None` lets msg-center
    /// pick a route via the contact manager.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_tunnel_did: Option<String>,
    /// Persisted kevent subscriptions for this session. The agent's event
    /// pump aggregates these into its reader and routes matched events back
    /// as `Inbound::Event { target_session_id: Some(<this id>), ... }`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event_subscriptions: Vec<EventSubscription>,
    /// Workspace this session is bound to. The session is the source of
    /// truth — the workspace record's `current_session` is just a hint /
    /// conflict-detection field. `None` ⇒ session not yet bound to any
    /// workspace (legacy MVP flow auto-binds on session create).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    /// Async-tool dispatches the LLM is waiting on. Populated by
    /// `handle_outcome::PendingTool` after each entry is registered with
    /// `task_mgr` and a `/task_mgr/<task_id>` subscription is in place.
    /// Persisted so a restart can pick up where it left off — the new
    /// worker re-subscribes via `event_subscriptions` and resumes when the
    /// completion events arrive.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_task_calls: Vec<PendingTaskCall>,
    /// Short human-friendly label for this session. Empty for the
    /// legacy auto-created UI sessions; populated by
    /// `create_worksession` so worksession-list UIs can show meaningful
    /// names.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// Goal / task statement. Empty for UI sessions (their job is to
    /// chat with the human); set by `create_worksession` so the work
    /// session's first inference has something to drive it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub objective: String,
    /// Has this session run at least one inference? Used so a freshly
    /// created Work session auto-kicks (objective drives the first turn,
    /// no external message needed) while subsequent restarts wait on
    /// real inputs. Always `false` on the first persist; flipped to
    /// `true` after the first turn returns from `run_one_turn`.
    #[serde(default)]
    pub bootstrap_done: bool,
    /// Entry behavior name of the **currently active independent process**.
    /// Default = `current_behavior` (top-level process whose entry is the
    /// initial behavior). Diverges from `current_behavior` only when the
    /// active process has done an intra-process normal switch. Drives the
    /// `.meta/behavior_<process_entry>.snap` filename when the active
    /// process is paused (parent gets pushed) or resumed (popped from stack).
    #[serde(default)]
    pub process_entry: String,
    /// Independent-mode call stack (excludes the active top frame).
    /// Pushed when this session switches into an `Independent` behavior;
    /// popped when that child reaches `END` while parent frames are waiting.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub process_stack: Vec<ProcessFrame>,
}

/// One async tool dispatch tracked at the session level. Bridges the LLM's
/// `call_id` to the `task_mgr` task that carries the real work, plus the
/// kevent pattern we listen on for completion. The session uses this map
/// to recognize incoming completion events and assemble a
/// `ResumeFill::ToolResults` once every pending call has a result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingTaskCall {
    /// LLM-side identifier from the `AiToolCall`. Must match what the
    /// snapshot's `pending_tool_calls` carries — resume validates this.
    pub call_id: String,
    /// Tool name (e.g. `download`) — informational, used for logging /
    /// fallback observation building.
    pub tool_name: String,
    /// task_mgr task id returned by `dispatch_async_tool`.
    pub task_id: i64,
    /// kevent pattern we subscribed to for this task. Stored so we can
    /// unsubscribe once the completion arrives.
    pub event_pattern: String,
}

/// Snapshot view of one session's externally-relevant fields. Returned by
/// [`AgentSession::summary`] and aggregated by [`crate::agent::AIAgent::list_session_summaries`]
/// for prompts that need to surface the agent's session inventory (the fork
/// sub-context inside `try_create_worksession` is the canonical consumer).
///
/// All fields are owned strings / copies — the struct is safe to pass through
/// JSON or hand to a long-running prompt rendering pipeline.
#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub session_id: String,
    pub kind: SessionKind,
    /// Short label; empty for legacy auto-created UI sessions.
    pub title: String,
    /// Task statement; empty for UI sessions.
    pub objective: String,
    pub status: SessionStatus,
    /// Last-activity one-liner that the worker writes through the
    /// `OneLineStatusSink`. Empty when the session has been idle since
    /// boot.
    pub one_line_status: String,
    pub workspace_id: Option<String>,
    pub current_behavior: String,
}

/// One frame on the **independent-mode process call stack**. Each frame
/// records the parent process's entry behavior name (drives the
/// `.meta/behavior_<entry>.snap` filename) plus the parent's
/// `current_behavior` at push time — so an intra-process normal switch is
/// faithfully restored when the child ends and we pop back.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessFrame {
    pub entry: String,
    pub current: String,
}

/// One persisted kevent subscription belonging to a session.
///
/// Subscriptions live in `SessionMeta` so that a restart re-establishes the
/// session's view of the event bus before the worker starts. The pump
/// aggregates subscriptions across all live sessions and rebuilds its
/// `EventReader` whenever the union changes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EventSubscription {
    /// kevent pattern — passed through to `KEventClient::create_event_reader`.
    pub pattern: String,
    /// Wall-clock timestamp the subscription was added; informational only,
    /// not used for ordering or matching.
    #[serde(default)]
    pub subscribed_at_ms: u64,
}

impl SessionMeta {
    pub fn new(
        session_id: String,
        kind: SessionKind,
        current_behavior: String,
        owner: String,
    ) -> Self {
        Self {
            session_id,
            kind,
            current_behavior: current_behavior.clone(),
            status: SessionStatus::Idle,
            owner,
            one_line_status: String::new(),
            pending_inputs: Vec::new(),
            peer_did: None,
            peer_tunnel_did: None,
            event_subscriptions: Vec::new(),
            workspace_id: None,
            pending_task_calls: Vec::new(),
            title: String::new(),
            objective: String::new(),
            bootstrap_done: false,
            // Top-level process entry defaults to the initial behavior.
            process_entry: current_behavior,
            process_stack: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum SessionReply {
    AssistantText { text: String },
    Error { message: String },
    Ended,
}

pub struct InMemoryStatus {
    current: std::sync::Mutex<String>,
}

impl InMemoryStatus {
    pub fn new() -> Self {
        Self {
            current: std::sync::Mutex::new(String::new()),
        }
    }

    pub fn snapshot(&self) -> String {
        self.current.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

impl OneLineStatusSink for InMemoryStatus {
    fn set(&self, status: String) {
        if let Ok(mut g) = self.current.lock() {
            *g = status;
        }
    }
}

#[derive(Clone)]
pub struct AgentSession {
    pub session_id: String,
    pub agent_name: String,
    pub kind: SessionKind,
    pub owner: String,

    pub runtime: Arc<AgentRuntime>,
    pub agent_config: Arc<AgentConfig>,
    pub tools: Arc<AgentToolManager>,

    pub inbox_tx: mpsc::Sender<SessionInput>,
    pub reply_tx: mpsc::Sender<SessionReply>,

    pub session_dir: PathBuf,
    pub state_snap_path: PathBuf,

    handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    pub meta: Arc<Mutex<SessionMeta>>,
    pub status: Arc<InMemoryStatus>,
    /// Per-agent kevent pump handle. `None` for CLI / test runs without a
    /// kevent client; otherwise the session pushes its current pattern
    /// list here whenever `subscribe_event` / `unsubscribe_event` mutates
    /// `event_subscriptions`, so the agent-wide reader rebuilds promptly.
    event_pump: Option<Arc<SessionEventPump>>,

    trace_seq: Arc<std::sync::atomic::AtomicU64>,

    /// In-memory **fork call stack** for diagnostics. Each frame = the
    /// parent's trace id at the moment of fork. Per design fork is a
    /// non-resumable sync sub-task, so this stack is not persisted —
    /// a crash mid-fork drops the sub-context, the parent recovers from
    /// its on-disk snapshot, and the fork is simply lost (acceptable
    /// per the design doc §Session-level 状态结构).
    fork_stack: Arc<Mutex<Vec<String>>>,

    /// Last user-text that triggered the current (or most recent) inference
    /// turn. Stashed by the worker right before `run_one_turn` so
    /// session-aware tools can pick it up without having to be told —
    /// `forward_msg` reads this to default its body to "the message that
    /// caused the parent LLM to think a forward was needed". §8.4 of the
    /// design doc calls this the "本轮 origin user 消息". Per-turn ephemeral
    /// state — not persisted, simply overwritten each turn.
    current_origin_msg: Arc<std::sync::Mutex<Option<String>>>,
}

pub struct AgentSessionBuild {
    pub session_id: String,
    pub agent_name: String,
    pub kind: SessionKind,
    pub owner: String,
    pub current_behavior: String,
    pub runtime: Arc<AgentRuntime>,
    pub agent_config: Arc<AgentConfig>,
    pub tools: Arc<AgentToolManager>,
    pub reply_tx: mpsc::Sender<SessionReply>,
    /// Existing on-disk meta to seed the session with. Used by
    /// `AIAgent::restore_active_sessions` so pending_inputs / peer info /
    /// event_subscriptions persisted before the last crash survive into
    /// the new in-memory session.
    pub existing_meta: Option<SessionMeta>,
    /// Optional event pump handle — when present, the session updates its
    /// subscription patterns directly through the pump so additions take
    /// effect without going through the AIAgent layer first.
    pub event_pump: Option<Arc<SessionEventPump>>,
}

impl AgentSession {
    pub fn new(b: AgentSessionBuild) -> (Self, mpsc::Receiver<SessionInput>) {
        let session_dir = b.agent_config.layout.session_dir(&b.session_id);
        let state_snap_path = session_dir.join(".meta").join("state.snap");
        let (inbox_tx, inbox_rx) = mpsc::channel(64);

        // Restore path: keep persistent fields (pending_inputs, peer info,
        // event_subscriptions) but reset transient status to Idle so the
        // worker re-enters the main loop cleanly.
        let meta = if let Some(mut existing) = b.existing_meta {
            existing.session_id = b.session_id.clone();
            existing.kind = b.kind;
            existing.current_behavior = b.current_behavior.clone();
            existing.owner = b.owner.clone();
            existing.status = SessionStatus::Idle;
            existing.one_line_status.clear();
            // Backfill: older session.json files predate `process_entry`. An
            // empty value here means "top-level process whose entry == the
            // current behavior" — restore that interpretation so the
            // independent-mode persistence path doesn't reject the session.
            if existing.process_entry.is_empty() {
                existing.process_entry = existing.current_behavior.clone();
            }
            existing
        } else {
            SessionMeta::new(
                b.session_id.clone(),
                b.kind,
                b.current_behavior.clone(),
                b.owner.clone(),
            )
        };
        let session = Self {
            session_id: b.session_id,
            agent_name: b.agent_name,
            kind: b.kind,
            owner: b.owner,
            runtime: b.runtime,
            agent_config: b.agent_config,
            tools: b.tools,
            inbox_tx,
            reply_tx: b.reply_tx,
            session_dir,
            state_snap_path,
            handle: Arc::new(Mutex::new(None)),
            meta: Arc::new(Mutex::new(meta)),
            status: Arc::new(InMemoryStatus::new()),
            event_pump: b.event_pump,
            trace_seq: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            fork_stack: Arc::new(Mutex::new(Vec::new())),
            current_origin_msg: Arc::new(std::sync::Mutex::new(None)),
        };
        (session, inbox_rx)
    }

    /// Persist the current `SessionMeta` to `.meta/session.json`. Returns
    /// `Ok(())` only after the write has hit disk (so callers like
    /// `enqueue_pending` can ack upstream once this returns).
    pub async fn flush_meta(&self) -> Result<()> {
        let dir = self.session_dir.join(".meta");
        tokio::fs::create_dir_all(&dir).await.map_err(|err| {
            anyhow!(
                "session[{}]: mkdir {} failed: {err}",
                self.session_id,
                dir.display()
            )
        })?;
        let meta = self.meta.lock().await.clone();
        let bytes = serde_json::to_vec_pretty(&meta).map_err(|err| {
            anyhow!("session[{}]: serialize meta failed: {err}", self.session_id)
        })?;
        let path = dir.join("session.json");
        let tmp = path.with_extension("json.tmp");
        // tmp + rename for crash-consistency: a half-written session.json
        // would prevent `restore_active_sessions` from booting this session.
        tokio::fs::write(&tmp, &bytes).await.map_err(|err| {
            anyhow!(
                "session[{}]: write {} failed: {err}",
                self.session_id,
                tmp.display()
            )
        })?;
        tokio::fs::rename(&tmp, &path).await.map_err(|err| {
            anyhow!(
                "session[{}]: rename to {} failed: {err}",
                self.session_id,
                path.display()
            )
        })?;
        Ok(())
    }

    /// Append `input` to the persistent pending queue. Returns once the
    /// queue has been flushed to disk — the caller (e.g. msg-center pump,
    /// CLI inject) can ack upstream the moment this returns, because the
    /// item is now durably owned by the session and will be replayed across
    /// restarts.
    ///
    /// Duplicates (same `dedup_key`) are silently dropped — the upstream
    /// system may legitimately replay an entry (msg-center lease timeout,
    /// kevent retry), and we don't want to feed the LLM the same input
    /// twice. Callers should treat `Ok(())` as "you may now ack regardless
    /// of whether the item was newly accepted or deduplicated".
    pub async fn enqueue_pending(&self, input: PendingInput) -> Result<()> {
        let key = input.dedup_key();
        let mut changed = false;
        {
            let mut meta = self.meta.lock().await;
            let already = meta
                .pending_inputs
                .iter()
                .any(|i| i.dedup_key() == key);
            if !already {
                meta.pending_inputs.push(input);
                changed = true;
            }
        }
        if changed {
            self.flush_meta().await?;
            // Wake the worker. send-failure means the receiver is gone
            // (worker exiting); the input is still durable on disk, so the
            // next boot will pick it up. No error path needed.
            let _ = self.inbox_tx.send(SessionInput::Wakeup).await;
        }
        Ok(())
    }

    /// Enqueue an interrupt barrier. The worker drains its queue strictly
    /// in order: items enqueued *before* this call are processed first
    /// (within the same logical turn), then the interrupt fires, then any
    /// items enqueued *after* this call run in a fresh turn. Upper-layer
    /// flows that want "stop, then send this message" should call
    /// `interrupt` and then `enqueue_pending(Msg)` in that order.
    ///
    /// The interrupt is a no-op when the session has no outstanding
    /// pending tool calls at the moment the worker processes it (the
    /// session is already at an outcome boundary; there is nothing to
    /// wind down). It is safe to call regardless of session state — the
    /// worker enforces the precondition.
    pub async fn interrupt(&self, mode: InterruptMode) -> Result<()> {
        let id = format!(
            "{}-{}",
            now_ms(),
            self.trace_seq
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        );
        self.enqueue_pending(PendingInput::Interrupt { mode, id }).await
    }

    pub async fn start(self: Arc<Self>, mut inbox_rx: mpsc::Receiver<SessionInput>) {
        let me = self.clone();
        let handle = tokio::spawn(async move {
            me.run_worker(&mut inbox_rx).await;
        });
        *self.handle.lock().await = Some(handle);
    }

    /// Send a no-op wake signal so the worker re-checks `pending_inputs`
    /// + the bootstrap-turn predicate. Used by `create_work_session` after
    /// seeding a fresh session, so it runs its first inference without
    /// waiting for an external message.
    pub async fn wake(&self) {
        let _ = self.inbox_tx.send(SessionInput::Wakeup).await;
    }

    pub async fn stop(&self) {
        let _ = self.inbox_tx.send(SessionInput::Cancel).await;
        let handle = self.handle.lock().await.take();
        if let Some(h) = handle {
            let _ = h.await;
        }
    }

    /// Convenience: enqueue a locally-injected human message. The synthetic
    /// `record_id` distinguishes CLI / test injections from msg-center
    /// records (which use the upstream record id).
    pub async fn submit_text(&self, text: String) -> Result<()> {
        let record_id = format!(
            "local-{}-{}",
            self.session_id,
            self.trace_seq
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        self.enqueue_pending(PendingInput::Msg {
            record_id,
            from: self.owner.clone(),
            from_did: None,
            from_name: None,
            tunnel_did: None,
            text,
        })
        .await
    }

    async fn run_worker(self: Arc<Self>, inbox_rx: &mut mpsc::Receiver<SessionInput>) {
        info!(
            "opendan.session[{}]: worker started (kind={:?})",
            self.session_id, self.kind
        );

        // First boot might have pending_inputs from a previous run that
        // never got consumed — process those before waiting for new wakeups.
        loop {
            // Drain non-Wakeup control signals first so a Cancel doesn't get
            // stalled behind a turn.
            while let Ok(signal) = inbox_rx.try_recv() {
                if matches!(signal, SessionInput::Cancel) {
                    self.set_status(SessionStatus::Idle).await;
                    if matches!(self.kind, SessionKind::Work) {
                        info!(
                            "opendan.session[{}]: cancel received on work session, exiting worker",
                            self.session_id
                        );
                        return;
                    }
                }
            }

            // Snapshot current pending queue. We DON'T remove items from
            // `meta.pending_inputs` here — that happens only after the turn
            // succeeds (handle_turn_result), so a crash mid-turn leaves the
            // inputs durable and they'll be replayed next boot.
            let mut pending = self.meta.lock().await.pending_inputs.clone();
            if pending.is_empty() {
                // Work session bootstrap: if a freshly-created Work session
                // has nothing pending and hasn't run yet, drive an initial
                // turn from its `objective` (per §8.1 step 4 of the design).
                // After the first successful turn this branch falls through
                // to the normal recv()-blocking path.
                let needs_bootstrap = matches!(self.kind, SessionKind::Work)
                    && self.needs_bootstrap_turn().await;
                if needs_bootstrap {
                    self.set_status(SessionStatus::Running).await;
                    let turn_result = self.run_one_turn(Vec::new()).await;
                    self.mark_bootstrap_done().await;
                    match turn_result {
                        Ok(action) => match action {
                            NextAction::Idle => self.set_status(SessionStatus::Idle).await,
                            NextAction::WaitForMsg => {
                                self.set_status(SessionStatus::WaitingInput).await
                            }
                            NextAction::WaitForTool => {
                                self.set_status(SessionStatus::WaitingTool).await
                            }
                            NextAction::End => {
                                self.set_status(SessionStatus::Ended).await;
                                let _ = self.reply_tx.send(SessionReply::Ended).await;
                                return;
                            }
                        },
                        Err(err) => {
                            warn!(
                                "opendan.session[{}]: bootstrap turn failed: {err:#}",
                                self.session_id
                            );
                            self.set_status(SessionStatus::Error).await;
                            let _ = self
                                .reply_tx
                                .send(SessionReply::Error {
                                    message: format!("{err:#}"),
                                })
                                .await;
                        }
                    }
                    continue;
                }
                match inbox_rx.recv().await {
                    None => {
                        info!(
                            "opendan.session[{}]: inbox closed, exiting worker",
                            self.session_id
                        );
                        return;
                    }
                    Some(SessionInput::Cancel) => {
                        self.set_status(SessionStatus::Idle).await;
                        if matches!(self.kind, SessionKind::Work) {
                            return;
                        }
                        continue;
                    }
                    Some(SessionInput::Wakeup) => continue,
                }
            }

            // Interrupt barrier handling. Interrupts split the queue:
            // anything queued *before* an Interrupt belongs to a prior
            // logical turn and is processed first; the Interrupt itself
            // fires on the next loop iteration; anything *after* it runs
            // as a fresh post-interrupt turn.
            //
            // The one exception (`pending_tools_active` below) is that a
            // later-queued Interrupt is fast-forwarded ahead of FIFO order
            // when the prefix cannot make progress on its own — without
            // that, `[Msg, Interrupt, ...]` while a tool round is still
            // in flight would deadlock (Msg can't run because tools are
            // pending; Interrupt can't run because Msg is ahead).
            let interrupt_pos = pending
                .iter()
                .position(|p| matches!(p, PendingInput::Interrupt { .. }));
            let pending_tools_active = self.snapshot_has_pending_tool_calls().await;
            if let Some(pos) = interrupt_pos {
                let head = pos == 0 || pending_tools_active;
                if head {
                    let (mode, key) = match &pending[pos] {
                        PendingInput::Interrupt { mode, .. } => {
                            (*mode, pending[pos].dedup_key())
                        }
                        _ => unreachable!("position matched Interrupt"),
                    };
                    if pos != 0 {
                        info!(
                            "opendan.session[{}]: fast-forwarding interrupt({mode:?}) ahead of {pos} pre-queued item(s) — pending tools blocked the prefix",
                            self.session_id
                        );
                    }
                    self.set_status(SessionStatus::Running).await;
                    if let Err(err) = self.execute_interrupt(mode).await {
                        warn!(
                            "opendan.session[{}]: interrupt({mode:?}) failed: {err:#}",
                            self.session_id
                        );
                        self.set_status(SessionStatus::Error).await;
                        let _ = self
                            .reply_tx
                            .send(SessionReply::Error {
                                message: format!("interrupt failed: {err:#}"),
                            })
                            .await;
                    }
                    // Consume the interrupt entry unconditionally — a
                    // failed execute_interrupt is logged + surfaced, but
                    // we don't want the bad entry pinning the queue.
                    self.discard_consumed(&[key]).await;
                    continue;
                }
                // Interrupt later in the queue AND prefix can still make
                // progress (no pending tools blocking it). Process the
                // prefix only this iteration; the Interrupt and anything
                // after it remain in `meta.pending_inputs` and surface on
                // the next loop.
                pending.truncate(pos);
            }

            // Three buckets:
            //   - Msg / generic Event → fold into the next turn as `turn_inputs`
            //   - Event whose id matches a `pending_task_calls` pattern →
            //     translates into an `Observation`, used to build a
            //     `ResumeFill::ToolResults` once every pending call has a
            //     result.
            // Latest peer info wins — the most recent Msg in this batch
            // dictates where outbound replies will be routed.
            let mut human_texts = Vec::new();
            let mut event_summaries = Vec::new();
            let mut consumed_keys = Vec::new();
            let mut task_completions: Vec<(String, Observation, String, String)> = Vec::new();
            let mut latest_peer_did: Option<String> = None;
            let mut latest_peer_tunnel: Option<String> = None;
            let pending_task_index = self.pending_task_index().await;
            for input in &pending {
                match input {
                    PendingInput::Msg {
                        text,
                        from_did,
                        tunnel_did,
                        ..
                    } => {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() {
                            human_texts.push(trimmed.to_string());
                        }
                        if let Some(did) = from_did.as_ref().filter(|s| !s.trim().is_empty()) {
                            latest_peer_did = Some(did.clone());
                        }
                        if let Some(t) = tunnel_did.as_ref().filter(|s| !s.trim().is_empty()) {
                            latest_peer_tunnel = Some(t.clone());
                        }
                        consumed_keys.push(input.dedup_key());
                    }
                    PendingInput::Event { event_id, data } => {
                        if let Some(entry) = pending_task_index.get(event_id) {
                            let obs = observation_from_task_event(&entry.call_id, data);
                            // Only consume task-completion events when they
                            // actually carry a terminal status; running /
                            // progress emissions are ignored so the pump
                            // doesn't keep waking us mid-task.
                            if let Some(obs) = obs {
                                task_completions.push((
                                    entry.call_id.clone(),
                                    obs,
                                    entry.event_pattern.clone(),
                                    input.dedup_key(),
                                ));
                            }
                            continue;
                        }
                        // Orphan task event — fired after we stopped tracking
                        // this call_id (interrupt cancelled it, or the
                        // upstream unsubscribe raced with an in-flight
                        // emission). Dropping silently is correct: feeding
                        // "task X completed" into the next turn after the
                        // session was already told "X cancelled" produces
                        // conflicting signals for the LLM.
                        if event_id.starts_with("/task_mgr/") {
                            consumed_keys.push(input.dedup_key());
                            continue;
                        }
                        // §9.6 event dispatch: surface non-task events into
                        // the turn so the LLM can react.
                        event_summaries.push(format_event_for_turn(event_id, data));
                        consumed_keys.push(input.dedup_key());
                    }
                    PendingInput::Interrupt { .. } => {
                        // The partition step above truncates the queue at
                        // the first Interrupt; any remaining one in this
                        // loop would be a programming error.
                        unreachable!("Interrupt should be filtered before drain")
                    }
                }
            }

            if latest_peer_did.is_some() || latest_peer_tunnel.is_some() {
                self.update_peer(latest_peer_did, latest_peer_tunnel).await;
            }

            // Tool completions take priority — if all pending_task_calls are
            // accounted for, resume the LLMContext via ResumeFill::ToolResults
            // and skip the human-text turn (the LLM is mid-run, not at a
            // free chat boundary).
            if !task_completions.is_empty() {
                let consumed_event_keys: Vec<String> = task_completions
                    .iter()
                    .map(|(_, _, _, k)| k.clone())
                    .collect();
                if self.all_pending_tasks_collected(&task_completions).await {
                    self.set_status(SessionStatus::Running).await;
                    let resume_result = self
                        .resume_with_tool_results(&task_completions)
                        .await;
                    match resume_result {
                        Ok(action) => {
                            // Only consume the task-completion events here.
                            // Any Msg / non-task Event also queued in this
                            // drain pass stays in `meta.pending_inputs`:
                            // resume_with_tool_results only feeds the tool
                            // results to the LLM, not those messages —
                            // dropping them would silently lose the input.
                            // They'll be picked up by the next worker loop,
                            // by which point `pending_tool_calls` is clear
                            // and `run_one_turn` handles them normally.
                            self.discard_consumed(&consumed_event_keys).await;
                            match action {
                                NextAction::Idle => self.set_status(SessionStatus::Idle).await,
                                NextAction::WaitForMsg => {
                                    self.set_status(SessionStatus::WaitingInput).await
                                }
                                NextAction::WaitForTool => {
                                    self.set_status(SessionStatus::WaitingTool).await
                                }
                                NextAction::End => {
                                    self.set_status(SessionStatus::Ended).await;
                                    let _ = self.reply_tx.send(SessionReply::Ended).await;
                                    return;
                                }
                            }
                            continue;
                        }
                        Err(err) => {
                            warn!(
                                "opendan.session[{}]: resume with tool results failed: {err:#}",
                                self.session_id
                            );
                            // Leave pending in place; surface error and wait.
                            self.set_status(SessionStatus::Error).await;
                            let _ = self
                                .reply_tx
                                .send(SessionReply::Error {
                                    message: format!("{err:#}"),
                                })
                                .await;
                            match inbox_rx.recv().await {
                                None => return,
                                Some(SessionInput::Cancel) => {
                                    self.set_status(SessionStatus::Idle).await;
                                    if matches!(self.kind, SessionKind::Work) {
                                        return;
                                    }
                                }
                                Some(SessionInput::Wakeup) => {}
                            }
                            continue;
                        }
                    }
                } else {
                    // Some calls still outstanding — keep all pending tool
                    // events on disk and wait for the rest. Recv via the
                    // sweeping wrapper so a lost kevent doesn't park us
                    // forever (task_mgr is polled on a timed tick and any
                    // terminal status is synthesized into the queue).
                    self.set_status(SessionStatus::WaitingTool).await;
                    match self.wait_with_tool_sweep(inbox_rx).await {
                        None => return,
                        Some(SessionInput::Cancel) => {
                            self.set_status(SessionStatus::Idle).await;
                            if matches!(self.kind, SessionKind::Work) {
                                return;
                            }
                        }
                        Some(SessionInput::Wakeup) => {}
                    }
                    continue;
                }
            }

            // Events are folded into the same turn as message text — they
            // arrive interleaved chronologically and represent the same
            // "what's new since last inference" surface to the LLM.
            let mut turn_inputs = human_texts;
            turn_inputs.extend(event_summaries);

            // If the snapshot is currently mid-PendingTool and the upper
            // layer queued bare Msg/Event entries without an Interrupt
            // barrier, defer: starting a fresh turn here would discard
            // the in-flight tool round. Upper layers that want immediate
            // attention should `interrupt()` first, then `enqueue_pending`.
            if !turn_inputs.is_empty() && self.snapshot_has_pending_tool_calls().await {
                self.set_status(SessionStatus::WaitingTool).await;
                match self.wait_with_tool_sweep(inbox_rx).await {
                    None => return,
                    Some(SessionInput::Cancel) => {
                        self.set_status(SessionStatus::Idle).await;
                        if matches!(self.kind, SessionKind::Work) {
                            return;
                        }
                    }
                    Some(SessionInput::Wakeup) => {}
                }
                continue;
            }

            if turn_inputs.is_empty() {
                self.discard_consumed(&consumed_keys).await;
                continue;
            }

            // Stash the most recent human-text as the turn's "origin user
            // message" so session-aware tools (forward_msg) can pick it up
            // without the LLM having to pass it through tool args (§8.4).
            // Events have no origin-user semantics — they only update the
            // stash when they happen to come bundled with chat text.
            let origin_msg = turn_inputs
                .iter()
                .rev()
                .find(|s| {
                    let t = s.trim();
                    !t.is_empty() && !t.starts_with("[environment event]")
                })
                .cloned();
            self.set_current_origin_msg(origin_msg);

            self.set_status(SessionStatus::Running).await;
            let turn_result = self.run_one_turn(turn_inputs).await;
            match turn_result {
                Ok(action) => {
                    // Successful turn ⇒ remove the items we just fed to the
                    // LLM from the persistent queue.
                    self.discard_consumed(&consumed_keys).await;
                    match action {
                        NextAction::Idle => self.set_status(SessionStatus::Idle).await,
                        NextAction::WaitForMsg => {
                            self.set_status(SessionStatus::WaitingInput).await
                        }
                        NextAction::WaitForTool => {
                            self.set_status(SessionStatus::WaitingTool).await
                        }
                        NextAction::End => {
                            self.set_status(SessionStatus::Ended).await;
                            let _ = self.reply_tx.send(SessionReply::Ended).await;
                            return;
                        }
                    }
                }
                Err(err) => {
                    // Turn failed — leave consumed_keys in `pending_inputs`
                    // so a restart / manual retry replays them. The session
                    // moves to Error so the supervisor can intervene.
                    warn!(
                        "opendan.session[{}]: turn failed (pending kept for retry): {err:#}",
                        self.session_id
                    );
                    self.set_status(SessionStatus::Error).await;
                    let _ = self
                        .reply_tx
                        .send(SessionReply::Error {
                            message: format!("{err:#}"),
                        })
                        .await;
                    // Wait for an external signal (Cancel / new Wakeup) before
                    // retrying — otherwise we'd hot-loop on the same bad
                    // input.
                    match inbox_rx.recv().await {
                        None => return,
                        Some(SessionInput::Cancel) => {
                            self.set_status(SessionStatus::Idle).await;
                            if matches!(self.kind, SessionKind::Work) {
                                return;
                            }
                        }
                        Some(SessionInput::Wakeup) => {}
                    }
                }
            }
        }
    }

    /// Remove items whose `dedup_key` is in `keys` from the persistent queue
    /// and flush. Called after a turn succeeds — the LLM has now "seen"
    /// those inputs, so they're safe to drop.
    async fn discard_consumed(&self, keys: &[String]) {
        if keys.is_empty() {
            return;
        }
        {
            let mut meta = self.meta.lock().await;
            meta.pending_inputs
                .retain(|i| !keys.contains(&i.dedup_key()));
        }
        if let Err(err) = self.flush_meta().await {
            warn!(
                "opendan.session[{}]: flush after consume failed: {err:#}",
                self.session_id
            );
        }
    }

    /// True for a freshly-created Work session that has an objective but
    /// hasn't run any inference yet — the worker should drive an initial
    /// turn from the objective rather than block on the inbox.
    async fn needs_bootstrap_turn(&self) -> bool {
        let meta = self.meta.lock().await;
        !meta.bootstrap_done && !meta.objective.trim().is_empty()
    }

    /// Flip `bootstrap_done = true` and flush. Idempotent — calling twice
    /// is harmless.
    async fn mark_bootstrap_done(&self) {
        let mut changed = false;
        {
            let mut meta = self.meta.lock().await;
            if !meta.bootstrap_done {
                meta.bootstrap_done = true;
                changed = true;
            }
        }
        if changed {
            if let Err(err) = self.flush_meta().await {
                warn!(
                    "opendan.session[{}]: flush after bootstrap_done failed: {err:#}",
                    self.session_id
                );
            }
        }
    }

    /// Build an event-id → `PendingTaskCall` lookup for the worker loop.
    /// The kevent pattern for a task is the literal event id
    /// (`/task_mgr/<task_id>`), so exact match works without globbing.
    async fn pending_task_index(&self) -> std::collections::HashMap<String, PendingTaskCall> {
        let meta = self.meta.lock().await;
        meta.pending_task_calls
            .iter()
            .map(|p| (p.event_pattern.clone(), p.clone()))
            .collect()
    }

    /// Returns true iff `completions` covers every entry in
    /// `meta.pending_task_calls` — required by `LLMContext::resume` which
    /// rejects partial fills.
    async fn all_pending_tasks_collected(
        &self,
        completions: &[(String, Observation, String, String)],
    ) -> bool {
        let pending = self.meta.lock().await.pending_task_calls.clone();
        if completions.len() != pending.len() {
            return false;
        }
        let got: std::collections::HashSet<&str> =
            completions.iter().map(|(c, _, _, _)| c.as_str()).collect();
        pending.iter().all(|p| got.contains(p.call_id.as_str()))
    }

    /// Load the saved snapshot, build a `ResumeFill::ToolResults` from
    /// `completions`, drive the context to its next outcome, then clear
    /// the pending_task_calls + unsubscribe from the task patterns.
    ///
    /// The completion order in `completions` is not guaranteed to match the
    /// snapshot's pending order; we reorder using the snapshot's
    /// `pending_tool_calls` so `LLMContext::resume` accepts the fill.
    async fn resume_with_tool_results(
        &self,
        completions: &[(String, Observation, String, String)],
    ) -> Result<NextAction> {
        let snapshot = self
            .try_load_snapshot()
            .ok_or_else(|| anyhow!("no snapshot to resume against"))?;
        let pending_order: Vec<String> = snapshot
            .state
            .pending_tool_calls
            .iter()
            .map(|p| p.call.call_id.clone())
            .collect();
        if pending_order.is_empty() {
            return Err(anyhow!("snapshot has no pending tool calls to fill"));
        }
        let mut by_id: std::collections::HashMap<String, Observation> = completions
            .iter()
            .map(|(c, o, _, _)| (c.clone(), o.clone()))
            .collect();
        let mut ordered = Vec::with_capacity(pending_order.len());
        for call_id in &pending_order {
            match by_id.remove(call_id) {
                Some(obs) => ordered.push((call_id.clone(), obs)),
                None => {
                    return Err(anyhow!(
                        "missing observation for call_id `{call_id}`"
                    ));
                }
            }
        }
        let fill = ResumeFill::ToolResults { results: ordered };
        let behavior = self.load_current_behavior().await?;
        let trace_id = self.next_trace_id();
        let ctx_runtime = SessionRuntimeContext {
            trace_id: trace_id.clone(),
            agent_name: self.agent_name.clone(),
            behavior: behavior.name.clone(),
            step_idx: snapshot.state.steps.len() as u32,
            wakeup_id: String::new(),
            session_id: self.session_id.clone(),
        };
        let deps = build_session_deps(
            &self.runtime,
            SessionDepsInput {
                tools: self.tools.clone(),
                ctx: ctx_runtime,
                snapshot_path: self.state_snap_path.clone(),
                approval_required: behavior.approval_required.clone(),
                one_line_status: Some(self.status.clone() as Arc<dyn OneLineStatusSink>),
                parser_renderer: behavior.build_parser_and_renderer(),
            },
        );
        let mut ctx = LLMContext::resume(snapshot, fill, deps).map_err(|e| anyhow!("resume: {e}"))?;
        let outcome = ctx.run().await;
        // Post-run snapshot — needed by Done+next_behavior switching to
        // preserve full history (final assistant reply included). Outcome::Done
        // itself carries no snapshot, but ctx is still alive here.
        let final_snapshot = ctx.snapshot();

        // Clear pending_task_calls + unsubscribe from /task_mgr/* patterns.
        // Done before handling the outcome so a subsequent PendingTool emit
        // (chained tool calls) starts from a clean slate.
        let patterns: Vec<String> = completions
            .iter()
            .map(|(_, _, p, _)| p.clone())
            .collect();
        self.clear_pending_task_calls().await;
        for pattern in patterns {
            let _ = self.unsubscribe_event(&pattern).await;
        }

        self.handle_outcome(outcome, &behavior, final_snapshot).await
    }

    /// True iff the worker should not start a fresh turn yet because a
    /// tool round is still in flight. Backed by `meta.pending_task_calls`
    /// (opendan only enters PendingTool via task_mgr-dispatched tools, so
    /// meta is the source of truth for the worker's gating decisions).
    async fn snapshot_has_pending_tool_calls(&self) -> bool {
        !self.meta.lock().await.pending_task_calls.is_empty()
    }

    /// Wind down all in-flight tool calls (per `mode`), persist the
    /// resulting snapshot, and clear session-level pending bookkeeping
    /// (`meta.pending_task_calls` + the corresponding event subscriptions).
    /// Best-effort cancels the upstream task_mgr tasks too.
    ///
    /// No-op when there are no pending tool calls — the session is already
    /// at an outcome boundary; there is nothing to interrupt.
    async fn execute_interrupt(&self, mode: InterruptMode) -> Result<()> {
        let snapshot = match self.try_load_snapshot() {
            Some(s) => s,
            None => {
                info!(
                    "opendan.session[{}]: interrupt({mode:?}) — no snapshot on disk, noop",
                    self.session_id
                );
                return Ok(());
            }
        };
        if snapshot.state.pending_tool_calls.is_empty() {
            info!(
                "opendan.session[{}]: interrupt({mode:?}) — snapshot has no pending tool calls, noop",
                self.session_id
            );
            return Ok(());
        }

        // Best-effort upstream cancel. The session-layer cancellation
        // (Cancelled observations injected below) is what matters for the
        // LLM's view; this just lets task_mgr release the slot for tools
        // that honour cancel signals.
        let pending_task_entries: Vec<PendingTaskCall> =
            self.meta.lock().await.pending_task_calls.clone();
        if let Some(client) = self.runtime.task_mgr.as_ref().cloned() {
            for entry in &pending_task_entries {
                if let Err(err) = client.cancel_task(entry.task_id, true).await {
                    warn!(
                        "opendan.session[{}]: interrupt: cancel_task({}) failed (best effort): {err:#}",
                        self.session_id, entry.task_id
                    );
                }
            }
        }
        // Unsubscribe regardless of cancel outcome — once we've decided to
        // interrupt, late-arriving task events are stale and would route
        // into a snapshot that no longer carries the call.
        for entry in &pending_task_entries {
            if let Err(err) = self.unsubscribe_event(&entry.event_pattern).await {
                warn!(
                    "opendan.session[{}]: interrupt: unsubscribe `{}` failed: {err:#}",
                    self.session_id, entry.event_pattern
                );
            }
        }

        let pending_calls = snapshot.state.pending_tool_calls.clone();
        let reason = self.agent_config.cancel_reason().to_string();

        match mode {
            InterruptMode::Graceful => {
                self.execute_interrupt_graceful(snapshot, &pending_calls, reason)
                    .await?
            }
            InterruptMode::Discard => {
                self.execute_interrupt_discard(snapshot, &pending_calls).await?
            }
        }

        self.clear_pending_task_calls().await;
        Ok(())
    }

    /// Graceful interrupt: feed `Observation::Cancelled` for each pending
    /// call via `ResumeFill::ToolResults` and drive the resumed context to
    /// a terminal outcome. The resumed snapshot has `tool_policy.max_rounds`
    /// overridden to 0 so the LLM's wind-down inference cannot launch new
    /// tool calls — any attempt becomes `BudgetExhausted(ToolRounds)` and
    /// the partial assistant text is preserved in `accumulated`.
    async fn execute_interrupt_graceful(
        &self,
        snapshot: LLMContextSnapshot,
        pending_calls: &[llm_context::observation::PendingToolCall],
        reason: String,
    ) -> Result<()> {
        let results: Vec<(String, Observation)> = pending_calls
            .iter()
            .map(|p| {
                (
                    p.call.call_id.clone(),
                    Observation::Cancelled {
                        call_id: p.call.call_id.clone(),
                        reason: reason.clone(),
                    },
                )
            })
            .collect();

        let mut tp = snapshot.request.tool_policy.clone();
        tp.max_rounds = 0;
        let snap_winddown = apply_overrides_to_snapshot(
            snapshot,
            RequestOverrides {
                tool_policy: Some(tp),
                reset_rounds: true,
                ..Default::default()
            },
        );

        let behavior = self.load_current_behavior().await?;
        let trace_id = self.next_trace_id();
        let ctx_runtime = SessionRuntimeContext {
            trace_id,
            agent_name: self.agent_name.clone(),
            behavior: behavior.name.clone(),
            step_idx: snap_winddown.state.steps.len() as u32,
            wakeup_id: String::new(),
            session_id: self.session_id.clone(),
        };
        let deps = build_session_deps(
            &self.runtime,
            SessionDepsInput {
                tools: self.tools.clone(),
                ctx: ctx_runtime,
                snapshot_path: self.state_snap_path.clone(),
                approval_required: behavior.approval_required.clone(),
                one_line_status: Some(self.status.clone() as Arc<dyn OneLineStatusSink>),
                parser_renderer: behavior.build_parser_and_renderer(),
            },
        );

        let mut ctx = LLMContext::resume(
            snap_winddown,
            ResumeFill::ToolResults { results },
            deps,
        )
        .map_err(|e| anyhow!("interrupt graceful resume: {e}"))?;
        // Whether the outcome is Done (LLM produced a clean acknowledgement)
        // or BudgetExhausted(ToolRounds) (LLM tried to launch a new tool and
        // got rejected), the post-run snapshot captures everything we want
        // — including the partial assistant text — in `state.accumulated`.
        let _outcome = ctx.run().await;
        let final_snapshot = ctx.snapshot();
        self.persist_snapshot(&final_snapshot).await;
        Ok(())
    }

    /// Discard interrupt: locate the trailing assistant turn that owns the
    /// unresolved `tool_use` blocks and truncate `accumulated` at (before)
    /// that index. Then clear `pending_tool_calls` and persist. Any tool
    /// side effects already in flight externally are *not* reflected in
    /// the post-truncation history.
    async fn execute_interrupt_discard(
        &self,
        mut snapshot: LLMContextSnapshot,
        pending_calls: &[llm_context::observation::PendingToolCall],
    ) -> Result<()> {
        let pending_ids: std::collections::HashSet<&str> = pending_calls
            .iter()
            .map(|p| p.call.call_id.as_str())
            .collect();

        let cutoff = snapshot.state.accumulated.iter().rposition(|msg| {
            matches!(msg.role, AiRole::Assistant)
                && msg.content.iter().any(|c| matches!(c,
                    AiContent::ToolUse { call_id, .. } if pending_ids.contains(call_id.as_str())
                ))
        });
        if let Some(idx) = cutoff {
            snapshot.state.accumulated.truncate(idx);
        } else {
            warn!(
                "opendan.session[{}]: interrupt(Discard): no assistant turn owns the pending tool_use blocks; clearing pending_tool_calls without truncation",
                self.session_id
            );
        }
        snapshot.state.pending_tool_calls.clear();
        self.persist_snapshot(&snapshot).await;
        Ok(())
    }

    /// Poll task_mgr for every entry in `meta.pending_task_calls`; for each
    /// task that has already reached a terminal status, synthesize the
    /// corresponding `/task_mgr/<id>` Event into `pending_inputs` so the
    /// regular drain path reconciles it. Returns `true` when at least one
    /// terminal event was synthesized.
    ///
    /// Rationale: kevent is an **acceleration channel**, not the source of
    /// truth — broker restarts, missed deliveries, or unsubscribe races can
    /// leave the session waiting forever for an event that already fired.
    /// The worker's WaitingTool recv sites call this on a timed tick to
    /// guarantee forward progress.
    async fn sweep_pending_tool_calls(&self) -> bool {
        let entries = self.meta.lock().await.pending_task_calls.clone();
        if entries.is_empty() {
            return false;
        }
        let Some(client) = self.runtime.task_mgr.as_ref().cloned() else {
            return false;
        };
        let mut synthesized = 0u32;
        for entry in entries {
            match client.get_task(entry.task_id).await {
                Ok(task) => {
                    if !task.status.is_terminal() {
                        continue;
                    }
                    let payload = serde_json::json!({
                        "to_status": task.status.to_string(),
                        "data": task.data,
                        "message": task.message.clone().unwrap_or_default(),
                    });
                    let event = PendingInput::Event {
                        event_id: entry.event_pattern.clone(),
                        data: payload,
                    };
                    // dedup_key on Event uses event_id; if a kevent for the
                    // same task is already queued (raced ahead), this is a
                    // no-op via enqueue_pending's de-dup. Otherwise the
                    // worker drains the synthetic next iteration.
                    if let Err(err) = self.enqueue_pending(event).await {
                        warn!(
                            "opendan.session[{}]: sweep enqueue for task {} failed: {err:#}",
                            self.session_id, entry.task_id
                        );
                    } else {
                        synthesized = synthesized.saturating_add(1);
                    }
                }
                Err(err) => {
                    // get_task failure is non-fatal: leave the entry alone
                    // so the next sweep retries.
                    warn!(
                        "opendan.session[{}]: sweep get_task({}) failed: {err:#}",
                        self.session_id, entry.task_id
                    );
                }
            }
        }
        if synthesized > 0 {
            info!(
                "opendan.session[{}]: sweep synthesized {synthesized} terminal task event(s)",
                self.session_id
            );
        }
        synthesized > 0
    }

    /// Wait for an inbox signal, but also fire `sweep_pending_tool_calls`
    /// on a periodic tick. When the sweep enqueues at least one synthetic
    /// event, return `Wakeup` immediately so the worker re-drains. Used
    /// only at recv sites where the session is actively in WaitingTool
    /// (idle session recvs don't need a sweep — there's nothing to
    /// reconcile).
    async fn wait_with_tool_sweep(
        &self,
        inbox_rx: &mut mpsc::Receiver<SessionInput>,
    ) -> Option<SessionInput> {
        const SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
        loop {
            tokio::select! {
                sig = inbox_rx.recv() => return sig,
                _ = tokio::time::sleep(SWEEP_INTERVAL) => {
                    if self.sweep_pending_tool_calls().await {
                        return Some(SessionInput::Wakeup);
                    }
                }
            }
        }
    }

    /// Empty `meta.pending_task_calls` and flush. Called after a successful
    /// resume so the next iteration doesn't try to match orphan entries.
    async fn clear_pending_task_calls(&self) {
        {
            let mut meta = self.meta.lock().await;
            meta.pending_task_calls.clear();
        }
        if let Err(err) = self.flush_meta().await {
            warn!(
                "opendan.session[{}]: flush after clear_pending_task_calls failed: {err:#}",
                self.session_id
            );
        }
    }

    /// Append a new pending tool task entry and flush. The caller is
    /// expected to also call `subscribe_event` so the event pump receives
    /// completion notifications.
    async fn add_pending_task_call(&self, entry: PendingTaskCall) {
        {
            let mut meta = self.meta.lock().await;
            // De-dup by call_id — a re-dispatch of the same call (e.g.
            // after a snapshot reload) shouldn't multiply entries.
            meta.pending_task_calls
                .retain(|p| p.call_id != entry.call_id);
            meta.pending_task_calls.push(entry);
        }
        if let Err(err) = self.flush_meta().await {
            warn!(
                "opendan.session[{}]: flush after add_pending_task_call failed: {err:#}",
                self.session_id
            );
        }
    }

    /// Persist `snapshot` to `state.snap` (atomic). Used by the
    /// PendingTool outcome path so a restart can resume from the freshest
    /// view — the TurnHook write happens *before* inference, which would
    /// miss the freshly-populated `pending_tool_calls`.
    async fn persist_snapshot(&self, snapshot: &LLMContextSnapshot) {
        self.persist_snapshot_to(&self.state_snap_path, snapshot)
            .await;
    }

    /// Lower-level: write a snapshot to a specific path (used by
    /// independent-mode per-behavior snapshot files). Same crash-consistency
    /// guarantees as `persist_snapshot` (tmp + rename).
    async fn persist_snapshot_to(&self, path: &Path, snapshot: &LLMContextSnapshot) {
        let bytes = match serde_json::to_vec(snapshot) {
            Ok(v) => v,
            Err(err) => {
                warn!(
                    "opendan.session[{}]: snapshot serialize failed: {err}",
                    self.session_id
                );
                return;
            }
        };
        if let Some(parent) = path.parent() {
            if let Err(err) = tokio::fs::create_dir_all(parent).await {
                warn!(
                    "opendan.session[{}]: snapshot mkdir failed: {err}",
                    self.session_id
                );
                return;
            }
        }
        let tmp = path.with_extension("snap.tmp");
        if let Err(err) = tokio::fs::write(&tmp, &bytes).await {
            warn!(
                "opendan.session[{}]: snapshot write failed: {err}",
                self.session_id
            );
            return;
        }
        if let Err(err) = tokio::fs::rename(&tmp, path).await {
            warn!(
                "opendan.session[{}]: snapshot rename failed: {err}",
                self.session_id
            );
        }
    }

    async fn run_one_turn(&self, human_texts: Vec<String>) -> Result<NextAction> {
        let behavior = self.load_current_behavior().await?;
        let trace_id = self.next_trace_id();
        let (ctx_owner, _request, deps) =
            self.build_or_resume(&behavior, &human_texts, &trace_id)?;
        let mut ctx = match ctx_owner {
            BuiltContext::Fresh(c) => c,
            BuiltContext::Resumed(c) => c,
        };
        // ContextLimitReached re-entry loop: compress the accumulated
        // history (opendan-side, message-level) and resume the same
        // snapshot via `RewrittenHistory`. Bounded so a pathological
        // history that keeps tripping the limit can't pin the worker.
        const MAX_COMPRESS_ROUNDS: u32 = 3;
        let mut compress_rounds = 0u32;
        loop {
            let outcome = ctx.run().await;
            match outcome {
                LLMContextOutcome::ContextLimitReached {
                    which,
                    accumulated,
                    snapshot,
                    ..
                } => {
                    if compress_rounds >= MAX_COMPRESS_ROUNDS {
                        warn!(
                            "opendan.session[{}]: ContextLimitReached after {compress_rounds} compress rounds ({:?}); aborting turn",
                            self.session_id, which
                        );
                        // Out of budget for compressions — surface to the
                        // standard outcome handler as a non-resumable error.
                        let final_snapshot = snapshot.clone();
                        return self
                            .handle_outcome(
                                LLMContextOutcome::Error {
                                    error: llm_context::error::LLMComputeError::Internal(
                                        format!(
                                            "context limit reached {:?} and {compress_rounds} \
                                             compress rounds exhausted",
                                            which
                                        ),
                                    ),
                                    usage: snapshot.state.usage.clone(),
                                },
                                &behavior,
                                final_snapshot,
                            )
                            .await;
                    }
                    compress_rounds += 1;
                    let before_len = accumulated.len();
                    let rewritten = compress_messages_for_context_limit(accumulated);
                    let after_len = rewritten.len();
                    info!(
                        "opendan.session[{}]: ContextLimitReached ({:?}); compressed history {before_len} → {after_len} messages (round {compress_rounds}/{MAX_COMPRESS_ROUNDS})",
                        self.session_id, which
                    );
                    // Persist the post-compression snapshot before re-running
                    // so a crash mid-compress doesn't lose the rewrite.
                    let mut prepared = snapshot;
                    prepared.state.accumulated = rewritten.clone();
                    self.persist_snapshot(&prepared).await;
                    ctx = LLMContext::resume(
                        prepared,
                        ResumeFill::RewrittenHistory { history: rewritten },
                        deps.clone(),
                    )
                    .map_err(|e| anyhow!("resume after compression: {e}"))?;
                    continue;
                }
                other => {
                    let final_snapshot = ctx.snapshot();
                    return self.handle_outcome(other, &behavior, final_snapshot).await;
                }
            }
        }
    }

    fn next_trace_id(&self) -> String {
        let n = self
            .trace_seq
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        format!("{}-{}", self.session_id, n)
    }

    fn build_or_resume(
        &self,
        behavior: &BehaviorCfg,
        human_texts: &[String],
        trace_id: &str,
    ) -> Result<(BuiltContext, LLMContextRequest, llm_context::deps::LLMContextDeps)> {
        let ctx = SessionRuntimeContext {
            trace_id: trace_id.to_string(),
            agent_name: self.agent_name.clone(),
            behavior: behavior.name.clone(),
            step_idx: 0,
            wakeup_id: String::new(),
            session_id: self.session_id.clone(),
        };
        let parser_renderer = behavior.build_parser_and_renderer();
        let approval_required = behavior.approval_required.clone();

        let deps = build_session_deps(
            &self.runtime,
            SessionDepsInput {
                tools: self.tools.clone(),
                ctx,
                snapshot_path: self.state_snap_path.clone(),
                approval_required,
                one_line_status: Some(self.status.clone() as Arc<dyn OneLineStatusSink>),
                parser_renderer,
            },
        );

        // Compose the per-turn "environment-aware message" once so both the
        // resume and fresh-build branches see it. The message is the
        // opendan-side surface for §5 "环境感知 message" — bundles current
        // workspace / behavior / activity hints so the LLM doesn't have to
        // re-discover them every turn.
        //
        // Emit env **only when there is real human/event input driving this
        // turn**. Mid-run resumes (no human text, snapshot present) must
        // not inject a synthetic User message or they'd promote an idle
        // wakeup into a fake conversational turn. Bootstrap turns (work
        // session first run, no input, no snapshot) get the objective via
        // System and don't need env either.
        let human_body = compose_human_text(human_texts);
        let user_message_text = match human_body {
            Some(h) => {
                let env_body = self.compose_environment_message(behavior);
                merge_env_and_human(env_body, Some(h))
            }
            None => None,
        };

        if let Some(snapshot) = self.try_load_snapshot() {
            if snapshot.state.pending_tool_calls.is_empty() {
                if let Some(text) = user_message_text.clone() {
                    // Idle session + new user message: build a fresh
                    // LLMContext whose conversation history *is* the
                    // snapshot's accumulated (already includes the system
                    // segment that was sediment-cloned at first inference),
                    // with the new user turn appended. Per-turn state
                    // (consecutive_errors, usage, steps, trace) resets here;
                    // cross-turn accumulation lives on SessionMeta.
                    let LLMContextSnapshot { mut request, state } = snapshot;
                    let mut input = state.accumulated;
                    input.push(AiMessage::text(AiRole::User, text));
                    request.input = input;
                    request.trace = Some(trace_id.to_string());
                    let fresh = LLMContext::new(request.clone(), deps.clone());
                    return Ok((BuiltContext::Fresh(fresh), request, deps));
                }
                // No new user input — resume the snapshot in place
                // (crash-recovery / idle re-entry without driver).
                let request = snapshot.request.clone();
                let resumed = LLMContext::resume(
                    snapshot,
                    ResumeFill::ResumeFromMidRun,
                    deps.clone(),
                )
                .map_err(|e| anyhow!("resume: {e}"))?;
                return Ok((BuiltContext::Resumed(resumed), request, deps));
            }
            warn!(
                "opendan.session[{}]: snapshot has pending tool calls but resume path is not wired; starting fresh",
                self.session_id
            );
        }

        let mut input = self.render_system_messages(behavior);
        if let Some(text) = user_message_text {
            input.push(AiMessage::text(AiRole::User, text));
        }
        let request = LLMContextRequest {
            owner: ContextOwnerRef::Agent {
                session_id: self.session_id.clone(),
            },
            trace: Some(trace_id.to_string()),
            objective: behavior.objective.clone(),
            input,
            model_policy: behavior.to_model_policy(),
            tool_policy: behavior.to_tool_policy(),
            output: behavior.to_output_spec(),
            budget: behavior.to_budget_spec(),
            human_policy: behavior.to_human_policy(),
            error_policy: behavior.to_error_policy(),
            forbid_next_behavior: false,
        };
        let fresh = LLMContext::new(request.clone(), deps.clone());
        Ok((BuiltContext::Fresh(fresh), request, deps))
    }

    fn try_load_snapshot(&self) -> Option<LLMContextSnapshot> {
        self.try_load_snapshot_from(&self.state_snap_path)
    }

    /// Read-only access to the session's most-recently-persisted snapshot.
    /// Returns `None` when no snapshot exists yet (fresh session, or one
    /// that has been `discard_snapshot`-ed). Intended for prompt-rendering
    /// consumers (e.g. fork sub-context history injection) — do **not** use
    /// this for resumption; that goes through `build_or_resume`.
    pub fn try_load_snapshot_for_prompt(&self) -> Option<LLMContextSnapshot> {
        self.try_load_snapshot()
    }

    /// Lower-level: load a snapshot from a specific path. Returns `None` on
    /// missing-file (silent) or unreadable / malformed (warns).
    fn try_load_snapshot_from(&self, path: &Path) -> Option<LLMContextSnapshot> {
        let bytes = std::fs::read(path).ok()?;
        match serde_json::from_slice::<LLMContextSnapshot>(&bytes) {
            Ok(s) => Some(s),
            Err(err) => {
                warn!(
                    "opendan.session[{}]: snapshot at {} unreadable: {err}",
                    self.session_id,
                    path.display()
                );
                None
            }
        }
    }

    /// Resolve the per-process snapshot path for an independent-mode entry
    /// behavior. Rejects names that could escape `.meta/` via path traversal.
    fn behavior_snap_path(&self, entry: &str) -> Result<PathBuf> {
        if entry.is_empty() || entry.contains('/') || entry.contains('\\') || entry.contains("..")
        {
            return Err(anyhow!(
                "invalid process entry name `{entry}` for snapshot path"
            ));
        }
        Ok(self
            .session_dir
            .join(".meta")
            .join(format!("behavior_{entry}.snap")))
    }

    /// Build a fresh (no inherited state) [`LLMContextRequest`] for the given
    /// behavior. Used by independent-mode first-time entry into a process.
    fn fresh_request_for(&self, cfg: &BehaviorCfg) -> LLMContextRequest {
        LLMContextRequest {
            owner: ContextOwnerRef::Agent {
                session_id: self.session_id.clone(),
            },
            trace: None,
            objective: cfg.objective.clone(),
            input: self.render_system_messages(cfg),
            model_policy: cfg.to_model_policy(),
            tool_policy: cfg.to_tool_policy(),
            output: cfg.to_output_spec(),
            budget: cfg.to_budget_spec(),
            human_policy: cfg.to_human_policy(),
            error_policy: cfg.to_error_policy(),
            forbid_next_behavior: false,
        }
    }

    /// Compose the "environment-aware message" — a short, structured
    /// summary of the session's current environment that we prefix onto
    /// each turn's user input. Per §5 of `notepads/NewOpenDANRuntime.md`
    /// the message should eventually include auto-recalled memory and an
    /// event/message diff; the MVP version assembles the bits we can read
    /// synchronously without grabbing the async meta lock:
    ///
    /// - Current behavior name (so the LLM knows which prompt context it's
    ///   operating under after a `Normal`-mode switch).
    /// - Workspace binding id (when present).
    /// - One-line activity status (filled by tools through the
    ///   `OneLineStatusSink`).
    /// - Wall-clock timestamp so the LLM can reason about "now".
    ///
    /// Returns `None` when nothing useful can be rendered — caller then
    /// falls back to just the raw human-text input (or `ResumeFromMidRun`).
    /// `meta.try_lock` failures degrade silently (returns `None`); the
    /// fact that a turn is currently driving an inference is rare to
    /// happen concurrently with build_or_resume anyway.
    fn compose_environment_message(&self, behavior: &BehaviorCfg) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();
        let workspace_id;
        let one_line;
        let title;
        match self.meta.try_lock() {
            Ok(g) => {
                workspace_id = g.workspace_id.clone();
                one_line = g.one_line_status.clone();
                title = g.title.clone();
            }
            Err(_) => return None,
        }
        // Always include behavior — the LLM can otherwise lose track after
        // a Normal-mode switch with no explicit hand-off.
        parts.push(format!("behavior: `{}`", behavior.name));
        if !title.trim().is_empty() {
            parts.push(format!("session: `{}` (\"{}\")", self.session_id, title.trim()));
        } else {
            parts.push(format!("session: `{}`", self.session_id));
        }
        if let Some(ws) = workspace_id.as_deref().filter(|s| !s.is_empty()) {
            parts.push(format!("workspace: `{}`", ws));
        }
        let trimmed_status = one_line.trim();
        if !trimmed_status.is_empty() {
            parts.push(format!("recent activity: {}", trimmed_status));
        }
        parts.push(format!("clock: unix_ms={}", now_ms()));
        Some(format!("[environment]\n{}", parts.join("\n")))
    }

    fn render_system_messages(&self, behavior: &BehaviorCfg) -> Vec<AiMessage> {
        let mut messages = Vec::new();
        let template = behavior.system_prompt_template.trim();
        if !template.is_empty() {
            messages.push(AiMessage::text(AiRole::System, template.to_string()));
            return messages;
        }
        let mut chunks = Vec::new();
        for fname in ["role.md", "self.md"] {
            if let Ok(text) = std::fs::read_to_string(self.agent_config.layout.root.join(fname)) {
                if !text.trim().is_empty() {
                    chunks.push(text);
                }
            }
        }
        // Work-session objective: surface as a dedicated block ahead of the
        // session readme so the LLM sees its task statement first.
        let (objective, title) = match self.meta.try_lock() {
            Ok(g) => (g.objective.clone(), g.title.clone()),
            Err(_) => (String::new(), String::new()),
        };
        if !objective.trim().is_empty() {
            let header = if title.trim().is_empty() {
                "## Objective".to_string()
            } else {
                format!("## Objective: {}", title.trim())
            };
            chunks.push(format!("{header}\n{}", objective.trim()));
        }
        if let Ok(text) = std::fs::read_to_string(self.session_dir.join("readme.md")) {
            if !text.trim().is_empty() {
                chunks.push(text);
            }
        }
        if chunks.is_empty() {
            chunks.push(format!(
                "You are agent `{}` (session {}). Be helpful, concise, and use the available tools when appropriate.",
                self.agent_name, self.session_id
            ));
        }
        messages.push(AiMessage::text(AiRole::System, chunks.join("\n\n")));
        messages
    }

    async fn load_current_behavior(&self) -> Result<BehaviorCfg> {
        let name = self.meta.lock().await.current_behavior.clone();
        if name.trim().is_empty() {
            return Ok(AgentConfig::builtin_ui_default());
        }
        match self.agent_config.load_behavior(&name) {
            Ok(b) => Ok(b),
            Err(err) => {
                warn!(
                    "opendan.session[{}]: load behavior `{}` failed: {err}; falling back to builtin ui_default",
                    self.session_id, name
                );
                Ok(AgentConfig::builtin_ui_default())
            }
        }
    }

    async fn handle_outcome(
        &self,
        outcome: LLMContextOutcome,
        behavior: &BehaviorCfg,
        final_snapshot: LLMContextSnapshot,
    ) -> Result<NextAction> {
        match outcome {
            LLMContextOutcome::Done {
                output,
                behavior_result,
                ..
            } => {
                if let Some(text) = output_to_text(&output) {
                    // Outbound first: send the reply through msg-center so
                    // the original peer receives it. Failure is logged but
                    // not fatal — local SessionReply::AssistantText still
                    // fires so CLI / log consumers see the answer.
                    self.post_outbound_text(&text).await;
                    let _ = self
                        .reply_tx
                        .send(SessionReply::AssistantText { text })
                        .await;
                }
                if let Some(next) = behavior_result.and_then(|r| r.next_behavior) {
                    let trimmed = next.trim();
                    if trimmed.eq_ignore_ascii_case("END") {
                        // Independent-mode call-stack-aware End: pop a
                        // parent frame if one is waiting; only an empty
                        // stack means the session itself is done.
                        return self.handle_process_end(final_snapshot).await;
                    }
                    if trimmed.eq_ignore_ascii_case(NEXT_BEHAVIOR_WAIT_USER_MSG) {
                        // Behavior state machine yields: current intent has
                        // run its course, no autonomous next step — park
                        // the session until the next user message arrives.
                        // Persist the post-run snapshot so the next-turn
                        // rebuild (`build_or_resume` → `LLMContext::new`
                        // from `state.accumulated + [new_user_msg]`)
                        // continues from the final assistant turn rather
                        // than the stale pre-inference TurnHook write.
                        // The worker maps `WaitForMsg` to
                        // `SessionStatus::WaitingInput`, which is what
                        // forward_msg's inbox routing uses to find this
                        // session.
                        self.persist_snapshot(&final_snapshot).await;
                        return Ok(NextAction::WaitForMsg);
                    }
                    // Switch — preserve history by handing the post-run
                    // snapshot to switch_behavior (which applies the new
                    // behavior's overrides and persists). Do **not** discard
                    // here; next turn resumes from the rebuilt snapshot.
                    self.switch_behavior(trimmed, behavior, final_snapshot).await?;
                    return Ok(NextAction::Idle);
                }
                // Natural Done (no next_behavior). Independent-mode
                // sub-processes must keep their stream alive across this
                // boundary so a future wake / re-entry resumes from where
                // it left off; top-level processes keep the existing
                // "discard, next turn rebuilds fresh" semantics.
                let in_subprocess = !self.meta.lock().await.process_stack.is_empty();
                if in_subprocess {
                    self.persist_snapshot(&final_snapshot).await;
                } else {
                    self.discard_snapshot();
                }
                if matches!(self.kind, SessionKind::Ui) {
                    Ok(NextAction::WaitForMsg)
                } else {
                    Ok(NextAction::End)
                }
            }
            LLMContextOutcome::PendingTool {
                pending, snapshot, ..
            } => {
                // Persist the snapshot first — `pending_tool_calls` is the
                // load-bearing field for the resume path, and the TurnHook
                // pre-inference write would have missed it.
                self.persist_snapshot(&snapshot).await;

                let Some(client) = self.runtime.task_mgr.as_ref().cloned() else {
                    warn!(
                        "opendan.session[{}]: PendingTool outcome — task_mgr unavailable, parking session",
                        self.session_id
                    );
                    return Ok(NextAction::WaitForMsg);
                };
                // Owner key for the dispatched task — fall back to the
                // session's owner / agent name so multi-tenant deployments
                // can scope correctly.
                let owner_for_task = if !self.owner.trim().is_empty() {
                    self.owner.clone()
                } else {
                    self.agent_name.clone()
                };
                let dispatcher = TaskDispatch::new(client, owner_for_task);

                let mut dispatched_any = false;
                for pcall in pending {
                    let call_id = pcall.call.call_id.clone();
                    let tool_name = pcall.call.name.clone();
                    let args_json = serde_json::to_value(&pcall.call.args)
                        .unwrap_or(serde_json::Value::Null);
                    match dispatcher
                        .dispatch_async_tool(
                            &self.session_id,
                            &tool_name,
                            args_json,
                        )
                        .await
                    {
                        Ok(handle) => {
                            let pattern = format!("/task_mgr/{}", handle.task_id);
                            self.add_pending_task_call(PendingTaskCall {
                                call_id: call_id.clone(),
                                tool_name: tool_name.clone(),
                                task_id: handle.task_id,
                                event_pattern: pattern.clone(),
                            })
                            .await;
                            // subscribe_event refreshes the event pump
                            // automatically; ignore the bool — adding the
                            // same pattern twice is a no-op.
                            if let Err(err) = self.subscribe_event(pattern.clone()).await {
                                warn!(
                                    "opendan.session[{}]: subscribe `{pattern}` for task {} failed: {err:#}",
                                    self.session_id, handle.task_id
                                );
                            }
                            dispatched_any = true;
                        }
                        Err(err) => {
                            warn!(
                                "opendan.session[{}]: dispatch task for call_id={} tool={} failed: {err:#}",
                                self.session_id, call_id, tool_name
                            );
                        }
                    }
                }
                if !dispatched_any {
                    // Couldn't park anything externally — session can't
                    // make progress here. Drop the snapshot so the next
                    // user message starts a fresh turn rather than trying
                    // to resume against a snapshot we can't fulfill.
                    self.discard_snapshot();
                    return Ok(NextAction::WaitForMsg);
                }
                Ok(NextAction::WaitForTool)
            }
            LLMContextOutcome::BudgetExhausted { which, partial, .. } => {
                // The producer (`context_loop.rs`) preserves whatever
                // assistant text the LLM had emitted before the budget
                // gate fired (e.g. token cap mid-stream, or the explicit
                // wind-down case where a tool attempt is rejected by
                // `max_rounds=0` but the assistant ack is already there).
                // Surface that text before discarding the snapshot so it
                // isn't silently lost.
                if let Some(text) = partial.as_ref().and_then(output_to_text) {
                    self.post_outbound_text(&text).await;
                    let _ = self
                        .reply_tx
                        .send(SessionReply::AssistantText { text })
                        .await;
                }
                let _ = self
                    .reply_tx
                    .send(SessionReply::Error {
                        message: format!("budget exhausted: {:?}", which),
                    })
                    .await;
                self.discard_snapshot();
                Ok(NextAction::WaitForMsg)
            }
            LLMContextOutcome::Error { error, .. } => {
                let _ = self
                    .reply_tx
                    .send(SessionReply::Error {
                        message: error.to_string(),
                    })
                    .await;
                self.discard_snapshot();
                Ok(NextAction::WaitForMsg)
            }
            LLMContextOutcome::ContextLimitReached { which, .. } => {
                // Should not happen — `run_one_turn` intercepts
                // ContextLimitReached and either resumes via
                // `ResumeFill::RewrittenHistory` or maps to an Error after
                // exhausting the compress budget. If we land here, the
                // re-entry loop is broken; surface it so the bug is loud.
                warn!(
                    "opendan.session[{}]: ContextLimitReached reached handle_outcome (compress loop bypassed?); kind={:?}",
                    self.session_id, which
                );
                let _ = self
                    .reply_tx
                    .send(SessionReply::Error {
                        message: format!("context limit reached: {:?}", which),
                    })
                    .await;
                Ok(NextAction::WaitForMsg)
            }
        }
    }

    fn discard_snapshot(&self) {
        if self.state_snap_path.exists() {
            if let Err(err) = std::fs::remove_file(&self.state_snap_path) {
                warn!(
                    "opendan.session[{}]: remove snapshot {} failed: {err}",
                    self.session_id,
                    self.state_snap_path.display()
                );
            }
        }
    }

    async fn switch_behavior(
        &self,
        next: &str,
        _prev: &BehaviorCfg,
        final_snapshot: LLMContextSnapshot,
    ) -> Result<()> {
        let new_cfg = self
            .agent_config
            .load_behavior(next)
            .map_err(|err| anyhow!("load behavior `{next}`: {err}"))?;
        match new_cfg.switch_mode {
            SwitchMode::Normal => {
                self.apply_switch_normal(&new_cfg, final_snapshot).await;
                self.meta.lock().await.current_behavior = new_cfg.name.clone();
            }
            SwitchMode::Independent => {
                self.apply_switch_independent(&new_cfg, final_snapshot)
                    .await?;
                // process_entry / current_behavior already updated inside
                // apply_switch_independent (push happens under the same lock).
            }
            SwitchMode::Fork => {
                warn!(
                    "opendan.session[{}]: switch_mode=Fork not yet wired \
                     (Phase 4 — treating as Normal for now)",
                    self.session_id
                );
                self.apply_switch_normal(&new_cfg, final_snapshot).await;
                self.meta.lock().await.current_behavior = new_cfg.name.clone();
            }
        }
        if let Err(err) = self.flush_meta().await {
            warn!(
                "opendan.session[{}]: flush after behavior switch failed: {err:#}",
                self.session_id
            );
        }
        Ok(())
    }

    /// Switch mode = Normal: keep accumulated history + step records, swap
    /// system messages and behavior policies via [`apply_overrides_to_snapshot`],
    /// persist as the new `state.snap`. Next turn's `build_or_resume` picks it
    /// up and resumes under the new behavior.
    ///
    /// Per the design doc (llm_context_helper.rs §旋钮):
    /// - rounds_left: NOT reset (continue parent budget)
    /// - consecutive_errors: NOT cleared (block LLM from bypassing the cap
    ///   by switching behavior)
    async fn apply_switch_normal(
        &self,
        new_cfg: &BehaviorCfg,
        final_snapshot: LLMContextSnapshot,
    ) {
        let new_system = self.render_system_messages(new_cfg);
        let overrides = RequestOverrides {
            system_messages: Some(new_system),
            tool_policy: Some(new_cfg.to_tool_policy()),
            objective: Some(new_cfg.objective.clone()),
            model_policy: Some(new_cfg.to_model_policy()),
            budget: Some(new_cfg.to_budget_spec()),
            human_policy: Some(new_cfg.to_human_policy()),
            error_policy: Some(new_cfg.to_error_policy()),
            output: Some(new_cfg.to_output_spec()),
            trace: None,
            reset_rounds: false,
            reset_errors: false,
            forbid_next_behavior: false,
        };
        let rebuilt = apply_overrides_to_snapshot(final_snapshot, overrides);
        self.persist_snapshot(&rebuilt).await;
    }

    /// Switch mode = Independent: each behavior name is its own "process"
    /// with its own step record stream. The parent's `final_snapshot` is
    /// archived to `.meta/behavior_<parent_entry>.snap`; the child resumes
    /// from `.meta/behavior_<child>.snap` (if it has been entered before) or
    /// is built fresh. The active `state.snap` always mirrors the top-of-
    /// stack process.
    ///
    /// Per design旋钮: rounds_left and consecutive_errors are reset on every
    /// (re-)entry so each process has its own budget / error window.
    async fn apply_switch_independent(
        &self,
        new_cfg: &BehaviorCfg,
        final_snapshot: LLMContextSnapshot,
    ) -> Result<()> {
        // 1. Persist the parent process's terminal state to its per-process
        //    snapshot file. Use the captured `process_entry` so an intra-
        //    process normal switch on the parent still archives to the
        //    right file.
        let (parent_entry, parent_current) = {
            let meta = self.meta.lock().await;
            (meta.process_entry.clone(), meta.current_behavior.clone())
        };
        let parent_path = self.behavior_snap_path(&parent_entry)?;
        self.persist_snapshot_to(&parent_path, &final_snapshot).await;

        // 2. Resume (or build fresh) the child process's snapshot.
        let child_path = self.behavior_snap_path(&new_cfg.name)?;
        let child_snap = if let Some(loaded) = self.try_load_snapshot_from(&child_path) {
            // Existing stream — keep its system / accumulated / steps, just
            // reset the ephemeral counters so the new "turn under this
            // process" starts with a clean budget.
            let overrides = RequestOverrides {
                reset_rounds: true,
                reset_errors: true,
                ..Default::default()
            };
            apply_overrides_to_snapshot(loaded, overrides)
        } else {
            // First-time entry — synthesize a fresh snapshot from this
            // behavior's request template. Mirrors `build_fresh` at the
            // snapshot level (we don't construct an LLMContext here because
            // the next worker turn will do the resume).
            let request = self.fresh_request_for(new_cfg);
            let state = LLMContextState::from_request(&request, now_ms());
            LLMContextSnapshot { request, state }
        };
        self.persist_snapshot(&child_snap).await;

        // 3. Push parent frame, update active-process tracking.
        {
            let mut meta = self.meta.lock().await;
            meta.process_stack.push(ProcessFrame {
                entry: parent_entry,
                current: parent_current,
            });
            meta.process_entry = new_cfg.name.clone();
            meta.current_behavior = new_cfg.name.clone();
        }
        Ok(())
    }

    /// Drive the independent-mode call-stack pop on `END`. If a parent
    /// frame is waiting, persist this process's terminal state (so a future
    /// re-entry resumes its stream), restore the parent's snapshot to
    /// `state.snap`, inject a marker `[independent process `<X>` ended]`
    /// message into the parent's `pending_inputs` so the parent's next turn
    /// has something to wake on, and return `NextAction::Idle`.
    ///
    /// Returns `NextAction::End` only when the call stack is empty — i.e.
    /// the top-level process itself ended.
    async fn handle_process_end(&self, final_snapshot: LLMContextSnapshot) -> Result<NextAction> {
        // Pop under the lock; capture both the child entry name (for the
        // marker payload + file persistence) and the parent frame.
        let popped = {
            let mut meta = self.meta.lock().await;
            if let Some(parent) = meta.process_stack.pop() {
                let child_entry =
                    std::mem::replace(&mut meta.process_entry, parent.entry.clone());
                meta.current_behavior = parent.current.clone();
                Some((child_entry, parent))
            } else {
                None
            }
        };

        let Some((child_entry, parent_frame)) = popped else {
            // Top-level process ended — real session End.
            self.discard_snapshot();
            return Ok(NextAction::End);
        };

        // Persist child's terminal snapshot so a future re-entry sees its
        // full step record stream.
        if let Ok(child_path) = self.behavior_snap_path(&child_entry) {
            self.persist_snapshot_to(&child_path, &final_snapshot).await;
        }

        // Restore parent's snapshot to state.snap. If the file vanished
        // (manual deletion / disk corruption), warn and start the parent
        // fresh on its next turn — the meta-level call stack is still
        // correct, and `build_or_resume` falls back to render-fresh.
        let parent_path = self
            .behavior_snap_path(&parent_frame.entry)
            .ok();
        let mut parent_restored = false;
        if let Some(path) = &parent_path {
            if let Some(parent_snap) = self.try_load_snapshot_from(path) {
                self.persist_snapshot(&parent_snap).await;
                parent_restored = true;
            }
        }
        if !parent_restored {
            warn!(
                "opendan.session[{}]: parent snapshot for `{}` missing on \
                 pop — next turn will rebuild fresh",
                self.session_id, parent_frame.entry
            );
            self.discard_snapshot();
        }

        // Inject a marker so the parent's next turn wakes up with something
        // resembling a user-side hand-off. Going through enqueue_pending
        // both persists it and fires the Wakeup signal.
        let marker = PendingInput::Msg {
            record_id: format!(
                "process-end:{}:{}",
                child_entry,
                uuid::Uuid::new_v4().simple()
            ),
            from: "system".to_string(),
            from_did: None,
            from_name: Some("system".to_string()),
            tunnel_did: None,
            text: format!("[independent process `{}` ended]", child_entry),
        };
        if let Err(err) = self.enqueue_pending(marker).await {
            warn!(
                "opendan.session[{}]: enqueue end-marker after pop failed: {err:#}",
                self.session_id
            );
        }
        Ok(NextAction::Idle)
    }

    /// **Fork primitive** (Phase 4 of llm_context_helper.rs design).
    ///
    /// Fork a sub-`LLMContext` from the parent's most recent on-disk
    /// snapshot (written by `TurnHook` before the current inference), apply
    /// `overrides`, run the sub-context to a terminal outcome, and return
    /// its `ContextOutput`. The parent session's `state.snap` and step
    /// history are **not** touched — fork is a non-resumable sync sub-task
    /// (per design doc §Fork).
    ///
    /// `sub_behavior_name` selects the behavior cfg used to build the
    /// sub-context's `LLMContextDeps` (parser/renderer, approval list,
    /// one_line_status sink). The sub-cfg's own system prompt is *not*
    /// auto-rendered into the sub-ctx — callers must populate
    /// `overrides.system_messages` themselves (otherwise the sub-ctx
    /// inherits parent's system segment verbatim, which is rarely what you
    /// want for an exploratory fork).
    ///
    /// Errors:
    /// - No parent snapshot on disk (must be invoked mid-turn, after at
    ///   least one TurnHook write)
    /// - Snapshot in suspended state (`pending_tool_calls` non-empty) —
    ///   `rebuild_with_inherit`'s pre-condition fails
    /// - Sub-context produces a suspended outcome (PendingTool
    ///   / ContextLimitReached) — fork has no resume path; this is mapped
    ///   to an error so the caller knows to abort
    ///
    /// In-memory `fork_stack` tracks the parent trace id per frame for
    /// diagnostics; not persisted (a mid-fork crash drops the sub-ctx and
    /// the parent recovers from its on-disk snapshot).
    pub async fn fork_and_run(
        &self,
        overrides: RequestOverrides,
        sub_behavior_name: &str,
    ) -> Result<ContextOutput> {
        let parent_snap = self.try_load_snapshot().ok_or_else(|| {
            anyhow!(
                "fork_and_run: session[{}] has no parent snapshot — fork must be invoked mid-turn",
                self.session_id
            )
        })?;
        let sub_cfg = self
            .agent_config
            .load_behavior(sub_behavior_name)
            .map_err(|err| {
                anyhow!("fork_and_run: load behavior `{sub_behavior_name}`: {err}")
            })?;

        let parent_trace = parent_snap
            .request
            .trace
            .clone()
            .unwrap_or_else(|| self.session_id.clone());
        let depth = {
            let mut stack = self.fork_stack.lock().await;
            stack.push(parent_trace.clone());
            stack.len()
        };
        let trace_id = format!("{}::fork-{}", parent_trace, depth);

        let run_result = self
            .run_fork_sub(parent_snap, overrides, &sub_cfg, &trace_id, depth)
            .await;

        // Pop regardless of success — fork frame lifetime ends here.
        self.fork_stack.lock().await.pop();
        run_result
    }

    /// Inner async helper for `fork_and_run`. Split out so the
    /// fork_stack pop in the public method always runs (Rust async
    /// destructors are best-effort; an explicit pop is clearer than
    /// stashing a guard).
    async fn run_fork_sub(
        &self,
        parent_snap: LLMContextSnapshot,
        overrides: RequestOverrides,
        sub_cfg: &BehaviorCfg,
        trace_id: &str,
        depth: usize,
    ) -> Result<ContextOutput> {
        use crate::llm_context_helper::rebuild_with_inherit;

        // Sub-ctx gets its own snapshot file so its TurnHook writes don't
        // clobber the parent's `state.snap`. The parent's on-disk state
        // therefore stays consistent throughout the fork, and a mid-fork
        // crash leaves the *sub-ctx's* most-recent state on disk under a
        // distinct name — independent of the parent recovery path.
        let fork_snap_path = self.state_snap_path.with_file_name(format!(
            "state.snap.fork-{depth}"
        ));

        let ctx_runtime = SessionRuntimeContext {
            trace_id: trace_id.to_string(),
            agent_name: self.agent_name.clone(),
            behavior: sub_cfg.name.clone(),
            step_idx: parent_snap.state.steps.len() as u32,
            wakeup_id: String::new(),
            session_id: self.session_id.clone(),
        };
        let deps = build_session_deps(
            &self.runtime,
            SessionDepsInput {
                tools: self.tools.clone(),
                ctx: ctx_runtime,
                snapshot_path: fork_snap_path.clone(),
                approval_required: sub_cfg.approval_required.clone(),
                one_line_status: Some(self.status.clone() as Arc<dyn OneLineStatusSink>),
                parser_renderer: sub_cfg.build_parser_and_renderer(),
            },
        );
        let mut ctx = rebuild_with_inherit(parent_snap, overrides, deps)
            .map_err(|e| anyhow!("fork_and_run: rebuild_with_inherit: {e}"))?;
        let outcome = ctx.run().await;

        // Best-effort cleanup of the sub-ctx's snapshot file. Leftover
        // files only matter if a future fork at the same depth races a
        // load against the stale content — and even then `rebuild_with_inherit`
        // takes the parent_snap parameter (not the disk), so the worst
        // case is harmless wasted bytes. We still tidy up on success.
        if fork_snap_path.exists() {
            if let Err(err) = std::fs::remove_file(&fork_snap_path) {
                warn!(
                    "opendan.session[{}]: fork snapshot cleanup at {} failed: {err}",
                    self.session_id,
                    fork_snap_path.display()
                );
            }
        }

        match outcome {
            LLMContextOutcome::Done { output, .. } => Ok(output),
            LLMContextOutcome::Error { error, .. } => {
                Err(anyhow!("fork sub-ctx errored: {error}"))
            }
            LLMContextOutcome::BudgetExhausted { which, .. } => {
                Err(anyhow!("fork sub-ctx budget exhausted: {:?}", which))
            }
            LLMContextOutcome::PendingTool { .. }
            | LLMContextOutcome::ContextLimitReached { .. } => Err(anyhow!(
                "fork sub-ctx unexpectedly suspended — fork sub-contexts must reach a terminal outcome (Done / Error / BudgetExhausted)"
            )),
        }
    }

    /// Current fork-call-stack depth. `0` ⇒ not inside any active fork.
    /// Async to share the same mutex as `fork_and_run`; intended for
    /// diagnostics / tests.
    pub async fn fork_depth(&self) -> usize {
        self.fork_stack.lock().await.len()
    }

    /// Read the "origin user message" stashed for the current turn — the
    /// most recent user-side `PendingInput::Msg` text the worker drained
    /// before running inference. Used by session-aware tools (`forward_msg`)
    /// so the LLM doesn't have to echo the message back as a tool argument.
    pub fn current_origin_user_message(&self) -> Option<String> {
        self.current_origin_msg
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .filter(|s| !s.trim().is_empty())
    }

    /// Worker-internal: stash / clear the per-turn origin message. Pass
    /// `Some(text)` right before running a turn; `None` to clear (e.g. on
    /// session exit).
    fn set_current_origin_msg(&self, value: Option<String>) {
        if let Ok(mut g) = self.current_origin_msg.lock() {
            *g = value;
        }
    }

    /// Lightweight snapshot of the session's externally-relevant fields,
    /// suitable for embedding into another LLM's prompt (e.g. a
    /// `try_create_worksession` sub-context choosing "reuse vs new"). Reads
    /// the in-memory `SessionMeta`, so it reflects the most recent
    /// status / one_line_status without touching disk.
    pub async fn summary(&self) -> SessionSummary {
        let meta = self.meta.lock().await;
        SessionSummary {
            session_id: meta.session_id.clone(),
            kind: meta.kind,
            title: meta.title.clone(),
            objective: meta.objective.clone(),
            status: meta.status,
            one_line_status: meta.one_line_status.clone(),
            workspace_id: meta.workspace_id.clone(),
            current_behavior: meta.current_behavior.clone(),
        }
    }

    async fn set_status(&self, status: SessionStatus) {
        {
            let mut g = self.meta.lock().await;
            g.status = status;
            g.one_line_status = self.status.snapshot();
        }
        if let Err(err) = self.flush_meta().await {
            warn!(
                "opendan.session[{}]: flush after status set failed: {err:#}",
                self.session_id
            );
        }
    }

    /// Stash the latest peer routing info (DID + tunnel) extracted from a
    /// `PendingInput::Msg` batch. Persisted via `flush_meta` so a restart
    /// still knows where to reply to.
    async fn update_peer(&self, peer_did: Option<String>, peer_tunnel: Option<String>) {
        let mut changed = false;
        {
            let mut meta = self.meta.lock().await;
            if let Some(did) = peer_did {
                if meta.peer_did.as_deref() != Some(did.as_str()) {
                    meta.peer_did = Some(did);
                    changed = true;
                }
            }
            if let Some(t) = peer_tunnel {
                if meta.peer_tunnel_did.as_deref() != Some(t.as_str()) {
                    meta.peer_tunnel_did = Some(t);
                    changed = true;
                }
            }
        }
        if changed {
            if let Err(err) = self.flush_meta().await {
                warn!(
                    "opendan.session[{}]: flush after peer update failed: {err:#}",
                    self.session_id
                );
            }
        }
    }

    /// Add `pattern` to the session's persistent kevent subscription list.
    /// No-op if the pattern is already subscribed. Returns `true` when the
    /// subscription set actually changed so the caller can refresh the
    /// agent-wide event pump.
    pub async fn subscribe_event(&self, pattern: impl Into<String>) -> Result<bool> {
        let pattern = pattern.into();
        let trimmed = pattern.trim();
        if trimmed.is_empty() {
            return Ok(false);
        }
        let now = now_ms();
        let mut changed = false;
        {
            let mut meta = self.meta.lock().await;
            if !meta
                .event_subscriptions
                .iter()
                .any(|s| s.pattern == trimmed)
            {
                meta.event_subscriptions.push(EventSubscription {
                    pattern: trimmed.to_string(),
                    subscribed_at_ms: now,
                });
                changed = true;
            }
        }
        if changed {
            self.flush_meta().await?;
            self.refresh_event_pump().await;
        }
        Ok(changed)
    }

    /// Remove `pattern` from the session's subscriptions. Returns `true`
    /// when something was actually removed.
    pub async fn unsubscribe_event(&self, pattern: &str) -> Result<bool> {
        let mut changed = false;
        {
            let mut meta = self.meta.lock().await;
            let before = meta.event_subscriptions.len();
            meta.event_subscriptions.retain(|s| s.pattern != pattern);
            if meta.event_subscriptions.len() != before {
                changed = true;
            }
        }
        if changed {
            self.flush_meta().await?;
            self.refresh_event_pump().await;
        }
        Ok(changed)
    }

    /// Push the session's current pattern list into the event pump so the
    /// agent-wide kevent reader sees additions / removals immediately. No-op
    /// when the runtime has no pump (CLI / tests).
    async fn refresh_event_pump(&self) {
        if let Some(pump) = self.event_pump.as_ref() {
            let patterns = self.subscription_patterns().await;
            pump.set_session_subscriptions(&self.session_id, patterns).await;
        }
    }

    /// Record the workspace this session is currently bound to. Returns
    /// `true` if the binding actually changed so the caller can drive the
    /// reciprocal update on the workspace record (set its
    /// `current_session`). Persisted via `flush_meta`.
    pub async fn set_workspace(&self, workspace_id: Option<String>) -> Result<bool> {
        let mut changed = false;
        {
            let mut meta = self.meta.lock().await;
            if meta.workspace_id != workspace_id {
                meta.workspace_id = workspace_id;
                changed = true;
            }
        }
        if changed {
            self.flush_meta().await?;
        }
        Ok(changed)
    }

    /// Snapshot the session's currently-bound workspace id, if any.
    pub async fn workspace_id(&self) -> Option<String> {
        self.meta.lock().await.workspace_id.clone()
    }

    /// Snapshot the session's current subscription patterns.
    pub async fn subscription_patterns(&self) -> Vec<String> {
        self.meta
            .lock()
            .await
            .event_subscriptions
            .iter()
            .map(|s| s.pattern.clone())
            .collect()
    }

    /// Compose + post an assistant reply through msg-center back to the
    /// session's last-known peer. Quietly skips when any prerequisite is
    /// missing (no msg-center bound, no peer DID, locally-injected session,
    /// unparseable agent DID). Errors are warn-logged; the local reply path
    /// is unaffected.
    async fn post_outbound_text(&self, text: &str) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        // UI sessions are the only ones that reply through msg-center
        // today — work sessions surface their result via report.md instead.
        if !matches!(self.kind, SessionKind::Ui) {
            return;
        }
        let Some(msg_center) = self.runtime.msg_center.as_ref().cloned() else {
            return;
        };
        let (peer_did_str, peer_tunnel_str) = {
            let meta = self.meta.lock().await;
            (meta.peer_did.clone(), meta.peer_tunnel_did.clone())
        };
        let Some(peer_did_str) = peer_did_str else {
            return;
        };
        let Ok(peer_did) = name_lib::DID::from_str(&peer_did_str) else {
            warn!(
                "opendan.session[{}]: outbound skipped — unparseable peer_did `{}`",
                self.session_id, peer_did_str
            );
            return;
        };
        let agent_did_raw = self.agent_config.toml.agent_did.trim();
        if agent_did_raw.is_empty() {
            warn!(
                "opendan.session[{}]: outbound skipped — agent.toml has no agent_did",
                self.session_id
            );
            return;
        }
        let Ok(agent_did) = name_lib::DID::from_str(agent_did_raw) else {
            warn!(
                "opendan.session[{}]: outbound skipped — agent_did `{}` is not parseable",
                self.session_id, agent_did_raw
            );
            return;
        };
        if agent_did == peer_did {
            // Don't echo back to ourselves — locally-injected sessions
            // sometimes set peer = owner = agent.
            return;
        }
        let tunnel = peer_tunnel_str
            .as_deref()
            .and_then(|raw| name_lib::DID::from_str(raw).ok());

        let mut msg = ndn_lib::MsgObject {
            from: agent_did.clone(),
            to: vec![peer_did.clone()],
            kind: ndn_lib::MsgObjKind::Chat,
            created_at_ms: now_ms(),
            content: ndn_lib::MsgContent {
                format: Some(ndn_lib::MsgContentFormat::TextPlain),
                content: trimmed.to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        msg.thread.topic = Some(self.session_id.clone());
        msg.thread.correlation_id = Some(self.session_id.clone());
        msg.meta.insert(
            "session_id".to_string(),
            serde_json::Value::String(self.session_id.clone()),
        );
        msg.meta.insert(
            "owner_session_id".to_string(),
            serde_json::Value::String(self.session_id.clone()),
        );

        let send_ctx = buckyos_api::SendContext {
            contact_mgr_owner: Some(agent_did),
            preferred_tunnel: tunnel,
            ..Default::default()
        };

        match msg_center.post_send(msg, Some(send_ctx), None).await {
            Ok(result) if result.ok => {}
            Ok(result) => warn!(
                "opendan.session[{}]: outbound rejected — reason={:?}",
                self.session_id, result.reason
            ),
            Err(err) => warn!(
                "opendan.session[{}]: outbound post_send failed: {err}",
                self.session_id
            ),
        }
    }
}

enum NextAction {
    Idle,
    WaitForMsg,
    /// Session yielded on async tool dispatch — the worker is parked until
    /// the matching task-completion events arrive in `pending_inputs`.
    WaitForTool,
    End,
}

enum BuiltContext {
    Fresh(LLMContext),
    Resumed(LLMContext),
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Build an `Observation` from a task_mgr kevent payload — returns `None`
/// when the event isn't terminal (the task is still running / progressing
/// and we should wait). Terminal kinds:
///   - `Completed` → `Observation::Success` with the task's `data` field
///     as `content` (falls back to the whole payload when `data` is absent)
///   - `Failed` → `Observation::Error` carrying `message`
///   - `Canceled` → `Observation::Cancelled` carrying the upstream reason
fn observation_from_task_event(call_id: &str, data: &serde_json::Value) -> Option<Observation> {
    let to_status = data.get("to_status").and_then(|v| v.as_str()).unwrap_or("");
    match to_status {
        "Completed" => {
            let content = data
                .get("data")
                .cloned()
                .unwrap_or_else(|| data.clone());
            let bytes = serde_json::to_vec(&content).map(|v| v.len()).unwrap_or(0);
            Some(Observation::Success {
                call_id: call_id.to_string(),
                content,
                bytes,
                truncated: false,
            })
        }
        "Failed" => {
            let message = data
                .get("message")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or_else(|| {
                    data.get("error_message")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or_else(|| "task Failed".to_string());
            Some(Observation::Error {
                call_id: call_id.to_string(),
                message,
            })
        }
        "Canceled" => {
            let reason = data
                .get("message")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or_else(|| {
                    data.get("error_message")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
                .unwrap_or_else(|| "task Canceled".to_string());
            Some(Observation::Cancelled {
                call_id: call_id.to_string(),
                reason,
            })
        }
        _ => None,
    }
}

/// Translate a subscribed kevent into a short note the LLM can react to as
/// part of the next turn. Keeps the JSON payload but tags it so the model
/// knows this came from the environment, not from a human.
fn format_event_for_turn(event_id: &str, data: &serde_json::Value) -> String {
    let body = if data.is_null() {
        String::new()
    } else if let Ok(rendered) = serde_json::to_string(data) {
        rendered
    } else {
        data.to_string()
    };
    if body.is_empty() {
        format!("[environment event] {event_id}")
    } else {
        format!("[environment event] {event_id} {body}")
    }
}

/// Cap on the size of the tail preserved when compressing `accumulated` on
/// `ContextLimitReached`. Picked empirically — small enough to slash the
/// window dramatically (so a near-limit history reliably fits afterward)
/// while keeping enough recent exchange that the LLM doesn't lose the
/// thread.
const COMPRESS_KEEP_TAIL: usize = 12;

/// Heuristic message-level compressor used by `run_one_turn` when the waist
/// emits `Outcome::ContextLimitReached`. Strategy:
///   1. Keep the leading run of `System` messages verbatim (identity /
///      role / objective text — never drop these).
///   2. Drop the middle of the conversation, keeping the last
///      [`COMPRESS_KEEP_TAIL`] non-system messages.
///   3. Insert a single synthetic `User` message between the System block
///      and the tail describing what was dropped, so the LLM sees an
///      explicit gap rather than wondering why history seems to skip.
///
/// Best-effort on role alternation: if the tail starts with an
/// `Assistant` message, we drop it so the synthetic `User` slots in
/// cleanly. Providers vary in their strictness; this keeps the common
/// case (tail starts with `User`) clean and the edge case from emitting
/// two `Assistant` messages in a row.
///
/// Note: this is an opendan-level compressor (message dimension), distinct
/// from the optional `HistoryCompressor` inside the Behavior Loop (step
/// dimension). They can coexist.
pub fn compress_messages_for_context_limit(accumulated: Vec<AiMessage>) -> Vec<AiMessage> {
    let leading_system = accumulated
        .iter()
        .position(|m| m.role != AiRole::System)
        .unwrap_or(accumulated.len());
    let total = accumulated.len();
    let rest_len = total - leading_system;
    if rest_len <= COMPRESS_KEEP_TAIL {
        // Nothing to drop — the body already fits the budget. Returning
        // the input verbatim is still useful: the `ResumeFill::RewrittenHistory`
        // path re-establishes `state.accumulated` from this vec.
        return accumulated;
    }
    let dropped = rest_len - COMPRESS_KEEP_TAIL;
    let mut out: Vec<AiMessage> = accumulated.iter().take(leading_system).cloned().collect();
    out.push(AiMessage::text(
        AiRole::User,
        format!(
            "[context compressed: {} earlier message{} dropped to fit the model context window; resume from the recent tail below]",
            dropped,
            if dropped == 1 { "" } else { "s" }
        ),
    ));
    // Realign tail so it doesn't open with an Assistant message right after
    // our synthetic User (would make the LLM see User→Assistant→Assistant→...).
    let mut tail_start = leading_system + dropped;
    while tail_start < total && matches!(accumulated[tail_start].role, AiRole::Assistant) {
        tail_start += 1;
    }
    out.extend(accumulated.into_iter().skip(tail_start));
    out
}

fn compose_human_text(texts: &[String]) -> Option<String> {
    let joined: Vec<&str> = texts
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if joined.is_empty() {
        None
    } else {
        Some(joined.join("\n\n"))
    }
}

/// Build the user-message body fed into the next inference from the
/// environment-aware preamble and the actual human/event text.
///
/// Rules:
/// - Both present → `{env}\n\n{human}` (env first so the LLM reads it before
///   the user input that drives the turn).
/// - Only one present → return it verbatim.
/// - Both empty → `None` (caller will fall through to `ResumeFromMidRun` or
///   omit the user message entirely on fresh build).
fn merge_env_and_human(env: Option<String>, human: Option<String>) -> Option<String> {
    match (env, human) {
        (Some(e), Some(h)) => Some(format!("{e}\n\n{h}")),
        (Some(e), None) => Some(e),
        (None, Some(h)) => Some(h),
        (None, None) => None,
    }
}

fn output_to_text(output: &ContextOutput) -> Option<String> {
    match output {
        ContextOutput::Text { content } => {
            if content.is_empty() {
                None
            } else {
                Some(content.clone())
            }
        }
        ContextOutput::Json { content } => Some(content.to_string()),
    }
}

#[allow(dead_code)]
fn message_first_text(m: &AiMessage) -> Option<&str> {
    m.content.iter().find_map(|b| match b {
        AiContent::Text { text } => Some(text.as_str()),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_human_text_skips_empties() {
        let v = vec!["  ".to_string(), "hello".to_string(), "".to_string()];
        assert_eq!(compose_human_text(&v).as_deref(), Some("hello"));
    }

    #[test]
    fn compose_human_text_joins() {
        let v = vec!["a".to_string(), "b".to_string()];
        assert_eq!(compose_human_text(&v).as_deref(), Some("a\n\nb"));
    }

    #[test]
    fn output_text_extraction() {
        let out = ContextOutput::Text {
            content: "hi".to_string(),
        };
        assert_eq!(output_to_text(&out).as_deref(), Some("hi"));
        let out = ContextOutput::Text {
            content: String::new(),
        };
        assert!(output_to_text(&out).is_none());
    }

    #[test]
    fn pending_input_dedup_key_distinguishes_variants() {
        let msg = PendingInput::Msg {
            record_id: "abc".to_string(),
            from: "alice".to_string(),
            from_did: None,
            from_name: None,
            tunnel_did: None,
            text: "hi".to_string(),
        };
        let event = PendingInput::Event {
            event_id: "abc".to_string(),
            data: serde_json::Value::Null,
        };
        assert_eq!(msg.dedup_key(), "msg:abc");
        assert_eq!(event.dedup_key(), "event:abc");
        assert_ne!(msg.dedup_key(), event.dedup_key());
    }

    #[test]
    fn format_event_for_turn_includes_id_and_data() {
        let s = format_event_for_turn("/timer/wake", &serde_json::json!({"tick": 1}));
        assert!(s.contains("/timer/wake"));
        assert!(s.contains("tick"));
    }

    #[test]
    fn format_event_for_turn_handles_null_payload() {
        let s = format_event_for_turn("/timer/wake", &serde_json::Value::Null);
        assert!(s.contains("/timer/wake"));
        assert!(!s.contains("null"));
    }

    #[test]
    fn session_meta_round_trips_pending_inputs() {
        // SessionMeta + PendingInput must round-trip through JSON so
        // `.meta/session.json` correctly preserves unconsumed inputs across
        // process restarts. If this breaks, persisted pendings are lost.
        let meta = SessionMeta {
            session_id: "s1".to_string(),
            kind: SessionKind::Ui,
            current_behavior: "ui_default".to_string(),
            status: SessionStatus::WaitingInput,
            owner: "alice".to_string(),
            one_line_status: String::new(),
            pending_inputs: vec![
                PendingInput::Msg {
                    record_id: "rec-1".to_string(),
                    from: "alice".to_string(),
                    from_did: Some("did:dev:alice".to_string()),
                    from_name: Some("Alice".to_string()),
                    tunnel_did: Some("did:dev:tunnel".to_string()),
                    text: "hi".to_string(),
                },
                PendingInput::Event {
                    event_id: "/timer/wake".to_string(),
                    data: serde_json::json!({"tick": 7}),
                },
            ],
            peer_did: Some("did:dev:alice".to_string()),
            peer_tunnel_did: Some("did:dev:tunnel".to_string()),
            event_subscriptions: vec![EventSubscription {
                pattern: "/timer/**".to_string(),
                subscribed_at_ms: 0,
            }],
            workspace_id: Some("ws-1".to_string()),
            pending_task_calls: vec![PendingTaskCall {
                call_id: "call-1".to_string(),
                tool_name: "download".to_string(),
                task_id: 42,
                event_pattern: "/task_mgr/42".to_string(),
            }],
            title: "design review".to_string(),
            objective: "draft the rollout plan".to_string(),
            bootstrap_done: true,
            process_entry: "planner".to_string(),
            process_stack: vec![ProcessFrame {
                entry: "ui_default".to_string(),
                current: "ui_default".to_string(),
            }],
        };
        let json = serde_json::to_string(&meta).unwrap();
        let restored: SessionMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.pending_inputs.len(), 2);
        match &restored.pending_inputs[0] {
            PendingInput::Msg {
                record_id,
                text,
                from_did,
                from_name,
                tunnel_did,
                ..
            } => {
                assert_eq!(record_id, "rec-1");
                assert_eq!(text, "hi");
                assert_eq!(from_did.as_deref(), Some("did:dev:alice"));
                assert_eq!(from_name.as_deref(), Some("Alice"));
                assert_eq!(tunnel_did.as_deref(), Some("did:dev:tunnel"));
            }
            _ => panic!("expected Msg variant first"),
        }
        match &restored.pending_inputs[1] {
            PendingInput::Event { event_id, data } => {
                assert_eq!(event_id, "/timer/wake");
                assert_eq!(data.get("tick").and_then(|v| v.as_i64()), Some(7));
            }
            _ => panic!("expected Event variant second"),
        }
        assert_eq!(restored.peer_did.as_deref(), Some("did:dev:alice"));
        assert_eq!(restored.event_subscriptions.len(), 1);
        assert_eq!(restored.event_subscriptions[0].pattern, "/timer/**");
        assert_eq!(restored.workspace_id.as_deref(), Some("ws-1"));
        assert_eq!(restored.pending_task_calls.len(), 1);
        assert_eq!(restored.pending_task_calls[0].task_id, 42);
        assert_eq!(restored.pending_task_calls[0].call_id, "call-1");
        assert_eq!(restored.title, "design review");
        assert_eq!(restored.objective, "draft the rollout plan");
        assert!(restored.bootstrap_done);
        assert_eq!(restored.process_entry, "planner");
        assert_eq!(restored.process_stack.len(), 1);
        assert_eq!(restored.process_stack[0].entry, "ui_default");
        assert_eq!(restored.process_stack[0].current, "ui_default");
    }

    #[test]
    fn session_meta_backfills_process_entry_for_legacy_json() {
        // Older `.meta/session.json` files predate the
        // `process_entry` / `process_stack` fields. They must still
        // deserialize (serde defaults) and `AgentSession::new`'s restore
        // path backfills `process_entry` from `current_behavior` so the
        // independent-mode snapshot path is well-formed.
        let legacy = serde_json::json!({
            "session_id": "s2",
            "kind": "ui",
            "current_behavior": "ui_default",
            "status": "idle",
        });
        let restored: SessionMeta = serde_json::from_value(legacy).unwrap();
        assert_eq!(restored.process_entry, "");
        assert!(restored.process_stack.is_empty());
        // (The backfill itself lives in AgentSession::new and is exercised
        // by the restore-path integration tests; here we only assert that
        // the legacy JSON does NOT fail to deserialize.)
    }

    #[test]
    fn observation_from_task_event_translates_completed() {
        let payload = serde_json::json!({
            "to_status": "Completed",
            "data": {"result": "ok"},
        });
        let obs = observation_from_task_event("call-9", &payload).expect("terminal observation");
        match obs {
            Observation::Success {
                call_id, content, ..
            } => {
                assert_eq!(call_id, "call-9");
                assert_eq!(content.get("result").and_then(|v| v.as_str()), Some("ok"));
            }
            _ => panic!("expected Success"),
        }
    }

    #[test]
    fn observation_from_task_event_translates_failed() {
        let payload = serde_json::json!({
            "to_status": "Failed",
            "message": "network unreachable",
        });
        let obs = observation_from_task_event("call-9", &payload).expect("terminal observation");
        match obs {
            Observation::Error { call_id, message } => {
                assert_eq!(call_id, "call-9");
                assert!(message.contains("network"));
            }
            _ => panic!("expected Error"),
        }
    }

    #[test]
    fn observation_from_task_event_ignores_non_terminal_status() {
        // Running / Progress events shouldn't move the session — they emit
        // frequently and the session must wait for the terminal one.
        let payload = serde_json::json!({"to_status": "Running"});
        assert!(observation_from_task_event("c", &payload).is_none());
    }

    #[test]
    fn compress_messages_preserves_short_history_verbatim() {
        // Under the keep-tail threshold ⇒ no compression, output == input.
        let msgs = vec![
            AiMessage::text(AiRole::System, "sys"),
            AiMessage::text(AiRole::User, "u1"),
            AiMessage::text(AiRole::Assistant, "a1"),
        ];
        let out = compress_messages_for_context_limit(msgs.clone());
        assert_eq!(out.len(), msgs.len());
        assert_eq!(out[0].role, AiRole::System);
    }

    #[test]
    fn compress_messages_drops_middle_and_keeps_tail() {
        let mut msgs = vec![AiMessage::text(AiRole::System, "sys")];
        // Generate alternating user/assistant pairs well beyond the tail cap.
        for i in 0..(COMPRESS_KEEP_TAIL + 20) {
            let role = if i % 2 == 0 { AiRole::User } else { AiRole::Assistant };
            msgs.push(AiMessage::text(role, format!("m-{i}")));
        }
        let out = compress_messages_for_context_limit(msgs);
        assert_eq!(out[0].role, AiRole::System);
        // Second message is the synthetic compression note.
        assert_eq!(out[1].role, AiRole::User);
        let note = out[1]
            .content
            .iter()
            .find_map(|b| match b {
                AiContent::Text { text } => Some(text.clone()),
                _ => None,
            })
            .unwrap_or_default();
        assert!(note.contains("context compressed"));
        assert!(note.contains("earlier"));
        // Tail length is at most the keep cap (may be one less when we
        // realign past a leading Assistant).
        let tail_len = out.len() - 2;
        assert!(tail_len <= COMPRESS_KEEP_TAIL);
        assert!(tail_len >= COMPRESS_KEEP_TAIL - 1);
        // No two assistant messages in a row (our realignment guarantee).
        for w in out.windows(2) {
            assert!(
                !(w[0].role == AiRole::Assistant && w[1].role == AiRole::Assistant),
                "compress must not produce back-to-back assistant messages"
            );
        }
    }

    #[test]
    fn merge_env_and_human_combines_both_with_env_first() {
        let m = merge_env_and_human(Some("E".into()), Some("H".into()));
        assert_eq!(m.as_deref(), Some("E\n\nH"));
    }

    #[test]
    fn merge_env_and_human_handles_missing_pieces() {
        assert_eq!(
            merge_env_and_human(None, Some("h".into())).as_deref(),
            Some("h")
        );
        assert_eq!(
            merge_env_and_human(Some("e".into()), None).as_deref(),
            Some("e")
        );
        assert!(merge_env_and_human(None, None).is_none());
    }

    #[test]
    fn session_meta_tolerates_missing_pending_inputs_field() {
        // Older session.json files were written before pending_inputs
        // existed; restoring them must default the field to an empty
        // vec rather than erroring out.
        let legacy = r#"{
            "session_id": "old",
            "kind": "ui",
            "current_behavior": "ui_default",
            "status": "idle",
            "owner": "alice"
        }"#;
        let meta: SessionMeta = serde_json::from_str(legacy).unwrap();
        assert!(meta.pending_inputs.is_empty());
        assert_eq!(meta.owner, "alice");
    }
}
