use crate::network::{
    KDataClient, KLOG_FORWARD_HOPS_HEADER, KLOG_FORWARDED_BY_HEADER, KLogAppendRequest,
    KLogAppendResponse, KLogDataRequestType, KLogQueryRequest, KLogQueryResponse,
};
use crate::state_store::{KLogQuery, KLogQueryOrder, KLogStateStoreManagerRef};
use crate::{
    KLogEntry, KLogRequest, KLogResponse, KNode, KRaftRef,
    rpc::{
        KLOG_JSON_RPC_PATH, KLOG_JSON_RPC_VERSION, KLOG_RPC_ERR_INTERNAL,
        KLOG_RPC_ERR_INVALID_PARAMS, KLOG_RPC_ERR_INVALID_REQUEST, KLOG_RPC_ERR_METHOD_NOT_FOUND,
        KLOG_RPC_METHOD_APPEND, KLOG_RPC_METHOD_QUERY, KLogJsonRpcRequest, KLogJsonRpcResponse,
    },
};
use axum::Json;
use axum::Router;
use axum::error_handling::HandleErrorLayer;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use std::future::Future;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tower::BoxError;
use tower::ServiceBuilder;
use tower::limit::ConcurrencyLimitLayer;
use tower::load_shed::LoadShedLayer;
use tower::timeout::TimeoutLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

const RPC_BODY_LIMIT_BYTES: usize = 1 * 1024 * 1024;
const RPC_CONCURRENCY_LIMIT: usize = 128;
const RPC_TIMEOUT_MS: u64 = 3_000;
const DATA_QUERY_DEFAULT_LIMIT: usize = 200;
const DATA_QUERY_MAX_LIMIT: usize = 2_000;
const DATA_APPEND_MAX_MESSAGE_BYTES: usize = 64 * 1024;
const DATA_APPEND_MAX_FORWARD_HOPS: u32 = 2;

#[derive(Clone)]
struct KRpcServerState {
    raft: KRaftRef,
    state_store_manager: KLogStateStoreManagerRef,
    data_client: KDataClient,
}

pub struct KRpcServer {
    addr: String,
    raft: KRaftRef,
    state_store_manager: KLogStateStoreManagerRef,
}

impl KRpcServer {
    pub fn new(
        addr: String,
        raft: KRaftRef,
        state_store_manager: KLogStateStoreManagerRef,
    ) -> Self {
        Self {
            addr,
            raft,
            state_store_manager,
        }
    }

    pub async fn run(&self) -> Result<(), String> {
        self.run_with_shutdown(std::future::pending::<()>()).await
    }

    pub async fn run_with_shutdown<F>(&self, shutdown: F) -> Result<(), String>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let state = KRpcServerState {
            raft: self.raft.clone(),
            state_store_manager: self.state_store_manager.clone(),
            data_client: KDataClient::new(),
        };

        let rpc_middleware = ServiceBuilder::new()
            .layer(HandleErrorLayer::new(Self::handle_middleware_error))
            .layer(LoadShedLayer::new())
            .layer(ConcurrencyLimitLayer::new(RPC_CONCURRENCY_LIMIT))
            .layer(TimeoutLayer::new(Duration::from_millis(RPC_TIMEOUT_MS)))
            .layer(RequestBodyLimitLayer::new(RPC_BODY_LIMIT_BYTES));

        let data_append_path = KLogDataRequestType::Append.klog_path();
        let data_query_path = KLogDataRequestType::Query.klog_path();
        let app = Router::new()
            .route(&data_append_path, post(Self::handle_data_append_request))
            .route(&data_query_path, get(Self::handle_data_query_request))
            .route(KLOG_JSON_RPC_PATH, post(Self::handle_json_rpc_request))
            .route_layer(rpc_middleware)
            .layer(TraceLayer::new_for_http())
            .with_state(state);

        info!(
            "KRpcServer start listening at {}, rpc_limit_bytes={}, rpc_concurrency={}, rpc_timeout_ms={}, data_append_path={}, data_query_path={}, json_rpc_path={}",
            self.addr,
            RPC_BODY_LIMIT_BYTES,
            RPC_CONCURRENCY_LIMIT,
            RPC_TIMEOUT_MS,
            data_append_path,
            data_query_path,
            KLOG_JSON_RPC_PATH
        );

        let listener = tokio::net::TcpListener::bind(&self.addr)
            .await
            .map_err(|e| {
                let msg = format!("KRpcServer bind failed at {}: {}", self.addr, e);
                error!("{}", msg);
                msg
            })?;

        let addr = self.addr.clone();
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                shutdown.await;
                info!(
                    "KRpcServer shutdown signal received at {}, stop accepting new connections and draining in-flight requests",
                    addr
                );
            })
            .await
            .map_err(|e| {
                let msg = format!("KRpcServer serve failed at {}: {}", self.addr, e);
                error!("{}", msg);
                msg
            })
    }

    async fn handle_middleware_error(err: BoxError) -> Response {
        let msg = format!("KRpcServer middleware rejected request: {}", err);
        error!("{}", msg);

        if err.is::<tower::timeout::error::Elapsed>() {
            return Self::error_response(StatusCode::REQUEST_TIMEOUT, msg);
        }
        if err.is::<tower::load_shed::error::Overloaded>() {
            return Self::error_response(StatusCode::SERVICE_UNAVAILABLE, msg);
        }
        Self::error_response(StatusCode::INTERNAL_SERVER_ERROR, msg)
    }

    async fn handle_data_append_request(
        State(state): State<KRpcServerState>,
        headers: HeaderMap,
        Json(req): Json<KLogAppendRequest>,
    ) -> Response {
        match Self::process_data_append(&state, &headers, req).await {
            Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
            Err((status, msg)) => Self::error_response(status, msg),
        }
    }

    async fn handle_data_query_request(
        State(state): State<KRpcServerState>,
        Query(query): Query<KLogQueryRequest>,
    ) -> Response {
        match Self::process_data_query(&state, query).await {
            Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
            Err((status, msg)) => Self::error_response(status, msg),
        }
    }

    async fn handle_json_rpc_request(
        State(state): State<KRpcServerState>,
        headers: HeaderMap,
        Json(request): Json<KLogJsonRpcRequest>,
    ) -> Response {
        let req_id = request.id;
        if request.jsonrpc != KLOG_JSON_RPC_VERSION {
            let resp = KLogJsonRpcResponse::error(
                req_id,
                KLOG_RPC_ERR_INVALID_REQUEST,
                format!(
                    "Invalid jsonrpc version: expected={}, got={}",
                    KLOG_JSON_RPC_VERSION, request.jsonrpc
                ),
            );
            return (StatusCode::OK, Json(resp)).into_response();
        }

        match request.method.as_str() {
            KLOG_RPC_METHOD_APPEND => {
                let params: KLogAppendRequest = match serde_json::from_value(request.params) {
                    Ok(params) => params,
                    Err(e) => {
                        let resp = KLogJsonRpcResponse::error(
                            req_id,
                            KLOG_RPC_ERR_INVALID_PARAMS,
                            format!("Invalid params for {}: {}", KLOG_RPC_METHOD_APPEND, e),
                        );
                        return (StatusCode::OK, Json(resp)).into_response();
                    }
                };

                match Self::process_data_append(&state, &headers, params).await {
                    Ok(result) => (
                        StatusCode::OK,
                        Json(KLogJsonRpcResponse::success(req_id, result)),
                    )
                        .into_response(),
                    Err((status, msg)) => (
                        StatusCode::OK,
                        Json(KLogJsonRpcResponse::error(
                            req_id,
                            Self::rpc_error_code_from_status(status),
                            msg,
                        )),
                    )
                        .into_response(),
                }
            }
            KLOG_RPC_METHOD_QUERY => {
                let params: KLogQueryRequest = if request.params.is_null() {
                    KLogQueryRequest::default()
                } else {
                    match serde_json::from_value(request.params) {
                        Ok(params) => params,
                        Err(e) => {
                            let resp = KLogJsonRpcResponse::error(
                                req_id,
                                KLOG_RPC_ERR_INVALID_PARAMS,
                                format!("Invalid params for {}: {}", KLOG_RPC_METHOD_QUERY, e),
                            );
                            return (StatusCode::OK, Json(resp)).into_response();
                        }
                    }
                };

                match Self::process_data_query(&state, params).await {
                    Ok(result) => (
                        StatusCode::OK,
                        Json(KLogJsonRpcResponse::success(req_id, result)),
                    )
                        .into_response(),
                    Err((status, msg)) => (
                        StatusCode::OK,
                        Json(KLogJsonRpcResponse::error(
                            req_id,
                            Self::rpc_error_code_from_status(status),
                            msg,
                        )),
                    )
                        .into_response(),
                }
            }
            _ => (
                StatusCode::OK,
                Json(KLogJsonRpcResponse::error(
                    req_id,
                    KLOG_RPC_ERR_METHOD_NOT_FOUND,
                    format!("Unknown method: {}", request.method),
                )),
            )
                .into_response(),
        }
    }

    async fn process_data_append(
        state: &KRpcServerState,
        headers: &HeaderMap,
        req: KLogAppendRequest,
    ) -> Result<KLogAppendResponse, (StatusCode, String)> {
        if req.message.trim().is_empty() {
            let msg = "KRpcServer data append rejected: empty message".to_string();
            error!("{}", msg);
            return Err((StatusCode::BAD_REQUEST, msg));
        }

        if req.message.len() > DATA_APPEND_MAX_MESSAGE_BYTES {
            let msg = format!(
                "KRpcServer data append rejected: message too large, bytes={}, max_bytes={}",
                req.message.len(),
                DATA_APPEND_MAX_MESSAGE_BYTES
            );
            error!("{}", msg);
            return Err((StatusCode::PAYLOAD_TOO_LARGE, msg));
        }

        let forward_hops = Self::parse_forward_hops(headers).map_err(|msg| {
            error!("{}", msg);
            (StatusCode::BAD_REQUEST, msg)
        })?;
        let forwarded_by = headers
            .get(KLOG_FORWARDED_BY_HEADER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("-");

        if forward_hops > DATA_APPEND_MAX_FORWARD_HOPS {
            let msg = format!(
                "KRpcServer data append rejected: too many forward hops, hops={}, max_hops={}, forwarded_by={}",
                forward_hops, DATA_APPEND_MAX_FORWARD_HOPS, forwarded_by
            );
            error!("{}", msg);
            return Err((StatusCode::BAD_GATEWAY, msg));
        }

        let metrics = state.raft.metrics().borrow().clone();
        let local_node_id = metrics.id;
        let req = KLogAppendRequest {
            message: req.message,
            timestamp: req.timestamp.or_else(|| Some(Self::now_millis())),
            node_id: req.node_id.or(Some(local_node_id)),
        };

        let item = state.state_store_manager.prepare_append_entry(KLogEntry {
            id: 0,
            timestamp: req.timestamp.unwrap_or(0),
            node_id: req.node_id.unwrap_or(local_node_id),
            message: req.message.clone(),
        });
        let requested_id = item.id;

        info!(
            "KRpcServer data append request: id={}, timestamp={}, node_id={}, msg_len={}, local_node_id={}, current_leader={:?}, forward_hops={}, forwarded_by={}",
            item.id,
            item.timestamp,
            item.node_id,
            item.message.len(),
            local_node_id,
            metrics.current_leader,
            forward_hops,
            forwarded_by
        );

        match state
            .raft
            .client_write(KLogRequest::AppendLog { item })
            .await
        {
            Ok(resp) => match resp.data {
                KLogResponse::AppendOk { id } => {
                    info!("KRpcServer data append committed: id={}", id);
                    Ok(KLogAppendResponse { id })
                }
                KLogResponse::Err(err_msg) => {
                    let msg = format!(
                        "KRpcServer data append failed in state machine: requested_id={}, err={}",
                        requested_id, err_msg
                    );
                    error!("{}", msg);
                    Err((StatusCode::INTERNAL_SERVER_ERROR, msg))
                }
                other => {
                    let msg = format!(
                        "KRpcServer data append unexpected response: requested_id={}, response={:?}",
                        requested_id, other
                    );
                    error!("{}", msg);
                    Err((StatusCode::INTERNAL_SERVER_ERROR, msg))
                }
            },
            Err(err) => {
                if let Some(forward) = err.forward_to_leader::<KNode>() {
                    if forward_hops >= DATA_APPEND_MAX_FORWARD_HOPS {
                        let msg = format!(
                            "KRpcServer data append forward aborted due to hop limit: local_node_id={}, requested_id={}, leader_id={:?}, leader_node={:?}, hops={}, max_hops={}",
                            local_node_id,
                            requested_id,
                            forward.leader_id,
                            forward.leader_node,
                            forward_hops,
                            DATA_APPEND_MAX_FORWARD_HOPS
                        );
                        error!("{}", msg);
                        return Err((StatusCode::BAD_GATEWAY, msg));
                    }

                    let leader_node = forward.leader_node.clone().or_else(|| {
                        forward.leader_id.and_then(|leader_id| {
                            metrics
                                .membership_config
                                .nodes()
                                .find_map(|(id, node)| (*id == leader_id).then_some(node.clone()))
                        })
                    });
                    let Some(leader_node) = leader_node else {
                        let msg = format!(
                            "KRpcServer data append can not resolve leader node for forwarding: local_node_id={}, requested_id={}, leader_id={:?}",
                            local_node_id, requested_id, forward.leader_id
                        );
                        warn!("{}", msg);
                        return Err((StatusCode::SERVICE_UNAVAILABLE, msg));
                    };

                    let target_hops = forward_hops + 1;
                    warn!(
                        "KRpcServer data append forwarding to leader: local_node_id={}, requested_id={}, leader_id={}, leader_addr={}:{}, leader_rpc_port={}, hops={} -> {}",
                        local_node_id,
                        requested_id,
                        leader_node.id,
                        leader_node.addr,
                        leader_node.port,
                        if leader_node.rpc_port == 0 {
                            leader_node.port
                        } else {
                            leader_node.rpc_port
                        },
                        forward_hops,
                        target_hops
                    );
                    match state
                        .data_client
                        .append_to_node(&leader_node, &req, target_hops, local_node_id)
                        .await
                    {
                        Ok(resp) => {
                            info!(
                                "KRpcServer data append forwarded and committed: local_node_id={}, requested_id={}, committed_id={}, leader_id={}, hops={}",
                                local_node_id, requested_id, resp.id, leader_node.id, target_hops
                            );
                            Ok(resp)
                        }
                        Err(forward_err) => {
                            let msg = format!(
                                "KRpcServer data append forward failed: local_node_id={}, requested_id={}, leader_id={}, err={}",
                                local_node_id, requested_id, leader_node.id, forward_err
                            );
                            error!("{}", msg);
                            Err((StatusCode::BAD_GATEWAY, msg))
                        }
                    }
                } else {
                    let msg = format!(
                        "KRpcServer data append raft client_write failed: requested_id={}, err={}",
                        requested_id, err
                    );
                    error!("{}", msg);
                    Err((StatusCode::INTERNAL_SERVER_ERROR, msg))
                }
            }
        }
    }

    async fn process_data_query(
        state: &KRpcServerState,
        query: KLogQueryRequest,
    ) -> Result<KLogQueryResponse, (StatusCode, String)> {
        if let (Some(start_id), Some(end_id)) = (query.start_id, query.end_id)
            && start_id > end_id
        {
            let msg = format!(
                "KRpcServer data query invalid range: start_id={} > end_id={}",
                start_id, end_id
            );
            error!("{}", msg);
            return Err((StatusCode::BAD_REQUEST, msg));
        }

        let limit = query.limit.unwrap_or(DATA_QUERY_DEFAULT_LIMIT);
        if limit == 0 || limit > DATA_QUERY_MAX_LIMIT {
            let msg = format!(
                "KRpcServer data query invalid limit: limit={}, allowed=1..={}",
                limit, DATA_QUERY_MAX_LIMIT
            );
            error!("{}", msg);
            return Err((StatusCode::BAD_REQUEST, msg));
        }

        let order = if query.desc.unwrap_or(false) {
            KLogQueryOrder::Desc
        } else {
            KLogQueryOrder::Asc
        };
        info!(
            "KRpcServer data query request: start_id={:?}, end_id={:?}, limit={}, order={:?}",
            query.start_id, query.end_id, limit, order
        );

        let entries = state
            .state_store_manager
            .query_entries(KLogQuery {
                start_id: query.start_id,
                end_id: query.end_id,
                limit,
                order,
            })
            .await
            .map_err(|e| {
                let msg = format!("KRpcServer data query failed: {}", e);
                error!("{}", msg);
                (StatusCode::INTERNAL_SERVER_ERROR, msg)
            })?;

        info!("KRpcServer data query response: items={}", entries.len());
        Ok(KLogQueryResponse { items: entries })
    }

    fn rpc_error_code_from_status(status: StatusCode) -> i64 {
        if status == StatusCode::BAD_REQUEST || status == StatusCode::PAYLOAD_TOO_LARGE {
            KLOG_RPC_ERR_INVALID_PARAMS
        } else if status == StatusCode::NOT_FOUND {
            KLOG_RPC_ERR_METHOD_NOT_FOUND
        } else if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            KLOG_RPC_ERR_INVALID_REQUEST
        } else {
            KLOG_RPC_ERR_INTERNAL
        }
    }

    fn parse_forward_hops(headers: &HeaderMap) -> Result<u32, String> {
        let Some(raw) = headers.get(KLOG_FORWARD_HOPS_HEADER) else {
            return Ok(0);
        };
        let raw = raw.to_str().map_err(|e| {
            format!(
                "KRpcServer data append invalid {} header utf8: {}",
                KLOG_FORWARD_HOPS_HEADER, e
            )
        })?;
        raw.parse::<u32>().map_err(|e| {
            format!(
                "KRpcServer data append invalid {} header '{}': {}",
                KLOG_FORWARD_HOPS_HEADER, raw, e
            )
        })
    }

    fn now_millis() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    fn error_response(status: StatusCode, msg: String) -> Response {
        (status, msg).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::KRpcServer;
    use crate::network::KLOG_FORWARD_HOPS_HEADER;
    use axum::http::HeaderMap;

    #[test]
    fn test_parse_forward_hops_default_zero() {
        let headers = HeaderMap::new();
        let hops = KRpcServer::parse_forward_hops(&headers).expect("parse hops");
        assert_eq!(hops, 0);
    }

    #[test]
    fn test_parse_forward_hops_ok() {
        let mut headers = HeaderMap::new();
        headers.insert(KLOG_FORWARD_HOPS_HEADER, "2".parse().expect("header value"));
        let hops = KRpcServer::parse_forward_hops(&headers).expect("parse hops");
        assert_eq!(hops, 2);
    }

    #[test]
    fn test_parse_forward_hops_invalid_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert(
            KLOG_FORWARD_HOPS_HEADER,
            "abc".parse().expect("header value"),
        );
        let err = KRpcServer::parse_forward_hops(&headers).expect_err("invalid hops");
        assert!(err.contains("invalid"));
    }
}
