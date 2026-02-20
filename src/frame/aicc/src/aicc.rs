use crate::complete_request_queue::QUEUE_STATUS_QUEUED;
use ::kRPC::*;
use async_trait::async_trait;
use base64::engine::general_purpose;
use base64::Engine as _;
use buckyos_api::{
    AiResponseSummary, AiccHandler, CancelResponse, Capability, CompleteRequest, CompleteResponse,
    CompleteStatus, CreateTaskOptions, Feature, ResourceRef, TaskManagerClient, TaskStatus,
    AICC_SERVICE_SERVICE_NAME,
};
use log::{info, warn};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex as AsyncMutex;

const DEFAULT_FALLBACK_LIMIT: usize = 2;
const DEFAULT_BASE64_MAX_BYTES: usize = 8 * 1024 * 1024;
const EWMA_ALPHA: f64 = 0.2;
const AICC_TASK_TYPE: &str = "aicc.compute";
const AICC_TASK_EVENT_RETENTION: usize = 64;

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

#[derive(Clone, Debug)]
pub struct ProviderInstance {
    pub instance_id: String,
    pub provider_type: String,
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
    pub request: CompleteRequest,
}

impl ResolvedRequest {
    pub fn new(request: CompleteRequest) -> Self {
        Self { request }
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
        request: &CompleteRequest,
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
        request: &CompleteRequest,
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
        req: &CompleteRequest,
    ) -> std::result::Result<ResolvedRequest, RPCErrors>;
}

#[derive(Default)]
pub struct PassthroughResourceResolver;

#[async_trait]
impl ResourceResolver for PassthroughResourceResolver {
    async fn resolve(
        &self,
        _ctx: &InvokeCtx,
        req: &CompleteRequest,
    ) -> std::result::Result<ResolvedRequest, RPCErrors> {
        Ok(ResolvedRequest::new(req.clone()))
    }
}

#[async_trait]
pub trait Provider: Send + Sync {
    fn instance(&self) -> &ProviderInstance;
    fn estimate_cost(&self, req: &CompleteRequest, provider_model: &str) -> CostEstimate;
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
    instance: ProviderInstance,
    provider: Arc<dyn Provider>,
    metrics: ProviderMetrics,
}

#[derive(Clone, Debug)]
pub struct RegistryCandidate {
    pub instance: ProviderInstance,
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
    pub fn add_provider(&self, provider: Arc<dyn Provider>) {
        let instance = provider.instance().clone();
        let mut entries = self
            .entries
            .write()
            .expect("registry lock should be available");
        entries.insert(
            instance.instance_id.clone(),
            ProviderEntry {
                instance,
                provider,
                metrics: ProviderMetrics::default(),
            },
        );
    }

    pub fn remove_instance(&self, instance_id: &str) {
        let mut entries = self
            .entries
            .write()
            .expect("registry lock should be available");
        entries.remove(instance_id);
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
            .filter(|entry| entry.instance.supports_capability(&capability))
            .map(|entry| RegistryCandidate {
                instance: entry.instance.clone(),
                metrics: entry.metrics.clone(),
            })
            .collect::<Vec<_>>();

        RegistrySnapshot { candidates }
    }

    pub fn get_provider(&self, instance_id: &str) -> Option<Arc<dyn Provider>> {
        let entries = self.entries.read().ok()?;
        entries.get(instance_id).map(|entry| entry.provider.clone())
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
}

#[derive(Clone, Debug)]
struct RouteAttempt {
    instance_id: String,
    provider_model: String,
}

#[derive(Clone, Debug)]
pub struct RouteDecision {
    pub primary_instance_id: String,
    pub fallback_instance_ids: Vec<String>,
    pub provider_model: String,
    attempts: Vec<RouteAttempt>,
}

impl RouteDecision {
    fn attempts(&self) -> &[RouteAttempt] {
        &self.attempts
    }
}

#[derive(Clone, Default)]
pub struct Router;

impl Router {
    pub fn route(
        &self,
        tenant_id: &str,
        req: &CompleteRequest,
        snapshot: &RegistrySnapshot,
        registry: &Registry,
        route_cfg: &RouteConfig,
        model_catalog: &ModelCatalog,
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

        for candidate in snapshot.candidates.iter() {
            if !candidate
                .instance
                .supports_features(&req.requirements.must_features)
            {
                continue;
            }

            if let Some(allow) = allow_set.as_ref() {
                if !allow.contains(candidate.instance.provider_type.as_str()) {
                    continue;
                }
            }
            if let Some(deny) = deny_set.as_ref() {
                if deny.contains(candidate.instance.provider_type.as_str()) {
                    continue;
                }
            }

            let provider_model = model_catalog.resolve(
                tenant_id,
                &req.capability,
                req.model.alias.as_str(),
                candidate.instance.provider_type.as_str(),
            );
            let Some(provider_model) = provider_model else {
                continue;
            };

            alias_mapped = true;
            let Some(provider) = registry.get_provider(candidate.instance.instance_id.as_str())
            else {
                continue;
            };

            let estimate = provider.estimate_cost(req, provider_model.as_str());
            if let Some(max_cost) = req.requirements.max_cost_usd {
                if let Some(estimated_cost) = estimate.estimated_cost_usd {
                    if estimated_cost > max_cost {
                        continue;
                    }
                }
            }

            let predicted_latency_ms = if candidate.metrics.ewma_latency_ms > 0.0 {
                candidate.metrics.ewma_latency_ms
            } else {
                estimate.estimated_latency_ms.unwrap_or(0) as f64
            };

            if let Some(max_latency_ms) = req.requirements.max_latency_ms {
                if predicted_latency_ms > max_latency_ms as f64 {
                    continue;
                }
            }

            scored.push(ScoredRouteCandidate {
                instance_id: candidate.instance.instance_id.clone(),
                provider_model,
                cost: estimate.estimated_cost_usd.unwrap_or(1.0).max(0.0),
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
        })
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
    model_catalog: ModelCatalog,
    resource_resolver: Arc<dyn ResourceResolver>,
    sink_factory: Arc<dyn TaskEventSinkFactory>,
    taskmgr: Option<Arc<TaskManagerClient>>,
    task_bindings: Arc<RwLock<HashMap<String, TaskBinding>>>,
    task_id_seq: AtomicU64,
    base64_max_bytes: usize,
    base64_mime_allowlist: HashSet<String>,
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

        Self {
            registry,
            router: Router,
            route_cfg: Arc::new(RwLock::new(RouteConfig::default())),
            model_catalog,
            resource_resolver: Arc::new(PassthroughResourceResolver),
            sink_factory: Arc::new(DefaultTaskEventSinkFactory),
            taskmgr: None,
            task_bindings: Arc::new(RwLock::new(HashMap::new())),
            task_id_seq: AtomicU64::new(1),
            base64_max_bytes: DEFAULT_BASE64_MAX_BYTES,
            base64_mime_allowlist,
        }
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    pub fn model_catalog(&self) -> &ModelCatalog {
        &self.model_catalog
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

    pub fn set_base64_policy(&mut self, max_bytes: usize, mime_allowlist: HashSet<String>) {
        self.base64_max_bytes = max_bytes;
        self.base64_mime_allowlist = mime_allowlist;
    }

    pub async fn complete(
        &self,
        request: CompleteRequest,
        rpc_ctx: RPCContext,
    ) -> std::result::Result<CompleteResponse, RPCErrors> {
        let invoke_ctx = InvokeCtx::from_rpc(&rpc_ctx);
        info!(
            "aicc.complete received: tenant={} caller_app={:?} capability={:?} model_alias={} idempotency_key={:?}",
            invoke_ctx.tenant_id,
            invoke_ctx.caller_app_id,
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
            warn!(
                "aicc.complete bad_request: task_id={} tenant={} err={}",
                external_task_id, invoke_ctx.tenant_id, error
            );
            self.emit_task_error(
                event_sink.clone(),
                external_task_id.as_str(),
                "bad_request",
                error.to_string(),
            )
            .await;
            return Ok(CompleteResponse::new(
                external_task_id,
                CompleteStatus::Failed,
                None,
                event_ref,
            ));
        }

        let snapshot = self.registry.snapshot(request.capability.clone());
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
            snapshot.candidates.len(),
            request.requirements.must_features,
            request.requirements.max_cost_usd,
            request.requirements.max_latency_ms
        );

        let decision = match self.router.route(
            invoke_ctx.tenant_id.as_str(),
            &request,
            &snapshot,
            &self.registry,
            &route_cfg,
            &self.model_catalog,
        ) {
            Ok(result) => result,
            Err(error) => {
                warn!(
                    "aicc.routing failed: task_id={} tenant={} capability={:?} model_alias={} providers={} err={}",
                    external_task_id,
                    invoke_ctx.tenant_id,
                    request.capability,
                    request.model.alias,
                    snapshot.candidates.len(),
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
                return Ok(CompleteResponse::new(
                    external_task_id,
                    CompleteStatus::Failed,
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

        let resolved = match self.resource_resolver.resolve(&invoke_ctx, &request).await {
            Ok(result) => result,
            Err(error) => {
                self.emit_task_error(
                    event_sink.clone(),
                    external_task_id.as_str(),
                    "resource_invalid",
                    error.to_string(),
                )
                .await;
                return Ok(CompleteResponse::new(
                    external_task_id,
                    CompleteStatus::Failed,
                    None,
                    event_ref,
                ));
            }
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
                Ok(CompleteResponse::new(
                    external_task_id,
                    CompleteStatus::Succeeded,
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
                Ok(CompleteResponse::new(
                    external_task_id,
                    CompleteStatus::Running,
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
                Ok(CompleteResponse::new(
                    external_task_id,
                    CompleteStatus::Running,
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
                Ok(CompleteResponse::new(
                    external_task_id,
                    CompleteStatus::Failed,
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
        let request_log = serde_json::to_string(&req.request)
            .unwrap_or_else(|err| format!("{{\"serialize_error\":\"{}\"}}", err));
        info!(
            "aicc.llm.input task_id={} tenant={} trace_id={:?} request={}",
            task_id, ctx.tenant_id, ctx.trace_id, request_log
        );

        for attempt in decision.attempts() {
            let provider = self.registry.get_provider(attempt.instance_id.as_str());
            let Some(provider) = provider else {
                continue;
            };
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
                Ok(start_result) => {
                    self.registry
                        .record_start_success(attempt.instance_id.as_str(), elapsed_ms);
                    match &start_result {
                        ProviderStartResult::Immediate(summary) => {
                            let summary_log =
                                serde_json::to_string(summary).unwrap_or_else(|err| {
                                    format!("{{\"serialize_error\":\"{}\"}}", err)
                                });
                            info!(
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
                    if !error.is_retryable() {
                        break;
                    }
                }
            }
        }

        let reason = last_err
            .map(|error| format!("provider start failed for task {}: {}", task_id, error))
            .unwrap_or_else(|| format!("provider start failed for task {}: no candidate", task_id));
        Err(reason_error("provider_start_failed", reason))
    }

    fn validate_request(&self, req: &CompleteRequest) -> std::result::Result<(), RPCErrors> {
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
        let event = TaskEvent {
            task_id: task_id.to_string(),
            kind: TaskEventKind::Final,
            timestamp_ms: now_ms(),
            data: Some(json!({
                "summary": summary,
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
    async fn handle_complete(
        &self,
        request: CompleteRequest,
        ctx: RPCContext,
    ) -> std::result::Result<CompleteResponse, RPCErrors> {
        self.complete(request, ctx).await
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

fn extract_session_id_from_complete_request(request: &CompleteRequest) -> Option<String> {
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

fn extract_rootid_from_complete_request(request: &CompleteRequest) -> Option<String> {
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
    request: &CompleteRequest,
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
        AiPayload, CompleteTaskOptions, CreateTaskOptions, ModelSpec, Requirements, Task,
        TaskFilter, TaskManagerClient, TaskManagerHandler, TaskPermissions, TaskStatus,
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
                permissions: opts.permissions.unwrap_or(TaskPermissions::default()),
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
        cost: CostEstimate,
        start_results: Mutex<VecDeque<std::result::Result<ProviderStartResult, ProviderError>>>,
        canceled: Mutex<Vec<String>>,
    }

    impl MockProvider {
        fn new(
            instance: ProviderInstance,
            cost: CostEstimate,
            start_results: Vec<std::result::Result<ProviderStartResult, ProviderError>>,
        ) -> Self {
            Self {
                instance,
                cost,
                start_results: Mutex::new(start_results.into_iter().collect()),
                canceled: Mutex::new(vec![]),
            }
        }
    }

    #[async_trait]
    impl Provider for MockProvider {
        fn instance(&self) -> &ProviderInstance {
            &self.instance
        }

        fn estimate_cost(&self, _req: &CompleteRequest, _provider_model: &str) -> CostEstimate {
            self.cost.clone()
        }

        async fn start(
            &self,
            _ctx: InvokeCtx,
            _provider_model: String,
            _req: ResolvedRequest,
            _sink: Arc<dyn TaskEventSink>,
        ) -> std::result::Result<ProviderStartResult, ProviderError> {
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

    fn base_request() -> CompleteRequest {
        CompleteRequest::new(
            Capability::LlmRouter,
            ModelSpec::new("llm.plan.default".to_string(), None),
            Requirements::new(vec!["plan".to_string()], Some(3000), Some(0.1), None),
            AiPayload::new(
                Some("hello".to_string()),
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
            instance_id: instance_id.to_string(),
            provider_type: provider_type.to_string(),
            capabilities: vec![Capability::LlmRouter],
            features: vec!["plan".to_string()],
            endpoint: Some("http://127.0.0.1:8080".to_string()),
            plugin_key: None,
        }
    }

    fn center_with_taskmgr(registry: Registry, catalog: ModelCatalog) -> AIComputeCenter {
        let mut center = AIComputeCenter::new(registry, catalog);
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
            Capability::LlmRouter,
            "llm.plan.default",
            "provider-a",
            "gpt-4o-mini",
        );

        let provider = Arc::new(MockProvider::new(
            mock_instance("provider-a-1", "provider-a"),
            CostEstimate {
                estimated_cost_usd: Some(0.001),
                estimated_latency_ms: Some(200),
            },
            vec![Ok(ProviderStartResult::Immediate(AiResponseSummary {
                text: Some("ok".to_string()),
                json: None,
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
            .handle_complete(
                base_request(),
                RPCContext::from_request(
                    &RPCRequest {
                        method: "complete".to_string(),
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

        assert_eq!(response.status, CompleteStatus::Succeeded);
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
            Capability::LlmRouter,
            "llm.plan.default",
            "provider-a",
            "gpt-4o-mini",
        );
        catalog.set_mapping(
            Capability::LlmRouter,
            "llm.plan.default",
            "provider-b",
            "gpt-4.1-mini",
        );

        let p1 = Arc::new(MockProvider::new(
            mock_instance("provider-a-1", "provider-a"),
            CostEstimate {
                estimated_cost_usd: Some(0.001),
                estimated_latency_ms: Some(100),
            },
            vec![Err(ProviderError::retryable(
                "upstream temporary unavailable",
            ))],
        ));
        let p2 = Arc::new(MockProvider::new(
            mock_instance("provider-b-1", "provider-b"),
            CostEstimate {
                estimated_cost_usd: Some(0.002),
                estimated_latency_ms: Some(250),
            },
            vec![Ok(ProviderStartResult::Started)],
        ));
        registry.add_provider(p1);
        registry.add_provider(p2);

        let center = center_with_taskmgr(registry, catalog);
        let response = center
            .handle_complete(base_request(), RPCContext::default())
            .await
            .unwrap();

        assert_eq!(response.status, CompleteStatus::Running);
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
            Capability::LlmRouter,
            "llm.plan.default",
            "provider-a",
            "gpt-4o-mini",
        );

        let provider = Arc::new(MockProvider::new(
            mock_instance("provider-a-1", "provider-a"),
            CostEstimate {
                estimated_cost_usd: Some(0.001),
                estimated_latency_ms: Some(100),
            },
            vec![Ok(ProviderStartResult::Queued { position: 3 })],
        ));
        registry.add_provider(provider);

        let center = center_with_taskmgr(registry, catalog);
        let response = center
            .handle_complete(base_request(), RPCContext::default())
            .await
            .expect("complete should return queued task");
        assert_eq!(response.status, CompleteStatus::Running);

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
            Capability::LlmRouter,
            "llm.plan.default",
            "provider-a",
            "gpt-4o-mini",
        );
        let provider = Arc::new(MockProvider::new(
            mock_instance("provider-a-1", "provider-a"),
            CostEstimate {
                estimated_cost_usd: Some(0.001),
                estimated_latency_ms: Some(200),
            },
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
        let request = base_request().with_task_options(Some(CompleteTaskOptions {
            parent_id: Some(parent_task.id),
        }));
        let response = center
            .handle_complete(request, RPCContext::default())
            .await
            .expect("complete should succeed");
        assert_eq!(response.status, CompleteStatus::Running);

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
            Capability::LlmRouter,
            "llm.plan.default",
            "provider-a",
            "gpt-4o-mini",
        );
        let provider = Arc::new(MockProvider::new(
            mock_instance("provider-a-1", "provider-a"),
            CostEstimate {
                estimated_cost_usd: Some(0.001),
                estimated_latency_ms: Some(120),
            },
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
            .handle_complete(request, RPCContext::default())
            .await
            .expect("complete should succeed");
        assert_eq!(response.status, CompleteStatus::Running);

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
    async fn complete_no_provider_does_not_create_task() {
        let registry = Registry::default();
        let catalog = ModelCatalog::default();
        let center = center_with_taskmgr(registry, catalog);

        let response = center
            .handle_complete(base_request(), RPCContext::default())
            .await
            .expect("complete should return failed response");
        assert_eq!(response.status, CompleteStatus::Failed);

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
            Capability::LlmRouter,
            "llm.plan.default",
            "provider-a",
            "gpt-4o-mini",
        );

        let provider = Arc::new(MockProvider::new(
            mock_instance("provider-a-1", "provider-a"),
            CostEstimate {
                estimated_cost_usd: Some(0.001),
                estimated_latency_ms: Some(100),
            },
            vec![Ok(ProviderStartResult::Started)],
        ));
        registry.add_provider(provider);

        let center = center_with_taskmgr(registry, catalog);

        let mut alice_ctx = RPCContext::default();
        alice_ctx.token = Some("tenant-alice".to_string());
        let start_response = center
            .handle_complete(base_request(), alice_ctx)
            .await
            .unwrap();
        assert_eq!(start_response.status, CompleteStatus::Running);

        let mut bob_ctx = RPCContext::default();
        bob_ctx.token = Some("tenant-bob".to_string());
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
