#![allow(dead_code)]

use aicc::{
    AIComputeCenter, CostEstimate, InvokeCtx, ModelCatalog, Provider, ProviderError,
    ProviderInstance, ProviderStartResult, Registry, ResolvedRequest, ResourceResolver,
    RouteConfig, TaskEvent, TaskEventSink, TaskEventSinkFactory,
};
use async_trait::async_trait;
use base64::Engine as _;
use buckyos_api::{
    AiPayload, Capability, CompleteRequest, CreateTaskOptions, ModelSpec, Requirements,
    ResourceRef, Task, TaskFilter, TaskManagerClient, TaskManagerHandler, TaskStatus,
};
use kRPC::{RPCContext, RPCErrors, RPCHandler, RPCRequest, RPCResponse};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;
use tokio::net::TcpListener;

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
        instance_id: instance_id.to_string(),
        provider_type: provider_type.to_string(),
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
        Self {
            instance,
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

pub async fn spawn_rpc_http_server(handler: Arc<dyn RPCHandler + Send + Sync>) -> RpcHttpTestServer {
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
    _local_server: Option<RpcHttpTestServer>,
}

impl RpcTestEndpoint {
    pub fn from_remote(endpoint: String) -> Self {
        Self {
            endpoint,
            _local_server: None,
        }
    }

    pub fn from_local(server: RpcHttpTestServer) -> Self {
        Self {
            endpoint: server.endpoint.clone(),
            _local_server: Some(server),
        }
    }
}

pub async fn resolve_rpc_test_endpoint(
    handler: Arc<dyn RPCHandler + Send + Sync>,
) -> RpcTestEndpoint {
    if let Ok(endpoint) = std::env::var("AICC_RPC_TEST_ENDPOINT") {
        let endpoint = endpoint.trim().to_string();
        if !endpoint.is_empty() {
            return RpcTestEndpoint::from_remote(endpoint);
        }
    }

    let server = spawn_rpc_http_server(handler).await;
    RpcTestEndpoint::from_local(server)
}

pub async fn resolve_rpc_gateway_test_endpoint(
    handler: Arc<dyn RPCHandler + Send + Sync>,
) -> RpcTestEndpoint {
    if let Ok(endpoint) = std::env::var("AICC_GATEWAY_RPC_TEST_ENDPOINT") {
        let endpoint = endpoint.trim().to_string();
        if !endpoint.is_empty() {
            return RpcTestEndpoint::from_remote(endpoint);
        }
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
