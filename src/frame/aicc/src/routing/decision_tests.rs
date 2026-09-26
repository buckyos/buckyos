fn decision_registry(inventory_enabled: bool) -> ModelRegistry {
    let builtin = crate::model::llm_tests::builtin_catalog();
    let catalog = CatalogSnapshot::build(
        1,
        CatalogDocuments {
            model_drivers: vec![builtin.model_driver("typesafe").unwrap().clone()],
            ..Default::default()
        },
        &CatalogBuildOptions::default(),
    )
    .unwrap();
    let capabilities = catalog
        .resolve_model("typesafe", "jev-1.13.0")
        .unwrap()
        .semantics
        .capabilities
        .unwrap();
    let mut inventories = Vec::new();
    for provider in ["partial", "full", "backup"] {
        let mut caps = capabilities.clone();
        if provider == "partial" {
            caps.remove("decision.score");
        }
        inventories.push(ProviderInventory {
            provider_instance_name: provider.into(),
            provider_profile_id: "typesafe".into(),
            protocol_adapter_id: "typesafe-systemone".into(),
            inventory_revision: "1".into(),
            models: vec![InventoryModel {
                provider_model_id: "jev-1.13.0".into(),
                model_driver_id: "typesafe".into(),
                origin_model_id: "jev-1.13.0".into(),
                api_types: vec![ApiType::Decision],
                logical_mounts: vec![],
                variants: vec![],
                capabilities: caps,
                canonical_fields: BTreeMap::new(),
                attributes: BTreeMap::new(),
                operations: BTreeMap::from([("decision".into(), "systemone.evaluate".into())]),
            }],
        });
    }
    let factory = AiccRouteOverlay {
        logical_tree: BTreeMap::from([(
            "decision".into(),
            node(&[
                ("partial", "jev-1.13.0@partial", 2.0),
                ("full", "jev-1.13.0@full", 2.0),
                ("backup", "jev-1.13.0@backup", 1.0),
            ]),
        )]),
        ..Default::default()
    };
    let mut definition = definition(
        "decision",
        AiccFallbackMode::Strict,
        AiccSchedulerProfile::CostFirst,
    );
    definition.api_type = ApiType::Decision;
    ModelRegistry::build(
        &catalog,
        if inventory_enabled { &inventories } else { &[] },
        vec![definition],
        RegistryLayers {
            factory: Some(&factory),
            ..Default::default()
        },
    )
    .unwrap()
}

fn decision_request(path: &str) -> RoutingRequest {
    let canonical = buckyos_api::DecisionEvaluateRequest::from_json(
        serde_json::from_str::<Value>(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../test/aicc_test/acceptance/fixtures/typesafe-systemone.json"
        )))
        .unwrap()["canonical_request"]
            .clone(),
    )
    .unwrap();
    let mut request = RoutingRequest::new(
        "trace-decision",
        "request-decision",
        path,
        ApiType::Decision,
        request("image").caller,
    );
    request.requirements = canonical.requirements();
    request
}

#[test]
fn decision_routing_filters_mixed_and_exact_requests_before_weight_group_expansion() {
    let registry = decision_registry(true);
    let policy = engine(&RoutingPolicyPatch::default());
    let mut states = BTreeMap::from([
        ("jev-1.13.0@partial".into(), state(false, 0.01, 10.0)),
        ("jev-1.13.0@full".into(), state(false, 0.02, 10.0)),
        ("jev-1.13.0@backup".into(), state(false, 0.001, 10.0)),
    ]);
    let mixed = decision_request("decision");
    let route = Router::new(&registry, &policy, &states)
        .route(&mixed)
        .unwrap();
    assert_eq!(route.selected.exact_model, "jev-1.13.0@full");
    assert!(route
        .fallback_candidates
        .iter()
        .all(|c| c.exact_model != "jev-1.13.0@partial"));
    assert!(Router::new(&registry, &policy, &states)
        .route(&decision_request("jev-1.13.0@partial"))
        .is_err());
    let mut choice = mixed.clone();
    choice
        .requirements
        .decision
        .as_mut()
        .unwrap()
        .question_types
        .remove(&buckyos_api::DecisionQuestionType::Score);
    choice.requirements.decision.as_mut().unwrap().max_levels = 0;
    assert_eq!(
        Router::new(&registry, &policy, &states)
            .route(&choice)
            .unwrap()
            .selected
            .exact_model,
        "jev-1.13.0@partial"
    );
    states.get_mut("jev-1.13.0@full").unwrap().enabled = false;
    assert_eq!(
        Router::new(&registry, &policy, &states)
            .route(&mixed)
            .unwrap()
            .selected
            .exact_model,
        "jev-1.13.0@backup"
    );
    let mut oversized = decision_request("jev-1.13.0@backup");
    oversized
        .requirements
        .decision
        .as_mut()
        .unwrap()
        .max_options = 256;
    assert!(Router::new(&registry, &policy, &states)
        .route(&oversized)
        .is_err());
    let empty = decision_registry(false);
    assert!(Router::new(&empty, &policy, &BTreeMap::new())
        .route(&mixed)
        .is_err());
    assert!(empty
        .logical_model_views()
        .iter()
        .any(|v| v.path == "decision"));
    let mut wrong_api = mixed;
    wrong_api.api_type = ApiType::Llm;
    wrong_api.method = ApiType::Llm.typed_method().into();
    wrong_api.capability = Capability::Llm;
    assert!(Router::new(&registry, &policy, &states)
        .route(&wrong_api)
        .is_err());
}

#[test]
fn decision_unknown_price_is_not_free_under_a_budget() {
    let registry = decision_registry(true);
    let policy = engine(&RoutingPolicyPatch {
        route: AiccPolicyConfig {
            max_estimated_cost: Some(LockedValue::new(Money::new(0.01, "USD"))),
            ..Default::default()
        },
        ..Default::default()
    });
    let mut candidate = state(false, 0.0, 10.0);
    candidate.estimated_cost = None;
    let mut states = BTreeMap::from([("jev-1.13.0@full".into(), candidate)]);
    let request = decision_request("jev-1.13.0@full");
    assert!(Router::new(&registry, &policy, &states)
        .route(&request)
        .is_err());
    states.get_mut("jev-1.13.0@full").unwrap().estimated_cost = Some(Money::new(0.0, "USD"));
    assert!(Router::new(&registry, &policy, &states)
        .route(&request)
        .is_ok());
}

#[test]
fn openrouter_channel_capacity_filters_logical_and_exact_decision_routes() {
    let catalog = crate::settings::MetadataSources {
        builtin: crate::settings::load_builtin_metadata().unwrap(),
        ..Default::default()
    }
    .build_snapshot(crate::settings::BUILTIN_CATALOG_REVISION_SEQ, &Default::default())
    .unwrap();
    let facts = catalog
        .resolve_model("typesafe", "jev-1.13.0")
        .unwrap()
        .semantics;
    let rule = catalog
        .resolve_provider_rule(
            "openrouter",
            "typesafe/jev-1.13",
            &crate::matching::MatchContext::new(),
        )
        .unwrap()
        .unwrap();
    let narrowed = rule
        .action
        .narrow(&facts.api_types.unwrap(), &facts.capabilities.unwrap());
    let catalog = CatalogSnapshot::build(
        1,
        CatalogDocuments {
            model_drivers: vec![catalog.model_driver("typesafe").unwrap().clone()],
            ..Default::default()
        },
        &CatalogBuildOptions::default(),
    )
    .unwrap();
    let inventory = ProviderInventory {
        provider_instance_name: "openrouter-test".into(),
        provider_profile_id: "openrouter".into(),
        protocol_adapter_id: "openrouter-responses".into(),
        inventory_revision: "fixture".into(),
        models: vec![InventoryModel {
            provider_model_id: "typesafe/jev-1.13".into(),
            model_driver_id: "typesafe".into(),
            origin_model_id: "jev-1.13.0".into(),
            api_types: vec![ApiType::Decision],
            logical_mounts: vec![],
            variants: vec![],
            capabilities: narrowed.capabilities,
            canonical_fields: BTreeMap::new(),
            attributes: BTreeMap::new(),
            operations: BTreeMap::from([("decision".into(), "decisions.create".into())]),
        }],
    };
    let exact = "typesafe/jev-1.13@openrouter-test";
    let factory = AiccRouteOverlay {
        logical_tree: BTreeMap::from([("decision".into(), node(&[("jev", exact, 1.0)]))]),
        ..Default::default()
    };
    let mut definition = definition(
        "decision",
        AiccFallbackMode::Strict,
        AiccSchedulerProfile::CostFirst,
    );
    definition.api_type = ApiType::Decision;
    let registry = ModelRegistry::build(
        &catalog,
        &[inventory],
        vec![definition],
        RegistryLayers {
            factory: Some(&factory),
            ..Default::default()
        },
    )
    .unwrap();
    let policy = engine(&RoutingPolicyPatch::default());
    let states = BTreeMap::from([(exact.into(), state(false, 0.001, 10.0))]);
    for path in ["decision", exact] {
        let mut request = decision_request(path);
        assert!(Router::new(&registry, &policy, &states)
            .route(&request)
            .is_ok());
        request.requirements.decision.as_mut().unwrap().input_bytes = 32001;
        assert!(Router::new(&registry, &policy, &states)
            .route(&request)
            .is_err());
        request.requirements.decision.as_mut().unwrap().input_bytes = 32000;
        request
            .requirements
            .decision
            .as_mut()
            .unwrap()
            .max_state_question_bytes = 32001;
        assert!(Router::new(&registry, &policy, &states)
            .route(&request)
            .is_err());
        request
            .requirements
            .decision
            .as_mut()
            .unwrap()
            .max_state_question_bytes = 32000;
        request.requirements.min_context_tokens = Some(32001);
        assert!(Router::new(&registry, &policy, &states)
            .route(&request)
            .is_err());
    }
}
