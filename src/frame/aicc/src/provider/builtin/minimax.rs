use super::super::{
    CredentialDescriptor, DiscoveryMode, ProviderConnectionContract, ProviderConnectionInput,
    ProviderFieldSchema, ProviderProfile, ProviderResult, RefreshPolicy,
};
use super::anthropic_models::{AnthropicModelsDiscovery, AnthropicModelsSpec};
use crate::catalog::{KnownProvider, ProviderPatternRule, ProviderRulesCatalog};
use crate::matching::MatchRule;
use crate::protocol::{
    CredentialKind, HttpTransport, CLAUDE_MESSAGES_OPERATION_ID, MINIMAX_MESSAGES_ADAPTER_ID,
};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const MINIMAX_PROVIDER_PROFILE_ID: &str = "minimax";
pub(crate) const MINIMAX_DISPLAY_NAME: &str = "MiniMax";
pub(crate) const MINIMAX_DEFAULT_BASE_URL: &str = "https://api.minimax.io/anthropic";
pub(crate) const MINIMAX_CHINA_BASE_URL: &str = "https://api.minimaxi.com/anthropic";
pub(crate) const MINIMAX_CREDENTIAL_HEADER: &str = "x-api-key";

pub(super) const MINIMAX_SPEC: AnthropicModelsSpec = AnthropicModelsSpec {
    provider_profile_id: MINIMAX_PROVIDER_PROFILE_ID,
    protocol_adapter_id: MINIMAX_MESSAGES_ADAPTER_ID,
    version_header: false,
    connection_contract: minimax_connection_contract,
    label: "MiniMax",
};

pub(crate) fn minimax_profile() -> ProviderProfile {
    ProviderProfile {
        provider_profile_id: MINIMAX_PROVIDER_PROFILE_ID.to_owned(),
        display_name: MINIMAX_DISPLAY_NAME.to_owned(),
        default_protocol_adapter_id: MINIMAX_MESSAGES_ADAPTER_ID.to_owned(),
        credential: CredentialDescriptor {
            kind: CredentialKind::NamedHeader,
            header_name: Some(MINIMAX_CREDENTIAL_HEADER.to_owned()),
        },
        discovery_mode: DiscoveryMode::MachineApi,
        refresh: RefreshPolicy::default(),
        default_inventory: None,
    }
}

pub(crate) fn minimax_known_provider() -> KnownProvider {
    KnownProvider {
        provider_profile_id: MINIMAX_PROVIDER_PROFILE_ID.to_owned(),
        display_name: MINIMAX_DISPLAY_NAME.to_owned(),
        base_url: MINIMAX_DEFAULT_BASE_URL.to_owned(),
        protocol_adapter_id: MINIMAX_MESSAGES_ADAPTER_ID.to_owned(),
        provider_rules_id: Some(MINIMAX_PROVIDER_PROFILE_ID.to_owned()),
        ui_hints: BTreeMap::from([
            (
                "credential".to_owned(),
                json!({
                    "kind": "named_header",
                    "header_name": MINIMAX_CREDENTIAL_HEADER,
                    "required": true,
                    "secret": true
                }),
            ),
            (
                "instance_fields".to_owned(),
                json!({
                    "region": {"default": "global", "values": ["global", "china"]},
                    "workspace": "unsupported",
                    "account": "unsupported"
                }),
            ),
            (
                "region_base_urls".to_owned(),
                json!({
                    "global": MINIMAX_DEFAULT_BASE_URL,
                    "china": MINIMAX_CHINA_BASE_URL
                }),
            ),
        ]),
    }
}

pub(crate) fn minimax_connection_contract() -> ProviderConnectionContract {
    ProviderConnectionContract {
        default_base_url: MINIMAX_DEFAULT_BASE_URL.to_owned(),
        region: ProviderFieldSchema::optional_with_default("global")
            .with_allowed_values(["global", "china"]),
        workspace: ProviderFieldSchema::unsupported(),
        account: ProviderFieldSchema::unsupported(),
    }
}

pub(crate) fn resolve_minimax_connection(
    input: ProviderConnectionInput<'_>,
) -> ProviderResult<super::super::ResolvedProviderConnection> {
    let default_base_url = match input.region.unwrap_or("global") {
        "global" => MINIMAX_DEFAULT_BASE_URL,
        "china" => MINIMAX_CHINA_BASE_URL,
        _ => MINIMAX_DEFAULT_BASE_URL,
    };
    minimax_connection_contract().resolve(ProviderConnectionInput {
        base_url: input.base_url.or(Some(default_base_url)),
        ..input
    })
}

pub(crate) fn minimax_provider_rules(revision_seq: u64) -> ProviderRulesCatalog {
    ProviderRulesCatalog {
        format: "buckyos.aicc.provider-rules-catalog".to_owned(),
        schema_version: 1,
        schema_revision: 0,
        revision_seq,
        provider_profile_id: MINIMAX_PROVIDER_PROFILE_ID.to_owned(),
        metadata_drivers: Some(vec![MINIMAX_PROVIDER_PROFILE_ID.to_owned()]),
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

pub(crate) fn minimax_discovery(transport: HttpTransport) -> AnthropicModelsDiscovery {
    AnthropicModelsDiscovery::new(MINIMAX_SPEC, transport)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{
        minimax_messages_adapter, minimax_messages_dialect_contract, CodecRegistry,
        CLAUDE_MESSAGES_ADAPTER_ID,
    };

    #[test]
    fn builtin_identity_regions_rules_and_dialect_are_stable() {
        let profile = minimax_profile();
        let known = minimax_known_provider();
        let rules = minimax_provider_rules(9);
        let contract = minimax_messages_dialect_contract();
        let (adapter, registration) = minimax_messages_adapter();

        assert_eq!(profile.provider_profile_id, "minimax");
        assert_eq!(profile.default_protocol_adapter_id, "minimax-messages");
        assert_eq!(profile.credential.kind, CredentialKind::NamedHeader);
        assert_eq!(
            resolve_minimax_connection(Default::default())
                .unwrap()
                .base_url,
            MINIMAX_DEFAULT_BASE_URL
        );
        assert_eq!(
            resolve_minimax_connection(ProviderConnectionInput {
                region: Some("china"),
                ..Default::default()
            })
            .unwrap()
            .base_url,
            MINIMAX_CHINA_BASE_URL
        );
        assert!(resolve_minimax_connection(ProviderConnectionInput {
            region: Some("unknown"),
            ..Default::default()
        })
        .is_err());
        assert_eq!(known.provider_rules_id.as_deref(), Some("minimax"));
        assert_eq!(rules.metadata_drivers, Some(vec!["minimax".to_owned()]));
        assert_eq!(
            rules.patterns[0].operations["llm"],
            CLAUDE_MESSAGES_OPERATION_ID
        );
        assert_eq!(contract.base_adapter_id, CLAUDE_MESSAGES_ADAPTER_ID);
        assert_eq!(
            adapter.base_adapter_id.as_deref(),
            Some(CLAUDE_MESSAGES_ADAPTER_ID)
        );
        assert_eq!(registration.operation_codecs.len(), 1);
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
