#![allow(dead_code)]

use aicc::model_types::{
    ApiType, CostEstimateInput, CostEstimateOutput, PricingMode, ProviderInventory, ProviderOrigin,
    ProviderType, ProviderTypeTrustedSource, QuotaState,
};
use aicc::{
    provider_model_metadata, AIComputeCenter, CostEstimate, InvokeCtx, ModelCatalog, Provider,
    ProviderError, ProviderInstance, ProviderStartResult, Registry, ResolvedRequest,
    ResourceResolver, RouteConfig, TaskEvent, TaskEventSink, TaskEventSinkFactory,
};
use async_trait::async_trait;
use base64::Engine as _;
use buckyos_api::{
    AiPayload, Capability, CompleteRequest, CreateTaskOptions, ModelSpec, Requirements,
    ResourceRef, Task, TaskFilter, TaskManagerClient, TaskManagerHandler, TaskStatus,
};
use kRPC::{RPCContext, RPCErrors, RPCHandler, RPCRequest, RPCResponse};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{oneshot, OnceCell};

pub fn base_request() -> CompleteRequest {
    CompleteRequest::new(
        Capability::LlmRouter,
        ModelSpec::new("llm.plan.default".to_string(), None),
        Requirements::new(vec!["plan".to_string()], Some(3000), Some(0.2), None),
        AiPayload::new(
            Some("hello".to_string()),
            vec![],
            vec![],
            vec![],
            None,
            Some(json!({"temperature": 0.2})),
        ),
        Some("idem-test".to_string()),
    )
}

#[allow(dead_code)]
pub fn request_with_resource(resource: ResourceRef) -> CompleteRequest {
    let mut req = base_request();
    req.payload.resources = vec![resource];
    req
}

pub fn base_request_for(capability: Capability, alias: &str) -> CompleteRequest {
    CompleteRequest::new(
        capability,
        ModelSpec::new(alias.to_string(), None),
        Requirements::new(vec!["plan".to_string()], Some(3000), Some(0.2), None),
        AiPayload::new(
            Some("hello".to_string()),
            vec![],
            vec![],
            vec![],
            None,
            None,
        ),
        Some("idem-test".to_string()),
    )
}

pub fn rpc_ctx_with_tenant(tenant: Option<&str>) -> RPCContext {
    RPCContext {
        token: tenant.map(|v| v.to_string()),
        ..Default::default()
    }
}

#[allow(dead_code)]
pub fn mock_instance(
    instance_id: &str,
    provider_type: &str,
    capabilities: Vec<Capability>,
    features: Vec<String>,
) -> ProviderInstance {
    ProviderInstance {
        provider_instance_name: instance_id.to_string(),
        provider_type: ProviderType::CloudApi,
        provider_driver: provider_type.to_string(),
        provider_origin: ProviderOrigin::SystemConfig,
        provider_type_trusted_source: ProviderTypeTrustedSource::SystemConfig,
        provider_type_revision: None,
        capabilities,
        features,
        endpoint: Some("http://127.0.0.1:8080".to_string()),
        plugin_key: None,
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct MockProvider {
    instance: ProviderInstance,
    inventory: ProviderInventory,
    cost: CostEstimate,
    start_results: Mutex<VecDeque<std::result::Result<ProviderStartResult, ProviderError>>>,
    start_calls: AtomicUsize,
    canceled: Mutex<Vec<String>>,
}

impl MockProvider {
    #[allow(dead_code)]
    pub fn new(
        instance: ProviderInstance,
        cost: CostEstimate,
        start_results: Vec<std::result::Result<ProviderStartResult, ProviderError>>,
    ) -> Self {
        let inventory = mock_inventory(&instance, &cost);
        Self {
            instance,
            inventory,
            cost,
            start_results: Mutex::new(start_results.into_iter().collect()),
            start_calls: AtomicUsize::new(0),
            canceled: Mutex::new(vec![]),
        }
    }

    #[allow(dead_code)]
    pub fn start_calls(&self) -> usize {
        self.start_calls.load(Ordering::Relaxed)
    }

    #[allow(dead_code)]
    pub fn canceled_tasks(&self) -> Vec<String> {
        self.canceled.lock().expect("canceled lock").clone()
    }
}

#[async_trait]
impl Provider for MockProvider {
    fn inventory(&self) -> ProviderInventory {
        self.inventory.clone()
    }

    fn legacy_instance(&self) -> Option<&ProviderInstance> {
        Some(&self.instance)
    }

    fn estimate_cost(&self, _input: &CostEstimateInput) -> CostEstimateOutput {
        CostEstimateOutput {
            estimated_cost_usd: self.cost.estimated_cost_usd.unwrap_or(1.0),
            pricing_mode: PricingMode::Unknown,
            quota_state: QuotaState::Unknown,
            confidence: 0.0,
            estimated_latency_ms: self.cost.estimated_latency_ms,
        }
    }

    async fn start(
        &self,
        _ctx: InvokeCtx,
        _provider_model: String,
        _req: ResolvedRequest,
        _sink: Arc<dyn TaskEventSink>,
    ) -> std::result::Result<ProviderStartResult, ProviderError> {
        self.start_calls.fetch_add(1, Ordering::Relaxed);
        let mut queue = self.start_results.lock().expect("start_results lock");
        queue
            .pop_front()
            .unwrap_or_else(|| Err(ProviderError::fatal("no preset start result")))
    }

    async fn cancel(
        &self,
        _ctx: InvokeCtx,
        task_id: &str,
    ) -> std::result::Result<(), ProviderError> {
        self.canceled
            .lock()
            .expect("canceled lock")
            .push(task_id.to_string());
        Ok(())
    }
}

fn mock_inventory(instance: &ProviderInstance, cost: &CostEstimate) -> ProviderInventory {
    let mut models = Vec::new();
    for capability in instance.capabilities.iter() {
        let (api_type, mount, provider_model_id) = match capability {
            Capability::LlmRouter => (ApiType::LlmChat, "llm.plan.default", "m"),
            Capability::Text2Image => (ApiType::ImageTextToImage, "text2image.default", "m"),
            Capability::Image2Text => (ApiType::ImageToImage, "i2t.default", "m"),
            Capability::Voice2Text => (ApiType::LlmCompletion, "v2t.default", "m"),
            Capability::Video2Text => (ApiType::LlmCompletion, "v2t.default", "m"),
            Capability::Text2Voice => (ApiType::LlmCompletion, "t2v.default", "m"),
            Capability::Text2Video => (ApiType::LlmCompletion, "t2v.default", "m"),
        };
        models.push(provider_model_metadata(
            instance.provider_instance_name.as_str(),
            instance.provider_type.clone(),
            provider_model_id,
            api_type,
            vec![mount.to_string()],
            &instance.features,
            cost.estimated_cost_usd,
            cost.estimated_latency_ms,
        ));
    }

    ProviderInventory {
        provider_instance_name: instance.provider_instance_name.clone(),
        provider_type: instance.provider_type.clone(),
        provider_driver: instance.provider_driver.clone(),
        provider_origin: instance.provider_origin.clone(),
        provider_type_trusted_source: instance.provider_type_trusted_source.clone(),
        provider_type_revision: instance.provider_type_revision.clone(),
        version: None,
        inventory_revision: Some("test".to_string()),
        models,
    }
}

#[derive(Default)]
pub struct CollectingSinkFactory {
    events: Arc<Mutex<HashMap<String, Vec<TaskEvent>>>>,
}

impl CollectingSinkFactory {
    pub fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn events_for(&self, task_id: &str) -> Vec<TaskEvent> {
        self.events
            .lock()
            .expect("events lock")
            .get(task_id)
            .cloned()
            .unwrap_or_default()
    }
}

struct CollectingSink {
    task_id: String,
    events: Arc<Mutex<HashMap<String, Vec<TaskEvent>>>>,
}

#[async_trait]
impl TaskEventSink for CollectingSink {
    fn event_ref(&self) -> Option<String> {
        Some(format!("task://{}/events", self.task_id))
    }

    async fn emit(&self, event: TaskEvent) -> std::result::Result<(), RPCErrors> {
        let mut lock = self.events.lock().expect("events lock");
        lock.entry(self.task_id.clone()).or_default().push(event);
        Ok(())
    }
}

impl TaskEventSinkFactory for CollectingSinkFactory {
    fn build(&self, _ctx: &InvokeCtx, task_id: &str) -> Arc<dyn TaskEventSink> {
        Arc::new(CollectingSink {
            task_id: task_id.to_string(),
            events: self.events.clone(),
        })
    }
}

pub struct MockTaskMgrHandler {
    counter: Mutex<u64>,
    tasks: Arc<Mutex<HashMap<i64, Task>>>,
}

impl MockTaskMgrHandler {
    pub fn new() -> Self {
        Self {
            counter: Mutex::new(0),
            tasks: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for MockTaskMgrHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TaskManagerHandler for MockTaskMgrHandler {
    async fn handle_create_task(
        &self,
        name: &str,
        task_type: &str,
        data: Option<Value>,
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
        Ok(self
            .tasks
            .lock()
            .expect("tasks lock")
            .values()
            .cloned()
            .collect())
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
        data: Option<Value>,
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
        data: Value,
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

pub fn center_with_taskmgr(registry: Registry, catalog: ModelCatalog) -> AIComputeCenter {
    let mut center = AIComputeCenter::new(registry, catalog);
    let client = TaskManagerClient::new_in_process(Box::new(MockTaskMgrHandler::new()));
    center.set_task_manager_client(Arc::new(client));
    center
}

pub fn extract_error_code(events: &[TaskEvent]) -> Option<String> {
    events.iter().rev().find_map(|e| {
        if let Some(data) = e.data.as_ref() {
            data.get("code")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string())
        } else {
            None
        }
    })
}

pub struct FailingResolver {
    pub message: String,
}

#[async_trait]
impl ResourceResolver for FailingResolver {
    async fn resolve(
        &self,
        _ctx: &InvokeCtx,
        _req: &CompleteRequest,
    ) -> std::result::Result<ResolvedRequest, RPCErrors> {
        Err(RPCErrors::ReasonError(self.message.clone()))
    }
}

pub struct NoopSink;

#[async_trait]
impl TaskEventSink for NoopSink {
    fn event_ref(&self) -> Option<String> {
        None
    }

    async fn emit(&self, _event: TaskEvent) -> std::result::Result<(), RPCErrors> {
        Ok(())
    }
}

#[derive(Clone)]
pub struct MockHttpReply {
    pub status_code: u16,
    pub body: String,
    pub content_type: &'static str,
    pub delay_ms: u64,
}

pub async fn spawn_fake_http_server(replies: Vec<MockHttpReply>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        for reply in replies {
            let (mut socket, _) = match listener.accept().await {
                Ok(pair) => pair,
                Err(_) => break,
            };
            let mut buf = [0u8; 8192];
            let _ = socket.read(&mut buf).await;
            if reply.delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(reply.delay_ms)).await;
            }
            let response = format!(
                "HTTP/1.1 {} OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                reply.status_code,
                reply.content_type,
                reply.body.len(),
                reply.body
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.shutdown().await;
        }
    });
    format!("http://{}", addr)
}

pub fn openai_b64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

pub fn default_route_cfg() -> RouteConfig {
    RouteConfig::default()
}

pub fn string_set(values: &[&str]) -> HashSet<String> {
    values.iter().map(|v| v.to_string()).collect()
}

pub fn localhost_ctx_from_request() -> RPCContext {
    let req = kRPC::RPCRequest {
        method: "complete".to_string(),
        params: json!({}),
        seq: 1,
        token: None,
        trace_id: None,
    };
    RPCContext::from_request(&req, IpAddr::V4(Ipv4Addr::LOCALHOST))
}

pub struct RpcHttpTestServer {
    pub endpoint: String,
    shutdown: Option<oneshot::Sender<()>>,
}

impl Drop for RpcHttpTestServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

pub async fn spawn_rpc_http_server(
    handler: Arc<dyn RPCHandler + Send + Sync>,
) -> RpcHttpTestServer {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind rpc test server");
    let addr = listener.local_addr().expect("rpc test server local addr");
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => {
                    break;
                }
                accepted = listener.accept() => {
                    let (mut socket, peer_addr) = match accepted {
                        Ok(pair) => pair,
                        Err(_) => break,
                    };
                    let handler = handler.clone();
                    tokio::spawn(async move {
                        let mut buffer = vec![0u8; 16384];
                        let mut total = 0usize;
                        let mut header_end = None;
                        loop {
                            if total >= buffer.len() {
                                break;
                            }
                            let n = match socket.read(&mut buffer[total..]).await {
                                Ok(n) => n,
                                Err(_) => return,
                            };
                            if n == 0 {
                                return;
                            }
                            total += n;
                            if let Some(pos) = find_header_end(&buffer[..total]) {
                                header_end = Some(pos);
                                break;
                            }
                        }

                        let Some(header_end) = header_end else {
                            return;
                        };

                        let header_bytes = &buffer[..header_end];
                        let headers_text = String::from_utf8_lossy(header_bytes);
                        let mut content_length = 0usize;
                        for line in headers_text.lines() {
                            let lower = line.to_ascii_lowercase();
                            if let Some(v) = lower.strip_prefix("content-length:") {
                                content_length = v.trim().parse::<usize>().unwrap_or(0);
                            }
                        }

                        let body_start = header_end + 4;
                        let body_end = body_start.saturating_add(content_length);
                        if body_end > buffer.len() {
                            buffer.resize(body_end, 0);
                        }
                        while total < body_end {
                            let n = match socket.read(&mut buffer[total..]).await {
                                Ok(n) => n,
                                Err(_) => return,
                            };
                            if n == 0 {
                                return;
                            }
                            total += n;
                        }
                        let body = &buffer[body_start..body_end];

                        let parsed_req: std::result::Result<RPCRequest, _> =
                            serde_json::from_slice(body);
                        let response_body = match parsed_req {
                            Ok(req) => match handler.handle_rpc_call(req, peer_addr.ip()).await {
                                Ok(resp) => serde_json::to_string(&resp)
                                    .unwrap_or_else(|e| format!("{{\"error\":\"{}\"}}", e)),
                                Err(e) => serde_json::json!({"error": e.to_string()}).to_string(),
                            },
                            Err(e) => serde_json::json!({"error": format!("invalid rpc request json: {}", e)}).to_string(),
                        };

                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            response_body.len(),
                            response_body
                        );
                        let _ = socket.write_all(response.as_bytes()).await;
                        let _ = socket.shutdown().await;
                    });
                }
            }
        }
    });

    RpcHttpTestServer {
        endpoint: format!("http://{}/kapi/aicc", addr),
        shutdown: Some(shutdown_tx),
    }
}

pub struct RpcTestEndpoint {
    pub endpoint: String,
    pub is_remote: bool,
    _local_server: Option<RpcHttpTestServer>,
}

impl RpcTestEndpoint {
    pub fn from_remote(endpoint: String) -> Self {
        Self {
            endpoint,
            is_remote: true,
            _local_server: None,
        }
    }

    pub fn from_local(server: RpcHttpTestServer) -> Self {
        Self {
            endpoint: server.endpoint.clone(),
            is_remote: false,
            _local_server: Some(server),
        }
    }
}

pub async fn resolve_rpc_test_endpoint(
    handler: Arc<dyn RPCHandler + Send + Sync>,
) -> RpcTestEndpoint {
    if let Some(endpoint) = resolve_krpc_aicc_endpoint_from_env() {
        ensure_remote_mock_provider_bootstrapped(&endpoint)
            .await
            .expect("bootstrap remote mock provider");
        return RpcTestEndpoint::from_remote(endpoint);
    }

    let server = spawn_rpc_http_server(handler).await;
    RpcTestEndpoint::from_local(server)
}

pub async fn resolve_rpc_gateway_test_endpoint(
    handler: Arc<dyn RPCHandler + Send + Sync>,
) -> RpcTestEndpoint {
    if let Some(endpoint) = resolve_gateway_aicc_endpoint_from_env() {
        ensure_remote_mock_provider_bootstrapped(&endpoint)
            .await
            .expect("bootstrap remote mock provider");
        return RpcTestEndpoint::from_remote(endpoint);
    }
    resolve_rpc_test_endpoint(handler).await
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

pub async fn post_rpc_over_http(
    endpoint: &str,
    req: &RPCRequest,
) -> std::result::Result<RPCResponse, String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(endpoint)
        .json(req)
        .send()
        .await
        .map_err(|e| format!("http request failed: {}", e))?;

    let status = resp.status();
    let value: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("decode response json failed: {}", e))?;
    if !status.is_success() {
        return Err(format!("http status {} body {}", status, value));
    }
    if let Some(err) = value.get("error").and_then(|v| v.as_str()) {
        return Err(err.to_string());
    }
    serde_json::from_value(value).map_err(|e| format!("parse rpc response failed: {}", e))
}

static REMOTE_AUTO_OPENAI_MOCK_BASE_URL: OnceCell<std::result::Result<String, String>> =
    OnceCell::const_new();

fn first_non_empty_env(keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        std::env::var(key)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn endpoint_from_host(host: &str, path: &str) -> Option<String> {
    let mut parsed = reqwest::Url::parse(host.trim()).ok()?;
    parsed.set_path(path);
    parsed.set_query(None);
    Some(parsed.to_string())
}

pub fn resolve_endpoint_from_env(
    endpoint_keys: &[&str],
    host_keys: &[&str],
    path: &str,
) -> Option<String> {
    if let Some(endpoint) = first_non_empty_env(endpoint_keys) {
        return Some(endpoint);
    }
    first_non_empty_env(host_keys).and_then(|host| endpoint_from_host(&host, path))
}

pub fn resolve_krpc_aicc_endpoint_from_env() -> Option<String> {
    resolve_endpoint_from_env(&[], &["AICC_KRPC_HOST", "AICC_HOST"], "/kapi/aicc")
}

pub fn resolve_gateway_aicc_endpoint_from_env() -> Option<String> {
    resolve_endpoint_from_env(&[], &["AICC_GATEWAY_HOST", "AICC_HOST"], "/kapi/aicc")
}

pub fn resolve_gateway_system_config_endpoint_from_env() -> Option<String> {
    resolve_gateway_aicc_endpoint_from_env().and_then(|gateway_endpoint| {
        endpoint_from_host(gateway_endpoint.as_str(), "/kapi/system_config")
    })
}

fn derive_verify_hub_endpoint(url: &str) -> Option<String> {
    derive_verify_hub_endpoint_with_path(url, "/kapi/verify-hub")
}

fn derive_verify_hub_endpoint_with_path(url: &str, path: &str) -> Option<String> {
    let mut parsed = reqwest::Url::parse(url).ok()?;
    parsed.set_path(path);
    parsed.set_query(None);
    Some(parsed.to_string())
}

fn derive_system_config_endpoint(url: &str) -> Option<String> {
    let mut parsed = reqwest::Url::parse(url).ok()?;
    parsed.set_path("/kapi/system_config");
    parsed.set_query(None);
    Some(parsed.to_string())
}

fn parse_http_request_path(header_text: &str) -> Option<String> {
    let request_line = header_text.lines().next()?;
    let mut parts = request_line.split_whitespace();
    let _method = parts.next()?;
    let path = parts.next()?.trim();
    if path.is_empty() {
        None
    } else {
        Some(path.to_string())
    }
}

fn build_http_json_response(status_code: u16, value: Value) -> String {
    let body = value.to_string();
    let reason = if status_code == 200 { "OK" } else { "ERR" };
    format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status_code,
        reason,
        body.len(),
        body
    )
}

fn resolve_auto_mock_advertise_host(aicc_endpoint: &str) -> std::result::Result<String, String> {
    if let Some(value) = first_non_empty_env(&["AICC_REMOTE_MOCK_ADVERTISE_HOST"]) {
        return Ok(value);
    }

    let parsed =
        reqwest::Url::parse(aicc_endpoint).map_err(|e| format!("invalid aicc endpoint: {}", e))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| format!("aicc endpoint host is missing: {}", aicc_endpoint))?;
    let port = parsed.port_or_known_default().unwrap_or(80);
    let udp = std::net::UdpSocket::bind("0.0.0.0:0")
        .map_err(|e| format!("bind udp for mock advertise ip failed: {}", e))?;
    udp.connect((host, port))
        .map_err(|e| format!("connect udp for mock advertise ip failed: {}", e))?;
    let local_ip = udp
        .local_addr()
        .map_err(|e| format!("get local udp addr for mock advertise ip failed: {}", e))?
        .ip();
    Ok(local_ip.to_string())
}

async fn start_auto_openai_mock_server(aicc_endpoint: &str) -> std::result::Result<String, String> {
    let bind_addr = first_non_empty_env(&["AICC_REMOTE_MOCK_BIND_ADDR"])
        .unwrap_or_else(|| "0.0.0.0:0".to_string());
    let listener = TcpListener::bind(bind_addr.as_str())
        .await
        .map_err(|e| format!("bind auto openai mock server failed: {}", e))?;
    let addr = listener
        .local_addr()
        .map_err(|e| format!("get auto openai mock server addr failed: {}", e))?;

    tokio::spawn(async move {
        loop {
            let (mut socket, _) = match listener.accept().await {
                Ok(pair) => pair,
                Err(_) => break,
            };
            tokio::spawn(async move {
                let mut buffer = vec![0u8; 16384];
                let mut total = 0usize;
                let mut header_end = None;
                loop {
                    if total >= buffer.len() {
                        break;
                    }
                    let n = match socket.read(&mut buffer[total..]).await {
                        Ok(n) => n,
                        Err(_) => return,
                    };
                    if n == 0 {
                        return;
                    }
                    total += n;
                    if let Some(pos) = find_header_end(&buffer[..total]) {
                        header_end = Some(pos);
                        break;
                    }
                }

                let Some(header_end) = header_end else {
                    return;
                };
                let header_text = String::from_utf8_lossy(&buffer[..header_end]).to_string();
                let path = parse_http_request_path(header_text.as_str()).unwrap_or_default();

                let body = if path.ends_with("/responses") {
                    json!({
                        "id": "mock-response-1",
                        "status": "completed",
                        "output_text": "mock-ok",
                        "usage": {
                            "input_tokens": 1,
                            "output_tokens": 1,
                            "total_tokens": 2
                        }
                    })
                } else if path.ends_with("/images/generations") {
                    json!({
                        "id": "mock-image-1",
                        "data": [{
                            "url": "https://example.invalid/mock-image.png",
                            "revised_prompt": "mock image"
                        }]
                    })
                } else {
                    json!({
                        "error": {
                            "message": format!("mock openai unsupported path: {}", path),
                            "code": "mock_unsupported_path"
                        }
                    })
                };
                let status =
                    if path.ends_with("/responses") || path.ends_with("/images/generations") {
                        200
                    } else {
                        404
                    };
                let response = build_http_json_response(status, body);
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });

    let advertise_host = resolve_auto_mock_advertise_host(aicc_endpoint)?;
    Ok(format!("http://{}:{}/v1", advertise_host, addr.port()))
}

async fn resolve_mock_openai_base_url(aicc_endpoint: &str) -> std::result::Result<String, String> {
    if let Some(value) = resolve_endpoint_from_env(&[], &["AICC_REMOTE_MOCK_OPENAI_HOST"], "/v1") {
        return Ok(value);
    }

    let result = REMOTE_AUTO_OPENAI_MOCK_BASE_URL
        .get_or_init(|| async { start_auto_openai_mock_server(aicc_endpoint).await })
        .await;
    result.clone()
}

async fn build_remote_mock_openai_settings(
    aicc_endpoint: &str,
) -> std::result::Result<Value, String> {
    let base_url = resolve_mock_openai_base_url(aicc_endpoint).await?;
    let api_token = "mock-token".to_string();
    let model = "mock-chat".to_string();
    let provider_type = "openai-mock".to_string();
    let instance_id = "openai-mock-remote".to_string();
    let timeout_ms = 5000_u64;

    Ok(json!({
        "openai": {
            "enabled": true,
            "api_token": api_token,
            "instances": [{
                "instance_id": instance_id,
                "provider_type": provider_type,
                "base_url": base_url,
                "timeout_ms": timeout_ms,
                "models": [model.clone()],
                "default_model": model.clone(),
                "features": ["plan"]
            }],
            "alias_map": {
                "llm.default": model.clone(),
                "llm.chat.default": model.clone(),
                "llm.plan.default": model.clone(),
                "llm.code.default": model
            }
        }
    }))
}

async fn bootstrap_remote_mock_provider_once(
    aicc_endpoint: &str,
) -> std::result::Result<(), String> {
    let settings = build_remote_mock_openai_settings(aicc_endpoint).await?;

    let sys_endpoint = derive_system_config_endpoint(aicc_endpoint).ok_or_else(|| {
        format!(
            "cannot derive system_config endpoint from aicc endpoint {}",
            aicc_endpoint
        )
    })?;

    let token = resolve_remote_sys_config_token(Some(&sys_endpoint)).await?;
    let seq = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    post_rpc_over_http(
        &sys_endpoint,
        &RPCRequest {
            method: "sys_config_set".to_string(),
            params: json!({
                "key": "services/aicc/settings",
                "value": settings.to_string()
            }),
            seq,
            token: token.clone(),
            trace_id: Some("aicc-tests-bootstrap-settings".to_string()),
        },
    )
    .await
    .map_err(|err| {
        format!(
            "bootstrap sys_config_set failed via {}: {}",
            sys_endpoint, err
        )
    })?;

    post_rpc_over_http(
        aicc_endpoint,
        &RPCRequest {
            method: "service.reload_settings".to_string(),
            params: json!({}),
            seq: seq.saturating_add(1),
            token,
            trace_id: Some("aicc-tests-bootstrap-reload".to_string()),
        },
    )
    .await
    .map_err(|err| {
        format!(
            "bootstrap reload_settings failed via {}: {}",
            aicc_endpoint, err
        )
    })?;

    Ok(())
}

async fn ensure_remote_mock_provider_bootstrapped(
    aicc_endpoint: &str,
) -> std::result::Result<(), String> {
    bootstrap_remote_mock_provider_once(aicc_endpoint).await
}

fn derive_login_password_hash(username: &str, password: &str, login_nonce: u64) -> String {
    let stage1 = {
        let mut hasher = Sha256::new();
        hasher.update(format!("{}{}.buckyos", password, username).as_bytes());
        base64::engine::general_purpose::STANDARD.encode(hasher.finalize())
    };

    let mut hasher = Sha256::new();
    hasher.update(format!("{}{}", stage1, login_nonce).as_bytes());
    base64::engine::general_purpose::STANDARD.encode(hasher.finalize())
}

fn resolve_login_endpoint(endpoint_hint: Option<&str>) -> std::result::Result<String, String> {
    if let Some(login_url) = resolve_endpoint_from_env(&["AICC_VERIFY_HUB_URL"], &[], "") {
        return Ok(login_url);
    }

    if let Some(endpoint) = endpoint_hint.and_then(|value| derive_verify_hub_endpoint(value.trim()))
    {
        return Ok(endpoint);
    }

    if let Some(endpoint) = endpoint_hint
        .and_then(|value| derive_verify_hub_endpoint_with_path(value.trim(), "/kapi/verify_hub"))
    {
        return Ok(endpoint);
    }

    if let Some(login_url) = resolve_endpoint_from_env(
        &[],
        &["AICC_GATEWAY_HOST", "AICC_KRPC_HOST", "AICC_HOST"],
        "/kapi/verify-hub",
    ) {
        return Ok(login_url);
    }

    if let Some(login_url) = resolve_endpoint_from_env(
        &[],
        &["AICC_GATEWAY_HOST", "AICC_KRPC_HOST", "AICC_HOST"],
        "/kapi/verify_hub",
    ) {
        return Ok(login_url);
    }

    if let Some(endpoint_seed) =
        first_non_empty_env(&["AICC_GATEWAY_HOST", "AICC_KRPC_HOST", "AICC_HOST"])
    {
        if let Some(endpoint) = derive_verify_hub_endpoint(&endpoint_seed) {
            return Ok(endpoint);
        }
        if let Some(endpoint) =
            derive_verify_hub_endpoint_with_path(&endpoint_seed, "/kapi/verify_hub")
        {
            return Ok(endpoint);
        }
    }

    Err(
        "cannot resolve verify-hub login endpoint; set AICC_VERIFY_HUB_URL or AICC_GATEWAY_HOST/AICC_KRPC_HOST/AICC_HOST"
            .to_string(),
    )
}

async fn resolve_remote_sys_config_token(
    endpoint_hint: Option<&str>,
) -> std::result::Result<Option<String>, String> {
    if let Some(token) = first_non_empty_env(&["AICC_SYS_CONFIG_RPC_TOKEN"]) {
        return Ok(Some(token));
    }
    resolve_remote_test_token(endpoint_hint).await
}

async fn login_remote_token_once(
    endpoint_hint: Option<&str>,
) -> std::result::Result<Option<String>, String> {
    if let Some(token) = first_non_empty_env(&["AICC_RPC_TOKEN"]) {
        return Ok(Some(token));
    }

    let username = match first_non_empty_env(&["AICC_LOGIN_USERNAME"]) {
        Some(value) => value,
        None => return Ok(None),
    };
    let password = match first_non_empty_env(&["AICC_LOGIN_PASSWORD"]) {
        Some(value) => value,
        None => return Ok(None),
    };

    let appid = first_non_empty_env(&["AICC_LOGIN_APPID"]).unwrap_or_else(|| "aicc-tests".into());
    let login_endpoint = resolve_login_endpoint(endpoint_hint)?;
    let seq = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let login_nonce = seq;
    let password_hash = derive_login_password_hash(&username, &password, login_nonce);

    let login_resp = post_rpc_over_http(
        &login_endpoint,
        &RPCRequest {
            method: "login_by_password".to_string(),
            params: json!({
                "type": "password",
                "username": username,
                "password": password_hash,
                "appid": appid,
                "login_nonce": login_nonce,
            }),
            seq,
            token: None,
            trace_id: Some("aicc-tests-login".to_string()),
        },
    )
    .await
    .map_err(|err| format!("login_by_password failed via {}: {}", login_endpoint, err))?;

    let payload = match login_resp.result {
        kRPC::RPCResult::Success(value) => value,
        other => {
            return Err(format!(
                "login_by_password failed, unexpected rpc result: {:?}",
                other
            ))
        }
    };

    let token = payload
        .get("session_token")
        .and_then(|v| v.as_str())
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            format!(
                "login_by_password response missing session_token: {}",
                payload
            )
        })?;

    Ok(Some(token.to_string()))
}

pub async fn resolve_remote_test_token(
    endpoint_hint: Option<&str>,
) -> std::result::Result<Option<String>, String> {
    login_remote_token_once(endpoint_hint).await
}
