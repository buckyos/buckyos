use crate::aicc::{
    image_logical_mounts, llm_logical_mounts, logical_mount_segment, provider_model_metadata,
    provider_type_from_settings, AIComputeCenter, Provider, ProviderError, ProviderInstance,
    ProviderStartResult, ResolvedRequest, TaskEventSink,
};
use crate::model_types::{
    ApiType, CostEstimateInput, CostEstimateOutput, PricingMode, ProviderInventory, ProviderOrigin,
    ProviderType, ProviderTypeTrustedSource, QuotaState,
};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use base64::engine::general_purpose;
use base64::Engine as _;
use buckyos_api::{
    ai_methods, features, AiArtifact, AiCost, AiMethodRequest, AiResponseSummary, AiUsage,
    Capability, Feature, ResourceRef,
};
use log::{info, warn};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::time;

const DEFAULT_GIMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";
const DEFAULT_GIMINI_TIMEOUT_MS: u64 = 60_000;
const DEFAULT_GIMINI_MODELS: &str = "gemini-2.5-flash,gemini-2.5-pro";
const DEFAULT_GIMINI_IMAGE_MODELS: &str =
    "gemini-2.0-flash-exp-image-generation,gemini-2.5-flash-image-preview";
const DEFAULT_GIMINI_EMBEDDING_MODELS: &str = "gemini-embedding-001";
const DEFAULT_GIMINI_TTS_MODELS: &str = "gemini-2.5-flash-preview-tts";
const DEFAULT_GIMINI_MUSIC_MODELS: &str = "lyria-002";
const DEFAULT_GIMINI_VIDEO_MODELS: &str = "veo-3.1-generate-preview";
const DEFAULT_GIMINI_INVENTORY_REFRESH_INTERVAL: Duration = Duration::from_secs(300);
const GIMINI_MODELS_PAGE_SIZE: u32 = 1000;
const GIMINI_IMAGE_INPUT_ALLOWLIST: &[&str] = &[
    "candidate_count",
    "max_output_tokens",
    "n",
    "prompt",
    "response_mime_type",
    "response_modalities",
    "seed",
    "stop",
    "temperature",
    "top_k",
    "top_p",
];
const GIMINI_IMAGE_OPTION_ALLOWLIST: &[&str] = &[
    "candidate_count",
    "max_output_tokens",
    "n",
    "output_format",
    "quality",
    "response_mime_type",
    "response_modalities",
    "seed",
    "size",
    "stop",
    "style",
    "temperature",
    "top_k",
    "top_p",
    "user",
];

#[derive(Debug, Clone)]
pub struct GoogleGiminiInstanceConfig {
    pub provider_instance_name: String,
    pub provider_type: String,
    pub provider_driver: String,
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
pub struct GoogleGiminiProvider {
    instance: ProviderInstance,
    inventory: Arc<RwLock<ProviderInventory>>,
    client: Client,
    api_token: String,
    base_url: String,
    provider_type: ProviderType,
    provider_driver: String,
    provider_instance_name: String,
    features: Vec<Feature>,
}

#[derive(Debug, Default)]
struct GiminiModelBuckets {
    llm: Vec<String>,
    image: Vec<String>,
    embedding: Vec<String>,
    tts: Vec<String>,
    music: Vec<String>,
    video: Vec<String>,
}

impl GiminiModelBuckets {
    fn is_empty(&self) -> bool {
        self.llm.is_empty()
            && self.image.is_empty()
            && self.embedding.is_empty()
            && self.tts.is_empty()
            && self.music.is_empty()
            && self.video.is_empty()
    }

    fn fingerprint(&self) -> String {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.llm.hash(&mut hasher);
        self.image.hash(&mut hasher);
        self.embedding.hash(&mut hasher);
        self.tts.hash(&mut hasher);
        self.music.hash(&mut hasher);
        self.video.hash(&mut hasher);
        format!(
            "gimini-models-{}-{}-{}-{}-{}-{}-{:x}",
            self.llm.len(),
            self.image.len(),
            self.embedding.len(),
            self.tts.len(),
            self.music.len(),
            self.video.len(),
            hasher.finish()
        )
    }
}

#[derive(Debug, Deserialize)]
struct GiminiModelsResponse {
    #[serde(default)]
    models: Vec<GiminiModelEntry>,
    #[serde(default, alias = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GiminiModelEntry {
    #[serde(default)]
    name: String,
    #[serde(default, alias = "supportedGenerationMethods")]
    supported_generation_methods: Vec<String>,
}

impl GoogleGiminiProvider {
    pub fn new(cfg: GoogleGiminiInstanceConfig, api_token: String) -> Result<Self> {
        let timeout_ms = if cfg.timeout_ms == 0 {
            DEFAULT_GIMINI_TIMEOUT_MS
        } else {
            cfg.timeout_ms
        };

        let client = Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .context("failed to build reqwest client for google gimini provider")?;

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
            capabilities: vec![
                Capability::Llm,
                Capability::Embedding,
                Capability::Image,
                Capability::Vision,
                Capability::Audio,
                Capability::Video,
            ],
            features: cfg.features.clone(),
            endpoint: Some(cfg.base_url.clone()),
            plugin_key: None,
        };
        let buckets = GiminiModelBuckets {
            llm: cfg
                .models
                .iter()
                .filter(|model| !is_text2image_model_name(model))
                .cloned()
                .collect(),
            image: cfg.image_models.clone(),
            embedding: parse_csv_list(DEFAULT_GIMINI_EMBEDDING_MODELS),
            tts: parse_csv_list(DEFAULT_GIMINI_TTS_MODELS),
            music: parse_csv_list(DEFAULT_GIMINI_MUSIC_MODELS),
            video: parse_csv_list(DEFAULT_GIMINI_VIDEO_MODELS),
        };
        let inventory = Self::build_inventory_from_buckets(
            provider_instance_name.as_str(),
            provider_type.clone(),
            provider_driver.as_str(),
            &buckets,
            cfg.features.as_slice(),
            Some("settings-v1".to_string()),
        );

        Ok(Self {
            instance,
            inventory: Arc::new(RwLock::new(inventory)),
            client,
            api_token,
            base_url: cfg.base_url.trim_end_matches('/').to_string(),
            provider_type,
            provider_driver,
            provider_instance_name,
            features: cfg.features,
        })
    }

    fn build_inventory_from_buckets(
        provider_instance_name: &str,
        provider_type: ProviderType,
        provider_driver: &str,
        buckets: &GiminiModelBuckets,
        features: &[Feature],
        inventory_revision: Option<String>,
    ) -> ProviderInventory {
        let mut models = Vec::new();
        for model in buckets.llm.iter() {
            let mut metadata = provider_model_metadata(
                provider_instance_name,
                provider_type.clone(),
                model.as_str(),
                ApiType::LlmChat,
                llm_logical_mounts(provider_driver, model.as_str()),
                features,
                Some(0.01),
                Some(1400),
            );
            for api_type in [
                ApiType::VisionOcr,
                ApiType::VisionCaption,
                ApiType::VisionDetect,
                ApiType::VisionSegment,
            ] {
                metadata.api_types.push(api_type.clone());
                metadata
                    .logical_mounts
                    .extend(gimini_method_mounts(api_type, model.as_str()));
            }
            metadata.capabilities.vision = true;
            metadata.logical_mounts = dedupe_strings(metadata.logical_mounts);
            models.push(metadata);
        }
        for model in buckets.image.iter() {
            let mut metadata = provider_model_metadata(
                provider_instance_name,
                provider_type.clone(),
                model.as_str(),
                ApiType::ImageTextToImage,
                image_logical_mounts(provider_driver, model.as_str()),
                features,
                Some(0.04),
                Some(6000),
            );
            metadata.api_types.push(ApiType::ImageToImage);
            metadata
                .logical_mounts
                .extend(gimini_method_mounts(ApiType::ImageToImage, model.as_str()));
            metadata.logical_mounts = dedupe_strings(metadata.logical_mounts);
            models.push(metadata);
        }
        for model in buckets.embedding.iter() {
            let mut metadata = provider_model_metadata(
                provider_instance_name,
                provider_type.clone(),
                model.as_str(),
                ApiType::Embedding,
                gimini_method_mounts(ApiType::Embedding, model.as_str()),
                features,
                Some(0.0001),
                Some(800),
            );
            metadata.api_types.push(ApiType::EmbeddingMultimodal);
            metadata.logical_mounts.extend(gimini_method_mounts(
                ApiType::EmbeddingMultimodal,
                model.as_str(),
            ));
            metadata.logical_mounts = dedupe_strings(metadata.logical_mounts);
            models.push(metadata);
        }
        for model in buckets.tts.iter() {
            models.push(provider_model_metadata(
                provider_instance_name,
                provider_type.clone(),
                model.as_str(),
                ApiType::AudioTts,
                gimini_method_mounts(ApiType::AudioTts, model.as_str()),
                features,
                Some(0.01),
                Some(3000),
            ));
        }
        for model in buckets.music.iter() {
            models.push(provider_model_metadata(
                provider_instance_name,
                provider_type.clone(),
                model.as_str(),
                ApiType::AudioMusic,
                gimini_method_mounts(ApiType::AudioMusic, model.as_str()),
                features,
                Some(0.10),
                Some(60_000),
            ));
        }
        for model in buckets.video.iter() {
            let mut metadata = provider_model_metadata(
                provider_instance_name,
                provider_type.clone(),
                model.as_str(),
                ApiType::VideoTextToVideo,
                gimini_method_mounts(ApiType::VideoTextToVideo, model.as_str()),
                features,
                Some(0.50),
                Some(120_000),
            );
            for api_type in [
                ApiType::VideoImageToVideo,
                ApiType::VideoToVideo,
                ApiType::VideoExtend,
            ] {
                metadata.api_types.push(api_type.clone());
                metadata
                    .logical_mounts
                    .extend(gimini_method_mounts(api_type, model.as_str()));
            }
            metadata.logical_mounts = dedupe_strings(metadata.logical_mounts);
            models.push(metadata);
        }
        ProviderInventory {
            provider_instance_name: provider_instance_name.to_string(),
            provider_type,
            provider_driver: provider_driver.to_string(),
            provider_origin: ProviderOrigin::SystemConfig,
            provider_type_trusted_source: ProviderTypeTrustedSource::SystemConfig,
            provider_type_revision: None,
            version: None,
            inventory_revision,
            models,
        }
    }

    pub fn start_inventory_refresh(self: Arc<Self>) {
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }

        tokio::spawn(async move {
            if let Err(err) = self.refresh_inventory_once().await {
                warn!(
                    "aicc.gimini.inventory.initial_refresh_failed provider_instance_name={} err={}",
                    self.provider_instance_name, err
                );
            }

            let mut interval = time::interval(DEFAULT_GIMINI_INVENTORY_REFRESH_INTERVAL);
            interval.tick().await;
            loop {
                interval.tick().await;
                if let Err(err) = self.refresh_inventory_once().await {
                    warn!(
                        "aicc.gimini.inventory.refresh_failed provider_instance_name={} err={}",
                        self.provider_instance_name, err
                    );
                }
            }
        });
    }

    async fn refresh_inventory_once(&self) -> Result<ProviderInventory> {
        let mut buckets = GiminiModelBuckets::default();
        let mut llm_seen = HashSet::<String>::new();
        let mut image_seen = HashSet::<String>::new();
        let mut embedding_seen = HashSet::<String>::new();
        let mut tts_seen = HashSet::<String>::new();
        let mut music_seen = HashSet::<String>::new();
        let mut video_seen = HashSet::<String>::new();
        let mut page_token: Option<String> = None;

        let endpoint = format!("{}/models", self.base_url);
        let page_size = GIMINI_MODELS_PAGE_SIZE.to_string();
        loop {
            let mut request = self
                .client
                .get(endpoint.as_str())
                .query(&[("key", self.api_token.as_str())])
                .query(&[("pageSize", page_size.as_str())]);
            if let Some(token) = page_token.as_deref() {
                request = request.query(&[("pageToken", token)]);
            }

            let response = request
                .send()
                .await
                .context("gimini inventory refresh request failed")?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                return Err(anyhow!(
                    "gimini inventory refresh failed status={} body={}",
                    status,
                    body
                ));
            }

            let parsed = response
                .json::<GiminiModelsResponse>()
                .await
                .context("failed to parse gimini models response")?;

            for entry in parsed.models.iter() {
                let id = strip_gimini_model_prefix(entry.name.as_str()).trim();
                if id.is_empty() {
                    continue;
                }
                let methods = entry
                    .supported_generation_methods
                    .iter()
                    .map(|method| method.to_ascii_lowercase())
                    .collect::<HashSet<_>>();
                let key = id.to_ascii_lowercase();
                match classify_gimini_model(id, &methods) {
                    Some(GiminiModelKind::Llm) => {
                        if llm_seen.insert(key) {
                            buckets.llm.push(id.to_string());
                        }
                    }
                    Some(GiminiModelKind::Image) => {
                        if image_seen.insert(key) {
                            buckets.image.push(id.to_string());
                        }
                    }
                    Some(GiminiModelKind::Embedding) => {
                        if embedding_seen.insert(key) {
                            buckets.embedding.push(id.to_string());
                        }
                    }
                    Some(GiminiModelKind::Tts) => {
                        if tts_seen.insert(key) {
                            buckets.tts.push(id.to_string());
                        }
                    }
                    Some(GiminiModelKind::Music) => {
                        if music_seen.insert(key) {
                            buckets.music.push(id.to_string());
                        }
                    }
                    Some(GiminiModelKind::Video) => {
                        if video_seen.insert(key) {
                            buckets.video.push(id.to_string());
                        }
                    }
                    None => continue,
                }
            }

            match parsed.next_page_token {
                Some(token) if !token.is_empty() => page_token = Some(token),
                _ => break,
            }
        }

        if buckets.is_empty() {
            return Err(anyhow!(
                "gimini inventory refresh returned no supported models"
            ));
        }

        // Categories that the API never returns (lyria/veo are typically not
        // listed) fall back to defaults so we don't drop them on refresh.
        if buckets.embedding.is_empty() {
            buckets.embedding = parse_csv_list(DEFAULT_GIMINI_EMBEDDING_MODELS);
        }
        if buckets.tts.is_empty() {
            buckets.tts = parse_csv_list(DEFAULT_GIMINI_TTS_MODELS);
        }
        if buckets.music.is_empty() {
            buckets.music = parse_csv_list(DEFAULT_GIMINI_MUSIC_MODELS);
        }
        if buckets.video.is_empty() {
            buckets.video = parse_csv_list(DEFAULT_GIMINI_VIDEO_MODELS);
        }

        let revision = Some(buckets.fingerprint());
        let inventory = Self::build_inventory_from_buckets(
            self.provider_instance_name.as_str(),
            self.provider_type.clone(),
            self.provider_driver.as_str(),
            &buckets,
            self.features.as_slice(),
            revision,
        );

        {
            let mut current = self
                .inventory
                .write()
                .map_err(|_| anyhow!("gimini inventory lock poisoned"))?;
            *current = inventory.clone();
        }
        info!(
            "aicc.gimini.inventory.refreshed provider_instance_name={} llm={} image={} embedding={} tts={} music={} video={}",
            self.provider_instance_name,
            buckets.llm.len(),
            buckets.image.len(),
            buckets.embedding.len(),
            buckets.tts.len(),
            buckets.music.len(),
            buckets.video.len(),
        );
        Ok(inventory)
    }

    fn price_per_1m_tokens(model: &str) -> (f64, f64) {
        let lowered = model.to_ascii_lowercase();
        if lowered.contains("2.5-pro") {
            (1.25, 10.0)
        } else if lowered.contains("2.5-flash") {
            (0.30, 2.50)
        } else if lowered.contains("1.5-pro") {
            (1.25, 5.0)
        } else if lowered.contains("1.5-flash") {
            (0.075, 0.30)
        } else {
            (0.50, 2.0)
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
            .and_then(|value| value.get("max_tokens"))
            .and_then(|value| value.as_u64())
            .or_else(|| {
                req.payload
                    .input_json
                    .as_ref()
                    .and_then(|value| value.get("max_output_tokens"))
                    .and_then(|value| value.as_u64())
            })
            .or_else(|| {
                req.payload
                    .options
                    .as_ref()
                    .and_then(|value| value.get("max_tokens"))
                    .and_then(|value| value.as_u64())
            })
            .or_else(|| {
                req.payload
                    .options
                    .as_ref()
                    .and_then(|value| value.get("max_output_tokens"))
                    .and_then(|value| value.as_u64())
            })
            .unwrap_or(1024);

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
                    .options
                    .as_ref()
                    .and_then(|value| value.get("candidate_count"))
                    .and_then(|value| value.as_u64())
            })
            .or_else(|| {
                req.payload
                    .input_json
                    .as_ref()
                    .and_then(|value| value.get("n"))
                    .and_then(|value| value.as_u64())
            })
            .or_else(|| {
                req.payload
                    .input_json
                    .as_ref()
                    .and_then(|value| value.get("candidate_count"))
                    .and_then(|value| value.as_u64())
            })
            .unwrap_or(1)
            .max(1)
    }

    fn estimate_text2image_cost(req: &AiMethodRequest, model: &str) -> Option<f64> {
        let lowered = model.to_ascii_lowercase();
        let per_image = if lowered.contains("2.5-flash-image") {
            0.039
        } else if lowered.contains("2.0-flash-exp-image-generation")
            || lowered.contains("2.0-flash-preview-image-generation")
        {
            0.03
        } else if lowered.contains("2.5") {
            0.04
        } else {
            0.03
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

    fn role_to_gimini(role: &str) -> &'static str {
        match role.trim().to_ascii_lowercase().as_str() {
            "assistant" => "model",
            _ => "user",
        }
    }

    fn resource_text(resource: &ResourceRef) -> Result<String, ProviderError> {
        match resource {
            ResourceRef::Url { url, .. } => Ok(format!("resource_url: {}", url)),
            ResourceRef::NamedObject { obj_id } => Ok(format!("named_object: {}", obj_id)),
            ResourceRef::Base64 { .. } => Err(ProviderError::fatal(
                "google gimini provider does not support base64 resources in this version",
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
                Some("text") => {
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
                let content = msg.content.trim();
                (!content.is_empty()).then(|| (msg.role.clone(), content.to_string()))
            })
            .collect())
    }

    fn build_contents(&self, req: &AiMethodRequest) -> Result<Vec<Value>, ProviderError> {
        let mut contents = vec![];

        for (role, content) in Self::canonical_message_texts(req)? {
            contents.push(json!({
                "role": Self::role_to_gimini(role.as_str()),
                "parts": [
                    {
                        "text": content
                    }
                ]
            }));
        }

        if contents.is_empty() {
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
                contents.push(json!({
                    "role": "user",
                    "parts": [
                        {
                            "text": content
                        }
                    ]
                }));
            }
        }

        if contents.is_empty() {
            return Err(ProviderError::fatal(
                "request payload has no usable text/messages for llm",
            ));
        }

        Ok(contents)
    }

    fn extract_text_content(body: &Value) -> Option<String> {
        let parts = body.pointer("/candidates/0/content/parts")?.as_array()?;
        let joined = parts
            .iter()
            .filter_map(|part| part.get("text").and_then(|value| value.as_str()))
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

    fn normalize_stop_sequences(stop: &Value) -> Result<Value, ProviderError> {
        if let Some(stop_str) = stop.as_str() {
            return Ok(Value::Array(vec![Value::String(stop_str.to_string())]));
        }

        let Some(stop_values) = stop.as_array() else {
            return Err(ProviderError::fatal(
                "stop must be a string or array of strings",
            ));
        };

        let mut normalized = Vec::with_capacity(stop_values.len());
        for (idx, item) in stop_values.iter().enumerate() {
            let Some(stop_str) = item.as_str() else {
                return Err(ProviderError::fatal(format!(
                    "stop[{}] must be a string",
                    idx
                )));
            };
            normalized.push(Value::String(stop_str.to_string()));
        }

        Ok(Value::Array(normalized))
    }

    fn ensure_generation_config(target: &mut Map<String, Value>) -> &mut Map<String, Value> {
        if !target.contains_key("generationConfig") {
            target.insert("generationConfig".to_string(), Value::Object(Map::new()));
        }
        target
            .get_mut("generationConfig")
            .and_then(|value| value.as_object_mut())
            .expect("generationConfig should be an object")
    }

    fn merge_llm_options(
        target: &mut Map<String, Value>,
        options: &Value,
        json_output_required: bool,
    ) -> Result<Vec<String>, ProviderError> {
        let Some(options_map) = options.as_object() else {
            if json_output_required {
                let generation = Self::ensure_generation_config(target);
                if !generation.contains_key("responseMimeType") {
                    generation.insert(
                        "responseMimeType".to_string(),
                        Value::String("application/json".to_string()),
                    );
                }
            }
            return Ok(vec![]);
        };

        let mut ignored = vec![];
        for (key, value) in options_map.iter() {
            if key == "model" || key == "messages" {
                continue;
            }
            if key == "protocol" || key == "process_name" || key == "tool_messages" {
                ignored.push(key.clone());
                continue;
            }

            match key.as_str() {
                "temperature" => {
                    Self::ensure_generation_config(target)
                        .insert("temperature".to_string(), value.clone());
                }
                "top_p" | "topP" => {
                    Self::ensure_generation_config(target)
                        .insert("topP".to_string(), value.clone());
                }
                "top_k" | "topK" => {
                    Self::ensure_generation_config(target)
                        .insert("topK".to_string(), value.clone());
                }
                "max_tokens" | "max_completion_tokens" | "max_output_tokens" => {
                    Self::ensure_generation_config(target)
                        .insert("maxOutputTokens".to_string(), value.clone());
                }
                "candidate_count" => {
                    Self::ensure_generation_config(target)
                        .insert("candidateCount".to_string(), value.clone());
                }
                "stop" => {
                    Self::ensure_generation_config(target).insert(
                        "stopSequences".to_string(),
                        Self::normalize_stop_sequences(value)?,
                    );
                }
                "response_mime_type" => {
                    Self::ensure_generation_config(target)
                        .insert("responseMimeType".to_string(), value.clone());
                }
                "response_schema" => {
                    let generation = Self::ensure_generation_config(target);
                    generation.insert("responseSchema".to_string(), value.clone());
                    if !generation.contains_key("responseMimeType") {
                        generation.insert(
                            "responseMimeType".to_string(),
                            Value::String("application/json".to_string()),
                        );
                    }
                }
                _ => {
                    ignored.push(key.clone());
                }
            }
        }

        if json_output_required {
            let generation = Self::ensure_generation_config(target);
            if !generation.contains_key("responseMimeType") {
                generation.insert(
                    "responseMimeType".to_string(),
                    Value::String("application/json".to_string()),
                );
            }
        }

        Ok(ignored)
    }

    fn merge_text2image_input_json(
        target: &mut Map<String, Value>,
        input_json: &Value,
    ) -> Result<(), ProviderError> {
        let Some(input_map) = input_json.as_object() else {
            return Ok(());
        };

        for (key, value) in input_map.iter() {
            if !GIMINI_IMAGE_INPUT_ALLOWLIST.contains(&key.as_str()) {
                continue;
            }
            match key.as_str() {
                "prompt" => {
                    target.insert("prompt".to_string(), value.clone());
                }
                "response_modalities" => {
                    Self::ensure_generation_config(target)
                        .insert("responseModalities".to_string(), value.clone());
                }
                "response_mime_type" => {
                    Self::ensure_generation_config(target)
                        .insert("responseMimeType".to_string(), value.clone());
                }
                "max_output_tokens" => {
                    Self::ensure_generation_config(target)
                        .insert("maxOutputTokens".to_string(), value.clone());
                }
                "candidate_count" | "n" => {
                    Self::ensure_generation_config(target)
                        .insert("candidateCount".to_string(), value.clone());
                }
                "stop" => {
                    Self::ensure_generation_config(target).insert(
                        "stopSequences".to_string(),
                        Self::normalize_stop_sequences(value)?,
                    );
                }
                "temperature" => {
                    Self::ensure_generation_config(target)
                        .insert("temperature".to_string(), value.clone());
                }
                "top_k" => {
                    Self::ensure_generation_config(target)
                        .insert("topK".to_string(), value.clone());
                }
                "top_p" => {
                    Self::ensure_generation_config(target)
                        .insert("topP".to_string(), value.clone());
                }
                "seed" => {
                    Self::ensure_generation_config(target)
                        .insert("seed".to_string(), value.clone());
                }
                _ => {}
            }
        }
        Ok(())
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
            if key == "model" || key == "messages" {
                continue;
            }
            if key == "protocol" || key == "process_name" || key == "tool_messages" {
                ignored.push(key.clone());
                continue;
            }
            if !GIMINI_IMAGE_OPTION_ALLOWLIST.contains(&key.as_str()) && key != "prompt" {
                ignored.push(key.clone());
                continue;
            }

            match key.as_str() {
                "prompt" => {
                    target.insert("prompt".to_string(), value.clone());
                }
                "response_modalities" => {
                    Self::ensure_generation_config(target)
                        .insert("responseModalities".to_string(), value.clone());
                }
                "response_mime_type" | "output_format" => {
                    Self::ensure_generation_config(target)
                        .insert("responseMimeType".to_string(), value.clone());
                }
                "max_output_tokens" => {
                    Self::ensure_generation_config(target)
                        .insert("maxOutputTokens".to_string(), value.clone());
                }
                "candidate_count" | "n" => {
                    Self::ensure_generation_config(target)
                        .insert("candidateCount".to_string(), value.clone());
                }
                "stop" => {
                    Self::ensure_generation_config(target).insert(
                        "stopSequences".to_string(),
                        Self::normalize_stop_sequences(value)?,
                    );
                }
                "temperature" => {
                    Self::ensure_generation_config(target)
                        .insert("temperature".to_string(), value.clone());
                }
                "top_k" => {
                    Self::ensure_generation_config(target)
                        .insert("topK".to_string(), value.clone());
                }
                "top_p" => {
                    Self::ensure_generation_config(target)
                        .insert("topP".to_string(), value.clone());
                }
                "seed" => {
                    Self::ensure_generation_config(target)
                        .insert("seed".to_string(), value.clone());
                }
                _ => {
                    ignored.push(key.clone());
                }
            }
        }
        Ok(ignored)
    }

    fn parse_text2image_result(
        body: &Value,
    ) -> Result<(Vec<AiArtifact>, Option<String>), ProviderError> {
        let Some(candidates) = body.get("candidates").and_then(|value| value.as_array()) else {
            return Err(ProviderError::fatal(
                "google gimini image response is missing candidates array",
            ));
        };

        let mut artifacts = vec![];
        let mut text_notes = vec![];
        for candidate in candidates.iter() {
            if let Some(parts) = candidate
                .pointer("/content/parts")
                .and_then(|value| value.as_array())
            {
                for part in parts.iter() {
                    if let Some(inline_data) =
                        part.get("inlineData").and_then(|value| value.as_object())
                    {
                        let Some(data_base64) =
                            inline_data.get("data").and_then(|value| value.as_str())
                        else {
                            continue;
                        };
                        let mime = inline_data
                            .get("mimeType")
                            .and_then(|value| value.as_str())
                            .unwrap_or("image/png");
                        if general_purpose::STANDARD.decode(data_base64).is_err() {
                            warn!(
                                "aicc.gimini received invalid inlineData base64 in image response"
                            );
                            continue;
                        }

                        let seq = artifacts.len() + 1;
                        artifacts.push(AiArtifact {
                            name: format!("image_{}", seq),
                            resource: ResourceRef::Base64 {
                                mime: mime.to_string(),
                                data_base64: data_base64.to_string(),
                            },
                            mime: Some(mime.to_string()),
                            metadata: None,
                        });
                        continue;
                    }

                    if let Some(file_data) =
                        part.get("fileData").and_then(|value| value.as_object())
                    {
                        let Some(uri) = file_data.get("fileUri").and_then(|value| value.as_str())
                        else {
                            continue;
                        };
                        let mime = file_data
                            .get("mimeType")
                            .and_then(|value| value.as_str())
                            .unwrap_or("image/png");
                        let seq = artifacts.len() + 1;
                        artifacts.push(AiArtifact {
                            name: format!("image_{}", seq),
                            resource: ResourceRef::Url {
                                url: uri.to_string(),
                                mime_hint: Some(mime.to_string()),
                            },
                            mime: Some(mime.to_string()),
                            metadata: None,
                        });
                        continue;
                    }

                    if let Some(text) = part.get("text").and_then(|value| value.as_str()) {
                        if !text.trim().is_empty() {
                            text_notes.push(text.trim().to_string());
                        }
                    }
                }
            }
        }

        if artifacts.is_empty() {
            return Err(ProviderError::fatal(
                "google gimini image response has no usable image outputs",
            ));
        }

        let text = if text_notes.is_empty() {
            None
        } else {
            Some(text_notes.join("\n"))
        };
        Ok((artifacts, text))
    }

    async fn post_generate_content(
        &self,
        provider_model: &str,
        request_obj: &Map<String, Value>,
    ) -> Result<(StatusCode, Value, u64), ProviderError> {
        let url = format!(
            "{}/models/{}:generateContent",
            self.base_url, provider_model
        );
        let started_at = std::time::Instant::now();
        let response = self
            .client
            .post(url.as_str())
            .header("x-goog-api-key", self.api_token.as_str())
            .json(request_obj)
            .send()
            .await
            .map_err(|err| {
                if err.is_timeout() || err.is_connect() {
                    ProviderError::retryable(format!("google gimini request failed: {}", err))
                } else {
                    ProviderError::fatal(format!("google gimini request failed: {}", err))
                }
            })?;
        let latency_ms = started_at.elapsed().as_millis() as u64;

        let status = response.status();
        let body: Value = response.json().await.map_err(|err| {
            if status.as_u16() == 429 || status.is_server_error() {
                ProviderError::retryable(format!(
                    "failed to parse google gimini response body: {}",
                    err
                ))
            } else {
                ProviderError::fatal(format!(
                    "failed to parse google gimini response body: {}",
                    err
                ))
            }
        })?;

        Ok((status, body, latency_ms))
    }

    async fn post_model_action(
        &self,
        provider_model: &str,
        action: &str,
        request_obj: &Map<String, Value>,
    ) -> Result<(StatusCode, Value, u64), ProviderError> {
        let url = format!("{}/models/{}:{}", self.base_url, provider_model, action);
        let started_at = std::time::Instant::now();
        let response = self
            .client
            .post(url.as_str())
            .header("x-goog-api-key", self.api_token.as_str())
            .json(request_obj)
            .send()
            .await
            .map_err(|err| {
                if err.is_timeout() || err.is_connect() {
                    ProviderError::retryable(format!("google gimini request failed: {}", err))
                } else {
                    ProviderError::fatal(format!("google gimini request failed: {}", err))
                }
            })?;
        let latency_ms = started_at.elapsed().as_millis() as u64;
        let status = response.status();
        let body: Value = response.json().await.map_err(|err| {
            Self::classify_api_error(
                status,
                format!("failed to parse google gimini response body: {}", err),
            )
        })?;
        Ok((status, body, latency_ms))
    }

    fn resource_from_input_json(req: &AiMethodRequest, keys: &[&str]) -> Option<ResourceRef> {
        let input = req.payload.input_json.as_ref()?;
        for key in keys {
            if let Some(value) = input.get(*key) {
                if let Ok(resource) = serde_json::from_value::<ResourceRef>(value.clone()) {
                    return Some(resource);
                }
            }
        }
        None
    }

    fn resource_part(resource: &ResourceRef) -> Result<Value, ProviderError> {
        match resource {
            ResourceRef::Url { url, mime_hint } => Ok(json!({
                "fileData": {
                    "fileUri": url,
                    "mimeType": mime_hint.as_deref().unwrap_or("application/octet-stream")
                }
            })),
            ResourceRef::Base64 { mime, data_base64 } => Ok(json!({
                "inlineData": {
                    "mimeType": mime,
                    "data": data_base64
                }
            })),
            ResourceRef::NamedObject { obj_id } => Err(ProviderError::fatal(format!(
                "google gimini provider cannot resolve named object resource {} without resolver bytes",
                obj_id
            ))),
        }
    }

    fn prompt_for_method(method: &str, req: &AiMethodRequest) -> String {
        if let Some(prompt) = Self::extract_text2image_prompt(req) {
            return prompt;
        }
        match method {
            ai_methods::VISION_OCR => "Extract readable text from the image and return structured OCR JSON.".to_string(),
            ai_methods::VISION_CAPTION => "Caption the image concisely.".to_string(),
            ai_methods::VISION_DETECT => "Detect objects in the image. Return JSON detections with label, score and bbox.".to_string(),
            ai_methods::VISION_SEGMENT => "Segment the requested subject in the image. Return JSON masks or mask descriptions.".to_string(),
            ai_methods::AUDIO_TTS => "Synthesize the requested text as speech.".to_string(),
            ai_methods::AUDIO_MUSIC => "Generate music from the requested prompt.".to_string(),
            _ => "Process the request.".to_string(),
        }
    }

    fn parse_media_artifacts(body: &Value, default_mime: &str) -> Vec<AiArtifact> {
        let mut artifacts = Vec::new();
        if let Some(parts) = body
            .pointer("/candidates/0/content/parts")
            .and_then(|value| value.as_array())
        {
            for part in parts {
                if let Some(inline_data) =
                    part.get("inlineData").and_then(|value| value.as_object())
                {
                    if let Some(data_base64) =
                        inline_data.get("data").and_then(|value| value.as_str())
                    {
                        let mime = inline_data
                            .get("mimeType")
                            .and_then(|value| value.as_str())
                            .unwrap_or(default_mime);
                        artifacts.push(AiArtifact {
                            name: format!("artifact_{}", artifacts.len() + 1),
                            resource: ResourceRef::Base64 {
                                mime: mime.to_string(),
                                data_base64: data_base64.to_string(),
                            },
                            mime: Some(mime.to_string()),
                            metadata: None,
                        });
                    }
                }
                if let Some(file_data) = part.get("fileData").and_then(|value| value.as_object()) {
                    if let Some(uri) = file_data.get("fileUri").and_then(|value| value.as_str()) {
                        let mime = file_data
                            .get("mimeType")
                            .and_then(|value| value.as_str())
                            .unwrap_or(default_mime);
                        artifacts.push(AiArtifact {
                            name: format!("artifact_{}", artifacts.len() + 1),
                            resource: ResourceRef::Url {
                                url: uri.to_string(),
                                mime_hint: Some(mime.to_string()),
                            },
                            mime: Some(mime.to_string()),
                            metadata: None,
                        });
                    }
                }
            }
        }
        artifacts
    }

    async fn start_llm(
        &self,
        ctx: &crate::aicc::InvokeCtx,
        provider_model: &str,
        req: &AiMethodRequest,
    ) -> Result<ProviderStartResult, ProviderError> {
        let contents = self.build_contents(req)?;
        let mut request_obj = Map::new();
        request_obj.insert("contents".to_string(), Value::Array(contents));

        let json_output_required = req
            .requirements
            .must_features
            .iter()
            .any(|feature| feature == features::JSON_OUTPUT);
        let mut ignored_options = vec![];
        if let Some(input_json) = req.payload.input_json.as_ref() {
            ignored_options.extend(Self::merge_llm_options(
                &mut request_obj,
                input_json,
                json_output_required,
            )?);
        }
        if let Some(options) = req.payload.options.as_ref() {
            ignored_options.extend(Self::merge_llm_options(
                &mut request_obj,
                options,
                json_output_required,
            )?);
        }
        if req.payload.input_json.is_none() && req.payload.options.is_none() && json_output_required
        {
            let generation = Self::ensure_generation_config(&mut request_obj);
            generation.insert(
                "responseMimeType".to_string(),
                Value::String("application/json".to_string()),
            );
        }

        if !ignored_options.is_empty() {
            warn!(
                "aicc.gimini ignored unsupported llm options: provider_instance_name={} model={} trace_id={:?} ignored={:?}",
                self.instance.provider_instance_name, provider_model, ctx.trace_id, ignored_options
            );
        }

        let request_log = Value::Object(request_obj.clone()).to_string();
        info!(
            "aicc.gimini.llm.input provider_instance_name={} model={} trace_id={:?} request={}",
            self.instance.provider_instance_name, provider_model, ctx.trace_id, request_log
        );

        let (status, body, latency_ms) = self
            .post_generate_content(provider_model, &request_obj)
            .await?;
        let response_log = body.to_string();

        if !status.is_success() {
            warn!(
                "aicc.gimini.llm.output provider_instance_name={} model={} trace_id={:?} status={} response={}",
                self.instance.provider_instance_name,
                provider_model,
                ctx.trace_id,
                status.as_u16(),
                response_log
            );
            let message = body
                .pointer("/error/message")
                .and_then(|value| value.as_str())
                .unwrap_or("google gimini api returned non-success status")
                .to_string();
            let code = body
                .pointer("/error/status")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            return Err(Self::classify_api_error(
                status,
                format!("google gimini api error [{}]: {}", code, message),
            ));
        }

        info!(
            "aicc.gimini.llm.output provider_instance_name={} model={} trace_id={:?} status={} response={}",
            self.instance.provider_instance_name,
            provider_model,
            ctx.trace_id,
            status.as_u16(),
            response_log
        );

        let content = Self::extract_text_content(&body);
        let usage = body.get("usageMetadata").map(|usage| AiUsage {
            input_tokens: usage
                .get("promptTokenCount")
                .and_then(|value| value.as_u64()),
            output_tokens: usage
                .get("candidatesTokenCount")
                .and_then(|value| value.as_u64()),
            total_tokens: usage
                .get("totalTokenCount")
                .and_then(|value| value.as_u64()),
        });

        let cost = usage
            .as_ref()
            .and_then(|usage| self.estimate_cost_for_usage(provider_model, usage));

        let mut extra = Map::new();
        extra.insert(
            "provider".to_string(),
            Value::String("google_gemini".to_string()),
        );
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
            tool_calls: vec![],
            artifacts: vec![],
            usage,
            cost,
            finish_reason: body
                .pointer("/candidates/0/finishReason")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string()),
            provider_task_ref: None,
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
        if let Some(input_json) = req.payload.input_json.as_ref() {
            Self::merge_text2image_input_json(&mut request_obj, input_json)?;
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
                "aicc.gimini ignored unsupported text2image options: provider_instance_name={} model={} trace_id={:?} ignored={:?}",
                self.instance.provider_instance_name, provider_model, ctx.trace_id, ignored_options
            );
        }

        let prompt = request_obj
            .get("prompt")
            .and_then(|value| value.as_str())
            .ok_or_else(|| ProviderError::fatal("text2image prompt must be a string"))?
            .to_string();

        let generation = Self::ensure_generation_config(&mut request_obj);
        if !generation.contains_key("responseModalities") {
            generation.insert("responseModalities".to_string(), json!(["IMAGE"]));
        }
        let contents = json!([
            {
                "role": "user",
                "parts": [
                    {
                        "text": prompt
                    }
                ]
            }
        ]);
        request_obj.insert("contents".to_string(), contents);
        request_obj.remove("prompt");

        let request_log = Value::Object(request_obj.clone()).to_string();
        info!(
            "aicc.gimini.text2image.input provider_instance_name={} model={} trace_id={:?} request={}",
            self.instance.provider_instance_name, provider_model, ctx.trace_id, request_log
        );

        let (status, body, latency_ms) = self
            .post_generate_content(provider_model, &request_obj)
            .await?;
        let response_log = body.to_string();

        if !status.is_success() {
            warn!(
                "aicc.gimini.text2image.output provider_instance_name={} model={} trace_id={:?} status={} response={}",
                self.instance.provider_instance_name,
                provider_model,
                ctx.trace_id,
                status.as_u16(),
                response_log
            );
            let message = body
                .pointer("/error/message")
                .and_then(|value| value.as_str())
                .unwrap_or("google gimini api returned non-success status")
                .to_string();
            let code = body
                .pointer("/error/status")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            return Err(Self::classify_api_error(
                status,
                format!("google gimini api error [{}]: {}", code, message),
            ));
        }

        info!(
            "aicc.gimini.text2image.output provider_instance_name={} model={} trace_id={:?} status={} response={}",
            self.instance.provider_instance_name,
            provider_model,
            ctx.trace_id,
            status.as_u16(),
            response_log
        );

        let (artifacts, text) = Self::parse_text2image_result(&body)?;
        let estimated_cost =
            Self::estimate_text2image_cost(req, provider_model).map(|amount| AiCost {
                amount,
                currency: "USD".to_string(),
            });

        let mut extra = Map::new();
        extra.insert(
            "provider".to_string(),
            Value::String("google_gemini".to_string()),
        );
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
            text,
            tool_calls: vec![],
            artifacts,
            usage: None,
            cost: estimated_cost,
            finish_reason: body
                .pointer("/candidates/0/finishReason")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string()),
            provider_task_ref: None,
            extra: Some(Value::Object(extra)),
        };

        Ok(ProviderStartResult::Immediate(summary))
    }

    async fn start_image2image(
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
            .or_else(|| Self::resource_from_input_json(req, &["image"]))
            .ok_or_else(|| ProviderError::fatal("image.img2img requires an image resource"))?;
        let prompt = Self::extract_text2image_prompt(req).ok_or_else(|| {
            ProviderError::fatal("image.img2img requires prompt in payload text/input_json/options")
        })?;
        let mut request_obj = Map::new();
        request_obj.insert(
            "contents".to_string(),
            json!([{
                "role": "user",
                "parts": [
                    Self::resource_part(&resource)?,
                    { "text": prompt }
                ]
            }]),
        );
        Self::ensure_generation_config(&mut request_obj)
            .insert("responseModalities".to_string(), json!(["IMAGE"]));
        let (status, body, latency_ms) = self
            .post_generate_content(provider_model, &request_obj)
            .await?;
        if !status.is_success() {
            let message = body
                .pointer("/error/message")
                .and_then(|value| value.as_str())
                .unwrap_or("google gimini image edit returned non-success status");
            return Err(Self::classify_api_error(status, message.to_string()));
        }
        let (artifacts, text) = Self::parse_text2image_result(&body)?;
        let mut extra = Map::new();
        extra.insert(
            "provider".to_string(),
            Value::String("google_gemini".to_string()),
        );
        extra.insert(
            "model".to_string(),
            Value::String(provider_model.to_string()),
        );
        extra.insert("latency_ms".to_string(), Value::from(latency_ms));
        extra.insert(
            "provider_io".to_string(),
            json!({ "input": request_obj, "output": body }),
        );
        Ok(ProviderStartResult::Immediate(AiResponseSummary {
            text,
            artifacts,
            finish_reason: Some("stop".to_string()),
            extra: Some(Value::Object(extra)),
            ..Default::default()
        }))
    }

    async fn start_embedding(
        &self,
        ctx: &crate::aicc::InvokeCtx,
        provider_model: &str,
        req: &AiMethodRequest,
        multimodal: bool,
    ) -> Result<ProviderStartResult, ProviderError> {
        let text = req
            .payload
            .input_json
            .as_ref()
            .and_then(|value| value.pointer("/items/0/text"))
            .and_then(|value| value.as_str())
            .or(req.payload.text.as_deref())
            .unwrap_or("");
        let mut parts = Vec::new();
        if !text.trim().is_empty() {
            parts.push(json!({ "text": text }));
        }
        if multimodal {
            if let Some(resource) = req
                .payload
                .resources
                .first()
                .cloned()
                .or_else(|| Self::resource_from_input_json(req, &["image", "audio", "video"]))
            {
                parts.push(Self::resource_part(&resource)?);
            }
        }
        if parts.is_empty() {
            return Err(ProviderError::fatal(
                "embedding request requires text or multimodal resource",
            ));
        }
        let mut request_obj = Map::new();
        request_obj.insert("content".to_string(), json!({ "parts": parts }));
        if let Some(dimensions) = req
            .payload
            .input_json
            .as_ref()
            .and_then(|value| value.get("dimensions"))
            .cloned()
        {
            request_obj.insert("outputDimensionality".to_string(), dimensions);
        }
        let (status, body, latency_ms) = self
            .post_model_action(provider_model, "embedContent", &request_obj)
            .await?;
        if !status.is_success() {
            let message = body
                .pointer("/error/message")
                .and_then(|value| value.as_str())
                .unwrap_or("google gimini embedding returned non-success status");
            return Err(Self::classify_api_error(status, message.to_string()));
        }
        let dimensions = body
            .pointer("/embedding/values")
            .and_then(|value| value.as_array())
            .map(|items| items.len())
            .unwrap_or(0);
        let embedding_space_id =
            format!("google-gemini:{}:{}:multimodal", provider_model, dimensions);
        let mut extra = Map::new();
        extra.insert(
            "embedding".to_string(),
            json!({
                "data": [{
                    "index": 0,
                    "embedding": body.pointer("/embedding/values").cloned().unwrap_or(Value::Array(vec![])),
                    "embedding_space_id": embedding_space_id
                }],
                "embedding_space_id": embedding_space_id,
                "provider_io": { "input": request_obj, "output": body },
                "latency_ms": latency_ms
            }),
        );
        Ok(ProviderStartResult::Immediate(AiResponseSummary {
            finish_reason: Some("stop".to_string()),
            extra: Some(Value::Object(extra)),
            ..Default::default()
        }))
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
            .ok_or_else(|| {
                ProviderError::fatal("vision request requires image/document resource")
            })?;
        let request_obj = json!({
            "contents": [{
                "role": "user",
                "parts": [
                    Self::resource_part(&resource)?,
                    { "text": Self::prompt_for_method(method, req) }
                ]
            }],
            "generationConfig": { "responseMimeType": "application/json" }
        })
        .as_object()
        .cloned()
        .unwrap_or_default();
        let (status, body, latency_ms) = self
            .post_generate_content(provider_model, &request_obj)
            .await?;
        if !status.is_success() {
            let message = body
                .pointer("/error/message")
                .and_then(|value| value.as_str())
                .unwrap_or("google gimini vision returned non-success status");
            return Err(Self::classify_api_error(status, message.to_string()));
        }
        let text = Self::extract_text_content(&body);
        let parsed = text
            .as_ref()
            .and_then(|value| serde_json::from_str::<Value>(value).ok())
            .unwrap_or_else(|| json!({ "text": text }));
        let extra_key = match method {
            ai_methods::VISION_OCR => "ocr",
            ai_methods::VISION_DETECT => "detections",
            ai_methods::VISION_SEGMENT => "segments",
            _ => "captions",
        };
        let mut extra = Map::new();
        extra.insert(extra_key.to_string(), parsed);
        extra.insert("latency_ms".to_string(), Value::from(latency_ms));
        extra.insert(
            "provider_io".to_string(),
            json!({ "input": request_obj, "output": body }),
        );
        Ok(ProviderStartResult::Immediate(AiResponseSummary {
            text,
            finish_reason: Some("stop".to_string()),
            extra: Some(Value::Object(extra)),
            ..Default::default()
        }))
    }

    async fn start_audio_media(
        &self,
        ctx: &crate::aicc::InvokeCtx,
        provider_model: &str,
        method: &str,
        req: &AiMethodRequest,
    ) -> Result<ProviderStartResult, ProviderError> {
        let prompt = Self::prompt_for_method(method, req);
        let request_obj = json!({
            "contents": [{
                "role": "user",
                "parts": [{ "text": prompt }]
            }],
            "generationConfig": { "responseModalities": ["AUDIO"] }
        })
        .as_object()
        .cloned()
        .unwrap_or_default();
        let (status, body, latency_ms) = self
            .post_generate_content(provider_model, &request_obj)
            .await?;
        if !status.is_success() {
            let message = body
                .pointer("/error/message")
                .and_then(|value| value.as_str())
                .unwrap_or("google gimini audio returned non-success status");
            return Err(Self::classify_api_error(status, message.to_string()));
        }
        let artifacts = Self::parse_media_artifacts(&body, "audio/mpeg");
        let mut extra = Map::new();
        extra.insert("latency_ms".to_string(), Value::from(latency_ms));
        extra.insert(
            "provider_io".to_string(),
            json!({ "input": request_obj, "output": body }),
        );
        Ok(ProviderStartResult::Immediate(AiResponseSummary {
            artifacts,
            finish_reason: Some("stop".to_string()),
            extra: Some(Value::Object(extra)),
            ..Default::default()
        }))
    }

    async fn start_video(
        &self,
        ctx: &crate::aicc::InvokeCtx,
        provider_model: &str,
        method: &str,
        req: &AiMethodRequest,
    ) -> Result<ProviderStartResult, ProviderError> {
        let mut instance = Map::new();
        instance.insert(
            "prompt".to_string(),
            Value::String(Self::prompt_for_method(method, req)),
        );
        if let Some(resource) = req
            .payload
            .resources
            .first()
            .cloned()
            .or_else(|| Self::resource_from_input_json(req, &["image", "video"]))
        {
            instance.insert("input".to_string(), Self::resource_part(&resource)?);
        }
        if let Some(handle) = req
            .payload
            .input_json
            .as_ref()
            .and_then(|value| value.get("continuation_handle"))
            .cloned()
        {
            instance.insert("continuation_handle".to_string(), handle);
        }
        let mut request_obj = Map::new();
        request_obj.insert(
            "instances".to_string(),
            Value::Array(vec![Value::Object(instance)]),
        );
        if let Some(options) = req
            .payload
            .options
            .clone()
            .or_else(|| req.payload.input_json.clone())
        {
            request_obj.insert("parameters".to_string(), options);
        }
        let (status, body, latency_ms) = self
            .post_model_action(provider_model, "predictLongRunning", &request_obj)
            .await?;
        if !status.is_success() {
            let message = body
                .pointer("/error/message")
                .and_then(|value| value.as_str())
                .unwrap_or("google gimini video returned non-success status");
            return Err(Self::classify_api_error(status, message.to_string()));
        }
        let mut extra = Map::new();
        extra.insert(
            "provider".to_string(),
            Value::String("google_gemini".to_string()),
        );
        extra.insert("method".to_string(), Value::String(method.to_string()));
        extra.insert(
            "model".to_string(),
            Value::String(provider_model.to_string()),
        );
        extra.insert("latency_ms".to_string(), Value::from(latency_ms));
        if let Some(name) = body.get("name").cloned() {
            extra.insert("operation_name".to_string(), name.clone());
            if method == ai_methods::VIDEO_EXTEND {
                extra.insert("continuation_handle".to_string(), name);
            }
        }
        extra.insert(
            "provider_io".to_string(),
            json!({ "input": request_obj, "output": body }),
        );
        Ok(ProviderStartResult::Immediate(AiResponseSummary {
            provider_task_ref: extra
                .get("operation_name")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string()),
            finish_reason: Some("started".to_string()),
            extra: Some(Value::Object(extra)),
            ..Default::default()
        }))
    }
}

#[async_trait]
impl Provider for GoogleGiminiProvider {
    fn inventory(&self) -> ProviderInventory {
        self.inventory
            .read()
            .map(|inventory| inventory.clone())
            .unwrap_or_else(|_| {
                Self::build_inventory_from_buckets(
                    self.provider_instance_name.as_str(),
                    self.provider_type.clone(),
                    self.provider_driver.as_str(),
                    &GiminiModelBuckets::default(),
                    self.features.as_slice(),
                    Some("inventory-lock-poisoned".to_string()),
                )
            })
    }

    fn estimate_cost(&self, input: &CostEstimateInput) -> CostEstimateOutput {
        let provider_model = provider_model_from_exact(input.exact_model.as_str());
        if matches!(
            input.api_type,
            ApiType::ImageTextToImage | ApiType::ImageToImage
        ) {
            return CostEstimateOutput {
                estimated_cost_usd: 0.04,
                pricing_mode: PricingMode::PerToken,
                quota_state: QuotaState::Normal,
                confidence: 0.5,
                estimated_latency_ms: Some(6000),
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
                self.start_image2image(&ctx, provider_model.as_str(), &req.request)
                    .await
            }
            ai_methods::EMBEDDING_TEXT => {
                self.start_embedding(&ctx, provider_model.as_str(), &req.request, false)
                    .await
            }
            ai_methods::EMBEDDING_MULTIMODAL => {
                self.start_embedding(&ctx, provider_model.as_str(), &req.request, true)
                    .await
            }
            ai_methods::VISION_OCR
            | ai_methods::VISION_CAPTION
            | ai_methods::VISION_DETECT
            | ai_methods::VISION_SEGMENT => {
                self.start_vision(
                    &ctx,
                    provider_model.as_str(),
                    req.method.as_str(),
                    &req.request,
                )
                .await
            }
            ai_methods::AUDIO_TTS | ai_methods::AUDIO_MUSIC => {
                self.start_audio_media(
                    &ctx,
                    provider_model.as_str(),
                    req.method.as_str(),
                    &req.request,
                )
                .await
            }
            ai_methods::VIDEO_TXT2VIDEO
            | ai_methods::VIDEO_IMG2VIDEO
            | ai_methods::VIDEO_VIDEO2VIDEO
            | ai_methods::VIDEO_EXTEND => {
                self.start_video(
                    &ctx,
                    provider_model.as_str(),
                    req.method.as_str(),
                    &req.request,
                )
                .await
            }
            method => Err(ProviderError::fatal(format!(
                "google gimini provider does not support method '{}'",
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

fn json_text_len(value: &Value) -> usize {
    match value {
        Value::String(text) => text.len(),
        Value::Array(items) => items.iter().map(json_text_len).sum(),
        Value::Object(map) => map.values().map(json_text_len).sum(),
        _ => 0,
    }
}

#[derive(Debug, Deserialize, Default)]
struct GiminiSettings {
    #[serde(default = "default_gimini_enabled")]
    enabled: bool,
    #[serde(default, alias = "api_key", alias = "apiKey")]
    api_token: String,
    #[serde(default)]
    alias_map: HashMap<String, String>,
    #[serde(default)]
    instances: Vec<SettingsGoogleGiminiInstanceConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct SettingsGoogleGiminiInstanceConfig {
    #[serde(default = "default_instance_id")]
    provider_instance_name: String,
    #[serde(default = "default_provider_type")]
    provider_type: String,
    #[serde(default = "default_provider_driver")]
    provider_driver: String,
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

fn default_gimini_enabled() -> bool {
    true
}

fn default_instance_id() -> String {
    "google-gemini-default".to_string()
}

fn default_provider_type() -> String {
    "cloud_api".to_string()
}

fn default_provider_driver() -> String {
    "google-gemini".to_string()
}

fn default_base_url() -> String {
    DEFAULT_GIMINI_BASE_URL.to_string()
}

fn default_timeout_ms() -> u64 {
    DEFAULT_GIMINI_TIMEOUT_MS
}

fn default_features() -> Vec<String> {
    vec![
        features::PLAN.to_string(),
        features::JSON_OUTPUT.to_string(),
    ]
}

fn is_text2image_model_name(model: &str) -> bool {
    let lowered = model.trim().to_ascii_lowercase();
    lowered.contains("image") || lowered.contains("nano-banana") || lowered.contains("imagen")
}

#[derive(Debug, Clone, Copy)]
enum GiminiModelKind {
    Llm,
    Image,
    Embedding,
    Tts,
    Music,
    Video,
}

fn strip_gimini_model_prefix(name: &str) -> &str {
    name.strip_prefix("models/")
        .or_else(|| name.strip_prefix("tunedModels/"))
        .unwrap_or(name)
}

fn classify_gimini_model(id: &str, methods: &HashSet<String>) -> Option<GiminiModelKind> {
    let lowered = id.to_ascii_lowercase();

    if lowered.contains("embedding") || methods.contains("embedcontent") {
        return Some(GiminiModelKind::Embedding);
    }
    if lowered.contains("tts") {
        return Some(GiminiModelKind::Tts);
    }
    if lowered.contains("lyria") {
        return Some(GiminiModelKind::Music);
    }
    if lowered.contains("veo") {
        return Some(GiminiModelKind::Video);
    }
    if is_text2image_model_name(id) {
        return Some(GiminiModelKind::Image);
    }
    if lowered.starts_with("gemini")
        && (methods.contains("generatecontent") || methods.is_empty())
    {
        return Some(GiminiModelKind::Llm);
    }
    None
}

fn gimini_method_mounts(api_type: ApiType, provider_model_id: &str) -> Vec<String> {
    let base = match api_type {
        ApiType::Embedding => "embedding.text",
        ApiType::EmbeddingMultimodal => "embedding.multimodal",
        ApiType::ImageToImage => "image.img2img",
        ApiType::VisionOcr => "vision.ocr",
        ApiType::VisionCaption => "vision.caption",
        ApiType::VisionDetect => "vision.detect",
        ApiType::VisionSegment => "vision.segment",
        ApiType::AudioTts => "audio.tts",
        ApiType::AudioMusic => "audio.music",
        ApiType::VideoTextToVideo => "video.txt2video",
        ApiType::VideoImageToVideo => "video.img2video",
        ApiType::VideoToVideo => "video.video2video",
        ApiType::VideoExtend => "video.extend",
        _ => api_type.namespace(),
    };
    let model = logical_mount_segment(provider_model_id);
    vec![
        base.to_string(),
        format!("{}.google", base),
        format!("{}.gemini", base),
        format!("{}.{}", base, model),
    ]
}

fn dedupe_strings(values: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::<String>::new();
    let mut result = Vec::new();
    for value in values {
        if seen.insert(value.clone()) {
            result.push(value);
        }
    }
    result
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

fn parse_gimini_settings(settings: &Value) -> Result<Option<GiminiSettings>> {
    let raw = settings
        .get("gemini")
        .or_else(|| settings.get("google_gemini"))
        .or_else(|| settings.get("gimini"))
        .or_else(|| settings.get("google_gimini"))
        .or_else(|| settings.get("google"));
    let Some(raw_settings) = raw else {
        return Ok(None);
    };
    if raw_settings.is_null() {
        return Ok(None);
    }

    let gimini_settings = serde_json::from_value::<GiminiSettings>(raw_settings.clone())
        .map_err(|err| anyhow!("failed to parse gimini settings: {}", err))?;
    if !gimini_settings.enabled {
        return Ok(None);
    }

    Ok(Some(gimini_settings))
}

fn normalize_legacy_gemini_instance_name(value: String) -> String {
    if value == "google-gimini-default" {
        "google-gemini-default".to_string()
    } else {
        value
    }
}

fn normalize_legacy_gemini_driver(value: String) -> String {
    if value == "google-gimini" {
        "google-gemini".to_string()
    } else {
        value
    }
}

fn build_gimini_instances(settings: &GiminiSettings) -> Result<Vec<GoogleGiminiInstanceConfig>> {
    let raw_instances = if settings.instances.is_empty() {
        vec![SettingsGoogleGiminiInstanceConfig {
            provider_instance_name: default_instance_id(),
            provider_type: default_provider_type(),
            provider_driver: default_provider_driver(),
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
            models = normalize_model_list(parse_csv_list(DEFAULT_GIMINI_MODELS));
        }
        if models.is_empty() {
            return Err(anyhow!(
                "gimini instance {} has no models configured",
                raw_instance.provider_instance_name
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
            image_models = normalize_model_list(parse_csv_list(DEFAULT_GIMINI_IMAGE_MODELS));
        }
        let default_image_model = raw_instance
            .default_image_model
            .or_else(|| image_models.first().cloned());
        let features = if raw_instance.features.is_empty() {
            default_features()
        } else {
            raw_instance.features
        };

        instances.push(GoogleGiminiInstanceConfig {
            provider_instance_name: normalize_legacy_gemini_instance_name(
                raw_instance.provider_instance_name,
            ),
            provider_type: raw_instance.provider_type,
            provider_driver: normalize_legacy_gemini_driver(raw_instance.provider_driver),
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
            "text2image.nano_banana",
            "t2i.nano_banana",
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

pub fn register_google_gimini_providers(
    center: &AIComputeCenter,
    settings: &Value,
) -> Result<usize> {
    let Some(gimini_settings) = parse_gimini_settings(settings)? else {
        info!("aicc google gimini provider is disabled (gimini settings missing or disabled)");
        return Ok(0);
    };
    if gimini_settings.api_token.trim().is_empty() {
        return Err(anyhow!(
            "gimini.api_token (or api_key) is required when gimini provider is enabled"
        ));
    }

    let instances = build_gimini_instances(&gimini_settings)?;
    let mut prepared = Vec::<(GoogleGiminiInstanceConfig, Arc<GoogleGiminiProvider>)>::new();
    for config in instances.iter() {
        let provider = Arc::new(GoogleGiminiProvider::new(
            config.clone(),
            gimini_settings.api_token.clone(),
        )?);
        provider.clone().start_inventory_refresh();
        prepared.push((config.clone(), provider));
    }

    for (config, provider) in prepared.into_iter() {
        let inventory = center.registry().add_provider(provider);
        info!(
            "registered google gimini base_url={} inventory={:?}",
            config.base_url, inventory
        );
        center
            .model_registry()
            .write()
            .map_err(|_| anyhow!("model registry lock poisoned"))?
            .apply_inventory(inventory)
            .map_err(|err| anyhow!("failed to apply gimini inventory: {}", err))?;
    }

    Ok(instances.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aicc::ModelCatalog;
    use buckyos_api::{AiPayload, ModelSpec, Requirements};
    use serde_json::json;

    fn build_text2image_request(options: Option<Value>) -> AiMethodRequest {
        AiMethodRequest::new(
            Capability::Image,
            ModelSpec::new("text2image.default".to_string(), None),
            Requirements::default(),
            AiPayload::new(
                Some("draw a banana".to_string()),
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
    fn build_gimini_instances_infers_image_models() {
        let settings = GiminiSettings {
            enabled: true,
            api_token: "token".to_string(),
            alias_map: HashMap::new(),
            instances: vec![SettingsGoogleGiminiInstanceConfig {
                provider_instance_name: "gimini-1".to_string(),
                provider_type: "cloud_api".to_string(),
                provider_driver: "google-gimini".to_string(),
                base_url: default_base_url(),
                timeout_ms: default_timeout_ms(),
                models: vec![
                    "gemini-2.5-flash".to_string(),
                    "gemini-2.0-flash-exp-image-generation".to_string(),
                ],
                default_model: None,
                image_models: vec![],
                default_image_model: None,
                features: vec![],
                alias_map: HashMap::new(),
            }],
        };

        let instances = build_gimini_instances(&settings).expect("instances should be built");
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].provider_driver, "google-gemini");
        assert_eq!(
            instances[0].default_model.as_deref(),
            Some("gemini-2.5-flash")
        );
        assert_eq!(
            instances[0].default_image_model.as_deref(),
            Some("gemini-2.0-flash-exp-image-generation")
        );
    }

    #[test]
    fn build_gimini_instances_uses_gemini_default_names() {
        let settings = GiminiSettings {
            enabled: true,
            api_token: "token".to_string(),
            alias_map: HashMap::new(),
            instances: vec![],
        };

        let instances = build_gimini_instances(&settings).expect("instances should be built");
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].provider_instance_name, "google-gemini-default");
        assert_eq!(instances[0].provider_driver, "google-gemini");
    }

    #[test]
    fn register_gimini_inventory_exposes_stable_gemini_mounts() {
        let center = AIComputeCenter::default();
        let settings = json!({
            "gemini": {
                "enabled": true,
                "api_token": "token",
                "instances": [
                    {
                        "provider_instance_name": "google-gimini-default",
                        "provider_type": "cloud_api",
                        "provider_driver": "google-gimini",
                        "base_url": "https://generativelanguage.googleapis.com/v1beta",
                        "models": ["gemini-2.5-flash", "gemini-2.5-pro"],
                        "image_models": ["gemini-2.5-flash-image-preview"]
                    }
                ]
            }
        });

        let count =
            register_google_gimini_providers(&center, &settings).expect("register should work");
        assert_eq!(count, 1);

        let registry = center.model_registry().read().expect("model registry lock");
        let flash_items = registry.default_items_for_path("llm.gemini-flash");
        assert!(flash_items
            .values()
            .any(|item| { item.target == "gemini-2.5-flash@google-gemini-default" }));
        let pro_items = registry.default_items_for_path("llm.gemini-pro");
        assert!(pro_items
            .values()
            .any(|item| item.target == "gemini-2.5-pro@google-gemini-default"));
        let legacy_items = registry.default_items_for_path("llm.gimini");
        assert!(legacy_items.is_empty());
        let image_items = registry.default_items_for_path("image.txt2img.gemini");
        assert!(image_items
            .values()
            .any(|item| { item.target == "gemini-2.5-flash-image-preview@google-gemini-default" }));
        let split_model_items = registry.default_items_for_path("image.img2img.gemini-2");
        assert!(split_model_items.is_empty());
    }

    #[test]
    fn register_custom_aliases_routes_text2image_prefix() {
        let center = AIComputeCenter::new(Default::default(), ModelCatalog::default());
        let aliases = HashMap::from([
            (
                "llm.plan.default".to_string(),
                "gemini-2.5-flash".to_string(),
            ),
            (
                "text2image.nano_banana".to_string(),
                "gemini-2.0-flash-exp-image-generation".to_string(),
            ),
        ]);
        register_custom_aliases(&center, "google-gemini", &aliases);

        let llm = center.model_catalog().resolve(
            "",
            &Capability::Llm,
            "llm.plan.default",
            "google-gemini",
        );
        let image = center.model_catalog().resolve(
            "",
            &Capability::Image,
            "text2image.nano_banana",
            "google-gemini",
        );
        assert_eq!(llm.as_deref(), Some("gemini-2.5-flash"));
        assert_eq!(
            image.as_deref(),
            Some("gemini-2.0-flash-exp-image-generation")
        );
    }

    #[test]
    fn register_default_aliases_exposes_code_default_not_json_default() {
        let center = AIComputeCenter::new(Default::default(), ModelCatalog::default());
        let models = vec!["gemini-2.5-flash".to_string()];
        let image_models = Vec::<String>::new();
        register_default_aliases(
            &center,
            "google-gemini",
            &models,
            Some("gemini-2.5-flash"),
            &image_models,
            None,
        );

        let code_alias = center.model_catalog().resolve(
            "",
            &Capability::Llm,
            "llm.code.default",
            "google-gemini",
        );
        let removed_alias = center.model_catalog().resolve(
            "",
            &Capability::Llm,
            "llm.json.default",
            "google-gemini",
        );

        assert_eq!(code_alias.as_deref(), Some("gemini-2.5-flash"));
        assert!(removed_alias.is_none());
    }

    #[test]
    fn estimate_text2image_cost_covers_current_image_models() {
        let preview = build_text2image_request(Some(json!({ "n": 2 })));
        assert_eq!(
            GoogleGiminiProvider::estimate_text2image_cost(
                &preview,
                "gemini-2.5-flash-image-preview"
            ),
            Some(0.078)
        );

        let legacy = build_text2image_request(None);
        assert_eq!(
            GoogleGiminiProvider::estimate_text2image_cost(
                &legacy,
                "gemini-2.0-flash-exp-image-generation"
            ),
            Some(0.03)
        );
    }

    #[test]
    fn parse_text2image_result_supports_inline_data() {
        let body = json!({
            "candidates": [
                {
                    "content": {
                        "parts": [
                            {
                                "inlineData": {
                                    "mimeType": "image/png",
                                    "data": "aGVsbG8="
                                }
                            }
                        ]
                    }
                }
            ]
        });

        let (artifacts, text) =
            GoogleGiminiProvider::parse_text2image_result(&body).expect("artifacts should parse");
        assert_eq!(artifacts.len(), 1);
        assert!(text.is_none());
        match &artifacts[0].resource {
            ResourceRef::Base64 { data_base64, .. } => assert_eq!(data_base64, "aGVsbG8="),
            other => panic!("unexpected resource: {:?}", other),
        }
    }
}
