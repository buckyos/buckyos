use super::super::ProviderFieldSchema;
#[cfg(test)]
use super::super::{
    CredentialDescriptor, DiscoveryMode, ProviderConnectionContract, ProviderConnectionInput,
    ProviderProfile, ProviderResult, RefreshPolicy,
};
use super::anthropic_models::{AnthropicModelsDiscovery, AnthropicModelsSpec};
use crate::catalog::KnownProvider;
#[cfg(test)]
use crate::catalog::{
    CatalogKind, CurrentCatalogFile, KnownProviderCatalog, ModelDriverCatalog, ProviderRulesCatalog,
};
#[cfg(test)]
use crate::protocol::CredentialKind;
use crate::protocol::HttpTransport;
use serde::de::DeserializeOwned;
use serde::Deserialize;
#[cfg(test)]
use std::collections::BTreeMap;

pub(crate) const MINIMAX_PROVIDER_PROFILE_ID: &str = "minimax";

pub(super) const MINIMAX_SPEC: AnthropicModelsSpec = AnthropicModelsSpec {
    provider_profile_id: MINIMAX_PROVIDER_PROFILE_ID,
    version_header: false,
    label: "MiniMax",
};

#[cfg(test)]
pub(crate) fn minimax_profile() -> ProviderProfile {
    let known = minimax_known_provider();
    let credential: CredentialDeclaration = embedded_value(
        &known,
        "credential",
        "MiniMax Known Provider credential declaration",
    );
    assert_eq!(credential.kind, "named_header");
    assert!(credential.required && credential.secret);
    ProviderProfile {
        provider_profile_id: MINIMAX_PROVIDER_PROFILE_ID.to_owned(),
        display_name: known.display_name,
        default_protocol_adapter_id: known.protocol_adapter_id,
        credential: CredentialDescriptor {
            kind: CredentialKind::NamedHeader,
            header_name: Some(credential.header_name),
        },
        discovery_mode: DiscoveryMode::MachineApi,
        refresh: RefreshPolicy::default(),
        default_inventory: None,
    }
}

#[cfg(test)]
pub(crate) fn minimax_known_provider() -> KnownProvider {
    super::builtin_catalog_document::<KnownProviderCatalog>(
        CatalogKind::KnownProvider,
        MINIMAX_PROVIDER_PROFILE_ID,
    )
    .providers
    .into_iter()
    .find(|provider| provider.provider_profile_id == MINIMAX_PROVIDER_PROFILE_ID)
    .expect("MiniMax Known Provider catalog must contain the MiniMax profile")
}

#[cfg(test)]
pub(crate) fn minimax_connection_contract() -> ProviderConnectionContract {
    let known = minimax_known_provider();
    let fields: InstanceFieldDeclarations = embedded_value(
        &known,
        "instance_fields",
        "MiniMax Known Provider instance fields",
    );
    ProviderConnectionContract {
        default_base_url: known.base_url,
        region: fields.region,
        workspace: fields.workspace,
        account: fields.account,
    }
}

#[cfg(test)]
pub(crate) fn resolve_minimax_connection(
    input: ProviderConnectionInput<'_>,
) -> ProviderResult<super::super::ResolvedProviderConnection> {
    let known = minimax_known_provider();
    let region_base_urls: BTreeMap<String, String> = embedded_value(
        &known,
        "region_base_urls",
        "MiniMax Known Provider regional base URLs",
    );
    let contract = minimax_connection_contract();
    contract.resolve(ProviderConnectionInput {
        base_url: Some(&known.base_url),
        ..input.clone()
    })?;
    let region = input
        .region
        .or(contract.region.default_value.as_deref())
        .expect("MiniMax region must have a configured default");
    let regional_base_url = region_base_urls
        .get(region)
        .expect("every allowed MiniMax region must have a configured base URL");
    contract.resolve(ProviderConnectionInput {
        base_url: input.base_url.or(Some(regional_base_url)),
        ..input
    })
}

#[cfg(test)]
pub(crate) fn minimax_provider_rules(_revision_seq: u64) -> ProviderRulesCatalog {
    super::builtin_catalog_document(CatalogKind::ProviderRules, MINIMAX_PROVIDER_PROFILE_ID)
}

#[cfg(test)]
pub(crate) fn minimax_model_driver() -> ModelDriverCatalog {
    super::builtin_catalog_document(CatalogKind::ModelDriver, MINIMAX_PROVIDER_PROFILE_ID)
}

#[cfg(test)]
pub(crate) fn minimax_catalog_files() -> Vec<CurrentCatalogFile> {
    super::builtin_catalog_files(&[MINIMAX_PROVIDER_PROFILE_ID])
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialDeclaration {
    kind: String,
    header_name: String,
    required: bool,
    secret: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InstanceFieldDeclarations {
    region: ProviderFieldSchema,
    workspace: ProviderFieldSchema,
    account: ProviderFieldSchema,
}

fn embedded_value<T: DeserializeOwned>(known: &KnownProvider, key: &str, label: &str) -> T {
    serde_json::from_value(
        known
            .ui_hints
            .get(key)
            .unwrap_or_else(|| panic!("{label} is missing"))
            .clone(),
    )
    .unwrap_or_else(|error| panic!("{label} is invalid: {error}"))
}

fn embedded_json<T: DeserializeOwned>(contents: &[u8], label: &str) -> T {
    serde_json::from_slice(contents).unwrap_or_else(|error| panic!("{label} is invalid: {error}"))
}

pub(crate) fn minimax_discovery(transport: HttpTransport) -> AnthropicModelsDiscovery {
    AnthropicModelsDiscovery::new(MINIMAX_SPEC, transport)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::CatalogBuildOptions;
    use crate::protocol::{
        minimax_messages_adapter, minimax_messages_dialect_contract, CodecRegistry,
        CLAUDE_MESSAGES_ADAPTER_ID, CLAUDE_MESSAGES_OPERATION_ID, MINIMAX_MESSAGES_ADAPTER_ID,
    };
    use crate::settings::{MetadataFile, MetadataSource, MetadataSources};

    #[test]
    fn embedded_catalogs_drive_identity_regions_rules_and_models() {
        let profile = minimax_profile();
        let known = minimax_known_provider();
        let rules = minimax_provider_rules(9);
        let models = minimax_model_driver();
        let contract = minimax_messages_dialect_contract();
        let (adapter, registration) = minimax_messages_adapter();

        assert_eq!(profile.provider_profile_id, MINIMAX_PROVIDER_PROFILE_ID);
        assert_eq!(
            profile.default_protocol_adapter_id,
            MINIMAX_MESSAGES_ADAPTER_ID
        );
        assert_eq!(profile.credential.kind, CredentialKind::NamedHeader);
        assert_eq!(
            resolve_minimax_connection(Default::default())
                .unwrap()
                .base_url,
            known.base_url
        );
        let regional_urls: BTreeMap<String, String> = embedded_value(
            &known,
            "region_base_urls",
            "MiniMax Known Provider regional base URLs",
        );
        assert_eq!(
            resolve_minimax_connection(ProviderConnectionInput {
                region: Some("china"),
                ..Default::default()
            })
            .unwrap()
            .base_url,
            regional_urls["china"]
        );
        assert!(resolve_minimax_connection(ProviderConnectionInput {
            region: Some("unknown"),
            ..Default::default()
        })
        .is_err());
        assert_eq!(
            known.provider_rules_id.as_deref(),
            Some(MINIMAX_PROVIDER_PROFILE_ID)
        );
        assert_eq!(
            rules.metadata_drivers,
            Some(vec![MINIMAX_PROVIDER_PROFILE_ID.to_owned()])
        );
        assert_eq!(
            rules.models[0].operations["llm"],
            CLAUDE_MESSAGES_OPERATION_ID
        );
        assert_eq!(rules.models[0].request_rules[0].defaults["top_p"], 0.95);
        assert!(rules.models[0].request_rules[0]
            .remove
            .contains(&"/stop".to_owned()));
        assert_eq!(models.model_driver_id, MINIMAX_PROVIDER_PROFILE_ID);
        assert!(models.models.iter().any(|model| model.id == "MiniMax-M3"));
        assert_eq!(contract.base_adapter_id, CLAUDE_MESSAGES_ADAPTER_ID);
        assert_eq!(
            adapter.base_adapter_id.as_deref(),
            Some(CLAUDE_MESSAGES_ADAPTER_ID)
        );
        assert_eq!(registration.operation_codecs.len(), 1);
        let builtin = minimax_catalog_files()
            .into_iter()
            .map(|file| MetadataFile::parse(MetadataSource::Builtin, file.kind, file.contents))
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let catalog = MetadataSources {
            builtin,
            ..MetadataSources::default()
        }
        .build_snapshot(1, &CatalogBuildOptions::default())
        .unwrap();
        assert!(catalog
            .known_provider(MINIMAX_PROVIDER_PROFILE_ID)
            .is_some());
        assert!(catalog
            .provider_rules(MINIMAX_PROVIDER_PROFILE_ID)
            .is_some());
        assert!(catalog.model_driver(MINIMAX_PROVIDER_PROFILE_ID).is_some());
    }

    #[test]
    fn derived_registration_depends_one_way_on_the_unchanged_base_adapter() {
        let (base_descriptor, base_registration) = super::super::claude_messages_adapter();
        let mut registry = CodecRegistry::default();
        registry
            .register_codecs(base_descriptor, base_registration)
            .unwrap();
        let (derived_descriptor, derived_registration) = minimax_messages_adapter();
        registry
            .register_codecs(derived_descriptor, derived_registration)
            .unwrap();

        assert!(registry.adapter(CLAUDE_MESSAGES_ADAPTER_ID).is_some());
        assert!(registry.adapter(MINIMAX_MESSAGES_ADAPTER_ID).is_some());

        let (base_descriptor, base_registration) = super::super::claude_messages_adapter();
        let mut base_only = CodecRegistry::default();
        base_only
            .register_codecs(base_descriptor, base_registration)
            .unwrap();
        assert!(base_only.adapter(CLAUDE_MESSAGES_ADAPTER_ID).is_some());
    }
}
