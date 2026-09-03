use super::super::{
    CredentialDescriptor, DiscoveryMode, ProviderConnectionContract, ProviderFieldSchema,
    ProviderProfile, RefreshPolicy,
};
use super::anthropic_models::{AnthropicModelsDiscovery, AnthropicModelsSpec};
use crate::catalog::{
    CatalogKind, CurrentCatalogFile, KnownProvider, KnownProviderCatalog, ModelDriverCatalog,
    ProviderRulesCatalog,
};
use crate::protocol::{ClaudeMessagesCodec, CodecRegistration, CredentialKind, HttpTransport};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::sync::Arc;

pub(crate) const CLAUDE_PROVIDER_PROFILE_ID: &str = "claude";

const CLAUDE_PROVIDER_RULES: &[u8] =
    include_bytes!("../../../driver_metadata/providers/claude.provider.json");
const CLAUDE_KNOWN_PROVIDER: &[u8] =
    include_bytes!("../../../driver_metadata/known-providers/claude.known-provider.json");
const CLAUDE_MODEL_DRIVER: &[u8] =
    include_bytes!("../../../driver_metadata/models/anthropic.model.json");

pub(super) const CLAUDE_SPEC: AnthropicModelsSpec = AnthropicModelsSpec {
    profile: claude_profile,
    version_header: true,
    connection_contract: claude_connection_contract,
    label: "Claude",
};

pub(crate) fn claude_profile() -> ProviderProfile {
    let known = claude_known_provider();
    let credential: CredentialDeclaration = embedded_value(
        &known,
        "credential",
        "Claude Known Provider credential declaration",
    );
    assert_eq!(credential.kind, "named_header");
    assert!(credential.required && credential.secret);
    ProviderProfile {
        provider_profile_id: CLAUDE_PROVIDER_PROFILE_ID.to_owned(),
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

pub(crate) fn claude_connection_contract() -> ProviderConnectionContract {
    let known = claude_known_provider();
    let fields: InstanceFieldDeclarations = embedded_value(
        &known,
        "instance_fields",
        "Claude Known Provider instance fields",
    );
    ProviderConnectionContract {
        default_base_url: known.base_url,
        region: fields.region,
        workspace: fields.workspace,
        account: fields.account,
    }
}

pub(crate) fn claude_known_provider() -> KnownProvider {
    embedded_json::<KnownProviderCatalog>(CLAUDE_KNOWN_PROVIDER, "Claude Known Provider catalog")
        .providers
        .into_iter()
        .find(|provider| provider.provider_profile_id == CLAUDE_PROVIDER_PROFILE_ID)
        .expect("Claude Known Provider catalog must contain the Claude profile")
}

pub(crate) fn claude_provider_rules(_revision_seq: u64) -> ProviderRulesCatalog {
    embedded_json(CLAUDE_PROVIDER_RULES, "Claude Provider Rules catalog")
}

pub(crate) fn claude_model_driver() -> ModelDriverCatalog {
    embedded_json(CLAUDE_MODEL_DRIVER, "Claude Model Driver catalog")
}

pub(crate) fn claude_catalog_files() -> Vec<CurrentCatalogFile> {
    [
        (CatalogKind::KnownProvider, CLAUDE_KNOWN_PROVIDER),
        (CatalogKind::ProviderRules, CLAUDE_PROVIDER_RULES),
        (CatalogKind::ModelDriver, CLAUDE_MODEL_DRIVER),
    ]
    .into_iter()
    .map(|(kind, contents)| CurrentCatalogFile {
        kind,
        contents: contents.to_vec(),
    })
    .collect()
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

pub(crate) fn claude_discovery(transport: HttpTransport) -> AnthropicModelsDiscovery {
    AnthropicModelsDiscovery::new(CLAUDE_SPEC, transport)
}

pub(crate) fn claude_messages_adapter() -> (crate::protocol::AdapterDescriptor, CodecRegistration) {
    let codec = ClaudeMessagesCodec::new();
    (
        codec.adapter_descriptor(),
        CodecRegistration {
            operation_codecs: vec![Arc::new(codec)],
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
        assert_eq!(registration.operation_codecs.len(), 1);
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
