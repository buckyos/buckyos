use super::super::{
    validate_discovery, DiscoveredModel, DiscoveryContext, ModelAvailability, ProviderDiscovery,
    ProviderDiscoverySnapshot, ProviderError, ProviderHealthState, ProviderResult,
};
#[cfg(test)]
use super::super::{DiscoveryMode, ProviderConnectionContract, ProviderProfile};
use crate::catalog::Pricing;
#[cfg(test)]
use crate::catalog::{CurrentCatalogFile, ProviderRulesCatalog};
use crate::protocol::{
    CredentialKind, HttpRequest, HttpResponse, HttpTransport, OPENAI_CHAT_COMPLETIONS_OPERATION_ID,
    OPENAI_EMBEDDINGS_OPERATION_ID, OPENROUTER_CHAT_ADAPTER_ID, OPENROUTER_RERANK_OPERATION_ID,
};
use async_trait::async_trait;
use buckyos_api::{features, ApiType};
use reqwest::header::ETAG;
use reqwest::{Method, Url};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

pub(crate) const OPENROUTER_PROVIDER_PROFILE_ID: &str = "openrouter";

const MODELS_RESPONSE_LIMIT: usize = 16 * 1024 * 1024;

#[cfg(test)]
pub(crate) fn openrouter_profile() -> ProviderProfile {
    super::builtin_profile(OPENROUTER_PROVIDER_PROFILE_ID, DiscoveryMode::MachineApi)
}

#[cfg(test)]
pub(crate) fn openrouter_connection_contract() -> ProviderConnectionContract {
    super::builtin_connection_contract(OPENROUTER_PROVIDER_PROFILE_ID)
}

#[cfg(test)]
pub(crate) fn openrouter_known_provider() -> crate::catalog::KnownProvider {
    super::builtin_known_provider(OPENROUTER_PROVIDER_PROFILE_ID)
}

#[cfg(test)]
pub(crate) fn openrouter_provider_rules(_revision_seq: u64) -> ProviderRulesCatalog {
    super::builtin_provider_rules(OPENROUTER_PROVIDER_PROFILE_ID)
}

#[cfg(test)]
pub(crate) fn openrouter_catalog_files() -> Vec<CurrentCatalogFile> {
    super::builtin_catalog_files(&[OPENROUTER_PROVIDER_PROFILE_ID])
}

#[async_trait]
trait OpenRouterModelsTransport: Send + Sync {
    async fn send(
        &self,
        request: HttpRequest,
    ) -> crate::protocol::ProtocolResultValue<HttpResponse>;
}

#[async_trait]
impl OpenRouterModelsTransport for HttpTransport {
    async fn send(
        &self,
        request: HttpRequest,
    ) -> crate::protocol::ProtocolResultValue<HttpResponse> {
        HttpTransport::send(self, request).await
    }
}

#[derive(Clone)]
pub(crate) struct OpenRouterDiscovery {
    transport: Arc<dyn OpenRouterModelsTransport>,
}

impl OpenRouterDiscovery {
    pub(crate) fn new(transport: HttpTransport) -> Self {
        Self {
            transport: Arc::new(transport),
        }
    }

    #[cfg(test)]
    fn with_transport(transport: Arc<dyn OpenRouterModelsTransport>) -> Self {
        Self { transport }
    }
}

#[async_trait]
impl ProviderDiscovery for OpenRouterDiscovery {
    async fn discover(
        &self,
        context: &DiscoveryContext<'_>,
    ) -> ProviderResult<ProviderDiscoverySnapshot> {
        validate_context(context)?;
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
        let wire: ModelsResponse = serde_json::from_slice(&response.body).map_err(|error| {
            ProviderError::Discovery(format!("OpenRouter models response is invalid: {error}"))
        })?;
        let mut models = BTreeMap::new();
        for model in wire.data {
            if !is_canonical_model(&model) {
                continue;
            }
            let (_, origin_model_id) = model.id.split_once('/').ok_or_else(|| {
                ProviderError::Discovery(
                    "OpenRouter model ID must use vendor/model form".to_owned(),
                )
            })?;
            let origin_model_id = origin_model_id.to_owned();
            let mut supported_features = BTreeSet::new();
            if model
                .supported_parameters
                .iter()
                .any(|parameter| parameter == "tools")
            {
                supported_features.insert(features::TOOL_CALL.to_owned());
            }
            if model
                .supported_parameters
                .iter()
                .any(|parameter| parameter == "response_format")
            {
                supported_features.insert(features::JSON_SCHEMA.to_owned());
            }
            if model.architecture.as_ref().is_some_and(|architecture| {
                architecture
                    .input_modalities
                    .iter()
                    .any(|item| item == "image")
            }) {
                supported_features.insert(features::VISION.to_owned());
            }
            let rerank = model.architecture.as_ref().is_some_and(|architecture| {
                architecture
                    .output_modalities
                    .iter()
                    .any(|item| item == "rerank")
            });
            let embedding = model.architecture.as_ref().is_some_and(|architecture| {
                architecture
                    .output_modalities
                    .iter()
                    .any(|item| item == "embeddings")
            });
            let api_type = if rerank {
                ApiType::Rerank
            } else if embedding {
                ApiType::EmbeddingText
            } else {
                ApiType::Llm
            };
            let operation = if rerank {
                OPENROUTER_RERANK_OPERATION_ID
            } else if embedding {
                OPENAI_EMBEDDINGS_OPERATION_ID
            } else {
                OPENAI_CHAT_COMPLETIONS_OPERATION_ID
            };
            models.insert(
                model.id.clone(),
                DiscoveredModel {
                    provider_model_id: model.id,
                    origin_model_id: Some(origin_model_id),
                    api_types: Some(vec![api_type]),
                    supported_features: Some(supported_features),
                    remote_methods: Some(BTreeSet::from([operation.to_owned()])),
                    availability: ModelAvailability::Available,
                    deprecated: model.expiration_date.is_some(),
                    pricing: parse_pricing(model.pricing)?,
                },
            );
        }
        let models = models.into_values().collect::<Vec<_>>();
        let revision = response
            .headers
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
            .or_else(|| Some(models_revision(&models)));
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

fn is_canonical_model(model: &ModelObject) -> bool {
    if model.id.trim().is_empty()
        || model.id.contains('@')
        || model.id.contains(':')
        || model.id.starts_with('~')
        || model.id.starts_with("openrouter/")
    {
        return false;
    }
    model
        .canonical_slug
        .as_deref()
        .is_none_or(|canonical| canonical == model.id)
}

fn parse_pricing(pricing: Option<ModelPricing>) -> ProviderResult<Option<Pricing>> {
    let Some(pricing) = pricing else {
        return Ok(None);
    };
    let input_token = parse_nonnegative_price("prompt", pricing.prompt.as_deref())?;
    let output_token = parse_nonnegative_price("completion", pricing.completion.as_deref())?;
    if input_token.is_none() && output_token.is_none() {
        return Ok(None);
    }
    Ok(Some(Pricing {
        currency: "USD".to_owned(),
        input_token,
        output_token,
        cache_input_token: None,
        estimated_cost: None,
        unit: None,
        amount: None,
        rules: Vec::new(),
    }))
}

fn parse_nonnegative_price(name: &str, value: Option<&str>) -> ProviderResult<Option<f64>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value
        .parse::<f64>()
        .map_err(|_| ProviderError::Discovery(format!("OpenRouter {name} price is invalid")))?;
    if !value.is_finite() || value < 0.0 {
        return Err(ProviderError::Discovery(format!(
            "OpenRouter {name} price must be finite and non-negative"
        )));
    }
    Ok(Some(value))
}

fn validate_context(context: &DiscoveryContext<'_>) -> ProviderResult<()> {
    if context.profile.provider_profile_id != OPENROUTER_PROVIDER_PROFILE_ID
        || context.profile.default_protocol_adapter_id != OPENROUTER_CHAT_ADAPTER_ID
        || context.instance.provider_profile_id != OPENROUTER_PROVIDER_PROFILE_ID
        || context.instance.protocol_adapter_id != OPENROUTER_CHAT_ADAPTER_ID
    {
        return Err(ProviderError::InvalidConfiguration(
            "OpenRouter discovery requires its builtin profile and adapter".to_owned(),
        ));
    }
    if context.credential.audit().kind != CredentialKind::Bearer {
        return Err(ProviderError::Credential(
            "OpenRouter discovery requires a Bearer credential".to_owned(),
        ));
    }
    if context.instance.region.is_some() || context.instance.account.is_some() {
        return Err(ProviderError::InvalidConfiguration(
            "OpenRouter profile does not accept region or account".to_owned(),
        ));
    }
    Ok(())
}

fn models_endpoint(base_url: &str) -> ProviderResult<String> {
    let mut url = Url::parse(base_url).map_err(|_| {
        ProviderError::InvalidConfiguration("OpenRouter base_url is invalid".to_owned())
    })?;
    if !matches!(url.scheme(), "http" | "https") || url.cannot_be_a_base() {
        return Err(ProviderError::InvalidConfiguration(
            "OpenRouter base_url must be an absolute HTTP URL".to_owned(),
        ));
    }
    let path = url.path().trim_end_matches('/');
    let prefix = if path.ends_with("/api/v1") {
        path.to_owned()
    } else if path.is_empty() {
        "/api/v1".to_owned()
    } else {
        format!("{path}/api/v1")
    };
    url.set_path(&format!("{prefix}/models"));
    url.set_query(Some("output_modalities=all"));
    url.set_fragment(None);
    Ok(url.to_string())
}

fn ensure_success(response: &HttpResponse) -> ProviderResult<()> {
    if response.status.is_success() {
        return Ok(());
    }
    Err(ProviderError::Discovery(format!(
        "OpenRouter models request failed with status {} (request {})",
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
    data: Vec<ModelObject>,
}

#[derive(Deserialize)]
struct ModelObject {
    id: String,
    canonical_slug: Option<String>,
    #[serde(default)]
    supported_parameters: Vec<String>,
    architecture: Option<ModelArchitecture>,
    pricing: Option<ModelPricing>,
    expiration_date: Option<String>,
}

#[derive(Deserialize)]
struct ModelArchitecture {
    #[serde(default)]
    input_modalities: Vec<String>,
    #[serde(default)]
    output_modalities: Vec<String>,
}

#[derive(Deserialize)]
struct ModelPricing {
    prompt: Option<String>,
    completion: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{ProtocolError, ResolvedCredential};
    use crate::provider::{CredentialReference, ProviderInstanceConfig};
    use bytes::Bytes;
    use reqwest::header::{HeaderMap, AUTHORIZATION};
    use reqwest::StatusCode;
    use std::sync::Mutex;

    struct FakeTransport {
        request: Mutex<Option<HttpRequest>>,
        response: Mutex<Option<Result<HttpResponse, ProtocolError>>>,
    }

    #[async_trait]
    impl OpenRouterModelsTransport for FakeTransport {
        async fn send(
            &self,
            request: HttpRequest,
        ) -> crate::protocol::ProtocolResultValue<HttpResponse> {
            *self.request.lock().unwrap() = Some(request);
            self.response.lock().unwrap().take().unwrap()
        }
    }

    #[tokio::test]
    async fn discovery_keeps_only_canonical_models_and_dynamic_prices() {
        let transport = Arc::new(FakeTransport {
            request: Mutex::new(None),
            response: Mutex::new(Some(Ok(HttpResponse {
                status: StatusCode::OK,
                headers: HeaderMap::new(),
                body: Bytes::from_static(br#"{"data":[{"id":"openai/model-a","canonical_slug":"openai/model-a","supported_parameters":["tools","response_format"],"architecture":{"input_modalities":["text","image"],"output_modalities":["text"]},"pricing":{"prompt":"0.000001","completion":"0.000002"},"expiration_date":null},{"id":"openai/text-embedding-3-small","canonical_slug":"openai/text-embedding-3-small","supported_parameters":[],"architecture":{"input_modalities":["text"],"output_modalities":["embeddings"]},"pricing":null,"expiration_date":null},{"id":"cohere/rerank-v3.5","canonical_slug":"cohere/rerank-v3.5","supported_parameters":[],"architecture":{"input_modalities":["text"],"output_modalities":["rerank"]},"pricing":null,"expiration_date":null},{"id":"openai/model-a:free","canonical_slug":"openai/model-a","supported_parameters":[],"architecture":null,"pricing":null,"expiration_date":null},{"id":"openrouter/auto","canonical_slug":"openrouter/auto","supported_parameters":[],"architecture":null,"pricing":null,"expiration_date":null}]}"#),
                request_id: "request-1".to_owned(),
                retry_after: None,
            }))),
        });
        let discovery = OpenRouterDiscovery::with_transport(transport.clone());
        let profile = openrouter_profile();
        let instance = ProviderInstanceConfig {
            provider_instance_name: "openrouter-main".to_owned(),
            provider_profile_id: OPENROUTER_PROVIDER_PROFILE_ID.to_owned(),
            protocol_adapter_id: OPENROUTER_CHAT_ADAPTER_ID.to_owned(),
            base_url: openrouter_known_provider().base_url,
            credential: CredentialReference {
                reference: "secret://openrouter".to_owned(),
            },
            credential_kind: None,
            provider_rules_id: Some(OPENROUTER_PROVIDER_PROFILE_ID.to_owned()),
            region: None,
            workspace: None,
            account: None,
        };
        let credential = ResolvedCredential::bearer("secret://openrouter", "secret").unwrap();
        let snapshot = discovery
            .discover(&DiscoveryContext {
                profile: &profile,
                instance: &instance,
                credential: &credential,
            })
            .await
            .unwrap();
        assert_eq!(snapshot.models.len(), 3);
        let language_model = snapshot
            .models
            .iter()
            .find(|model| model.provider_model_id == "openai/model-a")
            .unwrap();
        assert_eq!(language_model.origin_model_id.as_deref(), Some("model-a"));
        assert_eq!(
            language_model.pricing.as_ref().unwrap().input_token,
            Some(0.000001)
        );
        assert!(language_model
            .supported_features
            .as_ref()
            .unwrap()
            .contains(features::VISION));
        let reranker = snapshot
            .models
            .iter()
            .find(|model| model.provider_model_id == "cohere/rerank-v3.5")
            .unwrap();
        assert_eq!(reranker.api_types, Some(vec![ApiType::Rerank]));
        assert_eq!(
            reranker.remote_methods,
            Some(BTreeSet::from([OPENROUTER_RERANK_OPERATION_ID.to_owned()]))
        );
        let embedding = snapshot
            .models
            .iter()
            .find(|model| model.provider_model_id == "openai/text-embedding-3-small")
            .unwrap();
        assert_eq!(embedding.api_types, Some(vec![ApiType::EmbeddingText]));
        assert_eq!(
            embedding.remote_methods,
            Some(BTreeSet::from([OPENAI_EMBEDDINGS_OPERATION_ID.to_owned()]))
        );
        let request = transport.request.lock().unwrap().take().unwrap();
        assert_eq!(
            request.url,
            "https://openrouter.ai/api/v1/models?output_modalities=all"
        );
        assert_eq!(request.headers[AUTHORIZATION], "Bearer secret");
        assert_eq!(
            openrouter_known_provider().base_url,
            "https://openrouter.ai/api/v1"
        );
        let rules = openrouter_provider_rules(3);
        let llm_pattern = rules
            .patterns
            .iter()
            .find(|pattern| pattern.operations.contains_key("llm"))
            .unwrap();
        assert_eq!(
            llm_pattern.operations["llm"],
            OPENAI_CHAT_COMPLETIONS_OPERATION_ID
        );
        assert_eq!(openrouter_provider_rules(3).origin_mappings.len(), 1);
    }

    #[test]
    fn endpoint_and_pricing_reject_invalid_boundary_values() {
        assert_eq!(
            models_endpoint("https://openrouter.ai").unwrap(),
            "https://openrouter.ai/api/v1/models?output_modalities=all"
        );
        assert!(models_endpoint("file:///tmp/openrouter").is_err());
        assert!(parse_nonnegative_price("prompt", Some("-0.1")).is_err());
        assert!(parse_nonnegative_price("prompt", Some("NaN")).is_err());
    }
}
