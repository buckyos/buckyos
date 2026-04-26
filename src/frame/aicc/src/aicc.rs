use crate::aicc_usage_log_db::AiccUsageLogDb;
use crate::complete_request_queue::QUEUE_STATUS_QUEUED;
use crate::model_registry::{
    InventoryRefreshScheduler, ModelRegistry, DEFAULT_INVENTORY_REFRESH_INTERVAL,
};
use crate::model_router::{ModelRouter, RouteRequest};
use crate::model_scheduler::{ModelScheduler, StickyBindingKey, StickyBindingStore};
use crate::model_session::{merge_session_config, SessionConfig, SessionConfigStore};
use crate::model_types::{
    ApiType, CostClass, CostEstimateInput, CostEstimateOutput, HealthStatus, LatencyClass,
    ModelAttributes, ModelCandidate, ModelCapabilities, ModelHealth, ModelMetadata, ModelPricing,
    PolicyConfig, PricingMode, PrivacyClass, ProviderInventory, ProviderOrigin, ProviderType,
    ProviderTypeTrustedSource, QuotaState, RequiredModelFeatures, RoutePolicy, RouteTrace,
    UserFacingProviderOrigin, UserFacingRouteSummary,
};
use ::kRPC::*;
use async_trait::async_trait;
use base64::engine::general_purpose;
use base64::Engine as _;
use buckyos_api::{
    ai_methods, AiMethodRequest, AiMethodResponse, AiMethodStatus, AiResponseSummary, AiccHandler,
    AiccUsageEvent, CancelResponse, Capability, CreateTaskOptions, Feature, ResourceRef,
    TaskManagerClient, TaskStatus, AICC_SERVICE_SERVICE_NAME,
};
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex as AsyncMutex;

const DEFAULT_FALLBACK_LIMIT: usize = 2;
const DEFAULT_BASE64_MAX_BYTES: usize = 8 * 1024 * 1024;
const EWMA_ALPHA: f64 = 0.2;
const AICC_TASK_TYPE: &str = "aicc.compute";
const AICC_TASK_EVENT_RETENTION: usize = 64;
const REDACTED_BASE64_PLACEHOLDER: &str = "[redacted_base64]";
const SN_AI_PROVIDER_FREE_CREDIT_USD: f64 = 15.0;

#[derive(Clone, Debug, Default)]
pub struct InvokeCtx {
    pub tenant_id: String,
    pub caller_app_id: Option<String>,
    pub session_token: Option<String>,
    pub trace_id: Option<String>,
}

impl InvokeCtx {
    pub fn from_rpc(ctx: &RPCContext) -> Self {
        let session_token = ctx.token.clone();
        let mut tenant_id = "anonymous".to_string();
        let mut caller_app_id: Option<String> = None;

        if let Some(token) = session_token.as_ref() {
            if !token.trim().is_empty() {
                if let Ok(parsed) = RPCSessionToken::from_string(token.as_str()) {
                    if let Ok((sub, appid)) = parsed.get_subs() {
                        tenant_id = sub;
                        caller_app_id = Some(appid);
                    } else {
                        tenant_id = token.clone();
                    }
                } else {
                    tenant_id = token.clone();
                }
            }
        }

        Self {
            tenant_id,
            caller_app_id,
            session_token,
            trace_id: ctx.trace_id.clone(),
        }
    }
}

fn redact_base64_fields(value: &mut Value) {
    match value {
        Value::Array(items) => {
            for item in items {
                redact_base64_fields(item);
            }
        }
        Value::Object(map) => {
            if let Some(data_base64) = map.get_mut("data_base64") {
                *data_base64 = json!(REDACTED_BASE64_PLACEHOLDER);
            }
            for nested in map.values_mut() {
                redact_base64_fields(nested);
            }
        }
        _ => {}
    }
}

fn redacted_summary_value(summary: &AiResponseSummary) -> Value {
    let mut value = serde_json::to_value(summary).unwrap_or_else(|_| json!({}));
    redact_base64_fields(&mut value);
    value
}

#[derive(Clone, Debug)]
pub struct ProviderInstance {
    pub provider_instance_name: String,
    pub provider_type: ProviderType,
    pub provider_driver: String,
    pub provider_origin: ProviderOrigin,
    pub provider_type_trusted_source: ProviderTypeTrustedSource,
    pub provider_type_revision: Option<String>,
    pub capabilities: Vec<Capability>,
    pub features: Vec<Feature>,
    pub endpoint: Option<String>,
    pub plugin_key: Option<String>,
}

impl ProviderInstance {
    pub fn supports_capability(&self, capability: &Capability) -> bool {
        self.capabilities.iter().any(|item| item == capability)
    }

    pub fn supports_features(&self, required_features: &[Feature]) -> bool {
        required_features
            .iter()
            .all(|feature| self.features.iter().any(|item| item == feature))
    }
}

#[derive(Clone, Debug, Default)]
pub struct CostEstimate {
    pub estimated_cost_usd: Option<f64>,
    pub estimated_latency_ms: Option<u64>,
}

impl From<&CostEstimateOutput> for CostEstimate {
    fn from(value: &CostEstimateOutput) -> Self {
        Self {
            estimated_cost_usd: Some(value.estimated_cost_usd),
            estimated_latency_ms: value.estimated_latency_ms,
        }
    }
}

impl From<CostEstimate> for CostEstimateOutput {
    fn from(value: CostEstimate) -> Self {
        Self {
            estimated_cost_usd: value.estimated_cost_usd.unwrap_or(1.0),
            pricing_mode: PricingMode::Unknown,
            quota_state: QuotaState::Unknown,
            confidence: 0.0,
            estimated_latency_ms: value.estimated_latency_ms,
        }
    }
}

#[derive(Clone, Debug)]
struct SnAIProviderBillingAdjustment {
    raw_cost_usd: f64,
    billed_cost_usd: f64,
    credit_applied_usd: f64,
    remaining_credit_usd: f64,
}

#[derive(Clone, Default)]
struct SnAIProviderBillingLedger {
    spent_raw_cost_usd: Arc<RwLock<HashMap<String, f64>>>,
}

impl SnAIProviderBillingLedger {
    fn preview_billed_cost(
        &self,
        tenant_id: &str,
        provider_driver: &str,
        raw_cost_usd: f64,
    ) -> Option<f64> {
        let raw_cost_usd = raw_cost_usd.max(0.0);
        if provider_driver != "sn-ai-provider" {
            return Some(raw_cost_usd);
        }

        let spent_raw_cost_usd = self
            .spent_raw_cost_usd
            .read()
            .ok()
            .and_then(|items| items.get(tenant_id).copied())
            .unwrap_or(0.0)
            .max(0.0);
        Some(
            Self::adjust_from_spent(spent_raw_cost_usd, raw_cost_usd)
                .billed_cost_usd
                .max(0.0),
        )
    }

    fn apply_charge(
        &self,
        tenant_id: &str,
        provider_driver: &str,
        raw_cost_usd: Option<f64>,
    ) -> Option<SnAIProviderBillingAdjustment> {
        if provider_driver != "sn-ai-provider" {
            return None;
        }

        let raw_cost_usd = raw_cost_usd?.max(0.0);
        let mut spent = self.spent_raw_cost_usd.write().ok()?;
        let spent_raw_cost_usd = spent.get(tenant_id).copied().unwrap_or(0.0).max(0.0);
        let adjustment = Self::adjust_from_spent(spent_raw_cost_usd, raw_cost_usd);
        spent.insert(tenant_id.to_string(), spent_raw_cost_usd + raw_cost_usd);
        Some(adjustment)
    }

    fn adjust_from_spent(
        spent_raw_cost_usd: f64,
        raw_cost_usd: f64,
    ) -> SnAIProviderBillingAdjustment {
        let remaining_credit_usd = (SN_AI_PROVIDER_FREE_CREDIT_USD - spent_raw_cost_usd).max(0.0);
        let credit_applied_usd = raw_cost_usd.min(remaining_credit_usd).max(0.0);
        let billed_cost_usd = (raw_cost_usd - credit_applied_usd).max(0.0);

        SnAIProviderBillingAdjustment {
            raw_cost_usd,
            billed_cost_usd,
            credit_applied_usd,
            remaining_credit_usd: (remaining_credit_usd - credit_applied_usd).max(0.0),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProviderError {
    message: String,
    retryable: bool,
}

impl ProviderError {
    pub fn retryable(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: true,
        }
    }

    pub fn fatal(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: false,
        }
    }

    pub fn is_retryable(&self) -> bool {
        self.retryable
    }
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.retryable {
            write!(f, "retryable: {}", self.message)
        } else {
            write!(f, "fatal: {}", self.message)
        }
    }
}

impl std::error::Error for ProviderError {}

#[derive(Debug, Clone)]
pub enum ProviderStartResult {
    Immediate(AiResponseSummary),
    Started,
    Queued { position: usize },
}

#[derive(Clone, Debug)]
pub struct ResolvedRequest {
    pub method: String,
    pub request: AiMethodRequest,
}

impl ResolvedRequest {
    pub fn new(request: AiMethodRequest) -> Self {
        let method = default_method_for_capability(&request.capability).to_string();
        Self { method, request }
    }

    pub fn new_with_method(method: impl Into<String>, request: AiMethodRequest) -> Self {
        Self {
            method: method.into(),
            request,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskEventKind {
    Queued,
    Started,
    Final,
    Error,
    CancelRequested,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskEvent {
    pub task_id: String,
    pub kind: TaskEventKind,
    pub timestamp_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

#[async_trait]
pub trait TaskEventSink: Send + Sync {
    fn event_ref(&self) -> Option<String>;
    async fn emit(&self, event: TaskEvent) -> std::result::Result<(), RPCErrors>;
}

pub trait TaskEventSinkFactory: Send + Sync {
    fn build(&self, _ctx: &InvokeCtx, task_id: &str) -> Arc<dyn TaskEventSink>;
}

#[derive(Debug)]
pub struct MemoryTaskEventSink {
    event_ref: Option<String>,
    events: Mutex<Vec<TaskEvent>>,
}

impl MemoryTaskEventSink {
    pub fn new(event_ref: Option<String>) -> Self {
        Self {
            event_ref,
            events: Mutex::new(vec![]),
        }
    }

    pub fn events(&self) -> Vec<TaskEvent> {
        self.events
            .lock()
            .map(|events| events.clone())
            .unwrap_or_default()
    }
}

#[async_trait]
impl TaskEventSink for MemoryTaskEventSink {
    fn event_ref(&self) -> Option<String> {
        self.event_ref.clone()
    }

    async fn emit(&self, event: TaskEvent) -> std::result::Result<(), RPCErrors> {
        let mut events = self.events.lock().map_err(|_| {
            RPCErrors::ReasonError("internal_error: event sink lock poisoned".to_string())
        })?;
        events.push(event);
        Ok(())
    }
}

#[derive(Default)]
pub struct DefaultTaskEventSinkFactory;

impl TaskEventSinkFactory for DefaultTaskEventSinkFactory {
    fn build(&self, _ctx: &InvokeCtx, task_id: &str) -> Arc<dyn TaskEventSink> {
        Arc::new(MemoryTaskEventSink::new(Some(format!(
            "task://{}/events",
            task_id
        ))))
    }
}

struct TaskAuditSink {
    taskmgr: Arc<TaskManagerClient>,
    task_mgr_id: i64,
    lock: AsyncMutex<()>,
}

impl TaskAuditSink {
    fn new(taskmgr: Arc<TaskManagerClient>, task_mgr_id: i64) -> Self {
        Self {
            taskmgr,
            task_mgr_id,
            lock: AsyncMutex::new(()),
        }
    }
}

struct DeferredTaskEventSinkState {
    delegate: Option<Arc<dyn TaskEventSink>>,
    buffered: Vec<TaskEvent>,
}

struct DeferredTaskEventSink {
    inner: Arc<dyn TaskEventSink>,
    state: AsyncMutex<DeferredTaskEventSinkState>,
}

impl DeferredTaskEventSink {
    fn new(inner: Arc<dyn TaskEventSink>) -> Self {
        Self {
            inner,
            state: AsyncMutex::new(DeferredTaskEventSinkState {
                delegate: None,
                buffered: vec![],
            }),
        }
    }

    async fn promote(
        &self,
        delegate: Arc<dyn TaskEventSink>,
    ) -> std::result::Result<(), RPCErrors> {
        let buffered = {
            let mut state = self.state.lock().await;
            state.delegate = Some(delegate.clone());
            std::mem::take(&mut state.buffered)
        };

        for event in buffered {
            delegate.emit(event).await?;
        }
        Ok(())
    }
}

#[async_trait]
impl TaskEventSink for DeferredTaskEventSink {
    fn event_ref(&self) -> Option<String> {
        self.inner.event_ref()
    }

    async fn emit(&self, event: TaskEvent) -> std::result::Result<(), RPCErrors> {
        self.inner.emit(event.clone()).await?;
        let delegate = {
            let mut state = self.state.lock().await;
            if let Some(delegate) = state.delegate.as_ref() {
                Some(delegate.clone())
            } else {
                state.buffered.push(event.clone());
                None
            }
        };

        if let Some(delegate) = delegate {
            delegate.emit(event).await?;
        }
        Ok(())
    }
}

/// Snapshot of everything the usage-log writer needs to build one durable
/// `aicc_usage_event` row. Captured once at routing time so the wrapping sink
/// can persist usage without re-reading the request on every event.
#[derive(Clone, Debug)]
struct UsageLogContext {
    external_task_id: String,
    tenant_id: String,
    caller_app_id: Option<String>,
    capability: String,
    request_model: String,
    provider_model: String,
    idempotency_key: Option<String>,
}

/// Wraps an underlying task event sink. When a `Final` event flows through it
/// and the provider reported `usage`, a row is written to the usage-log db
/// exactly once. Missing `usage` on a successful Final is logged as a
/// protocol error per section 5 of the requirements doc — we do not invent
/// placeholder usage rows.
struct UsageLoggingSink {
    inner: Arc<dyn TaskEventSink>,
    db: Arc<AiccUsageLogDb>,
    context: UsageLogContext,
}

impl UsageLoggingSink {
    fn new(
        inner: Arc<dyn TaskEventSink>,
        db: Arc<AiccUsageLogDb>,
        context: UsageLogContext,
    ) -> Self {
        Self { inner, db, context }
    }

    async fn record_usage(&self, data: &Value) {
        let summary = match data.get("summary") {
            Some(value) => value,
            None => {
                warn!(
                    "aicc.usage_log skipped: task_id={} tenant={} reason=missing_summary",
                    self.context.external_task_id, self.context.tenant_id
                );
                return;
            }
        };

        let usage = match summary.get("usage") {
            Some(value) if !value.is_null() => value.clone(),
            _ => {
                warn!(
                    "aicc.usage_log skipped: task_id={} tenant={} reason=missing_usage_protocol_error",
                    self.context.external_task_id, self.context.tenant_id
                );
                return;
            }
        };

        let input_tokens = usage.get("input_tokens").and_then(Value::as_u64);
        let output_tokens = usage.get("output_tokens").and_then(Value::as_u64);
        let total_tokens = usage.get("total_tokens").and_then(Value::as_u64);
        let request_units = usage.get("request_units").and_then(Value::as_u64);

        let finance_snapshot = build_finance_snapshot(summary);

        let event = AiccUsageEvent {
            event_id: format!("usage-{}", self.context.external_task_id),
            tenant_id: self.context.tenant_id.clone(),
            caller_app_id: self.context.caller_app_id.clone(),
            task_id: self.context.external_task_id.clone(),
            idempotency_key: self.context.idempotency_key.clone(),
            capability: self.context.capability.clone(),
            request_model: self.context.request_model.clone(),
            provider_model: self.context.provider_model.clone(),
            input_tokens,
            output_tokens,
            total_tokens,
            request_units,
            usage_json: usage,
            finance_snapshot_json: finance_snapshot,
            created_at_ms: now_ms() as i64,
        };

        match self.db.insert_usage_event(&event).await {
            Ok(true) => {
                info!(
                    "aicc.usage_log wrote: task_id={} tenant={} provider_model={} input_tokens={:?} output_tokens={:?}",
                    event.task_id,
                    event.tenant_id,
                    event.provider_model,
                    event.input_tokens,
                    event.output_tokens
                );
            }
            Ok(false) => {
                info!(
                    "aicc.usage_log duplicate_skipped: task_id={} tenant={} idempotency_key={:?}",
                    event.task_id, event.tenant_id, event.idempotency_key
                );
            }
            Err(err) => {
                warn!(
                    "aicc.usage_log write_failed: task_id={} tenant={} err={}",
                    event.task_id, event.tenant_id, err
                );
            }
        }
    }
}

fn build_finance_snapshot(summary: &Value) -> Option<Value> {
    let mut snapshot = Map::new();
    if let Some(cost) = summary.get("cost") {
        if let Some(amount) = cost.get("amount") {
            snapshot.insert("amount".to_string(), amount.clone());
        }
        if let Some(currency) = cost.get("currency") {
            snapshot.insert("currency".to_string(), currency.clone());
        }
    }
    if let Some(provider_task_ref) = summary.get("provider_task_ref") {
        if !provider_task_ref.is_null() {
            snapshot.insert("provider_trace_id".to_string(), provider_task_ref.clone());
        }
    }
    if let Some(extra) = summary.get("extra") {
        if let Some(billing) = extra.get("billing") {
            snapshot.insert("billing".to_string(), billing.clone());
        }
    }
    if snapshot.is_empty() {
        None
    } else {
        Some(Value::Object(snapshot))
    }
}

#[async_trait]
impl TaskEventSink for UsageLoggingSink {
    fn event_ref(&self) -> Option<String> {
        self.inner.event_ref()
    }

    async fn emit(&self, event: TaskEvent) -> std::result::Result<(), RPCErrors> {
        if matches!(event.kind, TaskEventKind::Final) {
            if let Some(data) = event.data.as_ref() {
                self.record_usage(data).await;
            } else {
                warn!(
                    "aicc.usage_log skipped: task_id={} tenant={} reason=missing_event_data",
                    self.context.external_task_id, self.context.tenant_id
                );
            }
        }
        self.inner.emit(event).await
    }
}

struct TaskScope {
    create_opts: CreateTaskOptions,
    user_id: String,
    app_id: String,
}

impl TaskScope {
    fn parent_id(&self) -> Option<i64> {
        self.create_opts.parent_id
    }
}

struct PreparedTask {
    taskmgr: Arc<TaskManagerClient>,
    task: buckyos_api::Task,
}

impl PreparedTask {
    fn id(&self) -> i64 {
        self.task.id
    }
}

#[derive(Clone, Copy)]
enum InitialTaskState {
    Running,
    Queued,
}

impl InitialTaskState {
    fn as_status(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Queued => "queued",
        }
    }
}

impl AIComputeCenter {
    async fn resolve_task_scope(
        &self,
        request: &AiMethodRequest,
        invoke_ctx: &InvokeCtx,
        external_task_id: &str,
    ) -> std::result::Result<TaskScope, RPCErrors> {
        let mut create_task_opts = CreateTaskOptions::default();
        if let Some(task_options) = request.task_options.as_ref() {
            create_task_opts.parent_id = task_options.parent_id;
        }

        let taskmgr = self.taskmgr.as_ref().cloned().ok_or_else(|| {
            warn!(
                "aicc.complete failed: task_manager_unavailable task_id={} tenant={}",
                external_task_id, invoke_ctx.tenant_id
            );
            reason_error("task_manager_unavailable", "task manager is not configured")
        })?;

        let parent_id = create_task_opts.parent_id;
        let mut task_user_id = invoke_ctx.tenant_id.clone();
        let mut task_app_id = AICC_SERVICE_SERVICE_NAME.to_string();
        if let Some(pid) = parent_id {
            let parent_task = match taskmgr.get_task(pid).await {
                Ok(task) => task,
                Err(err) => {
                    warn!(
                        "aicc.complete load_parent_task failed: task_id={} tenant={} parent_id={} err={}",
                        external_task_id, invoke_ctx.tenant_id, pid, err
                    );
                    return Err(err);
                }
            };
            task_user_id = parent_task.user_id;
            task_app_id = parent_task.app_id;
            info!(
                "aicc.complete inherit_parent_scope: task_id={} parent_id={} user_id={} app_id={}",
                external_task_id, pid, task_user_id, task_app_id
            );
        }

        Ok(TaskScope {
            create_opts: create_task_opts,
            user_id: task_user_id,
            app_id: task_app_id,
        })
    }

    async fn create_provider_task(
        &self,
        external_task_id: &str,
        request: &AiMethodRequest,
        invoke_ctx: &InvokeCtx,
        event_ref: Option<&str>,
        decision: &RouteDecision,
        initial_state: InitialTaskState,
    ) -> std::result::Result<PreparedTask, RPCErrors> {
        let scope = self
            .resolve_task_scope(request, invoke_ctx, external_task_id)
            .await?;

        let mut task_data =
            build_initial_aicc_task_data(request, external_task_id, event_ref, invoke_ctx);
        merge_route_decision_into_task_data(&mut task_data, decision, initial_state.as_status());

        let taskmgr = self.taskmgr.as_ref().cloned().ok_or_else(|| {
            reason_error("task_manager_unavailable", "task manager is not configured")
        })?;
        let parent_id = scope.parent_id();
        let task = taskmgr
            .create_task(
                &format!("aicc:{external_task_id}"),
                AICC_TASK_TYPE,
                Some(task_data.clone()),
                scope.user_id.as_str(),
                scope.app_id.as_str(),
                Some(scope.create_opts),
            )
            .await
            .map_err(|err| {
                warn!(
                    "aicc.complete create_task failed: task_id={} tenant={} parent_id={:?} err={}",
                    external_task_id, invoke_ctx.tenant_id, parent_id, err
                );
                err
            })?;

        Ok(PreparedTask { taskmgr, task })
    }
}

#[async_trait]
impl TaskEventSink for TaskAuditSink {
    fn event_ref(&self) -> Option<String> {
        None
    }

    async fn emit(&self, event: TaskEvent) -> std::result::Result<(), RPCErrors> {
        let _guard = self.lock.lock().await;

        let task = self.taskmgr.get_task(self.task_mgr_id).await?;
        let mut data = task.data;
        merge_task_data_with_event(&mut data, &event);

        match event.kind {
            TaskEventKind::Queued => {
                let message = event
                    .data
                    .as_ref()
                    .and_then(|value| value.get("message"))
                    .and_then(|value| value.as_str())
                    .unwrap_or(QUEUE_STATUS_QUEUED)
                    .to_string();
                self.taskmgr
                    .update_task(
                        self.task_mgr_id,
                        Some(TaskStatus::Pending),
                        Some(0.0),
                        Some(message),
                        Some(data),
                    )
                    .await?;
            }
            TaskEventKind::Started => {
                let message = event
                    .data
                    .as_ref()
                    .and_then(|value| value.get("message"))
                    .and_then(|value| value.as_str())
                    .unwrap_or("aicc provider started")
                    .to_string();
                self.taskmgr
                    .update_task(
                        self.task_mgr_id,
                        Some(TaskStatus::Running),
                        Some(0.1),
                        Some(message),
                        Some(data),
                    )
                    .await?;
            }
            TaskEventKind::Final => {
                self.taskmgr
                    .update_task(
                        self.task_mgr_id,
                        Some(TaskStatus::Completed),
                        Some(1.0),
                        Some("aicc task completed".to_string()),
                        Some(data),
                    )
                    .await?;
            }
            TaskEventKind::Error => {
                let message = event
                    .data
                    .as_ref()
                    .and_then(|value| value.get("message"))
                    .and_then(|value| value.as_str())
                    .unwrap_or("aicc task failed")
                    .to_string();
                self.taskmgr
                    .update_task(
                        self.task_mgr_id,
                        Some(TaskStatus::Failed),
                        Some(1.0),
                        Some(message),
                        Some(data),
                    )
                    .await?;
            }
            TaskEventKind::CancelRequested => {
                self.taskmgr
                    .update_task(
                        self.task_mgr_id,
                        Some(TaskStatus::Canceled),
                        Some(1.0),
                        Some("aicc task canceled".to_string()),
                        Some(data),
                    )
                    .await?;
            }
        }
        Ok(())
    }
}

#[async_trait]
pub trait ResourceResolver: Send + Sync {
    async fn resolve(
        &self,
        _ctx: &InvokeCtx,
        req: &AiMethodRequest,
    ) -> std::result::Result<ResolvedRequest, RPCErrors>;
}

#[derive(Default)]
pub struct PassthroughResourceResolver;

#[async_trait]
impl ResourceResolver for PassthroughResourceResolver {
    async fn resolve(
        &self,
        _ctx: &InvokeCtx,
        req: &AiMethodRequest,
    ) -> std::result::Result<ResolvedRequest, RPCErrors> {
        Ok(ResolvedRequest::new(req.clone()))
    }
}

#[async_trait]
pub trait Provider: Send + Sync {
    fn inventory(&self) -> ProviderInventory;
    fn legacy_instance(&self) -> Option<&ProviderInstance> {
        None
    }
    fn estimate_cost(&self, input: &CostEstimateInput) -> CostEstimateOutput;
    async fn start(
        &self,
        ctx: InvokeCtx,
        provider_model: String,
        req: ResolvedRequest,
        sink: Arc<dyn TaskEventSink>,
    ) -> std::result::Result<ProviderStartResult, ProviderError>;
    async fn cancel(&self, ctx: InvokeCtx, task_id: &str)
        -> std::result::Result<(), ProviderError>;
}

#[derive(Clone, Debug, Default)]
pub struct ProviderMetrics {
    pub in_flight: u64,
    pub ewma_latency_ms: f64,
    pub ewma_error_rate: f64,
}

#[derive(Clone)]
struct ProviderEntry {
    provider: Arc<dyn Provider>,
    metrics: ProviderMetrics,
}

#[derive(Clone, Debug)]
pub struct RegistryCandidate {
    pub inventory: ProviderInventory,
    pub metrics: ProviderMetrics,
}

#[derive(Clone, Debug, Default)]
pub struct RegistrySnapshot {
    pub candidates: Vec<RegistryCandidate>,
}

#[derive(Clone, Default)]
pub struct Registry {
    entries: Arc<RwLock<HashMap<String, ProviderEntry>>>,
}

impl Registry {
    pub fn add_provider(&self, provider: Arc<dyn Provider>) -> ProviderInventory {
        let inventory = provider.inventory();
        let mut entries = self
            .entries
            .write()
            .expect("registry lock should be available");
        entries.insert(
            inventory.provider_instance_name.clone(),
            ProviderEntry {
                provider,
                metrics: ProviderMetrics::default(),
            },
        );
        inventory
    }

    pub fn remove_instance(&self, provider_instance_name: &str) {
        let mut entries = self
            .entries
            .write()
            .expect("registry lock should be available");
        entries.remove(provider_instance_name);
    }

    pub fn clear(&self) {
        let mut entries = self
            .entries
            .write()
            .expect("registry lock should be available");
        entries.clear();
    }

    pub fn snapshot(&self, capability: Capability) -> RegistrySnapshot {
        let entries = self
            .entries
            .read()
            .expect("registry lock should be available");
        let candidates = entries
            .values()
            .map(|entry| (entry.provider.inventory(), entry.metrics.clone()))
            .filter(|(inventory, _)| inventory_supports_capability(inventory, &capability))
            .map(|entry| RegistryCandidate {
                inventory: entry.0,
                metrics: entry.1,
            })
            .collect::<Vec<_>>();

        RegistrySnapshot { candidates }
    }

    pub fn get_provider(&self, instance_id: &str) -> Option<Arc<dyn Provider>> {
        let entries = self.entries.read().ok()?;
        entries.get(instance_id).map(|entry| entry.provider.clone())
    }

    pub fn provider_count(&self) -> usize {
        self.entries
            .read()
            .map(|entries| entries.len())
            .unwrap_or(0)
    }

    pub fn inventory(&self, provider_instance_name: &str) -> Option<ProviderInventory> {
        let entries = self.entries.read().ok()?;
        entries
            .get(provider_instance_name)
            .map(|entry| entry.provider.inventory())
    }

    pub fn inventories(&self) -> Vec<ProviderInventory> {
        self.entries
            .read()
            .map(|entries| {
                entries
                    .values()
                    .map(|entry| entry.provider.inventory())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn mark_start_begin(&self, instance_id: &str) {
        if let Ok(mut entries) = self.entries.write() {
            if let Some(entry) = entries.get_mut(instance_id) {
                entry.metrics.in_flight = entry.metrics.in_flight.saturating_add(1);
            }
        }
    }

    pub fn record_start_success(&self, instance_id: &str, latency_ms: f64) {
        if let Ok(mut entries) = self.entries.write() {
            if let Some(entry) = entries.get_mut(instance_id) {
                entry.metrics.in_flight = entry.metrics.in_flight.saturating_sub(1);
                entry.metrics.ewma_latency_ms = ewma(
                    entry.metrics.ewma_latency_ms,
                    latency_ms.max(0.0),
                    EWMA_ALPHA,
                );
                entry.metrics.ewma_error_rate =
                    ewma(entry.metrics.ewma_error_rate, 0.0, EWMA_ALPHA).clamp(0.0, 1.0);
            }
        }
    }

    pub fn record_start_failure(&self, instance_id: &str, latency_ms: f64) {
        if let Ok(mut entries) = self.entries.write() {
            if let Some(entry) = entries.get_mut(instance_id) {
                entry.metrics.in_flight = entry.metrics.in_flight.saturating_sub(1);
                entry.metrics.ewma_latency_ms = ewma(
                    entry.metrics.ewma_latency_ms,
                    latency_ms.max(0.0),
                    EWMA_ALPHA,
                );
                entry.metrics.ewma_error_rate =
                    ewma(entry.metrics.ewma_error_rate, 1.0, EWMA_ALPHA).clamp(0.0, 1.0);
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct RouteWeights {
    pub w_cost: f64,
    pub w_latency: f64,
    pub w_load: f64,
    pub w_error: f64,
}

impl Default for RouteWeights {
    fn default() -> Self {
        Self {
            w_cost: 0.35,
            w_latency: 0.35,
            w_load: 0.2,
            w_error: 0.1,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct TenantRouteConfig {
    pub allow_provider_types: Option<Vec<String>>,
    pub deny_provider_types: Option<Vec<String>>,
    pub weights: Option<RouteWeights>,
}

#[derive(Clone, Debug)]
pub struct RouteConfig {
    pub global_weights: RouteWeights,
    pub tenant_overrides: HashMap<String, TenantRouteConfig>,
    pub fallback_limit: usize,
}

impl Default for RouteConfig {
    fn default() -> Self {
        Self {
            global_weights: RouteWeights::default(),
            tenant_overrides: HashMap::new(),
            fallback_limit: DEFAULT_FALLBACK_LIMIT,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
struct ModelMappingKey {
    capability: Capability,
    alias: String,
    provider_type: String,
}

#[derive(Clone, Default)]
pub struct ModelCatalog {
    mappings: Arc<RwLock<HashMap<ModelMappingKey, String>>>,
    tenant_overrides: Arc<RwLock<HashMap<(String, ModelMappingKey), String>>>,
}

impl ModelCatalog {
    pub fn set_mapping(
        &self,
        capability: Capability,
        alias: impl Into<String>,
        provider_type: impl Into<String>,
        provider_model: impl Into<String>,
    ) {
        let key = ModelMappingKey {
            capability,
            alias: alias.into(),
            provider_type: provider_type.into(),
        };
        if let Ok(mut mappings) = self.mappings.write() {
            mappings.insert(key, provider_model.into());
        }
    }

    pub fn set_tenant_mapping(
        &self,
        tenant_id: impl Into<String>,
        capability: Capability,
        alias: impl Into<String>,
        provider_type: impl Into<String>,
        provider_model: impl Into<String>,
    ) {
        let key = ModelMappingKey {
            capability,
            alias: alias.into(),
            provider_type: provider_type.into(),
        };
        if let Ok(mut mappings) = self.tenant_overrides.write() {
            mappings.insert((tenant_id.into(), key), provider_model.into());
        }
    }

    pub fn resolve(
        &self,
        tenant_id: &str,
        capability: &Capability,
        alias: &str,
        provider_type: &str,
    ) -> Option<String> {
        let key = ModelMappingKey {
            capability: capability.clone(),
            alias: alias.to_string(),
            provider_type: provider_type.to_string(),
        };

        if let Ok(tenant_map) = self.tenant_overrides.read() {
            if let Some(model) = tenant_map.get(&(tenant_id.to_string(), key.clone())) {
                return Some(model.clone());
            }
        }

        let mappings = self.mappings.read().ok()?;
        mappings.get(&key).cloned()
    }

    pub fn clear(&self) {
        if let Ok(mut mappings) = self.mappings.write() {
            mappings.clear();
        }
        if let Ok(mut tenant_overrides) = self.tenant_overrides.write() {
            tenant_overrides.clear();
        }
    }

    pub fn snapshot(&self) -> Vec<ModelCatalogEntry> {
        let mut out = vec![];
        if let Ok(mappings) = self.mappings.read() {
            for (key, provider_model) in mappings.iter() {
                out.push(ModelCatalogEntry {
                    capability: key.capability.clone(),
                    alias: key.alias.clone(),
                    provider_type: key.provider_type.clone(),
                    provider_model: provider_model.clone(),
                    tenant_id: None,
                });
            }
        }
        if let Ok(tenant) = self.tenant_overrides.read() {
            for ((tenant_id, key), provider_model) in tenant.iter() {
                out.push(ModelCatalogEntry {
                    capability: key.capability.clone(),
                    alias: key.alias.clone(),
                    provider_type: key.provider_type.clone(),
                    provider_model: provider_model.clone(),
                    tenant_id: Some(tenant_id.clone()),
                });
            }
        }
        out.sort_by(|left, right| {
            left.alias
                .cmp(&right.alias)
                .then_with(|| left.provider_type.cmp(&right.provider_type))
                .then_with(|| left.provider_model.cmp(&right.provider_model))
        });
        out
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ModelCatalogEntry {
    pub capability: Capability,
    pub alias: String,
    pub provider_type: String,
    pub provider_model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
}

#[derive(Clone, Debug)]
struct RouteAttempt {
    instance_id: String,
    provider_model: String,
    exact_model: String,
}

#[derive(Clone, Debug)]
pub struct RouteDecision {
    pub primary_instance_id: String,
    pub fallback_instance_ids: Vec<String>,
    pub provider_model: String,
    attempts: Vec<RouteAttempt>,
    route_trace: Arc<Mutex<RouteTrace>>,
    runtime_failover_enabled: bool,
    sticky_key: Option<StickyBindingKey>,
}

impl RouteDecision {
    fn attempts(&self) -> &[RouteAttempt] {
        &self.attempts
    }
}

#[derive(Clone, Default)]
pub struct Router;

impl Router {
    #[allow(dead_code)]
    pub fn route(
        &self,
        tenant_id: &str,
        req: &AiMethodRequest,
        snapshot: &RegistrySnapshot,
        registry: &Registry,
        route_cfg: &RouteConfig,
        model_catalog: &ModelCatalog,
    ) -> std::result::Result<RouteDecision, RPCErrors> {
        self.route_with_billing(
            tenant_id,
            req,
            snapshot,
            registry,
            route_cfg,
            model_catalog,
            None,
        )
    }

    fn route_with_billing(
        &self,
        tenant_id: &str,
        req: &AiMethodRequest,
        snapshot: &RegistrySnapshot,
        registry: &Registry,
        route_cfg: &RouteConfig,
        model_catalog: &ModelCatalog,
        sn_ai_provider_billing: Option<&SnAIProviderBillingLedger>,
    ) -> std::result::Result<RouteDecision, RPCErrors> {
        if snapshot.candidates.is_empty() {
            return Err(reason_error(
                "no_provider_available",
                "no provider instance supports requested capability",
            ));
        }

        let tenant_cfg = route_cfg.tenant_overrides.get(tenant_id);
        let weights = tenant_cfg
            .and_then(|cfg| cfg.weights.clone())
            .unwrap_or_else(|| route_cfg.global_weights.clone());

        let allow_set = tenant_cfg
            .and_then(|cfg| cfg.allow_provider_types.clone())
            .map(|items| items.into_iter().collect::<HashSet<_>>());
        let deny_set = tenant_cfg
            .and_then(|cfg| cfg.deny_provider_types.clone())
            .map(|items| items.into_iter().collect::<HashSet<_>>());

        let mut alias_mapped = false;
        let mut scored = vec![];
        let (input_tokens, output_tokens) = estimate_request_tokens(req);

        for candidate in snapshot.candidates.iter() {
            let instance_id = candidate.inventory.provider_instance_name.as_str();
            let Some(provider) = registry.get_provider(instance_id) else {
                continue;
            };
            let legacy_instance = provider.legacy_instance();
            let provider_type = legacy_instance
                .map(|instance| instance.provider_driver.as_str())
                .filter(|value| !value.is_empty())
                .unwrap_or(candidate.inventory.provider_driver.as_str());
            let provider_model = model_catalog.resolve(
                tenant_id,
                &req.capability,
                req.model.alias.as_str(),
                provider_type,
            );
            if provider_model.is_some() {
                alias_mapped = true;
            }

            if let Some(instance) = legacy_instance {
                if !instance.supports_features(&req.requirements.must_features) {
                    continue;
                }
            }

            if let Some(allow) = allow_set.as_ref() {
                if !allow.contains(provider_type) {
                    continue;
                }
            }
            if let Some(deny) = deny_set.as_ref() {
                if deny.contains(provider_type) {
                    continue;
                }
            }

            let Some(provider_model) = provider_model else {
                continue;
            };

            let estimate = provider.estimate_cost(&CostEstimateInput {
                api_type: api_type_for_capability(&req.capability).unwrap_or(ApiType::LlmChat),
                exact_model: exact_model_name(provider_model.as_str(), instance_id),
                input_tokens,
                estimated_output_tokens: Some(output_tokens),
                cached_input_tokens: None,
                request_features: req.requirements.must_features.clone(),
            });
            let compat_estimate = CostEstimate::from(&estimate);
            let effective_estimated_cost = compat_estimate.estimated_cost_usd.and_then(|cost| {
                sn_ai_provider_billing
                    .and_then(|billing| billing.preview_billed_cost(tenant_id, provider_type, cost))
                    .or(Some(cost))
            });
            if let Some(max_cost) = req.requirements.max_cost_usd {
                if let Some(estimated_cost) = effective_estimated_cost {
                    if estimated_cost > max_cost {
                        continue;
                    }
                }
            }

            let predicted_latency_ms = if candidate.metrics.ewma_latency_ms > 0.0 {
                candidate.metrics.ewma_latency_ms
            } else {
                compat_estimate.estimated_latency_ms.unwrap_or(0) as f64
            };

            if let Some(max_latency_ms) = req.requirements.max_latency_ms {
                if predicted_latency_ms > max_latency_ms as f64 {
                    continue;
                }
            }

            scored.push(ScoredRouteCandidate {
                instance_id: instance_id.to_string(),
                provider_model,
                cost: effective_estimated_cost.unwrap_or(1.0).max(0.0),
                latency: predicted_latency_ms.max(0.0),
                load: candidate.metrics.in_flight as f64,
                error: candidate.metrics.ewma_error_rate.clamp(0.0, 1.0),
                score: 0.0,
            });
        }

        if scored.is_empty() {
            if !alias_mapped {
                return Err(reason_error(
                    "model_alias_not_mapped",
                    format!(
                        "alias '{}' is not mapped for capability '{:?}'",
                        req.model.alias, req.capability
                    ),
                ));
            }
            return Err(reason_error(
                "no_provider_available",
                "all candidate providers were filtered out by policy or requirements",
            ));
        }

        let cost_range = range(scored.iter().map(|item| item.cost));
        let latency_range = range(scored.iter().map(|item| item.latency));
        let load_range = range(scored.iter().map(|item| item.load));
        let error_range = range(scored.iter().map(|item| item.error));

        for item in scored.iter_mut() {
            let cost_score = normalize(item.cost, cost_range.0, cost_range.1);
            let latency_score = normalize(item.latency, latency_range.0, latency_range.1);
            let load_score = normalize(item.load, load_range.0, load_range.1);
            let error_score = normalize(item.error, error_range.0, error_range.1);
            item.score = (weights.w_cost * cost_score)
                + (weights.w_latency * latency_score)
                + (weights.w_load * load_score)
                + (weights.w_error * error_score);
        }

        scored.sort_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(Ordering::Equal));

        let attempts = scored
            .into_iter()
            .map(|item| RouteAttempt {
                exact_model: exact_model_name(
                    item.provider_model.as_str(),
                    item.instance_id.as_str(),
                ),
                instance_id: item.instance_id,
                provider_model: item.provider_model,
            })
            .collect::<Vec<_>>();

        let primary = attempts
            .first()
            .cloned()
            .ok_or_else(|| reason_error("no_provider_available", "no route candidate generated"))?;

        let fallback_limit = route_cfg.fallback_limit.max(1);
        let fallback_instance_ids = attempts
            .iter()
            .skip(1)
            .take(fallback_limit)
            .map(|item| item.instance_id.clone())
            .collect::<Vec<_>>();

        let final_attempts = std::iter::once(primary.clone())
            .chain(attempts.iter().skip(1).take(fallback_limit).cloned())
            .collect::<Vec<_>>();

        Ok(RouteDecision {
            primary_instance_id: primary.instance_id.clone(),
            fallback_instance_ids,
            provider_model: primary.provider_model.clone(),
            attempts: final_attempts,
            route_trace: Arc::new(Mutex::new(legacy_route_trace(
                req.model.alias.clone(),
                api_type_for_capability(&req.capability).unwrap_or(ApiType::LlmChat),
            ))),
            runtime_failover_enabled: true,
            sticky_key: None,
        })
    }
}

fn legacy_route_trace(model: String, api_type: ApiType) -> RouteTrace {
    RouteTrace {
        request_id: String::new(),
        session_id: None,
        session_config_revision: None,
        session_config_updated: false,
        api_type,
        requested_model: model,
        requested_model_type: crate::model_types::RequestedModelType::Logical,
        resolved_logical_path: None,
        selected_exact_model: None,
        selected_provider_instance_name: None,
        candidate_count_before_filter: 0,
        candidate_count_after_filter: 0,
        filtered_candidates: Vec::new(),
        ranked_candidates: Vec::new(),
        fallback_applied: false,
        fallback_chain: Vec::new(),
        session_sticky_hit: false,
        scheduler_profile: Default::default(),
        runtime_failover_count: 0,
        user_summary: None,
        warnings: Vec::new(),
    }
}

#[derive(Clone, Debug)]
struct ScoredRouteCandidate {
    instance_id: String,
    provider_model: String,
    cost: f64,
    latency: f64,
    load: f64,
    error: f64,
    score: f64,
}

fn inventory_supports_capability(inventory: &ProviderInventory, capability: &Capability) -> bool {
    let Some(api_type) = api_type_for_capability(capability) else {
        return false;
    };
    inventory
        .models
        .iter()
        .any(|model| model.api_types.iter().any(|item| item == &api_type))
}

fn default_method_for_capability(capability: &Capability) -> &'static str {
    match capability {
        Capability::Llm => ai_methods::LLM_CHAT,
        Capability::Embedding => ai_methods::EMBEDDING_TEXT,
        Capability::Rerank => ai_methods::RERANK,
        Capability::Image => ai_methods::IMAGE_TXT2IMG,
        Capability::Vision => ai_methods::VISION_CAPTION,
        Capability::Audio => ai_methods::AUDIO_ASR,
        Capability::Video => ai_methods::VIDEO_TXT2VIDEO,
        Capability::Agent => ai_methods::AGENT_COMPUTER_USE,
    }
}

fn capability_for_method(method: &str) -> Option<Capability> {
    match method {
        ai_methods::LLM_CHAT | ai_methods::LLM_COMPLETION => Some(Capability::Llm),
        ai_methods::EMBEDDING_TEXT | ai_methods::EMBEDDING_MULTIMODAL => {
            Some(Capability::Embedding)
        }
        ai_methods::RERANK => Some(Capability::Rerank),
        ai_methods::IMAGE_TXT2IMG
        | ai_methods::IMAGE_IMG2IMG
        | ai_methods::IMAGE_INPAINT
        | ai_methods::IMAGE_UPSCALE
        | ai_methods::IMAGE_BG_REMOVE => Some(Capability::Image),
        ai_methods::VISION_OCR
        | ai_methods::VISION_CAPTION
        | ai_methods::VISION_DETECT
        | ai_methods::VISION_SEGMENT => Some(Capability::Vision),
        ai_methods::AUDIO_TTS
        | ai_methods::AUDIO_ASR
        | ai_methods::AUDIO_MUSIC
        | ai_methods::AUDIO_ENHANCE => Some(Capability::Audio),
        ai_methods::VIDEO_TXT2VIDEO
        | ai_methods::VIDEO_IMG2VIDEO
        | ai_methods::VIDEO_VIDEO2VIDEO
        | ai_methods::VIDEO_EXTEND
        | ai_methods::VIDEO_UPSCALE => Some(Capability::Video),
        ai_methods::AGENT_COMPUTER_USE => Some(Capability::Agent),
        _ => None,
    }
}

fn api_type_for_method(method: &str) -> Option<ApiType> {
    match method {
        ai_methods::LLM_CHAT => Some(ApiType::LlmChat),
        ai_methods::LLM_COMPLETION => Some(ApiType::LlmCompletion),
        ai_methods::EMBEDDING_TEXT => Some(ApiType::Embedding),
        ai_methods::EMBEDDING_MULTIMODAL => Some(ApiType::EmbeddingMultimodal),
        ai_methods::RERANK => Some(ApiType::Rerank),
        ai_methods::IMAGE_TXT2IMG => Some(ApiType::ImageTextToImage),
        ai_methods::IMAGE_IMG2IMG => Some(ApiType::ImageToImage),
        ai_methods::IMAGE_INPAINT => Some(ApiType::ImageInpaint),
        ai_methods::IMAGE_UPSCALE => Some(ApiType::ImageUpscale),
        ai_methods::IMAGE_BG_REMOVE => Some(ApiType::ImageBgRemove),
        ai_methods::VISION_OCR => Some(ApiType::VisionOcr),
        ai_methods::VISION_CAPTION => Some(ApiType::VisionCaption),
        ai_methods::VISION_DETECT => Some(ApiType::VisionDetect),
        ai_methods::VISION_SEGMENT => Some(ApiType::VisionSegment),
        ai_methods::AUDIO_TTS => Some(ApiType::AudioTts),
        ai_methods::AUDIO_ASR => Some(ApiType::AudioAsr),
        ai_methods::AUDIO_MUSIC => Some(ApiType::AudioMusic),
        ai_methods::AUDIO_ENHANCE => Some(ApiType::AudioEnhance),
        ai_methods::VIDEO_TXT2VIDEO => Some(ApiType::VideoTextToVideo),
        ai_methods::VIDEO_IMG2VIDEO => Some(ApiType::VideoImageToVideo),
        ai_methods::VIDEO_VIDEO2VIDEO => Some(ApiType::VideoToVideo),
        ai_methods::VIDEO_EXTEND => Some(ApiType::VideoExtend),
        ai_methods::VIDEO_UPSCALE => Some(ApiType::VideoUpscale),
        ai_methods::AGENT_COMPUTER_USE => Some(ApiType::AgentComputerUse),
        _ => None,
    }
}

fn api_type_for_capability(capability: &Capability) -> Option<ApiType> {
    api_type_for_method(default_method_for_capability(capability))
}

fn route_error_to_rpc(error: crate::model_types::RouteError) -> RPCErrors {
    let code = match error.code {
        crate::model_types::RouteErrorCode::NoCandidate => "no_provider_available",
        crate::model_types::RouteErrorCode::ModelNotFound => "model_alias_not_mapped",
        crate::model_types::RouteErrorCode::InvalidModelName => "bad_request",
        crate::model_types::RouteErrorCode::BudgetExceeded => "max_cost_exceeded",
        crate::model_types::RouteErrorCode::ContextTooLong => "context_too_long",
        crate::model_types::RouteErrorCode::FeatureUnsupported => "no_provider_available",
        crate::model_types::RouteErrorCode::ExactModelUnavailable
        | crate::model_types::RouteErrorCode::ProviderUnavailable
        | crate::model_types::RouteErrorCode::PolicyRejected => "no_provider_available",
        _ => error.code.as_str(),
    };
    reason_error(code, error.to_string())
}

fn route_policy_from_request(request: &AiMethodRequest) -> RoutePolicy {
    let mut policy = RoutePolicy {
        required_features: required_model_features(&request.requirements.must_features),
        max_estimated_cost_usd: request.requirements.max_cost_usd,
        ..Default::default()
    };
    if let Some(extra) = request.requirements.extra.as_ref() {
        if let Some(local_only) = extra.get("local_only").and_then(Value::as_bool) {
            policy.local_only = local_only;
        }
        if let Some(allow_fallback) = extra.get("allow_fallback").and_then(Value::as_bool) {
            policy.allow_fallback = allow_fallback;
        }
        if let Some(runtime_failover) = extra.get("runtime_failover").and_then(Value::as_bool) {
            policy.runtime_failover = runtime_failover;
        }
    }
    policy
}

fn apply_policy_config_to_route_policy(policy: &mut RoutePolicy, config: &PolicyConfig) {
    if let Some(value) = config.profile.as_ref() {
        policy.profile = value.value.clone();
    }
    if let Some(value) = config.scheduler_profiles.as_ref() {
        policy.scheduler_profiles = Some(value.value.clone());
    }
    if let Some(value) = config.local_only.as_ref() {
        policy.local_only = value.value;
    }
    if let Some(value) = config.allow_fallback.as_ref() {
        policy.allow_fallback = value.value;
    }
    if let Some(value) = config.allow_exact_model_fallback.as_ref() {
        policy.allow_exact_model_fallback = value.value;
    }
    if let Some(value) = config.runtime_failover.as_ref() {
        policy.runtime_failover = value.value;
    }
    if let Some(value) = config.explain.as_ref() {
        policy.explain = value.value;
    }
    if let Some(value) = config.blocked_provider_instances.as_ref() {
        policy.blocked_provider_instances = value.value.clone();
    }
    if let Some(value) = config.allowed_provider_instances.as_ref() {
        policy.allowed_provider_instances = value.value.clone();
    }
    if let Some(value) = config.max_estimated_cost_usd.as_ref() {
        policy.max_estimated_cost_usd = Some(value.value);
    }
}

fn required_model_features(features: &[Feature]) -> RequiredModelFeatures {
    let mut required = RequiredModelFeatures::default();
    for feature in features {
        match feature.as_str() {
            buckyos_api::features::TOOL_CALLING => required.tool_call = true,
            buckyos_api::features::JSON_OUTPUT => required.json_schema = true,
            buckyos_api::features::VISION => required.vision = true,
            "streaming" => required.streaming = true,
            _ => {}
        }
    }
    required
}

fn estimate_request_tokens(request: &AiMethodRequest) -> (u64, u64) {
    let mut text_len = request
        .payload
        .text
        .as_ref()
        .map(|text| text.len())
        .unwrap_or(0);
    for message in request.payload.messages.iter() {
        text_len = text_len.saturating_add(message.content.len());
    }
    if let Some(input_json) = request.payload.input_json.as_ref() {
        text_len = text_len.saturating_add(json_text_len(input_json));
    }
    let input_tokens = ((text_len as f64) / 4.0).ceil().max(1.0) as u64;
    let output_tokens = request
        .payload
        .input_json
        .as_ref()
        .and_then(|value| {
            value
                .get("max_output_tokens")
                .and_then(Value::as_u64)
                .or_else(|| value.get("max_tokens").and_then(Value::as_u64))
        })
        .or_else(|| {
            request
                .payload
                .options
                .as_ref()
                .and_then(|value| value.get("max_output_tokens").and_then(Value::as_u64))
        })
        .or_else(|| {
            request
                .payload
                .options
                .as_ref()
                .and_then(|value| value.get("max_tokens").and_then(Value::as_u64))
        })
        .unwrap_or(1024)
        .max(1);
    (input_tokens, output_tokens)
}

fn json_text_len(value: &Value) -> usize {
    match value {
        Value::String(text) => text.len(),
        Value::Array(items) => items.iter().map(json_text_len).sum(),
        Value::Object(map) => map.values().map(json_text_len).sum(),
        _ => 0,
    }
}

fn default_global_session_config() -> SessionConfig {
    SessionConfig {
        revision: Some("builtin-aicc-router-v1".to_string()),
        ..Default::default()
    }
}

pub fn provider_type_from_settings(value: &str) -> ProviderType {
    match value.trim().to_ascii_lowercase().as_str() {
        "local_inference" | "local" => ProviderType::LocalInference,
        "cloud_api" | "cloud" => ProviderType::CloudApi,
        "proxy_unknown" | "proxy" | "unknown" | "" => ProviderType::ProxyUnknown,
        _ => ProviderType::ProxyUnknown,
    }
}

pub fn exact_model_name(provider_model_id: &str, provider_instance_name: &str) -> String {
    format!("{}@{}", provider_model_id, provider_instance_name)
}

pub fn logical_mount_segment(value: &str) -> String {
    let normalized = value
        .trim()
        .replace('/', "-")
        .replace('_', "-")
        .replace('.', "-")
        .to_ascii_lowercase();
    normalized
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn provider_driver_mount_segment(provider_driver: &str) -> String {
    let normalized = provider_driver
        .trim()
        .replace('_', "-")
        .to_ascii_lowercase();
    let stripped = normalized
        .strip_prefix("google-")
        .unwrap_or(normalized.as_str());
    match stripped {
        "gimini" => "gemini".to_string(),
        _ => logical_mount_segment(stripped),
    }
}

fn add_unique_mount(mounts: &mut Vec<String>, mount: String) {
    if !mount.is_empty() && !mounts.iter().any(|item| item == &mount) {
        mounts.push(mount);
    }
}

pub fn llm_logical_mounts(provider_driver: &str, provider_model_id: &str) -> Vec<String> {
    let mut mounts = vec![format!(
        "llm.{}",
        provider_driver_mount_segment(provider_driver)
    )];
    let lowered = provider_model_id.to_ascii_lowercase();
    for family in ["gpt5", "gpt-5", "claude", "gemini", "gimini", "minimax"] {
        if lowered.contains(family) {
            let normalized = family.replace('-', "");
            let mount = if normalized == "gimini" {
                "llm.gemini".to_string()
            } else {
                format!("llm.{}", normalized)
            };
            if !mounts.iter().any(|item| item == &mount) {
                mounts.push(mount);
            }
        }
    }
    if lowered.contains("claude") {
        if lowered.contains("opus") {
            add_unique_mount(&mut mounts, "llm.opus".to_string());
        } else if lowered.contains("sonnet") {
            add_unique_mount(&mut mounts, "llm.sonnet".to_string());
        } else if lowered.contains("haiku") {
            add_unique_mount(&mut mounts, "llm.haiku".to_string());
        }
    }
    if lowered.contains("gemini") || lowered.contains("gimini") {
        if lowered.contains("deepthink") || lowered.contains("deep-think") {
            add_unique_mount(&mut mounts, "llm.gemini-deepthink".to_string());
        } else if lowered.contains("flash-lite") || lowered.contains("flash_lite") {
            add_unique_mount(&mut mounts, "llm.gemini-flash-lite".to_string());
        } else if lowered.contains("flash") {
            add_unique_mount(&mut mounts, "llm.gemini-flash".to_string());
        } else if lowered.contains("pro") {
            add_unique_mount(&mut mounts, "llm.gemini-pro".to_string());
        }
    }
    mounts
}

pub fn image_logical_mounts(provider_driver: &str, provider_model_id: &str) -> Vec<String> {
    let driver_mount = format!(
        "image.txt2img.{}",
        provider_driver_mount_segment(provider_driver)
    );
    let mut mounts = vec![driver_mount];
    let lowered = provider_model_id.to_ascii_lowercase();
    if lowered.contains("gpt") {
        mounts.push("image.txt2img.gpt_image".to_string());
    } else if lowered.contains("dall-e") {
        mounts.push("image.txt2img.dalle".to_string());
    } else if lowered.contains("gemini") || lowered.contains("gimini") {
        mounts.push("image.txt2img.gemini".to_string());
    }
    mounts
}

pub fn provider_model_metadata(
    provider_instance_name: &str,
    provider_type: ProviderType,
    provider_model_id: &str,
    api_type: ApiType,
    logical_mounts: Vec<String>,
    features: &[Feature],
    estimated_cost_usd: Option<f64>,
    estimated_latency_ms: Option<u64>,
) -> ModelMetadata {
    ModelMetadata {
        provider_model_id: provider_model_id.to_string(),
        exact_model: exact_model_name(provider_model_id, provider_instance_name),
        parameter_scale: None,
        api_types: vec![api_type],
        logical_mounts,
        capabilities: ModelCapabilities {
            streaming: features.iter().any(|item| item == "streaming"),
            tool_call: features
                .iter()
                .any(|item| item == buckyos_api::features::TOOL_CALLING),
            json_schema: features
                .iter()
                .any(|item| item == buckyos_api::features::JSON_OUTPUT),
            vision: features
                .iter()
                .any(|item| item == buckyos_api::features::VISION),
            max_context_tokens: None,
        },
        attributes: ModelAttributes {
            provider_type: provider_type.clone(),
            local: provider_type == ProviderType::LocalInference,
            privacy: if provider_type == ProviderType::LocalInference {
                PrivacyClass::Local
            } else {
                PrivacyClass::Cloud
            },
            quality_score: Some(0.7),
            latency_class: LatencyClass::Unknown,
            cost_class: CostClass::Unknown,
        },
        pricing: ModelPricing {
            estimated_cost_usd,
            ..Default::default()
        },
        health: ModelHealth {
            status: HealthStatus::Available,
            p95_latency_ms: estimated_latency_ms,
            quota_state: QuotaState::Normal,
            ..Default::default()
        },
    }
}

#[derive(Clone, Debug)]
struct TaskBinding {
    tenant_id: String,
    instance_id: String,
    task_mgr_id: i64,
}

pub struct AIComputeCenter {
    registry: Registry,
    router: Router,
    route_cfg: Arc<RwLock<RouteConfig>>,
    sn_ai_provider_billing: SnAIProviderBillingLedger,
    model_catalog: ModelCatalog,
    model_registry: Arc<RwLock<ModelRegistry>>,
    session_config: Arc<RwLock<SessionConfig>>,
    session_config_store: Arc<RwLock<SessionConfigStore>>,
    inventory_refresh_scheduler: Arc<InventoryRefreshScheduler>,
    model_scheduler: ModelScheduler,
    sticky_bindings: Arc<Mutex<StickyBindingStore>>,
    resource_resolver: Arc<dyn ResourceResolver>,
    sink_factory: Arc<dyn TaskEventSinkFactory>,
    taskmgr: Option<Arc<TaskManagerClient>>,
    task_bindings: Arc<RwLock<HashMap<String, TaskBinding>>>,
    task_id_seq: AtomicU64,
    base64_max_bytes: usize,
    base64_mime_allowlist: HashSet<String>,
    url_scheme_allowlist: HashSet<String>,
    usage_log_db: Option<Arc<AiccUsageLogDb>>,
}

impl Default for AIComputeCenter {
    fn default() -> Self {
        Self::new(Registry::default(), ModelCatalog::default())
    }
}

impl AIComputeCenter {
    pub fn new(registry: Registry, model_catalog: ModelCatalog) -> Self {
        let base64_mime_allowlist = [
            "image/png",
            "image/jpeg",
            "image/webp",
            "audio/wav",
            "audio/mpeg",
            "audio/ogg",
            "video/mp4",
            "application/json",
            "text/plain",
        ]
        .into_iter()
        .map(|item| item.to_string())
        .collect::<HashSet<_>>();
        let url_scheme_allowlist = ["http", "https"]
            .into_iter()
            .map(|item| item.to_string())
            .collect::<HashSet<_>>();
        let mut model_registry = ModelRegistry::new();
        for inventory in registry.inventories() {
            if let Err(err) = model_registry.apply_inventory(inventory) {
                warn!("aicc.model_registry.apply_inventory_failed err={}", err);
            }
        }

        let model_registry = Arc::new(RwLock::new(model_registry));
        let global_session_config = default_global_session_config();
        let session_config_store =
            SessionConfigStore::new(global_session_config.clone(), Duration::from_secs(60 * 60))
                .expect("default session config store should be valid");
        let inventory_registry = model_registry.clone();
        let inventory_source_registry = registry.clone();
        let inventory_refresh_scheduler = Arc::new(InventoryRefreshScheduler::new(
            inventory_registry,
            Arc::new(move || inventory_source_registry.inventories()),
            DEFAULT_INVENTORY_REFRESH_INTERVAL,
        ));
        if tokio::runtime::Handle::try_current().is_ok() {
            inventory_refresh_scheduler.start();
        }

        Self {
            registry,
            router: Router,
            route_cfg: Arc::new(RwLock::new(RouteConfig::default())),
            sn_ai_provider_billing: SnAIProviderBillingLedger::default(),
            model_catalog,
            model_registry,
            session_config: Arc::new(RwLock::new(global_session_config)),
            session_config_store: Arc::new(RwLock::new(session_config_store)),
            inventory_refresh_scheduler,
            model_scheduler: ModelScheduler,
            sticky_bindings: Arc::new(Mutex::new(StickyBindingStore::default())),
            resource_resolver: Arc::new(PassthroughResourceResolver),
            sink_factory: Arc::new(DefaultTaskEventSinkFactory),
            taskmgr: None,
            task_bindings: Arc::new(RwLock::new(HashMap::new())),
            task_id_seq: AtomicU64::new(1),
            base64_max_bytes: DEFAULT_BASE64_MAX_BYTES,
            base64_mime_allowlist,
            url_scheme_allowlist,
            usage_log_db: None,
        }
    }

    pub fn set_usage_log_db(&mut self, db: Arc<AiccUsageLogDb>) {
        self.usage_log_db = Some(db);
    }

    pub fn usage_log_db(&self) -> Option<Arc<AiccUsageLogDb>> {
        self.usage_log_db.clone()
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    pub fn model_catalog(&self) -> &ModelCatalog {
        &self.model_catalog
    }

    pub fn model_registry(&self) -> &Arc<RwLock<ModelRegistry>> {
        &self.model_registry
    }

    pub fn reset_model_routes(&self) {
        if let Ok(mut registry) = self.model_registry.write() {
            registry.clear();
        }
        if let Ok(mut sticky) = self.sticky_bindings.lock() {
            *sticky = StickyBindingStore::default();
        }
    }

    /// Build the default level-2 logical tree from the static templates and
    /// install it via `set_session_config`. Items whose target mount is not
    /// present in the current inventory are silently dropped, so adding a new
    /// provider later auto-extends the tree on the next reload.
    /// Returns the number of level-2 leaf nodes installed.
    pub fn apply_default_logical_tree(&self) -> std::result::Result<usize, RPCErrors> {
        let registry = self
            .model_registry
            .read()
            .map_err(|_| reason_error("internal_error", "model registry lock poisoned"))?;

        let mut available_targets: HashSet<String> = HashSet::new();
        for inventory in registry.inventories() {
            available_targets.insert(inventory.provider_instance_name.clone());
            for model in inventory.models.iter() {
                available_targets.insert(model.exact_model.clone());
                for mount in model.logical_mounts.iter() {
                    available_targets.insert(mount.clone());
                }
            }
        }
        drop(registry);

        let config = crate::default_logical_tree::build_default_session_config(&available_targets);
        let node_count = crate::default_logical_tree::level2_node_count(&config);
        self.set_session_config(config);
        info!(
            "aicc.default_logical_tree.applied level2_nodes={} available_targets={}",
            node_count,
            available_targets.len()
        );
        Ok(node_count)
    }

    pub fn dump_model_directory(&self) -> std::result::Result<Value, RPCErrors> {
        let registry = self
            .model_registry
            .read()
            .map_err(|_| reason_error("internal_error", "model registry lock poisoned"))?;

        let mut providers: Vec<&ProviderInventory> = registry.inventories().collect();
        providers.sort_by(|left, right| {
            left.provider_instance_name
                .cmp(&right.provider_instance_name)
        });

        let providers_json: Vec<Value> = providers
            .iter()
            .map(|inventory| {
                let mut models: Vec<&ModelMetadata> = inventory.models.iter().collect();
                models.sort_by(|left, right| left.exact_model.cmp(&right.exact_model));
                let models_json: Vec<Value> = models
                    .iter()
                    .map(|model| {
                        json!({
                            "exact_model": model.exact_model,
                            "provider_model_id": model.provider_model_id,
                            "api_types": model.api_types,
                            "logical_mounts": model.logical_mounts,
                            "health": model.health.status,
                            "quota": model.health.quota_state,
                        })
                    })
                    .collect();
                json!({
                    "provider_instance_name": inventory.provider_instance_name,
                    "provider_driver": inventory.provider_driver,
                    "provider_type": inventory.provider_type,
                    "version": inventory.version,
                    "inventory_revision": inventory.inventory_revision,
                    "models": models_json,
                })
            })
            .collect();

        let directory = registry.all_default_items();
        let mut directory_json = Map::new();
        for (logical_path, items) in directory.iter() {
            let mut items_json = Map::new();
            for (item_name, item) in items.iter() {
                items_json.insert(
                    item_name.clone(),
                    json!({
                        "target": item.target,
                        "weight": item.weight,
                    }),
                );
            }
            directory_json.insert(logical_path.clone(), Value::Object(items_json));
        }

        let aliases = self.model_catalog.snapshot();

        let session_config = self
            .session_config
            .read()
            .map(|guard| guard.clone())
            .map_err(|_| reason_error("internal_error", "session_config lock poisoned"))?;

        Ok(json!({
            "providers": providers_json,
            "directory": Value::Object(directory_json),
            "aliases": aliases,
            "session_config": session_config,
        }))
    }

    pub fn set_session_config(&self, config: SessionConfig) {
        if let Ok(mut current) = self.session_config.write() {
            *current = config.clone();
        }
        if let Ok(mut store) = self.session_config_store.write() {
            match SessionConfigStore::new(config, Duration::from_secs(60 * 60)) {
                Ok(next) => *store = next,
                Err(err) => warn!("aicc.session_config_store.rebuild_failed err={}", err),
            }
        }
    }

    pub fn inventory_changed(&self, _provider_instance_name: &str) {
        self.inventory_refresh_scheduler.inventory_changed();
        if let Err(err) = self.inventory_refresh_scheduler.refresh_once() {
            warn!(
                "aicc.model_registry.inventory_changed_refresh_failed err={}",
                err
            );
        }
    }

    pub fn update_route_config(&self, new_cfg: RouteConfig) {
        if let Ok(mut cfg) = self.route_cfg.write() {
            *cfg = new_cfg;
        }
    }

    pub fn set_resource_resolver(&mut self, resolver: Arc<dyn ResourceResolver>) {
        self.resource_resolver = resolver;
    }

    pub fn set_task_event_sink_factory(&mut self, factory: Arc<dyn TaskEventSinkFactory>) {
        self.sink_factory = factory;
    }

    pub fn set_task_manager_client(&mut self, taskmgr: Arc<TaskManagerClient>) {
        self.taskmgr = Some(taskmgr);
    }

    pub fn task_manager_client(&self) -> Option<Arc<TaskManagerClient>> {
        self.taskmgr.clone()
    }

    pub fn set_base64_policy(&mut self, max_bytes: usize, mime_allowlist: HashSet<String>) {
        self.base64_max_bytes = max_bytes;
        self.base64_mime_allowlist = mime_allowlist;
    }

    pub fn set_url_scheme_allowlist(&mut self, scheme_allowlist: HashSet<String>) {
        self.url_scheme_allowlist = scheme_allowlist;
    }

    fn route_request(
        &self,
        tenant_id: &str,
        method: &str,
        request: &AiMethodRequest,
        route_cfg: &RouteConfig,
        request_id: &str,
    ) -> std::result::Result<RouteDecision, RPCErrors> {
        let api_type = api_type_for_method(method).ok_or_else(|| {
            reason_error(
                "invalid_method",
                format!("method '{}' is not supported by model router", method),
            )
        })?;
        if capability_for_method(method).as_ref() != Some(&request.capability) {
            return Err(reason_error(
                "invalid_request",
                format!(
                    "method '{}' does not match capability '{:?}'",
                    method, request.capability
                ),
            ));
        }
        if let Err(err) = self.inventory_refresh_scheduler.refresh_once() {
            warn!(
                "aicc.model_registry.refresh_before_route_failed err={}",
                err
            );
        }
        let session_id = extract_session_id_from_complete_request(request);
        let (effective_session_config, session_config_updated, session_config_revision) = self
            .resolve_request_session_config(request, session_id.as_deref())
            .map_err(route_error_to_rpc)?;
        let mut policy = route_policy_from_request(request);
        apply_policy_config_to_route_policy(&mut policy, &effective_session_config.policy);
        if route_cfg.fallback_limit == 0 {
            policy.runtime_failover = false;
        }

        let registry = self
            .model_registry
            .read()
            .map_err(|_| reason_error("internal_error", "model registry lock poisoned"))?;
        let router = ModelRouter::new(&registry, &effective_session_config);
        let resolution = router.resolve(RouteRequest {
            request_id: request_id.to_string(),
            session_id: session_id.clone(),
            api_type: api_type.clone(),
            model: request.model.alias.clone(),
            policy: policy.clone(),
            session_config_revision,
            session_config_updated,
        });
        drop(registry);

        let mut resolution = resolution.map_err(route_error_to_rpc)?;
        self.apply_dynamic_cost_estimates(tenant_id, request, &mut resolution.candidates);
        self.apply_dynamic_budget_filters(&mut resolution, &policy)
            .map_err(route_error_to_rpc)?;

        let sticky_key = session_id.clone().map(|session_id| StickyBindingKey {
            session_id,
            logical_model: request.model.alias.clone(),
            api_type,
        });
        let mut sticky_store = self
            .sticky_bindings
            .lock()
            .map_err(|_| reason_error("internal_error", "sticky binding lock poisoned"))?;
        let scheduled = self
            .model_scheduler
            .schedule(
                &resolution.candidates,
                &policy,
                Some(&mut sticky_store),
                sticky_key.clone(),
            )
            .ok_or_else(|| reason_error("no_provider_available", "no route candidate generated"))?;
        drop(sticky_store);

        resolution.trace.selected_exact_model = Some(scheduled.selected.exact_model.clone());
        resolution.trace.selected_provider_instance_name =
            Some(scheduled.selected.provider_instance_name.clone());
        resolution.trace.session_sticky_hit = scheduled.sticky_hit;
        resolution.trace.ranked_candidates = scheduled.ranked_candidates;
        resolution.trace.user_summary = Some(user_summary_for_route(
            &resolution.trace,
            &scheduled.selected,
        ));

        debug!(
            "aicc.route.trace task_id={} trace={}",
            request_id,
            serde_json::to_string(&resolution.trace)
                .unwrap_or_else(|err| format!("{{\"serialize_error\":\"{}\"}}", err))
        );

        let selected_provider_model = self
            .legacy_catalog_provider_model(
                tenant_id,
                &request.capability,
                request.model.alias.as_str(),
                scheduled.selected.provider_instance_name.as_str(),
            )
            .unwrap_or_else(|| scheduled.selected.provider_model_id.clone());
        let mut attempts = vec![RouteAttempt {
            instance_id: scheduled.selected.provider_instance_name.clone(),
            provider_model: selected_provider_model.clone(),
            exact_model: scheduled.selected.exact_model.clone(),
        }];
        if policy.runtime_failover {
            let fallback_limit = route_cfg.fallback_limit;
            for candidate in resolution.candidates.iter() {
                if candidate.exact_model == scheduled.selected.exact_model {
                    continue;
                }
                if attempts.len() > fallback_limit {
                    break;
                }
                let provider_model = self
                    .legacy_catalog_provider_model(
                        tenant_id,
                        &request.capability,
                        request.model.alias.as_str(),
                        candidate.provider_instance_name.as_str(),
                    )
                    .unwrap_or_else(|| candidate.provider_model_id.clone());
                attempts.push(RouteAttempt {
                    instance_id: candidate.provider_instance_name.clone(),
                    provider_model,
                    exact_model: candidate.exact_model.clone(),
                });
            }
        }

        let fallback_instance_ids = attempts
            .iter()
            .skip(1)
            .map(|item| item.instance_id.clone())
            .collect::<Vec<_>>();

        Ok(RouteDecision {
            primary_instance_id: scheduled.selected.provider_instance_name,
            fallback_instance_ids,
            provider_model: selected_provider_model,
            attempts,
            route_trace: Arc::new(Mutex::new(resolution.trace)),
            runtime_failover_enabled: policy.runtime_failover,
            sticky_key,
        })
    }

    fn resolve_request_session_config(
        &self,
        request: &AiMethodRequest,
        session_id: Option<&str>,
    ) -> std::result::Result<(SessionConfig, bool, Option<String>), crate::model_types::RouteError>
    {
        let request_config = extract_session_config(request, "session_config")?;
        let request_patch = extract_session_config(request, "session_config_patch")?;
        let expected_revision = extract_expected_session_config_revision(request);

        if let Some(session_id) = session_id {
            let store = self.session_config_store.read().map_err(|_| {
                crate::model_types::RouteError::new(
                    crate::model_types::RouteErrorCode::ProviderUnavailable,
                    "session config store lock poisoned",
                )
            })?;
            let stored = if let Some(config) = request_config {
                store.replace(session_id, config, expected_revision.as_deref())?
            } else if let Some(patch) = request_patch {
                store.patch(session_id, patch, expected_revision.as_deref())?
            } else {
                store.get_or_create(session_id)?
            };
            return Ok((
                stored.config,
                expected_revision.is_some() || extract_has_session_config_update(request),
                Some(stored.revision),
            ));
        }

        let global = self.session_config.read().map_err(|_| {
            crate::model_types::RouteError::new(
                crate::model_types::RouteErrorCode::ProviderUnavailable,
                "session config lock poisoned",
            )
        })?;
        let mut config = global.clone();
        drop(global);
        let mut updated = false;
        if let Some(request_config) = request_config {
            config = request_config;
            updated = true;
        }
        if let Some(patch) = request_patch {
            config = merge_session_config(&config, &patch)?;
            updated = true;
        }
        config.validate()?;
        Ok((config.clone(), updated, config.revision.clone()))
    }

    fn legacy_catalog_provider_model(
        &self,
        tenant_id: &str,
        capability: &Capability,
        alias: &str,
        provider_instance_name: &str,
    ) -> Option<String> {
        let inventory = self.registry.inventory(provider_instance_name)?;
        if inventory.provider_driver.is_empty() {
            return None;
        }
        self.model_catalog.resolve(
            tenant_id,
            capability,
            alias,
            inventory.provider_driver.as_str(),
        )
    }

    fn apply_dynamic_cost_estimates(
        &self,
        tenant_id: &str,
        request: &AiMethodRequest,
        candidates: &mut [ModelCandidate],
    ) {
        let (input_tokens, output_tokens) = estimate_request_tokens(request);
        for candidate in candidates.iter_mut() {
            let Some(provider) = self
                .registry
                .get_provider(candidate.provider_instance_name.as_str())
            else {
                continue;
            };
            let estimate = provider.estimate_cost(&CostEstimateInput {
                api_type: candidate.api_type.clone(),
                exact_model: candidate.exact_model.clone(),
                input_tokens,
                estimated_output_tokens: Some(output_tokens),
                cached_input_tokens: None,
                request_features: request.requirements.must_features.clone(),
            });
            let provider_driver = self
                .registry
                .inventory(candidate.provider_instance_name.as_str())
                .map(|inventory| inventory.provider_driver)
                .unwrap_or_default();
            let effective_cost = self
                .sn_ai_provider_billing
                .preview_billed_cost(
                    tenant_id,
                    provider_driver.as_str(),
                    estimate.estimated_cost_usd,
                )
                .unwrap_or(estimate.estimated_cost_usd);
            candidate.metadata.pricing.estimated_cost_usd = Some(effective_cost.max(0.0));
            candidate.dynamic_cost_estimate = Some(CostEstimateOutput {
                estimated_cost_usd: effective_cost.max(0.0),
                pricing_mode: estimate.pricing_mode,
                quota_state: estimate.quota_state.clone(),
                confidence: estimate.confidence,
                estimated_latency_ms: estimate.estimated_latency_ms,
            });
            if let Some(latency) = estimate.estimated_latency_ms {
                candidate.metadata.health.p95_latency_ms = Some(latency);
            }
            candidate.metadata.health.quota_state = estimate.quota_state;
        }
    }

    fn apply_dynamic_budget_filters(
        &self,
        resolution: &mut crate::model_router::RouteResolution,
        policy: &RoutePolicy,
    ) -> std::result::Result<(), crate::model_types::RouteError> {
        let before = resolution.candidates.len();
        resolution.candidates.retain(|candidate| {
            if candidate.metadata.health.quota_state == QuotaState::Exhausted {
                return false;
            }
            if let Some(max_cost) = policy.max_estimated_cost_usd {
                let cost = candidate
                    .dynamic_cost_estimate
                    .as_ref()
                    .map(|estimate| estimate.estimated_cost_usd)
                    .or(candidate.metadata.pricing.estimated_cost_usd);
                if cost.map(|value| value > max_cost).unwrap_or(false) {
                    return false;
                }
            }
            true
        });
        if resolution.candidates.is_empty() && before > 0 {
            return Err(crate::model_types::RouteError::new(
                crate::model_types::RouteErrorCode::BudgetExceeded,
                "all candidates were rejected by dynamic cost or quota estimates",
            ));
        }
        resolution.trace.candidate_count_after_filter = resolution.candidates.len();
        Ok(())
    }

    fn apply_billing_to_summary(
        &self,
        ctx: &InvokeCtx,
        provider_driver: &str,
        summary: &mut AiResponseSummary,
    ) {
        let Some(cost) = summary.cost.clone() else {
            return;
        };
        let Some(adjustment) = self.sn_ai_provider_billing.apply_charge(
            ctx.tenant_id.as_str(),
            provider_driver,
            Some(cost.amount),
        ) else {
            return;
        };

        summary.cost = Some(buckyos_api::AiCost {
            amount: adjustment.billed_cost_usd,
            currency: cost.currency,
        });

        let extra_value = summary
            .extra
            .get_or_insert_with(|| Value::Object(Map::new()));
        if !extra_value.is_object() {
            *extra_value = Value::Object(Map::new());
        }
        if let Value::Object(extra) = extra_value {
            extra.insert(
                "billing".to_string(),
                json!({
                    "raw_cost_usd": adjustment.raw_cost_usd,
                    "billed_cost_usd": adjustment.billed_cost_usd,
                    "sn_ai_provider_credit_applied_usd": adjustment.credit_applied_usd,
                    "sn_ai_provider_credit_remaining_usd": adjustment.remaining_credit_usd,
                }),
            );
        }
    }

    pub async fn complete(
        &self,
        request: AiMethodRequest,
        rpc_ctx: RPCContext,
    ) -> std::result::Result<AiMethodResponse, RPCErrors> {
        self.complete_with_method(
            default_method_for_capability(&request.capability),
            request,
            rpc_ctx,
        )
        .await
    }

    pub async fn complete_with_method(
        &self,
        method: &str,
        request: AiMethodRequest,
        rpc_ctx: RPCContext,
    ) -> std::result::Result<AiMethodResponse, RPCErrors> {
        let invoke_ctx = InvokeCtx::from_rpc(&rpc_ctx);
        info!(
            "aicc.complete received: tenant={} caller_app={:?} method={} capability={:?} model_alias={} idempotency_key={:?}",
            invoke_ctx.tenant_id,
            invoke_ctx.caller_app_id,
            method,
            request.capability,
            request.model.alias,
            request.idempotency_key
        );
        let external_task_id = self.generate_task_id();
        let base_sink = self.sink_factory.build(&invoke_ctx, &external_task_id);
        let event_ref = base_sink.event_ref();
        let deferred_sink = Arc::new(DeferredTaskEventSink::new(base_sink));
        let event_sink: Arc<dyn TaskEventSink> = deferred_sink.clone();

        if let Err(error) = self.validate_request(&request) {
            let code = extract_error_code(&error);
            warn!(
                "aicc.complete validation_failed: task_id={} tenant={} code={} err={}",
                external_task_id, invoke_ctx.tenant_id, code, error
            );
            self.emit_task_error(
                event_sink.clone(),
                external_task_id.as_str(),
                code.as_str(),
                error.to_string(),
            )
            .await;
            return Ok(AiMethodResponse::new(
                external_task_id,
                AiMethodStatus::Failed,
                None,
                event_ref,
            ));
        }

        let mut resolved = match self.resource_resolver.resolve(&invoke_ctx, &request).await {
            Ok(result) => result,
            Err(error) => {
                self.emit_task_error(
                    event_sink.clone(),
                    external_task_id.as_str(),
                    "resource_invalid",
                    error.to_string(),
                )
                .await;
                return Ok(AiMethodResponse::new(
                    external_task_id,
                    AiMethodStatus::Failed,
                    None,
                    event_ref,
                ));
            }
        };
        resolved.method = method.to_string();

        let route_cfg = self
            .route_cfg
            .read()
            .map(|cfg| cfg.clone())
            .unwrap_or_default();
        info!(
            "aicc.routing input: task_id={} tenant={} caller_app={:?} capability={:?} model_alias={} providers={} required_features={:?} max_cost_usd={:?} max_latency_ms={:?}",
            external_task_id,
            invoke_ctx.tenant_id,
            invoke_ctx.caller_app_id,
            request.capability,
            request.model.alias,
            self.registry.provider_count(),
            request.requirements.must_features,
            request.requirements.max_cost_usd,
            request.requirements.max_latency_ms
        );

        let decision = match self.route_request(
            invoke_ctx.tenant_id.as_str(),
            method,
            &request,
            &route_cfg,
            external_task_id.as_str(),
        ) {
            Ok(result) => result,
            Err(error) => {
                warn!(
                    "aicc.routing failed: task_id={} tenant={} capability={:?} model_alias={} providers={} err={}",
                    external_task_id,
                    invoke_ctx.tenant_id,
                    request.capability,
                    request.model.alias,
                    self.registry.provider_count(),
                    error
                );
                let code = extract_error_code(&error);
                self.emit_task_error(
                    event_sink.clone(),
                    external_task_id.as_str(),
                    code.as_str(),
                    error.to_string(),
                )
                .await;
                return Ok(AiMethodResponse::new(
                    external_task_id,
                    AiMethodStatus::Failed,
                    None,
                    event_ref,
                ));
            }
        };
        let route_attempts = decision
            .attempts()
            .iter()
            .map(|item| format!("{}:{}", item.instance_id, item.provider_model))
            .collect::<Vec<_>>()
            .join(",");
        info!(
            "aicc.routing output: task_id={} tenant={} caller_app={:?} primary_instance={} provider_model={} fallback_instances={:?} attempts={}",
            external_task_id,
            invoke_ctx.tenant_id,
            invoke_ctx.caller_app_id,
            decision.primary_instance_id,
            decision.provider_model,
            decision.fallback_instance_ids,
            route_attempts
        );

        // Once we know the final provider model we can wrap the sink with a
        // usage-log layer: any Final event flowing through it (immediate call
        // or long-task completion) writes one durable row.
        let event_sink: Arc<dyn TaskEventSink> = if let Some(db) = self.usage_log_db.clone() {
            let context = UsageLogContext {
                external_task_id: external_task_id.clone(),
                tenant_id: invoke_ctx.tenant_id.clone(),
                caller_app_id: invoke_ctx.caller_app_id.clone(),
                capability: capability_name(&request.capability).to_string(),
                request_model: request.model.alias.clone(),
                provider_model: decision.provider_model.clone(),
                idempotency_key: request.idempotency_key.clone(),
            };
            Arc::new(UsageLoggingSink::new(event_sink, db, context))
        } else {
            event_sink
        };

        let start_result = self
            .start_with_fallback(
                &invoke_ctx,
                external_task_id.as_str(),
                resolved,
                &decision,
                event_sink.clone(),
            )
            .await;
        match start_result {
            Ok((ProviderStartResult::Immediate(summary), instance_id)) => {
                let prepared_task = self
                    .create_provider_task(
                        external_task_id.as_str(),
                        &request,
                        &invoke_ctx,
                        event_ref.as_deref(),
                        &decision,
                        InitialTaskState::Running,
                    )
                    .await?;
                let task_audit_sink: Arc<dyn TaskEventSink> = Arc::new(TaskAuditSink::new(
                    prepared_task.taskmgr.clone(),
                    prepared_task.id(),
                ));
                deferred_sink.promote(task_audit_sink).await?;
                self.emit_task_started(
                    event_sink.clone(),
                    external_task_id.as_str(),
                    instance_id.as_str(),
                )
                .await;
                self.emit_task_final(event_sink, external_task_id.as_str(), &summary)
                    .await;
                Ok(AiMethodResponse::new(
                    external_task_id,
                    AiMethodStatus::Succeeded,
                    Some(summary),
                    event_ref,
                ))
            }
            Ok((ProviderStartResult::Started, instance_id)) => {
                let prepared_task = self
                    .create_provider_task(
                        external_task_id.as_str(),
                        &request,
                        &invoke_ctx,
                        event_ref.as_deref(),
                        &decision,
                        InitialTaskState::Running,
                    )
                    .await?;
                let task_mgr_id = prepared_task.id();
                let task_audit_sink: Arc<dyn TaskEventSink> = Arc::new(TaskAuditSink::new(
                    prepared_task.taskmgr.clone(),
                    task_mgr_id,
                ));
                deferred_sink.promote(task_audit_sink).await?;
                self.bind_task(
                    external_task_id.as_str(),
                    invoke_ctx.tenant_id.as_str(),
                    instance_id.as_str(),
                    task_mgr_id,
                );
                self.emit_task_started(event_sink, external_task_id.as_str(), instance_id.as_str())
                    .await;
                Ok(AiMethodResponse::new(
                    external_task_id,
                    AiMethodStatus::Running,
                    None,
                    event_ref,
                ))
            }
            Ok((ProviderStartResult::Queued { position }, instance_id)) => {
                let prepared_task = self
                    .create_provider_task(
                        external_task_id.as_str(),
                        &request,
                        &invoke_ctx,
                        event_ref.as_deref(),
                        &decision,
                        InitialTaskState::Queued,
                    )
                    .await?;
                let task_mgr_id = prepared_task.id();
                let task_audit_sink: Arc<dyn TaskEventSink> = Arc::new(TaskAuditSink::new(
                    prepared_task.taskmgr.clone(),
                    task_mgr_id,
                ));
                deferred_sink.promote(task_audit_sink).await?;
                self.bind_task(
                    external_task_id.as_str(),
                    invoke_ctx.tenant_id.as_str(),
                    instance_id.as_str(),
                    task_mgr_id,
                );
                self.emit_task_queued(event_sink, external_task_id.as_str(), position)
                    .await;
                Ok(AiMethodResponse::new(
                    external_task_id,
                    AiMethodStatus::Running,
                    None,
                    event_ref,
                ))
            }
            Err(error) => {
                let code = extract_error_code(&error);
                self.emit_task_error(
                    event_sink,
                    external_task_id.as_str(),
                    code.as_str(),
                    error.to_string(),
                )
                .await;
                Ok(AiMethodResponse::new(
                    external_task_id,
                    AiMethodStatus::Failed,
                    None,
                    event_ref,
                ))
            }
        }
    }

    pub async fn cancel(
        &self,
        task_id: &str,
        rpc_ctx: RPCContext,
    ) -> std::result::Result<CancelResponse, RPCErrors> {
        let invoke_ctx = InvokeCtx::from_rpc(&rpc_ctx);
        info!(
            "aicc.cancel received: tenant={} caller_app={:?} task_id={}",
            invoke_ctx.tenant_id, invoke_ctx.caller_app_id, task_id
        );

        let binding = self
            .task_bindings
            .read()
            .ok()
            .and_then(|bindings| bindings.get(task_id).cloned());
        let Some(binding) = binding else {
            return Ok(CancelResponse::new(task_id.to_string(), false));
        };

        if !binding.tenant_id.is_empty() && binding.tenant_id != invoke_ctx.tenant_id {
            return Err(RPCErrors::NoPermission(
                "cross-tenant cancel is not allowed".to_string(),
            ));
        }

        let provider = self.registry.get_provider(binding.instance_id.as_str());
        let Some(provider) = provider else {
            return Ok(CancelResponse::new(task_id.to_string(), false));
        };

        let accepted = provider.cancel(invoke_ctx, task_id).await.is_ok();
        if accepted {
            if let Some(taskmgr) = self.taskmgr.as_ref() {
                let event = TaskEvent {
                    task_id: task_id.to_string(),
                    kind: TaskEventKind::CancelRequested,
                    timestamp_ms: now_ms(),
                    data: Some(json!({
                        "accepted": true,
                        "source": "cancel_api"
                    })),
                };
                let mut task_data = taskmgr
                    .get_task(binding.task_mgr_id)
                    .await
                    .map(|task| task.data)
                    .unwrap_or_else(|_| json!({}));
                merge_task_data_with_event(&mut task_data, &event);
                let _ = taskmgr
                    .update_task(
                        binding.task_mgr_id,
                        Some(TaskStatus::Canceled),
                        Some(1.0),
                        Some("aicc task canceled".to_string()),
                        Some(task_data),
                    )
                    .await;
            }
            if let Ok(mut bindings) = self.task_bindings.write() {
                bindings.remove(task_id);
            }
        }
        Ok(CancelResponse::new(task_id.to_string(), accepted))
    }

    async fn start_with_fallback(
        &self,
        ctx: &InvokeCtx,
        task_id: &str,
        req: ResolvedRequest,
        decision: &RouteDecision,
        sink: Arc<dyn TaskEventSink>,
    ) -> std::result::Result<(ProviderStartResult, String), RPCErrors> {
        let mut last_err: Option<ProviderError> = None;
        let _request_log = serde_json::to_string(&req.request)
            .unwrap_or_else(|err| format!("{{\"serialize_error\":\"{}\"}}", err));
        info!(
            "aicc.llm.input task_id={} tenant={} trace_id={:?}",
            task_id, ctx.tenant_id, ctx.trace_id
        );

        for (attempt_index, attempt) in decision.attempts().iter().enumerate() {
            let provider = self.registry.get_provider(attempt.instance_id.as_str());
            let Some(provider) = provider else {
                continue;
            };
            if attempt_index > 0 {
                if let Ok(mut trace) = decision.route_trace.lock() {
                    trace.runtime_failover_count = trace.runtime_failover_count.saturating_add(1);
                    trace.selected_exact_model = Some(attempt.exact_model.clone());
                    trace.selected_provider_instance_name = Some(attempt.instance_id.clone());
                    if let Some(summary) = trace.user_summary.as_mut() {
                        summary.display_name = attempt
                            .exact_model
                            .rsplit_once('@')
                            .map(|(model, provider)| format!("{} ({})", model, provider))
                            .unwrap_or_else(|| attempt.exact_model.clone());
                        summary.was_failover = true;
                        summary.reason_short =
                            "runtime failover selected next provider".to_string();
                    }
                }
            }
            info!(
                "aicc.provider.start task_id={} tenant={} trace_id={:?} instance_id={} provider_model={}",
                task_id, ctx.tenant_id, ctx.trace_id, attempt.instance_id, attempt.provider_model
            );

            self.registry.mark_start_begin(attempt.instance_id.as_str());
            let started_at = Instant::now();
            let result = provider
                .start(
                    ctx.clone(),
                    attempt.provider_model.clone(),
                    req.clone(),
                    sink.clone(),
                )
                .await;
            let elapsed_ms = started_at.elapsed().as_millis() as f64;

            match result {
                Ok(mut start_result) => {
                    if let ProviderStartResult::Immediate(summary) = &mut start_result {
                        self.apply_billing_to_summary(
                            ctx,
                            self.registry
                                .inventory(attempt.instance_id.as_str())
                                .map(|inventory| inventory.provider_driver)
                                .unwrap_or_default()
                                .as_str(),
                            summary,
                        );
                    }
                    self.registry
                        .record_start_success(attempt.instance_id.as_str(), elapsed_ms);
                    if attempt_index > 0 {
                        if let Some(sticky_key) = decision.sticky_key.clone() {
                            if let Ok(mut sticky) = self.sticky_bindings.lock() {
                                sticky.set_binding(
                                    sticky_key,
                                    attempt.exact_model.clone(),
                                    attempt.instance_id.clone(),
                                );
                            }
                        }
                    }
                    match &start_result {
                        ProviderStartResult::Immediate(summary) => {
                            let summary_log =
                                serde_json::to_string(&redacted_summary_value(summary))
                                    .unwrap_or_else(|err| {
                                        format!("{{\"serialize_error\":\"{}\"}}", err)
                                    });
                            debug!(
                                "aicc.llm.output task_id={} tenant={} trace_id={:?} instance_id={} provider_model={} elapsed_ms={} summary={}",
                                task_id,
                                ctx.tenant_id,
                                ctx.trace_id,
                                attempt.instance_id,
                                attempt.provider_model,
                                elapsed_ms,
                                summary_log
                            );
                        }
                        ProviderStartResult::Started => {
                            info!(
                                "aicc.llm.output task_id={} tenant={} trace_id={:?} instance_id={} provider_model={} elapsed_ms={} status=running",
                                task_id,
                                ctx.tenant_id,
                                ctx.trace_id,
                                attempt.instance_id,
                                attempt.provider_model,
                                elapsed_ms
                            );
                        }
                        ProviderStartResult::Queued { position } => {
                            info!(
                                "aicc.llm.output task_id={} tenant={} trace_id={:?} instance_id={} provider_model={} elapsed_ms={} status=queued queue_position={}",
                                task_id,
                                ctx.tenant_id,
                                ctx.trace_id,
                                attempt.instance_id,
                                attempt.provider_model,
                                elapsed_ms,
                                position
                            );
                        }
                    }
                    if let Ok(trace) = decision.route_trace.lock() {
                        debug!(
                            "aicc.route.trace.final task_id={} trace={}",
                            task_id,
                            serde_json::to_string(&*trace).unwrap_or_else(|err| format!(
                                "{{\"serialize_error\":\"{}\"}}",
                                err
                            ))
                        );
                    }
                    return Ok((start_result, attempt.instance_id.clone()));
                }
                Err(error) => {
                    self.registry
                        .record_start_failure(attempt.instance_id.as_str(), elapsed_ms);
                    warn!(
                        "aicc.provider.start_failed task_id={} tenant={} trace_id={:?} instance_id={} provider_model={} elapsed_ms={} retryable={} err={}",
                        task_id,
                        ctx.tenant_id,
                        ctx.trace_id,
                        attempt.instance_id,
                        attempt.provider_model,
                        elapsed_ms,
                        error.is_retryable(),
                        error
                    );
                    last_err = Some(error.clone());
                    if !error.is_retryable() || !decision.runtime_failover_enabled {
                        break;
                    }
                }
            }
        }

        let reason = last_err
            .map(|error| format!("provider start failed for task {}: {}", task_id, error))
            .unwrap_or_else(|| format!("provider start failed for task {}: no candidate", task_id));
        error!(
            "aicc.provider.start_failed.final task_id={} tenant={} trace_id={:?} reason={}",
            task_id, ctx.tenant_id, ctx.trace_id, reason
        );
        eprintln!(
            "aicc.provider.start_failed.final task_id={} tenant={} trace_id={:?} reason={}",
            task_id, ctx.tenant_id, ctx.trace_id, reason
        );
        Err(reason_error("provider_start_failed", reason))
    }

    fn validate_request(&self, req: &AiMethodRequest) -> std::result::Result<(), RPCErrors> {
        if req.model.alias.trim().is_empty() {
            return Err(reason_error("bad_request", "model.alias must not be empty"));
        }

        let has_payload = req.payload.text.is_some()
            || !req.payload.messages.is_empty()
            || !req.payload.resources.is_empty()
            || req.payload.input_json.is_some();
        if !has_payload {
            return Err(reason_error(
                "bad_request",
                "payload must include text/messages/resources/input_json",
            ));
        }

        for resource in req.payload.resources.iter() {
            self.validate_resource(resource)?;
        }
        Ok(())
    }

    fn validate_resource(&self, resource: &ResourceRef) -> std::result::Result<(), RPCErrors> {
        match resource {
            ResourceRef::Url { url, .. } => {
                if url.trim().is_empty() {
                    return Err(reason_error(
                        "resource_invalid",
                        "resource url must not be empty",
                    ));
                }
                if !url.contains("://") {
                    return Err(reason_error(
                        "resource_invalid",
                        "resource url must include scheme",
                    ));
                }
                let parsed = reqwest::Url::parse(url).map_err(|_| {
                    reason_error("resource_invalid", "resource url format is invalid")
                })?;
                if !self.url_scheme_allowlist.contains(parsed.scheme()) {
                    return Err(reason_error(
                        "resource_invalid",
                        "resource url scheme is not allowed",
                    ));
                }
                if parsed.host_str().is_none() {
                    return Err(reason_error(
                        "resource_invalid",
                        "resource url host is missing",
                    ));
                }
                Ok(())
            }
            ResourceRef::Base64 { mime, data_base64 } => {
                if !self.base64_mime_allowlist.contains(mime.as_str()) {
                    return Err(reason_error(
                        "resource_invalid",
                        format!("base64 mime '{}' is not allowed", mime),
                    ));
                }
                let decoded = general_purpose::STANDARD.decode(data_base64).map_err(|_| {
                    reason_error("resource_invalid", "resource base64 is not valid")
                })?;
                if decoded.len() > self.base64_max_bytes {
                    return Err(reason_error(
                        "resource_invalid",
                        format!(
                            "base64 payload exceeds limit: {} > {} bytes",
                            decoded.len(),
                            self.base64_max_bytes
                        ),
                    ));
                }
                Ok(())
            }
            ResourceRef::NamedObject { .. } => Ok(()),
        }
    }

    fn bind_task(&self, task_id: &str, tenant_id: &str, instance_id: &str, task_mgr_id: i64) {
        if let Ok(mut bindings) = self.task_bindings.write() {
            bindings.insert(
                task_id.to_string(),
                TaskBinding {
                    tenant_id: tenant_id.to_string(),
                    instance_id: instance_id.to_string(),
                    task_mgr_id,
                },
            );
        }
    }

    fn generate_task_id(&self) -> String {
        let seq = self.task_id_seq.fetch_add(1, AtomicOrdering::Relaxed);
        let ts_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        format!("aicc-{}-{}", ts_ms, seq)
    }

    async fn emit_task_started(
        &self,
        sink: Arc<dyn TaskEventSink>,
        task_id: &str,
        instance_id: &str,
    ) {
        let event = TaskEvent {
            task_id: task_id.to_string(),
            kind: TaskEventKind::Started,
            timestamp_ms: now_ms(),
            data: Some(json!({
                "instance_id": instance_id,
                "message": "request sent, waiting for provider response"
            })),
        };
        let _ = sink.emit(event).await;
    }

    async fn emit_task_queued(&self, sink: Arc<dyn TaskEventSink>, task_id: &str, position: usize) {
        let event = TaskEvent {
            task_id: task_id.to_string(),
            kind: TaskEventKind::Queued,
            timestamp_ms: now_ms(),
            data: Some(json!({
                "position": position,
                "message": QUEUE_STATUS_QUEUED
            })),
        };
        let _ = sink.emit(event).await;
    }

    async fn emit_task_final(
        &self,
        sink: Arc<dyn TaskEventSink>,
        task_id: &str,
        summary: &AiResponseSummary,
    ) {
        let summary_value = redacted_summary_value(summary);
        let event = TaskEvent {
            task_id: task_id.to_string(),
            kind: TaskEventKind::Final,
            timestamp_ms: now_ms(),
            data: Some(json!({
                "summary": summary_value,
                "finish_reason": summary.finish_reason.clone(),
                "has_text": summary.text.as_ref().map(|text| !text.is_empty()).unwrap_or(false),
                "artifact_count": summary.artifacts.len(),
            })),
        };
        let _ = sink.emit(event).await;
    }

    async fn emit_task_error(
        &self,
        sink: Arc<dyn TaskEventSink>,
        task_id: &str,
        code: &str,
        message: String,
    ) {
        let event = TaskEvent {
            task_id: task_id.to_string(),
            kind: TaskEventKind::Error,
            timestamp_ms: now_ms(),
            data: Some(json!({
                "code": code,
                "message": message,
            })),
        };
        let _ = sink.emit(event).await;
    }
}

#[async_trait]
impl AiccHandler for AIComputeCenter {
    async fn handle_method(
        &self,
        method: &str,
        request: AiMethodRequest,
        ctx: RPCContext,
    ) -> std::result::Result<AiMethodResponse, RPCErrors> {
        self.complete_with_method(method, request, ctx).await
    }

    async fn handle_cancel(
        &self,
        task_id: &str,
        ctx: RPCContext,
    ) -> std::result::Result<CancelResponse, RPCErrors> {
        self.cancel(task_id, ctx).await
    }
}

fn json_non_empty_string(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| item.to_string())
}

fn request_control_value<'a>(request: &'a AiMethodRequest, key: &str) -> Option<&'a Value> {
    request
        .requirements
        .extra
        .as_ref()
        .and_then(|value| value.get(key))
        .or_else(|| {
            request
                .payload
                .options
                .as_ref()
                .and_then(|value| value.get(key))
        })
        .or_else(|| {
            request
                .payload
                .input_json
                .as_ref()
                .and_then(|value| value.get(key))
        })
}

fn extract_session_config(
    request: &AiMethodRequest,
    key: &str,
) -> std::result::Result<Option<SessionConfig>, crate::model_types::RouteError> {
    let Some(value) = request_control_value(request, key) else {
        return Ok(None);
    };
    let config = serde_json::from_value::<SessionConfig>(value.clone()).map_err(|err| {
        crate::model_types::RouteError::new(
            crate::model_types::RouteErrorCode::SessionConfigInvalid,
            format!("{} is invalid: {}", key, err),
        )
    })?;
    config.validate()?;
    Ok(Some(config))
}

fn extract_expected_session_config_revision(request: &AiMethodRequest) -> Option<String> {
    request_control_value(request, "expected_session_config_revision")
        .or_else(|| request_control_value(request, "expected_revision"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
}

fn extract_has_session_config_update(request: &AiMethodRequest) -> bool {
    request_control_value(request, "session_config").is_some()
        || request_control_value(request, "session_config_patch").is_some()
}

fn extract_session_id_from_complete_request(request: &AiMethodRequest) -> Option<String> {
    let from_options = request.payload.options.as_ref().and_then(|options| {
        json_non_empty_string(options.get("session_id"))
            .or_else(|| json_non_empty_string(options.get("owner_session_id")))
    });
    if from_options.is_some() {
        return from_options;
    }

    let input = request.payload.input_json.as_ref();
    json_non_empty_string(input.and_then(|value| value.get("session_id")))
        .or_else(|| json_non_empty_string(input.and_then(|value| value.get("owner_session_id"))))
        .or_else(|| {
            json_non_empty_string(input.and_then(|value| value.pointer("/session/session_id")))
        })
}

fn user_summary_for_route(
    trace: &RouteTrace,
    candidate: &ModelCandidate,
) -> UserFacingRouteSummary {
    let display_name = candidate
        .exact_model
        .rsplit_once('@')
        .map(|(model, provider)| format!("{} ({})", model, provider))
        .unwrap_or_else(|| candidate.exact_model.clone());
    let model_family = trace
        .resolved_logical_path
        .as_ref()
        .or(Some(&trace.requested_model))
        .and_then(|model| model.split('.').last())
        .filter(|item| !item.is_empty())
        .unwrap_or(candidate.provider_model_id.as_str())
        .to_string();
    let provider_origin = match candidate.metadata.attributes.provider_type {
        ProviderType::LocalInference => UserFacingProviderOrigin::Local,
        ProviderType::CloudApi => UserFacingProviderOrigin::Cloud,
        ProviderType::ProxyUnknown => UserFacingProviderOrigin::ProxyUnknown,
    };
    let reason_short = if trace.session_sticky_hit {
        "hit session binding"
    } else if trace.runtime_failover_count > 0 {
        "runtime failover selected next provider"
    } else if trace.fallback_applied {
        "fallback policy selected available model"
    } else {
        match trace.scheduler_profile {
            crate::model_types::SchedulerProfile::CostFirst => "selected by lowest cost policy",
            crate::model_types::SchedulerProfile::LatencyFirst => {
                "selected by lowest latency policy"
            }
            crate::model_types::SchedulerProfile::QualityFirst => {
                "selected by highest quality policy"
            }
            crate::model_types::SchedulerProfile::Balanced => "selected by balanced policy",
            crate::model_types::SchedulerProfile::LocalFirst => "selected by local-first policy",
            crate::model_types::SchedulerProfile::StrictLocal => "selected by strict local policy",
        }
    };
    UserFacingRouteSummary {
        display_name,
        model_family,
        provider_origin,
        reason_short: reason_short.to_string(),
        was_fallback: trace.fallback_applied,
        was_failover: trace.runtime_failover_count > 0,
    }
}

fn extract_rootid_from_complete_request(request: &AiMethodRequest) -> Option<String> {
    request.payload.options.as_ref().and_then(|options| {
        json_non_empty_string(options.get("rootid"))
            .or_else(|| json_non_empty_string(options.get("root_id")))
    })
}

fn resolve_default_rootid(invoke_ctx: &InvokeCtx) -> String {
    let app_seed = invoke_ctx
        .caller_app_id
        .as_deref()
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .unwrap_or(AICC_SERVICE_SERVICE_NAME);
    format!("{app_seed}#default")
}

fn build_initial_aicc_task_data(
    request: &AiMethodRequest,
    external_task_id: &str,
    event_ref: Option<&str>,
    invoke_ctx: &InvokeCtx,
) -> serde_json::Value {
    let session_id = extract_session_id_from_complete_request(request);
    let rootid = extract_rootid_from_complete_request(request)
        .or_else(|| session_id.clone())
        .unwrap_or_else(|| resolve_default_rootid(invoke_ctx));

    json!({
        "rootid": rootid.clone(),
        "session_id": session_id.clone(),
        "owner_session_id": session_id.clone(),
        "aicc": {
            "version": 1,
            "external_task_id": external_task_id,
            "status": "pending",
            "created_at_ms": now_ms(),
            "updated_at_ms": now_ms(),
            "tenant_id": invoke_ctx.tenant_id,
            "event_ref": event_ref,
            "rootid": rootid,
            "session_id": session_id,
            "request": request,
            "provider_input": serde_json::Value::Null,
            "route": {},
            "output": serde_json::Value::Null,
            "provider_output": serde_json::Value::Null,
            "error": serde_json::Value::Null,
            "events": []
        }
    })
}

fn merge_route_decision_into_task_data(
    data: &mut serde_json::Value,
    decision: &RouteDecision,
    initial_status: &str,
) {
    if !data.is_object() {
        *data = json!({});
    }
    let root = data.as_object_mut().expect("task data should be object");
    if !root.contains_key("aicc") || !root["aicc"].is_object() {
        root.insert("aicc".to_string(), json!({}));
    }
    let aicc = root
        .get_mut("aicc")
        .and_then(|value| value.as_object_mut())
        .expect("aicc task payload should be object");
    aicc.insert(
        "route".to_string(),
        json!({
            "primary_instance_id": decision.primary_instance_id,
            "fallback_instance_ids": decision.fallback_instance_ids,
            "provider_model": decision.provider_model,
        }),
    );
    aicc.insert("status".to_string(), json!(initial_status));
    aicc.insert("updated_at_ms".to_string(), json!(now_ms()));
}

fn merge_task_data_with_event(data: &mut serde_json::Value, event: &TaskEvent) {
    if !data.is_object() {
        *data = json!({});
    }
    let root = data.as_object_mut().expect("task data should be object");
    if !root.contains_key("aicc") || !root["aicc"].is_object() {
        root.insert("aicc".to_string(), json!({}));
    }
    let aicc = root
        .get_mut("aicc")
        .and_then(|value| value.as_object_mut())
        .expect("aicc task payload should be object");

    let status = match event.kind {
        TaskEventKind::Queued => "queued",
        TaskEventKind::Started => "running",
        TaskEventKind::Final => "succeeded",
        TaskEventKind::Error => "failed",
        TaskEventKind::CancelRequested => "canceled",
    };
    aicc.insert("status".to_string(), json!(status));
    aicc.insert("updated_at_ms".to_string(), json!(now_ms()));

    let event_json = serde_json::to_value(event).unwrap_or_else(|_| json!({}));
    let events = aicc
        .entry("events".to_string())
        .or_insert_with(|| json!([]));
    if !events.is_array() {
        *events = json!([]);
    }
    let events_arr = events.as_array_mut().expect("events should be an array");
    events_arr.push(event_json);
    if events_arr.len() > AICC_TASK_EVENT_RETENTION {
        let to_drop = events_arr.len().saturating_sub(AICC_TASK_EVENT_RETENTION);
        events_arr.drain(0..to_drop);
    }

    match event.kind {
        TaskEventKind::Final => {
            if let Some(payload) = event.data.as_ref() {
                let summary = payload
                    .get("summary")
                    .cloned()
                    .unwrap_or_else(|| payload.clone());
                if let Some(extra) = summary.get("extra") {
                    if let Some(provider_io) = extra.get("provider_io") {
                        if let Some(input) = provider_io.get("input") {
                            aicc.insert("provider_input".to_string(), input.clone());
                        }
                        if let Some(output) = provider_io.get("output") {
                            aicc.insert("provider_output".to_string(), output.clone());
                        }
                    }
                }
                aicc.insert("output".to_string(), summary);
            }
            aicc.insert("error".to_string(), serde_json::Value::Null);
        }
        TaskEventKind::Error => {
            aicc.insert(
                "error".to_string(),
                event
                    .data
                    .clone()
                    .unwrap_or_else(|| json!({"message":"unknown"})),
            );
        }
        TaskEventKind::CancelRequested => {
            aicc.insert(
                "error".to_string(),
                event
                    .data
                    .clone()
                    .unwrap_or_else(|| json!({"message":"cancel requested"})),
            );
        }
        TaskEventKind::Started | TaskEventKind::Queued => {}
    }
}

fn capability_name(capability: &Capability) -> &'static str {
    match capability {
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

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn ewma(previous: f64, sample: f64, alpha: f64) -> f64 {
    if previous <= 0.0 {
        sample
    } else {
        ((1.0 - alpha) * previous) + (alpha * sample)
    }
}

fn range(values: impl Iterator<Item = f64>) -> (f64, f64) {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for value in values {
        min = min.min(value);
        max = max.max(value);
    }
    if min.is_infinite() || max.is_infinite() {
        (0.0, 0.0)
    } else {
        (min, max)
    }
}

fn normalize(value: f64, min: f64, max: f64) -> f64 {
    if (max - min).abs() < f64::EPSILON {
        0.0
    } else {
        (value - min) / (max - min)
    }
}

fn reason_error(code: &str, detail: impl Into<String>) -> RPCErrors {
    RPCErrors::ReasonError(format!("{}: {}", code, detail.into()))
}

fn extract_error_code(error: &RPCErrors) -> String {
    match error {
        RPCErrors::ReasonError(message) => message
            .split(':')
            .next()
            .map(|code| code.trim().to_string())
            .filter(|code| !code.is_empty())
            .unwrap_or_else(|| "internal_error".to_string()),
        RPCErrors::ParseRequestError(_) => "bad_request".to_string(),
        RPCErrors::NoPermission(_) => "forbidden".to_string(),
        _ => "internal_error".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::{
        AiPayload, AiTaskOptions, CreateTaskOptions, ModelSpec, Requirements, Task, TaskFilter,
        TaskManagerClient, TaskManagerHandler, TaskStatus,
    };
    use serde_json::json;
    use std::collections::{HashMap, VecDeque};
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct MockTaskMgrHandler {
        counter: Mutex<u64>,
        tasks: Arc<Mutex<HashMap<i64, Task>>>,
    }

    #[async_trait]
    impl TaskManagerHandler for MockTaskMgrHandler {
        async fn handle_create_task(
            &self,
            name: &str,
            task_type: &str,
            data: Option<serde_json::Value>,
            opts: CreateTaskOptions,
            user_id: &str,
            app_id: &str,
            _ctx: RPCContext,
        ) -> std::result::Result<Task, RPCErrors> {
            let mut guard = self.counter.lock().expect("counter lock");
            *guard += 1;
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let task = Task {
                id: *guard as i64,
                user_id: user_id.to_string(),
                app_id: app_id.to_string(),
                parent_id: opts.parent_id,
                root_id: String::new(),
                name: name.to_string(),
                task_type: task_type.to_string(),
                status: TaskStatus::Pending,
                progress: 0.0,
                message: None,
                data: data.unwrap_or_else(|| json!({})),
                permissions: opts.permissions.unwrap_or_default(),
                created_at: now,
                updated_at: now,
            };
            self.tasks
                .lock()
                .expect("tasks lock")
                .insert(task.id, task.clone());
            Ok(task)
        }

        async fn handle_get_task(
            &self,
            id: i64,
            _ctx: RPCContext,
        ) -> std::result::Result<Task, RPCErrors> {
            self.tasks
                .lock()
                .expect("tasks lock")
                .get(&id)
                .cloned()
                .ok_or_else(|| RPCErrors::ReasonError(format!("mock task {} not found", id)))
        }

        async fn handle_list_tasks(
            &self,
            _filter: TaskFilter,
            _source_user_id: Option<&str>,
            _source_app_id: Option<&str>,
            _ctx: RPCContext,
        ) -> std::result::Result<Vec<Task>, RPCErrors> {
            let tasks = self
                .tasks
                .lock()
                .expect("tasks lock")
                .values()
                .cloned()
                .collect::<Vec<_>>();
            Ok(tasks)
        }

        async fn handle_list_tasks_by_time_range(
            &self,
            _app_id: Option<&str>,
            _task_type: Option<&str>,
            _source_user_id: Option<&str>,
            _source_app_id: Option<&str>,
            _time_range: std::ops::Range<u64>,
            _ctx: RPCContext,
        ) -> std::result::Result<Vec<Task>, RPCErrors> {
            Ok(vec![])
        }

        async fn handle_get_subtasks(
            &self,
            _parent_id: i64,
            _ctx: RPCContext,
        ) -> std::result::Result<Vec<Task>, RPCErrors> {
            Ok(vec![])
        }

        async fn handle_update_task(
            &self,
            id: i64,
            status: Option<TaskStatus>,
            progress: Option<f32>,
            message: Option<String>,
            data: Option<serde_json::Value>,
            _ctx: RPCContext,
        ) -> std::result::Result<(), RPCErrors> {
            if let Some(task) = self.tasks.lock().expect("tasks lock").get_mut(&id) {
                if let Some(status) = status {
                    task.status = status;
                }
                if let Some(progress) = progress {
                    task.progress = progress;
                }
                if let Some(message) = message {
                    task.message = Some(message);
                }
                if let Some(data) = data {
                    task.data = data;
                }
                task.updated_at = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
            }
            Ok(())
        }

        async fn handle_update_task_progress(
            &self,
            id: i64,
            completed_items: u64,
            total_items: u64,
            _ctx: RPCContext,
        ) -> std::result::Result<(), RPCErrors> {
            if let Some(task) = self.tasks.lock().expect("tasks lock").get_mut(&id) {
                if total_items > 0 {
                    task.progress = (completed_items as f32 / total_items as f32).clamp(0.0, 1.0);
                }
            }
            Ok(())
        }

        async fn handle_update_task_status(
            &self,
            id: i64,
            status: TaskStatus,
            _ctx: RPCContext,
        ) -> std::result::Result<(), RPCErrors> {
            if let Some(task) = self.tasks.lock().expect("tasks lock").get_mut(&id) {
                task.status = status;
            }
            Ok(())
        }

        async fn handle_update_task_error(
            &self,
            id: i64,
            error_message: &str,
            _ctx: RPCContext,
        ) -> std::result::Result<(), RPCErrors> {
            if let Some(task) = self.tasks.lock().expect("tasks lock").get_mut(&id) {
                task.status = TaskStatus::Failed;
                task.message = Some(error_message.to_string());
            }
            Ok(())
        }

        async fn handle_update_task_data(
            &self,
            id: i64,
            data: serde_json::Value,
            _ctx: RPCContext,
        ) -> std::result::Result<(), RPCErrors> {
            if let Some(task) = self.tasks.lock().expect("tasks lock").get_mut(&id) {
                task.data = data;
            }
            Ok(())
        }

        async fn handle_cancel_task(
            &self,
            _id: i64,
            _recursive: bool,
            _ctx: RPCContext,
        ) -> std::result::Result<(), RPCErrors> {
            Ok(())
        }

        async fn handle_delete_task(
            &self,
            _id: i64,
            _ctx: RPCContext,
        ) -> std::result::Result<(), RPCErrors> {
            Ok(())
        }
    }

    #[derive(Debug)]
    struct MockProvider {
        instance: ProviderInstance,
        inventory: ProviderInventory,
        cost: CostEstimateOutput,
        start_results: Mutex<VecDeque<std::result::Result<ProviderStartResult, ProviderError>>>,
        start_call_count: std::sync::atomic::AtomicUsize,
        canceled: Mutex<Vec<String>>,
    }

    impl MockProvider {
        fn new(
            instance: ProviderInstance,
            cost: CostEstimateOutput,
            start_results: Vec<std::result::Result<ProviderStartResult, ProviderError>>,
        ) -> Self {
            let inventory = mock_inventory(&instance);
            Self {
                instance,
                inventory,
                cost,
                start_results: Mutex::new(start_results.into_iter().collect()),
                start_call_count: std::sync::atomic::AtomicUsize::new(0),
                canceled: Mutex::new(vec![]),
            }
        }

        fn start_calls(&self) -> usize {
            self.start_call_count.load(AtomicOrdering::Relaxed)
        }
    }

    #[async_trait]
    impl Provider for MockProvider {
        fn inventory(&self) -> ProviderInventory {
            self.inventory.clone()
        }

        fn estimate_cost(&self, _input: &CostEstimateInput) -> CostEstimateOutput {
            self.cost.clone()
        }

        async fn start(
            &self,
            _ctx: InvokeCtx,
            _provider_model: String,
            _req: ResolvedRequest,
            _sink: Arc<dyn TaskEventSink>,
        ) -> std::result::Result<ProviderStartResult, ProviderError> {
            self.start_call_count.fetch_add(1, AtomicOrdering::Relaxed);
            let mut queue = self.start_results.lock().unwrap();
            queue
                .pop_front()
                .unwrap_or_else(|| Err(ProviderError::fatal("no preset start result")))
        }

        async fn cancel(
            &self,
            _ctx: InvokeCtx,
            task_id: &str,
        ) -> std::result::Result<(), ProviderError> {
            let mut canceled = self.canceled.lock().unwrap();
            canceled.push(task_id.to_string());
            Ok(())
        }
    }

    fn base_request() -> AiMethodRequest {
        AiMethodRequest::new(
            Capability::Llm,
            ModelSpec::new("llm.plan.default".to_string(), None),
            Requirements::new(vec!["plan".to_string()], Some(3000), Some(0.1), None),
            AiPayload::new(
                Some("hello".to_string()),
                vec![],
                vec![],
                vec![],
                None,
                Some(json!({"temperature": 0.1})),
            ),
            Some("idem-1".to_string()),
        )
    }

    fn mock_instance(instance_id: &str, provider_type: &str) -> ProviderInstance {
        ProviderInstance {
            provider_instance_name: instance_id.to_string(),
            provider_type: ProviderType::CloudApi,
            provider_driver: provider_type.to_string(),
            provider_origin: ProviderOrigin::SystemConfig,
            provider_type_trusted_source: ProviderTypeTrustedSource::SystemConfig,
            provider_type_revision: None,
            capabilities: vec![Capability::Llm],
            features: vec!["plan".to_string()],
            endpoint: Some("http://127.0.0.1:8080".to_string()),
            plugin_key: None,
        }
    }

    fn mock_inventory(instance: &ProviderInstance) -> ProviderInventory {
        ProviderInventory {
            provider_instance_name: instance.provider_instance_name.clone(),
            provider_type: instance.provider_type.clone(),
            provider_driver: instance.provider_driver.clone(),
            provider_origin: instance.provider_origin.clone(),
            provider_type_trusted_source: instance.provider_type_trusted_source.clone(),
            provider_type_revision: None,
            version: None,
            inventory_revision: Some("test".to_string()),
            models: vec![provider_model_metadata(
                instance.provider_instance_name.as_str(),
                instance.provider_type.clone(),
                "gpt-4o-mini",
                ApiType::LlmChat,
                vec!["llm.plan.default".to_string()],
                &instance.features,
                Some(0.001),
                Some(100),
            )],
        }
    }

    fn cost(estimated_cost_usd: f64, estimated_latency_ms: u64) -> CostEstimateOutput {
        CostEstimateOutput {
            estimated_cost_usd,
            pricing_mode: PricingMode::PerToken,
            quota_state: QuotaState::Normal,
            confidence: 1.0,
            estimated_latency_ms: Some(estimated_latency_ms),
        }
    }

    fn center_with_taskmgr(registry: Registry, catalog: ModelCatalog) -> AIComputeCenter {
        let mut center = AIComputeCenter::new(registry, catalog);
        for inventory in center.registry().inventories() {
            center
                .model_registry()
                .write()
                .expect("model registry lock")
                .apply_inventory(inventory)
                .expect("mock inventory should be valid");
        }
        let taskmgr = TaskManagerClient::new_in_process(Box::new(MockTaskMgrHandler {
            counter: Mutex::new(0),
            tasks: Arc::new(Mutex::new(HashMap::new())),
        }));
        center.set_task_manager_client(Arc::new(taskmgr));
        center
    }

    #[tokio::test]
    async fn complete_returns_immediate_success() {
        let registry = Registry::default();
        let catalog = ModelCatalog::default();
        catalog.set_mapping(
            Capability::Llm,
            "llm.plan.default",
            "provider-a",
            "gpt-4o-mini",
        );

        let provider = Arc::new(MockProvider::new(
            mock_instance("provider-a-1", "provider-a"),
            cost(0.001, 200),
            vec![Ok(ProviderStartResult::Immediate(AiResponseSummary {
                text: Some("ok".to_string()),
                tool_calls: vec![],
                artifacts: vec![],
                usage: None,
                cost: None,
                finish_reason: Some("stop".to_string()),
                provider_task_ref: None,
                extra: None,
            }))],
        ));
        registry.add_provider(provider);

        let center = center_with_taskmgr(registry, catalog);
        let response = center
            .complete(
                base_request(),
                RPCContext::from_request(
                    &RPCRequest {
                        method: "llm.chat".to_string(),
                        params: json!({}),
                        seq: 1,
                        token: None,
                        trace_id: None,
                    },
                    IpAddr::V4(Ipv4Addr::LOCALHOST),
                ),
            )
            .await
            .unwrap();

        assert_eq!(response.status, AiMethodStatus::Succeeded);
        assert_eq!(
            response
                .result
                .as_ref()
                .and_then(|result| result.text.as_ref()),
            Some(&"ok".to_string())
        );

        let taskmgr = center.taskmgr.as_ref().expect("task manager").clone();
        let tasks = taskmgr
            .list_tasks(None, None, None)
            .await
            .expect("list tasks");
        let task = tasks
            .into_iter()
            .find(|item| {
                item.data
                    .pointer("/aicc/external_task_id")
                    .and_then(|value| value.as_str())
                    == Some(response.task_id.as_str())
            })
            .expect("immediate provider completion should persist aicc task");
        assert_eq!(task.status, TaskStatus::Completed);
        assert_eq!(
            task.data
                .pointer("/aicc/status")
                .and_then(|value| value.as_str()),
            Some("succeeded")
        );
        assert_eq!(
            task.data.get("rootid").and_then(|value| value.as_str()),
            Some("aicc#default")
        );
    }

    #[tokio::test]
    async fn complete_fallback_on_retryable_start_error() {
        let registry = Registry::default();
        let catalog = ModelCatalog::default();
        catalog.set_mapping(
            Capability::Llm,
            "llm.plan.default",
            "provider-a",
            "gpt-4o-mini",
        );
        catalog.set_mapping(
            Capability::Llm,
            "llm.plan.default",
            "provider-b",
            "gpt-4.1-mini",
        );

        let p1 = Arc::new(MockProvider::new(
            mock_instance("provider-a-1", "provider-a"),
            cost(0.001, 100),
            vec![Err(ProviderError::retryable(
                "upstream temporary unavailable",
            ))],
        ));
        let p2 = Arc::new(MockProvider::new(
            mock_instance("provider-b-1", "provider-b"),
            cost(0.002, 250),
            vec![Ok(ProviderStartResult::Started)],
        ));
        registry.add_provider(p1);
        registry.add_provider(p2);

        let center = center_with_taskmgr(registry, catalog);
        let response = center
            .complete(base_request(), RPCContext::default())
            .await
            .unwrap();

        assert_eq!(response.status, AiMethodStatus::Running);
        assert!(!response.task_id.is_empty());

        let taskmgr = center.taskmgr.as_ref().expect("task manager").clone();
        let tasks = taskmgr
            .list_tasks(None, None, None)
            .await
            .expect("list tasks");
        let task = tasks
            .into_iter()
            .find(|item| {
                item.data
                    .pointer("/aicc/external_task_id")
                    .and_then(|value| value.as_str())
                    == Some(response.task_id.as_str())
            })
            .expect("running response should persist task");
        assert_eq!(task.status, TaskStatus::Running);
        assert_eq!(
            task.message.as_deref(),
            Some("request sent, waiting for provider response")
        );
        assert!(task.data.pointer("/aicc/request").is_some());
    }

    #[tokio::test]
    async fn complete_persists_queued_task_state() {
        let registry = Registry::default();
        let catalog = ModelCatalog::default();
        catalog.set_mapping(
            Capability::Llm,
            "llm.plan.default",
            "provider-a",
            "gpt-4o-mini",
        );

        let provider = Arc::new(MockProvider::new(
            mock_instance("provider-a-1", "provider-a"),
            cost(0.001, 100),
            vec![Ok(ProviderStartResult::Queued { position: 3 })],
        ));
        registry.add_provider(provider);

        let center = center_with_taskmgr(registry, catalog);
        let response = center
            .complete(base_request(), RPCContext::default())
            .await
            .expect("complete should return queued task");
        assert_eq!(response.status, AiMethodStatus::Running);

        let taskmgr = center.taskmgr.as_ref().expect("task manager").clone();
        let tasks = taskmgr
            .list_tasks(None, None, None)
            .await
            .expect("list tasks");
        let task = tasks
            .into_iter()
            .find(|item| {
                item.data
                    .pointer("/aicc/external_task_id")
                    .and_then(|value| value.as_str())
                    == Some(response.task_id.as_str())
            })
            .expect("queued response should persist task");

        assert_eq!(task.status, TaskStatus::Pending);
        assert_eq!(task.message.as_deref(), Some(QUEUE_STATUS_QUEUED));
        assert_eq!(
            task.data
                .pointer("/aicc/status")
                .and_then(|value| value.as_str()),
            Some("queued")
        );
        assert_eq!(
            task.data
                .pointer("/aicc/events/0/kind")
                .and_then(|value| value.as_str()),
            Some("queued")
        );
    }

    #[tokio::test]
    async fn complete_respects_parent_task_option() {
        let registry = Registry::default();
        let catalog = ModelCatalog::default();
        catalog.set_mapping(
            Capability::Llm,
            "llm.plan.default",
            "provider-a",
            "gpt-4o-mini",
        );
        let provider = Arc::new(MockProvider::new(
            mock_instance("provider-a-1", "provider-a"),
            cost(0.001, 200),
            vec![Ok(ProviderStartResult::Started)],
        ));
        registry.add_provider(provider);

        let center = center_with_taskmgr(registry, catalog);
        let taskmgr = center.taskmgr.as_ref().expect("task manager").clone();
        let parent_task = taskmgr
            .create_task(
                "behavior-parent",
                "opendan.behavior",
                Some(json!({"kind":"behavior"})),
                "did:web:jarvis.test.buckyos.io",
                "opendan-llm-behavior",
                None,
            )
            .await
            .expect("create parent task");
        let request = base_request().with_task_options(Some(AiTaskOptions {
            parent_id: Some(parent_task.id),
        }));
        let response = center
            .complete(request, RPCContext::default())
            .await
            .expect("complete should succeed");
        assert_eq!(response.status, AiMethodStatus::Running);

        let tasks = taskmgr
            .list_tasks(None, None, None)
            .await
            .expect("list tasks");
        let task = tasks
            .into_iter()
            .find(|item| {
                item.data
                    .pointer("/aicc/external_task_id")
                    .and_then(|value| value.as_str())
                    == Some(response.task_id.as_str())
            })
            .expect("aicc task should exist");
        assert_eq!(task.parent_id, Some(parent_task.id));
    }

    #[tokio::test]
    async fn complete_persists_rootid_and_session_id_from_request_options() {
        let registry = Registry::default();
        let catalog = ModelCatalog::default();
        catalog.set_mapping(
            Capability::Llm,
            "llm.plan.default",
            "provider-a",
            "gpt-4o-mini",
        );
        let provider = Arc::new(MockProvider::new(
            mock_instance("provider-a-1", "provider-a"),
            cost(0.001, 120),
            vec![Ok(ProviderStartResult::Started)],
        ));
        registry.add_provider(provider);

        let center = center_with_taskmgr(registry, catalog);
        let mut request = base_request();
        if let Some(options) = request.payload.options.as_mut() {
            if let Some(map) = options.as_object_mut() {
                map.insert("session_id".to_string(), json!("session-xyz"));
                map.insert("rootid".to_string(), json!("session-xyz"));
            }
        }

        let response = center
            .complete(request, RPCContext::default())
            .await
            .expect("complete should succeed");
        assert_eq!(response.status, AiMethodStatus::Running);

        let taskmgr = center.taskmgr.as_ref().expect("task manager").clone();
        let tasks = taskmgr
            .list_tasks(None, None, None)
            .await
            .expect("list tasks");
        let task = tasks
            .into_iter()
            .find(|item| {
                item.data
                    .pointer("/aicc/external_task_id")
                    .and_then(|value| value.as_str())
                    == Some(response.task_id.as_str())
            })
            .expect("aicc task should exist");
        assert_eq!(
            task.data.get("rootid").and_then(|value| value.as_str()),
            Some("session-xyz")
        );
        assert_eq!(
            task.data.get("session_id").and_then(|value| value.as_str()),
            Some("session-xyz")
        );
        assert_eq!(
            task.data
                .pointer("/aicc/rootid")
                .and_then(|value| value.as_str()),
            Some("session-xyz")
        );
    }

    #[tokio::test]
    async fn complete_prefers_sn_ai_provider_when_free_credit_covers_estimated_cost() {
        let registry = Registry::default();
        let catalog = ModelCatalog::default();
        catalog.set_mapping(
            Capability::Llm,
            "llm.plan.default",
            "sn-ai-provider",
            "gpt-5-mini",
        );
        catalog.set_mapping(Capability::Llm, "llm.plan.default", "openai", "gpt-5-mini");

        let sn_provider = Arc::new(MockProvider::new(
            mock_instance("sn-ai-provider-1", "sn-ai-provider"),
            cost(0.20, 80),
            vec![Ok(ProviderStartResult::Started)],
        ));
        let paid_provider = Arc::new(MockProvider::new(
            mock_instance("openai-1", "openai"),
            cost(0.05, 20),
            vec![Ok(ProviderStartResult::Started)],
        ));
        registry.add_provider(sn_provider.clone());
        registry.add_provider(paid_provider.clone());

        let center = center_with_taskmgr(registry, catalog);
        center.update_route_config(RouteConfig {
            global_weights: RouteWeights {
                w_cost: 1.0,
                w_latency: 0.0,
                w_load: 0.0,
                w_error: 0.0,
            },
            ..RouteConfig::default()
        });

        let mut request = base_request();
        request.requirements.max_cost_usd = Some(0.10);
        let response = center
            .complete(request, RPCContext::default())
            .await
            .expect("complete should succeed");

        assert_eq!(response.status, AiMethodStatus::Running);
        assert_eq!(sn_provider.start_calls(), 1);
        assert_eq!(paid_provider.start_calls(), 0);
    }

    #[tokio::test]
    async fn complete_applies_sn_ai_provider_free_credit_before_reporting_cost() {
        let registry = Registry::default();
        let catalog = ModelCatalog::default();
        catalog.set_mapping(
            Capability::Llm,
            "llm.plan.default",
            "sn-ai-provider",
            "gpt-5-mini",
        );

        let provider = Arc::new(MockProvider::new(
            mock_instance("sn-ai-provider-1", "sn-ai-provider"),
            cost(2.0, 100),
            vec![
                Ok(ProviderStartResult::Immediate(AiResponseSummary {
                    text: Some("first".to_string()),
                    tool_calls: vec![],
                    artifacts: vec![],
                    usage: None,
                    cost: Some(buckyos_api::AiCost {
                        amount: 2.0,
                        currency: "USD".to_string(),
                    }),
                    finish_reason: Some("stop".to_string()),
                    provider_task_ref: None,
                    extra: None,
                })),
                Ok(ProviderStartResult::Immediate(AiResponseSummary {
                    text: Some("second".to_string()),
                    tool_calls: vec![],
                    artifacts: vec![],
                    usage: None,
                    cost: Some(buckyos_api::AiCost {
                        amount: 14.0,
                        currency: "USD".to_string(),
                    }),
                    finish_reason: Some("stop".to_string()),
                    provider_task_ref: None,
                    extra: None,
                })),
            ],
        ));
        registry.add_provider(provider);

        let center = center_with_taskmgr(registry, catalog);

        let mut first_request = base_request();
        first_request.requirements.max_cost_usd = Some(20.0);
        let first = center
            .complete(first_request, RPCContext::default())
            .await
            .expect("first complete should succeed");
        assert_eq!(first.status, AiMethodStatus::Succeeded);
        assert_eq!(
            first
                .result
                .as_ref()
                .and_then(|summary| summary.cost.as_ref())
                .map(|cost| cost.amount),
            Some(0.0)
        );
        assert_eq!(
            first
                .result
                .as_ref()
                .and_then(|summary| summary.extra.as_ref())
                .and_then(|extra| extra.pointer("/billing/sn_ai_provider_credit_applied_usd"))
                .and_then(|value| value.as_f64()),
            Some(2.0)
        );

        let mut second_request = base_request();
        second_request.requirements.max_cost_usd = Some(20.0);
        let second = center
            .complete(second_request, RPCContext::default())
            .await
            .expect("second complete should succeed");
        assert_eq!(second.status, AiMethodStatus::Succeeded);
        assert_eq!(
            second
                .result
                .as_ref()
                .and_then(|summary| summary.cost.as_ref())
                .map(|cost| cost.amount),
            Some(1.0)
        );
    }

    #[tokio::test]
    async fn complete_no_provider_does_not_create_task() {
        let registry = Registry::default();
        let catalog = ModelCatalog::default();
        let center = center_with_taskmgr(registry, catalog);

        let response = center
            .complete(base_request(), RPCContext::default())
            .await
            .expect("complete should return failed response");
        assert_eq!(response.status, AiMethodStatus::Failed);

        let taskmgr = center.taskmgr.as_ref().expect("task manager").clone();
        let tasks = taskmgr
            .list_tasks(None, None, None)
            .await
            .expect("list tasks");
        assert!(tasks.is_empty(), "routing failure should not persist task");
    }

    #[tokio::test]
    async fn cancel_rejects_cross_tenant_task() {
        let registry = Registry::default();
        let catalog = ModelCatalog::default();
        catalog.set_mapping(
            Capability::Llm,
            "llm.plan.default",
            "provider-a",
            "gpt-4o-mini",
        );

        let provider = Arc::new(MockProvider::new(
            mock_instance("provider-a-1", "provider-a"),
            cost(0.001, 100),
            vec![Ok(ProviderStartResult::Started)],
        ));
        registry.add_provider(provider);

        let center = center_with_taskmgr(registry, catalog);

        let alice_ctx = RPCContext {
            token: Some("tenant-alice".to_string()),
            ..Default::default()
        };
        let start_response = center.complete(base_request(), alice_ctx).await.unwrap();
        assert_eq!(start_response.status, AiMethodStatus::Running);

        let bob_ctx = RPCContext {
            token: Some("tenant-bob".to_string()),
            ..Default::default()
        };
        let cancel_result = center
            .handle_cancel(start_response.task_id.as_str(), bob_ctx)
            .await;
        assert!(cancel_result.is_err());
        assert!(matches!(
            cancel_result.unwrap_err(),
            RPCErrors::NoPermission(_)
        ));
    }
}
