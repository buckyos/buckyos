#[macro_use]
mod schema;
mod validation;

pub(crate) use schema::{
    family_segment, CatalogBuildOptions, CatalogDocuments, CatalogKind, CurrentCatalogFile,
    KnownProvider, KnownProviderCatalog, LlmModel, ModelDriverCatalog, ModelMatchKind,
    ModelPricingRule, ModelSemantics, ModelStability, ModelVersion, OriginMapping, Pricing,
    PricingTierStep, PricingTiers, PricingTimeWindow, PricingUnit, PricingWeekday,
    ProviderCredentialDescriptor, ProviderCredentialKind, ProviderExactRule, ProviderFieldMode,
    ProviderFieldSchema, ProviderPatternRule, ProviderRuleAction, ProviderRulesCatalog,
    ProviderVariantRule, RequestRule, ResolvedModelSemantics, ResolvedProviderConfiguration,
    ResolvedProviderOrigin, ResolvedProviderRule, TierDimension, TierMode,
};
#[cfg(test)]
pub(crate) use schema::{Effort, ProviderRuleMatchKind};
use validation::{
    llm_pattern_ids, validate_known_provider_catalog, validate_model_driver,
    validate_provider_rules, validate_references, validate_revisions, validate_segment,
};

use crate::error::{CatalogBuildError, CatalogResolveError, MatchCompileError};
use crate::matching::{
    CompiledMatchRule, CompiledRuleSet, MatchContext, MatchRule, MatchSchema, MatchTrace,
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
const MODEL_DRIVER_SCHEMA_VERSION: u32 = 2;
const PROVIDER_RULES_SCHEMA_VERSION: u32 = 1;
const KNOWN_PROVIDER_SCHEMA_VERSION: u32 = 1;
const MODEL_DRIVER_SUPPORTED_SCHEMA_REVISION: u32 = 0;
const PROVIDER_RULES_SUPPORTED_SCHEMA_REVISION: u32 = 1;
const KNOWN_PROVIDER_SUPPORTED_SCHEMA_REVISION: u32 = 1;

#[derive(Clone, Debug)]
struct CompiledModelDriverCatalog {
    document: ModelDriverCatalog,
    exact_index: BTreeMap<String, usize>,
    patterns: CompiledRuleSet,
    llm_models: BTreeMap<String, LlmModel>,
}

/// Prices, resolved independently of the technical rules.
///
/// An exact `id` always wins; wildcard entries are tried in declaration order,
/// so a provider can give a family default and still override single models.
#[derive(Clone, Debug)]
struct CompiledPricingTable {
    exact: BTreeMap<String, CompiledPricing>,
    patterns: CompiledRuleSet,
    pattern_pricing: Vec<CompiledPricing>,
}

#[derive(Clone, Debug)]
struct CompiledPricing {
    pricing: Pricing,
    rules: Vec<CompiledMatchRule>,
}

impl CompiledPricingTable {
    fn compile(
        entries: &[ModelPricingRule],
        schema: &MatchSchema,
    ) -> Result<Self, MatchCompileError> {
        let mut exact = BTreeMap::new();
        let mut wildcard_rules = Vec::new();
        let mut pattern_pricing = Vec::new();
        for entry in entries {
            let rules = entry
                .pricing
                .rules
                .iter()
                .map(|rule| {
                    CompiledMatchRule::compile(rule.when.clone(), &PRICING_RULE_MATCH_SCHEMA)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let compiled = CompiledPricing {
                pricing: entry.pricing.clone(),
                rules,
            };
            match (&entry.id, &entry.match_rule) {
                (Some(id), None) => {
                    exact.insert(id.clone(), compiled);
                }
                (None, Some(rule)) => {
                    wildcard_rules.push(RuleEntry {
                        rule_id: None,
                        rule: rule.clone(),
                    });
                    pattern_pricing.push(compiled);
                }
                // `validate_model_pricing` rejects the other two shapes.
                _ => {}
            }
        }
        let patterns = CompiledRuleSet::compile(wildcard_rules, schema)?;
        Ok(Self {
            exact,
            patterns,
            pattern_pricing,
        })
    }

    fn lookup(&self, model_id: &str, context: &MatchContext) -> Option<&CompiledPricing> {
        if let Some(pricing) = self.exact.get(model_id) {
            return Some(pricing);
        }
        let trace = self.patterns.first_match(context)?;
        self.pattern_pricing.get(trace.position)
    }
}

fn pricing_context(model_id: &str, dimension: &str, base: &MatchContext) -> MatchContext {
    let mut context = base.clone();
    context.insert(dimension.to_owned(), Value::String(model_id.to_owned()));
    context
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
    pricing: CompiledPricingTable,
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

        let mut names = BTreeMap::new();
        for (driver, catalog) in &model_drivers {
            for (path, owner) in catalog
                .document
                .specs
                .iter()
                .map(|spec| {
                    (
                        format!("llm.{}", spec.id),
                        format!("{driver} spec {}", spec.id),
                    )
                })
                .chain(
                    catalog
                        .llm_models
                        .iter()
                        .map(|(id, model)| (model.family.clone(), format!("{driver} model {id}"))),
                )
            {
                if let Some(previous) = names.insert(path.clone(), owner.clone()) {
                    return Err(CatalogBuildError::InvalidValue {
                        owner,
                        field: "llm.family_id/specs.id",
                        reason: format!("{path} conflicts with {previous}"),
                    });
                }
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

    pub(crate) fn model_drivers(&self) -> impl Iterator<Item = &ModelDriverCatalog> {
        self.model_drivers.values().map(|catalog| &catalog.document)
    }

    pub(crate) fn llm_model(&self, driver: &str, model: &str) -> Option<&LlmModel> {
        self.model_drivers.get(driver)?.llm_models.get(model)
    }

    pub(crate) fn llm_models(&self) -> impl Iterator<Item = (&str, &str, &LlmModel)> {
        self.model_drivers.iter().flat_map(|(driver, catalog)| {
            catalog
                .llm_models
                .iter()
                .map(move |(id, model)| (driver.as_str(), id.as_str(), model))
        })
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

    pub(crate) fn resolve_provider_configuration(
        &self,
        provider_profile_id: &str,
    ) -> Result<ResolvedProviderConfiguration, CatalogResolveError> {
        let provider = self.known_provider(provider_profile_id).ok_or_else(|| {
            CatalogResolveError::UnknownKnownProvider {
                provider_profile_id: provider_profile_id.to_owned(),
            }
        })?;
        let provider_rules_id = provider.provider_rules_id.as_ref().ok_or_else(|| {
            CatalogResolveError::MissingProviderRulesReference {
                provider_profile_id: provider_profile_id.to_owned(),
            }
        })?;
        let rules = self.provider_rules(provider_rules_id).ok_or_else(|| {
            CatalogResolveError::UnknownProviderRules {
                provider_profile_id: provider_rules_id.clone(),
            }
        })?;
        if rules.provider_profile_id != provider.provider_profile_id {
            return Err(CatalogResolveError::ProviderRulesIdentityMismatch {
                provider_profile_id: provider.provider_profile_id.clone(),
                provider_rules_id: provider_rules_id.clone(),
                rules_provider_profile_id: rules.provider_profile_id.clone(),
            });
        }
        Ok(ResolvedProviderConfiguration {
            provider_profile_id: provider.provider_profile_id.clone(),
            display_name: provider.display_name.clone(),
            default_base_url: provider.base_url.clone(),
            credential: provider.credential.clone(),
            credential_variants: provider.credential_variants.clone(),
            connection: provider.connection.clone(),
            protocol_adapter_id: provider.protocol_adapter_id.clone(),
            discovery_behavior_id: provider.discovery_behavior_id.clone(),
            dynamic_login_behavior_id: provider.dynamic_login_behavior_id.clone(),
            connection_behavior_id: provider.connection_behavior_id.clone(),
            provider_rules_id: provider_rules_id.clone(),
        })
    }

    #[cfg(test)]
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

    pub(crate) fn matching_provider_variants_for_model(
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
            .filter_map(|(variant, condition)| {
                let mut variant_context = context.clone();
                variant_context.insert("variant".into(), Value::String(variant.variant.clone()));
                condition.matches(&variant_context).then_some(variant)
            })
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
            let semantics = catalog.document.defaults.clone();
            return Ok(resolved_model(
                origin_model_id,
                driver_id,
                catalog.document.revision_seq,
                ModelMatchKind::Defaults,
                None,
                semantics,
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
            let mut action = provider_rule_action!(rule);
            let mut compiled = catalog.exact_compiled[*position].clone();
            apply_pricing(
                &mut action,
                &mut compiled,
                catalog,
                provider_model_id,
                &pricing_context(provider_model_id, "provider_model_id", dimensions),
            );
            return Ok(Some(ResolvedProviderRule {
                catalog_revision_seq: catalog.document.revision_seq,
                #[cfg(test)]
                match_kind: ProviderRuleMatchKind::Exact,
                #[cfg(test)]
                trace: None,
                action,
                compiled,
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
        let mut action = provider_rule_action!(rule);
        let mut compiled = catalog.pattern_compiled[position].clone();
        apply_pricing(
            &mut action,
            &mut compiled,
            catalog,
            provider_model_id,
            &context,
        );
        Ok(Some(ResolvedProviderRule {
            catalog_revision_seq: catalog.document.revision_seq,
            #[cfg(test)]
            match_kind: ProviderRuleMatchKind::Pattern,
            #[cfg(test)]
            trace: Some(trace),
            action,
            compiled,
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
    let mut ids: BTreeSet<String> = exact_index.keys().cloned().collect();
    for rule in &document.patterns {
        if document
            .defaults
            .overlay(&model_rule_semantics!(rule))
            .llm
            .is_some()
            && rule.exclude != Some(true)
        {
            ids.extend(llm_pattern_ids(
                &document.model_driver_id,
                &rule.match_rule,
            )?);
        }
    }
    let mut llm_models = BTreeMap::new();
    for id in ids {
        let rule = if let Some(position) = exact_index.get(&id) {
            model_rule_semantics!(&document.models[*position])
        } else {
            let context = BTreeMap::from([("origin_model_id".into(), Value::String(id.clone()))]);
            let Some(trace) = patterns.first_match(&context) else {
                continue;
            };
            model_rule_semantics!(&document.patterns[trace.position])
        };
        let semantics = document.defaults.overlay(&rule);
        if semantics.exclude == Some(true) {
            continue;
        }
        if let Some(llm) = semantics.llm {
            let family = llm.family_id.clone().unwrap_or_else(|| family_segment(&id));
            validate_segment(
                &format!("{} model {id}", document.model_driver_id),
                "llm.family_id",
                &family,
            )?;
            llm_models.insert(
                id.clone(),
                LlmModel {
                    model_driver_id: document.model_driver_id.clone(),
                    origin_model_id: id.clone(),
                    family: format!("llm.{family}"),
                    semantics: llm,
                    version: ModelVersion::from_model_id(&document.model_driver_id, &id),
                },
            );
        }
    }
    Ok(CompiledModelDriverCatalog {
        document,
        exact_index,
        patterns,
        llm_models,
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
    let pricing =
        CompiledPricingTable::compile(&document.model_pricing, &PROVIDER_RULE_MATCH_SCHEMA)?;
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
        pricing,
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
}

impl ProviderRuleData for ProviderExactRule {
    fn request_rules(&self) -> &[RequestRule] {
        &self.request_rules
    }
}

impl ProviderRuleData for ProviderPatternRule {
    fn request_rules(&self) -> &[RequestRule] {
        &self.request_rules
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
    // Conditional channel prices now come from `model_pricing`; the resolved
    // rules are attached in `apply_pricing`.
    Ok(CompiledProviderRule {
        request_conditions,
        pricing_rules: Vec::new(),
    })
}

fn apply_pricing(
    action: &mut ProviderRuleAction,
    compiled: &mut CompiledProviderRule,
    catalog: &CompiledProviderRulesCatalog,
    provider_model_id: &str,
    context: &MatchContext,
) {
    let Some(entry) = catalog.pricing.lookup(provider_model_id, context) else {
        return;
    };
    action.pricing = Some(entry.pricing.clone());
    compiled.pricing_rules = entry.rules.clone();
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

impl fmt::Display for CatalogResolveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownKnownProvider {
                provider_profile_id,
            } => write!(formatter, "unknown Known Provider {provider_profile_id:?}"),
            Self::MissingProviderRulesReference {
                provider_profile_id,
            } => write!(
                formatter,
                "Known Provider {provider_profile_id:?} has no Provider Rules reference"
            ),
            Self::ProviderRulesIdentityMismatch {
                provider_profile_id,
                provider_rules_id,
                rules_provider_profile_id,
            } => write!(
                formatter,
                "Known Provider {provider_profile_id:?} references Provider Rules {provider_rules_id:?} belonging to {rules_provider_profile_id:?}"
            ),
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
mod tests;
