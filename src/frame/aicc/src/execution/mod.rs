use crate::call::ResolvedProviderCall;
use crate::catalog::{
    Pricing, PricingTierStep, PricingTiers, PricingTimeWindow, PricingUnit, TierDimension, TierMode,
};
use crate::error::NativeTaskResumeError;
use crate::protocol::{
    cancellation_pair, CancelHandle, Cancellation, NativeTaskHandle, NativeTaskState,
    ProtocolError, ProtocolErrorKind, ProtocolEvent, ProtocolOutput, ProtocolStream,
};
use crate::resource::ResourceAccessContext;
use async_trait::async_trait;
use buckyos_api::{
    AiArtifact, AiCost, AiUsage, AiccComputeProgress, AiccComputeTaskData, AiccComputeTaskRequest,
    AiccError, AiccErrorCode, ApiType, Capability,
};
use futures_util::{future::join_all, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DEFAULT_IDEMPOTENCY_WINDOW_MS: u64 = 24 * 60 * 60 * 1_000;
const MAX_SAME_MODEL_ATTEMPTS: usize = 2;
const DEFAULT_SAME_MODEL_RETRY_DELAY: Duration = Duration::from_millis(100);
const MAX_SAME_MODEL_RETRY_DELAY: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct IdempotencyScope {
    pub tenant_id: String,
    pub method: String,
    pub key: String,
}

impl IdempotencyScope {
    pub(crate) fn new(
        tenant_id: impl Into<String>,
        method: impl Into<String>,
        key: impl Into<String>,
    ) -> Result<Self, AiccError> {
        let scope = Self {
            tenant_id: tenant_id.into(),
            method: method.into(),
            key: key.into(),
        };
        if scope.tenant_id.trim().is_empty()
            || scope.method.trim().is_empty()
            || scope.key.trim().is_empty()
            || scope.key.len() > 256
        {
            return Err(aicc_error(
                AiccErrorCode::InvalidRequest,
                "idempotency scope is invalid",
                false,
            ));
        }
        Ok(scope)
    }
}

pub(crate) fn canonical_body_fingerprint(body: &Value) -> Result<String, AiccError> {
    let mut body = body.clone();
    if let Value::Object(object) = &mut body {
        object.remove("idempotency_key");
    }
    let canonical = canonicalize_json(body);
    let encoded = serde_json::to_vec(&canonical).map_err(|_| {
        aicc_error(
            AiccErrorCode::InvalidRequest,
            "canonical request body cannot be serialized",
            false,
        )
    })?;
    Ok(hex_digest(&encoded))
}

fn canonicalize_json(value: Value) -> Value {
    match value {
        Value::Object(object) => {
            let ordered = object
                .into_iter()
                .map(|(key, value)| (key, canonicalize_json(value)))
                .collect::<BTreeMap<_, _>>();
            Value::Object(ordered.into_iter().collect::<Map<_, _>>())
        }
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize_json).collect()),
        value => value,
    }
}

fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn credential_reference_fingerprint(reference: &str) -> String {
    let digest = Sha256::digest(reference.as_bytes());
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExecutionState {
    Submitted,
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl ExecutionState {
    pub(crate) fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

impl From<NativeTaskState> for ExecutionState {
    fn from(value: NativeTaskState) -> Self {
        match value {
            NativeTaskState::Submitted => Self::Submitted,
            NativeTaskState::Queued => Self::Queued,
            NativeTaskState::Running => Self::Running,
            NativeTaskState::Succeeded => Self::Succeeded,
            NativeTaskState::Failed => Self::Failed,
            NativeTaskState::Cancelled => Self::Cancelled,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ExecutionOutput {
    pub value: Value,
    pub usage: AiUsage,
    pub cost: Option<AiCost>,
    pub artifacts: Vec<AiArtifact>,
}

impl TryFrom<ProtocolOutput> for ExecutionOutput {
    type Error = AiccError;

    fn try_from(value: ProtocolOutput) -> Result<Self, Self::Error> {
        let usage_empty = value.usage.as_ref().is_none_or(|usage| {
            usage.input_tokens.is_none()
                && usage.output_tokens.is_none()
                && usage.total_tokens.is_none()
                && usage.image_units.is_none()
                && usage.audio_seconds.is_none()
                && usage.video_seconds.is_none()
                && usage.request_units.is_none()
        });
        let usage = if usage_empty {
            log::warn!(
                "provider completed successfully without usage; billing for this completion is unknown"
            );
            AiUsage::default()
        } else {
            value.usage.unwrap_or_default()
        };
        Ok(Self {
            value: value.value,
            usage,
            cost: None,
            artifacts: value.artifacts,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct PinnedProviderTask {
    pub runtime_generation: u64,
    pub origin_provider: String,
    pub exact_model: String,
    pub provider_model_id: String,
    pub provider_instance_name: String,
    pub protocol_adapter_id: String,
    pub operation: String,
    pub api_type: ApiType,
    pub remote_task_id: Option<String>,
    pub result_artifacts: BTreeMap<String, crate::protocol::ProviderArtifactRef>,
    pub cancel_supported: bool,
    pub resume: Option<NativeTaskResumeDescriptor>,
    pub pricing: Option<PinnedPricingSnapshot>,
    pub reported_cost_currency: Option<String>,
}

impl PinnedProviderTask {
    fn completion_cost(&self, usage: &AiUsage) -> Option<AiCost> {
        if let (Some(currency), Some(amount)) = (&self.reported_cost_currency, usage.reported_cost)
        {
            if !invalid_price(amount) {
                return Some(AiCost {
                    currency: currency.clone(),
                    amount,
                });
            }
        }
        self.pricing.as_ref()?.completion_cost(usage)
    }

    fn from_call(runtime_generation: u64, call: &ResolvedProviderCall) -> Result<Self, AiccError> {
        Ok(Self {
            runtime_generation,
            origin_provider: call.context.state_coordinate.origin_provider.clone(),
            exact_model: call.exact_model.clone(),
            provider_model_id: call.provider_model_id.clone(),
            provider_instance_name: call.provider_instance_name.clone(),
            protocol_adapter_id: call.protocol_adapter_id.clone(),
            operation: call.operation.clone(),
            api_type: call.api_type,
            remote_task_id: None,
            result_artifacts: BTreeMap::new(),
            cancel_supported: false,
            resume: None,
            pricing: PinnedPricingSnapshot::from_call(call)?,
            reported_cost_currency: call.reported_cost_currency.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct PinnedPricingSnapshot {
    pub currency: String,
    pub basis: PinnedPricingBasis,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum PinnedPricingBasis {
    Tokens {
        input_token: Option<f64>,
        cache_input_token: Option<f64>,
        cache_write_input_token: Option<f64>,
        cache_write_1h_input_token: Option<f64>,
        audio_input_token: Option<f64>,
        image_input_token: Option<f64>,
        audio_output_token: Option<f64>,
        image_output_token: Option<f64>,
        output_token: Option<f64>,
        #[serde(default)]
        tiers: Option<PricingTiers>,
    },
    Units {
        unit: PricingUnit,
        amount: f64,
    },
}

/// Monday-based weekday (0 = Monday) and minute-of-day for `now` on the clock implied
/// by `utc_offset_minutes`. Derived from the epoch directly so no timezone database is
/// needed; this means the offset is fixed rather than DST-aware, which is called out in
/// the pricing contract.
fn local_clock(now: std::time::SystemTime, utc_offset_minutes: i32) -> Option<(u8, i64)> {
    let secs = now
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs()
        .min(i64::MAX as u64) as i64;
    let local = secs + i64::from(utc_offset_minutes) * 60;
    let day = local.div_euclid(86_400);
    let minute = local.rem_euclid(86_400) / 60;
    // 1970-01-01 was a Thursday.
    let weekday = u8::try_from((day + 3).rem_euclid(7)).ok()?;
    Some((weekday, minute))
}

fn parse_clock(value: &str) -> Option<i64> {
    let (hour, minute) = value.trim().split_once(':')?;
    let hour: i64 = hour.trim().parse().ok()?;
    let minute: i64 = minute.trim().parse().ok()?;
    if !(0..24).contains(&hour) || !(0..60).contains(&minute) {
        return None;
    }
    Some(hour * 60 + minute)
}

fn time_window_matches(window: &PricingTimeWindow, now: std::time::SystemTime) -> bool {
    let (Some((weekday, minute)), Some(from), Some(to)) = (
        local_clock(now, window.utc_offset_minutes),
        parse_clock(&window.from),
        parse_clock(&window.to),
    ) else {
        return false;
    };
    if from == to {
        return false;
    }
    let in_range = if from < to {
        minute >= from && minute < to
    } else {
        minute >= from || minute < to
    };
    if !in_range {
        return false;
    }
    match &window.days {
        Some(days) => days.is_empty() || days.iter().any(|day| day.index() == weekday),
        None => true,
    }
}

/// First matching window wins; `validate_pricing_time_windows` guarantees windows on one
/// pricing never overlap, so the choice is unambiguous.
fn active_time_window<'a>(
    windows: &'a [PricingTimeWindow],
    now: std::time::SystemTime,
) -> Option<&'a PricingTimeWindow> {
    windows
        .iter()
        .find(|window| time_window_matches(window, now))
}

/// Fold a matched window over the base pricing: only fields the window declares override
/// the base rates, so a provider lists just what changes (for example a peak surcharge).
fn apply_time_window(base: &Pricing, window: Option<&PricingTimeWindow>) -> Pricing {
    let Some(window) = window else {
        return base.clone();
    };
    Pricing {
        currency: base.currency.clone(),
        source_url: base.source_url.clone(),
        verified_at: base.verified_at.clone(),
        ratio_exception: base.ratio_exception.clone(),
        input_token: window.input_token.or(base.input_token),
        output_token: window.output_token.or(base.output_token),
        cache_input_token: window.cache_input_token.or(base.cache_input_token),
        cache_write_input_token: window
            .cache_write_input_token
            .or(base.cache_write_input_token),
        cache_write_1h_input_token: window
            .cache_write_1h_input_token
            .or(base.cache_write_1h_input_token),
        audio_input_token: window.audio_input_token.or(base.audio_input_token),
        image_input_token: window.image_input_token.or(base.image_input_token),
        audio_output_token: window.audio_output_token.or(base.audio_output_token),
        image_output_token: window.image_output_token.or(base.image_output_token),
        estimated_cost: base.estimated_cost,
        unit: window.unit.or(base.unit),
        amount: window.amount.or(base.amount),
        rules: base.rules.clone(),
        tiers: base.tiers.clone(),
        // Already resolved; clearing prevents a second pass from re-applying it.
        time_windows: Vec::new(),
    }
}

impl PinnedPricingSnapshot {
    fn from_call(call: &ResolvedProviderCall) -> Result<Option<Self>, AiccError> {
        let Some(base_pricing) = call.pricing.pricing.as_ref() else {
            return Ok(None);
        };
        Self::from_pricing(
            base_pricing,
            call.pricing.matched_amount,
            std::time::SystemTime::now(),
        )
    }

    pub(crate) fn from_pricing(
        base_pricing: &Pricing,
        matched_amount: Option<f64>,
        now: std::time::SystemTime,
    ) -> Result<Option<Self>, AiccError> {
        // Peak/off-peak billing is decided by wall-clock time, so it is pinned
        // here at request time. Tier selection still has to wait for the usage
        // numbers, which only exist once the response lands.
        let effective = apply_time_window(
            base_pricing,
            active_time_window(&base_pricing.time_windows, now),
        );
        let pricing = &effective;
        let currency = pricing.currency.trim().to_ascii_uppercase();
        if currency.is_empty() {
            return Err(invalid_pinned_pricing());
        }
        let has_token_price = [
            pricing.input_token,
            pricing.output_token,
            pricing.cache_input_token,
            pricing.cache_write_input_token,
            pricing.cache_write_1h_input_token,
            pricing.audio_input_token,
            pricing.image_input_token,
            pricing.audio_output_token,
            pricing.image_output_token,
        ]
        .iter()
        .any(Option::is_some);
        let basis = if has_token_price {
            if pricing.unit.is_some()
                || pricing.input_token.is_some_and(invalid_price)
                || pricing.cache_input_token.is_some_and(invalid_price)
                || pricing.output_token.is_some_and(invalid_price)
            {
                return Err(invalid_pinned_pricing());
            }
            PinnedPricingBasis::Tokens {
                input_token: pricing.input_token,
                cache_input_token: pricing.cache_input_token,
                cache_write_input_token: pricing.cache_write_input_token,
                cache_write_1h_input_token: pricing.cache_write_1h_input_token,
                audio_input_token: pricing.audio_input_token,
                image_input_token: pricing.image_input_token,
                audio_output_token: pricing.audio_output_token,
                image_output_token: pricing.image_output_token,
                output_token: pricing.output_token,
                tiers: pricing.tiers.clone(),
            }
        } else if let Some(unit) = pricing.unit {
            let Some(amount) = matched_amount.or(pricing.amount) else {
                return Ok(None);
            };
            if invalid_price(amount) {
                return Err(invalid_pinned_pricing());
            }
            PinnedPricingBasis::Units { unit, amount }
        } else {
            return Ok(None);
        };
        Ok(Some(Self { currency, basis }))
    }

    pub(crate) fn completion_cost(&self, usage: &AiUsage) -> Option<AiCost> {
        let amount = match &self.basis {
            PinnedPricingBasis::Tokens {
                input_token,
                cache_input_token,
                cache_write_input_token,
                cache_write_1h_input_token,
                audio_input_token,
                image_input_token,
                audio_output_token,
                image_output_token,
                output_token,
                tiers,
            } => {
                let base = TokenRates {
                    input_token: *input_token,
                    cache_input_token: *cache_input_token,
                    cache_write_input_token: *cache_write_input_token,
                    cache_write_1h_input_token: *cache_write_1h_input_token,
                    audio_input_token: *audio_input_token,
                    image_input_token: *image_input_token,
                    audio_output_token: *audio_output_token,
                    image_output_token: *image_output_token,
                    output_token: *output_token,
                };
                let resolved = match tiers {
                    Some(tiers) => tier_rates(tiers, &base, usage)?,
                    None => base,
                };
                resolved.apply(usage)?
            }
            PinnedPricingBasis::Units { unit, amount } => {
                let units = match unit {
                    PricingUnit::Request => usage.request_units? as f64,
                    PricingUnit::Image => usage.image_units? as f64,
                    PricingUnit::AudioSecond => usage.audio_seconds?,
                    PricingUnit::VideoSecond => usage.video_seconds?,
                    PricingUnit::Character => usage.characters? as f64,
                    // Billable by compute-second or megapixel (for example fal.ai);
                    // the vocabulary is expressible but no adapter reports these
                    // counters yet, so the cost stays unresolved rather than wrong.
                    PricingUnit::Second | PricingUnit::Megapixel => return None,
                };
                units * amount
            }
        };
        if invalid_price(amount) {
            return None;
        }
        Some(AiCost {
            amount,
            currency: self.currency.clone(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct TokenRates {
    input_token: Option<f64>,
    cache_input_token: Option<f64>,
    cache_write_input_token: Option<f64>,
    cache_write_1h_input_token: Option<f64>,
    audio_input_token: Option<f64>,
    image_input_token: Option<f64>,
    audio_output_token: Option<f64>,
    image_output_token: Option<f64>,
    output_token: Option<f64>,
}

impl TokenRates {
    fn apply(&self, usage: &AiUsage) -> Option<f64> {
        let input = usage.input_tokens?;
        let output = usage.output_tokens?;
        if usage.reasoning_tokens.is_some_and(|tokens| tokens > output)
            || usage
                .total_tokens
                .is_some_and(|total| input.checked_add(output).is_none_or(|sum| total < sum))
        {
            return None;
        }
        let read = usage.cache_read_input_tokens.unwrap_or(0);
        let write = usage.cache_write_input_tokens.unwrap_or(0);
        let write_1h = usage.cache_write_1h_input_tokens.unwrap_or(0);
        let write_short = write.checked_sub(write_1h)?;
        let audio_in = usage.audio_input_tokens.unwrap_or(0);
        let image_in = usage.image_input_tokens.unwrap_or(0);
        let audio_out = usage.audio_output_tokens.unwrap_or(0);
        let image_out = usage.image_output_tokens.unwrap_or(0);
        if read > 0 && (audio_in > 0 || image_in > 0) {
            return None;
        }
        let text_in = input
            .checked_sub(read)?
            .checked_sub(write)?
            .checked_sub(audio_in)?
            .checked_sub(image_in)?;
        let text_out = output.checked_sub(audio_out)?.checked_sub(image_out)?;
        [
            (text_in, self.input_token),
            (read, self.cache_input_token),
            (write_short, self.cache_write_input_token),
            (write_1h, self.cache_write_1h_input_token),
            (text_out, self.output_token),
            (audio_in, self.audio_input_token),
            (image_in, self.image_input_token),
            (audio_out, self.audio_output_token),
            (image_out, self.image_output_token),
        ]
        .into_iter()
        .try_fold(0.0, |total, (tokens, rate)| {
            if tokens == 0 {
                Some(total)
            } else {
                Some(total + tokens as f64 * rate?)
            }
        })
    }

    fn rate_for(&self, dimension: TierDimension) -> Option<f64> {
        match dimension {
            TierDimension::InputTokens
            | TierDimension::TotalTokens
            | TierDimension::ContextTokens => self.input_token,
            TierDimension::OutputTokens => self.output_token,
            TierDimension::RequestUnits | TierDimension::Characters => None,
        }
    }

    fn set_rate_for(&mut self, dimension: TierDimension, rate: f64) {
        match dimension {
            TierDimension::InputTokens
            | TierDimension::TotalTokens
            | TierDimension::ContextTokens => self.input_token = Some(rate),
            TierDimension::OutputTokens => self.output_token = Some(rate),
            TierDimension::RequestUnits | TierDimension::Characters => {}
        }
    }
}

impl PricingTierStep {
    fn resolve(&self, base: TokenRates) -> TokenRates {
        TokenRates {
            input_token: self.input_token.or(base.input_token),
            cache_input_token: self.cache_input_token.or(base.cache_input_token),
            cache_write_input_token: self
                .cache_write_input_token
                .or(base.cache_write_input_token),
            cache_write_1h_input_token: self
                .cache_write_1h_input_token
                .or(base.cache_write_1h_input_token),
            audio_input_token: self.audio_input_token.or(base.audio_input_token),
            image_input_token: self.image_input_token.or(base.image_input_token),
            audio_output_token: self.audio_output_token.or(base.audio_output_token),
            image_output_token: self.image_output_token.or(base.image_output_token),
            output_token: self.output_token.or(base.output_token),
        }
    }
}

fn tier_quantity(dimension: TierDimension, usage: &AiUsage) -> Option<f64> {
    Some(match dimension {
        TierDimension::InputTokens => usage.input_tokens? as f64,
        TierDimension::OutputTokens => usage.output_tokens? as f64,
        TierDimension::TotalTokens => usage
            .total_tokens
            .or_else(|| usage.input_tokens?.checked_add(usage.output_tokens?))?
            as f64,
        TierDimension::ContextTokens => {
            usage.input_tokens? as f64 + usage.output_tokens.unwrap_or(0) as f64
        }
        TierDimension::RequestUnits => usage.request_units.unwrap_or(1) as f64,
        TierDimension::Characters => usage.characters? as f64,
    })
}

fn tier_rates(tiers: &PricingTiers, base: &TokenRates, usage: &AiUsage) -> Option<TokenRates> {
    let quantity = tier_quantity(tiers.dimension, usage)?;
    let hit = tiers
        .steps
        .iter()
        .find(|step| step.up_to.is_none_or(|bound| quantity < bound as f64))?;
    if tiers.mode == TierMode::Volume {
        return Some(hit.resolve(*base));
    }
    // Graduated: walk the steps and fold them into one equivalent average rate for the
    // tiered dimension. Non-tiered quantities keep the rate of the step the total fell into.
    let hit_rates = hit.resolve(*base);
    let Some(hit_rate) = hit_rates.rate_for(tiers.dimension) else {
        return Some(hit_rates);
    };
    let mut sum = 0.0f64;
    let mut lower = 0.0f64;
    for step in &tiers.steps {
        let upper = step
            .up_to
            .map(|bound| bound as f64)
            .unwrap_or(f64::INFINITY);
        let span = (quantity.min(upper) - lower).max(0.0);
        if span > 0.0 {
            let rate = step
                .resolve(*base)
                .rate_for(tiers.dimension)
                .unwrap_or(hit_rate);
            sum += span * rate;
        }
        lower = upper;
        if quantity <= upper {
            break;
        }
    }
    let mut resolved = hit_rates;
    if quantity > 0.0 {
        resolved.set_rate_for(tiers.dimension, sum / quantity);
    }
    Some(resolved)
}

fn invalid_price(amount: f64) -> bool {
    !amount.is_finite() || amount < 0.0
}

fn invalid_pinned_pricing() -> AiccError {
    aicc_error(
        AiccErrorCode::InternalError,
        "resolved pricing cannot be pinned safely",
        false,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResumeCredentialKind {
    Bearer,
    NamedHeader,
    FalKey,
    GlmJwt,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ResumeCredential {
    pub reference: String,
    pub kind: ResumeCredentialKind,
    pub header_name: Option<String>,
    pub fingerprint: String,
}

impl std::fmt::Debug for ResumeCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResumeCredential")
            .field("reference", &"[REFERENCE]")
            .field("kind", &self.kind)
            .field("header_name", &self.header_name)
            .field("fingerprint", &self.fingerprint)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct NativeTaskResumeDescriptor {
    pub base_url: String,
    pub credential: Option<ResumeCredential>,
    pub resource_access_context: ResourceAccessContext,
    pub resolved_parameters: BTreeMap<String, Value>,
    pub request_timeout_ms: u64,
    pub max_request_bytes: u64,
    pub max_response_bytes: u64,
}

impl NativeTaskResumeDescriptor {
    fn validate(&self) -> Result<(), AiccError> {
        let base_url = reqwest::Url::parse(&self.base_url).map_err(|_| {
            aicc_error(
                AiccErrorCode::InternalError,
                "native task resume base URL is invalid",
                false,
            )
        })?;
        if !matches!(base_url.scheme(), "http" | "https")
            || base_url.cannot_be_a_base()
            || !base_url.username().is_empty()
            || base_url.password().is_some()
            || self.resource_access_context.tenant_id.trim().is_empty()
            || self.resource_access_context.caller_id.trim().is_empty()
            || self.resource_access_context.request_id.trim().is_empty()
            || self.request_timeout_ms == 0
            || self.max_request_bytes == 0
            || self.max_response_bytes == 0
        {
            return Err(aicc_error(
                AiccErrorCode::InternalError,
                "native task resume context is invalid",
                false,
            ));
        }
        if let Some(credential) = &self.credential {
            if credential.reference.trim().is_empty()
                || credential.fingerprint != credential_reference_fingerprint(&credential.reference)
            {
                return Err(aicc_error(
                    AiccErrorCode::InternalError,
                    "native task resume credential reference is invalid",
                    false,
                ));
            }
            if credential.kind == ResumeCredentialKind::NamedHeader
                && credential.header_name.as_deref().is_none_or(str::is_empty)
            {
                return Err(aicc_error(
                    AiccErrorCode::InternalError,
                    "native task resume named-header credential is incomplete",
                    false,
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ExecutionRecord {
    pub scope: IdempotencyScope,
    pub usage_event_id: String,
    pub trace_id: Option<String>,
    pub user_id: String,
    pub caller_app_id: Option<String>,
    pub request_model: String,
    pub body_fingerprint: String,
    pub task_id: String,
    pub event_ref: String,
    pub state: ExecutionState,
    pub binding: Option<PinnedProviderTask>,
    pub output: Option<ExecutionOutput>,
    pub error: Option<AiccError>,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone)]
pub(crate) enum IdempotencyClaim {
    Created(ExecutionRecord),
    Existing(ExecutionRecord),
    Conflict,
}

#[async_trait]
pub(crate) trait ExecutionStore: Send + Sync {
    async fn claim(&self, initial: ExecutionRecord) -> Result<IdempotencyClaim, AiccError>;
    async fn get_task(&self, task_id: &str) -> Result<Option<ExecutionRecord>, AiccError>;
    async fn set_running(
        &self,
        task_id: &str,
        state: ExecutionState,
        binding: PinnedProviderTask,
    ) -> Result<bool, AiccError>;
    async fn stage_output(&self, task_id: &str, output: ExecutionOutput)
        -> Result<bool, AiccError>;
    async fn try_complete(&self, task_id: &str, output: ExecutionOutput)
        -> Result<bool, AiccError>;
    async fn try_fail(&self, task_id: &str, error: AiccError) -> Result<bool, AiccError>;
    async fn try_cancel(&self, task_id: &str) -> Result<bool, AiccError>;
    async fn recoverable(&self) -> Result<Vec<ExecutionRecord>, AiccError>;
}

#[derive(Debug, Clone)]
pub(crate) struct TaskSpec {
    pub tenant_id: String,
    pub user_id: String,
    pub caller_app_id: Option<String>,
    pub method: String,
    pub trace_id: Option<String>,
    pub idempotency_key: String,
    pub parent_id: Option<String>,
    pub input: Value,
}

#[derive(Debug, Clone)]
pub(crate) struct TaskBinding {
    pub task_id: String,
    pub event_ref: String,
}

#[async_trait]
pub(crate) trait TaskManagerPort: Send + Sync {
    async fn ensure_task(&self, spec: TaskSpec) -> Result<TaskBinding, AiccError>;
    async fn report_state(
        &self,
        task_id: &str,
        state: ExecutionState,
        data: Value,
    ) -> Result<(), AiccError>;
    async fn commit_result(&self, task_id: &str, output: &ExecutionOutput)
        -> Result<(), AiccError>;
    async fn fail_task(&self, task_id: &str, error: &AiccError) -> Result<(), AiccError>;
    async fn cancel_task(
        &self,
        task_id: &str,
        user_id: &str,
        caller_app_id: &str,
    ) -> Result<(), AiccError>;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct UsageCompletion {
    pub event_id: String,
    pub tenant_id: String,
    pub user_id: String,
    pub caller_app_id: Option<String>,
    pub task_id: String,
    pub trace_id: Option<String>,
    pub idempotency_key: String,
    pub method: String,
    pub capability: String,
    pub request_model: String,
    pub provider_instance_name: String,
    pub provider_model: String,
    pub usage: AiUsage,
    pub finance_snapshot: Option<AiCost>,
    pub completed_at_ms: i64,
}

#[async_trait]
pub(crate) trait UsageCompletionPort: Send + Sync {
    async fn write_once(&self, completion: UsageCompletion) -> Result<(), AiccError>;
}

#[derive(Debug)]
pub(crate) struct ProviderStartFailure {
    pub error: ProtocolError,
    pub provider_accepted: bool,
    pub retryable: bool,
    pub retry_same_model: bool,
}

impl ProviderStartFailure {
    pub(crate) fn before_accept(error: ProtocolError, retryable: bool) -> Self {
        Self {
            error,
            provider_accepted: false,
            retryable,
            retry_same_model: retryable,
        }
    }

    pub(crate) fn after_accept(error: ProtocolError) -> Self {
        let retryable = error.allows_model_failover();
        let retry_same_model = error.retry_same_model();
        Self {
            error,
            provider_accepted: true,
            retryable,
            retry_same_model,
        }
    }
}

#[derive(Debug)]
pub(crate) enum NativeTaskPoll {
    Pending(NativeTaskState, Option<Value>, Option<Duration>),
    Complete(ProtocolOutput),
    Failed(ProtocolError),
}

#[derive(Debug)]
pub(crate) enum ProviderExecution {
    Immediate(ProtocolOutput),
    Stream(ProtocolStream),
    NativeTask {
        handle: NativeTaskHandle,
        resume: NativeTaskResumeDescriptor,
    },
}

#[async_trait]
pub(crate) trait ProviderExecutionPort: Send + Sync {
    async fn start(
        &self,
        runtime_generation: u64,
        call: &ResolvedProviderCall,
        cancellation: Cancellation,
    ) -> Result<ProviderExecution, ProviderStartFailure>;

    async fn poll_native(
        &self,
        binding: &PinnedProviderTask,
        cancellation: Cancellation,
    ) -> Result<NativeTaskPoll, NativeTaskResumeError>;

    async fn cancel_native(
        &self,
        binding: &PinnedProviderTask,
    ) -> Result<bool, NativeTaskResumeError>;

    fn completion_cost(
        &self,
        binding: &PinnedProviderTask,
        output: &ProtocolOutput,
    ) -> Option<AiCost> {
        binding.completion_cost(output.usage.as_ref()?)
    }
}

pub(crate) struct ExecutionRequest {
    pub tenant_id: String,
    pub user_id: String,
    pub caller_app_id: Option<String>,
    pub trace_id: Option<String>,
    pub request_model: String,
    pub idempotency_key: String,
    pub canonical_body: Value,
    pub parent_task_id: Option<String>,
    pub runtime_generation: u64,
    pub primary: ResolvedProviderCall,
    pub failover: Vec<ResolvedProviderCall>,
    pub runtime_failover: bool,
    pub now_ms: u64,
    pub idempotency_window_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ExecutionReceipt {
    pub task_id: String,
    pub event_ref: String,
    pub state: ExecutionState,
    pub output: Option<ExecutionOutput>,
    pub error: Option<AiccError>,
    pub provider_task_ref: Option<String>,
    #[serde(skip)]
    pub initial_poll_after: Option<Duration>,
}

impl From<ExecutionRecord> for ExecutionReceipt {
    fn from(record: ExecutionRecord) -> Self {
        Self {
            task_id: record.task_id,
            event_ref: record.event_ref,
            state: record.state,
            output: record.output,
            error: record.error,
            provider_task_ref: record.binding.and_then(|binding| binding.remote_task_id),
            initial_poll_after: None,
        }
    }
}

struct ActiveExecution {
    cancel: CancelHandle,
}

pub(crate) struct ExecutionEngine {
    store: Arc<dyn ExecutionStore>,
    tasks: Arc<dyn TaskManagerPort>,
    providers: Arc<dyn ProviderExecutionPort>,
    usage: Arc<dyn UsageCompletionPort>,
    active: Mutex<BTreeMap<String, ActiveExecution>>,
}

impl ExecutionEngine {
    pub(crate) fn new(
        store: Arc<dyn ExecutionStore>,
        tasks: Arc<dyn TaskManagerPort>,
        providers: Arc<dyn ProviderExecutionPort>,
        usage: Arc<dyn UsageCompletionPort>,
    ) -> Self {
        Self {
            store,
            tasks,
            providers,
            usage,
            active: Mutex::new(BTreeMap::new()),
        }
    }

    pub(crate) async fn execute(
        &self,
        request: ExecutionRequest,
    ) -> Result<ExecutionReceipt, AiccError> {
        let scope = IdempotencyScope::new(
            request.tenant_id.clone(),
            request.primary.method.clone(),
            request.idempotency_key.clone(),
        )?;
        if request.primary.method != request.primary.input.canonical_request.method() {
            return Err(aicc_error(
                AiccErrorCode::InvalidRequest,
                "resolved call method differs from canonical request",
                false,
            ));
        }
        if request.user_id.trim().is_empty() {
            return Err(aicc_error(
                AiccErrorCode::InvalidRequest,
                "execution user ID must not be empty",
                false,
            ));
        }
        if request.request_model.trim().is_empty() {
            return Err(aicc_error(
                AiccErrorCode::InvalidRequest,
                "execution request model must not be empty",
                false,
            ));
        }
        if request
            .trace_id
            .as_deref()
            .is_some_and(|trace_id| trace_id.trim().is_empty())
        {
            return Err(aicc_error(
                AiccErrorCode::InvalidRequest,
                "execution trace ID must not be empty",
                false,
            ));
        }
        let fingerprint = canonical_body_fingerprint(&request.canonical_body)?;
        let window = request
            .idempotency_window_ms
            .unwrap_or(DEFAULT_IDEMPOTENCY_WINDOW_MS);
        if window < DEFAULT_IDEMPOTENCY_WINDOW_MS {
            return Err(aicc_error(
                AiccErrorCode::InvalidRequest,
                "idempotency window must not be shorter than 24 hours",
                false,
            ));
        }
        let task = self
            .tasks
            .ensure_task(TaskSpec {
                tenant_id: request.tenant_id.clone(),
                user_id: request.user_id.clone(),
                caller_app_id: request.caller_app_id.clone(),
                method: request.primary.method.clone(),
                trace_id: request.trace_id.clone(),
                idempotency_key: request.idempotency_key.clone(),
                parent_id: request.parent_task_id,
                input: request.canonical_body,
            })
            .await?;
        let initial = ExecutionRecord {
            scope,
            usage_event_id: usage_event_id(&task.task_id),
            trace_id: request.trace_id,
            user_id: request.user_id,
            caller_app_id: request.caller_app_id,
            request_model: request.request_model,
            body_fingerprint: fingerprint,
            task_id: task.task_id,
            event_ref: task.event_ref,
            state: ExecutionState::Submitted,
            binding: None,
            output: None,
            error: None,
            created_at_ms: request.now_ms,
            expires_at_ms: request.now_ms.saturating_add(window),
        };
        let record = match self.store.claim(initial).await? {
            IdempotencyClaim::Existing(record) => return Ok(record.into()),
            IdempotencyClaim::Conflict => {
                return Err(aicc_error(
                    AiccErrorCode::IdempotencyConflict,
                    "idempotency key was already used with a different canonical request body",
                    false,
                ));
            }
            IdempotencyClaim::Created(record) => record,
        };

        self.tasks
            .report_state(
                &record.task_id,
                ExecutionState::Submitted,
                json_state("submitted", None, record.trace_id.as_deref()),
            )
            .await?;
        let (cancel, cancellation) = cancellation_pair();
        self.active
            .lock()
            .expect("active execution lock")
            .insert(record.task_id.clone(), ActiveExecution { cancel });

        let mut calls = Vec::with_capacity(1 + request.failover.len());
        calls.push(request.primary);
        if request.runtime_failover {
            calls.extend(request.failover);
        }
        let mut last_error = None;
        for (index, call) in calls.iter().enumerate() {
            if cancellation.is_cancelled() {
                self.remove_active(&record.task_id);
                return self.current_receipt(&record.task_id).await;
            }
            if call.method != record.scope.method || call.api_type != calls[0].api_type {
                return self
                    .finish_failure(
                        &record.task_id,
                        aicc_error(
                            AiccErrorCode::InternalError,
                            "runtime failover candidate changes method or API type",
                            false,
                        ),
                    )
                    .await;
            }
            let mut binding = match PinnedProviderTask::from_call(request.runtime_generation, call)
            {
                Ok(binding) => binding,
                Err(error) => return self.finish_failure(&record.task_id, error).await,
            };
            let mut same_model_attempt = 0;
            let execution = loop {
                same_model_attempt += 1;
                match self
                    .providers
                    .start(request.runtime_generation, call, cancellation.clone())
                    .await
                {
                    Ok(execution) => break Ok(execution),
                    Err(failure)
                        if failure.retry_same_model
                            && same_model_attempt < MAX_SAME_MODEL_ATTEMPTS =>
                    {
                        self.tasks
                            .report_state(
                                &record.task_id,
                                ExecutionState::Submitted,
                                json_state(
                                    "same_model_retry",
                                    Some(Value::String(call.exact_model.clone())),
                                    record.trace_id.as_deref(),
                                ),
                            )
                            .await?;
                        let delay = failure
                            .error
                            .retry_after
                            .unwrap_or(DEFAULT_SAME_MODEL_RETRY_DELAY)
                            .min(MAX_SAME_MODEL_RETRY_DELAY);
                        tokio::time::sleep(delay).await;
                    }
                    Err(failure) => break Err(failure),
                }
            };
            match execution {
                Ok(execution) => match execution {
                    ProviderExecution::Immediate(output) => {
                        if !self
                            .store
                            .set_running(&record.task_id, ExecutionState::Running, binding.clone())
                            .await?
                        {
                            self.remove_active(&record.task_id);
                            return self.current_receipt(&record.task_id).await;
                        }
                        return self
                            .finish_success(&record.task_id, &record.scope, binding, output)
                            .await;
                    }
                    ProviderExecution::Stream(stream) => {
                        if !self
                            .store
                            .set_running(&record.task_id, ExecutionState::Running, binding.clone())
                            .await?
                        {
                            self.remove_active(&record.task_id);
                            return self.current_receipt(&record.task_id).await;
                        }
                        self.tasks
                            .report_state(
                                &record.task_id,
                                ExecutionState::Running,
                                json_state("running", None, record.trace_id.as_deref()),
                            )
                            .await?;
                        return self
                            .consume_stream(
                                &record.task_id,
                                &record.scope,
                                binding,
                                stream,
                                cancellation.clone(),
                                record.trace_id.as_deref(),
                            )
                            .await;
                    }
                    ProviderExecution::NativeTask { handle, resume } => {
                        if let Err(error) = resume.validate() {
                            return self.finish_failure(&record.task_id, error).await;
                        }
                        binding.remote_task_id = Some(handle.remote_task_id.clone());
                        binding.result_artifacts = handle.result_artifacts.clone();
                        binding.cancel_supported = handle.cancel_supported;
                        binding.resume = Some(resume);
                        let state = ExecutionState::from(handle.state);
                        if !self
                            .store
                            .set_running(&record.task_id, state, binding.clone())
                            .await?
                        {
                            if binding.cancel_supported {
                                let _ = self.providers.cancel_native(&binding).await;
                            }
                            self.remove_active(&record.task_id);
                            return self.current_receipt(&record.task_id).await;
                        }
                        self.tasks
                            .report_state(
                                &record.task_id,
                                state,
                                json_state(
                                    "provider_task_started",
                                    None,
                                    record.trace_id.as_deref(),
                                ),
                            )
                            .await?;
                        self.remove_active(&record.task_id);
                        let mut receipt = self.current_receipt(&record.task_id).await?;
                        receipt.initial_poll_after = handle.poll_after;
                        return Ok(receipt);
                    }
                },
                Err(failure) => {
                    let can_failover =
                        failure.retryable && request.runtime_failover && index + 1 < calls.len();
                    let mut error: AiccError = failure.error.into();
                    if failure.provider_accepted {
                        error.code = AiccErrorCode::ProviderError;
                    }
                    if can_failover {
                        self.tasks
                            .report_state(
                                &record.task_id,
                                ExecutionState::Submitted,
                                json_state(
                                    "runtime_failover",
                                    Some(Value::String(call.exact_model.clone())),
                                    record.trace_id.as_deref(),
                                ),
                            )
                            .await?;
                        last_error = Some(error);
                        continue;
                    }
                    return self.finish_failure(&record.task_id, error).await;
                }
            }
        }
        self.finish_failure(
            &record.task_id,
            last_error.unwrap_or_else(|| {
                aicc_error(
                    AiccErrorCode::ProviderStartFailed,
                    "no Provider execution attempt was available",
                    true,
                )
            }),
        )
        .await
    }

    pub(crate) async fn drive_native(&self, task_id: &str) -> Result<ExecutionReceipt, AiccError> {
        self.drive_native_after(task_id, None).await
    }

    pub(crate) async fn drive_native_after(
        &self,
        task_id: &str,
        initial_poll_after: Option<Duration>,
    ) -> Result<ExecutionReceipt, AiccError> {
        let record = self.store.get_task(task_id).await?.ok_or_else(|| {
            aicc_error(
                AiccErrorCode::InvalidRequest,
                "task binding not found",
                false,
            )
        })?;
        if record.state.is_terminal() {
            return Ok(record.into());
        }
        let Some(binding) = record.binding.clone() else {
            return self
                .finish_failure(
                    task_id,
                    aicc_error(
                        AiccErrorCode::InternalError,
                        "running task has no pinned Provider binding",
                        false,
                    ),
                )
                .await;
        };
        if binding.remote_task_id.is_none() {
            return self
                .finish_failure(
                    task_id,
                    aicc_error(
                        AiccErrorCode::InternalError,
                        "non-native execution cannot be resumed after restart",
                        false,
                    ),
                )
                .await;
        }
        if binding.resume.is_none() {
            return self
                .finish_failure(
                    task_id,
                    aicc_error(
                        AiccErrorCode::InternalError,
                        "native task has no pinned resume descriptor",
                        false,
                    ),
                )
                .await;
        }
        if let Err(error) = binding.resume.as_ref().unwrap().validate() {
            return self.finish_failure(task_id, error).await;
        }
        let (cancel, cancellation) = cancellation_pair();
        let already_active = {
            let mut active = self.active.lock().expect("active execution lock");
            if active.contains_key(task_id) {
                true
            } else {
                active.insert(task_id.to_string(), ActiveExecution { cancel });
                false
            }
        };
        if already_active {
            return self.current_receipt(task_id).await;
        }
        let mut next_poll_after = initial_poll_after;
        loop {
            if let Some(delay) = next_poll_after.take().filter(|delay| !delay.is_zero()) {
                tokio::select! {
                    _ = cancellation.cancelled() => {
                        self.remove_active(task_id);
                        return self.current_receipt(task_id).await;
                    }
                    _ = tokio::time::sleep(delay) => {}
                }
            }
            if cancellation.is_cancelled() {
                self.remove_active(task_id);
                return self.current_receipt(task_id).await;
            }
            match self
                .providers
                .poll_native(&binding, cancellation.clone())
                .await
            {
                Ok(NativeTaskPoll::Pending(state, progress, retry_after)) => {
                    if state.is_terminal() {
                        let error = aicc_error(
                            AiccErrorCode::ProviderError,
                            "terminal Provider task state did not include a final result",
                            false,
                        );
                        return self.finish_failure(task_id, error).await;
                    }
                    if !self
                        .store
                        .set_running(task_id, ExecutionState::from(state), binding.clone())
                        .await?
                    {
                        self.remove_active(task_id);
                        return self.current_receipt(task_id).await;
                    }
                    self.tasks
                        .report_state(
                            task_id,
                            ExecutionState::from(state),
                            json_state("provider_progress", progress, record.trace_id.as_deref()),
                        )
                        .await?;
                    next_poll_after = Some(retry_after.unwrap_or(Duration::from_millis(250)));
                }
                Ok(NativeTaskPoll::Complete(output)) => {
                    return self
                        .finish_success(task_id, &record.scope, binding, output)
                        .await;
                }
                Ok(NativeTaskPoll::Failed(error)) => {
                    return self.finish_failure(task_id, error.into()).await;
                }
                Err(error) => {
                    return self.finish_failure(task_id, error.into_aicc_error()).await;
                }
            }
        }
    }

    pub(crate) async fn recover(&self) -> Result<Vec<ExecutionReceipt>, AiccError> {
        let records = self.store.recoverable().await?;
        let task_ids: Vec<String> = records
            .iter()
            .map(|record| record.task_id.clone())
            .collect();
        let results = join_all(records.iter().map(|record| async move {
            if let Some(output) = &record.output {
                return self
                    .commit_staged_success(&record.task_id, output.clone())
                    .await;
            }
            self.drive_native(&record.task_id).await
        }))
        .await;
        let mut receipts = Vec::with_capacity(results.len());
        for (task_id, result) in task_ids.into_iter().zip(results) {
            match result {
                Ok(receipt) => receipts.push(receipt),
                Err(error) => {
                    log::warn!("AICC task recovery: resume failed for task {task_id}: {error}");
                }
            }
        }
        Ok(receipts)
    }

    pub(crate) async fn cancel(&self, tenant_id: &str, task_id: &str) -> Result<bool, AiccError> {
        let Some(record) = self.store.get_task(task_id).await? else {
            return Ok(false);
        };
        if record.scope.tenant_id != tenant_id {
            return Err(aicc_error(
                AiccErrorCode::PolicyDenied,
                "cross-tenant task cancellation is denied",
                false,
            ));
        }
        if record.state.is_terminal() {
            return Ok(false);
        }
        if record
            .binding
            .as_ref()
            .is_some_and(|binding| binding.remote_task_id.is_some() && !binding.cancel_supported)
        {
            return Err(aicc_error(
                AiccErrorCode::UnsupportedOperation,
                "the provider task does not support cancellation",
                false,
            ));
        }
        let active = self
            .active
            .lock()
            .expect("active execution lock")
            .remove(task_id);
        if let Some(active) = active {
            if !self.store.try_cancel(task_id).await? {
                return Ok(false);
            }
            active.cancel.cancel();
            if let Some(binding) = record
                .binding
                .as_ref()
                .filter(|binding| binding.cancel_supported && binding.remote_task_id.is_some())
            {
                let _ = self.providers.cancel_native(binding).await;
            }
            self.tasks
                .cancel_task(
                    task_id,
                    &record.user_id,
                    record.caller_app_id.as_deref().ok_or_else(|| {
                        aicc_error(
                            AiccErrorCode::InternalError,
                            "task caller app identity is unavailable",
                            false,
                        )
                    })?,
                )
                .await?;
            return Ok(true);
        }
        let Some(binding) = record
            .binding
            .as_ref()
            .filter(|binding| binding.cancel_supported && binding.remote_task_id.is_some())
        else {
            return Ok(false);
        };
        let accepted = match self.providers.cancel_native(binding).await {
            Ok(accepted) => accepted,
            Err(NativeTaskResumeError::CredentialUnavailable) => {
                let error = NativeTaskResumeError::CredentialUnavailable.into_aicc_error();
                self.finish_failure(task_id, error.clone()).await?;
                return Err(error);
            }
            Err(NativeTaskResumeError::Protocol(_)) => return Ok(false),
        };
        if !accepted || !self.store.try_cancel(task_id).await? {
            return Ok(false);
        }
        self.tasks
            .cancel_task(
                task_id,
                &record.user_id,
                record.caller_app_id.as_deref().ok_or_else(|| {
                    aicc_error(
                        AiccErrorCode::InternalError,
                        "task caller app identity is unavailable",
                        false,
                    )
                })?,
            )
            .await?;
        Ok(true)
    }

    async fn consume_stream(
        &self,
        task_id: &str,
        scope: &IdempotencyScope,
        binding: PinnedProviderTask,
        mut stream: ProtocolStream,
        cancellation: Cancellation,
        trace_id: Option<&str>,
    ) -> Result<ExecutionReceipt, AiccError> {
        loop {
            let event = tokio::select! {
                _ = cancellation.cancelled() => {
                    self.remove_active(task_id);
                    return self.current_receipt(task_id).await;
                }
                event = stream.events.next() => event,
            };
            let Some(event) = event else {
                break;
            };
            match event {
                Ok(ProtocolEvent::Delta(delta)) => {
                    self.tasks
                        .report_state(
                            task_id,
                            ExecutionState::Running,
                            json_state("delta", Some(delta), trace_id),
                        )
                        .await?;
                }
                Ok(ProtocolEvent::Progress(progress)) => {
                    self.tasks
                        .report_state(
                            task_id,
                            ExecutionState::Running,
                            json_state("progress", Some(progress), trace_id),
                        )
                        .await?;
                }
                Ok(ProtocolEvent::Final(output)) => {
                    return self.finish_success(task_id, scope, binding, output).await;
                }
                Err(error) => return self.finish_failure(task_id, error.into()).await,
            }
        }
        self.finish_failure(
            task_id,
            aicc_error(
                AiccErrorCode::ProviderError,
                "Provider stream ended without a final result",
                false,
            ),
        )
        .await
    }

    async fn finish_success(
        &self,
        task_id: &str,
        scope: &IdempotencyScope,
        binding: PinnedProviderTask,
        output: ProtocolOutput,
    ) -> Result<ExecutionReceipt, AiccError> {
        let finance_snapshot = match self
            .providers
            .completion_cost(&binding, &output)
            .map(validate_completion_cost)
            .transpose()
        {
            Ok(cost) => cost,
            Err(error) => return self.finish_failure(task_id, error).await,
        };
        let completed_at_ms = match current_time_ms() {
            Ok(timestamp) => timestamp,
            Err(error) => return self.finish_failure(task_id, error).await,
        };
        let mut output = match ExecutionOutput::try_from(output) {
            Ok(output) => output,
            Err(error) => return self.finish_failure(task_id, error).await,
        };
        output.cost = finance_snapshot.clone();
        let record = self.store.get_task(task_id).await?.ok_or_else(|| {
            aicc_error(
                AiccErrorCode::InternalError,
                "execution state disappeared before usage completion",
                false,
            )
        })?;
        if let Err(error) = self
            .usage
            .write_once(UsageCompletion {
                event_id: record.usage_event_id,
                tenant_id: scope.tenant_id.clone(),
                user_id: record.user_id,
                caller_app_id: record.caller_app_id,
                task_id: task_id.to_string(),
                trace_id: record.trace_id,
                idempotency_key: scope.key.clone(),
                method: scope.method.clone(),
                capability: capability_name(binding.api_type).to_string(),
                request_model: record.request_model,
                provider_instance_name: binding.provider_instance_name,
                provider_model: binding.exact_model,
                usage: output.usage.clone(),
                finance_snapshot,
                completed_at_ms,
            })
            .await
        {
            return self.finish_failure(task_id, error).await;
        }
        if !self.store.stage_output(task_id, output.clone()).await? {
            self.remove_active(task_id);
            return self.current_receipt(task_id).await;
        }
        self.commit_staged_success(task_id, output).await
    }

    async fn commit_staged_success(
        &self,
        task_id: &str,
        output: ExecutionOutput,
    ) -> Result<ExecutionReceipt, AiccError> {
        if let Err(error) = self.tasks.commit_result(task_id, &output).await {
            self.remove_active(task_id);
            return Err(error);
        }
        self.store.try_complete(task_id, output.clone()).await?;
        self.remove_active(task_id);
        self.current_receipt(task_id).await
    }

    async fn finish_failure(
        &self,
        task_id: &str,
        error: AiccError,
    ) -> Result<ExecutionReceipt, AiccError> {
        if self.store.try_fail(task_id, error.clone()).await? {
            self.tasks.fail_task(task_id, &error).await?;
        }
        self.remove_active(task_id);
        self.current_receipt(task_id).await
    }

    async fn current_receipt(&self, task_id: &str) -> Result<ExecutionReceipt, AiccError> {
        self.store
            .get_task(task_id)
            .await?
            .map(ExecutionReceipt::from)
            .ok_or_else(|| {
                aicc_error(
                    AiccErrorCode::InternalError,
                    "execution state disappeared",
                    false,
                )
            })
    }

    fn remove_active(&self, task_id: &str) {
        self.active
            .lock()
            .expect("active execution lock")
            .remove(task_id);
    }
}

/// Task progress projection in the typed `aicc.compute` shape
/// (`AiccComputeTaskData`): `request` carries the trace id, `progress.status`
/// the state kind and `progress.events` the reported event.
fn json_state(kind: &str, value: Option<Value>, trace_id: Option<&str>) -> Value {
    let mut event = Map::new();
    event.insert("kind".into(), Value::String(kind.into()));
    if let Some(value) = value {
        event.insert("value".into(), value);
    }
    let data = AiccComputeTaskData {
        request: AiccComputeTaskRequest {
            version: 1,
            trace_id: trace_id.map(str::to_string),
            ..Default::default()
        },
        progress: Some(AiccComputeProgress {
            status: Some(kind.to_string()),
            updated_at_ms: current_time_ms().ok(),
            events: vec![Value::Object(event)],
        }),
        result: None,
        error: None,
    };
    serde_json::to_value(data).unwrap_or(Value::Null)
}

fn aicc_error(code: AiccErrorCode, message: &str, retriable: bool) -> AiccError {
    let mut error = AiccError::new(code, message);
    error.retriable = retriable;
    error
}

fn usage_event_id(task_id: &str) -> String {
    format!("aicc-usage-{}", hex_digest(task_id.as_bytes()))
}

fn current_time_ms() -> Result<i64, AiccError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| {
            aicc_error(
                AiccErrorCode::InternalError,
                "system clock is before the Unix epoch",
                false,
            )
        })?
        .as_millis();
    i64::try_from(millis).map_err(|_| {
        aicc_error(
            AiccErrorCode::InternalError,
            "completion timestamp exceeds the supported range",
            false,
        )
    })
}

fn validate_completion_cost(mut cost: AiCost) -> Result<AiCost, AiccError> {
    cost.currency = cost.currency.trim().to_ascii_uppercase();
    if !cost.amount.is_finite() || cost.amount < 0.0 || cost.currency.is_empty() {
        return Err(aicc_error(
            AiccErrorCode::ProviderError,
            "Provider completion returned an invalid final cost",
            false,
        ));
    }
    Ok(cost)
}

fn capability_name(api_type: ApiType) -> &'static str {
    match api_type.capability() {
        Capability::Llm => "llm",
        Capability::Embedding => "embedding",
        Capability::Rerank => "rerank",
        Capability::Decision => "decision",
        Capability::Image => "image",
        Capability::Vision => "vision",
        Capability::Audio => "audio",
        Capability::Video => "video",
        Capability::Agent => "agent",
    }
}

impl From<ProtocolErrorKind> for ExecutionState {
    fn from(value: ProtocolErrorKind) -> Self {
        if value == ProtocolErrorKind::Cancelled {
            Self::Cancelled
        } else {
            Self::Failed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call::{LoweringRevisions, PricingSource, ResolvedPricing};
    use crate::catalog::Pricing;
    use crate::catalog::PricingWeekday;
    use crate::protocol::{
        CodecContext, CodecInput, CodecLimits, CredentialAudit, CredentialKind, ExecutionMode,
        NativeTaskHandle, ResolvedCredential,
    };
    use buckyos_api::{AiccCall, LlmChatInvokeRequest};
    use futures_util::stream;
    use serde_json::json;
    use std::collections::{BTreeSet, VecDeque};
    use std::time::Duration;

    #[derive(Default)]
    struct MemoryStore {
        scopes: Mutex<BTreeMap<IdempotencyScope, ExecutionRecord>>,
        tasks: Mutex<BTreeMap<String, IdempotencyScope>>,
    }

    #[async_trait]
    impl ExecutionStore for MemoryStore {
        async fn claim(&self, initial: ExecutionRecord) -> Result<IdempotencyClaim, AiccError> {
            let mut scopes = self.scopes.lock().unwrap();
            if let Some(existing) = scopes.get(&initial.scope) {
                return Ok(if existing.body_fingerprint == initial.body_fingerprint {
                    IdempotencyClaim::Existing(existing.clone())
                } else {
                    IdempotencyClaim::Conflict
                });
            }
            self.tasks
                .lock()
                .unwrap()
                .insert(initial.task_id.clone(), initial.scope.clone());
            scopes.insert(initial.scope.clone(), initial.clone());
            Ok(IdempotencyClaim::Created(initial))
        }

        async fn get_task(&self, task_id: &str) -> Result<Option<ExecutionRecord>, AiccError> {
            let tasks = self.tasks.lock().unwrap();
            let Some(scope) = tasks.get(task_id) else {
                return Ok(None);
            };
            Ok(self.scopes.lock().unwrap().get(scope).cloned())
        }

        async fn set_running(
            &self,
            task_id: &str,
            state: ExecutionState,
            binding: PinnedProviderTask,
        ) -> Result<bool, AiccError> {
            self.mutate(task_id, |record| {
                if record.state.is_terminal() {
                    return false;
                }
                record.state = state;
                record.binding = Some(binding);
                true
            })
        }

        async fn stage_output(
            &self,
            task_id: &str,
            output: ExecutionOutput,
        ) -> Result<bool, AiccError> {
            self.mutate(task_id, |record| {
                if record.state.is_terminal() {
                    return false;
                }
                record.output = Some(output);
                true
            })
        }

        async fn try_complete(
            &self,
            task_id: &str,
            output: ExecutionOutput,
        ) -> Result<bool, AiccError> {
            self.mutate(task_id, |record| {
                if record.state.is_terminal() {
                    return false;
                }
                record.state = ExecutionState::Succeeded;
                record.output = Some(output);
                true
            })
        }

        async fn try_fail(&self, task_id: &str, error: AiccError) -> Result<bool, AiccError> {
            self.mutate(task_id, |record| {
                if record.state.is_terminal() {
                    return false;
                }
                record.state = if error.code == AiccErrorCode::Cancelled {
                    ExecutionState::Cancelled
                } else {
                    ExecutionState::Failed
                };
                record.error = Some(error);
                true
            })
        }

        async fn try_cancel(&self, task_id: &str) -> Result<bool, AiccError> {
            self.mutate(task_id, |record| {
                if record.state.is_terminal() {
                    return false;
                }
                record.state = ExecutionState::Cancelled;
                record.error = Some(aicc_error(
                    AiccErrorCode::Cancelled,
                    "task was cancelled",
                    false,
                ));
                true
            })
        }

        async fn recoverable(&self) -> Result<Vec<ExecutionRecord>, AiccError> {
            Ok(self
                .scopes
                .lock()
                .unwrap()
                .values()
                .filter(|record| !record.state.is_terminal())
                .cloned()
                .collect())
        }
    }

    impl MemoryStore {
        fn mutate(
            &self,
            task_id: &str,
            mutation: impl FnOnce(&mut ExecutionRecord) -> bool,
        ) -> Result<bool, AiccError> {
            let tasks = self.tasks.lock().unwrap();
            let scope = tasks.get(task_id).ok_or_else(|| {
                aicc_error(AiccErrorCode::InternalError, "test task missing", false)
            })?;
            let mut scopes = self.scopes.lock().unwrap();
            Ok(mutation(scopes.get_mut(scope).unwrap()))
        }
    }

    #[derive(Default)]
    struct MemoryTasks {
        next_id: Mutex<u64>,
        by_key: Mutex<BTreeMap<String, TaskBinding>>,
        specs: Mutex<Vec<TaskSpec>>,
        trace_by_task: Mutex<BTreeMap<String, Option<String>>>,
        events: Mutex<Vec<(String, ExecutionState, Value)>>,
        completed: Mutex<Vec<(String, Option<String>)>>,
        commit_error: Mutex<Option<AiccError>>,
        failed: Mutex<Vec<(String, Option<String>)>>,
        cancelled: Mutex<Vec<(String, Option<String>)>>,
    }

    impl MemoryTasks {
        fn trace_for_task(&self, task_id: &str) -> Option<String> {
            self.trace_by_task
                .lock()
                .unwrap()
                .get(task_id)
                .cloned()
                .flatten()
        }
    }

    #[async_trait]
    impl TaskManagerPort for MemoryTasks {
        async fn ensure_task(&self, spec: TaskSpec) -> Result<TaskBinding, AiccError> {
            let key = format!(
                "{}:{}:{}",
                spec.tenant_id, spec.method, spec.idempotency_key
            );
            let mut by_key = self.by_key.lock().unwrap();
            if let Some(binding) = by_key.get(&key) {
                return Ok(binding.clone());
            }
            self.specs.lock().unwrap().push(spec.clone());
            let mut next = self.next_id.lock().unwrap();
            *next += 1;
            let binding = TaskBinding {
                task_id: format!("task-{next}"),
                event_ref: format!("task-{next}/events"),
            };
            self.trace_by_task
                .lock()
                .unwrap()
                .insert(binding.task_id.clone(), spec.trace_id);
            by_key.insert(key, binding.clone());
            Ok(binding)
        }

        async fn report_state(
            &self,
            task_id: &str,
            state: ExecutionState,
            data: Value,
        ) -> Result<(), AiccError> {
            self.events
                .lock()
                .unwrap()
                .push((task_id.into(), state, data));
            Ok(())
        }

        async fn commit_result(
            &self,
            task_id: &str,
            _output: &ExecutionOutput,
        ) -> Result<(), AiccError> {
            if let Some(error) = self.commit_error.lock().unwrap().clone() {
                return Err(error);
            }
            self.completed
                .lock()
                .unwrap()
                .push((task_id.into(), self.trace_for_task(task_id)));
            Ok(())
        }

        async fn fail_task(&self, task_id: &str, _error: &AiccError) -> Result<(), AiccError> {
            self.failed
                .lock()
                .unwrap()
                .push((task_id.into(), self.trace_for_task(task_id)));
            Ok(())
        }

        async fn cancel_task(
            &self,
            task_id: &str,
            _user_id: &str,
            _caller_app_id: &str,
        ) -> Result<(), AiccError> {
            self.cancelled
                .lock()
                .unwrap()
                .push((task_id.into(), self.trace_for_task(task_id)));
            Ok(())
        }
    }

    #[derive(Default)]
    struct MemoryUsage {
        writes: Mutex<BTreeMap<String, UsageCompletion>>,
    }

    #[async_trait]
    impl UsageCompletionPort for MemoryUsage {
        async fn write_once(&self, completion: UsageCompletion) -> Result<(), AiccError> {
            self.writes
                .lock()
                .unwrap()
                .entry(completion.event_id.clone())
                .or_insert(completion);
            Ok(())
        }
    }

    enum StartPlan {
        Success(ProviderExecution),
        Failure(ProviderStartFailure),
    }

    #[derive(Default)]
    struct FakeProviders {
        starts: Mutex<Vec<String>>,
        start_generations: Mutex<Vec<u64>>,
        plans: Mutex<VecDeque<StartPlan>>,
        polls: Mutex<VecDeque<NativeTaskPoll>>,
        poll_error: Mutex<Option<NativeTaskResumeError>>,
        cancel_result: Mutex<bool>,
        cancel_error: Mutex<Option<NativeTaskResumeError>>,
        completion_cost: Mutex<Option<AiCost>>,
    }

    #[async_trait]
    impl ProviderExecutionPort for FakeProviders {
        async fn start(
            &self,
            runtime_generation: u64,
            call: &ResolvedProviderCall,
            _cancellation: Cancellation,
        ) -> Result<ProviderExecution, ProviderStartFailure> {
            self.starts.lock().unwrap().push(call.exact_model.clone());
            self.start_generations
                .lock()
                .unwrap()
                .push(runtime_generation);
            match self.plans.lock().unwrap().pop_front().unwrap() {
                StartPlan::Success(result) => Ok(result),
                StartPlan::Failure(error) => Err(error),
            }
        }

        async fn poll_native(
            &self,
            _binding: &PinnedProviderTask,
            _cancellation: Cancellation,
        ) -> Result<NativeTaskPoll, NativeTaskResumeError> {
            if let Some(error) = self.poll_error.lock().unwrap().clone() {
                return Err(error);
            }
            Ok(self.polls.lock().unwrap().pop_front().unwrap())
        }

        async fn cancel_native(
            &self,
            _binding: &PinnedProviderTask,
        ) -> Result<bool, NativeTaskResumeError> {
            if let Some(error) = self.cancel_error.lock().unwrap().clone() {
                return Err(error);
            }
            Ok(*self.cancel_result.lock().unwrap())
        }

        fn completion_cost(
            &self,
            binding: &PinnedProviderTask,
            output: &ProtocolOutput,
        ) -> Option<AiCost> {
            self.completion_cost
                .lock()
                .unwrap()
                .clone()
                .or_else(|| output.usage.as_ref()?.cost.clone())
                .or_else(|| {
                    binding
                        .pricing
                        .as_ref()
                        .and_then(|pricing| pricing.completion_cost(output.usage.as_ref()?))
                })
        }
    }

    fn output(text: &str) -> ProtocolOutput {
        ProtocolOutput {
            value: json!({"text": text}),
            usage: Some(AiUsage {
                input_tokens: Some(2),
                output_tokens: Some(1),
                total_tokens: Some(3),
                request_units: None,
                ..AiUsage::default()
            }),
            artifacts: Vec::new(),
        }
    }

    fn resume_descriptor() -> NativeTaskResumeDescriptor {
        let reference = "system-config://secrets/aicc/fixed-provider".to_string();
        NativeTaskResumeDescriptor {
            base_url: "https://fixed-provider.invalid/v1".into(),
            credential: Some(ResumeCredential {
                fingerprint: credential_reference_fingerprint(&reference),
                reference,
                kind: ResumeCredentialKind::Bearer,
                header_name: None,
            }),
            resolved_parameters: BTreeMap::from([("provider_model_id".into(), json!("model"))]),
            resource_access_context: ResourceAccessContext::new("tenant-a", "alice", "request-a")
                .unwrap(),
            request_timeout_ms: 10_000,
            max_request_bytes: 1_024,
            max_response_bytes: 2_048,
        }
    }

    fn call(instance: &str) -> ResolvedProviderCall {
        let request = LlmChatInvokeRequest::new(format!("model@{instance}"), Vec::new());
        ResolvedProviderCall {
            reported_cost_currency: None,
            exact_model: format!("model@{instance}"),
            provider_model_id: "model".into(),
            provider_instance_name: instance.into(),
            provider_profile_id: "fake".into(),
            protocol_adapter_id: "fake-adapter".into(),
            model_driver_id: "fake".into(),
            origin_model_id: "model".into(),
            variant: None,
            method: "chat.completions.create".into(),
            api_type: ApiType::Llm,
            operation: "fake.create".into(),
            execution_mode: ExecutionMode::Immediate,
            input: CodecInput {
                canonical_request: AiccCall::ChatCompletionsCreate(request),
                resolved_parameters: BTreeMap::new(),
            },
            context: CodecContext {
                base_url: "https://fake.invalid".into(),
                state_coordinate: buckyos_api::ProviderStateCoordinate {
                    provider_profile_id: "fake".into(),
                    adapter_type: "fake-adapter".into(),
                    origin_provider: "fake".into(),
                    origin_model: "model".into(),
                },
                credential: Some(ResolvedCredential::bearer("credential-1", "secret").unwrap()),
                resources: BTreeMap::new(),
                limits: CodecLimits {
                    request_timeout: Duration::from_secs(10),
                    max_request_bytes: 1024,
                    max_response_bytes: 1024,
                },
            },
            credential: CredentialAudit {
                kind: CredentialKind::Bearer,
                anonymous_ref: crate::protocol::AnonymousCredentialRef::from_reference(
                    "credential-1",
                )
                .unwrap(),
            },
            credential_reference: "credential-1".into(),
            credential_header_name: None,
            resource_requirements: Vec::new(),
            resource_access_context: Some(
                ResourceAccessContext::new("tenant-1", "user-1", "request-1").unwrap(),
            ),
            pricing: ResolvedPricing {
                source: PricingSource::RouteEstimate,
                pricing: None,
                matched_amount: None,
                estimated_cost: None,
            },
            revisions: LoweringRevisions {
                catalog_target_seq: 1,
                model_driver_revision_seq: 1,
                provider_rules_revision_seq: Some(1),
                inventory_revision: "inv-1".into(),
            },
        }
    }

    fn time_window(
        from: &str,
        to: &str,
        days: Option<Vec<PricingWeekday>>,
        input_token: Option<f64>,
        output_token: Option<f64>,
    ) -> PricingTimeWindow {
        PricingTimeWindow {
            cache_write_input_token: None,
            cache_write_1h_input_token: None,
            audio_input_token: None,
            image_input_token: None,
            audio_output_token: None,
            image_output_token: None,
            from: from.into(),
            to: to.into(),
            utc_offset_minutes: 480,
            days,
            input_token,
            output_token,
            cache_input_token: None,
            unit: None,
            amount: None,
        }
    }

    /// Epoch 0 is a Thursday 00:00 UTC, so with a +08:00 offset `at(3600)` is
    /// Thursday 09:00 local -- inside a 09:00-12:00 peak window.
    fn at(secs: u64) -> std::time::SystemTime {
        std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs)
    }

    #[test]
    fn time_window_matches_local_peak_hours() {
        let window = time_window("09:00", "12:00", None, Some(9e-6), None);
        assert!(time_window_matches(&window, at(3600)));
        assert!(!time_window_matches(&window, at(0)));
        // `to` is exclusive.
        assert!(!time_window_matches(&window, at(4 * 3600)));
    }

    #[test]
    fn time_window_honours_weekday_filter() {
        let thursday = time_window(
            "09:00",
            "12:00",
            Some(vec![PricingWeekday::Thu]),
            None,
            None,
        );
        let monday = time_window(
            "09:00",
            "12:00",
            Some(vec![PricingWeekday::Mon]),
            None,
            None,
        );
        assert!(time_window_matches(&thursday, at(3600)));
        assert!(!time_window_matches(&monday, at(3600)));
    }

    #[test]
    fn time_window_wraps_past_midnight() {
        let window = time_window("22:00", "02:00", None, None, None);
        // Thursday 23:00 and Friday 01:00 local both fall inside.
        assert!(time_window_matches(&window, at(15 * 3600)));
        assert!(time_window_matches(&window, at(17 * 3600)));
        assert!(!time_window_matches(&window, at(4 * 3600)));
    }

    #[test]
    fn time_window_override_only_replaces_declared_fields() {
        let base = Pricing {
            source_url: None,
            verified_at: None,
            ratio_exception: None,
            cache_write_input_token: None,
            cache_write_1h_input_token: None,
            audio_input_token: None,
            image_input_token: None,
            audio_output_token: None,
            image_output_token: None,
            currency: "USD".into(),
            input_token: Some(1e-6),
            output_token: Some(4e-6),
            cache_input_token: Some(1e-7),
            estimated_cost: None,
            unit: None,
            amount: None,
            rules: Vec::new(),
            tiers: None,
            time_windows: Vec::new(),
        };
        let window = time_window("09:00", "12:00", None, Some(9e-6), None);
        let merged = apply_time_window(&base, Some(&window));
        assert_eq!(merged.input_token, Some(9e-6));
        // Untouched fields keep the base (off-peak) rates.
        assert_eq!(merged.output_token, Some(4e-6));
        assert_eq!(merged.cache_input_token, Some(1e-7));
        // Already resolved, so a second pass cannot re-apply it.
        assert!(merged.time_windows.is_empty());
    }

    #[test]
    fn apply_time_window_without_match_returns_base() {
        let base = Pricing {
            source_url: None,
            verified_at: None,
            ratio_exception: None,
            cache_write_input_token: None,
            cache_write_1h_input_token: None,
            audio_input_token: None,
            image_input_token: None,
            audio_output_token: None,
            image_output_token: None,
            currency: "USD".into(),
            input_token: Some(1e-6),
            output_token: Some(4e-6),
            cache_input_token: None,
            estimated_cost: None,
            unit: None,
            amount: None,
            rules: Vec::new(),
            tiers: None,
            time_windows: vec![time_window("09:00", "12:00", None, Some(9e-6), None)],
        };
        let unchanged = apply_time_window(&base, None);
        assert_eq!(unchanged.input_token, Some(1e-6));
        assert_eq!(unchanged.time_windows.len(), 1);
    }

    fn token_priced_call(
        instance: &str,
        input_token: f64,
        output_token: f64,
    ) -> ResolvedProviderCall {
        let mut call = call(instance);
        call.pricing = ResolvedPricing {
            source: PricingSource::ProviderRules,
            pricing: Some(Pricing {
                source_url: None,
                verified_at: None,
                ratio_exception: None,
                cache_write_input_token: None,
                cache_write_1h_input_token: None,
                audio_input_token: None,
                image_input_token: None,
                audio_output_token: None,
                image_output_token: None,
                currency: "USD".into(),
                input_token: Some(input_token),
                output_token: Some(output_token),
                cache_input_token: None,
                estimated_cost: Some(999.0),
                unit: None,
                amount: None,
                rules: Vec::new(),
                tiers: None,

                time_windows: Vec::new(),
            }),
            matched_amount: None,
            estimated_cost: Some(buckyos_api::Money::new(777.0, "USD")),
        };
        call
    }

    fn unit_priced_call(instance: &str, amount: f64) -> ResolvedProviderCall {
        let mut call = call(instance);
        call.pricing = ResolvedPricing {
            source: PricingSource::ProviderRules,
            pricing: Some(Pricing {
                source_url: None,
                verified_at: None,
                ratio_exception: None,
                cache_write_input_token: None,
                cache_write_1h_input_token: None,
                audio_input_token: None,
                image_input_token: None,
                audio_output_token: None,
                image_output_token: None,
                currency: "CNY".into(),
                input_token: None,
                output_token: None,
                cache_input_token: None,
                estimated_cost: None,
                unit: Some(PricingUnit::Request),
                amount: Some(amount),
                rules: Vec::new(),
                tiers: None,

                time_windows: Vec::new(),
            }),
            matched_amount: None,
            estimated_cost: None,
        };
        call
    }

    fn token_usage(input_tokens: u64, output_tokens: u64) -> AiUsage {
        AiUsage {
            input_tokens: Some(input_tokens),
            output_tokens: Some(output_tokens),
            total_tokens: None,
            cache_read_input_tokens: None,
            cache_write_input_tokens: None,
            cache_write_1h_input_tokens: None,
            reasoning_tokens: None,
            image_units: None,
            audio_seconds: None,
            video_seconds: None,
            request_units: None,
            characters: None,
            audio_input_tokens: None,
            image_input_tokens: None,
            audio_output_tokens: None,
            image_output_tokens: None,
            reported_cost: None,
            cost: None,
        }
    }

    fn input_tiers(mode: TierMode) -> PricingTiers {
        PricingTiers {
            dimension: TierDimension::InputTokens,
            mode,
            steps: vec![
                PricingTierStep {
                    cache_write_input_token: None,
                    cache_write_1h_input_token: None,
                    audio_input_token: None,
                    image_input_token: None,
                    audio_output_token: None,
                    image_output_token: None,
                    up_to: Some(32 * 1024),
                    input_token: Some(6e-6),
                    output_token: Some(24e-6),
                    cache_input_token: None,
                    amount: None,
                    unit: None,
                },
                PricingTierStep {
                    cache_write_input_token: None,
                    cache_write_1h_input_token: None,
                    audio_input_token: None,
                    image_input_token: None,
                    audio_output_token: None,
                    image_output_token: None,
                    up_to: None,
                    input_token: Some(8e-6),
                    output_token: Some(28e-6),
                    cache_input_token: None,
                    amount: None,
                    unit: None,
                },
            ],
        }
    }

    #[test]
    fn request_priced_call_settles_without_matched_amount() {
        // Regression: `matched_amount` is always None on the provider-inventory path, which
        // used to make every per-request / per-image / per-second price settle to zero.
        let call = unit_priced_call("instance", 0.1);
        let pinned = PinnedPricingSnapshot::from_call(&call).unwrap().unwrap();
        assert_eq!(
            pinned.basis,
            PinnedPricingBasis::Units {
                unit: PricingUnit::Request,
                amount: 0.1
            }
        );
        let cost = pinned
            .completion_cost(&AiUsage::request_units(1))
            .expect("request usage");
        assert!((cost.amount - 0.1).abs() < 1e-12);
        assert_eq!(cost.currency, "CNY");
    }

    #[test]
    fn tiered_volume_reprices_the_whole_quantity() {
        let pricing = PinnedPricingSnapshot {
            currency: "CNY".into(),
            basis: PinnedPricingBasis::Tokens {
                cache_write_input_token: None,
                cache_write_1h_input_token: None,
                audio_input_token: None,
                image_input_token: None,
                audio_output_token: None,
                image_output_token: None,
                input_token: Some(6e-6),
                cache_input_token: None,
                output_token: Some(24e-6),
                tiers: Some(input_tiers(TierMode::Volume)),
            },
        };
        let below = pricing.completion_cost(&token_usage(1_000, 1_000)).unwrap();
        assert!((below.amount - (1_000.0 * 6e-6 + 1_000.0 * 24e-6)).abs() < 1e-12);
        // 40K input crosses into the second step; the entire input is repriced at 8.
        let above = pricing
            .completion_cost(&token_usage(40_000, 1_000))
            .unwrap();
        assert!((above.amount - (40_000.0 * 8e-6 + 1_000.0 * 28e-6)).abs() < 1e-12);
    }

    #[test]
    fn tiered_graduated_walks_each_step() {
        let pricing = PinnedPricingSnapshot {
            currency: "CNY".into(),
            basis: PinnedPricingBasis::Tokens {
                cache_write_input_token: None,
                cache_write_1h_input_token: None,
                audio_input_token: None,
                image_input_token: None,
                audio_output_token: None,
                image_output_token: None,
                input_token: Some(6e-6),
                cache_input_token: None,
                output_token: Some(24e-6),
                tiers: Some(input_tiers(TierMode::Graduated)),
            },
        };
        let cost = pricing.completion_cost(&token_usage(40_000, 0)).unwrap();
        let expected = 32_768.0 * 6e-6 + (40_000.0 - 32_768.0) * 8e-6;
        assert!((cost.amount - expected).abs() < 1e-12, "{}", cost.amount);
    }

    #[test]
    fn tier_boundary_is_exclusive() {
        let pricing = PinnedPricingSnapshot {
            currency: "CNY".into(),
            basis: PinnedPricingBasis::Tokens {
                cache_write_input_token: None,
                cache_write_1h_input_token: None,
                audio_input_token: None,
                image_input_token: None,
                audio_output_token: None,
                image_output_token: None,
                input_token: Some(6e-6),
                cache_input_token: None,
                output_token: Some(24e-6),
                tiers: Some(input_tiers(TierMode::Volume)),
            },
        };
        let boundary = 32 * 1024;
        let just_below = pricing
            .completion_cost(&token_usage(boundary - 1, 0))
            .unwrap();
        assert!((just_below.amount - ((boundary - 1) as f64 * 6e-6)).abs() < 1e-12);
        let at_boundary = pricing.completion_cost(&token_usage(boundary, 0)).unwrap();
        assert!((at_boundary.amount - (boundary as f64 * 8e-6)).abs() < 1e-12);
    }

    #[test]
    fn pinned_pricing_uses_cached_input_rate_and_provider_reported_cost_wins() {
        let pricing = PinnedPricingSnapshot {
            currency: "USD".into(),
            basis: PinnedPricingBasis::Tokens {
                cache_write_input_token: None,
                cache_write_1h_input_token: None,
                audio_input_token: None,
                image_input_token: None,
                audio_output_token: None,
                image_output_token: None,
                input_token: Some(0.01),
                cache_input_token: Some(0.001),
                output_token: Some(0.02),
                tiers: None,
            },
        };
        let usage = AiUsage {
            input_tokens: Some(100),
            output_tokens: Some(10),
            cache_read_input_tokens: Some(40),
            ..AiUsage::default()
        };
        assert!((pricing.completion_cost(&usage).unwrap().amount - 0.84).abs() < 1e-12);

        let output = ProtocolOutput {
            value: json!({}),
            usage: Some(AiUsage {
                cost: Some(AiCost {
                    amount: 0.25,
                    currency: "USD".into(),
                }),
                ..usage
            }),
            artifacts: Vec::new(),
        };
        let providers = FakeProviders::default();
        let binding =
            PinnedProviderTask::from_call(1, &token_priced_call("primary", 1.0, 1.0)).unwrap();
        assert_eq!(
            providers.completion_cost(&binding, &output).unwrap().amount,
            0.25
        );
    }

    fn request(primary: ResolvedProviderCall) -> ExecutionRequest {
        ExecutionRequest {
            tenant_id: "tenant-1".into(),
            user_id: "user-1".into(),
            caller_app_id: Some("app-1".into()),
            trace_id: Some("trace-1".into()),
            request_model: "smart-chat".into(),
            idempotency_key: "idem-1".into(),
            canonical_body: json!({"exact_model": primary.exact_model, "prompt": "hello", "idempotency_key": "idem-1"}),
            parent_task_id: None,
            runtime_generation: 7,
            primary,
            failover: Vec::new(),
            runtime_failover: false,
            now_ms: 1_000,
            idempotency_window_ms: None,
        }
    }

    fn make_engine(
        providers: Arc<FakeProviders>,
    ) -> (
        ExecutionEngine,
        Arc<MemoryStore>,
        Arc<MemoryTasks>,
        Arc<MemoryUsage>,
    ) {
        let store = Arc::new(MemoryStore::default());
        let tasks = Arc::new(MemoryTasks::default());
        let usage = Arc::new(MemoryUsage::default());
        (
            ExecutionEngine::new(store.clone(), tasks.clone(), providers, usage.clone()),
            store,
            tasks,
            usage,
        )
    }

    #[test]
    fn fingerprint_is_key_order_independent_and_excludes_idempotency_key() {
        let left = json!({"b": [2, {"z": true, "a": 1}], "a": 1, "idempotency_key": "x"});
        let right = json!({"a": 1, "b": [2, {"a": 1, "z": true}], "idempotency_key": "y"});
        assert_eq!(
            canonical_body_fingerprint(&left).unwrap(),
            canonical_body_fingerprint(&right).unwrap()
        );
    }

    #[test]
    fn all_provider_task_states_have_one_canonical_execution_state() {
        assert_eq!(
            ExecutionState::from(NativeTaskState::Submitted),
            ExecutionState::Submitted
        );
        assert_eq!(
            ExecutionState::from(NativeTaskState::Queued),
            ExecutionState::Queued
        );
        assert_eq!(
            ExecutionState::from(NativeTaskState::Running),
            ExecutionState::Running
        );
        assert_eq!(
            ExecutionState::from(NativeTaskState::Succeeded),
            ExecutionState::Succeeded
        );
        assert_eq!(
            ExecutionState::from(NativeTaskState::Failed),
            ExecutionState::Failed
        );
        assert_eq!(
            ExecutionState::from(NativeTaskState::Cancelled),
            ExecutionState::Cancelled
        );
    }

    #[tokio::test]
    async fn decision_failover_preserves_api_and_writes_only_successful_usage() {
        let providers = Arc::new(FakeProviders::default());
        providers.plans.lock().unwrap().extend([
            StartPlan::Failure(ProviderStartFailure::before_accept(
                ProtocolError::new(ProtocolErrorKind::Transport, "unavailable"),
                true,
            )),
            StartPlan::Failure(ProviderStartFailure::before_accept(
                ProtocolError::new(ProtocolErrorKind::Transport, "unavailable"),
                true,
            )),
            StartPlan::Success(ProviderExecution::Immediate(ProtocolOutput {
                value: json!({"answers":[{"id":"q","type":"boolean","probability_true":0.9}]}),
                usage: Some(token_usage(10, 5)),
                artifacts: Vec::new(),
            })),
        ]);
        let decision_call = |provider| {
            let mut resolved = token_priced_call(provider, 4.2e-8, 0.0);
            resolved.api_type = ApiType::Decision;
            resolved.method = "decision.evaluate".into();
            resolved.operation = "systemone.evaluate".into();
            resolved.input.canonical_request =
                buckyos_api::AiccCall::DecisionEvaluate(buckyos_api::DecisionEvaluateRequest::new(
                    resolved.exact_model.clone(),
                    json!("state"),
                    vec![buckyos_api::DecisionQuestion::Boolean {
                        id: "q".into(),
                        instructions: json!("Test"),
                        criteria: None,
                    }],
                ));
            resolved
        };
        let mut req = request(decision_call("primary"));
        req.request_model = "decision".into();
        req.canonical_body = req.primary.input.canonical_request.to_params().unwrap();
        req.failover.push(decision_call("backup"));
        req.runtime_failover = true;
        let (engine, _, _, usage) = make_engine(providers.clone());
        assert_eq!(
            engine.execute(req).await.unwrap().state,
            ExecutionState::Succeeded
        );
        assert_eq!(
            providers.starts.lock().unwrap().as_slice(),
            ["model@primary", "model@primary", "model@backup"]
        );
        let writes = usage.writes.lock().unwrap();
        assert_eq!(writes.len(), 1);
        let completion = writes.values().next().unwrap();
        assert_eq!(completion.capability, "decision");
        assert_eq!(completion.provider_instance_name, "backup");
        assert_eq!(completion.usage.output_tokens, Some(5));
    }

    #[tokio::test]
    async fn decision_replay_preserves_output_tokens_and_charges_input_once() {
        let providers = Arc::new(FakeProviders::default());
        let mut primary = token_priced_call("primary", 4.2e-8, 0.0);
        primary.api_type = ApiType::Decision;
        primary.method = "decision.evaluate".into();
        primary.operation = "systemone.evaluate".into();
        let canonical = buckyos_api::DecisionEvaluateRequest::new(
            "model@primary",
            json!("shared state"),
            vec![buckyos_api::DecisionQuestion::Boolean {
                id: "q".into(),
                instructions: json!("Is it shared?"),
                criteria: None,
            }],
        );
        primary.input.canonical_request =
            buckyos_api::AiccCall::DecisionEvaluate(canonical.clone());
        let mut replay = request(primary.clone());
        replay.request_model = "decision".into();
        replay.canonical_body = serde_json::to_value(&canonical).unwrap();
        let mut execution = request(primary);
        execution.request_model = "decision".into();
        execution.canonical_body = serde_json::to_value(canonical).unwrap();
        providers
            .plans
            .lock()
            .unwrap()
            .push_back(StartPlan::Success(ProviderExecution::Immediate(
                ProtocolOutput {
                    value: json!({"answers":[{"id":"q","type":"boolean","probability_true":0.9}]}),
                    usage: Some(token_usage(318, 72)),
                    artifacts: Vec::new(),
                },
            )));
        let (engine, _, _, usage) = make_engine(providers.clone());
        let first = engine.execute(execution).await.unwrap();
        assert_eq!(first.state, ExecutionState::Succeeded);
        assert_eq!(engine.execute(replay).await.unwrap(), first);
        assert_eq!(providers.starts.lock().unwrap().len(), 1);
        let writes = usage.writes.lock().unwrap();
        assert_eq!(writes.len(), 1);
        let completion = writes.values().next().unwrap();
        assert_eq!(completion.capability, "decision");
        assert_eq!(completion.method, "decision.evaluate");
        assert_eq!(completion.usage.output_tokens, Some(72));
        assert!(
            (completion.finance_snapshot.as_ref().unwrap().amount - 318.0 * 4.2e-8).abs() < 1e-12
        );
    }

    #[tokio::test]
    async fn immediate_and_idempotent_replay_write_usage_once() {
        let providers = Arc::new(FakeProviders::default());
        providers
            .plans
            .lock()
            .unwrap()
            .push_back(StartPlan::Success(ProviderExecution::Immediate(output(
                "ok",
            ))));
        *providers.completion_cost.lock().unwrap() = Some(AiCost {
            amount: 0.25,
            currency: "usd".into(),
        });
        let (engine, _, tasks, usage) = make_engine(providers.clone());
        let first = engine.execute(request(call("primary"))).await.unwrap();
        let replay = engine.execute(request(call("primary"))).await.unwrap();
        assert_eq!(first, replay);
        assert_eq!(first.state, ExecutionState::Succeeded);
        assert_eq!(providers.starts.lock().unwrap().len(), 1);
        let writes = usage.writes.lock().unwrap();
        assert_eq!(writes.len(), 1);
        let completion = writes.values().next().unwrap();
        assert_eq!(completion.event_id, usage_event_id("task-1"));
        assert_eq!(completion.tenant_id, "tenant-1");
        assert_eq!(completion.user_id, "user-1");
        assert_eq!(completion.caller_app_id.as_deref(), Some("app-1"));
        assert_eq!(completion.trace_id.as_deref(), Some("trace-1"));
        assert_eq!(completion.method, "chat.completions.create");
        assert_eq!(completion.capability, "llm");
        assert_eq!(completion.request_model, "smart-chat");
        assert_eq!(completion.provider_instance_name, "primary");
        assert_eq!(completion.provider_model, "model@primary");
        assert_eq!(
            completion.finance_snapshot,
            Some(AiCost {
                amount: 0.25,
                currency: "USD".into(),
            })
        );
        assert!(completion.completed_at_ms > 0);
        let encoded = serde_json::to_value(completion).unwrap();
        let decoded: UsageCompletion = serde_json::from_value(encoded).unwrap();
        assert_eq!(&decoded, completion);
        let specs = tasks.specs.lock().unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].trace_id.as_deref(), Some("trace-1"));
        drop(specs);
        let events = tasks.events.lock().unwrap();
        assert!(!events.is_empty());
        assert!(events.iter().all(|(_, _, data)| {
            data.pointer("/request/trace_id").and_then(Value::as_str) == Some("trace-1")
        }));
        assert_eq!(tasks.completed.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn taskmgr_commit_failure_keeps_execution_recoverable() {
        let providers = Arc::new(FakeProviders::default());
        providers
            .plans
            .lock()
            .unwrap()
            .push_back(StartPlan::Success(ProviderExecution::Immediate(output(
                "ok",
            ))));
        let (engine, store, tasks, usage) = make_engine(providers.clone());
        *tasks.commit_error.lock().unwrap() = Some(aicc_error(
            AiccErrorCode::InternalError,
            "taskmgr unavailable",
            true,
        ));

        let error = engine.execute(request(call("primary"))).await.unwrap_err();
        assert_eq!(error.message, "taskmgr unavailable");
        let record = store.get_task("task-1").await.unwrap().unwrap();
        assert_eq!(record.state, ExecutionState::Running);
        assert!(record.output.is_some());
        assert_eq!(store.recoverable().await.unwrap().len(), 1);
        assert_eq!(usage.writes.lock().unwrap().len(), 1);
        assert!(tasks.completed.lock().unwrap().is_empty());

        *tasks.commit_error.lock().unwrap() = None;
        let receipts = engine.recover().await.unwrap();
        assert_eq!(receipts[0].state, ExecutionState::Succeeded);
        assert_eq!(providers.starts.lock().unwrap().len(), 1);
        assert_eq!(tasks.completed.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn immediate_completion_uses_pinned_token_pricing() {
        let providers = Arc::new(FakeProviders::default());
        providers
            .plans
            .lock()
            .unwrap()
            .push_back(StartPlan::Success(ProviderExecution::Immediate(output(
                "priced-immediate",
            ))));
        let (engine, store, _, usage) = make_engine(providers);
        let receipt = engine
            .execute(request(token_priced_call("primary", 0.1, 0.2)))
            .await
            .unwrap();
        let completion = usage
            .writes
            .lock()
            .unwrap()
            .values()
            .next()
            .unwrap()
            .clone();
        assert_eq!(
            completion.finance_snapshot,
            Some(AiCost {
                amount: 0.4,
                currency: "USD".into(),
            })
        );
        assert!(store
            .get_task(&receipt.task_id)
            .await
            .unwrap()
            .unwrap()
            .binding
            .unwrap()
            .pricing
            .is_some());
    }

    #[tokio::test]
    async fn stream_completion_uses_pinned_token_pricing() {
        let providers = Arc::new(FakeProviders::default());
        providers
            .plans
            .lock()
            .unwrap()
            .push_back(StartPlan::Success(ProviderExecution::Stream(
                ProtocolStream {
                    events: Box::pin(stream::iter(vec![Ok(ProtocolEvent::Final(output(
                        "priced-stream",
                    )))])),
                },
            )));
        let (engine, _, _, usage) = make_engine(providers);
        engine
            .execute(request(token_priced_call("primary", 0.1, 0.2)))
            .await
            .unwrap();
        assert_eq!(
            usage
                .writes
                .lock()
                .unwrap()
                .values()
                .next()
                .unwrap()
                .finance_snapshot,
            Some(AiCost {
                amount: 0.4,
                currency: "USD".into(),
            })
        );
    }

    #[tokio::test]
    async fn concurrent_same_body_submission_executes_provider_once() {
        let providers = Arc::new(FakeProviders::default());
        providers
            .plans
            .lock()
            .unwrap()
            .push_back(StartPlan::Success(ProviderExecution::Immediate(output(
                "ok",
            ))));
        let (engine, _, _, usage) = make_engine(providers.clone());
        let (left, right) = tokio::join!(
            engine.execute(request(call("primary"))),
            engine.execute(request(call("primary")))
        );
        assert_eq!(left.unwrap().task_id, right.unwrap().task_id);
        assert_eq!(providers.starts.lock().unwrap().len(), 1);
        assert_eq!(usage.writes.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn same_scope_with_different_body_conflicts() {
        let providers = Arc::new(FakeProviders::default());
        providers
            .plans
            .lock()
            .unwrap()
            .push_back(StartPlan::Success(ProviderExecution::Immediate(output(
                "ok",
            ))));
        let (engine, _, _, _) = make_engine(providers);
        engine.execute(request(call("primary"))).await.unwrap();
        let mut conflict = request(call("primary"));
        conflict.canonical_body["prompt"] = json!("different");
        let error = engine.execute(conflict).await.unwrap_err();
        assert_eq!(error.code, AiccErrorCode::IdempotencyConflict);
    }

    #[tokio::test]
    async fn empty_user_is_rejected_before_task_or_provider_creation() {
        let providers = Arc::new(FakeProviders::default());
        let (engine, _, tasks, _) = make_engine(providers.clone());
        let mut request = request(call("primary"));
        request.user_id = " ".into();
        let error = engine.execute(request).await.unwrap_err();
        assert_eq!(error.code, AiccErrorCode::InvalidRequest);
        assert!(tasks.by_key.lock().unwrap().is_empty());
        assert!(providers.starts.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn empty_request_model_is_rejected_before_task_or_provider_creation() {
        let providers = Arc::new(FakeProviders::default());
        let (engine, _, tasks, _) = make_engine(providers.clone());
        let mut request = request(call("primary"));
        request.request_model = " ".into();
        let error = engine.execute(request).await.unwrap_err();
        assert_eq!(error.code, AiccErrorCode::InvalidRequest);
        assert!(tasks.by_key.lock().unwrap().is_empty());
        assert!(providers.starts.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn empty_trace_is_rejected_before_task_or_provider_creation() {
        let providers = Arc::new(FakeProviders::default());
        let (engine, _, tasks, _) = make_engine(providers.clone());
        let mut request = request(call("primary"));
        request.trace_id = Some(" ".into());
        let error = engine.execute(request).await.unwrap_err();
        assert_eq!(error.code, AiccErrorCode::InvalidRequest);
        assert!(tasks.by_key.lock().unwrap().is_empty());
        assert!(providers.starts.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn stream_progress_is_written_and_final_uses_common_completion() {
        let providers = Arc::new(FakeProviders::default());
        let events = vec![
            Ok(ProtocolEvent::Delta(json!({"partial_text": "a"}))),
            Ok(ProtocolEvent::Progress(json!({"tokens_generated": 1}))),
            Ok(ProtocolEvent::Final(output("ab"))),
        ];
        providers
            .plans
            .lock()
            .unwrap()
            .push_back(StartPlan::Success(ProviderExecution::Stream(
                ProtocolStream {
                    events: Box::pin(stream::iter(events)),
                },
            )));
        let (engine, _, tasks, usage) = make_engine(providers);
        let receipt = engine.execute(request(call("primary"))).await.unwrap();
        assert_eq!(receipt.state, ExecutionState::Succeeded);
        assert_eq!(receipt.output.as_ref().unwrap().value["text"], "ab");
        let event_kinds = tasks
            .events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(_, _, data)| data.pointer("/progress/status")?.as_str())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert_eq!(event_kinds, ["submitted", "running", "delta", "progress"]);
        assert_eq!(usage.writes.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn stream_provider_error_fails_without_usage_completion() {
        let providers = Arc::new(FakeProviders::default());
        providers
            .plans
            .lock()
            .unwrap()
            .push_back(StartPlan::Success(ProviderExecution::Stream(
                ProtocolStream {
                    events: Box::pin(stream::iter(vec![
                        Ok(ProtocolEvent::Delta(json!({"partial_text": "a"}))),
                        Err(ProtocolError::new(
                            ProtocolErrorKind::Authentication,
                            "stream Provider rejected the request",
                        )),
                    ])),
                },
            )));
        let (engine, _, tasks, usage) = make_engine(providers);
        let receipt = engine.execute(request(call("primary"))).await.unwrap();
        assert_eq!(receipt.state, ExecutionState::Failed);
        let error = receipt.error.unwrap();
        assert_eq!(error.code, AiccErrorCode::ProviderError);
        assert_eq!(error.message, "stream Provider rejected the request");
        assert!(usage.writes.lock().unwrap().is_empty());
        assert_eq!(tasks.failed.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn interrupted_stream_without_final_fails_without_usage_completion() {
        let providers = Arc::new(FakeProviders::default());
        providers
            .plans
            .lock()
            .unwrap()
            .push_back(StartPlan::Success(ProviderExecution::Stream(
                ProtocolStream {
                    events: Box::pin(stream::iter(vec![Ok(ProtocolEvent::Delta(json!({
                        "partial_text": "incomplete"
                    })))])),
                },
            )));
        let (engine, _, tasks, usage) = make_engine(providers);
        let receipt = engine.execute(request(call("primary"))).await.unwrap();
        assert_eq!(receipt.state, ExecutionState::Failed);
        let error = receipt.error.unwrap();
        assert_eq!(error.code, AiccErrorCode::ProviderError);
        assert_eq!(
            error.message,
            "Provider stream ended without a final result"
        );
        assert!(usage.writes.lock().unwrap().is_empty());
        assert_eq!(tasks.failed.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn stream_cancel_stops_local_consumption_and_keeps_cancelled_terminal() {
        let providers = Arc::new(FakeProviders::default());
        providers
            .plans
            .lock()
            .unwrap()
            .push_back(StartPlan::Success(ProviderExecution::Stream(
                ProtocolStream {
                    events: Box::pin(stream::pending()),
                },
            )));
        let (engine, store, tasks, usage) = make_engine(providers);
        let engine = Arc::new(engine);
        let running_engine = engine.clone();
        let running = tokio::spawn(async move {
            running_engine
                .execute(request(call("primary")))
                .await
                .unwrap()
        });
        let task_id = loop {
            if let Some(task_id) = tasks.by_key.lock().unwrap().values().next() {
                break task_id.task_id.clone();
            }
            tokio::task::yield_now().await;
        };
        while store.get_task(&task_id).await.unwrap().is_none() {
            tokio::task::yield_now().await;
        }
        assert!(engine.cancel("tenant-1", &task_id).await.unwrap());
        let receipt = running.await.unwrap();
        assert_eq!(receipt.state, ExecutionState::Cancelled);
        assert!(usage.writes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn transient_failure_retries_same_model_then_fails_over() {
        let providers = Arc::new(FakeProviders::default());
        providers.plans.lock().unwrap().extend([
            StartPlan::Failure(ProviderStartFailure::before_accept(
                ProtocolError::new(ProtocolErrorKind::Transport, "unavailable"),
                true,
            )),
            StartPlan::Failure(ProviderStartFailure::before_accept(
                ProtocolError::new(ProtocolErrorKind::Transport, "still unavailable"),
                true,
            )),
            StartPlan::Success(ProviderExecution::Immediate(output("fallback"))),
        ]);
        let (engine, _, _, _) = make_engine(providers.clone());
        let mut req = request(call("primary"));
        req.failover.push(call("backup"));
        req.runtime_failover = true;
        let receipt = engine.execute(req).await.unwrap();
        assert_eq!(receipt.state, ExecutionState::Succeeded);
        assert_eq!(
            providers.starts.lock().unwrap().as_slice(),
            ["model@primary", "model@primary", "model@backup"]
        );

        let providers = Arc::new(FakeProviders::default());
        providers.plans.lock().unwrap().extend([
            StartPlan::Failure(ProviderStartFailure::after_accept(ProtocolError::new(
                ProtocolErrorKind::Transport,
                "connection lost after submit",
            ))),
            StartPlan::Failure(ProviderStartFailure::after_accept(ProtocolError::new(
                ProtocolErrorKind::Transport,
                "connection still unavailable",
            ))),
            StartPlan::Success(ProviderExecution::Immediate(output("fallback"))),
        ]);
        let (engine, _, _, _) = make_engine(providers.clone());
        let mut req = request(call("primary"));
        req.idempotency_key = "idem-2".into();
        req.failover.push(call("backup"));
        req.runtime_failover = true;
        assert_eq!(
            engine.execute(req).await.unwrap().state,
            ExecutionState::Succeeded
        );
        assert_eq!(
            providers.starts.lock().unwrap().as_slice(),
            ["model@primary", "model@primary", "model@backup"]
        );
    }

    #[tokio::test]
    async fn unavailable_model_skips_same_model_retry_and_fails_over() {
        let providers = Arc::new(FakeProviders::default());
        providers.plans.lock().unwrap().extend([
            StartPlan::Failure(ProviderStartFailure::after_accept(
                ProtocolError::new(ProtocolErrorKind::InvalidRequest, "model does not exist")
                    .with_provider_code(Some("1211".to_owned())),
            )),
            StartPlan::Success(ProviderExecution::Immediate(output("fallback"))),
        ]);
        let (engine, _, _, _) = make_engine(providers.clone());
        let mut req = request(call("primary"));
        req.failover.push(call("backup"));
        req.runtime_failover = true;

        assert_eq!(
            engine.execute(req).await.unwrap().state,
            ExecutionState::Succeeded
        );
        assert_eq!(
            providers.starts.lock().unwrap().as_slice(),
            ["model@primary", "model@backup"]
        );
    }

    #[tokio::test]
    async fn native_task_is_pinned_and_recovers_after_engine_restart() {
        let providers = Arc::new(FakeProviders::default());
        let mut handle = NativeTaskHandle::new("remote-1").unwrap();
        handle.state = NativeTaskState::Queued;
        handle.poll_after = Some(Duration::from_secs(2));
        handle.cancel_supported = true;
        providers
            .plans
            .lock()
            .unwrap()
            .push_back(StartPlan::Success(ProviderExecution::NativeTask {
                handle,
                resume: resume_descriptor(),
            }));
        providers.polls.lock().unwrap().extend([
            NativeTaskPoll::Pending(
                NativeTaskState::Running,
                Some(json!({"frames_generated": 2})),
                None,
            ),
            NativeTaskPoll::Complete(output("video")),
        ]);
        *providers.completion_cost.lock().unwrap() = Some(AiCost {
            amount: 1.5,
            currency: "EUR".into(),
        });
        let (engine, store, tasks, usage) = make_engine(providers.clone());
        let started = engine.execute(request(call("primary"))).await.unwrap();
        assert_eq!(started.state, ExecutionState::Queued);
        assert_eq!(started.provider_task_ref.as_deref(), Some("remote-1"));
        assert_eq!(started.initial_poll_after, Some(Duration::from_secs(2)));
        let persisted = store.get_task(&started.task_id).await.unwrap().unwrap();
        assert_eq!(persisted.trace_id.as_deref(), Some("trace-1"));
        assert_eq!(persisted.user_id, "user-1");
        assert_eq!(persisted.request_model, "smart-chat");
        assert_eq!(persisted.usage_event_id, usage_event_id(&started.task_id));
        let binding = persisted.binding.as_ref().unwrap();
        assert_eq!(binding.runtime_generation, 7);
        assert_eq!(binding.resume.as_ref().unwrap(), &resume_descriptor());
        let encoded = serde_json::to_string(binding).unwrap();
        assert!(encoded.contains("system-config://secrets/aicc/fixed-provider"));
        assert!(!encoded.contains("plaintext-secret"));
        assert_eq!(providers.start_generations.lock().unwrap().as_slice(), [7]);

        let restarted = ExecutionEngine::new(store, tasks.clone(), providers, usage.clone());
        let recovered = restarted.recover().await.unwrap();
        assert_eq!(recovered[0].state, ExecutionState::Succeeded);
        let writes = usage.writes.lock().unwrap();
        assert_eq!(writes.len(), 1);
        let completion = writes.values().next().unwrap();
        assert_eq!(completion.event_id, usage_event_id(&started.task_id));
        assert_eq!(completion.trace_id.as_deref(), Some("trace-1"));
        assert_eq!(completion.user_id, "user-1");
        assert_eq!(completion.request_model, "smart-chat");
        assert_eq!(completion.provider_instance_name, "primary");
        assert_eq!(completion.provider_model, "model@primary");
        assert_eq!(
            completion.finance_snapshot,
            Some(AiCost {
                amount: 1.5,
                currency: "EUR".into(),
            })
        );
        assert!(completion.completed_at_ms > 0);
        drop(writes);
        assert!(tasks.events.lock().unwrap().iter().all(|(_, _, data)| {
            data.pointer("/request/trace_id").and_then(Value::as_str) == Some("trace-1")
        }));
        assert_eq!(
            tasks.completed.lock().unwrap().as_slice(),
            [(started.task_id, Some("trace-1".into()))]
        );
    }

    #[tokio::test]
    async fn native_task_honors_submit_poll_after_before_first_poll() {
        let providers = Arc::new(FakeProviders::default());
        let mut handle = NativeTaskHandle::new("remote-delayed").unwrap();
        handle.state = NativeTaskState::Queued;
        handle.poll_after = Some(Duration::from_millis(100));
        providers
            .plans
            .lock()
            .unwrap()
            .push_back(StartPlan::Success(ProviderExecution::NativeTask {
                handle,
                resume: resume_descriptor(),
            }));
        providers
            .polls
            .lock()
            .unwrap()
            .push_back(NativeTaskPoll::Complete(output("video")));
        let (engine, _, _, _) = make_engine(providers.clone());
        let started = engine.execute(request(call("primary"))).await.unwrap();
        let initial_poll_after = started.initial_poll_after;
        let task_id = started.task_id;
        let engine = Arc::new(engine);
        let drive = {
            let engine = Arc::clone(&engine);
            tokio::spawn(async move {
                engine
                    .drive_native_after(&task_id, initial_poll_after)
                    .await
            })
        };

        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(providers.polls.lock().unwrap().len(), 1);

        let completed = tokio::time::timeout(Duration::from_secs(1), drive)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(completed.state, ExecutionState::Succeeded);
        assert!(providers.polls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn native_recovery_uses_original_pricing_after_catalog_price_changes() {
        let providers = Arc::new(FakeProviders::default());
        let mut handle = NativeTaskHandle::new("remote-priced").unwrap();
        handle.state = NativeTaskState::Queued;
        providers
            .plans
            .lock()
            .unwrap()
            .push_back(StartPlan::Success(ProviderExecution::NativeTask {
                handle,
                resume: resume_descriptor(),
            }));
        providers
            .polls
            .lock()
            .unwrap()
            .push_back(NativeTaskPoll::Complete(output("priced-native")));
        let original_call = token_priced_call("primary", 0.1, 0.2);
        let (engine, store, tasks, usage) = make_engine(providers.clone());
        let started = engine.execute(request(original_call)).await.unwrap();
        let pinned = store
            .get_task(&started.task_id)
            .await
            .unwrap()
            .unwrap()
            .binding
            .unwrap()
            .pricing
            .unwrap();
        let updated_call = token_priced_call("primary", 10.0, 20.0);
        assert_ne!(
            pinned,
            PinnedPricingSnapshot::from_call(&updated_call)
                .unwrap()
                .unwrap()
        );

        let restarted = ExecutionEngine::new(store, tasks, providers, usage.clone());
        restarted.recover().await.unwrap();
        assert_eq!(
            usage
                .writes
                .lock()
                .unwrap()
                .values()
                .next()
                .unwrap()
                .finance_snapshot,
            Some(AiCost {
                amount: 0.4,
                currency: "USD".into(),
            })
        );
    }

    #[tokio::test]
    async fn recovery_fails_terminally_when_pinned_credential_is_unavailable() {
        let providers = Arc::new(FakeProviders::default());
        let mut handle = NativeTaskHandle::new("remote-credential-lost").unwrap();
        handle.state = NativeTaskState::Queued;
        providers
            .plans
            .lock()
            .unwrap()
            .push_back(StartPlan::Success(ProviderExecution::NativeTask {
                handle,
                resume: resume_descriptor(),
            }));
        let (engine, store, tasks, usage) = make_engine(providers.clone());
        let started = engine.execute(request(call("primary"))).await.unwrap();
        *providers.poll_error.lock().unwrap() = Some(NativeTaskResumeError::CredentialUnavailable);

        let restarted = ExecutionEngine::new(store.clone(), tasks.clone(), providers, usage);
        let recovered = restarted.recover().await.unwrap();
        assert_eq!(recovered[0].state, ExecutionState::Failed);
        let error = recovered[0].error.as_ref().unwrap();
        assert_eq!(error.code, AiccErrorCode::ProviderError);
        assert!(!error.retriable);
        assert_eq!(
            tasks.failed.lock().unwrap().as_slice(),
            [(started.task_id.clone(), Some("trace-1".into()))]
        );
        assert_eq!(
            store
                .get_task(&started.task_id)
                .await
                .unwrap()
                .unwrap()
                .state,
            ExecutionState::Failed
        );
    }

    #[tokio::test]
    async fn recovery_fails_orphan_submitted_task_and_still_resumes_healthy_tasks() {
        let providers = Arc::new(FakeProviders::default());
        let mut handle = NativeTaskHandle::new("remote-orphan-neighbor").unwrap();
        handle.state = NativeTaskState::Queued;
        providers
            .plans
            .lock()
            .unwrap()
            .push_back(StartPlan::Success(ProviderExecution::NativeTask {
                handle,
                resume: resume_descriptor(),
            }));
        let (engine, store, tasks, usage) = make_engine(providers.clone());

        let orphan_request = request(call("primary"));
        let orphan_scope = IdempotencyScope::new(
            orphan_request.tenant_id.clone(),
            orphan_request.primary.method.clone(),
            "orphan-1",
        )
        .unwrap();
        let orphan = ExecutionRecord {
            scope: orphan_scope,
            usage_event_id: "usage-orphan".into(),
            trace_id: orphan_request.trace_id.clone(),
            user_id: orphan_request.user_id.clone(),
            caller_app_id: orphan_request.caller_app_id.clone(),
            request_model: orphan_request.request_model.clone(),
            body_fingerprint: canonical_body_fingerprint(&orphan_request.canonical_body).unwrap(),
            task_id: "orphan-task".into(),
            event_ref: "orphan-task/events".into(),
            state: ExecutionState::Submitted,
            binding: None,
            output: None,
            error: None,
            created_at_ms: 1_000,
            expires_at_ms: 2_000,
        };
        store.claim(orphan).await.unwrap();

        let started = engine.execute(request(call("primary"))).await.unwrap();
        assert!(!started.state.is_terminal());
        *providers.poll_error.lock().unwrap() = Some(NativeTaskResumeError::CredentialUnavailable);

        let restarted = ExecutionEngine::new(store.clone(), tasks.clone(), providers, usage);
        let recovered = restarted.recover().await.unwrap();
        assert_eq!(recovered.len(), 2);

        let orphan_receipt = recovered
            .iter()
            .find(|receipt| receipt.task_id == "orphan-task")
            .expect("orphan task receipt");
        assert_eq!(orphan_receipt.state, ExecutionState::Failed);
        let orphan_error = orphan_receipt.error.as_ref().unwrap();
        assert_eq!(orphan_error.code, AiccErrorCode::InternalError);
        assert!(!orphan_error.retriable);
        assert_eq!(
            store.get_task("orphan-task").await.unwrap().unwrap().state,
            ExecutionState::Failed
        );

        let neighbor_receipt = recovered
            .iter()
            .find(|receipt| receipt.task_id == started.task_id)
            .expect("healthy task receipt");
        assert_eq!(neighbor_receipt.state, ExecutionState::Failed);
        assert_eq!(
            neighbor_receipt.error.as_ref().unwrap().code,
            AiccErrorCode::ProviderError
        );
        let failed_tasks: BTreeSet<String> = tasks
            .failed
            .lock()
            .unwrap()
            .iter()
            .map(|(task_id, _)| task_id.clone())
            .collect();
        assert_eq!(
            failed_tasks,
            BTreeSet::from(["orphan-task".to_string(), started.task_id.clone()])
        );
    }

    #[tokio::test]
    async fn cancel_fails_task_when_pinned_credential_is_unavailable() {
        let providers = Arc::new(FakeProviders::default());
        let mut handle = NativeTaskHandle::new("remote-cancel-credential-lost").unwrap();
        handle.state = NativeTaskState::Running;
        handle.cancel_supported = true;
        providers
            .plans
            .lock()
            .unwrap()
            .push_back(StartPlan::Success(ProviderExecution::NativeTask {
                handle,
                resume: resume_descriptor(),
            }));
        let (engine, store, tasks, _) = make_engine(providers.clone());
        let started = engine.execute(request(call("primary"))).await.unwrap();
        *providers.cancel_error.lock().unwrap() =
            Some(NativeTaskResumeError::CredentialUnavailable);

        let error = engine
            .cancel("tenant-1", &started.task_id)
            .await
            .unwrap_err();
        assert_eq!(error.code, AiccErrorCode::ProviderError);
        assert!(!error.retriable);
        assert_eq!(
            tasks.failed.lock().unwrap().as_slice(),
            [(started.task_id.clone(), Some("trace-1".into()))]
        );
        assert_eq!(
            store
                .get_task(&started.task_id)
                .await
                .unwrap()
                .unwrap()
                .state,
            ExecutionState::Failed
        );
    }

    #[tokio::test]
    async fn unsupported_native_cancel_returns_error_and_keeps_polling() {
        let providers = Arc::new(FakeProviders::default());
        let mut handle = NativeTaskHandle::new("remote-no-cancel").unwrap();
        handle.state = NativeTaskState::Running;
        handle.poll_after = Some(Duration::from_secs(60));
        handle.cancel_supported = false;
        providers
            .plans
            .lock()
            .unwrap()
            .push_back(StartPlan::Success(ProviderExecution::NativeTask {
                handle,
                resume: resume_descriptor(),
            }));
        let (engine, store, tasks, usage) = make_engine(providers.clone());
        let started = engine.execute(request(call("primary"))).await.unwrap();

        let error = engine
            .cancel("tenant-1", &started.task_id)
            .await
            .unwrap_err();
        assert_eq!(error.code, AiccErrorCode::UnsupportedOperation);
        assert_eq!(
            store
                .get_task(&started.task_id)
                .await
                .unwrap()
                .unwrap()
                .state,
            ExecutionState::Running
        );
        assert!(tasks.cancelled.lock().unwrap().is_empty());
        assert!(usage.writes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn cancel_is_tenant_scoped_and_blocks_late_completion() {
        let providers = Arc::new(FakeProviders::default());
        let mut handle = NativeTaskHandle::new("remote-1").unwrap();
        handle.state = NativeTaskState::Running;
        handle.cancel_supported = true;
        providers
            .plans
            .lock()
            .unwrap()
            .push_back(StartPlan::Success(ProviderExecution::NativeTask {
                handle,
                resume: resume_descriptor(),
            }));
        *providers.cancel_result.lock().unwrap() = true;
        let (engine, store, tasks, usage) = make_engine(providers);
        let receipt = engine.execute(request(call("primary"))).await.unwrap();
        let denied = engine
            .cancel("tenant-2", &receipt.task_id)
            .await
            .unwrap_err();
        assert_eq!(denied.code, AiccErrorCode::PolicyDenied);
        assert!(engine.cancel("tenant-1", &receipt.task_id).await.unwrap());
        assert!(!engine.cancel("tenant-1", &receipt.task_id).await.unwrap());
        assert_eq!(
            tasks.cancelled.lock().unwrap().as_slice(),
            [(receipt.task_id.clone(), Some("trace-1".into()))]
        );

        let late = ExecutionOutput::try_from(output("late")).unwrap();
        assert!(!store.try_complete(&receipt.task_id, late).await.unwrap());
        assert!(usage.writes.lock().unwrap().is_empty());
        assert_eq!(
            store
                .get_task(&receipt.task_id)
                .await
                .unwrap()
                .unwrap()
                .state,
            ExecutionState::Cancelled
        );
    }

    #[tokio::test]
    async fn native_task_without_upstream_cancel_returns_unsupported_and_keeps_running() {
        let providers = Arc::new(FakeProviders::default());
        let mut handle = NativeTaskHandle::new("remote-1").unwrap();
        handle.state = NativeTaskState::Running;
        providers
            .plans
            .lock()
            .unwrap()
            .push_back(StartPlan::Success(ProviderExecution::NativeTask {
                handle,
                resume: resume_descriptor(),
            }));
        let (engine, store, tasks, _) = make_engine(providers);
        let receipt = engine.execute(request(call("primary"))).await.unwrap();
        assert_eq!(
            engine
                .cancel("tenant-1", &receipt.task_id)
                .await
                .unwrap_err()
                .code,
            AiccErrorCode::UnsupportedOperation
        );
        assert_eq!(
            store
                .get_task(&receipt.task_id)
                .await
                .unwrap()
                .unwrap()
                .state,
            ExecutionState::Running
        );
        assert!(tasks.cancelled.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn missing_usage_keeps_success_and_records_zero_usage() {
        let providers = Arc::new(FakeProviders::default());
        providers
            .plans
            .lock()
            .unwrap()
            .push_back(StartPlan::Success(ProviderExecution::Immediate(
                ProtocolOutput::new(json!({"text": "ok"})),
            )));
        let (engine, _, tasks, usage) = make_engine(providers);
        let receipt = engine.execute(request(call("primary"))).await.unwrap();
        assert_eq!(receipt.state, ExecutionState::Succeeded);
        assert!(receipt.error.is_none());
        let writes = usage.writes.lock().unwrap();
        assert_eq!(writes.len(), 1);
        let recorded = writes.values().next().unwrap();
        assert!(recorded.usage.input_tokens.is_none());
        assert!(recorded.usage.output_tokens.is_none());
        assert!(recorded.usage.total_tokens.is_none());
        assert!(tasks.failed.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn invalid_final_cost_turns_success_into_provider_failure() {
        let providers = Arc::new(FakeProviders::default());
        providers
            .plans
            .lock()
            .unwrap()
            .push_back(StartPlan::Success(ProviderExecution::Immediate(output(
                "invalid-cost",
            ))));
        *providers.completion_cost.lock().unwrap() = Some(AiCost {
            amount: -0.01,
            currency: "USD".into(),
        });
        let (engine, _, tasks, usage) = make_engine(providers);
        let receipt = engine.execute(request(call("primary"))).await.unwrap();
        assert_eq!(receipt.state, ExecutionState::Failed);
        assert_eq!(receipt.error.unwrap().code, AiccErrorCode::ProviderError);
        assert!(usage.writes.lock().unwrap().is_empty());
        assert_eq!(
            tasks.failed.lock().unwrap().as_slice(),
            [("task-1".into(), Some("trace-1".into()))]
        );
    }
}

#[cfg(test)]
mod billing_boundary_tests {
    use super::*;
    use serde_json::json;
    fn pin(price: serde_json::Value) -> PinnedPricingSnapshot {
        PinnedPricingSnapshot::from_pricing(
            &serde_json::from_value(price).unwrap(),
            None,
            std::time::SystemTime::now(),
        )
        .unwrap()
        .unwrap()
    }
    #[test]
    fn missing_prices_missing_usage_and_uncovered_tiers_never_become_free() {
        let mut usage = AiUsage {
            input_tokens: Some(100),
            output_tokens: Some(10),
            ..Default::default()
        };
        let input_only = pin(json!({"currency":"USD","input_token":1.0}));
        assert!(input_only.completion_cost(&usage).is_none());
        usage.output_tokens = None;
        assert!(input_only.completion_cost(&usage).is_none());
        usage.output_tokens = Some(0);
        assert_eq!(input_only.completion_cost(&usage).unwrap().amount, 100.0);
        usage.cache_read_input_tokens = Some(10);
        assert!(input_only.completion_cost(&usage).is_none());
        usage.cache_read_input_tokens = None;
        usage.cache_write_input_tokens = Some(10);
        assert!(input_only.completion_cost(&usage).is_none());
        usage.cache_write_input_tokens = None;
        let tiered = pin(
            json!({"currency":"USD","input_token":1.0,"output_token":1.0,"tiers":{"dimension":"input_tokens","steps":[{"up_to":100,"input_token":0.5}]}}),
        );
        assert!(tiered.completion_cost(&usage).is_none());
        usage.input_tokens = Some(99);
        assert_eq!(tiered.completion_cost(&usage).unwrap().amount, 49.5);
    }
    #[test]
    fn multimodal_dimensions_are_charged_separately_and_not_twice() {
        let usage = AiUsage {
            input_tokens: Some(100),
            output_tokens: Some(50),
            audio_input_tokens: Some(20),
            image_input_tokens: Some(30),
            audio_output_tokens: Some(10),
            image_output_tokens: Some(20),
            ..Default::default()
        };
        let price = json!({"currency":"USD","input_token":1.0,"output_token":2.0,"audio_input_token":3.0,"image_input_token":4.0,"audio_output_token":5.0,"image_output_token":6.0});
        assert_eq!(
            pin(price.clone()).completion_cost(&usage).unwrap().amount,
            440.0
        );
        let mut missing = price;
        missing.as_object_mut().unwrap().remove("audio_input_token");
        assert!(pin(missing).completion_cost(&usage).is_none());
    }
    #[test]
    fn self_reported_cost_requires_declared_currency() {
        let mut binding: PinnedProviderTask=serde_json::from_value(json!({"runtime_generation":1,"origin_provider":"test","exact_model":"m@p","provider_model_id":"m","provider_instance_name":"p","protocol_adapter_id":"openai-responses","operation":"responses.create","api_type":"llm","remote_task_id":null,"result_artifacts":{},"cancel_supported":false,"resume":null,"pricing":null,"reported_cost_currency":null})).unwrap();
        let usage = AiUsage {
            reported_cost: Some(0.25),
            ..Default::default()
        };
        assert!(binding.completion_cost(&usage).is_none());
        binding.reported_cost_currency = Some("CNY".into());
        assert_eq!(
            binding.completion_cost(&usage),
            Some(AiCost {
                amount: 0.25,
                currency: "CNY".into()
            })
        );
    }
}
