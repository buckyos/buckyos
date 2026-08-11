use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use url::Url;

use crate::error::AgentDIDObjectError;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectRef {
    pub raw: String,
    pub normalized: String,
}

impl ObjectRef {
    pub fn parse(input: &str) -> Result<Self, AgentDIDObjectError> {
        let raw = input.trim().to_string();
        if raw.is_empty() {
            return Err(AgentDIDObjectError::UnsupportedObjectRef(
                "empty object ref".to_string(),
            ));
        }

        let url = Url::parse(&raw).map_err(|err| {
            AgentDIDObjectError::UnsupportedObjectRef(format!(
                "object must be a canonical URL after gateway resolution: {raw}: {err}"
            ))
        })?;
        if url.cannot_be_a_base() {
            return Err(AgentDIDObjectError::UnsupportedObjectRef(format!(
                "object must be a hierarchical URL after gateway resolution: {raw}"
            )));
        }

        let normalized = url.to_string();
        Ok(Self {
            raw: normalized.clone(),
            normalized,
        })
    }

    pub fn scheme(&self) -> Option<String> {
        Url::parse(&self.normalized)
            .ok()
            .map(|url| url.scheme().to_string())
    }

    pub fn is_url(&self) -> bool {
        true
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadLineRange {
    pub offset: usize,
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReadInput {
    pub object: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default)]
    pub content_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<ReadLineRange>,
    #[serde(skip)]
    pub max_tokens: Option<usize>,
    #[serde(default)]
    pub options: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct XCallInput {
    pub object: String,
    pub action: String,
    #[serde(default)]
    pub params: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubscribeEventInput {
    pub object: String,
    pub event: String,
    #[serde(default)]
    pub filter: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EventBridgeSubscription {
    pub subscription_id: String,
    pub object: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_did: Option<String>,
    pub event: String,
    pub kevent_pattern: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    pub route: crate::router::RouteTrace,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UnsubscribeEventInput {
    pub subscription_id: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReadMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default)]
    pub extra: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PromptGuidance {
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrustGuidance {
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReadAttachedError {
    pub adapter: Option<String>,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadStrategy {
    #[default]
    PriorityFirst,
    BestEffortMerge,
    FirstSuccess,
    MergeAll,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmRenderOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_chars: Option<usize>,
}

pub fn render_json_for_llm(value: &Value, options: LlmRenderOptions) -> String {
    let rendered = match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .map(|(idx, item)| format!("- {}: {}", idx + 1, render_json_scalar_or_inline(item)))
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(map) => map
            .iter()
            .map(|(key, value)| format!("- {key}: {}", render_json_scalar_or_inline(value)))
            .collect::<Vec<_>>()
            .join("\n"),
    };
    limit_chars(rendered, options.max_chars)
}

pub fn render_xml_for_llm(input: &str, options: LlmRenderOptions) -> String {
    let mut text = String::with_capacity(input.len());
    let mut in_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                text.push(' ');
            }
            _ if !in_tag => text.push(ch),
            _ => {}
        }
    }
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    limit_chars(compact, options.max_chars)
}

pub fn render_kv_for_llm(items: impl IntoIterator<Item = (String, String)>) -> String {
    items
        .into_iter()
        .map(|(key, value)| format!("- {key}: {value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn render_json_scalar_or_inline(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(value).unwrap_or_default(),
    }
}

pub fn limit_chars(mut text: String, max_chars: Option<usize>) -> String {
    let Some(max_chars) = max_chars else {
        return text;
    };
    if text.chars().count() <= max_chars {
        return text;
    }
    text = text.chars().take(max_chars).collect();
    text.push_str("\n[truncated]");
    text
}

pub fn apply_line_range(text: &str, range: &ReadLineRange) -> String {
    let start = range.offset.saturating_sub(1);
    let lines = text.lines().skip(start);
    match range.limit {
        Some(limit) => lines.take(limit).collect::<Vec<_>>().join("\n"),
        None => lines.collect::<Vec<_>>().join("\n"),
    }
}

pub fn cmd_args_value(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string())
}

pub fn default_json_object() -> Value {
    json!({})
}
