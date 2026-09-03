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
use crate::protocol::{
    CredentialKind, HttpRequest, HttpResponse, HttpTransport, GEMINI_ADAPTER_ID,
};
use async_trait::async_trait;
use reqwest::{Method, Url};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

pub(crate) const GEMINI_PROVIDER_PROFILE_ID: &str = "gemini";
pub(crate) const GEMINI_CREDENTIAL_HEADER: &str = "x-goog-api-key";

const GEMINI_PROVIDER_RULES: &[u8] =
    include_bytes!("../../../driver_metadata/providers/gemini.provider.json");
const GEMINI_KNOWN_PROVIDER: &[u8] =
    include_bytes!("../../../driver_metadata/known-providers/gemini.known-provider.json");
const GEMINI_MODEL_DRIVER: &[u8] =
    include_bytes!("../../../driver_metadata/models/gemini.model.json");

const MODELS_RESPONSE_LIMIT: usize = 8 * 1024 * 1024;
const DISCOVERY_PAGE_SIZE: usize = 1000;
const MAX_DISCOVERY_PAGES: usize = 100;

pub(crate) fn gemini_profile() -> ProviderProfile {
    let known = gemini_known_provider();
    let credential: CredentialDeclaration = embedded_value(
        &known,
        "credential",
        "Gemini Known Provider credential declaration",
    );
    assert_eq!(credential.kind, "named_header");
    assert!(credential.required && credential.secret);
    ProviderProfile {
        provider_profile_id: GEMINI_PROVIDER_PROFILE_ID.to_owned(),
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

pub(crate) fn gemini_connection_contract() -> ProviderConnectionContract {
    let known = gemini_known_provider();
    let fields: InstanceFieldDeclarations = embedded_value(
        &known,
        "instance_fields",
        "Gemini Known Provider instance fields",
    );
    ProviderConnectionContract {
        default_base_url: known.base_url,
        region: fields.region,
        workspace: fields.workspace,
        account: fields.account,
    }
}

pub(crate) fn gemini_known_provider() -> KnownProvider {
    embedded_json::<KnownProviderCatalog>(
        GEMINI_KNOWN_PROVIDER,
        "Gemini Known Provider catalog",
    )
    .providers
    .into_iter()
    .find(|provider| provider.provider_profile_id == GEMINI_PROVIDER_PROFILE_ID)
    .expect("Gemini Known Provider catalog must contain the Gemini profile")
}

pub(crate) fn gemini_provider_rules(_revision_seq: u64) -> ProviderRulesCatalog {
    embedded_json(GEMINI_PROVIDER_RULES, "Gemini Provider Rules catalog")
}

pub(crate) fn gemini_model_driver() -> ModelDriverCatalog {
    embedded_json(GEMINI_MODEL_DRIVER, "Gemini Model Driver catalog")
}

pub(crate) fn gemini_catalog_files() -> Vec<CurrentCatalogFile> {
    [
        (CatalogKind::KnownProvider, GEMINI_KNOWN_PROVIDER),
        (CatalogKind::ProviderRules, GEMINI_PROVIDER_RULES),
        (CatalogKind::ModelDriver, GEMINI_MODEL_DRIVER),
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

#[async_trait]
trait GeminiModelsTransport: Send + Sync {
    async fn send(
        &self,
        request: HttpRequest,
    ) -> crate::protocol::ProtocolResultValue<HttpResponse>;
}

#[async_trait]
impl GeminiModelsTransport for HttpTransport {
    async fn send(
        &self,
        request: HttpRequest,
    ) -> crate::protocol::ProtocolResultValue<HttpResponse> {
        HttpTransport::send(self, request).await
    }
}

#[derive(Clone)]
pub(crate) struct GeminiDiscovery {
    transport: Arc<dyn GeminiModelsTransport>,
}

impl GeminiDiscovery {
    pub(crate) fn new(transport: HttpTransport) -> Self {
        Self {
            transport: Arc::new(transport),
        }
    }

    #[cfg(test)]
    fn with_transport(transport: Arc<dyn GeminiModelsTransport>) -> Self {
        Self { transport }
    }
}

#[async_trait]
impl ProviderDiscovery for GeminiDiscovery {
    async fn discover(
        &self,
        context: &DiscoveryContext<'_>,
    ) -> ProviderResult<ProviderDiscoverySnapshot> {
        validate_gemini_context(context)?;
        let mut models = BTreeMap::<String, DiscoveredModel>::new();
        let mut page_token = None;
        let mut seen_page_tokens = BTreeSet::new();

        for _ in 0..MAX_DISCOVERY_PAGES {
            let mut request = HttpRequest::new(
                Method::GET,
                models_endpoint(&context.instance.base_url, page_token.as_deref())?,
            );
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
            let page: ModelsResponse = serde_json::from_slice(&response.body).map_err(|error| {
                ProviderError::Discovery(format!("Gemini models response is invalid: {error}"))
            })?;
            for model in page.models {
                let model = model.into_discovered()?;
                if models
                    .insert(model.provider_model_id.clone(), model)
                    .is_some()
                {
                    return Err(ProviderError::Discovery(
                        "Gemini Models API returned a duplicate model".to_owned(),
                    ));
                }
            }
            let Some(next_page_token) = page.next_page_token.filter(|token| !token.is_empty())
            else {
                let models = models.into_values().collect::<Vec<_>>();
                let snapshot = ProviderDiscoverySnapshot {
                    revision: Some(models_revision(&models)),
                    discovered_at_ms: super::super::now_ms()?,
                    health: ProviderHealthState::Healthy,
                    models,
                };
                validate_discovery(&snapshot)?;
                return Ok(snapshot);
            };
            if !seen_page_tokens.insert(next_page_token.clone()) {
                return Err(ProviderError::Discovery(
                    "Gemini Models API repeated a page token".to_owned(),
                ));
            }
            page_token = Some(next_page_token);
        }
        Err(ProviderError::Discovery(
            "Gemini Models API exceeded the pagination limit".to_owned(),
        ))
    }
}

fn validate_gemini_context(context: &DiscoveryContext<'_>) -> ProviderResult<()> {
    if context.profile.provider_profile_id != GEMINI_PROVIDER_PROFILE_ID
        || context.profile.default_protocol_adapter_id != GEMINI_ADAPTER_ID
        || context.instance.provider_profile_id != GEMINI_PROVIDER_PROFILE_ID
        || context.instance.protocol_adapter_id != GEMINI_ADAPTER_ID
    {
        return Err(ProviderError::InvalidConfiguration(
            "Gemini discovery requires the Gemini profile and Interactions adapter".to_owned(),
        ));
    }
    if context.credential.audit().kind != CredentialKind::NamedHeader {
        return Err(ProviderError::Credential(
            "Gemini discovery requires a named-header credential".to_owned(),
        ));
    }
    gemini_connection_contract().resolve(ProviderConnectionInput {
        base_url: Some(&context.instance.base_url),
        region: context.instance.region.as_deref(),
        workspace: None,
        account: context.instance.account.as_deref(),
    })?;
    Ok(())
}

fn models_endpoint(base_url: &str, page_token: Option<&str>) -> ProviderResult<String> {
    let mut url = Url::parse(base_url).map_err(|_| {
        ProviderError::InvalidConfiguration("Gemini base_url is invalid".to_owned())
    })?;
    if !matches!(url.scheme(), "http" | "https") || url.cannot_be_a_base() {
        return Err(ProviderError::InvalidConfiguration(
            "Gemini base_url must be an absolute HTTP URL".to_owned(),
        ));
    }
    let base_path = url.path().trim_end_matches('/');
    url.set_path(&format!("{base_path}/models"));
    url.set_query(None);
    url.set_fragment(None);
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("pageSize", &DISCOVERY_PAGE_SIZE.to_string());
        if let Some(page_token) = page_token {
            query.append_pair("pageToken", page_token);
        }
    }
    Ok(url.to_string())
}

fn ensure_success(response: &HttpResponse) -> ProviderResult<()> {
    if response.status.is_success() {
        return Ok(());
    }
    let message = serde_json::from_slice::<GeminiErrorResponse>(&response.body)
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
        "Gemini models request failed with status {} (request {}): {message}",
        response.status, response.request_id
    )))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelsResponse {
    #[serde(default)]
    models: Vec<ModelObject>,
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelObject {
    name: String,
    base_model_id: Option<String>,
}

impl ModelObject {
    fn into_discovered(self) -> ProviderResult<DiscoveredModel> {
        let provider_model_id = model_id(&self.name)?;
        let origin_model_id = match self.base_model_id {
            Some(value) if !value.trim().is_empty() => Some(model_id(&value)?),
            _ => None,
        };
        Ok(DiscoveredModel {
            provider_model_id,
            origin_model_id,
            api_types: None,
            supported_features: None,
            remote_methods: None,
            availability: ModelAvailability::Available,
            deprecated: false,
            pricing: None,
        })
    }
}

fn model_id(resource_name: &str) -> ProviderResult<String> {
    let id = resource_name
        .strip_prefix("models/")
        .unwrap_or(resource_name);
    if id.is_empty() || id.contains('/') || id.contains('@') {
        return Err(ProviderError::Discovery(
            "Gemini model name is not a valid models/* resource".to_owned(),
        ));
    }
    Ok(id.to_owned())
}

fn models_revision(models: &[DiscoveredModel]) -> String {
    let mut hasher = Sha256::new();
    for model in models {
        hasher.update((model.provider_model_id.len() as u64).to_be_bytes());
        hasher.update(model.provider_model_id.as_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}

#[derive(Deserialize)]
struct GeminiErrorResponse {
    error: Option<GeminiError>,
}

#[derive(Deserialize)]
struct GeminiError {
    message: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CatalogBuildOptions, CatalogSnapshot};
    use crate::protocol::{
        gemini_api_key, gemini_interactions_adapter, CodecRegistry, HttpBody, ProtocolError,
        ResolvedCredential, GEMINI_EMBED_CONTENT_OPERATION_ID, GEMINI_INTERACTIONS_OPERATION_ID,
        GEMINI_PREDICT_LONG_RUNNING_OPERATION_ID,
    };
    use crate::provider::{CredentialReference, InventoryBuilder, ProviderInstanceConfig};
    use bytes::Bytes;
    use reqwest::header::HeaderMap;
    use reqwest::StatusCode;
    use serde_json::{json, Value};
    use std::collections::VecDeque;
    use std::sync::Mutex;

    struct FakeTransport {
        responses: Mutex<VecDeque<Result<HttpResponse, ProtocolError>>>,
        requests: Mutex<Vec<HttpRequest>>,
    }

    impl FakeTransport {
        fn responses(bodies: impl IntoIterator<Item = Value>) -> Arc<Self> {
            Arc::new(Self {
                responses: Mutex::new(
                    bodies
                        .into_iter()
                        .map(|body| {
                            Ok(HttpResponse {
                                status: StatusCode::OK,
                                headers: HeaderMap::new(),
                                body: Bytes::from(serde_json::to_vec(&body).unwrap()),
                                request_id: "request-1".to_owned(),
                                retry_after: None,
                            })
                        })
                        .collect(),
                ),
                requests: Mutex::new(Vec::new()),
            })
        }

        fn error(status: StatusCode, body: Value) -> Arc<Self> {
            Arc::new(Self {
                responses: Mutex::new(VecDeque::from([Ok(HttpResponse {
                    status,
                    headers: HeaderMap::new(),
                    body: Bytes::from(serde_json::to_vec(&body).unwrap()),
                    request_id: "request-error".to_owned(),
                    retry_after: None,
                })])),
                requests: Mutex::new(Vec::new()),
            })
        }
    }

    #[async_trait]
    impl GeminiModelsTransport for FakeTransport {
        async fn send(
            &self,
            request: HttpRequest,
        ) -> crate::protocol::ProtocolResultValue<HttpResponse> {
            self.requests.lock().unwrap().push(request);
            self.responses.lock().unwrap().pop_front().unwrap()
        }
    }

    fn instance() -> ProviderInstanceConfig {
        let known = gemini_known_provider();
        ProviderInstanceConfig {
            provider_instance_name: "google-gemini-main".to_owned(),
            provider_profile_id: GEMINI_PROVIDER_PROFILE_ID.to_owned(),
            protocol_adapter_id: GEMINI_ADAPTER_ID.to_owned(),
            base_url: known.base_url,
            credential: CredentialReference {
                reference: "secret://gemini/main".to_owned(),
            },
            provider_rules_id: Some(GEMINI_PROVIDER_PROFILE_ID.to_owned()),
            region: None,
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

    #[test]
    fn builtin_identity_and_catalog_fixtures_are_stable() {
        let profile = gemini_profile();
        let known = gemini_known_provider();
        let rules = gemini_provider_rules(4);

        assert_eq!(profile.provider_profile_id, "gemini");
        assert_eq!(profile.default_protocol_adapter_id, "gemini-interactions");
        assert_eq!(profile.credential.kind, CredentialKind::NamedHeader);
        assert_eq!(
            profile.credential.header_name.as_deref(),
            Some("x-goog-api-key")
        );
        assert_eq!(
            known.base_url,
            "https://generativelanguage.googleapis.com/v1beta"
        );
        assert_eq!(
            gemini_connection_contract()
                .resolve(ProviderConnectionInput::default())
                .unwrap()
                .base_url,
            "https://generativelanguage.googleapis.com/v1beta"
        );
        assert!(gemini_connection_contract()
            .resolve(ProviderConnectionInput {
                workspace: Some("project"),
                ..ProviderConnectionInput::default()
            })
            .is_err());
        assert_eq!(
            known.ui_hints["instance_fields"],
            json!({
                "region": {"mode": "unsupported"},
                "workspace": {"mode": "unsupported"},
                "account": {"mode": "unsupported"}
            })
        );
        assert_eq!(rules.metadata_drivers, Some(vec!["gemini".to_owned()]));
        assert_eq!(
            rules.patterns[0].operations["embedding.multimodal"],
            GEMINI_EMBED_CONTENT_OPERATION_ID
        );
        assert_eq!(
            rules.patterns[0].operations["video.extend"],
            GEMINI_PREDICT_LONG_RUNNING_OPERATION_ID
        );
    }

    #[tokio::test]
    async fn discovers_and_paginates_with_named_header_auth() {
        let transport = FakeTransport::responses([
            json!({
                "models": [{
                    "name": "models/gemini-z",
                    "baseModelId": "gemini-z",
                    "supportedGenerationMethods": ["generateContent", "countTokens"]
                }],
                "nextPageToken": "next token"
            }),
            json!({
                "models": [{
                    "name": "models/gemini-a",
                    "supportedGenerationMethods": ["embedContent", "predictLongRunning"]
                }]
            }),
        ]);
        let discovery = GeminiDiscovery::with_transport(transport.clone());
        let profile = gemini_profile();
        let instance = instance();
        let credential = gemini_api_key("secret://gemini/main", "secret").unwrap();
        let snapshot = discovery
            .discover(&context(&profile, &instance, &credential))
            .await
            .unwrap();

        assert_eq!(snapshot.health, ProviderHealthState::Healthy);
        assert_eq!(
            snapshot
                .models
                .iter()
                .map(|model| model.provider_model_id.as_str())
                .collect::<Vec<_>>(),
            vec!["gemini-a", "gemini-z"]
        );
        assert!(snapshot.revision.unwrap().starts_with("sha256:"));
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[0].url,
            "https://generativelanguage.googleapis.com/v1beta/models?pageSize=1000"
        );
        assert!(requests[1]
            .url
            .ends_with("pageSize=1000&pageToken=next+token"));
        assert_eq!(requests[0].headers[GEMINI_CREDENTIAL_HEADER], "secret");
        assert!(matches!(requests[0].body, HttpBody::Empty));
        assert!(!format!("{:?}", requests[0]).contains("secret"));
        assert!(snapshot.models[0].remote_methods.is_none());
    }

    #[tokio::test]
    async fn rejects_wrong_instance_fields_and_repeated_page_tokens() {
        let transport = FakeTransport::responses([
            json!({"models": [], "nextPageToken": "same"}),
            json!({"models": [], "nextPageToken": "same"}),
        ]);
        let discovery = GeminiDiscovery::with_transport(transport);
        let profile = gemini_profile();
        let mut instance = instance();
        let credential = gemini_api_key("secret://gemini/main", "secret").unwrap();

        instance.region = Some("us-central1".to_owned());
        assert!(matches!(
            discovery
                .discover(&context(&profile, &instance, &credential))
                .await,
            Err(ProviderError::InvalidConfiguration(_))
        ));
        instance.region = None;
        assert!(discovery
            .discover(&context(&profile, &instance, &credential))
            .await
            .unwrap_err()
            .to_string()
            .contains("repeated a page token"));
    }

    #[tokio::test]
    async fn rejects_provider_errors_and_invalid_model_resources() {
        let transport = FakeTransport::error(
            StatusCode::UNAUTHORIZED,
            json!({"error": {"message": "invalid API key"}}),
        );
        let discovery = GeminiDiscovery::with_transport(transport);
        let profile = gemini_profile();
        let instance = instance();
        let credential = gemini_api_key("secret://gemini/main", "secret").unwrap();
        let error = discovery
            .discover(&context(&profile, &instance, &credential))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("status 401"));
        assert!(error.contains("request request-error"));
        assert!(error.contains("invalid API key"));
        assert!(!error.contains("secret"));

        let model: ModelObject = serde_json::from_value(json!({
            "name": "publishers/google/models/gemini-test",
            "supportedGenerationMethods": ["generateContent"]
        }))
        .unwrap();
        assert!(matches!(
            model.into_discovered(),
            Err(ProviderError::Discovery(_))
        ));
    }

    #[test]
    fn rules_bind_inventory_to_the_gemini_base_adapter() {
        let catalog = CatalogSnapshot::from_current_files(
            1,
            gemini_catalog_files(),
            &CatalogBuildOptions::default(),
        )
        .unwrap();
        let (adapter, codecs) = gemini_interactions_adapter();
        let mut registry = CodecRegistry::default();
        registry.register_codecs(adapter, codecs).unwrap();
        let inventory = InventoryBuilder::build(
            &gemini_profile(),
            &instance(),
            ProviderDiscoverySnapshot {
                revision: Some("models-v1".to_owned()),
                discovered_at_ms: 1,
                health: ProviderHealthState::Healthy,
                models: vec![DiscoveredModel {
                    provider_model_id: "gemini-3.8-flash".to_owned(),
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

        let operations = &inventory.models[0].operations;
        assert_eq!(operations["llm"], GEMINI_INTERACTIONS_OPERATION_ID);
        assert!(!operations.contains_key("embedding.text"));
        assert!(!operations.contains_key("video.txt2video"));
    }
}
