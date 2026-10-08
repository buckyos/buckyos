use super::*;
use crate::catalog::{
    CatalogBuildOptions, CatalogSnapshot, ModelIdentity, ModelMatchFailure, ProviderModelMatch,
};
use crate::protocol::*;
use crate::provider::inventory::ModelIdentitySource;
use crate::provider::*;
use crate::settings::{load_builtin_metadata, MetadataSources};
use async_trait::async_trait;
use buckyos_api::{AiMessage, AiRole, ApiType, LlmChatInvokeRequest};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;
use std::time::Duration;

fn catalog() -> Arc<CatalogSnapshot> {
    MetadataSources {
        builtin: load_builtin_metadata().unwrap(),
        ..Default::default()
    }
    .build_snapshot(
        crate::settings::BUILTIN_CATALOG_REVISION_SEQ,
        &CatalogBuildOptions::default(),
    )
    .unwrap()
}
fn instance(profile: &ProviderProfile, name: &str) -> ProviderInstanceConfig {
    ProviderInstanceConfig {
        provider_instance_name: name.into(),
        provider_profile_id: profile.provider_profile_id.clone(),
        protocol_adapter_id: profile.default_protocol_adapter_id.clone(),
        base_url: "https://example.test/v1".into(),
        operation_base_urls: BTreeMap::new(),
        credential: CredentialReference {
            reference: "secret://test".into(),
        },
        credential_kind: None,
        provider_rules_id: Some(profile.provider_profile_id.clone()),
        region: None,
        workspace: None,
        account: None,
        request_timeout: Duration::from_secs(30),
        auto_sync_models: true,
        instance_rules: None,
    }
}
fn discovery(ids: &[&str]) -> ProviderDiscoverySnapshot {
    ProviderDiscoverySnapshot {
        revision: None,
        discovered_at_ms: 10,
        health: ProviderHealthState::Healthy,
        models: ids
            .iter()
            .map(|id| DiscoveredModel {
                provider_model_id: (*id).into(),
                api_types: None,
                supported_features: None,
                unsupported_features: BTreeSet::new(),
                remote_methods: None,
                availability: ModelAvailability::Available,
                deprecated: false,
                pricing: None,
            })
            .collect(),
    }
}
struct Matcher(ProviderModelMatch);
#[async_trait]
impl ProviderDiscovery for Matcher {
    fn match_model_driver(&self, _: &str, _: &CatalogSnapshot) -> ProviderModelMatch {
        self.0.clone()
    }
    async fn discover(
        &self,
        _: &DiscoveryContext<'_>,
    ) -> ProviderResult<ProviderDiscoverySnapshot> {
        unreachable!()
    }
}
#[test]
fn overrides_provider_failures_and_aliases_are_terminal_and_isolated() {
    let catalog = catalog();
    let registry = builtin_provider_registry(&catalog).unwrap();
    let profile = registry
        .profiles()
        .find(|p| p.provider_profile_id == "openai")
        .unwrap();
    let mut config = instance(profile, "a");
    let codecs = registry.codecs();
    let build = |config: &ProviderInstanceConfig, matcher: Option<&dyn ProviderDiscovery>| {
        InventoryBuilder::build_with_matcher(
            profile,
            config,
            discovery(&["stable/gpt-5-6", "gpt-5.6"]),
            &catalog,
            &codecs,
            matcher,
        )
        .unwrap()
    };
    let first = build(&config, None);
    assert_eq!(first.models.len(), 1);
    assert_eq!(first.unmatched_models[0].reason, ModelMatchFailure::NoMatch);
    config.instance_rules = Some(buckyos_api::ProviderInstanceRules {
        model_driver_overrides: BTreeMap::from([(
            "stable/gpt-5-6".into(),
            "openai/gpt-5.6".into(),
        )]),
        ..Default::default()
    });
    let good = build(&config, None);
    assert_eq!(good.models.len(), 2);
    assert!(format!(
        "{:?}",
        good.models
            .iter()
            .find(|m| m.provider_model_id == "stable/gpt-5-6")
            .unwrap()
            .identity_source
    )
    .contains("InstanceOverride"));
    config
        .instance_rules
        .as_mut()
        .unwrap()
        .model_driver_overrides
        .insert("stable/gpt-5-6".into(), "openai/typo".into());
    assert!(matches!(
        build(&config, None).unmatched_models[0].reason,
        ModelMatchFailure::InvalidOverride { .. }
    ));
    config.instance_rules = None;
    for result in [
        ProviderModelMatch::Matched(ModelIdentity {
            model_driver_id: "openai".into(),
            model_id: "typo".into(),
        }),
        ProviderModelMatch::Failed(ModelMatchFailure::UnresolvedAlias),
    ] {
        let matcher = Matcher(result);
        let inv = build(&config, Some(&matcher));
        assert!(inv.models.is_empty());
        assert_eq!(inv.unmatched_models.len(), 2);
    }
    config.instance_rules = Some(buckyos_api::ProviderInstanceRules {
        exclude_models: BTreeSet::from(["stable/gpt-5-6".into()]),
        ..Default::default()
    });
    assert!(build(&config, None).unmatched_models.is_empty());
    let router =
        openrouter::OpenRouterDiscovery::new(HttpTransport::new(Default::default()).unwrap());
    assert_eq!(
        router.match_model_driver("anthropic/claude-sonnet-5", &catalog),
        ProviderModelMatch::Matched(ModelIdentity {
            model_driver_id: "claude".into(),
            model_id: "claude-sonnet-5".into()
        })
    );
    assert_eq!(
        catalog
            .provider_model_identity_override("deepseek", "deepseek-v4-flash")
            .unwrap(),
        Ok(ModelIdentity {
            model_driver_id: "deepseek".into(),
            model_id: "deepseek-v4.1-flash".into()
        })
    );
    assert_eq!(
        catalog
            .match_model("deepseek-v4-flash")
            .unwrap()
            .model_driver_id,
        "deepseek"
    );
}

#[test]
fn deepseek_inventory_uses_provider_metadata_for_aliases_and_exclusions() {
    let catalog = catalog();
    let providers = builtin_provider_registry(&catalog).unwrap();
    let profile = providers
        .profiles()
        .find(|profile| profile.provider_profile_id == "deepseek")
        .unwrap();
    let inventory = InventoryBuilder::build(
        profile,
        &instance(profile, "deepseek-test"),
        discovery(&[
            "deepseek-flash",
            "deepseek-v4-flash",
            "deepseek-v4-flash-vision-exp",
            "deepseek-v4-pro",
            "deepseek-chat",
            "deepseek-reasoner",
        ]),
        &catalog,
        &providers.codecs(),
    )
    .unwrap();

    assert!(inventory.unmatched_models.is_empty());
    assert_eq!(inventory.models.len(), 4);
    for provider_model_id in [
        "deepseek-flash",
        "deepseek-v4-flash",
        "deepseek-v4-flash-vision-exp",
    ] {
        let model = inventory
            .models
            .iter()
            .find(|model| model.provider_model_id == provider_model_id)
            .unwrap();
        assert_eq!(model.origin_model_id, "deepseek-v4.1-flash");
        assert!(matches!(
            model.identity_source,
            ModelIdentitySource::Provider
        ));
    }
    let pro = inventory
        .models
        .iter()
        .find(|model| model.provider_model_id == "deepseek-v4-pro")
        .unwrap();
    assert_eq!(pro.origin_model_id, "deepseek-v4-pro");
    assert!(matches!(pro.identity_source, ModelIdentitySource::Catalog));
    assert!(inventory.models.iter().all(|model| !matches!(
        model.provider_model_id.as_str(),
        "deepseek-chat" | "deepseek-reasoner"
    )));
}

#[tokio::test]
async fn builtin_presets_share_inventory_registry_and_wire_contracts() {
    use crate::call::{CallResolver, ProviderCallTarget};
    use crate::routing::policy::*;
    use crate::routing::{CandidateRuntimeState, ProviderHealthStatus, Router, RoutingRequest};
    struct Quota;
    impl QuotaSource for Quota {
        fn query(&self, _: &QuotaLookup) -> Result<QuotaSnapshot, QuotaSourceError> {
            Ok(QuotaSnapshot {
                state: Some(buckyos_api::QuotaState::Normal),
                remaining_request_units: None,
                remaining_cost: None,
                reset_at: None,
            })
        }
    }
    let catalog = catalog();
    let providers = builtin_provider_registry(&catalog).unwrap();
    let codecs = providers.codecs();
    for (provider, id, effort, pointer, expected) in [
        (
            "qwen",
            "qwen3.8-2.4t-a95b",
            "low",
            "/reasoning/effort",
            json!("low"),
        ),
        (
            "qwen",
            "qwen3.7-plus",
            "thinking",
            "/reasoning/effort",
            json!("xhigh"),
        ),
        (
            "qwen",
            "qwen3.5-27b",
            "thinking",
            "/reasoning/effort",
            json!("xhigh"),
        ),
        (
            "kimi",
            "kimi-k2.6",
            "medium",
            "/thinking/type",
            json!("enabled"),
        ),
        (
            "kimi",
            "kimi-k2.7-code",
            "medium",
            "/thinking/keep",
            json!("all"),
        ),
        (
            "kimi",
            "kimi-k3",
            "high",
            "/reasoning_effort",
            json!("high"),
        ),
        (
            "glm",
            "glm-4.6",
            "thinking",
            "/thinking/type",
            json!("enabled"),
        ),
        (
            "claude",
            "claude-haiku-4-5-20251001",
            "thinking",
            "/thinking/type",
            json!("enabled"),
        ),
        (
            "claude",
            "claude-haiku-4-5-20251001",
            "none",
            "/thinking/type",
            json!("disabled"),
        ),
        (
            "claude",
            "claude-opus-5-5",
            "xhigh",
            "/output_config/effort",
            json!("xhigh"),
        ),
        (
            "claude",
            "claude-sonnet-4-6",
            "max",
            "/thinking/type",
            json!("adaptive"),
        ),
        (
            "claude",
            "claude-opus-4-5-20251101",
            "thinking",
            "/thinking/budget_tokens",
            json!(1024),
        ),
        (
            "gemini",
            "gemini-3.1-flash-lite",
            "medium",
            "/generation_config/thinking_level",
            json!("medium"),
        ),
        (
            "gemini",
            "gemini-3.1-flash-lite",
            "minimal",
            "/generation_config/thinking_level",
            json!("minimal"),
        ),
        (
            "openai",
            "gpt-6-astra",
            "max",
            "/reasoning/effort",
            json!("max"),
        ),
    ] {
        let profile = providers
            .profiles()
            .find(|p| p.provider_profile_id == provider)
            .unwrap();
        let config = instance(profile, "test");
        let mut discovered = discovery(&[id]);
        if matches!(provider, "openai" | "kimi") {
            let transport =
                ModelsHttp(json!({"object":"list","data":[{"object":"model","id":id}]}));
            let (models, _) = openai_responses_compatible::discover_model_ids(
                &transport,
                HttpRequest::new(reqwest::Method::GET, "https://example.test/v1/models"),
                provider,
                true,
            )
            .await
            .unwrap();
            discovered.models = models;
        }
        let inv = InventoryBuilder::build(profile, &config, discovered, &catalog, &codecs).unwrap();
        let model = inv
            .models
            .iter()
            .find(|m| m.provider_model_id == id)
            .unwrap();
        assert!(
            model
                .variants
                .iter()
                .any(|v| v.name == format!("reasoning-{effort}")),
            "{provider}/{id}/{effort}: {:?}",
            inv.unavailable_presets
        );
        if id == "qwen3.8-2.4t-a95b" {
            assert_eq!(
                model
                    .variants
                    .iter()
                    .map(|v| v.name.as_str())
                    .collect::<Vec<_>>(),
                ["reasoning-low", "reasoning-medium", "reasoning-xhigh"]
            );
        }
        let models =
            crate::service::builtin_registry_for_test(&catalog, &[inv.as_model_inventory()]);
        let exact = format!("{id}:reasoning-{effort}@test");
        let caller = CallerIdentity {
            tenant_id: "tenant".into(),
            user_id: "user".into(),
            app_id: None,
        };
        let request = RoutingRequest::new("trace", "request", &exact, ApiType::Llm, caller);
        let runtime = models
            .model_views()
            .into_iter()
            .map(|m| {
                (
                    m.exact_model,
                    CandidateRuntimeState {
                        enabled: true,
                        credential_available: true,
                        model_available: true,
                        health: ProviderHealthStatus::Available,
                        provider_privacy: ProviderPrivacy::PublicCloud,
                        trust: Some(ProviderTrustView {
                            provider_type: ProviderType::CloudApi,
                            provider_type_source: ProviderTypeSource::SystemConfig,
                            provider_type_revision: "test".into(),
                            asserted_at_ms: 1,
                            trust_level: ProviderTrustLevel::Verified,
                        }),
                        credential_scope: CredentialScope::Tenant {
                            tenant_id: "tenant".into(),
                        },
                        estimated_cost: None,
                        p50_latency_ms: None,
                        p95_latency_ms: None,
                        error_rate_5m: None,
                        recent_failures: 0,
                        cache_hit_probability: None,
                    },
                )
            })
            .collect();
        let policy = PolicyEngine::new(
            EffectiveRoutingPolicy::merge(RoutingPolicyLayers {
                system: None,
                user: None,
                app: None,
                session: None,
                request: None,
            })
            .unwrap(),
            Quota,
        )
        .unwrap();
        let route = Router::new(&models, &policy, &runtime)
            .route(&request)
            .unwrap();
        // The output cap is optional in the canonical request; protocols that
        // require one get a default, the others must not grow one.
        for max_output_tokens in [Some(2048), None] {
            let mut request =
                LlmChatInvokeRequest::new(&exact, vec![AiMessage::text(AiRole::User, "hello")]);
            request.max_output_tokens = max_output_tokens;
            let call = buckyos_api::AiccCall::ChatCompletionsCreate(request);
            let credential = if provider == "claude" {
                ResolvedCredential::named_header("secret://test", "x-api-key", "secret").unwrap()
            } else if provider == "gemini" {
                ResolvedCredential::named_header("secret://test", "x-goog-api-key", "secret")
                    .unwrap()
            } else {
                ResolvedCredential::bearer("secret://test", "secret").unwrap()
            };
            let target = ProviderCallTarget {
                provider_rules_id: Some(provider.into()),
                base_url: config.base_url.clone(),
                operation_base_urls: BTreeMap::new(),
                credential,
                credential_reference: "secret://test".into(),
                credential_header_name: None,
                limits: CodecLimits {
                    request_timeout: Duration::from_secs(30),
                    max_request_bytes: 1024 * 1024,
                    max_response_bytes: 1024 * 1024,
                },
                pricing: None,
                match_dimensions: Default::default(),
            };
            let lowered = CallResolver::new(&catalog, &codecs)
                .lower(&route, &call, target)
                .unwrap();
            let wire = codecs
                .encode(
                    &lowered.protocol_adapter_id,
                    &lowered.operation,
                    lowered.api_type,
                    &lowered.input,
                    &lowered.context,
                )
                .unwrap();
            let HttpBody::Json(body) = wire.body else {
                panic!("expected JSON")
            };
            assert_eq!(
                body.pointer(pointer),
                Some(&expected),
                "{provider}/{id}/{effort}: {body}"
            );
            if provider == "claude" {
                assert_eq!(
                    body["max_tokens"],
                    json!(max_output_tokens.unwrap_or(32_000)),
                    "{provider}/{id}/{effort}: {body}"
                );
            } else if max_output_tokens.is_none() {
                let buckyos_api::AiccCall::ChatCompletionsCreate(lowered_request) =
                    &lowered.input.canonical_request
                else {
                    panic!("expected chat call")
                };
                assert_eq!(lowered_request.max_output_tokens, None, "{provider}/{id}");
            }
        }
    }
}

struct ModelsHttp(Value);
#[async_trait]
impl openai_responses_compatible::OpenAiCompatibleModelsTransport for ModelsHttp {
    async fn send(&self, _: HttpRequest) -> ProtocolResultValue<HttpResponse> {
        Ok(HttpResponse {
            status: reqwest::StatusCode::OK,
            headers: Default::default(),
            body: bytes::Bytes::from(serde_json::to_vec(&self.0).unwrap()),
            request_id: "models-test".into(),
            retry_after: None,
        })
    }
}

#[tokio::test]
async fn shared_discovery_preserves_unknowns_and_explicit_channel_restrictions() {
    let request = || HttpRequest::new(reqwest::Method::GET, "https://example.test/v1/models");
    let body = json!({"object":"list","data":[{"object":"model","id":"kimi-k2.6","supports_reasoning":false,"supports_image_in":false,"supports_video_in":false},{"object":"model","id":"unknown"}]});
    let (models, _) =
        openai_responses_compatible::discover_model_ids(&ModelsHttp(body), request(), "kimi", true)
            .await
            .unwrap();
    assert!(models[0].unsupported_features.contains("reasoning"));
    assert_eq!(models[0].api_types, Some(vec![ApiType::Llm]));
    assert!(models[1].api_types.is_none());
    assert!(models[1].unsupported_features.is_empty());
    let catalog = catalog();
    let providers = builtin_provider_registry(&catalog).unwrap();
    let profile = providers
        .profiles()
        .find(|p| p.provider_profile_id == "kimi")
        .unwrap();
    let mut discovered = discovery(&[]);
    discovered.models = models;
    let inv = InventoryBuilder::build(
        profile,
        &instance(profile, "k"),
        discovered,
        &catalog,
        &providers.codecs(),
    )
    .unwrap();
    assert_eq!(inv.unmatched_models.len(), 1);
    assert!(inv.models[0]
        .variants
        .iter()
        .all(|v| v.name == "reasoning-none"));
    for body in [
        json!({"object":"list"}),
        json!({"object":"list","data":[{"object":"model","id":"same"},{"object":"model","id":"same"}]}),
        json!({"object":"list","data":[{"object":"model","id":"bad@id"}]}),
    ] {
        assert!(matches!(
            openai_responses_compatible::discover_model_ids(
                &ModelsHttp(body),
                request(),
                "kimi",
                true
            )
            .await,
            Err(ProviderError::DiscoveryResponse(_))
        ));
    }
}

#[tokio::test]
async fn volcengine_ark_discovery_uses_catalog_lifecycle_and_task_types() {
    let body = json!({"object":"list","data":[
        {"object":"model","id":"active-vlm","task_type":["TextGeneration","VisualQuestionAnswering","SpeechToText"]},
        {"object":"model","id":"active-video","task_type":["MultimodalToVideo","VideoEditing","VideoExtension"]},
        {"object":"model","id":"retiring","status":"Retiring","task_type":["TextGeneration"]},
        {"object":"model","id":"unsupported-3d","task_type":["ImageTo3D"]}
    ]});
    let (models, _) = openai_responses_compatible::discover_volcengine_ark_models(
        &ModelsHttp(body),
        HttpRequest::new(reqwest::Method::GET, "https://example.test/api/v3/models"),
        "doubao",
    )
    .await
    .unwrap();
    let model = |id: &str| {
        models
            .iter()
            .find(|model| model.provider_model_id == id)
            .unwrap()
    };
    assert_eq!(
        model("active-vlm").api_types.clone(),
        Some(vec![
            ApiType::Llm,
            ApiType::VisionOcr,
            ApiType::VisionCaption
        ])
    );
    assert_eq!(
        model("active-video").api_types.clone(),
        Some(vec![
            ApiType::VideoTextToVideo,
            ApiType::VideoImageToVideo,
            ApiType::VideoToVideo,
            ApiType::VideoExtend,
        ])
    );
    assert_eq!(
        model("retiring").availability,
        ModelAvailability::Unavailable
    );
    assert!(model("retiring").deprecated);
    assert_eq!(model("unsupported-3d").api_types.clone(), Some(vec![]));
}

#[test]
fn claude_account_models_are_matched_with_every_declared_preset() {
    let catalog = catalog();
    let providers = builtin_provider_registry(&catalog).unwrap();
    let codecs = providers.codecs();
    let profile = providers
        .profiles()
        .find(|p| p.provider_profile_id == "claude")
        .unwrap();
    let ids = [
        "claude-fable-5-1",
        "claude-fable-5",
        "claude-opus-5-5",
        "claude-opus-5",
        "claude-opus-4-8",
        "claude-opus-4-7",
        "claude-opus-4-6",
        "claude-opus-4-5-20251101",
        "claude-sonnet-5",
        "claude-sonnet-4-6",
        "claude-sonnet-4-5-20250929",
        "claude-haiku-4-5-20251001",
    ];
    let inv = InventoryBuilder::build(
        profile,
        &instance(profile, "c"),
        discovery(&ids),
        &catalog,
        &codecs,
    )
    .unwrap();
    assert!(
        inv.unmatched_models.is_empty(),
        "{:?}",
        inv.unmatched_models
    );
    assert!(
        inv.unavailable_presets.is_empty(),
        "{:?}",
        inv.unavailable_presets
    );
    assert_eq!(inv.models.len(), ids.len());
    assert!(inv
        .models
        .iter()
        .all(|m| m.pricing.is_some() && !m.variants.is_empty()));
}

#[test]
fn invalid_exact_and_cached_presets_are_rejected_and_unmapped_presets_diagnosed() {
    let catalog = catalog();
    let providers = builtin_provider_registry(&catalog).unwrap();
    let codecs = providers.codecs();
    let profile = providers
        .profiles()
        .find(|p| p.provider_profile_id == "qwen")
        .unwrap();
    let inv = InventoryBuilder::build(
        profile,
        &instance(profile, "q"),
        discovery(&["qwen3.8-2.4t-a95b"]),
        &catalog,
        &codecs,
    )
    .unwrap();
    let registry = crate::service::builtin_registry_for_test(&catalog, &[inv.as_model_inventory()]);
    assert!(registry
        .resolve_candidates("qwen3.8-2.4t-a95b:reasoning-high@q", ApiType::Llm)
        .is_err());
    let mut cached: ProviderInventorySnapshot =
        serde_json::from_value(serde_json::to_value(inv).unwrap()).unwrap();
    cached.models[0]
        .variants
        .push(crate::model::InventoryModelVariant {
            name: "reasoning-high".into(),
            logical_mounts: vec![],
        });
    assert!(crate::model::ModelRegistry::build(
        &catalog,
        &[cached.as_model_inventory()],
        vec![],
        Default::default()
    )
    .is_err());
    let profile = providers
        .profiles()
        .find(|p| p.provider_profile_id == "gemini")
        .unwrap();
    let inv = InventoryBuilder::build(
        profile,
        &instance(profile, "g"),
        discovery(&["gemini-2.5-flash"]),
        &catalog,
        &codecs,
    )
    .unwrap();
    assert_eq!(
        inv.models[0]
            .variants
            .iter()
            .map(|variant| variant.name.as_str())
            .collect::<Vec<_>>(),
        vec!["reasoning-high", "reasoning-low", "reasoning-medium"]
    );
    assert!(inv.unavailable_presets.is_empty());
    assert!(inv.unmatched_models.is_empty());
}

#[test]
fn glm_supplements_use_effective_catalog_and_never_resurrect_dynamic_llms() {
    use crate::catalog::CatalogKind;
    use crate::settings::{MetadataFile, MetadataSource};
    let mut rules: Value = builtin_catalog_document(CatalogKind::ProviderRules, "glm");
    rules["static_inventory_models"] = json!(["glm-image", "glm-5.1"]);
    rules["models"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":"glm-image","exclude":true}));
    // Replace the existing entry, keeping exact ids unique.
    let mut seen = BTreeSet::new();
    rules["models"].as_array_mut().unwrap().reverse();
    rules["models"]
        .as_array_mut()
        .unwrap()
        .retain(|m| seen.insert(m["id"].as_str().unwrap().to_owned()));
    let catalog = MetadataSources {
        builtin: load_builtin_metadata().unwrap(),
        system_config: vec![MetadataFile::parse(
            MetadataSource::SystemConfig,
            CatalogKind::ProviderRules,
            serde_json::to_vec(&rules).unwrap(),
        )
        .unwrap()],
        ..Default::default()
    }
    .build_snapshot(
        crate::settings::BUILTIN_CATALOG_REVISION_SEQ,
        &Default::default(),
    )
    .unwrap();
    let providers = builtin_provider_registry(&catalog).unwrap();
    let profile = providers
        .profiles()
        .find(|p| p.provider_profile_id == "glm")
        .unwrap();
    let inv = InventoryBuilder::build(
        profile,
        &instance(profile, "g"),
        discovery(&[]),
        &catalog,
        &providers.codecs(),
    )
    .unwrap();
    assert!(inv.models.is_empty());
    assert!(inv.unmatched_models.is_empty());
}

#[tokio::test]
async fn fal_discovers_traceable_price_quotes_without_inventing_billable_units() {
    let catalog = catalog();
    let providers = builtin_provider_registry(&catalog).unwrap();
    let profile = providers
        .profiles()
        .find(|p| p.provider_profile_id == "fal")
        .unwrap();
    let instance = instance(profile, "fal");
    let discovery = fal::FalPricingDiscovery {
        inventory: Arc::new(CatalogOnlyDiscovery::new(discovery(&["fal-ai/esrgan"]))),
        transport: Arc::new(ModelsHttp(
            json!({"prices":[{"endpoint_id":"fal-ai/esrgan","unit":"image","currency":"USD","unit_price":0.025}],"has_more":false,"next_cursor":null}),
        )),
    };
    let credential = ResolvedCredential::fal_key("secret://test", "secret").unwrap();
    let discovered = discovery
        .discover(&DiscoveryContext {
            profile,
            instance: &instance,
            credential: &credential,
        })
        .await
        .unwrap();
    let price = discovered.models[0].pricing.as_ref().unwrap();
    assert_eq!(price.estimated_cost, Some(0.025));
    assert!(price.source_url.is_some());
    assert!(price.verified_at.is_some());
    assert!(crate::execution::PinnedPricingSnapshot::from_pricing(
        price,
        None,
        std::time::SystemTime::now()
    )
    .unwrap()
    .is_none());
}

#[test]
fn provider_normalization_is_scoped_and_reports_collisions() {
    struct Normalizing;
    #[async_trait]
    impl ProviderDiscovery for Normalizing {
        fn match_model_driver(&self, id: &str, catalog: &CatalogSnapshot) -> ProviderModelMatch {
            let Some(id) = id.strip_prefix("stable/") else {
                return ProviderModelMatch::NotHandled;
            };
            match catalog.find_by_normalized_id(id, |id| id.to_lowercase().replace('.', "-")) {
                Ok(id) => ProviderModelMatch::Matched(id),
                Err(reason) => ProviderModelMatch::Failed(reason),
            }
        }
        async fn discover(
            &self,
            _: &DiscoveryContext<'_>,
        ) -> ProviderResult<ProviderDiscoverySnapshot> {
            unreachable!()
        }
    }
    let catalog = catalog();
    let providers = builtin_provider_registry(&catalog).unwrap();
    let profile = providers
        .profiles()
        .find(|p| p.provider_profile_id == "openai")
        .unwrap();
    let inv = InventoryBuilder::build_with_matcher(
        profile,
        &instance(profile, "o"),
        discovery(&["stable/gpt-5-6"]),
        &catalog,
        &providers.codecs(),
        Some(&Normalizing),
    )
    .unwrap();
    assert_eq!(inv.models[0].origin_model_id, "gpt-5.6");
    let collision=crate::catalog::CatalogSnapshot::build(1,crate::catalog::CatalogDocuments {model_drivers:vec![serde_json::from_value(json!({"format":"buckyos.aicc.model-driver-catalog","schema_version":2,"schema_revision":0,"revision_seq":1,"model_driver_id":"collision","models":[{"id":"x-5.1","api_types":["image.txt2img"]},{"id":"x-5-1","api_types":["image.txt2img"]}],"specs":[]})).unwrap()],..Default::default()},&Default::default()).unwrap();
    assert!(
        matches!(Normalizing.match_model_driver("stable/x-5-1",&collision),ProviderModelMatch::Failed(ModelMatchFailure::Ambiguous{candidates}) if candidates.len()==2)
    );
}

#[test]
fn domestic_prices_do_not_fill_global_regions() {
    let catalog = catalog();
    let providers = builtin_provider_registry(&catalog).unwrap();
    let profile = providers
        .profiles()
        .find(|p| p.provider_profile_id == "glm")
        .unwrap();
    let mut instance = instance(profile, "glm");
    for region in ["china", "global"] {
        instance.region = Some(region.into());
        let inv = InventoryBuilder::build(
            profile,
            &instance,
            discovery(&["glm-5.3"]),
            &catalog,
            &providers.codecs(),
        )
        .unwrap();
        assert!(
            inv.models
                .iter()
                .find(|m| m.provider_model_id == "glm-5.3")
                .unwrap()
                .pricing
                .is_some()
                == (region == "china")
        );
    }
}

#[test]
fn builtin_media_models_mount_their_default_families() {
    let catalog = catalog();
    let providers = builtin_provider_registry(&catalog).unwrap();
    for (provider_id, discovered_id, expected_origin_id, mount, api_type, operation) in [
        (
            "doubao",
            "doubao-seedream-4-0-20260415",
            "doubao-seedream-4.0",
            "image.txt2img.seedream",
            "image.txt2img",
            "ark.images.generate",
        ),
        (
            "doubao-agent-plan",
            "doubao-seedream-5.0-lite",
            "doubao-seedream-5.0-lite",
            "image.txt2img.seedream",
            "image.txt2img",
            "ark.images.generate",
        ),
        (
            "doubao",
            "doubao-seedance-2-5-260628",
            "doubao-seedance-2-5-260628",
            "video.txt2video.seedance",
            "video.txt2video",
            "ark.contents.generate",
        ),
        (
            "doubao-agent-plan",
            "doubao-seedance-2.5",
            "doubao-seedance-2.5",
            "video.img2video.seedance",
            "video.img2video",
            "ark.contents.generate",
        ),
        (
            "minimax",
            "MiniMax-H3",
            "MiniMax-H3",
            "video.txt2video.minimax_h3",
            "video.txt2video",
            "video_generation.v2.create",
        ),
        (
            "glm",
            "vidu2-image",
            "vidu2-image",
            "video.img2video.vidu",
            "video.img2video",
            "videos.generate",
        ),
    ] {
        let profile = providers
            .profiles()
            .find(|profile| profile.provider_profile_id == provider_id)
            .unwrap();
        let inventory = InventoryBuilder::build(
            profile,
            &instance(profile, provider_id),
            discovery(&[discovered_id]),
            &catalog,
            &providers.codecs(),
        )
        .unwrap();
        let model = inventory
            .models
            .iter()
            .find(|model| model.provider_model_id == discovered_id)
            .unwrap();
        assert_eq!(model.origin_model_id, expected_origin_id);
        assert!(model.logical_mounts.contains(&mount.to_owned()));
        assert_eq!(
            model.operations.get(api_type).map(String::as_str),
            Some(operation)
        );
        if provider_id == "doubao-agent-plan" {
            assert!(matches!(
                model.identity_source,
                ModelIdentitySource::Catalog
            ));
        }
    }
}

#[test]
fn agent_plan_inventory_uses_model_specific_official_capabilities() {
    let catalog = catalog();
    let providers = builtin_provider_registry(&catalog).unwrap();
    let profile = providers
        .profiles()
        .find(|profile| profile.provider_profile_id == "doubao-agent-plan")
        .unwrap();
    let inventory = InventoryBuilder::build(
        profile,
        &instance(profile, "doubao-agent-plan"),
        discovery(&[
            "doubao-seed-2.0-mini",
            "deepseek-v4-flash",
            "deepseek-v4.1-flash",
            "glm-5.3",
            "glm-5.3-flash",
            "kimi-k2.7-code",
            "kimi-k2.6",
            "kimi-k2.8-preview",
            "kimi-k3",
            "minimax-m3",
        ]),
        &catalog,
        &providers.codecs(),
    )
    .unwrap();
    let api_types = |model_id: &str| {
        inventory
            .models
            .iter()
            .find(|model| model.provider_model_id == model_id)
            .unwrap()
            .api_types
            .iter()
            .copied()
            .collect::<HashSet<_>>()
    };
    assert!(
        inventory
            .models
            .iter()
            .all(|model| model.provider_model_id != "kimi-k2.6"),
        "unlisted Agent Plan models must fail closed"
    );
    let text_only = HashSet::from([ApiType::Llm]);
    let vision = HashSet::from([ApiType::Llm, ApiType::VisionOcr, ApiType::VisionCaption]);
    for model_id in ["deepseek-v4-flash", "glm-5.3"] {
        assert_eq!(api_types(model_id), text_only, "{model_id}");
    }
    for model_id in [
        "doubao-seed-2.0-mini",
        "deepseek-v4.1-flash",
        "glm-5.3-flash",
        "kimi-k2.7-code",
        "kimi-k2.8-preview",
        "kimi-k3",
        "minimax-m3",
    ] {
        assert_eq!(api_types(model_id), vision, "{model_id}");
    }
}

#[test]
fn provider_pricing_contains_all_rebased_model_defaults() {
    let catalog = catalog();
    for (provider, expected_count) in [
        ("claude", 12),
        ("deepseek", 3),
        ("doubao", 12),
        ("doubao-agent-plan", 0),
        ("fal", 4),
        ("gemini", 29),
        ("glm", 101),
        ("kimi", 4),
        ("minimax", 23),
        ("openai", 28),
        ("qwen", 85),
    ] {
        assert_eq!(
            catalog
                .provider_rules(provider)
                .unwrap()
                .model_pricing
                .len(),
            expected_count,
            "{provider} pricing migration is incomplete"
        );
    }

    let fallback = catalog
        .resolve_provider_rule("openai", "gpt-5.6", &Default::default())
        .unwrap()
        .unwrap()
        .action
        .pricing
        .unwrap();
    assert_eq!(fallback.input_token, Some(4e-6));

    let verified = catalog
        .resolve_provider_rule("openai", "gpt-6-astra", &Default::default())
        .unwrap()
        .unwrap()
        .action
        .pricing
        .unwrap();
    assert_eq!(verified.cache_write_input_token, Some(1.25e-5));
    assert_eq!(
        verified.source_url.as_deref(),
        Some("https://developers.openai.com/api/docs/models/gpt-6-astra")
    );
}

#[test]
fn every_builtin_provider_price_has_provenance() {
    let files = load_builtin_metadata().unwrap();
    let mut pricing_count = 0;
    for file in files
        .iter()
        .filter(|file| file.kind == crate::catalog::CatalogKind::ProviderRules)
    {
        let rules: crate::catalog::ProviderRulesCatalog =
            serde_json::from_slice(&file.contents).unwrap();
        for (index, rule) in rules.model_pricing.iter().enumerate() {
            pricing_count += 1;
            assert!(
                rule.pricing
                    .source_url
                    .as_deref()
                    .is_some_and(|url| url.starts_with("https://")),
                "{} model_pricing[{index}] is missing source_url",
                rules.provider_profile_id
            );
            assert!(
                rule.pricing
                    .verified_at
                    .as_deref()
                    .is_some_and(|date| date.len() == 10),
                "{} model_pricing[{index}] is missing verified_at",
                rules.provider_profile_id
            );
        }
    }
    assert_eq!(pricing_count, 307);
}

#[test]
fn corrected_provider_prices_match_official_billing_dimensions() {
    let catalog = catalog();

    let agent_plan = catalog.provider_rules("doubao-agent-plan").unwrap();
    assert!(agent_plan.model_pricing.is_empty());

    let seedream = catalog
        .resolve_provider_rule(
            "doubao",
            "doubao-seedream-4-0-20260415",
            &Default::default(),
        )
        .unwrap();
    let seedream = seedream.unwrap().action.pricing.unwrap();
    assert_eq!(seedream.unit, Some(crate::catalog::PricingUnit::Image));
    assert_eq!(seedream.amount, Some(0.2));
    let doubao = catalog.provider_rules("doubao").unwrap();
    assert!(doubao
        .model_pricing
        .iter()
        .all(|rule| rule.id.as_deref() != Some("doubao-seed-tts-2.0")));

    let fal = catalog.provider_rules("fal").unwrap();
    let rembg = fal
        .model_pricing
        .iter()
        .find(|rule| rule.id.as_deref() == Some("fal-ai/imageutils/rembg"))
        .unwrap();
    assert_eq!(rembg.pricing.amount, Some(0.0));

    let gemini = catalog.provider_rules("gemini").unwrap();
    let veo = gemini
        .model_pricing
        .iter()
        .find(|rule| rule.id.as_deref() == Some("veo-3.1-generate-preview"))
        .unwrap();
    assert_eq!(
        veo.pricing.unit,
        Some(crate::catalog::PricingUnit::VideoSecond)
    );
    assert_eq!(veo.pricing.amount, Some(0.4));

    let kimi = catalog.provider_rules("kimi").unwrap();
    let k3 = kimi
        .model_pricing
        .iter()
        .find(|rule| rule.pricing.cache_write_input_token == Some(2e-5))
        .unwrap();
    assert_eq!(k3.pricing.cache_write_input_token, Some(2e-5));
    assert_eq!(k3.pricing.cache_write_1h_input_token, Some(4e-5));

    let minimax = catalog.provider_rules("minimax").unwrap();
    let h3 = minimax
        .model_pricing
        .iter()
        .find(|rule| rule.pricing.rules.iter().any(|rule| rule.amount == 0.8))
        .unwrap();
    assert_eq!(h3.pricing.amount, Some(0.5));
    assert!(h3.pricing.rules.iter().any(|rule| rule.amount == 0.8));
    assert_eq!(
        h3.pricing.source_url.as_deref(),
        Some("https://platform.minimaxi.com/docs/guides/pricing-paygo")
    );

    let h3_max = minimax
        .model_pricing
        .iter()
        .find(|rule| rule.pricing.rules.iter().any(|rule| rule.amount == 0.33))
        .unwrap();
    assert!(h3_max.pricing.rules.iter().any(|rule| rule.amount == 0.33));
}

#[test]
fn domestic_currency_never_matches_global_and_minimax_vision_matches_contract() {
    let catalog = catalog();
    for (provider, model) in [
        ("glm", "glm-5.3"),
        ("minimax", "MiniMax-M3"),
        ("kimi", "kimi-k3"),
    ] {
        let context = BTreeMap::from([("region".into(), json!("global"))]);
        let pricing = catalog
            .resolve_provider_rule(provider, model, &context)
            .unwrap()
            .and_then(|rule| rule.action.pricing);
        assert!(
            pricing.is_none_or(|pricing| pricing.currency != "CNY"),
            "{provider}/{model}"
        );
    }
    let registry = builtin_provider_registry(&catalog).unwrap();
    let profile = registry
        .profiles()
        .find(|profile| profile.provider_profile_id == "minimax")
        .unwrap();
    let inventory = InventoryBuilder::build(
        profile,
        &instance(profile, "minimax"),
        discovery(&["MiniMax-M3", "MiniMax-M2.7"]),
        &catalog,
        &registry.codecs(),
    )
    .unwrap();
    for model in &inventory.models {
        assert_eq!(
            model.api_types.contains(&ApiType::VisionCaption),
            model.provider_model_id == "MiniMax-M3"
        );
    }
    assert_eq!(inventory.models.len(), 2);
}

#[test]
fn custom_base_urls_override_inherited_operation_endpoints_and_policy_region_is_validated() {
    let catalog = catalog();
    let registry = builtin_provider_registry(&catalog).unwrap();
    // TTS moved onto its own profile: the standard `doubao` profile now only
    // serves the Ark inference endpoints, and `doubao-agent-plan` is the Ark
    // card that still declares the openspeech TTS endpoint next to them.
    let binding = registry
        .resolve(BuiltinProviderRequest {
            provider_profile_id: "doubao-agent-plan",
            protocol_adapter_id: "doubao-responses",
            auth_mode: ProviderAuthMode::ApiKey,
            credential_kind: None,
            configured_inventory: None,
        })
        .unwrap();
    let custom = binding
        .connection
        .resolve(ProviderConnectionInput {
            base_url: Some("https://proxy.example/api/v3"),
            ..Default::default()
        })
        .unwrap();
    assert!(custom.operation_base_urls.is_empty());
    let defaults = binding.connection.resolve(Default::default()).unwrap();
    assert_eq!(
        defaults.operation_base_urls,
        BTreeMap::from([
            (
                "ark.contents.generate".into(),
                "https://ark.cn-beijing.volces.com/api/plan/v3".into(),
            ),
            (
                "ark.images.generate".into(),
                "https://ark.cn-beijing.volces.com/api/plan/v3".into(),
            ),
            (
                "tts.unidirectional".into(),
                "https://openspeech.bytedance.com/api/v3/plan/tts".into(),
            ),
        ])
    );
    let overrides = BTreeMap::from([(
        "tts.unidirectional".into(),
        "https://proxy.example/tts".into(),
    )]);
    let explicit = binding
        .connection
        .resolve(ProviderConnectionInput {
            base_url: Some("https://proxy.example/api/v3"),
            operation_base_urls: Some(&overrides),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(explicit.operation_base_urls, overrides);
    let schema = binding.connection.policy_region.as_ref().unwrap();
    assert_eq!(
        schema.resolve("policy_region", None).unwrap().as_deref(),
        Some("unknown")
    );
    assert!(schema
        .resolve("policy_region", Some("invalid-region"))
        .is_err());
    let profile = registry
        .profiles()
        .find(|profile| profile.provider_profile_id == "doubao")
        .unwrap();
    let mut plan = instance(profile, "plan");
    plan.base_url = "https://ark.cn-beijing.volces.com/api/plan/v3".into();
    let inventory = InventoryBuilder::build(
        profile,
        &plan,
        discovery(&["doubao-seed-2.1-pro", "doubao-seedream-5.0-lite"]),
        &catalog,
        &registry.codecs(),
    )
    .unwrap();
    assert!(inventory
        .models
        .iter()
        .any(|model| model.provider_model_id == "doubao-seed-2.1-pro"));
    assert!(inventory
        .models
        .iter()
        .any(|model| model.provider_model_id == "doubao-seedream-5.0-lite"));
    assert!(inventory.models.iter().all(|model| model.pricing.is_none()));
}

#[test]
fn all_builtin_prices_with_supported_units_can_complete_finance() {
    use crate::catalog::PricingUnit;
    let catalog = catalog();
    let usage = buckyos_api::AiUsage {
        input_tokens: Some(100),
        output_tokens: Some(10),
        total_tokens: Some(110),
        request_units: Some(1),
        image_units: Some(1),
        audio_seconds: Some(1.0),
        video_seconds: Some(1.0),
        characters: Some(1),
        ..Default::default()
    };
    for provider in catalog.known_providers() {
        let Some(rules) = catalog.provider_rules(&provider.provider_profile_id) else {
            continue;
        };
        for (index, entry) in rules.model_pricing.iter().enumerate() {
            let price = &entry.pricing;
            let mut usage = usage.clone();
            let first_tier = price.tiers.as_ref().and_then(|tiers| tiers.steps.first());
            if price.has_token_rates() {
                usage.input_tokens = Some(
                    if price
                        .input_token
                        .or_else(|| first_tier.and_then(|tier| tier.input_token))
                        .is_some()
                    {
                        100
                    } else {
                        0
                    },
                );
                usage.output_tokens = Some(
                    if price
                        .output_token
                        .or_else(|| first_tier.and_then(|tier| tier.output_token))
                        .is_some()
                    {
                        10
                    } else {
                        0
                    },
                );
                usage.total_tokens =
                    Some(usage.input_tokens.unwrap() + usage.output_tokens.unwrap());
            }
            let pinned = crate::execution::PinnedPricingSnapshot::from_pricing(
                price,
                price
                    .amount
                    .or_else(|| price.rules.first().map(|rule| rule.amount)),
                std::time::SystemTime::now(),
            )
            .unwrap()
            .unwrap();
            if matches!(
                price.unit,
                Some(PricingUnit::Second | PricingUnit::Megapixel)
            ) {
                assert!(pinned.completion_cost(&usage).is_none());
            } else {
                assert!(
                    pinned.completion_cost(&usage).is_some(),
                    "{} pricing[{index}] cannot be billed",
                    provider.provider_profile_id
                );
            }
        }
    }
}

#[test]
fn routable_builtin_unit_prices_match_operation_usage_dimensions() {
    use crate::catalog::PricingUnit;
    let catalog = catalog();
    let registry = builtin_provider_registry(&catalog).unwrap();
    let mut checked = 0;
    for profile in registry.profiles() {
        let Some(rules) = catalog.provider_rules(&profile.provider_profile_id) else {
            continue;
        };
        let ids = rules
            .static_inventory_models
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        for region in ["global", "china", "cn-beijing", "ap-southeast-1"] {
            let mut config = instance(profile, &profile.provider_profile_id);
            config.region = Some(region.into());
            let inventory = InventoryBuilder::build(
                profile,
                &config,
                discovery(&ids),
                &catalog,
                &registry.codecs(),
            )
            .unwrap();
            for model in inventory.models {
                let Some(unit) = model
                    .pricing
                    .as_ref()
                    .and_then(|pricing| pricing.value.unit)
                else {
                    continue;
                };
                for api in model.api_types {
                    let supported = match unit {
                        PricingUnit::Character => api == ApiType::AudioTextToSpeech,
                        PricingUnit::Image => matches!(
                            api,
                            ApiType::ImageTextToImage
                                | ApiType::ImageImageToImage
                                | ApiType::ImageInpaint
                                | ApiType::ImageUpscale
                                | ApiType::ImageBackgroundRemove
                        ),
                        PricingUnit::VideoSecond => matches!(
                            api,
                            ApiType::VideoTextToVideo
                                | ApiType::VideoImageToVideo
                                | ApiType::VideoToVideo
                                | ApiType::VideoExtend
                        ),
                        PricingUnit::AudioSecond => {
                            matches!(api, ApiType::AudioSpeechRecognition | ApiType::AudioEnhance)
                        }
                        PricingUnit::Request => true,
                        PricingUnit::Second | PricingUnit::Megapixel => continue,
                    };
                    assert!(
                        supported,
                        "{} {} {api:?} has incompatible {unit:?} pricing",
                        profile.provider_profile_id, model.provider_model_id
                    );
                    checked += 1;
                }
            }
        }
    }
    // The standard `doubao` profile discovers its inventory dynamically and the
    // `doubao-agent-plan` profile is a prepaid plan whose per-request pricing the
    // schema cannot express, so neither contributes unit-priced dimensions to
    // this sweep. The floor is what the remaining providers provide plus the three
    // active `doubao-speech` models (one character-priced TTS model and two
    // audio-second-priced ASR endpoints) across the four probed regions.
    assert!(
        checked >= 31,
        "pricing coverage unexpectedly shrank: {checked}"
    );
}

#[test]
fn qwen_media_protocol_fixtures_resolve_to_routable_inventory() {
    let catalog = catalog();
    let registry = builtin_provider_registry(&catalog).unwrap();
    let profile = registry
        .profiles()
        .find(|profile| profile.provider_profile_id == "qwen")
        .unwrap();
    let ids = ["wan2.1-t2i-turbo", "qwen-image-edit", "wan2.6-t2v"];
    let inventory = InventoryBuilder::build(
        profile,
        &instance(profile, "qwen"),
        discovery(&ids),
        &catalog,
        &registry.codecs(),
    )
    .unwrap();
    assert!(
        inventory.unmatched_models.is_empty(),
        "{:?}",
        inventory.unmatched_models
    );
    assert_eq!(inventory.models.len(), 3);
    for (model, api) in ids.into_iter().zip([
        ApiType::ImageTextToImage,
        ApiType::ImageImageToImage,
        ApiType::VideoTextToVideo,
    ]) {
        assert!(inventory
            .models
            .iter()
            .find(|item| item.provider_model_id == model)
            .unwrap()
            .api_types
            .contains(&api));
    }
}
