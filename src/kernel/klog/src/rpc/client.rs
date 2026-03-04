use super::{
    KLOG_JSON_RPC_PATH, KLOG_JSON_RPC_VERSION, KLOG_RPC_METHOD_APPEND, KLOG_RPC_METHOD_QUERY,
    KLogJsonRpcRequest, KLogJsonRpcResponse,
};
use crate::KNode;
use crate::error::{
    KLogErrorCode, KLogErrorEnvelope, generate_trace_id, map_http_status_to_error_code,
    map_json_rpc_error_code_to_klog_error_code, parse_error_envelope_json,
};
use crate::network::{
    KLOG_TRACE_ID_HEADER, KLogAppendRequest, KLogAppendResponse, KLogQueryRequest,
    KLogQueryResponse,
};
use reqwest::StatusCode;
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use uuid::Uuid;

const DEFAULT_RPC_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, thiserror::Error)]
#[error(
    "klog rpc error: endpoint={endpoint}, method={method}, status={http_status:?}, code={error_code:?}, retryable={retryable}, leader_hint={leader_hint:?}, trace_id={trace_id}, message={message}"
)]
pub struct KLogClientError {
    pub endpoint: String,
    pub method: String,
    pub http_status: Option<u16>,
    pub error_code: KLogErrorCode,
    pub message: String,
    pub retryable: bool,
    pub leader_hint: Option<KNode>,
    pub trace_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KLogCallTrace {
    pub trace_id: String,
}

impl KLogClientError {
    fn from_envelope(
        endpoint: &str,
        method: &str,
        http_status: Option<u16>,
        envelope: KLogErrorEnvelope,
    ) -> Self {
        Self {
            endpoint: endpoint.to_string(),
            method: method.to_string(),
            http_status,
            error_code: envelope.error_code,
            message: envelope.message,
            retryable: envelope.retryable,
            leader_hint: envelope.leader_hint,
            trace_id: envelope.trace_id,
        }
    }

    fn internal(endpoint: &str, method: &str, message: impl Into<String>) -> Self {
        let trace_id = generate_trace_id();
        let envelope = KLogErrorEnvelope::new(KLogErrorCode::Internal, message, trace_id);
        Self::from_envelope(endpoint, method, None, envelope)
    }

    pub fn is_retryable(&self) -> bool {
        self.retryable
    }

    pub fn leader_hint(&self) -> Option<&KNode> {
        self.leader_hint.as_ref()
    }
}

pub struct KLogClient {
    endpoint: String,
    client: reqwest::Client,
    next_id: AtomicU64,
    timeout: Duration,
    request_node_id: u64,
}

impl KLogClient {
    pub fn new(endpoint: impl Into<String>, request_node_id: u64) -> Self {
        Self {
            endpoint: normalize_endpoint(endpoint.into()),
            client: reqwest::Client::new(),
            next_id: AtomicU64::new(1),
            timeout: DEFAULT_RPC_TIMEOUT,
            request_node_id,
        }
    }

    pub fn from_daemon_addr(addr: &str, request_node_id: u64) -> Self {
        Self::new(
            format!("http://{}{}", addr, KLOG_JSON_RPC_PATH),
            request_node_id,
        )
    }

    pub fn local_default(request_node_id: u64) -> Self {
        Self::from_daemon_addr("127.0.0.1:21101", request_node_id)
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn generate_request_id(node_id: u64) -> String {
        format!("{}-{}", node_id, Uuid::now_v7())
    }

    pub async fn append(
        &self,
        req: KLogAppendRequest,
    ) -> Result<KLogAppendResponse, KLogClientError> {
        let (resp, _) = self.append_with_trace(req).await?;
        Ok(resp)
    }

    pub async fn append_with_trace(
        &self,
        req: KLogAppendRequest,
    ) -> Result<(KLogAppendResponse, KLogCallTrace), KLogClientError> {
        let req = self.fill_append_defaults(req);
        self.call_with_trace(KLOG_RPC_METHOD_APPEND, &req).await
    }

    pub async fn append_message(&self, message: impl Into<String>) -> Result<u64, KLogClientError> {
        let resp = self
            .append(KLogAppendRequest {
                message: message.into(),
                timestamp: None,
                node_id: None,
                request_id: None,
            })
            .await?;
        Ok(resp.id)
    }

    pub async fn query(&self, req: KLogQueryRequest) -> Result<KLogQueryResponse, KLogClientError> {
        let (resp, _) = self.query_with_trace(req).await?;
        Ok(resp)
    }

    pub async fn query_with_trace(
        &self,
        req: KLogQueryRequest,
    ) -> Result<(KLogQueryResponse, KLogCallTrace), KLogClientError> {
        self.call_with_trace(KLOG_RPC_METHOD_QUERY, &req).await
    }

    async fn call_with_trace<Req, Resp>(
        &self,
        method: &str,
        params: &Req,
    ) -> Result<(Resp, KLogCallTrace), KLogClientError>
    where
        Req: Serialize,
        Resp: for<'de> serde::Deserialize<'de>,
    {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request_trace_id = generate_trace_id();
        let params = serde_json::to_value(params).map_err(|e| {
            KLogClientError::internal(
                self.endpoint.as_str(),
                method,
                format!(
                    "Failed to encode json-rpc params for method {}: {}",
                    method, e
                ),
            )
        })?;
        let request = KLogJsonRpcRequest {
            jsonrpc: KLOG_JSON_RPC_VERSION.to_string(),
            method: method.to_string(),
            params,
            id,
        };

        let response = self
            .client
            .post(self.endpoint.as_str())
            .timeout(self.timeout)
            .header(KLOG_TRACE_ID_HEADER, request_trace_id.clone())
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                KLogClientError::internal(
                    self.endpoint.as_str(),
                    method,
                    format!(
                        "Failed to send json-rpc request: endpoint={}, method={}, id={}, err={}",
                        self.endpoint, method, id, e
                    ),
                )
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|e| format!("<failed to read body: {}>", e));
            let envelope = parse_error_envelope_json(&body).unwrap_or_else(|| {
                let msg = format!(
                    "json-rpc http status not success: endpoint={}, method={}, id={}, status={}, body={}",
                    self.endpoint, method, id, status, body
                );
                KLogErrorEnvelope {
                    error_code: map_http_status_to_error_code(status.as_u16()),
                    message: msg,
                    retryable: map_http_status_to_error_code(status.as_u16()).is_retryable(),
                    leader_hint: None,
                    trace_id: request_trace_id.clone(),
                }
            });
            return Err(KLogClientError::from_envelope(
                self.endpoint.as_str(),
                method,
                Some(status.as_u16()),
                envelope,
            ));
        }

        let response_trace_id = response_trace_id_from_headers(response.headers());
        let payload = response.json::<KLogJsonRpcResponse>().await.map_err(|e| {
            KLogClientError::internal(
                self.endpoint.as_str(),
                method,
                format!(
                    "Failed to decode json-rpc response: endpoint={}, method={}, id={}, err={}",
                    self.endpoint, method, id, e
                ),
            )
        })?;

        if payload.id != id {
            warn!(
                "json-rpc response id mismatch: endpoint={}, method={}, request_id={}, response_id={}",
                self.endpoint, method, id, payload.id
            );
        }

        if let Some(err) = payload.error {
            let envelope = err
                .data
                .and_then(|v| serde_json::from_value::<KLogErrorEnvelope>(v).ok())
                .unwrap_or_else(|| {
                    let error_code = map_json_rpc_error_code_to_klog_error_code(err.code);
                    KLogErrorEnvelope {
                        error_code,
                        message: err.message,
                        retryable: error_code.is_retryable(),
                        leader_hint: None,
                        trace_id: request_trace_id.clone(),
                    }
                });

            return Err(KLogClientError::from_envelope(
                self.endpoint.as_str(),
                method,
                Some(StatusCode::OK.as_u16()),
                envelope,
            ));
        }

        let result = payload.result.ok_or_else(|| {
            KLogClientError::internal(
                self.endpoint.as_str(),
                method,
                format!(
                    "json-rpc missing result: endpoint={}, method={}, id={}",
                    self.endpoint, method, payload.id
                ),
            )
        })?;
        let trace_id = response_trace_id.unwrap_or_else(|| request_trace_id.clone());
        let resp = serde_json::from_value(result).map_err(|e| {
            KLogClientError::internal(
                self.endpoint.as_str(),
                method,
                format!(
                    "Failed to decode json-rpc result: endpoint={}, method={}, id={}, err={}",
                    self.endpoint, method, payload.id, e
                ),
            )
        })?;

        Ok((resp, KLogCallTrace { trace_id }))
    }

    fn fill_append_defaults(&self, mut req: KLogAppendRequest) -> KLogAppendRequest {
        let request_id = req
            .request_id
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string());
        req.request_id = Some(request_id.unwrap_or_else(|| {
            let request_node_id = req.node_id.unwrap_or(self.request_node_id);
            Self::generate_request_id(request_node_id)
        }));
        req
    }
}

fn response_trace_id_from_headers(headers: &reqwest::header::HeaderMap) -> Option<String> {
    headers
        .get(KLOG_TRACE_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
}

fn normalize_endpoint(raw: String) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return trimmed.to_string();
    }
    format!("http://{}", trimmed)
}

#[cfg(test)]
mod tests {
    use super::{KLogClient, normalize_endpoint};
    use crate::KLogEntry;
    use crate::error::KLogErrorCode;
    use crate::network::{
        KLOG_TRACE_ID_HEADER, KLogAppendRequest, KLogAppendResponse, KLogQueryRequest,
        KLogQueryResponse,
    };
    use crate::rpc::{
        KLOG_JSON_RPC_PATH, KLOG_RPC_ERR_METHOD_NOT_FOUND, KLOG_RPC_METHOD_APPEND,
        KLOG_RPC_METHOD_QUERY, KLogJsonRpcRequest, KLogJsonRpcResponse,
    };
    use axum::Router;
    use axum::extract::Json;
    use axum::http::{HeaderMap, HeaderValue, StatusCode};
    use axum::routing::post;
    use std::net::SocketAddr;
    use std::time::Duration;
    use tokio::task::JoinHandle;
    use uuid::Uuid;

    struct TestJsonRpcServer {
        addr: SocketAddr,
        task: JoinHandle<()>,
    }

    impl Drop for TestJsonRpcServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    impl TestJsonRpcServer {
        async fn try_start(app: Router) -> anyhow::Result<Option<Self>> {
            let listener = match tokio::net::TcpListener::bind("127.0.0.1:0").await {
                Ok(listener) => listener,
                Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
                    warn!(
                        "skip json-rpc tests because socket bind is not permitted in this environment: {}",
                        err
                    );
                    return Ok(None);
                }
                Err(err) => return Err(err.into()),
            };

            let addr = listener.local_addr()?;
            let task = tokio::spawn(async move {
                if let Err(err) = axum::serve(listener, app).await {
                    error!("json-rpc test server stopped with error: {}", err);
                }
            });

            Ok(Some(Self { addr, task }))
        }

        fn client(&self) -> KLogClient {
            KLogClient::from_daemon_addr(&self.addr.to_string(), 9)
                .with_timeout(Duration::from_secs(1))
        }
    }

    #[test]
    fn test_normalize_endpoint_with_scheme() {
        assert_eq!(
            normalize_endpoint("http://127.0.0.1:21001/klog/rpc".to_string()),
            "http://127.0.0.1:21001/klog/rpc"
        );
    }

    #[test]
    fn test_normalize_endpoint_without_scheme() {
        assert_eq!(
            normalize_endpoint(format!("127.0.0.1:21001{}", KLOG_JSON_RPC_PATH)),
            "http://127.0.0.1:21001/klog/rpc"
        );
    }

    #[tokio::test]
    async fn test_json_rpc_client_append_success() -> anyhow::Result<()> {
        let app = Router::new().route(
            KLOG_JSON_RPC_PATH,
            post(|Json(request): Json<KLogJsonRpcRequest>| async move {
                assert_eq!(request.method, KLOG_RPC_METHOD_APPEND);
                let params: KLogAppendRequest =
                    serde_json::from_value(request.params).expect("append params");
                assert_eq!(params.message, "hello-klog");

                let response =
                    KLogJsonRpcResponse::success(request.id, KLogAppendResponse { id: 42 });
                (StatusCode::OK, Json(response))
            }),
        );

        let Some(server) = TestJsonRpcServer::try_start(app).await? else {
            return Ok(());
        };
        let client = server.client();
        let resp = client
            .append(KLogAppendRequest {
                message: "hello-klog".to_string(),
                timestamp: Some(1000),
                node_id: Some(1),
                request_id: Some("req-1".to_string()),
            })
            .await
            .map_err(|e| anyhow::anyhow!("append failed: {}", e))?;
        assert_eq!(resp.id, 42);
        Ok(())
    }

    #[tokio::test]
    async fn test_json_rpc_client_append_with_trace_roundtrip() -> anyhow::Result<()> {
        let app = Router::new().route(
            KLOG_JSON_RPC_PATH,
            post(
                |headers: HeaderMap, Json(request): Json<KLogJsonRpcRequest>| async move {
                    assert_eq!(request.method, KLOG_RPC_METHOD_APPEND);
                    let trace_id = headers
                        .get(KLOG_TRACE_ID_HEADER)
                        .and_then(|v| v.to_str().ok())
                        .map(str::trim)
                        .filter(|v| !v.is_empty())
                        .expect("trace id should be present in request header")
                        .to_string();

                    let response =
                        KLogJsonRpcResponse::success(request.id, KLogAppendResponse { id: 77 });
                    (
                        StatusCode::OK,
                        [(
                            KLOG_TRACE_ID_HEADER,
                            HeaderValue::from_str(&trace_id).expect("trace header"),
                        )],
                        Json(response),
                    )
                },
            ),
        );

        let Some(server) = TestJsonRpcServer::try_start(app).await? else {
            return Ok(());
        };
        let client = server.client();
        let (resp, trace) = client
            .append_with_trace(KLogAppendRequest {
                message: "hello-trace".to_string(),
                timestamp: Some(1001),
                node_id: Some(1),
                request_id: Some("req-trace".to_string()),
            })
            .await
            .map_err(|e| anyhow::anyhow!("append_with_trace failed: {}", e))?;
        assert_eq!(resp.id, 77);
        assert!(!trace.trace_id.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn test_json_rpc_client_append_auto_request_id_uses_nodeid_uuid() -> anyhow::Result<()> {
        let app = Router::new().route(
            KLOG_JSON_RPC_PATH,
            post(|Json(request): Json<KLogJsonRpcRequest>| async move {
                assert_eq!(request.method, KLOG_RPC_METHOD_APPEND);
                let params: KLogAppendRequest =
                    serde_json::from_value(request.params).expect("append params");
                let request_id = params.request_id.expect("request_id should be auto-filled");
                let (node_prefix, uuid_part) = request_id
                    .split_once('-')
                    .expect("request_id should be in nodeid-uuid format");
                assert_eq!(node_prefix, "9");
                Uuid::parse_str(uuid_part).expect("uuid part should be valid");

                let response =
                    KLogJsonRpcResponse::success(request.id, KLogAppendResponse { id: 43 });
                (StatusCode::OK, Json(response))
            }),
        );

        let Some(server) = TestJsonRpcServer::try_start(app).await? else {
            return Ok(());
        };
        let client = server.client();
        let resp = client
            .append(KLogAppendRequest {
                message: "auto-request-id".to_string(),
                timestamp: Some(2000),
                node_id: None,
                request_id: None,
            })
            .await
            .map_err(|e| anyhow::anyhow!("append failed: {}", e))?;
        assert_eq!(resp.id, 43);
        Ok(())
    }

    #[tokio::test]
    async fn test_json_rpc_client_query_success() -> anyhow::Result<()> {
        let app = Router::new().route(
            KLOG_JSON_RPC_PATH,
            post(|Json(request): Json<KLogJsonRpcRequest>| async move {
                assert_eq!(request.method, KLOG_RPC_METHOD_QUERY);
                let params: KLogQueryRequest =
                    serde_json::from_value(request.params).expect("query params");
                assert_eq!(params.limit, Some(2));

                let response = KLogJsonRpcResponse::success(
                    request.id,
                    KLogQueryResponse {
                        items: vec![KLogEntry {
                            id: 7,
                            timestamp: 123,
                            node_id: 1,
                            request_id: None,
                            message: "q-result".to_string(),
                        }],
                    },
                );
                (StatusCode::OK, Json(response))
            }),
        );

        let Some(server) = TestJsonRpcServer::try_start(app).await? else {
            return Ok(());
        };
        let client = server.client();
        let resp = client
            .query(KLogQueryRequest {
                start_id: Some(1),
                end_id: Some(9),
                limit: Some(2),
                desc: Some(false),
                strong_read: None,
            })
            .await
            .map_err(|e| anyhow::anyhow!("query failed: {}", e))?;
        assert_eq!(resp.items.len(), 1);
        assert_eq!(resp.items[0].id, 7);
        Ok(())
    }

    #[tokio::test]
    async fn test_json_rpc_client_error_passthrough() -> anyhow::Result<()> {
        let app = Router::new().route(
            KLOG_JSON_RPC_PATH,
            post(|Json(request): Json<KLogJsonRpcRequest>| async move {
                let response = KLogJsonRpcResponse::error(
                    request.id,
                    KLOG_RPC_ERR_METHOD_NOT_FOUND,
                    "Unknown method",
                );
                (StatusCode::OK, Json(response))
            }),
        );

        let Some(server) = TestJsonRpcServer::try_start(app).await? else {
            return Ok(());
        };
        let client = server.client();
        let err = client
            .append_message("should-fail")
            .await
            .expect_err("json-rpc error expected");
        assert_eq!(err.error_code, KLogErrorCode::InvalidArgument);
        assert_eq!(err.message, "Unknown method");
        assert!(!err.retryable);
        assert!(!err.trace_id.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn test_json_rpc_client_http_error_passthrough() -> anyhow::Result<()> {
        let app = Router::new().route(
            KLOG_JSON_RPC_PATH,
            post(|| async { (StatusCode::SERVICE_UNAVAILABLE, "overloaded") }),
        );

        let Some(server) = TestJsonRpcServer::try_start(app).await? else {
            return Ok(());
        };
        let client = server.client();
        let err = client
            .append_message("should-fail-http")
            .await
            .expect_err("http error expected");
        assert_eq!(
            err.http_status,
            Some(StatusCode::SERVICE_UNAVAILABLE.as_u16())
        );
        assert_eq!(err.error_code, KLogErrorCode::Unavailable);
        assert!(err.message.contains("overloaded"));
        assert!(err.retryable);
        Ok(())
    }
}
