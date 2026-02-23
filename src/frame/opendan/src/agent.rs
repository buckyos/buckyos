use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use buckyos_api::{AiccClient, BoxKind, MsgCenterClient, MsgObject, MsgRecordWithObject, MsgState, TaskManagerClient};
use log::{debug, info, warn};
use name_lib::DID;
use ndn_lib::ObjId;
use serde_json::{json, Value as Json};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, RwLock};
use tokio::time::sleep;

use crate::agent_config::AIAgentConfig;
use crate::agent_enviroment::AgentEnvironment;
use crate::agent_memory::{AgentMemory, AgentMemoryConfig};
use crate::agent_session::{AgentSessionMgr, GetSessionTool, SessionExecInput, SessionState};
use crate::agent_tool::{AgentPolicy, ToolCall, ToolManager};
use crate::behavior::{
    ActionExecutionMode, ActionKind, ActionSpec, AgentWorkEvent, BehaviorConfig, BehaviorExecInput,
    EnvKV, ExecutorReply, LLMBehavior, LLMBehaviorDeps, LLMOutput, LLMTrackingInfo, Tokenizer, TraceCtx,
    WorklogSink,
};

const AGENT_DOC_CANDIDATES: [&str; 2] = ["agent.json.doc", "Agent.json.doc"];
const LEGACY_ENV_DIR_NAME: &str = "enviroment";
const DEFAULT_SESSION_ID: &str = "default";
const MAX_MSG_PULL_PER_TICK: usize = 16;
const MSG_ROUTED_REASON: &str = "routed_by_opendan_runtime";

#[derive(Debug)]
struct PulledMsg {
    session_id: Option<String>,
    payload: Json,
    record_id: Option<String>,
}

#[derive(Debug)]
struct PulledEvent {
    session_id: Option<String>,
    payload: Json,
}

#[derive(Clone)]
pub struct AIAgentDeps {
    pub taskmgr: Arc<TaskManagerClient>,
    pub aicc: Arc<AiccClient>,
    pub msg_center: Option<Arc<MsgCenterClient>>,
}

pub struct AIAgent {
    cfg: AIAgentConfig,
    did: String,
    role_md: String,
    self_md: String,
    behaviors_dir: PathBuf,
    workspace_root: PathBuf,
    tools: Arc<ToolManager>,
    memory: AgentMemory,
    _environment: AgentEnvironment,
    session_mgr: Arc<AgentSessionMgr>,
    behavior_cfg_cache: Arc<RwLock<HashMap<String, BehaviorConfig>>>,
    policy: Arc<AgentPolicy>,
    worklog: Arc<JsonlWorklogSink>,
    tokenizer: Arc<SimpleTokenizer>,
    deps: AIAgentDeps,
    default_behavior: String,
    wakeup_seq: AtomicU64,
}

impl AIAgent {
    pub async fn load(mut cfg: AIAgentConfig, deps: AIAgentDeps) -> Result<Self> {
        cfg.normalize()
            .map_err(|err| anyhow!("invalid agent config: {err}"))?;

        let agent_root = to_abs_path(&cfg.agent_root)?;
        fs::create_dir_all(&agent_root).await.map_err(|err| {
            anyhow!(
                "create agent root failed: path={} err={}",
                agent_root.display(),
                err
            )
        })?;

        let did = load_agent_did(&agent_root).await?;
        let role_path = agent_root.join(&cfg.role_file_name);
        let self_path = agent_root.join(&cfg.self_file_name);
        let role_md = read_text_if_exists(&role_path)
            .await?
            .unwrap_or_else(|| "# Role\nYou are an OpenDAN agent.".to_string());
        let self_md = read_text_if_exists(&self_path)
            .await?
            .unwrap_or_else(|| "# Self\n- Keep tasks traceable\n".to_string());

        let behaviors_dir = agent_root.join(&cfg.behaviors_dir_name);
        fs::create_dir_all(&behaviors_dir).await.map_err(|err| {
            anyhow!(
                "create behaviors dir failed: path={} err={}",
                behaviors_dir.display(),
                err
            )
        })?;

        let workspace_root = resolve_workspace_root(&agent_root, &cfg.environment_dir_name).await?;
        let session_root = workspace_root.join("session");

        let tools = Arc::new(ToolManager::new());

        let environment = AgentEnvironment::new(workspace_root.clone())
            .await
            .map_err(|err| anyhow!("init agent environment failed: {err}"))?;
        environment
            .register_workshop_tools(&tools)
            .map_err(|err| anyhow!("register workshop tools failed: {err}"))?;

        let memory = AgentMemory::new(AgentMemoryConfig::new(agent_root.clone()))
            .await
            .map_err(|err| anyhow!("init agent memory failed: {err}"))?;
        memory
            .register_tools(&tools)
            .map_err(|err| anyhow!("register memory tools failed: {err}"))?;

        let default_behavior = resolve_default_behavior_name(&behaviors_dir)
            .await
            .unwrap_or_else(|| "on_wakeup".to_string());

        let session_store = Arc::new(
            AgentSessionMgr::new(did.clone(), session_root, Some(default_behavior.clone()))
                .await
                .map_err(|err| anyhow!("init session store failed: {err}"))?,
        );
        let _ = session_store
            .ensure_default_session()
            .await
            .map_err(|err| anyhow!("ensure default session failed: {err}"))?;

        tools
            .register_tool(GetSessionTool::new(session_store.clone()))
            .map_err(|err| anyhow!("register session tool failed: {err}"))?;

        let behavior_cfg_cache = Arc::new(RwLock::new(HashMap::new()));
        let policy = Arc::new(AgentPolicy::new(tools.clone(), behavior_cfg_cache.clone()));

        let worklog_path = workspace_root.join(&cfg.worklog_file_rel_path);
        let worklog = Arc::new(
            JsonlWorklogSink::new(worklog_path)
                .await
                .map_err(|err| anyhow!("init worklog sink failed: {err}"))?,
        );

        let agent = Self {
            cfg,
            did,
            role_md,
            self_md,
            behaviors_dir,
            workspace_root,
            tools,
            memory,
            _environment: environment,
            session_mgr: session_store,
            behavior_cfg_cache,
            policy,
            worklog,
            tokenizer: Arc::new(SimpleTokenizer),
            deps,
            default_behavior,
            wakeup_seq: AtomicU64::new(0),
        };

        let _ = agent.load_behavior_config(&agent.default_behavior).await;
        Ok(agent)
    }

    pub fn did(&self) -> &str {
        &self.did
    }

    pub async fn inject_msg(
        &self,
        session_id: Option<&str>,
        payload: Json,
    ) -> Result<String> {
        let sid = session_id.unwrap_or(DEFAULT_SESSION_ID);
        self.session_mgr
            .append_msg(sid, payload)
            .await
            .map_err(|err| anyhow!("append session msg failed: {err}"))
    }

    pub async fn inject_event(
        &self,
        session_id: Option<&str>,
        payload: Json,
    ) -> Result<String> {
        let sid = session_id.unwrap_or(DEFAULT_SESSION_ID);
        self.session_mgr
            .append_event(sid, payload)
            .await
            .map_err(|err| anyhow!("append session event failed: {err}"))
    }

    pub async fn run_agent_loop(&self, stop_after_ticks: Option<u32>) -> Result<()> {
        let mut tick = 0_u32;
        let mut sleep_ms = self.cfg.default_sleep_ms;

        loop {
            if let Some(max_tick) = stop_after_ticks {
                if tick >= max_tick {
                    break;
                }
            }
            tick = tick.saturating_add(1);

            self.session_mgr
                .refresh_all_statuses_from_disk()
                .await
                .map_err(|err| anyhow!("refresh session status failed: {err}"))?;

            let (pulled_msgs, pulled_events) = self.pull_msgs_and_events().await?;
            self.dispatch_pulled_inputs(pulled_msgs, pulled_events).await?;

            self.session_mgr
                .schedule_wait_timeouts(now_ms())
                .await
                .map_err(|err| anyhow!("schedule session wait-timeout failed: {err}"))?;

            let ready = self.session_mgr.list_ready_sessions().await;
            if ready.is_empty() {
                sleep(Duration::from_millis(sleep_ms)).await;
                sleep_ms = (sleep_ms.saturating_mul(2)).min(self.cfg.max_sleep_ms);
                continue;
            }

            sleep_ms = self.cfg.default_sleep_ms;
            self.run_ready_sessions(ready).await;
        }

        Ok(())
    }

    async fn pull_msgs_and_events(&self) -> Result<(Vec<PulledMsg>, Vec<PulledEvent>)> {
        let pulled_msgs = self.pull_msg_packs().await;
        let pulled_events = self.pull_event_packs().await;
        Ok((pulled_msgs, pulled_events))
    }

    async fn run_ready_sessions(
        &self,
        ready: Vec<Arc<Mutex<crate::agent_session::AgentSession>>>,
    ) {
        // Placeholder: docs expect worker-thread dispatch; current implementation keeps
        // deterministic serial execution until max_session_parallel is finalized.
        for session in ready {
            if let Err(err) = self.run_session_loop(session.clone()).await {
                let session_id = {
                    let mut guard = session.lock().await;
                    guard.append_worklog(json!({
                        "type": "agent_error",
                        "error": err.to_string(),
                        "ts_ms": now_ms(),
                    }));
                    guard.update_state(SessionState::Wait);
                    guard.session_id.clone()
                };
                let _ = self.session_mgr.save_session(&session_id).await;
                warn!("agent.session_loop failed: did={} err={}", self.did, err);
            }
        }
    }

    async fn pull_msg_packs(&self) -> Vec<PulledMsg> {
        let Some(msg_center) = self.deps.msg_center.as_ref() else {
            return vec![];
        };
        let Some(owner_did) = self.parse_owner_did_for_msg_center() else {
            return vec![];
        };

        let mut out = Vec::<PulledMsg>::new();
        for _ in 0..MAX_MSG_PULL_PER_TICK {
            match msg_center
                .get_next(
                    owner_did.clone(),
                    BoxKind::Inbox,
                    Some(vec![MsgState::Unread]),
                    Some(true),
                )
                .await
            {
                Ok(Some(record)) => out.push(Self::msg_record_to_pulled_msg(record)),
                Ok(None) => break,
                Err(err) => {
                    warn!("agent.msg_pull_failed: did={} err={}", self.did, err);
                    break;
                }
            }
        }
        out
    }

    async fn pull_event_packs(&self) -> Vec<PulledEvent> {
        // Placeholder: event source contract (MsgQueue/KEvent) is not finalized in doc.
        // Keep the hook so Agent Loop semantics still match pull->route->dispatch.
        vec![]
    }

    async fn dispatch_pulled_inputs(
        &self,
        pulled_msgs: Vec<PulledMsg>,
        pulled_events: Vec<PulledEvent>,
    ) -> Result<()> {
        for pulled in pulled_msgs {
            let session_id = self
                .resolve_session_for_msg(pulled.session_id.as_deref(), &pulled.payload)
                .await?;
            self.session_mgr
                .append_msg(session_id.as_str(), pulled.payload)
                .await
                .map_err(|err| anyhow!("dispatch msg to session `{session_id}` failed: {err}"))?;
            if let Some(record_id) = pulled.record_id {
                self.set_msg_readed(record_id).await;
            }
        }

        for pulled in pulled_events {
            let session_id = self
                .resolve_session_for_event(pulled.session_id.as_deref(), &pulled.payload)
                .await?;
            self.session_mgr
                .append_event(session_id.as_str(), pulled.payload)
                .await
                .map_err(|err| anyhow!("dispatch event to session `{session_id}` failed: {err}"))?;
        }
        Ok(())
    }

    async fn resolve_session_for_msg(&self, hinted_session_id: Option<&str>, payload: &Json) -> Result<String> {
        if let Some(session_id) = normalize_session_id(hinted_session_id) {
            return Ok(session_id);
        }
        if let Some(session_id) = extract_session_id_hint(payload) {
            return Ok(session_id);
        }
        self.resolve_router_by_llm("msg", payload).await
    }

    async fn resolve_session_for_event(
        &self,
        hinted_session_id: Option<&str>,
        payload: &Json,
    ) -> Result<String> {
        if let Some(session_id) = normalize_session_id(hinted_session_id) {
            return Ok(session_id);
        }
        if let Some(session_id) = extract_session_id_hint(payload) {
            return Ok(session_id);
        }
        self.resolve_router_by_llm("event", payload).await
    }

    async fn resolve_router_by_llm(&self, source_kind: &str, payload: &Json) -> Result<String> {

        let preview = compact_text_for_log(payload.to_string().as_str(), 160);
        debug!(
            "agent.resolve_router_by_llm: did={} source={} fallback_session={} payload={}",
            self.did, source_kind, DEFAULT_SESSION_ID, preview
        );

        let mut router_cfg: Option<(String, BehaviorConfig)> = None;
        for candidate in ["resolve_router", "router", "rouer"] {
            if let Ok(cfg) = self.load_behavior_config(candidate).await {
                router_cfg = Some((candidate.to_string(), cfg));
                break;
            }
        }

        let Some((router_behavior_name, router_cfg)) = router_cfg else {
            warn!(
                "agent.resolve_router_missing: did={} source={} fallback_session={}",
                self.did, source_kind, DEFAULT_SESSION_ID
            );
            return Ok(DEFAULT_SESSION_ID.to_string());
        };

        let router_id = format!(
            "router-{}-{}",
            now_ms(),
            self.wakeup_seq.fetch_add(1, Ordering::Relaxed)
        );
        let max_router_steps = router_cfg
            .step_limit
            .max(1)
            .min(self.cfg.max_steps_per_wakeup.max(1));
        let mut route_payload = None::<Json>;
        let mut route_next_behavior = None::<String>;

        for step_idx in 0..max_router_steps {
            let trace = TraceCtx {
                trace_id: format!("route-trace-{}-{}", now_ms(), step_idx),
                agent_did: self.did.clone(),
                behavior: router_behavior_name.clone(),
                step_idx,
                wakeup_id: router_id.clone(),
            };

            let memory_token_limit = if router_cfg.memory.total_limit > 0 {
                router_cfg.memory.total_limit
            } else {
                self.cfg.memory_token_limit
            };

            let memory = self
                .memory
                .load_memory(
                    Some(memory_token_limit),
                    vec![trace.behavior.clone()],
                    None,
                )
                .await
                .unwrap_or_else(|err| {
                    warn!(
                        "agent.resolve_router_memory_load_failed: did={} behavior={} err={}",
                        self.did, trace.behavior, err
                    );
                    json!({})
                });

            let mut env_context = vec![
                EnvKV {
                    key: "router.source".to_string(),
                    value: source_kind.to_string(),
                },
                EnvKV {
                    key: "step.index".to_string(),
                    value: step_idx.to_string(),
                },
                EnvKV {
                    key: "step.remaining".to_string(),
                    value: max_router_steps.saturating_sub(step_idx).to_string(),
                },
            ];

            if !router_cfg.policy.trim().is_empty() {
                env_context.push(EnvKV {
                    key: "policy.text".to_string(),
                    value: router_cfg.policy.clone(),
                });
            }
            if !router_cfg.input.trim().is_empty() {
                env_context.push(EnvKV {
                    key: "input.template".to_string(),
                    value: router_cfg.input.clone(),
                });
            }
            if !router_cfg.memory.is_empty() {
                env_context.push(EnvKV {
                    key: "memory.policy".to_string(),
                    value: router_cfg.memory.to_json_value().to_string(),
                });
            }

            let inbox = if source_kind.eq_ignore_ascii_case("event") {
                json!({
                    "new_msg": Json::Null,
                    "new_event": payload.clone(),
                })
            } else {
                json!({
                    "new_msg": payload.clone(),
                    "new_event": Json::Null,
                })
            };

            let input = BehaviorExecInput {
                trace: trace.clone(),
                role_md: self.role_md.clone(),
                self_md: self.self_md.clone(),
                session_id: None,
                behavior_prompt: router_cfg.process_rule.clone(),
                env_context,
                inbox,
                memory,
                last_observations: vec![],
                limits: router_cfg.limits.clone(),
            };

            let (llm_result, tracking, _action_results) =
                match self.run_behavior_step(&trace, &router_cfg, input).await {
                    Ok(result) => result,
                    Err(err) => {
                        warn!(
                            "agent.resolve_router_step_failed: did={} source={} behavior={} step={} err={}",
                            self.did, source_kind, router_behavior_name, step_idx, err
                        );
                        break;
                    }
                };

            if let LLMOutput::Json(value) = tracking.raw_output {
                route_payload = Some(value);
            }

            route_next_behavior = normalize_session_id(llm_result.next_behavior.as_deref());
            if route_next_behavior.is_some() {
                break;
            }
        }

        let route = parse_router_resolution(route_payload.as_ref(), route_next_behavior.as_deref());
        if let Some(reply) = route.reply.as_deref() {
            self.send_reply_via_msg_center(payload, "user", "text", reply, None)
                .await;
            info!(
                "agent.resolve_router_reply: did={} source={} reply={}",
                self.did,
                source_kind,
                compact_text_for_log(reply, 256)
            );
        }

        let mut resolved_session_id = normalize_routed_session_id(route.session_id.as_deref());
        if resolved_session_id.is_none() {
            if let Some(title) = route.new_session_title.as_deref() {
                resolved_session_id = Some(build_generated_session_id(title));
            }
        }
        let mut session_id = resolved_session_id.unwrap_or_else(|| DEFAULT_SESSION_ID.to_string());

        let session = match self
            .session_mgr
            .ensure_session(session_id.as_str(), route.new_session_title.clone())
            .await
        {
            Ok(session) => session,
            Err(err) => {
                warn!(
                    "agent.resolve_router_ensure_session_failed: did={} source={} session={} err={}",
                    self.did, source_kind, session_id, err
                );
                session_id = DEFAULT_SESSION_ID.to_string();
                self.session_mgr
                    .ensure_session(DEFAULT_SESSION_ID, None)
                    .await
                    .map_err(|ensure_err| {
                        anyhow!(
                            "ensure fallback session `{}` failed after router error: {}",
                            DEFAULT_SESSION_ID,
                            ensure_err
                        )
                    })?
            }
        };

        let next_behavior = route.next_behavior.clone();
        let route_reply = route.reply.clone();
        let memory_queries = route.memory_queries.clone();
        {
            let mut guard = session.lock().await;

            if let Some(summary) = route
                .new_session_summary
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                if guard.summary.trim().is_empty() {
                    guard.summary = summary.to_string();
                }
            }

            if let Some(next_behavior) = next_behavior
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                if !next_behavior.eq_ignore_ascii_case("END")
                    && !next_behavior.eq_ignore_ascii_case("WAIT")
                {
                    guard.current_behavior = Some(next_behavior.to_string());
                    guard.step_index = 0;
                }
            }

            guard.append_worklog(json!({
                "type": "router_resolved",
                "source": source_kind,
                "session_id": session_id,
                "next_behavior": next_behavior,
                "new_session_title": route.new_session_title,
                "memory_queries": memory_queries,
                "reply": route_reply,
                "ts_ms": now_ms(),
            }));
            self.session_mgr
                .save_session_locked(&guard)
                .await
                .map_err(|err| anyhow!("save routed session `{}` failed: {}", guard.session_id, err))?;
        }

        debug!(
            "agent.resolve_router_done: did={} source={} session={} next_behavior={}",
            self.did,
            source_kind,
            session_id,
            next_behavior.unwrap_or_else(|| "(none)".to_string())
        );

        Ok(session_id)
    }

    async fn set_msg_readed(&self, record_id: String) {
        let Some(msg_center) = self.deps.msg_center.as_ref() else {
            return;
        };
        if let Err(err) = msg_center
            .update_record_state(
                record_id.clone(),
                MsgState::Readed,
                Some(MSG_ROUTED_REASON.to_string()),
            )
            .await
        {
            warn!(
                "agent.msg_mark_read_failed: did={} record_id={} err={}",
                self.did, record_id, err
            );
        }
    }

    fn parse_owner_did_for_msg_center(&self) -> Option<DID> {
        match DID::from_str(self.did.as_str()) {
            Ok(did) => Some(did),
            Err(err) => {
                warn!(
                    "agent.msg_center_disabled_by_did: did={} err={}",
                    self.did, err
                );
                None
            }
        }
    }

    fn msg_record_to_pulled_msg(record: MsgRecordWithObject) -> PulledMsg {
        let session_id = extract_session_id_hint(&record.msg.payload);
        let record_id = record.record.record_id.clone();
        PulledMsg {
            session_id,
            payload: json!({
                "source": "msg_center",
                "record": record.record,
                "msg": record.msg,
            }),
            record_id: Some(record_id),
        }
    }

    async fn run_session_loop(&self, session: Arc<Mutex<crate::agent_session::AgentSession>>) -> Result<()> {
        let wakeup_id = format!(
            "wakeup-{}-{}",
            now_ms(),
            self.wakeup_seq.fetch_add(1, Ordering::Relaxed)
        );

        let started_at = now_ms();
        let mut step_count = 0_u32;
        let mut behavior_hops = 0_u32;

        loop {
            if step_count >= self.cfg.max_steps_per_wakeup {
                self.pause_running_session_to_wait(&session).await?;
                break;
            }
            if now_ms().saturating_sub(started_at) >= self.cfg.max_walltime_ms {
                self.pause_running_session_to_wait(&session).await?;
                break;
            }
            if behavior_hops > self.cfg.max_behavior_hops {
                self.pause_running_session_to_wait(&session).await?;
                break;
            }

            let (session_id, behavior_name, step_index, state) = {
                let mut guard = session.lock().await;
                if guard.state == SessionState::Pause {
                    (guard.session_id.clone(), String::new(), guard.step_index, guard.state)
                } else {
                    if guard.current_behavior.is_none() {
                        guard.current_behavior = Some(self.default_behavior.clone());
                        guard.step_index = 0;
                    }
                    (
                        guard.session_id.clone(),
                        guard.current_behavior.clone().unwrap_or_else(|| self.default_behavior.clone()),
                        guard.step_index,
                        guard.state,
                    )
                }
            };

            if state == SessionState::Pause || state != SessionState::Running {
                break;
            }

            let behavior_cfg = match self.load_behavior_config(&behavior_name).await {
                Ok(cfg) => cfg,
                Err(err) => {
                    warn!(
                        "agent.behavior_missing: did={} session={} behavior={} err={}",
                        self.did, session_id, behavior_name, err
                    );
                    let session_id = {
                        let mut guard = session.lock().await;
                        guard.current_behavior = Some(self.default_behavior.clone());
                        guard.step_index = 0;
                        guard.update_state(SessionState::Wait);
                        guard.append_worklog(json!({
                            "type": "behavior_missing",
                            "behavior": behavior_name,
                            "error": err.to_string(),
                            "ts_ms": now_ms(),
                        }));
                        guard.session_id.clone()
                    };
                    self.session_mgr.save_session(&session_id).await?;
                    break;
                }
            };

            let exec_input = {
                let guard = session.lock().await;
                if guard.state != SessionState::Running {
                    None
                } else {
                    guard.generate_input(&behavior_cfg)
                }
            };

            let Some(exec_input) = exec_input else {
                let session_id = {
                    let mut guard = session.lock().await;
                    guard.update_state(SessionState::Wait);
                    guard.session_id.clone()
                };
                self.session_mgr.save_session(&session_id).await?;
                break;
            };

            let trace = TraceCtx {
                trace_id: format!("trace-{}-{}", now_ms(), step_count),
                agent_did: self.did.clone(),
                behavior: behavior_name.clone(),
                step_idx: step_index,
                wakeup_id: wakeup_id.clone(),
            };

            let behavior_input = self
                .build_behavior_exec_input(&trace, &behavior_cfg, &exec_input)
                .await?;

            let step_result = self
                .run_behavior_step(&trace, &behavior_cfg, behavior_input)
                .await;

            let transition = match step_result {
                Ok((llm_result, tracking, action_results)) => {
                    self.handle_replies(trace.clone(), &exec_input.payload, llm_result.reply.as_slice())
                        .await;
                    self.apply_memory_updates(&trace, llm_result.set_memory.as_slice())
                        .await;

                    let transition = {
                        let mut guard = session.lock().await;
                        guard.update_input_used(&exec_input);
                        guard.apply_session_delta(llm_result.session_delta.as_slice());

                        let step_summary = build_step_summary(
                            &trace,
                            &behavior_cfg,
                            &llm_result.next_behavior,
                            &tracking,
                            action_results.as_slice(),
                        );
                        guard.set_last_step_summary(step_summary.clone());
                        guard.append_worklog(step_summary);

                        apply_behavior_transition(
                            &mut guard,
                            self.default_behavior.as_str(),
                            behavior_cfg.step_limit,
                            llm_result.next_behavior.as_deref(),
                        )
                    };
                    let _ = self.session_mgr.save_session(&transition.session_id).await;
                    transition
                }
                Err(err) => {
                    let session_id = {
                        let mut guard = session.lock().await;
                        guard.append_worklog(json!({
                            "type": "step_error",
                            "behavior": behavior_name,
                            "step_index": step_index,
                            "error": err.to_string(),
                            "ts_ms": now_ms(),
                        }));
                        guard.update_state(SessionState::Wait);
                        guard.session_id.clone()
                    };
                    let _ = self.session_mgr.save_session(&session_id).await;
                    return Err(err);
                }
            };

            step_count = step_count.saturating_add(1);
            if transition.behavior_switched {
                behavior_hops = behavior_hops.saturating_add(1);
            }
            if !transition.keep_running {
                break;
            }
        }

        Ok(())
    }

    async fn pause_running_session_to_wait(
        &self,
        session: &Arc<Mutex<crate::agent_session::AgentSession>>,
    ) -> Result<()> {
        let session_id = {
            let mut guard = session.lock().await;
            if guard.state == SessionState::Running {
                guard.update_state(SessionState::Wait);
            }
            guard.session_id.clone()
        };
        self.session_mgr.save_session(&session_id).await?;
        Ok(())
    }

    async fn build_behavior_exec_input(
        &self,
        trace: &TraceCtx,
        behavior_cfg: &BehaviorConfig,
        session_input: &SessionExecInput,
    ) -> Result<BehaviorExecInput> {
        let memory_token_limit = if behavior_cfg.memory.total_limit > 0 {
            behavior_cfg.memory.total_limit
        } else {
            self.cfg.memory_token_limit
        };

        let memory = self
            .memory
            .load_memory(
                Some(memory_token_limit),
                vec![trace.behavior.clone()],
                None,
            )
            .await
            .unwrap_or_else(|err| {
                warn!(
                    "agent.memory_load_failed: did={} behavior={} err={}",
                    self.did, trace.behavior, err
                );
                json!({})
            });

        let mut env_context = vec![
            EnvKV {
                key: "loop.session_id".to_string(),
                value: session_input.session_id.clone(),
            },
            EnvKV {
                key: "step.index".to_string(),
                value: trace.step_idx.to_string(),
            },
            EnvKV {
                key: "step.remaining".to_string(),
                value: self
                    .cfg
                    .max_steps_per_wakeup
                    .saturating_sub(trace.step_idx)
                    .to_string(),
            },
        ];

        if !behavior_cfg.policy.trim().is_empty() {
            env_context.push(EnvKV {
                key: "policy.text".to_string(),
                value: behavior_cfg.policy.clone(),
            });
        }
        if !behavior_cfg.input.trim().is_empty() {
            env_context.push(EnvKV {
                key: "input.template".to_string(),
                value: behavior_cfg.input.clone(),
            });
        }
        if !behavior_cfg.memory.is_empty() {
            env_context.push(EnvKV {
                key: "memory.policy".to_string(),
                value: behavior_cfg.memory.to_json_value().to_string(),
            });
        }

        Ok(BehaviorExecInput {
            trace: trace.clone(),
            role_md: self.role_md.clone(),
            self_md: self.self_md.clone(),
            session_id: Some(session_input.session_id.clone()),
            behavior_prompt: behavior_cfg.process_rule.clone(),
            env_context,
            inbox: session_input.payload.clone(),
            memory,
            last_observations: vec![],
            limits: behavior_cfg.limits.clone(),
        })
    }

    async fn run_behavior_step(
        &self,
        trace: &TraceCtx,
        behavior_cfg: &BehaviorConfig,
        input: BehaviorExecInput,
    ) -> Result<(crate::behavior::BehaviorLLMResult, LLMTrackingInfo, Vec<Json>)> {
        let llm = LLMBehavior::new(
            behavior_cfg.to_llm_behavior_config(),
            LLMBehaviorDeps {
                taskmgr: self.deps.taskmgr.clone(),
                aicc: self.deps.aicc.clone(),
                tools: self.tools.clone(),
                policy: self.policy.clone(),
                worklog: self.worklog.clone(),
                tokenizer: self.tokenizer.clone(),
            },
        );

        let (llm_result, tracking) = llm
            .run_step(input)
            .await
            .map_err(|err| anyhow!("llm behavior step failed: {err}"))?;

        let action_results = self
            .execute_actions(trace, llm_result.actions.as_slice())
            .await;

        Ok((llm_result, tracking, action_results))
    }

    async fn execute_actions(&self, trace: &TraceCtx, actions: &[ActionSpec]) -> Vec<Json> {
        if actions.is_empty() {
            return vec![];
        }

        let mut out = Vec::<Json>::with_capacity(actions.len());
        for action in actions {
            if action.kind != ActionKind::Bash {
                out.push(json!({
                    "ok": false,
                    "kind": format!("{:?}", action.kind),
                    "title": action.title,
                    "error": "unsupported action kind"
                }));
                continue;
            }

            let args = json!({
                "command": action.command,
                "cwd": action.cwd,
                "timeout_ms": action.timeout_ms,
                "allow_network": action.allow_network,
            });
            let call = ToolCall {
                name: "exec_bash".to_string(),
                args,
                call_id: format!(
                    "action-{}-{}-{}",
                    trace.step_idx,
                    now_ms(),
                    self.wakeup_seq.fetch_add(1, Ordering::Relaxed)
                ),
            };

            let run_result = self.tools.call_tool(trace, call).await;
            let record = match run_result {
                Ok(result) => json!({
                    "ok": true,
                    "title": action.title,
                    "execution_mode": format!("{:?}", action.execution_mode),
                    "result": result
                }),
                Err(err) => json!({
                    "ok": false,
                    "title": action.title,
                    "execution_mode": format!("{:?}", action.execution_mode),
                    "error": err.to_string()
                }),
            };
            out.push(record);

            if action.execution_mode == ActionExecutionMode::Parallel {
                debug!(
                    "agent.action_parallel_hint ignored for now: did={} behavior={} title={}",
                    self.did, trace.behavior, action.title
                );
            }
        }
        out
    }

    async fn apply_memory_updates(&self, trace: &TraceCtx, set_memory: &[Json]) {
        for item in set_memory {
            let Some(obj) = item.as_object() else {
                continue;
            };
            let Some(key) = obj
                .get("key")
                .or_else(|| obj.get("name"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };

            let content = obj
                .get("content")
                .or_else(|| obj.get("json_content"))
                .cloned()
                .unwrap_or(Json::Null);
            let source = obj
                .get("source")
                .cloned()
                .unwrap_or_else(|| {
                    json!({
                        "trace_id": trace.trace_id,
                        "behavior": trace.behavior,
                        "step_idx": trace.step_idx,
                        "agent_did": trace.agent_did,
                    })
                });

            if let Err(err) = self.memory.set_memory(key, content, source).await {
                warn!(
                    "agent.set_memory failed: did={} key={} err={}",
                    self.did, key, err
                );
            }
        }
    }

    async fn handle_replies(&self, trace: TraceCtx, source_payload: &Json, replies: &[ExecutorReply]) {
        if replies.is_empty() {
            return;
        }
        for reply in replies {
            self.send_reply_via_msg_center(
                source_payload,
                reply.audience.as_str(),
                reply.format.as_str(),
                reply.content.as_str(),
                Some(&trace),
            )
            .await;
            info!(
                "agent.reply: did={} behavior={} audience={} format={} content={}",
                self.did,
                trace.behavior,
                reply.audience,
                reply.format,
                compact_text_for_log(reply.content.as_str(), 512)
            );
        }
    }

    async fn send_reply_via_msg_center(
        &self,
        source_payload: &Json,
        audience: &str,
        format: &str,
        content: &str,
        trace: Option<&TraceCtx>,
    ) {
        let content = content.trim();
        if content.is_empty() {
            return;
        }

        let Some(msg_center) = self.deps.msg_center.as_ref() else {
            return;
        };
        let Some(sender_did) = self.parse_owner_did_for_msg_center() else {
            return;
        };
        let Some(target_did) = extract_reply_target_did(source_payload) else {
            debug!(
                "agent.reply_skip_no_target: did={} audience={} format={} payload={}",
                self.did,
                audience,
                format,
                compact_text_for_log(source_payload.to_string().as_str(), 256)
            );
            return;
        };

        if target_did == sender_did {
            debug!(
                "agent.reply_skip_self_target: did={} target={:?} audience={}",
                self.did, target_did, audience
            );
            return;
        }

        let msg_id = match ObjId::new(
            format!(
                "chunk:{}-{}",
                now_ms(),
                self.wakeup_seq.fetch_add(1, Ordering::Relaxed)
            )
            .as_str(),
        ) {
            Ok(id) => id,
            Err(err) => {
                warn!(
                    "agent.reply_build_msg_id_failed: did={} target={:?} err={}",
                    self.did, target_did, err
                );
                return;
            }
        };

        let mut payload = json!({
            "kind": "text",
            "audience": audience,
            "format": format,
            "content": content,
            "session_id": extract_session_id_hint(source_payload),
            "source": "opendan",
        });
        if let Some(trace) = trace {
            payload["trace"] = json!({
                "trace_id": trace.trace_id,
                "wakeup_id": trace.wakeup_id,
                "behavior": trace.behavior,
                "step_idx": trace.step_idx,
            });
        }

        let mut outbound = MsgObject::new(
            msg_id,
            sender_did,
            None,
            vec![target_did.clone()],
            payload,
            now_ms(),
        );
        outbound.thread_key = extract_reply_thread_key(source_payload);

        match msg_center.post_send(outbound, None, None).await {
            Ok(result) => {
                if !result.ok {
                    warn!(
                        "agent.reply_post_send_rejected: did={} target={:?} reason={}",
                        self.did,
                        target_did,
                        result.reason.unwrap_or_else(|| "unknown".to_string())
                    );
                }
            }
            Err(err) => {
                warn!(
                    "agent.reply_post_send_failed: did={} target={:?} err={}",
                    self.did, target_did, err
                );
            }
        }
    }

    async fn load_behavior_config(&self, behavior_name: &str) -> Result<BehaviorConfig> {
        let behavior_name = behavior_name.trim();
        if behavior_name.is_empty() {
            return Err(anyhow!("behavior name cannot be empty"));
        }

        if let Some(cached) = self
            .behavior_cfg_cache
            .read()
            .await
            .get(behavior_name)
            .cloned()
        {
            return Ok(cached);
        }

        let loaded = BehaviorConfig::load_from_dir(&self.behaviors_dir, behavior_name)
            .await
            .map_err(|err| anyhow!("load behavior `{behavior_name}` failed: {err}"))?;

        self.behavior_cfg_cache
            .write()
            .await
            .insert(behavior_name.to_string(), loaded.clone());
        Ok(loaded)
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }
}

#[derive(Clone, Debug)]
struct StepTransition {
    session_id: String,
    keep_running: bool,
    behavior_switched: bool,
}

#[derive(Debug, Default)]
struct RouterResolution {
    session_id: Option<String>,
    new_session_title: Option<String>,
    new_session_summary: Option<String>,
    next_behavior: Option<String>,
    reply: Option<String>,
    memory_queries: Vec<String>,
}

fn apply_behavior_transition(
    session: &mut crate::agent_session::AgentSession,
    default_behavior: &str,
    step_limit: u32,
    next_behavior: Option<&str>,
) -> StepTransition {
    let mut behavior_switched = false;
    let keep_running;

    if let Some(next_behavior) = next_behavior
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if next_behavior.eq_ignore_ascii_case("WAIT") {
            if session.state != SessionState::WaitForMsg
                && session.state != SessionState::WaitForEvent
                && session.state != SessionState::Pause
                && session.state != SessionState::Sleep
            {
                session.update_state(SessionState::Wait);
            } else {
                // Respect wait-like states previously written in session_delta.
                session.update_state(session.state);
            }
            keep_running = false;
        } else if next_behavior.eq_ignore_ascii_case("END") {
            session.update_state(SessionState::Wait);
            keep_running = false;
        } else {
            behavior_switched = session.current_behavior.as_deref() != Some(next_behavior);
            session.current_behavior = Some(next_behavior.to_string());
            session.step_index = 0;
            session.update_state(SessionState::Running);
            keep_running = true;
        }
    } else {
        if session.state != SessionState::Running {
            keep_running = false;
        } else {
            session.step_index = session.step_index.saturating_add(1);
            if step_limit > 0 && session.step_index > step_limit {
                session.current_behavior = Some(default_behavior.to_string());
                session.step_index = 0;
                session.update_state(SessionState::Wait);
                keep_running = false;
            } else {
                session.update_state(SessionState::Running);
                keep_running = true;
            }
        }
    }

    StepTransition {
        session_id: session.session_id.clone(),
        keep_running,
        behavior_switched,
    }
}

fn build_step_summary(
    trace: &TraceCtx,
    behavior_cfg: &BehaviorConfig,
    next_behavior: &Option<String>,
    tracking: &LLMTrackingInfo,
    action_results: &[Json],
) -> Json {
    json!({
        "type": "step_summary",
        "summary": format!(
            "behavior={} step={} next={} tokens={}",
            trace.behavior,
            trace.step_idx,
            next_behavior.clone().unwrap_or_else(|| "(none)".to_string()),
            tracking.token_usage.total,
        ),
        "ts_ms": now_ms(),
        "trace": {
            "trace_id": trace.trace_id,
            "wakeup_id": trace.wakeup_id,
            "behavior": trace.behavior,
            "step_idx": trace.step_idx,
        },
        "behavior_cfg": {
            "name": behavior_cfg.name,
            "step_limit": behavior_cfg.step_limit,
        },
        "llm": {
            "next_behavior": next_behavior,
            "token_usage": {
                "prompt": tracking.token_usage.prompt,
                "completion": tracking.token_usage.completion,
                "total": tracking.token_usage.total,
            },
            "model": tracking.track.model,
            "provider": tracking.track.provider,
            "latency_ms": tracking.track.latency_ms,
            "llm_task_ids": tracking.track.llm_task_ids,
            "errors": tracking.track.errors,
        },
        "tool_trace": tracking
            .tool_trace
            .iter()
            .map(|item| {
                json!({
                    "tool_name": item.tool_name,
                    "call_id": item.call_id,
                    "ok": item.ok,
                    "duration_ms": item.duration_ms,
                    "error": item.error,
                })
            })
            .collect::<Vec<_>>(),
        "actions": action_results,
    })
}

async fn resolve_workspace_root(agent_root: &Path, env_name: &str) -> Result<PathBuf> {
    let normal = agent_root.join(env_name);
    let legacy = agent_root.join(LEGACY_ENV_DIR_NAME);

    let root = if is_existing_dir(&normal).await {
        normal
    } else if is_existing_dir(&legacy).await {
        legacy
    } else {
        normal
    };

    fs::create_dir_all(&root).await.map_err(|err| {
        anyhow!(
            "create workspace root failed: path={} err={}",
            root.display(),
            err
        )
    })?;
    Ok(root)
}

async fn resolve_default_behavior_name(behaviors_dir: &Path) -> Option<String> {
    for candidate in ["on_wakeup", "router", "rouer", "resolve_router"] {
        if behavior_exists(behaviors_dir, candidate).await {
            return Some(candidate.to_string());
        }
    }

    let mut read_dir = fs::read_dir(behaviors_dir).await.ok()?;
    while let Some(entry) = read_dir.next_entry().await.ok()? {
        let path = entry.path();
        let ext = path
            .extension()
            .and_then(|v| v.to_str())
            .map(|v| v.to_ascii_lowercase());
        if matches!(ext.as_deref(), Some("yaml") | Some("yml") | Some("json")) {
            if let Some(stem) = path.file_stem().and_then(|v| v.to_str()) {
                let trimmed = stem.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
    }
    None
}

async fn behavior_exists(behaviors_dir: &Path, behavior_name: &str) -> bool {
    for ext in ["yaml", "yml", "json"] {
        let path = behaviors_dir.join(format!("{behavior_name}.{ext}"));
        if fs::metadata(&path).await.map(|meta| meta.is_file()).unwrap_or(false) {
            return true;
        }
    }
    false
}

async fn load_agent_did(agent_root: &Path) -> Result<String> {
    for name in AGENT_DOC_CANDIDATES {
        let path = agent_root.join(name);
        let Some(raw) = read_text_if_exists(&path).await? else {
            continue;
        };
        let parsed: Json = serde_json::from_str(&raw).with_context(|| {
            format!(
                "parse agent document failed: path={}",
                path.display()
            )
        })?;
        if let Some(did) = parsed
            .get("id")
            .or_else(|| parsed.get("did"))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Ok(did.to_string());
        }
    }

    let dir_name = agent_root
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("agent");
    Ok(format!("did:opendan:{dir_name}"))
}

async fn read_text_if_exists(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path).await {
        Ok(text) => Ok(Some(text)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(anyhow!(
            "read file failed: path={} err={}",
            path.display(),
            err
        )),
    }
}

fn to_abs_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    Ok(std::env::current_dir()
        .context("read current_dir failed")?
        .join(path))
}

async fn is_existing_dir(path: &Path) -> bool {
    fs::metadata(path).await.map(|meta| meta.is_dir()).unwrap_or(false)
}

fn normalize_session_id(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
}

fn normalize_routed_session_id(value: Option<&str>) -> Option<String> {
    let session_id = normalize_session_id(value)?;
    if session_id.len() > 180
        || session_id == "."
        || session_id == ".."
        || session_id.contains('/')
        || session_id.contains('\\')
        || session_id.chars().any(|ch| ch.is_control())
    {
        return None;
    }
    Some(session_id)
}

fn build_generated_session_id(title: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = false;
    for ch in title.chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            slug.push(c);
            last_dash = false;
        } else if (c.is_ascii_whitespace() || c == '-' || c == '_') && !last_dash && !slug.is_empty() {
            slug.push('-');
            last_dash = true;
        }
        if slug.len() >= 64 {
            break;
        }
    }

    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        slug.push_str("session");
    }

    format!("{}-{}", slug, now_ms())
}

fn parse_router_resolution(route_payload: Option<&Json>, next_behavior_hint: Option<&str>) -> RouterResolution {
    let mut out = RouterResolution {
        next_behavior: normalize_session_id(next_behavior_hint),
        ..Default::default()
    };
    let Some(route_payload) = route_payload else {
        return out;
    };

    out.session_id = normalize_session_id(
        route_payload
            .pointer("/session_id")
            .and_then(|value| value.as_str()),
    );

    if let Some(raw_new_session) = route_payload.pointer("/new_session") {
        if let Some(arr) = raw_new_session.as_array() {
            out.new_session_title = normalize_session_id(arr.first().and_then(|value| value.as_str()));
            out.new_session_summary =
                normalize_session_id(arr.get(1).and_then(|value| value.as_str()));
        } else if let Some(obj) = raw_new_session.as_object() {
            out.new_session_title = normalize_session_id(
                obj.get("title")
                    .and_then(|value| value.as_str())
                    .or_else(|| obj.get("name").and_then(|value| value.as_str())),
            );
            out.new_session_summary = normalize_session_id(
                obj.get("description")
                    .and_then(|value| value.as_str())
                    .or_else(|| obj.get("summary").and_then(|value| value.as_str())),
            );
        }
    }

    if out.next_behavior.is_none() {
        out.next_behavior = normalize_session_id(
            route_payload
                .pointer("/next_behavior")
                .and_then(|value| value.as_str()),
        );
    }

    out.reply = normalize_session_id(route_payload.pointer("/reply").and_then(|value| value.as_str()));
    out.memory_queries = route_payload
        .pointer("/memory_queries")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| normalize_session_id(item.as_str()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    out
}

fn extract_reply_target_did(payload: &Json) -> Option<DID> {
    if let Some(did) = extract_reply_target_did_from_msg(payload) {
        return Some(did);
    }

    let items = payload.pointer("/new_msg").and_then(|value| value.as_array())?;
    for item in items.iter().rev() {
        let inner = item.get("payload").unwrap_or(item);
        if let Some(did) = extract_reply_target_did_from_msg(inner) {
            return Some(did);
        }
    }
    None
}

fn extract_reply_target_did_from_msg(payload: &Json) -> Option<DID> {
    for pointer in ["/msg/from", "/from", "/msg/source", "/source"] {
        let Some(raw) = payload
            .pointer(pointer)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        if let Ok(did) = DID::from_str(raw) {
            return Some(did);
        }
    }
    None
}

fn extract_reply_thread_key(payload: &Json) -> Option<String> {
    if let Some(thread_key) = extract_reply_thread_key_from_msg(payload) {
        return Some(thread_key);
    }

    let Some(items) = payload.pointer("/new_msg").and_then(|value| value.as_array()) else {
        return None;
    };
    for item in items.iter().rev() {
        let inner = item.get("payload").unwrap_or(item);
        if let Some(thread_key) = extract_reply_thread_key_from_msg(inner) {
            return Some(thread_key);
        }
    }
    None
}

fn extract_reply_thread_key_from_msg(payload: &Json) -> Option<String> {
    for pointer in ["/msg/thread_key", "/thread_key", "/record/thread_key"] {
        if let Some(thread_key) = payload
            .pointer(pointer)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(thread_key.to_string());
        }
    }
    None
}

fn extract_session_id_hint(payload: &Json) -> Option<String> {
    for pointer in [
        "/session_id",
        "/payload/session_id",
        "/msg/session_id",
        "/msg/payload/session_id",
        "/meta/session_id",
        "/msg/meta/session_id",
    ] {
        if let Some(session_id) = payload
            .pointer(pointer)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(session_id.to_string());
        }
    }
    None
}

fn compact_text_for_log(value: &str, max_chars: usize) -> String {
    let escaped = value.replace('\n', "\\n").replace('\r', "\\r");
    if escaped.chars().count() <= max_chars {
        escaped
    } else {
        format!(
            "{}...[TRUNCATED]",
            escaped.chars().take(max_chars).collect::<String>()
        )
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

struct SimpleTokenizer;

impl Tokenizer for SimpleTokenizer {
    fn count_tokens(&self, text: &str) -> u32 {
        text.split_whitespace().count() as u32
    }
}

#[derive(Debug)]
struct JsonlWorklogSink {
    path: PathBuf,
    write_lock: Mutex<()>,
}

impl JsonlWorklogSink {
    async fn new(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await.map_err(|err| {
                anyhow!(
                    "create worklog dir failed: path={} err={}",
                    parent.display(),
                    err
                )
            })?;
        }
        Ok(Self {
            path,
            write_lock: Mutex::new(()),
        })
    }

    async fn append_json_line(&self, line: Json) {
        let _guard = self.write_lock.lock().await;
        let text = match serde_json::to_string(&line) {
            Ok(text) => text,
            Err(err) => {
                warn!("serialize worklog event failed: {}", err);
                return;
            }
        };

        let mut file = match tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await
        {
            Ok(file) => file,
            Err(err) => {
                warn!(
                    "open worklog sink failed: path={} err={}",
                    self.path.display(),
                    err
                );
                return;
            }
        };

        if let Err(err) = file.write_all(format!("{text}\n").as_bytes()).await {
            warn!(
                "write worklog sink failed: path={} err={}",
                self.path.display(),
                err
            );
        }
    }
}

#[async_trait]
impl WorklogSink for JsonlWorklogSink {
    async fn emit(&self, event: AgentWorkEvent) {
        let payload = match event {
            AgentWorkEvent::LLMStarted { trace, model } => json!({
                "kind": "llm_started",
                "ts_ms": now_ms(),
                "trace": trace,
                "model": model,
            }),
            AgentWorkEvent::LLMFinished { trace, usage, ok } => json!({
                "kind": "llm_finished",
                "ts_ms": now_ms(),
                "trace": trace,
                "ok": ok,
                "usage": {
                    "prompt": usage.prompt,
                    "completion": usage.completion,
                    "total": usage.total,
                }
            }),
            AgentWorkEvent::ToolCallPlanned {
                trace,
                tool,
                call_id,
            } => json!({
                "kind": "tool_call_planned",
                "ts_ms": now_ms(),
                "trace": trace,
                "tool": tool,
                "call_id": call_id,
            }),
            AgentWorkEvent::ToolCallFinished {
                trace,
                tool,
                call_id,
                ok,
                duration_ms,
            } => json!({
                "kind": "tool_call_finished",
                "ts_ms": now_ms(),
                "trace": trace,
                "tool": tool,
                "call_id": call_id,
                "ok": ok,
                "duration_ms": duration_ms,
            }),
            AgentWorkEvent::ParseWarning { trace, msg } => json!({
                "kind": "parse_warning",
                "ts_ms": now_ms(),
                "trace": trace,
                "message": msg,
            }),
        };
        self.append_json_line(payload).await;
    }
}
