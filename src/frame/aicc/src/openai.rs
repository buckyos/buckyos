use crate::aicc::{
    emit_background_provider_result, provider_type_from_settings, redacted_json_log,
    AIComputeCenter, Provider, ProviderError, ProviderInstance, ProviderRefreshTask,
    ProviderStartResult, ResolvedRequest, TaskEventSink,
};
use crate::metadata_resolver::{resolve_driver_inventory, DriverModelResolveRequest};
#[cfg(test)]
use crate::model_types::ProviderType;
use crate::model_types::{
    ApiType, CostEstimateInput, CostEstimateOutput, ModelMetadata, PricingMode, ProviderInventory,
    ProviderOrigin, ProviderTypeTrustedSource, QuotaState,
};
use crate::openai_protocol::{
    apply_provider_model_defaults, merge_options, merge_requirements_response_format,
    merge_tool_calls, strip_incompatible_sampling_options,
};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use base64::engine::general_purpose;
use base64::Engine as _;
#[cfg(test)]
use buckyos_api::Capability;
use buckyos_api::{
    ai_methods, features, value_to_object_map, AiArtifact, AiContent, AiCost, AiMessage,
    AiMethodRequest, AiResponse, AiRole, AiToolCall, AiToolResultContent, AiUsage, ResourceRef,
};
use buckyos_kit::buckyos_get_unix_timestamp;
use image::imageops::FilterType;
use image::ImageFormat;
use log::{error, info, warn};
use reqwest::header::{CONTENT_ENCODING, CONTENT_TYPE};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::error::Error as _;
use std::hash::{Hash, Hasher};
use std::io::Cursor;
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::Duration;
use tokio::sync::watch;
use tokio::time;

const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_OPENAI_TIMEOUT_MS: u64 = 300_000;
const DEFAULT_OPENAI_MODELS: &str = "gpt-5.6-sol,gpt-5.6-terra,gpt-5.6-luna";
const DEFAULT_OPENAI_IMAGE_MODELS: &str = "gpt-image-2";
const DEFAULT_OPENAI_EMBEDDING_MODELS: &str =
    "text-embedding-3-large,text-embedding-3-small,text-embedding-ada-002";
const DEFAULT_OPENAI_ASR_MODELS: &str =
    "gpt-transcribe,gpt-4o-transcribe,gpt-4o-mini-transcribe,whisper-1";
const DEFAULT_OPENAI_TTS_MODELS: &str = "gpt-4o-mini-tts,tts-1,tts-1-hd";
const DEFAULT_OPENAI_VIDEO_MODELS: &str = "";
const DEFAULT_OPENAI_PROVIDER_DRIVER: &str = "openai";
const DEFAULT_INVENTORY_REFRESH_INTERVAL: Duration = Duration::from_secs(300);
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
const OPENAI_IMAGE_EDIT_OPTION_ALLOWLIST: &[&str] = &[
    "background",
    "input_fidelity",
    "n",
    "output_compression",
    "output_format",
    "quality",
    "size",
    "user",
];
const OPENAI_RESPONSES_IMAGE_TOOL_OPTION_ALLOWLIST: &[&str] = &[
    "background",
    "input_fidelity",
    "moderation",
    "output_compression",
    "output_format",
    "partial_images",
    "quality",
    "size",
];
const OPENAI_VIDEO_POLL_INTERVAL: Duration = Duration::from_secs(2);
const OPENAI_VIDEO_MAX_WAIT: Duration = Duration::from_secs(600);
const RESOURCE_FETCH_ATTEMPTS: usize = 3;

#[derive(Debug, Clone)]
pub struct OpenAIInstanceConfig {
    pub provider_instance_name: String,
    pub provider_type: String,
    pub provider_driver: String,
    pub api_token: String,
    pub base_url: String,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone)]
pub struct OpenAIProvider {
    instance: ProviderInstance,
    inventory: Arc<RwLock<ProviderInventory>>,
    client: Client,
    api_token: String,
    base_url: String,
    provider_type: crate::model_types::ProviderType,
    provider_driver: String,
    refresh_task: Arc<Mutex<Option<Arc<ProviderRefreshTask>>>>,
}

#[derive(Debug, Deserialize)]
struct OpenAIModelsResponse {
    #[serde(default)]
    data: Vec<OpenAIModelEntry>,
}

#[derive(Debug, Deserialize)]
struct OpenAIModelEntry {
    id: String,
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

    pub fn new(cfg: OpenAIInstanceConfig, openai_api_token: &str) -> Result<Self> {
        let timeout_ms = if cfg.timeout_ms == 0 {
            DEFAULT_OPENAI_TIMEOUT_MS
        } else {
            cfg.timeout_ms
        };

        let client = Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .context("failed to build reqwest client for openai provider")?;

        let provider_type = provider_type_from_settings(cfg.provider_type.as_str());
        let provider_instance_name = cfg.provider_instance_name.clone();
        let provider_driver = if cfg.provider_driver.trim().is_empty() {
            default_provider_driver_for_instance(
                cfg.provider_instance_name.as_str(),
                cfg.base_url.as_str(),
            )
        } else {
            cfg.provider_driver.trim().to_string()
        };
        let instance = ProviderInstance {
            provider_instance_name: provider_instance_name.clone(),
            provider_type: provider_type.clone(),
            provider_driver: provider_driver.clone(),
            provider_origin: ProviderOrigin::SystemConfig,
            provider_type_trusted_source: ProviderTypeTrustedSource::SystemConfig,
            provider_type_revision: None,
            endpoint: Some(cfg.base_url.clone()),
            plugin_key: None,
        };
        let inventory = Self::default_inventory(
            provider_instance_name.as_str(),
            provider_type.clone(),
            provider_driver.as_str(),
        );

        let api_token = openai_api_token.trim().to_string();
        if api_token.is_empty() {
            return Err(anyhow!("openai requires non-empty api_token"));
        }

        Ok(Self {
            instance,
            inventory: Arc::new(RwLock::new(inventory)),
            client,
            api_token,
            base_url: cfg.base_url.trim_end_matches('/').to_string(),
            provider_type,
            provider_driver,
            refresh_task: Arc::new(Mutex::new(None)),
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
                    "aicc.openai.inventory.refresh_task_lock_poisoned provider_instance_name={}",
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
            info!(
                "aicc.openai.inventory.refresh_stopped provider_instance_name={}",
                provider_instance_name
            );
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
                "aicc.openai.inventory.initial_refresh_failed provider_instance_name={} err={}",
                provider_instance_name, err
            );
        }
        if refresh_task.is_stopped() {
            info!(
                "aicc.openai.inventory.refresh_stopped provider_instance_name={}",
                provider_instance_name
            );
            return;
        }

        let mut interval = time::interval(DEFAULT_INVENTORY_REFRESH_INTERVAL);
        interval.tick().await;
        loop {
            tokio::select! {
                biased;
                changed = shutdown_rx.changed() => {
                    if changed.is_err() || *shutdown_rx.borrow() {
                        info!(
                            "aicc.openai.inventory.refresh_stopped provider_instance_name={}",
                            provider_instance_name
                        );
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
                            "aicc.openai.inventory.refresh_failed provider_instance_name={} err={}",
                            provider_instance_name, err
                        );
                    }
                    if refresh_task.is_stopped() {
                        info!(
                            "aicc.openai.inventory.refresh_stopped provider_instance_name={}",
                            provider_instance_name
                        );
                        return;
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
                    info!(
                        "aicc.openai.inventory.refresh_stop_requested provider_instance_name={}",
                        self.instance.provider_instance_name
                    );
                }
            }
            Err(_) => warn!(
                "aicc.openai.inventory.refresh_task_lock_poisoned provider_instance_name={}",
                self.instance.provider_instance_name
            ),
        }
    }

    async fn refresh_inventory_once(&self) -> Result<ProviderInventory> {
        let endpoint = self.models_endpoint();
        let token = self.build_inventory_auth_token().await?;
        let response = self
            .client
            .get(endpoint.as_str())
            .bearer_auth(token)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "openai inventory refresh failed status={} body={}",
                status,
                body
            ));
        }

        let body = response
            .json::<Value>()
            .await
            .context("failed to parse openai inventory response")?;
        let inventory = self.build_inventory_from_remote_value(body)?;
        {
            let mut current = self
                .inventory
                .write()
                .map_err(|_| anyhow!("openai inventory lock poisoned"))?;
            *current = inventory.clone();
        }
        info!(
            "aicc.openai.inventory.refreshed provider_instance_name={} models={}",
            self.instance.provider_instance_name,
            inventory.models.len()
        );
        Ok(inventory)
    }

    fn models_endpoint(&self) -> String {
        let lower = self.base_url.to_ascii_lowercase();
        if lower.ends_with("/chat/completions") {
            let prefix = &self.base_url[..self.base_url.len() - "/chat/completions".len()];
            return format!("{}/models", prefix.trim_end_matches('/'));
        }
        if lower.ends_with("/responses") || lower.ends_with("/images/generations") {
            if let Some((prefix, _)) = self.base_url.rsplit_once('/') {
                return format!("{}/models", prefix.trim_end_matches('/'));
            }
        }
        format!("{}/models", self.base_url.trim_end_matches('/'))
    }

    fn default_inventory(
        provider_instance_name: &str,
        provider_type: crate::model_types::ProviderType,
        provider_driver: &str,
    ) -> ProviderInventory {
        let (mut models, image_models, embedding_models, asr_models, tts_models) =
            if provider_driver == "openrouter" {
                (vec![], vec![], vec![], vec![], vec![])
            } else {
                (
                    normalize_model_list(parse_csv_list(DEFAULT_OPENAI_MODELS)),
                    normalize_model_list(parse_csv_list(DEFAULT_OPENAI_IMAGE_MODELS)),
                    normalize_model_list(parse_csv_list(DEFAULT_OPENAI_EMBEDDING_MODELS)),
                    normalize_model_list(parse_csv_list(DEFAULT_OPENAI_ASR_MODELS)),
                    normalize_model_list(parse_csv_list(DEFAULT_OPENAI_TTS_MODELS)),
                )
            };
        if provider_driver == DEFAULT_OPENAI_PROVIDER_DRIVER {
            models.extend(parse_csv_list(DEFAULT_OPENAI_VIDEO_MODELS));
            models = normalize_model_list(models);
        }

        Self::build_inventory(
            provider_instance_name,
            provider_type,
            provider_driver,
            models.as_slice(),
            image_models.as_slice(),
            embedding_models.as_slice(),
            asr_models.as_slice(),
            tts_models.as_slice(),
            Some("default-v1".to_string()),
        )
    }

    fn build_inventory_from_models(
        &self,
        models: &[String],
        image_models: &[String],
        embedding_models: &[String],
        asr_models: &[String],
        tts_models: &[String],
        revision: Option<String>,
    ) -> ProviderInventory {
        Self::build_inventory(
            self.instance.provider_instance_name.as_str(),
            self.provider_type.clone(),
            self.provider_driver.as_str(),
            models,
            image_models,
            embedding_models,
            asr_models,
            tts_models,
            revision,
        )
    }

    fn build_inventory_from_remote_value(&self, body: Value) -> Result<ProviderInventory> {
        if body
            .get("models")
            .and_then(|value| value.as_array())
            .is_some()
        {
            let inventory = serde_json::from_value::<ProviderInventory>(body)
                .context("failed to parse provider inventory response")?;
            let inventory = self.normalize_remote_provider_inventory(inventory);
            if inventory.models.is_empty() {
                return Err(anyhow!(
                    "openai provider inventory returned no supported models"
                ));
            }
            return Ok(inventory);
        }

        let response = serde_json::from_value::<OpenAIModelsResponse>(body)
            .context("failed to parse openai models response")?;
        let (llm_models, image_models, embedding_models, asr_models, tts_models) =
            normalize_remote_model_ids(response.data, self.provider_driver.as_str());
        if llm_models.is_empty()
            && image_models.is_empty()
            && embedding_models.is_empty()
            && asr_models.is_empty()
            && tts_models.is_empty()
        {
            return Err(anyhow!(
                "openai inventory refresh returned no supported models"
            ));
        }

        Ok(self.build_inventory_from_models(
            llm_models.as_slice(),
            image_models.as_slice(),
            embedding_models.as_slice(),
            asr_models.as_slice(),
            tts_models.as_slice(),
            Some(inventory_revision(
                llm_models.as_slice(),
                image_models.as_slice(),
                embedding_models.as_slice(),
                asr_models.as_slice(),
                tts_models.as_slice(),
            )),
        ))
    }

    fn normalize_remote_provider_inventory(
        &self,
        inventory: ProviderInventory,
    ) -> ProviderInventory {
        let version = inventory.version.clone();
        let inventory_revision = inventory.inventory_revision.clone();
        let remote_models = inventory.models;
        let requests = remote_models
            .iter()
            .filter_map(remote_model_resolve_request)
            .collect::<Vec<_>>();
        let remote_by_id = remote_models
            .iter()
            .filter_map(|model| {
                remote_provider_model_id(model)
                    .filter(|id| !id.is_empty())
                    .map(|id| (id, model))
            })
            .collect::<HashMap<_, _>>();
        let mut normalized = resolve_driver_inventory(
            self.instance.provider_instance_name.as_str(),
            self.provider_type.clone(),
            self.provider_driver.as_str(),
            requests.as_slice(),
            inventory_revision,
        );
        for model in normalized.models.iter_mut() {
            let base_model_id = model
                .provider_actual_model_id
                .as_deref()
                .unwrap_or(model.provider_model_id.as_str());
            if let Some(remote_model) = remote_by_id.get(base_model_id) {
                merge_remote_dynamic_metadata(model, remote_model);
            }
        }
        normalized.version = version.clone();
        if normalized.inventory_revision.is_none() {
            normalized.inventory_revision = Some(inventory_revision_from_metadata(
                normalized.models.as_slice(),
                version.as_deref(),
            ));
        }
        normalized
    }

    fn build_inventory(
        provider_instance_name: &str,
        provider_type: crate::model_types::ProviderType,
        provider_driver: &str,
        models: &[String],
        image_models: &[String],
        embedding_models: &[String],
        asr_models: &[String],
        tts_models: &[String],
        revision: Option<String>,
    ) -> ProviderInventory {
        let mut requests = Vec::<DriverModelResolveRequest>::new();
        for model in models.iter() {
            requests.push(DriverModelResolveRequest::new(model.clone(), vec![]));
        }
        for model in image_models.iter() {
            requests.push(DriverModelResolveRequest::new(model.clone(), vec![]));
        }
        for model in embedding_models.iter() {
            requests.push(DriverModelResolveRequest::new(model.clone(), vec![]));
        }
        for model in asr_models.iter() {
            requests.push(DriverModelResolveRequest::new(model.clone(), vec![]));
        }
        for model in tts_models.iter() {
            requests.push(DriverModelResolveRequest::new(model.clone(), vec![]));
        }
        resolve_driver_inventory(
            provider_instance_name,
            provider_type,
            provider_driver,
            requests.as_slice(),
            revision,
        )
    }

    async fn build_inventory_auth_token(&self) -> Result<String> {
        Ok(self.api_token.clone())
    }

    async fn build_auth_token(
        &self,
        _ctx: &crate::aicc::InvokeCtx,
    ) -> Result<String, ProviderError> {
        Ok(self.api_token.clone())
    }

    fn price_per_1m_tokens(model: &str) -> (f64, f64) {
        if model.starts_with("gpt-5.6-terra") {
            (2.0, 12.0)
        } else if model.starts_with("gpt-5.6-luna") {
            (0.20, 1.20)
        } else if model.starts_with("gpt-5.6-sol") || model == "gpt-5.6" {
            (4.0, 20.0)
        } else if model.starts_with("gpt-5.4-pro") {
            (30.0, 180.0)
        } else if model.starts_with("gpt-5.4-mini") {
            (0.75, 4.50)
        } else if model.starts_with("gpt-5.4-nano") {
            (0.20, 1.25)
        } else if model.starts_with("gpt-5.4") {
            (2.50, 15.00)
        } else if model.starts_with("gpt-5-pro") {
            (15.00, 120.00)
        } else if model.starts_with("gpt-5-mini") {
            (0.25, 2.00)
        } else if model.starts_with("gpt-5-nano") || model.starts_with("gpt-5-nono") {
            (0.05, 0.40)
        } else if model.starts_with("gpt-5") {
            (1.25, 10.00)
        } else if model.starts_with("gpt-4.1-mini") {
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

    #[cfg(test)]
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
            .input_json
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
            .or_else(|| {
                req.payload.options.as_ref().and_then(|value| {
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
            })
            .unwrap_or(512);

        (input_tokens.max(1), output_tokens.max(1))
    }

    fn estimate_image_count(req: &AiMethodRequest) -> u64 {
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

    fn estimate_text2image_cost(req: &AiMethodRequest, model: &str) -> Option<f64> {
        let per_image = if model.starts_with("dall-e-2") {
            0.02
        } else if model.starts_with("gpt-image-1") {
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
                .unwrap_or("medium");
            let size = req
                .payload
                .options
                .as_ref()
                .and_then(|value| value.get("size"))
                .and_then(|value| value.as_str())
                .or_else(|| {
                    req.payload
                        .input_json
                        .as_ref()
                        .and_then(|value| value.get("size"))
                        .and_then(|value| value.as_str())
                })
                .unwrap_or("1024x1024");
            match (quality, size) {
                ("low", "1024x1536") | ("low", "1536x1024") => 0.016,
                ("medium", "1024x1536") | ("medium", "1536x1024") => 0.063,
                ("high", "1024x1536") | ("high", "1536x1024") => 0.25,
                ("low", _) => 0.011,
                ("high", _) => 0.167,
                (_, "1024x1536") | (_, "1536x1024") => 0.063,
                _ => 0.042,
            }
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

    fn resource_text(resource: &ResourceRef) -> Result<String, ProviderError> {
        match resource {
            ResourceRef::Url { url, .. } => Ok(format!("resource_url: {}", url)),
            ResourceRef::NamedObject { obj_id } => Ok(format!("named_object: {}", obj_id)),
            ResourceRef::Base64 { .. } => Err(ProviderError::fatal(
                "openai provider does not support base64 resources in this version",
            )),
        }
    }

    fn content_value_to_text(value: &Value) -> Result<Option<String>, ProviderError> {
        if let Some(text) = value.as_str() {
            let text = text.trim();
            return Ok((!text.is_empty()).then(|| text.to_string()));
        }

        let Some(parts) = value.as_array() else {
            return Ok(None);
        };

        let mut lines = Vec::new();
        for part in parts {
            let Some(part_obj) = part.as_object() else {
                continue;
            };
            match part_obj.get("type").and_then(|value| value.as_str()) {
                Some("text") | Some("input_text") => {
                    if let Some(text) = part_obj
                        .get("text")
                        .and_then(|value| value.as_str())
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    {
                        lines.push(text.to_string());
                    }
                }
                Some("resource") => {
                    if let Some(resource_value) = part_obj.get("resource") {
                        let resource: ResourceRef = serde_json::from_value(resource_value.clone())
                            .map_err(|err| {
                                ProviderError::fatal(format!(
                                    "invalid content resource part: {}",
                                    err
                                ))
                            })?;
                        lines.push(Self::resource_text(&resource)?);
                    }
                }
                _ => {}
            }
        }

        if lines.is_empty() {
            Ok(None)
        } else {
            Ok(Some(lines.join("\n")))
        }
    }

    fn canonical_message_texts(
        req: &AiMethodRequest,
    ) -> Result<Vec<(String, String)>, ProviderError> {
        if let Some(messages) = req
            .payload
            .input_json
            .as_ref()
            .and_then(|value| value.get("messages"))
            .and_then(|value| value.as_array())
        {
            let mut result = Vec::new();
            for msg in messages {
                let Some(msg_obj) = msg.as_object() else {
                    continue;
                };
                let role = msg_obj
                    .get("role")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("user")
                    .to_string();
                if let Some(text) = msg_obj
                    .get("content")
                    .map(Self::content_value_to_text)
                    .transpose()?
                    .flatten()
                {
                    result.push((role, text));
                }
            }
            if !result.is_empty() {
                return Ok(result);
            }
        }

        Ok(req
            .payload
            .messages
            .iter()
            .filter_map(|msg| {
                let role = msg.role.as_str();
                let content = msg.text_content();
                let content_trimmed = content.trim();
                (!content_trimmed.is_empty())
                    .then(|| (role.to_string(), content_trimmed.to_string()))
            })
            .collect())
    }

    fn estimate_cost_for_usage(&self, model: &str, usage: &AiUsage) -> Option<AiCost> {
        let input_tokens = usage.input_tokens? as f64;
        let output_tokens = usage.output_tokens? as f64;
        let pricing = self.inventory.read().ok().and_then(|inventory| {
            inventory.models.iter().find_map(|metadata| {
                (metadata.provider_model_id == model).then(|| {
                    (
                        metadata
                            .origin_model_id
                            .clone()
                            .unwrap_or_else(|| model.to_string()),
                        metadata.pricing.currency.clone(),
                        metadata.pricing.input_token,
                        metadata.pricing.output_token,
                    )
                })
            })
        });
        let (origin_model_id, currency, input_per_token, output_per_token) =
            pricing.unwrap_or_else(|| (model.to_string(), "USD".to_string(), None, None));
        let (amount, currency) = if let (Some(input_per_token), Some(output_per_token)) =
            (input_per_token, output_per_token)
        {
            (
                (input_tokens * input_per_token) + (output_tokens * output_per_token),
                currency,
            )
        } else {
            let (input_per_m, output_per_m) = Self::price_per_1m_tokens(origin_model_id.as_str());
            (
                ((input_tokens / 1_000_000.0) * input_per_m)
                    + ((output_tokens / 1_000_000.0) * output_per_m),
                "USD".to_string(),
            )
        };

        Some(AiCost { amount, currency })
    }

    fn model_supports_responses_image_generation(&self, provider_model: &str) -> bool {
        self.inventory.read().ok().is_some_and(|inventory| {
            inventory.models.iter().any(|metadata| {
                (metadata.provider_model_id == provider_model
                    || metadata.provider_actual_model_id.as_deref() == Some(provider_model))
                    && metadata.capabilities.image_generation
            })
        })
    }

    /// 把 `AiRole` 映射成 OpenAI Responses API 接受的 role 字符串。
    /// `Tool` 不会出现在 message-role 上 —— ToolResult 走的是 `function_call_output`
    /// 顶层 item,不再当作 role 消息;但兜底仍降级到 `user` 防止野生数据。
    fn ai_role_to_openai(role: &AiRole) -> &'static str {
        match role {
            AiRole::System => "system",
            AiRole::Developer => "developer",
            AiRole::User => "user",
            AiRole::Assistant => "assistant",
            _ => "user",
        }
    }

    /// 把 waist 喂来的 `Vec<AiMessage>`(每条带 `Vec<AiContent>` blocks)
    /// 拆成 OpenAI Responses API 的 input array items 列表。
    ///
    /// Responses API 的 input array 接受混合 item:
    /// - `{role, content: [{type, text}]}` —— 普通消息
    /// - `{type: "function_call", call_id, name, arguments}` —— 顶层 tool 调用
    /// - `{type: "function_call_output", call_id, output}` —— 顶层 tool 结果
    ///
    /// 一条 AiMessage 内若混合 Text + ToolUse 等 block,按顺序拆成多个 items:
    /// 先把累积的文本 flush 成一个 message item,再 emit function_call,
    /// 然后继续累积下一段 text。保持原始 block 顺序。
    fn build_messages(&self, req: &AiMethodRequest) -> Result<Vec<Value>, ProviderError> {
        let mut items: Vec<Value> = vec![];

        // 主路径:消费 typed `Vec<AiContent>`。
        //
        // 注意:`AiPayload::Deserialize` 在 wire 反序列化时把 `input_json.messages`
        // **同时**填进了 `payload.messages` (typed) 和保留在 `input_json` 副本里。
        // 因此当 caller (waist) 走 AiMessage 路径时,这两个字段都非空。
        // 必须**优先用 typed**,因为它带 `ToolUse` / `ToolResult` 结构;
        // 先走 input_json 兼容路径会被 `content_value_to_text` 抽干净
        // (只留 Text block),tool 信息全丢。
        for msg in &req.payload.messages {
            let role_str = Self::ai_role_to_openai(&msg.role);
            if role_str == "assistant" {
                let provider_items = msg
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        AiContent::ProviderState { provider, value }
                            if provider.eq_ignore_ascii_case(&self.provider_driver) =>
                        {
                            Some(value.clone())
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                if !provider_items.is_empty() {
                    items.extend(provider_items);
                    continue;
                }
            }
            let content_type = if role_str == "assistant" {
                "output_text"
            } else {
                "input_text"
            };
            let mut pending_text_parts: Vec<Value> = Vec::new();

            for block in &msg.content {
                match block {
                    AiContent::Text { text } => {
                        if text.is_empty() {
                            continue;
                        }
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
                        // 先把累积的 text 落成 message item,保持顺序。
                        if !pending_text_parts.is_empty() {
                            items.push(json!({
                                "role": role_str,
                                "content": std::mem::take(&mut pending_text_parts),
                            }));
                        }
                        let arguments_str =
                            serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string());
                        items.push(json!({
                            "type": "function_call",
                            "call_id": call_id,
                            "name": name,
                            "arguments": arguments_str,
                        }));
                    }
                    AiContent::ToolResult {
                        call_id,
                        content,
                        is_error: _,
                    } => {
                        // 同上,先 flush 累积的 text。
                        if !pending_text_parts.is_empty() {
                            items.push(json!({
                                "role": role_str,
                                "content": std::mem::take(&mut pending_text_parts),
                            }));
                        }
                        // Responses API `function_call_output.output` 是单 string。
                        // 把 ToolResultContent 里所有 Text block 串起来;非 text
                        // 暂时忽略(image/document 等需要单独走 input_image)。
                        let output_text = content
                            .iter()
                            .filter_map(|c| match c {
                                AiToolResultContent::Text { text } => Some(text.as_str()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        items.push(json!({
                            "type": "function_call_output",
                            "call_id": call_id,
                            "output": output_text,
                        }));
                    }
                    AiContent::Image { source } if role_str == "user" => {
                        pending_text_parts.push(Self::responses_image_part(source)?);
                    }
                    AiContent::Document { source, title } if role_str == "user" => {
                        let mut line = Self::chat_resource_text(source);
                        if let Some(title) = title.as_deref().filter(|s| !s.is_empty()) {
                            line = format!("{} ({})", line, title);
                        }
                        pending_text_parts.push(json!({
                            "type": content_type,
                            "text": line,
                        }));
                    }
                    // Thinking 等暂不处理 —— 现阶段 llm_explore /
                    // run_local_llm 都不产生这些 block,出现的话留给后续阶段。
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

        // 兜底:caller 只给了 payload.text + resources(没塞 messages)。
        if items.is_empty() {
            let mut content = String::new();
            if let Some(text) = req.payload.text.as_ref() {
                content.push_str(text);
            }

            if !content.trim().is_empty() {
                items.push(json!({
                    "role": "user",
                    "content": [
                        { "type": "input_text", "text": content }
                    ],
                }));
            }
        }

        if !req.payload.resources.is_empty() {
            let content = req
                .payload
                .resources
                .iter()
                .map(Self::responses_resource_part)
                .collect::<Result<Vec<_>, _>>()?;
            items.push(json!({
                "role": "user",
                "content": content,
            }));
        }

        if items.is_empty() {
            return Err(ProviderError::fatal(
                "request payload has no usable text/messages for llm",
            ));
        }

        Ok(items)
    }

    fn responses_image_part(source: &ResourceRef) -> Result<Value, ProviderError> {
        match source {
            ResourceRef::Url { url, .. } => Ok(json!({
                "type": "input_image",
                "image_url": url,
            })),
            ResourceRef::Base64 { mime, data_base64 } => Ok(json!({
                "type": "input_image",
                "image_url": format!("data:{};base64,{}", mime, data_base64),
            })),
            ResourceRef::NamedObject { obj_id } => Ok(json!({
                "type": "input_text",
                "text": format!("named_object: {}", obj_id),
            })),
        }
    }

    fn responses_resource_part(source: &ResourceRef) -> Result<Value, ProviderError> {
        let mime = match source {
            ResourceRef::Url { mime_hint, .. } => mime_hint.as_deref(),
            ResourceRef::Base64 { mime, .. } => Some(mime.as_str()),
            ResourceRef::NamedObject { .. } => None,
        };
        if mime.is_some_and(|value| value.starts_with("image/")) {
            return Self::responses_image_part(source);
        }
        match source {
            ResourceRef::Url { url, .. } => Ok(json!({
                "type": "input_file",
                "file_url": url,
            })),
            ResourceRef::Base64 { mime, data_base64 } => Ok(json!({
                "type": "input_file",
                "filename": format!("input.{}", Self::document_extension(mime)),
                "file_data": format!("data:{};base64,{}", mime, data_base64),
            })),
            ResourceRef::NamedObject { obj_id } => Ok(json!({
                "type": "input_text",
                "text": format!("named_object: {}", obj_id),
            })),
        }
    }

    fn document_extension(mime: &str) -> &'static str {
        match mime.split(';').next().unwrap_or(mime).trim() {
            "text/plain" => "txt",
            "text/markdown" => "md",
            "application/pdf" => "pdf",
            "application/msword" => "doc",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => "docx",
            "application/vnd.ms-excel" => "xls",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "xlsx",
            "text/csv" => "csv",
            "text/tab-separated-values" => "tsv",
            "application/vnd.ms-powerpoint" => "ppt",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation" => "pptx",
            "text/html" => "html",
            "application/xml" | "text/xml" => "xml",
            "application/json" => "json",
            "application/yaml" | "text/yaml" => "yaml",
            "application/rtf" | "text/rtf" => "rtf",
            "text/x-python" => "py",
            _ => "bin",
        }
    }

    /// Build OpenAI Chat Completions message array. Unlike the Responses
    /// path, tool calls and tool results live as sibling messages
    /// (`role:"assistant"+tool_calls`, `role:"tool"+tool_call_id`) rather
    /// than top-level `function_call` items.
    fn build_chat_messages(req: &AiMethodRequest) -> Result<Vec<Value>, ProviderError> {
        let mut messages: Vec<Value> = vec![];

        // 主路径:消费 typed `Vec<AiMessage>`,保留 ToolUse / ToolResult /
        // Image / Document 等 block。
        if !req.payload.messages.is_empty() {
            for msg in req.payload.messages.iter() {
                Self::lower_message_to_chat(msg, &mut messages)?;
            }
        }

        // 兼容路径:caller 通过 input_json.messages 喂裸 JSON,仅做文本降级。
        if messages.is_empty() {
            for (role, content) in Self::canonical_message_texts(req)? {
                messages.push(json!({
                    "role": role,
                    "content": content,
                }));
            }
        }

        if messages.is_empty() {
            let mut content = String::new();
            if let Some(text) = req.payload.text.as_ref() {
                content.push_str(text);
            }

            let mut resource_lines = vec![];
            for resource in req.payload.resources.iter() {
                resource_lines.push(Self::resource_text(resource)?);
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

    fn lower_message_to_chat(
        msg: &AiMessage,
        messages: &mut Vec<Value>,
    ) -> Result<(), ProviderError> {
        match msg.role {
            AiRole::System | AiRole::Developer => {
                let role = if matches!(msg.role, AiRole::Developer) {
                    "developer"
                } else {
                    "system"
                };
                let mut text = String::new();
                for block in msg.content.iter() {
                    if let AiContent::Text { text: chunk } = block {
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str(chunk.as_str());
                    }
                }
                if !text.is_empty() {
                    messages.push(json!({
                        "role": role,
                        "content": text,
                    }));
                }
            }
            AiRole::User => {
                let parts = Self::chat_user_content_parts(&msg.content)?;
                if parts.is_empty() {
                    return Ok(());
                }
                // If only plain text, emit `content: <string>` for legacy
                // proxies; else use the parts array.
                let only_text_simple = parts
                    .iter()
                    .all(|p| p.get("type").and_then(|v| v.as_str()) == Some("text"));
                if only_text_simple {
                    let joined = parts
                        .iter()
                        .filter_map(|p| p.get("text").and_then(|v| v.as_str()))
                        .collect::<Vec<_>>()
                        .join("\n");
                    messages.push(json!({
                        "role": "user",
                        "content": joined,
                    }));
                } else {
                    messages.push(json!({
                        "role": "user",
                        "content": parts,
                    }));
                }
            }
            AiRole::Assistant => {
                let mut text_chunks: Vec<String> = Vec::new();
                let mut tool_calls: Vec<Value> = Vec::new();
                for block in &msg.content {
                    match block {
                        AiContent::Text { text } => {
                            if !text.is_empty() {
                                text_chunks.push(text.clone());
                            }
                        }
                        AiContent::ToolUse {
                            call_id,
                            name,
                            args,
                        } => {
                            let arguments_str =
                                serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string());
                            tool_calls.push(json!({
                                "id": call_id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": arguments_str,
                                }
                            }));
                        }
                        AiContent::Thinking { text, summary, .. } => {
                            // Chat Completions has no canonical thinking
                            // surface; keep the textual trace so it survives
                            // a round-trip.
                            let candidate = text
                                .as_deref()
                                .filter(|s| !s.is_empty())
                                .or_else(|| summary.as_deref().filter(|s| !s.is_empty()));
                            if let Some(value) = candidate {
                                text_chunks.push(value.to_string());
                            }
                        }
                        AiContent::ProviderState { provider, value: _ } => {
                            // No legacy chat-completions surface accepts
                            // foreign provider state; drop unless target.
                            let _ = provider;
                        }
                        _ => {
                            // ToolResult / Image / Document not valid on
                            // assistant turn in Chat Completions.
                        }
                    }
                }
                let mut body = Map::new();
                body.insert("role".to_string(), Value::String("assistant".to_string()));
                if text_chunks.is_empty() {
                    body.insert("content".to_string(), Value::Null);
                } else {
                    body.insert("content".to_string(), Value::String(text_chunks.join("\n")));
                }
                if !tool_calls.is_empty() {
                    body.insert("tool_calls".to_string(), Value::Array(tool_calls));
                }
                messages.push(Value::Object(body));
            }
            AiRole::Tool => {
                let Some(AiContent::ToolResult {
                    call_id,
                    content,
                    is_error: _,
                }) = msg.content.first()
                else {
                    return Ok(());
                };
                let text = Self::tool_result_text_for_chat(content);
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": text,
                }));
            }
        }
        Ok(())
    }

    fn chat_user_content_parts(content: &[AiContent]) -> Result<Vec<Value>, ProviderError> {
        let mut parts = Vec::with_capacity(content.len());
        for block in content {
            match block {
                AiContent::Text { text } => {
                    if !text.is_empty() {
                        parts.push(json!({ "type": "text", "text": text }));
                    }
                }
                AiContent::Image { source } => {
                    parts.push(Self::chat_image_part(source)?);
                }
                AiContent::Document { source, title } => {
                    // Chat Completions has no dedicated `document` part —
                    // emit a text reference so the document URL/id is at
                    // least surfaced to the model.
                    let mut line = Self::chat_resource_text(source);
                    if let Some(title) = title.as_deref().filter(|s| !s.is_empty()) {
                        line = format!("{} ({})", line, title);
                    }
                    parts.push(json!({ "type": "text", "text": line }));
                }
                _ => {}
            }
        }
        Ok(parts)
    }

    fn chat_image_part(source: &ResourceRef) -> Result<Value, ProviderError> {
        match source {
            ResourceRef::Url { url, .. } => Ok(json!({
                "type": "image_url",
                "image_url": { "url": url },
            })),
            ResourceRef::Base64 { mime, data_base64 } => Ok(json!({
                "type": "image_url",
                "image_url": { "url": format!("data:{};base64,{}", mime, data_base64) },
            })),
            ResourceRef::NamedObject { obj_id } => Ok(json!({
                "type": "text",
                "text": format!("named_object: {}", obj_id),
            })),
        }
    }

    fn chat_resource_text(source: &ResourceRef) -> String {
        match source {
            ResourceRef::Url { url, .. } => format!("resource_url: {}", url),
            ResourceRef::NamedObject { obj_id } => format!("named_object: {}", obj_id),
            ResourceRef::Base64 { mime, .. } => format!("inline_{}", mime),
        }
    }

    fn tool_result_text_for_chat(content: &[AiToolResultContent]) -> String {
        let mut parts = Vec::new();
        for item in content {
            match item {
                AiToolResultContent::Text { text } => parts.push(text.clone()),
                AiToolResultContent::Image { source } => {
                    parts.push(Self::chat_resource_text(source));
                }
                AiToolResultContent::Document { source, title } => {
                    let mut line = Self::chat_resource_text(source);
                    if let Some(title) = title {
                        line.push_str(" (");
                        line.push_str(title);
                        line.push(')');
                    }
                    parts.push(line);
                }
            }
        }
        parts.join("\n")
    }

    fn use_chat_completions_endpoint(&self) -> bool {
        self.base_url
            .to_ascii_lowercase()
            .contains("/chat/completions")
    }

    fn convert_text_format_to_chat_response_format(format: Value) -> Value {
        let Some(format_obj) = format.as_object() else {
            return format;
        };
        let Some(format_type) = format_obj.get("type").and_then(|value| value.as_str()) else {
            return Value::Object(format_obj.clone());
        };

        if format_type != "json_schema" {
            return Value::Object(format_obj.clone());
        }

        let schema = format_obj
            .get("schema")
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        let name = format_obj
            .get("name")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("aicc_response");
        let strict = format_obj
            .get("strict")
            .and_then(|value| value.as_bool())
            .unwrap_or(true);

        json!({
            "type": "json_schema",
            "json_schema": {
                "name": name,
                "schema": schema,
                "strict": strict
            }
        })
    }

    fn normalize_chat_completions_request(request_obj: &mut Map<String, Value>) {
        if let Some(text_format) = request_obj
            .get("text")
            .and_then(|value| value.as_object())
            .and_then(|text_obj| text_obj.get("format"))
            .cloned()
        {
            if !request_obj.contains_key("response_format") {
                request_obj.insert(
                    "response_format".to_string(),
                    Self::convert_text_format_to_chat_response_format(text_format),
                );
            }
            request_obj.remove("text");
        }

        if let Some(max_output_tokens) = request_obj.remove("max_output_tokens") {
            if !request_obj.contains_key("max_tokens")
                && !request_obj.contains_key("max_completion_tokens")
            {
                request_obj.insert("max_tokens".to_string(), max_output_tokens);
            }
        }
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

    fn message_from_response_body(
        &self,
        body: &Value,
        text: Option<String>,
        tool_calls: Vec<AiToolCall>,
        artifacts: Vec<AiArtifact>,
    ) -> AiMessage {
        let mut message = AiResponse::message_from_parts(text, tool_calls, artifacts);
        if let Some(output_items) = body.get("output").and_then(Value::as_array) {
            message
                .content
                .extend(
                    output_items
                        .iter()
                        .cloned()
                        .map(|value| AiContent::ProviderState {
                            provider: self.provider_driver.clone(),
                            value,
                        }),
                );
        }
        message
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

        if let Some(choices) = event.get("choices").and_then(|value| value.as_array()) {
            for choice in choices {
                if let Some(text) = choice
                    .get("delta")
                    .and_then(|value| value.get("content"))
                    .and_then(|value| value.as_str())
                {
                    accumulated_text.push_str(text);
                }
            }
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

    fn extract_text2image_prompt(req: &AiMethodRequest) -> Option<String> {
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
            .map(|msg| msg.text_content().trim().to_string())
            .filter(|msg| !msg.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        if !message_prompt.is_empty() {
            return Some(message_prompt);
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

    fn responses_image_input_part(source: &ResourceRef) -> Result<Value, ProviderError> {
        match source {
            ResourceRef::Url { url, .. } => Ok(json!({
                "type": "input_image",
                "image_url": url,
            })),
            ResourceRef::Base64 { mime, data_base64 } => {
                if !mime.starts_with("image/") {
                    return Err(ProviderError::fatal(format!(
                        "OpenAI Responses image input requires image MIME type, got '{}'",
                        mime
                    )));
                }
                general_purpose::STANDARD
                    .decode(data_base64)
                    .map_err(|err| {
                        ProviderError::fatal(format!(
                            "OpenAI Responses image input contains invalid base64: {}",
                            err
                        ))
                    })?;
                Ok(json!({
                    "type": "input_image",
                    "image_url": format!("data:{};base64,{}", mime, data_base64),
                }))
            }
            ResourceRef::NamedObject { obj_id } => Err(ProviderError::fatal(format!(
                "named image object '{}' must be materialized before OpenAI dispatch",
                obj_id
            ))),
        }
    }

    fn image_resources_from_request(req: &AiMethodRequest) -> Vec<ResourceRef> {
        if !req.payload.resources.is_empty() {
            return req.payload.resources.clone();
        }
        let Some(input) = req.payload.input_json.as_ref() else {
            return vec![];
        };
        for key in ["images", "image"] {
            let Some(value) = input.get(key) else {
                continue;
            };
            let values = value
                .as_array()
                .cloned()
                .unwrap_or_else(|| vec![value.clone()]);
            let resources = values
                .into_iter()
                .filter_map(|value| serde_json::from_value::<ResourceRef>(value).ok())
                .collect::<Vec<_>>();
            if !resources.is_empty() {
                return resources;
            }
        }
        vec![]
    }

    fn build_responses_image_request(
        provider_model: &str,
        req: &AiMethodRequest,
        action: &str,
    ) -> Result<(Map<String, Value>, Vec<String>), ProviderError> {
        let prompt = Self::extract_text2image_prompt(req).ok_or_else(|| {
            ProviderError::fatal(
                "Responses image generation requires prompt in payload.text/messages/input_json/options",
            )
        })?;
        let mut tool = Map::new();
        tool.insert(
            "type".to_string(),
            Value::String("image_generation".to_string()),
        );
        tool.insert("action".to_string(), Value::String(action.to_string()));
        let mut ignored = vec![];
        for source in [
            req.payload.input_json.as_ref(),
            req.payload.options.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            let Some(options) = source.as_object() else {
                continue;
            };
            for (key, value) in options {
                if key == "model"
                    || key == "messages"
                    || key == "prompt"
                    || key == "image"
                    || key == "images"
                {
                    continue;
                }
                if OPENAI_RESPONSES_IMAGE_TOOL_OPTION_ALLOWLIST.contains(&key.as_str()) {
                    tool.insert(key.clone(), value.clone());
                } else if !ignored.contains(key) {
                    ignored.push(key.clone());
                }
            }
        }

        let input = if action == "edit" {
            let resources = Self::image_resources_from_request(req);
            if resources.is_empty() {
                return Err(ProviderError::fatal(
                    "Responses image edit requires at least one canonical image input",
                ));
            }
            let mut content = vec![json!({ "type": "input_text", "text": prompt })];
            for resource in &resources {
                content.push(Self::responses_image_input_part(resource)?);
            }
            json!([{ "role": "user", "content": content }])
        } else {
            Value::String(prompt)
        };

        let stream =
            req.requirements.requires_feature("streaming") || tool.contains_key("partial_images");
        let mut request_obj = Map::new();
        request_obj.insert(
            "model".to_string(),
            Value::String(provider_model.to_string()),
        );
        request_obj.insert("input".to_string(), input);
        request_obj.insert("tools".to_string(), Value::Array(vec![Value::Object(tool)]));
        if stream {
            request_obj.insert("stream".to_string(), Value::Bool(true));
        }
        Ok((request_obj, ignored))
    }

    fn image_mime_from_bytes(bytes: &[u8]) -> Result<&'static str, ProviderError> {
        match image::guess_format(bytes) {
            Ok(ImageFormat::Png) => Ok("image/png"),
            Ok(ImageFormat::Jpeg) => Ok("image/jpeg"),
            Ok(ImageFormat::WebP) => Ok("image/webp"),
            Ok(format) => Err(ProviderError::fatal(format!(
                "OpenAI Responses returned unsupported image format {:?}",
                format
            ))),
            Err(err) => Err(ProviderError::fatal(format!(
                "OpenAI Responses returned invalid image bytes: {}",
                err
            ))),
        }
    }

    fn parse_responses_image_artifacts(body: &Value) -> Result<Vec<AiArtifact>, ProviderError> {
        let output = body
            .get("output")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                ProviderError::fatal("OpenAI Responses image result is missing output")
            })?;
        let mut artifacts = vec![];
        for item in output {
            if item.get("type").and_then(Value::as_str) != Some("image_generation_call") {
                continue;
            }
            let status = item
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if status != "completed" {
                return Err(ProviderError::fatal(format!(
                    "OpenAI Responses image generation call did not complete (status='{}')",
                    status
                )));
            }
            let encoded = item
                .get("result")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    ProviderError::fatal(
                        "OpenAI Responses image generation call is missing base64 result",
                    )
                })?;
            let bytes = general_purpose::STANDARD.decode(encoded).map_err(|err| {
                ProviderError::fatal(format!(
                    "OpenAI Responses image generation returned invalid base64: {}",
                    err
                ))
            })?;
            let mime = Self::image_mime_from_bytes(bytes.as_slice())?.to_string();
            let index = artifacts.len() + 1;
            artifacts.push(AiArtifact {
                name: format!("image_{}", index),
                resource: ResourceRef::Base64 {
                    mime: mime.clone(),
                    data_base64: encoded.to_string(),
                },
                mime: Some(mime),
                metadata: Some(json!({
                    "provider_call_id": item.get("id").cloned(),
                    "action": item.get("action").cloned(),
                })),
            });
        }
        if artifacts.is_empty() {
            return Err(ProviderError::fatal(
                "OpenAI Responses result has no image_generation_call outputs",
            ));
        }
        Ok(artifacts)
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
        req: &AiMethodRequest,
    ) -> Result<(), ProviderError> {
        let web_search_required = req.requirements.requires_feature(features::WEB_SEARCH);
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

    fn normalize_web_search_reasoning(request_obj: &mut Map<String, Value>) -> bool {
        let has_web_search = request_obj
            .get("tools")
            .and_then(|value| value.as_array())
            .map(|tools| {
                tools.iter().any(|tool| {
                    tool.get("type")
                        .and_then(|value| value.as_str())
                        .map(|tool_type| {
                            tool_type == OPENAI_TOOL_TYPE_WEB_SEARCH || tool_type == "web_search"
                        })
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false);
        if !has_web_search {
            return false;
        }

        let Some(reasoning) = request_obj
            .get_mut("reasoning")
            .and_then(|value| value.as_object_mut())
        else {
            return false;
        };
        let Some(effort) = reasoning.get_mut("effort") else {
            return false;
        };
        if effort.as_str() != Some("minimal") {
            return false;
        }

        *effort = Value::String("low".to_string());
        true
    }

    async fn post_json(
        &self,
        ctx: &crate::aicc::InvokeCtx,
        url: &str,
        request_obj: &Map<String, Value>,
    ) -> Result<(StatusCode, Value, u64), ProviderError> {
        let auth_token = self.build_auth_token(ctx).await?;
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
                    "aicc.openai.http_send_failed provider_instance_name={} provider_type={} url={} retryable={} timeout={} connect={} status={:?} err_chain={}",
                    self.instance.provider_instance_name,
                    self.instance.provider_type,
                    url,
                    retryable,
                    err.is_timeout(),
                    err.is_connect(),
                    err.status(),
                    Self::format_error_chain(&err)
                );
                eprintln!(
                    "aicc.openai.http_send_failed provider_instance_name={} provider_type={} url={} retryable={} timeout={} connect={} status={:?} err_chain={}",
                    self.instance.provider_instance_name,
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
                "aicc.openai.response_decode_failed provider_instance_name={} provider_type={} url={} status={} content_type={} content_encoding={} err={}",
                self.instance.provider_instance_name,
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
                "aicc.openai.response_parse_failed provider_instance_name={} provider_type={} url={} status={} err={}",
                self.instance.provider_instance_name,
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

    async fn post_binary_json(
        &self,
        ctx: &crate::aicc::InvokeCtx,
        url: &str,
        request_obj: &Map<String, Value>,
    ) -> Result<(StatusCode, Vec<u8>, String, u64), ProviderError> {
        let auth_token = self.build_auth_token(ctx).await?;
        let started_at = std::time::Instant::now();
        let response = self
            .client
            .post(url)
            .bearer_auth(auth_token.as_str())
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
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        let bytes = response.bytes().await.map_err(|err| {
            Self::classify_api_error(status, format!("failed to decode openai response: {}", err))
        })?;
        Ok((status, bytes.to_vec(), content_type, latency_ms))
    }

    async fn resource_to_file_bytes(
        &self,
        resource: &ResourceRef,
        fallback_name: &str,
    ) -> Result<(String, String, Vec<u8>), ProviderError> {
        match resource {
            ResourceRef::Base64 { mime, data_base64 } => {
                let bytes = general_purpose::STANDARD
                    .decode(data_base64)
                    .map_err(|err| {
                        ProviderError::fatal(format!("invalid base64 resource: {}", err))
                    })?;
                Ok((fallback_name.to_string(), mime.clone(), bytes))
            }
            ResourceRef::Url { url, mime_hint } => {
                for attempt in 1..=RESOURCE_FETCH_ATTEMPTS {
                    let response = match self.client.get(url).send().await {
                        Ok(response) => response,
                        Err(_) if attempt < RESOURCE_FETCH_ATTEMPTS => {
                            time::sleep(Duration::from_millis(200 * attempt as u64)).await;
                            continue;
                        }
                        Err(err) => {
                            return Err(if err.is_timeout() || err.is_connect() {
                                ProviderError::retryable(format!(
                                    "failed to fetch resource url after {} attempts: {}",
                                    attempt, err
                                ))
                            } else {
                                ProviderError::fatal(format!(
                                    "failed to fetch resource url after {} attempts: {}",
                                    attempt, err
                                ))
                            });
                        }
                    };
                    let status = response.status();
                    if !status.is_success() {
                        let error = Self::classify_api_error(
                            status,
                            format!("resource url returned status {}", status.as_u16()),
                        );
                        if error.is_retryable() && attempt < RESOURCE_FETCH_ATTEMPTS {
                            time::sleep(Duration::from_millis(200 * attempt as u64)).await;
                            continue;
                        }
                        return Err(error);
                    }
                    let content_type = mime_hint
                        .clone()
                        .or_else(|| {
                            response
                                .headers()
                                .get(CONTENT_TYPE)
                                .and_then(|value| value.to_str().ok())
                                .map(|value| value.to_string())
                        })
                        .unwrap_or_else(|| "application/octet-stream".to_string());
                    match response.bytes().await {
                        Ok(bytes) => {
                            return Ok((fallback_name.to_string(), content_type, bytes.to_vec()));
                        }
                        Err(_) if attempt < RESOURCE_FETCH_ATTEMPTS => {
                            time::sleep(Duration::from_millis(200 * attempt as u64)).await;
                        }
                        Err(err) => {
                            return Err(ProviderError::retryable(format!(
                                "failed to read resource bytes after {} attempts: {}",
                                attempt, err
                            )));
                        }
                    }
                }
                unreachable!()
            }
            ResourceRef::NamedObject { obj_id } => Err(ProviderError::fatal(format!(
                "openai provider cannot resolve named object resource {} without resolver bytes",
                obj_id
            ))),
        }
    }

    async fn post_multipart(
        &self,
        ctx: &crate::aicc::InvokeCtx,
        url: &str,
        fields: Vec<(String, String)>,
        files: Vec<(String, String, String, Vec<u8>)>,
    ) -> Result<(StatusCode, Value, u64), ProviderError> {
        let boundary = format!("aicc-openai-{}", buckyos_get_unix_timestamp());
        let mut body = Vec::<u8>::new();
        for (name, value) in fields {
            body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
            body.extend_from_slice(
                format!("Content-Disposition: form-data; name=\"{}\"\r\n\r\n", name).as_bytes(),
            );
            body.extend_from_slice(value.as_bytes());
            body.extend_from_slice(b"\r\n");
        }
        for (field, filename, mime, data) in files {
            body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
            body.extend_from_slice(
                format!(
                    "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\n",
                    field, filename
                )
                .as_bytes(),
            );
            body.extend_from_slice(format!("Content-Type: {}\r\n\r\n", mime).as_bytes());
            body.extend_from_slice(data.as_slice());
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{}--\r\n", boundary).as_bytes());

        let auth_token = self.build_auth_token(ctx).await?;
        let started_at = std::time::Instant::now();
        let response = self
            .client
            .post(url)
            .bearer_auth(auth_token.as_str())
            .header(
                CONTENT_TYPE,
                format!("multipart/form-data; boundary={}", boundary),
            )
            .body(body)
            .send()
            .await
            .map_err(|err| {
                if err.is_timeout() || err.is_connect() {
                    ProviderError::retryable(format!("openai multipart request failed: {}", err))
                } else {
                    ProviderError::fatal(format!("openai multipart request failed: {}", err))
                }
            })?;
        let latency_ms = started_at.elapsed().as_millis() as u64;
        let status = response.status();
        let body = response.json::<Value>().await.map_err(|err| {
            Self::classify_api_error(
                status,
                format!("failed to parse openai multipart response: {}", err),
            )
        })?;
        Ok((status, body, latency_ms))
    }

    async fn get_json(
        &self,
        ctx: &crate::aicc::InvokeCtx,
        url: &str,
    ) -> Result<(StatusCode, Value), ProviderError> {
        let auth_token = self.build_auth_token(ctx).await?;
        let response = self
            .client
            .get(url)
            .bearer_auth(auth_token.as_str())
            .send()
            .await
            .map_err(|err| {
                ProviderError::fatal(format!("openai video status request failed: {}", err))
            })?;
        let status = response.status();
        let body = response.json::<Value>().await.map_err(|err| {
            ProviderError::fatal(format!(
                "failed to parse openai video status response: {}",
                err
            ))
        })?;
        Ok((status, body))
    }

    async fn get_binary(
        &self,
        ctx: &crate::aicc::InvokeCtx,
        url: &str,
    ) -> Result<(StatusCode, Vec<u8>, String), ProviderError> {
        let auth_token = self.build_auth_token(ctx).await?;
        let response = self
            .client
            .get(url)
            .bearer_auth(auth_token.as_str())
            .send()
            .await
            .map_err(|err| {
                ProviderError::fatal(format!("openai video download failed: {}", err))
            })?;
        let status = response.status();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("video/mp4")
            .to_string();
        let bytes = response.bytes().await.map_err(|err| {
            ProviderError::fatal(format!("failed to read openai video content: {}", err))
        })?;
        Ok((status, bytes.to_vec(), content_type))
    }

    async fn start_llm(
        &self,
        ctx: &crate::aicc::InvokeCtx,
        provider_model: &str,
        req: &AiMethodRequest,
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
                "aicc.openai omitted incompatible llm options: provider_instance_name={} model={} trace_id={:?} omitted={:?}",
                self.instance.provider_instance_name,
                provider_model,
                ctx.trace_id,
                stripped_options
            );
        }
        merge_requirements_response_format(&mut request_obj, req);
        merge_tool_calls(&mut request_obj, req.payload.tool_specs.as_slice())?;
        Self::merge_requirements_tools(&mut request_obj, req)?;
        if Self::normalize_web_search_reasoning(&mut request_obj) {
            info!(
                "aicc.openai adjusted reasoning.effort for web_search: provider_instance_name={} model={} trace_id={:?} effort=low",
                self.instance.provider_instance_name, provider_model, ctx.trace_id
            );
        }
        if self.use_chat_completions_endpoint() {
            Self::normalize_chat_completions_request(&mut request_obj);
        }
        if !ignored_options.is_empty() {
            warn!(
                "aicc.openai ignored unsupported llm options: provider_instance_name={} model={} trace_id={:?} ignored={:?}",
                self.instance.provider_instance_name, provider_model, ctx.trace_id, ignored_options
            );
        }

        let request_log = redacted_json_log(&Value::Object(request_obj.clone()));
        info!(
            "aicc.openai.llm.input provider_instance_name={} model={} trace_id={:?} request={}",
            self.instance.provider_instance_name, provider_model, ctx.trace_id, request_log
        );

        let url = if self.use_chat_completions_endpoint() {
            self.base_url.clone()
        } else {
            format!("{}/responses", self.base_url)
        };
        let mut retried_without_option = false;
        let (status, body, latency_ms) = loop {
            let (status, body, latency_ms) =
                self.post_json(ctx, url.as_str(), &request_obj).await?;
            if status == StatusCode::BAD_REQUEST && !retried_without_option {
                if let Some(param) = Self::extract_unsupported_request_param(&body) {
                    if Self::remove_retryable_unsupported_option(&mut request_obj, param.as_str()) {
                        warn!(
                            "aicc.openai.llm.retry_without_option provider_instance_name={} model={} trace_id={:?} param={} response={}",
                            self.instance.provider_instance_name,
                            provider_model,
                            ctx.trace_id,
                            param,
                            redacted_json_log(&body)
                        );
                        retried_without_option = true;
                        continue;
                    }
                }
            }
            break (status, body, latency_ms);
        };
        let response_log = redacted_json_log(&body);

        if !status.is_success() {
            warn!(
                "aicc.openai.llm.output provider_instance_name={} model={} trace_id={:?} status={} response={}",
                self.instance.provider_instance_name,
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
            "aicc.openai.llm.output provider_instance_name={} model={} trace_id={:?} status={} response={}",
            self.instance.provider_instance_name,
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
                "aicc.openai.llm.incomplete_output provider_instance_name={} model={} trace_id={:?} err={}",
                self.instance.provider_instance_name, provider_model, ctx.trace_id, err
            );
            return Err(err);
        }
        if !self.use_chat_completions_endpoint()
            && body
                .get("output")
                .and_then(Value::as_array)
                .map_or(true, Vec::is_empty)
        {
            return Err(ProviderError::fatal(
                "OpenAI Responses result is missing output items required for history replay",
            ));
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
            request_units: None,
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

        let summary = AiResponse {
            message: self.message_from_response_body(&body, content, tool_choices, vec![]),
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

    fn resource_from_input_json(req: &AiMethodRequest, keys: &[&str]) -> Option<ResourceRef> {
        let input = req.payload.input_json.as_ref()?;
        for key in keys {
            if let Some(value) = input.get(*key) {
                if let Ok(resource) = serde_json::from_value::<ResourceRef>(value.clone()) {
                    return Some(resource);
                }
                if let Some(resource) = value
                    .as_array()
                    .and_then(|items| items.first())
                    .and_then(|value| serde_json::from_value::<ResourceRef>(value.clone()).ok())
                {
                    return Some(resource);
                }
            }
        }
        None
    }

    fn vision_prompt(method: &str, req: &AiMethodRequest) -> String {
        if let Some(prompt) = req
            .payload
            .input_json
            .as_ref()
            .and_then(|value| value.get("prompt"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return prompt.to_string();
        }

        let mut prompt = if method == ai_methods::VISION_OCR {
            "Extract all readable text from this image. Preserve reading order, line breaks, and layout as closely as possible. Return only the extracted text."
                .to_string()
        } else {
            "Describe this image accurately and concisely.".to_string()
        };
        if let Some(options) = req.payload.input_json.as_ref().and_then(Value::as_object) {
            let mut options = options.clone();
            options.remove("image");
            options.remove("document");
            options.remove("prompt");
            if !options.is_empty() {
                prompt.push_str(" Follow these request options: ");
                prompt.push_str(Value::Object(options).to_string().as_str());
            }
        }
        prompt
    }

    async fn start_vision(
        &self,
        ctx: &crate::aicc::InvokeCtx,
        provider_model: &str,
        method: &str,
        req: &AiMethodRequest,
    ) -> Result<ProviderStartResult, ProviderError> {
        let resource = req
            .payload
            .resources
            .first()
            .cloned()
            .or_else(|| Self::resource_from_input_json(req, &["image", "document"]))
            .ok_or_else(|| ProviderError::fatal("vision request requires an image resource"))?;
        let prompt = Self::vision_prompt(method, req);
        let mut vision_req = req.clone();
        vision_req.payload.text = None;
        vision_req.payload.messages = vec![AiMessage::new(
            AiRole::User,
            vec![AiContent::text(prompt), AiContent::image(resource)],
        )];
        vision_req.payload.tool_specs.clear();
        vision_req.payload.resources.clear();
        vision_req.payload.input_json = None;
        vision_req.payload.options = None;

        match self.start_llm(ctx, provider_model, &vision_req).await? {
            ProviderStartResult::Immediate(mut response) => {
                let text = response.text_content();
                let key = if method == ai_methods::VISION_OCR {
                    "ocr"
                } else {
                    "captions"
                };
                let extra = response.extra.get_or_insert_with(|| json!({}));
                if !extra.is_object() {
                    *extra = json!({});
                }
                if let Some(extra) = extra.as_object_mut() {
                    extra.insert(key.to_string(), json!({ "text": text }));
                }
                Ok(ProviderStartResult::Immediate(response))
            }
            other => Ok(other),
        }
    }

    async fn start_text2image(
        &self,
        ctx: &crate::aicc::InvokeCtx,
        provider_model: &str,
        req: &AiMethodRequest,
    ) -> Result<ProviderStartResult, ProviderError> {
        if self.model_supports_responses_image_generation(provider_model) {
            return self
                .start_responses_image_generation(ctx, provider_model, req, "generate")
                .await;
        }
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
                "aicc.openai ignored unsupported text2image options: provider_instance_name={} model={} trace_id={:?} ignored={:?}",
                self.instance.provider_instance_name, provider_model, ctx.trace_id, ignored_options
            );
        }

        let request_log = redacted_json_log(&Value::Object(request_obj.clone()));
        info!(
            "aicc.openai.text2image.input provider_instance_name={} model={} trace_id={:?} request={}",
            self.instance.provider_instance_name, provider_model, ctx.trace_id, request_log
        );

        let url = format!("{}/images/generations", self.base_url);
        let (status, body, latency_ms) = self.post_json(ctx, url.as_str(), &request_obj).await?;
        let response_log = redacted_json_log(&body);

        if !status.is_success() {
            warn!(
                "aicc.openai.text2image.output provider_instance_name={} model={} trace_id={:?} status={} response={}",
                self.instance.provider_instance_name,
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
            "aicc.openai.text2image.output provider_instance_name={} model={} trace_id={:?} status={} response={}",
            self.instance.provider_instance_name,
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

        let summary = AiResponse {
            message: AiResponse::message_from_parts(revised_prompt, vec![], artifacts),
            usage: Some(AiUsage::request_units(1)),
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

    async fn start_responses_image_generation(
        &self,
        ctx: &crate::aicc::InvokeCtx,
        provider_model: &str,
        req: &AiMethodRequest,
        action: &str,
    ) -> Result<ProviderStartResult, ProviderError> {
        let (request_obj, ignored_options) =
            Self::build_responses_image_request(provider_model, req, action)?;
        if !ignored_options.is_empty() {
            warn!(
                "aicc.openai.responses_image ignored unsupported options: provider_instance_name={} model={} trace_id={:?} ignored={:?}",
                self.instance.provider_instance_name,
                provider_model,
                ctx.trace_id,
                ignored_options
            );
        }
        let request_log = redacted_json_log(&Value::Object(request_obj.clone()));
        info!(
            "aicc.openai.responses_image.input provider_instance_name={} model={} action={} trace_id={:?} request={}",
            self.instance.provider_instance_name,
            provider_model,
            action,
            ctx.trace_id,
            request_log
        );
        let url = format!("{}/responses", self.base_url);
        let (status, body, latency_ms) = self.post_json(ctx, url.as_str(), &request_obj).await?;
        let response_log = redacted_json_log(&body);
        if !status.is_success() {
            warn!(
                "aicc.openai.responses_image.output provider_instance_name={} model={} action={} trace_id={:?} status={} response={}",
                self.instance.provider_instance_name,
                provider_model,
                action,
                ctx.trace_id,
                status.as_u16(),
                response_log
            );
            let message = body
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("OpenAI Responses image generation returned non-success status");
            return Err(Self::classify_api_error(status, message.to_string()));
        }
        info!(
            "aicc.openai.responses_image.output provider_instance_name={} model={} action={} trace_id={:?} status={} response={}",
            self.instance.provider_instance_name,
            provider_model,
            action,
            ctx.trace_id,
            status.as_u16(),
            response_log
        );

        let artifacts = Self::parse_responses_image_artifacts(&body)?;
        let usage = body.get("usage").map(|usage| AiUsage {
            input_tokens: usage.get("input_tokens").and_then(Value::as_u64),
            output_tokens: usage.get("output_tokens").and_then(Value::as_u64),
            total_tokens: usage.get("total_tokens").and_then(Value::as_u64),
            request_units: Some(1),
        });
        let cost = Self::estimate_text2image_cost(req, provider_model).map(|amount| AiCost {
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
                "input": Value::Object(request_obj),
                "output": body.clone()
            }),
        );
        Ok(ProviderStartResult::Immediate(AiResponse {
            message: self.message_from_response_body(&body, None, vec![], artifacts),
            usage,
            cost,
            finish_reason: body
                .get("status")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            provider_task_ref: body
                .get("id")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            extra: Some(Value::Object(extra)),
        }))
    }

    fn embedding_inputs(
        req: &AiMethodRequest,
    ) -> Result<(Value, Vec<Option<String>>), ProviderError> {
        if let Some(items) = req
            .payload
            .input_json
            .as_ref()
            .and_then(|value| value.get("items"))
            .cloned()
        {
            if let Some(array) = items.as_array() {
                let mut texts = Vec::with_capacity(array.len());
                let mut ids = Vec::with_capacity(array.len());
                for (index, item) in array.iter().enumerate() {
                    let text = item
                        .get("text")
                        .and_then(Value::as_str)
                        .or_else(|| item.as_str())
                        .ok_or_else(|| {
                            ProviderError::fatal(format!(
                                "embedding.text item {} must contain text; resource items are unsupported by OpenAI text embeddings",
                                index
                            ))
                        })?;
                    texts.push(Value::String(text.to_string()));
                    ids.push(
                        item.get("id")
                            .and_then(Value::as_str)
                            .map(ToString::to_string),
                    );
                }
                if !texts.is_empty() {
                    return Ok((Value::Array(texts), ids));
                }
            }
        }
        if let Some(text) = req.payload.text.as_ref().map(String::as_str) {
            return Ok((Value::String(text.to_string()), vec![None]));
        }
        let texts = req
            .payload
            .messages
            .iter()
            .map(|msg| msg.text_content().trim().to_string())
            .filter(|value| !value.is_empty())
            .map(Value::String)
            .collect::<Vec<_>>();
        if !texts.is_empty() {
            let ids = vec![None; texts.len()];
            return Ok((Value::Array(texts), ids));
        }
        Err(ProviderError::fatal(
            "embedding.text requires payload.input_json.items or payload.text",
        ))
    }

    async fn start_embedding(
        &self,
        ctx: &crate::aicc::InvokeCtx,
        provider_model: &str,
        req: &AiMethodRequest,
    ) -> Result<ProviderStartResult, ProviderError> {
        let input_json = req.payload.input_json.as_ref();
        if input_json.and_then(|value| value.get("chunking")).is_some() {
            return Err(ProviderError::fatal(
                "OpenAI embedding endpoint does not support canonical chunking",
            ));
        }
        if input_json
            .and_then(|value| value.get("normalize"))
            .and_then(Value::as_bool)
            == Some(false)
        {
            return Err(ProviderError::fatal(
                "OpenAI embeddings are normalized and cannot satisfy normalize=false",
            ));
        }
        let (embedding_input, input_ids) = Self::embedding_inputs(req)?;
        let mut request_obj = Map::new();
        request_obj.insert(
            "model".to_string(),
            Value::String(provider_model.to_string()),
        );
        request_obj.insert("input".to_string(), embedding_input);
        if let Some(dimensions) = req
            .payload
            .input_json
            .as_ref()
            .and_then(|value| value.get("dimensions"))
            .cloned()
        {
            request_obj.insert("dimensions".to_string(), dimensions);
        }
        let url = format!("{}/embeddings", self.base_url);
        let (status, body, latency_ms) = self.post_json(ctx, url.as_str(), &request_obj).await?;
        if !status.is_success() {
            let message = body
                .pointer("/error/message")
                .and_then(|value| value.as_str())
                .unwrap_or("openai embeddings returned non-success status");
            return Err(Self::classify_api_error(status, message.to_string()));
        }
        let dimensions = body
            .pointer("/data/0/embedding")
            .and_then(|value| value.as_array())
            .map(|items| items.len());
        let computed_embedding_space_id = format!(
            "openai:{}:{}",
            provider_model,
            dimensions
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        );
        let embedding_space_id = input_json
            .and_then(|value| value.get("embedding_space_id"))
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .unwrap_or(computed_embedding_space_id);
        let mut data = body
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for (index, item) in data.iter_mut().enumerate() {
            if let (Some(id), Some(object)) = (
                input_ids.get(index).and_then(|value| value.as_ref()),
                item.as_object_mut(),
            ) {
                object.insert("id".to_string(), Value::String(id.clone()));
            }
        }
        let prefer_artifact = input_json
            .and_then(|value| value.get("prefer_artifact"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || data.len() > 100
            || input_json
                .and_then(|value| value.get("output"))
                .and_then(|value| value.get("resource_format"))
                .and_then(Value::as_str)
                .is_some_and(|value| value == "named_object")
            || input_json
                .and_then(|value| value.get("response_format"))
                .and_then(Value::as_str)
                .is_some_and(|value| matches!(value, "object_id" | "named_object"));
        let artifact = if prefer_artifact {
            let artifact_body = json!({
                "embedding_space_id": embedding_space_id.clone(),
                "data": data.clone(),
            });
            let bytes = serde_json::to_vec(&artifact_body).map_err(|error| {
                ProviderError::fatal(format!("serialize embedding artifact failed: {}", error))
            })?;
            Some(AiArtifact {
                name: "embeddings.json".to_string(),
                resource: ResourceRef::Base64 {
                    mime: "application/json".to_string(),
                    data_base64: general_purpose::STANDARD.encode(bytes),
                },
                mime: Some("application/json".to_string()),
                metadata: Some(json!({
                    "rows": data.len(),
                    "dimensions": dimensions,
                    "embedding_space_id": embedding_space_id.clone(),
                })),
            })
        } else {
            None
        };
        let mut extra = Map::new();
        extra.insert(
            "embedding".to_string(),
            json!({
                "data": if prefer_artifact { Value::Array(vec![]) } else { Value::Array(data.clone()) },
                "embedding_space_id": embedding_space_id,
                "artifact": artifact.as_ref().map(|value| json!({
                    "name": value.name.clone(),
                    "mime": value.mime.clone(),
                    "rows": data.len(),
                    "dimensions": dimensions,
                    "embedding_space_id": embedding_space_id.clone(),
                })),
                "provider_io": {
                    "input": Value::Object(request_obj.clone()),
                    "output": body.clone()
                },
                "latency_ms": latency_ms
            }),
        );
        Ok(ProviderStartResult::Immediate(AiResponse {
            message: AiResponse::message_from_parts(None, vec![], artifact.into_iter().collect()),
            finish_reason: Some("stop".to_string()),
            extra: Some(Value::Object(extra)),
            ..Default::default()
        }))
    }

    async fn start_rerank(
        &self,
        ctx: &crate::aicc::InvokeCtx,
        provider_model: &str,
        req: &AiMethodRequest,
    ) -> Result<ProviderStartResult, ProviderError> {
        let input = req.payload.input_json.clone().unwrap_or_else(|| json!({}));
        let prompt = format!(
            "Rerank the documents for the query. Return only JSON with key results, where each result has index, id and score from 0 to 1.\n{}",
            input
        );
        let rerank_req = AiMethodRequest {
            payload: buckyos_api::AiPayload::new(
                Some(prompt),
                vec![],
                vec![],
                vec![],
                Some(json!({
                    "text": {
                        "format": {
                            "type": "json_schema",
                            "name": "rerank_result",
                            "schema": {
                                "type": "object",
                                "properties": {
                                    "results": {
                                        "type": "array",
                                        "items": {
                                            "type": "object",
                                            "properties": {
                                                "index": { "type": "integer" },
                                                "id": { "type": "string" },
                                                "score": { "type": "number" }
                                            },
                                            "required": ["index", "id", "score"],
                                            "additionalProperties": false
                                        }
                                    }
                                },
                                "required": ["results"],
                                "additionalProperties": false
                            }
                        }
                    }
                })),
                req.payload.options.clone(),
            ),
            ..req.clone()
        };
        let mut result = self.start_llm(ctx, provider_model, &rerank_req).await?;
        if let ProviderStartResult::Immediate(summary) = &mut result {
            let summary_text = summary.text_content();
            let rerank_value = serde_json::from_str::<Value>(&summary_text)
                .unwrap_or_else(|_| json!({ "raw_text": summary_text }));
            let mut extra = summary
                .extra
                .take()
                .and_then(|value| value.as_object().cloned())
                .unwrap_or_default();
            extra.insert("rerank".to_string(), rerank_value);
            summary.extra = Some(Value::Object(extra));
        }
        Ok(result)
    }

    async fn start_tts(
        &self,
        ctx: &crate::aicc::InvokeCtx,
        provider_model: &str,
        req: &AiMethodRequest,
    ) -> Result<ProviderStartResult, ProviderError> {
        let input_json = req.payload.input_json.as_ref();
        for (pointer, name) in [
            ("/language", "language"),
            ("/voice/gender", "voice.gender"),
            ("/voice/style", "voice.style"),
            ("/voice/speaker_similarity", "voice.speaker_similarity"),
            ("/output/sample_rate", "output.sample_rate"),
        ] {
            if input_json
                .and_then(|value| value.pointer(pointer))
                .is_some()
            {
                return Err(ProviderError::fatal(format!(
                    "OpenAI TTS does not support canonical hard constraint {}",
                    name
                )));
            }
        }
        let text = req
            .payload
            .input_json
            .as_ref()
            .and_then(|value| value.get("text"))
            .and_then(|value| value.as_str())
            .or(req.payload.text.as_deref())
            .ok_or_else(|| ProviderError::fatal("audio.tts requires text"))?;
        let voice = req
            .payload
            .input_json
            .as_ref()
            .and_then(|value| value.pointer("/voice/voice_id"))
            .and_then(|value| value.as_str())
            .or_else(|| {
                req.payload
                    .input_json
                    .as_ref()
                    .and_then(|value| value.get("voice"))
                    .and_then(|value| value.as_str())
            })
            .unwrap_or("alloy");
        let response_format = req
            .payload
            .input_json
            .as_ref()
            .and_then(|value| value.pointer("/output/media_type"))
            .and_then(|value| value.as_str())
            .map(|mime| match mime {
                "audio/wav" => "wav",
                "audio/opus" => "opus",
                "audio/aac" => "aac",
                "audio/flac" => "flac",
                "audio/L16" | "audio/pcm" => "pcm",
                _ => "mp3",
            })
            .unwrap_or("mp3");
        let mut request_obj = Map::new();
        request_obj.insert(
            "model".to_string(),
            Value::String(provider_model.to_string()),
        );
        request_obj.insert("input".to_string(), Value::String(text.to_string()));
        request_obj.insert("voice".to_string(), Value::String(voice.to_string()));
        request_obj.insert(
            "response_format".to_string(),
            Value::String(response_format.to_string()),
        );
        if let Some(speed) = input_json
            .and_then(|value| value.get("speed"))
            .and_then(Value::as_f64)
        {
            if !(0.25..=4.0).contains(&speed) {
                return Err(ProviderError::fatal(
                    "OpenAI TTS speed must be between 0.25 and 4.0",
                ));
            }
            request_obj.insert("speed".to_string(), json!(speed));
        }
        let url = format!("{}/audio/speech", self.base_url);
        let (status, bytes, content_type, latency_ms) = self
            .post_binary_json(ctx, url.as_str(), &request_obj)
            .await?;
        if !status.is_success() {
            let message = String::from_utf8_lossy(bytes.as_slice()).to_string();
            return Err(Self::classify_api_error(status, message));
        }
        let mime = if content_type.contains("audio") {
            content_type
        } else {
            match response_format {
                "wav" => "audio/wav",
                "opus" => "audio/opus",
                "aac" => "audio/aac",
                "flac" => "audio/flac",
                "pcm" => "audio/pcm",
                _ => "audio/mpeg",
            }
            .to_string()
        };
        let artifact = AiArtifact {
            name: "audio".to_string(),
            resource: ResourceRef::Base64 {
                mime: mime.clone(),
                data_base64: general_purpose::STANDARD.encode(bytes),
            },
            mime: Some(mime),
            metadata: None,
        };
        let mut extra = Map::new();
        extra.insert("provider".to_string(), Value::String("openai".to_string()));
        extra.insert(
            "model".to_string(),
            Value::String(provider_model.to_string()),
        );
        extra.insert("latency_ms".to_string(), Value::from(latency_ms));
        Ok(ProviderStartResult::Immediate(AiResponse {
            message: AiMessage::new(AiRole::Assistant, vec![artifact.into_content()]),
            finish_reason: Some("stop".to_string()),
            extra: Some(Value::Object(extra)),
            ..Default::default()
        }))
    }

    async fn start_asr(
        &self,
        ctx: &crate::aicc::InvokeCtx,
        provider_model: &str,
        req: &AiMethodRequest,
    ) -> Result<ProviderStartResult, ProviderError> {
        let resource = req
            .payload
            .resources
            .first()
            .cloned()
            .or_else(|| Self::resource_from_input_json(req, &["audio"]))
            .ok_or_else(|| ProviderError::fatal("audio.asr requires canonical audio input"))?;
        let (filename, mime, bytes) = self.resource_to_file_bytes(&resource, "audio").await?;
        let mut fields = vec![
            ("model".to_string(), provider_model.to_string()),
            ("response_format".to_string(), "json".to_string()),
        ];
        if let Some(language) = req
            .payload
            .input_json
            .as_ref()
            .and_then(|value| value.get("language"))
            .and_then(|value| value.as_str())
        {
            fields.push(("language".to_string(), language.to_string()));
        }
        let url = format!("{}/audio/transcriptions", self.base_url);
        let (status, body, latency_ms) = self
            .post_multipart(
                ctx,
                url.as_str(),
                fields,
                vec![("file".to_string(), filename, mime, bytes)],
            )
            .await?;
        if !status.is_success() {
            let message = body
                .pointer("/error/message")
                .and_then(|value| value.as_str())
                .unwrap_or("openai transcription returned non-success status");
            return Err(Self::classify_api_error(status, message.to_string()));
        }
        let text = body
            .get("text")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string());
        let mut extra = Map::new();
        extra.insert(
            "asr".to_string(),
            json!({
                "segments": body.get("segments").cloned().unwrap_or_else(|| Value::Array(vec![])),
                "provider_io": { "output": body },
                "latency_ms": latency_ms
            }),
        );
        Ok(ProviderStartResult::Immediate(AiResponse {
            message: AiResponse::message_from_parts(text, vec![], vec![]),
            finish_reason: Some("stop".to_string()),
            extra: Some(Value::Object(extra)),
            ..Default::default()
        }))
    }

    fn video_option(req: &AiMethodRequest, keys: &[&str]) -> Option<Value> {
        for source in [
            req.payload.input_json.as_ref(),
            req.payload.options.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            for key in keys {
                if let Some(value) = source.get(*key) {
                    return Some(value.clone());
                }
            }
        }
        None
    }

    fn video_size(req: &AiMethodRequest) -> Option<String> {
        let raw = Self::video_option(req, &["size", "resolution"])?;
        let value = raw.as_str()?.trim();
        if value.contains('x') {
            return Some(value.to_string());
        }
        let aspect_ratio = Self::video_option(req, &["aspect_ratio", "aspectRatio"])
            .and_then(|value| value.as_str().map(ToString::to_string))
            .unwrap_or_else(|| "16:9".to_string());
        match (value.to_ascii_lowercase().as_str(), aspect_ratio.as_str()) {
            ("720p", "9:16") => Some("720x1280".to_string()),
            ("720p", _) => Some("1280x720".to_string()),
            ("1024p", "9:16") => Some("1024x1792".to_string()),
            ("1024p", _) => Some("1792x1024".to_string()),
            _ => None,
        }
    }

    fn video_dimensions(size: &str) -> Option<(u32, u32)> {
        match size {
            "720x1280" => Some((720, 1280)),
            "1280x720" => Some((1280, 720)),
            "1024x1792" => Some((1024, 1792)),
            "1792x1024" => Some((1792, 1024)),
            _ => None,
        }
    }

    fn normalize_video_reference_image(
        bytes: &[u8],
        requested_size: Option<&str>,
    ) -> Result<(String, Vec<u8>), ProviderError> {
        let image = image::load_from_memory(bytes).map_err(|err| {
            ProviderError::fatal(format!(
                "failed to decode video input_reference image: {}",
                err
            ))
        })?;
        let size = requested_size.map(ToString::to_string).unwrap_or_else(|| {
            if image.width() > image.height() {
                "1280x720".to_string()
            } else {
                "720x1280".to_string()
            }
        });
        let (width, height) = Self::video_dimensions(size.as_str()).ok_or_else(|| {
            ProviderError::fatal(format!("unsupported OpenAI video size '{}'", size))
        })?;
        let normalized = image.resize_to_fill(width, height, FilterType::Lanczos3);
        let mut output = Cursor::new(Vec::new());
        normalized
            .write_to(&mut output, ImageFormat::Png)
            .map_err(|err| {
                ProviderError::fatal(format!(
                    "failed to encode normalized video input_reference image: {}",
                    err
                ))
            })?;
        Ok((size, output.into_inner()))
    }

    async fn start_video(
        &self,
        ctx: &crate::aicc::InvokeCtx,
        provider_model: &str,
        method: &str,
        req: &AiMethodRequest,
        sink: Arc<dyn TaskEventSink>,
    ) -> Result<ProviderStartResult, ProviderError> {
        let prompt = Self::extract_text2image_prompt(req).ok_or_else(|| {
            ProviderError::fatal(format!("{} requires a non-empty prompt", method))
        })?;
        let mut fields = vec![
            ("model".to_string(), provider_model.to_string()),
            ("prompt".to_string(), prompt),
        ];
        if let Some(seconds) = Self::video_option(
            req,
            &["seconds", "duration", "duration_seconds", "durationSeconds"],
        ) {
            fields.push(("seconds".to_string(), value_to_form_field(&seconds)));
        }
        let requested_size = Self::video_size(req);

        let mut files = vec![];
        if method == ai_methods::VIDEO_IMG2VIDEO {
            let resource = req
                .payload
                .resources
                .first()
                .cloned()
                .or_else(|| Self::resource_from_input_json(req, &["image", "images"]))
                .ok_or_else(|| {
                    ProviderError::fatal("video.img2video requires canonical image input")
                })?;
            let (name, mime, bytes) = self
                .resource_to_file_bytes(&resource, "input_reference.png")
                .await?;
            let normalize_size = requested_size.clone();
            let (size, normalized_bytes) = tokio::task::spawn_blocking(move || {
                Self::normalize_video_reference_image(bytes.as_slice(), normalize_size.as_deref())
            })
            .await
            .map_err(|err| {
                ProviderError::fatal(format!(
                    "failed to normalize video input_reference image: {}",
                    err
                ))
            })??;
            info!(
                "aicc.openai.video.input_reference_normalized provider_instance_name={} model={} source_name={} source_mime={} target_size={}",
                self.instance.provider_instance_name,
                provider_model,
                name,
                mime,
                size
            );
            fields.push(("size".to_string(), size));
            files.push((
                "input_reference".to_string(),
                "input_reference.png".to_string(),
                "image/png".to_string(),
                normalized_bytes,
            ));
        } else if let Some(size) = requested_size {
            fields.push(("size".to_string(), size));
        }

        let started_at = std::time::Instant::now();
        let url = format!("{}/videos", self.base_url);
        let (status, job, _) = self
            .post_multipart(ctx, url.as_str(), fields, files)
            .await?;
        if !status.is_success() {
            let message = job
                .pointer("/error/message")
                .and_then(|value| value.as_str())
                .unwrap_or("openai video create returned non-success status");
            return Err(Self::classify_api_error(status, message.to_string()));
        }
        let video_id = job
            .get("id")
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ProviderError::fatal("openai video create response is missing id"))?
            .to_string();

        let provider = self.clone();
        let task_id = ctx.task_id.clone().unwrap_or_else(|| video_id.clone());
        let invoke_ctx = ctx.clone();
        let provider_model = provider_model.to_string();
        let method = method.to_string();
        let request = req.clone();
        tokio::spawn(async move {
            let result = provider
                .finish_video(
                    &invoke_ctx,
                    provider_model.as_str(),
                    method.as_str(),
                    video_id.as_str(),
                    job,
                    started_at,
                )
                .await;
            emit_background_provider_result(sink, task_id.as_str(), &request, result).await;
        });

        Ok(ProviderStartResult::Started)
    }

    async fn finish_video(
        &self,
        ctx: &crate::aicc::InvokeCtx,
        provider_model: &str,
        method: &str,
        video_id: &str,
        mut job: Value,
        started_at: std::time::Instant,
    ) -> Result<AiResponse, ProviderError> {
        loop {
            let state = job
                .get("status")
                .and_then(|value| value.as_str())
                .unwrap_or("queued");
            match state {
                "completed" => break,
                "queued" | "in_progress" => {}
                "failed" => {
                    let message = job
                        .pointer("/error/message")
                        .and_then(|value| value.as_str())
                        .unwrap_or("openai video generation failed");
                    return Err(ProviderError::fatal(message.to_string()));
                }
                other => {
                    return Err(ProviderError::fatal(format!(
                        "openai video generation returned unknown status '{}'",
                        other
                    )));
                }
            }
            if started_at.elapsed() >= OPENAI_VIDEO_MAX_WAIT {
                return Err(ProviderError::fatal(format!(
                    "openai video generation timed out after {} seconds",
                    OPENAI_VIDEO_MAX_WAIT.as_secs()
                )));
            }
            time::sleep(OPENAI_VIDEO_POLL_INTERVAL).await;
            let status_url = format!("{}/videos/{}", self.base_url, video_id);
            let (poll_status, next_job) = self.get_json(ctx, status_url.as_str()).await?;
            if !poll_status.is_success() {
                let message = next_job
                    .pointer("/error/message")
                    .and_then(|value| value.as_str())
                    .unwrap_or("openai video status returned non-success status");
                return Err(ProviderError::fatal(message.to_string()));
            }
            job = next_job;
        }

        let content_url = format!("{}/videos/{}/content", self.base_url, video_id);
        let (download_status, bytes, content_type) =
            self.get_binary(ctx, content_url.as_str()).await?;
        if !download_status.is_success() {
            return Err(ProviderError::fatal(format!(
                "openai video download returned status {}",
                download_status.as_u16()
            )));
        }
        let mime = if content_type.contains("video/") {
            content_type
        } else {
            "video/mp4".to_string()
        };
        let artifact = AiArtifact {
            name: "video.mp4".to_string(),
            resource: ResourceRef::Base64 {
                mime: mime.clone(),
                data_base64: general_purpose::STANDARD.encode(bytes),
            },
            mime: Some(mime),
            metadata: None,
        };
        Ok(AiResponse {
            message: AiResponse::message_from_parts(None, vec![], vec![artifact]),
            usage: Some(AiUsage::request_units(1)),
            cost: Some(AiCost {
                amount: if provider_model.contains("pro") {
                    1.2
                } else {
                    0.4
                },
                currency: "USD".to_string(),
            }),
            finish_reason: Some("stop".to_string()),
            provider_task_ref: Some(video_id.to_string()),
            extra: Some(json!({
                "provider": "openai",
                "method": method,
                "model": provider_model,
                "latency_ms": started_at.elapsed().as_millis() as u64,
                "provider_io": { "output": job }
            })),
            ..Default::default()
        })
    }

    async fn start_image_edit(
        &self,
        ctx: &crate::aicc::InvokeCtx,
        provider_model: &str,
        req: &AiMethodRequest,
        with_mask: bool,
    ) -> Result<ProviderStartResult, ProviderError> {
        if !with_mask && self.model_supports_responses_image_generation(provider_model) {
            return self
                .start_responses_image_generation(ctx, provider_model, req, "edit")
                .await;
        }
        let image_resource = req
            .payload
            .resources
            .first()
            .cloned()
            .or_else(|| Self::resource_from_input_json(req, &["image", "images"]))
            .ok_or_else(|| ProviderError::fatal("image edit requires canonical image input"))?;
        let (image_name, image_mime, image_bytes) = self
            .resource_to_file_bytes(&image_resource, "image.png")
            .await?;
        let mut files = vec![("image".to_string(), image_name, image_mime, image_bytes)];
        if with_mask {
            let mask_resource = req
                .payload
                .resources
                .get(1)
                .cloned()
                .or_else(|| Self::resource_from_input_json(req, &["mask"]))
                .ok_or_else(|| {
                    ProviderError::fatal("image.inpaint requires canonical mask input")
                })?;
            let (mask_name, mask_mime, mask_bytes) = self
                .resource_to_file_bytes(&mask_resource, "mask.png")
                .await?;
            files.push(("mask".to_string(), mask_name, mask_mime, mask_bytes));
        }

        let prompt = Self::extract_text2image_prompt(req).ok_or_else(|| {
            ProviderError::fatal("image edit requires prompt in payload.text/input_json/options")
        })?;
        let mut fields = vec![
            ("model".to_string(), provider_model.to_string()),
            ("prompt".to_string(), prompt),
        ];
        for source in [
            req.payload.input_json.as_ref(),
            req.payload.options.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if let Some(map) = source.as_object() {
                for (key, value) in map {
                    if key == "prompt" || key == "model" {
                        continue;
                    }
                    if key == "input_fidelity"
                        && (provider_model.starts_with("gpt-image-2")
                            || provider_model.starts_with("gpt-image-1-mini"))
                    {
                        continue;
                    }
                    if OPENAI_IMAGE_EDIT_OPTION_ALLOWLIST.contains(&key.as_str()) {
                        fields.push((key.clone(), value_to_form_field(value)));
                    }
                }
            }
        }
        let url = format!("{}/images/edits", self.base_url);
        let (status, body, latency_ms) = self
            .post_multipart(ctx, url.as_str(), fields, files)
            .await?;
        if !status.is_success() {
            let message = body
                .pointer("/error/message")
                .and_then(|value| value.as_str())
                .unwrap_or("openai image edit returned non-success status");
            return Err(Self::classify_api_error(status, message.to_string()));
        }
        let artifacts = Self::parse_text2image_artifacts(&body)?;
        let mut extra = Map::new();
        extra.insert("provider".to_string(), Value::String("openai".to_string()));
        extra.insert(
            "model".to_string(),
            Value::String(provider_model.to_string()),
        );
        extra.insert("latency_ms".to_string(), Value::from(latency_ms));
        extra.insert("provider_io".to_string(), json!({ "output": body }));
        Ok(ProviderStartResult::Immediate(AiResponse {
            message: AiResponse::message_from_parts(None, vec![], artifacts),
            finish_reason: Some("stop".to_string()),
            extra: Some(Value::Object(extra)),
            ..Default::default()
        }))
    }
}

#[async_trait]
impl Provider for OpenAIProvider {
    fn inventory(&self) -> ProviderInventory {
        self.inventory
            .read()
            .map(|inventory| inventory.clone())
            .unwrap_or_else(|_| {
                Self::default_inventory(
                    self.instance.provider_instance_name.as_str(),
                    self.provider_type.clone(),
                    self.provider_driver.as_str(),
                )
            })
    }

    fn shutdown(&self) {
        self.stop_inventory_refresh();
    }

    fn estimate_cost(&self, input: &CostEstimateInput) -> CostEstimateOutput {
        let provider_model = provider_model_from_exact(input.exact_model.as_str());
        if matches!(
            input.api_type,
            ApiType::ImageTextToImage | ApiType::ImageToImage | ApiType::ImageInpaint
        ) {
            return CostEstimateOutput {
                estimated_cost_usd: 0.04,
                pricing_mode: PricingMode::PerToken,
                quota_state: QuotaState::Normal,
                confidence: 0.5,
                estimated_latency_ms: Some(5000),
            };
        }
        if matches!(
            input.api_type,
            ApiType::VideoTextToVideo | ApiType::VideoImageToVideo
        ) {
            return CostEstimateOutput {
                estimated_cost_usd: if provider_model.contains("pro") {
                    1.2
                } else {
                    0.4
                },
                pricing_mode: PricingMode::Unknown,
                quota_state: QuotaState::Normal,
                confidence: 0.5,
                estimated_latency_ms: Some(120_000),
            };
        }

        let input_tokens = input.input_tokens.max(1);
        let output_tokens = input.estimated_output_tokens.unwrap_or(1024).max(1);
        let usage = AiUsage {
            input_tokens: Some(input_tokens),
            output_tokens: Some(output_tokens),
            total_tokens: Some(input_tokens.saturating_add(output_tokens)),
            request_units: None,
        };

        let estimated_cost_usd = self
            .estimate_cost_for_usage(provider_model, &usage)
            .map(|cost| cost.amount)
            .unwrap_or(1.0);

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
        ctx: crate::aicc::InvokeCtx,
        provider_model: String,
        req: ResolvedRequest,
        sink: Arc<dyn TaskEventSink>,
    ) -> std::result::Result<ProviderStartResult, ProviderError> {
        match req.method.as_str() {
            ai_methods::LLM_CHAT => {
                self.start_llm(&ctx, provider_model.as_str(), &req.request)
                    .await
            }
            ai_methods::IMAGE_TXT2IMG => {
                self.start_text2image(&ctx, provider_model.as_str(), &req.request)
                    .await
            }
            ai_methods::IMAGE_IMG2IMG => {
                self.start_image_edit(&ctx, provider_model.as_str(), &req.request, false)
                    .await
            }
            ai_methods::IMAGE_INPAINT => {
                self.start_image_edit(&ctx, provider_model.as_str(), &req.request, true)
                    .await
            }
            ai_methods::EMBEDDING_TEXT => {
                self.start_embedding(&ctx, provider_model.as_str(), &req.request)
                    .await
            }
            ai_methods::RERANK => {
                self.start_rerank(&ctx, provider_model.as_str(), &req.request)
                    .await
            }
            ai_methods::AUDIO_TTS => {
                self.start_tts(&ctx, provider_model.as_str(), &req.request)
                    .await
            }
            ai_methods::AUDIO_ASR => {
                self.start_asr(&ctx, provider_model.as_str(), &req.request)
                    .await
            }
            ai_methods::VISION_OCR | ai_methods::VISION_CAPTION => {
                self.start_vision(
                    &ctx,
                    provider_model.as_str(),
                    req.method.as_str(),
                    &req.request,
                )
                .await
            }
            ai_methods::VIDEO_TXT2VIDEO | ai_methods::VIDEO_IMG2VIDEO => {
                self.start_video(
                    &ctx,
                    provider_model.as_str(),
                    req.method.as_str(),
                    &req.request,
                    sink,
                )
                .await
            }
            method => Err(ProviderError::fatal(format!(
                "openai provider does not support method '{}'",
                method
            ))),
        }
    }

    async fn cancel(
        &self,
        _ctx: crate::aicc::InvokeCtx,
        _task_id: &str,
    ) -> std::result::Result<(), ProviderError> {
        Err(ProviderError::fatal(
            "openai provider cancellation is unsupported",
        ))
    }
}

impl Drop for OpenAIProvider {
    fn drop(&mut self) {
        self.stop_inventory_refresh();
    }
}

fn provider_model_from_exact(exact_model: &str) -> &str {
    exact_model
        .rsplit_once('@')
        .map(|(model, _)| model)
        .unwrap_or(exact_model)
}

fn remote_model_resolve_request(model: &ModelMetadata) -> Option<DriverModelResolveRequest> {
    let provider_model_id = remote_provider_model_id(model)?;
    if provider_model_id.is_empty() {
        return None;
    }
    let mut api_types = model
        .api_types
        .iter()
        .filter(|api_type| is_supported_openai_api_type(api_type))
        .cloned()
        .collect::<Vec<_>>();
    if api_types.is_empty() {
        if is_text2image_model_name(provider_model_id.as_str()) {
            api_types.push(ApiType::ImageTextToImage);
        } else if is_supported_llm_model_name(provider_model_id.as_str()) {
            api_types.push(ApiType::Llm);
        } else {
            return None;
        }
    }
    Some(
        DriverModelResolveRequest::new(provider_model_id, api_types)
            .with_cost(model.pricing.estimated_cost)
            .with_latency(model.health.p50_latency_ms.or(model.health.p95_latency_ms)),
    )
}

fn remote_provider_model_id(model: &ModelMetadata) -> Option<String> {
    let provider_model_id = model.provider_model_id.trim();
    if !provider_model_id.is_empty() {
        return Some(provider_model_id.to_string());
    }
    let provider_model_id = provider_model_from_exact(model.exact_model.as_str()).trim();
    (!provider_model_id.is_empty()).then(|| provider_model_id.to_string())
}

fn merge_remote_dynamic_metadata(model: &mut ModelMetadata, remote_model: &ModelMetadata) {
    model.pricing.currency = remote_model.pricing.currency.clone();
    if remote_model.pricing.input_token.is_some() {
        model.pricing.input_token = remote_model.pricing.input_token;
    }
    if remote_model.pricing.output_token.is_some() {
        model.pricing.output_token = remote_model.pricing.output_token;
    }
    if remote_model.pricing.cache_input_token.is_some() {
        model.pricing.cache_input_token = remote_model.pricing.cache_input_token;
    }
    if remote_model.pricing.estimated_cost.is_some() {
        model.pricing.estimated_cost = remote_model.pricing.estimated_cost;
    }
    if remote_model.health.p50_latency_ms.is_some() {
        model.health.p50_latency_ms = remote_model.health.p50_latency_ms;
    }
    if remote_model.health.p95_latency_ms.is_some() {
        model.health.p95_latency_ms = remote_model.health.p95_latency_ms;
    }
    if remote_model.health.error_rate_5m.is_some() {
        model.health.error_rate_5m = remote_model.health.error_rate_5m;
    }
    if remote_model.health.recent_failures.is_some() {
        model.health.recent_failures = remote_model.health.recent_failures;
    }
    if remote_model.health.queue_depth.is_some() {
        model.health.queue_depth = remote_model.health.queue_depth;
    }
    if remote_model.health.status != Default::default() {
        model.health.status = remote_model.health.status.clone();
    }
    if remote_model.health.quota_state != Default::default() {
        model.health.quota_state = remote_model.health.quota_state.clone();
    }
}

fn is_supported_openai_api_type(api_type: &ApiType) -> bool {
    matches!(
        api_type,
        ApiType::Llm
            | ApiType::Embedding
            | ApiType::Rerank
            | ApiType::VisionOcr
            | ApiType::VisionCaption
            | ApiType::ImageTextToImage
            | ApiType::ImageToImage
            | ApiType::ImageInpaint
            | ApiType::AudioAsr
            | ApiType::AudioTts
    )
}

fn value_to_form_field(value: &Value) -> String {
    value
        .as_str()
        .map(|value| value.to_string())
        .unwrap_or_else(|| value.to_string())
}

#[cfg(test)]
fn json_text_len(value: &Value) -> usize {
    match value {
        Value::String(text) => text.len(),
        Value::Array(items) => items.iter().map(json_text_len).sum(),
        Value::Object(map) => map.values().map(json_text_len).sum(),
        _ => 0,
    }
}

#[derive(Debug, Deserialize, Default)]
struct OpenAISettings {
    #[serde(default = "default_openai_enabled")]
    enabled: bool,
    #[serde(default)]
    api_token: String,
    #[serde(default)]
    instances: Vec<SettingsOpenAIInstanceConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct SettingsOpenAIInstanceConfig {
    #[serde(default = "default_instance_id", alias = "instance_id")]
    provider_instance_name: String,
    #[serde(default = "default_provider_type")]
    provider_type: String,
    #[serde(default)]
    provider_driver: String,
    #[serde(default, alias = "api_key", alias = "apiKey")]
    api_token: String,
    #[serde(default = "default_base_url")]
    base_url: String,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
}

fn default_openai_enabled() -> bool {
    true
}

fn default_instance_id() -> String {
    "openai-default".to_string()
}

fn default_provider_type() -> String {
    "cloud_api".to_string()
}

fn default_base_url() -> String {
    DEFAULT_OPENAI_BASE_URL.to_string()
}

fn default_timeout_ms() -> u64 {
    DEFAULT_OPENAI_TIMEOUT_MS
}

fn default_provider_driver_for_instance(_provider_instance_name: &str, _base_url: &str) -> String {
    DEFAULT_OPENAI_PROVIDER_DRIVER.to_string()
}

fn is_text2image_model_name(model: &str) -> bool {
    let normalized = model.trim().to_ascii_lowercase();
    normalized.starts_with("dall-e") || normalized.starts_with("gpt-image")
}

fn is_supported_llm_model_name(model: &str) -> bool {
    let normalized = model.trim().to_ascii_lowercase();
    if normalized.is_empty() || is_text2image_model_name(normalized.as_str()) {
        return false;
    }
    // gpt-* 命名族里这些是 ASR / TTS / 实时音频 modality（例如
    // gpt-4o-mini-transcribe / gpt-4o-mini-tts / gpt-4o-audio-preview /
    // gpt-4o-realtime-preview），它们已经在 DEFAULT_OPENAI_{ASR,TTS}_MODELS
    // 里登记。如果再当 LLM 收一遍，build_inventory 会产出两条 exact_model
    // 相同的 metadata，被 model_registry::validate_inventory 拒为
    // SessionConfigInvalid，整个 registry refresh 会卡死。
    if normalized.contains("transcribe")
        || normalized.contains("-tts")
        || normalized.contains("-audio")
        || normalized.contains("-realtime")
    {
        return false;
    }

    normalized.starts_with("gpt-")
        || normalized.starts_with("chatgpt-")
        || normalized.starts_with("sora-")
        || normalized.starts_with("o1")
        || normalized.starts_with("o3")
        || normalized.starts_with("o4")
}

fn normalize_remote_model_ids(
    entries: Vec<OpenAIModelEntry>,
    provider_driver: &str,
) -> (
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
) {
    let mut llm_seen = HashSet::<String>::new();
    let mut image_seen = HashSet::<String>::new();
    let mut embedding_seen = HashSet::<String>::new();
    let mut asr_seen = HashSet::<String>::new();
    let mut tts_seen = HashSet::<String>::new();
    let mut llm_models = Vec::new();
    let mut image_models = Vec::new();
    let mut embedding_models = Vec::new();
    let mut asr_models = Vec::new();
    let mut tts_models = Vec::new();

    for entry in entries.into_iter() {
        let model = entry.id.trim();
        if model.is_empty() {
            continue;
        }
        let key = model.to_ascii_lowercase();
        if provider_driver == "openrouter" {
            if llm_seen.insert(key) {
                llm_models.push(model.to_string());
            }
            continue;
        }
        if key.starts_with("gpt-live-") || key.starts_with("gpt-realtime-") {
            continue;
        } else if key.contains("embedding") {
            if embedding_seen.insert(key) {
                embedding_models.push(model.to_string());
            }
        } else if key.contains("transcribe") || key == "whisper-1" {
            if asr_seen.insert(key) {
                asr_models.push(model.to_string());
            }
        } else if key.starts_with("tts-") || key.contains("-tts") {
            if tts_seen.insert(key) {
                tts_models.push(model.to_string());
            }
        } else if key.contains("realtime") || key.contains("audio") {
            continue;
        } else if is_text2image_model_name(model) {
            if image_seen.insert(key) {
                image_models.push(model.to_string());
            }
        } else if is_supported_llm_model_name(model) && llm_seen.insert(key) {
            llm_models.push(model.to_string());
        }
    }

    (
        llm_models,
        image_models,
        embedding_models,
        asr_models,
        tts_models,
    )
}

fn inventory_revision(
    models: &[String],
    image_models: &[String],
    embedding_models: &[String],
    asr_models: &[String],
    tts_models: &[String],
) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    models.hash(&mut hasher);
    image_models.hash(&mut hasher);
    embedding_models.hash(&mut hasher);
    asr_models.hash(&mut hasher);
    tts_models.hash(&mut hasher);
    format!(
        "models-{}-{}-{}-{}-{}-{:x}",
        models.len(),
        image_models.len(),
        embedding_models.len(),
        asr_models.len(),
        tts_models.len(),
        hasher.finish()
    )
}

fn inventory_revision_from_metadata(models: &[ModelMetadata], version: Option<&str>) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    version.hash(&mut hasher);
    for model in models.iter() {
        model.provider_model_id.hash(&mut hasher);
        model.exact_model.hash(&mut hasher);
        model.api_types.hash(&mut hasher);
        model.logical_mounts.hash(&mut hasher);
    }
    format!("provider-inventory-{}-{:x}", models.len(), hasher.finish())
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
            provider_instance_name: default_instance_id(),
            provider_type: default_provider_type(),
            provider_driver: String::new(),
            api_token: settings.api_token.clone(),
            base_url: default_base_url(),
            timeout_ms: default_timeout_ms(),
        }]
    } else {
        settings.instances.clone()
    };

    let mut instances = vec![];
    for raw_instance in raw_instances.into_iter() {
        instances.push(OpenAIInstanceConfig {
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
        });
    }

    Ok(instances)
}

#[cfg(test)]
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

    if let Some(default_model) = default_model.filter(|model| !is_text2image_model_name(model)) {
        for alias in ["llm.default", "llm.plan.default", "llm.code.default"] {
            center.model_catalog().set_mapping(
                Capability::Llm,
                alias,
                provider_type,
                default_model,
            );
        }
    }

    for model in image_models.iter() {
        center.model_catalog().set_mapping(
            Capability::Image,
            model.as_str(),
            provider_type,
            model.as_str(),
        );

        for alias in [
            format!("text2image.{}", model),
            format!("t2i.{}", model),
            format!("image.{}", model),
            format!("image.txt2img.{}", model),
        ] {
            center.model_catalog().set_mapping(
                Capability::Image,
                alias,
                provider_type,
                model.as_str(),
            );
        }
    }

    if let Some(default_image_model) = default_image_model {
        for alias in [
            "text2image.default",
            "t2i.default",
            "image.default",
            "image.txt2img.default",
        ] {
            center.model_catalog().set_mapping(
                Capability::Image,
                alias,
                provider_type,
                default_image_model,
            );
        }
    }
}

#[cfg(test)]
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
            Capability::Image
        } else {
            Capability::Llm
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
        info!("aicc openai provider is disabled (settings.openai missing or disabled)");
        return Ok(0);
    };
    let instances = build_openai_instances(&openai_settings)?;
    let mut prepared = Vec::<(OpenAIInstanceConfig, Arc<OpenAIProvider>)>::new();
    for config in instances.iter() {
        let provider = Arc::new(OpenAIProvider::new(
            config.clone(),
            config.api_token.as_str(),
        )?);
        prepared.push((config.clone(), provider));
    }

    for (config, provider) in prepared.into_iter() {
        provider.clone().start_inventory_refresh();
        let inventory = center.registry().add_provider(provider);
        info!(
            "registered openai base_url={} inventory={:?}",
            config.base_url, inventory
        );
        center
            .model_registry()
            .write()
            .map_err(|_| anyhow!("model registry lock poisoned"))?
            .apply_inventory(inventory)
            .map_err(|err| anyhow!("failed to apply openai inventory: {}", err))?;
    }

    Ok(instances.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aicc::ModelCatalog;
    use buckyos_api::{AiPayload, ModelSpec, Requirements};
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn build_llm_request(options: Option<Value>) -> AiMethodRequest {
        AiMethodRequest::new(
            Capability::Llm,
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

    #[tokio::test]
    async fn resource_url_retries_incomplete_response_body() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let address = listener.local_addr().expect("address");
        let server = tokio::spawn(async move {
            for response in [
                b"HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nContent-Length: 10\r\nConnection: close\r\n\r\nbad".as_slice(),
                b"HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nContent-Length: 4\r\nConnection: close\r\n\r\ngood".as_slice(),
            ] {
                let (mut stream, _) = listener.accept().await.expect("accept");
                let mut request = [0u8; 1024];
                let _ = stream.read(&mut request).await.expect("read request");
                stream.write_all(response).await.expect("write response");
            }
        });
        let provider = OpenAIProvider::new(
            OpenAIInstanceConfig {
                provider_instance_name: "openai-resource-retry".to_string(),
                provider_type: "cloud_api".to_string(),
                provider_driver: "openai".to_string(),
                api_token: "token".to_string(),
                base_url: default_base_url(),
                timeout_ms: 1_000,
            },
            "token",
        )
        .expect("provider");

        let (_, mime, bytes) = provider
            .resource_to_file_bytes(
                &ResourceRef::Url {
                    url: format!("http://{address}/audio.wav"),
                    mime_hint: None,
                },
                "audio.wav",
            )
            .await
            .expect("retry should recover");

        assert_eq!(mime, "audio/wav");
        assert_eq!(bytes, b"good");
        server.await.expect("server task");
    }

    fn build_text2image_request(options: Option<Value>) -> AiMethodRequest {
        AiMethodRequest::new(
            Capability::Image,
            ModelSpec::new("text2image.default".to_string(), None),
            Requirements::default(),
            AiPayload::new(
                Some("draw a test image".to_string()),
                vec![],
                vec![],
                vec![],
                None,
                options,
            ),
            None,
        )
    }

    fn build_image_edit_request(
        resources: Vec<ResourceRef>,
        options: Option<Value>,
    ) -> AiMethodRequest {
        AiMethodRequest::new(
            Capability::Image,
            ModelSpec::new("image.img2img.default".to_string(), None),
            Requirements::default(),
            AiPayload::new(
                Some("edit the test image".to_string()),
                vec![],
                vec![],
                resources,
                None,
                options,
            ),
            None,
        )
    }

    fn build_video_request(options: Value) -> AiMethodRequest {
        AiMethodRequest::new(
            Capability::Video,
            ModelSpec::new("video.sora".to_string(), None),
            Requirements::default(),
            AiPayload::new(
                Some("animate the image".to_string()),
                vec![],
                vec![],
                vec![],
                Some(options),
                None,
            ),
            None,
        )
    }

    fn assert_model_mount(
        inventory: &ProviderInventory,
        provider_model_id: &str,
        mount: &str,
        expected: bool,
    ) {
        let model = inventory
            .models
            .iter()
            .find(|model| model.provider_model_id == provider_model_id)
            .expect("model should exist");
        assert_eq!(
            model
                .logical_mounts
                .iter()
                .any(|item| item.as_str() == mount),
            expected,
            "unexpected mount state for model={} mount={}",
            provider_model_id,
            mount
        );
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
    fn price_table_covers_current_gpt5_family_models() {
        assert_eq!(OpenAIProvider::price_per_1m_tokens("gpt-5"), (1.25, 10.0));
        assert_eq!(
            OpenAIProvider::price_per_1m_tokens("gpt-5-mini"),
            (0.25, 2.0)
        );
        assert_eq!(
            OpenAIProvider::price_per_1m_tokens("gpt-5-nano"),
            (0.05, 0.4)
        );
        assert_eq!(
            OpenAIProvider::price_per_1m_tokens("gpt-5-nono"),
            (0.05, 0.4)
        );
        assert_eq!(
            OpenAIProvider::price_per_1m_tokens("gpt-5-pro"),
            (15.0, 120.0)
        );
        assert_eq!(OpenAIProvider::price_per_1m_tokens("gpt-5.4"), (2.5, 15.0));
        assert_eq!(
            OpenAIProvider::price_per_1m_tokens("gpt-5.4-mini"),
            (0.75, 4.5)
        );
        assert_eq!(
            OpenAIProvider::price_per_1m_tokens("gpt-5.4-nano"),
            (0.20, 1.25)
        );
        assert_eq!(
            OpenAIProvider::price_per_1m_tokens("gpt-5.4-pro"),
            (30.0, 180.0)
        );
        assert_eq!(
            OpenAIProvider::price_per_1m_tokens("gpt-5.6-sol"),
            (4.0, 20.0)
        );
        assert_eq!(OpenAIProvider::price_per_1m_tokens("gpt-5.6"), (4.0, 20.0));
        assert_eq!(
            OpenAIProvider::price_per_1m_tokens("gpt-5.6-terra"),
            (2.0, 12.0)
        );
        assert_eq!(
            OpenAIProvider::price_per_1m_tokens("gpt-5.6-luna"),
            (0.20, 1.20)
        );
    }

    #[test]
    fn estimate_text2image_cost_supports_gpt_image_1_quality_and_size() {
        let medium_square = build_text2image_request(None);
        assert_eq!(
            OpenAIProvider::estimate_text2image_cost(&medium_square, "gpt-image-1"),
            Some(0.042)
        );

        let high_landscape = build_text2image_request(Some(json!({
            "quality": "high",
            "size": "1536x1024",
            "n": 2
        })));
        assert_eq!(
            OpenAIProvider::estimate_text2image_cost(&high_landscape, "gpt-image-1"),
            Some(0.5)
        );
    }

    #[test]
    fn normalize_video_reference_uses_orientation_default_size() {
        let mut source = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(640, 480)
            .write_to(&mut source, ImageFormat::Png)
            .expect("encode source image");

        let (size, normalized) =
            OpenAIProvider::normalize_video_reference_image(source.get_ref().as_slice(), None)
                .expect("normalize video reference");
        let normalized = image::load_from_memory(normalized.as_slice())
            .expect("decode normalized video reference");

        assert_eq!(size, "1280x720");
        assert_eq!((normalized.width(), normalized.height()), (1280, 720));
    }

    #[test]
    fn video_size_only_maps_openai_supported_dimensions() {
        assert_eq!(
            OpenAIProvider::video_size(&build_video_request(json!({
                "resolution": "1024p",
                "aspect_ratio": "9:16"
            }))),
            Some("1024x1792".to_string())
        );
        assert_eq!(
            OpenAIProvider::video_size(&build_video_request(json!({
                "resolution": "1080p",
                "aspect_ratio": "16:9"
            }))),
            None
        );
        assert_eq!(OpenAIProvider::video_dimensions("1920x1080"), None);
    }

    #[test]
    fn normalize_video_reference_honors_requested_size() {
        let mut source = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(480, 640)
            .write_to(&mut source, ImageFormat::Png)
            .expect("encode source image");

        let (size, normalized) = OpenAIProvider::normalize_video_reference_image(
            source.get_ref().as_slice(),
            Some("1024x1792"),
        )
        .expect("normalize video reference");
        let normalized = image::load_from_memory(normalized.as_slice())
            .expect("decode normalized video reference");

        assert_eq!(size, "1024x1792");
        assert_eq!((normalized.width(), normalized.height()), (1024, 1792));
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
    fn normalize_web_search_reasoning_promotes_minimal_effort() {
        let mut request = json!({
            "model": "gpt-5-nano",
            "input": "hello",
            "reasoning": {
                "effort": "minimal"
            },
            "tools": [{
                "type": OPENAI_TOOL_TYPE_WEB_SEARCH
            }]
        })
        .as_object()
        .cloned()
        .expect("request object");

        assert!(OpenAIProvider::normalize_web_search_reasoning(&mut request));
        assert_eq!(
            Value::Object(request.clone())
                .pointer("/reasoning/effort")
                .and_then(|value| value.as_str()),
            Some("low")
        );
    }

    #[test]
    fn normalize_web_search_reasoning_leaves_non_web_search_requests() {
        let mut request = json!({
            "model": "gpt-5-nano",
            "input": "hello",
            "reasoning": {
                "effort": "minimal"
            },
            "tools": [{
                "type": "function",
                "name": "lookup"
            }]
        })
        .as_object()
        .cloned()
        .expect("request object");

        assert!(!OpenAIProvider::normalize_web_search_reasoning(
            &mut request
        ));
        assert_eq!(
            Value::Object(request.clone())
                .pointer("/reasoning/effort")
                .and_then(|value| value.as_str()),
            Some("minimal")
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
    fn parse_sse_response_body_supports_chat_completions_stream_chunks() {
        let raw = r#"data: {"id":"chatcmpl-1","object":"chat.completion.chunk","choices":[{"delta":{"content":"foo"},"index":0}]}

data: {"id":"chatcmpl-1","object":"chat.completion.chunk","choices":[{"delta":{"content":"bar"},"index":0}]}

data: {"id":"chatcmpl-1","object":"chat.completion.chunk","choices":[{"delta":{},"finish_reason":"stop","index":0}]}

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
    fn build_openai_instances_uses_simplified_runtime_inventory_config() {
        let settings = OpenAISettings {
            enabled: true,
            api_token: "token".to_string(),
            instances: vec![SettingsOpenAIInstanceConfig {
                provider_instance_name: "openai-1".to_string(),
                provider_type: "cloud_api".to_string(),
                provider_driver: "openai".to_string(),
                api_token: String::new(),
                base_url: default_base_url(),
                timeout_ms: default_timeout_ms(),
            }],
        };

        let instances = build_openai_instances(&settings).expect("instances should be built");
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].provider_instance_name, "openai-1");
        assert_eq!(instances[0].base_url, DEFAULT_OPENAI_BASE_URL);
    }

    #[test]
    fn build_openai_instances_preserves_aggregator_driver() {
        let settings = OpenAISettings {
            enabled: true,
            api_token: "token".to_string(),
            instances: vec![SettingsOpenAIInstanceConfig {
                provider_instance_name: "openrouter-main".to_string(),
                provider_type: "cloud_api".to_string(),
                provider_driver: "openrouter".to_string(),
                api_token: String::new(),
                base_url: "https://openrouter.ai/api/v1".to_string(),
                timeout_ms: default_timeout_ms(),
            }],
        };

        let instances = build_openai_instances(&settings).expect("instances should be built");
        let provider =
            OpenAIProvider::new(instances[0].clone(), "token").expect("provider should be built");
        assert_eq!(provider.instance.provider_driver, "openrouter");
        assert!(provider.inventory().models.is_empty());
    }

    #[tokio::test]
    async fn auth_always_uses_configured_api_token() {
        let provider = OpenAIProvider::new(
            OpenAIInstanceConfig {
                provider_instance_name: "openai-auth".to_string(),
                provider_type: "cloud_api".to_string(),
                provider_driver: "openai".to_string(),
                api_token: "configured-token".to_string(),
                base_url: default_base_url(),
                timeout_ms: default_timeout_ms(),
            },
            "configured-token",
        )
        .expect("provider");
        let ctx = crate::aicc::InvokeCtx {
            session_token: Some("runtime-session-token".to_string()),
            ..Default::default()
        };

        assert_eq!(
            provider.build_auth_token(&ctx).await.expect("token"),
            "configured-token"
        );
    }

    #[tokio::test]
    async fn stopped_refresh_does_not_send_initial_request() {
        let provider = Arc::new(
            OpenAIProvider::new(
                OpenAIInstanceConfig {
                    provider_instance_name: "openai-stopped".to_string(),
                    provider_type: "cloud_api".to_string(),
                    provider_driver: "openai".to_string(),
                    api_token: "token".to_string(),
                    base_url: "http://127.0.0.1:1".to_string(),
                    timeout_ms: 100,
                },
                "token",
            )
            .expect("provider"),
        );
        let provider_instance_name = provider.instance.provider_instance_name.clone();
        let (refresh_task, shutdown_rx) = ProviderRefreshTask::new();
        refresh_task.shutdown();

        OpenAIProvider::run_inventory_refresh(
            Arc::downgrade(&provider),
            provider_instance_name,
            refresh_task.clone(),
            shutdown_rx,
        )
        .await;

        assert_eq!(refresh_task.started_requests(), 0);
    }

    #[test]
    fn dropping_provider_stops_refresh_task() {
        let provider = OpenAIProvider::new(
            OpenAIInstanceConfig {
                provider_instance_name: "openai-drop".to_string(),
                provider_type: "cloud_api".to_string(),
                provider_driver: "openai".to_string(),
                api_token: "token".to_string(),
                base_url: "http://127.0.0.1:1".to_string(),
                timeout_ms: 100,
            },
            "token",
        )
        .expect("provider");
        let (refresh_task, _) = ProviderRefreshTask::new();
        *provider.refresh_task.lock().expect("refresh task lock") = Some(refresh_task.clone());

        drop(provider);

        assert!(refresh_task.is_stopped());
    }

    #[tokio::test]
    async fn registration_error_does_not_start_prepared_refresh() {
        let center = AIComputeCenter::default();
        let settings = json!({
            "openai": {
                "enabled": true,
                "instances": [
                    {
                        "provider_instance_name": "openai-valid",
                        "api_token": "token",
                        "base_url": "http://127.0.0.1:1",
                        "timeout_ms": 100
                    },
                    {
                        "provider_instance_name": "openai-invalid"
                    }
                ]
            }
        });

        let result = register_openai_llm_providers(&center, &settings);

        assert!(result.is_err());
        assert!(center.registry().inventories().is_empty());
    }

    #[test]
    fn use_chat_completions_endpoint_detects_custom_compatible_path() {
        let provider = OpenAIProvider::new(
            OpenAIInstanceConfig {
                provider_instance_name: "custom-compatible-1".to_string(),
                provider_type: "cloud_api".to_string(),
                provider_driver: "custom-compatible".to_string(),
                api_token: "token".to_string(),
                base_url: "https://example.com/api/v1/chat/completions".to_string(),
                timeout_ms: default_timeout_ms(),
            },
            "token",
        )
        .expect("provider should be built");
        assert!(provider.use_chat_completions_endpoint());
    }

    #[test]
    fn default_inventory_uses_provider_instance_exact_model_names() {
        let provider = OpenAIProvider::new(
            OpenAIInstanceConfig {
                provider_instance_name: "openai-primary".to_string(),
                provider_type: "cloud_api".to_string(),
                provider_driver: "openai".to_string(),
                api_token: "token".to_string(),
                base_url: default_base_url(),
                timeout_ms: default_timeout_ms(),
            },
            "token",
        )
        .expect("provider should be built");

        let inventory = provider.inventory();
        assert_eq!(inventory.provider_driver, "openai");
        let gpt = inventory
            .models
            .iter()
            .find(|model| model.exact_model == "gpt-5.6-sol@openai-primary")
            .expect("default inventory should include gpt-5.6-sol");
        assert!(gpt.api_types.contains(&ApiType::VisionOcr));
        assert!(gpt.api_types.contains(&ApiType::VisionCaption));
        assert!(gpt.logical_mounts.iter().any(|mount| mount == "vision.ocr"));
        assert!(gpt
            .logical_mounts
            .iter()
            .any(|mount| mount == "vision.caption"));
        assert!(inventory
            .models
            .iter()
            .any(|model| model.exact_model == "gpt-image-2@openai-primary"));
        assert!(!inventory
            .models
            .iter()
            .any(|model| model.api_types.iter().any(|api_type| matches!(
                api_type,
                ApiType::VideoTextToVideo | ApiType::VideoImageToVideo
            ))));
    }

    #[test]
    fn remote_inventory_keeps_sora_video_models() {
        let (models, image_models, embedding_models, asr_models, tts_models) =
            normalize_remote_model_ids(
                vec![
                    OpenAIModelEntry {
                        id: "sora-2".to_string(),
                    },
                    OpenAIModelEntry {
                        id: "sora-2-pro".to_string(),
                    },
                ],
                "openai",
            );
        assert_eq!(models, vec!["sora-2", "sora-2-pro"]);
        assert!(image_models.is_empty());
        assert!(embedding_models.is_empty());
        assert!(asr_models.is_empty());
        assert!(tts_models.is_empty());
    }

    #[test]
    fn official_inventory_refresh_does_not_invent_absent_sora_models() {
        let provider = OpenAIProvider::new(
            OpenAIInstanceConfig {
                provider_instance_name: "openai-primary".to_string(),
                provider_type: "cloud_api".to_string(),
                provider_driver: "openai".to_string(),
                api_token: "token".to_string(),
                base_url: default_base_url(),
                timeout_ms: default_timeout_ms(),
            },
            "token",
        )
        .expect("provider should be built");
        let inventory = provider
            .build_inventory_from_remote_value(json!({
                "data": [{ "id": "gpt-5" }]
            }))
            .expect("inventory should resolve");
        assert!(!inventory
            .models
            .iter()
            .any(|model| model.provider_model_id == "sora-2"));
    }

    #[test]
    fn estimate_video_cost_uses_sora_render_pricing() {
        let provider = OpenAIProvider::new(
            OpenAIInstanceConfig {
                provider_instance_name: "openai-primary".to_string(),
                provider_type: "cloud_api".to_string(),
                provider_driver: "openai".to_string(),
                api_token: "token".to_string(),
                base_url: default_base_url(),
                timeout_ms: default_timeout_ms(),
            },
            "token",
        )
        .expect("provider should be built");
        let estimate = provider.estimate_cost(&CostEstimateInput {
            api_type: ApiType::VideoImageToVideo,
            exact_model: "sora-2-pro@openai-primary".to_string(),
            input_tokens: 0,
            estimated_output_tokens: None,
            cached_input_tokens: None,
            request_features: vec![],
        });
        assert_eq!(estimate.estimated_cost_usd, 1.2);
        assert_eq!(estimate.estimated_latency_ms, Some(120_000));
    }

    #[test]
    fn build_inventory_mounts_latest_gpt_tiers_to_family_models() {
        let models = vec![
            "gpt-5.4".to_string(),
            "gpt-5.5".to_string(),
            "gpt-5.6".to_string(),
            "gpt-5.6-sol".to_string(),
            "gpt-5.6-terra".to_string(),
            "gpt-5.6-luna".to_string(),
            "gpt-5.4-pro".to_string(),
            "gpt-5.5-pro".to_string(),
            "gpt-5-mini".to_string(),
            "gpt-5.4-mini".to_string(),
            "gpt-5.4-mini-2026-03-17".to_string(),
            "gpt-5-nano".to_string(),
            "gpt-5.4-nano".to_string(),
            "o1-2024-12-17".to_string(),
        ];

        let inventory = OpenAIProvider::build_inventory(
            "openai-primary",
            ProviderType::CloudApi,
            "openai",
            models.as_slice(),
            &[],
            &[],
            &[],
            &[],
            Some("test".to_string()),
        );

        assert_model_mount(&inventory, "gpt-5.5", "llm", false);
        assert_model_mount(&inventory, "gpt-5.5", "llm.code", false);
        assert_model_mount(&inventory, "gpt-5.5", "llm.gpt-standard", true);
        assert_model_mount(&inventory, "gpt-5.5", "llm.openai.gpt-5-5", true);
        assert!(
            inventory
                .models
                .iter()
                .find(|model| model.provider_model_id == "gpt-5.5")
                .expect("model should exist")
                .capabilities
                .vision
        );
        assert_model_mount(&inventory, "gpt-5.5", "llm.gpt", false);
        assert_model_mount(&inventory, "gpt-5.6", "llm.gpt-pro", false);
        assert_model_mount(&inventory, "gpt-5.6", "llm.gpt-standard", false);
        assert_model_mount(&inventory, "gpt-5.6-sol", "llm.gpt-pro", true);
        assert_model_mount(&inventory, "gpt-5.6-sol", "llm.gpt-standard", false);
        assert_model_mount(&inventory, "gpt-5.6-terra", "llm.gpt-mini", true);
        assert_model_mount(&inventory, "gpt-5.6-terra", "llm.gpt-standard", false);
        assert_model_mount(&inventory, "gpt-5.6-luna", "llm.gpt-nano", true);
        assert_model_mount(&inventory, "gpt-5.6-luna", "llm.gpt-standard", false);
        let sol = inventory
            .models
            .iter()
            .find(|model| model.provider_model_id == "gpt-5.6-sol")
            .expect("GPT-5.6 Sol should exist");
        assert_eq!(sol.capabilities.max_context_tokens, Some(1_050_000));
        assert_eq!(sol.capabilities.max_output_tokens, Some(128_000));
        assert_eq!(sol.pricing.input_token, Some(0.000004));
        let terra = inventory
            .models
            .iter()
            .find(|model| model.provider_model_id == "gpt-5.6-terra")
            .expect("GPT-5.6 Terra should exist");
        assert_eq!(terra.pricing.input_token, Some(0.000002));
        let luna = inventory
            .models
            .iter()
            .find(|model| model.provider_model_id == "gpt-5.6-luna")
            .expect("GPT-5.6 Luna should exist");
        assert_eq!(luna.pricing.input_token, Some(0.0000002));
        assert!(sol.attributes.quality_score > terra.attributes.quality_score);
        assert!(terra.attributes.quality_score > luna.attributes.quality_score);
        let sol_high = inventory
            .models
            .iter()
            .find(|model| model.provider_model_id == "gpt-5.6-sol:reasoning-high")
            .expect("GPT-5.6 Sol reasoning-high variant should exist");
        assert_eq!(
            sol_high.provider_actual_model_id.as_deref(),
            Some("gpt-5.6-sol")
        );
        assert_eq!(
            sol_high.provider_options,
            Some(json!({ "reasoning": { "effort": "high" } }))
        );
        assert_model_mount(
            &inventory,
            "gpt-5.6-sol:reasoning-high",
            "llm.gpt-pro.reasoning-high",
            true,
        );
        let terra_low = inventory
            .models
            .iter()
            .find(|model| model.provider_model_id == "gpt-5.6-terra:reasoning-low")
            .expect("GPT-5.6 Terra reasoning-low variant should exist");
        assert_eq!(
            terra_low.provider_options,
            Some(json!({ "reasoning": { "effort": "low" } }))
        );
        assert_model_mount(
            &inventory,
            "gpt-5.6-terra:reasoning-low",
            "llm.gpt-mini.reasoning-low",
            true,
        );
        assert_model_mount(
            &inventory,
            "gpt-5.6-luna:reasoning-high",
            "llm.gpt-nano.reasoning-high",
            true,
        );
        assert_model_mount(&inventory, "gpt-5.4", "llm", false);
        assert_model_mount(&inventory, "gpt-5.4", "llm.code", false);
        assert_model_mount(&inventory, "gpt-5.4", "llm.gpt-standard", false);
        assert_model_mount(&inventory, "gpt-5.4", "llm.openai.gpt-5-4", true);
        assert_model_mount(&inventory, "gpt-5.5-pro", "llm.plan", false);
        assert_model_mount(&inventory, "gpt-5.5-pro", "llm.reason", false);
        assert_model_mount(&inventory, "gpt-5.5-pro", "llm", false);
        assert_model_mount(&inventory, "gpt-5.5-pro", "llm.code", false);
        assert_model_mount(&inventory, "gpt-5.5-pro", "llm.gpt-pro", false);
        assert_model_mount(&inventory, "gpt-5.5-pro", "llm.openai.gpt-5-5-pro", true);
        assert_model_mount(&inventory, "gpt-5.4-pro", "llm.plan", false);
        assert_model_mount(&inventory, "gpt-5.4-pro", "llm.reason", false);
        assert_model_mount(&inventory, "gpt-5.4-mini", "llm", false);
        assert_model_mount(&inventory, "gpt-5.4-mini", "llm.code", false);
        assert_model_mount(&inventory, "gpt-5.4-mini", "llm.summarize", false);
        assert_model_mount(&inventory, "gpt-5.4-mini", "llm.gpt-mini", false);
        assert_model_mount(&inventory, "gpt-5-mini", "llm", false);
        assert_model_mount(&inventory, "gpt-5-mini", "llm.summarize", false);
        assert_model_mount(&inventory, "gpt-5.4-mini-2026-03-17", "llm", false);
        assert_model_mount(
            &inventory,
            "gpt-5.4-mini-2026-03-17",
            "llm.summarize",
            false,
        );
        assert_model_mount(
            &inventory,
            "gpt-5.4-mini-2026-03-17",
            "llm.openai.gpt-5-4-mini-2026-03-17",
            true,
        );
        assert_model_mount(&inventory, "gpt-5.4-mini-2026-03-17", "llm.gpt", false);
        assert_model_mount(&inventory, "gpt-5.4-nano", "llm.swift", false);
        assert_model_mount(&inventory, "gpt-5.4-nano", "llm", false);
        assert_model_mount(&inventory, "gpt-5.4-nano", "llm.gpt-nano", false);
        assert_model_mount(&inventory, "gpt-5-nano", "llm.swift", false);
        assert_model_mount(&inventory, "o1-2024-12-17", "llm.gpt", true);
        assert_model_mount(&inventory, "o1-2024-12-17", "llm", false);
        assert_model_mount(
            &inventory,
            "o1-2024-12-17",
            "llm.openai.o1-2024-12-17",
            true,
        );
    }

    #[test]
    fn provider_inventory_response_is_normalized_to_latest_gpt_family_mounts() {
        let provider = OpenAIProvider::new(
            OpenAIInstanceConfig {
                provider_instance_name: "openai-primary".to_string(),
                provider_type: "cloud_api".to_string(),
                provider_driver: "openai".to_string(),
                api_token: "token".to_string(),
                base_url: default_base_url(),
                timeout_ms: default_timeout_ms(),
            },
            "token",
        )
        .expect("provider should be built");

        let inventory = provider
            .build_inventory_from_remote_value(json!({
                "provider_instance_name": "remote-openai",
                "version": "1.0.0",
                "inventory_revision": "remote-r1",
                "models": [
                    {
                        "provider_model_id": "gpt-5.4",
                        "exact_model": "gpt-5.4@remote-openai",
                        "api_types": ["llm"],
                        "logical_mounts": ["llm.code", "llm.remote-general-old"]
                    },
                    {
                        "provider_model_id": "gpt-5.5",
                        "exact_model": "gpt-5.5@remote-openai",
                        "api_types": ["llm"],
                        "logical_mounts": ["llm"]
                    },
                    {
                        "provider_model_id": "gpt-5.4-pro",
                        "exact_model": "gpt-5.4-pro@remote-openai",
                        "api_types": ["llm"],
                        "logical_mounts": ["llm.gpt-pro", "llm.plan", "llm.remote-old"]
                    },
                    {
                        "provider_model_id": "gpt-5.5-pro",
                        "exact_model": "gpt-5.5-pro@remote-openai",
                        "api_types": ["llm"],
                        "logical_mounts": ["llm.gpt-pro"]
                    },
                    {
                        "provider_model_id": "gpt-5-mini",
                        "exact_model": "gpt-5-mini@remote-openai",
                        "api_types": ["llm"],
                        "logical_mounts": ["llm.summarize", "llm.remote-mini-old"]
                    },
                    {
                        "provider_model_id": "gpt-5.4-mini",
                        "exact_model": "gpt-5.4-mini@remote-openai",
                        "api_types": ["llm"],
                        "logical_mounts": ["llm.gpt-mini"]
                    },
                    {
                        "provider_model_id": "gpt-5.4-mini-2026-03-17",
                        "exact_model": "gpt-5.4-mini-2026-03-17@remote-openai",
                        "api_types": ["llm"],
                        "logical_mounts": ["llm"]
                    },
                    {
                        "provider_model_id": "o1-2024-12-17",
                        "exact_model": "o1-2024-12-17@remote-openai",
                        "api_types": ["llm"],
                        "logical_mounts": ["llm", "llm.remote-o1"]
                    }
                ]
            }))
            .expect("provider inventory response should parse");

        assert_eq!(inventory.provider_instance_name, "openai-primary");
        assert!(inventory
            .models
            .iter()
            .any(|model| model.exact_model == "gpt-5.5-pro@openai-primary"));
        assert_model_mount(&inventory, "gpt-5.5", "llm", false);
        assert_model_mount(&inventory, "gpt-5.5", "llm.code", false);
        assert_model_mount(&inventory, "gpt-5.5", "llm.gpt-standard", true);
        assert!(
            inventory
                .models
                .iter()
                .find(|model| model.provider_model_id == "gpt-5.5")
                .expect("model should exist")
                .capabilities
                .vision
        );
        assert_model_mount(&inventory, "gpt-5.4", "llm", false);
        assert_model_mount(&inventory, "gpt-5.4", "llm.code", false);
        assert_model_mount(&inventory, "gpt-5.4", "llm.gpt-standard", false);
        assert_model_mount(&inventory, "gpt-5.4", "llm.remote-general-old", false);
        assert_model_mount(&inventory, "gpt-5.5-pro", "llm.plan", false);
        assert_model_mount(&inventory, "gpt-5.5-pro", "llm.reason", false);
        assert_model_mount(&inventory, "gpt-5.5-pro", "llm.gpt-pro", true);
        assert_model_mount(&inventory, "gpt-5.5-pro", "llm.openai.gpt-5-5-pro", true);
        assert_model_mount(&inventory, "gpt-5.4-pro", "llm.plan", false);
        assert_model_mount(&inventory, "gpt-5.4-pro", "llm.reason", false);
        assert_model_mount(&inventory, "gpt-5.4-pro", "llm.gpt-pro", false);
        assert_model_mount(&inventory, "gpt-5.4-pro", "llm.openai.gpt-5-4-pro", true);
        assert_model_mount(&inventory, "gpt-5.4-pro", "llm.remote-old", false);
        assert_model_mount(&inventory, "gpt-5.4-mini", "llm", false);
        assert_model_mount(&inventory, "gpt-5.4-mini", "llm.summarize", false);
        assert_model_mount(&inventory, "gpt-5.4-mini", "llm.gpt-mini", true);
        assert_model_mount(&inventory, "gpt-5.4-mini-2026-03-17", "llm", false);
        assert_model_mount(
            &inventory,
            "gpt-5.4-mini-2026-03-17",
            "llm.summarize",
            false,
        );
        assert_model_mount(
            &inventory,
            "gpt-5.4-mini-2026-03-17",
            "llm.openai.gpt-5-4-mini-2026-03-17",
            true,
        );
        assert_model_mount(&inventory, "gpt-5.4-mini-2026-03-17", "llm.gpt", false);
        assert_model_mount(&inventory, "gpt-5-mini", "llm.summarize", false);
        assert_model_mount(&inventory, "gpt-5-mini", "llm.remote-mini-old", false);
        assert_model_mount(&inventory, "o1-2024-12-17", "llm.gpt", true);
        assert_model_mount(&inventory, "o1-2024-12-17", "llm", false);
        assert_model_mount(&inventory, "o1-2024-12-17", "llm.remote-o1", false);
    }

    #[test]
    fn remote_model_inventory_filters_supported_model_types() {
        let (llm_models, image_models, embedding_models, asr_models, tts_models) =
            normalize_remote_model_ids(
                vec![
                    OpenAIModelEntry {
                        id: "gpt-5.2".to_string(),
                    },
                    OpenAIModelEntry {
                        id: "text-embedding-3-large".to_string(),
                    },
                    OpenAIModelEntry {
                        id: "gpt-image-1".to_string(),
                    },
                    OpenAIModelEntry {
                        id: "gpt-image-2".to_string(),
                    },
                ],
                "openai",
            );

        assert_eq!(llm_models, vec!["gpt-5.2".to_string()]);
        assert_eq!(
            image_models,
            vec!["gpt-image-1".to_string(), "gpt-image-2".to_string()]
        );
        assert_eq!(embedding_models, vec!["text-embedding-3-large".to_string()]);
        assert!(asr_models.is_empty());
        assert!(tts_models.is_empty());
    }

    #[test]
    fn remote_model_inventory_classifies_audio_modalities() {
        let (llm_models, image_models, embedding_models, asr_models, tts_models) =
            normalize_remote_model_ids(
                vec![
                    OpenAIModelEntry {
                        id: "gpt-4o-mini-transcribe".to_string(),
                    },
                    OpenAIModelEntry {
                        id: "gpt-4o-transcribe".to_string(),
                    },
                    OpenAIModelEntry {
                        id: "gpt-4o-mini-tts".to_string(),
                    },
                    OpenAIModelEntry {
                        id: "tts-1-hd".to_string(),
                    },
                    OpenAIModelEntry {
                        id: "gpt-4o-audio-preview".to_string(),
                    },
                    OpenAIModelEntry {
                        id: "gpt-4o-realtime-preview".to_string(),
                    },
                    OpenAIModelEntry {
                        id: "gpt-live-transcribe".to_string(),
                    },
                    OpenAIModelEntry {
                        id: "gpt-realtime-mini".to_string(),
                    },
                    OpenAIModelEntry {
                        id: "gpt-5".to_string(),
                    },
                ],
                "openai",
            );

        assert_eq!(llm_models, vec!["gpt-5".to_string()]);
        assert!(image_models.is_empty());
        assert!(embedding_models.is_empty());
        assert_eq!(
            asr_models,
            vec![
                "gpt-4o-mini-transcribe".to_string(),
                "gpt-4o-transcribe".to_string(),
            ]
        );
        assert_eq!(
            tts_models,
            vec!["gpt-4o-mini-tts".to_string(), "tts-1-hd".to_string()]
        );
    }

    #[test]
    fn openrouter_inventory_preserves_provider_native_model_ids() {
        let (llm_models, image_models, embedding_models, asr_models, tts_models) =
            normalize_remote_model_ids(
                vec![
                    OpenAIModelEntry {
                        id: "openai/gpt-5.5".to_string(),
                    },
                    OpenAIModelEntry {
                        id: "anthropic/claude-sonnet-4".to_string(),
                    },
                    OpenAIModelEntry {
                        id: "tencent/hy3:free".to_string(),
                    },
                ],
                "openrouter",
            );

        assert_eq!(
            llm_models,
            vec![
                "openai/gpt-5.5".to_string(),
                "anthropic/claude-sonnet-4".to_string(),
                "tencent/hy3:free".to_string(),
            ]
        );
        assert!(image_models.is_empty());
        assert!(embedding_models.is_empty());
        assert!(asr_models.is_empty());
        assert!(tts_models.is_empty());
    }

    #[test]
    fn openrouter_remote_inventory_uses_strict_fixed_openai_whitelist() {
        let provider = OpenAIProvider::new(
            OpenAIInstanceConfig {
                provider_instance_name: "openrouter-main".to_string(),
                provider_type: "cloud_api".to_string(),
                provider_driver: "openrouter".to_string(),
                api_token: "token".to_string(),
                base_url: "https://openrouter.ai/api/v1".to_string(),
                timeout_ms: default_timeout_ms(),
            },
            "token",
        )
        .expect("provider should be built");

        let inventory = provider
            .build_inventory_from_remote_value(json!({
                "data": [
                    { "id": "openai/gpt-5.5" },
                    { "id": "anthropic/claude-sonnet-4" },
                    { "id": "openai/gpt-chat-latest" },
                    { "id": "openai/o3-mini-high" },
                    { "id": "openai/gpt-5.6-sol-pro" },
                    { "id": "openai/gpt-oss-20b:free" },
                    { "id": "openai/gpt-5.5:free" },
                    { "id": "openai/gpt-5.5:extended" },
                    { "id": "openai/gpt-5.5:thinking" },
                    { "id": "openai/gpt-5.5:nitro" },
                    { "id": "openai/gpt-5.5:floor" },
                    { "id": "openai/gpt-5.5:exacto" },
                    { "id": "openai/gpt-5.5:online" },
                    { "id": "openai/gpt-9-latest" }
                ]
            }))
            .expect("OpenRouter inventory should resolve");

        assert_eq!(inventory.models.len(), 3);
        let base = inventory
            .models
            .iter()
            .find(|model| model.provider_model_id == "openai/gpt-5.5")
            .expect("base model should resolve");
        assert_eq!(base.provider_actual_model_id, None);
        assert_eq!(base.origin_model_id.as_deref(), Some("gpt-5.5"));
        *provider.inventory.write().expect("inventory lock") = inventory.clone();
        let cost = provider
            .estimate_cost_for_usage(
                "openai/gpt-5.5",
                &AiUsage {
                    input_tokens: Some(1_000_000),
                    output_tokens: Some(1_000_000),
                    total_tokens: Some(2_000_000),
                    request_units: None,
                },
            )
            .expect("OpenRouter usage cost should resolve");
        assert_eq!(cost.amount, 35.0);
        assert!(inventory.models.iter().any(|model| {
            model.provider_model_id == "openai/gpt-5.5:reasoning-high"
                && model.provider_actual_model_id.as_deref() == Some("openai/gpt-5.5")
        }));
    }

    #[test]
    fn openrouter_cost_fallback_uses_metadata_origin_model_id() {
        let provider = OpenAIProvider::new(
            OpenAIInstanceConfig {
                provider_instance_name: "openrouter-main".to_string(),
                provider_type: "cloud_api".to_string(),
                provider_driver: "openrouter".to_string(),
                api_token: "token".to_string(),
                base_url: "https://openrouter.ai/api/v1".to_string(),
                timeout_ms: default_timeout_ms(),
            },
            "token",
        )
        .expect("provider should be built");

        let mut inventory = provider
            .build_inventory_from_remote_value(json!({
                "data": [{ "id": "openai/gpt-5.4-pro" }]
            }))
            .expect("OpenRouter inventory should resolve");
        let base = inventory
            .models
            .iter_mut()
            .find(|model| model.provider_model_id == "openai/gpt-5.4-pro")
            .expect("base model should resolve");
        assert_eq!(base.origin_model_id.as_deref(), Some("gpt-5.4-pro"));
        base.pricing.input_token = None;
        base.pricing.output_token = None;
        *provider.inventory.write().expect("inventory lock") = inventory;

        let cost = provider
            .estimate_cost_for_usage(
                "openai/gpt-5.4-pro",
                &AiUsage {
                    input_tokens: Some(1_000_000),
                    output_tokens: Some(1_000_000),
                    total_tokens: Some(2_000_000),
                    request_units: None,
                },
            )
            .expect("OpenRouter fallback cost should resolve");
        assert_eq!(cost.amount, 210.0);
    }

    #[test]
    fn responses_build_messages_keeps_user_image_blocks() {
        let request = AiMethodRequest::new(
            Capability::Llm,
            ModelSpec::new("llm.default".to_string(), None),
            Requirements::default(),
            AiPayload::new(
                None,
                vec![AiMessage::new(
                    AiRole::User,
                    vec![
                        AiContent::text("what is in this image?"),
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

        let provider = OpenAIProvider::new(
            OpenAIInstanceConfig {
                provider_instance_name: "openai-primary".to_string(),
                provider_type: "cloud_api".to_string(),
                provider_driver: "openai".to_string(),
                api_token: "token".to_string(),
                base_url: default_base_url(),
                timeout_ms: default_timeout_ms(),
            },
            "token",
        )
        .expect("provider should be built");
        let messages = provider.build_messages(&request).expect("messages");

        assert_eq!(
            messages[0]
                .pointer("/content/0/type")
                .and_then(|v| v.as_str()),
            Some("input_text")
        );
        assert_eq!(
            messages[0]
                .pointer("/content/1/type")
                .and_then(|v| v.as_str()),
            Some("input_image")
        );
        assert_eq!(
            messages[0]
                .pointer("/content/1/image_url")
                .and_then(|v| v.as_str()),
            Some("data:image/png;base64,aGVsbG8=")
        );
    }

    #[test]
    fn responses_build_messages_appends_canonical_document_resources() {
        let request = AiMethodRequest::new(
            Capability::Llm,
            ModelSpec::new("llm.default".to_string(), None),
            Requirements::default(),
            AiPayload::new(
                None,
                vec![AiMessage::text(AiRole::User, "read the document")],
                vec![],
                vec![ResourceRef::Base64 {
                    mime: "application/pdf".to_string(),
                    data_base64: "aGVsbG8=".to_string(),
                }],
                None,
                None,
            ),
            None,
        );
        let provider = OpenAIProvider::new(
            OpenAIInstanceConfig {
                provider_instance_name: "openai-primary".to_string(),
                provider_type: "cloud_api".to_string(),
                provider_driver: "openai".to_string(),
                api_token: "token".to_string(),
                base_url: default_base_url(),
                timeout_ms: default_timeout_ms(),
            },
            "token",
        )
        .expect("provider should be built");

        let messages = provider.build_messages(&request).expect("messages");

        assert_eq!(messages.len(), 2);
        assert_eq!(
            messages[1].pointer("/content/0/type"),
            Some(&json!("input_file"))
        );
        assert_eq!(
            messages[1].pointer("/content/0/filename"),
            Some(&json!("input.pdf"))
        );
        assert_eq!(
            messages[1].pointer("/content/0/file_data"),
            Some(&json!("data:application/pdf;base64,aGVsbG8="))
        );
    }

    #[test]
    fn responses_history_preserves_and_replays_provider_output_items() {
        let provider = OpenAIProvider::new(
            OpenAIInstanceConfig {
                provider_instance_name: "openrouter-main".to_string(),
                provider_type: "cloud_api".to_string(),
                provider_driver: "openrouter".to_string(),
                api_token: "token".to_string(),
                base_url: "https://openrouter.ai/api/v1".to_string(),
                timeout_ms: default_timeout_ms(),
            },
            "token",
        )
        .expect("provider should be built");
        let reasoning_item = json!({
            "type": "reasoning",
            "id": "rs_123",
            "summary": [],
            "encrypted_content": "opaque",
        });
        let message_item = json!({
            "type": "message",
            "id": "msg_123",
            "status": "completed",
            "role": "assistant",
            "content": [{
                "type": "output_text",
                "text": "hi",
                "annotations": [{"type": "url_citation", "url": "https://example.com"}],
            }],
        });
        let assistant = provider.message_from_response_body(
            &json!({"output": [reasoning_item.clone(), message_item.clone()]}),
            Some("hi".to_string()),
            vec![],
            vec![],
        );
        assert!(assistant.content.iter().all(|content| match content {
            AiContent::ProviderState { provider, .. } => provider == "openrouter",
            _ => true,
        }));
        let request = AiMethodRequest::new(
            Capability::Llm,
            ModelSpec::new("llm.default".to_string(), None),
            Requirements::default(),
            AiPayload::new(
                None,
                vec![AiMessage::text(AiRole::User, "hello"), assistant],
                vec![],
                vec![],
                None,
                None,
            ),
            None,
        );

        let messages = provider.build_messages(&request).expect("messages");

        assert_eq!(messages[1], reasoning_item);
        assert_eq!(messages[2], message_item);
    }

    #[test]
    fn responses_history_does_not_replay_foreign_provider_output_items() {
        let request = AiMethodRequest::new(
            Capability::Llm,
            ModelSpec::new("llm.default".to_string(), None),
            Requirements::default(),
            AiPayload::new(
                None,
                vec![AiMessage::new(
                    AiRole::Assistant,
                    vec![
                        AiContent::text("hi"),
                        AiContent::ProviderState {
                            provider: "openai".to_string(),
                            value: json!({
                                "type": "message",
                                "id": "msg_123",
                                "status": "completed",
                                "role": "assistant",
                                "content": [{
                                    "type": "output_text",
                                    "text": "hi",
                                    "annotations": [],
                                }],
                            }),
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
        let provider = OpenAIProvider::new(
            OpenAIInstanceConfig {
                provider_instance_name: "openrouter-main".to_string(),
                provider_type: "cloud_api".to_string(),
                provider_driver: "openrouter".to_string(),
                api_token: "token".to_string(),
                base_url: "https://openrouter.ai/api/v1".to_string(),
                timeout_ms: default_timeout_ms(),
            },
            "token",
        )
        .expect("provider should be built");

        let messages = provider
            .build_messages(&request)
            .expect("canonical assistant content should lower without foreign state");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["content"][0]["text"], "hi");
        assert_ne!(messages[0].get("id"), Some(&json!("msg_123")));
    }

    #[test]
    fn responses_history_lowers_assistant_without_provider_output_items() {
        let request = AiMethodRequest::new(
            Capability::Llm,
            ModelSpec::new("llm.default".to_string(), None),
            Requirements::default(),
            AiPayload::new(
                None,
                vec![AiMessage::text(AiRole::Assistant, "hi")],
                vec![],
                vec![],
                None,
                None,
            ),
            None,
        );

        let provider = OpenAIProvider::new(
            OpenAIInstanceConfig {
                provider_instance_name: "openai-main".to_string(),
                provider_type: "cloud_api".to_string(),
                provider_driver: "openai".to_string(),
                api_token: "token".to_string(),
                base_url: default_base_url(),
                timeout_ms: default_timeout_ms(),
            },
            "token",
        )
        .expect("provider should be built");
        let messages = provider
            .build_messages(&request)
            .expect("provider-neutral assistant history should lower");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "assistant");
        assert_eq!(messages[0]["content"][0]["type"], "output_text");
        assert_eq!(messages[0]["content"][0]["text"], "hi");
    }

    #[test]
    fn normalize_chat_completions_request_moves_text_and_max_tokens() {
        let mut request = json!({
            "model": "gpt-5.4",
            "messages": [{"role": "user", "content": "hello"}],
            "max_output_tokens": 320,
            "text": {
                "format": {
                    "type": "json_object"
                }
            }
        })
        .as_object()
        .cloned()
        .expect("request object");

        OpenAIProvider::normalize_chat_completions_request(&mut request);

        assert!(!request.contains_key("text"));
        assert!(!request.contains_key("max_output_tokens"));
        assert_eq!(request.get("max_tokens"), Some(&json!(320)));
        assert_eq!(
            request.get("response_format"),
            Some(&json!({
                "type": "json_object"
            }))
        );
    }

    #[test]
    fn normalize_chat_completions_request_converts_json_schema_shape() {
        let mut request = json!({
            "model": "gpt-5.4",
            "messages": [{"role": "user", "content": "hello"}],
            "text": {
                "format": {
                    "type": "json_schema",
                    "name": "plan_schema",
                    "schema": {
                        "type": "object",
                        "properties": {
                            "plan_id": {"type": "string"}
                        },
                        "required": ["plan_id"]
                    },
                    "strict": true
                }
            }
        })
        .as_object()
        .cloned()
        .expect("request object");

        OpenAIProvider::normalize_chat_completions_request(&mut request);
        let request_value = Value::Object(request);

        assert_eq!(
            request_value
                .pointer("/response_format/type")
                .and_then(|v| v.as_str()),
            Some("json_schema")
        );
        assert_eq!(
            request_value
                .pointer("/response_format/json_schema/name")
                .and_then(|v| v.as_str()),
            Some("plan_schema")
        );
        assert_eq!(
            request_value
                .pointer("/response_format/json_schema/strict")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            request_value
                .pointer("/response_format/json_schema/schema/required/0")
                .and_then(|v| v.as_str()),
            Some("plan_id")
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

        let llm =
            center
                .model_catalog()
                .resolve("", &Capability::Llm, "llm.plan.default", "openai");
        let image =
            center
                .model_catalog()
                .resolve("", &Capability::Image, "text2image.poster", "openai");
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

        let code_alias =
            center
                .model_catalog()
                .resolve("", &Capability::Llm, "llm.code.default", "openai");
        let removed_alias =
            center
                .model_catalog()
                .resolve("", &Capability::Llm, "llm.json.default", "openai");

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

    #[test]
    fn responses_image_request_nests_supported_options_in_tool() {
        let request = build_text2image_request(Some(json!({
            "size": "1024x1536",
            "quality": "high",
            "output_format": "png",
            "n": 2
        })));
        let (body, ignored) =
            OpenAIProvider::build_responses_image_request("gpt-5.6-sol", &request, "generate")
                .expect("request");
        let body = Value::Object(body);

        assert_eq!(body.get("input"), Some(&json!("draw a test image")));
        assert_eq!(
            body.pointer("/tools/0/type"),
            Some(&json!("image_generation"))
        );
        assert_eq!(body.pointer("/tools/0/action"), Some(&json!("generate")));
        assert_eq!(body.pointer("/tools/0/size"), Some(&json!("1024x1536")));
        assert_eq!(body.pointer("/tools/0/quality"), Some(&json!("high")));
        assert!(body.get("size").is_none());
        assert_eq!(ignored, vec!["n".to_string()]);
    }

    #[test]
    fn responses_image_edit_supports_multiple_materialized_images() {
        let resources = vec![
            ResourceRef::Url {
                url: "https://example.com/one.png".to_string(),
                mime_hint: Some("image/png".to_string()),
            },
            ResourceRef::Base64 {
                mime: "image/png".to_string(),
                data_base64: "iVBORw0KGgo=".to_string(),
            },
        ];
        let request = build_image_edit_request(
            resources,
            Some(json!({
                "input_fidelity": "high"
            })),
        );
        let (body, ignored) =
            OpenAIProvider::build_responses_image_request("gpt-5.6-terra", &request, "edit")
                .expect("request");
        let body = Value::Object(body);

        assert!(ignored.is_empty());
        assert_eq!(body.pointer("/tools/0/action"), Some(&json!("edit")));
        assert_eq!(
            body.pointer("/tools/0/input_fidelity"),
            Some(&json!("high"))
        );
        assert_eq!(
            body.pointer("/input/0/content/0/type"),
            Some(&json!("input_text"))
        );
        assert_eq!(
            body.pointer("/input/0/content/1/type"),
            Some(&json!("input_image"))
        );
        assert_eq!(
            body.pointer("/input/0/content/2/type"),
            Some(&json!("input_image"))
        );
    }

    #[test]
    fn responses_image_parser_preserves_multiple_outputs_and_provider_state() {
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let body = json!({
            "status": "completed",
            "output": [
                {"type": "image_generation_call", "id": "ig_1", "status": "completed", "action": "generate", "result": png},
                {"type": "image_generation_call", "id": "ig_2", "status": "completed", "action": "generate", "result": png}
            ]
        });
        let artifacts = OpenAIProvider::parse_responses_image_artifacts(&body).expect("artifacts");
        assert_eq!(artifacts.len(), 2);
        assert!(artifacts
            .iter()
            .all(|artifact| artifact.mime.as_deref() == Some("image/png")));

        let provider = OpenAIProvider::new(
            OpenAIInstanceConfig {
                provider_instance_name: "openai-history".to_string(),
                provider_type: "cloud_api".to_string(),
                provider_driver: "openai".to_string(),
                api_token: "token".to_string(),
                base_url: default_base_url(),
                timeout_ms: default_timeout_ms(),
            },
            "token",
        )
        .expect("provider");
        let message = provider.message_from_response_body(&body, None, vec![], artifacts);
        assert_eq!(
            message
                .content
                .iter()
                .filter(|content| matches!(content, AiContent::Image { .. }))
                .count(),
            2
        );
        assert_eq!(
            message
                .content
                .iter()
                .filter(|content| matches!(content, AiContent::ProviderState { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn responses_image_parser_rejects_invalid_or_incomplete_results() {
        for body in [
            json!({"output": [{"type": "image_generation_call", "status": "completed", "result": "not-base64"}]}),
            json!({"output": [{"type": "image_generation_call", "status": "in_progress", "result": "iVBORw0KGgo="}]}),
            json!({"output": [{"type": "message", "status": "completed"}]}),
        ] {
            assert!(OpenAIProvider::parse_responses_image_artifacts(&body).is_err());
        }
    }

    #[test]
    fn responses_image_stream_parser_uses_completed_response_payload() {
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let sse = format!(
            "event: response.completed\ndata: {}\n\ndata: [DONE]\n\n",
            json!({
                "type": "response.completed",
                "response": {
                    "id": "resp_stream",
                    "status": "completed",
                    "output": [{
                        "type": "image_generation_call",
                        "id": "ig_stream",
                        "status": "completed",
                        "result": png
                    }]
                }
            })
        );
        let response = OpenAIProvider::parse_sse_response_body(sse.as_str()).expect("SSE");
        let artifacts =
            OpenAIProvider::parse_responses_image_artifacts(&response).expect("artifacts");
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].mime.as_deref(), Some("image/png"));
    }

    #[test]
    fn image_protocol_selection_uses_resolved_metadata_capability() {
        let provider = OpenAIProvider::new(
            OpenAIInstanceConfig {
                provider_instance_name: "openai-capability".to_string(),
                provider_type: "cloud_api".to_string(),
                provider_driver: "openai".to_string(),
                api_token: "token".to_string(),
                base_url: default_base_url(),
                timeout_ms: default_timeout_ms(),
            },
            "token",
        )
        .expect("provider");
        assert!(provider.model_supports_responses_image_generation("gpt-5.6-sol"));
        assert!(!provider.model_supports_responses_image_generation("gpt-image-2"));
    }

    #[tokio::test]
    async fn gpt5_text2image_posts_to_responses_endpoint() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let address = listener.local_addr().expect("address");
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let response_body = json!({
            "id": "resp_1",
            "status": "completed",
            "output": [{
                "type": "image_generation_call",
                "id": "ig_1",
                "status": "completed",
                "action": "generate",
                "result": png
            }],
            "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}
        })
        .to_string();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut request = [0u8; 8192];
            let size = stream.read(&mut request).await.expect("read request");
            let request = String::from_utf8_lossy(&request[..size]).to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write response");
            request
        });
        let provider = OpenAIProvider::new(
            OpenAIInstanceConfig {
                provider_instance_name: "openai-responses-image".to_string(),
                provider_type: "cloud_api".to_string(),
                provider_driver: "openai".to_string(),
                api_token: "token".to_string(),
                base_url: format!("http://{}", address),
                timeout_ms: 1_000,
            },
            "token",
        )
        .expect("provider");
        let result = provider
            .start_text2image(
                &crate::aicc::InvokeCtx::default(),
                "gpt-5.6-sol",
                &build_text2image_request(None),
            )
            .await
            .expect("response");
        assert!(matches!(result, ProviderStartResult::Immediate(_)));
        let request = server.await.expect("server task");
        assert!(request.starts_with("POST /responses HTTP/1.1"), "{request}");
    }
}
