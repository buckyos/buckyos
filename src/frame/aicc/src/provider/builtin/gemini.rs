use super::super::{
    validate_discovery, CredentialDescriptor, DiscoveredModel, DiscoveryContext, DiscoveryMode,
    ModelAvailability, ProviderConnectionContract, ProviderConnectionInput, ProviderDiscovery,
    ProviderDiscoverySnapshot, ProviderError, ProviderFieldSchema, ProviderHealthState,
    ProviderProfile, ProviderResult, RefreshPolicy,
};
use crate::catalog::{KnownProvider, ProviderPatternRule, ProviderRulesCatalog};
use crate::matching::MatchRule;
use crate::protocol::{
    CredentialKind, HttpRequest, HttpResponse, HttpTransport, GEMINI_ADAPTER_ID,
    GEMINI_EMBED_CONTENT_OPERATION_ID, GEMINI_INTERACTIONS_OPERATION_ID,
    GEMINI_PREDICT_LONG_RUNNING_OPERATION_ID,
};
use async_trait::async_trait;
use reqwest::{Method, Url};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

pub(crate) const GEMINI_PROVIDER_PROFILE_ID: &str = "gemini";
pub(crate) const GEMINI_DISPLAY_NAME: &str = "Google Gemini";
pub(crate) const GEMINI_DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";
pub(crate) const GEMINI_CREDENTIAL_HEADER: &str = "x-goog-api-key";

const MODELS_RESPONSE_LIMIT: usize = 8 * 1024 * 1024;
const DISCOVERY_PAGE_SIZE: usize = 1000;
const MAX_DISCOVERY_PAGES: usize = 100;

pub(crate) fn gemini_profile() -> ProviderProfile {
    ProviderProfile {
        provider_profile_id: GEMINI_PROVIDER_PROFILE_ID.to_owned(),
        display_name: GEMINI_DISPLAY_NAME.to_owned(),
        default_protocol_adapter_id: GEMINI_ADAPTER_ID.to_owned(),
        credential: CredentialDescriptor {
            kind: CredentialKind::NamedHeader,
            header_name: Some(GEMINI_CREDENTIAL_HEADER.to_owned()),
        },
        discovery_mode: DiscoveryMode::MachineApi,
        refresh: RefreshPolicy::default(),
        default_inventory: None,
    }
}

pub(crate) fn gemini_connection_contract() -> ProviderConnectionContract {
    ProviderConnectionContract {
        default_base_url: GEMINI_DEFAULT_BASE_URL.to_owned(),
        region: ProviderFieldSchema::unsupported(),
        workspace: ProviderFieldSchema::unsupported(),
        account: ProviderFieldSchema::unsupported(),
    }
}

pub(crate) fn gemini_known_provider() -> KnownProvider {
    KnownProvider {
        provider_profile_id: GEMINI_PROVIDER_PROFILE_ID.to_owned(),
        display_name: GEMINI_DISPLAY_NAME.to_owned(),
        base_url: GEMINI_DEFAULT_BASE_URL.to_owned(),
        protocol_adapter_id: GEMINI_ADAPTER_ID.to_owned(),
        provider_rules_id: Some(GEMINI_PROVIDER_PROFILE_ID.to_owned()),
        ui_hints: BTreeMap::from([
            (
                "credential".to_owned(),
                json!({
                    "kind": "named_header",
                    "header_name": GEMINI_CREDENTIAL_HEADER,
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

pub(crate) fn gemini_provider_rules(revision_seq: u64) -> ProviderRulesCatalog {
    let interactions = GEMINI_INTERACTIONS_OPERATION_ID.to_owned();
    let embeddings = GEMINI_EMBED_CONTENT_OPERATION_ID.to_owned();
    let video = GEMINI_PREDICT_LONG_RUNNING_OPERATION_ID.to_owned();
    ProviderRulesCatalog {
        format: "buckyos.aicc.provider-rules-catalog".to_owned(),
        schema_version: 1,
        schema_revision: 0,
        revision_seq,
        provider_profile_id: GEMINI_PROVIDER_PROFILE_ID.to_owned(),
        metadata_drivers: Some(vec![GEMINI_PROVIDER_PROFILE_ID.to_owned()]),
        origin_provider_aliases: BTreeMap::new(),
        origin_mappings: Vec::new(),
        models: Vec::new(),
        patterns: vec![ProviderPatternRule {
            match_rule: MatchRule::Shorthand("*".to_owned()),
            exclude: false,
            operations: BTreeMap::from([
                ("llm".to_owned(), interactions.clone()),
                ("vision.ocr".to_owned(), interactions.clone()),
                ("vision.caption".to_owned(), interactions.clone()),
                ("vision.detect".to_owned(), interactions.clone()),
                ("vision.segment".to_owned(), interactions.clone()),
                ("audio.asr".to_owned(), interactions.clone()),
                ("image.txt2img".to_owned(), interactions.clone()),
                ("image.img2img".to_owned(), interactions.clone()),
                ("audio.tts".to_owned(), interactions.clone()),
                ("audio.music".to_owned(), interactions),
                ("embedding.text".to_owned(), embeddings.clone()),
                ("embedding.multimodal".to_owned(), embeddings),
                ("video.txt2video".to_owned(), video.clone()),
                ("video.img2video".to_owned(), video.clone()),
                ("video.video2video".to_owned(), video.clone()),
                ("video.extend".to_owned(), video),
            ]),
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
                models
                    .entry(model.provider_model_id.clone())
                    .and_modify(|current| merge_model(current, &model))
                    .or_insert(model);
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
            "Gemini discovery requires an x-goog-api-key credential".to_owned(),
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
    let prefix = if base_path.is_empty() {
        "/v1beta"
    } else {
        base_path
    };
    url.set_path(&format!("{prefix}/models"));
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
    #[serde(default)]
    supported_generation_methods: Vec<String>,
}

impl ModelObject {
    fn into_discovered(self) -> ProviderResult<DiscoveredModel> {
        let provider_model_id = model_id(&self.name)?;
        let origin_model_id = match self.base_model_id {
            Some(value) if !value.trim().is_empty() => Some(model_id(&value)?),
            _ => None,
        };
        let remote_methods = self
            .supported_generation_methods
            .iter()
            .filter_map(|method| operation_for_generation_method(method))
            .map(str::to_owned)
            .collect();
        Ok(DiscoveredModel {
            provider_model_id,
            origin_model_id,
            api_types: None,
            supported_features: None,
            remote_methods: Some(remote_methods),
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

fn operation_for_generation_method(method: &str) -> Option<&'static str> {
    match method {
        "generateContent" | "streamGenerateContent" => Some(GEMINI_INTERACTIONS_OPERATION_ID),
        "embedContent" | "batchEmbedContents" => Some(GEMINI_EMBED_CONTENT_OPERATION_ID),
        "predictLongRunning" => Some(GEMINI_PREDICT_LONG_RUNNING_OPERATION_ID),
        _ => None,
    }
}

fn merge_model(current: &mut DiscoveredModel, incoming: &DiscoveredModel) {
    if let (Some(current), Some(incoming)) = (&mut current.remote_methods, &incoming.remote_methods)
    {
        current.extend(incoming.iter().cloned());
    }
}

fn models_revision(models: &[DiscoveredModel]) -> String {
    let mut hasher = Sha256::new();
    for model in models {
        hasher.update((model.provider_model_id.len() as u64).to_be_bytes());
        hasher.update(model.provider_model_id.as_bytes());
        for method in model.remote_methods.iter().flatten() {
            hasher.update((method.len() as u64).to_be_bytes());
            hasher.update(method.as_bytes());
        }
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
    use crate::catalog::{
        CatalogBuildOptions, CatalogDocuments, CatalogSnapshot, KnownProviderCatalog,
        ModelDriverCatalog,
    };
    use crate::protocol::{
        gemini_api_key, gemini_interactions_adapter, CodecRegistry, HttpBody, ProtocolError,
        ResolvedCredential,
    };
    use crate::provider::{CredentialReference, InventoryBuilder, ProviderInstanceConfig};
    use bytes::Bytes;
    use reqwest::header::HeaderMap;
    use reqwest::StatusCode;
    use serde_json::Value;
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
        ProviderInstanceConfig {
            provider_instance_name: "google-gemini-main".to_owned(),
            provider_profile_id: GEMINI_PROVIDER_PROFILE_ID.to_owned(),
            protocol_adapter_id: GEMINI_ADAPTER_ID.to_owned(),
            base_url: GEMINI_DEFAULT_BASE_URL.to_owned(),
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
        assert_eq!(known.base_url, GEMINI_DEFAULT_BASE_URL);
        assert_eq!(
            gemini_connection_contract()
                .resolve(ProviderConnectionInput::default())
                .unwrap()
                .base_url,
            GEMINI_DEFAULT_BASE_URL
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
                "region": "unsupported",
                "workspace": "unsupported",
                "account": "unsupported"
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
        assert_eq!(
            snapshot.models[0].remote_methods.as_ref().unwrap(),
            &BTreeSet::from([
                GEMINI_EMBED_CONTENT_OPERATION_ID.to_owned(),
                GEMINI_PREDICT_LONG_RUNNING_OPERATION_ID.to_owned(),
            ])
        );
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
        let model_driver: ModelDriverCatalog = serde_json::from_value(json!({
            "format": "buckyos.aicc.model-driver-catalog",
            "schema_version": 1,
            "schema_revision": 0,
            "model_driver_id": "gemini",
            "revision_seq": 1,
            "models": [{
                "id": "gemini-test",
                "api_types": ["llm", "embedding.text", "image.txt2img", "audio.tts", "video.txt2video"]
            }],
            "patterns": [],
            "defaults": {},
            "variants": [],
            "version_rules": []
        }))
        .unwrap();
        let catalog = CatalogSnapshot::build(
            1,
            CatalogDocuments {
                model_drivers: vec![model_driver],
                provider_rules: vec![gemini_provider_rules(1)],
                known_providers: vec![KnownProviderCatalog {
                    format: "buckyos.aicc.known-provider-catalog".to_owned(),
                    schema_version: 1,
                    schema_revision: 0,
                    revision_seq: 1,
                    catalog_id: "builtin".to_owned(),
                    providers: vec![gemini_known_provider()],
                }],
            },
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
                    provider_model_id: "gemini-test".to_owned(),
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
        assert_eq!(
            operations["embedding.text"],
            GEMINI_EMBED_CONTENT_OPERATION_ID
        );
        assert_eq!(
            operations["video.txt2video"],
            GEMINI_PREDICT_LONG_RUNNING_OPERATION_ID
        );
    }
}
