#![allow(dead_code)]

use crate::matching::{
    CompiledMatchRule, CompiledRuleSet, MatchCompileError, MatchContext, MatchRule, MatchTrace,
    RuleEntry, MODEL_DRIVER_MATCH_SCHEMA, PRICING_RULE_MATCH_SCHEMA, PROVIDER_RULE_MATCH_SCHEMA,
    REQUEST_RULE_MATCH_SCHEMA,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

const MODEL_DRIVER_FORMAT: &str = "buckyos.aicc.model-driver-catalog";
const PROVIDER_RULES_FORMAT: &str = "buckyos.aicc.provider-rules-catalog";
const KNOWN_PROVIDER_FORMAT: &str = "buckyos.aicc.known-provider-catalog";
const SUPPORTED_SCHEMA_VERSION: u32 = 1;
const SUPPORTED_SCHEMA_REVISION: u32 = 0;

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
    fn overlay(&self, rule: &Self) -> Self {
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

    fn conservative() -> Self {
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_rules_id: Option<String>,
    #[serde(default)]
    pub ui_hints: BTreeMap<String, Value>,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProviderRuleMatchKind {
    Exact,
    Pattern,
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedProviderRule {
    pub provider_profile_id: String,
    pub catalog_revision_seq: u64,
    pub match_kind: ProviderRuleMatchKind,
    pub trace: Option<MatchTrace>,
    pub action: ProviderRuleAction,
    compiled: CompiledProviderRule,
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

#[derive(Clone, Debug)]
struct CompiledModelDriverCatalog {
    document: ModelDriverCatalog,
    exact_index: BTreeMap<String, usize>,
    patterns: CompiledRuleSet,
    compiled_variants: Vec<CompiledMatchRule>,
    compiled_version_rules: Vec<CompiledMatchRule>,
}

#[derive(Clone, Debug)]
struct CompiledProviderRule {
    request_conditions: Vec<Option<CompiledMatchRule>>,
    pricing_rules: Vec<CompiledMatchRule>,
}

#[derive(Clone, Debug)]
struct CompiledProviderRulesCatalog {
    document: ProviderRulesCatalog,
    origin_mappings: Vec<CompiledOriginMapping>,
    exact_index: BTreeMap<String, usize>,
    patterns: CompiledRuleSet,
    exact_compiled: Vec<CompiledProviderRule>,
    pattern_compiled: Vec<CompiledProviderRule>,
    compiled_variants: Vec<CompiledMatchRule>,
}

#[derive(Clone, Copy, Debug)]
enum CompiledOriginMapping {
    VendorModel,
}

#[derive(Clone, Debug)]
pub(crate) struct CatalogSnapshot {
    target_revision_seq: u64,
    model_drivers: BTreeMap<String, CompiledModelDriverCatalog>,
    provider_rules: BTreeMap<String, CompiledProviderRulesCatalog>,
    known_provider_catalogs: BTreeMap<String, KnownProviderCatalog>,
    model_exact_index: BTreeMap<String, Vec<String>>,
    known_provider_index: BTreeMap<String, (String, usize)>,
}

impl CatalogSnapshot {
    pub(crate) fn from_current_files(
        target_revision_seq: u64,
        files: impl IntoIterator<Item = CurrentCatalogFile>,
        options: &CatalogBuildOptions,
    ) -> Result<Self, CatalogBuildError> {
        let mut documents = CatalogDocuments::default();
        for (position, file) in files.into_iter().enumerate() {
            match file.kind {
                CatalogKind::ModelDriver => {
                    documents
                        .model_drivers
                        .push(serde_json::from_slice(&file.contents).map_err(|source| {
                            CatalogBuildError::InvalidJson {
                                kind: file.kind,
                                position,
                                source,
                            }
                        })?)
                }
                CatalogKind::ProviderRules => {
                    documents
                        .provider_rules
                        .push(serde_json::from_slice(&file.contents).map_err(|source| {
                            CatalogBuildError::InvalidJson {
                                kind: file.kind,
                                position,
                                source,
                            }
                        })?)
                }
                CatalogKind::KnownProvider => {
                    documents
                        .known_providers
                        .push(serde_json::from_slice(&file.contents).map_err(|source| {
                            CatalogBuildError::InvalidJson {
                                kind: file.kind,
                                position,
                                source,
                            }
                        })?)
                }
            }
        }
        Self::build(target_revision_seq, documents, options)
    }

    pub(crate) fn build(
        target_revision_seq: u64,
        documents: CatalogDocuments,
        options: &CatalogBuildOptions,
    ) -> Result<Self, CatalogBuildError> {
        let mut model_drivers = BTreeMap::new();
        for document in documents.model_drivers {
            validate_model_driver(&document, options)?;
            let id = document.model_driver_id.clone();
            let compiled = compile_model_driver(document)?;
            if model_drivers.insert(id.clone(), compiled).is_some() {
                return Err(CatalogBuildError::DuplicateCatalog {
                    kind: CatalogKind::ModelDriver,
                    id,
                });
            }
        }

        let mut provider_rules = BTreeMap::new();
        for document in documents.provider_rules {
            validate_provider_rules(&document)?;
            let id = document.provider_profile_id.clone();
            let compiled = compile_provider_rules(document)?;
            if provider_rules.insert(id.clone(), compiled).is_some() {
                return Err(CatalogBuildError::DuplicateCatalog {
                    kind: CatalogKind::ProviderRules,
                    id,
                });
            }
        }

        let mut known_provider_catalogs = BTreeMap::new();
        for document in documents.known_providers {
            validate_known_provider_catalog(&document)?;
            let id = document.catalog_id.clone();
            if known_provider_catalogs
                .insert(id.clone(), document)
                .is_some()
            {
                return Err(CatalogBuildError::DuplicateCatalog {
                    kind: CatalogKind::KnownProvider,
                    id,
                });
            }
        }

        validate_references(&model_drivers, &provider_rules, &known_provider_catalogs)?;
        validate_revisions(
            target_revision_seq,
            &model_drivers,
            &provider_rules,
            &known_provider_catalogs,
        )?;

        let mut model_exact_index: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (driver_id, catalog) in &model_drivers {
            for model_id in catalog.exact_index.keys() {
                model_exact_index
                    .entry(model_id.clone())
                    .or_default()
                    .push(driver_id.clone());
            }
        }

        let mut known_provider_index = BTreeMap::new();
        for (catalog_id, catalog) in &known_provider_catalogs {
            for (position, provider) in catalog.providers.iter().enumerate() {
                if known_provider_index
                    .insert(
                        provider.provider_profile_id.clone(),
                        (catalog_id.clone(), position),
                    )
                    .is_some()
                {
                    return Err(CatalogBuildError::DuplicateKnownProvider {
                        provider_profile_id: provider.provider_profile_id.clone(),
                    });
                }
            }
        }

        Ok(Self {
            target_revision_seq,
            model_drivers,
            provider_rules,
            known_provider_catalogs,
            model_exact_index,
            known_provider_index,
        })
    }

    pub(crate) fn target_revision_seq(&self) -> u64 {
        self.target_revision_seq
    }

    pub(crate) fn model_driver(&self, id: &str) -> Option<&ModelDriverCatalog> {
        self.model_drivers.get(id).map(|catalog| &catalog.document)
    }

    pub(crate) fn provider_rules(&self, id: &str) -> Option<&ProviderRulesCatalog> {
        self.provider_rules.get(id).map(|catalog| &catalog.document)
    }

    pub(crate) fn known_provider(&self, provider_profile_id: &str) -> Option<&KnownProvider> {
        let (catalog_id, position) = self.known_provider_index.get(provider_profile_id)?;
        self.known_provider_catalogs
            .get(catalog_id)
            .and_then(|catalog| catalog.providers.get(*position))
    }

    pub(crate) fn known_providers(&self) -> impl Iterator<Item = &KnownProvider> {
        self.known_provider_index
            .values()
            .filter_map(|(catalog_id, position)| {
                self.known_provider_catalogs
                    .get(catalog_id)
                    .and_then(|catalog| catalog.providers.get(*position))
            })
    }

    pub(crate) fn matching_model_variants(
        &self,
        model_driver_id: &str,
        context: &MatchContext,
    ) -> Result<Vec<&ModelVariant>, CatalogResolveError> {
        let catalog = self.model_drivers.get(model_driver_id).ok_or_else(|| {
            CatalogResolveError::UnknownModelDriver {
                model_driver_id: model_driver_id.to_owned(),
            }
        })?;
        Ok(catalog
            .document
            .variants
            .iter()
            .zip(&catalog.compiled_variants)
            .filter_map(|(variant, condition)| condition.matches(context).then_some(variant))
            .collect())
    }

    pub(crate) fn matching_version_rules(
        &self,
        model_driver_id: &str,
        context: &MatchContext,
    ) -> Result<Vec<&VersionRule>, CatalogResolveError> {
        let catalog = self.model_drivers.get(model_driver_id).ok_or_else(|| {
            CatalogResolveError::UnknownModelDriver {
                model_driver_id: model_driver_id.to_owned(),
            }
        })?;
        Ok(catalog
            .document
            .version_rules
            .iter()
            .zip(&catalog.compiled_version_rules)
            .filter_map(|(rule, condition)| condition.matches(context).then_some(rule))
            .collect())
    }

    pub(crate) fn matching_provider_variants(
        &self,
        provider_profile_id: &str,
        context: &MatchContext,
    ) -> Result<Vec<&ProviderVariantRule>, CatalogResolveError> {
        let catalog = self
            .provider_rules
            .get(provider_profile_id)
            .ok_or_else(|| CatalogResolveError::UnknownProviderRules {
                provider_profile_id: provider_profile_id.to_owned(),
            })?;
        Ok(catalog
            .document
            .variants
            .iter()
            .zip(&catalog.compiled_variants)
            .filter_map(|(variant, condition)| condition.matches(context).then_some(variant))
            .collect())
    }

    pub(crate) fn resolve_model(
        &self,
        origin_model_id: &str,
        candidate_driver_ids: Option<&[String]>,
        dimensions: &MatchContext,
    ) -> Result<ResolvedModelSemantics, CatalogResolveError> {
        let candidates = self.resolve_candidates(candidate_driver_ids)?;
        let exact_matches = self
            .model_exact_index
            .get(origin_model_id)
            .into_iter()
            .flatten()
            .filter(|driver_id| candidates.contains(*driver_id))
            .cloned()
            .collect::<Vec<_>>();

        if exact_matches.len() > 1 {
            return Err(CatalogResolveError::AmbiguousModelDrivers {
                origin_model_id: origin_model_id.to_owned(),
                model_driver_ids: exact_matches,
            });
        }
        if let Some(driver_id) = exact_matches.first() {
            let catalog = &self.model_drivers[driver_id];
            let position = catalog.exact_index[origin_model_id];
            let rule = &catalog.document.models[position];
            let semantics = catalog
                .document
                .defaults
                .overlay(&model_rule_semantics!(rule));
            return Ok(resolved_model(
                origin_model_id,
                driver_id,
                catalog.document.revision_seq,
                ModelMatchKind::Exact,
                None,
                semantics,
            ));
        }

        let mut context = dimensions.clone();
        context.insert(
            "origin_model_id".to_owned(),
            Value::String(origin_model_id.to_owned()),
        );
        let mut pattern_matches = Vec::new();
        for driver_id in &candidates {
            let catalog = &self.model_drivers[driver_id];
            if let Some(trace) = catalog.patterns.first_match(&context) {
                pattern_matches.push((driver_id.clone(), trace));
            }
        }

        if pattern_matches.len() > 1 {
            return Err(CatalogResolveError::AmbiguousModelDrivers {
                origin_model_id: origin_model_id.to_owned(),
                model_driver_ids: pattern_matches
                    .into_iter()
                    .map(|(driver_id, _)| driver_id)
                    .collect(),
            });
        }
        if let Some((driver_id, trace)) = pattern_matches.pop() {
            let catalog = &self.model_drivers[&driver_id];
            let rule = &catalog.document.patterns[trace.position];
            let semantics = catalog
                .document
                .defaults
                .overlay(&model_rule_semantics!(rule));
            return Ok(resolved_model(
                origin_model_id,
                &driver_id,
                catalog.document.revision_seq,
                ModelMatchKind::Pattern,
                Some(trace),
                semantics,
            ));
        }

        if candidates.len() == 1 {
            let driver_id = &candidates[0];
            let catalog = &self.model_drivers[driver_id];
            return Ok(resolved_model(
                origin_model_id,
                driver_id,
                catalog.document.revision_seq,
                ModelMatchKind::Defaults,
                None,
                catalog.document.defaults.clone(),
            ));
        }

        Ok(ResolvedModelSemantics {
            origin_model_id: origin_model_id.to_owned(),
            source_model_driver_id: None,
            model_driver_id: None,
            catalog_revision_seq: None,
            match_kind: ModelMatchKind::ConservativeFallback,
            trace: None,
            semantics: ModelSemantics::conservative(),
        })
    }

    pub(crate) fn resolve_provider_rule(
        &self,
        provider_profile_id: &str,
        provider_model_id: &str,
        dimensions: &MatchContext,
    ) -> Result<Option<ResolvedProviderRule>, CatalogResolveError> {
        let catalog = self
            .provider_rules
            .get(provider_profile_id)
            .ok_or_else(|| CatalogResolveError::UnknownProviderRules {
                provider_profile_id: provider_profile_id.to_owned(),
            })?;
        if let Some(position) = catalog.exact_index.get(provider_model_id) {
            let rule = &catalog.document.models[*position];
            return Ok(Some(ResolvedProviderRule {
                provider_profile_id: provider_profile_id.to_owned(),
                catalog_revision_seq: catalog.document.revision_seq,
                match_kind: ProviderRuleMatchKind::Exact,
                trace: None,
                action: provider_rule_action!(rule),
                compiled: catalog.exact_compiled[*position].clone(),
            }));
        }

        let mut context = dimensions.clone();
        context.insert(
            "provider_model_id".to_owned(),
            Value::String(provider_model_id.to_owned()),
        );
        let Some(trace) = catalog.patterns.first_match(&context) else {
            return Ok(None);
        };
        let position = trace.position;
        let rule = &catalog.document.patterns[position];
        Ok(Some(ResolvedProviderRule {
            provider_profile_id: provider_profile_id.to_owned(),
            catalog_revision_seq: catalog.document.revision_seq,
            match_kind: ProviderRuleMatchKind::Pattern,
            trace: Some(trace),
            action: provider_rule_action!(rule),
            compiled: catalog.pattern_compiled[position].clone(),
        }))
    }

    pub(crate) fn resolve_provider_origin(
        &self,
        provider_profile_id: &str,
        provider_model_id: &str,
    ) -> Result<ResolvedProviderOrigin, CatalogResolveError> {
        let catalog = self
            .provider_rules
            .get(provider_profile_id)
            .ok_or_else(|| CatalogResolveError::UnknownProviderRules {
                provider_profile_id: provider_profile_id.to_owned(),
            })?;
        let mut resolved = Vec::new();
        for (mapping, compiled) in catalog
            .document
            .origin_mappings
            .iter()
            .zip(&catalog.origin_mappings)
        {
            let Some(origin) = apply_origin_mapping(
                provider_profile_id,
                provider_model_id,
                mapping,
                *compiled,
                &catalog.document.origin_provider_aliases,
            )?
            else {
                continue;
            };
            if !self.model_drivers.contains_key(&origin.model_driver_id) {
                return Err(CatalogResolveError::UnknownOriginProvider {
                    provider_profile_id: provider_profile_id.to_owned(),
                    origin_provider: origin.model_driver_id,
                });
            }
            if catalog
                .document
                .metadata_drivers
                .as_ref()
                .is_some_and(|drivers| !drivers.contains(&origin.model_driver_id))
            {
                return Err(CatalogResolveError::OriginDriverOutsideMetadataDrivers {
                    provider_profile_id: provider_profile_id.to_owned(),
                    model_driver_id: origin.model_driver_id,
                });
            }
            if !resolved.contains(&origin) {
                resolved.push(origin);
            }
        }
        match resolved.len() {
            0 => Err(CatalogResolveError::OriginMappingNotFound {
                provider_profile_id: provider_profile_id.to_owned(),
                provider_model_id: provider_model_id.to_owned(),
            }),
            1 => Ok(resolved.pop().expect("one resolved origin")),
            _ => Err(CatalogResolveError::ConflictingOriginMappings {
                provider_profile_id: provider_profile_id.to_owned(),
                provider_model_id: provider_model_id.to_owned(),
                resolved,
            }),
        }
    }

    fn resolve_candidates(
        &self,
        requested: Option<&[String]>,
    ) -> Result<Vec<String>, CatalogResolveError> {
        match requested {
            Some(requested) => {
                let mut candidates = requested
                    .iter()
                    .map(|id| {
                        self.model_drivers
                            .contains_key(id)
                            .then(|| id.clone())
                            .ok_or_else(|| CatalogResolveError::UnknownModelDriver {
                                model_driver_id: id.clone(),
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                candidates.sort();
                candidates.dedup();
                Ok(candidates)
            }
            None => Ok(self.model_drivers.keys().cloned().collect()),
        }
    }
}

fn resolved_model(
    origin_model_id: &str,
    source_driver_id: &str,
    revision_seq: u64,
    match_kind: ModelMatchKind,
    trace: Option<MatchTrace>,
    semantics: ModelSemantics,
) -> ResolvedModelSemantics {
    let model_driver_id = semantics
        .model_driver
        .clone()
        .unwrap_or_else(|| source_driver_id.to_owned());
    ResolvedModelSemantics {
        origin_model_id: origin_model_id.to_owned(),
        source_model_driver_id: Some(source_driver_id.to_owned()),
        model_driver_id: Some(model_driver_id),
        catalog_revision_seq: Some(revision_seq),
        match_kind,
        trace,
        semantics,
    }
}

fn compile_model_driver(
    document: ModelDriverCatalog,
) -> Result<CompiledModelDriverCatalog, CatalogBuildError> {
    let mut exact_index = BTreeMap::new();
    for (position, rule) in document.models.iter().enumerate() {
        if exact_index.insert(rule.id.clone(), position).is_some() {
            return Err(CatalogBuildError::DuplicateExactRule {
                kind: CatalogKind::ModelDriver,
                catalog_id: document.model_driver_id.clone(),
                model_id: rule.id.clone(),
            });
        }
    }
    let patterns = CompiledRuleSet::compile(
        document.patterns.iter().map(|rule| RuleEntry {
            rule_id: None,
            rule: rule.match_rule.clone(),
        }),
        &MODEL_DRIVER_MATCH_SCHEMA,
    )?;
    let compiled_variants = document
        .variants
        .iter()
        .map(|variant| {
            CompiledMatchRule::compile(variant.match_rule.clone(), &MODEL_DRIVER_MATCH_SCHEMA)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let compiled_version_rules = document
        .version_rules
        .iter()
        .map(|rule| CompiledMatchRule::compile(rule.match_rule.clone(), &MODEL_DRIVER_MATCH_SCHEMA))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CompiledModelDriverCatalog {
        document,
        exact_index,
        patterns,
        compiled_variants,
        compiled_version_rules,
    })
}

fn compile_provider_rules(
    document: ProviderRulesCatalog,
) -> Result<CompiledProviderRulesCatalog, CatalogBuildError> {
    let origin_mappings = document
        .origin_mappings
        .iter()
        .map(|mapping| compile_origin_mapping(&document.provider_profile_id, mapping))
        .collect::<Result<Vec<_>, _>>()?;
    let mut exact_index = BTreeMap::new();
    for (position, rule) in document.models.iter().enumerate() {
        if exact_index.insert(rule.id.clone(), position).is_some() {
            return Err(CatalogBuildError::DuplicateExactRule {
                kind: CatalogKind::ProviderRules,
                catalog_id: document.provider_profile_id.clone(),
                model_id: rule.id.clone(),
            });
        }
    }
    let patterns = CompiledRuleSet::compile(
        document.patterns.iter().map(|rule| RuleEntry {
            rule_id: None,
            rule: rule.match_rule.clone(),
        }),
        &PROVIDER_RULE_MATCH_SCHEMA,
    )?;
    let exact_compiled = document
        .models
        .iter()
        .map(compile_provider_rule)
        .collect::<Result<Vec<_>, _>>()?;
    let pattern_compiled = document
        .patterns
        .iter()
        .map(compile_provider_rule)
        .collect::<Result<Vec<_>, _>>()?;
    let compiled_variants = document
        .variants
        .iter()
        .map(|variant| {
            CompiledMatchRule::compile(variant.match_rule.clone(), &PROVIDER_RULE_MATCH_SCHEMA)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CompiledProviderRulesCatalog {
        document,
        origin_mappings,
        exact_index,
        patterns,
        exact_compiled,
        pattern_compiled,
        compiled_variants,
    })
}

fn compile_origin_mapping(
    owner: &str,
    mapping: &OriginMapping,
) -> Result<CompiledOriginMapping, CatalogBuildError> {
    if mapping.extract.regex != "^(?<driver>[^/]+)/(?<model>.+)$" {
        return Err(CatalogBuildError::InvalidValue {
            owner: owner.to_owned(),
            field: "origin_mappings.extract.regex",
            reason: "only the built-in vendor/model capture is supported".to_owned(),
        });
    }
    for (capture, transforms) in &mapping.transforms {
        if capture != "driver" && capture != "model" {
            return Err(CatalogBuildError::InvalidValue {
                owner: owner.to_owned(),
                field: "origin_mappings.transforms",
                reason: format!("unknown capture {capture:?}"),
            });
        }
        for transform in transforms {
            match transform.op.as_str() {
                "trim" | "lowercase" => {
                    if transform.table.is_some() || transform.on_missing.is_some() {
                        return Err(CatalogBuildError::InvalidValue {
                            owner: owner.to_owned(),
                            field: "origin_mappings.transforms",
                            reason: format!("transform {:?} does not accept options", transform.op),
                        });
                    }
                }
                "alias" => {
                    if capture != "driver"
                        || transform.table.as_deref() != Some("origin_provider_aliases")
                        || !matches!(transform.on_missing.as_deref(), None | Some("keep"))
                    {
                        return Err(CatalogBuildError::InvalidValue {
                            owner: owner.to_owned(),
                            field: "origin_mappings.transforms",
                            reason: "alias must target driver/origin_provider_aliases and on_missing may only be keep".to_owned(),
                        });
                    }
                }
                _ => {
                    return Err(CatalogBuildError::InvalidValue {
                        owner: owner.to_owned(),
                        field: "origin_mappings.transforms",
                        reason: format!("unsupported transform {:?}", transform.op),
                    });
                }
            }
        }
    }
    Ok(CompiledOriginMapping::VendorModel)
}

fn apply_origin_mapping(
    provider_profile_id: &str,
    provider_model_id: &str,
    mapping: &OriginMapping,
    compiled: CompiledOriginMapping,
    aliases: &BTreeMap<String, String>,
) -> Result<Option<ResolvedProviderOrigin>, CatalogResolveError> {
    let (mut driver, mut model) = match compiled {
        CompiledOriginMapping::VendorModel => {
            let Some((driver, model)) = provider_model_id.split_once('/') else {
                return Ok(None);
            };
            if driver.is_empty() || model.is_empty() {
                return Ok(None);
            }
            (driver.to_owned(), model.to_owned())
        }
    };
    for (capture, value) in [("driver", &mut driver), ("model", &mut model)] {
        for transform in mapping.transforms.get(capture).into_iter().flatten() {
            match transform.op.as_str() {
                "trim" => *value = value.trim().to_owned(),
                "lowercase" => *value = value.to_lowercase(),
                "alias" => {
                    if let Some(mapped) = aliases.get(value.as_str()) {
                        *value = mapped.clone();
                    } else if transform.on_missing.as_deref() != Some("keep") {
                        return Err(CatalogResolveError::UnknownOriginProvider {
                            provider_profile_id: provider_profile_id.to_owned(),
                            origin_provider: value.clone(),
                        });
                    }
                }
                _ => unreachable!("origin transforms are validated during snapshot build"),
            }
        }
    }
    if driver.is_empty() || model.is_empty() {
        return Ok(None);
    }
    Ok(Some(ResolvedProviderOrigin {
        origin_model_id: model,
        model_driver_id: driver,
    }))
}

trait ProviderRuleData {
    fn request_rules(&self) -> &[RequestRule];
    fn pricing(&self) -> Option<&Pricing>;
}

impl ProviderRuleData for ProviderExactRule {
    fn request_rules(&self) -> &[RequestRule] {
        &self.request_rules
    }

    fn pricing(&self) -> Option<&Pricing> {
        self.pricing.as_ref()
    }
}

impl ProviderRuleData for ProviderPatternRule {
    fn request_rules(&self) -> &[RequestRule] {
        &self.request_rules
    }

    fn pricing(&self) -> Option<&Pricing> {
        self.pricing.as_ref()
    }
}

fn compile_provider_rule(
    rule: &impl ProviderRuleData,
) -> Result<CompiledProviderRule, MatchCompileError> {
    let request_conditions = rule
        .request_rules()
        .iter()
        .map(|rule| {
            rule.when
                .clone()
                .map(|condition| CompiledMatchRule::compile(condition, &REQUEST_RULE_MATCH_SCHEMA))
                .transpose()
        })
        .collect::<Result<Vec<_>, _>>()?;
    let pricing_rules = rule
        .pricing()
        .into_iter()
        .flat_map(|pricing| &pricing.rules)
        .map(|rule| CompiledMatchRule::compile(rule.when.clone(), &PRICING_RULE_MATCH_SCHEMA))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CompiledProviderRule {
        request_conditions,
        pricing_rules,
    })
}

fn validate_model_driver(
    catalog: &ModelDriverCatalog,
    options: &CatalogBuildOptions,
) -> Result<(), CatalogBuildError> {
    validate_header(
        CatalogKind::ModelDriver,
        &catalog.model_driver_id,
        &catalog.format,
        MODEL_DRIVER_FORMAT,
        catalog.schema_version,
        catalog.schema_revision,
    )?;
    validate_required_features(
        &catalog.model_driver_id,
        &catalog.required_features,
        options,
    )?;
    validate_nonempty_strings(
        CatalogKind::ModelDriver,
        &catalog.model_driver_id,
        "models.id",
        catalog.models.iter().map(|rule| rule.id.as_str()),
    )?;
    validate_model_semantics(&catalog.model_driver_id, &catalog.defaults)?;
    for rule in &catalog.models {
        validate_model_semantics(&catalog.model_driver_id, &model_rule_semantics!(rule))?;
    }
    for rule in &catalog.patterns {
        validate_model_semantics(&catalog.model_driver_id, &model_rule_semantics!(rule))?;
    }
    validate_unique_nonempty(
        CatalogKind::ModelDriver,
        &catalog.model_driver_id,
        "variants.name",
        catalog.variants.iter().map(|variant| variant.name.as_str()),
    )?;
    validate_unique_nonempty(
        CatalogKind::ModelDriver,
        &catalog.model_driver_id,
        "version_rules.id",
        catalog.version_rules.iter().map(|rule| rule.id.as_str()),
    )?;
    let version_rule_ids = catalog
        .version_rules
        .iter()
        .map(|rule| rule.id.as_str())
        .collect::<BTreeSet<_>>();
    for semantics in catalog
        .models
        .iter()
        .map(|rule| model_rule_semantics!(rule))
        .chain(
            catalog
                .patterns
                .iter()
                .map(|rule| model_rule_semantics!(rule)),
        )
        .chain(std::iter::once(catalog.defaults.clone()))
    {
        if let Some(references) = semantics.version_rules {
            for reference in references {
                if !version_rule_ids.contains(reference.as_str()) {
                    return Err(CatalogBuildError::UnknownReference {
                        owner: catalog.model_driver_id.clone(),
                        field: "version_rules",
                        target: reference,
                    });
                }
            }
        }
    }
    for rule in &catalog.version_rules {
        validate_nonempty_field(
            CatalogKind::ModelDriver,
            &catalog.model_driver_id,
            "version_rules.family",
            &rule.family,
        )?;
        validate_nonempty_field(
            CatalogKind::ModelDriver,
            &catalog.model_driver_id,
            "version_rules.tier",
            &rule.tier,
        )?;
        validate_nonempty_field(
            CatalogKind::ModelDriver,
            &catalog.model_driver_id,
            "version_rules.current_mount",
            &rule.current_mount,
        )?;
        validate_nonempty_field(
            CatalogKind::ModelDriver,
            &catalog.model_driver_id,
            "version_rules.version_mount",
            &rule.version_mount,
        )?;
    }
    Ok(())
}

fn validate_model_semantics(
    owner: &str,
    semantics: &ModelSemantics,
) -> Result<(), CatalogBuildError> {
    if let Some(score) = semantics.quality_score {
        if !score.is_finite() || !(0.0..=1.0).contains(&score) {
            return Err(CatalogBuildError::InvalidValue {
                owner: owner.to_owned(),
                field: "quality_score",
                reason: "must be a finite number between 0 and 1".to_owned(),
            });
        }
    }
    if let Some(capabilities) = &semantics.capabilities {
        for forbidden in ["availability", "deprecated", "remote_methods", "health"] {
            if capabilities.contains_key(forbidden) {
                return Err(CatalogBuildError::StaticDynamicBoundary {
                    owner: owner.to_owned(),
                    field: format!("capabilities.{forbidden}"),
                });
            }
        }
    }
    if let Some(pricing) = &semantics.pricing {
        validate_pricing(owner, pricing)?;
        if !pricing.rules.is_empty() {
            return Err(CatalogBuildError::InvalidValue {
                owner: owner.to_owned(),
                field: "pricing.rules",
                reason: "conditional channel pricing belongs to Provider Rules".to_owned(),
            });
        }
    }
    Ok(())
}

fn validate_provider_rules(catalog: &ProviderRulesCatalog) -> Result<(), CatalogBuildError> {
    validate_header(
        CatalogKind::ProviderRules,
        &catalog.provider_profile_id,
        &catalog.format,
        PROVIDER_RULES_FORMAT,
        catalog.schema_version,
        catalog.schema_revision,
    )?;
    if let Some(drivers) = &catalog.metadata_drivers {
        validate_unique_nonempty(
            CatalogKind::ProviderRules,
            &catalog.provider_profile_id,
            "metadata_drivers",
            drivers.iter().map(String::as_str),
        )?;
    }
    for (alias, driver) in &catalog.origin_provider_aliases {
        validate_nonempty_field(
            CatalogKind::ProviderRules,
            &catalog.provider_profile_id,
            "origin_provider_aliases.key",
            alias,
        )?;
        validate_nonempty_field(
            CatalogKind::ProviderRules,
            &catalog.provider_profile_id,
            "origin_provider_aliases.value",
            driver,
        )?;
    }
    for mapping in &catalog.origin_mappings {
        if mapping.extract.source != "provider_model_id" {
            return Err(CatalogBuildError::InvalidValue {
                owner: catalog.provider_profile_id.clone(),
                field: "origin_mappings.extract.source",
                reason: "must be provider_model_id".to_owned(),
            });
        }
        validate_nonempty_field(
            CatalogKind::ProviderRules,
            &catalog.provider_profile_id,
            "origin_mappings.extract.regex",
            &mapping.extract.regex,
        )?;
    }
    validate_nonempty_strings(
        CatalogKind::ProviderRules,
        &catalog.provider_profile_id,
        "models.id",
        catalog.models.iter().map(|rule| rule.id.as_str()),
    )?;
    for rule in &catalog.models {
        validate_provider_rule_data(&catalog.provider_profile_id, rule)?;
    }
    for rule in &catalog.patterns {
        validate_provider_rule_data(&catalog.provider_profile_id, rule)?;
    }
    for variant in &catalog.variants {
        validate_nonempty_field(
            CatalogKind::ProviderRules,
            &catalog.provider_profile_id,
            "variants.model_driver",
            &variant.model_driver,
        )?;
        validate_nonempty_field(
            CatalogKind::ProviderRules,
            &catalog.provider_profile_id,
            "variants.variant",
            &variant.variant,
        )?;
    }
    Ok(())
}

trait ProviderRuleValidation {
    fn operations(&self) -> &BTreeMap<String, String>;
    fn request_rules(&self) -> &[RequestRule];
    fn pricing(&self) -> Option<&Pricing>;
    fn remove_api_types(&self) -> &BTreeSet<String>;
    fn remove_features(&self) -> &BTreeSet<String>;
}

macro_rules! impl_provider_rule_validation {
    ($type:ty) => {
        impl ProviderRuleValidation for $type {
            fn operations(&self) -> &BTreeMap<String, String> {
                &self.operations
            }
            fn request_rules(&self) -> &[RequestRule] {
                &self.request_rules
            }
            fn pricing(&self) -> Option<&Pricing> {
                self.pricing.as_ref()
            }
            fn remove_api_types(&self) -> &BTreeSet<String> {
                &self.remove_api_types
            }
            fn remove_features(&self) -> &BTreeSet<String> {
                &self.remove_features
            }
        }
    };
}

impl_provider_rule_validation!(ProviderExactRule);
impl_provider_rule_validation!(ProviderPatternRule);

fn validate_provider_rule_data(
    owner: &str,
    rule: &impl ProviderRuleValidation,
) -> Result<(), CatalogBuildError> {
    for (key, value) in rule.operations() {
        if key.trim().is_empty() || value.trim().is_empty() {
            return Err(CatalogBuildError::InvalidValue {
                owner: owner.to_owned(),
                field: "operations",
                reason: "keys and operation names must be non-empty".to_owned(),
            });
        }
    }
    for request_rule in rule.request_rules() {
        for pointer in &request_rule.remove {
            if !valid_json_pointer(pointer) {
                return Err(CatalogBuildError::InvalidValue {
                    owner: owner.to_owned(),
                    field: "request_rules.remove",
                    reason: format!("invalid normalized JSON Pointer {pointer:?}"),
                });
            }
        }
    }
    if let Some(pricing) = rule.pricing() {
        validate_pricing(owner, pricing)?;
    }
    for value in rule.remove_api_types().iter().chain(rule.remove_features()) {
        if value.trim().is_empty() {
            return Err(CatalogBuildError::InvalidValue {
                owner: owner.to_owned(),
                field: "remove_api_types/remove_features",
                reason: "entries must be non-empty".to_owned(),
            });
        }
    }
    Ok(())
}

fn validate_pricing(owner: &str, pricing: &Pricing) -> Result<(), CatalogBuildError> {
    if pricing.currency.trim().is_empty() {
        return Err(CatalogBuildError::InvalidValue {
            owner: owner.to_owned(),
            field: "pricing.currency",
            reason: "must be non-empty".to_owned(),
        });
    }
    for (field, amount) in [
        ("input_token", pricing.input_token),
        ("output_token", pricing.output_token),
        ("cache_input_token", pricing.cache_input_token),
        ("estimated_cost", pricing.estimated_cost),
        ("amount", pricing.amount),
    ] {
        if amount.is_some_and(|amount| !amount.is_finite() || amount < 0.0) {
            return Err(CatalogBuildError::InvalidValue {
                owner: owner.to_owned(),
                field: "pricing",
                reason: format!("{field} must be finite and non-negative"),
            });
        }
    }
    for rule in &pricing.rules {
        if !rule.amount.is_finite() || rule.amount < 0.0 {
            return Err(CatalogBuildError::InvalidValue {
                owner: owner.to_owned(),
                field: "pricing.rules.amount",
                reason: "must be finite and non-negative".to_owned(),
            });
        }
    }
    Ok(())
}

fn validate_known_provider_catalog(
    catalog: &KnownProviderCatalog,
) -> Result<(), CatalogBuildError> {
    validate_header(
        CatalogKind::KnownProvider,
        &catalog.catalog_id,
        &catalog.format,
        KNOWN_PROVIDER_FORMAT,
        catalog.schema_version,
        catalog.schema_revision,
    )?;
    let mut profiles = BTreeSet::new();
    for provider in &catalog.providers {
        for (field, value) in [
            (
                "providers.provider_profile_id",
                &provider.provider_profile_id,
            ),
            ("providers.display_name", &provider.display_name),
            ("providers.base_url", &provider.base_url),
            (
                "providers.protocol_adapter_id",
                &provider.protocol_adapter_id,
            ),
        ] {
            validate_nonempty_field(
                CatalogKind::KnownProvider,
                &catalog.catalog_id,
                field,
                value,
            )?;
        }
        if !provider.base_url.starts_with("https://") && !provider.base_url.starts_with("http://") {
            return Err(CatalogBuildError::InvalidValue {
                owner: catalog.catalog_id.clone(),
                field: "providers.base_url",
                reason: "must use an http or https URL".to_owned(),
            });
        }
        if !profiles.insert(provider.provider_profile_id.as_str()) {
            return Err(CatalogBuildError::DuplicateKnownProvider {
                provider_profile_id: provider.provider_profile_id.clone(),
            });
        }
    }
    Ok(())
}

fn validate_header(
    kind: CatalogKind,
    id: &str,
    format: &str,
    expected_format: &'static str,
    schema_version: u32,
    schema_revision: u32,
) -> Result<(), CatalogBuildError> {
    validate_nonempty_field(kind, id, "catalog identity", id)?;
    if format != expected_format {
        return Err(CatalogBuildError::InvalidFormat {
            kind,
            id: id.to_owned(),
            expected: expected_format,
            actual: format.to_owned(),
        });
    }
    if schema_version != SUPPORTED_SCHEMA_VERSION || schema_revision > SUPPORTED_SCHEMA_REVISION {
        return Err(CatalogBuildError::UnsupportedSchema {
            kind,
            id: id.to_owned(),
            schema_version,
            schema_revision,
        });
    }
    Ok(())
}

fn validate_required_features(
    owner: &str,
    required: &[String],
    options: &CatalogBuildOptions,
) -> Result<(), CatalogBuildError> {
    let mut seen = BTreeSet::new();
    for feature in required {
        if feature.trim().is_empty() || !seen.insert(feature) {
            return Err(CatalogBuildError::InvalidValue {
                owner: owner.to_owned(),
                field: "required_features",
                reason: "features must be unique and non-empty".to_owned(),
            });
        }
        if !options.supported_features.contains(feature) {
            return Err(CatalogBuildError::UnsupportedFeature {
                owner: owner.to_owned(),
                feature: feature.clone(),
            });
        }
    }
    Ok(())
}

fn validate_references(
    model_drivers: &BTreeMap<String, CompiledModelDriverCatalog>,
    provider_rules: &BTreeMap<String, CompiledProviderRulesCatalog>,
    known_providers: &BTreeMap<String, KnownProviderCatalog>,
) -> Result<(), CatalogBuildError> {
    for (owner, catalog) in model_drivers {
        for target in catalog
            .document
            .models
            .iter()
            .filter_map(|rule| rule.model_driver.as_ref())
            .chain(
                catalog
                    .document
                    .patterns
                    .iter()
                    .filter_map(|rule| rule.model_driver.as_ref()),
            )
            .chain(catalog.document.defaults.model_driver.as_ref())
        {
            require_model_driver(model_drivers, owner, "model_driver", target)?;
        }
    }
    for (owner, catalog) in provider_rules {
        if let Some(drivers) = &catalog.document.metadata_drivers {
            for target in drivers {
                require_model_driver(model_drivers, owner, "metadata_drivers", target)?;
            }
        }
        for target in catalog.document.origin_provider_aliases.values() {
            require_model_driver(model_drivers, owner, "origin_provider_aliases", target)?;
            if catalog
                .document
                .metadata_drivers
                .as_ref()
                .is_some_and(|drivers| !drivers.contains(target))
            {
                return Err(CatalogBuildError::InvalidValue {
                    owner: owner.clone(),
                    field: "origin_provider_aliases",
                    reason: format!(
                        "Model Driver {target:?} is outside the provider's metadata_drivers"
                    ),
                });
            }
        }
        for variant in &catalog.document.variants {
            require_model_driver(
                model_drivers,
                owner,
                "variants.model_driver",
                &variant.model_driver,
            )?;
        }
    }
    for catalog in known_providers.values() {
        for provider in &catalog.providers {
            if let Some(rules_id) = &provider.provider_rules_id {
                let rules = provider_rules.get(rules_id).ok_or_else(|| {
                    CatalogBuildError::UnknownReference {
                        owner: catalog.catalog_id.clone(),
                        field: "providers.provider_rules_id",
                        target: rules_id.clone(),
                    }
                })?;
                if rules.document.provider_profile_id != provider.provider_profile_id {
                    return Err(CatalogBuildError::ReferenceMismatch {
                        owner: catalog.catalog_id.clone(),
                        field: "providers.provider_rules_id",
                        target: rules_id.clone(),
                        expected: provider.provider_profile_id.clone(),
                    });
                }
            }
        }
    }
    Ok(())
}

fn validate_revisions(
    target_revision_seq: u64,
    model_drivers: &BTreeMap<String, CompiledModelDriverCatalog>,
    provider_rules: &BTreeMap<String, CompiledProviderRulesCatalog>,
    known_providers: &BTreeMap<String, KnownProviderCatalog>,
) -> Result<(), CatalogBuildError> {
    for (kind, id, revision_seq) in model_drivers
        .iter()
        .map(|(id, catalog)| (CatalogKind::ModelDriver, id, catalog.document.revision_seq))
        .chain(provider_rules.iter().map(|(id, catalog)| {
            (
                CatalogKind::ProviderRules,
                id,
                catalog.document.revision_seq,
            )
        }))
        .chain(
            known_providers
                .iter()
                .map(|(id, catalog)| (CatalogKind::KnownProvider, id, catalog.revision_seq)),
        )
    {
        if revision_seq > target_revision_seq {
            return Err(CatalogBuildError::RevisionAheadOfSnapshot {
                kind,
                id: id.clone(),
                revision_seq,
                target_revision_seq,
            });
        }
    }
    Ok(())
}

fn require_model_driver(
    model_drivers: &BTreeMap<String, CompiledModelDriverCatalog>,
    owner: &str,
    field: &'static str,
    target: &str,
) -> Result<(), CatalogBuildError> {
    if model_drivers.contains_key(target) {
        Ok(())
    } else {
        Err(CatalogBuildError::UnknownReference {
            owner: owner.to_owned(),
            field,
            target: target.to_owned(),
        })
    }
}

fn validate_nonempty_strings<'a>(
    kind: CatalogKind,
    owner: &str,
    field: &'static str,
    values: impl IntoIterator<Item = &'a str>,
) -> Result<(), CatalogBuildError> {
    for value in values {
        validate_nonempty_field(kind, owner, field, value)?;
    }
    Ok(())
}

fn validate_unique_nonempty<'a>(
    kind: CatalogKind,
    owner: &str,
    field: &'static str,
    values: impl IntoIterator<Item = &'a str>,
) -> Result<(), CatalogBuildError> {
    let mut seen = BTreeSet::new();
    for value in values {
        validate_nonempty_field(kind, owner, field, value)?;
        if !seen.insert(value) {
            return Err(CatalogBuildError::InvalidValue {
                owner: owner.to_owned(),
                field,
                reason: format!("duplicate value {value:?}"),
            });
        }
    }
    Ok(())
}

fn validate_nonempty_field(
    _kind: CatalogKind,
    owner: &str,
    field: &'static str,
    value: &str,
) -> Result<(), CatalogBuildError> {
    if value.trim().is_empty() {
        Err(CatalogBuildError::InvalidValue {
            owner: owner.to_owned(),
            field,
            reason: "must be non-empty".to_owned(),
        })
    } else {
        Ok(())
    }
}

fn valid_json_pointer(value: &str) -> bool {
    value.starts_with('/')
        && value.bytes().enumerate().all(|(position, byte)| {
            byte != b'~' || matches!(value.as_bytes().get(position + 1), Some(b'0' | b'1'))
        })
}

#[derive(Debug)]
pub(crate) enum CatalogBuildError {
    InvalidJson {
        kind: CatalogKind,
        position: usize,
        source: serde_json::Error,
    },
    InvalidFormat {
        kind: CatalogKind,
        id: String,
        expected: &'static str,
        actual: String,
    },
    UnsupportedSchema {
        kind: CatalogKind,
        id: String,
        schema_version: u32,
        schema_revision: u32,
    },
    UnsupportedFeature {
        owner: String,
        feature: String,
    },
    RevisionAheadOfSnapshot {
        kind: CatalogKind,
        id: String,
        revision_seq: u64,
        target_revision_seq: u64,
    },
    DuplicateCatalog {
        kind: CatalogKind,
        id: String,
    },
    DuplicateExactRule {
        kind: CatalogKind,
        catalog_id: String,
        model_id: String,
    },
    DuplicateKnownProvider {
        provider_profile_id: String,
    },
    UnknownReference {
        owner: String,
        field: &'static str,
        target: String,
    },
    ReferenceMismatch {
        owner: String,
        field: &'static str,
        target: String,
        expected: String,
    },
    InvalidValue {
        owner: String,
        field: &'static str,
        reason: String,
    },
    StaticDynamicBoundary {
        owner: String,
        field: String,
    },
    Match(MatchCompileError),
}

impl From<MatchCompileError> for CatalogBuildError {
    fn from(value: MatchCompileError) -> Self {
        Self::Match(value)
    }
}

impl fmt::Display for CatalogBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson {
                kind,
                position,
                source,
            } => write!(
                formatter,
                "invalid {kind} catalog JSON at file position {position}: {source}"
            ),
            Self::InvalidFormat {
                kind,
                id,
                expected,
                actual,
            } => write!(
                formatter,
                "invalid {kind} catalog {id:?} format: expected {expected:?}, got {actual:?}"
            ),
            Self::UnsupportedSchema {
                kind,
                id,
                schema_version,
                schema_revision,
            } => write!(
                formatter,
                "unsupported {kind} catalog {id:?} schema {schema_version}.{schema_revision}"
            ),
            Self::UnsupportedFeature { owner, feature } => {
                write!(formatter, "catalog {owner:?} requires unsupported feature {feature:?}")
            }
            Self::RevisionAheadOfSnapshot {
                kind,
                id,
                revision_seq,
                target_revision_seq,
            } => write!(
                formatter,
                "{kind} catalog {id:?} revision {revision_seq} is ahead of snapshot target {target_revision_seq}"
            ),
            Self::DuplicateCatalog { kind, id } => {
                write!(formatter, "duplicate {kind} catalog identity {id:?}")
            }
            Self::DuplicateExactRule {
                kind,
                catalog_id,
                model_id,
            } => write!(
                formatter,
                "duplicate exact model {model_id:?} in {kind} catalog {catalog_id:?}"
            ),
            Self::DuplicateKnownProvider {
                provider_profile_id,
            } => write!(
                formatter,
                "duplicate known provider profile {provider_profile_id:?}"
            ),
            Self::UnknownReference {
                owner,
                field,
                target,
            } => write!(
                formatter,
                "catalog {owner:?} field {field} references unknown identity {target:?}"
            ),
            Self::ReferenceMismatch {
                owner,
                field,
                target,
                expected,
            } => write!(
                formatter,
                "catalog {owner:?} field {field} reference {target:?} does not belong to {expected:?}"
            ),
            Self::InvalidValue {
                owner,
                field,
                reason,
            } => write!(
                formatter,
                "catalog {owner:?} has invalid {field}: {reason}"
            ),
            Self::StaticDynamicBoundary { owner, field } => write!(
                formatter,
                "catalog {owner:?} contains dynamic discovery fact in static field {field}"
            ),
            Self::Match(source) => write!(formatter, "catalog match rule failed to compile: {source}"),
        }
    }
}

impl Error for CatalogBuildError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidJson { source, .. } => Some(source),
            Self::Match(source) => Some(source),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CatalogResolveError {
    UnknownModelDriver {
        model_driver_id: String,
    },
    UnknownProviderRules {
        provider_profile_id: String,
    },
    AmbiguousModelDrivers {
        origin_model_id: String,
        model_driver_ids: Vec<String>,
    },
    OriginMappingNotFound {
        provider_profile_id: String,
        provider_model_id: String,
    },
    UnknownOriginProvider {
        provider_profile_id: String,
        origin_provider: String,
    },
    OriginDriverOutsideMetadataDrivers {
        provider_profile_id: String,
        model_driver_id: String,
    },
    ConflictingOriginMappings {
        provider_profile_id: String,
        provider_model_id: String,
        resolved: Vec<ResolvedProviderOrigin>,
    },
}

impl fmt::Display for CatalogResolveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownModelDriver { model_driver_id } => {
                write!(formatter, "unknown model driver {model_driver_id:?}")
            }
            Self::UnknownProviderRules {
                provider_profile_id,
            } => write!(
                formatter,
                "unknown Provider Rules catalog {provider_profile_id:?}"
            ),
            Self::AmbiguousModelDrivers {
                origin_model_id,
                model_driver_ids,
            } => write!(
                formatter,
                "origin model {origin_model_id:?} matches multiple Model Drivers: {}",
                model_driver_ids.join(", ")
            ),
            Self::OriginMappingNotFound {
                provider_profile_id,
                provider_model_id,
            } => write!(
                formatter,
                "Provider Rules {provider_profile_id:?} cannot map provider model {provider_model_id:?} to an origin"
            ),
            Self::UnknownOriginProvider {
                provider_profile_id,
                origin_provider,
            } => write!(
                formatter,
                "Provider Rules {provider_profile_id:?} resolved unknown origin provider {origin_provider:?}"
            ),
            Self::OriginDriverOutsideMetadataDrivers {
                provider_profile_id,
                model_driver_id,
            } => write!(
                formatter,
                "Provider Rules {provider_profile_id:?} resolved Model Driver {model_driver_id:?} outside metadata_drivers"
            ),
            Self::ConflictingOriginMappings {
                provider_profile_id,
                provider_model_id,
                resolved,
            } => write!(
                formatter,
                "Provider Rules {provider_profile_id:?} has conflicting origin mappings for {provider_model_id:?}: {}",
                resolved
                    .iter()
                    .map(|origin| format!("{}/{}", origin.model_driver_id, origin.origin_model_id))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

impl Error for CatalogResolveError {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn file(kind: CatalogKind, value: Value) -> CurrentCatalogFile {
        CurrentCatalogFile {
            kind,
            contents: serde_json::to_vec(&value).unwrap(),
        }
    }

    fn model_driver(id: &str, models: Value, patterns: Value) -> Value {
        json!({
            "format": MODEL_DRIVER_FORMAT,
            "schema_version": 1,
            "schema_revision": 0,
            "model_driver_id": id,
            "revision_seq": 7,
            "required_features": [],
            "models": models,
            "patterns": patterns,
            "defaults": {
                "api_types": ["llm"],
                "capabilities": {"streaming": true}
            },
            "variants": [],
            "version_rules": []
        })
    }

    fn provider_rules() -> Value {
        json!({
            "format": PROVIDER_RULES_FORMAT,
            "schema_version": 1,
            "schema_revision": 0,
            "revision_seq": 9,
            "provider_profile_id": "openai",
            "metadata_drivers": ["openai"],
            "origin_provider_aliases": {"openai": "openai"},
            "origin_mappings": [],
            "models": [{
                "id": "gpt-special",
                "exclude": true
            }],
            "patterns": [{
                "match": "gpt-*",
                "operations": {"llm": "responses.create"},
                "request_rules": [{
                    "when": {"/quality": "high"},
                    "remove": ["/temperature"]
                }],
                "pricing": {
                    "currency": "USD",
                    "unit": "request",
                    "amount": 1.0,
                    "rules": [{
                        "when": {"/quality": "high"},
                        "amount": 2.0
                    }]
                },
                "remove_api_types": ["image.txt2img"],
                "remove_features": ["tool_call"]
            }, {
                "match": "*",
                "exclude": true
            }],
            "variants": [{
                "model_driver": "openai",
                "variant": "reasoning.high",
                "match": "gpt-*",
                "provider_options": {"reasoning": {"effort": "high"}}
            }]
        })
    }

    fn routed_provider_rules(
        metadata_drivers: Option<Vec<&str>>,
        aliases: Value,
        mappings: Value,
    ) -> Value {
        json!({
            "format": PROVIDER_RULES_FORMAT,
            "schema_version": 1,
            "schema_revision": 0,
            "revision_seq": 9,
            "provider_profile_id": "router",
            "metadata_drivers": metadata_drivers,
            "origin_provider_aliases": aliases,
            "origin_mappings": mappings,
            "models": [],
            "patterns": [],
            "variants": []
        })
    }

    fn vendor_model_mapping(driver_transforms: Value, model_transforms: Value) -> Value {
        json!({
            "extract": {
                "source": "provider_model_id",
                "regex": "^(?<driver>[^/]+)/(?<model>.+)$"
            },
            "transforms": {
                "driver": driver_transforms,
                "model": model_transforms
            }
        })
    }

    fn known_providers() -> Value {
        json!({
            "format": KNOWN_PROVIDER_FORMAT,
            "schema_version": 1,
            "schema_revision": 0,
            "revision_seq": 11,
            "catalog_id": "builtin",
            "providers": [{
                "provider_profile_id": "openai",
                "display_name": "OpenAI",
                "base_url": "https://api.openai.com",
                "protocol_adapter_id": "openai-responses",
                "provider_rules_id": "openai",
                "ui_hints": {"credential_label": "API key"}
            }]
        })
    }

    fn complete_files() -> Vec<CurrentCatalogFile> {
        vec![
            file(CatalogKind::KnownProvider, known_providers()),
            file(CatalogKind::ProviderRules, provider_rules()),
            file(
                CatalogKind::ModelDriver,
                model_driver(
                    "openai",
                    json!([{
                        "id": "gpt-special",
                        "api_types": ["llm", "image.txt2img"],
                        "capabilities": {"streaming": true, "tool_call": true}
                    }]),
                    json!([{
                        "match": "gpt-*",
                        "quality_score": 0.8
                    }, {
                        "match": "gpt-5-*",
                        "quality_score": 0.9
                    }]),
                ),
            ),
        ]
    }

    fn build(files: Vec<CurrentCatalogFile>) -> Result<CatalogSnapshot, CatalogBuildError> {
        CatalogSnapshot::from_current_files(42, files, &CatalogBuildOptions::default())
    }

    #[test]
    fn current_file_set_builds_immutable_indexes_and_deterministic_snapshot() {
        let first = build(complete_files()).unwrap();
        let mut reversed_files = complete_files();
        reversed_files.reverse();
        let second = build(reversed_files).unwrap();

        assert_eq!(first.target_revision_seq(), 42);
        assert_eq!(
            first.model_driver("openai").unwrap().revision_seq,
            second.model_driver("openai").unwrap().revision_seq
        );
        assert_eq!(
            first.known_provider("openai"),
            second.known_provider("openai")
        );

        let exact = first
            .resolve_model("gpt-special", None, &MatchContext::new())
            .unwrap();
        assert_eq!(exact.match_kind, ModelMatchKind::Exact);
        assert_eq!(exact.model_driver_id.as_deref(), Some("openai"));
        assert_eq!(
            exact.semantics.api_types.unwrap(),
            BTreeSet::from(["image.txt2img".to_owned(), "llm".to_owned()])
        );

        let candidates = vec!["openai".to_owned()];
        let pattern = first
            .resolve_model("gpt-5-new", Some(&candidates), &MatchContext::new())
            .unwrap();
        assert_eq!(pattern.match_kind, ModelMatchKind::Pattern);
        assert_eq!(pattern.trace.as_ref().unwrap().position, 0);
        assert_eq!(pattern.semantics.quality_score, Some(0.8));
        assert_eq!(
            pattern,
            second
                .resolve_model("gpt-5-new", Some(&candidates), &MatchContext::new(),)
                .unwrap()
        );

        let defaults = first
            .resolve_model("unknown", Some(&candidates), &MatchContext::new())
            .unwrap();
        assert_eq!(defaults.match_kind, ModelMatchKind::Defaults);
        assert_eq!(
            defaults.semantics.capabilities.unwrap()["streaming"],
            json!(true)
        );

        let no_candidates = Vec::new();
        let fallback = first
            .resolve_model("unknown", Some(&no_candidates), &MatchContext::new())
            .unwrap();
        assert_eq!(fallback.match_kind, ModelMatchKind::ConservativeFallback);
        assert!(fallback.model_driver_id.is_none());
        assert!(fallback.semantics.api_types.unwrap().is_empty());
        assert!(fallback.semantics.capabilities.unwrap().is_empty());
    }

    #[test]
    fn known_provider_enumeration_is_read_only_and_sorted_by_profile_id() {
        let catalog = |catalog_id: &str, provider_profile_id: &str| {
            json!({
                "format": KNOWN_PROVIDER_FORMAT,
                "schema_version": 1,
                "schema_revision": 0,
                "revision_seq": 11,
                "catalog_id": catalog_id,
                "providers": [{
                    "provider_profile_id": provider_profile_id,
                    "display_name": provider_profile_id,
                    "base_url": format!("https://{provider_profile_id}.example"),
                    "protocol_adapter_id": "test-adapter"
                }]
            })
        };
        let snapshot = build(vec![
            file(CatalogKind::KnownProvider, catalog("z-catalog", "zeta")),
            file(CatalogKind::KnownProvider, catalog("a-catalog", "alpha")),
        ])
        .unwrap();

        assert_eq!(
            snapshot
                .known_providers()
                .map(|provider| provider.provider_profile_id.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "zeta"]
        );
    }

    #[test]
    fn provider_exact_and_ordered_pattern_indexes_preserve_actions() {
        let snapshot = build(complete_files()).unwrap();
        let exact = snapshot
            .resolve_provider_rule("openai", "gpt-special", &MatchContext::new())
            .unwrap()
            .unwrap();
        assert_eq!(exact.match_kind, ProviderRuleMatchKind::Exact);
        assert!(exact.action.exclude);

        let pattern = snapshot
            .resolve_provider_rule("openai", "gpt-5-new", &MatchContext::new())
            .unwrap()
            .unwrap();
        assert_eq!(pattern.match_kind, ProviderRuleMatchKind::Pattern);
        assert_eq!(pattern.trace.as_ref().unwrap().position, 0);
        assert_eq!(pattern.action.operations["llm"], "responses.create");
        let high_quality = BTreeMap::from([("/quality".to_owned(), json!("high"))]);
        assert_eq!(pattern.matching_request_rules(&high_quality).len(), 1);
        assert_eq!(pattern.price_for(&high_quality), Some(2.0));
        assert_eq!(pattern.price_for(&MatchContext::new()), Some(1.0));
        let variant_context =
            BTreeMap::from([("provider_model_id".to_owned(), json!("gpt-5-new"))]);
        assert_eq!(
            snapshot
                .matching_provider_variants("openai", &variant_context)
                .unwrap()
                .len(),
            1
        );

        let narrowed = pattern.action.narrow(
            &BTreeSet::from(["llm".to_owned(), "image.txt2img".to_owned()]),
            &BTreeMap::from([
                ("streaming".to_owned(), json!(true)),
                ("tool_call".to_owned(), json!(true)),
            ]),
        );
        assert_eq!(narrowed.api_types, BTreeSet::from(["llm".to_owned()]));
        assert_eq!(
            narrowed.capabilities,
            BTreeMap::from([("streaming".to_owned(), json!(true))])
        );
    }

    #[test]
    fn exact_model_wins_globally_and_cross_driver_conflicts_are_rejected() {
        let files = vec![
            file(
                CatalogKind::ModelDriver,
                model_driver("openai", json!([{"id": "shared-model"}]), json!([])),
            ),
            file(
                CatalogKind::ModelDriver,
                model_driver("claude", json!([]), json!([{"match": "shared-*"}])),
            ),
        ];
        let snapshot = build(files).unwrap();
        let resolved = snapshot
            .resolve_model("shared-model", None, &MatchContext::new())
            .unwrap();
        assert_eq!(resolved.match_kind, ModelMatchKind::Exact);
        assert_eq!(resolved.model_driver_id.as_deref(), Some("openai"));

        let conflict = build(vec![
            file(
                CatalogKind::ModelDriver,
                model_driver("openai", json!([{"id": "shared-model"}]), json!([])),
            ),
            file(
                CatalogKind::ModelDriver,
                model_driver("claude", json!([{"id": "shared-model"}]), json!([])),
            ),
        ])
        .unwrap();
        assert_eq!(
            conflict
                .resolve_model("shared-model", None, &MatchContext::new())
                .unwrap_err(),
            CatalogResolveError::AmbiguousModelDrivers {
                origin_model_id: "shared-model".to_owned(),
                model_driver_ids: vec!["claude".to_owned(), "openai".to_owned()],
            }
        );

        let pattern_conflict = build(vec![
            file(
                CatalogKind::ModelDriver,
                model_driver("openai", json!([]), json!([{"match": "shared-*"}])),
            ),
            file(
                CatalogKind::ModelDriver,
                model_driver("claude", json!([]), json!([{"match": "shared-*"}])),
            ),
        ])
        .unwrap();
        assert!(matches!(
            pattern_conflict.resolve_model("shared-model", None, &MatchContext::new()),
            Err(CatalogResolveError::AmbiguousModelDrivers { .. })
        ));
    }

    #[test]
    fn provider_origin_mapping_uniquely_selects_driver_for_shared_model_id() {
        let rules = routed_provider_rules(
            None,
            json!({"anthropic": "claude", "openai": "openai"}),
            json!([vendor_model_mapping(
                json!([
                    {"op": "lowercase"},
                    {"op": "alias", "table": "origin_provider_aliases"}
                ]),
                json!([{"op": "trim"}])
            )]),
        );
        let snapshot = build(vec![
            file(
                CatalogKind::ModelDriver,
                model_driver("claude", json!([{"id": "shared-model"}]), json!([])),
            ),
            file(
                CatalogKind::ModelDriver,
                model_driver("openai", json!([{"id": "shared-model"}]), json!([])),
            ),
            file(CatalogKind::ProviderRules, rules),
        ])
        .unwrap();

        let origin = snapshot
            .resolve_provider_origin("router", "ANTHROPIC/shared-model")
            .unwrap();
        assert_eq!(
            origin,
            ResolvedProviderOrigin {
                origin_model_id: "shared-model".to_owned(),
                model_driver_id: "claude".to_owned(),
            }
        );
        let candidates = vec![origin.model_driver_id];
        let model = snapshot
            .resolve_model(
                &origin.origin_model_id,
                Some(&candidates),
                &MatchContext::new(),
            )
            .unwrap();
        assert_eq!(model.model_driver_id.as_deref(), Some("claude"));
    }

    #[test]
    fn provider_origin_mapping_rejects_unknown_vendor_and_conflicts() {
        let alias_mapping = vendor_model_mapping(
            json!([
                {"op": "lowercase"},
                {"op": "alias", "table": "origin_provider_aliases"}
            ]),
            json!([]),
        );
        let unknown_snapshot = build(vec![
            file(
                CatalogKind::ModelDriver,
                model_driver("claude", json!([{"id": "shared-model"}]), json!([])),
            ),
            file(
                CatalogKind::ProviderRules,
                routed_provider_rules(
                    None,
                    json!({"anthropic": "claude"}),
                    json!([alias_mapping.clone()]),
                ),
            ),
        ])
        .unwrap();
        assert_eq!(
            unknown_snapshot
                .resolve_provider_origin("router", "unknown/shared-model")
                .unwrap_err(),
            CatalogResolveError::UnknownOriginProvider {
                provider_profile_id: "router".to_owned(),
                origin_provider: "unknown".to_owned(),
            }
        );

        let conflict_snapshot = build(vec![
            file(
                CatalogKind::ModelDriver,
                model_driver("claude", json!([{"id": "shared-model"}]), json!([])),
            ),
            file(
                CatalogKind::ModelDriver,
                model_driver("anthropic", json!([{"id": "shared-model"}]), json!([])),
            ),
            file(
                CatalogKind::ProviderRules,
                routed_provider_rules(
                    None,
                    json!({"anthropic": "claude"}),
                    json!([
                        alias_mapping,
                        vendor_model_mapping(json!([{"op": "lowercase"}]), json!([]))
                    ]),
                ),
            ),
        ])
        .unwrap();
        assert!(matches!(
            conflict_snapshot.resolve_provider_origin("router", "ANTHROPIC/shared-model"),
            Err(CatalogResolveError::ConflictingOriginMappings { resolved, .. })
                if resolved.len() == 2
        ));
    }

    #[test]
    fn provider_origin_aliases_cannot_escape_metadata_drivers() {
        let rules = routed_provider_rules(
            Some(vec!["openai"]),
            json!({"anthropic": "claude"}),
            json!([vendor_model_mapping(
                json!([{"op": "alias", "table": "origin_provider_aliases"}]),
                json!([])
            )]),
        );
        assert!(matches!(
            build(vec![
                file(
                    CatalogKind::ModelDriver,
                    model_driver("openai", json!([]), json!([])),
                ),
                file(
                    CatalogKind::ModelDriver,
                    model_driver("claude", json!([]), json!([])),
                ),
                file(CatalogKind::ProviderRules, rules),
            ]),
            Err(CatalogBuildError::InvalidValue {
                field: "origin_provider_aliases",
                ..
            })
        ));

        let snapshot = build(vec![
            file(
                CatalogKind::ModelDriver,
                model_driver("openai", json!([]), json!([])),
            ),
            file(
                CatalogKind::ModelDriver,
                model_driver("claude", json!([]), json!([])),
            ),
            file(
                CatalogKind::ProviderRules,
                routed_provider_rules(
                    Some(vec!["openai"]),
                    json!({}),
                    json!([vendor_model_mapping(json!([]), json!([]))]),
                ),
            ),
        ])
        .unwrap();
        assert_eq!(
            snapshot
                .resolve_provider_origin("router", "claude/shared-model")
                .unwrap_err(),
            CatalogResolveError::OriginDriverOutsideMetadataDrivers {
                provider_profile_id: "router".to_owned(),
                model_driver_id: "claude".to_owned(),
            }
        );
    }

    #[test]
    fn schema_revision_required_features_and_references_are_validated() {
        let mut unsupported_schema = model_driver("openai", json!([]), json!([]));
        unsupported_schema["schema_revision"] = json!(1);
        assert!(matches!(
            build(vec![file(CatalogKind::ModelDriver, unsupported_schema)]),
            Err(CatalogBuildError::UnsupportedSchema { .. })
        ));

        let mut required_feature = model_driver("openai", json!([]), json!([]));
        required_feature["required_features"] = json!(["structured-capability-v1"]);
        assert!(matches!(
            build(vec![file(
                CatalogKind::ModelDriver,
                required_feature.clone()
            )]),
            Err(CatalogBuildError::UnsupportedFeature { .. })
        ));
        let options = CatalogBuildOptions {
            supported_features: BTreeSet::from(["structured-capability-v1".to_owned()]),
        };
        CatalogSnapshot::from_current_files(
            42,
            vec![file(CatalogKind::ModelDriver, required_feature)],
            &options,
        )
        .unwrap();

        let future_revision = model_driver("openai", json!([]), json!([]));
        assert!(matches!(
            CatalogSnapshot::from_current_files(
                6,
                vec![file(CatalogKind::ModelDriver, future_revision)],
                &CatalogBuildOptions::default(),
            ),
            Err(CatalogBuildError::RevisionAheadOfSnapshot {
                revision_seq: 7,
                target_revision_seq: 6,
                ..
            })
        ));

        let missing_driver = vec![file(CatalogKind::ProviderRules, provider_rules())];
        assert!(matches!(
            build(missing_driver),
            Err(CatalogBuildError::UnknownReference {
                field: "metadata_drivers",
                ..
            })
        ));

        let missing_rules = vec![
            file(
                CatalogKind::ModelDriver,
                model_driver("openai", json!([]), json!([])),
            ),
            file(CatalogKind::KnownProvider, known_providers()),
        ];
        assert!(matches!(
            build(missing_rules),
            Err(CatalogBuildError::UnknownReference {
                field: "providers.provider_rules_id",
                ..
            })
        ));
    }

    #[test]
    fn static_catalogs_reject_dynamic_discovery_facts_and_capability_additions() {
        let mut dynamic_top_level = model_driver("openai", json!([]), json!([]));
        dynamic_top_level["availability"] = json!("available");
        assert!(matches!(
            build(vec![file(CatalogKind::ModelDriver, dynamic_top_level)]),
            Err(CatalogBuildError::InvalidJson { .. })
        ));

        let dynamic_capability = model_driver(
            "openai",
            json!([{
                "id": "gpt",
                "capabilities": {"health": "available"}
            }]),
            json!([]),
        );
        assert!(matches!(
            build(vec![file(CatalogKind::ModelDriver, dynamic_capability)]),
            Err(CatalogBuildError::StaticDynamicBoundary { .. })
        ));

        let mut capability_addition = provider_rules();
        capability_addition["patterns"][0]["capabilities"] = json!({"tool_call": true});
        assert!(matches!(
            build(vec![file(CatalogKind::ProviderRules, capability_addition)]),
            Err(CatalogBuildError::InvalidJson { .. })
        ));
    }

    #[test]
    fn all_nested_match_rules_compile_during_snapshot_build() {
        let mut invalid_request_rule = provider_rules();
        invalid_request_rule["patterns"][0]["request_rules"][0]["when"] =
            json!({"unknown_option": true});
        let files = vec![
            file(
                CatalogKind::ModelDriver,
                model_driver("openai", json!([]), json!([])),
            ),
            file(CatalogKind::ProviderRules, invalid_request_rule),
        ];
        assert!(matches!(build(files), Err(CatalogBuildError::Match(_))));

        let mut invalid_model_pattern = model_driver("openai", json!([]), json!([]));
        invalid_model_pattern["patterns"] = json!([{
            "match": {"provider_model_id": "gpt-*"}
        }]);
        assert!(matches!(
            build(vec![file(CatalogKind::ModelDriver, invalid_model_pattern)]),
            Err(CatalogBuildError::Match(_))
        ));

        let conditional_model_price = model_driver(
            "openai",
            json!([{
                "id": "gpt",
                "pricing": {
                    "currency": "USD",
                    "rules": [{"when": {"/quality": "high"}, "amount": 1.0}]
                }
            }]),
            json!([]),
        );
        assert!(matches!(
            build(vec![file(
                CatalogKind::ModelDriver,
                conditional_model_price
            )]),
            Err(CatalogBuildError::InvalidValue {
                field: "pricing.rules",
                ..
            })
        ));
    }

    #[test]
    fn malformed_files_and_duplicate_identities_fail_atomically() {
        assert!(matches!(
            CatalogSnapshot::from_current_files(
                1,
                vec![CurrentCatalogFile {
                    kind: CatalogKind::ModelDriver,
                    contents: b"{".to_vec(),
                }],
                &CatalogBuildOptions::default(),
            ),
            Err(CatalogBuildError::InvalidJson { .. })
        ));

        let duplicate = model_driver("openai", json!([]), json!([]));
        assert!(matches!(
            build(vec![
                file(CatalogKind::ModelDriver, duplicate.clone()),
                file(CatalogKind::ModelDriver, duplicate),
            ]),
            Err(CatalogBuildError::DuplicateCatalog { .. })
        ));

        let duplicate_exact =
            model_driver("openai", json!([{"id": "gpt"}, {"id": "gpt"}]), json!([]));
        assert!(matches!(
            build(vec![file(CatalogKind::ModelDriver, duplicate_exact)]),
            Err(CatalogBuildError::DuplicateExactRule { .. })
        ));

        let snapshot = build(vec![file(
            CatalogKind::ModelDriver,
            model_driver("openai", json!([]), json!([{"match": "gpt-*"}])),
        )])
        .unwrap();
        let duplicate_candidates = vec!["openai".to_owned(), "openai".to_owned()];
        assert_eq!(
            snapshot
                .resolve_model("gpt-new", Some(&duplicate_candidates), &MatchContext::new(),)
                .unwrap()
                .match_kind,
            ModelMatchKind::Pattern
        );
    }
}
