#![allow(dead_code)]

mod builtin;

#[allow(unused_imports)]
pub(crate) use builtin::*;

use crate::catalog::{CatalogSnapshot, Pricing};
use crate::matching::MatchContext;
use crate::model::{
    InventoryModel, InventoryModelVariant, ModelUid, ProviderInventory as ModelProviderInventory,
};
use crate::protocol::{CodecRegistry, CredentialKind, ResolvedCredential};
use crate::storage::{AiccStorage, InventoryLkgsRecord};
use async_trait::async_trait;
use buckyos_api::{AiCost, ApiType};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::sync::{broadcast, watch, Mutex, RwLock};
use tokio::task::JoinHandle;

const INVENTORY_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub(crate) enum ProviderError {
    #[error("invalid provider configuration: {0}")]
    InvalidConfiguration(String),
    #[error("provider profile `{0}` is not registered")]
    UnknownProfile(String),
    #[error("protocol adapter `{0}` is not registered")]
    UnknownAdapter(String),
    #[error("provider instance `{0}` is already registered")]
    DuplicateInstance(String),
    #[error("provider instance `{0}` is not registered")]
    UnknownInstance(String),
    #[error("credential resolution failed: {0}")]
    Credential(String),
    #[error("provider discovery failed: {0}")]
    Discovery(String),
    #[error("inventory build failed: {0}")]
    Inventory(String),
    #[error("inventory storage failed: {0}")]
    Storage(String),
    #[error("provider instance stopped before refresh could commit")]
    Stopped,
    #[error("provider inventory candidate is stale")]
    StaleCandidate,
}

pub(crate) type ProviderResult<T> = Result<T, ProviderError>;

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
    pub discovery_mode: DiscoveryMode,
    pub refresh: RefreshPolicy,
    pub default_inventory: Option<ProviderDiscoverySnapshot>,
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
        if self.credential.kind == CredentialKind::NamedHeader
            && self
                .credential
                .header_name
                .as_deref()
                .is_none_or(str::is_empty)
        {
            return Err(ProviderError::InvalidConfiguration(
                "named-header credentials require a header name".into(),
            ));
        }
        self.refresh.validate()
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
            Self::ApiKey { credential_ref } => validate_nonempty("credential_ref", credential_ref),
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
            Self::ApiKey { credential_ref } => Some(CredentialReference {
                reference: credential_ref.clone(),
            }),
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderFieldMode {
    Unsupported,
    Optional,
    Required,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderFieldSchema {
    pub mode: ProviderFieldMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub allowed_values: BTreeSet<String>,
}

impl ProviderFieldSchema {
    pub(crate) fn unsupported() -> Self {
        Self {
            mode: ProviderFieldMode::Unsupported,
            default_value: None,
            allowed_values: BTreeSet::new(),
        }
    }

    pub(crate) fn optional() -> Self {
        Self {
            mode: ProviderFieldMode::Optional,
            default_value: None,
            allowed_values: BTreeSet::new(),
        }
    }

    pub(crate) fn optional_with_default(default_value: impl Into<String>) -> Self {
        Self {
            mode: ProviderFieldMode::Optional,
            default_value: Some(default_value.into()),
            allowed_values: BTreeSet::new(),
        }
    }

    pub(crate) fn required() -> Self {
        Self {
            mode: ProviderFieldMode::Required,
            default_value: None,
            allowed_values: BTreeSet::new(),
        }
    }

    pub(crate) fn with_allowed_values(
        mut self,
        values: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.allowed_values = values.into_iter().map(Into::into).collect();
        self
    }

    fn resolve(&self, field: &str, value: Option<&str>) -> ProviderResult<Option<String>> {
        if self.mode == ProviderFieldMode::Unsupported {
            if value.is_some() || self.default_value.is_some() || !self.allowed_values.is_empty() {
                return Err(ProviderError::InvalidConfiguration(format!(
                    "{field} is not supported"
                )));
            }
            return Ok(None);
        }
        let value = value
            .map(str::to_owned)
            .or_else(|| self.default_value.clone());
        if self.mode == ProviderFieldMode::Required && value.is_none() {
            return Err(ProviderError::InvalidConfiguration(format!(
                "{field} is required"
            )));
        }
        if let Some(value) = value.as_deref() {
            validate_endpoint_field(field, value)?;
            if !self.allowed_values.is_empty() && !self.allowed_values.contains(value) {
                return Err(ProviderError::InvalidConfiguration(format!(
                    "{field} has an unsupported value"
                )));
            }
        }
        Ok(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderConnectionContract {
    pub default_base_url: String,
    pub region: ProviderFieldSchema,
    pub workspace: ProviderFieldSchema,
    pub account: ProviderFieldSchema,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ProviderConnectionInput<'a> {
    pub base_url: Option<&'a str>,
    pub region: Option<&'a str>,
    pub workspace: Option<&'a str>,
    pub account: Option<&'a str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedProviderConnection {
    pub base_url: String,
    pub region: Option<String>,
    pub workspace: Option<String>,
    pub account: Option<String>,
}

impl ProviderConnectionContract {
    pub(crate) fn resolve(
        &self,
        input: ProviderConnectionInput<'_>,
    ) -> ProviderResult<ResolvedProviderConnection> {
        let region = self.region.resolve("region", input.region)?;
        let workspace = self.workspace.resolve("workspace", input.workspace)?;
        let account = self.account.resolve("account", input.account)?;
        let mut base_url = input.base_url.unwrap_or(&self.default_base_url).to_owned();
        for (placeholder, value) in [
            ("{region}", region.as_deref()),
            ("{workspace}", workspace.as_deref()),
            ("{account}", account.as_deref()),
        ] {
            if base_url.contains(placeholder) {
                let value = value.ok_or_else(|| {
                    ProviderError::InvalidConfiguration(format!(
                        "{placeholder} is required to resolve base_url"
                    ))
                })?;
                base_url = base_url.replace(placeholder, value);
            }
        }
        if base_url.contains('{') || base_url.contains('}') {
            return Err(ProviderError::InvalidConfiguration(
                "base_url contains an unsupported placeholder".into(),
            ));
        }
        validate_provider_url("base_url", &base_url)?;
        Ok(ResolvedProviderConnection {
            base_url,
            region,
            workspace,
            account,
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ProviderInstanceConfig {
    pub provider_instance_name: String,
    pub provider_profile_id: String,
    pub protocol_adapter_id: String,
    pub base_url: String,
    pub credential: CredentialReference,
    pub provider_rules_id: Option<String>,
    pub region: Option<String>,
    pub workspace: Option<String>,
    pub account: Option<String>,
}

impl ProviderInstanceConfig {
    fn validate(&self) -> ProviderResult<()> {
        validate_id("provider_instance_name", &self.provider_instance_name)?;
        validate_id("provider_profile_id", &self.provider_profile_id)?;
        validate_id("protocol_adapter_id", &self.protocol_adapter_id)?;
        if self.credential.reference.trim().is_empty() {
            return Err(ProviderError::InvalidConfiguration(
                "credential reference must not be empty".into(),
            ));
        }
        let url = reqwest::Url::parse(&self.base_url).map_err(|_| {
            ProviderError::InvalidConfiguration("base_url must be an absolute URL".into())
        })?;
        if !matches!(url.scheme(), "http" | "https") || url.cannot_be_a_base() {
            return Err(ProviderError::InvalidConfiguration(
                "base_url must use http or https and support relative paths".into(),
            ));
        }
        if let Some(provider_rules_id) = &self.provider_rules_id {
            validate_id("provider_rules_id", provider_rules_id)?;
        }
        Ok(())
    }
}

#[async_trait]
pub(crate) trait CredentialResolver: Send + Sync {
    async fn resolve(
        &self,
        descriptor: &CredentialDescriptor,
        reference: &CredentialReference,
    ) -> ProviderResult<ResolvedCredential>;
}

#[derive(Clone)]
pub(crate) struct StaticCredentialResolver {
    values: BTreeMap<String, String>,
}

impl StaticCredentialResolver {
    pub(crate) fn new(values: BTreeMap<String, String>) -> Self {
        Self { values }
    }
}

#[async_trait]
impl CredentialResolver for StaticCredentialResolver {
    async fn resolve(
        &self,
        descriptor: &CredentialDescriptor,
        reference: &CredentialReference,
    ) -> ProviderResult<ResolvedCredential> {
        let value = self.values.get(&reference.reference).ok_or_else(|| {
            ProviderError::Credential("credential reference was not resolved".into())
        })?;
        let result = match descriptor.kind {
            CredentialKind::Bearer => {
                ResolvedCredential::bearer(&reference.reference, value.clone())
            }
            CredentialKind::NamedHeader => ResolvedCredential::named_header(
                &reference.reference,
                descriptor.header_name.as_deref().unwrap_or_default(),
                value.clone(),
            ),
            CredentialKind::FalKey => {
                ResolvedCredential::fal_key(&reference.reference, value.clone())
            }
            CredentialKind::GlmJwt => ResolvedCredential::glm_jwt(
                &reference.reference,
                value,
                SystemTime::now(),
                Duration::from_secs(10 * 60),
            ),
        };
        result.map_err(|error| ProviderError::Credential(error.to_string()))
    }
}

pub(crate) struct DiscoveryContext<'a> {
    pub profile: &'a ProviderProfile,
    pub instance: &'a ProviderInstanceConfig,
    pub credential: &'a ResolvedCredential,
}

pub(crate) struct ProviderQuotaContext<'a> {
    pub profile: &'a ProviderProfile,
    pub instance: &'a ProviderInstanceConfig,
    pub credential: &'a ResolvedCredential,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderQuotaLevel {
    Normal,
    NearLimit,
    Exhausted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderQuotaObservationState {
    Normal,
    NearLimit,
    Exhausted,
    Unsupported,
    QueryFailed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderQuotaReading {
    pub state: ProviderQuotaLevel,
    pub remaining_request_units: Option<u64>,
    pub remaining_cost_usd: Option<AiCost>,
    pub reset_at_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderQuotaObservation {
    pub state: ProviderQuotaObservationState,
    pub remaining_request_units: Option<u64>,
    pub remaining_cost_usd: Option<AiCost>,
    pub reset_at_ms: Option<i64>,
    pub observed_at_ms: i64,
    pub source: String,
}

#[async_trait]
pub(crate) trait ProviderQuotaObserver: Send + Sync {
    fn source(&self) -> &'static str;

    async fn observe(
        &self,
        context: &ProviderQuotaContext<'_>,
    ) -> ProviderResult<ProviderQuotaReading>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ModelAvailability {
    Available,
    Unavailable,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderHealthState {
    Unknown,
    Healthy,
    Degraded,
    Unavailable,
    Stopped,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DiscoveredModel {
    pub provider_model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_types: Option<Vec<ApiType>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supported_features: Option<BTreeSet<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_methods: Option<BTreeSet<String>>,
    pub availability: ModelAvailability,
    #[serde(default)]
    pub deprecated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<Pricing>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderDiscoverySnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    pub discovered_at_ms: i64,
    pub health: ProviderHealthState,
    #[serde(default)]
    pub models: Vec<DiscoveredModel>,
}

#[async_trait]
pub(crate) trait ProviderDiscovery: Send + Sync {
    async fn discover(
        &self,
        context: &DiscoveryContext<'_>,
    ) -> ProviderResult<ProviderDiscoverySnapshot>;
}

pub(crate) struct CatalogOnlyDiscovery {
    snapshot: ProviderDiscoverySnapshot,
}

impl CatalogOnlyDiscovery {
    pub(crate) fn new(snapshot: ProviderDiscoverySnapshot) -> Self {
        Self { snapshot }
    }
}

#[async_trait]
impl ProviderDiscovery for CatalogOnlyDiscovery {
    async fn discover(
        &self,
        _context: &DiscoveryContext<'_>,
    ) -> ProviderResult<ProviderDiscoverySnapshot> {
        Ok(self.snapshot.clone())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PricingSource {
    Discovery,
    ProviderRules,
    ModelDriver,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InventoryPricing {
    pub source: PricingSource,
    pub value: Pricing,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderInventoryModel {
    pub provider_model_id: String,
    pub model_uid: String,
    pub model_driver_id: String,
    pub origin_model_id: String,
    pub api_types: Vec<ApiType>,
    #[serde(default)]
    pub logical_mounts: Vec<String>,
    #[serde(default)]
    pub capabilities: BTreeMap<String, Value>,
    #[serde(default)]
    pub operations: BTreeMap<String, String>,
    pub availability: ModelAvailability,
    #[serde(default)]
    pub deprecated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_methods: Option<BTreeSet<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<InventoryPricing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_catalog_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_rules_revision: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderInventorySnapshot {
    pub schema_version: u32,
    pub provider_instance_name: String,
    pub provider_profile_id: String,
    pub protocol_adapter_id: String,
    pub provider_model_list_fingerprint: String,
    pub metadata_applied_seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inventory_revision: Option<String>,
    pub discovered_at_ms: i64,
    pub health: ProviderHealthState,
    pub models: Vec<ProviderInventoryModel>,
}

impl ProviderInventorySnapshot {
    pub(crate) fn as_model_inventory(&self) -> ModelProviderInventory {
        ModelProviderInventory {
            provider_instance_name: self.provider_instance_name.clone(),
            provider_profile_id: self.provider_profile_id.clone(),
            protocol_adapter_id: self.protocol_adapter_id.clone(),
            inventory_revision: self.inventory_revision.clone().unwrap_or_default(),
            models: self
                .models
                .iter()
                .filter(|model| {
                    model.availability == ModelAvailability::Available
                        && !model.deprecated
                        && !model.api_types.is_empty()
                })
                .map(|model| InventoryModel {
                    provider_model_id: model.provider_model_id.clone(),
                    model_driver_id: model.model_driver_id.clone(),
                    origin_model_id: model.origin_model_id.clone(),
                    api_types: model.api_types.clone(),
                    logical_mounts: model.logical_mounts.clone(),
                    variants: Vec::<InventoryModelVariant>::new(),
                    capabilities: model.capabilities.clone(),
                    attributes: BTreeMap::from([
                        ("model_uid".into(), Value::String(model.model_uid.clone())),
                        (
                            "pricing".into(),
                            model
                                .pricing
                                .as_ref()
                                .and_then(|pricing| serde_json::to_value(pricing).ok())
                                .unwrap_or(Value::Null),
                        ),
                    ]),
                    operations: model.operations.clone(),
                })
                .collect(),
        }
    }
}

pub(crate) struct InventoryBuilder;

impl InventoryBuilder {
    pub(crate) fn build(
        profile: &ProviderProfile,
        instance: &ProviderInstanceConfig,
        discovery: ProviderDiscoverySnapshot,
        catalog: &CatalogSnapshot,
        codecs: &CodecRegistry,
    ) -> ProviderResult<ProviderInventorySnapshot> {
        validate_discovery(&discovery)?;
        let adapter = codecs
            .adapter(&instance.protocol_adapter_id)
            .ok_or_else(|| ProviderError::UnknownAdapter(instance.protocol_adapter_id.clone()))?;
        let rules_id = instance
            .provider_rules_id
            .as_deref()
            .unwrap_or(&profile.provider_profile_id);
        let rules = catalog.provider_rules(rules_id);
        let fingerprint = model_list_fingerprint(&discovery.models);
        let mut models = Vec::new();

        for discovered in discovery.models {
            let mapped_origin = rules
                .filter(|rules| !rules.origin_mappings.is_empty())
                .map(|_| catalog.resolve_provider_origin(rules_id, &discovered.provider_model_id))
                .transpose()
                .map_err(|error| ProviderError::Inventory(error.to_string()))?;
            if let (Some(discovered_origin), Some(mapped_origin)) = (
                discovered.origin_model_id.as_deref(),
                mapped_origin.as_ref(),
            ) {
                if discovered_origin != mapped_origin.origin_model_id {
                    return Err(ProviderError::Inventory(format!(
                        "discovery origin_model_id {discovered_origin:?} conflicts with Provider Rules mapping {:?}",
                        mapped_origin.origin_model_id
                    )));
                }
            }
            let origin_model_id = mapped_origin
                .as_ref()
                .map(|origin| origin.origin_model_id.clone())
                .or_else(|| discovered.origin_model_id.clone())
                .unwrap_or_else(|| discovered.provider_model_id.clone());
            let mapped_candidate_drivers = mapped_origin
                .as_ref()
                .map(|origin| vec![origin.model_driver_id.clone()]);
            let candidate_drivers = mapped_candidate_drivers
                .as_deref()
                .or_else(|| rules.and_then(|rules| rules.metadata_drivers.as_deref()));
            let dimensions = MatchContext::from([
                (
                    "provider_model_id".into(),
                    Value::String(discovered.provider_model_id.clone()),
                ),
                (
                    "origin_model_id".into(),
                    Value::String(origin_model_id.clone()),
                ),
            ]);
            let provider_rule = if catalog.provider_rules(rules_id).is_some() {
                catalog
                    .resolve_provider_rule(rules_id, &discovered.provider_model_id, &dimensions)
                    .map_err(|error| ProviderError::Inventory(error.to_string()))?
            } else {
                None
            };
            if provider_rule
                .as_ref()
                .is_some_and(|rule| rule.action.exclude)
            {
                continue;
            }
            let resolved = catalog
                .resolve_model(&origin_model_id, candidate_drivers, &dimensions)
                .map_err(|error| ProviderError::Inventory(error.to_string()))?;
            if resolved.semantics.exclude.unwrap_or(false) {
                continue;
            }
            let Some(model_driver_id) = resolved.model_driver_id.clone() else {
                continue;
            };
            let mut static_api_types = resolved.semantics.api_types.unwrap_or_default();
            let mut capabilities = resolved.semantics.capabilities.unwrap_or_default();
            let mut pricing = resolved.semantics.pricing.map(|value| InventoryPricing {
                source: PricingSource::ModelDriver,
                value,
            });
            let mut provider_rules_revision = None;
            let operation_overrides = if let Some(rule) = &provider_rule {
                let narrowed = rule.action.narrow(&static_api_types, &capabilities);
                static_api_types = narrowed.api_types;
                capabilities = narrowed.capabilities;
                if let Some(value) = &rule.action.pricing {
                    pricing = Some(InventoryPricing {
                        source: PricingSource::ProviderRules,
                        value: value.clone(),
                    });
                }
                provider_rules_revision = Some(rule.catalog_revision_seq);
                &rule.action.operations
            } else {
                static EMPTY_OPERATIONS: std::sync::LazyLock<BTreeMap<String, String>> =
                    std::sync::LazyLock::new(BTreeMap::new);
                &EMPTY_OPERATIONS
            };
            if let Some(value) = discovered.pricing.clone() {
                pricing = Some(InventoryPricing {
                    source: PricingSource::Discovery,
                    value,
                });
            }

            let discovered_api_types = discovered.api_types.as_ref().map(|items| {
                items
                    .iter()
                    .filter_map(|api_type| api_type_name(*api_type).ok())
                    .collect::<BTreeSet<_>>()
            });
            if let Some(discovered_api_types) = &discovered_api_types {
                static_api_types.retain(|api_type| discovered_api_types.contains(api_type));
            }

            let mut api_types = Vec::new();
            let mut operations = BTreeMap::new();
            let mut adapter_features = BTreeSet::new();
            for api_type_name in static_api_types {
                let api_type = parse_api_type(&api_type_name)?;
                let operation_id =
                    resolve_operation(adapter, operation_overrides, api_type, &api_type_name)?;
                let Some(operation_id) = operation_id else {
                    continue;
                };
                if discovered.remote_methods.as_ref().is_some_and(|methods| {
                    !methods.contains(&operation_id)
                        && !methods.contains(api_type.typed_method())
                        && !methods.contains(&api_type_name)
                }) {
                    continue;
                }
                let descriptor = codecs
                    .operation_descriptor(&instance.protocol_adapter_id, &operation_id, api_type)
                    .map_err(|error| ProviderError::Inventory(error.to_string()))?;
                let binding = descriptor
                    .binding(api_type)
                    .map_err(|error| ProviderError::Inventory(error.to_string()))?;
                adapter_features.extend(binding.supported_features.iter().cloned());
                api_types.push(api_type);
                operations.insert(api_type_name, operation_id);
            }
            retain_supported_features(
                &mut capabilities,
                &adapter_features,
                discovered.supported_features.as_ref(),
            );
            api_types.sort_by_key(|api_type| api_type.typed_method());
            if discovered.availability != ModelAvailability::Available || discovered.deprecated {
                api_types.clear();
                operations.clear();
            }
            let model_uid = ModelUid::new(
                &model_driver_id,
                &origin_model_id,
                &instance.protocol_adapter_id,
                None,
            )
            .map_err(|error| ProviderError::Inventory(error.to_string()))?
            .as_stable_string();
            models.push(ProviderInventoryModel {
                provider_model_id: discovered.provider_model_id,
                model_uid,
                model_driver_id,
                origin_model_id,
                api_types,
                logical_mounts: resolved.semantics.logical_mounts.unwrap_or_default(),
                capabilities,
                operations,
                availability: discovered.availability,
                deprecated: discovered.deprecated,
                remote_methods: discovered.remote_methods,
                pricing,
                model_catalog_revision: resolved.catalog_revision_seq,
                provider_rules_revision,
            });
        }
        models.sort_by(|left, right| left.provider_model_id.cmp(&right.provider_model_id));
        Ok(ProviderInventorySnapshot {
            schema_version: INVENTORY_SCHEMA_VERSION,
            provider_instance_name: instance.provider_instance_name.clone(),
            provider_profile_id: profile.provider_profile_id.clone(),
            protocol_adapter_id: instance.protocol_adapter_id.clone(),
            provider_model_list_fingerprint: fingerprint,
            metadata_applied_seq: catalog.target_revision_seq(),
            inventory_revision: discovery.revision,
            discovered_at_ms: discovery.discovered_at_ms,
            health: discovery.health,
            models,
        })
    }
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
            ProviderError::Discovery(_) => Self::Discovery,
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProviderDraftValidationError {
    pub stage: ProviderDraftValidationStage,
    pub kind: ProviderRefreshFailure,
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

impl ProviderInventoryCandidate {
    pub(crate) fn inventory(&self) -> &Arc<ProviderInventorySnapshot> {
        &self.inventory
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

struct ProviderRuntime {
    config: Arc<ProviderInstanceConfig>,
    profile: Arc<ProviderProfile>,
    discovery: Arc<dyn ProviderDiscovery>,
    credential_resolver: Arc<dyn CredentialResolver>,
    quota_observer: Option<Arc<dyn ProviderQuotaObserver>>,
    catalog: Arc<RwLock<Arc<CatalogSnapshot>>>,
    codecs: Arc<CodecRegistry>,
    store: Arc<dyn ProviderInventoryStore>,
    registry: Arc<RwLock<Arc<ProviderRegistry>>>,
    refresh_events: broadcast::Sender<ProviderRefreshEvent>,
    generation: u64,
    current_generation: Arc<AtomicU64>,
    current_candidate_seq: AtomicU64,
    commit_gate: RwLock<()>,
    refresh_lock: Mutex<()>,
    inventory: RwLock<Arc<ProviderInventorySnapshot>>,
    health: RwLock<ProviderHealth>,
    stopped: AtomicBool,
    stop_tx: watch::Sender<bool>,
    task: Mutex<Option<JoinHandle<()>>>,
    stop_lock: Mutex<()>,
}

impl ProviderRuntime {
    async fn resolve_credential(&self) -> ProviderResult<ResolvedCredential> {
        self.credential_resolver
            .resolve(&self.profile.credential, &self.config.credential)
            .await
    }

    async fn quota_observation(&self) -> ProviderQuotaObservation {
        let observed_at_ms = now_ms().unwrap_or(0);
        let Some(observer) = &self.quota_observer else {
            return ProviderQuotaObservation {
                state: ProviderQuotaObservationState::Unsupported,
                remaining_request_units: None,
                remaining_cost_usd: None,
                reset_at_ms: None,
                observed_at_ms,
                source: "unsupported".into(),
            };
        };
        let source = observer.source().to_owned();
        let reading = match self.resolve_credential().await {
            Ok(credential) => {
                observer
                    .observe(&ProviderQuotaContext {
                        profile: &self.profile,
                        instance: &self.config,
                        credential: &credential,
                    })
                    .await
            }
            Err(error) => Err(error),
        };
        match reading.and_then(validate_quota_reading) {
            Ok(reading) => ProviderQuotaObservation {
                state: match reading.state {
                    ProviderQuotaLevel::Normal => ProviderQuotaObservationState::Normal,
                    ProviderQuotaLevel::NearLimit => ProviderQuotaObservationState::NearLimit,
                    ProviderQuotaLevel::Exhausted => ProviderQuotaObservationState::Exhausted,
                },
                remaining_request_units: reading.remaining_request_units,
                remaining_cost_usd: reading.remaining_cost_usd,
                reset_at_ms: reading.reset_at_ms,
                observed_at_ms,
                source,
            },
            Err(_) => ProviderQuotaObservation {
                state: ProviderQuotaObservationState::QueryFailed,
                remaining_request_units: None,
                remaining_cost_usd: None,
                reset_at_ms: None,
                observed_at_ms,
                source,
            },
        }
    }

    async fn build_candidate(&self) -> ProviderResult<ProviderInventoryCandidate> {
        if !self.is_current() {
            return Err(ProviderError::Stopped);
        }
        let candidate_seq = self.current_candidate_seq.fetch_add(1, Ordering::AcqRel) + 1;
        let catalog = self.catalog.read().await.clone();
        let credential = self.resolve_credential().await?;
        let snapshot = self
            .discovery
            .discover(&DiscoveryContext {
                profile: &self.profile,
                instance: &self.config,
                credential: &credential,
            })
            .await?;
        let inventory = InventoryBuilder::build(
            &self.profile,
            &self.config,
            snapshot,
            &catalog,
            &self.codecs,
        )?;
        Ok(ProviderInventoryCandidate {
            provider_instance_name: self.config.provider_instance_name.clone(),
            generation: self.generation,
            candidate_seq,
            catalog_revision_seq: catalog.target_revision_seq(),
            inventory: Arc::new(inventory),
        })
    }

    async fn refresh_once(
        self: &Arc<Self>,
        force: bool,
        trigger: ProviderRefreshTrigger,
        publish: bool,
    ) -> ProviderResult<Arc<ProviderInventorySnapshot>> {
        let _refresh = self.refresh_lock.lock().await;
        let attempt_at_ms = now_ms()?;
        let candidate = match self.build_candidate().await {
            Ok(built) => built,
            Err(error) => {
                self.record_failure(attempt_at_ms, &error).await;
                self.publish_refresh_event(
                    trigger,
                    ProviderRefreshOutcome::Failed {
                        kind: ProviderRefreshFailure::from_error(&error),
                    },
                );
                return Err(error);
            }
        };
        let current = self.inventory.read().await.clone();
        let changed = force
            || current.provider_model_list_fingerprint
                != candidate.inventory.provider_model_list_fingerprint
            || current.metadata_applied_seq != candidate.inventory.metadata_applied_seq;
        if changed {
            if let Err(error) = self.commit_candidate(&candidate, publish).await {
                if !matches!(error, ProviderError::StaleCandidate) {
                    self.record_failure(attempt_at_ms, &error).await;
                }
                self.publish_refresh_event(
                    trigger,
                    ProviderRefreshOutcome::Failed {
                        kind: ProviderRefreshFailure::from_error(&error),
                    },
                );
                return Err(error);
            }
        }
        self.record_success(attempt_at_ms, candidate.inventory.health)
            .await?;
        let inventory = if changed {
            candidate.inventory.clone()
        } else {
            current
        };
        self.publish_refresh_event(
            trigger,
            ProviderRefreshOutcome::Committed {
                changed,
                inventory_revision: inventory.inventory_revision.clone(),
                metadata_applied_seq: inventory.metadata_applied_seq,
            },
        );
        Ok(inventory)
    }

    async fn commit_candidate(
        self: &Arc<Self>,
        candidate: &ProviderInventoryCandidate,
        publish: bool,
    ) -> ProviderResult<()> {
        let _gate = self.commit_gate.read().await;
        if !self.is_current() {
            return Err(ProviderError::Stopped);
        }
        let latest_catalog_revision = self.catalog.read().await.target_revision_seq();
        if candidate.provider_instance_name != self.config.provider_instance_name
            || candidate.generation != self.generation
            || candidate.candidate_seq != self.current_candidate_seq.load(Ordering::Acquire)
            || candidate.catalog_revision_seq != latest_catalog_revision
        {
            return Err(ProviderError::StaleCandidate);
        }
        let inventory = candidate.inventory.clone();
        let snapshot = serde_json::to_value(inventory.as_ref())
            .map_err(|error| ProviderError::Inventory(error.to_string()))?;
        let record = InventoryLkgsRecord::new(
            &inventory.provider_instance_name,
            &inventory.provider_profile_id,
            &inventory.protocol_adapter_id,
            &inventory.provider_model_list_fingerprint,
            inventory.metadata_applied_seq,
            inventory.inventory_revision.clone(),
            inventory.discovered_at_ms,
            snapshot,
            now_ms()?,
        )
        .map_err(|error| ProviderError::Storage(error.to_string()))?;
        self.store.commit(&record).await?;
        *self.inventory.write().await = inventory.clone();
        if publish {
            let executable = Arc::new(ExecutableProviderInstance {
                config: self.config.clone(),
                profile: self.profile.clone(),
                inventory,
                generation: self.generation,
                runtime: self.clone(),
            });
            let current = self.registry.read().await.clone();
            let mut next = current.instances.clone();
            next.insert(self.config.provider_instance_name.clone(), executable);
            *self.registry.write().await = Arc::new(ProviderRegistry { instances: next });
        }
        Ok(())
    }

    fn publish_refresh_event(
        &self,
        trigger: ProviderRefreshTrigger,
        outcome: ProviderRefreshOutcome,
    ) {
        let _ = self.refresh_events.send(ProviderRefreshEvent {
            provider_instance_name: self.config.provider_instance_name.clone(),
            trigger,
            outcome,
        });
    }

    async fn record_success(&self, at_ms: i64, state: ProviderHealthState) -> ProviderResult<()> {
        let _gate = self.commit_gate.read().await;
        if !self.is_current() {
            return Err(ProviderError::Stopped);
        }
        *self.health.write().await = ProviderHealth {
            state,
            consecutive_failures: 0,
            last_success_at_ms: Some(at_ms),
            last_attempt_at_ms: Some(at_ms),
            last_error: None,
        };
        Ok(())
    }

    async fn record_failure(&self, at_ms: i64, error: &ProviderError) {
        let _gate = self.commit_gate.read().await;
        if !self.is_current() {
            return;
        }
        let mut health = self.health.write().await;
        health.state = ProviderHealthState::Degraded;
        health.consecutive_failures = health.consecutive_failures.saturating_add(1);
        health.last_attempt_at_ms = Some(at_ms);
        health.last_error = Some(error.to_string());
    }

    fn is_current(&self) -> bool {
        !self.stopped.load(Ordering::Acquire)
            && self.current_generation.load(Ordering::Acquire) == self.generation
    }

    async fn run(self: Arc<Self>, mut stop_rx: watch::Receiver<bool>) {
        let mut delay = self.profile.refresh.interval;
        loop {
            tokio::select! {
                changed = stop_rx.changed() => {
                    if changed.is_err() || *stop_rx.borrow() {
                        break;
                    }
                }
                _ = tokio::time::sleep(delay) => {
                    match self.refresh_once(false, ProviderRefreshTrigger::Scheduled, true).await {
                        Ok(_) => delay = self.profile.refresh.interval,
                        Err(ProviderError::Stopped) => break,
                        Err(_) => {
                            let failures = self.health.read().await.consecutive_failures;
                            delay = exponential_backoff(&self.profile.refresh, failures);
                        }
                    }
                }
            }
        }
    }

    async fn stop(&self) {
        let _stop = self.stop_lock.lock().await;
        if self.stopped.load(Ordering::Acquire) {
            return;
        }
        {
            let _gate = self.commit_gate.write().await;
            self.stopped.store(true, Ordering::Release);
            self.current_generation.fetch_add(1, Ordering::AcqRel);
            let _ = self.stop_tx.send(true);
        }
        if let Some(task) = self.task.lock().await.take() {
            let _ = task.await;
        }
        let mut health = self.health.write().await;
        health.state = ProviderHealthState::Stopped;
        health.last_error = None;
    }
}

pub(crate) struct ProviderRuntimeManager {
    profiles: BTreeMap<String, Arc<ProviderProfile>>,
    credential_resolver: Arc<dyn CredentialResolver>,
    quota_observers: BTreeMap<String, Arc<dyn ProviderQuotaObserver>>,
    catalog: Arc<RwLock<Arc<CatalogSnapshot>>>,
    codecs: Arc<CodecRegistry>,
    store: Arc<dyn ProviderInventoryStore>,
    runtimes: Mutex<BTreeMap<String, Arc<ProviderRuntime>>>,
    registry: Arc<RwLock<Arc<ProviderRegistry>>>,
    refresh_events: broadcast::Sender<ProviderRefreshEvent>,
    generations: Mutex<BTreeMap<String, Arc<AtomicU64>>>,
    lifecycle_lock: Mutex<()>,
}

impl ProviderRuntimeManager {
    pub(crate) fn new(
        profiles: impl IntoIterator<Item = ProviderProfile>,
        credential_resolver: Arc<dyn CredentialResolver>,
        catalog: Arc<CatalogSnapshot>,
        codecs: Arc<CodecRegistry>,
        store: Arc<dyn ProviderInventoryStore>,
    ) -> ProviderResult<Self> {
        let mut profile_map = BTreeMap::new();
        for profile in profiles {
            profile.validate()?;
            let id = profile.provider_profile_id.clone();
            if profile_map.insert(id.clone(), Arc::new(profile)).is_some() {
                return Err(ProviderError::InvalidConfiguration(format!(
                    "duplicate provider profile `{id}`"
                )));
            }
        }
        let (refresh_events, _) = broadcast::channel(64);
        Ok(Self {
            profiles: profile_map,
            credential_resolver,
            quota_observers: BTreeMap::new(),
            catalog: Arc::new(RwLock::new(catalog)),
            codecs,
            store,
            runtimes: Mutex::new(BTreeMap::new()),
            registry: Arc::new(RwLock::new(Arc::new(ProviderRegistry::default()))),
            refresh_events,
            generations: Mutex::new(BTreeMap::new()),
            lifecycle_lock: Mutex::new(()),
        })
    }

    pub(crate) fn with_quota_observers(
        mut self,
        observers: impl IntoIterator<Item = (String, Arc<dyn ProviderQuotaObserver>)>,
    ) -> ProviderResult<Self> {
        for (provider_profile_id, observer) in observers {
            if !self.profiles.contains_key(&provider_profile_id) {
                return Err(ProviderError::UnknownProfile(provider_profile_id));
            }
            validate_id("quota source", observer.source())?;
            if self
                .quota_observers
                .insert(provider_profile_id.clone(), observer)
                .is_some()
            {
                return Err(ProviderError::InvalidConfiguration(format!(
                    "duplicate quota observer for provider profile `{provider_profile_id}`"
                )));
            }
        }
        Ok(self)
    }

    pub(crate) async fn registry(&self) -> Arc<ProviderRegistry> {
        self.registry.read().await.clone()
    }

    pub(crate) async fn quota_observation(
        &self,
        provider_instance_name: &str,
    ) -> ProviderResult<ProviderQuotaObservation> {
        Ok(self
            .runtime(provider_instance_name)
            .await?
            .quota_observation()
            .await)
    }

    pub(crate) fn subscribe_refresh_events(&self) -> broadcast::Receiver<ProviderRefreshEvent> {
        self.refresh_events.subscribe()
    }

    pub(crate) async fn current_catalog(&self) -> Arc<CatalogSnapshot> {
        self.catalog.read().await.clone()
    }

    pub(crate) async fn validate_draft(
        &self,
        draft: &ProviderDraftConfig,
        connection_contract: &ProviderConnectionContract,
        discovery: &dyn ProviderDiscovery,
        dynamic_login_resolver: Option<&dyn DynamicLoginCredentialResolver>,
    ) -> Result<ProviderDraftNegotiation, ProviderDraftValidationError> {
        let profile = self
            .profiles
            .get(&draft.provider_profile_id)
            .cloned()
            .ok_or_else(|| ProviderDraftValidationError {
                stage: ProviderDraftValidationStage::Protocol,
                kind: ProviderRefreshFailure::UnknownDependency,
            })?;
        if profile.default_protocol_adapter_id != draft.protocol_adapter_id
            && draft.provider_profile_id != "custom"
        {
            return Err(ProviderDraftValidationError {
                stage: ProviderDraftValidationStage::Protocol,
                kind: ProviderRefreshFailure::InvalidConfiguration,
            });
        }
        if self.codecs.adapter(&draft.protocol_adapter_id).is_none() {
            return Err(ProviderDraftValidationError {
                stage: ProviderDraftValidationStage::Protocol,
                kind: ProviderRefreshFailure::UnknownDependency,
            });
        }
        let connection = connection_contract
            .resolve(ProviderConnectionInput {
                base_url: draft.base_url.as_deref(),
                region: draft.region.as_deref(),
                workspace: draft.workspace.as_deref(),
                account: draft.account.as_deref(),
            })
            .map_err(|error| {
                ProviderDraftValidationError::from_provider_error(
                    ProviderDraftValidationStage::Connection,
                    &error,
                )
            })?;
        draft.auth.validate().map_err(|error| {
            ProviderDraftValidationError::from_provider_error(
                ProviderDraftValidationStage::Authentication,
                &error,
            )
        })?;
        let (credential_reference, credential) = match &draft.auth {
            ProviderAuthConfig::ApiKey { credential_ref } => {
                let reference = CredentialReference {
                    reference: credential_ref.clone(),
                };
                let credential = self
                    .credential_resolver
                    .resolve(&profile.credential, &reference)
                    .await
                    .map_err(|error| {
                        ProviderDraftValidationError::from_provider_error(
                            ProviderDraftValidationStage::Authentication,
                            &error,
                        )
                    })?;
                (reference, credential)
            }
            ProviderAuthConfig::DynamicLogin { .. } => {
                let user_name = draft.dynamic_login_user_name.as_deref().ok_or(
                    ProviderDraftValidationError {
                        stage: ProviderDraftValidationStage::Authentication,
                        kind: ProviderRefreshFailure::InvalidConfiguration,
                    },
                )?;
                let context = draft
                    .auth
                    .dynamic_login_context(&draft.provider_instance_name, user_name)
                    .map_err(|error| {
                        ProviderDraftValidationError::from_provider_error(
                            ProviderDraftValidationStage::Authentication,
                            &error,
                        )
                    })?;
                let resolver = dynamic_login_resolver.ok_or(ProviderDraftValidationError {
                    stage: ProviderDraftValidationStage::Authentication,
                    kind: ProviderRefreshFailure::UnknownDependency,
                })?;
                let credential = resolver.resolve_dynamic(&context).await.map_err(|error| {
                    ProviderDraftValidationError::from_provider_error(
                        ProviderDraftValidationStage::Authentication,
                        &error,
                    )
                })?;
                (
                    CredentialReference {
                        reference: "dynamic-login".into(),
                    },
                    credential,
                )
            }
        };
        let instance = ProviderInstanceConfig {
            provider_instance_name: draft.provider_instance_name.clone(),
            provider_profile_id: draft.provider_profile_id.clone(),
            protocol_adapter_id: draft.protocol_adapter_id.clone(),
            base_url: connection.base_url.clone(),
            credential: credential_reference,
            provider_rules_id: draft.provider_rules_id.clone(),
            region: connection.region.clone(),
            workspace: connection.workspace.clone(),
            account: connection.account.clone(),
        };
        instance.validate().map_err(|error| {
            ProviderDraftValidationError::from_provider_error(
                ProviderDraftValidationStage::Connection,
                &error,
            )
        })?;
        let discovered = discovery
            .discover(&DiscoveryContext {
                profile: &profile,
                instance: &instance,
                credential: &credential,
            })
            .await
            .map_err(|error| {
                ProviderDraftValidationError::from_provider_error(
                    ProviderDraftValidationStage::Discovery,
                    &error,
                )
            })?;
        let catalog = self.catalog.read().await.clone();
        let inventory =
            InventoryBuilder::build(&profile, &instance, discovered, &catalog, &self.codecs)
                .map_err(|error| {
                    let stage = if matches!(error, ProviderError::Discovery(_)) {
                        ProviderDraftValidationStage::Discovery
                    } else {
                        ProviderDraftValidationStage::Inventory
                    };
                    ProviderDraftValidationError::from_provider_error(stage, &error)
                })?;
        Ok(ProviderDraftNegotiation {
            provider_profile_id: profile.provider_profile_id.clone(),
            protocol_adapter_id: instance.protocol_adapter_id,
            auth_mode: draft.auth.mode(),
            connection,
            catalog_revision_seq: catalog.target_revision_seq(),
            inventory: Arc::new(inventory),
        })
    }

    pub(crate) async fn start(
        &self,
        config: ProviderInstanceConfig,
        discovery: Arc<dyn ProviderDiscovery>,
    ) -> ProviderResult<Arc<ExecutableProviderInstance>> {
        let _lifecycle = self.lifecycle_lock.lock().await;
        if self
            .runtimes
            .lock()
            .await
            .contains_key(&config.provider_instance_name)
        {
            return Err(ProviderError::DuplicateInstance(
                config.provider_instance_name,
            ));
        }
        self.start_unpublished(config, discovery).await
    }

    async fn start_unpublished(
        &self,
        config: ProviderInstanceConfig,
        discovery: Arc<dyn ProviderDiscovery>,
    ) -> ProviderResult<Arc<ExecutableProviderInstance>> {
        config.validate()?;
        let profile = self
            .profiles
            .get(&config.provider_profile_id)
            .cloned()
            .ok_or_else(|| ProviderError::UnknownProfile(config.provider_profile_id.clone()))?;
        if profile.default_protocol_adapter_id != config.protocol_adapter_id
            && config.provider_profile_id != "custom"
        {
            return Err(ProviderError::InvalidConfiguration(
                "dedicated provider must use its profile adapter".into(),
            ));
        }
        if self.codecs.adapter(&config.protocol_adapter_id).is_none() {
            return Err(ProviderError::UnknownAdapter(config.protocol_adapter_id));
        }
        let config = Arc::new(config);
        let generation_cell = {
            let mut generations = self.generations.lock().await;
            generations
                .entry(config.provider_instance_name.clone())
                .or_insert_with(|| Arc::new(AtomicU64::new(0)))
                .clone()
        };
        let generation = generation_cell.fetch_add(1, Ordering::AcqRel) + 1;
        let (stop_tx, stop_rx) = watch::channel(false);
        let catalog = self.catalog.read().await.clone();
        let placeholder = empty_inventory(&profile, &config, catalog.target_revision_seq());
        let runtime = Arc::new(ProviderRuntime {
            config: config.clone(),
            profile: profile.clone(),
            discovery,
            credential_resolver: self.credential_resolver.clone(),
            quota_observer: self
                .quota_observers
                .get(&profile.provider_profile_id)
                .cloned(),
            catalog: self.catalog.clone(),
            codecs: self.codecs.clone(),
            store: self.store.clone(),
            registry: self.registry.clone(),
            refresh_events: self.refresh_events.clone(),
            generation,
            current_generation: generation_cell,
            current_candidate_seq: AtomicU64::new(0),
            commit_gate: RwLock::new(()),
            refresh_lock: Mutex::new(()),
            inventory: RwLock::new(Arc::new(placeholder)),
            health: RwLock::new(ProviderHealth::default()),
            stopped: AtomicBool::new(false),
            stop_tx,
            task: Mutex::new(None),
            stop_lock: Mutex::new(()),
        });

        let inventory = match runtime
            .refresh_once(true, ProviderRefreshTrigger::Initial, false)
            .await
        {
            Ok(inventory) => inventory,
            Err(discovery_error) => match load_lkgs(&runtime).await {
                Ok(Some(inventory)) => inventory,
                Ok(None) => match profile.default_inventory.clone() {
                    Some(default) => Arc::new(InventoryBuilder::build(
                        &profile,
                        &config,
                        default,
                        &catalog,
                        &self.codecs,
                    )?),
                    None => return Err(discovery_error),
                },
                Err(_) => return Err(discovery_error),
            },
        };
        *runtime.inventory.write().await = inventory.clone();
        let executable = Arc::new(ExecutableProviderInstance {
            config: config.clone(),
            profile,
            inventory,
            generation,
            runtime: runtime.clone(),
        });
        {
            let mut runtimes = self.runtimes.lock().await;
            if runtimes
                .insert(config.provider_instance_name.clone(), runtime.clone())
                .is_some()
            {
                runtime.stop().await;
                return Err(ProviderError::DuplicateInstance(
                    config.provider_instance_name.clone(),
                ));
            }
        }
        self.publish_instance(executable.clone()).await;
        *runtime.task.lock().await = Some(tokio::spawn(runtime.clone().run(stop_rx)));
        Ok(executable)
    }

    pub(crate) async fn refresh(
        &self,
        provider_instance_name: &str,
    ) -> ProviderResult<Arc<ProviderInventorySnapshot>> {
        let runtime = self
            .runtimes
            .lock()
            .await
            .get(provider_instance_name)
            .cloned()
            .ok_or_else(|| ProviderError::UnknownInstance(provider_instance_name.into()))?;
        runtime
            .refresh_once(true, ProviderRefreshTrigger::Manual, true)
            .await
    }

    pub(crate) async fn build_inventory_candidate(
        &self,
        provider_instance_name: &str,
    ) -> ProviderResult<ProviderInventoryCandidate> {
        let runtime = self.runtime(provider_instance_name).await?;
        let _refresh = runtime.refresh_lock.lock().await;
        runtime.build_candidate().await
    }

    pub(crate) async fn commit_inventory_candidate(
        &self,
        candidate: ProviderInventoryCandidate,
        trigger: ProviderRefreshTrigger,
    ) -> ProviderResult<Arc<ProviderInventorySnapshot>> {
        let runtime = self.runtime(&candidate.provider_instance_name).await?;
        let attempt_at_ms = now_ms()?;
        match runtime.commit_candidate(&candidate, true).await {
            Ok(()) => {
                runtime
                    .record_success(attempt_at_ms, candidate.inventory.health)
                    .await?;
                runtime.publish_refresh_event(
                    trigger,
                    ProviderRefreshOutcome::Committed {
                        changed: true,
                        inventory_revision: candidate.inventory.inventory_revision.clone(),
                        metadata_applied_seq: candidate.inventory.metadata_applied_seq,
                    },
                );
                Ok(candidate.inventory)
            }
            Err(error) => {
                if !matches!(error, ProviderError::StaleCandidate) {
                    runtime.record_failure(attempt_at_ms, &error).await;
                }
                runtime.publish_refresh_event(
                    trigger,
                    ProviderRefreshOutcome::Failed {
                        kind: ProviderRefreshFailure::from_error(&error),
                    },
                );
                Err(error)
            }
        }
    }

    pub(crate) async fn reconcile_inventory(
        &self,
        catalog: Arc<CatalogSnapshot>,
    ) -> Vec<ProviderRefreshEvent> {
        *self.catalog.write().await = catalog;
        let runtimes: Vec<_> = self.runtimes.lock().await.values().cloned().collect();
        let mut results = Vec::with_capacity(runtimes.len());
        for runtime in runtimes {
            let outcome = match runtime
                .refresh_once(true, ProviderRefreshTrigger::Reconciliation, true)
                .await
            {
                Ok(inventory) => ProviderRefreshOutcome::Committed {
                    changed: true,
                    inventory_revision: inventory.inventory_revision.clone(),
                    metadata_applied_seq: inventory.metadata_applied_seq,
                },
                Err(error) => ProviderRefreshOutcome::Failed {
                    kind: ProviderRefreshFailure::from_error(&error),
                },
            };
            results.push(ProviderRefreshEvent {
                provider_instance_name: runtime.config.provider_instance_name.clone(),
                trigger: ProviderRefreshTrigger::Reconciliation,
                outcome,
            });
        }
        results
    }

    pub(crate) async fn replace(
        &self,
        config: ProviderInstanceConfig,
        discovery: Arc<dyn ProviderDiscovery>,
    ) -> ProviderResult<Arc<ExecutableProviderInstance>> {
        let _lifecycle = self.lifecycle_lock.lock().await;
        let name = config.provider_instance_name.clone();
        self.stop_and_remove_unlocked(&name).await?;
        self.start_unpublished(config, discovery).await
    }

    pub(crate) async fn stop_and_remove(&self, name: &str) -> ProviderResult<()> {
        let _lifecycle = self.lifecycle_lock.lock().await;
        self.stop_and_remove_unlocked(name).await
    }

    async fn stop_and_remove_unlocked(&self, name: &str) -> ProviderResult<()> {
        let runtime = self.runtimes.lock().await.remove(name);
        let Some(runtime) = runtime else {
            return Ok(());
        };
        runtime.stop().await;
        let current = self.registry.read().await.clone();
        let mut next = current.instances.clone();
        next.remove(name);
        *self.registry.write().await = Arc::new(ProviderRegistry { instances: next });
        Ok(())
    }

    pub(crate) async fn shutdown(&self) {
        let _lifecycle = self.lifecycle_lock.lock().await;
        let runtimes = {
            let mut runtimes = self.runtimes.lock().await;
            std::mem::take(&mut *runtimes)
        };
        for runtime in runtimes.values() {
            runtime.stop().await;
        }
        *self.registry.write().await = Arc::new(ProviderRegistry::default());
    }

    async fn publish_instance(&self, instance: Arc<ExecutableProviderInstance>) {
        let current = self.registry.read().await.clone();
        let mut next = current.instances.clone();
        next.insert(instance.config.provider_instance_name.clone(), instance);
        *self.registry.write().await = Arc::new(ProviderRegistry { instances: next });
    }

    async fn runtime(&self, name: &str) -> ProviderResult<Arc<ProviderRuntime>> {
        self.runtimes
            .lock()
            .await
            .get(name)
            .cloned()
            .ok_or_else(|| ProviderError::UnknownInstance(name.into()))
    }
}

async fn load_lkgs(
    runtime: &ProviderRuntime,
) -> ProviderResult<Option<Arc<ProviderInventorySnapshot>>> {
    let Some(record) = runtime
        .store
        .load(&runtime.config.provider_instance_name)
        .await?
    else {
        return Ok(None);
    };
    if record.provider_profile_id != runtime.profile.provider_profile_id
        || record.protocol_adapter_id != runtime.config.protocol_adapter_id
    {
        return Ok(None);
    }
    let inventory: ProviderInventorySnapshot = serde_json::from_value(record.snapshot.clone())
        .map_err(|error| ProviderError::Inventory(error.to_string()))?;
    validate_inventory_identity(&inventory, &runtime.profile, &runtime.config)?;
    if inventory.provider_model_list_fingerprint != record.provider_model_list_fingerprint
        || inventory.metadata_applied_seq != record.metadata_applied_seq
        || inventory.inventory_revision != record.inventory_revision
        || inventory.discovered_at_ms != record.discovered_at_ms
    {
        return Err(ProviderError::Inventory(
            "LKGS row columns do not match its inventory snapshot".into(),
        ));
    }
    let mut model_ids = BTreeSet::new();
    for model in &inventory.models {
        if !model_ids.insert(&model.provider_model_id) {
            return Err(ProviderError::Inventory(
                "LKGS contains duplicate provider model IDs".into(),
            ));
        }
        for api_type in &model.api_types {
            let name = api_type_name(*api_type)?;
            let operation = model.operations.get(&name).ok_or_else(|| {
                ProviderError::Inventory("LKGS model is missing an operation binding".into())
            })?;
            runtime
                .codecs
                .operation_descriptor(&inventory.protocol_adapter_id, operation, *api_type)
                .map_err(|error| ProviderError::Inventory(error.to_string()))?;
        }
    }
    Ok(Some(Arc::new(inventory)))
}

fn validate_inventory_identity(
    inventory: &ProviderInventorySnapshot,
    profile: &ProviderProfile,
    instance: &ProviderInstanceConfig,
) -> ProviderResult<()> {
    if inventory.schema_version != INVENTORY_SCHEMA_VERSION
        || inventory.provider_instance_name != instance.provider_instance_name
        || inventory.provider_profile_id != profile.provider_profile_id
        || inventory.protocol_adapter_id != instance.protocol_adapter_id
        || inventory.provider_model_list_fingerprint.trim().is_empty()
    {
        return Err(ProviderError::Inventory(
            "LKGS identity or schema does not match the provider instance".into(),
        ));
    }
    Ok(())
}

fn empty_inventory(
    profile: &ProviderProfile,
    instance: &ProviderInstanceConfig,
    metadata_applied_seq: u64,
) -> ProviderInventorySnapshot {
    ProviderInventorySnapshot {
        schema_version: INVENTORY_SCHEMA_VERSION,
        provider_instance_name: instance.provider_instance_name.clone(),
        provider_profile_id: profile.provider_profile_id.clone(),
        protocol_adapter_id: instance.protocol_adapter_id.clone(),
        provider_model_list_fingerprint: "pending".into(),
        metadata_applied_seq,
        inventory_revision: None,
        discovered_at_ms: 0,
        health: ProviderHealthState::Unknown,
        models: Vec::new(),
    }
}

fn validate_discovery(discovery: &ProviderDiscoverySnapshot) -> ProviderResult<()> {
    if discovery.discovered_at_ms < 0 {
        return Err(ProviderError::Discovery(
            "discovery timestamp must not be negative".into(),
        ));
    }
    let mut ids = BTreeSet::new();
    for model in &discovery.models {
        if model.provider_model_id.trim().is_empty() || model.provider_model_id.contains('@') {
            return Err(ProviderError::Discovery(
                "provider model IDs must be non-empty and must not contain `@`".into(),
            ));
        }
        if !ids.insert(&model.provider_model_id) {
            return Err(ProviderError::Discovery(format!(
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
    if pricing.currency.trim().is_empty()
        || [
            pricing.input_token,
            pricing.output_token,
            pricing.cache_input_token,
            pricing.estimated_cost,
            pricing.amount,
        ]
        .into_iter()
        .flatten()
        .any(|value| !value.is_finite() || value < 0.0)
        || pricing
            .rules
            .iter()
            .any(|rule| !rule.amount.is_finite() || rule.amount < 0.0)
    {
        return Err(ProviderError::Discovery(
            "discovery pricing must be finite, non-negative, and have a currency".into(),
        ));
    }
    Ok(())
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
mod tests {
    use super::*;
    use crate::catalog::{
        CatalogBuildOptions, CatalogDocuments, ModelDriverCatalog, ProviderRulesCatalog,
    };
    use crate::model::{LogicalModelDefinition, ModelRegistry, MountMode, RegistryLayers};
    use crate::protocol::{
        AdapterDescriptor, AdapterStatus, CodecCall, ExecutionMode, HttpRequest, HttpResponse,
        OperationBinding, OperationCodec, OperationDescriptor, ProtocolError, ProtocolExecution,
        ProtocolResultValue,
    };
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Notify;

    struct FakeCodec {
        descriptor: OperationDescriptor,
    }

    #[test]
    fn provider_auth_modes_are_structurally_exclusive() {
        let api_key: ProviderAuthConfig = serde_json::from_value(serde_json::json!({
            "mode": "api_key",
            "credential_ref": "system-config://secrets/aicc/sn-main"
        }))
        .unwrap();
        assert_eq!(api_key.mode(), ProviderAuthMode::ApiKey);
        assert_eq!(
            api_key.credential_reference().unwrap().reference,
            "system-config://secrets/aicc/sn-main"
        );
        assert!(api_key.validate().is_ok());

        let dynamic: ProviderAuthConfig = serde_json::from_value(serde_json::json!({
            "mode": "dynamic_login",
            "login_profile": "device_jwt",
            "login_endpoint": "https://sn.example/api/user/login_by_device_token"
        }))
        .unwrap();
        assert_eq!(dynamic.mode(), ProviderAuthMode::DynamicLogin);
        assert!(dynamic.credential_reference().is_none());
        let context = dynamic.dynamic_login_context("sn-main", "alice").unwrap();
        assert_eq!(context.cache_key(), "sn-main");
        assert_eq!(context.user_name, "alice");

        assert!(
            serde_json::from_value::<ProviderAuthConfig>(serde_json::json!({
                "mode": "api_key",
                "credential_ref": "secret-ref",
                "login_profile": "device_jwt",
                "login_endpoint": "https://sn.example/login"
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<ProviderAuthConfig>(serde_json::json!({
                "mode": "dynamic_login",
                "credential_ref": "secret-ref",
                "login_profile": "device_jwt",
                "login_endpoint": "https://sn.example/login"
            }))
            .is_err()
        );
    }

    #[test]
    fn provider_connection_resolves_workspace_and_default_base_url() {
        let contract = ProviderConnectionContract {
            default_base_url: "https://{workspace}.{region}.maas.example/compatible-mode/v1".into(),
            region: ProviderFieldSchema::optional_with_default("cn-beijing")
                .with_allowed_values(["cn-beijing", "cn-shanghai"]),
            workspace: ProviderFieldSchema::required(),
            account: ProviderFieldSchema::optional(),
        };
        let resolved = contract
            .resolve(ProviderConnectionInput {
                workspace: Some("workspace-1"),
                account: Some("account_1"),
                ..ProviderConnectionInput::default()
            })
            .unwrap();
        assert_eq!(
            resolved.base_url,
            "https://workspace-1.cn-beijing.maas.example/compatible-mode/v1"
        );
        assert_eq!(resolved.region.as_deref(), Some("cn-beijing"));
        assert_eq!(resolved.workspace.as_deref(), Some("workspace-1"));
        assert_eq!(resolved.account.as_deref(), Some("account_1"));

        let overridden = contract
            .resolve(ProviderConnectionInput {
                base_url: Some("https://gateway.example/v1"),
                region: Some("cn-shanghai"),
                workspace: Some("workspace-1"),
                ..ProviderConnectionInput::default()
            })
            .unwrap();
        assert_eq!(overridden.base_url, "https://gateway.example/v1");
        assert_eq!(overridden.region.as_deref(), Some("cn-shanghai"));
    }

    #[test]
    fn provider_connection_rejects_missing_or_unsupported_fields() {
        let contract = ProviderConnectionContract {
            default_base_url: "https://{workspace}.example/v1".into(),
            region: ProviderFieldSchema::unsupported(),
            workspace: ProviderFieldSchema::required(),
            account: ProviderFieldSchema::unsupported(),
        };
        assert!(matches!(
            contract.resolve(ProviderConnectionInput::default()),
            Err(ProviderError::InvalidConfiguration(_))
        ));
        assert!(matches!(
            contract.resolve(ProviderConnectionInput {
                region: Some("global"),
                workspace: Some("workspace-1"),
                ..ProviderConnectionInput::default()
            }),
            Err(ProviderError::InvalidConfiguration(_))
        ));
        assert!(matches!(
            contract.resolve(ProviderConnectionInput {
                workspace: Some("unsafe/workspace"),
                ..ProviderConnectionInput::default()
            }),
            Err(ProviderError::InvalidConfiguration(_))
        ));
    }

    #[async_trait]
    impl OperationCodec for FakeCodec {
        fn descriptor(&self) -> &OperationDescriptor {
            &self.descriptor
        }

        fn api_type(&self) -> ApiType {
            ApiType::Llm
        }

        fn execution_modes(&self) -> BTreeSet<ExecutionMode> {
            BTreeSet::from([ExecutionMode::Immediate])
        }

        fn encode(&self, _call: &CodecCall<'_>) -> ProtocolResultValue<HttpRequest> {
            Err(ProtocolError::invalid_request("not used by provider tests"))
        }

        async fn decode(&self, _response: HttpResponse) -> ProtocolResultValue<ProtocolExecution> {
            Err(ProtocolError::invalid_response(
                "not used by provider tests",
            ))
        }
    }

    #[derive(Default)]
    struct MemoryStore {
        records: Mutex<BTreeMap<String, InventoryLkgsRecord>>,
        commits: AtomicUsize,
    }

    #[async_trait]
    impl ProviderInventoryStore for MemoryStore {
        async fn load(
            &self,
            provider_instance_name: &str,
        ) -> ProviderResult<Option<InventoryLkgsRecord>> {
            Ok(self
                .records
                .lock()
                .await
                .get(provider_instance_name)
                .cloned())
        }

        async fn commit(&self, record: &InventoryLkgsRecord) -> ProviderResult<()> {
            self.commits.fetch_add(1, Ordering::SeqCst);
            self.records
                .lock()
                .await
                .insert(record.provider_instance_name.clone(), record.clone());
            Ok(())
        }
    }

    struct ScriptedDiscovery {
        results: Mutex<VecDeque<Result<ProviderDiscoverySnapshot, String>>>,
        fallback: ProviderDiscoverySnapshot,
        calls: AtomicUsize,
    }

    impl ScriptedDiscovery {
        fn new(
            results: impl IntoIterator<Item = Result<ProviderDiscoverySnapshot, String>>,
            fallback: ProviderDiscoverySnapshot,
        ) -> Self {
            Self {
                results: Mutex::new(results.into_iter().collect()),
                fallback,
                calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl ProviderDiscovery for ScriptedDiscovery {
        async fn discover(
            &self,
            _context: &DiscoveryContext<'_>,
        ) -> ProviderResult<ProviderDiscoverySnapshot> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match self.results.lock().await.pop_front() {
                Some(Ok(snapshot)) => Ok(snapshot),
                Some(Err(error)) => Err(ProviderError::Discovery(error)),
                None => Ok(self.fallback.clone()),
            }
        }
    }

    struct WorkspaceRecordingDiscovery {
        snapshot: ProviderDiscoverySnapshot,
        workspaces: Mutex<Vec<Option<String>>>,
    }

    #[async_trait]
    impl ProviderDiscovery for WorkspaceRecordingDiscovery {
        async fn discover(
            &self,
            context: &DiscoveryContext<'_>,
        ) -> ProviderResult<ProviderDiscoverySnapshot> {
            self.workspaces
                .lock()
                .await
                .push(context.instance.workspace.clone());
            Ok(self.snapshot.clone())
        }
    }

    struct FakeQuotaObserver {
        fail: AtomicBool,
        calls: AtomicUsize,
    }

    #[async_trait]
    impl ProviderQuotaObserver for FakeQuotaObserver {
        fn source(&self) -> &'static str {
            "provider_api"
        }

        async fn observe(
            &self,
            context: &ProviderQuotaContext<'_>,
        ) -> ProviderResult<ProviderQuotaReading> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(context.profile.provider_profile_id, "openai");
            assert_eq!(context.instance.provider_instance_name, "primary");
            assert!(!format!("{:?}", context.credential).contains("test-secret"));
            if self.fail.load(Ordering::SeqCst) {
                return Err(ProviderError::Discovery(
                    "quota endpoint leaked-private-detail".into(),
                ));
            }
            Ok(ProviderQuotaReading {
                state: ProviderQuotaLevel::NearLimit,
                remaining_request_units: Some(12),
                remaining_cost_usd: Some(AiCost {
                    amount: 3.5,
                    currency: "USD".into(),
                }),
                reset_at_ms: Some(4_000_000_000_000),
            })
        }
    }

    struct BlockingDiscovery {
        snapshot: ProviderDiscoverySnapshot,
        calls: AtomicUsize,
        started: Notify,
        release: Notify,
    }

    #[async_trait]
    impl ProviderDiscovery for BlockingDiscovery {
        async fn discover(
            &self,
            _context: &DiscoveryContext<'_>,
        ) -> ProviderResult<ProviderDiscoverySnapshot> {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                return Ok(self.snapshot.clone());
            }
            self.started.notify_one();
            self.release.notified().await;
            Ok(self.snapshot.clone())
        }
    }

    fn profile() -> ProviderProfile {
        ProviderProfile {
            provider_profile_id: "openai".into(),
            display_name: "OpenAI".into(),
            default_protocol_adapter_id: "openai-responses".into(),
            credential: CredentialDescriptor {
                kind: CredentialKind::Bearer,
                header_name: None,
            },
            discovery_mode: DiscoveryMode::MachineApi,
            refresh: RefreshPolicy {
                interval: Duration::from_secs(3_600),
                initial_backoff: Duration::from_millis(10),
                max_backoff: Duration::from_millis(40),
            },
            default_inventory: None,
        }
    }

    fn instance(name: &str) -> ProviderInstanceConfig {
        ProviderInstanceConfig {
            provider_instance_name: name.into(),
            provider_profile_id: "openai".into(),
            protocol_adapter_id: "openai-responses".into(),
            base_url: "https://api.example.test/v1/".into(),
            credential: CredentialReference {
                reference: "system-config://secrets/aicc/openai".into(),
            },
            provider_rules_id: None,
            region: None,
            workspace: None,
            account: None,
        }
    }

    fn discovery(model_id: &str) -> ProviderDiscoverySnapshot {
        ProviderDiscoverySnapshot {
            revision: Some("remote-1".into()),
            discovered_at_ms: 100,
            health: ProviderHealthState::Healthy,
            models: vec![DiscoveredModel {
                provider_model_id: model_id.into(),
                origin_model_id: None,
                api_types: Some(vec![ApiType::Llm, ApiType::EmbeddingText]),
                supported_features: Some(BTreeSet::from([
                    buckyos_api::features::TOOL_CALL.into(),
                    buckyos_api::features::JSON_SCHEMA.into(),
                ])),
                remote_methods: Some(BTreeSet::from(["responses.create".into()])),
                availability: ModelAvailability::Available,
                deprecated: false,
                pricing: Some(Pricing {
                    currency: "USD".into(),
                    input_token: Some(0.5),
                    output_token: None,
                    cache_input_token: None,
                    estimated_cost: None,
                    unit: None,
                    amount: None,
                    rules: vec![],
                }),
            }],
        }
    }

    fn catalog() -> Arc<CatalogSnapshot> {
        catalog_with_revision(7, 8192)
    }

    fn catalog_with_revision(revision_seq: u64, context_tokens: u64) -> Arc<CatalogSnapshot> {
        let model_driver: ModelDriverCatalog = serde_json::from_value(serde_json::json!({
            "format": "buckyos.aicc.model-driver-catalog",
            "schema_version": 1,
            "schema_revision": 0,
            "model_driver_id": "openai",
            "revision_seq": revision_seq,
            "models": [{
                "id": "gpt-test",
                "api_types": ["llm", "embedding.text"],
                "logical_mounts": ["llm.test"],
                "capabilities": {
                    "tool_call": true,
                    "json_schema": true,
                    "context_tokens": context_tokens
                },
                "pricing": {"currency": "USD", "input_token": 9.0}
            }],
            "patterns": [],
            "defaults": {},
            "variants": [],
            "version_rules": []
        }))
        .unwrap();
        let provider_rules: ProviderRulesCatalog = serde_json::from_value(serde_json::json!({
            "format": "buckyos.aicc.provider-rules-catalog",
            "schema_version": 1,
            "schema_revision": 0,
            "revision_seq": revision_seq,
            "provider_profile_id": "openai",
            "metadata_drivers": ["openai"],
            "models": [{
                "id": "gpt-test",
                "operations": {"llm": "responses.create"},
                "pricing": {"currency": "USD", "input_token": 2.0}
            }],
            "patterns": [],
            "variants": []
        }))
        .unwrap();
        Arc::new(
            CatalogSnapshot::build(
                revision_seq,
                CatalogDocuments {
                    model_drivers: vec![model_driver],
                    provider_rules: vec![provider_rules],
                    known_providers: vec![],
                },
                &CatalogBuildOptions::default(),
            )
            .unwrap(),
        )
    }

    fn routed_catalog() -> Arc<CatalogSnapshot> {
        let driver = |model_driver_id: &str| -> ModelDriverCatalog {
            serde_json::from_value(serde_json::json!({
                "format": "buckyos.aicc.model-driver-catalog",
                "schema_version": 1,
                "schema_revision": 0,
                "model_driver_id": model_driver_id,
                "revision_seq": 7,
                "models": [{
                    "id": "shared-model",
                    "api_types": ["llm"],
                    "capabilities": {"tool_call": true}
                }],
                "patterns": [],
                "defaults": {},
                "variants": [],
                "version_rules": []
            }))
            .unwrap()
        };
        let provider_rules: ProviderRulesCatalog = serde_json::from_value(serde_json::json!({
            "format": "buckyos.aicc.provider-rules-catalog",
            "schema_version": 1,
            "schema_revision": 0,
            "revision_seq": 7,
            "provider_profile_id": "openrouter",
            "metadata_drivers": ["openai", "claude"],
            "origin_provider_aliases": {
                "openai": "openai",
                "anthropic": "claude"
            },
            "origin_mappings": [{
                "extract": {
                    "source": "provider_model_id",
                    "regex": "^(?<driver>[^/]+)/(?<model>.+)$"
                },
                "transforms": {
                    "driver": [
                        {"op": "lowercase"},
                        {
                            "op": "alias",
                            "table": "origin_provider_aliases"
                        }
                    ],
                    "model": [{"op": "trim"}]
                }
            }],
            "models": [],
            "patterns": [{
                "match": "*",
                "operations": {"llm": "responses.create"}
            }],
            "variants": []
        }))
        .unwrap();
        Arc::new(
            CatalogSnapshot::build(
                7,
                CatalogDocuments {
                    model_drivers: vec![driver("openai"), driver("claude")],
                    provider_rules: vec![provider_rules],
                    known_providers: vec![],
                },
                &CatalogBuildOptions::default(),
            )
            .unwrap(),
        )
    }

    fn codecs() -> Arc<CodecRegistry> {
        let descriptor = OperationDescriptor {
            operation_id: "responses.create".into(),
            bindings: vec![OperationBinding {
                api_type: ApiType::Llm,
                capability: ApiType::Llm.capability(),
                supported_features: BTreeSet::from([
                    buckyos_api::features::TOOL_CALL.into(),
                    buckyos_api::features::JSON_SCHEMA.into(),
                ]),
                execution_modes: BTreeSet::from([ExecutionMode::Immediate]),
            }],
            supports_cancel: false,
            supports_webhook: false,
            max_request_bytes: 1024,
            max_response_bytes: 1024,
        };
        let adapter = AdapterDescriptor {
            protocol_family_id: "openai".into(),
            protocol_adapter_id: "openai-responses".into(),
            interface_generation: "responses-v1".into(),
            base_adapter_id: None,
            status: AdapterStatus::Stable,
            operations: BTreeMap::from([("responses.create".into(), descriptor.clone())]),
        };
        let mut registry = CodecRegistry::default();
        registry
            .register(adapter, vec![Arc::new(FakeCodec { descriptor })])
            .unwrap();
        Arc::new(registry)
    }

    fn resolver(secret: &str) -> Arc<StaticCredentialResolver> {
        Arc::new(StaticCredentialResolver::new(BTreeMap::from([(
            "system-config://secrets/aicc/openai".into(),
            secret.into(),
        )])))
    }

    fn manager(
        store: Arc<MemoryStore>,
        discovery: Arc<dyn ProviderDiscovery>,
    ) -> (Arc<ProviderRuntimeManager>, Arc<dyn ProviderDiscovery>) {
        let manager = Arc::new(
            ProviderRuntimeManager::new(
                [profile()],
                resolver("test-secret"),
                catalog(),
                codecs(),
                store,
            )
            .unwrap(),
        );
        (manager, discovery)
    }

    fn connection_contract() -> ProviderConnectionContract {
        ProviderConnectionContract {
            default_base_url: "https://{workspace}.example.test/v1/".into(),
            region: ProviderFieldSchema::unsupported(),
            workspace: ProviderFieldSchema::required(),
            account: ProviderFieldSchema::optional(),
        }
    }

    fn draft(auth: ProviderAuthConfig) -> ProviderDraftConfig {
        ProviderDraftConfig {
            provider_instance_name: "draft-provider".into(),
            provider_profile_id: "openai".into(),
            protocol_adapter_id: "openai-responses".into(),
            provider_rules_id: None,
            base_url: None,
            region: None,
            workspace: Some("workspace-1".into()),
            account: None,
            auth,
            dynamic_login_user_name: None,
        }
    }

    #[derive(Default)]
    struct FakeDynamicCredentialResolver {
        calls: AtomicUsize,
        contexts: Mutex<Vec<DynamicLoginContext>>,
    }

    #[async_trait]
    impl DynamicLoginCredentialResolver for FakeDynamicCredentialResolver {
        async fn resolve_dynamic(
            &self,
            context: &DynamicLoginContext,
        ) -> ProviderResult<ResolvedCredential> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.contexts.lock().await.push(context.clone());
            ResolvedCredential::bearer("dynamic-login", "draft-secret")
                .map_err(|error| ProviderError::Credential(error.to_string()))
        }
    }

    #[test]
    fn inventory_intersects_capabilities_and_uses_dynamic_pricing() {
        let inventory = InventoryBuilder::build(
            &profile(),
            &instance("primary"),
            discovery("gpt-test"),
            &catalog(),
            &codecs(),
        )
        .unwrap();
        assert_eq!(inventory.metadata_applied_seq, 7);
        assert_eq!(inventory.models.len(), 1);
        let model = &inventory.models[0];
        assert_eq!(model.api_types, vec![ApiType::Llm]);
        assert_eq!(model.operations["llm"], "responses.create");
        assert_eq!(model.capabilities["tool_call"], Value::Bool(true));
        assert_eq!(model.capabilities["json_schema"], Value::Bool(true));
        assert_eq!(model.capabilities["context_tokens"], Value::from(8192));
        assert_eq!(
            model.pricing.as_ref().unwrap().source,
            PricingSource::Discovery
        );
        assert_eq!(model.provider_rules_revision, Some(7));
        assert!(!inventory.provider_model_list_fingerprint.is_empty());
    }

    #[tokio::test]
    async fn draft_validation_negotiates_without_runtime_or_storage_side_effects() {
        let store = Arc::new(MemoryStore::default());
        let discovery = Arc::new(WorkspaceRecordingDiscovery {
            snapshot: discovery("gpt-test"),
            workspaces: Mutex::new(Vec::new()),
        });
        let (manager, _) = manager(store.clone(), discovery.clone());
        let draft = draft(ProviderAuthConfig::ApiKey {
            credential_ref: "system-config://secrets/aicc/openai".into(),
        });

        let negotiated = manager
            .validate_draft(&draft, &connection_contract(), discovery.as_ref(), None)
            .await
            .unwrap();

        assert_eq!(negotiated.provider_profile_id, "openai");
        assert_eq!(negotiated.protocol_adapter_id, "openai-responses");
        assert_eq!(negotiated.auth_mode, ProviderAuthMode::ApiKey);
        assert_eq!(
            negotiated.connection.workspace.as_deref(),
            Some("workspace-1")
        );
        assert_eq!(
            negotiated.connection.base_url,
            "https://workspace-1.example.test/v1/"
        );
        assert_eq!(negotiated.catalog_revision_seq, 7);
        assert_eq!(negotiated.inventory.models.len(), 1);
        assert_eq!(store.commits.load(Ordering::SeqCst), 0);
        assert!(store.records.lock().await.is_empty());
        assert!(manager.runtimes.lock().await.is_empty());
        assert!(manager.registry().await.list().is_empty());
        tokio::task::yield_now().await;
        assert_eq!(
            discovery.workspaces.lock().await.as_slice(),
            &[Some("workspace-1".into())]
        );
    }

    #[tokio::test]
    async fn workspace_survives_inventory_build_and_instance_replace() {
        let store = Arc::new(MemoryStore::default());
        let initial_discovery = Arc::new(WorkspaceRecordingDiscovery {
            snapshot: discovery("gpt-test"),
            workspaces: Mutex::new(Vec::new()),
        });
        let (manager, _) = manager(store, initial_discovery.clone());
        let mut initial = instance("primary");
        initial.workspace = Some("workspace-initial".into());
        manager
            .start(initial, initial_discovery.clone())
            .await
            .unwrap();
        assert_eq!(
            manager
                .registry()
                .await
                .get("primary")
                .unwrap()
                .config
                .workspace
                .as_deref(),
            Some("workspace-initial")
        );

        let replacement_discovery = Arc::new(WorkspaceRecordingDiscovery {
            snapshot: discovery("gpt-test"),
            workspaces: Mutex::new(Vec::new()),
        });
        let mut replacement = instance("primary");
        replacement.workspace = Some("workspace-reloaded".into());
        manager
            .replace(replacement, replacement_discovery.clone())
            .await
            .unwrap();
        manager.build_inventory_candidate("primary").await.unwrap();

        assert_eq!(
            replacement_discovery.workspaces.lock().await.as_slice(),
            &[
                Some("workspace-reloaded".into()),
                Some("workspace-reloaded".into())
            ]
        );
        assert_eq!(
            manager
                .registry()
                .await
                .get("primary")
                .unwrap()
                .config
                .workspace
                .as_deref(),
            Some("workspace-reloaded")
        );
        manager.shutdown().await;
    }

    #[tokio::test]
    async fn quota_view_uses_only_registered_truth_and_distinguishes_query_failure() {
        let store = Arc::new(MemoryStore::default());
        let mut untrusted_discovery = discovery("gpt-test");
        untrusted_discovery.models[0]
            .supported_features
            .get_or_insert_default()
            .insert("remaining_request_units=999999".into());
        let untrusted_discovery_source = Arc::new(ScriptedDiscovery::new([], untrusted_discovery));
        let (unsupported_manager, unsupported_discovery) =
            manager(store, untrusted_discovery_source);
        unsupported_manager
            .start(instance("primary"), unsupported_discovery)
            .await
            .unwrap();
        let unsupported = unsupported_manager
            .quota_observation("primary")
            .await
            .unwrap();
        assert_eq!(
            unsupported.state,
            ProviderQuotaObservationState::Unsupported
        );
        assert_eq!(unsupported.remaining_request_units, None);
        assert_eq!(unsupported.remaining_cost_usd, None);
        assert_eq!(unsupported.source, "unsupported");
        unsupported_manager.shutdown().await;

        let observer = Arc::new(FakeQuotaObserver {
            fail: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
        });
        let discovery = Arc::new(ScriptedDiscovery::new([], discovery("gpt-test")));
        let manager = ProviderRuntimeManager::new(
            [profile()],
            resolver("test-secret"),
            catalog(),
            codecs(),
            Arc::new(MemoryStore::default()),
        )
        .unwrap()
        .with_quota_observers([(
            "openai".into(),
            observer.clone() as Arc<dyn ProviderQuotaObserver>,
        )])
        .unwrap();
        manager.start(instance("primary"), discovery).await.unwrap();

        let observed = manager.quota_observation("primary").await.unwrap();
        assert_eq!(observed.state, ProviderQuotaObservationState::NearLimit);
        assert_eq!(observed.remaining_request_units, Some(12));
        assert_eq!(
            observed.remaining_cost_usd,
            Some(AiCost {
                amount: 3.5,
                currency: "USD".into()
            })
        );
        assert_eq!(observed.reset_at_ms, Some(4_000_000_000_000));
        assert!(observed.observed_at_ms > 0);
        assert_eq!(observed.source, "provider_api");

        observer.fail.store(true, Ordering::SeqCst);
        let failed = manager.quota_observation("primary").await.unwrap();
        assert_eq!(failed.state, ProviderQuotaObservationState::QueryFailed);
        assert_eq!(failed.remaining_request_units, None);
        assert_eq!(failed.remaining_cost_usd, None);
        assert_eq!(failed.reset_at_ms, None);
        assert_eq!(failed.source, "provider_api");
        assert!(!format!("{failed:?}").contains("leaked-private-detail"));
        assert_eq!(observer.calls.load(Ordering::SeqCst), 2);
        manager.shutdown().await;
    }

    #[tokio::test]
    async fn draft_validation_resolves_dynamic_login_without_exposing_token() {
        let store = Arc::new(MemoryStore::default());
        let discovery = Arc::new(ScriptedDiscovery::new([], discovery("gpt-test")));
        let (manager, _) = manager(store.clone(), discovery.clone());
        let resolver = FakeDynamicCredentialResolver::default();
        let mut draft = draft(ProviderAuthConfig::DynamicLogin {
            login_profile: "device_jwt".into(),
            login_endpoint: "https://sn.example.test/login".into(),
        });
        draft.dynamic_login_user_name = Some("alice".into());

        let negotiated = manager
            .validate_draft(
                &draft,
                &connection_contract(),
                discovery.as_ref(),
                Some(&resolver),
            )
            .await
            .unwrap();

        assert_eq!(negotiated.auth_mode, ProviderAuthMode::DynamicLogin);
        assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
        assert_eq!(resolver.contexts.lock().await[0].user_name, "alice");
        assert!(!format!("{negotiated:?}").contains("draft-secret"));
        assert!(!serde_json::to_string(negotiated.inventory.as_ref())
            .unwrap()
            .contains("draft-secret"));
        assert_eq!(store.commits.load(Ordering::SeqCst), 0);
        assert!(manager.runtimes.lock().await.is_empty());
    }

    #[tokio::test]
    async fn draft_validation_classifies_connection_auth_discovery_and_adapter_failures() {
        let store = Arc::new(MemoryStore::default());
        let discovery_impl = Arc::new(ScriptedDiscovery::new([], discovery("gpt-test")));
        let (manager, _) = manager(store.clone(), discovery_impl.clone());

        let mut invalid_connection = draft(ProviderAuthConfig::ApiKey {
            credential_ref: "system-config://secrets/aicc/openai".into(),
        });
        invalid_connection.workspace = None;
        let error = manager
            .validate_draft(
                &invalid_connection,
                &connection_contract(),
                discovery_impl.as_ref(),
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(error.stage, ProviderDraftValidationStage::Connection);

        let invalid_auth = draft(ProviderAuthConfig::ApiKey {
            credential_ref: "missing-secret".into(),
        });
        let error = manager
            .validate_draft(
                &invalid_auth,
                &connection_contract(),
                discovery_impl.as_ref(),
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(error.stage, ProviderDraftValidationStage::Authentication);
        assert_eq!(error.kind, ProviderRefreshFailure::Credential);

        let mut invalid_adapter = draft(ProviderAuthConfig::ApiKey {
            credential_ref: "system-config://secrets/aicc/openai".into(),
        });
        invalid_adapter.protocol_adapter_id = "missing-adapter".into();
        let error = manager
            .validate_draft(
                &invalid_adapter,
                &connection_contract(),
                discovery_impl.as_ref(),
                None,
            )
            .await
            .unwrap_err();
        assert_eq!(error.stage, ProviderDraftValidationStage::Protocol);

        let failed_discovery = ScriptedDiscovery::new(
            [Err("invalid discovery response".into())],
            discovery("gpt-test"),
        );
        let valid = draft(ProviderAuthConfig::ApiKey {
            credential_ref: "system-config://secrets/aicc/openai".into(),
        });
        let error = manager
            .validate_draft(&valid, &connection_contract(), &failed_discovery, None)
            .await
            .unwrap_err();
        assert_eq!(error.stage, ProviderDraftValidationStage::Discovery);
        assert_eq!(error.kind, ProviderRefreshFailure::Discovery);

        assert_eq!(discovery_impl.calls.load(Ordering::SeqCst), 0);
        assert_eq!(store.commits.load(Ordering::SeqCst), 0);
        assert!(manager.runtimes.lock().await.is_empty());
        assert!(manager.registry().await.list().is_empty());
    }

    #[test]
    fn openai_inventory_satisfies_canonical_tool_and_schema_requirements() {
        let catalog = catalog();
        let inventory = InventoryBuilder::build(
            &profile(),
            &instance("primary"),
            discovery("gpt-test"),
            &catalog,
            &codecs(),
        )
        .unwrap()
        .as_model_inventory();
        let registry = ModelRegistry::build(
            &catalog,
            &[inventory],
            vec![LogicalModelDefinition {
                path: "llm.contract".into(),
                api_type: ApiType::Llm,
                min_line: buckyos_api::ModelRequirement {
                    tool_call: true,
                    json_schema: true,
                    ..buckyos_api::ModelRequirement::default()
                },
                disable_line: buckyos_api::ModelDisable::default(),
                default_options: BTreeMap::new(),
                mount_mode: MountMode::Auto,
                scheduler_profile: buckyos_api::AiccSchedulerProfile::Balanced,
                fallback: None,
                route_policy: buckyos_api::AiccPolicyConfig::default(),
                user_visible_tier: None,
            }],
            RegistryLayers::default(),
        )
        .unwrap();

        let candidates = registry
            .resolve_candidates("llm.contract", ApiType::Llm)
            .unwrap();
        assert_eq!(candidates.candidates.len(), 1);
        assert!(candidates.admissions.iter().all(|record| record.admitted));
    }

    #[test]
    fn inventory_uses_provider_origin_mapping_to_select_unique_driver() {
        let mut profile = profile();
        profile.provider_profile_id = "openrouter".into();
        let mut instance = instance("router");
        instance.provider_profile_id = "openrouter".into();
        instance.provider_rules_id = Some("openrouter".into());
        let mut discovered = discovery("anthropic/shared-model");
        discovered.models[0].origin_model_id = Some("shared-model".into());

        let inventory = InventoryBuilder::build(
            &profile,
            &instance,
            discovered,
            &routed_catalog(),
            &codecs(),
        )
        .unwrap();
        assert_eq!(inventory.models.len(), 1);
        assert_eq!(
            inventory.models[0].provider_model_id,
            "anthropic/shared-model"
        );
        assert_eq!(inventory.models[0].origin_model_id, "shared-model");
        assert_eq!(inventory.models[0].model_driver_id, "claude");
    }

    #[tokio::test]
    async fn resolved_credential_never_enters_inventory_or_debug_output() {
        let secret = "super-secret-value";
        let store = Arc::new(MemoryStore::default());
        let discovery = Arc::new(ScriptedDiscovery::new([], discovery("gpt-test")));
        let (manager, discovery) = manager(store, discovery);
        let executable = manager.start(instance("primary"), discovery).await.unwrap();
        let credential = executable.resolve_credential().await.unwrap();
        assert!(!format!("{credential:?}").contains(secret));
        let json = serde_json::to_string(executable.current_inventory().await.as_ref()).unwrap();
        assert!(!json.contains(secret));
        assert!(!json.contains("credential"));
        manager.shutdown().await;
    }

    #[tokio::test]
    async fn discovery_failure_falls_back_to_valid_lkgs() {
        let store = Arc::new(MemoryStore::default());
        let good = Arc::new(ScriptedDiscovery::new([], discovery("gpt-test")));
        let (first_manager, good) = manager(store.clone(), good);
        first_manager
            .start(instance("primary"), good)
            .await
            .unwrap();
        first_manager.shutdown().await;

        let failed = Arc::new(ScriptedDiscovery::new(
            [Err("offline".into())],
            discovery("gpt-test"),
        ));
        let (second_manager, failed) = manager(store, failed);
        let executable = second_manager
            .start(instance("primary"), failed)
            .await
            .unwrap();
        assert_eq!(executable.current_inventory().await.models.len(), 1);
        assert_eq!(
            executable.health().await.state,
            ProviderHealthState::Degraded
        );
        second_manager.shutdown().await;
    }

    #[tokio::test]
    async fn unchanged_probe_updates_health_without_rewriting_inventory() {
        let store = Arc::new(MemoryStore::default());
        let discovery = Arc::new(ScriptedDiscovery::new([], discovery("gpt-test")));
        let (manager, discovery) = manager(store.clone(), discovery);
        manager.start(instance("primary"), discovery).await.unwrap();
        assert_eq!(store.commits.load(Ordering::SeqCst), 1);
        let runtime = manager
            .runtimes
            .lock()
            .await
            .get("primary")
            .cloned()
            .unwrap();
        runtime
            .refresh_once(false, ProviderRefreshTrigger::Manual, true)
            .await
            .unwrap();
        assert_eq!(store.commits.load(Ordering::SeqCst), 1);
        assert_eq!(
            runtime.health.read().await.state,
            ProviderHealthState::Healthy
        );
        manager.shutdown().await;
    }

    #[tokio::test]
    async fn reconciliation_uses_latest_catalog_and_atomically_publishes_registry() {
        let store = Arc::new(MemoryStore::default());
        let discovery = Arc::new(ScriptedDiscovery::new([], discovery("gpt-test")));
        let (manager, discovery) = manager(store, discovery);
        manager.start(instance("primary"), discovery).await.unwrap();
        let old_registry = manager.registry().await;
        assert_eq!(
            old_registry
                .get("primary")
                .unwrap()
                .current_inventory()
                .await
                .metadata_applied_seq,
            7
        );
        let mut events = manager.subscribe_refresh_events();

        let report = manager
            .reconcile_inventory(catalog_with_revision(8, 16_384))
            .await;

        assert_eq!(manager.current_catalog().await.target_revision_seq(), 8);
        assert_eq!(report.len(), 1);
        assert!(matches!(
            report[0].outcome,
            ProviderRefreshOutcome::Committed {
                metadata_applied_seq: 8,
                ..
            }
        ));
        let event = events.recv().await.unwrap();
        assert_eq!(event.trigger, ProviderRefreshTrigger::Reconciliation);
        assert!(matches!(
            event.outcome,
            ProviderRefreshOutcome::Committed {
                metadata_applied_seq: 8,
                ..
            }
        ));
        let new_registry = manager.registry().await;
        assert_eq!(
            new_registry
                .get("primary")
                .unwrap()
                .current_inventory()
                .await
                .models[0]
                .capabilities["context_tokens"],
            Value::from(16_384)
        );
        assert_eq!(
            old_registry
                .get("primary")
                .unwrap()
                .current_inventory()
                .await
                .metadata_applied_seq,
            7
        );
        manager.shutdown().await;
    }

    #[tokio::test]
    async fn candidate_commit_rejects_catalog_or_build_order_staleness() {
        let store = Arc::new(MemoryStore::default());
        let discovery = Arc::new(ScriptedDiscovery::new([], discovery("gpt-test")));
        let (manager, discovery) = manager(store, discovery);
        manager.start(instance("primary"), discovery).await.unwrap();

        let stale_catalog = manager.build_inventory_candidate("primary").await.unwrap();
        manager
            .reconcile_inventory(catalog_with_revision(8, 16_384))
            .await;
        assert!(matches!(
            manager
                .commit_inventory_candidate(stale_catalog, ProviderRefreshTrigger::Reconciliation)
                .await,
            Err(ProviderError::StaleCandidate)
        ));

        let stale_order = manager.build_inventory_candidate("primary").await.unwrap();
        let newest = manager.build_inventory_candidate("primary").await.unwrap();
        assert!(matches!(
            manager
                .commit_inventory_candidate(stale_order, ProviderRefreshTrigger::Manual)
                .await,
            Err(ProviderError::StaleCandidate)
        ));
        assert_eq!(
            manager
                .commit_inventory_candidate(newest, ProviderRefreshTrigger::Manual)
                .await
                .unwrap()
                .metadata_applied_seq,
            8
        );
        manager.shutdown().await;
    }

    #[tokio::test]
    async fn scheduled_refresh_publishes_success_and_error_without_credentials() {
        let store = Arc::new(MemoryStore::default());
        let secret = "event-must-not-contain-this-secret";
        let scripted = Arc::new(ScriptedDiscovery::new(
            [
                Ok(discovery("gpt-test")),
                Err("scheduled endpoint unavailable".into()),
            ],
            discovery("gpt-test"),
        ));
        let manager = Arc::new(
            ProviderRuntimeManager::new([profile()], resolver(secret), catalog(), codecs(), store)
                .unwrap(),
        );
        manager.start(instance("primary"), scripted).await.unwrap();
        let mut events = manager.subscribe_refresh_events();
        let runtime = manager.runtime("primary").await.unwrap();

        assert!(matches!(
            runtime
                .refresh_once(false, ProviderRefreshTrigger::Scheduled, true)
                .await,
            Err(ProviderError::Discovery(_))
        ));
        let event = events.recv().await.unwrap();
        assert_eq!(event.trigger, ProviderRefreshTrigger::Scheduled);
        assert!(matches!(
            event.outcome,
            ProviderRefreshOutcome::Failed { .. }
        ));
        assert!(!format!("{event:?}").contains(secret));
        manager.shutdown().await;
    }

    #[tokio::test]
    async fn failed_refresh_preserves_lkgs_and_reports_degraded_health() {
        let store = Arc::new(MemoryStore::default());
        let scripted = Arc::new(ScriptedDiscovery::new(
            [Ok(discovery("gpt-test")), Err("temporary failure".into())],
            discovery("gpt-test"),
        ));
        let (manager, discovery) = manager(store.clone(), scripted);
        let executable = manager.start(instance("primary"), discovery).await.unwrap();
        let original = executable.current_inventory().await;
        assert!(matches!(
            manager.refresh("primary").await,
            Err(ProviderError::Discovery(_))
        ));
        assert_eq!(store.commits.load(Ordering::SeqCst), 1);
        assert_eq!(
            executable
                .current_inventory()
                .await
                .provider_model_list_fingerprint,
            original.provider_model_list_fingerprint
        );
        let health = executable.health().await;
        assert_eq!(health.state, ProviderHealthState::Degraded);
        assert_eq!(health.consecutive_failures, 1);
        assert!(health.last_error.unwrap().contains("temporary failure"));
        manager.shutdown().await;
    }

    #[tokio::test]
    async fn registry_publication_is_copy_on_write_and_stop_is_idempotent() {
        let store = Arc::new(MemoryStore::default());
        let discovery = Arc::new(ScriptedDiscovery::new([], discovery("gpt-test")));
        let (manager, discovery) = manager(store, discovery);
        let before = manager.registry().await;
        manager.start(instance("primary"), discovery).await.unwrap();
        let after = manager.registry().await;
        assert!(before.get("primary").is_none());
        assert!(after.get("primary").is_some());
        assert_eq!(after.list().len(), 1);

        manager.stop_and_remove("primary").await.unwrap();
        manager.stop_and_remove("primary").await.unwrap();
        assert!(manager.registry().await.get("primary").is_none());
    }

    #[tokio::test]
    async fn invalid_lkgs_is_not_used_as_fallback() {
        let store = Arc::new(MemoryStore::default());
        let snapshot = InventoryBuilder::build(
            &profile(),
            &instance("primary"),
            discovery("gpt-test"),
            &catalog(),
            &codecs(),
        )
        .unwrap();
        let mut record = InventoryLkgsRecord::new(
            "primary",
            "openai",
            "openai-responses",
            &snapshot.provider_model_list_fingerprint,
            snapshot.metadata_applied_seq,
            snapshot.inventory_revision.clone(),
            snapshot.discovered_at_ms,
            serde_json::to_value(&snapshot).unwrap(),
            100,
        )
        .unwrap();
        record.provider_model_list_fingerprint = "tampered".into();
        store.records.lock().await.insert("primary".into(), record);
        let failed = Arc::new(ScriptedDiscovery::new(
            [Err("offline".into())],
            discovery("gpt-test"),
        ));
        let (manager, failed) = manager(store, failed);
        assert!(matches!(
            manager.start(instance("primary"), failed).await,
            Err(ProviderError::Discovery(_))
        ));
    }

    #[test]
    fn invalid_discovery_facts_are_rejected() {
        let mut duplicate = discovery("gpt-test");
        duplicate.models.push(duplicate.models[0].clone());
        assert!(matches!(
            InventoryBuilder::build(
                &profile(),
                &instance("primary"),
                duplicate,
                &catalog(),
                &codecs(),
            ),
            Err(ProviderError::Discovery(_))
        ));

        let mut invalid_price = discovery("gpt-test");
        invalid_price.models[0]
            .pricing
            .as_mut()
            .unwrap()
            .input_token = Some(-1.0);
        assert!(matches!(
            InventoryBuilder::build(
                &profile(),
                &instance("primary"),
                invalid_price,
                &catalog(),
                &codecs(),
            ),
            Err(ProviderError::Discovery(_))
        ));
    }

    #[tokio::test]
    async fn invalid_instance_is_rejected_before_discovery() {
        let store = Arc::new(MemoryStore::default());
        let scripted = Arc::new(ScriptedDiscovery::new([], discovery("gpt-test")));
        let (manager, discovery) = manager(store, scripted.clone());
        let mut invalid = instance("primary");
        invalid.base_url = "file:///tmp/provider".into();
        assert!(matches!(
            manager.start(invalid, discovery).await,
            Err(ProviderError::InvalidConfiguration(_))
        ));
        assert_eq!(scripted.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn stop_is_idempotent_and_rejects_late_refresh_commit() {
        let store = Arc::new(MemoryStore::default());
        let discovery = Arc::new(BlockingDiscovery {
            snapshot: discovery("gpt-test"),
            calls: AtomicUsize::new(0),
            started: Notify::new(),
            release: Notify::new(),
        });
        let (manager, discovery_trait) = manager(store.clone(), discovery.clone());
        manager
            .start(instance("primary"), discovery_trait)
            .await
            .unwrap();
        assert_eq!(store.commits.load(Ordering::SeqCst), 1);

        let refresh_manager = manager.clone();
        let refresh = tokio::spawn(async move { refresh_manager.refresh("primary").await });
        discovery.started.notified().await;
        manager.stop_and_remove("primary").await.unwrap();
        discovery.release.notify_waiters();
        assert!(matches!(
            refresh.await.unwrap(),
            Err(ProviderError::Stopped)
        ));
        assert_eq!(store.commits.load(Ordering::SeqCst), 1);

        manager.shutdown().await;
    }

    #[tokio::test]
    async fn concurrent_refreshes_are_serialized_per_instance() {
        let store = Arc::new(MemoryStore::default());
        let discovery = Arc::new(BlockingDiscovery {
            snapshot: discovery("gpt-test"),
            calls: AtomicUsize::new(0),
            started: Notify::new(),
            release: Notify::new(),
        });
        let (manager, discovery_trait) = manager(store, discovery.clone());
        manager
            .start(instance("primary"), discovery_trait)
            .await
            .unwrap();

        let first_manager = manager.clone();
        let first = tokio::spawn(async move { first_manager.refresh("primary").await });
        discovery.started.notified().await;
        let second_manager = manager.clone();
        let second = tokio::spawn(async move { second_manager.refresh("primary").await });
        tokio::task::yield_now().await;
        assert_eq!(discovery.calls.load(Ordering::SeqCst), 2);

        discovery.release.notify_one();
        discovery.started.notified().await;
        assert!(first.await.unwrap().is_ok());
        assert_eq!(discovery.calls.load(Ordering::SeqCst), 3);
        discovery.release.notify_one();
        assert!(second.await.unwrap().is_ok());
        manager.shutdown().await;
    }

    #[test]
    fn backoff_is_bounded_and_fingerprint_is_order_independent() {
        let policy = RefreshPolicy {
            interval: Duration::from_secs(1),
            initial_backoff: Duration::from_millis(10),
            max_backoff: Duration::from_millis(40),
        };
        assert_eq!(exponential_backoff(&policy, 1), Duration::from_millis(10));
        assert_eq!(exponential_backoff(&policy, 2), Duration::from_millis(20));
        assert_eq!(exponential_backoff(&policy, 9), Duration::from_millis(40));
        let mut discovered = discovery("gpt-test");
        let first = vec![
            discovered.models.remove(0),
            DiscoveredModel {
                provider_model_id: "gpt-other".into(),
                origin_model_id: None,
                api_types: None,
                supported_features: None,
                remote_methods: None,
                availability: ModelAvailability::Unknown,
                deprecated: false,
                pricing: None,
            },
        ];
        let mut second = first.clone();
        second.reverse();
        assert_eq!(
            model_list_fingerprint(&first),
            model_list_fingerprint(&second)
        );
    }
}
