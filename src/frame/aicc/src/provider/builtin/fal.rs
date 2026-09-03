use super::super::{
    CatalogOnlyDiscovery, CredentialDescriptor, DiscoveryMode, ProviderConnectionContract,
    ProviderDiscovery, ProviderDiscoverySnapshot, ProviderFieldSchema, ProviderProfile,
    ProviderResult, RefreshPolicy, validate_discovery,
};
use crate::catalog::{
    CatalogKind, CurrentCatalogFile, KnownProvider, KnownProviderCatalog, ProviderRulesCatalog,
};
use crate::protocol::{CredentialKind, FAL_QUEUE_ADAPTER_ID};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::sync::Arc;

pub(crate) const FAL_PROVIDER_PROFILE_ID: &str = "fal";

const FAL_KNOWN_PROVIDER: &[u8] =
    include_bytes!("../../../driver_metadata/known-providers/fal.known-provider.json");
const FAL_PROVIDER_RULES: &[u8] =
    include_bytes!("../../../driver_metadata/providers/fal.provider.json");

pub(crate) fn fal_profile() -> ProviderProfile {
    let known = fal_known_provider();
    let credential: CredentialDeclaration = embedded_value(
        &known,
        "credential",
        "fal Known Provider credential declaration",
    );
    assert!(credential.required && credential.secret);
    assert_eq!(credential.header_name, "Authorization");
    assert_eq!(credential.prefix, "Key");
    assert_eq!(known.protocol_adapter_id, FAL_QUEUE_ADAPTER_ID);
    let credential = match credential.kind.as_str() {
        "fal_key" => CredentialDescriptor {
            kind: CredentialKind::FalKey,
            header_name: None,
        },
        kind => panic!("fal Known Provider uses unsupported credential kind `{kind}`"),
    };
    ProviderProfile {
        provider_profile_id: FAL_PROVIDER_PROFILE_ID.to_owned(),
        display_name: known.display_name,
        default_protocol_adapter_id: known.protocol_adapter_id,
        credential,
        discovery_mode: DiscoveryMode::CatalogOnly,
        refresh: RefreshPolicy::default(),
        default_inventory: None,
    }
}

pub(crate) fn fal_known_provider() -> KnownProvider {
    embedded_json::<KnownProviderCatalog>(FAL_KNOWN_PROVIDER, "fal Known Provider catalog")
        .providers
        .into_iter()
        .find(|provider| provider.provider_profile_id == FAL_PROVIDER_PROFILE_ID)
        .expect("fal Known Provider catalog must contain the fal profile")
}

pub(crate) fn fal_connection_contract() -> ProviderConnectionContract {
    let known = fal_known_provider();
    let fields: InstanceFieldDeclarations = embedded_value(
        &known,
        "instance_fields",
        "fal Known Provider instance fields",
    );
    ProviderConnectionContract {
        default_base_url: known.base_url,
        region: fields.region,
        workspace: fields.workspace,
        account: fields.account,
    }
}

pub(crate) fn fal_provider_rules(_revision_seq: u64) -> ProviderRulesCatalog {
    embedded_json(FAL_PROVIDER_RULES, "fal Provider Rules catalog")
}

pub(crate) fn fal_catalog_files() -> Vec<CurrentCatalogFile> {
    [
        (CatalogKind::KnownProvider, FAL_KNOWN_PROVIDER),
        (CatalogKind::ProviderRules, FAL_PROVIDER_RULES),
    ]
    .into_iter()
    .map(|(kind, contents)| CurrentCatalogFile {
        kind,
        contents: contents.to_vec(),
    })
    .collect()
}

pub(crate) fn fal_discovery(
    configured_inventory: ProviderDiscoverySnapshot,
) -> ProviderResult<Arc<dyn ProviderDiscovery>> {
    validate_discovery(&configured_inventory)?;
    Ok(Arc::new(CatalogOnlyDiscovery::new(configured_inventory)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialDeclaration {
    kind: String,
    header_name: String,
    prefix: String,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CatalogBuildOptions, CatalogSnapshot, ModelDriverCatalog};
    use crate::protocol::{
        CodecRegistry, FAL_QUEUE_OPERATION_ID, ResolvedCredential, fal_queue_adapter,
    };
    use crate::provider::{
        CredentialReference, DiscoveredModel, DiscoveryContext, InventoryBuilder,
        ModelAvailability, ProviderHealthState, ProviderInstanceConfig,
    };
    use buckyos_api::ApiType;
    use serde_json::json;
    use std::collections::BTreeSet;

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

    fn instance() -> ProviderInstanceConfig {
        ProviderInstanceConfig {
            provider_instance_name: "fal-main".to_owned(),
            provider_profile_id: FAL_PROVIDER_PROFILE_ID.to_owned(),
            protocol_adapter_id: FAL_QUEUE_ADAPTER_ID.to_owned(),
            base_url: fal_connection_contract().default_base_url,
            credential: CredentialReference {
                reference: "secret://fal/main".to_owned(),
            },
            provider_rules_id: Some(FAL_PROVIDER_PROFILE_ID.to_owned()),
            region: None,
            account: None,
        }
    }

    #[test]
    fn provider_information_and_rules_come_from_builtin_catalog_files() {
        let profile = fal_profile();
        profile.validate().unwrap();
        assert_eq!(profile.provider_profile_id, FAL_PROVIDER_PROFILE_ID);
        assert_eq!(profile.default_protocol_adapter_id, FAL_QUEUE_ADAPTER_ID);
        assert_eq!(profile.credential.kind, CredentialKind::FalKey);
        assert_eq!(profile.discovery_mode, DiscoveryMode::CatalogOnly);
        assert!(profile.default_inventory.is_none());

        let known = fal_known_provider();
        assert_eq!(known.base_url, "https://queue.fal.run");
        assert_eq!(known.ui_hints["credential"]["prefix"], "Key");
        assert_eq!(
            known.ui_hints["instance_fields"]["workspace"],
            json!({"mode": "unsupported"})
        );
        let connection = fal_connection_contract();
        assert_eq!(
            connection.resolve(Default::default()).unwrap().base_url,
            "https://queue.fal.run"
        );

        let rules = fal_provider_rules(7);
        assert_eq!(rules.revision_seq, 1);
        assert_eq!(
            rules.patterns[0].operations["video.img2video"],
            FAL_QUEUE_OPERATION_ID
        );
        assert!(rules.metadata_drivers.is_none());

        let files = fal_catalog_files();
        assert_eq!(files.len(), 2);
        assert!(
            files
                .iter()
                .all(|file| file.kind != CatalogKind::ModelDriver)
        );
        let catalog =
            CatalogSnapshot::from_current_files(1, files, &CatalogBuildOptions::default()).unwrap();
        assert!(catalog.known_provider(FAL_PROVIDER_PROFILE_ID).is_some());
        assert!(catalog.provider_rules(FAL_PROVIDER_PROFILE_ID).is_some());
    }

    #[tokio::test]
    async fn catalog_only_discovery_uses_configured_inventory_without_builtin_models() {
        let profile = fal_profile();
        let instance = instance();
        let configured = ProviderDiscoverySnapshot {
            revision: Some("configured-v1".to_owned()),
            discovered_at_ms: 1,
            health: ProviderHealthState::Healthy,
            models: vec![
                model("vendor/image-upscaler", [ApiType::ImageUpscale]),
                model("vendor/audio-enhancer", [ApiType::AudioEnhance]),
                model("vendor/video-upscaler", [ApiType::VideoUpscale]),
            ],
        };
        let credential = ResolvedCredential::fal_key("secret://fal/main", "secret").unwrap();
        let snapshot = fal_discovery(configured.clone())
            .unwrap()
            .discover(&DiscoveryContext {
                profile: &profile,
                instance: &instance,
                credential: &credential,
            })
            .await
            .unwrap();
        assert_eq!(snapshot, configured);

        let duplicate = ProviderDiscoverySnapshot {
            revision: None,
            discovered_at_ms: 1,
            health: ProviderHealthState::Healthy,
            models: vec![
                model("same", [ApiType::ImageUpscale]),
                model("same", [ApiType::VideoUpscale]),
            ],
        };
        assert!(fal_discovery(duplicate).is_err());
    }

    #[test]
    fn configured_rules_and_adapter_build_complete_inventory_identity() {
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
        let mut files = fal_catalog_files();
        files.push(CurrentCatalogFile {
            kind: CatalogKind::ModelDriver,
            contents: serde_json::to_vec(&driver).unwrap(),
        });
        let catalog =
            CatalogSnapshot::from_current_files(1, files, &CatalogBuildOptions::default()).unwrap();
        let (descriptor, registration) = fal_queue_adapter();
        let mut codecs = CodecRegistry::default();
        codecs.register_codecs(descriptor, registration).unwrap();
        let inventory = InventoryBuilder::build(
            &fal_profile(),
            &instance(),
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
