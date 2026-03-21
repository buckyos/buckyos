use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(not(test))]
use buckyos_api::get_buckyos_api_runtime;
use buckyos_api::{
    value_to_object_map, AiToolCall, AiToolSpec, AiccClient, CompleteRequest, CompleteResponse,
    CompleteStatus, CompleteTaskOptions, CreateTaskOptions, TaskFilter, TaskManagerClient,
    TaskStatus, AICC_SERVICE_SERVICE_NAME,
};
use serde_json::{json, Map, Value as Json};

use super::config::{BehaviorConfig, BehaviorConfigError};
use super::observability::{AgentWorkEvent, WorklogSink};
use super::policy_adapter::PolicyEngine;
use super::prompt::{ChatMessage, ChatRole, PromptBuilder};
use super::sanitize::Sanitizer;
use super::tool_loop::{self, ToolContext};
use super::types::*;
use crate::agent_environment::AgentEnvironment;
use crate::agent_tool::AgentMemory;
use crate::agent_tool::{AgentToolManager, ToolSpec};

#[derive(Clone)]
pub struct LLMBehaviorDeps {
    pub taskmgr: Arc<TaskManagerClient>,
    #[cfg(test)]
    pub aicc: Arc<AiccClient>,
    pub tools: Arc<AgentToolManager>,
    pub memory: Option<AgentMemory>,
    pub policy: Arc<dyn PolicyEngine>,
    pub worklog: Arc<dyn WorklogSink>,
    pub tokenizer: Arc<dyn Tokenizer>,
    pub environment: Arc<AgentEnvironment>,
}

pub struct LLMBehavior {
    pub cfg: LLMBehaviorConfig,
    pub deps: LLMBehaviorDeps,
}

const LLM_BEHAVIOR_TASK_TYPE: &str = "llm_behavior";
const LLM_TASK_NAME_SEQ_BITS: u32 = 20;
const LLM_TASK_NAME_SEQ_MASK: u64 = (1_u64 << LLM_TASK_NAME_SEQ_BITS) - 1;
static LLM_TASK_NAME_SEQ: AtomicU64 = AtomicU64::new(0);

struct ToolExecutionRound {
    tool_ctx: ToolContext,
    tool_trace: Vec<ToolExecRecord>,
}

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

    pub async fn run_step(
        &self,
        input: &BehaviorExecInput,
    ) -> Result<(BehaviorLLMResult, LLMTrackingInfo), LLMComputeError> {
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
            Err(err) => return Err(err),
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

        self.deps
            .worklog
            .emit(AgentWorkEvent::LLMStarted {
                trace: input.trace.clone(),
                model: self.cfg.model_policy.preferred.clone(),
            })
            .await;

        let result = self
            .run_step_inner(&input, started, track, behavior_task_id)
            .await;

        let finish_usage = result
            .as_ref()
            .map(|(_, tracking)| tracking.token_usage.clone())
            .unwrap_or_default();

        self.deps
            .worklog
            .emit(AgentWorkEvent::LLMFinished {
                trace: input.trace.clone(),
                usage: finish_usage,
                ok: result.is_ok(),
            })
            .await;

        self.finalize_behavior_task(behavior_task_id, &result).await;
        result
    }

    async fn run_step_inner(
        &self,
        input: &BehaviorExecInput,
        started: u64,
        mut track: TrackInfo,
        behavior_task_id: i64,
    ) -> Result<(BehaviorLLMResult, LLMTrackingInfo), LLMComputeError> {
        if is_deadline_exceeded(started, input.limits.deadline_ms) {
            return Err(LLMComputeError::Timeout);
        }

        let allowed_tools = self
            .deps
            .policy
            .allowed_tools(input)
            .await
            .map_err(LLMComputeError::Internal)?;

        let ai_tool_specs: Vec<AiToolSpec> = allowed_tools
            .iter()
            .map(|tool| AiToolSpec {
                name: tool.name.clone(),
                description: tool.description.clone(),
                args_schema: value_to_object_map(tool.args_schema.clone()),
                output_schema: tool.output_schema.clone(),
            })
            .collect();

        let allowed_action_specs: Vec<ToolSpec> = allowed_tools
            .iter()
            .filter_map(|tool| self.deps.tools.get_action_tool_spec(tool.name.as_str()))
            .collect();

        let llm_req = PromptBuilder::build(
            input,
            &ai_tool_specs,
            &allowed_action_specs,
            &input.behavior_cfg,
            &*self.deps.tokenizer,
            input.session.clone(),
            self.deps.memory.clone(),
        )
        .await
        .map_err(LLMComputeError::Internal)?;

        let (mut usage, mut llm_resp, first_task_id) = self
            .do_inference_once(llm_req.clone(), None, behavior_task_id)
            .await?;

        Self::update_track_from_llm_response(&mut track, &llm_resp, first_task_id);

        let mut rounds_left = input.limits.max_tool_rounds;
        let tool_loop_enabled = input.limits.max_tool_rounds > 0;
        let mut tool_trace = Vec::new();

        while !llm_resp.tool_calls.is_empty() && rounds_left > 0 {
            rounds_left -= 1;

            if is_deadline_exceeded(started, input.limits.deadline_ms) {
                return Err(LLMComputeError::Timeout);
            }

            let executed = self
                .execute_tool_calls(input, &llm_resp.tool_calls, &mut track)
                .await?;
            tool_trace.extend(executed.tool_trace);

            let (usage2, followup_resp, followup_task_id) = self
                .do_inference_once(llm_req.clone(), Some(executed.tool_ctx), behavior_task_id)
                .await?;
            Self::update_track_from_llm_response(&mut track, &followup_resp, followup_task_id);
            llm_resp = followup_resp;
            usage = usage.add(usage2);
        }
        if tool_loop_enabled && !llm_resp.tool_calls.is_empty() && rounds_left == 0 {
            return Err(LLMComputeError::Internal(
                "tool loop exceeded max_tool_rounds".to_string(),
            ));
        }

        track.latency_ms = now_ms().saturating_sub(started);

        let behavior_result = BehaviorLLMResult::from_xml_str(&llm_resp.content)?;

        let tracking = LLMTrackingInfo {
            token_usage: usage,
            track,
            tool_trace,
            raw_output: LLMOutput::Text(llm_resp.content),
        };

        Ok((behavior_result, tracking))
    }

    fn update_track_from_llm_response(
        track: &mut TrackInfo,
        response: &LLMRawResponse,
        task_id: String,
    ) {
        if !task_id.trim().is_empty() {
            track.llm_task_ids.push(task_id);
        }
        track.model = response.model.clone();
        track.provider = response.provider.clone();
    }

    async fn execute_tool_calls(
        &self,
        input: &BehaviorExecInput,
        pending_tool_calls: &[AiToolCall],
        track: &mut TrackInfo,
    ) -> Result<ToolExecutionRound, LLMComputeError> {
        let gated_calls = self
            .deps
            .policy
            .gate_tool_calls(input, pending_tool_calls)
            .await
            .map_err(LLMComputeError::Internal)?;

        if gated_calls.len() > input.limits.max_tool_calls_per_round as usize {
            return Err(LLMComputeError::Internal(format!(
                "tool calls {} exceeds max_tool_calls_per_round {}",
                gated_calls.len(),
                input.limits.max_tool_calls_per_round
            )));
        }

        let mut tool_observations = Vec::new();
        let mut tool_trace = Vec::new();

        for call in gated_calls.clone() {
            let call_started = now_ms();
            self.deps
                .worklog
                .emit(AgentWorkEvent::ToolCallPlanned {
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
                    let raw_json = serde_json::to_value(&raw).unwrap_or_else(|_| {
                        json!({
                            "ok": false,
                            "error": "serialize tool result failed"
                        })
                    });
                    let obs = Sanitizer::sanitize_observation(
                        ObservationSource::Tool,
                        &call.name,
                        raw_json,
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
                        .emit(AgentWorkEvent::ToolCallFinished {
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
                        .emit(AgentWorkEvent::ToolCallFinished {
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

        Ok(ToolExecutionRound {
            tool_ctx: ToolContext {
                tool_calls: gated_calls,
                observations: tool_observations,
            },
            tool_trace,
        })
    }

    async fn create_behavior_task(
        &self,
        input: &BehaviorExecInput,
    ) -> Result<i64, LLMComputeError> {
        let session_id = input.session_id.clone();
        let rootid =
            resolve_rootid_for_task(input.trace.agent_name.as_str(), Some(session_id.as_str()));
        let data = json!({
            "trace_id": input.trace.trace_id,
            "agent_did": input.trace.agent_name,
            "behavior": input.trace.behavior,
            "step_idx": input.trace.step_idx,
            "wakeup_id": input.trace.wakeup_id,
            "kind": "behavior",
            "session_id": session_id.clone(),
            "owner_session_id": session_id,
            "rootid": rootid,
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
                &input.trace.agent_name,
                &self.cfg.process_name,
                Some(CreateTaskOptions::with_root_id(rootid)),
            )
            .await
            .map_err(|err| LLMComputeError::Internal(err.to_string()))?;
        Ok(task.id)
    }

    async fn finalize_behavior_task(
        &self,
        behavior_task_id: i64,
        result: &Result<(BehaviorLLMResult, LLMTrackingInfo), LLMComputeError>,
    ) {
        match result {
            Ok(_) => {
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
            Err(err) => {
                let _ = self
                    .deps
                    .taskmgr
                    .mark_task_as_failed(behavior_task_id, &err.to_string())
                    .await;
            }
        }
    }

    async fn do_inference_once(
        &self,
        base_req: CompleteRequest,
        tool_ctx: Option<ToolContext>,
        behavior_task_id: i64,
    ) -> Result<(TokenUsage, LLMRawResponse, String), LLMComputeError> {
        let req = AiccRequestBuilder::build(base_req, tool_ctx, Some(behavior_task_id));
        #[cfg(test)]
        let aicc_client = self.deps.aicc.clone();
        #[cfg(not(test))]
        let aicc_client = Self::get_aicc_client().await?;
        let resp = aicc_client.complete(req).await.map_err(map_aicc_error)?;
        let llm_task_id = resp.task_id.clone();
        let (usage, raw) = self
            .resolve_aicc_complete_response(resp, &self.cfg.model_policy.preferred)
            .await?;
        Ok((usage, raw, llm_task_id))
    }

    #[cfg(not(test))]
    async fn get_aicc_client() -> Result<Arc<AiccClient>, LLMComputeError> {
        let runtime = get_buckyos_api_runtime().map_err(|err| {
            LLMComputeError::Internal(format!("load buckyos runtime failed: {err}"))
        })?;
        let client = runtime
            .get_aicc_client()
            .await
            .map_err(|err| LLMComputeError::Provider(format!("init aicc client failed: {err}")))?;
        Ok(Arc::new(client))
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
            // FIXME(opendan-strong-typing): Weakly-typed compatibility lookup from Json is forbidden.
            // Replace with strongly-typed structs + serde deserialization.
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
}

#[derive(Clone, Debug, PartialEq)]
pub struct LLMRawResponse {
    pub content: String,
    pub tool_calls: Vec<AiToolCall>,
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
        mut base_req: CompleteRequest,
        tool_ctx: Option<ToolContext>,
        parent_task_id: Option<i64>,
    ) -> CompleteRequest {
        if let Some(ctx) = tool_ctx {
            let mut tool_messages = Vec::new();
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

            if let Some(opts) = base_req.payload.options.as_mut() {
                if let Some(obj) = opts.as_object_mut() {
                    obj.insert(
                        "tool_messages".to_string(),
                        serde_json::to_value(tool_messages).unwrap_or_else(|_| json!([])),
                    );
                }
            }
        }

        base_req.with_task_options(parent_task_id.map(|parent_id| CompleteTaskOptions {
            parent_id: Some(parent_id),
        }))
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

    // FIXME(opendan-strong-typing): Weakly-typed compatibility lookup from Json is forbidden.
    // Replace with strongly-typed structs + serde deserialization.
    if let Some(summary_value) = data.pointer("/aicc/output") {
        if let Ok(summary) =
            serde_json::from_value::<buckyos_api::AiResponseSummary>(summary_value.clone())
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
    let mut tool_calls = parse_tool_choices_from_summary(summary.tool_calls)?;
    let content = summary.text.unwrap_or_default();
    let finish_reason = summary.finish_reason.unwrap_or_default();
    let mut incomplete_reason = String::new();

    if let Some(extra) = summary.extra.as_ref() {
        if let Some(value) = extra.get("model").and_then(|v| v.as_str()) {
            model = value.to_string();
        }
        if let Some(value) = extra.get("provider").and_then(|v| v.as_str()) {
            provider = value.to_string();
        }
        if let Some(value) = extra.get("latency_ms").and_then(|v| v.as_u64()) {
            latency_ms = value;
        }
        if tool_calls.is_empty() {
            if let Some(tool_calls_value) = extra.get("tool_calls") {
                tool_calls = parse_tool_calls_from_aicc(tool_calls_value)?;
            }
        }
        if let Some(value) = extra
            .pointer("/provider_io/output/incomplete_details/reason")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            incomplete_reason = value.to_string();
        }
    }

    if content.trim().is_empty() && tool_calls.is_empty() {
        if incomplete_reason == "max_output_tokens" {
            let suffix = if finish_reason.trim().is_empty() {
                String::new()
            } else {
                format!(" (finish_reason={})", finish_reason)
            };
            return Err(LLMComputeError::Provider(format!(
                "TOKEN_LIMIT_EXCEEDED: llm token limit reached: max_output_tokens{}",
                suffix
            )));
        }
        let mut hints = Vec::new();
        if !finish_reason.trim().is_empty() {
            hints.push(format!("finish_reason={}", finish_reason));
        }
        if !incomplete_reason.is_empty() {
            hints.push(format!("incomplete_reason={}", incomplete_reason));
        }
        let hint_suffix = if hints.is_empty() {
            String::new()
        } else {
            format!(" ({})", hints.join(", "))
        };
        return Err(LLMComputeError::Provider(format!(
            "llm response is empty (no text and no tool_calls){}",
            hint_suffix
        )));
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

fn parse_tool_choices_from_summary(
    tool_choices: Vec<buckyos_api::AiToolCall>,
) -> Result<Vec<AiToolCall>, LLMComputeError> {
    Ok(tool_choices)
}

fn parse_tool_calls_from_aicc(value: &Json) -> Result<Vec<AiToolCall>, LLMComputeError> {
    if let Ok(tool_calls) = serde_json::from_value::<Vec<AiToolCall>>(value.clone()) {
        return Ok(tool_calls);
    }

    let items = value
        .as_array()
        .ok_or_else(|| LLMComputeError::Provider("tool_calls must be an array".to_string()))?;
    let mut parsed = Vec::with_capacity(items.len());

    for (idx, item) in items.iter().enumerate() {
        let item_obj = item.as_object().ok_or_else(|| {
            LLMComputeError::Provider(format!("tool_calls[{idx}] must be an object"))
        })?;

        let call_id = item_obj
            .get("call_id")
            .or_else(|| item_obj.get("id"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| {
                LLMComputeError::Provider(format!("tool_calls[{idx}].call_id is required"))
            })?
            .to_string();

        if let Some(name) = item_obj
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            let args = item_obj
                .get("args")
                .cloned()
                .unwrap_or_else(|| Json::Object(Map::new()));
            if !args.is_object() {
                return Err(LLMComputeError::Provider(format!(
                    "tool_calls[{idx}].args must be an object"
                )));
            }
            parsed.push(AiToolCall {
                name: name.to_string(),
                args: value_to_object_map(args),
                call_id,
            });
            continue;
        }

        let function = item_obj
            .get("function")
            .and_then(|v| v.as_object())
            .ok_or_else(|| {
                LLMComputeError::Provider(format!(
                    "tool_calls[{idx}] missing name/args and function payload"
                ))
            })?;
        let name = function
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| {
                LLMComputeError::Provider(format!("tool_calls[{idx}].function.name is required"))
            })?
            .to_string();

        let args_raw = function
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let args = match args_raw {
            Json::String(raw) => {
                let trimmed = raw.trim();
                if trimmed.is_empty() {
                    Json::Object(Map::new())
                } else {
                    serde_json::from_str::<Json>(trimmed).map_err(|err| {
                        LLMComputeError::Provider(format!(
                            "tool_calls[{idx}].function.arguments is invalid json: {err}"
                        ))
                    })?
                }
            }
            other => other,
        };
        if !args.is_object() {
            return Err(LLMComputeError::Provider(format!(
                "tool_calls[{idx}].function.arguments must decode to an object"
            )));
        }

        parsed.push(AiToolCall {
            name,
            args: value_to_object_map(args),
            call_id,
        });
    }

    Ok(parsed)
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

fn normalize_non_empty_str(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

fn resolve_rootid_for_task(agent_did: &str, session_id: Option<&str>) -> String {
    if let Some(session_id) = session_id.and_then(normalize_non_empty_str) {
        return session_id;
    }

    let agent_name = agent_did
        .split(|ch| ch == ':' || ch == '/' || ch == '#')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .last()
        .unwrap_or("agent");
    format!("{agent_name}#default")
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
