use crate::aicc::{
    provider_type_from_settings, AIComputeCenter, Provider, ProviderError, ProviderInstance,
    ProviderRefreshTask, ProviderStartResult, ResolvedRequest, TaskEventSink,
};
use crate::claude_protocol::{convert_complete_request_with_dialect, ProtocolDialect};
use crate::metadata_resolver::{resolve_driver_inventory, DriverModelResolveRequest};
use crate::model_registry::DEFAULT_INVENTORY_REFRESH_INTERVAL;
use crate::model_types::{
    ApiType, CostEstimateInput, CostEstimateOutput, PricingMode, ProviderInventory, ProviderOrigin,
    ProviderTypeTrustedSource, QuotaState,
};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use buckyos_api::{
    ai_methods, features, AiCost, AiMethodRequest, AiResponse, AiToolCall, AiUsage, Capability,
    Feature,
};
use log::{info, warn};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::Duration;
use tokio::sync::watch;
use tokio::time;

const DEFAULT_MINIMAX_BASE_URL: &str = "https://api.minimaxi.com/anthropic/v1";
const DEFAULT_MINIMAX_TIMEOUT_MS: u64 = 60_000;
const DEFAULT_MINIMAX_MODELS: &str =
    "MiniMax-M2.5,MiniMax-M2.5-highspeed,MiniMax-M2.1,MiniMax-M2.1-highspeed,MiniMax-M2";

#[derive(Debug, Clone)]
pub struct MiniMaxInstanceConfig {
    pub provider_instance_name: String,
    pub provider_type: String,
    pub provider_driver: String,
    pub api_token: String,
    pub base_url: String,
    pub timeout_ms: u64,
    pub models: Vec<String>,
    pub default_model: Option<String>,
    pub features: Vec<Feature>,
    #[allow(dead_code)]
    pub alias_map: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct MiniMaxProvider {
    instance: ProviderInstance,
    inventory: Arc<RwLock<ProviderInventory>>,
    inventory_requests: Vec<DriverModelResolveRequest>,
    refresh_task: Arc<Mutex<Option<Arc<ProviderRefreshTask>>>>,
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

        let provider_type = provider_type_from_settings(cfg.provider_type.as_str());
        let provider_instance_name = cfg.provider_instance_name.clone();
        let provider_driver = cfg.provider_driver.clone();
        let instance = ProviderInstance {
            provider_instance_name: provider_instance_name.clone(),
            provider_type: provider_type.clone(),
            provider_driver: provider_driver.clone(),
            provider_origin: ProviderOrigin::SystemConfig,
            provider_type_trusted_source: ProviderTypeTrustedSource::SystemConfig,
            provider_type_revision: None,
            capabilities: vec![Capability::Llm],
            features: cfg.features.clone(),
            endpoint: Some(cfg.base_url.clone()),
            plugin_key: None,
        };
        let requests = cfg
            .models
            .iter()
            .map(|model| {
                DriverModelResolveRequest::new(model.clone(), vec![ApiType::Llm])
                    .with_cost(Some(0.01))
                    .with_latency(Some(1400))
            })
            .collect::<Vec<_>>();
        let inventory = resolve_driver_inventory(
            provider_instance_name.as_str(),
            provider_type,
            provider_driver.as_str(),
            requests.as_slice(),
            Some("settings-v1".to_string()),
        );

        Ok(Self {
            instance,
            inventory: Arc::new(RwLock::new(inventory)),
            inventory_requests: requests,
            refresh_task: Arc::new(Mutex::new(None)),
            client,
            api_token,
            base_url: cfg.base_url.trim_end_matches('/').to_string(),
        })
    }

    pub fn start_inventory_refresh(self: Arc<Self>) {
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        let (refresh_task, shutdown_rx) = ProviderRefreshTask::new();
        let existing = match self.refresh_task.lock() {
            Ok(mut current) => current.replace(refresh_task.clone()),
            Err(_) => {
                warn!(
                    "aicc.minimax.inventory.refresh_task_lock_poisoned provider_instance_name={}",
                    self.instance.provider_instance_name
                );
                return;
            }
        };
        if let Some(existing) = existing {
            existing.shutdown();
        }
        let provider = Arc::downgrade(&self);
        let provider_instance_name = self.instance.provider_instance_name.clone();
        tokio::spawn(async move {
            Self::run_inventory_refresh(
                provider,
                provider_instance_name,
                refresh_task,
                shutdown_rx,
            )
            .await;
        });
    }

    async fn run_inventory_refresh(
        provider: Weak<Self>,
        provider_instance_name: String,
        refresh_task: Arc<ProviderRefreshTask>,
        mut shutdown_rx: watch::Receiver<bool>,
    ) {
        if !refresh_task.try_start_request() {
            return;
        }
        let Some(current) = provider.upgrade() else {
            refresh_task.shutdown();
            return;
        };
        let initial_result = current.refresh_inventory_once().await;
        refresh_task.finish_request();
        drop(current);
        if let Err(err) = initial_result {
            warn!(
                "aicc.minimax.inventory.initial_refresh_failed provider_instance_name={} err={}",
                provider_instance_name, err
            );
        }

        let mut interval = time::interval(DEFAULT_INVENTORY_REFRESH_INTERVAL);
        interval.tick().await;
        loop {
            tokio::select! {
                biased;
                changed = shutdown_rx.changed() => {
                    if changed.is_err() || *shutdown_rx.borrow() {
                        return;
                    }
                }
                _ = interval.tick() => {
                    if !refresh_task.try_start_request() {
                        return;
                    }
                    let Some(current) = provider.upgrade() else {
                        refresh_task.shutdown();
                        return;
                    };
                    let result = current.refresh_inventory_once().await;
                    refresh_task.finish_request();
                    drop(current);
                    if let Err(err) = result {
                        warn!(
                            "aicc.minimax.inventory.refresh_failed provider_instance_name={} err={}",
                            provider_instance_name, err
                        );
                    }
                }
            }
        }
    }

    fn stop_inventory_refresh(&self) {
        match self.refresh_task.lock() {
            Ok(mut current) => {
                if let Some(task) = current.take() {
                    task.shutdown();
                }
            }
            Err(_) => warn!(
                "aicc.minimax.inventory.refresh_task_lock_poisoned provider_instance_name={}",
                self.instance.provider_instance_name
            ),
        }
    }

    async fn refresh_inventory_once(&self) -> Result<ProviderInventory> {
        let inventory = resolve_driver_inventory(
            self.instance.provider_instance_name.as_str(),
            self.instance.provider_type.clone(),
            self.instance.provider_driver.as_str(),
            self.inventory_requests.as_slice(),
            Some("settings-v1".to_string()),
        );
        *self
            .inventory
            .write()
            .map_err(|_| anyhow!("minimax inventory lock poisoned"))? = inventory.clone();
        Ok(inventory)
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

    #[cfg(test)]
    #[allow(dead_code)]
    fn estimate_tokens(req: &AiMethodRequest) -> (u64, u64) {
        let mut text_len = 0usize;

        if let Some(text) = req.payload.text.as_ref() {
            text_len += text.len();
        }

        for message in req.payload.messages.iter() {
            text_len += message.estimate_text_len();
        }
        if let Some(input_json) = req.payload.input_json.as_ref() {
            text_len += json_text_len(input_json);
        }

        let input_tokens = ((text_len as f64) / 4.0).ceil() as u64;
        let output_tokens = req
            .payload
            .input_json
            .as_ref()
            .and_then(|value| {
                value
                    .get("max_output_tokens")
                    .and_then(|value| value.as_u64())
                    .or_else(|| value.get("max_tokens").and_then(|value| value.as_u64()))
            })
            .or_else(|| {
                req.payload
                    .options
                    .as_ref()
                    .and_then(|value| value.get("max_tokens").and_then(|value| value.as_u64()))
            })
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
                    .filter(|item| {
                        item.get("type").and_then(|value| value.as_str()) == Some("tool_use")
                    })
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
        req: &AiMethodRequest,
    ) -> std::result::Result<ProviderStartResult, ProviderError> {
        let (request_obj, _ignored) = convert_complete_request_with_dialect(
            req,
            provider_model,
            ProtocolDialect::MiniMax,
            None,
        )?;
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
            .map_err(|error| {
                ProviderError::retryable(format!("minimax request failed: {}", error))
            })?;

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
                "aicc.minimax.llm.error provider_instance_name={} model={} trace_id={:?} status={} body={}",
                self.instance.provider_instance_name,
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
        extra.insert(
            "model".to_string(),
            Value::String(provider_model.to_string()),
        );
        extra.insert(
            "provider_io".to_string(),
            json!({
                "input": request_value,
                "output": body,
            }),
        );

        Ok(ProviderStartResult::Immediate(AiResponse {
            message: AiResponse::message_from_parts(content, tool_calls, vec![]),
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
    fn inventory(&self) -> ProviderInventory {
        self.inventory
            .read()
            .map(|inventory| inventory.clone())
            .unwrap_or_else(|_| {
                resolve_driver_inventory(
                    self.instance.provider_instance_name.as_str(),
                    self.instance.provider_type.clone(),
                    self.instance.provider_driver.as_str(),
                    self.inventory_requests.as_slice(),
                    Some("settings-v1".to_string()),
                )
            })
    }

    fn shutdown(&self) {
        self.stop_inventory_refresh();
    }

    fn estimate_cost(&self, input: &CostEstimateInput) -> CostEstimateOutput {
        let provider_model = provider_model_from_exact(input.exact_model.as_str());
        let input_tokens = input.input_tokens.max(1);
        let output_tokens = input.estimated_output_tokens.unwrap_or(1024).max(1);
        let usage = AiUsage {
            input_tokens: Some(input_tokens),
            output_tokens: Some(output_tokens),
            total_tokens: Some(input_tokens.saturating_add(output_tokens)),
        };

        CostEstimateOutput {
            estimated_cost_usd: self
                .estimate_cost_for_usage(provider_model, &usage)
                .map(|cost| cost.amount)
                .unwrap_or(1.0),
            pricing_mode: PricingMode::PerToken,
            quota_state: QuotaState::Normal,
            confidence: 0.7,
            estimated_latency_ms: Some(1400),
        }
    }

    async fn refresh_inventory(&self) -> std::result::Result<ProviderInventory, ProviderError> {
        self.refresh_inventory_once()
            .await
            .map_err(|err| ProviderError::retryable(err.to_string()))
    }

    async fn start(
        &self,
        ctx: crate::aicc::InvokeCtx,
        provider_model: String,
        req: ResolvedRequest,
        _sink: Arc<dyn TaskEventSink>,
    ) -> std::result::Result<ProviderStartResult, ProviderError> {
        match req.method.as_str() {
            ai_methods::LLM_CHAT => {
                self.start_llm(&ctx, provider_model.as_str(), &req.request)
                    .await
            }
            method => Err(ProviderError::fatal(format!(
                "minimax provider does not support method '{}'",
                method
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

fn provider_model_from_exact(exact_model: &str) -> &str {
    exact_model
        .rsplit_once('@')
        .map(|(model, _)| model)
        .unwrap_or(exact_model)
}

#[cfg(test)]
#[allow(dead_code)]
fn json_text_len(value: &Value) -> usize {
    match value {
        Value::String(text) => text.len(),
        Value::Array(items) => items.iter().map(json_text_len).sum(),
        Value::Object(map) => map.values().map(json_text_len).sum(),
        _ => 0,
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
    provider_instance_name: String,
    #[serde(default = "default_provider_type")]
    provider_type: String,
    #[serde(default = "default_provider_driver")]
    provider_driver: String,
    #[serde(default, alias = "api_key", alias = "apiKey")]
    api_token: String,
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
    "cloud_api".to_string()
}

fn default_provider_driver() -> String {
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
            provider_instance_name: default_instance_id(),
            provider_type: default_provider_type(),
            provider_driver: default_provider_driver(),
            api_token: settings.api_token.clone(),
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
                raw_instance.provider_instance_name
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
            provider_instance_name: raw_instance.provider_instance_name,
            provider_type: raw_instance.provider_type,
            provider_driver: raw_instance.provider_driver,
            api_token: if raw_instance.api_token.trim().is_empty() {
                settings.api_token.clone()
            } else {
                raw_instance.api_token
            },
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

#[cfg(test)]
#[allow(dead_code)]
fn register_default_aliases(
    center: &AIComputeCenter,
    provider_type: &str,
    models: &[String],
    default_model: Option<&str>,
) {
    for model in models.iter() {
        center.model_catalog().set_mapping(
            Capability::Llm,
            model.as_str(),
            provider_type,
            model.as_str(),
        );

        center.model_catalog().set_mapping(
            Capability::Llm,
            format!("llm.{}", model),
            provider_type,
            model.as_str(),
        );
    }

    if let Some(default_model) = default_model {
        for alias in ["llm.default", "llm.plan.default", "llm.code.default"] {
            center.model_catalog().set_mapping(
                Capability::Llm,
                alias,
                provider_type,
                default_model,
            );
        }
    }
}

#[cfg(test)]
#[allow(dead_code)]
fn register_custom_aliases(
    center: &AIComputeCenter,
    provider_type: &str,
    alias_map: &HashMap<String, String>,
) {
    for (alias, model) in alias_map.iter() {
        center.model_catalog().set_mapping(
            Capability::Llm,
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
    let instances = build_minimax_instances(&minimax_settings)?;
    info!(
        "aicc minimax registering instances={} default_models={:?}",
        instances.len(),
        instances
            .iter()
            .map(|item| item.default_model.clone().unwrap_or_default())
            .collect::<Vec<_>>(),
    );
    let mut prepared = Vec::<(MiniMaxInstanceConfig, Arc<MiniMaxProvider>)>::new();
    for config in instances.iter() {
        if config.api_token.trim().is_empty() {
            return Err(anyhow!(
                "minimax instance {} api_token (or api_key) is required",
                config.provider_instance_name
            ));
        }
        let provider = Arc::new(MiniMaxProvider::new(
            config.clone(),
            config.api_token.clone(),
        )?);
        prepared.push((config.clone(), provider));
    }

    for (config, provider) in prepared.into_iter() {
        provider.clone().start_inventory_refresh();
        let inventory = center.registry().add_provider(provider);
        info!(
            "registered minimax base_url={} inventory={:?}",
            config.base_url, inventory
        );
        center
            .model_registry()
            .write()
            .map_err(|_| anyhow!("model registry lock poisoned"))?
            .apply_inventory(inventory)
            .map_err(|err| anyhow!("failed to apply minimax inventory: {}", err))?;
    }

    Ok(instances.len())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(instances[0].provider_type, "cloud_api");
        assert_eq!(instances[0].provider_driver, "minimax");
        assert_eq!(instances[0].default_model.as_deref(), Some("MiniMax-M2.5"));
    }

    #[test]
    fn register_minimax_inventory_exposes_default_mounts() {
        let center = AIComputeCenter::default();
        let settings = json!({
            "minimax": {
                "enabled": true,
                "api_token": "token",
                "instances": [
                    {
                        "provider_instance_name": "minimax-main",
                        "provider_type": "cloud_api",
                        "provider_driver": "minimax",
                        "base_url": "https://api.minimaxi.com/anthropic/v1",
                        "models": ["MiniMax-M2.5"],
                        "default_model": "MiniMax-M2.5"
                    }
                ]
            }
        });

        let count = register_minimax_providers(&center, &settings).expect("register should work");
        assert_eq!(count, 1);
        let items = center
            .model_registry()
            .read()
            .expect("model registry lock")
            .default_items_for_path("llm.minimax");
        assert!(items
            .values()
            .any(|item| item.target == "MiniMax-M2.5@minimax-main"));
    }

    #[tokio::test]
    async fn minimax_inventory_refresh_lifecycle_matches_dynamic_providers() {
        let provider = Arc::new(
            MiniMaxProvider::new(
                MiniMaxInstanceConfig {
                    provider_instance_name: "minimax-refresh".to_string(),
                    provider_type: "cloud_api".to_string(),
                    provider_driver: "minimax".to_string(),
                    api_token: "test-token".to_string(),
                    base_url: DEFAULT_MINIMAX_BASE_URL.to_string(),
                    timeout_ms: DEFAULT_MINIMAX_TIMEOUT_MS,
                    models: vec!["MiniMax-M2.5".to_string()],
                    default_model: Some("MiniMax-M2.5".to_string()),
                    features: vec![],
                    alias_map: HashMap::new(),
                },
                "test-token".to_string(),
            )
            .unwrap(),
        );

        provider.clone().start_inventory_refresh();
        tokio::task::yield_now().await;
        assert!(provider.refresh_task.lock().unwrap().is_some());
        assert!(!provider
            .refresh_inventory()
            .await
            .unwrap()
            .models
            .is_empty());
        provider.shutdown();
        assert!(provider.refresh_task.lock().unwrap().is_none());
    }
}
