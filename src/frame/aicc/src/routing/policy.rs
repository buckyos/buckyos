use crate::matching::{CompiledMatchRule, MatchContext, MatchRule, ROUTING_PROVIDER_MATCH_SCHEMA};
use async_trait::async_trait;
use buckyos_api::{
    AiccPolicyConfig, AiccSchedulerProfile, AiccSchedulerProfileConfig, ApiType, Capability,
    LockedValue, Money, QuotaQueryRequest, QuotaQueryResponse, QuotaState, QuotaView,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PolicyScope {
    System,
    User,
    App,
    Session,
    Request,
}

impl fmt::Display for PolicyScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::System => "system",
            Self::User => "user",
            Self::App => "app",
            Self::Session => "session",
            Self::Request => "request",
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum PrivacyRequirement {
    #[default]
    PublicAllowed,
    NoLogRequired,
    PrivateOnly,
    LocalOnly,
}

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum ProviderTrustLevel {
    #[default]
    Unknown,
    Registered,
    Verified,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RoutingPolicyPatch {
    pub route: AiccPolicyConfig,
    pub privacy: Option<LockedValue<PrivacyRequirement>>,
    pub minimum_provider_trust: Option<LockedValue<ProviderTrustLevel>>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RoutingPolicyLayers<'a> {
    pub system: Option<&'a RoutingPolicyPatch>,
    pub user: Option<&'a RoutingPolicyPatch>,
    pub app: Option<&'a RoutingPolicyPatch>,
    pub session: Option<&'a RoutingPolicyPatch>,
    pub request: Option<&'a RoutingPolicyPatch>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EffectiveValue<T> {
    pub value: T,
    pub source: Option<PolicyScope>,
    pub locked: bool,
}

impl<T> EffectiveValue<T> {
    fn new(value: T) -> Self {
        Self {
            value,
            source: None,
            locked: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EffectiveRoutingPolicy {
    pub profile: EffectiveValue<AiccSchedulerProfile>,
    pub scheduler_profiles: EffectiveValue<Option<AiccSchedulerProfileConfig>>,
    pub local_only: EffectiveValue<bool>,
    pub allow_fallback: EffectiveValue<bool>,
    pub allow_exact_model_fallback: EffectiveValue<bool>,
    pub runtime_failover: EffectiveValue<bool>,
    pub explain: EffectiveValue<bool>,
    pub blocked_provider_instances: EffectiveValue<Vec<String>>,
    pub allowed_provider_instances: EffectiveValue<Vec<String>>,
    pub max_estimated_cost: EffectiveValue<Option<Money>>,
    pub privacy: EffectiveValue<PrivacyRequirement>,
    pub minimum_provider_trust: EffectiveValue<ProviderTrustLevel>,
}

impl Default for EffectiveRoutingPolicy {
    fn default() -> Self {
        Self {
            profile: EffectiveValue::new(AiccSchedulerProfile::CostFirst),
            scheduler_profiles: EffectiveValue::new(None),
            local_only: EffectiveValue::new(false),
            allow_fallback: EffectiveValue::new(true),
            allow_exact_model_fallback: EffectiveValue::new(false),
            runtime_failover: EffectiveValue::new(true),
            explain: EffectiveValue::new(false),
            blocked_provider_instances: EffectiveValue::new(Vec::new()),
            allowed_provider_instances: EffectiveValue::new(Vec::new()),
            max_estimated_cost: EffectiveValue::new(None),
            privacy: EffectiveValue::new(PrivacyRequirement::PublicAllowed),
            minimum_provider_trust: EffectiveValue::new(ProviderTrustLevel::Registered),
        }
    }
}

impl EffectiveRoutingPolicy {
    pub(crate) fn merge(layers: RoutingPolicyLayers<'_>) -> Result<Self, PolicyError> {
        let mut result = Self::default();
        for (scope, patch) in [
            (PolicyScope::System, layers.system),
            (PolicyScope::User, layers.user),
            (PolicyScope::App, layers.app),
            (PolicyScope::Session, layers.session),
            (PolicyScope::Request, layers.request),
        ] {
            if let Some(patch) = patch {
                result.apply(scope, patch)?;
            }
        }
        if result
            .max_estimated_cost
            .value
            .as_ref()
            .is_some_and(|value| !valid_money(value))
        {
            return Err(PolicyError::InvalidPolicy(
                "max_estimated_cost must contain a finite non-negative amount and a non-empty currency"
                    .into(),
            ));
        }
        Ok(result)
    }

    fn apply(&mut self, scope: PolicyScope, patch: &RoutingPolicyPatch) -> Result<(), PolicyError> {
        apply(
            "profile",
            &mut self.profile,
            patch.route.profile.as_ref(),
            scope,
        )?;
        apply_optional(
            "scheduler_profiles",
            &mut self.scheduler_profiles,
            patch.route.scheduler_profiles.as_ref(),
            scope,
        )?;
        apply(
            "local_only",
            &mut self.local_only,
            patch.route.local_only.as_ref(),
            scope,
        )?;
        apply(
            "allow_fallback",
            &mut self.allow_fallback,
            patch.route.allow_fallback.as_ref(),
            scope,
        )?;
        apply(
            "allow_exact_model_fallback",
            &mut self.allow_exact_model_fallback,
            patch.route.allow_exact_model_fallback.as_ref(),
            scope,
        )?;
        apply(
            "runtime_failover",
            &mut self.runtime_failover,
            patch.route.runtime_failover.as_ref(),
            scope,
        )?;
        apply(
            "explain",
            &mut self.explain,
            patch.route.explain.as_ref(),
            scope,
        )?;
        apply(
            "blocked_provider_instances",
            &mut self.blocked_provider_instances,
            patch.route.blocked_provider_instances.as_ref(),
            scope,
        )?;
        apply(
            "allowed_provider_instances",
            &mut self.allowed_provider_instances,
            patch.route.allowed_provider_instances.as_ref(),
            scope,
        )?;
        apply_optional(
            "max_estimated_cost",
            &mut self.max_estimated_cost,
            patch.route.max_estimated_cost.as_ref(),
            scope,
        )?;
        apply("privacy", &mut self.privacy, patch.privacy.as_ref(), scope)?;
        apply(
            "minimum_provider_trust",
            &mut self.minimum_provider_trust,
            patch.minimum_provider_trust.as_ref(),
            scope,
        )
    }

    fn requires_local(&self) -> bool {
        self.local_only.value
            || self.privacy.value == PrivacyRequirement::LocalOnly
            || self.profile.value == AiccSchedulerProfile::StrictLocal
    }

    pub(crate) fn locality_preference(&self) -> LocalityPreference {
        if self.requires_local() {
            LocalityPreference::RequireLocal
        } else if self.profile.value == AiccSchedulerProfile::LocalFirst {
            LocalityPreference::PreferLocal
        } else {
            LocalityPreference::Neutral
        }
    }
}

fn apply<T: Clone + PartialEq>(
    field: &'static str,
    current: &mut EffectiveValue<T>,
    patch: Option<&LockedValue<T>>,
    scope: PolicyScope,
) -> Result<(), PolicyError> {
    let Some(patch) = patch else { return Ok(()) };
    if current.locked {
        if current.value != patch.value {
            return Err(PolicyError::LockedOverride {
                field,
                locked_by: current.source.expect("locked value must have a source"),
                attempted_by: scope,
            });
        }
        return Ok(());
    }
    current.value = patch.value.clone();
    current.source = Some(scope);
    current.locked = patch.locked;
    Ok(())
}

fn apply_optional<T: Clone + PartialEq>(
    field: &'static str,
    current: &mut EffectiveValue<Option<T>>,
    patch: Option<&LockedValue<T>>,
    scope: PolicyScope,
) -> Result<(), PolicyError> {
    let Some(patch) = patch else { return Ok(()) };
    apply(
        field,
        current,
        Some(&LockedValue {
            value: Some(patch.value.clone()),
            locked: patch.locked,
        }),
        scope,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProviderType {
    LocalInference,
    CloudApi,
    ProxyUnknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProviderTypeSource {
    SystemConfig,
    AdminOverride,
    ProviderInventory,
}

impl ProviderTypeSource {
    fn trusted(self) -> bool {
        matches!(self, Self::SystemConfig | Self::AdminOverride)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProviderPrivacy {
    Local,
    PrivateCloud,
    PublicCloud,
    PublicCloudNoLog,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProviderTrustView {
    pub provider_type: ProviderType,
    pub provider_type_source: ProviderTypeSource,
    pub provider_type_revision: String,
    pub asserted_at_ms: i64,
    pub trust_level: ProviderTrustLevel,
}

impl ProviderTrustView {
    fn trusted_local(&self) -> bool {
        self.provider_type == ProviderType::LocalInference
            && self.provider_type_source.trusted()
            && self.trust_level >= ProviderTrustLevel::Verified
            && !self.provider_type_revision.trim().is_empty()
            && self.asserted_at_ms >= 0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CredentialScope {
    Tenant {
        tenant_id: String,
    },
    User {
        tenant_id: String,
        user_id: String,
    },
    App {
        tenant_id: String,
        app_id: String,
    },
    UserApp {
        tenant_id: String,
        user_id: String,
        app_id: String,
    },
}

impl CredentialScope {
    fn permits(&self, caller: &CallerIdentity) -> bool {
        match self {
            Self::Tenant { tenant_id } => tenant_id == &caller.tenant_id,
            Self::User { tenant_id, user_id } => {
                tenant_id == &caller.tenant_id && user_id == &caller.user_id
            }
            Self::App { tenant_id, app_id } => {
                tenant_id == &caller.tenant_id && caller.app_id.as_ref() == Some(app_id)
            }
            Self::UserApp {
                tenant_id,
                user_id,
                app_id,
            } => {
                tenant_id == &caller.tenant_id
                    && user_id == &caller.user_id
                    && caller.app_id.as_ref() == Some(app_id)
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CallerIdentity {
    pub tenant_id: String,
    pub user_id: String,
    pub app_id: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct CandidatePolicyInput<'a> {
    pub provider_instance_name: &'a str,
    pub api_type: ApiType,
    pub logical_path: Option<&'a str>,
    pub provider_privacy: ProviderPrivacy,
    pub trust: Option<&'a ProviderTrustView>,
    pub credential_scope: &'a CredentialScope,
}

#[derive(Clone, Debug)]
pub(crate) struct RequestPolicyInput<'a> {
    pub caller: &'a CallerIdentity,
    pub method: &'a str,
    pub capability: Capability,
    pub estimated_cost: Option<Money>,
    pub request_units: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct QuotaLookup {
    pub caller: CallerIdentity,
    pub capability: Option<Capability>,
    pub method: Option<String>,
    pub provider_instance_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct QuotaSnapshot {
    pub state: Option<QuotaState>,
    pub remaining_request_units: Option<u64>,
    pub remaining_cost: Option<Money>,
    pub reset_at: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct QuotaSourceError;

pub(crate) trait QuotaSource: Send + Sync {
    fn query(&self, lookup: &QuotaLookup) -> Result<QuotaSnapshot, QuotaSourceError>;
}

#[async_trait]
pub(crate) trait QuotaTruthPort: Send + Sync {
    async fn query(&self, lookup: &QuotaLookup) -> Result<QuotaSnapshot, QuotaSourceError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PreparedQuotaScope {
    caller: CallerIdentity,
    capability: Capability,
    method: String,
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedQuotaSource {
    scope: PreparedQuotaScope,
    providers: BTreeMap<String, QuotaSnapshot>,
}

impl QuotaSource for PreparedQuotaSource {
    fn query(&self, lookup: &QuotaLookup) -> Result<QuotaSnapshot, QuotaSourceError> {
        if lookup.caller != self.scope.caller
            || lookup.capability.as_ref() != Some(&self.scope.capability)
            || lookup.method.as_deref() != Some(self.scope.method.as_str())
        {
            return Err(QuotaSourceError);
        }
        let provider = lookup
            .provider_instance_name
            .as_ref()
            .ok_or(QuotaSourceError)?;
        self.providers
            .get(provider)
            .cloned()
            .ok_or(QuotaSourceError)
    }
}

pub(crate) struct QuotaSourceFactory {
    truth: Arc<dyn QuotaTruthPort>,
}

impl QuotaSourceFactory {
    pub(crate) fn new(truth: Arc<dyn QuotaTruthPort>) -> Self {
        Self { truth }
    }

    pub(crate) async fn prepare_route<I, S>(
        &self,
        caller: &CallerIdentity,
        capability: Capability,
        method: &str,
        provider_instance_names: I,
    ) -> Result<PreparedQuotaSource, QuotaSourceError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        validate_lookup_scope(caller, Some(method))?;
        let provider_names = provider_instance_names
            .into_iter()
            .map(Into::into)
            .collect::<BTreeSet<_>>();
        if provider_names.is_empty() || provider_names.iter().any(|name| name.trim().is_empty()) {
            return Err(QuotaSourceError);
        }
        let mut providers = BTreeMap::new();
        for provider_instance_name in provider_names {
            let lookup = QuotaLookup {
                caller: caller.clone(),
                capability: Some(capability.clone()),
                method: Some(method.to_owned()),
                provider_instance_name: Some(provider_instance_name.clone()),
            };
            let snapshot = self.truth.query(&lookup).await?;
            validate_snapshot(&snapshot)?;
            providers.insert(provider_instance_name, snapshot);
        }
        Ok(PreparedQuotaSource {
            scope: PreparedQuotaScope {
                caller: caller.clone(),
                capability,
                method: method.to_owned(),
            },
            providers,
        })
    }

    pub(crate) async fn query_quota(
        &self,
        caller: &CallerIdentity,
        request: QuotaQueryRequest,
    ) -> Result<QuotaQueryResponse, PolicyError> {
        validate_lookup_scope(caller, request.method.as_deref())
            .map_err(|_| PolicyError::QuotaSourceUnavailable)?;
        let snapshot = self
            .truth
            .query(&QuotaLookup {
                caller: caller.clone(),
                capability: request.capability,
                method: request.method,
                provider_instance_name: None,
            })
            .await
            .map_err(|_| PolicyError::QuotaSourceUnavailable)?;
        quota_response(snapshot)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LocalityPreference {
    Neutral,
    PreferLocal,
    RequireLocal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PolicyReasonCode {
    ProviderNotAllowed,
    ProviderBlocked,
    ProviderTrustUnavailable,
    ProviderTypeSourceUntrusted,
    ProviderTrustInsufficient,
    LocalProviderRequired,
    PrivacyIncompatible,
    CredentialScopeMismatch,
    CostEstimateUnavailable,
    CostCurrencyMismatch,
    CostCeilingExceeded,
    QuotaSourceUnavailable,
    QuotaExhausted,
    RequestQuotaExceeded,
    BudgetExceeded,
}

impl PolicyReasonCode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ProviderNotAllowed => "provider_not_allowed",
            Self::ProviderBlocked => "provider_blocked",
            Self::ProviderTrustUnavailable => "provider_trust_unavailable",
            Self::ProviderTypeSourceUntrusted => "provider_type_source_untrusted",
            Self::ProviderTrustInsufficient => "provider_trust_insufficient",
            Self::LocalProviderRequired => "local_provider_required",
            Self::PrivacyIncompatible => "privacy_incompatible",
            Self::CredentialScopeMismatch => "credential_scope_mismatch",
            Self::CostEstimateUnavailable => "cost_estimate_unavailable",
            Self::CostCurrencyMismatch => "cost_currency_mismatch",
            Self::CostCeilingExceeded => "cost_ceiling_exceeded",
            Self::QuotaSourceUnavailable => "quota_source_unavailable",
            Self::QuotaExhausted => "quota_exhausted",
            Self::RequestQuotaExceeded => "request_quota_exceeded",
            Self::BudgetExceeded => "budget_exceeded",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PolicyReason {
    pub code: PolicyReasonCode,
    pub summary: &'static str,
}

impl PolicyReason {
    fn new(code: PolicyReasonCode, summary: &'static str) -> Self {
        Self { code, summary }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PolicyDecision {
    pub allowed: bool,
    pub locality_preference: LocalityPreference,
    pub reasons: Vec<PolicyReason>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum PolicyError {
    LockedOverride {
        field: &'static str,
        locked_by: PolicyScope,
        attempted_by: PolicyScope,
    },
    InvalidPolicy(String),
    QuotaSourceUnavailable,
}

impl fmt::Display for PolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LockedOverride { field, locked_by, attempted_by } => write!(
                f,
                "policy field {field} was locked by {locked_by} and cannot be changed by {attempted_by}"
            ),
            Self::InvalidPolicy(message) => write!(f, "invalid routing policy: {message}"),
            Self::QuotaSourceUnavailable => f.write_str("quota truth source unavailable"),
        }
    }
}

impl std::error::Error for PolicyError {}

pub(crate) struct PolicyEngine<Q> {
    policy: EffectiveRoutingPolicy,
    allowed_rules: Vec<CompiledMatchRule>,
    blocked_rules: Vec<CompiledMatchRule>,
    quota_source: Q,
}

impl<Q: QuotaSource> PolicyEngine<Q> {
    pub(crate) fn new(
        policy: EffectiveRoutingPolicy,
        quota_source: Q,
    ) -> Result<Self, PolicyError> {
        Ok(Self {
            allowed_rules: compile_rules(
                "allowed_provider_instances",
                &policy.allowed_provider_instances.value,
            )?,
            blocked_rules: compile_rules(
                "blocked_provider_instances",
                &policy.blocked_provider_instances.value,
            )?,
            policy,
            quota_source,
        })
    }

    pub(crate) fn policy(&self) -> &EffectiveRoutingPolicy {
        &self.policy
    }

    pub(crate) fn evaluate(
        &self,
        request: &RequestPolicyInput<'_>,
        candidate: &CandidatePolicyInput<'_>,
    ) -> PolicyDecision {
        let mut reasons = Vec::new();
        let match_context = provider_context(candidate);
        if !self.allowed_rules.is_empty()
            && !self
                .allowed_rules
                .iter()
                .any(|rule| rule.matches(&match_context))
        {
            reject(
                &mut reasons,
                PolicyReasonCode::ProviderNotAllowed,
                "provider is outside the effective allow list",
            );
        }
        if self
            .blocked_rules
            .iter()
            .any(|rule| rule.matches(&match_context))
        {
            reject(
                &mut reasons,
                PolicyReasonCode::ProviderBlocked,
                "provider is blocked by policy",
            );
        }

        match candidate.trust {
            None => reject(
                &mut reasons,
                PolicyReasonCode::ProviderTrustUnavailable,
                "provider trust truth is unavailable",
            ),
            Some(trust) => {
                if !trust.provider_type_source.trusted()
                    || trust.provider_type_revision.trim().is_empty()
                    || trust.asserted_at_ms < 0
                {
                    reject(
                        &mut reasons,
                        PolicyReasonCode::ProviderTypeSourceUntrusted,
                        "provider type was not asserted by a trusted system source",
                    );
                }
                if trust.trust_level < self.policy.minimum_provider_trust.value {
                    reject(
                        &mut reasons,
                        PolicyReasonCode::ProviderTrustInsufficient,
                        "provider trust level is below policy minimum",
                    );
                }
                if self.policy.requires_local() && !trust.trusted_local() {
                    reject(
                        &mut reasons,
                        PolicyReasonCode::LocalProviderRequired,
                        "policy requires a trusted local provider",
                    );
                }
            }
        }
        if !privacy_allows(self.policy.privacy.value, candidate.provider_privacy) {
            reject(
                &mut reasons,
                PolicyReasonCode::PrivacyIncompatible,
                "provider privacy class does not satisfy request policy",
            );
        }
        if !candidate.credential_scope.permits(request.caller) {
            reject(
                &mut reasons,
                PolicyReasonCode::CredentialScopeMismatch,
                "provider credential is outside the caller scope",
            );
        }

        let estimated_cost = request
            .estimated_cost
            .as_ref()
            .filter(|cost| valid_money(cost));
        if let Some(ceiling) = self.policy.max_estimated_cost.value.as_ref() {
            match estimated_cost {
                None => reject(
                    &mut reasons,
                    PolicyReasonCode::CostEstimateUnavailable,
                    "cost ceiling cannot be enforced without a valid estimate",
                ),
                Some(cost) if cost.currency != ceiling.currency => reject(
                    &mut reasons,
                    PolicyReasonCode::CostCurrencyMismatch,
                    "estimated cost and cost ceiling use different currencies",
                ),
                Some(cost) if cost.amount > ceiling.amount => reject(
                    &mut reasons,
                    PolicyReasonCode::CostCeilingExceeded,
                    "estimated request cost exceeds the single-request ceiling",
                ),
                Some(_) => {}
            }
        }

        let lookup = QuotaLookup {
            caller: request.caller.clone(),
            capability: Some(request.capability.clone()),
            method: Some(request.method.into()),
            provider_instance_name: Some(candidate.provider_instance_name.into()),
        };
        match self.quota_source.query(&lookup) {
            Err(_) => reject(
                &mut reasons,
                PolicyReasonCode::QuotaSourceUnavailable,
                "quota truth source is unavailable",
            ),
            Ok(quota) => apply_quota(&mut reasons, &quota, request.request_units, estimated_cost),
        }
        PolicyDecision {
            allowed: reasons.is_empty(),
            locality_preference: self.policy.locality_preference(),
            reasons,
        }
    }

    pub(crate) fn query_quota(
        &self,
        caller: &CallerIdentity,
        request: QuotaQueryRequest,
    ) -> Result<QuotaQueryResponse, PolicyError> {
        let snapshot = self
            .quota_source
            .query(&QuotaLookup {
                caller: caller.clone(),
                capability: request.capability,
                method: request.method,
                provider_instance_name: None,
            })
            .map_err(|_| PolicyError::QuotaSourceUnavailable)?;
        quota_response(snapshot)
    }
}

fn validate_lookup_scope(
    caller: &CallerIdentity,
    method: Option<&str>,
) -> Result<(), QuotaSourceError> {
    if caller.tenant_id.trim().is_empty()
        || caller.user_id.trim().is_empty()
        || caller
            .app_id
            .as_ref()
            .is_some_and(|app| app.trim().is_empty())
        || method.is_some_and(|value| value.trim().is_empty())
    {
        return Err(QuotaSourceError);
    }
    Ok(())
}

fn validate_snapshot(snapshot: &QuotaSnapshot) -> Result<(), QuotaSourceError> {
    if snapshot.state.is_none()
        || snapshot
            .remaining_cost
            .as_ref()
            .is_some_and(|value| !valid_money(value))
    {
        return Err(QuotaSourceError);
    }
    Ok(())
}

fn quota_response(snapshot: QuotaSnapshot) -> Result<QuotaQueryResponse, PolicyError> {
    validate_snapshot(&snapshot).map_err(|_| PolicyError::QuotaSourceUnavailable)?;
    Ok(QuotaQueryResponse {
        quota: QuotaView {
            state: snapshot.state.expect("validated quota state"),
            remaining_request_units: snapshot.remaining_request_units,
            remaining_cost: snapshot.remaining_cost,
            reset_at: snapshot.reset_at,
        },
    })
}

fn reject(reasons: &mut Vec<PolicyReason>, code: PolicyReasonCode, summary: &'static str) {
    reasons.push(PolicyReason::new(code, summary));
}

fn compile_rules(
    field: &'static str,
    patterns: &[String],
) -> Result<Vec<CompiledMatchRule>, PolicyError> {
    patterns
        .iter()
        .map(|pattern| {
            CompiledMatchRule::compile(
                MatchRule::Shorthand(pattern.clone()),
                &ROUTING_PROVIDER_MATCH_SCHEMA,
            )
            .map_err(|error| PolicyError::InvalidPolicy(format!("{field}: {error}")))
        })
        .collect()
}

fn provider_context(candidate: &CandidatePolicyInput<'_>) -> MatchContext {
    let mut context = MatchContext::from([
        (
            "provider_instance_name".into(),
            Value::String(candidate.provider_instance_name.into()),
        ),
        (
            "api_type".into(),
            serde_json::to_value(candidate.api_type).expect("ApiType serialization cannot fail"),
        ),
    ]);
    if let Some(path) = candidate.logical_path {
        context.insert("logical_path".into(), Value::String(path.into()));
    }
    context
}

fn privacy_allows(requirement: PrivacyRequirement, provider: ProviderPrivacy) -> bool {
    match requirement {
        PrivacyRequirement::PublicAllowed => true,
        PrivacyRequirement::NoLogRequired => matches!(
            provider,
            ProviderPrivacy::Local
                | ProviderPrivacy::PrivateCloud
                | ProviderPrivacy::PublicCloudNoLog
        ),
        PrivacyRequirement::PrivateOnly => matches!(
            provider,
            ProviderPrivacy::Local | ProviderPrivacy::PrivateCloud
        ),
        PrivacyRequirement::LocalOnly => provider == ProviderPrivacy::Local,
    }
}

fn apply_quota(
    reasons: &mut Vec<PolicyReason>,
    quota: &QuotaSnapshot,
    request_units: u64,
    estimated_cost: Option<&Money>,
) {
    match quota.state {
        None => reject(
            reasons,
            PolicyReasonCode::QuotaSourceUnavailable,
            "quota truth source returned an unknown state",
        ),
        Some(QuotaState::Exhausted) => reject(
            reasons,
            PolicyReasonCode::QuotaExhausted,
            "quota is exhausted",
        ),
        Some(QuotaState::Normal | QuotaState::NearLimit) => {}
    }
    if quota
        .remaining_request_units
        .is_some_and(|remaining| remaining < request_units)
    {
        reject(
            reasons,
            PolicyReasonCode::RequestQuotaExceeded,
            "remaining request quota is insufficient",
        );
    }
    if let Some(remaining) = quota.remaining_cost.as_ref() {
        if !valid_money(remaining) {
            reject(
                reasons,
                PolicyReasonCode::QuotaSourceUnavailable,
                "budget truth source returned an invalid value",
            );
        } else if let Some(cost) = estimated_cost {
            if cost.currency != remaining.currency {
                reject(
                    reasons,
                    PolicyReasonCode::CostCurrencyMismatch,
                    "estimated cost and remaining budget use different currencies",
                );
            } else if cost.amount > remaining.amount {
                reject(
                    reasons,
                    PolicyReasonCode::BudgetExceeded,
                    "remaining budget is below estimated request cost",
                );
            }
        } else {
            reject(
                reasons,
                PolicyReasonCode::CostEstimateUnavailable,
                "budget cannot be enforced without a valid cost estimate",
            );
        }
    }
}

fn valid_money(value: &Money) -> bool {
    value.amount.is_finite()
        && value.amount >= 0.0
        && !value.currency.is_empty()
        && value.currency.trim() == value.currency
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct FakeQuota {
        result: Result<QuotaSnapshot, QuotaSourceError>,
        seen: Arc<Mutex<Vec<QuotaLookup>>>,
    }

    impl FakeQuota {
        fn available() -> Self {
            Self {
                result: Ok(QuotaSnapshot {
                    state: Some(QuotaState::Normal),
                    remaining_request_units: Some(10),
                    remaining_cost: Some(Money::new(10.0, "USD")),
                    reset_at: None,
                }),
                seen: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl QuotaSource for FakeQuota {
        fn query(&self, lookup: &QuotaLookup) -> Result<QuotaSnapshot, QuotaSourceError> {
            self.seen.lock().unwrap().push(lookup.clone());
            self.result.clone()
        }
    }

    struct FakeTruthPort {
        result: Result<QuotaSnapshot, QuotaSourceError>,
        seen: Arc<Mutex<Vec<QuotaLookup>>>,
    }

    #[async_trait]
    impl QuotaTruthPort for FakeTruthPort {
        async fn query(&self, lookup: &QuotaLookup) -> Result<QuotaSnapshot, QuotaSourceError> {
            self.seen.lock().unwrap().push(lookup.clone());
            self.result.clone()
        }
    }

    fn caller() -> CallerIdentity {
        CallerIdentity {
            tenant_id: "tenant-a".into(),
            user_id: "user-a".into(),
            app_id: Some("app-a".into()),
        }
    }

    fn trust(provider_type: ProviderType) -> ProviderTrustView {
        ProviderTrustView {
            provider_type,
            provider_type_source: ProviderTypeSource::SystemConfig,
            provider_type_revision: "rev-1".into(),
            asserted_at_ms: 1,
            trust_level: ProviderTrustLevel::Verified,
        }
    }

    fn evaluate(
        engine: &PolicyEngine<FakeQuota>,
        caller: &CallerIdentity,
        trust: Option<&ProviderTrustView>,
        scope: &CredentialScope,
        privacy: ProviderPrivacy,
    ) -> PolicyDecision {
        engine.evaluate(
            &RequestPolicyInput {
                caller,
                method: "chat.completions.create",
                capability: Capability::Llm,
                estimated_cost: Some(Money::new(0.25, "USD")),
                request_units: 1,
            },
            &CandidatePolicyInput {
                provider_instance_name: "openai_primary",
                api_type: ApiType::Llm,
                logical_path: Some("llm.plan"),
                provider_privacy: privacy,
                trust,
                credential_scope: scope,
            },
        )
    }

    fn engine(patch: &RoutingPolicyPatch, quota: FakeQuota) -> PolicyEngine<FakeQuota> {
        let policy = EffectiveRoutingPolicy::merge(RoutingPolicyLayers {
            system: Some(patch),
            user: None,
            app: None,
            session: None,
            request: None,
        })
        .unwrap();
        PolicyEngine::new(policy, quota).unwrap()
    }

    #[test]
    fn merges_system_user_app_session_request_in_order() {
        fn profile(value: AiccSchedulerProfile) -> RoutingPolicyPatch {
            RoutingPolicyPatch {
                route: AiccPolicyConfig {
                    profile: Some(LockedValue::new(value)),
                    ..Default::default()
                },
                ..Default::default()
            }
        }
        let system = profile(AiccSchedulerProfile::QualityFirst);
        let user = profile(AiccSchedulerProfile::CostFirst);
        let app = profile(AiccSchedulerProfile::Balanced);
        let session = profile(AiccSchedulerProfile::LatencyFirst);
        let request = profile(AiccSchedulerProfile::LocalFirst);
        let effective = EffectiveRoutingPolicy::merge(RoutingPolicyLayers {
            system: Some(&system),
            user: Some(&user),
            app: Some(&app),
            session: Some(&session),
            request: Some(&request),
        })
        .unwrap();
        assert_eq!(effective.profile.value, AiccSchedulerProfile::LocalFirst);
        assert_eq!(effective.profile.source, Some(PolicyScope::Request));
        assert_eq!(
            effective.locality_preference(),
            LocalityPreference::PreferLocal
        );
    }

    #[test]
    fn locked_policy_cannot_be_relaxed_or_unlocked() {
        let system = RoutingPolicyPatch {
            route: AiccPolicyConfig {
                local_only: Some(LockedValue::locked(true)),
                ..Default::default()
            },
            ..Default::default()
        };
        let request = RoutingPolicyPatch {
            route: AiccPolicyConfig {
                local_only: Some(LockedValue::new(false)),
                ..Default::default()
            },
            ..Default::default()
        };
        let error = EffectiveRoutingPolicy::merge(RoutingPolicyLayers {
            system: Some(&system),
            user: None,
            app: None,
            session: None,
            request: Some(&request),
        })
        .unwrap_err();
        assert_eq!(
            error,
            PolicyError::LockedOverride {
                field: "local_only",
                locked_by: PolicyScope::System,
                attempted_by: PolicyScope::Request,
            }
        );

        let same = RoutingPolicyPatch {
            route: AiccPolicyConfig {
                local_only: Some(LockedValue::new(true)),
                ..Default::default()
            },
            ..Default::default()
        };
        let effective = EffectiveRoutingPolicy::merge(RoutingPolicyLayers {
            system: Some(&system),
            user: None,
            app: None,
            session: None,
            request: Some(&same),
        })
        .unwrap();
        assert!(effective.local_only.locked);
        assert_eq!(effective.local_only.source, Some(PolicyScope::System));
    }

    #[test]
    fn local_only_uses_trusted_system_provider_type() {
        let patch = RoutingPolicyPatch {
            route: AiccPolicyConfig {
                local_only: Some(LockedValue::new(true)),
                ..Default::default()
            },
            ..Default::default()
        };
        let engine = engine(&patch, FakeQuota::available());
        let caller = caller();
        let scope = CredentialScope::Tenant {
            tenant_id: "tenant-a".into(),
        };
        let cloud = trust(ProviderType::CloudApi);
        let decision = evaluate(
            &engine,
            &caller,
            Some(&cloud),
            &scope,
            ProviderPrivacy::PublicCloud,
        );
        assert!(decision
            .reasons
            .iter()
            .any(|r| r.code == PolicyReasonCode::LocalProviderRequired));

        let mut fake_local = trust(ProviderType::LocalInference);
        fake_local.provider_type_source = ProviderTypeSource::ProviderInventory;
        let decision = evaluate(
            &engine,
            &caller,
            Some(&fake_local),
            &scope,
            ProviderPrivacy::Local,
        );
        assert!(decision
            .reasons
            .iter()
            .any(|r| r.code == PolicyReasonCode::ProviderTypeSourceUntrusted));
        assert!(decision
            .reasons
            .iter()
            .any(|r| r.code == PolicyReasonCode::LocalProviderRequired));
    }

    #[test]
    fn privacy_and_trust_are_hard_filters() {
        let patch = RoutingPolicyPatch {
            privacy: Some(LockedValue::locked(PrivacyRequirement::PrivateOnly)),
            minimum_provider_trust: Some(LockedValue::locked(ProviderTrustLevel::Verified)),
            ..Default::default()
        };
        let engine = engine(&patch, FakeQuota::available());
        let caller = caller();
        let scope = CredentialScope::Tenant {
            tenant_id: "tenant-a".into(),
        };
        let cloud = trust(ProviderType::CloudApi);
        assert!(
            !evaluate(
                &engine,
                &caller,
                Some(&cloud),
                &scope,
                ProviderPrivacy::PublicCloud
            )
            .allowed
        );
        assert!(
            evaluate(
                &engine,
                &caller,
                Some(&cloud),
                &scope,
                ProviderPrivacy::PrivateCloud
            )
            .allowed
        );
    }

    #[test]
    fn cost_ceiling_quota_and_budget_each_reject() {
        let patch = RoutingPolicyPatch {
            route: AiccPolicyConfig {
                max_estimated_cost: Some(LockedValue::new(Money::new(0.20, "USD"))),
                ..Default::default()
            },
            ..Default::default()
        };
        let quota = FakeQuota {
            result: Ok(QuotaSnapshot {
                state: Some(QuotaState::NearLimit),
                remaining_request_units: Some(0),
                remaining_cost: Some(Money::new(0.10, "USD")),
                reset_at: None,
            }),
            seen: Arc::new(Mutex::new(Vec::new())),
        };
        let engine = engine(&patch, quota);
        let caller = caller();
        let scope = CredentialScope::Tenant {
            tenant_id: "tenant-a".into(),
        };
        let cloud = trust(ProviderType::CloudApi);
        let decision = evaluate(
            &engine,
            &caller,
            Some(&cloud),
            &scope,
            ProviderPrivacy::PublicCloud,
        );
        for expected in [
            PolicyReasonCode::CostCeilingExceeded,
            PolicyReasonCode::RequestQuotaExceeded,
            PolicyReasonCode::BudgetExceeded,
        ] {
            assert!(decision
                .reasons
                .iter()
                .any(|reason| reason.code == expected));
        }
    }

    #[test]
    fn cost_comparison_requires_matching_currency() {
        let patch = RoutingPolicyPatch {
            route: AiccPolicyConfig {
                max_estimated_cost: Some(LockedValue::new(Money::new(1.0, "EUR"))),
                ..Default::default()
            },
            ..Default::default()
        };
        let quota = FakeQuota {
            result: Ok(QuotaSnapshot {
                state: Some(QuotaState::Normal),
                remaining_request_units: Some(10),
                remaining_cost: Some(Money::new(10.0, "GBP")),
                reset_at: None,
            }),
            seen: Arc::new(Mutex::new(Vec::new())),
        };
        let engine = engine(&patch, quota);
        let caller = caller();
        let scope = CredentialScope::Tenant {
            tenant_id: "tenant-a".into(),
        };
        let cloud = trust(ProviderType::CloudApi);
        let decision = evaluate(
            &engine,
            &caller,
            Some(&cloud),
            &scope,
            ProviderPrivacy::PublicCloud,
        );
        assert_eq!(
            decision
                .reasons
                .iter()
                .filter(|reason| reason.code == PolicyReasonCode::CostCurrencyMismatch)
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn invalid_money_fails_closed() {
        let invalid_policy = RoutingPolicyPatch {
            route: AiccPolicyConfig {
                max_estimated_cost: Some(LockedValue::new(Money::new(f64::NAN, "USD"))),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(matches!(
            EffectiveRoutingPolicy::merge(RoutingPolicyLayers {
                system: Some(&invalid_policy),
                user: None,
                app: None,
                session: None,
                request: None,
            }),
            Err(PolicyError::InvalidPolicy(_))
        ));

        let port = Arc::new(FakeTruthPort {
            result: Ok(QuotaSnapshot {
                state: Some(QuotaState::Normal),
                remaining_request_units: None,
                remaining_cost: Some(Money::new(1.0, " USD ")),
                reset_at: None,
            }),
            seen: Arc::new(Mutex::new(Vec::new())),
        });
        let factory = QuotaSourceFactory::new(port);
        assert!(matches!(
            factory
                .query_quota(
                    &caller(),
                    QuotaQueryRequest::new(Some(Capability::Llm), None),
                )
                .await,
            Err(PolicyError::QuotaSourceUnavailable)
        ));
    }

    #[test]
    fn budget_with_unknown_cost_fails_closed() {
        let patch = RoutingPolicyPatch::default();
        let engine = engine(&patch, FakeQuota::available());
        let caller = caller();
        let scope = CredentialScope::Tenant {
            tenant_id: "tenant-a".into(),
        };
        let cloud = trust(ProviderType::CloudApi);
        let mut request = RequestPolicyInput {
            caller: &caller,
            method: "chat.completions.create",
            capability: Capability::Llm,
            estimated_cost: None,
            request_units: 1,
        };
        let candidate = CandidatePolicyInput {
            provider_instance_name: "openai_primary",
            api_type: ApiType::Llm,
            logical_path: Some("llm.plan"),
            provider_privacy: ProviderPrivacy::PublicCloud,
            trust: Some(&cloud),
            credential_scope: &scope,
        };
        let decision = engine.evaluate(&request, &candidate);
        assert!(decision
            .reasons
            .iter()
            .any(|reason| reason.code == PolicyReasonCode::CostEstimateUnavailable));
        request.estimated_cost = Some(Money::new(0.1, "USD"));
        assert!(engine.evaluate(&request, &candidate).allowed);
    }

    #[test]
    fn security_truth_failures_fail_closed() {
        let quota = FakeQuota {
            result: Err(QuotaSourceError),
            seen: Arc::new(Mutex::new(Vec::new())),
        };
        let patch = RoutingPolicyPatch::default();
        let engine = engine(&patch, quota);
        let caller = caller();
        let scope = CredentialScope::Tenant {
            tenant_id: "tenant-a".into(),
        };
        let decision = evaluate(&engine, &caller, None, &scope, ProviderPrivacy::PublicCloud);
        assert!(decision
            .reasons
            .iter()
            .any(|r| r.code == PolicyReasonCode::ProviderTrustUnavailable));
        assert!(decision
            .reasons
            .iter()
            .any(|r| r.code == PolicyReasonCode::QuotaSourceUnavailable));
    }

    #[test]
    fn caller_and_credential_scopes_never_cross_tenant_or_app() {
        let source = FakeQuota::available();
        let seen = source.seen.clone();
        let patch = RoutingPolicyPatch::default();
        let engine = engine(&patch, source);
        let caller = caller();
        let scope = CredentialScope::UserApp {
            tenant_id: "tenant-b".into(),
            user_id: "user-a".into(),
            app_id: "app-b".into(),
        };
        let cloud = trust(ProviderType::CloudApi);
        let decision = evaluate(
            &engine,
            &caller,
            Some(&cloud),
            &scope,
            ProviderPrivacy::PublicCloud,
        );
        assert!(decision
            .reasons
            .iter()
            .any(|r| r.code == PolicyReasonCode::CredentialScopeMismatch));
        let lookup = seen.lock().unwrap().pop().unwrap();
        assert_eq!(lookup.caller.tenant_id, "tenant-a");
        assert_eq!(lookup.caller.app_id.as_deref(), Some("app-a"));
    }

    #[test]
    fn shared_matcher_and_quota_query_contract_are_used() {
        let patch = RoutingPolicyPatch {
            route: AiccPolicyConfig {
                allowed_provider_instances: Some(LockedValue::new(vec!["openai_*".into()])),
                blocked_provider_instances: Some(LockedValue::new(vec!["*_backup".into()])),
                ..Default::default()
            },
            ..Default::default()
        };
        let source = FakeQuota::available();
        let seen = source.seen.clone();
        let engine = engine(&patch, source);
        let response = engine
            .query_quota(
                &caller(),
                QuotaQueryRequest::new(
                    Some(Capability::Llm),
                    Some("chat.completions.create".into()),
                ),
            )
            .unwrap();
        assert_eq!(response.quota.state, QuotaState::Normal);
        assert!(seen
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .provider_instance_name
            .is_none());
    }

    #[tokio::test]
    async fn production_factory_prepares_request_scoped_provider_truth() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let factory = QuotaSourceFactory::new(Arc::new(FakeTruthPort {
            result: FakeQuota::available().result,
            seen: seen.clone(),
        }));
        let caller = caller();
        let source = factory
            .prepare_route(
                &caller,
                Capability::Llm,
                "chat.completions.create",
                ["openai_backup", "openai_primary", "openai_primary"],
            )
            .await
            .unwrap();
        let calls = seen.lock().unwrap().clone();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            calls
                .iter()
                .map(|lookup| lookup.provider_instance_name.as_deref().unwrap())
                .collect::<Vec<_>>(),
            vec!["openai_backup", "openai_primary"]
        );
        assert!(source
            .query(&QuotaLookup {
                caller: caller.clone(),
                capability: Some(Capability::Llm),
                method: Some("chat.completions.create".into()),
                provider_instance_name: Some("openai_primary".into()),
            })
            .is_ok());
        assert!(source
            .query(&QuotaLookup {
                caller: CallerIdentity {
                    tenant_id: "tenant-b".into(),
                    ..caller
                },
                capability: Some(Capability::Llm),
                method: Some("chat.completions.create".into()),
                provider_instance_name: Some("openai_primary".into()),
            })
            .is_err());
    }

    #[tokio::test]
    async fn production_factory_exposes_service_quota_query_and_fails_closed() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let factory = QuotaSourceFactory::new(Arc::new(FakeTruthPort {
            result: FakeQuota::available().result,
            seen: seen.clone(),
        }));
        let response = factory
            .query_quota(
                &caller(),
                QuotaQueryRequest::new(
                    Some(Capability::Llm),
                    Some("chat.completions.create".into()),
                ),
            )
            .await
            .unwrap();
        assert_eq!(response.quota.state, QuotaState::Normal);
        assert!(seen
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .provider_instance_name
            .is_none());
        assert!(factory
            .query_quota(
                &caller(),
                QuotaQueryRequest::new(Some(Capability::Llm), None),
            )
            .await
            .is_ok());

        let failing = QuotaSourceFactory::new(Arc::new(FakeTruthPort {
            result: Err(QuotaSourceError),
            seen: Arc::new(Mutex::new(Vec::new())),
        }));
        assert!(failing
            .prepare_route(
                &caller(),
                Capability::Llm,
                "chat.completions.create",
                ["openai_primary"],
            )
            .await
            .is_err());
    }
}
