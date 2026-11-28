use super::pricing::{
    parse_entry_key, pricing_from_api, EntryKey, PinnedCreditKind, StoredPrice,
};
use super::*;
use buckyos_api::{AiCost, AiUsage, CreditChargeStatus, CreditPriceSource, ModelPolicy, ProviderPolicy};
use serde_json::json;

fn model(exact: &str, driver: &str, origin: &str) -> InventoryModel {
    let (model_part, provider) = exact.rsplit_once('@').unwrap();
    let (provider_model_id, variant) = match model_part.rsplit_once(':') {
        Some((id, variant)) => (id.to_string(), Some(variant.to_string())),
        None => (model_part.to_string(), None),
    };
    InventoryModel {
        exact_model: exact.into(),
        provider_instance_name: provider.into(),
        provider_model_id,
        variant,
        model_driver_id: driver.into(),
        origin_model_id: origin.into(),
        provider_cost: None,
    }
}

fn inventory(models: Vec<InventoryModel>) -> Inventory {
    let mut inventory = Inventory::default();
    for model in models {
        inventory
            .providers
            .entry(model.provider_instance_name.clone())
            .or_default()
            .insert(model.exact_model.clone(), model);
    }
    inventory
}

fn zone() -> Inventory {
    let mut local = model("llama@local-ollama", "", "");
    local.provider_cost = None;
    let mut sn = model("gpt-x@sn", "openai", "gpt-x");
    sn.provider_cost = Some(ProviderCostInfo {
        currency: "USD".into(),
        zero: false,
    });
    inventory(vec![
        model("gpt-6@openrouter", "openai", "gpt-6"),
        model("gpt-6@openai-main", "openai", "gpt-6"),
        model("claude@openrouter", "anthropic", "claude"),
        local,
        sn,
    ])
}

fn policy(subject: &str, body: Value) -> AiccPolicy {
    let subject = PolicySubject::parse(subject).unwrap();
    AiccPolicy {
        policy_id: format!("pol-{subject}"),
        source: PolicySource::Admin,
        subject,
        enabled: true,
        body: serde_json::from_value(body).unwrap(),
        version: 1,
        created_by: "admin".into(),
        created_at_ms: 0,
        updated_by: "admin".into(),
        updated_at_ms: 0,
    }
}

fn user_policy(subject: &str, body: Value) -> AiccPolicy {
    AiccPolicy {
        source: PolicySource::User,
        policy_id: format!("pol-user-{subject}"),
        ..policy(subject, body)
    }
}

fn defaults() -> Vec<AiccPolicy> {
    vec![
        policy("type:admin", json!({"all_providers": true, "price_coefficient": 0})),
        policy("type:user", json!({"all_providers": true, "price_coefficient": 0})),
    ]
}

fn principal(user_id: &str, user_type: UserType, groups: &[&str]) -> PolicyPrincipal {
    PolicyPrincipal {
        user_id: user_id.into(),
        user_type,
        groups: groups.iter().map(|group| group.to_string()).collect(),
    }
}

fn price(key: &str, pricing: Value) -> StoredPrice {
    StoredPrice {
        entry_key: key.into(),
        pricing: pricing_from_api(&pricing).unwrap(),
        note: None,
        updated_by: "admin".into(),
        updated_at_ms: 0,
    }
}

fn prices(entries: Vec<StoredPrice>) -> PriceList {
    PriceList {
        revision: 1,
        entries: entries.into_iter().map(|entry| (entry.entry_key.clone(), entry)).collect(),
        credit_rates: BTreeMap::new(),
    }
}

fn tokens(input: u64, output: u64) -> AiUsage {
    AiUsage {
        input_tokens: Some(input),
        output_tokens: Some(output),
        total_tokens: Some(input + output),
        ..AiUsage::default()
    }
}

fn pinned(entry: &StoredPrice, provider_coefficient: f64, model_coefficient: f64) -> PinnedCreditPrice {
    PinnedCreditPrice {
        kind: PinnedCreditKind::Credit {
            entry_key: entry.entry_key.clone(),
            snapshot: crate::execution::PinnedPricingSnapshot::from_pricing(
                &entry.pricing,
                None,
                std::time::SystemTime::now(),
            )
            .unwrap()
            .unwrap(),
        },
        provider_coefficient,
        model_coefficient,
    }
}

#[test]
fn c01_default_policies_open_everything_for_free_and_nothing_for_limited() {
    let policies = defaults();
    let inventory = zone();
    let list = PriceList::default();
    for (user_type, user) in [(UserType::Admin, "alice"), (UserType::User, "bob")] {
        let access = Evaluator::new(&policies, &principal(user, user_type, &[]), None)
            .access(&inventory, &list);
        assert_eq!(access.visible.values().map(BTreeSet::len).sum::<usize>(), 5);
        assert!(access.unpriced.is_empty(), "free with an empty price list");
        let decision = Evaluator::new(&policies, &principal(user, user_type, &[]), None)
            .model("openrouter", "gpt-6@openrouter");
        assert_eq!(decision.coefficient(), 0.0);
        assert!(decision.limits.is_empty());
    }
    let limited = Evaluator::new(&policies, &principal("lin", UserType::Limited, &[]), None)
        .access(&inventory, &list);
    assert!(limited.visible.is_empty());
    let subjects = principal("lin", UserType::Limited, &["customer"]).subjects();
    assert_eq!(subjects, ["type:limited", "group:customer"]);
}

#[test]
fn c02_a_group_hides_a_provider_its_members_see_through_their_type() {
    let mut policies = defaults();
    policies.push(policy(
        "group:students",
        json!({"providers": {"openrouter": {"visible": "hidden"}}}),
    ));
    let student = Evaluator::new(&policies, &principal("sam", UserType::User, &["students"]), None);
    let decision = student.model("openrouter", "gpt-6@openrouter");
    assert!(!decision.visible);
    assert_eq!(decision.visible_from, ["group:students"]);
    assert!(student.model("openai-main", "gpt-6@openai-main").visible);
    assert!(!student.provider_visible("openrouter"));
    let other = Evaluator::new(&policies, &principal("bob", UserType::User, &[]), None);
    let decision = other.model("openrouter", "gpt-6@openrouter");
    assert!(decision.visible);
    assert_eq!(decision.visible_from, ["type:user"]);
}

#[test]
fn c03_groups_take_the_smallest_coefficient_and_the_user_layer_overrides() {
    let mut policies = defaults();
    policies.push(policy("group:a", json!({"providers": {"openrouter": {"price_coefficient": 1.2}}})));
    policies.push(policy("group:b", json!({"providers": {"openrouter": {"price_coefficient": 0.8}}})));
    let user = principal("bob", UserType::User, &["a", "b"]);
    let decision = Evaluator::new(&policies, &user, None).model("openrouter", "gpt-6@openrouter");
    assert_eq!(decision.provider_coefficient, 0.8);
    assert_eq!(decision.coefficient_from, ["group:b"]);
    // The type layer's top level coefficient applies to Providers the groups
    // say nothing about.
    let other = Evaluator::new(&policies, &user, None).model("openai-main", "gpt-6@openai-main");
    assert_eq!(other.provider_coefficient, 0.0);
    policies.push(policy("user:bob", json!({"price_coefficient": 1.0})));
    let decision = Evaluator::new(&policies, &user, None).model("openrouter", "gpt-6@openrouter");
    assert_eq!(decision.provider_coefficient, 1.0);
}

#[test]
fn c04_an_admin_hides_one_model_from_one_user() {
    let mut policies = defaults();
    policies.push(policy("user:bob", json!({"models": {"gpt-6@openrouter": {"visible": false}}})));
    let bob = Evaluator::new(&policies, &principal("bob", UserType::User, &[]), None);
    assert!(!bob.model("openrouter", "gpt-6@openrouter").visible);
    assert!(bob.model("openrouter", "claude@openrouter").visible);
    let access = Access::Restricted(bob.access(&zone(), &PriceList::default()));
    assert!(!access.allows_model("gpt-6@openrouter"));
    assert!(access.allows_provider("openrouter"));
    let carol = Evaluator::new(&policies, &principal("carol", UserType::User, &[]), None);
    assert!(carol.model("openrouter", "gpt-6@openrouter").visible);
}

#[test]
fn c05_an_app_policy_only_narrows_calls_through_the_app() {
    let mut policies = defaults();
    policies.push(policy(
        "app:writer",
        json!({"providers": {"openrouter": {"visible": "hidden"}, "local-ollama": {"visible": "hidden"}, "sn": {"visible": "hidden"}}}),
    ));
    let bob = principal("bob", UserType::User, &[]);
    let through_writer = Evaluator::new(&policies, &bob, Some("writer"));
    assert!(through_writer.model("openai-main", "gpt-6@openai-main").visible);
    assert!(!through_writer.model("openrouter", "gpt-6@openrouter").visible);
    let direct = Evaluator::new(&policies, &bob, None);
    assert!(direct.model("openrouter", "gpt-6@openrouter").visible);
    // App side cannot open what the user side hides.
    let lin = principal("lin", UserType::Limited, &[]);
    policies.push(policy("app:open", json!({"all_providers": true})));
    assert!(!Evaluator::new(&policies, &lin, Some("open"))
        .model("openai-main", "gpt-6@openai-main")
        .visible);
}

#[test]
fn c06_a_users_policy_on_their_agent_only_limits_and_hides() {
    let mut policies = defaults();
    policies.push(user_policy(
        "user_app:bob/agent-1",
        json!({"limits": [{"period": "5h", "unit": "credit", "amount": "10"}]}),
    ));
    let bob = principal("bob", UserType::User, &[]);
    let agent = Evaluator::new(&policies, &bob, Some("agent-1")).model("openrouter", "gpt-6@openrouter");
    assert_eq!(agent.limits.len(), 1);
    assert_eq!(
        agent.limits[0].counter,
        LimitCounter::UserApp {
            user_id: "bob".into(),
            app_id: "agent-1".into()
        }
    );
    assert_eq!(agent.limits[0].amount, 10_000_000);
    assert!(Evaluator::new(&policies, &bob, Some("other-app"))
        .model("openrouter", "gpt-6@openrouter")
        .limits
        .is_empty());
    let subject = PolicySubject::parse("user_app:bob/agent-1").unwrap();
    let body = |value: Value| serde_json::from_value::<PolicyBody>(value).unwrap();
    assert!(validate_body(PolicySource::User, &subject, &body(json!({"price_coefficient": 0}))).is_err());
    assert!(validate_body(PolicySource::User, &subject, &body(json!({"all_providers": true}))).is_err());
    assert!(validate_body(
        PolicySource::User,
        &subject,
        &body(json!({"providers": {"openrouter": {"visible": "all_models"}}}))
    )
    .is_err());
    assert!(validate_body(
        PolicySource::User,
        &subject,
        &body(json!({"providers": {"openrouter": {"visible": "selected_models"}},
            "models": {"gpt-6@openrouter": {"visible": true}}}))
    )
    .is_ok());
    assert!(validate_body(
        PolicySource::User,
        &PolicySubject::parse("user:bob").unwrap(),
        &body(json!({}))
    )
    .is_err());
    assert!(validate_body(
        PolicySource::Admin,
        &PolicySubject::parse("app:writer").unwrap(),
        &body(json!({"providers": {"openrouter": {"price_coefficient": 2}}}))
    )
    .is_err());
}

#[test]
fn c07_user_side_limits_override_by_period_and_unit_and_all_keys_apply() {
    let mut policies = defaults();
    policies.push(policy("type:user", json!({"all_providers": true, "price_coefficient": 0,
        "limits": [{"period": "24h", "unit": "credit", "amount": "100"}]})));
    policies.remove(1);
    policies.push(policy("user:bob", json!({"limits": [
        {"period": "24h", "unit": "credit", "amount": "500"},
        {"period": "7d", "unit": "credit", "amount": "1000"}]})));
    let limits = Evaluator::new(&policies, &principal("bob", UserType::User, &[]), None)
        .model("openrouter", "gpt-6@openrouter")
        .limits;
    let mut amounts = limits
        .iter()
        .map(|limit| (limit.period, limit.amount_text.as_str()))
        .collect::<Vec<_>>();
    amounts.sort();
    assert_eq!(amounts, [(LimitPeriod::Day, "500"), (LimitPeriod::Week, "1000")]);
    assert!(limits.iter().all(|limit| limit.counter
        == LimitCounter::User {
            user_id: "bob".into()
        }));
}

#[test]
fn c08_app_limits_are_shared_by_every_user_and_group_limits_are_per_member() {
    let mut policies = defaults();
    policies.push(policy("app:writer", json!({"limits": [{"period": "24h", "unit": "credit", "amount": "1000"}]})));
    policies.push(policy("group:a", json!({"limits": [{"period": "24h", "unit": "credit", "amount": "5"}]})));
    policies.push(policy("group:b", json!({"limits": [{"period": "24h", "unit": "credit", "amount": "9"}]})));
    let alice = Evaluator::new(&policies, &principal("alice", UserType::User, &["a", "b"]), Some("writer"))
        .model("openrouter", "gpt-6@openrouter")
        .limits;
    let bob = Evaluator::new(&policies, &principal("bob", UserType::User, &["a"]), Some("writer"))
        .model("openrouter", "gpt-6@openrouter")
        .limits;
    let app_scope = |limits: &[AppliedLimit]| {
        limits
            .iter()
            .find(|limit| matches!(limit.counter, LimitCounter::App { .. }))
            .unwrap()
            .scope()
    };
    assert_eq!(app_scope(&alice).key(), app_scope(&bob).key());
    let group_limit = |limits: &[AppliedLimit]| {
        limits
            .iter()
            .find(|limit| matches!(limit.counter, LimitCounter::User { .. }))
            .cloned()
            .unwrap()
    };
    assert_eq!(group_limit(&alice).amount_text, "9", "the larger group limit");
    assert_ne!(group_limit(&alice).scope().key(), group_limit(&bob).scope().key());
}

#[test]
fn c09_c10_charges_round_to_micro_credit_and_free_is_explicit() {
    let entry = price(
        "model:openai:gpt-6",
        json!({"input_token": 1.0, "output_token": 4.0}),
    );
    // 1 / 4 Credit per million tokens, coefficient 1.5.
    let pinned = pinned(&entry, 1.5, 1.0);
    assert!(pinned.needs_payment());
    assert_eq!(
        pinned.charge(&tokens(1_000_000, 500_000), None),
        (4_500_000, CreditChargeStatus::Charged)
    );
    assert_eq!(pinned.charge(&tokens(1, 0), None), (2, CreditChargeStatus::Charged));
    let unknown = AiUsage::default();
    assert_eq!(pinned.charge(&unknown, None), (0, CreditChargeStatus::Unpriced));
    let free = PinnedCreditPrice {
        provider_coefficient: 0.0,
        ..pinned.clone()
    };
    assert!(!free.needs_payment());
    assert_eq!(free.charge(&tokens(1_000_000, 0), None), (0, CreditChargeStatus::Free));
    let zero = price("model:openai:gpt-6", json!({"input_token": 0, "output_token": 0}));
    assert!(pricing::pricing_is_zero(&zero.pricing));
    let list = prices(vec![zero]);
    assert_eq!(
        list.state(&model("gpt-6@openrouter", "openai", "gpt-6"), 1.0),
        CreditPriceState::Free
    );
    assert_eq!(
        list.state(&model("claude@openrouter", "anthropic", "claude"), 1.0),
        CreditPriceState::Unpriced
    );
    assert_eq!(
        list.state(&model("claude@openrouter", "anthropic", "claude"), 0.0),
        CreditPriceState::Free
    );
}

#[test]
fn c11_a_model_identity_price_covers_every_provider_and_exact_overrides_one() {
    let list = prices(vec![
        price("model:openai:gpt-6", json!({"input_token": 2, "output_token": 8})),
        price("exact:gpt-6@openrouter", json!({"input_token": 1, "output_token": 4})),
    ]);
    let main = model("gpt-6@openai-main", "openai", "gpt-6");
    let router = model("gpt-6@openrouter", "openai", "gpt-6");
    let (source, entry) = list.entry_for(&main).unwrap();
    assert_eq!((source, entry.entry_key.as_str()), (CreditPriceSource::Model, "model:openai:gpt-6"));
    let (source, entry) = list.entry_for(&router).unwrap();
    assert_eq!((source, entry.entry_key.as_str()), (CreditPriceSource::Exact, "exact:gpt-6@openrouter"));
    // An unmapped model only has exact prices.
    let unmapped = model("custom@local-ollama", "", "");
    assert!(list.entry_for(&unmapped).is_none());
    // An exact key without a variant prices every variant.
    let variant = model("gpt-6:high@openrouter", "openai", "gpt-6");
    assert_eq!(list.entry_for(&variant).unwrap().1.entry_key, "exact:gpt-6@openrouter");
    // Per million in the API, per token stored, the same on the way out.
    let stored = list.entries.get("model:openai:gpt-6").unwrap();
    assert_eq!(stored.pricing.input_token, Some(2e-6));
    assert_eq!(stored.to_entry().pricing["input_token"], json!(2.0));
    assert_eq!(
        parse_entry_key("exact:gpt-6:high@openrouter").unwrap(),
        EntryKey::Exact {
            provider_model_id: "gpt-6".into(),
            variant: Some("high".into()),
            provider_instance_name: "openrouter".into()
        }
    );
    assert!(parse_entry_key("model:openai").is_err());
    assert!(parse_entry_key("gpt-6").is_err());
    assert!(pricing_from_api(&json!({"currency": "USD", "input_token": 1})).is_err());
    assert!(pricing_from_api(&json!({})).is_err());
}

#[test]
fn c12_unit_prices_charge_images_and_seconds() {
    let image = price("model:openai:image", json!({"unit": "image", "amount": 0.5}));
    let pinned = PinnedCreditPrice {
        kind: PinnedCreditKind::Credit {
            entry_key: image.entry_key.clone(),
            snapshot: crate::execution::PinnedPricingSnapshot::from_pricing(
                &image.pricing,
                None,
                std::time::SystemTime::now(),
            )
            .unwrap()
            .unwrap(),
        },
        provider_coefficient: 1.0,
        model_coefficient: 2.0,
    };
    let usage = AiUsage {
        image_units: Some(3),
        ..AiUsage::default()
    };
    assert_eq!(pinned.charge(&usage, None), (3_000_000, CreditChargeStatus::Charged));
    let video = price("model:kling:video", json!({"unit": "video_second", "amount": 0.25}));
    let snapshot = crate::execution::PinnedPricingSnapshot::from_pricing(
        &video.pricing,
        None,
        std::time::SystemTime::now(),
    )
    .unwrap()
    .unwrap();
    let pinned = PinnedCreditPrice {
        kind: PinnedCreditKind::Credit {
            entry_key: video.entry_key.clone(),
            snapshot,
        },
        provider_coefficient: 1.0,
        model_coefficient: 1.0,
    };
    let usage = AiUsage {
        video_seconds: Some(8.0),
        ..AiUsage::default()
    };
    assert_eq!(pinned.charge(&usage, None), (2_000_000, CreditChargeStatus::Charged));
    assert!(!is_token_metered(ApiType::ImageTextToImage));
    assert!(is_token_metered(ApiType::Llm));
    // Tiers and rules keep working on Credit prices.
    let tiered = pricing_from_api(&json!({"tiers": {"dimension": "input_tokens", "steps": [
        {"up_to": 1000, "input_token": 1}, {"input_token": 2}]}}))
    .unwrap();
    assert!(tiered.tiers.is_some());
    let ruled = pricing_from_api(&json!({"unit": "image", "amount": 1,
        "rules": [{"when": {"/quality": ["hd"]}, "amount": 3}]}))
    .unwrap();
    let mut context = crate::matching::MatchContext::new();
    context.insert("/quality".into(), json!("hd"));
    assert_eq!(crate::call::matched_rule_amount(&ruled, &context), Some(3.0));
}

#[test]
fn c25_charging_users_makes_unpriced_models_unusable_and_free_providers_stay_free() {
    let mut policies = vec![
        policy("type:admin", json!({"all_providers": true, "price_coefficient": 0})),
        policy("type:user", json!({"all_providers": true, "price_coefficient": 1,
            "providers": {"local-ollama": {"price_coefficient": 0}}})),
    ];
    let mut list = prices(vec![price("model:openai:gpt-6", json!({"input_token": 1, "output_token": 2}))]);
    list.credit_rates.insert("USD".into(), 100.0);
    let inventory = zone();
    let bob = Evaluator::new(&policies, &principal("bob", UserType::User, &[]), None);
    let access = Access::Restricted(bob.access(&inventory, &list));
    assert!(access.usable_model("gpt-6@openrouter"));
    assert!(access.usable_model("gpt-6@openai-main"));
    assert!(access.allows_model("claude@openrouter"));
    assert!(!access.usable_model("claude@openrouter"));
    assert!(access.usable_model("llama@local-ollama"), "a free Provider");
    assert!(access.usable_model("gpt-x@sn"), "Provider cost at the USD rate");
    let decision = bob.model("sn", "gpt-x@sn");
    let described = list.describe(inventory.model("gpt-x@sn").unwrap(), decision.provider_coefficient, 1.0);
    assert_eq!(described.source, Some(CreditPriceSource::ProviderCost));
    assert_eq!(described.credit_rate, Some(100.0));
    let rate = PinnedCreditPrice {
        kind: PinnedCreditKind::ProviderCost {
            currency: "USD".into(),
            rate: 100.0,
        },
        provider_coefficient: 1.0,
        model_coefficient: 1.0,
    };
    assert!(rate.needs_payment());
    let cost = AiCost {
        amount: 0.0123,
        currency: "usd".into(),
    };
    assert_eq!(rate.charge(&tokens(10, 10), Some(&cost)), (1_230_000, CreditChargeStatus::Charged));
    assert_eq!(rate.charge(&tokens(10, 10), None), (0, CreditChargeStatus::Unpriced));
    let admin = Evaluator::new(&policies, &principal("alice", UserType::Admin, &[]), None);
    assert!(admin.access(&inventory, &list).unpriced.is_empty());
    // A Provider with a cost currency but no rate has no price.
    list.credit_rates.clear();
    let access = Access::Restricted(bob.access(&inventory, &list));
    assert!(!access.usable_model("gpt-x@sn"));
    policies.clear();
    assert!(Evaluator::new(&policies, &principal("bob", UserType::User, &[]), None)
        .access(&inventory, &list)
        .visible
        .is_empty());
}

#[test]
fn selected_models_and_provider_level_visibility() {
    let policies = vec![policy(
        "type:limited",
        json!({"providers": {"openrouter": {"visible": "selected_models"}},
            "models": {"claude@openrouter": {"visible": true}}}),
    )];
    let lin = Evaluator::new(&policies, &principal("lin", UserType::Limited, &[]), None);
    assert!(lin.model("openrouter", "claude@openrouter").visible);
    assert!(!lin.model("openrouter", "gpt-6@openrouter").visible);
    assert!(!lin.model("openai-main", "gpt-6@openai-main").visible);
    assert!(lin.provider_visible("openrouter"));
    let body: PolicyBody = serde_json::from_value(json!({
        "providers": {"openrouter": {"visible": "hidden"}},
        "models": {"claude@openrouter": {"visible": true}}}))
    .unwrap();
    assert!(validate_body(PolicySource::Admin, &PolicySubject::parse("type:user").unwrap(), &body).is_err());
    let _ = (ModelPolicy::default(), ProviderPolicy::default());
}

#[test]
fn counter_keys_separate_counter_resource_and_period() {
    let scope = |counter: LimitCounter, period: LimitPeriod| CounterScope {
        counter,
        resource: LimitResource::All,
        period,
    };
    let user = LimitCounter::User {
        user_id: "bob".into(),
    };
    let app = LimitCounter::App {
        app_id: "bob".into(),
    };
    assert_ne!(scope(user.clone(), LimitPeriod::Day).key(), scope(app, LimitPeriod::Day).key());
    assert_ne!(
        scope(user.clone(), LimitPeriod::Day).key(),
        scope(user, LimitPeriod::Week).key()
    );
}
