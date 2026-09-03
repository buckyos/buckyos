use super::super::{
    validate_discovery, CatalogOnlyDiscovery, CredentialDescriptor, DiscoveredModel,
    DiscoveryContext, DiscoveryMode, ModelAvailability, ProviderConnectionContract,
    ProviderConnectionInput, ProviderDiscovery, ProviderDiscoverySnapshot, ProviderError,
    ProviderHealthState, ProviderProfile, ProviderResult, RefreshPolicy,
};
use crate::catalog::{
    CatalogKind, CurrentCatalogFile, KnownProvider, KnownProviderCatalog, ModelDriverCatalog,
    ProviderRulesCatalog,
};
use crate::protocol::{
    CredentialKind, HttpRequest, HttpResponse, HttpTransport, ResponsesDialectKind,
    DEEPSEEK_RESPONSES_ADAPTER_ID, OPENAI_RESPONSES_OPERATION_ID,
};
use async_trait::async_trait;
use buckyos_api::ApiType;
use reqwest::header::ETAG;
use reqwest::Method;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

pub(crate) const DEEPSEEK_PROFILE_ID: &str = "deepseek";
pub(crate) const DOUBAO_PROFILE_ID: &str = "doubao";
pub(crate) const QWEN_PROFILE_ID: &str = "qwen";

const KNOWN_PROVIDERS: &[u8] =
    include_bytes!("../../../driver_metadata/known-providers/wp08e.known-provider.json");
const DEEPSEEK_PROVIDER_RULES: &[u8] =
    include_bytes!("../../../driver_metadata/providers/deepseek.provider.json");
const DOUBAO_PROVIDER_RULES: &[u8] =
    include_bytes!("../../../driver_metadata/providers/doubao.provider.json");
const QWEN_PROVIDER_RULES: &[u8] =
    include_bytes!("../../../driver_metadata/providers/qwen.provider.json");
const DEEPSEEK_MODEL_DRIVER: &[u8] =
    include_bytes!("../../../driver_metadata/models/deepseek.model.json");
const DOUBAO_MODEL_DRIVER: &[u8] =
    include_bytes!("../../../driver_metadata/models/doubao.model.json");
const QWEN_MODEL_DRIVER: &[u8] = include_bytes!("../../../driver_metadata/models/qwen.model.json");

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum BuiltinDiscoveryKind {
    OpenAiModelsApi,
    CatalogOnly,
}

#[derive(Clone, Debug)]
pub(crate) struct BuiltinProviderDescriptor {
    pub profile: ProviderProfile,
    pub connection: ProviderConnectionContract,
    pub discovery: BuiltinDiscoveryKind,
    pub dialect: ResponsesDialectKind,
    known_provider: KnownProvider,
    provider_rules: ProviderRulesCatalog,
}

impl BuiltinProviderDescriptor {
    pub(crate) fn known_provider(&self) -> KnownProvider {
        self.known_provider.clone()
    }

    pub(crate) fn provider_rules(&self, _revision_seq: u64) -> ProviderRulesCatalog {
        self.provider_rules.clone()
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
    let known = decode_catalog::<KnownProviderCatalog>(KNOWN_PROVIDERS, "WP-08E Known Provider");
    let rules: [ProviderRulesCatalog; 3] = [
        decode_catalog(DEEPSEEK_PROVIDER_RULES, "DeepSeek Provider Rules"),
        decode_catalog(DOUBAO_PROVIDER_RULES, "Doubao Provider Rules"),
        decode_catalog(QWEN_PROVIDER_RULES, "Qwen Provider Rules"),
    ];
    [
        (
            DEEPSEEK_PROFILE_ID,
            BuiltinDiscoveryKind::OpenAiModelsApi,
            ResponsesDialectKind::DeepSeek,
        ),
        (
            DOUBAO_PROFILE_ID,
            BuiltinDiscoveryKind::CatalogOnly,
            ResponsesDialectKind::Doubao,
        ),
        (
            QWEN_PROFILE_ID,
            BuiltinDiscoveryKind::CatalogOnly,
            ResponsesDialectKind::Qwen,
        ),
    ]
    .into_iter()
    .map(|(profile_id, discovery, dialect)| {
        let provider = known
            .providers
            .iter()
            .find(|provider| provider.provider_profile_id == profile_id)
            .cloned()
            .unwrap_or_else(|| panic!("WP-08E Known Provider is missing `{profile_id}`"));
        let provider_rules = rules
            .iter()
            .find(|rules| rules.provider_profile_id == profile_id)
            .cloned()
            .unwrap_or_else(|| panic!("WP-08E Provider Rules are missing `{profile_id}`"));
        descriptor(provider, provider_rules, discovery, dialect)
    })
    .collect()
}

pub(crate) fn wp08e_catalog_files() -> Vec<CurrentCatalogFile> {
    [
        (CatalogKind::KnownProvider, KNOWN_PROVIDERS),
        (CatalogKind::ProviderRules, DEEPSEEK_PROVIDER_RULES),
        (CatalogKind::ProviderRules, DOUBAO_PROVIDER_RULES),
        (CatalogKind::ProviderRules, QWEN_PROVIDER_RULES),
        (CatalogKind::ModelDriver, DEEPSEEK_MODEL_DRIVER),
        (CatalogKind::ModelDriver, DOUBAO_MODEL_DRIVER),
        (CatalogKind::ModelDriver, QWEN_MODEL_DRIVER),
    ]
    .into_iter()
    .map(|(kind, contents)| CurrentCatalogFile {
        kind,
        contents: contents.to_vec(),
    })
    .collect()
}

pub(crate) fn wp08e_model_driver_catalogs() -> Vec<ModelDriverCatalog> {
    [
        (DEEPSEEK_MODEL_DRIVER, "DeepSeek Model Driver"),
        (DOUBAO_MODEL_DRIVER, "Doubao Model Driver"),
        (QWEN_MODEL_DRIVER, "Qwen Model Driver"),
    ]
    .into_iter()
    .map(|(contents, label)| decode_catalog(contents, label))
    .collect()
}

pub(crate) fn deepseek_models_discovery(transport: HttpTransport) -> DeepSeekModelsDiscovery {
    DeepSeekModelsDiscovery::new(transport)
}

fn deepseek() -> BuiltinProviderDescriptor {
    configured_provider(DEEPSEEK_PROFILE_ID)
}

fn doubao() -> BuiltinProviderDescriptor {
    configured_provider(DOUBAO_PROFILE_ID)
}

fn qwen() -> BuiltinProviderDescriptor {
    configured_provider(QWEN_PROFILE_ID)
}

fn descriptor(
    known_provider: KnownProvider,
    provider_rules: ProviderRulesCatalog,
    discovery: BuiltinDiscoveryKind,
    dialect: ResponsesDialectKind,
) -> BuiltinProviderDescriptor {
    let connection = known_provider
        .ui_hints
        .get("instance_fields")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_else(|| {
            panic!(
                "Known Provider `{}` has an invalid instance_fields schema",
                known_provider.provider_profile_id
            )
        });
    let credential_type = known_provider
        .ui_hints
        .get("credential_type")
        .and_then(|value| value.as_str());
    let credential = match credential_type {
        Some("bearer") => CredentialDescriptor {
            kind: CredentialKind::Bearer,
            header_name: None,
        },
        _ => panic!(
            "Known Provider `{}` has an unsupported credential_type",
            known_provider.provider_profile_id
        ),
    };
    assert_eq!(
        known_provider.provider_rules_id.as_deref(),
        Some(provider_rules.provider_profile_id.as_str())
    );
    assert_eq!(
        known_provider.protocol_adapter_id,
        dialect.contract().protocol_adapter_id
    );
    BuiltinProviderDescriptor {
        profile: ProviderProfile {
            provider_profile_id: known_provider.provider_profile_id.clone(),
            display_name: known_provider.display_name.clone(),
            default_protocol_adapter_id: known_provider.protocol_adapter_id.clone(),
            credential,
            discovery_mode: match discovery {
                BuiltinDiscoveryKind::OpenAiModelsApi => DiscoveryMode::MachineApi,
                BuiltinDiscoveryKind::CatalogOnly => DiscoveryMode::CatalogOnly,
            },
            refresh: RefreshPolicy::default(),
            default_inventory: None,
        },
        connection,
        discovery,
        dialect,
        known_provider,
        provider_rules,
    }
}

fn configured_provider(profile_id: &str) -> BuiltinProviderDescriptor {
    wp08e_builtin_providers()
        .into_iter()
        .find(|provider| provider.profile.provider_profile_id == profile_id)
        .unwrap_or_else(|| panic!("WP-08E configuration is missing `{profile_id}`"))
}

fn decode_catalog<T: DeserializeOwned>(contents: &[u8], label: &str) -> T {
    serde_json::from_slice(contents)
        .unwrap_or_else(|error| panic!("{label} configuration is invalid: {error}"))
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
    use crate::catalog::{CatalogBuildOptions, CatalogSnapshot};
    use crate::protocol::{
        openai_responses_adapter, wp08e_responses_adapters, CodecRegistry, ResolvedCredential,
    };
    use crate::provider::{CredentialReference, InventoryBuilder, ProviderInstanceConfig};
    use reqwest::header::AUTHORIZATION;
    use serde_json::{json, Value};

    #[test]
    fn bundled_provider_and_model_catalogs_build_one_snapshot() {
        let catalog = CatalogSnapshot::from_current_files(
            1,
            wp08e_catalog_files(),
            &CatalogBuildOptions::default(),
        )
        .unwrap();
        for profile_id in [DEEPSEEK_PROFILE_ID, DOUBAO_PROFILE_ID, QWEN_PROFILE_ID] {
            assert!(catalog.known_provider(profile_id).is_some());
            let rules = catalog.provider_rules(profile_id).unwrap();
            assert_eq!(
                rules.metadata_drivers.as_deref(),
                Some(&[profile_id.to_owned()][..])
            );
            assert_eq!(
                rules.patterns[0].operations.get("llm"),
                Some(&OPENAI_RESPONSES_OPERATION_ID.to_owned())
            );
            assert!(catalog.model_driver(profile_id).is_some());
        }

        let deepseek = catalog.model_driver(DEEPSEEK_PROFILE_ID).unwrap();
        let flash = deepseek
            .models
            .iter()
            .find(|model| model.id == "deepseek-v4-flash")
            .unwrap();
        assert_eq!(
            flash.capabilities.as_ref().unwrap()["max_context_tokens"],
            1_000_000
        );
        assert_eq!(
            flash.capabilities.as_ref().unwrap()["max_output_tokens"],
            393_216
        );

        let doubao = catalog.model_driver(DOUBAO_PROFILE_ID).unwrap();
        assert!(doubao
            .models
            .iter()
            .any(|model| model.id == "doubao-seed-2-0-lite-260215"));
        assert!(doubao
            .patterns
            .iter()
            .any(|model| model.parameter_scale.as_deref() == Some("pro")));

        let qwen = catalog.model_driver(QWEN_PROFILE_ID).unwrap();
        assert!(qwen.patterns.iter().any(|model| {
            model.parameter_scale.as_deref() == Some("max")
                && model.capabilities.as_ref().unwrap()["max_context_tokens"] == 1_000_000
        }));
    }

    #[test]
    fn profiles_are_assembled_from_known_provider_configuration() {
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
        assert_eq!(
            providers[1].known_provider().base_url,
            "https://ark.cn-beijing.volces.com/api/v3"
        );
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
    fn provider_rules_are_loaded_without_rust_generated_revisions() {
        for provider in wp08e_builtin_providers() {
            let rules = provider.provider_rules(7);
            assert_eq!(rules.revision_seq, 1);
            assert!(rules.models.is_empty());
            assert_eq!(rules.patterns.len(), 1);
            assert_eq!(
                rules.patterns[0].operations.get("llm"),
                Some(&OPENAI_RESPONSES_OPERATION_ID.to_string())
            );
        }
        assert!(deepseek().provider_rules(99).patterns[0].request_rules[0]
            .remove
            .contains(&"/store".to_owned()));
        assert!(qwen().provider_rules(99).patterns[0].request_rules[0]
            .remove
            .contains(&"/background".to_owned()));
    }

    #[test]
    fn known_provider_fixture_keeps_templates_and_ui_schema() {
        let providers = wp08e_builtin_providers();
        let known = providers
            .iter()
            .map(BuiltinProviderDescriptor::known_provider)
            .collect::<Vec<_>>();
        assert_eq!(known[0].base_url, "https://api.deepseek.com");
        assert_eq!(
            known[2].base_url,
            "https://{workspace}.{region}.maas.aliyuncs.com/compatible-mode/v1"
        );
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
            base_url: deepseek().known_provider().base_url,
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
        let catalog = CatalogSnapshot::from_current_files(
            1,
            wp08e_catalog_files(),
            &CatalogBuildOptions::default(),
        )
        .unwrap();
        for (provider, model_id) in wp08e_builtin_providers().into_iter().zip([
            "deepseek-v4-flash",
            "doubao-seed-2-0-lite-260215",
            "qwen3.8-max",
        ]) {
            let profile_id = provider.profile.provider_profile_id.clone();
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
                    models: vec![catalog_model(model_id.to_owned())],
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
