use crate::aicc_usage_log_db::AiccUsageLogDb;
use crate::complete_request_queue::QUEUE_STATUS_QUEUED;
use crate::model_registry::{
    InventoryRefreshScheduler, ModelRegistry, DEFAULT_INVENTORY_REFRESH_INTERVAL,
};
use crate::model_router::{ModelRouter, RouteRequest};
use crate::model_scheduler::ModelScheduler;
use crate::model_session::{
    build_effective_session_config, merge_session_config, EffectiveSessionConfig, LogicalNode,
    SessionConfig,
};
use crate::model_types::{
    ApiType, CostClass, CostEstimateInput, CostEstimateOutput, ExactModelName, HealthStatus,
    LatencyClass, LogicalModelDefinition, ModelAttributes, ModelCandidate, ModelCapabilities,
    ModelHealth, ModelMetadata, ModelPricing, PolicyConfig, PricingMode, PrivacyClass,
    ProviderInventory, ProviderOrigin, ProviderType, ProviderTypeTrustedSource, QuotaState,
    RequiredModelFeatures, RouteError, RouteErrorCode, RoutePolicy,
    RoutePricingSnapshot, RouteTrace, UserFacingProviderOrigin, UserFacingRouteSummary,
};
use ::kRPC::*;
use async_trait::async_trait;
use base64::engine::general_purpose;
use base64::Engine as _;
use buckyos_api::{
    ai_methods, get_buckyos_api_runtime, task_mgr_error_code, validate_verify_hub_token_claims,
    AckControlReq, AiContent, AiMethodRequest, AiMethodResponse, AiMethodStatus, AiPayload,
    AiResponse, AiccComputeProgress, AiccComputeTaskData, AiccComputeTaskRequest, AiccHandler,
    AiccRouteOverlay, AiccRouteTraceEvent, AiccUsageEvent, AiccVideoContinuationSource,
    CancelResponse, Capability, CommitResultReq, CreateTaskExecutor, CreateTaskReq, FailTaskReq,
<<<<<<< HEAD
    Feature, LlmChatInvokeRequest, LlmChatInvokeResponse, LlmResponseFormat, ModelSpec,
    ReportProgressReq, ReportStartedReq, Requirements, ResourceRef, RouteFallbackAttempt,
    RouteResolveRequest, RouteResolveResponse, RunnerWriteEnvelope, TaskControlAction, TaskError,
    TaskManagerClient, TaskPhase, TextToImageInvokeRequest, TextToImageInvokeResponse, TokenUse,
    TypedTaskData, AICC_SERVICE_SERVICE_NAME,
use ndn_lib::{
    load_named_object_from_obj_str, ChunkHasher, ChunkId, FileObject, NamedObject, ObjId,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{watch, Mutex as AsyncMutex};

const DEFAULT_FALLBACK_LIMIT: usize = 2;
const DEFAULT_BASE64_MAX_BYTES: usize = 8 * 1024 * 1024;
const EWMA_ALPHA: f64 = 0.2;
const AICC_TASK_SCHEMA_ID: &str = "aicc.compute/v1";
const AICC_TASK_EVENT_RETENTION: usize = 64;
const REDACTED_BASE64_PLACEHOLDER: &str = "[redacted_base64]";
const REDACTED_DATA_URL_BASE64_PREFIX: &str = "[redacted_data_url_base64";
const REDACTED_LONG_BASE64_LIKE_PLACEHOLDER: &str = "[redacted_base64_like_string]";
const REDACTED_LOG_TEXT_PLACEHOLDER: &str = "[redacted_text]";
const LOG_BASE64_LIKE_MIN_CHARS: usize = 512;
const SN_AI_PROVIDER_FREE_CREDIT_USD: f64 = 15.0;
const REFRESH_IDLE: u8 = 0;
const REFRESH_REQUEST_ACTIVE: u8 = 1;
const REFRESH_STOPPED: u8 = 2;

#[derive(Debug)]
pub(crate) struct ProviderRefreshTask {
    shutdown_tx: watch::Sender<bool>,
    state: AtomicU8,
    #[cfg(test)]
    started_requests: AtomicU64,
}

impl ProviderRefreshTask {
    pub(crate) fn new() -> (Arc<Self>, watch::Receiver<bool>) {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        (
            Arc::new(Self {
                shutdown_tx,
                state: AtomicU8::new(REFRESH_IDLE),
                #[cfg(test)]
                started_requests: AtomicU64::new(0),
            }),
            shutdown_rx,
        )
    }

    pub(crate) fn try_start_request(&self) -> bool {
        let started = self
            .state
            .compare_exchange(
                REFRESH_IDLE,
                REFRESH_REQUEST_ACTIVE,
                AtomicOrdering::AcqRel,
                AtomicOrdering::Acquire,
            )
            .is_ok();
        #[cfg(test)]
        if started {
            self.started_requests.fetch_add(1, AtomicOrdering::Relaxed);
        }
        started
    }

    pub(crate) fn finish_request(&self) {
        let _ = self.state.compare_exchange(
            REFRESH_REQUEST_ACTIVE,
            REFRESH_IDLE,
            AtomicOrdering::AcqRel,
            AtomicOrdering::Acquire,
        );
    }

    pub(crate) fn shutdown(&self) {
        self.state.store(REFRESH_STOPPED, AtomicOrdering::Release);
        let _ = self.shutdown_tx.send(true);
    }

    pub(crate) fn is_stopped(&self) -> bool {
        self.state.load(AtomicOrdering::Acquire) == REFRESH_STOPPED
    }

    #[cfg(test)]
    pub(crate) fn started_requests(&self) -> u64 {
        self.started_requests.load(AtomicOrdering::Relaxed)
    }
}

#[derive(Clone, Debug, Default)]
pub struct InvokeCtx {
    pub tenant_id: String,
    pub caller_app_id: Option<String>,
    pub session_token: Option<String>,
    pub trace_id: Option<String>,
    pub task_id: Option<String>,
}

impl InvokeCtx {
    fn from_unverified_rpc(ctx: &RPCContext) -> Self {
        Self {
            tenant_id: "anonymous".to_string(),
            caller_app_id: None,
            session_token: ctx.token.clone(),
            trace_id: ctx.trace_id.clone(),
            task_id: None,
        }
    }

    fn apply_verified_session(&mut self, parsed: &RPCSessionToken) -> Result<()> {
        let claims = validate_verify_hub_token_claims(parsed, TokenUse::Session)?;
        let sub = parsed
            .sub
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| RPCErrors::InvalidToken("session token has no subject".to_string()))?;
        self.tenant_id = sub.to_string();
        self.caller_app_id = Some(claims.target.canonical_key());
        Ok(())
    }

    pub async fn from_rpc(ctx: &RPCContext) -> Self {
        let mut invoke_ctx = Self::from_unverified_rpc(ctx);
        if let Some(token) = ctx
            .token
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            if let Ok(runtime) = get_buckyos_api_runtime() {
                if let Ok(parsed) = runtime.verify_trusted_session_token(token).await {
                    let _ = invoke_ctx.apply_verified_session(&parsed);
                }
            }
        }
        invoke_ctx
    }
}

fn redact_data_url_base64(value: &mut String) {
    let Some((metadata, data)) = value.split_once(";base64,") else {
        return;
    };
    if !metadata.starts_with("data:") {
        return;
    }
    *value = format!(
        "{} mime={} len={}]",
        REDACTED_DATA_URL_BASE64_PREFIX,
        metadata.trim_start_matches("data:"),
        data.len()
    );
}

fn looks_like_base64_payload(value: &str) -> bool {
    if value.len() < LOG_BASE64_LIKE_MIN_CHARS {
        return false;
    }

    let mut normalized = String::with_capacity(value.len());
    for byte in value.bytes() {
        if matches!(byte, b'\n' | b'\r') {
            continue;
        }
        if byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=' | b'-' | b'_') {
            normalized.push(byte as char);
            continue;
        }
        return false;
    }

    if normalized.len() < LOG_BASE64_LIKE_MIN_CHARS {
        return false;
    }

    general_purpose::STANDARD
        .decode(normalized.as_bytes())
        .is_ok()
        || general_purpose::STANDARD_NO_PAD
            .decode(normalized.as_bytes())
            .is_ok()
        || general_purpose::URL_SAFE
            .decode(normalized.as_bytes())
            .is_ok()
        || general_purpose::URL_SAFE_NO_PAD
            .decode(normalized.as_bytes())
            .is_ok()
}

fn redact_base64_like_string(value: &mut String) {
    if looks_like_base64_payload(value.as_str()) {
        *value = format!(
            "{} len={}",
            REDACTED_LONG_BASE64_LIKE_PLACEHOLDER,
            value.len()
        );
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
                if let Some(text) = data_base64.as_str() {
                    *data_base64 = json!(format!(
                        "{} len={}",
                        REDACTED_BASE64_PLACEHOLDER,
                        text.len()
                    ));
                }
            }
            if let Some(b64_json) = map.get_mut("b64_json") {
                if let Some(text) = b64_json.as_str() {
                    *b64_json = json!(format!(
                        "{} len={}",
                        REDACTED_BASE64_PLACEHOLDER,
                        text.len()
                    ));
                }
            }
            if (map.contains_key("mimeType") || map.contains_key("mime_type"))
                && map.get("data").and_then(|value| value.as_str()).is_some()
            {
                let len = map
                    .get("data")
                    .and_then(|value| value.as_str())
                    .map(|value| value.len())
                    .unwrap_or_default();
                map.insert(
                    "data".to_string(),
                    json!(format!("{} len={}", REDACTED_BASE64_PLACEHOLDER, len)),
                );
            }
            for key in ["thoughtSignature", "thought_signature", "signature"] {
                if let Some(field) = map.get_mut(key) {
                    if let Some(text) = field.as_str() {
                        if looks_like_base64_payload(text) {
                            *field = json!(format!(
                                "{} len={}",
                                REDACTED_LONG_BASE64_LIKE_PLACEHOLDER,
                                text.len()
                            ));
                        }
                    }
                }
            }
            for nested in map.values_mut() {
                redact_base64_fields(nested);
            }
        }
        Value::String(text) => {
            redact_data_url_base64(text);
            redact_base64_like_string(text);
        }
        _ => {}
    }
}

fn redact_all_log_strings(value: &mut Value) {
    match value {
        Value::Array(items) => {
            for item in items {
                redact_all_log_strings(item);
            }
        }
        Value::Object(map) => {
            for nested in map.values_mut() {
                redact_all_log_strings(nested);
            }
        }
        Value::String(text) => {
            *text = format!("{} len={}", REDACTED_LOG_TEXT_PLACEHOLDER, text.len());
        }
        _ => {}
    }
}

fn redact_sensitive_log_fields(value: &mut Value) {
    match value {
        Value::Array(items) => {
            for item in items {
                redact_sensitive_log_fields(item);
            }
        }
        Value::Object(map) => {
            for (key, nested) in map.iter_mut() {
                let normalized = key.to_ascii_lowercase().replace('_', "").replace('-', "");
                if matches!(normalized.as_str(), "args" | "arguments" | "queries") {
                    redact_all_log_strings(nested);
                    continue;
                }
                if matches!(
                    normalized.as_str(),
                    "text"
                        | "prompt"
                        | "command"
                        | "instructions"
                        | "system"
                        | "content"
                        | "input"
                        | "output"
                        | "caption"
                        | "transcript"
                        | "query"
                        | "searchsuggestions"
                        | "url"
                        | "fileuri"
                        | "gcsuri"
                ) {
                    if nested.is_string() {
                        redact_all_log_strings(nested);
                        continue;
                    }
                }
                redact_sensitive_log_fields(nested);
            }
        }
        _ => {}
    }
}

fn redacted_summary_value(summary: &AiResponse) -> Value {
    let mut value = serde_json::to_value(summary).unwrap_or_else(|_| json!({}));
    redact_base64_fields(&mut value);
    value
}

pub(crate) fn redacted_json_log(value: &Value) -> String {
    let mut value = value.clone();
    redact_base64_fields(&mut value);
    redact_sensitive_log_fields(&mut value);
    value.to_string()
}

#[derive(Clone, Debug)]
pub struct ProviderInstance {
    pub provider_instance_name: String,
    pub provider_type: ProviderType,
    pub provider_driver: String,
    pub provider_origin: ProviderOrigin,
    pub provider_type_trusted_source: ProviderTypeTrustedSource,
    pub provider_type_revision: Option<String>,
    pub endpoint: Option<String>,
    pub plugin_key: Option<String>,
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
    Immediate(AiResponse),
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

    #[cfg(test)]
    #[allow(dead_code)]
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
    taskmgr_override: Option<Arc<TaskManagerClient>>,
    taskmgr_context: RPCContext,
    task_mgr_id: String,
    tenant_id: String,
    continuation_db: Option<Arc<AiccUsageLogDb>>,
    lock: AsyncMutex<()>,
}

impl TaskAuditSink {
    fn new(
        taskmgr_override: Option<Arc<TaskManagerClient>>,
        taskmgr_context: RPCContext,
        task_mgr_id: String,
        tenant_id: String,
        continuation_db: Option<Arc<AiccUsageLogDb>>,
    ) -> Self {
        Self {
            taskmgr_override,
            taskmgr_context,
            task_mgr_id,
            tenant_id,
            continuation_db,
            lock: AsyncMutex::new(()),
        }
    }

    async fn record_video_continuation_source(&self, event: &TaskEvent) {
        let Some(db) = self.continuation_db.as_ref() else {
            return;
        };
        let Some(extra) = event
            .data
            .as_ref()
            .and_then(|data| data.get("summary"))
            .and_then(|summary| summary.get("extra"))
        else {
            return;
        };
        let has_continuation = extra
            .get("continuation_handle")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty());
        if !has_continuation {
            return;
        }
        let Some(artifacts) = extra
            .get("materialized_artifacts")
            .and_then(Value::as_array)
        else {
            return;
        };
        for artifact in artifacts {
            let is_video = artifact
                .get("mime")
                .and_then(Value::as_str)
                .is_some_and(|mime| mime.starts_with("video/"));
            let Some(content_id) = artifact
                .get("content_id")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
            else {
                continue;
            };
            if !is_video {
                continue;
            }
            let record = AiccVideoContinuationSource {
                tenant_id: self.tenant_id.clone(),
                content_id: content_id.to_string(),
                source_task_id: self.task_mgr_id.clone(),
                created_at_ms: now_ms_i64(),
            };
            match db.upsert_video_continuation_source(&record).await {
                Ok(()) => info!(
                    "aicc.video_continuation source persisted: tenant={} content_id={} source_task_id={}",
                    record.tenant_id, record.content_id, record.source_task_id
                ),
                Err(error) => warn!(
                    "aicc.video_continuation source persist_failed: tenant={} source_task_id={} err={}",
                    record.tenant_id, record.source_task_id, error
                ),
            }
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
    provider_model: Arc<std::sync::RwLock<String>>,
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
            provider_model: self
                .context
                .provider_model
                .read()
                .map(|model| model.clone())
                .unwrap_or_else(|_| "unknown".to_string()),
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

struct PreparedTask {
    taskmgr_override: Option<Arc<TaskManagerClient>>,
    taskmgr_context: RPCContext,
    task: buckyos_api::Task,
}

impl PreparedTask {
    fn id(&self) -> String {
        self.task.task_id.clone()
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
    async fn acquire_task_manager_client(
        &self,
        invoke_ctx: &InvokeCtx,
    ) -> std::result::Result<Arc<TaskManagerClient>, RPCErrors> {
        let taskmgr_context = task_manager_rpc_context(invoke_ctx);
        acquire_task_manager_client(self.taskmgr.as_ref(), &taskmgr_context).await
    }

    fn resolve_task_parent(request: &AiMethodRequest) -> Option<String> {
        request
            .task_options
            .as_ref()
            .and_then(|task_options| task_options.parent_id.clone())
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
        let parent_id = Self::resolve_task_parent(request);

        let mut task_data =
            build_initial_aicc_task_data(request, external_task_id, event_ref, invoke_ctx);
        merge_route_decision_into_task_data(&mut task_data, decision, initial_state.as_status());

        let taskmgr_context = task_manager_rpc_context(invoke_ctx);
        let taskmgr = acquire_task_manager_client(self.taskmgr.as_ref(), &taskmgr_context).await?;
        let task = taskmgr
            .create_task(CreateTaskReq {
                name: format!("aicc:{external_task_id}"),
                schema_id: AICC_TASK_SCHEMA_ID.to_string(),
                schema_version: None,
                input: task_data.clone(),
                // The caller's app identity runs the compute through aicc;
                // later runner writes reuse the caller's session token.
                executor: CreateTaskExecutor::SelfApp {
                    app_instance_id: None,
                },
                parent_id: parent_id.clone(),
                child_control_policy: None,
                policy_preset: None,
                permission_boundary: false,
                storage_domain: None,
                idempotency_key: format!("aicc:{external_task_id}"),
                retry_of: None,
                supersedes: None,
                message: None,
            })
            .await
            .map_err(|err| {
                warn!(
                    "aicc.complete create_task failed: task_id={} tenant={} parent_id={:?} err={}",
                    external_task_id, invoke_ctx.tenant_id, parent_id, err
                );
                err
            })?;
        // Seed the mutable audit trail in the progress snapshot so readers
        // always find the merged event history there.
        let task = taskmgr
            .report_progress(ReportProgressReq {
                envelope: RunnerWriteEnvelope {
                    task_id: task.task_id.clone(),
                    app_instance_id: None,
                    runner_epoch: task.runner_epoch,
                    expected_revision: task.revision,
                },
                progress: Some(task_data),
                message: None,
            })
            .await
            .unwrap_or(task);

        Ok(PreparedTask {
            taskmgr_override: self.taskmgr.clone(),
            taskmgr_context,
            task,
        })
    }
}

fn task_manager_rpc_context(invoke_ctx: &InvokeCtx) -> RPCContext {
    RPCContext {
        token: invoke_ctx.session_token.clone(),
        trace_id: invoke_ctx.trace_id.clone(),
        ..Default::default()
    }
}

async fn acquire_task_manager_client(
    taskmgr_override: Option<&Arc<TaskManagerClient>>,
    context: &RPCContext,
) -> std::result::Result<Arc<TaskManagerClient>, RPCErrors> {
    let taskmgr = if let Some(taskmgr) = taskmgr_override {
        taskmgr.clone()
    } else {
        let runtime = get_buckyos_api_runtime().map_err(|err| {
            reason_error(
                "task_manager_unavailable",
                format!("load runtime failed: {err}"),
            )
        })?;
        Arc::new(runtime.get_task_mgr_client().await.map_err(|err| {
            reason_error(
                "task_manager_unavailable",
                format!("get task manager client failed: {err}"),
            )
        })?)
    };
    taskmgr.set_context(context.clone()).await;
    Ok(taskmgr)
}

/// Close one of aicc's own (creator == runner) tasks as Canceled: record the
/// control request, then acknowledge it as the runner. Terminal races count
/// as success.
async fn cancel_own_task(
    taskmgr: &TaskManagerClient,
    task: &buckyos_api::Task,
) -> std::result::Result<(), RPCErrors> {
    let request_id = format!("aicc-cancel-{}", task.task_id);
    let requested = taskmgr
        .request_control(buckyos_api::RequestControlReq {
            task_id: task.task_id.clone(),
            action: TaskControlAction::Cancel,
            request_id: request_id.clone(),
            recursive: false,
            expected_revision: None,
        })
        .await;
    let task = match requested {
        Ok(buckyos_api::RequestControlResult::Task { task }) => task,
        Ok(_) => return Ok(()),
        Err(err) => {
            return match task_mgr_error_code(&err) {
                Some(buckyos_api::TASK_ERR_ALREADY_COMPLETED) => Ok(()),
                _ => Err(err),
            }
        }
    };
    if task.phase.is_terminal() {
        return Ok(());
    }
    let _ = taskmgr
        .ack_control(AckControlReq {
            envelope: RunnerWriteEnvelope {
                task_id: task.task_id.clone(),
                app_instance_id: None,
                runner_epoch: task.runner_epoch,
                expected_revision: task.revision,
            },
            request_id,
            applied: true,
            reject_reason: None,
        })
        .await;
    Ok(())
}

#[async_trait]
impl TaskEventSink for TaskAuditSink {
    fn event_ref(&self) -> Option<String> {
        None
    }

    async fn emit(&self, event: TaskEvent) -> std::result::Result<(), RPCErrors> {
        let _guard = self.lock.lock().await;
        let taskmgr =
            acquire_task_manager_client(self.taskmgr_override.as_ref(), &self.taskmgr_context)
                .await?;

        let task = taskmgr.get_task(&self.task_mgr_id).await?;
        if task.phase.is_terminal() {
            // Late provider events after the one-shot result are audit noise.
            return Ok(());
        }
        let mut data = task.progress.clone().unwrap_or_else(|| json!({}));
        merge_task_data_with_event(&mut data, &event);
        let envelope = RunnerWriteEnvelope {
            task_id: task.task_id.clone(),
            app_instance_id: None,
            runner_epoch: task.runner_epoch,
            expected_revision: task.revision,
        };
        let event_message = event
            .data
            .as_ref()
            .and_then(|value| value.get("message"))
            .and_then(|value| value.as_str())
            .map(|value| value.to_string());

        match event.kind {
            TaskEventKind::Queued => {
                let message = event_message.unwrap_or_else(|| QUEUE_STATUS_QUEUED.to_string());
                taskmgr
                    .report_progress(ReportProgressReq {
                        envelope,
                        progress: Some(data),
                        message: Some(message),
                    })
                    .await?;
            }
            TaskEventKind::Started => {
                let task = if task.phase == TaskPhase::Accepted {
                    taskmgr
                        .report_started(ReportStartedReq {
                            envelope: envelope.clone(),
                        })
                        .await?
                } else {
                    task
                };
                let message = event_message.unwrap_or_else(|| "aicc provider started".to_string());
                taskmgr
                    .report_progress(ReportProgressReq {
                        envelope: RunnerWriteEnvelope {
                            expected_revision: task.revision,
                            ..envelope
                        },
                        progress: Some(data),
                        message: Some(message),
                    })
                    .await?;
            }
            TaskEventKind::Final => {
                // The merged event history (incl. the provider result) is the
                // one-shot Result payload.
                taskmgr
                    .commit_result(CommitResultReq {
                        task_id: task.task_id.clone(),
                        result: data,
                        app_instance_id: None,
                        runner_epoch: Some(task.runner_epoch),
                        expected_revision: task.revision,
                    })
                    .await?;
                self.record_video_continuation_source(&event).await;
            }
            TaskEventKind::Error => {
                let message = event_message.unwrap_or_else(|| "aicc task failed".to_string());
                taskmgr
                    .fail_task(FailTaskReq {
                        envelope,
                        error: TaskError {
                            code: "aicc_error".to_string(),
                            message,
                            detail: Some(data),
                        },
                    })
                    .await?;
            }
            TaskEventKind::CancelRequested => {
                cancel_own_task(&taskmgr, &task).await?;
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

#[derive(Default)]
pub struct NamedStoreResourceResolver;

#[async_trait]
impl ResourceResolver for NamedStoreResourceResolver {
    async fn resolve(
        &self,
        _ctx: &InvokeCtx,
        req: &AiMethodRequest,
    ) -> std::result::Result<ResolvedRequest, RPCErrors> {
        let mut resolved = req.clone();
        for resource in resolved.payload.resources.iter_mut() {
            self.resolve_resource_ref(resource).await?;
        }
        for message in resolved.payload.messages.iter_mut() {
            for content in message.content.iter_mut() {
                match content {
                    AiContent::Image { source } | AiContent::Document { source, .. } => {
                        self.resolve_resource_ref(source).await?;
                    }
                    _ => {}
                }
            }
        }
        if let Some(input_json) = resolved.payload.input_json.as_mut() {
            self.resolve_resource_refs_in_value(input_json).await?;
        }
        Ok(ResolvedRequest::new(resolved))
    }
}

impl NamedStoreResourceResolver {
    async fn resolve_resource_ref(
        &self,
        resource: &mut ResourceRef,
    ) -> std::result::Result<(), RPCErrors> {
        let ResourceRef::NamedObject { obj_id } = resource else {
            return Ok(());
        };
        let (mime, bytes) = load_named_object_resource(obj_id).await?;
        *resource = ResourceRef::Base64 {
            mime,
            data_base64: general_purpose::STANDARD.encode(bytes),
        };
        Ok(())
    }

    fn resolve_resource_refs_in_value<'a>(
        &'a self,
        value: &'a mut Value,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<(), RPCErrors>> + Send + 'a>> {
        Box::pin(async move {
            match value {
                Value::Object(object) => {
                    if object
                        .get("kind")
                        .and_then(|value| value.as_str())
                        .is_some_and(|kind| kind == "named_object")
                    {
                        let obj_id = object
                            .get("obj_id")
                            .and_then(|value| value.as_str())
                            .ok_or_else(|| {
                                reason_error("resource_invalid", "named_object obj_id is missing")
                            })?;
                        let obj_id = ObjId::new(obj_id).map_err(|error| {
                            reason_error(
                                "resource_invalid",
                                format!("named_object obj_id is invalid: {}", error),
                            )
                        })?;
                        let mut resource = ResourceRef::NamedObject { obj_id };
                        self.resolve_resource_ref(&mut resource).await?;
                        *value = serde_json::to_value(resource).map_err(|error| {
                            reason_error(
                                "resource_invalid",
                                format!("serialize resolved resource failed: {}", error),
                            )
                        })?;
                        return Ok(());
                    }
                    for child in object.values_mut() {
                        self.resolve_resource_refs_in_value(child).await?;
                    }
                }
                Value::Array(items) => {
                    for child in items {
                        self.resolve_resource_refs_in_value(child).await?;
                    }
                }
                _ => {}
            }
            Ok(())
        })
    }
}

async fn load_named_object_resource(
    obj_id: &ObjId,
) -> std::result::Result<(String, Vec<u8>), RPCErrors> {
    let runtime = get_buckyos_api_runtime().map_err(|error| {
        reason_error(
            "resource_invalid",
            format!("get buckyos runtime failed: {}", error),
        )
    })?;
    let named_store = runtime.get_named_store().await.map_err(|error| {
        reason_error(
            "resource_invalid",
            format!("get named_store failed: {}", error),
        )
    })?;

    if obj_id.is_chunk() {
        let chunk_id = ChunkId::from_obj_id(obj_id);
        let bytes = named_store
            .get_chunk_data(&chunk_id)
            .await
            .map_err(|error| {
                reason_error(
                    "resource_invalid",
                    format!("read named_object chunk {} failed: {}", obj_id, error),
                )
            })?;
        return Ok((infer_mime_from_bytes(&bytes), bytes));
    }

    let object_str = named_store.get_object(obj_id).await.map_err(|error| {
        reason_error(
            "resource_invalid",
            format!("read named_object {} failed: {}", obj_id, error),
        )
    })?;
    let object_json = serde_json::from_str::<Value>(object_str.as_str())
        .or_else(|_| load_named_object_from_obj_str(object_str.as_str()))
        .map_err(|error| {
            reason_error(
                "resource_invalid",
                format!("parse named_object {} failed: {}", obj_id, error),
            )
        })?;
    let file_object = serde_json::from_value::<FileObject>(object_json.clone()).ok();
    let chunk_ids = runtime
        .get_chunklist_from_known_named_object(obj_id, &object_json)
        .await
        .or_else(|_| {
            file_object
                .as_ref()
                .and_then(|file| ObjId::new(file.content.as_str()).ok())
                .filter(|content_id| content_id.is_chunk())
                .map(|content_id| vec![ChunkId::from_obj_id(&content_id)])
                .ok_or_else(|| {
                    reason_error(
                        "resource_invalid",
                        format!(
                            "named_object {} does not point to readable chunk data",
                            obj_id
                        ),
                    )
                })
        })?;

    let mut bytes = Vec::new();
    for chunk_id in chunk_ids {
        let mut chunk_bytes = named_store
            .get_chunk_data(&chunk_id)
            .await
            .map_err(|error| {
                reason_error(
                    "resource_invalid",
                    format!(
                        "read named_object {} chunk {} failed: {}",
                        obj_id,
                        chunk_id.to_string(),
                        error
                    ),
                )
            })?;
        bytes.append(&mut chunk_bytes);
    }

    let mime = file_object
        .as_ref()
        .and_then(|file| file_object_mime(file))
        .unwrap_or_else(|| infer_mime_from_bytes(&bytes));
    Ok((mime, bytes))
}

fn file_object_mime(file: &FileObject) -> Option<String> {
    for key in ["mime_type", "media_type", "content_type"] {
        if let Some(mime) = file
            .meta
            .get(key)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(mime.to_string());
        }
    }
    infer_mime_from_name(file.name.as_str())
}

fn video_content_id_from_resolved_request(
    request: &AiMethodRequest,
) -> std::result::Result<Option<String>, RPCErrors> {
    for resource in &request.payload.resources {
        let ResourceRef::Base64 { mime, data_base64 } = resource else {
            continue;
        };
        if !mime.starts_with("video/") {
            continue;
        }
        let encoded = data_base64
            .split_once(',')
            .filter(|(prefix, _)| prefix.trim_start().starts_with("data:"))
            .map(|(_, data)| data)
            .unwrap_or(data_base64);
        let bytes = general_purpose::STANDARD.decode(encoded).map_err(|error| {
            reason_error(
                "resource_invalid",
                format!("decode video resource failed: {}", error),
            )
        })?;
        let content_id = ChunkHasher::new(None)
            .map_err(|error| {
                reason_error(
                    "resource_invalid",
                    format!("create video content hasher failed: {}", error),
                )
            })?
            .calc_mix_chunk_id_from_bytes(&bytes)
            .map_err(|error| {
                reason_error(
                    "resource_invalid",
                    format!("calculate video content id failed: {}", error),
                )
            })?;
        return Ok(Some(content_id.to_string()));
    }
    Ok(None)
}

fn request_has_continuation_handle(request: &AiMethodRequest) -> bool {
    request
        .payload
        .input_json
        .as_ref()
        .and_then(|input| input.get("continuation_handle"))
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
}

fn insert_continuation_handle(request: &mut AiMethodRequest, handle: String) {
    let input = request
        .payload
        .input_json
        .get_or_insert_with(|| Value::Object(Map::new()));
    if !input.is_object() {
        *input = json!({ "value": input.clone() });
    }
    input
        .as_object_mut()
        .expect("video extension input should be an object")
        .insert("continuation_handle".to_string(), Value::String(handle));
}

fn continuation_from_source_task(
    task: &buckyos_api::Task,
    tenant_id: &str,
) -> Option<(String, String)> {
    let task_data = parse_aicc_task_data(task.result.as_ref()?);
    continuation_from_task_data(&task_data, tenant_id)
}

fn continuation_from_task_data(
    task_data: &AiccComputeTaskData,
    tenant_id: &str,
) -> Option<(String, String)> {
    if task_data.request.tenant_id.as_deref() != Some(tenant_id) {
        return None;
    }
    let output = task_data.result.as_ref()?.output.as_ref()?;
    let handle = output
        .pointer("/extra/continuation_handle")?
        .as_str()?
        .trim();
    if handle.is_empty() {
        return None;
    }
    let exact_model = output
        .pointer("/extra/provider_audit/aicc_exact_model")
        .and_then(Value::as_str)
        .or_else(|| {
            task_data
                .request
                .route
                .as_ref()
                .and_then(|route| route.get("selected_exact_model"))
                .and_then(Value::as_str)
        })?
        .trim();
    if exact_model.is_empty() {
        return None;
    }
    Some((handle.to_string(), exact_model.to_string()))
}

fn infer_mime_from_name(name: &str) -> Option<String> {
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    match ext.as_str() {
        "png" => Some("image/png".to_string()),
        "jpg" | "jpeg" => Some("image/jpeg".to_string()),
        "webp" => Some("image/webp".to_string()),
        "gif" => Some("image/gif".to_string()),
        "mp3" => Some("audio/mpeg".to_string()),
        "wav" => Some("audio/wav".to_string()),
        "ogg" => Some("audio/ogg".to_string()),
        "mp4" => Some("video/mp4".to_string()),
        "json" => Some("application/json".to_string()),
        "txt" => Some("text/plain".to_string()),
        _ => None,
    }
}

fn infer_mime_from_bytes(bytes: &[u8]) -> String {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return "image/png".to_string();
    }
    if bytes.starts_with(b"\xff\xd8\xff") {
        return "image/jpeg".to_string();
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return "image/webp".to_string();
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return "image/gif".to_string();
    }
    if bytes.starts_with(b"ID3") || bytes.starts_with(b"\xff\xfb") {
        return "audio/mpeg".to_string();
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        return "audio/wav".to_string();
    }
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        return "video/mp4".to_string();
    }
    "application/octet-stream".to_string()
}

#[async_trait]
pub trait Provider: Send + Sync {
    fn inventory(&self) -> ProviderInventory;
    fn legacy_instance(&self) -> Option<&ProviderInstance> {
        None
    }
    fn shutdown(&self) {}
    fn estimate_cost(&self, input: &CostEstimateInput) -> CostEstimateOutput;
    async fn refresh_inventory(&self) -> std::result::Result<ProviderInventory, ProviderError> {
        Ok(self.inventory())
    }
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
        let replaced = {
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
            )
        };
        if let Some(entry) = replaced {
            entry.provider.shutdown();
        }
        inventory
    }

    pub fn remove_instance(&self, provider_instance_name: &str) {
        let removed = {
            let mut entries = self
                .entries
                .write()
                .expect("registry lock should be available");
            entries.remove(provider_instance_name)
        };
        if let Some(entry) = removed {
            entry.provider.shutdown();
        }
    }

    pub fn clear(&self) {
        let removed = {
            let mut entries = self
                .entries
                .write()
                .expect("registry lock should be available");
            std::mem::take(&mut *entries)
        };
        for entry in removed.values() {
            entry.provider.shutdown();
        }
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

    fn providers(&self) -> Vec<(String, Arc<dyn Provider>)> {
        self.entries
            .read()
            .map(|entries| {
                entries
                    .iter()
                    .map(|(name, entry)| (name.clone(), entry.provider.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn is_current_provider(&self, instance_id: &str, provider: &Arc<dyn Provider>) -> bool {
        self.entries
            .read()
            .ok()
            .and_then(|entries| {
                entries
                    .get(instance_id)
                    .map(|entry| Arc::ptr_eq(&entry.provider, provider))
            })
            .unwrap_or(false)
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
    provider_options: Option<Value>,
    exact_model: String,
    pricing_snapshot: Option<RoutePricingSnapshot>,
}

#[derive(Clone, Debug)]
pub struct RouteDecision {
    pub primary_instance_id: String,
    pub fallback_instance_ids: Vec<String>,
    pub provider_model: String,
    enabled_capabilities: Vec<Feature>,
    disabled_capabilities: Vec<Feature>,
    attempts: Vec<RouteAttempt>,
    route_trace: Arc<Mutex<RouteTrace>>,
    runtime_failover_enabled: bool,
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
        let request_policy = route_policy_from_request(req);
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
            if request_policy.local_only
                && candidate.inventory.provider_type != ProviderType::LocalInference
            {
                continue;
            }
            let estimate = provider.estimate_cost(&CostEstimateInput {
                api_type: api_type_for_capability(&req.capability).unwrap_or(ApiType::Llm),
                exact_model: exact_model_name(provider_model.as_str(), instance_id),
                input_tokens,
                estimated_output_tokens: Some(output_tokens),
                cached_input_tokens: None,
                request_features: req.requirements.effective_feature_names(),
            });
            let compat_estimate = CostEstimate::from(&estimate);
            let effective_estimated_cost = compat_estimate.estimated_cost_usd.and_then(|cost| {
                sn_ai_provider_billing
                    .and_then(|billing| billing.preview_billed_cost(tenant_id, provider_type, cost))
                    .or(Some(cost))
            });
            if let Some(max_cost) = request_policy.max_estimated_cost_usd {
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

            if let Some(max_latency_ms) = request_policy.max_latency_ms {
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
                provider_options: None,
                pricing_snapshot: None,
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
            enabled_capabilities: Vec::new(),
            disabled_capabilities: Vec::new(),
            attempts: final_attempts,
            route_trace: Arc::new(Mutex::new(legacy_route_trace(
                req.model.alias.clone(),
                api_type_for_capability(&req.capability).unwrap_or(ApiType::Llm),
            ))),
            runtime_failover_enabled: true,
        })
    }
}

fn legacy_route_trace(model: String, api_type: ApiType) -> RouteTrace {
    RouteTrace {
        request_id: String::new(),
        api_type,
        requested_model: model,
        requested_model_type: crate::model_types::RequestedModelType::Logical,
        resolved_logical_path: None,
        selected_exact_model: None,
        selected_provider_instance_name: None,
        selected_provider_model_id: None,
        provider_options: None,
        pricing_snapshot: None,
        candidate_count_before_filter: 0,
        candidate_count_after_filter: 0,
        filtered_candidates: Vec::new(),
        ranked_candidates: Vec::new(),
        fallback_applied: false,
        fallback_chain: Vec::new(),
        logical_item_sources: Vec::new(),
        logical_admission: Vec::new(),
        disabled_capability_sources: Vec::new(),
        session_overlays: Vec::new(),
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
        ai_methods::LLM_CHAT => Some(Capability::Llm),
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
        ai_methods::LLM_CHAT => Some(ApiType::Llm),
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

fn method_for_route_api_type(api_type: &str) -> std::result::Result<&'static str, RPCErrors> {
    match api_type {
        "llm" | "chat.completions.create" => Ok(ai_methods::LLM_CHAT),
        "image.txt2img" | "images.generate" => Ok(ai_methods::IMAGE_TXT2IMG),
        "image.img2img" | "images.edit" => Ok(ai_methods::IMAGE_IMG2IMG),
        "image.inpaint" => Ok(ai_methods::IMAGE_INPAINT),
        "image.upscale" => Ok(ai_methods::IMAGE_UPSCALE),
        "embedding.text" | "embeddings.create" => Ok(ai_methods::EMBEDDING_TEXT),
        "rerank" => Ok(ai_methods::RERANK),
        "audio.asr" | "audio.transcriptions.create" => Ok(ai_methods::AUDIO_ASR),
        "audio.tts" | "audio.speech.create" => Ok(ai_methods::AUDIO_TTS),
        "video.txt2video" | "videos.generate" => Ok(ai_methods::VIDEO_TXT2VIDEO),
        "video.img2video" => Ok(ai_methods::VIDEO_IMG2VIDEO),
        other => Err(reason_error(
            "invalid_request",
            format!("unsupported api_type '{}'", other),
        )),
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
    let request_policy = request.policy.as_ref();
    let mut policy = RoutePolicy {
        required_features: required_model_features(&request.requirements),
        max_estimated_cost_usd: request_policy
            .and_then(|policy| policy.max_cost_usd)
            .or(request.requirements.max_cost_usd),
        max_latency_ms: request_policy
            .and_then(|policy| policy.max_latency_ms)
            .or(request.requirements.max_latency_ms),
        ..Default::default()
    };
    if let Some(request_policy) = request_policy {
        policy.profile = match request_policy.profile {
            buckyos_api::RoutePolicyProfile::Cheap => {
                crate::model_types::SchedulerProfile::CostFirst
            }
            buckyos_api::RoutePolicyProfile::Fast => {
                crate::model_types::SchedulerProfile::LatencyFirst
            }
            buckyos_api::RoutePolicyProfile::Balanced => {
                crate::model_types::SchedulerProfile::Balanced
            }
            buckyos_api::RoutePolicyProfile::Quality => {
                crate::model_types::SchedulerProfile::QualityFirst
            }
        };
        policy.local_only = request_policy.local_only;
        policy.allow_fallback = request_policy.allow_fallback;
        policy.runtime_failover = request_policy.runtime_failover;
        policy.explain = request_policy.explain;
        policy.allowed_provider_instances = request_policy.allowed_provider_instances.clone();
        policy.blocked_provider_instances = request_policy.blocked_provider_instances.clone();
    }
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
        if let Some(min_context_tokens) = extra
            .get("min_context_tokens")
            .or_else(|| extra.get("min_context_window_tokens"))
            .and_then(Value::as_u64)
        {
            policy.required_features.min_context_tokens = Some(min_context_tokens);
        }
    }
    policy
}

fn route_probe_payload(estimated_input_tokens: Option<u64>) -> AiPayload {
    let text = estimated_input_tokens
        .map(|tokens| "x".repeat(tokens.saturating_mul(4).min(32 * 1024) as usize));
    AiPayload::new(
        text,
        vec![],
        vec![],
        vec![],
        Some(json!({})),
        Some(json!({})),
    )
}

fn provider_call_from_metadata(metadata: &ModelMetadata) -> (String, Option<Value>) {
    (
        metadata
            .provider_actual_model_id
            .clone()
            .unwrap_or_else(|| metadata.provider_model_id.clone()),
        metadata.provider_options.clone(),
    )
}

fn provider_call_from_candidate(candidate: &ModelCandidate) -> (String, Option<Value>) {
    provider_call_from_metadata(&candidate.metadata)
}

fn enabled_capabilities(capabilities: &ModelCapabilities, disabled: &[Feature]) -> Vec<Feature> {
    let mut enabled = Vec::new();
    let mut push_if_enabled = |feature: &str, value: bool| {
        if value && !disabled.iter().any(|item| item == feature) {
            enabled.push(feature.to_string());
        }
    };
    push_if_enabled("streaming", capabilities.streaming);
    push_if_enabled(buckyos_api::features::TOOL_CALLING, capabilities.tool_call);
    push_if_enabled(buckyos_api::features::JSON_OUTPUT, capabilities.json_schema);
    push_if_enabled(buckyos_api::features::WEB_SEARCH, capabilities.web_search);
    push_if_enabled(buckyos_api::features::VISION, capabilities.vision);
    if let Some(tokens) = capabilities.max_context_tokens {
        enabled.push(format!("max_context_tokens:{}", tokens));
    }
    if let Some(tokens) = capabilities.max_output_tokens {
        enabled.push(format!("max_output_tokens:{}", tokens));
    }
    enabled
}

fn merge_provider_options_values(base: Option<Value>, overlay: Option<Value>) -> Option<Value> {
    match (base, overlay) {
        (None, None) => None,
        (Some(value), None) | (None, Some(value)) => Some(value),
        (Some(mut base), Some(overlay)) => {
            merge_json_value(&mut base, overlay);
            Some(base)
        }
    }
}

fn merge_json_value(base: &mut Value, overlay: Value) {
    match (base, overlay) {
        (Value::Object(base_map), Value::Object(overlay_map)) => {
            for (key, value) in overlay_map {
                if let Some(base_value) = base_map.get_mut(&key) {
                    merge_json_value(base_value, value);
                } else {
                    base_map.insert(key, value);
                }
            }
        }
        (base_value, overlay_value) => {
            *base_value = overlay_value;
        }
    }
}

fn merge_provider_options(payload: &mut AiPayload, provider_options: Option<Value>) {
    let Some(provider_options) = provider_options else {
        return;
    };
    let mut options = payload.options.take().unwrap_or_else(|| json!({}));
    if !options.is_object() {
        options = json!({});
    }
    if let Some(object) = options.as_object_mut() {
        let existing = object.remove("provider_options");
        if let Some(merged) = merge_provider_options_values(existing, Some(provider_options)) {
            object.insert("provider_options".to_string(), merged);
        }
    }
    payload.options = Some(options);
}

fn payload_provider_options(payload: &AiPayload) -> Option<Value> {
    payload
        .options
        .as_ref()
        .and_then(|options| options.get("provider_options"))
        .cloned()
}

fn helper_provider_options(route_options: Option<Value>, payload: &AiPayload) -> Option<Value> {
    merge_provider_options_values(route_options, payload_provider_options(payload))
}

fn exact_model_spec(exact_model: &str) -> std::result::Result<ModelSpec, RPCErrors> {
    ExactModelName::parse(exact_model).map_err(route_error_to_rpc)?;
    Ok(ModelSpec::new(exact_model.to_string(), None))
}

fn route_request_from_method_request(
    method: &str,
    request: &AiMethodRequest,
) -> std::result::Result<RouteResolveRequest, RPCErrors> {
    let api_type = api_type_for_method(method).ok_or_else(|| {
        reason_error(
            "invalid_method",
            format!("method '{}' is not supported by route.resolve", method),
        )
    })?;
    let (estimated_input_tokens, estimated_output_tokens) = estimate_request_tokens(request);
    let session_overlay = request_control_value(request, "session_overlay")
        .map(|value| {
            serde_json::from_value::<AiccRouteOverlay>(value.clone()).map_err(|error| {
                reason_error(
                    "invalid_request",
                    format!("session route overlay is invalid: {}", error),
                )
            })
        })
        .transpose()?;
    Ok(RouteResolveRequest {
        request_id: None,
        api_type: api_type_string(&api_type).to_string(),
        logical_model: request.model.alias.clone(),
        requirements: request.requirements.clone(),
        disable: request.disable.clone(),
        policy: request.policy.clone(),
        estimated_input_tokens: Some(estimated_input_tokens),
        estimated_output_tokens: Some(estimated_output_tokens),
        session_overlay,
    })
}

fn api_type_string(api_type: &ApiType) -> &'static str {
    match api_type {
        ApiType::Llm => "llm",
        ApiType::ImageTextToImage => "image.txt2img",
        ApiType::ImageToImage => "image.img2img",
        ApiType::Embedding => "embedding.text",
        ApiType::EmbeddingMultimodal => "embedding.multimodal",
        ApiType::Rerank => "rerank",
        ApiType::ImageInpaint => "image.inpaint",
        ApiType::ImageUpscale => "image.upscale",
        ApiType::ImageBgRemove => "image.bg_remove",
        ApiType::VisionOcr => "vision.ocr",
        ApiType::VisionCaption => "vision.caption",
        ApiType::VisionDetect => "vision.detect",
        ApiType::VisionSegment => "vision.segment",
        ApiType::AudioTts => "audio.tts",
        ApiType::AudioAsr => "audio.asr",
        ApiType::AudioMusic => "audio.music",
        ApiType::AudioEnhance => "audio.enhance",
        ApiType::VideoTextToVideo => "video.txt2video",
        ApiType::VideoImageToVideo => "video.img2video",
        ApiType::VideoToVideo => "video.video2video",
        ApiType::VideoExtend => "video.extend",
        ApiType::VideoUpscale => "video.upscale",
        ApiType::AgentComputerUse => "agent.computer_use",
    }
}

fn ai_method_response_from_llm_chat(response: LlmChatInvokeResponse) -> AiMethodResponse {
    let result = response.message.map(|message| AiResponse {
        message,
        usage: response.usage,
        cost: response.cost,
        finish_reason: response.finish_reason,
        provider_task_ref: response.provider_task_ref,
        extra: response
            .route_trace
            .map(|trace| json!({ "route_trace": trace })),
    });
    AiMethodResponse::new(
        response.task_id,
        response.status,
        result,
        response.event_ref,
    )
}

fn ai_method_response_from_text_to_image(response: TextToImageInvokeResponse) -> AiMethodResponse {
    let result = (!response.artifacts.is_empty()).then(|| AiResponse {
        message: AiResponse::message_from_parts(None, Vec::new(), response.artifacts),
        usage: response.usage,
        cost: response.cost,
        finish_reason: None,
        provider_task_ref: response.provider_task_ref,
        extra: response
            .route_trace
            .map(|trace| json!({ "route_trace": trace })),
    });
    AiMethodResponse::new(
        response.task_id,
        response.status,
        result,
        response.event_ref,
    )
}

fn append_provider_audit_to_summary(summary: &mut AiResponse, attempt: &RouteAttempt) {
    let extra_value = summary
        .extra
        .get_or_insert_with(|| Value::Object(Map::new()));
    if !extra_value.is_object() {
        *extra_value = Value::Object(Map::new());
    }
    if let Value::Object(extra) = extra_value {
        extra.insert(
            "provider_audit".to_string(),
            json!({
                "aicc_exact_model": attempt.exact_model,
                "provider_actual_model": attempt.provider_model,
                "provider_options": attempt.provider_options,
            }),
        );
    }
}

fn ensure_summary_accounting(summary: &mut AiResponse, attempt: &RouteAttempt) {
    if summary.usage.is_none() {
        summary.usage = Some(buckyos_api::AiUsage::request_units(1));
    }
    if summary.cost.is_none() {
        if let Some(pricing) = attempt.pricing_snapshot.as_ref() {
            if let Some(amount) = pricing.estimated_cost {
                summary.cost = Some(buckyos_api::AiCost {
                    amount,
                    currency: pricing.currency.clone(),
                });
            }
        }
    }
}

fn llm_chat_invoke_to_method_request(
    request: LlmChatInvokeRequest,
) -> std::result::Result<AiMethodRequest, RPCErrors> {
    let model = exact_model_spec(request.exact_model.as_str())?;
    let mut payload = request.payload.unwrap_or_else(|| {
        AiPayload::new(
            None,
            request.messages.clone(),
            request.tools.clone(),
            vec![],
            Some(json!({})),
            Some(json!({})),
        )
    });
    if payload.messages.is_empty() {
        payload.messages = request.messages;
    }
    if payload.tool_specs.is_empty() {
        payload.tool_specs = request.tools;
    }
    let input_json = payload.input_json.get_or_insert_with(|| json!({}));
    if !input_json.is_object() {
        *input_json = json!({ "value": input_json.clone() });
    }
    if let Some(object) = input_json.as_object_mut() {
        if let Some(resp_format) = request.response_format {
            object.insert("response_format".to_string(), json!(resp_format));
        }
        if let Some(temperature) = request.temperature {
            object.insert("temperature".to_string(), json!(temperature));
        }
        if let Some(max_output_tokens) = request.max_output_tokens {
            object.insert("max_output_tokens".to_string(), json!(max_output_tokens));
        }
    }
    merge_provider_options(&mut payload, request.provider_options);
    let mut policy = buckyos_api::RoutePolicy::default();
    policy.allow_fallback = false;
    policy.runtime_failover = false;
    Ok(AiMethodRequest::new(
        Capability::Llm,
        model,
        Requirements::default(),
        payload,
        request.idempotency_key,
    )
    .with_policy(Some(policy))
    .with_task_options(request.task_options))
}

fn image_generate_to_method_request(
    request: TextToImageInvokeRequest,
) -> std::result::Result<AiMethodRequest, RPCErrors> {
    let model = exact_model_spec(request.exact_model.as_str())?;
    let mut payload = request.payload.unwrap_or_else(|| {
        AiPayload::new(
            Some(request.prompt.clone()),
            vec![],
            vec![],
            vec![],
            Some(json!({
                "prompt": request.prompt.clone()
            })),
            Some(json!({})),
        )
    });
    if payload.text.is_none() {
        payload.text = Some(request.prompt.clone());
    }
    let mut input = payload.input_json.take().unwrap_or_else(|| json!({}));
    if !input.is_object() {
        input = json!({ "value": input });
    }
    if let Some(object) = input.as_object_mut() {
        object
            .entry("prompt".to_string())
            .or_insert_with(|| json!(request.prompt));
        if let Some(value) = request.negative_prompt {
            object.insert("negative_prompt".to_string(), json!(value));
        }
        if let Some(value) = request.size {
            object.insert("size".to_string(), json!(value));
        }
        if let Some(value) = request.quality {
            object.insert("quality".to_string(), json!(value));
        }
        if let Some(value) = request.style {
            object.insert("style".to_string(), json!(value));
        }
        if let Some(value) = request.seed {
            object.insert("seed".to_string(), json!(value));
        }
        if let Some(value) = request.output {
            object.insert("output".to_string(), value);
        }
    }
    payload.input_json = Some(input);
    merge_provider_options(&mut payload, request.provider_options);
    let mut policy = buckyos_api::RoutePolicy::default();
    policy.allow_fallback = false;
    policy.runtime_failover = false;
    Ok(AiMethodRequest::new(
        Capability::Image,
        model,
        Requirements::default(),
        payload,
        request.idempotency_key,
    )
    .with_policy(Some(policy))
    .with_task_options(request.task_options))
}

fn apply_default_features_for_method(method: &str, request: &mut AiMethodRequest) {
    apply_disabled_capabilities(request);
    if method == ai_methods::LLM_CHAT
        && !request_disables_capability(request, buckyos_api::features::WEB_SEARCH)
    {
        request
            .requirements
            .set_feature_required(buckyos_api::features::WEB_SEARCH);
    }
}

fn apply_disabled_capabilities(request: &mut AiMethodRequest) {
    let disabled = disabled_capabilities(request);
    if disabled.is_empty() {
        return;
    }
    request
        .requirements
        .must_features
        .retain(|feature| !disabled.iter().any(|item| item == feature));
    for feature in disabled {
        match feature.as_str() {
            buckyos_api::features::TOOL_CALLING => request.requirements.required.tool_call = false,
            buckyos_api::features::JSON_OUTPUT => request.requirements.required.json_schema = false,
            buckyos_api::features::WEB_SEARCH => request.requirements.required.web_search = false,
            buckyos_api::features::VISION => request.requirements.required.vision = false,
            "streaming" => request.requirements.required.streaming = false,
            _ => {}
        }
    }
}

fn request_disables_capability(request: &AiMethodRequest, feature: &str) -> bool {
    disabled_capabilities(request)
        .iter()
        .any(|item| item == feature)
}

fn disabled_capabilities(request: &AiMethodRequest) -> Vec<String> {
    let mut disabled = request.disable.feature_names();
    let legacy_disabled: Vec<String> = request
        .requirements
        .extra
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|extra| extra.get("disable_capabilities"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    for feature in legacy_disabled {
        if !disabled.iter().any(|item| item == &feature) {
            disabled.push(feature);
        }
    }
    disabled
}

fn merge_disabled_capabilities(mut base: Vec<String>, overlay: Vec<String>) -> Vec<String> {
    for capability in overlay {
        if !base.iter().any(|item| item == &capability) {
            base.push(capability);
        }
    }
    base
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

fn apply_logical_node_policy_override(
    policy: &mut RoutePolicy,
    session_config: &SessionConfig,
    model: &str,
) -> std::result::Result<(), crate::model_types::RouteError> {
    if crate::model_types::is_exact_model_name(model) {
        return Ok(());
    }
    let Some(node) = session_config.node(model) else {
        return Ok(());
    };
    if let Some(config) = node.policy.as_ref() {
        apply_policy_config_to_route_policy(policy, config);
    }
    if let Some(config) = node.route_policy_override.as_ref() {
        apply_policy_config_to_route_policy(policy, config);
    }
    Ok(())
}

fn disabled_capabilities_for_logical_path(
    session_config: &SessionConfig,
    registry: &ModelRegistry,
    logical_path: &str,
) -> Vec<String> {
    if let Some(disable_line) = session_config
        .node(logical_path)
        .and_then(|node| node.disable_line.as_ref())
    {
        return disable_line.feature_names();
    }
    registry
        .logical_definition(logical_path)
        .map(|definition| definition.disable_line.feature_names())
        .unwrap_or_default()
}

fn mark_selected_session_overlay(trace: &mut RouteTrace, selected: &ModelCandidate) {
    if trace.session_overlays.is_empty() {
        return;
    }
    for overlay in trace.session_overlays.iter_mut() {
        let prefix = format!("{} -> ", overlay.overlay_path);
        overlay.selected_from_overlay = selected
            .route_paths
            .iter()
            .any(|path| path.starts_with(prefix.as_str()));
    }
}

fn required_model_features(requirements: &Requirements) -> RequiredModelFeatures {
    let mut required = RequiredModelFeatures::default();
    required.streaming = requirements.required.streaming;
    required.tool_call = requirements.required.tool_call;
    required.json_schema = requirements.required.json_schema;
    required.web_search = requirements.required.web_search;
    required.vision = requirements.required.vision;
    required.min_context_tokens = requirements.required.min_context_tokens;
    for feature in &requirements.must_features {
        match feature.as_str() {
            buckyos_api::features::TOOL_CALLING => required.tool_call = true,
            buckyos_api::features::JSON_OUTPUT => required.json_schema = true,
            buckyos_api::features::WEB_SEARCH => required.web_search = true,
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
        text_len = text_len.saturating_add(message.estimate_text_len());
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

fn load_local_logical_tree_for_route(
) -> std::result::Result<crate::default_logical_tree::LocalLogicalTreeConfig, RouteError> {
    crate::default_logical_tree::load_or_create_local_logical_tree_config().map_err(|err| {
        RouteError::new(
            RouteErrorCode::SessionConfigInvalid,
            format!("load local logical tree config failed: {}", err),
        )
    })
}

fn default_global_session_config(
    local_config: &crate::default_logical_tree::LocalLogicalTreeConfig,
) -> SessionConfig {
    SessionConfig {
        logical_tree: local_config.logical_tree.clone(),
        revision: Some(local_config.revision.clone()),
        ..Default::default()
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
struct SystemRoutingConfig {
    #[serde(default)]
    logical_definitions: Vec<LogicalModelDefinition>,
    #[serde(flatten)]
    session_config: SessionConfig,
}

fn parse_system_routing_config(
    settings: &Value,
) -> std::result::Result<SystemRoutingConfig, RouteError> {
    let Some(value) = settings
        .get("routing_config")
        .or_else(|| settings.get("routing_settings"))
    else {
        return Ok(SystemRoutingConfig::default());
    };
    serde_json::from_value::<SystemRoutingConfig>(value.clone()).map_err(|err| {
        RouteError::new(
            RouteErrorCode::SessionConfigInvalid,
            format!("parse system routing config failed: {}", err),
        )
    })
}

fn merged_logical_definitions(
    defaults: Vec<LogicalModelDefinition>,
    overrides: &[LogicalModelDefinition],
) -> Vec<LogicalModelDefinition> {
    let mut merged = defaults
        .into_iter()
        .map(|definition| (definition.path.clone(), definition))
        .collect::<BTreeMap<_, _>>();
    for definition in overrides {
        merged.insert(definition.path.clone(), definition.clone());
    }
    merged.into_values().collect()
}

fn collect_session_nodes<'a>(
    parent_path: Option<&str>,
    nodes: &'a BTreeMap<String, LogicalNode>,
    out: &mut BTreeMap<String, &'a LogicalNode>,
) {
    for (name, node) in nodes {
        let path = match parent_path {
            Some(parent) => format!("{}.{}", parent, name),
            None => name.clone(),
        };
        out.insert(path.clone(), node);
        collect_session_nodes(Some(path.as_str()), &node.children, out);
    }
}

fn system_routing_settings_json(session_config: &SessionConfig) -> Value {
    json!({
        "global_exact_model_weights": session_config.global_exact_model_weights,
        "provider_weights": session_config.provider_weights,
        "policy": session_config.policy,
        "revision": session_config.revision,
    })
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

#[allow(dead_code)]
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

#[allow(dead_code)]
fn provider_driver_mount_segment(provider_driver: &str) -> String {
    let normalized = provider_driver
        .trim()
        .replace('_', "-")
        .to_ascii_lowercase();
    let stripped = normalized
        .strip_prefix("google-")
        .unwrap_or(normalized.as_str());
    logical_mount_segment(stripped)
}

#[allow(dead_code)]
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
    } else if lowered.contains("gemini") {
        mounts.push("image.txt2img.gemini".to_string());
    }
    mounts
}

#[allow(dead_code)]
pub fn provider_model_metadata(
    provider_instance_name: &str,
    provider_type: ProviderType,
    model_driver: &str,
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
        model_driver: model_driver.to_string(),
        origin_model_id: None,
        provider_actual_model_id: None,
        provider_options: None,
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
            web_search: features
                .iter()
                .any(|item| item == buckyos_api::features::WEB_SEARCH),
            unsupported_feature_combinations: vec![],
            vision: features
                .iter()
                .any(|item| item == buckyos_api::features::VISION),
            max_context_tokens: None,
            max_output_tokens: None,
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
            estimated_cost: estimated_cost_usd,
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
    task_mgr_id: String,
}

pub struct AIComputeCenter {
    registry: Registry,
    route_cfg: Arc<RwLock<RouteConfig>>,
    sn_ai_provider_billing: SnAIProviderBillingLedger,
    model_catalog: ModelCatalog,
    model_registry: Arc<RwLock<ModelRegistry>>,
    session_config: Arc<RwLock<SessionConfig>>,
    system_logical_definition_overrides: Arc<RwLock<Vec<LogicalModelDefinition>>>,
    inventory_refresh_scheduler: Arc<InventoryRefreshScheduler>,
    model_scheduler: ModelScheduler,
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
            "application/pdf",
            "application/msword",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "application/vnd.ms-excel",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "application/vnd.ms-powerpoint",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            "application/xml",
            "application/yaml",
            "application/rtf",
            "text/plain",
            "text/csv",
            "text/tab-separated-values",
            "text/markdown",
            "text/html",
            "text/xml",
            "text/yaml",
            "text/rtf",
        ]
        .into_iter()
        .map(|item| item.to_string())
        .collect::<HashSet<_>>();
        let url_scheme_allowlist = ["http", "https"]
            .into_iter()
            .map(|item| item.to_string())
            .collect::<HashSet<_>>();
        let local_logical_tree_config =
            crate::default_logical_tree::load_or_create_local_logical_tree_config().unwrap_or_else(
                |err| {
                    warn!(
                        "aicc.default_logical_tree.load_local_failed err={}, use builtin",
                        err
                    );
                    crate::default_logical_tree::build_builtin_local_logical_tree_config()
                },
            );
        let mut model_registry = ModelRegistry::new();
        let global_session_config = default_global_session_config(&local_logical_tree_config);
        if let Err(err) =
            model_registry.set_logical_definitions(local_logical_tree_config.logical_definitions)
        {
            warn!(
                "aicc.model_registry.set_default_logical_definitions_failed err={}",
                err
            );
        }
        for inventory in registry.inventories() {
            if let Err(err) = model_registry.apply_inventory(inventory) {
                warn!("aicc.model_registry.apply_inventory_failed err={}", err);
            }
        }

        let model_registry = Arc::new(RwLock::new(model_registry));
        let session_config = Arc::new(RwLock::new(global_session_config.clone()));
        let system_logical_definition_overrides = Arc::new(RwLock::new(Vec::new()));
        let model_registry_for_refresh = model_registry.clone();
        let logical_definition_overrides_for_refresh = system_logical_definition_overrides.clone();
        let inventory_registry = model_registry.clone();
        let inventory_source_registry = registry.clone();
        let inventory_refresh_scheduler = Arc::new(
            InventoryRefreshScheduler::new(
                inventory_registry,
                Arc::new(move || inventory_source_registry.inventories()),
                DEFAULT_INVENTORY_REFRESH_INTERVAL,
            )
            .with_refresh_hook(Arc::new(move || {
                let config = load_local_logical_tree_for_route()?;
                let overrides = logical_definition_overrides_for_refresh
                    .read()
                    .map_err(|_| {
                        RouteError::new(
                            RouteErrorCode::ProviderUnavailable,
                            "system logical definition override lock poisoned",
                        )
                    })?
                    .clone();
                let definitions =
                    merged_logical_definitions(config.logical_definitions, overrides.as_slice());
                if let Ok(mut registry) = model_registry_for_refresh.write() {
                    registry.set_logical_definitions(definitions)?;
                } else {
                    return Err(RouteError::new(
                        RouteErrorCode::ProviderUnavailable,
                        "model registry lock poisoned",
                    ));
                }
                Ok(())
            })),
        );
        if tokio::runtime::Handle::try_current().is_ok() {
            inventory_refresh_scheduler.start();
        }

        Self {
            registry,
            route_cfg: Arc::new(RwLock::new(RouteConfig::default())),
            sn_ai_provider_billing: SnAIProviderBillingLedger::default(),
            model_catalog,
            model_registry,
            session_config,
            system_logical_definition_overrides,
            inventory_refresh_scheduler,
            model_scheduler: ModelScheduler,
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

    fn record_route_trace(
        &self,
        decision: &RouteDecision,
        tenant_id: &str,
        caller_app_id: Option<String>,
    ) {
        let Some(db) = self.usage_log_db.clone() else {
            return;
        };
        let Ok(trace) = decision.route_trace.lock() else {
            return;
        };
        let Ok(trace) = serde_json::to_value(&*trace) else {
            return;
        };
        let trace_id = trace
            .get("request_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if trace_id.is_empty() {
            return;
        }
        let event = AiccRouteTraceEvent {
            trace_id: trace_id.clone(),
            tenant_id: tenant_id.to_string(),
            caller_app_id,
            task_id: trace_id,
            request_model: trace
                .get("requested_model")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            selected_exact_model: trace
                .get("selected_exact_model")
                .and_then(Value::as_str)
                .map(str::to_string),
            provider_instance_name: trace
                .get("selected_provider_instance_name")
                .and_then(Value::as_str)
                .map(str::to_string),
            api_type: trace
                .get("api_type")
                .and_then(Value::as_str)
                .unwrap_or("llm")
                .to_string(),
            route_trace_json: trace,
            created_at_ms: now_ms_i64(),
        };
        tokio::spawn(async move {
            if let Err(err) = db.insert_route_trace_event(&event).await {
                warn!(
                    "aicc.route_trace_log write_failed: trace_id={} tenant={} err={}",
                    event.trace_id, event.tenant_id, err
                );
            }
        });
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
    }

    pub fn apply_system_routing_config(
        &self,
        settings: &Value,
    ) -> std::result::Result<usize, RouteError> {
        let local_config = load_local_logical_tree_for_route()?;
        let base = default_global_session_config(&local_config);
        let system_config = parse_system_routing_config(settings)?;
        let definition_overrides = system_config.logical_definitions;
        let definitions = merged_logical_definitions(
            local_config.logical_definitions,
            definition_overrides.as_slice(),
        );
        let definition_count = definitions.len();

        {
            let mut registry = self.model_registry.write().map_err(|_| {
                RouteError::new(
                    RouteErrorCode::ProviderUnavailable,
                    "model registry lock poisoned",
                )
            })?;
            registry.set_logical_definitions(definitions)?;
        }
        {
            let mut overrides = self
                .system_logical_definition_overrides
                .write()
                .map_err(|_| {
                    RouteError::new(
                        RouteErrorCode::ProviderUnavailable,
                        "system logical definition override lock poisoned",
                    )
                })?;
            *overrides = definition_overrides;
        }

        let merged = merge_session_config(&base, &system_config.session_config)?;
        {
            let mut session_config = self.session_config.write().map_err(|_| {
                RouteError::new(
                    RouteErrorCode::ProviderUnavailable,
                    "session config lock poisoned",
                )
            })?;
            *session_config = merged;
        }

        Ok(definition_count)
    }

    pub fn apply_default_logical_tree(&self) -> std::result::Result<usize, RPCErrors> {
        let definition_count = self
            .apply_system_routing_config(&json!({}))
            .map_err(route_error_to_rpc)?;
        info!(
            "aicc.default_logical_tree.applied logical_definitions={}",
            definition_count
        );
        Ok(definition_count)
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
                            "provider_actual_model_id": model.provider_actual_model_id,
                            "provider_options": model.provider_options,
                            "model_driver": model.model_driver,
                            "api_types": model.api_types,
                            "logical_mounts": model.logical_mounts,
                            "capabilities": model.capabilities,
                            "attributes": model.attributes,
                            "pricing": model.pricing,
                            "health": model.health.status,
                            "quota": model.health.quota_state,
                        })
                    })
                    .collect();
                json!({
                    "provider_instance_name": inventory.provider_instance_name,
                    "provider_driver": inventory.provider_driver,
                    "provider_type": inventory.provider_type,
                    "provider_origin": inventory.provider_origin,
                    "provider_type_revision": inventory.provider_type_revision,
                    "version": inventory.version,
                    "inventory_revision": inventory.inventory_revision,
                    "models": models_json,
                })
            })
            .collect();

        let default_directory = registry.all_default_items();
        let session_config = self
            .session_config
            .read()
            .map_err(|_| reason_error("internal_error", "session config lock poisoned"))?;
        let mut session_nodes = BTreeMap::new();
        collect_session_nodes(None, &session_config.logical_tree, &mut session_nodes);
        let mut directory = default_directory.clone();
        for path in session_nodes.keys() {
            directory.entry(path.clone()).or_default();
        }
        let mut directory_json = Map::new();
        for (logical_path, default_items) in directory.iter() {
            let items = if let Some(node) = session_nodes.get(logical_path) {
                node.effective_items(Some(default_items))
                    .map_err(route_error_to_rpc)?
            } else {
                default_items.clone()
            };
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

        let mut logical_definitions_json = Vec::new();
        for definition in registry.logical_definitions() {
            logical_definitions_json.push(serde_json::to_value(definition).map_err(|err| {
                reason_error(
                    "internal_error",
                    format!("serialize logical definition failed: {}", err),
                )
            })?);
        }

        let aliases = self.model_catalog.snapshot();

        Ok(json!({
            "providers": providers_json,
            "directory": Value::Object(directory_json),
            "logical_definitions": logical_definitions_json,
            "routing_settings": system_routing_settings_json(&session_config),
            "aliases": aliases,
        }))
    }

    pub fn reload_local_logical_tree_config(&self) -> std::result::Result<(), RouteError> {
        let config = load_local_logical_tree_for_route()?;
        let overrides = self
            .system_logical_definition_overrides
            .read()
            .map_err(|_| {
                RouteError::new(
                    RouteErrorCode::ProviderUnavailable,
                    "system logical definition override lock poisoned",
                )
            })?
            .clone();
        let definitions =
            merged_logical_definitions(config.logical_definitions, overrides.as_slice());
        self.model_registry
            .write()
            .map_err(|_| {
                RouteError::new(
                    RouteErrorCode::ProviderUnavailable,
                    "model registry lock poisoned",
                )
            })?
            .set_logical_definitions(definitions)?;
        Ok(())
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

    pub async fn refresh_provider_inventory(
        &self,
        provider_instance_name: &str,
    ) -> std::result::Result<ProviderInventory, RPCErrors> {
        self.reload_local_logical_tree_config()
            .map_err(route_error_to_rpc)?;
        let provider = self
            .registry
            .get_provider(provider_instance_name)
            .ok_or_else(|| reason_error("provider_not_found", "provider not found"))?;
        let inventory = provider.refresh_inventory().await.map_err(|err| {
            reason_error(
                "provider_refresh_failed",
                format!("refresh provider inventory failed: {}", err),
            )
        })?;
        self.model_registry
            .write()
            .map_err(|_| reason_error("internal_error", "model registry lock poisoned"))?
            .apply_inventory(inventory.clone())
            .map_err(route_error_to_rpc)?;
        Ok(inventory)
    }

    pub async fn refresh_all_provider_inventories(&self) -> (usize, Vec<(String, String)>) {
        let mut refreshes = tokio::task::JoinSet::new();
        for (provider_instance_name, provider) in self.registry.providers() {
            refreshes.spawn(async move {
                let inventory = provider.refresh_inventory().await;
                (provider_instance_name, provider, inventory)
            });
        }
        let mut refreshed = 0usize;
        let mut errors = Vec::new();
        while let Some(refresh_result) = refreshes.join_next().await {
            let (provider_instance_name, provider, inventory) = match refresh_result {
                Ok(result) => result,
                Err(err) => {
                    errors.push(("<refresh-task>".to_string(), err.to_string()));
                    continue;
                }
            };
            let inventory = match inventory {
                Ok(inventory) => inventory,
                Err(err) => {
                    errors.push((provider_instance_name, err.to_string()));
                    continue;
                }
            };
            if !self
                .registry
                .is_current_provider(provider_instance_name.as_str(), &provider)
            {
                warn!(
                    "aicc.provider_inventory_refresh_discarded provider_instance_name={} reason=registration_changed",
                    provider_instance_name
                );
                continue;
            }
            let apply_result = self
                .model_registry
                .write()
                .map_err(|_| "model registry lock poisoned".to_string())
                .and_then(|mut registry| {
                    registry
                        .apply_inventory(inventory)
                        .map_err(|err| err.to_string())
                });
            match apply_result {
                Ok(()) => refreshed = refreshed.saturating_add(1),
                Err(err) => errors.push((provider_instance_name, err)),
            }
        }
        (refreshed, errors)
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

    /// Installs a fixed client override for in-process tests or an explicitly
    /// token-managed caller. Production uses the registered runtime per call.
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
        let effective_session_config = self
            .resolve_request_session_config(request)
            .map_err(route_error_to_rpc)?;
        let mut policy = route_policy_from_request(request);
        apply_policy_config_to_route_policy(&mut policy, &effective_session_config.config.policy);
        apply_logical_node_policy_override(
            &mut policy,
            &effective_session_config.config,
            request.model.alias.as_str(),
        )
        .map_err(route_error_to_rpc)?;
        if route_cfg.fallback_limit == 0 {
            policy.runtime_failover = false;
        }

        let registry = self
            .model_registry
            .read()
            .map_err(|_| reason_error("internal_error", "model registry lock poisoned"))?;
        let router = ModelRouter::new(&registry, &effective_session_config.config);
        let resolution = router.resolve(RouteRequest {
            request_id: request_id.to_string(),
            api_type: api_type.clone(),
            model: request.model.alias.clone(),
            policy: policy.clone(),
            session_overlay_trace: effective_session_config.overlay_trace.clone(),
        });

        let mut resolution = resolution.map_err(route_error_to_rpc)?;
        let definition_disabled_capabilities = resolution
            .trace
            .resolved_logical_path
            .as_deref()
            .map(|path| {
                disabled_capabilities_for_logical_path(
                    &effective_session_config.config,
                    &registry,
                    path,
                )
            })
            .unwrap_or_default();
        drop(registry);
        self.apply_dynamic_cost_estimates(tenant_id, request, &mut resolution.candidates);
        self.apply_dynamic_budget_filters(&mut resolution, &policy)
            .map_err(route_error_to_rpc)?;

        let scheduled = self
            .model_scheduler
            .schedule(&resolution.candidates, &policy)
            .ok_or_else(|| reason_error("no_provider_available", "no route candidate generated"))?;

        resolution.trace.selected_exact_model = Some(scheduled.selected.exact_model.clone());
        resolution.trace.selected_provider_instance_name =
            Some(scheduled.selected.provider_instance_name.clone());
        let (selected_provider_model, mut selected_provider_options) =
            provider_call_from_candidate(&scheduled.selected);
        resolution.trace.selected_provider_model_id = Some(selected_provider_model.clone());
        resolution.trace.provider_options = selected_provider_options.clone();
        resolution.trace.pricing_snapshot =
            RoutePricingSnapshot::from_candidate(&scheduled.selected);
        resolution.trace.ranked_candidates = scheduled.ranked_candidates;
        mark_selected_session_overlay(&mut resolution.trace, &scheduled.selected);
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
            .map(|provider_model| {
                selected_provider_options = None;
                resolution.trace.provider_options = None;
                resolution.trace.selected_provider_model_id = Some(provider_model.clone());
                provider_model
            })
            .unwrap_or(selected_provider_model);
        let mut attempts = vec![RouteAttempt {
            instance_id: scheduled.selected.provider_instance_name.clone(),
            provider_model: selected_provider_model.clone(),
            provider_options: selected_provider_options.clone(),
            exact_model: scheduled.selected.exact_model.clone(),
            pricing_snapshot: RoutePricingSnapshot::from_candidate(&scheduled.selected),
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
                let (candidate_provider_model, candidate_provider_options) =
                    provider_call_from_candidate(candidate);
                let mut provider_options = candidate_provider_options;
                let provider_model = self
                    .legacy_catalog_provider_model(
                        tenant_id,
                        &request.capability,
                        request.model.alias.as_str(),
                        candidate.provider_instance_name.as_str(),
                    )
                    .map(|provider_model| {
                        provider_options = None;
                        provider_model
                    })
                    .unwrap_or(candidate_provider_model);
                attempts.push(RouteAttempt {
                    instance_id: candidate.provider_instance_name.clone(),
                    provider_model,
                    provider_options,
                    exact_model: candidate.exact_model.clone(),
                    pricing_snapshot: RoutePricingSnapshot::from_candidate(&candidate),
                });
            }
        }

        let fallback_instance_ids = attempts
            .iter()
            .skip(1)
            .map(|item| item.instance_id.clone())
            .collect::<Vec<_>>();
        let disabled_capabilities = merge_disabled_capabilities(
            disabled_capabilities(request),
            definition_disabled_capabilities,
        );

        Ok(RouteDecision {
            primary_instance_id: scheduled.selected.provider_instance_name,
            fallback_instance_ids,
            provider_model: selected_provider_model,
            enabled_capabilities: enabled_capabilities(
                &scheduled.selected.metadata.capabilities,
                &disabled_capabilities,
            ),
            disabled_capabilities,
            attempts,
            route_trace: Arc::new(Mutex::new(resolution.trace)),
            runtime_failover_enabled: policy.runtime_failover,
        })
    }

    fn route_decision_response(
        &self,
        decision: &RouteDecision,
    ) -> std::result::Result<RouteResolveResponse, RPCErrors> {
        let primary = decision
            .attempts()
            .first()
            .ok_or_else(|| reason_error("no_provider_available", "route produced no attempts"))?;
        let inventory = self.registry.inventory(primary.instance_id.as_str());
        let trace = decision
            .route_trace
            .lock()
            .ok()
            .and_then(|trace| serde_json::to_value(&*trace).ok());
        let fallback_attempts = decision
            .attempts()
            .iter()
            .skip(1)
            .map(|attempt| RouteFallbackAttempt {
                exact_model: attempt.exact_model.clone(),
                provider_instance_name: attempt.instance_id.clone(),
                provider_model_id: attempt.provider_model.clone(),
                provider_options: attempt.provider_options.clone(),
            })
            .collect();

        Ok(RouteResolveResponse {
            selected_exact_model: primary.exact_model.clone(),
            provider_instance_name: primary.instance_id.clone(),
            provider_driver: inventory.as_ref().map(|item| item.provider_driver.clone()),
            provider_model_id: primary.provider_model.clone(),
            provider_options: primary.provider_options.clone(),
            enabled_capabilities: decision.enabled_capabilities.clone(),
            disabled_capabilities: decision.disabled_capabilities.clone(),
            fallback_attempts,
            route_trace: trace,
            inventory_revision: inventory.and_then(|item| item.inventory_revision),
        })
    }

    fn resolve_request_session_config(
        &self,
        request: &AiMethodRequest,
    ) -> std::result::Result<EffectiveSessionConfig, crate::model_types::RouteError> {
        let session_overlay = extract_session_config(request, "session_overlay")?;
        let global = self.session_config.read().map_err(|_| {
            crate::model_types::RouteError::new(
                crate::model_types::RouteErrorCode::ProviderUnavailable,
                "session config lock poisoned",
            )
        })?;
        let mut config = global.clone();
        drop(global);
        if let Some(session_overlay) = session_overlay {
            config = merge_session_config(&config, &session_overlay)?;
        }
        config.validate()?;
        build_effective_session_config(&config)
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
                request_features: request.requirements.effective_feature_names(),
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
            candidate.metadata.pricing.estimated_cost = Some(effective_cost.max(0.0));
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
                    .or(candidate.metadata.pricing.estimated_cost);
                if cost.map(|value| value > max_cost).unwrap_or(false) {
                    return false;
                }
            }
            if let Some(max_latency_ms) = policy.max_latency_ms {
                let latency = candidate
                    .dynamic_cost_estimate
                    .as_ref()
                    .and_then(|estimate| estimate.estimated_latency_ms)
                    .or(candidate.metadata.health.p95_latency_ms);
                if latency.map(|value| value > max_latency_ms).unwrap_or(false) {
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
        summary: &mut AiResponse,
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

    fn resolve_route_with_invoke_ctx(
        &self,
        request: RouteResolveRequest,
        invoke_ctx: InvokeCtx,
    ) -> std::result::Result<RouteResolveResponse, RPCErrors> {
        if crate::model_types::is_exact_model_name(request.logical_model.as_str()) {
            return Err(reason_error(
                "bad_request",
                "route.resolve logical_model must be a logical model name; exact model names are only valid for typed inference APIs",
            ));
        }
        let request_id = request
            .request_id
            .clone()
            .unwrap_or_else(|| self.generate_task_id());
        let method = method_for_route_api_type(request.api_type.as_str())?;
        let capability = capability_for_method(method).ok_or_else(|| {
            reason_error(
                "invalid_request",
                format!("api_type '{}' is not supported", request.api_type),
            )
        })?;
        let mut requirements = request.requirements.clone();
        if let Some(input_tokens) = request.estimated_input_tokens {
            let mut extra = requirements.extra.take().unwrap_or_else(|| json!({}));
            if !extra.is_object() {
                extra = json!({ "value": extra });
            }
            if let Some(object) = extra.as_object_mut() {
                object.insert("estimated_input_tokens".to_string(), json!(input_tokens));
                if let Some(output_tokens) = request.estimated_output_tokens {
                    object.insert("estimated_output_tokens".to_string(), json!(output_tokens));
                }
            }
            requirements.extra = Some(extra);
        }
        if let Some(session_overlay) = request.session_overlay.clone() {
            let mut extra = requirements.extra.take().unwrap_or_else(|| json!({}));
            if !extra.is_object() {
                extra = json!({ "value": extra });
            }
            if let Some(object) = extra.as_object_mut() {
                object.insert(
                    "session_overlay".to_string(),
                    serde_json::to_value(session_overlay).map_err(|error| {
                        reason_error(
                            "invalid_request",
                            format!("serialize session_overlay failed: {}", error),
                        )
                    })?,
                );
            }
            requirements.extra = Some(extra);
        }

        let mut method_request = AiMethodRequest::new(
            capability,
            ModelSpec::new(request.logical_model.clone(), None),
            requirements,
            route_probe_payload(request.estimated_input_tokens),
            None,
        )
        .with_policy(request.policy.clone());
        method_request.disable = request.disable.clone();
        apply_default_features_for_method(method, &mut method_request);

        let route_cfg = self
            .route_cfg
            .read()
            .map(|cfg| cfg.clone())
            .unwrap_or_default();
        let decision = self.route_request(
            invoke_ctx.tenant_id.as_str(),
            method,
            &method_request,
            &route_cfg,
            request_id.as_str(),
        )?;
        let response = self.route_decision_response(&decision)?;
        self.record_route_trace(
            &decision,
            invoke_ctx.tenant_id.as_str(),
            invoke_ctx.caller_app_id.clone(),
        );
        Ok(response)
    }

    pub fn resolve_route(
        &self,
        request: RouteResolveRequest,
        rpc_ctx: RPCContext,
    ) -> std::result::Result<RouteResolveResponse, RPCErrors> {
        self.resolve_route_with_invoke_ctx(request, InvokeCtx::from_unverified_rpc(&rpc_ctx))
    }

    async fn resolve_route_authenticated(
        &self,
        request: RouteResolveRequest,
        rpc_ctx: RPCContext,
    ) -> std::result::Result<RouteResolveResponse, RPCErrors> {
        let invoke_ctx = InvokeCtx::from_rpc(&rpc_ctx).await;
        self.resolve_route_with_invoke_ctx(request, invoke_ctx)
    }

    pub async fn create_chat_completion(
        &self,
        request: LlmChatInvokeRequest,
        rpc_ctx: RPCContext,
    ) -> std::result::Result<LlmChatInvokeResponse, RPCErrors> {
        let method_request = llm_chat_invoke_to_method_request(request)?;
        let response = self
            .complete_with_method(ai_methods::LLM_CHAT, method_request, rpc_ctx)
            .await?;
        Ok(response.into())
    }

    pub async fn generate_image(
        &self,
        request: TextToImageInvokeRequest,
        rpc_ctx: RPCContext,
    ) -> std::result::Result<TextToImageInvokeResponse, RPCErrors> {
        let method_request = image_generate_to_method_request(request)?;
        let response = self
            .complete_with_method(ai_methods::IMAGE_TXT2IMG, method_request, rpc_ctx)
            .await?;
        Ok(response.into())
    }

    pub async fn helper_llm_chat(
        &self,
        request: AiMethodRequest,
        rpc_ctx: RPCContext,
    ) -> std::result::Result<AiMethodResponse, RPCErrors> {
        let response_format = request
            .payload
            .input_json
            .as_ref()
            .and_then(|input| input.get("response_format"))
            .cloned()
            .map(|value| {
                serde_json::from_value::<LlmResponseFormat>(value).map_err(|error| {
                    RPCErrors::ParseRequestError(format!("invalid llm response_format: {}", error))
                })
            })
            .transpose()?
            .or_else(|| match request.requirements.resp_format {
                buckyos_api::RespFormat::Json => Some(LlmResponseFormat::json_object()),
                buckyos_api::RespFormat::Text => None,
            });
        let route = self
            .resolve_route_authenticated(
                route_request_from_method_request(ai_methods::LLM_CHAT, &request)?,
                rpc_ctx.clone(),
            )
            .await?;
        let provider_options = helper_provider_options(route.provider_options, &request.payload);
        let typed_response = self
            .create_chat_completion(
                LlmChatInvokeRequest {
                    exact_model: route.selected_exact_model,
                    messages: request.payload.messages.clone(),
                    tools: request.payload.tool_specs.clone(),
                    response_format,
                    temperature: request
                        .payload
                        .options
                        .as_ref()
                        .and_then(|options| options.get("temperature"))
                        .and_then(Value::as_f64),
                    max_output_tokens: request
                        .payload
                        .options
                        .as_ref()
                        .and_then(|options| options.get("max_output_tokens"))
                        .and_then(Value::as_u64),
                    payload: Some(request.payload),
                    provider_options,
                    idempotency_key: request.idempotency_key,
                    task_options: request.task_options,
                },
                rpc_ctx,
            )
            .await?;
        Ok(ai_method_response_from_llm_chat(typed_response))
    }

    pub async fn helper_text_to_image(
        &self,
        request: AiMethodRequest,
        rpc_ctx: RPCContext,
    ) -> std::result::Result<AiMethodResponse, RPCErrors> {
        let route = self
            .resolve_route_authenticated(
                route_request_from_method_request(ai_methods::IMAGE_TXT2IMG, &request)?,
                rpc_ctx.clone(),
            )
            .await?;
        let provider_options = helper_provider_options(route.provider_options, &request.payload);
        let prompt = request.payload.text.clone().unwrap_or_else(|| {
            request
                .payload
                .input_json
                .as_ref()
                .and_then(|value| value.get("prompt"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        });
        let typed_response = self
            .generate_image(
                TextToImageInvokeRequest {
                    exact_model: route.selected_exact_model,
                    prompt,
                    negative_prompt: request
                        .payload
                        .input_json
                        .as_ref()
                        .and_then(|value| value.get("negative_prompt"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    size: request
                        .payload
                        .input_json
                        .as_ref()
                        .and_then(|value| value.get("size"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    quality: request
                        .payload
                        .input_json
                        .as_ref()
                        .and_then(|value| value.get("quality"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    style: request
                        .payload
                        .input_json
                        .as_ref()
                        .and_then(|value| value.get("style"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    seed: request
                        .payload
                        .input_json
                        .as_ref()
                        .and_then(|value| value.get("seed"))
                        .and_then(Value::as_u64),
                    output: request
                        .payload
                        .input_json
                        .as_ref()
                        .and_then(|value| value.get("output"))
                        .cloned(),
                    payload: Some(request.payload),
                    provider_options,
                    idempotency_key: request.idempotency_key,
                    task_options: request.task_options,
                },
                rpc_ctx,
            )
            .await?;
        Ok(ai_method_response_from_text_to_image(typed_response))
    }

    pub async fn complete_with_method(
        &self,
        method: &str,
        mut request: AiMethodRequest,
        rpc_ctx: RPCContext,
    ) -> std::result::Result<AiMethodResponse, RPCErrors> {
        let mut invoke_ctx = InvokeCtx::from_rpc(&rpc_ctx).await;
        apply_default_features_for_method(method, &mut request);
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
        invoke_ctx.task_id = Some(external_task_id.clone());
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

        if method == ai_methods::VIDEO_EXTEND && !request_has_continuation_handle(&request) {
            if let (Some(db), Some(content_id)) = (
                self.usage_log_db.as_ref(),
                video_content_id_from_resolved_request(&resolved.request)?,
            ) {
                match db
                    .get_video_continuation_source(
                        invoke_ctx.tenant_id.as_str(),
                        content_id.as_str(),
                    )
                    .await
                {
                    Ok(Some(source)) => match self.acquire_task_manager_client(&invoke_ctx).await {
                        Ok(taskmgr) => match taskmgr.get_task(source.source_task_id.as_str()).await {
                            Ok(task) => {
                                if let Some((handle, exact_model)) =
                                    continuation_from_source_task(&task, invoke_ctx.tenant_id.as_str())
                                {
                                    request.model.alias = exact_model.clone();
                                    resolved.request.model.alias = exact_model.clone();
                                    insert_continuation_handle(&mut request, handle.clone());
                                    insert_continuation_handle(&mut resolved.request, handle);
                                    info!(
                                        "aicc.video_continuation restored: task_id={} tenant={} content_id={} source_task_id={} exact_model={}",
                                        external_task_id,
                                        invoke_ctx.tenant_id,
                                        content_id,
                                        source.source_task_id,
                                        exact_model
                                    );
                                } else {
                                    info!(
                                        "aicc.video_continuation source unusable: task_id={} tenant={} content_id={} source_task_id={}",
                                        external_task_id,
                                        invoke_ctx.tenant_id,
                                        content_id,
                                        source.source_task_id
                                    );
                                }
                            }
                            Err(error) => warn!(
                                "aicc.video_continuation source_task_failed: task_id={} tenant={} content_id={} source_task_id={} err={}",
                                external_task_id,
                                invoke_ctx.tenant_id,
                                content_id,
                                source.source_task_id,
                                error
                            ),
                        },
                        Err(error) => warn!(
                            "aicc.video_continuation task_manager_failed: task_id={} tenant={} content_id={} err={}",
                            external_task_id, invoke_ctx.tenant_id, content_id, error
                        ),
                    },
                    Ok(None) => info!(
                        "aicc.video_continuation source unavailable: task_id={} tenant={} content_id={}",
                        external_task_id, invoke_ctx.tenant_id, content_id
                    ),
                    Err(error) => warn!(
                        "aicc.video_continuation source_lookup_failed: task_id={} tenant={} content_id={} err={}",
                        external_task_id, invoke_ctx.tenant_id, content_id, error
                    ),
                }
            }
        }

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
            request.requirements.effective_feature_names(),
            route_policy_from_request(&request).max_estimated_cost_usd,
            route_policy_from_request(&request).max_latency_ms
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
        let selected_provider_model = Arc::new(std::sync::RwLock::new(
            decision
                .attempts()
                .first()
                .map(|attempt| attempt.exact_model.clone())
                .unwrap_or_else(|| decision.provider_model.clone()),
        ));
        let event_sink: Arc<dyn TaskEventSink> = if let Some(db) = self.usage_log_db.clone() {
            let context = UsageLogContext {
                external_task_id: external_task_id.clone(),
                tenant_id: invoke_ctx.tenant_id.clone(),
                caller_app_id: invoke_ctx.caller_app_id.clone(),
                capability: capability_name(&request.capability).to_string(),
                request_model: request.model.alias.clone(),
                provider_model: selected_provider_model.clone(),
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
                selected_provider_model,
            )
            .await;
        self.record_route_trace(
            &decision,
            invoke_ctx.tenant_id.as_str(),
            invoke_ctx.caller_app_id.clone(),
        );
        match start_result {
            Ok((ProviderStartResult::Immediate(mut summary), instance_id)) => {
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
                    prepared_task.taskmgr_override.clone(),
                    prepared_task.taskmgr_context.clone(),
                    prepared_task.id(),
                    invoke_ctx.tenant_id.clone(),
                    self.usage_log_db.clone(),
                ));
                deferred_sink.promote(task_audit_sink).await?;
                self.emit_task_started(
                    event_sink.clone(),
                    external_task_id.as_str(),
                    instance_id.as_str(),
                )
                .await;
                match materialize_response_artifacts_if_needed(
                    external_task_id.as_str(),
                    &request,
                    &mut summary,
                )
                .await
                {
                    Ok(count) => {
                        if count > 0 {
                            info!(
                                "aicc.output materialized base64 artifacts: task_id={} count={}",
                                external_task_id, count
                            );
                        }
                    }
                    Err(error) => {
                        let code = extract_error_code(&error);
                        warn!(
                            "aicc.output materialize_failed: task_id={} tenant={} code={} err={}",
                            external_task_id, invoke_ctx.tenant_id, code, error
                        );
                        self.emit_task_error(
                            event_sink,
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
                }
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
                    prepared_task.taskmgr_override.clone(),
                    prepared_task.taskmgr_context.clone(),
                    task_mgr_id.clone(),
                    invoke_ctx.tenant_id.clone(),
                    self.usage_log_db.clone(),
                ));
                deferred_sink.promote(task_audit_sink).await?;
                self.bind_task(
                    external_task_id.as_str(),
                    invoke_ctx.tenant_id.as_str(),
                    instance_id.as_str(),
                    task_mgr_id.clone(),
                );
                self.emit_task_started(event_sink, external_task_id.as_str(), instance_id.as_str())
                    .await;
                Ok(AiMethodResponse::new(
                    task_mgr_id,
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
                    prepared_task.taskmgr_override.clone(),
                    prepared_task.taskmgr_context.clone(),
                    task_mgr_id.clone(),
                    invoke_ctx.tenant_id.clone(),
                    self.usage_log_db.clone(),
                ));
                deferred_sink.promote(task_audit_sink).await?;
                self.bind_task(
                    external_task_id.as_str(),
                    invoke_ctx.tenant_id.as_str(),
                    instance_id.as_str(),
                    task_mgr_id.clone(),
                );
                self.emit_task_queued(event_sink, external_task_id.as_str(), position)
                    .await;
                Ok(AiMethodResponse::new(
                    task_mgr_id,
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
        let invoke_ctx = InvokeCtx::from_rpc(&rpc_ctx).await;
        info!(
            "aicc.cancel received: tenant={} caller_app={:?} task_id={}",
            invoke_ctx.tenant_id, invoke_ctx.caller_app_id, task_id
        );

        let binding = self.task_bindings.read().ok().and_then(|bindings| {
            bindings
                .get_key_value(task_id)
                .or_else(|| {
                    bindings
                        .iter()
                        .find(|(_, item)| item.task_mgr_id == task_id)
                })
                .map(|(external_task_id, item)| (external_task_id.clone(), item.clone()))
        });
        let Some((external_task_id, binding)) = binding else {
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

        let accepted = provider
            .cancel(invoke_ctx.clone(), external_task_id.as_str())
            .await
            .is_ok();
        if accepted {
            if let Ok(taskmgr) = self.acquire_task_manager_client(&invoke_ctx).await {
                let event = TaskEvent {
                    task_id: external_task_id.clone(),
                    kind: TaskEventKind::CancelRequested,
                    timestamp_ms: now_ms(),
                    data: Some(json!({
                        "accepted": true,
                        "source": "cancel_api"
                    })),
                };
                if let Ok(task) = taskmgr.get_task(&binding.task_mgr_id).await {
                    if !task.phase.is_terminal() {
                        let mut task_data = task.progress.clone().unwrap_or_else(|| json!({}));
                        merge_task_data_with_event(&mut task_data, &event);
                        let _ = taskmgr
                            .report_progress(ReportProgressReq {
                                envelope: RunnerWriteEnvelope {
                                    task_id: task.task_id.clone(),
                                    app_instance_id: None,
                                    runner_epoch: task.runner_epoch,
                                    expected_revision: task.revision,
                                },
                                progress: Some(task_data),
                                message: Some("aicc task canceled".to_string()),
                            })
                            .await;
                        let _ = cancel_own_task(&taskmgr, &task).await;
                    }
                }
            }
            if let Ok(mut bindings) = self.task_bindings.write() {
                bindings.remove(external_task_id.as_str());
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
        selected_provider_model: Arc<std::sync::RwLock<String>>,
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
                    trace.selected_provider_model_id = Some(attempt.provider_model.clone());
                    trace.provider_options = attempt.provider_options.clone();
                    trace.pricing_snapshot = attempt.pricing_snapshot.clone();
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

            if let Ok(mut selected) = selected_provider_model.write() {
                *selected = attempt.exact_model.clone();
            }

            self.registry.mark_start_begin(attempt.instance_id.as_str());
            let started_at = Instant::now();
            let mut provider_req = req.clone();
            merge_provider_options(
                &mut provider_req.request.payload,
                attempt.provider_options.clone(),
            );
            let result = provider
                .start(
                    ctx.clone(),
                    attempt.provider_model.clone(),
                    provider_req,
                    sink.clone(),
                )
                .await;
            let elapsed_ms = started_at.elapsed().as_millis() as f64;

            match result {
                Ok(mut start_result) => {
                    if let ProviderStartResult::Immediate(summary) = &mut start_result {
                        ensure_summary_accounting(summary, attempt);
                        self.apply_billing_to_summary(
                            ctx,
                            self.registry
                                .inventory(attempt.instance_id.as_str())
                                .map(|inventory| inventory.provider_driver)
                                .unwrap_or_default()
                                .as_str(),
                            summary,
                        );
                        append_provider_audit_to_summary(summary, attempt);
                    }
                    self.registry
                        .record_start_success(attempt.instance_id.as_str(), elapsed_ms);
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
        if let Some(input_json) = req.payload.input_json.as_ref() {
            self.validate_resources_in_value(input_json)?;
        }
        Ok(())
    }

    fn validate_resources_in_value(&self, value: &Value) -> std::result::Result<(), RPCErrors> {
        match value {
            Value::Object(object) => {
                if object
                    .get("kind")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| matches!(kind, "url" | "base64" | "named_object"))
                {
                    let resource =
                        serde_json::from_value::<ResourceRef>(value.clone()).map_err(|error| {
                            reason_error(
                                "resource_invalid",
                                format!("canonical resource is invalid: {}", error),
                            )
                        })?;
                    return self.validate_resource(&resource);
                }
                for child in object.values() {
                    self.validate_resources_in_value(child)?;
                }
            }
            Value::Array(items) => {
                for child in items {
                    self.validate_resources_in_value(child)?;
                }
            }
            _ => {}
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

    fn bind_task(&self, task_id: &str, tenant_id: &str, instance_id: &str, task_mgr_id: String) {
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
        summary: &AiResponse,
    ) {
        let summary_value = serde_json::to_value(summary).unwrap_or_else(|_| json!({}));
        let event = TaskEvent {
            task_id: task_id.to_string(),
            kind: TaskEventKind::Final,
            timestamp_ms: now_ms(),
            data: Some(json!({
                "summary": summary_value,
                "finish_reason": summary.finish_reason.clone(),
                "has_text": !summary.text_content().is_empty(),
                "artifact_count": summary.artifacts().len(),
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

    async fn handle_route_resolve(
        &self,
        request: RouteResolveRequest,
        ctx: RPCContext,
    ) -> std::result::Result<RouteResolveResponse, RPCErrors> {
        self.resolve_route_authenticated(request, ctx).await
    }

    async fn handle_chat_completions_create(
        &self,
        request: LlmChatInvokeRequest,
        ctx: RPCContext,
    ) -> std::result::Result<LlmChatInvokeResponse, RPCErrors> {
        self.create_chat_completion(request, ctx).await
    }

    async fn handle_images_generate(
        &self,
        request: TextToImageInvokeRequest,
        ctx: RPCContext,
    ) -> std::result::Result<TextToImageInvokeResponse, RPCErrors> {
        self.generate_image(request, ctx).await
    }

    async fn handle_helper_llm_chat(
        &self,
        request: AiMethodRequest,
        ctx: RPCContext,
    ) -> std::result::Result<AiMethodResponse, RPCErrors> {
        self.helper_llm_chat(request, ctx).await
    }

    async fn handle_helper_text_to_image(
        &self,
        request: AiMethodRequest,
        ctx: RPCContext,
    ) -> std::result::Result<AiMethodResponse, RPCErrors> {
        self.helper_text_to_image(request, ctx).await
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
    let reason_short = if trace.runtime_failover_count > 0 {
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

fn build_initial_aicc_task_data(
    request: &AiMethodRequest,
    external_task_id: &str,
    event_ref: Option<&str>,
    invoke_ctx: &InvokeCtx,
) -> serde_json::Value {
    let session_id = extract_session_id_from_complete_request(request);
    aicc_task_data_value(AiccComputeTaskData {
        request: AiccComputeTaskRequest {
            version: 1,
            external_task_id: Some(external_task_id.to_string()),
            tenant_id: Some(invoke_ctx.tenant_id.clone()),
            event_ref: event_ref.map(ToString::to_string),
            session_id: session_id.clone(),
            owner_session_id: session_id,
            request: Some(serde_json::to_value(request).unwrap_or(Value::Null)),
            provider_input: None,
            route: Some(json!({})),
            created_at_ms: Some(now_ms_i64()),
        },
        progress: Some(AiccComputeProgress {
            status: Some("pending".to_string()),
            updated_at_ms: Some(now_ms_i64()),
            events: Vec::new(),
        }),
        result: None,
        error: None,
    })
}

fn merge_route_decision_into_task_data(
    data: &mut serde_json::Value,
    decision: &RouteDecision,
    initial_status: &str,
) {
    let mut task_data = parse_aicc_task_data(data);
    let primary = decision.attempts().first();
    task_data.request.route = Some(json!({
            "primary_instance_id": decision.primary_instance_id,
            "fallback_instance_ids": decision.fallback_instance_ids,
            "provider_model": decision.provider_model,
            "selected_exact_model": primary.map(|attempt| attempt.exact_model.clone()),
            "provider_actual_model": primary.map(|attempt| attempt.provider_model.clone()),
            "provider_options": primary.and_then(|attempt| attempt.provider_options.clone()),
    }));
    let progress = task_data.progress.get_or_insert_with(Default::default);
    progress.status = Some(initial_status.to_string());
    progress.updated_at_ms = Some(now_ms_i64());
    *data = aicc_task_data_value(task_data);
}

fn merge_task_data_with_event(data: &mut serde_json::Value, event: &TaskEvent) {
    let mut task_data = parse_aicc_task_data(data);
    let status = match event.kind {
        TaskEventKind::Queued => "queued",
        TaskEventKind::Started => "running",
        TaskEventKind::Final => "succeeded",
        TaskEventKind::Error => "failed",
        TaskEventKind::CancelRequested => "canceled",
    };
    let progress = task_data.progress.get_or_insert_with(Default::default);
    progress.status = Some(status.to_string());
    progress.updated_at_ms = Some(now_ms_i64());

    let event_json = serde_json::to_value(event).unwrap_or_else(|_| json!({}));
    progress.events.push(event_json);
    if progress.events.len() > AICC_TASK_EVENT_RETENTION {
        let to_drop = progress
            .events
            .len()
            .saturating_sub(AICC_TASK_EVENT_RETENTION);
        progress.events.drain(0..to_drop);
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
                            task_data.request.provider_input = Some(input.clone());
                        }
                        if let Some(output) = provider_io.get("output") {
                            task_data
                                .result
                                .get_or_insert_with(Default::default)
                                .provider_output = Some(output.clone());
                        }
                    }
                }
                task_data.result.get_or_insert_with(Default::default).output = Some(summary);
            }
            task_data.error = None;
        }
        TaskEventKind::Error => {
            task_data.error = Some(
                event
                    .data
                    .clone()
                    .unwrap_or_else(|| json!({"message":"unknown"})),
            );
        }
        TaskEventKind::CancelRequested => {
            task_data.error = Some(
                event
                    .data
                    .clone()
                    .unwrap_or_else(|| json!({"message":"cancel requested"})),
            );
        }
        TaskEventKind::Started | TaskEventKind::Queued => {}
    }
    *data = aicc_task_data_value(task_data);
}

fn parse_aicc_task_data(data: &Value) -> AiccComputeTaskData {
    match buckyos_api::parse_typed_task_data("aicc.compute", data.clone()) {
        Ok(TypedTaskData::AiccCompute(data)) => data,
        _ => AiccComputeTaskData {
            request: AiccComputeTaskRequest::default(),
            progress: Some(AiccComputeProgress::default()),
            result: None,
            error: None,
        },
    }
}

fn aicc_task_data_value(data: AiccComputeTaskData) -> Value {
    serde_json::to_value(data).unwrap_or_else(|_| Value::Object(Default::default()))
}

fn now_ms_i64() -> i64 {
    now_ms().min(i64::MAX as u128) as i64
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ArtifactOutputStorage {
    NamedObject,
    InlineBase64,
}

fn requested_artifact_output_storage(req: &AiMethodRequest) -> ArtifactOutputStorage {
    if artifact_output_storage_from_value(req.payload.input_json.as_ref())
        == Some(ArtifactOutputStorage::InlineBase64)
        || artifact_output_storage_from_value(req.payload.options.as_ref())
            == Some(ArtifactOutputStorage::InlineBase64)
    {
        return ArtifactOutputStorage::InlineBase64;
    }
    ArtifactOutputStorage::NamedObject
}

fn artifact_output_storage_from_value(value: Option<&Value>) -> Option<ArtifactOutputStorage> {
    let object = value?.as_object()?;
    for key in ["resource_format", "response_format"] {
        if let Some(storage) = artifact_output_storage_from_format_value(object.get(key)) {
            return Some(storage);
        }
    }
    if let Some(output) = object.get("output").and_then(|value| value.as_object()) {
        for key in ["resource_format", "response_format"] {
            if let Some(storage) = artifact_output_storage_from_format_value(output.get(key)) {
                return Some(storage);
            }
        }
    }
    None
}

fn artifact_output_storage_from_format_value(
    value: Option<&Value>,
) -> Option<ArtifactOutputStorage> {
    let format = value?.as_str()?.trim().to_ascii_lowercase();
    match format.as_str() {
        "base64" | "b64_json" | "inline_base64" | "data_url" => {
            Some(ArtifactOutputStorage::InlineBase64)
        }
        "named_object" | "object_id" | "obj_id" | "url" => Some(ArtifactOutputStorage::NamedObject),
        _ => None,
    }
}

fn response_has_base64_artifacts(summary: &AiResponse) -> bool {
    summary.message.content.iter().any(|block| match block {
        AiContent::Image {
            source: ResourceRef::Base64 { .. },
        }
        | AiContent::Document {
            source: ResourceRef::Base64 { .. },
            ..
        } => true,
        _ => false,
    })
}

#[derive(Clone, Debug)]
struct StoredNamedArtifact {
    obj_id: ObjId,
    content_id: String,
    width: Option<u32>,
    height: Option<u32>,
}

pub(crate) async fn materialize_response_artifacts_if_needed(
    task_id: &str,
    req: &AiMethodRequest,
    summary: &mut AiResponse,
) -> std::result::Result<usize, RPCErrors> {
    if requested_artifact_output_storage(req) == ArtifactOutputStorage::InlineBase64 {
        return Ok(0);
    }
    if !response_has_base64_artifacts(summary) {
        return Ok(0);
    }

    let runtime = get_buckyos_api_runtime().map_err(|error| {
        reason_error(
            "artifact_materialize_failed",
            format!("get buckyos runtime failed: {}", error),
        )
    })?;
    let named_store = runtime.get_named_store().await.map_err(|error| {
        reason_error(
            "artifact_materialize_failed",
            format!("get named_store failed: {}", error),
        )
    })?;

    let mut count = 0usize;
    let mut materialized = Vec::new();
    for (idx, block) in summary.message.content.iter_mut().enumerate() {
        match block {
            AiContent::Image { source } => {
                if let ResourceRef::Base64 { mime, data_base64 } = source {
                    let mime = mime.clone();
                    let data_base64 = data_base64.clone();
                    let stored = store_base64_artifact(
                        &named_store,
                        task_id,
                        idx,
                        None,
                        mime.as_str(),
                        data_base64.as_str(),
                    )
                    .await?;
                    materialized.push(materialized_artifact_value(idx, &mime, &stored));
                    *source = ResourceRef::NamedObject {
                        obj_id: stored.obj_id,
                    };
                    count += 1;
                }
            }
            AiContent::Document { source, title } => {
                if let ResourceRef::Base64 { mime, data_base64 } = source {
                    let mime = mime.clone();
                    let data_base64 = data_base64.clone();
                    let title = title.clone();
                    let stored = store_base64_artifact(
                        &named_store,
                        task_id,
                        idx,
                        title.as_deref(),
                        mime.as_str(),
                        data_base64.as_str(),
                    )
                    .await?;
                    materialized.push(materialized_artifact_value(idx, &mime, &stored));
                    *source = ResourceRef::NamedObject {
                        obj_id: stored.obj_id,
                    };
                    count += 1;
                }
            }
            _ => {}
        }
    }
    if !materialized.is_empty() {
        append_materialized_artifact_extra(summary, materialized);
    }
    Ok(count)
}

pub(crate) async fn emit_background_provider_result(
    sink: Arc<dyn TaskEventSink>,
    task_id: &str,
    request: &AiMethodRequest,
    result: std::result::Result<AiResponse, ProviderError>,
) {
    let event = match result {
        Ok(mut summary) => {
            if let Err(error) =
                materialize_response_artifacts_if_needed(task_id, request, &mut summary).await
            {
                TaskEvent {
                    task_id: task_id.to_string(),
                    kind: TaskEventKind::Error,
                    timestamp_ms: now_ms(),
                    data: Some(json!({
                        "code": "artifact_materialize_failed",
                        "message": format!("materialize artifact failed: {}", error)
                    })),
                }
            } else {
                let has_text = !summary.text_content().is_empty();
                let artifact_count = summary.artifacts().len();
                let finish_reason = summary.finish_reason.clone();
                TaskEvent {
                    task_id: task_id.to_string(),
                    kind: TaskEventKind::Final,
                    timestamp_ms: now_ms(),
                    data: Some(json!({
                        "summary": serde_json::to_value(&summary).unwrap_or_else(|_| json!({})),
                        "finish_reason": finish_reason,
                        "has_text": has_text,
                        "artifact_count": artifact_count
                    })),
                }
            }
        }
        Err(error) => TaskEvent {
            task_id: task_id.to_string(),
            kind: TaskEventKind::Error,
            timestamp_ms: now_ms(),
            data: Some(json!({
                "code": "provider_error",
                "message": error.to_string()
            })),
        },
    };
    if let Err(error) = sink.emit(event).await {
        error!(
            "aicc.background_provider_event_failed task_id={} err={}",
            task_id, error
        );
    }
}

async fn store_base64_artifact(
    named_store: &named_store::NamedDataMgr,
    task_id: &str,
    idx: usize,
    title: Option<&str>,
    mime: &str,
    data_base64: &str,
) -> std::result::Result<StoredNamedArtifact, RPCErrors> {
    let encoded = data_base64
        .split_once(',')
        .filter(|(prefix, _)| prefix.trim_start().starts_with("data:"))
        .map(|(_, data)| data)
        .unwrap_or(data_base64);
    let bytes = general_purpose::STANDARD.decode(encoded).map_err(|error| {
        reason_error(
            "artifact_materialize_failed",
            format!("decode base64 artifact {} failed: {}", idx + 1, error),
        )
    })?;
    let dimensions = image_dimensions_from_bytes(mime, &bytes);
    let chunk_id = ChunkHasher::new(None)
        .map_err(|error| {
            reason_error(
                "artifact_materialize_failed",
                format!("create chunk hasher failed: {}", error),
            )
        })?
        .calc_mix_chunk_id_from_bytes(&bytes)
        .map_err(|error| {
            reason_error(
                "artifact_materialize_failed",
                format!("calculate artifact chunk id failed: {}", error),
            )
        })?;
    named_store
        .put_chunk(&chunk_id, &bytes)
        .await
        .map_err(|error| {
            reason_error(
                "artifact_materialize_failed",
                format!(
                    "store artifact chunk {} failed: {}",
                    chunk_id.to_string(),
                    error
                ),
            )
        })?;

    let file_name = artifact_file_name(task_id, idx, title, mime);
    let mut file_obj = FileObject::new(file_name, bytes.len() as u64, chunk_id.to_string());
    file_obj
        .meta
        .insert("mime_type".to_string(), json!(mime.to_string()));
    file_obj
        .meta
        .insert("aicc_task_id".to_string(), json!(task_id.to_string()));

    let (file_obj_id, file_obj_json) = file_obj.gen_obj_id();
    named_store
        .put_object(&file_obj_id, file_obj_json.as_str())
        .await
        .map_err(|error| {
            reason_error(
                "artifact_materialize_failed",
                format!(
                    "store artifact file object {} failed: {}",
                    file_obj_id.to_string(),
                    error
                ),
            )
        })?;
    Ok(StoredNamedArtifact {
        obj_id: file_obj_id,
        content_id: chunk_id.to_string(),
        width: dimensions.map(|item| item.0),
        height: dimensions.map(|item| item.1),
    })
}

fn materialized_artifact_value(idx: usize, mime: &str, stored: &StoredNamedArtifact) -> Value {
    let mut value = json!({
        "content_index": idx,
        "resource_kind": "named_object",
        "obj_id": stored.obj_id.to_string(),
        "content_id": stored.content_id,
        "mime": mime,
    });
    if let Some(object) = value.as_object_mut() {
        if let (Some(width), Some(height)) = (stored.width, stored.height) {
            object.insert("width".to_string(), json!(width));
            object.insert("height".to_string(), json!(height));
        }
    }
    value
}

fn append_materialized_artifact_extra(summary: &mut AiResponse, materialized: Vec<Value>) {
    let mut extra = summary.extra.take().unwrap_or_else(|| json!({}));
    if !extra.is_object() {
        extra = json!({ "provider_extra": extra });
    }
    if let Some(object) = extra.as_object_mut() {
        object.insert(
            "materialized_artifacts".to_string(),
            Value::Array(materialized),
        );
    }
    summary.extra = Some(extra);
}

fn image_dimensions_from_bytes(mime: &str, bytes: &[u8]) -> Option<(u32, u32)> {
    let normalized = mime.split(';').next().unwrap_or("").trim();
    match normalized {
        "image/png" => parse_png_dimensions(bytes),
        "image/jpeg" | "image/jpg" => parse_jpeg_dimensions(bytes),
        _ => parse_png_dimensions(bytes).or_else(|| parse_jpeg_dimensions(bytes)),
    }
}

fn parse_png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((width, height))
}

fn parse_jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 4 || bytes[0] != 0xff || bytes[1] != 0xd8 {
        return None;
    }
    let mut offset = 2usize;
    while offset + 9 < bytes.len() {
        while offset < bytes.len() && bytes[offset] == 0xff {
            offset += 1;
        }
        if offset >= bytes.len() {
            return None;
        }
        let marker = bytes[offset];
        offset += 1;
        if marker == 0xd9 || marker == 0xda {
            return None;
        }
        if offset + 2 > bytes.len() {
            return None;
        }
        let length = u16::from_be_bytes(bytes[offset..offset + 2].try_into().ok()?) as usize;
        if length < 2 || offset + length > bytes.len() {
            return None;
        }
        let is_sof = matches!(
            marker,
            0xc0 | 0xc1
                | 0xc2
                | 0xc3
                | 0xc5
                | 0xc6
                | 0xc7
                | 0xc9
                | 0xca
                | 0xcb
                | 0xcd
                | 0xce
                | 0xcf
        );
        if is_sof && offset + 7 < bytes.len() {
            let height = u16::from_be_bytes(bytes[offset + 3..offset + 5].try_into().ok()?) as u32;
            let width = u16::from_be_bytes(bytes[offset + 5..offset + 7].try_into().ok()?) as u32;
            return Some((width, height));
        }
        offset += length;
    }
    None
}

fn artifact_file_name(task_id: &str, idx: usize, title: Option<&str>, mime: &str) -> String {
    let base = title
        .map(sanitize_file_name)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| format!("aicc-{}-artifact-{}", sanitize_file_name(task_id), idx + 1));
    let ext = extension_for_mime(mime);
    if ext.is_empty() || base.ends_with(ext) {
        base
    } else {
        format!("{}{}", base, ext)
    }
}

fn sanitize_file_name(value: &str) -> String {
    let mut out = String::with_capacity(value.len().min(96));
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            out.push(ch);
        } else if !out.ends_with('_') {
            out.push('_');
        }
        if out.len() >= 96 {
            break;
        }
    }
    out.trim_matches('_').to_string()
}

fn extension_for_mime(mime: &str) -> &'static str {
    match mime.split(';').next().unwrap_or("").trim() {
        "image/png" => ".png",
        "image/jpeg" => ".jpg",
        "image/webp" => ".webp",
        "image/gif" => ".gif",
        "audio/mpeg" | "audio/mp3" => ".mp3",
        "audio/wav" | "audio/x-wav" => ".wav",
        "audio/ogg" => ".ogg",
        "audio/aac" => ".aac",
        "video/mp4" => ".mp4",
        "video/webm" => ".webm",
        "application/pdf" => ".pdf",
        "application/json" => ".json",
        "text/plain" => ".txt",
        _ => "",
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
        AckControlReq, AddTaskNoteReq, AiPayload, AiTaskOptions, CommitResultReq, CreateTaskReq,
        GetTaskReq, ListTaskNotesReq, ListTasksReq, ModelSpec, ReportProgressReq, ReportStartedReq,
        RequestControlReq, RequestControlResult, Requirements, StorageDomain, Task,
        TaskControlRequest, TaskExecutor, TaskManagerClient, TaskManagerHandler, TaskNote,
        TaskOutcome, TaskSummaryPage, TypedTaskData,
    };
    use serde_json::json;
    use std::collections::{HashMap, VecDeque};
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn aicc_task_data(task: &Task) -> Option<AiccComputeTaskData> {
        let data = task
            .result
            .clone()
            .or_else(|| task.progress.clone())
            .unwrap_or_else(|| task.input.clone());
        match buckyos_api::parse_typed_task_data("aicc.compute", data).ok()? {
            TypedTaskData::AiccCompute(data) => Some(data),
            _ => None,
        }
    }

    fn aicc_external_task_id(task: &Task) -> Option<String> {
        aicc_task_data(task).and_then(|data| data.request.external_task_id)
    }

    fn aicc_status(task: &Task) -> Option<String> {
        aicc_task_data(task).and_then(|data| data.progress.and_then(|progress| progress.status))
    }

    #[allow(dead_code)]
    fn aicc_event_kind(task: &Task, index: usize) -> Option<String> {
        aicc_task_data(task)
            .and_then(|data| data.progress)
            .and_then(|progress| progress.events.get(index).cloned())
            .and_then(|event| {
                event
                    .get("kind")
                    .and_then(|value| value.as_str())
                    .map(ToString::to_string)
            })
    }

    async fn all_tasks(taskmgr: &TaskManagerClient) -> Vec<Task> {
        let page = taskmgr
            .list_tasks(ListTasksReq::default())
            .await
            .expect("list tasks");
        let mut tasks = Vec::new();
        for summary in page.tasks {
            tasks.push(taskmgr.get_task(&summary.task_id).await.expect("get task"));
        }
        tasks
    }

    struct MockTaskMgrHandler {
        counter: Mutex<u64>,
        note_counter: Mutex<u64>,
        tasks: Arc<Mutex<HashMap<String, Task>>>,
        notes: Arc<Mutex<HashMap<String, Vec<TaskNote>>>>,
    }

    impl MockTaskMgrHandler {
        fn now() -> u64 {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64
        }

        fn with_task<T>(
            &self,
            task_id: &str,
            edit: impl FnOnce(&mut Task) -> T,
        ) -> std::result::Result<(Task, T), RPCErrors> {
            let mut guard = self.tasks.lock().expect("tasks lock");
            let task = guard.get_mut(task_id).ok_or_else(|| {
                RPCErrors::ReasonError(format!("mock task {} not found", task_id))
            })?;
            let value = edit(task);
            task.revision += 1;
            task.updated_at = Self::now();
            Ok((task.clone(), value))
        }
    }

    #[async_trait]
    impl TaskManagerHandler for MockTaskMgrHandler {
        async fn handle_create_task(
            &self,
            req: CreateTaskReq,
            _ctx: RPCContext,
        ) -> std::result::Result<Task, RPCErrors> {
            let mut guard = self.counter.lock().expect("counter lock");
            *guard += 1;
            let now = Self::now();
            let task_id = format!("t-mock-{}", *guard);
            let executor = match &req.executor {
                buckyos_api::CreateTaskExecutor::SelfApp { app_instance_id } => TaskExecutor::App {
                    target_id: None,
                    app_id: "aicc".to_string(),
                    app_instance_id: app_instance_id.clone(),
                },
                buckyos_api::CreateTaskExecutor::HumanSet { .. } => TaskExecutor::HumanSet,
            };
            let task = Task {
                task_id: task_id.clone(),
                name: req.name.clone(),
                parent_id: req.parent_id.clone(),
                root_id: req.parent_id.clone().unwrap_or_else(|| task_id.clone()),
                child_control_policy: req.child_control_policy.unwrap_or_default(),
                schema_id: req.schema_id.clone(),
                schema_version: 1,
                input: req.input.clone(),
                input_digest: buckyos_api::compute_task_input_digest(&req.input),
                creator: buckyos_api::ActorRef::new("tester", "aicc"),
                storage_domain: req.storage_domain.unwrap_or(StorageDomain::System),
                idempotency_key: req.idempotency_key.clone(),
                origin_ref: None,
                retry_of: None,
                supersedes: None,
                executor,
                runner_epoch: 1,
                assignees: None,
                phase: TaskPhase::Accepted,
                wait_reason: None,
                pending_control: None,
                control_profile: buckyos_api::TaskControlProfile::baseline(now),
                progress: None,
                message: req.message.clone(),
                outcome: None,
                result: None,
                error: None,
                completed_by: None,
                policy_preset: "collaborative-tree/v1".to_string(),
                permission_boundary: false,
                revision: 1,
                data_scope: None,
                created_at: now,
                updated_at: now,
                completed_at: None,
                archived_at: None,
            };
            self.tasks
                .lock()
                .expect("tasks lock")
                .insert(task_id, task.clone());
            Ok(task)
        }

        async fn handle_get_task(
            &self,
            req: GetTaskReq,
            _ctx: RPCContext,
        ) -> std::result::Result<Task, RPCErrors> {
            self.tasks
                .lock()
                .expect("tasks lock")
                .get(&req.task_id)
                .cloned()
                .ok_or_else(|| {
                    RPCErrors::ReasonError(format!("mock task {} not found", req.task_id))
                })
        }

        async fn handle_list_tasks(
            &self,
            _req: ListTasksReq,
            _ctx: RPCContext,
        ) -> std::result::Result<TaskSummaryPage, RPCErrors> {
            let tasks = self
                .tasks
                .lock()
                .expect("tasks lock")
                .values()
                .map(|task| buckyos_api::TaskSummary {
                    task_id: task.task_id.clone(),
                    name: task.name.clone(),
                    parent_id: task.parent_id.clone(),
                    root_id: task.root_id.clone(),
                    schema_id: task.schema_id.clone(),
                    schema_version: task.schema_version,
                    creator: task.creator.clone(),
                    storage_domain: task.storage_domain,
                    executor_kind: task.executor.kind(),
                    phase: task.phase,
                    wait_reason: task.wait_reason.clone(),
                    pending_control_action: task.pending_control.as_ref().map(|c| c.action),
                    outcome: task.outcome,
                    message: task.message.clone(),
                    revision: task.revision,
                    created_at: task.created_at,
                    updated_at: task.updated_at,
                    completed_at: task.completed_at,
                    archived_at: task.archived_at,
                })
                .collect();
            Ok(TaskSummaryPage {
                tasks,
                next_cursor: None,
            })
        }

        async fn handle_report_started(
            &self,
            req: ReportStartedReq,
            _ctx: RPCContext,
        ) -> std::result::Result<Task, RPCErrors> {
            let (task, _) = self.with_task(&req.envelope.task_id, |task| {
                task.phase = TaskPhase::Running;
            })?;
            Ok(task)
        }

        async fn handle_report_progress(
            &self,
            req: ReportProgressReq,
            _ctx: RPCContext,
        ) -> std::result::Result<Task, RPCErrors> {
            let (task, _) = self.with_task(&req.envelope.task_id, |task| {
                if let Some(progress) = req.progress.clone() {
                    task.progress = Some(progress);
                }
                if let Some(message) = req.message.clone() {
                    task.message = Some(message);
                }
            })?;
            Ok(task)
        }

        async fn handle_commit_result(
            &self,
            req: CommitResultReq,
            _ctx: RPCContext,
        ) -> std::result::Result<Task, RPCErrors> {
            let (task, already) = self.with_task(&req.task_id, |task| {
                if task.phase.is_terminal() || task.result.is_some() {
                    return true;
                }
                task.result = Some(req.result.clone());
                task.outcome = Some(TaskOutcome::Succeeded);
                task.phase = TaskPhase::Terminal;
                task.completed_at = Some(Self::now());
                false
            })?;
            if already {
                return Err(RPCErrors::ReasonError(
                    "task_already_completed: mock".to_string(),
                ));
            }
            Ok(task)
        }

        async fn handle_fail_task(
            &self,
            req: buckyos_api::FailTaskReq,
            _ctx: RPCContext,
        ) -> std::result::Result<Task, RPCErrors> {
            let (task, _) = self.with_task(&req.envelope.task_id, |task| {
                task.error = Some(req.error.clone());
                task.outcome = Some(TaskOutcome::Failed);
                task.phase = TaskPhase::Terminal;
                task.completed_at = Some(Self::now());
            })?;
            Ok(task)
        }

        async fn handle_request_control(
            &self,
            req: RequestControlReq,
            _ctx: RPCContext,
        ) -> std::result::Result<RequestControlResult, RPCErrors> {
            let (task, _) = self.with_task(&req.task_id, |task| {
                task.pending_control = Some(TaskControlRequest {
                    request_id: req.request_id.clone(),
                    action: req.action,
                    requested_by: buckyos_api::ActorRef::new("tester", "aicc"),
                    requested_at: Self::now(),
                });
            })?;
            Ok(RequestControlResult::Task { task })
        }

        async fn handle_ack_control(
            &self,
            req: AckControlReq,
            _ctx: RPCContext,
        ) -> std::result::Result<Task, RPCErrors> {
            let (task, _) = self.with_task(&req.envelope.task_id, |task| {
                task.pending_control = None;
                if req.applied {
                    task.outcome = Some(TaskOutcome::Canceled);
                    task.phase = TaskPhase::Terminal;
                    task.completed_at = Some(Self::now());
                }
            })?;
            Ok(task)
        }

        async fn handle_add_task_note(
            &self,
            req: AddTaskNoteReq,
            _ctx: RPCContext,
        ) -> std::result::Result<TaskNote, RPCErrors> {
            let task = self
                .tasks
                .lock()
                .expect("tasks lock")
                .get(&req.task_id)
                .cloned()
                .ok_or_else(|| {
                    RPCErrors::ReasonError(format!("mock task {} not found", req.task_id))
                })?;
            let now = Self::now();
            let mut guard = self.note_counter.lock().expect("note counter lock");
            *guard += 1;
            let note = TaskNote {
                id: *guard as i64,
                task_id: req.task_id.clone(),
                note_type: req.note_type.clone().unwrap_or_else(|| "human".to_string()),
                content: req.content.clone(),
                data: req.data.clone().unwrap_or_else(|| json!({})),
                author_user_id: task.creator.user_id.clone(),
                author_app_id: task.creator.app_id.clone(),
                created_at: now,
                updated_at: now,
            };
            self.notes
                .lock()
                .expect("notes lock")
                .entry(req.task_id.clone())
                .or_default()
                .push(note.clone());
            Ok(note)
        }

        async fn handle_list_task_notes(
            &self,
            req: ListTaskNotesReq,
            _ctx: RPCContext,
        ) -> std::result::Result<Vec<TaskNote>, RPCErrors> {
            Ok(self
                .notes
                .lock()
                .expect("notes lock")
                .get(&req.task_id)
                .cloned()
                .unwrap_or_default())
        }
    }

    #[derive(Debug)]
    struct MockProvider {
        #[allow(dead_code)]
        instance: ProviderInstance,
        inventory: ProviderInventory,
        cost: CostEstimateOutput,
        refresh_inventory: Option<ProviderInventory>,
        refresh_started: Option<Arc<tokio::sync::Notify>>,
        refresh_release: Option<Arc<tokio::sync::Notify>>,
        start_results: Mutex<VecDeque<std::result::Result<ProviderStartResult, ProviderError>>>,
        start_call_count: std::sync::atomic::AtomicUsize,
        shutdown_call_count: std::sync::atomic::AtomicUsize,
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
                refresh_inventory: None,
                refresh_started: None,
                refresh_release: None,
                start_results: Mutex::new(start_results.into_iter().collect()),
                start_call_count: std::sync::atomic::AtomicUsize::new(0),
                shutdown_call_count: std::sync::atomic::AtomicUsize::new(0),
                canceled: Mutex::new(vec![]),
            }
        }

        fn start_calls(&self) -> usize {
            self.start_call_count.load(AtomicOrdering::Relaxed)
        }

        fn shutdown_calls(&self) -> usize {
            self.shutdown_call_count.load(AtomicOrdering::Relaxed)
        }

        fn canceled_ids(&self) -> Vec<String> {
            self.canceled.lock().unwrap().clone()
        }

        fn with_refresh_inventory(mut self, inventory: ProviderInventory) -> Self {
            self.refresh_inventory = Some(inventory);
            self
        }

        fn with_refresh_gate(
            mut self,
            started: Arc<tokio::sync::Notify>,
            release: Arc<tokio::sync::Notify>,
        ) -> Self {
            self.refresh_started = Some(started);
            self.refresh_release = Some(release);
            self
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

        fn shutdown(&self) {
            self.shutdown_call_count
                .fetch_add(1, AtomicOrdering::Relaxed);
        }

        async fn refresh_inventory(&self) -> std::result::Result<ProviderInventory, ProviderError> {
            if let Some(started) = self.refresh_started.as_ref() {
                started.notify_one();
            }
            if let Some(release) = self.refresh_release.as_ref() {
                release.notified().await;
            }
            Ok(self
                .refresh_inventory
                .clone()
                .unwrap_or_else(|| self.inventory.clone()))
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

    #[test]
    fn llm_chat_default_features_include_web_search_once() {
        let mut request = base_request();
        apply_default_features_for_method(ai_methods::LLM_CHAT, &mut request);
        apply_default_features_for_method(ai_methods::LLM_CHAT, &mut request);

        assert!(request.requirements.required.web_search);
        assert!(
            route_policy_from_request(&request)
                .required_features
                .web_search
        );
    }

    #[test]
    fn llm_chat_default_web_search_can_be_disabled() {
        let mut request = base_request();
        request.disable.web_search = true;

        apply_default_features_for_method(ai_methods::LLM_CHAT, &mut request);

        assert!(!request.requirements.required.web_search);
        assert!(
            !route_policy_from_request(&request)
                .required_features
                .web_search
        );
    }

    #[test]
    fn disable_removes_existing_required_feature() {
        let mut request = base_request();
        request
            .requirements
            .set_feature_required(buckyos_api::features::WEB_SEARCH);
        request.disable.web_search = true;

        apply_default_features_for_method(ai_methods::LLM_CHAT, &mut request);

        assert!(!request.requirements.required.web_search);
    }

    #[test]
    fn non_chat_methods_do_not_default_web_search() {
        let mut request = base_request();
        apply_default_features_for_method(ai_methods::EMBEDDING_TEXT, &mut request);

        assert!(!request.requirements.required.web_search);
    }

    #[test]
    fn route_policy_reads_min_context_tokens_from_requirements_extra() {
        let mut request = base_request();
        request.requirements.extra = Some(json!({
            "min_context_tokens": 128000
        }));

        assert_eq!(
            route_policy_from_request(&request)
                .required_features
                .min_context_tokens,
            Some(128000)
        );
    }

    #[test]
    fn route_policy_reads_structured_requirements() {
        let mut request = base_request();
        request.requirements.required.tool_call = true;
        request.requirements.required.json_schema = true;
        request.requirements.required.min_context_tokens = Some(64_000);

        let policy = route_policy_from_request(&request);

        assert!(policy.required_features.tool_call);
        assert!(policy.required_features.json_schema);
        assert_eq!(policy.required_features.min_context_tokens, Some(64_000));
    }

    #[test]
    fn route_policy_reads_request_policy_fields() {
        let mut request = base_request();
        let mut policy = buckyos_api::RoutePolicy::default();
        policy.profile = buckyos_api::RoutePolicyProfile::Fast;
        policy.local_only = true;
        policy.allow_fallback = false;
        policy.runtime_failover = false;
        policy.explain = true;
        policy.allowed_provider_instances = vec!["local-a".to_string()];
        policy.blocked_provider_instances = vec!["cloud-b".to_string()];
        policy.max_cost_usd = Some(0.25);
        policy.max_latency_ms = Some(1500);
        request.policy = Some(policy);

        let route_policy = route_policy_from_request(&request);

        assert_eq!(
            route_policy.profile,
            crate::model_types::SchedulerProfile::LatencyFirst
        );
        assert!(route_policy.local_only);
        assert!(!route_policy.allow_fallback);
        assert!(!route_policy.runtime_failover);
        assert!(route_policy.explain);
        assert_eq!(route_policy.allowed_provider_instances, vec!["local-a"]);
        assert_eq!(route_policy.blocked_provider_instances, vec!["cloud-b"]);
        assert_eq!(route_policy.max_estimated_cost_usd, Some(0.25));
        assert_eq!(route_policy.max_latency_ms, Some(1500));
    }

    #[test]
    fn artifact_output_storage_defaults_to_named_object() {
        let request = base_request();

        assert_eq!(
            requested_artifact_output_storage(&request),
            ArtifactOutputStorage::NamedObject
        );
    }

    #[test]
    fn continuation_uses_existing_task_result_as_truth_source() {
        let task_data = AiccComputeTaskData {
            request: AiccComputeTaskRequest {
                tenant_id: Some("alice".to_string()),
                route: Some(json!({
                    "selected_exact_model": "veo-route@google-main"
                })),
                ..Default::default()
            },
            progress: None,
            result: Some(buckyos_api::AiccComputeTaskResult {
                output: Some(json!({
                    "extra": {
                        "continuation_handle": "provider://continue",
                        "provider_audit": {
                            "aicc_exact_model": "veo-final@google-main"
                        }
                    }
                })),
                provider_output: None,
            }),
            error: None,
        };

        assert_eq!(
            continuation_from_task_data(&task_data, "alice"),
            Some((
                "provider://continue".to_string(),
                "veo-final@google-main".to_string()
            ))
        );
        assert_eq!(continuation_from_task_data(&task_data, "bob"), None);
    }

    #[test]
    fn artifact_output_storage_uses_named_object_for_object_id_request() {
        let mut request = base_request();
        request.payload.input_json = Some(json!({
            "response_format": "object_id",
            "output": {
                "resource_format": "named_object"
            }
        }));

        assert_eq!(
            requested_artifact_output_storage(&request),
            ArtifactOutputStorage::NamedObject
        );
    }

    #[test]
    fn artifact_output_storage_preserves_explicit_base64_request() {
        let mut request = base_request();
        request.payload.input_json = Some(json!({
            "output": {
                "resource_format": "base64"
            }
        }));

        assert_eq!(
            requested_artifact_output_storage(&request),
            ArtifactOutputStorage::InlineBase64
        );
    }

    #[test]
    fn redacted_json_log_trims_inline_base64_payloads() {
        let long_signature = "a".repeat(LOG_BASE64_LIKE_MIN_CHARS);
        let long_base64 = general_purpose::STANDARD.encode(vec![0x5a; LOG_BASE64_LIKE_MIN_CHARS]);
        let logged = redacted_json_log(&json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "inlineData": {
                            "mimeType": "image/jpeg",
                            "data": "abc123"
                        },
                        "thoughtSignature": long_signature
                    }]
                }
            }],
            "data": [{
                "b64_json": "def456"
            }],
            "tool_output": long_base64,
            "image_url": "data:image/png;base64,ghi789"
        }));

        assert!(logged.contains("[redacted_base64] len=6"));
        assert!(logged.contains("[redacted_data_url_base64 mime=image/png len=6]"));
        assert!(logged.contains("[redacted_base64_like_string] len=512"));
        assert!(logged.contains("[redacted_base64_like_string] len=684"));
        assert!(!logged.contains("abc123"));
        assert!(!logged.contains("def456"));
        assert!(!logged.contains("ghi789"));
        assert!(!logged.contains(&"a".repeat(LOG_BASE64_LIKE_MIN_CHARS)));
    }

    #[test]
    fn async_final_task_data_redacts_provider_inline_data() {
        let request = base_request();
        let mut data = build_initial_aicc_task_data(
            &request,
            "aicc-redaction-test",
            None,
            &InvokeCtx::default(),
        );
        let event = TaskEvent {
            task_id: "aicc-redaction-test".to_string(),
            kind: TaskEventKind::Final,
            timestamp_ms: 1,
            data: Some(json!({
                "summary": redacted_summary_value(&AiResponse {
                    extra: Some(json!({
                        "provider_io": {
                            "input": {
                                "contents": [{
                                    "parts": [{
                                        "inlineData": {
                                            "mimeType": "image/jpeg",
                                            "data": "raw-image-base64"
                                        }
                                    }]
                                }]
                            }
                        }
                    })),
                    ..Default::default()
                })
            })),
        };

        merge_task_data_with_event(&mut data, &event);

        let encoded = serde_json::to_string(&data).expect("serialize task data");
        assert!(!encoded.contains("raw-image-base64"));
        assert!(encoded.contains("[redacted_base64] len=16"));
    }

    #[test]
    fn redacted_json_log_hides_plain_multiline_tool_output() {
        let output = [
            "PROJECTS_DIR=/Users/liuzhicong/project",
            "COUNT=39",
            "NAMES_BEGIN",
            ".SynologyWorkingDirectory",
            ".sisyphus",
            "Agent-Spider",
            "BuckyOSApp",
            "OpenDAN",
            "SourceDAO",
            "arozos",
            "bucky-p2p",
            "bucky_backup_suite",
            "buckyos",
            "buckyos-base",
            "buckyos-base-wt-fix-windows-process-flash",
            "buckyos-devkit",
            "buckyos-websdk",
            "buckyos.ai",
            "buckyos_webdesktop",
            "claudecode",
            "cyfs-gateway",
            "cyfs-ndn",
            "demo_desktop",
            "document",
            "filebrowser",
            "forks",
            "gitpot",
            "linux",
            "nfsserve",
            "review",
            "sn-business",
            "temp",
            "test_agent",
            "test_cdp",
            "test_iot",
            "test_unixcrypto",
            "usdb",
            "usdb_doc",
            "vibe-base-ui",
            "vibe-web-base",
            "web3_bridge_data",
            "win",
            "NAMES_END",
        ]
        .join("\n");
        assert!(output.len() >= LOG_BASE64_LIKE_MIN_CHARS);

        let logged = redacted_json_log(&json!({
            "call_id": "call_bXG3pIMWsgSVipAd8XqawO92",
            "output": output
        }));

        assert!(logged.contains(REDACTED_LOG_TEXT_PLACEHOLDER));
        assert!(!logged.contains("PROJECTS_DIR=/Users/liuzhicong/project"));
        assert!(!logged.contains("buckyos-websdk"));
    }

    #[test]
    fn redacted_json_log_hides_prompts_and_tool_arguments() {
        let logged = redacted_json_log(&json!({
            "contents": [{ "parts": [{ "text": "private conversation" }] }],
            "functionCall": {
                "name": "exec_bash",
                "args": { "command": "print secret", "goal": "private goal" }
            },
            "usageMetadata": { "totalTokenCount": 42 }
        }));

        assert!(!logged.contains("private conversation"));
        assert!(!logged.contains("print secret"));
        assert!(!logged.contains("private goal"));
        assert!(logged.contains("exec_bash"));
        assert!(logged.contains("totalTokenCount"));
    }

    fn mock_instance(instance_id: &str, provider_type: &str) -> ProviderInstance {
        ProviderInstance {
            provider_instance_name: instance_id.to_string(),
            provider_type: ProviderType::CloudApi,
            provider_driver: provider_type.to_string(),
            provider_origin: ProviderOrigin::SystemConfig,
            provider_type_trusted_source: ProviderTypeTrustedSource::SystemConfig,
            provider_type_revision: None,
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
            driver_metadata_generation: 0,
            models: vec![provider_model_metadata(
                instance.provider_instance_name.as_str(),
                instance.provider_type.clone(),
                instance.provider_driver.as_str(),
                "gpt-4o-mini",
                ApiType::Llm,
                vec!["llm.plan.default".to_string()],
                &[
                    "plan".to_string(),
                    buckyos_api::features::WEB_SEARCH.to_string(),
                ],
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

    #[test]
    fn registry_stops_displaced_and_removed_providers() {
        let registry = Registry::default();
        let first = Arc::new(MockProvider::new(
            mock_instance("provider-1", "provider-a"),
            cost(0.001, 100),
            vec![],
        ));
        let replacement = Arc::new(MockProvider::new(
            mock_instance("provider-1", "provider-b"),
            cost(0.002, 200),
            vec![],
        ));
        let other = Arc::new(MockProvider::new(
            mock_instance("provider-2", "provider-c"),
            cost(0.003, 300),
            vec![],
        ));

        registry.add_provider(first.clone());
        registry.add_provider(replacement.clone());
        assert_eq!(first.shutdown_calls(), 1);
        assert_eq!(replacement.shutdown_calls(), 0);

        registry.remove_instance("provider-1");
        assert_eq!(replacement.shutdown_calls(), 1);

        registry.add_provider(other.clone());
        registry.clear();
        assert_eq!(other.shutdown_calls(), 1);
    }

    #[test]
    fn refresh_task_shutdown_wins_over_request_completion() {
        let (task, _) = ProviderRefreshTask::new();
        assert!(task.try_start_request());

        task.shutdown();
        task.finish_request();

        assert!(task.is_stopped());
        assert!(!task.try_start_request());
        assert_eq!(task.started_requests(), 1);
    }

    #[tokio::test]
    async fn refresh_all_provider_inventories_applies_metadata_generation_immediately() {
        let instance = mock_instance("provider-1", "provider-a");
        let mut refreshed_inventory = mock_inventory(&instance);
        refreshed_inventory.driver_metadata_generation = 1;
        refreshed_inventory.models[0].logical_mounts = vec!["llm.refreshed".to_string()];
        let provider = Arc::new(
            MockProvider::new(instance, cost(0.001, 100), vec![])
                .with_refresh_inventory(refreshed_inventory),
        );
        let registry = Registry::default();
        registry.add_provider(provider);
        let center = AIComputeCenter::new(registry, ModelCatalog::default());

        let (refreshed, errors) = center.refresh_all_provider_inventories().await;

        assert_eq!(refreshed, 1);
        assert!(errors.is_empty());
        let model_registry = center.model_registry.read().unwrap();
        assert_eq!(
            model_registry.default_items_for_path("llm.refreshed").len(),
            1
        );
        assert!(model_registry
            .default_items_for_path("llm.plan.default")
            .is_empty());
    }

    #[tokio::test]
    async fn refresh_all_provider_inventories_rejects_removed_provider_result() {
        let instance = mock_instance("provider-removed", "provider-a");
        let started = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let provider = Arc::new(
            MockProvider::new(instance, cost(0.001, 100), vec![])
                .with_refresh_gate(started.clone(), release.clone()),
        );
        let registry = Registry::default();
        let inventory = registry.add_provider(provider);
        let center = Arc::new(AIComputeCenter::new(registry, ModelCatalog::default()));
        center
            .model_registry()
            .write()
            .unwrap()
            .apply_inventory(inventory)
            .unwrap();

        let refresh_center = center.clone();
        let refresh =
            tokio::spawn(async move { refresh_center.refresh_all_provider_inventories().await });
        started.notified().await;
        center.registry().remove_instance("provider-removed");
        center.reset_model_routes();
        release.notify_one();

        let (refreshed, errors) = refresh.await.unwrap();
        assert_eq!(refreshed, 0);
        assert!(errors.is_empty());
        assert!(center
            .model_registry()
            .read()
            .unwrap()
            .inventory_revision("provider-removed")
            .is_none());
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
            note_counter: Mutex::new(0),
            tasks: Arc::new(Mutex::new(HashMap::new())),
            notes: Arc::new(Mutex::new(HashMap::new())),
        }));
        center.set_task_manager_client(Arc::new(taskmgr));
        center
    }

    #[test]
    fn task_manager_context_forwards_upstream_token_and_trace() {
        let mut session_token = RPCSessionToken {
            token_type: RPCSessionTokenType::Normal,
            token: Some("test-user-session".to_string()),
            aud: None,
            exp: None,
            iss: Some("verify-hub".to_string()),
            jti: None,
            sub: Some("alice".to_string()),
            appid: None,
            sudo: false,
            extra: HashMap::new(),
        };
        bind_token_principal_kind(&mut session_token, TokenPrincipalKind::User);
        bind_token_target(
            &mut session_token,
            &AuthTarget::app("third-party-app@alice".parse().unwrap()),
            TokenUse::Session,
        )
        .unwrap();
        let raw_token = serde_json::to_string(&session_token).unwrap();
        let upstream = RPCContext {
            token: Some(raw_token),
            trace_id: Some("trace-aicc-task".to_string()),
            ..Default::default()
        };
        let mut invoke_ctx = InvokeCtx {
            tenant_id: "anonymous".to_string(),
            caller_app_id: None,
            session_token: upstream.token.clone(),
            trace_id: upstream.trace_id.clone(),
            task_id: None,
        };
        invoke_ctx.apply_verified_session(&session_token).unwrap();
        let downstream = task_manager_rpc_context(&invoke_ctx);

        assert_eq!(invoke_ctx.tenant_id, "alice");
        assert_eq!(
            invoke_ctx.caller_app_id.as_deref(),
            Some("app:third-party-app@alice")
        );
        assert_eq!(downstream.token, upstream.token);
        assert_eq!(downstream.trace_id, upstream.trace_id);
    }

    #[test]
    fn system_routing_config_is_visible_in_models_list() {
        let center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
        center
            .apply_system_routing_config(&json!({
                "routing_config": {
                    "revision": "sys-routing-rev-1",
                    "provider_weights": {
                        "openai_primary": 0.25
                    },
                    "logical_tree": {
                        "llm": {
                            "children": {
                                "plan": {
                                    "items": {
                                        "fallback": {
                                            "target": "llm.fallback",
                                            "weight": 2.0
                                        }
                                    }
                                }
                            }
                        }
                    },
                    "logical_definitions": [
                        {
                            "path": "audio.asr",
                            "api_type": "audio.asr"
                        }
                    ]
                }
            }))
            .unwrap();

        let directory = center.dump_model_directory().unwrap();
        assert_eq!(
            directory["routing_settings"]["revision"],
            json!("sys-routing-rev-1")
        );
        assert_eq!(
            directory["routing_settings"]["provider_weights"]["openai_primary"],
            json!(0.25)
        );
        assert_eq!(
            directory["directory"]["llm.plan"]["fallback"]["target"],
            json!("llm.fallback")
        );
        assert_eq!(
            directory["directory"]["llm.plan"]["fallback"]["weight"],
            json!(2.0)
        );
        let definitions = directory["logical_definitions"].as_array().unwrap();
        assert!(definitions
            .iter()
            .any(|definition| definition["path"] == json!("audio.asr")));
    }

    #[test]
    fn models_list_preserves_provider_variant_routing_metadata() {
        let registry = Registry::default();
        let instance = mock_instance("openai-main", "openai");
        let mut provider = MockProvider::new(instance, cost(0.001, 100), vec![]);
        provider.inventory.models[0].provider_actual_model_id = Some("gpt-5.6-sol".to_string());
        provider.inventory.models[0].provider_options =
            Some(json!({"reasoning": {"effort": "high"}}));
        registry.add_provider(Arc::new(provider));
        let center = AIComputeCenter::new(registry, ModelCatalog::default());

        let directory = center.dump_model_directory().unwrap();
        let model = &directory["providers"][0]["models"][0];
        assert_eq!(model["provider_actual_model_id"], json!("gpt-5.6-sol"));
        assert_eq!(model["provider_options"]["reasoning"]["effort"], json!("high"));
    }

    #[test]
    fn builtin_logical_tree_is_visible_in_models_list_without_system_config() {
        let center = AIComputeCenter::new(Registry::default(), ModelCatalog::default());
        center.apply_system_routing_config(&json!({})).unwrap();

        let directory = center.dump_model_directory().unwrap();
        assert_eq!(
            directory["directory"]["llm.plan"]["opus"]["target"],
            json!("llm.opus")
        );
        assert_eq!(
            directory["directory"]["llm.plan"]["gemini"]["target"],
            json!("llm.gemini-pro")
        );
        assert_eq!(
            directory["directory"]["llm.code"]["local"]["target"],
            json!("llm.qwen-coder")
        );
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
            vec![Ok(ProviderStartResult::Immediate({
                let mut response = AiResponse::text("ok");
                response.finish_reason = Some("stop".to_string());
                response
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
                        token: Some("user-session-token".to_string()),
                        trace_id: Some("trace-immediate-task".to_string()),
                    },
                    IpAddr::V4(Ipv4Addr::LOCALHOST),
                ),
            )
            .await
            .unwrap();

        assert_eq!(response.status, AiMethodStatus::Succeeded);
        assert_eq!(
            response.result.as_ref().map(|result| result.text_content()),
            Some("ok".to_string())
        );

        let taskmgr = center.taskmgr.as_ref().expect("task manager").clone();
        let tasks = all_tasks(&taskmgr).await;
        let task = tasks
            .into_iter()
            .find(|item| aicc_external_task_id(item).as_deref() == Some(response.task_id.as_str()))
            .expect("immediate provider completion should persist aicc task");
        assert_eq!(task.phase, TaskPhase::Terminal);
        assert_eq!(task.outcome, Some(TaskOutcome::Succeeded));
        assert_eq!(aicc_status(&task).as_deref(), Some("succeeded"));
        // Creator identity comes from the verified session token in 2.0; the
        // create request carries no owner fields at all, and free-form root
        // ids are gone (roots derive from parent links).
        assert_eq!(task.root_id, task.task_id);
        assert!(task.input.get("rootid").is_none());
        assert!(task.input.pointer("/aicc/rootid").is_none());
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
        let task = taskmgr
            .get_task(&response.task_id)
            .await
            .expect("running response should return the task-manager task id");
        assert_eq!(task.phase, TaskPhase::Running);
        assert_eq!(
            task.message.as_deref(),
            Some("request sent, waiting for provider response")
        );
        assert!(aicc_task_data(&task)
            .and_then(|data| data.request.request)
            .is_some());
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
        let task = taskmgr
            .get_task(&response.task_id)
            .await
            .expect("queued response should return the task-manager task id");

        assert_eq!(task.phase, TaskPhase::Accepted);
        assert_eq!(task.message.as_deref(), Some(QUEUE_STATUS_QUEUED));
        assert_eq!(aicc_status(&task).as_deref(), Some("queued"));
        assert_eq!(aicc_event_kind(&task, 0).as_deref(), Some("queued"));
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
            .create_task(CreateTaskReq {
                name: "behavior-parent".to_string(),
                schema_id: AICC_TASK_SCHEMA_ID.to_string(),
                schema_version: None,
                input: json!({"kind":"behavior"}),
                executor: buckyos_api::CreateTaskExecutor::SelfApp {
                    app_instance_id: None,
                },
                parent_id: None,
                child_control_policy: None,
                policy_preset: None,
                permission_boundary: false,
                storage_domain: Some(StorageDomain::System),
                idempotency_key: "behavior-parent".to_string(),
                retry_of: None,
                supersedes: None,
                message: None,
            })
            .await
            .expect("create parent task");
        let request = base_request().with_task_options(Some(AiTaskOptions {
            parent_id: Some(parent_task.task_id.clone()),
        }));
        let response = center
            .complete(request, RPCContext::default())
            .await
            .expect("complete should succeed");
        assert_eq!(response.status, AiMethodStatus::Running);

        let task = taskmgr
            .get_task(&response.task_id)
            .await
            .expect("running response should return the task-manager task id");
        assert_eq!(task.parent_id, Some(parent_task.task_id.clone()));
    }

    #[tokio::test]
    async fn complete_persists_root_id_and_session_id_from_request_options() {
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
        let task = taskmgr
            .get_task(&response.task_id)
            .await
            .expect("running response should return the task-manager task id");
        // Free-form root ids are gone in 2.0; the session id still rides in
        // the immutable request payload.
        assert_eq!(task.root_id, task.task_id);
        assert_eq!(
            aicc_task_data(&task)
                .and_then(|data| data.request.session_id)
                .as_deref(),
            Some("session-xyz")
        );
        assert!(task.input.get("rootid").is_none());
        assert!(task.input.pointer("/aicc/rootid").is_none());
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
                Ok(ProviderStartResult::Immediate({
                    let mut response = AiResponse::text("first");
                    response.cost = Some(buckyos_api::AiCost {
                        amount: 2.0,
                        currency: "USD".to_string(),
                    });
                    response.finish_reason = Some("stop".to_string());
                    response
                })),
                Ok(ProviderStartResult::Immediate({
                    let mut response = AiResponse::text("second");
                    response.cost = Some(buckyos_api::AiCost {
                        amount: 14.0,
                        currency: "USD".to_string(),
                    });
                    response.finish_reason = Some("stop".to_string());
                    response
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
        let tasks = all_tasks(&taskmgr).await;
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

        let start_response = center
            .complete(base_request(), RPCContext::default())
            .await
            .unwrap();
        assert_eq!(start_response.status, AiMethodStatus::Running);
        center
            .task_bindings
            .write()
            .unwrap()
            .values_mut()
            .find(|binding| binding.task_mgr_id == start_response.task_id)
            .expect("task binding")
            .tenant_id = "tenant-alice".to_string();

        let cancel_result = center
            .handle_cancel(start_response.task_id.as_str(), RPCContext::default())
            .await;
        assert!(cancel_result.is_err());
        assert!(matches!(
            cancel_result.unwrap_err(),
            RPCErrors::NoPermission(_)
        ));
    }

    #[tokio::test]
    async fn cancel_with_task_manager_id_uses_provider_task_id() {
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
        registry.add_provider(provider.clone());
        let center = center_with_taskmgr(registry, catalog);
        let ctx = RPCContext {
            token: Some("tenant-alice".to_string()),
            ..Default::default()
        };

        let response = center.complete(base_request(), ctx.clone()).await.unwrap();
        let task = center
            .taskmgr
            .as_ref()
            .unwrap()
            .get_task(&response.task_id)
            .await
            .unwrap();
        let external_task_id = aicc_external_task_id(&task).unwrap();

        let canceled = center
            .handle_cancel(response.task_id.as_str(), ctx)
            .await
            .unwrap();

        assert!(canceled.accepted);
        assert_ne!(external_task_id, response.task_id);
        assert_eq!(provider.canceled_ids(), vec![external_task_id]);
    }
}
