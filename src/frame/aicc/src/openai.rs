use crate::aicc::{
    AIComputeCenter, CostEstimate, Provider, ProviderError, ProviderInstance, ProviderStartResult,
    ResolvedRequest, TaskEventSink,
};
use crate::openai_protocol::{
    merge_options, merge_requirements_response_format, merge_tool_calls,
    strip_incompatible_sampling_options,
};
use ::kRPC::{RPCSessionToken, RPCSessionTokenType};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use base64::engine::general_purpose;
use base64::Engine as _;
use buckyos_api::{
    features, value_to_object_map, AiArtifact, AiCost, AiResponseSummary, AiToolCall, AiUsage,
    Capability, CompleteRequest, Feature, ResourceRef,
};
use buckyos_kit::{buckyos_get_unix_timestamp, get_buckyos_system_etc_dir};
use log::{error, info, warn};
use name_lib::load_private_key;
use reqwest::header::{CONTENT_ENCODING, CONTENT_TYPE};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::error::Error as _;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_OPENAI_TIMEOUT_MS: u64 = 300_000;
const DEFAULT_OPENAI_MODELS: &str = "gpt-5,gpt-5-mini,gpt-5-nono,gpt-5-pro";
const DEFAULT_OPENAI_IMAGE_MODELS: &str = "dall-e-3,dall-e-2";
const DEFAULT_AUTH_MODE: &str = "bearer";
const DEVICE_JWT_AUTH_MODE: &str = "device_jwt";
const DEFAULT_DEVICE_AUTH_APP_ID: &str = "aicc";
const OPENAI_TOOL_TYPE_WEB_SEARCH: &str = "web_search_preview";
const OPENAI_IMAGE_OPTION_ALLOWLIST: &[&str] = &[
    "background",
    "n",
    "output_compression",
    "output_format",
    "quality",
    "response_format",
    "size",
    "style",
    "user",
];
const OPENAI_IMAGE_INPUT_ALLOWLIST: &[&str] = &[
    "background",
    "n",
    "output_compression",
    "output_format",
    "prompt",
    "quality",
    "response_format",
    "size",
    "style",
    "user",
];

#[derive(Debug, Clone)]
pub struct OpenAIInstanceConfig {
    pub instance_id: String,
    pub provider_type: String,
    pub base_url: String,
    pub auth_mode: String,
    pub api_token: Option<String>,
    pub auth_subject: Option<String>,
    pub auth_appid: Option<String>,
    pub auth_private_key_path: Option<String>,
    pub timeout_ms: u64,
    pub models: Vec<String>,
    pub default_model: Option<String>,
    pub image_models: Vec<String>,
    pub default_image_model: Option<String>,
    pub features: Vec<Feature>,
    pub alias_map: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct OpenAIProvider {
    instance: ProviderInstance,
    client: Client,
    auth_mode: OpenAIAuthMode,
    base_url: String,
}

#[derive(Debug, Clone)]
enum OpenAIAuthMode {
    Bearer(String),
    DeviceJwt {
        subject: String,
        appid: String,
        private_key_path: PathBuf,
    },
}

impl OpenAIProvider {
    fn format_error_chain(err: &reqwest::Error) -> String {
        let mut segments = vec![err.to_string()];
        let mut source = err.source();
        while let Some(cause) = source {
            segments.push(cause.to_string());
            source = cause.source();
        }
        segments.join(" | caused_by: ")
    }

    pub fn new(cfg: OpenAIInstanceConfig) -> Result<Self> {
        let timeout_ms = if cfg.timeout_ms == 0 {
            DEFAULT_OPENAI_TIMEOUT_MS
        } else {
            cfg.timeout_ms
        };

        let client = Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .context("failed to build reqwest client for openai provider")?;

        let instance = ProviderInstance {
            instance_id: cfg.instance_id,
            provider_type: cfg.provider_type,
            capabilities: vec![Capability::LlmRouter, Capability::Text2Image],
            features: cfg.features,
            endpoint: Some(cfg.base_url.clone()),
            plugin_key: None,
        };

        let auth_mode = Self::parse_auth_mode(
            cfg.auth_mode.as_str(),
            cfg.api_token,
            cfg.auth_subject,
            cfg.auth_appid,
            cfg.auth_private_key_path,
        )?;

        Ok(Self {
            instance,
            client,
            auth_mode,
            base_url: cfg.base_url.trim_end_matches('/').to_string(),
        })
    }

    fn parse_auth_mode(
        auth_mode: &str,
        api_token: Option<String>,
        auth_subject: Option<String>,
        auth_appid: Option<String>,
        auth_private_key_path: Option<String>,
    ) -> Result<OpenAIAuthMode> {
        let mode = auth_mode.trim().to_ascii_lowercase();
        if mode.is_empty() || mode == DEFAULT_AUTH_MODE {
            let token = api_token
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow!("openai bearer auth requires non-empty api_token"))?;
            return Ok(OpenAIAuthMode::Bearer(token));
        }

        if mode == DEVICE_JWT_AUTH_MODE {
            let appid = auth_appid
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| DEFAULT_DEVICE_AUTH_APP_ID.to_string());
            let subject = auth_subject
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(Self::read_default_device_subject);
            let private_key_path = auth_private_key_path
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| get_buckyos_system_etc_dir().join("node_private_key.pem"));
            return Ok(OpenAIAuthMode::DeviceJwt {
                subject,
                appid,
                private_key_path,
            });
        }

        Err(anyhow!(
            "unsupported openai auth_mode '{}', expected '{}' or '{}'",
            auth_mode,
            DEFAULT_AUTH_MODE,
            DEVICE_JWT_AUTH_MODE
        ))
    }

    fn read_default_device_subject() -> String {
        let device_cfg_path = get_buckyos_system_etc_dir().join("node_device_config.json");
        let content = std::fs::read_to_string(device_cfg_path.as_path());
        if let Ok(content) = content {
            if let Ok(json_value) = serde_json::from_str::<Value>(content.as_str()) {
                if let Some(name) = json_value
                    .get("name")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    return name.to_string();
                }
            }
        }
        "ood1".to_string()
    }

    fn build_auth_token(&self) -> Result<String, ProviderError> {
        match &self.auth_mode {
            OpenAIAuthMode::Bearer(token) => Ok(token.clone()),
            OpenAIAuthMode::DeviceJwt {
                subject,
                appid,
                private_key_path,
            } => {
                let private_key = load_private_key(private_key_path.as_path()).map_err(|err| {
                    ProviderError::fatal(format!(
                        "openai device_jwt auth failed to load private key '{}': {}",
                        private_key_path.display(),
                        err
                    ))
                })?;
                let now = buckyos_get_unix_timestamp();
                let claims = RPCSessionToken {
                    token_type: RPCSessionTokenType::JWT,
                    token: None,
                    aud: None,
                    exp: Some(now + 60 * 15),
                    // SN expects issuer to be device name, while subject/appid are caller identity.
                    iss: Some(Self::read_default_device_subject()),
                    jti: None,
                    session: None,
                    sub: Some(subject.to_string()),
                    appid: Some(appid.to_string()),
                    extra: HashMap::new(),
                };
                let jwt = claims.generate_jwt(None, &private_key).map_err(|err| {
                    ProviderError::fatal(format!("openai device_jwt auth failed: {}", err))
                })?;
                Ok(jwt)
            }
        }
    }

    fn price_per_1m_tokens(model: &str) -> (f64, f64) {
        if model.starts_with("gpt-4.1-mini") {
            (0.40, 1.60)
        } else if model.starts_with("gpt-4.1") {
            (2.00, 8.00)
        } else if model.starts_with("gpt-4o-mini") {
            (0.15, 0.60)
        } else if model.starts_with("gpt-4o") {
            (2.50, 10.00)
        } else if model.starts_with("gpt-3.5") {
            (0.50, 1.50)
        } else {
            (1.00, 3.00)
        }
    }

    fn estimate_tokens(req: &CompleteRequest) -> (u64, u64) {
        let mut text_len = 0usize;

        if let Some(text) = req.payload.text.as_ref() {
            text_len += text.len();
        }

        for message in req.payload.messages.iter() {
            text_len += message.content.len();
        }

        for resource in req.payload.resources.iter() {
            match resource {
                ResourceRef::Url { url, .. } => {
                    text_len += url.len();
                }
                ResourceRef::NamedObject { obj_id } => {
                    text_len += obj_id.to_string().len();
                }
                ResourceRef::Base64 { .. } => {
                    text_len += 256;
                }
            }
        }

        let input_tokens = ((text_len as f64) / 4.0).ceil() as u64;
        let output_tokens = req
            .payload
            .options
            .as_ref()
            .and_then(|value| {
                value
                    .get("max_output_tokens")
                    .and_then(|value| value.as_u64())
                    .or_else(|| value.get("max_tokens").and_then(|value| value.as_u64()))
                    .or_else(|| {
                        value
                            .get("max_completion_tokens")
                            .and_then(|value| value.as_u64())
                    })
            })
            .unwrap_or(512);

        (input_tokens.max(1), output_tokens.max(1))
    }

    fn estimate_image_count(req: &CompleteRequest) -> u64 {
        req.payload
            .options
            .as_ref()
            .and_then(|value| value.get("n"))
            .and_then(|value| value.as_u64())
            .or_else(|| {
                req.payload
                    .input_json
                    .as_ref()
                    .and_then(|value| value.get("n"))
                    .and_then(|value| value.as_u64())
            })
            .unwrap_or(1)
            .max(1)
    }

    fn estimate_text2image_cost(req: &CompleteRequest, model: &str) -> Option<f64> {
        let per_image = if model.starts_with("dall-e-2") {
            0.02
        } else if model.starts_with("dall-e-3") {
            let quality = req
                .payload
                .options
                .as_ref()
                .and_then(|value| value.get("quality"))
                .and_then(|value| value.as_str())
                .or_else(|| {
                    req.payload
                        .input_json
                        .as_ref()
                        .and_then(|value| value.get("quality"))
                        .and_then(|value| value.as_str())
                })
                .unwrap_or("standard");
            if quality == "hd" {
                0.08
            } else {
                0.04
            }
        } else {
            0.04
        };

        Some((Self::estimate_image_count(req) as f64) * per_image)
    }

    fn estimate_cost_for_usage(&self, model: &str, usage: &AiUsage) -> Option<AiCost> {
        let input_tokens = usage.input_tokens? as f64;
        let output_tokens = usage.output_tokens? as f64;
        let (input_per_m, output_per_m) = Self::price_per_1m_tokens(model);

        let amount = ((input_tokens / 1_000_000.0) * input_per_m)
            + ((output_tokens / 1_000_000.0) * output_per_m);

        Some(AiCost {
            amount,
            currency: "USD".to_string(),
        })
    }

    fn build_messages(&self, req: &CompleteRequest) -> Result<Vec<Value>, ProviderError> {
        let mut messages = vec![];

        for msg in req.payload.messages.iter() {
            if msg.role.trim().is_empty() || msg.content.trim().is_empty() {
                continue;
            }
            messages.push(json!({
                "role": msg.role,
                "content": [
                    {
                        "type": "input_text",
                        "text": msg.content
                    }
                ],
            }));
        }

        if messages.is_empty() {
            let mut content = String::new();
            if let Some(text) = req.payload.text.as_ref() {
                content.push_str(text);
            }

            let mut resource_lines = vec![];
            for resource in req.payload.resources.iter() {
                match resource {
                    ResourceRef::Url { url, .. } => {
                        resource_lines.push(format!("resource_url: {}", url));
                    }
                    ResourceRef::NamedObject { obj_id } => {
                        resource_lines.push(format!("named_object: {}", obj_id));
                    }
                    ResourceRef::Base64 { .. } => {
                        return Err(ProviderError::fatal(
                            "openai provider does not support base64 resources in this version",
                        ));
                    }
                }
            }

            if !resource_lines.is_empty() {
                if !content.is_empty() {
                    content.push('\n');
                    content.push('\n');
                }
                content.push_str(resource_lines.join("\n").as_str());
            }

            if !content.trim().is_empty() {
                messages.push(json!({
                    "role": "user",
                    "content": [
                        {
                            "type": "input_text",
                            "text": content
                        }
                    ],
                }));
            }
        }

        if messages.is_empty() {
            return Err(ProviderError::fatal(
                "request payload has no usable text/messages for llm",
            ));
        }

        Ok(messages)
    }

    fn build_chat_messages(req: &CompleteRequest) -> Result<Vec<Value>, ProviderError> {
        let mut messages = vec![];

        for msg in req.payload.messages.iter() {
            if msg.role.trim().is_empty() || msg.content.trim().is_empty() {
                continue;
            }
            messages.push(json!({
                "role": msg.role,
                "content": msg.content,
            }));
        }

        if messages.is_empty() {
            let mut content = String::new();
            if let Some(text) = req.payload.text.as_ref() {
                content.push_str(text);
            }

            let mut resource_lines = vec![];
            for resource in req.payload.resources.iter() {
                match resource {
                    ResourceRef::Url { url, .. } => {
                        resource_lines.push(format!("resource_url: {}", url));
                    }
                    ResourceRef::NamedObject { obj_id } => {
                        resource_lines.push(format!("named_object: {}", obj_id));
                    }
                    ResourceRef::Base64 { .. } => {
                        return Err(ProviderError::fatal(
                            "openai provider does not support base64 resources in this version",
                        ));
                    }
                }
            }

            if !resource_lines.is_empty() {
                if !content.is_empty() {
                    content.push('\n');
                    content.push('\n');
                }
                content.push_str(resource_lines.join("\n").as_str());
            }

            if !content.trim().is_empty() {
                messages.push(json!({
                    "role": "user",
                    "content": content
                }));
            }
        }

        if messages.is_empty() {
            return Err(ProviderError::fatal(
                "request payload has no usable text/messages for llm",
            ));
        }

        Ok(messages)
    }

    fn use_chat_completions_endpoint(&self) -> bool {
        self.base_url
            .to_ascii_lowercase()
            .contains("/chat/completions")
    }

    fn extract_legacy_message_text(choice_message: &Value) -> Option<String> {
        let content = choice_message.get("content")?;
        if let Some(text) = content.as_str() {
            return Some(text.to_string());
        }

        let segments = content.as_array()?;
        let joined = segments
            .iter()
            .filter_map(|segment| {
                if segment.get("type").and_then(|value| value.as_str()) == Some("text") {
                    segment
                        .get("text")
                        .and_then(|value| value.as_str())
                        .map(|text| text.to_string())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        if joined.is_empty() {
            None
        } else {
            Some(joined)
        }
    }

    fn parse_tool_arguments(raw: Value, field_path: &str) -> Option<Value> {
        match raw {
            Value::String(raw) => {
                let trimmed = raw.trim();
                if trimmed.is_empty() {
                    return Some(Value::Object(Map::new()));
                }
                match serde_json::from_str::<Value>(trimmed) {
                    Ok(parsed) => Some(parsed),
                    Err(err) => {
                        warn!(
                            "aicc.openai {} is invalid json arguments: {}",
                            field_path, err
                        );
                        None
                    }
                }
            }
            Value::Null => Some(Value::Object(Map::new())),
            other => Some(other),
        }
    }

    fn extract_text_content(payload: &Value) -> Option<String> {
        if let Some(text) = payload.get("output_text").and_then(|value| value.as_str()) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }

        if let Some(output_items) = payload.get("output").and_then(|value| value.as_array()) {
            let mut parts = Vec::new();
            for item in output_items.iter() {
                let Some(item_obj) = item.as_object() else {
                    continue;
                };
                let item_type = item_obj
                    .get("type")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                if item_type == "output_text" {
                    if let Some(text) = item_obj.get("text").and_then(|value| value.as_str()) {
                        if text.trim().is_empty() {
                            continue;
                        }
                        parts.push(text.to_string());
                    }
                    continue;
                }

                if item_type != "message" {
                    continue;
                }

                let Some(content_items) =
                    item_obj.get("content").and_then(|value| value.as_array())
                else {
                    continue;
                };
                for content_item in content_items.iter() {
                    let Some(content_obj) = content_item.as_object() else {
                        continue;
                    };
                    let content_type = content_obj
                        .get("type")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default();
                    if content_type != "output_text" && content_type != "text" {
                        continue;
                    }
                    if let Some(text) = content_obj.get("text").and_then(|value| value.as_str()) {
                        if text.trim().is_empty() {
                            continue;
                        }
                        parts.push(text.to_string());
                    }
                }
            }
            if !parts.is_empty() {
                let merged = parts.concat();
                let trimmed = merged.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }

        if let Some(text) = Self::extract_legacy_message_text(payload) {
            return Some(text);
        }

        payload
            .pointer("/choices/0/message")
            .and_then(Self::extract_legacy_message_text)
    }

    fn extract_tool_choices(payload: &Value) -> Vec<AiToolCall> {
        let mut tool_choices = Vec::new();
        if let Some(items) = payload.get("output").and_then(|value| value.as_array()) {
            for (idx, item) in items.iter().enumerate() {
                let Some(item_obj) = item.as_object() else {
                    continue;
                };
                if item_obj.get("type").and_then(|value| value.as_str()) != Some("function_call") {
                    continue;
                }

                let call_id = item_obj
                    .get("call_id")
                    .or_else(|| item_obj.get("id"))
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_string());
                let Some(call_id) = call_id else {
                    warn!(
                        "aicc.openai output[{}] function_call is missing call_id/id",
                        idx
                    );
                    continue;
                };

                let Some(name) = item_obj
                    .get("name")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                else {
                    warn!("aicc.openai output[{}] function_call is missing name", idx);
                    continue;
                };

                let args_raw = item_obj
                    .get("arguments")
                    .or_else(|| item_obj.get("args"))
                    .cloned()
                    .unwrap_or(Value::Null);
                let Some(args) = Self::parse_tool_arguments(
                    args_raw,
                    format!("output[{}].arguments", idx).as_str(),
                ) else {
                    continue;
                };
                if !args.is_object() {
                    warn!(
                        "aicc.openai output[{}].arguments must decode to an object",
                        idx
                    );
                    continue;
                }

                tool_choices.push(AiToolCall {
                    name: name.to_string(),
                    args: value_to_object_map(args),
                    call_id,
                });
            }
            if !tool_choices.is_empty() {
                return tool_choices;
            }
        }

        let fallback_source = payload
            .pointer("/choices/0/message")
            .filter(|value| !value.is_null())
            .unwrap_or(payload);
        let Some(items) = fallback_source
            .get("tool_calls")
            .and_then(|value| value.as_array())
        else {
            return tool_choices;
        };

        for (idx, item) in items.iter().enumerate() {
            let Some(item_obj) = item.as_object() else {
                warn!("aicc.openai tool_calls[{}] must be an object", idx);
                continue;
            };

            let call_id = item_obj
                .get("id")
                .or_else(|| item_obj.get("call_id"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string());
            let Some(call_id) = call_id else {
                warn!("aicc.openai tool_calls[{}] is missing id/call_id", idx);
                continue;
            };

            if let Some(name) = item_obj
                .get("name")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                let args_value = item_obj
                    .get("args")
                    .cloned()
                    .unwrap_or_else(|| Value::Object(Map::new()));
                if !args_value.is_object() {
                    warn!("aicc.openai tool_calls[{}].args must be an object", idx);
                    continue;
                }
                tool_choices.push(AiToolCall {
                    name: name.to_string(),
                    args: value_to_object_map(args_value),
                    call_id,
                });
                continue;
            }

            let Some(function_obj) = item_obj.get("function").and_then(|value| value.as_object())
            else {
                warn!(
                    "aicc.openai tool_calls[{}] is missing name/args and function payload",
                    idx
                );
                continue;
            };

            let Some(name) = function_obj
                .get("name")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                warn!("aicc.openai tool_calls[{}].function.name is required", idx);
                continue;
            };

            let args_raw = function_obj
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let Some(args) = Self::parse_tool_arguments(
                args_raw,
                format!("tool_calls[{}].function.arguments", idx).as_str(),
            ) else {
                continue;
            };
            if !args.is_object() {
                warn!(
                    "aicc.openai tool_calls[{}].function.arguments must decode to an object",
                    idx
                );
                continue;
            }

            tool_choices.push(AiToolCall {
                name: name.to_string(),
                args: value_to_object_map(args),
                call_id,
            });
        }

        tool_choices
    }

    fn classify_api_error(status: StatusCode, message: String) -> ProviderError {
        if status.as_u16() == 429 || status.is_server_error() {
            ProviderError::retryable(message)
        } else {
            ProviderError::fatal(message)
        }
    }

    fn incomplete_output_error(
        body: &Value,
        content: Option<&str>,
        tool_choices: &[AiToolCall],
    ) -> Option<ProviderError> {
        let status = body
            .get("status")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        if status != "incomplete" {
            return None;
        }

        let reason = body
            .pointer("/incomplete_details/reason")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown");
        let response_id = body
            .get("id")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        if reason == "max_output_tokens" {
            let message = if response_id.is_empty() {
                "TOKEN_LIMIT_EXCEEDED: openai max_output_tokens exhausted before response completed"
                    .to_string()
            } else {
                format!(
                    "TOKEN_LIMIT_EXCEEDED: openai max_output_tokens exhausted before response completed (response_id={})",
                    response_id
                )
            };
            return Some(ProviderError::fatal(message));
        }

        let has_text = content
            .map(str::trim)
            .map(|value| !value.is_empty())
            .unwrap_or(false);
        if has_text || !tool_choices.is_empty() {
            return None;
        }

        let message = if response_id.is_empty() {
            format!(
                "openai response incomplete before output text/tool calls (reason={})",
                reason
            )
        } else {
            format!(
                "openai response incomplete before output text/tool calls (reason={}, response_id={})",
                reason, response_id
            )
        };

        Some(ProviderError::fatal(message))
    }

    fn extract_unsupported_request_param(body: &Value) -> Option<String> {
        let param = body
            .pointer("/error/param")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.trim_matches('\'').trim_matches('"').to_string())?;

        let message = body
            .pointer("/error/message")
            .and_then(|value| value.as_str())
            .map(|value| value.to_ascii_lowercase())
            .unwrap_or_default();
        if !message.contains("unsupported parameter") && !message.contains("not supported") {
            return None;
        }

        Some(param)
    }

    fn remove_retryable_unsupported_option(
        request_obj: &mut Map<String, Value>,
        param: &str,
    ) -> bool {
        const RETRYABLE_OPTION_KEYS: &[&str] =
            &["temperature", "top_p", "top_logprobs", "logprobs"];
        if !RETRYABLE_OPTION_KEYS.contains(&param) {
            return false;
        }
        request_obj.remove(param).is_some()
    }

    fn process_sse_event_payload(
        payload: &str,
        final_response: &mut Option<Value>,
        accumulated_text: &mut String,
    ) -> Result<(), String> {
        let trimmed = payload.trim();
        if trimmed.is_empty() || trimmed == "[DONE]" {
            return Ok(());
        }

        let event: Value = serde_json::from_str(trimmed)
            .map_err(|err| format!("invalid sse event json: {}; payload={}", err, trimmed))?;

        if let Some(response) = event.get("response") {
            if response.is_object() {
                *final_response = Some(response.clone());
            }
        }

        let event_type = event
            .get("type")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        if (event_type == "response.output_text.delta" || event_type.ends_with("output_text.delta"))
            && event
                .get("delta")
                .and_then(|value| value.as_str())
                .is_some()
        {
            accumulated_text.push_str(
                event
                    .get("delta")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default(),
            );
        }
        Ok(())
    }

    fn parse_sse_response_body(raw: &str) -> Result<Value, String> {
        let mut final_response: Option<Value> = None;
        let mut accumulated_text = String::new();
        let mut pending_data_lines: Vec<String> = vec![];

        for line in raw.lines() {
            let normalized = line.trim_end_matches('\r');
            if normalized.is_empty() {
                if !pending_data_lines.is_empty() {
                    let payload = pending_data_lines.join("\n");
                    Self::process_sse_event_payload(
                        payload.as_str(),
                        &mut final_response,
                        &mut accumulated_text,
                    )?;
                    pending_data_lines.clear();
                }
                continue;
            }

            if let Some(data) = normalized.strip_prefix("data:") {
                pending_data_lines.push(data.trim_start().to_string());
            }
        }

        if !pending_data_lines.is_empty() {
            let payload = pending_data_lines.join("\n");
            Self::process_sse_event_payload(
                payload.as_str(),
                &mut final_response,
                &mut accumulated_text,
            )?;
        }

        if let Some(mut response) = final_response {
            if !accumulated_text.is_empty()
                && response
                    .get("output_text")
                    .and_then(|value| value.as_str())
                    .map(|value| value.trim().is_empty())
                    .unwrap_or(true)
            {
                if let Some(obj) = response.as_object_mut() {
                    obj.insert("output_text".to_string(), Value::String(accumulated_text));
                }
            }
            return Ok(response);
        }

        if !accumulated_text.is_empty() {
            return Ok(json!({
                "status": "completed",
                "output_text": accumulated_text,
                "output": []
            }));
        }

        Err("sse stream ended without response payload".to_string())
    }

    fn extract_text2image_prompt(req: &CompleteRequest) -> Option<String> {
        if let Some(text) = req
            .payload
            .text
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            return Some(text.to_string());
        }

        let message_prompt = req
            .payload
            .messages
            .iter()
            .map(|msg| msg.content.trim())
            .filter(|msg| !msg.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        if !message_prompt.is_empty() {
            return Some(message_prompt);
        }

        if let Some(prompt) = req
            .payload
            .input_json
            .as_ref()
            .and_then(|value| value.get("prompt"))
            .and_then(|value| value.as_str())
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            return Some(prompt.to_string());
        }

        req.payload
            .options
            .as_ref()
            .and_then(|value| value.get("prompt"))
            .and_then(|value| value.as_str())
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
    }

    fn merge_text2image_options(
        target: &mut Map<String, Value>,
        options: &Value,
    ) -> Result<Vec<String>, ProviderError> {
        let Some(options_map) = options.as_object() else {
            return Ok(vec![]);
        };

        let mut ignored = vec![];
        for (key, value) in options_map.iter() {
            if key == "model" || key == "messages" || key == "prompt" {
                continue;
            }
            if key == "protocol" || key == "process_name" || key == "tool_messages" {
                ignored.push(key.clone());
                continue;
            }
            if !OPENAI_IMAGE_OPTION_ALLOWLIST.contains(&key.as_str()) {
                ignored.push(key.clone());
                continue;
            }
            target.insert(key.clone(), value.clone());
        }
        Ok(ignored)
    }

    fn parse_text2image_artifacts(body: &Value) -> Result<Vec<AiArtifact>, ProviderError> {
        let Some(items) = body.get("data").and_then(|value| value.as_array()) else {
            return Err(ProviderError::fatal(
                "openai image response is missing data array",
            ));
        };

        let mut artifacts = vec![];
        for (idx, item) in items.iter().enumerate() {
            let metadata = item
                .get("revised_prompt")
                .and_then(|value| value.as_str())
                .map(|prompt| json!({ "revised_prompt": prompt }));
            if let Some(url) = item
                .get("url")
                .and_then(|value| value.as_str())
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
            {
                artifacts.push(AiArtifact {
                    name: format!("image_{}", idx + 1),
                    resource: ResourceRef::Url {
                        url: url.to_string(),
                        mime_hint: Some("image/png".to_string()),
                    },
                    mime: Some("image/png".to_string()),
                    metadata,
                });
                continue;
            }

            if let Some(b64_json) = item
                .get("b64_json")
                .and_then(|value| value.as_str())
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
            {
                if general_purpose::STANDARD.decode(b64_json).is_err() {
                    warn!(
                        "aicc.openai received invalid b64_json at index {} in image response",
                        idx
                    );
                    continue;
                }
                artifacts.push(AiArtifact {
                    name: format!("image_{}", idx + 1),
                    resource: ResourceRef::Base64 {
                        mime: "image/png".to_string(),
                        data_base64: b64_json.to_string(),
                    },
                    mime: Some("image/png".to_string()),
                    metadata,
                });
            }
        }

        if artifacts.is_empty() {
            return Err(ProviderError::fatal(
                "openai image response has no usable image outputs",
            ));
        }
        Ok(artifacts)
    }

    fn merge_requirements_tools(
        target: &mut Map<String, Value>,
        req: &CompleteRequest,
    ) -> Result<(), ProviderError> {
        let web_search_required = req
            .requirements
            .must_features
            .iter()
            .any(|feature| feature == features::WEB_SEARCH);
        if !web_search_required {
            return Ok(());
        }

        let web_search_tool = json!({
            "type": OPENAI_TOOL_TYPE_WEB_SEARCH
        });
        if let Some(tools_value) = target.get_mut("tools") {
            let Some(tools) = tools_value.as_array_mut() else {
                return Err(ProviderError::fatal(
                    "tools must be an array when enabling web_search",
                ));
            };
            if !tools.iter().any(|item| {
                item.get("type")
                    .and_then(|value| value.as_str())
                    .map(|value| value == OPENAI_TOOL_TYPE_WEB_SEARCH || value == "web_search")
                    .unwrap_or(false)
            }) {
                tools.push(web_search_tool);
            }
            return Ok(());
        }

        target.insert("tools".to_string(), Value::Array(vec![web_search_tool]));
        Ok(())
    }

    async fn post_json(
        &self,
        url: &str,
        request_obj: &Map<String, Value>,
    ) -> Result<(StatusCode, Value, u64), ProviderError> {
        let auth_token = self.build_auth_token()?;
        let started_at = std::time::Instant::now();
        let response = self
            .client
            .post(url)
            .bearer_auth(auth_token.as_str())
            .json(request_obj)
            .send()
            .await
            .map_err(|err| {
                let retryable = err.is_timeout() || err.is_connect();
                error!(
                    "aicc.openai.http_send_failed instance_id={} provider_type={} url={} retryable={} timeout={} connect={} status={:?} err_chain={}",
                    self.instance.instance_id,
                    self.instance.provider_type,
                    url,
                    retryable,
                    err.is_timeout(),
                    err.is_connect(),
                    err.status(),
                    Self::format_error_chain(&err)
                );
                eprintln!(
                    "aicc.openai.http_send_failed instance_id={} provider_type={} url={} retryable={} timeout={} connect={} status={:?} err_chain={}",
                    self.instance.instance_id,
                    self.instance.provider_type,
                    url,
                    retryable,
                    err.is_timeout(),
                    err.is_connect(),
                    err.status(),
                    Self::format_error_chain(&err)
                );
                if err.is_timeout() || err.is_connect() {
                    ProviderError::retryable(format!("openai request failed: {}", err))
                } else {
                    ProviderError::fatal(format!("openai request failed: {}", err))
                }
            })?;
        let latency_ms = started_at.elapsed().as_millis() as u64;

        let status = response.status();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let content_encoding = response
            .headers()
            .get(CONTENT_ENCODING)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let raw_body = response.text().await.map_err(|err| {
            error!(
                "aicc.openai.response_decode_failed instance_id={} provider_type={} url={} status={} content_type={} content_encoding={} err={}",
                self.instance.instance_id,
                self.instance.provider_type,
                url,
                status.as_u16(),
                content_type,
                if content_encoding.is_empty() {
                    "<none>"
                } else {
                    content_encoding.as_str()
                },
                err
            );
            let decode_err = format!(
                "failed to decode openai response body: {}; status={} content_type={} content_encoding={}",
                err,
                status.as_u16(),
                content_type,
                if content_encoding.is_empty() {
                    "<none>"
                } else {
                    content_encoding.as_str()
                }
            );
            if status.as_u16() == 429 || status.is_server_error() {
                ProviderError::retryable(decode_err)
            } else {
                ProviderError::fatal(decode_err)
            }
        })?;
        let body_parse_result = if content_type.contains("text/event-stream") {
            Self::parse_sse_response_body(raw_body.as_str())
        } else {
            serde_json::from_str::<Value>(raw_body.as_str()).map_err(|err| {
                format!(
                    "invalid json response: {}; body_head={}",
                    err,
                    raw_body.chars().take(320).collect::<String>()
                )
            })
        };
        let body: Value = body_parse_result.map_err(|err| {
            error!(
                "aicc.openai.response_parse_failed instance_id={} provider_type={} url={} status={} err={}",
                self.instance.instance_id,
                self.instance.provider_type,
                url,
                status.as_u16(),
                err
            );
            if status.as_u16() == 429 || status.is_server_error() {
                ProviderError::retryable(format!("failed to parse openai response body: {}", err))
            } else {
                ProviderError::fatal(format!("failed to parse openai response body: {}", err))
            }
        })?;

        Ok((status, body, latency_ms))
    }

    async fn start_llm(
        &self,
        ctx: &crate::aicc::InvokeCtx,
        provider_model: &str,
        req: &CompleteRequest,
    ) -> Result<ProviderStartResult, ProviderError> {
        let mut request_obj = Map::new();
        request_obj.insert(
            "model".to_string(),
            Value::String(provider_model.to_string()),
        );
        if self.use_chat_completions_endpoint() {
            let messages = Self::build_chat_messages(req)?;
            request_obj.insert("messages".to_string(), Value::Array(messages));
        } else {
            let messages = self.build_messages(req)?;
            request_obj.insert("input".to_string(), Value::Array(messages));
        }

        let mut ignored_options = vec![];
        if let Some(options) = req.payload.options.as_ref() {
            ignored_options = merge_options(&mut request_obj, options)?;
        }
        let stripped_options =
            strip_incompatible_sampling_options(&mut request_obj, provider_model);
        ignored_options.extend(stripped_options);
        merge_requirements_response_format(&mut request_obj, req);
        merge_tool_calls(&mut request_obj, req.payload.tool_specs.as_slice())?;
        Self::merge_requirements_tools(&mut request_obj, req)?;
        if !ignored_options.is_empty() {
            warn!(
                "aicc.openai ignored unsupported llm options: instance_id={} model={} trace_id={:?} ignored={:?}",
                self.instance.instance_id, provider_model, ctx.trace_id, ignored_options
            );
        }

        let request_log = Value::Object(request_obj.clone()).to_string();
        info!(
            "aicc.openai.llm.input instance_id={} model={} trace_id={:?} request={}",
            self.instance.instance_id, provider_model, ctx.trace_id, request_log
        );

        let url = if self.use_chat_completions_endpoint() {
            self.base_url.clone()
        } else {
            format!("{}/responses", self.base_url)
        };
        let mut retried_without_option = false;
        let (status, body, latency_ms) = loop {
            let (status, body, latency_ms) = self.post_json(url.as_str(), &request_obj).await?;
            if status == StatusCode::BAD_REQUEST && !retried_without_option {
                if let Some(param) = Self::extract_unsupported_request_param(&body) {
                    if Self::remove_retryable_unsupported_option(&mut request_obj, param.as_str()) {
                        warn!(
                            "aicc.openai.llm.retry_without_option instance_id={} model={} trace_id={:?} param={} response={}",
                            self.instance.instance_id,
                            provider_model,
                            ctx.trace_id,
                            param,
                            body
                        );
                        retried_without_option = true;
                        continue;
                    }
                }
            }
            break (status, body, latency_ms);
        };
        let response_log = body.to_string();

        if !status.is_success() {
            warn!(
                "aicc.openai.llm.output instance_id={} model={} trace_id={:?} status={} response={}",
                self.instance.instance_id,
                provider_model,
                ctx.trace_id,
                status.as_u16(),
                response_log
            );
            let message = body
                .pointer("/error/message")
                .and_then(|value| value.as_str())
                .unwrap_or("openai api returned non-success status")
                .to_string();
            let code = body
                .pointer("/error/code")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            return Err(Self::classify_api_error(
                status,
                format!("openai api error [{}]: {}", code, message),
            ));
        }
        info!(
            "aicc.openai.llm.output instance_id={} model={} trace_id={:?} status={} response={}",
            self.instance.instance_id,
            provider_model,
            ctx.trace_id,
            status.as_u16(),
            response_log
        );

        let content = Self::extract_text_content(&body);
        let tool_choices = Self::extract_tool_choices(&body);
        if let Some(err) =
            Self::incomplete_output_error(&body, content.as_deref(), tool_choices.as_slice())
        {
            warn!(
                "aicc.openai.llm.incomplete_output instance_id={} model={} trace_id={:?} err={}",
                self.instance.instance_id, provider_model, ctx.trace_id, err
            );
            return Err(err);
        }

        let usage = body.get("usage").map(|usage| AiUsage {
            input_tokens: usage
                .get("input_tokens")
                .and_then(|value| value.as_u64())
                .or_else(|| usage.get("prompt_tokens").and_then(|value| value.as_u64())),
            output_tokens: usage
                .get("output_tokens")
                .and_then(|value| value.as_u64())
                .or_else(|| {
                    usage
                        .get("completion_tokens")
                        .and_then(|value| value.as_u64())
                }),
            total_tokens: usage.get("total_tokens").and_then(|value| value.as_u64()),
        });

        let cost = usage
            .as_ref()
            .and_then(|usage| self.estimate_cost_for_usage(provider_model, usage));

        let mut extra = Map::new();
        extra.insert("provider".to_string(), Value::String("openai".to_string()));
        extra.insert(
            "model".to_string(),
            Value::String(provider_model.to_string()),
        );
        extra.insert("latency_ms".to_string(), Value::from(latency_ms));
        extra.insert(
            "provider_io".to_string(),
            json!({
                "input": Value::Object(request_obj.clone()),
                "output": body.clone()
            }),
        );

        let summary = AiResponseSummary {
            text: content,
            tool_calls: tool_choices,
            artifacts: vec![],
            usage,
            cost,
            finish_reason: body
                .get("status")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string())
                .or_else(|| {
                    body.pointer("/output/0/status")
                        .and_then(|value| value.as_str())
                        .map(|value| value.to_string())
                })
                .or_else(|| {
                    body.pointer("/choices/0/finish_reason")
                        .and_then(|value| value.as_str())
                        .map(|value| value.to_string())
                }),
            provider_task_ref: body
                .get("id")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string()),
            extra: Some(Value::Object(extra)),
        };

        Ok(ProviderStartResult::Immediate(summary))
    }

    async fn start_text2image(
        &self,
        ctx: &crate::aicc::InvokeCtx,
        provider_model: &str,
        req: &CompleteRequest,
    ) -> Result<ProviderStartResult, ProviderError> {
        let mut request_obj = Map::new();
        request_obj.insert(
            "model".to_string(),
            Value::String(provider_model.to_string()),
        );

        if let Some(input_json) = req
            .payload
            .input_json
            .as_ref()
            .and_then(|value| value.as_object())
        {
            for (key, value) in input_json.iter() {
                if OPENAI_IMAGE_INPUT_ALLOWLIST.contains(&key.as_str()) {
                    request_obj.insert(key.clone(), value.clone());
                }
            }
        }

        if let Some(prompt) = Self::extract_text2image_prompt(req) {
            request_obj.insert("prompt".to_string(), Value::String(prompt));
        }

        if !request_obj.contains_key("prompt") {
            return Err(ProviderError::fatal(
                "text2image request requires prompt in payload.text/messages/input_json/options",
            ));
        }

        let mut ignored_options = vec![];
        if let Some(options) = req.payload.options.as_ref() {
            ignored_options = Self::merge_text2image_options(&mut request_obj, options)?;
        }
        if !ignored_options.is_empty() {
            warn!(
                "aicc.openai ignored unsupported text2image options: instance_id={} model={} trace_id={:?} ignored={:?}",
                self.instance.instance_id, provider_model, ctx.trace_id, ignored_options
            );
        }

        let request_log = Value::Object(request_obj.clone()).to_string();
        info!(
            "aicc.openai.text2image.input instance_id={} model={} trace_id={:?} request={}",
            self.instance.instance_id, provider_model, ctx.trace_id, request_log
        );

        let url = format!("{}/images/generations", self.base_url);
        let (status, body, latency_ms) = self.post_json(url.as_str(), &request_obj).await?;
        let response_log = body.to_string();

        if !status.is_success() {
            warn!(
                "aicc.openai.text2image.output instance_id={} model={} trace_id={:?} status={} response={}",
                self.instance.instance_id,
                provider_model,
                ctx.trace_id,
                status.as_u16(),
                response_log
            );
            let message = body
                .pointer("/error/message")
                .and_then(|value| value.as_str())
                .unwrap_or("openai api returned non-success status")
                .to_string();
            let code = body
                .pointer("/error/code")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            return Err(Self::classify_api_error(
                status,
                format!("openai api error [{}]: {}", code, message),
            ));
        }
        info!(
            "aicc.openai.text2image.output instance_id={} model={} trace_id={:?} status={} response={}",
            self.instance.instance_id,
            provider_model,
            ctx.trace_id,
            status.as_u16(),
            response_log
        );

        let artifacts = Self::parse_text2image_artifacts(&body)?;
        let revised_prompt = body
            .pointer("/data/0/revised_prompt")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string());
        let estimated_cost =
            Self::estimate_text2image_cost(req, provider_model).map(|amount| AiCost {
                amount,
                currency: "USD".to_string(),
            });

        let mut extra = Map::new();
        extra.insert("provider".to_string(), Value::String("openai".to_string()));
        extra.insert(
            "model".to_string(),
            Value::String(provider_model.to_string()),
        );
        extra.insert("latency_ms".to_string(), Value::from(latency_ms));
        extra.insert(
            "provider_io".to_string(),
            json!({
                "input": Value::Object(request_obj.clone()),
                "output": body.clone()
            }),
        );

        let summary = AiResponseSummary {
            text: revised_prompt,
            tool_calls: vec![],
            artifacts,
            usage: None,
            cost: estimated_cost,
            finish_reason: Some("stop".to_string()),
            provider_task_ref: body
                .get("id")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string()),
            extra: Some(Value::Object(extra)),
        };
        Ok(ProviderStartResult::Immediate(summary))
    }
}

#[async_trait]
impl Provider for OpenAIProvider {
    fn instance(&self) -> &ProviderInstance {
        &self.instance
    }

    fn estimate_cost(&self, req: &CompleteRequest, provider_model: &str) -> CostEstimate {
        if req.capability == Capability::Text2Image {
            return CostEstimate {
                estimated_cost_usd: Self::estimate_text2image_cost(req, provider_model),
                estimated_latency_ms: Some(5000),
            };
        }

        let (input_tokens, output_tokens) = Self::estimate_tokens(req);
        let usage = AiUsage {
            input_tokens: Some(input_tokens),
            output_tokens: Some(output_tokens),
            total_tokens: Some(input_tokens.saturating_add(output_tokens)),
        };

        let estimated_cost_usd = self
            .estimate_cost_for_usage(provider_model, &usage)
            .map(|cost| cost.amount);

        CostEstimate {
            estimated_cost_usd,
            estimated_latency_ms: Some(1200),
        }
    }

    async fn start(
        &self,
        ctx: crate::aicc::InvokeCtx,
        provider_model: String,
        req: ResolvedRequest,
        _sink: Arc<dyn TaskEventSink>,
    ) -> std::result::Result<ProviderStartResult, ProviderError> {
        match req.request.capability {
            Capability::LlmRouter => {
                self.start_llm(&ctx, provider_model.as_str(), &req.request)
                    .await
            }
            Capability::Text2Image => {
                self.start_text2image(&ctx, provider_model.as_str(), &req.request)
                    .await
            }
            capability => Err(ProviderError::fatal(format!(
                "openai provider does not support capability '{:?}'",
                capability
            ))),
        }
    }

    async fn cancel(
        &self,
        _ctx: crate::aicc::InvokeCtx,
        _task_id: &str,
    ) -> std::result::Result<(), ProviderError> {
        Ok(())
    }
}

#[derive(Debug, Deserialize, Default)]
struct OpenAISettings {
    #[serde(default = "default_openai_enabled")]
    enabled: bool,
    #[serde(default)]
    api_token: String,
    #[serde(default)]
    alias_map: HashMap<String, String>,
    #[serde(default)]
    instances: Vec<SettingsOpenAIInstanceConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct SettingsOpenAIInstanceConfig {
    #[serde(default = "default_instance_id")]
    instance_id: String,
    #[serde(default = "default_provider_type")]
    provider_type: String,
    #[serde(default = "default_base_url")]
    base_url: String,
    #[serde(default = "default_auth_mode")]
    auth_mode: String,
    #[serde(default)]
    api_token: Option<String>,
    #[serde(default)]
    auth_subject: Option<String>,
    #[serde(default)]
    auth_appid: Option<String>,
    #[serde(default)]
    auth_private_key_path: Option<String>,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
    #[serde(default)]
    models: Vec<String>,
    #[serde(default)]
    default_model: Option<String>,
    #[serde(default)]
    image_models: Vec<String>,
    #[serde(default)]
    default_image_model: Option<String>,
    #[serde(default)]
    features: Vec<String>,
    #[serde(default)]
    alias_map: HashMap<String, String>,
}

fn default_openai_enabled() -> bool {
    true
}

fn default_instance_id() -> String {
    "openai-default".to_string()
}

fn default_provider_type() -> String {
    "openai".to_string()
}

fn default_base_url() -> String {
    DEFAULT_OPENAI_BASE_URL.to_string()
}

fn default_timeout_ms() -> u64 {
    DEFAULT_OPENAI_TIMEOUT_MS
}

fn default_auth_mode() -> String {
    DEFAULT_AUTH_MODE.to_string()
}

fn default_features() -> Vec<String> {
    vec![
        features::PLAN.to_string(),
        features::JSON_OUTPUT.to_string(),
        features::TOOL_CALLING.to_string(),
        features::WEB_SEARCH.to_string(),
    ]
}

fn is_text2image_model_name(model: &str) -> bool {
    model.trim().to_ascii_lowercase().starts_with("dall-e")
}

fn normalize_model_list(models: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::<String>::new();
    let mut normalized = vec![];
    for model in models.into_iter() {
        let value = model.trim();
        if value.is_empty() {
            continue;
        }
        let key = value.to_ascii_lowercase();
        if seen.insert(key) {
            normalized.push(value.to_string());
        }
    }
    normalized
}

fn parse_csv_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
        .map(|item| item.to_string())
        .collect::<Vec<_>>()
}

fn parse_openai_settings(settings: &Value) -> Result<Option<OpenAISettings>> {
    let Some(raw_openai_settings) = settings.get("openai") else {
        return Ok(None);
    };
    if raw_openai_settings.is_null() {
        return Ok(None);
    }

    let openai_settings = serde_json::from_value::<OpenAISettings>(raw_openai_settings.clone())
        .map_err(|err| anyhow!("failed to parse settings.openai: {}", err))?;
    if !openai_settings.enabled {
        return Ok(None);
    }

    Ok(Some(openai_settings))
}

fn build_openai_instances(settings: &OpenAISettings) -> Result<Vec<OpenAIInstanceConfig>> {
    let raw_instances = if settings.instances.is_empty() {
        vec![SettingsOpenAIInstanceConfig {
            instance_id: default_instance_id(),
            provider_type: default_provider_type(),
            base_url: default_base_url(),
            auth_mode: default_auth_mode(),
            api_token: None,
            auth_subject: None,
            auth_appid: None,
            auth_private_key_path: None,
            timeout_ms: default_timeout_ms(),
            models: vec![],
            default_model: None,
            image_models: vec![],
            default_image_model: None,
            features: vec![],
            alias_map: HashMap::new(),
        }]
    } else {
        settings.instances.clone()
    };

    let mut instances = vec![];
    for raw_instance in raw_instances.into_iter() {
        let auth_mode = raw_instance.auth_mode.trim().to_ascii_lowercase();
        let resolved_token = raw_instance
            .api_token
            .clone()
            .or_else(|| Some(settings.api_token.clone()))
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if (auth_mode.is_empty() || auth_mode == DEFAULT_AUTH_MODE) && resolved_token.is_none() {
            return Err(anyhow!(
                "openai instance {} requires api_token when auth_mode is bearer",
                raw_instance.instance_id
            ));
        }

        let mut models = normalize_model_list(raw_instance.models);
        if models.is_empty() {
            models = normalize_model_list(parse_csv_list(DEFAULT_OPENAI_MODELS));
        }
        if models.is_empty() {
            return Err(anyhow!(
                "openai instance {} has no models configured",
                raw_instance.instance_id
            ));
        }

        let default_model = raw_instance
            .default_model
            .or_else(|| {
                models
                    .iter()
                    .find(|model| !is_text2image_model_name(model))
                    .cloned()
            })
            .or_else(|| models.first().cloned());
        let mut image_models = normalize_model_list(raw_instance.image_models);
        if image_models.is_empty() {
            image_models = models
                .iter()
                .filter(|model| is_text2image_model_name(model))
                .cloned()
                .collect::<Vec<_>>();
        }
        if image_models.is_empty() {
            image_models = normalize_model_list(parse_csv_list(DEFAULT_OPENAI_IMAGE_MODELS));
        }
        let default_image_model = raw_instance
            .default_image_model
            .or_else(|| image_models.first().cloned());
        let features = if raw_instance.features.is_empty() {
            default_features()
        } else {
            raw_instance.features
        };

        instances.push(OpenAIInstanceConfig {
            instance_id: raw_instance.instance_id,
            provider_type: raw_instance.provider_type,
            base_url: raw_instance.base_url,
            auth_mode: raw_instance.auth_mode,
            api_token: resolved_token,
            auth_subject: raw_instance.auth_subject,
            auth_appid: raw_instance.auth_appid,
            auth_private_key_path: raw_instance.auth_private_key_path,
            timeout_ms: raw_instance.timeout_ms,
            models,
            default_model,
            image_models,
            default_image_model,
            features,
            alias_map: raw_instance.alias_map,
        });
    }

    Ok(instances)
}

fn register_default_aliases(
    center: &AIComputeCenter,
    provider_type: &str,
    models: &[String],
    default_model: Option<&str>,
    image_models: &[String],
    default_image_model: Option<&str>,
) {
    for model in models.iter() {
        if is_text2image_model_name(model) {
            continue;
        }
        center.model_catalog().set_mapping(
            Capability::LlmRouter,
            model.as_str(),
            provider_type,
            model.as_str(),
        );

        center.model_catalog().set_mapping(
            Capability::LlmRouter,
            format!("llm.{}", model),
            provider_type,
            model.as_str(),
        );
    }

    if let Some(default_model) = default_model.filter(|model| !is_text2image_model_name(model)) {
        for alias in [
            "llm.default",
            "llm.chat.default",
            "llm.plan.default",
            "llm.code.default",
        ] {
            center.model_catalog().set_mapping(
                Capability::LlmRouter,
                alias,
                provider_type,
                default_model,
            );
        }
    }

    for model in image_models.iter() {
        center.model_catalog().set_mapping(
            Capability::Text2Image,
            model.as_str(),
            provider_type,
            model.as_str(),
        );

        for alias in [
            format!("text2image.{}", model),
            format!("t2i.{}", model),
            format!("image.{}", model),
        ] {
            center.model_catalog().set_mapping(
                Capability::Text2Image,
                alias,
                provider_type,
                model.as_str(),
            );
        }
    }

    if let Some(default_image_model) = default_image_model {
        for alias in ["text2image.default", "t2i.default", "image.default"] {
            center.model_catalog().set_mapping(
                Capability::Text2Image,
                alias,
                provider_type,
                default_image_model,
            );
        }
    }
}

fn register_custom_aliases(
    center: &AIComputeCenter,
    provider_type: &str,
    alias_map: &HashMap<String, String>,
) {
    for (alias, model) in alias_map.iter() {
        let normalized_alias = alias.to_ascii_lowercase();
        let capability = if normalized_alias.starts_with("text2image.")
            || normalized_alias.starts_with("t2i.")
            || normalized_alias.starts_with("image.")
        {
            Capability::Text2Image
        } else {
            Capability::LlmRouter
        };
        center.model_catalog().set_mapping(
            capability,
            alias.as_str(),
            provider_type,
            model.as_str(),
        );
    }
}

pub fn register_openai_llm_providers(center: &AIComputeCenter, settings: &Value) -> Result<usize> {
    let Some(openai_settings) = parse_openai_settings(settings)? else {
        center.registry().clear();
        center.model_catalog().clear();
        info!("aicc openai provider is disabled (settings.openai missing or disabled)");
        return Ok(0);
    };
    let instances = build_openai_instances(&openai_settings)?;
    let mut prepared = Vec::<(OpenAIInstanceConfig, Arc<dyn Provider>)>::new();
    for config in instances.iter() {
        let provider = OpenAIProvider::new(config.clone())?;
        prepared.push((config.clone(), Arc::new(provider)));
    }

    center.registry().clear();
    center.model_catalog().clear();

    for (config, provider) in prepared.into_iter() {
        center.registry().add_provider(provider);

        register_default_aliases(
            center,
            config.provider_type.as_str(),
            &config.models,
            config.default_model.as_deref(),
            &config.image_models,
            config.default_image_model.as_deref(),
        );

        register_custom_aliases(
            center,
            config.provider_type.as_str(),
            &openai_settings.alias_map,
        );
        register_custom_aliases(center, config.provider_type.as_str(), &config.alias_map);

        info!(
            "registered openai instance id={} provider_type={} base_url={} models={:?} image_models={:?}",
            config.instance_id,
            config.provider_type,
            config.base_url,
            config.models,
            config.image_models
        );
    }

    Ok(instances.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aicc::ModelCatalog;
    use buckyos_api::{AiPayload, ModelSpec, Requirements};
    use serde_json::json;

    fn build_llm_request(options: Option<Value>) -> CompleteRequest {
        CompleteRequest::new(
            Capability::LlmRouter,
            ModelSpec::new("llm.default".to_string(), None),
            Requirements::default(),
            AiPayload::new(
                Some("hello world".to_string()),
                vec![],
                vec![],
                vec![],
                None,
                options,
            ),
            None,
        )
    }

    #[test]
    fn estimate_tokens_uses_max_tokens_first() {
        let request = build_llm_request(Some(json!({
            "max_tokens": 120,
            "max_completion_tokens": 456
        })));

        let (_input_tokens, output_tokens) = OpenAIProvider::estimate_tokens(&request);
        assert_eq!(output_tokens, 120);
    }

    #[test]
    fn estimate_tokens_prefers_max_output_tokens() {
        let request = build_llm_request(Some(json!({
            "max_output_tokens": 90,
            "max_tokens": 120,
            "max_completion_tokens": 456
        })));

        let (_input_tokens, output_tokens) = OpenAIProvider::estimate_tokens(&request);
        assert_eq!(output_tokens, 90);
    }

    #[test]
    fn estimate_tokens_falls_back_to_max_completion_tokens() {
        let request = build_llm_request(Some(json!({
            "max_completion_tokens": 333
        })));

        let (_input_tokens, output_tokens) = OpenAIProvider::estimate_tokens(&request);
        assert_eq!(output_tokens, 333);
    }

    #[test]
    fn estimate_tokens_defaults_output_tokens() {
        let request = build_llm_request(None);

        let (_input_tokens, output_tokens) = OpenAIProvider::estimate_tokens(&request);
        assert_eq!(output_tokens, 512);
    }

    #[test]
    fn merge_requirements_tools_adds_web_search_when_required() {
        let mut target = Map::new();
        let mut req = build_llm_request(None);
        req.requirements.must_features = vec![features::WEB_SEARCH.to_string()];

        OpenAIProvider::merge_requirements_tools(&mut target, &req)
            .expect("merge requirements tools should work");

        let value = Value::Object(target);
        assert_eq!(
            value
                .pointer("/tools/0/type")
                .and_then(|item| item.as_str()),
            Some(OPENAI_TOOL_TYPE_WEB_SEARCH)
        );
    }

    #[test]
    fn merge_requirements_tools_dedupes_existing_web_search() {
        let mut target = Map::new();
        target.insert(
            "tools".to_string(),
            json!([
                {
                    "type": "function",
                    "function": {
                        "name": "workshop_exec_bash",
                        "parameters": {
                            "type": "object"
                        }
                    }
                },
                {
                    "type": OPENAI_TOOL_TYPE_WEB_SEARCH
                }
            ]),
        );
        let mut req = build_llm_request(None);
        req.requirements.must_features = vec![features::WEB_SEARCH.to_string()];

        OpenAIProvider::merge_requirements_tools(&mut target, &req)
            .expect("merge requirements tools should work");

        let value = Value::Object(target);
        assert_eq!(
            value
                .pointer("/tools")
                .and_then(|tools| tools.as_array())
                .map(|tools| tools.len()),
            Some(2)
        );
    }

    #[test]
    fn extract_tool_choices_parses_openai_function_call() {
        let message = json!({
            "tool_calls": [{
                "id": "call_1",
                "type": "function",
                "function": {
                    "name": "workshop_exec_bash",
                    "arguments": "{\"command\":\"ls -la\"}"
                }
            }]
        });

        let tool_choices = OpenAIProvider::extract_tool_choices(&message);
        assert_eq!(tool_choices.len(), 1);
        assert_eq!(tool_choices[0].name, "workshop_exec_bash");
        assert_eq!(tool_choices[0].call_id, "call_1");
        assert_eq!(tool_choices[0].args["command"], json!("ls -la"));
    }

    #[test]
    fn extract_tool_choices_parses_responses_function_call() {
        let body = json!({
            "output": [
                {
                    "type": "function_call",
                    "call_id": "call_2",
                    "name": "workshop_exec_bash",
                    "arguments": "{\"command\":\"pwd\"}"
                }
            ]
        });

        let tool_choices = OpenAIProvider::extract_tool_choices(&body);
        assert_eq!(tool_choices.len(), 1);
        assert_eq!(tool_choices[0].name, "workshop_exec_bash");
        assert_eq!(tool_choices[0].call_id, "call_2");
        assert_eq!(tool_choices[0].args["command"], json!("pwd"));
    }

    #[test]
    fn extract_text_content_concatenates_responses_blocks_without_newline_injection() {
        let body = json!({
            "output": [
                {
                    "type": "message",
                    "content": [
                        { "type": "output_text", "text": "{\"reply\":\"hel" },
                        { "type": "output_text", "text": "lo\",\"actions\":{\"mode\":\"all\",\"cmds\":[]}}" }
                    ]
                }
            ]
        });

        let text = OpenAIProvider::extract_text_content(&body).expect("text should exist");
        let parsed: Value = serde_json::from_str(&text).expect("text should stay valid json");
        assert_eq!(
            parsed.pointer("/reply").and_then(Value::as_str),
            Some("hello")
        );
    }

    #[test]
    fn extract_text_content_trims_final_result_once() {
        let body = json!({
            "output": [
                {
                    "type": "message",
                    "content": [
                        { "type": "output_text", "text": "  {\"next_behavior\":\"END\"}" },
                        { "type": "output_text", "text": "  " }
                    ]
                }
            ]
        });

        let text = OpenAIProvider::extract_text_content(&body).expect("text should exist");
        assert_eq!(text, "{\"next_behavior\":\"END\"}");
    }

    #[test]
    fn incomplete_output_error_reports_token_limit_as_fatal() {
        let body = json!({
            "id": "resp_test",
            "status": "incomplete",
            "incomplete_details": {
                "reason": "max_output_tokens"
            },
            "output": [
                { "type": "reasoning" }
            ]
        });

        let err = OpenAIProvider::incomplete_output_error(&body, None, &[])
            .expect("incomplete response without text should return an error");
        assert!(!err.is_retryable());
        let message = err.to_string();
        assert!(message.contains("TOKEN_LIMIT_EXCEEDED"));
        assert!(message.contains("resp_test"));
    }

    #[test]
    fn incomplete_output_error_reports_token_limit_even_when_text_exists() {
        let body = json!({
            "status": "incomplete",
            "incomplete_details": {
                "reason": "max_output_tokens"
            }
        });

        let err = OpenAIProvider::incomplete_output_error(
            &body,
            Some("{\"next_behavior\":\"END\"}"),
            &[],
        );
        assert!(err.is_some());
        let err = err.expect("token limit should still be reported");
        assert!(!err.is_retryable());
        assert!(err.to_string().contains("TOKEN_LIMIT_EXCEEDED"));
    }

    #[test]
    fn incomplete_output_error_skips_non_token_incomplete_when_text_exists() {
        let body = json!({
            "status": "incomplete",
            "incomplete_details": {
                "reason": "content_filter"
            }
        });

        let err = OpenAIProvider::incomplete_output_error(
            &body,
            Some("{\"next_behavior\":\"END\"}"),
            &[],
        );
        assert!(err.is_none());
    }

    #[test]
    fn extract_unsupported_request_param_recognizes_not_supported_error() {
        let body = json!({
            "error": {
                "param": "temperature",
                "message": "Unsupported parameter: 'temperature' is not supported with this model."
            }
        });

        let param = OpenAIProvider::extract_unsupported_request_param(&body);
        assert_eq!(param.as_deref(), Some("temperature"));
    }

    #[test]
    fn remove_retryable_unsupported_option_removes_temperature() {
        let mut request_obj = Map::new();
        request_obj.insert("temperature".to_string(), json!(0.2));
        request_obj.insert("model".to_string(), json!("gpt-5.2-codex"));

        let removed =
            OpenAIProvider::remove_retryable_unsupported_option(&mut request_obj, "temperature");
        assert!(removed);
        assert!(!request_obj.contains_key("temperature"));
        assert_eq!(request_obj.get("model"), Some(&json!("gpt-5.2-codex")));
    }

    #[test]
    fn parse_sse_response_body_uses_completed_response_payload() {
        let raw = r#"event: response.created
data: {"type":"response.created","response":{"id":"resp_1","status":"in_progress"}}

event: response.output_text.delta
data: {"type":"response.output_text.delta","delta":"hel"}

event: response.output_text.delta
data: {"type":"response.output_text.delta","delta":"lo"}

event: response.completed
data: {"type":"response.completed","response":{"id":"resp_1","status":"completed","output_text":"hello"}}

data: [DONE]
"#;

        let parsed = OpenAIProvider::parse_sse_response_body(raw).expect("sse should parse");
        assert_eq!(
            parsed.get("status").and_then(|value| value.as_str()),
            Some("completed")
        );
        assert_eq!(
            parsed.get("output_text").and_then(|value| value.as_str()),
            Some("hello")
        );
    }

    #[test]
    fn parse_sse_response_body_falls_back_to_accumulated_deltas() {
        let raw = r#"data: {"type":"response.output_text.delta","delta":"foo"}

data: {"type":"response.output_text.delta","delta":"bar"}

data: [DONE]
"#;

        let parsed = OpenAIProvider::parse_sse_response_body(raw).expect("sse should parse");
        assert_eq!(
            parsed.get("status").and_then(|value| value.as_str()),
            Some("completed")
        );
        assert_eq!(
            parsed.get("output_text").and_then(|value| value.as_str()),
            Some("foobar")
        );
    }

    #[test]
    fn default_features_include_web_search() {
        let all_features = default_features();
        assert!(
            all_features
                .iter()
                .any(|feature| feature == features::WEB_SEARCH),
            "openai default features should include web_search"
        );
    }

    #[test]
    fn build_openai_instances_infers_image_models_from_dalle() {
        let settings = OpenAISettings {
            enabled: true,
            api_token: "token".to_string(),
            alias_map: HashMap::new(),
            instances: vec![SettingsOpenAIInstanceConfig {
                instance_id: "openai-1".to_string(),
                provider_type: "openai".to_string(),
                base_url: default_base_url(),
                auth_mode: default_auth_mode(),
                api_token: None,
                auth_subject: None,
                auth_appid: None,
                auth_private_key_path: None,
                timeout_ms: default_timeout_ms(),
                models: vec!["gpt-4o-mini".to_string(), "dall-e-3".to_string()],
                default_model: None,
                image_models: vec![],
                default_image_model: None,
                features: vec![],
                alias_map: HashMap::new(),
            }],
        };

        let instances = build_openai_instances(&settings).expect("instances should be built");
        assert_eq!(instances.len(), 1);
        assert_eq!(
            instances[0].default_model.as_deref(),
            Some("gpt-4o-mini"),
            "llm default should prefer non-image model"
        );
        assert_eq!(
            instances[0].image_models,
            vec!["dall-e-3".to_string()],
            "image models should infer from configured dall-e model"
        );
        assert_eq!(
            instances[0].default_image_model.as_deref(),
            Some("dall-e-3"),
            "image default should point to inferred image model"
        );
    }

    #[test]
    fn build_openai_instances_allows_device_jwt_without_api_token() {
        let settings = OpenAISettings {
            enabled: true,
            api_token: String::new(),
            alias_map: HashMap::new(),
            instances: vec![SettingsOpenAIInstanceConfig {
                instance_id: "sn-openai-1".to_string(),
                provider_type: "sn-openai".to_string(),
                base_url: "https://sn.buckyos.ai/v1".to_string(),
                auth_mode: DEVICE_JWT_AUTH_MODE.to_string(),
                api_token: None,
                auth_subject: Some("ood1".to_string()),
                auth_appid: Some("aicc".to_string()),
                auth_private_key_path: Some("/tmp/non-exist.pem".to_string()),
                timeout_ms: default_timeout_ms(),
                models: vec!["gpt-5-mini".to_string()],
                default_model: Some("gpt-5-mini".to_string()),
                image_models: vec![],
                default_image_model: None,
                features: vec![],
                alias_map: HashMap::new(),
            }],
        };

        let instances = build_openai_instances(&settings).expect("instances should be built");
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].auth_mode, DEVICE_JWT_AUTH_MODE);
        assert!(instances[0].api_token.is_none());
    }

    #[test]
    fn use_chat_completions_endpoint_detects_custom_sn_path() {
        let provider = OpenAIProvider::new(OpenAIInstanceConfig {
            instance_id: "sn-openai-1".to_string(),
            provider_type: "sn-openai".to_string(),
            base_url: "https://sn.buckyos.ai/api/v1/ai/chat/completions".to_string(),
            auth_mode: "bearer".to_string(),
            api_token: Some("token".to_string()),
            auth_subject: None,
            auth_appid: None,
            auth_private_key_path: None,
            timeout_ms: default_timeout_ms(),
            models: vec!["gpt-5-mini".to_string()],
            default_model: Some("gpt-5-mini".to_string()),
            image_models: vec![],
            default_image_model: None,
            features: vec![],
            alias_map: HashMap::new(),
        })
        .expect("provider should be built");
        assert!(provider.use_chat_completions_endpoint());
    }

    #[test]
    fn register_custom_aliases_routes_image_prefix_to_text2image() {
        let center = AIComputeCenter::new(Default::default(), ModelCatalog::default());
        let aliases = HashMap::from([
            ("llm.plan.default".to_string(), "gpt-4o-mini".to_string()),
            ("text2image.poster".to_string(), "dall-e-3".to_string()),
        ]);
        register_custom_aliases(&center, "openai", &aliases);

        let llm = center.model_catalog().resolve(
            "",
            &Capability::LlmRouter,
            "llm.plan.default",
            "openai",
        );
        let image = center.model_catalog().resolve(
            "",
            &Capability::Text2Image,
            "text2image.poster",
            "openai",
        );
        assert_eq!(llm.as_deref(), Some("gpt-4o-mini"));
        assert_eq!(image.as_deref(), Some("dall-e-3"));
    }

    #[test]
    fn register_default_aliases_exposes_code_default_not_json_default() {
        let center = AIComputeCenter::new(Default::default(), ModelCatalog::default());
        let models = vec!["gpt-4o-mini".to_string()];
        let image_models = Vec::<String>::new();
        register_default_aliases(
            &center,
            "openai",
            &models,
            Some("gpt-4o-mini"),
            &image_models,
            None,
        );

        let code_alias = center.model_catalog().resolve(
            "",
            &Capability::LlmRouter,
            "llm.code.default",
            "openai",
        );
        let removed_alias = center.model_catalog().resolve(
            "",
            &Capability::LlmRouter,
            "llm.json.default",
            "openai",
        );

        assert_eq!(code_alias.as_deref(), Some("gpt-4o-mini"));
        assert!(removed_alias.is_none());
    }

    #[test]
    fn parse_text2image_artifacts_supports_url_and_base64() {
        let body = json!({
            "data": [
                {
                    "url": "https://example.com/test.png",
                    "revised_prompt": "a cat with glasses"
                },
                {
                    "b64_json": "aGVsbG8="
                }
            ]
        });

        let artifacts =
            OpenAIProvider::parse_text2image_artifacts(&body).expect("artifacts should parse");
        assert_eq!(artifacts.len(), 2);
        match &artifacts[0].resource {
            ResourceRef::Url { url, .. } => assert_eq!(url, "https://example.com/test.png"),
            other => panic!("unexpected first artifact resource: {:?}", other),
        }
        match &artifacts[1].resource {
            ResourceRef::Base64 { data_base64, .. } => assert_eq!(data_base64, "aGVsbG8="),
            other => panic!("unexpected second artifact resource: {:?}", other),
        }
    }
}
