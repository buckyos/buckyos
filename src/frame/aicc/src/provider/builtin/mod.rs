mod fal;
mod gemini;
mod glm;
mod kimi;
mod openai;
mod openrouter;
mod sn;
mod wp08e;

#[allow(unused_imports)]
pub(crate) use fal::*;
#[allow(unused_imports)]
pub(crate) use gemini::*;
#[allow(unused_imports)]
pub(crate) use glm::*;
#[allow(unused_imports)]
pub(crate) use kimi::*;
#[allow(unused_imports)]
pub(crate) use openai::*;
#[allow(unused_imports)]
pub(crate) use openrouter::*;
#[allow(unused_imports)]
pub(crate) use sn::*;
#[allow(unused_imports)]
pub(crate) use wp08e::*;

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
            provider_rules_id: Some(profile.to_owned()),
            region: None,
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
                    driver("kimi", "kimi-model"),
                    driver("glm", "glm-model"),
                ],
                provider_rules: vec![
                    openrouter_provider_rules(1),
                    kimi_provider_rules(1),
                    glm_provider_rules(1),
                ],
                known_providers: vec![KnownProviderCatalog {
                    format: "buckyos.aicc.known-provider-catalog".to_owned(),
                    schema_version: 1,
                    schema_revision: 0,
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
                discovery("kimi-model", None),
                "kimi",
            ),
            (
                glm_profile(),
                instance("glm", GLM_CHAT_ADAPTER_ID),
                discovery("glm-model", None),
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
}
