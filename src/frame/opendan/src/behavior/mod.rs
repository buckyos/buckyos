use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use buckyos_api::{
    features, AiMessage, AiPayload, AiccClient, Capability, CompleteRequest, CompleteResponse,
    CompleteStatus, CompleteTaskOptions, CreateTaskOptions, ModelSpec, Requirements, TaskFilter,
    TaskManagerClient, TaskStatus, AICC_SERVICE_SERVICE_NAME,
};
use serde_json::{json, Map, Value as Json};

use crate::agent_tool::{ToolCall, ToolManager, ToolSpec};

pub mod config;
pub mod observability;
pub mod parser;
pub mod policy_adapter;
pub mod prompt;
pub mod sanitize;
pub mod tool_loop;
pub mod types;

pub use config::{BehaviorConfig, BehaviorConfigError};
pub use observability::{Event, WorklogSink};
pub use parser::{OutputParser, StepDraft};
pub use policy_adapter::PolicyEngine;
pub use prompt::{ChatMessage, ChatRole, PromptBuilder, PromptPack, Truncator};
pub use sanitize::Sanitizer;
pub use tool_loop::ToolContext;
pub use types::*;

#[derive(Clone)]
pub struct LLMBehaviorDeps {
    pub taskmgr: Arc<TaskManagerClient>,
    pub aicc: Arc<AiccClient>,
    pub tools: Arc<ToolManager>,
    pub policy: Arc<dyn PolicyEngine>,
    pub worklog: Arc<dyn WorklogSink>,
    pub tokenizer: Arc<dyn Tokenizer>,
}

pub struct LLMBehavior {
    pub cfg: LLMBehaviorConfig,
    pub deps: LLMBehaviorDeps,
}

const LLM_BEHAVIOR_TASK_TYPE: &str = "llm_behavior";
const LLM_INFER_TASK_TYPE: &str = "llm_infer";
const LLM_TASK_NAME_SEQ_BITS: u32 = 20;
const LLM_TASK_NAME_SEQ_MASK: u64 = (1_u64 << LLM_TASK_NAME_SEQ_BITS) - 1;
static LLM_TASK_NAME_SEQ: AtomicU64 = AtomicU64::new(0);

impl LLMBehavior {
    pub fn new(cfg: LLMBehaviorConfig, deps: LLMBehaviorDeps) -> Self {
        Self { cfg, deps }
    }

    pub async fn from_behavior_dir(
        behaviors_dir: impl AsRef<Path>,
        behavior_name: &str,
        deps: LLMBehaviorDeps,
    ) -> Result<(Self, BehaviorConfig), BehaviorConfigError> {
        let behavior_cfg = BehaviorConfig::load_from_dir(behaviors_dir, behavior_name).await?;
        let llm_cfg = behavior_cfg.to_llm_behavior_config();
        Ok((Self::new(llm_cfg, deps), behavior_cfg))
    }

    pub async fn run_step(&self, input: ProcessInput) -> LLMResult {
        let started = now_ms();
        let track = TrackInfo {
            trace_id: input.trace.trace_id.clone(),
            model: self.cfg.model_policy.preferred.clone(),
            provider: "unknown".to_string(),
            latency_ms: 0,
            llm_task_ids: vec![],
            errors: vec![],
        };
        let behavior_task_id = match self.create_behavior_task(&input).await {
            Ok(task_id) => task_id,
            Err(err) => return self.err_from_llm(&input, track, started, err),
        };
        let _ = self
            .deps
            .taskmgr
            .update_task(
                behavior_task_id,
                Some(TaskStatus::Running),
                Some(0.05),
                Some("behavior started".to_string()),
                None,
            )
            .await;

        let result = self
            .run_step_inner(&input, started, track, behavior_task_id)
            .await;
        self.finalize_behavior_task(behavior_task_id, &result).await;
        result
    }

    async fn run_step_inner(
        &self,
        input: &ProcessInput,
        started: u64,
        mut track: TrackInfo,
        behavior_task_id: i64,
    ) -> LLMResult {
        if is_deadline_exceeded(started, input.limits.deadline_ms) {
            return self.err(
                input,
                track,
                started,
                LLMErrorKind::Timeout,
                "step deadline exceeded before inference".to_string(),
            );
        }

        let allowed_tools = match self.deps.policy.allowed_tools(input).await {
            Ok(tools) => tools,
            Err(err) => {
                return self.err(input, track, started, LLMErrorKind::ToolDenied, err);
            }
        };

        let prompt =
            match PromptBuilder::build(input, &allowed_tools, &self.cfg, &*self.deps.tokenizer) {
                Ok(p) => p,
                Err(err) => {
                    return self.err(input, track, started, LLMErrorKind::PromptBuildFailed, err);
                }
            };

        self.deps
            .worklog
            .emit(Event::LLMStarted {
                trace: input.trace.clone(),
                model: self.cfg.model_policy.preferred.clone(),
            })
            .await;

        let (mut usage, first_resp, first_task_id) = match self
            .do_inference_once(
                input,
                &allowed_tools,
                prompt.clone(),
                None,
                behavior_task_id,
            )
            .await
        {
            Ok(v) => v,
            Err(err) => return self.err_from_llm(input, track, started, err),
        };
        track.llm_task_ids.push(first_task_id);
        track.model = first_resp.model.clone();
        track.provider = first_resp.provider.clone();

        let mut draft = match OutputParser::parse_first(&first_resp, self.cfg.force_json) {
            Ok(d) => d,
            Err(err) => {
                return self.err(input, track, started, LLMErrorKind::OutputParseFailed, err);
            }
        };

        let mut rounds_left = input.limits.max_tool_rounds;
        let mut tool_trace = Vec::new();

        while !draft.tool_calls.is_empty() {
            if rounds_left == 0 {
                return self.err(
                    input,
                    track,
                    started,
                    LLMErrorKind::ToolLoopExceeded,
                    "tool loop exceeded max_tool_rounds".to_string(),
                );
            }
            rounds_left -= 1;

            if is_deadline_exceeded(started, input.limits.deadline_ms) {
                return self.err(
                    input,
                    track,
                    started,
                    LLMErrorKind::Timeout,
                    "step deadline exceeded during tool loop".to_string(),
                );
            }

            let gated_calls = match self
                .deps
                .policy
                .gate_tool_calls(input, &draft.tool_calls)
                .await
            {
                Ok(calls) => calls,
                Err(err) => {
                    return self.err(input, track, started, LLMErrorKind::ToolDenied, err);
                }
            };

            if gated_calls.len() > input.limits.max_tool_calls_per_round as usize {
                return self.err(
                    input,
                    track,
                    started,
                    LLMErrorKind::ToolLoopExceeded,
                    format!(
                        "tool calls {} exceeds max_tool_calls_per_round {}",
                        gated_calls.len(),
                        input.limits.max_tool_calls_per_round
                    ),
                );
            }

            let mut tool_observations = Vec::new();
            for call in gated_calls.clone() {
                let call_started = now_ms();
                self.deps
                    .worklog
                    .emit(Event::ToolCallPlanned {
                        trace: input.trace.clone(),
                        tool: call.name.clone(),
                        call_id: call.call_id.clone(),
                    })
                    .await;

                let ctx = tool_loop::trace_to_tool_call_context(&input.trace);
                let exec = self.deps.tools.call_tool(&ctx, call.clone()).await;
                let duration_ms = now_ms().saturating_sub(call_started);

                match exec {
                    Ok(raw) => {
                        let obs = Sanitizer::sanitize_observation(
                            ObservationSource::Tool,
                            &call.name,
                            raw,
                            input.limits.max_observation_bytes,
                        );
                        tool_observations.push(obs);
                        tool_trace.push(ToolExecRecord {
                            tool_name: call.name.clone(),
                            call_id: call.call_id.clone(),
                            ok: true,
                            duration_ms,
                            error: None,
                        });
                        self.deps
                            .worklog
                            .emit(Event::ToolCallFinished {
                                trace: input.trace.clone(),
                                tool: call.name,
                                call_id: call.call_id,
                                ok: true,
                                duration_ms,
                            })
                            .await;
                    }
                    Err(err) => {
                        let err_msg = err.to_string();
                        let obs = Sanitizer::tool_error_observation(
                            &call.name,
                            err_msg.clone(),
                            input.limits.max_observation_bytes,
                        );
                        tool_observations.push(obs);
                        tool_trace.push(ToolExecRecord {
                            tool_name: call.name.clone(),
                            call_id: call.call_id.clone(),
                            ok: false,
                            duration_ms,
                            error: Some(err_msg.clone()),
                        });
                        track
                            .errors
                            .push(format!("tool {} failed: {}", call.name, err_msg));
                        self.deps
                            .worklog
                            .emit(Event::ToolCallFinished {
                                trace: input.trace.clone(),
                                tool: call.name,
                                call_id: call.call_id,
                                ok: false,
                                duration_ms,
                            })
                            .await;
                    }
                }
            }

            let tool_ctx = ToolContext {
                tool_calls: gated_calls,
                observations: tool_observations,
            };

            let (usage2, followup_resp, followup_task_id) = match self
                .do_inference_once(
                    input,
                    &allowed_tools,
                    prompt.clone(),
                    Some(tool_ctx),
                    behavior_task_id,
                )
                .await
            {
                Ok(v) => v,
                Err(err) => return self.err_from_llm(input, track, started, err),
            };
            track.llm_task_ids.push(followup_task_id);
            track.model = followup_resp.model.clone();
            track.provider = followup_resp.provider.clone();
            usage = usage.add(usage2);

            draft = match OutputParser::parse_followup(&followup_resp, self.cfg.force_json) {
                Ok(d) => d,
                Err(err) => {
                    return self.err(input, track, started, LLMErrorKind::OutputParseFailed, err);
                }
            };
        }

        track.latency_ms = now_ms().saturating_sub(started);
        self.deps
            .worklog
            .emit(Event::LLMFinished {
                trace: input.trace.clone(),
                usage: usage.clone(),
                ok: true,
            })
            .await;

        LLMResult {
            status: LLMStatus::Ok,
            token_usage: usage,
            actions: draft.actions,
            output: draft.output,
            next_behavior: draft.next_behavior,
            is_sleep: draft.is_sleep,
            track,
            tool_trace,
        }
    }

    async fn create_behavior_task(&self, input: &ProcessInput) -> Result<i64, LLMComputeError> {
        let data = json!({
            "trace_id": input.trace.trace_id,
            "agent_did": input.trace.agent_did,
            "behavior": input.trace.behavior,
            "step_idx": input.trace.step_idx,
            "wakeup_id": input.trace.wakeup_id,
            "kind": "behavior",
        });
        let task = self
            .deps
            .taskmgr
            .create_task(
                &format!(
                    "LLM behavior: {}#{}@{}#{}",
                    input.trace.behavior,
                    input.trace.step_idx,
                    input.trace.wakeup_id,
                    next_task_name_suffix()
                ),
                LLM_BEHAVIOR_TASK_TYPE,
                Some(data),
                &input.trace.agent_did,
                &self.cfg.process_name,
                Some(CreateTaskOptions::default()),
            )
            .await
            .map_err(|err| LLMComputeError::Internal(err.to_string()))?;
        Ok(task.id)
    }

    async fn finalize_behavior_task(&self, behavior_task_id: i64, result: &LLMResult) {
        match &result.status {
            LLMStatus::Ok => {
                let _ = self
                    .deps
                    .taskmgr
                    .update_task(
                        behavior_task_id,
                        Some(TaskStatus::Running),
                        Some(1.0),
                        Some("behavior finished".to_string()),
                        None,
                    )
                    .await;
                let _ = self.deps.taskmgr.complete_task(behavior_task_id).await;
            }
            LLMStatus::Error(err) => {
                let _ = self
                    .deps
                    .taskmgr
                    .mark_task_as_failed(behavior_task_id, &err.message)
                    .await;
            }
        }
    }

    async fn do_inference_once(
        &self,
        input: &ProcessInput,
        allowed_tools: &[ToolSpec],
        prompt: PromptPack,
        tool_ctx: Option<ToolContext>,
        behavior_task_id: i64,
    ) -> Result<(TokenUsage, LLMRawResponse, String), LLMComputeError> {
        let task_id = self
            .create_infer_task(input, Some(behavior_task_id))
            .await?;

        let req = AiccRequestBuilder::build(
            &self.cfg,
            input,
            allowed_tools,
            prompt,
            tool_ctx,
            Some(behavior_task_id),
        );
        let _ = self
            .deps
            .taskmgr
            .update_task(
                task_id,
                Some(TaskStatus::Running),
                Some(0.1),
                Some("llm request submitted".to_string()),
                None,
            )
            .await;

        let result = self.deps.aicc.complete(req).await.map_err(map_aicc_error);

        match result {
            Ok(resp) => {
                let (usage, raw) = self
                    .resolve_aicc_complete_response(resp, &self.cfg.model_policy.preferred)
                    .await?;
                let _ = self
                    .deps
                    .taskmgr
                    .update_task(
                        task_id,
                        Some(TaskStatus::Running),
                        Some(1.0),
                        Some("llm inference finished".to_string()),
                        None,
                    )
                    .await;
                let _ = self.deps.taskmgr.complete_task(task_id).await;
                Ok((usage, raw, task_id.to_string()))
            }
            Err(err) => {
                let _ = self
                    .deps
                    .taskmgr
                    .mark_task_as_failed(task_id, &err.to_string())
                    .await;
                Err(err)
            }
        }
    }

    async fn resolve_aicc_complete_response(
        &self,
        response: CompleteResponse,
        fallback_model: &str,
    ) -> Result<(TokenUsage, LLMRawResponse), LLMComputeError> {
        match response.status {
            // AICC may return a non-empty string task_id (e.g. "aicc-...") even for
            // immediate results. For succeeded status, consume response.result directly.
            CompleteStatus::Succeeded => parse_aicc_complete_response(response, fallback_model),
            // Only running status should go through async task observation.
            CompleteStatus::Running => {
                let aicc_task_id = response.task_id.trim();
                if aicc_task_id.is_empty() {
                    return Err(LLMComputeError::Provider(
                        "aicc response status is running but task_id is empty".to_string(),
                    ));
                }
                self.wait_and_load_aicc_task_result(aicc_task_id, fallback_model)
                    .await
            }
            CompleteStatus::Failed => Err(LLMComputeError::Provider(
                "aicc complete status is Failed".to_string(),
            )),
        }
    }

    async fn wait_and_load_aicc_task_result(
        &self,
        aicc_task_id: &str,
        fallback_model: &str,
    ) -> Result<(TokenUsage, LLMRawResponse), LLMComputeError> {
        let task_id = match aicc_task_id.parse::<i64>() {
            Ok(task_id) => task_id,
            Err(_) => {
                self.resolve_aicc_task_id_from_task_manager(aicc_task_id)
                    .await?
            }
        };

        let status = self
            .deps
            .taskmgr
            .wait_for_task_end_with_interval(task_id, Duration::from_millis(500))
            .await
            .map_err(|err| LLMComputeError::Provider(err.to_string()))?;

        if status != TaskStatus::Completed {
            return Err(LLMComputeError::Provider(format!(
                "aicc task {} ended with status {:?}",
                task_id, status
            )));
        }

        let task = self
            .deps
            .taskmgr
            .get_task(task_id)
            .await
            .map_err(|err| LLMComputeError::Provider(err.to_string()))?;

        parse_aicc_result_from_task_data(task.data, fallback_model)
    }

    async fn resolve_aicc_task_id_from_task_manager(
        &self,
        aicc_task_id: &str,
    ) -> Result<i64, LLMComputeError> {
        let filter = TaskFilter {
            app_id: Some(AICC_SERVICE_SERVICE_NAME.to_string()),
            task_type: None,
            status: None,
            parent_id: None,
            root_id: None,
        };
        let tasks = self
            .deps
            .taskmgr
            .list_tasks(Some(filter), None, None)
            .await
            .map_err(|err| LLMComputeError::Provider(err.to_string()))?;

        for task in tasks {
            let matched = task
                .data
                .pointer("/aicc/external_task_id")
                .and_then(|value| value.as_str())
                .map(|value| value == aicc_task_id)
                .unwrap_or(false);
            if matched {
                return Ok(task.id);
            }
        }
        Err(LLMComputeError::Provider(format!(
            "aicc task_id '{}' is not a valid task manager id and no mapped task is found",
            aicc_task_id
        )))
    }

    async fn create_infer_task(
        &self,
        input: &ProcessInput,
        parent_id: Option<i64>,
    ) -> Result<i64, LLMComputeError> {
        let data = json!({
            "trace_id": input.trace.trace_id,
            "agent_did": input.trace.agent_did,
            "behavior": input.trace.behavior,
            "step_idx": input.trace.step_idx,
            "wakeup_id": input.trace.wakeup_id,
            "kind": "inference",
            "parent_behavior_task_id": parent_id,
        });
        let task = self
            .deps
            .taskmgr
            .create_task(
                &format!(
                    "LLM infer: {}#{}@{}#{}",
                    input.trace.behavior,
                    input.trace.step_idx,
                    input.trace.wakeup_id,
                    next_task_name_suffix()
                ),
                LLM_INFER_TASK_TYPE,
                Some(data),
                &input.trace.agent_did,
                &self.cfg.process_name,
                Some(CreateTaskOptions {
                    parent_id,
                    ..CreateTaskOptions::default()
                }),
            )
            .await
            .map_err(|err| LLMComputeError::Internal(err.to_string()))?;
        Ok(task.id)
    }

    fn err(
        &self,
        input: &ProcessInput,
        mut track: TrackInfo,
        started: u64,
        kind: LLMErrorKind,
        message: String,
    ) -> LLMResult {
        track.latency_ms = now_ms().saturating_sub(started);
        track.errors.push(message.clone());
        LLMResult {
            status: LLMStatus::Error(LLMError {
                kind,
                message,
                retriable: false,
            }),
            token_usage: TokenUsage::default(),
            actions: vec![],
            output: LLMOutput::Text(String::new()),
            next_behavior: Some(input.trace.behavior.clone()),
            is_sleep: true,
            track,
            tool_trace: vec![],
        }
    }

    fn err_from_llm(
        &self,
        input: &ProcessInput,
        track: TrackInfo,
        started: u64,
        err: LLMComputeError,
    ) -> LLMResult {
        self.err(
            input,
            track,
            started,
            LLMErrorKind::LLMComputeFailed,
            err.to_string(),
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LLMRawResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub model: String,
    pub provider: String,
    pub latency_ms: u64,
}

#[derive(thiserror::Error, Debug)]
pub enum LLMComputeError {
    #[error("llm timeout")]
    Timeout,
    #[error("llm cancelled")]
    Cancelled,
    #[error("llm provider failed: {0}")]
    Provider(String),
    #[error("llm internal error: {0}")]
    Internal(String),
}

pub trait Tokenizer: Send + Sync {
    fn count_tokens(&self, text: &str) -> u32;
}

pub struct AiccRequestBuilder;

impl AiccRequestBuilder {
    pub fn build(
        cfg: &LLMBehaviorConfig,
        input: &ProcessInput,
        allowed_tools: &[ToolSpec],
        prompt: PromptPack,
        tool_ctx: Option<ToolContext>,
        parent_task_id: Option<i64>,
    ) -> CompleteRequest {
        let mut tool_messages = Vec::new();

        if let Some(ctx) = tool_ctx {
            let assistant_msg = json!({ "tool_calls": ctx.tool_calls });
            let assistant_content =
                serde_json::to_string(&assistant_msg).unwrap_or_else(|_| "{}".to_string());
            tool_messages.push(ChatMessage {
                role: ChatRole::Assistant,
                name: Some("tool_calls".to_string()),
                content: assistant_content,
            });

            for obs in ctx.observations {
                let content = serde_json::to_string(&obs).unwrap_or_else(|_| {
                    "{\"error\":\"failed to serialize tool observation\"}".to_string()
                });
                tool_messages.push(ChatMessage {
                    role: ChatRole::Tool,
                    name: Some(obs.name),
                    content,
                });
            }
        }

        let messages = prompt
            .messages
            .iter()
            .map(|m| AiMessage::new(chat_role_to_aicc_role(&m.role), m.content.clone()))
            .collect::<Vec<_>>();

        let mut must_features = vec![features::JSON_OUTPUT.to_string()];
        if !allowed_tools.is_empty() {
            must_features.push(features::TOOL_CALLING.to_string());
        }

        let mut options = Map::new();
        options.insert(
            "protocol".to_string(),
            Json::String("opendan_llm_behavior_v1".to_string()),
        );
        options.insert(
            "process_name".to_string(),
            Json::String(cfg.process_name.clone()),
        );
        options.insert(
            "max_completion_tokens".to_string(),
            json!(input.limits.max_completion_tokens),
        );
        options.insert(
            "temperature".to_string(),
            json!(cfg.model_policy.temperature),
        );
        if let Some(schema) = cfg.response_schema.clone() {
            options.insert("response_schema".to_string(), schema);
        }
        if !allowed_tools.is_empty() {
            options.insert(
                "tools".to_string(),
                serde_json::to_value(allowed_tools).unwrap_or_else(|_| json!([])),
            );
        }
        if !tool_messages.is_empty() {
            options.insert(
                "tool_messages".to_string(),
                serde_json::to_value(tool_messages).unwrap_or_else(|_| json!([])),
            );
        }

        CompleteRequest::new(
            Capability::LlmRouter,
            ModelSpec::new(cfg.model_policy.preferred.clone(), None),
            Requirements::new(must_features, None, None, None),
            AiPayload::new(None, messages, vec![], None, Some(Json::Object(options))),
            Some(format!(
                "{}:{}:{}:{}",
                input.trace.trace_id,
                input.trace.wakeup_id,
                input.trace.behavior,
                input.trace.step_idx
            )),
        )
        .with_task_options(parent_task_id.map(|parent_id| CompleteTaskOptions {
            parent_id: Some(parent_id),
        }))
    }
}

fn chat_role_to_aicc_role(role: &ChatRole) -> String {
    match role {
        ChatRole::System => "system".to_string(),
        ChatRole::User => "user".to_string(),
        ChatRole::Assistant => "assistant".to_string(),
        ChatRole::Tool => "tool".to_string(),
    }
}

fn map_aicc_error(err: kRPC::RPCErrors) -> LLMComputeError {
    let msg = err.to_string().to_lowercase();
    if msg.contains("timeout") {
        return LLMComputeError::Timeout;
    }
    if msg.contains("cancel") {
        return LLMComputeError::Cancelled;
    }
    LLMComputeError::Provider(err.to_string())
}

fn parse_aicc_complete_response(
    response: CompleteResponse,
    fallback_model: &str,
) -> Result<(TokenUsage, LLMRawResponse), LLMComputeError> {
    if response.status != CompleteStatus::Succeeded {
        return Err(LLMComputeError::Provider(format!(
            "aicc complete status is {:?}",
            response.status
        )));
    }

    let summary = response
        .result
        .ok_or_else(|| LLMComputeError::Provider("aicc response missing result".to_string()))?;

    parse_aicc_summary(summary, fallback_model)
}

fn parse_aicc_result_from_task_data(
    data: serde_json::Value,
    fallback_model: &str,
) -> Result<(TokenUsage, LLMRawResponse), LLMComputeError> {
    if let Ok(resp) = serde_json::from_value::<CompleteResponse>(data.clone()) {
        return parse_aicc_complete_response(resp, fallback_model);
    }

    if let Some(result_value) = data.get("result") {
        if let Ok(summary) =
            serde_json::from_value::<buckyos_api::AiResponseSummary>(result_value.clone())
        {
            return parse_aicc_summary(summary, fallback_model);
        }
    }

    if let Ok(summary) = serde_json::from_value::<buckyos_api::AiResponseSummary>(data.clone()) {
        return parse_aicc_summary(summary, fallback_model);
    }

    Err(LLMComputeError::Provider(
        "task data does not contain parseable AICC completion result".to_string(),
    ))
}

fn parse_aicc_summary(
    summary: buckyos_api::AiResponseSummary,
    fallback_model: &str,
) -> Result<(TokenUsage, LLMRawResponse), LLMComputeError> {
    let usage = ai_usage_to_token_usage(summary.usage.as_ref());
    let mut model = fallback_model.to_string();
    let mut provider = "aicc".to_string();
    let mut latency_ms = 0_u64;
    let mut tool_calls = Vec::<ToolCall>::new();
    let mut content = summary.text.unwrap_or_default();

    if let Some(json_value) = summary.json {
        if let Some(tool_calls_value) = json_value.get("tool_calls") {
            tool_calls = serde_json::from_value::<Vec<ToolCall>>(tool_calls_value.clone())
                .map_err(|err| {
                    LLMComputeError::Provider(format!(
                        "failed to parse tool_calls from aicc response: {}",
                        err
                    ))
                })?;
        }

        if content.is_empty() && tool_calls.is_empty() {
            content = if let Some(text) = json_value.as_str() {
                text.to_string()
            } else {
                serde_json::to_string(&json_value).unwrap_or_default()
            };
        }
    }

    if let Some(extra) = summary.extra {
        if let Some(value) = extra.get("model").and_then(|v| v.as_str()) {
            model = value.to_string();
        }
        if let Some(value) = extra.get("provider").and_then(|v| v.as_str()) {
            provider = value.to_string();
        }
        if let Some(value) = extra.get("latency_ms").and_then(|v| v.as_u64()) {
            latency_ms = value;
        }
    }

    Ok((
        usage,
        LLMRawResponse {
            content,
            tool_calls,
            model,
            provider,
            latency_ms,
        },
    ))
}

fn ai_usage_to_token_usage(usage: Option<&buckyos_api::AiUsage>) -> TokenUsage {
    fn to_u32(v: Option<u64>) -> u32 {
        v.unwrap_or(0).min(u32::MAX as u64) as u32
    }

    if let Some(u) = usage {
        TokenUsage {
            prompt: to_u32(u.input_tokens),
            completion: to_u32(u.output_tokens),
            total: to_u32(u.total_tokens),
        }
    } else {
        TokenUsage::default()
    }
}

fn now_ms() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    now.as_millis() as u64
}

fn next_task_name_suffix() -> u64 {
    let time_high = now_ms()
        .checked_shl(LLM_TASK_NAME_SEQ_BITS)
        .unwrap_or(u64::MAX & !LLM_TASK_NAME_SEQ_MASK);
    let seq_low = LLM_TASK_NAME_SEQ.fetch_add(1, Ordering::Relaxed) & LLM_TASK_NAME_SEQ_MASK;
    time_high | seq_low
}

fn is_deadline_exceeded(started_ms: u64, deadline_ms: u64) -> bool {
    deadline_ms > 0 && now_ms().saturating_sub(started_ms) > deadline_ms
}

#[cfg(test)]
mod tests;
