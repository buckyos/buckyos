use super::*;
use crate::catalog::{
    CatalogBuildOptions, CatalogSnapshot, ModelIdentity, ModelMatchFailure, ProviderModelMatch,
};
use crate::protocol::*;
use crate::provider::*;
use crate::settings::{load_builtin_metadata, MetadataSources};
use async_trait::async_trait;
use buckyos_api::{AiMessage, AiRole, ApiType, LlmChatInvokeRequest};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
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
    let deepseek = openai_responses_compatible::OpenAiCompatibleModelsDiscovery::new(
        "deepseek",
        DEEPSEEK_RESPONSES_ADAPTER_ID,
        HttpTransport::new(Default::default()).unwrap(),
    );
    assert_eq!(
        deepseek.match_model_driver("deepseek-v4-flash", &catalog),
        ProviderModelMatch::Failed(ModelMatchFailure::UnresolvedAlias)
    );
    let mut updated: Value =
        builtin_catalog_document(crate::catalog::CatalogKind::ModelDriver, "deepseek");
    let mut next_model = updated["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|model| model["id"] == "deepseek-v4-flash")
        .unwrap()
        .clone();
    next_model["id"] = json!("deepseek-v4.1-flash");
    updated["models"].as_array_mut().unwrap().push(next_model);
    let updated = MetadataSources {
        builtin: load_builtin_metadata().unwrap(),
        system_config: vec![crate::settings::MetadataFile::parse(
            crate::settings::MetadataSource::SystemConfig,
            crate::catalog::CatalogKind::ModelDriver,
            serde_json::to_vec(&updated).unwrap(),
        )
        .unwrap()],
        ..Default::default()
    }
    .build_snapshot(3, &Default::default())
    .unwrap();
    assert_eq!(
        deepseek.match_model_driver("deepseek-v4-flash", &updated),
        ProviderModelMatch::Matched(ModelIdentity {
            model_driver_id: "deepseek".into(),
            model_id: "deepseek-v4.1-flash".into()
        })
    );
    assert_eq!(
        updated.match_model("deepseek-v4-flash").unwrap().model_id,
        "deepseek-v4-flash"
    );
    assert_eq!(
        catalog
            .match_model("deepseek-v4-flash")
            .unwrap()
            .model_driver_id,
        "deepseek"
    );
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
            "/enable_thinking",
            json!(true),
        ),
        (
            "qwen",
            "qwen3.5-27b",
            "thinking",
            "/enable_thinking",
            json!(true),
        ),
        (
            "kimi",
            "kimi-k2.6",
            "thinking",
            "/thinking/type",
            json!("enabled"),
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
        let mut request =
            LlmChatInvokeRequest::new(&exact, vec![AiMessage::text(AiRole::User, "hello")]);
        request.max_output_tokens = Some(2048);
        let call = buckyos_api::AiccCall::ChatCompletionsCreate(request);
        let credential = if provider == "claude" {
            ResolvedCredential::named_header("secret://test", "x-api-key", "secret").unwrap()
        } else if provider == "gemini" {
            ResolvedCredential::named_header("secret://test", "x-goog-api-key", "secret").unwrap()
        } else {
            ResolvedCredential::bearer("secret://test", "secret").unwrap()
        };
        let target = ProviderCallTarget {
            provider_rules_id: Some(provider.into()),
            base_url: config.base_url.clone(),
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
    assert!(inv.models[0].variants.is_empty());
    assert!(inv
        .unavailable_presets
        .iter()
        .any(|p| p.effort == "thinking"));
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
    .build_snapshot(3, &Default::default())
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
fn static_prices_are_scoped_to_the_verified_billing_region() {
    let catalog = catalog();
    let providers = builtin_provider_registry(&catalog).unwrap();
    let profile = providers
        .profiles()
        .find(|p| p.provider_profile_id == "glm")
        .unwrap();
    let mut instance = instance(profile, "glm");
    for (region, priced) in [("china", true), ("global", false)] {
        instance.region = Some(region.into());
        let inv = InventoryBuilder::build(
            profile,
            &instance,
            discovery(&["glm-5.3"]),
            &catalog,
            &providers.codecs(),
        )
        .unwrap();
        assert_eq!(
            inv.models
                .iter()
                .find(|m| m.provider_model_id == "glm-5.3")
                .unwrap()
                .pricing
                .is_some(),
            priced
        );
    }
}
