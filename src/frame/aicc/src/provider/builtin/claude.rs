use super::super::{
    CredentialDescriptor, DiscoveryMode, ProviderConnectionContract, ProviderFieldSchema,
    ProviderProfile, RefreshPolicy,
};
use super::anthropic_models::{AnthropicModelsDiscovery, AnthropicModelsSpec};
use crate::catalog::{KnownProvider, ProviderPatternRule, ProviderRulesCatalog};
use crate::matching::MatchRule;
use crate::protocol::{
    ClaudeMessagesCodec, CodecRegistration, CredentialKind, HttpTransport,
    CLAUDE_MESSAGES_ADAPTER_ID, CLAUDE_MESSAGES_OPERATION_ID,
};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub(crate) const CLAUDE_PROVIDER_PROFILE_ID: &str = "claude";
pub(crate) const CLAUDE_DISPLAY_NAME: &str = "Anthropic Claude";
pub(crate) const CLAUDE_DEFAULT_BASE_URL: &str = "https://api.anthropic.com/v1";
pub(crate) const CLAUDE_CREDENTIAL_HEADER: &str = "x-api-key";

pub(super) const CLAUDE_SPEC: AnthropicModelsSpec = AnthropicModelsSpec {
    provider_profile_id: CLAUDE_PROVIDER_PROFILE_ID,
    protocol_adapter_id: CLAUDE_MESSAGES_ADAPTER_ID,
    version_header: true,
    connection_contract: claude_connection_contract,
    label: "Claude",
};

pub(crate) fn claude_profile() -> ProviderProfile {
    ProviderProfile {
        provider_profile_id: CLAUDE_PROVIDER_PROFILE_ID.to_owned(),
        display_name: CLAUDE_DISPLAY_NAME.to_owned(),
        default_protocol_adapter_id: CLAUDE_MESSAGES_ADAPTER_ID.to_owned(),
        credential: CredentialDescriptor {
            kind: CredentialKind::NamedHeader,
            header_name: Some(CLAUDE_CREDENTIAL_HEADER.to_owned()),
        },
        discovery_mode: DiscoveryMode::MachineApi,
        refresh: RefreshPolicy::default(),
        default_inventory: None,
    }
}

pub(crate) fn claude_connection_contract() -> ProviderConnectionContract {
    ProviderConnectionContract {
        default_base_url: CLAUDE_DEFAULT_BASE_URL.to_owned(),
        region: ProviderFieldSchema::unsupported(),
        workspace: ProviderFieldSchema::unsupported(),
        account: ProviderFieldSchema::unsupported(),
    }
}

pub(crate) fn claude_known_provider() -> KnownProvider {
    KnownProvider {
        provider_profile_id: CLAUDE_PROVIDER_PROFILE_ID.to_owned(),
        display_name: CLAUDE_DISPLAY_NAME.to_owned(),
        base_url: CLAUDE_DEFAULT_BASE_URL.to_owned(),
        protocol_adapter_id: CLAUDE_MESSAGES_ADAPTER_ID.to_owned(),
        provider_rules_id: Some(CLAUDE_PROVIDER_PROFILE_ID.to_owned()),
        ui_hints: BTreeMap::from([
            (
                "credential".to_owned(),
                json!({
                    "kind": "named_header",
                    "header_name": CLAUDE_CREDENTIAL_HEADER,
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

pub(crate) fn claude_provider_rules(revision_seq: u64) -> ProviderRulesCatalog {
    ProviderRulesCatalog {
        format: "buckyos.aicc.provider-rules-catalog".to_owned(),
        schema_version: 1,
        schema_revision: 0,
        revision_seq,
        provider_profile_id: CLAUDE_PROVIDER_PROFILE_ID.to_owned(),
        metadata_drivers: Some(vec![CLAUDE_PROVIDER_PROFILE_ID.to_owned()]),
        origin_provider_aliases: BTreeMap::new(),
        origin_mappings: Vec::new(),
        models: Vec::new(),
        patterns: vec![ProviderPatternRule {
            match_rule: MatchRule::Shorthand("*".to_owned()),
            exclude: false,
            operations: BTreeMap::from([(
                "llm".to_owned(),
                CLAUDE_MESSAGES_OPERATION_ID.to_owned(),
            )]),
            provider_options: BTreeMap::new(),
            request_rules: Vec::new(),
            pricing: None,
            remove_api_types: BTreeSet::new(),
            remove_features: BTreeSet::new(),
            estimated_latency_ms: None,
            latency_class: None,
            cost_class: None,
        }],
        variants: Vec::new(),
    }
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

    #[test]
    fn builtin_identity_rules_and_registration_are_stable() {
        let profile = claude_profile();
        let known = claude_known_provider();
        let rules = claude_provider_rules(7);
        let (adapter, registration) = claude_messages_adapter();

        assert_eq!(profile.provider_profile_id, "claude");
        assert_eq!(profile.default_protocol_adapter_id, "claude-messages");
        assert_eq!(profile.credential.kind, CredentialKind::NamedHeader);
        assert_eq!(
            profile.credential.header_name.as_deref(),
            Some(CLAUDE_CREDENTIAL_HEADER)
        );
        assert_eq!(known.base_url, CLAUDE_DEFAULT_BASE_URL);
        assert!(claude_connection_contract()
            .resolve(Default::default())
            .is_ok());
        assert_eq!(known.provider_rules_id.as_deref(), Some("claude"));
        assert_eq!(rules.metadata_drivers, Some(vec!["claude".to_owned()]));
        assert_eq!(
            rules.patterns[0].operations["llm"],
            CLAUDE_MESSAGES_OPERATION_ID
        );
        assert_eq!(adapter.base_adapter_id, None);
        assert_eq!(registration.operation_codecs.len(), 1);
    }
}
