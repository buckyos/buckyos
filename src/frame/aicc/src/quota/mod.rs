//! Token quota (product/ai_center/AI_Center_Quota_PRD.md §4–§8): rule scopes,
//! the live state of periodic windows and total accounts, the budget an
//! upstream attempt reserves and the denial details of §7.3.
//!
//! The durable ledger is in `storage::quota`; this module holds the rules
//! that do not touch the database.

use buckyos_api::{
    ApiType, QuotaBlockingRule, QuotaDenialDetails, QuotaKind, QuotaObject, QuotaPeriod,
    QuotaResource, QuotaRule, QuotaStatus, QuotaUsageState, ResourceRef,
    QUOTA_NEAR_LIMIT_PERCENT,
};
use serde_json::Value;

use crate::call::ResolvedProviderCall;

/// A reconciled attempt that never reported usage is charged its full
/// reservation after this long (§7.2: one hour for an ordinary call).
pub(crate) const RECONCILE_AFTER_MS: i64 = 60 * 60 * 1000;
/// Native Provider tasks poll for at most 30 minutes; their reservation is
/// reconciled one hour after that deadline.
pub(crate) const LONG_TASK_TIMEOUT_MS: i64 = 30 * 60 * 1000;
/// Tokens reserved for each image, audio or video input of a metered call.
const MEDIA_INPUT_TOKENS: u64 = 4_096;
/// Framing of each message (role, separators) on top of its content.
const MESSAGE_OVERHEAD_TOKENS: u64 = 16;
/// Output budget used when neither the call nor the model metadata bounds it.
const FALLBACK_OUTPUT_TOKENS: u64 = 8_192;

/// Separator of scope key parts; never part of a user id, App id or model.
const KEY_SEPARATOR: char = '\u{1f}';

/// Only these API types report a reliable token count and have a budget
/// that bounds their consumption; other operations are not under token quota.
pub(crate) fn is_metered(api_type: ApiType) -> bool {
    matches!(
        api_type,
        ApiType::Llm | ApiType::Decision | ApiType::EmbeddingText | ApiType::EmbeddingMultimodal
    )
}

pub(crate) fn object_parts(object: &QuotaObject) -> (&'static str, &str, &str) {
    match object {
        QuotaObject::User { user_id } => ("user", user_id.as_str(), ""),
        QuotaObject::App { app_id } => ("app", "", app_id.as_str()),
        QuotaObject::UserApp { user_id, app_id } => ("user_app", user_id.as_str(), app_id.as_str()),
    }
}

pub(crate) fn resource_parts(resource: &QuotaResource) -> (&'static str, &str) {
    match resource {
        QuotaResource::All => ("all", ""),
        QuotaResource::Provider {
            provider_instance_name,
        } => ("provider", provider_instance_name.as_str()),
        QuotaResource::Model { exact_model } => ("model", exact_model.as_str()),
        QuotaResource::Hidden => ("hidden", ""),
    }
}

pub(crate) fn object_from_parts(object_type: &str, user_id: &str, app_id: &str) -> Option<QuotaObject> {
    match object_type {
        "user" => Some(QuotaObject::User {
            user_id: user_id.to_string(),
        }),
        "app" => Some(QuotaObject::App {
            app_id: app_id.to_string(),
        }),
        "user_app" => Some(QuotaObject::UserApp {
            user_id: user_id.to_string(),
            app_id: app_id.to_string(),
        }),
        _ => None,
    }
}

pub(crate) fn resource_from_parts(resource_type: &str, resource_id: &str) -> Option<QuotaResource> {
    match resource_type {
        "all" => Some(QuotaResource::All),
        "provider" => Some(QuotaResource::Provider {
            provider_instance_name: resource_id.to_string(),
        }),
        "model" => Some(QuotaResource::Model {
            exact_model: resource_id.to_string(),
        }),
        _ => None,
    }
}

/// Object + resource: what a periodic counter or a total account measures.
pub(crate) fn scope_key(object: &QuotaObject, resource: &QuotaResource) -> String {
    let (object_type, user_id, app_id) = object_parts(object);
    let (resource_type, resource_id) = resource_parts(resource);
    [object_type, user_id, app_id, resource_type, resource_id].join(&KEY_SEPARATOR.to_string())
}

pub(crate) fn counter_key(scope_key: &str, period: QuotaPeriod, window_start_ms: i64) -> String {
    format!("{scope_key}{KEY_SEPARATOR}{}{KEY_SEPARATOR}{window_start_ms}", period.as_str())
}

/// Every scope a call of `user_id` / `app_id` on this model falls into.
pub(crate) fn call_scopes(
    user_id: &str,
    app_id: Option<&str>,
    provider_instance_name: &str,
    exact_model: &str,
) -> Vec<(QuotaObject, QuotaResource)> {
    let mut objects = vec![QuotaObject::User {
        user_id: user_id.to_string(),
    }];
    if let Some(app_id) = app_id.filter(|app| !app.is_empty()) {
        objects.push(QuotaObject::App {
            app_id: app_id.to_string(),
        });
        objects.push(QuotaObject::UserApp {
            user_id: user_id.to_string(),
            app_id: app_id.to_string(),
        });
    }
    let resources = [
        QuotaResource::All,
        QuotaResource::Provider {
            provider_instance_name: provider_instance_name.to_string(),
        },
        QuotaResource::Model {
            exact_model: exact_model.to_string(),
        },
    ];
    objects
        .into_iter()
        .flat_map(|object| {
            resources
                .iter()
                .cloned()
                .map(move |resource| (object.clone(), resource))
        })
        .collect()
}

/// Settled / reserved / pending tokens of one window or account.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Ledger {
    pub settled: u64,
    pub reserved: u64,
    pub pending: u64,
}

impl Ledger {
    pub(crate) fn occupied(&self) -> u64 {
        self.settled.saturating_add(self.reserved)
    }

    fn available(&self, limit: u64) -> u64 {
        limit.saturating_sub(self.occupied())
    }

    fn debt(&self, limit: u64) -> u64 {
        self.occupied().saturating_sub(limit)
    }
}

fn status_for(enabled: bool, limit: u64, ledger: &Ledger, zero_is_forbidden: bool) -> QuotaStatus {
    if !enabled {
        return QuotaStatus::Disabled;
    }
    if limit == 0 && zero_is_forbidden {
        return QuotaStatus::Forbidden;
    }
    if ledger.available(limit) == 0 {
        return QuotaStatus::Exhausted;
    }
    if ledger.occupied().saturating_mul(100) >= limit.saturating_mul(QUOTA_NEAR_LIMIT_PERCENT) {
        QuotaStatus::NearLimit
    } else {
        QuotaStatus::Normal
    }
}

pub(crate) fn periodic_state(
    rule: &QuotaRule,
    period: QuotaPeriod,
    ledger: Ledger,
    now_ms: i64,
    updated_at_ms: i64,
) -> QuotaUsageState {
    let (start, end) = period.window(now_ms);
    let limit = rule.limit_tokens.unwrap_or(0);
    QuotaUsageState {
        status: status_for(rule.enabled, limit, &ledger, true),
        window_start_ms: Some(start),
        window_end_ms: Some(end),
        reset_at_ms: Some(end),
        limit_tokens: Some(limit),
        granted_tokens: None,
        account_start_ms: None,
        settled_tokens: ledger.settled,
        reserved_tokens: ledger.reserved,
        pending_tokens: ledger.pending,
        available_tokens: ledger.available(limit),
        debt_tokens: ledger.debt(limit),
        updated_at_ms,
    }
}

pub(crate) fn total_state(
    enabled: bool,
    granted: u64,
    ledger: Ledger,
    start_at_ms: i64,
    updated_at_ms: i64,
) -> QuotaUsageState {
    QuotaUsageState {
        status: status_for(enabled, granted, &ledger, false),
        window_start_ms: None,
        window_end_ms: None,
        reset_at_ms: None,
        limit_tokens: None,
        granted_tokens: Some(granted),
        account_start_ms: Some(start_at_ms),
        settled_tokens: ledger.settled,
        reserved_tokens: ledger.reserved,
        pending_tokens: ledger.pending,
        available_tokens: ledger.available(granted),
        debt_tokens: ledger.debt(granted),
        updated_at_ms,
    }
}

pub(crate) fn mark_resource_unavailable(state: &mut QuotaUsageState) {
    if state.status != QuotaStatus::Disabled {
        state.status = QuotaStatus::ResourceUnavailable;
    }
}

/// A rule checked during admission with the state it was checked against.
#[derive(Clone, Debug)]
pub(crate) struct CheckedRule {
    pub rule: QuotaRule,
    pub state: QuotaUsageState,
}

/// §7.3: every enabled rule must cover `required`; the blocking ones and the
/// largest budget all of them still admit.
pub(crate) fn denial_details(
    checked: &[CheckedRule],
    required: u64,
    provider_instance_name: &str,
    exact_model: &str,
) -> Option<QuotaDenialDetails> {
    let enabled = checked
        .iter()
        .filter(|checked| checked.rule.enabled && !checked.rule.deleted)
        .collect::<Vec<_>>();
    let blocking = enabled
        .iter()
        .filter(|checked| checked.state.available_tokens < required)
        .map(|checked| {
            let limit = match checked.rule.kind {
                QuotaKind::Periodic => checked.state.limit_tokens.unwrap_or(0),
                QuotaKind::Total => 0,
            };
            QuotaBlockingRule {
                rule_id: checked.rule.rule_id.clone(),
                source: checked.rule.source,
                object: checked.rule.object.clone(),
                resource: checked.rule.resource.clone(),
                kind: checked.rule.kind,
                period: checked.rule.period,
                state: checked.state.clone(),
                recoverable: checked.rule.kind == QuotaKind::Periodic
                    && limit > 0
                    && limit >= required,
            }
        })
        .collect::<Vec<_>>();
    if blocking.is_empty() {
        return None;
    }
    let max_allowed_tokens = enabled
        .iter()
        .map(|checked| checked.state.available_tokens)
        .min()
        .unwrap_or(0);
    let reset_at_ms = if blocking.iter().all(|rule| rule.recoverable) {
        blocking
            .iter()
            .filter_map(|rule| rule.state.reset_at_ms)
            .max()
    } else {
        None
    };
    Some(QuotaDenialDetails {
        required_tokens: required,
        max_allowed_tokens,
        provider_instance_name: provider_instance_name.to_string(),
        exact_model: exact_model.to_string(),
        blocking,
        reset_at_ms,
    })
}

/// `quota_exhausted` when a blocking rule has nothing left, otherwise
/// `quota_budget_exceeded` (the call asks for more than what is left).
pub(crate) fn denial_error(details: QuotaDenialDetails) -> buckyos_api::AiccError {
    let exhausted = details
        .blocking
        .iter()
        .any(|rule| rule.state.available_tokens == 0);
    let (code, message) = if exhausted {
        (
            buckyos_api::AiccErrorCode::QuotaExhausted,
            "token quota is exhausted",
        )
    } else {
        (
            buckyos_api::AiccErrorCode::QuotaBudgetExceeded,
            "the request budget exceeds the remaining token quota",
        )
    };
    let mut error = buckyos_api::AiccError::new(code, message);
    error.details = serde_json::to_value(details).ok();
    error
}

pub(crate) fn unavailable_error(scope: &str, now_ms: i64) -> buckyos_api::AiccError {
    let mut error = buckyos_api::AiccError::new(
        buckyos_api::AiccErrorCode::QuotaUnavailable,
        "token quota state is temporarily unavailable",
    );
    error.retriable = true;
    error.details = serde_json::to_value(buckyos_api::QuotaUnavailableDetails {
        scope: scope.to_string(),
        occurred_at_ms: now_ms,
    })
    .ok();
    error
}

/// What one upstream attempt reserves (§7.1 step 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AttemptBudget {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// The output is bounded by the request or the model, not by a fallback.
    pub hard_limit: bool,
}

impl AttemptBudget {
    pub(crate) fn total(&self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }
}

/// The reliable upper bound of the input (UTF-8 bytes of the request text,
/// never fewer than its tokens, plus an allowance per media input) and the
/// output budget: the call's `max_output_tokens` (AICC fills its default
/// during lowering), then the model metadata, else a fallback without a hard
/// guarantee. `None` for operations outside token quota.
pub(crate) fn attempt_budget(call: &ResolvedProviderCall) -> Option<AttemptBudget> {
    if !is_metered(call.api_type) {
        return None;
    }
    let params = call.input.canonical_request.to_params().ok()?;
    let mut input = 0u64;
    count_input(&params, &mut input);
    if let Some(messages) = params.get("messages").and_then(Value::as_array) {
        input = input.saturating_add(MESSAGE_OVERHEAD_TOKENS.saturating_mul(messages.len() as u64));
    }
    let embedding = matches!(
        call.api_type,
        ApiType::EmbeddingText | ApiType::EmbeddingMultimodal
    );
    let requested = params.get("max_output_tokens").and_then(Value::as_u64);
    let (output, hard_limit) = if embedding {
        (0, true)
    } else if let Some(requested) = requested {
        (requested, true)
    } else if let Some(model_limit) = call.model_max_output_tokens {
        (model_limit, true)
    } else {
        (FALLBACK_OUTPUT_TOKENS, false)
    };
    Some(AttemptBudget {
        input_tokens: input,
        output_tokens: output,
        hard_limit,
    })
}

const BUDGET_EXCLUDED_KEYS: &[&str] = &[
    "exact_model",
    "logical_model",
    "max_output_tokens",
    "temperature",
    "top_p",
    "seed",
    "trace_id",
    "idempotency_key",
    "session_id",
    "execution_mode",
    "task_options",
    "output",
];

fn count_input(value: &Value, total: &mut u64) {
    match value {
        Value::String(text) => *total = total.saturating_add(text.len() as u64),
        Value::Array(items) => items.iter().for_each(|item| count_input(item, total)),
        Value::Object(map) => {
            if serde_json::from_value::<ResourceRef>(value.clone()).is_ok() {
                *total = total.saturating_add(MEDIA_INPUT_TOKENS);
                return;
            }
            for (key, item) in map {
                if BUDGET_EXCLUDED_KEYS.contains(&key.as_str()) {
                    continue;
                }
                *total = total.saturating_add(key.len() as u64);
                count_input(item, total);
            }
        }
        Value::Number(number) => *total = total.saturating_add(number.to_string().len() as u64),
        Value::Bool(_) | Value::Null => {}
    }
}

/// Normalized total of a reported usage (§6.1): the reported total, or
/// input plus output when both are known (input alone for an embedding,
/// which has no output). `None` means the usage is unknown, never zero.
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

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::QuotaRuleSource;

    fn rule(kind: QuotaKind, limit: Option<u64>, enabled: bool) -> QuotaRule {
        QuotaRule {
            rule_id: "qr_1".into(),
            source: QuotaRuleSource::Admin,
            object: QuotaObject::User {
                user_id: "alice".into(),
            },
            resource: QuotaResource::All,
            kind,
            period: (kind == QuotaKind::Periodic).then_some(QuotaPeriod::FourHours),
            limit_tokens: limit,
            account_id: None,
            enabled,
            deleted: false,
            pending_period: None,
            version: 1,
            created_by: "admin".into(),
            created_at_ms: 0,
            updated_by: "admin".into(),
            updated_at_ms: 0,
        }
    }

    #[test]
    fn periodic_state_reports_near_limit_at_eighty_percent_and_zero_as_forbidden() {
        let near = periodic_state(
            &rule(QuotaKind::Periodic, Some(100), true),
            QuotaPeriod::FourHours,
            Ledger {
                settled: 60,
                reserved: 20,
                pending: 0,
            },
            0,
            0,
        );
        assert_eq!(near.status, QuotaStatus::NearLimit);
        assert_eq!(near.available_tokens, 20);
        let forbidden = periodic_state(
            &rule(QuotaKind::Periodic, Some(0), true),
            QuotaPeriod::FourHours,
            Ledger::default(),
            0,
            0,
        );
        assert_eq!(forbidden.status, QuotaStatus::Forbidden);
    }

    #[test]
    fn total_state_keeps_debt_beyond_the_grant() {
        let state = total_state(
            true,
            100_000,
            Ledger {
                settled: 110_000,
                reserved: 0,
                pending: 0,
            },
            0,
            0,
        );
        assert_eq!(state.available_tokens, 0);
        assert_eq!(state.debt_tokens, 10_000);
        assert_eq!(state.reset_at_ms, None);
        assert_eq!(state.status, QuotaStatus::Exhausted);
    }

    #[test]
    fn denial_distinguishes_exhausted_from_budget_and_withholds_reset_for_totals() {
        let periodic = CheckedRule {
            rule: rule(QuotaKind::Periodic, Some(10_000), true),
            state: periodic_state(
                &rule(QuotaKind::Periodic, Some(10_000), true),
                QuotaPeriod::FourHours,
                Ledger {
                    settled: 4_000,
                    reserved: 0,
                    pending: 0,
                },
                0,
                0,
            ),
        };
        let details = denial_details(&[periodic.clone()], 8_000, "p", "m@p").unwrap();
        assert_eq!(details.max_allowed_tokens, 6_000);
        assert_eq!(details.reset_at_ms, Some(QuotaPeriod::FourHours.window(0).1));
        assert_eq!(
            denial_error(details).code,
            buckyos_api::AiccErrorCode::QuotaBudgetExceeded
        );
        let mut total = rule(QuotaKind::Total, None, true);
        total.rule_id = "qr_2".into();
        let total = CheckedRule {
            state: total_state(true, 100, Ledger { settled: 100, reserved: 0, pending: 0 }, 0, 0),
            rule: total,
        };
        let details = denial_details(&[periodic, total], 10, "p", "m@p").unwrap();
        assert_eq!(details.reset_at_ms, None);
        assert_eq!(denial_error(details).code, buckyos_api::AiccErrorCode::QuotaExhausted);
        assert!(denial_details(&[], 10, "p", "m@p").is_none());
    }

    #[test]
    fn budget_exceeding_a_whole_period_is_not_recoverable() {
        let limited = rule(QuotaKind::Periodic, Some(1_000), true);
        let checked = CheckedRule {
            state: periodic_state(&limited, QuotaPeriod::FourHours, Ledger::default(), 0, 0),
            rule: limited,
        };
        let details = denial_details(&[checked], 5_000, "p", "m@p").unwrap();
        assert!(!details.blocking[0].recoverable);
        assert_eq!(details.reset_at_ms, None);
    }

    #[test]
    fn usage_total_never_treats_missing_counts_as_zero() {
        let mut usage = buckyos_api::AiUsage::default();
        assert_eq!(usage_total_tokens(&usage, ApiType::Llm), None);
        usage.input_tokens = Some(10);
        assert_eq!(usage_total_tokens(&usage, ApiType::Llm), None);
        assert_eq!(usage_total_tokens(&usage, ApiType::EmbeddingText), Some(10));
        usage.output_tokens = Some(5);
        assert_eq!(usage_total_tokens(&usage, ApiType::Llm), Some(15));
        usage.total_tokens = Some(18);
        assert_eq!(usage_total_tokens(&usage, ApiType::Llm), Some(18));
    }

    #[test]
    fn scopes_cover_user_app_and_shared_app_on_every_resource() {
        let scopes = call_scopes("alice", Some("writer"), "cloud", "m@cloud");
        assert_eq!(scopes.len(), 9);
        assert_eq!(call_scopes("alice", None, "cloud", "m@cloud").len(), 3);
        let key = scope_key(&scopes[0].0, &scopes[0].1);
        assert!(key.starts_with("user\u{1f}alice"));
    }
}
