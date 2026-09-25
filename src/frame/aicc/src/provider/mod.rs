mod builtin;
mod inventory;
mod runtime;

#[cfg(test)]
pub(crate) use builtin::openai_catalog_files;
pub(crate) use builtin::{
    builtin_provider_codecs, builtin_provider_registry, claude_messages_adapter,
    register_sn_openai_adapter, resolve_sn_provider_instance_with_config, BuiltinProviderRequest,
    SnCredentialBroker, SnProviderInstanceInput,
};
pub(crate) use inventory::{
    catalog_only_inventory, CatalogOnlyDiscovery, CredentialResolver, DiscoveredModel,
    DiscoveryContext, FallbackDiscovery, InventoryBuilder, ModelAvailability, PricingSource,
    ProviderConnectionContract, ProviderConnectionInput, ProviderDiscovery,
    ProviderDiscoverySnapshot, ProviderFieldMode, ProviderFieldSchema, ProviderHealthState,
    ProviderInstanceConfig, ProviderInventorySnapshot, ProviderQuotaContext, ProviderQuotaLevel,
    ProviderQuotaObservation, ProviderQuotaObservationState, ProviderQuotaObserver,
    ProviderQuotaReading, ResolvedProviderConnection, StaticCredentialResolver,
};
use runtime::ProviderRuntime;
pub(crate) use runtime::ProviderRuntimeManager;

use crate::catalog::{CatalogSnapshot, Pricing};
use crate::error::{ProviderDraftValidationError, ProviderError, ProviderResult};
use crate::matching::MatchContext;
use crate::model::{
    InventoryModel, InventoryModelVariant, ModelUid, ProviderInventory as ModelProviderInventory,
};
use crate::protocol::{
    ArtifactUrlReader, CodecContext, CodecLimits, CodecRegistry, CredentialKind, HttpTransport,
    HttpTransportConfig, ProtocolError, ProtocolErrorKind, ProtocolResultValue, ResolvedCredential,
};
use crate::storage::{AiccStorage, InventoryLkgsRecord};
use async_trait::async_trait;
use buckyos_api::{AiCost, ApiType, ProviderStateCoordinate};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, watch, Mutex, RwLock};
use tokio::task::JoinHandle;

const INVENTORY_SCHEMA_VERSION: u32 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DiscoveryMode {
    MachineApi,
    CatalogOnly,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CredentialDescriptor {
    pub kind: CredentialKind,
    pub header_name: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct RefreshPolicy {
    pub interval: Duration,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
}

impl Default for RefreshPolicy {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(15 * 60),
            initial_backoff: Duration::from_secs(5),
            max_backoff: Duration::from_secs(5 * 60),
        }
    }
}

impl RefreshPolicy {
    fn validate(&self) -> ProviderResult<()> {
        if self.interval.is_zero()
            || self.initial_backoff.is_zero()
            || self.max_backoff < self.initial_backoff
        {
            return Err(ProviderError::InvalidConfiguration(
                "refresh interval/backoff is invalid".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ProviderProfile {
    pub provider_profile_id: String,
    pub display_name: String,
    pub default_protocol_adapter_id: String,
    pub credential: CredentialDescriptor,
    pub credential_variants: Vec<CredentialDescriptor>,
    pub discovery_mode: DiscoveryMode,
    pub refresh: RefreshPolicy,
    pub default_inventory: Option<ProviderDiscoverySnapshot>,
    pub accepts_any_adapter: bool,
}

impl ProviderProfile {
    fn validate(&self) -> ProviderResult<()> {
        validate_id("provider_profile_id", &self.provider_profile_id)?;
        validate_id(
            "default_protocol_adapter_id",
            &self.default_protocol_adapter_id,
        )?;
        if self.display_name.trim().is_empty() {
            return Err(ProviderError::InvalidConfiguration(
                "provider display name must not be empty".into(),
            ));
        }
        let mut kinds = BTreeSet::new();
        for credential in std::iter::once(&self.credential).chain(&self.credential_variants) {
            if !kinds.insert(credential.kind) {
                return Err(ProviderError::InvalidConfiguration(
                    "provider credential kinds must be unique".into(),
                ));
            }
            if credential.kind == CredentialKind::NamedHeader
                && credential.header_name.as_deref().is_none_or(str::is_empty)
            {
                return Err(ProviderError::InvalidConfiguration(
                    "named-header credentials require a header name".into(),
                ));
            }
        }
        self.refresh.validate()
    }

    pub(crate) fn credential_for(
        &self,
        requested: Option<CredentialKind>,
    ) -> ProviderResult<&CredentialDescriptor> {
        let Some(requested) = requested else {
            return Ok(&self.credential);
        };
        std::iter::once(&self.credential)
            .chain(&self.credential_variants)
            .find(|credential| credential.kind == requested)
            .ok_or_else(|| {
                ProviderError::InvalidConfiguration(format!(
                    "provider profile `{}` does not support credential kind `{}`",
                    self.provider_profile_id,
                    requested.as_str()
                ))
            })
    }

    fn with_credential(&self, requested: Option<CredentialKind>) -> ProviderResult<Self> {
        let credential = self.credential_for(requested)?.clone();
        let mut selected = self.clone();
        selected.credential = credential;
        selected.credential_variants.clear();
        Ok(selected)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CredentialReference {
    pub reference: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProviderAuthMode {
    ApiKey,
    DynamicLogin,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ProviderAuthConfig {
    ApiKey {
        credential_ref: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        credential_kind: Option<CredentialKind>,
    },
    DynamicLogin {
        login_profile: String,
        login_endpoint: String,
    },
}

impl ProviderAuthConfig {
    pub(crate) fn mode(&self) -> ProviderAuthMode {
        match self {
            Self::ApiKey { .. } => ProviderAuthMode::ApiKey,
            Self::DynamicLogin { .. } => ProviderAuthMode::DynamicLogin,
        }
    }

    pub(crate) fn validate(&self) -> ProviderResult<()> {
        match self {
            Self::ApiKey { credential_ref, .. } => {
                validate_nonempty("credential_ref", credential_ref)
            }
            Self::DynamicLogin {
                login_profile,
                login_endpoint,
            } => {
                validate_id("login_profile", login_profile)?;
                validate_provider_url("login_endpoint", login_endpoint)
            }
        }
    }

    pub(crate) fn credential_reference(&self) -> Option<CredentialReference> {
        match self {
            Self::ApiKey { credential_ref, .. } => Some(CredentialReference {
                reference: credential_ref.clone(),
            }),
            Self::DynamicLogin { .. } => None,
        }
    }

    pub(crate) fn credential_kind(&self) -> Option<CredentialKind> {
        match self {
            Self::ApiKey {
                credential_kind, ..
            } => *credential_kind,
            Self::DynamicLogin { .. } => None,
        }
    }

    pub(crate) fn dynamic_login_context(
        &self,
        provider_instance_name: impl Into<String>,
        user_name: impl Into<String>,
    ) -> ProviderResult<DynamicLoginContext> {
        self.validate()?;
        let Self::DynamicLogin {
            login_profile,
            login_endpoint,
        } = self
        else {
            return Err(ProviderError::InvalidConfiguration(
                "dynamic login context requires auth.mode=dynamic_login".into(),
            ));
        };
        DynamicLoginContext::new(
            provider_instance_name,
            user_name,
            login_profile.clone(),
            login_endpoint.clone(),
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DynamicLoginContext {
    pub provider_instance_name: String,
    pub user_name: String,
    pub login_profile: String,
    pub login_endpoint: String,
}

impl DynamicLoginContext {
    pub(crate) fn new(
        provider_instance_name: impl Into<String>,
        user_name: impl Into<String>,
        login_profile: impl Into<String>,
        login_endpoint: impl Into<String>,
    ) -> ProviderResult<Self> {
        let result = Self {
            provider_instance_name: provider_instance_name.into(),
            user_name: user_name.into(),
            login_profile: login_profile.into(),
            login_endpoint: login_endpoint.into(),
        };
        validate_id("provider_instance_name", &result.provider_instance_name)?;
        validate_nonempty("user_name", &result.user_name)?;
        validate_id("login_profile", &result.login_profile)?;
        validate_provider_url("login_endpoint", &result.login_endpoint)?;
        Ok(result)
    }

    pub(crate) fn cache_key(&self) -> &str {
        &self.provider_instance_name
    }
}

#[async_trait]
pub(crate) trait DynamicLoginCredentialResolver: Send + Sync {
    async fn resolve_dynamic(
        &self,
        context: &DynamicLoginContext,
    ) -> ProviderResult<ResolvedCredential>;

    async fn invalidate(&self, _provider_instance_name: &str) {}
}

#[async_trait]
pub(crate) trait ProviderInventoryStore: Send + Sync {
    async fn load(
        &self,
        provider_instance_name: &str,
    ) -> ProviderResult<Option<InventoryLkgsRecord>>;
    async fn commit(&self, record: &InventoryLkgsRecord) -> ProviderResult<()>;
}

#[async_trait]
impl ProviderInventoryStore for AiccStorage {
    async fn load(
        &self,
        provider_instance_name: &str,
    ) -> ProviderResult<Option<InventoryLkgsRecord>> {
        self.load_inventory(provider_instance_name)
            .await
            .map_err(|error| ProviderError::Storage(error.to_string()))
    }

    async fn commit(&self, record: &InventoryLkgsRecord) -> ProviderResult<()> {
        self.upsert_inventory(record)
            .await
            .map_err(|error| ProviderError::Storage(error.to_string()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProviderHealth {
    pub state: ProviderHealthState,
    pub consecutive_failures: u32,
    pub last_success_at_ms: Option<i64>,
    pub last_attempt_at_ms: Option<i64>,
    pub last_error: Option<String>,
}

impl Default for ProviderHealth {
    fn default() -> Self {
        Self {
            state: ProviderHealthState::Unknown,
            consecutive_failures: 0,
            last_success_at_ms: None,
            last_attempt_at_ms: None,
            last_error: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProviderRefreshTrigger {
    Initial,
    Manual,
    Scheduled,
    Reconciliation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProviderRefreshOutcome {
    Committed {
        changed: bool,
        inventory_revision: Option<String>,
        metadata_applied_seq: u64,
    },
    Failed {
        kind: ProviderRefreshFailure,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProviderRefreshFailure {
    InvalidConfiguration,
    UnknownDependency,
    Credential,
    Discovery,
    Inventory,
    Storage,
    Stopped,
    StaleCandidate,
}

impl ProviderRefreshFailure {
    fn from_error(error: &ProviderError) -> Self {
        match error {
            ProviderError::InvalidConfiguration(_)
            | ProviderError::DuplicateInstance(_)
            | ProviderError::UnknownInstance(_) => Self::InvalidConfiguration,
            ProviderError::UnknownProfile(_) | ProviderError::UnknownAdapter(_) => {
                Self::UnknownDependency
            }
            ProviderError::Credential(_) => Self::Credential,
            ProviderError::Discovery(_) | ProviderError::DiscoveryResponse(_) => Self::Discovery,
            ProviderError::Inventory(_) => Self::Inventory,
            ProviderError::Storage(_) => Self::Storage,
            ProviderError::Stopped => Self::Stopped,
            ProviderError::StaleCandidate => Self::StaleCandidate,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProviderRefreshEvent {
    pub provider_instance_name: String,
    pub trigger: ProviderRefreshTrigger,
    pub outcome: ProviderRefreshOutcome,
}

#[derive(Clone)]
pub(crate) struct ProviderInventoryCandidate {
    provider_instance_name: String,
    generation: u64,
    candidate_seq: u64,
    catalog_revision_seq: u64,
    inventory: Arc<ProviderInventorySnapshot>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProviderDraftValidationStage {
    Connection,
    Authentication,
    Protocol,
    Discovery,
    Inventory,
}

impl ProviderDraftValidationError {
    fn from_provider_error(stage: ProviderDraftValidationStage, error: &ProviderError) -> Self {
        Self {
            stage,
            kind: ProviderRefreshFailure::from_error(error),
        }
    }
}

#[derive(Clone)]
pub(crate) struct ProviderDraftConfig {
    pub provider_instance_name: String,
    pub provider_profile_id: String,
    pub protocol_adapter_id: String,
    pub provider_rules_id: Option<String>,
    pub base_url: Option<String>,
    pub region: Option<String>,
    pub workspace: Option<String>,
    pub account: Option<String>,
    pub auth: ProviderAuthConfig,
    pub dynamic_login_user_name: Option<String>,
}

impl fmt::Debug for ProviderDraftConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderDraftConfig")
            .field("provider_instance_name", &self.provider_instance_name)
            .field("provider_profile_id", &self.provider_profile_id)
            .field("protocol_adapter_id", &self.protocol_adapter_id)
            .field("provider_rules_id", &self.provider_rules_id)
            .field("base_url", &self.base_url)
            .field("region", &self.region)
            .field("workspace", &self.workspace)
            .field("account", &self.account)
            .field("auth_mode", &self.auth.mode())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ProviderDraftNegotiation {
    pub provider_profile_id: String,
    pub protocol_adapter_id: String,
    pub auth_mode: ProviderAuthMode,
    pub connection: ResolvedProviderConnection,
    pub catalog_revision_seq: u64,
    pub inventory: Arc<ProviderInventorySnapshot>,
}

impl fmt::Debug for ProviderInventoryCandidate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderInventoryCandidate")
            .field("provider_instance_name", &self.provider_instance_name)
            .field("generation", &self.generation)
            .field("candidate_seq", &self.candidate_seq)
            .field("catalog_revision_seq", &self.catalog_revision_seq)
            .field("inventory_revision", &self.inventory.inventory_revision)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub(crate) struct ExecutableProviderInstance {
    pub config: Arc<ProviderInstanceConfig>,
    pub profile: Arc<ProviderProfile>,
    inventory: Arc<ProviderInventorySnapshot>,
    generation: u64,
    runtime: Arc<ProviderRuntime>,
}

impl fmt::Debug for ExecutableProviderInstance {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExecutableProviderInstance")
            .field(
                "provider_instance_name",
                &self.config.provider_instance_name,
            )
            .field("provider_profile_id", &self.profile.provider_profile_id)
            .field("protocol_adapter_id", &self.config.protocol_adapter_id)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

impl ExecutableProviderInstance {
    pub(crate) async fn resolve_credential(&self) -> ProviderResult<ResolvedCredential> {
        self.runtime.resolve_credential().await
    }

    pub(crate) async fn health(&self) -> ProviderHealth {
        self.runtime.health.read().await.clone()
    }

    pub(crate) async fn current_inventory(&self) -> Arc<ProviderInventorySnapshot> {
        self.inventory.clone()
    }

    pub(crate) async fn quota_observation(&self) -> ProviderQuotaObservation {
        self.runtime.quota_observation().await
    }

    pub(crate) async fn open_artifact_url_reader(
        &self,
        codecs: &CodecRegistry,
        url: &str,
    ) -> ProtocolResultValue<ArtifactUrlReader> {
        let credential = self.resolve_credential().await.map_err(|_| {
            ProtocolError::new(
                ProtocolErrorKind::Authentication,
                "Provider artifact credential is unavailable",
            )
        })?;
        let max_response_bytes = 1024 * 1024 * 1024;
        let limits = CodecLimits {
            request_timeout: self.config.request_timeout,
            max_request_bytes: 1024,
            max_response_bytes,
        };
        let context = CodecContext {
            base_url: self.config.base_url.clone(),
            state_coordinate: ProviderStateCoordinate {
                provider_profile_id: self.profile.provider_profile_id.clone(),
                adapter_type: self.config.protocol_adapter_id.clone(),
                origin_provider: self.profile.provider_profile_id.clone(),
                origin_model: "artifact".to_owned(),
            },
            credential: Some(credential),
            resources: BTreeMap::new(),
            limits: limits.clone(),
        };
        let transport = HttpTransport::new(HttpTransportConfig {
            request_timeout: limits.request_timeout,
            max_request_bytes: limits.max_request_bytes,
            max_response_bytes: limits.max_response_bytes,
            ..HttpTransportConfig::default()
        })?;
        codecs
            .open_artifact_url_reader(&self.config.protocol_adapter_id, url, &context, &transport)
            .await
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ProviderRegistry {
    instances: BTreeMap<String, Arc<ExecutableProviderInstance>>,
}

impl ProviderRegistry {
    pub(crate) fn get(&self, name: &str) -> Option<Arc<ExecutableProviderInstance>> {
        self.instances.get(name).cloned()
    }

    pub(crate) fn list(&self) -> Vec<Arc<ExecutableProviderInstance>> {
        self.instances.values().cloned().collect()
    }
}

fn validate_discovery(discovery: &ProviderDiscoverySnapshot) -> ProviderResult<()> {
    if discovery.discovered_at_ms < 0 {
        return Err(ProviderError::DiscoveryResponse(
            "discovery timestamp must not be negative".into(),
        ));
    }
    let mut ids = BTreeSet::new();
    for model in &discovery.models {
        if model.provider_model_id.trim().is_empty() || model.provider_model_id.contains('@') {
            return Err(ProviderError::DiscoveryResponse(
                "provider model IDs must be non-empty and must not contain `@`".into(),
            ));
        }
        if !ids.insert(&model.provider_model_id) {
            return Err(ProviderError::DiscoveryResponse(format!(
                "duplicate provider model `{}`",
                model.provider_model_id
            )));
        }
        if let Some(pricing) = &model.pricing {
            validate_pricing(pricing)?;
        }
    }
    Ok(())
}

fn validate_pricing(pricing: &Pricing) -> ProviderResult<()> {
    crate::catalog::validate_pricing("discovery", pricing)
        .map_err(|error| ProviderError::DiscoveryResponse(error.to_string()))
}

fn validate_quota_reading(reading: ProviderQuotaReading) -> ProviderResult<ProviderQuotaReading> {
    if reading.remaining_cost_usd.as_ref().is_some_and(|value| {
        value.currency.trim().is_empty()
            || value.currency.trim() != value.currency
            || !value.amount.is_finite()
            || value.amount < 0.0
    }) || reading.reset_at_ms.is_some_and(|value| value < 0)
    {
        return Err(ProviderError::InvalidConfiguration(
            "provider quota observation contains an invalid value".into(),
        ));
    }
    Ok(reading)
}

fn resolve_operation(
    adapter: &crate::protocol::AdapterDescriptor,
    overrides: &BTreeMap<String, String>,
    remote_methods: Option<&BTreeSet<String>>,
    api_type: ApiType,
    api_type_name: &str,
) -> ProviderResult<Option<String>> {
    if let Some(operation) = overrides
        .get(api_type.typed_method())
        .or_else(|| overrides.get(api_type_name))
    {
        return Ok(Some(operation.clone()));
    }
    let matching = adapter
        .operations
        .values()
        .filter(|operation| {
            operation
                .bindings
                .iter()
                .any(|binding| binding.api_type == api_type)
        })
        .map(|operation| operation.operation_id.clone())
        .collect::<Vec<_>>();
    let matching = match remote_methods {
        Some(methods) => {
            let exact = matching
                .iter()
                .filter(|operation| methods.contains(*operation))
                .cloned()
                .collect::<Vec<_>>();
            if !exact.is_empty() {
                exact
            } else if methods.contains(api_type.typed_method()) || methods.contains(api_type_name) {
                matching
            } else {
                Vec::new()
            }
        }
        None => matching,
    };
    match matching.as_slice() {
        [] => Ok(None),
        [operation] => Ok(Some(operation.clone())),
        _ => Err(ProviderError::Inventory(format!(
            "adapter has multiple default operations for api_type `{api_type_name}`"
        ))),
    }
}

fn retain_supported_features(
    capabilities: &mut BTreeMap<String, Value>,
    adapter_features: &BTreeSet<String>,
    discovery_features: Option<&BTreeSet<String>>,
) {
    capabilities.retain(|name, value| {
        if !value.as_bool().unwrap_or(false) {
            return true;
        }
        adapter_features.contains(name)
            && discovery_features.is_none_or(|features| features.contains(name))
    });
}

fn model_list_fingerprint(models: &[DiscoveredModel]) -> String {
    let mut ids = models
        .iter()
        .map(|model| model.provider_model_id.as_str())
        .collect::<Vec<_>>();
    ids.sort_unstable();
    let mut hasher = Sha256::new();
    for id in ids {
        hasher.update((id.len() as u64).to_be_bytes());
        hasher.update(id.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn api_type_name(api_type: ApiType) -> ProviderResult<String> {
    serde_json::to_value(api_type)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or_else(|| ProviderError::Inventory("invalid API type".into()))
}

fn parse_api_type(value: &str) -> ProviderResult<ApiType> {
    serde_json::from_value(Value::String(value.to_owned())).map_err(|_| {
        ProviderError::Inventory(format!("catalog contains unsupported api_type `{value}`"))
    })
}

fn exponential_backoff(policy: &RefreshPolicy, failures: u32) -> Duration {
    let shift = failures.saturating_sub(1).min(31);
    policy
        .initial_backoff
        .checked_mul(1_u32 << shift)
        .unwrap_or(policy.max_backoff)
        .min(policy.max_backoff)
}

fn now_ms() -> ProviderResult<i64> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ProviderError::Inventory("system time is before unix epoch".into()))?
        .as_millis();
    i64::try_from(millis)
        .map_err(|_| ProviderError::Inventory("system time does not fit i64".into()))
}

fn validate_id(field: &str, value: &str) -> ProviderResult<()> {
    if value.is_empty()
        || value.trim() != value
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(ProviderError::InvalidConfiguration(format!(
            "{field} is not a valid stable ID"
        )));
    }
    Ok(())
}

fn validate_nonempty(field: &str, value: &str) -> ProviderResult<()> {
    if value.is_empty() || value.trim() != value || value.chars().any(char::is_control) {
        return Err(ProviderError::InvalidConfiguration(format!(
            "{field} must not be empty or contain surrounding/control whitespace"
        )));
    }
    Ok(())
}

fn validate_endpoint_field(field: &str, value: &str) -> ProviderResult<()> {
    validate_nonempty(field, value)?;
    if value
        .bytes()
        .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')))
    {
        return Err(ProviderError::InvalidConfiguration(format!(
            "{field} contains characters that are unsafe in a base_url template"
        )));
    }
    Ok(())
}

fn validate_provider_url(field: &str, value: &str) -> ProviderResult<()> {
    let url = reqwest::Url::parse(value).map_err(|_| {
        ProviderError::InvalidConfiguration(format!("{field} must be an absolute URL"))
    })?;
    if !matches!(url.scheme(), "http" | "https") || url.cannot_be_a_base() {
        return Err(ProviderError::InvalidConfiguration(format!(
            "{field} must use http or https and support relative paths"
        )));
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ProviderError::InvalidConfiguration(format!(
            "{field} must not contain user info, query, or fragment"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
