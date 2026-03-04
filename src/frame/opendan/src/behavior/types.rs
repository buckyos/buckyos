use std::collections::HashMap;
use std::sync::Arc;

use log::warn;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use tokio::sync::Mutex;

use crate::agent_session::AgentSession;
use crate::agent_tool::DoActions;
use crate::behavior::{BehaviorConfig, LLMComputeError};

pub type InboxPack = Json;
pub type MemoryPack = Json;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionRuntimeContext {
    pub trace_id: String,
    pub agent_name: String,
    pub behavior: String,
    pub step_idx: u32,
    pub wakeup_id: String,
    pub session_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvKV {
    pub key: String,
    pub value: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
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

#[derive(Clone, Serialize, Deserialize)]
pub struct BehaviorExecInput {
    pub session_id: String,
    pub trace: SessionRuntimeContext,

    pub input_prompt: String,
    pub last_step_prompt: String,
    pub role_md: String,
    pub self_md: String,
    pub behavior_prompt: String,
    pub limits: StepLimits,
    pub behavior_cfg: BehaviorConfig,
    /// Session for template rendering ({{key}} from session values).
    #[serde(skip)]
    pub session: Option<Arc<Mutex<AgentSession>>>,
}

impl std::fmt::Debug for BehaviorExecInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BehaviorExecInput")
            .field("session_id", &self.session_id)
            .field("trace", &self.trace)
            .field("input_prompt", &self.input_prompt)
            .field("last_step_prompt", &self.last_step_prompt)
            .field("role_md", &self.role_md)
            .field("self_md", &self.self_md)
            .field("behavior_prompt", &self.behavior_prompt)
            .field("limits", &self.limits)
            .field("behavior_cfg", &self.behavior_cfg)
            .field("session", &self.session.as_ref().map(|_| "Some(_)"))
            .finish()
    }
}

impl PartialEq for BehaviorExecInput {
    fn eq(&self, other: &Self) -> bool {
        self.session_id == other.session_id
            && self.trace == other.trace
            && self.input_prompt == other.input_prompt
            && self.last_step_prompt == other.last_step_prompt
            && self.role_md == other.role_md
            && self.self_md == other.self_md
            && self.behavior_prompt == other.behavior_prompt
            && self.limits == other.limits
            && self.behavior_cfg == other.behavior_cfg
    }
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct ExecutorReply {
    pub audience: String,
    pub format: String,
    pub content: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct BehaviorLLMResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_behavior: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub topic_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "DoActions::is_empty")]
    pub actions: DoActions,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shell_commands: Vec<String>,

    // #[serde(default, skip_serializing_if = "Vec::is_empty")]
    // pub todo: Vec<Json>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub set_memory: HashMap<String, String>,
    //#[serde(default, skip_serializing_if = "Vec::is_empty")]
    //pub load_skills: Vec<String>,
    //#[serde(default, skip_serializing_if = "Vec::is_empty")]
    //pub enable_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_session: Option<(String, String)>,
}
impl BehaviorLLMResult {
    pub fn is_sleep(&self) -> bool {
        self.next_behavior.as_deref() == Some("END")
    }

    pub fn from_json_str(input: &str) -> Result<Self, LLMComputeError> {
        let normalized = input.trim();
        if let Ok(wrapper) = serde_json::from_str::<Json>(normalized) {
            if let Some(content) = extract_openai_wrapped_content(&wrapper) {
                if let Ok(parsed) = Self::from_json_str(content) {
                    return Ok(parsed);
                }
            }
        }

        let direct = serde_json::from_str::<Self>(normalized);
        if let Ok(parsed) = direct {
            return Ok(parsed);
        }

        if let Some(json_block) = extract_json_from_markdown_fence(normalized) {
            if let Ok(parsed) = serde_json::from_str::<Self>(json_block.as_str()) {
                return Ok(parsed);
            }
        }

        if let Some(merged) = parse_concatenated_json_objects(normalized) {
            if let Ok(parsed) = serde_json::from_value::<Self>(merged) {
                return Ok(parsed);
            }
        }

        let err = direct
            .err()
            .map(|v| v.to_string())
            .unwrap_or_else(|| "invalid behavior llm result".to_string());
        warn!("failed to parse BehaviorLLMResult output: {}", err);
        Err(LLMComputeError::Internal(err))
    }
}

fn extract_json_from_markdown_fence(input: &str) -> Option<String> {
    if !input.contains("```") {
        return None;
    }
    let mut parts = input.split("```");
    let _ = parts.next();
    while let Some(block) = parts.next() {
        let mut candidate = block.trim();
        if candidate.is_empty() {
            continue;
        }

        if let Some(stripped) = candidate.strip_prefix("json") {
            candidate = stripped.trim_start();
        } else if let Some(stripped) = candidate.strip_prefix("JSON") {
            candidate = stripped.trim_start();
        }

        if candidate.starts_with('{') || candidate.starts_with('[') {
            return Some(candidate.to_string());
        }
    }
    None
}

fn parse_concatenated_json_objects(input: &str) -> Option<Json> {
    let mut stream = serde_json::Deserializer::from_str(input).into_iter::<Json>();
    let mut merged = serde_json::Map::new();
    let mut count = 0usize;

    while let Some(item) = stream.next() {
        let Json::Object(object) = item.ok()? else {
            return None;
        };
        count = count.saturating_add(1);
        for (key, value) in object {
            merged.insert(key, value);
        }
    }

    if count > 1 {
        Some(Json::Object(merged))
    } else {
        None
    }
}

fn extract_openai_wrapped_content(wrapper: &Json) -> Option<&str> {
    wrapper
        .pointer("/choices/0/message/content")
        .and_then(Json::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
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
pub struct LLMTrackingInfo {
    pub token_usage: TokenUsage,
    pub track: TrackInfo,
    pub tool_trace: Vec<ToolExecRecord>,
    pub raw_output: LLMOutput,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LLMBehaviorConfig {
    pub process_name: String,
    pub model_policy: ModelPolicy,
    pub response_schema: Option<Json>,
    pub force_json: bool,
    pub output_mode: String,
    pub output_protocol: String,
}

impl Default for LLMBehaviorConfig {
    fn default() -> Self {
        Self {
            process_name: "opendan-llm-behavior".to_string(),
            model_policy: ModelPolicy::default(),
            response_schema: None,
            force_json: true,
            output_mode: "auto".to_string(),
            output_protocol: String::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ModelPolicy {
    pub preferred: String,
    pub fallback: Vec<String>,
    pub temperature: f32,
}

impl Default for ModelPolicy {
    fn default() -> Self {
        Self {
            preferred: "llm.default".to_string(),
            fallback: vec![],
            temperature: 0.2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn behavior_result_accepts_concatenated_json_objects() {
        let raw = r#"{"actions":{"mode":"all","cmds":[["create_local_workspace",{"name":"small_toy_web"}]]}}{"next_behavior":"DO:1","thinking":"done"}"#;
        let parsed =
            BehaviorLLMResult::from_json_str(raw).expect("concatenated json should be parsed");

        assert_eq!(parsed.actions.mode, "all");
        assert_eq!(parsed.actions.cmds.len(), 1);
        assert_eq!(parsed.next_behavior.as_deref(), Some("DO:1"));
        assert_eq!(parsed.thinking.as_deref(), Some("done"));
    }

    #[test]
    fn behavior_result_accepts_markdown_wrapped_json() {
        let raw = r#"```json
{"next_behavior":"END","thinking":"ok"}
```"#;
        let parsed = BehaviorLLMResult::from_json_str(raw).expect("markdown wrapped json");
        assert_eq!(parsed.next_behavior.as_deref(), Some("END"));
        assert_eq!(parsed.thinking.as_deref(), Some("ok"));
    }

    #[test]
    fn behavior_result_accepts_openai_wrapped_content_payload() {
        let raw = r#"{
  "id":"chatcmpl-test",
  "object":"chat.completion",
  "choices":[
    {
      "message":{
        "role":"assistant",
        "content":"{\"thinking\":\"switch\",\"next_behavior\":\"DO:todo=T01\"}"
      }
    }
  ]
}"#;
        let parsed =
            BehaviorLLMResult::from_json_str(raw).expect("openai wrapped content should parse");
        assert_eq!(parsed.next_behavior.as_deref(), Some("DO:todo=T01"));
        assert_eq!(parsed.thinking.as_deref(), Some("switch"));
    }
}
