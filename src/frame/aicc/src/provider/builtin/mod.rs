mod anthropic_models;
mod claude;
mod fal;
mod gemini;
mod glm;
mod kimi;
mod minimax;
mod openai;
mod openai_responses_compatible;
mod openrouter;
mod registry;
mod sn;

#[cfg(test)]
fn builtin_catalog_document<T: serde::de::DeserializeOwned>(
    kind: crate::catalog::CatalogKind,
    catalog_id: &str,
) -> T {
    let file = crate::settings::load_builtin_metadata()
        .expect("WP-15 builtin metadata must load")
        .into_iter()
        .find(|file| file.kind == kind && file.catalog_id == catalog_id)
        .unwrap_or_else(|| panic!("WP-15 builtin metadata is missing `{catalog_id}`"));
    serde_json::from_slice(&file.contents)
        .unwrap_or_else(|error| panic!("WP-15 builtin metadata `{catalog_id}` is invalid: {error}"))
}

#[cfg(test)]
fn builtin_known_provider(profile_id: &str) -> crate::catalog::KnownProvider {
    builtin_catalog_document::<crate::catalog::KnownProviderCatalog>(
        crate::catalog::CatalogKind::KnownProvider,
        profile_id,
    )
    .providers
    .into_iter()
    .find(|provider| provider.provider_profile_id == profile_id)
    .unwrap_or_else(|| panic!("Known Provider catalog must contain `{profile_id}`"))
}

#[cfg(test)]
fn builtin_provider_rules(profile_id: &str) -> crate::catalog::ProviderRulesCatalog {
    builtin_catalog_document(crate::catalog::CatalogKind::ProviderRules, profile_id)
}

#[cfg(test)]
fn builtin_model_driver(profile_id: &str) -> crate::catalog::ModelDriverCatalog {
    builtin_catalog_document(crate::catalog::CatalogKind::ModelDriver, profile_id)
}

#[cfg(test)]
fn builtin_profile(
    profile_id: &str,
    discovery_mode: crate::provider::DiscoveryMode,
) -> crate::provider::ProviderProfile {
    let known = builtin_known_provider(profile_id);
    crate::provider::ProviderProfile {
        provider_profile_id: profile_id.to_owned(),
        display_name: known.display_name,
        default_protocol_adapter_id: known.protocol_adapter_id,
        credential: credential_from_catalog(&known.credential),
        credential_variants: known
            .credential_variants
            .iter()
            .map(credential_from_catalog)
            .collect(),
        discovery_mode,
        refresh: crate::provider::RefreshPolicy::default(),
        default_inventory: None,
    }
}

#[cfg(test)]
fn builtin_profile_with_credential(
    profile_id: &str,
    discovery_mode: crate::provider::DiscoveryMode,
    kind: crate::protocol::CredentialKind,
) -> crate::provider::ProviderProfile {
    let mut profile = builtin_profile(profile_id, discovery_mode);
    let credential = std::iter::once(&profile.credential)
        .chain(&profile.credential_variants)
        .find(|credential| credential.kind == kind)
        .cloned()
        .unwrap_or_else(|| panic!("Known Provider `{profile_id}` does not declare `{kind:?}`"));
    profile.credential = credential;
    profile.credential_variants.clear();
    profile
}

#[cfg(test)]
fn credential_from_catalog(
    credential: &crate::catalog::ProviderCredentialDescriptor,
) -> crate::provider::CredentialDescriptor {
    registry::credential_from_catalog(credential)
}

#[cfg(test)]
fn builtin_connection_contract(profile_id: &str) -> crate::provider::ProviderConnectionContract {
    let known = builtin_known_provider(profile_id);
    crate::provider::ProviderConnectionContract {
        default_base_url: known.base_url,
        region: field_from_catalog(&known.connection.region),
        workspace: field_from_catalog(&known.connection.workspace),
        account: field_from_catalog(&known.connection.account),
        region_base_urls: known.connection.region_base_urls,
    }
}

#[cfg(test)]
fn field_from_catalog(
    field: &crate::catalog::ProviderFieldSchema,
) -> crate::provider::ProviderFieldSchema {
    registry::field_from_catalog(field)
}

#[cfg(test)]
fn builtin_catalog_files(catalog_ids: &[&str]) -> Vec<crate::catalog::CurrentCatalogFile> {
    crate::settings::load_builtin_metadata()
        .expect("WP-15 builtin metadata must load")
        .into_iter()
        .filter(|file| catalog_ids.contains(&file.catalog_id.as_str()))
        .map(|file| crate::catalog::CurrentCatalogFile {
            kind: file.kind,
            contents: file.contents,
        })
        .collect()
}

#[allow(unused_imports)]
pub(crate) use claude::*;
#[allow(unused_imports)]
pub(crate) use fal::*;
#[allow(unused_imports)]
pub(crate) use gemini::*;
#[allow(unused_imports)]
pub(crate) use glm::*;
#[allow(unused_imports)]
pub(crate) use kimi::*;
#[allow(unused_imports)]
pub(crate) use minimax::*;
#[allow(unused_imports)]
pub(crate) use openai::*;
#[allow(unused_imports)]
pub(crate) use openai_responses_compatible::*;
#[allow(unused_imports)]
pub(crate) use openrouter::*;
#[allow(unused_imports)]
pub(crate) use registry::*;
#[allow(unused_imports)]
pub(crate) use sn::*;

#[cfg(test)]
mod wp08d_tests {
    use super::*;
    use crate::catalog::{
        CatalogBuildOptions, CatalogDocuments, CatalogSnapshot, KnownProviderCatalog,
        ModelDriverCatalog,
    };
    use crate::protocol::{
        glm_chat_adapter, kimi_chat_adapter, openai_chat_completions_adapter,
        openrouter_chat_adapter, CodecRegistry, GLM_CHAT_ADAPTER_ID, KIMI_CHAT_ADAPTER_ID,
        OPENAI_CHAT_COMPLETIONS_OPERATION_ID, OPENROUTER_CHAT_ADAPTER_ID,
    };
    use crate::provider::{
        CredentialReference, DiscoveredModel, InventoryBuilder, ModelAvailability,
        ProviderDiscoverySnapshot, ProviderHealthState, ProviderInstanceConfig,
    };
    use crate::settings::{MetadataFile, MetadataSource, MetadataSources};
    use buckyos_api::ApiType;
    use serde_json::json;
    use std::collections::BTreeSet;

    fn driver(id: &str, model: &str) -> ModelDriverCatalog {
        serde_json::from_value(json!({
            "format": "buckyos.aicc.model-driver-catalog",
            "schema_version": 1,
            "schema_revision": 0,
            "model_driver_id": id,
            "revision_seq": 1,
            "models": [{"id": model, "api_types": ["llm"]}],
            "patterns": [],
            "defaults": {},
            "variants": [],
            "version_rules": []
        }))
        .unwrap()
    }

    fn instance(profile: &str, adapter: &str) -> ProviderInstanceConfig {
        ProviderInstanceConfig {
            provider_instance_name: format!("{profile}-main"),
            provider_profile_id: profile.to_owned(),
            protocol_adapter_id: adapter.to_owned(),
            base_url: "https://example.test/v1".to_owned(),
            credential: CredentialReference {
                reference: format!("secret://{profile}"),
            },
            credential_kind: None,
            provider_rules_id: Some(profile.to_owned()),
            region: None,
            workspace: None,
            account: None,
        }
    }

    fn discovery(
        provider_model_id: &str,
        origin_model_id: Option<&str>,
    ) -> ProviderDiscoverySnapshot {
        ProviderDiscoverySnapshot {
            revision: Some("fixture-1".to_owned()),
            discovered_at_ms: 1,
            health: ProviderHealthState::Healthy,
            models: vec![DiscoveredModel {
                provider_model_id: provider_model_id.to_owned(),
                origin_model_id: origin_model_id.map(str::to_owned),
                api_types: Some(vec![ApiType::Llm]),
                supported_features: None,
                remote_methods: Some(BTreeSet::from([
                    OPENAI_CHAT_COMPLETIONS_OPERATION_ID.to_owned()
                ])),
                availability: ModelAvailability::Available,
                deprecated: false,
                pricing: None,
            }],
        }
    }

    #[test]
    fn wp08d_profiles_rules_dialects_and_inventory_form_complete_identity_chains() {
        let catalog = CatalogSnapshot::build(
            1,
            CatalogDocuments {
                model_drivers: vec![
                    driver("openai", "router-model"),
                    driver("claude", "router-model"),
                    driver("gemini", "gemini-fixture"),
                    kimi_model_driver(),
                    glm_model_driver(),
                ],
                provider_rules: vec![
                    openrouter_provider_rules(1),
                    kimi_provider_rules(1),
                    glm_provider_rules(1),
                ],
                known_providers: vec![KnownProviderCatalog {
                    format: "buckyos.aicc.known-provider-catalog".to_owned(),
                    schema_version: 1,
                    schema_revision: 1,
                    revision_seq: 1,
                    catalog_id: "wp08d-builtins".to_owned(),
                    providers: vec![
                        openrouter_known_provider(),
                        kimi_known_provider(),
                        glm_known_provider(),
                    ],
                }],
            },
            &CatalogBuildOptions::default(),
        )
        .unwrap();
        let mut codecs = CodecRegistry::default();
        let (base, registration) = openai_chat_completions_adapter();
        codecs.register_codecs(base, registration).unwrap();
        for (descriptor, registration) in [
            openrouter_chat_adapter(),
            kimi_chat_adapter(),
            glm_chat_adapter(),
        ] {
            codecs.register_derived(descriptor, registration).unwrap();
        }

        let cases = [
            (
                openrouter_profile(),
                instance("openrouter", OPENROUTER_CHAT_ADAPTER_ID),
                discovery("openai/router-model", Some("router-model")),
                "openai",
            ),
            (
                kimi_profile(),
                instance("kimi", KIMI_CHAT_ADAPTER_ID),
                discovery("kimi-k2.6", None),
                "kimi",
            ),
            (
                glm_profile(),
                instance("glm", GLM_CHAT_ADAPTER_ID),
                discovery("glm-5.1", None),
                "glm",
            ),
        ];
        for (profile, instance, discovered, expected_driver) in cases {
            let inventory =
                InventoryBuilder::build(&profile, &instance, discovered, &catalog, &codecs)
                    .unwrap();
            assert_eq!(inventory.provider_profile_id, profile.provider_profile_id);
            assert_eq!(
                inventory.protocol_adapter_id,
                profile.default_protocol_adapter_id
            );
            assert_eq!(inventory.models.len(), 1);
            assert_eq!(inventory.models[0].model_driver_id, expected_driver);
            assert_eq!(
                inventory.models[0].operations["llm"],
                OPENAI_CHAT_COMPLETIONS_OPERATION_ID
            );
        }
    }

    #[test]
    fn wp15_metadata_sources_load_the_complete_wp08d_builtin_set() {
        let mut builtin = openrouter_catalog_files()
            .into_iter()
            .chain(kimi_catalog_files())
            .chain(glm_catalog_files())
            .map(|file| MetadataFile::parse(MetadataSource::Builtin, file.kind, file.contents))
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        for model_driver in [
            driver("openai", "gpt-5.6"),
            driver("claude", "claude-fixture"),
            driver("gemini", "gemini-fixture"),
        ] {
            builtin.push(
                MetadataFile::parse(
                    MetadataSource::Builtin,
                    crate::catalog::CatalogKind::ModelDriver,
                    serde_json::to_vec(&model_driver).unwrap(),
                )
                .unwrap(),
            );
        }
        let catalog = MetadataSources {
            builtin,
            ..MetadataSources::default()
        }
        .build_snapshot(1, &CatalogBuildOptions::default())
        .unwrap();

        for provider in ["openrouter", "kimi", "glm"] {
            assert!(catalog.known_provider(provider).is_some());
            assert!(catalog.provider_rules(provider).is_some());
        }
        assert!(catalog.model_driver("openrouter").is_none());
        assert!(catalog.model_driver("kimi").is_some());
        assert!(catalog.model_driver("glm").is_some());
    }
}
