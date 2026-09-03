use super::super::{
    validate_discovery, CatalogOnlyDiscovery, CredentialDescriptor, DiscoveredModel,
    DiscoveryContext, DiscoveryMode, ModelAvailability, ProviderConnectionContract,
    ProviderConnectionInput, ProviderDiscovery, ProviderDiscoverySnapshot, ProviderError,
    ProviderFieldSchema, ProviderHealthState, ProviderProfile, ProviderResult, RefreshPolicy,
};
use crate::catalog::{KnownProvider, ProviderPatternRule, ProviderRulesCatalog};
use crate::matching::MatchRule;
use crate::protocol::{
    CredentialKind, HttpRequest, HttpResponse, HttpTransport, ResponsesDialectKind,
    DEEPSEEK_RESPONSES_ADAPTER_ID, DOUBAO_RESPONSES_ADAPTER_ID, OPENAI_RESPONSES_OPERATION_ID,
    QWEN_RESPONSES_ADAPTER_ID,
};
use async_trait::async_trait;
use buckyos_api::ApiType;
use reqwest::header::ETAG;
use reqwest::Method;
use serde::Deserialize;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

pub(crate) const DEEPSEEK_PROFILE_ID: &str = "deepseek";
pub(crate) const DOUBAO_PROFILE_ID: &str = "doubao";
pub(crate) const QWEN_PROFILE_ID: &str = "qwen";

const DEEPSEEK_DEFAULT_BASE_URL: &str = "https://api.deepseek.com";
const DOUBAO_DEFAULT_BASE_URL: &str = "https://ark.cn-beijing.volces.com/api/v3";
const QWEN_DEFAULT_BASE_URL: &str =
    "https://{workspace}.{region}.maas.aliyuncs.com/compatible-mode/v1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum BuiltinDiscoveryKind {
    OpenAiModelsApi,
    CatalogOnly,
}

#[derive(Clone, Debug)]
pub(crate) struct BuiltinProviderDescriptor {
    pub profile: ProviderProfile,
    pub default_base_url: &'static str,
    pub connection: ProviderConnectionContract,
    pub discovery: BuiltinDiscoveryKind,
    pub operation_bindings: BTreeMap<&'static str, &'static str>,
    pub dialect: ResponsesDialectKind,
}

impl BuiltinProviderDescriptor {
    pub(crate) fn known_provider(&self) -> KnownProvider {
        KnownProvider {
            provider_profile_id: self.profile.provider_profile_id.clone(),
            display_name: self.profile.display_name.clone(),
            base_url: self.default_base_url.to_string(),
            protocol_adapter_id: self.profile.default_protocol_adapter_id.clone(),
            provider_rules_id: Some(self.profile.provider_profile_id.clone()),
            ui_hints: BTreeMap::from([
                ("credential_label".to_string(), json!("API key")),
                (
                    "credential_type".to_string(),
                    json!(self.profile.credential.kind.as_str()),
                ),
                (
                    "instance_fields".to_string(),
                    serde_json::to_value(&self.connection)
                        .expect("provider connection schema is serializable"),
                ),
            ]),
        }
    }

    pub(crate) fn provider_rules(&self, revision_seq: u64) -> ProviderRulesCatalog {
        let metadata_drivers = match self.profile.provider_profile_id.as_str() {
            DEEPSEEK_PROFILE_ID => Some(vec!["deepseek".to_owned()]),
            QWEN_PROFILE_ID => Some(vec!["qwen".to_owned()]),
            DOUBAO_PROFILE_ID => Some(vec!["doubao".to_owned()]),
            _ => None,
        };
        ProviderRulesCatalog {
            format: "buckyos.aicc.provider-rules-catalog".to_owned(),
            schema_version: 1,
            schema_revision: 0,
            revision_seq,
            provider_profile_id: self.profile.provider_profile_id.clone(),
            metadata_drivers,
            origin_provider_aliases: BTreeMap::new(),
            origin_mappings: Vec::new(),
            models: Vec::new(),
            patterns: vec![ProviderPatternRule {
                match_rule: MatchRule::Shorthand("*".to_owned()),
                exclude: false,
                operations: BTreeMap::from([(
                    "llm".to_owned(),
                    OPENAI_RESPONSES_OPERATION_ID.to_owned(),
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

    pub(crate) fn resolve_base_url(
        &self,
        region: Option<&str>,
        workspace: Option<&str>,
    ) -> ProviderResult<String> {
        self.connection
            .resolve(ProviderConnectionInput {
                region,
                workspace,
                ..ProviderConnectionInput::default()
            })
            .map(|connection| connection.base_url)
    }

    pub(crate) fn catalog_only_inventory(
        &self,
        model_ids: impl IntoIterator<Item = String>,
    ) -> ProviderResult<ProviderDiscoverySnapshot> {
        if self.discovery != BuiltinDiscoveryKind::CatalogOnly {
            return Err(ProviderError::InvalidConfiguration(
                "provider uses machine API discovery".into(),
            ));
        }
        let models = model_ids
            .into_iter()
            .map(|provider_model_id| catalog_model(provider_model_id))
            .collect::<Vec<_>>();
        validate_fixture_model_ids(&models)?;
        Ok(ProviderDiscoverySnapshot {
            revision: None,
            discovered_at_ms: super::super::now_ms()?,
            health: ProviderHealthState::Healthy,
            models,
        })
    }

    pub(crate) fn catalog_only_discovery(
        &self,
        model_ids: impl IntoIterator<Item = String>,
    ) -> ProviderResult<Arc<dyn ProviderDiscovery>> {
        Ok(Arc::new(CatalogOnlyDiscovery::new(
            self.catalog_only_inventory(model_ids)?,
        )))
    }
}

pub(crate) fn wp08e_builtin_providers() -> Vec<BuiltinProviderDescriptor> {
    vec![deepseek(), doubao(), qwen()]
}

pub(crate) fn deepseek_models_discovery(transport: HttpTransport) -> DeepSeekModelsDiscovery {
    DeepSeekModelsDiscovery::new(transport)
}

fn deepseek() -> BuiltinProviderDescriptor {
    descriptor(
        DEEPSEEK_PROFILE_ID,
        "DeepSeek",
        DEEPSEEK_RESPONSES_ADAPTER_ID,
        DEEPSEEK_DEFAULT_BASE_URL,
        ProviderConnectionContract {
            default_base_url: DEEPSEEK_DEFAULT_BASE_URL.to_owned(),
            region: ProviderFieldSchema::unsupported(),
            workspace: ProviderFieldSchema::unsupported(),
            account: ProviderFieldSchema::unsupported(),
        },
        BuiltinDiscoveryKind::OpenAiModelsApi,
        ResponsesDialectKind::DeepSeek,
    )
}

fn doubao() -> BuiltinProviderDescriptor {
    descriptor(
        DOUBAO_PROFILE_ID,
        "豆包（火山方舟）",
        DOUBAO_RESPONSES_ADAPTER_ID,
        DOUBAO_DEFAULT_BASE_URL,
        ProviderConnectionContract {
            default_base_url: "https://ark.{region}.volces.com/api/v3".to_owned(),
            region: ProviderFieldSchema::optional_with_default("cn-beijing")
                .with_allowed_values(["cn-beijing"]),
            workspace: ProviderFieldSchema::unsupported(),
            account: ProviderFieldSchema::unsupported(),
        },
        BuiltinDiscoveryKind::CatalogOnly,
        ResponsesDialectKind::Doubao,
    )
}

fn qwen() -> BuiltinProviderDescriptor {
    descriptor(
        QWEN_PROFILE_ID,
        "Qwen（阿里云百炼）",
        QWEN_RESPONSES_ADAPTER_ID,
        QWEN_DEFAULT_BASE_URL,
        ProviderConnectionContract {
            default_base_url: QWEN_DEFAULT_BASE_URL.to_owned(),
            region: ProviderFieldSchema::optional_with_default("cn-beijing").with_allowed_values([
                "cn-beijing",
                "ap-southeast-1",
                "us-east-1",
                "eu-central-1",
                "ap-northeast-1",
            ]),
            workspace: ProviderFieldSchema::required(),
            account: ProviderFieldSchema::unsupported(),
        },
        BuiltinDiscoveryKind::CatalogOnly,
        ResponsesDialectKind::Qwen,
    )
}

fn descriptor(
    profile_id: &str,
    display_name: &str,
    adapter_id: &str,
    default_base_url: &'static str,
    connection: ProviderConnectionContract,
    discovery: BuiltinDiscoveryKind,
    dialect: ResponsesDialectKind,
) -> BuiltinProviderDescriptor {
    BuiltinProviderDescriptor {
        profile: ProviderProfile {
            provider_profile_id: profile_id.to_string(),
            display_name: display_name.to_string(),
            default_protocol_adapter_id: adapter_id.to_string(),
            credential: CredentialDescriptor {
                kind: CredentialKind::Bearer,
                header_name: None,
            },
            discovery_mode: match discovery {
                BuiltinDiscoveryKind::OpenAiModelsApi => DiscoveryMode::MachineApi,
                BuiltinDiscoveryKind::CatalogOnly => DiscoveryMode::CatalogOnly,
            },
            refresh: RefreshPolicy::default(),
            default_inventory: None,
        },
        default_base_url,
        connection,
        discovery,
        operation_bindings: BTreeMap::from([("llm", OPENAI_RESPONSES_OPERATION_ID)]),
        dialect,
    }
}

fn catalog_model(provider_model_id: String) -> DiscoveredModel {
    DiscoveredModel {
        provider_model_id,
        origin_model_id: None,
        api_types: Some(vec![ApiType::Llm]),
        supported_features: None,
        remote_methods: Some(BTreeSet::from([OPENAI_RESPONSES_OPERATION_ID.to_string()])),
        availability: ModelAvailability::Available,
        deprecated: false,
        pricing: None,
    }
}

fn validate_fixture_model_ids(models: &[DiscoveredModel]) -> ProviderResult<()> {
    let mut ids = BTreeSet::new();
    for model in models {
        if model.provider_model_id.trim().is_empty()
            || model.provider_model_id.contains('@')
            || !ids.insert(model.provider_model_id.as_str())
        {
            return Err(ProviderError::InvalidConfiguration(
                "catalog-only model IDs must be unique, non-empty, and omit `@`".into(),
            ));
        }
    }
    Ok(())
}

#[async_trait]
trait DeepSeekModelsTransport: Send + Sync {
    async fn send(
        &self,
        request: HttpRequest,
    ) -> crate::protocol::ProtocolResultValue<HttpResponse>;
}

#[async_trait]
impl DeepSeekModelsTransport for HttpTransport {
    async fn send(
        &self,
        request: HttpRequest,
    ) -> crate::protocol::ProtocolResultValue<HttpResponse> {
        HttpTransport::send(self, request).await
    }
}

#[derive(Clone)]
pub(crate) struct DeepSeekModelsDiscovery {
    transport: Arc<dyn DeepSeekModelsTransport>,
}

impl DeepSeekModelsDiscovery {
    pub(crate) fn new(transport: HttpTransport) -> Self {
        Self {
            transport: Arc::new(transport),
        }
    }

    #[cfg(test)]
    fn with_transport(transport: Arc<dyn DeepSeekModelsTransport>) -> Self {
        Self { transport }
    }
}

#[derive(Deserialize)]
struct ModelsEnvelope {
    data: Vec<ModelObject>,
    #[serde(default)]
    object: Option<String>,
}

#[derive(Deserialize)]
struct ModelObject {
    id: String,
    #[serde(default)]
    object: Option<String>,
    #[serde(default)]
    owned_by: Option<String>,
    #[serde(default)]
    created: Option<u64>,
}

#[async_trait]
impl ProviderDiscovery for DeepSeekModelsDiscovery {
    async fn discover(
        &self,
        context: &DiscoveryContext<'_>,
    ) -> ProviderResult<ProviderDiscoverySnapshot> {
        validate_deepseek_context(context)?;
        let request = deepseek_models_request(context)?;
        let response = self
            .transport
            .send(request)
            .await
            .map_err(|error| ProviderError::Discovery(error.to_string()))?;
        ensure_deepseek_success(&response)?;
        let revision = response
            .headers
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let envelope: ModelsEnvelope = response
            .json(1024 * 1024)
            .map_err(|error| ProviderError::Discovery(error.to_string()))?;
        parse_deepseek_models(envelope, revision)
    }
}

fn validate_deepseek_context(context: &DiscoveryContext<'_>) -> ProviderResult<()> {
    if context.profile.provider_profile_id != DEEPSEEK_PROFILE_ID
        || context.profile.default_protocol_adapter_id != DEEPSEEK_RESPONSES_ADAPTER_ID
        || context.instance.provider_profile_id != DEEPSEEK_PROFILE_ID
        || context.instance.protocol_adapter_id != DEEPSEEK_RESPONSES_ADAPTER_ID
    {
        return Err(ProviderError::InvalidConfiguration(
            "DeepSeek discovery requires the DeepSeek profile and Responses dialect".to_owned(),
        ));
    }
    if context.credential.audit().kind != CredentialKind::Bearer {
        return Err(ProviderError::Credential(
            "DeepSeek discovery requires a Bearer credential".to_owned(),
        ));
    }
    if context.instance.region.is_some() || context.instance.account.is_some() {
        return Err(ProviderError::InvalidConfiguration(
            "DeepSeek profile does not accept region or account fields".to_owned(),
        ));
    }
    Ok(())
}

fn deepseek_models_request(context: &DiscoveryContext<'_>) -> ProviderResult<HttpRequest> {
    let mut base = reqwest::Url::parse(&context.instance.base_url)
        .map_err(|_| ProviderError::InvalidConfiguration("DeepSeek base_url is invalid".into()))?;
    if !base.path().ends_with('/') {
        let path = format!("{}/", base.path());
        base.set_path(&path);
    }
    let url = base.join("models").map_err(|_| {
        ProviderError::InvalidConfiguration("DeepSeek models URL is invalid".into())
    })?;
    let mut request = HttpRequest::new(Method::GET, url.to_string());
    context
        .credential
        .apply(&mut request.headers)
        .map_err(|error| ProviderError::Credential(error.to_string()))?;
    request.timeout = Some(Duration::from_secs(30));
    request.max_response_bytes = Some(1024 * 1024);
    Ok(request)
}

fn ensure_deepseek_success(response: &HttpResponse) -> ProviderResult<()> {
    if response.status.is_success() {
        return Ok(());
    }
    Err(ProviderError::Discovery(format!(
        "DeepSeek Models API returned HTTP {} (request {})",
        response.status.as_u16(),
        response.request_id
    )))
}

fn parse_deepseek_models(
    envelope: ModelsEnvelope,
    revision: Option<String>,
) -> ProviderResult<ProviderDiscoverySnapshot> {
    if envelope
        .object
        .as_deref()
        .is_some_and(|value| value != "list")
    {
        return Err(ProviderError::Discovery(
            "DeepSeek Models API returned an invalid object type".into(),
        ));
    }
    let mut models = envelope
        .data
        .into_iter()
        .map(|model| {
            let _metadata = (model.object, model.owned_by, model.created);
            catalog_model(model.id)
        })
        .collect::<Vec<_>>();
    validate_fixture_model_ids(&models)
        .map_err(|error| ProviderError::Discovery(error.to_string()))?;
    models.sort_by(|left, right| left.provider_model_id.cmp(&right.provider_model_id));
    let snapshot = ProviderDiscoverySnapshot {
        revision,
        discovered_at_ms: super::super::now_ms()?,
        health: ProviderHealthState::Healthy,
        models,
    };
    validate_discovery(&snapshot)?;
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{
        CatalogBuildOptions, CatalogDocuments, CatalogSnapshot, KnownProviderCatalog,
        ModelDriverCatalog,
    };
    use crate::protocol::{
        openai_responses_adapter, wp08e_responses_adapters, CodecRegistry, ResolvedCredential,
    };
    use crate::provider::{CredentialReference, InventoryBuilder, ProviderInstanceConfig};
    use reqwest::header::AUTHORIZATION;
    use serde_json::Value;

    #[test]
    fn profiles_have_stable_ids_urls_schemas_and_responses_bindings() {
        let providers = wp08e_builtin_providers();
        assert_eq!(
            providers
                .iter()
                .map(|provider| provider.profile.provider_profile_id.as_str())
                .collect::<Vec<_>>(),
            vec![DEEPSEEK_PROFILE_ID, DOUBAO_PROFILE_ID, QWEN_PROFILE_ID]
        );
        for provider in &providers {
            assert_eq!(provider.profile.credential.kind, CredentialKind::Bearer);
            assert_eq!(
                provider.operation_bindings.get("llm"),
                Some(&OPENAI_RESPONSES_OPERATION_ID)
            );
            assert_eq!(
                provider.profile.default_protocol_adapter_id,
                provider.dialect.contract().protocol_adapter_id
            );
            assert_eq!(
                provider.dialect.contract().base_adapter_id,
                "openai-responses"
            );
        }
        assert_eq!(
            providers[2].connection.workspace.mode,
            crate::provider::ProviderFieldMode::Required
        );
        assert_eq!(providers[1].default_base_url, DOUBAO_DEFAULT_BASE_URL);
    }

    #[test]
    fn qwen_resolves_every_declared_region_and_requires_safe_workspace() {
        let qwen = qwen();
        assert_eq!(
            qwen.resolve_base_url(Some("ap-southeast-1"), Some("ws-123"))
                .unwrap(),
            "https://ws-123.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1"
        );
        assert!(qwen.resolve_base_url(None, None).is_err());
        assert!(qwen.resolve_base_url(None, Some("bad/workspace")).is_err());
        assert!(qwen
            .resolve_base_url(Some("cn-unknown"), Some("ws-123"))
            .is_err());
    }

    #[test]
    fn catalog_only_inventory_uses_explicit_models_without_guessing_capabilities() {
        let snapshot = doubao()
            .catalog_only_inventory(["endpoint-a".to_string(), "endpoint-b".to_string()])
            .unwrap();
        assert_eq!(snapshot.health, ProviderHealthState::Healthy);
        assert_eq!(snapshot.models.len(), 2);
        assert!(snapshot.models[0].supported_features.is_none());
        assert_eq!(snapshot.models[0].api_types, Some(vec![ApiType::Llm]));
        assert!(doubao()
            .catalog_only_inventory(["endpoint-a".to_string(), "endpoint-a".to_string()])
            .is_err());
        assert!(deepseek()
            .catalog_only_inventory(["deepseek-model".to_string()])
            .is_err());
    }

    #[test]
    fn provider_rules_fixtures_bind_llm_to_responses_without_model_versions() {
        for provider in wp08e_builtin_providers() {
            let rules = provider.provider_rules(7);
            assert_eq!(rules.revision_seq, 7);
            assert!(rules.models.is_empty());
            assert_eq!(rules.patterns.len(), 1);
            assert_eq!(
                rules.patterns[0].operations.get("llm"),
                Some(&OPENAI_RESPONSES_OPERATION_ID.to_string())
            );
            assert!(serde_json::to_string(&rules)
                .unwrap()
                .find("deepseek-v")
                .is_none());
        }
    }

    #[test]
    fn known_provider_fixture_keeps_templates_and_ui_schema() {
        let providers = wp08e_builtin_providers();
        let known = providers
            .iter()
            .map(BuiltinProviderDescriptor::known_provider)
            .collect::<Vec<_>>();
        assert_eq!(known[0].base_url, DEEPSEEK_DEFAULT_BASE_URL);
        assert_eq!(known[2].base_url, QWEN_DEFAULT_BASE_URL);
        assert_eq!(
            known[2].ui_hints["instance_fields"]["workspace"]["mode"],
            Value::String("required".to_owned())
        );
    }

    #[test]
    fn deepseek_models_parser_is_catalog_neutral_and_rejects_bad_envelopes() {
        let envelope: ModelsEnvelope = serde_json::from_value(json!({
            "object": "list",
            "data": [
                {"id": "model-b", "object": "model", "owned_by": "deepseek"},
                {"id": "model-a", "object": "model", "owned_by": "deepseek"}
            ]
        }))
        .unwrap();
        let snapshot = parse_deepseek_models(envelope, Some("models-v1".to_owned())).unwrap();
        assert_eq!(snapshot.health, ProviderHealthState::Healthy);
        assert_eq!(snapshot.revision.as_deref(), Some("models-v1"));
        assert_eq!(snapshot.models[0].provider_model_id, "model-a");
        assert!(snapshot.models[0].supported_features.is_none());

        let invalid: ModelsEnvelope = serde_json::from_value(json!({
            "object": "model",
            "data": []
        }))
        .unwrap();
        assert!(parse_deepseek_models(invalid, None).is_err());
    }

    #[test]
    fn deepseek_discovery_request_uses_official_endpoint_and_bearer_auth() {
        let profile = deepseek().profile;
        let instance = ProviderInstanceConfig {
            provider_instance_name: "deepseek-main".to_owned(),
            provider_profile_id: DEEPSEEK_PROFILE_ID.to_owned(),
            protocol_adapter_id: DEEPSEEK_RESPONSES_ADAPTER_ID.to_owned(),
            base_url: DEEPSEEK_DEFAULT_BASE_URL.to_owned(),
            credential: CredentialReference {
                reference: "secret://deepseek/main".to_owned(),
            },
            provider_rules_id: Some(DEEPSEEK_PROFILE_ID.to_owned()),
            region: None,
            account: None,
        };
        let credential =
            ResolvedCredential::bearer("secret://deepseek/main", "secret-value").unwrap();
        let context = DiscoveryContext {
            profile: &profile,
            instance: &instance,
            credential: &credential,
        };
        validate_deepseek_context(&context).unwrap();
        let request = deepseek_models_request(&context).unwrap();
        assert_eq!(request.method, Method::GET);
        assert_eq!(request.url, "https://api.deepseek.com/models");
        assert_eq!(request.headers[AUTHORIZATION], "Bearer secret-value");
        assert!(!format!("{request:?}").contains("secret-value"));
    }

    #[test]
    fn rules_and_dialects_build_complete_inventory_identity_for_all_three_providers() {
        for provider in wp08e_builtin_providers() {
            let profile_id = provider.profile.provider_profile_id.clone();
            let model_id = format!("{profile_id}-fixture-model");
            let model_driver: ModelDriverCatalog = serde_json::from_value(json!({
                "format": "buckyos.aicc.model-driver-catalog",
                "schema_version": 1,
                "schema_revision": 0,
                "model_driver_id": profile_id,
                "revision_seq": 3,
                "models": [{
                    "id": model_id,
                    "api_types": ["llm"],
                    "capabilities": {"reasoning": true, "tool_calling": true}
                }],
                "patterns": [],
                "defaults": {},
                "variants": [],
                "version_rules": []
            }))
            .unwrap();
            let known = KnownProviderCatalog {
                format: "buckyos.aicc.known-provider-catalog".to_owned(),
                schema_version: 1,
                schema_revision: 0,
                revision_seq: 3,
                catalog_id: format!("{profile_id}-fixture"),
                providers: vec![provider.known_provider()],
            };
            let catalog = CatalogSnapshot::build(
                3,
                CatalogDocuments {
                    model_drivers: vec![model_driver],
                    provider_rules: vec![provider.provider_rules(3)],
                    known_providers: vec![known],
                },
                &CatalogBuildOptions::default(),
            )
            .unwrap();
            let (base_descriptor, base_registration) = openai_responses_adapter();
            let mut codecs = CodecRegistry::default();
            codecs
                .register_codecs(base_descriptor, base_registration)
                .unwrap();
            for (descriptor, registration) in wp08e_responses_adapters().unwrap() {
                codecs.register_derived(descriptor, registration).unwrap();
            }
            let base_url = if profile_id == QWEN_PROFILE_ID {
                provider.resolve_base_url(None, Some("workspace1")).unwrap()
            } else {
                provider.resolve_base_url(None, None).unwrap()
            };
            let instance = ProviderInstanceConfig {
                provider_instance_name: format!("{profile_id}-main"),
                provider_profile_id: profile_id.clone(),
                protocol_adapter_id: provider.profile.default_protocol_adapter_id.clone(),
                base_url,
                credential: CredentialReference {
                    reference: format!("secret://{profile_id}/main"),
                },
                provider_rules_id: Some(profile_id.clone()),
                region: None,
                account: None,
            };
            let inventory = InventoryBuilder::build(
                &provider.profile,
                &instance,
                ProviderDiscoverySnapshot {
                    revision: Some("fixture-v1".to_owned()),
                    discovered_at_ms: 1,
                    health: ProviderHealthState::Healthy,
                    models: vec![catalog_model(model_id.clone())],
                },
                &catalog,
                &codecs,
            )
            .unwrap();
            assert_eq!(inventory.provider_profile_id, profile_id);
            assert_eq!(inventory.models.len(), 1);
            assert_eq!(inventory.models[0].provider_model_id, model_id);
            assert_eq!(inventory.models[0].api_types, vec![ApiType::Llm]);
            assert_eq!(
                inventory.models[0].operations["llm"],
                OPENAI_RESPONSES_OPERATION_ID
            );
        }
    }
}
