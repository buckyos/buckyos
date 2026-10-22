//! AICC token quota and resource visibility protocol
//! (product/ai_center/AI_Center_Quota_PRD.md, doc/aicc/aicc_token_quota.md).
//!
//! The rules, the live state of their windows and accounts, the denial
//! details carried by the §7.4 error codes, the per-group Provider / Model
//! allow lists and the delegated attribution a system service sends when it
//! calls on a user's behalf.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod quota_methods {
    pub const QUERY: &str = "quota.query";
    pub const RULE_LIST: &str = "quota.rule.list";
    pub const RULE_GET: &str = "quota.rule.get";
    pub const RULE_PREVIEW: &str = "quota.rule.preview";
    pub const RULE_CREATE: &str = "quota.rule.create";
    pub const RULE_UPDATE: &str = "quota.rule.update";
    pub const RULE_DELETE: &str = "quota.rule.delete";
    pub const ACCOUNT_GET: &str = "quota.account.get";
    pub const ACCOUNT_RECHARGE: &str = "quota.account.recharge";
    pub const RECHARGE_GET: &str = "quota.recharge.get";
    pub const VISIBILITY_GET: &str = "visibility.get";
    pub const VISIBILITY_UPDATE: &str = "visibility.update";
    pub const VISIBILITY_PREVIEW: &str = "visibility.preview";

    pub fn is_quota_method(method: &str) -> bool {
        method.starts_with("quota.") || method.starts_with("visibility.")
    }
}

/// Reserved top-level request parameter of a delegated call.
pub const AICC_ON_BEHALF_OF_PARAM: &str = "on_behalf_of";
/// RBAC resource a system service needs (`write`) to send `on_behalf_of`.
pub const AICC_ON_BEHALF_OF_RESOURCE: &str = "obj://aicc/on_behalf_of";
/// Share of a limit at which a rule is shown as near its limit (§9.2, fixed).
pub const QUOTA_NEAR_LIMIT_PERCENT: u64 = 80;

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
    /// The quota user (an account of this Zone) or, for an undelegated
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
pub enum QuotaRuleSource {
    Admin,
    User,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum QuotaObject {
    User { user_id: String },
    /// Shared by every user of the App in this Zone.
    App { app_id: String },
    UserApp { user_id: String, app_id: String },
}

impl QuotaObject {
    pub fn user_id(&self) -> Option<&str> {
        match self {
            Self::User { user_id } | Self::UserApp { user_id, .. } => Some(user_id),
            Self::App { .. } => None,
        }
    }

    pub fn app_id(&self) -> Option<&str> {
        match self {
            Self::App { app_id } | Self::UserApp { app_id, .. } => Some(app_id),
            Self::User { .. } => None,
        }
    }

    pub fn matches(&self, user_id: &str, app_id: Option<&str>) -> bool {
        match self {
            Self::User { user_id: rule_user } => rule_user == user_id,
            Self::App { app_id: rule_app } => app_id == Some(rule_app.as_str()),
            Self::UserApp {
                user_id: rule_user,
                app_id: rule_app,
            } => rule_user == user_id && app_id == Some(rule_app.as_str()),
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Self::User { .. } => "user",
            Self::App { .. } => "app",
            Self::UserApp { .. } => "user_app",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum QuotaResource {
    /// Every token-metered model.
    All,
    Provider { provider_instance_name: String },
    /// `provider_model_id[:variant]@provider_instance_name`.
    Model { exact_model: String },
    /// A resource the viewer may no longer see; identity withheld.
    Hidden,
}

impl QuotaResource {
    pub fn matches(&self, provider_instance_name: &str, exact_model: &str) -> bool {
        match self {
            Self::All => true,
            Self::Provider {
                provider_instance_name: rule_provider,
            } => rule_provider == provider_instance_name,
            Self::Model {
                exact_model: rule_model,
            } => rule_model == exact_model,
            Self::Hidden => false,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Provider { .. } => "provider",
            Self::Model { .. } => "model",
            Self::Hidden => "hidden",
        }
    }

    /// Provider instance this resource lives in, if it names one.
    pub fn provider_instance_name(&self) -> Option<&str> {
        match self {
            Self::Provider {
                provider_instance_name,
            } => Some(provider_instance_name),
            Self::Model { exact_model } => exact_model.rsplit_once('@').map(|(_, provider)| provider),
            Self::All | Self::Hidden => None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum QuotaPeriod {
    #[serde(rename = "4h")]
    FourHours,
    #[serde(rename = "24h")]
    Day,
    #[serde(rename = "72h")]
    ThreeDays,
    #[serde(rename = "week")]
    Week,
}

const HOUR_MS: i64 = 60 * 60 * 1000;
/// `1970-01-05 00:00 UTC`, a Monday: base of the 72-hour and weekly windows.
const PERIOD_EPOCH_MS: i64 = 4 * 24 * HOUR_MS;

impl QuotaPeriod {
    pub const ALL: [QuotaPeriod; 4] = [
        QuotaPeriod::FourHours,
        QuotaPeriod::Day,
        QuotaPeriod::ThreeDays,
        QuotaPeriod::Week,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::FourHours => "4h",
            Self::Day => "24h",
            Self::ThreeDays => "72h",
            Self::Week => "week",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|period| period.as_str() == value)
    }

    pub fn length_ms(self) -> i64 {
        match self {
            Self::FourHours => 4 * HOUR_MS,
            Self::Day => 24 * HOUR_MS,
            Self::ThreeDays => 72 * HOUR_MS,
            Self::Week => 7 * 24 * HOUR_MS,
        }
    }

    /// The fixed UTC window `[start, end)` containing `at_ms` (§5.1).
    pub fn window(self, at_ms: i64) -> (i64, i64) {
        let length = self.length_ms();
        let base = match self {
            Self::FourHours | Self::Day => 0,
            Self::ThreeDays | Self::Week => PERIOD_EPOCH_MS,
        };
        let start = base + (at_ms - base).div_euclid(length) * length;
        (start, start + length)
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum QuotaKind {
    Periodic,
    Total,
}

/// A period change that takes effect when the current window ends.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuotaPendingPeriod {
    pub period: QuotaPeriod,
    pub effective_at_ms: i64,
    /// The switch found another rule with the target period and kept the old one.
    #[serde(default)]
    pub conflict: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuotaRule {
    pub rule_id: String,
    pub source: QuotaRuleSource,
    pub object: QuotaObject,
    pub resource: QuotaResource,
    pub kind: QuotaKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period: Option<QuotaPeriod>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    pub enabled: bool,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_period: Option<QuotaPendingPeriod>,
    pub version: u64,
    pub created_by: String,
    pub created_at_ms: i64,
    pub updated_by: String,
    pub updated_at_ms: i64,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QuotaStatus {
    Normal,
    NearLimit,
    Exhausted,
    /// Periodic limit of zero: use is forbidden.
    Forbidden,
    Disabled,
    /// The Provider or exact model is no longer in the inventory.
    ResourceUnavailable,
}

/// Live state of a rule's current window or of its total account.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuotaUsageState {
    pub status: QuotaStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_start_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_end_ms: Option<i64>,
    /// Next automatic recovery; always `null` for a total account.
    pub reset_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub granted_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_start_ms: Option<i64>,
    pub settled_tokens: u64,
    /// In-flight reservations, including those waiting for reconciliation.
    pub reserved_tokens: u64,
    /// Part of `reserved_tokens` whose real usage is still unknown.
    pub pending_tokens: u64,
    pub available_tokens: u64,
    /// Consumption beyond the grant or limit; a recharge pays it first.
    pub debt_tokens: u64,
    pub updated_at_ms: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuotaRuleView {
    pub rule: QuotaRule,
    pub state: QuotaUsageState,
    /// The viewer may change this rule.
    pub editable: bool,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum QuotaListScope {
    /// Rules that constrain the caller, and the caller's own rules.
    #[default]
    Mine,
    /// Every rule of the Zone (admin only).
    Manage,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuotaQueryRequest {
    /// Only the rules that apply to calls of this App.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuotaQueryResponse {
    pub user_id: String,
    pub rules: Vec<QuotaRuleView>,
    pub near_limit_count: u64,
    pub exhausted_count: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuotaRuleListRequest {
    #[serde(default)]
    pub scope: QuotaListScope,
    #[serde(default)]
    pub include_deleted: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuotaRuleListResponse {
    pub rules: Vec<QuotaRuleView>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuotaRuleGetRequest {
    pub rule_id: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuotaAuditRecord {
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

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QuotaGrantKind {
    Initial,
    Recharge,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuotaGrantRecord {
    pub grant_id: String,
    pub account_id: String,
    pub kind: QuotaGrantKind,
    pub amount_tokens: u64,
    pub operator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuotaAccountView {
    pub account_id: String,
    pub object: QuotaObject,
    pub resource: QuotaResource,
    pub granted_tokens: u64,
    pub settled_tokens: u64,
    pub reserved_tokens: u64,
    pub pending_tokens: u64,
    pub available_tokens: u64,
    pub debt_tokens: u64,
    pub start_at_ms: i64,
    pub version: u64,
    pub updated_at_ms: i64,
    /// Whether an enabled total rule currently blocks on this account.
    pub enforced: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuotaRuleDetail {
    pub rule: QuotaRuleView,
    /// Every enabled rule that constrains the same calls, this one included.
    pub constraints: Vec<QuotaRuleView>,
    /// Rules among `constraints` with no available tokens.
    pub blocking_rule_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<QuotaAccountView>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grants: Vec<QuotaGrantRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audit: Vec<QuotaAuditRecord>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuotaRuleDraft {
    /// Defaults to `admin` for an admin and `user` otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<QuotaRuleSource>,
    pub object: QuotaObject,
    pub resource: QuotaResource,
    pub kind: QuotaKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period: Option<QuotaPeriod>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_tokens: Option<u64>,
    /// Initial grant of a new total account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_tokens: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuotaRulePreviewRequest {
    pub draft: QuotaRuleDraft,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuotaRulePreviewResponse {
    /// State the rule would have if saved now.
    pub state: QuotaUsageState,
    /// A total account of the same scope exists and would be restored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub existing_account: Option<QuotaAccountView>,
    /// An existing rule of the same identity; saving is refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict_rule_id: Option<String>,
    /// Other enabled rules that constrain the same calls.
    pub other_constraints: Vec<QuotaRuleView>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuotaRuleCreateRequest {
    pub draft: QuotaRuleDraft,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuotaRuleUpdateRequest {
    pub rule_id: String,
    pub expected_version: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_tokens: Option<u64>,
    /// New period of a periodic rule, switched when the current window ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period: Option<QuotaPeriod>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub cancel_pending_period: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuotaRuleDeleteRequest {
    pub rule_id: String,
    pub expected_version: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuotaRuleDeleteResponse {
    pub ok: bool,
    pub rule_id: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuotaAccountGetRequest {
    pub account_id: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuotaAccountGetResponse {
    pub account: QuotaAccountView,
    pub grants: Vec<QuotaGrantRecord>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuotaRechargeRequest {
    pub account_id: String,
    /// Client-generated idempotency key: the same id is credited once.
    pub recharge_id: String,
    pub amount_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuotaRechargeResponse {
    pub recharge: QuotaGrantRecord,
    pub account: QuotaAccountView,
    /// The recharge id had already been credited; nothing new was added.
    pub replayed: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuotaRechargeGetRequest {
    pub recharge_id: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuotaRechargeGetResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recharge: Option<QuotaGrantRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<QuotaAccountView>,
}

/// One rule that refused a reservation (§7.3).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuotaBlockingRule {
    pub rule_id: String,
    pub source: QuotaRuleSource,
    pub object: QuotaObject,
    pub resource: QuotaResource,
    pub kind: QuotaKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period: Option<QuotaPeriod>,
    pub state: QuotaUsageState,
    /// Waiting for the window to reset can help: the limit is above zero and
    /// covers the requested budget.
    pub recoverable: bool,
}

/// `details` of `quota_exhausted` and `quota_budget_exceeded`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuotaDenialDetails {
    pub required_tokens: u64,
    /// Largest budget every enabled rule still admits.
    pub max_allowed_tokens: u64,
    pub provider_instance_name: String,
    pub exact_model: String,
    pub blocking: Vec<QuotaBlockingRule>,
    /// When every blocking rule recovers; `null` if one never does by itself.
    pub reset_at_ms: Option<i64>,
}

/// `details` of `quota_unavailable` (§7.4).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuotaUnavailableDetails {
    pub scope: String,
    pub occurred_at_ms: i64,
}

/// `details` of `attribution_unavailable` (§7.4).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AttributionUnavailableDetails {
    pub mode: AiccAttributionMode,
}

pub const VISIBILITY_SUBJECT_TYPE_PREFIX: &str = "type:";
pub const VISIBILITY_SUBJECT_GROUP_PREFIX: &str = "group:";

/// One allow item of a group (§3.4).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum VisibilityEntry {
    /// Every Provider instance, present and future, with all of its models.
    AllProviders,
    /// One Provider with every model of its inventory, present and future.
    ProviderAllModels { provider_instance_name: String },
    /// One Provider with the exact models chosen; new models stay hidden.
    ProviderModels {
        provider_instance_name: String,
        exact_models: Vec<String>,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VisibilitySubjectPolicy {
    /// `type:admin|user|limited` or `group:<id>`.
    pub subject: String,
    pub entries: Vec<VisibilityEntry>,
    pub updated_by: String,
    pub updated_at_ms: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VisibilityProviderInventory {
    pub provider_instance_name: String,
    pub enabled: bool,
    pub exact_models: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VisibilityGetRequest {}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VisibilityGetResponse {
    pub revision: u64,
    pub subjects: Vec<VisibilitySubjectPolicy>,
    pub providers: Vec<VisibilityProviderInventory>,
    /// Inventory models no subject can see.
    pub unexposed_models: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VisibilityUpdateRequest {
    pub expected_revision: u64,
    pub subject: String,
    /// Empty removes every allow item of the subject.
    pub entries: Vec<VisibilityEntry>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VisibilityDraft {
    pub subject: String,
    pub entries: Vec<VisibilityEntry>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VisibilityPreviewRequest {
    /// Final list of this user (defaults to the caller).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    /// Unsaved change to compare against the stored policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<VisibilityDraft>,
}

/// A visible resource and the subjects granting it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VisibilityGrant {
    pub provider_instance_name: String,
    /// `None` for a Provider allowed without any current model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exact_model: Option<String>,
    pub sources: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VisibilityPreviewResponse {
    pub user_id: String,
    pub grants: Vec<VisibilityGrant>,
    /// Exact models the draft opens to its subject.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub added: Vec<String>,
    /// Exact models the draft takes from its subject.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<String>,
    /// Users the draft changes (members of its subject).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affected_users: Vec<String>,
    /// Exact models the caller would lose with the draft.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub caller_loses: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AiccListView {
    /// Resources the caller may use.
    #[default]
    Use,
    /// The complete inventory (admin only), each model marked with whether
    /// the caller may use it.
    Manage,
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 24 * HOUR_MS;

    #[test]
    fn windows_follow_fixed_utc_boundaries() {
        let at = 1_000 * DAY + 5 * HOUR_MS + 1;
        assert_eq!(
            QuotaPeriod::FourHours.window(at),
            (1_000 * DAY + 4 * HOUR_MS, 1_000 * DAY + 8 * HOUR_MS)
        );
        assert_eq!(QuotaPeriod::Day.window(at), (1_000 * DAY, 1_001 * DAY));
        let boundary = 1_000 * DAY + 4 * HOUR_MS;
        assert_eq!(QuotaPeriod::FourHours.window(boundary).0, boundary);
        assert_eq!(QuotaPeriod::FourHours.window(boundary - 1).1, boundary);
    }

    #[test]
    fn week_and_72h_windows_start_from_monday_1970_01_05() {
        assert_eq!(QuotaPeriod::Week.window(PERIOD_EPOCH_MS), (PERIOD_EPOCH_MS, PERIOD_EPOCH_MS + 7 * DAY));
        // 2026-10-19 is a Monday.
        let monday = 20_745 * DAY;
        let (start, end) = QuotaPeriod::Week.window(monday + 3 * DAY);
        assert_eq!((start, end), (monday, monday + 7 * DAY));
        let (start, _) = QuotaPeriod::ThreeDays.window(PERIOD_EPOCH_MS + 3 * DAY + 1);
        assert_eq!(start, PERIOD_EPOCH_MS + 3 * DAY);
        let (start, end) = QuotaPeriod::ThreeDays.window(0);
        assert_eq!((start, end), (PERIOD_EPOCH_MS - 6 * DAY, PERIOD_EPOCH_MS - 3 * DAY));
    }

    #[test]
    fn objects_and_resources_match_their_scope() {
        let shared = QuotaObject::App {
            app_id: "writer".into(),
        };
        assert!(shared.matches("alice", Some("writer")));
        assert!(shared.matches("bob", Some("writer")));
        assert!(!shared.matches("bob", None));
        let own = QuotaObject::UserApp {
            user_id: "alice".into(),
            app_id: "writer".into(),
        };
        assert!(!own.matches("bob", Some("writer")));
        let model = QuotaResource::Model {
            exact_model: "model-a@cloud-main".into(),
        };
        assert!(model.matches("cloud-main", "model-a@cloud-main"));
        assert!(!model.matches("cloud-2", "model-a@cloud-2"));
        assert_eq!(model.provider_instance_name(), Some("cloud-main"));
        assert!(!QuotaResource::Hidden.matches("cloud-main", "model-a@cloud-main"));
    }

    #[test]
    fn protocol_names_are_stable() {
        assert_eq!(
            serde_json::to_value(QuotaObject::UserApp {
                user_id: "alice".into(),
                app_id: "aiworkspace".into()
            })
            .unwrap(),
            serde_json::json!({"type": "user_app", "user_id": "alice", "app_id": "aiworkspace"})
        );
        assert_eq!(serde_json::to_value(QuotaPeriod::FourHours).unwrap(), "4h");
        assert_eq!(
            serde_json::to_value(VisibilityEntry::AllProviders).unwrap(),
            serde_json::json!({"type": "all_providers"})
        );
    }
}
