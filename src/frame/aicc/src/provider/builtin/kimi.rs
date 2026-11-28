#[cfg(test)]
use super::super::{DiscoveryMode, ProviderProfile};
#[cfg(test)]
use crate::catalog::{CurrentCatalogFile, ModelDriverCatalog, ProviderRulesCatalog};
#[cfg(test)]
use buckyos_api::features;

#[cfg(test)]
pub(crate) const KIMI_PROVIDER_PROFILE_ID: &str = "kimi";

#[cfg(test)]
pub(crate) fn kimi_profile() -> ProviderProfile {
    super::builtin_profile(KIMI_PROVIDER_PROFILE_ID, DiscoveryMode::MachineApi)
}

#[cfg(test)]
pub(crate) fn kimi_known_provider() -> crate::catalog::KnownProvider {
    super::builtin_known_provider(KIMI_PROVIDER_PROFILE_ID)
}

#[cfg(test)]
pub(crate) fn kimi_provider_rules(_revision_seq: u64) -> ProviderRulesCatalog {
    super::builtin_provider_rules(KIMI_PROVIDER_PROFILE_ID)
}

#[cfg(test)]
pub(crate) fn kimi_model_driver() -> ModelDriverCatalog {
    super::builtin_model_driver(KIMI_PROVIDER_PROFILE_ID)
}

#[cfg(test)]
pub(crate) fn kimi_catalog_files() -> Vec<CurrentCatalogFile> {
    super::builtin_catalog_files(&[KIMI_PROVIDER_PROFILE_ID])
}

#[cfg(test)]
mod tests {
    use super::super::openai_responses_compatible::{
        OpenAiCompatibleModelsDiscovery, OpenAiCompatibleModelsTransport,
    };
    use super::*;
    use crate::protocol::{HttpRequest, HttpResponse};
    use crate::protocol::{ProtocolError, ResolvedCredential};
    use crate::provider::{CredentialReference, ProviderInstanceConfig};
    use crate::provider::{DiscoveryContext, ProviderDiscovery};
    use async_trait::async_trait;
    use bytes::Bytes;
    use reqwest::header::ETAG;
    use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
    use reqwest::Method;
    use reqwest::StatusCode;
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::time::Duration;

    struct FakeTransport {
        request: Mutex<Option<HttpRequest>>,
        response: Mutex<Option<Result<HttpResponse, ProtocolError>>>,
    }

    #[async_trait]
    impl OpenAiCompatibleModelsTransport for FakeTransport {
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
            protocol_adapter_id: crate::protocol::LEGACY_OPENAI_CHAT_COMPLETIONS_ADAPTER_ID
                .to_owned(),
            base_url: kimi_known_provider().base_url,
            operation_base_urls: BTreeMap::new(),
            credential: CredentialReference {
                reference: "secret://kimi".to_owned(),
            },
            credential_kind: None,
            provider_rules_id: Some(KIMI_PROVIDER_PROFILE_ID.to_owned()),
            region: Some("global".to_owned()),
            workspace: None,
            account: None,
            request_timeout: Duration::from_secs(120),
            auto_sync_models: true,
            instance_rules: None,
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
        let discovery = OpenAiCompatibleModelsDiscovery::with_transport(false, transport.clone());
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
        assert!(!snapshot.models[0]
            .unsupported_features
            .contains(features::VISION));
        assert!(snapshot.models[0].supported_features.is_none());
        let request = transport.request.lock().unwrap().take().unwrap();
        assert_eq!(request.url, "https://api.moonshot.ai/v1/models");
        assert_eq!(request.headers[AUTHORIZATION], "Bearer secret");
        assert_eq!(
            kimi_provider_rules(7)
                .patterns
                .iter()
                .find_map(|rule| rule.operations.get("llm")),
            Some(&OPENAI_CHAT_COMPLETIONS_OPERATION_ID.to_owned())
        );
        assert!(kimi_provider_rules(7).static_inventory_models.is_empty());
        assert_eq!(kimi_known_provider().base_url, "https://api.moonshot.ai/v1");
        let known = kimi_known_provider();
        assert_eq!(
            known.ui_hints["instance_fields"]["region"]["mode"],
            "required"
        );
        assert!(known.ui_hints["instance_fields"]["region"]
            .get("default_value")
            .is_none());
        assert_eq!(
            known.ui_hints["access_endpoints"]["china"]["label"],
            "China"
        );
    }

    #[tokio::test]
    async fn http_errors_are_explicit() {
        let transport = FakeTransport {
            request: Mutex::new(None),
            response: Mutex::new(Some(Ok(HttpResponse {
                status: StatusCode::UNAUTHORIZED,
                headers: HeaderMap::new(),
                body: Bytes::new(),
                request_id: "request-denied".to_owned(),
                retry_after: None,
            }))),
        };
        assert!(
            super::super::openai_responses_compatible::discover_model_ids(
                &transport,
                HttpRequest::new(Method::GET, "https://example.com/models"),
                KIMI_PROVIDER_PROFILE_ID,
                false
            )
            .await
            .is_err()
        );
    }
}

#[cfg(test)]
use crate::protocol::OPENAI_CHAT_COMPLETIONS_OPERATION_ID;
