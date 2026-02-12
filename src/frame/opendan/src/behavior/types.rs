use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

pub type InboxPack = Json;
pub type MemoryPack = Json;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceCtx {
    pub trace_id: String,
    pub agent_did: String,
    pub behavior: String,
    pub step_idx: u32,
    pub wakeup_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvKV {
    pub key: String,
    pub value: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StepLimits {
    pub max_prompt_tokens: u32,
    pub max_completion_tokens: u32,
    pub max_tool_rounds: u8,
    pub max_tool_calls_per_round: u16,
    pub max_observation_bytes: usize,
    pub deadline_ms: u64,
}

impl Default for StepLimits {
    fn default() -> Self {
        Self {
            max_prompt_tokens: 12_000,
            max_completion_tokens: 2_000,
            max_tool_rounds: 1,
            max_tool_calls_per_round: 8,
            max_observation_bytes: 32 * 1024,
            deadline_ms: 30_000,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProcessInput {
    pub trace: TraceCtx,
    pub role_md: String,
    pub self_md: String,
    pub behavior_prompt: String,
    pub env_context: Vec<EnvKV>,
    pub inbox: InboxPack,
    pub memory: MemoryPack,
    pub last_observations: Vec<Observation>,
    pub limits: StepLimits,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub prompt: u32,
    pub completion: u32,
    pub total: u32,
}

impl TokenUsage {
    pub fn add(self, other: TokenUsage) -> TokenUsage {
        TokenUsage {
            prompt: self.prompt.saturating_add(other.prompt),
            completion: self.completion.saturating_add(other.completion),
            total: self.total.saturating_add(other.total),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum LLMOutput {
    Json(Json),
    Text(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LLMErrorKind {
    Timeout,
    Cancelled,
    LLMComputeFailed,
    PromptBuildFailed,
    OutputParseFailed,
    ToolDenied,
    ToolExecFailed,
    ToolLoopExceeded,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LLMError {
    pub kind: LLMErrorKind,
    pub message: String,
    pub retriable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LLMStatus {
    Ok,
    Error(LLMError),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Bash,
}

impl Default for ActionKind {
    fn default() -> Self {
        Self::Bash
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionExecutionMode {
    Serial,
    Parallel,
}

impl Default for ActionExecutionMode {
    fn default() -> Self {
        Self::Serial
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsScope {
    pub read_roots: Vec<String>,
    pub write_roots: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionSpec {
    #[serde(default)]
    pub kind: ActionKind,
    pub title: String,
    pub command: String,
    #[serde(default)]
    pub execution_mode: ActionExecutionMode,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default = "default_action_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub allow_network: bool,
    #[serde(default)]
    pub fs_scope: FsScope,
    #[serde(default)]
    pub rationale: String,
}

fn default_action_timeout_ms() -> u64 {
    120_000
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ObservationSource {
    Tool,
    Action,
    System,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Observation {
    pub source: ObservationSource,
    pub name: String,
    pub content: Json,
    pub ok: bool,
    pub truncated: bool,
    pub bytes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolExecRecord {
    pub tool_name: String,
    pub call_id: String,
    pub ok: bool,
    pub duration_ms: u64,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackInfo {
    pub trace_id: String,
    pub model: String,
    pub provider: String,
    pub latency_ms: u64,
    pub llm_task_ids: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LLMResult {
    pub status: LLMStatus,
    pub token_usage: TokenUsage,
    pub actions: Vec<ActionSpec>,
    pub output: LLMOutput,
    pub next_behavior: Option<String>,
    pub is_sleep: bool,
    pub track: TrackInfo,
    pub tool_trace: Vec<ToolExecRecord>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LLMBehaviorConfig {
    pub process_name: String,
    pub model_policy: ModelPolicy,
    pub response_schema: Option<Json>,
    pub force_json: bool,
}

impl Default for LLMBehaviorConfig {
    fn default() -> Self {
        Self {
            process_name: "opendan-llm-behavior".to_string(),
            model_policy: ModelPolicy::default(),
            response_schema: None,
            force_json: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModelPolicy {
    pub preferred: String,
    pub fallback: Vec<String>,
    pub temperature: f32,
}

impl Default for ModelPolicy {
    fn default() -> Self {
        Self {
            preferred: "default".to_string(),
            fallback: vec![],
            temperature: 0.2,
        }
    }
}
