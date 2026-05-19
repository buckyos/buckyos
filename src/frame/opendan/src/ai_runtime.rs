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
    AiMethodStatus, AiPayload, AiResponse, AiToolCall, AiToolSpec, AiccClient, Capability,
    KEventClient, ModelSpec, MsgCenterClient, Requirements, RespFormat, TaskFilter,
    TaskManagerClient, TaskStatus, AICC_SERVICE_SERVICE_NAME,
};
use log::warn;
use serde_json::{json, Value};

use ::agent_tool::{AgentToolManager, AgentToolResult, AgentToolStatus, SessionRuntimeContext};
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

use crate::i18n::AgentI18n;
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
    async fn infer(&self, req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
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
/// its `AiResponse`. Mirrors the polling path the legacy
/// `behavior::do_inference_once` used, but reuses
/// `TaskManagerClient::wait_for_task_end_kevent` so the wait is kevent-
/// accelerated with a sweep fallback.
async fn resolve_async_aicc_result(external_task_id: &str) -> Result<AiResponse, LLMComputeError> {
    let external_task_id = external_task_id.trim();
    if external_task_id.is_empty() {
        return Err(LLMComputeError::Provider(
            "aicc response status=running but task_id is empty".to_string(),
        ));
    }

    let runtime = get_buckyos_api_runtime()
        .map_err(|err| LLMComputeError::Internal(format!("load buckyos runtime failed: {err}")))?;
    let taskmgr = runtime.get_task_mgr_client().await.map_err(|err| {
        LLMComputeError::Provider(format!("init task-manager client failed: {err}"))
    })?;

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

    let output = task.data.pointer("/aicc/output").cloned().ok_or_else(|| {
        LLMComputeError::Provider(format!(
            "aicc task {} terminated without /aicc/output payload",
            id
        ))
    })?;
    // Envelope-tolerant: aicc.rs writes either `AiResponse` directly
    // or `{summary: AiResponse, ...}` depending on the event payload
    // shape. Strip the wrapper when present. (Drop once the task.data schema
    // is unified — see follow-up task.)
    let summary_value = output.get("summary").cloned().unwrap_or(output);
    serde_json::from_value::<AiResponse>(summary_value).map_err(|err| {
        LLMComputeError::Provider(format!(
            "decode AiResponse from aicc task {} output failed: {err}",
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
    /// §4.7.2 — DID of the user this turn is executing on behalf of.
    /// Runtime-injected into every `call.args` as `from_user_did` so tool
    /// implementations can enforce permission checks / billing audit
    /// regardless of what the LLM tries to claim in its arguments.
    /// `None` for tool-only contexts that have no upstream human (boot
    /// turns, autonomous work sessions).
    from_user_did: Option<String>,
}

impl OpendanToolAdapter {
    pub fn new(manager: Arc<AgentToolManager>, ctx: SessionRuntimeContext) -> Self {
        Self::with_from_user_did(manager, ctx, None)
    }

    pub fn with_from_user_did(
        manager: Arc<AgentToolManager>,
        ctx: SessionRuntimeContext,
        from_user_did: Option<String>,
    ) -> Self {
        let step_idx = AtomicU32::new(ctx.step_idx);
        Self {
            manager,
            ctx,
            step_idx,
            from_user_did,
        }
    }
}

#[async_trait]
impl ToolManager for OpendanToolAdapter {
    async fn call_tool(&self, mut call: AiToolCall) -> Observation {
        let mut ctx = self.ctx.clone();
        ctx.step_idx = self.step_idx.fetch_add(1, Ordering::Relaxed);
        let call_id = call.call_id.clone();
        // §4.7.2 — overwrite any pre-existing `from_user_did` in args.
        // The LLM has no business setting this field; only the runtime
        // does. An LLM-supplied value is treated as a prompt-injection
        // attempt and discarded.
        if let Some(did) = self.from_user_did.as_ref() {
            call.args
                .insert("from_user_did".to_string(), Value::String(did.clone()));
        } else {
            call.args.remove("from_user_did");
        }
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
        // Per agent_tool protocol, only the canonical `output` is fed back to
        // the LLM so history records carry the minimum needed for tiered
        // compression — detail / cmd_args / stdout / stderr / title are
        // metadata for the UI and worklog, not for the model. Tools that
        // don't populate `output` (write_file / edit_file / ...) fall back
        // to `summary`, which already embeds the operation result.
        let text = result
            .output
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| summary.clone());
        let bytes = text.len();
        Observation::Success {
            call_id,
            content: Value::String(text),
            bytes,
            truncated: false,
        }
    }
}

// =====================================================================
// PolicyEngine — behavior-driven gate (MVP)
// =====================================================================

/// `PolicyEngine` for opendan. Two gates:
///   - `approval_required`: invocations (tools *or* actions) in this list
///     become recoverable errors the LLM can self-correct against (or that
///     the L4 worksession can escalate to a human). Matched on the
///     normalized invocation name.
///   - `enforce_whitelist`: when set, the policy also re-validates each
///     invocation against the request's policy. Per beta2.2's split,
///     XML-action names are checked against `action_whitelist`/`action_mode`
///     and everything else against `whitelist`/`mode`. Defence-in-depth on
///     top of the waist's spec advertisement, since adversarial LLMs
///     sometimes emit calls to invocations that were never advertised.
///
/// Populated by §9.3 `behavior_cfg` translation in `agent_session::build_deps`.
pub struct AgentPolicy {
    pub approval_required: Vec<String>,
    pub enforce_whitelist: bool,
}

#[derive(Debug, Clone, Copy)]
enum InvocationSurface {
    Tool,
    Action,
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

    fn gate_calls_on_surface(
        &self,
        request: &LLMContextRequest,
        calls: Vec<AiToolCall>,
        surface: InvocationSurface,
    ) -> Result<Vec<AiToolCall>, String> {
        if !self.approval_required.is_empty() {
            if let Some(blocked) = calls
                .iter()
                .find(|c| self.approval_required.iter().any(|n| n == &c.name))
            {
                return Err(format!(
                    "invocation `{}` requires human approval",
                    blocked.name
                ));
            }
        }
        if self.enforce_whitelist {
            use llm_context::request::ToolMode;
            for call in &calls {
                let (mode, whitelist, kind, whitelist_name) = match surface {
                    InvocationSurface::Tool => (
                        request.tool_policy.mode,
                        &request.tool_policy.whitelist,
                        "tool",
                        "tool_whitelist",
                    ),
                    InvocationSurface::Action => (
                        request.tool_policy.action_mode,
                        &request.tool_policy.action_whitelist,
                        "action",
                        "action_whitelist",
                    ),
                };
                match mode {
                    ToolMode::None => {
                        return Err(format!(
                            "{kind} `{}` is rejected: this behavior disables the {kind} surface",
                            call.name
                        ));
                    }
                    ToolMode::Whitelist => {
                        if !whitelist.iter().any(|n| n == &call.name) {
                            return Err(format!(
                                "{kind} `{}` is not in this behavior's {whitelist_name}",
                                call.name,
                            ));
                        }
                    }
                    ToolMode::All => {}
                }
            }
        }
        Ok(calls)
    }
}

#[async_trait]
impl PolicyEngine for AgentPolicy {
    async fn gate_tool_calls(
        &self,
        request: &LLMContextRequest,
        calls: Vec<AiToolCall>,
    ) -> Result<Vec<AiToolCall>, String> {
        self.gate_calls_on_surface(request, calls, InvocationSurface::Tool)
    }

    async fn gate_action_calls(
        &self,
        request: &LLMContextRequest,
        calls: Vec<AiToolCall>,
    ) -> Result<Vec<AiToolCall>, String> {
        self.gate_calls_on_surface(request, calls, InvocationSurface::Action)
    }
}

#[cfg(test)]
mod policy_tests {
    use super::*;
    use llm_context::request::{ContextOwnerRef, ToolMode, ToolPolicy};
    use std::collections::HashMap;

    fn request_with_policy(tool_policy: ToolPolicy) -> LLMContextRequest {
        LLMContextRequest {
            owner: ContextOwnerRef::Other {
                label: "test".to_string(),
            },
            trace: None,
            objective: String::new(),
            behavior_name: "test".to_string(),
            input: Vec::new(),
            model_policy: Default::default(),
            tool_policy,
            output: Default::default(),
            budget: Default::default(),
            human_policy: Default::default(),
            error_policy: Default::default(),
            forbid_next_behavior: false,
        }
    }

    fn call(name: &str) -> AiToolCall {
        AiToolCall {
            name: name.to_string(),
            args: HashMap::new(),
            call_id: "call-1".to_string(),
        }
    }

    #[test]
    fn exec_bash_provider_tool_uses_tool_whitelist() {
        let policy = AgentPolicy::new(Vec::new());
        let request = request_with_policy(ToolPolicy {
            mode: ToolMode::Whitelist,
            whitelist: vec!["exec_bash".to_string()],
            action_mode: ToolMode::None,
            action_whitelist: Vec::new(),
            ..Default::default()
        });

        let result = policy.gate_calls_on_surface(
            &request,
            vec![call("exec_bash")],
            InvocationSurface::Tool,
        );

        assert!(result.is_ok());
    }

    #[test]
    fn exec_bash_xml_action_uses_action_whitelist() {
        let policy = AgentPolicy::new(Vec::new());
        let request = request_with_policy(ToolPolicy {
            mode: ToolMode::Whitelist,
            whitelist: vec!["exec_bash".to_string()],
            action_mode: ToolMode::None,
            action_whitelist: Vec::new(),
            ..Default::default()
        });

        let result = policy.gate_calls_on_surface(
            &request,
            vec![call("exec_bash")],
            InvocationSurface::Action,
        );

        assert_eq!(
            result.unwrap_err(),
            "action `exec_bash` is rejected: this behavior disables the action surface"
        );
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
    i18n: AgentI18n,
}

impl OpenDanWorklogSink {
    pub fn new(
        service: Arc<WorklogService>,
        ctx: WorklogAppendCtx,
        status: Option<Arc<dyn OneLineStatusSink>>,
        i18n: AgentI18n,
    ) -> Self {
        Self {
            service,
            ctx,
            status,
            i18n,
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
                Some(self.i18n.render("status.llm_started", &[("model", model)])),
            ),
            WorkEvent::LLMFinished { trace_id, ok } => (
                "LLMFinished",
                if ok { "ok" } else { "error" },
                json!({"trace_id": trace_id, "ok": ok}),
                Some(if ok {
                    self.i18n.render("status.llm_finished", &[])
                } else {
                    self.i18n.render("status.llm_failed", &[])
                }),
            ),
            WorkEvent::LLMInferenceFailed { trace_id, error } => (
                "LLMInferenceFailed",
                "error",
                json!({"trace_id": trace_id, "error": &error}),
                Some(self.i18n.render("status.llm_error", &[("error", error)])),
            ),
            WorkEvent::ToolCallPlanned {
                trace_id,
                tool,
                call_id,
            } => (
                "ToolCallPlanned",
                "ok",
                json!({"trace_id": trace_id, "tool": &tool, "call_id": call_id}),
                Some(self.i18n.render("status.tool_planned", &[("tool", tool)])),
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
                Some(self.i18n.render(
                    "status.tool_finished",
                    &[
                        ("tool", tool),
                        (
                            "result",
                            if ok {
                                self.i18n.render("status.tool_result_done", &[])
                            } else {
                                self.i18n.render("status.tool_result_failed", &[])
                            },
                        ),
                    ],
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
                Some(self.i18n.render(
                    "status.tool_failed",
                    &[("tool", tool), ("message", message)],
                )),
            ),
            WorkEvent::OutputParseFailed { trace_id, error } => (
                "OutputParseFailed",
                "error",
                json!({"trace_id": trace_id, "error": &error}),
                Some(self.i18n.render("status.parse_error", &[("error", error)])),
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
                Some(self.i18n.render(
                    "status.context_rewritten",
                    &[
                        ("from_messages", from_messages.to_string()),
                        ("to_messages", to_messages.to_string()),
                    ],
                )),
            ),
            WorkEvent::SelfReportSet { trace_id, chars } => (
                "SelfReportSet",
                "ok",
                json!({"trace_id": trace_id, "chars": chars}),
                Some(
                    self.i18n
                        .render("status.self_report_set", &[("chars", chars.to_string())]),
                ),
            ),
            WorkEvent::MessageSent {
                trace_id,
                target,
                chars,
            } => (
                "MessageSent",
                "ok",
                json!({"trace_id": trace_id, "target": &target, "chars": chars}),
                Some(self.i18n.render(
                    "status.message_sent",
                    &[("target", target), ("chars", chars.to_string())],
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
                warn!("opendan.snapshot: mkdir {} failed: {err}", parent.display());
                return;
            }
        }
        // tmp + rename for crash-consistency: a half-written `state.snap`
        // would prevent the session from recovering on next boot.
        let tmp = self.path.with_extension("snap.tmp");
        if let Err(err) = std::fs::write(&tmp, &bytes) {
            warn!("opendan.snapshot: write {} failed: {err}", tmp.display());
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
    pub i18n: AgentI18n,
    /// Behavior Loop: structured parser + matched renderer. `None` ⇒
    /// traditional Agent Loop. Filled by [`behavior_cfg`](crate::behavior_cfg).
    pub parser_renderer: Option<(Arc<dyn LLMResultParser>, Arc<dyn StepRenderer>)>,
    /// §4.7.2 — DID of the user the current turn is acting on behalf of.
    /// Forwarded into every dispatched tool call as a runtime-injected
    /// `from_user_did` arg. In 1-on-1 chat this is the peer's DID
    /// (== the agent owner); in group chat it's the @-mentioner. `None`
    /// for autonomous turns (boot, scheduled work).
    pub from_user_did: Option<String>,
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
        i18n,
        parser_renderer,
        from_user_did,
    } = input;

    let worklog_ctx = WorklogAppendCtx {
        trace_id: ctx.trace_id.clone(),
        agent_name: ctx.agent_name.clone(),
        behavior: ctx.behavior.clone(),
        session_id: ctx.session_id.clone(),
    };

    let llm: Arc<dyn LlmClient> = Arc::new(AiccLlmClient::new(runtime.aicc.clone()));
    let tools_adapter: Arc<dyn ToolManager> = Arc::new(OpendanToolAdapter::with_from_user_did(
        tools,
        ctx,
        from_user_did,
    ));
    let policy: Arc<dyn PolicyEngine> = Arc::new(AgentPolicy::new(approval_required));
    let worklog: Arc<dyn WorklogSink> = Arc::new(OpenDanWorklogSink::new(
        runtime.worklog.clone(),
        worklog_ctx,
        one_line_status,
        i18n,
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
