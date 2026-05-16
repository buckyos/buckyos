//! §9 step 2 of NewOpenDANRuntime — assemble `LLMContextDeps` from the
//! existing buckyos surfaces (aicc / agent_tool / worklog).
//!
//! The waist (`llm_context`) owns the actual LLM loop, tool dispatch,
//! snapshot/resume and behavior parsing. opendan only provides the seven
//! trait implementations the waist syscalls into:
//!
//! - [`AiccLlmClient`]       → `LlmClient`     (over `buckyos_api::AiccClient`)
//! - [`OpendanToolAdapter`]  → `ToolManager`   (over `agent_tool::AgentToolManager`)
//! - [`AgentPolicy`]         → `PolicyEngine`  (behavior-driven gate, MVP whitelist + approval)
//! - [`OpenDanWorklogSink`]  → `WorklogSink`   (over `crate::worklog::WorklogService`)
//! - [`SessionSnapshotHook`] → `TurnHook`      (writes `LLMContextSnapshot` to disk)
//!
//! Step 3 (`behavior_cfg`) will plug `LLMResultParser` / `StepRenderer` /
//! `HistoryCompressor` on top of [`build_session_deps`] when the Behavior
//! Loop is enabled for a given behavior.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use buckyos_api::{
    ai_methods, features, get_buckyos_api_runtime, value_to_object_map, AiMethodRequest,
    AiMethodStatus, AiPayload, AiResponseSummary, AiToolCall, AiToolSpec, AiccClient, Capability,
    KEventClient, MsgCenterClient, ModelSpec, Requirements, RespFormat, TaskFilter,
    TaskManagerClient, TaskStatus, AICC_SERVICE_SERVICE_NAME,
};
use log::warn;
use serde_json::{json, Value};

use ::agent_tool::{
    AgentToolManager, AgentToolResult, AgentToolStatus, SessionRuntimeContext,
};
use llm_context::{
    behavior_loop::{LLMResultParser, StepRenderer},
    deps::{
        LLMContextDeps, LlmClient, LlmInferenceRequest, PolicyEngine, ToolManager, ToolSpecLite,
        TurnHook, WorkEvent, WorklogSink,
    },
    error::LLMComputeError,
    observation::Observation,
    request::LLMContextRequest,
    state::LLMContextSnapshot,
};

use crate::worklog::{WorklogAppendCtx, WorklogService};

// =====================================================================
// LlmClient — aicc adapter
// =====================================================================

/// `LlmClient` over `AiccClient`. One `infer()` ⇒ one `llm.chat` round-trip;
/// adapter retry / fallback happens inside aicc, not here.
pub struct AiccLlmClient {
    aicc: Arc<AiccClient>,
}

impl AiccLlmClient {
    pub fn new(aicc: Arc<AiccClient>) -> Self {
        Self { aicc }
    }
}

#[async_trait]
impl LlmClient for AiccLlmClient {
    async fn infer(
        &self,
        req: LlmInferenceRequest,
    ) -> Result<AiResponseSummary, LLMComputeError> {
        let LlmInferenceRequest {
            messages,
            model_alias,
            fallbacks,
            temperature,
            max_completion_tokens,
            force_json,
            json_schema,
            provider_options,
            tool_specs,
            allow_tool_calls,
            // AICC's `call_method` does not currently support cancel
            // wire-through; the waist's `select!` already drops this future
            // on interrupt, so dropping the abort token here is safe — the
            // remote may keep generating tokens but the scheduler thread is
            // freed immediately.
            abort: _,
        } = req;

        // Tool catalogue (only advertised when the policy lets the LLM call tools)
        let advertised_tools: Vec<AiToolSpec> = if allow_tool_calls {
            tool_specs
                .into_iter()
                .map(|spec| AiToolSpec {
                    name: spec.name,
                    description: spec.description,
                    args_schema: value_to_object_map(spec.args_schema),
                    output_schema: json!({}),
                })
                .collect()
        } else {
            Vec::new()
        };

        // provider_options: opaque pass-through merged into payload options.
        let mut options = serde_json::Map::new();
        if let Some(t) = temperature {
            options.insert("temperature".to_string(), Value::from(t));
        }
        if let Some(m) = max_completion_tokens {
            options.insert("max_completion_tokens".to_string(), Value::from(m));
        }
        if let Some(schema) = json_schema {
            options.insert("json_schema".to_string(), schema);
        }
        if let Some(extra) = provider_options {
            match extra {
                Value::Object(obj) => {
                    for (k, v) in obj {
                        options.insert(k, v);
                    }
                }
                other => {
                    options.insert("provider_options".to_string(), other);
                }
            }
        }

        let payload = AiPayload::new(
            None,
            messages,
            advertised_tools,
            Vec::new(),
            None,
            Some(Value::Object(options)),
        );

        let mut must_features: Vec<String> = Vec::new();
        if allow_tool_calls && !payload.tool_specs.is_empty() {
            must_features.push(features::TOOL_CALLING.to_string());
        }
        if force_json {
            must_features.push(features::JSON_OUTPUT.to_string());
        }

        let mut requirements = Requirements::new(must_features, None, None, None);
        if force_json {
            requirements.resp_format = RespFormat::Json;
        }

        let request = AiMethodRequest::new(
            Capability::Llm,
            ModelSpec::new(model_alias, None),
            requirements,
            payload,
            None,
        );

        // Fallbacks aren't directly representable in `AiMethodRequest` yet
        // (model_spec carries a single alias); attach them to options so the
        // aicc adapter can pick them up when it adds fallback wiring.
        let _ = fallbacks;

        let resp = self
            .aicc
            .call_method(ai_methods::LLM_CHAT, request)
            .await
            .map_err(|err| LLMComputeError::Provider(err.to_string()))?;

        match resp.status {
            AiMethodStatus::Succeeded => resp.result.ok_or_else(|| {
                LLMComputeError::Provider(
                    "aicc returned status=succeeded without result".to_string(),
                )
            }),
            AiMethodStatus::Running => resolve_async_aicc_result(resp.task_id.as_str()).await,
            AiMethodStatus::Failed => {
                Err(LLMComputeError::Provider("aicc status=failed".to_string()))
            }
        }
    }
}

/// Block until an AICC-side async task reaches a terminal state and return
/// its `AiResponseSummary`. Mirrors the polling path the legacy
/// `behavior::do_inference_once` used, but reuses
/// `TaskManagerClient::wait_for_task_end_kevent` so the wait is kevent-
/// accelerated with a sweep fallback.
async fn resolve_async_aicc_result(
    external_task_id: &str,
) -> Result<AiResponseSummary, LLMComputeError> {
    let external_task_id = external_task_id.trim();
    if external_task_id.is_empty() {
        return Err(LLMComputeError::Provider(
            "aicc response status=running but task_id is empty".to_string(),
        ));
    }

    let runtime = get_buckyos_api_runtime().map_err(|err| {
        LLMComputeError::Internal(format!("load buckyos runtime failed: {err}"))
    })?;
    let taskmgr = runtime
        .get_task_mgr_client()
        .await
        .map_err(|err| LLMComputeError::Provider(format!("init task-manager client failed: {err}")))?;

    let id = resolve_aicc_task_id(&taskmgr, external_task_id).await?;

    let task = taskmgr
        .wait_for_task_end_kevent(id)
        .await
        .map_err(|err| LLMComputeError::Provider(err.to_string()))?;

    if task.status != TaskStatus::Completed {
        return Err(LLMComputeError::Provider(format!(
            "aicc task {} ended with status {:?}",
            id, task.status
        )));
    }

    let output = task
        .data
        .pointer("/aicc/output")
        .cloned()
        .ok_or_else(|| {
            LLMComputeError::Provider(format!(
                "aicc task {} terminated without /aicc/output payload",
                id
            ))
        })?;
    // Envelope-tolerant: aicc.rs writes either `AiResponseSummary` directly
    // or `{summary: AiResponseSummary, ...}` depending on the event payload
    // shape. Strip the wrapper when present. (Drop once the task.data schema
    // is unified — see follow-up task.)
    let summary_value = output
        .get("summary")
        .cloned()
        .unwrap_or(output);
    serde_json::from_value::<AiResponseSummary>(summary_value).map_err(|err| {
        LLMComputeError::Provider(format!(
            "decode AiResponseSummary from aicc task {} output failed: {err}",
            id
        ))
    })
}

/// AICC's `external_task_id` is always shaped like `aicc-{ts}-{seq}` and is
/// not a task-manager id. Walk task-manager listings filtered by the AICC
/// app to find the row whose `data.aicc.external_task_id` matches.
async fn resolve_aicc_task_id(
    taskmgr: &TaskManagerClient,
    external_task_id: &str,
) -> Result<i64, LLMComputeError> {
    if let Ok(id) = external_task_id.parse::<i64>() {
        return Ok(id);
    }

    let filter = TaskFilter {
        app_id: Some(AICC_SERVICE_SERVICE_NAME.to_string()),
        task_type: None,
        status: None,
        parent_id: None,
        root_id: None,
    };
    let tasks = taskmgr
        .list_tasks(Some(filter), None, None)
        .await
        .map_err(|err| LLMComputeError::Provider(err.to_string()))?;

    for task in tasks {
        let matched = task
            .data
            .pointer("/aicc/external_task_id")
            .and_then(|value| value.as_str())
            .map(|value| value == external_task_id)
            .unwrap_or(false);
        if matched {
            return Ok(task.id);
        }
    }
    Err(LLMComputeError::Provider(format!(
        "aicc task_id '{}' is not a numeric task id and no task-manager row references it",
        external_task_id
    )))
}

// =====================================================================
// ToolManager — agent_tool adapter
// =====================================================================

/// Wraps an `AgentToolManager` so the waist can dispatch tool calls. The
/// adapter is constructed per-session because `agent_tool::AgentTool::call`
/// requires a `SessionRuntimeContext` keyed by session / trace / behavior.
pub struct OpendanToolAdapter {
    manager: Arc<AgentToolManager>,
    ctx: SessionRuntimeContext,
    step_idx: AtomicU32,
}

impl OpendanToolAdapter {
    pub fn new(manager: Arc<AgentToolManager>, ctx: SessionRuntimeContext) -> Self {
        let step_idx = AtomicU32::new(ctx.step_idx);
        Self {
            manager,
            ctx,
            step_idx,
        }
    }
}

#[async_trait]
impl ToolManager for OpendanToolAdapter {
    async fn call_tool(&self, call: AiToolCall) -> Observation {
        let mut ctx = self.ctx.clone();
        ctx.step_idx = self.step_idx.fetch_add(1, Ordering::Relaxed);
        let call_id = call.call_id.clone();
        match self.manager.call_tool(&ctx, call).await {
            Ok(result) => result_to_observation(call_id, result),
            Err(err) => Observation::Error {
                call_id,
                message: err.to_string(),
            },
        }
    }

    fn list_tool_specs(&self) -> Vec<ToolSpecLite> {
        self.manager
            .list_tool_specs()
            .into_iter()
            .map(|s| ToolSpecLite {
                name: s.name,
                description: s.description,
                args_schema: s.args_schema,
            })
            .collect()
    }

    fn has_tool(&self, name: &str) -> bool {
        self.manager.has_tool(name)
    }
}

fn result_to_observation(call_id: String, result: AgentToolResult) -> Observation {
    if matches!(result.status, AgentToolStatus::Pending) {
        return Observation::Pending { call_id };
    }
    let is_error = matches!(result.status, AgentToolStatus::Error);
    let summary = result.summary.clone();
    let title = result.title.clone();
    let bytes = serde_json::to_vec(&result).map(|v| v.len()).unwrap_or(0);
    let content = serde_json::to_value(result).unwrap_or(Value::Null);
    if is_error {
        let message = if !summary.is_empty() {
            summary
        } else if !title.is_empty() {
            title
        } else {
            "tool error".to_string()
        };
        Observation::Error { call_id, message }
    } else {
        Observation::Success {
            call_id,
            content,
            bytes,
            truncated: false,
        }
    }
}

// =====================================================================
// PolicyEngine — behavior-driven gate (MVP)
// =====================================================================

/// `PolicyEngine` for opendan. Two gates:
///   - `approval_required`: tool calls in this list become recoverable errors
///     the LLM can self-correct against (or that the L4 worksession can
///     escalate to a human).
///   - `enforce_whitelist`: when set, the policy also re-validates each call
///     against the request's `tool_policy.whitelist`. Defence-in-depth on top
///     of the waist's spec advertisement, since adversarial LLMs sometimes
///     emit calls to tools that were never advertised.
///
/// Populated by §9.3 `behavior_cfg` translation in `agent_session::build_deps`.
pub struct AgentPolicy {
    pub approval_required: Vec<String>,
    pub enforce_whitelist: bool,
}

impl AgentPolicy {
    pub fn new(approval_required: Vec<String>) -> Self {
        Self {
            approval_required,
            enforce_whitelist: true,
        }
    }

    pub fn with_whitelist_enforcement(mut self, enforce: bool) -> Self {
        self.enforce_whitelist = enforce;
        self
    }
}

#[async_trait]
impl PolicyEngine for AgentPolicy {
    async fn gate_tool_calls(
        &self,
        request: &LLMContextRequest,
        calls: Vec<AiToolCall>,
    ) -> Result<Vec<AiToolCall>, String> {
        if !self.approval_required.is_empty() {
            if let Some(blocked) = calls
                .iter()
                .find(|c| self.approval_required.iter().any(|n| n == &c.name))
            {
                return Err(format!(
                    "tool `{}` requires human approval",
                    blocked.name
                ));
            }
        }
        if self.enforce_whitelist {
            use llm_context::request::ToolMode;
            if matches!(request.tool_policy.mode, ToolMode::Whitelist) {
                if let Some(off) = calls
                    .iter()
                    .find(|c| !request.tool_policy.whitelist.iter().any(|n| n == &c.name))
                {
                    return Err(format!(
                        "tool `{}` is not in this behavior's whitelist",
                        off.name
                    ));
                }
            }
        }
        Ok(calls)
    }
}

// =====================================================================
// WorklogSink — WorklogService adapter + one-line status fanout
// =====================================================================

/// Receiver for the session's "one-line status" string surfaced to UIs.
/// Implementations typically own an `Arc<Mutex<String>>` or a broadcast
/// channel; the worklog sink calls `set` on every meaningful `WorkEvent`.
pub trait OneLineStatusSink: Send + Sync {
    fn set(&self, status: String);
}

/// `WorklogSink` that translates waist `WorkEvent`s into worklog records
/// and updates the session's one-line status as a side effect.
pub struct OpenDanWorklogSink {
    service: Arc<WorklogService>,
    ctx: WorklogAppendCtx,
    status: Option<Arc<dyn OneLineStatusSink>>,
}

impl OpenDanWorklogSink {
    pub fn new(
        service: Arc<WorklogService>,
        ctx: WorklogAppendCtx,
        status: Option<Arc<dyn OneLineStatusSink>>,
    ) -> Self {
        Self {
            service,
            ctx,
            status,
        }
    }

    fn update_status(&self, line: Option<String>) {
        if let (Some(sink), Some(line)) = (self.status.as_ref(), line) {
            sink.set(line);
        }
    }
}

#[async_trait]
impl WorklogSink for OpenDanWorklogSink {
    async fn emit(&self, event: WorkEvent) {
        let (event_type, status, payload, status_line) = match event {
            WorkEvent::LLMStarted { trace_id, model } => (
                "LLMStarted",
                "ok",
                json!({"trace_id": trace_id, "model": &model}),
                Some(format!("LLM thinking ({model})")),
            ),
            WorkEvent::LLMFinished { trace_id, ok } => (
                "LLMFinished",
                if ok { "ok" } else { "error" },
                json!({"trace_id": trace_id, "ok": ok}),
                Some(if ok {
                    "LLM finished".to_string()
                } else {
                    "LLM failed".to_string()
                }),
            ),
            WorkEvent::LLMInferenceFailed { trace_id, error } => (
                "LLMInferenceFailed",
                "error",
                json!({"trace_id": trace_id, "error": &error}),
                Some(format!("LLM error: {error}")),
            ),
            WorkEvent::ToolCallPlanned {
                trace_id,
                tool,
                call_id,
            } => (
                "ToolCallPlanned",
                "ok",
                json!({"trace_id": trace_id, "tool": &tool, "call_id": call_id}),
                Some(format!("tool: {tool}")),
            ),
            WorkEvent::ToolCallFinished {
                trace_id,
                tool,
                call_id,
                ok,
                duration_ms,
            } => (
                "ToolCallFinished",
                if ok { "ok" } else { "error" },
                json!({
                    "trace_id": trace_id,
                    "tool": &tool,
                    "call_id": call_id,
                    "ok": ok,
                    "duration_ms": duration_ms,
                }),
                Some(format!(
                    "tool {tool} {}",
                    if ok { "done" } else { "failed" }
                )),
            ),
            WorkEvent::ToolCallFailed {
                trace_id,
                tool,
                call_id,
                message,
            } => (
                "ToolCallFailed",
                "error",
                json!({
                    "trace_id": trace_id,
                    "tool": &tool,
                    "call_id": call_id,
                    "message": &message,
                }),
                Some(format!("tool {tool} failed: {message}")),
            ),
            WorkEvent::OutputParseFailed { trace_id, error } => (
                "OutputParseFailed",
                "error",
                json!({"trace_id": trace_id, "error": &error}),
                Some(format!("parse error: {error}")),
            ),
            WorkEvent::ContextRewritten {
                trace_id,
                from_messages,
                to_messages,
            } => (
                "ContextRewritten",
                "ok",
                json!({
                    "trace_id": trace_id,
                    "from_messages": from_messages,
                    "to_messages": to_messages,
                }),
                Some(format!(
                    "compressed history: {from_messages} → {to_messages}"
                )),
            ),
        };

        self.update_status(status_line);

        let record = json!({
            "type": event_type,
            "status": status,
            "trace_id": payload.get("trace_id").cloned().unwrap_or(Value::Null),
            "payload": payload,
        });
        if let Err(err) = self.service.append_record(&self.ctx, record).await {
            warn!("opendan.worklog: append failed: {err}");
        }
    }
}

// =====================================================================
// TurnHook — snapshot persistence
// =====================================================================

/// `TurnHook` that flushes the latest `LLMContextSnapshot` to disk before
/// every LLM inference. Pair with `session/.meta/state.snap`.
///
/// Sync I/O is intentional — the waist blocks on this hook, and tokio's
/// `spawn_blocking` would add overhead for a small JSON write. If profiling
/// shows the write dominates latency, switch to a bounded channel + writer
/// task.
pub struct SessionSnapshotHook {
    path: PathBuf,
}

impl SessionSnapshotHook {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl TurnHook for SessionSnapshotHook {
    fn before_inference(&self, snapshot: &LLMContextSnapshot) {
        let bytes = match serde_json::to_vec(snapshot) {
            Ok(v) => v,
            Err(err) => {
                warn!("opendan.snapshot: serialize failed: {err}");
                return;
            }
        };
        if let Some(parent) = self.path.parent() {
            if let Err(err) = std::fs::create_dir_all(parent) {
                warn!(
                    "opendan.snapshot: mkdir {} failed: {err}",
                    parent.display()
                );
                return;
            }
        }
        // tmp + rename for crash-consistency: a half-written `state.snap`
        // would prevent the session from recovering on next boot.
        let tmp = self.path.with_extension("snap.tmp");
        if let Err(err) = std::fs::write(&tmp, &bytes) {
            warn!(
                "opendan.snapshot: write {} failed: {err}",
                tmp.display()
            );
            return;
        }
        if let Err(err) = std::fs::rename(&tmp, &self.path) {
            warn!(
                "opendan.snapshot: rename to {} failed: {err}",
                self.path.display()
            );
        }
    }
}

// =====================================================================
// AgentRuntime — process-level singleton, deps factory
// =====================================================================

/// Process-level shared state — singleton per opendan process (§1 of the
/// design doc). Future steps will extend this with `contact_mgr` and
/// `task_mgr` once they have consumers.
pub struct AgentRuntime {
    pub aicc: Arc<AiccClient>,
    pub worklog: Arc<WorklogService>,
    /// Optional msg-center handle. When set, [`AIAgent`] also pulls inbound
    /// messages from msg-center inbox boxes and feeds them to the session
    /// dispatcher; when `None` the agent only consumes whatever pushes into
    /// `AIAgent::inbox()` (tests / CLI).
    pub msg_center: Option<Arc<MsgCenterClient>>,
    /// Optional kevent handle. Paired with `msg_center` to drive the inbox
    /// pump — when both are set, the pump uses kevent as a poll accelerator
    /// and falls back to box sweeps on timeout / reader loss.
    pub kevent_client: Option<Arc<KEventClient>>,
    /// Optional task-manager handle. Wired through to async-tool dispatch
    /// (§9.4 `PendingTool` outcome) and cross-session task notifications.
    /// `None` ⇒ async-tool dispatch falls back to inline blocking execution
    /// + the session worker logs a warning when it can't park a long-running
    /// tool externally.
    pub task_mgr: Option<Arc<TaskManagerClient>>,
}

impl AgentRuntime {
    pub fn new(aicc: Arc<AiccClient>, worklog: Arc<WorklogService>) -> Self {
        Self {
            aicc,
            worklog,
            msg_center: None,
            kevent_client: None,
            task_mgr: None,
        }
    }

    pub fn with_msg_center(mut self, client: Arc<MsgCenterClient>) -> Self {
        self.msg_center = Some(client);
        self
    }

    pub fn with_kevent_client(mut self, client: Arc<KEventClient>) -> Self {
        self.kevent_client = Some(client);
        self
    }

    pub fn with_task_mgr(mut self, client: Arc<TaskManagerClient>) -> Self {
        self.task_mgr = Some(client);
        self
    }
}

/// Inputs to [`build_session_deps`]. Bundled so callers don't pass a
/// half-dozen positional args; every field has a well-defined home in the
/// new runtime's session layer.
pub struct SessionDepsInput {
    pub tools: Arc<AgentToolManager>,
    pub ctx: SessionRuntimeContext,
    pub snapshot_path: PathBuf,
    pub approval_required: Vec<String>,
    pub one_line_status: Option<Arc<dyn OneLineStatusSink>>,
    /// Behavior Loop: structured parser + matched renderer. `None` ⇒
    /// traditional Agent Loop. Filled by [`behavior_cfg`](crate::behavior_cfg).
    pub parser_renderer: Option<(Arc<dyn LLMResultParser>, Arc<dyn StepRenderer>)>,
}

/// Assemble per-session `LLMContextDeps`. Step 3 will compose the optional
/// behavior-loop fields (`result_parser` / `step_renderer` /
/// `history_compressor`) on top of the value returned here.
pub fn build_session_deps(runtime: &AgentRuntime, input: SessionDepsInput) -> LLMContextDeps {
    let SessionDepsInput {
        tools,
        ctx,
        snapshot_path,
        approval_required,
        one_line_status,
        parser_renderer,
    } = input;

    let worklog_ctx = WorklogAppendCtx {
        trace_id: ctx.trace_id.clone(),
        agent_name: ctx.agent_name.clone(),
        behavior: ctx.behavior.clone(),
        session_id: ctx.session_id.clone(),
    };

    let llm: Arc<dyn LlmClient> = Arc::new(AiccLlmClient::new(runtime.aicc.clone()));
    let tools_adapter: Arc<dyn ToolManager> = Arc::new(OpendanToolAdapter::new(tools, ctx));
    let policy: Arc<dyn PolicyEngine> = Arc::new(AgentPolicy::new(approval_required));
    let worklog: Arc<dyn WorklogSink> = Arc::new(OpenDanWorklogSink::new(
        runtime.worklog.clone(),
        worklog_ctx,
        one_line_status,
    ));
    let hook: Arc<dyn TurnHook> = Arc::new(SessionSnapshotHook::new(snapshot_path));

    let mut deps = LLMContextDeps::new(llm, tools_adapter)
        .with_policy(policy)
        .with_worklog(worklog)
        .with_turn_hook(hook);
    if let Some((parser, renderer)) = parser_renderer {
        deps = deps.with_result_parser(parser).with_step_renderer(renderer);
    }
    deps
}
