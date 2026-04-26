use crate::aicc::{
    exact_model_name, provider_type_from_settings, AIComputeCenter, Provider, ProviderError,
    ProviderInstance, ProviderStartResult, ResolvedRequest, TaskEventSink,
};
use crate::model_types::{
    ApiType, CostClass, CostEstimateInput, CostEstimateOutput, HealthStatus, LatencyClass,
    ModelAttributes, ModelCapabilities, ModelHealth, ModelMetadata, ModelPricing, PricingMode,
    PrivacyClass, ProviderInventory, ProviderOrigin, ProviderTypeTrustedSource, QuotaState,
};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use buckyos_api::{
    ai_methods, AiArtifact, AiCost, AiMethodRequest, AiResponseSummary, Capability, ResourceRef,
};
use log::{error, info, warn};
use reqwest::header::CONTENT_TYPE;
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::error::Error as _;
use std::sync::Arc;
use std::time::Duration;

const FAL_PROVIDER_SETTINGS_KEY: &str = "fal";
const DEFAULT_FAL_BASE_URL: &str = "https://fal.run";
const DEFAULT_FAL_TIMEOUT_MS: u64 = 600_000;
const FAL_PROVIDER_DRIVER: &str = "fal";

const DEFAULT_IMAGE_UPSCALE_MODEL: &str = "fal-ai/clarity-upscaler";
const DEFAULT_IMAGE_BG_REMOVE_MODEL: &str = "fal-ai/imageutils/rembg";
const DEFAULT_VIDEO_UPSCALE_MODEL: &str = "fal-ai/topaz/upscale/video";

#[derive(Debug, Clone)]
pub struct FalInstanceConfig {
    pub provider_instance_name: String,
    pub provider_type: String,
    pub base_url: String,
    pub timeout_ms: u64,
    pub image_upscale_models: Vec<String>,
    pub image_bg_remove_models: Vec<String>,
    pub video_upscale_models: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct FalProvider {
    instance: ProviderInstance,
    inventory: ProviderInventory,
    client: Client,
    api_token: String,
    base_url: String,
}

impl FalProvider {
    pub fn new(cfg: FalInstanceConfig, api_token: String) -> Result<Self> {
        let timeout_ms = if cfg.timeout_ms == 0 {
            DEFAULT_FAL_TIMEOUT_MS
        } else {
            cfg.timeout_ms
        };

        let client = Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .context("failed to build reqwest client for fal provider")?;

        let provider_type = provider_type_from_settings(cfg.provider_type.as_str());
        let provider_instance_name = cfg.provider_instance_name.clone();
        let provider_driver = FAL_PROVIDER_DRIVER.to_string();

        let instance = ProviderInstance {
            provider_instance_name: provider_instance_name.clone(),
            provider_type: provider_type.clone(),
            provider_driver: provider_driver.clone(),
            provider_origin: ProviderOrigin::SystemConfig,
            provider_type_trusted_source: ProviderTypeTrustedSource::SystemConfig,
            provider_type_revision: None,
            capabilities: vec![Capability::Image, Capability::Video],
            features: vec![],
            endpoint: Some(cfg.base_url.clone()),
            plugin_key: None,
        };

        let mut models = Vec::<ModelMetadata>::new();
        for model_id in cfg.image_upscale_models.iter() {
            models.push(build_model_metadata(
                provider_instance_name.as_str(),
                provider_type.clone(),
                model_id.as_str(),
                ApiType::ImageUpscale,
                logical_mounts_for_api(ApiType::ImageUpscale, model_id.as_str()),
                Some(0.05),
                Some(8000),
            ));
        }
        for model_id in cfg.image_bg_remove_models.iter() {
            models.push(build_model_metadata(
                provider_instance_name.as_str(),
                provider_type.clone(),
                model_id.as_str(),
                ApiType::ImageBgRemove,
                logical_mounts_for_api(ApiType::ImageBgRemove, model_id.as_str()),
                Some(0.01),
                Some(4000),
            ));
        }
        for model_id in cfg.video_upscale_models.iter() {
            models.push(build_model_metadata(
                provider_instance_name.as_str(),
                provider_type.clone(),
                model_id.as_str(),
                ApiType::VideoUpscale,
                logical_mounts_for_api(ApiType::VideoUpscale, model_id.as_str()),
                Some(0.50),
                Some(120_000),
            ));
        }

        let inventory = ProviderInventory {
            provider_instance_name,
            provider_type,
            provider_driver,
            provider_origin: ProviderOrigin::SystemConfig,
            provider_type_trusted_source: ProviderTypeTrustedSource::SystemConfig,
            provider_type_revision: None,
            version: None,
            inventory_revision: Some("settings-v1".to_string()),
            models,
        };

        Ok(Self {
            instance,
            inventory,
            client,
            api_token,
            base_url: cfg.base_url.trim_end_matches('/').to_string(),
        })
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

    fn classify_api_error(status: StatusCode, message: String) -> ProviderError {
        if status.as_u16() == 429 || status.is_server_error() {
            ProviderError::retryable(message)
        } else {
            ProviderError::fatal(message)
        }
    }

    async fn post_json(
        &self,
        ctx: &crate::aicc::InvokeCtx,
        url: &str,
        body: &Map<String, Value>,
    ) -> Result<(StatusCode, Value, u64), ProviderError> {
        let started_at = std::time::Instant::now();
        let response = self
            .client
            .post(url)
            .header("Authorization", format!("Key {}", self.api_token))
            .json(body)
            .send()
            .await
            .map_err(|err| {
                let retryable = err.is_timeout() || err.is_connect();
                error!(
                    "aicc.fal.http_send_failed provider_instance_name={} url={} retryable={} timeout={} connect={} err_chain={}",
                    self.instance.provider_instance_name,
                    url,
                    retryable,
                    err.is_timeout(),
                    err.is_connect(),
                    Self::format_error_chain(&err)
                );
                if retryable {
                    ProviderError::retryable(format!("fal request failed: {}", err))
                } else {
                    ProviderError::fatal(format!("fal request failed: {}", err))
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
        let raw_body = response.text().await.map_err(|err| {
            let message = format!(
                "failed to decode fal response body: {}; status={} content_type={}",
                err,
                status.as_u16(),
                content_type
            );
            if status.as_u16() == 429 || status.is_server_error() {
                ProviderError::retryable(message)
            } else {
                ProviderError::fatal(message)
            }
        })?;
        let body: Value = serde_json::from_str(raw_body.as_str()).map_err(|err| {
            let head = raw_body.chars().take(320).collect::<String>();
            let message = format!("invalid fal json response: {}; body_head={}", err, head);
            if status.as_u16() == 429 || status.is_server_error() {
                ProviderError::retryable(message)
            } else {
                ProviderError::fatal(message)
            }
        })?;
        info!(
            "aicc.fal.http_response provider_instance_name={} url={} status={} latency_ms={} trace_id={:?}",
            self.instance.provider_instance_name,
            url,
            status.as_u16(),
            latency_ms,
            ctx.trace_id
        );
        Ok((status, body, latency_ms))
    }

    fn build_request_body(
        method: &str,
        req: &AiMethodRequest,
    ) -> Result<Map<String, Value>, ProviderError> {
        let mut body = Map::<String, Value>::new();

        if let Some(input_json) = req
            .payload
            .input_json
            .as_ref()
            .and_then(|value| value.as_object())
        {
            for (k, v) in input_json.iter() {
                if k == "messages" || k == "tool_specs" || k == "text" {
                    continue;
                }
                body.insert(k.clone(), v.clone());
            }
        }

        if let Some(options) = req.payload.options.as_ref().and_then(|value| value.as_object()) {
            for (k, v) in options.iter() {
                body.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }

        let (input_key, has_input) = match method {
            ai_methods::IMAGE_UPSCALE | ai_methods::IMAGE_BG_REMOVE => {
                let exists = body.contains_key("image_url");
                ("image_url", exists)
            }
            ai_methods::VIDEO_UPSCALE => {
                let exists = body.contains_key("video_url");
                ("video_url", exists)
            }
            other => {
                return Err(ProviderError::fatal(format!(
                    "fal provider does not support method '{}'",
                    other
                )))
            }
        };

        if !has_input {
            let resource = req.payload.resources.first().ok_or_else(|| {
                ProviderError::fatal(format!(
                    "fal {} requires an input resource (resources[0] or payload.input_json.{}).",
                    method, input_key
                ))
            })?;
            let url = resource_to_url(resource).ok_or_else(|| {
                ProviderError::fatal(
                    "fal provider only accepts resources of kind 'url' or 'base64'".to_string(),
                )
            })?;
            body.insert(input_key.to_string(), Value::String(url));
        }

        Ok(body)
    }

    async fn run_method(
        &self,
        ctx: &crate::aicc::InvokeCtx,
        method: &str,
        provider_model: &str,
        req: &AiMethodRequest,
    ) -> Result<ProviderStartResult, ProviderError> {
        if self.api_token.trim().is_empty() {
            return Err(ProviderError::fatal(
                "fal provider api_token is empty".to_string(),
            ));
        }

        let body = Self::build_request_body(method, req)?;
        let url = format!(
            "{}/{}",
            self.base_url,
            provider_model.trim_start_matches('/')
        );

        let body_log = Value::Object(body.clone()).to_string();
        info!(
            "aicc.fal.request provider_instance_name={} method={} model={} url={} trace_id={:?} body={}",
            self.instance.provider_instance_name,
            method,
            provider_model,
            url,
            ctx.trace_id,
            body_log
        );

        let (status, response_body, latency_ms) = self.post_json(ctx, url.as_str(), &body).await?;
        if !status.is_success() {
            let message = response_body
                .pointer("/error/message")
                .or_else(|| response_body.get("detail"))
                .and_then(|value| value.as_str())
                .map(|value| value.to_string())
                .unwrap_or_else(|| response_body.to_string());
            warn!(
                "aicc.fal.error provider_instance_name={} method={} model={} status={} body={}",
                self.instance.provider_instance_name,
                method,
                provider_model,
                status.as_u16(),
                response_body
            );
            return Err(Self::classify_api_error(
                status,
                format!(
                    "fal api error [{}]: {}",
                    status.as_u16(),
                    truncate_message(message.as_str(), 512)
                ),
            ));
        }

        let artifacts = parse_fal_artifacts(method, &response_body);
        if artifacts.is_empty() {
            return Err(ProviderError::fatal(format!(
                "fal response did not contain any artifacts; method={} body_head={}",
                method,
                truncate_message(response_body.to_string().as_str(), 320)
            )));
        }

        let cost_amount = match method {
            ai_methods::IMAGE_UPSCALE => 0.05,
            ai_methods::IMAGE_BG_REMOVE => 0.01,
            ai_methods::VIDEO_UPSCALE => 0.50,
            _ => 0.0,
        };
        let cost = if cost_amount > 0.0 {
            Some(AiCost {
                amount: cost_amount,
                currency: "USD".to_string(),
            })
        } else {
            None
        };

        let mut extra = Map::new();
        extra.insert("provider".to_string(), Value::String("fal".to_string()));
        extra.insert("method".to_string(), Value::String(method.to_string()));
        extra.insert(
            "model".to_string(),
            Value::String(provider_model.to_string()),
        );
        extra.insert("latency_ms".to_string(), Value::from(latency_ms));
        extra.insert(
            "provider_io".to_string(),
            json!({
                "input": Value::Object(body),
                "output": response_body.clone(),
            }),
        );

        let summary = AiResponseSummary {
            text: None,
            tool_calls: vec![],
            artifacts,
            usage: None,
            cost,
            finish_reason: Some("stop".to_string()),
            provider_task_ref: response_body
                .get("request_id")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string()),
            extra: Some(Value::Object(extra)),
        };
        Ok(ProviderStartResult::Immediate(summary))
    }
}

#[async_trait]
impl Provider for FalProvider {
    fn inventory(&self) -> ProviderInventory {
        self.inventory.clone()
    }

    fn legacy_instance(&self) -> Option<&ProviderInstance> {
        Some(&self.instance)
    }

    fn estimate_cost(&self, input: &CostEstimateInput) -> CostEstimateOutput {
        let (cost, latency) = match input.api_type {
            ApiType::ImageUpscale => (0.05, 8_000),
            ApiType::ImageBgRemove => (0.01, 4_000),
            ApiType::VideoUpscale => (0.50, 120_000),
            _ => (1.0, 5_000),
        };
        CostEstimateOutput {
            estimated_cost_usd: cost,
            pricing_mode: PricingMode::PerToken,
            quota_state: QuotaState::Normal,
            confidence: 0.5,
            estimated_latency_ms: Some(latency),
        }
    }

    async fn start(
        &self,
        ctx: crate::aicc::InvokeCtx,
        provider_model: String,
        req: ResolvedRequest,
        _sink: Arc<dyn TaskEventSink>,
    ) -> std::result::Result<ProviderStartResult, ProviderError> {
        match req.method.as_str() {
            ai_methods::IMAGE_UPSCALE
            | ai_methods::IMAGE_BG_REMOVE
            | ai_methods::VIDEO_UPSCALE => {
                self.run_method(
                    &ctx,
                    req.method.as_str(),
                    provider_model.as_str(),
                    &req.request,
                )
                .await
            }
            method => Err(ProviderError::fatal(format!(
                "fal provider does not support method '{}'",
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

fn build_model_metadata(
    provider_instance_name: &str,
    provider_type: crate::model_types::ProviderType,
    provider_model_id: &str,
    api_type: ApiType,
    logical_mounts: Vec<String>,
    estimated_cost_usd: Option<f64>,
    estimated_latency_ms: Option<u64>,
) -> ModelMetadata {
    ModelMetadata {
        provider_model_id: provider_model_id.to_string(),
        exact_model: exact_model_name(provider_model_id, provider_instance_name),
        parameter_scale: None,
        api_types: vec![api_type],
        logical_mounts,
        capabilities: ModelCapabilities {
            streaming: false,
            tool_call: false,
            json_schema: false,
            vision: false,
            max_context_tokens: None,
        },
        attributes: ModelAttributes {
            provider_type: provider_type.clone(),
            local: false,
            privacy: PrivacyClass::Cloud,
            quality_score: Some(0.75),
            latency_class: LatencyClass::Normal,
            cost_class: CostClass::Medium,
        },
        pricing: ModelPricing {
            estimated_cost_usd,
            ..Default::default()
        },
        health: ModelHealth {
            status: HealthStatus::Available,
            p95_latency_ms: estimated_latency_ms,
            quota_state: QuotaState::Normal,
            ..Default::default()
        },
    }
}

fn logical_mounts_for_api(api_type: ApiType, provider_model_id: &str) -> Vec<String> {
    let base = match api_type {
        ApiType::ImageUpscale => "image.upscale",
        ApiType::ImageBgRemove => "image.bg_remove",
        ApiType::VideoUpscale => "video.upscale",
        _ => "unknown",
    };
    let mut mounts = vec![
        base.to_string(),
        format!("{}.{}", base, FAL_PROVIDER_DRIVER),
    ];
    let normalized = provider_model_id
        .trim()
        .trim_start_matches('/')
        .replace('/', ".")
        .to_ascii_lowercase();
    if !normalized.is_empty() {
        let mount = format!("{}.{}", base, normalized);
        if !mounts.iter().any(|item| item == &mount) {
            mounts.push(mount);
        }
    }
    mounts
}

fn resource_to_url(resource: &ResourceRef) -> Option<String> {
    match resource {
        ResourceRef::Url { url, .. } => Some(url.clone()),
        ResourceRef::Base64 { mime, data_base64 } => {
            Some(format!("data:{};base64,{}", mime, data_base64))
        }
        ResourceRef::NamedObject { .. } => None,
    }
}

fn parse_fal_artifacts(method: &str, body: &Value) -> Vec<AiArtifact> {
    let mut artifacts = Vec::<AiArtifact>::new();

    let collect_object = |obj: &Map<String, Value>, name: &str, default_mime: &str| -> Option<AiArtifact> {
        let url = obj.get("url").and_then(|value| value.as_str())?;
        let mime = obj
            .get("content_type")
            .and_then(|value| value.as_str())
            .unwrap_or(default_mime)
            .to_string();
        let mut metadata = Map::new();
        for (k, v) in obj.iter() {
            if k == "url" {
                continue;
            }
            metadata.insert(k.clone(), v.clone());
        }
        let metadata_value = if metadata.is_empty() {
            None
        } else {
            Some(Value::Object(metadata))
        };
        Some(AiArtifact {
            name: name.to_string(),
            resource: ResourceRef::Url {
                url: url.to_string(),
                mime_hint: Some(mime.clone()),
            },
            mime: Some(mime),
            metadata: metadata_value,
        })
    };

    let default_mime = match method {
        ai_methods::VIDEO_UPSCALE => "video/mp4",
        _ => "image/png",
    };

    if let Some(images) = body.get("images").and_then(|value| value.as_array()) {
        for (idx, item) in images.iter().enumerate() {
            if let Some(obj) = item.as_object() {
                if let Some(artifact) =
                    collect_object(obj, format!("image-{}", idx).as_str(), "image/png")
                {
                    artifacts.push(artifact);
                }
            }
        }
    }

    for (key, name) in &[("image", "image"), ("video", "video"), ("output", "output")] {
        if let Some(obj) = body.get(*key).and_then(|value| value.as_object()) {
            if let Some(artifact) = collect_object(obj, name, default_mime) {
                artifacts.push(artifact);
            }
        }
    }

    artifacts
}

fn truncate_message(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        value.to_string()
    } else {
        let mut head: String = value.chars().take(limit).collect();
        head.push_str("...");
        head
    }
}

#[derive(Debug, Deserialize, Default)]
struct FalSettings {
    #[serde(default = "default_fal_enabled")]
    enabled: bool,
    #[serde(default, alias = "api_key", alias = "apiKey")]
    api_token: String,
    #[serde(default)]
    instances: Vec<SettingsFalInstanceConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct SettingsFalInstanceConfig {
    #[serde(default = "default_instance_id")]
    provider_instance_name: String,
    #[serde(default = "default_provider_type")]
    provider_type: String,
    #[serde(default = "default_base_url")]
    base_url: String,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
    #[serde(default)]
    image_upscale_models: Vec<String>,
    #[serde(default)]
    image_bg_remove_models: Vec<String>,
    #[serde(default)]
    video_upscale_models: Vec<String>,
}

fn default_fal_enabled() -> bool {
    false
}

fn default_instance_id() -> String {
    "fal-default".to_string()
}

fn default_provider_type() -> String {
    "cloud_api".to_string()
}

fn default_base_url() -> String {
    DEFAULT_FAL_BASE_URL.to_string()
}

fn default_timeout_ms() -> u64 {
    DEFAULT_FAL_TIMEOUT_MS
}

fn normalize_model_list(models: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::<String>::new();
    let mut normalized = Vec::new();
    for model in models.into_iter() {
        let value = model.trim().trim_start_matches('/').to_string();
        if value.is_empty() {
            continue;
        }
        let key = value.to_ascii_lowercase();
        if seen.insert(key) {
            normalized.push(value);
        }
    }
    normalized
}

fn parse_fal_settings(settings: &Value) -> Result<Option<FalSettings>> {
    let Some(raw) = settings.get(FAL_PROVIDER_SETTINGS_KEY) else {
        return Ok(None);
    };
    if raw.is_null() {
        return Ok(None);
    }
    let parsed = serde_json::from_value::<FalSettings>(raw.clone())
        .map_err(|err| anyhow!("failed to parse settings.fal: {}", err))?;
    if !parsed.enabled {
        return Ok(None);
    }
    Ok(Some(parsed))
}

fn build_fal_instances(settings: &FalSettings) -> Result<Vec<FalInstanceConfig>> {
    let raw_instances = if settings.instances.is_empty() {
        vec![SettingsFalInstanceConfig {
            provider_instance_name: default_instance_id(),
            provider_type: default_provider_type(),
            base_url: default_base_url(),
            timeout_ms: default_timeout_ms(),
            image_upscale_models: vec![],
            image_bg_remove_models: vec![],
            video_upscale_models: vec![],
        }]
    } else {
        settings.instances.clone()
    };

    let mut instances = Vec::new();
    for raw in raw_instances.into_iter() {
        let mut image_upscale_models = normalize_model_list(raw.image_upscale_models);
        if image_upscale_models.is_empty() {
            image_upscale_models = vec![DEFAULT_IMAGE_UPSCALE_MODEL.to_string()];
        }
        let mut image_bg_remove_models = normalize_model_list(raw.image_bg_remove_models);
        if image_bg_remove_models.is_empty() {
            image_bg_remove_models = vec![DEFAULT_IMAGE_BG_REMOVE_MODEL.to_string()];
        }
        let mut video_upscale_models = normalize_model_list(raw.video_upscale_models);
        if video_upscale_models.is_empty() {
            video_upscale_models = vec![DEFAULT_VIDEO_UPSCALE_MODEL.to_string()];
        }

        instances.push(FalInstanceConfig {
            provider_instance_name: raw.provider_instance_name,
            provider_type: raw.provider_type,
            base_url: raw.base_url,
            timeout_ms: raw.timeout_ms,
            image_upscale_models,
            image_bg_remove_models,
            video_upscale_models,
        });
    }
    Ok(instances)
}

pub fn register_fal_providers(center: &AIComputeCenter, settings: &Value) -> Result<usize> {
    let Some(fal_settings) = parse_fal_settings(settings)? else {
        info!("aicc fal provider is disabled (settings.fal missing or disabled)");
        return Ok(0);
    };
    if fal_settings.api_token.trim().is_empty() {
        return Err(anyhow!(
            "settings.fal.api_token (or api_key) is required when fal provider is enabled"
        ));
    }

    let instances = build_fal_instances(&fal_settings)?;
    info!("aicc fal registering instances={}", instances.len());

    let mut prepared = Vec::<(FalInstanceConfig, Arc<dyn Provider>)>::new();
    for config in instances.iter() {
        let provider = FalProvider::new(config.clone(), fal_settings.api_token.clone())?;
        prepared.push((config.clone(), Arc::new(provider)));
    }

    for (config, provider) in prepared.into_iter() {
        let inventory = center.registry().add_provider(provider);
        info!(
            "registered fal provider base_url={} models={}",
            config.base_url,
            inventory.models.len()
        );
        center
            .model_registry()
            .write()
            .map_err(|_| anyhow!("model registry lock poisoned"))?
            .apply_inventory(inventory)
            .map_err(|err| anyhow!("failed to apply fal inventory: {}", err))?;
    }

    Ok(instances.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn build_instances_uses_default_models_when_unspecified() {
        let settings = FalSettings {
            enabled: true,
            api_token: "token".to_string(),
            instances: vec![],
        };
        let instances = build_fal_instances(&settings).expect("instances");
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].image_upscale_models[0], DEFAULT_IMAGE_UPSCALE_MODEL);
        assert_eq!(
            instances[0].image_bg_remove_models[0],
            DEFAULT_IMAGE_BG_REMOVE_MODEL
        );
        assert_eq!(
            instances[0].video_upscale_models[0],
            DEFAULT_VIDEO_UPSCALE_MODEL
        );
    }

    #[test]
    fn parse_settings_disabled_returns_none() {
        let settings = json!({ "fal": { "enabled": false, "api_token": "x" } });
        assert!(parse_fal_settings(&settings).expect("ok").is_none());
    }

    #[test]
    fn register_with_only_api_token_uses_all_defaults() {
        let center = AIComputeCenter::default();
        let settings = json!({
            "fal": {
                "enabled": true,
                "api_token": "test-token"
            }
        });
        let count = register_fal_providers(&center, &settings).expect("register");
        assert_eq!(count, 1);

        let registry = center.model_registry().read().expect("model registry lock");
        let upscale_items = registry.default_items_for_path("image.upscale");
        assert!(upscale_items
            .values()
            .any(|item| item.target == format!("{}@fal-default", DEFAULT_IMAGE_UPSCALE_MODEL)));
        let bg_items = registry.default_items_for_path("image.bg_remove");
        assert!(bg_items
            .values()
            .any(|item| item.target == format!("{}@fal-default", DEFAULT_IMAGE_BG_REMOVE_MODEL)));
        let video_items = registry.default_items_for_path("video.upscale");
        assert!(video_items
            .values()
            .any(|item| item.target == format!("{}@fal-default", DEFAULT_VIDEO_UPSCALE_MODEL)));
    }

    #[test]
    fn register_with_empty_instance_object_uses_defaults() {
        let center = AIComputeCenter::default();
        let settings = json!({
            "fal": {
                "enabled": true,
                "api_token": "test-token",
                "instances": [{}]
            }
        });
        let count = register_fal_providers(&center, &settings).expect("register");
        assert_eq!(count, 1);

        let registry = center.model_registry().read().expect("model registry lock");
        let upscale_items = registry.default_items_for_path("image.upscale");
        assert!(upscale_items
            .values()
            .any(|item| item.target == format!("{}@fal-default", DEFAULT_IMAGE_UPSCALE_MODEL)));
    }

    #[test]
    fn register_fal_providers_registers_inventory() {
        let center = AIComputeCenter::default();
        let settings = json!({
            "fal": {
                "enabled": true,
                "api_token": "test-token",
                "instances": [{
                    "provider_instance_name": "fal-main",
                    "provider_type": "cloud_api",
                    "base_url": "https://fal.run",
                    "image_upscale_models": ["fal-ai/clarity-upscaler"],
                    "image_bg_remove_models": ["fal-ai/imageutils/rembg"],
                    "video_upscale_models": ["fal-ai/topaz/upscale/video"]
                }]
            }
        });
        let count = register_fal_providers(&center, &settings).expect("register");
        assert_eq!(count, 1);

        let registry = center.model_registry().read().expect("model registry lock");
        let upscale_items = registry.default_items_for_path("image.upscale");
        assert!(upscale_items
            .values()
            .any(|item| item.target == "fal-ai/clarity-upscaler@fal-main"));
        let bg_items = registry.default_items_for_path("image.bg_remove");
        assert!(bg_items
            .values()
            .any(|item| item.target == "fal-ai/imageutils/rembg@fal-main"));
        let video_items = registry.default_items_for_path("video.upscale");
        assert!(video_items
            .values()
            .any(|item| item.target == "fal-ai/topaz/upscale/video@fal-main"));
    }

    #[test]
    fn build_request_body_inserts_image_url_from_resources() {
        let mut req = AiMethodRequest::new(
            Capability::Image,
            buckyos_api::ModelSpec::new("image.upscale".to_string(), None),
            Default::default(),
            Default::default(),
            None,
        );
        req.payload.resources = vec![ResourceRef::Url {
            url: "https://example.com/cat.png".to_string(),
            mime_hint: Some("image/png".to_string()),
        }];
        let body = FalProvider::build_request_body(ai_methods::IMAGE_UPSCALE, &req).expect("body");
        assert_eq!(
            body.get("image_url").and_then(|v| v.as_str()),
            Some("https://example.com/cat.png")
        );
    }

    #[test]
    fn parse_artifacts_extracts_image_object() {
        let body = json!({
            "image": { "url": "https://fal.example/out.png", "content_type": "image/png", "width": 2048 }
        });
        let artifacts = parse_fal_artifacts(ai_methods::IMAGE_UPSCALE, &body);
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].mime.as_deref(), Some("image/png"));
        match &artifacts[0].resource {
            ResourceRef::Url { url, .. } => assert_eq!(url, "https://fal.example/out.png"),
            _ => panic!("unexpected resource kind"),
        }
    }

    #[test]
    fn parse_artifacts_extracts_video_object() {
        let body = json!({
            "video": { "url": "https://fal.example/out.mp4", "content_type": "video/mp4" }
        });
        let artifacts = parse_fal_artifacts(ai_methods::VIDEO_UPSCALE, &body);
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].mime.as_deref(), Some("video/mp4"));
    }
}
