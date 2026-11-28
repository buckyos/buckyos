use super::*;
use crate::policy::pricing::PinnedCreditKind;
use crate::policy::PinnedCreditPrice;
use buckyos_api::{LimitCounter, LimitPeriod, LimitResource};

const HOUR: i64 = 60 * 60 * 1000;

async fn db() -> AiccStorage {
    AiccStorage::open("sqlite::memory:", RdbBackend::Sqlite)
        .await
        .unwrap()
}

fn recharge<'a>(user_id: &'a str, request_id: &'a str, amount_micro: i64) -> CreditEntryDraft<'a> {
    CreditEntryDraft {
        user_id,
        request_id,
        kind: CreditEntryKind::Recharge,
        amount_micro,
        note: Some("top up"),
        external_ref: None,
        operator: "admin",
        now_ms: 10,
    }
}

fn scope(period: LimitPeriod) -> CounterScope {
    CounterScope {
        counter: LimitCounter::User {
            user_id: "bob".into(),
        },
        resource: LimitResource::All,
        period,
    }
}

fn plan(attempt_id: &str, admitted_at_ms: i64) -> ChargePlan {
    ChargePlan {
        attempt_id: attempt_id.into(),
        task_id: format!("task-{attempt_id}"),
        usage_event_id: format!("usage-{attempt_id}"),
        user_id: "bob".into(),
        app_id: Some("writer".into()),
        operator_id: None,
        provider_instance_name: "p".into(),
        exact_model: "m@p".into(),
        admitted_at_ms,
        subjects: vec!["type:user".into()],
        price: PinnedCreditPrice {
            kind: PinnedCreditKind::ProviderCost {
                currency: "USD".into(),
                rate: 100.0,
            },
            provider_coefficient: 1.0,
            model_coefficient: 1.0,
        },
        counters: vec![scope(LimitPeriod::FiveHours)],
        token_metered: true,
    }
}

fn charged(credit_micro: i64, tokens: u64) -> ChargeOutcome {
    ChargeOutcome {
        credit_micro,
        tokens,
        status: CreditChargeStatus::Charged,
    }
}

async fn counter(db: &AiccStorage, scope: &CounterScope) -> Option<CounterState> {
    db.counters(&BTreeSet::from([scope.key()]))
        .await
        .unwrap()
        .remove(&scope.key())
}

#[tokio::test]
async fn a2_c17_recharges_apply_once_per_id_and_conflicting_reuse_fails() {
    let db = db().await;
    let (entry, balance, replayed) = db.apply_credit_entry(recharge("bob", "order-1", 5_000_000)).await.unwrap();
    assert_eq!((balance, replayed), (5_000_000, false));
    assert_eq!(entry.balance_after, "5");
    let (again, balance, replayed) = db.apply_credit_entry(recharge("bob", "order-1", 5_000_000)).await.unwrap();
    assert_eq!((balance, replayed), (5_000_000, true));
    assert_eq!(again.entry_id, entry.entry_id);
    assert!(matches!(
        db.apply_credit_entry(recharge("bob", "order-1", 6_000_000)).await,
        Err(PolicyWriteError::IdempotencyConflict)
    ));
    assert!(matches!(
        db.apply_credit_entry(recharge("alice", "order-1", 5_000_000)).await,
        Err(PolicyWriteError::IdempotencyConflict)
    ));
    let (first, second) = tokio::join!(
        db.apply_credit_entry(recharge("bob", "order-2", 1_000_000)),
        db.apply_credit_entry(recharge("bob", "order-3", 2_000_000)),
    );
    first.unwrap();
    second.unwrap();
    assert_eq!(db.credit_balance("bob").await.unwrap().0, 8_000_000);
    assert_eq!(db.credit_entry("order-1").await.unwrap().unwrap().user_id, "bob");
    assert!(db.credit_entry("missing").await.unwrap().is_none());
    assert_eq!(db.credit_balance("nobody").await.unwrap(), (0, 0));
    let mut adjust = recharge("bob", "refund-1", -9_000_000);
    adjust.kind = CreditEntryKind::Adjust;
    let (_, balance, _) = db.apply_credit_entry(adjust).await.unwrap();
    assert_eq!(balance, -1_000_000, "a balance may go negative");
}

#[tokio::test]
async fn c09_c14_charges_settle_once_and_may_overdraw() {
    let db = db().await;
    db.apply_credit_entry(recharge("bob", "order-1", 500_000)).await.unwrap();
    let first = db
        .settle_charge(&plan("a1", 10 * HOUR), charged(3_000_000, 100), Some("usage-a1"), 10 * HOUR)
        .await
        .unwrap();
    assert_eq!(first, charged(3_000_000, 100));
    assert_eq!(db.credit_balance("bob").await.unwrap().0, -2_500_000);
    // A replay (a restart settling the same attempt again) changes nothing.
    let replay = db
        .settle_charge(&plan("a1", 10 * HOUR), charged(9_000_000, 900), Some("usage-a1"), 11 * HOUR)
        .await
        .unwrap();
    assert_eq!(replay, first);
    assert_eq!(db.credit_balance("bob").await.unwrap().0, -2_500_000);
    let state = counter(&db, &scope(LimitPeriod::FiveHours)).await.unwrap();
    assert_eq!((state.used_credit_micro, state.used_tokens), (3_000_000, 100));
    let charge = db.credit_charge("a1").await.unwrap().unwrap();
    assert_eq!(charge.amount, "3");
    let (items, next) = db.credit_ledger("bob", 0, i64::MAX, None, 1).await.unwrap();
    assert_eq!(items.len(), 1);
    assert!(matches!(&items[0], CreditLedgerItem::Charge(charge) if charge.attempt_id == "a1"));
    let (rest, done) = db.credit_ledger("bob", 0, i64::MAX, next.as_deref(), 10).await.unwrap();
    assert!(matches!(&rest[0], CreditLedgerItem::Entry(entry) if entry.recharge_id.as_deref() == Some("order-1")));
    assert!(done.is_none());
}

#[tokio::test]
async fn c15_an_unknown_usage_is_charged_once_when_it_arrives() {
    let db = db().await;
    let unknown = ChargeOutcome {
        credit_micro: 0,
        tokens: 0,
        status: CreditChargeStatus::UnknownUsage,
    };
    db.settle_charge(&plan("a1", HOUR), unknown, None, HOUR).await.unwrap();
    assert_eq!(db.credit_balance("bob").await.unwrap().0, 0);
    assert_eq!(db.settle_charge(&plan("a1", HOUR), unknown, None, HOUR).await.unwrap(), unknown);
    let late = db
        .settle_charge(&plan("a1", HOUR), charged(1_000_000, 50), Some("usage-a1"), 2 * HOUR)
        .await
        .unwrap();
    assert_eq!(late, charged(1_000_000, 50));
    assert_eq!(db.credit_balance("bob").await.unwrap().0, -1_000_000);
    let again = db
        .settle_charge(&plan("a1", HOUR), charged(7_000_000, 70), Some("usage-a1"), 3 * HOUR)
        .await
        .unwrap();
    assert_eq!(again, late);
    assert_eq!(db.credit_balance("bob").await.unwrap().0, -1_000_000);
    let state = counter(&db, &scope(LimitPeriod::FiveHours)).await.unwrap();
    assert_eq!(state.used_credit_micro, 1_000_000);
    assert_eq!(state.window_start_ms, HOUR, "the window is of the admission time");
}

#[tokio::test]
async fn c16_windows_start_at_first_use_and_late_settlements_keep_their_window() {
    let db = db().await;
    let start = 10 * HOUR + 12 * 60 * 1000;
    db.settle_charge(&plan("a1", start), charged(1_000_000, 10), None, start).await.unwrap();
    let state = counter(&db, &scope(LimitPeriod::FiveHours)).await.unwrap();
    assert_eq!((state.window_start_ms, state.window_end_ms), (start, start + 5 * HOUR));
    // Admitted inside the window, settled after it ended: still counted there.
    let inside = start + 5 * HOUR - 12 * 60 * 1000;
    db.settle_charge(&plan("a2", inside), charged(2_000_000, 20), None, start + 6 * HOUR)
        .await
        .unwrap();
    let state = counter(&db, &scope(LimitPeriod::FiveHours)).await.unwrap();
    assert_eq!(state.used_credit_micro, 3_000_000);
    assert!(!state.active_at(start + 5 * HOUR));
    // The first use after the end opens a new window.
    let next = start + 5 * HOUR + 8 * 60 * 1000;
    db.settle_charge(&plan("a3", next), charged(500_000, 5), None, next).await.unwrap();
    let state = counter(&db, &scope(LimitPeriod::FiveHours)).await.unwrap();
    assert_eq!((state.window_start_ms, state.used_credit_micro), (next, 500_000));
    // Admitted in the previous window but settled after a new one opened:
    // counted in no window, the balance is still charged.
    db.settle_charge(&plan("a4", inside), charged(4_000_000, 40), None, next + HOUR)
        .await
        .unwrap();
    let state = counter(&db, &scope(LimitPeriod::FiveHours)).await.unwrap();
    assert_eq!(state.used_credit_micro, 500_000);
    assert_eq!(db.credit_balance("bob").await.unwrap().0, -7_500_000);
}

#[tokio::test]
async fn free_and_non_token_charges_do_not_touch_counters_needlessly() {
    let db = db().await;
    let free = ChargeOutcome {
        credit_micro: 0,
        tokens: 0,
        status: CreditChargeStatus::Free,
    };
    db.settle_charge(&plan("a1", HOUR), free, None, HOUR).await.unwrap();
    assert!(counter(&db, &scope(LimitPeriod::FiveHours)).await.is_none());
    let mut image = plan("a2", HOUR);
    image.token_metered = false;
    db.settle_charge(&image, charged(1_000_000, 4_096), None, HOUR).await.unwrap();
    let state = counter(&db, &scope(LimitPeriod::FiveHours)).await.unwrap();
    assert_eq!((state.used_credit_micro, state.used_tokens), (1_000_000, 0));
}

#[tokio::test]
async fn policies_upsert_with_versions_and_prices_with_a_revision() {
    let db = db().await;
    let subject = PolicySubject::parse("group:students").unwrap();
    let body = PolicyBody {
        price_coefficient: Some(1.0),
        ..PolicyBody::default()
    };
    let created = db
        .upsert_policy(PolicySource::Admin, &subject, true, &body, None, "admin", 5)
        .await
        .unwrap();
    assert_eq!(created.version, 1);
    assert!(matches!(
        db.upsert_policy(PolicySource::Admin, &subject, true, &body, None, "admin", 6).await,
        Err(PolicyWriteError::VersionConflict { expected: 0, actual: 1 })
    ));
    let updated = db
        .upsert_policy(PolicySource::Admin, &subject, false, &body, Some(1), "admin", 7)
        .await
        .unwrap();
    assert_eq!((updated.version, updated.enabled), (2, false));
    assert_eq!(updated.policy_id, created.policy_id);
    let user = db
        .upsert_policy(PolicySource::User, &subject, true, &body, None, "bob", 8)
        .await
        .unwrap();
    assert_ne!(user.policy_id, created.policy_id, "one policy per source and subject");
    assert_eq!(db.policy_audit(&created.policy_id).await.unwrap().len(), 2);
    assert!(matches!(
        db.delete_policy(&created.policy_id, 1, "admin", 9).await,
        Err(PolicyWriteError::VersionConflict { .. })
    ));
    db.delete_policy(&created.policy_id, 2, "admin", 9).await.unwrap();
    assert!(db.policy(&created.policy_id).await.unwrap().is_none());

    let list = db.price_list().await.unwrap();
    let entry = StoredPrice {
        entry_key: "model:openai:gpt-6".into(),
        pricing: crate::policy::pricing::pricing_from_api(&serde_json::json!({"input_token": 1})).unwrap(),
        note: None,
        updated_by: String::new(),
        updated_at_ms: 0,
    };
    let rates = BTreeMap::from([("USD".to_string(), 100.0)]);
    let revision = db
        .update_prices(list.revision, &[entry], &[], Some(&rates), "admin", 10)
        .await
        .unwrap();
    assert!(matches!(
        db.update_prices(list.revision, &[], &[], None, "admin", 11).await,
        Err(PolicyWriteError::VersionConflict { .. })
    ));
    let list = db.price_list().await.unwrap();
    assert_eq!(list.revision, revision);
    assert_eq!(list.credit_rates, rates);
    assert_eq!(list.entries["model:openai:gpt-6"].updated_by, "admin");
    db.update_prices(revision, &[], &["model:openai:gpt-6".into()], None, "admin", 12)
        .await
        .unwrap();
    let list = db.price_list().await.unwrap();
    assert!(list.entries.is_empty());
    assert_eq!(list.credit_rates, rates, "rates stay unless replaced");
}
