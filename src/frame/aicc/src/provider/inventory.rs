use super::*;
use crate::canonical::CanonicalFieldMapping;

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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub region_base_urls: BTreeMap<String, String>,
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
        let mut base_url = input
            .base_url
            .or_else(|| {
                region
                    .as_ref()
                    .and_then(|region| self.region_base_urls.get(region))
                    .map(String::as_str)
            })
            .unwrap_or(&self.default_base_url)
            .to_owned();
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
    pub credential_kind: Option<CredentialKind>,
    pub provider_rules_id: Option<String>,
    pub region: Option<String>,
    pub workspace: Option<String>,
    pub account: Option<String>,
    pub request_timeout: Duration,
    pub auto_sync_models: bool,
    pub instance_rules: Option<buckyos_api::ProviderInstanceRules>,
}

impl ProviderInstanceConfig {
    pub(super) fn validate(&self) -> ProviderResult<()> {
        validate_id("provider_instance_name", &self.provider_instance_name)?;
        validate_id("provider_profile_id", &self.provider_profile_id)?;
        validate_id("protocol_adapter_id", &self.protocol_adapter_id)?;
        if self.credential.reference.trim().is_empty() {
            return Err(ProviderError::InvalidConfiguration(
                "credential reference must not be empty".into(),
            ));
        }
        if self.request_timeout.is_zero() {
            return Err(ProviderError::InvalidConfiguration(
                "provider request timeout must be greater than zero".into(),
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
    async fn refresh_catalog(
        &self,
        _catalog: &CatalogSnapshot,
        _provider_profile_id: &str,
    ) -> ProviderResult<()> {
        Ok(())
    }

    async fn discover(
        &self,
        context: &DiscoveryContext<'_>,
    ) -> ProviderResult<ProviderDiscoverySnapshot>;
}

pub(crate) struct CatalogOnlyDiscovery {
    pub(super) snapshot: RwLock<ProviderDiscoverySnapshot>,
    catalog_managed: bool,
}

pub(crate) struct FallbackDiscovery {
    primary: Arc<dyn ProviderDiscovery>,
    fallback: Arc<dyn ProviderDiscovery>,
}

impl FallbackDiscovery {
    pub(crate) fn new(
        primary: Arc<dyn ProviderDiscovery>,
        fallback: Arc<dyn ProviderDiscovery>,
    ) -> Self {
        Self { primary, fallback }
    }
}

#[async_trait]
impl ProviderDiscovery for FallbackDiscovery {
    async fn refresh_catalog(
        &self,
        catalog: &CatalogSnapshot,
        provider_profile_id: &str,
    ) -> ProviderResult<()> {
        self.primary
            .refresh_catalog(catalog, provider_profile_id)
            .await?;
        self.fallback
            .refresh_catalog(catalog, provider_profile_id)
            .await
    }

    async fn discover(
        &self,
        context: &DiscoveryContext<'_>,
    ) -> ProviderResult<ProviderDiscoverySnapshot> {
        match self.primary.discover(context).await {
            Ok(snapshot) => Ok(snapshot),
            Err(ProviderError::Discovery(_)) => {
                let mut snapshot = self.fallback.discover(context).await?;
                snapshot.health = ProviderHealthState::Degraded;
                Ok(snapshot)
            }
            Err(error) => Err(error),
        }
    }
}

impl CatalogOnlyDiscovery {
    pub(crate) fn new(snapshot: ProviderDiscoverySnapshot) -> Self {
        Self {
            snapshot: RwLock::new(snapshot),
            catalog_managed: false,
        }
    }

    pub(crate) fn catalog_managed(snapshot: ProviderDiscoverySnapshot) -> Self {
        Self {
            snapshot: RwLock::new(snapshot),
            catalog_managed: true,
        }
    }
}

#[async_trait]
impl ProviderDiscovery for CatalogOnlyDiscovery {
    async fn refresh_catalog(
        &self,
        catalog: &CatalogSnapshot,
        provider_profile_id: &str,
    ) -> ProviderResult<()> {
        if self.catalog_managed {
            *self.snapshot.write().await = catalog_only_inventory(catalog, provider_profile_id)
                .ok_or_else(|| {
                    ProviderError::Discovery(format!(
                        "provider profile `{provider_profile_id}` has no catalog models"
                    ))
                })?;
        }
        Ok(())
    }

    async fn discover(
        &self,
        _context: &DiscoveryContext<'_>,
    ) -> ProviderResult<ProviderDiscoverySnapshot> {
        Ok(self.snapshot.read().await.clone())
    }
}

pub(crate) fn catalog_only_inventory(
    catalog: &CatalogSnapshot,
    provider_profile_id: &str,
) -> Option<ProviderDiscoverySnapshot> {
    let rules = catalog.provider_rules(provider_profile_id)?;
    let excluded = rules
        .models
        .iter()
        .filter(|model| model.exclude)
        .map(|model| model.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut model_ids = rules
        .models
        .iter()
        .filter(|model| !model.exclude)
        .map(|model| model.id.clone())
        .collect::<BTreeSet<_>>();
    if let Some(model_drivers) = &rules.metadata_drivers {
        for model_driver_id in model_drivers {
            if let Some(driver) = catalog.model_driver(model_driver_id) {
                model_ids.extend(
                    driver
                        .models
                        .iter()
                        .map(|model| model.id.clone())
                        .filter(|model_id| !excluded.contains(model_id.as_str())),
                );
            }
        }
    }
    let models = model_ids
        .into_iter()
        .map(|provider_model_id| DiscoveredModel {
            provider_model_id,
            origin_model_id: None,
            api_types: None,
            supported_features: None,
            remote_methods: None,
            availability: ModelAvailability::Available,
            deprecated: false,
            pricing: None,
        })
        .collect::<Vec<_>>();
    (!models.is_empty()).then(|| ProviderDiscoverySnapshot {
        revision: Some(format!(
            "catalog-{provider_profile_id}-{}",
            rules.revision_seq
        )),
        discovered_at_ms: 0,
        health: ProviderHealthState::Healthy,
        models,
    })
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub canonical_fields: BTreeMap<String, CanonicalFieldMapping>,
    #[serde(default)]
    pub operations: BTreeMap<String, String>,
    #[serde(default)]
    pub variants: Vec<InventoryModelVariant>,
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
            inventory_revision: self
                .inventory_revision
                .clone()
                .unwrap_or_else(|| self.provider_model_list_fingerprint.clone()),
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
                    variants: model.variants.clone(),
                    capabilities: model.capabilities.clone(),
                    canonical_fields: model.canonical_fields.clone(),
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
        let instance_rules = instance.instance_rules.clone().unwrap_or_default();
        let fingerprint = model_list_fingerprint(&discovery.models);
        let mut models = Vec::new();
        let mut version_rule_refs = BTreeMap::new();

        for discovered in discovery.models {
            if instance_rules
                .exclude_models
                .contains(&discovered.provider_model_id)
            {
                continue;
            }
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
            let origin_model_id = instance_rules
                .origin_model_overrides
                .get(&discovered.provider_model_id)
                .cloned()
                .unwrap_or(origin_model_id);
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
            let conservative_fallback = resolved.model_driver_id.is_none();
            let model_driver_id = resolved
                .model_driver_id
                .clone()
                .unwrap_or_else(|| "unclassified".to_owned());
            version_rule_refs.insert(
                discovered.provider_model_id.clone(),
                resolved.semantics.version_rules.clone(),
            );
            let mut static_api_types = resolved.semantics.api_types.unwrap_or_default();
            if conservative_fallback && static_api_types.is_empty() {
                static_api_types = discovered
                    .api_types
                    .as_ref()
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|api_type| api_type_name(*api_type).ok())
                            .collect()
                    })
                    .unwrap_or_else(|| BTreeSet::from(["llm".to_owned()]));
            }
            let mut capabilities = resolved.semantics.capabilities.unwrap_or_default();
            let mut canonical_fields = resolved.semantics.canonical_fields.unwrap_or_default();
            let mut pricing = resolved.semantics.pricing.map(|value| InventoryPricing {
                source: PricingSource::ModelDriver,
                value,
            });
            let mut provider_rules_revision = None;
            let operation_overrides = if let Some(rule) = &provider_rule {
                let narrowed = rule.action.narrow(&static_api_types, &capabilities);
                static_api_types = narrowed.api_types;
                capabilities = narrowed.capabilities;
                canonical_fields.extend(rule.action.canonical_fields.clone());
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
                let operation_id = resolve_operation(
                    adapter,
                    operation_overrides,
                    discovered.remote_methods.as_ref(),
                    api_type,
                    &api_type_name,
                )?;
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
            let mut logical_mounts: Vec<String> = resolved
                .semantics
                .logical_mounts
                .unwrap_or_default()
                .into_iter()
                .map(|mount| expand_mount_template(&mount, &model_driver_id, &origin_model_id))
                .collect();
            logical_mounts.retain(|mount| logical_mount_matches_api_types(mount, &api_types));
            let model_uid = ModelUid::new(
                &model_driver_id,
                &origin_model_id,
                &instance.protocol_adapter_id,
                None,
            )
            .map_err(|error| ProviderError::Inventory(error.to_string()))?
            .as_stable_string();
            let effective_variants = catalog
                .effective_model_variants(
                    catalog.provider_rules(rules_id).map(|_| rules_id),
                    &model_driver_id,
                    &dimensions,
                )
                .map_err(|error| ProviderError::Inventory(error.to_string()))?;
            let mut variants = effective_variants
                .variants
                .into_iter()
                .map(|variant| InventoryModelVariant {
                    name: variant.name().to_owned(),
                    logical_mounts: variant
                        .model
                        .and_then(|model| model.mount_suffix.as_ref())
                        .map(|suffix| {
                            logical_mounts
                                .clone()
                                .into_iter()
                                .map(|mount| format!("{mount}.{suffix}"))
                                .collect()
                        })
                        .unwrap_or_default(),
                })
                .collect::<Vec<_>>();
            variants.sort_by(|left, right| left.name.cmp(&right.name));
            variants.dedup_by(|left, right| left.name == right.name);
            models.push(ProviderInventoryModel {
                provider_model_id: discovered.provider_model_id,
                model_uid,
                model_driver_id,
                origin_model_id,
                api_types,
                logical_mounts,
                capabilities,
                canonical_fields,
                operations,
                variants,
                availability: discovered.availability,
                deprecated: discovered.deprecated,
                remote_methods: discovered.remote_methods,
                pricing,
                model_catalog_revision: resolved.catalog_revision_seq,
                provider_rules_revision,
            });
        }
        apply_version_rules(catalog, &version_rule_refs, &mut models)?;
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

fn apply_version_rules(
    catalog: &CatalogSnapshot,
    references: &BTreeMap<String, Option<Vec<String>>>,
    models: &mut [ProviderInventoryModel],
) -> ProviderResult<()> {
    let mut winners = BTreeMap::<(String, String), (usize, VersionRank)>::new();
    for (index, model) in models.iter_mut().enumerate() {
        let Some(rule_ids) = references
            .get(&model.provider_model_id)
            .and_then(Option::as_ref)
        else {
            continue;
        };
        let context = MatchContext::from([
            (
                "provider_model_id".to_owned(),
                Value::String(model.provider_model_id.clone()),
            ),
            (
                "origin_model_id".to_owned(),
                Value::String(model.origin_model_id.clone()),
            ),
        ]);
        for rule in catalog
            .matching_version_rules(&model.model_driver_id, &context)
            .map_err(|error| ProviderError::Inventory(error.to_string()))?
            .into_iter()
            .filter(|rule| rule_ids.contains(&rule.id))
        {
            if !matches_version_tier(&model.origin_model_id, rule) {
                continue;
            }
            let version_mount = expand_version_mount(&rule.version_mount, &model.origin_model_id);
            if !logical_mount_matches_api_types(&version_mount, &model.api_types) {
                continue;
            }
            if !model.logical_mounts.contains(&version_mount) {
                model.logical_mounts.push(version_mount);
            }
            for mount in &rule.auto_mounts {
                let auto_mount = expand_version_mount(mount, &model.origin_model_id);
                if logical_mount_matches_api_types(&auto_mount, &model.api_types)
                    && !model.logical_mounts.contains(&auto_mount)
                {
                    model.logical_mounts.push(auto_mount);
                }
            }
            let rank = version_rank(&model.origin_model_id, rule);
            if rule
                .stability
                .as_ref()
                .is_some_and(|stability| stability.current_requires_stable && !rank.stable)
            {
                continue;
            }
            let key = (model.model_driver_id.clone(), rule.id.clone());
            if winners.get(&key).is_none_or(|(_, current)| rank > *current) {
                winners.insert(key, (index, rank));
            }
        }
    }
    for ((model_driver_id, rule_id), (index, _)) in winners {
        let context = MatchContext::from([
            (
                "provider_model_id".to_owned(),
                Value::String(models[index].provider_model_id.clone()),
            ),
            (
                "origin_model_id".to_owned(),
                Value::String(models[index].origin_model_id.clone()),
            ),
        ]);
        let rule = catalog
            .matching_version_rules(&model_driver_id, &context)
            .map_err(|error| ProviderError::Inventory(error.to_string()))?
            .into_iter()
            .find(|rule| rule.id == rule_id)
            .ok_or_else(|| {
                ProviderError::Inventory(format!("version rule `{rule_id}` disappeared"))
            })?;
        let current_mount =
            expand_version_mount(&rule.current_mount, &models[index].origin_model_id);
        if logical_mount_matches_api_types(&current_mount, &models[index].api_types)
            && !models[index].logical_mounts.contains(&current_mount)
        {
            models[index].logical_mounts.push(current_mount);
        }
    }
    for model in models {
        model.logical_mounts.sort();
        model.logical_mounts.dedup();
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct VersionRank {
    version: Vec<u64>,
    pub(super) stable: bool,
    model_id: String,
}

pub(super) fn matches_version_tier(model_id: &str, rule: &VersionRule) -> bool {
    let tokens = version_tokens(model_id);
    (rule.tier_tokens.is_empty()
        || rule
            .tier_tokens
            .iter()
            .all(|token| tokens.contains(&token.to_ascii_lowercase())))
        && !rule
            .exclude_tier_tokens
            .iter()
            .any(|token| tokens.contains(&token.to_ascii_lowercase()))
}

pub(super) fn version_rank(model_id: &str, rule: &VersionRule) -> VersionRank {
    let normalized = model_id.trim().to_ascii_lowercase().replace('_', "-");
    let offset = rule
        .version_rank
        .as_ref()
        .and_then(|rank| {
            normalized
                .find(&rank.prefix.to_ascii_lowercase())
                .map(|pos| pos + rank.prefix.len())
        })
        .unwrap_or_default();
    let version = normalized[offset..]
        .trim_start_matches(['-', '.'])
        .split(|ch: char| !ch.is_ascii_digit())
        .take_while(|part| !part.is_empty())
        .filter_map(|part| part.parse::<u64>().ok())
        .collect();
    let tokens = version_tokens(&normalized);
    let stable = rule.stability.as_ref().is_none_or(|stability| {
        !stability
            .unstable_tokens
            .iter()
            .any(|token| tokens.contains(&token.to_ascii_lowercase()))
    });
    VersionRank {
        version,
        stable,
        model_id: normalized,
    }
}

fn version_tokens(model_id: &str) -> BTreeSet<String> {
    model_id
        .to_ascii_lowercase()
        .split(|ch: char| matches!(ch, '-' | '_' | '.' | '/'))
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .collect()
}

pub(super) fn expand_version_mount(template: &str, model_id: &str) -> String {
    template.replace("{model}", &logical_mount_segment(model_id))
}

fn expand_mount_template(template: &str, driver_id: &str, model_id: &str) -> String {
    template
        .replace("{driver}", &logical_mount_segment(driver_id))
        .replace("{model}", &logical_mount_segment(model_id))
}

fn logical_mount_segment(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('/')
        .split(|ch: char| matches!(ch, '/' | '_' | '.' | '-'))
        .filter(|part| !part.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join("-")
}

fn logical_mount_matches_api_types(mount: &str, api_types: &[ApiType]) -> bool {
    let namespace = mount.split('.').next().unwrap_or_default();
    api_types.iter().any(|api_type| {
        if *api_type == ApiType::AgentComputerUse {
            namespace == "agent_runtime"
        } else {
            api_type_name(*api_type)
                .ok()
                .is_some_and(|name| name.split('.').next() == Some(namespace))
        }
    })
}
