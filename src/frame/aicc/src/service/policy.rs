use super::identity::{aicc_rpc_error, AccountFacts, UserDirectory};
use super::*;
use crate::execution::{AdmissionPort, AttemptSettlement, ExecutionRecord, SettledCharge};
use crate::policy::pricing::{
    parse_entry_key, pricing_from_api, pricing_is_zero, EntryKey, PinnedCreditKind, StoredPrice,
};
use crate::policy::{
    all_limits, is_token_metered, unavailable_resources, usage_total_tokens, validate_body, Access,
    AppliedLimit, ChargePlan, Evaluator, Inventory, InventoryModel, PinnedCreditPrice,
    PolicyPrincipal, PriceList, ProviderCostInfo, UnitPrice,
};
use crate::error::StorageError;
use crate::storage::{ChargeOutcome, CounterState, CreditEntryDraft, PolicyWriteError};
use buckyos_api::{
    format_credit_micro, parse_credit_micro, policy_methods, AiccPolicy, AiccPolicyView,
    CreditAccount, CreditAccountGetRequest, CreditAccountListRequest, CreditAccountListResponse,
    CreditAdjustRequest, CreditChargeStatus, CreditEntryKind, CreditEntryResponse,
    CreditInsufficientDetails, CreditLedgerListRequest, CreditLedgerListResponse,
    CreditPriceState, CreditRechargeGetRequest, CreditRechargeGetResponse, CreditRechargeRequest,
    EffectiveLimit, EffectiveModel, LimitResource, LimitStatus, LimitUnit, ModelCreditPrice,
    PolicyDeleteRequest, PolicyDeleteResponse, PolicyDraft, PolicyEffectiveRequest,
    PolicyEffectiveResponse, PolicyGetRequest, PolicyGetResponse, PolicyListRequest,
    PolicyListResponse, PolicyListScope, PolicySource, PolicyStatusRequest, PolicyStatusResponse,
    PolicySubject, PolicyUpsertRequest, PriceGetRequest, PriceGetResponse, PriceUpdateRequest,
    QuotaBlockingLimit, QuotaDenialDetails, ResourceUnpricedDetails, UserType,
    QUOTA_NEAR_LIMIT_PERCENT,
};

/// Upstream quota of Provider accounts (`quota_state`), used by routing only;
/// unrelated to the AICC user policy.
pub(crate) struct ProviderQuotaTruthPort {
    runtime: Arc<RuntimeState>,
}

impl ProviderQuotaTruthPort {
    pub(crate) fn new(runtime: Arc<RuntimeState>) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl QuotaTruthPort for ProviderQuotaTruthPort {
    async fn query(&self, lookup: &QuotaLookup) -> Result<QuotaSnapshot, QuotaSourceError> {
        let observation = match lookup.provider_instance_name.as_deref() {
            Some(name) => self
                .runtime
                .capture()
                .await
                .providers
                .quota_observation(name)
                .await
                .ok(),
            None => None,
        };
        Ok(provider_quota(observation.as_ref()))
    }
}

pub(super) fn provider_quota(provider: Option<&ProviderQuotaObservation>) -> QuotaSnapshot {
    let unknown = QuotaSnapshot {
        state: Some(QuotaState::Unknown),
        remaining_request_units: None,
        remaining_cost: None,
        reset_at: None,
    };
    let Some(provider) = provider else {
        return unknown;
    };
    let state = match provider.state {
        ProviderQuotaObservationState::Normal => QuotaState::Normal,
        ProviderQuotaObservationState::NearLimit => QuotaState::NearLimit,
        ProviderQuotaObservationState::Exhausted => QuotaState::Exhausted,
        ProviderQuotaObservationState::Unsupported | ProviderQuotaObservationState::QueryFailed => {
            return unknown;
        }
    };
    let remaining_cost = provider.remaining_cost.as_ref().and_then(|cost| {
        (cost.amount.is_finite() && cost.amount >= 0.0 && !cost.currency.trim().is_empty())
            .then(|| buckyos_api::Money::new(cost.amount, cost.currency.clone()))
    });
    QuotaSnapshot {
        state: Some(state),
        remaining_request_units: provider.remaining_request_units,
        remaining_cost,
        reset_at: None,
    }
}

/// Visibility evaluation, the Credit price of models and the `policy.*` /
/// `price.*` / `credit.*` methods.
#[async_trait]
pub(crate) trait PolicyPort: Send + Sync {
    async fn access(
        &self,
        caller: &AuthorizedCaller,
        inventory: &Inventory,
    ) -> Result<Access, RPCErrors>;

    /// The caller's Credit price of every visible model.
    async fn model_prices(
        &self,
        caller: &AuthorizedCaller,
        inventory: &Inventory,
    ) -> Result<BTreeMap<String, ModelCreditPrice>, RPCErrors>;

    async fn handle(
        &self,
        caller: &AuthorizedCaller,
        method: &str,
        params: Value,
        inventory: &Inventory,
    ) -> Result<Value, RPCErrors>;
}

pub(crate) struct PolicyService {
    storage: Arc<AiccStorage>,
    directory: Arc<dyn UserDirectory>,
}

const MAX_REQUEST_ID_LEN: usize = 128;
const MAX_NOTE_LEN: usize = 500;
const MAX_EXTERNAL_REF_BYTES: usize = 4_096;
const LEDGER_PAGE: u32 = 100;

fn now_i64() -> i64 {
    now_ms().min(i64::MAX as u64) as i64
}

fn parse<T: DeserializeOwned>(params: Value, method: &str) -> Result<T, RPCErrors> {
    serde_json::from_value(params)
        .map_err(|error| RPCErrors::ParseRequestError(format!("invalid {method} request: {error}")))
}

fn to_value<T: serde::Serialize>(value: T) -> Result<Value, RPCErrors> {
    serde_json::to_value(value).map_err(to_rpc_error)
}

fn invalid(message: impl Into<String>) -> RPCErrors {
    aicc_rpc_error(AiccErrorCode::InvalidRequest, message)
}

fn denied(message: &'static str) -> RPCErrors {
    aicc_rpc_error(AiccErrorCode::PolicyDenied, message)
}

pub(crate) fn unavailable_error(scope: &str, now_ms: i64) -> AiccError {
    let mut error = AiccError::new(
        AiccErrorCode::QuotaUnavailable,
        "AI policy or Credit state is temporarily unavailable",
    );
    error.retriable = true;
    error.details = serde_json::to_value(buckyos_api::QuotaUnavailableDetails {
        scope: scope.to_string(),
        occurred_at_ms: now_ms,
    })
    .ok();
    error
}

fn storage_unavailable(scope: &str) -> RPCErrors {
    unavailable_error(scope, now_i64()).to_krpc_error()
}

fn not_permitted_error() -> AiccError {
    AiccError::new(
        AiccErrorCode::ResourceNotPermitted,
        "the resource is not permitted",
    )
}

fn write_error(error: PolicyWriteError) -> RPCErrors {
    match error {
        PolicyWriteError::NotFound => invalid("the policy was not found"),
        PolicyWriteError::VersionConflict { expected, actual } => conflict_error(expected, actual),
        PolicyWriteError::IdempotencyConflict => aicc_rpc_error(
            AiccErrorCode::IdempotencyConflict,
            "the id was already used with different parameters",
        ),
        PolicyWriteError::Storage(error) => {
            log::warn!("policy write failed: {error}");
            storage_unavailable("policy_configuration")
        }
    }
}

pub(crate) fn principal_of(caller: &AuthorizedCaller) -> Option<PolicyPrincipal> {
    match &caller.principal {
        CallerPrincipal::Account {
            user_type, groups, ..
        } => Some(PolicyPrincipal {
            user_id: caller.user_id.clone(),
            user_type: *user_type,
            groups: groups.clone(),
        }),
        CallerPrincipal::System { .. } => None,
    }
}

fn principal_from(account: &AccountFacts) -> PolicyPrincipal {
    PolicyPrincipal {
        user_id: account.user_id.clone(),
        user_type: account.user_type,
        groups: account.groups.clone(),
    }
}

/// The inventory view of the model a call resolved to, with the Provider
/// cost the call pinned.
fn call_model(call: &crate::call::ResolvedProviderCall) -> InventoryModel {
    let provider_cost = call
        .pricing
        .pricing
        .as_ref()
        .map(|pricing| ProviderCostInfo {
            currency: pricing.currency.trim().to_ascii_uppercase(),
            zero: pricing_is_zero(pricing),
        })
        .filter(|cost| !cost.currency.is_empty())
        .or_else(|| {
            call.reported_cost_currency.as_ref().map(|currency| ProviderCostInfo {
                currency: currency.trim().to_ascii_uppercase(),
                zero: false,
            })
        });
    InventoryModel {
        exact_model: call.exact_model.clone(),
        provider_instance_name: call.provider_instance_name.clone(),
        provider_model_id: call.provider_model_id.clone(),
        variant: call.variant.clone(),
        model_driver_id: call.model_driver_id.clone(),
        origin_model_id: call.origin_model_id.clone(),
        provider_cost,
    }
}

/// The Credit price of a call, pinned at admission.
fn pin_price(
    prices: &PriceList,
    call: &crate::call::ResolvedProviderCall,
    provider_coefficient: f64,
    model_coefficient: f64,
) -> PinnedCreditPrice {
    let model = call_model(call);
    let kind = match prices.unit_price(&model) {
        UnitPrice::Credit { entry, .. } if pricing_is_zero(&entry.pricing) => PinnedCreditKind::Free,
        UnitPrice::Credit { entry, .. } => {
            let matched = crate::call::matched_rule_amount(&entry.pricing, &call.pricing_context);
            match crate::execution::PinnedPricingSnapshot::from_pricing(
                &entry.pricing,
                matched,
                std::time::SystemTime::now(),
            ) {
                Ok(Some(snapshot)) => PinnedCreditKind::Credit {
                    entry_key: entry.entry_key.clone(),
                    snapshot,
                },
                Ok(None) => PinnedCreditKind::Unpriced,
                Err(error) => {
                    log::warn!("price list entry {} cannot be pinned: {}", entry.entry_key, error.message);
                    PinnedCreditKind::Unpriced
                }
            }
        }
        UnitPrice::ProviderCost { rate, .. } if rate == 0.0 => PinnedCreditKind::Free,
        UnitPrice::ProviderCost { currency, rate } => PinnedCreditKind::ProviderCost { currency, rate },
        UnitPrice::None => PinnedCreditKind::Unpriced,
    };
    PinnedCreditPrice {
        kind,
        provider_coefficient,
        model_coefficient,
    }
}

fn limit_used(limit: &AppliedLimit, state: Option<&CounterState>, now: i64) -> (i64, Option<CounterState>) {
    match state.filter(|state| state.active_at(now)) {
        Some(state) => (
            match limit.unit {
                LimitUnit::Credit => state.used_credit_micro,
                LimitUnit::Token => state.used_tokens,
            },
            Some(*state),
        ),
        None => (0, None),
    }
}

fn format_amount(unit: LimitUnit, amount: i64) -> String {
    match unit {
        LimitUnit::Credit => format_credit_micro(amount),
        LimitUnit::Token => amount.to_string(),
    }
}

fn limit_status(amount: i64, used: i64) -> LimitStatus {
    if amount == 0 {
        LimitStatus::Forbidden
    } else if used >= amount {
        LimitStatus::Exhausted
    } else if (used as i128) * 100 >= (amount as i128) * QUOTA_NEAR_LIMIT_PERCENT as i128 {
        LimitStatus::NearLimit
    } else {
        LimitStatus::Normal
    }
}

fn effective_limit(
    limit: &AppliedLimit,
    state: Option<&CounterState>,
    now: i64,
    access: Option<&Access>,
) -> EffectiveLimit {
    let (used, window) = limit_used(limit, state, now);
    let hidden = access.is_some_and(|access| match &limit.resource {
        LimitResource::Provider {
            provider_instance_name,
        } => !access.allows_provider(provider_instance_name),
        LimitResource::Model { exact_model } => !access.allows_model(exact_model),
        LimitResource::All | LimitResource::Hidden => false,
    });
    EffectiveLimit {
        policy_id: limit.policy_id.clone(),
        source: limit.source,
        subject: limit.subject.clone(),
        counter: limit.counter.clone(),
        resource: if hidden {
            LimitResource::Hidden
        } else {
            limit.resource.clone()
        },
        period: limit.period,
        unit: limit.unit,
        amount: limit.amount_text.clone(),
        used: format_amount(limit.unit, used),
        window_start_ms: window.map(|window| window.window_start_ms),
        reset_at_ms: window.map(|window| window.window_end_ms),
        status: limit_status(limit.amount, used),
    }
}

impl PolicyService {
    pub(crate) fn new(storage: Arc<AiccStorage>, directory: Arc<dyn UserDirectory>) -> Self {
        Self { storage, directory }
    }

    /// Admin / Root account, or a system principal that passed the RBAC of
    /// the method.
    fn manages(caller: &AuthorizedCaller) -> bool {
        caller.is_admin() || matches!(caller.principal, CallerPrincipal::System { .. })
    }

    fn require_admin(caller: &AuthorizedCaller) -> Result<(), RPCErrors> {
        if Self::manages(caller) {
            Ok(())
        } else {
            Err(denied("this operation requires an administrator"))
        }
    }

    async fn policies(&self) -> Result<Vec<AiccPolicy>, RPCErrors> {
        self.storage
            .policies()
            .await
            .map_err(|_| storage_unavailable("policy_configuration"))
    }

    async fn prices(&self) -> Result<PriceList, RPCErrors> {
        self.storage
            .price_list()
            .await
            .map_err(|_| storage_unavailable("price_list"))
    }

    async fn balance(&self, user_id: &str) -> Result<(i64, i64), RPCErrors> {
        self.storage
            .credit_balance(user_id)
            .await
            .map_err(|_| storage_unavailable("credit_account"))
    }

    async fn counters_for(&self, limits: &[AppliedLimit]) -> Result<BTreeMap<String, CounterState>, RPCErrors> {
        let keys = limits.iter().map(|limit| limit.scope().key()).collect();
        self.storage
            .counters(&keys)
            .await
            .map_err(|_| storage_unavailable("quota_counter"))
    }

    /// The shared read-only check of one call: visibility, price, limits and
    /// balance, and the charge plan to pin when it passes.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn evaluate_call(
        &self,
        principal: &PolicyPrincipal,
        app_id: Option<&str>,
        operator_id: Option<String>,
        call: &crate::call::ResolvedProviderCall,
        attempt_id: &str,
        task_id: &str,
        usage_event_id: &str,
    ) -> Result<ChargePlan, AiccError> {
        let now = now_i64();
        let unavailable = |scope: &str, error: StorageError| {
            log::warn!("policy admission ({scope}) failed: {error}");
            unavailable_error(scope, now)
        };
        let policies = self
            .storage
            .policies()
            .await
            .map_err(|error| unavailable("policy_configuration", error))?;
        let prices = self
            .storage
            .price_list()
            .await
            .map_err(|error| unavailable("price_list", error))?;
        let evaluator = Evaluator::new(&policies, principal, app_id);
        let decision = evaluator.model(&call.provider_instance_name, &call.exact_model);
        if !decision.visible {
            return Err(not_permitted_error());
        }
        let price = pin_price(
            &prices,
            call,
            decision.provider_coefficient,
            decision.model_coefficient,
        );
        if decision.coefficient() != 0.0 && price.kind == PinnedCreditKind::Unpriced {
            let mut error = AiccError::new(
                AiccErrorCode::ResourceNotPermitted,
                "the model has no price; ask an administrator to price it",
            );
            error.details = serde_json::to_value(ResourceUnpricedDetails {
                reason: "unpriced".to_string(),
                provider_instance_name: call.provider_instance_name.clone(),
                exact_model: call.exact_model.clone(),
            })
            .ok();
            return Err(error);
        }
        let token_metered = is_token_metered(call.api_type);
        let limits = decision
            .limits
            .into_iter()
            .filter(|limit| token_metered || limit.unit == LimitUnit::Credit)
            .collect::<Vec<_>>();
        let keys = limits.iter().map(|limit| limit.scope().key()).collect();
        let counters = self
            .storage
            .counters(&keys)
            .await
            .map_err(|error| unavailable("quota_counter", error))?;
        let blocking = limits
            .iter()
            .filter_map(|limit| {
                let state = counters.get(&limit.scope().key());
                let (used, window) = limit_used(limit, state, now);
                (used >= limit.amount).then(|| QuotaBlockingLimit {
                    policy_id: limit.policy_id.clone(),
                    source: limit.source,
                    subject: limit.subject.clone(),
                    resource: limit.resource.clone(),
                    period: limit.period,
                    unit: limit.unit,
                    used: format_amount(limit.unit, used),
                    amount: limit.amount_text.clone(),
                    reset_at_ms: (limit.amount > 0)
                        .then(|| window.map(|window| window.window_end_ms))
                        .flatten(),
                })
            })
            .collect::<Vec<_>>();
        if !blocking.is_empty() {
            let reset_at_ms = if blocking.iter().all(|limit| limit.reset_at_ms.is_some()) {
                blocking.iter().filter_map(|limit| limit.reset_at_ms).max()
            } else {
                None
            };
            let mut error = AiccError::new(AiccErrorCode::QuotaExhausted, "a usage limit is used up");
            error.details = serde_json::to_value(QuotaDenialDetails {
                provider_instance_name: call.provider_instance_name.clone(),
                exact_model: call.exact_model.clone(),
                blocking,
                reset_at_ms,
            })
            .ok();
            return Err(error);
        }
        if price.needs_payment() {
            let (balance, _) = self
                .storage
                .credit_balance(&principal.user_id)
                .await
                .map_err(|error| unavailable("credit_account", error))?;
            if balance <= 0 {
                let mut error = AiccError::new(
                    AiccErrorCode::CreditInsufficient,
                    "your AI Credit balance is used up; ask an administrator to recharge it",
                );
                error.details = serde_json::to_value(CreditInsufficientDetails {
                    balance: format_credit_micro(balance),
                    provider_instance_name: call.provider_instance_name.clone(),
                    exact_model: call.exact_model.clone(),
                })
                .ok();
                return Err(error);
            }
        }
        let mut counters = limits.iter().map(AppliedLimit::scope).collect::<Vec<_>>();
        counters.sort();
        counters.dedup();
        Ok(ChargePlan {
            attempt_id: attempt_id.to_string(),
            task_id: task_id.to_string(),
            usage_event_id: usage_event_id.to_string(),
            user_id: principal.user_id.clone(),
            app_id: app_id.map(str::to_owned),
            operator_id,
            provider_instance_name: call.provider_instance_name.clone(),
            exact_model: call.exact_model.clone(),
            admitted_at_ms: now,
            subjects: principal.subjects(),
            price,
            counters,
            token_metered,
        })
    }

    /// Read-only check of one route candidate.
    pub(crate) async fn precheck(
        &self,
        caller: &AuthorizedCaller,
        call: &crate::call::ResolvedProviderCall,
    ) -> Result<(), AiccError> {
        let Some(principal) = principal_of(caller) else {
            return Ok(());
        };
        self.evaluate_call(
            &principal,
            caller.app_id.as_deref(),
            None,
            call,
            "precheck",
            "precheck",
            "precheck",
        )
        .await
        .map(|_| ())
    }

    async fn access_for(
        &self,
        principal: Option<&PolicyPrincipal>,
        app_id: Option<&str>,
        inventory: &Inventory,
    ) -> Result<Access, RPCErrors> {
        let Some(principal) = principal else {
            return Ok(Access::Unrestricted);
        };
        let policies = self.policies().await?;
        let prices = self.prices().await?;
        Ok(Access::Restricted(
            Evaluator::new(&policies, principal, app_id).access(inventory, &prices),
        ))
    }

    fn concerns(caller: &AuthorizedCaller, policy: &AiccPolicy) -> bool {
        let Some(principal) = principal_of(caller) else {
            return false;
        };
        match &policy.subject {
            subject @ PolicySubject::UserType(_) => {
                *subject == PolicySubject::for_user_type(principal.user_type)
            }
            PolicySubject::Group(group) => principal.groups.contains(group),
            PolicySubject::User(user_id) | PolicySubject::UserApp { user_id, .. } => {
                user_id == &principal.user_id
            }
            PolicySubject::App(_) => true,
        }
    }

    fn editable(caller: &AuthorizedCaller, policy: &AiccPolicy) -> bool {
        match policy.source {
            PolicySource::Admin => Self::manages(caller),
            PolicySource::User => policy.subject.user_id() == Some(caller.user_id.as_str())
                && principal_of(caller).is_some(),
        }
    }

    /// A non-admin sees no Provider or model they cannot see themselves.
    fn view(
        caller: &AuthorizedCaller,
        mut policy: AiccPolicy,
        access: &Access,
        inventory: &Inventory,
    ) -> AiccPolicyView {
        let unavailable = unavailable_resources(&policy.body, inventory);
        if !Self::manages(caller) {
            policy
                .body
                .providers
                .retain(|provider, _| access.allows_provider(provider));
            policy.body.models.retain(|model, _| access.allows_model(model));
        }
        AiccPolicyView {
            editable: Self::editable(caller, &policy),
            unavailable_resources: if Self::manages(caller) {
                unavailable
            } else {
                Vec::new()
            },
            policy,
        }
    }

    async fn caller_access(&self, caller: &AuthorizedCaller, inventory: &Inventory) -> Result<Access, RPCErrors> {
        self.access_for(principal_of(caller).as_ref(), None, inventory).await
    }

    /// Apps a user may put their own policy on: installed Apps, their
    /// Agents, AIWorkspace and Apps already seen in their usage.
    async fn user_apps(&self, user_id: &str) -> Result<BTreeSet<String>, RPCErrors> {
        let mut apps = self.directory.user_apps(user_id).await?;
        apps.insert("aiworkspace".to_string());
        apps.extend(
            self.storage
                .usage_apps(user_id)
                .await
                .map_err(|_| storage_unavailable("usage"))?,
        );
        Ok(apps)
    }

    /// Check a draft against the caller's rights; returns its source.
    async fn validate_draft(
        &self,
        caller: &AuthorizedCaller,
        draft: &PolicyDraft,
        inventory: &Inventory,
    ) -> Result<PolicySource, RPCErrors> {
        let source = draft.source.unwrap_or(if Self::manages(caller) {
            PolicySource::Admin
        } else {
            PolicySource::User
        });
        validate_body(source, &draft.subject, &draft.body).map_err(|message| {
            if source == PolicySource::User && !matches!(draft.subject, PolicySubject::UserApp { .. }) {
                denied("a personal policy applies to one of your own Apps")
            } else {
                invalid(message)
            }
        })?;
        match source {
            PolicySource::Admin => {
                Self::require_admin(caller)?;
                match &draft.subject {
                    PolicySubject::Group(group) => {
                        if !self.directory.defined_groups().await?.contains(group) {
                            return Err(invalid("the policy names an undefined user group"));
                        }
                    }
                    PolicySubject::User(user_id) | PolicySubject::UserApp { user_id, .. } => {
                        if self.directory.account(user_id).await?.is_none() {
                            return Err(invalid("the user is not an account of this Zone"));
                        }
                    }
                    PolicySubject::UserType(_) | PolicySubject::App(_) => {}
                }
            }
            PolicySource::User => {
                let PolicySubject::UserApp { user_id, app_id } = &draft.subject else {
                    return Err(denied("a personal policy applies to one of your own Apps"));
                };
                if user_id != &caller.user_id || principal_of(caller).is_none() {
                    return Err(denied("a personal policy applies to one of your own Apps"));
                }
                if !self.user_apps(user_id).await?.contains(app_id) {
                    return Err(denied("the App is not one of yours"));
                }
                let access = self.caller_access(caller, inventory).await?;
                let names_hidden = draft
                    .body
                    .providers
                    .keys()
                    .any(|provider| !access.allows_provider(provider))
                    || draft.body.models.keys().any(|model| !access.allows_model(model));
                if names_hidden {
                    return Err(not_permitted_error().to_krpc_error());
                }
            }
        }
        Ok(source)
    }

    async fn list(
        &self,
        caller: &AuthorizedCaller,
        request: PolicyListRequest,
        inventory: &Inventory,
    ) -> Result<PolicyListResponse, RPCErrors> {
        let policies = self.policies().await?;
        let policies = match request.scope {
            PolicyListScope::Manage => {
                Self::require_admin(caller)?;
                policies
            }
            PolicyListScope::Mine => policies
                .into_iter()
                .filter(|policy| Self::concerns(caller, policy))
                .filter(|policy| {
                    !matches!(policy.subject, PolicySubject::App(_)) || Self::manages(caller)
                })
                .collect(),
        };
        let access = self.caller_access(caller, inventory).await?;
        Ok(PolicyListResponse {
            policies: policies
                .into_iter()
                .map(|policy| Self::view(caller, policy, &access, inventory))
                .collect(),
        })
    }

    async fn policy_for(&self, caller: &AuthorizedCaller, policy_id: &str) -> Result<AiccPolicy, RPCErrors> {
        let policy = self
            .storage
            .policy(policy_id)
            .await
            .map_err(|_| storage_unavailable("policy_configuration"))?
            .ok_or_else(|| invalid("the policy was not found"))?;
        if !Self::manages(caller) && !Self::concerns(caller, &policy) {
            return Err(invalid("the policy was not found"));
        }
        Ok(policy)
    }

    async fn get(
        &self,
        caller: &AuthorizedCaller,
        request: PolicyGetRequest,
        inventory: &Inventory,
    ) -> Result<PolicyGetResponse, RPCErrors> {
        let policy = self.policy_for(caller, &request.policy_id).await?;
        let audit = if Self::manages(caller) || policy.source == PolicySource::User {
            self.storage
                .policy_audit(&policy.policy_id)
                .await
                .map_err(|_| storage_unavailable("policy_audit"))?
        } else {
            Vec::new()
        };
        let access = self.caller_access(caller, inventory).await?;
        Ok(PolicyGetResponse {
            policy: Self::view(caller, policy, &access, inventory),
            audit,
        })
    }

    async fn upsert(
        &self,
        caller: &AuthorizedCaller,
        request: PolicyUpsertRequest,
        inventory: &Inventory,
    ) -> Result<AiccPolicyView, RPCErrors> {
        let source = self.validate_draft(caller, &request.draft, inventory).await?;
        let policy = self
            .storage
            .upsert_policy(
                source,
                &request.draft.subject,
                request.draft.enabled,
                &request.draft.body,
                request.expected_version,
                &caller.user_id,
                now_i64(),
            )
            .await
            .map_err(write_error)?;
        let access = self.caller_access(caller, inventory).await?;
        Ok(Self::view(caller, policy, &access, inventory))
    }

    async fn delete(
        &self,
        caller: &AuthorizedCaller,
        request: PolicyDeleteRequest,
    ) -> Result<PolicyDeleteResponse, RPCErrors> {
        let policy = self.policy_for(caller, &request.policy_id).await?;
        if !Self::editable(caller, &policy) {
            return Err(denied("the policy cannot be changed by the caller"));
        }
        self.storage
            .delete_policy(
                &request.policy_id,
                request.expected_version,
                &caller.user_id,
                now_i64(),
            )
            .await
            .map_err(write_error)?;
        Ok(PolicyDeleteResponse {
            ok: true,
            policy_id: request.policy_id,
        })
    }

    async fn account_of(&self, user_id: &str) -> Result<AccountFacts, RPCErrors> {
        self.directory
            .account(user_id)
            .await?
            .ok_or_else(|| invalid("the user is not an account of this Zone"))
    }

    /// The target user of a per-user read: the caller, or anyone for an admin.
    async fn target_user(
        &self,
        caller: &AuthorizedCaller,
        user_id: Option<&str>,
    ) -> Result<PolicyPrincipal, RPCErrors> {
        match user_id.filter(|user_id| *user_id != caller.user_id) {
            Some(user_id) => {
                Self::require_admin(caller)?;
                Ok(principal_from(&self.account_of(user_id).await?))
            }
            None => principal_of(caller).ok_or_else(|| invalid("the caller has no AI policy")),
        }
    }

    fn usable_models(
        policies: &[AiccPolicy],
        principal: &PolicyPrincipal,
        app_id: Option<&str>,
        inventory: &Inventory,
        prices: &PriceList,
    ) -> BTreeSet<String> {
        let access = Evaluator::new(policies, principal, app_id).access(inventory, prices);
        access
            .visible
            .values()
            .flatten()
            .filter(|model| !access.unpriced.contains(*model))
            .cloned()
            .collect()
    }

    async fn effective(
        &self,
        caller: &AuthorizedCaller,
        request: PolicyEffectiveRequest,
        inventory: &Inventory,
    ) -> Result<PolicyEffectiveResponse, RPCErrors> {
        let principal = self.target_user(caller, request.user_id.as_deref()).await?;
        let app_id = request.app_id.as_deref();
        let stored = self.policies().await?;
        let prices = self.prices().await?;
        let mut drafted = stored.clone();
        if let Some(draft) = &request.draft {
            let source = self.validate_draft(caller, draft, inventory).await?;
            let now = now_i64();
            let existing = drafted
                .iter()
                .position(|policy| policy.source == source && policy.subject == draft.subject);
            let policy = AiccPolicy {
                policy_id: existing
                    .map(|index| drafted[index].policy_id.clone())
                    .unwrap_or_else(|| "draft".to_string()),
                source,
                subject: draft.subject.clone(),
                enabled: draft.enabled,
                body: draft.body.clone(),
                version: 0,
                created_by: caller.user_id.clone(),
                created_at_ms: now,
                updated_by: caller.user_id.clone(),
                updated_at_ms: now,
            };
            match existing {
                Some(index) => drafted[index] = policy,
                None => drafted.push(policy),
            }
        }
        let evaluator = Evaluator::new(&drafted, &principal, app_id);
        let mut models = Vec::new();
        let mut unpriced_models = Vec::new();
        for model in inventory.models() {
            let decision = evaluator.model(&model.provider_instance_name, &model.exact_model);
            if !decision.visible {
                continue;
            }
            let price = prices.describe(model, decision.provider_coefficient, decision.model_coefficient);
            let usable = price.state != CreditPriceState::Unpriced;
            if !usable {
                unpriced_models.push(model.exact_model.clone());
            }
            models.push(EffectiveModel {
                exact_model: model.exact_model.clone(),
                provider_instance_name: model.provider_instance_name.clone(),
                visible_from: decision.visible_from,
                coefficient_from: decision.coefficient_from,
                price,
                usable,
            });
        }
        let limits = all_limits(&evaluator, inventory);
        let counters = self.counters_for(&limits).await?;
        let now = now_i64();
        let viewer_access = if Self::manages(caller) {
            None
        } else {
            Some(self.caller_access(caller, inventory).await?)
        };
        let limits = limits
            .iter()
            .map(|limit| effective_limit(limit, counters.get(&limit.scope().key()), now, viewer_access.as_ref()))
            .collect();
        let (balance, _) = self.balance(&principal.user_id).await?;
        let mut response = PolicyEffectiveResponse {
            user_id: principal.user_id.clone(),
            app_id: request.app_id.clone(),
            balance: format_credit_micro(balance),
            models,
            limits,
            unpriced_models,
            added: Vec::new(),
            removed: Vec::new(),
            affected_users: Vec::new(),
            caller_loses: Vec::new(),
        };
        if let Some(draft) = &request.draft {
            let before = Self::usable_models(&stored, &principal, app_id, inventory, &prices);
            let after = Self::usable_models(&drafted, &principal, app_id, inventory, &prices);
            response.added = after.difference(&before).cloned().collect();
            response.removed = before.difference(&after).cloned().collect();
            response.affected_users = self
                .directory
                .accounts()
                .await?
                .into_iter()
                .filter(|account| account.user_type != UserType::Root)
                .filter(|account| match &draft.subject {
                    subject @ PolicySubject::UserType(_) => {
                        *subject == PolicySubject::for_user_type(account.user_type)
                    }
                    PolicySubject::Group(group) => account.groups.contains(group),
                    PolicySubject::User(user_id) | PolicySubject::UserApp { user_id, .. } => {
                        user_id == &account.user_id
                    }
                    PolicySubject::App(_) => true,
                })
                .map(|account| account.user_id)
                .collect();
            if let Some(mine) = principal_of(caller) {
                let caller_app = draft.subject.app_id();
                let before = Self::usable_models(&stored, &mine, caller_app, inventory, &prices);
                let after = Self::usable_models(&drafted, &mine, caller_app, inventory, &prices);
                response.caller_loses = before.difference(&after).cloned().collect();
            }
        }
        Ok(response)
    }

    async fn status(
        &self,
        caller: &AuthorizedCaller,
        request: PolicyStatusRequest,
        inventory: &Inventory,
    ) -> Result<PolicyStatusResponse, RPCErrors> {
        let principal = principal_of(caller).ok_or_else(|| invalid("the caller has no AI policy"))?;
        let policies = self.policies().await?;
        let app_id = request.app_id.as_deref();
        let evaluator = Evaluator::new(&policies, &principal, app_id);
        let limits = all_limits(&evaluator, inventory)
            .into_iter()
            .filter(|limit| {
                app_id.is_some() || matches!(limit.counter, buckyos_api::LimitCounter::User { .. })
            })
            .collect::<Vec<_>>();
        let counters = self.counters_for(&limits).await?;
        let access = self.caller_access(caller, inventory).await?;
        let now = now_i64();
        let limits = limits
            .iter()
            .map(|limit| effective_limit(limit, counters.get(&limit.scope().key()), now, Some(&access)))
            .collect::<Vec<_>>();
        let near_limit_count = limits
            .iter()
            .filter(|limit| limit.status == LimitStatus::NearLimit)
            .count() as u64;
        let exhausted_count = limits
            .iter()
            .filter(|limit| matches!(limit.status, LimitStatus::Exhausted | LimitStatus::Forbidden))
            .count() as u64;
        let (balance, _) = self.balance(&principal.user_id).await?;
        Ok(PolicyStatusResponse {
            user_id: principal.user_id,
            balance: format_credit_micro(balance),
            limits,
            near_limit_count,
            exhausted_count,
        })
    }

    async fn price_get(
        &self,
        caller: &AuthorizedCaller,
        inventory: &Inventory,
    ) -> Result<PriceGetResponse, RPCErrors> {
        let prices = self.prices().await?;
        if Self::manages(caller) {
            return Ok(PriceGetResponse {
                revision: prices.revision,
                entries: prices.entries.values().map(StoredPrice::to_entry).collect(),
                credit_rates: prices.credit_rates.clone(),
                unpriced_models: prices.unpriced(inventory),
            });
        }
        let access = self.caller_access(caller, inventory).await?;
        let keys = inventory
            .models()
            .filter(|model| access.allows_model(&model.exact_model))
            .filter_map(|model| prices.entry_for(model).map(|(_, entry)| entry.entry_key.clone()))
            .collect::<BTreeSet<_>>();
        Ok(PriceGetResponse {
            revision: prices.revision,
            entries: keys
                .iter()
                .filter_map(|key| prices.entries.get(key))
                .map(StoredPrice::to_entry)
                .collect(),
            credit_rates: BTreeMap::new(),
            unpriced_models: Vec::new(),
        })
    }

    async fn price_update(
        &self,
        caller: &AuthorizedCaller,
        request: PriceUpdateRequest,
        inventory: &Inventory,
    ) -> Result<PriceGetResponse, RPCErrors> {
        Self::require_admin(caller)?;
        let now = now_i64();
        let mut upserts = Vec::with_capacity(request.upserts.len());
        let mut keys = BTreeSet::new();
        for input in &request.upserts {
            let key = parse_entry_key(&input.entry_key).map_err(invalid)?;
            if !keys.insert(input.entry_key.clone()) || request.deletes.contains(&input.entry_key) {
                return Err(invalid("a price key appears more than once"));
            }
            if let EntryKey::Exact {
                provider_model_id,
                variant,
                provider_instance_name,
            } = &key
            {
                if let Some(models) = inventory.providers.get(provider_instance_name) {
                    let known = models.values().any(|model| {
                        &model.provider_model_id == provider_model_id
                            && (variant.is_none() || &model.variant == variant)
                    });
                    if !known {
                        return Err(invalid(format!(
                            "{} is not a model or variant of {provider_instance_name}",
                            input.entry_key
                        )));
                    }
                }
            }
            if input.note.as_ref().is_some_and(|note| note.len() > MAX_NOTE_LEN) {
                return Err(invalid("the price note is too long"));
            }
            let pricing = pricing_from_api(&input.pricing)
                .map_err(|error| invalid(format!("{}: {error}", input.entry_key)))?;
            upserts.push(StoredPrice {
                entry_key: input.entry_key.clone(),
                pricing,
                note: input.note.clone(),
                updated_by: caller.user_id.clone(),
                updated_at_ms: now,
            });
        }
        let rates = request
            .credit_rates
            .as_ref()
            .map(|rates| {
                rates
                    .iter()
                    .map(|(currency, rate)| {
                        let currency = currency.trim().to_ascii_uppercase();
                        if currency.is_empty() || currency == "CREDIT" || !rate.is_finite() || *rate < 0.0 {
                            Err(invalid("a Credit rate needs a currency and a non-negative number"))
                        } else {
                            Ok((currency, *rate))
                        }
                    })
                    .collect::<Result<BTreeMap<_, _>, _>>()
            })
            .transpose()?;
        self.storage
            .update_prices(
                request.expected_revision,
                &upserts,
                &request.deletes,
                rates.as_ref(),
                &caller.user_id,
                now,
            )
            .await
            .map_err(write_error)?;
        self.price_get(caller, inventory).await
    }

    fn account_view(user_id: &str, (balance, updated_at_ms): (i64, i64)) -> CreditAccount {
        CreditAccount {
            user_id: user_id.to_string(),
            balance: format_credit_micro(balance),
            updated_at_ms,
        }
    }

    async fn account_get(
        &self,
        caller: &AuthorizedCaller,
        request: CreditAccountGetRequest,
    ) -> Result<CreditAccount, RPCErrors> {
        let principal = self.target_user(caller, request.user_id.as_deref()).await?;
        Ok(Self::account_view(&principal.user_id, self.balance(&principal.user_id).await?))
    }

    async fn account_list(
        &self,
        caller: &AuthorizedCaller,
        request: CreditAccountListRequest,
    ) -> Result<CreditAccountListResponse, RPCErrors> {
        Self::require_admin(caller)?;
        let balances = self
            .storage
            .credit_balances()
            .await
            .map_err(|_| storage_unavailable("credit_account"))?;
        let accounts = self
            .directory
            .accounts()
            .await?
            .into_iter()
            .filter(|account| account.user_type != UserType::Root)
            .filter(|account| {
                request
                    .group
                    .as_ref()
                    .is_none_or(|group| account.groups.contains(group))
            })
            .map(|account| {
                let balance = balances.get(&account.user_id).copied().unwrap_or((0, 0));
                Self::account_view(&account.user_id, balance)
            })
            .collect();
        Ok(CreditAccountListResponse { accounts })
    }

    async fn ledger(
        &self,
        caller: &AuthorizedCaller,
        request: CreditLedgerListRequest,
    ) -> Result<CreditLedgerListResponse, RPCErrors> {
        let principal = self.target_user(caller, request.user_id.as_deref()).await?;
        let page = request.limit.unwrap_or(LEDGER_PAGE).clamp(1, 500) as usize;
        let (items, next_cursor) = self
            .storage
            .credit_ledger(
                &principal.user_id,
                request.start_time_ms.unwrap_or(0),
                request.end_time_ms.unwrap_or(i64::MAX),
                request.cursor.as_deref(),
                page,
            )
            .await
            .map_err(|error| match error {
                StorageError::InvalidCursor => invalid("the cursor is invalid"),
                _ => storage_unavailable("credit_ledger"),
            })?;
        let (balance, _) = self.balance(&principal.user_id).await?;
        Ok(CreditLedgerListResponse {
            user_id: principal.user_id,
            balance: format_credit_micro(balance),
            items,
            next_cursor,
        })
    }

    fn operator(caller: &AuthorizedCaller) -> String {
        match caller.app_id.as_deref() {
            Some(app) if !app.starts_with("system:") => format!("{}@{app}", caller.user_id),
            _ => caller.user_id.clone(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn apply_entry(
        &self,
        caller: &AuthorizedCaller,
        user_id: &str,
        request_id: &str,
        kind: CreditEntryKind,
        amount_micro: i64,
        note: Option<&str>,
        external_ref: Option<&Value>,
    ) -> Result<CreditEntryResponse, RPCErrors> {
        if request_id.trim().is_empty() || request_id.len() > MAX_REQUEST_ID_LEN {
            return Err(invalid("the id is invalid"));
        }
        if note.is_some_and(|note| note.len() > MAX_NOTE_LEN) {
            return Err(invalid("the note is too long"));
        }
        if external_ref.is_some_and(|value| value.to_string().len() > MAX_EXTERNAL_REF_BYTES) {
            return Err(invalid("external_ref is too large"));
        }
        self.account_of(user_id).await?;
        let operator = Self::operator(caller);
        let (entry, balance, replayed) = self
            .storage
            .apply_credit_entry(CreditEntryDraft {
                user_id,
                request_id,
                kind,
                amount_micro,
                note,
                external_ref,
                operator: &operator,
                now_ms: now_i64(),
            })
            .await
            .map_err(write_error)?;
        Ok(CreditEntryResponse {
            account: Self::account_view(user_id, (balance, entry.created_at_ms)),
            entry,
            replayed,
        })
    }

    async fn recharge(
        &self,
        caller: &AuthorizedCaller,
        request: CreditRechargeRequest,
    ) -> Result<CreditEntryResponse, RPCErrors> {
        let amount = parse_credit_micro(&request.amount)
            .filter(|amount| *amount > 0)
            .ok_or_else(|| invalid("a recharge adds a positive amount (up to six decimals)"))?;
        self.apply_entry(
            caller,
            &request.user_id,
            &request.recharge_id,
            CreditEntryKind::Recharge,
            amount,
            request.note.as_deref(),
            request.external_ref.as_ref(),
        )
        .await
    }

    async fn adjust(
        &self,
        caller: &AuthorizedCaller,
        request: CreditAdjustRequest,
    ) -> Result<CreditEntryResponse, RPCErrors> {
        let amount = parse_credit_micro(&request.amount)
            .filter(|amount| *amount != 0)
            .ok_or_else(|| invalid("an adjustment is a non-zero signed amount (up to six decimals)"))?;
        if request.note.trim().is_empty() {
            return Err(invalid("an adjustment needs a note"));
        }
        self.apply_entry(
            caller,
            &request.user_id,
            &request.adjust_id,
            CreditEntryKind::Adjust,
            amount,
            Some(&request.note),
            request.external_ref.as_ref(),
        )
        .await
    }

    async fn recharge_get(
        &self,
        request: CreditRechargeGetRequest,
    ) -> Result<CreditRechargeGetResponse, RPCErrors> {
        let entry = self
            .storage
            .credit_entry(&request.recharge_id)
            .await
            .map_err(|_| storage_unavailable("credit_account"))?;
        let account = match &entry {
            Some(entry) => Some(Self::account_view(&entry.user_id, self.balance(&entry.user_id).await?)),
            None => None,
        };
        Ok(CreditRechargeGetResponse { entry, account })
    }
}

#[async_trait]
impl PolicyPort for PolicyService {
    async fn access(
        &self,
        caller: &AuthorizedCaller,
        inventory: &Inventory,
    ) -> Result<Access, RPCErrors> {
        self.access_for(principal_of(caller).as_ref(), caller.app_id.as_deref(), inventory)
            .await
    }

    async fn model_prices(
        &self,
        caller: &AuthorizedCaller,
        inventory: &Inventory,
    ) -> Result<BTreeMap<String, ModelCreditPrice>, RPCErrors> {
        let prices = self.prices().await?;
        let Some(principal) = principal_of(caller) else {
            return Ok(inventory
                .models()
                .map(|model| (model.exact_model.clone(), prices.describe(model, 0.0, 1.0)))
                .collect());
        };
        let policies = self.policies().await?;
        let evaluator = Evaluator::new(&policies, &principal, caller.app_id.as_deref());
        Ok(inventory
            .models()
            .map(|model| {
                let decision = evaluator.model(&model.provider_instance_name, &model.exact_model);
                (
                    model.exact_model.clone(),
                    prices.describe(model, decision.provider_coefficient, decision.model_coefficient),
                )
            })
            .collect())
    }

    async fn handle(
        &self,
        caller: &AuthorizedCaller,
        method: &str,
        params: Value,
        inventory: &Inventory,
    ) -> Result<Value, RPCErrors> {
        match method {
            policy_methods::POLICY_LIST => {
                to_value(self.list(caller, parse(params, method)?, inventory).await?)
            }
            policy_methods::POLICY_GET => {
                to_value(self.get(caller, parse(params, method)?, inventory).await?)
            }
            policy_methods::POLICY_UPSERT => {
                to_value(self.upsert(caller, parse(params, method)?, inventory).await?)
            }
            policy_methods::POLICY_DELETE => {
                to_value(self.delete(caller, parse(params, method)?).await?)
            }
            policy_methods::POLICY_EFFECTIVE => {
                to_value(self.effective(caller, parse(params, method)?, inventory).await?)
            }
            policy_methods::POLICY_STATUS => {
                to_value(self.status(caller, parse(params, method)?, inventory).await?)
            }
            policy_methods::PRICE_GET => {
                let _: PriceGetRequest = parse(params, method)?;
                to_value(self.price_get(caller, inventory).await?)
            }
            policy_methods::PRICE_UPDATE => {
                to_value(self.price_update(caller, parse(params, method)?, inventory).await?)
            }
            policy_methods::CREDIT_ACCOUNT_GET => {
                to_value(self.account_get(caller, parse(params, method)?).await?)
            }
            policy_methods::CREDIT_ACCOUNT_LIST => {
                to_value(self.account_list(caller, parse(params, method)?).await?)
            }
            policy_methods::CREDIT_LEDGER_LIST => {
                to_value(self.ledger(caller, parse(params, method)?).await?)
            }
            policy_methods::CREDIT_RECHARGE => {
                to_value(self.recharge(caller, parse(params, method)?).await?)
            }
            policy_methods::CREDIT_RECHARGE_GET => {
                to_value(self.recharge_get(parse(params, method)?).await?)
            }
            policy_methods::CREDIT_ADJUST => {
                to_value(self.adjust(caller, parse(params, method)?).await?)
            }
            _ => Err(RPCErrors::UnknownMethod(method.to_string())),
        }
    }
}

#[async_trait]
impl AdmissionPort for PolicyService {
    async fn admit(
        &self,
        record: &ExecutionRecord,
        call: &crate::call::ResolvedProviderCall,
        attempt_id: &str,
    ) -> Result<Option<ChargePlan>, AiccError> {
        let Some(principal) = record.principal.as_ref() else {
            return Ok(None);
        };
        let operator_id = record
            .attribution
            .as_ref()
            .and_then(|attribution| attribution.operator_id.clone());
        self.evaluate_call(
            principal,
            record.caller_app_id.as_deref(),
            operator_id,
            call,
            attempt_id,
            &record.task_id,
            &record.usage_event_id,
        )
        .await
        .map(Some)
    }

    async fn settle(&self, plan: &ChargePlan, settlement: AttemptSettlement) -> Option<SettledCharge> {
        let (outcome, usage_event_id) = match settlement {
            AttemptSettlement::Usage(usage, api_type, cost) => {
                let (credit_micro, status) = plan.price.charge(&usage, cost.as_ref());
                (
                    ChargeOutcome {
                        credit_micro,
                        tokens: usage_total_tokens(&usage, api_type).unwrap_or(0),
                        status,
                    },
                    Some(plan.usage_event_id.as_str()),
                )
            }
            AttemptSettlement::Released => return None,
            AttemptSettlement::Unknown => (
                ChargeOutcome {
                    credit_micro: 0,
                    tokens: 0,
                    status: CreditChargeStatus::UnknownUsage,
                },
                None,
            ),
        };
        match self
            .storage
            .settle_charge(plan, outcome, usage_event_id, now_i64())
            .await
        {
            Ok(stored) => Some((stored.credit_micro, stored.status)),
            Err(error) => {
                log::warn!(
                    "credit settlement of {} failed ({outcome:?}): {error}",
                    plan.attempt_id
                );
                None
            }
        }
    }
}

#[cfg(test)]
mod tests;
