use crate::{AppDoc, AppType, SelectorType};
use ::kRPC::*;
use async_trait::async_trait;
use name_lib::DID;
use ndn_lib::ObjId;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::net::IpAddr;

pub const AICC_SERVICE_UNIQUE_ID: &str = "aicc";
pub const AICC_SERVICE_SERVICE_NAME: &str = "aicc";
pub const AICC_SERVICE_SERVICE_PORT: u16 = 4040;

pub mod ai_methods {
    pub const ROUTE_RESOLVE: &str = "route.resolve";
    pub const CHAT_COMPLETIONS_CREATE: &str = "chat.completions.create";
    pub const IMAGES_GENERATE: &str = "images.generate";
    pub const HELPER_LLM_CHAT: &str = "helper.llm_chat";
    pub const HELPER_TEXT_TO_IMAGE: &str = "helper.text_to_image";

    pub const LLM_CHAT: &str = "llm.chat";
    pub const EMBEDDING_TEXT: &str = "embedding.text";
    pub const EMBEDDING_MULTIMODAL: &str = "embedding.multimodal";
    pub const RERANK: &str = "rerank";
    pub const IMAGE_TXT2IMG: &str = "image.txt2img";
    pub const IMAGE_IMG2IMG: &str = "image.img2img";
    pub const IMAGE_INPAINT: &str = "image.inpaint";
    pub const IMAGE_UPSCALE: &str = "image.upscale";
    pub const IMAGE_BG_REMOVE: &str = "image.bg_remove";
    pub const VISION_OCR: &str = "vision.ocr";
    pub const VISION_CAPTION: &str = "vision.caption";
    pub const VISION_DETECT: &str = "vision.detect";
    pub const VISION_SEGMENT: &str = "vision.segment";
    pub const AUDIO_TTS: &str = "audio.tts";
    pub const AUDIO_ASR: &str = "audio.asr";
    pub const AUDIO_MUSIC: &str = "audio.music";
    pub const AUDIO_ENHANCE: &str = "audio.enhance";
    pub const VIDEO_TXT2VIDEO: &str = "video.txt2video";
    pub const VIDEO_IMG2VIDEO: &str = "video.img2video";
    pub const VIDEO_VIDEO2VIDEO: &str = "video.video2video";
    pub const VIDEO_EXTEND: &str = "video.extend";
    pub const VIDEO_UPSCALE: &str = "video.upscale";
    pub const AGENT_COMPUTER_USE: &str = "agent.computer_use";

    pub const CANCEL: &str = "cancel";
    pub const RELOAD_SETTINGS: &str = "reload_settings";
    pub const SERVICE_RELOAD_SETTINGS: &str = "service.reload_settings";
    pub const QUOTA_QUERY: &str = "quota.query";
    pub const PROVIDER_LIST: &str = "provider.list";
    pub const PROVIDER_HEALTH: &str = "provider.health";
    pub const DRIVER_METADATA_UPDATE_GET: &str = "driver_metadata_update.get";
    pub const DRIVER_METADATA_UPDATE_SET: &str = "driver_metadata_update.set";

    pub fn is_ai_method(method: &str) -> bool {
        matches!(
            method,
            LLM_CHAT
                | EMBEDDING_TEXT
                | EMBEDDING_MULTIMODAL
                | RERANK
                | IMAGE_TXT2IMG
                | IMAGE_IMG2IMG
                | IMAGE_INPAINT
                | IMAGE_UPSCALE
                | IMAGE_BG_REMOVE
                | VISION_OCR
                | VISION_CAPTION
                | VISION_DETECT
                | VISION_SEGMENT
                | AUDIO_TTS
                | AUDIO_ASR
                | AUDIO_MUSIC
                | AUDIO_ENHANCE
                | VIDEO_TXT2VIDEO
                | VIDEO_IMG2VIDEO
                | VIDEO_VIDEO2VIDEO
                | VIDEO_EXTEND
                | VIDEO_UPSCALE
                | AGENT_COMPUTER_USE
        )
    }

    pub fn is_aicc_core_method(method: &str) -> bool {
        matches!(
            method,
            ROUTE_RESOLVE
                | CHAT_COMPLETIONS_CREATE
                | IMAGES_GENERATE
                | HELPER_LLM_CHAT
                | HELPER_TEXT_TO_IMAGE
        ) || is_ai_method(method)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Capability {
    Llm,
    Embedding,
    Rerank,
    Image,
    Vision,
    Audio,
    Video,
    Agent,
}

pub type Feature = String;

pub mod features {
    pub const PLAN: &str = "plan";
    pub const TOOL_CALLING: &str = "tool_calling";
    pub const JSON_OUTPUT: &str = "json_output";
    pub const WEB_SEARCH: &str = "web_search";
    pub const VISION: &str = "vision";
    pub const ASR: &str = "asr";
    pub const VIDEO_UNDERSTAND: &str = "video_understand";
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum RespFormat {
    #[default]
    #[serde(alias = "Text")]
    Text,
    #[serde(alias = "Json", alias = "JSON")]
    Json,
}

fn is_default_resp_format(resp_format: &RespFormat) -> bool {
    matches!(resp_format, RespFormat::Text)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResourceRef {
    Url {
        url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        mime_hint: Option<String>,
    },
    Base64 {
        mime: String,
        data_base64: String,
    },
    NamedObject {
        obj_id: ObjId,
    },
}

impl ResourceRef {
    pub fn url(url: String, mime_hint: Option<String>) -> Self {
        Self::Url { url, mime_hint }
    }

    pub fn base64(mime: String, data_base64: String) -> Self {
        Self::Base64 { mime, data_base64 }
    }

    pub fn named_object(obj_id: ObjId) -> Self {
        Self::NamedObject { obj_id }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelSpec {
    pub alias: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_model_hint: Option<String>,
}

impl ModelSpec {
    pub fn new(alias: String, provider_model_hint: Option<String>) -> Self {
        Self {
            alias,
            provider_model_hint,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ModelRequirement {
    #[serde(default, skip_serializing_if = "is_false")]
    pub streaming: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub tool_call: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub json_schema: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub web_search: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub vision: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_context_tokens: Option<u64>,
}

impl ModelRequirement {
    pub fn set_feature_required(&mut self, feature: &str) {
        match feature {
            features::TOOL_CALLING => self.tool_call = true,
            features::JSON_OUTPUT => self.json_schema = true,
            features::WEB_SEARCH => self.web_search = true,
            features::VISION => self.vision = true,
            "streaming" => self.streaming = true,
            _ => {}
        }
    }

    pub fn requires_feature(&self, feature: &str) -> bool {
        match feature {
            features::TOOL_CALLING => self.tool_call,
            features::JSON_OUTPUT => self.json_schema,
            features::WEB_SEARCH => self.web_search,
            features::VISION => self.vision,
            "streaming" => self.streaming,
            _ => false,
        }
    }

    pub fn feature_names(&self) -> Vec<Feature> {
        let mut features = Vec::new();
        if self.streaming {
            features.push("streaming".to_string());
        }
        if self.tool_call {
            features.push(features::TOOL_CALLING.to_string());
        }
        if self.json_schema {
            features.push(features::JSON_OUTPUT.to_string());
        }
        if self.web_search {
            features.push(features::WEB_SEARCH.to_string());
        }
        if self.vision {
            features.push(features::VISION.to_string());
        }
        features
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ModelDisable {
    #[serde(default, skip_serializing_if = "is_false")]
    pub streaming: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub tool_call: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub json_schema: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub web_search: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub vision: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_context_tokens: Option<u64>,
}

impl ModelDisable {
    pub fn set_feature_disabled(&mut self, feature: &str) {
        match feature {
            features::TOOL_CALLING => self.tool_call = true,
            features::JSON_OUTPUT => self.json_schema = true,
            features::WEB_SEARCH => self.web_search = true,
            features::VISION => self.vision = true,
            "streaming" => self.streaming = true,
            _ => {}
        }
    }

    pub fn disables_feature(&self, feature: &str) -> bool {
        match feature {
            features::TOOL_CALLING => self.tool_call,
            features::JSON_OUTPUT => self.json_schema,
            features::WEB_SEARCH => self.web_search,
            features::VISION => self.vision,
            "streaming" => self.streaming,
            _ => false,
        }
    }

    pub fn feature_names(&self) -> Vec<Feature> {
        let mut features = Vec::new();
        if self.streaming {
            features.push("streaming".to_string());
        }
        if self.tool_call {
            features.push(features::TOOL_CALLING.to_string());
        }
        if self.json_schema {
            features.push(features::JSON_OUTPUT.to_string());
        }
        if self.web_search {
            features.push(features::WEB_SEARCH.to_string());
        }
        if self.vision {
            features.push(features::VISION.to_string());
        }
        if let Some(tokens) = self.min_context_tokens {
            features.push(format!("min_context_tokens:{}", tokens));
        }
        features
    }
}

fn is_default_model_disable(disable: &ModelDisable) -> bool {
    disable == &ModelDisable::default()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelItem {
    pub target: String,
    #[serde(default = "default_model_item_weight")]
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

fn default_model_item_weight() -> f64 {
    1.0
}

pub type LogicalItems = BTreeMap<String, ModelItem>;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ModelItemPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiccFallbackMode {
    Strict,
    Parent,
    TargetExact,
    TargetLogical,
    Disabled,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct AiccFallbackRule {
    pub mode: AiccFallbackMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiccSchedulerProfile {
    CostFirst,
    LatencyFirst,
    QualityFirst,
    Balanced,
    LocalFirst,
    StrictLocal,
}

impl Default for AiccSchedulerProfile {
    fn default() -> Self {
        Self::CostFirst
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LockedValue<T> {
    pub value: T,
    #[serde(default, skip_serializing_if = "is_false")]
    pub locked: bool,
}

impl<'de, T> Deserialize<'de> for LockedValue<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AiccPolicyConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<LockedValue<AiccSchedulerProfile>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduler_profiles: Option<LockedValue<AiccSchedulerProfileConfig>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_only: Option<LockedValue<bool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_fallback: Option<LockedValue<bool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_exact_model_fallback: Option<LockedValue<bool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_failover: Option<LockedValue<bool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explain: Option<LockedValue<bool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_provider_instances: Option<LockedValue<Vec<String>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_provider_instances: Option<LockedValue<Vec<String>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_estimated_cost_usd: Option<LockedValue<f64>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AiccSchedulerProfileConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_first: Option<AiccSchedulerProfileWeights>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_first: Option<AiccSchedulerProfileWeights>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality_first: Option<AiccSchedulerProfileWeights>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub balanced: Option<AiccSchedulerProfileWeights>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_first: Option<AiccSchedulerProfileWeights>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strict_local: Option<AiccSchedulerProfileWeights>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AiccSchedulerProfileWeights {
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AiccLogicalNodeOverlay {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub children: BTreeMap<String, AiccLogicalNodeOverlay>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<LogicalItems>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_overrides: Option<BTreeMap<String, ModelItemPatch>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub exact_model_weights: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_line: Option<ModelDisable>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<AiccFallbackRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<AiccPolicyConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_policy_override: Option<AiccPolicyConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AiccSessionLogicalProfile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overlays: Vec<AiccLogicalTreeOverlay>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_policy_override: Option<AiccPolicyConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AiccLogicalTreeOverlay {
    pub path: String,
    #[serde(default, skip_serializing_if = "is_default_overlay_merge_mode")]
    pub merge_mode: OverlayMergeMode,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub items: LogicalItems,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub item_overrides: BTreeMap<String, ModelItemPatch>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub exact_model_weights: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_line: Option<ModelDisable>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<AiccFallbackRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_policy_override: Option<AiccPolicyConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

fn is_default_overlay_merge_mode(mode: &OverlayMergeMode) -> bool {
    *mode == OverlayMergeMode::default()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AiccRouteOverlay {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub logical_tree: BTreeMap<String, AiccLogicalNodeOverlay>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_profile: Option<AiccSessionLogicalProfile>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub logical_profiles: BTreeMap<String, AiccSessionLogicalProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_logical_profile: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub global_exact_model_weights: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub provider_weights: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "is_default_aicc_policy_config")]
    pub policy: AiccPolicyConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_seconds: Option<u64>,
}

fn is_default_aicc_policy_config(policy: &AiccPolicyConfig) -> bool {
    policy == &AiccPolicyConfig::default()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Requirements {
    #[serde(default, flatten)]
    pub required: ModelRequirement,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub must_features: Vec<Feature>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_latency_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "is_default_resp_format")]
    pub resp_format: RespFormat,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<Value>,
}

impl Requirements {
    pub fn new(
        must_features: Vec<Feature>,
        max_latency_ms: Option<u64>,
        max_cost_usd: Option<f64>,
        extra: Option<Value>,
    ) -> Self {
        Self {
            required: ModelRequirement::default(),
            must_features,
            max_latency_ms,
            max_cost_usd,
            resp_format: RespFormat::default(),
            extra,
        }
    }

    pub fn set_feature_required(&mut self, feature: &str) {
        self.required.set_feature_required(feature);
    }

    pub fn requires_feature(&self, feature: &str) -> bool {
        self.required.requires_feature(feature)
            || self.must_features.iter().any(|item| item == feature)
    }

    pub fn effective_feature_names(&self) -> Vec<Feature> {
        let mut features = self.required.feature_names();
        for feature in &self.must_features {
            if !features.iter().any(|item| item == feature) {
                features.push(feature.clone());
            }
        }
        features
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum RoutePolicyProfile {
    Cheap,
    Fast,
    #[default]
    Balanced,
    Quality,
}

fn is_default_route_policy_profile(profile: &RoutePolicyProfile) -> bool {
    matches!(profile, RoutePolicyProfile::Balanced)
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoutePolicy {
    #[serde(default, skip_serializing_if = "is_default_route_policy_profile")]
    pub profile: RoutePolicyProfile,
    #[serde(default, skip_serializing_if = "is_false")]
    pub local_only: bool,
    #[serde(default = "default_allow_fallback")]
    pub allow_fallback: bool,
    #[serde(default = "default_runtime_failover")]
    pub runtime_failover: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub explain: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_provider_instances: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_provider_instances: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_latency_ms: Option<u64>,
}

fn default_allow_fallback() -> bool {
    true
}

fn default_runtime_failover() -> bool {
    true
}

impl Default for RoutePolicy {
    fn default() -> Self {
        Self {
            profile: RoutePolicyProfile::Balanced,
            local_only: false,
            allow_fallback: true,
            runtime_failover: true,
            explain: false,
            allowed_provider_instances: Vec::new(),
            blocked_provider_instances: Vec::new(),
            max_cost_usd: None,
            max_latency_ms: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AiToolSpec {
    pub name: String,
    pub description: String,
    pub args_schema: HashMap<String, Value>,
    pub output_schema: Value,
}

pub fn value_to_object_map(value: Value) -> HashMap<String, Value> {
    match value {
        Value::Object(map) => map.into_iter().collect(),
        _ => HashMap::new(),
    }
}

/// IR-level role for a message in `AiMessage`. Provider lowering rewrites
/// `Tool` and `Developer` per §1.4 of the AiMessage 重构 design doc.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AiRole {
    System,
    User,
    Assistant,
    /// IR-internal carrier role for tool results. Each adapter MUST rewrite
    /// into the provider's native form (function_call_output / tool message
    /// / nested user+tool_result block / etc.).
    Tool,
    /// OpenAI Responses native; other providers fold into nearest `System`
    /// or downgrade to `System` role.
    Developer,
}

impl AiRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
            Self::Developer => "developer",
        }
    }
}

/// Strict content subset allowed inside `AiContent::ToolResult.content` —
/// excludes ToolUse / ToolResult / Thinking, which have no meaning nested
/// inside a tool result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AiToolResultContent {
    Text {
        text: String,
    },
    Image {
        source: ResourceRef,
    },
    Document {
        source: ResourceRef,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
}

impl AiToolResultContent {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    pub fn text_str(&self) -> Option<&str> {
        match self {
            Self::Text { text } => Some(text.as_str()),
            _ => None,
        }
    }
}

/// Content block. Mirrors the Anthropic content-block model, generalized
/// enough to round-trip OpenAI Responses items and Gemini parts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AiContent {
    /// Plain text segment.
    Text { text: String },

    /// Image block; reuses `ResourceRef` (URL / base64 / named object).
    Image { source: ResourceRef },

    /// Long-document attachment (PDF / large text), mirrors Claude document API.
    Document {
        source: ResourceRef,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },

    /// Assistant requesting a tool call.
    ToolUse {
        call_id: String,
        name: String,
        #[serde(default)]
        args: HashMap<String, Value>,
    },

    /// Tool result echoed back to the LLM, keyed by `call_id` of the
    /// originating `ToolUse`.
    ToolResult {
        call_id: String,
        content: Vec<AiToolResultContent>,
        #[serde(default, skip_serializing_if = "is_false")]
        is_error: bool,
    },

    /// Extended thinking / reasoning block. `summary` is OpenAI Responses
    /// reasoning summary; `text` is Claude thinking plaintext;
    /// `provider_metadata` holds per-provider signature/state bits that
    /// aren't worth a dedicated field.
    Thinking {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
    },

    /// Provider-specific native item that needs to round-trip but cannot be
    /// abstracted across providers (OpenAI reasoning item id/encrypted_content,
    /// Claude server_tool_use / web_search_tool_result, etc.).
    ///
    /// `provider` is the stable owner/consumer namespace for the opaque item,
    /// not the native item's protocol or type name. Each adapter defines the
    /// provider namespaces it can restore; the rest are dropped.
    ProviderState { provider: String, value: Value },
}

#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum AiMessageError {
    #[error("block type `{block_type}` is not allowed for role `{role:?}`")]
    InvalidBlockForRole {
        role: AiRole,
        block_type: &'static str,
    },
    #[error("tool_use / tool_result missing call_id")]
    MissingCallId,
    #[error("tool_result content must not be empty")]
    EmptyToolResult,
    #[error("role `Tool` requires exactly one ToolResult block")]
    ToolRoleShape,
}

impl AiContent {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    pub fn image(source: ResourceRef) -> Self {
        Self::Image { source }
    }

    pub fn tool_use(
        call_id: impl Into<String>,
        name: impl Into<String>,
        args: HashMap<String, Value>,
    ) -> Self {
        Self::ToolUse {
            call_id: call_id.into(),
            name: name.into(),
            args,
        }
    }

    pub fn tool_result_text(
        call_id: impl Into<String>,
        text: impl Into<String>,
        is_error: bool,
    ) -> Self {
        Self::ToolResult {
            call_id: call_id.into(),
            content: vec![AiToolResultContent::text(text)],
            is_error,
        }
    }

    fn type_tag(&self) -> &'static str {
        match self {
            Self::Text { .. } => "text",
            Self::Image { .. } => "image",
            Self::Document { .. } => "document",
            Self::ToolUse { .. } => "tool_use",
            Self::ToolResult { .. } => "tool_result",
            Self::Thinking { .. } => "thinking",
            Self::ProviderState { .. } => "provider_state",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiMessage {
    pub role: AiRole,
    pub content: Vec<AiContent>,
}

impl AiMessage {
    /// Single text block constructor — covers ~90% of call sites
    /// (system prompts, plain user/assistant messages).
    pub fn text(role: AiRole, text: impl Into<String>) -> Self {
        Self {
            role,
            content: vec![AiContent::text(text)],
        }
    }

    /// Construct from explicit blocks. Caller is responsible for `validate()`.
    pub fn new(role: AiRole, content: Vec<AiContent>) -> Self {
        Self { role, content }
    }

    /// Concatenate all `Text` blocks' `text`, joined by `\n`. Non-text
    /// blocks are skipped. Use this when you need a string-shaped view of
    /// the message (transcript rendering, logging).
    pub fn text_content(&self) -> String {
        let mut out = String::new();
        for block in &self.content {
            if let AiContent::Text { text } = block {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(text);
            }
        }
        out
    }

    /// Extract assistant tool-use blocks as normalized tool calls, preserving
    /// their order within the message.
    pub fn tool_calls(&self) -> Vec<AiToolCall> {
        self.content
            .iter()
            .filter_map(|block| match block {
                AiContent::ToolUse {
                    call_id,
                    name,
                    args,
                } => Some(AiToolCall {
                    name: name.clone(),
                    args: args.clone(),
                    call_id: call_id.clone(),
                }),
                _ => None,
            })
            .collect()
    }

    /// First `Text` block's content, if any. Use this for "I used to read
    /// `&msg.content`" replacement sites.
    pub fn first_text(&self) -> Option<&str> {
        self.content.iter().find_map(|block| match block {
            AiContent::Text { text } => Some(text.as_str()),
            _ => None,
        })
    }

    /// Human-readable debug rendering of every block. Used by transcript
    /// dumps and worklog text. Stable enough for snapshot tests.
    pub fn render_for_debug(&self) -> String {
        let mut out = String::new();
        for (idx, block) in self.content.iter().enumerate() {
            if idx > 0 {
                out.push('\n');
            }
            match block {
                AiContent::Text { text } => out.push_str(text),
                AiContent::Image { source: _ } => out.push_str("[image]"),
                AiContent::Document { title, .. } => {
                    out.push_str("[document");
                    if let Some(t) = title {
                        out.push_str(": ");
                        out.push_str(t);
                    }
                    out.push(']');
                }
                AiContent::ToolUse { call_id, name, .. } => {
                    out.push_str(&format!("[tool_use name={name} call_id={call_id}]"));
                }
                AiContent::ToolResult {
                    call_id,
                    content,
                    is_error,
                } => {
                    out.push_str(&format!(
                        "[tool_result call_id={call_id}{}]",
                        if *is_error { " error" } else { "" }
                    ));
                    for c in content {
                        if let AiToolResultContent::Text { text } = c {
                            out.push('\n');
                            out.push_str(text);
                        }
                    }
                }
                AiContent::Thinking { summary, text, .. } => {
                    out.push_str("[thinking");
                    if let Some(s) = summary {
                        out.push_str(" summary=");
                        out.push_str(s);
                    }
                    if let Some(t) = text {
                        out.push('\n');
                        out.push_str(t);
                    }
                    out.push(']');
                }
                AiContent::ProviderState { provider, .. } => {
                    out.push_str(&format!("[provider_state provider={provider}]"));
                }
            }
        }
        out
    }

    /// Rough byte-length estimate used by `llm_compress` to budget context.
    /// Non-text blocks contribute a conservative constant (~256 bytes for
    /// Image/Document, ToolUse args measured via JSON).
    pub fn estimate_text_len(&self) -> usize {
        let mut total = 0;
        for block in &self.content {
            match block {
                AiContent::Text { text } => total += text.len(),
                AiContent::Image { .. } | AiContent::Document { .. } => total += 256,
                AiContent::ToolUse {
                    name,
                    call_id,
                    args,
                } => {
                    total += name.len() + call_id.len();
                    if let Ok(s) = serde_json::to_string(args) {
                        total += s.len();
                    }
                }
                AiContent::ToolResult {
                    content, call_id, ..
                } => {
                    total += call_id.len();
                    for c in content {
                        match c {
                            AiToolResultContent::Text { text } => total += text.len(),
                            AiToolResultContent::Image { .. }
                            | AiToolResultContent::Document { .. } => total += 256,
                        }
                    }
                }
                AiContent::Thinking { summary, text, .. } => {
                    if let Some(s) = summary {
                        total += s.len();
                    }
                    if let Some(t) = text {
                        total += t.len();
                    }
                }
                AiContent::ProviderState { value, .. } => {
                    if let Ok(s) = serde_json::to_string(value) {
                        total += s.len();
                    }
                }
            }
        }
        total
    }

    /// Validate role × content combinations per §1.1 of the design doc.
    /// `AiPayload::validate_all_messages` calls this for every message before
    /// the request leaves the aicc client.
    pub fn validate(&self) -> std::result::Result<(), AiMessageError> {
        match self.role {
            AiRole::System | AiRole::Developer => {
                for block in &self.content {
                    if !matches!(block, AiContent::Text { .. }) {
                        return Err(AiMessageError::InvalidBlockForRole {
                            role: self.role,
                            block_type: block.type_tag(),
                        });
                    }
                }
            }
            AiRole::User => {
                for block in &self.content {
                    match block {
                        AiContent::Text { .. }
                        | AiContent::Image { .. }
                        | AiContent::Document { .. } => {}
                        _ => {
                            return Err(AiMessageError::InvalidBlockForRole {
                                role: self.role,
                                block_type: block.type_tag(),
                            });
                        }
                    }
                }
            }
            AiRole::Assistant => {
                for block in &self.content {
                    match block {
                        AiContent::Text { .. }
                        | AiContent::Image { .. }
                        | AiContent::Document { .. }
                        | AiContent::ToolUse { .. }
                        | AiContent::Thinking { .. }
                        | AiContent::ProviderState { .. } => {}
                        _ => {
                            return Err(AiMessageError::InvalidBlockForRole {
                                role: self.role,
                                block_type: block.type_tag(),
                            });
                        }
                    }
                    if let AiContent::ToolUse { call_id, .. } = block {
                        if call_id.trim().is_empty() {
                            return Err(AiMessageError::MissingCallId);
                        }
                    }
                }
            }
            AiRole::Tool => {
                if self.content.len() != 1 {
                    return Err(AiMessageError::ToolRoleShape);
                }
                let AiContent::ToolResult {
                    call_id, content, ..
                } = &self.content[0]
                else {
                    return Err(AiMessageError::InvalidBlockForRole {
                        role: self.role,
                        block_type: self.content[0].type_tag(),
                    });
                };
                if call_id.trim().is_empty() {
                    return Err(AiMessageError::MissingCallId);
                }
                if content.is_empty() {
                    return Err(AiMessageError::EmptyToolResult);
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AiPayload {
    pub text: Option<String>,
    pub messages: Vec<AiMessage>,
    pub tool_specs: Vec<AiToolSpec>,
    pub resources: Vec<ResourceRef>,
    pub input_json: Option<Value>,
    pub options: Option<Value>,
}

impl AiPayload {
    pub fn new(
        text: Option<String>,
        messages: Vec<AiMessage>,
        tool_specs: Vec<AiToolSpec>,
        resources: Vec<ResourceRef>,
        input_json: Option<Value>,
        options: Option<Value>,
    ) -> Self {
        Self {
            text,
            messages,
            tool_specs,
            resources,
            input_json,
            options,
        }
    }

    /// Validate every message in `messages`. Called by the aicc client right
    /// before serializing the payload, so all paths funnel through one gate.
    pub fn validate_all_messages(&self) -> std::result::Result<(), AiMessageError> {
        for msg in &self.messages {
            msg.validate()?;
        }
        Ok(())
    }

    fn protocol_input_json(&self) -> Value {
        let mut input_json = match self.input_json.clone() {
            Some(Value::Object(map)) => map,
            Some(value) => {
                let mut map = serde_json::Map::new();
                map.insert("value".to_string(), value);
                map
            }
            None => serde_json::Map::new(),
        };

        if let Some(text) = self.text.as_ref() {
            input_json
                .entry("text".to_string())
                .or_insert_with(|| Value::String(text.clone()));
        }
        if !self.messages.is_empty() && !input_json.contains_key("messages") {
            if let Ok(value) = serde_json::to_value(&self.messages) {
                input_json.insert("messages".to_string(), value);
            }
        }
        if !self.tool_specs.is_empty() && !input_json.contains_key("tool_specs") {
            if let Ok(value) = serde_json::to_value(&self.tool_specs) {
                input_json.insert("tool_specs".to_string(), value);
            }
        }

        Value::Object(input_json)
    }
}

impl Default for AiPayload {
    fn default() -> Self {
        Self {
            text: None,
            messages: vec![],
            tool_specs: vec![],
            resources: vec![],
            input_json: Some(json!({})),
            options: Some(json!({})),
        }
    }
}

impl Serialize for AiPayload {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let options = self.options.clone().unwrap_or_else(|| json!({}));
        let mut state = serializer.serialize_struct("AiPayload", 3)?;
        state.serialize_field("input_json", &self.protocol_input_json())?;
        state.serialize_field("resources", &self.resources)?;
        state.serialize_field("options", &options)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for AiPayload {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct AiPayloadHelper {
            #[serde(default)]
            input_json: Option<Value>,
            #[serde(default)]
            resources: Vec<ResourceRef>,
            #[serde(default)]
            options: Option<Value>,
        }

        let helper = AiPayloadHelper::deserialize(deserializer)?;
        let mut payload = Self {
            text: None,
            messages: vec![],
            tool_specs: vec![],
            resources: helper.resources,
            input_json: helper.input_json,
            options: helper.options,
        };
        if let Some(Value::Object(body)) = payload.input_json.as_mut() {
            if let Some(Value::String(text)) = body.remove("text") {
                payload.text = Some(text);
            }
            if let Some(value) = body.get("messages").cloned() {
                match serde_json::from_value::<Vec<AiMessage>>(value) {
                    Ok(messages) => {
                        payload.messages = messages;
                        body.remove("messages");
                    }
                    Err(_) => {
                        // Legacy/non-typed shape; leave it in input_json so the
                        // provider's compat path can still pick it up.
                    }
                }
            }
            if let Some(value) = body.get("tool_specs").cloned() {
                match serde_json::from_value::<Vec<AiToolSpec>>(value) {
                    Ok(specs) => {
                        payload.tool_specs = specs;
                        body.remove("tool_specs");
                    }
                    Err(_) => {
                        // Same reasoning as messages — preserve the raw shape
                        // for any provider that knows how to handle it.
                    }
                }
            }
        }
        Ok(payload)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiUsage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiCost {
    pub amount: f64,
    pub currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiArtifact {
    pub name: String,
    pub resource: ResourceRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiToolCall {
    pub name: String,
    pub args: HashMap<String, Value>,
    pub call_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiResponse {
    pub message: AiMessage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<AiUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<AiCost>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_task_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<Value>,
}

impl Default for AiResponse {
    fn default() -> Self {
        Self {
            message: AiMessage::text(AiRole::Assistant, String::new()),
            usage: None,
            cost: None,
            finish_reason: None,
            provider_task_ref: None,
            extra: None,
        }
    }
}

impl AiResponse {
    pub fn new(message: AiMessage) -> Self {
        Self {
            message,
            ..Self::default()
        }
    }

    pub fn text(text: impl Into<String>) -> Self {
        Self::new(AiMessage::text(AiRole::Assistant, text))
    }

    pub fn from_parts(
        text: Option<String>,
        tool_calls: Vec<AiToolCall>,
        artifacts: Vec<AiArtifact>,
    ) -> Self {
        Self::new(Self::message_from_parts(text, tool_calls, artifacts))
    }

    pub fn message_from_parts(
        text: Option<String>,
        tool_calls: Vec<AiToolCall>,
        artifacts: Vec<AiArtifact>,
    ) -> AiMessage {
        let mut content = Vec::new();
        if let Some(text) = text {
            content.push(AiContent::Text { text });
        }
        for call in tool_calls {
            content.push(AiContent::ToolUse {
                call_id: call.call_id,
                name: call.name,
                args: call.args,
            });
        }
        for artifact in artifacts {
            content.push(artifact.into_content());
        }
        if content.is_empty() {
            content.push(AiContent::Text {
                text: String::new(),
            });
        }
        AiMessage::new(AiRole::Assistant, content)
    }

    pub fn text_content(&self) -> String {
        self.message.text_content()
    }

    pub fn tool_calls(&self) -> Vec<AiToolCall> {
        self.message.tool_calls()
    }

    pub fn artifacts(&self) -> Vec<AiArtifact> {
        self.message
            .content
            .iter()
            .enumerate()
            .filter_map(|(idx, block)| match block {
                AiContent::Image { source } => Some(AiArtifact {
                    name: format!("image_{}", idx + 1),
                    resource: source.clone(),
                    mime: resource_ref_mime(source),
                    metadata: None,
                }),
                AiContent::Document { source, title } => Some(AiArtifact {
                    name: title
                        .clone()
                        .unwrap_or_else(|| format!("document_{}", idx + 1)),
                    resource: source.clone(),
                    mime: resource_ref_mime(source),
                    metadata: None,
                }),
                _ => None,
            })
            .collect()
    }

    pub fn validate(&self) -> std::result::Result<(), AiMessageError> {
        if self.message.role != AiRole::Assistant {
            return Err(AiMessageError::InvalidBlockForRole {
                role: self.message.role,
                block_type: "response_message",
            });
        }
        self.message.validate()
    }
}

impl AiArtifact {
    pub fn into_content(self) -> AiContent {
        let is_image = self
            .mime
            .as_deref()
            .map(|mime| mime.starts_with("image/"))
            .unwrap_or(false);
        if is_image {
            AiContent::Image {
                source: self.resource,
            }
        } else {
            AiContent::Document {
                source: self.resource,
                title: Some(self.name),
            }
        }
    }
}

fn resource_ref_mime(source: &ResourceRef) -> Option<String> {
    match source {
        ResourceRef::Url { mime_hint, .. } => mime_hint.clone(),
        ResourceRef::Base64 { mime, .. } => Some(mime.clone()),
        ResourceRef::NamedObject { .. } => None,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiMethodRequest {
    pub capability: Capability,
    pub model: ModelSpec,
    pub requirements: Requirements,
    #[serde(default, skip_serializing_if = "is_default_model_disable")]
    pub disable: ModelDisable,
    pub payload: AiPayload,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<RoutePolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_options: Option<AiTaskOptions>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiTaskOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<i64>,
}

impl AiMethodRequest {
    pub fn new(
        capability: Capability,
        model: ModelSpec,
        requirements: Requirements,
        payload: AiPayload,
        idempotency_key: Option<String>,
    ) -> Self {
        Self {
            capability,
            model,
            requirements,
            disable: ModelDisable::default(),
            payload,
            policy: None,
            idempotency_key,
            task_options: None,
        }
    }

    pub fn with_policy(mut self, policy: Option<RoutePolicy>) -> Self {
        self.policy = policy;
        self
    }

    pub fn with_task_options(mut self, task_options: Option<AiTaskOptions>) -> Self {
        self.task_options = task_options;
        self
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Failed to parse AiMethodRequest: {}", error))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiMethodStatus {
    Succeeded,
    Running,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiMethodResponse {
    pub task_id: String,
    pub status: AiMethodStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<AiResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_ref: Option<String>,
}

impl AiMethodResponse {
    pub fn new(
        task_id: String,
        status: AiMethodStatus,
        result: Option<AiResponse>,
        event_ref: Option<String>,
    ) -> Self {
        Self {
            task_id,
            status,
            result,
            event_ref,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RouteResolveRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub api_type: String,
    pub logical_model: String,
    #[serde(default)]
    pub requirements: Requirements,
    #[serde(default, skip_serializing_if = "is_default_model_disable")]
    pub disable: ModelDisable,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<RoutePolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_overlay: Option<AiccRouteOverlay>,
}

impl RouteResolveRequest {
    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Failed to parse RouteResolveRequest: {}", error))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RouteFallbackAttempt {
    pub exact_model: String,
    pub provider_instance_name: String,
    pub provider_model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RouteResolveResponse {
    pub selected_exact_model: String,
    pub provider_instance_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_driver: Option<String>,
    pub provider_model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enabled_capabilities: Vec<Feature>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled_capabilities: Vec<Feature>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fallback_attempts: Vec<RouteFallbackAttempt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_trace: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inventory_revision: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LlmChatInvokeRequest {
    pub exact_model: String,
    #[serde(default)]
    pub messages: Vec<AiMessage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<AiToolSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_format: Option<RespFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<AiPayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_options: Option<AiTaskOptions>,
}

impl LlmChatInvokeRequest {
    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Failed to parse LlmChatInvokeRequest: {}", error))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LlmChatInvokeResponse {
    pub task_id: String,
    pub status: AiMethodStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<AiMessage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<AiToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<AiUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<AiCost>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_task_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_trace: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_ref: Option<String>,
}

impl From<AiMethodResponse> for LlmChatInvokeResponse {
    fn from(value: AiMethodResponse) -> Self {
        let tool_calls = value
            .result
            .as_ref()
            .map(|result| result.tool_calls())
            .unwrap_or_default();
        Self {
            task_id: value.task_id,
            status: value.status,
            message: value.result.as_ref().map(|result| result.message.clone()),
            tool_calls,
            usage: value
                .result
                .as_ref()
                .and_then(|result| result.usage.clone()),
            cost: value.result.as_ref().and_then(|result| result.cost.clone()),
            finish_reason: value
                .result
                .as_ref()
                .and_then(|result| result.finish_reason.clone()),
            provider_task_ref: value
                .result
                .as_ref()
                .and_then(|result| result.provider_task_ref.clone()),
            route_trace: value
                .result
                .as_ref()
                .and_then(|result| result.extra.as_ref())
                .and_then(|extra| extra.get("route_trace").cloned()),
            event_ref: value.event_ref,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextToImageInvokeRequest {
    pub exact_model: String,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub negative_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<AiPayload>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_options: Option<AiTaskOptions>,
}

impl TextToImageInvokeRequest {
    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse TextToImageInvokeRequest: {}",
                error
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextToImageInvokeResponse {
    pub task_id: String,
    pub status: AiMethodStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<AiArtifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<AiUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<AiCost>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_task_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_trace: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_ref: Option<String>,
}

impl From<AiMethodResponse> for TextToImageInvokeResponse {
    fn from(value: AiMethodResponse) -> Self {
        Self {
            task_id: value.task_id,
            status: value.status,
            artifacts: value
                .result
                .as_ref()
                .map(|result| result.artifacts())
                .unwrap_or_default(),
            usage: value
                .result
                .as_ref()
                .and_then(|result| result.usage.clone()),
            cost: value.result.as_ref().and_then(|result| result.cost.clone()),
            provider_task_ref: value
                .result
                .as_ref()
                .and_then(|result| result.provider_task_ref.clone()),
            route_trace: value
                .result
                .as_ref()
                .and_then(|result| result.extra.as_ref())
                .and_then(|extra| extra.get("route_trace").cloned()),
            event_ref: value.event_ref,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CancelRequest {
    pub task_id: String,
}

impl CancelRequest {
    pub fn new(task_id: String) -> Self {
        Self { task_id }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Failed to parse CancelRequest: {}", error))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CancelResponse {
    pub task_id: String,
    pub accepted: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DriverMetadataUpdateGetReq {}

impl DriverMetadataUpdateGetReq {
    pub fn new() -> Self {
        Self {}
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse DriverMetadataUpdateGetReq: {}",
                error
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DriverMetadataUpdateSetReq {
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_secs: Option<u64>,
}

impl DriverMetadataUpdateSetReq {
    pub fn new(enabled: bool, source_url: Option<String>, interval_secs: Option<u64>) -> Self {
        Self {
            enabled,
            source_url,
            interval_secs,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse DriverMetadataUpdateSetReq: {}",
                error
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DriverMetadataUpdateStatus {
    Disabled,
    Idle,
    Updating,
    Healthy,
    Degraded,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DriverMetadataUpdateView {
    pub enabled: bool,
    #[serde(default)]
    pub source_url: Option<String>,
    pub source_configured: bool,
    pub interval_secs: u64,
    pub status: DriverMetadataUpdateStatus,
    #[serde(default)]
    pub active_revision: Option<u64>,
    #[serde(default)]
    pub last_attempt_at_ms: Option<u64>,
    #[serde(default)]
    pub last_success_at_ms: Option<u64>,
    #[serde(default)]
    pub last_error: Option<String>,
    pub consecutive_failures: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DriverMetadataRuntimeApply {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_scheduled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DriverMetadataUpdateSetResponse {
    pub ok: bool,
    pub settings_revision: u64,
    pub settings: DriverMetadataUpdateView,
    pub runtime_apply: DriverMetadataRuntimeApply,
}

impl CancelResponse {
    pub fn new(task_id: String, accepted: bool) -> Self {
        Self { task_id, accepted }
    }
}

pub enum AiccClient {
    InProcess(Box<dyn AiccHandler>),
    KRPC(Box<kRPC>),
}

impl AiccClient {
    pub fn new(krpc_client: kRPC) -> Self {
        Self::new_krpc(Box::new(krpc_client))
    }

    pub fn new_in_process(handler: Box<dyn AiccHandler>) -> Self {
        Self::InProcess(handler)
    }

    pub fn new_krpc(krpc_client: Box<kRPC>) -> Self {
        Self::KRPC(krpc_client)
    }

    pub async fn set_context(&self, context: RPCContext) {
        match self {
            Self::InProcess(_) => {}
            Self::KRPC(client) => {
                client.set_context(context).await;
            }
        }
    }

    pub async fn call_method(
        &self,
        method: &str,
        request: AiMethodRequest,
    ) -> std::result::Result<AiMethodResponse, RPCErrors> {
        if !ai_methods::is_ai_method(method) {
            return Err(RPCErrors::UnknownMethod(method.to_string()));
        }

        request
            .payload
            .validate_all_messages()
            .map_err(|err| RPCErrors::ParseRequestError(format!("invalid AiMessage: {err}")))?;

        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_method(method, request, ctx).await
            }
            Self::KRPC(client) => {
                let req_json = serde_json::to_value(&request).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize AiMethodRequest: {}",
                        error
                    ))
                })?;
                let result = client.call(method, req_json).await?;
                serde_json::from_value(result).map_err(|error| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to parse AI method response: {}",
                        error
                    ))
                })
            }
        }
    }

    pub async fn route_resolve(
        &self,
        request: RouteResolveRequest,
    ) -> std::result::Result<RouteResolveResponse, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_route_resolve(request, ctx).await
            }
            Self::KRPC(client) => {
                let req_json = serde_json::to_value(&request).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize RouteResolveRequest: {}",
                        error
                    ))
                })?;
                let result = client.call(ai_methods::ROUTE_RESOLVE, req_json).await?;
                serde_json::from_value(result).map_err(|error| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to parse route.resolve response: {}",
                        error
                    ))
                })
            }
        }
    }

    pub async fn chat_completions_create(
        &self,
        request: LlmChatInvokeRequest,
    ) -> std::result::Result<LlmChatInvokeResponse, RPCErrors> {
        request
            .messages
            .iter()
            .try_for_each(AiMessage::validate)
            .map_err(|err| RPCErrors::ParseRequestError(format!("invalid AiMessage: {err}")))?;
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_chat_completions_create(request, ctx).await
            }
            Self::KRPC(client) => {
                let req_json = serde_json::to_value(&request).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize LlmChatInvokeRequest: {}",
                        error
                    ))
                })?;
                let result = client
                    .call(ai_methods::CHAT_COMPLETIONS_CREATE, req_json)
                    .await?;
                serde_json::from_value(result).map_err(|error| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to parse chat.completions.create response: {}",
                        error
                    ))
                })
            }
        }
    }

    pub async fn images_generate(
        &self,
        request: TextToImageInvokeRequest,
    ) -> std::result::Result<TextToImageInvokeResponse, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_images_generate(request, ctx).await
            }
            Self::KRPC(client) => {
                let req_json = serde_json::to_value(&request).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize TextToImageInvokeRequest: {}",
                        error
                    ))
                })?;
                let result = client.call(ai_methods::IMAGES_GENERATE, req_json).await?;
                serde_json::from_value(result).map_err(|error| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to parse images.generate response: {}",
                        error
                    ))
                })
            }
        }
    }

    pub async fn helper_llm_chat(
        &self,
        request: AiMethodRequest,
    ) -> std::result::Result<AiMethodResponse, RPCErrors> {
        request
            .payload
            .validate_all_messages()
            .map_err(|err| RPCErrors::ParseRequestError(format!("invalid AiMessage: {err}")))?;

        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_helper_llm_chat(request, ctx).await
            }
            Self::KRPC(client) => {
                let req_json = serde_json::to_value(&request).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize AiMethodRequest: {}",
                        error
                    ))
                })?;
                let result = client.call(ai_methods::HELPER_LLM_CHAT, req_json).await?;
                serde_json::from_value(result).map_err(|error| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to parse helper.llm_chat response: {}",
                        error
                    ))
                })
            }
        }
    }

    pub async fn helper_text_to_image(
        &self,
        request: AiMethodRequest,
    ) -> std::result::Result<AiMethodResponse, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_helper_text_to_image(request, ctx).await
            }
            Self::KRPC(client) => {
                let req_json = serde_json::to_value(&request).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize AiMethodRequest: {}",
                        error
                    ))
                })?;
                let result = client
                    .call(ai_methods::HELPER_TEXT_TO_IMAGE, req_json)
                    .await?;
                serde_json::from_value(result).map_err(|error| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to parse helper.text_to_image response: {}",
                        error
                    ))
                })
            }
        }
    }

    pub async fn cancel(&self, task_id: &str) -> std::result::Result<CancelResponse, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_cancel(task_id, ctx).await
            }
            Self::KRPC(client) => {
                let req = CancelRequest::new(task_id.to_string());
                let req_json = serde_json::to_value(&req).map_err(|error| {
                    RPCErrors::ReasonError(format!("Failed to serialize CancelRequest: {}", error))
                })?;
                let result = client.call("cancel", req_json).await?;
                serde_json::from_value(result).map_err(|error| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to parse cancel response: {}",
                        error
                    ))
                })
            }
        }
    }

    pub async fn list_models(&self) -> std::result::Result<Value, RPCErrors> {
        match self {
            Self::InProcess(_) => Err(RPCErrors::UnknownMethod("models.list".to_string())),
            Self::KRPC(client) => client.call("models.list", serde_json::json!({})).await,
        }
    }

    pub async fn get_driver_metadata_update(
        &self,
    ) -> std::result::Result<DriverMetadataUpdateView, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_driver_metadata_update_get(ctx).await
            }
            Self::KRPC(client) => {
                let request = DriverMetadataUpdateGetReq::new();
                let request = serde_json::to_value(request).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize DriverMetadataUpdateGetReq: {}",
                        error
                    ))
                })?;
                let result = client
                    .call(ai_methods::DRIVER_METADATA_UPDATE_GET, request)
                    .await?;
                serde_json::from_value(result).map_err(|error| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to parse driver metadata update view: {}",
                        error
                    ))
                })
            }
        }
    }

    pub async fn set_driver_metadata_update(
        &self,
        request: DriverMetadataUpdateSetReq,
    ) -> std::result::Result<DriverMetadataUpdateSetResponse, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler
                    .handle_driver_metadata_update_set(request, ctx)
                    .await
            }
            Self::KRPC(client) => {
                let request = serde_json::to_value(request).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize DriverMetadataUpdateSetReq: {}",
                        error
                    ))
                })?;
                let result = client
                    .call(ai_methods::DRIVER_METADATA_UPDATE_SET, request)
                    .await?;
                serde_json::from_value(result).map_err(|error| {
                    RPCErrors::ParserResponseError(format!(
                        "Failed to parse driver metadata update set response: {}",
                        error
                    ))
                })
            }
        }
    }
}

#[async_trait]
pub trait AiccHandler: Send + Sync {
    async fn handle_method(
        &self,
        method: &str,
        request: AiMethodRequest,
        ctx: RPCContext,
    ) -> std::result::Result<AiMethodResponse, RPCErrors>;

    async fn handle_cancel(
        &self,
        task_id: &str,
        ctx: RPCContext,
    ) -> std::result::Result<CancelResponse, RPCErrors>;

    async fn handle_route_resolve(
        &self,
        _request: RouteResolveRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<RouteResolveResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::ROUTE_RESOLVE.to_string(),
        ))
    }

    async fn handle_chat_completions_create(
        &self,
        _request: LlmChatInvokeRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<LlmChatInvokeResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::CHAT_COMPLETIONS_CREATE.to_string(),
        ))
    }

    async fn handle_images_generate(
        &self,
        _request: TextToImageInvokeRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<TextToImageInvokeResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::IMAGES_GENERATE.to_string(),
        ))
    }

    async fn handle_helper_llm_chat(
        &self,
        _request: AiMethodRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<AiMethodResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::HELPER_LLM_CHAT.to_string(),
        ))
    }

    async fn handle_helper_text_to_image(
        &self,
        _request: AiMethodRequest,
        _ctx: RPCContext,
    ) -> std::result::Result<AiMethodResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::HELPER_TEXT_TO_IMAGE.to_string(),
        ))
    }

    async fn handle_driver_metadata_update_get(
        &self,
        _ctx: RPCContext,
    ) -> std::result::Result<DriverMetadataUpdateView, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::DRIVER_METADATA_UPDATE_GET.to_string(),
        ))
    }

    async fn handle_driver_metadata_update_set(
        &self,
        _request: DriverMetadataUpdateSetReq,
        _ctx: RPCContext,
    ) -> std::result::Result<DriverMetadataUpdateSetResponse, RPCErrors> {
        Err(RPCErrors::UnknownMethod(
            ai_methods::DRIVER_METADATA_UPDATE_SET.to_string(),
        ))
    }
}

pub struct AiccServerHandler<T: AiccHandler>(pub T);

impl<T: AiccHandler> AiccServerHandler<T> {
    pub fn new(handler: T) -> Self {
        Self(handler)
    }
}

#[async_trait]
impl<T: AiccHandler> RPCHandler for AiccServerHandler<T> {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        ip_from: IpAddr,
    ) -> std::result::Result<RPCResponse, RPCErrors> {
        let seq = req.seq;
        let trace_id = req.trace_id.clone();
        let ctx = RPCContext::from_request(&req, ip_from);

        let method = req.method.clone();
        let result = match method.as_str() {
            ai_methods::CANCEL => {
                let cancel_req = CancelRequest::from_json(req.params)?;
                let result = self.0.handle_cancel(&cancel_req.task_id, ctx).await?;
                RPCResult::Success(json!(result))
            }
            ai_methods::ROUTE_RESOLVE => {
                let route_req = RouteResolveRequest::from_json(req.params)?;
                let result = self.0.handle_route_resolve(route_req, ctx).await?;
                RPCResult::Success(json!(result))
            }
            ai_methods::CHAT_COMPLETIONS_CREATE => {
                let invoke_req = LlmChatInvokeRequest::from_json(req.params)?;
                invoke_req
                    .messages
                    .iter()
                    .try_for_each(AiMessage::validate)
                    .map_err(|err| {
                        RPCErrors::ParseRequestError(format!("invalid AiMessage: {err}"))
                    })?;
                let result = self
                    .0
                    .handle_chat_completions_create(invoke_req, ctx)
                    .await?;
                RPCResult::Success(json!(result))
            }
            ai_methods::IMAGES_GENERATE => {
                let invoke_req = TextToImageInvokeRequest::from_json(req.params)?;
                let result = self.0.handle_images_generate(invoke_req, ctx).await?;
                RPCResult::Success(json!(result))
            }
            ai_methods::HELPER_LLM_CHAT => {
                let method_req = AiMethodRequest::from_json(req.params)?;
                let result = self.0.handle_helper_llm_chat(method_req, ctx).await?;
                RPCResult::Success(json!(result))
            }
            ai_methods::HELPER_TEXT_TO_IMAGE => {
                let method_req = AiMethodRequest::from_json(req.params)?;
                let result = self.0.handle_helper_text_to_image(method_req, ctx).await?;
                RPCResult::Success(json!(result))
            }
            ai_methods::DRIVER_METADATA_UPDATE_GET => {
                DriverMetadataUpdateGetReq::from_json(req.params)?;
                let result = self.0.handle_driver_metadata_update_get(ctx).await?;
                RPCResult::Success(json!(result))
            }
            ai_methods::DRIVER_METADATA_UPDATE_SET => {
                let update_req = DriverMetadataUpdateSetReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_driver_metadata_update_set(update_req, ctx)
                    .await?;
                RPCResult::Success(json!(result))
            }
            method if ai_methods::is_ai_method(method) => {
                let method_req = AiMethodRequest::from_json(req.params)?;
                let result = self.0.handle_method(method, method_req, ctx).await?;
                RPCResult::Success(json!(result))
            }
            _ => return Err(RPCErrors::UnknownMethod(req.method.clone())),
        };

        Ok(RPCResponse {
            result,
            seq,
            trace_id,
        })
    }
}

pub fn generate_aicc_service_doc() -> AppDoc {
    const VERSION: &str = env!("CARGO_PKG_VERSION");
    let owner_did = DID::from_str("did:bns:buckyos").unwrap();
    AppDoc::builder(
        AppType::Service,
        AICC_SERVICE_UNIQUE_ID,
        VERSION,
        "did:bns:buckyos",
        &owner_did,
    )
    .show_name("AI Compute Center")
    .selector_type(SelectorType::Random)
    .build()
    .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::{Arc, Mutex};

    #[derive(Default, Debug)]
    struct MockCalls {
        method: Option<String>,
        request: Option<AiMethodRequest>,
        cancel_task_id: Option<String>,
    }

    #[derive(Clone)]
    struct MockAicc {
        calls: Arc<Mutex<MockCalls>>,
    }

    impl MockAicc {
        fn new() -> Self {
            Self {
                calls: Arc::new(Mutex::new(MockCalls::default())),
            }
        }
    }

    #[async_trait]
    impl AiccHandler for MockAicc {
        async fn handle_method(
            &self,
            method: &str,
            request: AiMethodRequest,
            _ctx: RPCContext,
        ) -> std::result::Result<AiMethodResponse, RPCErrors> {
            let mut calls = self.calls.lock().unwrap();
            calls.method = Some(method.to_string());
            calls.request = Some(request);
            Ok(AiMethodResponse::new(
                "task-001".to_string(),
                AiMethodStatus::Succeeded,
                Some(AiResponse {
                    message: AiMessage::text(AiRole::Assistant, "mock result"),
                    usage: Some(AiUsage {
                        input_tokens: Some(4),
                        output_tokens: Some(8),
                        total_tokens: Some(12),
                    }),
                    cost: Some(AiCost {
                        amount: 0.001,
                        currency: "USD".to_string(),
                    }),
                    finish_reason: Some("stop".to_string()),
                    provider_task_ref: Some("provider-task-001".to_string()),
                    extra: None,
                }),
                Some("task://task-001/events".to_string()),
            ))
        }

        async fn handle_cancel(
            &self,
            task_id: &str,
            _ctx: RPCContext,
        ) -> std::result::Result<CancelResponse, RPCErrors> {
            let mut calls = self.calls.lock().unwrap();
            calls.cancel_task_id = Some(task_id.to_string());
            Ok(CancelResponse::new(task_id.to_string(), true))
        }

        async fn handle_driver_metadata_update_get(
            &self,
            _ctx: RPCContext,
        ) -> std::result::Result<DriverMetadataUpdateView, RPCErrors> {
            Ok(DriverMetadataUpdateView {
                enabled: true,
                source_url: Some(
                    "https://metadata.example/aicc/driver-metadata/index.json".to_string(),
                ),
                source_configured: true,
                interval_secs: 900,
                status: DriverMetadataUpdateStatus::Healthy,
                active_revision: Some(7),
                last_attempt_at_ms: None,
                last_success_at_ms: None,
                last_error: None,
                consecutive_failures: 0,
            })
        }

        async fn handle_driver_metadata_update_set(
            &self,
            request: DriverMetadataUpdateSetReq,
            ctx: RPCContext,
        ) -> std::result::Result<DriverMetadataUpdateSetResponse, RPCErrors> {
            let mut settings = self.handle_driver_metadata_update_get(ctx).await?;
            settings.enabled = request.enabled;
            Ok(DriverMetadataUpdateSetResponse {
                ok: true,
                settings_revision: 42,
                settings,
                runtime_apply: DriverMetadataRuntimeApply {
                    ok: true,
                    refresh_scheduled: Some(true),
                    error: None,
                },
            })
        }
    }

    fn sample_method_request() -> AiMethodRequest {
        AiMethodRequest::new(
            Capability::Llm,
            ModelSpec::new("llm.plan.default".to_string(), None),
            Requirements::new(vec![features::PLAN.to_string()], Some(3000), None, None),
            AiPayload::new(
                Some("write a release note".to_string()),
                vec![AiMessage::text(AiRole::User, "summarize this commit")],
                vec![],
                vec![
                    ResourceRef::url(
                        "cyfs://example/object/1".to_string(),
                        Some("text/plain".to_string()),
                    ),
                    ResourceRef::named_object(ObjId::new("chunk:123456").unwrap()),
                ],
                Some(json!({})),
                Some(json!({"temperature": 0.3})),
            ),
            Some("idem-1".to_string()),
        )
    }

    #[test]
    fn test_generate_aicc_service_doc() {
        let doc = generate_aicc_service_doc();
        let json_str = serde_json::to_string_pretty(&doc).unwrap();
        println!("json: {}", json_str);
    }

    #[test]
    fn test_protocol_field_names() {
        let mut request = sample_method_request();
        request.requirements.resp_format = RespFormat::Json;
        request.policy = Some(RoutePolicy::default());

        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(value.pointer("/capability"), Some(&json!("llm")));
        assert_eq!(
            value.pointer("/requirements/resp_format"),
            Some(&json!("json"))
        );
        assert!(value.pointer("/requirements/resp_foramt").is_none());
        assert!(value.pointer("/payload/text").is_none());
        assert!(value.pointer("/payload/messages").is_none());
        assert!(value.pointer("/payload/tool_specs").is_none());
        assert_eq!(
            value.pointer("/payload/input_json/messages/0/content/0/type"),
            Some(&json!("text"))
        );
        assert_eq!(
            value.pointer("/payload/input_json/messages/0/content/0/text"),
            Some(&json!("summarize this commit"))
        );
        assert_eq!(
            value.pointer("/payload/input_json/messages/0/role"),
            Some(&json!("user"))
        );
    }

    #[test]
    fn ai_message_tool_calls_preserve_block_order() {
        let mut first_args = HashMap::new();
        first_args.insert("q".to_string(), json!("first"));
        let mut second_args = HashMap::new();
        second_args.insert("q".to_string(), json!("second"));
        let msg = AiMessage::new(
            AiRole::Assistant,
            vec![
                AiContent::text("before"),
                AiContent::tool_use("call-1", "lookup", first_args),
                AiContent::text("middle"),
                AiContent::tool_use("call-2", "search", second_args),
            ],
        );

        let calls = msg.tool_calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].call_id, "call-1");
        assert_eq!(calls[0].name, "lookup");
        assert_eq!(calls[1].call_id, "call-2");
        assert_eq!(calls[1].name, "search");
    }

    #[tokio::test]
    async fn test_in_process_client_with_mock() {
        let mock = MockAicc::new();
        let calls = mock.calls.clone();
        let client = AiccClient::new_in_process(Box::new(mock));

        let request = sample_method_request();
        let method_result = client
            .call_method(ai_methods::LLM_CHAT, request.clone())
            .await
            .unwrap();
        assert_eq!(method_result.task_id, "task-001");
        assert_eq!(method_result.status, AiMethodStatus::Succeeded);
        assert_eq!(
            method_result
                .result
                .as_ref()
                .map(|summary| summary.text_content()),
            Some("mock result".to_string())
        );

        let cancel_result = client.cancel("task-001").await.unwrap();
        assert_eq!(cancel_result.task_id, "task-001");
        assert!(cancel_result.accepted);

        let calls = calls.lock().unwrap();
        assert_eq!(calls.method.as_deref(), Some(ai_methods::LLM_CHAT));
        assert_eq!(calls.request, Some(request));
        assert_eq!(calls.cancel_task_id.as_deref(), Some("task-001"));
    }

    #[tokio::test]
    async fn driver_metadata_update_protocol_is_typed_end_to_end() {
        let client = AiccClient::new_in_process(Box::new(MockAicc::new()));
        let view = client.get_driver_metadata_update().await.unwrap();
        assert_eq!(view.status, DriverMetadataUpdateStatus::Healthy);
        assert_eq!(view.active_revision, Some(7));

        let response = client
            .set_driver_metadata_update(DriverMetadataUpdateSetReq::new(false, None, Some(900)))
            .await
            .unwrap();
        assert!(response.ok);
        assert!(!response.settings.enabled);
        assert_eq!(response.settings_revision, 42);
        assert!(DriverMetadataUpdateSetReq::from_json(json!({
            "enabled": true,
            "unexpected": true
        }))
        .is_err());
    }

    #[tokio::test]
    async fn test_rpc_handler_adapter_with_mock() {
        let mock = MockAicc::new();
        let calls = mock.calls.clone();
        let rpc_handler = AiccServerHandler::new(mock);
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

        let request = sample_method_request();
        let method_req = RPCRequest {
            method: ai_methods::LLM_CHAT.to_string(),
            params: serde_json::to_value(&request).unwrap(),
            seq: 9,
            token: None,
            trace_id: None,
        };
        let method_resp = rpc_handler.handle_rpc_call(method_req, ip).await.unwrap();
        match method_resp.result {
            RPCResult::Success(value) => {
                let method_result: AiMethodResponse = serde_json::from_value(value).unwrap();
                assert_eq!(method_result.task_id, "task-001");
                assert_eq!(method_result.status, AiMethodStatus::Succeeded);
            }
            _ => panic!("Expected success response"),
        }

        let cancel_req = RPCRequest {
            method: "cancel".to_string(),
            params: json!({"task_id": "task-001"}),
            seq: 10,
            token: None,
            trace_id: None,
        };
        let cancel_resp = rpc_handler.handle_rpc_call(cancel_req, ip).await.unwrap();
        match cancel_resp.result {
            RPCResult::Success(value) => {
                let cancel_result: CancelResponse = serde_json::from_value(value).unwrap();
                assert_eq!(cancel_result.task_id, "task-001");
                assert!(cancel_result.accepted);
            }
            _ => panic!("Expected success response"),
        }

        let metadata_get = RPCRequest {
            method: ai_methods::DRIVER_METADATA_UPDATE_GET.to_string(),
            params: json!({}),
            seq: 11,
            token: None,
            trace_id: None,
        };
        let metadata_get = rpc_handler.handle_rpc_call(metadata_get, ip).await.unwrap();
        match metadata_get.result {
            RPCResult::Success(value) => {
                let view: DriverMetadataUpdateView = serde_json::from_value(value).unwrap();
                assert_eq!(view.status, DriverMetadataUpdateStatus::Healthy);
            }
            _ => panic!("Expected success response"),
        }

        let metadata_set = RPCRequest {
            method: ai_methods::DRIVER_METADATA_UPDATE_SET.to_string(),
            params: json!({"enabled": false, "interval_secs": 900}),
            seq: 12,
            token: None,
            trace_id: None,
        };
        let metadata_set = rpc_handler.handle_rpc_call(metadata_set, ip).await.unwrap();
        match metadata_set.result {
            RPCResult::Success(value) => {
                let response: DriverMetadataUpdateSetResponse =
                    serde_json::from_value(value).unwrap();
                assert!(!response.settings.enabled);
            }
            _ => panic!("Expected success response"),
        }

        let calls = calls.lock().unwrap();
        assert_eq!(calls.method.as_deref(), Some(ai_methods::LLM_CHAT));
        assert_eq!(calls.request, Some(request));
        assert_eq!(calls.cancel_task_id.as_deref(), Some("task-001"));
    }
}
