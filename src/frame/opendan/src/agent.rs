use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use buckyos_api::{AiccClient, MsgCenterClient, TaskManagerClient};
use log::{debug, error, info, warn};
use serde::Serialize;
use serde_json::{json, Value as Json};
use tokio::fs;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinSet;

use crate::agent_enviroment::AgentEnvironment;
use crate::agent_memory::{AgentMemory, AgentMemoryConfig, TOOL_LOAD_MEMORY};
use crate::agent_tool::{ToolCall, ToolCallContext, ToolError, ToolManager, ToolSpec};
use crate::ai_runtime::{AiRuntime, AiRuntimeConfig};
use crate::behavior::{
    BehaviorConfig, BehaviorConfigError, EnvKV, Event, LLMBehavior, LLMBehaviorDeps, LLMStatus,
    Observation, ObservationSource, PolicyEngine, ProcessInput, TokenUsage, Tokenizer, TraceCtx,
    WorklogSink,
};
use crate::workspace::TOOL_EXEC_BASH;

const AGENT_DOC_CANDIDATES: [&str; 2] = ["agent.json.doc", "Agent.json.doc"];
const DEFAULT_ROLE_MD: &str = "role.md";
const DEFAULT_SELF_MD: &str = "self.md";
const DEFAULT_BEHAVIORS_DIR: &str = "behaviors";
const DEFAULT_ENVIRONMENT_DIR: &str = "environment";
const DEFAULT_WORKLOG_FILE: &str = "worklog/agent-loop.jsonl";
const DEFAULT_SLEEP_REASON: &str = "no_new_input";

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
}

#[derive(Clone, Debug)]
pub struct AIAgentConfig {
    pub agent_root: PathBuf,
    pub behaviors_dir_name: String,
    pub environment_dir_name: String,
    pub role_file_name: String,
    pub self_file_name: String,
    pub worklog_file_rel_path: PathBuf,
    pub max_steps_per_wakeup: u32,
    pub max_behavior_hops: u32,
    pub max_walltime_ms: u64,
    pub hp_max: u32,
    pub hp_floor: u32,
    pub hp_per_token: u32,
    pub hp_per_action: u32,
    pub default_sleep_ms: u64,
    pub max_sleep_ms: u64,
    pub memory_token_limit: u32,
}

impl AIAgentConfig {
    pub fn new(agent_root: impl Into<PathBuf>) -> Self {
        Self {
            agent_root: agent_root.into(),
            behaviors_dir_name: DEFAULT_BEHAVIORS_DIR.to_string(),
            environment_dir_name: DEFAULT_ENVIRONMENT_DIR.to_string(),
            role_file_name: DEFAULT_ROLE_MD.to_string(),
            self_file_name: DEFAULT_SELF_MD.to_string(),
            worklog_file_rel_path: PathBuf::from(DEFAULT_WORKLOG_FILE),
            max_steps_per_wakeup: 8,
            max_behavior_hops: 3,
            max_walltime_ms: 120_000,
            hp_max: 10_000,
            hp_floor: 1,
            hp_per_token: 1,
            hp_per_action: 10,
            default_sleep_ms: 2_000,
            max_sleep_ms: 120_000,
            memory_token_limit: 1_500,
        }
    }
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
    state: Mutex<AIAgentState>,
}

impl AIAgent {
    pub async fn load(mut cfg: AIAgentConfig, deps: AIAgentDeps) -> Result<Self, AIAgentError> {
        info!("ai_agent.load start: root={}", cfg.agent_root.display());
        normalize_config(&mut cfg)?;
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
        let environment = AgentEnvironment::new(environment_root).await?;
        let memory =
            AgentMemory::new(AgentMemoryConfig::new(&agent_root), deps.msg_center.clone()).await?;

        let tool_mgr = Arc::new(ToolManager::new());
        environment.register_workshop_tools(tool_mgr.as_ref())?;
        memory.register_tools(tool_mgr.as_ref())?;
        let runtime = AiRuntime::new(AiRuntimeConfig::new(&cfg.agent_root)).await?;
        runtime.register_agent(&did, &cfg.agent_root).await?;
        runtime.register_tools(tool_mgr.as_ref()).await?;

        let behavior_cfg_cache = Arc::new(RwLock::new(HashMap::<String, BehaviorConfig>::new()));
        preload_behavior_configs(&behaviors_dir, behavior_cfg_cache.clone()).await?;
        ensure_on_wakeup_behavior(behavior_cfg_cache.clone()).await;

        let policy = Arc::new(AgentPolicy::new(
            tool_mgr.clone(),
            behavior_cfg_cache.clone(),
        ));
        let worklog_path = environment
            .workspace_root()
            .join(&cfg.worklog_file_rel_path);
        let worklog = Arc::new(JsonlFileWorklogSink::new(worklog_path).await?);
        let tokenizer = Arc::new(WhitespaceTokenizer {});

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
            state: Mutex::new(AIAgentState::new(&cfg)),
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
    }

    pub async fn push_inbox_message(&self, msg: Json) {
        let mut guard = self.state.lock().await;
        guard.inbox_msgs.push_back(msg);
        debug!(
            "ai_agent.push_inbox_message: did={} inbox_len={}",
            self.did,
            guard.inbox_msgs.len()
        );
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

    pub async fn start(&self, max_wakeups: Option<u32>) -> Result<(), AIAgentError> {
        info!(
            "ai_agent.start: did={} max_wakeups={:?}",
            self.did, max_wakeups
        );
        let mut rounds = 0_u32;
        loop {
            if let Some(limit) = max_wakeups {
                if rounds >= limit {
                    info!(
                        "ai_agent.start stop: did={} reached max_wakeups={}",
                        self.did, limit
                    );
                    break;
                }
            }

            let report = self.on_wakeup(None).await?;
            rounds = rounds.saturating_add(1);
            match report.status {
                WakeupStatus::Error => {
                    error!(
                        "ai_agent.wakeup report: did={} wakeup_id={} status=error behavior={} steps={} hops={} hp={}=>{} tokens={} sleep_ms={} err={:?}",
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
                        "ai_agent.wakeup report: did={} wakeup_id={} status=safe_stop behavior={} steps={} hops={} hp={}=>{} tokens={} sleep_ms={} err={:?}",
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
                        "ai_agent.wakeup report: did={} wakeup_id={} status={} behavior={} steps={} hops={} hp={}=>{} tokens={} sleep_ms={}",
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

    pub async fn on_wakeup(&self, reason: Option<Json>) -> Result<WakeupReport, AIAgentError> {
        let wakeup_started = Instant::now();
        let now = now_ms();
        info!(
            "ai_agent.on_wakeup start: did={} explicit_reason={}",
            self.did,
            reason.is_some()
        );

        let (wakeup_id, hp_before, trace_id, input_payload) = match self
            .prepare_wakeup_context(reason, now)
            .await
        {
            PreparedWakeup::Disabled {
                wakeup_id,
                trace_id,
                hp,
                sleep_ms,
                reason,
            } => {
                warn!(
                        "ai_agent.on_wakeup skip: did={} wakeup_id={} status=disabled hp={} sleep_ms={}",
                        self.did, wakeup_id, hp, sleep_ms
                    );
                return Ok(WakeupReport {
                    wakeup_id,
                    trace_id,
                    status: WakeupStatus::Disabled,
                    final_behavior: "on_wakeup".to_string(),
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
                });
            }
            PreparedWakeup::SkippedNoInput {
                wakeup_id,
                trace_id,
                hp,
                sleep_ms,
                reason,
            } => {
                debug!(
                        "ai_agent.on_wakeup skip: did={} wakeup_id={} status=no_input hp={} sleep_ms={}",
                        self.did, wakeup_id, hp, sleep_ms
                    );
                return Ok(WakeupReport {
                    wakeup_id,
                    trace_id,
                    status: WakeupStatus::SkippedNoInput,
                    final_behavior: "on_wakeup".to_string(),
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
                });
            }
            PreparedWakeup::Ready {
                wakeup_id,
                hp_before,
                trace_id,
                input_payload,
            } => (wakeup_id, hp_before, trace_id, input_payload),
        };
        let (inbox_count, event_count) = wakeup_input_counts(&input_payload);
        info!(
            "ai_agent.on_wakeup ready: did={} wakeup_id={} hp_before={} inbox_count={} event_count={}",
            self.did, wakeup_id, hp_before, inbox_count, event_count
        );

        let mut status = WakeupStatus::Completed;
        let mut last_error = None::<String>;
        let mut token_usage = TokenUsage::default();
        let mut steps: u32 = 0;
        let mut behavior_hops: u32 = 0;
        let mut current_behavior = "on_wakeup".to_string();
        let mut pending_observations: Vec<Observation> = vec![];

        loop {
            debug!(
                "ai_agent.loop step: did={} wakeup_id={} step={} behavior={} pending_observations={}",
                self.did,
                wakeup_id,
                steps,
                current_behavior,
                pending_observations.len()
            );
            if steps >= self.cfg.max_steps_per_wakeup {
                status = WakeupStatus::SafeStop;
                last_error = Some(format!(
                    "max_steps_per_wakeup reached: {}",
                    self.cfg.max_steps_per_wakeup
                ));
                warn!(
                    "ai_agent.loop safe_stop: did={} wakeup_id={} reason=max_steps limit={}",
                    self.did, wakeup_id, self.cfg.max_steps_per_wakeup
                );
                break;
            }
            if wakeup_started.elapsed().as_millis() as u64 >= self.cfg.max_walltime_ms {
                status = WakeupStatus::SafeStop;
                last_error = Some(format!(
                    "max_walltime_ms reached: {}",
                    self.cfg.max_walltime_ms
                ));
                warn!(
                    "ai_agent.loop safe_stop: did={} wakeup_id={} reason=max_walltime limit_ms={}",
                    self.did, wakeup_id, self.cfg.max_walltime_ms
                );
                break;
            }

            if self.current_hp().await <= self.cfg.hp_floor {
                status = WakeupStatus::SafeStop;
                last_error = Some(format!("hp <= hp_floor ({})", self.cfg.hp_floor));
                warn!(
                    "ai_agent.loop safe_stop: did={} wakeup_id={} reason=hp_floor hp_floor={}",
                    self.did, wakeup_id, self.cfg.hp_floor
                );
                break;
            }

            let cfg = self.ensure_behavior_config(&current_behavior).await?;
            let trace = TraceCtx {
                trace_id: trace_id.clone(),
                agent_did: self.did.clone(),
                behavior: current_behavior.clone(),
                step_idx: steps,
                wakeup_id: wakeup_id.clone(),
            };

            let behavior =
                LLMBehavior::new(cfg.to_llm_behavior_config(), self.build_behavior_deps());
            let memory_pack = self.load_memory_pack(&trace).await;
            let env_context = self.build_env_context(now).await;
            let input = ProcessInput {
                trace: trace.clone(),
                role_md: self.role_md.clone(),
                self_md: self.self_md.clone(),
                behavior_prompt: cfg.process_rule.clone(),
                env_context,
                inbox: input_payload.clone(),
                memory: memory_pack,
                last_observations: pending_observations.clone(),
                limits: cfg.limits.clone(),
            };

            let llm_result = behavior.run_step(input).await;
            token_usage = token_usage.add(llm_result.token_usage.clone());
            self.consume_hp(
                llm_result.token_usage.total,
                llm_result.actions.len() as u32,
            )
            .await;
            debug!(
                "ai_agent.loop llm_done: did={} wakeup_id={} step={} behavior={} tokens={} actions={} is_sleep={} next_behavior={:?}",
                self.did,
                wakeup_id,
                steps,
                current_behavior,
                llm_result.token_usage.total,
                llm_result.actions.len(),
                llm_result.is_sleep,
                llm_result.next_behavior
            );

            if let LLMStatus::Error(err) = llm_result.status {
                status = WakeupStatus::Error;
                last_error = Some(err.message);
                error!(
                    "ai_agent.loop llm_error: did={} wakeup_id={} step={} behavior={} err={:?}",
                    self.did, wakeup_id, steps, current_behavior, last_error
                );
                break;
            }

            pending_observations = self.execute_actions(&trace, &llm_result.actions).await;

            let mut should_break = llm_result.is_sleep;
            if let Some(next_behavior) = llm_result.next_behavior.as_ref() {
                if next_behavior != &current_behavior {
                    behavior_hops = behavior_hops.saturating_add(1);
                    if behavior_hops > self.cfg.max_behavior_hops {
                        status = WakeupStatus::SafeStop;
                        last_error = Some(format!(
                            "max_behavior_hops reached: {}",
                            self.cfg.max_behavior_hops
                        ));
                        warn!(
                            "ai_agent.loop safe_stop: did={} wakeup_id={} reason=max_behavior_hops limit={}",
                            self.did, wakeup_id, self.cfg.max_behavior_hops
                        );
                        break;
                    }
                    info!(
                        "ai_agent.loop behavior_switch: did={} wakeup_id={} from={} to={} hops={}",
                        self.did, wakeup_id, current_behavior, next_behavior, behavior_hops
                    );
                    current_behavior = next_behavior.to_string();
                    should_break = false;
                }
            }

            steps = steps.saturating_add(1);
            if should_break {
                debug!(
                    "ai_agent.loop stop: did={} wakeup_id={} reason=llm_sleep",
                    self.did, wakeup_id
                );
                break;
            }
            if llm_result.actions.is_empty() && llm_result.next_behavior.is_none() {
                debug!(
                    "ai_agent.loop stop: did={} wakeup_id={} reason=no_actions_no_next_behavior",
                    self.did, wakeup_id
                );
                break;
            }
        }

        let hp_after = self.current_hp().await;
        if hp_after == 0 {
            self.disable().await;
            warn!(
                "ai_agent.on_wakeup disable: did={} wakeup_id={} reason=hp_exhausted",
                self.did, wakeup_id
            );
        }
        let sleep_ms = self.update_sleep_after_wakeup(&status).await;
        if matches!(status, WakeupStatus::Error) {
            error!(
                "ai_agent.on_wakeup finish: did={} wakeup_id={} status=error behavior={} steps={} hops={} hp={}=>{} tokens={} sleep_ms={} err={:?}",
                self.did,
                wakeup_id,
                current_behavior,
                steps,
                behavior_hops,
                hp_before,
                hp_after,
                token_usage.total,
                sleep_ms,
                last_error
            );
        } else if matches!(status, WakeupStatus::SafeStop) {
            warn!(
                "ai_agent.on_wakeup finish: did={} wakeup_id={} status=safe_stop behavior={} steps={} hops={} hp={}=>{} tokens={} sleep_ms={} err={:?}",
                self.did,
                wakeup_id,
                current_behavior,
                steps,
                behavior_hops,
                hp_before,
                hp_after,
                token_usage.total,
                sleep_ms,
                last_error
            );
        } else {
            info!(
                "ai_agent.on_wakeup finish: did={} wakeup_id={} status={} behavior={} steps={} hops={} hp={}=>{} tokens={} sleep_ms={}",
                self.did,
                wakeup_id,
                wakeup_status_name(&status),
                current_behavior,
                steps,
                behavior_hops,
                hp_before,
                hp_after,
                token_usage.total,
                sleep_ms
            );
        }

        Ok(WakeupReport {
            wakeup_id,
            trace_id,
            status,
            final_behavior: current_behavior,
            steps,
            behavior_hops,
            hp_before,
            hp_after,
            token_prompt: token_usage.prompt,
            token_completion: token_usage.completion,
            token_total: token_usage.total,
            sleep_ms,
            reason: input_payload,
            last_error,
        })
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

    async fn prepare_wakeup_context(&self, reason: Option<Json>, now: u64) -> PreparedWakeup {
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

        let input_payload = if let Some(reason) = reason {
            reason
        } else {
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
                &ToolCallContext {
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
    },
}

struct WhitespaceTokenizer;

impl Tokenizer for WhitespaceTokenizer {
    fn count_tokens(&self, text: &str) -> u32 {
        text.split_whitespace().count() as u32
    }
}

#[derive(Clone)]
struct AgentPolicy {
    tool_mgr: Arc<ToolManager>,
    behavior_cfg_cache: Arc<RwLock<HashMap<String, BehaviorConfig>>>,
}

impl AgentPolicy {
    fn new(
        tool_mgr: Arc<ToolManager>,
        behavior_cfg_cache: Arc<RwLock<HashMap<String, BehaviorConfig>>>,
    ) -> Self {
        Self {
            tool_mgr,
            behavior_cfg_cache,
        }
    }
}

#[async_trait]
impl PolicyEngine for AgentPolicy {
    async fn allowed_tools(&self, input: &ProcessInput) -> Result<Vec<ToolSpec>, String> {
        let all = self.tool_mgr.list_tool_specs();
        let cfg = {
            let guard = self.behavior_cfg_cache.read().await;
            guard.get(&input.trace.behavior).cloned()
        };
        if let Some(cfg) = cfg {
            let filtered = cfg.tools.filter_tool_specs(&all);
            debug!(
                "ai_agent.policy allowed_tools: behavior={} all={} filtered={}",
                input.trace.behavior,
                all.len(),
                filtered.len()
            );
            return Ok(filtered);
        }
        debug!(
            "ai_agent.policy allowed_tools: behavior={} all={} (no_behavior_cfg)",
            input.trace.behavior,
            all.len()
        );
        Ok(all)
    }

    async fn gate_tool_calls(
        &self,
        input: &ProcessInput,
        calls: &[ToolCall],
    ) -> Result<Vec<ToolCall>, String> {
        let allowed = self.allowed_tools(input).await?;
        let allowed_set = allowed
            .into_iter()
            .map(|item| item.name)
            .collect::<HashSet<_>>();

        let mut out = Vec::with_capacity(calls.len());
        for call in calls {
            if !allowed_set.contains(&call.name) {
                warn!(
                    "ai_agent.policy deny_tool_call: behavior={} tool={} calls={}",
                    input.trace.behavior,
                    call.name,
                    calls.len()
                );
                return Err(format!(
                    "tool `{}` is not allowed for behavior `{}`",
                    call.name, input.trace.behavior
                ));
            }
            out.push(call.clone());
        }
        Ok(out)
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
    async fn emit(&self, event: Event) {
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

fn worklog_event_to_json(event: Event) -> Json {
    match event {
        Event::LLMStarted { trace, model } => json!({
            "kind": "llm_started",
            "trace": trace_to_json(trace),
            "model": model
        }),
        Event::LLMFinished { trace, usage, ok } => json!({
            "kind": "llm_finished",
            "trace": trace_to_json(trace),
            "usage": {
                "prompt": usage.prompt,
                "completion": usage.completion,
                "total": usage.total
            },
            "ok": ok
        }),
        Event::ToolCallPlanned {
            trace,
            tool,
            call_id,
        } => json!({
            "kind": "tool_call_planned",
            "trace": trace_to_json(trace),
            "tool": tool,
            "call_id": call_id
        }),
        Event::ToolCallFinished {
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
        Event::ParseWarning { trace, msg } => json!({
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

    let call_id = format!(
        "{}-{}-{}",
        trace.wakeup_id,
        trace.step_idx,
        now_ms().saturating_sub(1)
    );
    let result = tool_mgr
        .call_tool(
            &ToolCallContext {
                trace_id: trace.trace_id.clone(),
                agent_did: trace.agent_did.clone(),
                behavior: trace.behavior.clone(),
                step_idx: trace.step_idx,
                wakeup_id: trace.wakeup_id.clone(),
            },
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

async fn ensure_on_wakeup_behavior(cache: Arc<RwLock<HashMap<String, BehaviorConfig>>>) {
    let mut guard = cache.write().await;
    if guard.contains_key("on_wakeup") {
        return;
    }
    warn!("ai_agent.behavior fallback: injecting default on_wakeup behavior");
    let mut fallback = BehaviorConfig::default();
    fallback.name = "on_wakeup".to_string();
    fallback.process_rule =
        "Check inbox/events, run safe steps, and return structured output.".to_string();
    guard.insert("on_wakeup".to_string(), fallback);
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

fn normalize_config(cfg: &mut AIAgentConfig) -> Result<(), AIAgentError> {
    if cfg.max_steps_per_wakeup == 0 {
        return Err(AIAgentError::InvalidConfig(
            "max_steps_per_wakeup must be > 0".to_string(),
        ));
    }
    if cfg.max_walltime_ms == 0 {
        return Err(AIAgentError::InvalidConfig(
            "max_walltime_ms must be > 0".to_string(),
        ));
    }
    if cfg.default_sleep_ms == 0 || cfg.max_sleep_ms == 0 || cfg.default_sleep_ms > cfg.max_sleep_ms
    {
        return Err(AIAgentError::InvalidConfig(
            "sleep config invalid: require 0 < default_sleep_ms <= max_sleep_ms".to_string(),
        ));
    }
    if cfg.hp_max == 0 {
        return Err(AIAgentError::InvalidConfig(
            "hp_max must be > 0".to_string(),
        ));
    }
    if cfg.memory_token_limit == 0 {
        cfg.memory_token_limit = 1_500;
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, VecDeque};
    use std::ops::Range;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use buckyos_api::{
        AiResponseSummary, AiUsage, AiccHandler, CancelResponse, CompleteRequest, CompleteResponse,
        CompleteStatus, CreateTaskOptions, Task, TaskFilter, TaskManagerHandler, TaskPermissions,
        TaskStatus,
    };
    use kRPC::{RPCContext, RPCErrors, Result as KRPCResult};
    use rusqlite::Connection;
    use serde_json::json;
    use tempfile::tempdir;
    use tokio::fs;

    use super::*;
    use crate::workspace::{TOOL_EXEC_BASH, TOOL_TODO_MANAGE};

    struct MockTaskMgrHandler {
        counter: Mutex<u64>,
        tasks: Arc<Mutex<HashMap<i64, Task>>>,
    }

    #[async_trait]
    impl TaskManagerHandler for MockTaskMgrHandler {
        async fn handle_create_task(
            &self,
            name: &str,
            task_type: &str,
            data: Option<Json>,
            opts: CreateTaskOptions,
            user_id: &str,
            app_id: &str,
            _ctx: RPCContext,
        ) -> KRPCResult<Task> {
            let mut guard = self.counter.lock().expect("counter lock");
            *guard += 1;
            let now = now_ms();
            let task = Task {
                id: *guard as i64,
                user_id: user_id.to_string(),
                app_id: app_id.to_string(),
                parent_id: opts.parent_id,
                root_id: None,
                name: name.to_string(),
                task_type: task_type.to_string(),
                status: TaskStatus::Pending,
                progress: 0.0,
                message: None,
                data: data.unwrap_or_else(|| json!({})),
                permissions: opts.permissions.unwrap_or(TaskPermissions::default()),
                created_at: now,
                updated_at: now,
            };
            self.tasks
                .lock()
                .expect("tasks lock")
                .insert(task.id, task.clone());
            Ok(task)
        }

        async fn handle_get_task(&self, id: i64, _ctx: RPCContext) -> KRPCResult<Task> {
            self.tasks
                .lock()
                .expect("tasks lock")
                .get(&id)
                .cloned()
                .ok_or_else(|| RPCErrors::ReasonError(format!("mock task {} not found", id)))
        }

        async fn handle_list_tasks(
            &self,
            _filter: TaskFilter,
            _source_user_id: Option<&str>,
            _source_app_id: Option<&str>,
            _ctx: RPCContext,
        ) -> KRPCResult<Vec<Task>> {
            Ok(vec![])
        }

        async fn handle_list_tasks_by_time_range(
            &self,
            _app_id: Option<&str>,
            _task_type: Option<&str>,
            _source_user_id: Option<&str>,
            _source_app_id: Option<&str>,
            _time_range: Range<u64>,
            _ctx: RPCContext,
        ) -> KRPCResult<Vec<Task>> {
            Ok(vec![])
        }

        async fn handle_get_subtasks(
            &self,
            _parent_id: i64,
            _ctx: RPCContext,
        ) -> KRPCResult<Vec<Task>> {
            Ok(vec![])
        }

        async fn handle_update_task(
            &self,
            id: i64,
            status: Option<TaskStatus>,
            progress: Option<f32>,
            message: Option<String>,
            data: Option<Json>,
            _ctx: RPCContext,
        ) -> KRPCResult<()> {
            if let Some(task) = self.tasks.lock().expect("tasks lock").get_mut(&id) {
                if let Some(s) = status {
                    task.status = s;
                }
                if let Some(p) = progress {
                    task.progress = p;
                }
                task.message = message;
                if let Some(patch) = data {
                    task.data = patch;
                }
            }
            Ok(())
        }

        async fn handle_update_task_progress(
            &self,
            id: i64,
            completed_items: u64,
            total_items: u64,
            _ctx: RPCContext,
        ) -> KRPCResult<()> {
            if let Some(task) = self.tasks.lock().expect("tasks lock").get_mut(&id) {
                if total_items > 0 {
                    task.progress = (completed_items as f32 / total_items as f32).clamp(0.0, 1.0);
                }
            }
            Ok(())
        }

        async fn handle_update_task_status(
            &self,
            id: i64,
            status: TaskStatus,
            _ctx: RPCContext,
        ) -> KRPCResult<()> {
            if let Some(task) = self.tasks.lock().expect("tasks lock").get_mut(&id) {
                task.status = status;
            }
            Ok(())
        }

        async fn handle_update_task_error(
            &self,
            id: i64,
            error_message: &str,
            _ctx: RPCContext,
        ) -> KRPCResult<()> {
            if let Some(task) = self.tasks.lock().expect("tasks lock").get_mut(&id) {
                task.status = TaskStatus::Failed;
                task.message = Some(error_message.to_string());
            }
            Ok(())
        }

        async fn handle_update_task_data(
            &self,
            id: i64,
            data: Json,
            _ctx: RPCContext,
        ) -> KRPCResult<()> {
            if let Some(task) = self.tasks.lock().expect("tasks lock").get_mut(&id) {
                task.data = data;
            }
            Ok(())
        }

        async fn handle_cancel_task(
            &self,
            _id: i64,
            _recursive: bool,
            _ctx: RPCContext,
        ) -> KRPCResult<()> {
            Ok(())
        }

        async fn handle_delete_task(&self, _id: i64, _ctx: RPCContext) -> KRPCResult<()> {
            Ok(())
        }
    }

    struct MockAicc {
        responses: Arc<Mutex<VecDeque<CompleteResponse>>>,
        requests: Arc<Mutex<Vec<CompleteRequest>>>,
    }

    #[async_trait]
    impl AiccHandler for MockAicc {
        async fn handle_complete(
            &self,
            request: CompleteRequest,
            _ctx: RPCContext,
        ) -> KRPCResult<CompleteResponse> {
            self.requests.lock().expect("requests lock").push(request);
            self.responses
                .lock()
                .expect("responses lock")
                .pop_front()
                .ok_or_else(|| RPCErrors::ReasonError("no response queued".to_string()))
        }

        async fn handle_cancel(
            &self,
            task_id: &str,
            _ctx: RPCContext,
        ) -> KRPCResult<CancelResponse> {
            Ok(CancelResponse::new(task_id.to_string(), true))
        }
    }

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
process_rule: Handle incoming requests, update todo, then write an artifact.
tools:
  mode: allow_list
  names:
    - workshop.todo_manage
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
        let report = agent.on_wakeup(None).await.expect("run on_wakeup");
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
}
