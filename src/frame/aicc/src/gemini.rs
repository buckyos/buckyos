use crate::aicc::{
    AIComputeCenter, Provider, ProviderError, ProviderInstance, ProviderRefreshTask,
    ProviderStartResult, ResolvedRequest, TaskEventSink, emit_background_provider_result,
    provider_type_from_settings, redacted_json_log,
};
use crate::metadata_resolver::{DriverModelResolveRequest, resolve_driver_inventory};
use crate::model_types::{
    ApiType, CostEstimateInput, CostEstimateOutput, PricingMode, ProviderInventory, ProviderOrigin,
    ProviderType, ProviderTypeTrustedSource, QuotaState,
};
use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose;
#[cfg(test)]
use buckyos_api::Capability;
use buckyos_api::{
    AiArtifact, AiContent, AiCost, AiMessage, AiMethodRequest, AiResponse, AiRole, AiToolCall,
    AiToolResultContent, AiToolSpec, AiUsage, Feature, ResourceRef, ai_methods, features,
    value_to_object_map,
};
use log::{info, warn};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::Duration;
use tokio::sync::watch;
use tokio::time;

const DEFAULT_GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";
const DEFAULT_GEMINI_TIMEOUT_MS: u64 = 300_000;
const DEFAULT_GEMINI_MODELS: &str = "gemini-3.7-flash,gemini-3.6-flash,gemini-3.5-flash,gemini-3.5-flash-lite,gemini-3.1-flash-lite,gemini-3.1-pro-preview,gemini-3-flash-preview,gemini-2.5-flash,gemini-2.5-flash-lite,gemini-2.5-pro,gemini-2.5-computer-use-preview-10-2025,gemini-3.5-transcribe,gemini-robotics-er-2-preview";
const DEFAULT_GEMINI_IMAGE_MODELS: &str =
    "gemini-3.1-flash-image,gemini-3.1-flash-lite-image,gemini-3-pro-image,gemini-2.5-flash-image";
const DEFAULT_GEMINI_EMBEDDING_MODELS: &str = "gemini-embedding-001,gemini-embedding-2";
const DEFAULT_GEMINI_TTS_MODELS: &str =
    "gemini-3.1-flash-tts-preview,gemini-2.5-flash-preview-tts,gemini-2.5-pro-preview-tts";
const DEFAULT_GEMINI_MUSIC_MODELS: &str = "lyria-3-clip-preview,lyria-3-pro-preview";
const DEFAULT_GEMINI_VIDEO_MODELS: &str = "gemini-omni-1.1-flash,gemini-omni-flash-preview,veo-3.1-fast-generate-preview,veo-3.1-generate-preview,veo-3.1-lite-generate-preview";

const DEFAULT_GEMINI_INVENTORY_REFRESH_INTERVAL: Duration = Duration::from_secs(300);
const GEMINI_VIDEO_POLL_INTERVAL: Duration = Duration::from_secs(2);
const GEMINI_VIDEO_MAX_WAIT: Duration = Duration::from_secs(600);
const GEMINI_MODELS_PAGE_SIZE: u32 = 1000;
const GEMINI_MODELS_MAX_PAGES: usize = 10;
const GEMINI_INTERACTIONS_API_REVISION: &str = "2026-05-20";
const MIN_RELIABLE_ASR_CONFIDENCE: f64 = 0.75;

#[derive(Debug, PartialEq)]
struct GeminiAsrAssessment {
    transcript: String,
    candidate_text: String,
    segments: Value,
    speech_detected: bool,
    confidence: f64,
    status: &'static str,
}

fn assess_gemini_asr(parsed: &Value) -> GeminiAsrAssessment {
    let candidate_text = parsed
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let segments = parsed
        .get("segments")
        .cloned()
        .unwrap_or_else(|| Value::Array(vec![]));
    let speech_detected = parsed
        .get("speech_detected")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let confidence = parsed
        .get("transcript_confidence")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    let reliable =
        speech_detected && !candidate_text.is_empty() && confidence >= MIN_RELIABLE_ASR_CONFIDENCE;
    let status = if reliable {
        "reliable"
    } else if speech_detected {
        "uncertain"
    } else {
        "no_speech"
    };

    GeminiAsrAssessment {
        transcript: if reliable {
            candidate_text.clone()
        } else {
            String::new()
        },
        candidate_text,
        segments,
        speech_detected,
        confidence,
        status,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GeminiToolCallContext {
    name: String,
    provider_call_id: Option<String>,
}

const GEMINI_IMAGE_INPUT_ALLOWLIST: &[&str] = &[
    "aspect_ratio",
    "candidate_count",
    "max_output_tokens",
    "n",
    "negative_prompt",
    "output",
    "prompt",
    "quality",
    "response_mime_type",
    "response_modalities",
    "seed",
    "size",
    "stop",
    "temperature",
    "top_k",
    "top_p",
];
const GEMINI_IMAGE_OPTION_ALLOWLIST: &[&str] = &[
    "aspect_ratio",
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
const GEMINI_VIDEO_PARAMETER_ALLOWLIST: &[&str] = &[
    "aspect_ratio",
    "compression_quality",
    "duration",
    "duration_seconds",
    "enhance_prompt",
    "generate_audio",
    "negative_prompt",
    "person_generation",
    "resolution",
    "resize_mode",
    "sample_count",
    "seed",
];

const GEMINI_VIDEO_CONSUMED_INPUT_KEYS: &[&str] = &[
    "continuation_handle",
    "image",
    "output",
    "prompt",
    "resource_format",
    "response_format",
    "video",
];

#[derive(Debug, Clone)]
pub struct GoogleGeminiInstanceConfig {
    pub provider_instance_name: String,
    pub provider_type: String,
    pub provider_driver: String,
    pub api_token: String,
    pub base_url: String,
    pub timeout_ms: u64,
    pub models: Vec<String>,
    #[allow(dead_code)]
    pub default_model: Option<String>,
    pub image_models: Vec<String>,
    #[allow(dead_code)]
    pub default_image_model: Option<String>,
    pub features: Vec<Feature>,
    #[allow(dead_code)]
    pub alias_map: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct GoogleGeminiProvider {
    instance: ProviderInstance,
    inventory: Arc<RwLock<ProviderInventory>>,
    client: Client,
    api_token: String,
    base_url: String,
    provider_type: ProviderType,
    provider_driver: String,
    provider_instance_name: String,
    features: Vec<Feature>,
    refresh_task: Arc<Mutex<Option<Arc<ProviderRefreshTask>>>>,
}

#[derive(Debug, Default)]
struct GeminiModelBuckets {
    llm: Vec<String>,
    image: Vec<String>,
    embedding: Vec<String>,
    tts: Vec<String>,
    music: Vec<String>,
    video: Vec<String>,
}

impl GeminiModelBuckets {
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
            "gemini-models-{}-{}-{}-{}-{}-{}-{:x}",
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
struct GeminiModelsResponse {
    #[serde(default)]
    models: Vec<GeminiModelEntry>,
    #[serde(default, alias = "nextPageToken")]
    next_page_token: Option<String>,
}

fn next_gemini_models_page_token(
    next_page_token: Option<String>,
    seen_tokens: &mut HashSet<String>,
) -> Result<Option<String>> {
    let Some(token) = next_page_token
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    if !seen_tokens.insert(token.clone()) {
        return Err(anyhow!("gemini models pagination repeated next_page_token"));
    }
    Ok(Some(token))
}

#[derive(Debug, Deserialize)]
struct GeminiModelEntry {
    #[serde(default)]
    name: String,
    #[serde(default, alias = "supportedGenerationMethods")]
    supported_generation_methods: Vec<String>,
    // Google `/v1beta/models` 不会用一个独立字段标 deprecation，但 displayName /
    // description 里经常有 "(Discontinued)" / "Deprecated." / "no longer" 之类的
    // 字眼，我们用它过滤掉刷出来其实已经不能用的模型。
    #[serde(default, alias = "displayName")]
    display_name: String,
    #[serde(default)]
    description: String,
}

impl GoogleGeminiProvider {
    pub fn new(cfg: GoogleGeminiInstanceConfig, api_token: String) -> Result<Self> {
        let timeout_ms = if cfg.timeout_ms == 0 {
            DEFAULT_GEMINI_TIMEOUT_MS
        } else {
            cfg.timeout_ms
        };

        let client = Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .context("failed to build reqwest client for google gemini provider")?;

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
            endpoint: Some(cfg.base_url.clone()),
            plugin_key: None,
        };
        let buckets = GeminiModelBuckets {
            llm: cfg
                .models
                .iter()
                .filter(|model| !is_text2image_model_name(model))
                .cloned()
                .collect(),
            image: cfg.image_models.clone(),
            embedding: parse_csv_list(DEFAULT_GEMINI_EMBEDDING_MODELS),
            tts: parse_csv_list(DEFAULT_GEMINI_TTS_MODELS),
            music: parse_csv_list(DEFAULT_GEMINI_MUSIC_MODELS),
            video: parse_csv_list(DEFAULT_GEMINI_VIDEO_MODELS),
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
            refresh_task: Arc::new(Mutex::new(None)),
        })
    }

    fn build_inventory_from_buckets(
        provider_instance_name: &str,
        provider_type: ProviderType,
        provider_driver: &str,
        buckets: &GeminiModelBuckets,
        _features: &[Feature],
        inventory_revision: Option<String>,
    ) -> ProviderInventory {
        let mut requests = Vec::<DriverModelResolveRequest>::new();
        for model in buckets.llm.iter() {
            requests.push(
                DriverModelResolveRequest::new(model.clone(), vec![ApiType::Llm])
                    .with_cost(Some(0.01))
                    .with_latency(Some(1400)),
            );
        }
        for model in buckets.image.iter() {
            requests.push(
                DriverModelResolveRequest::new(
                    model.clone(),
                    vec![ApiType::ImageTextToImage, ApiType::ImageToImage],
                )
                .with_cost(Some(0.04))
                .with_latency(Some(6000)),
            );
        }
        for model in buckets.embedding.iter() {
            requests.push(
                DriverModelResolveRequest::new(model.clone(), vec![ApiType::Embedding])
                    .with_cost(Some(0.0001))
                    .with_latency(Some(800)),
            );
        }
        for model in buckets.tts.iter() {
            requests.push(
                DriverModelResolveRequest::new(model.clone(), vec![ApiType::AudioTts])
                    .with_cost(Some(0.01))
                    .with_latency(Some(3000)),
            );
        }
        for model in buckets.music.iter() {
            requests.push(
                DriverModelResolveRequest::new(model.clone(), vec![ApiType::AudioMusic])
                    .with_cost(Some(0.10))
                    .with_latency(Some(60_000)),
            );
        }
        for model in buckets.video.iter() {
            requests.push(
                DriverModelResolveRequest::new(
                    model.clone(),
                    vec![ApiType::VideoTextToVideo, ApiType::VideoImageToVideo],
                )
                .with_cost(Some(0.50))
                .with_latency(Some(120_000)),
            );
        }
        resolve_driver_inventory(
            provider_instance_name,
            provider_type,
            provider_driver,
            requests.as_slice(),
            inventory_revision,
        )
    }

    fn model_supports_feature_combination(
        &self,
        provider_model: &str,
        combination: &[&str],
    ) -> bool {
        self.inventory
            .read()
            .ok()
            .and_then(|inventory| {
                inventory
                    .models
                    .iter()
                    .find(|model| model.provider_model_id == provider_model)
                    .map(|model| model.capabilities.supports_feature_combination(combination))
            })
            .unwrap_or(false)
    }

    fn model_max_output_tokens(&self, provider_model: &str) -> Option<u64> {
        self.inventory.read().ok().and_then(|inventory| {
            inventory
                .models
                .iter()
                .find(|model| model.provider_model_id == provider_model)
                .and_then(|model| model.capabilities.max_output_tokens)
        })
    }

    fn thinking_retry_output_limit(
        request_obj: &Map<String, Value>,
        body: &Value,
        completion_tokens: u64,
        model_max_output_tokens: Option<u64>,
    ) -> Option<u64> {
        if body
            .pointer("/candidates/0/finishReason")
            .and_then(Value::as_str)
            != Some("MAX_TOKENS")
        {
            return None;
        }
        let thinking_tokens = body
            .pointer("/usageMetadata/thoughtsTokenCount")
            .and_then(Value::as_u64)
            .filter(|tokens| *tokens > 0)?;
        let current_output_limit = request_obj
            .get("generationConfig")
            .and_then(Value::as_object)
            .and_then(|config| config.get("maxOutputTokens"))
            .and_then(Value::as_u64)?;
        let combined = completion_tokens.saturating_add(thinking_tokens);
        let combined = model_max_output_tokens
            .map(|limit| combined.min(limit))
            .unwrap_or(combined);
        (combined > current_output_limit).then_some(combined)
    }

    fn apply_separate_thinking_budget(
        request_obj: &mut Map<String, Value>,
        model_max_output_tokens: Option<u64>,
    ) -> Option<u64> {
        let generation = request_obj
            .get("generationConfig")
            .and_then(Value::as_object)?;
        let completion_tokens = generation.get("maxOutputTokens")?.as_u64()?;
        let thinking_tokens = generation
            .get("thinkingConfig")
            .and_then(Value::as_object)
            .and_then(|thinking| thinking.get("thinkingBudget"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if thinking_tokens == 0 {
            return Some(completion_tokens);
        }
        let combined = completion_tokens.saturating_add(thinking_tokens);
        let combined = model_max_output_tokens
            .map(|limit| combined.min(limit))
            .unwrap_or(combined);
        Self::ensure_generation_config(request_obj)
            .insert("maxOutputTokens".to_string(), Value::from(combined));
        Some(completion_tokens)
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
                    "aicc.gemini.inventory.refresh_task_lock_poisoned provider_instance_name={}",
                    self.provider_instance_name
                );
                return;
            }
        };
        if let Some(existing) = existing {
            existing.shutdown();
        }
        let provider = Arc::downgrade(&self);
        let provider_instance_name = self.provider_instance_name.clone();
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
                "aicc.gemini.inventory.refresh_stopped provider_instance_name={}",
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
                "aicc.gemini.inventory.initial_refresh_failed provider_instance_name={} err={}",
                provider_instance_name, err
            );
        }
        if refresh_task.is_stopped() {
            info!(
                "aicc.gemini.inventory.refresh_stopped provider_instance_name={}",
                provider_instance_name
            );
            return;
        }

        let mut interval = time::interval(DEFAULT_GEMINI_INVENTORY_REFRESH_INTERVAL);
        interval.tick().await;
        loop {
            tokio::select! {
                biased;
                changed = shutdown_rx.changed() => {
                    if changed.is_err() || *shutdown_rx.borrow() {
                        info!(
                            "aicc.gemini.inventory.refresh_stopped provider_instance_name={}",
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
                            "aicc.gemini.inventory.refresh_failed provider_instance_name={} err={}",
                            provider_instance_name, err
                        );
                    }
                    if refresh_task.is_stopped() {
                        info!(
                            "aicc.gemini.inventory.refresh_stopped provider_instance_name={}",
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
                        "aicc.gemini.inventory.refresh_stop_requested provider_instance_name={}",
                        self.provider_instance_name
                    );
                }
            }
            Err(_) => warn!(
                "aicc.gemini.inventory.refresh_task_lock_poisoned provider_instance_name={}",
                self.provider_instance_name
            ),
        }
    }

    async fn refresh_inventory_once(&self) -> Result<ProviderInventory> {
        let mut buckets = GeminiModelBuckets::default();
        let mut llm_seen = HashSet::<String>::new();
        let mut image_seen = HashSet::<String>::new();
        let mut embedding_seen = HashSet::<String>::new();
        let mut tts_seen = HashSet::<String>::new();
        let mut music_seen = HashSet::<String>::new();
        let mut video_seen = HashSet::<String>::new();
        let mut seen_page_tokens = HashSet::<String>::new();
        let mut page_token: Option<String> = None;

        let endpoint = format!("{}/models", self.base_url);
        let page_size = GEMINI_MODELS_PAGE_SIZE.to_string();
        for page in 0..GEMINI_MODELS_MAX_PAGES {
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
                .context("gemini inventory refresh request failed")?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                return Err(anyhow!(
                    "gemini inventory refresh failed status={} body={}",
                    status,
                    body
                ));
            }

            let parsed = response
                .json::<GeminiModelsResponse>()
                .await
                .context("failed to parse gemini models response")?;

            for entry in parsed.models.iter() {
                let id = strip_gemini_model_prefix(entry.name.as_str()).trim();
                if id.is_empty() {
                    continue;
                }
                if is_deprecated_gemini_entry(id, &entry.display_name, &entry.description) {
                    log::debug!(
                        "aicc.gemini.inventory.skip_deprecated id={} display_name={:?} description={:?}",
                        id,
                        entry.display_name,
                        entry.description
                    );
                    continue;
                }
                let methods = entry
                    .supported_generation_methods
                    .iter()
                    .map(|method| method.to_ascii_lowercase())
                    .collect::<HashSet<_>>();
                let key = id.to_ascii_lowercase();
                match classify_gemini_model(id, &methods) {
                    Some(GeminiModelKind::Llm) => {
                        if llm_seen.insert(key) {
                            buckets.llm.push(id.to_string());
                        }
                    }
                    Some(GeminiModelKind::Image) => {
                        if image_seen.insert(key) {
                            buckets.image.push(id.to_string());
                        }
                    }
                    Some(GeminiModelKind::Embedding) => {
                        if embedding_seen.insert(key) {
                            buckets.embedding.push(id.to_string());
                        }
                    }
                    Some(GeminiModelKind::Tts) => {
                        if tts_seen.insert(key) {
                            buckets.tts.push(id.to_string());
                        }
                    }
                    Some(GeminiModelKind::Music) => {
                        if music_seen.insert(key) {
                            buckets.music.push(id.to_string());
                        }
                    }
                    Some(GeminiModelKind::Video) => {
                        if video_seen.insert(key) {
                            buckets.video.push(id.to_string());
                        }
                    }
                    None => continue,
                }
            }

            let Some(token) =
                next_gemini_models_page_token(parsed.next_page_token, &mut seen_page_tokens)?
            else {
                break;
            };
            page_token = Some(token);
            if page + 1 == GEMINI_MODELS_MAX_PAGES {
                return Err(anyhow!(
                    "gemini models pagination exceeded {} pages",
                    GEMINI_MODELS_MAX_PAGES
                ));
            }
        }

        if buckets.is_empty() {
            return Err(anyhow!(
                "gemini inventory refresh returned no supported models"
            ));
        }

        // Google `/v1beta/models` 同时返回 alias（例如 `gemini-2.0-flash-lite`）
        // 和它的版本快照（`gemini-2.0-flash-lite-001`），两者底层是同一个模型，
        // 但 Google 弃用一般是先停版本快照、alias 再续命一段时间。如果两者并存
        // 就把版本快照丢掉，只信 alias，避免路由命中已被 Google 拒绝的快照。
        prefer_alias_over_versioned(&mut buckets.llm);
        prefer_alias_over_versioned(&mut buckets.image);
        prefer_alias_over_versioned(&mut buckets.embedding);
        prefer_alias_over_versioned(&mut buckets.tts);
        prefer_alias_over_versioned(&mut buckets.music);
        prefer_alias_over_versioned(&mut buckets.video);

        // Categories that the API never returns (lyria/veo are typically not
        // listed) fall back to defaults so we don't drop them on refresh.
        if buckets.embedding.is_empty() {
            buckets.embedding = parse_csv_list(DEFAULT_GEMINI_EMBEDDING_MODELS);
        }
        if buckets.tts.is_empty() {
            buckets.tts = parse_csv_list(DEFAULT_GEMINI_TTS_MODELS);
        }
        if buckets.music.is_empty() {
            buckets.music = parse_csv_list(DEFAULT_GEMINI_MUSIC_MODELS);
        }
        if buckets.video.is_empty() {
            buckets.video = parse_csv_list(DEFAULT_GEMINI_VIDEO_MODELS);
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
                .map_err(|_| anyhow!("gemini inventory lock poisoned"))?;
            *current = inventory.clone();
        }
        info!(
            "aicc.gemini.inventory.refreshed provider_instance_name={} llm={} image={} embedding={} tts={} music={} video={}",
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

    fn role_to_gemini(role: &str) -> &'static str {
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
                "google gemini provider does not support base64 resources in this version",
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
                let content = msg.text_content();
                let trimmed = content.trim();
                (!trimmed.is_empty()).then(|| (msg.role.as_str().to_string(), trimmed.to_string()))
            })
            .collect())
    }

    fn build_contents(req: &AiMethodRequest) -> Result<Vec<Value>, ProviderError> {
        let mut contents: Vec<Value> = vec![];
        let mut tool_calls = HashMap::new();

        // 主路径:消费 typed `Vec<AiMessage>`,保留 ToolUse/ToolResult/Image
        // 等 block,转成 Gemini parts(functionCall / functionResponse /
        // inlineData / fileData / text)。
        if !req.payload.messages.is_empty() {
            for msg in req.payload.messages.iter() {
                Self::lower_message_to_gemini(msg, &mut contents, &mut tool_calls)?;
            }
        }

        // 兼容路径:caller 把 messages 塞在 input_json 里,仅做文本降级。
        if contents.is_empty() {
            for (role, content) in Self::canonical_message_texts(req)? {
                contents.push(json!({
                    "role": Self::role_to_gemini(role.as_str()),
                    "parts": [
                        {
                            "text": content
                        }
                    ]
                }));
            }
        }

        if contents.is_empty() {
            let mut content = String::new();
            if let Some(text) = req.payload.text.as_ref() {
                content.push_str(text);
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

        if !req.payload.resources.is_empty() {
            let resource_parts = req
                .payload
                .resources
                .iter()
                .map(Self::content_resource_part)
                .collect::<Result<Vec<_>, _>>()?;
            if let Some(parts) = contents.iter_mut().rev().find_map(|content| {
                let content = content.as_object_mut()?;
                if content.get("role").and_then(Value::as_str) != Some("user") {
                    return None;
                }
                content.get_mut("parts")?.as_array_mut()
            }) {
                parts.extend(resource_parts);
            } else {
                contents.push(json!({ "role": "user", "parts": resource_parts }));
            }
        }

        if contents.is_empty() {
            return Err(ProviderError::fatal(
                "request payload has no usable text/messages for llm",
            ));
        }

        Ok(contents)
    }

    /// Lower a single `AiMessage` to Gemini `Content` shape. Tool results
    /// land in a separate `role: "user"` content with a `functionResponse`
    /// part (Gemini's tool-result convention). System/Developer are folded
    /// into the `user` role — mirroring the legacy `role_to_gemini` default —
    /// because this provider doesn't currently surface `systemInstruction`.
    fn lower_message_to_gemini(
        msg: &AiMessage,
        contents: &mut Vec<Value>,
        tool_calls: &mut HashMap<String, GeminiToolCallContext>,
    ) -> Result<(), ProviderError> {
        match msg.role {
            AiRole::Tool => {
                let Some(AiContent::ToolResult {
                    call_id,
                    content,
                    is_error: _,
                }) = msg.content.first()
                else {
                    return Ok(());
                };
                let context = tool_calls.get(call_id);
                let name = context
                    .map(|context| context.name.clone())
                    .unwrap_or_else(|| call_id.clone());
                let mut function_response = Map::new();
                function_response.insert("name".to_string(), Value::String(name));
                function_response.insert("response".to_string(), Self::tool_result_value(content));
                if let Some(provider_call_id) =
                    context.and_then(|context| context.provider_call_id.as_ref())
                {
                    function_response
                        .insert("id".to_string(), Value::String(provider_call_id.clone()));
                }
                let part = json!({ "functionResponse": function_response });
                contents.push(json!({
                    "role": "user",
                    "parts": [part],
                }));
                Ok(())
            }
            _ => {
                if let Some(provider_content) = Self::gemini_provider_content(&msg.content) {
                    Self::remember_provider_tool_calls(provider_content, tool_calls);
                    contents.push(provider_content.clone());
                    return Ok(());
                }
                let role = match msg.role {
                    AiRole::Assistant => "model",
                    _ => "user",
                };
                let parts = Self::lower_blocks_to_parts(&msg.content, tool_calls)?;
                if !parts.is_empty() {
                    contents.push(json!({
                        "role": role,
                        "parts": parts,
                    }));
                }
                Ok(())
            }
        }
    }

    fn lower_blocks_to_parts(
        content: &[AiContent],
        tool_calls: &mut HashMap<String, GeminiToolCallContext>,
    ) -> Result<Vec<Value>, ProviderError> {
        let mut parts = Vec::with_capacity(content.len());
        for block in content {
            match block {
                AiContent::Text { text } => {
                    if !text.is_empty() {
                        parts.push(json!({ "text": text }));
                    }
                }
                AiContent::Image { source } => {
                    parts.push(Self::resource_to_gemini_part(source, None, true)?);
                }
                AiContent::Document { source, title } => {
                    parts.push(Self::resource_to_gemini_part(
                        source,
                        title.as_deref(),
                        false,
                    )?);
                }
                AiContent::ToolUse {
                    call_id,
                    name,
                    args,
                } => {
                    tool_calls.insert(
                        call_id.clone(),
                        GeminiToolCallContext {
                            name: name.clone(),
                            provider_call_id: Some(call_id.clone()),
                        },
                    );
                    let args_value = serde_json::to_value(args).unwrap_or_else(|_| json!({}));
                    parts.push(json!({
                        "functionCall": {
                            "name": name,
                            "args": args_value,
                            "id": call_id,
                        }
                    }));
                }
                AiContent::Thinking { text, summary, .. } => {
                    // Gemini does not surface a verifier-signed thinking block;
                    // we drop the cryptographic state and keep a textual hint
                    // so the model still sees the reasoning trace.
                    let text_slice = text
                        .as_deref()
                        .filter(|s| !s.is_empty())
                        .or_else(|| summary.as_deref().filter(|s| !s.is_empty()));
                    if let Some(text) = text_slice {
                        parts.push(json!({ "text": text }));
                    }
                }
                AiContent::ProviderState { .. } => {}
                AiContent::ToolResult { .. } => {
                    // Tool role is handled by the caller; ignore defensively.
                }
            }
        }
        Ok(parts)
    }

    fn function_declaration(spec: &AiToolSpec) -> Value {
        let mut declaration = Map::new();
        declaration.insert("name".to_string(), Value::String(spec.name.clone()));
        if !spec.description.trim().is_empty() {
            declaration.insert(
                "description".to_string(),
                Value::String(spec.description.clone()),
            );
        }
        declaration.insert(
            "parametersJsonSchema".to_string(),
            Value::Object(spec.args_schema.clone().into_iter().collect()),
        );
        if !spec.output_schema.is_null()
            && !spec.output_schema.as_object().is_some_and(Map::is_empty)
        {
            declaration.insert("responseJsonSchema".to_string(), spec.output_schema.clone());
        }
        Value::Object(declaration)
    }

    fn merge_llm_tools(
        target: &mut Map<String, Value>,
        provider_model: &str,
        req: &AiMethodRequest,
        combined_tools_supported: bool,
    ) -> Result<(), ProviderError> {
        let web_search_required = req.requirements.requires_feature(features::WEB_SEARCH);
        if web_search_required && !req.payload.tool_specs.is_empty() && !combined_tools_supported {
            return Err(ProviderError::fatal(format!(
                "google gemini model {} does not support combining Google Search with function calling",
                provider_model
            )));
        }
        let mut tools = Vec::new();
        if !req.payload.tool_specs.is_empty() {
            tools.push(json!({
                "functionDeclarations": req
                    .payload
                    .tool_specs
                    .iter()
                    .map(Self::function_declaration)
                    .collect::<Vec<_>>()
            }));
        }
        if web_search_required {
            tools.push(json!({ "googleSearch": {} }));
        }
        if !tools.is_empty() {
            target.insert("tools".to_string(), Value::Array(tools));
        }
        if web_search_required && !req.payload.tool_specs.is_empty() {
            target.insert(
                "toolConfig".to_string(),
                json!({
                    "functionCallingConfig": {
                        "mode": "VALIDATED"
                    },
                    "includeServerSideToolInvocations": true
                }),
            );
        }
        Ok(())
    }

    fn extract_tool_calls(body: &Value) -> Vec<AiToolCall> {
        body.pointer("/candidates/0/content/parts")
            .and_then(Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .enumerate()
                    .filter_map(|(index, part)| {
                        let function = part.get("functionCall")?.as_object()?;
                        let name = function.get("name")?.as_str()?.to_string();
                        let provider_call_id = function
                            .get("id")
                            .and_then(Value::as_str)
                            .filter(|value| !value.trim().is_empty())
                            .map(str::to_string);
                        let call_id =
                            Self::internal_tool_call_id(index, &name, provider_call_id.as_deref());
                        let args = function
                            .get("args")
                            .cloned()
                            .filter(Value::is_object)
                            .unwrap_or_else(|| Value::Object(Map::new()));
                        Some(AiToolCall {
                            call_id,
                            name,
                            args: value_to_object_map(args),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn extract_provider_state(body: &Value) -> Option<AiContent> {
        body.pointer("/candidates/0/content")
            .cloned()
            .map(|value| AiContent::ProviderState {
                provider: "google-gemini".to_string(),
                value,
            })
    }

    fn internal_tool_call_id(index: usize, name: &str, provider_call_id: Option<&str>) -> String {
        provider_call_id
            .map(str::to_string)
            .unwrap_or_else(|| format!("gemini-no-id-{}-{}", index, name))
    }

    fn gemini_provider_content(content: &[AiContent]) -> Option<&Value> {
        content.iter().find_map(|block| match block {
            AiContent::ProviderState { provider, value }
                if (provider.eq_ignore_ascii_case("google")
                    || provider.eq_ignore_ascii_case("gemini")
                    || provider.eq_ignore_ascii_case("google-gemini"))
                    && value.get("parts").and_then(Value::as_array).is_some() =>
            {
                Some(value)
            }
            _ => None,
        })
    }

    fn remember_provider_tool_calls(
        provider_content: &Value,
        tool_calls: &mut HashMap<String, GeminiToolCallContext>,
    ) {
        let Some(parts) = provider_content.get("parts").and_then(Value::as_array) else {
            return;
        };
        for (index, part) in parts.iter().enumerate() {
            let Some(function) = part.get("functionCall").and_then(Value::as_object) else {
                continue;
            };
            let Some(name) = function.get("name").and_then(Value::as_str) else {
                continue;
            };
            let provider_call_id = function
                .get("id")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string);
            let call_id = Self::internal_tool_call_id(index, name, provider_call_id.as_deref());
            tool_calls.insert(
                call_id,
                GeminiToolCallContext {
                    name: name.to_string(),
                    provider_call_id,
                },
            );
        }
    }

    fn resource_to_gemini_part(
        source: &ResourceRef,
        _title: Option<&str>,
        is_image: bool,
    ) -> Result<Value, ProviderError> {
        match source {
            ResourceRef::Url { url, mime_hint } => {
                let mime = mime_hint.clone().unwrap_or_else(|| {
                    if is_image {
                        "image/*".to_string()
                    } else {
                        "application/octet-stream".to_string()
                    }
                });
                Ok(json!({
                    "fileData": {
                        "fileUri": url,
                        "mimeType": mime,
                    }
                }))
            }
            ResourceRef::Base64 { mime, data_base64 } => Ok(json!({
                "inlineData": {
                    "mimeType": mime,
                    "data": data_base64,
                }
            })),
            ResourceRef::NamedObject { obj_id } => Ok(json!({
                "text": format!("named_object: {}", obj_id),
            })),
        }
    }

    fn tool_result_text(content: &[AiToolResultContent]) -> String {
        let mut parts = Vec::new();
        for item in content {
            match item {
                AiToolResultContent::Text { text } => parts.push(text.clone()),
                AiToolResultContent::Image { source } => {
                    parts.push(Self::resource_placeholder_text(source));
                }
                AiToolResultContent::Document { source, title } => {
                    let mut line = Self::resource_placeholder_text(source);
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

    fn tool_result_value(content: &[AiToolResultContent]) -> Value {
        let value = match content {
            [AiToolResultContent::Text { text }] => {
                serde_json::from_str(text.trim()).unwrap_or_else(|_| Value::String(text.clone()))
            }
            _ => Value::String(Self::tool_result_text(content)),
        };
        match value {
            Value::Object(_) => value,
            value => json!({ "output": value }),
        }
    }

    fn resource_placeholder_text(source: &ResourceRef) -> String {
        match source {
            ResourceRef::Url { url, .. } => format!("resource_url: {}", url),
            ResourceRef::NamedObject { obj_id } => format!("named_object: {}", obj_id),
            ResourceRef::Base64 { mime, .. } => format!("inline_{}", mime),
        }
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

    fn api_error_message(body: &Value, fallback: &str) -> String {
        let status = body
            .pointer("/error/status")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let message = body
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or(fallback);
        let mut diagnostics = Vec::new();
        for detail in body
            .pointer("/error/details")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(delay) = detail.get("retryDelay").and_then(Value::as_str) {
                diagnostics.push(format!("retry_after={delay}"));
            }
            for violation in detail
                .get("violations")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(quota_id) = violation.get("quotaId").and_then(Value::as_str) {
                    diagnostics.push(format!("quota_id={quota_id}"));
                }
                if let Some(metric) = violation.get("quotaMetric").and_then(Value::as_str) {
                    diagnostics.push(format!("quota_metric={metric}"));
                }
                if let Some(value) = violation.get("quotaValue") {
                    diagnostics.push(format!("quota_value={value}"));
                }
            }
        }
        diagnostics.sort();
        diagnostics.dedup();
        if diagnostics.is_empty() {
            format!("google gemini api error [{status}]: {message}")
        } else {
            format!(
                "google gemini api error [{status}]: {message} ({})",
                diagnostics.join(", ")
            )
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

    fn merge_tool_choice(
        target: &mut Map<String, Value>,
        value: &Value,
    ) -> Result<(), ProviderError> {
        let (mode, allowed_name) = if let Some(choice) = value.as_str() {
            let mode = match choice {
                "auto" => "AUTO",
                "required" | "any" => "ANY",
                "none" => "NONE",
                _ => {
                    return Err(ProviderError::fatal(format!(
                        "tool_choice '{}' is unsupported",
                        choice
                    )));
                }
            };
            (mode, None)
        } else {
            let choice = value
                .as_object()
                .ok_or_else(|| ProviderError::fatal("tool_choice must be a string or object"))?;
            let name = choice
                .get("function")
                .and_then(Value::as_object)
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
                .or_else(|| choice.get("name").and_then(Value::as_str))
                .filter(|name| !name.trim().is_empty())
                .ok_or_else(|| ProviderError::fatal("tool_choice function name is required"))?;
            ("ANY", Some(name.trim().to_string()))
        };
        let mut config = json!({ "mode": mode });
        if let Some(name) = allowed_name {
            config
                .as_object_mut()
                .expect("function calling config should be an object")
                .insert("allowedFunctionNames".to_string(), json!([name]));
        }
        target.insert(
            "toolConfig".to_string(),
            json!({ "functionCallingConfig": config }),
        );
        Ok(())
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
            if key == "provider_options" {
                ignored.extend(Self::merge_llm_options(
                    target,
                    value,
                    json_output_required,
                )?);
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
                "thinking_budget" | "thinkingBudget" => {
                    let generation = Self::ensure_generation_config(target);
                    let thinking = generation
                        .entry("thinkingConfig".to_string())
                        .or_insert_with(|| json!({}));
                    thinking
                        .as_object_mut()
                        .expect("thinkingConfig should be an object")
                        .insert("thinkingBudget".to_string(), value.clone());
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
                    generation.insert("responseJsonSchema".to_string(), value.clone());
                    if !generation.contains_key("responseMimeType") {
                        generation.insert(
                            "responseMimeType".to_string(),
                            Value::String("application/json".to_string()),
                        );
                    }
                }
                "tool_choice" => Self::merge_tool_choice(target, value)?,
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
            if !GEMINI_IMAGE_INPUT_ALLOWLIST.contains(&key.as_str()) {
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
                    Self::set_image_mime_type(target, value)?;
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
        Self::merge_image_response_preferences(target, input_map)?;
        Ok(())
    }

    fn ensure_image_response_config(target: &mut Map<String, Value>) -> &mut Map<String, Value> {
        let generation = Self::ensure_generation_config(target);
        if !generation
            .get("responseFormat")
            .is_some_and(Value::is_object)
        {
            generation.insert("responseFormat".to_string(), json!({}));
        }
        let response_format = generation
            .get_mut("responseFormat")
            .and_then(Value::as_object_mut)
            .expect("responseFormat should be an object");
        if !response_format.get("image").is_some_and(Value::is_object) {
            response_format.insert("image".to_string(), json!({}));
        }
        response_format
            .get_mut("image")
            .and_then(Value::as_object_mut)
            .expect("responseFormat.image should be an object")
    }

    fn set_image_response_field(target: &mut Map<String, Value>, key: &str, value: Value) {
        Self::ensure_image_response_config(target).insert(key.to_string(), value);
    }

    fn set_image_mime_type(
        target: &mut Map<String, Value>,
        value: &Value,
    ) -> Result<(), ProviderError> {
        let mime = value.as_str().ok_or_else(|| {
            ProviderError::fatal("google gemini image output media type must be a string")
        })?;
        match mime.trim().to_ascii_lowercase().as_str() {
            "image/png" => Ok(()),
            "image/jpeg" | "image/jpg" | "image_jpeg" => {
                Self::set_image_response_field(
                    target,
                    "mimeType",
                    Value::String("IMAGE_JPEG".to_string()),
                );
                Ok(())
            }
            _ => Err(ProviderError::fatal(format!(
                "google gemini image output supports PNG or JPEG, got {}",
                mime
            ))),
        }
    }

    fn gemini_image_size(value: &Value) -> Result<Value, ProviderError> {
        let raw = value
            .as_str()
            .ok_or_else(|| ProviderError::fatal("google gemini image size must be a string"))?;
        let normalized = match raw.trim().to_ascii_uppercase().as_str() {
            "512" => "512",
            "1K" => "1K",
            "2K" => "2K",
            "4K" => "4K",
            _ => {
                let dimensions =
                    raw.trim()
                        .to_ascii_lowercase()
                        .split_once('x')
                        .and_then(|(width, height)| {
                            Some((width.parse::<u32>().ok()?, height.parse::<u32>().ok()?))
                        });
                match dimensions.map(|(width, height)| width.max(height)) {
                    Some(512) => "512",
                    Some(1024) => "1K",
                    Some(2048) => "2K",
                    Some(4096) => "4K",
                    _ => {
                        return Err(ProviderError::fatal(format!(
                            "google gemini image size must use a 512, 1K, 2K or 4K edge, got {}",
                            raw
                        )));
                    }
                }
            }
        };
        Ok(Value::String(normalized.to_string()))
    }

    fn merge_image_response_preferences(
        target: &mut Map<String, Value>,
        input: &Map<String, Value>,
    ) -> Result<(), ProviderError> {
        if let Some(aspect_ratio) = input.get("aspect_ratio") {
            Self::set_image_response_field(target, "aspectRatio", aspect_ratio.clone());
        }
        if let Some(size) = input.get("size") {
            Self::set_image_response_field(target, "imageSize", Self::gemini_image_size(size)?);
        }
        if let Some(media_type) = input
            .get("output")
            .and_then(Value::as_object)
            .and_then(|output| output.get("media_type"))
        {
            Self::set_image_mime_type(target, media_type)?;
        }
        Ok(())
    }

    fn merge_image_prompt_preferences(target: &mut Map<String, Value>, input: &Map<String, Value>) {
        let Some(prompt) = target.get("prompt").and_then(Value::as_str) else {
            return;
        };
        let mut preferences = Vec::new();
        if let Some(negative_prompt) = input
            .get("negative_prompt")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            preferences.push(format!("Avoid: {}", negative_prompt));
        }
        if let Some(quality) = input
            .get("quality")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            preferences.push(format!("Quality preference: {}", quality));
        }
        if !preferences.is_empty() {
            target.insert(
                "prompt".to_string(),
                Value::String(format!("{}\n{}", prompt, preferences.join("\n"))),
            );
        }
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
            if !GEMINI_IMAGE_OPTION_ALLOWLIST.contains(&key.as_str()) && key != "prompt" {
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
                    Self::set_image_mime_type(target, value)?;
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
        Self::merge_image_response_preferences(target, options_map)?;
        Ok(ignored)
    }

    fn normalize_video_parameter_key(key: &str) -> Option<&'static str> {
        match key {
            "aspect_ratio" | "aspectRatio" => Some("aspectRatio"),
            "compression_quality" | "compressionQuality" => Some("compressionQuality"),
            "duration" | "duration_seconds" | "durationSeconds" => Some("durationSeconds"),
            "enhance_prompt" | "enhancePrompt" => Some("enhancePrompt"),
            "generate_audio" | "generateAudio" => Some("generateAudio"),
            "negative_prompt" | "negativePrompt" => Some("negativePrompt"),
            "person_generation" | "personGeneration" => Some("personGeneration"),
            "resolution" => Some("resolution"),
            "resize_mode" | "resizeMode" => Some("resizeMode"),
            "sample_count" | "sampleCount" => Some("sampleCount"),
            "seed" => Some("seed"),
            _ => None,
        }
    }

    fn merge_video_parameters(target: &mut Map<String, Value>, value: &Value) -> Vec<String> {
        let Some(map) = value.as_object() else {
            return vec![];
        };

        let mut ignored = vec![];
        for (key, item) in map.iter() {
            if GEMINI_VIDEO_CONSUMED_INPUT_KEYS.contains(&key.as_str()) {
                continue;
            }
            if !GEMINI_VIDEO_PARAMETER_ALLOWLIST.contains(&key.as_str())
                && Self::normalize_video_parameter_key(key.as_str()).is_none()
            {
                ignored.push(key.clone());
                continue;
            }
            if let Some(normalized) = Self::normalize_video_parameter_key(key.as_str()) {
                target.insert(normalized.to_string(), item.clone());
            }
        }
        ignored
    }

    fn parse_text2image_result(
        body: &Value,
    ) -> Result<(Vec<AiArtifact>, Option<String>), ProviderError> {
        let Some(candidates) = body.get("candidates").and_then(|value| value.as_array()) else {
            return Err(ProviderError::fatal(
                "google gemini image response is missing candidates array",
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
                                "aicc.gemini received invalid inlineData base64 in image response"
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
                "google gemini image response has no usable image outputs",
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
                    ProviderError::retryable(format!("google gemini request failed: {}", err))
                } else {
                    ProviderError::fatal(format!("google gemini request failed: {}", err))
                }
            })?;
        let latency_ms = started_at.elapsed().as_millis() as u64;

        let status = response.status();
        let body: Value = response.json().await.map_err(|err| {
            if status.as_u16() == 429 || status.is_server_error() {
                ProviderError::retryable(format!(
                    "failed to parse google gemini response body: {}",
                    err
                ))
            } else {
                ProviderError::fatal(format!(
                    "failed to parse google gemini response body: {}",
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
                    ProviderError::retryable(format!("google gemini request failed: {}", err))
                } else {
                    ProviderError::fatal(format!("google gemini request failed: {}", err))
                }
            })?;
        let latency_ms = started_at.elapsed().as_millis() as u64;
        let status = response.status();
        let body: Value = response.json().await.map_err(|err| {
            Self::classify_api_error(
                status,
                format!("failed to parse google gemini response body: {}", err),
            )
        })?;
        Ok((status, body, latency_ms))
    }

    async fn post_interaction(
        &self,
        request_obj: &Map<String, Value>,
    ) -> Result<(StatusCode, Value, u64), ProviderError> {
        let url = format!("{}/interactions", self.base_url);
        let started_at = std::time::Instant::now();
        let response = self
            .client
            .post(url.as_str())
            .header("x-goog-api-key", self.api_token.as_str())
            .header("Api-Revision", GEMINI_INTERACTIONS_API_REVISION)
            .json(request_obj)
            .send()
            .await
            .map_err(|err| {
                if err.is_timeout() || err.is_connect() {
                    ProviderError::retryable(format!(
                        "google gemini interactions request failed: {}",
                        err
                    ))
                } else {
                    ProviderError::fatal(format!(
                        "google gemini interactions request failed: {}",
                        err
                    ))
                }
            })?;
        let latency_ms = started_at.elapsed().as_millis() as u64;
        let status = response.status();
        let body = response.json::<Value>().await.map_err(|err| {
            if status.as_u16() == 429 || status.is_server_error() {
                ProviderError::retryable(format!(
                    "failed to parse google gemini interactions response: {}",
                    err
                ))
            } else {
                ProviderError::fatal(format!(
                    "failed to parse google gemini interactions response: {}",
                    err
                ))
            }
        })?;
        Ok((status, body, latency_ms))
    }

    async fn get_video_operation(
        &self,
        operation_name: &str,
    ) -> Result<(StatusCode, Value), ProviderError> {
        let url = if operation_name.starts_with("http://") || operation_name.starts_with("https://")
        {
            operation_name.to_string()
        } else {
            format!(
                "{}/{}",
                self.base_url,
                operation_name.trim_start_matches('/')
            )
        };
        let response = self
            .client
            .get(url)
            .header("x-goog-api-key", self.api_token.as_str())
            .send()
            .await
            .map_err(|err| {
                ProviderError::fatal(format!(
                    "google gemini video status request failed: {}",
                    err
                ))
            })?;
        let status = response.status();
        let body = response.json::<Value>().await.map_err(|err| {
            ProviderError::fatal(format!(
                "failed to parse google gemini video status response: {}",
                err
            ))
        })?;
        Ok((status, body))
    }

    async fn download_video(
        &self,
        url: &str,
    ) -> Result<(StatusCode, Vec<u8>, String), ProviderError> {
        let url = if url.starts_with("http://") || url.starts_with("https://") {
            url.to_string()
        } else {
            format!("{}/{}", self.base_url, url.trim_start_matches('/'))
        };
        let response = self
            .client
            .get(url)
            .header("x-goog-api-key", self.api_token.as_str())
            .send()
            .await
            .map_err(|err| {
                ProviderError::fatal(format!("google gemini video download failed: {}", err))
            })?;
        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("video/mp4")
            .to_string();
        let bytes = response.bytes().await.map_err(|err| {
            ProviderError::fatal(format!(
                "failed to read google gemini video content: {}",
                err
            ))
        })?;
        Ok((status, bytes.to_vec(), content_type))
    }

    fn video_uri(operation: &Value) -> Option<&str> {
        operation
            .pointer("/response/generateVideoResponse/generatedSamples/0/video/uri")
            .or_else(|| operation.pointer("/response/generatedVideos/0/video/uri"))
            .or_else(|| operation.pointer("/response/generated_videos/0/video/uri"))
            .and_then(|value| value.as_str())
    }

    fn video_inline_data(operation: &Value) -> Option<(&str, &str)> {
        let video = operation
            .pointer("/response/generateVideoResponse/generatedSamples/0/video")
            .or_else(|| operation.pointer("/response/generatedVideos/0/video"))
            .or_else(|| operation.pointer("/response/generated_videos/0/video"))?;
        let data = video
            .get("bytesBase64Encoded")
            .or_else(|| video.pointer("/inlineData/data"))
            .and_then(Value::as_str)?;
        let mime = video
            .get("mimeType")
            .or_else(|| video.pointer("/inlineData/mimeType"))
            .and_then(Value::as_str)
            .unwrap_or("video/mp4");
        Some((mime, data))
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

    fn content_resource_part(resource: &ResourceRef) -> Result<Value, ProviderError> {
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
                "google gemini provider cannot resolve named object resource {} without resolver bytes",
                obj_id
            ))),
        }
    }

    fn veo_resource_part(resource: &ResourceRef) -> Result<Value, ProviderError> {
        match resource {
            ResourceRef::Url { url, mime_hint } if url.starts_with("gs://") => Ok(json!({
                "gcsUri": url,
                "mimeType": mime_hint.as_deref().unwrap_or("application/octet-stream")
            })),
            ResourceRef::Url { url, .. } => Err(ProviderError::fatal(format!(
                "google gemini Veo requires base64 data or a gs:// URI, got {}",
                url
            ))),
            ResourceRef::Base64 { mime, data_base64 } => Ok(json!({
                "mimeType": mime,
                "bytesBase64Encoded": data_base64
            })),
            ResourceRef::NamedObject { obj_id } => Err(ProviderError::fatal(format!(
                "google gemini provider cannot resolve named object resource {} without resolver bytes",
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
            ai_methods::AUDIO_ASR => "Transcribe the supplied audio.".to_string(),
            ai_methods::AUDIO_MUSIC => "Generate music from the requested prompt.".to_string(),
            _ => "Process the request.".to_string(),
        }
    }

    fn tts_text(req: &AiMethodRequest) -> Option<&str> {
        req.payload
            .input_json
            .as_ref()
            .and_then(|value| value.get("text"))
            .and_then(Value::as_str)
            .or(req.payload.text.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    fn tts_prompt(req: &AiMethodRequest) -> Result<String, ProviderError> {
        let text = Self::tts_text(req).ok_or_else(|| {
            ProviderError::fatal("audio.tts requires text in input_json.text or payload.text")
        })?;
        let input = req.payload.input_json.as_ref();
        let mut controls = Vec::new();
        if let Some(style) = input
            .and_then(|value| value.get("style"))
            .and_then(Value::as_str)
        {
            controls.push(format!("Style: {}.", style));
        }
        if let Some(gender) = input
            .and_then(|value| value.get("gender"))
            .and_then(Value::as_str)
        {
            controls.push(format!("Voice gender: {}.", gender));
        }
        if let Some(speed) = input
            .and_then(|value| value.get("speed"))
            .and_then(Value::as_f64)
        {
            controls.push(format!("Speaking rate: {}x.", speed));
        }
        if controls.is_empty() {
            Ok(text.to_string())
        } else {
            Ok(format!(
                "{}\nRead the following text exactly:\n{}",
                controls.join(" "),
                text
            ))
        }
    }

    async fn interactions_resource_part(
        &self,
        resource: &ResourceRef,
        media_type: &str,
    ) -> Result<Value, ProviderError> {
        match resource {
            ResourceRef::Base64 { mime, data_base64 } => Ok(json!({
                "type": media_type,
                "mime_type": mime,
                "data": data_base64
            })),
            ResourceRef::Url { url, mime_hint }
                if url.starts_with("http://") || url.starts_with("https://") =>
            {
                let response = self.client.get(url).send().await.map_err(|err| {
                    if err.is_timeout() || err.is_connect() {
                        ProviderError::retryable(format!(
                            "failed to fetch interactions input resource: {err}"
                        ))
                    } else {
                        ProviderError::fatal(format!(
                            "failed to fetch interactions input resource: {err}"
                        ))
                    }
                })?;
                let status = response.status();
                if !status.is_success() {
                    let message = format!(
                        "interactions input resource returned HTTP {}",
                        status.as_u16()
                    );
                    return Err(if status.is_server_error() {
                        ProviderError::retryable(message)
                    } else {
                        ProviderError::fatal(message)
                    });
                }
                let mime = mime_hint
                    .clone()
                    .or_else(|| {
                        response
                            .headers()
                            .get(reqwest::header::CONTENT_TYPE)
                            .and_then(|value| value.to_str().ok())
                            .map(str::to_string)
                    })
                    .unwrap_or_else(|| "application/octet-stream".to_string());
                let bytes = response.bytes().await.map_err(|err| {
                    ProviderError::retryable(format!(
                        "failed to read interactions input resource: {err}"
                    ))
                })?;
                Ok(json!({
                    "type": media_type,
                    "mime_type": mime,
                    "data": general_purpose::STANDARD.encode(bytes)
                }))
            }
            ResourceRef::Url { url, mime_hint } => Ok(json!({
                "type": media_type,
                "mime_type": mime_hint.as_deref().unwrap_or("application/octet-stream"),
                "uri": url
            })),
            ResourceRef::NamedObject { obj_id } => Err(ProviderError::fatal(format!(
                "google gemini provider cannot resolve named object resource {} without resolver bytes",
                obj_id
            ))),
        }
    }

    fn provider_protocol(req: &AiMethodRequest) -> Option<&str> {
        req.payload
            .options
            .as_ref()
            .and_then(|options| options.get("provider_options"))
            .and_then(|options| options.get("protocol"))
            .and_then(Value::as_str)
    }

    fn interactions_output_text(body: &Value) -> String {
        body.get("steps")
            .and_then(Value::as_array)
            .and_then(|steps| {
                steps
                    .iter()
                    .rev()
                    .find(|step| step.get("type").and_then(Value::as_str) == Some("model_output"))
            })
            .and_then(|step| step.get("content"))
            .and_then(Value::as_array)
            .map(|content| {
                content
                    .iter()
                    .filter(|item| item.get("type").and_then(Value::as_str) == Some("text"))
                    .filter_map(|item| item.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default()
    }

    fn interactions_word_segments(body: &Value) -> Vec<Value> {
        body.get("steps")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|step| step.get("content").and_then(Value::as_array))
            .flatten()
            .filter_map(|content| content.get("annotations").and_then(Value::as_array))
            .flatten()
            .filter(|annotation| {
                annotation.get("type").and_then(Value::as_str) == Some("word_info")
            })
            .cloned()
            .collect()
    }

    fn interactions_usage(body: &Value) -> Option<AiUsage> {
        let usage = body.get("usage")?;
        Some(AiUsage {
            input_tokens: usage.get("total_input_tokens").and_then(Value::as_u64),
            output_tokens: usage.get("total_output_tokens").and_then(Value::as_u64),
            total_tokens: usage.get("total_tokens").and_then(Value::as_u64),
            request_units: None,
        })
    }

    fn build_interactions_asr_request(
        provider_model: &str,
        req: &AiMethodRequest,
        resource: Value,
    ) -> Result<Map<String, Value>, ProviderError> {
        let input = req.payload.input_json.as_ref();
        let timestamps = input
            .and_then(|value| value.get("timestamps"))
            .and_then(Value::as_str)
            .unwrap_or("none");
        let diarization = input
            .and_then(|value| value.get("diarization"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mode = input
            .and_then(|value| value.get("mode"))
            .and_then(Value::as_str)
            .unwrap_or("verbatim");
        if mode == "smart" && (diarization || timestamps != "none") {
            return Err(ProviderError::fatal(
                "google gemini smart transcription is incompatible with timestamps and diarization",
            ));
        }
        if !matches!(mode, "verbatim" | "smart") {
            return Err(ProviderError::fatal(format!(
                "unsupported google gemini transcription mode {}",
                mode
            )));
        }

        let mut transcription_config = Map::new();
        if let Some(language) = input
            .and_then(|value| value.get("language"))
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
        {
            transcription_config.insert("language_codes".to_string(), json!([language]));
        }
        if let Some(vocabulary) = input
            .and_then(|value| value.get("custom_vocabulary"))
            .and_then(Value::as_array)
        {
            transcription_config.insert(
                "custom_vocabulary".to_string(),
                Value::Array(vocabulary.clone()),
            );
        }
        if mode == "smart" {
            transcription_config.insert("mode".to_string(), Value::String("smart".to_string()));
        } else {
            let mut mode_config =
                Map::from_iter([("type".to_string(), Value::String("verbatim".to_string()))]);
            if diarization {
                mode_config.insert(
                    "diarization_mode".to_string(),
                    Value::String("speaker".to_string()),
                );
            }
            if timestamps != "none" {
                mode_config.insert("timestamp_granularities".to_string(), json!(["word"]));
            }
            transcription_config.insert("mode".to_string(), Value::Object(mode_config));
        }

        Ok(Map::from_iter([
            (
                "model".to_string(),
                Value::String(provider_model.to_string()),
            ),
            ("input".to_string(), Value::Array(vec![resource])),
            (
                "generation_config".to_string(),
                json!({ "transcription_config": transcription_config }),
            ),
        ]))
    }

    async fn start_interactions_asr(
        &self,
        provider_model: &str,
        req: &AiMethodRequest,
        resource: ResourceRef,
    ) -> Result<ProviderStartResult, ProviderError> {
        let resource = self.interactions_resource_part(&resource, "audio").await?;
        let request_obj = Self::build_interactions_asr_request(provider_model, req, resource)?;
        let (status, body, latency_ms) = self.post_interaction(&request_obj).await?;
        if !status.is_success() {
            return Err(Self::classify_api_error(
                status,
                Self::api_error_message(
                    &body,
                    "google gemini interactions transcription returned non-success status",
                ),
            ));
        }
        match body.get("status").and_then(Value::as_str) {
            Some("completed") => {}
            Some("in_progress") => {
                return Err(ProviderError::retryable(
                    "google gemini interactions transcription remained in progress",
                ));
            }
            Some(other) => {
                return Err(ProviderError::fatal(format!(
                    "google gemini interactions transcription finished with status {}",
                    other
                )));
            }
            None => {
                return Err(ProviderError::fatal(
                    "google gemini interactions transcription response is missing status",
                ));
            }
        }
        let text = Self::interactions_output_text(&body);
        let segments = Self::interactions_word_segments(&body);
        let usage = Self::interactions_usage(&body);
        let cost = usage
            .as_ref()
            .and_then(|usage| self.estimate_cost_for_usage(provider_model, usage));
        Ok(ProviderStartResult::Immediate(AiResponse {
            message: AiResponse::message_from_parts(Some(text.clone()), vec![], vec![]),
            usage,
            cost,
            finish_reason: Some("stop".to_string()),
            extra: Some(json!({
                "asr": {
                    "status": if text.trim().is_empty() { "no_speech" } else { "reliable" },
                    "speech_detected": !text.trim().is_empty(),
                    "text": text,
                    "segments": segments
                },
                "latency_ms": latency_ms,
                "provider_io": { "input": request_obj, "output": body }
            })),
            ..Default::default()
        }))
    }

    fn interactions_video_artifact(body: &Value) -> Result<AiArtifact, ProviderError> {
        let video = body
            .get("steps")
            .and_then(Value::as_array)
            .and_then(|steps| {
                steps.iter().rev().find_map(|step| {
                    step.get("content")
                        .and_then(Value::as_array)
                        .and_then(|content| {
                            content.iter().find(|item| {
                                item.get("type").and_then(Value::as_str) == Some("video")
                            })
                        })
                })
            })
            .ok_or_else(|| {
                ProviderError::fatal("google gemini interactions response is missing video output")
            })?;
        let mime = video
            .get("mime_type")
            .or_else(|| video.get("mimeType"))
            .and_then(Value::as_str)
            .unwrap_or("video/mp4")
            .to_string();
        let resource = if let Some(data_base64) = video.get("data").and_then(Value::as_str) {
            general_purpose::STANDARD
                .decode(data_base64)
                .map_err(|err| {
                    ProviderError::fatal(format!(
                        "google gemini interactions video contains invalid base64: {}",
                        err
                    ))
                })?;
            ResourceRef::Base64 {
                mime: mime.clone(),
                data_base64: data_base64.to_string(),
            }
        } else if let Some(url) = video.get("uri").and_then(Value::as_str) {
            ResourceRef::Url {
                url: url.to_string(),
                mime_hint: Some(mime.clone()),
            }
        } else {
            return Err(ProviderError::fatal(
                "google gemini interactions video output has neither data nor uri",
            ));
        };
        Ok(AiArtifact {
            name: "video.mp4".to_string(),
            resource,
            mime: Some(mime),
            metadata: None,
        })
    }

    fn music_prompt(req: &AiMethodRequest) -> Result<String, ProviderError> {
        let mut prompt = Self::extract_text2image_prompt(req).ok_or_else(|| {
            ProviderError::fatal("audio.music requires prompt in input_json.prompt or payload.text")
        })?;
        let input = req.payload.input_json.as_ref();
        let instrumental = input
            .and_then(|value| value.get("instrumental"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let lyrics = input
            .and_then(|value| value.get("lyrics"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if instrumental && lyrics.is_some() {
            return Err(ProviderError::fatal(
                "audio.music cannot combine instrumental=true with explicit lyrics",
            ));
        }
        if instrumental {
            prompt.push_str("\nInstrumental only, no vocals.");
        }
        if let Some(duration) = input
            .and_then(|value| value.get("duration"))
            .and_then(Value::as_u64)
        {
            prompt.push_str(format!("\nTarget duration: {} seconds.", duration).as_str());
        }
        if let Some(lyrics) = lyrics {
            prompt.push_str(format!("\nLyrics:\n{}", lyrics).as_str());
        }
        Ok(prompt)
    }

    fn merge_audio_response_preferences(
        request: &mut Map<String, Value>,
        req: &AiMethodRequest,
    ) -> Result<(), ProviderError> {
        let Some(input) = req.payload.input_json.as_ref() else {
            return Ok(());
        };
        if let Some(seed) = input.get("seed") {
            Self::ensure_generation_config(request).insert("seed".to_string(), seed.clone());
        }
        let output = input.get("output").and_then(Value::as_object);
        if output.is_none() {
            return Ok(());
        }
        let mut audio = Map::new();
        if let Some(media_type) = output
            .and_then(|value| value.get("media_type"))
            .and_then(Value::as_str)
        {
            let mime_type = match media_type.trim().to_ascii_lowercase().as_str() {
                "audio/mpeg" | "audio/mp3" => "AUDIO_MP3",
                "audio/ogg" | "audio/opus" => "AUDIO_OGG_OPUS",
                "audio/wav" | "audio/wave" => "AUDIO_WAV",
                value if value.starts_with("audio/l16") => "AUDIO_L16",
                _ => {
                    return Err(ProviderError::fatal(format!(
                        "google gemini audio output does not support {}",
                        media_type
                    )));
                }
            };
            audio.insert("mimeType".to_string(), Value::String(mime_type.to_string()));
        }
        if let Some(sample_rate) = output.and_then(|value| value.get("sample_rate")) {
            audio.insert("sampleRate".to_string(), sample_rate.clone());
        }
        if !audio.is_empty() {
            let generation = Self::ensure_generation_config(request);
            let response_format = generation
                .entry("responseFormat".to_string())
                .or_insert_with(|| json!({}));
            response_format
                .as_object_mut()
                .expect("responseFormat should be an object")
                .insert("audio".to_string(), Value::Object(audio));
        }
        Ok(())
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
        let contents = Self::build_contents(req)?;
        let mut request_obj = Map::new();
        request_obj.insert("contents".to_string(), Value::Array(contents));

        let json_output_required = req.requirements.requires_feature(features::JSON_OUTPUT);
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
        let combined_tools_supported = self.model_supports_feature_combination(
            provider_model,
            &[features::WEB_SEARCH, features::TOOL_CALLING],
        );
        Self::merge_llm_tools(
            &mut request_obj,
            provider_model,
            req,
            combined_tools_supported,
        )?;

        if !ignored_options.is_empty() {
            warn!(
                "aicc.gemini ignored unsupported llm options: provider_instance_name={} model={} trace_id={:?} ignored={:?}",
                self.instance.provider_instance_name, provider_model, ctx.trace_id, ignored_options
            );
        }

        let model_max_output_tokens = self.model_max_output_tokens(provider_model);
        let completion_tokens =
            Self::apply_separate_thinking_budget(&mut request_obj, model_max_output_tokens);

        let request_log = redacted_json_log(&Value::Object(request_obj.clone()));
        info!(
            "aicc.gemini.llm.input provider_instance_name={} model={} trace_id={:?} request={}",
            self.instance.provider_instance_name, provider_model, ctx.trace_id, request_log
        );

        let (mut status, mut body, mut latency_ms) = self
            .post_generate_content(provider_model, &request_obj)
            .await?;
        let mut response_log = redacted_json_log(&body);
        let mut previous_usage_metadata = None;

        if !status.is_success() {
            warn!(
                "aicc.gemini.llm.output provider_instance_name={} model={} trace_id={:?} status={} response={}",
                self.instance.provider_instance_name,
                provider_model,
                ctx.trace_id,
                status.as_u16(),
                response_log
            );
            let message = body
                .pointer("/error/message")
                .and_then(|value| value.as_str())
                .unwrap_or("google gemini api returned non-success status")
                .to_string();
            let code = body
                .pointer("/error/status")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            return Err(Self::classify_api_error(
                status,
                format!("google gemini api error [{}]: {}", code, message),
            ));
        }

        let thinking_retry = completion_tokens.and_then(|completion_tokens| {
            Self::thinking_retry_output_limit(
                &request_obj,
                &body,
                completion_tokens,
                model_max_output_tokens,
            )
        });
        if let Some(retry_output_limit) = thinking_retry {
            previous_usage_metadata = body.get("usageMetadata").cloned();
            let thinking_tokens = body
                .pointer("/usageMetadata/thoughtsTokenCount")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            Self::ensure_generation_config(&mut request_obj).insert(
                "maxOutputTokens".to_string(),
                Value::from(retry_output_limit),
            );
            info!(
                "aicc.gemini retrying truncated response with separate thinking budget: provider_instance_name={} model={} trace_id={:?} thinking_tokens={} max_output_tokens={}",
                self.instance.provider_instance_name,
                provider_model,
                ctx.trace_id,
                thinking_tokens,
                retry_output_limit
            );
            let (retry_status, retry_body, retry_latency_ms) = self
                .post_generate_content(provider_model, &request_obj)
                .await?;
            status = retry_status;
            body = retry_body;
            latency_ms = latency_ms.saturating_add(retry_latency_ms);
            response_log = redacted_json_log(&body);
            if !status.is_success() {
                let message = body
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("google gemini retry returned non-success status");
                return Err(Self::classify_api_error(status, message.to_string()));
            }
        }

        info!(
            "aicc.gemini.llm.output provider_instance_name={} model={} trace_id={:?} status={} response={}",
            self.instance.provider_instance_name,
            provider_model,
            ctx.trace_id,
            status.as_u16(),
            response_log
        );

        let content = Self::extract_text_content(&body);
        let tool_calls = Self::extract_tool_calls(&body);
        let usage_metadata = body.get("usageMetadata");
        let usage_value = |key: &str| {
            let previous = previous_usage_metadata
                .as_ref()
                .and_then(|usage| usage.get(key))
                .and_then(Value::as_u64);
            let current = usage_metadata
                .and_then(|usage| usage.get(key))
                .and_then(Value::as_u64);
            match (previous, current) {
                (Some(left), Some(right)) => Some(left.saturating_add(right)),
                (Some(value), None) | (None, Some(value)) => Some(value),
                (None, None) => None,
            }
        };
        let usage =
            (usage_metadata.is_some() || previous_usage_metadata.is_some()).then(|| AiUsage {
                input_tokens: usage_value("promptTokenCount"),
                output_tokens: usage_value("candidatesTokenCount"),
                total_tokens: usage_value("totalTokenCount"),
                request_units: None,
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
        if let Some(thinking_tokens) = usage_value("thoughtsTokenCount") {
            extra.insert("thinking_tokens".to_string(), Value::from(thinking_tokens));
        }
        extra.insert(
            "provider_io".to_string(),
            json!({
                "input": Value::Object(request_obj.clone()),
                "output": body.clone()
            }),
        );
        if let Some(grounding_metadata) = body.pointer("/candidates/0/groundingMetadata") {
            extra.insert("grounding_metadata".to_string(), grounding_metadata.clone());
        }

        let mut message = AiResponse::message_from_parts(content, tool_calls, vec![]);
        if let Some(provider_state) = Self::extract_provider_state(&body) {
            message.content.push(provider_state);
        }
        let summary = AiResponse {
            message,
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
        if let Some(input_json) = req.payload.input_json.as_ref().and_then(Value::as_object) {
            Self::merge_image_prompt_preferences(&mut request_obj, input_json);
        }

        let mut ignored_options = vec![];
        if let Some(options) = req.payload.options.as_ref() {
            ignored_options = Self::merge_text2image_options(&mut request_obj, options)?;
        }
        if !ignored_options.is_empty() {
            warn!(
                "aicc.gemini ignored unsupported text2image options: provider_instance_name={} model={} trace_id={:?} ignored={:?}",
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

        let request_log = redacted_json_log(&Value::Object(request_obj.clone()));
        info!(
            "aicc.gemini.text2image.input provider_instance_name={} model={} trace_id={:?} request={}",
            self.instance.provider_instance_name, provider_model, ctx.trace_id, request_log
        );

        let (status, body, latency_ms) = self
            .post_generate_content(provider_model, &request_obj)
            .await?;
        let response_log = redacted_json_log(&body);

        if !status.is_success() {
            warn!(
                "aicc.gemini.text2image.output provider_instance_name={} model={} trace_id={:?} status={} response={}",
                self.instance.provider_instance_name,
                provider_model,
                ctx.trace_id,
                status.as_u16(),
                response_log
            );
            let message = body
                .pointer("/error/message")
                .and_then(|value| value.as_str())
                .unwrap_or("google gemini api returned non-success status")
                .to_string();
            let code = body
                .pointer("/error/status")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            return Err(Self::classify_api_error(
                status,
                format!("google gemini api error [{}]: {}", code, message),
            ));
        }

        info!(
            "aicc.gemini.text2image.output provider_instance_name={} model={} trace_id={:?} status={} response={}",
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

        let summary = AiResponse {
            message: AiResponse::message_from_parts(text, vec![], artifacts),
            usage: Some(AiUsage::request_units(1)),
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
        _ctx: &crate::aicc::InvokeCtx,
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
                    Self::content_resource_part(&resource)?,
                    { "text": prompt }
                ]
            }]),
        );
        Self::ensure_generation_config(&mut request_obj)
            .insert("responseModalities".to_string(), json!(["IMAGE"]));
        if let Some(input) = req.payload.input_json.as_ref().and_then(Value::as_object) {
            Self::merge_image_response_preferences(&mut request_obj, input)?;
            if let Some(strength) = input.get("strength").and_then(Value::as_f64) {
                let prompt = request_obj
                    .get("contents")
                    .and_then(Value::as_array)
                    .and_then(|contents| contents.first())
                    .and_then(|content| content.get("parts"))
                    .and_then(Value::as_array)
                    .and_then(|parts| parts.get(1))
                    .and_then(|part| part.get("text"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                request_obj["contents"][0]["parts"][1]["text"] = Value::String(format!(
                    "Apply the requested edit with strength {} on a scale from 0 to 1. {}",
                    strength, prompt
                ));
            }
        }
        let (status, body, latency_ms) = self
            .post_generate_content(provider_model, &request_obj)
            .await?;
        if !status.is_success() {
            let message = body
                .pointer("/error/message")
                .and_then(|value| value.as_str())
                .unwrap_or("google gemini image edit returned non-success status");
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
        Ok(ProviderStartResult::Immediate(AiResponse {
            message: AiResponse::message_from_parts(text, vec![], artifacts),
            usage: Some(AiUsage::request_units(1)),
            finish_reason: Some("stop".to_string()),
            extra: Some(Value::Object(extra)),
            ..Default::default()
        }))
    }

    async fn start_embedding(
        &self,
        _ctx: &crate::aicc::InvokeCtx,
        provider_model: &str,
        req: &AiMethodRequest,
        multimodal: bool,
    ) -> Result<ProviderStartResult, ProviderError> {
        let mut texts = req
            .payload
            .input_json
            .as_ref()
            .and_then(|value| value.get("items"))
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        item.get("text")
                            .and_then(Value::as_str)
                            .or_else(|| item.as_str())
                            .map(str::trim)
                            .filter(|text| !text.is_empty())
                            .map(ToString::to_string)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if texts.is_empty() {
            if let Some(text) = req
                .payload
                .text
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
            {
                texts.push(text.to_string());
            } else {
                texts.extend(
                    req.payload
                        .messages
                        .iter()
                        .map(|message| message.text_content().trim().to_string())
                        .filter(|text| !text.is_empty()),
                );
            }
        }
        let resource = if multimodal {
            req.payload
                .resources
                .first()
                .cloned()
                .or_else(|| Self::resource_from_input_json(req, &["image", "audio", "video"]))
        } else {
            None
        };
        if texts.is_empty() && resource.is_none() {
            return Err(ProviderError::fatal(
                "embedding request requires text or multimodal resource",
            ));
        }
        if resource.is_some() && texts.len() > 1 {
            return Err(ProviderError::fatal(
                "embedding.multimodal accepts one text item with one resource",
            ));
        }

        let dimensions = req
            .payload
            .input_json
            .as_ref()
            .and_then(|value| value.get("dimensions"))
            .cloned();
        let build_config = || {
            dimensions
                .as_ref()
                .map(|value| json!({ "outputDimensionality": value }))
        };
        let mut request_objects = Vec::new();
        let action;
        if texts.len() > 1 {
            let model = format!("models/{}", provider_model.trim_start_matches("models/"));
            for batch in texts.chunks(100) {
                let requests = batch
                    .iter()
                    .map(|text| {
                        let mut request = json!({
                            "model": model,
                            "content": { "parts": [{ "text": text }] }
                        });
                        if let Some(config) = build_config() {
                            request["embedContentConfig"] = config;
                        }
                        request
                    })
                    .collect::<Vec<_>>();
                request_objects.push(Map::from_iter([(
                    "requests".to_string(),
                    Value::Array(requests),
                )]));
            }
            action = "batchEmbedContents";
        } else {
            let mut request_obj = Map::new();
            let mut parts = Vec::new();
            if let Some(text) = texts.first() {
                parts.push(json!({ "text": text }));
            }
            if let Some(resource) = resource.as_ref() {
                parts.push(Self::content_resource_part(resource)?);
            }
            request_obj.insert("content".to_string(), json!({ "parts": parts }));
            if let Some(config) = build_config() {
                request_obj.insert("embedContentConfig".to_string(), config);
            }
            request_objects.push(request_obj);
            action = "embedContent";
        }
        let mut embeddings = Vec::new();
        let mut response_bodies = Vec::new();
        let mut latency_ms = 0u64;
        let mut prompt_tokens = 0u64;
        for request_obj in request_objects.iter() {
            let (status, body, request_latency_ms) = self
                .post_model_action(provider_model, action, request_obj)
                .await?;
            if !status.is_success() {
                let message = body
                    .pointer("/error/message")
                    .and_then(|value| value.as_str())
                    .unwrap_or("google gemini embedding returned non-success status");
                return Err(Self::classify_api_error(status, message.to_string()));
            }
            latency_ms = latency_ms.saturating_add(request_latency_ms);
            prompt_tokens = prompt_tokens.saturating_add(
                body.pointer("/usageMetadata/promptTokenCount")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            );
            if action == "batchEmbedContents" {
                embeddings.extend(
                    body.get("embeddings")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default(),
                );
            } else if let Some(embedding) = body.get("embedding") {
                embeddings.push(embedding.clone());
            }
            response_bodies.push(body);
        }
        if embeddings.is_empty() {
            return Err(ProviderError::fatal(
                "google gemini embedding response is missing embedding values",
            ));
        }
        let output_dimensions = embeddings
            .first()
            .and_then(|embedding| embedding.get("values"))
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        let embedding_space_id = format!("google-gemini:{}:{}", provider_model, output_dimensions);
        let data = embeddings
            .iter()
            .enumerate()
            .map(|(index, embedding)| {
                json!({
                    "index": index,
                    "embedding": embedding.get("values").cloned().unwrap_or(Value::Array(vec![])),
                    "embedding_space_id": embedding_space_id
                })
            })
            .collect::<Vec<_>>();
        let input_json = req.payload.input_json.as_ref();
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
                    "dimensions": output_dimensions,
                    "embedding_space_id": embedding_space_id.clone(),
                })),
            })
        } else {
            None
        };
        let usage = (prompt_tokens > 0)
            .then(|| AiUsage {
                input_tokens: Some(prompt_tokens),
                output_tokens: Some(0),
                total_tokens: Some(prompt_tokens),
                request_units: None,
            })
            .unwrap_or_else(|| AiUsage::request_units(request_objects.len() as u64));
        let cost = self.estimate_cost_for_usage(provider_model, &usage);
        let provider_input = if request_objects.len() == 1 {
            Value::Object(request_objects[0].clone())
        } else {
            Value::Array(request_objects.into_iter().map(Value::Object).collect())
        };
        let provider_output = if response_bodies.len() == 1 {
            response_bodies.remove(0)
        } else {
            Value::Array(response_bodies)
        };
        let mut extra = Map::new();
        extra.insert(
            "embedding".to_string(),
            json!({
                "data": if prefer_artifact { Value::Array(vec![]) } else { Value::Array(data.clone()) },
                "embedding_space_id": embedding_space_id.clone(),
                "artifact": artifact.as_ref().map(|value| json!({
                    "name": value.name.clone(),
                    "mime": value.mime.clone(),
                    "rows": data.len(),
                    "dimensions": output_dimensions,
                    "embedding_space_id": embedding_space_id.clone(),
                })),
                "provider_io": { "input": provider_input, "output": provider_output },
                "latency_ms": latency_ms
            }),
        );
        Ok(ProviderStartResult::Immediate(AiResponse {
            message: AiResponse::message_from_parts(None, vec![], artifact.into_iter().collect()),
            usage: Some(usage),
            cost,
            finish_reason: Some("stop".to_string()),
            extra: Some(Value::Object(extra)),
            ..Default::default()
        }))
    }

    async fn start_vision(
        &self,
        _ctx: &crate::aicc::InvokeCtx,
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
        let mut request_obj = json!({
            "contents": [{
                "role": "user",
                "parts": [
                    Self::content_resource_part(&resource)?,
                    { "text": Self::prompt_for_method(method, req) }
                ]
            }],
            "generationConfig": { "responseMimeType": "application/json" }
        })
        .as_object()
        .cloned()
        .unwrap_or_default();
        if let Some(options) = req.payload.options.as_ref() {
            Self::merge_llm_options(&mut request_obj, options, true)?;
        }
        Self::apply_separate_thinking_budget(
            &mut request_obj,
            self.model_max_output_tokens(provider_model),
        );
        let (status, body, latency_ms) = self
            .post_generate_content(provider_model, &request_obj)
            .await?;
        if !status.is_success() {
            return Err(Self::classify_api_error(
                status,
                Self::api_error_message(&body, "google gemini vision returned non-success status"),
            ));
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
        // Gemini 的 generateContent 响应里 vision 调用同样会带 usageMetadata，
        // 不抽出来的话 UsageLoggingSink 在 emit Final 时会因为 summary.usage
        // 缺失而打 missing_usage_protocol_error 跳过整条 usage row，导致
        // /opt/buckyos/data/aicc/aicc-usage-log.db 里这部分调用丢账。
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
            request_units: None,
        });
        let cost = usage
            .as_ref()
            .and_then(|usage| self.estimate_cost_for_usage(provider_model, usage));
        let mut extra = Map::new();
        extra.insert(extra_key.to_string(), parsed);
        extra.insert("latency_ms".to_string(), Value::from(latency_ms));
        extra.insert(
            "provider_io".to_string(),
            json!({ "input": request_obj, "output": body }),
        );
        Ok(ProviderStartResult::Immediate(AiResponse {
            message: AiResponse::message_from_parts(text, vec![], vec![]),
            usage,
            cost,
            finish_reason: Some("stop".to_string()),
            extra: Some(Value::Object(extra)),
            ..Default::default()
        }))
    }

    async fn start_asr(
        &self,
        _ctx: &crate::aicc::InvokeCtx,
        provider_model: &str,
        req: &AiMethodRequest,
    ) -> Result<ProviderStartResult, ProviderError> {
        let resource = req
            .payload
            .resources
            .first()
            .cloned()
            .or_else(|| Self::resource_from_input_json(req, &["audio"]))
            .ok_or_else(|| ProviderError::fatal("audio.asr requires an audio resource"))?;
        if Self::provider_protocol(req) == Some("interactions") {
            return self
                .start_interactions_asr(provider_model, req, resource)
                .await;
        }
        let input = req.payload.input_json.as_ref();
        let language = input
            .and_then(|value| value.get("language"))
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty());
        let timestamps = input
            .and_then(|value| value.get("timestamps"))
            .and_then(Value::as_str)
            .unwrap_or("none");
        let diarization = input
            .and_then(|value| value.get("diarization"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let language_instruction = language
            .map(|value| format!(" The expected language is {value}."))
            .unwrap_or_default();
        let prompt = format!(
            "First determine whether the supplied audio contains clear, intelligible human speech. Classify sound effects, tones, music, noise, and ambiguous vocal-like sounds as non-speech. Base the transcript strictly on clearly audible words.{language_instruction} Return JSON with `speech_detected`, calibrated `transcript_confidence` from 0 to 1, the complete transcript in `text`, and chronological `segments`. Use an empty `text` and empty `segments` for non-speech audio, and use a low confidence when words are ambiguous. Timestamp detail: {timestamps}. Speaker diarization: {diarization}."
        );
        let schema = json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["speech_detected", "transcript_confidence", "text", "segments"],
            "properties": {
                "speech_detected": { "type": "boolean" },
                "transcript_confidence": { "type": "number", "minimum": 0, "maximum": 1 },
                "text": { "type": "string" },
                "segments": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["id", "start_seconds", "end_seconds", "text"],
                        "properties": {
                            "id": { "type": "string" },
                            "start_seconds": { "type": "number" },
                            "end_seconds": { "type": "number" },
                            "text": { "type": "string" },
                            "speaker": { "type": "string" },
                            "confidence": { "type": "number", "minimum": 0, "maximum": 1 }
                        }
                    }
                }
            }
        });
        let mut request_obj = json!({
            "contents": [{
                "role": "user",
                "parts": [
                    Self::content_resource_part(&resource)?,
                    { "text": prompt }
                ]
            }],
            "generationConfig": {
                "responseMimeType": "application/json",
                "responseJsonSchema": schema
            }
        })
        .as_object()
        .cloned()
        .expect("Gemini ASR request should be an object");
        if let Some(options) = req.payload.options.as_ref() {
            Self::merge_llm_options(&mut request_obj, options, true)?;
        }
        Self::apply_separate_thinking_budget(
            &mut request_obj,
            self.model_max_output_tokens(provider_model),
        );
        let (status, body, latency_ms) = self
            .post_generate_content(provider_model, &request_obj)
            .await?;
        if !status.is_success() {
            let message = body
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("google gemini transcription returned non-success status");
            return Err(Self::classify_api_error(status, message.to_string()));
        }
        let raw_text = Self::extract_text_content(&body).unwrap_or_default();
        let parsed = serde_json::from_str::<Value>(raw_text.trim()).unwrap_or_else(|_| {
            json!({
                "speech_detected": false,
                "transcript_confidence": 0,
                "text": raw_text,
                "segments": []
            })
        });
        let assessment = assess_gemini_asr(&parsed);
        let usage = body.get("usageMetadata").map(|usage| AiUsage {
            input_tokens: usage.get("promptTokenCount").and_then(Value::as_u64),
            output_tokens: usage.get("candidatesTokenCount").and_then(Value::as_u64),
            total_tokens: usage.get("totalTokenCount").and_then(Value::as_u64),
            request_units: None,
        });
        let cost = usage
            .as_ref()
            .and_then(|usage| self.estimate_cost_for_usage(provider_model, usage));
        Ok(ProviderStartResult::Immediate(AiResponse {
            message: AiResponse::message_from_parts(Some(assessment.transcript), vec![], vec![]),
            usage,
            cost,
            finish_reason: Some("stop".to_string()),
            extra: Some(json!({
                "asr": {
                    "status": assessment.status,
                    "speech_detected": assessment.speech_detected,
                    "confidence": assessment.confidence,
                    "candidate_text": assessment.candidate_text,
                    "segments": assessment.segments,
                    "provider_io": { "input": request_obj, "output": body },
                    "latency_ms": latency_ms
                }
            })),
            ..Default::default()
        }))
    }

    async fn start_audio_media(
        &self,
        _ctx: &crate::aicc::InvokeCtx,
        provider_model: &str,
        method: &str,
        req: &AiMethodRequest,
    ) -> Result<ProviderStartResult, ProviderError> {
        let prompt = if method == ai_methods::AUDIO_TTS {
            Self::tts_prompt(req)?
        } else {
            Self::music_prompt(req)?
        };
        let mut request_obj = json!({
            "contents": [{
                "role": "user",
                "parts": [{ "text": prompt }]
            }],
            "generationConfig": { "responseModalities": ["AUDIO"] }
        })
        .as_object()
        .cloned()
        .unwrap_or_default();
        Self::merge_audio_response_preferences(&mut request_obj, req)?;
        if method == ai_methods::AUDIO_TTS {
            let input = req.payload.input_json.as_ref();
            let voice_name = input
                .and_then(|value| value.get("voice_id"))
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or("Kore");
            let speech_config = Self::ensure_generation_config(&mut request_obj)
                .entry("speechConfig".to_string())
                .or_insert_with(|| json!({}));
            let speech_config = speech_config
                .as_object_mut()
                .expect("speechConfig should be an object");
            speech_config.insert(
                "voiceConfig".to_string(),
                json!({ "prebuiltVoiceConfig": { "voiceName": voice_name } }),
            );
            if let Some(language) = input
                .and_then(|value| value.get("language"))
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
            {
                speech_config.insert(
                    "languageCode".to_string(),
                    Value::String(language.to_string()),
                );
            }
        }
        let (status, body, latency_ms) = self
            .post_generate_content(provider_model, &request_obj)
            .await?;
        if !status.is_success() {
            let message = body
                .pointer("/error/message")
                .and_then(|value| value.as_str())
                .unwrap_or("google gemini audio returned non-success status");
            return Err(Self::classify_api_error(status, message.to_string()));
        }
        let artifacts = Self::parse_media_artifacts(&body, "audio/mpeg");
        let mut extra = Map::new();
        extra.insert("latency_ms".to_string(), Value::from(latency_ms));
        extra.insert(
            "provider_io".to_string(),
            json!({ "input": request_obj, "output": body }),
        );
        Ok(ProviderStartResult::Immediate(AiResponse {
            message: AiResponse::message_from_parts(None, vec![], artifacts),
            usage: Some(AiUsage::request_units(1)),
            finish_reason: Some("stop".to_string()),
            extra: Some(Value::Object(extra)),
            ..Default::default()
        }))
    }

    async fn start_interactions_video(
        &self,
        provider_model: &str,
        method: &str,
        req: &AiMethodRequest,
    ) -> Result<ProviderStartResult, ProviderError> {
        let prompt = Self::prompt_for_method(method, req);
        let resource = req
            .payload
            .resources
            .first()
            .cloned()
            .or_else(|| Self::resource_from_input_json(req, &["image", "video"]));
        let input = if let Some(resource) = resource {
            let media_type = if method == ai_methods::VIDEO_VIDEO2VIDEO {
                "video"
            } else {
                "image"
            };
            let content = vec![
                self.interactions_resource_part(&resource, media_type)
                    .await?,
                json!({ "type": "text", "text": prompt }),
            ];
            if method == ai_methods::VIDEO_VIDEO2VIDEO {
                json!([{ "type": "user_input", "content": content }])
            } else {
                Value::Array(content)
            }
        } else {
            Value::String(prompt)
        };

        let task = match method {
            ai_methods::VIDEO_TXT2VIDEO => "text_to_video",
            ai_methods::VIDEO_IMG2VIDEO => "image_to_video",
            ai_methods::VIDEO_VIDEO2VIDEO => "edit",
            _ => {
                return Err(ProviderError::fatal(format!(
                    "google gemini interactions protocol does not support {}",
                    method
                )));
            }
        };
        let mut response_format = json!({ "type": "video" });
        if let Some(aspect_ratio) = req
            .payload
            .input_json
            .as_ref()
            .and_then(|input| input.get("aspect_ratio"))
            .and_then(Value::as_str)
        {
            response_format
                .as_object_mut()
                .expect("response_format should be an object")
                .insert(
                    "aspect_ratio".to_string(),
                    Value::String(aspect_ratio.to_string()),
                );
        }
        let mut request_obj = json!({
            "model": provider_model,
            "input": input,
            "response_format": response_format,
            "generation_config": {
                "video_config": { "task": task }
            }
        })
        .as_object()
        .cloned()
        .expect("interactions request should be an object");
        if let Some(previous_interaction_id) = req
            .payload
            .input_json
            .as_ref()
            .and_then(|input| input.get("previous_interaction_id"))
            .and_then(Value::as_str)
        {
            request_obj.insert(
                "previous_interaction_id".to_string(),
                Value::String(previous_interaction_id.to_string()),
            );
        }

        let (status, body, latency_ms) = self.post_interaction(&request_obj).await?;
        if !status.is_success() {
            let message = body
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("google gemini interactions video returned non-success status");
            return Err(Self::classify_api_error(status, message.to_string()));
        }
        let artifact = Self::interactions_video_artifact(&body)?;
        let provider_task_ref = body.get("id").and_then(Value::as_str).map(str::to_string);
        Ok(ProviderStartResult::Immediate(AiResponse {
            message: AiResponse::message_from_parts(None, vec![], vec![artifact]),
            usage: Some(AiUsage::request_units(1)),
            provider_task_ref,
            finish_reason: Some("stop".to_string()),
            extra: Some(json!({
                "provider": "google_gemini",
                "method": method,
                "model": provider_model,
                "latency_ms": latency_ms,
                "provider_io": { "input": request_obj, "output": body }
            })),
            ..Default::default()
        }))
    }

    async fn start_video(
        &self,
        ctx: &crate::aicc::InvokeCtx,
        provider_model: &str,
        method: &str,
        req: &AiMethodRequest,
        sink: Arc<dyn TaskEventSink>,
    ) -> Result<ProviderStartResult, ProviderError> {
        if method == ai_methods::VIDEO_IMG2VIDEO
            && req.payload.resources.is_empty()
            && Self::resource_from_input_json(req, &["image"]).is_none()
        {
            return Err(ProviderError::fatal(
                "video.img2video requires an image input",
            ));
        }
        if matches!(
            method,
            ai_methods::VIDEO_VIDEO2VIDEO | ai_methods::VIDEO_EXTEND
        ) && req.payload.resources.is_empty()
            && Self::resource_from_input_json(req, &["video"]).is_none()
        {
            return Err(ProviderError::fatal(format!(
                "{} requires a video input",
                method
            )));
        }
        if Self::provider_protocol(req) == Some("interactions") {
            return self
                .start_interactions_video(provider_model, method, req)
                .await;
        }
        if method == ai_methods::VIDEO_VIDEO2VIDEO {
            return Err(ProviderError::fatal(
                "google gemini predictLongRunning does not support arbitrary video editing",
            ));
        }
        let mut instance = Map::new();
        instance.insert(
            "prompt".to_string(),
            Value::String(Self::prompt_for_method(method, req)),
        );
        if method == ai_methods::VIDEO_EXTEND {
            let continuation_handle = req
                .payload
                .input_json
                .as_ref()
                .and_then(|input| input.get("continuation_handle"))
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    ProviderError::fatal(
                        "google gemini Veo video.extend requires continuation_handle from a previous Veo generation",
                    )
                })?;
            instance.insert("video".to_string(), json!({ "uri": continuation_handle }));
        } else if let Some(resource) = req
            .payload
            .resources
            .first()
            .cloned()
            .or_else(|| Self::resource_from_input_json(req, &["image"]))
        {
            instance.insert("image".to_string(), Self::veo_resource_part(&resource)?);
        }
        let mut request_obj = Map::new();
        request_obj.insert(
            "instances".to_string(),
            Value::Array(vec![Value::Object(instance)]),
        );
        let mut parameters = Map::new();
        let mut ignored_parameters = vec![];
        if let Some(input_json) = req.payload.input_json.as_ref() {
            ignored_parameters.extend(Self::merge_video_parameters(&mut parameters, input_json));
        }
        if let Some(options) = req.payload.options.as_ref() {
            ignored_parameters.extend(Self::merge_video_parameters(&mut parameters, options));
        }
        if method == ai_methods::VIDEO_EXTEND {
            if let Some(duration) = parameters.remove("durationSeconds") {
                if duration.as_u64() != Some(7) {
                    return Err(ProviderError::fatal(
                        "google gemini Veo 3.1 video extension has a fixed duration of 7 seconds",
                    ));
                }
            }
            if let Some(resolution) = parameters.get("resolution") {
                if resolution.as_str() != Some("720p") {
                    return Err(ProviderError::fatal(
                        "google gemini Veo 3.1 video extension requires 720p resolution",
                    ));
                }
            } else {
                parameters.insert("resolution".to_string(), Value::String("720p".to_string()));
            }
        }
        if !parameters.is_empty() {
            request_obj.insert("parameters".to_string(), Value::Object(parameters));
        }
        if !ignored_parameters.is_empty() {
            warn!(
                "aicc.gemini ignored unsupported video parameters: provider_instance_name={} model={} trace_id={:?} ignored={:?}",
                self.instance.provider_instance_name,
                provider_model,
                ctx.trace_id,
                ignored_parameters
            );
        }
        let started_at = std::time::Instant::now();
        let (status, body, latency_ms) = self
            .post_model_action(provider_model, "predictLongRunning", &request_obj)
            .await?;
        if !status.is_success() {
            let message = body
                .pointer("/error/message")
                .and_then(|value| value.as_str())
                .unwrap_or("google gemini video returned non-success status");
            return Err(Self::classify_api_error(status, message.to_string()));
        }
        let operation_name = body
            .get("name")
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ProviderError::fatal("google gemini video response is missing operation name")
            })?
            .to_string();

        let provider = self.clone();
        let task_id = ctx
            .task_id
            .clone()
            .unwrap_or_else(|| operation_name.clone());
        let provider_model = provider_model.to_string();
        let method = method.to_string();
        let request = req.clone();
        tokio::spawn(async move {
            let result = provider
                .finish_video(
                    provider_model.as_str(),
                    method.as_str(),
                    operation_name,
                    body,
                    request_obj,
                    started_at,
                    latency_ms,
                )
                .await;
            emit_background_provider_result(sink, task_id.as_str(), &request, result).await;
        });

        Ok(ProviderStartResult::Started)
    }

    async fn finish_video(
        &self,
        provider_model: &str,
        method: &str,
        operation_name: String,
        mut operation: Value,
        request_obj: Map<String, Value>,
        started_at: std::time::Instant,
        latency_ms: u64,
    ) -> Result<AiResponse, ProviderError> {
        loop {
            if let Some(error) = operation.get("error") {
                let message = error
                    .get("message")
                    .and_then(|value| value.as_str())
                    .unwrap_or("google gemini video generation failed");
                return Err(ProviderError::fatal(message.to_string()));
            }
            if operation
                .get("done")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                break;
            }
            if started_at.elapsed() >= GEMINI_VIDEO_MAX_WAIT {
                return Err(ProviderError::fatal(format!(
                    "google gemini video generation timed out after {} seconds",
                    GEMINI_VIDEO_MAX_WAIT.as_secs()
                )));
            }
            time::sleep(GEMINI_VIDEO_POLL_INTERVAL).await;
            let (poll_status, next_operation) =
                self.get_video_operation(operation_name.as_str()).await?;
            if !poll_status.is_success() {
                let message = next_operation
                    .pointer("/error/message")
                    .and_then(|value| value.as_str())
                    .unwrap_or("google gemini video status returned non-success status");
                return Err(ProviderError::fatal(message.to_string()));
            }
            operation = next_operation;
        }
        let video_uri = Self::video_uri(&operation).map(str::to_string);
        let (mime, data_base64) = if let Some(video_uri) = video_uri.as_deref() {
            let (download_status, bytes, content_type) = self.download_video(video_uri).await?;
            if !download_status.is_success() {
                return Err(ProviderError::fatal(format!(
                    "google gemini video download returned status {}",
                    download_status.as_u16()
                )));
            }
            let mime = if content_type.contains("video/") {
                content_type
            } else {
                "video/mp4".to_string()
            };
            (mime, general_purpose::STANDARD.encode(bytes))
        } else if let Some((mime, data_base64)) = Self::video_inline_data(&operation) {
            general_purpose::STANDARD
                .decode(data_base64)
                .map_err(|err| {
                    ProviderError::fatal(format!(
                        "google gemini completed video response contains invalid base64: {}",
                        err
                    ))
                })?;
            (mime.to_string(), data_base64.to_string())
        } else {
            return Err(ProviderError::fatal(
                "google gemini completed video response is missing video data",
            ));
        };
        let artifact = AiArtifact {
            name: "video.mp4".to_string(),
            resource: ResourceRef::Base64 {
                mime: mime.clone(),
                data_base64,
            },
            mime: Some(mime),
            metadata: None,
        };
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
        extra.insert(
            "latency_ms".to_string(),
            Value::from(started_at.elapsed().as_millis() as u64),
        );
        extra.insert("submit_latency_ms".to_string(), Value::from(latency_ms));
        extra.insert(
            "operation_name".to_string(),
            Value::String(operation_name.clone()),
        );
        if let Some(video_uri) = video_uri {
            extra.insert("continuation_handle".to_string(), Value::String(video_uri));
        }
        extra.insert(
            "provider_io".to_string(),
            json!({ "input": request_obj, "output": operation }),
        );
        Ok(AiResponse {
            message: AiResponse::message_from_parts(None, vec![], vec![artifact]),
            usage: Some(AiUsage::request_units(1)),
            cost: Some(AiCost {
                amount: 0.5,
                currency: "USD".to_string(),
            }),
            provider_task_ref: Some(operation_name),
            finish_reason: Some("stop".to_string()),
            extra: Some(Value::Object(extra)),
            ..Default::default()
        })
    }
}

#[async_trait]
impl Provider for GoogleGeminiProvider {
    fn inventory(&self) -> ProviderInventory {
        self.inventory
            .read()
            .map(|inventory| inventory.clone())
            .unwrap_or_else(|_| {
                Self::build_inventory_from_buckets(
                    self.provider_instance_name.as_str(),
                    self.provider_type.clone(),
                    self.provider_driver.as_str(),
                    &GeminiModelBuckets::default(),
                    self.features.as_slice(),
                    Some("inventory-lock-poisoned".to_string()),
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
        if matches!(
            input.api_type,
            ApiType::VideoTextToVideo
                | ApiType::VideoImageToVideo
                | ApiType::VideoToVideo
                | ApiType::VideoExtend
        ) {
            return CostEstimateOutput {
                estimated_cost_usd: 0.5,
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
            ai_methods::AUDIO_ASR => {
                self.start_asr(&ctx, provider_model.as_str(), &req.request)
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
                    sink,
                )
                .await
            }
            method => Err(ProviderError::fatal(format!(
                "google gemini provider does not support method '{}'",
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
            "google gemini provider cancellation is unsupported",
        ))
    }
}

impl Drop for GoogleGeminiProvider {
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
struct GeminiSettings {
    #[serde(default = "default_gemini_enabled")]
    enabled: bool,
    #[serde(default, alias = "api_key", alias = "apiKey")]
    api_token: String,
    #[serde(default)]
    #[allow(dead_code)]
    alias_map: HashMap<String, String>,
    #[serde(default)]
    instances: Vec<SettingsGoogleGeminiInstanceConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct SettingsGoogleGeminiInstanceConfig {
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
    image_models: Vec<String>,
    #[serde(default)]
    default_image_model: Option<String>,
    #[serde(default)]
    features: Vec<String>,
    #[serde(default)]
    alias_map: HashMap<String, String>,
}

fn default_gemini_enabled() -> bool {
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
    DEFAULT_GEMINI_BASE_URL.to_string()
}

fn default_timeout_ms() -> u64 {
    DEFAULT_GEMINI_TIMEOUT_MS
}

fn default_features() -> Vec<String> {
    vec![
        features::PLAN.to_string(),
        features::JSON_OUTPUT.to_string(),
    ]
}

fn is_text2image_model_name(model: &str) -> bool {
    let lowered = model.trim().to_ascii_lowercase();
    !lowered.contains("imagen") && (lowered.contains("image") || lowered.contains("nano-banana"))
}

#[derive(Debug, Clone, Copy)]
enum GeminiModelKind {
    Llm,
    Image,
    Embedding,
    Tts,
    Music,
    Video,
}

fn strip_gemini_model_prefix(name: &str) -> &str {
    name.strip_prefix("models/")
        .or_else(|| name.strip_prefix("tunedModels/"))
        .unwrap_or(name)
}

/// 若同一 bucket 里既有 alias `X` 又有它的数字后缀版本 `X-NNN`（NNN 是 2~4 位
/// 数字），就只保留 alias、把版本快照剔除。Google `/v1beta/models` 同时返回这两
/// 种命名，alias 通常生命周期更长，先停的是版本快照（`gemini-2.0-flash-001` /
/// `gemini-2.0-flash-lite-001` 这种）。
///
/// 注意：只有在同一份模型列表里 alias **本身**也存在时才剔除版本号；如果只剩
/// 版本号变体（比如 `gemini-embedding-001` 没 alias 兄弟），照样保留——它就是
/// 这个模型在 Google 那边唯一的命名。
fn prefer_alias_over_versioned(models: &mut Vec<String>) {
    let aliases: HashSet<String> = models
        .iter()
        .map(|name| name.to_ascii_lowercase())
        .collect();
    models.retain(|name| {
        let Some(alias_part) = strip_numeric_version_suffix(name) else {
            return true;
        };
        let alias_key = alias_part.to_ascii_lowercase();
        // 自己的小写形式肯定也在集合里，alias 必须是另一个不同的条目
        let alias_exists = aliases.contains(&alias_key) && alias_key != name.to_ascii_lowercase();
        !alias_exists
    });
}

/// 识别 `<base>-<digits>` 命名（如 `gemini-2.0-flash-001`），返回去掉 `-<digits>`
/// 之后的 `<base>`。识别规则约束 2~4 位纯数字尾巴，避免误吃 `gpt-4o` 这种本身
/// 名字里就带数字的情况。
fn strip_numeric_version_suffix(name: &str) -> Option<&str> {
    let bytes = name.as_bytes();
    let mut digits = 0usize;
    while digits < bytes.len() && bytes[bytes.len() - 1 - digits].is_ascii_digit() {
        digits += 1;
    }
    if !(2..=4).contains(&digits) {
        return None;
    }
    let split_at = bytes.len() - digits;
    if split_at == 0 || bytes[split_at - 1] != b'-' {
        return None;
    }
    Some(&name[..split_at - 1])
}

/// Google `/v1beta/models` 仍会列出已经下线的模型（比如 `gemini-2.0-flash-001`
/// 对新用户已不可用），但运行期才会以 `fatal: ... is no longer available to
/// new users` 的形式报出来。Provider 刷新模型列表本身就是为了"只保留能用的"，
/// 所以这里在写 inventory 之前就基于 displayName / description 上的 deprecation
/// 关键词把它们筛掉。匹配是大小写无关的子串匹配。
///
/// 这只是描述文字层面的过滤；如果 Google 哪天没在文案里写明就停服，最终还是要
/// 由运行期 health 反馈再降级一次（属于另外一条独立的链路）。
fn is_deprecated_gemini_entry(id: &str, display_name: &str, description: &str) -> bool {
    const SIGNALS: &[&str] = &[
        "deprecat",   // deprecated / deprecation
        "discontinu", // discontinued / discontinuation
        "no longer",  // "no longer available", "no longer supported"
        "retired",
        "(legacy)", // 用 "(legacy)" 而非裸 "legacy"，避免误伤 "Gemini Legacy Workshop" 这种命名
        "sunset",   // "sunset on ..."
        "end of life",
        "end-of-life",
    ];
    let haystack = format!("{} {} {}", id, display_name, description).to_ascii_lowercase();
    SIGNALS.iter().any(|signal| haystack.contains(signal))
}

fn classify_gemini_model(id: &str, methods: &HashSet<String>) -> Option<GeminiModelKind> {
    let configured = resolve_driver_inventory(
        "gemini-classifier",
        ProviderType::CloudApi,
        "google-gemini",
        &[DriverModelResolveRequest::new(
            id.to_string(),
            vec![ApiType::Llm],
        )],
        None,
    );
    if configured.models.first().is_some_and(|model| {
        model.api_types.iter().any(|api_type| {
            matches!(
                api_type,
                ApiType::VideoTextToVideo
                    | ApiType::VideoImageToVideo
                    | ApiType::VideoToVideo
                    | ApiType::VideoExtend
                    | ApiType::VideoUpscale
            )
        })
    }) {
        return Some(GeminiModelKind::Video);
    }
    let lowered = id.to_ascii_lowercase();

    if lowered.contains("embedding") || methods.contains("embedcontent") {
        return Some(GeminiModelKind::Embedding);
    }
    if lowered.contains("tts") {
        return Some(GeminiModelKind::Tts);
    }
    if lowered.contains("lyria") {
        return Some(GeminiModelKind::Music);
    }
    if lowered.contains("veo") {
        return Some(GeminiModelKind::Video);
    }
    if is_text2image_model_name(id) {
        return Some(GeminiModelKind::Image);
    }
    if methods.contains("generatecontent")
        || (methods.is_empty()
            && (lowered.starts_with("gemini")
                || lowered.starts_with("gemma-")
                || lowered == "aqa"))
    {
        return Some(GeminiModelKind::Llm);
    }
    None
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

fn parse_gemini_settings(settings: &Value) -> Result<Option<GeminiSettings>> {
    let raw = settings
        .get("gemini")
        .or_else(|| settings.get("google_gemini"))
        .or_else(|| settings.get("google"));
    let Some(raw_settings) = raw else {
        return Ok(None);
    };
    if raw_settings.is_null() {
        return Ok(None);
    }

    let gemini_settings = serde_json::from_value::<GeminiSettings>(raw_settings.clone())
        .map_err(|err| anyhow!("failed to parse gemini settings: {}", err))?;
    if !gemini_settings.enabled {
        return Ok(None);
    }

    Ok(Some(gemini_settings))
}

fn build_gemini_instances(settings: &GeminiSettings) -> Result<Vec<GoogleGeminiInstanceConfig>> {
    let raw_instances = if settings.instances.is_empty() {
        vec![SettingsGoogleGeminiInstanceConfig {
            provider_instance_name: default_instance_id(),
            provider_type: default_provider_type(),
            provider_driver: default_provider_driver(),
            api_token: settings.api_token.clone(),
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
            models = normalize_model_list(parse_csv_list(DEFAULT_GEMINI_MODELS));
        }
        if models.is_empty() {
            return Err(anyhow!(
                "gemini instance {} has no models configured",
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
            image_models = normalize_model_list(parse_csv_list(DEFAULT_GEMINI_IMAGE_MODELS));
        }
        let default_image_model = raw_instance
            .default_image_model
            .or_else(|| image_models.first().cloned());
        let features = if raw_instance.features.is_empty() {
            default_features()
        } else {
            raw_instance.features
        };

        instances.push(GoogleGeminiInstanceConfig {
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
            image_models,
            default_image_model,
            features,
            alias_map: raw_instance.alias_map,
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

pub fn register_google_gemini_providers(
    center: &AIComputeCenter,
    settings: &Value,
) -> Result<usize> {
    let Some(gemini_settings) = parse_gemini_settings(settings)? else {
        info!("aicc google gemini provider is disabled (gemini settings missing or disabled)");
        return Ok(0);
    };
    let instances = build_gemini_instances(&gemini_settings)?;
    let mut prepared = Vec::<(GoogleGeminiInstanceConfig, Arc<GoogleGeminiProvider>)>::new();
    for config in instances.iter() {
        if config.api_token.trim().is_empty() {
            return Err(anyhow!(
                "gemini instance {} api_token (or api_key) is required",
                config.provider_instance_name
            ));
        }
        let provider = Arc::new(GoogleGeminiProvider::new(
            config.clone(),
            config.api_token.clone(),
        )?);
        prepared.push((config.clone(), provider));
    }

    for (config, provider) in prepared.into_iter() {
        provider.clone().start_inventory_refresh();
        let inventory = center.registry().add_provider(provider);
        info!(
            "registered google gemini base_url={} inventory={:?}",
            config.base_url, inventory
        );
        center
            .model_registry()
            .write()
            .map_err(|_| anyhow!("model registry lock poisoned"))?
            .apply_inventory(inventory)
            .map_err(|err| anyhow!("failed to apply gemini inventory: {}", err))?;
    }

    Ok(instances.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aicc::ModelCatalog;
    use buckyos_api::{AiPayload, ModelSpec, Requirements};
    use serde_json::json;

    #[test]
    fn default_timeout_covers_high_latency_media() {
        assert_eq!(default_timeout_ms(), 300_000);
    }

    #[test]
    fn asr_rejects_text_when_no_speech_was_detected() {
        let assessment = assess_gemini_asr(&json!({
            "speech_detected": false,
            "transcript_confidence": 0.98,
            "text": "Welcome.",
            "segments": []
        }));

        assert_eq!(assessment.status, "no_speech");
        assert!(assessment.transcript.is_empty());
        assert_eq!(assessment.candidate_text, "Welcome.");
    }

    #[test]
    fn asr_keeps_uncertain_candidate_out_of_response_text() {
        let assessment = assess_gemini_asr(&json!({
            "speech_detected": true,
            "transcript_confidence": 0.4,
            "text": "Possible words",
            "segments": []
        }));

        assert_eq!(assessment.status, "uncertain");
        assert!(assessment.transcript.is_empty());
        assert_eq!(assessment.candidate_text, "Possible words");
    }

    #[test]
    fn asr_returns_clear_speech_as_transcript() {
        let assessment = assess_gemini_asr(&json!({
            "speech_detected": true,
            "transcript_confidence": 0.95,
            "text": "Clear speech",
            "segments": [{ "text": "Clear speech" }]
        }));

        assert_eq!(assessment.status, "reliable");
        assert_eq!(assessment.transcript, "Clear speech");
    }

    fn build_llm_request(
        must_features: Vec<Feature>,
        tool_specs: Vec<AiToolSpec>,
    ) -> AiMethodRequest {
        AiMethodRequest::new(
            Capability::Llm,
            ModelSpec::new("llm.chat".to_string(), None),
            Requirements::new(must_features, None, None, None),
            AiPayload::new(
                None,
                vec![AiMessage::text(AiRole::User, "hello")],
                tool_specs,
                vec![],
                None,
                None,
            ),
            None,
        )
    }

    #[test]
    fn llm_contents_append_payload_resources_to_user_message() {
        let mut request = build_llm_request(vec![], vec![]);
        request.payload.resources.push(ResourceRef::Base64 {
            mime: "audio/mpeg".to_string(),
            data_base64: "YXVkaW8=".to_string(),
        });

        let contents = GoogleGeminiProvider::build_contents(&request)
            .expect("LLM resources should lower to Gemini parts");

        assert_eq!(
            contents[0].pointer("/parts/1/inlineData/mimeType"),
            Some(&json!("audio/mpeg"))
        );
        assert_eq!(
            contents[0].pointer("/parts/1/inlineData/data"),
            Some(&json!("YXVkaW8="))
        );
    }

    #[test]
    fn interactions_asr_request_uses_official_transcription_config() {
        let mut request = build_llm_request(vec![], vec![]);
        request.payload.input_json = Some(json!({
            "language": "zh",
            "timestamps": "word",
            "diarization": true,
            "custom_vocabulary": ["BuckyOS"]
        }));
        let body = GoogleGeminiProvider::build_interactions_asr_request(
            "gemini-3.5-transcribe",
            &request,
            json!({ "type": "audio", "mime_type": "audio/wav", "data": "YXVkaW8=" }),
        )
        .expect("interactions transcription request");
        let body = Value::Object(body);

        assert_eq!(
            body.pointer("/model"),
            Some(&json!("gemini-3.5-transcribe"))
        );
        assert_eq!(body.pointer("/input/0/type"), Some(&json!("audio")));
        assert_eq!(
            body.pointer("/generation_config/transcription_config/language_codes/0"),
            Some(&json!("zh"))
        );
        assert_eq!(
            body.pointer("/generation_config/transcription_config/mode/diarization_mode"),
            Some(&json!("speaker"))
        );
        assert_eq!(
            body.pointer("/generation_config/transcription_config/mode/timestamp_granularities/0"),
            Some(&json!("word"))
        );
    }

    #[test]
    fn interactions_asr_response_extracts_text_annotations_and_usage() {
        let response = json!({
            "status": "completed",
            "steps": [{
                "type": "model_output",
                "content": [{
                    "type": "text",
                    "text": "hello world",
                    "annotations": [{
                        "type": "word_info",
                        "text": "hello",
                        "speaker": "spk_1",
                        "start_offset": "0.100s",
                        "end_offset": "0.450s"
                    }]
                }]
            }],
            "usage": {
                "total_input_tokens": 10,
                "total_output_tokens": 2,
                "total_tokens": 12
            }
        });

        assert_eq!(
            GoogleGeminiProvider::interactions_output_text(&response),
            "hello world"
        );
        assert_eq!(
            GoogleGeminiProvider::interactions_word_segments(&response),
            vec![json!({
                "type": "word_info",
                "text": "hello",
                "speaker": "spk_1",
                "start_offset": "0.100s",
                "end_offset": "0.450s"
            })]
        );
        assert_eq!(
            GoogleGeminiProvider::interactions_usage(&response),
            Some(AiUsage {
                input_tokens: Some(10),
                output_tokens: Some(2),
                total_tokens: Some(12),
                request_units: None,
            })
        );
    }

    #[test]
    fn llm_tools_include_function_declarations_and_google_search() {
        let request = build_llm_request(
            vec![
                features::TOOL_CALLING.to_string(),
                features::WEB_SEARCH.to_string(),
            ],
            vec![AiToolSpec {
                name: "get_weather".to_string(),
                description: "Get current weather".to_string(),
                args_schema: value_to_object_map(json!({
                    "type": "object",
                    "properties": {
                        "city": { "type": "string" }
                    },
                    "required": ["city"]
                })),
                output_schema: json!({ "type": "object" }),
            }],
        );
        let mut request_obj = Map::new();

        GoogleGeminiProvider::merge_llm_tools(
            &mut request_obj,
            "gemini-3.1-pro-preview",
            &request,
            true,
        )
        .expect("Gemini 3 should support combined tools");
        let request_value = Value::Object(request_obj);

        assert_eq!(
            request_value.pointer("/tools/0/functionDeclarations/0/name"),
            Some(&json!("get_weather"))
        );
        assert_eq!(
            request_value.pointer(
                "/tools/0/functionDeclarations/0/parametersJsonSchema/properties/city/type"
            ),
            Some(&json!("string"))
        );
        assert_eq!(
            request_value.pointer("/tools/0/functionDeclarations/0/responseJsonSchema/type"),
            Some(&json!("object"))
        );
        assert_eq!(
            request_value.pointer("/tools/1/googleSearch"),
            Some(&json!({}))
        );
        assert_eq!(
            request_value.pointer("/toolConfig/includeServerSideToolInvocations"),
            Some(&json!(true))
        );
        assert_eq!(
            request_value.pointer("/toolConfig/functionCallingConfig/mode"),
            Some(&json!("VALIDATED"))
        );
    }

    #[test]
    fn gemini_builtin_tool_alone_does_not_enable_combination_config() {
        let request = build_llm_request(vec![features::WEB_SEARCH.to_string()], vec![]);
        let mut request_obj = Map::new();

        GoogleGeminiProvider::merge_llm_tools(
            &mut request_obj,
            "gemini-3-flash-preview",
            &request,
            true,
        )
        .expect("Google Search should be enabled");

        assert!(request_obj.get("toolConfig").is_none());
        assert_eq!(
            Value::Object(request_obj).pointer("/tools/0/googleSearch"),
            Some(&json!({}))
        );
    }

    #[test]
    fn gemini_tool_schema_is_passed_through_as_json_schema() {
        let schema = json!({
            "type": "object",
            "additionalProperties": false,
            "definitions": {
                "RecallMode": {
                    "type": "string",
                    "enum": ["auto", "mechanical", "llm"]
                },
                "TagInput": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" }
                    },
                    "required": ["name"]
                }
            },
            "properties": {
                "mode": {
                    "anyOf": [
                        { "$ref": "#/definitions/RecallMode" },
                        { "type": "null" }
                    ]
                },
                "tags": {
                    "type": "array",
                    "items": { "$ref": "#/definitions/TagInput" }
                }
            }
        });

        let declaration = GoogleGeminiProvider::function_declaration(&AiToolSpec {
            name: "recall".to_string(),
            description: String::new(),
            args_schema: value_to_object_map(schema.clone()),
            output_schema: Value::Null,
        });

        assert_eq!(declaration.get("parametersJsonSchema"), Some(&schema));
        assert!(declaration.get("parameters").is_none());
    }

    #[test]
    fn gemini_response_schema_uses_json_schema_field() {
        let schema = json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["answer"],
            "properties": { "answer": { "type": "string" } }
        });
        let mut request = Map::new();

        GoogleGeminiProvider::merge_llm_options(
            &mut request,
            &json!({ "response_schema": schema.clone() }),
            true,
        )
        .expect("response schema should be accepted");

        let request = Value::Object(request);
        assert_eq!(
            request.pointer("/generationConfig/responseJsonSchema"),
            Some(&schema)
        );
        assert!(
            request
                .pointer("/generationConfig/responseSchema")
                .is_none()
        );
        assert_eq!(
            request.pointer("/generationConfig/responseMimeType"),
            Some(&json!("application/json"))
        );
    }

    #[test]
    fn gemini_2_5_rejects_combined_builtin_and_function_tools() {
        let request = build_llm_request(
            vec![
                features::TOOL_CALLING.to_string(),
                features::WEB_SEARCH.to_string(),
            ],
            vec![AiToolSpec {
                name: "get_weather".to_string(),
                description: String::new(),
                args_schema: value_to_object_map(json!({ "type": "object" })),
                output_schema: Value::Null,
            }],
        );

        let error = GoogleGeminiProvider::merge_llm_tools(
            &mut Map::new(),
            "gemini-2.5-flash",
            &request,
            false,
        )
        .expect_err("Gemini 2.5 must reject combined tools");

        assert!(error.to_string().contains("does not support combining"));
    }

    #[test]
    fn gemini_candidate_and_structured_tool_result_round_trip() {
        let body = json!({
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [
                        {
                            "text": "checking",
                            "thoughtSignature": "text-signed-state"
                        },
                        {
                            "toolCall": {
                                "id": "server-call-1",
                                "name": "google_search",
                                "args": { "query": "Shanghai weather" }
                            }
                        },
                        {
                            "toolResponse": {
                                "id": "server-call-1",
                                "name": "google_search",
                                "response": { "result": "search result" }
                            }
                        },
                        {
                            "functionCall": {
                                "id": "call-1",
                                "name": "get_weather",
                                "args": { "city": "Shanghai" }
                            },
                            "thoughtSignature": "signed-state"
                        }
                    ]
                }
            }]
        });

        let calls = GoogleGeminiProvider::extract_tool_calls(&body);

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].call_id, "call-1");
        assert_eq!(calls[0].name, "get_weather");
        assert_eq!(calls[0].args.get("city"), Some(&json!("Shanghai")));

        let mut message = AiResponse::message_from_parts(None, calls, vec![]);
        message.content.push(
            GoogleGeminiProvider::extract_provider_state(&body)
                .expect("candidate content should be preserved"),
        );
        let mut contents = Vec::new();
        let mut tool_calls = HashMap::new();
        GoogleGeminiProvider::lower_message_to_gemini(&message, &mut contents, &mut tool_calls)
            .expect("Gemini provider state should lower");
        assert_eq!(contents.len(), 1);
        assert_eq!(&contents[0], body.pointer("/candidates/0/content").unwrap());

        let tool_result = AiMessage::new(
            AiRole::Tool,
            vec![AiContent::tool_result_text(
                "call-1",
                r#"{"temperature":24,"condition":"sunny"}"#,
                false,
            )],
        );
        GoogleGeminiProvider::lower_message_to_gemini(&tool_result, &mut contents, &mut tool_calls)
            .expect("structured tool result should lower");
        assert_eq!(
            contents[1].pointer("/parts/0/functionResponse"),
            Some(&json!({
                "id": "call-1",
                "name": "get_weather",
                "response": { "temperature": 24, "condition": "sunny" }
            }))
        );
    }

    #[test]
    fn gemini_no_id_function_call_stays_without_provider_id() {
        let body = json!({
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [{
                        "functionCall": {
                            "name": "get_weather",
                            "args": { "city": "Shanghai" }
                        },
                        "thoughtSignature": "signed-state"
                    }]
                }
            }]
        });
        let calls = GoogleGeminiProvider::extract_tool_calls(&body);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].call_id, "gemini-no-id-0-get_weather");

        let mut message = AiResponse::message_from_parts(None, calls, vec![]);
        message.content.push(
            GoogleGeminiProvider::extract_provider_state(&body)
                .expect("candidate content should be preserved"),
        );
        let mut contents = Vec::new();
        let mut tool_calls = HashMap::new();
        GoogleGeminiProvider::lower_message_to_gemini(&message, &mut contents, &mut tool_calls)
            .expect("candidate should lower");
        let tool_result = AiMessage::new(
            AiRole::Tool,
            vec![AiContent::tool_result_text(
                "gemini-no-id-0-get_weather",
                "sunny",
                false,
            )],
        );
        GoogleGeminiProvider::lower_message_to_gemini(&tool_result, &mut contents, &mut tool_calls)
            .expect("tool result should lower");

        assert_eq!(&contents[0], body.pointer("/candidates/0/content").unwrap());
        assert!(contents[0].pointer("/parts/0/functionCall/id").is_none());
        assert!(
            contents[1]
                .pointer("/parts/0/functionResponse/id")
                .is_none()
        );
        assert_eq!(
            contents[1].pointer("/parts/0/functionResponse/name"),
            Some(&json!("get_weather"))
        );
        assert!(
            !serde_json::to_string(&contents)
                .unwrap()
                .contains("gemini-no-id")
        );
    }

    #[test]
    fn gemini_tool_result_keeps_function_name_and_call_id() {
        let messages = vec![
            AiResponse::message_from_parts(
                None,
                vec![AiToolCall {
                    call_id: "call-1".to_string(),
                    name: "get_weather".to_string(),
                    args: value_to_object_map(json!({ "city": "Shanghai" })),
                }],
                vec![],
            ),
            AiMessage::new(
                AiRole::Tool,
                vec![AiContent::tool_result_text("call-1", "sunny", false)],
            ),
        ];
        let mut contents = Vec::new();
        let mut tool_calls = HashMap::new();
        for message in &messages {
            GoogleGeminiProvider::lower_message_to_gemini(message, &mut contents, &mut tool_calls)
                .expect("message should lower");
        }

        assert_eq!(
            contents[1].pointer("/parts/0/functionResponse/name"),
            Some(&json!("get_weather"))
        );
        assert_eq!(
            contents[1].pointer("/parts/0/functionResponse/id"),
            Some(&json!("call-1"))
        );
        assert_eq!(
            contents[1].pointer("/parts/0/functionResponse/response"),
            Some(&json!({ "output": "sunny" }))
        );
    }

    #[test]
    fn gemini_models_pagination_rejects_repeated_tokens() {
        let mut seen = HashSet::new();
        assert_eq!(
            next_gemini_models_page_token(None, &mut seen).unwrap(),
            None
        );
        assert_eq!(
            next_gemini_models_page_token(Some(" page/2 ".to_string()), &mut seen)
                .unwrap()
                .as_deref(),
            Some("page/2")
        );
        assert!(next_gemini_models_page_token(Some("page/2".to_string()), &mut seen).is_err());
    }

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
    fn gemini_video_inventory_uses_configured_protocols() {
        let inventory = GoogleGeminiProvider::build_inventory_from_buckets(
            "gemini-primary",
            ProviderType::CloudApi,
            "google-gemini",
            &GeminiModelBuckets {
                video: vec![
                    "gemini-omni-flash-preview".to_string(),
                    "veo-3.1-generate-preview".to_string(),
                ],
                ..Default::default()
            },
            &[],
            Some("test".to_string()),
        );
        let veo = inventory
            .models
            .iter()
            .find(|model| model.provider_model_id == "veo-3.1-generate-preview")
            .expect("veo model should exist");
        assert!(veo.api_types.contains(&ApiType::VideoTextToVideo));
        assert!(veo.api_types.contains(&ApiType::VideoImageToVideo));
        assert!(!veo.api_types.contains(&ApiType::VideoToVideo));
        assert!(veo.api_types.contains(&ApiType::VideoExtend));
        assert_eq!(
            veo.provider_options
                .as_ref()
                .and_then(|options| options.get("protocol"))
                .and_then(Value::as_str),
            Some("predict_long_running")
        );
        assert!(
            veo.logical_mounts
                .iter()
                .any(|mount| mount == "video.img2video")
        );
        assert!(
            veo.logical_mounts
                .iter()
                .any(|mount| mount == "video.extend")
        );

        let omni = inventory
            .models
            .iter()
            .find(|model| model.provider_model_id == "gemini-omni-flash-preview")
            .expect("omni model should exist");
        assert!(omni.api_types.contains(&ApiType::VideoToVideo));
        assert!(!omni.api_types.contains(&ApiType::VideoExtend));
        assert_eq!(
            omni.provider_options
                .as_ref()
                .and_then(|options| options.get("protocol"))
                .and_then(Value::as_str),
            Some("interactions")
        );
    }

    #[test]
    fn veo_inventory_capabilities_follow_metadata_patterns() {
        let inventory = GoogleGeminiProvider::build_inventory_from_buckets(
            "gemini-primary",
            ProviderType::CloudApi,
            "google-gemini",
            &GeminiModelBuckets {
                video: vec![
                    "veo-3.1-fast-preview".to_string(),
                    "veo-3.1-fast-lite-preview".to_string(),
                ],
                ..Default::default()
            },
            &[],
            Some("test".to_string()),
        );
        let full = inventory
            .models
            .iter()
            .find(|model| model.provider_model_id == "veo-3.1-fast-preview")
            .expect("full veo model should exist");
        assert!(!full.api_types.contains(&ApiType::VideoToVideo));
        assert!(full.api_types.contains(&ApiType::VideoExtend));

        let lite = inventory
            .models
            .iter()
            .find(|model| model.provider_model_id == "veo-3.1-fast-lite-preview")
            .expect("lite veo model should exist");
        assert!(!lite.api_types.contains(&ApiType::VideoToVideo));
        assert!(!lite.api_types.contains(&ApiType::VideoExtend));
    }

    #[tokio::test]
    async fn interactions_video_input_uses_video_content_part() {
        let config = build_gemini_instances(&GeminiSettings {
            enabled: true,
            api_token: "token".to_string(),
            alias_map: HashMap::new(),
            instances: vec![],
        })
        .expect("instances")
        .remove(0);
        let provider = GoogleGeminiProvider::new(config, "token".to_string()).expect("provider");
        let resource = ResourceRef::Base64 {
            mime: "video/mp4".to_string(),
            data_base64: "dmlkZW8=".to_string(),
        };
        assert_eq!(
            provider
                .interactions_resource_part(&resource, "video")
                .await
                .unwrap(),
            json!({
                "type": "video",
                "mime_type": "video/mp4",
                "data": "dmlkZW8="
            })
        );
        let resource = ResourceRef::Url {
            url: "gs://example/input.mp4".to_string(),
            mime_hint: Some("video/mp4".to_string()),
        };
        assert_eq!(
            provider
                .interactions_resource_part(&resource, "video")
                .await
                .unwrap(),
            json!({
                "type": "video",
                "mime_type": "video/mp4",
                "uri": "gs://example/input.mp4"
            })
        );
    }

    #[test]
    fn nested_provider_options_configure_thinking_budget() {
        let mut target = Map::new();
        let ignored = GoogleGeminiProvider::merge_llm_options(
            &mut target,
            &json!({
                "provider_options": {
                    "protocol": "generate_content",
                    "thinking_budget": 0
                }
            }),
            false,
        )
        .expect("provider options should merge");

        let target = Value::Object(target);
        assert_eq!(
            target.pointer("/generationConfig/thinkingConfig/thinkingBudget"),
            Some(&json!(0))
        );
        assert_eq!(ignored, vec!["protocol".to_string()]);
    }

    #[test]
    fn quota_error_message_preserves_retry_and_metric_diagnostics() {
        let body = json!({
            "error": {
                "status": "RESOURCE_EXHAUSTED",
                "message": "quota exhausted",
                "details": [
                    { "retryDelay": "37s" },
                    {
                        "violations": [{
                            "quotaId": "GenerateRequestsPerDayPerProjectPerModel",
                            "quotaMetric": "generativelanguage.googleapis.com/generate_requests_per_model_per_day",
                            "quotaValue": "100"
                        }]
                    }
                ]
            }
        });

        let message = GoogleGeminiProvider::api_error_message(&body, "fallback");
        assert!(message.contains("RESOURCE_EXHAUSTED"));
        assert!(message.contains("retry_after=37s"));
        assert!(message.contains("GenerateRequestsPerDayPerProjectPerModel"));
        assert!(message.contains("generate_requests_per_model_per_day"));
        assert!(message.contains("quota_value=\"100\""));
    }

    #[test]
    fn required_tool_choice_forces_gemini_function_calling() {
        let mut target = Map::new();
        let ignored = GoogleGeminiProvider::merge_llm_options(
            &mut target,
            &json!({ "tool_choice": "required" }),
            false,
        )
        .expect("required tool choice should merge");

        assert!(ignored.is_empty());
        assert_eq!(
            Value::Object(target).pointer("/toolConfig/functionCallingConfig/mode"),
            Some(&json!("ANY"))
        );
    }

    #[test]
    fn thinking_budget_is_added_outside_completion_budget_and_capped() {
        let mut request = json!({
            "generationConfig": {
                "maxOutputTokens": 4096,
                "thinkingConfig": { "thinkingBudget": 512 }
            }
        })
        .as_object()
        .cloned()
        .unwrap();
        assert_eq!(
            GoogleGeminiProvider::apply_separate_thinking_budget(&mut request, Some(4_500)),
            Some(4_096)
        );
        assert_eq!(
            Value::Object(request)
                .pointer("/generationConfig/maxOutputTokens")
                .and_then(Value::as_u64),
            Some(4_500)
        );
    }

    #[test]
    fn gemini_25_metadata_uses_current_output_limit() {
        let inventory = GoogleGeminiProvider::build_inventory_from_buckets(
            "gemini-primary",
            ProviderType::CloudApi,
            "google-gemini",
            &GeminiModelBuckets {
                llm: vec!["gemini-2.5-flash".to_string()],
                ..Default::default()
            },
            &[],
            Some("test".to_string()),
        );
        let model = inventory
            .models
            .iter()
            .find(|model| model.provider_model_id == "gemini-2.5-flash")
            .unwrap();
        assert_eq!(model.capabilities.max_output_tokens, Some(65_536));
        assert!(model.api_types.contains(&ApiType::AudioAsr));
        assert!(
            model
                .logical_mounts
                .iter()
                .any(|mount| mount == "audio.asr")
        );
    }

    #[test]
    fn audio_asr_inventory_uses_metadata_model_patterns() {
        let inventory = GoogleGeminiProvider::build_inventory_from_buckets(
            "gemini-primary",
            ProviderType::CloudApi,
            "google-gemini",
            &GeminiModelBuckets {
                llm: vec![
                    "gemini-pro-latest".to_string(),
                    "gemini-2.5-pro".to_string(),
                    "gemini-3.1-pro-preview".to_string(),
                    "gemini-3.1-pro-preview-001".to_string(),
                    "gemini-3.5-flash-lite".to_string(),
                    "gemini-3.5-transcribe".to_string(),
                    "gemini-deepthink-preview".to_string(),
                    "gemini-future-model".to_string(),
                ],
                tts: vec![
                    "gemini-2.5-flash-preview-tts".to_string(),
                    "gemini-2.5-pro-preview-tts".to_string(),
                ],
                image: vec!["gemini-3.1-flash-image".to_string()],
                ..Default::default()
            },
            &[],
            Some("test".to_string()),
        );

        for id in [
            "gemini-pro-latest",
            "gemini-2.5-pro",
            "gemini-3.1-pro-preview",
            "gemini-3.1-pro-preview-001",
            "gemini-3.5-flash-lite",
            "gemini-3.5-transcribe",
        ] {
            let model = inventory
                .models
                .iter()
                .find(|model| model.provider_model_id == id)
                .unwrap();
            assert!(model.api_types.contains(&ApiType::AudioAsr), "{id}");
            if id == "gemini-3.5-transcribe" {
                assert_eq!(
                    model
                        .provider_options
                        .as_ref()
                        .and_then(|options| options.get("protocol"))
                        .and_then(Value::as_str),
                    Some("interactions")
                );
                assert_eq!(model.api_types, vec![ApiType::AudioAsr]);
            }
        }
        for id in [
            "gemini-deepthink-preview",
            "gemini-future-model",
            "gemini-2.5-flash-preview-tts",
            "gemini-2.5-pro-preview-tts",
            "gemini-3.1-flash-image",
        ] {
            let model = inventory
                .models
                .iter()
                .find(|model| model.provider_model_id == id)
                .unwrap();
            assert!(!model.api_types.contains(&ApiType::AudioAsr), "{id}");
            assert!(
                !model
                    .logical_mounts
                    .iter()
                    .any(|mount| mount.starts_with("audio.asr")),
                "{id}"
            );
        }
    }

    #[test]
    fn embedding_inventory_only_exposes_multimodal_for_embedding_2() {
        let inventory = GoogleGeminiProvider::build_inventory_from_buckets(
            "gemini-primary",
            ProviderType::CloudApi,
            "google-gemini",
            &GeminiModelBuckets {
                embedding: vec![
                    "gemini-embedding-001".to_string(),
                    "gemini-embedding-2".to_string(),
                ],
                ..Default::default()
            },
            &[],
            Some("test".to_string()),
        );
        let text_only = inventory
            .models
            .iter()
            .find(|model| model.provider_model_id == "gemini-embedding-001")
            .expect("embedding-001 should exist");
        assert_eq!(text_only.api_types, vec![ApiType::Embedding]);
        let multimodal = inventory
            .models
            .iter()
            .find(|model| model.provider_model_id == "gemini-embedding-2")
            .expect("embedding-2 should exist");
        assert!(multimodal.api_types.contains(&ApiType::Embedding));
        assert!(multimodal.api_types.contains(&ApiType::EmbeddingMultimodal));
    }

    #[test]
    fn inventory_uses_current_lyria_and_excludes_imagen() {
        let inventory = GoogleGeminiProvider::build_inventory_from_buckets(
            "gemini-primary",
            ProviderType::CloudApi,
            "google-gemini",
            &GeminiModelBuckets {
                image: vec![
                    "gemini-2.5-flash-image".to_string(),
                    "imagen-4.0-generate-001".to_string(),
                ],
                music: vec![
                    "lyria-3-clip-preview".to_string(),
                    "lyria-3-pro-preview".to_string(),
                ],
                ..Default::default()
            },
            &[],
            Some("test".to_string()),
        );
        assert!(
            inventory
                .models
                .iter()
                .any(|model| model.provider_model_id == "gemini-2.5-flash-image")
        );
        assert!(
            inventory
                .models
                .iter()
                .any(|model| model.provider_model_id == "lyria-3-clip-preview")
        );
        assert!(
            inventory
                .models
                .iter()
                .any(|model| model.provider_model_id == "lyria-3-pro-preview")
        );
        assert!(
            !inventory
                .models
                .iter()
                .any(|model| model.provider_model_id.starts_with("imagen-"))
        );
    }

    #[test]
    fn image_preferences_map_to_gemini_response_format() {
        let input = json!({
            "prompt": "draw a cat",
            "negative_prompt": "blurry",
            "aspect_ratio": "16:9",
            "size": "2048x1152",
            "output": { "media_type": "image/jpeg" }
        });
        let mut request = Map::new();
        GoogleGeminiProvider::merge_text2image_input_json(&mut request, &input).unwrap();
        GoogleGeminiProvider::merge_image_prompt_preferences(
            &mut request,
            input.as_object().expect("image input object"),
        );
        assert_eq!(
            request.get("prompt"),
            Some(&json!("draw a cat\nAvoid: blurry"))
        );
        assert_eq!(
            request
                .get("generationConfig")
                .and_then(|value| value.pointer("/responseFormat/image")),
            Some(&json!({
                "aspectRatio": "16:9",
                "imageSize": "2K",
                "mimeType": "IMAGE_JPEG"
            }))
        );
    }

    #[test]
    fn music_fields_are_preserved_in_prompt() {
        let mut req = build_text2image_request(None);
        req.payload.text = None;
        req.payload.input_json = Some(json!({
            "prompt": "an ambient synth track",
            "duration": 45,
            "lyrics": "Across the sky"
        }));
        let prompt = GoogleGeminiProvider::music_prompt(&req).unwrap();
        assert!(prompt.contains("Target duration: 45 seconds"));
        assert!(prompt.contains("Lyrics:\nAcross the sky"));
    }

    #[test]
    fn video_uri_parses_gemini_operation_response() {
        let operation = json!({
            "done": true,
            "response": {
                "generateVideoResponse": {
                    "generatedSamples": [{
                        "video": { "uri": "https://example.com/video.mp4" }
                    }]
                }
            }
        });
        assert_eq!(
            GoogleGeminiProvider::video_uri(&operation),
            Some("https://example.com/video.mp4")
        );
    }

    #[test]
    fn video_inline_data_parses_gemini_operation_response() {
        let operation = json!({
            "done": true,
            "response": {
                "generateVideoResponse": {
                    "generatedSamples": [{
                        "video": {
                            "mimeType": "video/mp4",
                            "bytesBase64Encoded": "dmlkZW8="
                        }
                    }]
                }
            }
        });
        assert_eq!(
            GoogleGeminiProvider::video_inline_data(&operation),
            Some(("video/mp4", "dmlkZW8="))
        );
    }

    #[test]
    fn estimate_video_cost_uses_veo_long_task_latency() {
        let provider = GoogleGeminiProvider::new(
            GoogleGeminiInstanceConfig {
                provider_instance_name: "gemini-primary".to_string(),
                provider_type: "cloud_api".to_string(),
                provider_driver: "google-gemini".to_string(),
                api_token: "token".to_string(),
                base_url: default_base_url(),
                timeout_ms: default_timeout_ms(),
                models: vec![],
                default_model: None,
                image_models: vec![],
                default_image_model: None,
                features: vec![],
                alias_map: HashMap::new(),
            },
            "token".to_string(),
        )
        .expect("provider should be built");
        let estimate = provider.estimate_cost(&CostEstimateInput {
            api_type: ApiType::VideoImageToVideo,
            exact_model: "veo-3.1-generate-preview@gemini-primary".to_string(),
            input_tokens: 0,
            estimated_output_tokens: None,
            cached_input_tokens: None,
            request_features: vec![],
        });
        assert_eq!(estimate.estimated_cost_usd, 0.5);
        assert_eq!(estimate.estimated_latency_ms, Some(120_000));
    }

    #[test]
    fn strip_numeric_version_suffix_matches_only_short_digit_tails() {
        assert_eq!(
            strip_numeric_version_suffix("gemini-2.0-flash-001"),
            Some("gemini-2.0-flash")
        );
        assert_eq!(
            strip_numeric_version_suffix("gemini-2.5-flash-002"),
            Some("gemini-2.5-flash")
        );
        // 4 位也算（保险一点，比如 -1234）
        assert_eq!(
            strip_numeric_version_suffix("gemini-x-1234"),
            Some("gemini-x")
        );
        // 1 位太短，可能是模型自带后缀，不算版本号
        assert_eq!(strip_numeric_version_suffix("gpt-4"), None);
        // 5 位以上不算（避免误吃像 claude-3-7-sonnet-20250219 这种 datestamp）
        assert_eq!(
            strip_numeric_version_suffix("claude-3-7-sonnet-20250219"),
            None
        );
        // 没有 `-` 边界
        assert_eq!(strip_numeric_version_suffix("gpt4o"), None);
        // 名字本身是纯数字
        assert_eq!(strip_numeric_version_suffix("123"), None);
    }

    #[test]
    fn prefer_alias_over_versioned_drops_version_when_alias_present() {
        let mut models = vec![
            "gemini-2.0-flash".to_string(),
            "gemini-2.0-flash-001".to_string(),
            "gemini-2.0-flash-lite".to_string(),
            "gemini-2.0-flash-lite-001".to_string(),
            "gemini-2.5-flash".to_string(),
            "gemini-2.5-flash-002".to_string(),
            "gemini-2.5-pro".to_string(),
            // 没 alias 兄弟，要保留
            "gemini-embedding-001".to_string(),
            // claude-style datestamp（8 位）不被识别为版本号，原样保留
            "claude-3-7-sonnet-20250219".to_string(),
        ];
        prefer_alias_over_versioned(&mut models);
        assert_eq!(
            models,
            vec![
                "gemini-2.0-flash".to_string(),
                "gemini-2.0-flash-lite".to_string(),
                "gemini-2.5-flash".to_string(),
                "gemini-2.5-pro".to_string(),
                "gemini-embedding-001".to_string(),
                "claude-3-7-sonnet-20250219".to_string(),
            ]
        );
    }

    #[test]
    fn prefer_alias_over_versioned_keeps_lonely_versioned() {
        // 只有版本快照、没有 alias 兄弟 → 保留
        let mut models = vec![
            "text-embedding-004".to_string(),
            "veo-3.1-generate-preview".to_string(),
        ];
        prefer_alias_over_versioned(&mut models);
        assert_eq!(
            models,
            vec![
                "text-embedding-004".to_string(),
                "veo-3.1-generate-preview".to_string(),
            ]
        );
    }

    #[test]
    fn classify_generate_content_models_without_gemini_prefix() {
        let methods = HashSet::from(["generatecontent".to_string()]);
        assert!(matches!(
            classify_gemini_model("gemma-4-31b-it", &methods),
            Some(GeminiModelKind::Llm)
        ));
        assert!(matches!(
            classify_gemini_model("aqa", &methods),
            Some(GeminiModelKind::Llm)
        ));
    }

    #[test]
    fn deprecated_gemini_entries_are_filtered() {
        // 描述里出现 deprecation 信号 → 必须过滤
        assert!(is_deprecated_gemini_entry(
            "gemini-2.0-flash-001",
            "Gemini 2.0 Flash (Discontinued)",
            "Stable version of Gemini 2.0 Flash."
        ));
        assert!(is_deprecated_gemini_entry(
            "gemini-1.5-pro-002",
            "Gemini 1.5 Pro",
            "This model is deprecated. Please migrate to gemini-2.5-pro."
        ));
        assert!(is_deprecated_gemini_entry(
            "gemini-1.0-pro",
            "Gemini 1.0 Pro",
            "Legacy: gemini-1.0-pro is no longer available to new users.",
        ));
        assert!(is_deprecated_gemini_entry(
            "gemini-2.0-flash-lite",
            "Gemini 2.0 Flash-Lite",
            "Will be retired on 2026-01-01."
        ));

        // 健康的当前模型不能误伤
        assert!(!is_deprecated_gemini_entry(
            "gemini-2.5-flash",
            "Gemini 2.5 Flash",
            "Fast and versatile multimodal model."
        ));
        assert!(!is_deprecated_gemini_entry(
            "gemini-2.5-pro",
            "Gemini 2.5 Pro",
            "Most capable Gemini model for complex reasoning tasks."
        ));
    }

    #[test]
    fn build_gemini_instances_infers_image_models() {
        let settings = GeminiSettings {
            enabled: true,
            api_token: "token".to_string(),
            alias_map: HashMap::new(),
            instances: vec![SettingsGoogleGeminiInstanceConfig {
                provider_instance_name: "gemini-1".to_string(),
                provider_type: "cloud_api".to_string(),
                provider_driver: "google-gemini".to_string(),
                api_token: String::new(),
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

        let instances = build_gemini_instances(&settings).expect("instances should be built");
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
    fn build_gemini_instances_uses_gemini_default_names() {
        let settings = GeminiSettings {
            enabled: true,
            api_token: "token".to_string(),
            alias_map: HashMap::new(),
            instances: vec![],
        };

        let instances = build_gemini_instances(&settings).expect("instances should be built");
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].provider_instance_name, "google-gemini-default");
        assert_eq!(instances[0].provider_driver, "google-gemini");
        for model in [
            "gemini-3.7-flash",
            "gemini-3.6-flash",
            "gemini-2.5-flash",
            "gemini-2.5-pro",
            "gemini-3.5-transcribe",
        ] {
            assert!(instances[0].models.iter().any(|item| item == model), "{model}");
        }
        for model in ["gemini-3.1-flash-image", "gemini-2.5-flash-image"] {
            assert!(
                instances[0].image_models.iter().any(|item| item == model),
                "{model}"
            );
        }
    }

    #[tokio::test]
    async fn stopped_refresh_does_not_send_initial_request() {
        let config = build_gemini_instances(&GeminiSettings {
            enabled: true,
            api_token: "token".to_string(),
            alias_map: HashMap::new(),
            instances: vec![],
        })
        .expect("instances")
        .remove(0);
        let provider = Arc::new(
            GoogleGeminiProvider::new(config, "token".to_string()).expect("provider should build"),
        );
        let provider_instance_name = provider.provider_instance_name.clone();
        let (refresh_task, shutdown_rx) = ProviderRefreshTask::new();
        refresh_task.shutdown();

        GoogleGeminiProvider::run_inventory_refresh(
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
        let config = build_gemini_instances(&GeminiSettings {
            enabled: true,
            api_token: "token".to_string(),
            alias_map: HashMap::new(),
            instances: vec![],
        })
        .expect("instances")
        .remove(0);
        let provider = GoogleGeminiProvider::new(config, "token".to_string()).expect("provider");
        let (refresh_task, _) = ProviderRefreshTask::new();
        *provider.refresh_task.lock().expect("refresh task lock") = Some(refresh_task.clone());

        drop(provider);

        assert!(refresh_task.is_stopped());
    }

    #[tokio::test]
    async fn registration_error_does_not_start_prepared_refresh() {
        let center = AIComputeCenter::default();
        let settings = json!({
            "gemini": {
                "enabled": true,
                "instances": [
                    {
                        "provider_instance_name": "gemini-valid",
                        "api_token": "token",
                        "base_url": "http://127.0.0.1:1"
                    },
                    {
                        "provider_instance_name": "gemini-invalid"
                    }
                ]
            }
        });

        let result = register_google_gemini_providers(&center, &settings);

        assert!(result.is_err());
        assert!(center.registry().inventories().is_empty());
    }

    #[test]
    fn register_gemini_inventory_exposes_stable_gemini_mounts() {
        let center = AIComputeCenter::default();
        let settings = json!({
            "gemini": {
                "enabled": true,
                "api_token": "token",
                "instances": [
                    {
                        "provider_instance_name": "google-gemini-default",
                        "provider_type": "cloud_api",
                        "provider_driver": "google-gemini",
                        "base_url": "https://generativelanguage.googleapis.com/v1beta",
                        "models": ["gemini-2.5-flash", "gemini-2.5-pro", "gemini-3.1-pro-preview"],
                        "image_models": ["gemini-2.5-flash-image-preview"]
                    }
                ]
            }
        });

        let count =
            register_google_gemini_providers(&center, &settings).expect("register should work");
        assert_eq!(count, 1);

        let registry = center.model_registry().read().expect("model registry lock");
        let flash_items = registry.default_items_for_path("llm.gemini-flash");
        assert!(
            flash_items
                .values()
                .any(|item| { item.target == "gemini-2.5-flash@google-gemini-default" })
        );
        let pro_items = registry.default_items_for_path("llm.gemini-pro");
        assert!(
            pro_items
                .values()
                .any(|item| item.target == "gemini-2.5-pro@google-gemini-default")
        );
        let inventories = center.registry().inventories();
        let inventory = inventories
            .iter()
            .find(|inventory| inventory.provider_instance_name == "google-gemini-default")
            .expect("Gemini inventory");
        let flash = inventory
            .models
            .iter()
            .find(|model| model.provider_model_id == "gemini-2.5-flash")
            .expect("Gemini 2.5 Flash metadata");
        assert!(flash.capabilities.web_search);
        assert_eq!(
            flash.capabilities.unsupported_feature_combinations,
            vec![vec![
                features::TOOL_CALLING.to_string(),
                features::WEB_SEARCH.to_string()
            ]]
        );
        let gemini_3 = inventory
            .models
            .iter()
            .find(|model| model.provider_model_id == "gemini-3.1-pro-preview")
            .expect("Gemini 3 metadata");
        assert!(gemini_3.capabilities.web_search);
        assert!(
            gemini_3
                .capabilities
                .unsupported_feature_combinations
                .is_empty()
        );
        let image_items = registry.default_items_for_path("image.txt2img.gemini");
        assert!(
            image_items.values().any(|item| {
                item.target == "gemini-2.5-flash-image-preview@google-gemini-default"
            })
        );
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
            GoogleGeminiProvider::estimate_text2image_cost(
                &preview,
                "gemini-2.5-flash-image-preview"
            ),
            Some(0.078)
        );

        let legacy = build_text2image_request(None);
        assert_eq!(
            GoogleGeminiProvider::estimate_text2image_cost(
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
            GoogleGeminiProvider::parse_text2image_result(&body).expect("artifacts should parse");
        assert_eq!(artifacts.len(), 1);
        assert!(text.is_none());
        match &artifacts[0].resource {
            ResourceRef::Base64 { data_base64, .. } => assert_eq!(data_base64, "aGVsbG8="),
            other => panic!("unexpected resource: {:?}", other),
        }
    }
}
