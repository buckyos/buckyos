use super::super::{
    CatalogOnlyDiscovery, CredentialDescriptor, DiscoveredModel, DiscoveryMode, ModelAvailability,
    ProviderConnectionContract, ProviderConnectionInput, ProviderDiscoverySnapshot, ProviderError,
    ProviderFieldSchema, ProviderHealthState, ProviderProfile, ProviderResult, RefreshPolicy,
    ResolvedProviderConnection,
};
use crate::catalog::{KnownProvider, ProviderPatternRule, ProviderRulesCatalog};
use crate::matching::MatchRule;
use crate::protocol::{CredentialKind, GLM_CHAT_ADAPTER_ID, OPENAI_CHAT_COMPLETIONS_OPERATION_ID};
use buckyos_api::ApiType;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const GLM_PROVIDER_PROFILE_ID: &str = "glm";
pub(crate) const GLM_DISPLAY_NAME: &str = "Z.ai GLM";
pub(crate) const GLM_DEFAULT_BASE_URL: &str = "https://api.z.ai/api/paas/v4";
pub(crate) const GLM_CHINA_BASE_URL: &str = "https://open.bigmodel.cn/api/paas/v4";

pub(crate) fn glm_profile() -> ProviderProfile {
    glm_profile_with_credential(CredentialKind::Bearer)
}

pub(crate) fn glm_jwt_profile() -> ProviderProfile {
    glm_profile_with_credential(CredentialKind::GlmJwt)
}

fn glm_profile_with_credential(kind: CredentialKind) -> ProviderProfile {
    ProviderProfile {
        provider_profile_id: GLM_PROVIDER_PROFILE_ID.to_owned(),
        display_name: GLM_DISPLAY_NAME.to_owned(),
        default_protocol_adapter_id: GLM_CHAT_ADAPTER_ID.to_owned(),
        credential: CredentialDescriptor {
            kind,
            header_name: None,
        },
        discovery_mode: DiscoveryMode::CatalogOnly,
        refresh: RefreshPolicy::default(),
        default_inventory: None,
    }
}

pub(crate) fn glm_connection_contract() -> ProviderConnectionContract {
    ProviderConnectionContract {
        default_base_url: GLM_DEFAULT_BASE_URL.to_owned(),
        region: ProviderFieldSchema::optional_with_default("global")
            .with_allowed_values(["global", "china"]),
        workspace: ProviderFieldSchema::unsupported(),
        account: ProviderFieldSchema::unsupported(),
    }
}

pub(crate) fn resolve_glm_connection(
    input: ProviderConnectionInput<'_>,
) -> ProviderResult<ResolvedProviderConnection> {
    let base_url = match (input.base_url, input.region.unwrap_or("global")) {
        (Some(base_url), _) => Some(base_url),
        (None, "global") => Some(GLM_DEFAULT_BASE_URL),
        (None, "china") => Some(GLM_CHINA_BASE_URL),
        (None, _) => None,
    };
    glm_connection_contract().resolve(ProviderConnectionInput { base_url, ..input })
}

pub(crate) fn glm_known_provider() -> KnownProvider {
    KnownProvider {
        provider_profile_id: GLM_PROVIDER_PROFILE_ID.to_owned(),
        display_name: GLM_DISPLAY_NAME.to_owned(),
        base_url: GLM_DEFAULT_BASE_URL.to_owned(),
        protocol_adapter_id: GLM_CHAT_ADAPTER_ID.to_owned(),
        provider_rules_id: Some(GLM_PROVIDER_PROFILE_ID.to_owned()),
        ui_hints: BTreeMap::from([
            (
                "credential".to_owned(),
                json!({
                    "kinds": ["bearer", "glm_jwt"],
                    "default": "bearer",
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
                json!({"global": GLM_DEFAULT_BASE_URL, "china": GLM_CHINA_BASE_URL}),
            ),
        ]),
    }
}

pub(crate) fn glm_provider_rules(revision_seq: u64) -> ProviderRulesCatalog {
    ProviderRulesCatalog {
        format: "buckyos.aicc.provider-rules-catalog".to_owned(),
        schema_version: 1,
        schema_revision: 0,
        revision_seq,
        provider_profile_id: GLM_PROVIDER_PROFILE_ID.to_owned(),
        metadata_drivers: Some(vec![GLM_PROVIDER_PROFILE_ID.to_owned()]),
        origin_provider_aliases: BTreeMap::new(),
        origin_mappings: Vec::new(),
        models: Vec::new(),
        patterns: vec![ProviderPatternRule {
            match_rule: MatchRule::Shorthand("*".to_owned()),
            exclude: false,
            operations: BTreeMap::from([(
                "llm".to_owned(),
                OPENAI_CHAT_COMPLETIONS_OPERATION_ID.to_owned(),
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

pub(crate) fn glm_catalog_only_inventory(
    model_ids: impl IntoIterator<Item = String>,
) -> ProviderResult<CatalogOnlyDiscovery> {
    let mut models = BTreeMap::new();
    for model_id in model_ids {
        if model_id.trim().is_empty() || model_id.contains('@') {
            return Err(ProviderError::InvalidConfiguration(
                "GLM catalog model ID is invalid".to_owned(),
            ));
        }
        if models.contains_key(&model_id) {
            return Err(ProviderError::InvalidConfiguration(
                "GLM catalog contains a duplicate model ID".to_owned(),
            ));
        }
        models.insert(
            model_id.clone(),
            DiscoveredModel {
                provider_model_id: model_id,
                origin_model_id: None,
                api_types: Some(vec![ApiType::Llm]),
                supported_features: None,
                remote_methods: Some(BTreeSet::from([
                    OPENAI_CHAT_COMPLETIONS_OPERATION_ID.to_owned()
                ])),
                availability: ModelAvailability::Available,
                deprecated: false,
                pricing: None,
            },
        );
    }
    if models.is_empty() {
        return Err(ProviderError::InvalidConfiguration(
            "GLM catalog must contain at least one model".to_owned(),
        ));
    }
    Ok(CatalogOnlyDiscovery::new(ProviderDiscoverySnapshot {
        revision: None,
        discovered_at_ms: super::super::now_ms()?,
        health: ProviderHealthState::Healthy,
        models: models.into_values().collect(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ResolvedCredential;
    use crate::provider::{
        CredentialReference, DiscoveryContext, ProviderDiscovery, ProviderInstanceConfig,
    };

    #[test]
    fn profiles_connection_rules_and_credentials_are_stable() {
        assert_eq!(glm_profile().credential.kind, CredentialKind::Bearer);
        assert_eq!(glm_jwt_profile().credential.kind, CredentialKind::GlmJwt);
        assert_eq!(
            resolve_glm_connection(ProviderConnectionInput::default())
                .unwrap()
                .base_url,
            GLM_DEFAULT_BASE_URL
        );
        assert_eq!(
            resolve_glm_connection(ProviderConnectionInput {
                region: Some("china"),
                ..ProviderConnectionInput::default()
            })
            .unwrap()
            .base_url,
            GLM_CHINA_BASE_URL
        );
        assert!(resolve_glm_connection(ProviderConnectionInput {
            region: Some("unknown"),
            ..ProviderConnectionInput::default()
        })
        .is_err());
        assert_eq!(
            glm_known_provider().protocol_adapter_id,
            GLM_CHAT_ADAPTER_ID
        );
        assert_eq!(
            glm_provider_rules(5).patterns[0].operations["llm"],
            OPENAI_CHAT_COMPLETIONS_OPERATION_ID
        );
    }

    #[tokio::test]
    async fn catalog_only_inventory_is_explicit_and_validated() {
        let discovery = glm_catalog_only_inventory(vec!["glm-model".to_owned()]).unwrap();
        let profile = glm_profile();
        let instance = ProviderInstanceConfig {
            provider_instance_name: "glm-main".to_owned(),
            provider_profile_id: GLM_PROVIDER_PROFILE_ID.to_owned(),
            protocol_adapter_id: GLM_CHAT_ADAPTER_ID.to_owned(),
            base_url: GLM_DEFAULT_BASE_URL.to_owned(),
            credential: CredentialReference {
                reference: "secret://glm".to_owned(),
            },
            provider_rules_id: Some(GLM_PROVIDER_PROFILE_ID.to_owned()),
            region: Some("global".to_owned()),
            account: None,
        };
        let credential = ResolvedCredential::bearer("secret://glm", "secret").unwrap();
        let snapshot = discovery
            .discover(&DiscoveryContext {
                profile: &profile,
                instance: &instance,
                credential: &credential,
            })
            .await
            .unwrap();
        assert_eq!(snapshot.models[0].provider_model_id, "glm-model");
        assert!(glm_catalog_only_inventory(Vec::<String>::new()).is_err());
        assert!(glm_catalog_only_inventory(vec!["same".to_owned(), "same".to_owned()]).is_err());
    }
}
