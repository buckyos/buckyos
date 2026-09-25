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
        "schema_version": 1,
        "schema_revision": 0,
        "model_driver_id": id,
        "revision_seq": 7,
        "required_features": [],
        "models": models,
        "patterns": patterns,
        "defaults": {
            "api_types": ["llm"],
            "capabilities": {"streaming": true}
        },
        "variants": [],
        "version_rules": []
    })
}

fn provider_rules() -> Value {
    json!({
        "format": PROVIDER_RULES_FORMAT,
        "schema_version": 1,
        "schema_revision": 0,
        "revision_seq": 9,
        "provider_profile_id": "openai",
        "metadata_drivers": ["openai"],
        "origin_provider_aliases": {"openai": "openai"},
        "origin_mappings": [],
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

#[test]
fn provider_access_is_independent_from_protocol_rules_and_unknown_is_default() {
    let mut files = complete_files();
    let mut rules = provider_rules();
    rules["schema_revision"] = json!(1);
    rules["access_rules"] = json!([{
        "match": {"provider_model_id": "gpt-*", "policy_region": "eu"},
        "access": "denied"
    }]);
    files[1] = file(CatalogKind::ProviderRules, rules);
    let snapshot = build(files).unwrap();
    let mut eu = MatchContext::new();
    eu.insert("policy_region".into(), json!("eu"));
    assert_eq!(
        snapshot
            .resolve_provider_access("openai", "gpt-new", &eu)
            .unwrap(),
        ProviderModelAccess::Denied
    );
    let mut unknown = MatchContext::new();
    unknown.insert("policy_region".into(), json!("unknown"));
    assert_eq!(
        snapshot
            .resolve_provider_access("openai", "gpt-new", &unknown)
            .unwrap(),
        ProviderModelAccess::Unknown
    );
    assert_eq!(
        snapshot
            .resolve_provider_rule("openai", "gpt-new", &eu)
            .unwrap()
            .unwrap()
            .action
            .operations["llm"],
        "responses.create"
    );
}

fn routed_provider_rules(
    metadata_drivers: Option<Vec<&str>>,
    aliases: Value,
    mappings: Value,
) -> Value {
    json!({
        "format": PROVIDER_RULES_FORMAT,
        "schema_version": 1,
        "schema_revision": 0,
        "revision_seq": 9,
        "provider_profile_id": "router",
        "metadata_drivers": metadata_drivers,
        "origin_provider_aliases": aliases,
        "origin_mappings": mappings,
        "models": [],
        "patterns": [],
        "variants": []
    })
}

fn vendor_model_mapping(driver_transforms: Value, model_transforms: Value) -> Value {
    json!({
        "extract": {
            "source": "provider_model_id",
            "regex": "^(?<driver>[^/]+)/(?<model>.+)$"
        },
        "transforms": {
            "driver": driver_transforms,
            "model": model_transforms
        }
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
                    "api_types": ["llm", "image.txt2img"],
                    "capabilities": {"streaming": true, "tool_call": true}
                }]),
                json!([{
                    "match": "gpt-*",
                    "quality_score": 0.8
                }, {
                    "match": "gpt-5-*",
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
fn anthropic_effort_variants_exclude_unsupported_haiku() {
    let snapshot = build(vec![
        CurrentCatalogFile {
            kind: CatalogKind::ModelDriver,
            contents: include_bytes!("../../driver_metadata/models/anthropic.model.json").to_vec(),
        },
        CurrentCatalogFile {
            kind: CatalogKind::ProviderRules,
            contents: include_bytes!("../../driver_metadata/providers/claude.provider.json")
                .to_vec(),
        },
    ])
    .unwrap();
    assert_eq!(snapshot.model_driver("claude").unwrap().revision_seq, 1);
    let variants = |model: &str| {
        let context = BTreeMap::from([("origin_model_id".to_owned(), json!(model))]);
        snapshot
            .matching_model_variants("claude", &context)
            .unwrap()
            .into_iter()
            .map(|variant| variant.name.as_str())
            .collect::<Vec<_>>()
    };

    assert!(variants("claude-haiku-4-5-20251001").is_empty());
    assert_eq!(
        variants("claude-sonnet-5"),
        [
            "reasoning-low",
            "reasoning-medium",
            "reasoning-high",
            "reasoning-xhigh",
            "reasoning-max",
        ]
    );
    for model in ["claude-haiku-4-5-20251001", "claude-sonnet-5"] {
        let context = BTreeMap::from([
            ("provider_model_id".to_owned(), json!(model)),
            ("variant".to_owned(), json!("reasoning-high")),
        ]);
        let matched = snapshot
            .matching_provider_variants("claude", &context)
            .unwrap();
        assert_eq!(matched.len(), usize::from(model == "claude-sonnet-5"));
    }
}

#[test]
fn builtin_model_variant_defaults_match_origin_provider_variants() {
    let catalogs: [(&str, &[u8], &[u8]); 8] = [
        (
            "anthropic",
            include_bytes!("../../driver_metadata/models/anthropic.model.json"),
            include_bytes!("../../driver_metadata/providers/claude.provider.json"),
        ),
        (
            "deepseek",
            include_bytes!("../../driver_metadata/models/deepseek.model.json"),
            include_bytes!("../../driver_metadata/providers/deepseek.provider.json"),
        ),
        (
            "doubao",
            include_bytes!("../../driver_metadata/models/doubao.model.json"),
            include_bytes!("../../driver_metadata/providers/doubao.provider.json"),
        ),
        (
            "gemini",
            include_bytes!("../../driver_metadata/models/gemini.model.json"),
            include_bytes!("../../driver_metadata/providers/gemini.provider.json"),
        ),
        (
            "glm",
            include_bytes!("../../driver_metadata/models/glm.model.json"),
            include_bytes!("../../driver_metadata/providers/glm.provider.json"),
        ),
        (
            "kimi",
            include_bytes!("../../driver_metadata/models/kimi.model.json"),
            include_bytes!("../../driver_metadata/providers/kimi.provider.json"),
        ),
        (
            "openai",
            include_bytes!("../../driver_metadata/models/openai.model.json"),
            include_bytes!("../../driver_metadata/providers/openai.provider.json"),
        ),
        (
            "qwen",
            include_bytes!("../../driver_metadata/models/qwen.model.json"),
            include_bytes!("../../driver_metadata/providers/qwen.provider.json"),
        ),
    ];

    for (catalog_id, model_contents, provider_contents) in catalogs {
        let mut model_documents = vec![serde_json::from_slice::<Value>(model_contents).unwrap()];
        let mut files = vec![
            CurrentCatalogFile {
                kind: CatalogKind::ModelDriver,
                contents: model_contents.to_vec(),
            },
            CurrentCatalogFile {
                kind: CatalogKind::ProviderRules,
                contents: provider_contents.to_vec(),
            },
        ];
        if catalog_id == "doubao" {
            for contents in [
                include_bytes!("../../driver_metadata/models/deepseek.model.json").as_slice(),
                include_bytes!("../../driver_metadata/models/kimi.model.json").as_slice(),
                include_bytes!("../../driver_metadata/models/minimax.model.json").as_slice(),
                include_bytes!("../../driver_metadata/models/glm.model.json").as_slice(),
            ] {
                files.push(CurrentCatalogFile {
                    kind: CatalogKind::ModelDriver,
                    contents: contents.to_vec(),
                });
                model_documents.push(serde_json::from_slice(contents).unwrap());
            }
        }
        build(files).unwrap();
        let provider: Value = serde_json::from_slice(provider_contents).unwrap();
        let provider_variants = provider["variants"].as_array().unwrap();

        for provider_variant in provider_variants {
            let model_driver = provider_variant["model_driver"]
                .as_str()
                .unwrap_or(catalog_id);
            let model = model_documents
                .iter()
                .find(|model| model["model_driver_id"] == model_driver)
                .unwrap_or_else(|| panic!("missing {model_driver}.model.json for {catalog_id}"));
            let model_variants = model["variants"].as_array().unwrap();
            let variant_name = provider_variant["variant"].as_str().unwrap();
            let provider_models = &provider_variant["match"]["provider_model_id"];
            let matching_defaults = model_variants
                .iter()
                .filter(|model_variant| {
                    let model_match = &model_variant["match"];
                    let raw_origin_models =
                        model_match.get("origin_model_id").unwrap_or(model_match);
                    let origin_patterns = match raw_origin_models {
                        Value::String(value) => vec![value.as_str()],
                        Value::Array(values) => {
                            values.iter().map(|value| value.as_str().unwrap()).collect()
                        }
                        _ => panic!("model variant origin_model_id must be a string or array"),
                    };
                    let provider_patterns = match provider_models {
                        Value::String(value) => vec![value.as_str()],
                        Value::Array(values) => {
                            values.iter().map(|value| value.as_str().unwrap()).collect()
                        }
                        _ => panic!("provider variant provider_model_id must be a string or array"),
                    };
                    let compiled = crate::matching::CompiledMatchRule::compile(
                        serde_json::from_value(model_match.clone()).unwrap(),
                        &crate::matching::MODEL_DRIVER_MATCH_SCHEMA,
                    )
                    .unwrap();
                    model_variant["name"] == variant_name
                        && provider_patterns.iter().all(|provider_pattern| {
                            if provider_pattern.contains('*') || provider_pattern.contains('?') {
                                origin_patterns.contains(provider_pattern)
                            } else {
                                compiled.matches(&std::collections::BTreeMap::from([(
                                    "origin_model_id".to_owned(),
                                    Value::String((*provider_pattern).to_owned()),
                                )]))
                            }
                        })
                })
                .collect::<Vec<_>>();

            assert_eq!(
                matching_defaults.len(),
                1,
                "{catalog_id}.model.json is missing variant {variant_name:?} for {provider_models}"
            );
            if model_driver == catalog_id {
                assert_eq!(
                    matching_defaults[0]["provider_options"], provider_variant["provider_options"],
                    "{catalog_id}.model.json has different provider_options for variant {variant_name:?} and {provider_models}"
                );
            }
        }
    }
}

#[test]
fn builtin_provider_aliases_are_explicitly_excluded_from_physical_inventory() {
    let mut files = Vec::new();
    for contents in [
        include_bytes!("../../driver_metadata/models/deepseek.model.json").as_slice(),
        include_bytes!("../../driver_metadata/models/doubao.model.json").as_slice(),
        include_bytes!("../../driver_metadata/models/glm.model.json").as_slice(),
        include_bytes!("../../driver_metadata/models/kimi.model.json").as_slice(),
        include_bytes!("../../driver_metadata/models/minimax.model.json").as_slice(),
        include_bytes!("../../driver_metadata/models/qwen.model.json").as_slice(),
    ] {
        files.push(CurrentCatalogFile {
            kind: CatalogKind::ModelDriver,
            contents: contents.to_vec(),
        });
    }
    for contents in [
        include_bytes!("../../driver_metadata/providers/deepseek.provider.json").as_slice(),
        include_bytes!("../../driver_metadata/providers/doubao.provider.json").as_slice(),
        include_bytes!("../../driver_metadata/providers/glm.provider.json").as_slice(),
        include_bytes!("../../driver_metadata/providers/kimi.provider.json").as_slice(),
        include_bytes!("../../driver_metadata/providers/minimax.provider.json").as_slice(),
        include_bytes!("../../driver_metadata/providers/qwen.provider.json").as_slice(),
    ] {
        files.push(CurrentCatalogFile {
            kind: CatalogKind::ProviderRules,
            contents: contents.to_vec(),
        });
    }
    let snapshot = build(files).unwrap();

    for (profile, alias) in [
        ("deepseek", "deepseek-chat"),
        ("deepseek", "deepseek-reasoner"),
        ("doubao", "glm-latest"),
        ("glm", "glm-latest"),
        ("kimi", "kimi-latest"),
        ("kimi", "moonshot-v1-auto"),
        ("qwen", "qwen-plus-latest"),
    ] {
        let rule = snapshot
            .resolve_provider_rule(profile, alias, &MatchContext::new())
            .unwrap()
            .unwrap_or_else(|| panic!("{profile} is missing an exclusion for {alias}"));
        assert!(rule.action.exclude, "{profile} must exclude {alias}");
    }

    for (profile, model) in [
        ("deepseek", "deepseek-v4-flash"),
        ("doubao", "glm-5.3"),
        ("glm", "glm-5.3"),
        ("kimi", "kimi-k3"),
        ("minimax", "MiniMax-M3"),
        ("qwen", "qwen3.8-max"),
    ] {
        let rule = snapshot
            .resolve_provider_rule(profile, model, &MatchContext::new())
            .unwrap()
            .unwrap_or_else(|| panic!("{profile} is missing a rule for {model}"));
        assert!(
            !rule.action.exclude,
            "{profile} must retain physical model {model}"
        );
    }
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

    let exact = first
        .resolve_model("gpt-special", None, &MatchContext::new())
        .unwrap();
    assert_eq!(exact.match_kind, ModelMatchKind::Exact);
    assert_eq!(exact.model_driver_id.as_deref(), Some("openai"));
    assert_eq!(
        exact.semantics.api_types.unwrap(),
        BTreeSet::from(["image.txt2img".to_owned(), "llm".to_owned()])
    );

    let candidates = vec!["openai".to_owned()];
    let pattern = first
        .resolve_model("gpt-5-new", Some(&candidates), &MatchContext::new())
        .unwrap();
    assert_eq!(pattern.match_kind, ModelMatchKind::Pattern);
    assert_eq!(pattern.trace.as_ref().unwrap().position, 0);
    assert_eq!(pattern.semantics.quality_score, Some(0.8));
    assert_eq!(
        pattern,
        second
            .resolve_model("gpt-5-new", Some(&candidates), &MatchContext::new(),)
            .unwrap()
    );

    let defaults = first
        .resolve_model("unknown", Some(&candidates), &MatchContext::new())
        .unwrap();
    assert_eq!(defaults.match_kind, ModelMatchKind::Defaults);
    assert_eq!(
        defaults.semantics.capabilities.unwrap()["streaming"],
        json!(true)
    );

    let no_candidates = Vec::new();
    let fallback = first
        .resolve_model("unknown", Some(&no_candidates), &MatchContext::new())
        .unwrap();
    assert_eq!(fallback.match_kind, ModelMatchKind::ConservativeFallback);
    assert!(fallback.model_driver_id.is_none());
    assert!(fallback.semantics.api_types.unwrap().is_empty());
    assert!(fallback.semantics.capabilities.unwrap().is_empty());
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
fn provider_variants_override_model_variants_per_matching_model() {
    let mut model = model_driver("openai", json!([]), json!([]));
    model["variants"] = json!([
        {"name": "model-only", "match": "gpt-*"},
        {"name": "shared", "match": "gpt-*", "mount_suffix": "shared"}
    ]);
    let mut rules = provider_rules();
    rules["variants"] = json!([
        {
            "model_driver": "openai",
            "variant": "provider-only",
            "match": "gpt-provider-*",
            "provider_options": {"reasoning": {"effort": "high"}}
        },
        {
            "model_driver": "openai",
            "variant": "shared",
            "match": "gpt-provider-*",
            "provider_options": {"reasoning": {"effort": "medium"}}
        }
    ]);
    let snapshot = build(vec![
        file(CatalogKind::ModelDriver, model),
        file(CatalogKind::ProviderRules, rules),
    ])
    .unwrap();

    let provider_context = MatchContext::from([
        ("provider_model_id".to_owned(), json!("gpt-provider-1")),
        ("origin_model_id".to_owned(), json!("gpt-provider-1")),
    ]);
    let effective = snapshot
        .effective_model_variants(Some("openai"), "openai", &provider_context)
        .unwrap();
    assert!(effective.provider_override);
    assert_eq!(
        effective
            .variants
            .iter()
            .map(EffectiveModelVariant::name)
            .collect::<Vec<_>>(),
        ["provider-only", "shared"]
    );
    assert_eq!(
        effective.variants[1]
            .model
            .and_then(|model| model.mount_suffix.as_deref()),
        Some("shared")
    );

    let fallback_context = MatchContext::from([
        ("provider_model_id".to_owned(), json!("other-model")),
        ("origin_model_id".to_owned(), json!("gpt-fallback")),
    ]);
    let effective = snapshot
        .effective_model_variants(Some("openai"), "openai", &fallback_context)
        .unwrap();
    assert!(!effective.provider_override);
    assert_eq!(
        effective
            .variants
            .iter()
            .map(EffectiveModelVariant::name)
            .collect::<Vec<_>>(),
        ["model-only", "shared"]
    );
}

#[test]
fn exact_model_wins_globally_and_cross_driver_conflicts_are_rejected() {
    let files = vec![
        file(
            CatalogKind::ModelDriver,
            model_driver("openai", json!([{"id": "shared-model"}]), json!([])),
        ),
        file(
            CatalogKind::ModelDriver,
            model_driver("claude", json!([]), json!([{"match": "shared-*"}])),
        ),
    ];
    let snapshot = build(files).unwrap();
    let resolved = snapshot
        .resolve_model("shared-model", None, &MatchContext::new())
        .unwrap();
    assert_eq!(resolved.match_kind, ModelMatchKind::Exact);
    assert_eq!(resolved.model_driver_id.as_deref(), Some("openai"));

    let conflict = build(vec![
        file(
            CatalogKind::ModelDriver,
            model_driver("openai", json!([{"id": "shared-model"}]), json!([])),
        ),
        file(
            CatalogKind::ModelDriver,
            model_driver("claude", json!([{"id": "shared-model"}]), json!([])),
        ),
    ])
    .unwrap();
    assert_eq!(
        conflict
            .resolve_model("shared-model", None, &MatchContext::new())
            .unwrap_err(),
        CatalogResolveError::AmbiguousModelDrivers {
            origin_model_id: "shared-model".to_owned(),
            model_driver_ids: vec!["claude".to_owned(), "openai".to_owned()],
        }
    );

    let pattern_conflict = build(vec![
        file(
            CatalogKind::ModelDriver,
            model_driver("openai", json!([]), json!([{"match": "shared-*"}])),
        ),
        file(
            CatalogKind::ModelDriver,
            model_driver("claude", json!([]), json!([{"match": "shared-*"}])),
        ),
    ])
    .unwrap();
    assert!(matches!(
        pattern_conflict.resolve_model("shared-model", None, &MatchContext::new()),
        Err(CatalogResolveError::AmbiguousModelDrivers { .. })
    ));
}

#[test]
fn provider_origin_mapping_uniquely_selects_driver_for_shared_model_id() {
    let rules = routed_provider_rules(
        None,
        json!({"anthropic": "claude", "openai": "openai"}),
        json!([vendor_model_mapping(
            json!([
                {"op": "lowercase"},
                {"op": "alias", "table": "origin_provider_aliases"}
            ]),
            json!([{"op": "trim"}])
        )]),
    );
    let snapshot = build(vec![
        file(
            CatalogKind::ModelDriver,
            model_driver("claude", json!([{"id": "shared-model"}]), json!([])),
        ),
        file(
            CatalogKind::ModelDriver,
            model_driver("openai", json!([{"id": "shared-model"}]), json!([])),
        ),
        file(CatalogKind::ProviderRules, rules),
    ])
    .unwrap();

    let origin = snapshot
        .resolve_provider_origin("router", "ANTHROPIC/shared-model")
        .unwrap();
    assert_eq!(
        origin,
        ResolvedProviderOrigin {
            origin_model_id: "shared-model".to_owned(),
            model_driver_id: "claude".to_owned(),
        }
    );
    let candidates = vec![origin.model_driver_id];
    let model = snapshot
        .resolve_model(
            &origin.origin_model_id,
            Some(&candidates),
            &MatchContext::new(),
        )
        .unwrap();
    assert_eq!(model.model_driver_id.as_deref(), Some("claude"));
}

#[test]
fn provider_origin_mapping_rejects_unknown_vendor_and_conflicts() {
    let alias_mapping = vendor_model_mapping(
        json!([
            {"op": "lowercase"},
            {"op": "alias", "table": "origin_provider_aliases"}
        ]),
        json!([]),
    );
    let unknown_snapshot = build(vec![
        file(
            CatalogKind::ModelDriver,
            model_driver("claude", json!([{"id": "shared-model"}]), json!([])),
        ),
        file(
            CatalogKind::ProviderRules,
            routed_provider_rules(
                None,
                json!({"anthropic": "claude"}),
                json!([alias_mapping.clone()]),
            ),
        ),
    ])
    .unwrap();
    assert_eq!(
        unknown_snapshot
            .resolve_provider_origin("router", "unknown/shared-model")
            .unwrap_err(),
        CatalogResolveError::UnknownOriginProvider {
            provider_profile_id: "router".to_owned(),
            origin_provider: "unknown".to_owned(),
        }
    );

    let conflict_snapshot = build(vec![
        file(
            CatalogKind::ModelDriver,
            model_driver("claude", json!([{"id": "shared-model"}]), json!([])),
        ),
        file(
            CatalogKind::ModelDriver,
            model_driver("anthropic", json!([{"id": "shared-model"}]), json!([])),
        ),
        file(
            CatalogKind::ProviderRules,
            routed_provider_rules(
                None,
                json!({"anthropic": "claude"}),
                json!([
                    alias_mapping,
                    vendor_model_mapping(json!([{"op": "lowercase"}]), json!([]))
                ]),
            ),
        ),
    ])
    .unwrap();
    assert!(matches!(
        conflict_snapshot.resolve_provider_origin("router", "ANTHROPIC/shared-model"),
        Err(CatalogResolveError::ConflictingOriginMappings { resolved, .. })
            if resolved.len() == 2
    ));
}

#[test]
fn provider_origin_aliases_cannot_escape_metadata_drivers() {
    let rules = routed_provider_rules(
        Some(vec!["openai"]),
        json!({"anthropic": "claude"}),
        json!([vendor_model_mapping(
            json!([{"op": "alias", "table": "origin_provider_aliases"}]),
            json!([])
        )]),
    );
    assert!(matches!(
        build(vec![
            file(
                CatalogKind::ModelDriver,
                model_driver("openai", json!([]), json!([])),
            ),
            file(
                CatalogKind::ModelDriver,
                model_driver("claude", json!([]), json!([])),
            ),
            file(CatalogKind::ProviderRules, rules),
        ]),
        Err(CatalogBuildError::InvalidValue {
            field: "origin_provider_aliases",
            ..
        })
    ));

    let snapshot = build(vec![
        file(
            CatalogKind::ModelDriver,
            model_driver("openai", json!([]), json!([])),
        ),
        file(
            CatalogKind::ModelDriver,
            model_driver("claude", json!([]), json!([])),
        ),
        file(
            CatalogKind::ProviderRules,
            routed_provider_rules(
                Some(vec!["openai"]),
                json!({}),
                json!([vendor_model_mapping(json!([]), json!([]))]),
            ),
        ),
    ])
    .unwrap();
    assert_eq!(
        snapshot
            .resolve_provider_origin("router", "claude/shared-model")
            .unwrap_err(),
        CatalogResolveError::OriginDriverOutsideMetadataDrivers {
            provider_profile_id: "router".to_owned(),
            model_driver_id: "claude".to_owned(),
        }
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
        Err(CatalogBuildError::InvalidValue { .. })
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

    let mut revision_zero_access_rules = provider_rules();
    revision_zero_access_rules["access_rules"] = json!([{
        "match": {"provider_model_id": "gpt-*", "policy_region": "eu"},
        "access": "denied"
    }]);
    let mut files = complete_files();
    files[1] = file(CatalogKind::ProviderRules, revision_zero_access_rules);
    assert!(matches!(
        build(files),
        Err(CatalogBuildError::InvalidValue { field, .. }) if field == "schema_revision"
    ));

    let mut revision_one_static_inventory = provider_rules();
    revision_one_static_inventory["schema_revision"] = json!(1);
    revision_one_static_inventory["static_inventory_models"] = json!(["gpt-special"]);
    revision_one_static_inventory["models"] = json!([]);
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

    let mut uncovered_static_inventory = provider_rules();
    uncovered_static_inventory["schema_revision"] = json!(1);
    uncovered_static_inventory["static_inventory_models"] = json!(["claude-special"]);
    uncovered_static_inventory["models"] = json!([]);
    let mut files = complete_files();
    files[1] = file(CatalogKind::ProviderRules, uncovered_static_inventory);
    assert!(matches!(
        build(files),
        Err(CatalogBuildError::InvalidValue { field, .. }) if field == "static_inventory_models"
    ));

    let mut revision_zero_custom_adapters = provider_rules();
    revision_zero_custom_adapters["custom_provider_adapters"] = json!(["openai-responses"]);
    let mut files = complete_files();
    files[1] = file(CatalogKind::ProviderRules, revision_zero_custom_adapters);
    assert!(matches!(
        build(files),
        Err(CatalogBuildError::InvalidValue { field, .. }) if field == "schema_revision"
    ));

    let mut duplicate_custom_adapters = provider_rules();
    duplicate_custom_adapters["schema_revision"] = json!(1);
    duplicate_custom_adapters["custom_provider_adapters"] =
        json!(["openai-responses", "openai-responses"]);
    let mut files = complete_files();
    files[1] = file(CatalogKind::ProviderRules, duplicate_custom_adapters);
    assert!(matches!(
        build(files),
        Err(CatalogBuildError::InvalidValue { field, .. })
            if field == "custom_provider_adapters"
    ));

    let missing_driver = vec![file(CatalogKind::ProviderRules, provider_rules())];
    assert!(matches!(
        build(missing_driver),
        Err(CatalogBuildError::UnknownReference {
            field: "metadata_drivers",
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
        Err(CatalogBuildError::Match(_))
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
        Err(CatalogBuildError::InvalidValue {
            field: "model_pricing.pricing.rules",
            ..
        })
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

    let snapshot = build(vec![file(
        CatalogKind::ModelDriver,
        model_driver("openai", json!([]), json!([{"match": "gpt-*"}])),
    )])
    .unwrap();
    let duplicate_candidates = vec!["openai".to_owned(), "openai".to_owned()];
    assert_eq!(
        snapshot
            .resolve_model("gpt-new", Some(&duplicate_candidates), &MatchContext::new(),)
            .unwrap()
            .match_kind,
        ModelMatchKind::Pattern
    );
}

#[test]
fn logical_mounts_are_available_without_a_provider_instance() {
    let driver = model_driver(
        "openai",
        json!([{
            "id": "gpt-test",
            "api_types": ["llm"],
            "logical_mounts": ["llm.{driver}.{model}", "llm.openai"]
        }]),
        json!([]),
    );
    let snapshot = build(vec![file(CatalogKind::ModelDriver, driver)]).unwrap();
    let view = snapshot.logical_mounts_for("openai", "gpt-test").unwrap();
    assert_eq!(view.origin_model_id, "gpt-test");
    assert_eq!(
        view.logical_mounts,
        vec!["llm.openai".to_owned(), "llm.openai.gpt-test".to_owned()]
    );
}

#[test]
fn invalid_logical_mount_templates_fail_at_catalog_load() {
    let driver = model_driver(
        "openai",
        json!([{
            "id": "gpt-test",
            "logical_mounts": ["llm.{unknown}"]
        }]),
        json!([]),
    );
    assert!(matches!(
        build(vec![file(CatalogKind::ModelDriver, driver)]),
        Err(CatalogBuildError::InvalidValue {
            field: "logical_mounts",
            ..
        })
    ));
}
