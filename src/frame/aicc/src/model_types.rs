use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

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
    #[serde(rename = "llm.chat")]
    LlmChat,
    #[serde(rename = "llm.completion")]
    LlmCompletion,
    #[serde(rename = "image.txt2image")]
    ImageTextToImage,
    #[serde(rename = "image.img2image")]
    ImageToImage,
    #[serde(rename = "embedding")]
    Embedding,
}

impl ApiType {
    pub fn namespace(&self) -> &'static str {
        match self {
            Self::LlmChat | Self::LlmCompletion => "llm",
            Self::ImageTextToImage | Self::ImageToImage => "image",
            Self::Embedding => "embedding",
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
    pub vision: bool,
    #[serde(default)]
    pub max_context_tokens: Option<u64>,
}

impl ModelCapabilities {
    pub fn supports(&self, required: &RequiredModelFeatures) -> bool {
        (!required.streaming || self.streaming)
            && (!required.tool_call || self.tool_call)
            && (!required.json_schema || self.json_schema)
            && (!required.vision || self.vision)
            && required
                .min_context_tokens
                .map(|min| self.max_context_tokens.unwrap_or(0) >= min)
                .unwrap_or(true)
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
    pub vision: bool,
    #[serde(default)]
    pub min_context_tokens: Option<u64>,
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

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ModelPricing {
    #[serde(default)]
    pub input_token_usd: Option<f64>,
    #[serde(default)]
    pub output_token_usd: Option<f64>,
    #[serde(default)]
    pub cache_input_token_usd: Option<f64>,
    #[serde(default)]
    pub estimated_cost_usd: Option<f64>,
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
    pub version: Option<String>,
    #[serde(default)]
    pub inventory_revision: Option<String>,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LockedValue<T> {
    pub value: T,
    #[serde(default)]
    pub locked: bool,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoutePolicy {
    #[serde(default)]
    pub profile: SchedulerProfile,
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
            fallback: None,
        }
    }
}

impl RoutePolicy {
    pub fn from_config(config: &PolicyConfig) -> Self {
        let mut policy = RoutePolicy::default();
        if let Some(value) = config.profile.as_ref() {
            policy.profile = value.value.clone();
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
    #[serde(default)]
    pub route_paths: Vec<String>,
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
            route_paths: Vec::new(),
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
    #[serde(default)]
    pub priority_path: Vec<f64>,
    pub exact_model_weight: f64,
    #[serde(default)]
    pub final_score: Option<f64>,
    pub selected: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FallbackTraceItem {
    pub from: String,
    pub to: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RouteTrace {
    pub request_id: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub session_config_revision: Option<String>,
    #[serde(default)]
    pub session_config_updated: bool,
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
    pub session_sticky_hit: bool,
    pub scheduler_profile: SchedulerProfile,
    #[serde(default)]
    pub runtime_failover_count: u64,
    #[serde(default)]
    pub warnings: Vec<String>,
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
            parameter_scale: None,
            api_types: vec![ApiType::LlmChat],
            logical_mounts: vec!["llm.gpt5".to_string()],
            capabilities: ModelCapabilities {
                streaming: true,
                tool_call: true,
                json_schema: true,
                vision: false,
                max_context_tokens: Some(128_000),
            },
            attributes: ModelAttributes::default(),
            pricing: ModelPricing::default(),
            health: ModelHealth::default(),
        };

        assert!(model.supports_api_type(&ApiType::LlmChat));
        assert!(!model.supports_api_type(&ApiType::ImageTextToImage));
        assert!(model.supports_requirements(&RequiredModelFeatures {
            tool_call: true,
            min_context_tokens: Some(32_000),
            ..Default::default()
        }));
        assert!(!model.supports_requirements(&RequiredModelFeatures {
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
                "api_types": ["llm.chat", "llm.completion"],
                "logical_mounts": ["llm.gpt5"],
                "capabilities": {
                    "streaming": true,
                    "tool_call": true,
                    "json_schema": true,
                    "max_context_tokens": 128000
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
        assert_eq!(
            inventory.models[0].api_types,
            vec![ApiType::LlmChat, ApiType::LlmCompletion]
        );
        assert!(inventory.models[0].supports_api_type(&ApiType::LlmChat));
        let encoded = serde_json::to_value(&inventory).unwrap();
        assert_eq!(encoded["models"][0]["api_types"][0], "llm.chat");
    }
}
