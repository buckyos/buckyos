use super::*;
use crate::service::identity::AccountFacts;
use buckyos_api::{
    AiccAttributionMode, AiUsage, ApiType, CreditAccount, CreditPriceState, PolicyListResponse, RdbBackend,
    UserType,
};

struct Directory {
    accounts: Vec<AccountFacts>,
}

#[async_trait]
impl UserDirectory for Directory {
    async fn account(&self, user_id: &str) -> Result<Option<AccountFacts>, RPCErrors> {
        Ok(self.accounts.iter().find(|account| account.user_id == user_id).cloned())
    }
    async fn agent_owner(&self, _agent_did: &str) -> Result<Option<(String, String)>, RPCErrors> {
        Ok(None)
    }
    async fn accounts(&self) -> Result<Vec<AccountFacts>, RPCErrors> {
        Ok(self.accounts.clone())
    }
    async fn defined_groups(&self) -> Result<BTreeSet<String>, RPCErrors> {
        Ok(BTreeSet::from(["students".to_string()]))
    }
    async fn user_apps(&self, user_id: &str) -> Result<BTreeSet<String>, RPCErrors> {
        Ok(if user_id == "bob" {
            BTreeSet::from(["writer".to_string()])
        } else {
            BTreeSet::new()
        })
    }
}

fn account(user_id: &str, user_type: UserType, groups: &[&str]) -> AccountFacts {
    AccountFacts {
        user_id: user_id.into(),
        user_type,
        active: true,
        groups: groups.iter().map(|group| group.to_string()).collect(),
    }
}

fn caller(facts: &AccountFacts, app_id: &str) -> AuthorizedCaller {
    AuthorizedCaller {
        tenant_id: facts.user_id.clone(),
        user_id: facts.user_id.clone(),
        app_id: Some(app_id.into()),
        token: "token".into(),
        principal: CallerPrincipal::Account {
            user_type: facts.user_type,
            groups: facts.groups.clone(),
            operator_id: Some(facts.user_id.clone()),
            caller: app_id.into(),
            mode: AiccAttributionMode::Direct,
            run_ref: None,
        },
    }
}

fn inventory() -> Inventory {
    let model = |exact: &str, driver: &str, origin: &str| {
        let (id, provider) = exact.rsplit_once('@').unwrap();
        InventoryModel {
            exact_model: exact.into(),
            provider_instance_name: provider.into(),
            provider_model_id: id.into(),
            variant: None,
            model_driver_id: driver.into(),
            origin_model_id: origin.into(),
            provider_cost: None,
        }
    };
    let mut inventory = Inventory::default();
    for model in [
        model("gpt-6@openrouter", "openai", "gpt-6"),
        model("claude@openrouter", "anthropic", "claude"),
        model("llama@local", "", ""),
    ] {
        inventory
            .providers
            .entry(model.provider_instance_name.clone())
            .or_default()
            .insert(model.exact_model.clone(), model);
    }
    inventory
}

struct Fixture {
    service: PolicyService,
    admin: AuthorizedCaller,
    bob: AuthorizedCaller,
    sam: AuthorizedCaller,
}

async fn fixture() -> Fixture {
    let storage = Arc::new(AiccStorage::open("sqlite::memory:", RdbBackend::Sqlite).await.unwrap());
    let admin = account("alice", UserType::Admin, &[]);
    let bob = account("bob", UserType::User, &[]);
    let sam = account("sam", UserType::User, &["students"]);
    let directory = Arc::new(Directory {
        accounts: vec![admin.clone(), bob.clone(), sam.clone()],
    });
    Fixture {
        service: PolicyService::new(storage, directory),
        admin: caller(&admin, "system:control-panel"),
        bob: caller(&bob, "system:control-panel"),
        sam: caller(&sam, "system:control-panel"),
    }
}

async fn call(service: &PolicyService, caller: &AuthorizedCaller, method: &str, params: Value) -> Result<Value, RPCErrors> {
    service.handle(caller, method, params, &inventory()).await
}

async fn ok<T: DeserializeOwned>(service: &PolicyService, caller: &AuthorizedCaller, method: &str, params: Value) -> T {
    serde_json::from_value(call(service, caller, method, params).await.unwrap()).unwrap()
}

fn code(error: RPCErrors) -> AiccErrorCode {
    AiccError::from_krpc_error(&error).expect("AICC error").code
}

fn error_of(result: Result<ChargePlan, AiccError>) -> AiccError {
    result.expect_err("refused")
}

fn llm_call(exact: &str, driver: &str, origin: &str) -> crate::call::ResolvedProviderCall {
    let (id, provider) = exact.rsplit_once('@').unwrap();
    let mut call = crate::execution::tests::call(provider);
    call.exact_model = exact.into();
    call.provider_model_id = id.into();
    call.model_driver_id = driver.into();
    call.origin_model_id = origin.into();
    call
}

async fn admit(fixture: &Fixture, who: &AuthorizedCaller, call: &crate::call::ResolvedProviderCall) -> Result<ChargePlan, AiccError> {
    fixture
        .service
        .evaluate_call(
            &principal_of(who).unwrap(),
            who.app_id.as_deref(),
            None,
            call,
            "attempt-1",
            "task-1",
            "usage-1",
        )
        .await
}

async fn start_charging(fixture: &Fixture) {
    let policies: PolicyListResponse =
        ok(&fixture.service, &fixture.admin, policy_methods::POLICY_LIST, json!({"scope": "manage"})).await;
    let user_type = policies
        .policies
        .iter()
        .find(|view| view.policy.subject.to_string() == "type:user")
        .unwrap();
    let _: AiccPolicyView = ok(&fixture.service, &fixture.admin, policy_methods::POLICY_UPSERT, json!({
        "draft": {"subject": "type:user", "body": {"all_providers": true, "price_coefficient": 1,
            "providers": {"local": {"price_coefficient": 0}}}},
        "expected_version": user_type.policy.version}))
    .await;
    let _: PriceGetResponse = ok(&fixture.service, &fixture.admin, policy_methods::PRICE_UPDATE, json!({
        "expected_revision": 0,
        "upserts": [{"entry_key": "model:openai:gpt-6", "pricing": {"input_token": 2, "output_token": 8}}]}))
    .await;
}

#[tokio::test]
async fn c01_out_of_the_box_everything_is_free_and_charged_nothing() {
    let fixture = fixture().await;
    let plan = admit(&fixture, &fixture.bob, &llm_call("claude@openrouter", "anthropic", "claude"))
        .await
        .unwrap();
    assert!(!plan.price.needs_payment());
    assert_eq!(plan.subjects, ["type:user"]);
    let charged = fixture
        .service
        .settle(&plan, AttemptSettlement::Usage(AiUsage::default(), ApiType::Llm, None))
        .await;
    assert_eq!(charged, Some((0, CreditChargeStatus::Free)));
    let access = fixture.service.access(&fixture.bob, &inventory()).await.unwrap();
    assert!(access.usable_model("llama@local"));
}

#[tokio::test]
async fn c09_c25_paying_users_need_a_balance_and_a_price() {
    let fixture = fixture().await;
    start_charging(&fixture).await;
    let priced = llm_call("gpt-6@openrouter", "openai", "gpt-6");
    let error = error_of(admit(&fixture, &fixture.bob, &priced).await);
    assert_eq!(error.code, AiccErrorCode::CreditInsufficient);
    assert_eq!(error.details.unwrap()["balance"], "0");
    let error = error_of(admit(&fixture, &fixture.bob, &llm_call("claude@openrouter", "anthropic", "claude")).await);
    assert_eq!(error.code, AiccErrorCode::ResourceNotPermitted);
    assert_eq!(error.details.unwrap()["reason"], "unpriced");
    // Free Provider and the still free admin need nothing.
    admit(&fixture, &fixture.bob, &llm_call("llama@local", "", "")).await.unwrap();
    admit(&fixture, &fixture.admin, &llm_call("claude@openrouter", "anthropic", "claude")).await.unwrap();
    let recharged: CreditEntryResponse = ok(&fixture.service, &fixture.admin, policy_methods::CREDIT_RECHARGE, json!({
        "user_id": "bob", "recharge_id": "order-1", "amount": "0.5",
        "external_ref": {"gateway": "test", "order": "1"}}))
    .await;
    assert_eq!(recharged.account.balance, "0.5");
    let plan = admit(&fixture, &fixture.bob, &priced).await.unwrap();
    let usage = AiUsage {
        input_tokens: Some(1_000_000),
        output_tokens: Some(125_000),
        total_tokens: Some(1_125_000),
        ..AiUsage::default()
    };
    let charged = fixture
        .service
        .settle(&plan, AttemptSettlement::Usage(usage, ApiType::Llm, None))
        .await;
    assert_eq!(charged, Some((3_000_000, CreditChargeStatus::Charged)));
    let account: CreditAccount = ok(&fixture.service, &fixture.bob, policy_methods::CREDIT_ACCOUNT_GET, json!({})).await;
    assert_eq!(account.balance, "-2.5");
    assert_eq!(
        error_of(admit(&fixture, &fixture.bob, &priced).await).code,
        AiccErrorCode::CreditInsufficient
    );
    let ledger: CreditLedgerListResponse =
        ok(&fixture.service, &fixture.bob, policy_methods::CREDIT_LEDGER_LIST, json!({})).await;
    assert_eq!(ledger.items.len(), 2);
    let effective: PolicyEffectiveResponse =
        ok(&fixture.service, &fixture.bob, policy_methods::POLICY_EFFECTIVE, json!({})).await;
    assert_eq!(effective.unpriced_models, ["claude@openrouter"]);
    let gpt = effective.models.iter().find(|model| model.exact_model == "gpt-6@openrouter").unwrap();
    assert_eq!(gpt.price.state, CreditPriceState::Priced);
    assert_eq!(gpt.price.pricing.as_ref().unwrap()["input_token"], json!(2.0));
}

#[tokio::test]
async fn c10_a_used_up_window_refuses_and_says_when_it_resets() {
    let fixture = fixture().await;
    start_charging(&fixture).await;
    let _: AiccPolicyView = ok(&fixture.service, &fixture.admin, policy_methods::POLICY_UPSERT, json!({
        "draft": {"subject": "user:bob", "body": {"limits": [{"period": "5h", "unit": "credit", "amount": "1"}]}}}))
    .await;
    let _: CreditEntryResponse = ok(&fixture.service, &fixture.admin, policy_methods::CREDIT_RECHARGE, json!({
        "user_id": "bob", "recharge_id": "order-1", "amount": "100"}))
    .await;
    let priced = llm_call("gpt-6@openrouter", "openai", "gpt-6");
    let plan = admit(&fixture, &fixture.bob, &priced).await.unwrap();
    assert_eq!(plan.counters.len(), 1);
    let usage = AiUsage {
        input_tokens: Some(1_000_000),
        output_tokens: Some(0),
        total_tokens: Some(1_000_000),
        ..AiUsage::default()
    };
    fixture
        .service
        .settle(&plan, AttemptSettlement::Usage(usage, ApiType::Llm, None))
        .await
        .unwrap();
    let error = error_of(admit(&fixture, &fixture.bob, &priced).await);
    assert_eq!(error.code, AiccErrorCode::QuotaExhausted);
    let details: QuotaDenialDetails = serde_json::from_value(error.details.unwrap()).unwrap();
    assert_eq!(details.blocking[0].used, "2");
    assert_eq!(details.reset_at_ms, Some(plan.admitted_at_ms + 5 * 60 * 60 * 1000));
    let status: PolicyStatusResponse =
        ok(&fixture.service, &fixture.bob, policy_methods::POLICY_STATUS, json!({})).await;
    assert_eq!(status.exhausted_count, 1);
    assert_eq!(status.balance, "98");
    // A zero limit forbids and never resets by itself.
    let _: AiccPolicyView = ok(&fixture.service, &fixture.admin, policy_methods::POLICY_UPSERT, json!({
        "draft": {"subject": "user:bob", "body": {"limits": [{"period": "24h", "unit": "token", "amount": "0"}]}},
        "expected_version": 1}))
    .await;
    let error = error_of(admit(&fixture, &fixture.bob, &priced).await);
    assert_eq!(error.code, AiccErrorCode::QuotaExhausted);
    assert_eq!(error.details.unwrap()["reset_at_ms"], Value::Null);
}

#[tokio::test]
async fn c06_users_only_narrow_their_own_apps() {
    let fixture = fixture().await;
    let own: AiccPolicyView = ok(&fixture.service, &fixture.bob, policy_methods::POLICY_UPSERT, json!({
        "draft": {"subject": "user_app:bob/writer", "body": {
            "limits": [{"period": "5h", "unit": "credit", "amount": "10"}]}}}))
    .await;
    assert_eq!(own.policy.source, PolicySource::User);
    assert!(own.editable);
    for draft in [
        json!({"subject": "user_app:alice/writer", "body": {}}),
        json!({"subject": "user_app:bob/not-mine", "body": {}}),
        json!({"subject": "user:bob", "body": {}}),
        json!({"subject": "type:user", "body": {}, "source": "admin"}),
    ] {
        assert_eq!(
            code(call(&fixture.service, &fixture.bob, policy_methods::POLICY_UPSERT, json!({"draft": draft}))
                .await
                .unwrap_err()),
            AiccErrorCode::PolicyDenied,
            "{draft}"
        );
    }
    assert_eq!(
        code(call(&fixture.service, &fixture.bob, policy_methods::POLICY_UPSERT, json!({"draft": {
            "subject": "user_app:bob/writer", "body": {"price_coefficient": 0}}, "expected_version": 1}))
        .await
        .unwrap_err()),
        AiccErrorCode::InvalidRequest
    );
    let mine: PolicyListResponse = ok(&fixture.service, &fixture.bob, policy_methods::POLICY_LIST, json!({})).await;
    assert!(mine.policies.iter().any(|view| view.policy.subject.to_string() == "type:user" && !view.editable));
    assert_eq!(
        code(call(&fixture.service, &fixture.bob, policy_methods::POLICY_LIST, json!({"scope": "manage"}))
            .await
            .unwrap_err()),
        AiccErrorCode::PolicyDenied
    );
    let admin_list: PolicyListResponse =
        ok(&fixture.service, &fixture.admin, policy_methods::POLICY_LIST, json!({"scope": "manage"})).await;
    assert!(admin_list
        .policies
        .iter()
        .any(|view| view.policy.source == PolicySource::User && !view.editable));
    // Through the Agent only.
    let mut through_writer = fixture.bob.clone();
    through_writer.app_id = Some("writer".into());
    let plan = admit(&fixture, &through_writer, &llm_call("gpt-6@openrouter", "openai", "gpt-6")).await.unwrap();
    assert_eq!(plan.counters.len(), 1);
    let plan = admit(&fixture, &fixture.bob, &llm_call("gpt-6@openrouter", "openai", "gpt-6")).await.unwrap();
    assert!(plan.counters.is_empty());
}

#[tokio::test]
async fn c02_effective_preview_names_sources_and_what_a_draft_changes() {
    let fixture = fixture().await;
    let preview: PolicyEffectiveResponse = ok(&fixture.service, &fixture.admin, policy_methods::POLICY_EFFECTIVE, json!({
        "user_id": "sam",
        "draft": {"subject": "group:students", "body": {"providers": {"openrouter": {"visible": "hidden"}}}}}))
    .await;
    assert_eq!(
        preview.models.iter().map(|model| model.exact_model.as_str()).collect::<Vec<_>>(),
        ["llama@local"]
    );
    assert_eq!(preview.removed, ["claude@openrouter", "gpt-6@openrouter"]);
    assert_eq!(preview.affected_users, ["sam"]);
    assert!(preview.caller_loses.is_empty());
    let self_loss: PolicyEffectiveResponse = ok(&fixture.service, &fixture.admin, policy_methods::POLICY_EFFECTIVE, json!({
        "draft": {"subject": "type:admin", "body": {}}}))
    .await;
    assert_eq!(self_loss.caller_loses.len(), 3);
    assert_eq!(
        code(call(&fixture.service, &fixture.bob, policy_methods::POLICY_EFFECTIVE, json!({"user_id": "sam"}))
            .await
            .unwrap_err()),
        AiccErrorCode::PolicyDenied
    );
    let stored: PolicyEffectiveResponse =
        ok(&fixture.service, &fixture.sam, policy_methods::POLICY_EFFECTIVE, json!({})).await;
    assert_eq!(stored.models[0].visible_from, ["type:user"]);
    assert_eq!(
        code(call(&fixture.service, &fixture.admin, policy_methods::POLICY_UPSERT, json!({"draft": {
            "subject": "group:gone", "body": {}}}))
        .await
        .unwrap_err()),
        AiccErrorCode::InvalidRequest
    );
}

#[tokio::test]
async fn c04_hidden_resources_do_not_leak_through_policies_or_admission() {
    let fixture = fixture().await;
    let _: AiccPolicyView = ok(&fixture.service, &fixture.admin, policy_methods::POLICY_UPSERT, json!({
        "draft": {"subject": "user:bob", "body": {"models": {
            "gpt-6@openrouter": {"visible": false}}}}}))
    .await;
    let error = error_of(admit(&fixture, &fixture.bob, &llm_call("gpt-6@openrouter", "openai", "gpt-6")).await);
    assert_eq!(error.code, AiccErrorCode::ResourceNotPermitted);
    assert!(error.details.is_none());
    let mine: PolicyListResponse = ok(&fixture.service, &fixture.bob, policy_methods::POLICY_LIST, json!({})).await;
    let own = mine
        .policies
        .iter()
        .find(|view| view.policy.subject.to_string() == "user:bob")
        .unwrap();
    assert!(own.policy.body.models.is_empty());
    let prices: PriceGetResponse = ok(&fixture.service, &fixture.bob, policy_methods::PRICE_GET, json!({})).await;
    assert!(prices.credit_rates.is_empty());
}

#[tokio::test]
async fn credit_methods_check_amounts_targets_and_readers() {
    let fixture = fixture().await;
    for (method, params) in [
        (policy_methods::CREDIT_RECHARGE, json!({"user_id": "bob", "recharge_id": "r", "amount": "0"})),
        (policy_methods::CREDIT_RECHARGE, json!({"user_id": "bob", "recharge_id": "r", "amount": "-1"})),
        (policy_methods::CREDIT_RECHARGE, json!({"user_id": "nobody", "recharge_id": "r", "amount": "1"})),
        (policy_methods::CREDIT_ADJUST, json!({"user_id": "bob", "adjust_id": "a", "amount": "-1", "note": " "})),
        (policy_methods::CREDIT_ADJUST, json!({"user_id": "bob", "adjust_id": "a", "amount": "0", "note": "x"})),
    ] {
        assert_eq!(
            code(call(&fixture.service, &fixture.admin, method, params.clone()).await.unwrap_err()),
            AiccErrorCode::InvalidRequest,
            "{params}"
        );
    }
    let adjusted: CreditEntryResponse = ok(&fixture.service, &fixture.admin, policy_methods::CREDIT_ADJUST, json!({
        "user_id": "bob", "adjust_id": "refund-1", "amount": "-1.25", "note": "refund"}))
    .await;
    assert_eq!(adjusted.account.balance, "-1.25");
    let found: CreditRechargeGetResponse =
        ok(&fixture.service, &fixture.admin, policy_methods::CREDIT_RECHARGE_GET, json!({"recharge_id": "refund-1"})).await;
    assert_eq!(found.account.unwrap().balance, "-1.25");
    let accounts: CreditAccountListResponse =
        ok(&fixture.service, &fixture.admin, policy_methods::CREDIT_ACCOUNT_LIST, json!({})).await;
    assert_eq!(accounts.accounts.len(), 3);
    let students: CreditAccountListResponse =
        ok(&fixture.service, &fixture.admin, policy_methods::CREDIT_ACCOUNT_LIST, json!({"group": "students"})).await;
    assert_eq!(students.accounts.len(), 1);
    assert_eq!(
        code(call(&fixture.service, &fixture.bob, policy_methods::CREDIT_ACCOUNT_GET, json!({"user_id": "sam"}))
            .await
            .unwrap_err()),
        AiccErrorCode::PolicyDenied
    );
    assert_eq!(
        code(call(&fixture.service, &fixture.bob, policy_methods::CREDIT_ACCOUNT_LIST, json!({}))
            .await
            .unwrap_err()),
        AiccErrorCode::PolicyDenied
    );
    assert_eq!(
        code(call(&fixture.service, &fixture.admin, policy_methods::PRICE_UPDATE, json!({
            "expected_revision": 0,
            "upserts": [{"entry_key": "exact:missing@openrouter", "pricing": {"input_token": 1}}]}))
        .await
        .unwrap_err()),
        AiccErrorCode::InvalidRequest
    );
    assert_eq!(
        code(call(&fixture.service, &fixture.admin, policy_methods::PRICE_UPDATE, json!({
            "expected_revision": 0, "credit_rates": {"USD": -1}}))
        .await
        .unwrap_err()),
        AiccErrorCode::InvalidRequest
    );
    let unknown_instance: PriceGetResponse = ok(&fixture.service, &fixture.admin, policy_methods::PRICE_UPDATE, json!({
        "expected_revision": 0,
        "upserts": [{"entry_key": "exact:gpt-6@future-provider", "pricing": {"input_token": 1}}]}))
    .await;
    assert_eq!(unknown_instance.entries.len(), 1, "a price may come before its Provider");
}

#[tokio::test]
async fn c19_unreadable_policy_state_refuses_instead_of_allowing() {
    let fixture = fixture().await;
    sqlx::query("DROP TABLE aicc_policy")
        .execute(fixture.service.storage.pool_for_tests())
        .await
        .unwrap();
    let error = error_of(admit(&fixture, &fixture.bob, &llm_call("gpt-6@openrouter", "openai", "gpt-6")).await);
    assert_eq!(error.code, AiccErrorCode::QuotaUnavailable);
    assert!(error.retriable);
    assert_eq!(
        code(fixture.service.access(&fixture.bob, &inventory()).await.unwrap_err()),
        AiccErrorCode::QuotaUnavailable
    );
}

#[tokio::test]
async fn c13_a_running_attempt_keeps_the_price_and_coefficients_it_was_admitted_with() {
    let fixture = fixture().await;
    start_charging(&fixture).await;
    let _: CreditEntryResponse = ok(&fixture.service, &fixture.admin, policy_methods::CREDIT_RECHARGE, json!({
        "user_id": "bob", "recharge_id": "order-1", "amount": "100"}))
    .await;
    let plan = admit(&fixture, &fixture.bob, &llm_call("gpt-6@openrouter", "openai", "gpt-6")).await.unwrap();
    // Mid-call: the price triples and Bob becomes free.
    let _: PriceGetResponse = ok(&fixture.service, &fixture.admin, policy_methods::PRICE_UPDATE, json!({
        "expected_revision": 1,
        "upserts": [{"entry_key": "model:openai:gpt-6", "pricing": {"input_token": 6, "output_token": 24}}]}))
    .await;
    let _: AiccPolicyView = ok(&fixture.service, &fixture.admin, policy_methods::POLICY_UPSERT, json!({
        "draft": {"subject": "user:bob", "body": {"price_coefficient": 0}}}))
    .await;
    let usage = AiUsage {
        input_tokens: Some(1_000_000),
        output_tokens: Some(0),
        total_tokens: Some(1_000_000),
        ..AiUsage::default()
    };
    let charged = fixture
        .service
        .settle(&plan, AttemptSettlement::Usage(usage.clone(), ApiType::Llm, None))
        .await;
    assert_eq!(charged, Some((2_000_000, CreditChargeStatus::Charged)));
    let next = admit(&fixture, &fixture.bob, &llm_call("gpt-6@openrouter", "openai", "gpt-6")).await.unwrap();
    let mut next = next;
    next.attempt_id = "attempt-2".into();
    assert_eq!(
        fixture
            .service
            .settle(&next, AttemptSettlement::Usage(usage, ApiType::Llm, None))
            .await,
        Some((0, CreditChargeStatus::Free))
    );
}
