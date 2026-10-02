//! Exact-count tokenizer, scripted LLM / tools and request builders of the X7 tests.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use buckyos_api::{AiContent, AiMessage, AiResponse, AiRole, AiToolCall, AiUsage};
use serde_json::json;

use crate::deps::{
    LLMContextDeps,
    LlmClient,
    LlmInferenceRequest,
    Tokenizer,
    ToolDispatchError,
    ToolManager,
};
use crate::error::{LLMComputeError};
use crate::observation::{Observation};
use crate::request::{
    ContextOwnerRef,
    ContextThreshold,
    LLMContextRequest,
    ModelPolicy,
    OutputSpec,
    ToolMode,
    ToolPolicy,
};
use crate::state::{LLMContextSnapshot};
use crate::{LLMContext, XmlBehaviorParser, XmlStepRenderer};

/// One token per char: estimates are exact in tests.
pub(super) struct CharTokenizer;

impl Tokenizer for CharTokenizer {
    fn count_tokens(&self, text: &str) -> u32 {
        text.chars().count() as u32
    }
}

/// Scripted responses / errors; records every request's messages.
pub(super) struct Llm {
    script: Mutex<Vec<Result<AiResponse, LLMComputeError>>>,
    seen: Mutex<Vec<Vec<AiMessage>>>,
}

impl Llm {
    pub(super) fn new(script: Vec<AiResponse>) -> Arc<Self> {
        Self::with_results(script.into_iter().map(Ok).collect())
    }

    pub(super) fn with_results(script: Vec<Result<AiResponse, LLMComputeError>>) -> Arc<Self> {
        Arc::new(Self {
            script: Mutex::new(script),
            seen: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn seen(&self) -> Vec<Vec<AiMessage>> {
        self.seen.lock().unwrap().clone()
    }

    pub(super) fn calls(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
}

#[async_trait]
impl LlmClient for Llm {
    async fn infer(&self, req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
        self.seen.lock().unwrap().push(req.messages);
        let mut guard = self.script.lock().unwrap();
        if guard.is_empty() {
            return Err(LLMComputeError::Internal("script empty".into()));
        }
        guard.remove(0)
    }
}

/// A call whose name or arguments mention `defer` is deferred, `fail` is a
/// business error, `big` returns a large result; everything else succeeds.
/// Records every dispatched call id.
pub(super) struct Tools {
    calls: Mutex<Vec<String>>,
}

impl Tools {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl ToolManager for Tools {
    async fn call_tool(&self, call: AiToolCall) -> Result<Observation, ToolDispatchError> {
        self.calls.lock().unwrap().push(call.call_id.clone());
        let text = format!(
            "{} {}",
            call.name,
            serde_json::to_string(&call.args).unwrap_or_default()
        );
        if text.contains("defer") {
            return Ok(Observation::Pending {
                call_id: call.call_id,
                tool_result: None,
            });
        }
        if text.contains("fail") {
            return Ok(Observation::Error {
                call_id: call.call_id,
                message: "bad args".into(),
                tool_result: None,
            });
        }
        let content = if text.contains("big") {
            json!("x".repeat(400))
        } else {
            json!(format!("ok {}", call.call_id))
        };
        Ok(Observation::Success {
            call_id: call.call_id,
            content,
            bytes: 0,
            truncated: false,
            tool_result: None,
        })
    }
}

pub(super) fn request() -> LLMContextRequest {
    LLMContextRequest {
        owner: ContextOwnerRef::OneShot { id: "x7".into() },
        trace: Some("x7".into()),
        objective: "x7".into(),
        behavior_name: String::new(),
        input: vec![AiMessage::text(AiRole::User, "hello")],
        model_policy: ModelPolicy {
            preferred: "m".into(),
            ..ModelPolicy::default()
        },
        tool_policy: ToolPolicy {
            mode: ToolMode::All,
            max_tool_iterations: 6,
            max_calls_per_round: 6,
            allow_deferred: true,
            ..ToolPolicy::default()
        },
        output: OutputSpec::Text,
        budget: Default::default(),
        human_policy: Default::default(),
        error_policy: Default::default(),
        forbid_next_behavior: false,
    }
}

pub(super) fn behavior_request() -> LLMContextRequest {
    let mut r = request();
    r.behavior_name = "do".into();
    r
}

pub(super) fn deps(llm: Arc<Llm>, tools: Arc<Tools>) -> LLMContextDeps {
    LLMContextDeps::new(llm, tools).with_tokenizer(Arc::new(CharTokenizer))
}

pub(super) fn behavior_deps(llm: Arc<Llm>, tools: Arc<Tools>) -> LLMContextDeps {
    deps(llm, tools)
        .with_result_parser(Arc::new(XmlBehaviorParser::new()))
        .with_step_renderer(Arc::new(XmlStepRenderer::new().without_timestamps()))
}

pub(super) fn call(name: &str, id: &str) -> AiToolCall {
    AiToolCall {
        name: name.into(),
        args: HashMap::new(),
        call_id: id.into(),
    }
}

pub(super) fn tools_response(calls: Vec<AiToolCall>) -> AiResponse {
    AiResponse::from_parts(None, calls, vec![])
}

pub(super) fn with_usage(mut r: AiResponse, total: u64) -> AiResponse {
    r.usage = Some(AiUsage {
        total_tokens: Some(total),
        ..Default::default()
    });
    r
}

/// Snapshots cross processes as JSON.
pub(super) fn round_trip(s: &LLMContextSnapshot) -> LLMContextSnapshot {
    serde_json::from_str(&serde_json::to_string(s).unwrap()).unwrap()
}

pub(super) fn success(id: &str, text: &str) -> (String, Observation) {
    (
        id.to_string(),
        Observation::Success {
            call_id: id.into(),
            content: json!(text),
            bytes: 0,
            truncated: false,
            tool_result: None,
        },
    )
}

pub(super) fn tool_result_ids(messages: &[AiMessage]) -> Vec<String> {
    messages
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|c| match c {
            AiContent::ToolResult { call_id, .. } => Some(call_id.clone()),
            _ => None,
        })
        .collect()
}

pub(super) fn corrupted(r: Result<LLMContext, LLMComputeError>) -> String {
    match r {
        Err(LLMComputeError::SnapshotCorrupted(m)) => m,
        Err(e) => panic!("expected SnapshotCorrupted, got {e:?}"),
        Ok(_) => panic!("expected SnapshotCorrupted, resume succeeded"),
    }
}

pub(super) fn threshold(value: u32) -> Option<ContextThreshold> {
    Some(ContextThreshold::AbsoluteTokens { value })
}

// "hello" as the only message: 4 (overhead) + 5 = 9 tokens.
pub(super) const HELLO_TOKENS: u32 = 9;
