use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::vec;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use buckyos_api::{
    get_buckyos_api_runtime,
    msg_queue::{Message, MsgQueueClient, QueueConfig, SubPosition},
    value_to_object_map, AiToolCall, AiccClient, BoxKind, Event, EventReader, KEventClient,
    KEventError, MsgCenterClient, MsgRecord, MsgRecordWithObject, MsgState, PostSendResult,
    SendContext, TaskManagerClient,
};
use chrono::Utc;
use log::{debug, info, warn};
use name_lib::DID;
use ndn_lib::{MsgContent, MsgContentFormat, MsgObjKind, MsgObject};

use serde_json::{json, Value as Json};
use tokio::sync::{Mutex, RwLock};
use tokio::time::sleep;
use tokio::{fs, task};
use uuid::Uuid;

use crate::agent_config::AIAgentConfig;
use crate::agent_environment::AgentEnvironment;
use crate::agent_memory::{AgentMemory, AgentMemoryConfig};
use crate::agent_session::{
    AgentSession, AgentSessionMgr, GetSessionTool, SessionInputItem, SessionState,
};
use crate::agent_tool::{
    normalize_tool_name, AgentPolicy, AgentToolManager, DoAction, DoActionResults, DoActions,
    TOOL_EXEC_BASH,
};
use crate::behavior::{
    AgentWorkEvent, BehaviorConfig, BehaviorExecInput, BehaviorLLMResult, LLMBehavior,
    LLMBehaviorDeps, LLMComputeError, LLMTrackingInfo, SessionRuntimeContext, Tokenizer,
    WorklogSink,
};

const AGENT_DOC_CANDIDATES: [&str; 2] = ["agent.json.doc", "Agent.json.doc"];
const MAX_MSG_PULL_PER_TICK: usize = 128;
const MAX_EVENT_PULL_TIMEOUT_MS: u64 = 1_000;
const MAX_SESSION_WORKER_IDLE_SLEEP_MS: u64 = 10_000;
const MSG_ROUTED_REASON: &str = "routed_by_opendan_runtime";
const MSG_CENTER_EVENT_BOX_PATTERN_NAMES: [&str; 9] = [
    "in",
    "inbox",
    "INBOX",
    "group_in",
    "group_inbox",
    "GROUP_INBOX",
    "request",
    "request_box",
    "REQUEST_BOX",
];
const SESSION_QUEUE_APP_ID: &str = "opendan";
const SESSION_QUEUE_RETENTION_SECONDS: u64 = 7 * 24 * 60 * 60;
const SESSION_QUEUE_MAX_MESSAGES: u64 = 4096;
const AGENT_BEHAVIOR_ROUTER_RESOLVE: &str = "resolve_router";
const AGENT_BEHAVIOR_WORK_DEFAULT: &str = "plan";
const SESSION_META_CREATOR_UI_SESSION_ID: &str = "creator_ui_session_id";

#[derive(Debug)]
struct PulledMsg {
    session_id: Option<String>,
    record: MsgRecordWithObject,
}

#[derive(Debug)]
struct PulledEvent;

#[derive(Clone, Copy, Debug)]
pub(crate) enum InputQueueKind {
    Msg,
    Event,
}

#[derive(Clone, Copy, Debug)]
enum RouteLinkReason {
    SessionHint,
    MsgRecordSession,
    DefaultSession,
    DefaultFallback,
}

impl RouteLinkReason {
    fn as_str(self) -> &'static str {
        match self {
            RouteLinkReason::SessionHint => "SESSION_HINT",
            RouteLinkReason::MsgRecordSession => "ACTIVE_SESSION",
            RouteLinkReason::DefaultFallback => "DEFAULT_FALLBACK",
            RouteLinkReason::DefaultSession => "DEFAULT_SESSION",
        }
    }
}

impl Default for RouteLinkReason {
    fn default() -> Self {
        Self::DefaultFallback
    }
}

#[derive(Clone, Debug, Default)]
struct RouteDecision {
    linked_session_ids: Vec<String>,
    reason: RouteLinkReason,
}

#[derive(Clone, Debug, Default)]
struct StepRouteTarget {
    title: Option<String>,
    summary: Option<String>,
    behavior: Option<String>,
}

#[derive(Clone, Debug)]
struct ReplyHistoryRecord {
    outbound: MsgObject,
    result: PostSendResult,
}

#[derive(Clone, Debug)]
struct ResourceOverlayStatus {
    selected_path: Option<PathBuf>,
    env_path: Option<PathBuf>,
    package_path: Option<PathBuf>,
}

#[derive(Clone, Debug)]
struct SessionQueueBinding {
    msg_queue_urn: String,
    event_queue_urn: String,
    msg_sub_id: String,
    event_sub_id: String,
}

#[derive(Clone, Debug)]
struct StepTransition {
    keep_running: bool,
    behavior_switched: bool,
}

#[derive(Clone, Debug)]
struct BehaviorLoopReport {
    executed_steps: u32,
    keep_running: bool,
    behavior_switched: bool,
    last_result: Option<BehaviorLLMResult>,
}

impl Default for BehaviorLoopReport {
    fn default() -> Self {
        Self {
            executed_steps: 0,
            keep_running: false,
            behavior_switched: false,
            last_result: None,
        }
    }
}

struct NoopWorklogSink;

#[async_trait]
impl WorklogSink for NoopWorklogSink {
    async fn emit(&self, _event: AgentWorkEvent) {}
}

#[derive(Clone)]
pub struct AIAgentDeps {
    pub taskmgr: Arc<TaskManagerClient>,
    pub msg_center: Option<Arc<MsgCenterClient>>,
    pub msg_queue: Option<Arc<MsgQueueClient>>,
}

impl AIAgentDeps {
    pub async fn get_aicc_client(&self) -> Result<Arc<AiccClient>, LLMComputeError> {
        let runtime = get_buckyos_api_runtime().map_err(|err| {
            LLMComputeError::Internal(format!("load buckyos runtime failed: {err}"))
        })?;
        let client = runtime
            .get_aicc_client()
            .await
            .map_err(|err| LLMComputeError::Provider(format!("init aicc client failed: {err}")))?;
        Ok(Arc::new(client))
    }
}

pub struct AIAgent {
    cfg: AIAgentConfig,
    did: DID,
    agent_name: String,
    msg_owner_did: DID,
    contact_mgr_owner_did: Option<DID>,

    role_md: String,
    self_md: String,

    policy: Arc<AgentPolicy>,
    behavior_cfg_cache: Arc<RwLock<HashMap<String, BehaviorConfig>>>,
    behavior_roots: Vec<PathBuf>,
    default_behavior: String,
    default_worker_behavior: String,
    tools: Arc<AgentToolManager>,
    wakeup_seq: AtomicU64,

    memory: AgentMemory,
    session_mgr: Arc<AgentSessionMgr>,
    environment: Arc<AgentEnvironment>,
    agent_env_root: PathBuf,

    tokenizer: Arc<SimpleTokenizer>,

    deps: AIAgentDeps,
    kevent_client: KEventClient,
    msg_center_event_reader: Mutex<Option<Arc<EventReader>>>,
    session_queue_bindings: Arc<RwLock<HashMap<String, SessionQueueBinding>>>,
}

impl AIAgent {
    pub async fn load(mut cfg: AIAgentConfig, deps: AIAgentDeps) -> Result<Self> {
        cfg.normalize()
            .map_err(|err| anyhow!("invalid agent config: {err}"))?;

        let agent_root = to_abs_path(&cfg.agent_root)?;
        let package_root = cfg
            .agent_package_root
            .as_ref()
            .map(|path| to_abs_path(path))
            .transpose()?;
        info!(
            "agent.persist_entity_prepare: kind=agent_root instance={} path={}",
            cfg.agent_instance_id,
            agent_root.display()
        );
        fs::create_dir_all(&agent_root).await.map_err(|err| {
            anyhow!(
                "create agent root failed: path={} err={}",
                agent_root.display(),
                err
            )
        })?;
        if let Some(package_root) = &package_root {
            info!(
                "agent.loader.package_root: instance={} path={}",
                cfg.agent_instance_id,
                package_root.display()
            );
        }

        let did_raw = load_agent_did(&cfg, &agent_root, package_root.as_deref()).await?;
        let did = DID::from_str(did_raw.as_str()).map_err(|err| {
            anyhow!(
                "invalid owner did in agent doc: did={:?} err={}",
                did_raw,
                err
            )
        })?;
        let contact_mgr_owner_did = if let Some(owner_did) = cfg
            .agent_owner_did
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            Some(DID::from_str(owner_did).map_err(|err| {
                anyhow!(
                    "invalid owner did in agent config: did={:?} err={}",
                    owner_did,
                    err
                )
            })?)
        } else {
            None
        };
        let msg_owner_did = did.clone();
        let agent_name = did.to_raw_host_name();
        let (role_md, role_status) = load_overlay_text_resource(
            &agent_root,
            package_root.as_deref(),
            &[cfg.role_file_name.as_str(), "prompts/role.md"],
            "# Role\nYou are an OpenDAN agent.",
        )
        .await?;
        let (self_md, self_status) = load_overlay_text_resource(
            &agent_root,
            package_root.as_deref(),
            &[cfg.self_file_name.as_str(), "prompts/self.md"],
            "# Self\n- Keep tasks traceable\n",
        )
        .await?;

        let behavior_roots = build_behavior_roots(
            &agent_root,
            package_root.as_deref(),
            &cfg.behaviors_dir_name,
        )
        .await?;

        let agent_env_root = resolve_agent_env_root(&agent_root).await?;
        let session_root = agent_env_root.join("sessions");

        let tools = Arc::new(AgentToolManager::new());

        let environment = Arc::new(
            AgentEnvironment::new(agent_env_root.clone())
                .await
                .map_err(|err| anyhow!("init agent environment failed: {err}"))?,
        );

        let default_behavior = resolve_default_behavior_name(&behavior_roots)
            .await
            .unwrap_or_else(|| AGENT_BEHAVIOR_ROUTER_RESOLVE.to_string());
        let default_worker_behavior =
            resolve_default_worker_behavior_name(&behavior_roots, default_behavior.as_str()).await;

        let session_store = Arc::new(
            AgentSessionMgr::new(agent_name.clone(), session_root, default_behavior.clone())
                .await
                .map_err(|err| anyhow!("init session store failed: {err}"))?,
        );

        environment
            .register_workshop_tools(&tools, session_store.clone())
            .map_err(|err| anyhow!("register workshop tools failed: {err}"))?;

        let memory = AgentMemory::new(AgentMemoryConfig::new(agent_root.clone()))
            .await
            .map_err(|err| anyhow!("init agent memory failed: {err}"))?;
        memory
            .register_tools(&tools)
            .map_err(|err| anyhow!("register memory tools failed: {err}"))?;

        tools
            .register_tool(GetSessionTool::new(session_store.clone()))
            .map_err(|err| anyhow!("register session tool failed: {err}"))?;

        let behavior_cfg_cache = Arc::new(RwLock::new(HashMap::new()));
        let policy = Arc::new(AgentPolicy::new(tools.clone(), behavior_cfg_cache.clone()));
        let kevent_source_node = msg_owner_did.to_raw_host_name();
        log_overlay_status("role", &cfg.agent_instance_id, &role_status);
        log_overlay_status("self", &cfg.agent_instance_id, &self_status);
        info!(
            "agent.loader.behaviors: instance={} roots={:?}",
            cfg.agent_instance_id, behavior_roots
        );

        let agent = Self {
            cfg,
            did,
            agent_name,
            msg_owner_did,
            contact_mgr_owner_did,
            role_md,
            self_md,
            behavior_roots,
            agent_env_root,
            tools,
            memory,
            environment,
            session_mgr: session_store,
            behavior_cfg_cache,
            policy,

            tokenizer: Arc::new(SimpleTokenizer),
            deps,
            kevent_client: KEventClient::new_full(kevent_source_node, None),
            msg_center_event_reader: Mutex::new(None),
            session_queue_bindings: Arc::new(RwLock::new(HashMap::new())),
            default_behavior,
            default_worker_behavior,
            wakeup_seq: AtomicU64::new(0),
        };
        info!(
            "agent.loader.identity: did={} msg_owner_did={} contact_mgr_owner_did={}",
            agent.did.to_string(),
            agent.msg_owner_did.to_string(),
            agent
                .contact_mgr_owner_did
                .as_ref()
                .map(|did| did.to_string())
                .unwrap_or_else(|| "<none>".to_string())
        );

        let _ = agent.load_behavior_config(&agent.default_behavior).await?;
        let _ = agent
            .load_behavior_config(&agent.default_worker_behavior)
            .await?;
        Ok(agent)
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
                        "agent.session_worker_loop exited with error: did={:?} worker={} err={}",
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
        let event_pull_timeout_ms = MAX_EVENT_PULL_TIMEOUT_MS;

        loop {
            if let Some(max_tick) = stop_after_ticks {
                if tick >= max_tick {
                    break;
                }
            }
            tick = tick.saturating_add(1);

            //支持运行时，通过修改session相关配置影响行为，不过位置似乎不对
            // self.session_mgr
            //     .refresh_all_statuses_from_disk()
            //     .await
            //     .map_err(|err| anyhow!("refresh session status failed: {err}"))?;

            //从 agent_pull_input ->dispatch到 session -> session behavior genereate_input() 最终消费
            let (pulled_msgs, pulled_events, waited_on_events) =
                self.pull_msgs_and_events(event_pull_timeout_ms).await?;

            let has_inputs = !pulled_msgs.is_empty() || !pulled_events.is_empty();
            if has_inputs {
                info!(
                    "{} pull_msgs_and_events success, dispatch_inputs: pulled_msgs={} pulled_events={} waited_on_events={}",
                    self.agent_name,
                    pulled_msgs.len(),
                    pulled_events.len(),
                    waited_on_events
                );
                self.dispatch_pulled_inputs(pulled_msgs, pulled_events)
                    .await?;
            }

            self.session_mgr
                .schedule_wait_timeouts(now_ms())
                .await
                .map_err(|err| anyhow!("schedule session wait-timeout failed: {err}"))?;
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
                //t
                let wait_ms = sleep_ms.min(MAX_SESSION_WORKER_IDLE_SLEEP_MS);
                let woke_by_notify = self
                    .session_mgr
                    .wait_for_ready_or_timeout(Duration::from_millis(wait_ms))
                    .await;
                if woke_by_notify {
                    debug!(
                        "{}.session_worker_wakeup: reason=notify wait_ms={}",
                        self.agent_name, wait_ms
                    );
                    sleep_ms = self.cfg.default_sleep_ms;
                } else {
                    sleep_ms = (sleep_ms.saturating_mul(2))
                        .min(self.cfg.max_sleep_ms)
                        .min(MAX_SESSION_WORKER_IDLE_SLEEP_MS);
                }
                continue;
            };

            sleep_ms = self.cfg.default_sleep_ms;
            let result = self.run_session_loop(session.clone()).await;

            if let Err(err) = result {
                warn!("agent.session_loop failed: did={:?} err={}", self.did, err);
            }
            let session_id = {
                let guard = session.lock().await;
                guard.session_id.clone()
            };
            if let Err(err) = self.session_mgr.save_session(&session_id).await {
                warn!(
                    "agent.session_save_failed: did={:?} session_id={} err={}",
                    self.did, session_id, err
                );
            }
        }

        Ok(())
    }

    async fn pull_msgs_and_events(
        &self,
        wait_timeout_ms: u64,
    ) -> Result<(Vec<PulledMsg>, Vec<PulledEvent>, bool)> {
        //目前Agent关心的外部输入,后续需要根据agent的配置订阅新的event
        // - /msg_center/{owner_did}/box/{box_name}/**
        let Some(event_reader) = self.ensure_msg_center_event_reader().await else {
            warn!("{}.event_reader_unavailable", self.agent_name);
            let pulled_msgs = self.pull_msg_packs().await;
            return Ok((pulled_msgs, vec![], false));
        };
        let mut pulled_events = Vec::<PulledEvent>::new();
        let mut msg_pull_boxes = Vec::<BoxKind>::new();
        match event_reader.pull_event(Some(wait_timeout_ms)).await {
            Ok(Some(event)) => {
                debug!(
                    "{}.event_pull_hit: event_id={} source_node={} ingress_node={}",
                    self.agent_name,
                    event.eventid,
                    event.source_node,
                    event.ingress_node.as_deref().unwrap_or("-")
                );
                Self::collect_event_pull_targets(event, &mut msg_pull_boxes, &mut pulled_events);

                debug!(
                    "{}.event_pull_targets: msg_pull_boxes={:?} pulled_events={}",
                    self.agent_name,
                    msg_pull_boxes,
                    pulled_events.len()
                );
            }
            Ok(None) => {
                // KEvent is a poll accelerator. Timeout still falls back to queue pull.
                debug!("{}.event_pull_timeout", self.agent_name);
                Self::append_all_msg_center_boxes_updated(&mut msg_pull_boxes);
            }
            Err(err) => {
                warn!(
                    "{}.event_pull_failed: phase=wait timeout_ms={} err={:?}",
                    self.agent_name, wait_timeout_ms, err
                );
                if matches!(err, KEventError::ReaderClosed(_)) {
                    self.reset_msg_center_event_reader().await;
                }
            }
        }

        let pulled_msgs = if msg_pull_boxes.is_empty() {
            vec![]
        } else {
            self.pull_msg_packs_by_boxes(msg_pull_boxes.as_slice())
                .await
        };
        debug!(
            "{}.pull_msgs_and_events_done: msg_pull_boxes={:?} pulled_msgs={} pulled_events={}",
            self.agent_name,
            msg_pull_boxes,
            pulled_msgs.len(),
            pulled_events.len()
        );
        Ok((pulled_msgs, pulled_events, true))
    }

    async fn pull_msg_packs(&self) -> Vec<PulledMsg> {
        let mut box_kinds = Vec::new();
        Self::append_all_msg_center_boxes_updated(&mut box_kinds);
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
            let state_filter = Self::msg_pull_state_filter_for_box(box_kind);
            debug!(
                "agent.msg_pull_box_begin: did={:?} box_kind={:?} state_filter={:?}",
                self.did, box_kind, state_filter
            );
            let mut pulled_in_box = 0usize;
            for attempt in 0..MAX_MSG_PULL_PER_TICK {
                debug!(
                    "agent.msg_pull_get_next_call: did={:?} box_kind={:?} attempt={} state_filter={:?}",
                    self.did,
                    box_kind,
                    attempt + 1,
                    state_filter
                );
                match msg_center
                    .get_next(
                        owner_did.clone(),
                        box_kind.clone(),
                        state_filter.clone(),
                        Some(true),
                        Some(true),
                    )
                    .await
                {
                    Ok(Some(record)) => {
                        pulled_in_box = pulled_in_box.saturating_add(1);
                        info!(
                            "{}.msg_pull_get_next_hit: box_kind={:?} attempt={} record_id={} state={:?} thread_key={:?}",
                            self.agent_name,
                            box_kind,
                            attempt + 1,
                            record.record.record_id,
                            record.record.state,
                            record.record.ui_session_id
                        );
                        if !Self::is_expected_pulled_msg_state(box_kind, &record.record.state) {
                            warn!(
                                "agent.msg_pull_unexpected_state: did={:?} box_kind={:?} record_id={} state={:?} expected=unread_or_reading",
                                self.did, box_kind, record.record.record_id, record.record.state
                            );
                            break;
                        }
                        out.push(Self::msg_record_to_pulled_msg(record));
                    }
                    Ok(None) => {
                        debug!(
                            "agent.msg_pull_get_next_miss: did={:?} box_kind={:?} attempt={} pulled_in_box={}",
                            self.did,
                            box_kind,
                            attempt + 1,
                            pulled_in_box
                        );
                        break;
                    }
                    Err(err) => {
                        warn!(
                            "agent.msg_pull_failed: did={:?} box_kind={:?} attempt={} err={}",
                            self.did,
                            box_kind,
                            attempt + 1,
                            err
                        );
                        break;
                    }
                }
            }
            debug!(
                "agent.msg_pull_box_done: did={:?} box_kind={:?} pulled_in_box={}",
                self.did, box_kind, pulled_in_box
            );
        }
        out
    }

    fn msg_pull_state_filter_for_box(box_kind: &BoxKind) -> Option<Vec<MsgState>> {
        match box_kind {
            BoxKind::Inbox | BoxKind::GroupInbox | BoxKind::RequestBox => {
                Some(vec![MsgState::Unread])
            }
            BoxKind::Outbox | BoxKind::TunnelOutbox => None,
        }
    }

    fn is_expected_pulled_msg_state(box_kind: &BoxKind, state: &MsgState) -> bool {
        match box_kind {
            BoxKind::Inbox | BoxKind::GroupInbox | BoxKind::RequestBox => {
                matches!(state, MsgState::Unread | MsgState::Reading)
            }
            BoxKind::Outbox | BoxKind::TunnelOutbox => true,
        }
    }

    async fn dispatch_pulled_inputs(
        &self,
        pulled_msgs: Vec<PulledMsg>,
        pulled_events: Vec<PulledEvent>,
    ) -> Result<()> {
        debug!(
            "agent.dispatch_pulled_inputs_begin: did={:?} pulled_msgs={} pulled_events={}",
            self.did,
            pulled_msgs.len(),
            pulled_events.len()
        );
        for pulled in pulled_msgs {
            let record_id = pulled.record.record.record_id.clone();
            let mut msg_record = pulled.record.record.clone();
            if msg_record
                .from_name
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none()
            {
                let session_from_name = pulled
                    .record
                    .msg
                    .as_ref()
                    .map(|msg| msg.from.to_raw_host_name())
                    .filter(|value| !value.trim().is_empty())
                    .or_else(|| {
                        let fallback = msg_record.from.to_raw_host_name();
                        (!fallback.trim().is_empty()).then_some(fallback)
                    });
                msg_record.from_name = AgentSession::resolve_msg_from_name(
                    &msg_record.from,
                    session_from_name.as_deref(),
                    Some(self.contact_mgr_owner_did()),
                )
                .await;
            }
            info!(
                "{}.dispatch_msg_begin: record_id={} state={:?} ui_session_id={:?}",
                self.agent_name, record_id, msg_record.state, msg_record.ui_session_id
            );

            let route_result = self
                .route_msg_pack(pulled.session_id.as_deref(), &pulled.record)
                .await?;

            info!(
                "{}.route_msg_pack: record_id={} route_reason={} target_sessions={:?}",
                self.agent_name,
                record_id,
                route_result.reason.as_str(),
                route_result.linked_session_ids,
            );

            let session_input = SessionInputItem {
                msg: Some(msg_record),
                event_id: None,
            };

            for session_id in &route_result.linked_session_ids {
                self.enqueue_session_input(
                    session_id.as_str(),
                    &session_input,
                    InputQueueKind::Msg,
                )
                .await?;

                self.session_mgr
                    .try_wakeup_session_by_input_item(session_id.as_str(), &session_input)
                    .await
                    .map_err(|err| {
                        anyhow!("mark msg arrival for session `{session_id}` failed: {err}")
                    })?;
                info!(
                    "{}.try_wakeup_session_by_input_item: record_id={} session_id={}",
                    self.agent_name, record_id, session_id
                );
            }

            self.set_msg_readed(record_id).await;
        }

        for _pulled in pulled_events {
            //TODO：Event可能能1次唤醒多个Session，这里需要改造
            unimplemented!()
        }
        info!("agent.dispatch_pulled_inputs_done: did={:?}", self.did);
        Ok(())
    }

    async fn route_msg_pack(
        &self,
        hinted_session_id: Option<&str>,
        record: &MsgRecordWithObject,
    ) -> Result<RouteDecision> {
        if let Some(session_id) = hinted_session_id {
            return Ok(RouteDecision {
                linked_session_ids: vec![session_id.to_string()],
                reason: RouteLinkReason::SessionHint,
            });
        }
        if let Some(session_id) = &record.record.ui_session_id {
            return Ok(RouteDecision {
                linked_session_ids: vec![session_id.clone()],
                reason: RouteLinkReason::MsgRecordSession,
            });
        }

        let default_ui_session_id = self
            .session_mgr
            .get_ui_session_id(&record.get_target_did(), &record.get_msg_tunnel_ui_id());

        return Ok(RouteDecision {
            linked_session_ids: vec![default_ui_session_id],
            reason: RouteLinkReason::DefaultSession,
        });
    }

    async fn run_session_loop(
        &self,
        session: Arc<Mutex<crate::agent_session::AgentSession>>,
    ) -> Result<()> {
        let wakeup_id = format!(
            "{}.session-wakeup-{}",
            self.agent_name,
            self.wakeup_seq.fetch_add(1, Ordering::Relaxed)
        );

        let started_at = now_ms();
        let deadline_ms = started_at.saturating_add(self.cfg.max_walltime_ms);
        let mut step_count = 0_u32;
        let session_id_for_log = {
            let guard = session.lock().await;
            guard.session_id.clone()
        };
        info!(
            "agent.session_loop_start: session_id={} wakeup_id={} started_at_ms={}",
            session_id_for_log, wakeup_id, started_at
        );

        loop {
            if step_count >= self.cfg.max_steps_per_wakeup {
                warn!(
                    "agent.session_loop_yield: session_id={} wakeup_id={} reason=step_budget_reached step_count={} max_steps_per_wakeup={}",
                    session_id_for_log, wakeup_id, step_count, self.cfg.max_steps_per_wakeup
                );
                self.set_running_session_to_ready(&session).await?;
                break;
            }
            if now_ms() >= deadline_ms {
                warn!(
                    "agent.session_loop_yield: session_id={} wakeup_id={} reason=walltime_reached deadline_ms={}",
                    session_id_for_log, wakeup_id, deadline_ms
                );
                self.set_running_session_to_ready(&session).await?;
                break;
            }

            let (session_id, behavior_name, state) = {
                let mut guard = session.lock().await;
                if guard.current_behavior.trim().is_empty() {
                    let fallback = if AgentSession::is_work_session_id(guard.session_id.as_str()) {
                        AGENT_BEHAVIOR_WORK_DEFAULT
                    } else {
                        AGENT_BEHAVIOR_ROUTER_RESOLVE
                    };
                    warn!(
                        "agent.session_empty_behavior_defaulted: session_id={} behavior={}",
                        guard.session_id, fallback
                    );
                    guard.current_behavior = fallback.to_string();
                }
                (
                    guard.session_id.clone(),
                    guard.current_behavior.clone(),
                    guard.state,
                )
            };

            if state != SessionState::Running {
                break;
            }

            let behavior_cfg = match self.load_behavior_config(&behavior_name).await {
                Ok(cfg) => cfg,
                Err(err) => {
                    warn!(
                        "{}'s behavior {} not found! err={}",
                        self.agent_name, behavior_name, err
                    );
                    break;
                }
            };

            let llm_report = self
                .run_behavior_loop(
                    session.clone(),
                    behavior_name.as_str(),
                    &behavior_cfg,
                    wakeup_id.as_str(),
                )
                .await;

            if llm_report.is_err() {
                //很少会到这里，通常异常都在run_behavior_loop中处理
                warn!(
                    "{}.{} run behavior {} loop failed! err={}",
                    self.agent_name,
                    session_id,
                    behavior_name,
                    llm_report.err().unwrap()
                );
                self.set_running_session_to_wait(&session).await?;
                break;
            }

            let report = llm_report?;
            step_count = step_count.saturating_add(report.executed_steps);

            if report.behavior_switched {
                info!(
                    "{}.{} behavior switched from {}",
                    self.agent_name, session_id, behavior_name,
                );
            }

            if !report.keep_running {
                break;
            }
        }

        let need_demote_running = {
            let guard = session.lock().await;
            guard.state == SessionState::Running
        };
        if need_demote_running {
            warn!(
                "agent.session_loop_finalize_running_to_wait: session_id={} wakeup_id={}",
                session_id_for_log, wakeup_id
            );
            self.set_running_session_to_wait(&session).await?;
        }

        Ok(())
    }

    //Loop执行到 wait或next_behavior != none (switch behavior)
    async fn run_behavior_loop(
        &self,
        session: Arc<Mutex<AgentSession>>,
        behavior_name: &str,
        behavior_cfg: &BehaviorConfig,
        wakeup_id: &str,
    ) -> Result<BehaviorLoopReport> {
        let mut result_report = BehaviorLoopReport::default();

        loop {
            let (session_id, current_step_index) = {
                //TODO 支持sub agent,可能还需要考虑读取owner agent的pause状态
                let mut guard = session.lock().await;
                if guard.state != SessionState::Running {
                    break;
                }
                if guard.is_paused {
                    break;
                }
                let current_step_index = guard.step_index;
                let session_id = guard.session_id.clone();
                if guard.step_index == 0 {
                    //TODO 应该是session有一个通用函数，自动load当前behavior的skills
                    guard.loaded_skills = behavior_cfg.toolbox.load_skills.clone();
                }
                (session_id, current_step_index)
            };

            info!(
                "{}.run_behavior_loop: session_id={} behavior_name={} current_step_index={}",
                self.agent_name, session_id, behavior_name, current_step_index
            );

            let trace = SessionRuntimeContext {
                trace_id: wakeup_id.to_string(),
                agent_name: self.agent_name.clone(),
                behavior: behavior_name.to_string(),
                step_idx: current_step_index,
                wakeup_id: wakeup_id.to_string(),
                session_id: session_id.clone(),
            };

            // Ensure per-session queues/subscriptions exist before template placeholders
            // pull `new_msg`/`new_event` from kmsg.
            self.ensure_session_queue_binding(session_id.as_str())
                .await?;

            //build input
            let input = self
                .generate_input(&trace, behavior_name, behavior_cfg, session.clone())
                .await?;

            if input.is_none() {
                result_report.keep_running = false;
                //DO NOTHING, no side effects
                break;
            }
            let input = input.unwrap();
            self.append_incoming_message_worklogs(session.clone(), &trace)
                .await;

            let llm_behavior = LLMBehavior::new(
                behavior_cfg.to_llm_behavior_config(),
                LLMBehaviorDeps {
                    taskmgr: self.deps.taskmgr.clone(),
                    #[cfg(test)]
                    aicc: self
                        .deps
                        .get_aicc_client()
                        .await
                        .map_err(|err| anyhow!("load aicc client failed: {err}"))?,
                    tools: self.tools.clone(),
                    memory: Some(self.memory.clone()),
                    policy: self.policy.clone(),
                    worklog: Arc::new(NoopWorklogSink),
                    tokenizer: self.tokenizer.clone(),
                    environment: self.environment.clone(),
                },
            );

            //run step
            let (llm_result, tracking) = llm_behavior
                .run_step(&input)
                .await
                .map_err(|err| anyhow!("llm behavior step failed: {err}"))?;

            //execute side effects
            self.dispatch_step_msg_records(session.clone(), &llm_result)
                .await?;

            let mut reply_history = Vec::<ReplyHistoryRecord>::new();
            if llm_result.route_session_id.is_none() {
                reply_history = self
                    .handle_reply(session.clone(), &trace, llm_result.reply.as_deref())
                    .await;
            }
            self.append_reply_message_worklogs(session.clone(), &trace, &reply_history)
                .await;

            self.apply_memory_updates(&trace, &llm_result.set_memory)
                .await;

            //如果这里执行action时，触发了请求用户授权，如何从这里重启恢复? 不恢复，此时没有side event,相当于把这个step重新做一次
            //所有action都通过授权才会执行
            let action_plan = merged_actions_from_llm_result(&llm_result);
            let action_results = self.execute_actions(&trace, &action_plan).await;
            self.append_action_record_worklogs(session.clone(), &trace, &tracking, &action_results)
                .await;

            let step_summary = build_step_summary(
                &trace,
                behavior_cfg,
                &llm_result,
                &tracking,
                &action_results,
                session.clone(),
            )
            .await;
            self.append_step_summary_worklog(
                session.clone(),
                &trace,
                &llm_result,
                &action_results,
                step_summary.as_deref(),
            )
            .await;

            let (msg_cursor, msg_owner_agent) = {
                let mut guard = session.lock().await;
                guard.last_step_summary = step_summary.clone();
                (guard.msg_kmsgqueue_curosr, guard.owner_agent.clone())
            };

            //write just readed input msg to msg_record(both work-session record and ui-session record)
            self.persist_step_history_records(
                session.clone(),
                session_id.as_str(),
                reply_history.as_slice(),
            )
            .await;

            //update input is all used
            self.commit_session_queue_msg_ack(
                msg_owner_agent.as_str(),
                session_id.as_str(),
                msg_cursor,
            )
            .await?;
            {
                let mut guard = session.lock().await;
                guard.just_readed_input_msg.clear();
            }

            result_report.executed_steps = result_report.executed_steps + 1;
            result_report.last_result = Some(llm_result.clone());

            //process next_behavior
            let transition = {
                let mut guard = session.lock().await;
                apply_session_behavior_transition(
                    &mut guard,
                    self.default_behavior.as_str(),
                    behavior_cfg.step_limit,
                    behavior_cfg.faild_back.as_deref(),
                    llm_result.next_behavior.as_deref(),
                )
            };
            result_report.keep_running = transition.keep_running;
            result_report.behavior_switched = transition.behavior_switched;

            if !transition.keep_running || llm_result.next_behavior.is_some() {
                break;
            }
        }

        Ok(result_report)
    }

    async fn dispatch_step_msg_records(
        &self,
        session: Arc<Mutex<AgentSession>>,
        llm_result: &BehaviorLLMResult,
    ) -> Result<()> {
        let mut route_targets = HashMap::<String, StepRouteTarget>::new();

        if let Some(session_id) = llm_result.route_session_id.as_deref() {
            route_targets
                .entry(session_id.to_string())
                .or_insert_with(StepRouteTarget::default);
        } else if let Some((new_session_title, new_session_summary)) =
            llm_result.new_session.as_ref()
        {
            let new_session_id = self.gen_new_work_session_id();
            route_targets.insert(
                new_session_id,
                StepRouteTarget {
                    title: Some(new_session_title.clone()),
                    summary: Some(new_session_summary.clone()),
                    behavior: Some(self.default_worker_behavior.clone()),
                },
            );
        }

        if route_targets.is_empty() {
            return Ok(());
        }

        let (source_session_id, step_inputs_raw) = {
            let guard = session.lock().await;
            (
                guard.session_id.clone(),
                guard.just_readed_input_msg.clone(),
            )
        };
        if step_inputs_raw.is_empty() {
            return Ok(());
        }

        let mut step_msg_inputs = Vec::<SessionInputItem>::with_capacity(step_inputs_raw.len());
        for payload in step_inputs_raw {
            let item =
                serde_json::from_slice::<SessionInputItem>(payload.as_slice()).map_err(|err| {
                    anyhow!(
                        "deserialize step input payload failed: source_session={} err={}",
                        source_session_id,
                        err
                    )
                })?;
            if item.msg.is_some() {
                step_msg_inputs.push(item);
            }
        }

        if step_msg_inputs.is_empty() {
            return Ok(());
        }

        let default_remote = step_msg_inputs
            .iter()
            .find_map(|item| item.msg.as_ref().map(|record| record.from.to_string()));
        let creator_ui_session_id = self
            .resolve_creator_ui_session_id(source_session_id.as_str(), step_msg_inputs.as_slice())
            .await;

        for (target_session_id, target) in route_targets {
            let target_session = self
                .session_mgr
                .ensure_session(
                    target_session_id.as_str(),
                    target.title,
                    target.behavior.as_deref(),
                    default_remote.as_deref(),
                )
                .await?;

            if let Some(summary) = target.summary {
                let mut guard = target_session.lock().await;
                if guard.summary.trim().is_empty() {
                    guard.summary = summary;
                }
                if let Some(ui_session_id) = creator_ui_session_id.as_deref() {
                    Self::set_creator_ui_session_id_meta(&mut guard.meta, ui_session_id);
                }
            } else if let Some(ui_session_id) = creator_ui_session_id.as_deref() {
                let mut guard = target_session.lock().await;
                Self::set_creator_ui_session_id_meta(&mut guard.meta, ui_session_id);
            }

            for input_item in &step_msg_inputs {
                self.enqueue_session_input(
                    target_session_id.as_str(),
                    input_item,
                    InputQueueKind::Msg,
                )
                .await?;
                self.session_mgr
                    .try_wakeup_session_by_input_item(target_session_id.as_str(), input_item)
                    .await
                    .map_err(|err| {
                        anyhow!(
                            "wake routed session by step msg failed: target_session={} err={}",
                            target_session_id,
                            err
                        )
                    })?;
            }
        }

        Ok(())
    }

    fn gen_new_work_session_id(&self) -> String {
        let new_uuid = Uuid::new_v4().simple().to_string();
        let now = Utc::now().format("%y%m%d").to_string();
        format!("work-{}-{}", now, new_uuid)
    }

    fn get_params_from_behavior_name(behavior_name: &str) -> Option<Json> {
        // behavior_name = "DO:todo=T001" or "DO:todo=T001:step=2"
        // return Some(json!({ "todo": "T001" }));
        let params_str = behavior_name.split(':').nth(1)?.trim();
        if params_str.is_empty() {
            return None;
        }
        let mut map = serde_json::Map::new();
        for pair in params_str.split(':') {
            let pair = pair.trim();
            if let Some((k, v)) = pair.split_once('=') {
                let key = k.trim();
                let value = v.trim();
                if !key.is_empty() {
                    map.insert(key.to_string(), Json::String(value.to_string()));
                }
            }
        }
        if map.is_empty() {
            None
        } else {
            Some(Json::Object(map))
        }
    }

    async fn generate_input(
        &self,
        trace: &SessionRuntimeContext,
        behavior_name: &str,
        behavior_cfg: &BehaviorConfig,
        session: Arc<Mutex<AgentSession>>,
    ) -> Result<Option<BehaviorExecInput>> {
        //核心:用agent_environment构造step_summary 和 input，至少要有一个，否则就没有有效的收入
        //如果step>0,则构造step_summary
        let mut env_context = HashMap::<String, Json>::new();
        let (session_id, step_index, last_step_summary) = {
            let guard = session.lock().await;
            (
                guard.session_id.clone(),
                guard.step_index,
                guard.last_step_summary.clone(),
            )
        };
        if step_index > 0 {
            if let Some(step_summary) = last_step_summary {
                let value = serde_json::to_value(&step_summary).unwrap_or(Json::Null);
                env_context.insert("step_summary".to_string(), value);
            }
        }

        let params = Self::get_params_from_behavior_name(behavior_name);
        if let Some(params) = params {
            env_context.insert("params".to_string(), params);
        }

        //构造input_prompt
        let input_prompt_result = AgentEnvironment::render_prompt(
            behavior_cfg.input.as_str(),
            &env_context,
            session.clone(),
        )
        .await?;

        if input_prompt_result.successful_count > 0 {
            return Ok(Some(BehaviorExecInput {
                trace: trace.clone(),
                role_md: self.role_md.clone(),
                self_md: self.self_md.clone(),
                behavior_prompt: behavior_cfg.process_rule.clone(),
                limits: behavior_cfg.limits.clone(),
                behavior_cfg: behavior_cfg.clone(),
                session_id,
                input_prompt: input_prompt_result.rendered,
                last_step_prompt: String::new(),
                session: Some(session.clone()),
            }));
        } else {
            return Ok(None);
        }
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
                "agent.msg_mark_read_failed: did={:?} record_id={} err={}",
                self.did, record_id, err
            );
        } else {
            info!(
                "agent.msg_mark_read_ok: did={:?} record_id={} state={:?}",
                self.did,
                record_id,
                MsgState::Readed
            );
        }
    }

    fn build_msg_center_event_patterns(owner: &DID) -> Vec<String> {
        let owner_token = owner.to_raw_host_name();
        let mut owner_tokens = vec![owner_token.clone()];
        let normalized_owner_token = owner_token.to_ascii_lowercase();
        if normalized_owner_token != owner_token {
            owner_tokens.push(normalized_owner_token);
        }

        let mut out = Vec::new();
        let mut dedup = HashSet::<String>::new();
        for owner_token in owner_tokens {
            for box_name in MSG_CENTER_EVENT_BOX_PATTERN_NAMES {
                for pattern in [
                    format!("/msg_center/{owner_token}/box/{box_name}/**"),
                    format!("/msg_center/{owner_token}/{box_name}/**"),
                ] {
                    if dedup.insert(pattern.clone()) {
                        out.push(pattern);
                    }
                }
            }
        }
        out
    }

    fn msg_center_event_name_to_box_kind(raw_name: &str) -> Option<BoxKind> {
        let normalized = raw_name.trim().to_ascii_lowercase().replace('-', "_");
        match normalized.as_str() {
            "in" | "inbox" => Some(BoxKind::Inbox),
            "group_in" | "group_inbox" => Some(BoxKind::GroupInbox),
            "request" | "request_box" => Some(BoxKind::RequestBox),
            _ => None,
        }
    }

    async fn ensure_msg_center_event_reader(&self) -> Option<Arc<EventReader>> {
        let mut guard = self.msg_center_event_reader.lock().await;
        if let Some(reader) = guard.as_ref() {
            return Some(reader.clone());
        }

        let patterns = Self::build_msg_center_event_patterns(&self.msg_owner_did);
        match self
            .kevent_client
            .create_event_reader(patterns.clone())
            .await
        {
            Ok(reader) => {
                let reader = Arc::new(reader);
                *guard = Some(reader.clone());
                info!(
                    "agent.event_reader_created: did={:?} msg_owner_did={:?} patterns={:?} reader_id={}",
                    self.did,
                    self.msg_owner_did.to_string(),
                    patterns,
                    reader.reader_id()
                );
                Some(reader)
            }
            Err(err) => {
                if matches!(err, KEventError::InvalidPattern(_)) {
                    warn!(
                        "agent.event_reader_create_failed: did={:?} msg_owner_did={:?} reason=invalid_pattern patterns={:?} err={:?}",
                        self.did,
                        self.msg_owner_did.to_string(),
                        patterns,
                        err
                    );
                } else {
                    debug!(
                        "agent.event_reader_create_failed: did={:?} msg_owner_did={:?} patterns={:?} err={:?}",
                        self.did,
                        self.msg_owner_did.to_string(),
                        patterns,
                        err
                    );
                }
                None
            }
        }
    }

    async fn reset_msg_center_event_reader(&self) {
        let mut guard = self.msg_center_event_reader.lock().await;
        *guard = None;
    }

    fn msg_center_event_box_kind(event: &Event) -> Option<BoxKind> {
        let parts: Vec<&str> = event
            .eventid
            .split('/')
            .filter(|part| !part.is_empty())
            .collect();
        if parts.len() < 3 {
            return None;
        }
        if parts[0] != "msg_center" {
            return None;
        }

        if let Some(index) = parts.iter().position(|segment| *segment == "box") {
            if let Some(box_name) = parts.get(index + 1) {
                return Self::msg_center_event_name_to_box_kind(box_name);
            }
        }
        Self::msg_center_event_name_to_box_kind(parts[2])
    }

    fn append_all_msg_center_boxes_updated(target: &mut Vec<BoxKind>) {
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
        if event.eventid.starts_with("/msg_center/") {
            warn!(
                "agent.msg_center_event_unrecognized: event_id={} fallback=pull_all_boxes",
                event.eventid
            );
            Self::append_all_msg_center_boxes_updated(msg_pull_boxes);
            return;
        }
        if let Some(pulled) = Self::kevent_event_to_pulled(event) {
            pulled_events.push(pulled);
        }
    }

    fn kevent_event_to_pulled(event: Event) -> Option<PulledEvent> {
        if event.eventid.starts_with("/msg_center/") {
            debug!(
                "agent.kevent_event_ignored: scope=msg_center event_id={}",
                event.eventid
            );
            return None;
        }
        Some(PulledEvent)
    }

    fn msg_record_to_pulled_msg(record: MsgRecordWithObject) -> PulledMsg {
        let session_id = record.record.ui_session_id.clone();
        PulledMsg { session_id, record }
    }

    fn build_session_queue_binding(&self, session_id: &str) -> SessionQueueBinding {
        SessionQueueBinding {
            msg_queue_urn: Self::get_session_kmsgqueue_urn(
                self.agent_name.as_str(),
                session_id,
                InputQueueKind::Msg,
            ),
            event_queue_urn: Self::get_session_kmsgqueue_urn(
                self.agent_name.as_str(),
                session_id,
                InputQueueKind::Event,
            ),
            msg_sub_id: Self::get_session_kmsgqueue_sub_id(
                self.agent_name.as_str(),
                session_id,
                InputQueueKind::Msg,
            ),
            event_sub_id: Self::get_session_kmsgqueue_sub_id(
                self.agent_name.as_str(),
                session_id,
                InputQueueKind::Event,
            ),
        }
    }

    fn queue_resource_not_found(err: &kRPC::RPCErrors) -> bool {
        err.to_string().to_ascii_lowercase().contains("not found")
    }

    fn queue_resource_already_exists(err: &kRPC::RPCErrors) -> bool {
        err.to_string()
            .to_ascii_lowercase()
            .contains("already exists")
    }

    async fn ensure_session_queue_exists(
        &self,
        msg_queue: &MsgQueueClient,
        session_id: &str,
        queue_name: &str,
        queue_urn: &str,
        queue_cfg: QueueConfig,
    ) -> Result<()> {
        match msg_queue.get_queue_stats(queue_urn).await {
            Ok(_) => Ok(()),
            Err(check_err) => {
                if !Self::queue_resource_not_found(&check_err) {
                    warn!(
                        "check session queue failed, fallback create: session={} queue={} err={}",
                        session_id, queue_urn, check_err
                    );
                }
                info!(
                    "{} will create session kmsgqueue:{}",
                    self.agent_name, queue_urn
                );
                match msg_queue
                    .create_queue(
                        Some(queue_name),
                        SESSION_QUEUE_APP_ID,
                        self.agent_name.as_str(),
                        queue_cfg,
                    )
                    .await
                {
                    Ok(_) => Ok(()),
                    Err(create_err) => {
                        if Self::queue_resource_already_exists(&create_err) {
                            return Ok(());
                        }
                        match msg_queue.get_queue_stats(queue_urn).await {
                            Ok(_) => Ok(()),
                            Err(recheck_err) => Err(anyhow!(
                                "ensure session queue failed: session={} queue={} check_err={} create_err={} recheck_err={}",
                                session_id,
                                queue_urn,
                                check_err,
                                create_err,
                                recheck_err
                            )),
                        }
                    }
                }
            }
        }
    }

    async fn ensure_session_queue_subscription_exists(
        &self,
        msg_queue: &MsgQueueClient,
        session_id: &str,
        queue_urn: &str,
        sub_id: &str,
    ) -> Result<()> {
        match msg_queue.fetch_messages(sub_id, 1, false).await {
            Ok(_) => Ok(()),
            Err(check_err) => {
                if !Self::queue_resource_not_found(&check_err) {
                    warn!(
                        "check session queue subscription failed, fallback subscribe: session={} queue={} sub_id={} err={}",
                        session_id, queue_urn, sub_id, check_err
                    );
                }
                info!(
                    "agent.persist_entity_prepare: kind=kmsgqueue_subscription session={} queue_urn={} sub_id={}",
                    session_id, queue_urn, sub_id
                );
                match msg_queue
                    .subscribe(
                        queue_urn,
                        self.agent_name.as_str(),
                        SESSION_QUEUE_APP_ID,
                        Some(sub_id.to_string()),
                        SubPosition::Earliest,
                    )
                    .await
                {
                    Ok(_) => Ok(()),
                    Err(subscribe_err) => {
                        if Self::queue_resource_already_exists(&subscribe_err) {
                            return Ok(());
                        }
                        match msg_queue.fetch_messages(sub_id, 1, false).await {
                            Ok(_) => Ok(()),
                            Err(recheck_err) => Err(anyhow!(
                                "ensure session queue subscription failed: session={} queue={} sub_id={} check_err={} subscribe_err={} recheck_err={}",
                                session_id,
                                queue_urn,
                                sub_id,
                                check_err,
                                subscribe_err,
                                recheck_err
                            )),
                        }
                    }
                }
            }
        }
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

        self.ensure_session_queue_exists(
            msg_queue.as_ref(),
            session_id,
            binding.msg_queue_urn.as_str(),
            binding.msg_queue_urn.as_str(),
            queue_cfg.clone(),
        )
        .await?;
        self.ensure_session_queue_exists(
            msg_queue.as_ref(),
            session_id,
            binding.event_queue_urn.as_str(),
            binding.event_queue_urn.as_str(),
            queue_cfg,
        )
        .await?;

        self.ensure_session_queue_subscription_exists(
            msg_queue.as_ref(),
            session_id,
            binding.msg_queue_urn.as_str(),
            binding.msg_sub_id.as_str(),
        )
        .await?;
        self.ensure_session_queue_subscription_exists(
            msg_queue.as_ref(),
            session_id,
            binding.event_queue_urn.as_str(),
            binding.event_sub_id.as_str(),
        )
        .await?;

        self.session_queue_bindings
            .write()
            .await
            .entry(session_id.to_string())
            .or_insert_with(|| binding.clone());
        Ok(Some(binding))
    }

    pub(crate) fn get_session_kmsgqueue_urn(
        agent_name: &str,
        session_id: &str,
        kind: InputQueueKind,
    ) -> String {
        let kind_token = match kind {
            InputQueueKind::Msg => "msg",
            InputQueueKind::Event => "event",
        };
        format!("/{}/sessions/{}/{}", agent_name, session_id, kind_token)
    }

    pub(crate) fn get_session_kmsgqueue_sub_id(
        agent_name: &str,
        session_id: &str,
        kind: InputQueueKind,
    ) -> String {
        let kind_token = match kind {
            InputQueueKind::Msg => "msg_subscription",
            InputQueueKind::Event => "event_subscription",
        };
        format!("/{}/sessions/{}/{}", agent_name, session_id, kind_token)
    }

    async fn enqueue_session_input(
        &self,
        session_id: &str,
        session_input: &SessionInputItem,
        kind: InputQueueKind,
    ) -> Result<()> {
        let Some(msg_queue) = self.deps.msg_queue.as_ref() else {
            return Err(anyhow!("message queue dependency not available"));
        };
        let Some(binding) = self.ensure_session_queue_binding(session_id).await? else {
            return Err(anyhow!("failed to ensure session queue binding"));
        };
        let queue_urn = match kind {
            InputQueueKind::Msg => binding.msg_queue_urn.as_str(),
            InputQueueKind::Event => binding.event_queue_urn.as_str(),
        };
        let kmsg_payload = serde_json::to_vec(&session_input).map_err(|err| {
            anyhow!(
                "serialize session queue payload failed: session={} kind={:?} err={}",
                session_id,
                kind,
                err
            )
        })?;
        let mut message = Message::new(kmsg_payload);
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
        Ok(())
    }

    async fn commit_session_queue_msg_ack(
        &self,
        owner_id: &str,
        session_id: &str,
        last_pulled_msg_index: u64,
    ) -> Result<()> {
        if last_pulled_msg_index == 0 {
            return Ok(());
        }
        let Some(msg_queue) = self.deps.msg_queue.as_ref() else {
            return Err(anyhow!("message queue dependency not available"));
        };
        let sub_id = Self::get_session_kmsgqueue_sub_id(owner_id, session_id, InputQueueKind::Msg);
        debug!(
            "agent.commit_msg_ack: session={} sub_id={} index={}",
            session_id, sub_id, last_pulled_msg_index
        );
        msg_queue
            .commit_ack(sub_id.as_str(), last_pulled_msg_index as u64)
            .await?;
        Ok(())
    }

    async fn set_running_session_to_wait(
        &self,
        session: &Arc<Mutex<crate::agent_session::AgentSession>>,
    ) -> Result<()> {
        let session_id = {
            let mut guard = session.lock().await;
            if guard.state == SessionState::Running {
                guard.state = SessionState::Wait;
            }
            guard.session_id.clone()
        };
        self.session_mgr.save_session(&session_id).await?;
        Ok(())
    }

    async fn set_running_session_to_ready(
        &self,
        session: &Arc<Mutex<crate::agent_session::AgentSession>>,
    ) -> Result<()> {
        let session_id = {
            let mut guard = session.lock().await;
            if guard.state == SessionState::Running {
                guard.state = SessionState::Ready;
            }
            guard.session_id.clone()
        };
        self.session_mgr.save_session(&session_id).await?;
        Ok(())
    }

    async fn execute_actions(
        &self,
        trace: &SessionRuntimeContext,
        actions: &DoActions,
    ) -> DoActionResults {
        let mut out = DoActionResults::default();
        if actions.cmds.is_empty() {
            out.summary = "SUCCESS (0), FAILED (0)".to_string();
            return out;
        }

        let allowed_tool_names = {
            let mut all = self.tools.list_tool_specs();
            all.extend(self.tools.list_action_specs());
            all.sort_by(|a, b| a.name.cmp(&b.name));
            all.dedup_by(|a, b| a.name == b.name);
            let cfg = {
                let guard = self.behavior_cfg_cache.read().await;
                guard.get(trace.behavior.as_str()).cloned()
            };
            let allowed = if let Some(cfg) = cfg {
                cfg.tools.filter_tool_specs(&all)
            } else {
                all
            };
            allowed
                .into_iter()
                .map(|spec| spec.name)
                .collect::<HashSet<_>>()
        };

        let run_all = actions.mode.trim().eq_ignore_ascii_case("all");
        let mut success = 0usize;
        let mut failed = 0usize;
        let mut skipped = 0usize;
        let mut latest_pwd = None::<String>;

        for (idx, action) in actions.cmds.iter().enumerate() {
            let (tool_name, tool_args, exec_id, detail_action, action_cmd_line) = match action {
                DoAction::Exec(command) => {
                    let command = command.trim();
                    if command.is_empty() {
                        failed = failed.saturating_add(1);
                        out.details.insert(
                            format!("#{idx}:exec"),
                            json!({
                                "ok": false,
                                "action": "exec",
                                "error": "empty command is not allowed",
                            }),
                        );
                        if !run_all {
                            skipped = actions.cmds.len().saturating_sub(idx + 1);
                            break;
                        }
                        continue;
                    }
                    (
                        TOOL_EXEC_BASH.to_string(),
                        json!({ "command": command }),
                        format!("#{idx}:`{command}`"),
                        json!({
                            "kind": "exec",
                            "command": command,
                        }),
                        command.to_string(),
                    )
                }
                DoAction::Call(call) => {
                    let normalized_name = normalize_tool_name(&call.call_action_name);
                    if normalized_name.is_empty() {
                        failed = failed.saturating_add(1);
                        out.details.insert(
                            format!("#{idx}:call"),
                            json!({
                                "ok": false,
                                "action": "call_tool",
                                "error": "action name cannot be empty",
                                "raw_action_name": call.call_action_name,
                            }),
                        );
                        if !run_all {
                            skipped = actions.cmds.len().saturating_sub(idx + 1);
                            break;
                        }
                        continue;
                    }

                    if !call.call_params.is_object() {
                        failed = failed.saturating_add(1);
                        out.details.insert(
                            format!("#{idx}:`{normalized_name}`"),
                            json!({
                                "ok": false,
                                "action": "call_tool",
                                "action_name": normalized_name,
                                "error": "action params must be json object",
                                "raw_params": call.call_params,
                            }),
                        );
                        if !run_all {
                            skipped = actions.cmds.len().saturating_sub(idx + 1);
                            break;
                        }
                        continue;
                    }

                    let mut params = call.call_params.clone();
                    if normalized_name == TOOL_EXEC_BASH {
                        if let Some(obj) = params.as_object_mut() {
                            obj.remove("session_id");
                            obj.remove("cwd");
                            obj.remove("pwd");
                        }
                    }
                    (
                        normalized_name.clone(),
                        params.clone(),
                        format!("#{idx}:call `{normalized_name}`"),
                        json!({
                            "kind": "call_tool",
                            "action_name": normalized_name,
                            "params": params,
                        }),
                        compact_action_cmd_line(normalized_name.as_str(), &params),
                    )
                }
            };

            if !allowed_tool_names.contains(tool_name.as_str()) {
                failed = failed.saturating_add(1);
                out.details.insert(
                    exec_id,
                    json!({
                        "ok": false,
                        "tool": tool_name,
                        "action": detail_action,
                        "prompt": format!(
                            "{}  =>  {}",
                            action_cmd_line,
                            format!(
                                "tool `{}` is unavailable or not allowed for behavior `{}`",
                                tool_name, trace.behavior
                            )
                        ),
                        "error": format!(
                            "tool `{}` is unavailable or not allowed for behavior `{}`",
                            tool_name, trace.behavior
                        ),
                    }),
                );
                if !run_all {
                    skipped = actions.cmds.len().saturating_sub(idx + 1);
                    break;
                }
                continue;
            }

            let run_result = self
                .tools
                .call_tool(
                    trace,
                    AiToolCall {
                        name: tool_name.clone(),
                        args: value_to_object_map(tool_args.clone()),
                        call_id: format!(
                            "action-{}-{}-{}",
                            trace.step_idx,
                            now_ms(),
                            self.wakeup_seq.fetch_add(1, Ordering::Relaxed)
                        ),
                    },
                )
                .await;

            match run_result {
                Ok(result) => {
                    success = success.saturating_add(1);
                    if let Some(pwd) = extract_action_result_pwd(&result) {
                        latest_pwd = Some(pwd);
                    }
                    let rendered = result.render_prompt();
                    let result_json = serde_json::to_value(&result)
                        .unwrap_or_else(|_| json!({"error": "serialize result failed"}));
                    out.details.insert(
                        exec_id,
                        json!({
                            "ok": true,
                            "tool": tool_name,
                            "action": detail_action,
                            "prompt": rendered,
                            "result": result_json,
                        }),
                    );
                }
                Err(err) => {
                    failed = failed.saturating_add(1);
                    let prompt_error = compact_action_error_for_prompt(&err);
                    out.details.insert(
                        exec_id,
                        json!({
                            "ok": false,
                            "tool": tool_name,
                            "action": detail_action,
                            "prompt": format!("{}  =>  {}", action_cmd_line, prompt_error),
                            "error": err.to_string(),
                        }),
                    );

                    if !run_all {
                        skipped = actions.cmds.len().saturating_sub(idx + 1);
                        break;
                    }
                }
            }
        }

        if skipped > 0 {
            out.details.insert(
                "__skipped__".to_string(),
                json!({
                    "count": skipped,
                    "reason": "mode=failed_end and previous action failed",
                }),
            );
        }
        out.pwd = latest_pwd;

        out.summary = if skipped > 0 {
            format!("SUCCESS ({success}), FAILED ({failed}), SKIPPED ({skipped})")
        } else {
            format!("SUCCESS ({success}), FAILED ({failed})")
        };
        out
    }

    async fn apply_memory_updates(
        &self,
        trace: &SessionRuntimeContext,
        set_memory: &HashMap<String, String>,
    ) {
        let source = json!({
            "trace_id": trace.trace_id,
            "behavior": trace.behavior,
            "step_idx": trace.step_idx,
            "agent_did": trace.agent_name,
        });

        for (raw_key, content) in set_memory {
            let key = raw_key.trim();
            if key.is_empty() {
                continue;
            }

            if let Err(err) = self
                .memory
                .set_memory(key, content.as_str(), source.clone())
                .await
            {
                warn!(
                    "agent.set_memory failed: did={:?} key={} err={}",
                    self.did, key, err
                );
            }
        }
    }

    async fn send_msg_reply(
        &self,
        trace: SessionRuntimeContext,
        source_tunnel_did: Option<DID>,
        default_audience: Option<&str>,
        reply: Option<&str>,
    ) -> Vec<ReplyHistoryRecord> {
        let Some(content) = reply
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
        else {
            return vec![];
        };

        let audience = default_audience
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_default();
        if audience.is_empty() {
            warn!(
                "agent.reply_missing_default_remote: did={:?} behavior={} content={}",
                self.did,
                trace.behavior,
                compact_text_for_log(content.as_str(), 256),
            );
            return vec![];
        }

        info!(
            "agent.reply: did={:?} behavior={} audience={} format=text content={}",
            self.did,
            trace.behavior,
            audience,
            compact_text_for_log(content.as_str(), 512)
        );

        if let Some(record) = self
            .send_reply_via_msg_center(
                source_tunnel_did,
                audience.as_str(),
                "text",
                content.as_str(),
                Some(trace.session_id.as_str()),
                Some(&trace),
            )
            .await
        {
            vec![record]
        } else {
            vec![]
        }
    }

    async fn send_reply_via_msg_center(
        &self,
        source_tunnel_did: Option<DID>,
        audience: &str,
        _format: &str,
        content: &str,
        session_id: Option<&str>,
        _trace: Option<&SessionRuntimeContext>,
    ) -> Option<ReplyHistoryRecord> {
        let content = content.trim();
        if content.is_empty() {
            return None;
        }

        let Some(msg_center) = self.deps.msg_center.as_ref() else {
            return None;
        };
        let Some(sender_did) = self.parse_owner_did_for_msg_center() else {
            return None;
        };
        //TODO:get target_did by owner's contact list
        let target_did: DID = match DID::from_str(audience) {
            Ok(did) => did,
            Err(_) => {
                warn!(
                    "agent.reply_invalid_audience: did={:?} audience={}",
                    self.did, audience
                );
                return None;
            }
        };

        if target_did == sender_did {
            debug!(
                "agent.reply_skip_self_target: did={:?} target={:?} audience={}",
                self.did, target_did, audience
            );
            return None;
        }

        let mut will_send_msg = MsgObject {
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
        let normalized_session_id = session_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        if will_send_msg
            .thread
            .topic
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
        {
            will_send_msg.thread.topic = normalized_session_id.clone();
        }
        will_send_msg.thread.correlation_id = normalized_session_id.clone();
        if let Some(session_id) = normalized_session_id.as_ref() {
            will_send_msg
                .meta
                .insert("session_id".to_string(), Json::String(session_id.clone()));
            will_send_msg.meta.insert(
                "owner_session_id".to_string(),
                Json::String(session_id.clone()),
            );
        }

        let outbound_for_history = will_send_msg.clone();

        let send_ctx = SendContext {
            contact_mgr_owner: Some(sender_did),
            preferred_tunnel: source_tunnel_did,
            ..Default::default()
        };

        match msg_center
            .post_send(will_send_msg, Some(send_ctx), None)
            .await
        {
            Ok(result) => {
                if !result.ok {
                    warn!(
                        "agent.reply_post_send_rejected: did={:?} target={:?} reason={}",
                        self.did,
                        target_did,
                        result.reason.unwrap_or_else(|| "unknown".to_string())
                    );
                    return None;
                }
                Some(ReplyHistoryRecord {
                    outbound: outbound_for_history,
                    result,
                })
            }
            Err(err) => {
                warn!(
                    "agent.reply_post_send_failed: did={:?} target={:?} err={}",
                    self.did, target_did, err
                );
                None
            }
        }
    }

    async fn handle_reply(
        &self,
        session: Arc<Mutex<AgentSession>>,
        trace: &SessionRuntimeContext,
        reply: Option<&str>,
    ) -> Vec<ReplyHistoryRecord> {
        let (default_audience, source_tunnel_did) = self.resolve_reply_defaults(session).await;
        self.send_msg_reply(
            trace.clone(),
            source_tunnel_did,
            default_audience.as_deref(),
            reply,
        )
        .await
    }

    async fn resolve_reply_defaults(
        &self,
        session: Arc<Mutex<AgentSession>>,
    ) -> (Option<String>, Option<DID>) {
        let default_audience = {
            let guard = session.lock().await;
            guard.default_remote.clone()
        };
        (default_audience, None)
    }

    fn should_append_worklog_for_trace(trace: &SessionRuntimeContext) -> bool {
        AgentSession::is_work_session_id(trace.session_id.as_str())
    }

    fn parse_step_input_msg_record(payload: &[u8]) -> Option<MsgRecord> {
        serde_json::from_slice::<SessionInputItem>(payload)
            .ok()
            .and_then(|item| item.msg)
            .or_else(|| serde_json::from_slice::<MsgRecord>(payload).ok())
    }

    fn normalize_ui_session_id(value: &str) -> Option<String> {
        let session_id = value.trim();
        if session_id.is_empty() {
            return None;
        }
        if !AgentSessionMgr::is_ui_session(session_id) {
            return None;
        }
        Some(session_id.to_string())
    }

    fn creator_ui_session_id_from_meta(meta: &Json) -> Option<String> {
        let obj = meta.as_object()?;
        let raw = obj.get(SESSION_META_CREATOR_UI_SESSION_ID)?;
        match raw {
            Json::String(value) => Self::normalize_ui_session_id(value),
            Json::Array(items) => items.iter().find_map(|item| {
                item.as_str()
                    .and_then(|value| Self::normalize_ui_session_id(value))
            }),
            _ => None,
        }
    }

    fn set_creator_ui_session_id_meta(meta: &mut Json, creator_ui_session_id: &str) {
        let Some(creator_ui_session_id) = Self::normalize_ui_session_id(creator_ui_session_id)
        else {
            return;
        };
        if !meta.is_object() {
            *meta = json!({});
        }
        let Some(obj) = meta.as_object_mut() else {
            return;
        };
        if obj
            .get(SESSION_META_CREATOR_UI_SESSION_ID)
            .and_then(Json::as_str)
            .and_then(Self::normalize_ui_session_id)
            .is_some()
        {
            return;
        }
        obj.insert(
            SESSION_META_CREATOR_UI_SESSION_ID.to_string(),
            Json::String(creator_ui_session_id),
        );
    }

    async fn resolve_creator_ui_session_id(
        &self,
        source_session_id: &str,
        step_msg_inputs: &[SessionInputItem],
    ) -> Option<String> {
        if let Some(session_id) = Self::normalize_ui_session_id(source_session_id) {
            return Some(session_id);
        }

        if let Some(session_id) = step_msg_inputs.iter().find_map(|item| {
            item.msg
                .as_ref()
                .and_then(|msg| msg.ui_session_id.as_deref())
                .and_then(Self::normalize_ui_session_id)
        }) {
            return Some(session_id);
        }

        let Some(source_session) = self.session_mgr.get_session(source_session_id).await else {
            return None;
        };
        let guard = source_session.lock().await;
        Self::creator_ui_session_id_from_meta(&guard.meta)
    }

    fn collect_reply_sync_ui_session_ids(
        session_id: &str,
        creator_ui_session_id: Option<String>,
        step_inputs: &[Vec<u8>],
    ) -> Vec<String> {
        let current_session_id = session_id.trim();
        let mut targets = HashSet::<String>::new();

        if let Some(ui_session_id) = creator_ui_session_id {
            if ui_session_id.as_str() != current_session_id {
                targets.insert(ui_session_id);
            }
        }

        for raw in step_inputs {
            let Some(msg) = Self::parse_step_input_msg_record(raw.as_slice()) else {
                continue;
            };
            let Some(ui_session_id) = msg
                .ui_session_id
                .as_deref()
                .and_then(Self::normalize_ui_session_id)
            else {
                continue;
            };
            if ui_session_id.as_str() != current_session_id {
                targets.insert(ui_session_id);
            }
        }

        let mut out = targets.into_iter().collect::<Vec<_>>();
        out.sort();
        out
    }

    async fn append_worklog_action_record(
        &self,
        session: Arc<Mutex<AgentSession>>,
        trace: &SessionRuntimeContext,
        status: &str,
        payload: Json,
    ) {
        if !Self::should_append_worklog_for_trace(trace) {
            return;
        }

        let mut guard = session.lock().await;
        if let Err(err) = guard
            .append_worklog_with_runtime_context(
                trace,
                "ActionRecord",
                status,
                payload,
                Some(self.environment.local_workspace_manager()),
            )
            .await
        {
            warn!(
                "agent.worklog_append_failed: did={:?} session={} step={} type=ActionRecord err={}",
                self.did, trace.session_id, trace.step_idx, err
            );
        }
    }

    async fn append_incoming_message_worklogs(
        &self,
        session: Arc<Mutex<AgentSession>>,
        trace: &SessionRuntimeContext,
    ) {
        if !Self::should_append_worklog_for_trace(trace) {
            return;
        }

        let step_inputs = {
            let guard = session.lock().await;
            guard.just_readed_input_msg.clone()
        };
        for raw in step_inputs {
            let Some(msg) = Self::parse_step_input_msg_record(raw.as_slice()) else {
                continue;
            };
            let snippet = MsgRecordWithObject {
                record: msg.clone(),
                msg: None,
            }
            .get_msg()
            .await
            .ok()
            .map(|msg_obj| msg_obj.content.content.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(|value| compact_text_for_log(value.as_str(), 220))
            .unwrap_or_else(|| format!("kind={:?}", msg.msg_kind));
            let payload = json!({
                "msg_id": msg.msg_id,
                "record_id": msg.record_id,
                "from": msg.from.to_string(),
                "to": msg.to.to_string(),
                "channel": format!("{:?}", msg.box_kind),
                "snippet": snippet.clone(),
                "content_digest": snippet,
            });
            let mut guard = session.lock().await;
            if let Err(err) = guard
                .append_worklog_with_runtime_context(
                    trace,
                    "GetMessage",
                    "OK",
                    payload,
                    Some(self.environment.local_workspace_manager()),
                )
                .await
            {
                warn!(
                    "agent.worklog_append_failed: did={:?} session={} step={} type=GetMessage err={}",
                    self.did, trace.session_id, trace.step_idx, err
                );
            }
        }
    }

    async fn append_reply_message_worklogs(
        &self,
        session: Arc<Mutex<AgentSession>>,
        trace: &SessionRuntimeContext,
        reply_history: &[ReplyHistoryRecord],
    ) {
        if !Self::should_append_worklog_for_trace(trace) {
            return;
        }
        if reply_history.is_empty() {
            return;
        }

        let reply_to_msg_id = {
            let guard = session.lock().await;
            guard
                .just_readed_input_msg
                .iter()
                .find_map(|raw| Self::parse_step_input_msg_record(raw.as_slice()))
                .map(|msg| msg.msg_id)
        };

        for reply in reply_history {
            let said = reply.outbound.content.content.trim();
            let to = reply
                .outbound
                .to
                .first()
                .map(|did| did.to_string())
                .unwrap_or_default();
            let payload = json!({
                "out_msg_id": reply.result.msg_id,
                "to": to,
                "reply_to": reply_to_msg_id.clone(),
                "content_digest": compact_text_for_log(said, 220),
                "delivery_count": reply.result.deliveries.len(),
            });

            let mut guard = session.lock().await;
            if let Err(err) = guard
                .append_worklog_with_runtime_context(
                    trace,
                    "ReplyMessage",
                    "OK",
                    payload,
                    Some(self.environment.local_workspace_manager()),
                )
                .await
            {
                warn!(
                    "agent.worklog_append_failed: did={:?} session={} step={} type=ReplyMessage err={}",
                    self.did, trace.session_id, trace.step_idx, err
                );
            }
        }
    }

    async fn append_action_record_worklogs(
        &self,
        session: Arc<Mutex<AgentSession>>,
        trace: &SessionRuntimeContext,
        tracking: &LLMTrackingInfo,
        action_results: &DoActionResults,
    ) {
        if !Self::should_append_worklog_for_trace(trace) {
            return;
        }

        for tool_record in &tracking.tool_trace {
            let status = if tool_record.ok { "OK" } else { "FAILED" };
            let payload = json!({
                "action_type": "function",
                "tool_name": tool_record.tool_name,
                "cmd_digest": tool_record.tool_name,
                "call_id": tool_record.call_id,
                "duration_ms": tool_record.duration_ms,
                "result_digest": tool_record.error.clone().unwrap_or_else(|| "ok".to_string()),
            });
            self.append_worklog_action_record(session.clone(), trace, status, payload)
                .await;
        }

        let mut exec_ids = action_results.details.keys().cloned().collect::<Vec<_>>();
        exec_ids.sort();

        for exec_id in exec_ids {
            if exec_id.starts_with("__") {
                continue;
            }
            let Some(detail) = action_results.details.get(exec_id.as_str()) else {
                continue;
            };
            let ok = detail.get("ok").and_then(Json::as_bool).unwrap_or(false);
            let status = if ok { "OK" } else { "FAILED" };

            let action_kind = detail
                .get("action")
                .and_then(|v| v.get("kind"))
                .and_then(Json::as_str)
                .unwrap_or("action");
            let action_type = match action_kind {
                "exec" => "bash",
                "call_tool" => "tool_call",
                other => other,
            };

            let mut cmd_digest = detail
                .get("action")
                .and_then(|v| v.get("command"))
                .and_then(Json::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .unwrap_or_default();
            if cmd_digest.is_empty() {
                if let Some(action_name) = detail
                    .get("action")
                    .and_then(|v| v.get("action_name"))
                    .and_then(Json::as_str)
                {
                    let params = detail
                        .get("action")
                        .and_then(|v| v.get("params"))
                        .cloned()
                        .unwrap_or_else(|| json!({}));
                    cmd_digest = compact_action_cmd_line(action_name, &params);
                }
            }
            if cmd_digest.is_empty() {
                cmd_digest = detail
                    .get("tool")
                    .and_then(Json::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| "action".to_string());
            }

            let mut payload = json!({
                "action_type": action_type,
                "cmd_digest": compact_text_for_log(cmd_digest.as_str(), 220),
                "exec_id": exec_id,
                "result_digest": detail
                    .get("prompt")
                    .and_then(Json::as_str)
                    .map(|v| compact_text_for_log(v, 220))
                    .unwrap_or_default(),
            });
            if let Some(tool_name) = detail.get("tool").and_then(Json::as_str) {
                payload["tool_name"] = Json::String(tool_name.to_string());
            }
            if let Some(exit_code) = detail
                .get("result")
                .and_then(|v| v.get("details"))
                .and_then(|v| v.get("exit_code"))
                .and_then(Json::as_i64)
            {
                payload["exit_code"] = Json::from(exit_code);
            }
            if let Some(cwd) = detail
                .get("result")
                .and_then(|v| v.get("details"))
                .and_then(|v| v.get("cwd"))
                .and_then(Json::as_str)
                .or_else(|| {
                    detail
                        .get("result")
                        .and_then(|v| v.get("details"))
                        .and_then(|v| v.get("pwd"))
                        .and_then(Json::as_str)
                })
            {
                payload["cwd"] = Json::String(cwd.to_string());
            }
            if let Some(error_text) = detail
                .get("error")
                .and_then(Json::as_str)
                .map(|v| compact_text_for_log(v, 220))
            {
                payload["stderr_digest"] = Json::String(error_text);
            }
            self.append_worklog_action_record(session.clone(), trace, status, payload)
                .await;
        }
    }

    async fn append_step_summary_worklog(
        &self,
        session: Arc<Mutex<AgentSession>>,
        trace: &SessionRuntimeContext,
        llm_result: &BehaviorLLMResult,
        action_results: &DoActionResults,
        summary_text: Option<&str>,
    ) {
        if !Self::should_append_worklog_for_trace(trace) {
            return;
        }

        let did_digest = llm_result
            .thinking
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| compact_text_for_log(value, 220))
            .or_else(|| {
                summary_text
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| compact_text_for_log(value, 220))
            })
            .unwrap_or_else(|| "step completed".to_string());
        let result_digest = summary_text
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| compact_text_for_log(value, 220))
            .unwrap_or_else(|| compact_text_for_log(action_results.summary.as_str(), 220));

        let payload = json!({
            "did_digest": did_digest,
            "result_digest": result_digest,
            "next_behavior": llm_result.next_behavior.clone(),
            "action_summary": action_results.summary.as_str(),
        });

        let mut guard = session.lock().await;
        if let Err(err) = guard
            .append_step_summary_with_runtime_context(
                trace,
                payload,
                summary_text,
                Some(self.environment.local_workspace_manager()),
            )
            .await
        {
            warn!(
                "agent.worklog_append_failed: did={:?} session={} step={} type=StepSummary err={}",
                self.did, trace.session_id, trace.step_idx, err
            );
        }
    }

    async fn persist_step_history_records(
        &self,
        session: Arc<Mutex<AgentSession>>,
        session_id: &str,
        reply_history: &[ReplyHistoryRecord],
    ) {
        let (just_readed_input_msg, creator_ui_session_id) = {
            let guard = session.lock().await;
            (
                guard.just_readed_input_msg.clone(),
                Self::creator_ui_session_id_from_meta(&guard.meta),
            )
        };
        let reply_sync_ui_session_ids = Self::collect_reply_sync_ui_session_ids(
            session_id,
            creator_ui_session_id,
            just_readed_input_msg.as_slice(),
        );

        for readed_input_item in &just_readed_input_msg {
            let msg_record =
                serde_json::from_slice::<SessionInputItem>(readed_input_item.as_slice())
                    .ok()
                    .and_then(|item| item.msg)
                    .or_else(|| {
                        serde_json::from_slice::<MsgRecord>(readed_input_item.as_slice()).ok()
                    });
            let Some(msg_record) = msg_record else {
                warn!(
                    "agent.persist_step_history_skip_invalid_input: did={:?} session={} payload_bytes={}",
                    self.did,
                    session_id,
                    readed_input_item.len()
                );
                continue;
            };
            let history_record = MsgRecordWithObject {
                record: msg_record,
                msg: None,
            };
            self.persist_session_msg_history_record(session_id, &history_record)
                .await;
        }

        for reply in reply_history {
            self.persist_post_send_history(session_id, &reply.outbound, &reply.result)
                .await;
            for ui_session_id in &reply_sync_ui_session_ids {
                self.persist_post_send_history(
                    ui_session_id.as_str(),
                    &reply.outbound,
                    &reply.result,
                )
                .await;
            }
        }
    }

    async fn persist_session_msg_history_record(
        &self,
        session_id: &str,
        record: &MsgRecordWithObject,
    ) {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return;
        }
        let session_id = session_id.to_string();
        let msg_obj = if let Some(msg) = record.msg.clone() {
            msg
        } else {
            match record.get_msg().await {
                Ok(msg) => msg,
                Err(err) => {
                    warn!(
                        "agent.msg_history_load_msg_failed: did={:?} session={} record_id={} err={}",
                        self.did, session_id, record.record.record_id, err
                    );
                    return;
                }
            }
        };
        let mut msg_record = record.record.clone();
        if msg_record
            .from_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
        {
            let session_from_name = msg_obj.from.to_raw_host_name();
            msg_record.from_name = AgentSession::resolve_msg_from_name(
                &msg_record.from,
                Some(session_from_name.as_str()),
                Some(self.contact_mgr_owner_did()),
            )
            .await;
        }

        let session_dir = self.session_mgr.sessions_root().join(session_id.as_str());
        let session_dir_str = session_dir.to_string_lossy().to_string();

        if let Err(err) =
            AgentSession::append_msg_record(session_dir_str.as_str(), msg_record, msg_obj).await
        {
            warn!(
                "agent.msg_history_append_failed: did={:?} session={} session_dir={} record_id={} err={}",
                self.did,
                session_id,
                session_dir.display(),
                record.record.record_id,
                err
            );
        }
    }

    //TODO: 逻辑需要优化，有点复杂
    async fn persist_post_send_history(
        &self,
        session_id: &str,
        outbound: &MsgObject,
        result: &PostSendResult,
    ) {
        let Some(msg_center) = self.deps.msg_center.as_ref() else {
            return;
        };
        if result.deliveries.is_empty() {
            return;
        }

        for delivery in &result.deliveries {
            let mut record_with_obj = None::<MsgRecordWithObject>;
            for attempt in 0..3 {
                match msg_center
                    .get_record(delivery.record_id.clone(), Some(true))
                    .await
                {
                    Ok(Some(record)) => {
                        record_with_obj = Some(record);
                        break;
                    }
                    Ok(None) => {
                        if attempt < 2 {
                            sleep(Duration::from_millis(40)).await;
                        } else {
                            warn!(
                                "agent.reply_history_record_missing: did={:?} session={} record_id={} msg_id={}",
                                self.did, session_id, delivery.record_id, result.msg_id
                            );
                        }
                    }
                    Err(err) => {
                        warn!(
                            "agent.reply_history_record_fetch_failed: did={:?} session={} record_id={} msg_id={} err={}",
                            self.did, session_id, delivery.record_id, result.msg_id, err
                        );
                        break;
                    }
                }
            }

            if let Some(record) = record_with_obj {
                self.persist_session_msg_history_record(session_id, &record)
                    .await;
                continue;
            }

            let synthetic = MsgRecordWithObject {
                record: buckyos_api::MsgRecord {
                    record_id: delivery.record_id.clone(),
                    box_kind: BoxKind::Outbox,
                    msg_id: result.msg_id.clone(),
                    msg_kind: outbound.kind.clone(),
                    state: MsgState::Sent,
                    from: outbound.from.clone(),
                    from_name: None,
                    to: delivery
                        .target_did
                        .as_ref()
                        .cloned()
                        .or_else(|| outbound.to.first().cloned())
                        .unwrap_or_else(|| outbound.from.clone()),
                    created_at_ms: outbound.created_at_ms,
                    updated_at_ms: now_ms(),
                    route: None,
                    delivery: None,
                    ui_session_id: Some(session_id.to_string()),
                    sort_key: now_ms(),
                    tags: vec![],
                },
                msg: Some(outbound.clone()),
            };
            self.persist_session_msg_history_record(session_id, &synthetic)
                .await;
        }
    }

    //behavior_name is full name like do:todo=T01:param2=abc
    async fn load_behavior_config(&self, behavior_name: &str) -> Result<BehaviorConfig> {
        let behavior_name = behavior_name.trim();
        if behavior_name.is_empty() {
            return Err(anyhow!("behavior name cannot be empty"));
        }

        let lookup_names = Self::build_behavior_lookup_names(behavior_name);

        let mut last_err: Option<anyhow::Error> = None;
        for lookup_name in &lookup_names {
            match BehaviorConfig::load_from_roots(&self.behavior_roots, lookup_name).await {
                Ok(loaded) => {
                    let mut cache = self.behavior_cfg_cache.write().await;
                    for alias in &lookup_names {
                        cache.insert(alias.clone(), loaded.clone());
                    }
                    return Ok(loaded);
                }
                Err(err) => {
                    last_err = Some(anyhow!(
                        "lookup `{lookup_name}` failed while loading behavior `{behavior_name}`: {err}"
                    ));
                }
            }
        }

        let looked_up = lookup_names.join(", ");
        Err(last_err.unwrap_or_else(|| {
            anyhow!(
                "load behavior `{behavior_name}` failed: no matching behavior config found (tried: {looked_up})"
            )
        }))
    }

    fn build_behavior_lookup_names(behavior_name: &str) -> Vec<String> {
        let trimmed = behavior_name.trim();
        if trimmed.is_empty() {
            return Vec::new();
        }

        let mut out = Vec::new();
        out.push(trimmed.to_string());

        //类似 DO:todo=t01:p2=3 这样的名字，有
        //  DO:todo=t01:p2=3
        //  DO:todo=t01
        //  DO
        //共计3个lookup name
        let base = trimmed
            .split_once(':')
            .map(|(name, _)| name.trim())
            .unwrap_or(trimmed);
        if !base.is_empty() && !out.iter().any(|name| name == base) {
            out.push(base.to_string());
        }

        let lower = base.to_ascii_lowercase();
        if !lower.is_empty() && !out.iter().any(|name| name == &lower) {
            out.push(lower);
        }

        out
    }

    pub fn did(&self) -> String {
        self.did.to_string()
    }

    pub fn agent_env_root(&self) -> &Path {
        &self.agent_env_root
    }

    fn contact_mgr_owner_did(&self) -> &DID {
        self.contact_mgr_owner_did
            .as_ref()
            .unwrap_or(&self.msg_owner_did)
    }

    fn parse_owner_did_for_msg_center(&self) -> Option<DID> {
        Some(self.msg_owner_did.clone())
    }
}

fn apply_session_behavior_transition(
    session: &mut crate::agent_session::AgentSession,
    default_behavior: &str,
    step_limit: u32,
    faild_back_behavior: Option<&str>,
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
                && session.state != SessionState::End
            {
                session.state = SessionState::Wait;
            }
            keep_running = false;
        } else if starts_with_ignore_ascii_case(next_behavior, "WAIT_FOR_MSG") {
            session.state = SessionState::WaitForMsg;
            keep_running = false;
        } else if next_behavior.eq_ignore_ascii_case("END") {
            //不切换behavior,但是当前behavior loop结束了
            session.last_step_summary = None;
            session.state = SessionState::End;
            keep_running = false;
        } else {
            let previous_behavior = session.current_behavior.clone();
            behavior_switched = !session.current_behavior.eq_ignore_ascii_case(next_behavior);
            //session.state = SessionState::Running;
            if behavior_switched {
                session.current_behavior = next_behavior.to_string();
                session.step_index = 0;
                // Keep last_step_summary: session still running, next behavior may use it for continuity
                info!(
                    "agent.session_behavior_switch: session={} from={} to={} reason=next_behavior",
                    session.session_id, previous_behavior, session.current_behavior
                );
                keep_running = true;
            } else {
                keep_running = advance_session_step_or_apply_limit_fallback(
                    session,
                    default_behavior,
                    step_limit,
                    faild_back_behavior,
                    &mut behavior_switched,
                );
            }
        }
    } else {
        keep_running = advance_session_step_or_apply_limit_fallback(
            session,
            default_behavior,
            step_limit,
            faild_back_behavior,
            &mut behavior_switched,
        );
    }

    StepTransition {
        keep_running,
        behavior_switched,
    }
}

fn advance_session_step_or_apply_limit_fallback(
    session: &mut crate::agent_session::AgentSession,
    default_behavior: &str,
    step_limit: u32,
    faild_back_behavior: Option<&str>,
    behavior_switched: &mut bool,
) -> bool {
    if session.state != SessionState::Running {
        return false;
    }

    session.step_index = session.step_index.saturating_add(1);
    if step_limit > 0 && session.step_index >= step_limit {
        let fallback_behavior = faild_back_behavior
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(default_behavior);
        let previous_behavior = session.current_behavior.clone();
        *behavior_switched = !session
            .current_behavior
            .eq_ignore_ascii_case(fallback_behavior);
        session.current_behavior = fallback_behavior.to_string();
        session.step_index = 0;

        if *behavior_switched {
            info!(
                "agent.session_behavior_switch: session={} from={} to={} reason=step_limit_fallback step_limit={}",
                session.session_id, previous_behavior, session.current_behavior, step_limit
            );
            true
        } else {
            session.state = SessionState::Wait;
            false
        }
    } else {
        true
    }
}

fn starts_with_ignore_ascii_case(value: &str, prefix: &str) -> bool {
    value
        .get(..prefix.len())
        .map(|head| head.eq_ignore_ascii_case(prefix))
        .unwrap_or(false)
}

fn render_action_results_for_prompt(results: &DoActionResults) -> String {
    let mut keys = results.details.keys().cloned().collect::<Vec<_>>();
    keys.sort();

    let mut lines = vec![format!("ActionResults: {}", results.summary)];
    if let Some(pwd) = results
        .pwd
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        lines.push(format!("pwd: {pwd}"));
    }
    for key in keys {
        let detail = results.details.get(&key).cloned().unwrap_or(Json::Null);
        if let Some(prompt) = detail
            .get("prompt")
            .and_then(Json::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            let mut prompt_lines = prompt.lines();
            if let Some(first) = prompt_lines.next() {
                lines.push(format!("- {}", first));
                for line in prompt_lines {
                    lines.push(line.to_string());
                }
            } else {
                lines.push("-".to_string());
            }
            continue;
        }
        if let Some(error) = detail
            .get("error")
            .and_then(Json::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            lines.push(format!("- {} ERROR: {}", key, error));
            continue;
        }
        lines.push(format!(
            "- {} {}",
            key,
            serde_json::to_string(&detail).unwrap_or_else(|_| "{}".to_string())
        ));
    }
    lines.join("\n")
}

async fn build_step_summary(
    trace: &SessionRuntimeContext,
    behavior_cfg: &BehaviorConfig,
    llm_result: &BehaviorLLMResult,
    _tracking: &LLMTrackingInfo,
    action_results: &DoActionResults,
    session: Arc<Mutex<AgentSession>>,
) -> Option<String> {
    let mut env_context = HashMap::<String, Json>::new();
    env_context.insert("step_index".to_string(), Json::from(trace.step_idx));
    env_context.insert(
        "step_limit".to_string(),
        Json::from(behavior_cfg.step_limit),
    );

    if let Ok(mut llm_result_json) = serde_json::to_value(llm_result) {
        llm_result_json["action_results"] =
            Json::String(render_action_results_for_prompt(action_results));
        env_context.insert("llm_result".to_string(), llm_result_json);
    }

    if let Ok(trace_json) = serde_json::to_value(trace) {
        env_context.insert("trace".to_string(), trace_json);
    }

    AgentEnvironment::render_prompt(&behavior_cfg.step_summary, &env_context, session)
        .await
        .ok()
        .map(|render_result| render_result.rendered)
}

fn merged_actions_from_llm_result(llm_result: &BehaviorLLMResult) -> DoActions {
    let mut merged = DoActions {
        mode: llm_result.actions.mode.clone(),
        cmds: Vec::new(),
    };
    for command in &llm_result.shell_commands {
        let command = command.trim();
        if command.is_empty() {
            continue;
        }
        merged.cmds.push(DoAction::Exec(command.to_string()));
    }
    merged.cmds.extend(llm_result.actions.cmds.clone());
    merged
}

fn compact_action_cmd_line(tool_name: &str, args: &Json) -> String {
    let Some(map) = args.as_object() else {
        return tool_name.to_string();
    };

    if let Some(command) = map
        .get("command")
        .and_then(Json::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        return command.to_string();
    }

    let mut parts = Vec::<String>::new();
    for key in [
        "path",
        "range",
        "first_chunk",
        "name",
        "workspace",
        "workspace_id",
        "mode",
    ] {
        let Some(value) = map.get(key) else {
            continue;
        };
        if let Some(raw) = value.as_str() {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                continue;
            }
            if key == "path" || key == "range" {
                parts.push(trimmed.to_string());
            } else {
                parts.push(format!("{key}={trimmed}"));
            }
            continue;
        }
        if !value.is_null() {
            parts.push(format!(
                "{key}={}",
                serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
            ));
        }
    }

    if parts.is_empty() {
        return tool_name.to_string();
    }
    format!("{tool_name} {}", parts.join(" "))
}

fn compact_action_error_for_prompt(err: &crate::agent_tool::AgentToolError) -> String {
    match err {
        crate::agent_tool::AgentToolError::ExecFailed(msg)
        | crate::agent_tool::AgentToolError::InvalidArgs(msg)
        | crate::agent_tool::AgentToolError::NotFound(msg)
        | crate::agent_tool::AgentToolError::AlreadyExists(msg) => msg.trim().to_string(),
        crate::agent_tool::AgentToolError::Timeout => "timeout".to_string(),
    }
}

fn extract_action_result_pwd(result: &crate::agent_tool::AgentToolResult) -> Option<String> {
    result
        .details
        .get("pwd")
        .or_else(|| result.details.get("cwd"))
        .and_then(Json::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
}

async fn resolve_agent_env_root(agent_root: &Path) -> Result<PathBuf> {
    let root = agent_root.to_path_buf();
    info!(
        "agent.persist_entity_prepare: kind=agent_env_root path={}",
        root.display()
    );
    fs::create_dir_all(&root).await.map_err(|err| {
        anyhow!(
            "create agent env root failed: path={} err={}",
            root.display(),
            err
        )
    })?;
    Ok(root)
}

async fn resolve_default_behavior_name(behavior_roots: &[PathBuf]) -> Option<String> {
    for candidate in [AGENT_BEHAVIOR_ROUTER_RESOLVE] {
        if behavior_exists(behavior_roots, candidate).await {
            return Some(candidate.to_string());
        }
    }

    for behavior_root in behavior_roots {
        let mut read_dir = match fs::read_dir(behavior_root).await {
            Ok(read_dir) => read_dir,
            Err(_) => continue,
        };
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
            } else if path.is_dir() {
                if let Some(name) = path.file_name().and_then(|v| v.to_str()) {
                    let trimmed = name.trim();
                    if !trimmed.is_empty() {
                        return Some(trimmed.to_string());
                    }
                }
            }
        }
    }
    None
}

async fn resolve_default_worker_behavior_name(
    behavior_roots: &[PathBuf],
    default_behavior: &str,
) -> String {
    for candidate in ["plan", "do", AGENT_BEHAVIOR_ROUTER_RESOLVE] {
        if behavior_exists(behavior_roots, candidate).await {
            return candidate.to_string();
        }
    }

    default_behavior.to_string()
}

async fn behavior_exists(behavior_roots: &[PathBuf], behavior_name: &str) -> bool {
    for behavior_root in behavior_roots {
        let dir_path = behavior_root.join(behavior_name);
        if fs::metadata(&dir_path)
            .await
            .map(|meta| meta.is_dir())
            .unwrap_or(false)
        {
            return true;
        }

        for ext in ["yaml", "yml", "json"] {
            let path = behavior_root.join(format!("{behavior_name}.{ext}"));
            if fs::metadata(&path)
                .await
                .map(|meta| meta.is_file())
                .unwrap_or(false)
            {
                return true;
            }
        }
    }
    false
}

async fn load_agent_did(
    cfg: &AIAgentConfig,
    agent_root: &Path,
    package_root: Option<&Path>,
) -> Result<String> {
    if let Some(did) = cfg
        .agent_did
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        return Ok(did.to_string());
    }

    for root in [Some(agent_root), package_root].into_iter().flatten() {
        for name in AGENT_DOC_CANDIDATES {
            let path = root.join(name);
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
    }

    let instance_name = cfg.agent_instance_id.trim();
    if !instance_name.is_empty() {
        return Ok(format!("did:opendan:{instance_name}"));
    }

    let dir_name = agent_root
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("agent");
    Ok(format!("did:opendan:{dir_name}"))
}

async fn build_behavior_roots(
    agent_root: &Path,
    package_root: Option<&Path>,
    dir_name: &str,
) -> Result<Vec<PathBuf>> {
    let mut roots = Vec::new();
    let env_behaviors_dir = agent_root.join(dir_name);
    if is_existing_dir(&env_behaviors_dir).await {
        roots.push(env_behaviors_dir);
    }

    if let Some(package_root) = package_root {
        let package_behaviors_dir = package_root.join(dir_name);
        if is_existing_dir(&package_behaviors_dir).await {
            roots.push(package_behaviors_dir);
        }
    }

    if roots.is_empty() {
        let fallback = agent_root.join(dir_name);
        info!(
            "agent.persist_entity_prepare: kind=behaviors_dir path={}",
            fallback.display()
        );
        fs::create_dir_all(&fallback).await.map_err(|err| {
            anyhow!(
                "create behaviors dir failed: path={} err={}",
                fallback.display(),
                err
            )
        })?;
        roots.push(fallback);
    }

    Ok(roots)
}

async fn load_overlay_text_resource(
    agent_root: &Path,
    package_root: Option<&Path>,
    candidate_rel_paths: &[&str],
    default_text: &str,
) -> Result<(String, ResourceOverlayStatus)> {
    let mut status = ResourceOverlayStatus {
        selected_path: None,
        env_path: None,
        package_path: None,
    };
    let mut selected_content = None::<String>;

    for rel_path in candidate_rel_paths {
        let path = agent_root.join(rel_path);
        if let Some(content) = read_text_if_exists(&path).await? {
            status.env_path = Some(path);
            selected_content = Some(content);
            break;
        }
    }

    if let Some(package_root) = package_root {
        for rel_path in candidate_rel_paths {
            let path = package_root.join(rel_path);
            if let Some(content) = read_text_if_exists(&path).await? {
                status.package_path = Some(path);
                if selected_content.is_none() {
                    selected_content = Some(content);
                }
                break;
            }
        }
    }

    status.selected_path = status
        .env_path
        .clone()
        .or_else(|| status.package_path.clone());

    Ok((
        selected_content.unwrap_or_else(|| default_text.to_string()),
        status,
    ))
}

fn log_overlay_status(resource: &str, instance_id: &str, status: &ResourceOverlayStatus) {
    info!(
        "agent.loader.resource: instance={} resource={} selected={} env_override={} package_default={}",
        instance_id,
        resource,
        status
            .selected_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<builtin-default>".to_string()),
        status.env_path.is_some(),
        status.package_path.is_some()
    );
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use serde_json::json;
    use tempfile::tempdir;
    use tokio::fs;

    use crate::test_utils::MockTaskMgrHandler;

    fn make_event(eventid: &str) -> Event {
        Event {
            eventid: eventid.to_string(),
            source_node: "test-node".to_string(),
            source_pid: 1,
            ingress_node: None,
            timestamp: 0,
            data: json!({}),
        }
    }

    #[test]
    fn msg_center_event_box_kind_parses_known_box() {
        let event = make_event("/msg_center/agent.example/box/in/changed");
        assert_eq!(
            AIAgent::msg_center_event_box_kind(&event),
            Some(BoxKind::Inbox)
        );
    }

    #[test]
    fn msg_center_event_box_kind_accepts_extended_suffix() {
        let event = make_event("/msg_center/agent.example/box/request/changed/v2");
        assert_eq!(
            AIAgent::msg_center_event_box_kind(&event),
            Some(BoxKind::RequestBox)
        );
    }

    #[test]
    fn msg_center_event_box_kind_accepts_legacy_box_name() {
        let event = make_event("/msg_center/agent.example/box/INBOX/changed");
        assert_eq!(
            AIAgent::msg_center_event_box_kind(&event),
            Some(BoxKind::Inbox)
        );
    }

    #[test]
    fn msg_center_event_box_kind_accepts_legacy_path_without_box_segment() {
        let event = make_event("/msg_center/agent.example/inbox/changed");
        assert_eq!(
            AIAgent::msg_center_event_box_kind(&event),
            Some(BoxKind::Inbox)
        );
    }

    #[test]
    fn build_msg_center_event_patterns_include_legacy_aliases() {
        let owner = DID::new("bns", "Agent.Example");
        let patterns = AIAgent::build_msg_center_event_patterns(&owner);

        assert!(patterns.contains(&"/msg_center/Agent.Example.bns.did/box/in/**".to_string()));
        assert!(patterns.contains(&"/msg_center/Agent.Example.bns.did/box/INBOX/**".to_string()));
        assert!(patterns.contains(&"/msg_center/Agent.Example.bns.did/inbox/**".to_string()));
        assert!(patterns.contains(&"/msg_center/agent.example.bns.did/box/in/**".to_string()));
    }

    #[test]
    fn collect_event_pull_targets_falls_back_for_unknown_msg_center_event() {
        let event = make_event("/msg_center/agent.example/box/unknown/changed");
        let mut msg_pull_boxes = Vec::new();
        let mut pulled_events = Vec::new();

        AIAgent::collect_event_pull_targets(event, &mut msg_pull_boxes, &mut pulled_events);

        assert!(msg_pull_boxes.contains(&BoxKind::Inbox));
        assert!(msg_pull_boxes.contains(&BoxKind::GroupInbox));
        assert!(msg_pull_boxes.contains(&BoxKind::RequestBox));
        assert!(pulled_events.is_empty());
    }

    #[test]
    fn merged_actions_executes_shell_commands_before_actions() {
        let mut llm_result = BehaviorLLMResult::default();
        llm_result.actions.mode = "all".to_string();
        llm_result
            .actions
            .cmds
            .push(DoAction::Exec("echo from-actions".to_string()));
        llm_result.shell_commands = vec![
            "echo from-shell-1".to_string(),
            "  ".to_string(),
            "echo from-shell-2".to_string(),
        ];

        let merged = merged_actions_from_llm_result(&llm_result);
        assert_eq!(merged.mode, "all");
        assert_eq!(merged.cmds.len(), 3);
        assert_eq!(
            merged.cmds,
            vec![
                DoAction::Exec("echo from-shell-1".to_string()),
                DoAction::Exec("echo from-shell-2".to_string()),
                DoAction::Exec("echo from-actions".to_string()),
            ]
        );
    }

    #[test]
    fn apply_behavior_transition_is_case_insensitive_for_same_behavior() {
        let mut session = AgentSession::new("work-1", "did:test:agent", Some("plan"));
        session.state = SessionState::Running;

        let transition =
            apply_session_behavior_transition(&mut session, "plan", 8, None, Some("PLAN"));

        assert!(!transition.behavior_switched);
        assert!(transition.keep_running);
        assert_eq!(session.current_behavior, "plan");
        assert_eq!(session.step_index, 1);
    }

    #[test]
    fn apply_behavior_transition_same_behavior_honors_step_limit_fallback() {
        let mut session = AgentSession::new("work-1", "did:test:agent", Some("plan"));
        session.state = SessionState::Running;
        session.step_index = 1;

        let transition =
            apply_session_behavior_transition(&mut session, "plan", 2, Some("PLAN"), Some("plan"));

        assert!(!transition.behavior_switched);
        assert!(!transition.keep_running);
        assert_eq!(session.current_behavior, "PLAN");
        assert_eq!(session.step_index, 0);
        assert_eq!(session.state, SessionState::Wait);
    }

    #[test]
    fn apply_behavior_transition_parses_wait_for_msg_case_insensitive() {
        let mut session = AgentSession::new("work-1", "did:test:agent", Some("plan"));
        session.state = SessionState::Running;

        let transition =
            apply_session_behavior_transition(&mut session, "plan", 8, None, Some("wait_for_msg"));

        assert!(!transition.behavior_switched);
        assert!(!transition.keep_running);
        assert_eq!(session.state, SessionState::WaitForMsg);
    }

    #[test]
    fn apply_behavior_transition_step_limit_fallback_is_case_insensitive() {
        let mut session = AgentSession::new("work-1", "did:test:agent", Some("plan"));
        session.state = SessionState::Running;
        session.step_index = 1;

        let transition =
            apply_session_behavior_transition(&mut session, "plan", 2, Some("PLAN"), None);

        assert!(!transition.behavior_switched);
        assert!(!transition.keep_running);
        assert_eq!(session.current_behavior, "PLAN");
        assert_eq!(session.step_index, 0);
        assert_eq!(session.state, SessionState::Wait);
    }

    fn build_step_input_payload(record_id: &str, ui_session_id: Option<&str>) -> Vec<u8> {
        let mut msg_record = json!({
            "record_id": record_id,
            "box_kind": "INBOX",
            "msg_id": "sha256:11111111111111111111111111111111",
            "msg_kind": "chat",
            "state": "UNREAD",
            "from": "did:web:alice.example.com",
            "to": "did:web:agent.example.com",
            "created_at_ms": 1000,
            "updated_at_ms": 1000,
            "sort_key": 1000,
            "tags": []
        });
        if let Some(value) = ui_session_id {
            msg_record
                .as_object_mut()
                .expect("msg record object")
                .insert("ui_session_id".to_string(), Json::String(value.to_string()));
        }
        serde_json::to_vec(&json!({
            "msg": msg_record,
            "event_id": null
        }))
        .expect("serialize step input payload")
    }

    #[test]
    fn collect_reply_sync_ui_session_ids_prefers_ui_sessions_only() {
        let step_inputs = vec![
            build_step_input_payload("r1", Some("ui-owner")),
            build_step_input_payload("r2", Some("ui-extra")),
            build_step_input_payload("r3", Some("work-should-skip")),
            b"{\"msg\":\"invalid\"}".to_vec(),
        ];

        let targets = AIAgent::collect_reply_sync_ui_session_ids(
            "work-abc",
            Some("ui-owner".to_string()),
            step_inputs.as_slice(),
        );
        assert_eq!(
            targets,
            vec!["ui-extra".to_string(), "ui-owner".to_string()]
        );
    }

    #[test]
    fn set_creator_ui_session_id_meta_keeps_existing_value() {
        let mut meta = json!({
            "creator_ui_session_id": "ui-first"
        });
        AIAgent::set_creator_ui_session_id_meta(&mut meta, "ui-second");
        assert_eq!(
            AIAgent::creator_ui_session_id_from_meta(&meta),
            Some("ui-first".to_string())
        );

        let mut empty_meta = json!({});
        AIAgent::set_creator_ui_session_id_meta(&mut empty_meta, "ui-owner");
        assert_eq!(
            AIAgent::creator_ui_session_id_from_meta(&empty_meta),
            Some("ui-owner".to_string())
        );
    }

    #[tokio::test]
    async fn build_step_summary_uses_current_behavior_step_limit() {
        let session = Arc::new(tokio::sync::Mutex::new(AgentSession::new(
            "s1",
            "did:test:agent",
            Some("resolve_router"),
        )));
        session.lock().await.step_index = 99;

        let trace = SessionRuntimeContext {
            trace_id: "trace-1".to_string(),
            agent_name: "agent-test".to_string(),
            behavior: "plan".to_string(),
            step_idx: 2,
            wakeup_id: "wakeup-1".to_string(),
            session_id: "s1".to_string(),
        };
        let mut behavior_cfg = BehaviorConfig::default();
        behavior_cfg.step_limit = 16;
        behavior_cfg.step_summary =
            "idx={{step_index}} limit={{step_limit}} thinking={{llm_result.thinking}}".to_string();

        let llm_result = BehaviorLLMResult {
            thinking: Some("break down tasks".to_string()),
            ..Default::default()
        };
        let tracking = crate::behavior::LLMTrackingInfo {
            token_usage: crate::behavior::TokenUsage::default(),
            track: crate::behavior::TrackInfo {
                trace_id: "trace-1".to_string(),
                model: "test-model".to_string(),
                provider: "test-provider".to_string(),
                latency_ms: 0,
                llm_task_ids: vec![],
                errors: vec![],
            },
            tool_trace: vec![],
            raw_output: crate::behavior::LLMOutput::Text(String::new()),
        };

        let rendered = build_step_summary(
            &trace,
            &behavior_cfg,
            &llm_result,
            &tracking,
            &DoActionResults::default(),
            session,
        )
        .await;

        assert_eq!(
            rendered.as_deref(),
            Some("idx=2 limit=16 thinking=break down tasks")
        );
    }

    #[tokio::test]
    async fn build_step_summary_renders_action_results_prompt_preview() {
        let session = Arc::new(tokio::sync::Mutex::new(AgentSession::new(
            "s1",
            "did:test:agent",
            Some("resolve_router"),
        )));

        let trace = SessionRuntimeContext {
            trace_id: "trace-1".to_string(),
            agent_name: "agent-test".to_string(),
            behavior: "plan".to_string(),
            step_idx: 2,
            wakeup_id: "wakeup-1".to_string(),
            session_id: "s1".to_string(),
        };
        let mut behavior_cfg = BehaviorConfig::default();
        behavior_cfg.step_summary = "### Run Results\n{{llm_result.action_results}}".to_string();

        let llm_result = BehaviorLLMResult::default();
        let tracking = crate::behavior::LLMTrackingInfo {
            token_usage: crate::behavior::TokenUsage::default(),
            track: crate::behavior::TrackInfo {
                trace_id: "trace-1".to_string(),
                model: "test-model".to_string(),
                provider: "test-provider".to_string(),
                latency_ms: 0,
                llm_task_ids: vec![],
                errors: vec![],
            },
            tool_trace: vec![],
            raw_output: crate::behavior::LLMOutput::Text(String::new()),
        };
        let mut action_results = DoActionResults {
            summary: "SUCCESS (1), FAILED (0)".to_string(),
            pwd: Some("/tmp/demo".to_string()),
            details: HashMap::new(),
        };
        action_results.details.insert(
            "#0".to_string(),
            json!({
                "prompt": "read_file demo.txt range=1-2 => read 2 lines\nline-1\nline-2"
            }),
        );

        let rendered = build_step_summary(
            &trace,
            &behavior_cfg,
            &llm_result,
            &tracking,
            &action_results,
            session,
        )
        .await
        .expect("rendered summary");

        assert!(rendered.contains("ActionResults: SUCCESS (1), FAILED (0)"));
        assert!(rendered.contains("pwd: /tmp/demo"));
        assert!(rendered.contains("- read_file demo.txt range=1-2 => read 2 lines"));
        assert!(!rendered.contains("\"summary\""));
        assert!(!rendered.contains("\"details\""));
    }

    #[tokio::test]
    async fn print_action_results_prompt_preview() {
        let temp = tempdir().expect("create tempdir");
        let agent_root = temp.path().join("agent");
        fs::create_dir_all(agent_root.join("behaviors"))
            .await
            .expect("create behaviors dir");
        fs::write(
            agent_root.join("agent.json.doc"),
            r#"{"id":"did:opendan:test-agent"}"#,
        )
        .await
        .expect("write agent doc");
        fs::write(
            agent_root.join("behaviors/resolve_router.yaml"),
            r#"
process_rule: "test behavior for action rendering"
"#,
        )
        .await
        .expect("write behavior config");

        let taskmgr = Arc::new(buckyos_api::TaskManagerClient::new_in_process(Box::new(
            MockTaskMgrHandler {
                counter: Mutex::new(0),
                tasks: Arc::new(Mutex::new(HashMap::new())),
            },
        )));
        let agent = AIAgent::load(
            AIAgentConfig::new(agent_root.clone()),
            AIAgentDeps {
                taskmgr,
                msg_center: None,
                msg_queue: None,
            },
        )
        .await
        .expect("load agent");
        let runtime = crate::ai_runtime::AiRuntime::new(crate::ai_runtime::AiRuntimeConfig::new(
            agent_root.join("runtime_agents"),
        ))
        .await
        .expect("create runtime");
        runtime
            .register_tools(&agent.tools)
            .await
            .expect("register runtime tools");

        let llm_tools = agent.tools.list_tool_specs();
        let bash_tools = agent.tools.list_bash_cmd_specs();
        let action_tools = agent.tools.list_action_tool_specs();
        let mut all_tool_names = std::collections::HashSet::<String>::new();
        all_tool_names.extend(llm_tools.iter().map(|s| s.name.clone()));
        all_tool_names.extend(bash_tools.iter().map(|s| s.name.clone()));
        all_tool_names.extend(action_tools.iter().map(|s| s.name.clone()));

        println!(
            "registered tools => llm={} bash={} action={} all={:?}",
            llm_tools.len(),
            bash_tools.len(),
            action_tools.len(),
            {
                let mut names = all_tool_names.iter().cloned().collect::<Vec<_>>();
                names.sort();
                names
            }
        );

        for expected in [
            "exec",
            "edit_file",
            "write_file",
            "read_file",
            "todo",
            "create_workspace",
            "bind_workspace",
            "load_memory",
            "get_session",
            "create_sub_agent",
            "bind_external_workspace",
            "list_external_workspaces",
        ] {
            assert!(
                all_tool_names.contains(expected),
                "missing expected tool in catalog: {expected}"
            );
        }

        let session = agent
            .session_mgr
            .ensure_session(
                "session-test",
                Some("Session Test".to_string()),
                None,
                Some("resolve_router"),
            )
            .await
            .expect("ensure session");
        {
            let mut guard = session.lock().await;
            guard.pwd = agent.agent_env_root.clone();
        }
        agent
            .session_mgr
            .save_session("session-test")
            .await
            .expect("save session");

        fs::write(
            agent.agent_env_root.join("prompt_preview.txt"),
            "line-1\nline-2\nline-3\n",
        )
        .await
        .expect("write preview file");
        fs::write(
            agent.agent_env_root.join("edit_preview.txt"),
            "line-alpha\nline-beta\nline-gamma\n",
        )
        .await
        .expect("write edit preview file");

        let mut large_content = String::new();
        for i in 0..20_000 {
            large_content.push_str(format!("large-line-{i:05}\n").as_str());
        }
        let large_bytes = large_content.len();
        fs::write(
            agent.agent_env_root.join("large_preview.txt"),
            large_content,
        )
        .await
        .expect("write large preview file");

        let tree_root = agent.agent_env_root.join("tree_preview");
        for i in 0..8 {
            let branch = tree_root.join(format!("dir-{i:02}"));
            fs::create_dir_all(branch.join("nested"))
                .await
                .expect("create tree branch");
            for j in 0..8 {
                fs::write(
                    branch.join(format!("file-{j:02}.txt")),
                    format!("tree-file-{i}-{j}\n"),
                )
                .await
                .expect("write tree file");
            }
            fs::write(
                branch.join("nested").join("leaf.txt"),
                format!("tree-leaf-{i}\n"),
            )
            .await
            .expect("write nested tree file");
        }

        let curl_source = "curl-source-line\n".repeat(1024);
        fs::write(agent.agent_env_root.join("curl_source.txt"), &curl_source)
            .await
            .expect("write curl source file");

        let actions: DoActions = serde_json::from_value(json!({
            "mode": "all",
            "cmds": [
                "echo step-summary-preview",
                "cat missing_exec_preview.txt",
                "create_workspace preview_ws \"Workspace structure preview for action rendering\"",
                "todo clear",
                "todo add \"Preview task\" --priority=3",
                "todo next",
                "todo start T001 \"start preview\"",
                "todo done T001 \"done preview\"",
                "todo start T999 \"missing preview\"",
                "todo ls --all",
                ["read_file", {"path":"prompt_preview.txt","range":"1-2"}],
                ["read_file", {"path":"prompt_preview.txt","first_chunk":"line-2"}],
                ["read_file", {"path":"large_preview.txt"}],
                ["write_file", {"path":"write_preview.txt","content":"preview-write-line\n","mode":"new"}],
                ["write_file", {"path":"write_preview.txt","content":"should-fail\n","mode":"new"}],
                ["edit_file", {"path":"edit_preview.txt","pos_chunk":"line-beta","new_content":"line-beta-updated","mode":"replace"}],
                ["edit_file", {"path":"edit_preview.txt","pos_chunk":"line-gamma","new_content":"\nline-gamma-after","mode":"after"}],
                ["edit_file", {"path":"edit_preview.txt","pos_chunk":"line-alpha","new_content":"line-alpha-before\n","mode":"before"}],
                "if command -v tree >/dev/null 2>&1; then tree tree_preview; else find tree_preview -print | sort; fi",
                "sleep 1 && if command -v curl >/dev/null 2>&1; then curl -fsSL \"file://$(pwd)/curl_source.txt\" -o curl_download.txt && echo curl_download_ok; else cp curl_source.txt curl_download.txt && echo curl_fallback_ok; fi && wc -c curl_download.txt",
                "cd tree_preview/dir-03",
                "read_file file-00.txt 1-1",
                ["read_file", {"path":"missing-file.txt"}]
            ]
        }))
        .expect("parse actions");
        let trace = SessionRuntimeContext {
            trace_id: "trace-preview".to_string(),
            agent_name: "did:opendan:test-agent".to_string(),
            behavior: "resolve_router".to_string(),
            step_idx: 3,
            wakeup_id: "wakeup-preview".to_string(),
            session_id: "session-test".to_string(),
        };

        let results = agent.execute_actions(&trace, &actions).await;
        let rendered = render_action_results_for_prompt(&results);
        println!(
            "\n=== Action Results Prompt Preview ===\n{}\n=== End Preview ===\n",
            rendered
        );

        assert!(rendered.contains("ActionResults:"));
        assert!(rendered.contains("step-summary-preview"));
        assert!(rendered.contains("cat missing_exec_preview.txt => FAILED (exit="));
        assert!(rendered.contains("missing_exec_preview.txt"));
        assert!(rendered.contains(
            "create_workspace preview_ws \"Workspace structure preview for action rendering\" =>"
        ));
        assert!(!rendered.contains("\"session_updated\""));
        assert!(rendered.contains("todo clear => cleared 0 todo items"));
        assert!(rendered.contains("todo add \"Preview task\" --priority=3 => added todo T001"));
        assert!(rendered.contains("todo next => next todo T001: Preview task"));
        assert!(rendered
            .contains("todo start T999 \"missing preview\" => failed: todo `T999` not found"));
        assert!(rendered.contains("todo ls --all => listed 1 todo item"));
        assert!(rendered.contains("- T001 [COMPLETE]"));
        assert!(rendered.contains("read_file prompt_preview.txt range=1-2 => read"));
        assert!(rendered.contains("read_file prompt_preview.txt first_chunk=\"line-2\" => read"));
        assert!(rendered.contains("line-1"));
        assert!(rendered.contains("line-2"));
        assert!(rendered.contains("line-3"));
        assert!(rendered.contains("first_chunk=\"line-2\""));
        assert!(rendered.contains("read_file large_preview.txt => read"));
        assert!(rendered.contains(format!("read {large_bytes} bytes (truncated)").as_str()));
        assert!(rendered
            .contains("... [TRUNCATED FOR ACTION PREVIEW: Showing first 3000 lines only] ..."));
        assert!(rendered.contains("write_file write_preview.txt mode=new content=\""));
        assert!(
            rendered.contains("write mode `new` requires target file not exist: write_preview.txt")
        );
        assert!(rendered.contains("pwd: "));
        assert!(rendered.contains("tree_preview/dir-03"));
        assert!(rendered.contains("pos_chunk=\"line-beta\""));
        assert!(rendered.contains("pos_chunk=\"line-gamma\""));
        assert!(rendered.contains("pos_chunk=\"line-alpha\""));
        assert!(rendered.contains("=> replace "));
        assert!(rendered.contains("=> after "));
        assert!(rendered.contains("=> before "));
        assert!(!rendered.contains("- edit_file /"));
        assert!(!rendered.contains("unsupported worklog type"));
        assert!(rendered.contains("tree tree_preview"));
        assert!(rendered.contains("tree_preview"));
        assert!(rendered.contains("curl -fsSL"));
        assert!(rendered.contains("read_file file-00.txt 1-1 => read 14 bytes"));
        assert!(!rendered.contains("\"abs_path\""));
        assert!(rendered.contains("tree-file-3-0"));
        assert!(rendered.contains("curl_download_ok") || rendered.contains("curl_fallback_ok"));
        assert!(rendered.contains(
            "read_file missing-file.txt  =>  read file failed: No such file or directory (os error 2)"
        ));

        let edited = fs::read_to_string(agent.agent_env_root.join("edit_preview.txt"))
            .await
            .expect("read edited file");
        assert!(edited.contains("line-alpha-before\nline-alpha\n"));
        assert!(edited.contains("line-beta-updated"));
        assert!(edited.contains("line-gamma\nline-gamma-after\n"));
        let write_preview = fs::read_to_string(agent.agent_env_root.join("write_preview.txt"))
            .await
            .expect("read write preview file");
        assert_eq!(write_preview, "preview-write-line\n");

        let downloaded = fs::read_to_string(agent.agent_env_root.join("curl_download.txt"))
            .await
            .expect("read downloaded file");
        assert_eq!(downloaded, curl_source);
    }
}
