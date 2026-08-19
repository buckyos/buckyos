use crate::aicc::{
    provider_type_from_settings, redacted_json_log, AIComputeCenter, InvokeCtx, Provider,
    ProviderError, ProviderInstance, ProviderRefreshTask, ProviderStartResult, ResolvedRequest,
    TaskEventSink,
};
use crate::metadata_resolver::{
    driver_metadata_model_ids, driver_model_has_specific_metadata, max_driver_metadata_cost,
    resolve_driver_inventory, DriverModelResolveRequest,
};
use crate::model_types::{
    ApiType, CostEstimateInput, CostEstimateOutput, ModelMetadata, PricingMode, ProviderInventory,
    ProviderOrigin, ProviderType, ProviderTypeTrustedSource, QuotaState,
};
use crate::openai_protocol::{
    apply_provider_model_defaults, merge_options, merge_requirements_response_format,
    merge_tool_calls, strip_incompatible_sampling_options,
};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use buckyos_api::{
    ai_methods, get_buckyos_api_runtime, value_to_object_map, AiContent, AiMethodRequest,
    AiResponse, AiRole, AiToolCall, AiToolResultContent, AiUsage, Capability, ResourceRef,
};
use log::{error, info, warn};
use reqwest::header::{CONTENT_ENCODING, CONTENT_TYPE};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::error::Error as _;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::Duration;
use tokio::sync::watch;
use tokio::time;

const SN_AI_PROVIDER_SETTINGS_KEY: &str = "sn-ai-provider";
const SN_AI_PROVIDER_DRIVER: &str = "sn-ai-provider";
const DEFAULT_SN_AI_PROVIDER_BASE_URL: &str = "https://sn.buckyos.ai/api/v1/ai/";
const DEFAULT_SN_AI_PROVIDER_TIMEOUT_MS: u64 = 300_000;
const DEFAULT_INVENTORY_REFRESH_INTERVAL_SECS: u64 = 300;

#[derive(Debug, Deserialize, Default)]
struct SnAIProviderSettings {
    #[serde(default = "default_enabled")]
    enabled: bool,
    #[serde(default)]
    instances: Vec<SettingsSnAIProviderInstanceConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct SettingsSnAIProviderInstanceConfig {
    #[serde(default = "default_instance_id", alias = "instance_id")]
    provider_instance_name: String,
    #[serde(default = "default_provider_type")]
    provider_type: String,
    #[serde(default = "default_base_url")]
    base_url: String,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
}

#[derive(Debug, Clone)]
struct SnAIProviderInstanceConfig {
    provider_instance_name: String,
    provider_type: String,
    base_url: String,
    timeout_ms: u64,
}

#[derive(Debug, Clone)]
pub struct SnAIProvider {
    instance: ProviderInstance,
    inventory: Arc<RwLock<ProviderInventory>>,
    client: Client,
    base_url: String,
    provider_type: ProviderType,
    inventory_refresh_interval: Duration,
    refresh_task: Arc<Mutex<Option<Arc<ProviderRefreshTask>>>>,
}

#[derive(Debug, Deserialize)]
struct SnModelsResponse {
    #[serde(default)]
    data: Vec<SnModelEntry>,
}

#[derive(Debug, Deserialize)]
struct SnModelEntry {
    id: String,
}

fn default_enabled() -> bool {
    true
}

fn default_instance_id() -> String {
    "sn-ai-provider-default".to_string()
}

fn default_provider_type() -> String {
    "cloud_api".to_string()
}

fn default_base_url() -> String {
    DEFAULT_SN_AI_PROVIDER_BASE_URL.to_string()
}

fn default_timeout_ms() -> u64 {
    DEFAULT_SN_AI_PROVIDER_TIMEOUT_MS
}

fn normalize_model_list(models: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::new();
    for model in models {
        let model = model.trim();
        if !model.is_empty() && !normalized.iter().any(|item: &String| item == model) {
            normalized.push(model.to_string());
        }
    }
    normalized
}

fn parse_sn_ai_provider_settings(settings: &Value) -> Result<Option<SnAIProviderSettings>> {
    let Some(raw_settings) = settings.get(SN_AI_PROVIDER_SETTINGS_KEY) else {
        return Ok(None);
    };
    if raw_settings.is_null() {
        return Ok(None);
    }

    let parsed = serde_json::from_value::<SnAIProviderSettings>(raw_settings.clone())
        .map_err(|err| anyhow!("failed to parse settings.sn-ai-provider: {}", err))?;
    if !parsed.enabled {
        return Ok(None);
    }

    Ok(Some(parsed))
}

fn build_sn_ai_provider_instances(
    settings: &SnAIProviderSettings,
) -> Result<Vec<SnAIProviderInstanceConfig>> {
    let raw_instances = if settings.instances.is_empty() {
        vec![SettingsSnAIProviderInstanceConfig {
            provider_instance_name: default_instance_id(),
            provider_type: default_provider_type(),
            base_url: default_base_url(),
            timeout_ms: default_timeout_ms(),
        }]
    } else {
        settings.instances.clone()
    };

    raw_instances
        .into_iter()
        .map(|raw_instance| {
            Ok(SnAIProviderInstanceConfig {
                provider_instance_name: raw_instance.provider_instance_name,
                provider_type: raw_instance.provider_type,
                base_url: raw_instance.base_url,
                timeout_ms: raw_instance.timeout_ms,
            })
        })
        .collect()
}

impl SnAIProvider {
    fn new(cfg: SnAIProviderInstanceConfig) -> Result<Self> {
        let timeout_ms = if cfg.timeout_ms == 0 {
            DEFAULT_SN_AI_PROVIDER_TIMEOUT_MS
        } else {
            cfg.timeout_ms
        };
        let client = Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .context("failed to build reqwest client for sn-ai-provider")?;
        let provider_type = provider_type_from_settings(cfg.provider_type.as_str());
        let instance = ProviderInstance {
            provider_instance_name: cfg.provider_instance_name.clone(),
            provider_type: provider_type.clone(),
            provider_driver: SN_AI_PROVIDER_DRIVER.to_string(),
            provider_origin: ProviderOrigin::SystemConfig,
            provider_type_trusted_source: ProviderTypeTrustedSource::SystemConfig,
            provider_type_revision: None,
            capabilities: vec![Capability::Llm],
            features: vec![],
            endpoint: Some(cfg.base_url.clone()),
            plugin_key: None,
        };
        let models = driver_metadata_model_ids(SN_AI_PROVIDER_DRIVER, &ApiType::Llm);
        let inventory = Self::build_inventory(
            cfg.provider_instance_name.as_str(),
            provider_type.clone(),
            models.as_slice(),
            Some("default-v1".to_string()),
        );
        Ok(Self {
            instance,
            inventory: Arc::new(RwLock::new(inventory)),
            client,
            base_url: cfg.base_url.trim_end_matches('/').to_string(),
            provider_type,
            inventory_refresh_interval: Duration::from_secs(
                DEFAULT_INVENTORY_REFRESH_INTERVAL_SECS,
            ),
            refresh_task: Arc::new(Mutex::new(None)),
        })
    }

    fn start_inventory_refresh(self: Arc<Self>) {
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }

        let (refresh_task, shutdown_rx) = ProviderRefreshTask::new();
        let existing = match self.refresh_task.lock() {
            Ok(mut current) => current.replace(refresh_task.clone()),
            Err(_) => {
                warn!(
                    "aicc.sn_ai_provider.inventory.refresh_task_lock_poisoned provider_instance_name={}",
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
        let refresh_interval = self.inventory_refresh_interval;
        tokio::spawn(async move {
            Self::run_inventory_refresh(
                provider,
                provider_instance_name,
                refresh_task,
                shutdown_rx,
                refresh_interval,
            )
            .await;
        });
    }

    async fn run_inventory_refresh(
        provider: Weak<Self>,
        provider_instance_name: String,
        refresh_task: Arc<ProviderRefreshTask>,
        mut shutdown_rx: watch::Receiver<bool>,
        refresh_interval: Duration,
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
                    "aicc.sn_ai_provider.inventory.initial_refresh_failed provider_instance_name={} err={}",
                    provider_instance_name, err
                );
        }

        let mut interval = time::interval(refresh_interval);
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
                        "aicc.sn_ai_provider.inventory.refresh_failed provider_instance_name={} err={}",
                        provider_instance_name, err
                    );
                    }
                }
            }
        }
    }

    fn stop_inventory_refresh(&self) {
        if let Ok(mut current) = self.refresh_task.lock() {
            if let Some(task) = current.take() {
                task.shutdown();
            }
        }
    }

    fn build_inventory(
        provider_instance_name: &str,
        provider_type: ProviderType,
        models: &[String],
        revision: Option<String>,
    ) -> ProviderInventory {
        let requests = models
            .iter()
            .map(|model| DriverModelResolveRequest::new(model.clone(), vec![ApiType::Llm]))
            .collect::<Vec<_>>();
        let mut inventory = resolve_driver_inventory(
            provider_instance_name,
            provider_type,
            SN_AI_PROVIDER_DRIVER,
            requests.as_slice(),
            revision,
        );
        inventory
            .models
            .retain(|model| model.supports_api_type(&ApiType::Llm));
        inventory
    }

    fn normalize_remote_provider_inventory(
        &self,
        inventory: ProviderInventory,
    ) -> ProviderInventory {
        let models = inventory
            .models
            .iter()
            .map(|model| {
                model
                    .provider_actual_model_id
                    .clone()
                    .unwrap_or_else(|| model.provider_model_id.clone())
            })
            .collect::<Vec<_>>();
        let mut normalized = Self::build_inventory(
            self.instance.provider_instance_name.as_str(),
            self.provider_type.clone(),
            models.as_slice(),
            inventory.inventory_revision.clone(),
        );
        normalized.version = inventory.version;
        normalized
    }

    fn build_inventory_from_remote_value(&self, body: Value) -> Result<ProviderInventory> {
        if body
            .get("models")
            .and_then(|value| value.as_array())
            .is_some()
        {
            let inventory = serde_json::from_value::<ProviderInventory>(body)
                .context("failed to parse sn-ai-provider inventory response")?;
            return Ok(self.normalize_remote_provider_inventory(inventory));
        }

        let mut model_ids = Vec::<String>::new();
        if let Some(items) = body.get("items").and_then(Value::as_array) {
            for item in items {
                if let Some(model) = item
                    .get("model")
                    .or_else(|| item.get("id"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    model_ids.push(model.to_string());
                }
            }
        } else {
            let response = serde_json::from_value::<SnModelsResponse>(body)
                .context("failed to parse sn-ai-provider models response")?;
            model_ids.extend(response.data.into_iter().map(|entry| entry.id));
        }

        let models = normalize_model_list(model_ids);
        if models.is_empty() {
            return Err(anyhow!(
                "sn-ai-provider inventory refresh returned no llm models"
            ));
        }
        Ok(Self::build_inventory(
            self.instance.provider_instance_name.as_str(),
            self.provider_type.clone(),
            models.as_slice(),
            Some(inventory_revision(models.as_slice())),
        ))
    }

    fn models_endpoint(&self) -> String {
        if self.base_url.to_ascii_lowercase().ends_with("/responses") {
            if let Some((prefix, _)) = self.base_url.rsplit_once('/') {
                return format!("{}/models", prefix.trim_end_matches('/'));
            }
        }
        format!("{}/models", self.base_url.trim_end_matches('/'))
    }

    fn responses_endpoint(&self) -> String {
        if self.base_url.to_ascii_lowercase().ends_with("/responses") {
            self.base_url.clone()
        } else {
            format!("{}/responses", self.base_url.trim_end_matches('/'))
        }
    }

    async fn refresh_inventory_once(&self) -> Result<ProviderInventory> {
        let token = self.build_auth_token(&InvokeCtx::default()).await?;
        let response = self
            .client
            .get(self.models_endpoint())
            .bearer_auth(token.as_str())
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "sn-ai-provider inventory refresh failed status={} body={}",
                status,
                body
            ));
        }
        let body = response
            .json::<Value>()
            .await
            .context("failed to parse sn-ai-provider inventory response")?;
        let inventory = self.build_inventory_from_remote_value(body)?;
        {
            let mut current = self
                .inventory
                .write()
                .map_err(|_| anyhow!("sn-ai-provider inventory lock poisoned"))?;
            *current = inventory.clone();
        }
        info!(
            "aicc.sn_ai_provider.inventory.refreshed provider_instance_name={} models={}",
            self.instance.provider_instance_name,
            inventory.models.len()
        );
        Ok(inventory)
    }

    async fn build_auth_token(&self, ctx: &InvokeCtx) -> Result<String, ProviderError> {
        if let Some(token) = ctx
            .session_token
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            return Ok(token.to_string());
        }
        let runtime = get_buckyos_api_runtime().map_err(|err| {
            ProviderError::fatal(format!(
                "sn-ai-provider runtime_session auth requires runtime: {}",
                err
            ))
        })?;
        let token = runtime.get_session_token().await;
        if token.trim().is_empty() {
            return Err(ProviderError::fatal(
                "sn-ai-provider runtime_session auth requires non-empty session token",
            ));
        }
        Ok(token)
    }

    fn format_error_chain(err: &reqwest::Error) -> String {
        let mut segments = vec![err.to_string()];
        let mut source = err.source();
        while let Some(cause) = source {
            segments.push(cause.to_string());
            source = cause.source();
        }
        segments.join(" | caused_by: ")
    }

    async fn post_json(
        &self,
        ctx: &InvokeCtx,
        request_obj: &Map<String, Value>,
    ) -> Result<(StatusCode, Value, u64), ProviderError> {
        let auth_token = self.build_auth_token(ctx).await?;
        let url = self.responses_endpoint();
        let started_at = std::time::Instant::now();
        let response = self
            .client
            .post(url.as_str())
            .bearer_auth(auth_token.as_str())
            .json(request_obj)
            .send()
            .await
            .map_err(|err| {
                let retryable = err.is_timeout() || err.is_connect();
                error!(
                    "aicc.sn_ai_provider.http_send_failed provider_instance_name={} url={} retryable={} status={:?} err_chain={}",
                    self.instance.provider_instance_name,
                    url,
                    retryable,
                    err.status(),
                    Self::format_error_chain(&err)
                );
                if retryable {
                    ProviderError::retryable(format!("sn-ai-provider request failed: {}", err))
                } else {
                    ProviderError::fatal(format!("sn-ai-provider request failed: {}", err))
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
            Self::classify_api_error(
                status,
                format!(
                    "failed to decode sn-ai-provider response body: {}; content_type={} content_encoding={}",
                    err,
                    content_type,
                    if content_encoding.is_empty() {
                        "<none>"
                    } else {
                        content_encoding.as_str()
                    }
                ),
            )
        })?;
        let body = serde_json::from_str::<Value>(raw_body.as_str()).map_err(|err| {
            Self::classify_api_error(
                status,
                format!(
                    "invalid sn-ai-provider json response: {}; body_head={}",
                    err,
                    raw_body.chars().take(320).collect::<String>()
                ),
            )
        })?;
        Ok((status, body, latency_ms))
    }

    fn classify_api_error(status: StatusCode, message: String) -> ProviderError {
        let lower = message.to_ascii_lowercase();
        if status.as_u16() == 401 {
            return ProviderError::fatal(format!(
                "{}; SN AI Provider requires a valid BuckyOS runtime session through the SN AI gateway",
                message
            ));
        }
        if status.as_u16() == 403
            && (lower.contains("no active token gateway key")
                || lower.contains("token gateway key")
                || lower.contains("not active")
                || lower.contains("not available"))
        {
            return ProviderError::fatal(format!(
                "{}; SN AI Provider is not active for this device/user yet. It requires SN relay traffic mode and invite-code activation. Activation may have been completed from another client, so retry after that state syncs.",
                message
            ));
        }
        if status.as_u16() == 429 || status.is_server_error() {
            ProviderError::retryable(message)
        } else {
            ProviderError::fatal(message)
        }
    }

    fn estimate_cost_for_usage(&self, model: &str, usage: &AiUsage) -> Option<buckyos_api::AiCost> {
        let input_tokens = usage.input_tokens?;
        let output_tokens = usage.output_tokens?;
        let pricing = self.inventory.read().ok().and_then(|inventory| {
            inventory.models.iter().find_map(|metadata| {
                (metadata.provider_model_id == model).then(|| {
                    (
                        metadata.pricing.currency.clone(),
                        metadata.pricing.input_token,
                        metadata.pricing.output_token,
                        metadata.pricing.estimated_cost,
                    )
                })
            })
        });
        if !driver_model_has_specific_metadata(SN_AI_PROVIDER_DRIVER, model) {
            return None;
        }
        if let Some((currency, Some(input_price), Some(output_price), _)) = pricing.as_ref() {
            return Some(buckyos_api::AiCost {
                amount: (input_tokens as f64 * input_price) + (output_tokens as f64 * output_price),
                currency: currency.clone(),
            });
        }
        let (currency, _, _, estimated_cost) = pricing?;
        Some(buckyos_api::AiCost {
            amount: estimated_cost?,
            currency,
        })
    }

    fn ai_role_to_openai(role: &AiRole) -> &'static str {
        match role {
            AiRole::System => "system",
            AiRole::Developer => "developer",
            AiRole::User => "user",
            AiRole::Assistant => "assistant",
            _ => "user",
        }
    }

    fn resource_text(resource: &ResourceRef) -> String {
        match resource {
            ResourceRef::Url { url, .. } => format!("resource_url: {}", url),
            ResourceRef::NamedObject { obj_id } => format!("named_object: {}", obj_id),
            ResourceRef::Base64 { mime, data_base64 } => {
                format!("data:{};base64,{}", mime, data_base64)
            }
        }
    }

    fn responses_image_part(source: &ResourceRef) -> Value {
        match source {
            ResourceRef::Url { url, .. } => json!({
                "type": "input_image",
                "image_url": url,
            }),
            ResourceRef::Base64 { mime, data_base64 } => json!({
                "type": "input_image",
                "image_url": format!("data:{};base64,{}", mime, data_base64),
            }),
            ResourceRef::NamedObject { obj_id } => json!({
                "type": "input_text",
                "text": format!("named_object: {}", obj_id),
            }),
        }
    }

    fn build_messages(req: &AiMethodRequest) -> Result<Vec<Value>, ProviderError> {
        let mut items: Vec<Value> = vec![];
        for msg in &req.payload.messages {
            let role_str = Self::ai_role_to_openai(&msg.role);
            if role_str == "assistant" {
                let provider_items = msg
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        AiContent::ProviderState { provider, value }
                            if provider.eq_ignore_ascii_case(SN_AI_PROVIDER_DRIVER) =>
                        {
                            Some(value.clone())
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                if provider_items.is_empty() {
                    if msg.text_content().trim().is_empty() && msg.tool_calls().is_empty() {
                        continue;
                    }
                    return Err(ProviderError::fatal(
                        "assistant history is missing Responses output items for provider `sn-ai-provider`",
                    ));
                }
                items.extend(provider_items);
                continue;
            }
            let content_type = if role_str == "assistant" {
                "output_text"
            } else {
                "input_text"
            };
            let mut pending_text_parts: Vec<Value> = Vec::new();
            for block in &msg.content {
                match block {
                    AiContent::Text { text } if !text.is_empty() => {
                        pending_text_parts.push(json!({
                            "type": content_type,
                            "text": text,
                        }));
                    }
                    AiContent::ToolUse {
                        call_id,
                        name,
                        args,
                    } => {
                        if !pending_text_parts.is_empty() {
                            items.push(json!({
                                "role": role_str,
                                "content": std::mem::take(&mut pending_text_parts),
                            }));
                        }
                        items.push(json!({
                            "type": "function_call",
                            "call_id": call_id,
                            "name": name,
                            "arguments": serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string()),
                        }));
                    }
                    AiContent::ToolResult {
                        call_id, content, ..
                    } => {
                        if !pending_text_parts.is_empty() {
                            items.push(json!({
                                "role": role_str,
                                "content": std::mem::take(&mut pending_text_parts),
                            }));
                        }
                        let output = content
                            .iter()
                            .filter_map(|item| match item {
                                AiToolResultContent::Text { text } => Some(text.as_str()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        items.push(json!({
                            "type": "function_call_output",
                            "call_id": call_id,
                            "output": output,
                        }));
                    }
                    AiContent::Image { source } if role_str == "user" => {
                        pending_text_parts.push(Self::responses_image_part(source));
                    }
                    AiContent::Document { source, title } if role_str == "user" => {
                        let mut text = Self::resource_text(source);
                        if let Some(title) = title.as_deref().filter(|value| !value.is_empty()) {
                            text = format!("{} ({})", text, title);
                        }
                        pending_text_parts.push(json!({
                            "type": content_type,
                            "text": text,
                        }));
                    }
                    _ => {}
                }
            }
            if !pending_text_parts.is_empty() {
                items.push(json!({
                    "role": role_str,
                    "content": pending_text_parts,
                }));
            }
        }

        if items.is_empty() {
            let mut content = req.payload.text.clone().unwrap_or_default();
            let resource_lines = req
                .payload
                .resources
                .iter()
                .map(Self::resource_text)
                .collect::<Vec<_>>();
            if !resource_lines.is_empty() {
                if !content.is_empty() {
                    content.push_str("\n\n");
                }
                content.push_str(resource_lines.join("\n").as_str());
            }
            if !content.trim().is_empty() {
                items.push(json!({
                    "role": "user",
                    "content": [{ "type": "input_text", "text": content }],
                }));
            }
        }

        if items.is_empty() {
            return Err(ProviderError::fatal(
                "request payload has no usable text/messages for sn-ai-provider llm",
            ));
        }
        Ok(items)
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
                            "aicc.sn_ai_provider {} is invalid json arguments: {}",
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
        if let Some(text) = payload.get("output_text").and_then(Value::as_str) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }

        let mut parts = Vec::new();
        if let Some(output_items) = payload.get("output").and_then(Value::as_array) {
            for item in output_items {
                let Some(item_obj) = item.as_object() else {
                    continue;
                };
                if item_obj.get("type").and_then(Value::as_str) == Some("output_text") {
                    if let Some(text) = item_obj.get("text").and_then(Value::as_str) {
                        if !text.trim().is_empty() {
                            parts.push(text.to_string());
                        }
                    }
                    continue;
                }
                if item_obj.get("type").and_then(Value::as_str) != Some("message") {
                    continue;
                }
                if let Some(content_items) = item_obj.get("content").and_then(Value::as_array) {
                    for content_item in content_items {
                        let Some(content_obj) = content_item.as_object() else {
                            continue;
                        };
                        let content_type = content_obj.get("type").and_then(Value::as_str);
                        if content_type != Some("output_text") && content_type != Some("text") {
                            continue;
                        }
                        if let Some(text) = content_obj.get("text").and_then(Value::as_str) {
                            if !text.trim().is_empty() {
                                parts.push(text.to_string());
                            }
                        }
                    }
                }
            }
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.concat().trim().to_string())
        }
    }

    fn extract_tool_choices(payload: &Value) -> Vec<AiToolCall> {
        let mut tool_choices = Vec::new();
        let Some(items) = payload.get("output").and_then(Value::as_array) else {
            return tool_choices;
        };
        for (idx, item) in items.iter().enumerate() {
            let Some(item_obj) = item.as_object() else {
                continue;
            };
            if item_obj.get("type").and_then(Value::as_str) != Some("function_call") {
                continue;
            }
            let Some(call_id) = item_obj
                .get("call_id")
                .or_else(|| item_obj.get("id"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
            else {
                warn!(
                    "aicc.sn_ai_provider output[{}] function_call missing id",
                    idx
                );
                continue;
            };
            let Some(name) = item_obj
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                warn!(
                    "aicc.sn_ai_provider output[{}] function_call missing name",
                    idx
                );
                continue;
            };
            let Some(args) = Self::parse_tool_arguments(
                item_obj
                    .get("arguments")
                    .or_else(|| item_obj.get("args"))
                    .cloned()
                    .unwrap_or(Value::Null),
                format!("output[{}].arguments", idx).as_str(),
            ) else {
                continue;
            };
            if !args.is_object() {
                warn!(
                    "aicc.sn_ai_provider output[{}].arguments must decode to an object",
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

    fn message_from_response_body(
        body: &Value,
        text: Option<String>,
        tool_calls: Vec<AiToolCall>,
    ) -> buckyos_api::AiMessage {
        let mut message = AiResponse::message_from_parts(text, tool_calls, vec![]);
        if let Some(output_items) = body.get("output").and_then(Value::as_array) {
            message
                .content
                .extend(
                    output_items
                        .iter()
                        .cloned()
                        .map(|value| AiContent::ProviderState {
                            provider: SN_AI_PROVIDER_DRIVER.to_string(),
                            value,
                        }),
                );
        }
        message
    }

    async fn start_llm(
        &self,
        ctx: &InvokeCtx,
        provider_model: &str,
        req: &AiMethodRequest,
    ) -> Result<ProviderStartResult, ProviderError> {
        let mut request_obj = Map::new();
        request_obj.insert(
            "model".to_string(),
            Value::String(provider_model.to_string()),
        );
        request_obj.insert(
            "input".to_string(),
            Value::Array(Self::build_messages(req)?),
        );

        let mut ignored_options = vec![];
        if let Some(input_json) = req.payload.input_json.as_ref() {
            ignored_options.extend(merge_options(&mut request_obj, input_json)?);
        }
        if let Some(options) = req.payload.options.as_ref() {
            ignored_options.extend(merge_options(&mut request_obj, options)?);
        }
        apply_provider_model_defaults(&mut request_obj, provider_model);
        let stripped_options =
            strip_incompatible_sampling_options(&mut request_obj, provider_model);
        if !stripped_options.is_empty() {
            info!(
                "aicc.sn_ai_provider omitted incompatible llm options: provider_instance_name={} model={} trace_id={:?} omitted={:?}",
                self.instance.provider_instance_name, provider_model, ctx.trace_id, stripped_options
            );
        }
        merge_requirements_response_format(&mut request_obj, req);
        merge_tool_calls(&mut request_obj, req.payload.tool_specs.as_slice())?;
        if !ignored_options.is_empty() {
            warn!(
                "aicc.sn_ai_provider ignored unsupported llm options: provider_instance_name={} model={} trace_id={:?} ignored={:?}",
                self.instance.provider_instance_name, provider_model, ctx.trace_id, ignored_options
            );
        }

        info!(
            "aicc.sn_ai_provider.llm.input provider_instance_name={} model={} trace_id={:?} request={}",
            self.instance.provider_instance_name,
            provider_model,
            ctx.trace_id,
            redacted_json_log(&Value::Object(request_obj.clone()))
        );
        let (status, body, latency_ms) = self.post_json(ctx, &request_obj).await?;
        let response_log = redacted_json_log(&body);

        if !status.is_success() {
            warn!(
                "aicc.sn_ai_provider.llm.output provider_instance_name={} model={} trace_id={:?} status={} response={}",
                self.instance.provider_instance_name,
                provider_model,
                ctx.trace_id,
                status.as_u16(),
                response_log
            );
            let message = body
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("sn-ai-provider api returned non-success status")
                .to_string();
            let code = body
                .pointer("/error/code")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            return Err(Self::classify_api_error(
                status,
                format!("sn-ai-provider api error [{}]: {}", code, message),
            ));
        }
        info!(
            "aicc.sn_ai_provider.llm.output provider_instance_name={} model={} trace_id={:?} status={} response={}",
            self.instance.provider_instance_name,
            provider_model,
            ctx.trace_id,
            status.as_u16(),
            response_log
        );

        let content = Self::extract_text_content(&body);
        let tool_choices = Self::extract_tool_choices(&body);
        let usage = body.get("usage").map(|usage| AiUsage {
            input_tokens: usage
                .get("input_tokens")
                .and_then(Value::as_u64)
                .or_else(|| usage.get("prompt_tokens").and_then(Value::as_u64)),
            output_tokens: usage
                .get("output_tokens")
                .and_then(Value::as_u64)
                .or_else(|| usage.get("completion_tokens").and_then(Value::as_u64)),
            total_tokens: usage.get("total_tokens").and_then(Value::as_u64),
            request_units: None,
        });
        let cost = usage
            .as_ref()
            .and_then(|usage| self.estimate_cost_for_usage(provider_model, usage));

        let summary = AiResponse {
            message: Self::message_from_response_body(&body, content, tool_choices),
            usage,
            cost,
            finish_reason: body
                .get("status")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            provider_task_ref: body
                .get("id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            extra: Some(json!({
                "provider": SN_AI_PROVIDER_DRIVER,
                "model": provider_model,
                "latency_ms": latency_ms,
                "provider_io": {
                    "input": Value::Object(request_obj),
                    "output": body
                }
            })),
        };
        Ok(ProviderStartResult::Immediate(summary))
    }
}

fn provider_model_from_exact(exact_model: &str) -> String {
    exact_model
        .split_once('@')
        .map(|(model, _)| model)
        .unwrap_or(exact_model)
        .to_string()
}

fn inventory_revision(models: &[String]) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for model in models {
        model.hash(&mut hasher);
    }
    format!("sn-ai-provider-{:x}", hasher.finish())
}

#[async_trait]
impl Provider for SnAIProvider {
    fn inventory(&self) -> ProviderInventory {
        self.inventory
            .read()
            .map(|inventory| inventory.clone())
            .unwrap_or_else(|_| {
                warn!("sn-ai-provider inventory lock poisoned, returning empty inventory");
                ProviderInventory {
                    provider_instance_name: self.instance.provider_instance_name.clone(),
                    provider_type: self.provider_type.clone(),
                    provider_driver: SN_AI_PROVIDER_DRIVER.to_string(),
                    provider_origin: ProviderOrigin::SystemConfig,
                    provider_type_trusted_source: ProviderTypeTrustedSource::SystemConfig,
                    provider_type_revision: None,
                    driver_metadata_generation: 0,
                    version: None,
                    inventory_revision: None,
                    models: Vec::<ModelMetadata>::new(),
                }
            })
    }

    fn legacy_instance(&self) -> Option<&ProviderInstance> {
        Some(&self.instance)
    }

    fn shutdown(&self) {
        self.stop_inventory_refresh();
    }

    fn estimate_cost(&self, input: &CostEstimateInput) -> CostEstimateOutput {
        let usage = AiUsage {
            input_tokens: Some(input.input_tokens.max(1)),
            output_tokens: Some(input.estimated_output_tokens.unwrap_or(1024).max(1)),
            total_tokens: None,
            request_units: None,
        };
        let provider_model = provider_model_from_exact(input.exact_model.as_str());
        let estimated_cost_usd = self
            .estimate_cost_for_usage(provider_model.as_str(), &usage)
            .map(|cost| cost.amount)
            .or_else(|| {
                max_driver_metadata_cost(
                    SN_AI_PROVIDER_DRIVER,
                    usage.input_tokens.unwrap_or_default(),
                    usage.output_tokens.unwrap_or_default(),
                )
                .map(|(amount, _)| amount)
            })
            .unwrap_or_default();
        CostEstimateOutput {
            estimated_cost_usd,
            pricing_mode: PricingMode::PerToken,
            quota_state: QuotaState::Normal,
            confidence: 0.7,
            estimated_latency_ms: Some(1200),
        }
    }

    async fn refresh_inventory(&self) -> std::result::Result<ProviderInventory, ProviderError> {
        self.refresh_inventory_once()
            .await
            .map_err(|err| ProviderError::retryable(err.to_string()))
    }

    async fn start(
        &self,
        ctx: InvokeCtx,
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
                "sn-ai-provider does not support method '{}'",
                method
            ))),
        }
    }

    async fn cancel(
        &self,
        _ctx: InvokeCtx,
        _task_id: &str,
    ) -> std::result::Result<(), ProviderError> {
        Ok(())
    }
}

impl Drop for SnAIProvider {
    fn drop(&mut self) {
        self.stop_inventory_refresh();
    }
}

pub fn register_sn_ai_provider(center: &AIComputeCenter, settings: &Value) -> Result<usize> {
    let Some(sn_settings) = parse_sn_ai_provider_settings(settings)? else {
        info!("aicc sn-ai-provider is disabled (settings.sn-ai-provider missing or disabled)");
        return Ok(0);
    };
    let instances = build_sn_ai_provider_instances(&sn_settings)?;
    let mut prepared = Vec::<(SnAIProviderInstanceConfig, Arc<SnAIProvider>)>::new();
    for config in instances.iter() {
        let provider = Arc::new(SnAIProvider::new(config.clone())?);
        prepared.push((config.clone(), provider));
    }

    for (config, provider) in prepared.into_iter() {
        provider.clone().start_inventory_refresh();
        let inventory = center.registry().add_provider(provider);
        info!(
            "registered sn-ai-provider base_url={} inventory={:?}",
            config.base_url, inventory
        );
        center
            .model_registry()
            .write()
            .map_err(|_| anyhow!("model registry lock poisoned"))?
            .apply_inventory(inventory)
            .map_err(|err| anyhow!("failed to apply sn-ai-provider inventory: {}", err))?;
    }

    Ok(instances.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::{AiMessage, AiPayload, ModelSpec, Requirements};
    use serde_json::json;

    fn test_instance_config() -> SnAIProviderInstanceConfig {
        SnAIProviderInstanceConfig {
            provider_instance_name: "sn-ai-provider-1".to_string(),
            provider_type: "cloud_api".to_string(),
            base_url: default_base_url(),
            timeout_ms: default_timeout_ms(),
        }
    }

    #[test]
    fn build_sn_ai_provider_instances_uses_endpoint_config() {
        let settings = SnAIProviderSettings {
            enabled: true,
            instances: vec![SettingsSnAIProviderInstanceConfig {
                provider_instance_name: "sn-ai-provider-1".to_string(),
                provider_type: "cloud_api".to_string(),
                base_url: "https://sn.buckyos.ai/api/v1/ai/".to_string(),
                timeout_ms: default_timeout_ms(),
            }],
        };

        let instances = build_sn_ai_provider_instances(&settings).expect("instances");
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].provider_instance_name, "sn-ai-provider-1");
    }

    #[test]
    fn provider_uses_metadata_models_without_hardcoded_features() {
        let instances = build_sn_ai_provider_instances(&SnAIProviderSettings {
            enabled: true,
            instances: vec![],
        })
        .expect("instances");

        assert_eq!(instances.len(), 1);
        let provider = SnAIProvider::new(instances[0].clone()).expect("provider");
        assert_eq!(
            provider
                .inventory()
                .models
                .iter()
                .filter(|model| !model.provider_model_id.contains(':'))
                .map(|model| model.provider_model_id.clone())
                .collect::<Vec<_>>(),
            driver_metadata_model_ids(SN_AI_PROVIDER_DRIVER, &ApiType::Llm)
        );
        assert!(provider.instance.features.is_empty());
    }

    #[test]
    fn parse_sn_ai_provider_settings_accepts_instance_id_alias() {
        let settings = json!({
            "sn-ai-provider": {
                "enabled": true,
                "instances": [
                    {
                        "instance_id": "sn-ai-provider-alias"
                    }
                ]
            }
        });

        let parsed = parse_sn_ai_provider_settings(&settings)
            .expect("parse")
            .expect("settings");
        assert_eq!(
            parsed.instances[0].provider_instance_name,
            "sn-ai-provider-alias"
        );
    }

    #[test]
    fn responses_endpoint_appends_responses_to_base_url() {
        let provider = SnAIProvider::new(test_instance_config()).expect("provider");
        assert_eq!(
            provider.responses_endpoint(),
            "https://sn.buckyos.ai/api/v1/ai/responses"
        );
        assert_eq!(
            provider.models_endpoint(),
            "https://sn.buckyos.ai/api/v1/ai/models"
        );
    }

    #[test]
    fn build_messages_keeps_user_image_blocks() {
        let request = AiMethodRequest::new(
            Capability::Llm,
            ModelSpec::new("llm.default".to_string(), None),
            Requirements::default(),
            AiPayload::new(
                None,
                vec![AiMessage::new(
                    AiRole::User,
                    vec![
                        AiContent::Text {
                            text: "what is in this image?".to_string(),
                        },
                        AiContent::Image {
                            source: ResourceRef::Base64 {
                                mime: "image/png".to_string(),
                                data_base64: "aGVsbG8=".to_string(),
                            },
                        },
                    ],
                )],
                vec![],
                vec![],
                None,
                None,
            ),
            None,
        );

        let messages = SnAIProvider::build_messages(&request).expect("messages");
        assert_eq!(
            messages[0]
                .pointer("/content/0/type")
                .and_then(Value::as_str),
            Some("input_text")
        );
        assert_eq!(
            messages[0]
                .pointer("/content/1/type")
                .and_then(Value::as_str),
            Some("input_image")
        );
    }

    #[test]
    fn remote_items_inventory_is_normalized_as_sn_provider() {
        let provider = SnAIProvider::new(test_instance_config()).expect("provider");

        let inventory = provider
            .build_inventory_from_remote_value(json!({
                "items": [
                    {"model": "gpt-5.4"},
                    {"id": "gpt-5.4-mini"},
                    {"model": "vendor-new-model"}
                ]
            }))
            .expect("inventory");

        assert_eq!(inventory.provider_driver, SN_AI_PROVIDER_DRIVER);
        assert!(inventory
            .models
            .iter()
            .any(|model| model.provider_model_id == "gpt-5.4"));
        assert!(inventory
            .models
            .iter()
            .any(|model| model.provider_model_id == "vendor-new-model"));
    }

    #[test]
    fn cost_estimate_uses_model_price_then_metadata_maximum_for_unknown_model() {
        let provider = SnAIProvider::new(test_instance_config()).expect("provider");
        {
            let mut inventory = provider.inventory.write().expect("inventory");
            let metadata = inventory
                .models
                .iter_mut()
                .find(|metadata| metadata.provider_model_id == "gpt-5.4")
                .expect("configured model metadata");
            metadata.pricing.currency = "USD".to_string();
            metadata.pricing.input_token = Some(0.001);
            metadata.pricing.output_token = Some(0.002);
        }
        let known = provider.estimate_cost(&CostEstimateInput {
            api_type: ApiType::Llm,
            exact_model: "gpt-5.4@sn-ai-provider-1".to_string(),
            input_tokens: 1_000,
            estimated_output_tokens: Some(1_000),
            cached_input_tokens: None,
            request_features: vec![],
        });
        let unknown = provider.estimate_cost(&CostEstimateInput {
            api_type: ApiType::Llm,
            exact_model: "unknown@sn-ai-provider-1".to_string(),
            input_tokens: 1_000,
            estimated_output_tokens: Some(1_000),
            cached_input_tokens: None,
            request_features: vec![],
        });

        assert_eq!(known.estimated_cost_usd, 3.0);
        assert_eq!(unknown.estimated_cost_usd, 1.2);
        assert!(provider
            .estimate_cost_for_usage(
                "unknown",
                &AiUsage {
                    input_tokens: Some(1_000),
                    output_tokens: Some(1_000),
                    total_tokens: Some(2_000),
                    request_units: None,
                },
            )
            .is_none());
    }

    #[test]
    fn responses_output_extracts_text_and_tool_calls() {
        let body = json!({
            "output": [
                {
                    "type": "message",
                    "content": [{"type": "output_text", "text": "hello"}]
                },
                {
                    "type": "function_call",
                    "call_id": "call-1",
                    "name": "lookup",
                    "arguments": "{\"query\":\"buckyos\"}"
                }
            ]
        });

        assert_eq!(
            SnAIProvider::extract_text_content(&body).as_deref(),
            Some("hello")
        );
        let calls = SnAIProvider::extract_tool_choices(&body);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].call_id, "call-1");
        assert_eq!(calls[0].name, "lookup");
        assert_eq!(calls[0].args.get("query"), Some(&json!("buckyos")));

        let message =
            SnAIProvider::message_from_response_body(&body, Some("hello".to_string()), calls);
        assert!(message.content.iter().any(|content| matches!(
            content,
            AiContent::ProviderState { provider, .. } if provider == SN_AI_PROVIDER_DRIVER
        )));

        let request = AiMethodRequest::new(
            Capability::Llm,
            ModelSpec::new("llm.default".to_string(), None),
            Requirements::default(),
            AiPayload::new(None, vec![message], vec![], vec![], None, None),
            None,
        );
        let replay = SnAIProvider::build_messages(&request).expect("provider state replay");
        assert_eq!(
            replay,
            body["output"].as_array().expect("output array").clone()
        );
    }

    #[tokio::test]
    async fn runtime_session_auth_prefers_invocation_token() {
        let provider = SnAIProvider::new(test_instance_config()).expect("provider");
        let ctx = InvokeCtx {
            session_token: Some("caller-session".to_string()),
            ..Default::default()
        };

        assert_eq!(
            provider.build_auth_token(&ctx).await.expect("token"),
            "caller-session"
        );
    }

    #[test]
    fn dropping_provider_stops_refresh_task() {
        let provider = SnAIProvider::new(test_instance_config()).expect("provider");
        let (refresh_task, _) = ProviderRefreshTask::new();
        *provider.refresh_task.lock().expect("refresh task lock") = Some(refresh_task.clone());

        drop(provider);

        assert!(refresh_task.is_stopped());
    }
}
