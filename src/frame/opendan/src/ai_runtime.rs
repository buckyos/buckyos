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
    ai_methods, features, value_to_object_map, AiMethodRequest, AiMethodStatus, AiPayload,
    AiResponseSummary, AiToolCall, AiToolSpec, AiccClient, Capability, ModelSpec, Requirements,
    RespFormat,
};
use log::warn;
use serde_json::{json, Value};

use ::agent_tool::{
    AgentToolManager, AgentToolResult, AgentToolStatus, SessionRuntimeContext,
};
use llm_context::{
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
            // TODO(step 6): poll via TaskManagerClient (see old/opendan
            // behavior::do_inference_once for the production polling path).
            AiMethodStatus::Running => Err(LLMComputeError::Provider(format!(
                "aicc returned async task_id={} but polling is not wired in step 2 deps",
                resp.task_id
            ))),
            AiMethodStatus::Failed => {
                Err(LLMComputeError::Provider("aicc status=failed".to_string()))
            }
        }
    }
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

/// `PolicyEngine` for opendan. The waist already enforces
/// `tool_policy.whitelist` when advertising specs to the LLM; this gate
/// covers the orthogonal axis of *human approval* (rejected calls become
/// recoverable errors that the LLM can self-correct against).
///
/// Step 3 (`behavior_cfg`) populates `approval_required` from the active
/// behavior TOML.
pub struct AgentPolicy {
    pub approval_required: Vec<String>,
}

impl AgentPolicy {
    pub fn new(approval_required: Vec<String>) -> Self {
        Self { approval_required }
    }
}

#[async_trait]
impl PolicyEngine for AgentPolicy {
    async fn gate_tool_calls(
        &self,
        _request: &LLMContextRequest,
        calls: Vec<AiToolCall>,
    ) -> Result<Vec<AiToolCall>, String> {
        if self.approval_required.is_empty() {
            return Ok(calls);
        }
        if let Some(blocked) = calls
            .iter()
            .find(|c| self.approval_required.iter().any(|n| n == &c.name))
        {
            return Err(format!(
                "tool `{}` requires human approval",
                blocked.name
            ));
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
}

impl AgentRuntime {
    pub fn new(aicc: Arc<AiccClient>, worklog: Arc<WorklogService>) -> Self {
        Self { aicc, worklog }
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

    LLMContextDeps::new(llm, tools_adapter)
        .with_policy(policy)
        .with_worklog(worklog)
        .with_turn_hook(hook)
}
