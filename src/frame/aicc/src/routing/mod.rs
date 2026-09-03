#![allow(dead_code)]

pub(crate) mod policy;

use crate::model::{
    AdmissionRecord, CandidatePath, ExactModelName, FallbackStep, LogicalItemSource, ModelRegistry,
    ModelRegistryError, RegisteredModel, RegistryCandidate,
};
use buckyos_api::{
    features, AiccFallbackMode, AiccFallbackRule, AiccSchedulerProfile, AiccSchedulerProfileConfig,
    AiccSchedulerProfileWeights, ApiType, Capability, Feature, ModelDisable, ModelRequirement,
};
use policy::{
    CallerIdentity, CandidatePolicyInput, CredentialScope, LocalityPreference, PolicyEngine,
    PolicyReason, ProviderPrivacy, ProviderTrustView, ProviderType, QuotaSource,
    RequestPolicyInput,
};
use serde::Serialize;
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

const FALLBACK_DEPTH_LIMIT: usize = 5;
const EPSILON: f64 = 1e-12;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RouteModelKind {
    Exact,
    Logical,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProviderHealthStatus {
    Available,
    Degraded,
    Unavailable,
    CircuitOpen,
}

#[derive(Clone, Debug)]
pub(crate) struct CandidateRuntimeState {
    pub enabled: bool,
    pub credential_available: bool,
    pub model_available: bool,
    pub health: ProviderHealthStatus,
    pub provider_privacy: ProviderPrivacy,
    pub trust: Option<ProviderTrustView>,
    pub credential_scope: CredentialScope,
    pub estimated_cost_usd: Option<f64>,
    pub p95_latency_ms: Option<f64>,
    pub error_rate_5m: Option<f64>,
    pub recent_failures: u32,
    pub quality_score: Option<f64>,
    pub cache_hit_probability: Option<f64>,
}

impl CandidateRuntimeState {
    fn is_local(&self) -> bool {
        self.trust
            .as_ref()
            .is_some_and(|trust| trust.provider_type == ProviderType::LocalInference)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RoutingRequest {
    pub request_id: String,
    pub model: String,
    pub api_type: ApiType,
    pub method: String,
    pub capability: Capability,
    pub requirements: ModelRequirement,
    pub disable: ModelDisable,
    pub exact_fallback: Option<AiccFallbackRule>,
    pub previous_exact_model: Option<String>,
    pub request_units: u64,
    pub caller: CallerIdentity,
}

impl RoutingRequest {
    pub(crate) fn new(
        request_id: impl Into<String>,
        model: impl Into<String>,
        api_type: ApiType,
        caller: CallerIdentity,
    ) -> Self {
        Self {
            request_id: request_id.into(),
            model: model.into(),
            api_type,
            method: api_type.typed_method().to_owned(),
            capability: api_type.capability(),
            requirements: ModelRequirement::default(),
            disable: ModelDisable::default(),
            exact_fallback: None,
            previous_exact_model: None,
            request_units: 1,
            caller,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SelectedRoute {
    pub exact_model: String,
    pub model_uid: String,
    pub provider_instance_name: String,
    pub provider_profile_id: String,
    pub protocol_adapter_id: String,
    pub model_driver_id: String,
    pub origin_model_id: String,
    pub provider_model_id: String,
    pub operation: String,
    pub inventory_revision: String,
    pub enabled_capabilities: Vec<Feature>,
    pub disabled_capabilities: Vec<Feature>,
    pub estimated_cost_usd: Option<f64>,
    pub final_score: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RouteDecision {
    pub selected: SelectedRoute,
    pub fallback_candidates: Vec<SelectedRoute>,
    pub trace: RoutingTrace,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct RoutingTrace {
    pub request_id: String,
    pub api_type: String,
    pub requested_model: String,
    pub requested_model_type: RouteModelKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_logical_path: Option<String>,
    pub selected_exact_model: String,
    pub selected_provider_instance_name: String,
    pub candidate_count_before_filter: usize,
    pub candidate_count_after_filter: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub filtered_candidates: Vec<FilteredCandidateTrace>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ranked_candidates: Vec<RankedCandidateTrace>,
    pub fallback_applied: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fallback_chain: Vec<FallbackTraceStep>,
    pub scheduler_profile: String,
    pub score_breakdown: ScoreBreakdown,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_cost_usd: Option<f64>,
    pub runtime_failover_count: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub logical_item_sources: Vec<LogicalItemSourceTrace>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub logical_admission: Vec<LogicalAdmissionTrace>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub disabled_capabilities: Vec<Feature>,
    pub user_summary: UserFacingRouteSummary,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct FilteredCandidateTrace {
    pub exact_model: String,
    pub provider_instance_name: String,
    pub reasons: Vec<FilterReasonTrace>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct FilterReasonTrace {
    pub code: String,
    pub summary: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct RankedCandidateTrace {
    pub exact_model: String,
    pub provider_instance_name: String,
    pub priority_path: Vec<f64>,
    pub exact_model_weight: f64,
    pub provider_weight: f64,
    pub score_inputs: ScoreInputs,
    pub final_score: f64,
    pub selected: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct ScoreInputs {
    pub cost: f64,
    pub latency: f64,
    pub reliability: f64,
    pub quality: f64,
    pub preference: f64,
    pub cache: f64,
    pub local: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct ScoreBreakdown {
    pub cost: f64,
    pub latency: f64,
    pub reliability: f64,
    pub quality: f64,
    pub preference: f64,
    pub cache: f64,
    pub local: f64,
    pub final_score: f64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct FallbackTraceStep {
    pub from: String,
    pub to: String,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct LogicalItemSourceTrace {
    pub exact_model: String,
    pub source: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct LogicalAdmissionTrace {
    pub logical_path: String,
    pub exact_model: String,
    pub provider_model_id: String,
    pub admitted: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub missing_requirements: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct UserFacingRouteSummary {
    pub display_name: String,
    pub model_family: String,
    pub provider_origin: String,
    pub reason_short: String,
    pub was_fallback: bool,
    pub was_failover: bool,
}

#[derive(Debug)]
pub(crate) enum RoutingError {
    InvalidRequest(String),
    InvalidExactModel(String),
    ExactModelUnavailable {
        exact_model: String,
        reasons: Vec<FilterReasonTrace>,
    },
    NoCandidate {
        model: String,
        filtered: Vec<FilteredCandidateTrace>,
    },
    FallbackNotAllowed(String),
    InvalidFallback(String),
    FallbackLoop(String),
    FallbackDepthExceeded(usize),
    Registry(ModelRegistryError),
}

impl fmt::Display for RoutingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest(reason) => write!(formatter, "invalid route request: {reason}"),
            Self::InvalidExactModel(model) => write!(formatter, "exact model not found: {model}"),
            Self::ExactModelUnavailable { exact_model, .. } => {
                write!(formatter, "exact model unavailable: {exact_model}")
            }
            Self::NoCandidate { model, .. } => write!(formatter, "no candidate for {model}"),
            Self::FallbackNotAllowed(model) => {
                write!(formatter, "fallback is not allowed for {model}")
            }
            Self::InvalidFallback(reason) => write!(formatter, "invalid fallback: {reason}"),
            Self::FallbackLoop(model) => write!(formatter, "fallback loop at {model}"),
            Self::FallbackDepthExceeded(limit) => {
                write!(formatter, "fallback depth exceeds {limit}")
            }
            Self::Registry(error) => error.fmt(formatter),
        }
    }
}

impl Error for RoutingError {}

impl From<ModelRegistryError> for RoutingError {
    fn from(value: ModelRegistryError) -> Self {
        Self::Registry(value)
    }
}

pub(crate) struct Router<'a, Q> {
    registry: &'a ModelRegistry,
    policy: &'a PolicyEngine<Q>,
    runtime: &'a BTreeMap<String, CandidateRuntimeState>,
}

impl<'a, Q: QuotaSource> Router<'a, Q> {
    pub(crate) fn new(
        registry: &'a ModelRegistry,
        policy: &'a PolicyEngine<Q>,
        runtime: &'a BTreeMap<String, CandidateRuntimeState>,
    ) -> Self {
        Self {
            registry,
            policy,
            runtime,
        }
    }

    pub(crate) fn route(&self, request: &RoutingRequest) -> Result<RouteDecision, RoutingError> {
        validate_request(request)?;
        if request.model.contains('@') {
            self.route_exact(request)
        } else {
            self.route_logical(request, &request.model, RouteModelKind::Logical, Vec::new())
        }
    }

    fn route_exact(&self, request: &RoutingRequest) -> Result<RouteDecision, RoutingError> {
        ExactModelName::parse(&request.model)
            .map_err(|_| RoutingError::InvalidExactModel(request.model.clone()))?;
        let model = self
            .registry
            .exact_model(&request.model)
            .ok_or_else(|| RoutingError::InvalidExactModel(request.model.clone()))?;
        let evaluated = self.evaluate_candidates(
            request,
            None,
            ModelDisable::default(),
            vec![RegistryCandidate {
                model: model.clone(),
                paths: Vec::new(),
                exact_model_weight: 1.0,
                provider_weight: 1.0,
            }],
        );
        if !evaluated.allowed.is_empty() {
            return self.finish(
                request,
                RouteModelKind::Exact,
                None,
                Vec::new(),
                Vec::new(),
                evaluated,
                AiccSchedulerProfile::Balanced,
            );
        }
        let reasons = evaluated
            .filtered
            .first()
            .map(|candidate| candidate.reasons.clone())
            .unwrap_or_default();
        if !self.policy.policy().allow_exact_model_fallback.value {
            return Err(RoutingError::ExactModelUnavailable {
                exact_model: request.model.clone(),
                reasons,
            });
        }
        if !self.policy.policy().allow_fallback.value {
            return Err(RoutingError::FallbackNotAllowed(request.model.clone()));
        }
        let rule = request.exact_fallback.as_ref().ok_or_else(|| {
            RoutingError::InvalidFallback("exact fallback requires an explicit target".into())
        })?;
        let target = rule.target.as_ref().ok_or_else(|| {
            RoutingError::InvalidFallback("exact fallback target is missing".into())
        })?;
        let step = FallbackTraceStep {
            from: request.model.clone(),
            to: target.clone(),
            reason: "exact_model_unavailable".into(),
        };
        match rule.mode {
            AiccFallbackMode::TargetLogical => {
                self.route_logical(request, target, RouteModelKind::Exact, vec![step])
            }
            AiccFallbackMode::TargetExact => {
                let mut fallback_request = request.clone();
                fallback_request.model = target.clone();
                fallback_request.exact_fallback = None;
                let mut decision = self.route_exact(&fallback_request)?;
                decision.trace.requested_model = request.model.clone();
                decision.trace.fallback_applied = true;
                decision.trace.fallback_chain.insert(0, step);
                decision.trace.user_summary.was_fallback = true;
                Ok(decision)
            }
            _ => Err(RoutingError::InvalidFallback(
                "exact fallback only supports target_exact or target_logical".into(),
            )),
        }
    }

    fn route_logical(
        &self,
        request: &RoutingRequest,
        initial_path: &str,
        requested_kind: RouteModelKind,
        mut trace_chain: Vec<FallbackTraceStep>,
    ) -> Result<RouteDecision, RoutingError> {
        let mut current = initial_path.to_owned();
        let mut visited = BTreeSet::new();
        let mut all_filtered = Vec::new();
        let mut all_admissions = Vec::new();
        loop {
            if !visited.insert(current.clone()) {
                return Err(RoutingError::FallbackLoop(current));
            }
            if trace_chain.len() > FALLBACK_DEPTH_LIMIT {
                return Err(RoutingError::FallbackDepthExceeded(FALLBACK_DEPTH_LIMIT));
            }
            let set = self
                .registry
                .resolve_candidates(&current, request.api_type)?;
            append_registry_fallbacks(&mut trace_chain, &set.fallback_chain);
            if !trace_chain.is_empty() && !self.policy.policy().allow_fallback.value {
                return Err(RoutingError::FallbackNotAllowed(request.model.clone()));
            }
            all_admissions.extend(set.admissions.clone());
            let evaluated = self.evaluate_candidates(
                request,
                Some(&set.resolved_logical_path),
                set.disable_line.clone(),
                set.candidates,
            );
            if !evaluated.allowed.is_empty() {
                let mut evaluated = evaluated;
                evaluated.before_count += all_filtered.len();
                evaluated.filtered.splice(0..0, all_filtered);
                if trace_chain.len() > FALLBACK_DEPTH_LIMIT {
                    return Err(RoutingError::FallbackDepthExceeded(FALLBACK_DEPTH_LIMIT));
                }
                return self.finish(
                    request,
                    requested_kind,
                    Some(set.resolved_logical_path),
                    trace_chain,
                    all_admissions,
                    evaluated,
                    set.scheduler_profile,
                );
            }
            all_filtered.extend(evaluated.filtered);
            let Some(target) = self.next_fallback_target(&set.resolved_logical_path)? else {
                return Err(RoutingError::NoCandidate {
                    model: request.model.clone(),
                    filtered: all_filtered,
                });
            };
            if !self.policy.policy().allow_fallback.value {
                return Err(RoutingError::FallbackNotAllowed(request.model.clone()));
            }
            trace_chain.push(FallbackTraceStep {
                from: set.resolved_logical_path,
                to: target.clone(),
                reason: "all_candidates_filtered".into(),
            });
            if target.contains('@') {
                let mut fallback_request = request.clone();
                fallback_request.model = target;
                fallback_request.exact_fallback = None;
                let mut decision = self.route_exact(&fallback_request)?;
                decision.trace.requested_model = request.model.clone();
                decision.trace.requested_model_type = requested_kind;
                decision.trace.fallback_applied = true;
                decision.trace.fallback_chain = trace_chain;
                decision.trace.candidate_count_before_filter += all_filtered.len();
                decision
                    .trace
                    .filtered_candidates
                    .splice(0..0, all_filtered);
                decision.trace.user_summary.was_fallback = true;
                return Ok(decision);
            }
            current = target;
        }
    }

    fn next_fallback_target(&self, path: &str) -> Result<Option<String>, RoutingError> {
        let rule = self
            .registry
            .logical_model_views()
            .into_iter()
            .find(|view| view.path == path)
            .and_then(|view| view.fallback);
        let mode = rule
            .as_ref()
            .map(|rule| &rule.mode)
            .unwrap_or(&AiccFallbackMode::Parent);
        match mode {
            AiccFallbackMode::Strict | AiccFallbackMode::Disabled => Ok(None),
            AiccFallbackMode::Parent => {
                Ok(path.rsplit_once('.').map(|(parent, _)| parent.to_owned()))
            }
            AiccFallbackMode::TargetExact | AiccFallbackMode::TargetLogical => rule
                .and_then(|rule| rule.target)
                .map(Some)
                .ok_or_else(|| RoutingError::InvalidFallback("fallback target is missing".into())),
        }
    }

    fn evaluate_candidates(
        &self,
        request: &RoutingRequest,
        logical_path: Option<&str>,
        directory_disable: ModelDisable,
        candidates: Vec<RegistryCandidate>,
    ) -> EvaluatedCandidates {
        let before_count = candidates.len();
        let mut allowed = Vec::new();
        let mut filtered = Vec::new();
        for candidate in candidates {
            let exact_model = candidate.model.exact_model.as_str().to_owned();
            let provider = candidate.model.identity.provider_instance_name.clone();
            let mut reasons = hard_filter_model(request, &candidate.model, &directory_disable);
            let state = self.runtime.get(&exact_model);
            if let Some(state) = state {
                hard_filter_runtime(state, &mut reasons);
                let policy_request = RequestPolicyInput {
                    caller: &request.caller,
                    method: &request.method,
                    capability: request.capability.clone(),
                    estimated_cost_usd: state.estimated_cost_usd,
                    request_units: request.request_units,
                };
                let policy_candidate = CandidatePolicyInput {
                    provider_instance_name: &provider,
                    api_type: request.api_type,
                    logical_path,
                    provider_privacy: state.provider_privacy,
                    trust: state.trust.as_ref(),
                    credential_scope: &state.credential_scope,
                };
                reasons.extend(
                    self.policy
                        .evaluate(&policy_request, &policy_candidate)
                        .reasons
                        .into_iter()
                        .map(filter_reason_from_policy),
                );
            } else {
                reasons.push(filter_reason(
                    "provider_state_unavailable",
                    "provider runtime state is unavailable",
                ));
            }
            if reasons.is_empty() {
                allowed.push(PendingCandidate {
                    candidate,
                    state: state.expect("allowed candidate has runtime state").clone(),
                });
            } else {
                filtered.push(FilteredCandidateTrace {
                    exact_model,
                    provider_instance_name: provider,
                    reasons,
                });
            }
        }
        EvaluatedCandidates {
            before_count,
            allowed,
            filtered,
            directory_disable,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn finish(
        &self,
        request: &RoutingRequest,
        requested_kind: RouteModelKind,
        resolved_logical_path: Option<String>,
        fallback_chain: Vec<FallbackTraceStep>,
        admissions: Vec<AdmissionRecord>,
        evaluated: EvaluatedCandidates,
        directory_profile: AiccSchedulerProfile,
    ) -> Result<RouteDecision, RoutingError> {
        let profile = if self.policy.policy().profile.source.is_some() {
            self.policy.policy().profile.value.clone()
        } else {
            directory_profile
        };
        let candidate_count_after_filter = evaluated.allowed.len();
        let weights = scheduler_weights(
            &profile,
            self.policy.policy().scheduler_profiles.value.as_ref(),
        )?;
        let mut ranked = score_candidates(
            evaluated.allowed,
            &weights,
            request.previous_exact_model.as_deref(),
            self.policy.policy().locality_preference(),
        );
        ranked.sort_by(compare_ranked);
        let selected = ranked.first().ok_or_else(|| RoutingError::NoCandidate {
            model: request.model.clone(),
            filtered: evaluated.filtered.clone(),
        })?;
        let selected_result = selected_route(selected, request, &evaluated.directory_disable);
        let fallback_candidates = ranked
            .iter()
            .skip(1)
            .map(|candidate| selected_route(candidate, request, &evaluated.directory_disable))
            .collect();
        let selected_score = selected.score.clone();
        let was_fallback = !fallback_chain.is_empty();
        let trace = RoutingTrace {
            request_id: request.request_id.clone(),
            api_type: api_type_name(request.api_type),
            requested_model: request.model.clone(),
            requested_model_type: requested_kind,
            resolved_logical_path,
            selected_exact_model: selected_result.exact_model.clone(),
            selected_provider_instance_name: selected_result.provider_instance_name.clone(),
            candidate_count_before_filter: evaluated.before_count,
            candidate_count_after_filter,
            filtered_candidates: evaluated.filtered,
            ranked_candidates: if self.policy.policy().explain.value {
                ranked
                    .iter()
                    .enumerate()
                    .map(|(index, candidate)| ranked_trace(candidate, index == 0))
                    .collect()
            } else {
                Vec::new()
            },
            fallback_applied: was_fallback,
            fallback_chain,
            scheduler_profile: scheduler_profile_name(&profile).into(),
            score_breakdown: score_breakdown(&selected_score, &weights),
            estimated_cost_usd: selected_result.estimated_cost_usd,
            runtime_failover_count: 0,
            logical_item_sources: logical_sources(&ranked),
            logical_admission: admission_trace(admissions),
            disabled_capabilities: disabled_features(
                &evaluated.directory_disable,
                &request.disable,
            ),
            user_summary: user_summary(selected, &profile, was_fallback),
        };
        Ok(RouteDecision {
            selected: selected_result,
            fallback_candidates,
            trace,
        })
    }
}

#[derive(Clone, Debug)]
struct PendingCandidate {
    candidate: RegistryCandidate,
    state: CandidateRuntimeState,
}

#[derive(Clone, Debug)]
struct EvaluatedCandidates {
    before_count: usize,
    allowed: Vec<PendingCandidate>,
    filtered: Vec<FilteredCandidateTrace>,
    directory_disable: ModelDisable,
}

#[derive(Clone, Debug)]
struct RankedCandidate {
    pending: PendingCandidate,
    priority_path: Vec<f64>,
    score: ScoreComponents,
}

#[derive(Clone, Debug)]
struct ScoreComponents {
    inputs: ScoreInputs,
    final_score: f64,
}

fn validate_request(request: &RoutingRequest) -> Result<(), RoutingError> {
    if request.request_id.trim().is_empty() || request.model.trim().is_empty() {
        return Err(RoutingError::InvalidRequest(
            "request_id and model must not be empty".into(),
        ));
    }
    if request.method != request.api_type.typed_method() {
        return Err(RoutingError::InvalidRequest(
            "method does not match api_type".into(),
        ));
    }
    if request.capability != request.api_type.capability() {
        return Err(RoutingError::InvalidRequest(
            "capability does not match api_type".into(),
        ));
    }
    Ok(())
}

fn hard_filter_model(
    request: &RoutingRequest,
    model: &RegisteredModel,
    directory_disable: &ModelDisable,
) -> Vec<FilterReasonTrace> {
    let mut reasons = Vec::new();
    if !model.api_types.contains(&request.api_type) {
        reasons.push(filter_reason(
            "api_type_mismatch",
            "model does not support api_type",
        ));
    }
    if resolve_operation(model, request).is_none() {
        reasons.push(filter_reason(
            "operation_not_supported",
            "model has no operation for the canonical method or api_type",
        ));
    }
    for feature in request.requirements.feature_names() {
        if request.disable.disables_feature(&feature)
            || directory_disable.disables_feature(&feature)
        {
            reasons.push(filter_reason(
                "required_feature_disabled",
                format!("required feature {feature} is disabled"),
            ));
        } else if !model
            .capabilities
            .get(&feature)
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            reasons.push(filter_reason(
                "required_feature_missing",
                format!("model does not support required feature {feature}"),
            ));
        }
    }
    if let Some(required) = request.requirements.min_context_tokens {
        let available = model
            .capabilities
            .get("max_context_tokens")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        if available < required {
            reasons.push(filter_reason(
                "context_length_insufficient",
                format!("model context length is below {required}"),
            ));
        }
    }
    reasons
}

fn hard_filter_runtime(state: &CandidateRuntimeState, reasons: &mut Vec<FilterReasonTrace>) {
    if !state.enabled {
        reasons.push(filter_reason("provider_disabled", "provider is disabled"));
    }
    if !state.credential_available {
        reasons.push(filter_reason(
            "credential_unavailable",
            "provider credential is unavailable",
        ));
    }
    if !state.model_available {
        reasons.push(filter_reason(
            "model_unavailable",
            "provider model is unavailable",
        ));
    }
    match state.health {
        ProviderHealthStatus::Unavailable => reasons.push(filter_reason(
            "health_unavailable",
            "provider health is unavailable",
        )),
        ProviderHealthStatus::CircuitOpen => reasons.push(filter_reason(
            "circuit_open",
            "provider circuit breaker is open",
        )),
        ProviderHealthStatus::Available | ProviderHealthStatus::Degraded => {}
    }
}

fn resolve_operation<'a>(model: &'a RegisteredModel, request: &RoutingRequest) -> Option<&'a str> {
    model
        .operations
        .get(&request.method)
        .or_else(|| model.operations.get(&api_type_name(request.api_type)))
        .map(String::as_str)
        .filter(|operation| !operation.trim().is_empty())
}

fn best_path(paths: &[CandidatePath]) -> Option<&Vec<f64>> {
    paths
        .iter()
        .map(|path| &path.priority)
        .max_by(|left, right| compare_priority(left, right))
}

fn compare_priority(left: &[f64], right: &[f64]) -> Ordering {
    for (left, right) in left.iter().zip(right) {
        let order = left.partial_cmp(right).unwrap_or(Ordering::Equal);
        if order != Ordering::Equal {
            return order;
        }
    }
    left.len().cmp(&right.len())
}

fn scheduler_weights(
    profile: &AiccSchedulerProfile,
    config: Option<&AiccSchedulerProfileConfig>,
) -> Result<AiccSchedulerProfileWeights, RoutingError> {
    let configured = config.and_then(|config| match profile {
        AiccSchedulerProfile::CostFirst => config.cost_first.as_ref(),
        AiccSchedulerProfile::LatencyFirst => config.latency_first.as_ref(),
        AiccSchedulerProfile::QualityFirst => config.quality_first.as_ref(),
        AiccSchedulerProfile::Balanced => config.balanced.as_ref(),
        AiccSchedulerProfile::LocalFirst => config.local_first.as_ref(),
        AiccSchedulerProfile::StrictLocal => config.strict_local.as_ref(),
    });
    let weights = configured
        .cloned()
        .unwrap_or_else(|| default_weights(profile));
    let values = [
        weights.cost,
        weights.latency,
        weights.reliability,
        weights.quality,
        weights.preference,
        weights.cache,
        weights.local,
    ];
    if values
        .iter()
        .any(|weight| !weight.is_finite() || *weight < 0.0)
        || values.iter().all(|weight| *weight == 0.0)
    {
        return Err(RoutingError::InvalidRequest(
            "scheduler weights must be finite, non-negative, and not all zero".into(),
        ));
    }
    Ok(weights)
}

fn default_weights(profile: &AiccSchedulerProfile) -> AiccSchedulerProfileWeights {
    let values = match profile {
        AiccSchedulerProfile::CostFirst => (0.55, 0.15, 0.15, 0.10, 0.05, 0.0, 0.0),
        AiccSchedulerProfile::LatencyFirst => (0.10, 0.55, 0.15, 0.10, 0.05, 0.05, 0.0),
        AiccSchedulerProfile::QualityFirst => (0.10, 0.10, 0.15, 0.55, 0.10, 0.0, 0.0),
        AiccSchedulerProfile::Balanced => (0.25, 0.20, 0.20, 0.25, 0.10, 0.0, 0.0),
        AiccSchedulerProfile::LocalFirst => (0.10, 0.10, 0.10, 0.10, 0.05, 0.0, 0.55),
        AiccSchedulerProfile::StrictLocal => (0.20, 0.15, 0.20, 0.25, 0.20, 0.0, 0.0),
    };
    AiccSchedulerProfileWeights {
        cost: values.0,
        latency: values.1,
        reliability: values.2,
        quality: values.3,
        preference: values.4,
        cache: values.5,
        local: values.6,
    }
}

fn score_candidates(
    candidates: Vec<PendingCandidate>,
    weights: &AiccSchedulerProfileWeights,
    previous_exact_model: Option<&str>,
    locality: LocalityPreference,
) -> Vec<RankedCandidate> {
    let costs = candidates
        .iter()
        .map(|candidate| candidate.state.estimated_cost_usd)
        .collect::<Vec<_>>();
    let latencies = candidates
        .iter()
        .map(|candidate| candidate.state.p95_latency_ms)
        .collect::<Vec<_>>();
    let qualities = candidates
        .iter()
        .map(|candidate| candidate.state.quality_score)
        .collect::<Vec<_>>();
    let cache = candidates
        .iter()
        .map(|candidate| candidate.state.cache_hit_probability)
        .collect::<Vec<_>>();
    let provider_weights = candidates
        .iter()
        .map(|candidate| Some(candidate.candidate.provider_weight))
        .collect::<Vec<_>>();
    let cost_scores = normalize(&costs, false);
    let latency_scores = normalize(&latencies, false);
    let quality_scores = normalize(&qualities, true);
    let cache_scores = normalize(&cache, true);
    let provider_scores = normalize(&provider_weights, true);
    candidates
        .into_iter()
        .enumerate()
        .map(|(index, pending)| {
            let exact = pending.candidate.model.exact_model.as_str();
            let history =
                previous_exact_model.map(|previous| if previous == exact { 0.0 } else { 1.0 });
            let preference = history
                .map(|history| (provider_scores[index] + history) / 2.0)
                .unwrap_or(provider_scores[index]);
            let mut reliability = pending
                .state
                .error_rate_5m
                .filter(|value| (0.0..=1.0).contains(value))
                .unwrap_or(1.0);
            reliability =
                (reliability + f64::from(pending.state.recent_failures).min(10.0) / 10.0).min(1.0);
            if pending.state.health == ProviderHealthStatus::Degraded {
                reliability = reliability.max(0.5);
            }
            let local = match locality {
                LocalityPreference::PreferLocal if !pending.state.is_local() => 1.0,
                _ => 0.0,
            };
            let inputs = ScoreInputs {
                cost: cost_scores[index],
                latency: latency_scores[index],
                reliability,
                quality: quality_scores[index],
                preference,
                cache: cache_scores[index],
                local,
            };
            let final_score = weighted_score(&inputs, weights);
            let priority_path = best_path(&pending.candidate.paths)
                .cloned()
                .unwrap_or_default();
            RankedCandidate {
                pending,
                priority_path,
                score: ScoreComponents {
                    inputs,
                    final_score,
                },
            }
        })
        .collect()
}

fn normalize(values: &[Option<f64>], invert: bool) -> Vec<f64> {
    let valid = values
        .iter()
        .flatten()
        .copied()
        .filter(|value| value.is_finite() && *value >= 0.0)
        .collect::<Vec<_>>();
    let min = valid.iter().copied().reduce(f64::min);
    let max = valid.iter().copied().reduce(f64::max);
    values
        .iter()
        .map(|value| match (value, min, max) {
            (Some(value), Some(min), Some(max)) if value.is_finite() && *value >= 0.0 => {
                let normalized = if (max - min).abs() < EPSILON {
                    if invert {
                        1.0
                    } else {
                        0.0
                    }
                } else {
                    (*value - min) / (max - min)
                };
                if invert {
                    1.0 - normalized
                } else {
                    normalized
                }
            }
            _ => 1.0,
        })
        .collect()
}

fn weighted_score(inputs: &ScoreInputs, weights: &AiccSchedulerProfileWeights) -> f64 {
    inputs.cost * weights.cost
        + inputs.latency * weights.latency
        + inputs.reliability * weights.reliability
        + inputs.quality * weights.quality
        + inputs.preference * weights.preference
        + inputs.cache * weights.cache
        + inputs.local * weights.local
}

fn compare_ranked(left: &RankedCandidate, right: &RankedCandidate) -> Ordering {
    compare_priority(&right.priority_path, &left.priority_path)
        .then_with(|| {
            right
                .pending
                .candidate
                .exact_model_weight
                .partial_cmp(&left.pending.candidate.exact_model_weight)
                .unwrap_or(Ordering::Equal)
        })
        .then_with(|| {
            left.score
                .final_score
                .partial_cmp(&right.score.final_score)
                .unwrap_or(Ordering::Equal)
        })
        .then_with(|| {
            left.pending
                .candidate
                .model
                .exact_model
                .as_str()
                .cmp(right.pending.candidate.model.exact_model.as_str())
        })
}

fn selected_route(
    candidate: &RankedCandidate,
    request: &RoutingRequest,
    directory_disable: &ModelDisable,
) -> SelectedRoute {
    let model = &candidate.pending.candidate.model;
    SelectedRoute {
        exact_model: model.exact_model.as_str().into(),
        model_uid: model.model_uid.as_stable_string(),
        provider_instance_name: model.identity.provider_instance_name.clone(),
        provider_profile_id: model.identity.provider_profile_id.clone(),
        protocol_adapter_id: model.identity.protocol_adapter_id.clone(),
        model_driver_id: model.identity.model_driver_id.clone(),
        origin_model_id: model.identity.origin_model_id.clone(),
        provider_model_id: model.identity.provider_model_id.clone(),
        operation: resolve_operation(model, request)
            .expect("ranked model has operation")
            .into(),
        inventory_revision: model.inventory_revision.clone(),
        enabled_capabilities: enabled_features(model, directory_disable, &request.disable),
        disabled_capabilities: disabled_features(directory_disable, &request.disable),
        estimated_cost_usd: candidate.pending.state.estimated_cost_usd,
        final_score: candidate.score.final_score,
    }
}

fn enabled_features(
    model: &RegisteredModel,
    directory_disable: &ModelDisable,
    request_disable: &ModelDisable,
) -> Vec<Feature> {
    let mut result = model
        .capabilities
        .iter()
        .filter_map(|(name, value)| {
            value
                .as_bool()
                .filter(|supported| *supported)
                .filter(|_| !directory_disable.disables_feature(name))
                .filter(|_| !request_disable.disables_feature(name))
                .map(|_| name.clone())
        })
        .collect::<Vec<_>>();
    result.sort();
    result
}

fn disabled_features(directory: &ModelDisable, request: &ModelDisable) -> Vec<Feature> {
    [
        ("streaming", directory.streaming || request.streaming),
        (
            features::TOOL_CALLING,
            directory.tool_call || request.tool_call,
        ),
        (
            features::JSON_OUTPUT,
            directory.json_schema || request.json_schema,
        ),
        (
            features::WEB_SEARCH,
            directory.web_search || request.web_search,
        ),
        (features::VISION, directory.vision || request.vision),
        (
            features::IMAGE_GENERATION,
            directory.image_generation || request.image_generation,
        ),
    ]
    .into_iter()
    .filter(|(_, disabled)| *disabled)
    .map(|(name, _)| name.to_owned())
    .collect()
}

fn ranked_trace(candidate: &RankedCandidate, selected: bool) -> RankedCandidateTrace {
    RankedCandidateTrace {
        exact_model: candidate
            .pending
            .candidate
            .model
            .exact_model
            .as_str()
            .into(),
        provider_instance_name: candidate
            .pending
            .candidate
            .model
            .identity
            .provider_instance_name
            .clone(),
        priority_path: candidate.priority_path.clone(),
        exact_model_weight: candidate.pending.candidate.exact_model_weight,
        provider_weight: candidate.pending.candidate.provider_weight,
        score_inputs: candidate.score.inputs.clone(),
        final_score: candidate.score.final_score,
        selected,
    }
}

fn score_breakdown(
    score: &ScoreComponents,
    weights: &AiccSchedulerProfileWeights,
) -> ScoreBreakdown {
    ScoreBreakdown {
        cost: score.inputs.cost * weights.cost,
        latency: score.inputs.latency * weights.latency,
        reliability: score.inputs.reliability * weights.reliability,
        quality: score.inputs.quality * weights.quality,
        preference: score.inputs.preference * weights.preference,
        cache: score.inputs.cache * weights.cache,
        local: score.inputs.local * weights.local,
        final_score: score.final_score,
    }
}

fn logical_sources(candidates: &[RankedCandidate]) -> Vec<LogicalItemSourceTrace> {
    let mut result = BTreeSet::new();
    for candidate in candidates {
        let exact = candidate.pending.candidate.model.exact_model.as_str();
        for path in &candidate.pending.candidate.paths {
            for source in &path.sources {
                result.insert((
                    exact.to_owned(),
                    logical_item_source_name(*source).to_owned(),
                ));
            }
        }
    }
    result
        .into_iter()
        .map(|(exact_model, source)| LogicalItemSourceTrace {
            exact_model,
            source,
        })
        .collect()
}

fn admission_trace(admissions: Vec<AdmissionRecord>) -> Vec<LogicalAdmissionTrace> {
    admissions
        .into_iter()
        .map(|admission| {
            let provider_model_id = admission
                .exact_model
                .split_once('@')
                .map(|(model, _)| model.split_once(':').map_or(model, |(base, _)| base))
                .unwrap_or(&admission.exact_model)
                .to_owned();
            LogicalAdmissionTrace {
                logical_path: admission.logical_path,
                exact_model: admission.exact_model,
                provider_model_id,
                admitted: admission.admitted,
                missing_requirements: admission.missing_requirements,
            }
        })
        .collect()
}

fn user_summary(
    selected: &RankedCandidate,
    profile: &AiccSchedulerProfile,
    was_fallback: bool,
) -> UserFacingRouteSummary {
    let model = &selected.pending.candidate.model;
    let provider_origin = if selected.pending.state.is_local() {
        "local"
    } else if selected
        .pending
        .state
        .trust
        .as_ref()
        .is_some_and(|trust| trust.provider_type == ProviderType::CloudApi)
    {
        "cloud"
    } else {
        "proxy_unknown"
    };
    let reason = match profile {
        AiccSchedulerProfile::CostFirst => "同优先级内成本最低",
        AiccSchedulerProfile::LatencyFirst => "按最低延迟策略选择",
        AiccSchedulerProfile::QualityFirst => "按最高质量策略选择",
        AiccSchedulerProfile::Balanced => "按均衡策略选择",
        AiccSchedulerProfile::LocalFirst => "按本地优先策略选择",
        AiccSchedulerProfile::StrictLocal => "高隐私策略只允许本地 Provider",
    };
    UserFacingRouteSummary {
        display_name: model.exact_model.as_str().into(),
        model_family: model.identity.model_driver_id.clone(),
        provider_origin: provider_origin.into(),
        reason_short: reason.into(),
        was_fallback,
        was_failover: false,
    }
}

fn filter_reason(code: impl Into<String>, summary: impl Into<String>) -> FilterReasonTrace {
    FilterReasonTrace {
        code: code.into(),
        summary: summary.into(),
    }
}

fn filter_reason_from_policy(reason: PolicyReason) -> FilterReasonTrace {
    filter_reason(reason.code.as_str(), reason.summary)
}

fn append_registry_fallbacks(trace: &mut Vec<FallbackTraceStep>, steps: &[FallbackStep]) {
    trace.extend(steps.iter().map(|step| FallbackTraceStep {
        from: step.from.clone(),
        to: step.to.clone(),
        reason: "no_registry_candidate".into(),
    }));
}

fn api_type_name(api_type: ApiType) -> String {
    serde_json::to_value(api_type)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .expect("ApiType serialization is a string")
}

fn scheduler_profile_name(profile: &AiccSchedulerProfile) -> &'static str {
    match profile {
        AiccSchedulerProfile::CostFirst => "cost_first",
        AiccSchedulerProfile::LatencyFirst => "latency_first",
        AiccSchedulerProfile::QualityFirst => "quality_first",
        AiccSchedulerProfile::Balanced => "balanced",
        AiccSchedulerProfile::LocalFirst => "local_first",
        AiccSchedulerProfile::StrictLocal => "strict_local",
    }
}

fn logical_item_source_name(source: LogicalItemSource) -> &'static str {
    match source {
        LogicalItemSource::BuiltinDefinition => "builtin_definition",
        LogicalItemSource::DriverMetadataMount => "driver_metadata_mount",
        LogicalItemSource::AutoAdmission => "auto_admission",
        LogicalItemSource::ManualOverride => "manual_override",
        LogicalItemSource::UserOverlay => "user_overlay",
        LogicalItemSource::SessionOverlay => "session_overlay",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{
        CatalogBuildOptions, CatalogDocuments, CatalogSnapshot, ModelDriverCatalog,
    };
    use crate::model::{
        InventoryModel, LogicalModelDefinition, MountMode, ProviderInventory, RegistryLayers,
    };
    use buckyos_api::{
        AiccLogicalNodeOverlay, AiccPolicyConfig, AiccRouteOverlay, LockedValue, ModelItem,
        QuotaState,
    };
    use policy::{
        EffectiveRoutingPolicy, ProviderTrustLevel, ProviderTypeSource, QuotaLookup, QuotaSnapshot,
        QuotaSourceError, RoutingPolicyLayers, RoutingPolicyPatch,
    };
    use serde_json::json;

    #[derive(Clone)]
    struct OpenQuota;

    impl QuotaSource for OpenQuota {
        fn query(&self, _lookup: &QuotaLookup) -> Result<QuotaSnapshot, QuotaSourceError> {
            Ok(QuotaSnapshot {
                state: Some(QuotaState::Normal),
                remaining_request_units: Some(100),
                remaining_cost_usd: None,
                reset_at: None,
            })
        }
    }

    fn catalog() -> CatalogSnapshot {
        let driver: ModelDriverCatalog = serde_json::from_value(json!({
            "format": "buckyos.aicc.model-driver-catalog",
            "schema_version": 1,
            "schema_revision": 0,
            "model_driver_id": "driver",
            "revision_seq": 1,
            "models": [],
            "patterns": [{
                "match": "*",
                "api_types": ["llm"],
                "capabilities": {"streaming": true}
            }],
            "defaults": {},
            "variants": [],
            "version_rules": []
        }))
        .unwrap();
        CatalogSnapshot::build(
            1,
            CatalogDocuments {
                model_drivers: vec![driver],
                ..CatalogDocuments::default()
            },
            &CatalogBuildOptions::default(),
        )
        .unwrap()
    }

    fn inventory(instance: &str, id: &str, tool_calling: bool) -> ProviderInventory {
        ProviderInventory {
            provider_instance_name: instance.into(),
            provider_profile_id: "profile".into(),
            protocol_adapter_id: "adapter".into(),
            inventory_revision: format!("rev-{instance}"),
            models: vec![InventoryModel {
                provider_model_id: id.into(),
                model_driver_id: "driver".into(),
                origin_model_id: id.into(),
                api_types: vec![ApiType::Llm],
                logical_mounts: Vec::new(),
                variants: Vec::new(),
                capabilities: BTreeMap::from([
                    ("tool_calling".into(), json!(tool_calling)),
                    ("max_context_tokens".into(), json!(128_000)),
                ]),
                attributes: BTreeMap::new(),
                operations: BTreeMap::from([(
                    "chat.completions.create".into(),
                    "responses.create".into(),
                )]),
            }],
        }
    }

    fn definition(
        path: &str,
        fallback: AiccFallbackMode,
        profile: AiccSchedulerProfile,
    ) -> LogicalModelDefinition {
        LogicalModelDefinition {
            path: path.into(),
            api_type: ApiType::Llm,
            min_line: ModelRequirement::default(),
            disable_line: ModelDisable::default(),
            default_options: BTreeMap::new(),
            mount_mode: MountMode::Manual,
            scheduler_profile: profile,
            fallback: Some(AiccFallbackRule {
                mode: fallback,
                target: None,
            }),
            route_policy: AiccPolicyConfig::default(),
            user_visible_tier: None,
        }
    }

    fn node(items: &[(&str, &str, f64)]) -> AiccLogicalNodeOverlay {
        AiccLogicalNodeOverlay {
            items: Some(
                items
                    .iter()
                    .map(|(name, target, weight)| {
                        ((*name).into(), ModelItem::new(*target, *weight))
                    })
                    .collect(),
            ),
            ..AiccLogicalNodeOverlay::default()
        }
    }

    fn registry(profile: AiccSchedulerProfile) -> ModelRegistry {
        let inventories = vec![
            inventory("cloud-a", "cheap", true),
            inventory("cloud-b", "fast", false),
            inventory("local", "local", true),
        ];
        let mut weighted = node(&[("a", "cheap@cloud-a", 1.0), ("b", "fast@cloud-b", 1.0)]);
        weighted
            .exact_model_weights
            .insert("fast@cloud-b".into(), 2.0);
        let factory = AiccRouteOverlay {
            logical_tree: BTreeMap::from([
                ("llm".into(), node(&[("fallback", "cheap@cloud-a", 1.0)])),
                (
                    "llm.plan".into(),
                    node(&[
                        ("preferred", "llm.family", 3.0),
                        ("backup", "local@local", 2.0),
                    ]),
                ),
                (
                    "llm.family".into(),
                    node(&[("a", "cheap@cloud-a", 1.0), ("b", "fast@cloud-b", 1.0)]),
                ),
                ("llm.special".into(), node(&[("only", "fast@cloud-b", 1.0)])),
                (
                    "llm.all".into(),
                    node(&[
                        ("cloud", "cheap@cloud-a", 1.0),
                        ("local", "local@local", 1.0),
                    ]),
                ),
                ("llm.weighted".into(), weighted),
            ]),
            ..AiccRouteOverlay::default()
        };
        ModelRegistry::build(
            &catalog(),
            &inventories,
            vec![
                definition("llm", AiccFallbackMode::Strict, profile.clone()),
                definition("llm.plan", AiccFallbackMode::Parent, profile.clone()),
                definition("llm.family", AiccFallbackMode::Strict, profile.clone()),
                definition("llm.special", AiccFallbackMode::Parent, profile.clone()),
                definition(
                    "llm.all",
                    AiccFallbackMode::Strict,
                    AiccSchedulerProfile::Balanced,
                ),
                definition(
                    "llm.weighted",
                    AiccFallbackMode::Strict,
                    AiccSchedulerProfile::CostFirst,
                ),
            ],
            RegistryLayers {
                factory: Some(&factory),
                ..RegistryLayers::default()
            },
        )
        .unwrap()
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

    fn state(local: bool, cost: f64, latency: f64, quality: f64) -> CandidateRuntimeState {
        CandidateRuntimeState {
            enabled: true,
            credential_available: true,
            model_available: true,
            health: ProviderHealthStatus::Available,
            provider_privacy: if local {
                ProviderPrivacy::Local
            } else {
                ProviderPrivacy::PublicCloud
            },
            trust: Some(trust(if local {
                ProviderType::LocalInference
            } else {
                ProviderType::CloudApi
            })),
            credential_scope: CredentialScope::Tenant {
                tenant_id: "tenant".into(),
            },
            estimated_cost_usd: Some(cost),
            p95_latency_ms: Some(latency),
            error_rate_5m: Some(0.0),
            recent_failures: 0,
            quality_score: Some(quality),
            cache_hit_probability: None,
        }
    }

    fn runtime() -> BTreeMap<String, CandidateRuntimeState> {
        BTreeMap::from([
            ("cheap@cloud-a".into(), state(false, 0.1, 500.0, 0.7)),
            ("fast@cloud-b".into(), state(false, 0.3, 100.0, 0.8)),
            ("local@local".into(), state(true, 0.2, 300.0, 0.9)),
        ])
    }

    fn request(model: &str) -> RoutingRequest {
        RoutingRequest::new(
            "request-1",
            model,
            ApiType::Llm,
            CallerIdentity {
                tenant_id: "tenant".into(),
                user_id: "user".into(),
                app_id: Some("app".into()),
            },
        )
    }

    fn engine(patch: &RoutingPolicyPatch) -> PolicyEngine<OpenQuota> {
        let effective = EffectiveRoutingPolicy::merge(RoutingPolicyLayers {
            system: Some(patch),
            user: None,
            app: None,
            session: None,
            request: None,
        })
        .unwrap();
        PolicyEngine::new(effective, OpenQuota).unwrap()
    }

    fn route(
        profile: AiccSchedulerProfile,
        patch: &RoutingPolicyPatch,
        request: &RoutingRequest,
        runtime: &BTreeMap<String, CandidateRuntimeState>,
    ) -> Result<RouteDecision, RoutingError> {
        let registry = registry(profile);
        let engine = engine(patch);
        Router::new(&registry, &engine, runtime).route(request)
    }

    #[test]
    fn logical_route_honors_branch_weight_before_scheduler() {
        let decision = route(
            AiccSchedulerProfile::CostFirst,
            &RoutingPolicyPatch::default(),
            &request("llm.plan"),
            &runtime(),
        )
        .unwrap();
        assert_eq!(decision.selected.exact_model, "cheap@cloud-a");
        assert_eq!(decision.fallback_candidates.len(), 2);
        assert_eq!(decision.fallback_candidates[1].exact_model, "local@local");
    }

    #[test]
    fn exact_route_does_not_fallback_by_default() {
        let mut runtime = runtime();
        runtime.get_mut("fast@cloud-b").unwrap().health = ProviderHealthStatus::Unavailable;
        let error = route(
            AiccSchedulerProfile::Balanced,
            &RoutingPolicyPatch::default(),
            &request("fast@cloud-b"),
            &runtime,
        )
        .unwrap_err();
        assert!(matches!(error, RoutingError::ExactModelUnavailable { .. }));
    }

    #[test]
    fn exact_route_fallback_requires_explicit_policy_and_target() {
        let mut runtime = runtime();
        runtime.get_mut("fast@cloud-b").unwrap().health = ProviderHealthStatus::Unavailable;
        let patch = RoutingPolicyPatch {
            route: AiccPolicyConfig {
                allow_exact_model_fallback: Some(LockedValue::new(true)),
                ..AiccPolicyConfig::default()
            },
            ..RoutingPolicyPatch::default()
        };
        let mut request = request("fast@cloud-b");
        request.exact_fallback = Some(AiccFallbackRule {
            mode: AiccFallbackMode::TargetLogical,
            target: Some("llm".into()),
        });
        let decision = route(AiccSchedulerProfile::Balanced, &patch, &request, &runtime).unwrap();
        assert_eq!(decision.selected.exact_model, "cheap@cloud-a");
        assert!(decision.trace.fallback_applied);
    }

    #[test]
    fn exact_model_weight_precedes_profile_score() {
        let decision = route(
            AiccSchedulerProfile::CostFirst,
            &RoutingPolicyPatch::default(),
            &request("llm.weighted"),
            &runtime(),
        )
        .unwrap();
        assert_eq!(decision.selected.exact_model, "fast@cloud-b");
    }

    #[test]
    fn hard_filters_feature_health_and_allow_list() {
        let mut request = request("llm.family");
        request.requirements.tool_call = true;
        let patch = RoutingPolicyPatch {
            route: AiccPolicyConfig {
                allowed_provider_instances: Some(LockedValue::new(vec!["cloud-*".into()])),
                ..AiccPolicyConfig::default()
            },
            ..RoutingPolicyPatch::default()
        };
        let decision = route(AiccSchedulerProfile::Balanced, &patch, &request, &runtime()).unwrap();
        assert_eq!(decision.selected.exact_model, "cheap@cloud-a");
    }

    #[test]
    fn budget_and_block_rules_are_applied_before_weight_and_score() {
        let patch = RoutingPolicyPatch {
            route: AiccPolicyConfig {
                blocked_provider_instances: Some(LockedValue::new(vec!["cloud-a".into()])),
                max_estimated_cost_usd: Some(LockedValue::new(0.25)),
                ..AiccPolicyConfig::default()
            },
            ..RoutingPolicyPatch::default()
        };
        let decision = route(
            AiccSchedulerProfile::CostFirst,
            &patch,
            &request("llm.plan"),
            &runtime(),
        )
        .unwrap();
        assert_eq!(decision.selected.exact_model, "local@local");
        let codes = decision
            .trace
            .filtered_candidates
            .iter()
            .flat_map(|candidate| candidate.reasons.iter().map(|reason| reason.code.as_str()))
            .collect::<Vec<_>>();
        assert!(codes.contains(&"provider_blocked"));
        assert!(codes.contains(&"cost_ceiling_exceeded"));
    }

    #[test]
    fn all_filtered_candidates_fallback_to_parent() {
        let mut runtime = runtime();
        runtime.get_mut("fast@cloud-b").unwrap().health = ProviderHealthStatus::Unavailable;
        let decision = route(
            AiccSchedulerProfile::Balanced,
            &RoutingPolicyPatch::default(),
            &request("llm.special"),
            &runtime,
        )
        .unwrap();
        assert_eq!(decision.selected.exact_model, "cheap@cloud-a");
        assert!(decision.trace.fallback_applied);
        assert_eq!(decision.trace.fallback_chain[0].from, "llm.special");
    }

    #[test]
    fn scheduler_profiles_choose_expected_candidate() {
        for (profile, expected) in [
            (AiccSchedulerProfile::CostFirst, "cheap@cloud-a"),
            (AiccSchedulerProfile::LatencyFirst, "fast@cloud-b"),
            (AiccSchedulerProfile::QualityFirst, "fast@cloud-b"),
        ] {
            let decision = route(
                profile,
                &RoutingPolicyPatch::default(),
                &request("llm.family"),
                &runtime(),
            )
            .unwrap();
            assert_eq!(decision.selected.exact_model, expected);
        }
    }

    #[test]
    fn local_and_strict_local_profiles_work() {
        for profile in [
            AiccSchedulerProfile::LocalFirst,
            AiccSchedulerProfile::StrictLocal,
        ] {
            let patch = RoutingPolicyPatch {
                route: AiccPolicyConfig {
                    profile: Some(LockedValue::new(profile.clone())),
                    ..AiccPolicyConfig::default()
                },
                ..RoutingPolicyPatch::default()
            };
            let model = if profile == AiccSchedulerProfile::LocalFirst {
                "llm.all"
            } else {
                "llm.plan"
            };
            let decision = route(
                AiccSchedulerProfile::Balanced,
                &patch,
                &request(model),
                &runtime(),
            )
            .unwrap();
            assert_eq!(decision.selected.exact_model, "local@local");
        }
    }

    #[test]
    fn session_history_is_soft_and_cannot_bypass_health() {
        let mut request = request("llm.family");
        request.previous_exact_model = Some("fast@cloud-b".into());
        let patch = RoutingPolicyPatch {
            route: AiccPolicyConfig {
                profile: Some(LockedValue::new(AiccSchedulerProfile::Balanced)),
                scheduler_profiles: Some(LockedValue::new(AiccSchedulerProfileConfig {
                    balanced: Some(AiccSchedulerProfileWeights {
                        preference: 1.0,
                        ..AiccSchedulerProfileWeights::default()
                    }),
                    ..AiccSchedulerProfileConfig::default()
                })),
                ..AiccPolicyConfig::default()
            },
            ..RoutingPolicyPatch::default()
        };
        let decision = route(AiccSchedulerProfile::Balanced, &patch, &request, &runtime()).unwrap();
        assert_eq!(decision.selected.exact_model, "fast@cloud-b");

        let mut runtime = runtime();
        runtime.get_mut("fast@cloud-b").unwrap().health = ProviderHealthStatus::Unavailable;
        let decision = route(AiccSchedulerProfile::Balanced, &patch, &request, &runtime).unwrap();
        assert_eq!(decision.selected.exact_model, "cheap@cloud-a");
    }

    #[test]
    fn tie_break_is_deterministic_and_trace_is_redacted() {
        let patch = RoutingPolicyPatch {
            route: AiccPolicyConfig {
                profile: Some(LockedValue::new(AiccSchedulerProfile::Balanced)),
                explain: Some(LockedValue::new(true)),
                scheduler_profiles: Some(LockedValue::new(AiccSchedulerProfileConfig {
                    balanced: Some(AiccSchedulerProfileWeights {
                        preference: 1.0,
                        ..AiccSchedulerProfileWeights::default()
                    }),
                    ..AiccSchedulerProfileConfig::default()
                })),
                ..AiccPolicyConfig::default()
            },
            ..RoutingPolicyPatch::default()
        };
        let decision = route(
            AiccSchedulerProfile::Balanced,
            &patch,
            &request("llm.family"),
            &runtime(),
        )
        .unwrap();
        assert_eq!(decision.selected.exact_model, "cheap@cloud-a");
        assert_eq!(decision.trace.ranked_candidates.len(), 2);
        let trace = serde_json::to_string(&decision.trace).unwrap();
        for forbidden in ["prompt", "credential", "secret", "options"] {
            assert!(!trace.contains(forbidden));
        }
    }

    #[test]
    fn method_and_capability_must_match_api_type() {
        let mut bad_method = request("llm.family");
        bad_method.method = "images.generate".into();
        assert!(matches!(
            route(
                AiccSchedulerProfile::Balanced,
                &RoutingPolicyPatch::default(),
                &bad_method,
                &runtime()
            ),
            Err(RoutingError::InvalidRequest(_))
        ));

        let mut bad_capability = request("llm.family");
        bad_capability.capability = Capability::Image;
        assert!(matches!(
            route(
                AiccSchedulerProfile::Balanced,
                &RoutingPolicyPatch::default(),
                &bad_capability,
                &runtime()
            ),
            Err(RoutingError::InvalidRequest(_))
        ));
    }
}
