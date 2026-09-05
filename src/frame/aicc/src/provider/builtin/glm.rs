use super::super::{
    CatalogOnlyDiscovery, DiscoveredModel, ModelAvailability, ProviderDiscoverySnapshot,
    ProviderError, ProviderHealthState, ProviderResult,
};
#[cfg(test)]
use super::super::{
    DiscoveryMode, ProviderConnectionContract, ProviderConnectionInput, ProviderProfile,
    ResolvedProviderConnection,
};
#[cfg(test)]
use crate::catalog::{CurrentCatalogFile, ModelDriverCatalog, ProviderRulesCatalog};
#[cfg(test)]
use crate::protocol::CredentialKind;
use crate::protocol::OPENAI_CHAT_COMPLETIONS_OPERATION_ID;
use buckyos_api::ApiType;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const GLM_PROVIDER_PROFILE_ID: &str = "glm";

#[cfg(test)]
pub(crate) fn glm_profile() -> ProviderProfile {
    glm_profile_with_credential(CredentialKind::Bearer)
}

#[cfg(test)]
pub(crate) fn glm_jwt_profile() -> ProviderProfile {
    glm_profile_with_credential(CredentialKind::GlmJwt)
}

#[cfg(test)]
fn glm_profile_with_credential(kind: CredentialKind) -> ProviderProfile {
    super::builtin_profile_with_credential(
        GLM_PROVIDER_PROFILE_ID,
        DiscoveryMode::CatalogOnly,
        kind,
    )
}

#[cfg(test)]
pub(crate) fn glm_connection_contract() -> ProviderConnectionContract {
    super::builtin_connection_contract(GLM_PROVIDER_PROFILE_ID)
}

#[cfg(test)]
pub(crate) fn resolve_glm_connection(
    input: ProviderConnectionInput<'_>,
) -> ProviderResult<ResolvedProviderConnection> {
    glm_connection_contract().resolve(input)
}

#[cfg(test)]
pub(crate) fn glm_known_provider() -> crate::catalog::KnownProvider {
    super::builtin_known_provider(GLM_PROVIDER_PROFILE_ID)
}

#[cfg(test)]
pub(crate) fn glm_provider_rules(_revision_seq: u64) -> ProviderRulesCatalog {
    super::builtin_provider_rules(GLM_PROVIDER_PROFILE_ID)
}

#[cfg(test)]
pub(crate) fn glm_model_driver() -> ModelDriverCatalog {
    super::builtin_model_driver(GLM_PROVIDER_PROFILE_ID)
}

#[cfg(test)]
pub(crate) fn glm_catalog_files() -> Vec<CurrentCatalogFile> {
    super::builtin_catalog_files(&[GLM_PROVIDER_PROFILE_ID])
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
    use crate::protocol::{ResolvedCredential, GLM_CHAT_ADAPTER_ID};
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
            "https://api.z.ai/api/paas/v4"
        );
        assert_eq!(
            resolve_glm_connection(ProviderConnectionInput {
                region: Some("china"),
                ..ProviderConnectionInput::default()
            })
            .unwrap()
            .base_url,
            "https://open.bigmodel.cn/api/paas/v4"
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
            base_url: glm_known_provider().base_url,
            credential: CredentialReference {
                reference: "secret://glm".to_owned(),
            },
            credential_kind: None,
            provider_rules_id: Some(GLM_PROVIDER_PROFILE_ID.to_owned()),
            region: Some("global".to_owned()),
            workspace: None,
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
