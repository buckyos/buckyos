use super::*;
use crate::catalog::{CatalogBuildOptions, CatalogDocuments, ModelDriverCatalog};
use serde_json::json;

pub(crate) fn openai_document() -> Value {
    serde_json::from_slice(include_bytes!(
        "../../driver_metadata/models/openai.model.json"
    ))
    .unwrap()
}

pub(crate) fn documents() -> Vec<ModelDriverCatalog> {
    std::fs::read_dir(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/driver_metadata/models"
    ))
    .unwrap()
    .map(|entry| serde_json::from_slice(&std::fs::read(entry.unwrap().path()).unwrap()).unwrap())
    .collect()
}

pub(crate) fn compile(
    documents: Vec<ModelDriverCatalog>,
) -> Result<CatalogSnapshot, crate::error::CatalogBuildError> {
    CatalogSnapshot::build(
        100,
        CatalogDocuments {
            model_drivers: documents,
            provider_rules: vec![],
            known_providers: vec![],
        },
        &CatalogBuildOptions::default(),
    )
}

pub(crate) fn builtin_catalog() -> CatalogSnapshot {
    compile(documents()).unwrap()
}

pub(crate) fn definition(path: &str) -> LogicalModelDefinition {
    LogicalModelDefinition {
        path: path.into(),
        api_type: ApiType::Llm,
        min_line: ModelRequirement::default(),
        disable_line: ModelDisable::default(),
        default_options: BTreeMap::new(),
        mount_mode: MountMode::Manual,
        scheduler_profile: AiccSchedulerProfile::Balanced,
        fallback: None,
        route_policy: AiccPolicyConfig::default(),
        user_visible_tier: None,
    }
}

pub(crate) fn gpt_overlay() -> AiccRouteOverlay {
    let items = ["nano", "mini", "standard", "pro", "max", "codex"]
        .into_iter()
        .enumerate()
        .map(|(index, spec)| {
            (
                spec.to_owned(),
                ModelItem::new(format!("llm.gpt-{spec}"), index as f64 + 1.0),
            )
        })
        .collect();
    AiccRouteOverlay {
        logical_tree: BTreeMap::from([(
            "llm.chat".into(),
            AiccLogicalNodeOverlay {
                items: Some(items),
                ..Default::default()
            },
        )]),
        ..Default::default()
    }
}

pub(crate) fn gpt_registry(inventories: &[ProviderInventory]) -> ModelRegistry {
    let catalog = compile(vec![serde_json::from_value(openai_document()).unwrap()]).unwrap();
    ModelRegistry::build(
        &catalog,
        inventories,
        vec![
            definition("llm"),
            definition("llm.chat"),
            definition("llm.plan"),
        ],
        RegistryLayers {
            factory: Some(&gpt_overlay()),
            ..Default::default()
        },
    )
    .unwrap()
}

pub(crate) fn inventory(
    driver: &str,
    origin: &str,
    channel: &str,
    instance: &str,
    efforts: &[&str],
) -> ProviderInventory {
    let catalog = builtin_catalog();
    let semantics = catalog.resolve_model(driver, origin).unwrap().semantics;
    ProviderInventory {
        provider_instance_name: instance.into(),
        provider_profile_id: "synthetic".into(),
        protocol_adapter_id: "synthetic".into(),
        inventory_revision: "1".into(),
        models: vec![InventoryModel {
            provider_model_id: channel.into(),
            model_driver_id: driver.into(),
            origin_model_id: origin.into(),
            api_types: vec![ApiType::Llm],
            logical_mounts: vec!["llm.plan".into()],
            variants: efforts
                .iter()
                .map(|effort| InventoryModelVariant {
                    name: format!("reasoning-{effort}"),
                    logical_mounts: vec!["llm.plan.high".into()],
                })
                .collect(),
            capabilities: semantics.capabilities.unwrap_or_default(),
            canonical_fields: semantics.canonical_fields.unwrap_or_default(),
            attributes: BTreeMap::new(),
            operations: BTreeMap::from([("llm".into(), "responses.create".into())]),
        }],
    }
}

#[test]
fn z01_z02_real_openai_catalog_and_empty_specifications_need_no_provider() {
    let catalog = compile(vec![serde_json::from_value(openai_document()).unwrap()]).unwrap();
    assert!(catalog.provider_rules("openai").is_none());
    assert!(catalog.known_provider("openai").is_none());
    assert_eq!(catalog.model_driver("openai").unwrap().specs.len(), 6);
    let model = catalog.llm_model("openai", "gpt-5.6-sol").unwrap();
    assert_eq!(model.family, "llm.gpt-5-6-sol");
    assert_eq!(model.version.unwrap().decimal_rank(), Some(560));
    assert_eq!(
        model.semantics.effort.variant().as_deref(),
        Some("reasoning-high")
    );
    let registry = gpt_registry(&[]);
    assert!(registry.model_views().is_empty());
    let views = registry.logical_model_views();
    assert!(!views.iter().any(|node| node.path == model.family));
    for spec in &catalog.model_driver("openai").unwrap().specs {
        let path = format!("llm.{}", spec.id);
        assert!(views
            .iter()
            .any(|node| node.path == path && node.items.is_empty()));
        assert!(registry
            .resolve_candidates(&path, ApiType::Llm)
            .unwrap()
            .candidates
            .is_empty());
    }
    let chat = views.iter().find(|node| node.path == "llm.chat").unwrap();
    assert_eq!(chat.items.len(), 6);
    assert!(chat
        .items
        .iter()
        .any(|item| item.target == "llm.gpt-pro" && item.weight == 4.0));
    for path in ["llm", "llm.plan", "llm.chat", "llm.missing"] {
        let result = registry.resolve_candidates(path, ApiType::Llm).unwrap();
        assert!(result.candidates.is_empty());
        assert!(result.fallback_chain.is_empty());
    }
}

#[test]
fn z06_z08_invalid_catalogs_fail_without_inventory() {
    for (name, edit) in [
        ("duplicate spec", 0),
        ("unknown spec", 1),
        ("missing llm", 2),
        ("invalid effort", 3),
        ("duplicate effort", 4),
        ("family collision", 5),
        ("model_pricing", 6),
        ("pricing", 7),
        ("variants", 8),
        ("version_order", 9),
        ("v1", 10),
        ("version_rules", 11),
    ] {
        let mut value = openai_document();
        match edit {
            0 => {
                let spec = value["specs"][0].clone();
                value["specs"].as_array_mut().unwrap().push(spec);
            }
            1 => value["models"][0]["llm"]["spec"] = json!("missing"),
            2 => {
                value["models"][0].as_object_mut().unwrap().remove("llm");
            }
            3 => value["models"][0]["llm"]["effort"] = json!("invalid"),
            4 => value["models"][0]["llm"]["supported_efforts"] = json!(["none", "none"]),
            5 => value["models"][0]["llm"]["family_id"] = json!("gpt-5-6-sol"),
            6 => value["model_pricing"] = json!([]),
            7 => value["models"][0]["pricing"] = json!({"currency":"USD","amount":0}),
            8 => value["variants"] = json!([]),
            9 => value["models"][0]["llm"]["version_order"] = json!(900),
            10 => value["schema_version"] = json!(1),
            _ => value["version_rules"] = json!([]),
        }
        let result = serde_json::from_value(value)
            .map_err(|error| error.to_string())
            .and_then(|document| compile(vec![document]).map_err(|error| error.to_string()));
        assert!(result.is_err(), "{name}");
        let error = result.unwrap_err();
        assert!(!error.is_empty(), "{name}");
        if edit <= 5 && edit != 3 {
            assert!(error.contains("openai"), "{name}: {error}");
        }
    }
}

#[test]
fn z07_specs_require_explicit_admission_and_direct_only_cannot_be_bypassed() {
    let mut value = openai_document();
    let document = serde_json::from_value(value.clone()).unwrap();
    let catalog = compile(vec![document]).unwrap();
    assert!(ModelRegistry::build(&catalog, &[], vec![], RegistryLayers::default()).is_err());
    for spec in value["specs"].as_array_mut().unwrap() {
        spec["direct_only"] = json!(true);
    }
    let catalog = compile(vec![serde_json::from_value(value).unwrap()]).unwrap();
    ModelRegistry::build(&catalog, &[], vec![], RegistryLayers::default()).unwrap();
    assert!(ModelRegistry::build(
        &catalog,
        &[],
        vec![definition("llm.chat")],
        RegistryLayers {
            factory: Some(&gpt_overlay()),
            ..Default::default()
        }
    )
    .is_err());
    let mut task = definition("llm.plan");
    task.fallback = Some(AiccFallbackRule {
        mode: AiccFallbackMode::TargetLogical,
        target: Some("llm.gpt-pro".into()),
    });
    assert!(ModelRegistry::build(&catalog, &[], vec![task], RegistryLayers::default()).is_err());
    let mut value = openai_document();
    value["models"][0]["llm"]["family_id"] = json!("plan");
    let catalog = compile(vec![serde_json::from_value(value).unwrap()]).unwrap();
    assert!(ModelRegistry::build(
        &catalog,
        &[],
        vec![definition("llm.plan")],
        RegistryLayers {
            factory: Some(&gpt_overlay()),
            ..Default::default()
        }
    )
    .is_err());
    let registry = gpt_registry(&[]);
    let cycle: AiccRouteOverlay = serde_json::from_value(json!({"logical_tree": {"llm.plan":{"fallback":{"mode":"target_logical","target":"llm.chat"}},"llm.chat":{"fallback":{"mode":"target_logical","target":"llm.plan"}}}})).unwrap();
    assert!(registry.with_session_overlay(&cycle).is_err());
}

#[test]
fn d01_d02_d03_d10_families_share_instances_and_disappear_only_after_last_inventory() {
    let a = inventory("openai", "gpt-5.6-sol", "channel/A", "official", &["high"]);
    let b = inventory(
        "openai",
        "gpt-5.6-sol",
        "vendor/model-alias",
        "third-party",
        &["high"],
    );
    for stock in [vec![], vec![a.clone(), b.clone()], vec![b], vec![]] {
        let registry = gpt_registry(&stock);
        let views = registry.logical_model_views();
        let spec = views
            .iter()
            .find(|node| node.path == "llm.gpt-pro")
            .unwrap();
        assert_eq!(spec.items.len(), usize::from(!stock.is_empty()));
        assert_eq!(
            views.iter().any(|node| node.path == "llm.gpt-5-6-sol"),
            !stock.is_empty()
        );
        assert!(!views.iter().any(|node| node.path.ends_with(".high")));
        let candidates = registry
            .resolve_candidates("llm.gpt-pro", ApiType::Llm)
            .unwrap()
            .candidates;
        assert_eq!(candidates.len(), stock.len());
        for candidate in candidates {
            assert_eq!(
                candidate.model.exact_model.variant(),
                Some("reasoning-high")
            );
        }
        assert!(registry
            .resolve_candidates("llm.openai.gpt-5-6-sol", ApiType::Llm)
            .unwrap()
            .candidates
            .is_empty());
        let again = gpt_registry(&stock);
        assert_eq!(registry.logical_model_views(), again.logical_model_views());
    }
    let registry = gpt_registry(&[a]);
    let fixed = registry
        .resolve_candidates("llm.gpt-5-6-sol:high", ApiType::Llm)
        .unwrap();
    assert_eq!(
        fixed.candidates[0].model.exact_model.as_str(),
        "channel/A:reasoning-high@official"
    );
}

#[test]
fn d07_d08_efforts_never_fabricate_channel_support_or_bypass_task_membership() {
    let model = inventory("openai", "gpt-5.6-sol", "sol", "a", &["low"]);
    let registry = gpt_registry(&[model.clone()]);
    assert!(registry
        .resolve_candidates("llm.gpt-pro", ApiType::Llm)
        .unwrap()
        .candidates
        .is_empty());
    assert_eq!(
        registry
            .resolve_candidates("llm.gpt-5-6-sol:low", ApiType::Llm)
            .unwrap()
            .candidates
            .len(),
        1
    );
    assert!(registry
        .resolve_candidates("llm.plan", ApiType::Llm)
        .unwrap()
        .candidates
        .is_empty());
    let bypass: AiccRouteOverlay = serde_json::from_value(json!({"logical_tree":{"llm.plan":{"items":{"bypass":{"target":"sol:reasoning-low@a","weight":100}}}}})).unwrap();
    assert!(registry.with_session_overlay(&bypass).is_err());
    for mode in [MountMode::Auto, MountMode::Hybrid] {
        let mut task = definition("llm.chat");
        task.mount_mode = mode;
        let catalog = compile(vec![serde_json::from_value(openai_document()).unwrap()]).unwrap();
        assert!(ModelRegistry::build(
            &catalog,
            &[model.clone()],
            vec![task],
            RegistryLayers {
                factory: Some(&gpt_overlay()),
                ..Default::default()
            }
        )
        .is_err());
    }
    let mut value = openai_document();
    value["models"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|model| model["id"] == "gpt-5.6-sol")
        .unwrap()["llm"]["default_effort"] = json!("low");
    let catalog = compile(vec![serde_json::from_value(value).unwrap()]).unwrap();
    let registry = ModelRegistry::build(
        &catalog,
        &[inventory(
            "openai",
            "gpt-5.6-sol",
            "sol",
            "a",
            &["low", "high"],
        )],
        vec![definition("llm.chat")],
        RegistryLayers {
            factory: Some(&gpt_overlay()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        registry
            .resolve_candidates("llm.gpt-5-6-sol", ApiType::Llm)
            .unwrap()
            .candidates[0]
            .model
            .exact_model
            .variant(),
        Some("reasoning-low")
    );
    assert_eq!(
        registry
            .resolve_candidates("llm.gpt-pro", ApiType::Llm)
            .unwrap()
            .candidates[0]
            .model
            .exact_model
            .variant(),
        Some("reasoning-high")
    );
}

#[test]
fn d04_versions_are_numeric_ignore_dates_sizes_and_tie_break_by_family() {
    let mut value = openai_document();
    let template = value["models"][6].clone();
    let ids = [
        "gpt-5.5",
        "gpt-5.6",
        "gpt-6",
        "gpt-5.6.1",
        "gpt-5.10",
        "gpt-5.6-z",
        "gpt-5.6-20260925",
        "gpt-5.6-27b",
        "gpt-unknown",
        "gpt-20260925",
    ];
    value["models"] = Value::Array(
        ids.iter()
            .map(|id| {
                let mut model = template.clone();
                model["id"] = json!(id);
                model
            })
            .collect(),
    );
    value["specs"] = json!([{"id":"gpt-pro","direct_only":true}]);
    let catalog = compile(vec![serde_json::from_value(value.clone()).unwrap()]).unwrap();
    let versions: Vec<_> = ids
        .iter()
        .map(|id| catalog.llm_model("openai", id).unwrap().version)
        .collect();
    assert_eq!(versions[0].unwrap().decimal_rank(), Some(550));
    assert_eq!(versions[1].unwrap().decimal_rank(), Some(560));
    assert_eq!(versions[2].unwrap().decimal_rank(), Some(600));
    assert_eq!(versions[3].unwrap().decimal_rank(), Some(561));
    assert!(versions[1] < versions[4] && versions[4] < versions[2]);
    assert!(versions[4].unwrap().decimal_rank().is_none());
    assert_eq!(&versions[5..8], &[versions[1]; 3]);
    assert_eq!(&versions[8..], &[None, None]);
    let base = inventory("openai", "gpt-5.6-sol", "x", "a", &["high"]);
    let stocks: Vec<_> = ids
        .iter()
        .map(|id| {
            let mut stock = base.clone();
            stock.provider_instance_name = (*id).into();
            stock.models[0].origin_model_id = (*id).into();
            stock
        })
        .collect();
    let first = ModelRegistry::build(&catalog, &stocks, vec![], RegistryLayers::default()).unwrap();
    value["models"].as_array_mut().unwrap().reverse();
    let catalog = compile(vec![serde_json::from_value(value).unwrap()]).unwrap();
    let mut reversed = stocks;
    reversed.reverse();
    let second =
        ModelRegistry::build(&catalog, &reversed, vec![], RegistryLayers::default()).unwrap();
    assert_eq!(first.logical_model_views(), second.logical_model_views());
    let candidates = first
        .resolve_candidates("llm.gpt-pro", ApiType::Llm)
        .unwrap()
        .candidates;
    let order: Vec<_> = candidates
        .iter()
        .map(|candidate| candidate.model.identity.origin_model_id.as_str())
        .collect();
    assert_eq!(
        order,
        [
            "gpt-6",
            "gpt-5.10",
            "gpt-5.6.1",
            "gpt-5.6",
            "gpt-5.6-20260925",
            "gpt-5.6-27b",
            "gpt-5.6-z",
            "gpt-5.5",
            "gpt-20260925",
            "gpt-unknown"
        ]
    );
}

#[test]
fn finite_llm_patterns_preserve_exact_and_first_match_precedence() {
    let mut value = openai_document();
    let exact = value["models"][6].clone();
    let id = exact["id"].as_str().unwrap().to_owned();
    let mut pattern = exact.clone();
    pattern.as_object_mut().unwrap().remove("id");
    pattern["match"] = json!({"origin_model_id": [id, "gpt-5.5-pattern"]});
    pattern["llm"]["spec"] = json!("gpt-standard");
    value["models"] = json!([exact]);
    value["patterns"] = json!([pattern.clone(), pattern]);
    value["patterns"][1]["llm"]["spec"] = json!("gpt-max");
    let catalog = compile(vec![serde_json::from_value(value.clone()).unwrap()]).unwrap();
    assert_eq!(
        catalog.llm_model("openai", &id).unwrap().semantics.spec,
        "gpt-pro"
    );
    let pattern_model = catalog.llm_model("openai", "gpt-5.5-pattern").unwrap();
    assert_eq!(pattern_model.semantics.spec, "gpt-standard");
    let resolved = catalog.resolve_model("openai", "gpt-5.5-pattern").unwrap();
    assert_eq!(
        resolved.semantics.llm.as_ref().unwrap(),
        &pattern_model.semantics
    );
    let serialized = serde_json::to_value(resolved.semantics).unwrap();
    assert!(serialized.get("pricing").is_none());
    value["patterns"][0]["match"] = json!("gpt-*");
    assert!(compile(vec![serde_json::from_value(value).unwrap()]).is_err());
}

#[test]
fn d08_native_uses_base_and_thinking_is_not_medium() {
    let mut docs = documents();
    for doc in &mut docs {
        for spec in &mut doc.specs {
            spec.direct_only = true;
        }
    }
    let catalog = compile(docs).unwrap();
    assert_eq!(
        catalog
            .llm_model("qwen", "qwen3.8-2.4t-a95b")
            .unwrap()
            .semantics
            .supported_efforts,
        vec![
            crate::catalog::Effort::Low,
            crate::catalog::Effort::Medium,
            crate::catalog::Effort::Xhigh
        ]
    );
    let native = inventory("minimax", "MiniMax-M2.7", "native-channel", "a", &[]);
    let thinking = inventory(
        "qwen",
        "qwen3.6-35b-a3b",
        "thinking-channel",
        "b",
        &["thinking"],
    );
    let registry = ModelRegistry::build(
        &catalog,
        &[native, thinking],
        vec![],
        RegistryLayers::default(),
    )
    .unwrap();
    for path in [
        "llm.minimax-m2-7",
        "llm.minimax-m2-7:native",
        "llm.minimax-standard",
    ] {
        let candidates = registry
            .resolve_candidates(path, ApiType::Llm)
            .unwrap()
            .candidates;
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].model.exact_model.as_str(), "native-channel@a");
        assert!(candidates[0].model.exact_model.variant().is_none());
    }
    assert!(registry
        .resolve_candidates("llm.minimax-m2-7:high", ApiType::Llm)
        .unwrap()
        .candidates
        .is_empty());
    let candidates = registry
        .resolve_candidates("llm.qwen3-6-35b-a3b:thinking", ApiType::Llm)
        .unwrap()
        .candidates;
    assert_eq!(
        candidates[0].model.exact_model.as_str(),
        "thinking-channel:reasoning-thinking@b"
    );
    assert!(registry
        .resolve_candidates("llm.qwen3-6-35b-a3b:medium", ApiType::Llm)
        .unwrap()
        .candidates
        .is_empty());
}

#[test]
fn direct_only_exact_fallback_cannot_bypass_a_missing_effort() {
    let mut value = openai_document();
    for spec in value["specs"].as_array_mut().unwrap() {
        spec["direct_only"] = json!(true);
    }
    let catalog = compile(vec![serde_json::from_value(value).unwrap()]).unwrap();
    let stock = inventory("openai", "gpt-5.6-sol", "base", "a", &[]);
    let mut task = definition("llm.plan");
    task.fallback = Some(AiccFallbackRule {
        mode: AiccFallbackMode::TargetExact,
        target: Some("base@a".into()),
    });
    assert!(
        ModelRegistry::build(&catalog, &[stock], vec![task], RegistryLayers::default()).is_err()
    );
}

#[test]
fn d09_explicit_fallback_chain_retains_each_task_requirement() {
    let catalog = compile(vec![serde_json::from_value(openai_document()).unwrap()]).unwrap();
    let mut stock = inventory("openai", "gpt-5.6-sol", "weak", "a", &["high"]);
    stock.models[0].capabilities.remove("tool_call");
    let mut plan = definition("llm.plan");
    plan.fallback = Some(AiccFallbackRule {
        mode: AiccFallbackMode::TargetLogical,
        target: Some("llm.guard".into()),
    });
    let mut guard = definition("llm.guard");
    guard.min_line.tool_call = true;
    guard.fallback = Some(AiccFallbackRule {
        mode: AiccFallbackMode::TargetExact,
        target: Some("weak:reasoning-high@a".into()),
    });
    let registry = ModelRegistry::build(
        &catalog,
        &[stock],
        vec![definition("llm.chat"), plan, guard],
        RegistryLayers {
            factory: Some(&gpt_overlay()),
            ..Default::default()
        },
    )
    .unwrap();
    let result = registry
        .resolve_candidates("llm.plan", ApiType::Llm)
        .unwrap();
    assert_eq!(result.fallback_chain.len(), 2);
    assert!(result.candidates.is_empty());
}
