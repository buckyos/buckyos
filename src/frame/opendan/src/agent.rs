use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use buckyos_api::{
    AiccClient, BoxKind, MsgCenterClient, MsgObject, MsgRecordWithObject, MsgState, SendContext,
    TaskManagerClient,
};
use log::{debug, error, info, warn};
use name_lib::DID;
use ndn_lib::ObjId;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use tokio::fs;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinSet;

use crate::agent_config::AIAgentConfig;
use crate::agent_enviroment::AgentEnvironment;
use crate::agent_memory::{AgentMemory, AgentMemoryConfig, TOOL_LOAD_MEMORY};
use crate::agent_session::{AgentSession, AgentSessionConfig};
use crate::agent_tool::{AgentPolicy, ToolCall, ToolError, ToolManager, ToolSpec};
use crate::ai_runtime::{AiRuntime, AiRuntimeConfig};
use crate::behavior::{
    AgentWorkEvent, BehaviorConfig, BehaviorConfigError, BehaviorExecInput, BehaviorLLMResult,
    EnvKV, LLMBehavior, LLMBehaviorDeps, LLMOutput, LLMTrackingInfo, Observation,
    ObservationSource, PolicyEngine, Sanitizer, TokenUsage, Tokenizer, TraceCtx, WorklogSink,
};
use crate::workspace::{TOOL_EXEC_BASH, TOOL_WORKLOG_MANAGE};

const AGENT_DOC_CANDIDATES: [&str; 2] = ["agent.json.doc", "Agent.json.doc"];
const DEFAULT_SESSION_LOOP_STATE_FILE: &str = "session/session_loop_state.json";
const DEFAULT_SLEEP_REASON: &str = "no_new_input";
const MSG_CENTER_INBOX_PULL_LIMIT: usize = 32;
const BEHAVIOR_RESOLVE_ROUTER: &str = "resolve_router";
const BEHAVIOR_ROUTER_PASS: &str = "router_pass";
const BEHAVIOR_ON_WAKEUP: &str = "on_wakeup";
const MAX_RECENT_TURNS: usize = 6;
const MAX_ROUTER_TOOL_CALLS: usize = 8;
static REPLY_MSG_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(thiserror::Error, Debug)]
pub enum AIAgentError {
    #[error("invalid agent config: {0}")]
    InvalidConfig(String),
    #[error("io error on `{path}`: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse json `{path}`: {source}")]
    Json {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("agent tool error: {0}")]
    Tool(#[from] ToolError),
    #[error("behavior config error: {0}")]
    BehaviorConfig(#[from] BehaviorConfigError),
    #[error("runtime error: {0}")]
    Runtime(#[from] crate::ai_runtime::AiRuntimeError),
    #[error("llm behavior failed: {0}")]
    LLMBehavior(String),
}

#[derive(Clone)]
pub struct AIAgentDeps {
    pub taskmgr: Arc<TaskManagerClient>,
    pub aicc: Arc<AiccClient>,
    pub msg_center: Option<Arc<MsgCenterClient>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WakeupStatus {
    SkippedNoInput,
    Completed,
    SafeStop,
    Error,
    Disabled,
}

#[derive(Clone, Debug, Serialize)]
pub struct WakeupReport {
    pub wakeup_id: String,
    pub trace_id: String,
    pub status: WakeupStatus,
    pub final_behavior: String,
    pub steps: u32,
    pub behavior_hops: u32,
    pub hp_before: u32,
    pub hp_after: u32,
    pub token_prompt: u32,
    pub token_completion: u32,
    pub token_total: u32,
    pub sleep_ms: u64,
    pub reason: Json,
    pub last_error: Option<String>,
}

#[derive(Debug)]
struct AIAgentState {
    enabled: bool,
    hp: u32,
    next_sleep_ms: u64,
    wakeup_seq: u64,
    inbox_msgs: VecDeque<Json>,
    queued_events: VecDeque<Json>,
    last_wakeup_ms: Option<u64>,
    session_runtime_states: HashMap<String, SessionRuntimeState>,
    active_session_loop: Option<String>,
}

impl AIAgentState {
    fn new(cfg: &AIAgentConfig) -> Self {
        Self {
            enabled: true,
            hp: cfg.hp_max,
            next_sleep_ms: cfg.default_sleep_ms,
            wakeup_seq: 0,
            inbox_msgs: VecDeque::new(),
            queued_events: VecDeque::new(),
            last_wakeup_ms: None,
            session_runtime_states: HashMap::new(),
            active_session_loop: None,
        }
    }
}

pub struct AIAgent {
    cfg: AIAgentConfig,
    deps: AIAgentDeps,
    did: String,
    did_document: Json,
    role_md: String,
    self_md: String,
    behaviors_dir: PathBuf,
    behavior_cfg_cache: Arc<RwLock<HashMap<String, BehaviorConfig>>>,
    tool_mgr: Arc<ToolManager>,
    environment: AgentEnvironment,
    memory: AgentMemory,
    policy: Arc<dyn PolicyEngine>,
    worklog: Arc<dyn WorklogSink>,
    tokenizer: Arc<dyn Tokenizer>,
    session_runtime_store_path: PathBuf,
    state: Mutex<AIAgentState>,
}

/*
Main Loop更清晰
核心要点
- 构造wakeup的input,并处理rouer result
- 构造behavior step的input,并处理BehaviorLLMResult
- Agent Loop 关注激活Session的调度
- Session里面关注切换不同的Behavior和执行Behavior Step，并保存状态
*/
impl AIAgent {
    pub async fn load(mut cfg: AIAgentConfig, deps: AIAgentDeps) -> Result<Self, AIAgentError> {
        info!("ai_agent.load start: root={}", cfg.agent_root.display());
        cfg.normalize().map_err(AIAgentError::InvalidConfig)?;
        let agent_root = normalize_agent_root(&cfg.agent_root).await?;
        cfg.agent_root = agent_root.clone();

        let did_document = load_agent_doc(&agent_root).await?;
        let did = extract_agent_did(&did_document, &agent_root);

        let role_md = load_text_or_empty(agent_root.join(&cfg.role_file_name)).await?;
        let self_md = load_text_or_empty(agent_root.join(&cfg.self_file_name)).await?;

        let behaviors_dir = agent_root.join(&cfg.behaviors_dir_name);
        fs::create_dir_all(&behaviors_dir)
            .await
            .map_err(|source| AIAgentError::Io {
                path: behaviors_dir.display().to_string(),
                source,
            })?;

        let environment_root = agent_root.join(&cfg.environment_dir_name);
        let environment = AgentEnvironment::new(environment_root.clone()).await?;
        let session_runtime_store_path = environment_root.join(DEFAULT_SESSION_LOOP_STATE_FILE);
        let session = AgentSession::new(
            AgentSessionConfig::new(&environment_root),
            deps.msg_center.clone(),
        )
        .await?;
        let memory = AgentMemory::new(AgentMemoryConfig::new(&agent_root)).await?;

        let tool_mgr = Arc::new(ToolManager::new());
        environment.register_workshop_tools(tool_mgr.as_ref())?;
        session.register_tools(tool_mgr.as_ref())?;
        memory.register_tools(tool_mgr.as_ref())?;
        let runtime = AiRuntime::new(AiRuntimeConfig::new(&cfg.agent_root)).await?;
        runtime.register_agent(&did, &cfg.agent_root).await?;
        runtime.register_tools(tool_mgr.as_ref()).await?;

        let behavior_cfg_cache = Arc::new(RwLock::new(HashMap::<String, BehaviorConfig>::new()));
        preload_behavior_configs(&behaviors_dir, behavior_cfg_cache.clone()).await?;

        let policy = Arc::new(AgentPolicy::new(
            tool_mgr.clone(),
            behavior_cfg_cache.clone(),
        ));
        let worklog_path = environment
            .workspace_root()
            .join(&cfg.worklog_file_rel_path);
        let worklog = Arc::new(JsonlFileWorklogSink::new(worklog_path).await?);
        let tokenizer = Arc::new(WhitespaceTokenizer {});
        let runtime_store =
            load_session_runtime_store(&session_runtime_store_path, did.as_str()).await;
        let mut initial_state = AIAgentState::new(&cfg);
        initial_state.session_runtime_states = runtime_store.sessions;
        initial_state.active_session_loop = None;

        let behavior_count = behavior_cfg_cache.read().await.len();
        let tool_count = tool_mgr.list_tool_specs().len();
        info!(
            "ai_agent.load ok: did={} root={} behaviors={} tools={} workspace_root={}",
            did,
            cfg.agent_root.display(),
            behavior_count,
            tool_count,
            environment.workspace_root().display()
        );

        Ok(Self {
            cfg: cfg.clone(),
            deps,
            did,
            did_document,
            role_md,
            self_md,
            behaviors_dir,
            behavior_cfg_cache,
            tool_mgr,
            environment,
            memory,
            policy,
            worklog,
            tokenizer,
            session_runtime_store_path,
            state: Mutex::new(initial_state),
        })
    }

    pub fn did(&self) -> &str {
        &self.did
    }

    pub fn did_document(&self) -> &Json {
        &self.did_document
    }

    pub fn agent_root(&self) -> &Path {
        &self.cfg.agent_root
    }

    pub fn environment_root(&self) -> &Path {
        self.environment.workspace_root()
    }

    pub fn memory_dir(&self) -> &Path {
        self.memory.memory_dir()
    }

    pub async fn list_behavior_names(&self) -> Vec<String> {
        let guard = self.behavior_cfg_cache.read().await;
        let mut names: Vec<String> = guard.keys().cloned().collect();
        names.sort();
        names
    }

    pub fn list_tool_specs(&self) -> Vec<ToolSpec> {
        self.tool_mgr.list_tool_specs()
    }

    fn is_sub_agent(&self) -> bool {
        self.did_document
            .get("kind")
            .and_then(|v| v.as_str())
            .map(|v| v == "sub-agent")
            .unwrap_or(false)
            || self.did_document.get("parent_did").is_some()
    }

    pub async fn enable(&self) {
        let mut guard = self.state.lock().await;
        guard.enabled = true;
        if guard.hp == 0 {
            guard.hp = self.cfg.hp_floor.max(1);
        }
        info!(
            "ai_agent.enable: did={} hp={} hp_floor={}",
            self.did, guard.hp, self.cfg.hp_floor
        );
    }

    pub async fn disable(&self) {
        let mut guard = self.state.lock().await;
        guard.enabled = false;
        info!("ai_agent.disable: did={} hp={}", self.did, guard.hp);
        drop(guard);
        if self.is_sub_agent() {
            let ctx = TraceCtx {
                trace_id: format!("{}:disable:{}", self.did, now_ms()),
                agent_did: self.did.clone(),
                behavior: "on_wakeup".to_string(),
                step_idx: 0,
                wakeup_id: format!("disable-{}", now_ms()),
            };
            append_workspace_worklog_entry(
                self.tool_mgr.clone(),
                &ctx,
                "sub_agent_disabled",
                "info",
                "sub-agent disabled".to_string(),
                json!({"reason":"runtime.disable"}),
                None,
                vec!["sub_agent".to_string(), "disable".to_string()],
                None,
                None,
            )
            .await;
        }
    }

    pub async fn push_inbox_message(&self, msg: Json) {
        let inbox_len = {
            let mut guard = self.state.lock().await;
            guard.inbox_msgs.push_back(msg.clone());
            guard.inbox_msgs.len()
        };
        debug!(
            "ai_agent.push_inbox_message: did={} inbox_len={}",
            self.did, inbox_len
        );
        let ctx = TraceCtx {
            trace_id: format!("{}:inbox:{}", self.did, now_ms()),
            agent_did: self.did.clone(),
            behavior: "on_wakeup".to_string(),
            step_idx: 0,
            wakeup_id: format!("inbox-{}", now_ms()),
        };
        let session_id = extract_session_id_from_message_payload(&msg);
        append_workspace_worklog_entry(
            self.tool_mgr.clone(),
            &ctx,
            "message_reply",
            "info",
            "received runtime inbox message".to_string(),
            json!({
                "source": "runtime.push_inbox_message",
                "inbox_len": inbox_len,
                "message": compact_json_for_worklog(msg, 4 * 1024)
            }),
            session_id,
            vec!["message".to_string(), "recv".to_string()],
            None,
            None,
        )
        .await;
    }

    pub async fn push_event(&self, event: Json) {
        let mut guard = self.state.lock().await;
        guard.queued_events.push_back(event);
        debug!(
            "ai_agent.push_event: did={} events_len={}",
            self.did,
            guard.queued_events.len()
        );
    }

    pub async fn run_agent_loop(&self, max_wakeups: Option<u32>) -> Result<(), AIAgentError> {
        info!(
            "ai_agent.run_agent_loop: did={} max_wakeups={:?}",
            self.did, max_wakeups
        );
        let mut rounds = 0_u32;
        loop {
            //wait for events or inbox messages
            let report = self.wait_wakeup(None).await?;
            rounds = rounds.saturating_add(1);
            match report.status {
                WakeupStatus::Error => {
                    error!(
                        "ai_agent.wait_wakeup report: did={} wakeup_id={} status=error behavior={} steps={} hops={} hp={}=>{} tokens={} sleep_ms={} err={:?}",
                        self.did,
                        report.wakeup_id,
                        report.final_behavior,
                        report.steps,
                        report.behavior_hops,
                        report.hp_before,
                        report.hp_after,
                        report.token_total,
                        report.sleep_ms,
                        report.last_error
                    );
                }
                WakeupStatus::SafeStop => {
                    warn!(
                        "ai_agent.wait_wakeup report: did={} wakeup_id={} status=safe_stop behavior={} steps={} hops={} hp={}=>{} tokens={} sleep_ms={} err={:?}",
                        self.did,
                        report.wakeup_id,
                        report.final_behavior,
                        report.steps,
                        report.behavior_hops,
                        report.hp_before,
                        report.hp_after,
                        report.token_total,
                        report.sleep_ms,
                        report.last_error
                    );
                }
                _ => {
                    info!(
                        "ai_agent.wait_wakeup report: did={} wakeup_id={} status={} behavior={} steps={} hops={} hp={}=>{} tokens={} sleep_ms={}",
                        self.did,
                        report.wakeup_id,
                        wakeup_status_name(&report.status),
                        report.final_behavior,
                        report.steps,
                        report.behavior_hops,
                        report.hp_before,
                        report.hp_after,
                        report.token_total,
                        report.sleep_ms
                    );
                }
            }

            let should_break = matches!(report.status, WakeupStatus::Disabled);
            if should_break {
                info!(
                    "ai_agent.start stop: did={} wakeup_id={} reason=disabled",
                    self.did, report.wakeup_id
                );
                break;
            }
            tokio::time::sleep(Duration::from_millis(report.sleep_ms)).await;
        }
        info!("ai_agent.start exit: did={} rounds={}", self.did, rounds);
        Ok(())
    }

    pub async fn wait_wakeup(&self, reason: Option<Json>) -> Result<WakeupReport, AIAgentError> {
        let now = now_ms();
        let explicit_reason = reason.is_some();
        info!(
            "ai_agent.wait_wakeup start: did={} explicit_reason={}",
            self.did, explicit_reason
        );

        // Step 1: wait_for_event (collect external input or explicit trigger)
        self.log_sub_agent_wakeup(explicit_reason, now).await;
        let mut loop_state = match self.wait_for_wakeup_events(reason, now).await {
            PreparedWakeup::Disabled {
                wakeup_id,
                trace_id,
                hp,
                sleep_ms,
                reason,
            } => {
                warn!(
                    "ai_agent.wait_wakeup skip: did={} wakeup_id={} status=disabled hp={} sleep_ms={}",
                    self.did, wakeup_id, hp, sleep_ms
                );
                return Ok(self.build_skip_report(
                    wakeup_id,
                    trace_id,
                    WakeupStatus::Disabled,
                    hp,
                    sleep_ms,
                    reason,
                ));
            }
            PreparedWakeup::SkippedNoInput {
                wakeup_id,
                trace_id,
                hp,
                sleep_ms,
                reason,
            } => {
                debug!(
                    "ai_agent.wait_wakeup skip: did={} wakeup_id={} status=no_input hp={} sleep_ms={}",
                    self.did, wakeup_id, hp, sleep_ms
                );
                return Ok(self.build_skip_report(
                    wakeup_id,
                    trace_id,
                    WakeupStatus::SkippedNoInput,
                    hp,
                    sleep_ms,
                    reason,
                ));
            }
            PreparedWakeup::Ready {
                wakeup_id,
                hp_before,
                trace_id,
                input_payload,
                inbox_record_ids,
            } => self.init_agent_loop_state(
                wakeup_id,
                hp_before,
                trace_id,
                input_payload,
                inbox_record_ids,
            ),
        };

        // Step 2~5: Main Loop Flow (session runtime + staged behavior engine)
        self.run_agent_loop_main_flow(&mut loop_state).await?;

        // Step 6: State Persistence
        self.agent_loop_persist_state(loop_state).await
    }

    async fn run_agent_loop_main_flow(
        &self,
        state: &mut AgentLoopState,
    ) -> Result<(), AIAgentError> {
        // Agent loop only resolves router and dispatches to session loop.
        let mut session_runtime = self.load_or_init_session_runtime_state(state).await?;
        self.maybe_force_route_on_new_input(state, &mut session_runtime)
            .await;

        // Stage 1: Mode Selection
        let mode_selection = self.select_behavior_mode(state).await;
        self.apply_mode_selection(state, &mode_selection);
        session_runtime.current_behavior = state.current_behavior.clone();

        // Stage 2: Resolve Router
        let should_sleep = self
            .run_resolve_router_stage(state, &mut session_runtime)
            .await?;
        self.persist_session_runtime_state(&session_runtime).await;
        if should_sleep {
            session_runtime.phase = SessionLoopPhase::Waiting;
            session_runtime.waiting = Some(WaitSpec::default_router_sleep());
            self.persist_session_checkpoint_placeholder(state, &session_runtime)
                .await;
            return Ok(());
        }

        // Stage 3: Session Loop Dispatch
        self.dispatch_session_loop(state, session_runtime).await
    }

    async fn load_or_init_session_runtime_state(
        &self,
        state: &AgentLoopState,
    ) -> Result<SessionRuntimeState, AIAgentError> {
        let session_id = state.loop_ctx.session_id.clone();
        let (runtime, created_snapshot) = {
            let mut guard = self.state.lock().await;
            if let Some(existing) = guard.session_runtime_states.get(&session_id).cloned() {
                return Ok(existing);
            }

            let created = SessionRuntimeState {
                agent_id: self.did.clone(),
                session_id: session_id.clone(),
                current_behavior: state.current_behavior.clone(),
                step_idx: state.steps,
                phase: SessionLoopPhase::CollectInput,
                waiting: None,
                last_input: state.input_payload.clone(),
            };
            guard
                .session_runtime_states
                .insert(session_id.clone(), created.clone());
            let snapshot = self.build_session_runtime_store_snapshot(&guard);
            (created, Some(snapshot))
        };
        if let Some(snapshot) = created_snapshot {
            self.persist_session_runtime_store_snapshot(snapshot).await;
        }
        Ok(runtime)
    }

    async fn maybe_force_route_on_new_input(
        &self,
        _state: &AgentLoopState,
        _runtime: &mut SessionRuntimeState,
    ) {
        // Placeholder: event-driven forced route switching will be added later.
    }

    async fn run_resolve_router_stage(
        &self,
        state: &mut AgentLoopState,
        runtime: &mut SessionRuntimeState,
    ) -> Result<bool, AIAgentError> {
        runtime.phase = SessionLoopPhase::ResolveRouter;

        let (resolve_router, resolve_tracking, resolve_actions) =
            self.resolve_router(state).await?;
        if let Some(tracking) = resolve_tracking.as_ref() {
            self.record_stage_cost(state, tracking, resolve_actions)
                .await;
        }
        self.apply_resolve_router(state, &resolve_router);
        self.send_router_reply_via_msg_center(state, &resolve_router)
            .await;

        runtime.session_id = state.loop_ctx.session_id.clone();
        runtime.current_behavior = state.current_behavior.clone();
        runtime.step_idx = state.steps;
        runtime.last_input = state.input_payload.clone();

        if resolve_router.is_sleep() {
            self.hydrate_context(state);
            state.input_payload["resolve_router_sleep"] = json!({
                "enabled": true,
                "is_sleep": true
            });
            info!(
                "ai_agent.wait_wakeup sleep after resolve_router: did={} wakeup_id={}",
                self.did, state.wakeup_id,
            );
            return Ok(true);
        }
        Ok(false)
    }

    async fn dispatch_session_loop(
        &self,
        state: &mut AgentLoopState,
        mut runtime: SessionRuntimeState,
    ) -> Result<(), AIAgentError> {
        match self
            .try_enter_session_loop(runtime.session_id.as_str())
            .await
        {
            Some(active_session) => {
                runtime.phase = SessionLoopPhase::Waiting;
                runtime.waiting =
                    Some(WaitSpec::default_session_loop_busy(active_session.as_str()));
                self.persist_session_checkpoint_placeholder(state, &runtime)
                    .await;
                debug!(
                    "ai_agent.session_loop dispatch_skipped: did={} wakeup_id={} target_session={} active_session={}",
                    self.did,
                    state.wakeup_id,
                    runtime.session_id,
                    active_session
                );
                return Ok(());
            }
            None => {
                debug!(
                    "ai_agent.session_loop dispatch_start: did={} wakeup_id={} session_id={}",
                    self.did, state.wakeup_id, runtime.session_id
                );
            }
        }

        let run_result = self.run_single_session_loop(state, &mut runtime).await;
        self.exit_session_loop(runtime.session_id.as_str()).await;
        run_result
    }

    async fn run_single_session_loop(
        &self,
        state: &mut AgentLoopState,
        runtime: &mut SessionRuntimeState,
    ) -> Result<(), AIAgentError> {
        // Keep backward-compatible behavior while moving execution ownership into
        // session-loop pipeline.
        self.hydrate_context(state);
        self.run_session_step_state_machine(state, runtime).await?;
        self.persist_session_checkpoint_placeholder(state, runtime)
            .await;
        Ok(())
    }

    async fn run_session_step_state_machine(
        &self,
        state: &mut AgentLoopState,
        runtime: &mut SessionRuntimeState,
    ) -> Result<(), AIAgentError> {
        runtime.phase = SessionLoopPhase::CollectInput;

        loop {
            match runtime.phase {
                SessionLoopPhase::CollectInput => {
                    let has_input = self
                        .generate_step_input_from_session_state_placeholder(state, runtime)
                        .await;
                    if !has_input {
                        runtime.phase = SessionLoopPhase::Waiting;
                        runtime.waiting = Some(WaitSpec::default_no_input());
                        self.persist_session_checkpoint_placeholder(state, runtime)
                            .await;
                        break;
                    }
                    runtime.waiting = None;
                    runtime.phase = SessionLoopPhase::RunBehaviorStep;
                }
                SessionLoopPhase::RunBehaviorStep => {
                    state.current_behavior = runtime.current_behavior.clone();
                    state.steps = runtime.step_idx;
                    self.execute_behavior_steps(state).await?;
                    runtime.current_behavior = state.current_behavior.clone();
                    runtime.step_idx = state.steps;
                    runtime.last_input = state.input_payload.clone();
                    runtime.phase = SessionLoopPhase::ApplyStepEffects;
                }
                SessionLoopPhase::ApplyStepEffects => {
                    self.apply_step_side_effects_placeholder(state, runtime)
                        .await?;
                    self.persist_session_checkpoint_placeholder(state, runtime)
                        .await;
                    break;
                }
                SessionLoopPhase::Waiting => break,
                SessionLoopPhase::ResolveRouter => {
                    runtime.phase = SessionLoopPhase::CollectInput;
                }
            }
        }

        Ok(())
    }

    async fn persist_session_runtime_state(&self, runtime: &SessionRuntimeState) {
        let snapshot = {
            let mut guard = self.state.lock().await;
            guard
                .session_runtime_states
                .insert(runtime.session_id.clone(), runtime.clone());
            self.build_session_runtime_store_snapshot(&guard)
        };
        self.persist_session_runtime_store_snapshot(snapshot).await;
    }

    async fn try_enter_session_loop(&self, session_id: &str) -> Option<String> {
        let snapshot = {
            let mut guard = self.state.lock().await;
            if let Some(active) = guard.active_session_loop.as_ref() {
                if active != session_id {
                    return Some(active.clone());
                }
                return None;
            }
            guard.active_session_loop = Some(session_id.to_string());
            self.build_session_runtime_store_snapshot(&guard)
        };
        self.persist_session_runtime_store_snapshot(snapshot).await;
        None
    }

    async fn exit_session_loop(&self, session_id: &str) {
        let snapshot = {
            let mut guard = self.state.lock().await;
            if guard.active_session_loop.as_deref() == Some(session_id) {
                guard.active_session_loop = None;
                Some(self.build_session_runtime_store_snapshot(&guard))
            } else {
                None
            }
        };
        if let Some(snapshot) = snapshot {
            self.persist_session_runtime_store_snapshot(snapshot).await;
        }
    }

    fn build_session_runtime_store_snapshot(&self, state: &AIAgentState) -> SessionRuntimeStore {
        SessionRuntimeStore {
            agent_id: self.did.clone(),
            active_session_loop: state.active_session_loop.clone(),
            sessions: state.session_runtime_states.clone(),
            updated_at_ms: now_ms(),
        }
    }

    async fn persist_session_runtime_store_snapshot(&self, snapshot: SessionRuntimeStore) {
        if let Some(parent) = self.session_runtime_store_path.parent() {
            if let Err(err) = fs::create_dir_all(parent).await {
                warn!(
                    "ai_agent.session_runtime persist_failed: did={} path={} err={}",
                    self.did,
                    self.session_runtime_store_path.display(),
                    err
                );
                return;
            }
        }

        let payload = match serde_json::to_string_pretty(&snapshot) {
            Ok(v) => v,
            Err(err) => {
                warn!(
                    "ai_agent.session_runtime serialize_failed: did={} err={}",
                    self.did, err
                );
                return;
            }
        };
        if let Err(err) = fs::write(&self.session_runtime_store_path, payload).await {
            warn!(
                "ai_agent.session_runtime write_failed: did={} path={} err={}",
                self.did,
                self.session_runtime_store_path.display(),
                err
            );
        }
    }

    async fn generate_step_input_from_session_state_placeholder(
        &self,
        state: &mut AgentLoopState,
        runtime: &mut SessionRuntimeState,
    ) -> bool {
        // Placeholder: the final implementation should rebuild prompt input by
        // session runtime + newly arrived messages/events for each step.
        runtime.last_input = state.input_payload.clone();
        let (inbox_count, event_count) = wakeup_input_counts(&state.input_payload);
        inbox_count > 0 || event_count > 0 || runtime.waiting.is_none()
    }

    async fn apply_step_side_effects_placeholder(
        &self,
        _state: &mut AgentLoopState,
        _runtime: &mut SessionRuntimeState,
    ) -> Result<(), AIAgentError> {
        // Placeholder: apply session_delta / set_memory / todo patches in follow-up.
        Ok(())
    }

    async fn persist_session_checkpoint_placeholder(
        &self,
        state: &AgentLoopState,
        runtime: &SessionRuntimeState,
    ) {
        self.persist_session_runtime_state(runtime).await;
        // Placeholder: durable checkpoint persistence is not wired yet.
        debug!(
            "ai_agent.session_checkpoint placeholder: did={} wakeup_id={} session_id={} behavior={} step={} phase={}",
            self.did,
            state.wakeup_id,
            runtime.session_id,
            runtime.current_behavior,
            runtime.step_idx,
            runtime.phase.as_str()
        );
    }

    async fn log_sub_agent_wakeup(&self, explicit_reason: bool, now: u64) {
        if !self.is_sub_agent() {
            return;
        }
        let ctx = TraceCtx {
            trace_id: format!("{}:wakeup:{}", self.did, now),
            agent_did: self.did.clone(),
            behavior: "on_wakeup".to_string(),
            step_idx: 0,
            wakeup_id: format!("wakeup-start-{}", now),
        };
        append_workspace_worklog_entry(
            self.tool_mgr.clone(),
            &ctx,
            "sub_agent_wake",
            "info",
            "sub-agent wakeup".to_string(),
            json!({
                "explicit_reason": explicit_reason
            }),
            None,
            vec!["sub_agent".to_string(), "active".to_string()],
            None,
            None,
        )
        .await;
    }

    fn build_skip_report(
        &self,
        wakeup_id: String,
        trace_id: String,
        status: WakeupStatus,
        hp: u32,
        sleep_ms: u64,
        reason: Json,
    ) -> WakeupReport {
        WakeupReport {
            wakeup_id,
            trace_id,
            status,
            final_behavior: BEHAVIOR_ON_WAKEUP.to_string(),
            steps: 0,
            behavior_hops: 0,
            hp_before: hp,
            hp_after: hp,
            token_prompt: 0,
            token_completion: 0,
            token_total: 0,
            sleep_ms,
            reason,
            last_error: None,
        }
    }

    fn init_agent_loop_state(
        &self,
        wakeup_id: String,
        hp_before: u32,
        trace_id: String,
        mut input_payload: Json,
        inbox_record_ids: Vec<String>,
    ) -> AgentLoopState {
        let loop_ctx = derive_wakeup_loop_context(&wakeup_id, &input_payload);
        enrich_wakeup_payload_with_loop_context(&mut input_payload, &loop_ctx);
        let (inbox_count, event_count) = wakeup_input_counts(&input_payload);
        info!(
            "ai_agent.on_wakeup ready: did={} wakeup_id={} hp_before={} inbox_count={} event_count={} session_id={}",
            self.did,
            wakeup_id,
            hp_before,
            inbox_count,
            event_count,
            loop_ctx.session_id.as_str()
        );

        AgentLoopState {
            wakeup_started: Instant::now(),
            wakeup_id,
            hp_before,
            trace_id,
            input_payload,
            inbox_record_ids,
            loop_ctx,
            status: WakeupStatus::Completed,
            last_error: None,
            token_usage: TokenUsage::default(),
            steps: 0,
            behavior_hops: 0,
            current_behavior: BEHAVIOR_ON_WAKEUP.to_string(),
            pending_observations: vec![],
            memory_queries: Vec::new(),
        }
    }

    async fn resolve_router(
        &self,
        state: &AgentLoopState,
    ) -> Result<(ResolveRouterResult, Option<LLMTrackingInfo>, usize), AIAgentError> {
        if !matches!(state.status, WakeupStatus::Completed) {
            return Ok((ResolveRouterResult::default(), None, 0));
        }
        let Some((trace, llm_result, tracking)) = self
            .run_optional_stage_behavior(
                BEHAVIOR_RESOLVE_ROUTER,
                &state.trace_id,
                &state.wakeup_id,
                0,
                now_ms(),
                json!({
                    "loop_context": state.loop_ctx.clone(),
                    "input_event": compact_json_for_worklog(state.input_payload.clone(), 8 * 1024)
                }),
                json!({}),
                state.pending_observations.clone(),
                vec![EnvKV {
                    key: "stage.name".to_string(),
                    value: BEHAVIOR_RESOLVE_ROUTER.to_string(),
                }],
            )
            .await?
        else {
            return Ok((ResolveRouterResult::default(), None, 0));
        };

        let mut output = parse_resolve_router_result(&tracking.raw_output).unwrap_or_default();
        output.session_id = sanitize_non_empty_string(output.session_id.clone());
        output.reply = sanitize_non_empty_string(output.reply.clone());
        if output.next_behavior.is_none() {
            output.next_behavior = Some("END".to_string());
        }
        if let Some(next_behavior) = output.next_behavior.clone() {
            if next_behavior == "END" {
                output.next_behavior = Some("END".to_string());
            } else if !self.behavior_exists(&next_behavior).await {
                warn!(
                    "ai_agent.resolve_router next_behavior ignored: did={} wakeup_id={} next_behavior={} reason=behavior_not_found",
                    self.did, state.wakeup_id, next_behavior
                );
                output.next_behavior = None;
            }
        }
        if !llm_result.tool_calls.is_empty() {
            let planned_calls = sanitize_tool_calls(&llm_result.tool_calls);
            if !planned_calls.is_empty() {
                let _ = self.execute_router_tool_calls(&trace, &planned_calls).await;
            }
        }
        Ok((output, Some(tracking), llm_result.actions.len()))
    }

    async fn select_behavior_mode(&self, state: &AgentLoopState) -> ModeSelectionResult {
        let selected_mode = BEHAVIOR_ON_WAKEUP.to_string();
        debug!(
            "ai_agent.mode_select: did={} wakeup_id={} mode={} source=default",
            self.did, state.wakeup_id, selected_mode
        );
        ModeSelectionResult {
            behavior: selected_mode,
            source: "default",
        }
    }

    fn apply_mode_selection(&self, state: &mut AgentLoopState, result: &ModeSelectionResult) {
        state.current_behavior = result.behavior.clone();
        debug!(
            "ai_agent.mode_apply: did={} wakeup_id={} mode={} source={}",
            self.did, state.wakeup_id, state.current_behavior, result.source
        );
    }

    fn apply_resolve_router(&self, state: &mut AgentLoopState, result: &ResolveRouterResult) {
        if let Some(session_id) = result.session_id.as_ref() {
            state.loop_ctx.session_id = session_id.clone();
        }
        append_unique_strings(&mut state.memory_queries, &result.memory_queries);
        if let Some((title, description)) = result.new_session.as_ref() {
            state.input_payload["new_session"] = json!({
                "title": title,
                "description": description
            });
        }
        if let Some(reply) = result.reply.as_ref() {
            state.input_payload["router_reply"] = json!(reply);
        }
        if let Some(next_behavior) = result.next_behavior.as_ref() {
            state.current_behavior = next_behavior.clone();
        }
        enrich_wakeup_payload_with_loop_context(&mut state.input_payload, &state.loop_ctx);
    }

    fn hydrate_context(&self, state: &mut AgentLoopState) {
        if !state.memory_queries.is_empty() {
            state.input_payload["memory_queries"] = json!(state.memory_queries.clone());
        }
        enrich_wakeup_payload_with_loop_context(&mut state.input_payload, &state.loop_ctx);
    }

    async fn execute_behavior_steps(&self, state: &mut AgentLoopState) -> Result<(), AIAgentError> {
        if !matches!(state.status, WakeupStatus::Completed) {
            return Ok(());
        }

        loop {
            debug!(
                "ai_agent.loop step: did={} wakeup_id={} step={} behavior={} pending_observations={}",
                self.did,
                state.wakeup_id,
                state.steps,
                state.current_behavior,
                state.pending_observations.len()
            );
            if state.steps >= self.cfg.max_steps_per_wakeup {
                state.status = WakeupStatus::SafeStop;
                state.last_error = Some(format!(
                    "max_steps_per_wakeup reached: {}",
                    self.cfg.max_steps_per_wakeup
                ));
                warn!(
                    "ai_agent.loop safe_stop: did={} wakeup_id={} reason=max_steps limit={}",
                    self.did, state.wakeup_id, self.cfg.max_steps_per_wakeup
                );
                break;
            }
            if state.wakeup_started.elapsed().as_millis() as u64 >= self.cfg.max_walltime_ms {
                state.status = WakeupStatus::SafeStop;
                state.last_error = Some(format!(
                    "max_walltime_ms reached: {}",
                    self.cfg.max_walltime_ms
                ));
                warn!(
                    "ai_agent.loop safe_stop: did={} wakeup_id={} reason=max_walltime limit_ms={}",
                    self.did, state.wakeup_id, self.cfg.max_walltime_ms
                );
                break;
            }

            if self.current_hp().await <= self.cfg.hp_floor {
                state.status = WakeupStatus::SafeStop;
                state.last_error = Some(format!("hp <= hp_floor ({})", self.cfg.hp_floor));
                warn!(
                    "ai_agent.loop safe_stop: did={} wakeup_id={} reason=hp_floor hp_floor={}",
                    self.did, state.wakeup_id, self.cfg.hp_floor
                );
                break;
            }

            if !self.behavior_exists(&state.current_behavior).await {
                warn!(
                    "ai_agent.loop skip: did={} wakeup_id={} reason=behavior_not_found behavior={}",
                    self.did, state.wakeup_id, state.current_behavior
                );
                break;
            }

            let cfg = self.ensure_behavior_config(&state.current_behavior).await?;
            let trace = TraceCtx {
                trace_id: state.trace_id.clone(),
                agent_did: self.did.clone(),
                behavior: state.current_behavior.clone(),
                step_idx: state.steps,
                wakeup_id: state.wakeup_id.clone(),
            };

            let behavior =
                LLMBehavior::new(cfg.to_llm_behavior_config(), self.build_behavior_deps());
            let memory_pack = self.load_memory_pack(&trace).await;
            let remaining_steps = self.cfg.max_steps_per_wakeup.saturating_sub(state.steps);
            let mut env_context = self.build_env_context(now_ms()).await;
            env_context.extend(vec![
                EnvKV {
                    key: "loop.session_id".to_string(),
                    value: state.loop_ctx.session_id.clone(),
                },
                EnvKV {
                    key: "loop.event_id".to_string(),
                    value: state.loop_ctx.event_id.clone(),
                },
                EnvKV {
                    key: "step.index".to_string(),
                    value: state.steps.to_string(),
                },
                EnvKV {
                    key: "step.remaining".to_string(),
                    value: remaining_steps.to_string(),
                },
            ]);
            let mut step_payload = state.input_payload.clone();
            step_payload["session"] = json!({
                "session_id": state.loop_ctx.session_id.clone(),
                "event_id": state.loop_ctx.event_id.clone()
            });
            step_payload["step_meta"] = json!({
                "step_index": state.steps,
                "remaining_steps": remaining_steps
            });
            let input = BehaviorExecInput {
                trace: trace.clone(),
                role_md: self.role_md.clone(),
                self_md: self.self_md.clone(),
                session_id: Some(state.loop_ctx.session_id.clone()),
                behavior_prompt: cfg.process_rule.clone(),
                env_context,
                inbox: step_payload,
                memory: memory_pack,
                last_observations: state.pending_observations.clone(),
                limits: cfg.limits.clone(),
            };

            let (llm_result, tracking) = match behavior.run_step(input).await {
                Ok(v) => v,
                Err(err) => {
                    state.status = WakeupStatus::Error;
                    state.last_error = Some(err.to_string());
                    error!(
                        "ai_agent.loop llm_error: did={} wakeup_id={} step={} behavior={} err={:?}",
                        self.did,
                        state.wakeup_id,
                        state.steps,
                        state.current_behavior,
                        state.last_error
                    );
                    break;
                }
            };
            self.record_stage_cost(state, &tracking, llm_result.actions.len())
                .await;
            debug!(
                "ai_agent.loop llm_done: did={} wakeup_id={} step={} behavior={} tokens={} actions={} is_sleep={} next_behavior={:?}",
                self.did,
                state.wakeup_id,
                state.steps,
                state.current_behavior,
                tracking.token_usage.total,
                llm_result.actions.len(),
                llm_result.is_sleep(),
                llm_result.next_behavior
            );

            state.pending_observations = self.execute_actions(&trace, &llm_result.actions).await;
            let mut should_break = llm_result.is_sleep();

            if !llm_result.reply.is_empty() {
                for msg in llm_result.reply.iter().take(3) {
                    let audience = msg.audience.trim();
                    let format = msg.format.trim();
                    let content = msg.content.trim();
                    if content.is_empty() {
                        continue;
                    }
                    state.pending_observations.push(Observation {
                        source: ObservationSource::System,
                        name: "executor.reply".to_string(),
                        content: json!({
                            "audience": audience,
                            "format": format,
                            "content": content,
                            "untrusted": true
                        }),
                        ok: true,
                        truncated: false,
                        bytes: content.len(),
                    });
                }
            }
            if !llm_result.tool_calls.is_empty() {
                let planned_calls = sanitize_tool_calls(&llm_result.tool_calls);
                let tool_obs = self.execute_router_tool_calls(&trace, &planned_calls).await;
                if !tool_obs.is_empty() {
                    state.pending_observations.extend(tool_obs);
                    should_break = false;
                }
            }
            if let Some(next_behavior) = llm_result.next_behavior.as_ref() {
                if next_behavior == "END" {
                    debug!(
                        "ai_agent.loop stop_marker: did={} wakeup_id={} behavior={} marker=END",
                        self.did, state.wakeup_id, state.current_behavior
                    );
                } else if next_behavior != &state.current_behavior {
                    state.behavior_hops = state.behavior_hops.saturating_add(1);
                    if state.behavior_hops > self.cfg.max_behavior_hops {
                        state.status = WakeupStatus::SafeStop;
                        state.last_error = Some(format!(
                            "max_behavior_hops reached: {}",
                            self.cfg.max_behavior_hops
                        ));
                        warn!(
                            "ai_agent.loop safe_stop: did={} wakeup_id={} reason=max_behavior_hops limit={}",
                            self.did, state.wakeup_id, self.cfg.max_behavior_hops
                        );
                        break;
                    }
                    info!(
                        "ai_agent.loop behavior_switch: did={} wakeup_id={} from={} to={} hops={}",
                        self.did,
                        state.wakeup_id,
                        state.current_behavior,
                        next_behavior,
                        state.behavior_hops
                    );
                    state.current_behavior = next_behavior.to_string();
                    should_break = false;
                }
            }

            state.steps = state.steps.saturating_add(1);
            if should_break {
                debug!(
                    "ai_agent.loop stop: did={} wakeup_id={} reason=llm_sleep",
                    self.did, state.wakeup_id
                );
                break;
            }
            if llm_result.actions.is_empty() && llm_result.next_behavior.is_none() {
                debug!(
                    "ai_agent.loop stop: did={} wakeup_id={} reason=no_actions_no_next_behavior",
                    self.did, state.wakeup_id
                );
                break;
            }
        }
        Ok(())
    }

    async fn agent_loop_persist_state(
        &self,
        state: AgentLoopState,
    ) -> Result<WakeupReport, AIAgentError> {
        let hp_after = self.current_hp().await;
        self.finalize_msg_center_inbox_states(
            &state.wakeup_id,
            &state.status,
            &state.inbox_record_ids,
        )
        .await;
        if hp_after == 0 {
            self.disable().await;
            warn!(
                "ai_agent.on_wakeup disable: did={} wakeup_id={} reason=hp_exhausted",
                self.did, state.wakeup_id
            );
        }
        let sleep_ms = self.update_sleep_after_wakeup(&state.status).await;
        if matches!(state.status, WakeupStatus::Error) {
            error!(
                "ai_agent.on_wakeup finish: did={} wakeup_id={} status=error behavior={} steps={} hops={} hp={}=>{} tokens={} sleep_ms={} err={:?}",
                self.did,
                state.wakeup_id,
                state.current_behavior,
                state.steps,
                state.behavior_hops,
                state.hp_before,
                hp_after,
                state.token_usage.total,
                sleep_ms,
                state.last_error
            );
        } else if matches!(state.status, WakeupStatus::SafeStop) {
            warn!(
                "ai_agent.on_wakeup finish: did={} wakeup_id={} status=safe_stop behavior={} steps={} hops={} hp={}=>{} tokens={} sleep_ms={} err={:?}",
                self.did,
                state.wakeup_id,
                state.current_behavior,
                state.steps,
                state.behavior_hops,
                state.hp_before,
                hp_after,
                state.token_usage.total,
                sleep_ms,
                state.last_error
            );
        } else {
            info!(
                "ai_agent.on_wakeup finish: did={} wakeup_id={} status={} behavior={} steps={} hops={} hp={}=>{} tokens={} sleep_ms={}",
                self.did,
                state.wakeup_id,
                wakeup_status_name(&state.status),
                state.current_behavior,
                state.steps,
                state.behavior_hops,
                state.hp_before,
                hp_after,
                state.token_usage.total,
                sleep_ms
            );
        }

        Ok(WakeupReport {
            wakeup_id: state.wakeup_id,
            trace_id: state.trace_id,
            status: state.status,
            final_behavior: state.current_behavior,
            steps: state.steps,
            behavior_hops: state.behavior_hops,
            hp_before: state.hp_before,
            hp_after,
            token_prompt: state.token_usage.prompt,
            token_completion: state.token_usage.completion,
            token_total: state.token_usage.total,
            sleep_ms,
            reason: state.input_payload,
            last_error: state.last_error,
        })
    }

    async fn record_stage_cost(
        &self,
        state: &mut AgentLoopState,
        tracking: &LLMTrackingInfo,
        action_count: usize,
    ) {
        state.token_usage = state.token_usage.clone().add(tracking.token_usage.clone());
        self.consume_hp(tracking.token_usage.total, action_count as u32)
            .await;
    }

    fn build_behavior_deps(&self) -> LLMBehaviorDeps {
        LLMBehaviorDeps {
            taskmgr: self.deps.taskmgr.clone(),
            aicc: self.deps.aicc.clone(),
            tools: self.tool_mgr.clone(),
            policy: self.policy.clone(),
            worklog: self.worklog.clone(),
            tokenizer: self.tokenizer.clone(),
        }
    }

    async fn behavior_exists(&self, behavior_name: &str) -> bool {
        let guard = self.behavior_cfg_cache.read().await;
        guard.contains_key(behavior_name)
    }

    async fn run_optional_stage_behavior(
        &self,
        behavior_name: &str,
        trace_id: &str,
        wakeup_id: &str,
        step_idx: u32,
        now: u64,
        inbox: Json,
        memory: Json,
        last_observations: Vec<Observation>,
        extra_env: Vec<EnvKV>,
    ) -> Result<Option<(TraceCtx, BehaviorLLMResult, LLMTrackingInfo)>, AIAgentError> {
        if !self.behavior_exists(behavior_name).await {
            return Ok(None);
        }
        let cfg = self.ensure_behavior_config(behavior_name).await?;
        let trace = TraceCtx {
            trace_id: trace_id.to_string(),
            agent_did: self.did.clone(),
            behavior: behavior_name.to_string(),
            step_idx,
            wakeup_id: wakeup_id.to_string(),
        };
        let mut env_context = self.build_env_context(now).await;
        env_context.extend(extra_env);
        let input = BehaviorExecInput {
            trace: trace.clone(),
            role_md: self.role_md.clone(),
            self_md: self.self_md.clone(),
            session_id: inbox
                .pointer("/loop_context/session_id")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string()),
            behavior_prompt: cfg.process_rule.clone(),
            env_context,
            inbox,
            memory,
            last_observations,
            limits: cfg.limits.clone(),
        };
        let (llm_result, tracking) =
            LLMBehavior::new(cfg.to_llm_behavior_config(), self.build_behavior_deps())
                .run_step(input)
                .await
                .map_err(|err| AIAgentError::LLMBehavior(err.to_string()))?;
        Ok(Some((trace, llm_result, tracking)))
    }

    async fn execute_router_tool_calls(
        &self,
        trace: &TraceCtx,
        calls: &[ToolCall],
    ) -> Vec<Observation> {
        if calls.is_empty() {
            return vec![];
        }

        let mut observations = Vec::<Observation>::new();
        for (idx, call) in calls.iter().take(MAX_ROUTER_TOOL_CALLS).enumerate() {
            let name = call.name.trim();
            if name.is_empty() {
                continue;
            }
            let call_id = if call.call_id.trim().is_empty() {
                format!("{}-router-{}-{}", trace.wakeup_id, trace.step_idx, idx)
            } else {
                call.call_id.trim().to_string()
            };
            let tool_call = ToolCall {
                name: name.to_string(),
                args: call.args.clone(),
                call_id,
            };
            match self.tool_mgr.call_tool(trace, tool_call).await {
                Ok(raw) => observations.push(Sanitizer::sanitize_observation(
                    ObservationSource::Tool,
                    name,
                    raw,
                    8 * 1024,
                )),
                Err(err) => observations.push(Sanitizer::tool_error_observation(
                    name,
                    err.to_string(),
                    8 * 1024,
                )),
            }
        }

        observations
    }

    async fn send_router_reply_via_msg_center(
        &self,
        state: &AgentLoopState,
        result: &ResolveRouterResult,
    ) {
        let Some(reply_text) = result.reply.as_ref() else {
            return;
        };
        let reply_text = reply_text.trim();
        if reply_text.is_empty() {
            return;
        }

        let Some(msg_center) = self.deps.msg_center.as_ref() else {
            debug!(
                "ai_agent.router_reply skip: did={} wakeup_id={} reason=no_msg_center",
                self.did, state.wakeup_id
            );
            return;
        };

        let Some(reply_target) = extract_msg_center_reply_target(&state.input_payload) else {
            debug!(
                "ai_agent.router_reply skip: did={} wakeup_id={} reason=no_reply_target",
                self.did, state.wakeup_id
            );
            return;
        };

        let from = match DID::from_str(self.did.as_str()) {
            Ok(v) => v,
            Err(err) => {
                warn!(
                    "ai_agent.router_reply skip: did={} wakeup_id={} reason=invalid_agent_did err={}",
                    self.did, state.wakeup_id, err
                );
                return;
            }
        };

        let msg_id = match build_msg_center_reply_obj_id() {
            Some(v) => v,
            None => {
                warn!(
                    "ai_agent.router_reply skip: did={} wakeup_id={} reason=msg_id_alloc_failed",
                    self.did, state.wakeup_id
                );
                return;
            }
        };

        let session_id = result
            .session_id
            .clone()
            .unwrap_or_else(|| state.loop_ctx.session_id.clone());
        let mut payload = json!({
            "kind": "text",
            "text": reply_text,
            "session_id": session_id,
            "event_id": state.loop_ctx.event_id,
            "meta": {
                "source": "ai_agent.resolve_router",
                "wakeup_id": state.wakeup_id
            }
        });
        if let Some(record_id) = reply_target.record_id.as_ref() {
            payload["reply_record_id"] = json!(record_id);
        }
        if let Some(msg_id) = reply_target.origin_msg_id.as_ref() {
            payload["reply_msg_id"] = json!(msg_id);
        }

        let mut out_msg = MsgObject::new(
            msg_id,
            from,
            None,
            vec![reply_target.target_did.clone()],
            payload,
            now_ms(),
        );
        out_msg.thread_key = reply_target.thread_key.clone();
        out_msg.meta = Some(json!({
            "source": "opendan.ai_agent",
            "wakeup_id": state.wakeup_id,
            "reply_origin": "resolve_router"
        }));

        let idempotency_target = reply_target
            .record_id
            .as_deref()
            .unwrap_or("no_record")
            .to_string();
        let idempotency_key = Some(format!(
            "opendan-router-reply:{}:{}:{}",
            self.did, state.wakeup_id, idempotency_target
        ));

        match msg_center
            .post_send(out_msg, reply_target.send_ctx, idempotency_key)
            .await
        {
            Ok(posted) => {
                info!(
                    "ai_agent.router_reply queued: did={} wakeup_id={} msg_id={} deliveries={}",
                    self.did,
                    state.wakeup_id,
                    posted.msg_id.to_string(),
                    posted.deliveries.len()
                );
            }
            Err(err) => {
                warn!(
                    "ai_agent.router_reply failed: did={} wakeup_id={} err={}",
                    self.did, state.wakeup_id, err
                );
            }
        }
    }

    async fn wait_for_wakeup_events(&self, reason: Option<Json>, now: u64) -> PreparedWakeup {
        let pulled_inbox = if reason.is_none() {
            self.pull_inbox_from_msg_center(MSG_CENTER_INBOX_PULL_LIMIT)
                .await
        } else {
            Vec::new()
        };

        let mut guard = self.state.lock().await;
        guard.wakeup_seq = guard.wakeup_seq.saturating_add(1);
        let wakeup_id = format!("wakeup-{}", guard.wakeup_seq);
        let trace_id = format!("{}:{}", self.did, wakeup_id);
        let hp_before = guard.hp;

        if let Some(last_wakeup_ms) = guard.last_wakeup_ms {
            let elapsed_ms = now.saturating_sub(last_wakeup_ms);
            let hp_gain = (elapsed_ms / 1_000) as u32;
            guard.hp = guard.hp.saturating_add(hp_gain).min(self.cfg.hp_max);
            debug!(
                "ai_agent.wakeup regen_hp: did={} elapsed_ms={} gain={} hp_now={}",
                self.did, elapsed_ms, hp_gain, guard.hp
            );
        }
        guard.last_wakeup_ms = Some(now);

        if !guard.enabled || guard.hp == 0 {
            warn!(
                "ai_agent.wakeup blocked: did={} enabled={} hp={}",
                self.did, guard.enabled, guard.hp
            );
            return PreparedWakeup::Disabled {
                wakeup_id,
                trace_id,
                hp: guard.hp,
                sleep_ms: guard.next_sleep_ms,
                reason: reason.unwrap_or_else(|| json!({"trigger":"disabled"})),
            };
        }

        let mut inbox_record_ids = Vec::<String>::new();
        let input_payload = if let Some(reason) = reason {
            reason
        } else {
            for pulled in pulled_inbox {
                inbox_record_ids.push(pulled.record_id);
                guard.inbox_msgs.push_back(pulled.input);
            }
            let inbox: Vec<Json> = guard.inbox_msgs.drain(..).collect();
            let events: Vec<Json> = guard.queued_events.drain(..).collect();
            if inbox.is_empty() && events.is_empty() {
                guard.next_sleep_ms = (guard.next_sleep_ms.saturating_mul(2))
                    .clamp(self.cfg.default_sleep_ms, self.cfg.max_sleep_ms);
                debug!(
                    "ai_agent.wakeup no_input: did={} next_sleep_ms={}",
                    self.did, guard.next_sleep_ms
                );
                return PreparedWakeup::SkippedNoInput {
                    wakeup_id,
                    trace_id,
                    hp: guard.hp,
                    sleep_ms: guard.next_sleep_ms,
                    reason: json!({
                        "trigger": "on_wakeup",
                        "decision": DEFAULT_SLEEP_REASON,
                        "inbox_count": 0,
                        "event_count": 0
                    }),
                };
            }

            json!({
                "trigger": "on_wakeup",
                "inbox": inbox,
                "events": events
            })
        };

        guard.next_sleep_ms = self.cfg.default_sleep_ms;
        debug!(
            "ai_agent.wakeup accepted: did={} next_sleep_ms={}",
            self.did, guard.next_sleep_ms
        );
        PreparedWakeup::Ready {
            wakeup_id,
            hp_before,
            trace_id,
            input_payload,
            inbox_record_ids,
        }
    }

    async fn pull_inbox_from_msg_center(&self, limit: usize) -> Vec<PulledInboxMessage> {
        let Some(msg_center) = self.deps.msg_center.as_ref() else {
            return Vec::new();
        };
        let owner = match DID::from_str(self.did.as_str()) {
            Ok(owner) => owner,
            Err(err) => {
                warn!(
                    "ai_agent.msg_center pull_inbox skipped: did={} reason=invalid_did err={}",
                    self.did, err
                );
                return Vec::new();
            }
        };

        let mut inbox = Vec::<PulledInboxMessage>::new();
        for _ in 0..limit {
            let record = match msg_center
                .get_next(
                    owner.clone(),
                    BoxKind::Inbox,
                    Some(vec![MsgState::Unread]),
                    Some(true),
                )
                .await
            {
                Ok(Some(record)) => record,
                Ok(None) => break,
                Err(err) => {
                    warn!(
                        "ai_agent.msg_center pull_inbox failed: did={} err={}",
                        self.did, err
                    );
                    break;
                }
            };
            let pulled = msg_center_record_to_inbox_message(record);
            let ctx = TraceCtx {
                trace_id: format!("{}:pull_inbox:{}", self.did, now_ms()),
                agent_did: self.did.clone(),
                behavior: "on_wakeup".to_string(),
                step_idx: 0,
                wakeup_id: format!("pull-{}", now_ms()),
            };
            append_workspace_worklog_entry(
                self.tool_mgr.clone(),
                &ctx,
                "message_reply",
                "info",
                "received msg_center inbox message".to_string(),
                json!({
                    "source": "msg_center.get_next",
                    "record_id": pulled.record_id,
                    "message": compact_json_for_worklog(pulled.input.clone(), 6 * 1024)
                }),
                extract_session_id_from_inbox_payload(&pulled.input),
                vec!["message".to_string(), "recv".to_string()],
                None,
                None,
            )
            .await;
            inbox.push(pulled);
        }
        if !inbox.is_empty() {
            info!(
                "ai_agent.msg_center pull_inbox: did={} pulled={}",
                self.did,
                inbox.len()
            );
        }
        inbox
    }

    async fn finalize_msg_center_inbox_states(
        &self,
        wakeup_id: &str,
        status: &WakeupStatus,
        record_ids: &[String],
    ) {
        if record_ids.is_empty() {
            return;
        }
        let Some(msg_center) = self.deps.msg_center.as_ref() else {
            warn!(
                "ai_agent.msg_center finalize_inbox skipped: did={} wakeup_id={} reason=no_client records={}",
                self.did,
                wakeup_id,
                record_ids.len()
            );
            return;
        };

        let target_state = match status {
            WakeupStatus::Completed => MsgState::Readed,
            WakeupStatus::SafeStop | WakeupStatus::Error => MsgState::Unread,
            WakeupStatus::Disabled | WakeupStatus::SkippedNoInput => return,
        };

        let mut ok = 0usize;
        let mut failed = 0usize;
        for record_id in record_ids {
            match msg_center
                .update_record_state(record_id.clone(), target_state.clone(), None)
                .await
            {
                Ok(_) => ok = ok.saturating_add(1),
                Err(err) => {
                    failed = failed.saturating_add(1);
                    warn!(
                        "ai_agent.msg_center finalize_inbox failed: did={} wakeup_id={} record_id={} target_state={:?} err={}",
                        self.did,
                        wakeup_id,
                        record_id,
                        target_state,
                        err
                    );
                }
            }
        }
        if failed == 0 {
            info!(
                "ai_agent.msg_center finalize_inbox: did={} wakeup_id={} records={} target_state={:?}",
                self.did,
                wakeup_id,
                ok,
                target_state
            );
        } else {
            warn!(
                "ai_agent.msg_center finalize_inbox partial: did={} wakeup_id={} ok={} failed={} target_state={:?}",
                self.did,
                wakeup_id,
                ok,
                failed,
                target_state
            );
        }
    }

    async fn ensure_behavior_config(
        &self,
        behavior_name: &str,
    ) -> Result<BehaviorConfig, AIAgentError> {
        let from_cache = {
            let guard = self.behavior_cfg_cache.read().await;
            guard.get(behavior_name).cloned()
        };
        if let Some(cfg) = from_cache {
            return Ok(cfg);
        }

        info!(
            "ai_agent.behavior load: did={} behavior={} source=disk",
            self.did, behavior_name
        );
        let loaded = BehaviorConfig::load_from_dir(&self.behaviors_dir, behavior_name).await?;
        let mut guard = self.behavior_cfg_cache.write().await;
        guard.insert(behavior_name.to_string(), loaded.clone());
        Ok(loaded)
    }

    async fn build_env_context(&self, now: u64) -> Vec<EnvKV> {
        let hp = self.current_hp().await;
        vec![
            EnvKV {
                key: "agent.did".to_string(),
                value: self.did.clone(),
            },
            EnvKV {
                key: "agent.root".to_string(),
                value: self.cfg.agent_root.to_string_lossy().to_string(),
            },
            EnvKV {
                key: "workspace.root".to_string(),
                value: self
                    .environment
                    .workspace_root()
                    .to_string_lossy()
                    .to_string(),
            },
            EnvKV {
                key: "agent.hp".to_string(),
                value: hp.to_string(),
            },
            EnvKV {
                key: "now.unix_ms".to_string(),
                value: now.to_string(),
            },
        ]
    }

    async fn load_memory_pack(&self, trace: &TraceCtx) -> Json {
        if !self.tool_mgr.has_tool(TOOL_LOAD_MEMORY) {
            debug!(
                "ai_agent.memory skip: did={} wakeup_id={} reason=no_tool",
                self.did, trace.wakeup_id
            );
            return json!({});
        }

        let call = ToolCall {
            name: TOOL_LOAD_MEMORY.to_string(),
            args: json!({
                "token_limit": self.cfg.memory_token_limit
            }),
            call_id: format!("{}-load-memory-{}", trace.wakeup_id, trace.step_idx),
        };

        match self
            .tool_mgr
            .call_tool(
                &TraceCtx {
                    trace_id: trace.trace_id.clone(),
                    agent_did: trace.agent_did.clone(),
                    behavior: trace.behavior.clone(),
                    step_idx: trace.step_idx,
                    wakeup_id: trace.wakeup_id.clone(),
                },
                call,
            )
            .await
        {
            Ok(memory) => {
                debug!(
                    "ai_agent.memory loaded: did={} wakeup_id={} step={}",
                    self.did, trace.wakeup_id, trace.step_idx
                );
                json!({ "memory": memory })
            }
            Err(err) => {
                warn!(
                    "ai_agent.memory load_failed: did={} wakeup_id={} step={} err={}",
                    self.did, trace.wakeup_id, trace.step_idx, err
                );
                json!({ "memory_error": err.to_string() })
            }
        }
    }

    async fn execute_actions(
        &self,
        trace: &TraceCtx,
        actions: &[crate::behavior::ActionSpec],
    ) -> Vec<Observation> {
        if actions.is_empty() {
            return vec![];
        }
        info!(
            "ai_agent.actions start: did={} wakeup_id={} step={} actions={}",
            self.did,
            trace.wakeup_id,
            trace.step_idx,
            actions.len()
        );

        let mut output = Vec::<Observation>::new();
        let mut parallel_batch = Vec::<crate::behavior::ActionSpec>::new();
        for action in actions {
            match action.execution_mode {
                crate::behavior::ActionExecutionMode::Serial => {
                    if !parallel_batch.is_empty() {
                        debug!(
                            "ai_agent.actions flush_parallel: did={} wakeup_id={} step={} batch={}",
                            self.did,
                            trace.wakeup_id,
                            trace.step_idx,
                            parallel_batch.len()
                        );
                        output.extend(
                            run_parallel_action_batch(
                                self.tool_mgr.clone(),
                                trace.clone(),
                                std::mem::take(&mut parallel_batch),
                            )
                            .await,
                        );
                    }
                    output.push(
                        run_single_action(self.tool_mgr.clone(), trace.clone(), action.clone())
                            .await,
                    );
                }
                crate::behavior::ActionExecutionMode::Parallel => {
                    parallel_batch.push(action.clone());
                }
            }
        }
        if !parallel_batch.is_empty() {
            debug!(
                "ai_agent.actions flush_parallel: did={} wakeup_id={} step={} batch={}",
                self.did,
                trace.wakeup_id,
                trace.step_idx,
                parallel_batch.len()
            );
            output.extend(
                run_parallel_action_batch(
                    self.tool_mgr.clone(),
                    trace.clone(),
                    std::mem::take(&mut parallel_batch),
                )
                .await,
            );
        }
        let ok_count = output.iter().filter(|obs| obs.ok).count();
        let failed_count = output.len().saturating_sub(ok_count);
        if failed_count > 0 {
            warn!(
                "ai_agent.actions finish: did={} wakeup_id={} step={} total={} ok={} failed={}",
                self.did,
                trace.wakeup_id,
                trace.step_idx,
                output.len(),
                ok_count,
                failed_count
            );
        } else {
            info!(
                "ai_agent.actions finish: did={} wakeup_id={} step={} total={} ok={}",
                self.did,
                trace.wakeup_id,
                trace.step_idx,
                output.len(),
                ok_count
            );
        }
        output
    }

    async fn consume_hp(&self, token_total: u32, action_count: u32) {
        let token_cost = token_total.saturating_mul(self.cfg.hp_per_token);
        let action_cost = action_count.saturating_mul(self.cfg.hp_per_action);
        let hp_cost = token_cost.saturating_add(action_cost);

        let mut guard = self.state.lock().await;
        let before = guard.hp;
        guard.hp = guard.hp.saturating_sub(hp_cost);
        debug!(
            "ai_agent.hp consume: did={} token_total={} action_count={} cost={} hp={}=>{}",
            self.did, token_total, action_count, hp_cost, before, guard.hp
        );
        if guard.hp == 0 {
            guard.enabled = false;
            warn!("ai_agent.hp exhausted: did={} enabled=false", self.did);
        }
    }

    async fn current_hp(&self) -> u32 {
        self.state.lock().await.hp
    }

    async fn update_sleep_after_wakeup(&self, status: &WakeupStatus) -> u64 {
        let mut guard = self.state.lock().await;
        let before = guard.next_sleep_ms;
        match status {
            WakeupStatus::SkippedNoInput => {
                guard.next_sleep_ms = (guard.next_sleep_ms.saturating_mul(2))
                    .clamp(self.cfg.default_sleep_ms, self.cfg.max_sleep_ms);
            }
            WakeupStatus::Completed | WakeupStatus::SafeStop | WakeupStatus::Error => {
                guard.next_sleep_ms = self.cfg.default_sleep_ms;
            }
            WakeupStatus::Disabled => {}
        }
        debug!(
            "ai_agent.sleep update: did={} status={} {}=>{}",
            self.did,
            wakeup_status_name(status),
            before,
            guard.next_sleep_ms
        );
        guard.next_sleep_ms
    }
}

enum PreparedWakeup {
    Disabled {
        wakeup_id: String,
        trace_id: String,
        hp: u32,
        sleep_ms: u64,
        reason: Json,
    },
    SkippedNoInput {
        wakeup_id: String,
        trace_id: String,
        hp: u32,
        sleep_ms: u64,
        reason: Json,
    },
    Ready {
        wakeup_id: String,
        hp_before: u32,
        trace_id: String,
        input_payload: Json,
        inbox_record_ids: Vec<String>,
    },
}

struct AgentLoopState {
    wakeup_started: Instant,
    wakeup_id: String,
    hp_before: u32,
    trace_id: String,
    input_payload: Json,
    inbox_record_ids: Vec<String>,
    loop_ctx: WakeupLoopContext,
    status: WakeupStatus,
    last_error: Option<String>,
    token_usage: TokenUsage,
    steps: u32,
    behavior_hops: u32,
    current_behavior: String,
    pending_observations: Vec<Observation>,
    memory_queries: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum SessionLoopPhase {
    CollectInput,
    ResolveRouter,
    RunBehaviorStep,
    ApplyStepEffects,
    Waiting,
}

impl SessionLoopPhase {
    fn as_str(&self) -> &'static str {
        match self {
            Self::CollectInput => "collect_input",
            Self::ResolveRouter => "resolve_router",
            Self::RunBehaviorStep => "run_behavior_step",
            Self::ApplyStepEffects => "apply_step_effects",
            Self::Waiting => "waiting",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
struct WaitSpec {
    kind: String,
    events: Vec<String>,
    note: Option<String>,
}

impl Default for WaitSpec {
    fn default() -> Self {
        Self {
            kind: "input".to_string(),
            events: vec!["message".to_string(), "event".to_string()],
            note: None,
        }
    }
}

impl WaitSpec {
    fn default_no_input() -> Self {
        Self {
            note: Some("no_input_for_next_step".to_string()),
            ..Self::default()
        }
    }

    fn default_router_sleep() -> Self {
        Self {
            kind: "router_sleep".to_string(),
            events: vec!["message".to_string(), "event".to_string()],
            note: Some("resolve_router_requested_sleep".to_string()),
        }
    }

    fn default_session_loop_busy(active_session: &str) -> Self {
        Self {
            kind: "session_loop_busy".to_string(),
            events: vec!["message".to_string(), "event".to_string()],
            note: Some(format!("active_session_loop={active_session}")),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
struct SessionRuntimeState {
    agent_id: String,
    session_id: String,
    current_behavior: String,
    step_idx: u32,
    phase: SessionLoopPhase,
    waiting: Option<WaitSpec>,
    last_input: Json,
}

impl Default for SessionRuntimeState {
    fn default() -> Self {
        Self {
            agent_id: String::new(),
            session_id: String::new(),
            current_behavior: BEHAVIOR_ON_WAKEUP.to_string(),
            step_idx: 0,
            phase: SessionLoopPhase::CollectInput,
            waiting: None,
            last_input: json!({}),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct SessionRuntimeStore {
    agent_id: String,
    active_session_loop: Option<String>,
    sessions: HashMap<String, SessionRuntimeState>,
    updated_at_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct ResolveRouterResult {
    session_id: Option<String>,
    //(title,description)
    new_session: Option<(String, String)>,
    next_behavior: Option<String>,
    memory_queries: Vec<String>,
    reply: Option<String>,
}

impl ResolveRouterResult {
    fn is_sleep(&self) -> bool {
        self.next_behavior.as_deref() == Some("END") || self.next_behavior.is_none()
    }
}

#[derive(Clone, Debug)]
struct ModeSelectionResult {
    behavior: String,
    source: &'static str,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct WakeupLoopContext {
    session_id: String,
    event_id: String,
    recent_turns: Vec<Json>,
}
struct PulledInboxMessage {
    input: Json,
    record_id: String,
}

#[derive(Debug)]
struct MsgCenterReplyTarget {
    target_did: DID,
    send_ctx: Option<SendContext>,
    thread_key: Option<String>,
    record_id: Option<String>,
    origin_msg_id: Option<String>,
}

struct WhitespaceTokenizer;

impl Tokenizer for WhitespaceTokenizer {
    fn count_tokens(&self, text: &str) -> u32 {
        text.split_whitespace().count() as u32
    }
}

struct JsonlFileWorklogSink {
    worklog_path: PathBuf,
    write_lock: Mutex<()>,
}

impl JsonlFileWorklogSink {
    async fn new(worklog_path: PathBuf) -> Result<Self, AIAgentError> {
        if let Some(parent) = worklog_path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|source| AIAgentError::Io {
                    path: parent.display().to_string(),
                    source,
                })?;
        }
        info!("ai_agent.worklog init: path={}", worklog_path.display());
        Ok(Self {
            worklog_path,
            write_lock: Mutex::new(()),
        })
    }
}

#[async_trait]
impl WorklogSink for JsonlFileWorklogSink {
    async fn emit(&self, event: AgentWorkEvent) {
        let line = json!({
            "ts": now_ms(),
            "event": worklog_event_to_json(event),
        })
        .to_string()
            + "\n";

        let _guard = self.write_lock.lock().await;
        let mut old = match fs::read_to_string(&self.worklog_path).await {
            Ok(content) => content,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(err) => {
                warn!(
                    "ai_agent.worklog read_failed: path={} err={}",
                    self.worklog_path.display(),
                    err
                );
                String::new()
            }
        };
        old.push_str(&line);
        if let Err(err) = fs::write(&self.worklog_path, old.as_bytes()).await {
            warn!(
                "ai_agent.worklog write_failed: path={} err={}",
                self.worklog_path.display(),
                err
            );
        }
    }
}

fn worklog_event_to_json(event: AgentWorkEvent) -> Json {
    match event {
        AgentWorkEvent::LLMStarted { trace, model } => json!({
            "kind": "llm_started",
            "trace": trace_to_json(trace),
            "model": model
        }),
        AgentWorkEvent::LLMFinished { trace, usage, ok } => json!({
            "kind": "llm_finished",
            "trace": trace_to_json(trace),
            "usage": {
                "prompt": usage.prompt,
                "completion": usage.completion,
                "total": usage.total
            },
            "ok": ok
        }),
        AgentWorkEvent::ToolCallPlanned {
            trace,
            tool,
            call_id,
        } => json!({
            "kind": "tool_call_planned",
            "trace": trace_to_json(trace),
            "tool": tool,
            "call_id": call_id
        }),
        AgentWorkEvent::ToolCallFinished {
            trace,
            tool,
            call_id,
            ok,
            duration_ms,
        } => json!({
            "kind": "tool_call_finished",
            "trace": trace_to_json(trace),
            "tool": tool,
            "call_id": call_id,
            "ok": ok,
            "duration_ms": duration_ms
        }),
        AgentWorkEvent::ParseWarning { trace, msg } => json!({
            "kind": "parse_warning",
            "trace": trace_to_json(trace),
            "msg": msg
        }),
    }
}

fn trace_to_json(trace: TraceCtx) -> Json {
    json!({
        "trace_id": trace.trace_id,
        "agent_did": trace.agent_did,
        "behavior": trace.behavior,
        "step_idx": trace.step_idx,
        "wakeup_id": trace.wakeup_id
    })
}

async fn run_parallel_action_batch(
    tool_mgr: Arc<ToolManager>,
    trace: TraceCtx,
    actions: Vec<crate::behavior::ActionSpec>,
) -> Vec<Observation> {
    debug!(
        "ai_agent.actions parallel_batch_start: wakeup_id={} step={} size={}",
        trace.wakeup_id,
        trace.step_idx,
        actions.len()
    );
    let mut join_set = JoinSet::new();
    for action in actions {
        let tool_mgr = tool_mgr.clone();
        let trace = trace.clone();
        join_set.spawn(async move { run_single_action(tool_mgr, trace, action).await });
    }

    let mut out = Vec::new();
    while let Some(joined) = join_set.join_next().await {
        match joined {
            Ok(observation) => out.push(observation),
            Err(err) => {
                error!(
                    "ai_agent.actions parallel_join_failed: wakeup_id={} step={} err={}",
                    trace.wakeup_id, trace.step_idx, err
                );
                out.push(Observation {
                    source: ObservationSource::Action,
                    name: "parallel_action".to_string(),
                    content: json!({
                        "ok": false,
                        "error": format!("join parallel action failed: {err}")
                    }),
                    ok: false,
                    truncated: false,
                    bytes: 0,
                })
            }
        }
    }
    debug!(
        "ai_agent.actions parallel_batch_finish: wakeup_id={} step={} observations={}",
        trace.wakeup_id,
        trace.step_idx,
        out.len()
    );
    out
}

async fn run_single_action(
    tool_mgr: Arc<ToolManager>,
    trace: TraceCtx,
    action: crate::behavior::ActionSpec,
) -> Observation {
    let action_title = action.title.clone();
    let action_command = action.command.clone();
    debug!(
        "ai_agent.action start: wakeup_id={} step={} behavior={} title={} timeout_ms={}",
        trace.wakeup_id, trace.step_idx, trace.behavior, action_title, action.timeout_ms
    );
    let mut args = json!({
        "command": action.command,
        "timeout_ms": action.timeout_ms
    });
    if let Some(cwd) = action.cwd.as_ref() {
        args["cwd"] = json!(cwd);
    }

    let ctx = TraceCtx {
        trace_id: trace.trace_id.clone(),
        agent_did: trace.agent_did.clone(),
        behavior: trace.behavior.clone(),
        step_idx: trace.step_idx,
        wakeup_id: trace.wakeup_id.clone(),
    };
    append_workspace_worklog_entry(
        tool_mgr.clone(),
        &ctx,
        "action",
        "info",
        format!("action `{}` started", action_title),
        json!({
            "title": action.title,
            "command": action.command,
            "execution_mode": action.execution_mode,
            "timeout_ms": action.timeout_ms,
            "cwd": action.cwd,
            "rationale": action.rationale
        }),
        None,
        vec!["action".to_string(), "exec".to_string()],
        None,
        None,
    )
    .await;

    let call_id = format!(
        "{}-{}-{}",
        trace.wakeup_id,
        trace.step_idx,
        now_ms().saturating_sub(1)
    );
    let result = tool_mgr
        .call_tool(
            &ctx,
            ToolCall {
                name: TOOL_EXEC_BASH.to_string(),
                args,
                call_id,
            },
        )
        .await;

    match result {
        Ok(raw) => {
            let ok = raw.get("ok").and_then(|v| v.as_bool()).unwrap_or(true);
            if ok {
                info!(
                    "ai_agent.action finish: wakeup_id={} step={} title={} ok=true",
                    trace.wakeup_id, trace.step_idx, action_title
                );
            } else {
                warn!(
                    "ai_agent.action finish: wakeup_id={} step={} title={} ok=false raw={}",
                    trace.wakeup_id, trace.step_idx, action_title, raw
                );
            }
            append_workspace_worklog_entry(
                tool_mgr.clone(),
                &ctx,
                "action",
                if ok { "success" } else { "failed" },
                format!(
                    "action `{}` {}",
                    action_title,
                    if ok { "succeeded" } else { "failed" }
                ),
                json!({
                    "title": action.title,
                    "command": action.command,
                    "execution_mode": action.execution_mode,
                    "rationale": action.rationale,
                    "result": compact_json_for_worklog(raw.clone(), 8 * 1024)
                }),
                None,
                vec!["action".to_string(), "exec".to_string()],
                None,
                None,
            )
            .await;
            let content = json!({
                "ok": ok,
                "title": action.title,
                "command": action.command,
                "execution_mode": action.execution_mode,
                "rationale": action.rationale,
                "result": raw
            });
            Observation {
                source: ObservationSource::Action,
                name: action.title,
                bytes: serde_json::to_vec(&content).map(|v| v.len()).unwrap_or(0),
                content,
                ok,
                truncated: false,
            }
        }
        Err(err) => {
            error!(
                "ai_agent.action failed: wakeup_id={} step={} title={} command={} err={}",
                trace.wakeup_id, trace.step_idx, action_title, action_command, err
            );
            append_workspace_worklog_entry(
                tool_mgr.clone(),
                &ctx,
                "action",
                "failed",
                format!("action `{}` failed", action_title),
                json!({
                    "title": action.title,
                    "command": action.command,
                    "execution_mode": action.execution_mode,
                    "error": err.to_string()
                }),
                None,
                vec!["action".to_string(), "exec".to_string()],
                None,
                None,
            )
            .await;
            let content = json!({
                "ok": false,
                "title": action.title,
                "command": action.command,
                "execution_mode": action.execution_mode,
                "error": err.to_string()
            });
            Observation {
                source: ObservationSource::Action,
                name: action.title,
                bytes: serde_json::to_vec(&content).map(|v| v.len()).unwrap_or(0),
                content,
                ok: false,
                truncated: false,
            }
        }
    }
}

async fn preload_behavior_configs(
    behaviors_dir: &Path,
    cache: Arc<RwLock<HashMap<String, BehaviorConfig>>>,
) -> Result<(), AIAgentError> {
    let names = discover_behavior_names(behaviors_dir).await?;
    info!(
        "ai_agent.behavior preload_start: dir={} discovered={}",
        behaviors_dir.display(),
        names.len()
    );
    let mut loaded = HashMap::<String, BehaviorConfig>::new();
    for name in names {
        let cfg = BehaviorConfig::load_from_dir(behaviors_dir, &name).await?;
        loaded.insert(name, cfg);
    }
    let mut guard = cache.write().await;
    guard.extend(loaded);
    info!(
        "ai_agent.behavior preload_finish: dir={} loaded={}",
        behaviors_dir.display(),
        guard.len()
    );
    Ok(())
}

async fn discover_behavior_names(behaviors_dir: &Path) -> Result<Vec<String>, AIAgentError> {
    let exists = fs::try_exists(behaviors_dir)
        .await
        .map_err(|source| AIAgentError::Io {
            path: behaviors_dir.display().to_string(),
            source,
        })?;
    if !exists {
        info!(
            "ai_agent.behavior discover: dir={} not_found -> empty",
            behaviors_dir.display()
        );
        return Ok(vec![]);
    }

    let mut names = Vec::<String>::new();
    let mut read_dir = fs::read_dir(behaviors_dir)
        .await
        .map_err(|source| AIAgentError::Io {
            path: behaviors_dir.display().to_string(),
            source,
        })?;

    while let Some(entry) = read_dir
        .next_entry()
        .await
        .map_err(|source| AIAgentError::Io {
            path: behaviors_dir.display().to_string(),
            source,
        })?
    {
        let path = entry.path();
        let file_type = entry.file_type().await.map_err(|source| AIAgentError::Io {
            path: path.display().to_string(),
            source,
        })?;
        if !file_type.is_file() {
            continue;
        }

        let Some(ext) = path.extension().and_then(|v| v.to_str()) else {
            continue;
        };
        let lower = ext.to_ascii_lowercase();
        if lower != "yaml" && lower != "yml" && lower != "json" {
            continue;
        }

        if let Some(stem) = path.file_stem().and_then(|v| v.to_str()) {
            names.push(stem.to_string());
        }
    }

    names.sort();
    names.dedup();
    debug!(
        "ai_agent.behavior discover: dir={} names={:?}",
        behaviors_dir.display(),
        names
    );
    Ok(names)
}

async fn normalize_agent_root(root: &Path) -> Result<PathBuf, AIAgentError> {
    let abs = if root.is_absolute() {
        root.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|source| AIAgentError::Io {
                path: ".".to_string(),
                source,
            })?
            .join(root)
    };
    fs::create_dir_all(&abs)
        .await
        .map_err(|source| AIAgentError::Io {
            path: abs.display().to_string(),
            source,
        })?;
    Ok(normalize_abs_path(&abs))
}

async fn load_agent_doc(agent_root: &Path) -> Result<Json, AIAgentError> {
    for name in AGENT_DOC_CANDIDATES {
        let path = agent_root.join(name);
        if !fs::try_exists(&path)
            .await
            .map_err(|source| AIAgentError::Io {
                path: path.display().to_string(),
                source,
            })?
        {
            continue;
        }
        let content = fs::read_to_string(&path)
            .await
            .map_err(|source| AIAgentError::Io {
                path: path.display().to_string(),
                source,
            })?;
        let parsed =
            serde_json::from_str::<Json>(&content).map_err(|source| AIAgentError::Json {
                path: path.display().to_string(),
                source,
            })?;
        info!("ai_agent.did_doc loaded: path={}", path.display());
        return Ok(parsed);
    }
    warn!(
        "ai_agent.did_doc missing: root={} candidates={:?}",
        agent_root.display(),
        AGENT_DOC_CANDIDATES
    );
    Ok(json!({}))
}

fn extract_agent_did(did_document: &Json, agent_root: &Path) -> String {
    did_document
        .get("id")
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
        .or_else(|| {
            did_document
                .get("did")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string())
        })
        .unwrap_or_else(|| {
            let dir_name = agent_root
                .file_name()
                .and_then(|v| v.to_str())
                .unwrap_or("agent");
            format!("did:opendan:{dir_name}")
        })
}

async fn load_text_or_empty(path: PathBuf) -> Result<String, AIAgentError> {
    match fs::read_to_string(&path).await {
        Ok(text) => Ok(text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            debug!("ai_agent.text optional_missing: path={}", path.display());
            Ok(String::new())
        }
        Err(source) => Err(AIAgentError::Io {
            path: path.display().to_string(),
            source,
        }),
    }
}

async fn load_session_runtime_store(path: &Path, agent_id: &str) -> SessionRuntimeStore {
    let mut default = SessionRuntimeStore {
        agent_id: agent_id.to_string(),
        active_session_loop: None,
        sessions: HashMap::new(),
        updated_at_ms: now_ms(),
    };

    let raw = match fs::read_to_string(path).await {
        Ok(v) => v,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return default;
        }
        Err(err) => {
            warn!(
                "ai_agent.session_runtime load_failed: path={} err={}",
                path.display(),
                err
            );
            return default;
        }
    };

    let mut parsed = match serde_json::from_str::<SessionRuntimeStore>(&raw) {
        Ok(v) => v,
        Err(err) => {
            warn!(
                "ai_agent.session_runtime parse_failed: path={} err={}",
                path.display(),
                err
            );
            return default;
        }
    };

    if parsed.agent_id.trim().is_empty() {
        parsed.agent_id = agent_id.to_string();
    }
    // Never restore active execution slot from disk, otherwise stale value may
    // block all session loops after an unexpected restart.
    parsed.active_session_loop = None;
    for (session_id, runtime) in parsed.sessions.iter_mut() {
        if runtime.agent_id.trim().is_empty() {
            runtime.agent_id = parsed.agent_id.clone();
        }
        if runtime.session_id.trim().is_empty() {
            runtime.session_id = session_id.clone();
        }
    }
    default.sessions = parsed.sessions;
    default.updated_at_ms = parsed.updated_at_ms;
    default
}

fn normalize_abs_path(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                let _ = normalized.pop();
            }
            Component::Normal(seg) => normalized.push(seg),
        }
    }
    normalized
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn wakeup_status_name(status: &WakeupStatus) -> &'static str {
    match status {
        WakeupStatus::SkippedNoInput => "skipped_no_input",
        WakeupStatus::Completed => "completed",
        WakeupStatus::SafeStop => "safe_stop",
        WakeupStatus::Error => "error",
        WakeupStatus::Disabled => "disabled",
    }
}

async fn append_workspace_worklog_entry(
    tool_mgr: Arc<ToolManager>,
    ctx: &TraceCtx,
    log_type: &str,
    status: &str,
    summary: String,
    payload: Json,
    session_id: Option<String>,
    tags: Vec<String>,
    related_agent_id: Option<String>,
    task_id: Option<String>,
) {
    if !tool_mgr.has_tool(TOOL_WORKLOG_MANAGE) {
        return;
    }

    let mut args = json!({
        "action": "append",
        "type": log_type,
        "status": status,
        "agent_id": ctx.agent_did.clone(),
        "run_id": ctx.wakeup_id.clone(),
        "step_id": format!("step-{}", ctx.step_idx),
        "summary": summary,
        "payload": compact_json_for_worklog(payload, 8 * 1024),
        "tags": tags,
        "timestamp": now_ms()
    });
    if let Some(v) = session_id {
        args["owner_session_id"] = Json::String(v);
    }
    if let Some(v) = related_agent_id {
        args["related_agent_id"] = Json::String(v);
    }
    if let Some(v) = task_id {
        args["task_id"] = Json::String(v);
    }

    let call = ToolCall {
        name: TOOL_WORKLOG_MANAGE.to_string(),
        args,
        call_id: format!("{}-{}-wl-{}", ctx.wakeup_id, ctx.step_idx, now_ms()),
    };
    if let Err(err) = tool_mgr.call_tool(ctx, call).await {
        warn!(
            "ai_agent.worklog_append_failed: did={} wakeup_id={} step={} behavior={} err={}",
            ctx.agent_did, ctx.wakeup_id, ctx.step_idx, ctx.behavior, err
        );
    }
}

fn compact_json_for_worklog(value: Json, max_bytes: usize) -> Json {
    match serde_json::to_vec(&value) {
        Ok(bytes) if bytes.len() > max_bytes => {
            let text = String::from_utf8_lossy(&bytes);
            Json::String(format!(
                "{}...[TRUNCATED]",
                text.chars().take(max_bytes).collect::<String>()
            ))
        }
        Ok(_) => value,
        Err(_) => json!({"error":"serialize_failed"}),
    }
}

fn extract_session_id_from_inbox_payload(payload: &Json) -> Option<String> {
    payload
        .pointer("/msg/session_id")
        .or_else(|| payload.pointer("/record/session_id"))
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn extract_session_id_from_message_payload(payload: &Json) -> Option<String> {
    payload
        .get("session_id")
        .or_else(|| payload.get("session"))
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn msg_center_record_to_inbox_message(record: MsgRecordWithObject) -> PulledInboxMessage {
    let record_id = record.record.record_id.clone();
    let input = json!({
        "source": "msg_center.krpc",
        "record": record.record,
        "msg": record.msg
    });
    PulledInboxMessage { input, record_id }
}

fn extract_msg_center_reply_target(payload: &Json) -> Option<MsgCenterReplyTarget> {
    let inbox = payload.get("inbox").and_then(|v| v.as_array())?;
    for item in inbox.iter().rev() {
        if item
            .get("source")
            .and_then(|v| v.as_str())
            .map(|v| v != "msg_center.krpc")
            .unwrap_or(true)
        {
            continue;
        }

        let raw_target = item
            .pointer("/msg/source")
            .or_else(|| item.pointer("/msg/from"))
            .and_then(|v| v.as_str())
            .map(|v| v.trim())
            .filter(|v| !v.is_empty());
        let Some(raw_target) = raw_target else {
            continue;
        };
        let target_did = match DID::from_str(raw_target) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let preferred_tunnel = item
            .pointer("/record/route/tunnel_did")
            .and_then(|v| v.as_str())
            .and_then(|v| DID::from_str(v.trim()).ok());
        let priority = item
            .pointer("/record/route/priority")
            .and_then(|v| v.as_i64())
            .and_then(|v| i32::try_from(v).ok());
        let mut extra = item.pointer("/record/route/extra").cloned();
        if let Some(route_snapshot) = item.pointer("/record/route").cloned() {
            extra = Some(json!({
                "source": "msg_center.reply.original_route",
                "route": route_snapshot
            }));
        }
        let send_ctx = Some(SendContext {
            context_id: None,
            preferred_tunnel,
            priority,
            extra,
        });
        let thread_key = item
            .pointer("/msg/thread_key")
            .or_else(|| item.pointer("/record/thread_key"))
            .and_then(|v| v.as_str())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let record_id = item
            .pointer("/record/record_id")
            .and_then(|v| v.as_str())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let origin_msg_id = item
            .pointer("/record/msg_id")
            .and_then(|v| v.as_str())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());

        return Some(MsgCenterReplyTarget {
            target_did,
            send_ctx,
            thread_key,
            record_id,
            origin_msg_id,
        });
    }
    None
}

fn build_msg_center_reply_obj_id() -> Option<ObjId> {
    let seq = REPLY_MSG_SEQ.fetch_add(1, Ordering::Relaxed);
    let raw = format!("chunk:{:016x}", now_ms().saturating_add(seq));
    ObjId::new(raw.as_str()).ok()
}

fn wakeup_input_counts(payload: &Json) -> (usize, usize) {
    let inbox = payload
        .get("inbox")
        .and_then(|v| v.as_array())
        .map(|v| v.len())
        .unwrap_or(0);
    let events = payload
        .get("events")
        .and_then(|v| v.as_array())
        .map(|v| v.len())
        .unwrap_or(0);
    (inbox, events)
}

fn derive_wakeup_loop_context(wakeup_id: &str, payload: &Json) -> WakeupLoopContext {
    let session_id = extract_session_id_from_wakeup_payload(payload)
        .unwrap_or_else(|| format!("session-{wakeup_id}"));
    let event_id = extract_event_id_from_wakeup_payload(payload)
        .unwrap_or_else(|| format!("event-{wakeup_id}"));
    let recent_turns = collect_recent_turns(payload, MAX_RECENT_TURNS);
    WakeupLoopContext {
        session_id,
        event_id,
        recent_turns,
    }
}

fn enrich_wakeup_payload_with_loop_context(payload: &mut Json, loop_ctx: &WakeupLoopContext) {
    if !payload.is_object() {
        *payload = json!({ "trigger": "on_wakeup", "raw_payload": payload.clone() });
    }
    payload["session_id"] = json!(loop_ctx.session_id);
    payload["event_id"] = json!(loop_ctx.event_id);
    payload["recent_turns"] = json!(loop_ctx.recent_turns);
}

fn extract_session_id_from_wakeup_payload(payload: &Json) -> Option<String> {
    payload
        .get("session_id")
        .or_else(|| payload.get("session"))
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| {
            payload
                .pointer("/record/session_id")
                .and_then(|v| v.as_str())
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        })
        .or_else(|| {
            payload
                .get("inbox")
                .and_then(|v| v.as_array())
                .and_then(|items| {
                    items.iter().find_map(|item| {
                        extract_session_id_from_message_payload(item)
                            .or_else(|| extract_session_id_from_inbox_payload(item))
                    })
                })
        })
}

fn extract_event_id_from_wakeup_payload(payload: &Json) -> Option<String> {
    payload
        .get("event_id")
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| {
            payload
                .get("inbox")
                .and_then(|v| v.as_array())
                .and_then(|inbox| {
                    inbox.iter().find_map(|msg| {
                        msg.get("event_id")
                            .or_else(|| msg.get("id"))
                            .or_else(|| msg.pointer("/record/record_id"))
                            .and_then(|v| v.as_str())
                            .map(|v| v.trim().to_string())
                            .filter(|v| !v.is_empty())
                    })
                })
        })
}

fn collect_recent_turns(payload: &Json, max_turns: usize) -> Vec<Json> {
    let mut turns = payload
        .get("inbox")
        .and_then(|v| v.as_array())
        .map(|inbox| {
            inbox
                .iter()
                .filter_map(|item| {
                    extract_message_text(item).map(|text| {
                        json!({
                            "role": "user",
                            "text": text
                        })
                    })
                })
                .collect::<Vec<Json>>()
        })
        .unwrap_or_default();
    if turns.len() > max_turns {
        turns = turns.split_off(turns.len().saturating_sub(max_turns));
    }
    turns
}

fn extract_message_text(value: &Json) -> Option<String> {
    let candidate = value
        .get("text")
        .or_else(|| value.get("message"))
        .or_else(|| value.pointer("/msg/text"))
        .or_else(|| value.pointer("/msg/body"))
        .or_else(|| value.pointer("/record/summary"))
        .or_else(|| value.pointer("/payload/text"))
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_string())?;
    if candidate.is_empty() {
        return None;
    }
    Some(candidate)
}

fn llm_output_to_json(output: &LLMOutput) -> Option<Json> {
    match output {
        LLMOutput::Json(value) => Some(value.clone()),
        LLMOutput::Text(text) => serde_json::from_str::<Json>(text).ok(),
    }
}

fn parse_resolve_router_result(output: &LLMOutput) -> Option<ResolveRouterResult> {
    llm_output_to_json(output).and_then(|value| serde_json::from_value(value).ok())
}

fn sanitize_non_empty_string(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn append_unique_strings(target: &mut Vec<String>, values: &[String]) {
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !target.iter().any(|item| item == trimmed) {
            target.push(trimmed.to_string());
        }
    }
}

fn sanitize_tool_calls(calls: &[ToolCall]) -> Vec<ToolCall> {
    calls
        .iter()
        .filter_map(|call| {
            let name = call.name.trim();
            if name.is_empty() {
                return None;
            }
            Some(ToolCall {
                name: name.to_string(),
                args: call.args.clone(),
                call_id: call.call_id.trim().to_string(),
            })
        })
        .collect::<Vec<_>>()
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, VecDeque};
    use std::sync::{Arc, Mutex};

    use buckyos_api::{
        AiResponseSummary, AiUsage, CompleteRequest, CompleteResponse, CompleteStatus,
    };
    use rusqlite::Connection;
    use serde_json::json;
    use tempfile::tempdir;
    use tokio::fs;

    use super::*;
    use crate::agent_session::{TOOL_CREATE_SESSION, TOOL_UPDATE_SESSION};
    use crate::test_utils::{MockAicc, MockTaskMgrHandler};
    use crate::workspace::{TOOL_EXEC_BASH, TOOL_TODO_MANAGE};

    fn mocked_response(
        payload: Json,
        prompt_tokens: u32,
        completion_tokens: u32,
    ) -> CompleteResponse {
        CompleteResponse::new(
            "".to_string(),
            CompleteStatus::Succeeded,
            Some(AiResponseSummary {
                text: None,
                json: Some(payload),
                artifacts: vec![],
                usage: Some(AiUsage {
                    input_tokens: Some(prompt_tokens as u64),
                    output_tokens: Some(completion_tokens as u64),
                    total_tokens: Some((prompt_tokens + completion_tokens) as u64),
                }),
                cost: None,
                finish_reason: Some("stop".to_string()),
                provider_task_ref: Some("mock-provider-task".to_string()),
                extra: Some(json!({
                    "provider": "mock",
                    "model": "mock-1",
                    "latency_ms": 12
                })),
            }),
            None,
        )
    }

    #[test]
    fn extract_msg_center_reply_target_uses_latest_msg_center_item() {
        let payload = json!({
            "inbox": [
                {
                    "source": "runtime.push_inbox_message",
                    "from": "did:example:alice",
                    "text": "hello"
                },
                {
                    "source": "msg_center.krpc",
                    "record": {
                        "record_id": "record-1",
                        "msg_id": "chunk:0001",
                        "thread_key": "thread-1",
                        "route": {
                            "tunnel_did": "did:example:tunnel-a",
                            "priority": 3,
                            "chat_id": "10086"
                        }
                    },
                    "msg": {
                        "from": "did:example:old-user",
                        "thread_key": "thread-1"
                    }
                },
                {
                    "source": "msg_center.krpc",
                    "record": {
                        "record_id": "record-2",
                        "msg_id": "chunk:0002",
                        "route": {
                            "tunnel_did": "did:example:tunnel-b",
                            "priority": 7
                        }
                    },
                    "msg": {
                        "source": "did:example:target-user",
                        "from": "did:example:group"
                    }
                }
            ]
        });

        let target =
            extract_msg_center_reply_target(&payload).expect("extract latest msg_center target");
        assert_eq!(target.target_did.to_string(), "did:example:target-user");
        assert_eq!(target.record_id.as_deref(), Some("record-2"));
        assert_eq!(target.origin_msg_id.as_deref(), Some("chunk:0002"));
        assert_eq!(target.thread_key, None);

        let send_ctx = target.send_ctx.expect("send_ctx");
        assert_eq!(
            send_ctx
                .preferred_tunnel
                .expect("preferred tunnel")
                .to_string(),
            "did:example:tunnel-b"
        );
        assert_eq!(send_ctx.priority, Some(7));
        assert!(send_ctx.extra.is_some());
    }

    #[test]
    fn extract_msg_center_reply_target_returns_none_without_msg_center_inbox() {
        let payload = json!({
            "inbox": [{
                "source": "runtime.push_inbox_message",
                "from": "did:example:alice",
                "text": "hello"
            }]
        });
        assert!(extract_msg_center_reply_target(&payload).is_none());
    }

    async fn write_agent_fixture(agent_root: &Path) {
        fs::create_dir_all(agent_root.join("behaviors"))
            .await
            .expect("create behaviors dir");
        fs::create_dir_all(agent_root.join("environment/tools"))
            .await
            .expect("create environment tools dir");
        fs::create_dir_all(agent_root.join("memory"))
            .await
            .expect("create memory dir");

        fs::write(
            agent_root.join("agent.json.doc"),
            json!({
                "id": "did:example:jarvis",
                "name": "Jarvis",
                "description": "fixture agent for wakeup e2e"
            })
            .to_string(),
        )
        .await
        .expect("write agent.json.doc");

        fs::write(
            agent_root.join("role.md"),
            "# Role\nYou are Jarvis.\nFocus on safe automation and deliverables.\n",
        )
        .await
        .expect("write role.md");

        fs::write(
            agent_root.join("self.md"),
            "# Self\n- Keep traces auditable\n- Prefer concise outputs\n",
        )
        .await
        .expect("write self.md");

        fs::write(
            agent_root.join("memory/memory.md"),
            "## Long-term memory\n- project: opendan\n- preference: clear status updates\n",
        )
        .await
        .expect("write memory.md");

        fs::write(
            agent_root.join("behaviors/on_wakeup.yaml"),
            r#"
process_rule: test_rule
tools:
  mode: allow_list
  names:
    - todo_manage
limits:
  max_tool_rounds: 2
  max_tool_calls_per_round: 4
  deadline_ms: 60000
"#,
        )
        .await
        .expect("write behavior yaml");

        fs::write(
            agent_root.join("environment/tools/tools.json"),
            json!({
                "enabled_tools": [
                    { "name": TOOL_EXEC_BASH, "enabled": true, "params": {} },
                    { "name": TOOL_TODO_MANAGE, "enabled": true, "params": {} }
                ]
            })
            .to_string(),
        )
        .await
        .expect("write tools.json");
    }

    #[tokio::test]
    async fn ai_agent_manual_wakeup_runs_tool_loop_and_action_with_tmpdir_root() {
        let tmp = tempdir().expect("create tmpdir");
        let agent_root = tmp.path().join("jarvis");
        write_agent_fixture(&agent_root).await;

        let requests = Arc::new(Mutex::new(Vec::<CompleteRequest>::new()));
        let responses = Arc::new(Mutex::new(VecDeque::from(vec![
            mocked_response(
                json!({
                    "tool_calls": [{
                        "name": TOOL_TODO_MANAGE,
                        "args": {
                            "action": "create",
                            "title": "Reply project status",
                            "description": "Prepare status update for Alice",
                            "owner_session_id": null,
                            "status": "in_progress",
                            "priority": "high",
                            "tags": ["inbox", "report"]
                        },
                        "call_id": "call-todo-create-1"
                    }]
                }),
                18,
                7,
            ),
            mocked_response(
                json!({
                    "is_sleep": true,
                    "next_behavior": null,
                    "actions": [{
                        "kind": "bash",
                        "title": "write wakeup artifact",
                        "command": "cat > artifacts/wakeup_report.md <<'MD'\n# Wakeup Report\n- todo item created\n- waiting for follow-up\nMD",
                        "execution_mode": "serial",
                        "cwd": null,
                        "timeout_ms": 20000,
                        "allow_network": false,
                        "fs_scope": {
                            "read_roots": [],
                            "write_roots": ["artifacts"]
                        },
                        "rationale": "deliver one artifact in workspace"
                    }],
                    "output": {
                        "ok": true,
                        "summary": "todo created and artifact generated"
                    }
                }),
                16,
                9,
            ),
        ])));

        let deps = AIAgentDeps {
            taskmgr: Arc::new(TaskManagerClient::new_in_process(Box::new(
                MockTaskMgrHandler {
                    counter: Mutex::new(0),
                    tasks: Arc::new(Mutex::new(HashMap::new())),
                },
            ))),
            aicc: Arc::new(AiccClient::new_in_process(Box::new(MockAicc {
                responses,
                requests: requests.clone(),
            }))),
            msg_center: None,
        };

        let agent = AIAgent::load(AIAgentConfig::new(&agent_root), deps)
            .await
            .expect("load ai agent");
        assert_eq!(agent.did(), "did:example:jarvis");
        assert!(agent
            .list_behavior_names()
            .await
            .iter()
            .any(|name| name == "on_wakeup"));

        agent
            .push_inbox_message(json!({
                "from": "did:example:alice",
                "text": "请整理项目状态并更新待办"
            }))
            .await;
        agent
            .push_event(json!({
                "kind": "task_due",
                "task": "status_report"
            }))
            .await;

        // Manual single wakeup trigger.
        let report = agent.wait_wakeup(None).await.expect("run on_wakeup");
        assert!(matches!(report.status, WakeupStatus::Completed));
        assert_eq!(report.steps, 1);
        assert_eq!(report.behavior_hops, 0);
        assert!(report.token_total > 0);
        assert!(
            report.reason["inbox"]
                .as_array()
                .map(|v| v.len())
                .unwrap_or(0)
                > 0
        );
        assert!(
            report.reason["events"]
                .as_array()
                .map(|v| v.len())
                .unwrap_or(0)
                > 0
        );

        let artifact_path = agent_root.join("environment/artifacts/wakeup_report.md");
        let artifact_content = fs::read_to_string(&artifact_path)
            .await
            .expect("read wakeup artifact");
        assert!(artifact_content.contains("Wakeup Report"));
        assert!(artifact_content.contains("todo item created"));

        let todo_count = tokio::task::spawn_blocking({
            let todo_db_path = agent_root.join("environment/todo/todo.db");
            move || {
                let conn = Connection::open(todo_db_path).expect("open todo db");
                conn.query_row(
                    "SELECT COUNT(1) FROM todos WHERE title = 'Reply project status'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .expect("query todo count")
            }
        })
        .await
        .expect("join todo query");
        assert_eq!(todo_count, 1);

        let requests_guard = requests.lock().expect("requests lock");
        assert_eq!(requests_guard.len(), 2);
        let tool_messages_text = requests_guard[1]
            .payload
            .options
            .as_ref()
            .and_then(|v| v.get("tool_messages"))
            .cloned()
            .unwrap_or_else(|| json!([]))
            .to_string();
        assert!(tool_messages_text.contains(TOOL_TODO_MANAGE));
        assert!(tool_messages_text.contains("Reply project status"));
    }

    #[tokio::test]
    async fn ai_agent_manual_wakeup_runs_staged_loop_when_stage_behaviors_exist() {
        let tmp = tempdir().expect("create tmpdir");
        let agent_root = tmp.path().join("jarvis");
        write_agent_fixture(&agent_root).await;

        fs::write(
            agent_root.join("behaviors/resolve_router.yaml"),
            r#"
process_rule: test_rule
limits:
  max_tool_rounds: 0
  max_tool_calls_per_round: 0
  deadline_ms: 60000
"#,
        )
        .await
        .expect("write resolve_router yaml");

        let requests = Arc::new(Mutex::new(Vec::<CompleteRequest>::new()));
        let responses = Arc::new(Mutex::new(VecDeque::from(vec![
            mocked_response(
                json!({
                    "session_id": "session-router-1",
                    "new_session": null,
                    "next_behavior": "on_wakeup",
                    "memory_queries": ["project status", "todo follow-up", "router query"],
                    "reply": "收到，先整理项目状态。"
                }),
                21,
                17,
            ),
            mocked_response(
                json!({
                    "is_sleep": true,
                    "next_behavior": null,
                    "actions": [{
                        "kind": "bash",
                        "title": "write staged artifact",
                        "command": "cat > artifacts/staged_report.md <<'MD'\n# Staged Report\n- resolver and router executed\nMD",
                        "execution_mode": "serial",
                        "cwd": null,
                        "timeout_ms": 20000,
                        "allow_network": false,
                        "fs_scope": {
                            "read_roots": [],
                            "write_roots": ["artifacts"]
                        },
                        "rationale": "persist staged loop execution marker"
                    }],
                    "output": {
                        "ok": true,
                        "summary": "staged loop done"
                    }
                }),
                12,
                10,
            ),
        ])));

        let deps = AIAgentDeps {
            taskmgr: Arc::new(TaskManagerClient::new_in_process(Box::new(
                MockTaskMgrHandler {
                    counter: Mutex::new(0),
                    tasks: Arc::new(Mutex::new(HashMap::new())),
                },
            ))),
            aicc: Arc::new(AiccClient::new_in_process(Box::new(MockAicc {
                responses,
                requests: requests.clone(),
            }))),
            msg_center: None,
        };

        let agent = AIAgent::load(AIAgentConfig::new(&agent_root), deps)
            .await
            .expect("load ai agent");
        agent
            .push_inbox_message(json!({
                "from": "did:example:alice",
                "text": "请更新项目状态",
                "session_id": "session-user-1"
            }))
            .await;

        let report = agent.wait_wakeup(None).await.expect("run staged wakeup");
        assert!(matches!(report.status, WakeupStatus::Completed));
        assert_eq!(report.steps, 1);
        assert!(report.token_total > 0);
        assert_eq!(report.reason["session_id"], "session-router-1");
        assert_eq!(report.reason["event_id"].is_string(), true);
        assert!(report.reason["memory_queries"]
            .as_array()
            .map(|v| v.iter().any(|q| q == "router query"))
            .unwrap_or(false));

        let artifact_path = agent_root.join("environment/artifacts/staged_report.md");
        let artifact_content = fs::read_to_string(&artifact_path)
            .await
            .expect("read staged artifact");
        assert!(artifact_content.contains("Staged Report"));

        let requests_guard = requests.lock().expect("requests lock");
        assert_eq!(requests_guard.len(), 2);
    }

    #[tokio::test]
    async fn ai_agent_resolve_router_end_uses_fast_sleep_path() {
        let tmp = tempdir().expect("create tmpdir");
        let agent_root = tmp.path().join("jarvis");
        write_agent_fixture(&agent_root).await;

        fs::write(
            agent_root.join("behaviors/resolve_router.yaml"),
            r#"
process_rule: test_rule
limits:
  max_tool_rounds: 0
  max_tool_calls_per_round: 0
  deadline_ms: 60000
"#,
        )
        .await
        .expect("write resolve_router yaml");

        let requests = Arc::new(Mutex::new(Vec::<CompleteRequest>::new()));
        let responses = Arc::new(Mutex::new(VecDeque::from(vec![mocked_response(
            json!({
                "session_id": "session-router-end",
                "new_session": null,
                "next_behavior": "END",
                "memory_queries": ["router done"],
                "reply": "收到，先暂停。"
            }),
            9,
            6,
        )])));

        let deps = AIAgentDeps {
            taskmgr: Arc::new(TaskManagerClient::new_in_process(Box::new(
                MockTaskMgrHandler {
                    counter: Mutex::new(0),
                    tasks: Arc::new(Mutex::new(HashMap::new())),
                },
            ))),
            aicc: Arc::new(AiccClient::new_in_process(Box::new(MockAicc {
                responses,
                requests: requests.clone(),
            }))),
            msg_center: None,
        };

        let agent = AIAgent::load(AIAgentConfig::new(&agent_root), deps)
            .await
            .expect("load ai agent");
        agent
            .push_inbox_message(json!({
                "from": "did:example:alice",
                "text": "请先给我一个简短结论",
                "session_id": "session-user-quick"
            }))
            .await;

        let report = agent
            .wait_wakeup(None)
            .await
            .expect("run fast sleep wakeup");
        assert!(matches!(report.status, WakeupStatus::Completed));
        assert_eq!(report.steps, 0);
        assert_eq!(report.final_behavior, "END");
        assert_eq!(report.reason["session_id"], "session-router-end");
        assert!(report.reason["memory_queries"]
            .as_array()
            .map(|v| v.iter().any(|q| q == "router done"))
            .unwrap_or(false));
        assert_eq!(report.reason["resolve_router_sleep"]["enabled"], true);

        let requests_guard = requests.lock().expect("requests lock");
        assert_eq!(requests_guard.len(), 1);
    }

    #[tokio::test]
    async fn ai_agent_stops_next_llm_when_session_paused_by_tool_call() {
        let tmp = tempdir().expect("create tmpdir");
        let agent_root = tmp.path().join("jarvis");
        write_agent_fixture(&agent_root).await;

        fs::write(
            agent_root.join("behaviors/resolve_router.yaml"),
            r#"
process_rule: test_rule
limits:
  max_tool_rounds: 0
  max_tool_calls_per_round: 0
  deadline_ms: 60000
"#,
        )
        .await
        .expect("write resolve_router yaml");

        fs::write(
            agent_root.join("behaviors/on_wakeup.yaml"),
            r#"
process_rule: test_rule
tools:
  mode: allow_list
  names:
    - create_session
    - update_session
limits:
  max_tool_rounds: 2
  max_tool_calls_per_round: 4
  deadline_ms: 60000
"#,
        )
        .await
        .expect("write on_wakeup yaml");

        let requests = Arc::new(Mutex::new(Vec::<CompleteRequest>::new()));
        let responses = Arc::new(Mutex::new(VecDeque::from(vec![
            mocked_response(
                json!({
                    "session_id": "session-pause-1",
                    "new_session": null,
                    "next_behavior": "on_wakeup",
                    "memory_queries": [],
                    "reply": null
                }),
                10,
                6,
            ),
            mocked_response(
                json!({
                    "tool_calls": [
                        {
                            "name": TOOL_CREATE_SESSION,
                            "args": {
                                "session_id": "session-pause-1",
                                "owner_agent": "did:example:jarvis",
                                "title": "pause session",
                                "summary": "",
                                "status": "normal"
                            },
                            "call_id": "call-create-session"
                        },
                        {
                            "name": TOOL_UPDATE_SESSION,
                            "args": {
                                "session_id": "session-pause-1",
                                "status": "pause"
                            },
                            "call_id": "call-pause-session"
                        }
                    ]
                }),
                14,
                8,
            ),
        ])));

        let deps = AIAgentDeps {
            taskmgr: Arc::new(TaskManagerClient::new_in_process(Box::new(
                MockTaskMgrHandler {
                    counter: Mutex::new(0),
                    tasks: Arc::new(Mutex::new(HashMap::new())),
                },
            ))),
            aicc: Arc::new(AiccClient::new_in_process(Box::new(MockAicc {
                responses,
                requests: requests.clone(),
            }))),
            msg_center: None,
        };

        let agent = AIAgent::load(AIAgentConfig::new(&agent_root), deps)
            .await
            .expect("load ai agent");
        agent
            .push_inbox_message(json!({
                "from": "did:example:alice",
                "text": "请暂停本会话",
                "session_id": "session-pause-1"
            }))
            .await;

        let report = agent.wait_wakeup(None).await.expect("run paused wakeup");
        assert!(matches!(report.status, WakeupStatus::Error));
        assert_eq!(report.steps, 0);
        assert!(report
            .last_error
            .as_deref()
            .unwrap_or_default()
            .contains("user canceled"));

        let requests_guard = requests.lock().expect("requests lock");
        assert_eq!(requests_guard.len(), 2);
    }
}
