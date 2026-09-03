#![allow(dead_code)]

use crate::call::ResolvedProviderCall;
use crate::protocol::{
    cancellation_pair, CancelHandle, Cancellation, NativeTaskState, ProtocolError,
    ProtocolErrorKind, ProtocolEvent, ProtocolExecution, ProtocolOutput, ProtocolStream,
};
use async_trait::async_trait;
use buckyos_api::{AiArtifact, AiUsage, AiccError, AiccErrorCode, ApiType};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
}

impl PinnedProviderTask {
    fn from_call(runtime_generation: u64, call: &ResolvedProviderCall) -> Self {
        Self {
            runtime_generation,
            exact_model: call.exact_model.clone(),
            provider_model_id: call.provider_model_id.clone(),
            provider_instance_name: call.provider_instance_name.clone(),
            protocol_adapter_id: call.protocol_adapter_id.clone(),
            operation: call.operation.clone(),
            api_type: call.api_type,
            remote_task_id: None,
            cancel_supported: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ExecutionRecord {
    pub scope: IdempotencyScope,
    pub caller_app_id: Option<String>,
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

#[derive(Debug, Clone)]
pub(crate) struct UsageCompletion {
    pub tenant_id: String,
    pub caller_app_id: Option<String>,
    pub task_id: String,
    pub idempotency_key: String,
    pub method: String,
    pub exact_model: String,
    pub provider_model: String,
    pub usage: AiUsage,
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

#[async_trait]
pub(crate) trait ProviderExecutionPort: Send + Sync {
    async fn start(
        &self,
        call: &ResolvedProviderCall,
        cancellation: Cancellation,
    ) -> Result<ProtocolExecution, ProviderStartFailure>;

    async fn poll_native(
        &self,
        binding: &PinnedProviderTask,
        cancellation: Cancellation,
    ) -> Result<NativeTaskPoll, ProtocolError>;

    async fn cancel_native(&self, binding: &PinnedProviderTask) -> Result<bool, ProtocolError>;
}

pub(crate) struct ExecutionRequest {
    pub tenant_id: String,
    pub caller_app_id: Option<String>,
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
                method: request.primary.method.clone(),
                idempotency_key: request.idempotency_key.clone(),
                parent_id: request.parent_task_id,
                input: request.canonical_body,
            })
            .await?;
        let initial = ExecutionRecord {
            scope,
            caller_app_id: request.caller_app_id,
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
            let mut binding = PinnedProviderTask::from_call(request.runtime_generation, call);
            match self.providers.start(call, cancellation.clone()).await {
                Ok(execution) => match execution {
                    ProtocolExecution::Immediate(output) => {
                        if !self
                            .store
                            .set_running(&record.task_id, ExecutionState::Running, binding.clone())
                            .await?
                        {
                            self.remove_active(&record.task_id);
                            return self.current_receipt(&record.task_id).await;
                        }
                        return self
                            .finish_success(
                                &record.task_id,
                                &record.scope,
                                &call.provider_model_id,
                                binding,
                                output,
                            )
                            .await;
                    }
                    ProtocolExecution::Stream(stream) => {
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
                                &call.provider_model_id,
                                binding,
                                stream,
                                cancellation.clone(),
                            )
                            .await;
                    }
                    ProtocolExecution::NativeTask(handle) => {
                        binding.remote_task_id = Some(handle.remote_task_id.clone());
                        binding.cancel_supported = handle.cancel_supported;
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
                    let provider_model_id = binding.provider_model_id.clone();
                    return self
                        .finish_success(task_id, &record.scope, &provider_model_id, binding, output)
                        .await;
                }
                Ok(NativeTaskPoll::Failed(error)) | Err(error) => {
                    return self.finish_failure(task_id, error.into()).await;
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
        if !self.providers.cancel_native(binding).await.unwrap_or(false)
            || !self.store.try_cancel(task_id).await?
        {
            return Ok(false);
        }
        self.tasks.cancel_task(task_id).await?;
        Ok(true)
    }

    async fn consume_stream(
        &self,
        task_id: &str,
        scope: &IdempotencyScope,
        provider_model: &str,
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
                    return self
                        .finish_success(task_id, scope, provider_model, binding, output)
                        .await;
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
        provider_model: &str,
        binding: PinnedProviderTask,
        output: ProtocolOutput,
    ) -> Result<ExecutionReceipt, AiccError> {
        let output = match ExecutionOutput::try_from(output) {
            Ok(output) => output,
            Err(error) => return self.finish_failure(task_id, error).await,
        };
        if let Err(error) = self
            .usage
            .write_once(UsageCompletion {
                tenant_id: scope.tenant_id.clone(),
                caller_app_id: self
                    .store
                    .get_task(task_id)
                    .await?
                    .and_then(|record| record.caller_app_id),
                task_id: task_id.to_string(),
                idempotency_key: scope.key.clone(),
                method: scope.method.clone(),
                exact_model: binding.exact_model,
                provider_model: provider_model.to_string(),
                usage: output.usage.clone(),
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
                .entry(completion.task_id.clone())
                .or_insert(completion);
            Ok(())
        }
    }

    enum StartPlan {
        Success(ProtocolExecution),
        Failure(ProviderStartFailure),
    }

    #[derive(Default)]
    struct FakeProviders {
        starts: Mutex<Vec<String>>,
        plans: Mutex<VecDeque<StartPlan>>,
        polls: Mutex<VecDeque<NativeTaskPoll>>,
        cancel_result: Mutex<bool>,
    }

    #[async_trait]
    impl ProviderExecutionPort for FakeProviders {
        async fn start(
            &self,
            call: &ResolvedProviderCall,
            _cancellation: Cancellation,
        ) -> Result<ProtocolExecution, ProviderStartFailure> {
            self.starts.lock().unwrap().push(call.exact_model.clone());
            match self.plans.lock().unwrap().pop_front().unwrap() {
                StartPlan::Success(result) => Ok(result),
                StartPlan::Failure(error) => Err(error),
            }
        }

        async fn poll_native(
            &self,
            _binding: &PinnedProviderTask,
            _cancellation: Cancellation,
        ) -> Result<NativeTaskPoll, ProtocolError> {
            Ok(self.polls.lock().unwrap().pop_front().unwrap())
        }

        async fn cancel_native(
            &self,
            _binding: &PinnedProviderTask,
        ) -> Result<bool, ProtocolError> {
            Ok(*self.cancel_result.lock().unwrap())
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

    fn request(primary: ResolvedProviderCall) -> ExecutionRequest {
        ExecutionRequest {
            tenant_id: "tenant-1".into(),
            caller_app_id: Some("app-1".into()),
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
            .push_back(StartPlan::Success(ProtocolExecution::Immediate(output(
                "ok",
            ))));
        let (engine, _, tasks, usage) = make_engine(providers.clone());
        let first = engine.execute(request(call("primary"))).await.unwrap();
        let replay = engine.execute(request(call("primary"))).await.unwrap();
        assert_eq!(first, replay);
        assert_eq!(first.state, ExecutionState::Succeeded);
        assert_eq!(providers.starts.lock().unwrap().len(), 1);
        assert_eq!(usage.writes.lock().unwrap().len(), 1);
        assert_eq!(tasks.completed.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn concurrent_same_body_submission_executes_provider_once() {
        let providers = Arc::new(FakeProviders::default());
        providers
            .plans
            .lock()
            .unwrap()
            .push_back(StartPlan::Success(ProtocolExecution::Immediate(output(
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
            .push_back(StartPlan::Success(ProtocolExecution::Immediate(output(
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
            .push_back(StartPlan::Success(ProtocolExecution::Stream(
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
            .push_back(StartPlan::Success(ProtocolExecution::Stream(
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
            StartPlan::Success(ProtocolExecution::Immediate(output("fallback"))),
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
            StartPlan::Success(ProtocolExecution::Immediate(output("must-not-run"))),
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
            .push_back(StartPlan::Success(ProtocolExecution::NativeTask(handle)));
        providers.polls.lock().unwrap().extend([
            NativeTaskPoll::Pending(
                NativeTaskState::Running,
                Some(json!({"frames_generated": 2})),
            ),
            NativeTaskPoll::Complete(output("video")),
        ]);
        let (engine, store, tasks, usage) = make_engine(providers.clone());
        let started = engine.execute(request(call("primary"))).await.unwrap();
        assert_eq!(started.state, ExecutionState::Queued);
        assert_eq!(started.provider_task_ref.as_deref(), Some("remote-1"));

        let restarted = ExecutionEngine::new(store, tasks, providers, usage.clone());
        let recovered = restarted.recover().await.unwrap();
        assert_eq!(recovered[0].state, ExecutionState::Succeeded);
        assert_eq!(usage.writes.lock().unwrap().len(), 1);
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
            .push_back(StartPlan::Success(ProtocolExecution::NativeTask(handle)));
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
            .push_back(StartPlan::Success(ProtocolExecution::NativeTask(handle)));
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
            .push_back(StartPlan::Success(ProtocolExecution::Immediate(
                ProtocolOutput::new(json!({"text": "bad"})),
            )));
        let (engine, _, tasks, usage) = make_engine(providers);
        let receipt = engine.execute(request(call("primary"))).await.unwrap();
        assert_eq!(receipt.state, ExecutionState::Failed);
        assert_eq!(receipt.error.unwrap().code, AiccErrorCode::ProviderError);
        assert!(usage.writes.lock().unwrap().is_empty());
        assert_eq!(tasks.failed.lock().unwrap().len(), 1);
    }
}
