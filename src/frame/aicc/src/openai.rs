use crate::aicc::{
    exact_model_name, image_logical_mounts, logical_mount_segment, provider_model_metadata,
    provider_type_from_settings, AIComputeCenter, Provider, ProviderError, ProviderInstance,
    ProviderStartResult, ResolvedRequest, TaskEventSink,
};
use crate::model_types::{
    ApiType, CostEstimateInput, CostEstimateOutput, ModelMetadata, PricingMode, PrivacyClass,
    ProviderInventory, ProviderOrigin, ProviderType, ProviderTypeTrustedSource, QuotaState,
};
use crate::openai_protocol::{
    apply_provider_model_defaults, merge_options, merge_requirements_response_format,
    merge_tool_calls, strip_incompatible_sampling_options,
};
use ::kRPC::{RPCSessionToken, RPCSessionTokenType};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use base64::engine::general_purpose;
use base64::Engine as _;
use buckyos_api::{
    ai_methods, features, value_to_object_map, AiArtifact, AiCost, AiMethodRequest,
    AiResponseSummary, AiToolCall, AiUsage, Capability, ResourceRef,
};
use buckyos_kit::{buckyos_get_unix_timestamp, get_buckyos_system_etc_dir};
use log::{error, info, warn};
use name_lib::load_private_key;
use reqwest::header::{CONTENT_ENCODING, CONTENT_TYPE};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::error::Error as _;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::time;

const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_OPENAI_TIMEOUT_MS: u64 = 300_000;
const DEFAULT_OPENAI_MODELS: &str = "gpt-5,gpt-5-mini,gpt-5-nano,gpt-5-pro";
const DEFAULT_OPENAI_IMAGE_MODELS: &str = "gpt-image-1,dall-e-3,dall-e-2";
const DEFAULT_OPENAI_EMBEDDING_MODELS: &str = "text-embedding-3-large,text-embedding-3-small";
const DEFAULT_OPENAI_ASR_MODELS: &str = "gpt-4o-mini-transcribe,whisper-1";
const DEFAULT_OPENAI_TTS_MODELS: &str = "gpt-4o-mini-tts,tts-1";
const DEFAULT_SN_AI_PROVIDER_MODELS: &str = "gpt-5.4,gpt-5.4-mini,gpt-5.4-nano,gpt-5.4-pro";
const DEFAULT_SN_AI_PROVIDER_IMAGE_MODELS: &str = "gpt-image-1,dall-e-3,dall-e-2";
const DEFAULT_OPENAI_PROVIDER_DRIVER: &str = "openai";
const SN_AI_PROVIDER_DRIVER: &str = "sn-ai-provider";
const DEFAULT_INVENTORY_REFRESH_INTERVAL: Duration = Duration::from_secs(300);
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
    pub provider_instance_name: String,
    pub provider_type: String,
    pub base_url: String,
    pub auth_mode: String,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone)]
pub struct OpenAIProvider {
    instance: ProviderInstance,
    inventory: Arc<RwLock<ProviderInventory>>,
    client: Client,
    auth_mode: OpenAIAuthMode,
    base_url: String,
    provider_type: crate::model_types::ProviderType,
    provider_driver: String,
}

#[derive(Debug, Clone)]
enum OpenAIAuthMode {
    Bearer(String),
    DeviceJwt,
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
        let provider_driver = default_provider_driver_for_instance(
            cfg.provider_instance_name.as_str(),
            cfg.base_url.as_str(),
        );
        let instance = ProviderInstance {
            provider_instance_name: provider_instance_name.clone(),
            provider_type: provider_type.clone(),
            provider_driver: provider_driver.clone(),
            provider_origin: ProviderOrigin::SystemConfig,
            provider_type_trusted_source: ProviderTypeTrustedSource::SystemConfig,
            provider_type_revision: None,
            capabilities: vec![Capability::Llm, Capability::Image],
            features: default_features(),
            endpoint: Some(cfg.base_url.clone()),
            plugin_key: None,
        };
        let inventory = Self::default_inventory(
            provider_instance_name.as_str(),
            provider_type.clone(),
            provider_driver.as_str(),
        );

        let auth_mode = Self::parse_auth_mode(cfg.auth_mode.as_str(), openai_api_token)?;

        Ok(Self {
            instance,
            inventory: Arc::new(RwLock::new(inventory)),
            client,
            auth_mode,
            base_url: cfg.base_url.trim_end_matches('/').to_string(),
            provider_type,
            provider_driver,
        })
    }

    pub fn start_inventory_refresh(self: Arc<Self>) {
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }

        tokio::spawn(async move {
            if let Err(err) = self.refresh_inventory_once().await {
                warn!(
                    "aicc.openai.inventory.initial_refresh_failed provider_instance_name={} err={}",
                    self.instance.provider_instance_name, err
                );
            }

            let mut interval = time::interval(DEFAULT_INVENTORY_REFRESH_INTERVAL);
            interval.tick().await;
            loop {
                interval.tick().await;
                if let Err(err) = self.refresh_inventory_once().await {
                    warn!(
                        "aicc.openai.inventory.refresh_failed provider_instance_name={} err={}",
                        self.instance.provider_instance_name, err
                    );
                }
            }
        });
    }

    async fn refresh_inventory_once(&self) -> Result<ProviderInventory> {
        let endpoint = self.models_endpoint();
        let token = self.build_inventory_auth_token()?;
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
        let (models, image_models) = if provider_driver == SN_AI_PROVIDER_DRIVER {
            (
                normalize_model_list(parse_csv_list(DEFAULT_SN_AI_PROVIDER_MODELS)),
                normalize_model_list(parse_csv_list(DEFAULT_SN_AI_PROVIDER_IMAGE_MODELS)),
            )
        } else {
            (
                normalize_model_list(parse_csv_list(DEFAULT_OPENAI_MODELS)),
                normalize_model_list(parse_csv_list(DEFAULT_OPENAI_IMAGE_MODELS)),
            )
        };
        let embedding_models =
            normalize_model_list(parse_csv_list(DEFAULT_OPENAI_EMBEDDING_MODELS));
        let asr_models = normalize_model_list(parse_csv_list(DEFAULT_OPENAI_ASR_MODELS));
        let tts_models = normalize_model_list(parse_csv_list(DEFAULT_OPENAI_TTS_MODELS));

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
        revision: Option<String>,
    ) -> ProviderInventory {
        Self::build_inventory(
            self.instance.provider_instance_name.as_str(),
            self.provider_type.clone(),
            self.provider_driver.as_str(),
            models,
            image_models,
            normalize_model_list(parse_csv_list(DEFAULT_OPENAI_EMBEDDING_MODELS)).as_slice(),
            normalize_model_list(parse_csv_list(DEFAULT_OPENAI_ASR_MODELS)).as_slice(),
            normalize_model_list(parse_csv_list(DEFAULT_OPENAI_TTS_MODELS)).as_slice(),
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
        let (llm_models, image_models) = normalize_remote_model_ids(response.data);
        if llm_models.is_empty() && image_models.is_empty() {
            return Err(anyhow!(
                "openai inventory refresh returned no supported models"
            ));
        }

        Ok(self.build_inventory_from_models(
            llm_models.as_slice(),
            image_models.as_slice(),
            Some(inventory_revision(
                llm_models.as_slice(),
                image_models.as_slice(),
            )),
        ))
    }

    fn normalize_remote_provider_inventory(
        &self,
        inventory: ProviderInventory,
    ) -> ProviderInventory {
        let version = inventory.version.clone();
        let mut models = inventory
            .models
            .into_iter()
            .filter_map(|model| {
                normalize_remote_provider_model(
                    model,
                    self.instance.provider_instance_name.as_str(),
                    self.provider_type.clone(),
                    self.provider_driver.as_str(),
                )
            })
            .collect::<Vec<_>>();
        apply_openai_latest_llm_mounts(self.provider_driver.as_str(), &mut models);

        ProviderInventory {
            provider_instance_name: self.instance.provider_instance_name.clone(),
            provider_type: self.provider_type.clone(),
            provider_driver: self.provider_driver.clone(),
            provider_origin: ProviderOrigin::SystemConfig,
            provider_type_trusted_source: ProviderTypeTrustedSource::SystemConfig,
            provider_type_revision: None,
            version: version.clone(),
            inventory_revision: inventory.inventory_revision.or_else(|| {
                Some(inventory_revision_from_metadata(
                    models.as_slice(),
                    version.as_deref(),
                ))
            }),
            models,
        }
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
        let features = default_features();
        let mut metadata = Vec::new();
        for model in models
            .iter()
            .filter(|model| !is_text2image_model_name(model))
        {
            let mut model_metadata = provider_model_metadata(
                provider_instance_name,
                provider_type.clone(),
                model.as_str(),
                ApiType::LlmChat,
                openai_llm_logical_mounts(provider_driver, model.as_str()),
                features.as_slice(),
                Some(0.01),
                Some(1200),
            );
            add_unique_api_type(&mut model_metadata.api_types, ApiType::LlmCompletion);
            add_unique_api_type(&mut model_metadata.api_types, ApiType::Rerank);
            add_unique_mount(&mut model_metadata.logical_mounts, "rerank".to_string());
            add_unique_mount(
                &mut model_metadata.logical_mounts,
                "rerank.openai".to_string(),
            );
            metadata.push(model_metadata);
        }
        for model in image_models.iter() {
            let mut api_types = vec![ApiType::ImageTextToImage];
            let mut mounts = image_logical_mounts(provider_driver, model.as_str());
            if supports_openai_image_edit(model.as_str()) {
                api_types.push(ApiType::ImageToImage);
                api_types.push(ApiType::ImageInpaint);
                mounts.extend(openai_method_mounts(
                    ApiType::ImageToImage,
                    provider_driver,
                    model,
                ));
                mounts.extend(openai_method_mounts(
                    ApiType::ImageInpaint,
                    provider_driver,
                    model,
                ));
            }
            metadata.push(provider_model_metadata_multi(
                provider_instance_name,
                provider_type.clone(),
                model.as_str(),
                api_types,
                dedupe_strings(mounts),
                features.as_slice(),
                Some(0.04),
                Some(5000),
            ));
        }
        for model in embedding_models.iter() {
            metadata.push(provider_model_metadata_multi(
                provider_instance_name,
                provider_type.clone(),
                model.as_str(),
                vec![ApiType::Embedding],
                openai_method_mounts(ApiType::Embedding, provider_driver, model),
                features.as_slice(),
                Some(0.0001),
                Some(800),
            ));
        }
        for model in asr_models.iter() {
            metadata.push(provider_model_metadata_multi(
                provider_instance_name,
                provider_type.clone(),
                model.as_str(),
                vec![ApiType::AudioAsr],
                openai_method_mounts(ApiType::AudioAsr, provider_driver, model),
                features.as_slice(),
                Some(0.006),
                Some(5000),
            ));
        }
        for model in tts_models.iter() {
            metadata.push(provider_model_metadata_multi(
                provider_instance_name,
                provider_type.clone(),
                model.as_str(),
                vec![ApiType::AudioTts],
                openai_method_mounts(ApiType::AudioTts, provider_driver, model),
                features.as_slice(),
                Some(0.015),
                Some(3000),
            ));
        }
        apply_openai_latest_llm_mounts(provider_driver, &mut metadata);

        ProviderInventory {
            provider_instance_name: provider_instance_name.to_string(),
            provider_type,
            provider_driver: provider_driver.to_string(),
            provider_origin: ProviderOrigin::SystemConfig,
            provider_type_trusted_source: ProviderTypeTrustedSource::SystemConfig,
            provider_type_revision: None,
            version: None,
            inventory_revision: revision,
            models: metadata,
        }
    }

    fn parse_auth_mode(auth_mode: &str, openai_api_token: &str) -> Result<OpenAIAuthMode> {
        let mode = auth_mode.trim().to_ascii_lowercase();
        if mode.is_empty() || mode == DEFAULT_AUTH_MODE {
            let token = Some(openai_api_token)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow!("openai bearer auth requires non-empty api_token"))?;
            return Ok(OpenAIAuthMode::Bearer(token));
        }

        if mode == DEVICE_JWT_AUTH_MODE {
            return Ok(OpenAIAuthMode::DeviceJwt);
        }

        Err(anyhow!(
            "unsupported openai auth_mode '{}', expected '{}' or '{}'",
            auth_mode,
            DEFAULT_AUTH_MODE,
            DEVICE_JWT_AUTH_MODE
        ))
    }

    fn build_inventory_auth_token(&self) -> Result<String> {
        match &self.auth_mode {
            OpenAIAuthMode::Bearer(token) => Ok(token.clone()),
            OpenAIAuthMode::DeviceJwt => {
                let private_key_path = Self::resolve_private_key_path();
                let private_key = load_private_key(private_key_path.as_path()).map_err(|err| {
                    anyhow!(
                        "openai device_jwt inventory auth failed to load private key '{}': {}",
                        private_key_path.display(),
                        err
                    )
                })?;
                let now = buckyos_get_unix_timestamp();
                let subject = Self::read_default_device_subject();
                let claims = RPCSessionToken {
                    token_type: RPCSessionTokenType::JWT,
                    token: None,
                    aud: None,
                    exp: Some(now + 60 * 15),
                    iss: Some(subject.clone()),
                    jti: None,
                    session: None,
                    sub: Some(subject),
                    appid: Some(DEFAULT_DEVICE_AUTH_APP_ID.to_string()),
                    extra: HashMap::new(),
                };
                claims
                    .generate_jwt(None, &private_key)
                    .map_err(|err| anyhow!("openai device_jwt inventory auth failed: {}", err))
            }
        }
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

    fn resolve_device_subject(ctx: &crate::aicc::InvokeCtx) -> String {
        let subject = ctx.tenant_id.trim();
        if !subject.is_empty() && subject != "anonymous" {
            return subject.to_string();
        }
        Self::read_default_device_subject()
    }

    fn resolve_device_appid(ctx: &crate::aicc::InvokeCtx) -> String {
        ctx.caller_app_id
            .as_ref()
            .map(|appid| appid.trim().to_string())
            .filter(|appid| !appid.is_empty())
            .unwrap_or_else(|| DEFAULT_DEVICE_AUTH_APP_ID.to_string())
    }

    fn resolve_private_key_path() -> PathBuf {
        get_buckyos_system_etc_dir().join("node_private_key.pem")
    }

    fn build_auth_token(&self, ctx: &crate::aicc::InvokeCtx) -> Result<String, ProviderError> {
        match &self.auth_mode {
            OpenAIAuthMode::Bearer(token) => Ok(token.clone()),
            OpenAIAuthMode::DeviceJwt => {
                let private_key_path = Self::resolve_private_key_path();
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
                    sub: Some(Self::resolve_device_subject(ctx)),
                    appid: Some(Self::resolve_device_appid(ctx)),
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
        if model.starts_with("gpt-5.4-pro") {
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

    fn estimate_tokens(req: &AiMethodRequest) -> (u64, u64) {
        let mut text_len = 0usize;

        if let Some(text) = req.payload.text.as_ref() {
            text_len += text.len();
        }

        for message in req.payload.messages.iter() {
            text_len += message.content.len();
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
                let role = msg.role.trim();
                let content = msg.content.trim();
                (!role.is_empty() && !content.is_empty())
                    .then(|| (role.to_string(), content.to_string()))
            })
            .collect())
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

    fn build_messages(&self, req: &AiMethodRequest) -> Result<Vec<Value>, ProviderError> {
        let mut messages = vec![];

        for (role, content) in Self::canonical_message_texts(req)? {
            messages.push(json!({
                "role": role,
                "content": [
                    {
                        "type": "input_text",
                        "text": content
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

    fn build_chat_messages(req: &AiMethodRequest) -> Result<Vec<Value>, ProviderError> {
        let mut messages = vec![];

        for (role, content) in Self::canonical_message_texts(req)? {
            messages.push(json!({
                "role": role,
                "content": content,
            }));
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
            .map(|msg| msg.content.trim())
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
        ctx: &crate::aicc::InvokeCtx,
        url: &str,
        request_obj: &Map<String, Value>,
    ) -> Result<(StatusCode, Value, u64), ProviderError> {
        let auth_token = self.build_auth_token(ctx)?;
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
        let auth_token = self.build_auth_token(ctx)?;
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
                let response = self.client.get(url).send().await.map_err(|err| {
                    if err.is_timeout() || err.is_connect() {
                        ProviderError::retryable(format!("failed to fetch resource url: {}", err))
                    } else {
                        ProviderError::fatal(format!("failed to fetch resource url: {}", err))
                    }
                })?;
                let status = response.status();
                if !status.is_success() {
                    return Err(Self::classify_api_error(
                        status,
                        format!("resource url returned status {}", status.as_u16()),
                    ));
                }
                let content_type = response
                    .headers()
                    .get(CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .map(|value| value.to_string())
                    .or_else(|| mime_hint.clone())
                    .unwrap_or_else(|| "application/octet-stream".to_string());
                let bytes = response.bytes().await.map_err(|err| {
                    ProviderError::fatal(format!("failed to read resource bytes: {}", err))
                })?;
                Ok((fallback_name.to_string(), content_type, bytes.to_vec()))
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

        let auth_token = self.build_auth_token(ctx)?;
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
                self.instance.provider_instance_name, provider_model, ctx.trace_id, stripped_options
            );
        }
        merge_requirements_response_format(&mut request_obj, req);
        merge_tool_calls(&mut request_obj, req.payload.tool_specs.as_slice())?;
        Self::merge_requirements_tools(&mut request_obj, req)?;
        if self.use_chat_completions_endpoint() {
            Self::normalize_chat_completions_request(&mut request_obj);
        }
        if !ignored_options.is_empty() {
            warn!(
                "aicc.openai ignored unsupported llm options: provider_instance_name={} model={} trace_id={:?} ignored={:?}",
                self.instance.provider_instance_name, provider_model, ctx.trace_id, ignored_options
            );
        }

        let request_log = Value::Object(request_obj.clone()).to_string();
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
        req: &AiMethodRequest,
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
                "aicc.openai ignored unsupported text2image options: provider_instance_name={} model={} trace_id={:?} ignored={:?}",
                self.instance.provider_instance_name, provider_model, ctx.trace_id, ignored_options
            );
        }

        let request_log = Value::Object(request_obj.clone()).to_string();
        info!(
            "aicc.openai.text2image.input provider_instance_name={} model={} trace_id={:?} request={}",
            self.instance.provider_instance_name, provider_model, ctx.trace_id, request_log
        );

        let url = format!("{}/images/generations", self.base_url);
        let (status, body, latency_ms) = self.post_json(ctx, url.as_str(), &request_obj).await?;
        let response_log = body.to_string();

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

    fn embedding_inputs(req: &AiMethodRequest) -> Result<Value, ProviderError> {
        if let Some(items) = req
            .payload
            .input_json
            .as_ref()
            .and_then(|value| value.get("items"))
            .cloned()
        {
            if let Some(array) = items.as_array() {
                let texts = array
                    .iter()
                    .filter_map(|item| {
                        item.get("text")
                            .and_then(|value| value.as_str())
                            .map(|value| value.to_string())
                            .or_else(|| item.as_str().map(|value| value.to_string()))
                    })
                    .collect::<Vec<_>>();
                if !texts.is_empty() {
                    return Ok(Value::Array(texts.into_iter().map(Value::String).collect()));
                }
            }
        }
        if let Some(text) = req.payload.text.as_ref().map(String::as_str) {
            return Ok(Value::String(text.to_string()));
        }
        let texts = req
            .payload
            .messages
            .iter()
            .map(|msg| msg.content.trim())
            .filter(|value| !value.is_empty())
            .map(|value| Value::String(value.to_string()))
            .collect::<Vec<_>>();
        if !texts.is_empty() {
            return Ok(Value::Array(texts));
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
        let mut request_obj = Map::new();
        request_obj.insert(
            "model".to_string(),
            Value::String(provider_model.to_string()),
        );
        request_obj.insert("input".to_string(), Self::embedding_inputs(req)?);
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
        let embedding_space_id = format!(
            "openai:{}:{}",
            provider_model,
            dimensions
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        );
        let mut extra = Map::new();
        extra.insert(
            "embedding".to_string(),
            json!({
                "data": body.get("data").cloned().unwrap_or_else(|| Value::Array(vec![])),
                "embedding_space_id": embedding_space_id,
                "provider_io": {
                    "input": Value::Object(request_obj.clone()),
                    "output": body.clone()
                },
                "latency_ms": latency_ms
            }),
        );
        Ok(ProviderStartResult::Immediate(AiResponseSummary {
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
                                    "results": { "type": "array" }
                                },
                                "required": ["results"],
                                "additionalProperties": true
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
            let rerank_value = summary
                .text
                .as_ref()
                .and_then(|text| serde_json::from_str::<Value>(text).ok())
                .unwrap_or_else(|| json!({ "raw_text": summary.text }));
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
            .map(|mime| if mime.contains("wav") { "wav" } else { "mp3" })
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
        } else if response_format == "wav" {
            "audio/wav".to_string()
        } else {
            "audio/mpeg".to_string()
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
        Ok(ProviderStartResult::Immediate(AiResponseSummary {
            artifacts: vec![artifact],
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
        let resource =
            req.payload.resources.first().ok_or_else(|| {
                ProviderError::fatal("audio.asr requires resources[0] audio input")
            })?;
        let (filename, mime, bytes) = self.resource_to_file_bytes(resource, "audio").await?;
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
        Ok(ProviderStartResult::Immediate(AiResponseSummary {
            text,
            finish_reason: Some("stop".to_string()),
            extra: Some(Value::Object(extra)),
            ..Default::default()
        }))
    }

    async fn start_image_edit(
        &self,
        ctx: &crate::aicc::InvokeCtx,
        provider_model: &str,
        req: &AiMethodRequest,
        with_mask: bool,
    ) -> Result<ProviderStartResult, ProviderError> {
        let image_resource = req.payload.resources.first().ok_or_else(|| {
            ProviderError::fatal("image edit requires resources[0] source image input")
        })?;
        let (image_name, image_mime, image_bytes) = self
            .resource_to_file_bytes(image_resource, "image.png")
            .await?;
        let mut files = vec![("image".to_string(), image_name, image_mime, image_bytes)];
        if with_mask {
            let mask_resource = req.payload.resources.get(1).ok_or_else(|| {
                ProviderError::fatal("image.inpaint requires resources[1] mask input")
            })?;
            let (mask_name, mask_mime, mask_bytes) = self
                .resource_to_file_bytes(mask_resource, "mask.png")
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
                    if OPENAI_IMAGE_OPTION_ALLOWLIST.contains(&key.as_str()) {
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
        Ok(ProviderStartResult::Immediate(AiResponseSummary {
            artifacts,
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

        let input_tokens = input.input_tokens.max(1);
        let output_tokens = input.estimated_output_tokens.unwrap_or(1024).max(1);
        let usage = AiUsage {
            input_tokens: Some(input_tokens),
            output_tokens: Some(output_tokens),
            total_tokens: Some(input_tokens.saturating_add(output_tokens)),
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

    async fn start(
        &self,
        ctx: crate::aicc::InvokeCtx,
        provider_model: String,
        req: ResolvedRequest,
        _sink: Arc<dyn TaskEventSink>,
    ) -> std::result::Result<ProviderStartResult, ProviderError> {
        match req.method.as_str() {
            ai_methods::LLM_CHAT | ai_methods::LLM_COMPLETION => {
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
        Ok(())
    }
}

fn provider_model_from_exact(exact_model: &str) -> &str {
    exact_model
        .rsplit_once('@')
        .map(|(model, _)| model)
        .unwrap_or(exact_model)
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum OpenAIGptTier {
    Pro,
    General,
    Mini,
    Nano,
}

impl OpenAIGptTier {
    fn logical_mount(self) -> &'static str {
        match self {
            Self::Pro => "llm.gpt-pro",
            Self::General => "llm.gpt",
            Self::Mini => "llm.gpt-mini",
            Self::Nano => "llm.gpt-nano",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GptModelRank {
    version: Vec<u64>,
    stable: bool,
    model_id: String,
}

fn normalize_remote_provider_model(
    mut model: ModelMetadata,
    provider_instance_name: &str,
    provider_type: ProviderType,
    provider_driver: &str,
) -> Option<ModelMetadata> {
    let provider_model_id = model.provider_model_id.trim().to_string();
    let provider_model_id = if provider_model_id.is_empty() {
        provider_model_from_exact(model.exact_model.as_str())
            .trim()
            .to_string()
    } else {
        provider_model_id
    };
    if provider_model_id.is_empty() {
        return None;
    }

    let mut api_types = model
        .api_types
        .into_iter()
        .filter(is_supported_openai_api_type)
        .collect::<Vec<_>>();
    if api_types.is_empty() {
        if is_text2image_model_name(provider_model_id.as_str()) {
            api_types.push(ApiType::ImageTextToImage);
        } else if is_supported_llm_model_name(provider_model_id.as_str()) {
            api_types.push(ApiType::LlmChat);
        } else {
            return None;
        }
    }

    let mut logical_mounts = model
        .logical_mounts
        .into_iter()
        .filter(|mount| !is_openai_gpt_tier_mount(mount.as_str()))
        .collect::<Vec<_>>();
    if api_types.iter().any(is_llm_api_type) {
        for mount in openai_llm_logical_mounts(provider_driver, provider_model_id.as_str()) {
            add_unique_mount(&mut logical_mounts, mount);
        }
    }
    if api_types
        .iter()
        .any(|api_type| api_type == &ApiType::ImageTextToImage)
    {
        for mount in image_logical_mounts(provider_driver, provider_model_id.as_str()) {
            add_unique_mount(&mut logical_mounts, mount);
        }
    }
    for api_type in api_types.iter() {
        if matches!(
            api_type,
            ApiType::Embedding
                | ApiType::Rerank
                | ApiType::ImageToImage
                | ApiType::ImageInpaint
                | ApiType::AudioAsr
                | ApiType::AudioTts
        ) {
            for mount in openai_method_mounts(
                api_type.clone(),
                provider_driver,
                provider_model_id.as_str(),
            ) {
                add_unique_mount(&mut logical_mounts, mount);
            }
        }
    }

    model.provider_model_id = provider_model_id.clone();
    model.exact_model = exact_model_name(provider_model_id.as_str(), provider_instance_name);
    model.api_types = api_types;
    model.logical_mounts = logical_mounts;
    model.attributes.provider_type = provider_type.clone();
    model.attributes.local = provider_type == ProviderType::LocalInference;
    model.attributes.privacy = if provider_type == ProviderType::LocalInference {
        PrivacyClass::Local
    } else {
        PrivacyClass::Cloud
    };
    Some(model)
}

fn is_supported_openai_api_type(api_type: &ApiType) -> bool {
    matches!(
        api_type,
        ApiType::LlmChat
            | ApiType::LlmCompletion
            | ApiType::Embedding
            | ApiType::Rerank
            | ApiType::ImageTextToImage
            | ApiType::ImageToImage
            | ApiType::ImageInpaint
            | ApiType::AudioAsr
            | ApiType::AudioTts
    )
}

fn is_llm_api_type(api_type: &ApiType) -> bool {
    matches!(api_type, ApiType::LlmChat | ApiType::LlmCompletion)
}

fn openai_llm_logical_mounts(provider_driver: &str, provider_model_id: &str) -> Vec<String> {
    vec![
        format!("llm.{}", logical_mount_segment(provider_driver)),
        format!("llm.openai.{}", logical_mount_segment(provider_model_id)),
    ]
}

fn apply_openai_latest_llm_mounts(_provider_driver: &str, models: &mut [ModelMetadata]) {
    let mut latest = HashMap::<OpenAIGptTier, (usize, GptModelRank)>::new();

    for (index, model) in models.iter_mut().enumerate() {
        remove_openai_gpt_tier_mounts(&mut model.logical_mounts);
        if !model.api_types.iter().any(is_llm_api_type) {
            continue;
        }
        let Some((tier, rank)) = classify_openai_gpt_model(model.provider_model_id.as_str()) else {
            continue;
        };

        let replace = latest
            .get(&tier)
            .map(|(_, current)| compare_gpt_model_rank(&rank, current) == Ordering::Greater)
            .unwrap_or(true);
        if replace {
            latest.insert(tier, (index, rank));
        }
    }

    for (tier, (index, _)) in latest {
        let model = &mut models[index];
        add_unique_mount(&mut model.logical_mounts, tier.logical_mount().to_string());
    }
}

fn classify_openai_gpt_model(provider_model_id: &str) -> Option<(OpenAIGptTier, GptModelRank)> {
    if is_text2image_model_name(provider_model_id) {
        return None;
    }

    let normalized = provider_model_id
        .trim()
        .to_ascii_lowercase()
        .replace('_', "-");
    if !normalized.contains("gpt") {
        return None;
    }

    let tokens = normalized
        .split(|ch: char| ch == '-' || ch == '.' || ch == '/')
        .filter(|token| !token.is_empty())
        .map(|token| token.to_string())
        .collect::<HashSet<_>>();
    let tier = if tokens.contains("pro") {
        OpenAIGptTier::Pro
    } else if tokens.contains("mini") {
        OpenAIGptTier::Mini
    } else if tokens.contains("nano") || tokens.contains("nono") {
        OpenAIGptTier::Nano
    } else {
        OpenAIGptTier::General
    };

    Some((
        tier,
        GptModelRank {
            version: parse_gpt_version(normalized.as_str()),
            stable: !tokens.contains("preview")
                && !tokens.contains("experimental")
                && !tokens.contains("beta"),
            model_id: normalized,
        },
    ))
}

fn parse_gpt_version(normalized_model_id: &str) -> Vec<u64> {
    let Some(gpt_pos) = normalized_model_id.find("gpt") else {
        return Vec::new();
    };
    let mut chars = normalized_model_id[gpt_pos + "gpt".len()..]
        .trim_start_matches('-')
        .chars()
        .peekable();
    let mut version = Vec::new();

    loop {
        let mut value = String::new();
        while let Some(ch) = chars.peek().copied() {
            if ch.is_ascii_digit() {
                value.push(ch);
                chars.next();
            } else {
                break;
            }
        }
        if value.is_empty() {
            break;
        }
        if let Ok(parsed) = value.parse::<u64>() {
            version.push(parsed);
        }

        if chars.peek().copied() == Some('.') {
            chars.next();
            continue;
        }
        break;
    }

    version
}

fn compare_gpt_model_rank(left: &GptModelRank, right: &GptModelRank) -> Ordering {
    let max_len = left.version.len().max(right.version.len());
    for index in 0..max_len {
        let left_value = left.version.get(index).copied().unwrap_or(0);
        let right_value = right.version.get(index).copied().unwrap_or(0);
        match left_value.cmp(&right_value) {
            Ordering::Equal => {}
            ordering => return ordering,
        }
    }

    left.stable
        .cmp(&right.stable)
        .then_with(|| left.model_id.cmp(&right.model_id))
}

fn remove_openai_gpt_tier_mounts(mounts: &mut Vec<String>) {
    mounts.retain(|mount| !is_openai_gpt_tier_mount(mount.as_str()));
}

fn is_openai_gpt_tier_mount(mount: &str) -> bool {
    matches!(
        mount,
        "llm.gpt" | "llm.gpt-pro" | "llm.gpt-mini" | "llm.gpt-nano"
    )
}

fn add_unique_mount(mounts: &mut Vec<String>, mount: String) {
    if !mounts.iter().any(|item| item == &mount) {
        mounts.push(mount);
    }
}

fn add_unique_api_type(api_types: &mut Vec<ApiType>, api_type: ApiType) {
    if !api_types.iter().any(|item| item == &api_type) {
        api_types.push(api_type);
    }
}

fn dedupe_strings(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::<String>::new();
    let mut deduped = Vec::new();
    for value in values.into_iter() {
        if seen.insert(value.clone()) {
            deduped.push(value);
        }
    }
    deduped
}

fn provider_model_metadata_multi(
    provider_instance_name: &str,
    provider_type: ProviderType,
    provider_model_id: &str,
    api_types: Vec<ApiType>,
    logical_mounts: Vec<String>,
    features: &[buckyos_api::Feature],
    estimated_cost_usd: Option<f64>,
    estimated_latency_ms: Option<u64>,
) -> ModelMetadata {
    let first_api_type = api_types.first().cloned().unwrap_or(ApiType::LlmChat);
    let mut metadata = provider_model_metadata(
        provider_instance_name,
        provider_type,
        provider_model_id,
        first_api_type,
        logical_mounts,
        features,
        estimated_cost_usd,
        estimated_latency_ms,
    );
    metadata.api_types = api_types;
    metadata
}

fn openai_method_mounts(
    api_type: ApiType,
    provider_driver: &str,
    provider_model_id: &str,
) -> Vec<String> {
    let base = match api_type {
        ApiType::Embedding => "embedding.text",
        ApiType::Rerank => "rerank",
        ApiType::ImageToImage => "image.img2img",
        ApiType::ImageInpaint => "image.inpaint",
        ApiType::AudioAsr => "audio.asr",
        ApiType::AudioTts => "audio.tts",
        _ => api_type.namespace(),
    };
    let driver = logical_mount_segment(provider_driver);
    let model = logical_mount_segment(provider_model_id);
    vec![
        base.to_string(),
        format!("{}.{}", base, driver),
        format!("{}.{}", base, model),
    ]
}

fn value_to_form_field(value: &Value) -> String {
    value
        .as_str()
        .map(|value| value.to_string())
        .unwrap_or_else(|| value.to_string())
}

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
    #[serde(default = "default_base_url")]
    base_url: String,
    #[serde(default = "default_auth_mode")]
    auth_mode: String,
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

fn default_auth_mode() -> String {
    DEFAULT_AUTH_MODE.to_string()
}

fn default_provider_driver_for_instance(provider_instance_name: &str, base_url: &str) -> String {
    let instance = provider_instance_name.to_ascii_lowercase();
    let endpoint = base_url.to_ascii_lowercase();
    if instance.contains(SN_AI_PROVIDER_DRIVER) || endpoint.contains("sn.buckyos.ai") {
        SN_AI_PROVIDER_DRIVER.to_string()
    } else {
        DEFAULT_OPENAI_PROVIDER_DRIVER.to_string()
    }
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
    let normalized = model.trim().to_ascii_lowercase();
    normalized.starts_with("dall-e") || normalized == "gpt-image-1"
}

fn supports_openai_image_edit(model: &str) -> bool {
    model.trim().eq_ignore_ascii_case("gpt-image-1")
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
        || normalized.starts_with("o1")
        || normalized.starts_with("o3")
        || normalized.starts_with("o4")
}

fn normalize_remote_model_ids(entries: Vec<OpenAIModelEntry>) -> (Vec<String>, Vec<String>) {
    let mut llm_seen = HashSet::<String>::new();
    let mut image_seen = HashSet::<String>::new();
    let mut llm_models = Vec::new();
    let mut image_models = Vec::new();

    for entry in entries.into_iter() {
        let model = entry.id.trim();
        if model.is_empty() {
            continue;
        }
        let key = model.to_ascii_lowercase();
        if is_text2image_model_name(model) {
            if image_seen.insert(key) {
                image_models.push(model.to_string());
            }
        } else if is_supported_llm_model_name(model) && llm_seen.insert(key) {
            llm_models.push(model.to_string());
        }
    }

    (llm_models, image_models)
}

fn inventory_revision(models: &[String], image_models: &[String]) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    models.hash(&mut hasher);
    image_models.hash(&mut hasher);
    format!(
        "models-{}-{}-{:x}",
        models.len(),
        image_models.len(),
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
            base_url: default_base_url(),
            auth_mode: default_auth_mode(),
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
            base_url: raw_instance.base_url,
            auth_mode: raw_instance.auth_mode,
            timeout_ms: raw_instance.timeout_ms,
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
        for alias in [
            "llm.default",
            "llm.chat.default",
            "llm.plan.default",
            "llm.code.default",
        ] {
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
    let api_token = openai_settings.api_token.trim().to_string();
    let mut prepared = Vec::<(OpenAIInstanceConfig, Arc<dyn Provider>)>::new();
    for config in instances.iter() {
        let provider = Arc::new(OpenAIProvider::new(config.clone(), api_token.as_str())?);
        provider.clone().start_inventory_refresh();
        prepared.push((config.clone(), provider));
    }

    for (config, provider) in prepared.into_iter() {
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
    fn build_openai_instances_uses_simplified_runtime_inventory_config() {
        let settings = OpenAISettings {
            enabled: true,
            api_token: "token".to_string(),
            instances: vec![SettingsOpenAIInstanceConfig {
                provider_instance_name: "openai-1".to_string(),
                provider_type: "cloud_api".to_string(),
                base_url: default_base_url(),
                auth_mode: default_auth_mode(),
                timeout_ms: default_timeout_ms(),
            }],
        };

        let instances = build_openai_instances(&settings).expect("instances should be built");
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].provider_instance_name, "openai-1");
        assert_eq!(instances[0].base_url, DEFAULT_OPENAI_BASE_URL);
    }

    #[test]
    fn build_openai_instances_allows_device_jwt_without_static_auth_fields() {
        let settings = OpenAISettings {
            enabled: true,
            api_token: String::new(),
            instances: vec![SettingsOpenAIInstanceConfig {
                provider_instance_name: "sn-ai-provider-1".to_string(),
                provider_type: "cloud_api".to_string(),
                base_url: "https://sn.buckyos.ai/v1".to_string(),
                auth_mode: DEVICE_JWT_AUTH_MODE.to_string(),
                timeout_ms: default_timeout_ms(),
            }],
        };

        let instances = build_openai_instances(&settings).expect("instances should be built");
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].auth_mode, DEVICE_JWT_AUTH_MODE);
    }

    #[test]
    fn use_chat_completions_endpoint_detects_custom_sn_path() {
        let provider = OpenAIProvider::new(
            OpenAIInstanceConfig {
                provider_instance_name: "sn-ai-provider-1".to_string(),
                provider_type: "cloud_api".to_string(),
                base_url: "https://sn.buckyos.ai/api/v1/ai/chat/completions".to_string(),
                auth_mode: "bearer".to_string(),
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
                base_url: default_base_url(),
                auth_mode: "bearer".to_string(),
                timeout_ms: default_timeout_ms(),
            },
            "token",
        )
        .expect("provider should be built");

        let inventory = provider.inventory();
        assert_eq!(inventory.provider_driver, "openai");
        assert!(inventory
            .models
            .iter()
            .any(|model| model.exact_model == "gpt-5@openai-primary"));
        assert!(inventory
            .models
            .iter()
            .any(|model| model.exact_model == "gpt-image-1@openai-primary"));
    }

    #[test]
    fn build_inventory_mounts_only_latest_gpt_tier_models() {
        let models = vec![
            "gpt-5.4".to_string(),
            "gpt-5.5".to_string(),
            "gpt-5.4-pro".to_string(),
            "gpt-5.5-pro".to_string(),
            "gpt-5-mini".to_string(),
            "gpt-5.4-mini".to_string(),
            "gpt-5-nano".to_string(),
            "gpt-5.4-nano".to_string(),
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

        assert_model_mount(&inventory, "gpt-5.5", "llm.gpt", true);
        assert_model_mount(&inventory, "gpt-5.5", "llm.openai.gpt-5-5", true);
        assert_model_mount(&inventory, "gpt-5.4", "llm.gpt", false);
        assert_model_mount(&inventory, "gpt-5.4", "llm.openai.gpt-5-4", true);
        assert_model_mount(&inventory, "gpt-5.5-pro", "llm.gpt-pro", true);
        assert_model_mount(&inventory, "gpt-5.5-pro", "llm.openai.gpt-5-5-pro", true);
        assert_model_mount(&inventory, "gpt-5.4-pro", "llm.gpt-pro", false);
        assert_model_mount(&inventory, "gpt-5.4-mini", "llm.gpt-mini", true);
        assert_model_mount(&inventory, "gpt-5-mini", "llm.gpt-mini", false);
        assert_model_mount(&inventory, "gpt-5.4-nano", "llm.gpt-nano", true);
        assert_model_mount(&inventory, "gpt-5-nano", "llm.gpt-nano", false);
    }

    #[test]
    fn provider_inventory_response_is_normalized_to_latest_gpt_mounts() {
        let provider = OpenAIProvider::new(
            OpenAIInstanceConfig {
                provider_instance_name: "openai-primary".to_string(),
                provider_type: "cloud_api".to_string(),
                base_url: default_base_url(),
                auth_mode: "bearer".to_string(),
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
                        "provider_model_id": "gpt-5.4-pro",
                        "exact_model": "gpt-5.4-pro@remote-openai",
                        "api_types": ["llm.chat"],
                        "logical_mounts": ["llm.gpt-pro", "llm.remote-old"]
                    },
                    {
                        "provider_model_id": "gpt-5.5-pro",
                        "exact_model": "gpt-5.5-pro@remote-openai",
                        "api_types": ["llm.chat"],
                        "logical_mounts": ["llm.gpt-pro"]
                    }
                ]
            }))
            .expect("provider inventory response should parse");

        assert_eq!(inventory.provider_instance_name, "openai-primary");
        assert!(inventory
            .models
            .iter()
            .any(|model| model.exact_model == "gpt-5.5-pro@openai-primary"));
        assert_model_mount(&inventory, "gpt-5.5-pro", "llm.gpt-pro", true);
        assert_model_mount(&inventory, "gpt-5.5-pro", "llm.openai.gpt-5-5-pro", true);
        assert_model_mount(&inventory, "gpt-5.4-pro", "llm.gpt-pro", false);
        assert_model_mount(&inventory, "gpt-5.4-pro", "llm.openai.gpt-5-4-pro", true);
        assert_model_mount(&inventory, "gpt-5.4-pro", "llm.remote-old", true);
    }

    #[test]
    fn remote_model_inventory_filters_supported_model_types() {
        let (llm_models, image_models) = normalize_remote_model_ids(vec![
            OpenAIModelEntry {
                id: "gpt-5.2".to_string(),
            },
            OpenAIModelEntry {
                id: "text-embedding-3-large".to_string(),
            },
            OpenAIModelEntry {
                id: "gpt-image-1".to_string(),
            },
        ]);

        assert_eq!(llm_models, vec!["gpt-5.2".to_string()]);
        assert_eq!(image_models, vec!["gpt-image-1".to_string()]);
    }

    #[test]
    fn remote_model_inventory_excludes_audio_realtime_modalities() {
        // 这些都是 ASR / TTS / 实时音频 modality 的 gpt-* 命名族；它们已经在
        // DEFAULT_OPENAI_{ASR,TTS}_MODELS 里登记，再当 LLM 收一遍会让
        // build_inventory 产生重复 exact_model，触发 model_registry
        // SessionConfigInvalid 把整个 refresh 卡死。
        let (llm_models, image_models) = normalize_remote_model_ids(vec![
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
                id: "gpt-4o-audio-preview".to_string(),
            },
            OpenAIModelEntry {
                id: "gpt-4o-realtime-preview".to_string(),
            },
            OpenAIModelEntry {
                id: "gpt-5".to_string(),
            },
        ]);

        assert_eq!(llm_models, vec!["gpt-5".to_string()]);
        assert!(image_models.is_empty());
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
}
