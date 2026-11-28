//! User policy evaluation (doc/aicc/aicc_user_policy.md §2).
//!
//! Pure functions over the stored policies, the price list and the current
//! inventory: who may see which Provider / Model, which coefficients and time
//! window limits apply, and what a call costs. Nothing here touches the
//! database; `storage::policy` keeps the ledger and `service::policy` the
//! kRPC methods.

pub(crate) mod pricing;

use std::collections::{BTreeMap, BTreeSet};

use buckyos_api::{
    AiccPolicy, ApiType, CreditPriceState, LimitCounter, LimitPeriod, LimitResource, LimitUnit,
    PolicyBody, PolicyLimit, PolicySource, PolicySubject, ProviderVisibility, UserType,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(crate) use pricing::{PinnedCreditPrice, PriceList, UnitPrice};

/// Token limits count only these API types: they report a reliable token
/// count. Other operations are under Credit limits and the balance only.
pub(crate) fn is_token_metered(api_type: ApiType) -> bool {
    matches!(
        api_type,
        ApiType::Llm | ApiType::Decision | ApiType::EmbeddingText | ApiType::EmbeddingMultimodal
    )
}

/// Normalized total of a reported usage: the reported total, or input plus
/// output when both are known (input alone for an embedding). `None` means
/// the usage is unknown, never zero.
pub(crate) fn usage_total_tokens(usage: &buckyos_api::AiUsage, api_type: ApiType) -> Option<u64> {
    if let Some(total) = usage.total_tokens {
        return Some(total);
    }
    match (usage.input_tokens, usage.output_tokens) {
        (Some(input), Some(output)) => Some(input.saturating_add(output)),
        (Some(input), None)
            if matches!(
                api_type,
                ApiType::EmbeddingText | ApiType::EmbeddingMultimodal
            ) =>
        {
            Some(input)
        }
        _ => None,
    }
}

/// Provider cost a model has before any call: its inventory price (or the
/// Provider's reported cost) currency, and whether that price is all zero.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ProviderCostInfo {
    pub currency: String,
    pub zero: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct InventoryModel {
    pub exact_model: String,
    pub provider_instance_name: String,
    pub provider_model_id: String,
    pub variant: Option<String>,
    pub model_driver_id: String,
    pub origin_model_id: String,
    pub provider_cost: Option<ProviderCostInfo>,
}

/// Exact models of every configured Provider instance (empty for a Provider
/// whose inventory has no model).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Inventory {
    pub providers: BTreeMap<String, BTreeMap<String, InventoryModel>>,
    /// Configured but disabled Provider instances (with no inventory).
    pub disabled: BTreeSet<String>,
}

impl Inventory {
    pub(crate) fn from_runtime(snapshot: &crate::runtime::RuntimeSnapshot) -> Self {
        let mut providers = snapshot
            .settings
            .providers
            .iter()
            .map(|provider| (provider.provider_instance_name.clone(), BTreeMap::new()))
            .collect::<BTreeMap<_, _>>();
        let costs = provider_costs(snapshot);
        for model in snapshot.models.model_views() {
            let provider_cost = costs
                .get(&(model.provider_instance_name.clone(), model.provider_model_id.clone()))
                .cloned();
            providers
                .entry(model.provider_instance_name.clone())
                .or_default()
                .insert(
                    model.exact_model.clone(),
                    InventoryModel {
                        exact_model: model.exact_model,
                        provider_instance_name: model.provider_instance_name,
                        provider_model_id: model.provider_model_id,
                        variant: model.variant,
                        model_driver_id: model.model_driver_id,
                        origin_model_id: model.origin_model_id,
                        provider_cost,
                    },
                );
        }
        let disabled = snapshot
            .settings
            .providers
            .iter()
            .filter(|provider| !provider.enabled)
            .map(|provider| provider.provider_instance_name.clone())
            .collect();
        Self {
            providers,
            disabled,
        }
    }

    /// From the Provider views (name, enabled) and the `models.list` value of
    /// an admin snapshot.
    pub(crate) fn from_views(
        provider_views: impl IntoIterator<Item = (String, bool)>,
        models: &Value,
    ) -> Self {
        let mut providers = BTreeMap::new();
        let mut disabled = BTreeSet::new();
        for (name, enabled) in provider_views {
            if !enabled {
                disabled.insert(name.clone());
            }
            providers.insert(name, BTreeMap::new());
        }
        let text = |model: &Value, key: &str| {
            model
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        for model in models
            .get("models")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let (provider, exact_model) = (
                text(model, "provider_instance_name"),
                text(model, "exact_model"),
            );
            if provider.is_empty() || exact_model.is_empty() {
                continue;
            }
            let provider_cost = model
                .get("provider_cost_currency")
                .and_then(Value::as_str)
                .map(|currency| ProviderCostInfo {
                    currency: currency.to_string(),
                    zero: model
                        .get("provider_cost_zero")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                });
            providers.entry(provider.clone()).or_insert_with(BTreeMap::new).insert(
                exact_model.clone(),
                InventoryModel {
                    exact_model,
                    provider_instance_name: provider,
                    provider_model_id: text(model, "provider_model_id"),
                    variant: model
                        .get("variant")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    model_driver_id: text(model, "model_driver_id"),
                    origin_model_id: text(model, "origin_model_id"),
                    provider_cost,
                },
            );
        }
        Self {
            providers,
            disabled,
        }
    }

    pub(crate) fn model(&self, exact_model: &str) -> Option<&InventoryModel> {
        self.providers
            .values()
            .find_map(|models| models.get(exact_model))
    }

    pub(crate) fn contains_model(&self, exact_model: &str) -> bool {
        self.model(exact_model).is_some()
    }

    pub(crate) fn models(&self) -> impl Iterator<Item = &InventoryModel> {
        self.providers.values().flat_map(BTreeMap::values)
    }
}

/// Provider cost currency of each (instance, provider model id): the
/// inventory price, else the currency the Provider reports costs in.
pub(crate) fn provider_costs(
    snapshot: &crate::runtime::RuntimeSnapshot,
) -> BTreeMap<(String, String), ProviderCostInfo> {
    let mut costs = BTreeMap::new();
    for provider in snapshot.providers.list() {
        let reported = provider
            .config
            .provider_rules_id
            .as_deref()
            .and_then(|id| snapshot.catalog.provider_rules(id))
            .and_then(|rules| rules.reported_cost.as_ref())
            .map(|policy| policy.currency.trim().to_ascii_uppercase())
            .filter(|currency| !currency.is_empty());
        for model in &provider.inventory.models {
            let info = match &model.pricing {
                Some(pricing) if !pricing.value.currency.trim().is_empty() => Some(ProviderCostInfo {
                    currency: pricing.value.currency.trim().to_ascii_uppercase(),
                    zero: pricing::pricing_is_zero(&pricing.value),
                }),
                _ => reported.clone().map(|currency| ProviderCostInfo {
                    currency,
                    zero: false,
                }),
            };
            if let Some(info) = info {
                costs.insert(
                    (
                        provider.config.provider_instance_name.clone(),
                        model.provider_model_id.clone(),
                    ),
                    info,
                );
            }
        }
    }
    costs
}

/// What a caller may see and use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Access {
    /// A system service or device acting for itself.
    Unrestricted,
    Restricted(RestrictedAccess),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RestrictedAccess {
    /// Visible Provider → visible exact models (possibly none).
    pub visible: BTreeMap<String, BTreeSet<String>>,
    /// Visible models without a price for this caller.
    pub unpriced: BTreeSet<String>,
}

impl Access {
    /// The caller may see the model (list it, read its usage).
    pub(crate) fn allows_model(&self, exact_model: &str) -> bool {
        match self {
            Self::Unrestricted => true,
            Self::Restricted(access) => access
                .visible
                .values()
                .any(|models| models.contains(exact_model)),
        }
    }

    /// The caller may call the model: visible and priced (or free).
    pub(crate) fn usable_model(&self, exact_model: &str) -> bool {
        self.allows_model(exact_model) && !self.is_unpriced(exact_model)
    }

    pub(crate) fn is_unpriced(&self, exact_model: &str) -> bool {
        match self {
            Self::Unrestricted => false,
            Self::Restricted(access) => access.unpriced.contains(exact_model),
        }
    }

    pub(crate) fn allows_provider(&self, provider_instance_name: &str) -> bool {
        match self {
            Self::Unrestricted => true,
            Self::Restricted(access) => access.visible.contains_key(provider_instance_name),
        }
    }

    pub(crate) fn is_restricted(&self) -> bool {
        matches!(self, Self::Restricted(_))
    }
}

/// The charged user of a call as policies see it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PolicyPrincipal {
    pub user_id: String,
    pub user_type: UserType,
    pub groups: Vec<String>,
}

impl PolicyPrincipal {
    /// `type:*` then `group:*`: the user's subjects at the time of a call.
    pub(crate) fn subjects(&self) -> Vec<String> {
        let mut subjects = vec![PolicySubject::for_user_type(self.user_type).to_string()];
        subjects.extend(
            self.groups
                .iter()
                .map(|group| PolicySubject::Group(group.clone()).to_string()),
        );
        subjects
    }
}

/// A limit that applies to one call, and the counter it is measured on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AppliedLimit {
    pub policy_id: String,
    pub source: PolicySource,
    pub subject: PolicySubject,
    pub counter: LimitCounter,
    pub resource: LimitResource,
    pub period: LimitPeriod,
    pub unit: LimitUnit,
    /// Micro-Credit or tokens.
    pub amount: i64,
    pub amount_text: String,
}

impl AppliedLimit {
    pub(crate) fn scope(&self) -> CounterScope {
        CounterScope {
            counter: self.counter.clone(),
            resource: self.resource.clone(),
            period: self.period,
        }
    }
}

/// One counter row: who, on which resource, over which period.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct CounterScope {
    pub counter: LimitCounter,
    pub resource: LimitResource,
    pub period: LimitPeriod,
}

const KEY_SEPARATOR: char = '\u{1f}';

impl CounterScope {
    pub(crate) fn key(&self) -> String {
        let (counter_type, user_id, app_id) = match &self.counter {
            LimitCounter::User { user_id } => ("user", user_id.as_str(), ""),
            LimitCounter::App { app_id } => ("app", "", app_id.as_str()),
            LimitCounter::UserApp { user_id, app_id } => ("user_app", user_id.as_str(), app_id.as_str()),
        };
        let (resource_type, resource_id) = match &self.resource {
            LimitResource::All => ("all", ""),
            LimitResource::Provider {
                provider_instance_name,
            } => ("provider", provider_instance_name.as_str()),
            LimitResource::Model { exact_model } => ("model", exact_model.as_str()),
            LimitResource::Hidden => ("hidden", ""),
        };
        [
            counter_type,
            user_id,
            app_id,
            resource_type,
            resource_id,
            self.period.as_str(),
        ]
        .join(&KEY_SEPARATOR.to_string())
    }
}

/// Visibility, coefficients and limits of one exact model for one caller.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ModelDecision {
    pub visible: bool,
    pub visible_from: Vec<String>,
    pub provider_coefficient: f64,
    pub model_coefficient: f64,
    pub coefficient_from: Vec<String>,
    pub limits: Vec<AppliedLimit>,
}

impl ModelDecision {
    pub(crate) fn coefficient(&self) -> f64 {
        self.provider_coefficient * self.model_coefficient
    }
}

/// What one policy says about the visibility of a Provider (and model).
fn layer_visibility(body: &PolicyBody, provider: &str, exact_model: Option<&str>) -> Option<bool> {
    if let Some(visible) = exact_model
        .and_then(|model| body.models.get(model))
        .and_then(|model| model.visible)
    {
        return Some(visible);
    }
    if let Some(visibility) = body.providers.get(provider).and_then(|policy| policy.visible) {
        return Some(match visibility {
            ProviderVisibility::AllModels => true,
            ProviderVisibility::Hidden => false,
            ProviderVisibility::SelectedModels => exact_model.is_none(),
        });
    }
    (body.all_providers == Some(true)).then_some(true)
}

fn layer_provider_coefficient(body: &PolicyBody, provider: &str) -> Option<f64> {
    body.providers
        .get(provider)
        .and_then(|policy| policy.price_coefficient)
        .or(body.price_coefficient)
}

fn layer_model_coefficient(body: &PolicyBody, exact_model: &str) -> Option<f64> {
    body.models
        .get(exact_model)
        .and_then(|policy| policy.price_coefficient)
}

/// Limits of one policy on a call of `exact_model` in `provider`.
fn layer_limits<'a>(
    body: &'a PolicyBody,
    provider: &str,
    exact_model: &str,
) -> Vec<(LimitResource, &'a PolicyLimit)> {
    let mut limits = body
        .limits
        .iter()
        .map(|limit| (LimitResource::All, limit))
        .collect::<Vec<_>>();
    if let Some(policy) = body.providers.get(provider) {
        limits.extend(policy.limits.iter().map(|limit| {
            (
                LimitResource::Provider {
                    provider_instance_name: provider.to_string(),
                },
                limit,
            )
        }));
    }
    if let Some(policy) = body.models.get(exact_model) {
        limits.extend(policy.limits.iter().map(|limit| {
            (
                LimitResource::Model {
                    exact_model: exact_model.to_string(),
                },
                limit,
            )
        }));
    }
    limits
}

/// The policies that apply to one caller, by layer.
pub(crate) struct Evaluator<'a> {
    user_id: String,
    app_id: Option<String>,
    /// User side, least specific first: `type:*`, the user's groups, `user:*`.
    layers: [Vec<&'a AiccPolicy>; 3],
    /// App side: `app:*`, then the `user_app:*` policies of both sources.
    app_side: Vec<&'a AiccPolicy>,
}

impl<'a> Evaluator<'a> {
    pub(crate) fn new(
        policies: &'a [AiccPolicy],
        principal: &PolicyPrincipal,
        app_id: Option<&str>,
    ) -> Self {
        let type_subject = PolicySubject::for_user_type(principal.user_type);
        let user_subject = PolicySubject::User(principal.user_id.clone());
        let app_id = app_id.filter(|app| !app.is_empty()).map(str::to_owned);
        let mut layers: [Vec<&AiccPolicy>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        let mut app_side = Vec::new();
        for policy in policies.iter().filter(|policy| policy.enabled) {
            match &policy.subject {
                subject if *subject == type_subject => layers[0].push(policy),
                PolicySubject::Group(group) if principal.groups.contains(group) => {
                    layers[1].push(policy)
                }
                subject if *subject == user_subject => layers[2].push(policy),
                PolicySubject::App(app) if Some(app) == app_id.as_ref() => app_side.push(policy),
                PolicySubject::UserApp {
                    user_id,
                    app_id: app,
                } if user_id == &principal.user_id && Some(app) == app_id.as_ref() => {
                    app_side.push(policy)
                }
                _ => {}
            }
        }
        app_side.sort_by_key(|policy| match policy.subject {
            PolicySubject::App(_) => 0,
            _ if policy.source == PolicySource::Admin => 1,
            _ => 2,
        });
        Self {
            user_id: principal.user_id.clone(),
            app_id,
            layers,
            app_side,
        }
    }

    /// User side visibility: the most specific layer with an opinion; in the
    /// group layer one group allowing it is enough.
    fn user_visibility(&self, provider: &str, exact_model: Option<&str>) -> (bool, Vec<String>) {
        for layer in self.layers.iter().rev() {
            let opinions = layer
                .iter()
                .filter_map(|policy| {
                    layer_visibility(&policy.body, provider, exact_model)
                        .map(|visible| (visible, policy.subject.to_string()))
                })
                .collect::<Vec<_>>();
            if opinions.is_empty() {
                continue;
            }
            let allowing = opinions
                .iter()
                .filter(|(visible, _)| *visible)
                .map(|(_, subject)| subject.clone())
                .collect::<Vec<_>>();
            return if allowing.is_empty() {
                (false, opinions.into_iter().map(|(_, subject)| subject).collect())
            } else {
                (true, allowing)
            };
        }
        (false, Vec::new())
    }

    fn app_hides(&self, provider: &str, exact_model: Option<&str>) -> Option<String> {
        self.app_side
            .iter()
            .find(|policy| layer_visibility(&policy.body, provider, exact_model) == Some(false))
            .map(|policy| policy.subject.to_string())
    }

    pub(crate) fn provider_visible(&self, provider: &str) -> bool {
        self.user_visibility(provider, None).0 && self.app_hides(provider, None).is_none()
    }

    /// The most specific layer's coefficient; the smallest in the group layer.
    fn coefficient(&self, value: impl Fn(&PolicyBody) -> Option<f64>) -> (f64, Option<String>) {
        for layer in self.layers.iter().rev() {
            let best = layer
                .iter()
                .filter_map(|policy| value(&policy.body).map(|coefficient| (coefficient, policy)))
                .min_by(|left, right| left.0.total_cmp(&right.0));
            if let Some((coefficient, policy)) = best {
                return (coefficient, Some(policy.subject.to_string()));
            }
        }
        (1.0, None)
    }

    pub(crate) fn model(&self, provider: &str, exact_model: &str) -> ModelDecision {
        let (mut visible, mut visible_from) = self.user_visibility(provider, Some(exact_model));
        if visible {
            if let Some(subject) = self.app_hides(provider, Some(exact_model)) {
                visible = false;
                visible_from = vec![subject];
            }
        }
        let (provider_coefficient, provider_from) =
            self.coefficient(|body| layer_provider_coefficient(body, provider));
        let (model_coefficient, model_from) =
            self.coefficient(|body| layer_model_coefficient(body, exact_model));
        let coefficient_from = provider_from.into_iter().chain(model_from).collect();
        ModelDecision {
            visible,
            visible_from,
            provider_coefficient,
            model_coefficient,
            coefficient_from,
            limits: self.limits(provider, exact_model),
        }
    }

    /// User side limits by (resource, period, unit): the most specific layer
    /// wins, the largest within the group layer; App side limits all apply.
    fn limits(&self, provider: &str, exact_model: &str) -> Vec<AppliedLimit> {
        let user_counter = LimitCounter::User {
            user_id: self.user_id.clone(),
        };
        let mut chosen: BTreeMap<(LimitResource, LimitPeriod, LimitUnit), AppliedLimit> =
            BTreeMap::new();
        for layer in &self.layers {
            let mut layer_best: BTreeMap<(LimitResource, LimitPeriod, LimitUnit), AppliedLimit> =
                BTreeMap::new();
            for policy in layer {
                for (resource, limit) in layer_limits(&policy.body, provider, exact_model) {
                    let Some(amount) = limit.amount_value() else {
                        continue;
                    };
                    let key = (resource.clone(), limit.period, limit.unit);
                    if layer_best.get(&key).is_some_and(|best| best.amount >= amount) {
                        continue;
                    }
                    layer_best.insert(
                        key,
                        AppliedLimit {
                            policy_id: policy.policy_id.clone(),
                            source: policy.source,
                            subject: policy.subject.clone(),
                            counter: user_counter.clone(),
                            resource,
                            period: limit.period,
                            unit: limit.unit,
                            amount,
                            amount_text: limit.amount.clone(),
                        },
                    );
                }
            }
            chosen.extend(layer_best);
        }
        let mut limits = chosen.into_values().collect::<Vec<_>>();
        for policy in &self.app_side {
            let counter = match &policy.subject {
                PolicySubject::App(app_id) => LimitCounter::App {
                    app_id: app_id.clone(),
                },
                _ => LimitCounter::UserApp {
                    user_id: self.user_id.clone(),
                    app_id: self.app_id.clone().unwrap_or_default(),
                },
            };
            for (resource, limit) in layer_limits(&policy.body, provider, exact_model) {
                let Some(amount) = limit.amount_value() else {
                    continue;
                };
                limits.push(AppliedLimit {
                    policy_id: policy.policy_id.clone(),
                    source: policy.source,
                    subject: policy.subject.clone(),
                    counter: counter.clone(),
                    resource,
                    period: limit.period,
                    unit: limit.unit,
                    amount,
                    amount_text: limit.amount.clone(),
                });
            }
        }
        limits
    }

    /// Visibility and pricing of the whole inventory for this caller.
    pub(crate) fn access(&self, inventory: &Inventory, prices: &PriceList) -> RestrictedAccess {
        let mut access = RestrictedAccess::default();
        for (provider, models) in &inventory.providers {
            let mut visible = BTreeSet::new();
            for model in models.values() {
                let decision = self.model(provider, &model.exact_model);
                if !decision.visible {
                    continue;
                }
                if prices.state(model, decision.coefficient()) == CreditPriceState::Unpriced {
                    access.unpriced.insert(model.exact_model.clone());
                }
                visible.insert(model.exact_model.clone());
            }
            if !visible.is_empty() || self.provider_visible(provider) {
                access.visible.insert(provider.clone(), visible);
            }
        }
        access
    }
}

/// Every limit an evaluator would apply to some model of the inventory, once
/// per (policy, counter, resource, period, unit); limits on resources the
/// inventory no longer has are kept with their own resource.
pub(crate) fn all_limits(evaluator: &Evaluator<'_>, inventory: &Inventory) -> Vec<AppliedLimit> {
    let mut seen = BTreeSet::new();
    let mut limits = Vec::new();
    let mut push = |limit: AppliedLimit| {
        let key = (
            limit.policy_id.clone(),
            limit.counter.clone(),
            limit.resource.clone(),
            limit.period,
            limit.unit,
        );
        if seen.insert(key) {
            limits.push(limit);
        }
    };
    for model in inventory.models() {
        for limit in evaluator.limits(&model.provider_instance_name, &model.exact_model) {
            push(limit);
        }
    }
    for limit in evaluator.limits("", "") {
        push(limit);
    }
    let policies = evaluator
        .layers
        .iter()
        .flatten()
        .chain(evaluator.app_side.iter());
    for policy in policies {
        for provider in policy.body.providers.keys() {
            for limit in evaluator.limits(provider, "") {
                push(limit);
            }
        }
        for model in policy.body.models.keys() {
            let provider = model.rsplit_once('@').map(|(_, provider)| provider).unwrap_or_default();
            for limit in evaluator.limits(provider, model) {
                push(limit);
            }
        }
    }
    limits
}

/// Field rules of a policy that do not depend on the directory (§3.1).
pub(crate) fn validate_body(
    source: PolicySource,
    subject: &PolicySubject,
    body: &PolicyBody,
) -> Result<(), &'static str> {
    let coefficient_ok = |value: Option<f64>| value.is_none_or(|value| value.is_finite() && value >= 0.0 && value <= 1_000_000.0);
    let limits_ok = |limits: &[PolicyLimit]| {
        let mut keys = BTreeSet::new();
        limits.iter().all(|limit| {
            limit.amount_value().is_some() && keys.insert((limit.period, limit.unit))
        })
    };
    if !coefficient_ok(body.price_coefficient) || !limits_ok(&body.limits) {
        return Err("a price coefficient must be a non-negative number and each limit a non-negative amount, one per period and unit");
    }
    for (provider, policy) in &body.providers {
        if provider.trim().is_empty() || !coefficient_ok(policy.price_coefficient) || !limits_ok(&policy.limits) {
            return Err("a Provider entry has an invalid name, coefficient or limit");
        }
    }
    for (model, policy) in &body.models {
        let Some((_, provider)) = model.rsplit_once('@') else {
            return Err("a model entry must be an exact model (model@provider)");
        };
        if provider.is_empty()
            || !coefficient_ok(policy.price_coefficient)
            || !limits_ok(&policy.limits)
        {
            return Err("a model entry has an invalid coefficient or limit");
        }
        if policy.visible == Some(true)
            && body.providers.get(provider).and_then(|policy| policy.visible)
                == Some(ProviderVisibility::Hidden)
        {
            return Err("a model is visible while its Provider is hidden in the same policy");
        }
    }
    let has_coefficient = body.price_coefficient.is_some()
        || body.providers.values().any(|policy| policy.price_coefficient.is_some())
        || body.models.values().any(|policy| policy.price_coefficient.is_some());
    if subject.is_app_side() && has_coefficient {
        return Err("an App policy cannot set a price coefficient");
    }
    match source {
        PolicySource::Admin => Ok(()),
        PolicySource::User => {
            if !matches!(subject, PolicySubject::UserApp { .. }) {
                return Err("a personal policy applies to one of your own Apps");
            }
            let selects = |model: &str| {
                model.rsplit_once('@').is_some_and(|(_, provider)| {
                    body.providers.get(provider).and_then(|policy| policy.visible)
                        == Some(ProviderVisibility::SelectedModels)
                })
            };
            let widens = body.all_providers.is_some()
                || body
                    .providers
                    .values()
                    .any(|policy| policy.visible == Some(ProviderVisibility::AllModels))
                || body
                    .models
                    .iter()
                    .any(|(model, policy)| policy.visible == Some(true) && !selects(model));
            if widens {
                return Err("a personal policy can only hide resources and add limits");
            }
            Ok(())
        }
    }
}

/// Providers and exact models a policy names that are not in the inventory.
pub(crate) fn unavailable_resources(body: &PolicyBody, inventory: &Inventory) -> Vec<String> {
    body.providers
        .keys()
        .filter(|provider| !inventory.providers.contains_key(*provider))
        .chain(body.models.keys().filter(|model| !inventory.contains_model(model)))
        .cloned()
        .collect()
}

/// Everything settlement needs about an admitted attempt, pinned when it was
/// admitted so later policy or price changes do not affect its charge.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct ChargePlan {
    pub attempt_id: String,
    pub task_id: String,
    /// Usage event of the task, linked from the charge of a reported usage.
    pub usage_event_id: String,
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_id: Option<String>,
    pub provider_instance_name: String,
    pub exact_model: String,
    pub admitted_at_ms: i64,
    /// `type:*` / `group:*` of the user when the attempt was admitted.
    pub subjects: Vec<String>,
    pub price: PinnedCreditPrice,
    /// Counters to add the charge to (only scopes some limit refers to).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub counters: Vec<CounterScope>,
    pub token_metered: bool,
}

#[cfg(test)]
mod tests;
