use crate::network::{KLogAppendRequest, KLogDataRequestType, KLogQueryRequest};
use crate::service::{KLogQueryService, KLogWriteService};
use crate::state_store::KLogStateStoreManagerRef;
use crate::{
    KRaftRef,
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
use std::time::Duration;
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
#[derive(Clone)]
struct KRpcServerState {
    write_service: KLogWriteService,
    query_service: KLogQueryService,
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
            write_service: KLogWriteService::new(
                "KRpcServer",
                self.raft.clone(),
                self.state_store_manager.clone(),
            ),
            query_service: KLogQueryService::new(
                "KRpcServer",
                self.raft.clone(),
                self.state_store_manager.clone(),
            ),
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
        match state.write_service.append(&headers, req).await {
            Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
            Err((status, msg)) => Self::error_response(status, msg),
        }
    }

    async fn handle_data_query_request(
        State(state): State<KRpcServerState>,
        Query(query): Query<KLogQueryRequest>,
    ) -> Response {
        match state.query_service.query(query).await {
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

                match state.write_service.append(&headers, params).await {
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

                match state.query_service.query(params).await {
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

    fn error_response(status: StatusCode, msg: String) -> Response {
        (status, msg).into_response()
    }
}
