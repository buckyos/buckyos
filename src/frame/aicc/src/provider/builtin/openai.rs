use super::super::{
    validate_discovery, CredentialDescriptor, DiscoveredModel, DiscoveryContext, DiscoveryMode,
    ModelAvailability, ProviderConnectionContract, ProviderConnectionInput, ProviderDiscovery,
    ProviderDiscoverySnapshot, ProviderError, ProviderFieldSchema, ProviderHealthState,
    ProviderProfile, ProviderResult, RefreshPolicy,
};
use crate::catalog::{
    CatalogKind, CurrentCatalogFile, KnownProvider, KnownProviderCatalog, ModelDriverCatalog,
    ProviderRulesCatalog,
};
use crate::protocol::{CredentialKind, HttpRequest, HttpResponse, HttpTransport};
use async_trait::async_trait;
use reqwest::header::ETAG;
use reqwest::{Method, Url};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;

pub(crate) const OPENAI_PROVIDER_PROFILE_ID: &str = "openai";

const OPENAI_KNOWN_PROVIDER: &[u8] =
    include_bytes!("../../../driver_metadata/known-providers/openai.known-provider.json");
const OPENAI_PROVIDER_RULES: &[u8] =
    include_bytes!("../../../driver_metadata/providers/openai.provider.json");
const OPENAI_MODEL_DRIVER: &[u8] =
    include_bytes!("../../../driver_metadata/models/openai.model.json");

const MODELS_RESPONSE_LIMIT: usize = 8 * 1024 * 1024;

pub(crate) fn openai_profile() -> ProviderProfile {
    let known = openai_known_provider();
    let credential: CredentialDeclaration = embedded_value(
        &known,
        "credential",
        "OpenAI Known Provider credential declaration",
    );
    assert!(credential.required && credential.secret);
    let credential = match credential.kind.as_str() {
        "bearer" => CredentialDescriptor {
            kind: CredentialKind::Bearer,
            header_name: None,
        },
        kind => panic!("OpenAI Known Provider uses unsupported credential kind `{kind}`"),
    };
    ProviderProfile {
        provider_profile_id: OPENAI_PROVIDER_PROFILE_ID.to_owned(),
        display_name: known.display_name,
        default_protocol_adapter_id: known.protocol_adapter_id,
        credential,
        discovery_mode: DiscoveryMode::MachineApi,
        refresh: RefreshPolicy::default(),
        default_inventory: None,
    }
}

pub(crate) fn openai_known_provider() -> KnownProvider {
    decode_catalog::<KnownProviderCatalog>(OPENAI_KNOWN_PROVIDER, "OpenAI Known Provider catalog")
        .providers
        .into_iter()
        .find(|provider| provider.provider_profile_id == OPENAI_PROVIDER_PROFILE_ID)
        .expect("OpenAI Known Provider catalog must contain the OpenAI profile")
}

pub(crate) fn openai_connection_contract() -> ProviderConnectionContract {
    let known = openai_known_provider();
    let fields: InstanceFieldDeclarations = embedded_value(
        &known,
        "instance_fields",
        "OpenAI Known Provider instance fields",
    );
    ProviderConnectionContract {
        default_base_url: known.base_url,
        region: fields.region,
        workspace: fields.workspace,
        account: fields.account,
    }
}

pub(crate) fn openai_provider_rules(_revision_seq: u64) -> ProviderRulesCatalog {
    decode_catalog(OPENAI_PROVIDER_RULES, "OpenAI Provider Rules catalog")
}

pub(crate) fn openai_model_driver() -> ModelDriverCatalog {
    decode_catalog(OPENAI_MODEL_DRIVER, "OpenAI Model Driver catalog")
}

pub(crate) fn openai_catalog_files() -> Vec<CurrentCatalogFile> {
    [
        (CatalogKind::KnownProvider, OPENAI_KNOWN_PROVIDER),
        (CatalogKind::ProviderRules, OPENAI_PROVIDER_RULES),
        (CatalogKind::ModelDriver, OPENAI_MODEL_DRIVER),
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

fn decode_catalog<T: DeserializeOwned>(contents: &[u8], label: &str) -> T {
    serde_json::from_slice(contents)
        .unwrap_or_else(|error| panic!("{label} configuration is invalid: {error}"))
}

#[async_trait]
trait OpenAiModelsTransport: Send + Sync {
    async fn send(
        &self,
        request: HttpRequest,
    ) -> crate::protocol::ProtocolResultValue<HttpResponse>;
}

#[async_trait]
impl OpenAiModelsTransport for HttpTransport {
    async fn send(
        &self,
        request: HttpRequest,
    ) -> crate::protocol::ProtocolResultValue<HttpResponse> {
        HttpTransport::send(self, request).await
    }
}

#[derive(Clone)]
pub(crate) struct OpenAiDiscovery {
    transport: Arc<dyn OpenAiModelsTransport>,
}

impl OpenAiDiscovery {
    pub(crate) fn new(transport: HttpTransport) -> Self {
        Self {
            transport: Arc::new(transport),
        }
    }

    #[cfg(test)]
    fn with_transport(transport: Arc<dyn OpenAiModelsTransport>) -> Self {
        Self { transport }
    }
}

#[async_trait]
impl ProviderDiscovery for OpenAiDiscovery {
    async fn discover(
        &self,
        context: &DiscoveryContext<'_>,
    ) -> ProviderResult<ProviderDiscoverySnapshot> {
        validate_openai_context(context)?;
        let mut request =
            HttpRequest::new(Method::GET, models_endpoint(&context.instance.base_url)?);
        context
            .credential
            .apply(&mut request.headers)
            .map_err(|error| ProviderError::Credential(error.to_string()))?;
        request.timeout = Some(Duration::from_secs(30));
        request.max_response_bytes = Some(MODELS_RESPONSE_LIMIT);

        let response = self
            .transport
            .send(request)
            .await
            .map_err(|error| ProviderError::Discovery(error.to_string()))?;
        ensure_success(&response)?;
        let revision = response
            .headers
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let wire: ModelsResponse = serde_json::from_slice(&response.body).map_err(|error| {
            ProviderError::Discovery(format!("OpenAI models response is invalid: {error}"))
        })?;
        if wire.object != "list" {
            return Err(ProviderError::Discovery(
                "OpenAI models response must be a list".to_owned(),
            ));
        }
        let snapshot = ProviderDiscoverySnapshot {
            revision,
            discovered_at_ms: super::super::now_ms()?,
            health: ProviderHealthState::Healthy,
            models: wire
                .data
                .into_iter()
                .map(|model| DiscoveredModel {
                    provider_model_id: model.id,
                    origin_model_id: None,
                    api_types: None,
                    supported_features: None,
                    remote_methods: None,
                    availability: ModelAvailability::Available,
                    deprecated: false,
                    pricing: None,
                })
                .collect(),
        };
        validate_discovery(&snapshot)?;
        Ok(snapshot)
    }
}

fn validate_openai_context(context: &DiscoveryContext<'_>) -> ProviderResult<()> {
    if context.profile.provider_profile_id != OPENAI_PROVIDER_PROFILE_ID
        || context.instance.provider_profile_id != OPENAI_PROVIDER_PROFILE_ID
        || context.instance.protocol_adapter_id != context.profile.default_protocol_adapter_id
    {
        return Err(ProviderError::InvalidConfiguration(
            "OpenAI discovery requires the OpenAI profile and Responses adapter".to_owned(),
        ));
    }
    if context.credential.audit().kind != CredentialKind::Bearer {
        return Err(ProviderError::Credential(
            "OpenAI discovery requires a Bearer credential".to_owned(),
        ));
    }
    openai_connection_contract().resolve(ProviderConnectionInput {
        base_url: Some(&context.instance.base_url),
        region: context.instance.region.as_deref(),
        account: context.instance.account.as_deref(),
        ..ProviderConnectionInput::default()
    })?;
    Ok(())
}

fn models_endpoint(base_url: &str) -> ProviderResult<String> {
    let mut url = Url::parse(base_url).map_err(|_| {
        ProviderError::InvalidConfiguration("OpenAI base_url is invalid".to_owned())
    })?;
    if !matches!(url.scheme(), "http" | "https") || url.cannot_be_a_base() {
        return Err(ProviderError::InvalidConfiguration(
            "OpenAI base_url must be an absolute HTTP URL".to_owned(),
        ));
    }
    let base_path = url.path().trim_end_matches('/');
    let prefix = if base_path.is_empty() {
        "/v1"
    } else {
        base_path
    };
    url.set_path(&format!("{prefix}/models"));
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string())
}

fn ensure_success(response: &HttpResponse) -> ProviderResult<()> {
    if response.status.is_success() {
        return Ok(());
    }
    let message = serde_json::from_slice::<OpenAiErrorResponse>(&response.body)
        .ok()
        .and_then(|body| body.error)
        .and_then(|error| error.message)
        .unwrap_or_else(|| {
            response
                .status
                .canonical_reason()
                .unwrap_or("request failed")
                .to_owned()
        });
    Err(ProviderError::Discovery(format!(
        "OpenAI models request failed with status {} (request {}): {message}",
        response.status, response.request_id
    )))
}

#[derive(Deserialize)]
struct ModelsResponse {
    object: String,
    data: Vec<ModelObject>,
}

#[derive(Deserialize)]
struct ModelObject {
    id: String,
}

#[derive(Deserialize)]
struct OpenAiErrorResponse {
    error: Option<OpenAiError>,
}

#[derive(Deserialize)]
struct OpenAiError {
    message: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CatalogBuildOptions, CatalogSnapshot};
    use crate::protocol::{
        openai_responses_adapter, CodecRegistry, ProtocolError, OPENAI_AUDIO_SPEECH_OPERATION_ID,
        OPENAI_AUDIO_TRANSCRIPTIONS_OPERATION_ID, OPENAI_EMBEDDINGS_OPERATION_ID,
        OPENAI_IMAGES_EDIT_OPERATION_ID, OPENAI_IMAGES_GENERATE_OPERATION_ID,
        OPENAI_RESPONSES_ADAPTER_ID, OPENAI_RESPONSES_OPERATION_ID, OPENAI_VIDEOS_OPERATION_ID,
    };
    use crate::protocol::{HttpBody, ResolvedCredential};
    use crate::provider::{CredentialReference, InventoryBuilder, ProviderInstanceConfig};
    use crate::settings::{MetadataFile, MetadataSource, MetadataSources};
    use bytes::Bytes;
    use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
    use reqwest::StatusCode;
    use serde_json::{json, Value};
    use std::sync::Mutex;

    struct FakeTransport {
        response: Mutex<Option<Result<HttpResponse, ProtocolError>>>,
        request: Mutex<Option<HttpRequest>>,
    }

    impl FakeTransport {
        fn response(status: StatusCode, headers: HeaderMap, body: Value) -> Arc<Self> {
            Arc::new(Self {
                response: Mutex::new(Some(Ok(HttpResponse {
                    status,
                    headers,
                    body: Bytes::from(serde_json::to_vec(&body).unwrap()),
                    request_id: "request-1".to_owned(),
                    retry_after: None,
                }))),
                request: Mutex::new(None),
            })
        }
    }

    #[async_trait]
    impl OpenAiModelsTransport for FakeTransport {
        async fn send(
            &self,
            request: HttpRequest,
        ) -> crate::protocol::ProtocolResultValue<HttpResponse> {
            *self.request.lock().unwrap() = Some(request);
            self.response.lock().unwrap().take().unwrap()
        }
    }

    fn instance() -> ProviderInstanceConfig {
        let known = openai_known_provider();
        ProviderInstanceConfig {
            provider_instance_name: "openai-main".to_owned(),
            provider_profile_id: OPENAI_PROVIDER_PROFILE_ID.to_owned(),
            protocol_adapter_id: known.protocol_adapter_id,
            base_url: known.base_url,
            credential: CredentialReference {
                reference: "secret://openai/main".to_owned(),
            },
            provider_rules_id: Some(OPENAI_PROVIDER_PROFILE_ID.to_owned()),
            region: None,
            workspace: None,
            account: None,
        }
    }

    fn context<'a>(
        profile: &'a ProviderProfile,
        instance: &'a ProviderInstanceConfig,
        credential: &'a ResolvedCredential,
    ) -> DiscoveryContext<'a> {
        DiscoveryContext {
            profile,
            instance,
            credential,
        }
    }

    fn configured_catalog() -> Arc<CatalogSnapshot> {
        let builtin = openai_catalog_files()
            .into_iter()
            .map(|file| {
                MetadataFile::parse(MetadataSource::Builtin, file.kind, file.contents).unwrap()
            })
            .collect();
        MetadataSources {
            builtin,
            ..MetadataSources::default()
        }
        .build_snapshot(1, &CatalogBuildOptions::default())
        .unwrap()
    }

    #[test]
    fn builtin_identity_and_catalog_fixtures_are_stable() {
        let profile = openai_profile();
        let known = openai_known_provider();
        let rules = openai_provider_rules(4);
        let models = openai_model_driver();

        assert_eq!(profile.provider_profile_id, "openai");
        assert_eq!(profile.default_protocol_adapter_id, "openai-responses");
        assert_eq!(profile.credential.kind, CredentialKind::Bearer);
        assert_eq!(profile.discovery_mode, DiscoveryMode::MachineApi);
        assert_eq!(known.base_url, "https://api.openai.com/v1");
        assert_eq!(known.provider_rules_id.as_deref(), Some("openai"));
        assert_eq!(
            known.ui_hints["instance_fields"]["region"]["mode"],
            "unsupported"
        );
        assert_eq!(rules.revision_seq, 1);
        assert_eq!(rules.metadata_drivers, Some(vec!["openai".to_owned()]));
        assert_eq!(
            rules.patterns[0].operations["image.txt2img"],
            OPENAI_IMAGES_GENERATE_OPERATION_ID
        );
        assert_eq!(
            rules.patterns[0].operations["image.img2img"],
            OPENAI_IMAGES_EDIT_OPERATION_ID
        );
        assert_eq!(
            rules.patterns[0].operations["video.txt2video"],
            OPENAI_VIDEOS_OPERATION_ID
        );
        assert_eq!(
            rules.patterns[0].operations["embedding.text"],
            OPENAI_EMBEDDINGS_OPERATION_ID
        );
        assert_eq!(
            rules.patterns[0].operations["audio.tts"],
            OPENAI_AUDIO_SPEECH_OPERATION_ID
        );
        assert_eq!(
            rules.patterns[0].operations["audio.asr"],
            OPENAI_AUDIO_TRANSCRIPTIONS_OPERATION_ID
        );
        assert_eq!(models.model_driver_id, "openai");
        let sol = models
            .patterns
            .iter()
            .find(|rule| {
                rule.match_rule == crate::matching::MatchRule::Shorthand("gpt-5.6-sol*".into())
            })
            .unwrap();
        assert_eq!(
            sol.capabilities.as_ref().unwrap()["max_context_tokens"],
            1_050_000
        );
        assert_eq!(sol.pricing.as_ref().unwrap().input_token, Some(0.000004));
        assert_eq!(models.variants.len(), 6);
    }

    #[test]
    fn embedded_catalogs_build_through_the_builtin_metadata_source() {
        let catalog = configured_catalog();

        assert_eq!(
            catalog.known_provider("openai").unwrap().display_name,
            "OpenAI"
        );
        assert_eq!(catalog.provider_rules("openai").unwrap().revision_seq, 1);
        assert_eq!(catalog.model_driver("openai").unwrap().revision_seq, 1);
    }

    #[tokio::test]
    async fn discovers_official_models_with_bearer_auth_and_etag() {
        let mut headers = HeaderMap::new();
        headers.insert(ETAG, HeaderValue::from_static("models-v1"));
        let transport = FakeTransport::response(
            StatusCode::OK,
            headers,
            json!({
                "object": "list",
                "data": [
                    {"id": "gpt-5", "object": "model", "owned_by": "openai"},
                    {"id": "text-embedding-3-small", "object": "model", "owned_by": "openai"}
                ]
            }),
        );
        let discovery = OpenAiDiscovery::with_transport(transport.clone());
        let profile = openai_profile();
        let instance = instance();
        let credential = ResolvedCredential::bearer("secret://openai/main", "secret").unwrap();

        let snapshot = discovery
            .discover(&context(&profile, &instance, &credential))
            .await
            .unwrap();

        assert_eq!(snapshot.revision.as_deref(), Some("models-v1"));
        assert_eq!(snapshot.health, ProviderHealthState::Healthy);
        assert_eq!(snapshot.models.len(), 2);
        assert_eq!(snapshot.models[0].provider_model_id, "gpt-5");
        let request = transport.request.lock().unwrap();
        let request = request.as_ref().unwrap();
        assert_eq!(request.method, Method::GET);
        assert_eq!(request.url, "https://api.openai.com/v1/models");
        assert!(matches!(request.body, HttpBody::Empty));
        assert_eq!(request.max_response_bytes, Some(MODELS_RESPONSE_LIMIT));
        assert_eq!(request.headers[AUTHORIZATION], "Bearer secret");
        assert!(!format!("{request:?}").contains("secret"));
    }

    #[tokio::test]
    async fn rejects_wrong_profile_fields_and_official_errors() {
        let transport = FakeTransport::response(
            StatusCode::UNAUTHORIZED,
            HeaderMap::new(),
            json!({"error": {"message": "invalid API key"}}),
        );
        let discovery = OpenAiDiscovery::with_transport(transport);
        let profile = openai_profile();
        let mut instance = instance();
        let credential = ResolvedCredential::bearer("secret://openai/main", "secret").unwrap();

        instance.region = Some("us".to_owned());
        let error = discovery
            .discover(&context(&profile, &instance, &credential))
            .await
            .unwrap_err();
        assert!(matches!(error, ProviderError::InvalidConfiguration(_)));

        instance.region = None;
        let error = discovery
            .discover(&context(&profile, &instance, &credential))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("status 401"));
        assert!(error.to_string().contains("request request-1"));
        assert!(error.to_string().contains("invalid API key"));
    }

    #[test]
    fn configured_model_and_rules_build_inventory_without_a_dialect() {
        let catalog = configured_catalog();
        let (adapter, codecs) = openai_responses_adapter();
        let mut registry = CodecRegistry::default();
        registry.register_codecs(adapter, codecs).unwrap();
        let inventory = InventoryBuilder::build(
            &openai_profile(),
            &instance(),
            ProviderDiscoverySnapshot {
                revision: Some("models-v1".to_owned()),
                discovered_at_ms: 1,
                health: ProviderHealthState::Healthy,
                models: vec![DiscoveredModel {
                    provider_model_id: "gpt-5.6-sol".to_owned(),
                    origin_model_id: None,
                    api_types: None,
                    supported_features: None,
                    remote_methods: None,
                    availability: ModelAvailability::Available,
                    deprecated: false,
                    pricing: None,
                }],
            },
            &catalog,
            &registry,
        )
        .unwrap();

        assert_eq!(inventory.protocol_adapter_id, OPENAI_RESPONSES_ADAPTER_ID);
        assert_eq!(inventory.models.len(), 1);
        let model = &inventory.models[0];
        let operations = &model.operations;
        assert_eq!(operations["llm"], OPENAI_RESPONSES_OPERATION_ID);
        assert_eq!(
            operations["image.txt2img"],
            OPENAI_IMAGES_GENERATE_OPERATION_ID
        );
        assert_eq!(operations["image.img2img"], OPENAI_IMAGES_EDIT_OPERATION_ID);
        assert_eq!(model.capabilities["tool_call"], true);
        assert_eq!(model.capabilities["json_schema"], true);
        assert_eq!(model.capabilities["max_context_tokens"], 1_050_000);
    }
}
