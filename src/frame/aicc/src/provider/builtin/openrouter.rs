use super::super::{
    validate_discovery, DiscoveredModel, DiscoveryContext, ModelAvailability, ProviderDiscovery,
    ProviderDiscoverySnapshot, ProviderError, ProviderHealthState, ProviderResult,
};
#[cfg(test)]
use super::super::{DiscoveryMode, ProviderProfile};
use crate::catalog::{
    CatalogSnapshot, ModelIdentity, ModelMatchFailure, Pricing, ProviderModelMatch,
};
#[cfg(test)]
use crate::catalog::{CurrentCatalogFile, ProviderRulesCatalog};
use crate::protocol::openrouter_decisions::{
    DECISION_FEATURES, JEV_ALIAS_ID, JEV_BUILD_ID, JEV_CHANNEL_ID, JEV_ORIGIN_ID,
    OPENROUTER_DECISIONS_OPERATION_ID,
};
use crate::protocol::{
    CredentialKind, HttpRequest, HttpResponse, HttpTransport, OPENAI_EMBEDDINGS_OPERATION_ID,
    OPENAI_RESPONSES_OPERATION_ID, OPENROUTER_RERANK_OPERATION_ID, OPENROUTER_RESPONSES_ADAPTER_ID,
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
    fn match_model_driver(&self, id: &str, catalog: &CatalogSnapshot) -> ProviderModelMatch {
        if matches!(id, JEV_CHANNEL_ID | JEV_ALIAS_ID) {
            return ProviderModelMatch::Matched(ModelIdentity {
                model_driver_id: "typesafe".into(),
                model_id: JEV_ORIGIN_ID.into(),
            });
        }
        if id.starts_with("typesafe/") || id.starts_with("~typesafe/") {
            return ProviderModelMatch::Failed(ModelMatchFailure::UnresolvedAlias);
        }
        let Some((vendor, model)) = id.split_once('/') else {
            return ProviderModelMatch::NotHandled;
        };
        let vendor = vendor.to_ascii_lowercase();
        let driver = match vendor.as_str() {
            "anthropic" => "claude",
            "google" => "gemini",
            "moonshotai" => "kimi",
            "z-ai" => "glm",
            other => other,
        };
        if catalog.resolve_model(driver, model).is_ok() {
            return ProviderModelMatch::Matched(ModelIdentity {
                model_driver_id: driver.into(),
                model_id: model.into(),
            });
        }
        let matches = catalog
            .model_driver(driver)
            .into_iter()
            .flat_map(|driver| &driver.models)
            .filter(|entry| entry.id.eq_ignore_ascii_case(model))
            .map(|entry| ModelIdentity {
                model_driver_id: driver.into(),
                model_id: entry.id.clone(),
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [identity] => ProviderModelMatch::Matched(identity.clone()),
            [] => ProviderModelMatch::Failed(ModelMatchFailure::UnresolvedAlias),
            _ => ProviderModelMatch::Failed(ModelMatchFailure::Ambiguous {
                candidates: matches,
            }),
        }
    }

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
            ProviderError::DiscoveryResponse(format!(
                "OpenRouter models response is invalid: {error}"
            ))
        })?;
        let mut models = BTreeMap::new();
        let verified_target = wire
            .data
            .iter()
            .find(|m| m.id == JEV_CHANNEL_ID)
            .is_some_and(|m| {
                m.canonical_slug.as_deref() == Some(JEV_BUILD_ID)
                    && m.context_length == Some(32000)
                    && m.architecture.as_ref().is_some_and(|a| {
                        a.output_modalities == ["decisions"] && a.input_modalities == ["text"]
                    })
            });
        for model in wire.data {
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
            let classification = classify_modalities(model.architecture.as_ref());
            let (api_types, remote_methods) = match classification {
                Some((api_type, operation)) => {
                    (vec![api_type], BTreeSet::from([operation.to_owned()]))
                }
                None => {
                    log::warn!(
                        "OpenRouter model {} has unknown or conflicting output modalities",
                        model.id
                    );
                    (Vec::new(), BTreeSet::new())
                }
            };
            let decision = api_types == [ApiType::Decision];
            let mut availability = ModelAvailability::Available;
            if decision {
                supported_features = DECISION_FEATURES.into_iter().map(str::to_owned).collect();
                let identity_verified = match model.id.as_str() {
                    JEV_CHANNEL_ID => model.canonical_slug.as_deref() == Some(JEV_BUILD_ID),
                    JEV_ALIAS_ID => {
                        verified_target
                            && model
                                .alias_target
                                .as_ref()
                                .is_some_and(|target| target.slug == JEV_CHANNEL_ID)
                    }
                    _ => false,
                };
                if !identity_verified
                    || model.context_length != Some(32000)
                    || !model
                        .architecture
                        .as_ref()
                        .is_some_and(|a| a.input_modalities == ["text"])
                {
                    availability = ModelAvailability::Unavailable;
                    log::warn!("OpenRouter decision model {} unavailable: unverified build/alias target or channel context/input modalities", model.id);
                }
            }
            if models.contains_key(&model.id) {
                return Err(ProviderError::DiscoveryResponse(
                    "duplicate OpenRouter model ID".into(),
                ));
            }
            models.insert(
                model.id.clone(),
                DiscoveredModel {
                    provider_model_id: model.id,

                    api_types: Some(api_types),
                    supported_features: Some(supported_features),
                    unsupported_features: BTreeSet::new(),
                    remote_methods: Some(remote_methods),
                    availability,
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
        source_url: Some("https://openrouter.ai/api/v1/models?output_modalities=all".into()),
        verified_at: Some(super::super::now_ms()?.to_string()),
        ratio_exception: None,
        cache_write_input_token: None,
        audio_input_token: None,
        image_input_token: None,
        cache_write_1h_input_token: None,
        audio_output_token: None,
        image_output_token: None,
        input_token,
        output_token,
        cache_input_token: None,
        estimated_cost: None,
        unit: None,
        amount: None,
        rules: Vec::new(),
        tiers: None,

        time_windows: Vec::new(),
    }))
}

fn parse_nonnegative_price(name: &str, value: Option<&str>) -> ProviderResult<Option<f64>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.parse::<f64>().map_err(|_| {
        ProviderError::DiscoveryResponse(format!("OpenRouter {name} price is invalid"))
    })?;
    if value == -1.0 {
        return Ok(None);
    }
    if !value.is_finite() || value < 0.0 {
        return Err(ProviderError::DiscoveryResponse(format!(
            "OpenRouter {name} price must be finite and non-negative"
        )));
    }
    Ok(Some(value))
}

fn validate_context(context: &DiscoveryContext<'_>) -> ProviderResult<()> {
    if context.profile.provider_profile_id != OPENROUTER_PROVIDER_PROFILE_ID
        || context.profile.default_protocol_adapter_id != OPENROUTER_RESPONSES_ADAPTER_ID
        || context.instance.provider_profile_id != OPENROUTER_PROVIDER_PROFILE_ID
        || context.instance.protocol_adapter_id != OPENROUTER_RESPONSES_ADAPTER_ID
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
    let message = format!(
        "OpenRouter models request failed with status {} (request {})",
        response.status, response.request_id
    );
    Err(if matches!(response.status.as_u16(), 401 | 403) {
        ProviderError::Credential(message)
    } else {
        ProviderError::Discovery(message)
    })
}

fn models_revision(models: &[DiscoveredModel]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(serde_json::to_vec(models).expect("validated discovery is serializable"));
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
    alias_target: Option<AliasTarget>,
    context_length: Option<u64>,
    #[serde(default)]
    supported_parameters: Vec<String>,
    architecture: Option<ModelArchitecture>,
    pricing: Option<ModelPricing>,
    expiration_date: Option<String>,
}

#[derive(Deserialize)]
struct AliasTarget {
    slug: String,
}

fn classify_modalities(
    architecture: Option<&ModelArchitecture>,
) -> Option<(ApiType, &'static str)> {
    let output = &architecture?.output_modalities;
    match output.as_slice() {
        [one] if one == "decisions" => Some((ApiType::Decision, OPENROUTER_DECISIONS_OPERATION_ID)),
        [one] if one == "rerank" => Some((ApiType::Rerank, OPENROUTER_RERANK_OPERATION_ID)),
        [one] if one == "embeddings" => {
            Some((ApiType::EmbeddingText, OPENAI_EMBEDDINGS_OPERATION_ID))
        }
        _ if output.iter().any(|m| m == "text")
            && output
                .iter()
                .all(|m| matches!(m.as_str(), "text" | "image" | "audio")) =>
        {
            Some((ApiType::Llm, OPENAI_RESPONSES_OPERATION_ID))
        }
        _ => None,
    }
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
    async fn discovery_preserves_aliases_for_identity_diagnostics_and_dynamic_prices() {
        let transport = Arc::new(FakeTransport {
            request: Mutex::new(None),
            response: Mutex::new(Some(Ok(HttpResponse {
                status: StatusCode::OK,
                headers: HeaderMap::new(),
                body: Bytes::from_static(br#"{"data":[{"id":"openai/gpt-5.4-mini","canonical_slug":"openai/gpt-5.4-mini","supported_parameters":["tools","response_format"],"architecture":{"input_modalities":["text","image"],"output_modalities":["text"]},"pricing":{"prompt":"0.000001","completion":"0.000002"},"expiration_date":null},{"id":"openai/text-embedding-3-small","canonical_slug":"openai/text-embedding-3-small","supported_parameters":[],"architecture":{"input_modalities":["text"],"output_modalities":["embeddings"]},"pricing":null,"expiration_date":null},{"id":"cohere/rerank-v3.5","canonical_slug":"cohere/rerank-v3.5","supported_parameters":[],"architecture":{"input_modalities":["text"],"output_modalities":["rerank"]},"pricing":null,"expiration_date":null},{"id":"openai/gpt-5.4-mini:free","canonical_slug":"openai/gpt-5.4-mini","supported_parameters":[],"architecture":null,"pricing":null,"expiration_date":null},{"id":"openrouter/auto","canonical_slug":"openrouter/auto","supported_parameters":[],"architecture":null,"pricing":{"prompt":"-1","completion":"-1"},"expiration_date":null}]}"#),
                request_id: "request-1".to_owned(),
                retry_after: None,
            }))),
        });
        let discovery = OpenRouterDiscovery::with_transport(transport.clone());
        let profile = openrouter_profile();
        let instance = ProviderInstanceConfig {
            provider_instance_name: "openrouter-main".to_owned(),
            provider_profile_id: OPENROUTER_PROVIDER_PROFILE_ID.to_owned(),
            protocol_adapter_id: OPENROUTER_RESPONSES_ADAPTER_ID.to_owned(),
            base_url: openrouter_known_provider().base_url,
            credential: CredentialReference {
                reference: "secret://openrouter".to_owned(),
            },
            credential_kind: None,
            provider_rules_id: Some(OPENROUTER_PROVIDER_PROFILE_ID.to_owned()),
            region: None,
            workspace: None,
            account: None,
            request_timeout: Duration::from_secs(120),
            auto_sync_models: true,
            instance_rules: None,
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
        assert_eq!(snapshot.models.len(), 5);
        let language_model = snapshot
            .models
            .iter()
            .find(|model| model.provider_model_id == "openai/gpt-5.4-mini")
            .unwrap();
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
        assert!(snapshot
            .models
            .iter()
            .find(|model| model.provider_model_id == "openrouter/auto")
            .unwrap()
            .pricing
            .is_none());
        let catalog = crate::settings::MetadataSources {
            builtin: crate::settings::load_builtin_metadata().unwrap(),
            ..Default::default()
        }
        .build_snapshot(2, &Default::default())
        .unwrap();
        let providers = super::super::builtin_provider_registry(&catalog).unwrap();
        let inventory = crate::provider::InventoryBuilder::build_with_matcher(
            &profile,
            &instance,
            snapshot,
            &catalog,
            &providers.codecs(),
            Some(&discovery),
        )
        .unwrap();
        assert_eq!(inventory.models.len(), 3);
        assert!(inventory
            .unmatched_models
            .iter()
            .any(|model| model.provider_model_id == "openrouter/auto"));
        let registry =
            crate::service::builtin_registry_for_test(&catalog, &[inventory.as_model_inventory()]);
        let candidates = registry
            .resolve_candidates("llm.gpt-mini", ApiType::Llm)
            .unwrap();
        assert!(!candidates.candidates.is_empty());
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
        assert_eq!(llm_pattern.operations["llm"], OPENAI_RESPONSES_OPERATION_ID);
    }

    fn jev_fixture() -> serde_json::Value {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../test/aicc_test/acceptance/fixtures/openrouter-jev-models.json"
        )))
        .unwrap()
    }

    async fn jev_inventory(
        body: serde_json::Value,
    ) -> ProviderResult<crate::provider::ProviderInventorySnapshot> {
        let transport = Arc::new(FakeTransport {
            request: Mutex::new(None),
            response: Mutex::new(Some(Ok(HttpResponse {
                status: StatusCode::OK,
                headers: HeaderMap::new(),
                body: Bytes::from(serde_json::to_vec(&body).unwrap()),
                request_id: "jev-discovery".into(),
                retry_after: None,
            }))),
        });
        let discovery = OpenRouterDiscovery::with_transport(transport);
        let profile = openrouter_profile();
        let instance = ProviderInstanceConfig {
            provider_instance_name: "openrouter-test".into(),
            provider_profile_id: "openrouter".into(),
            protocol_adapter_id: "openrouter-responses".into(),
            base_url: "https://proxy.test/api/v1".into(),
            credential: CredentialReference {
                reference: "secret://test".into(),
            },
            credential_kind: None,
            provider_rules_id: Some("openrouter".into()),
            region: None,
            workspace: None,
            account: None,
            request_timeout: Duration::from_secs(30),
            auto_sync_models: true,
            instance_rules: None,
        };
        let credential = ResolvedCredential::bearer("secret://test", "test").unwrap();
        let snapshot = discovery
            .discover(&DiscoveryContext {
                profile: &profile,
                instance: &instance,
                credential: &credential,
            })
            .await?;
        let catalog = crate::settings::MetadataSources {
            builtin: crate::settings::load_builtin_metadata().unwrap(),
            ..Default::default()
        }
        .build_snapshot(2, &Default::default())
        .unwrap();
        let providers = super::super::builtin_provider_registry(&catalog).unwrap();
        crate::provider::InventoryBuilder::build_with_matcher(
            &profile,
            &instance,
            snapshot,
            &catalog,
            &providers.codecs(),
            Some(&discovery),
        )
    }

    #[tokio::test]
    async fn openrouter_decision_discovery_requires_verified_inventory_and_narrows_limits() {
        let inventory = jev_inventory(jev_fixture()).await.unwrap();
        assert_eq!(inventory.models.len(), 2);
        for model in &inventory.models {
            assert_eq!(model.origin_model_id, JEV_ORIGIN_ID);
            assert_eq!(model.api_types, vec![ApiType::Decision]);
            assert_eq!(
                model.operations,
                BTreeMap::from([("decision".into(), OPENROUTER_DECISIONS_OPERATION_ID.into())])
            );
            assert_eq!(model.capabilities["max_context_tokens"], 32000);
            assert_eq!(model.capabilities["decision.max_input_bytes"], 32000);
            assert_eq!(model.capabilities["decision.max_options"], 255);
            assert_eq!(model.capabilities["decision.max_levels"], 10);
            assert!(!model.capabilities.contains_key("tool_call"));
            let price = &model.pricing.as_ref().unwrap().value;
            assert_eq!(price.input_token, Some(0.000000042));
            assert_eq!(price.output_token, Some(0.0));
            assert!(price.source_url.as_ref().unwrap().contains("openrouter.ai"));
        }
        assert!(inventory
            .unmatched_models
            .iter()
            .any(|m| m.provider_model_id == "typesafe/jev-router"));
        assert!(jev_inventory(serde_json::json!({"data":[]}))
            .await
            .unwrap()
            .models
            .is_empty());
        for (field, value) in [
            (
                "canonical_slug",
                serde_json::json!("typesafe/jev-1.14-20261001"),
            ),
            ("context_length", serde_json::json!(16000)),
        ] {
            let mut fixture = jev_fixture();
            let target = fixture["data"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|m| m["id"] == JEV_CHANNEL_ID)
                .unwrap();
            target[field] = value;
            assert!(jev_inventory(fixture)
                .await
                .unwrap()
                .models
                .iter()
                .all(|m| m.api_types.is_empty()));
        }
        let mut fixture = jev_fixture();
        fixture["data"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|m| m["id"] == JEV_ALIAS_ID)
            .unwrap()["alias_target"]["slug"] = serde_json::json!("typesafe/jev-1.14");
        let drift = jev_inventory(fixture).await.unwrap();
        assert!(drift
            .models
            .iter()
            .find(|m| m.provider_model_id == JEV_ALIAS_ID)
            .unwrap()
            .api_types
            .is_empty());
        assert_eq!(
            drift
                .models
                .iter()
                .find(|m| m.provider_model_id == JEV_CHANNEL_ID)
                .unwrap()
                .api_types,
            vec![ApiType::Decision]
        );
        let mut fixture = jev_fixture();
        for model in fixture["data"].as_array_mut().unwrap() {
            model["pricing"] = serde_json::Value::Null;
        }
        assert!(jev_inventory(fixture)
            .await
            .unwrap()
            .models
            .iter()
            .all(|m| m.pricing.is_none()));
        let mut fixture = jev_fixture();
        let duplicate = fixture["data"][0].clone();
        fixture["data"].as_array_mut().unwrap().push(duplicate);
        assert!(jev_inventory(fixture).await.is_err());
    }

    #[test]
    fn openrouter_unknown_modalities_and_future_identities_fail_closed() {
        for output in [
            vec!["decisions", "text"],
            vec!["rerank", "embeddings"],
            vec!["future"],
            vec![],
        ] {
            assert!(classify_modalities(Some(&ModelArchitecture {
                input_modalities: vec!["text".into()],
                output_modalities: output.into_iter().map(str::to_owned).collect()
            }))
            .is_none());
        }
        let catalog = crate::settings::MetadataSources {
            builtin: crate::settings::load_builtin_metadata().unwrap(),
            ..Default::default()
        }
        .build_snapshot(2, &Default::default())
        .unwrap();
        let discovery = OpenRouterDiscovery::with_transport(Arc::new(FakeTransport {
            request: Mutex::new(None),
            response: Mutex::new(None),
        }));
        for id in [
            "typesafe/jev-1.14",
            JEV_BUILD_ID,
            "typesafe/jev-router",
            "typesafe/jev-1.13.0",
            "~typesafe/jev-preview",
        ] {
            assert!(matches!(
                discovery.match_model_driver(id, &catalog),
                ProviderModelMatch::Failed(ModelMatchFailure::UnresolvedAlias)
            ));
        }
        assert!(matches!(
            discovery.match_model_driver("openai/gpt-5.4-mini", &catalog),
            ProviderModelMatch::Matched(_)
        ));
        for (base, expected) in [
            (
                "https://proxy.test/tenant/api/v1/",
                "https://proxy.test/tenant/api/v1/models?output_modalities=all",
            ),
            (
                "https://proxy.test/tenant",
                "https://proxy.test/tenant/api/v1/models?output_modalities=all",
            ),
        ] {
            assert_eq!(models_endpoint(base).unwrap(), expected);
        }
    }

    #[test]
    fn endpoint_and_pricing_reject_invalid_boundary_values() {
        assert_eq!(
            models_endpoint("https://openrouter.ai").unwrap(),
            "https://openrouter.ai/api/v1/models?output_modalities=all"
        );
        assert!(models_endpoint("file:///tmp/openrouter").is_err());
        assert_eq!(parse_nonnegative_price("prompt", Some("-1")).unwrap(), None);
        assert_eq!(
            parse_nonnegative_price("completion", Some("-1.0")).unwrap(),
            None
        );
        assert_eq!(
            parse_nonnegative_price("prompt", Some("0")).unwrap(),
            Some(0.0)
        );
        let partial = parse_pricing(Some(ModelPricing {
            prompt: Some("-1".into()),
            completion: Some("0.000002".into()),
        }))
        .unwrap()
        .unwrap();
        assert_eq!(partial.input_token, None);
        assert_eq!(partial.output_token, Some(0.000002));
        assert!(parse_nonnegative_price("prompt", Some("-0.1")).is_err());
        assert!(parse_nonnegative_price("prompt", Some("-2")).is_err());
        assert!(parse_nonnegative_price("prompt", Some("invalid")).is_err());
        assert!(parse_nonnegative_price("prompt", Some("NaN")).is_err());
    }
}
