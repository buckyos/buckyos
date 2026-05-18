use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use buckyos_api::{match_event_patterns, AiContent, AiMessage, AiRole};
use log::{info, warn};
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;

use agent_tool::{AgentToolManager, SessionRuntimeContext};
use llm_context::{
    context_loop::LLMContext,
    interrupt::LLMContextInterruptHandle,
    observation::Observation,
    outcome::{ContextOutput, LLMContextOutcome, ResumeFill},
    request::{ContextOwnerRef, LLMContextRequest},
    state::{LLMContextSnapshot, LLMContextState},
};

use crate::agent_config::{AgentConfig, LoopMode, SwitchMode};
use crate::ai_runtime::{build_session_deps, AgentRuntime, OneLineStatusSink, SessionDepsInput};
use crate::behavior_cfg::BehaviorCfg;
use crate::behavior_hooks::{self, CtxLimitOutcome, InterruptOutcome, ProviderFailedOutcome};
use crate::llm_context_helper::{
    apply_overrides_to_snapshot, run_fork_sub_context, ForkSubContextInput, RequestOverrides,
};
use crate::prompt_env::{self, AgentSessionEnv, ENVIRONMENT_BLOCK_TEMPLATE};
use crate::round_history::{
    CompactionTarget, ContextMode, HistoryEvent, InterruptMode as HistoryInterruptMode,
    RoundStatus, RoundTrigger, SessionHistoryRecorder,
};
use crate::session_event_pump::SessionEventPump;
pub use crate::session_model::{
    EventSubscription, InterruptMode, PendingInput, PendingTaskCall, ProcessFrame, SessionInput,
    SessionKind, SessionMeta, SessionStatus, SessionSummary,
};
use crate::task_dispatch::TaskDispatch;

/// Sentinel emitted by a behavior parser in
/// `LLMBehaviorResult.next_behavior` to mean "current intent ran its course,
/// no autonomous next step — park the session until the next inbound user
/// message". Interpreted only at the session layer; the waist treats it as
/// an opaque jump-target string.
pub const NEXT_BEHAVIOR_WAIT_USER_MSG: &str = "WAIT_USER_MSG";
const MAX_PENDING_INPUTS: usize = 256;

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
    fork_stack: Arc<std::sync::Mutex<Vec<String>>>,

    /// Last user-text that triggered the current (or most recent) inference
    /// round. Stashed by the worker right before `run_one_round` so
    /// session-aware tools can pick it up without having to be told —
    /// `forward_msg` reads this to default its body to "the message that
    /// caused the parent LLM to think a forward was needed". §8.4 of the
    /// design doc calls this the "本轮 origin user 消息". Per-turn ephemeral
    /// state — not persisted, simply overwritten each turn.
    current_origin_msg: Arc<std::sync::Mutex<Option<String>>>,

    /// Interrupt handle of the LLMContext currently inside a `run()` call.
    /// `Some` while an inference is in flight; `None` between turns or when
    /// the session is parked. `AgentSession::interrupt(Discard)` reads this
    /// to preempt the inference via the waist's §3.13 control plane —
    /// without it, the worker can only act on interrupts after the LLM has
    /// already finished generating, defeating the point of "force" mode.
    current_interrupt_handle: Arc<std::sync::Mutex<Option<LLMContextInterruptHandle>>>,

    /// Append-only round-history writer. Lazy-initialised on first use so the
    /// synchronous `new()` doesn't have to touch disk. Failures to open or
    /// write are warn-logged but never propagate — history is best-effort
    /// auxiliary state; an I/O issue here must not block the worker.
    history: Arc<SessionHistoryRecorder>,
}

/// Per-round history seed handed from the worker drain step into
/// [`AgentSession::run_one_round`]. Carries the metadata the writer needs to
/// open a fresh round plus the raw user / system-event payloads to seed as
/// the first entries of that round. `None` means "do not open a new round
/// — append against whichever round is already open" (used by the
/// PendingTool resume path).
struct RoundSeed {
    trigger: RoundTrigger,
    input_keys: Vec<String>,
    user_messages: Vec<AiMessage>,
    /// `(source, payload)` pairs for non-task events that landed in this
    /// drain. Each becomes a `HistoryEvent::SystemInput` entry.
    system_events: Vec<(String, serde_json::Value)>,
}

#[derive(Debug, Clone)]
struct EventForTurn {
    event_id: String,
    data: serde_json::Value,
    message: String,
}

/// RAII handle slot — installs `LLMContextInterruptHandle` into a session's
/// `current_interrupt_handle` for the lifetime of the guard. Dropping it
/// (normal return, early return, panic during run) clears the slot so a
/// later `interrupt(Discard)` doesn't fire on a stale handle.
struct InterruptHandleGuard {
    slot: Arc<std::sync::Mutex<Option<LLMContextInterruptHandle>>>,
}

impl Drop for InterruptHandleGuard {
    fn drop(&mut self) {
        if let Ok(mut g) = self.slot.lock() {
            *g = None;
        }
    }
}

struct ForkStackGuard {
    stack: Arc<std::sync::Mutex<Vec<String>>>,
}

impl Drop for ForkStackGuard {
    fn drop(&mut self) {
        if let Ok(mut stack) = self.stack.lock() {
            stack.pop();
        }
    }
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
        let history = Arc::new(SessionHistoryRecorder::new(
            b.session_id.clone(),
            session_dir.clone(),
        ));
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
            fork_stack: Arc::new(std::sync::Mutex::new(Vec::new())),
            current_origin_msg: Arc::new(std::sync::Mutex::new(None)),
            current_interrupt_handle: Arc::new(std::sync::Mutex::new(None)),
            history,
        };
        (session, inbox_rx)
    }

    /// Install `handle` as the session's "currently in-flight" interrupt
    /// handle. The returned guard clears the slot on drop. Callers must hold
    /// the guard for the entire scope of the `ctx.run().await` it pairs with.
    fn register_interrupt_handle(&self, handle: LLMContextInterruptHandle) -> InterruptHandleGuard {
        if let Ok(mut g) = self.current_interrupt_handle.lock() {
            *g = Some(handle);
        }
        InterruptHandleGuard {
            slot: Arc::clone(&self.current_interrupt_handle),
        }
    }

    /// Snapshot the currently-installed handle (if any). Returns `None` when
    /// no inference is in flight (between turns, parked on PendingTool,
    /// session idle).
    fn snapshot_interrupt_handle(&self) -> Option<LLMContextInterruptHandle> {
        self.current_interrupt_handle
            .lock()
            .ok()
            .and_then(|g| g.clone())
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
        let bytes = serde_json::to_vec_pretty(&meta)
            .map_err(|err| anyhow!("session[{}]: serialize meta failed: {err}", self.session_id))?;
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
    /// Duplicates (same `dedup_key`) are collapsed — replayed messages and
    /// interrupts are dropped, while events replace the older snapshot when
    /// they are equally or more final. Callers should treat `Ok(())` as
    /// "you may now ack regardless of whether the item was newly accepted,
    /// deduplicated, or coalesced".
    pub async fn enqueue_pending(&self, input: PendingInput) -> Result<()> {
        let key = input.dedup_key();
        let mut changed = false;
        {
            let mut meta = self.meta.lock().await;
            if let PendingInput::Event { .. } = &input {
                if let Some(existing) = meta
                    .pending_inputs
                    .iter_mut()
                    .find(|i| i.dedup_key() == key)
                {
                    if should_replace_pending_event(existing, &input) {
                        *existing = input;
                        changed = true;
                    }
                } else {
                    meta.pending_inputs.push(input);
                    changed = true;
                }
            } else {
                let already = meta.pending_inputs.iter().any(|i| i.dedup_key() == key);
                if !already {
                    meta.pending_inputs.push(input);
                    changed = true;
                }
            }
            if changed {
                let dropped = enforce_pending_queue_limit(
                    &mut meta.pending_inputs,
                    MAX_PENDING_INPUTS,
                    &self.agent_name,
                );
                if dropped > 0 {
                    warn!(
                        "opendan.session[{}]: pending queue exceeded {}; dropped {dropped} older unprotected item(s)",
                        self.session_id, MAX_PENDING_INPUTS
                    );
                }
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
    /// `Graceful` is a no-op when the session has no outstanding pending
    /// tool calls at the moment the worker processes it (the session is
    /// already at an outcome boundary; there is nothing to wind down).
    ///
    /// `Discard` is the **force** mode: if a `LLMContext::run()` is currently
    /// in flight, this call additionally fires the waist's §3.13 interrupt
    /// handle so the provider inference is preempted right now rather than
    /// allowed to run to completion. The queued `PendingInput::Interrupt`
    /// barrier still rides through the worker so any post-run cleanup
    /// (trim the trailing assistant turn that owned unresolved tool_use
    /// blocks, drop pending tool calls) runs uniformly with the
    /// "interrupt while parked on PendingTool" case.
    pub async fn interrupt(&self, mode: InterruptMode) -> Result<()> {
        // Force mode: preempt the in-flight inference immediately. When no
        // run is in flight, `snapshot_interrupt_handle` returns None and we
        // just fall through to the existing enqueue path.
        if matches!(mode, InterruptMode::Discard) {
            if let Some(handle) = self.snapshot_interrupt_handle() {
                let reason = format!("agent_session[{}].interrupt(Discard)", self.session_id);
                let first = handle.interrupt(reason);
                if first {
                    info!(
                        "opendan.session[{}]: interrupt(Discard) preempted in-flight inference",
                        self.session_id
                    );
                }
            }
        }

        let id = format!(
            "{}-{}",
            now_ms(),
            self.trace_seq
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        );
        self.enqueue_pending(PendingInput::Interrupt { mode, id })
            .await
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
            text: text.clone(),
            ai_message: AiMessage::text(AiRole::User, text.trim().to_string()),
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
            // succeeds, so a crash mid-round leaves the
            // inputs durable and they'll be replayed next boot.
            let mut pending = self.meta.lock().await.pending_inputs.clone();
            if pending.is_empty() {
                // Work session bootstrap: if a freshly-created Work session
                // has nothing pending and hasn't run yet, drive an initial
                // turn from its `objective` (per §8.1 step 4 of the design).
                // After the first successful turn this branch falls through
                // to the normal recv()-blocking path.
                let needs_bootstrap =
                    matches!(self.kind, SessionKind::Work) && self.needs_bootstrap_turn().await;
                if needs_bootstrap {
                    self.set_status(SessionStatus::Running).await;
                    let seed = RoundSeed {
                        trigger: RoundTrigger::SystemEvent {
                            source: "bootstrap".to_string(),
                            event_kind: "objective".to_string(),
                        },
                        input_keys: Vec::new(),
                        user_messages: Vec::new(),
                        system_events: Vec::new(),
                    };
                    let round_result = self.run_one_round(Vec::new(), Some(seed)).await;
                    self.mark_bootstrap_done().await;
                    match round_result {
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
                        PendingInput::Interrupt { mode, .. } => (*mode, pending[pos].dedup_key()),
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
            //   - Msg / generic Event → fold into the next round as `round_inputs`
            //   - Event whose id matches a `pending_task_calls` pattern →
            //     translates into an `Observation`, used to build a
            //     `ResumeFill::ToolResults` once every pending call has a
            //     result.
            // Latest peer info wins — the most recent Msg in this batch
            // dictates where outbound replies will be routed.
            let mut turn_messages: Vec<AiMessage> = Vec::new();
            let mut turn_events = Vec::new();
            let mut consumed_keys = Vec::new();
            let mut task_completions: Vec<(String, Observation, String, String)> = Vec::new();
            let mut latest_peer_did: Option<String> = None;
            let mut latest_peer_tunnel: Option<String> = None;
            let mut latest_origin_msg: Option<String> = None;
            // Parallel collections destined for the round-history seed. We
            // mirror the per-input visit so user-msg ordering & system-event
            // payloads are captured intact rather than the post-formatted
            // string the LLM sees.
            let mut hist_user_messages: Vec<AiMessage> = Vec::new();
            let mut hist_system_events: Vec<(String, serde_json::Value)> = Vec::new();
            let mut msg_count: u32 = 0;
            let mut event_count: u32 = 0;
            let mut first_msg_preview: Option<String> = None;
            let mut first_event_meta: Option<(String, String)> = None;
            let pending_task_index = self.pending_task_index().await;
            for input in &pending {
                match input {
                    PendingInput::Msg {
                        text,
                        from_did,
                        tunnel_did,
                        ai_message,
                        ..
                    } => {
                        let message = pending_msg_ai_message(ai_message);
                        if ai_message_has_payload(&message) {
                            let preview_text = pending_msg_preview(text, &message);
                            if first_msg_preview.is_none() {
                                first_msg_preview = Some(trigger_preview(&preview_text));
                            }
                            if !preview_text.trim().is_empty() {
                                latest_origin_msg = Some(preview_text);
                            }
                            turn_messages.push(message.clone());
                            hist_user_messages.push(message);
                            msg_count += 1;
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
                        // the turn so the LLM can react. Rendering happens
                        // through the matching subscription when it supplied
                        // a natural-language template.
                        turn_events.push(EventForTurn {
                            event_id: event_id.clone(),
                            data: data.clone(),
                            message: self.format_event_for_turn(event_id, data).await,
                        });
                        hist_system_events.push((event_id.clone(), data.clone()));
                        if first_event_meta.is_none() {
                            first_event_meta =
                                Some((event_id.clone(), trigger_event_kind(event_id)));
                        }
                        event_count += 1;
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
                    let resume_result = self.resume_with_tool_results(&task_completions).await;
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
                            // and `run_one_round` handles them normally.
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

            let mut round_inputs = turn_messages;
            if let Some(batch) = format_event_batch_for_turn(&turn_events) {
                round_inputs.push(AiMessage::text(AiRole::User, batch));
            }

            // If the snapshot is currently mid-PendingTool and the upper
            // layer queued bare Msg/Event entries without an Interrupt
            // barrier, defer: starting a fresh turn here would discard
            // the in-flight tool round. Upper layers that want immediate
            // attention should `interrupt()` first, then `enqueue_pending`.
            if !round_inputs.is_empty() && self.snapshot_has_pending_tool_calls().await {
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

            if round_inputs.is_empty() {
                self.discard_consumed(&consumed_keys).await;
                continue;
            }

            // Stash the most recent human-text as the turn's "origin user
            // message" so session-aware tools (forward_msg) can pick it up
            // without the LLM having to pass it through tool args (§8.4).
            // Events have no origin-user semantics — they only update the
            // stash when they happen to come bundled with chat text.
            self.set_current_origin_msg(latest_origin_msg);

            self.set_status(SessionStatus::Running).await;
            let trigger = match (msg_count, event_count) {
                (0, 0) => RoundTrigger::Resume,
                (n, 0) if n > 0 => RoundTrigger::UserMsg {
                    preview: first_msg_preview.clone().unwrap_or_default(),
                },
                (0, _) => {
                    let (source, kind) = first_event_meta
                        .clone()
                        .unwrap_or_else(|| ("event".to_string(), "unknown".to_string()));
                    RoundTrigger::SystemEvent {
                        source,
                        event_kind: kind,
                    }
                }
                _ => RoundTrigger::Mixed,
            };
            let seed = RoundSeed {
                trigger,
                input_keys: consumed_keys.clone(),
                user_messages: hist_user_messages,
                system_events: hist_system_events,
            };
            let round_result = self.run_one_round(round_inputs, Some(seed)).await;
            match round_result {
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
                    return Err(anyhow!("missing observation for call_id `{call_id}`"));
                }
            }
        }
        let fill = ResumeFill::ToolResults { results: ordered };
        let behavior = self.load_current_behavior().await?;
        let mode = self.history_mode_for(&behavior);
        // Ensure a round is open — the writer auto-reopens a `WaitingTool`
        // round on startup; this is a safety net for the rare case where the
        // round was finalised on the prior process (e.g. crash + restart with
        // a stale `state.snap`).
        if self.history.current_round().await.is_none() {
            self.history
                .begin_round(
                    RoundTrigger::Resume,
                    completions.iter().map(|(_, _, _, k)| k.clone()).collect(),
                    mode,
                )
                .await;
        }
        let trace_id = self.next_trace_id();
        let ctx_runtime = SessionRuntimeContext {
            trace_id: trace_id.clone(),
            agent_name: self.agent_name.clone(),
            behavior: behavior.meta.name.clone(),
            step_idx: snapshot.state.steps.len() as u32,
            wakeup_id: String::new(),
            session_id: self.session_id.clone(),
        };
        let from_user_did = self.current_from_user_did().await;
        let deps = build_session_deps(
            &self.runtime,
            SessionDepsInput {
                tools: self.tools.clone(),
                ctx: ctx_runtime,
                snapshot_path: self.state_snap_path.clone(),
                approval_required: behavior.capabilities.approval_required.clone(),
                one_line_status: Some(self.status.clone() as Arc<dyn OneLineStatusSink>),
                parser_renderer: behavior.build_parser_and_renderer(self.session_class_loop_mode()),
                from_user_did,
            },
        );
        let mut ctx =
            LLMContext::resume(snapshot, fill, deps).map_err(|e| anyhow!("resume: {e}"))?;
        // Capture the post-resume baseline before the next inference so the
        // diff records exactly what the post-tool-result run produces. The
        // `ResumeFill::ToolResults` injection has already extended
        // `accumulated`/`steps` at this point — those tool-result rows are
        // therefore part of the baseline and will not be double-written.
        let pre = ctx.snapshot();
        let baseline_accumulated_len = pre.state.accumulated.len();
        let baseline_steps_len = pre.state.steps.len();
        let baseline_last_step_text = pre
            .state
            .last_step
            .as_ref()
            .map(|s| s.assistant_text.clone());
        let llm_call = self
            .trace_seq
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // The post-tool-results inference is a regular ReAct step — keep it
        // preemptable by `AgentSession::interrupt(Discard)` (§3.13).
        let _interrupt_guard = self.register_interrupt_handle(ctx.interrupt_handle());
        let outcome = ctx.run().await;
        drop(_interrupt_guard);
        // Post-run snapshot — needed by Done+next_behavior switching to
        // preserve full history (final assistant reply included). Outcome::Done
        // itself carries no snapshot, but ctx is still alive here.
        let final_snapshot = ctx.snapshot();

        self.history
            .record_run_diff(
                mode,
                baseline_accumulated_len,
                baseline_steps_len,
                baseline_last_step_text,
                &final_snapshot,
                &outcome,
                llm_call,
            )
            .await;
        self.history.append_outcome(&outcome).await;
        if let Some(status) = SessionHistoryRecorder::round_status_for(&outcome) {
            self.history.finalize_round(status).await;
        }

        // Clear pending_task_calls + unsubscribe from /task_mgr/* patterns.
        // Done before handling the outcome so a subsequent PendingTool emit
        // (chained tool calls) starts from a clean slate.
        let patterns: Vec<String> = completions.iter().map(|(_, _, p, _)| p.clone()).collect();
        self.clear_pending_task_calls().await;
        for pattern in patterns {
            let _ = self.unsubscribe_event(&pattern).await;
        }

        self.handle_outcome(outcome, &behavior, final_snapshot)
            .await
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

        // Record the user-visible interrupt against the open round (if any)
        // before we start the wind-down work. `finalize_round(Interrupted)`
        // lands at the end of either branch below.
        let history_mode = match mode {
            InterruptMode::Graceful => HistoryInterruptMode::Graceful,
            InterruptMode::Discard => HistoryInterruptMode::Discard,
        };
        self.history
            .append_event(HistoryEvent::Interrupt {
                mode: history_mode,
                reason: None,
            })
            .await;

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

        // Behavior `[on_interrupt_graceful]` / `[on_interrupt_discard]`
        // hooks: peek at the current behavior to decide whether to honor
        // the wind-down (the default) or short-circuit to a different
        // policy. v0 modes intentionally mirror the historical behavior
        // — see [`behavior_hooks::resolve_interrupt_graceful`] /
        // [`behavior_hooks::resolve_interrupt_discard`].
        let behavior_for_hook = self.load_current_behavior().await.ok();
        match mode {
            InterruptMode::Graceful => {
                let outcome = behavior_for_hook
                    .as_ref()
                    .and_then(|b| {
                        behavior_hooks::resolve_interrupt_graceful(b.on_interrupt_graceful.as_ref())
                            .ok()
                    })
                    .unwrap_or(InterruptOutcome::Default);
                // v0 has only one mode here; both Default and the explicit
                // opt-in walk the same wind-down path. Future modes can
                // branch off without restructuring the surrounding code.
                let _ = outcome;
                self.execute_interrupt_graceful(snapshot, &pending_calls, reason)
                    .await?
            }
            InterruptMode::Discard => {
                let outcome = behavior_for_hook
                    .as_ref()
                    .and_then(|b| {
                        behavior_hooks::resolve_interrupt_discard(b.on_interrupt_discard.as_ref())
                            .ok()
                    })
                    .unwrap_or(InterruptOutcome::Default);
                let _ = outcome;
                self.execute_interrupt_discard(snapshot, &pending_calls)
                    .await?
            }
        }

        self.clear_pending_task_calls().await;
        // Finalise the round — the interrupt path is terminal for whatever
        // turn was in flight; the next inbound input opens a fresh round.
        self.history.finalize_round(RoundStatus::Interrupted).await;
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
            behavior: behavior.meta.name.clone(),
            step_idx: snap_winddown.state.steps.len() as u32,
            wakeup_id: String::new(),
            session_id: self.session_id.clone(),
        };
        let from_user_did = self.current_from_user_did().await;
        let deps = build_session_deps(
            &self.runtime,
            SessionDepsInput {
                tools: self.tools.clone(),
                ctx: ctx_runtime,
                snapshot_path: self.state_snap_path.clone(),
                approval_required: behavior.capabilities.approval_required.clone(),
                one_line_status: Some(self.status.clone() as Arc<dyn OneLineStatusSink>),
                parser_renderer: behavior.build_parser_and_renderer(self.session_class_loop_mode()),
                from_user_did,
            },
        );

        let mut ctx = LLMContext::resume(snap_winddown, ResumeFill::ToolResults { results }, deps)
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
                && msg.content.iter().any(|c| {
                    matches!(c,
                        AiContent::ToolUse { call_id, .. } if pending_ids.contains(call_id.as_str())
                    )
                })
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

    /// Look up the session class config for this session, falling back to
    /// the canonical name (`"ui"` / `"work"`) when no `[session.<class>]`
    /// is configured. Returns owned values to keep the borrow off
    /// `agent_config` short.
    fn session_class_loop_mode(&self) -> LoopMode {
        let class = self.agent_config.class_name_for_kind(self.kind);
        self.agent_config
            .session_class(&class)
            .map(|c| c.loop_mode)
            // Built-in defaults: UI ⇒ Agent loop, Work ⇒ Behavior loop.
            // Matches the pre-config-rewrite hardcoded behavior so an
            // `agent.toml` without `[session.<class>]` entries still
            // boots into the expected loop shape.
            .unwrap_or(match self.kind {
                SessionKind::Ui => LoopMode::Agent,
                SessionKind::Work => LoopMode::Behavior,
            })
    }

    fn session_class_switch_mode(&self) -> SwitchMode {
        let class = self.agent_config.class_name_for_kind(self.kind);
        self.agent_config
            .session_class(&class)
            .map(|c| c.switch_mode)
            .unwrap_or(SwitchMode::Normal)
    }

    /// Map a `BehaviorCfg` to the round-history mode tag (parser-presence is
    /// the canonical signal for Behavior vs Chat per `notepads/session-history.md`
    /// §3).
    fn history_mode_for(&self, behavior: &BehaviorCfg) -> ContextMode {
        if behavior
            .build_parser_and_renderer(self.session_class_loop_mode())
            .is_some()
        {
            ContextMode::Behavior
        } else {
            ContextMode::Chat
        }
    }

    async fn run_one_round(
        &self,
        turn_messages: Vec<AiMessage>,
        seed: Option<RoundSeed>,
    ) -> Result<NextAction> {
        let behavior = self.load_current_behavior().await?;
        let mode = self.history_mode_for(&behavior);

        // Open a round (or attach to one already open). For the PendingTool
        // resume path the worker passes `seed = None`; the caller is
        // responsible for ensuring an open round exists (auto-reopened by
        // the writer on startup when the prior round ended `WaitingTool`).
        if let Some(seed) = seed {
            let opened = self.history.current_round().await.is_some();
            if !opened {
                self.history
                    .begin_round(seed.trigger, seed.input_keys, mode)
                    .await;
            }
            for msg in seed.user_messages {
                self.history.append_message(msg, None).await;
            }
            for (source, payload) in seed.system_events {
                self.history
                    .append_event(HistoryEvent::SystemInput { source, payload })
                    .await;
            }
        }

        let trace_id = self.next_trace_id();
        let (ctx_owner, _request, deps) = self
            .build_or_resume(&behavior, &turn_messages, &trace_id)
            .await?;
        let mut ctx = match ctx_owner {
            BuiltContext::Fresh(c) => c,
            BuiltContext::Resumed(c) => c,
        };
        // Capture the baseline view of the snapshot so the post-run diff
        // can identify exactly which messages / steps this turn produced.
        // `last_step` is compared by `assistant_text` (StepRecord lacks Eq)
        // — sufficient because behavior_loop never overwrites a step's
        // assistant text in place; a different text means a new step.
        let pre = ctx.snapshot();
        let baseline_accumulated_len = pre.state.accumulated.len();
        let baseline_steps_len = pre.state.steps.len();
        let baseline_last_step_text = pre
            .state
            .last_step
            .as_ref()
            .map(|s| s.assistant_text.clone());
        let llm_call = self
            .trace_seq
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        // ContextLimitReached re-entry loop: compress the accumulated
        // history (opendan-side, message-level) and resume the same
        // snapshot via `RewrittenHistory`. Bounded so a pathological
        // history that keeps tripping the limit can't pin the worker.
        //
        // Strategy is gated by the behavior's `[on_context_limit_reached]`
        // hook (see [`behavior_hooks::resolve_ctx_limit`]). v0 modes:
        //   * unset / `Default` ⇒ run the compress loop below (historical
        //     safety net — keeps today's behavior when the hook is omitted).
        //   * `compress_then_continue` ⇒ same compress loop, but signalling
        //     explicit opt-in so future revisions can hang a different
        //     compress strategy on the same on-disk slot.
        // Future "skip compress / fail fast" modes will read this and
        // jump straight to the synthesized-error branch.
        let ctx_limit_outcome =
            behavior_hooks::resolve_ctx_limit(behavior.on_context_limit_reached.as_ref())
                .unwrap_or_else(|err| {
                    warn!(
                        "opendan.session[{}]: invalid on_context_limit_reached hook: {err} \
                 — falling back to runtime default",
                        self.session_id
                    );
                    CtxLimitOutcome::Default
                });
        // Both v0 modes currently route into the compress loop; the variant
        // is captured here so future modes don't have to refactor the loop.
        let _ = matches!(
            ctx_limit_outcome,
            CtxLimitOutcome::Default | CtxLimitOutcome::CompressThenContinue
        );
        const MAX_COMPRESS_ROUNDS: u32 = 3;
        let mut compress_rounds = 0u32;
        loop {
            // Register the *current* ctx's interrupt handle for this run.
            // The compress-resume branch below replaces `ctx` with a freshly
            // resumed instance (and therefore a fresh abort state), so the
            // registration MUST happen inside the loop body — re-registering
            // each iteration is the cheapest way to keep the slot pointed
            // at the live ctx.
            let _interrupt_guard = self.register_interrupt_handle(ctx.interrupt_handle());
            let outcome = ctx.run().await;
            drop(_interrupt_guard);
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
                        let synth_outcome = LLMContextOutcome::Error {
                            error: llm_context::error::LLMComputeError::Internal(format!(
                                "context limit reached {:?} and {compress_rounds} \
                                     compress rounds exhausted",
                                which
                            )),
                            usage: snapshot.state.usage.clone(),
                        };
                        self.history
                            .record_run_diff(
                                mode,
                                baseline_accumulated_len,
                                baseline_steps_len,
                                baseline_last_step_text.clone(),
                                &final_snapshot,
                                &synth_outcome,
                                llm_call,
                            )
                            .await;
                        self.history.append_outcome(&synth_outcome).await;
                        if let Some(status) =
                            SessionHistoryRecorder::round_status_for(&synth_outcome)
                        {
                            self.history.finalize_round(status).await;
                        }
                        return self
                            .handle_outcome(synth_outcome, &behavior, final_snapshot)
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
                    // Record an audit-only Compaction event — history's main
                    // body stays intact; this entry lets reviewers see when
                    // the message-dimension compressor fired.
                    let dropped = before_len.saturating_sub(after_len) as u32;
                    let leading_system = rewritten
                        .iter()
                        .take_while(|m| matches!(m.role, AiRole::System))
                        .count() as u32;
                    let kept_tail = (after_len as u32).saturating_sub(leading_system);
                    self.history
                        .append_event(HistoryEvent::Compaction {
                            target: CompactionTarget::Accumulated,
                            dropped,
                            kept_head: leading_system,
                            kept_tail,
                            summary_preview: format!(
                            "context limit ({:?}): compressed {before_len} → {after_len} messages",
                            which
                        ),
                        })
                        .await;
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
                    self.history
                        .record_run_diff(
                            mode,
                            baseline_accumulated_len,
                            baseline_steps_len,
                            baseline_last_step_text,
                            &final_snapshot,
                            &other,
                            llm_call,
                        )
                        .await;
                    self.history.append_outcome(&other).await;
                    if let Some(status) = SessionHistoryRecorder::round_status_for(&other) {
                        self.history.finalize_round(status).await;
                    }
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

    /// Build the [`RequestOverrides`] that refreshes a resumed snapshot's
    /// request side with the **current** behavior config. Mirrors
    /// `apply_switch_normal` but with `reset_rounds = reset_errors = false`
    /// — this is a soft refresh on every resume, not a behavior switch.
    ///
    /// Without this, edits to the behavior config (system prompt, model,
    /// tool policy, budget, …) made between turns silently fail to land:
    /// resume re-uses the snapshot's stored `request` and only a `switch` /
    /// `discard` path would otherwise pick up the new config.
    async fn current_behavior_overrides(&self, behavior: &BehaviorCfg) -> RequestOverrides {
        RequestOverrides {
            system_messages: Some(self.render_system_messages(behavior).await),
            tool_policy: Some(behavior.to_tool_policy()),
            objective: Some(behavior.meta.objective.clone()),
            behavior_name: Some(behavior.meta.name.clone()),
            model_policy: Some(behavior.to_model_policy()),
            budget: Some(behavior.to_budget_spec()),
            human_policy: Some(behavior.to_human_policy()),
            error_policy: Some(behavior.to_error_policy()),
            output: Some(behavior.to_output_spec()),
            trace: None,
            reset_rounds: false,
            reset_errors: false,
            reset_behavior_hot_tail: false,
            forbid_next_behavior: false,
        }
    }

    async fn build_or_resume(
        &self,
        behavior: &BehaviorCfg,
        turn_messages: &[AiMessage],
        trace_id: &str,
    ) -> Result<(
        BuiltContext,
        LLMContextRequest,
        llm_context::deps::LLMContextDeps,
    )> {
        let ctx = SessionRuntimeContext {
            trace_id: trace_id.to_string(),
            agent_name: self.agent_name.clone(),
            behavior: behavior.meta.name.clone(),
            step_idx: 0,
            wakeup_id: String::new(),
            session_id: self.session_id.clone(),
        };
        let parser_renderer = behavior.build_parser_and_renderer(self.session_class_loop_mode());
        let approval_required = behavior.capabilities.approval_required.clone();
        let from_user_did = self.current_from_user_did().await;

        let deps = build_session_deps(
            &self.runtime,
            SessionDepsInput {
                tools: self.tools.clone(),
                ctx,
                snapshot_path: self.state_snap_path.clone(),
                approval_required,
                one_line_status: Some(self.status.clone() as Arc<dyn OneLineStatusSink>),
                parser_renderer,
                from_user_did,
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
        let turn_message = compose_turn_message(
            turn_messages,
            self.compose_environment_message(behavior).await,
        );

        if let Some(snapshot) = self.try_load_snapshot() {
            if snapshot.state.pending_tool_calls.is_empty() {
                // Refresh the snapshot's request side with the current
                // behavior config before resuming. The cost (one
                // leading-system swap + a handful of policy field copies)
                // is negligible next to history tokens + inference, and it
                // guarantees mid-session config edits actually land — without
                // this, only a `switch` or a `discard` round would pick up
                // the new system prompt / model / tool policy.
                let snapshot = apply_overrides_to_snapshot(
                    snapshot,
                    self.current_behavior_overrides(behavior).await,
                );

                if let Some(message) = turn_message.clone() {
                    // Idle session + new user message: build a fresh
                    // LLMContext whose conversation history *is* the
                    // snapshot's accumulated (already includes the system
                    // segment that was sediment-cloned at first inference),
                    // with the new user turn appended. Per-turn state
                    // (consecutive_errors, usage, steps, trace) resets here;
                    // cross-turn accumulation lives on SessionMeta.
                    let LLMContextSnapshot { mut request, state } = snapshot;
                    let mut input = state.accumulated;
                    input.push(message);
                    request.input = input;
                    request.trace = Some(trace_id.to_string());
                    let fresh = LLMContext::new(request.clone(), deps.clone());
                    return Ok((BuiltContext::Fresh(fresh), request, deps));
                }
                // No new user input — resume the snapshot in place
                // (crash-recovery / idle re-entry without driver).
                let request = snapshot.request.clone();
                let resumed =
                    LLMContext::resume(snapshot, ResumeFill::ResumeFromMidRun, deps.clone())
                        .map_err(|e| anyhow!("resume: {e}"))?;
                return Ok((BuiltContext::Resumed(resumed), request, deps));
            }

            // Snapshot is in a suspended state (pending_tool_calls non-empty)
            // but the worker reached `build_or_resume` instead of
            // `resume_with_tool_results` — meta-level `pending_task_calls` is
            // empty, i.e. there are no in-flight task_mgr handles to wait on.
            // Typical cause: crash between `PendingTool`'s snapshot persist
            // and task dispatch, leaving an orphan suspended snapshot. We
            // cannot synthesize observations to feed `ResumeFill::ToolResults`,
            // so drop the snapshot and start fresh on the current user input.
            // Emit a SystemInput marker so the gap is visible in round history.
            let pending_count = snapshot.state.pending_tool_calls.len();
            warn!(
                "opendan.session[{}]: discarding snapshot with {pending_count} pending tool calls — no resume fill available",
                self.session_id
            );
            self.discard_snapshot();
            self.history.append_event(HistoryEvent::SystemInput {
                source: "session.snapshot_dropped".to_string(),
                payload: serde_json::json!({
                    "reason": "pending_tool_calls present but no in-flight task handles to resume against (likely crash between PendingTool persist and task dispatch)",
                    "pending_count": pending_count,
                }),
            })
            .await;
        }

        let mut input = self.render_system_messages(behavior).await;
        if let Some(message) = turn_message {
            input.push(message);
        }
        let request = LLMContextRequest {
            owner: ContextOwnerRef::Agent {
                session_id: self.session_id.clone(),
            },
            trace: Some(trace_id.to_string()),
            objective: behavior.meta.objective.clone(),
            behavior_name: behavior.meta.name.clone(),
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
        if entry.is_empty() || entry.contains('/') || entry.contains('\\') || entry.contains("..") {
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
    async fn fresh_request_for(&self, cfg: &BehaviorCfg) -> LLMContextRequest {
        LLMContextRequest {
            owner: ContextOwnerRef::Agent {
                session_id: self.session_id.clone(),
            },
            trace: None,
            objective: cfg.meta.objective.clone(),
            behavior_name: cfg.meta.name.clone(),
            input: self.render_system_messages(cfg).await,
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
    /// Build the Phase-1 [`AgentSessionEnv`] snapshot used by both the
    /// behavior-template render path and the environment-block template. See
    /// `doc/opendan/Agent Enviroment.md` §15.1 for the variable contract.
    /// `input_text` is left empty in Phase 1 — `$input.*` is not consumed by
    /// the two currently-templated sections, and the user-input section is
    /// still composed by the legacy `compose_turn_message` path.
    async fn build_prompt_env(&self, behavior: &BehaviorCfg) -> AgentSessionEnv {
        let (kind, title, objective, owner, workspace_id, one_line) = {
            let meta = self.meta.lock().await;
            (
                meta.kind,
                meta.title.clone(),
                meta.objective.clone(),
                meta.owner.clone(),
                meta.workspace_id.clone(),
                meta.one_line_status.clone(),
            )
        };
        let session_objective = if objective.trim().is_empty() {
            behavior.meta.objective.clone()
        } else {
            objective
        };
        let workspace_id = workspace_id.filter(|s| !s.is_empty());
        let workspace_root = workspace_id
            .as_deref()
            .map(|ws| self.agent_config.layout.workspaces_dir.join(ws));
        AgentSessionEnv {
            session_id: self.session_id.clone(),
            session_kind: AgentSessionEnv::kind_str(kind),
            session_title: title.trim().to_string(),
            session_objective,
            session_owner: owner,
            behavior_name: behavior.meta.name.clone(),
            behavior_objective: behavior.meta.objective.clone(),
            behavior_mode: "behavior",
            workspace_id,
            workspace_root,
            agent_root: self.agent_config.layout.root.clone(),
            session_root: self.session_dir.clone(),
            input_text: String::new(),
            input_has_user_text: false,
            input_has_events: false,
            recent_activity: one_line.trim().to_string(),
            clock_unix_ms: now_ms(),
        }
    }

    async fn compose_environment_message(&self, behavior: &BehaviorCfg) -> Option<String> {
        let env = self.build_prompt_env(behavior).await;
        match prompt_env::render_template(ENVIRONMENT_BLOCK_TEMPLATE, &env, &[]).await {
            Ok(text) => Some(text),
            Err(err) => {
                warn!(
                    "opendan.session[{}]: render environment block failed: {err}; falling back to empty",
                    self.session_id
                );
                None
            }
        }
    }

    async fn render_system_messages(&self, behavior: &BehaviorCfg) -> Vec<AiMessage> {
        // Read once: file-system anchors `role.md` / `self.md`, current
        // session env. role.md / self.md are pre-read and injected as
        // `{{ role_md }}` / `{{ self_md }}` template extras for the four
        // shipped behaviors that reference them by name. A future phase
        // migrates the templates to `__INCLUDE($paths.agent_root/role.md)__`
        // and drops these pre-reads entirely.
        let role_md = std::fs::read_to_string(self.agent_config.layout.root.join("role.md"))
            .unwrap_or_default();
        let self_md = std::fs::read_to_string(self.agent_config.layout.root.join("self.md"))
            .unwrap_or_default();
        let env = self.build_prompt_env(behavior).await;

        // `[prompt].on_init` template — render through `PromptRenderEngine`
        // (Phase-1 vars + include_roots) with `role_md` / `self_md` overlaid
        // as render-time extras.
        let template = behavior.prompt.on_init.trim();
        if !template.is_empty() {
            let extras: Vec<(&str, serde_json::Value)> = vec![
                ("role_md", serde_json::Value::String(role_md.clone())),
                ("self_md", serde_json::Value::String(self_md.clone())),
            ];
            match prompt_env::render_template(template, &env, &extras).await {
                Ok(rendered) => return vec![AiMessage::text(AiRole::System, rendered)],
                Err(err) => {
                    warn!(
                        "opendan.session[{}]: render system prompt template failed: {err}; falling back to built-in composition",
                        self.session_id
                    );
                    // fall through to the built-in composition path below
                }
            }
        }

        // No template (or render error) ⇒ runtime built-in composition
        // (matches pre-config-rewrite behavior). Worksession objective
        // surfaces as a dedicated block ahead of the session readme so the
        // LLM sees its task statement first.
        let mut chunks = Vec::new();
        if !role_md.trim().is_empty() {
            chunks.push(role_md);
        }
        if !self_md.trim().is_empty() {
            chunks.push(self_md);
        }
        let objective = env.session_objective.clone();
        let title = env.session_title.clone();
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
        vec![AiMessage::text(AiRole::System, chunks.join("\n\n"))]
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
                response,
                behavior_result,
                ..
            } => {
                self.post_outbound_message(&response.message).await;
                if let Some(text) = output_to_text(&output) {
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
                    self.switch_behavior(trimmed, behavior, final_snapshot)
                        .await?;
                    return Ok(NextAction::Idle);
                }
                // Natural Done (no next_behavior). Persist the completed
                // snapshot so the next round starts from the previous round's
                // accumulated state instead of rebuilding from round-history.
                self.persist_snapshot(&final_snapshot).await;
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
                // §4.7.2 — same runtime-injected `from_user_did` rule
                // applies to async tools as to sync ones: the tool worker
                // must see the real user DID, not whatever the LLM stuffed
                // into args.
                let from_user_did = self.current_from_user_did().await;

                let mut dispatched_any = false;
                for pcall in pending {
                    let call_id = pcall.call.call_id.clone();
                    let tool_name = pcall.call.name.clone();
                    let mut args_json =
                        serde_json::to_value(&pcall.call.args).unwrap_or(serde_json::Value::Null);
                    if let serde_json::Value::Object(map) = &mut args_json {
                        if let Some(did) = from_user_did.as_ref() {
                            map.insert(
                                "from_user_did".to_string(),
                                serde_json::Value::String(did.clone()),
                            );
                        } else {
                            map.remove("from_user_did");
                        }
                    }
                    match dispatcher
                        .dispatch_async_tool(&self.session_id, &tool_name, args_json)
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
                if let Some(message) = partial.as_ref().and_then(output_to_ai_message) {
                    self.post_outbound_message(&message).await;
                    let text = message.text_content();
                    if !text.trim().is_empty() {
                        let _ = self
                            .reply_tx
                            .send(SessionReply::AssistantText { text })
                            .await;
                    }
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
                // `[on_provider_failed]` hook: when configured, swap behavior
                // to the named fallback (e.g. a smaller-model safe-mode) and
                // continue the next turn there. Unset / Default ⇒ surface
                // the error and park the session (historical behavior).
                match behavior_hooks::resolve_provider_failed(behavior.on_provider_failed.as_ref())
                {
                    Ok(ProviderFailedOutcome::FallbackBehavior { target }) => {
                        warn!(
                            "opendan.session[{}]: provider failed ({}); on_provider_failed → fallback_behavior `{target}`",
                            self.session_id, error
                        );
                        self.discard_snapshot();
                        self.meta.lock().await.current_behavior = target.clone();
                        if let Err(err) = self.flush_meta().await {
                            warn!(
                                "opendan.session[{}]: flush after provider-fail fallback failed: {err:#}",
                                self.session_id
                            );
                        }
                        Ok(NextAction::WaitForMsg)
                    }
                    Ok(ProviderFailedOutcome::Default) | Err(_) => {
                        let _ = self
                            .reply_tx
                            .send(SessionReply::Error {
                                message: error.to_string(),
                            })
                            .await;
                        self.discard_snapshot();
                        Ok(NextAction::WaitForMsg)
                    }
                }
            }
            LLMContextOutcome::ContextLimitReached { which, .. } => {
                // Should not happen — `run_one_round` intercepts
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
            LLMContextOutcome::Interrupted {
                reason, snapshot, ..
            } => {
                // §3.13 inference interrupt — scheduler preempted the
                // in-flight inference. `snapshot` is s0 (pre-inference state),
                // so persisting it lets the next turn pick up via
                // `ResumeFromMidRun`. We park the session waiting for either
                // a new user message or an explicit resume.
                self.persist_snapshot(&snapshot).await;
                let _ = self
                    .reply_tx
                    .send(SessionReply::Error {
                        message: format!("inference interrupted: {reason}"),
                    })
                    .await;
                Ok(NextAction::WaitForMsg)
            }
        }
    }

    /// `/clear` command — drop the LLM accumulated state plus every
    /// pending input. After this returns the session looks brand-new from
    /// the LLM's perspective but the on-disk meta (session id, behavior,
    /// workspace binding, owner, peer routing) survives so the next user
    /// message lands on the same session id.
    pub async fn clear_history(&self) -> Result<()> {
        self.discard_snapshot();
        {
            let mut meta = self.meta.lock().await;
            meta.pending_inputs.clear();
            meta.pending_task_calls.clear();
            meta.status = SessionStatus::Idle;
            meta.bootstrap_done = false;
        }
        self.flush_meta().await?;
        Ok(())
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
        // §4.2 of the config-rewrite doc: switch_mode is a session-class
        // property — the LLM picks `<next_behavior>`, the runtime decides
        // whether to go Normal / Fork / Independent.
        match self.session_class_switch_mode() {
            SwitchMode::Normal => {
                self.apply_switch_normal(&new_cfg, final_snapshot).await;
                self.meta.lock().await.current_behavior = new_cfg.meta.name.clone();
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
                self.meta.lock().await.current_behavior = new_cfg.meta.name.clone();
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
    async fn apply_switch_normal(&self, new_cfg: &BehaviorCfg, final_snapshot: LLMContextSnapshot) {
        let new_system = self.render_system_messages(new_cfg).await;
        let overrides = RequestOverrides {
            system_messages: Some(new_system),
            tool_policy: Some(new_cfg.to_tool_policy()),
            objective: Some(new_cfg.meta.objective.clone()),
            behavior_name: Some(new_cfg.meta.name.clone()),
            model_policy: Some(new_cfg.to_model_policy()),
            budget: Some(new_cfg.to_budget_spec()),
            human_policy: Some(new_cfg.to_human_policy()),
            error_policy: Some(new_cfg.to_error_policy()),
            output: Some(new_cfg.to_output_spec()),
            trace: None,
            reset_rounds: false,
            reset_errors: false,
            reset_behavior_hot_tail: true,
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
        self.persist_snapshot_to(&parent_path, &final_snapshot)
            .await;

        // 2. Resume (or build fresh) the child process's snapshot.
        let child_path = self.behavior_snap_path(&new_cfg.meta.name)?;
        let child_snap = if let Some(loaded) = self.try_load_snapshot_from(&child_path) {
            // Existing stream — keep its system / accumulated / steps, just
            // reset the ephemeral counters so the new "turn under this
            // process" starts with a clean budget.
            let overrides = RequestOverrides {
                reset_rounds: true,
                reset_errors: true,
                behavior_name: Some(new_cfg.meta.name.clone()),
                reset_behavior_hot_tail: true,
                ..Default::default()
            };
            apply_overrides_to_snapshot(loaded, overrides)
        } else {
            // First-time entry — synthesize a fresh snapshot from this
            // behavior's request template. Mirrors `build_fresh` at the
            // snapshot level (we don't construct an LLMContext here because
            // the next worker turn will do the resume).
            let request = self.fresh_request_for(new_cfg).await;
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
            meta.process_entry = new_cfg.meta.name.clone();
            meta.current_behavior = new_cfg.meta.name.clone();
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
                let child_entry = std::mem::replace(&mut meta.process_entry, parent.entry.clone());
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
        let parent_path = self.behavior_snap_path(&parent_frame.entry).ok();
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
            ai_message: AiMessage::text(
                AiRole::User,
                format!("[independent process `{}` ended]", child_entry),
            ),
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
            .map_err(|err| anyhow!("fork_and_run: load behavior `{sub_behavior_name}`: {err}"))?;

        let parent_trace = parent_snap
            .request
            .trace
            .clone()
            .unwrap_or_else(|| self.session_id.clone());
        let depth = {
            let mut stack = self
                .fork_stack
                .lock()
                .map_err(|_| anyhow!("fork_and_run: fork stack lock poisoned"))?;
            stack.push(parent_trace.clone());
            stack.len()
        };
        let _fork_stack_guard = ForkStackGuard {
            stack: Arc::clone(&self.fork_stack),
        };
        let trace_id = format!("{}::fork-{}", parent_trace, depth);

        let mut overrides = overrides;
        if overrides.system_messages.is_none() {
            overrides.system_messages = Some(self.render_system_messages(&sub_cfg).await);
        }
        if overrides.behavior_name.is_none() {
            overrides.behavior_name = Some(sub_cfg.meta.name.clone());
        }
        overrides.reset_behavior_hot_tail = true;
        overrides.forbid_next_behavior = true;

        let from_user_did = self.current_from_user_did().await;
        let run_result = run_fork_sub_context(ForkSubContextInput {
            session_id: &self.session_id,
            agent_name: &self.agent_name,
            runtime: &self.runtime,
            tools: self.tools.clone(),
            status: Some(self.status.clone() as Arc<dyn OneLineStatusSink>),
            state_snap_path: &self.state_snap_path,
            parent_snap,
            overrides,
            sub_cfg: &sub_cfg,
            loop_mode: self.session_class_loop_mode(),
            trace_id: &trace_id,
            depth,
            from_user_did,
        })
        .await;
        run_result
    }

    /// Current fork-call-stack depth. `0` ⇒ not inside any active fork.
    /// Async to share the same mutex as `fork_and_run`; intended for
    /// diagnostics / tests.
    pub async fn fork_depth(&self) -> usize {
        self.fork_stack.lock().map(|stack| stack.len()).unwrap_or(0)
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

    /// §4.7.2 — DID the current turn is acting on behalf of. In 1-on-1
    /// chat this is the peer DID stored on `meta.peer_did`; in autonomous
    /// or work sessions there is no upstream human and this is `None`.
    /// The result feeds straight into [`OpendanToolAdapter`] so every
    /// dispatched tool gets the runtime-injected `from_user_did` arg.
    async fn current_from_user_did(&self) -> Option<String> {
        self.meta
            .lock()
            .await
            .peer_did
            .clone()
            .filter(|s| !s.trim().is_empty())
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
        self.subscribe_event_with_template(pattern, None).await
    }

    /// Add or update a persistent kevent subscription. `message_template`
    /// lets the Agent author render events as natural-language messages
    /// instead of leaking raw event JSON into the prompt. Supported
    /// placeholders: `{event_id}`, `{data}`, and top-level JSON fields such
    /// as `{status}` or `{message}`.
    pub async fn subscribe_event_with_template(
        &self,
        pattern: impl Into<String>,
        message_template: Option<String>,
    ) -> Result<bool> {
        let pattern = pattern.into();
        let trimmed = pattern.trim();
        if trimmed.is_empty() {
            return Ok(false);
        }
        let template = message_template.and_then(|s| {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });
        let now = now_ms();
        let mut changed = false;
        {
            let mut meta = self.meta.lock().await;
            if let Some(pos) = meta
                .event_subscriptions
                .iter()
                .position(|s| s.pattern == trimmed)
            {
                let existing = &mut meta.event_subscriptions[pos];
                if existing.message_template != template {
                    existing.message_template = template;
                    changed = true;
                }
            } else {
                meta.event_subscriptions.push(EventSubscription {
                    pattern: trimmed.to_string(),
                    subscribed_at_ms: now,
                    message_template: template,
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
            pump.set_session_subscriptions(&self.session_id, patterns)
                .await;
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

    async fn format_event_for_turn(&self, event_id: &str, data: &serde_json::Value) -> String {
        let subscriptions = self.meta.lock().await.event_subscriptions.clone();
        // Behavior-level fallback template (`[prompt].on_input_event`): used
        // only when no per-subscription template matched. Tolerate a missing
        // behavior file (first-boot / restoring with deleted behavior).
        let behavior_template = match self.load_current_behavior().await {
            Ok(b) => b.prompt.on_input_event.clone(),
            Err(_) => None,
        };
        format_event_for_turn_with_subscriptions(
            event_id,
            data,
            &subscriptions,
            behavior_template.as_deref(),
        )
    }

    async fn post_outbound_message(&self, message: &AiMessage) {
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
        let agent_did_raw = self.agent_config.toml.identity.agent_did.trim();
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
            content: ndn_lib::MsgContent::default(),
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

        let workspace_id = {
            let meta = self.meta.lock().await;
            meta.workspace_id.clone()
        };
        let workspace_dir = workspace_id
            .as_deref()
            .map(|ws| self.agent_config.layout.workspaces_dir.join(ws));
        let validator = crate::attachment_policy::WorkspaceAttachmentValidator::new(
            workspace_dir,
            self.agent_name.clone(),
        );
        let egress_options = llm_context::MsgEgressOptions {
            preserve_attachment_tag_in_egress: self
                .agent_config
                .toml
                .runtime
                .preserve_attachment_tag_in_egress,
        };
        let msg = match llm_context::ai_message_to_msg_object_with_base_validated_with_options(
            message,
            msg,
            &validator,
            egress_options,
        ) {
            Ok(msg) => msg,
            Err(err) => {
                warn!(
                    "opendan.session[{}]: outbound message conversion failed: {err}",
                    self.session_id
                );
                return;
            }
        };
        if msg.content.content.trim().is_empty()
            && msg.content.refs.is_empty()
            && msg.content.machine.is_none()
        {
            return;
        }

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
            let content = data.get("data").cloned().unwrap_or_else(|| data.clone());
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

fn should_replace_pending_event(existing: &PendingInput, incoming: &PendingInput) -> bool {
    let (
        PendingInput::Event {
            data: existing_data,
            ..
        },
        PendingInput::Event {
            data: incoming_data,
            ..
        },
    ) = (existing, incoming)
    else {
        return false;
    };
    event_status_rank(incoming_data) >= event_status_rank(existing_data)
}

fn event_status_rank(data: &serde_json::Value) -> u8 {
    let status = data
        .get("to_status")
        .or_else(|| data.get("status"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match status.as_str() {
        "completed" | "failed" | "canceled" | "cancelled" | "done" | "error" => 2,
        "running" | "pending" | "progress" | "in_progress" => 1,
        _ => 0,
    }
}

/// Translate a subscribed kevent into a short note the LLM can react to as
/// part of the next turn. Keeps the JSON payload but tags it so the model
/// knows this came from the environment, not from a human.
#[cfg(test)]
fn format_event_for_turn(event_id: &str, data: &serde_json::Value) -> String {
    format_event_for_turn_with_subscriptions(event_id, data, &[], None)
}

fn format_event_for_turn_with_subscriptions(
    event_id: &str,
    data: &serde_json::Value,
    subscriptions: &[EventSubscription],
    behavior_template: Option<&str>,
) -> String {
    // Per-subscription template wins (most specific). Behavior-level
    // `[prompt].on_input_event` is the next fallback, then the built-in
    // "An event occurred at ..." string.
    if let Some(template) = subscriptions
        .iter()
        .filter(|s| match_event_patterns(&[s.pattern.clone()], event_id))
        .filter_map(|s| s.message_template.as_deref())
        .find(|s| !s.trim().is_empty())
    {
        return render_event_template(template, event_id, data);
    }
    if let Some(template) = behavior_template.filter(|s| !s.trim().is_empty()) {
        return render_event_template(template, event_id, data);
    }
    let body = if data.is_null() {
        String::new()
    } else if let Ok(rendered) = serde_json::to_string(data) {
        rendered
    } else {
        data.to_string()
    };
    if body.is_empty() {
        format!("An event occurred at `{event_id}`.")
    } else {
        format!("An event occurred at `{event_id}`. Payload: {body}")
    }
}

fn render_event_template(template: &str, event_id: &str, data: &serde_json::Value) -> String {
    let mut rendered = template
        .replace("{event_id}", event_id)
        .replace("{data}", &json_compact(data));
    if let Some(obj) = data.as_object() {
        for (key, value) in obj {
            let placeholder = format!("{{{key}}}");
            if rendered.contains(&placeholder) {
                rendered = rendered.replace(&placeholder, &json_scalar_to_text(value));
            }
        }
    }
    rendered
}

fn json_compact(value: &serde_json::Value) -> String {
    if value.is_null() {
        String::new()
    } else {
        serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
    }
}

fn json_scalar_to_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        _ => json_compact(value),
    }
}

fn format_event_batch_for_turn(events: &[EventForTurn]) -> Option<String> {
    if events.is_empty() {
        return None;
    }
    let mut out = String::from("[event batch]\nThe following subscribed event");
    if events.len() == 1 {
        out.push_str(" arrived and should be handled as a user-visible wakeup:\n");
    } else {
        out.push_str("s arrived and should be handled together as one wakeup:\n");
    }
    for (idx, event) in events.iter().enumerate() {
        out.push_str(&format!(
            "{}. {} (path: `{}`",
            idx + 1,
            event.message.trim(),
            event.event_id
        ));
        if !event.data.is_null() {
            out.push_str(&format!(", data: {}", json_compact(&event.data)));
        }
        out.push_str(")\n");
    }
    Some(out.trim_end().to_string())
}

/// First 100 chars (char-aware) of `text`, used as the `RoundTrigger::UserMsg`
/// preview. Stays well under the design's ~100-char hint and never splits a
/// multi-byte codepoint mid-way.
fn trigger_preview(text: &str) -> String {
    const MAX_CHARS: usize = 100;
    let mut out = String::new();
    for (i, c) in text.chars().enumerate() {
        if i >= MAX_CHARS {
            out.push('…');
            break;
        }
        out.push(c);
    }
    out
}

/// Derive a coarse `event_kind` label from a kevent id. Today's pump produces
/// hierarchical paths like `/task_mgr/123` — the first segment is the most
/// useful classifier (`task_mgr`); fall back to the whole id otherwise.
fn trigger_event_kind(event_id: &str) -> String {
    let trimmed = event_id.trim_start_matches('/');
    trimmed
        .split('/')
        .next()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| event_id.to_string())
}

/// Cap on the size of the tail preserved when compressing `accumulated` on
/// `ContextLimitReached`. Picked empirically — small enough to slash the
/// window dramatically (so a near-limit history reliably fits afterward)
/// while keeping enough recent exchange that the LLM doesn't lose the
/// thread.
const COMPRESS_KEEP_TAIL: usize = 12;

/// Heuristic message-level compressor used by `run_one_round` when the waist
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

#[cfg(test)]
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

fn pending_msg_ai_message(message: &AiMessage) -> AiMessage {
    let mut message = message.clone();
    message.role = AiRole::User;
    message
}

fn ai_message_has_payload(message: &AiMessage) -> bool {
    message.content.iter().any(|block| match block {
        AiContent::Text { text } => !text.trim().is_empty(),
        AiContent::Image { .. }
        | AiContent::Document { .. }
        | AiContent::ToolUse { .. }
        | AiContent::ToolResult { .. }
        | AiContent::Thinking { .. }
        | AiContent::ProviderState { .. } => true,
    })
}

fn pending_msg_preview(text: &str, message: &AiMessage) -> String {
    let trimmed = text.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    let text_content = message.text_content();
    let trimmed = text_content.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    if message
        .content
        .iter()
        .any(|block| matches!(block, AiContent::Image { .. }))
    {
        return "[image]".to_string();
    }
    if message
        .content
        .iter()
        .any(|block| matches!(block, AiContent::Document { .. }))
    {
        return "[document]".to_string();
    }
    "[attachment]".to_string()
}

fn enforce_pending_queue_limit(
    pending: &mut Vec<PendingInput>,
    max: usize,
    agent_name: &str,
) -> usize {
    if max == 0 {
        let dropped = pending.len();
        pending.clear();
        return dropped;
    }
    let mut dropped = 0usize;
    while pending.len() > max {
        if remove_first_pending(pending, |input| matches!(input, PendingInput::Event { .. })) {
            dropped += 1;
            continue;
        }
        if remove_first_pending(pending, |input| {
            matches!(input, PendingInput::Msg { .. })
                && !pending_input_mentions_agent(input, agent_name)
        }) {
            dropped += 1;
            continue;
        }
        if remove_first_pending(pending, |input| {
            !matches!(input, PendingInput::Interrupt { .. })
        }) {
            dropped += 1;
            continue;
        }
        break;
    }
    dropped
}

fn remove_first_pending<F>(pending: &mut Vec<PendingInput>, mut f: F) -> bool
where
    F: FnMut(&PendingInput) -> bool,
{
    if let Some(pos) = pending.iter().position(|input| f(input)) {
        pending.remove(pos);
        true
    } else {
        false
    }
}

fn pending_input_mentions_agent(input: &PendingInput, agent_name: &str) -> bool {
    let needle = agent_mention_token(agent_name);
    if needle.is_empty() {
        return false;
    }
    match input {
        PendingInput::Msg {
            text, ai_message, ..
        } => {
            text.to_ascii_lowercase().contains(&needle)
                || ai_message
                    .text_content()
                    .to_ascii_lowercase()
                    .contains(&needle)
        }
        PendingInput::Event { .. } | PendingInput::Interrupt { .. } => false,
    }
}

fn agent_mention_token(agent_name: &str) -> String {
    let normalized = agent_name
        .trim()
        .trim_start_matches('@')
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    if normalized.is_empty() {
        String::new()
    } else {
        format!("@{normalized}")
    }
}

fn compose_turn_message(messages: &[AiMessage], env: Option<String>) -> Option<AiMessage> {
    if messages.is_empty() {
        return None;
    }
    let mut blocks = Vec::new();
    if let Some(env) = env.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        append_turn_content(
            &mut blocks,
            AiContent::text(background_environment_block(&env)),
        );
    }
    for message in messages {
        for block in &message.content {
            append_turn_content(&mut blocks, block.clone());
        }
    }
    if blocks.is_empty() {
        None
    } else {
        Some(AiMessage::new(AiRole::User, blocks))
    }
}

fn background_environment_block(env: &str) -> String {
    format!(
        "<background_environment>\n{}\n</background_environment>",
        env.trim()
    )
}

fn append_turn_content(blocks: &mut Vec<AiContent>, block: AiContent) {
    if let Some(AiContent::Text { text: previous }) = blocks.last_mut() {
        if let AiContent::Text { text } = block {
            if !previous.trim().is_empty() && !text.trim().is_empty() {
                previous.push_str("\n\n");
            }
            previous.push_str(&text);
            return;
        }
    }
    blocks.push(block);
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
#[cfg(test)]
fn merge_env_and_human(env: Option<String>, human: Option<String>) -> Option<String> {
    match (env, human) {
        (Some(e), Some(h)) => Some(format!("{e}\n\n{h}")),
        (Some(e), None) => Some(e),
        (None, Some(h)) => Some(h),
        (None, None) => None,
    }
}

fn output_to_ai_message(output: &ContextOutput) -> Option<AiMessage> {
    output_to_text(output).map(|text| AiMessage::text(AiRole::Assistant, text))
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
#[path = "agent_session_test.rs"]
mod tests;
