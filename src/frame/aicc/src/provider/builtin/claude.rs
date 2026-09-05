#[cfg(test)]
use super::super::{
    DiscoveryMode, ProviderConnectionContract, ProviderProfile,
};
use super::anthropic_models::{AnthropicModelsDiscovery, AnthropicModelsSpec};
#[cfg(test)]
use crate::catalog::{CurrentCatalogFile, ModelDriverCatalog, ProviderRulesCatalog};
#[cfg(test)]
use crate::protocol::CredentialKind;
use crate::protocol::{ClaudeMessagesCodec, CodecRegistration, HttpTransport};
#[cfg(test)]
use serde::de::DeserializeOwned;
use std::sync::Arc;

pub(crate) const CLAUDE_PROVIDER_PROFILE_ID: &str = "claude";

pub(super) const CLAUDE_SPEC: AnthropicModelsSpec = AnthropicModelsSpec {
    provider_profile_id: CLAUDE_PROVIDER_PROFILE_ID,
    version_header: true,
    label: "Claude",
};

#[cfg(test)]
pub(crate) fn claude_profile() -> ProviderProfile {
    super::builtin_profile(CLAUDE_PROVIDER_PROFILE_ID, DiscoveryMode::MachineApi)
}

#[cfg(test)]
pub(crate) fn claude_connection_contract() -> ProviderConnectionContract {
    super::builtin_connection_contract(CLAUDE_PROVIDER_PROFILE_ID)
}

#[cfg(test)]
pub(crate) fn claude_known_provider() -> crate::catalog::KnownProvider {
    super::builtin_known_provider(CLAUDE_PROVIDER_PROFILE_ID)
}

#[cfg(test)]
pub(crate) fn claude_provider_rules(_revision_seq: u64) -> ProviderRulesCatalog {
    super::builtin_provider_rules(CLAUDE_PROVIDER_PROFILE_ID)
}

#[cfg(test)]
pub(crate) fn claude_model_driver() -> ModelDriverCatalog {
    super::builtin_model_driver(CLAUDE_PROVIDER_PROFILE_ID)
}

#[cfg(test)]
pub(crate) fn claude_catalog_files() -> Vec<CurrentCatalogFile> {
    super::builtin_catalog_files(&[CLAUDE_PROVIDER_PROFILE_ID])
}

#[cfg(test)]
fn embedded_json<T: DeserializeOwned>(contents: &[u8], label: &str) -> T {
    serde_json::from_slice(contents).unwrap_or_else(|error| panic!("{label} is invalid: {error}"))
}

pub(crate) fn claude_discovery(transport: HttpTransport) -> AnthropicModelsDiscovery {
    AnthropicModelsDiscovery::new(CLAUDE_SPEC, transport)
}

pub(crate) fn claude_messages_adapter() -> (crate::protocol::AdapterDescriptor, CodecRegistration) {
    let codec = ClaudeMessagesCodec::new();
    let descriptor = codec.adapter_descriptor();
    (
        descriptor,
        CodecRegistration {
            operation_codecs: vec![
                Arc::new(codec),
                Arc::new(ClaudeMessagesCodec::new_for(
                    buckyos_api::ApiType::VisionOcr,
                )),
                Arc::new(ClaudeMessagesCodec::new_for(
                    buckyos_api::ApiType::VisionCaption,
                )),
            ],
            native_task_codecs: Vec::new(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::CatalogBuildOptions;
    use crate::protocol::{CLAUDE_MESSAGES_ADAPTER_ID, CLAUDE_MESSAGES_OPERATION_ID};
    use crate::settings::{MetadataFile, MetadataSource, MetadataSources};

    #[test]
    fn embedded_catalogs_drive_profile_rules_and_models() {
        let profile = claude_profile();
        let known = claude_known_provider();
        let rules = claude_provider_rules(7);
        let models = claude_model_driver();
        let (adapter, registration) = claude_messages_adapter();

        assert_eq!(profile.provider_profile_id, CLAUDE_PROVIDER_PROFILE_ID);
        assert_eq!(
            profile.default_protocol_adapter_id,
            CLAUDE_MESSAGES_ADAPTER_ID
        );
        assert_eq!(profile.credential.kind, CredentialKind::NamedHeader);
        assert_eq!(
            known.base_url,
            claude_connection_contract().default_base_url
        );
        assert_eq!(
            known.provider_rules_id.as_deref(),
            Some(CLAUDE_PROVIDER_PROFILE_ID)
        );
        assert_eq!(
            rules.metadata_drivers,
            Some(vec![CLAUDE_PROVIDER_PROFILE_ID.to_owned()])
        );
        assert_eq!(
            rules.patterns[0].operations["llm"],
            CLAUDE_MESSAGES_OPERATION_ID
        );
        assert_eq!(models.model_driver_id, CLAUDE_PROVIDER_PROFILE_ID);
        assert!(models
            .models
            .iter()
            .any(|model| model.id == "claude-sonnet-5"));
        let sonnet = models
            .models
            .iter()
            .find(|model| model.id == "claude-sonnet-5")
            .unwrap();
        assert_eq!(
            sonnet.capabilities.as_ref().unwrap()["max_context_tokens"],
            1_000_000
        );
        assert_eq!(sonnet.pricing.as_ref().unwrap().input_token, Some(0.000003));
        assert_eq!(adapter.base_adapter_id, None);
        assert_eq!(registration.operation_codecs.len(), 3);
        let builtin = claude_catalog_files()
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
        assert!(catalog.known_provider(CLAUDE_PROVIDER_PROFILE_ID).is_some());
        assert!(catalog.provider_rules(CLAUDE_PROVIDER_PROFILE_ID).is_some());
        assert!(catalog.model_driver(CLAUDE_PROVIDER_PROFILE_ID).is_some());
    }
}
