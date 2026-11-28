//! AICC user policy, Credit price list and Credit accounts
//! (doc/aicc/aicc_user_policy.md).
//!
//! A policy is what an administrator (or a user, for their own Apps) grants
//! and limits: Provider / Model visibility, price coefficients and time
//! window limits. Credit is the Zone's unit of account for AI use; prices
//! come from the price list, a usage is charged after it happened and a
//! balance may go negative. The delegated attribution a system service sends
//! when it calls on a user's behalf is also defined here.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

pub mod policy_methods {
    pub const POLICY_LIST: &str = "policy.list";
    pub const POLICY_GET: &str = "policy.get";
    pub const POLICY_UPSERT: &str = "policy.upsert";
    pub const POLICY_DELETE: &str = "policy.delete";
    pub const POLICY_EFFECTIVE: &str = "policy.effective";
    pub const POLICY_STATUS: &str = "policy.status";
    pub const PRICE_GET: &str = "price.get";
    pub const PRICE_UPDATE: &str = "price.update";
    pub const CREDIT_ACCOUNT_GET: &str = "credit.account.get";
    pub const CREDIT_ACCOUNT_LIST: &str = "credit.account.list";
    pub const CREDIT_LEDGER_LIST: &str = "credit.ledger.list";
    pub const CREDIT_RECHARGE: &str = "credit.recharge";
    pub const CREDIT_RECHARGE_GET: &str = "credit.recharge.get";
    pub const CREDIT_ADJUST: &str = "credit.adjust";

    pub fn is_policy_method(method: &str) -> bool {
        method.starts_with("policy.") || method.starts_with("price.") || method.starts_with("credit.")
    }
}

/// Reserved top-level request parameter of a delegated call.
pub const AICC_ON_BEHALF_OF_PARAM: &str = "on_behalf_of";
/// RBAC resource a system service needs (`write`) to send `on_behalf_of`.
pub const AICC_ON_BEHALF_OF_RESOURCE: &str = "obj://aicc/on_behalf_of";
/// RBAC resource (`write`) of `credit.recharge`, `credit.recharge.get` and
/// `credit.adjust`: the admin role by default, a payment gateway by a grant
/// appended to `system/rbac/policy`.
pub const AICC_CREDIT_RECHARGE_RESOURCE: &str = "obj://aicc/credit/recharge";
/// Share of a limit at which it is shown as near its limit.
pub const QUOTA_NEAR_LIMIT_PERCENT: u64 = 80;
/// Credit is stored in millionths.
pub const CREDIT_MICRO_PER_UNIT: i64 = 1_000_000;

/// `12.5` → 12_500_000. At most six decimals, optional sign; `None` for
/// anything else (exponents, spaces, overflow).
pub fn parse_credit_micro(value: &str) -> Option<i64> {
    let (negative, digits) = match value.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, value.strip_prefix('+').unwrap_or(value)),
    };
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
    if whole.is_empty() && fraction.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.len() > 6
        || (digits.contains('.') && fraction.is_empty())
    {
        return None;
    }
    let whole: i64 = if whole.is_empty() { 0 } else { whole.parse().ok()? };
    let fraction: i64 = format!("{fraction:0<6}").parse().ok()?;
    let micro = whole.checked_mul(CREDIT_MICRO_PER_UNIT)?.checked_add(fraction)?;
    Some(if negative { -micro } else { micro })
}

/// 12_500_000 → `"12.5"`; whole numbers have no decimal point.
pub fn format_credit_micro(micro: i64) -> String {
    let sign = if micro < 0 { "-" } else { "" };
    let abs = micro.unsigned_abs();
    let whole = abs / CREDIT_MICRO_PER_UNIT as u64;
    let fraction = abs % CREDIT_MICRO_PER_UNIT as u64;
    if fraction == 0 {
        format!("{sign}{whole}")
    } else {
        let fraction = format!("{fraction:06}");
        format!("{sign}{whole}.{}", fraction.trim_end_matches('0'))
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AiccAttributionMode {
    /// The call is the attributed user's own (direct, App or Agent).
    Direct,
    /// A workspace wish run using the workspace owner's AICC.
    WorkspaceOwner,
    /// A workspace wish run using the AICC of the user who started it.
    CurrentUser,
}

/// Business run a delegated call belongs to, for Usage and Trace.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AiccRunRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
}

/// What a system service asserts when it calls AICC for a user. AICC accepts
/// it only from a system principal granted [`AICC_ON_BEHALF_OF_RESOURCE`];
/// the business App is that service, never a value from the request.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AiccOnBehalfOf {
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_id: Option<String>,
    pub mode: AiccAttributionMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_ref: Option<AiccRunRef>,
}

/// Who a call is charged to, as AICC resolved it from the trusted identity.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AiccAttribution {
    /// The charged user (an account of this Zone) or, for an undelegated
    /// system or device principal, that principal.
    pub user_id: String,
    /// Trusted business App: an AppId, an AgentId, or `system:<service>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    /// Authenticated user who triggered the call; traceability only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_id: Option<String>,
    /// Raw caller identity (`app:<instance>` / `system:<service>`).
    pub caller: String,
    pub mode: AiccAttributionMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_ref: Option<AiccRunRef>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum PolicySource {
    Admin,
    User,
}

impl PolicySource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::User => "user",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "admin" => Some(Self::Admin),
            "user" => Some(Self::User),
            _ => None,
        }
    }
}

/// Who a policy applies to. UserType groups, system user groups and users
/// are the user side (more specific overrides); Apps and user + App
/// combinations are the App side (only narrows).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PolicySubject {
    /// `type:admin` / `type:user` / `type:limited`.
    UserType(String),
    /// `group:<group_id>`, a system user group.
    Group(String),
    /// `user:<user_id>`.
    User(String),
    /// `app:<app_id>`: every user's calls through the App (or Agent).
    App(String),
    /// `user_app:<user_id>/<app_id>`.
    UserApp { user_id: String, app_id: String },
}

pub const POLICY_USER_TYPES: [&str; 3] = ["admin", "user", "limited"];

impl PolicySubject {
    pub fn parse(value: &str) -> Option<Self> {
        let (kind, rest) = value.split_once(':')?;
        let valid = |id: &str| !id.trim().is_empty() && id.len() <= 256 && !id.contains('\u{1f}');
        match kind {
            "type" => POLICY_USER_TYPES
                .contains(&rest)
                .then(|| Self::UserType(rest.to_string())),
            "group" => valid(rest).then(|| Self::Group(rest.to_string())),
            "user" => (valid(rest) && !rest.contains('/')).then(|| Self::User(rest.to_string())),
            "app" => valid(rest).then(|| Self::App(rest.to_string())),
            "user_app" => {
                let (user_id, app_id) = rest.split_once('/')?;
                (valid(user_id) && valid(app_id)).then(|| Self::UserApp {
                    user_id: user_id.to_string(),
                    app_id: app_id.to_string(),
                })
            }
            _ => None,
        }
    }

    pub fn is_app_side(&self) -> bool {
        matches!(self, Self::App(_) | Self::UserApp { .. })
    }

    pub fn user_id(&self) -> Option<&str> {
        match self {
            Self::User(user_id) | Self::UserApp { user_id, .. } => Some(user_id),
            _ => None,
        }
    }

    pub fn app_id(&self) -> Option<&str> {
        match self {
            Self::App(app_id) | Self::UserApp { app_id, .. } => Some(app_id),
            _ => None,
        }
    }

    /// The UserType group an account type belongs to (Root is an admin).
    pub fn for_user_type(user_type: crate::UserType) -> Self {
        Self::UserType(
            match user_type {
                crate::UserType::Root | crate::UserType::Admin => "admin",
                crate::UserType::User => "user",
                crate::UserType::Limited => "limited",
            }
            .to_string(),
        )
    }
}

impl fmt::Display for PolicySubject {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UserType(name) => write!(formatter, "type:{name}"),
            Self::Group(group) => write!(formatter, "group:{group}"),
            Self::User(user_id) => write!(formatter, "user:{user_id}"),
            Self::App(app_id) => write!(formatter, "app:{app_id}"),
            Self::UserApp { user_id, app_id } => write!(formatter, "user_app:{user_id}/{app_id}"),
        }
    }
}

impl Serialize for PolicySubject {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for PolicySubject {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "`{value}` is not a policy subject (type:<admin|user|limited>, group:<id>, user:<id>, app:<id>, user_app:<user>/<app>)"
            ))
        })
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LimitPeriod {
    #[serde(rename = "5h")]
    FiveHours,
    #[serde(rename = "24h")]
    Day,
    #[serde(rename = "7d")]
    Week,
}

const HOUR_MS: i64 = 60 * 60 * 1000;

impl LimitPeriod {
    pub const ALL: [LimitPeriod; 3] = [LimitPeriod::FiveHours, LimitPeriod::Day, LimitPeriod::Week];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::FiveHours => "5h",
            Self::Day => "24h",
            Self::Week => "7d",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|period| period.as_str() == value)
    }

    /// A window starts at the first use after the previous one ended.
    pub fn length_ms(self) -> i64 {
        match self {
            Self::FiveHours => 5 * HOUR_MS,
            Self::Day => 24 * HOUR_MS,
            Self::Week => 7 * 24 * HOUR_MS,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum LimitUnit {
    Credit,
    /// Counts only token-metered calls (LLM, decision, embeddings).
    Token,
}

impl LimitUnit {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Credit => "credit",
            Self::Token => "token",
        }
    }
}

/// One time window limit. `amount` is a decimal string (Credit, up to six
/// decimals) or a whole number of tokens; `0` forbids use.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PolicyLimit {
    pub period: LimitPeriod,
    pub unit: LimitUnit,
    pub amount: String,
}

impl PolicyLimit {
    /// The amount in the counter's unit (micro-Credit or tokens).
    pub fn amount_value(&self) -> Option<i64> {
        match self.unit {
            LimitUnit::Credit => parse_credit_micro(&self.amount).filter(|amount| *amount >= 0),
            LimitUnit::Token => self.amount.parse::<i64>().ok().filter(|amount| *amount >= 0),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ProviderVisibility {
    /// Every model of the Provider's inventory, present and future.
    AllModels,
    /// Only the models this policy marks `visible: true`.
    SelectedModels,
    Hidden,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProviderPolicy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<ProviderVisibility>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_coefficient: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limits: Vec<PolicyLimit>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ModelPolicy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_coefficient: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limits: Vec<PolicyLimit>,
}

/// What one policy says. Every field is optional: an absent field has no
/// opinion and leaves the decision to another layer.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PolicyBody {
    /// Every Provider, present and future, with all of its models.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all_providers: Option<bool>,
    /// This layer's coefficient for every Provider (`0` is free).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_coefficient: Option<f64>,
    /// Limits on every resource.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limits: Vec<PolicyLimit>,
    /// Provider instance → its visibility, coefficient and limits.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub providers: BTreeMap<String, ProviderPolicy>,
    /// Exact model (`provider_model_id[:variant]@instance`) → its settings.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, ModelPolicy>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AiccPolicy {
    pub policy_id: String,
    pub source: PolicySource,
    pub subject: PolicySubject,
    pub enabled: bool,
    pub body: PolicyBody,
    pub version: u64,
    pub created_by: String,
    pub created_at_ms: i64,
    pub updated_by: String,
    pub updated_at_ms: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AiccPolicyView {
    pub policy: AiccPolicy,
    /// The viewer may change this policy.
    pub editable: bool,
    /// Providers or exact models the body names that are not in the inventory.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unavailable_resources: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PolicyDraft {
    /// Defaults to `admin` for an administrator and `user` otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PolicySource>,
    pub subject: PolicySubject,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub body: PolicyBody,
}

fn default_true() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PolicyListScope {
    /// Policies that apply to the caller, and the caller's own.
    #[default]
    Mine,
    /// Every policy of the Zone (admin only).
    Manage,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PolicyListRequest {
    #[serde(default)]
    pub scope: PolicyListScope,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PolicyListResponse {
    pub policies: Vec<AiccPolicyView>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PolicyGetRequest {
    pub policy_id: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PolicyGetResponse {
    pub policy: AiccPolicyView,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audit: Vec<PolicyAuditRecord>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PolicyUpsertRequest {
    pub draft: PolicyDraft,
    /// Version of the stored policy of the same source and subject; absent
    /// when creating one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_version: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PolicyDeleteRequest {
    pub policy_id: String,
    pub expected_version: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PolicyDeleteResponse {
    pub ok: bool,
    pub policy_id: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PolicyAuditRecord {
    pub audit_id: String,
    pub target_type: String,
    pub target_id: String,
    pub action: String,
    pub operator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<Value>,
    pub created_at_ms: i64,
}

/// Counter a limit is measured on: the charged user, an App shared by all of
/// its users, or one user's calls through one App.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum LimitCounter {
    User { user_id: String },
    App { app_id: String },
    UserApp { user_id: String, app_id: String },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum LimitResource {
    All,
    Provider { provider_instance_name: String },
    /// `provider_model_id[:variant]@provider_instance_name`.
    Model { exact_model: String },
    /// A resource the viewer may not see; identity withheld.
    Hidden,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LimitStatus {
    Normal,
    NearLimit,
    Exhausted,
    /// The amount is zero: use is forbidden.
    Forbidden,
}

/// A limit that applies, with the state of its current window.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EffectiveLimit {
    pub policy_id: String,
    pub source: PolicySource,
    pub subject: PolicySubject,
    pub counter: LimitCounter,
    pub resource: LimitResource,
    pub period: LimitPeriod,
    pub unit: LimitUnit,
    pub amount: String,
    /// Used in the current window, in the limit's unit.
    pub used: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_start_ms: Option<i64>,
    /// End of the current window; `null` when no window is open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_at_ms: Option<i64>,
    pub status: LimitStatus,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CreditPriceSource {
    /// An `exact:` entry of the price list.
    Exact,
    /// A `model:` entry (Provider independent model identity).
    Model,
    /// Provider cost × the administrator's Credit rate of its currency.
    ProviderCost,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CreditPriceState {
    /// Explicitly free: a zero price or a zero coefficient.
    Free,
    /// Calls are charged.
    Priced,
    /// No price and a non-zero coefficient: the model cannot be used.
    Unpriced,
}

/// What a model costs one user (through one App).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ModelCreditPrice {
    pub state: CreditPriceState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<CreditPriceSource>,
    /// The price list entry (`model:…` / `exact:…`) the price comes from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_key: Option<String>,
    /// Credit pricing of the entry, token rates per million tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<Value>,
    /// Currency and Credit per unit of a Provider cost price.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_cost_currency: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_rate: Option<f64>,
    pub provider_coefficient: f64,
    pub model_coefficient: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EffectiveModel {
    pub exact_model: String,
    pub provider_instance_name: String,
    /// Policies (subjects) the visibility comes from.
    pub visible_from: Vec<String>,
    /// The policies of the provider and model coefficients.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub coefficient_from: Vec<String>,
    pub price: ModelCreditPrice,
    /// Visible and priced (or free).
    pub usable: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PolicyEffectiveRequest {
    /// Defaults to the caller; another user needs an administrator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    /// Calls through this App (App side policies apply).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    /// Unsaved policy to compare against the stored ones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<PolicyDraft>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PolicyEffectiveResponse {
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    pub balance: String,
    /// Visible models (usable or not).
    pub models: Vec<EffectiveModel>,
    pub limits: Vec<EffectiveLimit>,
    /// Visible models without a price for this user.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unpriced_models: Vec<String>,
    /// Exact models the draft opens to this user.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub added: Vec<String>,
    /// Exact models the draft takes from this user (hidden or unpriced).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<String>,
    /// Accounts the draft's subject covers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affected_users: Vec<String>,
    /// Exact models the caller would no longer be able to use with the draft.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub caller_loses: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PolicyStatusRequest {
    /// Only the limits that apply to calls through this App.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PolicyStatusResponse {
    pub user_id: String,
    pub balance: String,
    pub limits: Vec<EffectiveLimit>,
    pub near_limit_count: u64,
    pub exhausted_count: u64,
}

/// One price list entry. `entry_key` is `model:<model_driver_id>:<origin_model_id>`
/// or `exact:<provider_model_id>[:variant]@<instance>`; `pricing` has the
/// shape of a Provider price with `currency: "CREDIT"` and token rates per
/// million tokens.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PriceEntry {
    pub entry_key: String,
    pub pricing: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default)]
    pub updated_by: String,
    #[serde(default)]
    pub updated_at_ms: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PriceEntryInput {
    pub entry_key: String,
    pub pricing: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PriceGetRequest {}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PriceGetResponse {
    pub revision: u64,
    pub entries: Vec<PriceEntry>,
    /// Currency → Credit per unit of Provider cost (administrators only).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub credit_rates: BTreeMap<String, f64>,
    /// Inventory models without any price (administrators only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unpriced_models: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PriceUpdateRequest {
    pub expected_revision: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub upserts: Vec<PriceEntryInput>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deletes: Vec<String>,
    /// Replaces every rate when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_rates: Option<BTreeMap<String, f64>>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CreditEntryKind {
    Recharge,
    Adjust,
}

/// A recharge or an adjustment.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CreditEntry {
    pub entry_id: String,
    pub user_id: String,
    pub kind: CreditEntryKind,
    pub amount: String,
    pub balance_after: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recharge_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_ref: Option<Value>,
    pub operator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CreditChargeStatus {
    Charged,
    /// Explicitly free: a zero price or a zero coefficient.
    Free,
    /// The usage has a dimension the price does not cover, or the Provider
    /// reported no cost for a Provider cost price: nothing charged.
    Unpriced,
    /// The outcome is unknown; charged once the real usage arrives.
    UnknownUsage,
}

impl CreditChargeStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Charged => "charged",
            Self::Free => "free",
            Self::Unpriced => "unpriced",
            Self::UnknownUsage => "unknown_usage",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "charged" => Some(Self::Charged),
            "free" => Some(Self::Free),
            "unpriced" => Some(Self::Unpriced),
            "unknown_usage" => Some(Self::UnknownUsage),
            _ => None,
        }
    }
}

/// The charge of one upstream attempt.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CreditCharge {
    pub attempt_id: String,
    pub task_id: String,
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_id: Option<String>,
    pub provider_instance_name: String,
    pub exact_model: String,
    pub admitted_at_ms: i64,
    pub tokens: u64,
    pub amount: String,
    pub status: CreditChargeStatus,
    pub settled_at_ms: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CreditAccount {
    pub user_id: String,
    pub balance: String,
    pub updated_at_ms: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CreditAccountGetRequest {
    /// Defaults to the caller; another user needs an administrator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CreditAccountListRequest {
    /// Only members of this system user group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CreditAccountListResponse {
    /// Every account of the Zone; users never charged or recharged have 0.
    pub accounts: Vec<CreditAccount>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CreditLedgerListRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_time_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_time_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// One line of a user's Credit ledger, newest first.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CreditLedgerItem {
    Entry(CreditEntry),
    Charge(CreditCharge),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CreditLedgerListResponse {
    pub user_id: String,
    pub balance: String,
    pub items: Vec<CreditLedgerItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CreditRechargeRequest {
    pub user_id: String,
    /// Caller-generated idempotency key (a payment order id): the same id
    /// is credited once.
    pub recharge_id: String,
    /// Positive decimal string.
    pub amount: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Free JSON a payment gateway keeps with the entry; never interpreted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_ref: Option<Value>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CreditAdjustRequest {
    pub user_id: String,
    /// Idempotency key of the adjustment.
    pub adjust_id: String,
    /// Signed decimal string, not zero.
    pub amount: String,
    pub note: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_ref: Option<Value>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CreditEntryResponse {
    pub entry: CreditEntry,
    pub account: CreditAccount,
    /// The id had already been applied; nothing new was added.
    pub replayed: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CreditRechargeGetRequest {
    pub recharge_id: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CreditRechargeGetResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<CreditEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<CreditAccount>,
}

/// One limit that refused a call.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuotaBlockingLimit {
    pub policy_id: String,
    pub source: PolicySource,
    pub subject: PolicySubject,
    pub resource: LimitResource,
    pub period: LimitPeriod,
    pub unit: LimitUnit,
    pub used: String,
    pub amount: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_at_ms: Option<i64>,
}

/// `details` of `quota_exhausted`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuotaDenialDetails {
    pub provider_instance_name: String,
    pub exact_model: String,
    pub blocking: Vec<QuotaBlockingLimit>,
    /// When every blocking limit recovers; `null` when one of them is zero.
    pub reset_at_ms: Option<i64>,
}

/// `details` of `credit_insufficient`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CreditInsufficientDetails {
    pub balance: String,
    pub provider_instance_name: String,
    pub exact_model: String,
}

/// `details` of `resource_not_permitted` for a visible model without a price.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceUnpricedDetails {
    /// Always `unpriced`.
    pub reason: String,
    pub provider_instance_name: String,
    pub exact_model: String,
}

/// `details` of `quota_unavailable`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuotaUnavailableDetails {
    pub scope: String,
    pub occurred_at_ms: i64,
}

/// `details` of `attribution_unavailable`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AttributionUnavailableDetails {
    pub mode: AiccAttributionMode,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AiccListView {
    /// Resources the caller may see.
    #[default]
    Use,
    /// The complete inventory (admin only), each model marked with whether
    /// the caller may use it.
    Manage,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credit_amounts_round_trip_with_six_decimals() {
        assert_eq!(parse_credit_micro("12.5"), Some(12_500_000));
        assert_eq!(parse_credit_micro("-2.5"), Some(-2_500_000));
        assert_eq!(parse_credit_micro("0.000001"), Some(1));
        assert_eq!(parse_credit_micro(".5"), Some(500_000));
        assert_eq!(parse_credit_micro("3"), Some(3_000_000));
        for invalid in ["", "-", "1.", "1.0000001", "1e3", " 1", "1,5", "abc", "99999999999999999"] {
            assert_eq!(parse_credit_micro(invalid), None, "{invalid}");
        }
        assert_eq!(format_credit_micro(12_500_000), "12.5");
        assert_eq!(format_credit_micro(-2_500_000), "-2.5");
        assert_eq!(format_credit_micro(1), "0.000001");
        assert_eq!(format_credit_micro(0), "0");
        assert_eq!(format_credit_micro(-1), "-0.000001");
    }

    #[test]
    fn subjects_parse_and_print_their_wire_form() {
        for text in [
            "type:admin",
            "type:limited",
            "group:students",
            "user:bob",
            "app:writer.test.buckyos.io",
            "user_app:bob/writer",
        ] {
            let subject = PolicySubject::parse(text).unwrap();
            assert_eq!(subject.to_string(), text);
            assert_eq!(serde_json::to_value(&subject).unwrap(), text);
        }
        for invalid in ["type:root", "group:", "user:a/b", "user_app:bob", "team:x", "bob"] {
            assert!(PolicySubject::parse(invalid).is_none(), "{invalid}");
        }
        assert!(PolicySubject::parse("app:writer").unwrap().is_app_side());
        assert!(!PolicySubject::parse("group:a").unwrap().is_app_side());
        assert_eq!(
            PolicySubject::for_user_type(crate::UserType::Root),
            PolicySubject::UserType("admin".into())
        );
    }

    #[test]
    fn limits_and_periods_keep_their_protocol_names() {
        let limit: PolicyLimit =
            serde_json::from_value(serde_json::json!({"period": "5h", "unit": "credit", "amount": "10.5"}))
                .unwrap();
        assert_eq!(limit.amount_value(), Some(10_500_000));
        let tokens = PolicyLimit {
            period: LimitPeriod::Week,
            unit: LimitUnit::Token,
            amount: "200000".into(),
        };
        assert_eq!(tokens.amount_value(), Some(200_000));
        assert_eq!(serde_json::to_value(LimitPeriod::Week).unwrap(), "7d");
        assert_eq!(LimitPeriod::FiveHours.length_ms(), 5 * HOUR_MS);
        assert!(serde_json::from_value::<LimitPeriod>(serde_json::json!("4h")).is_err());
        assert!(serde_json::from_value::<PolicyBody>(serde_json::json!({"price_coeficient": 1})).is_err());
    }
}
