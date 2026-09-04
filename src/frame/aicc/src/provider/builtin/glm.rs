use super::super::{
    CatalogOnlyDiscovery, DiscoveredModel, ModelAvailability, ProviderDiscoverySnapshot,
    ProviderError, ProviderFieldSchema, ProviderHealthState, ProviderResult,
};
#[cfg(test)]
use super::super::{
    CredentialDescriptor, DiscoveryMode, ProviderConnectionContract, ProviderConnectionInput,
    ProviderProfile, RefreshPolicy, ResolvedProviderConnection,
};
#[cfg(test)]
use crate::catalog::KnownProvider;
#[cfg(test)]
use crate::catalog::{
    CatalogKind, CurrentCatalogFile, KnownProviderCatalog, ModelDriverCatalog,
    ProviderCredentialKind, ProviderRulesCatalog,
};
#[cfg(test)]
use crate::protocol::CredentialKind;
use crate::protocol::OPENAI_CHAT_COMPLETIONS_OPERATION_ID;
use buckyos_api::ApiType;
#[cfg(test)]
use serde::de::DeserializeOwned;
use serde::Deserialize;
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
    let known = glm_known_provider();
    let declared_kind = match kind {
        CredentialKind::Bearer => ProviderCredentialKind::Bearer,
        CredentialKind::GlmJwt => ProviderCredentialKind::GlmJwt,
        _ => panic!("GLM profile requested an unsupported credential kind"),
    };
    assert!(std::iter::once(&known.credential)
        .chain(&known.credential_variants)
        .any(|item| item.kind == declared_kind));
    ProviderProfile {
        provider_profile_id: GLM_PROVIDER_PROFILE_ID.to_owned(),
        display_name: known.display_name,
        default_protocol_adapter_id: known.protocol_adapter_id,
        credential: CredentialDescriptor {
            kind,
            header_name: None,
        },
        credential_variants: Vec::new(),
        discovery_mode: DiscoveryMode::CatalogOnly,
        refresh: RefreshPolicy::default(),
        default_inventory: None,
    }
}

#[cfg(test)]
pub(crate) fn glm_connection_contract() -> ProviderConnectionContract {
    let known = glm_known_provider();
    let fields: InstanceFieldDeclarations = embedded_value(
        &known,
        "instance_fields",
        "GLM Known Provider instance fields",
    );
    ProviderConnectionContract {
        default_base_url: known.base_url,
        region: fields.region,
        workspace: fields.workspace,
        account: fields.account,
        region_base_urls: known.connection.region_base_urls,
    }
}

#[cfg(test)]
pub(crate) fn resolve_glm_connection(
    input: ProviderConnectionInput<'_>,
) -> ProviderResult<ResolvedProviderConnection> {
    glm_connection_contract().resolve(input)
}

#[cfg(test)]
pub(crate) fn glm_known_provider() -> KnownProvider {
    super::builtin_catalog_document::<KnownProviderCatalog>(
        CatalogKind::KnownProvider,
        GLM_PROVIDER_PROFILE_ID,
    )
    .providers
    .into_iter()
    .find(|provider| provider.provider_profile_id == GLM_PROVIDER_PROFILE_ID)
    .expect("GLM Known Provider catalog must contain the GLM profile")
}

#[cfg(test)]
pub(crate) fn glm_provider_rules(_revision_seq: u64) -> ProviderRulesCatalog {
    super::builtin_catalog_document(CatalogKind::ProviderRules, GLM_PROVIDER_PROFILE_ID)
}

#[cfg(test)]
pub(crate) fn glm_model_driver() -> ModelDriverCatalog {
    super::builtin_catalog_document(CatalogKind::ModelDriver, GLM_PROVIDER_PROFILE_ID)
}

#[cfg(test)]
pub(crate) fn glm_catalog_files() -> Vec<CurrentCatalogFile> {
    super::builtin_catalog_files(&[GLM_PROVIDER_PROFILE_ID])
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InstanceFieldDeclarations {
    region: ProviderFieldSchema,
    workspace: ProviderFieldSchema,
    account: ProviderFieldSchema,
}

#[cfg(test)]
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

#[cfg(test)]
fn embedded_json<T: DeserializeOwned>(contents: &[u8], label: &str) -> T {
    serde_json::from_slice(contents).unwrap_or_else(|error| panic!("{label} is invalid: {error}"))
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
