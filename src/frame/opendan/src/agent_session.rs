//! §9.4 of NewOpenDANRuntime — Agent session.
//!
//! MVP scope: text-only inbox, single-turn `LLMContext::run` per arrival batch,
//! resume from a `state.snap` when present, simple Outcome handling, normal-mode
//! behavior switch. PendingTool / Fork / Independent / async-tool dispatch are
//! left as `warn!` + idle for now (§9.6 will round these out once contact_mgr /
//! task_mgr wiring lands).

use std::path::PathBuf;
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
    outcome::{ContextOutput, LLMContextOutcome, ResumeFill},
    request::{ContextOwnerRef, LLMContextRequest},
    state::LLMContextSnapshot,
};

use crate::agent_config::AgentConfig;
use crate::ai_runtime::{
    build_session_deps, AgentRuntime, OneLineStatusSink, SessionDepsInput,
};
use crate::behavior_cfg::{BehaviorCfg, SwitchMode};

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
    Msg {
        record_id: String,
        from: String,
        text: String,
    },
    /// A kevent / system event the session has subscribed to. `event_id` is
    /// the kevent eventid; `data` is the opaque payload.
    Event {
        event_id: String,
        data: serde_json::Value,
    },
}

impl PendingInput {
    /// Stable dedup key. Two `PendingInput`s with the same key are treated
    /// as the same logical item — the second `enqueue_pending` becomes a
    /// no-op so a msg-center lease replay can't double-feed the LLM.
    pub fn dedup_key(&self) -> String {
        match self {
            PendingInput::Msg { record_id, .. } => format!("msg:{record_id}"),
            PendingInput::Event { event_id, .. } => format!("event:{event_id}"),
        }
    }
}

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
            current_behavior,
            status: SessionStatus::Idle,
            owner,
            one_line_status: String::new(),
            pending_inputs: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum SessionReply {
    AssistantText { text: String },
    Error { message: String },
    PromptToHuman { text: String },
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

    trace_seq: Arc<std::sync::atomic::AtomicU64>,
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
}

impl AgentSession {
    pub fn new(b: AgentSessionBuild) -> (Self, mpsc::Receiver<SessionInput>) {
        let session_dir = b.agent_config.layout.session_dir(&b.session_id);
        let state_snap_path = session_dir.join(".meta").join("state.snap");
        let (inbox_tx, inbox_rx) = mpsc::channel(64);

        let meta = SessionMeta::new(
            b.session_id.clone(),
            b.kind,
            b.current_behavior.clone(),
            b.owner.clone(),
        );
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
            trace_seq: Arc::new(std::sync::atomic::AtomicU64::new(0)),
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

    pub async fn start(self: Arc<Self>, mut inbox_rx: mpsc::Receiver<SessionInput>) {
        let me = self.clone();
        let handle = tokio::spawn(async move {
            me.run_worker(&mut inbox_rx).await;
        });
        *self.handle.lock().await = Some(handle);
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
            let pending = self.meta.lock().await.pending_inputs.clone();
            if pending.is_empty() {
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

            // Only Msg variants drive a turn in MVP — Event handling lands
            // when per-session kevent subscriptions are wired. Drop event
            // entries with a warn for now so they don't wedge the queue.
            let mut human_texts = Vec::new();
            let mut consumed_keys = Vec::new();
            for input in &pending {
                match input {
                    PendingInput::Msg { text, .. } => {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() {
                            human_texts.push(trimmed.to_string());
                        }
                        consumed_keys.push(input.dedup_key());
                    }
                    PendingInput::Event { event_id, .. } => {
                        warn!(
                            "opendan.session[{}]: event {} consumed without handler (event dispatch is TODO)",
                            self.session_id, event_id
                        );
                        consumed_keys.push(input.dedup_key());
                    }
                }
            }

            if human_texts.is_empty() {
                // Nothing actionable — clear the consumed (events-only) keys
                // and wait for the next wakeup.
                self.discard_consumed(&consumed_keys).await;
                continue;
            }

            self.set_status(SessionStatus::Running).await;
            let turn_result = self.run_one_turn(human_texts).await;
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

    async fn run_one_turn(&self, human_texts: Vec<String>) -> Result<NextAction> {
        let behavior = self.load_current_behavior().await?;
        let trace_id = self.next_trace_id();
        let (ctx_owner, _request, _deps) =
            self.build_or_resume(&behavior, &human_texts, &trace_id)?;
        let mut ctx = match ctx_owner {
            BuiltContext::Fresh(c) => c,
            BuiltContext::Resumed(c) => c,
        };
        let outcome = ctx.run().await;
        self.handle_outcome(outcome, &behavior).await
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

        if let Some(snapshot) = self.try_load_snapshot() {
            if snapshot.state.pending_tool_calls.is_empty() {
                let fill = if let Some(text) = compose_human_text(human_texts) {
                    ResumeFill::HumanInput {
                        message: AiMessage::text(AiRole::User, text),
                    }
                } else {
                    ResumeFill::ResumeFromMidRun
                };
                let request = snapshot.request.clone();
                let resumed = LLMContext::resume(snapshot, fill, deps.clone())
                    .map_err(|e| anyhow!("resume: {e}"))?;
                return Ok((BuiltContext::Resumed(resumed), request, deps));
            }
            warn!(
                "opendan.session[{}]: snapshot has pending tool calls but resume path is not wired; starting fresh",
                self.session_id
            );
        }

        let mut input = self.render_system_messages(behavior);
        if let Some(text) = compose_human_text(human_texts) {
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
        };
        let fresh = LLMContext::new(request.clone(), deps.clone());
        Ok((BuiltContext::Fresh(fresh), request, deps))
    }

    fn try_load_snapshot(&self) -> Option<LLMContextSnapshot> {
        let bytes = std::fs::read(&self.state_snap_path).ok()?;
        match serde_json::from_slice::<LLMContextSnapshot>(&bytes) {
            Ok(s) => Some(s),
            Err(err) => {
                warn!(
                    "opendan.session[{}]: snapshot at {} unreadable: {err}",
                    self.session_id,
                    self.state_snap_path.display()
                );
                None
            }
        }
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
    ) -> Result<NextAction> {
        match outcome {
            LLMContextOutcome::Done {
                output,
                behavior_result,
                ..
            } => {
                if let Some(text) = output_to_text(&output) {
                    let _ = self
                        .reply_tx
                        .send(SessionReply::AssistantText { text })
                        .await;
                }
                self.discard_snapshot();
                if let Some(next) = behavior_result.and_then(|r| r.next_behavior) {
                    let trimmed = next.trim();
                    if trimmed.eq_ignore_ascii_case("END") {
                        return Ok(NextAction::End);
                    }
                    self.switch_behavior(trimmed, behavior).await?;
                    return Ok(NextAction::Idle);
                }
                if matches!(self.kind, SessionKind::Ui) {
                    Ok(NextAction::WaitForMsg)
                } else {
                    Ok(NextAction::End)
                }
            }
            LLMContextOutcome::WaitInput {
                prompt_to_human, ..
            } => {
                if let Some(prompt) = prompt_to_human {
                    let _ = self
                        .reply_tx
                        .send(SessionReply::PromptToHuman { text: prompt })
                        .await;
                }
                Ok(NextAction::WaitForMsg)
            }
            LLMContextOutcome::PendingTool { .. } => {
                warn!(
                    "opendan.session[{}]: PendingTool outcome — async tool dispatch not yet wired",
                    self.session_id
                );
                Ok(NextAction::WaitForMsg)
            }
            LLMContextOutcome::BudgetExhausted { which, .. } => {
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

    async fn switch_behavior(&self, next: &str, _prev: &BehaviorCfg) -> Result<()> {
        let new_cfg = self
            .agent_config
            .load_behavior(next)
            .map_err(|err| anyhow!("load behavior `{next}`: {err}"))?;
        if !matches!(new_cfg.switch_mode, SwitchMode::Normal) {
            warn!(
                "opendan.session[{}]: switch_mode={:?} not yet wired (treating as Normal)",
                self.session_id, new_cfg.switch_mode
            );
        }
        self.meta.lock().await.current_behavior = new_cfg.name.clone();
        if let Err(err) = self.flush_meta().await {
            warn!(
                "opendan.session[{}]: flush after behavior switch failed: {err:#}",
                self.session_id
            );
        }
        Ok(())
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
}

enum NextAction {
    Idle,
    WaitForMsg,
    End,
}

enum BuiltContext {
    Fresh(LLMContext),
    Resumed(LLMContext),
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
                    text: "hi".to_string(),
                },
                PendingInput::Event {
                    event_id: "/timer/wake".to_string(),
                    data: serde_json::json!({"tick": 7}),
                },
            ],
        };
        let json = serde_json::to_string(&meta).unwrap();
        let restored: SessionMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.pending_inputs.len(), 2);
        match &restored.pending_inputs[0] {
            PendingInput::Msg { record_id, text, .. } => {
                assert_eq!(record_id, "rec-1");
                assert_eq!(text, "hi");
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
