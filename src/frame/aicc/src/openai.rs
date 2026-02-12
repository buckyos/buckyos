use crate::aicc::{
    AIComputeCenter, CostEstimate, Provider, ProviderError, ProviderInstance, ProviderStartResult,
    ResolvedRequest, TaskEventSink,
};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use buckyos_api::{
    features, AiCost, AiResponseSummary, AiUsage, Capability, CompleteRequest, Feature, ResourceRef,
};
use log::info;
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_OPENAI_TIMEOUT_MS: u64 = 60_000;
const DEFAULT_OPENAI_MODELS: &str = "gpt-5,gpt-5-mini,gpt-5-nono,gpt-5-pro";

#[derive(Debug, Clone)]
pub struct OpenAIInstanceConfig {
    pub instance_id: String,
    pub provider_type: String,
    pub base_url: String,
    pub timeout_ms: u64,
    pub models: Vec<String>,
    pub default_model: Option<String>,
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
            capabilities: vec![Capability::LlmRouter],
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
            .and_then(|value| value.get("max_tokens"))
            .and_then(|value| value.as_u64())
            .unwrap_or(512);

        (input_tokens.max(1), output_tokens.max(1))
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

    fn merge_options(target: &mut Map<String, Value>, options: &Value) {
        let Some(options_map) = options.as_object() else {
            return;
        };

        for (key, value) in options_map.iter() {
            if key == "model" || key == "messages" {
                continue;
            }
            target.insert(key.clone(), value.clone());
        }
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
}

#[async_trait]
impl Provider for OpenAIProvider {
    fn instance(&self) -> &ProviderInstance {
        &self.instance
    }

    fn estimate_cost(&self, req: &CompleteRequest, provider_model: &str) -> CostEstimate {
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
        _ctx: crate::aicc::InvokeCtx,
        provider_model: String,
        req: ResolvedRequest,
        _sink: Arc<dyn TaskEventSink>,
    ) -> std::result::Result<ProviderStartResult, ProviderError> {
        if req.request.capability != Capability::LlmRouter {
            return Err(ProviderError::fatal(
                "openai provider currently supports capability llm_router only",
            ));
        }

        let messages = self.build_messages(&req.request)?;

        let mut request_obj = Map::new();
        request_obj.insert("model".to_string(), Value::String(provider_model.clone()));
        request_obj.insert("messages".to_string(), Value::Array(messages));

        if let Some(options) = req.request.payload.options.as_ref() {
            Self::merge_options(&mut request_obj, options);
        }

        let url = format!("{}/chat/completions", self.base_url);
        let response = self
            .client
            .post(url)
            .bearer_auth(self.api_token.as_str())
            .json(&request_obj)
            .send()
            .await
            .map_err(|err| {
                if err.is_timeout() || err.is_connect() {
                    ProviderError::retryable(format!("openai request failed: {}", err))
                } else {
                    ProviderError::fatal(format!("openai request failed: {}", err))
                }
            })?;

        let status = response.status();
        let body: Value = response.json().await.map_err(|err| {
            if status.as_u16() == 429 || status.is_server_error() {
                ProviderError::retryable(format!("failed to parse openai response body: {}", err))
            } else {
                ProviderError::fatal(format!("failed to parse openai response body: {}", err))
            }
        })?;

        if !status.is_success() {
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
            .request
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
            .and_then(|usage| self.estimate_cost_for_usage(provider_model.as_str(), usage));

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
            extra: body
                .pointer("/choices/0/message/tool_calls")
                .cloned()
                .map(|tool_calls| json!({ "tool_calls": tool_calls })),
        };

        Ok(ProviderStartResult::Immediate(summary))
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
            features: vec![],
            alias_map: HashMap::new(),
        }]
    } else {
        settings.instances.clone()
    };

    let mut instances = vec![];
    for raw_instance in raw_instances.into_iter() {
        let mut models = raw_instance.models;
        if models.is_empty() {
            models = parse_csv_list(DEFAULT_OPENAI_MODELS);
        }
        if models.is_empty() {
            return Err(anyhow!(
                "openai instance {} has no models configured",
                raw_instance.instance_id
            ));
        }

        let default_model = raw_instance
            .default_model
            .or_else(|| models.first().cloned());
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
) {
    for model in models.iter() {
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

    if let Some(default_model) = default_model {
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
}

pub fn register_openai_llm_providers(center: &AIComputeCenter, settings: &Value) -> Result<usize> {
    let Some(openai_settings) = parse_openai_settings(settings)? else {
        info!("aicc openai provider is disabled (settings.openai missing or disabled)");
        return Ok(0);
    };
    if openai_settings.api_token.trim().is_empty() {
        return Err(anyhow!(
            "settings.openai.api_token is required when openai provider is enabled"
        ));
    }
    let instances = build_openai_instances(&openai_settings)?;

    for config in instances.iter() {
        let provider = OpenAIProvider::new(config.clone(), openai_settings.api_token.clone())?;
        center.registry().add_provider(Arc::new(provider));

        register_default_aliases(
            center,
            config.provider_type.as_str(),
            &config.models,
            config.default_model.as_deref(),
        );

        for (alias, model) in openai_settings.alias_map.iter() {
            center.model_catalog().set_mapping(
                Capability::LlmRouter,
                alias.as_str(),
                config.provider_type.as_str(),
                model.as_str(),
            );
        }

        for (alias, model) in config.alias_map.iter() {
            center.model_catalog().set_mapping(
                Capability::LlmRouter,
                alias.as_str(),
                config.provider_type.as_str(),
                model.as_str(),
            );
        }

        info!(
            "registered openai instance id={} provider_type={} base_url={} models={:?}",
            config.instance_id, config.provider_type, config.base_url, config.models
        );
    }

    Ok(instances.len())
}
