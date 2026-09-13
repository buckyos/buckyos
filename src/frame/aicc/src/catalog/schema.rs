use super::*;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum CatalogKind {
    ModelDriver,
    ProviderRules,
    KnownProvider,
}

impl fmt::Display for CatalogKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ModelDriver => "model_driver",
            Self::ProviderRules => "provider_rules",
            Self::KnownProvider => "known_provider",
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CurrentCatalogFile {
    pub kind: CatalogKind,
    pub contents: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct CatalogDocuments {
    pub model_drivers: Vec<ModelDriverCatalog>,
    pub provider_rules: Vec<ProviderRulesCatalog>,
    pub known_providers: Vec<KnownProviderCatalog>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct CatalogBuildOptions {
    pub supported_features: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelDriverCatalog {
    pub format: String,
    pub schema_version: u32,
    pub schema_revision: u32,
    pub model_driver_id: String,
    pub revision_seq: u64,
    #[serde(default)]
    pub required_features: Vec<String>,
    #[serde(default)]
    pub models: Vec<ModelExactRule>,
    #[serde(default)]
    pub patterns: Vec<ModelPatternRule>,
    #[serde(default)]
    pub defaults: ModelSemantics,
    #[serde(default)]
    pub variants: Vec<ModelVariant>,
    #[serde(default)]
    pub version_rules: Vec<VersionRule>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelSemantics {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_driver: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameter_scale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_types: Option<BTreeSet<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_mounts: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<BTreeMap<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<Pricing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_latency_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality_score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_rules: Option<Vec<String>>,
}

impl ModelSemantics {
    pub(super) fn overlay(&self, rule: &Self) -> Self {
        Self {
            model_driver: rule
                .model_driver
                .clone()
                .or_else(|| self.model_driver.clone()),
            exclude: rule.exclude.or(self.exclude),
            parameter_scale: rule
                .parameter_scale
                .clone()
                .or_else(|| self.parameter_scale.clone()),
            api_types: rule.api_types.clone().or_else(|| self.api_types.clone()),
            logical_mounts: rule
                .logical_mounts
                .clone()
                .or_else(|| self.logical_mounts.clone()),
            capabilities: rule
                .capabilities
                .clone()
                .or_else(|| self.capabilities.clone()),
            pricing: rule.pricing.clone().or_else(|| self.pricing.clone()),
            estimated_latency_ms: rule.estimated_latency_ms.or(self.estimated_latency_ms),
            quality_score: rule.quality_score.or(self.quality_score),
            latency_class: rule
                .latency_class
                .clone()
                .or_else(|| self.latency_class.clone()),
            cost_class: rule.cost_class.clone().or_else(|| self.cost_class.clone()),
            version_rules: rule
                .version_rules
                .clone()
                .or_else(|| self.version_rules.clone()),
        }
    }

    pub(super) fn conservative() -> Self {
        Self {
            exclude: Some(false),
            api_types: Some(BTreeSet::new()),
            logical_mounts: Some(Vec::new()),
            capabilities: Some(BTreeMap::new()),
            ..Self::default()
        }
    }
}

macro_rules! define_model_rule {
    ($name:ident, { $($identity:tt)* }) => {
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub(crate) struct $name {
            $($identity)*
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub model_driver: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub exclude: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub parameter_scale: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub api_types: Option<BTreeSet<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub logical_mounts: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub capabilities: Option<BTreeMap<String, Value>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub pricing: Option<Pricing>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub estimated_latency_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub quality_score: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub latency_class: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub cost_class: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub version_rules: Option<Vec<String>>,
        }
    };
}

define_model_rule!(ModelExactRule, { pub id: String, });
define_model_rule!(
    ModelPatternRule,
    {
        #[serde(rename = "match")]
        pub match_rule: MatchRule,
    }
);

macro_rules! model_rule_semantics {
    ($rule:expr) => {
        ModelSemantics {
            model_driver: $rule.model_driver.clone(),
            exclude: $rule.exclude,
            parameter_scale: $rule.parameter_scale.clone(),
            api_types: $rule.api_types.clone(),
            logical_mounts: $rule.logical_mounts.clone(),
            capabilities: $rule.capabilities.clone(),
            pricing: $rule.pricing.clone(),
            estimated_latency_ms: $rule.estimated_latency_ms,
            quality_score: $rule.quality_score,
            latency_class: $rule.latency_class.clone(),
            cost_class: $rule.cost_class.clone(),
            version_rules: $rule.version_rules.clone(),
        }
    };
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelVariant {
    pub name: String,
    #[serde(rename = "match")]
    pub match_rule: MatchRule,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mount_suffix: Option<String>,
    #[serde(default)]
    pub provider_options: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VersionRule {
    pub id: String,
    pub family: String,
    pub tier: String,
    #[serde(rename = "match")]
    pub match_rule: MatchRule,
    #[serde(default)]
    pub tier_tokens: Vec<String>,
    #[serde(default)]
    pub exclude_tier_tokens: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_rank: Option<VersionRank>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stability: Option<VersionStability>,
    pub current_mount: String,
    pub version_mount: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub auto_mounts: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VersionRank {
    pub prefix: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VersionStability {
    #[serde(default)]
    pub unstable_tokens: Vec<String>,
    #[serde(default)]
    pub current_requires_stable: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Pricing {
    pub currency: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_token: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_token: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_input_token: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_cost: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<PricingUnit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<f64>,
    #[serde(default)]
    pub rules: Vec<PricingRule>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PricingUnit {
    Request,
    Image,
    AudioSecond,
    VideoSecond,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PricingRule {
    pub when: MatchRule,
    pub amount: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderRulesCatalog {
    pub format: String,
    pub schema_version: u32,
    pub schema_revision: u32,
    pub revision_seq: u64,
    pub provider_profile_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata_drivers: Option<Vec<String>>,
    #[serde(default)]
    pub origin_provider_aliases: BTreeMap<String, String>,
    #[serde(default)]
    pub origin_mappings: Vec<OriginMapping>,
    #[serde(default)]
    pub models: Vec<ProviderExactRule>,
    #[serde(default)]
    pub patterns: Vec<ProviderPatternRule>,
    #[serde(default)]
    pub variants: Vec<ProviderVariantRule>,
}

macro_rules! define_provider_rule {
    ($name:ident, { $($identity:tt)* }) => {
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub(crate) struct $name {
            $($identity)*
        #[serde(default)]
        pub exclude: bool,
        #[serde(default)]
        pub operations: BTreeMap<String, String>,
        #[serde(default)]
        pub provider_options: BTreeMap<String, Value>,
        #[serde(default)]
        pub request_rules: Vec<RequestRule>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub pricing: Option<Pricing>,
        #[serde(default)]
        pub remove_api_types: BTreeSet<String>,
        #[serde(default)]
        pub remove_features: BTreeSet<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub estimated_latency_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub latency_class: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub cost_class: Option<String>,
        }
    };
}

define_provider_rule!(ProviderExactRule, { pub id: String, });
define_provider_rule!(
    ProviderPatternRule,
    {
        #[serde(rename = "match")]
        pub match_rule: MatchRule,
    }
);

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ProviderRuleAction {
    pub exclude: bool,
    pub operations: BTreeMap<String, String>,
    pub provider_options: BTreeMap<String, Value>,
    pub request_rules: Vec<RequestRule>,
    pub pricing: Option<Pricing>,
    pub remove_api_types: BTreeSet<String>,
    pub remove_features: BTreeSet<String>,
    pub estimated_latency_ms: Option<u64>,
    pub latency_class: Option<String>,
    pub cost_class: Option<String>,
}

macro_rules! provider_rule_action {
    ($rule:expr) => {
        ProviderRuleAction {
            exclude: $rule.exclude,
            operations: $rule.operations.clone(),
            provider_options: $rule.provider_options.clone(),
            request_rules: $rule.request_rules.clone(),
            pricing: $rule.pricing.clone(),
            remove_api_types: $rule.remove_api_types.clone(),
            remove_features: $rule.remove_features.clone(),
            estimated_latency_ms: $rule.estimated_latency_ms,
            latency_class: $rule.latency_class.clone(),
            cost_class: $rule.cost_class.clone(),
        }
    };
}

impl ProviderRuleAction {
    pub(crate) fn narrow(
        &self,
        api_types: &BTreeSet<String>,
        capabilities: &BTreeMap<String, Value>,
    ) -> NarrowedCapabilities {
        NarrowedCapabilities {
            api_types: api_types
                .difference(&self.remove_api_types)
                .cloned()
                .collect(),
            capabilities: capabilities
                .iter()
                .filter(|(name, _)| !self.remove_features.contains(*name))
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NarrowedCapabilities {
    pub api_types: BTreeSet<String>,
    pub capabilities: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<MatchRule>,
    #[serde(default)]
    pub defaults: BTreeMap<String, Value>,
    #[serde(default)]
    pub set: BTreeMap<String, Value>,
    #[serde(default)]
    pub remove: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OriginMapping {
    pub extract: OriginExtract,
    #[serde(default)]
    pub transforms: BTreeMap<String, Vec<OriginTransform>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OriginExtract {
    pub source: String,
    pub regex: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OriginTransform {
    pub op: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_missing: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedProviderOrigin {
    pub origin_model_id: String,
    pub model_driver_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderVariantRule {
    pub model_driver: String,
    pub variant: String,
    #[serde(rename = "match")]
    pub match_rule: MatchRule,
    #[serde(default)]
    pub provider_options: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct KnownProviderCatalog {
    pub format: String,
    pub schema_version: u32,
    pub schema_revision: u32,
    pub revision_seq: u64,
    pub catalog_id: String,
    #[serde(default)]
    pub providers: Vec<KnownProvider>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct KnownProvider {
    pub provider_profile_id: String,
    pub display_name: String,
    pub base_url: String,
    pub protocol_adapter_id: String,
    pub discovery_behavior_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_login_behavior_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_behavior_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_rules_id: Option<String>,
    pub credential: ProviderCredentialDescriptor,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub credential_variants: Vec<ProviderCredentialDescriptor>,
    pub connection: ProviderConnectionSchema,
    #[serde(default)]
    pub ui_hints: BTreeMap<String, Value>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderCredentialKind {
    Bearer,
    NamedHeader,
    FalKey,
    GlmJwt,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderCredentialDescriptor {
    pub kind: ProviderCredentialKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_name: Option<String>,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_values: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderConnectionSchema {
    pub region: ProviderFieldSchema,
    pub workspace: ProviderFieldSchema,
    pub account: ProviderFieldSchema,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub region_base_urls: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedProviderConfiguration {
    pub provider_profile_id: String,
    pub display_name: String,
    pub default_base_url: String,
    pub credential: ProviderCredentialDescriptor,
    pub credential_variants: Vec<ProviderCredentialDescriptor>,
    pub connection: ProviderConnectionSchema,
    pub protocol_adapter_id: String,
    pub discovery_behavior_id: String,
    pub dynamic_login_behavior_id: Option<String>,
    pub connection_behavior_id: Option<String>,
    pub provider_rules_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ModelMatchKind {
    Exact,
    Pattern,
    Defaults,
    ConservativeFallback,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ResolvedModelSemantics {
    pub origin_model_id: String,
    pub source_model_driver_id: Option<String>,
    pub model_driver_id: Option<String>,
    pub catalog_revision_seq: Option<u64>,
    pub match_kind: ModelMatchKind,
    pub trace: Option<MatchTrace>,
    pub semantics: ModelSemantics,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProviderRuleMatchKind {
    Exact,
    Pattern,
}
#[derive(Clone, Debug)]
pub(crate) struct ResolvedProviderRule {
    pub catalog_revision_seq: u64,
    #[cfg(test)]
    pub match_kind: ProviderRuleMatchKind,
    #[cfg(test)]
    pub trace: Option<MatchTrace>,
    pub action: ProviderRuleAction,
    pub(super) compiled: CompiledProviderRule,
}

impl ResolvedProviderRule {
    pub(crate) fn matching_request_rules(&self, context: &MatchContext) -> Vec<&RequestRule> {
        self.action
            .request_rules
            .iter()
            .zip(&self.compiled.request_conditions)
            .filter_map(|(rule, condition)| {
                condition
                    .as_ref()
                    .is_none_or(|condition| condition.matches(context))
                    .then_some(rule)
            })
            .collect()
    }

    pub(crate) fn price_for(&self, context: &MatchContext) -> Option<f64> {
        let pricing = self.action.pricing.as_ref()?;
        pricing
            .rules
            .iter()
            .zip(&self.compiled.pricing_rules)
            .find_map(|(rule, condition)| condition.matches(context).then_some(rule.amount))
            .or(pricing.amount)
            .or(pricing.estimated_cost)
    }
}
