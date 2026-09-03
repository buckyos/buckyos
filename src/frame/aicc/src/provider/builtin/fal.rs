use super::super::{
    CatalogOnlyDiscovery, CredentialDescriptor, DiscoveredModel, DiscoveryMode, ModelAvailability,
    ProviderDiscovery, ProviderDiscoverySnapshot, ProviderHealthState, ProviderProfile,
    RefreshPolicy,
};
use crate::catalog::{KnownProvider, ProviderPatternRule, ProviderRulesCatalog};
use crate::matching::MatchRule;
use crate::protocol::{CredentialKind, FAL_QUEUE_ADAPTER_ID, FAL_QUEUE_OPERATION_ID};
use buckyos_api::ApiType;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub(crate) const FAL_PROVIDER_PROFILE_ID: &str = "fal";
pub(crate) const FAL_DISPLAY_NAME: &str = "fal";
pub(crate) const FAL_DEFAULT_BASE_URL: &str = "https://queue.fal.run";

pub(crate) fn fal_profile() -> ProviderProfile {
    ProviderProfile {
        provider_profile_id: FAL_PROVIDER_PROFILE_ID.to_owned(),
        display_name: FAL_DISPLAY_NAME.to_owned(),
        default_protocol_adapter_id: FAL_QUEUE_ADAPTER_ID.to_owned(),
        credential: CredentialDescriptor {
            kind: CredentialKind::FalKey,
            header_name: None,
        },
        discovery_mode: DiscoveryMode::CatalogOnly,
        refresh: RefreshPolicy::default(),
        default_inventory: Some(fal_catalog_inventory()),
    }
}

pub(crate) fn fal_known_provider() -> KnownProvider {
    KnownProvider {
        provider_profile_id: FAL_PROVIDER_PROFILE_ID.to_owned(),
        display_name: FAL_DISPLAY_NAME.to_owned(),
        base_url: FAL_DEFAULT_BASE_URL.to_owned(),
        protocol_adapter_id: FAL_QUEUE_ADAPTER_ID.to_owned(),
        provider_rules_id: Some(FAL_PROVIDER_PROFILE_ID.to_owned()),
        ui_hints: BTreeMap::from([
            (
                "credential".to_owned(),
                json!({
                    "kind": "fal_key",
                    "header_name": "Authorization",
                    "prefix": "Key",
                    "required": true,
                    "secret": true
                }),
            ),
            (
                "instance_fields".to_owned(),
                json!({
                    "region": "unsupported",
                    "workspace": "unsupported",
                    "account": "unsupported"
                }),
            ),
        ]),
    }
}

pub(crate) fn fal_provider_rules(revision_seq: u64) -> ProviderRulesCatalog {
    ProviderRulesCatalog {
        format: "buckyos.aicc.provider-rules-catalog".to_owned(),
        schema_version: 1,
        schema_revision: 0,
        revision_seq,
        provider_profile_id: FAL_PROVIDER_PROFILE_ID.to_owned(),
        metadata_drivers: None,
        origin_provider_aliases: BTreeMap::new(),
        origin_mappings: Vec::new(),
        models: Vec::new(),
        patterns: vec![ProviderPatternRule {
            match_rule: MatchRule::Shorthand("*".to_owned()),
            exclude: false,
            operations: [
                "image.txt2img",
                "image.img2img",
                "image.inpaint",
                "image.upscale",
                "image.bg_remove",
                "audio.tts",
                "audio.asr",
                "audio.music",
                "audio.enhance",
                "video.txt2video",
                "video.img2video",
                "video.video2video",
                "video.extend",
                "video.upscale",
            ]
            .into_iter()
            .map(|api_type| (api_type.to_owned(), FAL_QUEUE_OPERATION_ID.to_owned()))
            .collect(),
            provider_options: BTreeMap::new(),
            request_rules: Vec::new(),
            pricing: None,
            remove_api_types: BTreeSet::new(),
            remove_features: BTreeSet::new(),
            estimated_latency_ms: None,
            latency_class: Some("async".to_owned()),
            cost_class: None,
        }],
        variants: Vec::new(),
    }
}

pub(crate) fn fal_catalog_inventory() -> ProviderDiscoverySnapshot {
    ProviderDiscoverySnapshot {
        revision: Some("builtin-fal-v1".to_owned()),
        discovered_at_ms: 0,
        health: ProviderHealthState::Healthy,
        models: vec![
            model("fal-ai/esrgan", [ApiType::ImageUpscale]),
            model("fal-ai/imageutils/rembg", [ApiType::ImageBackgroundRemove]),
            model("fal-ai/deepfilternet3", [ApiType::AudioEnhance]),
            model("fal-ai/video-upscaler", [ApiType::VideoUpscale]),
        ],
    }
}

pub(crate) fn fal_discovery() -> Arc<dyn ProviderDiscovery> {
    Arc::new(CatalogOnlyDiscovery::new(fal_catalog_inventory()))
}

fn model<const N: usize>(provider_model_id: &str, api_types: [ApiType; N]) -> DiscoveredModel {
    DiscoveredModel {
        provider_model_id: provider_model_id.to_owned(),
        origin_model_id: None,
        api_types: Some(api_types.into_iter().collect()),
        supported_features: Some(BTreeSet::new()),
        remote_methods: Some(BTreeSet::from([FAL_QUEUE_OPERATION_ID.to_owned()])),
        availability: ModelAvailability::Available,
        deprecated: false,
        pricing: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{
        CatalogBuildOptions, CatalogDocuments, CatalogSnapshot, KnownProviderCatalog,
        ModelDriverCatalog,
    };
    use crate::protocol::{fal_queue_adapter, CodecRegistry, ResolvedCredential};
    use crate::provider::{
        CredentialReference, DiscoveryContext, InventoryBuilder, ProviderInstanceConfig,
    };

    #[test]
    fn profile_known_provider_and_rules_form_stable_builtin_contract() {
        let profile = fal_profile();
        profile.validate().unwrap();
        assert_eq!(profile.provider_profile_id, FAL_PROVIDER_PROFILE_ID);
        assert_eq!(profile.default_protocol_adapter_id, FAL_QUEUE_ADAPTER_ID);
        assert_eq!(profile.credential.kind, CredentialKind::FalKey);
        assert_eq!(profile.discovery_mode, DiscoveryMode::CatalogOnly);

        let known = fal_known_provider();
        assert_eq!(known.base_url, FAL_DEFAULT_BASE_URL);
        assert_eq!(known.ui_hints["credential"]["prefix"], "Key");
        assert_eq!(
            known.ui_hints["instance_fields"]["workspace"],
            "unsupported"
        );

        let rules = fal_provider_rules(7);
        assert_eq!(rules.revision_seq, 7);
        assert_eq!(
            rules.patterns[0].operations["video.img2video"],
            FAL_QUEUE_OPERATION_ID
        );
        assert!(rules.metadata_drivers.is_none());
    }

    #[tokio::test]
    async fn catalog_only_discovery_exposes_image_audio_and_video_endpoints() {
        let profile = fal_profile();
        let instance = ProviderInstanceConfig {
            provider_instance_name: "fal-main".to_owned(),
            provider_profile_id: FAL_PROVIDER_PROFILE_ID.to_owned(),
            protocol_adapter_id: FAL_QUEUE_ADAPTER_ID.to_owned(),
            base_url: FAL_DEFAULT_BASE_URL.to_owned(),
            credential: CredentialReference {
                reference: "secret://fal/main".to_owned(),
            },
            provider_rules_id: Some(FAL_PROVIDER_PROFILE_ID.to_owned()),
            region: None,
            account: None,
        };
        let credential = ResolvedCredential::fal_key("secret://fal/main", "secret").unwrap();
        let snapshot = fal_discovery()
            .discover(&DiscoveryContext {
                profile: &profile,
                instance: &instance,
                credential: &credential,
            })
            .await
            .unwrap();
        assert_eq!(snapshot.health, ProviderHealthState::Healthy);
        assert!(snapshot.models.iter().any(|model| {
            model
                .api_types
                .as_ref()
                .is_some_and(|types| types.contains(&ApiType::ImageUpscale))
        }));
        assert!(snapshot.models.iter().any(|model| {
            model
                .api_types
                .as_ref()
                .is_some_and(|types| types.contains(&ApiType::AudioEnhance))
        }));
        assert!(snapshot.models.iter().any(|model| {
            model
                .api_types
                .as_ref()
                .is_some_and(|types| types.contains(&ApiType::VideoUpscale))
        }));
    }

    #[test]
    fn rules_and_adapter_build_complete_inventory_identity() {
        let driver: ModelDriverCatalog = serde_json::from_value(json!({
            "format": "buckyos.aicc.model-driver-catalog",
            "schema_version": 1,
            "schema_revision": 0,
            "model_driver_id": "fal-fixture",
            "revision_seq": 1,
            "models": [{
                "id": "fal-fixture-model",
                "api_types": ["image.txt2img", "video.img2video"]
            }],
            "patterns": [],
            "defaults": {},
            "variants": [],
            "version_rules": []
        }))
        .unwrap();
        let catalog = CatalogSnapshot::build(
            1,
            CatalogDocuments {
                model_drivers: vec![driver],
                provider_rules: vec![fal_provider_rules(1)],
                known_providers: vec![KnownProviderCatalog {
                    format: "buckyos.aicc.known-provider-catalog".to_owned(),
                    schema_version: 1,
                    schema_revision: 0,
                    revision_seq: 1,
                    catalog_id: "builtin".to_owned(),
                    providers: vec![fal_known_provider()],
                }],
            },
            &CatalogBuildOptions::default(),
        )
        .unwrap();
        let (descriptor, registration) = fal_queue_adapter();
        let mut codecs = CodecRegistry::default();
        codecs.register_codecs(descriptor, registration).unwrap();
        let inventory = InventoryBuilder::build(
            &fal_profile(),
            &ProviderInstanceConfig {
                provider_instance_name: "fal-main".to_owned(),
                provider_profile_id: FAL_PROVIDER_PROFILE_ID.to_owned(),
                protocol_adapter_id: FAL_QUEUE_ADAPTER_ID.to_owned(),
                base_url: FAL_DEFAULT_BASE_URL.to_owned(),
                credential: CredentialReference {
                    reference: "secret://fal/main".to_owned(),
                },
                provider_rules_id: Some(FAL_PROVIDER_PROFILE_ID.to_owned()),
                region: None,
                account: None,
            },
            ProviderDiscoverySnapshot {
                revision: Some("fixture-v1".to_owned()),
                discovered_at_ms: 1,
                health: ProviderHealthState::Healthy,
                models: vec![model(
                    "fal-fixture-model",
                    [ApiType::ImageTextToImage, ApiType::VideoImageToVideo],
                )],
            },
            &catalog,
            &codecs,
        )
        .unwrap();
        assert_eq!(inventory.models.len(), 1);
        assert_eq!(inventory.models[0].model_driver_id, "fal-fixture");
        assert_eq!(
            inventory.models[0].operations["video.img2video"],
            FAL_QUEUE_OPERATION_ID
        );
        assert_eq!(inventory.protocol_adapter_id, FAL_QUEUE_ADAPTER_ID);
    }
}
