use super::super::{
    validate_discovery, DiscoveredModel, DiscoveryContext, ModelAvailability, ProviderDiscovery,
    ProviderDiscoverySnapshot, ProviderError, ProviderHealthState, ProviderResult,
};
use crate::protocol::{CredentialKind, HttpRequest, HttpResponse, HttpTransport};
use async_trait::async_trait;
use reqwest::{Method, Url};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

const MODELS_RESPONSE_LIMIT: usize = 8 * 1024 * 1024;
const DISCOVERY_PAGE_SIZE: usize = 1000;
const MAX_DISCOVERY_PAGES: usize = 100;

#[derive(Clone, Copy)]
pub(super) struct AnthropicModelsSpec {
    pub provider_profile_id: &'static str,
    pub version_header: bool,
    pub label: &'static str,
}

#[async_trait]
trait AnthropicModelsTransport: Send + Sync {
    async fn send(
        &self,
        request: HttpRequest,
    ) -> crate::protocol::ProtocolResultValue<HttpResponse>;
}

#[async_trait]
impl AnthropicModelsTransport for HttpTransport {
    async fn send(
        &self,
        request: HttpRequest,
    ) -> crate::protocol::ProtocolResultValue<HttpResponse> {
        HttpTransport::send(self, request).await
    }
}

#[derive(Clone)]
pub(crate) struct AnthropicModelsDiscovery {
    spec: AnthropicModelsSpec,
    transport: Arc<dyn AnthropicModelsTransport>,
}

impl AnthropicModelsDiscovery {
    pub(super) fn new(spec: AnthropicModelsSpec, transport: HttpTransport) -> Self {
        Self {
            spec,
            transport: Arc::new(transport),
        }
    }

    #[cfg(test)]
    fn with_transport(
        spec: AnthropicModelsSpec,
        transport: Arc<dyn AnthropicModelsTransport>,
    ) -> Self {
        Self { spec, transport }
    }
}

#[async_trait]
impl ProviderDiscovery for AnthropicModelsDiscovery {
    async fn discover(
        &self,
        context: &DiscoveryContext<'_>,
    ) -> ProviderResult<ProviderDiscoverySnapshot> {
        validate_context(self.spec, context)?;
        let mut models = BTreeMap::<String, DiscoveredModel>::new();
        let mut after_id = None;
        let mut seen_cursors = BTreeSet::new();

        for _ in 0..MAX_DISCOVERY_PAGES {
            let mut request = HttpRequest::new(
                Method::GET,
                models_endpoint(&context.instance.base_url, after_id.as_deref())?,
            );
            context
                .credential
                .apply(&mut request.headers)
                .map_err(|error| ProviderError::Credential(error.to_string()))?;
            if self.spec.version_header {
                request.headers.insert(
                    "anthropic-version",
                    reqwest::header::HeaderValue::from_static("2023-06-01"),
                );
            }
            request.timeout = Some(Duration::from_secs(30));
            request.max_response_bytes = Some(MODELS_RESPONSE_LIMIT);

            let response = self
                .transport
                .send(request)
                .await
                .map_err(|error| ProviderError::Discovery(error.to_string()))?;
            ensure_success(self.spec.label, &response)?;
            let page: ModelsResponse = serde_json::from_slice(&response.body).map_err(|error| {
                ProviderError::Discovery(format!(
                    "{} models response is invalid: {error}",
                    self.spec.label
                ))
            })?;
            for model in page.data {
                if model.object != "model" || model.id.trim().is_empty() || model.id.contains('@') {
                    return Err(ProviderError::Discovery(format!(
                        "{} Models API returned an invalid model object",
                        self.spec.label
                    )));
                }
                models.entry(model.id.clone()).or_insert(DiscoveredModel {
                    provider_model_id: model.id,
                    origin_model_id: None,
                    api_types: None,
                    supported_features: None,
                    remote_methods: None,
                    availability: ModelAvailability::Available,
                    deprecated: false,
                    pricing: None,
                });
            }

            if !page.has_more {
                let models = models.into_values().collect::<Vec<_>>();
                let snapshot = ProviderDiscoverySnapshot {
                    revision: Some(models_revision(&models)),
                    discovered_at_ms: super::super::now_ms()?,
                    health: ProviderHealthState::Healthy,
                    models,
                };
                validate_discovery(&snapshot)?;
                return Ok(snapshot);
            }
            let cursor = page
                .last_id
                .filter(|cursor| !cursor.trim().is_empty())
                .ok_or_else(|| {
                    ProviderError::Discovery(format!(
                        "{} Models API omitted last_id while has_more is true",
                        self.spec.label
                    ))
                })?;
            if !seen_cursors.insert(cursor.clone()) {
                return Err(ProviderError::Discovery(format!(
                    "{} Models API repeated a pagination cursor",
                    self.spec.label
                )));
            }
            after_id = Some(cursor);
        }
        Err(ProviderError::Discovery(format!(
            "{} Models API exceeded the pagination limit",
            self.spec.label
        )))
    }
}

fn validate_context(
    spec: AnthropicModelsSpec,
    context: &DiscoveryContext<'_>,
) -> ProviderResult<()> {
    if context.profile.provider_profile_id != spec.provider_profile_id
        || context.instance.provider_profile_id != spec.provider_profile_id
        || context.instance.protocol_adapter_id != context.profile.default_protocol_adapter_id
    {
        return Err(ProviderError::InvalidConfiguration(format!(
            "{} discovery requires its builtin profile and adapter",
            spec.label
        )));
    }
    if context.credential.audit().kind != CredentialKind::NamedHeader {
        return Err(ProviderError::Credential(format!(
            "{} discovery requires its configured named-header credential",
            spec.label
        )));
    }
    Url::parse(&context.instance.base_url).map_err(|_| {
        ProviderError::InvalidConfiguration(format!("{} base_url is invalid", spec.label))
    })?;
    Ok(())
}

fn models_endpoint(base_url: &str, after_id: Option<&str>) -> ProviderResult<String> {
    let mut url = Url::parse(base_url).map_err(|_| {
        ProviderError::InvalidConfiguration("models base_url is invalid".to_owned())
    })?;
    if !matches!(url.scheme(), "http" | "https") || url.cannot_be_a_base() {
        return Err(ProviderError::InvalidConfiguration(
            "models base_url must be an absolute HTTP URL".to_owned(),
        ));
    }
    let base_path = url.path().trim_end_matches('/');
    let prefix = if base_path.ends_with("/v1") {
        base_path.to_owned()
    } else if base_path.is_empty() {
        "/v1".to_owned()
    } else {
        format!("{base_path}/v1")
    };
    url.set_path(&format!("{prefix}/models"));
    url.set_query(None);
    url.set_fragment(None);
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("limit", &DISCOVERY_PAGE_SIZE.to_string());
        if let Some(after_id) = after_id {
            query.append_pair("after_id", after_id);
        }
    }
    Ok(url.to_string())
}

fn ensure_success(label: &str, response: &HttpResponse) -> ProviderResult<()> {
    if response.status.is_success() {
        return Ok(());
    }
    let message = serde_json::from_slice::<ErrorResponse>(&response.body)
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
        "{label} models request failed with status {} (request {}): {message}",
        response.status, response.request_id
    )))
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
struct ModelsResponse {
    #[serde(default)]
    data: Vec<ModelObject>,
    #[serde(default)]
    has_more: bool,
    last_id: Option<String>,
}

#[derive(Deserialize)]
struct ModelObject {
    id: String,
    #[serde(rename = "type")]
    object: String,
}

#[derive(Deserialize)]
struct ErrorResponse {
    error: Option<ErrorObject>,
}

#[derive(Deserialize)]
struct ErrorObject {
    message: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{HttpBody, ProtocolError, ResolvedCredential};
    use crate::provider::builtin::claude::{
        claude_connection_contract, claude_profile, CLAUDE_SPEC,
    };
    use crate::provider::builtin::minimax::{
        minimax_connection_contract, minimax_profile, MINIMAX_SPEC,
    };
    use crate::provider::{CredentialReference, ProviderInstanceConfig};
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
    }

    #[async_trait]
    impl AnthropicModelsTransport for FakeTransport {
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
            provider_instance_name: "claude-main".to_owned(),
            provider_profile_id: "claude".to_owned(),
            protocol_adapter_id: "claude-messages".to_owned(),
            base_url: claude_connection_contract().default_base_url,
            credential: CredentialReference {
                reference: "secret://claude/main".to_owned(),
            },
            credential_kind: None,
            provider_rules_id: Some("claude".to_owned()),
            region: None,
            workspace: None,
            account: None,
        }
    }

    #[tokio::test]
    async fn paginates_models_and_applies_versioned_named_header_auth() {
        let transport = FakeTransport::responses([
            json!({
                "data": [{"id": "claude-z", "type": "model"}],
                "has_more": true,
                "last_id": "next model"
            }),
            json!({
                "data": [{"id": "claude-a", "type": "model"}],
                "has_more": false
            }),
        ]);
        let discovery = AnthropicModelsDiscovery::with_transport(CLAUDE_SPEC, transport.clone());
        let profile = claude_profile();
        let instance = instance();
        let credential =
            ResolvedCredential::named_header("secret://claude/main", "x-api-key", "secret")
                .unwrap();
        let snapshot = discovery
            .discover(&DiscoveryContext {
                profile: &profile,
                instance: &instance,
                credential: &credential,
            })
            .await
            .unwrap();

        assert_eq!(
            snapshot
                .models
                .iter()
                .map(|model| model.provider_model_id.as_str())
                .collect::<Vec<_>>(),
            vec!["claude-a", "claude-z"]
        );
        assert!(snapshot.revision.unwrap().starts_with("sha256:"));
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            requests[0].url,
            "https://api.anthropic.com/v1/models?limit=1000"
        );
        assert!(requests[1].url.ends_with("limit=1000&after_id=next+model"));
        assert_eq!(requests[0].headers["x-api-key"], "secret");
        assert_eq!(requests[0].headers["anthropic-version"], "2023-06-01");
        assert!(matches!(requests[0].body, HttpBody::Empty));
        assert!(!format!("{:?}", requests[0]).contains("secret"));
    }

    #[tokio::test]
    async fn discovers_minimax_models_without_the_claude_version_header() {
        let transport = FakeTransport::responses([json!({
            "data": [{"id": "MiniMax-M2.7", "type": "model"}],
            "has_more": false
        })]);
        let discovery = AnthropicModelsDiscovery::with_transport(MINIMAX_SPEC, transport.clone());
        let profile = minimax_profile();
        let mut instance = instance();
        instance.provider_instance_name = "minimax-main".to_owned();
        instance.provider_profile_id = "minimax".to_owned();
        instance.protocol_adapter_id = "minimax-messages".to_owned();
        instance.base_url = minimax_connection_contract().default_base_url;
        instance.provider_rules_id = Some("minimax".to_owned());
        instance.region = Some("global".to_owned());
        let credential =
            ResolvedCredential::named_header("secret://minimax/main", "x-api-key", "secret")
                .unwrap();

        let snapshot = discovery
            .discover(&DiscoveryContext {
                profile: &profile,
                instance: &instance,
                credential: &credential,
            })
            .await
            .unwrap();

        assert_eq!(snapshot.models[0].provider_model_id, "MiniMax-M2.7");
        let requests = transport.requests.lock().unwrap();
        assert_eq!(
            requests[0].url,
            "https://api.minimax.io/anthropic/v1/models?limit=1000"
        );
        assert_eq!(requests[0].headers["x-api-key"], "secret");
        assert!(!requests[0].headers.contains_key("anthropic-version"));
    }
}
