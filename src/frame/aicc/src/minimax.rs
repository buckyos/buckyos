use crate::aicc::{
    AIComputeCenter, CostEstimate, Provider, ProviderError, ProviderInstance, ProviderStartResult,
    ResolvedRequest, TaskEventSink,
};
use crate::claude_protocol::convert_complete_request;
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use buckyos_api::{
    features, AiCost, AiResponseSummary, AiToolCall, AiUsage, Capability, CompleteRequest, Feature,
};
use log::{info, warn};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

const DEFAULT_MINIMAX_BASE_URL: &str = "https://api.minimaxi.com/anthropic/v1";
const DEFAULT_MINIMAX_TIMEOUT_MS: u64 = 60_000;
const DEFAULT_MINIMAX_MODELS: &str =
    "MiniMax-M2.5,MiniMax-M2.5-highspeed,MiniMax-M2.1,MiniMax-M2.1-highspeed,MiniMax-M2";

#[derive(Debug, Clone)]
pub struct MiniMaxInstanceConfig {
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
pub struct MiniMaxProvider {
    instance: ProviderInstance,
    client: Client,
    api_token: String,
    base_url: String,
}

impl MiniMaxProvider {
    pub fn new(cfg: MiniMaxInstanceConfig, api_token: String) -> Result<Self> {
        let timeout_ms = if cfg.timeout_ms == 0 {
            DEFAULT_MINIMAX_TIMEOUT_MS
        } else {
            cfg.timeout_ms
        };

        let client = Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .context("failed to build reqwest client for minimax provider")?;

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
        let lowered = model.to_ascii_lowercase();
        if lowered.contains("m1") {
            (1.20, 4.80)
        } else if lowered.contains("coding") || lowered.contains("plan") {
            (0.90, 3.60)
        } else {
            (0.80, 3.20)
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

        let input_tokens = ((text_len as f64) / 4.0).ceil() as u64;
        let output_tokens = req
            .payload
            .options
            .as_ref()
            .and_then(|value| value.get("max_tokens").and_then(|value| value.as_u64()))
            .unwrap_or(1024);

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

    fn extract_text_content(body: &Value) -> Option<String> {
        let content = body.get("content")?.as_array()?;
        let joined = content
            .iter()
            .filter(|item| item.get("type").and_then(|value| value.as_str()) == Some("text"))
            .filter_map(|item| item.get("text").and_then(|value| value.as_str()))
            .collect::<Vec<_>>()
            .join("\n");
        if joined.trim().is_empty() {
            None
        } else {
            Some(joined)
        }
    }

    fn extract_tool_calls(body: &Value) -> Vec<AiToolCall> {
        body.get("content")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter(|item| item.get("type").and_then(|value| value.as_str()) == Some("tool_use"))
                    .filter_map(|item| {
                        Some(AiToolCall {
                            name: item.get("name")?.as_str()?.to_string(),
                            call_id: item.get("id")?.as_str()?.to_string(),
                            args: item
                                .get("input")?
                                .as_object()?
                                .clone()
                                .into_iter()
                                .collect(),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    fn classify_api_error(status: StatusCode, message: String) -> ProviderError {
        if status.as_u16() == 429 || status.is_server_error() {
            ProviderError::retryable(message)
        } else {
            ProviderError::fatal(message)
        }
    }

    async fn start_llm(
        &self,
        ctx: &crate::aicc::InvokeCtx,
        provider_model: &str,
        req: &CompleteRequest,
    ) -> std::result::Result<ProviderStartResult, ProviderError> {
        let (request_obj, _ignored) = convert_complete_request(req, provider_model)?;
        let request_value = Value::Object(request_obj.clone());
        let endpoint = format!("{}/messages", self.base_url);

        let response = self
            .client
            .post(&endpoint)
            .header("content-type", "application/json")
            .header("anthropic-version", "2023-06-01")
            .header("x-api-key", self.api_token.as_str())
            .json(&request_value)
            .send()
            .await
            .map_err(|error| ProviderError::retryable(format!("minimax request failed: {}", error)))?;

        let status = response.status();
        let body = response.json::<Value>().await.map_err(|error| {
            ProviderError::retryable(format!("minimax response decode failed: {}", error))
        })?;

        if !status.is_success() {
            let message = body
                .pointer("/error/message")
                .and_then(|value| value.as_str())
                .or_else(|| body.get("message").and_then(|value| value.as_str()))
                .unwrap_or("minimax api returned non-success status")
                .to_string();
            warn!(
                "aicc.minimax.llm.error instance_id={} model={} trace_id={:?} status={} body={}",
                self.instance.instance_id,
                provider_model,
                ctx.trace_id,
                status.as_u16(),
                body
            );
            return Err(Self::classify_api_error(status, message));
        }

        let content = Self::extract_text_content(&body);
        let tool_calls = Self::extract_tool_calls(&body);
        let usage = body.get("usage").map(|usage| AiUsage {
            input_tokens: usage.get("input_tokens").and_then(|value| value.as_u64()),
            output_tokens: usage.get("output_tokens").and_then(|value| value.as_u64()),
            total_tokens: usage
                .get("input_tokens")
                .and_then(|value| value.as_u64())
                .zip(usage.get("output_tokens").and_then(|value| value.as_u64()))
                .map(|(input, output)| input.saturating_add(output)),
        });
        let cost = usage
            .as_ref()
            .and_then(|usage| self.estimate_cost_for_usage(provider_model, usage));

        let mut extra = Map::new();
        extra.insert("provider".to_string(), Value::String("minimax".to_string()));
        extra.insert("model".to_string(), Value::String(provider_model.to_string()));
        extra.insert(
            "provider_io".to_string(),
            json!({
                "input": request_value,
                "output": body,
            }),
        );

        Ok(ProviderStartResult::Immediate(AiResponseSummary {
            text: content,
            tool_calls,
            artifacts: vec![],
            usage,
            cost,
            finish_reason: body
                .get("stop_reason")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string()),
            provider_task_ref: None,
            extra: Some(Value::Object(extra)),
        }))
    }
}

#[async_trait]
impl Provider for MiniMaxProvider {
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

        CostEstimate {
            estimated_cost_usd: self
                .estimate_cost_for_usage(provider_model, &usage)
                .map(|cost| cost.amount),
            estimated_latency_ms: Some(1400),
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
            Capability::LlmRouter => self.start_llm(&ctx, provider_model.as_str(), &req.request).await,
            capability => Err(ProviderError::fatal(format!(
                "minimax provider does not support capability '{:?}'",
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
struct MiniMaxSettings {
    #[serde(default = "default_minimax_enabled")]
    enabled: bool,
    #[serde(default, alias = "api_key", alias = "apiKey")]
    api_token: String,
    #[serde(default)]
    alias_map: HashMap<String, String>,
    #[serde(default)]
    instances: Vec<SettingsMiniMaxInstanceConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct SettingsMiniMaxInstanceConfig {
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

fn default_minimax_enabled() -> bool {
    false
}

fn default_instance_id() -> String {
    "minimax-default".to_string()
}

fn default_provider_type() -> String {
    "minimax".to_string()
}

fn default_base_url() -> String {
    DEFAULT_MINIMAX_BASE_URL.to_string()
}

fn default_timeout_ms() -> u64 {
    DEFAULT_MINIMAX_TIMEOUT_MS
}

fn default_features() -> Vec<String> {
    vec![
        features::PLAN.to_string(),
        features::JSON_OUTPUT.to_string(),
        features::TOOL_CALLING.to_string(),
    ]
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

fn parse_minimax_settings(settings: &Value) -> Result<Option<MiniMaxSettings>> {
    let Some(raw) = settings.get("minimax") else {
        info!("aicc minimax settings missing at settings.minimax");
        return Ok(None);
    };
    if raw.is_null() {
        info!("aicc minimax settings present but null");
        return Ok(None);
    }

    let minimax_settings = serde_json::from_value::<MiniMaxSettings>(raw.clone())
        .map_err(|err| anyhow!("failed to parse minimax settings: {}", err))?;
    info!(
        "aicc minimax settings parsed enabled={} api_key_present={} instances={} alias_map_keys={}",
        minimax_settings.enabled,
        !minimax_settings.api_token.trim().is_empty(),
        minimax_settings.instances.len(),
        minimax_settings.alias_map.len(),
    );
    if !minimax_settings.enabled {
        info!("aicc minimax provider disabled by settings.minimax.enabled=false");
        return Ok(None);
    }

    Ok(Some(minimax_settings))
}

fn build_minimax_instances(settings: &MiniMaxSettings) -> Result<Vec<MiniMaxInstanceConfig>> {
    let raw_instances = if settings.instances.is_empty() {
        vec![SettingsMiniMaxInstanceConfig {
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
        let mut models = normalize_model_list(raw_instance.models);
        if models.is_empty() {
            models = normalize_model_list(parse_csv_list(DEFAULT_MINIMAX_MODELS));
        }
        if models.is_empty() {
            return Err(anyhow!(
                "minimax instance {} has no models configured",
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

        instances.push(MiniMaxInstanceConfig {
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
}

fn register_custom_aliases(
    center: &AIComputeCenter,
    provider_type: &str,
    alias_map: &HashMap<String, String>,
) {
    for (alias, model) in alias_map.iter() {
        center.model_catalog().set_mapping(
            Capability::LlmRouter,
            alias.as_str(),
            provider_type,
            model.as_str(),
        );
    }
}

pub fn register_minimax_providers(center: &AIComputeCenter, settings: &Value) -> Result<usize> {
    let Some(minimax_settings) = parse_minimax_settings(settings)? else {
        info!("aicc minimax provider is disabled (settings.minimax missing or disabled)");
        return Ok(0);
    };
    if minimax_settings.api_token.trim().is_empty() {
        warn!("aicc minimax provider enabled but api_token is empty");
        return Err(anyhow!(
            "settings.minimax.api_token (or api_key) is required when minimax provider is enabled"
        ));
    }

    let instances = build_minimax_instances(&minimax_settings)?;
    info!(
        "aicc minimax registering instances={} default_models={:?}",
        instances.len(),
        instances
            .iter()
            .map(|item| item.default_model.clone().unwrap_or_default())
            .collect::<Vec<_>>(),
    );
    let mut prepared = Vec::<(MiniMaxInstanceConfig, Arc<dyn Provider>)>::new();
    for config in instances.iter() {
        let provider = MiniMaxProvider::new(config.clone(), minimax_settings.api_token.clone())?;
        prepared.push((config.clone(), Arc::new(provider)));
    }

    for (config, provider) in prepared.into_iter() {
        center.registry().add_provider(provider);
        register_default_aliases(
            center,
            config.provider_type.as_str(),
            &config.models,
            config.default_model.as_deref(),
        );
        register_custom_aliases(
            center,
            config.provider_type.as_str(),
            &minimax_settings.alias_map,
        );
        register_custom_aliases(center, config.provider_type.as_str(), &config.alias_map);

        info!(
            "registered minimax instance id={} provider_type={} base_url={} models={:?}",
            config.instance_id,
            config.provider_type,
            config.base_url,
            config.models,
        );
    }

    Ok(instances.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aicc::ModelCatalog;

    #[test]
    fn build_minimax_instances_uses_defaults() {
        let settings = MiniMaxSettings {
            enabled: true,
            api_token: "token".to_string(),
            alias_map: HashMap::new(),
            instances: vec![],
        };

        let instances = build_minimax_instances(&settings).expect("instances should build");
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].provider_type, "minimax");
        assert_eq!(instances[0].default_model.as_deref(), Some("MiniMax-M2.5"));
    }

    #[test]
    fn register_minimax_aliases_exposes_defaults() {
        let center = AIComputeCenter::new(Default::default(), ModelCatalog::default());
        let settings = json!({
            "minimax": {
                "enabled": true,
                "api_token": "token",
                "instances": [
                    {
                        "instance_id": "minimax-main",
                        "provider_type": "minimax",
                        "base_url": "https://api.minimaxi.com/anthropic/v1",
                        "models": ["MiniMax-M2.5"],
                        "default_model": "MiniMax-M2.5"
                    }
                ]
            }
        });

        let count = register_minimax_providers(&center, &settings).expect("register should work");
        assert_eq!(count, 1);
        assert_eq!(
            center
                .model_catalog()
                .resolve("", &Capability::LlmRouter, "llm.plan.default", "minimax")
                .as_deref(),
            Some("MiniMax-M2.5")
        );
    }
}
