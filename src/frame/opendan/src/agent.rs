use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use buckyos_api::{
    msg_queue::{Message, MsgQueueClient, QueueConfig, SubPosition},
    AiccClient, BoxKind, Event, EventReader, KEventClient, KEventError, MsgCenterClient,
    MsgRecordWithObject, MsgState, SendContext, TaskManagerClient,
};
use log::{debug, info, warn};
use name_lib::DID;
use ndn_lib::{MsgContent, MsgContentFormat, MsgObjKind, MsgObject};
use serde::Deserialize;
use serde_json::{json, Value as Json};
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, RwLock};
use tokio::time::sleep;
use tokio::{fs, task};
use uuid::Uuid;

use crate::agent_config::AIAgentConfig;
use crate::agent_enviroment::AgentEnvironment;
use crate::agent_memory::{AgentMemory, AgentMemoryConfig};
use crate::agent_session::{
    AgentSessionMgr, GetSessionTool, SessionExecInput, SessionInputItem, SessionState,
};
use crate::agent_tool::{AgentPolicy, ToolCall, ToolManager};
use crate::behavior::{
    ActionExecutionMode, ActionKind, ActionSpec, AgentWorkEvent, BehaviorConfig, BehaviorExecInput,
    EnvKV, ExecutorReply, LLMBehavior, LLMBehaviorDeps, LLMOutput, LLMTrackingInfo, Tokenizer,
    TraceCtx, WorklogSink,
};
use crate::worklog::TOOL_WORKLOG_MANAGE;
use crate::workspace::TOOL_TODO_MANAGE;

const AGENT_DOC_CANDIDATES: [&str; 2] = ["agent.json.doc", "Agent.json.doc"];
const LEGACY_ENV_DIR_NAME: &str = "enviroment";
const DEFAULT_SESSION_ID: &str = "default";
const MAX_MSG_PULL_PER_TICK: usize = 16;
const MAX_EVENT_PULL_PER_TICK: usize = 16;
const MSG_ROUTED_REASON: &str = "routed_by_opendan_runtime";
const MSG_CENTER_EVENT_BOX_NAMES: [&str; 3] = ["in", "group_in", "request"];
const SESSION_QUEUE_APP_ID: &str = "opendan";
const SESSION_QUEUE_RETENTION_SECONDS: u64 = 7 * 24 * 60 * 60;
const SESSION_QUEUE_MAX_MESSAGES: u64 = 4096;
const SESSION_QUEUE_FETCH_BATCH: usize = 64;

#[derive(Debug)]
struct PulledMsg {
    session_id: Option<String>,
    record: MsgRecordWithObject,
}

#[derive(Debug)]
struct PulledEvent {
    session_id: Option<String>,
    payload: Event,
}

#[derive(Clone, Copy, Debug)]
enum InputQueueKind {
    Msg,
    Event,
}

#[derive(Clone, Debug)]
struct SessionQueueBinding {
    msg_queue_name: String,
    event_queue_name: String,
    msg_queue_urn: String,
    event_queue_urn: String,
    msg_sub_id: String,
    event_sub_id: String,
}

#[derive(Clone)]
pub struct AIAgentDeps {
    pub taskmgr: Arc<TaskManagerClient>,
    pub aicc: Arc<AiccClient>,
    pub msg_center: Option<Arc<MsgCenterClient>>,
    pub msg_queue: Option<Arc<MsgQueueClient>>,
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
    kevent_client: KEventClient,
    msg_center_event_reader: Mutex<Option<Arc<EventReader>>>,
    session_queue_bindings: Arc<RwLock<HashMap<String, SessionQueueBinding>>>,
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
        let kevent_source_node = Self::sanitize_kevent_token(did.as_str());

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
            kevent_client: KEventClient::new_full(kevent_source_node, None),
            msg_center_event_reader: Mutex::new(None),
            session_queue_bindings: Arc::new(RwLock::new(HashMap::new())),
            default_behavior,
            wakeup_seq: AtomicU64::new(0),
        };

        let _ = agent.load_behavior_config(&agent.default_behavior).await;
        Ok(agent)
    }

    pub fn did(&self) -> &str {
        &self.did
    }

    pub async fn run_agent_loop(self: Arc<Self>, stop_after_ticks: Option<u32>) -> Result<()> {
        self.session_mgr
            .refresh_all_statuses_from_disk()
            .await
            .map_err(|err| anyhow!("refresh session status failed: {err}"))?;

        let mut worker_handles = Vec::with_capacity(self.cfg.session_worker_threads);
        for worker_idx in 0..self.cfg.session_worker_threads {
            let worker_agent = self.clone();
            let handle = task::spawn(async move {
                if let Err(err) = worker_agent.run_session_worker_loop(stop_after_ticks).await {
                    warn!(
                        "agent.session_worker_loop exited with error: did={} worker={} err={}",
                        worker_agent.did, worker_idx, err
                    );
                }
            });
            worker_handles.push(handle);
        }

        let result = self.run_agent_dispatch_loop(stop_after_ticks).await;
        for worker_handle in &worker_handles {
            worker_handle.abort();
        }
        for worker_handle in worker_handles {
            let _ = worker_handle.await;
        }
        result
    }

    async fn run_agent_dispatch_loop(&self, stop_after_ticks: Option<u32>) -> Result<()> {
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

            let (pulled_msgs, pulled_events, waited_on_events) =
                self.pull_msgs_and_events(sleep_ms).await?;
            let has_inputs = !pulled_msgs.is_empty() || !pulled_events.is_empty();
            self.dispatch_pulled_inputs(pulled_msgs, pulled_events)
                .await?;

            self.session_mgr
                .schedule_wait_timeouts(now_ms())
                .await
                .map_err(|err| anyhow!("schedule session wait-timeout failed: {err}"))?;

            if has_inputs {
                sleep_ms = self.cfg.default_sleep_ms;
            } else {
                if !waited_on_events {
                    sleep(Duration::from_millis(sleep_ms)).await;
                }
                sleep_ms = (sleep_ms.saturating_mul(2)).min(self.cfg.max_sleep_ms);
            }
        }

        Ok(())
    }

    async fn run_session_worker_loop(&self, stop_after_ticks: Option<u32>) -> Result<()> {
        let mut tick = 0_u32;
        let mut sleep_ms = self.cfg.default_sleep_ms;

        loop {
            if let Some(max_tick) = stop_after_ticks {
                if tick >= max_tick {
                    break;
                }
            }
            tick = tick.saturating_add(1);

            let Some(session) = self.session_mgr.get_next_ready_session().await else {
                sleep(Duration::from_millis(sleep_ms)).await;
                sleep_ms = (sleep_ms.saturating_mul(2)).min(self.cfg.max_sleep_ms);
                continue;
            };

            sleep_ms = self.cfg.default_sleep_ms;
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

        Ok(())
    }

    async fn pull_msgs_and_events(
        &self,
        wait_timeout_ms: u64,
    ) -> Result<(Vec<PulledMsg>, Vec<PulledEvent>, bool)> {
        let Some(event_reader) = self.ensure_msg_center_event_reader().await else {
            let pulled_msgs = self.pull_msg_packs().await;
            return Ok((pulled_msgs, vec![], false));
        };
        let mut pulled_events = Vec::<PulledEvent>::new();
        let mut msg_pull_boxes = Vec::<BoxKind>::new();
        match event_reader.pull_event(Some(wait_timeout_ms)).await {
            Ok(Some(event)) => {
                Self::collect_event_pull_targets(event, &mut msg_pull_boxes, &mut pulled_events);
                for _ in 0..MAX_EVENT_PULL_PER_TICK.saturating_sub(1) {
                    match event_reader.pull_event(Some(0)).await {
                        Ok(Some(event)) => {
                            Self::collect_event_pull_targets(
                                event,
                                &mut msg_pull_boxes,
                                &mut pulled_events,
                            );
                        }
                        Ok(None) => break,
                        Err(err) => {
                            warn!("agent.event_pull_failed: did={} err={:?}", self.did, err);
                            if matches!(err, KEventError::ReaderClosed(_)) {
                                self.reset_msg_center_event_reader().await;
                            }
                            break;
                        }
                    }
                }
            }
            Ok(None) => {
                // KEvent is a poll accelerator. Timeout still falls back to queue pull.
                Self::append_all_msg_center_boxes(&mut msg_pull_boxes);
            }
            Err(err) => {
                warn!("agent.event_pull_failed: did={} err={:?}", self.did, err);
                if matches!(err, KEventError::ReaderClosed(_)) {
                    self.reset_msg_center_event_reader().await;
                }
                // Keep queue polling as fallback when event pull fails.
                Self::append_all_msg_center_boxes(&mut msg_pull_boxes);
            }
        }

        let pulled_msgs = if msg_pull_boxes.is_empty() {
            vec![]
        } else {
            self.pull_msg_packs_by_boxes(msg_pull_boxes.as_slice())
                .await
        };
        Ok((pulled_msgs, pulled_events, true))
    }

    async fn pull_msg_packs(&self) -> Vec<PulledMsg> {
        let mut box_kinds = Vec::new();
        Self::append_all_msg_center_boxes(&mut box_kinds);
        self.pull_msg_packs_by_boxes(box_kinds.as_slice()).await
    }

    async fn pull_msg_packs_by_boxes(&self, box_kinds: &[BoxKind]) -> Vec<PulledMsg> {
        let Some(msg_center) = self.deps.msg_center.as_ref() else {
            return vec![];
        };
        let Some(owner_did) = self.parse_owner_did_for_msg_center() else {
            return vec![];
        };

        let mut out = Vec::<PulledMsg>::new();
        for box_kind in box_kinds {
            for _ in 0..MAX_MSG_PULL_PER_TICK {
                match msg_center
                    .get_next(
                        owner_did.clone(),
                        box_kind.clone(),
                        Some(vec![MsgState::Unread]),
                        Some(true),
                        Some(true),
                    )
                    .await
                {
                    Ok(Some(record)) => out.push(Self::msg_record_to_pulled_msg(record)),
                    Ok(None) => break,
                    Err(err) => {
                        warn!(
                            "agent.msg_pull_failed: did={} box_kind={:?} err={}",
                            self.did, box_kind, err
                        );
                        break;
                    }
                }
            }
        }
        out
    }

    async fn dispatch_pulled_inputs(
        &self,
        pulled_msgs: Vec<PulledMsg>,
        pulled_events: Vec<PulledEvent>,
    ) -> Result<()> {
        for pulled in pulled_msgs {
            let record_id = pulled.record.record.record_id.clone();
            let previous_session_id = pulled.record.record.session_id.clone();
            let session_id = self
                .resolve_session_for_msg(pulled.session_id.as_deref(), &pulled.record)
                .await?;
            let payload = Self::msg_record_to_runtime_payload(&pulled.record);
            let item = self.build_session_input_item(payload, "msg");
            if self
                .enqueue_session_input(session_id.as_str(), &item, InputQueueKind::Msg)
                .await?
            {
                self.session_mgr
                    .mark_msg_arrived(session_id.as_str(), &item)
                    .await
                    .map_err(|err| {
                        anyhow!("mark msg arrival for session `{session_id}` failed: {err}")
                    })?;
            } else {
                self.session_mgr
                    .append_msg(session_id.as_str(), item.payload.clone())
                    .await
                    .map_err(|err| anyhow!("dispatch msg to session `{session_id}` failed: {err}"))?;
            }
            if previous_session_id.as_deref() != Some(session_id.as_str()) {
                self.update_msg_record_session(record_id.clone(), session_id.clone())
                    .await;
            }
            self.set_msg_readed(record_id).await;
        }

        for pulled in pulled_events {
            let payload = Self::event_to_runtime_payload(&pulled.payload);
            let session_id = self
                .resolve_session_for_event(pulled.session_id.as_deref(), &payload)
                .await?;
            let item = self.build_session_input_item(payload, "event");
            if self
                .enqueue_session_input(session_id.as_str(), &item, InputQueueKind::Event)
                .await?
            {
                self.session_mgr
                    .mark_event_arrived(session_id.as_str(), &item)
                    .await
                    .map_err(|err| {
                        anyhow!("mark event arrival for session `{session_id}` failed: {err}")
                    })?;
            } else {
                self.session_mgr
                    .append_event(session_id.as_str(), item.payload.clone())
                    .await
                    .map_err(|err| anyhow!("dispatch event to session `{session_id}` failed: {err}"))?;
            }
        }
        Ok(())
    }

    async fn resolve_session_for_msg(
        &self,
        hinted_session_id: Option<&str>,
        record: &MsgRecordWithObject,
    ) -> Result<String> {
        if let Some(session_id) = normalize_session_id(hinted_session_id) {
            return Ok(session_id);
        }
        let msg_payload = serde_json::to_value(&record.msg).unwrap_or_else(|_| json!({}));
        if let Some(session_id) = extract_session_id_hint(&msg_payload) {
            return Ok(session_id);
        }

        let payload = Self::msg_record_to_runtime_payload(record);
        if let Some(session_id) = extract_session_id_hint(&payload) {
            return Ok(session_id);
        }
        self.resolve_router_by_llm("msg", &payload).await
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
        if let Err(err) = self
            .run_behavior_loop(
                router_behavior_name.as_str(),
                &router_cfg,
                router_id.as_str(),
                max_router_steps,
                None,
                BehaviorLoopMode::Router {
                    source_kind,
                    payload,
                    route_payload: &mut route_payload,
                    route_next_behavior: &mut route_next_behavior,
                },
            )
            .await
        {
            warn!(
                "agent.resolve_router_step_failed: did={} source={} behavior={} err={}",
                self.did, source_kind, router_behavior_name, err
            );
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
        let routed_new_session = route.new_session.clone();
        let mut routed_new_session_title = None::<String>;
        let mut routed_new_session_summary = None::<String>;
        let mut created_new_session = false;
        if resolved_session_id.is_none() {
            if let Some((title, summary)) = routed_new_session.as_ref() {
                routed_new_session_title = normalize_session_id(Some(title.as_str()));
                routed_new_session_summary = normalize_session_id(Some(summary.as_str()));
                if routed_new_session_title.is_some() || routed_new_session_summary.is_some() {
                    resolved_session_id = Some(format!("session-{}", Uuid::new_v4()));
                    created_new_session = true;
                }
            }
        }
        let mut session_id = resolved_session_id.unwrap_or_else(|| DEFAULT_SESSION_ID.to_string());
        let ensured_title = if created_new_session {
            routed_new_session_title.clone()
        } else {
            None
        };

        let session = match self
            .session_mgr
            .ensure_session(session_id.as_str(), ensured_title)
            .await
        {
            Ok(session) => session,
            Err(err) => {
                warn!(
                    "agent.resolve_router_ensure_session_failed: did={} source={} session={} err={}",
                    self.did, source_kind, session_id, err
                );
                session_id = DEFAULT_SESSION_ID.to_string();
                created_new_session = false;
                routed_new_session_summary = None;
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

            if created_new_session {
                if let Some(summary) = routed_new_session_summary
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    if guard.summary.trim().is_empty() {
                        guard.summary = summary.to_string();
                    }
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
                "new_session": routed_new_session,
                "new_session_created": created_new_session,
                "memory_queries": memory_queries,
                "reply": route_reply,
                "ts_ms": now_ms(),
            }));
            self.session_mgr
                .save_session_locked(&guard)
                .await
                .map_err(|err| {
                    anyhow!("save routed session `{}` failed: {}", guard.session_id, err)
                })?;
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

    async fn update_msg_record_session(&self, record_id: String, session_id: String) {
        let Some(msg_center) = self.deps.msg_center.as_ref() else {
            return;
        };
        if let Err(err) = msg_center
            .update_record_session(record_id.clone(), session_id.clone())
            .await
        {
            warn!(
                "agent.msg_update_session_failed: did={} record_id={} session_id={} err={}",
                self.did, record_id, session_id, err
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

    fn sanitize_kevent_token(raw: &str) -> String {
        let mut output = String::with_capacity(raw.len());
        let mut prev_dash = false;
        for ch in raw.chars() {
            if ch.is_ascii_alphanumeric() {
                output.push(ch.to_ascii_lowercase());
                prev_dash = false;
            } else if !prev_dash {
                output.push('-');
                prev_dash = true;
            }
        }
        let trimmed = output.trim_matches('-');
        if trimmed.is_empty() {
            "default".to_string()
        } else {
            trimmed.chars().take(80).collect()
        }
    }

    fn build_msg_center_event_patterns(owner: &DID) -> Vec<String> {
        let owner_token = Self::sanitize_kevent_token(owner.to_string().as_str());
        MSG_CENTER_EVENT_BOX_NAMES
            .iter()
            .map(|box_name| format!("/msg_center/{owner_token}/box/{box_name}/*"))
            .collect()
    }

    async fn ensure_msg_center_event_reader(&self) -> Option<Arc<EventReader>> {
        if self.deps.msg_center.is_none() {
            return None;
        }
        let Some(owner_did) = self.parse_owner_did_for_msg_center() else {
            return None;
        };

        let mut guard = self.msg_center_event_reader.lock().await;
        if let Some(reader) = guard.as_ref() {
            return Some(reader.clone());
        }

        let patterns = Self::build_msg_center_event_patterns(&owner_did);
        match self
            .kevent_client
            .create_event_reader(patterns.clone())
            .await
        {
            Ok(reader) => {
                let reader = Arc::new(reader);
                *guard = Some(reader.clone());
                debug!(
                    "agent.event_reader_created: did={} patterns={:?}",
                    self.did, patterns
                );
                Some(reader)
            }
            Err(err) => {
                debug!(
                    "agent.event_reader_create_failed: did={} patterns={:?} err={:?}",
                    self.did, patterns, err
                );
                None
            }
        }
    }

    async fn reset_msg_center_event_reader(&self) {
        let mut guard = self.msg_center_event_reader.lock().await;
        *guard = None;
    }

    fn msg_center_event_box_kind(event: &Event) -> Option<BoxKind> {
        let mut parts = event.eventid.split('/').filter(|part| !part.is_empty());
        let Some(prefix) = parts.next() else {
            return None;
        };
        let Some(_owner) = parts.next() else {
            return None;
        };
        let Some(scope) = parts.next() else {
            return None;
        };
        let Some(box_name) = parts.next() else {
            return None;
        };
        let Some(_event_name) = parts.next() else {
            return None;
        };

        if prefix != "msg_center" || scope != "box" {
            return None;
        }

        match box_name {
            "in" => Some(BoxKind::Inbox),
            "group_in" => Some(BoxKind::GroupInbox),
            "request" => Some(BoxKind::RequestBox),
            _ => None,
        }
    }

    fn append_all_msg_center_boxes(target: &mut Vec<BoxKind>) {
        for box_kind in [BoxKind::Inbox, BoxKind::GroupInbox, BoxKind::RequestBox] {
            if !target.contains(&box_kind) {
                target.push(box_kind);
            }
        }
    }

    fn collect_event_pull_targets(
        event: Event,
        msg_pull_boxes: &mut Vec<BoxKind>,
        pulled_events: &mut Vec<PulledEvent>,
    ) {
        if let Some(box_kind) = Self::msg_center_event_box_kind(&event) {
            if !msg_pull_boxes.contains(&box_kind) {
                msg_pull_boxes.push(box_kind);
            }
            return;
        }
        if let Some(pulled) = Self::kevent_event_to_pulled(event) {
            pulled_events.push(pulled);
        }
    }

    fn kevent_event_to_pulled(event: Event) -> Option<PulledEvent> {
        if event.eventid.starts_with("/msg_center/") {
            return None;
        }
        let session_id = extract_session_id_hint(&event.data).or_else(|| {
            let payload = serde_json::to_value(&event).unwrap_or_else(|_| json!({}));
            extract_session_id_hint(&payload)
        });
        Some(PulledEvent {
            session_id,
            payload: event,
        })
    }

    fn msg_record_to_pulled_msg(record: MsgRecordWithObject) -> PulledMsg {
        let session_id = normalize_session_id(record.record.session_id.as_deref()).or_else(|| {
            let msg_payload = serde_json::to_value(&record.msg).unwrap_or_else(|_| json!({}));
            extract_session_id_hint(&msg_payload)
        });
        PulledMsg { session_id, record }
    }

    fn msg_record_to_runtime_payload(record: &MsgRecordWithObject) -> Json {
        json!({
            "source": "msg_center",
            "record": record.record,
            "msg": record.msg,
        })
    }

    fn event_to_runtime_payload(event: &Event) -> Json {
        serde_json::to_value(event).unwrap_or_else(|_| json!({}))
    }

    fn build_session_input_item(&self, payload: Json, prefix: &str) -> SessionInputItem {
        SessionInputItem {
            id: self.extract_session_input_id(&payload, prefix),
            ts_ms: now_ms(),
            payload,
        }
    }

    fn extract_session_input_id(&self, payload: &Json, prefix: &str) -> String {
        for key in ["id", "msg_id", "event_id", "record_id"] {
            if let Some(id) = payload
                .get(key)
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return id.to_string();
            }
        }
        let seq = self.wakeup_seq.fetch_add(1, Ordering::Relaxed);
        format!("{prefix}-{}-{seq}", now_ms())
    }

    fn build_session_queue_binding(&self, session_id: &str) -> SessionQueueBinding {
        let session_token = Self::sanitize_kevent_token(session_id);
        let owner_token = Self::sanitize_kevent_token(self.did.as_str());
        let msg_queue_name = format!("agent-session-{session_token}-msg");
        let event_queue_name = format!("agent-session-{session_token}-event");
        SessionQueueBinding {
            msg_queue_urn: format!(
                "{}::{}::{}",
                SESSION_QUEUE_APP_ID, self.did, msg_queue_name
            ),
            event_queue_urn: format!(
                "{}::{}::{}",
                SESSION_QUEUE_APP_ID, self.did, event_queue_name
            ),
            msg_sub_id: format!("opendan-{owner_token}-{session_token}-msg"),
            event_sub_id: format!("opendan-{owner_token}-{session_token}-event"),
            msg_queue_name,
            event_queue_name,
        }
    }

    fn queue_already_exists(err: &kRPC::RPCErrors) -> bool {
        err.to_string().to_ascii_lowercase().contains("already exists")
    }

    async fn ensure_session_queue_binding(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionQueueBinding>> {
        let Some(msg_queue) = self.deps.msg_queue.as_ref() else {
            return Ok(None);
        };
        if let Some(binding) = self
            .session_queue_bindings
            .read()
            .await
            .get(session_id)
            .cloned()
        {
            return Ok(Some(binding));
        }

        let binding = self.build_session_queue_binding(session_id);
        let queue_cfg = QueueConfig {
            max_messages: Some(SESSION_QUEUE_MAX_MESSAGES),
            retention_seconds: Some(SESSION_QUEUE_RETENTION_SECONDS),
            sync_write: false,
            other_app_can_read: false,
            other_app_can_write: false,
            other_user_can_read: false,
            other_user_can_write: false,
        };

        if let Err(err) = msg_queue
            .create_queue(
                Some(binding.msg_queue_name.as_str()),
                SESSION_QUEUE_APP_ID,
                self.did.as_str(),
                queue_cfg.clone(),
            )
            .await
        {
            if !Self::queue_already_exists(&err) {
                return Err(anyhow!(
                    "create session msg queue failed: session={} err={}",
                    session_id,
                    err
                ));
            }
        }
        if let Err(err) = msg_queue
            .create_queue(
                Some(binding.event_queue_name.as_str()),
                SESSION_QUEUE_APP_ID,
                self.did.as_str(),
                queue_cfg,
            )
            .await
        {
            if !Self::queue_already_exists(&err) {
                return Err(anyhow!(
                    "create session event queue failed: session={} err={}",
                    session_id,
                    err
                ));
            }
        }

        if let Err(err) = msg_queue
            .subscribe(
                binding.msg_queue_urn.as_str(),
                self.did.as_str(),
                SESSION_QUEUE_APP_ID,
                Some(binding.msg_sub_id.clone()),
                SubPosition::Earliest,
            )
            .await
        {
            if !Self::queue_already_exists(&err) {
                return Err(anyhow!(
                    "subscribe session msg queue failed: session={} err={}",
                    session_id,
                    err
                ));
            }
        }
        if let Err(err) = msg_queue
            .subscribe(
                binding.event_queue_urn.as_str(),
                self.did.as_str(),
                SESSION_QUEUE_APP_ID,
                Some(binding.event_sub_id.clone()),
                SubPosition::Earliest,
            )
            .await
        {
            if !Self::queue_already_exists(&err) {
                return Err(anyhow!(
                    "subscribe session event queue failed: session={} err={}",
                    session_id,
                    err
                ));
            }
        }

        self.session_queue_bindings
            .write()
            .await
            .entry(session_id.to_string())
            .or_insert_with(|| binding.clone());
        Ok(Some(binding))
    }

    async fn enqueue_session_input(
        &self,
        session_id: &str,
        item: &SessionInputItem,
        kind: InputQueueKind,
    ) -> Result<bool> {
        let Some(msg_queue) = self.deps.msg_queue.as_ref() else {
            return Ok(false);
        };
        let Some(binding) = self.ensure_session_queue_binding(session_id).await? else {
            return Ok(false);
        };
        let queue_urn = match kind {
            InputQueueKind::Msg => binding.msg_queue_urn.as_str(),
            InputQueueKind::Event => binding.event_queue_urn.as_str(),
        };
        let payload = serde_json::to_vec(item).map_err(|err| {
            anyhow!(
                "serialize session queue payload failed: session={} kind={:?} err={}",
                session_id,
                kind,
                err
            )
        })?;
        let mut message = Message::new(payload);
        message.created_at = now_ms();
        msg_queue
            .post_message(queue_urn, message)
            .await
            .map_err(|err| {
                anyhow!(
                    "post session queue message failed: session={} kind={:?} err={}",
                    session_id,
                    kind,
                    err
                )
            })?;
        Ok(true)
    }

    async fn pull_session_queue_inputs(
        &self,
        session_id: &str,
    ) -> Result<(Vec<SessionInputItem>, Vec<SessionInputItem>)> {
        let Some(_msg_queue) = self.deps.msg_queue.as_ref() else {
            return Ok((vec![], vec![]));
        };
        let Some(binding) = self.ensure_session_queue_binding(session_id).await? else {
            return Ok((vec![], vec![]));
        };
        let msgs = self
            .pull_session_queue_inputs_by_kind(
                session_id,
                binding.msg_queue_urn.as_str(),
                binding.msg_sub_id.as_str(),
                "msg",
            )
            .await?;
        let events = self
            .pull_session_queue_inputs_by_kind(
                session_id,
                binding.event_queue_urn.as_str(),
                binding.event_sub_id.as_str(),
                "event",
            )
            .await?;
        Ok((msgs, events))
    }

    async fn pull_session_queue_inputs_by_kind(
        &self,
        session_id: &str,
        queue_urn: &str,
        sub_id: &str,
        prefix: &str,
    ) -> Result<Vec<SessionInputItem>> {
        let Some(msg_queue) = self.deps.msg_queue.as_ref() else {
            return Ok(vec![]);
        };
        let messages = msg_queue
            .fetch_messages(sub_id, SESSION_QUEUE_FETCH_BATCH, true)
            .await
            .map_err(|err| {
                anyhow!(
                    "fetch session queue messages failed: session={} queue={} err={}",
                    session_id,
                    queue_urn,
                    err
                )
            })?;
        if messages.is_empty() {
            return Ok(vec![]);
        }

        let mut output = Vec::<SessionInputItem>::with_capacity(messages.len());
        let mut last_index = None::<u64>;
        for msg in messages {
            last_index = Some(msg.index);
            let item = match serde_json::from_slice::<SessionInputItem>(msg.payload.as_slice()) {
                Ok(item) => item,
                Err(_) => {
                    let payload = serde_json::from_slice::<Json>(msg.payload.as_slice())
                        .unwrap_or(Json::String(String::from_utf8_lossy(&msg.payload).to_string()));
                    SessionInputItem {
                        id: self.extract_session_input_id(&payload, prefix),
                        ts_ms: now_ms(),
                        payload,
                    }
                }
            };
            output.push(item);
        }

        if let Some(last_index) = last_index {
            let _ = msg_queue.delete_message_before(queue_urn, last_index + 1).await;
        }
        Ok(output)
    }

    async fn run_session_loop(
        &self,
        session: Arc<Mutex<crate::agent_session::AgentSession>>,
    ) -> Result<()> {
        let wakeup_id = format!(
            "wakeup-{}-{}",
            now_ms(),
            self.wakeup_seq.fetch_add(1, Ordering::Relaxed)
        );

        let started_at = now_ms();
        let deadline_ms = started_at.saturating_add(self.cfg.max_walltime_ms);
        let mut step_count = 0_u32;
        let mut behavior_hops = 0_u32;

        loop {
            if step_count >= self.cfg.max_steps_per_wakeup {
                self.pause_running_session_to_wait(&session).await?;
                break;
            }
            if now_ms() >= deadline_ms {
                self.pause_running_session_to_wait(&session).await?;
                break;
            }
            if behavior_hops > self.cfg.max_behavior_hops {
                self.pause_running_session_to_wait(&session).await?;
                break;
            }

            let (session_id, behavior_name, state) = {
                let mut guard = session.lock().await;
                if guard.state == SessionState::Pause {
                    (guard.session_id.clone(), String::new(), guard.state)
                } else {
                    if guard.current_behavior.is_none() {
                        guard.current_behavior = Some(self.default_behavior.clone());
                        guard.step_index = 0;
                    }
                    (
                        guard.session_id.clone(),
                        guard
                            .current_behavior
                            .clone()
                            .unwrap_or_else(|| self.default_behavior.clone()),
                        guard.state,
                    )
                }
            };

            if state != SessionState::Running {
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

            let remaining_steps = self.cfg.max_steps_per_wakeup.saturating_sub(step_count);
            if remaining_steps == 0 {
                self.pause_running_session_to_wait(&session).await?;
                break;
            }

            let report = self
                .run_behavior_loop(
                    behavior_name.as_str(),
                    &behavior_cfg,
                    wakeup_id.as_str(),
                    remaining_steps,
                    Some(deadline_ms),
                    BehaviorLoopMode::Session { session: &session },
                )
                .await;

            let report = report?;
            step_count = step_count.saturating_add(report.executed_steps);

            if report.behavior_switched {
                behavior_hops = behavior_hops.saturating_add(1);
                debug!(
                    "agent.session_behavior_switched: did={} session={} from={} hops={} total_steps={}",
                    self.did, session_id, behavior_name, behavior_hops, step_count
                );
            }

            if report.hit_step_limit || report.hit_walltime {
                self.pause_running_session_to_wait(&session).await?;
                break;
            }

            if !report.keep_running {
                break;
            }

            if report.behavior_switched {
                continue;
            }

            if report.executed_steps == 0 {
                self.pause_running_session_to_wait(&session).await?;
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

    async fn run_behavior_loop(
        &self,
        behavior_name: &str,
        behavior_cfg: &BehaviorConfig,
        wakeup_id: &str,
        max_steps: u32,
        deadline_ms: Option<u64>,
        mut mode: BehaviorLoopMode<'_>,
    ) -> Result<BehaviorLoopReport> {
        enum PreparedBehaviorStep {
            Session {
                trace: TraceCtx,
                input: BehaviorExecInput,
                exec_input: SessionExecInput,
            },
            Router {
                trace: TraceCtx,
                input: BehaviorExecInput,
                source_kind: String,
            },
        }

        let is_session_mode = matches!(&mode, BehaviorLoopMode::Session { .. });
        let mut report = BehaviorLoopReport {
            keep_running: is_session_mode,
            ..Default::default()
        };

        while report.executed_steps < max_steps {
            if let Some(deadline) = deadline_ms {
                if now_ms() >= deadline {
                    report.hit_walltime = true;
                    report.keep_running = is_session_mode;
                    break;
                }
            }

            let prepared = match &mut mode {
                BehaviorLoopMode::Session { session } => {
                    let session_id = {
                        let guard = session.lock().await;
                        guard.session_id.clone()
                    };
                    let (queue_msgs, queue_events) =
                        self.pull_session_queue_inputs(session_id.as_str()).await?;
                    let (state, step_index, exec_input) = {
                        let mut guard = session.lock().await;
                        if !queue_msgs.is_empty() {
                            guard.new_msgs.extend(queue_msgs);
                        }
                        if !queue_events.is_empty() {
                            guard.new_events.extend(queue_events);
                        }
                        if guard.state != SessionState::Pause && guard.current_behavior.is_none() {
                            guard.current_behavior = Some(self.default_behavior.clone());
                            guard.step_index = 0;
                        }
                        if guard.state == SessionState::Running
                            && guard
                                .current_behavior
                                .as_deref()
                                .map(|name| !name.eq_ignore_ascii_case(behavior_name))
                                .unwrap_or(false)
                        {
                            report.behavior_switched = true;
                            return Ok(report);
                        }
                        (
                            guard.state,
                            guard.step_index,
                            if guard.state == SessionState::Running {
                                guard.generate_input(behavior_cfg)
                            } else {
                                None
                            },
                        )
                    };

                    if state != SessionState::Running {
                        report.keep_running = false;
                        return Ok(report);
                    }

                    let Some(exec_input) = exec_input else {
                        let session_id = {
                            let mut guard = session.lock().await;
                            guard.update_state(SessionState::Wait);
                            guard.session_id.clone()
                        };
                        self.session_mgr.save_session(&session_id).await?;
                        report.keep_running = false;
                        return Ok(report);
                    };

                    let trace = TraceCtx {
                        trace_id: format!("trace-{}-{}", now_ms(), report.executed_steps),
                        agent_did: self.did.clone(),
                        behavior: behavior_name.to_string(),
                        step_idx: step_index,
                        wakeup_id: wakeup_id.to_string(),
                    };
                    let input = self
                        .build_behavior_exec_input(&trace, behavior_cfg, &exec_input)
                        .await?;

                    PreparedBehaviorStep::Session {
                        trace,
                        input,
                        exec_input,
                    }
                }
                BehaviorLoopMode::Router {
                    source_kind,
                    payload,
                    ..
                } => {
                    let step_idx = report.executed_steps;
                    let trace = TraceCtx {
                        trace_id: format!("route-trace-{}-{}", now_ms(), step_idx),
                        agent_did: self.did.clone(),
                        behavior: behavior_name.to_string(),
                        step_idx,
                        wakeup_id: wakeup_id.to_string(),
                    };
                    let input = self
                        .build_router_exec_input(
                            &trace,
                            behavior_cfg,
                            source_kind,
                            payload,
                            max_steps,
                        )
                        .await;
                    PreparedBehaviorStep::Router {
                        trace,
                        input,
                        source_kind: source_kind.to_string(),
                    }
                }
            };

            let (trace, input, session_exec_input, router_source_kind) = match prepared {
                PreparedBehaviorStep::Session {
                    trace,
                    input,
                    exec_input,
                } => (trace, input, Some(exec_input), None),
                PreparedBehaviorStep::Router {
                    trace,
                    input,
                    source_kind,
                } => (trace, input, None, Some(source_kind)),
            };

            let step_result = self.run_behavior_step(&trace, behavior_cfg, input).await;
            let (llm_result, tracking, action_results) = match step_result {
                Ok(result) => result,
                Err(err) => {
                    if let Some(source_kind) = router_source_kind {
                        return Err(anyhow!(
                            "behavior loop step failed: source={} behavior={} step={} err={}",
                            source_kind,
                            behavior_name,
                            trace.step_idx,
                            err
                        ));
                    }

                    let session_id = if let BehaviorLoopMode::Session { session } = &mode {
                        let mut guard = session.lock().await;
                        guard.append_worklog(json!({
                            "type": "step_error",
                            "behavior": behavior_name,
                            "step_index": trace.step_idx,
                            "error": err.to_string(),
                            "ts_ms": now_ms(),
                        }));
                        guard.update_state(SessionState::Wait);
                        guard.session_id.clone()
                    } else {
                        String::new()
                    };
                    if !session_id.is_empty() {
                        let _ = self.session_mgr.save_session(&session_id).await;
                    }
                    return Err(err);
                }
            };

            report.executed_steps = report.executed_steps.saturating_add(1);

            if let Some(exec_input) = session_exec_input {
                self.handle_replies(
                    trace.clone(),
                    &exec_input.payload,
                    llm_result.reply.as_slice(),
                )
                .await;
                self.apply_memory_updates(&trace, llm_result.set_memory.as_slice())
                    .await;

                let (transition, side_effect_workspace_id, side_effect_session_id, step_summary) =
                    if let BehaviorLoopMode::Session { session } = &mode {
                        let mut guard = session.lock().await;
                        guard.update_input_used(&exec_input);
                        guard.apply_session_delta(llm_result.session_delta.as_slice());

                        let step_summary = build_step_summary(
                            &trace,
                            behavior_cfg,
                            &llm_result.next_behavior,
                            &tracking,
                            action_results.as_slice(),
                        );
                        guard.set_last_step_summary(step_summary.clone());
                        guard.append_worklog(step_summary.clone());

                        let workspace_id = resolve_session_workspace_id(&guard);
                        let session_id = guard.session_id.clone();
                        let transition = apply_behavior_transition(
                            &mut guard,
                            self.default_behavior.as_str(),
                            behavior_cfg.step_limit,
                            llm_result.next_behavior.as_deref(),
                        );
                        (
                            transition,
                            workspace_id,
                            session_id,
                            step_summary,
                        )
                    } else {
                        continue;
                    };
                self.apply_workspace_side_effects(
                    &trace,
                    side_effect_session_id.as_str(),
                    side_effect_workspace_id.as_deref(),
                    &step_summary,
                    llm_result.todo.as_slice(),
                )
                .await;
                let _ = self.session_mgr.save_session(&transition.session_id).await;

                if transition.behavior_switched {
                    report.behavior_switched = true;
                    report.keep_running = true;
                    return Ok(report);
                }
                if !transition.keep_running {
                    report.keep_running = false;
                    return Ok(report);
                }
            } else if let BehaviorLoopMode::Router {
                route_payload,
                route_next_behavior,
                ..
            } = &mut mode
            {
                if let LLMOutput::Json(value) = &tracking.raw_output {
                    **route_payload = Some(value.clone());
                }
                **route_next_behavior = normalize_session_id(llm_result.next_behavior.as_deref());
                if route_next_behavior.is_some() {
                    report.keep_running = false;
                    return Ok(report);
                }
            }
        }

        if report.executed_steps >= max_steps {
            report.hit_step_limit = true;
            report.keep_running = is_session_mode;
        }

        Ok(report)
    }

    async fn build_router_exec_input(
        &self,
        trace: &TraceCtx,
        behavior_cfg: &BehaviorConfig,
        source_kind: &str,
        payload: &Json,
        max_steps: u32,
    ) -> BehaviorExecInput {
        let memory_token_limit = if behavior_cfg.memory.total_limit > 0 {
            behavior_cfg.memory.total_limit
        } else {
            self.cfg.memory_token_limit
        };

        let memory = self
            .memory
            .load_memory(Some(memory_token_limit), vec![trace.behavior.clone()], None)
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
                value: trace.step_idx.to_string(),
            },
            EnvKV {
                key: "step.remaining".to_string(),
                value: max_steps.saturating_sub(trace.step_idx).to_string(),
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

        BehaviorExecInput {
            trace: trace.clone(),
            role_md: self.role_md.clone(),
            self_md: self.self_md.clone(),
            session_id: None,
            behavior_prompt: behavior_cfg.process_rule.clone(),
            env_context,
            inbox,
            memory,
            last_observations: vec![],
            limits: behavior_cfg.limits.clone(),
        }
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
            .load_memory(Some(memory_token_limit), vec![trace.behavior.clone()], None)
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
    ) -> Result<(
        crate::behavior::BehaviorLLMResult,
        LLMTrackingInfo,
        Vec<Json>,
    )> {
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
            let source = obj.get("source").cloned().unwrap_or_else(|| {
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

    async fn apply_workspace_side_effects(
        &self,
        trace: &TraceCtx,
        session_id: &str,
        workspace_id: Option<&str>,
        step_summary: &Json,
        todo: &[Json],
    ) {
        let Some(workspace_id) = normalize_session_id(workspace_id) else {
            return;
        };

        if self.tools.has_tool(TOOL_WORKLOG_MANAGE) {
            let summary_text = step_summary
                .get("summary")
                .and_then(|value| value.as_str())
                .unwrap_or("step summary");
            let call = ToolCall {
                name: TOOL_WORKLOG_MANAGE.to_string(),
                args: json!({
                    "action": "append_step_summary",
                    "owner_session_id": session_id,
                    "workspace_id": workspace_id,
                    "step_id": format!("{}#{}", trace.behavior, trace.step_idx),
                    "step_index": trace.step_idx,
                    "behavior": trace.behavior,
                    "summary": summary_text,
                    "payload": step_summary,
                }),
                call_id: format!(
                    "step-worklog-{}-{}",
                    trace.step_idx,
                    self.wakeup_seq.fetch_add(1, Ordering::Relaxed)
                ),
            };
            if let Err(err) = self.tools.call_tool(trace, call).await {
                warn!(
                    "agent.workspace_worklog_side_effect_failed: did={} session={} workspace={} behavior={} step={} err={}",
                    self.did, session_id, workspace_id, trace.behavior, trace.step_idx, err
                );
            }
        }

        if todo.is_empty() || !self.tools.has_tool(TOOL_TODO_MANAGE) {
            return;
        }

        let Some(mut delta) = build_todo_delta_payload(todo) else {
            return;
        };
        if let Some(delta_obj) = delta.as_object_mut() {
            let has_op_id = delta_obj
                .get("op_id")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_some();
            if !has_op_id {
                delta_obj.insert(
                    "op_id".to_string(),
                    Json::String(format!(
                        "{}:{}:{}",
                        trace.wakeup_id, trace.behavior, trace.step_idx
                    )),
                );
            }
        }

        let call = ToolCall {
            name: TOOL_TODO_MANAGE.to_string(),
            args: json!({
                "action": "apply_delta",
                "workspace_id": workspace_id,
                "session_id": session_id,
                "delta": delta,
                "actor_ctx": {
                    "kind": "root_agent",
                    "did": trace.agent_did,
                    "session_id": session_id,
                    "trace_id": trace.trace_id,
                },
            }),
            call_id: format!(
                "step-todo-{}-{}",
                trace.step_idx,
                self.wakeup_seq.fetch_add(1, Ordering::Relaxed)
            ),
        };
        if let Err(err) = self.tools.call_tool(trace, call).await {
            warn!(
                "agent.workspace_todo_side_effect_failed: did={} session={} workspace={} behavior={} step={} err={}",
                self.did, session_id, workspace_id, trace.behavior, trace.step_idx, err
            );
        }
    }

    async fn handle_replies(
        &self,
        trace: TraceCtx,
        source_payload: &Json,
        replies: &[ExecutorReply],
    ) {
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

        let mut outbound = MsgObject {
            from: sender_did.clone(),
            to: vec![target_did.clone()],
            kind: MsgObjKind::Chat,
            created_at_ms: now_ms(),
            content: MsgContent {
                format: Some(MsgContentFormat::TextPlain),
                content: content.to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        outbound.thread.topic = extract_reply_thread_key(source_payload);
        outbound.meta.insert("payload".to_string(), payload);

        let send_ctx = SendContext {
            contact_mgr_owner: Some(sender_did),
            ..Default::default()
        };
        match msg_center.post_send(outbound, Some(send_ctx), None).await {
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

enum BehaviorLoopMode<'a> {
    Session {
        session: &'a Arc<Mutex<crate::agent_session::AgentSession>>,
    },
    Router {
        source_kind: &'a str,
        payload: &'a Json,
        route_payload: &'a mut Option<Json>,
        route_next_behavior: &'a mut Option<String>,
    },
}

#[derive(Clone, Copy, Debug, Default)]
struct BehaviorLoopReport {
    executed_steps: u32,
    keep_running: bool,
    behavior_switched: bool,
    hit_step_limit: bool,
    hit_walltime: bool,
}

#[derive(Debug, Default)]
struct RouterResolution {
    session_id: Option<String>,
    new_session: Option<(String, String)>,
    next_behavior: Option<String>,
    reply: Option<String>,
    memory_queries: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RouteLLMResult {
    session_id: Option<String>,
    new_session: Option<(String, String)>,
    next_behavior: Option<String>,
    memory_queries: Vec<String>,
    reply: Option<String>,
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
        if fs::metadata(&path)
            .await
            .map(|meta| meta.is_file())
            .unwrap_or(false)
        {
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
        let parsed: Json = serde_json::from_str(&raw)
            .with_context(|| format!("parse agent document failed: path={}", path.display()))?;
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
    fs::metadata(path)
        .await
        .map(|meta| meta.is_dir())
        .unwrap_or(false)
}

fn resolve_session_workspace_id(session: &crate::agent_session::AgentSession) -> Option<String> {
    normalize_session_id(session.local_workspace_id.as_deref())
        .or_else(|| extract_workspace_id_from_json(session.workspace_info.as_ref()))
        .or_else(|| extract_workspace_id_from_json(Some(&session.meta)))
}

fn extract_workspace_id_from_json(value: Option<&Json>) -> Option<String> {
    let value = value?;
    for pointer in [
        "/workspace_id",
        "/local_workspace_id",
        "/id",
        "/workspace/id",
        "/workspace/workspace_id",
        "/workspace/local_workspace_id",
        "/binding/workspace_id",
        "/binding/local_workspace_id",
    ] {
        let parsed = value
            .pointer(pointer)
            .and_then(|item| item.as_str())
            .map(str::trim)
            .filter(|item| !item.is_empty());
        if let Some(workspace_id) = parsed {
            return Some(workspace_id.to_string());
        }
    }
    None
}

fn build_todo_delta_payload(todo: &[Json]) -> Option<Json> {
    let ops = todo
        .iter()
        .filter(|item| !item.is_null())
        .cloned()
        .collect::<Vec<_>>();
    if ops.is_empty() {
        return None;
    }

    if ops.len() == 1 {
        if let Some(delta) = ops[0].as_object() {
            if delta.get("ops").and_then(|value| value.as_array()).is_some() {
                return Some(Json::Object(delta.clone()));
            }
            if let Some(nested_delta) = delta.get("delta").and_then(|value| value.as_object()) {
                if nested_delta
                    .get("ops")
                    .and_then(|value| value.as_array())
                    .is_some()
                {
                    return Some(Json::Object(nested_delta.clone()));
                }
            }
        }
    }

    Some(json!({ "ops": ops }))
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

fn parse_router_resolution(
    route_payload: Option<&Json>,
    next_behavior_hint: Option<&str>,
) -> RouterResolution {
    let mut out = RouterResolution {
        next_behavior: normalize_session_id(next_behavior_hint),
        ..Default::default()
    };
    let Some(route_payload) = route_payload else {
        return out;
    };

    let route = serde_json::from_value::<RouteLLMResult>(route_payload.clone()).unwrap_or_default();
    out.session_id = normalize_session_id(route.session_id.as_deref());
    out.new_session = route.new_session.and_then(|(title, summary)| {
        let title = normalize_session_id(Some(title.as_str()));
        let summary = normalize_session_id(Some(summary.as_str()));
        if title.is_none() && summary.is_none() {
            None
        } else {
            Some((title.unwrap_or_default(), summary.unwrap_or_default()))
        }
    });

    if out.next_behavior.is_none() {
        out.next_behavior = normalize_session_id(route.next_behavior.as_deref());
    }

    out.reply = normalize_session_id(route.reply.as_deref());
    out.memory_queries = route
        .memory_queries
        .iter()
        .filter_map(|item| normalize_session_id(Some(item.as_str())))
        .collect::<Vec<_>>();

    out
}

fn extract_reply_target_did(payload: &Json) -> Option<DID> {
    if let Some(did) = extract_reply_target_did_from_msg(payload) {
        return Some(did);
    }

    let items = payload
        .pointer("/new_msg")
        .and_then(|value| value.as_array())?;
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

    let Some(items) = payload
        .pointer("/new_msg")
        .and_then(|value| value.as_array())
    else {
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
    for pointer in [
        "/msg/thread/topic",
        "/thread/topic",
        "/msg/thread_key",
        "/thread_key",
        "/record/thread_key",
    ] {
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
        "/record/session_id",
        "/payload/session_id",
        "/payload/payload/session_id",
        "/msg/session_id",
        "/msg/payload/session_id",
        "/msg/meta/session_id",
        "/content/machine/data/session_id",
        "/msg/content/machine/data/session_id",
        "/msg/meta/payload/session_id",
        "/meta/payload/session_id",
        "/meta/session_id",
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
