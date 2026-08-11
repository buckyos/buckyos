use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

pub(crate) fn is_model_feature_name(feature: &str) -> bool {
    matches!(
        feature,
        "streaming" | "tool_calling" | "json_output" | "web_search" | "vision"
    )
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct ExactModelName {
    pub provider_model_id: String,
    pub provider_instance_name: String,
}

impl ExactModelName {
    pub fn parse(value: &str) -> Result<Self, RouteError> {
        let Some((provider_model_id, provider_instance_name)) = value.rsplit_once('@') else {
            return Err(RouteError::new(
                RouteErrorCode::InvalidModelName,
                "exact model name must contain provider instance suffix",
            ));
        };

        if provider_model_id.trim().is_empty() || provider_instance_name.trim().is_empty() {
            return Err(RouteError::new(
                RouteErrorCode::InvalidModelName,
                "exact model name contains empty model or provider instance",
            ));
        }
        if !is_valid_provider_instance_name(provider_instance_name) {
            return Err(RouteError::new(
                RouteErrorCode::InvalidModelName,
                "provider instance name is invalid",
            ));
        }

        Ok(Self {
            provider_model_id: provider_model_id.to_string(),
            provider_instance_name: provider_instance_name.to_string(),
        })
    }

    pub fn as_string(&self) -> String {
        format!("{}@{}", self.provider_model_id, self.provider_instance_name)
    }
}

impl fmt::Display for ExactModelName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_string().as_str())
    }
}

impl FromStr for ExactModelName {
    type Err = RouteError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

pub fn is_valid_provider_instance_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|item| item.is_ascii_alphanumeric() || matches!(item, b'_' | b'-' | b'.'))
}

pub fn is_exact_model_name(value: &str) -> bool {
    value.contains('@')
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiType {
    #[serde(rename = "llm")]
    Llm,
    #[serde(rename = "image.txt2img")]
    ImageTextToImage,
    #[serde(rename = "image.img2img")]
    ImageToImage,
    #[serde(rename = "embedding.text")]
    Embedding,
    #[serde(rename = "embedding.multimodal")]
    EmbeddingMultimodal,
    #[serde(rename = "rerank")]
    Rerank,
    #[serde(rename = "image.inpaint")]
    ImageInpaint,
    #[serde(rename = "image.upscale")]
    ImageUpscale,
    #[serde(rename = "image.bg_remove")]
    ImageBgRemove,
    #[serde(rename = "vision.ocr")]
    VisionOcr,
    #[serde(rename = "vision.caption")]
    VisionCaption,
    #[serde(rename = "vision.detect")]
    VisionDetect,
    #[serde(rename = "vision.segment")]
    VisionSegment,
    #[serde(rename = "audio.tts")]
    AudioTts,
    #[serde(rename = "audio.asr")]
    AudioAsr,
    #[serde(rename = "audio.music")]
    AudioMusic,
    #[serde(rename = "audio.enhance")]
    AudioEnhance,
    #[serde(rename = "video.txt2video")]
    VideoTextToVideo,
    #[serde(rename = "video.img2video")]
    VideoImageToVideo,
    #[serde(rename = "video.video2video")]
    VideoToVideo,
    #[serde(rename = "video.extend")]
    VideoExtend,
    #[serde(rename = "video.upscale")]
    VideoUpscale,
    #[serde(rename = "agent.computer_use")]
    AgentComputerUse,
}

impl ApiType {
    pub fn namespace(&self) -> &'static str {
        match self {
            Self::Llm => "llm",
            Self::Embedding | Self::EmbeddingMultimodal => "embedding",
            Self::Rerank => "rerank",
            Self::ImageTextToImage
            | Self::ImageToImage
            | Self::ImageInpaint
            | Self::ImageUpscale
            | Self::ImageBgRemove => "image",
            Self::VisionOcr | Self::VisionCaption | Self::VisionDetect | Self::VisionSegment => {
                "vision"
            }
            Self::AudioTts | Self::AudioAsr | Self::AudioMusic | Self::AudioEnhance => "audio",
            Self::VideoTextToVideo
            | Self::VideoImageToVideo
            | Self::VideoToVideo
            | Self::VideoExtend
            | Self::VideoUpscale => "video",
            Self::AgentComputerUse => "agent",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    LocalInference,
    CloudApi,
    ProxyUnknown,
}

impl Default for ProviderType {
    fn default() -> Self {
        Self::ProxyUnknown
    }
}

impl fmt::Display for ProviderType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::LocalInference => "local_inference",
            Self::CloudApi => "cloud_api",
            Self::ProxyUnknown => "proxy_unknown",
        };
        f.write_str(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderOrigin {
    SystemConfig,
    UserConfig,
    BuiltIn,
    ProviderClaimed,
    Unknown,
}

impl Default for ProviderOrigin {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderTypeTrustedSource {
    SystemConfig,
    AdminOverride,
    ProviderInventory,
    DefaultUnknown,
}

impl Default for ProviderTypeTrustedSource {
    fn default() -> Self {
        Self::DefaultUnknown
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PricingMode {
    PerToken,
    Subscription,
    FreeQuota,
    Unknown,
}

impl Default for PricingMode {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CostEstimateInput {
    pub api_type: ApiType,
    pub exact_model: String,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub estimated_output_tokens: Option<u64>,
    #[serde(default)]
    pub cached_input_tokens: Option<u64>,
    #[serde(default)]
    pub request_features: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CostEstimateOutput {
    pub estimated_cost_usd: f64,
    #[serde(default)]
    pub pricing_mode: PricingMode,
    #[serde(default)]
    pub quota_state: QuotaState,
    pub confidence: f64,
    #[serde(default)]
    pub estimated_latency_ms: Option<u64>,
}

impl Default for CostEstimateOutput {
    fn default() -> Self {
        Self {
            estimated_cost_usd: 1.0,
            pricing_mode: PricingMode::Unknown,
            quota_state: QuotaState::Unknown,
            confidence: 0.0,
            estimated_latency_ms: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Available,
    Degraded,
    Unavailable,
}

impl Default for HealthStatus {
    fn default() -> Self {
        Self::Available
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaState {
    Normal,
    NearLimit,
    Exhausted,
    Unknown,
}

impl Default for QuotaState {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LatencyClass {
    Fast,
    Normal,
    Slow,
    Unknown,
}

impl Default for LatencyClass {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostClass {
    Low,
    Medium,
    High,
    Unknown,
}

impl Default for CostClass {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyClass {
    Local,
    Cloud,
    PrivateSafe,
    PublicCloud,
    Unknown,
}

impl Default for PrivacyClass {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ModelCapabilities {
    #[serde(default)]
    pub streaming: bool,
    #[serde(default)]
    pub tool_call: bool,
    #[serde(default)]
    pub json_schema: bool,
    #[serde(default)]
    pub web_search: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unsupported_feature_combinations: Vec<Vec<String>>,
    #[serde(default)]
    pub vision: bool,
    #[serde(default)]
    pub max_context_tokens: Option<u64>,
    #[serde(default)]
    pub max_output_tokens: Option<u64>,
}

impl ModelCapabilities {
    pub fn supports(&self, required: &RequiredModelFeatures) -> bool {
        let required_features = required.feature_names();
        (!required.streaming || self.streaming)
            && (!required.tool_call || self.tool_call)
            && (!required.json_schema || self.json_schema)
            && (!required.web_search || self.web_search)
            && self
                .unsupported_combination(required_features.as_slice())
                .is_none()
            && (!required.vision || self.vision)
            && required
                .min_context_tokens
                .map(|min| self.max_context_tokens.unwrap_or(0) >= min)
                .unwrap_or(true)
    }

    pub fn explain_missing_requirements(&self, required: &ModelRequirement) -> Vec<String> {
        let mut missing = Vec::new();
        if required.streaming && !self.streaming {
            missing.push("streaming".to_string());
        }
        if required.tool_call && !self.tool_call {
            missing.push("tool_call".to_string());
        }
        if required.json_schema && !self.json_schema {
            missing.push("json_schema".to_string());
        }
        if required.web_search && !self.web_search {
            missing.push("web_search".to_string());
        }
        let required_features = required.feature_names();
        if let Some(combination) = self
            .unsupported_combination(required_features.as_slice())
            .filter(|combination| {
                combination
                    .iter()
                    .all(|feature| self.supports_feature(feature))
            })
        {
            let mut combination = combination.to_vec();
            combination.sort();
            missing.push(format!(
                "unsupported_feature_combination:{}",
                combination.join("+")
            ));
        }
        if required.vision && !self.vision {
            missing.push("vision".to_string());
        }
        if let Some(min) = required.min_context_tokens {
            if self.max_context_tokens.unwrap_or(0) < min {
                missing.push(format!("min_context_tokens:{}", min));
            }
        }
        missing
    }

    pub fn set_feature_combination_supported(&mut self, features: &[&str], supported: bool) {
        let mut combination = features
            .iter()
            .map(|feature| feature.to_string())
            .collect::<Vec<_>>();
        combination.sort();
        combination.dedup();
        if combination.len() < 2 {
            return;
        }
        self.unsupported_feature_combinations.retain(|existing| {
            let mut existing = existing.clone();
            existing.sort();
            existing.dedup();
            existing != combination
        });
        if !supported {
            self.unsupported_feature_combinations.push(combination);
        }
    }

    pub fn supports_feature_combination(&self, features: &[&str]) -> bool {
        let features = features
            .iter()
            .map(|feature| feature.to_string())
            .collect::<Vec<_>>();
        self.unsupported_combination(features.as_slice()).is_none()
    }

    fn unsupported_combination(&self, required_features: &[String]) -> Option<&[String]> {
        self.unsupported_feature_combinations
            .iter()
            .find(|combination| {
                combination
                    .iter()
                    .all(|feature| required_features.iter().any(|required| required == feature))
            })
            .map(Vec::as_slice)
    }

    fn supports_feature(&self, feature: &str) -> bool {
        match feature {
            "streaming" => self.streaming,
            "tool_calling" => self.tool_call,
            "json_output" => self.json_schema,
            "web_search" => self.web_search,
            "vision" => self.vision,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RequiredModelFeatures {
    #[serde(default)]
    pub streaming: bool,
    #[serde(default)]
    pub tool_call: bool,
    #[serde(default)]
    pub json_schema: bool,
    #[serde(default)]
    pub web_search: bool,
    #[serde(default)]
    pub vision: bool,
    #[serde(default)]
    pub min_context_tokens: Option<u64>,
}

impl RequiredModelFeatures {
    fn feature_names(&self) -> Vec<String> {
        ModelRequirement {
            streaming: self.streaming,
            tool_call: self.tool_call,
            json_schema: self.json_schema,
            web_search: self.web_search,
            vision: self.vision,
            min_context_tokens: None,
        }
        .feature_names()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ModelRequirement {
    #[serde(default)]
    pub streaming: bool,
    #[serde(default)]
    pub tool_call: bool,
    #[serde(default)]
    pub json_schema: bool,
    #[serde(default)]
    pub web_search: bool,
    #[serde(default)]
    pub vision: bool,
    #[serde(default)]
    pub min_context_tokens: Option<u64>,
}

impl ModelRequirement {
    pub fn is_satisfied_by(&self, capabilities: &ModelCapabilities) -> bool {
        capabilities.explain_missing_requirements(self).is_empty()
    }

    pub fn feature_names(&self) -> Vec<String> {
        let mut features = Vec::new();
        if self.streaming {
            features.push("streaming".to_string());
        }
        if self.tool_call {
            features.push("tool_calling".to_string());
        }
        if self.json_schema {
            features.push("json_output".to_string());
        }
        if self.web_search {
            features.push("web_search".to_string());
        }
        if self.vision {
            features.push("vision".to_string());
        }
        if let Some(tokens) = self.min_context_tokens {
            features.push(format!("min_context_tokens:{}", tokens));
        }
        features
    }
}

impl From<&ModelRequirement> for RequiredModelFeatures {
    fn from(value: &ModelRequirement) -> Self {
        Self {
            streaming: value.streaming,
            tool_call: value.tool_call,
            json_schema: value.json_schema,
            web_search: value.web_search,
            vision: value.vision,
            min_context_tokens: value.min_context_tokens,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ModelDisable {
    #[serde(default)]
    pub streaming: bool,
    #[serde(default)]
    pub tool_call: bool,
    #[serde(default)]
    pub json_schema: bool,
    #[serde(default)]
    pub web_search: bool,
    #[serde(default)]
    pub vision: bool,
    #[serde(default)]
    pub min_context_tokens: Option<u64>,
}

impl ModelDisable {
    pub fn feature_names(&self) -> Vec<String> {
        let mut features = Vec::new();
        if self.streaming {
            features.push("streaming".to_string());
        }
        if self.tool_call {
            features.push("tool_calling".to_string());
        }
        if self.json_schema {
            features.push("json_output".to_string());
        }
        if self.web_search {
            features.push("web_search".to_string());
        }
        if self.vision {
            features.push("vision".to_string());
        }
        if let Some(tokens) = self.min_context_tokens {
            features.push(format!("min_context_tokens:{}", tokens));
        }
        features
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelAttributes {
    #[serde(default)]
    pub provider_type: ProviderType,
    #[serde(default)]
    pub local: bool,
    #[serde(default)]
    pub privacy: PrivacyClass,
    #[serde(default)]
    pub quality_score: Option<f64>,
    #[serde(default)]
    pub latency_class: LatencyClass,
    #[serde(default)]
    pub cost_class: CostClass,
}

impl Default for ModelAttributes {
    fn default() -> Self {
        Self {
            provider_type: ProviderType::ProxyUnknown,
            local: false,
            privacy: PrivacyClass::Unknown,
            quality_score: None,
            latency_class: LatencyClass::Unknown,
            cost_class: CostClass::Unknown,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelPricing {
    pub currency: String,
    #[serde(default)]
    pub input_token: Option<f64>,
    #[serde(default)]
    pub output_token: Option<f64>,
    #[serde(default)]
    pub cache_input_token: Option<f64>,
    #[serde(default)]
    pub estimated_cost: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelHealth {
    #[serde(default)]
    pub status: HealthStatus,
    #[serde(default)]
    pub p50_latency_ms: Option<u64>,
    #[serde(default)]
    pub p95_latency_ms: Option<u64>,
    #[serde(default)]
    pub error_rate_5m: Option<f64>,
    #[serde(default)]
    pub recent_failures: Option<u64>,
    #[serde(default)]
    pub queue_depth: Option<u64>,
    #[serde(default)]
    pub quota_state: QuotaState,
}

impl Default for ModelHealth {
    fn default() -> Self {
        Self {
            status: HealthStatus::Available,
            p50_latency_ms: None,
            p95_latency_ms: None,
            error_rate_5m: None,
            recent_failures: None,
            queue_depth: None,
            quota_state: QuotaState::Unknown,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelMetadata {
    pub provider_model_id: String,
    pub exact_model: String,
    #[serde(default)]
    pub model_driver: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_actual_model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<serde_json::Value>,
    #[serde(default)]
    pub parameter_scale: Option<String>,
    #[serde(default)]
    pub api_types: Vec<ApiType>,
    #[serde(default)]
    pub logical_mounts: Vec<String>,
    #[serde(default)]
    pub capabilities: ModelCapabilities,
    #[serde(default)]
    pub attributes: ModelAttributes,
    #[serde(default)]
    pub pricing: ModelPricing,
    #[serde(default)]
    pub health: ModelHealth,
}

impl ModelMetadata {
    pub fn exact_name(&self) -> Result<ExactModelName, RouteError> {
        ExactModelName::parse(self.exact_model.as_str())
    }

    pub fn supports_api_type(&self, api_type: &ApiType) -> bool {
        self.api_types.iter().any(|item| item == api_type)
    }

    pub fn supports_requirements(&self, required: &RequiredModelFeatures) -> bool {
        self.capabilities.supports(required)
    }

    pub fn is_available(&self) -> bool {
        self.health.status != HealthStatus::Unavailable
            && self.health.quota_state != QuotaState::Exhausted
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderInventory {
    pub provider_instance_name: String,
    #[serde(default)]
    pub provider_type: ProviderType,
    #[serde(default)]
    pub provider_driver: String,
    #[serde(default)]
    pub provider_origin: ProviderOrigin,
    #[serde(default)]
    pub provider_type_trusted_source: ProviderTypeTrustedSource,
    #[serde(default)]
    pub provider_type_revision: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub inventory_revision: Option<String>,
    #[serde(default, skip_serializing)]
    pub driver_metadata_generation: u64,
    #[serde(default)]
    pub models: Vec<ModelMetadata>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelItem {
    pub target: String,
    #[serde(default = "default_item_weight")]
    pub weight: f64,
}

impl ModelItem {
    pub fn new(target: impl Into<String>, weight: f64) -> Self {
        Self {
            target: target.into(),
            weight,
        }
    }
}

pub fn default_item_weight() -> f64 {
    1.0
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ModelItemPatch {
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub weight: Option<f64>,
}

impl ModelItemPatch {
    pub fn apply_to(&self, base: &ModelItem) -> ModelItem {
        ModelItem {
            target: self.target.clone().unwrap_or_else(|| base.target.clone()),
            weight: self.weight.unwrap_or(base.weight),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayMergeMode {
    Inherit,
    Replace,
}

impl Default for OverlayMergeMode {
    fn default() -> Self {
        Self::Inherit
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackMode {
    Strict,
    Parent,
    TargetExact,
    TargetLogical,
    Disabled,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FallbackRule {
    pub mode: FallbackMode,
    #[serde(default)]
    pub target: Option<String>,
}

impl FallbackRule {
    pub fn strict() -> Self {
        Self {
            mode: FallbackMode::Strict,
            target: None,
        }
    }

    pub fn parent() -> Self {
        Self {
            mode: FallbackMode::Parent,
            target: None,
        }
    }

    pub fn target_logical(target: impl Into<String>) -> Self {
        Self {
            mode: FallbackMode::TargetLogical,
            target: Some(target.into()),
        }
    }

    pub fn target_exact(target: impl Into<String>) -> Self {
        Self {
            mode: FallbackMode::TargetExact,
            target: Some(target.into()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchedulerProfile {
    CostFirst,
    LatencyFirst,
    QualityFirst,
    Balanced,
    LocalFirst,
    StrictLocal,
}

impl Default for SchedulerProfile {
    fn default() -> Self {
        Self::CostFirst
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct LockedValue<T> {
    pub value: T,
    #[serde(default)]
    pub locked: bool,
}

impl<'de, T> Deserialize<'de> for LockedValue<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum LockedValueSerde<T> {
            Raw(T),
            Object {
                value: T,
                #[serde(default)]
                locked: bool,
            },
        }

        match LockedValueSerde::deserialize(deserializer)? {
            LockedValueSerde::Raw(value) => Ok(Self {
                value,
                locked: false,
            }),
            LockedValueSerde::Object { value, locked } => Ok(Self { value, locked }),
        }
    }
}

impl<T> LockedValue<T> {
    pub fn new(value: T) -> Self {
        Self {
            value,
            locked: false,
        }
    }

    pub fn locked(value: T) -> Self {
        Self {
            value,
            locked: true,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PolicyConfig {
    #[serde(default)]
    pub profile: Option<LockedValue<SchedulerProfile>>,
    #[serde(default)]
    pub scheduler_profiles: Option<LockedValue<SchedulerProfileConfig>>,
    #[serde(default)]
    pub local_only: Option<LockedValue<bool>>,
    #[serde(default)]
    pub allow_fallback: Option<LockedValue<bool>>,
    #[serde(default)]
    pub allow_exact_model_fallback: Option<LockedValue<bool>>,
    #[serde(default)]
    pub runtime_failover: Option<LockedValue<bool>>,
    #[serde(default)]
    pub explain: Option<LockedValue<bool>>,
    #[serde(default)]
    pub blocked_provider_instances: Option<LockedValue<Vec<String>>>,
    #[serde(default)]
    pub allowed_provider_instances: Option<LockedValue<Vec<String>>>,
    #[serde(default)]
    pub max_estimated_cost_usd: Option<LockedValue<f64>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SchedulerProfileConfig {
    #[serde(default)]
    pub cost_first: Option<SchedulerProfileWeights>,
    #[serde(default)]
    pub latency_first: Option<SchedulerProfileWeights>,
    #[serde(default)]
    pub quality_first: Option<SchedulerProfileWeights>,
    #[serde(default)]
    pub balanced: Option<SchedulerProfileWeights>,
    #[serde(default)]
    pub local_first: Option<SchedulerProfileWeights>,
    #[serde(default)]
    pub strict_local: Option<SchedulerProfileWeights>,
}

impl SchedulerProfileConfig {
    pub fn weights_for(&self, profile: &SchedulerProfile) -> Option<&SchedulerProfileWeights> {
        match profile {
            SchedulerProfile::CostFirst => self.cost_first.as_ref(),
            SchedulerProfile::LatencyFirst => self.latency_first.as_ref(),
            SchedulerProfile::QualityFirst => self.quality_first.as_ref(),
            SchedulerProfile::Balanced => self.balanced.as_ref(),
            SchedulerProfile::LocalFirst => self.local_first.as_ref(),
            SchedulerProfile::StrictLocal => self.strict_local.as_ref(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SchedulerProfileWeights {
    #[serde(default)]
    pub cost: f64,
    #[serde(default)]
    pub latency: f64,
    #[serde(default)]
    pub reliability: f64,
    #[serde(default)]
    pub quality: f64,
    #[serde(default)]
    pub preference: f64,
    #[serde(default)]
    pub cache: f64,
    #[serde(default)]
    pub local: f64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MountMode {
    Manual,
    Auto,
    Hybrid,
}

impl Default for MountMode {
    fn default() -> Self {
        Self::Hybrid
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogicalModelDefinition {
    pub path: String,
    pub api_type: ApiType,
    #[serde(default)]
    pub min_line: ModelRequirement,
    #[serde(default)]
    pub disable_line: ModelDisable,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_options: Option<serde_json::Value>,
    #[serde(default)]
    pub mount_mode: MountMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduler_profile: Option<SchedulerProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<FallbackRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_policy: Option<PolicyConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_visible_tier: Option<String>,
}

impl SchedulerProfileWeights {
    pub fn validate(&self) -> Result<(), RouteError> {
        for value in [
            self.cost,
            self.latency,
            self.reliability,
            self.quality,
            self.preference,
            self.cache,
            self.local,
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(RouteError::new(
                    RouteErrorCode::SessionConfigInvalid,
                    "scheduler profile weights must be non-negative finite numbers",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoutePolicy {
    #[serde(default)]
    pub profile: SchedulerProfile,
    #[serde(default)]
    pub scheduler_profiles: Option<SchedulerProfileConfig>,
    #[serde(default)]
    pub local_only: bool,
    #[serde(default = "default_true")]
    pub allow_fallback: bool,
    #[serde(default)]
    pub allow_exact_model_fallback: bool,
    #[serde(default = "default_true")]
    pub runtime_failover: bool,
    #[serde(default)]
    pub explain: bool,
    #[serde(default)]
    pub required_features: RequiredModelFeatures,
    #[serde(default)]
    pub blocked_provider_instances: Vec<String>,
    #[serde(default)]
    pub allowed_provider_instances: Vec<String>,
    #[serde(default)]
    pub max_estimated_cost_usd: Option<f64>,
    #[serde(default)]
    pub max_latency_ms: Option<u64>,
    #[serde(default)]
    pub fallback: Option<FallbackRule>,
}

impl Default for RoutePolicy {
    fn default() -> Self {
        Self {
            profile: SchedulerProfile::CostFirst,
            local_only: false,
            allow_fallback: true,
            allow_exact_model_fallback: false,
            runtime_failover: true,
            explain: false,
            required_features: RequiredModelFeatures::default(),
            blocked_provider_instances: Vec::new(),
            allowed_provider_instances: Vec::new(),
            max_estimated_cost_usd: None,
            max_latency_ms: None,
            fallback: None,
            scheduler_profiles: None,
        }
    }
}

impl RoutePolicy {
    #[allow(dead_code)]
    pub fn from_config(config: &PolicyConfig) -> Self {
        let mut policy = RoutePolicy::default();
        if let Some(value) = config.profile.as_ref() {
            policy.profile = value.value.clone();
        }
        if let Some(value) = config.scheduler_profiles.as_ref() {
            policy.scheduler_profiles = Some(value.value.clone());
        }
        if let Some(value) = config.local_only.as_ref() {
            policy.local_only = value.value;
        }
        if let Some(value) = config.allow_fallback.as_ref() {
            policy.allow_fallback = value.value;
        }
        if let Some(value) = config.allow_exact_model_fallback.as_ref() {
            policy.allow_exact_model_fallback = value.value;
        }
        if let Some(value) = config.runtime_failover.as_ref() {
            policy.runtime_failover = value.value;
        }
        if let Some(value) = config.explain.as_ref() {
            policy.explain = value.value;
        }
        if let Some(value) = config.blocked_provider_instances.as_ref() {
            policy.blocked_provider_instances = value.value.clone();
        }
        if let Some(value) = config.allowed_provider_instances.as_ref() {
            policy.allowed_provider_instances = value.value.clone();
        }
        if let Some(value) = config.max_estimated_cost_usd.as_ref() {
            policy.max_estimated_cost_usd = Some(value.value);
        }
        policy
    }
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelCandidate {
    pub exact_model: String,
    pub provider_model_id: String,
    pub provider_instance_name: String,
    pub api_type: ApiType,
    pub metadata: ModelMetadata,
    #[serde(default)]
    pub resolved_logical_path: Option<String>,
    #[serde(default)]
    pub priority_path: Vec<f64>,
    #[serde(default = "default_item_weight")]
    pub exact_model_weight: f64,
    #[serde(default = "default_item_weight")]
    pub provider_weight: f64,
    #[serde(default)]
    pub route_paths: Vec<String>,
    #[serde(default)]
    pub dynamic_cost_estimate: Option<CostEstimateOutput>,
}

impl ModelCandidate {
    pub fn from_metadata(metadata: ModelMetadata, api_type: ApiType) -> Result<Self, RouteError> {
        let exact = metadata.exact_name()?;
        Ok(Self {
            exact_model: metadata.exact_model.clone(),
            provider_model_id: metadata.provider_model_id.clone(),
            provider_instance_name: exact.provider_instance_name,
            api_type,
            metadata,
            resolved_logical_path: None,
            priority_path: Vec::new(),
            exact_model_weight: 1.0,
            provider_weight: 1.0,
            route_paths: Vec::new(),
            dynamic_cost_estimate: None,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FilteredCandidateTrace {
    pub exact_model: String,
    pub provider_instance_name: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RankedCandidateTrace {
    pub exact_model: String,
    pub provider_instance_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing_snapshot: Option<RoutePricingSnapshot>,
    #[serde(default)]
    pub priority_path: Vec<f64>,
    pub exact_model_weight: f64,
    #[serde(default = "default_item_weight")]
    pub provider_weight: f64,
    #[serde(default)]
    pub preference_score_inputs: PreferenceScoreInputs,
    #[serde(default)]
    pub score_inputs: ScoreInputs,
    #[serde(default)]
    pub final_score: Option<f64>,
    pub selected: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScoreInputs {
    pub cost: f64,
    pub latency: f64,
    pub reliability: f64,
    pub quality: f64,
    pub preference: f64,
    pub cache: f64,
    pub local: f64,
}

impl Default for ScoreInputs {
    fn default() -> Self {
        Self {
            cost: 0.0,
            latency: 0.0,
            reliability: 0.0,
            quality: 0.0,
            preference: 0.0,
            cache: 0.0,
            local: 0.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PreferenceScoreInputs {
    pub exact_model_weight: f64,
    pub provider_weight: f64,
    pub combined_weight: f64,
    pub preference_penalty: f64,
    pub exact_model_weight_effect: String,
    pub provider_weight_effect: String,
}

impl Default for PreferenceScoreInputs {
    fn default() -> Self {
        Self::from_weights(1.0, 1.0)
    }
}

impl PreferenceScoreInputs {
    pub fn from_weights(exact_model_weight: f64, provider_weight: f64) -> Self {
        let combined_weight = (exact_model_weight * provider_weight).max(0.0);
        let preference_penalty = 1.0 / combined_weight.max(0.000_001);
        Self {
            exact_model_weight,
            provider_weight,
            combined_weight,
            preference_penalty,
            exact_model_weight_effect: weight_effect(exact_model_weight).to_string(),
            provider_weight_effect: weight_effect(provider_weight).to_string(),
        }
    }
}

fn weight_effect(weight: f64) -> &'static str {
    if weight <= 0.0 {
        "disabled"
    } else if weight < 1.0 {
        "downweighted"
    } else if weight > 1.0 {
        "upweighted"
    } else {
        "neutral"
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FallbackTraceItem {
    pub from: String,
    pub to: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogicalItemSourceTrace {
    pub logical_path: String,
    pub item_name: String,
    pub target: String,
    pub source: String,
    pub weight: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogicalAdmissionTrace {
    pub logical_path: String,
    pub exact_model: String,
    pub source: String,
    pub accepted: bool,
    #[serde(default)]
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DisabledCapabilityTrace {
    pub logical_path: String,
    pub capability: String,
    pub source: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionOverlayTrace {
    pub logical_profile_scope: String,
    pub overlay_path: String,
    pub merge_mode: OverlayMergeMode,
    #[serde(default)]
    pub selected_from_overlay: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoutePricingSnapshot {
    pub currency: String,
    #[serde(default)]
    pub input_token: Option<f64>,
    #[serde(default)]
    pub output_token: Option<f64>,
    #[serde(default)]
    pub cache_input_token: Option<f64>,
    #[serde(default)]
    pub estimated_cost: Option<f64>,
}

impl Default for ModelPricing {
    fn default() -> Self {
        Self {
            currency: "USD".to_string(),
            input_token: None,
            output_token: None,
            cache_input_token: None,
            estimated_cost: None,
        }
    }
}

impl RoutePricingSnapshot {
    pub fn from_candidate(candidate: &ModelCandidate) -> Option<Self> {
        Self::from_values(
            candidate.metadata.pricing.currency.clone(),
            candidate.metadata.pricing.input_token,
            candidate.metadata.pricing.output_token,
            candidate.metadata.pricing.cache_input_token,
            candidate
                .dynamic_cost_estimate
                .as_ref()
                .map(|estimate| estimate.estimated_cost_usd)
                .or(candidate.metadata.pricing.estimated_cost),
        )
    }

    fn from_values(
        currency: String,
        input_token: Option<f64>,
        output_token: Option<f64>,
        cache_input_token: Option<f64>,
        estimated_cost: Option<f64>,
    ) -> Option<Self> {
        if input_token.is_none()
            && output_token.is_none()
            && cache_input_token.is_none()
            && estimated_cost.is_none()
        {
            return None;
        }
        Some(Self {
            currency,
            input_token,
            output_token,
            cache_input_token,
            estimated_cost,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RouteTrace {
    pub request_id: String,
    pub api_type: ApiType,
    pub requested_model: String,
    pub requested_model_type: RequestedModelType,
    #[serde(default)]
    pub resolved_logical_path: Option<String>,
    #[serde(default)]
    pub selected_exact_model: Option<String>,
    #[serde(default)]
    pub selected_provider_instance_name: Option<String>,
    #[serde(default)]
    pub selected_provider_model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing_snapshot: Option<RoutePricingSnapshot>,
    #[serde(default)]
    pub candidate_count_before_filter: usize,
    #[serde(default)]
    pub candidate_count_after_filter: usize,
    #[serde(default)]
    pub filtered_candidates: Vec<FilteredCandidateTrace>,
    #[serde(default)]
    pub ranked_candidates: Vec<RankedCandidateTrace>,
    #[serde(default)]
    pub fallback_applied: bool,
    #[serde(default)]
    pub fallback_chain: Vec<FallbackTraceItem>,
    #[serde(default)]
    pub logical_item_sources: Vec<LogicalItemSourceTrace>,
    #[serde(default)]
    pub logical_admission: Vec<LogicalAdmissionTrace>,
    #[serde(default)]
    pub disabled_capability_sources: Vec<DisabledCapabilityTrace>,
    #[serde(default)]
    pub session_overlays: Vec<SessionOverlayTrace>,
    pub scheduler_profile: SchedulerProfile,
    #[serde(default)]
    pub runtime_failover_count: u64,
    #[serde(default)]
    pub user_summary: Option<UserFacingRouteSummary>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserFacingRouteSummary {
    pub display_name: String,
    pub model_family: String,
    pub provider_origin: UserFacingProviderOrigin,
    pub reason_short: String,
    pub was_fallback: bool,
    pub was_failover: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserFacingProviderOrigin {
    Cloud,
    Local,
    ProxyUnknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestedModelType {
    Exact,
    Logical,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RouteErrorCode {
    #[serde(rename = "AICC_ROUTE_INVALID_MODEL_NAME")]
    InvalidModelName,
    #[serde(rename = "AICC_ROUTE_MODEL_NOT_FOUND")]
    ModelNotFound,
    #[serde(rename = "AICC_ROUTE_NO_CANDIDATE")]
    NoCandidate,
    #[serde(rename = "AICC_ROUTE_POLICY_REJECTED")]
    PolicyRejected,
    #[serde(rename = "AICC_ROUTE_FALLBACK_LOOP")]
    FallbackLoop,
    #[serde(rename = "AICC_ROUTE_LOGICAL_TREE_LOOP")]
    LogicalTreeLoop,
    #[serde(rename = "AICC_ROUTE_SESSION_CONFIG_INVALID")]
    SessionConfigInvalid,
    #[serde(rename = "AICC_ROUTE_SESSION_CONFIG_CONFLICT")]
    SessionConfigConflict,
    #[serde(rename = "AICC_ROUTE_SESSION_CONFIG_EXPIRED")]
    SessionConfigExpired,
    #[serde(rename = "AICC_ROUTE_POLICY_LOCKED")]
    PolicyLocked,
    #[serde(rename = "AICC_ROUTE_EXACT_MODEL_UNAVAILABLE")]
    ExactModelUnavailable,
    #[serde(rename = "AICC_ROUTE_PROVIDER_UNAVAILABLE")]
    ProviderUnavailable,
    #[serde(rename = "AICC_ROUTE_BUDGET_EXCEEDED")]
    BudgetExceeded,
    #[serde(rename = "AICC_ROUTE_CONTEXT_TOO_LONG")]
    ContextTooLong,
    #[serde(rename = "AICC_ROUTE_FEATURE_UNSUPPORTED")]
    FeatureUnsupported,
}

impl RouteErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::InvalidModelName => "AICC_ROUTE_INVALID_MODEL_NAME",
            Self::ModelNotFound => "AICC_ROUTE_MODEL_NOT_FOUND",
            Self::NoCandidate => "AICC_ROUTE_NO_CANDIDATE",
            Self::PolicyRejected => "AICC_ROUTE_POLICY_REJECTED",
            Self::FallbackLoop => "AICC_ROUTE_FALLBACK_LOOP",
            Self::LogicalTreeLoop => "AICC_ROUTE_LOGICAL_TREE_LOOP",
            Self::SessionConfigInvalid => "AICC_ROUTE_SESSION_CONFIG_INVALID",
            Self::SessionConfigConflict => "AICC_ROUTE_SESSION_CONFIG_CONFLICT",
            Self::SessionConfigExpired => "AICC_ROUTE_SESSION_CONFIG_EXPIRED",
            Self::PolicyLocked => "AICC_ROUTE_POLICY_LOCKED",
            Self::ExactModelUnavailable => "AICC_ROUTE_EXACT_MODEL_UNAVAILABLE",
            Self::ProviderUnavailable => "AICC_ROUTE_PROVIDER_UNAVAILABLE",
            Self::BudgetExceeded => "AICC_ROUTE_BUDGET_EXCEEDED",
            Self::ContextTooLong => "AICC_ROUTE_CONTEXT_TOO_LONG",
            Self::FeatureUnsupported => "AICC_ROUTE_FEATURE_UNSUPPORTED",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RouteError {
    pub code: RouteErrorCode,
    pub message: String,
}

impl RouteError {
    pub fn new(code: RouteErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for RouteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for RouteError {}

pub type LogicalItems = BTreeMap<String, ModelItem>;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn exact_model_parse_uses_last_at() {
        let parsed = ExactModelName::parse("vendor@model@gpt@openai_primary").unwrap();
        assert_eq!(parsed.provider_model_id, "vendor@model@gpt");
        assert_eq!(parsed.provider_instance_name, "openai_primary");
        assert_eq!(parsed.as_string(), "vendor@model@gpt@openai_primary");
    }

    #[test]
    fn exact_model_rejects_invalid_provider_instance() {
        let err = ExactModelName::parse("gpt-5.2@openai/primary").unwrap_err();
        assert_eq!(err.code, RouteErrorCode::InvalidModelName);
    }

    #[test]
    fn api_type_and_capability_match() {
        let model = ModelMetadata {
            provider_model_id: "gpt-5.2".to_string(),
            exact_model: "gpt-5.2@openai_primary".to_string(),
            model_driver: "openai".to_string(),
            origin_model_id: None,
            provider_actual_model_id: None,
            provider_options: None,
            parameter_scale: None,
            api_types: vec![ApiType::Llm],
            logical_mounts: vec!["llm.gpt5".to_string()],
            capabilities: ModelCapabilities {
                streaming: true,
                tool_call: true,
                json_schema: true,
                web_search: true,
                unsupported_feature_combinations: vec![],
                vision: false,
                max_context_tokens: Some(128_000),
                max_output_tokens: Some(16_384),
            },
            attributes: ModelAttributes::default(),
            pricing: ModelPricing::default(),
            health: ModelHealth::default(),
        };

        assert!(model.supports_api_type(&ApiType::Llm));
        assert!(!model.supports_api_type(&ApiType::ImageTextToImage));
        assert!(model.supports_requirements(&RequiredModelFeatures {
            web_search: true,
            tool_call: true,
            min_context_tokens: Some(32_000),
            ..Default::default()
        }));
        assert!(!model.supports_requirements(&RequiredModelFeatures {
            vision: true,
            ..Default::default()
        }));
        assert!(!ModelMetadata {
            capabilities: ModelCapabilities {
                web_search: false,
                ..model.capabilities.clone()
            },
            ..model
        }
        .supports_requirements(&RequiredModelFeatures {
            web_search: true,
            ..Default::default()
        }));
    }

    #[test]
    fn unsupported_feature_combination_rejects_matching_request() {
        let required = RequiredModelFeatures {
            web_search: true,
            tool_call: true,
            ..Default::default()
        };
        let capabilities = ModelCapabilities {
            web_search: true,
            tool_call: true,
            unsupported_feature_combinations: vec![vec![
                "web_search".to_string(),
                "tool_calling".to_string(),
            ]],
            ..Default::default()
        };

        assert!(!capabilities.supports(&required));
        assert_eq!(
            capabilities.explain_missing_requirements(&ModelRequirement {
                web_search: true,
                tool_call: true,
                ..Default::default()
            }),
            vec!["unsupported_feature_combination:tool_calling+web_search"]
        );
    }

    #[test]
    fn unsupported_feature_combination_matches_only_the_complete_set() {
        let mut capabilities = ModelCapabilities {
            tool_call: true,
            web_search: true,
            vision: true,
            ..Default::default()
        };
        capabilities
            .set_feature_combination_supported(&["vision", "web_search", "tool_calling"], false);

        assert!(capabilities.supports(&RequiredModelFeatures {
            web_search: true,
            tool_call: true,
            ..Default::default()
        }));
        assert!(!capabilities.supports(&RequiredModelFeatures {
            web_search: true,
            tool_call: true,
            vision: true,
            ..Default::default()
        }));

        capabilities
            .set_feature_combination_supported(&["tool_calling", "vision", "web_search"], true);
        assert!(capabilities.supports(&RequiredModelFeatures {
            web_search: true,
            tool_call: true,
            vision: true,
            ..Default::default()
        }));
    }

    #[test]
    fn provider_inventory_serde_fixture() {
        let value = json!({
            "provider_instance_name": "openai_primary",
            "provider_type": "cloud_api",
            "inventory_revision": "rev-1",
            "models": [{
                "provider_model_id": "gpt-5.2",
                "exact_model": "gpt-5.2@openai_primary",
                "api_types": ["llm"],
                "logical_mounts": ["llm.gpt5"],
                "capabilities": {
                    "streaming": true,
                    "tool_call": true,
                    "json_schema": true,
                    "max_context_tokens": 128000,
                    "max_output_tokens": 16384
                },
                "attributes": {
                    "provider_type": "cloud_api",
                    "local": false,
                    "privacy": "cloud",
                    "quality_score": 0.95,
                    "latency_class": "normal",
                    "cost_class": "high"
                },
                "health": {
                    "status": "available",
                    "quota_state": "normal"
                }
            }]
        });

        let inventory: ProviderInventory = serde_json::from_value(value).unwrap();
        assert_eq!(inventory.provider_instance_name, "openai_primary");
        assert_eq!(inventory.models[0].api_types, vec![ApiType::Llm]);
        assert!(inventory.models[0].supports_api_type(&ApiType::Llm));
        let encoded = serde_json::to_value(&inventory).unwrap();
        assert_eq!(encoded["models"][0]["api_types"][0], "llm");
    }
}
