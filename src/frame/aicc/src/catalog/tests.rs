use super::*;
use serde_json::json;

fn file(kind: CatalogKind, value: Value) -> CurrentCatalogFile {
    CurrentCatalogFile {
        kind,
        contents: serde_json::to_vec(&value).unwrap(),
    }
}

fn model_driver(id: &str, models: Value, patterns: Value) -> Value {
    json!({
        "format": MODEL_DRIVER_FORMAT,
        "schema_version": 2,
        "schema_revision": 0,
        "model_driver_id": id,
        "revision_seq": 7,
        "required_features": [],
        "models": models,
        "patterns": patterns,
        "defaults": {
            "api_types": ["image.txt2img"],
            "capabilities": {"streaming": true}
        },
        "specs": []
    })
}

fn provider_rules() -> Value {
    json!({
        "format": PROVIDER_RULES_FORMAT,
        "schema_version": 1,
        "schema_revision": 0,
        "revision_seq": 9,
        "provider_profile_id": "openai",
        "models": [{
            "id": "gpt-special",
            "exclude": true
        }],
        "patterns": [{
            "match": "gpt-*",
            "operations": {"llm": "responses.create"},
            "request_rules": [{
                "when": {"/quality": "high"},
                "remove": ["/temperature"]
            }],
            "remove_api_types": ["image.txt2img"],
            "remove_features": ["tool_call"]
        }, {
            "match": "*",
            "exclude": true
        }],
        "model_pricing": [{
            "match": "gpt-*",
            "pricing": {
                "currency": "USD",
                "unit": "request",
                "amount": 1.0,
                "rules": [{
                    "when": {"/quality": "high"},
                    "amount": 2.0
                }]
            }
        }],
        "variants": [{
            "model_driver": "openai",
            "variant": "reasoning.high",
            "match": "gpt-*",
            "provider_options": {"reasoning": {"effort": "high"}}
        }]
    })
}

fn known_providers() -> Value {
    json!({
        "format": KNOWN_PROVIDER_FORMAT,
        "schema_version": 1,
        "schema_revision": 0,
        "revision_seq": 11,
        "catalog_id": "builtin",
        "providers": [{
            "provider_profile_id": "openai",
            "display_name": "OpenAI",
            "base_url": "https://api.openai.com",
            "protocol_adapter_id": "openai-responses",
            "discovery_behavior_id": "openai-models",
            "provider_rules_id": "openai",
            "credential": {"kind": "bearer"},
            "connection": {
                "region": {"mode": "unsupported"},
                "workspace": {"mode": "unsupported"},
                "account": {"mode": "unsupported"}
            },
            "ui_hints": {"credential_label": "API key"}
        }]
    })
}

fn complete_files() -> Vec<CurrentCatalogFile> {
    vec![
        file(CatalogKind::KnownProvider, known_providers()),
        file(CatalogKind::ProviderRules, provider_rules()),
        file(
            CatalogKind::ModelDriver,
            model_driver(
                "openai",
                json!([{
                    "id": "gpt-special",
                    "api_types": ["image.txt2img"],
                    "capabilities": {"streaming": true, "tool_call": true}
                }]),
                json!([{
                    "match": "gpt-new",
                    "quality_score": 0.8
                }, {
                    "match": "gpt-5-new",
                    "quality_score": 0.9
                }]),
            ),
        ),
    ]
}

fn build(files: Vec<CurrentCatalogFile>) -> Result<CatalogSnapshot, CatalogBuildError> {
    CatalogSnapshot::from_current_files(42, files, &CatalogBuildOptions::default())
}

#[test]
fn current_file_set_builds_immutable_indexes_and_deterministic_snapshot() {
    let first = build(complete_files()).unwrap();
    let mut reversed_files = complete_files();
    reversed_files.reverse();
    let second = build(reversed_files).unwrap();

    assert_eq!(first.target_revision_seq(), 42);
    assert_eq!(
        first.model_driver("openai").unwrap().revision_seq,
        second.model_driver("openai").unwrap().revision_seq
    );
    assert_eq!(
        first.known_provider("openai"),
        second.known_provider("openai")
    );

    let exact = first.resolve_model("openai", "gpt-special").unwrap();
    assert_eq!(
        exact.semantics.api_types.unwrap(),
        BTreeSet::from(["image.txt2img".to_owned()])
    );
    assert_eq!(
        first.match_model("gpt-special"),
        second.match_model("gpt-special")
    );
    assert!(first.resolve_model("openai", "unknown").is_err());
    assert_eq!(
        first.match_model("unknown"),
        Err(ModelMatchFailure::NoMatch)
    );
}

#[test]
fn resolves_typed_provider_configuration_and_rules_identity() {
    let snapshot = build(complete_files()).unwrap();
    let resolved = snapshot.resolve_provider_configuration("openai").unwrap();

    assert_eq!(resolved.provider_profile_id, "openai");
    assert_eq!(resolved.display_name, "OpenAI");
    assert_eq!(resolved.default_base_url, "https://api.openai.com");
    assert_eq!(resolved.credential.kind, ProviderCredentialKind::Bearer);
    assert_eq!(
        resolved.connection.region.mode,
        ProviderFieldMode::Unsupported
    );
    assert_eq!(resolved.protocol_adapter_id, "openai-responses");
    assert_eq!(resolved.provider_rules_id, "openai");
    assert_eq!(
        snapshot
            .provider_rules(&resolved.provider_rules_id)
            .unwrap()
            .provider_profile_id,
        resolved.provider_profile_id
    );
}

#[test]
fn resolves_typed_credential_variants_and_region_base_urls() {
    let mut document = known_providers();
    document["schema_revision"] = json!(1);
    document["providers"][0]["credential_variants"] = json!([{"kind": "glm_jwt"}]);
    document["providers"][0]["connection"] = json!({
        "region": {
            "mode": "optional",
            "default_value": "global",
            "allowed_values": ["china", "global"]
        },
        "workspace": {"mode": "unsupported"},
        "account": {"mode": "unsupported"},
        "region_base_urls": {
            "china": "https://china.example/v1",
            "global": "https://global.example/v1"
        }
    });
    let mut files = complete_files();
    files[0] = file(CatalogKind::KnownProvider, document);
    let snapshot = build(files).unwrap();
    let resolved = snapshot.resolve_provider_configuration("openai").unwrap();

    assert_eq!(resolved.credential_variants.len(), 1);
    assert_eq!(
        resolved.credential_variants[0].kind,
        ProviderCredentialKind::GlmJwt
    );
    assert_eq!(
        resolved.connection.region_base_urls["china"],
        "https://china.example/v1"
    );
}

#[test]
fn provider_configuration_resolution_fails_closed() {
    let snapshot = build(complete_files()).unwrap();
    assert_eq!(
        snapshot.resolve_provider_configuration("unknown"),
        Err(CatalogResolveError::UnknownKnownProvider {
            provider_profile_id: "unknown".to_owned(),
        })
    );

    let mut missing_reference = known_providers();
    missing_reference["providers"][0]
        .as_object_mut()
        .unwrap()
        .remove("provider_rules_id");
    let snapshot = build(vec![file(CatalogKind::KnownProvider, missing_reference)]).unwrap();
    assert_eq!(
        snapshot.resolve_provider_configuration("openai"),
        Err(CatalogResolveError::MissingProviderRulesReference {
            provider_profile_id: "openai".to_owned(),
        })
    );
}

#[test]
fn invalid_typed_provider_configuration_is_rejected() {
    let cases = [
        ("credential", json!({"kind": "named_header"})),
        ("credential_variants", json!([{"kind": "bearer"}])),
        (
            "connection",
            json!({
                "region": {"mode": "unsupported", "default_value": "global"},
                "workspace": {"mode": "unsupported"},
                "account": {"mode": "unsupported"}
            }),
        ),
        (
            "connection",
            json!({
                "region": {
                    "mode": "optional",
                    "default_value": "global",
                    "allowed_values": ["china"]
                },
                "workspace": {"mode": "unsupported"},
                "account": {"mode": "unsupported"}
            }),
        ),
        (
            "connection",
            json!({
                "region": {
                    "mode": "optional",
                    "allowed_values": ["global"]
                },
                "workspace": {"mode": "unsupported"},
                "account": {"mode": "unsupported"},
                "region_base_urls": {"china": "https://china.example/v1"}
            }),
        ),
    ];
    for (field, invalid) in cases {
        let mut document = known_providers();
        document["providers"][0][field] = invalid;
        assert!(matches!(
            build(vec![file(CatalogKind::KnownProvider, document)]),
            Err(CatalogBuildError::InvalidValue { .. })
        ));
    }
}

#[test]
fn known_provider_enumeration_is_read_only_and_sorted_by_profile_id() {
    let catalog = |catalog_id: &str, provider_profile_id: &str| {
        json!({
            "format": KNOWN_PROVIDER_FORMAT,
            "schema_version": 1,
            "schema_revision": 0,
            "revision_seq": 11,
            "catalog_id": catalog_id,
            "providers": [{
                "provider_profile_id": provider_profile_id,
                "display_name": provider_profile_id,
                "base_url": format!("https://{provider_profile_id}.example"),
                "protocol_adapter_id": "test-adapter",
                "discovery_behavior_id": "standard-models",
                "credential": {"kind": "bearer"},
                "connection": {
                    "region": {"mode": "unsupported"},
                    "workspace": {"mode": "unsupported"},
                    "account": {"mode": "unsupported"}
                }
            }]
        })
    };
    let snapshot = build(vec![
        file(CatalogKind::KnownProvider, catalog("z-catalog", "zeta")),
        file(CatalogKind::KnownProvider, catalog("a-catalog", "alpha")),
    ])
    .unwrap();

    assert_eq!(
        snapshot
            .known_providers()
            .map(|provider| provider.provider_profile_id.as_str())
            .collect::<Vec<_>>(),
        vec!["alpha", "zeta"]
    );
}

#[test]
fn provider_exact_and_ordered_pattern_indexes_preserve_actions() {
    let snapshot = build(complete_files()).unwrap();
    let exact = snapshot
        .resolve_provider_rule("openai", "gpt-special", &MatchContext::new())
        .unwrap()
        .unwrap();
    assert_eq!(exact.match_kind, ProviderRuleMatchKind::Exact);
    assert!(exact.action.exclude);

    let pattern = snapshot
        .resolve_provider_rule("openai", "gpt-5-new", &MatchContext::new())
        .unwrap()
        .unwrap();
    assert_eq!(pattern.match_kind, ProviderRuleMatchKind::Pattern);
    assert_eq!(pattern.trace.as_ref().unwrap().position, 0);
    assert_eq!(pattern.action.operations["llm"], "responses.create");
    let high_quality = BTreeMap::from([("/quality".to_owned(), json!("high"))]);
    assert_eq!(pattern.matching_request_rules(&high_quality).len(), 1);
    assert_eq!(pattern.price_for(&high_quality), Some(2.0));
    assert_eq!(pattern.price_for(&MatchContext::new()), Some(1.0));
    let variant_context = BTreeMap::from([("provider_model_id".to_owned(), json!("gpt-5-new"))]);
    assert_eq!(
        snapshot
            .matching_provider_variants("openai", &variant_context)
            .unwrap()
            .len(),
        1
    );

    let narrowed = pattern.action.narrow(
        &BTreeSet::from(["llm".to_owned(), "image.txt2img".to_owned()]),
        &BTreeMap::from([
            ("streaming".to_owned(), json!(true)),
            ("tool_call".to_owned(), json!(true)),
        ]),
    );
    assert_eq!(narrowed.api_types, BTreeSet::from(["llm".to_owned()]));
    assert_eq!(
        narrowed.capabilities,
        BTreeMap::from([("streaming".to_owned(), json!(true))])
    );
}

#[test]
fn schema_revision_required_features_and_references_are_validated() {
    let mut unsupported_schema = model_driver("openai", json!([]), json!([]));
    unsupported_schema["schema_revision"] = json!(2);
    assert!(matches!(
        build(vec![file(CatalogKind::ModelDriver, unsupported_schema)]),
        Err(CatalogBuildError::UnsupportedSchema { .. })
    ));

    let mut revision_zero_variant_options = model_driver("openai", json!([]), json!([]));
    revision_zero_variant_options["variants"] = json!([{
        "name": "reasoning-high",
        "match": "gpt-*",
        "provider_options": {"reasoning": {"effort": "high"}}
    }]);
    assert!(matches!(
        build(vec![file(
            CatalogKind::ModelDriver,
            revision_zero_variant_options
        )]),
        Err(CatalogBuildError::InvalidJson { .. })
    ));

    let mut required_feature = model_driver("openai", json!([]), json!([]));
    required_feature["required_features"] = json!(["structured-capability-v1"]);
    assert!(matches!(
        build(vec![file(
            CatalogKind::ModelDriver,
            required_feature.clone()
        )]),
        Err(CatalogBuildError::UnsupportedFeature { .. })
    ));
    let options = CatalogBuildOptions {
        supported_features: BTreeSet::from(["structured-capability-v1".to_owned()]),
    };
    CatalogSnapshot::from_current_files(
        42,
        vec![file(CatalogKind::ModelDriver, required_feature)],
        &options,
    )
    .unwrap();

    let future_revision = model_driver("openai", json!([]), json!([]));
    assert!(matches!(
        CatalogSnapshot::from_current_files(
            6,
            vec![file(CatalogKind::ModelDriver, future_revision)],
            &CatalogBuildOptions::default(),
        ),
        Err(CatalogBuildError::RevisionAheadOfSnapshot {
            revision_seq: 7,
            target_revision_seq: 6,
            ..
        })
    ));

    let mut revision_zero_static_inventory = provider_rules();
    revision_zero_static_inventory["static_inventory_models"] = json!(["gpt-special"]);
    let mut files = complete_files();
    files[1] = file(CatalogKind::ProviderRules, revision_zero_static_inventory);
    assert!(matches!(
        build(files),
        Err(CatalogBuildError::InvalidValue { field, .. }) if field == "schema_revision"
    ));

    let mut revision_one_static_inventory = provider_rules();
    revision_one_static_inventory["schema_revision"] = json!(1);
    revision_one_static_inventory["static_inventory_models"] = json!(["gpt-special"]);
    let mut files = complete_files();
    files[1] = file(CatalogKind::ProviderRules, revision_one_static_inventory);
    let snapshot = build(files).unwrap();
    assert_eq!(
        snapshot
            .provider_rules("openai")
            .unwrap()
            .static_inventory_models,
        vec!["gpt-special".to_owned()]
    );

    let missing_driver = vec![file(CatalogKind::ProviderRules, provider_rules())];
    assert!(matches!(
        build(missing_driver),
        Err(CatalogBuildError::UnknownReference {
            field: "variants.model_driver",
            ..
        })
    ));

    let missing_rules = vec![
        file(
            CatalogKind::ModelDriver,
            model_driver("openai", json!([]), json!([])),
        ),
        file(CatalogKind::KnownProvider, known_providers()),
    ];
    assert!(matches!(
        build(missing_rules),
        Err(CatalogBuildError::UnknownReference {
            field: "providers.provider_rules_id",
            ..
        })
    ));
}

#[test]
fn static_catalogs_reject_dynamic_discovery_facts_and_capability_additions() {
    let mut dynamic_top_level = model_driver("openai", json!([]), json!([]));
    dynamic_top_level["availability"] = json!("available");
    assert!(matches!(
        build(vec![file(CatalogKind::ModelDriver, dynamic_top_level)]),
        Err(CatalogBuildError::InvalidJson { .. })
    ));

    let dynamic_capability = model_driver(
        "openai",
        json!([{
            "id": "gpt",
            "capabilities": {"health": "available"}
        }]),
        json!([]),
    );
    assert!(matches!(
        build(vec![file(CatalogKind::ModelDriver, dynamic_capability)]),
        Err(CatalogBuildError::StaticDynamicBoundary { .. })
    ));

    let mut capability_addition = provider_rules();
    capability_addition["patterns"][0]["capabilities"] = json!({"tool_call": true});
    assert!(matches!(
        build(vec![file(CatalogKind::ProviderRules, capability_addition)]),
        Err(CatalogBuildError::InvalidJson { .. })
    ));
}

#[test]
fn all_nested_match_rules_compile_during_snapshot_build() {
    let mut invalid_request_rule = provider_rules();
    invalid_request_rule["patterns"][0]["request_rules"][0]["when"] =
        json!({"unknown_option": true});
    let files = vec![
        file(
            CatalogKind::ModelDriver,
            model_driver("openai", json!([]), json!([])),
        ),
        file(CatalogKind::ProviderRules, invalid_request_rule),
    ];
    assert!(matches!(build(files), Err(CatalogBuildError::Match(_))));

    let mut invalid_model_pattern = model_driver("openai", json!([]), json!([]));
    invalid_model_pattern["patterns"] = json!([{
        "match": {"provider_model_id": "gpt-*"}
    }]);
    assert!(matches!(
        build(vec![file(CatalogKind::ModelDriver, invalid_model_pattern)]),
        Err(CatalogBuildError::InvalidValue {
            field: "patterns",
            ..
        })
    ));

    let mut conditional_model_price = model_driver("openai", json!([{"id": "gpt"}]), json!([]));
    conditional_model_price["model_pricing"] = json!([{
        "id": "gpt",
        "pricing": {
            "currency": "USD",
            "rules": [{"when": {"/quality": "high"}, "amount": 1.0}]
        }
    }]);
    assert!(matches!(
        build(vec![file(
            CatalogKind::ModelDriver,
            conditional_model_price
        )]),
        Err(CatalogBuildError::InvalidJson { .. })
    ));
}

#[test]
fn malformed_files_and_duplicate_identities_fail_atomically() {
    assert!(matches!(
        CatalogSnapshot::from_current_files(
            1,
            vec![CurrentCatalogFile {
                kind: CatalogKind::ModelDriver,
                contents: b"{".to_vec(),
            }],
            &CatalogBuildOptions::default(),
        ),
        Err(CatalogBuildError::InvalidJson { .. })
    ));

    let duplicate = model_driver("openai", json!([]), json!([]));
    assert!(matches!(
        build(vec![
            file(CatalogKind::ModelDriver, duplicate.clone()),
            file(CatalogKind::ModelDriver, duplicate),
        ]),
        Err(CatalogBuildError::DuplicateCatalog { .. })
    ));

    let duplicate_exact = model_driver("openai", json!([{"id": "gpt"}, {"id": "gpt"}]), json!([]));
    assert!(matches!(
        build(vec![file(CatalogKind::ModelDriver, duplicate_exact)]),
        Err(CatalogBuildError::DuplicateExactRule { .. })
    ));

    assert!(build(vec![file(
        CatalogKind::ModelDriver,
        model_driver("openai", json!([]), json!([{"match": "gpt-*"}]))
    )])
    .is_err());
}

#[test]
fn claude_versions_follow_both_official_naming_orders() {
    assert_eq!(
        ModelVersion::from_model_id("claude", "claude-3-5-sonnet-20241022")
            .unwrap()
            .decimal_rank(),
        Some(350)
    );
    assert_eq!(
        ModelVersion::from_model_id("claude", "claude-sonnet-4-6")
            .unwrap()
            .decimal_rank(),
        Some(460)
    );
}

#[test]
fn provider_identity_matching_examples_and_boundaries() {
    let catalog = crate::model::llm_tests::builtin_catalog();
    for (id, driver, model) in [
        ("gpt-5.6", "openai", "gpt-5.6"),
        ("gpt-5.6-2026-05-01", "openai", "gpt-5.6"),
        ("gpt-5.6-sol-2026-05-01", "openai", "gpt-5.6-sol"),
        ("accounts/fw/models/kimi-k2.6", "kimi", "kimi-k2.6"),
        ("minimax-m2.7", "minimax", "MiniMax-M2.7"),
        ("prefix/GPT-5.6-20260501", "openai", "gpt-5.6"),
        ("gpt-5.6-260501", "openai", "gpt-5.6"),
        ("gpt-5.6-0501", "openai", "gpt-5.6"),
    ] {
        assert_eq!(
            catalog.match_model(id),
            Ok(ModelIdentity {
                model_driver_id: driver.into(),
                model_id: model.into()
            }),
            "{id}"
        );
    }
    for id in [
        "stable/gpt-5-6",
        "gpt-5.6-mini",
        "claude-haiku-4-5",
        "my-private-finetune",
        "agpt-5.6",
        "gpt-5.6.1",
        "gpt-5.6-6",
        "gpt-5.6-preview",
    ] {
        assert_eq!(
            catalog.match_model(id),
            Err(ModelMatchFailure::NoMatch),
            "{id}"
        );
    }
    let ambiguous = build(vec![
        file(
            CatalogKind::ModelDriver,
            model_driver("a", json!([{"id":"same"},{"id":"x-5-1"}]), json!([])),
        ),
        file(
            CatalogKind::ModelDriver,
            model_driver("b", json!([{"id":"same"},{"id":"x-5.1"}]), json!([])),
        ),
    ])
    .unwrap();
    for id in ["same", "channel/same-20260501"] {
        assert!(
            matches!(ambiguous.match_model(id), Err(ModelMatchFailure::Ambiguous { candidates }) if candidates.len() == 2)
        );
    }
    assert!(
        matches!(ambiguous.find_by_normalized_id("x-5-1", |id| id.replace('.', "-").to_lowercase()), Err(ModelMatchFailure::Ambiguous { candidates }) if candidates.len() == 2)
    );
    let absent = build(vec![file(
        CatalogKind::ModelDriver,
        model_driver("a", json!([{"id":"gpt-5"}]), json!([])),
    )])
    .unwrap();
    assert_eq!(
        absent.match_model("gpt-5.6"),
        Err(ModelMatchFailure::NoMatch)
    );
    assert_eq!(
        absent.match_model("stable/gpt-5-6"),
        Err(ModelMatchFailure::NoMatch)
    );
}

#[test]
fn exchange_rates_are_effective_only_inside_their_validity_window() {
    let mut files = complete_files();
    let mut known = known_providers();
    known["exchange_rates"] = json!({"source_url":"https://rates.example/2026-09-25", "observed_at_ms":100, "expires_at_ms":200, "usd_per_unit":{"CNY":0.14}});
    files[0] = file(CatalogKind::KnownProvider, known);
    let catalog = build(files).unwrap();
    assert_eq!(
        catalog
            .cost_in_usd(buckyos_api::Money::new(1.0, "CNY"), 150)
            .unwrap()
            .amount,
        0.14
    );
    for time in [99, 200, 201] {
        assert!(catalog
            .cost_in_usd(buckyos_api::Money::new(1.0, "CNY"), time)
            .is_none());
    }
    assert!(catalog
        .cost_in_usd(buckyos_api::Money::new(1.0, "EUR"), 150)
        .is_none());
    assert_eq!(
        catalog
            .cost_in_usd(buckyos_api::Money::new(1.0, "USD"), 201)
            .unwrap()
            .amount,
        1.0
    );
}

#[test]
fn removed_provider_fields_and_nested_invalid_prices_are_rejected() {
    for field in [
        "metadata_drivers",
        "origin_mappings",
        "origin_provider_aliases",
    ] {
        let mut value = provider_rules();
        value[field] = json!([]);
        assert!(serde_json::from_value::<ProviderRulesCatalog>(value).is_err());
    }
    for price in [
        json!({"currency":"USD","input_token":1.0,"tiers":{"dimension":"input_tokens","steps":[{"input_token":-1.0}]}}),
        json!({"currency":"USD","input_token":1.0,"time_windows":[{"from":"00:00","to":"12:00","cache_input_token":2.0}]}),
    ] {
        assert!(validate_pricing("test", &serde_json::from_value(price).unwrap()).is_err());
    }
    let justified:Pricing=serde_json::from_value(json!({"currency":"USD","input_token":1.0,"cache_input_token":2.0,"ratio_exception":"specialized cache storage tariff"})).unwrap();
    assert!(validate_pricing("test", &justified).is_ok());
}
