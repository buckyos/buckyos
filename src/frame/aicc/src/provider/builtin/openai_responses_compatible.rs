use super::super::{
    validate_discovery, DiscoveredModel, DiscoveryContext, ModelAvailability, ProviderDiscovery,
    ProviderDiscoverySnapshot, ProviderError, ProviderHealthState, ProviderResult,
};
#[cfg(test)]
use super::super::{
    CredentialDescriptor, DiscoveryMode, ProviderConnectionContract, ProviderConnectionInput,
    ProviderProfile, RefreshPolicy,
};
#[cfg(test)]
use crate::catalog::{
    CatalogKind, CurrentCatalogFile, KnownProvider, KnownProviderCatalog, ProviderRulesCatalog,
};
#[cfg(test)]
use crate::protocol::CredentialKind;
#[cfg(test)]
use crate::protocol::ResponsesDialectKind;
use crate::protocol::{HttpRequest, HttpResponse, HttpTransport};
use async_trait::async_trait;
use buckyos_api::ApiType;
use reqwest::header::ETAG;
use reqwest::Method;
#[cfg(test)]
use serde::Deserialize;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

pub(crate) const DEEPSEEK_PROFILE_ID: &str = "deepseek";
pub(crate) const DOUBAO_PROFILE_ID: &str = "doubao";
pub(crate) const DOUBAO_AGENT_PLAN_PROFILE_ID: &str = "doubao-agent-plan";
pub(crate) const DOUBAO_SPEECH_PROFILE_ID: &str = "doubao-speech";
pub(crate) const QWEN_PROFILE_ID: &str = "qwen";

#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum BuiltinDiscoveryKind {
    OpenAiModelsApi,
    CatalogOnly,
}

#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) struct BuiltinProviderDescriptor {
    pub profile: ProviderProfile,
    pub connection: ProviderConnectionContract,
    pub discovery: BuiltinDiscoveryKind,
    pub dialect: ResponsesDialectKind,
    known_provider: KnownProvider,
    provider_rules: ProviderRulesCatalog,
}

#[cfg(test)]
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
}

#[cfg(test)]
pub(crate) fn openai_responses_compatible_builtin_providers() -> Vec<BuiltinProviderDescriptor> {
    let known = crate::settings::load_builtin_metadata()
        .expect("WP-15 builtin metadata must load")
        .into_iter()
        .filter(|file| file.kind == CatalogKind::KnownProvider)
        .map(|file| {
            serde_json::from_slice::<KnownProviderCatalog>(&file.contents).unwrap_or_else(|error| {
                panic!(
                    "WP-15 builtin metadata `{}` is invalid: {error}",
                    file.catalog_id
                )
            })
        })
        .collect::<Vec<_>>();
    let rules: [ProviderRulesCatalog; 4] = [
        super::builtin_catalog_document(CatalogKind::ProviderRules, DEEPSEEK_PROFILE_ID),
        super::builtin_catalog_document(CatalogKind::ProviderRules, DOUBAO_PROFILE_ID),
        super::builtin_catalog_document(CatalogKind::ProviderRules, DOUBAO_AGENT_PLAN_PROFILE_ID),
        super::builtin_catalog_document(CatalogKind::ProviderRules, QWEN_PROFILE_ID),
    ];
    [
        (
            DEEPSEEK_PROFILE_ID,
            BuiltinDiscoveryKind::OpenAiModelsApi,
            ResponsesDialectKind::DeepSeek,
        ),
        (
            DOUBAO_PROFILE_ID,
            BuiltinDiscoveryKind::OpenAiModelsApi,
            ResponsesDialectKind::Doubao,
        ),
        (
            DOUBAO_AGENT_PLAN_PROFILE_ID,
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
            .iter()
            .flat_map(|catalog| catalog.providers.iter())
            .find(|provider| provider.provider_profile_id == profile_id)
            .cloned()
            .unwrap_or_else(|| panic!("Known Provider configuration is missing `{profile_id}`"));
        let provider_rules = rules
            .iter()
            .find(|rules| rules.provider_profile_id == profile_id)
            .cloned()
            .unwrap_or_else(|| panic!("WP-08E Provider Rules are missing `{profile_id}`"));
        descriptor(provider, provider_rules, discovery, dialect)
    })
    .collect()
}

#[cfg(test)]
pub(crate) fn openai_responses_compatible_catalog_files() -> Vec<CurrentCatalogFile> {
    super::builtin_catalog_files(&[
        DEEPSEEK_PROFILE_ID,
        DOUBAO_PROFILE_ID,
        DOUBAO_AGENT_PLAN_PROFILE_ID,
        DOUBAO_SPEECH_PROFILE_ID,
        QWEN_PROFILE_ID,
        "glm",
        "kimi",
        "minimax",
    ])
}

pub(crate) fn openai_compatible_models_discovery(
    provider_profile_id: impl Into<String>,
    protocol_adapter_id: impl Into<String>,
    transport: HttpTransport,
) -> OpenAiCompatibleModelsDiscovery {
    OpenAiCompatibleModelsDiscovery::new(provider_profile_id, protocol_adapter_id, transport)
}

#[cfg(test)]
fn deepseek() -> BuiltinProviderDescriptor {
    configured_provider(DEEPSEEK_PROFILE_ID)
}

#[cfg(test)]
fn doubao() -> BuiltinProviderDescriptor {
    configured_provider(DOUBAO_PROFILE_ID)
}

#[cfg(test)]
fn doubao_agent_plan() -> BuiltinProviderDescriptor {
    configured_provider(DOUBAO_AGENT_PLAN_PROFILE_ID)
}

#[cfg(test)]
fn qwen() -> BuiltinProviderDescriptor {
    configured_provider(QWEN_PROFILE_ID)
}

#[cfg(test)]
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
            credential_variants: Vec::new(),
            discovery_mode: match discovery {
                BuiltinDiscoveryKind::OpenAiModelsApi => DiscoveryMode::MachineApi,
                BuiltinDiscoveryKind::CatalogOnly => DiscoveryMode::CatalogOnly,
            },
            refresh: RefreshPolicy::default(),
            default_inventory: None,
            accepts_any_adapter: false,
        },
        connection,
        discovery,
        dialect,
        known_provider,
        provider_rules,
    }
}

#[cfg(test)]
fn configured_provider(profile_id: &str) -> BuiltinProviderDescriptor {
    openai_responses_compatible_builtin_providers()
        .into_iter()
        .find(|provider| provider.profile.provider_profile_id == profile_id)
        .unwrap_or_else(|| panic!("Provider configuration is missing `{profile_id}`"))
}

fn catalog_model(provider_model_id: String) -> DiscoveredModel {
    DiscoveredModel {
        provider_model_id,
        api_types: None,
        supported_features: None,
        unsupported_features: BTreeSet::new(),
        remote_methods: None,
        availability: ModelAvailability::Available,
        deprecated: false,
        pricing: None,
    }
}

#[cfg(test)]
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
pub(super) trait OpenAiCompatibleModelsTransport: Send + Sync {
    async fn send(
        &self,
        request: HttpRequest,
    ) -> crate::protocol::ProtocolResultValue<HttpResponse>;
}

#[async_trait]
impl OpenAiCompatibleModelsTransport for HttpTransport {
    async fn send(
        &self,
        request: HttpRequest,
    ) -> crate::protocol::ProtocolResultValue<HttpResponse> {
        HttpTransport::send(self, request).await
    }
}

#[derive(Clone)]
pub(crate) struct OpenAiCompatibleModelsDiscovery {
    provider_profile_id: String,
    protocol_adapter_id: String,
    transport: Arc<dyn OpenAiCompatibleModelsTransport>,
}

#[derive(Clone)]
pub(crate) struct VolcengineArkModelsDiscovery(OpenAiCompatibleModelsDiscovery);

impl VolcengineArkModelsDiscovery {
    pub(crate) fn new(
        provider_profile_id: impl Into<String>,
        protocol_adapter_id: impl Into<String>,
        transport: HttpTransport,
    ) -> Self {
        Self(OpenAiCompatibleModelsDiscovery::new(
            provider_profile_id,
            protocol_adapter_id,
            transport,
        ))
    }
}

impl OpenAiCompatibleModelsDiscovery {
    pub(crate) fn new(
        provider_profile_id: impl Into<String>,
        protocol_adapter_id: impl Into<String>,
        transport: HttpTransport,
    ) -> Self {
        Self {
            provider_profile_id: provider_profile_id.into(),
            protocol_adapter_id: protocol_adapter_id.into(),
            transport: Arc::new(transport),
        }
    }
}

#[derive(Deserialize)]
#[cfg(test)]
struct ModelsEnvelope {
    data: Vec<ModelObject>,
    #[serde(default)]
    object: Option<String>,
}

#[derive(Deserialize)]
#[cfg(test)]
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
impl ProviderDiscovery for OpenAiCompatibleModelsDiscovery {
    fn match_model_driver(
        &self,
        id: &str,
        catalog: &crate::catalog::CatalogSnapshot,
    ) -> crate::catalog::ProviderModelMatch {
        use crate::catalog::{ModelIdentity, ModelMatchFailure, ProviderModelMatch};
        if self.provider_profile_id == DEEPSEEK_PROFILE_ID {
            if matches!(
                id,
                "deepseek-v4-flash" | "deepseek-v4-flash-vision-exp" | "deepseek-flash"
            ) {
                let model = "deepseek-v4.1-flash";
                return if catalog.resolve_model("deepseek", model).is_ok() {
                    ProviderModelMatch::Matched(ModelIdentity {
                        model_driver_id: "deepseek".into(),
                        model_id: model.into(),
                    })
                } else {
                    ProviderModelMatch::Failed(ModelMatchFailure::UnresolvedAlias)
                };
            }
            if matches!(id, "deepseek-chat" | "deepseek-reasoner") {
                return ProviderModelMatch::Failed(ModelMatchFailure::UnresolvedAlias);
            }
        }
        ProviderModelMatch::NotHandled
    }

    async fn discover(
        &self,
        context: &DiscoveryContext<'_>,
    ) -> ProviderResult<ProviderDiscoverySnapshot> {
        validate_openai_compatible_models_context(
            context,
            &self.provider_profile_id,
            &self.protocol_adapter_id,
        )?;
        let request = openai_compatible_models_request(context, &self.provider_profile_id)?;
        let (models, revision) = discover_model_ids(
            self.transport.as_ref(),
            request,
            &self.provider_profile_id,
            false,
        )
        .await?;
        let snapshot = ProviderDiscoverySnapshot {
            revision,
            discovered_at_ms: super::super::now_ms()?,
            health: ProviderHealthState::Healthy,
            models,
        };
        validate_discovery(&snapshot)?;
        Ok(snapshot)
    }
}

#[async_trait]
impl ProviderDiscovery for VolcengineArkModelsDiscovery {
    async fn discover(
        &self,
        context: &DiscoveryContext<'_>,
    ) -> ProviderResult<ProviderDiscoverySnapshot> {
        validate_openai_compatible_models_context(
            context,
            &self.0.provider_profile_id,
            &self.0.protocol_adapter_id,
        )?;
        let request = openai_compatible_models_request(context, &self.0.provider_profile_id)?;
        let (models, revision) = discover_volcengine_ark_models(
            self.0.transport.as_ref(),
            request,
            &self.0.provider_profile_id,
        )
        .await?;
        let snapshot = ProviderDiscoverySnapshot {
            revision,
            discovered_at_ms: super::super::now_ms()?,
            health: ProviderHealthState::Healthy,
            models,
        };
        validate_discovery(&snapshot)?;
        Ok(snapshot)
    }
}

pub(super) async fn discover_volcengine_ark_models(
    transport: &dyn OpenAiCompatibleModelsTransport,
    request: HttpRequest,
    provider: &str,
) -> ProviderResult<(Vec<DiscoveredModel>, Option<String>)> {
    let limit = request.max_response_bytes.unwrap_or(1024 * 1024);
    let response = transport
        .send(request)
        .await
        .map_err(|error| ProviderError::Discovery(error.to_string()))?;
    ensure_openai_compatible_models_success(&response, provider)?;
    let revision = response
        .headers
        .get(ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let value: serde_json::Value = response
        .json(limit)
        .map_err(|error| ProviderError::DiscoveryResponse(error.to_string()))?;
    if value
        .get("object")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|kind| kind != "list")
    {
        return Err(ProviderError::DiscoveryResponse(format!(
            "{provider} models response must be a list"
        )));
    }
    let data = value
        .get("data")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            ProviderError::DiscoveryResponse(format!("{provider} models response requires data"))
        })?;
    let mut models = std::collections::BTreeMap::new();
    for item in data {
        let id = item
            .get("id")
            .and_then(serde_json::Value::as_str)
            .filter(|id| !id.trim().is_empty() && !id.contains('@'))
            .ok_or_else(|| {
                ProviderError::DiscoveryResponse(format!("{provider} invalid model id"))
            })?;
        if item
            .get("object")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|kind| kind != "model")
        {
            return Err(ProviderError::DiscoveryResponse(format!(
                "{provider} invalid model object"
            )));
        }
        let mut model = catalog_model(id.into());
        if matches!(
            item.get("status").and_then(serde_json::Value::as_str),
            Some("Retiring" | "Shutdown")
        ) {
            model.availability = ModelAvailability::Unavailable;
            model.deprecated = true;
        }
        if let Some(task_types) = item.get("task_type").and_then(serde_json::Value::as_array) {
            let mut api_types = Vec::new();
            let mut add_api_type = |api_type| {
                if !api_types.contains(&api_type) {
                    api_types.push(api_type);
                }
            };
            for task_type in task_types.iter().filter_map(serde_json::Value::as_str) {
                match task_type {
                    "TextGeneration" => {
                        add_api_type(ApiType::Llm);
                    }
                    "VisualQuestionAnswering" => {
                        add_api_type(ApiType::VisionOcr);
                        add_api_type(ApiType::VisionCaption);
                    }
                    "TextEmbedding" => {
                        add_api_type(ApiType::EmbeddingText);
                    }
                    "ImageEmbedding" => {
                        add_api_type(ApiType::EmbeddingMultimodal);
                    }
                    "TextToImage" => {
                        add_api_type(ApiType::ImageTextToImage);
                    }
                    "ImageToImage" => {
                        add_api_type(ApiType::ImageImageToImage);
                    }
                    "TextToVideo" => {
                        add_api_type(ApiType::VideoTextToVideo);
                    }
                    "ImageToVideo" => {
                        add_api_type(ApiType::VideoImageToVideo);
                    }
                    "MultimodalToVideo" => {
                        add_api_type(ApiType::VideoTextToVideo);
                        add_api_type(ApiType::VideoImageToVideo);
                    }
                    "VideoEditing" => {
                        add_api_type(ApiType::VideoToVideo);
                    }
                    "VideoExtension" => {
                        add_api_type(ApiType::VideoExtend);
                    }
                    _ => {}
                }
            }
            if !task_types.is_empty() {
                model.api_types = Some(api_types);
            }
        }
        if models.insert(id.to_owned(), model).is_some() {
            return Err(ProviderError::DiscoveryResponse(format!(
                "{provider} duplicate model id {id}"
            )));
        }
    }
    Ok((models.into_values().collect(), revision))
}

pub(super) async fn discover_model_ids(
    transport: &dyn OpenAiCompatibleModelsTransport,
    request: HttpRequest,
    provider: &str,
    strict: bool,
) -> ProviderResult<(Vec<DiscoveredModel>, Option<String>)> {
    let limit = request.max_response_bytes.unwrap_or(1024 * 1024);
    let response = transport
        .send(request)
        .await
        .map_err(|error| ProviderError::Discovery(error.to_string()))?;
    ensure_openai_compatible_models_success(&response, provider)?;
    let revision = response
        .headers
        .get(ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let value: serde_json::Value = response
        .json(limit)
        .map_err(|error| ProviderError::DiscoveryResponse(error.to_string()))?;
    if value
        .get("object")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|kind| kind != "list")
        || (strict && value.get("object").and_then(serde_json::Value::as_str) != Some("list"))
    {
        return Err(ProviderError::DiscoveryResponse(format!(
            "{provider} models response must be a list"
        )));
    }
    let data = value
        .get("data")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            ProviderError::DiscoveryResponse(format!("{provider} models response requires data"))
        })?;
    let mut models = std::collections::BTreeMap::new();
    for item in data {
        let id = item
            .get("id")
            .and_then(serde_json::Value::as_str)
            .filter(|id| !id.trim().is_empty() && !id.contains('@'))
            .ok_or_else(|| {
                ProviderError::DiscoveryResponse(format!("{provider} invalid model id"))
            })?;
        if item
            .get("object")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|kind| kind != "model")
            || (strict && item.get("object").and_then(serde_json::Value::as_str) != Some("model"))
        {
            return Err(ProviderError::DiscoveryResponse(format!(
                "{provider} invalid model object"
            )));
        }
        let mut model = catalog_model(id.into());
        if provider == "kimi" {
            for (field, feature) in [("supports_reasoning", "reasoning")] {
                if item.get(field).and_then(serde_json::Value::as_bool) == Some(false) {
                    model.unsupported_features.insert(feature.into());
                }
            }
            if item
                .get("supports_image_in")
                .and_then(serde_json::Value::as_bool)
                == Some(false)
                && item
                    .get("supports_video_in")
                    .and_then(serde_json::Value::as_bool)
                    == Some(false)
            {
                model.unsupported_features.insert("vision".into());
                model.api_types = Some(vec![ApiType::Llm]);
            }
        }
        if models.insert(id.to_owned(), model).is_some() {
            return Err(ProviderError::DiscoveryResponse(format!(
                "{provider} duplicate model id {id}"
            )));
        }
    }
    Ok((models.into_values().collect(), revision))
}

fn validate_openai_compatible_models_context(
    context: &DiscoveryContext<'_>,
    provider_profile_id: &str,
    protocol_adapter_id: &str,
) -> ProviderResult<()> {
    if context.profile.provider_profile_id != provider_profile_id
        || context.profile.default_protocol_adapter_id != protocol_adapter_id
        || context.instance.provider_profile_id != provider_profile_id
        || context.instance.protocol_adapter_id != protocol_adapter_id
    {
        return Err(ProviderError::InvalidConfiguration(format!(
            "{provider_profile_id} discovery requires its Responses dialect"
        )));
    }
    if context.instance.account.is_some() {
        return Err(ProviderError::InvalidConfiguration(format!(
            "{provider_profile_id} profile does not accept account fields"
        )));
    }
    Ok(())
}

fn openai_compatible_models_request(
    context: &DiscoveryContext<'_>,
    provider_profile_id: &str,
) -> ProviderResult<HttpRequest> {
    let mut base = reqwest::Url::parse(&context.instance.base_url).map_err(|_| {
        ProviderError::InvalidConfiguration(format!("{provider_profile_id} base_url is invalid"))
    })?;
    if !base.path().ends_with('/') {
        let path = format!("{}/", base.path());
        base.set_path(&path);
    }
    let url = base.join("models").map_err(|_| {
        ProviderError::InvalidConfiguration(format!("{provider_profile_id} models URL is invalid"))
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

fn ensure_openai_compatible_models_success(
    response: &HttpResponse,
    provider_profile_id: &str,
) -> ProviderResult<()> {
    if response.status.is_success() {
        return Ok(());
    }
    let message = serde_json::from_slice::<serde_json::Value>(&response.body)
        .ok()
        .and_then(|body| {
            body.pointer("/error/message")
                .and_then(serde_json::Value::as_str)
                .map(|m| m.chars().take(512).collect::<String>())
        })
        .unwrap_or_default();
    let message = format!(
        "{provider_profile_id} Models API returned status {} (request {}): {message}",
        response.status.as_u16(),
        response.request_id
    );
    Err(if matches!(response.status.as_u16(), 401 | 403) {
        ProviderError::Credential(message)
    } else {
        ProviderError::Discovery(message)
    })
}

#[cfg(test)]
fn parse_deepseek_models(
    envelope: ModelsEnvelope,
    revision: Option<String>,
) -> ProviderResult<ProviderDiscoverySnapshot> {
    parse_openai_compatible_models(envelope, revision, DEEPSEEK_PROFILE_ID)
}

#[cfg(test)]
fn parse_openai_compatible_models(
    envelope: ModelsEnvelope,
    revision: Option<String>,
    _provider_profile_id: &str,
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
fn validate_deepseek_context(context: &DiscoveryContext<'_>) -> ProviderResult<()> {
    validate_openai_compatible_models_context(
        context,
        DEEPSEEK_PROFILE_ID,
        DEEPSEEK_RESPONSES_ADAPTER_ID,
    )
}

#[cfg(test)]
fn deepseek_models_request(context: &DiscoveryContext<'_>) -> ProviderResult<HttpRequest> {
    openai_compatible_models_request(context, DEEPSEEK_PROFILE_ID)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CatalogBuildOptions, CatalogSnapshot};
    use crate::protocol::{
        openai_responses_adapter, openai_responses_compatible_adapters, CodecRegistry,
        ResolvedCredential,
    };
    use crate::provider::{CredentialReference, InventoryBuilder, ProviderInstanceConfig};
    use crate::settings::{MetadataFile, MetadataSource, MetadataSources};
    use reqwest::header::AUTHORIZATION;
    use serde_json::{json, Value};
    use std::collections::BTreeMap;

    #[test]
    fn bundled_provider_and_model_catalogs_build_one_snapshot() {
        let builtin = openai_responses_compatible_catalog_files()
            .into_iter()
            .map(|file| MetadataFile::parse(MetadataSource::Builtin, file.kind, file.contents))
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let catalog = MetadataSources {
            builtin,
            ..MetadataSources::default()
        }
        .build_snapshot(
            crate::settings::BUILTIN_CATALOG_REVISION_SEQ,
            &CatalogBuildOptions::default(),
        )
        .unwrap();
        for profile_id in [
            DEEPSEEK_PROFILE_ID,
            DOUBAO_PROFILE_ID,
            DOUBAO_AGENT_PLAN_PROFILE_ID,
            QWEN_PROFILE_ID,
        ] {
            assert!(catalog.known_provider(profile_id).is_some());
            let rules = catalog.provider_rules(profile_id).unwrap();
            assert_eq!(
                rules
                    .patterns
                    .iter()
                    .find_map(|rule| rule.operations.get("llm")),
                Some(&OPENAI_RESPONSES_OPERATION_ID.to_owned())
            );
            let model_driver_id = if profile_id == DOUBAO_AGENT_PLAN_PROFILE_ID {
                DOUBAO_PROFILE_ID
            } else {
                profile_id
            };
            assert!(catalog.model_driver(model_driver_id).is_some());
        }

        let deepseek = catalog.model_driver(DEEPSEEK_PROFILE_ID).unwrap();
        let flash = deepseek
            .models
            .iter()
            .find(|model| model.id == "deepseek-v4-flash")
            .unwrap();
        assert_eq!(
            flash.capabilities.as_ref().unwrap()["max_context_tokens"],
            1_048_576
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
        assert!(doubao.specs.iter().any(|spec| spec.id == "doubao-pro"));

        let qwen = catalog.model_driver(QWEN_PROFILE_ID).unwrap();
        assert!(qwen.models.iter().any(|model| {
            model.parameter_scale.as_deref() == Some("max")
                && model.capabilities.as_ref().unwrap()["max_context_tokens"] == 1_000_000
        }));
    }

    #[test]
    fn profiles_are_assembled_from_known_provider_configuration() {
        let providers = openai_responses_compatible_builtin_providers();
        assert_eq!(
            providers
                .iter()
                .map(|provider| provider.profile.provider_profile_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                DEEPSEEK_PROFILE_ID,
                DOUBAO_PROFILE_ID,
                DOUBAO_AGENT_PLAN_PROFILE_ID,
                QWEN_PROFILE_ID
            ]
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
            providers[3].connection.workspace.mode,
            crate::provider::ProviderFieldMode::Required
        );
        assert_eq!(
            providers[1].known_provider().base_url,
            "https://ark.cn-beijing.volces.com/api/v3"
        );
        assert_eq!(
            providers[2].known_provider().base_url,
            "https://ark.cn-beijing.volces.com/api/plan/v3"
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
        let snapshot = doubao_agent_plan()
            .catalog_only_inventory(["endpoint-a".to_string(), "endpoint-b".to_string()])
            .unwrap();
        assert_eq!(snapshot.health, ProviderHealthState::Healthy);
        assert_eq!(snapshot.models.len(), 2);
        assert!(snapshot.models[0].supported_features.is_none());
        assert!(snapshot.models[0].api_types.is_none());
        assert!(snapshot.models[0].remote_methods.is_none());
        assert!(doubao_agent_plan()
            .catalog_only_inventory(["endpoint-a".to_string(), "endpoint-a".to_string()])
            .is_err());
        assert!(doubao()
            .catalog_only_inventory(["doubao-seed-2-0-lite-260428".to_string()])
            .is_err());
        assert!(deepseek()
            .catalog_only_inventory(["deepseek-model".to_string()])
            .is_err());
    }

    #[test]
    fn provider_rules_are_loaded_without_rust_generated_revisions() {
        for provider in openai_responses_compatible_builtin_providers() {
            let rules = provider.provider_rules(7);
            assert_eq!(
                rules.revision_seq,
                match provider.profile.provider_profile_id.as_str() {
                    DOUBAO_PROFILE_ID => 12,
                    DOUBAO_AGENT_PLAN_PROFILE_ID => 7,
                    QWEN_PROFILE_ID | DEEPSEEK_PROFILE_ID | "openai" => 4,
                    _ => 3,
                }
            );
            if matches!(
                provider.profile.provider_profile_id.as_str(),
                DOUBAO_PROFILE_ID | DOUBAO_AGENT_PLAN_PROFILE_ID
            ) {
                assert!(!rules.models.is_empty());
            } else {
                assert!(rules.models.is_empty());
            }
            let expected_patterns = match provider.profile.provider_profile_id.as_str() {
                DOUBAO_PROFILE_ID => 7,
                DOUBAO_AGENT_PLAN_PROFILE_ID => 4,
                QWEN_PROFILE_ID => 8,
                _ => 1,
            };
            assert_eq!(rules.patterns.len(), expected_patterns);
            if matches!(
                provider.profile.provider_profile_id.as_str(),
                DOUBAO_PROFILE_ID | DOUBAO_AGENT_PLAN_PROFILE_ID
            ) {
                let vision_rule = rules
                    .patterns
                    .iter()
                    .find(|rule| !rule.request_rules.is_empty())
                    .unwrap();
                assert_eq!(
                    vision_rule.request_rules[0].defaults["max_output_tokens"],
                    2048
                );
                assert_eq!(
                    vision_rule.request_rules[0].defaults["reasoning"]["effort"],
                    "minimal"
                );
            }
            assert_eq!(
                rules
                    .patterns
                    .iter()
                    .find_map(|rule| rule.operations.get("llm")),
                Some(&OPENAI_RESPONSES_OPERATION_ID.to_string())
            );
        }
        assert!(deepseek().provider_rules(99).patterns[0].request_rules[0]
            .remove
            .contains(&"/store".to_owned()));
        let qwen_rules = qwen().provider_rules(99);
        let qwen_llm_rule = qwen_rules
            .patterns
            .iter()
            .find(|rule| rule.operations.contains_key("llm"))
            .unwrap();
        assert!(qwen_llm_rule.request_rules[0]
            .remove
            .contains(&"/background".to_owned()));
    }

    #[test]
    fn known_provider_fixture_keeps_templates_and_ui_schema() {
        let providers = openai_responses_compatible_builtin_providers();
        let known = providers
            .iter()
            .map(BuiltinProviderDescriptor::known_provider)
            .collect::<Vec<_>>();
        assert_eq!(known[0].base_url, "https://api.deepseek.com");
        assert_eq!(known[1].ui_hints["setup_group"]["id"], "doubao");
        assert_eq!(known[1].ui_hints["setup_group"]["account_type"], "standard");
        assert_eq!(known[1].ui_hints["setup_group"]["default"], true);
        assert_eq!(known[2].ui_hints["setup_group"]["id"], "doubao");
        assert_eq!(
            known[2].ui_hints["setup_group"]["account_type"],
            "agent_plan"
        );
        assert_eq!(known[2].ui_hints["setup_group"]["default"], false);
        assert_eq!(
            known[3].base_url,
            "https://{workspace}.{region}.maas.aliyuncs.com/compatible-mode/v1"
        );
        assert_eq!(
            known[3].ui_hints["instance_fields"]["workspace"]["mode"],
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
            operation_base_urls: BTreeMap::new(),
            credential: CredentialReference {
                reference: "secret://deepseek/main".to_owned(),
            },
            credential_kind: None,
            provider_rules_id: Some(DEEPSEEK_PROFILE_ID.to_owned()),
            region: None,
            workspace: None,
            account: None,
            request_timeout: Duration::from_secs(120),
            auto_sync_models: true,
            instance_rules: None,
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
            crate::settings::BUILTIN_CATALOG_REVISION_SEQ,
            openai_responses_compatible_catalog_files(),
            &CatalogBuildOptions::default(),
        )
        .unwrap();
        for (provider, model_id) in openai_responses_compatible_builtin_providers()
            .into_iter()
            .zip([
                "deepseek-v4-flash",
                "doubao-seed-2-0-lite-260215",
                "doubao-seed-2-0-lite-260428",
                "qwen3.8-omni-flash",
            ])
        {
            let profile_id = provider.profile.provider_profile_id.clone();
            let (base_descriptor, base_registration) = openai_responses_adapter();
            let mut codecs = CodecRegistry::default();
            codecs
                .register_codecs(base_descriptor, base_registration)
                .unwrap();
            for (descriptor, registration) in [
                crate::protocol::doubao_media_adapter(),
                crate::protocol::doubao_speech_adapter(),
                crate::protocol::qwen_media_adapter(),
            ] {
                codecs.register_codecs(descriptor, registration).unwrap();
            }
            for (descriptor, registration) in openai_responses_compatible_adapters().unwrap() {
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
                operation_base_urls: BTreeMap::new(),
                credential: CredentialReference {
                    reference: format!("secret://{profile_id}/main"),
                },
                credential_kind: None,
                provider_rules_id: Some(profile_id.clone()),
                region: None,
                workspace: (profile_id == QWEN_PROFILE_ID).then(|| "workspace1".to_owned()),
                account: None,
                request_timeout: Duration::from_secs(120),
                auto_sync_models: true,
                instance_rules: None,
            };
            let mut models = vec![catalog_model(model_id.to_owned())];
            if profile_id == DOUBAO_PROFILE_ID {
                models.push(catalog_model("deepseek-v4-flash".to_owned()));
            }
            let inventory = InventoryBuilder::build(
                &provider.profile,
                &instance,
                ProviderDiscoverySnapshot {
                    revision: Some("fixture-v1".to_owned()),
                    discovered_at_ms: 1,
                    health: ProviderHealthState::Healthy,
                    models,
                },
                &catalog,
                &codecs,
            )
            .unwrap();
            assert_eq!(inventory.provider_profile_id, profile_id);
            let model = inventory
                .models
                .iter()
                .find(|model| model.provider_model_id == model_id)
                .unwrap();
            assert!(model.api_types.contains(&ApiType::Llm));
            assert_eq!(model.operations["llm"], OPENAI_RESPONSES_OPERATION_ID);
            if profile_id == DEEPSEEK_PROFILE_ID {
                assert_eq!(
                    model
                        .variants
                        .iter()
                        .map(|variant| variant.name.as_str())
                        .collect::<Vec<_>>(),
                    vec![
                        "reasoning-high",
                        "reasoning-low",
                        "reasoning-max",
                        "reasoning-none",
                    ]
                );
            }
            if profile_id == QWEN_PROFILE_ID {
                assert!(model.api_types.contains(&ApiType::VisionOcr));
                assert!(model.api_types.contains(&ApiType::VisionCaption));
                assert_eq!(
                    model
                        .variants
                        .iter()
                        .map(|variant| variant.name.as_str())
                        .collect::<Vec<_>>(),
                    vec![
                        "reasoning-low",
                        "reasoning-medium",
                        "reasoning-none",
                        "reasoning-xhigh",
                    ]
                );
            }
        }
    }

    #[test]
    fn doubao_speech_static_inventory_and_model_allowlist_are_explicit() {
        let catalog = CatalogSnapshot::from_current_files(
            crate::settings::BUILTIN_CATALOG_REVISION_SEQ,
            openai_responses_compatible_catalog_files(),
            &CatalogBuildOptions::default(),
        )
        .unwrap();
        let known = catalog
            .known_provider(DOUBAO_SPEECH_PROFILE_ID)
            .expect("doubao-speech Known Provider must be bundled")
            .clone();
        let rules = catalog
            .provider_rules(DOUBAO_SPEECH_PROFILE_ID)
            .expect("doubao-speech Provider Rules must be bundled")
            .clone();
        assert_eq!(
            rules.static_inventory_models,
            vec![
                "doubao-seed-tts-2.0",
                "doubao-seed-icl-2.0",
                "doubao-seed-asr-2.0",
                "doubao-seed-asr-2.0-fast"
            ]
        );
        assert_eq!(
            rules
                .supplemental_inventory_api_types
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            vec!["audio.asr", "audio.tts"]
        );

        let provider = descriptor(
            known,
            rules,
            BuiltinDiscoveryKind::CatalogOnly,
            ResponsesDialectKind::Doubao,
        );
        assert_eq!(
            provider.profile.provider_profile_id,
            DOUBAO_SPEECH_PROFILE_ID
        );
        assert_eq!(provider.profile.discovery_mode, DiscoveryMode::CatalogOnly);

        let mut codecs = CodecRegistry::default();
        let (base_descriptor, base_registration) = openai_responses_adapter();
        codecs
            .register_codecs(base_descriptor, base_registration)
            .unwrap();
        for (descriptor, registration) in [
            crate::protocol::doubao_media_adapter(),
            crate::protocol::doubao_speech_adapter(),
            crate::protocol::qwen_media_adapter(),
        ] {
            codecs.register_codecs(descriptor, registration).unwrap();
        }
        for (descriptor, registration) in openai_responses_compatible_adapters().unwrap() {
            codecs.register_derived(descriptor, registration).unwrap();
        }

        let instance = |enabled: Option<BTreeSet<String>>| ProviderInstanceConfig {
            provider_instance_name: "doubao-speech-main".to_owned(),
            provider_profile_id: DOUBAO_SPEECH_PROFILE_ID.to_owned(),
            protocol_adapter_id: provider.profile.default_protocol_adapter_id.clone(),
            base_url: "https://openspeech.bytedance.com/api/v3/tts".to_owned(),
            operation_base_urls: BTreeMap::new(),
            credential: CredentialReference {
                reference: "secret://doubao-speech/main".to_owned(),
            },
            credential_kind: None,
            provider_rules_id: Some(DOUBAO_SPEECH_PROFILE_ID.to_owned()),
            region: None,
            workspace: None,
            account: None,
            request_timeout: Duration::from_secs(120),
            auto_sync_models: true,
            instance_rules: enabled.map(|enabled| buckyos_api::ProviderInstanceRules {
                enabled_inventory_models: Some(enabled),
                ..Default::default()
            }),
        };
        let empty = || ProviderDiscoverySnapshot {
            revision: Some("fixture-v1".to_owned()),
            discovered_at_ms: 1,
            health: ProviderHealthState::Healthy,
            models: Vec::new(),
        };
        let build = |enabled: Option<BTreeSet<String>>| {
            InventoryBuilder::build(
                &provider.profile,
                &instance(enabled),
                empty(),
                &catalog,
                &codecs,
            )
            .unwrap()
        };

        // Declared static models are published from the wire-free catalog snapshot.
        let inventory = build(None);
        assert_eq!(
            inventory
                .models
                .iter()
                .map(|model| model.provider_model_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "doubao-seed-asr-2.0",
                "doubao-seed-asr-2.0-fast",
                "doubao-seed-tts-2.0"
            ]
        );
        for model in &inventory.models {
            match model.provider_model_id.as_str() {
                "doubao-seed-tts-2.0" => {
                    assert!(model.api_types.contains(&ApiType::AudioTextToSpeech));
                    assert_eq!(model.operations["audio.tts"], "tts.unidirectional");
                    assert!(model
                        .logical_mounts
                        .iter()
                        .any(|mount| mount == "audio.tts"));
                }
                "doubao-seed-asr-2.0" => {
                    assert!(model.api_types.contains(&ApiType::AudioSpeechRecognition));
                    assert_eq!(model.operations["audio.asr"], "asr.recognize.submit");
                    assert!(model
                        .logical_mounts
                        .iter()
                        .any(|mount| mount == "audio.asr.doubao"));
                }
                "doubao-seed-asr-2.0-fast" => {
                    assert!(model.api_types.contains(&ApiType::AudioSpeechRecognition));
                    assert_eq!(model.operations["audio.asr"], "asr.recognize.flash");
                    assert!(model
                        .logical_mounts
                        .iter()
                        .any(|mount| mount == "audio.asr.doubao"));
                }
                other => panic!("unexpected doubao-speech static model `{other}`"),
            }
        }

        // The operator allowlist narrows the static catalog before publishing.
        let narrowed = build(Some(BTreeSet::from(["doubao-seed-tts-2.0".to_owned()])));
        assert_eq!(
            narrowed
                .models
                .iter()
                .map(|model| model.provider_model_id.as_str())
                .collect::<Vec<_>>(),
            vec!["doubao-seed-tts-2.0"]
        );

        // An empty allowlist keeps the profile configured but publishes no models.
        assert!(build(Some(BTreeSet::new())).models.is_empty());
    }
}

#[cfg(test)]
use crate::protocol::{DEEPSEEK_RESPONSES_ADAPTER_ID, OPENAI_RESPONSES_OPERATION_ID};
