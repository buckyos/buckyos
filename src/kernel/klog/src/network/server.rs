use super::request::{RaftRequest, RaftRequestType, RaftResponse};
use crate::KRaftRef;
use axum::Router;
use axum::body::Bytes;
use axum::error_handling::HandleErrorLayer;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use std::future::Future;
use std::time::Duration;
use tower::BoxError;
use tower::ServiceBuilder;
use tower::limit::ConcurrencyLimitLayer;
use tower::load_shed::LoadShedLayer;
use tower::timeout::TimeoutLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

const CONTROL_RPC_BODY_LIMIT_BYTES: usize = 1 * 1024 * 1024;
const SNAPSHOT_RPC_BODY_LIMIT_BYTES: usize = 8 * 1024 * 1024;
const CONTROL_RPC_CONCURRENCY_LIMIT: usize = 128;
const SNAPSHOT_RPC_CONCURRENCY_LIMIT: usize = 8;
const CONTROL_RPC_TIMEOUT_MS: u64 = 3_000;
const SNAPSHOT_RPC_TIMEOUT_MS: u64 = 30_000;

#[derive(Clone)]
struct KNetworkServerState {
    raft: KRaftRef,
}

pub struct KNetworkServer {
    addr: String,
    raft: KRaftRef,
}

impl KNetworkServer {
    pub fn new(addr: String, raft: KRaftRef) -> Self {
        Self { addr, raft }
    }

    pub async fn run(&self) -> Result<(), String> {
        self.run_with_shutdown(std::future::pending::<()>()).await
    }

    pub async fn run_with_shutdown<F>(&self, shutdown: F) -> Result<(), String>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let state = KNetworkServerState {
            raft: self.raft.clone(),
        };

        let control_rpc_middleware = ServiceBuilder::new()
            .layer(HandleErrorLayer::new(Self::handle_middleware_error))
            .layer(LoadShedLayer::new())
            .layer(ConcurrencyLimitLayer::new(CONTROL_RPC_CONCURRENCY_LIMIT))
            .layer(TimeoutLayer::new(Duration::from_millis(
                CONTROL_RPC_TIMEOUT_MS,
            )))
            .layer(RequestBodyLimitLayer::new(CONTROL_RPC_BODY_LIMIT_BYTES));

        let append_entries_path = RaftRequestType::AppendEntries.klog_path();
        let vote_path = RaftRequestType::Vote.klog_path();
        let control_rpc_routes = Router::new()
            .route(
                &append_entries_path,
                post(Self::handle_append_entries_request),
            )
            .route(&vote_path, post(Self::handle_vote_request))
            .route_layer(control_rpc_middleware);

        let snapshot_rpc_middleware = ServiceBuilder::new()
            .layer(HandleErrorLayer::new(Self::handle_middleware_error))
            .layer(LoadShedLayer::new())
            .layer(ConcurrencyLimitLayer::new(SNAPSHOT_RPC_CONCURRENCY_LIMIT))
            .layer(TimeoutLayer::new(Duration::from_millis(
                SNAPSHOT_RPC_TIMEOUT_MS,
            )))
            .layer(RequestBodyLimitLayer::new(SNAPSHOT_RPC_BODY_LIMIT_BYTES));

        let install_snapshot_path = RaftRequestType::InstallSnapshot.klog_path();
        let snapshot_routes = Router::new()
            .route(
                &install_snapshot_path,
                post(Self::handle_install_snapshot_request),
            )
            .route_layer(snapshot_rpc_middleware);

        let app = Router::new()
            .merge(control_rpc_routes)
            .merge(snapshot_routes)
            .layer(TraceLayer::new_for_http())
            .with_state(state);

        info!(
            "KNetworkServer start listening at {}, control_limit_bytes={}, snapshot_limit_bytes={}, control_concurrency={}, snapshot_concurrency={}, control_timeout_ms={}, snapshot_timeout_ms={}",
            self.addr,
            CONTROL_RPC_BODY_LIMIT_BYTES,
            SNAPSHOT_RPC_BODY_LIMIT_BYTES,
            CONTROL_RPC_CONCURRENCY_LIMIT,
            SNAPSHOT_RPC_CONCURRENCY_LIMIT,
            CONTROL_RPC_TIMEOUT_MS,
            SNAPSHOT_RPC_TIMEOUT_MS
        );

        let listener = tokio::net::TcpListener::bind(&self.addr)
            .await
            .map_err(|e| {
                let msg = format!("KNetworkServer bind failed at {}: {}", self.addr, e);
                error!("{}", msg);
                msg
            })?;

        let addr = self.addr.clone();
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                shutdown.await;
                info!(
                    "KNetworkServer shutdown signal received at {}, stop accepting new connections and draining in-flight requests",
                    addr
                );
            })
            .await
            .map_err(|e| {
                let msg = format!("KNetworkServer serve failed at {}: {}", self.addr, e);
                error!("{}", msg);
                msg
            })
    }

    async fn handle_middleware_error(err: BoxError) -> Response {
        let msg = format!("KNetworkServer middleware rejected request: {}", err);
        error!("{}", msg);

        if err.is::<tower::timeout::error::Elapsed>() {
            return Self::error_response(StatusCode::REQUEST_TIMEOUT, msg);
        }

        if err.is::<tower::load_shed::error::Overloaded>() {
            return Self::error_response(StatusCode::SERVICE_UNAVAILABLE, msg);
        }

        Self::error_response(StatusCode::INTERNAL_SERVER_ERROR, msg)
    }

    async fn handle_append_entries_request(
        State(state): State<KNetworkServerState>,
        body: Bytes,
    ) -> Response {
        let req = match Self::decode_request(RaftRequestType::AppendEntries, &body) {
            Ok(req) => req,
            Err(resp) => return resp,
        };
        let req = match req {
            RaftRequest::AppendEntries(req) => req,
            other => {
                let msg = format!(
                    "KNetworkServer append-entries type check failed: got={}",
                    other.request_type().as_str()
                );
                error!("{}", msg);
                return Self::error_response(StatusCode::BAD_REQUEST, msg);
            }
        };

        debug!(
            "KNetworkServer append-entries request: entries={}, body_bytes={}",
            req.entries.len(),
            body.len()
        );

        match state.raft.append_entries(req).await {
            Ok(resp) => Self::encode_response(
                RaftRequestType::AppendEntries,
                RaftResponse::AppendEntries(resp),
            ),
            Err(e) => {
                error!("KNetworkServer append-entries raft call failed: {}", e);
                Self::encode_response(
                    RaftRequestType::AppendEntries,
                    RaftResponse::AppendEntriesError(e),
                )
            }
        }
    }

    async fn handle_install_snapshot_request(
        State(state): State<KNetworkServerState>,
        body: Bytes,
    ) -> Response {
        let req = match Self::decode_request(RaftRequestType::InstallSnapshot, &body) {
            Ok(req) => req,
            Err(resp) => return resp,
        };
        let req = match req {
            RaftRequest::InstallSnapshot(req) => req,
            other => {
                let msg = format!(
                    "KNetworkServer install-snapshot type check failed: got={}",
                    other.request_type().as_str()
                );
                error!("{}", msg);
                return Self::error_response(StatusCode::BAD_REQUEST, msg);
            }
        };

        debug!(
            "KNetworkServer install-snapshot request: snapshot_id={}, offset={}, chunk_bytes={}, done={}, body_bytes={}",
            req.meta.snapshot_id,
            req.offset,
            req.data.len(),
            req.done,
            body.len()
        );

        match state.raft.install_snapshot(req).await {
            Ok(resp) => Self::encode_response(
                RaftRequestType::InstallSnapshot,
                RaftResponse::InstallSnapshot(resp),
            ),
            Err(e) => {
                error!("KNetworkServer install-snapshot raft call failed: {}", e);
                Self::encode_response(
                    RaftRequestType::InstallSnapshot,
                    RaftResponse::InstallSnapshotError(e),
                )
            }
        }
    }

    async fn handle_vote_request(
        State(state): State<KNetworkServerState>,
        body: Bytes,
    ) -> Response {
        let req = match Self::decode_request(RaftRequestType::Vote, &body) {
            Ok(req) => req,
            Err(resp) => return resp,
        };
        let req = match req {
            RaftRequest::Vote(req) => req,
            other => {
                let msg = format!(
                    "KNetworkServer vote type check failed: got={}",
                    other.request_type().as_str()
                );
                error!("{}", msg);
                return Self::error_response(StatusCode::BAD_REQUEST, msg);
            }
        };

        debug!(
            "KNetworkServer vote request: vote={}, last_log_id={:?}, body_bytes={}",
            req.vote,
            req.last_log_id,
            body.len()
        );

        match state.raft.vote(req).await {
            Ok(resp) => Self::encode_response(RaftRequestType::Vote, RaftResponse::Vote(resp)),
            Err(e) => {
                error!("KNetworkServer vote raft call failed: {}", e);
                Self::encode_response(RaftRequestType::Vote, RaftResponse::VoteError(e))
            }
        }
    }

    fn decode_request(expected: RaftRequestType, body: &[u8]) -> Result<RaftRequest, Response> {
        info!(
            "KNetworkServer decode request: rpc={}, body_bytes={}",
            expected.as_str(),
            body.len()
        );

        let req = RaftRequest::deserialize(body).map_err(|e| {
            let msg = format!(
                "KNetworkServer deserialize request failed: rpc={}, body_bytes={}, err={}",
                expected.as_str(),
                body.len(),
                e
            );
            error!("{}", msg);
            Self::error_response(StatusCode::BAD_REQUEST, msg)
        })?;

        if req.request_type().as_str() != expected.as_str() {
            let msg = format!(
                "KNetworkServer request type mismatch: expected={}, got={}",
                expected.as_str(),
                req.request_type().as_str()
            );
            error!("{}", msg);
            return Err(Self::error_response(StatusCode::BAD_REQUEST, msg));
        }

        Ok(req)
    }

    fn encode_response(rpc: RaftRequestType, resp: RaftResponse) -> Response {
        let bytes = match resp.serialize() {
            Ok(bytes) => bytes,
            Err(e) => {
                let msg = format!(
                    "KNetworkServer serialize response failed: rpc={}, err={}",
                    rpc.as_str(),
                    e
                );
                error!("{}", msg);
                return Self::error_response(StatusCode::INTERNAL_SERVER_ERROR, msg);
            }
        };

        info!(
            "KNetworkServer response ready: rpc={}, payload_bytes={}",
            rpc.as_str(),
            bytes.len()
        );

        (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/octet-stream")],
            bytes,
        )
            .into_response()
    }

    fn error_response(status: StatusCode, msg: String) -> Response {
        (status, msg).into_response()
    }
}
