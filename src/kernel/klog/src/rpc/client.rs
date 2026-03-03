use super::{
    KLOG_JSON_RPC_PATH, KLOG_JSON_RPC_VERSION, KLOG_RPC_METHOD_APPEND, KLOG_RPC_METHOD_QUERY,
    KLogJsonRpcRequest, KLogJsonRpcResponse,
};
use crate::network::{KLogAppendRequest, KLogAppendResponse, KLogQueryRequest, KLogQueryResponse};
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const DEFAULT_RPC_TIMEOUT: Duration = Duration::from_secs(3);

pub struct KLogClient {
    endpoint: String,
    client: reqwest::Client,
    next_id: AtomicU64,
    timeout: Duration,
}

impl KLogClient {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: normalize_endpoint(endpoint.into()),
            client: reqwest::Client::new(),
            next_id: AtomicU64::new(1),
            timeout: DEFAULT_RPC_TIMEOUT,
        }
    }

    pub fn from_daemon_addr(addr: &str) -> Self {
        Self::new(format!("http://{}{}", addr, KLOG_JSON_RPC_PATH))
    }

    pub fn local_default() -> Self {
        Self::from_daemon_addr("127.0.0.1:21101")
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub async fn append(&self, req: KLogAppendRequest) -> Result<KLogAppendResponse, String> {
        self.call(KLOG_RPC_METHOD_APPEND, &req).await
    }

    pub async fn append_message(&self, message: impl Into<String>) -> Result<u64, String> {
        let resp = self
            .append(KLogAppendRequest {
                message: message.into(),
                timestamp: None,
                node_id: None,
            })
            .await?;
        Ok(resp.id)
    }

    pub async fn query(&self, req: KLogQueryRequest) -> Result<KLogQueryResponse, String> {
        self.call(KLOG_RPC_METHOD_QUERY, &req).await
    }

    async fn call<Req, Resp>(&self, method: &str, params: &Req) -> Result<Resp, String>
    where
        Req: Serialize,
        Resp: for<'de> serde::Deserialize<'de>,
    {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let params = serde_json::to_value(params).map_err(|e| {
            format!(
                "Failed to encode json-rpc params for method {}: {}",
                method, e
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
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                format!(
                    "Failed to send json-rpc request: endpoint={}, method={}, id={}, err={}",
                    self.endpoint, method, id, e
                )
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|e| format!("<failed to read body: {}>", e));
            return Err(format!(
                "json-rpc http status not success: endpoint={}, method={}, id={}, status={}, body={}",
                self.endpoint, method, id, status, body
            ));
        }

        let payload = response.json::<KLogJsonRpcResponse>().await.map_err(|e| {
            format!(
                "Failed to decode json-rpc response: endpoint={}, method={}, id={}, err={}",
                self.endpoint, method, id, e
            )
        })?;

        if payload.id != id {
            warn!(
                "json-rpc response id mismatch: endpoint={}, method={}, request_id={}, response_id={}",
                self.endpoint, method, id, payload.id
            );
        }

        if let Some(err) = payload.error {
            return Err(format!(
                "json-rpc error: endpoint={}, method={}, id={}, code={}, message={}",
                self.endpoint, method, payload.id, err.code, err.message
            ));
        }

        let result = payload.result.ok_or_else(|| {
            format!(
                "json-rpc missing result: endpoint={}, method={}, id={}",
                self.endpoint, method, payload.id
            )
        })?;
        serde_json::from_value(result).map_err(|e| {
            format!(
                "Failed to decode json-rpc result: endpoint={}, method={}, id={}, err={}",
                self.endpoint, method, payload.id, e
            )
        })
    }
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
    use crate::network::{
        KLogAppendRequest, KLogAppendResponse, KLogQueryRequest, KLogQueryResponse,
    };
    use crate::rpc::{
        KLOG_JSON_RPC_PATH, KLOG_RPC_ERR_METHOD_NOT_FOUND, KLOG_RPC_METHOD_APPEND,
        KLOG_RPC_METHOD_QUERY, KLogJsonRpcRequest, KLogJsonRpcResponse,
    };
    use axum::Router;
    use axum::extract::Json;
    use axum::http::StatusCode;
    use axum::routing::post;
    use std::net::SocketAddr;
    use std::time::Duration;
    use tokio::task::JoinHandle;

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
            KLogClient::from_daemon_addr(&self.addr.to_string())
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
            })
            .await
            .map_err(|e| anyhow::anyhow!("append failed: {}", e))?;
        assert_eq!(resp.id, 42);
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
        assert!(err.contains("code=-32601"));
        assert!(err.contains("Unknown method"));
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
        assert!(err.contains("status=503 Service Unavailable"));
        assert!(err.contains("overloaded"));
        Ok(())
    }
}
