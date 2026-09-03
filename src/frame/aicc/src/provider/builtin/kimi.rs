use super::super::{
    validate_discovery, CredentialDescriptor, DiscoveredModel, DiscoveryContext, DiscoveryMode,
    ModelAvailability, ProviderConnectionContract, ProviderDiscovery, ProviderDiscoverySnapshot,
    ProviderError, ProviderFieldSchema, ProviderHealthState, ProviderProfile, ProviderResult,
    RefreshPolicy,
};
use crate::catalog::{KnownProvider, ProviderPatternRule, ProviderRulesCatalog};
use crate::matching::MatchRule;
use crate::protocol::{
    CredentialKind, HttpRequest, HttpResponse, HttpTransport, KIMI_CHAT_ADAPTER_ID,
    OPENAI_CHAT_COMPLETIONS_OPERATION_ID,
};
use async_trait::async_trait;
use buckyos_api::{features, ApiType};
use reqwest::header::ETAG;
use reqwest::{Method, Url};
use serde::Deserialize;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

pub(crate) const KIMI_PROVIDER_PROFILE_ID: &str = "kimi";
pub(crate) const KIMI_DISPLAY_NAME: &str = "Moonshot Kimi";
pub(crate) const KIMI_DEFAULT_BASE_URL: &str = "https://api.moonshot.ai/v1";

const MODELS_RESPONSE_LIMIT: usize = 8 * 1024 * 1024;

pub(crate) fn kimi_profile() -> ProviderProfile {
    ProviderProfile {
        provider_profile_id: KIMI_PROVIDER_PROFILE_ID.to_owned(),
        display_name: KIMI_DISPLAY_NAME.to_owned(),
        default_protocol_adapter_id: KIMI_CHAT_ADAPTER_ID.to_owned(),
        credential: CredentialDescriptor {
            kind: CredentialKind::Bearer,
            header_name: None,
        },
        discovery_mode: DiscoveryMode::MachineApi,
        refresh: RefreshPolicy::default(),
        default_inventory: None,
    }
}

pub(crate) fn kimi_connection_contract() -> ProviderConnectionContract {
    ProviderConnectionContract {
        default_base_url: KIMI_DEFAULT_BASE_URL.to_owned(),
        region: ProviderFieldSchema::unsupported(),
        workspace: ProviderFieldSchema::unsupported(),
        account: ProviderFieldSchema::unsupported(),
    }
}

pub(crate) fn kimi_known_provider() -> KnownProvider {
    KnownProvider {
        provider_profile_id: KIMI_PROVIDER_PROFILE_ID.to_owned(),
        display_name: KIMI_DISPLAY_NAME.to_owned(),
        base_url: KIMI_DEFAULT_BASE_URL.to_owned(),
        protocol_adapter_id: KIMI_CHAT_ADAPTER_ID.to_owned(),
        provider_rules_id: Some(KIMI_PROVIDER_PROFILE_ID.to_owned()),
        ui_hints: BTreeMap::from([
            (
                "credential".to_owned(),
                json!({"kind": "bearer", "required": true, "secret": true}),
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

pub(crate) fn kimi_provider_rules(revision_seq: u64) -> ProviderRulesCatalog {
    ProviderRulesCatalog {
        format: "buckyos.aicc.provider-rules-catalog".to_owned(),
        schema_version: 1,
        schema_revision: 0,
        revision_seq,
        provider_profile_id: KIMI_PROVIDER_PROFILE_ID.to_owned(),
        metadata_drivers: Some(vec![KIMI_PROVIDER_PROFILE_ID.to_owned()]),
        origin_provider_aliases: BTreeMap::new(),
        origin_mappings: Vec::new(),
        models: Vec::new(),
        patterns: vec![ProviderPatternRule {
            match_rule: MatchRule::Shorthand("*".to_owned()),
            exclude: false,
            operations: BTreeMap::from([(
                "llm".to_owned(),
                OPENAI_CHAT_COMPLETIONS_OPERATION_ID.to_owned(),
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

#[async_trait]
trait KimiModelsTransport: Send + Sync {
    async fn send(
        &self,
        request: HttpRequest,
    ) -> crate::protocol::ProtocolResultValue<HttpResponse>;
}

#[async_trait]
impl KimiModelsTransport for HttpTransport {
    async fn send(
        &self,
        request: HttpRequest,
    ) -> crate::protocol::ProtocolResultValue<HttpResponse> {
        HttpTransport::send(self, request).await
    }
}

#[derive(Clone)]
pub(crate) struct KimiDiscovery {
    transport: Arc<dyn KimiModelsTransport>,
}

impl KimiDiscovery {
    pub(crate) fn new(transport: HttpTransport) -> Self {
        Self {
            transport: Arc::new(transport),
        }
    }

    #[cfg(test)]
    fn with_transport(transport: Arc<dyn KimiModelsTransport>) -> Self {
        Self { transport }
    }
}

#[async_trait]
impl ProviderDiscovery for KimiDiscovery {
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
        let revision = response
            .headers
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let wire: ModelsResponse = serde_json::from_slice(&response.body).map_err(|error| {
            ProviderError::Discovery(format!("Kimi models response is invalid: {error}"))
        })?;
        if wire.object != "list" {
            return Err(ProviderError::Discovery(
                "Kimi models response must be a list".to_owned(),
            ));
        }
        let mut models = BTreeMap::new();
        for model in wire.data {
            if model.object != "model" || model.id.trim().is_empty() || model.id.contains('@') {
                return Err(ProviderError::Discovery(
                    "Kimi Models API returned an invalid model object".to_owned(),
                ));
            }
            let mut supported_features = BTreeSet::from([
                features::TOOL_CALL.to_owned(),
                features::JSON_SCHEMA.to_owned(),
            ]);
            if model.supports_image_in.unwrap_or(false) || model.supports_video_in.unwrap_or(false)
            {
                supported_features.insert(features::VISION.to_owned());
            }
            if model.supports_reasoning.unwrap_or(false) {
                supported_features.insert("reasoning".to_owned());
            }
            models.insert(
                model.id.clone(),
                DiscoveredModel {
                    provider_model_id: model.id,
                    origin_model_id: None,
                    api_types: Some(vec![ApiType::Llm]),
                    supported_features: Some(supported_features),
                    remote_methods: Some(BTreeSet::from([
                        OPENAI_CHAT_COMPLETIONS_OPERATION_ID.to_owned()
                    ])),
                    availability: ModelAvailability::Available,
                    deprecated: false,
                    pricing: None,
                },
            );
        }
        let snapshot = ProviderDiscoverySnapshot {
            revision,
            discovered_at_ms: super::super::now_ms()?,
            health: ProviderHealthState::Healthy,
            models: models.into_values().collect(),
        };
        validate_discovery(&snapshot)?;
        Ok(snapshot)
    }
}

fn validate_context(context: &DiscoveryContext<'_>) -> ProviderResult<()> {
    if context.profile.provider_profile_id != KIMI_PROVIDER_PROFILE_ID
        || context.profile.default_protocol_adapter_id != KIMI_CHAT_ADAPTER_ID
        || context.instance.provider_profile_id != KIMI_PROVIDER_PROFILE_ID
        || context.instance.protocol_adapter_id != KIMI_CHAT_ADAPTER_ID
    {
        return Err(ProviderError::InvalidConfiguration(
            "Kimi discovery requires its builtin profile and adapter".to_owned(),
        ));
    }
    if context.credential.audit().kind != CredentialKind::Bearer {
        return Err(ProviderError::Credential(
            "Kimi discovery requires a Bearer credential".to_owned(),
        ));
    }
    if context.instance.region.is_some() || context.instance.account.is_some() {
        return Err(ProviderError::InvalidConfiguration(
            "Kimi profile does not accept region or account".to_owned(),
        ));
    }
    Ok(())
}

fn models_endpoint(base_url: &str) -> ProviderResult<String> {
    let mut url = Url::parse(base_url)
        .map_err(|_| ProviderError::InvalidConfiguration("Kimi base_url is invalid".to_owned()))?;
    if !matches!(url.scheme(), "http" | "https") || url.cannot_be_a_base() {
        return Err(ProviderError::InvalidConfiguration(
            "Kimi base_url must be an absolute HTTP URL".to_owned(),
        ));
    }
    let path = url.path().trim_end_matches('/');
    let prefix = if path.ends_with("/v1") {
        path.to_owned()
    } else if path.is_empty() {
        "/v1".to_owned()
    } else {
        format!("{path}/v1")
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
    Err(ProviderError::Discovery(format!(
        "Kimi models request failed with status {} (request {})",
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
    object: String,
    supports_image_in: Option<bool>,
    supports_video_in: Option<bool>,
    supports_reasoning: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{ProtocolError, ResolvedCredential};
    use crate::provider::{CredentialReference, ProviderInstanceConfig};
    use bytes::Bytes;
    use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
    use reqwest::StatusCode;
    use std::sync::Mutex;

    struct FakeTransport {
        request: Mutex<Option<HttpRequest>>,
        response: Mutex<Option<Result<HttpResponse, ProtocolError>>>,
    }

    #[async_trait]
    impl KimiModelsTransport for FakeTransport {
        async fn send(
            &self,
            request: HttpRequest,
        ) -> crate::protocol::ProtocolResultValue<HttpResponse> {
            *self.request.lock().unwrap() = Some(request);
            self.response.lock().unwrap().take().unwrap()
        }
    }

    fn instance() -> ProviderInstanceConfig {
        ProviderInstanceConfig {
            provider_instance_name: "kimi-main".to_owned(),
            provider_profile_id: KIMI_PROVIDER_PROFILE_ID.to_owned(),
            protocol_adapter_id: KIMI_CHAT_ADAPTER_ID.to_owned(),
            base_url: KIMI_DEFAULT_BASE_URL.to_owned(),
            credential: CredentialReference {
                reference: "secret://kimi".to_owned(),
            },
            provider_rules_id: Some(KIMI_PROVIDER_PROFILE_ID.to_owned()),
            region: None,
            account: None,
        }
    }

    #[tokio::test]
    async fn profile_rules_and_machine_discovery_are_stable() {
        let transport = Arc::new(FakeTransport {
            request: Mutex::new(None),
            response: Mutex::new(Some(Ok(HttpResponse {
                status: StatusCode::OK,
                headers: HeaderMap::from_iter([(
                    ETAG,
                    HeaderValue::from_static("models-1"),
                )]),
                body: Bytes::from_static(
                    br#"{"object":"list","data":[{"id":"kimi-model","object":"model","supports_image_in":true,"supports_video_in":false,"supports_reasoning":true}]}"#,
                ),
                request_id: "request-1".to_owned(),
                retry_after: None,
            }))),
        });
        let discovery = KimiDiscovery::with_transport(transport.clone());
        let profile = kimi_profile();
        let instance = instance();
        let credential = ResolvedCredential::bearer("secret://kimi", "secret").unwrap();
        let snapshot = discovery
            .discover(&DiscoveryContext {
                profile: &profile,
                instance: &instance,
                credential: &credential,
            })
            .await
            .unwrap();
        assert_eq!(snapshot.revision.as_deref(), Some("models-1"));
        assert_eq!(snapshot.models.len(), 1);
        assert!(snapshot.models[0]
            .supported_features
            .as_ref()
            .unwrap()
            .contains(features::VISION));
        let request = transport.request.lock().unwrap().take().unwrap();
        assert_eq!(request.url, "https://api.moonshot.ai/v1/models");
        assert_eq!(request.headers[AUTHORIZATION], "Bearer secret");
        assert_eq!(
            kimi_provider_rules(7).patterns[0].operations["llm"],
            OPENAI_CHAT_COMPLETIONS_OPERATION_ID
        );
        assert_eq!(kimi_known_provider().base_url, KIMI_DEFAULT_BASE_URL);
    }

    #[test]
    fn endpoint_and_http_errors_are_explicit() {
        assert_eq!(
            models_endpoint("https://api.moonshot.ai").unwrap(),
            "https://api.moonshot.ai/v1/models"
        );
        assert!(models_endpoint("relative/path").is_err());
        assert!(ensure_success(&HttpResponse {
            status: StatusCode::UNAUTHORIZED,
            headers: HeaderMap::new(),
            body: Bytes::new(),
            request_id: "request-denied".to_owned(),
            retry_after: None,
        })
        .is_err());
    }
}
