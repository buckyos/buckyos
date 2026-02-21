use crate::aicc::{
    AIComputeCenter, CostEstimate, Provider, ProviderError, ProviderInstance, ProviderStartResult,
    ResolvedRequest, TaskEventSink,
};
use crate::openai_protocol::merge_options;
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use base64::engine::general_purpose;
use base64::Engine as _;
use buckyos_api::{
    features, AiArtifact, AiCost, AiResponseSummary, AiUsage, Capability, CompleteRequest, Feature,
    ResourceRef,
};
use log::{info, warn};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_OPENAI_TIMEOUT_MS: u64 = 60_000;
const DEFAULT_OPENAI_MODELS: &str = "gpt-5,gpt-5-mini,gpt-5-nono,gpt-5-pro";
const DEFAULT_OPENAI_IMAGE_MODELS: &str = "dall-e-3,dall-e-2";
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
    api_token: String,
    base_url: String,
}

impl OpenAIProvider {
    pub fn new(cfg: OpenAIInstanceConfig, api_token: String) -> Result<Self> {
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

        Ok(Self {
            instance,
            client,
            api_token,
            base_url: cfg.base_url.trim_end_matches('/').to_string(),
        })
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
                    .get("max_tokens")
                    .and_then(|value| value.as_u64())
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
                    "content": content,
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

    fn extract_text_content(choice_message: &Value) -> Option<String> {
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

    fn classify_api_error(status: StatusCode, message: String) -> ProviderError {
        if status.as_u16() == 429 || status.is_server_error() {
            ProviderError::retryable(message)
        } else {
            ProviderError::fatal(message)
        }
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

    async fn post_json(
        &self,
        url: &str,
        request_obj: &Map<String, Value>,
    ) -> Result<(StatusCode, Value, u64), ProviderError> {
        let started_at = std::time::Instant::now();
        let response = self
            .client
            .post(url)
            .bearer_auth(self.api_token.as_str())
            .json(request_obj)
            .send()
            .await
            .map_err(|err| {
                if err.is_timeout() || err.is_connect() {
                    ProviderError::retryable(format!("openai request failed: {}", err))
                } else {
                    ProviderError::fatal(format!("openai request failed: {}", err))
                }
            })?;
        let latency_ms = started_at.elapsed().as_millis() as u64;

        let status = response.status();
        let body: Value = response.json().await.map_err(|err| {
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
        let messages = self.build_messages(req)?;

        let mut request_obj = Map::new();
        request_obj.insert(
            "model".to_string(),
            Value::String(provider_model.to_string()),
        );
        request_obj.insert("messages".to_string(), Value::Array(messages));

        let mut ignored_options = vec![];
        if let Some(options) = req.payload.options.as_ref() {
            ignored_options = merge_options(&mut request_obj, options)?;
        }
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

        let url = format!("{}/chat/completions", self.base_url);
        let (status, body, latency_ms) = self.post_json(url.as_str(), &request_obj).await?;
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

        let message = body
            .pointer("/choices/0/message")
            .cloned()
            .unwrap_or(Value::Null);
        let content = Self::extract_text_content(&message);

        let usage = body.get("usage").map(|usage| AiUsage {
            input_tokens: usage.get("prompt_tokens").and_then(|value| value.as_u64()),
            output_tokens: usage
                .get("completion_tokens")
                .and_then(|value| value.as_u64()),
            total_tokens: usage.get("total_tokens").and_then(|value| value.as_u64()),
        });

        let json_output_required = req
            .requirements
            .must_features
            .iter()
            .any(|feature| feature == features::JSON_OUTPUT);

        let parsed_json = if json_output_required {
            content
                .as_ref()
                .and_then(|text| serde_json::from_str::<Value>(text).ok())
        } else {
            None
        };

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
        if let Some(tool_calls) = body.pointer("/choices/0/message/tool_calls").cloned() {
            extra.insert("tool_calls".to_string(), tool_calls);
        }
        extra.insert(
            "provider_io".to_string(),
            json!({
                "input": Value::Object(request_obj.clone()),
                "output": body.clone()
            }),
        );

        let summary = AiResponseSummary {
            text: content,
            json: parsed_json,
            artifacts: vec![],
            usage,
            cost,
            finish_reason: body
                .pointer("/choices/0/finish_reason")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string()),
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
            json: None,
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

fn default_features() -> Vec<String> {
    vec![
        features::PLAN.to_string(),
        features::JSON_OUTPUT.to_string(),
        features::TOOL_CALLING.to_string(),
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
            "llm.json.default",
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
    if openai_settings.api_token.trim().is_empty() {
        return Err(anyhow!(
            "settings.openai.api_token is required when openai provider is enabled"
        ));
    }
    let instances = build_openai_instances(&openai_settings)?;
    let mut prepared = Vec::<(OpenAIInstanceConfig, Arc<dyn Provider>)>::new();
    for config in instances.iter() {
        let provider = OpenAIProvider::new(config.clone(), openai_settings.api_token.clone())?;
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
    fn build_openai_instances_infers_image_models_from_dalle() {
        let settings = OpenAISettings {
            enabled: true,
            api_token: "token".to_string(),
            alias_map: HashMap::new(),
            instances: vec![SettingsOpenAIInstanceConfig {
                instance_id: "openai-1".to_string(),
                provider_type: "openai".to_string(),
                base_url: default_base_url(),
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
