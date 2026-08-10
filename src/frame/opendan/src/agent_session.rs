use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use buckyos_api::{
    get_buckyos_api_runtime, match_event_patterns, parse_typed_task_data, AiContent, AiMessage,
    AiRole, ListTasksReq, MsgCenterClient, Task, TaskManagerClient, TaskNote, TaskOutcome,
    TaskPhase,
    TypedTaskData, UI_SESSION_STATE_STATUS_LINE_KEY, UI_SESSION_STATE_TYPING_KEY,
};
use log::{info, warn};
use ndn_lib::{MsgContent, MsgObjKind, MsgObject};
use serde_json::json;
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tokio::time::{timeout, Duration};

use agent_tool::agent_notebook::{AgentNotebook, AgentNotebookConfig};
use agent_tool::todo_tools::read_todo_records;
use agent_tool::{
    llm_compress, AgentToolManager, SessionRuntimeContext, TodoRecord, DEFAULT_READ_TOKEN_LIMIT,
};
use llm_context::{
    behavior_loop::{
        HistoryInputRecord, SendMessageRecord, StepRecord, StepResultHook, StepResultHookOutput,
    },
    context_loop::LLMContext,
    interrupt::LLMContextInterruptHandle,
    observation::Observation,
    outcome::{ContextOutput, LLMContextOutcome, ResumeFill},
    request::{ContextOwnerRef, LLMContextRequest, ModelPolicy},
    state::{LLMContextSnapshot, LLMContextState},
    step_record::XmlStepRenderer,
    StepRenderer,
};

use crate::agent::AIAgent;
use crate::agent_config::{
    AgentConfig, BehaviorFilter, HookPointCfg, LoadBackgroundHintsPolicy, LoopMode,
    PullEventPolicy, PullMsgPolicy, ReportDeliveryMode, SessionDriverCfg, SessionHookPoint,
    SwitchMode,
};
use crate::ai_runtime::{build_session_deps, AgentRuntime, OneLineStatusSink, SessionDepsInput};
use crate::behavior_cfg::BehaviorCfg;
use crate::behavior_hooks::{
    self, CtxLimitOutcome, InterruptOutcome, LlmMessageCompressPolicy, ProviderFailedOutcome,
};
use crate::llm_context_helper::{
    apply_overrides_to_snapshot, run_fork_sub_context, ForkSubContextInput, RequestOverrides,
};
use crate::local_workspace::LocalWorkspaceManager;
use crate::prompt_env::{self, AgentSessionEnv, LlmContextEnv};
use crate::round_history::{
    CompactionTarget, ContextMode, HistoryEvent, InterruptMode as HistoryInterruptMode,
    RoundStatus, RoundTrigger, SessionHistoryRecorder,
};
use crate::session_event_pump::SessionEventPump;
pub use crate::session_model::{
    BackgroundHint, BgEventSnapshot, EventRef, EventSubscription, EventSubscriptionMode,
    ImprovementBudget, ImprovementBudgetUnit, ImprovementTask, ImprovementTaskStatus,
    InternalContinuation, InterruptMode, LogicalTreeOverlay, OverlayMergeMode, PendingInput,
    PendingTaskCall, ProcessFrame, ReportDeliveryState, SessionInput, SessionKind,
    SessionLogicalProfile, SessionMeta, SessionModelDisable, SessionModelItem,
    SessionModelItemPatch, SessionStatus, SessionSummary, TimerEventKind,
};
use crate::session_topic::{
    read_topic_doc, HintRecallEngine, RecallInput, RecallItem, RecallMode, RecallPayload,
    RecallPolicy, RecallResult, RecallService, TagEntry, TagSet, TagTier,
};
#[cfg(test)]
use crate::session_topic::{RecallHintType, RecallSourceSystem, RecallTarget};
use crate::task_dispatch::TaskDispatch;
use crate::worksession_tools::render_workspace_inventory;

/// Sentinel emitted by a behavior parser in
/// `LLMBehaviorResult.next_behavior` to mean "current intent ran its course,
/// no autonomous next step — park the session until the next inbound user
/// message". Interpreted only at the session layer; the waist treats it as
/// an opaque jump-target string.
pub const NEXT_BEHAVIOR_WAIT_USER_MSG: &str = "WAIT_USER_MSG";
pub const NEXT_BEHAVIOR_WAIT_FOR_MSG: &str = "WAIT_FOR_MSG";
const MAX_PENDING_INPUTS: usize = 256;
const WORKSESSION_REPORT_EVENT_TYPE: &str = "worksession_report";
const UI_IDLE_WORKER_RETIRE_MS: u64 = 15 * 60 * 1000;
const DEFAULT_IDLE_WORKER_RETIRE_MS: u64 = 3 * 60 * 1000;
/// Self-Check heartbeat (doc/opendan/Self-Check Behavior.md §8.3): force a
/// round every 15min while the Notebook is active, backing off to 30min once
/// it goes quiet so an input-free system wakes at most ~48 times/day. The
/// underlying timer still ticks every 60s — these only gate the LLM round.
const SELF_CHECK_ACTIVE_HEARTBEAT_MS: u64 = 15 * 60 * 1000;
const SELF_CHECK_IDLE_HEARTBEAT_MS: u64 = 30 * 60 * 1000;
const BACKGROUND_HINT_NON_EMPTY_INTERVAL_MS: u64 = 60 * 1000;
static FLUSH_META_TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn session_meta_tmp_path(dir: &Path) -> PathBuf {
    let seq = FLUSH_META_TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    dir.join(format!("session.json.{}.{}.tmp", std::process::id(), seq))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorksessionReportPhase {
    Checkpoint,
    Final,
}

impl WorksessionReportPhase {
    fn as_str(self) -> &'static str {
        match self {
            WorksessionReportPhase::Checkpoint => "checkpoint",
            WorksessionReportPhase::Final => "final",
        }
    }
}

#[derive(Debug, Clone)]
pub enum SessionReply {
    AssistantText { text: String },
    Error { message: String },
    Ended,
}

#[derive(Debug, Clone)]
pub enum ManualCompressOutcome {
    NoSnapshot,
    NoChange {
        messages: usize,
        tokens: u32,
        target_tokens: u32,
    },
    Applied {
        before_messages: usize,
        after_messages: usize,
        before_tokens: u32,
        after_tokens: u32,
        target_tokens: u32,
    },
}

pub struct InMemoryStatus {
    current: std::sync::Mutex<String>,
    turn_nonce: std::sync::Mutex<Option<String>>,
    ui_session_sync: Option<UiSessionStateSync>,
}

impl InMemoryStatus {
    pub fn new(ui_session_sync: Option<UiSessionStateSync>) -> Self {
        Self {
            current: std::sync::Mutex::new(String::new()),
            turn_nonce: std::sync::Mutex::new(None),
            ui_session_sync,
        }
    }

    pub fn snapshot(&self) -> String {
        self.current.lock().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn nonce_snapshot(&self) -> Option<String> {
        self.turn_nonce.lock().ok().and_then(|g| g.clone())
    }

    fn set_turn_nonce(&self, nonce: Option<String>) {
        if let Ok(mut g) = self.turn_nonce.lock() {
            *g = nonce;
        }
    }

    fn status_line_value(&self, status: String) -> serde_json::Value {
        match self.nonce_snapshot() {
            Some(nonce) => serde_json::json!({
                "value": status,
                "turn_nonce": nonce,
            }),
            None => serde_json::Value::String(status),
        }
    }

    fn update_ui_state(&self, key: &'static str, value: serde_json::Value) {
        if let Some(sync) = self.ui_session_sync.as_ref() {
            sync.update(key, value);
        }
    }
}

impl OneLineStatusSink for InMemoryStatus {
    fn set(&self, status: String) {
        self.set_with_nonce(status, None);
    }

    fn set_with_nonce(&self, status: String, nonce: Option<String>) {
        self.set_turn_nonce(nonce);
        if let Ok(mut g) = self.current.lock() {
            *g = status;
        }
        self.update_ui_state(
            UI_SESSION_STATE_STATUS_LINE_KEY,
            self.status_line_value(self.snapshot()),
        );
    }
}

#[derive(Clone)]
pub struct UiSessionStateSync {
    msg_center: Option<Arc<MsgCenterClient>>,
    session_id: String,
}

impl UiSessionStateSync {
    fn new(msg_center: Option<Arc<MsgCenterClient>>, session_id: String) -> Self {
        Self {
            msg_center,
            session_id,
        }
    }

    fn update(&self, key: &'static str, value: serde_json::Value) {
        let msg_center = self.msg_center.clone();
        let session_id = self.session_id.clone();
        let key = key.to_string();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let msg_center = match msg_center {
                    Some(client) => client,
                    None => match get_buckyos_api_runtime() {
                        Ok(runtime) => match runtime.get_msg_center_client().await {
                            Ok(client) => Arc::new(client),
                            Err(err) => {
                                warn!(
                                    "opendan.session[{}]: get msg-center client for ui state failed: {err}",
                                    session_id
                                );
                                return;
                            }
                        },
                        Err(err) => {
                            warn!(
                                "opendan.session[{}]: get runtime for ui state failed: {err}",
                                session_id
                            );
                            return;
                        }
                    },
                };
                if let Err(err) = msg_center
                    .update_ui_session_state(session_id.clone(), key.clone(), value)
                    .await
                {
                    warn!(
                        "opendan.session[{}]: update ui_session state key={} failed: {err}",
                        session_id, key
                    );
                }
            });
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
    parent_agent: Weak<AIAgent>,

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
    current_origin_msg: Arc<std::sync::Mutex<Option<AiMessage>>>,

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

#[derive(Debug, Clone)]
struct TurnMessage {
    message: AiMessage,
    runtime_auto: bool,
}

enum RenderedUserMessage {
    NotConfigured,
    Empty,
    Text(String),
}

impl RenderedUserMessage {
    fn into_text(self) -> Option<String> {
        match self {
            Self::Text(text) => Some(text),
            Self::NotConfigured | Self::Empty => None,
        }
    }
}

struct OpenDanStepResultHook {
    template: String,
    behavior: BehaviorCfg,
    agent_config: Arc<AgentConfig>,
    agent_name: String,
    driver: SessionDriverCfg,
    meta: Arc<Mutex<SessionMeta>>,
    session_id: String,
    session_dir: PathBuf,
    excluded_pending_keys: HashSet<String>,
}

#[async_trait]
impl StepResultHook for OpenDanStepResultHook {
    async fn on_behavior_step_ob(
        &self,
        snapshot: &LLMContextSnapshot,
        step: &StepRecord,
    ) -> std::result::Result<StepResultHookOutput, String> {
        let template = self.template.trim();
        if template.is_empty() {
            return Ok(StepResultHookOutput::default());
        }

        let (_, default_user) = XmlStepRenderer::new().render(step);
        let default_user_message = default_user.text_content();
        let default_last_step_action_results_content =
            serde_json::to_value(&step.action_results).unwrap_or(serde_json::Value::Null);
        let selected_pending = self.selected_pending_inputs().await;
        let selected_keys = selected_pending
            .iter()
            .map(PendingInput::dedup_key)
            .collect::<Vec<_>>();
        let pending_inputs = selected_pending
            .iter()
            .map(pending_input_hook_value)
            .collect::<Vec<_>>();
        let pending_input_text = render_pending_input_values(&pending_inputs);
        let observed_at_ms = now_ms();
        let msg_refs = selected_pending
            .iter()
            .filter_map(|input| prompt_env::msg_ref_from_pending(input, observed_at_ms))
            .collect::<Vec<_>>();
        let event_refs = self
            .event_refs_from_selected(&selected_pending, observed_at_ms)
            .await;
        info!(
            "opendan.session[{}]: build user message hook=on_behavior_step_ob behavior=`{}` actions={} results={} pending_inputs={} env_msgs={} env_events={} template_chars={}",
            self.session_id,
            self.behavior.meta.name,
            step.actions.len(),
            step.action_results.len(),
            pending_inputs.len(),
            msg_refs.len(),
            event_refs.len(),
            template.chars().count()
        );
        let mut env = build_agent_session_env(
            &self.session_id,
            &self.agent_config,
            &self.agent_name,
            &self.meta,
            &self.session_dir,
            &self.behavior,
            String::new(),
        )
        .await;
        if let Err(err) = self.flush_meta().await {
            warn!(
                "opendan.session[{}]: flush prompt env cursors after on_behavior_step_ob failed: {err:#}",
                self.session_id
            );
        }
        env.llm_context.msgs = msg_refs;
        env.llm_context.events = event_refs;
        env.llm_context.last_step = snapshot
            .state
            .last_step
            .as_ref()
            .map(prompt_env::step_record_prompt_value);
        env.llm_context.last_report = snapshot.state.last_report.clone();
        env.llm_context.behavior_history = snapshot
            .state
            .steps
            .iter()
            .map(prompt_env::step_record_prompt_value)
            .collect();
        env.llm_context.agent_global_state = merge_global_state_hook_stats(
            serde_json::json!({
                "agent_name": self.agent_config.toml.identity.display_name,
                "session_id": self.session_id,
            }),
            SessionHookPoint::OnBehaviorStepOb.as_key(),
            env.llm_context.msgs.len(),
            env.llm_context.events.len(),
        );
        let extras = [
            (
                "step",
                serde_json::to_value(step).unwrap_or(serde_json::Value::Null),
            ),
            (
                "step_result",
                serde_json::json!({
                    "behavior": step.meta.behavior_name,
                    "step_index": step.meta.step_index,
                    "action_count": step.actions.len(),
                    "result_count": step.action_results.len(),
                    "default_user_message": default_user_message.clone(),
                    "actions": step.actions,
                    "action_results": step.action_results,
                    "messages_sent": step.messages_sent,
                }),
            ),
            (
                "default_last_step_action_results_text",
                serde_json::Value::String(default_user_message),
            ),
            (
                "default_last_step_action_results_content",
                default_last_step_action_results_content,
            ),
            (
                "pending_inputs",
                serde_json::Value::Array(pending_inputs.clone()),
            ),
            (
                "pending_input_text",
                serde_json::Value::String(pending_input_text),
            ),
        ];

        let rendered = prompt_env::render_template(template, &env, &extras)
            .await
            .map_err(|err| err.to_string())?;
        let rendered = rendered.trim();
        if rendered.is_empty() {
            self.discard_selected_pending(&selected_keys).await;
            warn!(
                "opendan.session[{}]: rendered empty user message hook=on_behavior_step_ob behavior=`{}`; skipping next inference",
                self.session_id, self.behavior.meta.name
            );
            return Ok(StepResultHookOutput {
                skip_next_inference: true,
                ..Default::default()
            });
        }

        self.discard_selected_pending(&selected_keys).await;
        info!(
            "opendan.session[{}]: rendered user message hook=on_behavior_step_ob behavior=`{}` chars={} preview=`{}`",
            self.session_id,
            self.behavior.meta.name,
            rendered.chars().count(),
            text_log_preview(rendered)
        );
        Ok(StepResultHookOutput {
            user_message: Some(AiMessage::text(AiRole::User, rendered.to_string())),
            history_inputs: Vec::new(),
            skip_next_inference: false,
        })
    }
}

impl OpenDanStepResultHook {
    async fn selected_pending_inputs(&self) -> Vec<PendingInput> {
        let cfg = self.driver.hook(SessionHookPoint::OnBehaviorStepOb);
        let meta = self.meta.lock().await;
        let default_behavior = self
            .agent_config
            .default_behavior_for_class(&self.agent_config.class_name_for_kind(meta.kind));
        if !hook_filter_allows(
            &cfg.filter,
            &meta.current_behavior,
            &default_behavior,
            &meta.process_stack,
        ) {
            return Vec::new();
        }
        let pending = meta
            .pending_inputs
            .iter()
            .filter(|input| !self.excluded_pending_keys.contains(&input.dedup_key()))
            .cloned()
            .collect::<Vec<_>>();
        select_pending_for_hook_with_subscriptions(
            cfg,
            &pending,
            &HashMap::new(),
            &meta.event_subscriptions,
        )
    }

    async fn event_refs_from_selected(
        &self,
        selected: &[PendingInput],
        observed_at_ms: u64,
    ) -> Vec<EventRef> {
        let subscriptions = self.meta.lock().await.event_subscriptions.clone();
        selected
            .iter()
            .filter_map(|input| {
                let PendingInput::Event { event_id, data } = input else {
                    return None;
                };
                let reason = subscriptions
                    .iter()
                    .find(|sub| sub.matches(event_id))
                    .and_then(|sub| sub.message_template.as_ref())
                    .cloned();
                Some(EventRef {
                    event_id: event_id.clone(),
                    data: data.clone(),
                    reason,
                    observed_at_ms,
                })
            })
            .collect()
    }

    async fn discard_selected_pending(&self, keys: &[String]) {
        if keys.is_empty() {
            return;
        }
        {
            let mut meta = self.meta.lock().await;
            meta.pending_inputs
                .retain(|input| !keys.contains(&input.dedup_key()));
        }
        if let Err(err) = self.flush_meta().await {
            warn!(
                "opendan.session[{}]: flush after on_behavior_step_ob consume failed: {err:#}",
                self.session_id
            );
        }
    }

    async fn flush_meta(&self) -> Result<()> {
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
        let tmp = session_meta_tmp_path(&dir);
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
    pub parent_agent: Weak<AIAgent>,
}

impl AgentSession {
    pub fn new(b: AgentSessionBuild) -> (Self, mpsc::Receiver<SessionInput>) {
        let session_dir = b.agent_config.layout.session_dir(&b.session_id);
        let state_snap_path = session_dir.join(".meta").join("state.snap");
        let (inbox_tx, inbox_rx) = mpsc::channel(64);

        // Restore path: keep persistent fields (pending_inputs, peer info,
        // event_subscriptions) but reset transient status to Idle so the
        // worker re-enters the main loop cleanly.
        let mut meta = if let Some(mut existing) = b.existing_meta {
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
        if matches!(b.kind, SessionKind::SelfImprove) && meta.improvement_budget.is_none() {
            meta.improvement_budget = Some(ImprovementBudget {
                unit: ImprovementBudgetUnit::Token,
                remaining: 32_000,
            });
        }
        if meta.ensure_default_event_subscriptions(now_ms()) {
            info!(
                "opendan.session[{}]: install default ui clock subscription event_id={} mode=background_only",
                b.session_id,
                crate::session_model::UI_CLOCK_TIMER_EVENT_ID
            );
        }
        let history = Arc::new(SessionHistoryRecorder::new(
            b.session_id.clone(),
            session_dir.clone(),
        ));
        let ui_session_sync = if b.runtime.msg_center.is_some() || get_buckyos_api_runtime().is_ok()
        {
            Some(UiSessionStateSync::new(
                b.runtime.msg_center.clone(),
                b.session_id.clone(),
            ))
        } else {
            None
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
            status: Arc::new(InMemoryStatus::new(ui_session_sync)),
            event_pump: b.event_pump,
            parent_agent: b.parent_agent,
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

    pub async fn append_history_event(&self, event: HistoryEvent) {
        self.history.append_event(event).await;
    }

    pub async fn record_recall_background_hints(
        &self,
        recall: Option<&RecallPayload>,
    ) -> Result<()> {
        let Some(recall) = recall else {
            return Ok(());
        };
        let hints = recall_items_to_background_hints(&recall.items);
        if hints.is_empty() {
            return Ok(());
        }
        {
            let mut meta = self.meta.lock().await;
            for hint in hints {
                meta.background_hint_state
                    .hint_fingerprints
                    .insert(hint.path, hint.fingerprint);
            }
        }
        self.flush_meta().await
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
        let tmp = session_meta_tmp_path(&dir);
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

    async fn set_internal_continuation(&self, continuation: InternalContinuation) -> Result<()> {
        {
            let mut meta = self.meta.lock().await;
            meta.internal_continuation = Some(continuation);
        }
        self.flush_meta().await?;
        Ok(())
    }

    pub async fn enqueue_internal_continuation(
        &self,
        reason: String,
        user_messages: Vec<AiMessage>,
    ) -> Result<()> {
        self.set_internal_continuation(InternalContinuation {
            reason,
            user_messages,
        })
        .await?;
        let _ = self.inbox_tx.send(SessionInput::Wakeup).await;
        Ok(())
    }

    pub async fn enqueue_behavior_internal_continuation(
        &self,
        behavior: String,
        reason: String,
        user_messages: Vec<AiMessage>,
    ) -> Result<()> {
        {
            let mut meta = self.meta.lock().await;
            meta.current_behavior = behavior.clone();
            meta.process_entry = behavior;
            meta.process_stack.clear();
            meta.internal_continuation = Some(InternalContinuation {
                reason,
                user_messages,
            });
        }
        self.flush_meta().await?;
        let _ = self.inbox_tx.send(SessionInput::Wakeup).await;
        Ok(())
    }

    async fn take_internal_continuation(&self) -> Option<InternalContinuation> {
        let continuation = {
            let mut meta = self.meta.lock().await;
            meta.internal_continuation.take()
        };
        if continuation.is_some() {
            if let Err(err) = self.flush_meta().await {
                warn!(
                    "opendan.session[{}]: flush after taking internal continuation failed: {err:#}",
                    self.session_id
                );
            }
        }
        continuation
    }

    async fn restore_internal_continuation(&self, continuation: InternalContinuation) {
        {
            let mut meta = self.meta.lock().await;
            meta.internal_continuation = Some(continuation);
        }
        if let Err(err) = self.flush_meta().await {
            warn!(
                "opendan.session[{}]: flush after restoring internal continuation failed: {err:#}",
                self.session_id
            );
        }
    }

    async fn prune_legacy_internal_pending_inputs(&self) {
        let removed = {
            let mut meta = self.meta.lock().await;
            prune_legacy_internal_pending_inputs(&mut meta.pending_inputs)
        };
        if removed > 0 {
            if let Err(err) = self.flush_meta().await {
                warn!(
                    "opendan.session[{}]: flush after pruning legacy internal pending inputs failed: {err:#}",
                    self.session_id
                );
            }
        }
    }

    async fn run_internal_continuation(
        &self,
        continuation: InternalContinuation,
    ) -> Result<NextAction> {
        let messages = continuation.user_messages;
        if messages.is_empty() {
            warn!(
                "opendan.session[{}]: skipped internal continuation `{}` with empty user message; skipping inference",
                self.session_id, continuation.reason
            );
            return Ok(NextAction::Continue);
        }
        let seed = RoundSeed {
            trigger: RoundTrigger::SystemEvent {
                source: "session.internal_continuation".to_string(),
                event_kind: continuation.reason,
            },
            input_keys: Vec::new(),
            user_messages: messages.clone(),
            system_events: Vec::new(),
        };
        self.set_current_origin_msg(messages.first().cloned());
        self.run_one_round(messages, Vec::new(), Some(seed), false)
            .await
    }

    pub async fn push_msg(&self, input: PendingInput) -> Result<()> {
        if !matches!(input, PendingInput::Msg { .. }) {
            return Err(anyhow!("push_msg expects PendingInput::Msg"));
        }
        self.enqueue_pending(input).await
    }

    pub async fn notify_event(&self, event_id: String, data: serde_json::Value) -> Result<bool> {
        let (full_interested, background_interested) = {
            let meta = self.meta.lock().await;
            (
                meta.event_subscriptions
                    .iter()
                    .any(|sub| sub.mode == EventSubscriptionMode::Full && sub.matches(&event_id)),
                meta.event_subscriptions.iter().any(|sub| {
                    sub.mode == EventSubscriptionMode::BackgroundOnly && sub.matches(&event_id)
                }),
            )
        };
        if full_interested {
            info!(
                "opendan.session[{}]: event {} matched full subscription; enqueue pending input",
                self.session_id, event_id
            );
            self.enqueue_pending(PendingInput::Event { event_id, data })
                .await?;
            return Ok(true);
        }

        {
            let mut meta = self.meta.lock().await;
            if let Some(existing) = meta
                .background_events
                .iter_mut()
                .find(|item| item.event_id == event_id)
            {
                info!(
                    "opendan.session[{}]: event {} stored as background snapshot; subscription_match={} action=refresh",
                    self.session_id, event_id, background_interested
                );
                existing.data = data;
                existing.observed_at_ms = now_ms();
            } else {
                info!(
                    "opendan.session[{}]: event {} stored as background snapshot; subscription_match={} action=create",
                    self.session_id, event_id, background_interested
                );
                meta.background_events.push(BgEventSnapshot {
                    event_id,
                    data,
                    reason: None,
                    observed_at_ms: now_ms(),
                });
                const MAX_BG_EVENTS: usize = 32;
                if meta.background_events.len() > MAX_BG_EVENTS {
                    let drop_count = meta.background_events.len() - MAX_BG_EVENTS;
                    meta.background_events.drain(0..drop_count);
                }
            }
        }
        self.flush_meta().await?;
        Ok(false)
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

    pub async fn abort_worker(&self) {
        let handle = self.handle.lock().await.take();
        if let Some(h) = handle {
            h.abort();
            let _ = h.await;
        }
    }

    async fn finish_ended(&self) {
        self.set_status(SessionStatus::Ended).await;
        let _ = self.reply_tx.send(SessionReply::Ended).await;
        if let Some(agent) = self.parent_agent.upgrade() {
            agent.retire_ended_session(&self.session_id).await;
        }
    }

    async fn handle_end_action(&self) -> bool {
        match session_end_disposition(self.kind) {
            SessionEndDisposition::Idle => {
                self.set_status(SessionStatus::Idle).await;
                true
            }
            SessionEndDisposition::Ended => {
                self.finish_ended().await;
                false
            }
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
                    if self.kind.is_work_family() {
                        info!(
                            "opendan.session[{}]: cancel received on work session, exiting worker",
                            self.session_id
                        );
                        return;
                    }
                }
            }

            if let Some(continuation) = self.take_internal_continuation().await {
                self.set_status(SessionStatus::Running).await;
                let round_result = self.run_internal_continuation(continuation.clone()).await;
                match round_result {
                    Ok(action) => match action {
                        NextAction::Continue => continue,
                        NextAction::WaitForMsg => {
                            self.set_status(SessionStatus::WaitingInput).await
                        }
                        NextAction::WaitForTool => {
                            self.set_status(SessionStatus::WaitingTool).await
                        }
                        NextAction::End => {
                            if self.handle_end_action().await {
                                continue;
                            }
                            return;
                        }
                    },
                    Err(err) => {
                        self.restore_internal_continuation(continuation).await;
                        warn!(
                            "opendan.session[{}]: internal continuation failed (kept for retry): {err:#}",
                            self.session_id
                        );
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
                                if self.kind.is_work_family() {
                                    return;
                                }
                            }
                            Some(SessionInput::Wakeup) => {}
                        }
                    }
                }
                continue;
            }

            // Snapshot current pending queue. We DON'T remove items from
            // `meta.pending_inputs` here — that happens only after the turn
            // succeeds, so a crash mid-round leaves the
            // inputs durable and they'll be replayed next boot.
            self.prune_legacy_internal_pending_inputs().await;
            let mut pending = self.meta.lock().await.pending_inputs.clone();
            if pending.is_empty() {
                // Work session bootstrap: if a freshly-created Work session
                // has nothing pending and hasn't run yet, drive an initial
                // turn from its `objective` (per §8.1 step 4 of the design).
                // After the first successful turn this branch falls through
                // to the normal recv()-blocking path.
                let needs_bootstrap =
                    self.kind.is_work_family() && self.needs_bootstrap_turn().await;
                if needs_bootstrap {
                    self.set_status(SessionStatus::Running).await;
                    let behavior = match self.load_current_behavior().await {
                        Ok(behavior) => behavior,
                        Err(err) => {
                            warn!(
                                "opendan.session[{}]: bootstrap load behavior failed: {err:#}",
                                self.session_id
                            );
                            self.set_status(SessionStatus::Error).await;
                            let _ = self
                                .reply_tx
                                .send(SessionReply::Error {
                                    message: format!("bootstrap load behavior failed: {err:#}"),
                                })
                                .await;
                            continue;
                        }
                    };
                    let bootstrap_message = self
                        .render_on_behavior_switch_input_text("none", &behavior, None, None)
                        .await
                        .into_text()
                        .map(|text| AiMessage::text(AiRole::User, text));
                    let bootstrap_messages = bootstrap_message.into_iter().collect::<Vec<_>>();
                    if bootstrap_messages.is_empty() {
                        self.mark_bootstrap_done().await;
                        self.set_status(SessionStatus::WaitingInput).await;
                        continue;
                    }
                    self.set_current_origin_msg(bootstrap_messages.first().cloned());
                    let seed = RoundSeed {
                        trigger: RoundTrigger::SystemEvent {
                            source: "bootstrap".to_string(),
                            event_kind: "objective".to_string(),
                        },
                        input_keys: Vec::new(),
                        user_messages: bootstrap_messages.clone(),
                        system_events: Vec::new(),
                    };
                    let round_result = self
                        .run_one_round(bootstrap_messages, Vec::new(), Some(seed), false)
                        .await;
                    self.mark_bootstrap_done().await;
                    match round_result {
                        Ok(action) => match action {
                            NextAction::Continue => continue,
                            NextAction::WaitForMsg => {
                                self.set_status(SessionStatus::WaitingInput).await
                            }
                            NextAction::WaitForTool => {
                                self.set_status(SessionStatus::WaitingTool).await
                            }
                            NextAction::End => {
                                if self.handle_end_action().await {
                                    continue;
                                }
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
                if self.should_exit_when_idle().await {
                    let idle_retire_ms = idle_worker_retire_ms(self.kind);
                    match timeout(Duration::from_millis(idle_retire_ms), inbox_rx.recv()).await {
                        Ok(None) => {
                            info!(
                                "opendan.session[{}]: inbox closed, exiting worker",
                                self.session_id
                            );
                            return;
                        }
                        Ok(Some(SessionInput::Cancel)) => {
                            self.set_status(SessionStatus::Idle).await;
                            if self.kind.is_work_family() {
                                return;
                            }
                            continue;
                        }
                        Ok(Some(SessionInput::Wakeup)) => continue,
                        Err(_) => {
                            if let Err(err) = self.flush_meta().await {
                                warn!(
                                    "opendan.session[{}]: flush before idle worker exit failed: {err:#}",
                                    self.session_id
                                );
                            }
                            if let Some(agent) = self.parent_agent.upgrade() {
                                agent.retire_idle_session(&self.session_id).await;
                            }
                            info!(
                                "opendan.session[{}]: idle transient worker exiting",
                                self.session_id
                            );
                            return;
                        }
                    }
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
                        if self.kind.is_work_family() {
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

            let pending_task_index = self.pending_task_index().await;
            let driver = self.session_class_driver();
            let selected_pending = self
                .select_pending_for_hook(
                    SessionHookPoint::OnWakeup,
                    &driver,
                    &pending,
                    &pending_task_index,
                )
                .await;
            let on_wakeup_cfg = driver.hook(SessionHookPoint::OnWakeup);
            let selected_stats = pending_input_stats(&selected_pending);
            info!(
                "opendan.session[{}]: driver hook=on_wakeup pending_total={} selected_total={} selected_msg={} selected_event={} selected_interrupt={} pull_msg={:?} pull_event={:?}",
                self.session_id,
                pending.len(),
                selected_pending.len(),
                selected_stats.msg,
                selected_stats.event,
                selected_stats.interrupt,
                on_wakeup_cfg.pull_msg,
                on_wakeup_cfg.pull_event
            );
            if selected_pending.is_empty() {
                self.set_status(SessionStatus::WaitingInput).await;
                if self.should_exit_when_driver_blocked().await {
                    if let Err(err) = self.flush_meta().await {
                        warn!(
                            "opendan.session[{}]: flush before driver-blocked worker exit failed: {err:#}",
                            self.session_id
                        );
                    }
                    if let Some(agent) = self.parent_agent.upgrade() {
                        agent.retire_idle_session(&self.session_id).await;
                    }
                    info!(
                        "opendan.session[{}]: transient worker exiting; pending input is not pulled by current driver hook",
                        self.session_id
                    );
                    return;
                }
                match inbox_rx.recv().await {
                    None => return,
                    Some(SessionInput::Cancel) => {
                        self.set_status(SessionStatus::Idle).await;
                        if self.kind.is_work_family() {
                            return;
                        }
                    }
                    Some(SessionInput::Wakeup) => {}
                }
                continue;
            }
            pending = selected_pending;

            // Three buckets:
            //   - Msg / generic Event → fold into the next round as `round_inputs`
            //   - Event whose id matches a `pending_task_calls` pattern →
            //     translates into an `Observation`, used to build a
            //     `ResumeFill::ToolResults` once every pending call has a
            //     result.
            // Latest peer info wins — the most recent Msg in this batch
            // dictates where outbound replies will be routed.
            let mut turn_messages: Vec<TurnMessage> = Vec::new();
            let mut history_inputs: Vec<HistoryInputRecord> = Vec::new();
            let mut turn_events = Vec::new();
            let mut consumed_keys = Vec::new();
            let mut task_completions: Vec<(String, Observation, String, String)> = Vec::new();
            let mut latest_peer_did: Option<String> = None;
            let mut latest_peer_tunnel: Option<String> = None;
            let mut latest_origin_msg: Option<AiMessage> = None;
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
            for input in &pending {
                match input {
                    PendingInput::Msg {
                        record_id,
                        from,
                        text,
                        from_did,
                        tunnel_did,
                        ai_message,
                        ..
                    } => {
                        let message = pending_msg_ai_message(ai_message);
                        if ai_message_has_payload(&message) {
                            let preview_text = pending_msg_preview(text, &message);
                            if is_history_input_pending(record_id) {
                                history_inputs.push(HistoryInputRecord {
                                    source: from.clone(),
                                    text: message.text_content().trim().to_string(),
                                    at_ms: now_ms(),
                                });
                            } else if !preview_text.trim().is_empty() {
                                if !is_runtime_auto_user_pending(from) {
                                    if first_msg_preview.is_none() {
                                        first_msg_preview = Some(trigger_preview(&preview_text));
                                    }
                                    latest_origin_msg = Some(message.clone());
                                    hist_user_messages.push(message.clone());
                                    msg_count += 1;
                                }
                                turn_messages.push(TurnMessage {
                                    message: message.clone(),
                                    runtime_auto: is_runtime_auto_user_pending(from),
                                });
                            }
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
                                NextAction::Continue => continue,
                                NextAction::WaitForMsg => {
                                    self.set_status(SessionStatus::WaitingInput).await
                                }
                                NextAction::WaitForTool => {
                                    self.set_status(SessionStatus::WaitingTool).await
                                }
                                NextAction::End => {
                                    if self.handle_end_action().await {
                                        continue;
                                    }
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
                                    if self.kind.is_work_family() {
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
                            if self.kind.is_work_family() {
                                return;
                            }
                        }
                        Some(SessionInput::Wakeup) => {}
                    }
                    continue;
                }
            }

            // Self-Check wakeup gate (§8.3): the hard-barrier timer ticks
            // every 60s, but we only want to spend an LLM round when there is
            // real work — a new/changed Notebook item, a user message, or a
            // non-heartbeat timer event — or when a heartbeat is due. Skipping
            // the no-op ticks is what keeps an idle system at ~48 rounds/day.
            if matches!(self.kind, SessionKind::SelfCheck) {
                let has_non_heartbeat_event = turn_events
                    .iter()
                    .any(|e| e.event_id != TimerEventKind::HardBarrier.event_id());
                if !self
                    .self_check_should_run_round(msg_count > 0, has_non_heartbeat_event)
                    .await
                {
                    self.discard_consumed(&consumed_keys).await;
                    continue;
                }
            }

            let hook_env = self
                .apply_hook(SessionHookPoint::OnWakeup, &driver, &pending)
                .await;
            let wakeup_template_text = match self
                .render_on_wakeup_input_text(hook_env.clone())
                .await
            {
                Ok(text) => text,
                Err(err) => {
                    warn!(
                        "opendan.session[{}]: render on_wakeup failed (pending kept for retry): {err:#}",
                        self.session_id
                    );
                    self.set_status(SessionStatus::Error).await;
                    let _ = self
                        .reply_tx
                        .send(SessionReply::Error {
                            message: format!("render on_wakeup failed: {err:#}"),
                        })
                        .await;
                    match inbox_rx.recv().await {
                        None => return,
                        Some(SessionInput::Cancel) => {
                            self.set_status(SessionStatus::Idle).await;
                            if self.kind.is_work_family() {
                                return;
                            }
                        }
                        Some(SessionInput::Wakeup) => {}
                    }
                    continue;
                }
            };
            let round_inputs = match wakeup_template_text {
                RenderedUserMessage::Text(text) => vec![AiMessage::text(AiRole::User, text)],
                RenderedUserMessage::Empty => {
                    self.discard_consumed(&consumed_keys).await;
                    continue;
                }
                RenderedUserMessage::NotConfigured => {
                    let mut round_inputs =
                        prepare_turn_messages_for_run(turn_messages, self.kind.is_work_family());
                    if let Some(batch) = format_event_batch_for_turn(&turn_events) {
                        round_inputs.push(AiMessage::text(AiRole::User, batch));
                    }
                    round_inputs
                }
            };
            info!(
                "opendan.session[{}]: build user input hook=on_wakeup msgs={} events={} history_inputs={} round_user_messages={} first_msg=`{}` first_event={}",
                self.session_id,
                msg_count,
                event_count,
                history_inputs.len(),
                round_inputs.len(),
                first_msg_preview.as_deref().unwrap_or(""),
                first_event_meta
                    .as_ref()
                    .map(|(id, kind)| format!("{id}:{kind}"))
                    .unwrap_or_else(|| "none".to_string())
            );

            // If the snapshot is currently mid-PendingTool and the upper
            // layer queued bare Msg/Event entries without an Interrupt
            // barrier, defer: starting a fresh turn here would discard
            // the in-flight tool round. Upper layers that want immediate
            // attention should `interrupt()` first, then `enqueue_pending`.
            if (!round_inputs.is_empty() || !history_inputs.is_empty())
                && self.snapshot_has_pending_tool_calls().await
            {
                self.set_status(SessionStatus::WaitingTool).await;
                match self.wait_with_tool_sweep(inbox_rx).await {
                    None => return,
                    Some(SessionInput::Cancel) => {
                        self.set_status(SessionStatus::Idle).await;
                        if self.kind.is_work_family() {
                            return;
                        }
                    }
                    Some(SessionInput::Wakeup) => {}
                }
                continue;
            }

            if round_inputs.is_empty() && history_inputs.is_empty() {
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
            let round_result = self
                .run_one_round(
                    round_inputs,
                    history_inputs,
                    Some(seed),
                    msg_count > 0 || event_count > 0,
                )
                .await;
            match round_result {
                Ok(action) => {
                    // Successful turn ⇒ remove the items we just fed to the
                    // LLM from the persistent queue.
                    self.discard_consumed(&consumed_keys).await;
                    // Advance the Self-Check change-watermark to the Notebook's
                    // state *after* this round. Self-Check appends review
                    // remarks, and `append_item_remark` bumps the item's
                    // `updated_at` — capturing the post-round max absorbs those
                    // self-inflicted writes so the next 60s tick doesn't treat
                    // them as a fresh change (which would loop every minute).
                    if matches!(self.kind, SessionKind::SelfCheck) {
                        self.self_check_advance_seen_watermark().await;
                    }
                    match action {
                        NextAction::Continue => continue,
                        NextAction::WaitForMsg => {
                            self.set_status(SessionStatus::WaitingInput).await
                        }
                        NextAction::WaitForTool => {
                            self.set_status(SessionStatus::WaitingTool).await
                        }
                        NextAction::End => {
                            if self.handle_end_action().await {
                                continue;
                            }
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
                            if self.kind.is_work_family() {
                                return;
                            }
                        }
                        Some(SessionInput::Wakeup) => {}
                    }
                }
            }
        }
    }

    /// Decide whether this Self-Check timer wakeup should run a real LLM round
    /// or be skipped as a cheap no-op tick (doc/opendan/Self-Check Behavior.md
    /// §8.3). Returns `true` to run; on a run it advances the persisted gate
    /// state (seen-watermark, last-round time, idle-heartbeat count) and
    /// flushes. A run happens when there is real work — a pending user message,
    /// a non-heartbeat timer event, or a Notebook item updated since the last
    /// round we acted on — or when the heartbeat is due (every 15min while
    /// active, backing off to 30min once the Notebook stays quiet).
    async fn self_check_should_run_round(
        &self,
        has_user_msg: bool,
        has_non_heartbeat_event: bool,
    ) -> bool {
        let now = now_ms();
        let latest_update_secs = notebook_latest_active_update_secs(
            &self.agent_config,
            self.owner.trim(),
            &self.agent_name,
        );
        let (seen_secs, last_round_ms, idle_heartbeats) = {
            let meta = self.meta.lock().await;
            (
                meta.self_check_seen_item_update_secs,
                meta.self_check_last_round_at_ms,
                meta.self_check_idle_heartbeats,
            )
        };
        let notebook_changed = latest_update_secs > seen_secs;
        let real_work = has_user_msg || has_non_heartbeat_event || notebook_changed;

        let (run, next_idle_heartbeats, reason) = if real_work {
            (
                true,
                0u32,
                if notebook_changed {
                    "notebook_changed"
                } else {
                    "input"
                },
            )
        } else {
            // No change: only the periodic hard-barrier heartbeat keeps us
            // running. First gap after activity is 15min; once we've fired an
            // empty heartbeat we stretch to 30min until something changes.
            let interval_ms = if idle_heartbeats == 0 {
                SELF_CHECK_ACTIVE_HEARTBEAT_MS
            } else {
                SELF_CHECK_IDLE_HEARTBEAT_MS
            };
            let heartbeat_due =
                last_round_ms == 0 || now.saturating_sub(last_round_ms) >= interval_ms;
            if heartbeat_due {
                (true, idle_heartbeats.saturating_add(1), "heartbeat")
            } else {
                (false, idle_heartbeats, "skip")
            }
        };

        if run {
            // Record that we're spending a round now. The change-watermark is
            // advanced *after* the round completes (see
            // `self_check_advance_seen_watermark`) so Self-Check's own remark
            // writes are absorbed rather than re-triggering us next tick.
            {
                let mut meta = self.meta.lock().await;
                meta.self_check_last_round_at_ms = now;
                meta.self_check_idle_heartbeats = next_idle_heartbeats;
            }
            if let Err(err) = self.flush_meta().await {
                warn!(
                    "opendan.session[{}]: flush self_check gate state failed: {err:#}",
                    self.session_id
                );
            }
            info!(
                "opendan.session[{}]: self_check gate=run reason={reason} latest_update_secs={latest_update_secs} seen_secs={seen_secs} idle_heartbeats={idle_heartbeats}->{next_idle_heartbeats}",
                self.session_id
            );
        } else {
            info!(
                "opendan.session[{}]: self_check gate=skip latest_update_secs={latest_update_secs} seen_secs={seen_secs} idle_heartbeats={idle_heartbeats} since_last_round_ms={}",
                self.session_id,
                now.saturating_sub(last_round_ms)
            );
        }
        run
    }

    /// Advance the Self-Check change-watermark to the Notebook's current max
    /// `updated_at`, called after a round completes. This absorbs the
    /// `updated_at` bumps caused by Self-Check's own review-remark writes so
    /// the next timer tick only fires a real round on a genuinely newer change.
    /// (A change that lands in the brief window *during* a round is absorbed
    /// too; the 15–30min heartbeat is the backstop that re-reviews it.)
    async fn self_check_advance_seen_watermark(&self) {
        let latest_update_secs = notebook_latest_active_update_secs(
            &self.agent_config,
            self.owner.trim(),
            &self.agent_name,
        );
        let mut changed = false;
        {
            let mut meta = self.meta.lock().await;
            if latest_update_secs > meta.self_check_seen_item_update_secs {
                meta.self_check_seen_item_update_secs = latest_update_secs;
                changed = true;
            }
        }
        if changed {
            if let Err(err) = self.flush_meta().await {
                warn!(
                    "opendan.session[{}]: flush self_check watermark failed: {err:#}",
                    self.session_id
                );
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

    async fn should_exit_when_idle(&self) -> bool {
        let meta = self.meta.lock().await;
        meta.pending_inputs.is_empty() && !matches!(meta.status, SessionStatus::WaitingTool)
    }

    async fn should_exit_when_driver_blocked(&self) -> bool {
        let meta = self.meta.lock().await;
        !matches!(meta.status, SessionStatus::WaitingTool)
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
        self.status.set_turn_nonce(Some(trace_id.clone()));
        let ctx_runtime = SessionRuntimeContext {
            trace_id: trace_id.clone(),
            agent_name: self.agent_name.clone(),
            behavior: behavior.meta.name.clone(),
            step_idx: snapshot.state.steps.len() as u32,
            wakeup_id: String::new(),
            session_id: self.session_id.clone(),
            read_token_limit: DEFAULT_READ_TOKEN_LIMIT,
        };
        let from_user_did = self.current_from_user_did().await;
        let mut deps = build_session_deps(
            &self.runtime,
            SessionDepsInput {
                tools: self.tools.clone(),
                ctx: ctx_runtime,
                snapshot_path: self.state_snap_path.clone(),
                approval_required: behavior.capabilities.approval_required.clone(),
                one_line_status: Some(self.status.clone() as Arc<dyn OneLineStatusSink>),
                i18n: self.agent_config.i18n.clone(),
                parser_renderer: behavior.build_parser_and_renderer(self.session_class_loop_mode()),
                from_user_did,
            },
        );
        let completion_keys: Vec<String> =
            completions.iter().map(|(_, _, _, k)| k.clone()).collect();
        deps = self.attach_step_result_hook(&behavior, deps, &completion_keys);
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
        if let Ok(client) = self.runtime.task_mgr_client().await {
            for entry in &pending_task_entries {
                if let Err(err) = client.cancel_task(&entry.task_id, true).await {
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
        self.status.set_turn_nonce(Some(trace_id.clone()));
        let ctx_runtime = SessionRuntimeContext {
            trace_id,
            agent_name: self.agent_name.clone(),
            behavior: behavior.meta.name.clone(),
            step_idx: snap_winddown.state.steps.len() as u32,
            wakeup_id: String::new(),
            session_id: self.session_id.clone(),
            read_token_limit: DEFAULT_READ_TOKEN_LIMIT,
        };
        let from_user_did = self.current_from_user_did().await;
        let mut deps = build_session_deps(
            &self.runtime,
            SessionDepsInput {
                tools: self.tools.clone(),
                ctx: ctx_runtime,
                snapshot_path: self.state_snap_path.clone(),
                approval_required: behavior.capabilities.approval_required.clone(),
                one_line_status: Some(self.status.clone() as Arc<dyn OneLineStatusSink>),
                i18n: self.agent_config.i18n.clone(),
                parser_renderer: behavior.build_parser_and_renderer(self.session_class_loop_mode()),
                from_user_did,
            },
        );
        deps = self.attach_step_result_hook(&behavior, deps, &[]);

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
        let Ok(client) = self.runtime.task_mgr_client().await else {
            return false;
        };
        let mut synthesized = 0u32;
        for entry in entries {
            match client.get_task(&entry.task_id).await {
                Ok(task) => {
                    if !task.phase.is_terminal() {
                        continue;
                    }
                    let payload = serde_json::json!({
                        "phase": task.phase.to_string(),
                        "outcome": task.outcome,
                        "data": crate::task_util::task_payload(&task),
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
            .unwrap_or_else(|| self.agent_config.default_loop_mode_for_kind(self.kind))
    }

    fn session_class_switch_mode(&self) -> SwitchMode {
        let class = self.agent_config.class_name_for_kind(self.kind);
        self.agent_config
            .session_class(&class)
            .map(|c| c.driver.switch_mode)
            .unwrap_or(SwitchMode::Normal)
    }

    /// Configured `process_stack_limit` for this session class. `0` ⇒
    /// unbounded (the documented default in `SessionClassCfg`).
    fn session_class_process_stack_limit(&self) -> u32 {
        let class = self.agent_config.class_name_for_kind(self.kind);
        self.agent_config
            .session_class(&class)
            .map(|c| c.process_stack_limit)
            .unwrap_or(0)
    }

    /// Side-channel for the §10 #3 "refused fork/independent switch" path.
    /// Writes both a `warn!` line (engineering visibility) and a worklog
    /// record (operator-visible audit trail). Best-effort: any worklog
    /// failure is logged but never propagates — refusing the switch is
    /// already the right user-visible outcome.
    async fn refuse_switch_for_stack_limit(
        &self,
        prev: &BehaviorCfg,
        next_cfg: &BehaviorCfg,
        switch_mode: SwitchMode,
        current_depth: u32,
        limit: u32,
        final_snapshot: &LLMContextSnapshot,
    ) {
        let last_report = final_snapshot.state.last_report.clone().unwrap_or_default();
        warn!(
            "opendan.session[{}]: refusing {:?} switch `{}` -> `{}`: process_stack depth={} >= limit={} (config: session_class.process_stack_limit)",
            self.session_id,
            switch_mode,
            prev.meta.name,
            next_cfg.meta.name,
            current_depth,
            limit
        );
        let record = serde_json::json!({
            "type": "agent.session.behavior_switch_refused",
            "owner_session_id": self.session_id,
            "status": "Refused",
            "payload": {
                "reason": "process_stack_limit_exceeded",
                "from_behavior": prev.meta.name,
                "to_behavior": next_cfg.meta.name,
                "switch_mode": format!("{:?}", switch_mode),
                "process_stack_depth": current_depth,
                "process_stack_limit": limit,
                "last_report": last_report,
            }
        });
        if let Err(err) = self
            .runtime
            .worklog
            .append_record_for_session(
                &self.session_id,
                &self.agent_name,
                &prev.meta.name,
                0,
                record,
            )
            .await
        {
            warn!(
                "opendan.session[{}]: append behavior_switch_refused worklog failed: {err:#}",
                self.session_id
            );
        }
    }

    fn session_class_report_delivery(&self) -> ReportDeliveryMode {
        let class = self.agent_config.class_name_for_kind(self.kind);
        self.agent_config
            .session_class(&class)
            .map(|c| c.driver.report_delivery)
            .unwrap_or_default()
    }

    fn session_class_driver(&self) -> SessionDriverCfg {
        let class = self.agent_config.class_name_for_kind(self.kind);
        self.agent_config
            .session_class(&class)
            .map(|c| c.driver.clone())
            .unwrap_or_else(|| self.agent_config.default_driver_for_kind(self.kind))
    }

    async fn apply_hook(
        &self,
        point: SessionHookPoint,
        driver: &SessionDriverCfg,
        pending: &[PendingInput],
    ) -> LlmContextEnv {
        let cfg = driver.hook(point);
        let observed_at_ms = now_ms();
        let filter_allows = {
            let meta = self.meta.lock().await;
            let default_behavior = self
                .agent_config
                .default_behavior_for_class(&self.agent_config.class_name_for_kind(meta.kind));
            hook_filter_allows(
                &cfg.filter,
                &meta.current_behavior,
                &default_behavior,
                &meta.process_stack,
            )
        };
        let (events, consumed_event_keys, msgs) = if filter_allows {
            let (events, consumed_event_keys) =
                self.pull_events_for_env(cfg, pending, observed_at_ms).await;
            let (msgs, _) = self.pull_msgs_for_env(cfg, pending, observed_at_ms);
            (events, consumed_event_keys, msgs)
        } else {
            (Vec::new(), Vec::new(), Vec::new())
        };
        let msg_count = msgs.len();
        let event_count = consumed_event_keys.len();
        let snapshot = self.try_load_snapshot_for_prompt();
        let agent_global_state = if let Some(agent) = self.parent_agent.upgrade() {
            agent.snapshot_global_state(Some(&self.session_id)).await
        } else {
            serde_json::json!({
                "agent_name": self.agent_name,
                "session_id": self.session_id,
            })
        };
        let (last_step, last_report, behavior_history) = snapshot
            .as_ref()
            .map(|snapshot| {
                (
                    snapshot
                        .state
                        .last_step
                        .as_ref()
                        .map(prompt_env::step_record_prompt_value),
                    snapshot.state.last_report.clone(),
                    snapshot
                        .state
                        .steps
                        .iter()
                        .map(prompt_env::step_record_prompt_value)
                        .collect(),
                )
            })
            .unwrap_or((None, None, Vec::new()));
        let bg_events = self.meta.lock().await.background_events.clone();
        let background_hints =
            if filter_allows && cfg.load_background_hits == LoadBackgroundHintsPolicy::All {
                self.load_changed_background_hits().await
            } else {
                Vec::new()
            };
        let default_changed_background_hint_text =
            render_changed_background_hint_text(&background_hints);
        info!(
            "opendan.session[{}]: apply driver hook={} pending_total={} env_msgs={} env_events={} bg_events={} background_hints={} filter={:?} filter_allows={} pull_msg={:?} pull_event={:?} load_background_hits={:?}",
            self.session_id,
            point.as_key(),
            pending.len(),
            msg_count,
            event_count,
            bg_events.len(),
            background_hints.len(),
            cfg.filter,
            filter_allows,
            cfg.pull_msg,
            cfg.pull_event,
            cfg.load_background_hits
        );
        LlmContextEnv {
            msgs,
            events,
            bg_events,
            background_hints,
            default_changed_background_hint_text,
            last_step,
            last_report,
            behavior_history,
            agent_global_state: merge_global_state_hook_stats(
                agent_global_state,
                point.as_key(),
                msg_count,
                event_count,
            ),
        }
    }

    fn pull_msgs_for_env(
        &self,
        cfg: &HookPointCfg,
        pending: &[PendingInput],
        received_at_ms: u64,
    ) -> (Vec<serde_json::Value>, Vec<String>) {
        let limit = match cfg.pull_msg {
            PullMsgPolicy::None => return (Vec::new(), Vec::new()),
            PullMsgPolicy::One => Some(1usize),
            PullMsgPolicy::All => None,
        };
        let mut values = Vec::new();
        let mut keys = Vec::new();
        for input in pending {
            let Some(msg) = prompt_env::msg_ref_from_pending(input, received_at_ms) else {
                continue;
            };
            values.push(msg);
            keys.push(input.dedup_key());
            if limit.is_some_and(|limit| values.len() >= limit) {
                break;
            }
        }
        (values, keys)
    }

    async fn select_pending_for_hook(
        &self,
        point: SessionHookPoint,
        driver: &SessionDriverCfg,
        pending: &[PendingInput],
        pending_task_index: &std::collections::HashMap<String, PendingTaskCall>,
    ) -> Vec<PendingInput> {
        let cfg = driver.hook(point);
        let meta = self.meta.lock().await;
        let default_behavior = self
            .agent_config
            .default_behavior_for_class(&self.agent_config.class_name_for_kind(meta.kind));
        if !hook_filter_allows(
            &cfg.filter,
            &meta.current_behavior,
            &default_behavior,
            &meta.process_stack,
        ) {
            return Vec::new();
        }
        let subscriptions = meta.event_subscriptions.clone();
        select_pending_for_hook_with_subscriptions(cfg, pending, pending_task_index, &subscriptions)
    }

    async fn pull_events_for_env(
        &self,
        cfg: &HookPointCfg,
        pending: &[PendingInput],
        observed_at_ms: u64,
    ) -> (Vec<EventRef>, Vec<String>) {
        let subscriptions = self.meta.lock().await.event_subscriptions.clone();
        let mut events = Vec::new();
        let mut keys = Vec::new();
        for input in pending {
            let PendingInput::Event { event_id, data } = input else {
                continue;
            };
            let matched = match &cfg.pull_event {
                policy => event_matches_pull_policy(event_id, policy, &subscriptions),
            };
            if matched {
                let reason = subscriptions
                    .iter()
                    .find(|sub| sub.matches(event_id))
                    .and_then(|sub| sub.message_template.as_ref())
                    .cloned();
                events.push(EventRef {
                    event_id: event_id.clone(),
                    data: data.clone(),
                    reason,
                    observed_at_ms,
                });
                keys.push(input.dedup_key());
            }
        }
        (events, keys)
    }

    pub(crate) async fn maybe_publish_worksession_report(
        &self,
        final_snapshot: &LLMContextSnapshot,
        phase: WorksessionReportPhase,
        next_behavior: Option<&str>,
        trace_id: &str,
    ) -> Result<()> {
        if !matches!(self.kind, SessionKind::Work) {
            return Ok(());
        }
        let report = final_snapshot
            .state
            .last_report
            .as_deref()
            .unwrap_or("")
            .trim()
            .to_string();
        if report.is_empty() {
            return Ok(());
        }
        let meta = self.meta.lock().await.clone();
        let context_depth = meta.process_stack.len();
        if !worksession_report_delivery_allows(
            self.session_class_report_delivery(),
            phase,
            context_depth,
        ) {
            return Ok(());
        }
        let target_session_id = meta.owner.trim().to_string();
        if target_session_id.is_empty() {
            warn!(
                "opendan.session[{}]: worksession report has no owner UI session",
                self.session_id
            );
            return Ok(());
        }
        let report_hash = stable_report_hash(&report);
        if meta
            .last_report_delivery
            .as_ref()
            .is_some_and(|state| state.report_hash == report_hash && state.phase == phase.as_str())
        {
            return Ok(());
        }
        let report_id = format!(
            "report:{}:{}:{}",
            self.session_id,
            phase.as_str(),
            report_hash
        );
        let parent_process_entry = meta
            .process_stack
            .last()
            .map(|frame| frame.entry.clone())
            .filter(|entry| !entry.trim().is_empty());
        let created_at_ms = now_ms();
        let data = serde_json::json!({
            "type": WORKSESSION_REPORT_EVENT_TYPE,
            "report_id": report_id,
            "source_session_id": self.session_id,
            "source_kind": "worksession",
            "target_session_id": target_session_id,
            "title": meta.title,
            "objective": meta.objective,
            "workspace_id": meta.workspace_id,
            "behavior": meta.current_behavior,
            "context_depth": context_depth,
            "process_entry": meta.process_entry,
            "parent_process_entry": parent_process_entry,
            "phase": phase.as_str(),
            "report": report,
            "next_behavior": next_behavior,
            "is_final": matches!(phase, WorksessionReportPhase::Final),
            "trace_id": trace_id,
            "created_at_ms": created_at_ms,
        });
        self.write_worksession_report_file(&data).await;
        let target = match self.parent_agent.upgrade() {
            Some(agent) => agent.get_session(&target_session_id).await,
            None => None,
        };
        let posted = if let Some(target) = target {
            if !matches!(target.kind, SessionKind::Ui) {
                warn!(
                    "opendan.session[{}]: worksession report target {} is not a UI session",
                    self.session_id, target_session_id
                );
                return Ok(());
            }
            target
                .post_worksession_report_outbound(&data, Some(report_id.clone()))
                .await?
        } else {
            self.post_worksession_report_outbound_to_persisted_ui(
                &target_session_id,
                &data,
                Some(report_id.clone()),
            )
            .await?
        };
        if posted {
            {
                let mut meta = self.meta.lock().await;
                if meta.last_report_delivery.as_ref().is_some_and(|state| {
                    state.report_hash == report_hash && state.phase == phase.as_str()
                }) {
                    return Ok(());
                }
                meta.last_report_delivery = Some(ReportDeliveryState {
                    report_hash,
                    phase: phase.as_str().to_string(),
                    report_id,
                    delivered_at_ms: now_ms(),
                });
            }
            if let Err(err) = self.flush_meta().await {
                warn!(
                    "opendan.session[{}]: flush report delivery state failed: {err:#}",
                    self.session_id
                );
            }
        }
        Ok(())
    }

    async fn write_worksession_report_file(&self, data: &serde_json::Value) {
        let report = data.get("report").and_then(|v| v.as_str()).unwrap_or("");
        let content = format!(
            "# WorkSession Report\n\n- phase: {}\n- source_session_id: {}\n- target_session_id: {}\n- behavior: {}\n- context_depth: {}\n- created_at_ms: {}\n\n{}",
            data.get("phase").and_then(|v| v.as_str()).unwrap_or(""),
            data.get("source_session_id").and_then(|v| v.as_str()).unwrap_or(""),
            data.get("target_session_id").and_then(|v| v.as_str()).unwrap_or(""),
            data.get("behavior").and_then(|v| v.as_str()).unwrap_or(""),
            data.get("context_depth").and_then(|v| v.as_u64()).unwrap_or(0),
            data.get("created_at_ms").and_then(|v| v.as_u64()).unwrap_or(0),
            report
        );
        let path = self.session_dir.join("report.md");
        if let Err(err) = tokio::fs::write(&path, content).await {
            warn!(
                "opendan.session[{}]: write {} failed: {err}",
                self.session_id,
                path.display()
            );
        }
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
        history_inputs: Vec<HistoryInputRecord>,
        seed: Option<RoundSeed>,
        _inject_background_environment: bool,
    ) -> Result<NextAction> {
        let behavior = self.load_current_behavior().await?;
        let mode = self.history_mode_for(&behavior);
        let in_flight_input_keys = seed
            .as_ref()
            .map(|seed| seed.input_keys.clone())
            .unwrap_or_default();

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
        self.status.set_turn_nonce(Some(trace_id.clone()));
        let (ctx_owner, _request, deps) = self
            .build_or_resume(
                &behavior,
                &turn_messages,
                history_inputs,
                &trace_id,
                &in_flight_input_keys,
                false,
            )
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

        // Commit-pop boundary: once build_or_resume has rendered the turn and
        // produced a context that is about to enter `ctx.run()`, the input is
        // no longer replayable. Keeping it queued after this point risks
        // duplicating tool side effects after a crash or provider panic.
        if !in_flight_input_keys.is_empty() {
            self.discard_consumed(&in_flight_input_keys).await;
        }

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
                    let rewritten = self
                        .compress_accumulated_for_context_limit(
                            &accumulated,
                            &snapshot,
                            &deps,
                            &behavior,
                        )
                        .await
                        .unwrap_or_else(|| compress_messages_for_context_limit(accumulated));
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
                    let raw_final_snapshot = ctx.snapshot();
                    self.history
                        .record_run_diff(
                            mode,
                            baseline_accumulated_len,
                            baseline_steps_len,
                            baseline_last_step_text,
                            &raw_final_snapshot,
                            &other,
                            llm_call,
                        )
                        .await;
                    self.history.append_outcome(&other).await;
                    let final_snapshot = if matches!(other, LLMContextOutcome::Done { .. }) {
                        self.maybe_auto_compress_after_completed_pair(
                            raw_final_snapshot,
                            &deps,
                            &behavior,
                        )
                        .await
                    } else {
                        raw_final_snapshot
                    };
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

    async fn maybe_auto_compress_after_completed_pair(
        &self,
        snapshot: LLMContextSnapshot,
        deps: &llm_context::deps::LLMContextDeps,
        behavior: &BehaviorCfg,
    ) -> LLMContextSnapshot {
        let policy = match behavior_hooks::resolve_llm_message_compress(
            behavior.on_llm_message_compress.as_ref(),
        ) {
            Ok(Some(policy)) => policy,
            Ok(None) => return snapshot,
            Err(err) => {
                warn!(
                    "opendan.session[{}]: invalid on_llm_message_compress hook: {err}; skip auto compression",
                    self.session_id
                );
                return snapshot;
            }
        };

        let Some(context_window_tokens) = self
            .resolve_context_window_tokens(&snapshot.request.model_policy.preferred, &policy)
            .await
        else {
            warn!(
                "opendan.session[{}]: skip llm_message_compress: context window tokens unavailable for model `{}`",
                self.session_id, snapshot.request.model_policy.preferred
            );
            return snapshot;
        };

        let current_tokens = estimate_history_tokens(deps, &snapshot.state.accumulated);
        let trigger_tokens = ratio_budget(context_window_tokens, policy.trigger_ratio);
        let hard_limit_tokens = ratio_budget(context_window_tokens, policy.hard_limit_ratio);
        let above_trigger = current_tokens >= trigger_tokens;
        let above_hard_limit = current_tokens >= hard_limit_tokens;
        if !above_trigger && !above_hard_limit {
            return snapshot;
        }

        if policy.preserve_cache_stability && !above_hard_limit {
            let turns_since = turns_since_last_llm_message_compress(&snapshot.state.accumulated);
            if turns_since < policy.min_turns_between_compress {
                info!(
                    "opendan.session[{}]: skip llm_message_compress: {current_tokens}/{context_window_tokens} tokens but only {turns_since} turn(s) since last compression",
                    self.session_id
                );
                return snapshot;
            }
        }

        let target_token_budget = ratio_budget(context_window_tokens, policy.target_ratio);
        self.compress_snapshot_accumulated(
            snapshot,
            deps,
            target_token_budget,
            "context_window_ratio",
        )
        .await
    }

    pub async fn compress_context_window_once(&self) -> Result<ManualCompressOutcome> {
        let status = self.meta.lock().await.status;
        if matches!(status, SessionStatus::Running | SessionStatus::WaitingTool) {
            return Err(anyhow!(
                "session status is {:?}; compress after the current run/tool wait finishes",
                status
            ));
        }

        let Some(mut snapshot) = self.try_load_snapshot() else {
            return Ok(ManualCompressOutcome::NoSnapshot);
        };
        if !snapshot.state.pending_tool_calls.is_empty() {
            return Err(anyhow!(
                "snapshot has {} pending tool call(s); compress after tool results are resolved",
                snapshot.state.pending_tool_calls.len()
            ));
        }

        let behavior = self.load_current_behavior().await?;
        let policy =
            behavior_hooks::resolve_llm_message_compress(behavior.on_llm_message_compress.as_ref())
                .map_err(|err| anyhow!("invalid on_llm_message_compress hook: {err}"))?
                .unwrap_or_default();
        let context_window_tokens = self
            .resolve_context_window_tokens(&snapshot.request.model_policy.preferred, &policy)
            .await
            .or(snapshot.request.budget.max_total_tokens)
            .ok_or_else(|| {
                anyhow!(
                    "context window tokens unavailable for model `{}`",
                    snapshot.request.model_policy.preferred
                )
            })?;
        let target_token_budget = ratio_budget(context_window_tokens, policy.target_ratio);
        let model_alias = snapshot.request.model_policy.preferred.trim();
        if model_alias.is_empty() {
            return Err(anyhow!("model preferred is empty"));
        }

        let trace_id = self.next_trace_id();
        let from_user_did = self.current_from_user_did().await;
        let deps = build_session_deps(
            &self.runtime,
            SessionDepsInput {
                tools: self.tools.clone(),
                ctx: SessionRuntimeContext {
                    trace_id,
                    agent_name: self.agent_name.clone(),
                    behavior: behavior.meta.name.clone(),
                    step_idx: snapshot.state.steps.len() as u32,
                    wakeup_id: String::new(),
                    session_id: self.session_id.clone(),
                    read_token_limit: DEFAULT_READ_TOKEN_LIMIT,
                },
                snapshot_path: self.state_snap_path.clone(),
                approval_required: behavior.capabilities.approval_required.clone(),
                one_line_status: Some(self.status.clone() as Arc<dyn OneLineStatusSink>),
                i18n: self.agent_config.i18n.clone(),
                parser_renderer: behavior.build_parser_and_renderer(self.session_class_loop_mode()),
                from_user_did,
            },
        );

        let before_messages = snapshot.state.accumulated.len();
        let before_tokens = estimate_history_tokens(&deps, &snapshot.state.accumulated);
        let extra_focus = llm_message_compress_extra_focus(&self.agent_name, &behavior);
        let rewritten = llm_compress::compress(
            &snapshot.state.accumulated,
            &deps,
            target_token_budget,
            model_alias,
            Some(extra_focus.as_str()),
        )
        .await
        .map_err(|err| anyhow!("llm_message_compress failed: {err}"))?;
        let after_messages = rewritten.len();
        let after_tokens = estimate_history_tokens(&deps, &rewritten);

        if after_messages == before_messages && after_tokens >= before_tokens {
            return Ok(ManualCompressOutcome::NoChange {
                messages: before_messages,
                tokens: before_tokens,
                target_tokens: target_token_budget,
            });
        }

        info!(
            "opendan.session[{}]: manual llm_message_compress messages {before_messages} -> {after_messages}, tokens {before_tokens} -> {after_tokens}, target={target_token_budget}",
            self.session_id
        );
        self.history
            .append_event(HistoryEvent::Compaction {
                target: CompactionTarget::Accumulated,
                dropped: before_messages.saturating_sub(after_messages) as u32,
                kept_head: leading_system_messages(&rewritten) as u32,
                kept_tail: after_messages.saturating_sub(leading_system_messages(&rewritten))
                    as u32,
                summary_preview: format!(
                    "llm_message_compress(manual): messages {before_messages} -> {after_messages}, tokens {before_tokens} -> {after_tokens}"
                ),
            })
            .await;
        snapshot.state.accumulated = rewritten;
        self.persist_snapshot(&snapshot).await;
        Ok(ManualCompressOutcome::Applied {
            before_messages,
            after_messages,
            before_tokens,
            after_tokens,
            target_tokens: target_token_budget,
        })
    }

    async fn compress_accumulated_for_context_limit(
        &self,
        accumulated: &[AiMessage],
        snapshot: &LLMContextSnapshot,
        deps: &llm_context::deps::LLMContextDeps,
        behavior: &BehaviorCfg,
    ) -> Option<Vec<AiMessage>> {
        let policy = match behavior_hooks::resolve_llm_message_compress(
            behavior.on_llm_message_compress.as_ref(),
        ) {
            Ok(Some(policy)) => policy,
            Ok(None) => LlmMessageCompressPolicy::default(),
            Err(err) => {
                warn!(
                    "opendan.session[{}]: invalid on_llm_message_compress hook during context-limit compression: {err}; use legacy compressor",
                    self.session_id
                );
                return None;
            }
        };
        let context_window_tokens = self
            .resolve_context_window_tokens(&snapshot.request.model_policy.preferred, &policy)
            .await
            .or(snapshot.request.budget.max_total_tokens)?;
        let target_token_budget = ratio_budget(context_window_tokens, policy.target_ratio);
        let model_alias = snapshot.request.model_policy.preferred.trim();
        if model_alias.is_empty() {
            warn!(
                "opendan.session[{}]: cannot run llm_message_compress: model preferred is empty",
                self.session_id
            );
            return None;
        }
        let extra_focus = llm_message_compress_extra_focus(&self.agent_name, behavior);
        match llm_compress::compress(
            accumulated,
            deps,
            target_token_budget,
            model_alias,
            Some(extra_focus.as_str()),
        )
        .await
        {
            Ok(rewritten) => Some(rewritten),
            Err(err) => {
                warn!(
                    "opendan.session[{}]: llm_message_compress failed during context-limit compression: {err}; use legacy compressor",
                    self.session_id
                );
                None
            }
        }
    }

    async fn compress_snapshot_accumulated(
        &self,
        mut snapshot: LLMContextSnapshot,
        deps: &llm_context::deps::LLMContextDeps,
        target_token_budget: u32,
        reason: &'static str,
    ) -> LLMContextSnapshot {
        let model_alias = snapshot.request.model_policy.preferred.trim();
        if model_alias.is_empty() {
            warn!(
                "opendan.session[{}]: skip llm_message_compress: model preferred is empty",
                self.session_id
            );
            return snapshot;
        }
        let before_len = snapshot.state.accumulated.len();
        let before_tokens = estimate_history_tokens(deps, &snapshot.state.accumulated);
        let behavior = self.load_current_behavior().await.ok();
        let extra_focus = behavior
            .as_ref()
            .map(|behavior| llm_message_compress_extra_focus(&self.agent_name, behavior));
        let rewritten = match llm_compress::compress(
            &snapshot.state.accumulated,
            deps,
            target_token_budget,
            model_alias,
            extra_focus.as_deref(),
        )
        .await
        {
            Ok(rewritten) => rewritten,
            Err(err) => {
                warn!(
                    "opendan.session[{}]: llm_message_compress failed: {err}",
                    self.session_id
                );
                return snapshot;
            }
        };
        let after_len = rewritten.len();
        let after_tokens = estimate_history_tokens(deps, &rewritten);
        if after_len == before_len && after_tokens >= before_tokens {
            info!(
                "opendan.session[{}]: llm_message_compress made no change ({before_tokens} tokens, target {target_token_budget})",
                self.session_id
            );
            return snapshot;
        }

        info!(
            "opendan.session[{}]: llm_message_compress reason={reason} messages {before_len} -> {after_len}, tokens {before_tokens} -> {after_tokens}, target={target_token_budget}",
            self.session_id
        );
        self.history
            .append_event(HistoryEvent::Compaction {
                target: CompactionTarget::Accumulated,
                dropped: before_len.saturating_sub(after_len) as u32,
                kept_head: leading_system_messages(&rewritten) as u32,
                kept_tail: after_len.saturating_sub(leading_system_messages(&rewritten)) as u32,
                summary_preview: format!(
                    "llm_message_compress({reason}): messages {before_len} -> {after_len}, tokens {before_tokens} -> {after_tokens}"
                ),
            })
            .await;
        snapshot.state.accumulated = rewritten;
        snapshot
    }

    async fn resolve_context_window_tokens(
        &self,
        model_alias: &str,
        policy: &LlmMessageCompressPolicy,
    ) -> Option<u32> {
        if let Some(tokens) = policy.context_window_tokens {
            return Some(tokens);
        }
        let aicc = match self.runtime.aicc_client().await {
            Ok(client) => client,
            Err(err) => {
                warn!(
                    "opendan.session[{}]: get aicc client while resolving context window failed: {err}",
                    self.session_id
                );
                return None;
            }
        };
        let directory = match aicc.list_models().await {
            Ok(value) => value,
            Err(err) => {
                warn!(
                    "opendan.session[{}]: models.list failed while resolving context window: {err}",
                    self.session_id
                );
                return None;
            }
        };
        context_window_tokens_from_model_directory(&directory, model_alias)
    }

    /// Build the [`RequestOverrides`] that refreshes non-message request
    /// policy for a resumed snapshot. The message stream is inherited
    /// strictly from the snapshot: `on_init` / system prompt rendering only
    /// happens when a session context is created fresh.
    async fn model_policy_for_behavior(&self, behavior: &BehaviorCfg) -> ModelPolicy {
        let session_profile = {
            let meta = self.meta.lock().await;
            meta.session_profile.clone()
        };
        model_policy_with_session_profile(
            behavior.to_model_policy(),
            self.session_id.as_str(),
            session_profile.as_ref(),
        )
    }

    async fn current_behavior_overrides(&self, behavior: &BehaviorCfg) -> RequestOverrides {
        RequestOverrides {
            system_messages: None,
            user_messages: None,
            tool_policy: Some(behavior.to_tool_policy()),
            objective: Some(behavior.meta.objective.clone()),
            behavior_name: Some(behavior.meta.name.clone()),
            model_policy: Some(self.model_policy_for_behavior(behavior).await),
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
        history_inputs: Vec<HistoryInputRecord>,
        trace_id: &str,
        in_flight_input_keys: &[String],
        _inject_background_environment: bool,
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
            read_token_limit: DEFAULT_READ_TOKEN_LIMIT,
        };
        let parser_renderer = behavior.build_parser_and_renderer(self.session_class_loop_mode());
        let preserve_behavior_state = parser_renderer.is_some();
        let approval_required = behavior.capabilities.approval_required.clone();
        let from_user_did = self.current_from_user_did().await;

        let mut deps = build_session_deps(
            &self.runtime,
            SessionDepsInput {
                tools: self.tools.clone(),
                ctx,
                snapshot_path: self.state_snap_path.clone(),
                approval_required,
                one_line_status: Some(self.status.clone() as Arc<dyn OneLineStatusSink>),
                i18n: self.agent_config.i18n.clone(),
                parser_renderer,
                from_user_did,
            },
        );
        deps = self.attach_step_result_hook(behavior, deps, in_flight_input_keys);

        let turn_message = compose_turn_message(turn_messages);
        if let Some(message) = turn_message.as_ref() {
            info!(
                "opendan.session[{}]: composed user message trace={} behavior=`{}` source_messages={} blocks={} text_chars={} preview=`{}`",
                self.session_id,
                trace_id,
                behavior.meta.name,
                turn_messages.len(),
                message.content.len(),
                message_text_chars(message),
                ai_message_log_preview(message)
            );
        } else if !turn_messages.is_empty() {
            warn!(
                "opendan.session[{}]: skipped empty user message trace={} behavior=`{}` source_messages={}",
                self.session_id,
                trace_id,
                behavior.meta.name,
                turn_messages.len()
            );
        }

        if let Some(snapshot) = self.try_load_snapshot() {
            if snapshot.state.pending_tool_calls.is_empty() {
                // Resume from the snapshot's persisted message stream.
                // Refresh only non-message request policy here; `on_init`
                // system prompt rendering belongs to fresh context creation.
                let snapshot = apply_overrides_to_snapshot(
                    snapshot,
                    self.current_behavior_overrides(behavior).await,
                );

                if turn_message.is_some() || !history_inputs.is_empty() {
                    // Idle session + new user message: rebuild the snapshot
                    // with the new user turn appended while resetting
                    // per-run counters. In behavior mode the StepRecord
                    // stream is the durable execution memory; keep it so a
                    // behavior switch (plan -> do) can see the previous
                    // assistant intent and action results.
                    let snapshot = append_turn_message_to_snapshot(
                        snapshot,
                        turn_message.clone(),
                        history_inputs,
                        trace_id,
                        preserve_behavior_state,
                    );
                    let request = snapshot.request.clone();
                    let resumed =
                        LLMContext::resume(snapshot, ResumeFill::ResumeFromMidRun, deps.clone())
                            .map_err(|e| anyhow!("resume with new turn: {e}"))?;
                    return Ok((BuiltContext::Resumed(resumed), request, deps));
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

        let mut input = self.render_system_messages(behavior).await?;
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
            model_policy: self.model_policy_for_behavior(behavior).await,
            tool_policy: behavior.to_tool_policy(),
            output: behavior.to_output_spec(),
            budget: behavior.to_budget_spec(),
            human_policy: behavior.to_human_policy(),
            error_policy: behavior.to_error_policy(),
            forbid_next_behavior: false,
        };
        let fresh = if history_inputs.is_empty() {
            LLMContext::new(request.clone(), deps.clone())
        } else {
            let mut state = LLMContextState::from_request(&request, now_ms());
            state.history_inputs = history_inputs;
            LLMContext::resume(
                LLMContextSnapshot {
                    request: request.clone(),
                    state,
                },
                ResumeFill::ResumeFromMidRun,
                deps.clone(),
            )
            .map_err(|e| anyhow!("resume fresh with history input: {e}"))?
        };
        Ok((BuiltContext::Fresh(fresh), request, deps))
    }

    fn attach_step_result_hook(
        &self,
        behavior: &BehaviorCfg,
        deps: llm_context::deps::LLMContextDeps,
        in_flight_input_keys: &[String],
    ) -> llm_context::deps::LLMContextDeps {
        let Some(template) = behavior
            .prompt
            .on_behavior_step_ob
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return deps;
        };
        let hook = OpenDanStepResultHook {
            template: template.to_string(),
            behavior: behavior.clone(),
            agent_config: self.agent_config.clone(),
            agent_name: self.agent_name.clone(),
            driver: self.session_class_driver(),
            meta: self.meta.clone(),
            session_id: self.session_id.clone(),
            session_dir: self.session_dir.clone(),
            excluded_pending_keys: in_flight_input_keys.iter().cloned().collect(),
        };
        deps.with_step_result_hook(Arc::new(hook))
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
    async fn fresh_request_for(&self, cfg: &BehaviorCfg) -> Result<LLMContextRequest> {
        Ok(LLMContextRequest {
            owner: ContextOwnerRef::Agent {
                session_id: self.session_id.clone(),
            },
            trace: None,
            objective: cfg.meta.objective.clone(),
            behavior_name: cfg.meta.name.clone(),
            input: self.render_system_messages(cfg).await?,
            model_policy: self.model_policy_for_behavior(cfg).await,
            tool_policy: cfg.to_tool_policy(),
            output: cfg.to_output_spec(),
            budget: cfg.to_budget_spec(),
            human_policy: cfg.to_human_policy(),
            error_policy: cfg.to_error_policy(),
            forbid_next_behavior: false,
        })
    }

    /// Build the Phase-1 [`AgentSessionEnv`] snapshot used by both the
    /// behavior-template render path and prompt environment variables. See
    /// `doc/opendan/Agent Enviroment.md` §15.1 for the variable contract.
    /// `input_text` is left empty in Phase 1 — `$input.*` is not consumed by
    /// the two currently-templated sections, and the user-input section is
    /// still composed by the legacy `compose_turn_message` path.
    async fn build_prompt_env(&self, behavior: &BehaviorCfg) -> AgentSessionEnv {
        let schedule_task_list = self.build_runtime_schedule_task_list_text().await;
        let env = build_agent_session_env(
            &self.session_id,
            &self.agent_config,
            &self.agent_name,
            &self.meta,
            &self.session_dir,
            behavior,
            schedule_task_list,
        )
        .await;
        if let Err(err) = self.flush_meta().await {
            warn!(
                "opendan.session[{}]: flush prompt env cursors failed: {err:#}",
                self.session_id
            );
        }
        env
    }

    async fn build_runtime_schedule_task_list_text(&self) -> String {
        let task_mgr = match self.runtime.task_mgr_client().await {
            Ok(client) => client,
            Err(_) => return "Recent schedule tasks: unavailable.".to_string(),
        };
        let now_secs = now_ms() / 1000;
        let since_secs = {
            let meta = self.meta.lock().await;
            meta.last_schedule_task_list_access_at
        };
        let text = match render_last_schedule_task_list_text(
            task_mgr.as_ref(),
            since_secs,
            now_secs,
        )
        .await
        {
            Ok(text) => text,
            Err(err) => {
                warn!(
                    "opendan.session[{}]: build schedule task prompt text failed: {err}",
                    self.session_id
                );
                return "Recent schedule tasks: unavailable.".to_string();
            }
        };
        {
            let mut meta = self.meta.lock().await;
            meta.last_schedule_task_list_access_at = now_secs.saturating_sub(1);
        }
        if let Err(err) = self.flush_meta().await {
            warn!(
                "opendan.session[{}]: flush schedule task prompt cursor failed: {err:#}",
                self.session_id
            );
        }
        text
    }

    async fn load_changed_background_hits(&self) -> Vec<BackgroundHint> {
        let now = now_ms();
        let (old_fingerprints, bg_events) = {
            let meta = self.meta.lock().await;
            if background_hint_interval_active(
                meta.background_hint_state
                    .last_non_empty_background_hints_at_ms,
                now,
            ) {
                return Vec::new();
            }
            (
                meta.background_hint_state.hint_fingerprints.clone(),
                meta.background_events.clone(),
            )
        };

        let topic_state = load_session_topic_state(&self.session_dir);
        let mut candidates = Vec::new();
        candidates.extend(build_background_event_hints(&bg_events));
        candidates.extend(self.build_unified_background_hints(&topic_state).await);
        let (changed, next_fingerprints) = changed_background_hints(&old_fingerprints, candidates);

        {
            let mut meta = self.meta.lock().await;
            meta.background_hint_state.hint_fingerprints = next_fingerprints;
            if !changed.is_empty() {
                meta.background_hint_state
                    .last_non_empty_background_hints_at_ms = now;
            }
        }
        if let Err(err) = self.flush_meta().await {
            warn!(
                "opendan.session[{}]: flush background hint state failed: {err:#}",
                self.session_id
            );
        }

        changed
    }

    async fn build_unified_background_hints(
        &self,
        topic_state: &SessionTopicState,
    ) -> Vec<BackgroundHint> {
        if topic_state.tags.tags.is_empty() {
            return Vec::new();
        }
        let recall = HintRecallEngine::with_local_roots(
            self.agent_config.layout.memory_dir.clone(),
            self.agent_config.layout.notebook_dir.clone(),
        )
        .recall(
            RecallInput {
                session_id: &self.session_id,
                session_dir: &self.session_dir,
                topic: &topic_state.title,
                tags: &topic_state.tags,
            },
            RecallMode::Mechanical,
            &RecallPolicy {
                mode: RecallMode::Mechanical,
                ..RecallPolicy::default()
            },
        )
        .await;
        match recall {
            RecallResult::Recalled { items, .. } => recall_items_to_background_hints(&items),
            RecallResult::Failed { reason } => {
                warn!(
                    "opendan.session[{}]: unified background hint recall failed: {reason}",
                    self.session_id
                );
                Vec::new()
            }
            RecallResult::NotTriggered => Vec::new(),
        }
    }

    async fn render_system_messages(&self, behavior: &BehaviorCfg) -> Result<Vec<AiMessage>> {
        // Read once: file-system anchors `role.md` / `self.md`, current
        // session env. role.md / self.md are pre-read and injected as
        // `{{ role_md }}` / `{{ self_md }}` template extras for the four
        // shipped behaviors that reference them by name. A future phase
        // migrates the templates to `__INCLUDE(/role.md)__` and drops these
        // pre-reads entirely.
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
                Ok(rendered) => return Ok(vec![AiMessage::text(AiRole::System, rendered)]),
                Err(err) => {
                    let detail = render_template_failure_detail(
                        behavior,
                        "prompt.on_init",
                        template,
                        &env,
                        &err,
                    );
                    warn!(
                        "opendan.session[{}]: render system prompt template failed: {}",
                        self.session_id, detail
                    );
                    return Err(anyhow!("render system prompt template failed: {detail}"));
                }
            }
        }

        // No template ⇒ runtime built-in composition
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
        Ok(vec![AiMessage::text(AiRole::System, chunks.join("\n\n"))])
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
                trace,
                ..
            } => {
                self.post_outbound_message(&response.message).await;
                if matches!(self.kind, SessionKind::SelfCheck) {
                    self.dispatch_behavior_send_messages(&final_snapshot).await;
                }
                if matches!(self.kind, SessionKind::SelfImprove) {
                    self.dispatch_self_improvement_tasks(&final_snapshot).await;
                    if behavior.meta.name == "self_improve_signals" {
                        if let Some(agent) = self.parent_agent.upgrade() {
                            agent.mark_self_improve_stage1_completed().await;
                        }
                    }
                }
                if let Some(text) = output_to_text(&output) {
                    let _ = self
                        .reply_tx
                        .send(SessionReply::AssistantText { text })
                        .await;
                }
                let mut next_behavior = behavior_result
                    .as_ref()
                    .and_then(|r| r.next_behavior.as_deref())
                    .map(str::to_string);
                if matches!(self.kind, SessionKind::SelfImprove)
                    && behavior.meta.name == "self_improve_signals"
                    && next_behavior
                        .as_deref()
                        .map(|next| next.trim().eq_ignore_ascii_case("self_improve_set_memory"))
                        .unwrap_or(false)
                {
                    next_behavior = Some("END".to_string());
                }
                if let Some(next) = next_behavior.as_deref() {
                    let trimmed = next.trim();
                    if trimmed.eq_ignore_ascii_case("END") {
                        // Independent-mode call-stack-aware End: pop a
                        // parent frame if one is waiting; only an empty
                        // stack means the session itself is done.
                        let phase = if self.meta.lock().await.process_stack.is_empty() {
                            WorksessionReportPhase::Final
                        } else {
                            WorksessionReportPhase::Checkpoint
                        };
                        if let Err(err) = self
                            .maybe_publish_worksession_report(
                                &final_snapshot,
                                phase,
                                Some(trimmed),
                                &trace.trace_id,
                            )
                            .await
                        {
                            warn!(
                                "opendan.session[{}]: publish worksession report failed: {err:#}",
                                self.session_id
                            );
                        }
                        let action = self.handle_process_end(final_snapshot.clone()).await?;
                        if matches!(action, NextAction::End) {
                            self.feedback_task_completed(&final_snapshot, Some(trimmed))
                                .await;
                        }
                        return Ok(action);
                    }
                    if trimmed.eq_ignore_ascii_case(NEXT_BEHAVIOR_WAIT_USER_MSG)
                        || trimmed.eq_ignore_ascii_case(NEXT_BEHAVIOR_WAIT_FOR_MSG)
                    {
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
                        if let Err(err) = self
                            .maybe_publish_worksession_report(
                                &final_snapshot,
                                WorksessionReportPhase::Checkpoint,
                                Some(trimmed),
                                &trace.trace_id,
                            )
                            .await
                        {
                            warn!(
                                "opendan.session[{}]: publish worksession report failed: {err:#}",
                                self.session_id
                            );
                        }
                        self.persist_snapshot(&final_snapshot).await;
                        self.feedback_task_waiting_for_input(&final_snapshot, Some(trimmed))
                            .await;
                        return Ok(NextAction::WaitForMsg);
                    }
                    // Switch — preserve history by handing the post-run
                    // snapshot to switch_behavior (which applies the new
                    // behavior's overrides and persists). Do **not** discard
                    // here; next turn resumes from the rebuilt snapshot.
                    if let Err(err) = self
                        .maybe_publish_worksession_report(
                            &final_snapshot,
                            WorksessionReportPhase::Checkpoint,
                            Some(trimmed),
                            &trace.trace_id,
                        )
                        .await
                    {
                        warn!(
                            "opendan.session[{}]: publish worksession report failed: {err:#}",
                            self.session_id
                        );
                    }
                    return self
                        .switch_behavior(trimmed, behavior, final_snapshot)
                        .await;
                }
                // Natural Done (no next_behavior). Persist the completed
                // snapshot so the next round starts from the previous round's
                // accumulated state instead of rebuilding from round-history.
                let phase = if self.meta.lock().await.process_stack.is_empty() {
                    WorksessionReportPhase::Final
                } else {
                    WorksessionReportPhase::Checkpoint
                };
                if let Err(err) = self
                    .maybe_publish_worksession_report(&final_snapshot, phase, None, &trace.trace_id)
                    .await
                {
                    warn!(
                        "opendan.session[{}]: publish worksession report failed: {err:#}",
                        self.session_id
                    );
                }
                self.persist_snapshot(&final_snapshot).await;
                if matches!(self.kind, SessionKind::Ui) {
                    Ok(NextAction::WaitForMsg)
                } else {
                    self.feedback_task_completed(&final_snapshot, None).await;
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

                // Owner key for the dispatched task — fall back to the
                // session's owner / agent name so multi-tenant deployments
                // can scope correctly.
                let owner_for_task = if !self.owner.trim().is_empty() {
                    self.owner.clone()
                } else {
                    self.agent_name.clone()
                };
                let dispatcher =
                    TaskDispatch::from_runtime(self.runtime.task_mgr.clone(), owner_for_task);
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
                            let handle_task_id = handle.task_id.clone();
                            self.add_pending_task_call(PendingTaskCall {
                                call_id: call_id.clone(),
                                tool_name: tool_name.clone(),
                                task_id: handle_task_id,
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
                self.feedback_task_failed(format!("budget exhausted: {:?}", which))
                    .await;
                self.discard_snapshot();
                if matches!(self.kind, SessionKind::SelfImprove) {
                    self.mark_improvement_budget_exhausted().await;
                    return Ok(NextAction::End);
                }
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
                        self.feedback_task_failed(error.to_string()).await;
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
                self.feedback_task_failed(format!("context limit reached: {:?}", which))
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
                self.feedback_task_canceled(format!("inference interrupted: {reason}"))
                    .await;
                Ok(NextAction::WaitForMsg)
            }
        }
    }

    async fn task_binding(&self) -> Option<crate::session_model::AgentTaskBinding> {
        self.meta.lock().await.task_binding.clone()
    }

    async fn feedback_task_completed(
        &self,
        final_snapshot: &LLMContextSnapshot,
        next_behavior: Option<&str>,
    ) {
        let cfg = self.session_class_driver().task_feedback;
        if !cfg.enabled || !cfg.complete_on_end {
            return;
        }
        let Some(binding) = self.task_binding().await else {
            return;
        };
        let Ok(client) = self.runtime.task_mgr_client().await else {
            return;
        };
        let report = final_snapshot.state.last_report.clone().unwrap_or_default();
        let patch = json!({
            "agent_delegate": {
                "execution": {
                    "session_id": self.session_id,
                    "workspace_id": self.workspace_id().await,
                    "status": "completed",
                },
                "result": {
                    "status": "completed",
                    "report": report,
                    "next_behavior": next_behavior,
                }
            }
        });
        let Ok(task) = client.get_task(&binding.task_id).await else {
            return;
        };
        if task.phase.is_terminal() {
            return;
        }
        let mut merged = crate::task_util::task_payload(&task);
        crate::task_util::merge_json(&mut merged, &patch);
        if let Err(err) =
            crate::task_util::commit_task_result(&client, task, merged).await
        {
            warn!(
                "opendan.session[{}]: feedback task completed failed: {err:#}",
                self.session_id
            );
        }
    }

    async fn feedback_task_waiting_for_input(
        &self,
        final_snapshot: &LLMContextSnapshot,
        next_behavior: Option<&str>,
    ) {
        let cfg = self.session_class_driver().task_feedback;
        if !cfg.enabled || !cfg.create_human_input_on_wait {
            return;
        }
        let Some(binding) = self.task_binding().await else {
            return;
        };
        let Some(agent) = self.parent_agent.upgrade() else {
            return;
        };
        let Ok(client) = self.runtime.task_mgr_client().await else {
            return;
        };
        let Ok(task) = client.get_task(&binding.task_id).await else {
            return;
        };
        if task.phase.is_terminal() {
            return;
        }
        let report = final_snapshot
            .state
            .last_report
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("The agent needs user input before it can continue.");
        let question = if next_behavior.is_some() {
            report.to_string()
        } else {
            "The agent is waiting for user input.".to_string()
        };
        if let Err(err) = agent
            .create_human_input_task(&task, &question, "agent_wait_user_msg", Vec::new())
            .await
        {
            warn!(
                "opendan.session[{}]: create human input task failed: {err:#}",
                self.session_id
            );
        }
    }

    async fn feedback_task_failed(&self, message: String) {
        let cfg = self.session_class_driver().task_feedback;
        if !cfg.enabled || !cfg.fail_on_error {
            return;
        }
        let Some(binding) = self.task_binding().await else {
            return;
        };
        let Ok(client) = self.runtime.task_mgr_client().await else {
            return;
        };
        let Ok(task) = client.get_task(&binding.task_id).await else {
            return;
        };
        if task.phase.is_terminal() {
            return;
        }
        let mut detail = crate::task_util::task_payload(&task);
        crate::task_util::merge_json(
            &mut detail,
            &json!({
                "agent_delegate": {
                    "execution": {
                        "session_id": self.session_id,
                        "status": "failed",
                    },
                    "error": {
                        "message": message,
                    }
                }
            }),
        );
        if let Err(err) = crate::task_util::fail_own_task(
            &client,
            task,
            "agent_session_failed",
            message.clone(),
            Some(detail),
        )
        .await
        {
            warn!(
                "opendan.session[{}]: feedback task failed failed: {err:#}",
                self.session_id
            );
        }
    }

    async fn feedback_task_canceled(&self, reason: String) {
        let cfg = self.session_class_driver().task_feedback;
        if !cfg.enabled || !cfg.cancel_on_interrupt {
            return;
        }
        let Some(binding) = self.task_binding().await else {
            return;
        };
        let Ok(client) = self.runtime.task_mgr_client().await else {
            return;
        };
        let Ok(task) = client.get_task(&binding.task_id).await else {
            return;
        };
        if task.phase.is_terminal() {
            return;
        }
        let progress_patch = json!({
            "agent_delegate": {
                "execution": {
                    "session_id": self.session_id,
                    "status": "canceled",
                    "reason": reason,
                }
            }
        });
        let mut merged = crate::task_util::task_payload(&task);
        crate::task_util::merge_json(&mut merged, &progress_patch);
        let task = match crate::task_util::report_progress_value(
            &client,
            task,
            Some(merged),
            Some(reason.clone()),
        )
        .await
        {
            Ok(task) => task,
            Err(_) => match client.get_task(&binding.task_id).await {
                Ok(task) => task,
                Err(_) => return,
            },
        };
        if let Err(err) = crate::task_util::cancel_own_task(&client, &task).await {
            warn!(
                "opendan.session[{}]: feedback task canceled failed: {err:#}",
                self.session_id
            );
        }
    }

    async fn mark_improvement_budget_exhausted(&self) {
        {
            let mut meta = self.meta.lock().await;
            let budget = meta.improvement_budget.get_or_insert(ImprovementBudget {
                unit: ImprovementBudgetUnit::Token,
                remaining: 0,
            });
            budget.remaining = 0;
        }
        if let Err(err) = self.flush_meta().await {
            warn!(
                "opendan.session[{}]: flush improvement budget failed: {err:#}",
                self.session_id
            );
        }
    }

    async fn dispatch_self_improvement_tasks(&self, snapshot: &LLMContextSnapshot) {
        let Some(report) = snapshot
            .state
            .last_report
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return;
        };
        let seq = self
            .trace_seq
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let task = ImprovementTask {
            task_id: format!("improve-{}-{seq}", now_ms()),
            summary: first_non_empty_line(report)
                .unwrap_or("self improvement task")
                .to_string(),
            source_report: report.to_string(),
            created_at_ms: now_ms(),
            status: ImprovementTaskStatus::Dispatched,
        };
        {
            let mut meta = self.meta.lock().await;
            meta.pending_improvement_tasks.push(task.clone());
        }
        if let Err(err) = self.flush_meta().await {
            warn!(
                "opendan.session[{}]: flush improvement task failed: {err:#}",
                self.session_id
            );
        }

        let path = self.session_dir.join("improvement_tasks.jsonl");
        if let Some(parent) = path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        match tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
        {
            Ok(mut file) => {
                let mut line = match serde_json::to_string(&task) {
                    Ok(line) => line,
                    Err(err) => {
                        warn!(
                            "opendan.session[{}]: encode improvement task failed: {err}",
                            self.session_id
                        );
                        return;
                    }
                };
                line.push('\n');
                if let Err(err) = file.write_all(line.as_bytes()).await {
                    warn!(
                        "opendan.session[{}]: write {} failed: {err}",
                        self.session_id,
                        path.display()
                    );
                } else {
                    info!(
                        "opendan.session[{}]: dispatched self improvement task {}",
                        self.session_id, task.task_id
                    );
                }
            }
            Err(err) => warn!(
                "opendan.session[{}]: open {} failed: {err}",
                self.session_id,
                path.display()
            ),
        }
    }

    async fn dispatch_behavior_send_messages(&self, snapshot: &LLMContextSnapshot) {
        let messages = snapshot
            .state
            .steps
            .iter()
            .flat_map(|step| step.messages_sent.iter())
            .chain(
                snapshot
                    .state
                    .last_step
                    .as_ref()
                    .into_iter()
                    .flat_map(|step| step.messages_sent.iter()),
            )
            .collect::<Vec<_>>();
        for message in messages {
            self.post_send_message_record(message).await;
        }
    }

    async fn post_send_message_record(&self, record: &SendMessageRecord) {
        let Ok(msg_center) = self.runtime.msg_center_client().await else {
            return;
        };
        let target = record.target.trim();
        let body = record.body.trim();
        if target.is_empty() || body.is_empty() {
            return;
        }
        let Ok(peer_did) = name_lib::DID::from_str(target) else {
            warn!(
                "opendan.session[{}]: sendmsg target `{}` is not a DID",
                self.session_id, target
            );
            return;
        };
        let agent_did_raw = self.agent_config.toml.identity.agent_did.trim();
        if agent_did_raw.is_empty() {
            return;
        }
        let Ok(agent_did) = name_lib::DID::from_str(agent_did_raw) else {
            return;
        };
        let msg = ndn_lib::MsgObject {
            from: agent_did.clone(),
            to: vec![peer_did],
            kind: ndn_lib::MsgObjKind::Chat,
            created_at_ms: now_ms(),
            content: ndn_lib::MsgContent {
                format: Some(ndn_lib::MsgContentFormat::TextPlain),
                content: body.to_string(),
                ..ndn_lib::MsgContent::default()
            },
            ..ndn_lib::MsgObject::default()
        };
        match msg_center.post_send(msg, None).await {
            Ok(result) if result.ok => {}
            Ok(result) => warn!(
                "opendan.session[{}]: sendmsg rejected — reason={:?}",
                self.session_id, result.reason
            ),
            Err(err) => warn!(
                "opendan.session[{}]: sendmsg post_send failed: {err}",
                self.session_id
            ),
        }
    }

    /// Drop the LLM accumulated state plus every
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
        prev: &BehaviorCfg,
        final_snapshot: LLMContextSnapshot,
    ) -> Result<NextAction> {
        let new_cfg = self
            .agent_config
            .load_behavior(next)
            .map_err(|err| anyhow!("load behavior `{next}`: {err}"))?;
        // §4.2 of the config-rewrite doc: switch_mode is a session-class
        // property — the LLM picks `<next_behavior>`, the runtime decides
        // whether to go Normal / Fork / Independent.
        let switch_mode = self.session_class_switch_mode();
        // §10 #3: enforce `process_stack_limit`. Normal switches don't grow
        // the stack, so they're exempt. Fork / Independent would push a
        // ProcessFrame — refuse if it would exceed the configured ceiling.
        // Policy (per spec §10 #3 — "refuse fork + warn to worklog,
        // escape valve for ops"): drop the switch, persist the post-run
        // snapshot, and end the current process gracefully so the
        // session lifecycle stays observable.
        if matches!(switch_mode, SwitchMode::Fork | SwitchMode::Independent) {
            let limit = self.session_class_process_stack_limit();
            if limit > 0 {
                let current_depth = self.meta.lock().await.process_stack.len() as u32;
                if current_depth >= limit {
                    self.refuse_switch_for_stack_limit(
                        prev,
                        &new_cfg,
                        switch_mode,
                        current_depth,
                        limit,
                        &final_snapshot,
                    )
                    .await;
                    self.persist_snapshot(&final_snapshot).await;
                    return Ok(match self.kind {
                        SessionKind::Ui => NextAction::WaitForMsg,
                        _ => NextAction::End,
                    });
                }
            }
        }
        let from_context = prompt_env::context_snapshot_prompt_value(&final_snapshot);
        let from_context_report = final_snapshot.state.last_report.clone().unwrap_or_default();
        match switch_mode {
            SwitchMode::Normal => {
                self.apply_switch_normal(&new_cfg, final_snapshot).await?;
                self.meta.lock().await.current_behavior = new_cfg.meta.name.clone();
            }
            SwitchMode::Independent => {
                self.apply_switch_independent(&new_cfg, final_snapshot)
                    .await?;
                // process_entry / current_behavior already updated inside
                // apply_switch_independent (push happens under the same lock).
            }
            SwitchMode::Fork => {
                self.apply_switch_fork(&new_cfg, final_snapshot).await?;
                // process_entry / current_behavior already updated inside
                // apply_switch_fork (push happens under the same lock).
            }
        }
        if let Err(err) = self.flush_meta().await {
            warn!(
                "opendan.session[{}]: flush after behavior switch failed: {err:#}",
                self.session_id
            );
        }
        let messages: Vec<AiMessage> = self
            .render_on_behavior_switch_input_text(
                &prev.meta.name,
                &new_cfg,
                Some(from_context_report.as_str()),
                Some(from_context),
            )
            .await
            .into_text()
            .map(|text| AiMessage::text(AiRole::User, text))
            .into_iter()
            .collect();
        if messages.is_empty() {
            return Ok(NextAction::Continue);
        }
        self.set_internal_continuation(InternalContinuation {
            reason: format!("behavior_switch:{}->{}", prev.meta.name, new_cfg.meta.name),
            user_messages: messages,
        })
        .await?;
        Ok(NextAction::Continue)
    }

    async fn render_on_behavior_switch_input_text(
        &self,
        prev_name: &str,
        next: &BehaviorCfg,
        from_context_report: Option<&str>,
        from_context: Option<serde_json::Value>,
    ) -> RenderedUserMessage {
        let Some(template) = next.prompt.on_behavior_switch.as_deref().map(str::trim) else {
            return RenderedUserMessage::NotConfigured;
        };
        if template.is_empty() {
            return RenderedUserMessage::NotConfigured;
        }
        let mut env = self.build_prompt_env(next).await;
        let pending = self.meta.lock().await.pending_inputs.clone();
        let hook_env = self
            .apply_hook(
                SessionHookPoint::OnBehaviorSwitch,
                &self.session_class_driver(),
                &pending,
            )
            .await;
        apply_hook_env_to_prompt_env(&mut env, hook_env);
        info!(
            "opendan.session[{}]: build user message hook=on_behavior_switch from=`{}` to=`{}` pending_total={} env_msgs={} env_events={} template_chars={}",
            self.session_id,
            prev_name,
            next.meta.name,
            pending.len(),
            env.llm_context.msgs.len(),
            env.llm_context.events.len(),
            template.chars().count()
        );
        let report = from_context_report
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let from_context = from_context.unwrap_or_else(|| {
            report
                .map(|value| {
                    serde_json::json!({
                        "last_report": value,
                        "report": value,
                    })
                })
                .unwrap_or(serde_json::Value::Null)
        });
        let to_context = prompt_env::context_snapshot_prompt_value_from_env(&env);
        let extras = [
            (
                "switch",
                serde_json::json!({
                    "from": prev_name,
                    "to": next.meta.name.clone(),
                    "from_context": from_context.clone(),
                    "to_context": to_context.clone(),
                }),
            ),
            (
                "from_behavior",
                serde_json::Value::String(prev_name.to_string()),
            ),
            ("from_context", from_context),
            ("to_context", to_context),
        ];
        match prompt_env::render_template(template, &env, &extras).await {
            Ok(text) => {
                let text = text.trim().to_string();
                if text.is_empty() {
                    warn!(
                        "opendan.session[{}]: rendered empty user message hook=on_behavior_switch from=`{}` to=`{}`; skipping next inference",
                        self.session_id, prev_name, next.meta.name
                    );
                    return RenderedUserMessage::Empty;
                }
                info!(
                    "opendan.session[{}]: rendered user message hook=on_behavior_switch from=`{}` to=`{}` chars={} preview=`{}`",
                    self.session_id,
                    prev_name,
                    next.meta.name,
                    text.chars().count(),
                    text_log_preview(&text)
                );
                RenderedUserMessage::Text(text)
            }
            Err(err) => {
                warn!(
                    "opendan.session[{}]: render on_behavior_switch for behavior `{}` failed: {err}",
                    self.session_id, next.meta.name
                );
                RenderedUserMessage::NotConfigured
            }
        }
    }

    async fn render_on_wakeup_input_text(
        &self,
        llm_context: LlmContextEnv,
    ) -> Result<RenderedUserMessage> {
        let behavior = self.load_current_behavior().await?;
        let Some(template) = behavior.prompt.on_wakeup.as_deref().map(str::trim) else {
            return Ok(RenderedUserMessage::NotConfigured);
        };
        if template.is_empty() {
            return Ok(RenderedUserMessage::NotConfigured);
        }
        let mut env = self.build_prompt_env(&behavior).await;
        apply_hook_env_to_prompt_env(&mut env, llm_context);
        info!(
            "opendan.session[{}]: build user message hook=on_wakeup behavior=`{}` env_msgs={} env_events={} template_chars={}",
            self.session_id,
            behavior.meta.name,
            env.llm_context.msgs.len(),
            env.llm_context.events.len(),
            template.chars().count()
        );
        let rendered = prompt_env::render_template(template, &env, &[])
            .await
            .map_err(|err| {
                anyhow!(
                    "render prompt.on_wakeup for behavior `{}` failed: {err}",
                    behavior.meta.name
                )
            })?;
        let rendered = rendered.trim().to_string();
        if !rendered.is_empty() {
            info!(
                "opendan.session[{}]: rendered user message hook=on_wakeup behavior=`{}` chars={} preview=`{}`",
                self.session_id,
                behavior.meta.name,
                rendered.chars().count(),
                text_log_preview(&rendered)
            );
            Ok(RenderedUserMessage::Text(rendered))
        } else {
            warn!(
                "opendan.session[{}]: rendered empty user message hook=on_wakeup behavior=`{}`; skipping next inference",
                self.session_id, behavior.meta.name
            );
            Ok(RenderedUserMessage::Empty)
        }
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
    ) -> Result<()> {
        let new_system = self.render_system_messages(new_cfg).await?;
        let overrides = RequestOverrides {
            system_messages: Some(new_system),
            user_messages: None,
            tool_policy: Some(new_cfg.to_tool_policy()),
            objective: Some(new_cfg.meta.objective.clone()),
            behavior_name: Some(new_cfg.meta.name.clone()),
            model_policy: Some(self.model_policy_for_behavior(new_cfg).await),
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
        Ok(())
    }

    /// Switch mode = Fork: parent process is suspended and the child behavior
    /// runs as a one-shot subtask. The child inherits the parent's interpreted
    /// step records, but starts with the child's own system prompt and hot tail.
    /// On child `END`, the parent snapshot is restored and the child report is
    /// handed back as the parent's next input.
    async fn apply_switch_fork(
        &self,
        new_cfg: &BehaviorCfg,
        final_snapshot: LLMContextSnapshot,
    ) -> Result<()> {
        let (parent_entry, parent_current) = {
            let meta = self.meta.lock().await;
            (meta.process_entry.clone(), meta.current_behavior.clone())
        };
        let parent_path = self.behavior_snap_path(&parent_entry)?;
        self.persist_snapshot_to(&parent_path, &final_snapshot)
            .await;

        let request = self.fresh_request_for(new_cfg).await?;
        let mut state = LLMContextState::from_request(&request, now_ms());
        state.steps = final_snapshot.state.steps;
        if let Some(last) = final_snapshot.state.last_step {
            state.steps.push(last);
        }
        state.history_summaries = final_snapshot.state.history_summaries;
        state.next_step_index = final_snapshot.state.next_step_index;
        state.next_action_id = final_snapshot.state.next_action_id;

        self.persist_snapshot(&LLMContextSnapshot { request, state })
            .await;

        {
            let mut meta = self.meta.lock().await;
            meta.process_stack.push(ProcessFrame {
                entry: parent_entry,
                current: parent_current,
                fork: true,
            });
            meta.process_entry = new_cfg.meta.name.clone();
            meta.current_behavior = new_cfg.meta.name.clone();
        }
        Ok(())
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
            let request = self.fresh_request_for(new_cfg).await?;
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
                fork: false,
            });
            meta.process_entry = new_cfg.meta.name.clone();
            meta.current_behavior = new_cfg.meta.name.clone();
        }
        Ok(())
    }

    /// Drive the independent-mode call-stack pop on `END`. If a parent
    /// frame is waiting, persist this process's terminal state (so a future
    /// re-entry resumes its stream), restore the parent's snapshot to
    /// `state.snap`, and persist an internal continuation for the parent's next
    /// turn. This hand-off is session state, not external pending input.
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

        let child_report = final_snapshot.state.last_report.clone().unwrap_or_default();

        // Independent children keep their own stream for future re-entry.
        // Fork children are one-shot calls; only their report is returned to
        // the parent, so their internal stream is intentionally discarded.
        if !parent_frame.fork {
            if let Ok(child_path) = self.behavior_snap_path(&child_entry) {
                self.persist_snapshot_to(&child_path, &final_snapshot).await;
            }
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

        let on_behavior_switch_text = if parent_frame.fork {
            match self.agent_config.load_behavior(&parent_frame.current) {
                Ok(parent_cfg) => self
                    .render_on_behavior_switch_input_text(
                        child_entry.as_str(),
                        &parent_cfg,
                        Some(child_report.as_str()),
                        None,
                    )
                    .await
                    .into_text(),
                Err(err) => {
                    warn!(
                        "opendan.session[{}]: load parent behavior `{}` after fork pop failed: {err:#}",
                        self.session_id, parent_frame.current
                    );
                    None
                }
            }
        } else {
            None
        };

        let (reason, user_messages) = if let Some(text) = on_behavior_switch_text {
            (
                format!("process_return:{}->{}", child_entry, parent_frame.current),
                vec![AiMessage::text(AiRole::User, text)],
            )
        } else {
            let marker_text = if parent_frame.fork {
                fork_child_end_marker(&child_entry, &child_report)
            } else {
                format!("[independent process `{}` ended]", child_entry)
            };
            (
                format!("process_return:{}->{}", child_entry, parent_frame.current),
                vec![AiMessage::text(AiRole::User, marker_text)],
            )
        };
        self.set_internal_continuation(InternalContinuation {
            reason,
            user_messages,
        })
        .await?;
        Ok(NextAction::Continue)
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
        self.fork_and_run_with_loop_mode(
            overrides,
            sub_behavior_name,
            self.session_class_loop_mode(),
        )
        .await
    }

    pub async fn fork_and_run_agent_loop(
        &self,
        overrides: RequestOverrides,
        sub_behavior_name: &str,
    ) -> Result<ContextOutput> {
        self.fork_and_run_with_loop_mode(overrides, sub_behavior_name, LoopMode::Agent)
            .await
    }

    async fn fork_and_run_with_loop_mode(
        &self,
        overrides: RequestOverrides,
        sub_behavior_name: &str,
        loop_mode: LoopMode,
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
            overrides.system_messages = Some(self.render_system_messages(&sub_cfg).await?);
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
            i18n: self.agent_config.i18n.clone(),
            state_snap_path: &self.state_snap_path,
            parent_snap,
            overrides,
            sub_cfg: &sub_cfg,
            loop_mode,
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
            .and_then(|g| g.as_ref().map(AiMessage::text_content))
            .filter(|s| !s.trim().is_empty())
    }

    pub fn current_origin_user_ai_message(&self) -> Option<AiMessage> {
        self.current_origin_msg.lock().ok().and_then(|g| g.clone())
    }

    /// Worker-internal: stash / clear the per-turn origin message. Pass
    /// `Some(message)` right before running a turn; `None` to clear (e.g. on
    /// session exit).
    fn set_current_origin_msg(&self, value: Option<AiMessage>) {
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
        let one_line_status = self.status.snapshot();
        {
            let mut g = self.meta.lock().await;
            g.status = status;
            g.status_changed_at_ms = now_ms();
            g.one_line_status = one_line_status.clone();
        }
        let typing = matches!(status, SessionStatus::Running | SessionStatus::WaitingTool);
        self.status
            .update_ui_state(UI_SESSION_STATE_TYPING_KEY, serde_json::json!(typing));
        self.status.update_ui_state(
            UI_SESSION_STATE_STATUS_LINE_KEY,
            self.status.status_line_value(one_line_status.clone()),
        );
        if let Err(err) = self.flush_meta().await {
            warn!(
                "opendan.session[{}]: flush after status set failed: {err:#}",
                self.session_id
            );
        }
        self.mirror_status_to_task(status, one_line_status).await;
    }

    async fn mirror_status_to_task(&self, status: SessionStatus, one_line_status: String) {
        let cfg = self.session_class_driver().task_feedback;
        if !cfg.enabled || !self.kind.is_work_family() {
            return;
        }
        let Some(binding) = self.task_binding().await else {
            return;
        };
        let Ok(client) = self.runtime.task_mgr_client().await else {
            return;
        };
        let Ok(task) = client.get_task(&binding.task_id).await else {
            return;
        };
        if task.phase.is_terminal() || task.phase == TaskPhase::Paused {
            return;
        }
        let message = task_message_for_session_status(status, &one_line_status);
        let status_text = session_status_text(status);
        let patch = json!({
            "agent_delegate": {
                "execution": {
                    "session_id": self.session_id,
                    "workspace_id": self.workspace_id().await,
                    "session_status": status_text,
                    "status": status_text,
                    "one_line_status": one_line_status,
                    "updated_at_ms": now_ms(),
                }
            }
        });
        let mut merged = crate::task_util::task_payload(&task);
        crate::task_util::merge_json(&mut merged, &patch);
        let result = match status {
            SessionStatus::Ended => {
                crate::task_util::commit_task_result(&client, task, merged).await
            }
            SessionStatus::Error => {
                crate::task_util::fail_own_task(
                    &client,
                    task,
                    "agent_session_failed",
                    one_line_status.clone(),
                    Some(merged),
                )
                .await
            }
            SessionStatus::Running | SessionStatus::WaitingTool => {
                match crate::task_util::ensure_running(&client, task).await {
                    Ok(task) => {
                        crate::task_util::report_progress_value(&client, task, Some(merged), message)
                            .await
                    }
                    Err(err) => Err(err),
                }
            }
            SessionStatus::WaitingInput => {
                match crate::task_util::report_progress_value(&client, task, Some(merged), message)
                    .await
                {
                    Ok(task) if task.phase == TaskPhase::Running
                        || task.phase == TaskPhase::Accepted =>
                    {
                        crate::task_util::report_waiting_reason(
                            &client,
                            task,
                            buckyos_api::TaskWaitReason::with_code(
                                buckyos_api::TaskWaitReasonKind::HumanInput,
                                "agent_wait_user_msg",
                            ),
                        )
                        .await
                    }
                    other => other,
                }
            }
            SessionStatus::Idle => {
                crate::task_util::report_progress_value(&client, task, Some(merged), message).await
            }
        };
        if let Err(err) = result {
            warn!(
                "opendan.session[{}]: mirror status {:?} to task {} failed: {err:#}",
                self.session_id, status, binding.task_id
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
                    mode: EventSubscriptionMode::Full,
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

    pub(crate) async fn post_outbound_error(&self, error: &str) {
        self.status.set_with_nonce(
            self.agent_config.i18n.render("status.llm_failed", &[]),
            self.status.nonce_snapshot(),
        );
        let text = self
            .agent_config
            .i18n
            .render("response.failed", &[("error", error.trim().to_string())]);
        self.post_outbound_message(&AiMessage::text(AiRole::Assistant, text))
            .await;
    }

    async fn post_outbound_message(&self, message: &AiMessage) {
        // UI sessions are the only ones that reply through msg-center
        // today — work sessions surface their result via report.md instead.
        if !matches!(self.kind, SessionKind::Ui) {
            return;
        }
        let Ok(msg_center) = self.runtime.msg_center_client().await else {
            return;
        };
        let peer_did_str = {
            let meta = self.meta.lock().await;
            meta.peer_did.clone()
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
        // `peer_did` is the session's stored peer DID — a determined target
        // (second-level `did:msgtunnel:*` endpoint for tunnel traffic) that
        // carries its own routing. No preferred_tunnel / route hint needed.

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
        if let Some(turn_nonce) = self.status.nonce_snapshot() {
            msg.meta.insert(
                "turn_nonce".to_string(),
                serde_json::Value::String(turn_nonce),
            );
        }
        msg.meta.insert(
            "delivery_failure_notice".to_string(),
            serde_json::Value::String(
                self.agent_config
                    .i18n
                    .render("response.delivery_failed", &[]),
            ),
        );

        let workspace_id = {
            let meta = self.meta.lock().await;
            meta.workspace_id.clone()
        };
        let workspace_dir = workspace_id
            .as_deref()
            .map(|ws| self.agent_config.layout.workspaces_dir.join(ws));
        let validator = crate::attachment_policy::WorkspaceAttachmentValidator::with_policy(
            workspace_dir.clone(),
            self.agent_name.clone(),
            self.agent_config.toml.runtime.filesystem_policy,
        );
        let resolver = crate::attachment_resolver::NamedStoreLocalLinkResolver::new(
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
        let fallback_msg = msg.clone();
        let msg = match llm_context::ai_message_to_msg_object_with_base_validated_async(
            message,
            msg,
            &validator,
            egress_options,
            &resolver,
        )
        .await
        {
            Ok(msg) => msg,
            Err(err) => {
                warn!(
                    "opendan.session[{}]: outbound message conversion failed: {err}",
                    self.session_id
                );
                self.post_delivery_failure_notice(&msg_center, fallback_msg)
                    .await;
                return;
            }
        };
        if msg.content.content.trim().is_empty()
            && msg.content.refs.is_empty()
            && msg.content.machine.is_none()
        {
            return;
        }

        match msg_center.post_send(msg, None).await {
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

    async fn post_delivery_failure_notice(
        &self,
        msg_center: &buckyos_api::MsgCenterClient,
        mut msg: ndn_lib::MsgObject,
    ) {
        msg.content = ndn_lib::MsgContent {
            format: Some(ndn_lib::MsgContentFormat::TextPlain),
            content: self
                .agent_config
                .i18n
                .render("response.delivery_failed", &[]),
            ..Default::default()
        };
        msg.meta.remove("delivery_failure_notice");
        msg.meta.insert(
            "delivery_failure_fallback".to_string(),
            serde_json::Value::Bool(true),
        );
        let idempotency_key = Some(format!(
            "delivery-failure:{}:{}",
            self.session_id, msg.created_at_ms
        ));
        if let Err(err) = msg_center.post_send(msg, idempotency_key).await {
            warn!(
                "opendan.session[{}]: delivery failure notice post_send failed: {err}",
                self.session_id
            );
        }
    }

    async fn post_worksession_report_outbound(
        &self,
        data: &serde_json::Value,
        idempotency_key: Option<String>,
    ) -> Result<bool> {
        if !matches!(self.kind, SessionKind::Ui) {
            return Ok(false);
        }
        let target_meta = self.meta.lock().await.clone();
        self.post_worksession_report_outbound_to_ui_meta(
            &self.session_id,
            &target_meta,
            data,
            idempotency_key,
        )
        .await
    }

    async fn post_worksession_report_outbound_to_persisted_ui(
        &self,
        target_session_id: &str,
        data: &serde_json::Value,
        idempotency_key: Option<String>,
    ) -> Result<bool> {
        let meta_path = self
            .agent_config
            .layout
            .session_dir(target_session_id)
            .join(".meta")
            .join("session.json");
        let bytes = match tokio::fs::read(&meta_path).await {
            Ok(bytes) => bytes,
            Err(err) => {
                warn!(
                    "opendan.session[{}]: worksession report target UI session {} not found: read {} failed: {err}",
                    self.session_id,
                    target_session_id,
                    meta_path.display()
                );
                return Ok(false);
            }
        };
        let target_meta: SessionMeta = match serde_json::from_slice(&bytes) {
            Ok(meta) => meta,
            Err(err) => {
                warn!(
                    "opendan.session[{}]: worksession report target UI session {} invalid: parse {} failed: {err}",
                    self.session_id,
                    target_session_id,
                    meta_path.display()
                );
                return Ok(false);
            }
        };
        self.post_worksession_report_outbound_to_ui_meta(
            target_session_id,
            &target_meta,
            data,
            idempotency_key,
        )
        .await
    }

    async fn post_worksession_report_outbound_to_ui_meta(
        &self,
        target_session_id: &str,
        target_meta: &SessionMeta,
        data: &serde_json::Value,
        idempotency_key: Option<String>,
    ) -> Result<bool> {
        if !matches!(target_meta.kind, SessionKind::Ui) {
            warn!(
                "opendan.session[{}]: worksession report target {} is not a UI session",
                self.session_id, target_session_id
            );
            return Ok(false);
        }
        let Ok(msg_center) = self.runtime.msg_center_client().await else {
            return Ok(false);
        };
        let peer_did_raw = target_meta.peer_did.clone();
        let Some(peer_did_raw) = peer_did_raw.as_deref().filter(|s| !s.trim().is_empty()) else {
            warn!(
                "opendan.session[{}]: cannot post worksession report — UI session has no peer DID",
                target_session_id
            );
            return Ok(false);
        };
        let Ok(peer_did) = name_lib::DID::from_str(peer_did_raw) else {
            warn!(
                "opendan.session[{}]: cannot post worksession report — invalid peer DID `{}`",
                target_session_id, peer_did_raw
            );
            return Ok(false);
        };
        let agent_did_raw = self.agent_config.toml.identity.agent_did.trim();
        if agent_did_raw.is_empty() {
            return Ok(false);
        }
        let Ok(agent_did) = name_lib::DID::from_str(agent_did_raw) else {
            warn!(
                "opendan.session[{}]: cannot post worksession report — invalid agent DID `{}`",
                target_session_id, agent_did_raw
            );
            return Ok(false);
        };
        if agent_did == peer_did {
            return Ok(false);
        }
        // `peer_did` (from the target session meta) is a determined endpoint DID
        // that carries its own routing; the report content lives in `msg`.
        let msg = self
            .build_worksession_report_msg(&agent_did, &peer_did, target_session_id, data)
            .await;
        match msg_center.post_send(msg, idempotency_key).await {
            Ok(result) if result.ok => Ok(true),
            Ok(result) => {
                warn!(
                    "opendan.session[{}]: worksession report outbound rejected — reason={:?}",
                    target_session_id, result.reason
                );
                Ok(false)
            }
            Err(err) => Err(anyhow!("worksession report post_send failed: {err}")),
        }
    }
}

enum NextAction {
    Continue,
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

async fn build_agent_session_env(
    session_id: &str,
    agent_config: &AgentConfig,
    agent_name: &str,
    meta: &Arc<Mutex<SessionMeta>>,
    session_dir: &Path,
    behavior: &BehaviorCfg,
    runtime_last_schedule_task_list_text: String,
) -> AgentSessionEnv {
    let (
        kind,
        title,
        objective,
        owner,
        workspace_id,
        one_line,
        bg_events,
        task_binding,
        workspace_since_secs,
        notebook_since_secs,
    ) = {
        let meta = meta.lock().await;
        (
            meta.kind,
            meta.title.clone(),
            meta.objective.clone(),
            meta.owner.clone(),
            meta.workspace_id.clone(),
            meta.one_line_status.clone(),
            meta.background_events.clone(),
            meta.task_binding.clone(),
            meta.last_workspace_list_access_at,
            meta.last_notebook_last_items_access_at,
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
        .map(|ws| agent_config.layout.workspaces_dir.join(ws));
    let (notebook_list_text, notebook_last_items_text) = build_notebook_prompt_texts(
        agent_config,
        owner.trim(),
        agent_name,
        notebook_since_secs,
        kind,
    );
    let runtime_workspace_list_text =
        build_runtime_workspace_list_text(agent_config, workspace_since_secs).await;
    {
        let now_secs = now_ms() / 1000;
        let mut meta = meta.lock().await;
        meta.last_workspace_list_access_at = now_secs.saturating_sub(1);
        meta.last_notebook_last_items_access_at = now_secs.saturating_sub(1);
    }
    AgentSessionEnv {
        session_id: session_id.to_string(),
        session_kind: AgentSessionEnv::kind_str(kind),
        session_title: title.trim().to_string(),
        session_objective,
        session_owner: owner,
        session_current_todo: load_current_todo(session_dir),
        session_current_todo_list: render_current_todo_list(session_dir),
        session_background_hints: Vec::new(),
        session_default_changed_background_hint_text: String::new(),
        behavior_name: behavior.meta.name.clone(),
        behavior_objective: behavior.meta.objective.clone(),
        behavior_mode: "behavior",
        behavior_template_dir: behavior
            .source_path
            .as_ref()
            .and_then(|path| path.parent().map(|parent| parent.to_path_buf())),
        workspace_id,
        workspace_root,
        agent_root: agent_config.layout.root.clone(),
        session_root: session_dir.to_path_buf(),
        input_text: String::new(),
        input_has_user_text: false,
        input_has_events: false,
        recent_activity: one_line.trim().to_string(),
        clock_unix_ms: now_ms(),
        runtime_workspace_list_text,
        runtime_last_schedule_task_list_text,
        notebook_list_text,
        notebook_last_items_text,
        llm_context: LlmContextEnv {
            bg_events,
            ..Default::default()
        },
        task: task_binding.map(Into::into),
    }
}

async fn build_runtime_workspace_list_text(agent_config: &AgentConfig, since_secs: u64) -> String {
    let manager = LocalWorkspaceManager::new(agent_config.layout.workspaces_dir.clone());
    match manager.list().await {
        Ok(mut workspaces) => {
            if since_secs > 0 {
                let since_ms = since_secs.saturating_mul(1000);
                workspaces.retain(|workspace| workspace.updated_at_ms >= since_ms);
                if workspaces.is_empty() {
                    return "Recent workspace changes: none.\n".to_string();
                }
            }
            workspaces.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms));
            render_workspace_inventory(&workspaces)
        }
        Err(err) => {
            warn!("opendan.session: build workspace list prompt text failed: {err}");
            String::new()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScheduleTaskPromptLine {
    at: u64,
    action: &'static str,
    task_id: String,
    task_title: String,
    note: String,
}

async fn render_last_schedule_task_list_text(
    task_mgr: &TaskManagerClient,
    since_secs: u64,
    now_secs: u64,
) -> std::result::Result<String, String> {
    let page = task_mgr
        .list_tasks(ListTasksReq {
            schema_id: Some("workflow.schedule/v1".to_string()),
            ..Default::default()
        })
        .await
        .map_err(|err| err.to_string())?;
    let mut summaries = page.tasks;
    summaries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    let mut lines = Vec::new();
    for summary in summaries {
        let Ok(task) = task_mgr.get_task(&summary.task_id).await else {
            continue;
        };
        if is_prompt_time_window(task.created_at, since_secs, now_secs) {
            lines.push(ScheduleTaskPromptLine {
                at: task.created_at,
                action: "created",
                task_id: task.task_id.clone(),
                task_title: schedule_task_title(&task),
                note: schedule_task_created_note(&task),
            });
        }

        if task.outcome == Some(TaskOutcome::Failed)
            && is_prompt_time_window(task.updated_at, since_secs, now_secs)
        {
            lines.push(ScheduleTaskPromptLine {
                at: task.updated_at,
                action: "failed",
                task_id: task.task_id.clone(),
                task_title: schedule_task_title(&task),
                note: schedule_task_failure_note(&task),
            });
        }

        match task_mgr.list_task_notes(&task.task_id).await {
            Ok(notes) => {
                for note in notes.into_iter().filter(is_user_manual_task_note) {
                    if is_prompt_time_window(note.created_at, since_secs, now_secs) {
                        lines.push(ScheduleTaskPromptLine {
                            at: note.created_at,
                            action: "user_noted",
                            task_id: task.task_id.clone(),
                            task_title: schedule_task_title(&task),
                            note: schedule_task_note_summary(&note),
                        });
                    }
                }
            }
            Err(err) => {
                warn!(
                    "opendan.session: list schedule task notes failed task_id={}: {err}",
                    task.task_id
                );
            }
        }
    }

    if lines.is_empty() {
        return Ok("Recent schedule tasks: none.".to_string());
    }

    lines.sort_by(|a, b| {
        b.at.cmp(&a.at)
            .then_with(|| a.action.cmp(b.action))
            .then_with(|| a.task_id.cmp(&b.task_id))
    });
    Ok(lines
        .into_iter()
        .map(|line| {
            format!(
                "- {} task_id={} task_title={} note={}",
                line.action,
                line.task_id,
                quote_prompt_text(&line.task_title),
                quote_prompt_text(&line.note)
            )
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

fn is_prompt_time_window(ts_secs: u64, since_secs: u64, now_secs: u64) -> bool {
    ts_secs > since_secs && ts_secs <= now_secs
}

fn schedule_task_title(task: &Task) -> String {
    if let Ok(TypedTaskData::WorkflowSchedule(data)) =
        parse_typed_task_data("workflow/schedule", crate::task_util::task_payload(task))
    {
        if let Some(name) = data.request.name.filter(|name| !name.trim().is_empty()) {
            return name.trim().to_string();
        }
    }
    task.name
        .strip_prefix("workflow/schedule/")
        .unwrap_or(task.name.as_str())
        .trim()
        .to_string()
}

fn schedule_task_created_note(task: &Task) -> String {
    let mut sources = Vec::new();
    collect_notebook_source_fragments(&crate::task_util::task_payload(task), &mut sources);
    if notebook_source_like(&task.name) {
        sources.push(task.name.clone());
    }
    if let Some(message) = task
        .message
        .as_deref()
        .filter(|value| notebook_source_like(value))
    {
        sources.push(message.to_string());
    }
    dedup_prompt_fragments(&mut sources);
    if sources.is_empty() {
        "not create from Notebook Item".to_string()
    } else {
        format!(
            "create from Notebook Item {}",
            truncate_prompt_text(&sources.join("; "), 240)
        )
    }
}

fn schedule_task_failure_note(task: &Task) -> String {
    if let Ok(TypedTaskData::WorkflowSchedule(data)) =
        parse_typed_task_data("workflow/schedule", crate::task_util::task_payload(task))
    {
        if let Some(result) = data.result {
            if let Some(last_error) = result.last_error {
                return format!("last run failed: {}", value_prompt_text(&last_error));
            }
        }
    }
    task.message
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|message| format!("last run failed: {}", truncate_prompt_text(message, 240)))
        .unwrap_or_else(|| "last run failed".to_string())
}

fn schedule_task_note_summary(note: &TaskNote) -> String {
    let author = if note.author_user_id.trim().is_empty() {
        "user".to_string()
    } else {
        note.author_user_id.trim().to_string()
    };
    format!(
        "{} noted at {}: {}",
        author,
        note.created_at,
        truncate_prompt_text(&note.content, 240)
    )
}

fn is_user_manual_task_note(note: &TaskNote) -> bool {
    matches!(
        note.note_type.trim().to_ascii_lowercase().as_str(),
        "human" | "manual" | "user" | "user_noted"
    )
}

fn collect_notebook_source_fragments(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(text) => {
            if notebook_source_like(text) {
                out.push(truncate_prompt_text(text, 240));
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_notebook_source_fragments(item, out);
            }
        }
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                let key_lower = key.to_ascii_lowercase();
                if key_lower.contains("notebook") || key_lower.contains("note_item") {
                    out.push(format!("{}={}", key, value_prompt_text(value)));
                }
                collect_notebook_source_fragments(value, out);
            }
        }
        _ => {}
    }
}

fn notebook_source_like(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("notebook item")
        || lower.contains("source notebook")
        || lower.contains("source_notebook")
        || lower.contains("notebook_item")
        || lower.contains("note_")
}

fn dedup_prompt_fragments(values: &mut Vec<String>) {
    let mut seen = HashSet::new();
    values.retain(|value| seen.insert(value.clone()));
}

fn value_prompt_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => truncate_prompt_text(text, 240),
        serde_json::Value::Null => "null".to_string(),
        other => truncate_prompt_text(&other.to_string(), 240),
    }
}

fn quote_prompt_text(value: &str) -> String {
    format!("\"{}\"", truncate_prompt_text(value, 240).replace('"', "'"))
}

fn truncate_prompt_text(value: &str, max_chars: usize) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = compact.chars();
    let truncated = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

fn build_notebook_prompt_texts(
    agent_config: &AgentConfig,
    owner: &str,
    agent_name: &str,
    recent_since_secs: u64,
    session_kind: SessionKind,
) -> (String, String) {
    let owner = resolve_notebook_prompt_owner(owner, agent_name);
    if owner.is_empty() || !agent_config.layout.notebook_dir.exists() {
        return (
            "Available notebooks: 0 total.\n".to_string(),
            "Recent notebook items: none.\n".to_string(),
        );
    }

    let notebook = match AgentNotebook::open(AgentNotebookConfig::new(
        agent_config.layout.notebook_dir.clone(),
    )) {
        Ok(notebook) => notebook,
        Err(err) => {
            warn!("opendan.session: open notebook for prompt env failed: {err}");
            return (String::new(), String::new());
        }
    };
    let list_text = match notebook.build_notebook_list_text() {
        Ok(text) => text,
        Err(err) => {
            warn!("opendan.session: build notebook list prompt text failed: {err}");
            String::new()
        }
    };
    let include_unreviewed = matches!(session_kind, SessionKind::SelfCheck);
    let last_items_result = if include_unreviewed {
        notebook.build_recent_items_text_for_self_check(8, recent_since_secs)
    } else {
        notebook.build_recent_items_text(8, recent_since_secs)
    };
    let last_items_text = match last_items_result {
        Ok(text) => text,
        Err(err) => {
            warn!("opendan.session: build notebook recent items prompt text failed: {err}");
            String::new()
        }
    };
    (list_text, last_items_text)
}

/// Max `updated_at` (unix secs) of any active Notebook item for this agent, or
/// 0 when the notebook is empty/absent or unreadable. Cheap probe used by the
/// Self-Check wakeup gate to detect change without building the prompt.
fn notebook_latest_active_update_secs(
    agent_config: &AgentConfig,
    owner: &str,
    agent_name: &str,
) -> u64 {
    let owner = resolve_notebook_prompt_owner(owner, agent_name);
    if owner.is_empty() || !agent_config.layout.notebook_dir.exists() {
        return 0;
    }
    let notebook = match AgentNotebook::open(AgentNotebookConfig::new(
        agent_config.layout.notebook_dir.clone(),
    )) {
        Ok(notebook) => notebook,
        Err(err) => {
            warn!("opendan.session: open notebook for self_check gate failed: {err}");
            return 0;
        }
    };
    match notebook.latest_active_item_update_secs() {
        Ok(secs) => secs,
        Err(err) => {
            warn!(
                "opendan.session: query notebook latest update for self_check gate failed: {err}"
            );
            0
        }
    }
}

fn resolve_notebook_prompt_owner(session_owner: &str, agent_appid: &str) -> String {
    let appid = agent_appid.trim();
    if appid.is_empty() {
        session_owner.trim().to_string()
    } else {
        appid.to_string()
    }
}

fn apply_hook_env_to_prompt_env(env: &mut AgentSessionEnv, hook_env: LlmContextEnv) {
    env.session_background_hints = hook_env.background_hints.clone();
    env.session_default_changed_background_hint_text =
        hook_env.default_changed_background_hint_text.clone();
    env.llm_context = hook_env;
}

fn background_hint_interval_active(last_non_empty_at_ms: u64, now_ms: u64) -> bool {
    last_non_empty_at_ms > 0
        && now_ms.saturating_sub(last_non_empty_at_ms) < BACKGROUND_HINT_NON_EMPTY_INTERVAL_MS
}

fn build_background_event_hints(events: &[BgEventSnapshot]) -> Vec<BackgroundHint> {
    let mut events = events.to_vec();
    events.sort_by(|a, b| a.event_id.cmp(&b.event_id));
    events
        .into_iter()
        .map(|event| {
            let data = serde_json::to_value(&event).unwrap_or(serde_json::Value::Null);
            let fingerprint = stable_json_fingerprint(&data);
            let reason = background_event_reason(&event);
            let event_data =
                serde_json::to_string(&event.data).unwrap_or_else(|_| String::from("null"));
            BackgroundHint {
                path: format!("event/{}", event.event_id),
                kind: "event".to_string(),
                text: format!("{} updated : {} ({})", reason, event_data, event.event_id),
                fingerprint,
                data,
            }
        })
        .collect()
}

fn background_event_reason(event: &BgEventSnapshot) -> String {
    if let Some(reason) = event
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return reason.to_string();
    }

    for key in ["reason", "purpose", "type"] {
        if let Some(reason) = event
            .data
            .get(key)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return reason.to_string();
        }
    }

    "event".to_string()
}

fn render_changed_background_hint_text(hints: &[BackgroundHint]) -> String {
    hints
        .iter()
        .map(|hint| {
            let text = hint.text.trim();
            if text.is_empty() {
                hint.path.trim()
            } else {
                text
            }
        })
        .filter(|text| !text.is_empty())
        .map(|text| format!("- {text}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Debug, Clone, Default)]
struct SessionTopicState {
    title: String,
    tags: TagSet,
}

fn load_session_topic_state(session_dir: &Path) -> SessionTopicState {
    let topic_path = session_dir.join(".meta").join("topic.md");
    let topic_doc = read_topic_doc(&topic_path).ok();
    let title = topic_doc
        .as_ref()
        .map(|doc| doc.topic.clone())
        .unwrap_or_default();

    let tag_set_path = session_dir.join(".meta").join("tag_set.json");
    if let Ok(text) = std::fs::read_to_string(&tag_set_path) {
        if let Ok(tag_set) = serde_json::from_str::<TagSet>(&text) {
            if !tag_set.tags.is_empty() {
                return SessionTopicState {
                    title,
                    tags: tag_set,
                };
            }
        }
    }

    let tags = topic_doc
        .map(|doc| doc.tags)
        .unwrap_or_else(|| load_session_topic_tags(session_dir))
        .into_iter()
        .map(|name| TagEntry {
            name,
            weight: 1.0,
            last_touched: String::new(),
            tier: TagTier::Transient,
            position: 0,
            reason: None,
        })
        .collect();
    SessionTopicState {
        title,
        tags: TagSet {
            tags,
            ..TagSet::default()
        },
    }
}

fn recall_items_to_background_hints(items: &[RecallItem]) -> Vec<BackgroundHint> {
    items.iter().map(background_hint_from_recall_item).collect()
}

fn background_hint_from_recall_item(item: &RecallItem) -> BackgroundHint {
    let source = recall_source_label(item);
    let hint_type = recall_hint_type_label(item);
    let data = serde_json::to_value(item).unwrap_or(serde_json::Value::Null);
    let fingerprint = recall_item_background_fingerprint(item);
    BackgroundHint {
        path: format!(
            "{source}/{hint_type}/{}/{}",
            item.target.kind, item.target.id
        ),
        kind: source,
        text: format!(
            "[{}/{}] {} (reason: {}; action: {})",
            recall_source_label(item),
            hint_type,
            item.hint,
            item.reason,
            item.suggested_action
        ),
        fingerprint,
        data,
    }
}

fn recall_item_background_fingerprint(item: &RecallItem) -> String {
    let data = serde_json::json!({
        "source_system": item.source_system,
        "hint_type": item.hint_type,
        "target": item.target,
        "matched_tags": item.matched_tags,
        "version": item.debug.get("version"),
        "fingerprint": item.debug.get("fingerprint"),
        "updated_at": item.debug.get("updated_at"),
        "noticed_at": item.debug.get("noticed_at"),
        "hint": item.hint,
        "reason": item.reason,
        "suggested_action": item.suggested_action,
    });
    stable_json_fingerprint(&data)
}

fn changed_background_hints(
    old_fingerprints: &BTreeMap<String, String>,
    candidates: Vec<BackgroundHint>,
) -> (Vec<BackgroundHint>, BTreeMap<String, String>) {
    let mut next_fingerprints = BTreeMap::new();
    let mut changed = Vec::new();
    for hint in candidates {
        let previous = old_fingerprints.get(&hint.path);
        next_fingerprints.insert(hint.path.clone(), hint.fingerprint.clone());
        if previous != Some(&hint.fingerprint) {
            changed.push(hint);
        }
    }
    (changed, next_fingerprints)
}

fn recall_source_label(item: &RecallItem) -> String {
    serde_json::to_value(&item.source_system)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| format!("{:?}", item.source_system).to_ascii_lowercase())
}

fn recall_hint_type_label(item: &RecallItem) -> String {
    serde_json::to_value(&item.hint_type)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| format!("{:?}", item.hint_type).to_ascii_lowercase())
}

fn load_session_topic_tags(session_dir: &Path) -> Vec<String> {
    let tag_set_path = session_dir.join(".meta").join("tag_set.json");
    if let Ok(value) = read_json_file(&tag_set_path) {
        let tags = value
            .get("tags")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.get("name").and_then(|value| value.as_str()))
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if !tags.is_empty() {
            return tags;
        }
    }

    let topic_path = session_dir.join(".meta").join("topic.md");
    let Ok(text) = std::fs::read_to_string(topic_path) else {
        return Vec::new();
    };
    parse_topic_doc_tags(&text)
}

fn parse_topic_doc_tags(text: &str) -> Vec<String> {
    if !text.starts_with("---\n") {
        return Vec::new();
    }
    let Some(rest) = text.strip_prefix("---\n") else {
        return Vec::new();
    };
    let Some((frontmatter, _body)) = rest.split_once("\n---") else {
        return Vec::new();
    };
    for line in frontmatter.lines() {
        let Some(raw) = line.trim().strip_prefix("tags:") else {
            continue;
        };
        let Ok(tags) = serde_json::from_str::<Vec<String>>(raw.trim()) else {
            return Vec::new();
        };
        return tags
            .into_iter()
            .map(|tag| tag.trim().to_string())
            .filter(|tag| !tag.is_empty())
            .collect();
    }
    Vec::new()
}

fn read_json_file(path: &Path) -> std::io::Result<serde_json::Value> {
    let text = std::fs::read_to_string(path)?;
    serde_json::from_str(&text)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))
}

fn stable_json_fingerprint(value: &serde_json::Value) -> String {
    let text = serde_json::to_string(value).unwrap_or_default();
    stable_report_hash(&text)
}

fn merge_global_state_hook_stats(
    mut state: serde_json::Value,
    hook_point: &str,
    pulled_msg_count: usize,
    pulled_event_count: usize,
) -> serde_json::Value {
    let stats = serde_json::json!({
        "hook_point": hook_point,
        "pulled_msg_count": pulled_msg_count,
        "pulled_event_count": pulled_event_count,
    });
    match &mut state {
        serde_json::Value::Object(map) => {
            map.insert("driver".to_string(), stats);
            state
        }
        _ => serde_json::json!({
            "value": state,
            "driver": stats,
        }),
    }
}

#[derive(Default)]
struct PendingInputStats {
    msg: usize,
    event: usize,
    interrupt: usize,
}

fn pending_input_stats(inputs: &[PendingInput]) -> PendingInputStats {
    let mut stats = PendingInputStats::default();
    for input in inputs {
        match input {
            PendingInput::Msg { .. } => stats.msg += 1,
            PendingInput::Event { .. } => stats.event += 1,
            PendingInput::Interrupt { .. } => stats.interrupt += 1,
        }
    }
    stats
}

fn first_non_empty_line(value: &str) -> Option<&str> {
    value.lines().map(str::trim).find(|line| !line.is_empty())
}

fn append_turn_message_to_snapshot(
    mut snapshot: LLMContextSnapshot,
    message: Option<AiMessage>,
    history_inputs: Vec<HistoryInputRecord>,
    trace_id: &str,
    preserve_behavior_state: bool,
) -> LLMContextSnapshot {
    snapshot.request.trace = Some(trace_id.to_string());

    let previous_state = snapshot.state;
    let state = if preserve_behavior_state {
        let mut state = LLMContextState::from_request(&snapshot.request, now_ms());
        let mut steps = previous_state.steps;
        let mut last_step = previous_state.last_step;
        if last_step.is_none()
            && steps
                .last()
                .is_some_and(|step| step.meta.behavior_name == snapshot.request.behavior_name)
        {
            last_step = steps.pop();
        }
        // Behavior-switch pair (Agent Context Messages.md §状态机切换): when
        // the resolved hot tail ended with `<next_behavior>`, the incoming
        // user message IS that step's on_behavior_switch payload, not a
        // free-floating next-turn input. Attach it as `next_user_message`
        // so the rendered shape is `[assistant:next_behavior=X user:on_switch]`
        // instead of `[assistant:next_behavior=X user:empty][user:on_switch]`
        // (two consecutive user messages — the bug).
        if let Some(message) = message {
            let attach_to_switch_step = last_step
                .as_ref()
                .is_some_and(|s| s.next_behavior.is_some() && s.next_user_message.is_none());
            if attach_to_switch_step {
                if let Some(last) = last_step.as_mut() {
                    last.next_user_message = Some(message);
                }
            } else {
                state.accumulated.push(message);
            }
        }
        state.steps = steps;
        state.history_summaries = previous_state.history_summaries;
        state.history_inputs = previous_state.history_inputs;
        state.history_inputs.extend(history_inputs);
        state.last_step = last_step;
        state.last_report = previous_state.last_report;
        state.next_step_index = previous_state.next_step_index;
        state.next_action_id = previous_state.next_action_id;
        state
    } else {
        let mut input = previous_state.accumulated;
        if let Some(message) = message {
            input.push(message);
        }
        snapshot.request.input = input;
        let mut state = LLMContextState::from_request(&snapshot.request, now_ms());
        state.history_inputs = previous_state.history_inputs;
        state.history_inputs.extend(history_inputs);
        state
    };
    snapshot.state = state;
    snapshot
}

fn is_runtime_auto_user_pending(from: &str) -> bool {
    from == "opendan:on_behavior_switch"
}

fn prune_legacy_internal_pending_inputs(pending: &mut Vec<PendingInput>) -> usize {
    let before = pending.len();
    pending.retain(|input| {
        let is_legacy_internal = matches!(
            input,
            PendingInput::Msg { from, .. } if is_runtime_auto_user_pending(from)
        ) || matches!(
            input,
            PendingInput::Msg { record_id, .. } if is_history_input_pending(record_id)
        );
        !is_legacy_internal
    });
    before.saturating_sub(pending.len())
}

fn is_history_input_pending(record_id: &str) -> bool {
    record_id.starts_with("process-end:")
}

fn fork_child_end_marker(child_entry: &str, child_report: &str) -> String {
    let report = child_report.trim();
    if report.is_empty() {
        return format!("[fork process `{child_entry}` ended]");
    }
    format!("[fork process `{child_entry}` ended]\n\n## Child Report:\n{report}")
}

fn worksession_report_delivery_allows(
    mode: ReportDeliveryMode,
    phase: WorksessionReportPhase,
    context_depth: usize,
) -> bool {
    match mode {
        ReportDeliveryMode::FinalOnly => {
            context_depth == 0 && matches!(phase, WorksessionReportPhase::Final)
        }
        ReportDeliveryMode::TopLevel => context_depth == 0,
        ReportDeliveryMode::All => true,
    }
}

fn stable_report_hash(report: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in report.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionEndDisposition {
    Idle,
    Ended,
}

fn session_end_disposition(kind: SessionKind) -> SessionEndDisposition {
    match kind {
        SessionKind::SelfCheck => SessionEndDisposition::Idle,
        _ => SessionEndDisposition::Ended,
    }
}

fn idle_worker_retire_ms(kind: SessionKind) -> u64 {
    if matches!(kind, SessionKind::Ui) {
        UI_IDLE_WORKER_RETIRE_MS
    } else {
        DEFAULT_IDLE_WORKER_RETIRE_MS
    }
}

pub(crate) fn model_policy_with_session_profile(
    mut policy: ModelPolicy,
    session_id: &str,
    session_profile: Option<&SessionLogicalProfile>,
) -> ModelPolicy {
    let mut options = match policy.provider_options.take() {
        Some(serde_json::Value::Object(map)) => map,
        Some(other) => {
            let mut map = serde_json::Map::new();
            map.insert("provider_options".to_string(), other);
            map
        }
        None => serde_json::Map::new(),
    };
    options.insert(
        "session_id".to_string(),
        serde_json::Value::String(session_id.to_string()),
    );
    if let Some(profile) = session_profile {
        options.insert(
            "session_profile".to_string(),
            serde_json::to_value(profile).unwrap_or(serde_json::Value::Null),
        );
    }
    policy.provider_options = Some(serde_json::Value::Object(options));
    policy
}

impl AgentSession {
    async fn build_worksession_report_msg(
        &self,
        agent_did: &name_lib::DID,
        peer_did: &name_lib::DID,
        target_session_id: &str,
        data: &serde_json::Value,
    ) -> MsgObject {
        let content_title = worksession_report_content_title(data);
        let mut msg =
            build_worksession_report_base_msg(agent_did, peer_did, target_session_id, data);
        let report = normalize_report_attachment_tags(&json_str(data, "report"));
        let workspace_id = json_str(data, "workspace_id");
        let workspace_dir = if workspace_id.is_empty() {
            None
        } else {
            Some(self.agent_config.layout.workspaces_dir.join(workspace_id))
        };
        let validator = crate::attachment_policy::WorkspaceAttachmentValidator::with_policy(
            workspace_dir.clone(),
            self.agent_name.clone(),
            self.agent_config.toml.runtime.filesystem_policy,
        );
        let resolver = crate::attachment_resolver::NamedStoreLocalLinkResolver::new(
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
        match llm_context::ai_message_to_msg_object_with_base_validated_async(
            &AiMessage::text(AiRole::Assistant, report),
            msg.clone(),
            &validator,
            egress_options,
            &resolver,
        )
        .await
        {
            Ok(mut converted) => {
                converted.content.title = Some(content_title);
                converted
            }
            Err(err) => {
                warn!(
                    "opendan.session[{}]: worksession report conversion failed: {err}",
                    self.session_id
                );
                msg.content.content = normalize_report_attachment_tags(&json_str(data, "report"));
                msg.content.title = Some(content_title);
                msg
            }
        }
    }
}

fn build_worksession_report_base_msg(
    agent_did: &name_lib::DID,
    peer_did: &name_lib::DID,
    target_session_id: &str,
    data: &serde_json::Value,
) -> MsgObject {
    let title = json_str(data, "title");
    let source_session_id = json_str(data, "source_session_id");
    let mut msg = MsgObject {
        from: agent_did.clone(),
        to: vec![peer_did.clone()],
        kind: MsgObjKind::Chat,
        created_at_ms: data
            .get("created_at_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or_else(now_ms),
        content: MsgContent::default(),
        ..MsgObject::default()
    };
    msg.thread.topic = Some(target_session_id.to_string());
    msg.thread.correlation_id = Some(source_session_id.clone());
    msg.meta.insert(
        "llm_role".to_string(),
        serde_json::Value::String(WORKSESSION_REPORT_EVENT_TYPE.to_string()),
    );
    msg.meta.insert(
        "message_type".to_string(),
        serde_json::Value::String(WORKSESSION_REPORT_EVENT_TYPE.to_string()),
    );
    msg.meta.insert(
        "session_id".to_string(),
        serde_json::Value::String(target_session_id.to_string()),
    );
    msg.meta.insert(
        "owner_session_id".to_string(),
        serde_json::Value::String(target_session_id.to_string()),
    );
    msg.meta.insert(
        "source_session_id".to_string(),
        serde_json::Value::String(source_session_id.clone()),
    );
    msg.meta.insert(
        "source_kind".to_string(),
        serde_json::Value::String("worksession".to_string()),
    );
    msg.meta.insert(
        "source".to_string(),
        serde_json::json!({
            "kind": "worksession",
            "session_id": source_session_id,
            "title": title,
            "objective": data.get("objective").cloned().unwrap_or(serde_json::Value::Null),
            "workspace_id": data.get("workspace_id").cloned().unwrap_or(serde_json::Value::Null),
            "behavior": data.get("behavior").cloned().unwrap_or(serde_json::Value::Null),
        }),
    );
    for key in [
        "report_id",
        "phase",
        "is_final",
        "context_depth",
        "process_entry",
        "parent_process_entry",
        "trace_id",
    ] {
        if let Some(value) = data.get(key) {
            msg.meta.insert(key.to_string(), value.clone());
        }
    }
    msg
}

fn worksession_report_content_title(data: &serde_json::Value) -> String {
    let title = json_str(data, "title");
    let source_session_id = json_str(data, "source_session_id");
    if title.is_empty() {
        format!("WorkSession report: {source_session_id}")
    } else {
        format!("WorkSession report: {title}")
    }
}

fn normalize_report_attachment_tags(report: &str) -> String {
    report
        .replace("</attachement>", "</attachment>")
        .replace("<attachement", "<attachment")
        .lines()
        .map(normalize_report_attachment_line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalize_report_attachment_line(line: &str) -> String {
    let trimmed = line.trim();
    let Some(body) = trimmed.strip_prefix("<attachment>") else {
        return line.to_string();
    };
    let Some(body) = body.strip_suffix("<attachment>") else {
        return line.to_string();
    };
    if body.contains("</attachment>") {
        return line.to_string();
    }
    let indent_len = line.len() - line.trim_start().len();
    let indent = &line[..indent_len];
    format!("{indent}<attachment>{}</attachment>", body.trim())
}

fn json_str(data: &serde_json::Value, key: &str) -> String {
    data.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn task_message_for_session_status(status: SessionStatus, one_line_status: &str) -> Option<String> {
    let fallback = match status {
        SessionStatus::Idle => "Agent session idle",
        SessionStatus::Running => "Agent session running",
        SessionStatus::WaitingInput => "Agent session waiting for input",
        SessionStatus::WaitingTool => "Agent session waiting for tool",
        SessionStatus::Ended => "Agent session completed",
        SessionStatus::Error => "Agent session failed",
    };
    let message = one_line_status.trim();
    Some(if message.is_empty() {
        fallback.to_string()
    } else {
        message.to_string()
    })
}

fn session_status_text(status: SessionStatus) -> &'static str {
    match status {
        SessionStatus::Idle => "idle",
        SessionStatus::Running => "running",
        SessionStatus::WaitingInput => "waiting_input",
        SessionStatus::WaitingTool => "waiting_tool",
        SessionStatus::Ended => "ended",
        SessionStatus::Error => "error",
    }
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
                tool_result: None,
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
                tool_result: None,
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

fn event_matches_pull_policy(
    event_id: &str,
    policy: &PullEventPolicy,
    subscriptions: &[EventSubscription],
) -> bool {
    match policy {
        PullEventPolicy::None => false,
        PullEventPolicy::All => true,
        PullEventPolicy::Filter(name) => {
            let filter = name
                .strip_suffix(".*")
                .or_else(|| name.strip_suffix("/**"))
                .unwrap_or(name);
            event_id == filter
                || event_id.starts_with(&format!("{filter}."))
                || event_id.starts_with(&format!("{filter}/"))
                || subscriptions
                    .iter()
                    .any(|sub| sub.mode == EventSubscriptionMode::Full && sub.matches(event_id))
        }
    }
}

fn select_pending_for_hook_with_subscriptions(
    cfg: &HookPointCfg,
    pending: &[PendingInput],
    pending_task_index: &std::collections::HashMap<String, PendingTaskCall>,
    subscriptions: &[EventSubscription],
) -> Vec<PendingInput> {
    let mut out = Vec::new();
    let msg_limit = match cfg.pull_msg {
        PullMsgPolicy::None => Some(0usize),
        PullMsgPolicy::One => Some(1usize),
        PullMsgPolicy::All => None,
    };
    let mut msg_count = 0usize;
    for input in pending {
        match input {
            PendingInput::Msg { .. } => {
                if msg_limit.is_some_and(|limit| msg_count >= limit) {
                    continue;
                }
                msg_count += 1;
                out.push(input.clone());
            }
            PendingInput::Event { event_id, .. } => {
                if pending_task_index.contains_key(event_id)
                    || event_matches_pull_policy(event_id, &cfg.pull_event, subscriptions)
                {
                    out.push(input.clone());
                }
            }
            PendingInput::Interrupt { .. } => {}
        }
    }
    out
}

fn hook_filter_allows(
    filter: &BehaviorFilter,
    current_behavior: &str,
    default_behavior: &str,
    process_stack: &[ProcessFrame],
) -> bool {
    match filter {
        BehaviorFilter::None => false,
        BehaviorFilter::All => true,
        BehaviorFilter::Top => process_stack.is_empty(),
        BehaviorFilter::DefaultOnly => {
            process_stack.is_empty() && current_behavior == default_behavior
        }
        BehaviorFilter::Behavior(name) => current_behavior == name,
    }
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

fn load_current_todo(session_dir: &Path) -> serde_json::Value {
    let path = session_dir.join("todos.json");
    let todos = match read_todo_records(&path) {
        Ok(todos) => todos,
        _ => return serde_json::Value::Null,
    };
    todos
        .iter()
        .find(|todo| !todo.status.is_terminal())
        .and_then(|todo| serde_json::to_value(todo).ok())
        .unwrap_or(serde_json::Value::Null)
}

fn render_current_todo_list(session_dir: &Path) -> String {
    let path = session_dir.join("todos.json");
    let todos = match read_todo_records(&path) {
        Ok(todos) => todos,
        Err(err) => return format!("(invalid todos.json: {err})"),
    };
    if todos.is_empty() {
        return "(empty)".to_string();
    }
    let current_id = todos
        .iter()
        .find(|todo| !todo.status.is_terminal())
        .map(|todo| todo.todo_id.clone());
    let mut lines = Vec::with_capacity(todos.len());
    for todo in todos {
        let marker = if current_id.as_deref() == Some(todo.todo_id.as_str()) {
            " current"
        } else {
            ""
        };
        lines.push(format!(
            "- {} [{}{marker}] {}",
            todo.todo_id,
            todo_status_label(&todo),
            todo_title_for_display(&todo)
        ));
    }
    lines.join("\n")
}

fn todo_title_for_display(todo: &TodoRecord) -> &str {
    let title = todo.title.trim();
    if title.is_empty() {
        todo.content.trim()
    } else {
        title
    }
}

fn todo_status_label(todo: &TodoRecord) -> String {
    serde_json::to_value(todo.status)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| format!("{:?}", todo.status))
}

#[derive(Debug)]
struct TemplateExpression {
    expr: String,
    line: usize,
    raw: String,
}

fn render_template_failure_detail(
    behavior: &BehaviorCfg,
    field: &str,
    template: &str,
    env: &AgentSessionEnv,
    err: &dyn std::fmt::Display,
) -> String {
    let source_path = behavior
        .source_path
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<builtin>".to_string());
    let value_start_line = behavior
        .source_path
        .as_ref()
        .and_then(|path| toml_field_value_start_line(path, "prompt", field.rsplit('.').next()?));

    let expressions = extract_template_expressions(template);
    let mut hints = Vec::new();
    if env.session_current_todo.is_null() {
        for expr in &expressions {
            if expression_primary_path(&expr.expr).starts_with("session.current_todo.") {
                let loc = template_expr_location(&source_path, value_start_line, expr.line);
                hints.push(format!(
                    "likely null access: `session.current_todo` is null, but {loc} uses `{}`",
                    expr.raw
                ));
            }
        }
    }

    let location = value_start_line
        .map(|line| format!("{source_path}:{line}"))
        .unwrap_or(source_path);
    let mut detail = format!(
        "behavior=`{}`, field=`{field}`, template={location}, error={err}",
        behavior.meta.name
    );
    if !hints.is_empty() {
        detail.push_str(", ");
        detail.push_str(&hints.join("; "));
    }
    detail
}

fn template_expr_location(
    source_path: &str,
    value_start_line: Option<usize>,
    expr_line: usize,
) -> String {
    match value_start_line {
        Some(start) => format!("{source_path}:{}", start + expr_line.saturating_sub(1)),
        None => source_path.to_string(),
    }
}

fn toml_field_value_start_line(path: &Path, table: &str, field: &str) -> Option<usize> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut in_table = false;
    for (idx, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_table = trimmed == format!("[{table}]");
            continue;
        }
        if !in_table {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        if key.trim() != field {
            continue;
        }
        let line_number = idx + 1;
        return if value.trim_start().starts_with("\"\"\"") {
            Some(line_number + 1)
        } else {
            Some(line_number)
        };
    }
    None
}

fn extract_template_expressions(template: &str) -> Vec<TemplateExpression> {
    let mut out = Vec::new();
    for (line_idx, line) in template.lines().enumerate() {
        let mut rest = line;
        while let Some(start) = rest.find("{{") {
            let after_start = &rest[start + 2..];
            let Some(end) = after_start.find("}}") else {
                break;
            };
            let raw = &rest[start..start + 2 + end + 2];
            let expr = after_start[..end].trim();
            if !expr.is_empty() {
                out.push(TemplateExpression {
                    expr: expr.to_string(),
                    line: line_idx + 1,
                    raw: raw.to_string(),
                });
            }
            rest = &after_start[end + 2..];
        }
    }
    out
}

fn expression_primary_path(expr: &str) -> &str {
    expr.split(|ch: char| ch.is_whitespace() || ch == '|' || ch == ')')
        .next()
        .unwrap_or("")
        .trim_start_matches('(')
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

fn text_log_preview(text: &str) -> String {
    trigger_preview(&text.split_whitespace().collect::<Vec<_>>().join(" "))
}

fn message_text_chars(message: &AiMessage) -> usize {
    message.text_content().chars().count()
}

fn ai_message_log_preview(message: &AiMessage) -> String {
    let text = message.text_content();
    let trimmed = text.trim();
    if !trimmed.is_empty() {
        return text_log_preview(trimmed);
    }
    pending_msg_preview("", message)
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

fn ratio_budget(context_window_tokens: u32, ratio: f32) -> u32 {
    ((context_window_tokens as f64) * (ratio as f64))
        .ceil()
        .clamp(1.0, u32::MAX as f64) as u32
}

fn estimate_history_tokens(deps: &llm_context::deps::LLMContextDeps, msgs: &[AiMessage]) -> u32 {
    msgs.iter().fold(0u32, |total, msg| {
        total
            .saturating_add(deps.tokenizer.count_tokens(msg.role.as_str()))
            .saturating_add(deps.tokenizer.count_tokens(&msg.render_for_debug()))
    })
}

fn llm_message_compress_extra_focus(agent_name: &str, behavior: &BehaviorCfg) -> String {
    format!(
        "Agent identity: {agent_name}\nBehavior: {}\nObjective: {}",
        behavior.meta.name.trim(),
        behavior.meta.objective.trim()
    )
}

fn leading_system_messages(msgs: &[AiMessage]) -> usize {
    msgs.iter()
        .take_while(|msg| matches!(msg.role, AiRole::System | AiRole::Developer))
        .count()
}

fn turns_since_last_llm_message_compress(msgs: &[AiMessage]) -> u32 {
    let start = msgs
        .iter()
        .rposition(is_llm_message_compress_marker)
        .map(|idx| idx + 1)
        .unwrap_or(0);
    msgs[start..]
        .iter()
        .filter(|msg| matches!(msg.role, AiRole::User) && !is_llm_message_compress_marker(msg))
        .count()
        .min(u32::MAX as usize) as u32
}

fn is_llm_message_compress_marker(msg: &AiMessage) -> bool {
    let text = msg.text_content();
    text.contains("[LLM_MESSAGE_COMPRESS_META_V1]")
        || text.contains("[LLM_MESSAGE_COMPRESS_SUMMARY_V1]")
        || text.contains("[Conversation summary]")
}

fn context_window_tokens_from_model_directory(
    directory: &serde_json::Value,
    alias: &str,
) -> Option<u32> {
    let aliases = model_directory_alias_targets(directory, alias);
    let mut exact = Vec::new();
    let mut logical = Vec::new();
    let providers = directory.get("providers")?.as_array()?;
    for provider in providers {
        let Some(models) = provider.get("models").and_then(|value| value.as_array()) else {
            continue;
        };
        for model in models {
            let Some(tokens) = model
                .pointer("/capabilities/max_context_tokens")
                .and_then(|value| value.as_u64())
            else {
                continue;
            };
            if tokens == 0 {
                continue;
            }
            let exact_model = model
                .get("exact_model")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            let provider_model_id = model
                .get("provider_model_id")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            if aliases
                .iter()
                .any(|item| item == exact_model || item == provider_model_id)
            {
                exact.push(tokens);
                continue;
            }
            let has_logical_mount = model
                .get("logical_mounts")
                .and_then(|value| value.as_array())
                .map(|mounts| {
                    mounts.iter().any(|mount| {
                        mount
                            .as_str()
                            .map(|mount| aliases.iter().any(|item| item == mount))
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false);
            if has_logical_mount {
                logical.push(tokens);
            }
        }
    }
    exact
        .into_iter()
        .chain(logical)
        .min()
        .map(|tokens| tokens.min(u32::MAX as u64) as u32)
}

fn model_directory_alias_targets(directory: &serde_json::Value, alias: &str) -> Vec<String> {
    let mut out = vec![alias.to_string()];
    let mut cursor = 0usize;
    while cursor < out.len() && out.len() < 64 {
        let current = out[cursor].clone();
        cursor += 1;
        if let Some(items) = directory
            .get("directory")
            .and_then(|value| value.get(&current))
            .and_then(|value| value.as_object())
        {
            for target in items
                .values()
                .filter_map(|item| item.get("target").and_then(|value| value.as_str()))
            {
                if !out.iter().any(|item| item == target) {
                    out.push(target.to_string());
                }
            }
        }
    }
    out
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
/// Note: this is an opendan-level compressor (message dimension). The
/// behavior loop itself only appends and renders persisted step history; it
/// does not run a separate step compressor.
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

fn pending_input_hook_value(input: &PendingInput) -> serde_json::Value {
    match input {
        PendingInput::Msg {
            record_id,
            from,
            from_did,
            from_name,
            tunnel_did,
            text,
            ai_message,
        } => serde_json::json!({
            "kind": "msg",
            "key": input.dedup_key(),
            "record_id": record_id,
            "from": from,
            "from_did": from_did,
            "from_name": from_name,
            "tunnel_did": tunnel_did,
            "text": text,
            "message_text": ai_message.text_content(),
        }),
        PendingInput::Event { event_id, data } => serde_json::json!({
            "kind": "event",
            "key": input.dedup_key(),
            "event_id": event_id,
            "data": data,
        }),
        PendingInput::Interrupt { mode, id } => serde_json::json!({
            "kind": "interrupt",
            "key": input.dedup_key(),
            "mode": mode,
            "id": id,
        }),
    }
}

fn render_pending_input_values(values: &[serde_json::Value]) -> String {
    if values.is_empty() {
        return String::new();
    }
    values
        .iter()
        .filter_map(|value| {
            let kind = value.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            match kind {
                "msg" => {
                    let from = value.get("from").and_then(|v| v.as_str()).unwrap_or("");
                    let text = value
                        .get("text")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                        .or_else(|| value.get("message_text").and_then(|v| v.as_str()))
                        .unwrap_or("");
                    Some(format!("- msg from {from}: {}", text.trim()))
                }
                "event" => {
                    let event_id = value.get("event_id").and_then(|v| v.as_str()).unwrap_or("");
                    Some(format!("- event {event_id}: {}", value["data"]))
                }
                "interrupt" => {
                    let mode = value.get("mode").map(|v| v.to_string()).unwrap_or_default();
                    Some(format!("- interrupt {mode}"))
                }
                _ => None,
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
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

fn compose_turn_message(messages: &[AiMessage]) -> Option<AiMessage> {
    if messages.is_empty() {
        return None;
    }
    let mut blocks = Vec::new();
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

fn prepare_turn_messages_for_run(
    messages: Vec<TurnMessage>,
    embed_user_supplement: bool,
) -> Vec<AiMessage> {
    if !embed_user_supplement {
        return messages.into_iter().map(|entry| entry.message).collect();
    }

    let has_runtime_auto = messages.iter().any(|entry| entry.runtime_auto);
    let user_messages = messages
        .iter()
        .filter(|entry| !entry.runtime_auto)
        .map(|entry| &entry.message)
        .collect::<Vec<_>>();
    if !has_runtime_auto
        || user_messages.is_empty()
        || !user_messages
            .iter()
            .all(|message| is_plain_text_user_message(message))
    {
        return messages.into_iter().map(|entry| entry.message).collect();
    }

    let Some(supplement) = render_user_supplement_section(&user_messages) else {
        return messages.into_iter().map(|entry| entry.message).collect();
    };

    let mut injected = false;
    messages
        .into_iter()
        .filter_map(|entry| {
            if entry.runtime_auto {
                if injected {
                    Some(entry.message)
                } else {
                    injected = true;
                    Some(inject_user_supplement_into_message(
                        entry.message,
                        &supplement,
                    ))
                }
            } else {
                None
            }
        })
        .collect()
}

fn is_plain_text_user_message(message: &AiMessage) -> bool {
    message.content.iter().all(|block| match block {
        AiContent::Text { .. } => true,
        AiContent::Thinking { .. } | AiContent::ProviderState { .. } => true,
        AiContent::Image { .. }
        | AiContent::Document { .. }
        | AiContent::ToolUse { .. }
        | AiContent::ToolResult { .. } => false,
    })
}

fn render_user_supplement_section(messages: &[&AiMessage]) -> Option<String> {
    let text = messages
        .iter()
        .map(|message| message.text_content())
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    if text.is_empty() {
        None
    } else {
        Some(format!("## 刚刚用户补充的信息\n\n{text}"))
    }
}

fn inject_user_supplement_into_message(mut message: AiMessage, supplement: &str) -> AiMessage {
    for block in &mut message.content {
        let AiContent::Text { text } = block else {
            continue;
        };
        *text = inject_user_supplement_into_text(text, supplement);
        return message;
    }
    message.content.insert(0, AiContent::text(supplement));
    message
}

fn inject_user_supplement_into_text(text: &str, supplement: &str) -> String {
    const MARKERS: [&str; 2] = ["Continue from PROCESS_RULES.", "Continue TASK_ANCHOR."];
    for marker in MARKERS {
        if let Some(pos) = text.find(marker) {
            let before = text[..pos].trim_end();
            let after = &text[pos..];
            return format!("{before}\n\n{supplement}\n\n{after}");
        }
    }

    let trimmed = text.trim_end();
    if trimmed.is_empty() {
        supplement.to_string()
    } else {
        format!("{trimmed}\n\n{supplement}")
    }
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
