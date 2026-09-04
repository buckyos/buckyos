#![allow(dead_code)]

use crate::call::ResolvedProviderCall;
use crate::catalog::PricingUnit;
use crate::protocol::{
    cancellation_pair, CancelHandle, Cancellation, NativeTaskHandle, NativeTaskState,
    ProtocolError, ProtocolErrorKind, ProtocolEvent, ProtocolOutput, ProtocolStream,
};
use async_trait::async_trait;
use buckyos_api::{AiArtifact, AiCost, AiUsage, AiccError, AiccErrorCode, ApiType, Capability};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_IDEMPOTENCY_WINDOW_MS: u64 = 24 * 60 * 60 * 1_000;

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
    fn is_terminal(self) -> bool {
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
    pub artifacts: Vec<AiArtifact>,
}

impl TryFrom<ProtocolOutput> for ExecutionOutput {
    type Error = AiccError;

    fn try_from(value: ProtocolOutput) -> Result<Self, Self::Error> {
        let usage = value.usage.ok_or_else(|| {
            aicc_error(
                AiccErrorCode::ProviderError,
                "successful Provider completion is missing usage",
                false,
            )
        })?;
        if usage.input_tokens.is_none()
            && usage.output_tokens.is_none()
            && usage.total_tokens.is_none()
            && usage.request_units.is_none()
        {
            return Err(aicc_error(
                AiccErrorCode::ProviderError,
                "successful Provider completion contains empty usage",
                false,
            ));
        }
        Ok(Self {
            value: value.value,
            usage,
            artifacts: value.artifacts,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct PinnedProviderTask {
    pub runtime_generation: u64,
    pub exact_model: String,
    pub provider_model_id: String,
    pub provider_instance_name: String,
    pub protocol_adapter_id: String,
    pub operation: String,
    pub api_type: ApiType,
    pub remote_task_id: Option<String>,
    pub cancel_supported: bool,
    pub resume: Option<NativeTaskResumeDescriptor>,
    pub pricing: Option<PinnedPricingSnapshot>,
}

impl PinnedProviderTask {
    fn from_call(runtime_generation: u64, call: &ResolvedProviderCall) -> Result<Self, AiccError> {
        Ok(Self {
            runtime_generation,
            exact_model: call.exact_model.clone(),
            provider_model_id: call.provider_model_id.clone(),
            provider_instance_name: call.provider_instance_name.clone(),
            protocol_adapter_id: call.protocol_adapter_id.clone(),
            operation: call.operation.clone(),
            api_type: call.api_type,
            remote_task_id: None,
            cancel_supported: false,
            resume: None,
            pricing: PinnedPricingSnapshot::from_call(call)?,
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
        output_token: Option<f64>,
    },
    Units {
        unit: PricingUnit,
        amount: f64,
    },
}

impl PinnedPricingSnapshot {
    fn from_call(call: &ResolvedProviderCall) -> Result<Option<Self>, AiccError> {
        let Some(pricing) = call.pricing.pricing.as_ref() else {
            return Ok(None);
        };
        let currency = pricing.currency.trim().to_ascii_uppercase();
        if currency.is_empty() {
            return Err(invalid_pinned_pricing());
        }
        let has_token_price = pricing.input_token.is_some() || pricing.output_token.is_some();
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
                output_token: pricing.output_token,
            }
        } else if let (Some(unit), Some(amount)) = (pricing.unit, call.pricing.matched_amount) {
            if invalid_price(amount) {
                return Err(invalid_pinned_pricing());
            }
            PinnedPricingBasis::Units { unit, amount }
        } else {
            return Ok(None);
        };
        Ok(Some(Self { currency, basis }))
    }

    fn completion_cost(&self, usage: &AiUsage) -> Option<AiCost> {
        let amount = match self.basis {
            PinnedPricingBasis::Tokens {
                input_token,
                cache_input_token: _,
                output_token,
            } => {
                let input = match input_token {
                    Some(rate) => usage.input_tokens? as f64 * rate,
                    None => 0.0,
                };
                let output = match output_token {
                    Some(rate) => usage.output_tokens? as f64 * rate,
                    None => 0.0,
                };
                input + output
            }
            PinnedPricingBasis::Units { amount, .. } => usage.request_units? as f64 * amount,
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
    pub method: String,
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
    async fn cancel_task(&self, task_id: &str) -> Result<(), AiccError>;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct UsageCompletion {
    pub event_id: String,
    pub tenant_id: String,
    pub user_id: String,
    pub caller_app_id: Option<String>,
    pub task_id: String,
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
}

impl ProviderStartFailure {
    pub(crate) fn before_accept(error: ProtocolError, retryable: bool) -> Self {
        Self {
            error,
            provider_accepted: false,
            retryable,
        }
    }

    pub(crate) fn after_accept(error: ProtocolError) -> Self {
        Self {
            error,
            provider_accepted: true,
            retryable: false,
        }
    }
}

#[derive(Debug)]
pub(crate) enum NativeTaskPoll {
    Pending(NativeTaskState, Option<Value>),
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

#[derive(Debug, Clone)]
pub(crate) enum NativeTaskResumeError {
    CredentialUnavailable,
    Protocol(ProtocolError),
}

impl NativeTaskResumeError {
    fn into_aicc_error(self) -> AiccError {
        match self {
            Self::CredentialUnavailable => aicc_error(
                AiccErrorCode::ProviderError,
                "pinned native task credential can no longer be resolved",
                false,
            ),
            Self::Protocol(error) => error.into(),
        }
    }
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
        binding
            .pricing
            .as_ref()?
            .completion_cost(output.usage.as_ref()?)
    }
}

pub(crate) struct ExecutionRequest {
    pub tenant_id: String,
    pub user_id: String,
    pub caller_app_id: Option<String>,
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
                method: request.primary.method.clone(),
                idempotency_key: request.idempotency_key.clone(),
                parent_id: request.parent_task_id,
                input: request.canonical_body,
            })
            .await?;
        let initial = ExecutionRecord {
            scope,
            usage_event_id: usage_event_id(&task.task_id),
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
                json_state("submitted", None),
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
            match self
                .providers
                .start(request.runtime_generation, call, cancellation.clone())
                .await
            {
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
                                json_state("running", None),
                            )
                            .await?;
                        return self
                            .consume_stream(
                                &record.task_id,
                                &record.scope,
                                binding,
                                stream,
                                cancellation.clone(),
                            )
                            .await;
                    }
                    ProviderExecution::NativeTask { handle, resume } => {
                        if let Err(error) = resume.validate() {
                            return self.finish_failure(&record.task_id, error).await;
                        }
                        binding.remote_task_id = Some(handle.remote_task_id.clone());
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
                                json_state("provider_task_started", None),
                            )
                            .await?;
                        self.remove_active(&record.task_id);
                        return self.current_receipt(&record.task_id).await;
                    }
                },
                Err(failure) => {
                    let can_failover = !failure.provider_accepted
                        && failure.retryable
                        && request.runtime_failover
                        && index + 1 < calls.len();
                    let error: AiccError = failure.error.into();
                    if can_failover {
                        self.tasks
                            .report_state(
                                &record.task_id,
                                ExecutionState::Submitted,
                                json_state(
                                    "runtime_failover",
                                    Some(Value::String(call.exact_model.clone())),
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
        let binding = record.binding.clone().ok_or_else(|| {
            aicc_error(
                AiccErrorCode::InternalError,
                "running task has no pinned Provider binding",
                false,
            )
        })?;
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
        self.active
            .lock()
            .expect("active execution lock")
            .insert(task_id.to_string(), ActiveExecution { cancel });
        loop {
            if cancellation.is_cancelled() {
                self.remove_active(task_id);
                return self.current_receipt(task_id).await;
            }
            match self
                .providers
                .poll_native(&binding, cancellation.clone())
                .await
            {
                Ok(NativeTaskPoll::Pending(state, progress)) => {
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
                            json_state("provider_progress", progress),
                        )
                        .await?;
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
        let mut receipts = Vec::with_capacity(records.len());
        for record in records {
            receipts.push(self.drive_native(&record.task_id).await?);
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
            self.tasks.cancel_task(task_id).await?;
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
        self.tasks.cancel_task(task_id).await?;
        Ok(true)
    }

    async fn consume_stream(
        &self,
        task_id: &str,
        scope: &IdempotencyScope,
        binding: PinnedProviderTask,
        mut stream: ProtocolStream,
        cancellation: Cancellation,
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
                            json_state("delta", Some(delta)),
                        )
                        .await?;
                }
                Ok(ProtocolEvent::Progress(progress)) => {
                    self.tasks
                        .report_state(
                            task_id,
                            ExecutionState::Running,
                            json_state("progress", Some(progress)),
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
        let output = match ExecutionOutput::try_from(output) {
            Ok(output) => output,
            Err(error) => return self.finish_failure(task_id, error).await,
        };
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
        if self.store.try_complete(task_id, output.clone()).await? {
            self.tasks.commit_result(task_id, &output).await?;
        }
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

fn json_state(kind: &str, value: Option<Value>) -> Value {
    let mut progress = Map::new();
    progress.insert("kind".into(), Value::String(kind.into()));
    if let Some(value) = value {
        progress.insert("value".into(), value);
    }
    Value::Object(Map::from_iter([(
        "aicc".into(),
        Value::Object(Map::from_iter([(
            "progress".into(),
            Value::Object(progress),
        )])),
    )]))
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
    use crate::protocol::{
        CodecContext, CodecInput, CodecLimits, CredentialAudit, CredentialKind, NativeTaskHandle,
        ResolvedCredential,
    };
    use buckyos_api::{AiccCall, LlmChatInvokeRequest};
    use futures_util::stream;
    use serde_json::json;
    use std::collections::VecDeque;
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
        events: Mutex<Vec<(String, ExecutionState, Value)>>,
        completed: Mutex<Vec<String>>,
        failed: Mutex<Vec<String>>,
        cancelled: Mutex<Vec<String>>,
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
            let mut next = self.next_id.lock().unwrap();
            *next += 1;
            let binding = TaskBinding {
                task_id: format!("task-{next}"),
                event_ref: format!("task-{next}/events"),
            };
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
            self.completed.lock().unwrap().push(task_id.into());
            Ok(())
        }

        async fn fail_task(&self, task_id: &str, _error: &AiccError) -> Result<(), AiccError> {
            self.failed.lock().unwrap().push(task_id.into());
            Ok(())
        }

        async fn cancel_task(&self, task_id: &str) -> Result<(), AiccError> {
            self.cancelled.lock().unwrap().push(task_id.into());
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
            self.completion_cost.lock().unwrap().clone().or_else(|| {
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
            request_timeout_ms: 10_000,
            max_request_bytes: 1_024,
            max_response_bytes: 2_048,
        }
    }

    fn call(instance: &str) -> ResolvedProviderCall {
        let request = LlmChatInvokeRequest::new(format!("model@{instance}"), Vec::new());
        ResolvedProviderCall {
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
            input: CodecInput {
                canonical_request: AiccCall::ChatCompletionsCreate(request),
                resolved_parameters: BTreeMap::new(),
            },
            context: CodecContext {
                base_url: "https://fake.invalid".into(),
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
            resource_requirements: Vec::new(),
            pricing: ResolvedPricing {
                source: PricingSource::RouteEstimate,
                pricing: None,
                matched_amount: None,
                estimated_cost_usd: None,
            },
            revisions: LoweringRevisions {
                catalog_target_seq: 1,
                model_driver_revision_seq: 1,
                provider_rules_revision_seq: 1,
                inventory_revision: "inv-1".into(),
            },
        }
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
                currency: "USD".into(),
                input_token: Some(input_token),
                output_token: Some(output_token),
                cache_input_token: None,
                estimated_cost: Some(999.0),
                unit: None,
                amount: None,
                rules: Vec::new(),
            }),
            matched_amount: None,
            estimated_cost_usd: Some(777.0),
        };
        call
    }

    fn request(primary: ResolvedProviderCall) -> ExecutionRequest {
        ExecutionRequest {
            tenant_id: "tenant-1".into(),
            user_id: "user-1".into(),
            caller_app_id: Some("app-1".into()),
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
        assert!(tasks.events.lock().unwrap().len() >= 4);
        assert_eq!(usage.writes.lock().unwrap().len(), 1);
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
    async fn startup_failover_stops_after_provider_acceptance() {
        let providers = Arc::new(FakeProviders::default());
        providers.plans.lock().unwrap().extend([
            StartPlan::Failure(ProviderStartFailure::before_accept(
                ProtocolError::new(ProtocolErrorKind::Transport, "unavailable"),
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
            ["model@primary", "model@backup"]
        );

        let providers = Arc::new(FakeProviders::default());
        providers.plans.lock().unwrap().extend([
            StartPlan::Failure(ProviderStartFailure::after_accept(ProtocolError::new(
                ProtocolErrorKind::Transport,
                "connection lost after submit",
            ))),
            StartPlan::Success(ProviderExecution::Immediate(output("must-not-run"))),
        ]);
        let (engine, _, _, _) = make_engine(providers.clone());
        let mut req = request(call("primary"));
        req.idempotency_key = "idem-2".into();
        req.failover.push(call("backup"));
        req.runtime_failover = true;
        assert_eq!(
            engine.execute(req).await.unwrap().state,
            ExecutionState::Failed
        );
        assert_eq!(providers.starts.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn native_task_is_pinned_and_recovers_after_engine_restart() {
        let providers = Arc::new(FakeProviders::default());
        let mut handle = NativeTaskHandle::new("remote-1").unwrap();
        handle.state = NativeTaskState::Queued;
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
        let persisted = store.get_task(&started.task_id).await.unwrap().unwrap();
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

        let restarted = ExecutionEngine::new(store, tasks, providers, usage.clone());
        let recovered = restarted.recover().await.unwrap();
        assert_eq!(recovered[0].state, ExecutionState::Succeeded);
        let writes = usage.writes.lock().unwrap();
        assert_eq!(writes.len(), 1);
        let completion = writes.values().next().unwrap();
        assert_eq!(completion.event_id, usage_event_id(&started.task_id));
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
            [started.task_id.clone()]
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
            [started.task_id.clone()]
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
        assert_eq!(tasks.cancelled.lock().unwrap().len(), 1);

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
    async fn idle_native_task_without_upstream_cancel_returns_not_accepted() {
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
        assert!(!engine.cancel("tenant-1", &receipt.task_id).await.unwrap());
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
    async fn missing_usage_turns_success_into_provider_failure() {
        let providers = Arc::new(FakeProviders::default());
        providers
            .plans
            .lock()
            .unwrap()
            .push_back(StartPlan::Success(ProviderExecution::Immediate(
                ProtocolOutput::new(json!({"text": "bad"})),
            )));
        let (engine, _, tasks, usage) = make_engine(providers);
        let receipt = engine.execute(request(call("primary"))).await.unwrap();
        assert_eq!(receipt.state, ExecutionState::Failed);
        assert_eq!(receipt.error.unwrap().code, AiccErrorCode::ProviderError);
        assert!(usage.writes.lock().unwrap().is_empty());
        assert_eq!(tasks.failed.lock().unwrap().len(), 1);
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
        assert_eq!(tasks.failed.lock().unwrap().as_slice(), ["task-1"]);
    }
}
