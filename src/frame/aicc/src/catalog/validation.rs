use super::*;

pub(super) fn validate_model_driver(
    catalog: &ModelDriverCatalog,
    options: &CatalogBuildOptions,
) -> Result<(), CatalogBuildError> {
    validate_header(
        CatalogKind::ModelDriver,
        &catalog.model_driver_id,
        &catalog.format,
        MODEL_DRIVER_FORMAT,
        MODEL_DRIVER_SCHEMA_VERSION,
        MODEL_DRIVER_SUPPORTED_SCHEMA_REVISION,
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
    validate_nonempty_strings(
        CatalogKind::ModelDriver,
        &catalog.model_driver_id,
        "variants.name",
        catalog.variants.iter().map(|variant| variant.name.as_str()),
    )?;
    if catalog.schema_revision == 0
        && catalog
            .variants
            .iter()
            .any(|variant| !variant.provider_options.is_empty())
    {
        return Err(CatalogBuildError::InvalidValue {
            owner: catalog.model_driver_id.clone(),
            field: "schema_revision",
            reason: "variant provider_options require schema_revision 1".to_owned(),
        });
    }
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
        for mount in &rule.auto_mounts {
            validate_nonempty_field(
                CatalogKind::ModelDriver,
                &catalog.model_driver_id,
                "version_rules.auto_mounts",
                mount,
            )?;
        }
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
    if let Some(mappings) = &semantics.canonical_fields {
        validate_canonical_fields(owner, mappings)?;
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

pub(super) fn validate_provider_rules(
    catalog: &ProviderRulesCatalog,
) -> Result<(), CatalogBuildError> {
    validate_header(
        CatalogKind::ProviderRules,
        &catalog.provider_profile_id,
        &catalog.format,
        PROVIDER_RULES_FORMAT,
        PROVIDER_RULES_SCHEMA_VERSION,
        PROVIDER_RULES_SUPPORTED_SCHEMA_REVISION,
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
    validate_unique_nonempty(
        CatalogKind::ProviderRules,
        &catalog.provider_profile_id,
        "static_inventory_models",
        catalog.static_inventory_models.iter().map(String::as_str),
    )?;
    if catalog.schema_revision == 0 && !catalog.static_inventory_models.is_empty() {
        return Err(CatalogBuildError::InvalidValue {
            owner: catalog.provider_profile_id.clone(),
            field: "schema_revision",
            reason: "static_inventory_models requires schema_revision 1".to_owned(),
        });
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
    fn canonical_fields(&self) -> &BTreeMap<String, crate::canonical::CanonicalFieldMapping>;
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
            fn canonical_fields(
                &self,
            ) -> &BTreeMap<String, crate::canonical::CanonicalFieldMapping> {
                &self.canonical_fields
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
    validate_canonical_fields(owner, rule.canonical_fields())?;
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

fn validate_canonical_fields(
    owner: &str,
    mappings: &BTreeMap<String, crate::canonical::CanonicalFieldMapping>,
) -> Result<(), CatalogBuildError> {
    for (pointer, mapping) in mappings {
        if pointer.is_empty() || !valid_json_pointer(pointer) {
            return Err(CatalogBuildError::InvalidValue {
                owner: owner.to_owned(),
                field: "canonical_fields",
                reason: format!("field key must be a non-empty JSON Pointer: {pointer:?}"),
            });
        }
        mapping
            .validate()
            .map_err(|reason| CatalogBuildError::InvalidValue {
                owner: owner.to_owned(),
                field: "canonical_fields",
                reason: format!("invalid mapping for {pointer:?}: {reason}"),
            })?;
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

pub(super) fn validate_known_provider_catalog(
    catalog: &KnownProviderCatalog,
) -> Result<(), CatalogBuildError> {
    validate_header(
        CatalogKind::KnownProvider,
        &catalog.catalog_id,
        &catalog.format,
        KNOWN_PROVIDER_FORMAT,
        KNOWN_PROVIDER_SCHEMA_VERSION,
        KNOWN_PROVIDER_SUPPORTED_SCHEMA_REVISION,
        catalog.schema_version,
        catalog.schema_revision,
    )?;
    let mut profiles = BTreeSet::new();
    for provider in &catalog.providers {
        if catalog.schema_revision == 0
            && (!provider.credential_variants.is_empty()
                || !provider.connection.region_base_urls.is_empty())
        {
            return Err(CatalogBuildError::InvalidValue {
                owner: catalog.catalog_id.clone(),
                field: "schema_revision",
                reason: "credential_variants and region_base_urls require schema_revision 1"
                    .to_owned(),
            });
        }
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
            (
                "providers.discovery_behavior_id",
                &provider.discovery_behavior_id,
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
        validate_provider_configuration(&catalog.catalog_id, provider)?;
        if !profiles.insert(provider.provider_profile_id.as_str()) {
            return Err(CatalogBuildError::DuplicateKnownProvider {
                provider_profile_id: provider.provider_profile_id.clone(),
            });
        }
    }
    Ok(())
}

fn validate_provider_configuration(
    owner: &str,
    provider: &KnownProvider,
) -> Result<(), CatalogBuildError> {
    let mut credential_kinds = BTreeSet::new();
    for credential in std::iter::once(&provider.credential).chain(&provider.credential_variants) {
        if !credential_kinds.insert(credential.kind) {
            return Err(CatalogBuildError::InvalidValue {
                owner: owner.to_owned(),
                field: "providers.credential_variants",
                reason: "credential kinds must be unique".to_owned(),
            });
        }
        validate_provider_credential(owner, credential)?;
    }
    for (field, schema) in [
        ("providers.connection.region", &provider.connection.region),
        (
            "providers.connection.workspace",
            &provider.connection.workspace,
        ),
        ("providers.connection.account", &provider.connection.account),
    ] {
        if schema.mode == ProviderFieldMode::Unsupported
            && (schema.default_value.is_some() || !schema.allowed_values.is_empty())
        {
            return Err(CatalogBuildError::InvalidValue {
                owner: owner.to_owned(),
                field,
                reason: "unsupported fields cannot define defaults or allowed values".to_owned(),
            });
        }
        let mut values = BTreeSet::new();
        for value in &schema.allowed_values {
            if value.trim().is_empty() || !values.insert(value) {
                return Err(CatalogBuildError::InvalidValue {
                    owner: owner.to_owned(),
                    field,
                    reason: "allowed values must be unique and non-empty".to_owned(),
                });
            }
        }
        if let Some(default) = schema.default_value.as_deref() {
            if default.trim().is_empty()
                || (!schema.allowed_values.is_empty()
                    && !schema.allowed_values.iter().any(|value| value == default))
            {
                return Err(CatalogBuildError::InvalidValue {
                    owner: owner.to_owned(),
                    field,
                    reason: "default must be non-empty and belong to allowed_values".to_owned(),
                });
            }
        }
    }
    for (region, base_url) in &provider.connection.region_base_urls {
        if region.trim().is_empty()
            || !provider
                .connection
                .region
                .allowed_values
                .iter()
                .any(|value| value == region)
        {
            return Err(CatalogBuildError::InvalidValue {
                owner: owner.to_owned(),
                field: "providers.connection.region_base_urls",
                reason: "every region URL must name an allowed region".to_owned(),
            });
        }
        if !base_url.starts_with("https://") && !base_url.starts_with("http://") {
            return Err(CatalogBuildError::InvalidValue {
                owner: owner.to_owned(),
                field: "providers.connection.region_base_urls",
                reason: "region base URLs must use http or https".to_owned(),
            });
        }
    }
    Ok(())
}

fn validate_provider_credential(
    owner: &str,
    credential: &ProviderCredentialDescriptor,
) -> Result<(), CatalogBuildError> {
    match credential.kind {
        ProviderCredentialKind::NamedHeader => {
            if credential
                .header_name
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            {
                return Err(CatalogBuildError::InvalidValue {
                    owner: owner.to_owned(),
                    field: "providers.credential.header_name",
                    reason: "named_header credentials require a non-empty header name".to_owned(),
                });
            }
        }
        _ if credential.header_name.is_some() => {
            return Err(CatalogBuildError::InvalidValue {
                owner: owner.to_owned(),
                field: "providers.credential.header_name",
                reason: "is only valid for named_header credentials".to_owned(),
            });
        }
        _ => {}
    }
    Ok(())
}

fn validate_header(
    kind: CatalogKind,
    id: &str,
    format: &str,
    expected_format: &'static str,
    expected_schema_version: u32,
    supported_schema_revision: u32,
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
    if schema_version != expected_schema_version || schema_revision > supported_schema_revision {
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

pub(super) fn validate_references(
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

pub(super) fn validate_revisions(
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
