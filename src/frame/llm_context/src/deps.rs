//! Runtime dependencies injected into an LLMContext.
//!
//! These are the "syscall surface" — provider abstractions, the tool
//! manager, the policy gate, the worklog sink, and a tokenizer. waist only
//! sees the traits; concrete implementations live in higher layers
//! (Agent / Workflow / OneShot schedulers).

use std::sync::Arc;

use async_trait::async_trait;
use buckyos_api::{AiMessage, AiResponseSummary, AiToolCall};
use serde_json::Value;

use crate::error::LLMComputeError;
use crate::observation::Observation;
use crate::request::{LLMContextRequest, ToolPolicy};

/// One inference request sent down to the provider adapter.
#[derive(Debug, Clone)]
pub struct LlmInferenceRequest {
    pub messages: Vec<AiMessage>,
    pub model_alias: String,
    pub fallbacks: Vec<String>,
    pub temperature: Option<f32>,
    pub max_completion_tokens: Option<u32>,
    pub force_json: bool,
    pub json_schema: Option<Value>,
    pub provider_options: Option<Value>,
    /// Tool catalogue the adapter may advertise to the provider. Empty when
    /// `tool_policy.mode == None` or no tools are available.
    pub tool_specs: Vec<ToolSpecLite>,
    pub allow_tool_calls: bool,
}

/// Provider-agnostic tool descriptor passed in inference requests. Trimmed
/// version of `agent_tool::ToolSpec` so the waist trait surface does not
/// expand whenever upstream extends its own spec.
#[derive(Debug, Clone)]
pub struct ToolSpecLite {
    pub name: String,
    pub description: String,
    pub args_schema: Value,
}

/// LLM provider boundary. One call ⇒ one inference. Provider-internal
/// retry / fallback happens *inside* the adapter, not in the waist loop.
#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn infer(
        &self,
        req: LlmInferenceRequest,
    ) -> Result<AiResponseSummary, LLMComputeError>;
}

/// Effect-side dispatcher. Implementations bridge to whatever tool
/// substrate the scheduler owns (Agent tool manager, MCP, sandbox, ...).
#[async_trait]
pub trait ToolManager: Send + Sync {
    /// Run one tool call and return a normalised observation.
    async fn call_tool(&self, call: AiToolCall) -> Observation;

    /// Specs advertised to the LLM. Returning empty is fine — callers can
    /// also disable tool dispatch via `ToolPolicy.mode = None`.
    fn list_tool_specs(&self) -> Vec<ToolSpecLite> {
        Vec::new()
    }

    /// Quick existence check. Default falls back to scanning `list_tool_specs`.
    fn has_tool(&self, name: &str) -> bool {
        self.list_tool_specs().iter().any(|spec| spec.name == name)
    }
}

/// Policy gate. Filters tool calls before dispatch. Implementations can
/// also raise approval requirements; the waist treats a rejection as a
/// Recoverable error and routes through `ErrorPolicy`.
#[async_trait]
pub trait PolicyEngine: Send + Sync {
    async fn gate_tool_calls(
        &self,
        request: &LLMContextRequest,
        calls: Vec<AiToolCall>,
    ) -> Result<Vec<AiToolCall>, String>;
}

/// Worklog event sink. The schema of `WorkEvent` is intentionally
/// opaque — schedulers carry their own audit shapes. waist only ever
/// emits one of the variants below; downstream sinks translate.
#[async_trait]
pub trait WorklogSink: Send + Sync {
    async fn emit(&self, event: WorkEvent);
}

#[derive(Debug, Clone)]
pub enum WorkEvent {
    LLMStarted {
        trace_id: Option<String>,
        model: String,
    },
    LLMFinished {
        trace_id: Option<String>,
        ok: bool,
    },
    LLMInferenceFailed {
        trace_id: Option<String>,
        error: String,
    },
    ToolCallPlanned {
        trace_id: Option<String>,
        tool: String,
        call_id: String,
    },
    ToolCallFinished {
        trace_id: Option<String>,
        tool: String,
        call_id: String,
        ok: bool,
        duration_ms: u64,
    },
    ToolCallFailed {
        trace_id: Option<String>,
        tool: String,
        call_id: String,
        message: String,
    },
    OutputParseFailed {
        trace_id: Option<String>,
        error: String,
    },
    ContextRewritten {
        trace_id: Option<String>,
        from_messages: usize,
        to_messages: usize,
    },
}

/// Cheap token estimator. Implementations can wrap a tokeniser library or
/// fall back to byte-length heuristics; waist only uses this for budget /
/// threshold checks, not for billing.
pub trait Tokenizer: Send + Sync {
    fn count_tokens(&self, text: &str) -> u32;
}

/// No-op worklog sink. Useful for tests and `OneShot` scenarios.
pub struct NoopWorklogSink;

#[async_trait]
impl WorklogSink for NoopWorklogSink {
    async fn emit(&self, _event: WorkEvent) {}
}

/// Trivial pass-through policy. Lets every tool call go through.
pub struct AllowAllPolicy;

#[async_trait]
impl PolicyEngine for AllowAllPolicy {
    async fn gate_tool_calls(
        &self,
        _request: &LLMContextRequest,
        calls: Vec<AiToolCall>,
    ) -> Result<Vec<AiToolCall>, String> {
        Ok(calls)
    }
}

/// Byte-length heuristic tokenizer (1 token ≈ 4 bytes).
pub struct ByteHeuristicTokenizer;

impl Tokenizer for ByteHeuristicTokenizer {
    fn count_tokens(&self, text: &str) -> u32 {
        ((text.len() as u64 + 3) / 4).min(u32::MAX as u64) as u32
    }
}

/// Bundle of runtime deps for one LLMContext run. Cloning shares the inner
/// Arcs.
#[derive(Clone)]
pub struct LLMContextDeps {
    pub llm: Arc<dyn LlmClient>,
    pub tools: Arc<dyn ToolManager>,
    pub policy: Arc<dyn PolicyEngine>,
    pub worklog: Arc<dyn WorklogSink>,
    pub tokenizer: Arc<dyn Tokenizer>,
}

impl LLMContextDeps {
    pub fn new(llm: Arc<dyn LlmClient>, tools: Arc<dyn ToolManager>) -> Self {
        Self {
            llm,
            tools,
            policy: Arc::new(AllowAllPolicy),
            worklog: Arc::new(NoopWorklogSink),
            tokenizer: Arc::new(ByteHeuristicTokenizer),
        }
    }

    pub fn with_policy(mut self, policy: Arc<dyn PolicyEngine>) -> Self {
        self.policy = policy;
        self
    }

    pub fn with_worklog(mut self, worklog: Arc<dyn WorklogSink>) -> Self {
        self.worklog = worklog;
        self
    }

    pub fn with_tokenizer(mut self, tokenizer: Arc<dyn Tokenizer>) -> Self {
        self.tokenizer = tokenizer;
        self
    }
}

/// Resolve the tool specs the adapter should advertise on each inference,
/// taking `ToolPolicy.mode` / `whitelist` into account.
pub fn resolve_tool_specs(policy: &ToolPolicy, tools: &dyn ToolManager) -> Vec<ToolSpecLite> {
    use crate::request::ToolMode;
    match policy.mode {
        ToolMode::None => Vec::new(),
        ToolMode::All => tools.list_tool_specs(),
        ToolMode::Whitelist => tools
            .list_tool_specs()
            .into_iter()
            .filter(|spec| policy.whitelist.iter().any(|w| w == &spec.name))
            .collect(),
    }
}
