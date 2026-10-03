//! Scripted LLM clients, tool managers and hooks shared by the loop tests.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use buckyos_api::{AiContent, AiMessage, AiResponse, AiRole, AiToolCall};
use serde_json::json;

use crate::deps::{
    ToolCallCtx,
    InferenceHook,
    LlmClient,
    LlmInferenceRequest,
    ToolDispatchError,
    ToolManager,
    ToolSpecLite,
};
use crate::error::{LLMComputeError, ProviderFailure};
use crate::observation::{Observation, ToolExecStatus};
use crate::request::{
    ContextOwnerRef,
    LLMContextRequest,
    ModelPolicy,
    OutputSpec,
    ToolMode,
    ToolPolicy,
};
use crate::state::{LLMContextSnapshot};
use crate::{StepResultHook, StepResultHookOutput};

/// Scripted LLM responses popped off in order.
pub(super) struct ScriptedLlm {
    script: Mutex<Vec<AiResponse>>,
}

impl ScriptedLlm {
    pub(super) fn new(script: Vec<AiResponse>) -> Self {
        Self {
            script: Mutex::new(script),
        }
    }
}

#[async_trait]
impl LlmClient for ScriptedLlm {
    async fn infer(&self, _req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
        let mut guard = self.script.lock().unwrap();
        if guard.is_empty() {
            return Err(LLMComputeError::Internal("script empty".into()));
        }
        Ok(guard.remove(0))
    }
}

pub(super) struct RecordingLlm {
    response: AiResponse,
    seen: Mutex<Vec<Vec<AiMessage>>>,
}

impl RecordingLlm {
    pub(super) fn new(response: AiResponse) -> Self {
        Self {
            response,
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn seen(&self) -> Vec<Vec<AiMessage>> {
        self.seen.lock().unwrap().clone()
    }
}

#[async_trait]
impl LlmClient for RecordingLlm {
    async fn infer(&self, req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
        self.seen.lock().unwrap().push(req.messages);
        Ok(self.response.clone())
    }
}

pub(super) struct RecoverOnceLlm {
    response: AiResponse,
    seen: Mutex<Vec<Vec<AiMessage>>>,
    failed: Mutex<bool>,
}

impl RecoverOnceLlm {
    pub(super) fn new(response: AiResponse) -> Self {
        Self {
            response,
            seen: Mutex::new(Vec::new()),
            failed: Mutex::new(false),
        }
    }

    pub(super) fn seen(&self) -> Vec<Vec<AiMessage>> {
        self.seen.lock().unwrap().clone()
    }
}

#[async_trait]
impl LlmClient for RecoverOnceLlm {
    async fn infer(&self, req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
        self.seen.lock().unwrap().push(req.messages);
        let mut failed = self.failed.lock().unwrap();
        if !*failed {
            *failed = true;
            return Err(LLMComputeError::provider(
                ProviderFailure::Transient,
                "temporary aicc failure",
            ));
        }
        Ok(self.response.clone())
    }
}

pub(super) struct EchoTools;

#[async_trait]
impl ToolManager for EchoTools {
    async fn call_tool(&self, call: AiToolCall, _ctx: ToolCallCtx) -> Result<Observation, ToolDispatchError> {
        let value = serde_json::to_value(&call.args).unwrap_or(serde_json::Value::Null);
        Ok(Observation::Success {
            call_id: call.call_id,
            content: json!({ "echo": value }),
            bytes: 0,
            truncated: false,
            tool_result: None,
        })
    }

    fn list_tool_specs(&self) -> Vec<ToolSpecLite> {
        vec![ToolSpecLite {
            name: "echo".into(),
            description: "echo the args".into(),
            args_schema: json!({}),
        }]
    }
}

pub(super) struct FailingTools;

#[async_trait]
impl ToolManager for FailingTools {
    async fn call_tool(&self, call: AiToolCall, _ctx: ToolCallCtx) -> Result<Observation, ToolDispatchError> {
        Ok(Observation::Error {
            call_id: call.call_id,
            message: "boom".into(),
            tool_result: None,
        })
    }

    fn list_tool_specs(&self) -> Vec<ToolSpecLite> {
        Vec::new()
    }
}

pub(super) struct CustomStepResultHook;

#[async_trait]
impl StepResultHook for CustomStepResultHook {
    async fn on_behavior_step_ob(
        &self,
        _snapshot: &LLMContextSnapshot,
        step: &crate::behavior_loop::StepRecord,
    ) -> Result<StepResultHookOutput, String> {
        Ok(StepResultHookOutput {
            user_message: Some(AiMessage::text(
                AiRole::User,
                format!("custom step result {}", step.meta.step_index),
            )),
            history_inputs: Vec::new(),
            skip_next_inference: false,
        })
    }
}

pub(super) struct SkipStepResultHook;

#[async_trait]
impl StepResultHook for SkipStepResultHook {
    async fn on_behavior_step_ob(
        &self,
        _snapshot: &LLMContextSnapshot,
        _step: &crate::behavior_loop::StepRecord,
    ) -> Result<StepResultHookOutput, String> {
        Ok(StepResultHookOutput {
            skip_next_inference: true,
            ..Default::default()
        })
    }
}

pub(super) fn base_request() -> LLMContextRequest {
    LLMContextRequest {
        owner: ContextOwnerRef::OneShot { id: "t".into() },
        trace: Some("trace-1".into()),
        objective: "test".into(),
        behavior_name: String::new(),
        input: vec![AiMessage::text(AiRole::User, "hello")],
        model_policy: ModelPolicy {
            preferred: "test-model".into(),
            ..ModelPolicy::default()
        },
        tool_policy: ToolPolicy {
            mode: ToolMode::All,
            max_tool_iterations: 4,
            max_calls_per_round: 4,
            ..ToolPolicy::default()
        },
        output: OutputSpec::Text,
        budget: Default::default(),
        human_policy: Default::default(),
        error_policy: Default::default(),
        forbid_next_behavior: false,
    }
}

pub(super) fn text_response(text: &str) -> AiResponse {
    AiResponse::text(text)
}

pub(super) fn tool_response(text: Option<&str>, calls: Vec<AiToolCall>) -> AiResponse {
    AiResponse::from_parts(text.map(str::to_string), calls, vec![])
}

/// Scripted responses / errors, recording every request like `RecordingLlm`.
pub(super) struct ScriptedRecordingLlm {
    script: Mutex<Vec<Result<AiResponse, LLMComputeError>>>,
    seen: Mutex<Vec<Vec<AiMessage>>>,
}

impl ScriptedRecordingLlm {
    pub(super) fn new(script: Vec<AiResponse>) -> Self {
        Self::with_results(script.into_iter().map(Ok).collect())
    }

    pub(super) fn with_results(script: Vec<Result<AiResponse, LLMComputeError>>) -> Self {
        Self {
            script: Mutex::new(script),
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn seen(&self) -> Vec<Vec<AiMessage>> {
        self.seen.lock().unwrap().clone()
    }
}

#[async_trait]
impl LlmClient for ScriptedRecordingLlm {
    async fn infer(&self, req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
        self.seen.lock().unwrap().push(req.messages);
        let mut guard = self.script.lock().unwrap();
        if guard.is_empty() {
            return Err(LLMComputeError::Internal("script empty".into()));
        }
        guard.remove(0)
    }
}

/// Tool manager scripted by tool name: `fail:*` is a business error,
/// `dispatch:*` is an infrastructure failure with unknown effect, anything
/// else succeeds. Counts every dispatch attempt.
pub(super) struct ScriptedTools {
    calls: Mutex<Vec<String>>,
}

impl ScriptedTools {
    pub(super) fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl ToolManager for ScriptedTools {
    async fn call_tool(&self, call: AiToolCall, _ctx: ToolCallCtx) -> Result<Observation, ToolDispatchError> {
        self.calls.lock().unwrap().push(call.name.clone());
        if call.name.starts_with("dispatch:") {
            return Err(ToolDispatchError::effect_unknown("sandbox connection lost"));
        }
        if call.name.starts_with("fail:") {
            return Ok(Observation::Error {
                call_id: call.call_id,
                message: "bad args".to_string(),
                tool_result: None,
            });
        }
        Ok(Observation::Success {
            call_id: call.call_id,
            content: json!("ok"),
            bytes: 2,
            truncated: false,
            tool_result: None,
        })
    }

    fn list_tool_specs(&self) -> Vec<ToolSpecLite> {
        ["a", "fail:b", "dispatch:b", "c", "shell"]
            .iter()
            .map(|name| ToolSpecLite {
                name: (*name).to_string(),
                description: String::new(),
                args_schema: json!({}),
            })
            .collect()
    }
}

pub(super) fn call(name: &str, id: &str) -> AiToolCall {
    AiToolCall {
        name: name.into(),
        args: HashMap::new(),
        call_id: id.into(),
    }
}

pub(super) fn tool_result_text(message: &AiMessage) -> String {
    message
        .content
        .iter()
        .filter_map(|block| match block {
            AiContent::ToolResult { content, .. } => Some(
                content
                    .iter()
                    .filter_map(|part| match part {
                        buckyos_api::AiToolResultContent::Text { text } => Some(text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn statuses(trace: &crate::outcome::ContextRunTrace) -> Vec<(String, ToolExecStatus)> {
    trace
        .tool_trace
        .iter()
        .map(|r| (r.tool_name.clone(), r.status))
        .collect()
}

pub(super) struct CountingHook {
    pub(super) count: Arc<Mutex<u32>>,
}

impl InferenceHook for CountingHook {
    fn before_inference(&self, _snapshot: &LLMContextSnapshot) -> Result<(), String> {
        *self.count.lock().unwrap() += 1;
        Ok(())
    }
}
