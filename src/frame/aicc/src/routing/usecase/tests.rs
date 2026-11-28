use super::*;
use crate::catalog::{
    CatalogBuildOptions, CatalogDocuments, CatalogSnapshot, ModelDriverCatalog, Pricing,
    PricingTierStep, PricingTiers, TierDimension, TierMode,
};
use crate::model::{
    InventoryModel, LogicalModelDefinition, ModelRegistry, MountMode, ProviderInventory,
    RegistryLayers,
};
use crate::routing::policy::{
    CallerIdentity, CredentialScope, EffectiveRoutingPolicy, PolicyEngine, ProviderPrivacy,
    ProviderTrustLevel, ProviderTrustView, ProviderType, QuotaLookup, QuotaSnapshot,
    QuotaSourceError,
};
use crate::routing::CandidateRuntimeState;
use buckyos_api::{
    AiccLogicalNodeOverlay, AiccPolicyConfig, AiccRouteOverlay, AiccSchedulerProfile,
    AiccUsecaseRoute, ApiType, LockedValue, LogicalItem, ModelDisable, ModelRequirement, Money,
    QuotaState,
};
use serde_json::json;
use std::collections::BTreeMap;

const USECASE: &str = "image.txt2img";

#[derive(Clone)]
struct OpenQuota;

impl QuotaSource for OpenQuota {
    fn query(&self, _lookup: &QuotaLookup) -> Result<QuotaSnapshot, QuotaSourceError> {
        Ok(QuotaSnapshot {
            state: Some(QuotaState::Normal),
            remaining_request_units: None,
            remaining_cost: None,
            reset_at: None,
        })
    }
}

fn catalog() -> CatalogSnapshot {
    let driver: ModelDriverCatalog = serde_json::from_value(json!({
        "format": "buckyos.aicc.model-driver-catalog",
        "schema_version": 2,
        "schema_revision": 0,
        "model_driver_id": "driver",
        "revision_seq": 1,
        "models": [],
        "patterns": [{
            "match": {"origin_model_id": ["flux", "sd", "mj"]},
            "api_types": ["image.txt2img"],
            "capabilities": {"streaming": true}
        }],
        "defaults": {},
        "specs": []
    }))
    .unwrap();
    CatalogSnapshot::build(
        1,
        CatalogDocuments {
            model_drivers: vec![driver],
            ..CatalogDocuments::default()
        },
        &CatalogBuildOptions::default(),
    )
    .unwrap()
}

/// `models` are `(origin_model_id)` on one Provider instance.
fn inventory(instance: &str, models: &[&str]) -> ProviderInventory {
    ProviderInventory {
        provider_instance_name: instance.into(),
        provider_profile_id: "profile".into(),
        protocol_adapter_id: "adapter".into(),
        inventory_revision: format!("rev-{instance}"),
        models: models
            .iter()
            .map(|id| InventoryModel {
                protocol_adapter_id: "adapter".into(),
                provider_model_id: (*id).into(),
                model_driver_id: "driver".into(),
                origin_model_id: (*id).into(),
                api_types: vec![ApiType::ImageTextToImage],
                logical_mounts: Vec::new(),
                variants: Vec::new(),
                capabilities: BTreeMap::new(),
                canonical_fields: BTreeMap::new(),
                attributes: BTreeMap::new(),
                operations: BTreeMap::from([("image.txt2img".into(), "images.generate".into())]),
            })
            .collect(),
    }
}

/// `image.txt2img` lists `items` (name, exact model, weight).
fn registry(inventories: &[ProviderInventory], items: &[(&str, &str, f64)]) -> ModelRegistry {
    let factory = AiccRouteOverlay {
        logical_tree: BTreeMap::from([(
            USECASE.to_string(),
            AiccLogicalNodeOverlay {
                items: Some(
                    items
                        .iter()
                        .map(|(name, target, weight)| LogicalItem::new(*name, *target, *weight))
                        .collect(),
                ),
                ..AiccLogicalNodeOverlay::default()
            },
        )]),
        ..AiccRouteOverlay::default()
    };
    ModelRegistry::build(
        &catalog(),
        inventories,
        vec![LogicalModelDefinition {
            path: USECASE.into(),
            api_type: ApiType::ImageTextToImage,
            min_line: ModelRequirement::default(),
            disable_line: ModelDisable::default(),
            default_options: BTreeMap::new(),
            mount_mode: MountMode::Manual,
            scheduler_profile: AiccSchedulerProfile::Balanced,
            route_policy: AiccPolicyConfig::default(),
            user_visible_tier: None,
        }],
        RegistryLayers {
            factory: Some(&factory),
            system: None,
        },
    )
    .unwrap()
}

fn state(enabled: bool) -> CandidateRuntimeState {
    CandidateRuntimeState {
        enabled,
        credential_available: true,
        provider_privacy: ProviderPrivacy::PublicCloud,
        trust: Some(ProviderTrustView {
            provider_type: ProviderType::CloudApi,
            provider_type_revision: "rev".into(),
            asserted_at_ms: 1,
            trust_level: ProviderTrustLevel::Verified,
        }),
        credential_scope: CredentialScope::Tenant {
            tenant_id: "tenant".into(),
        },
        estimated_cost: None,
        p95_latency_ms: None,
        cache_hit_probability: None,
    }
}

fn runtime(registry: &ModelRegistry, disabled: &[&str]) -> BTreeMap<String, CandidateRuntimeState> {
    registry
        .model_views()
        .into_iter()
        .map(|model| {
            let enabled = !disabled.contains(&model.exact_model.as_str());
            (model.exact_model, state(enabled))
        })
        .collect()
}

fn engine() -> PolicyEngine<OpenQuota> {
    PolicyEngine::new(EffectiveRoutingPolicy::default(), OpenQuota).unwrap()
}

fn request() -> RoutingRequest {
    RoutingRequest::new(
        "trace",
        "request",
        USECASE,
        ApiType::ImageTextToImage,
        CallerIdentity {
            tenant_id: "tenant".into(),
            user_id: "user".into(),
            app_id: None,
        },
    )
}

fn route(
    model: Option<LockedValue<String>>,
    policy: Option<LockedValue<OnUnavailable>>,
) -> AiccUsecaseRoute {
    AiccUsecaseRoute {
        model,
        on_unavailable: policy,
        enabled: None,
    }
}

fn layers(entries: Vec<(LayerId, AiccUsecaseRoute)>) -> UsecaseLayers {
    UsecaseLayers {
        layers: entries
            .into_iter()
            .map(|(layer, route)| (layer, BTreeMap::from([(USECASE.to_string(), route)])))
            .collect(),
        ..UsecaseLayers::default()
    }
}

fn per_unit(amount: f64, currency: &str) -> Pricing {
    serde_json::from_value(json!({"currency": currency, "unit": "image", "amount": amount}))
        .unwrap()
}

#[test]
fn higher_layers_win_cell_by_cell_and_zone_locks_hold() {
    let layers = layers(vec![
        (
            LayerId::Zone,
            route(
                Some(LockedValue::new("flux@a".into())),
                Some(LockedValue::locked(OnUnavailable::SameModel)),
            ),
        ),
        (
            LayerId::User,
            route(
                Some(LockedValue::new("sd@b".into())),
                Some(LockedValue::new(OnUnavailable::NextModel)),
            ),
        ),
        (
            LayerId::Group("g1".into()),
            route(None, Some(LockedValue::new(OnUnavailable::NotPricier))),
        ),
    ]);
    let cells = layers.cells(USECASE);
    assert_eq!(
        cells.models,
        vec![
            ModelChoice {
                layer: "user".into(),
                model: "sd@b".into()
            },
            ModelChoice {
                layer: "zone".into(),
                model: "flux@a".into()
            },
        ]
    );
    assert!(!cells.model_locked);
    assert_eq!(cells.on_unavailable, OnUnavailable::SameModel);
    assert_eq!(cells.on_unavailable_source, "zone");
    assert!(cells.on_unavailable_locked);

    let locked = self::layers(vec![
        (
            LayerId::Zone,
            route(Some(LockedValue::locked("flux@a".into())), None),
        ),
        (
            LayerId::User,
            route(Some(LockedValue::new("sd@b".into())), None),
        ),
    ]);
    let cells = locked.cells(USECASE);
    assert!(cells.model_locked);
    assert_eq!(cells.models.len(), 1);
    assert_eq!(cells.models[0].model, "flux@a");

    // A `locked` written in a user layer means nothing.
    let user_lock = self::layers(vec![
        (
            LayerId::User,
            route(Some(LockedValue::locked("sd@b".into())), None),
        ),
        (
            LayerId::Group("g".into()),
            route(Some(LockedValue::new("mj@c".into())), None),
        ),
    ]);
    assert_eq!(user_lock.cells(USECASE).models[0].model, "mj@c");
    assert_eq!(user_lock.cells(USECASE).on_unavailable, OnUnavailable::None);
    assert_eq!(user_lock.cells(USECASE).on_unavailable_source, "default");
}

#[test]
fn a_chosen_model_routes_and_an_unusable_one_follows_the_next_layer() {
    let registry = registry(
        &[inventory("a", &["flux"]), inventory("b", &["sd"])],
        &[("flux", "flux@a", 2.0), ("sd", "sd@b", 1.0)],
    );
    let policy = engine();
    let all = runtime(&registry, &[]);
    let router = Router::new(&registry, &policy, &all);
    let default = resolve(&router, &UsecaseLayers::default(), &request(), true).unwrap();
    assert_eq!(default.decision.selected.exact_model, "flux@a");
    assert_eq!(
        default
            .decision
            .trace
            .usecase
            .as_ref()
            .unwrap()
            .usecase_source,
        "default"
    );

    let table = layers(vec![
        (
            LayerId::Zone,
            route(Some(LockedValue::new("sd@b".into())), None),
        ),
        (
            LayerId::User,
            route(Some(LockedValue::new("missing@nowhere".into())), None),
        ),
    ]);
    let chosen = resolve(&router, &table, &request(), true).unwrap();
    assert_eq!(chosen.decision.selected.exact_model, "sd@b");
    let trace = chosen.decision.trace.usecase.as_ref().unwrap();
    assert_eq!(trace.usecase_source, "zone");
    assert_eq!(trace.skipped_models[0].from, "missing@nowhere");
    assert_eq!(trace.skipped_models[0].reason, "model_missing");
    assert_eq!(chosen.decision.trace.requested_model, USECASE);

    // Not usable by this caller (hidden or disabled): the default decides.
    let hidden = runtime(&registry, &["sd@b"]);
    let router = Router::new(&registry, &policy, &hidden);
    let fallback = resolve(&router, &table, &request(), true).unwrap();
    assert_eq!(fallback.decision.selected.exact_model, "flux@a");
    assert_eq!(
        fallback
            .decision
            .trace
            .usecase
            .as_ref()
            .unwrap()
            .usecase_source,
        "default"
    );
}

#[test]
fn the_default_route_ignores_observed_failures_and_is_stable() {
    let registry = registry(
        &[inventory("a", &["flux"]), inventory("b", &["flux"])],
        &[("a", "flux@a", 1.0), ("b", "flux@b", 1.0)],
    );
    let policy = engine();
    let mut states = runtime(&registry, &[]);
    // Latency observations are not a routing input without `max_latency_ms`.
    states.get_mut("flux@a").unwrap().p95_latency_ms = Some(120_000.0);
    let router = Router::new(&registry, &policy, &states);
    let first = resolve(&router, &UsecaseLayers::default(), &request(), true).unwrap();
    for _ in 0..10 {
        let again = resolve(&router, &UsecaseLayers::default(), &request(), true).unwrap();
        assert_eq!(
            again.decision.selected.exact_model,
            first.decision.selected.exact_model
        );
    }
}

fn search<'a>(
    price_of: &'a dyn Fn(&str) -> Option<Pricing>,
    to_usd: &'a dyn Fn(f64, &str) -> Option<f64>,
    order: &'a [String],
    faulty: &'a BTreeSet<String>,
) -> FallbackSearch<'a> {
    FallbackSearch {
        price_of,
        to_usd,
        provider_order: order,
        faulty,
    }
}

#[test]
fn same_model_moves_to_another_provider_by_configuration_order() {
    let registry = registry(
        &[
            inventory("a", &["flux"]),
            inventory("b", &["flux", "sd"]),
            inventory("c", &["flux"]),
        ],
        &[("flux", "flux@a", 2.0), ("sd", "sd@b", 1.0)],
    );
    let policy = engine();
    let states = runtime(&registry, &[]);
    let router = Router::new(&registry, &policy, &states);
    let table = layers(vec![(
        LayerId::User,
        route(None, Some(LockedValue::new(OnUnavailable::SameModel))),
    )]);
    let primary = resolve(&router, &table, &request(), true).unwrap();
    assert_eq!(primary.on_unavailable, OnUnavailable::SameModel);
    let none = |_: &str| None;
    let usd = |amount: f64, _: &str| Some(amount);
    let order = vec!["c".to_string(), "a".to_string(), "b".to_string()];
    let faulty = BTreeSet::new();
    let backup = unavailable_fallback(
        &router,
        &table,
        &request(),
        false,
        &primary.decision.selected,
        OnUnavailable::SameModel,
        &search(&none, &usd, &order, &faulty),
    )
    .unwrap()
    .unwrap();
    assert_eq!(backup.selected.exact_model, "flux@c");
    let switch = backup.trace.fallback.unwrap();
    assert_eq!(switch.kind, buckyos_api::RouteFallbackKind::Unavailable);
    assert_eq!(switch.from, "flux@a");
    assert_eq!(switch.to, "flux@c");
    // A Provider marked faulty is never a backup.
    let faulty = BTreeSet::from(["c".to_string()]);
    let backup = unavailable_fallback(
        &router,
        &table,
        &request(),
        false,
        &primary.decision.selected,
        OnUnavailable::SameModel,
        &search(&none, &usd, &order, &faulty),
    )
    .unwrap()
    .unwrap();
    assert_eq!(backup.selected.exact_model, "flux@b");
}

#[test]
fn a_single_provider_has_no_backup_under_any_policy() {
    let registry = registry(
        &[inventory("only", &["flux", "sd"])],
        &[("flux", "flux@only", 2.0), ("sd", "sd@only", 1.0)],
    );
    let policy = engine();
    let states = runtime(&registry, &[]);
    let router = Router::new(&registry, &policy, &states);
    let primary = resolve(&router, &UsecaseLayers::default(), &request(), true).unwrap();
    let price = |_: &str| Some(per_unit(0.01, "USD"));
    let usd = |amount: f64, _: &str| Some(amount);
    let order = vec!["only".to_string()];
    let faulty = BTreeSet::new();
    for policy in [
        OnUnavailable::None,
        OnUnavailable::SameModel,
        OnUnavailable::NotPricier,
        OnUnavailable::NextModel,
    ] {
        assert!(unavailable_fallback(
            &router,
            &UsecaseLayers::default(),
            &request(),
            false,
            &primary.decision.selected,
            policy,
            &search(&price, &usd, &order, &faulty),
        )
        .unwrap()
        .is_none());
    }
}

#[test]
fn next_model_and_not_pricier_route_again_without_the_failed_provider() {
    let registry = registry(
        &[
            inventory("a", &["flux"]),
            inventory("b", &["sd"]),
            inventory("c", &["mj"]),
        ],
        &[
            ("flux", "flux@a", 3.0),
            ("sd", "sd@b", 2.0),
            ("mj", "mj@c", 1.0),
        ],
    );
    let policy = engine();
    let states = runtime(&registry, &[]);
    let router = Router::new(&registry, &policy, &states);
    let primary = resolve(&router, &UsecaseLayers::default(), &request(), true).unwrap();
    assert_eq!(primary.decision.selected.exact_model, "flux@a");
    let usd = |amount: f64, currency: &str| match currency {
        "USD" => Some(amount),
        "CNY" => Some(amount / 7.0),
        _ => None,
    };
    let order = vec!["a".to_string(), "b".to_string(), "c".to_string()];
    let faulty = BTreeSet::new();
    let run = |prices: &dyn Fn(&str) -> Option<Pricing>, policy| {
        unavailable_fallback(
            &router,
            &UsecaseLayers::default(),
            &request(),
            false,
            &primary.decision.selected,
            policy,
            &search(prices, &usd, &order, &faulty),
        )
        .unwrap()
        .map(|decision| decision.selected.exact_model)
    };
    let prices = |exact: &str| match exact {
        "flux@a" => Some(per_unit(0.04, "USD")),
        "sd@b" => Some(per_unit(0.05, "USD")),
        "mj@c" => Some(per_unit(0.03, "USD")),
        _ => None,
    };
    assert_eq!(
        run(&prices, OnUnavailable::NextModel).as_deref(),
        Some("sd@b")
    );
    // sd costs more, so not_pricier keeps looking and finds mj.
    assert_eq!(
        run(&prices, OnUnavailable::NotPricier).as_deref(),
        Some("mj@c")
    );
    // An unknown price is never "not pricier".
    let unknown = |exact: &str| match exact {
        "flux@a" => Some(per_unit(0.04, "USD")),
        "sd@b" => None,
        "mj@c" => None,
        _ => None,
    };
    assert_eq!(run(&unknown, OnUnavailable::NotPricier), None);
    // Converted through the exchange rate: 0.21 CNY = 0.03 USD.
    let converted = |exact: &str| match exact {
        "flux@a" => Some(per_unit(0.04, "USD")),
        "sd@b" => Some(per_unit(0.21, "CNY")),
        _ => None,
    };
    assert_eq!(
        run(&converted, OnUnavailable::NotPricier).as_deref(),
        Some("sd@b")
    );
    // No rate for the currency: not proof.
    let foreign = |exact: &str| match exact {
        "flux@a" => Some(per_unit(0.04, "USD")),
        "sd@b" => Some(per_unit(0.01, "EUR")),
        "mj@c" => Some(per_unit(0.01, "EUR")),
        _ => None,
    };
    assert_eq!(run(&foreign, OnUnavailable::NotPricier), None);
    // The original has no known price: nothing can be shown not pricier.
    let original_unknown = |exact: &str| match exact {
        "flux@a" => None,
        _ => Some(per_unit(0.0, "USD")),
    };
    assert_eq!(run(&original_unknown, OnUnavailable::NotPricier), None);
}

#[test]
fn price_comparison_uses_every_dimension_and_the_worst_tier() {
    let usd = |amount: f64, _: &str| Some(amount);
    let tokens = |input: f64, output: f64| -> Pricing {
        serde_json::from_value(
            json!({"currency": "USD", "input_token": input, "output_token": output}),
        )
        .unwrap()
    };
    let original = tokens(2e-6, 8e-6);
    assert!(crate::policy::pricing::not_pricier(
        &tokens(1e-6, 8e-6),
        &original,
        &usd
    ));
    assert!(!crate::policy::pricing::not_pricier(
        &tokens(1e-6, 9e-6),
        &original,
        &usd
    ));
    // A dimension the original does not bill: not comparable.
    let mut cached = tokens(1e-6, 1e-6);
    cached.cache_input_token = Some(1e-7);
    assert!(!crate::policy::pricing::not_pricier(
        &cached, &original, &usd
    ));
    // Tiered prices compare the candidate's highest step with the original's lowest.
    let mut tiered = tokens(1e-6, 4e-6);
    tiered.tiers = Some(PricingTiers {
        dimension: TierDimension::InputTokens,
        mode: TierMode::Volume,
        steps: vec![
            PricingTierStep {
                up_to: Some(200_000),
                input_token: Some(1e-6),
                output_token: Some(4e-6),
                ..serde_json::from_value(json!({})).unwrap()
            },
            PricingTierStep {
                up_to: None,
                input_token: Some(3e-6),
                output_token: Some(12e-6),
                ..serde_json::from_value(json!({})).unwrap()
            },
        ],
    });
    assert!(!crate::policy::pricing::not_pricier(
        &tiered, &original, &usd
    ));
    assert!(!crate::policy::pricing::not_pricier(
        &original, &tiered, &usd
    ));
    assert!(crate::policy::pricing::not_pricier(
        &tokens(1e-6, 4e-6),
        &tiered,
        &usd
    ));
    let _ = Money::new(0.0, "USD");
}

#[test]
fn the_fallback_llm_only_covers_llm_usecases_and_follows_its_own_row() {
    let layers = UsecaseLayers {
        layers: vec![(
            LayerId::Zone,
            BTreeMap::from([(
                AICC_FALLBACK_LLM.to_string(),
                AiccUsecaseRoute {
                    enabled: Some(true),
                    on_unavailable: Some(LockedValue::new(OnUnavailable::NextModel)),
                    ..Default::default()
                },
            )]),
        )],
        ..UsecaseLayers::default()
    };
    assert!(layers.fallback_llm_enabled());
    assert_eq!(
        layers.cells(AICC_FALLBACK_LLM).on_unavailable,
        OnUnavailable::NextModel
    );
    // image.txt2img is not an LLM usecase: no model means no route.
    let registry = registry(&[inventory("a", &["flux"])], &[]);
    let policy = engine();
    let states = runtime(&registry, &[]);
    let router = Router::new(&registry, &policy, &states);
    assert!(resolve(&router, &layers, &request(), true).is_err());
    assert!(!UsecaseLayers::default().fallback_llm_enabled());
    assert!(is_usecase("llm.plan") && is_usecase(AICC_FALLBACK_LLM));
    assert!(!is_usecase("llm.gpt-pro") && !is_usecase("image.txt2img.flux"));
}
