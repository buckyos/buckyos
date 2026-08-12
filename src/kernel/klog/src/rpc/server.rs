use crate::error::{
    KLogErrorCode, KLogErrorEnvelope, KLogServiceError, generate_trace_id, normalize_trace_id,
};
use crate::network::{
    KLOG_TRACE_ID_HEADER, KLogAppendRequest, KLogDataRequestType, KLogMetaDeleteRequest,
    KLogMetaPutRequest, KLogMetaQueryRequest, KLogQueryRequest,
};
use crate::service::{KLogQueryService, KLogWriteService};
use crate::state_store::KLogStateStoreManagerRef;
use crate::{
    KRaftRef,
    rpc::{
        KLOG_JSON_RPC_PATH, KLOG_JSON_RPC_VERSION, KLOG_RPC_ERR_INTERNAL,
        KLOG_RPC_ERR_INVALID_PARAMS, KLOG_RPC_ERR_INVALID_REQUEST, KLOG_RPC_ERR_METHOD_NOT_FOUND,
        KLOG_RPC_METHOD_LOG_APPEND, KLOG_RPC_METHOD_LOG_APPEND_LEGACY, KLOG_RPC_METHOD_LOG_QUERY,
        KLOG_RPC_METHOD_LOG_QUERY_LEGACY, KLOG_RPC_METHOD_META_DELETE, KLOG_RPC_METHOD_META_PUT,
        KLOG_RPC_METHOD_META_QUERY, KLogJsonRpcRequest, KLogJsonRpcResponse,
    },
};
use axum::Json;
use axum::Router;
use axum::error_handling::HandleErrorLayer;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
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

const DEFAULT_RPC_BODY_LIMIT_BYTES: usize = 1 * 1024 * 1024;
const DEFAULT_RPC_CONCURRENCY_LIMIT: usize = 128;
const DEFAULT_RPC_TIMEOUT_MS: u64 = 3_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KRpcRoutePolicy {
    pub timeout_ms: u64,
    pub body_limit_bytes: usize,
    pub concurrency: usize,
}

impl KRpcRoutePolicy {
    fn validate(self, route_name: &str) -> Result<(), String> {
        if self.timeout_ms == 0 {
            let msg = format!(
                "Invalid rpc {} policy: timeout_ms must be > 0, got {}",
                route_name, self.timeout_ms
            );
            error!("{}", msg);
            return Err(msg);
        }
        if self.body_limit_bytes == 0 {
            let msg = format!(
                "Invalid rpc {} policy: body_limit_bytes must be > 0, got {}",
                route_name, self.body_limit_bytes
            );
            error!("{}", msg);
            return Err(msg);
        }
        if self.concurrency == 0 {
            let msg = format!(
                "Invalid rpc {} policy: concurrency must be > 0, got {}",
                route_name, self.concurrency
            );
            error!("{}", msg);
            return Err(msg);
        }
        Ok(())
    }
}

impl Default for KRpcRoutePolicy {
    fn default() -> Self {
        Self {
            timeout_ms: DEFAULT_RPC_TIMEOUT_MS,
            body_limit_bytes: DEFAULT_RPC_BODY_LIMIT_BYTES,
            concurrency: DEFAULT_RPC_CONCURRENCY_LIMIT,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KRpcServerPolicy {
    pub append: KRpcRoutePolicy,
    pub query: KRpcRoutePolicy,
    pub jsonrpc: KRpcRoutePolicy,
}
#[derive(Clone)]
struct KRpcServerState {
    write_service: KLogWriteService,
    query_service: KLogQueryService,
}

pub struct KRpcServer {
    addr: String,
    raft: KRaftRef,
    state_store_manager: KLogStateStoreManagerRef,
    policy: KRpcServerPolicy,
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
            policy: KRpcServerPolicy::default(),
        }
    }

    pub fn with_policy(mut self, policy: KRpcServerPolicy) -> Self {
        self.policy = policy;
        self
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

        self.policy.append.validate("append")?;
        self.policy.query.validate("query")?;
        self.policy.jsonrpc.validate("jsonrpc")?;

        let append_middleware = ServiceBuilder::new()
            .layer(HandleErrorLayer::new(Self::handle_middleware_error))
            .layer(LoadShedLayer::new())
            .layer(ConcurrencyLimitLayer::new(self.policy.append.concurrency))
            .layer(TimeoutLayer::new(Duration::from_millis(
                self.policy.append.timeout_ms,
            )))
            .layer(RequestBodyLimitLayer::new(
                self.policy.append.body_limit_bytes,
            ));

        let query_middleware = ServiceBuilder::new()
            .layer(HandleErrorLayer::new(Self::handle_middleware_error))
            .layer(LoadShedLayer::new())
            .layer(ConcurrencyLimitLayer::new(self.policy.query.concurrency))
            .layer(TimeoutLayer::new(Duration::from_millis(
                self.policy.query.timeout_ms,
            )))
            .layer(RequestBodyLimitLayer::new(
                self.policy.query.body_limit_bytes,
            ));

        let jsonrpc_middleware = ServiceBuilder::new()
            .layer(HandleErrorLayer::new(Self::handle_middleware_error))
            .layer(LoadShedLayer::new())
            .layer(ConcurrencyLimitLayer::new(self.policy.jsonrpc.concurrency))
            .layer(TimeoutLayer::new(Duration::from_millis(
                self.policy.jsonrpc.timeout_ms,
            )))
            .layer(RequestBodyLimitLayer::new(
                self.policy.jsonrpc.body_limit_bytes,
            ));

        let data_append_path = KLogDataRequestType::Append.klog_path();
        let data_query_path = KLogDataRequestType::Query.klog_path();
        let data_meta_put_path = KLogDataRequestType::MetaPut.klog_path();
        let data_meta_delete_path = KLogDataRequestType::MetaDelete.klog_path();
        let data_meta_query_path = KLogDataRequestType::MetaQuery.klog_path();
        let app = Router::new()
            .merge(
                Router::new()
                    .route(&data_append_path, post(Self::handle_data_append_request))
                    .route(&data_meta_put_path, post(Self::handle_meta_put_request))
                    .route(
                        &data_meta_delete_path,
                        post(Self::handle_meta_delete_request),
                    )
                    .route_layer(append_middleware),
            )
            .merge(
                Router::new()
                    .route(&data_query_path, get(Self::handle_data_query_request))
                    .route(&data_meta_query_path, get(Self::handle_meta_query_request))
                    .route_layer(query_middleware),
            )
            .merge(
                Router::new()
                    .route(KLOG_JSON_RPC_PATH, post(Self::handle_json_rpc_request))
                    .route_layer(jsonrpc_middleware),
            )
            .layer(TraceLayer::new_for_http())
            .with_state(state);

        info!(
            "KRpcServer start listening at {}, append(body_limit_bytes={}, concurrency={}, timeout_ms={}), query(body_limit_bytes={}, concurrency={}, timeout_ms={}), jsonrpc(body_limit_bytes={}, concurrency={}, timeout_ms={}), data_append_path={}, data_query_path={}, data_meta_put_path={}, data_meta_delete_path={}, data_meta_query_path={}, json_rpc_path={}",
            self.addr,
            self.policy.append.body_limit_bytes,
            self.policy.append.concurrency,
            self.policy.append.timeout_ms,
            self.policy.query.body_limit_bytes,
            self.policy.query.concurrency,
            self.policy.query.timeout_ms,
            self.policy.jsonrpc.body_limit_bytes,
            self.policy.jsonrpc.concurrency,
            self.policy.jsonrpc.timeout_ms,
            data_append_path,
            data_query_path,
            data_meta_put_path,
            data_meta_delete_path,
            data_meta_query_path,
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
        let trace_id = normalize_trace_id(
            headers
                .get(KLOG_TRACE_ID_HEADER)
                .and_then(|v| v.to_str().ok()),
        );
        let headers = Self::inject_trace_id_header(headers, &trace_id);
        match state.write_service.append(&headers, req).await {
            Ok(resp) => {
                Self::with_trace_id((StatusCode::OK, Json(resp)).into_response(), &trace_id)
            }
            Err(err) => Self::service_error_response(err),
        }
    }

    async fn handle_data_query_request(
        State(state): State<KRpcServerState>,
        headers: HeaderMap,
        Query(query): Query<KLogQueryRequest>,
    ) -> Response {
        let trace_id = normalize_trace_id(
            headers
                .get(KLOG_TRACE_ID_HEADER)
                .and_then(|v| v.to_str().ok()),
        );
        let headers = Self::inject_trace_id_header(headers, &trace_id);
        match state.query_service.query(&headers, query).await {
            Ok(resp) => {
                Self::with_trace_id((StatusCode::OK, Json(resp)).into_response(), &trace_id)
            }
            Err(err) => Self::service_error_response(err),
        }
    }

    async fn handle_meta_put_request(
        State(state): State<KRpcServerState>,
        headers: HeaderMap,
        Json(req): Json<KLogMetaPutRequest>,
    ) -> Response {
        let trace_id = normalize_trace_id(
            headers
                .get(KLOG_TRACE_ID_HEADER)
                .and_then(|v| v.to_str().ok()),
        );
        let headers = Self::inject_trace_id_header(headers, &trace_id);
        match state.write_service.put_meta(&headers, req).await {
            Ok(resp) => {
                Self::with_trace_id((StatusCode::OK, Json(resp)).into_response(), &trace_id)
            }
            Err(err) => Self::service_error_response(err),
        }
    }

    async fn handle_meta_delete_request(
        State(state): State<KRpcServerState>,
        headers: HeaderMap,
        Json(req): Json<KLogMetaDeleteRequest>,
    ) -> Response {
        let trace_id = normalize_trace_id(
            headers
                .get(KLOG_TRACE_ID_HEADER)
                .and_then(|v| v.to_str().ok()),
        );
        let headers = Self::inject_trace_id_header(headers, &trace_id);
        match state.write_service.delete_meta(&headers, req).await {
            Ok(resp) => {
                Self::with_trace_id((StatusCode::OK, Json(resp)).into_response(), &trace_id)
            }
            Err(err) => Self::service_error_response(err),
        }
    }

    async fn handle_meta_query_request(
        State(state): State<KRpcServerState>,
        headers: HeaderMap,
        Query(query): Query<KLogMetaQueryRequest>,
    ) -> Response {
        let trace_id = normalize_trace_id(
            headers
                .get(KLOG_TRACE_ID_HEADER)
                .and_then(|v| v.to_str().ok()),
        );
        let headers = Self::inject_trace_id_header(headers, &trace_id);
        match state.query_service.query_meta(&headers, query).await {
            Ok(resp) => {
                Self::with_trace_id((StatusCode::OK, Json(resp)).into_response(), &trace_id)
            }
            Err(err) => Self::service_error_response(err),
        }
    }

    async fn handle_json_rpc_request(
        State(state): State<KRpcServerState>,
        headers: HeaderMap,
        Json(request): Json<KLogJsonRpcRequest>,
    ) -> Response {
        let req_id = request.id;
        let trace_id = normalize_trace_id(
            headers
                .get(KLOG_TRACE_ID_HEADER)
                .and_then(|v| v.to_str().ok()),
        );
        let headers = Self::inject_trace_id_header(headers, &trace_id);
        if request.jsonrpc != KLOG_JSON_RPC_VERSION {
            let msg = format!(
                "Invalid jsonrpc version: expected={}, got={}",
                KLOG_JSON_RPC_VERSION, request.jsonrpc
            );
            let envelope = KLogErrorEnvelope::new(
                KLogErrorCode::InvalidArgument,
                msg.clone(),
                trace_id.clone(),
            );
            let resp = KLogJsonRpcResponse::error_with_data(
                req_id,
                KLOG_RPC_ERR_INVALID_REQUEST,
                envelope.message.clone(),
                Some(serde_json::to_value(envelope).unwrap_or_else(|_| serde_json::Value::Null)),
            );
            return Self::with_trace_id((StatusCode::OK, Json(resp)).into_response(), &trace_id);
        }

        match request.method.as_str() {
            KLOG_RPC_METHOD_LOG_APPEND | KLOG_RPC_METHOD_LOG_APPEND_LEGACY => {
                let params: KLogAppendRequest = match serde_json::from_value(request.params) {
                    Ok(params) => params,
                    Err(e) => {
                        let msg = format!("Invalid params for {}: {}", request.method, e);
                        let envelope = KLogErrorEnvelope::new(
                            KLogErrorCode::InvalidArgument,
                            msg.clone(),
                            trace_id.clone(),
                        );
                        let resp = KLogJsonRpcResponse::error_with_data(
                            req_id,
                            KLOG_RPC_ERR_INVALID_PARAMS,
                            envelope.message.clone(),
                            Some(
                                serde_json::to_value(envelope)
                                    .unwrap_or_else(|_| serde_json::Value::Null),
                            ),
                        );
                        return Self::with_trace_id(
                            (StatusCode::OK, Json(resp)).into_response(),
                            &trace_id,
                        );
                    }
                };

                match state.write_service.append(&headers, params).await {
                    Ok(result) => Self::with_trace_id(
                        (
                            StatusCode::OK,
                            Json(KLogJsonRpcResponse::success(req_id, result)),
                        )
                            .into_response(),
                        &trace_id,
                    ),
                    Err(err) => {
                        let err_trace_id = err.error.trace_id.clone();
                        Self::with_trace_id(
                            (
                                StatusCode::OK,
                                Json(KLogJsonRpcResponse::error_with_data(
                                    req_id,
                                    Self::rpc_error_code_from_error_code(err.error.error_code),
                                    err.error.message.clone(),
                                    Some(
                                        serde_json::to_value(err.error)
                                            .unwrap_or_else(|_| serde_json::Value::Null),
                                    ),
                                )),
                            )
                                .into_response(),
                            &err_trace_id,
                        )
                    }
                }
            }
            KLOG_RPC_METHOD_LOG_QUERY | KLOG_RPC_METHOD_LOG_QUERY_LEGACY => {
                let params: KLogQueryRequest = if request.params.is_null() {
                    KLogQueryRequest::default()
                } else {
                    match serde_json::from_value(request.params) {
                        Ok(params) => params,
                        Err(e) => {
                            let msg = format!("Invalid params for {}: {}", request.method, e);
                            let envelope = KLogErrorEnvelope::new(
                                KLogErrorCode::InvalidArgument,
                                msg.clone(),
                                trace_id.clone(),
                            );
                            let resp = KLogJsonRpcResponse::error_with_data(
                                req_id,
                                KLOG_RPC_ERR_INVALID_PARAMS,
                                envelope.message.clone(),
                                Some(
                                    serde_json::to_value(envelope)
                                        .unwrap_or_else(|_| serde_json::Value::Null),
                                ),
                            );
                            return Self::with_trace_id(
                                (StatusCode::OK, Json(resp)).into_response(),
                                &trace_id,
                            );
                        }
                    }
                };

                match state.query_service.query(&headers, params).await {
                    Ok(result) => Self::with_trace_id(
                        (
                            StatusCode::OK,
                            Json(KLogJsonRpcResponse::success(req_id, result)),
                        )
                            .into_response(),
                        &trace_id,
                    ),
                    Err(err) => {
                        let err_trace_id = err.error.trace_id.clone();
                        Self::with_trace_id(
                            (
                                StatusCode::OK,
                                Json(KLogJsonRpcResponse::error_with_data(
                                    req_id,
                                    Self::rpc_error_code_from_error_code(err.error.error_code),
                                    err.error.message.clone(),
                                    Some(
                                        serde_json::to_value(err.error)
                                            .unwrap_or_else(|_| serde_json::Value::Null),
                                    ),
                                )),
                            )
                                .into_response(),
                            &err_trace_id,
                        )
                    }
                }
            }
            KLOG_RPC_METHOD_META_PUT => {
                let params: KLogMetaPutRequest = match serde_json::from_value(request.params) {
                    Ok(params) => params,
                    Err(e) => {
                        let msg = format!("Invalid params for {}: {}", KLOG_RPC_METHOD_META_PUT, e);
                        let envelope = KLogErrorEnvelope::new(
                            KLogErrorCode::InvalidArgument,
                            msg.clone(),
                            trace_id.clone(),
                        );
                        let resp = KLogJsonRpcResponse::error_with_data(
                            req_id,
                            KLOG_RPC_ERR_INVALID_PARAMS,
                            envelope.message.clone(),
                            Some(
                                serde_json::to_value(envelope)
                                    .unwrap_or_else(|_| serde_json::Value::Null),
                            ),
                        );
                        return Self::with_trace_id(
                            (StatusCode::OK, Json(resp)).into_response(),
                            &trace_id,
                        );
                    }
                };

                match state.write_service.put_meta(&headers, params).await {
                    Ok(result) => Self::with_trace_id(
                        (
                            StatusCode::OK,
                            Json(KLogJsonRpcResponse::success(req_id, result)),
                        )
                            .into_response(),
                        &trace_id,
                    ),
                    Err(err) => {
                        let err_trace_id = err.error.trace_id.clone();
                        Self::with_trace_id(
                            (
                                StatusCode::OK,
                                Json(KLogJsonRpcResponse::error_with_data(
                                    req_id,
                                    Self::rpc_error_code_from_error_code(err.error.error_code),
                                    err.error.message.clone(),
                                    Some(
                                        serde_json::to_value(err.error)
                                            .unwrap_or_else(|_| serde_json::Value::Null),
                                    ),
                                )),
                            )
                                .into_response(),
                            &err_trace_id,
                        )
                    }
                }
            }
            KLOG_RPC_METHOD_META_DELETE => {
                let params: KLogMetaDeleteRequest = match serde_json::from_value(request.params) {
                    Ok(params) => params,
                    Err(e) => {
                        let msg =
                            format!("Invalid params for {}: {}", KLOG_RPC_METHOD_META_DELETE, e);
                        let envelope = KLogErrorEnvelope::new(
                            KLogErrorCode::InvalidArgument,
                            msg.clone(),
                            trace_id.clone(),
                        );
                        let resp = KLogJsonRpcResponse::error_with_data(
                            req_id,
                            KLOG_RPC_ERR_INVALID_PARAMS,
                            envelope.message.clone(),
                            Some(
                                serde_json::to_value(envelope)
                                    .unwrap_or_else(|_| serde_json::Value::Null),
                            ),
                        );
                        return Self::with_trace_id(
                            (StatusCode::OK, Json(resp)).into_response(),
                            &trace_id,
                        );
                    }
                };

                match state.write_service.delete_meta(&headers, params).await {
                    Ok(result) => Self::with_trace_id(
                        (
                            StatusCode::OK,
                            Json(KLogJsonRpcResponse::success(req_id, result)),
                        )
                            .into_response(),
                        &trace_id,
                    ),
                    Err(err) => {
                        let err_trace_id = err.error.trace_id.clone();
                        Self::with_trace_id(
                            (
                                StatusCode::OK,
                                Json(KLogJsonRpcResponse::error_with_data(
                                    req_id,
                                    Self::rpc_error_code_from_error_code(err.error.error_code),
                                    err.error.message.clone(),
                                    Some(
                                        serde_json::to_value(err.error)
                                            .unwrap_or_else(|_| serde_json::Value::Null),
                                    ),
                                )),
                            )
                                .into_response(),
                            &err_trace_id,
                        )
                    }
                }
            }
            KLOG_RPC_METHOD_META_QUERY => {
                let params: KLogMetaQueryRequest = if request.params.is_null() {
                    KLogMetaQueryRequest::default()
                } else {
                    match serde_json::from_value(request.params) {
                        Ok(params) => params,
                        Err(e) => {
                            let msg =
                                format!("Invalid params for {}: {}", KLOG_RPC_METHOD_META_QUERY, e);
                            let envelope = KLogErrorEnvelope::new(
                                KLogErrorCode::InvalidArgument,
                                msg.clone(),
                                trace_id.clone(),
                            );
                            let resp = KLogJsonRpcResponse::error_with_data(
                                req_id,
                                KLOG_RPC_ERR_INVALID_PARAMS,
                                envelope.message.clone(),
                                Some(
                                    serde_json::to_value(envelope)
                                        .unwrap_or_else(|_| serde_json::Value::Null),
                                ),
                            );
                            return Self::with_trace_id(
                                (StatusCode::OK, Json(resp)).into_response(),
                                &trace_id,
                            );
                        }
                    }
                };

                match state.query_service.query_meta(&headers, params).await {
                    Ok(result) => Self::with_trace_id(
                        (
                            StatusCode::OK,
                            Json(KLogJsonRpcResponse::success(req_id, result)),
                        )
                            .into_response(),
                        &trace_id,
                    ),
                    Err(err) => {
                        let err_trace_id = err.error.trace_id.clone();
                        Self::with_trace_id(
                            (
                                StatusCode::OK,
                                Json(KLogJsonRpcResponse::error_with_data(
                                    req_id,
                                    Self::rpc_error_code_from_error_code(err.error.error_code),
                                    err.error.message.clone(),
                                    Some(
                                        serde_json::to_value(err.error)
                                            .unwrap_or_else(|_| serde_json::Value::Null),
                                    ),
                                )),
                            )
                                .into_response(),
                            &err_trace_id,
                        )
                    }
                }
            }
            _ => {
                let envelope = KLogErrorEnvelope::new(
                    KLogErrorCode::InvalidArgument,
                    format!("Unknown method: {}", request.method),
                    trace_id.clone(),
                );
                let resp = (
                    StatusCode::OK,
                    Json(KLogJsonRpcResponse::error_with_data(
                        req_id,
                        KLOG_RPC_ERR_METHOD_NOT_FOUND,
                        envelope.message.clone(),
                        Some(
                            serde_json::to_value(envelope)
                                .unwrap_or_else(|_| serde_json::Value::Null),
                        ),
                    )),
                )
                    .into_response();
                Self::with_trace_id(resp, &trace_id)
            }
        }
    }

    fn rpc_error_code_from_error_code(code: KLogErrorCode) -> i64 {
        if matches!(
            code,
            KLogErrorCode::InvalidArgument
                | KLogErrorCode::PayloadTooLarge
                | KLogErrorCode::VersionConflict
        ) {
            KLOG_RPC_ERR_INVALID_PARAMS
        } else if matches!(
            code,
            KLogErrorCode::NotLeader | KLogErrorCode::LeaderUnavailable
        ) {
            KLOG_RPC_ERR_INTERNAL
        } else if matches!(code, KLogErrorCode::AuthRequired | KLogErrorCode::Forbidden) {
            KLOG_RPC_ERR_INVALID_REQUEST
        } else if matches!(code, KLogErrorCode::Unavailable | KLogErrorCode::Timeout) {
            KLOG_RPC_ERR_INTERNAL
        } else if code == KLogErrorCode::Internal {
            KLOG_RPC_ERR_INTERNAL
        } else {
            KLOG_RPC_ERR_METHOD_NOT_FOUND
        }
    }

    fn error_response(status: StatusCode, msg: String) -> Response {
        let trace_id = generate_trace_id();
        let envelope = KLogErrorEnvelope::from_http_status(status.as_u16(), msg, trace_id);
        let trace_id = envelope.trace_id.clone();
        let resp = (status, Json(envelope)).into_response();
        Self::with_trace_id(resp, &trace_id)
    }

    fn service_error_response(err: KLogServiceError) -> Response {
        let status =
            StatusCode::from_u16(err.http_status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let trace_id = err.error.trace_id.clone();
        let resp = (status, Json(err.error)).into_response();
        Self::with_trace_id(resp, &trace_id)
    }

    fn inject_trace_id_header(mut headers: HeaderMap, trace_id: &str) -> HeaderMap {
        if let Ok(v) = HeaderValue::from_str(trace_id) {
            headers.insert(HeaderName::from_static(KLOG_TRACE_ID_HEADER), v);
        }
        headers
    }

    fn with_trace_id(mut resp: Response, trace_id: &str) -> Response {
        if let Ok(v) = HeaderValue::from_str(trace_id) {
            resp.headers_mut()
                .insert(HeaderName::from_static(KLOG_TRACE_ID_HEADER), v);
        }
        resp
    }
}
