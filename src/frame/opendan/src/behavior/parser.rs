use serde::Deserialize;
use serde_json::Value as Json;

use super::types::{ActionSpec, LLMOutput};
use super::LLMRawResponse;
use crate::agent_tool::ToolCall;

#[derive(Clone, Debug, PartialEq)]
pub struct StepDraft {
    pub tool_calls: Vec<ToolCall>,
    pub actions: Vec<ActionSpec>,
    pub output: LLMOutput,
    pub next_behavior: Option<String>,
    pub is_sleep: bool,
}

pub struct OutputParser;

impl OutputParser {
    pub fn parse_first(raw: &LLMRawResponse, force_json: bool) -> Result<StepDraft, String> {
        if !raw.tool_calls.is_empty() {
            return Ok(StepDraft {
                tool_calls: raw.tool_calls.clone(),
                actions: vec![],
                output: LLMOutput::Text(String::new()),
                next_behavior: None,
                is_sleep: false,
            });
        }
        Self::parse_final_content(&raw.content, force_json)
    }

    pub fn parse_followup(raw: &LLMRawResponse, force_json: bool) -> Result<StepDraft, String> {
        if !raw.tool_calls.is_empty() {
            return Ok(StepDraft {
                tool_calls: raw.tool_calls.clone(),
                actions: vec![],
                output: LLMOutput::Text(String::new()),
                next_behavior: None,
                is_sleep: false,
            });
        }
        Self::parse_final_content(&raw.content, force_json)
    }

    fn parse_final_content(content: &str, force_json: bool) -> Result<StepDraft, String> {
        if let Ok(value) = serde_json::from_str::<Json>(content) {
            return Self::from_json(value);
        }

        if force_json {
            if let Some(extracted) = try_extract_json_block(content) {
                let value = serde_json::from_str::<Json>(&extracted)
                    .map_err(|err| format!("json extract ok but parse failed: {err}"))?;
                return Self::from_json(value);
            }
            return Err("force_json enabled but failed to parse JSON".to_string());
        }

        Ok(StepDraft {
            tool_calls: vec![],
            actions: vec![],
            output: LLMOutput::Text(content.to_string()),
            next_behavior: None,
            is_sleep: false,
        })
    }

    fn from_json(value: Json) -> Result<StepDraft, String> {
        let payload: FinalPayload =
            serde_json::from_value(value).map_err(|err| format!("invalid output schema: {err}"))?;

        let output: LLMOutput = payload.output.into();

        Ok(StepDraft {
            tool_calls: vec![],
            actions: payload.actions,
            output,
            next_behavior: payload.next_behavior,
            is_sleep: payload.is_sleep,
        })
    }
}

#[derive(Clone, Debug, Deserialize)]
struct FinalPayload {
    #[serde(default)]
    pub next_behavior: Option<String>,
    #[serde(default)]
    pub is_sleep: bool,
    #[serde(default)]
    pub actions: Vec<ActionSpec>,
    #[serde(default)]
    pub output: OutputPayload,
}

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(untagged)]
enum OutputPayload {
    Text(String),
    #[default]
    JsonNull,
    Json(Json),
}

impl OutputPayload {
    fn into_llm_output(self) -> LLMOutput {
        match self {
            OutputPayload::Text(text) => LLMOutput::Text(text),
            OutputPayload::Json(value) => LLMOutput::Json(value),
            OutputPayload::JsonNull => LLMOutput::Json(Json::Null),
        }
    }
}

impl From<OutputPayload> for LLMOutput {
    fn from(value: OutputPayload) -> Self {
        value.into_llm_output()
    }
}

fn try_extract_json_block(content: &str) -> Option<String> {
    let fence_parts: Vec<&str> = content.split("```").collect();
    if fence_parts.len() >= 3 {
        for segment in fence_parts.iter().skip(1).step_by(2) {
            let trimmed = segment.trim();
            let payload = if let Some(rest) = trimmed.strip_prefix("json") {
                rest.trim()
            } else {
                trimmed
            };
            if serde_json::from_str::<Json>(payload).is_ok() {
                return Some(payload.to_string());
            }
        }
    }

    let start = content.find('{')?;
    let end = content.rfind('}')?;
    if end <= start {
        return None;
    }

    let candidate = &content[start..=end];
    if serde_json::from_str::<Json>(candidate).is_ok() {
        return Some(candidate.to_string());
    }

    None
}
