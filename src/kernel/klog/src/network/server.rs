use super::KDataClient;
use super::request::{
    KLOG_FORWARD_HOPS_HEADER, KLOG_FORWARDED_BY_HEADER, KLogAdminRequestType, KLogAppendRequest,
    KLogAppendResponse, KLogClusterStateResponse, KLogDataRequestType, RaftRequest,
    RaftRequestType, RaftResponse,
};
use crate::state_store::KLogStateStoreManagerRef;
use crate::{KLogEntry, KLogRequest, KLogResponse, KNode, KNodeId, KRaftRef};
use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::error_handling::HandleErrorLayer;
use axum::extract::{ConnectInfo, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use openraft::ChangeMembers;
use openraft::error::{ClientWriteError, RaftError};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::future::Future;
use std::net::SocketAddr;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};
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
const DATA_APPEND_MAX_MESSAGE_BYTES: usize = 64 * 1024;
const DATA_APPEND_MAX_FORWARD_HOPS: u32 = 2;

#[derive(Debug, Deserialize)]
struct AddLearnerQuery {
    node_id: KNodeId,
    addr: String,
    port: u16,
    rpc_port: Option<u16>,
    blocking: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ChangeMembershipQuery {
    voters: String,
    retain: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct RemoveLearnerQuery {
    node_id: KNodeId,
}

#[derive(Clone)]
struct KNetworkServerState {
    raft: KRaftRef,
    state_store_manager: Option<KLogStateStoreManagerRef>,
    data_client: KDataClient,
    admin_local_only: bool,
    cluster_name: String,
    cluster_id: String,
}

pub struct KNetworkServer {
    addr: String,
    raft: KRaftRef,
    state_store_manager: Option<KLogStateStoreManagerRef>,
    admin_local_only: bool,
    cluster_name: String,
    cluster_id: String,
}

impl KNetworkServer {
    pub fn new(addr: String, raft: KRaftRef) -> Self {
        Self {
            addr,
            raft,
            state_store_manager: None,
            admin_local_only: false,
            cluster_name: "klog".to_string(),
            cluster_id: "klog".to_string(),
        }
    }

    pub fn with_state_store_manager(
        mut self,
        state_store_manager: KLogStateStoreManagerRef,
    ) -> Self {
        self.state_store_manager = Some(state_store_manager);
        self
    }

    pub fn with_admin_local_only(mut self, admin_local_only: bool) -> Self {
        self.admin_local_only = admin_local_only;
        self
    }

    pub fn with_cluster_identity(mut self, cluster_name: String, cluster_id: String) -> Self {
        self.cluster_name = cluster_name;
        self.cluster_id = cluster_id;
        self
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
            state_store_manager: self.state_store_manager.clone(),
            data_client: KDataClient::new(),
            admin_local_only: self.admin_local_only,
            cluster_name: self.cluster_name.clone(),
            cluster_id: self.cluster_id.clone(),
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
        let data_append_path = KLogDataRequestType::Append.klog_path();
        let admin_add_learner_path = KLogAdminRequestType::AddLearner.klog_path();
        let admin_remove_learner_path = KLogAdminRequestType::RemoveLearner.klog_path();
        let admin_change_membership_path = KLogAdminRequestType::ChangeMembership.klog_path();
        let admin_cluster_state_path = KLogAdminRequestType::ClusterState.klog_path();

        let control_rpc_routes = Router::new()
            .route(
                &append_entries_path,
                post(Self::handle_append_entries_request),
            )
            .route(&vote_path, post(Self::handle_vote_request))
            .route(&data_append_path, post(Self::handle_data_append_request))
            .route(
                &admin_add_learner_path,
                post(Self::handle_add_learner_request),
            )
            .route(
                &admin_change_membership_path,
                post(Self::handle_change_membership_request),
            )
            .route(
                &admin_remove_learner_path,
                post(Self::handle_remove_learner_request),
            )
            .route(
                &admin_cluster_state_path,
                get(Self::handle_cluster_state_request),
            )
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
            "KNetworkServer start listening at {}, cluster_name={}, cluster_id={}, control_limit_bytes={}, snapshot_limit_bytes={}, control_concurrency={}, snapshot_concurrency={}, control_timeout_ms={}, snapshot_timeout_ms={}, admin_local_only={}, data_append_path={}, admin_add_learner_path={}, admin_remove_learner_path={}, admin_change_membership_path={}, admin_cluster_state_path={}",
            self.addr,
            self.cluster_name.as_str(),
            self.cluster_id.as_str(),
            CONTROL_RPC_BODY_LIMIT_BYTES,
            SNAPSHOT_RPC_BODY_LIMIT_BYTES,
            CONTROL_RPC_CONCURRENCY_LIMIT,
            SNAPSHOT_RPC_CONCURRENCY_LIMIT,
            CONTROL_RPC_TIMEOUT_MS,
            SNAPSHOT_RPC_TIMEOUT_MS,
            self.admin_local_only,
            data_append_path,
            admin_add_learner_path,
            admin_remove_learner_path,
            admin_change_membership_path,
            admin_cluster_state_path
        );

        let listener = tokio::net::TcpListener::bind(&self.addr)
            .await
            .map_err(|e| {
                let msg = format!("KNetworkServer bind failed at {}: {}", self.addr, e);
                error!("{}", msg);
                msg
            })?;

        let addr = self.addr.clone();
        axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
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

    async fn handle_data_append_request(
        State(state): State<KNetworkServerState>,
        headers: HeaderMap,
        Json(req): Json<KLogAppendRequest>,
    ) -> Response {
        match Self::process_data_append(&state, &headers, req).await {
            Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
            Err((status, msg)) => Self::error_response(status, msg),
        }
    }

    async fn process_data_append(
        state: &KNetworkServerState,
        headers: &HeaderMap,
        req: KLogAppendRequest,
    ) -> Result<KLogAppendResponse, (StatusCode, String)> {
        if req.message.trim().is_empty() {
            let msg = "KNetworkServer data append rejected: empty message".to_string();
            error!("{}", msg);
            return Err((StatusCode::BAD_REQUEST, msg));
        }

        if req.message.len() > DATA_APPEND_MAX_MESSAGE_BYTES {
            let msg = format!(
                "KNetworkServer data append rejected: message too large, bytes={}, max_bytes={}",
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
                "KNetworkServer data append rejected: too many forward hops, hops={}, max_hops={}, forwarded_by={}",
                forward_hops, DATA_APPEND_MAX_FORWARD_HOPS, forwarded_by
            );
            error!("{}", msg);
            return Err((StatusCode::BAD_GATEWAY, msg));
        }

        let Some(state_store_manager) = state.state_store_manager.as_ref() else {
            let msg = "KNetworkServer data append rejected: state store manager is not configured"
                .to_string();
            error!("{}", msg);
            return Err((StatusCode::INTERNAL_SERVER_ERROR, msg));
        };

        let metrics = state.raft.metrics().borrow().clone();
        let local_node_id = metrics.id;
        let req = KLogAppendRequest {
            message: req.message,
            timestamp: req.timestamp.or_else(|| Some(Self::now_millis())),
            node_id: req.node_id.or(Some(local_node_id)),
        };

        let item = state_store_manager.prepare_append_entry(KLogEntry {
            id: 0,
            timestamp: req.timestamp.unwrap_or(0),
            node_id: req.node_id.unwrap_or(local_node_id),
            message: req.message.clone(),
        });
        let requested_id = item.id;

        info!(
            "KNetworkServer data append request: id={}, timestamp={}, node_id={}, msg_len={}, local_node_id={}, current_leader={:?}, forward_hops={}, forwarded_by={}",
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
                    info!("KNetworkServer data append committed: id={}", id);
                    Ok(KLogAppendResponse { id })
                }
                KLogResponse::Err(err_msg) => {
                    let msg = format!(
                        "KNetworkServer data append failed in state machine: requested_id={}, err={}",
                        requested_id, err_msg
                    );
                    error!("{}", msg);
                    Err((StatusCode::INTERNAL_SERVER_ERROR, msg))
                }
                other => {
                    let msg = format!(
                        "KNetworkServer data append unexpected response: requested_id={}, response={:?}",
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
                            "KNetworkServer data append forward aborted due to hop limit: local_node_id={}, requested_id={}, leader_id={:?}, leader_node={:?}, hops={}, max_hops={}",
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
                            "KNetworkServer data append can not resolve leader node for forwarding: local_node_id={}, requested_id={}, leader_id={:?}",
                            local_node_id, requested_id, forward.leader_id
                        );
                        warn!("{}", msg);
                        return Err((StatusCode::SERVICE_UNAVAILABLE, msg));
                    };

                    let target_hops = forward_hops + 1;
                    warn!(
                        "KNetworkServer data append forwarding to leader: local_node_id={}, requested_id={}, leader_id={}, leader_addr={}:{}, hops={} -> {}",
                        local_node_id,
                        requested_id,
                        leader_node.id,
                        leader_node.addr,
                        leader_node.port,
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
                                "KNetworkServer data append forwarded and committed: local_node_id={}, requested_id={}, committed_id={}, leader_id={}, hops={}",
                                local_node_id, requested_id, resp.id, leader_node.id, target_hops
                            );
                            Ok(resp)
                        }
                        Err(forward_err) => {
                            let msg = format!(
                                "KNetworkServer data append forward failed: local_node_id={}, requested_id={}, leader_id={}, err={}",
                                local_node_id, requested_id, leader_node.id, forward_err
                            );
                            error!("{}", msg);
                            Err((StatusCode::BAD_GATEWAY, msg))
                        }
                    }
                } else {
                    let msg = format!(
                        "KNetworkServer data append raft client_write failed: requested_id={}, err={}",
                        requested_id, err
                    );
                    error!("{}", msg);
                    Err((StatusCode::INTERNAL_SERVER_ERROR, msg))
                }
            }
        }
    }

    async fn handle_add_learner_request(
        State(state): State<KNetworkServerState>,
        ConnectInfo(peer): ConnectInfo<SocketAddr>,
        Query(query): Query<AddLearnerQuery>,
    ) -> Response {
        if let Some(resp) =
            Self::reject_non_loopback_admin_access(state.admin_local_only, peer, "add-learner")
        {
            return resp;
        }

        let blocking = query.blocking.unwrap_or(true);
        let node = KNode {
            id: query.node_id,
            addr: query.addr.clone(),
            port: query.port,
            rpc_port: query.rpc_port.unwrap_or(query.port),
        };
        info!(
            "KNetworkServer admin add-learner request: node_id={}, addr={}, port={}, rpc_port={}, blocking={}",
            query.node_id, query.addr, query.port, node.rpc_port, blocking
        );

        match state.raft.add_learner(query.node_id, node, blocking).await {
            Ok(resp) => {
                let msg = format!(
                    "add-learner committed: node_id={}, log_id={}, membership={:?}",
                    query.node_id, resp.log_id, resp.membership
                );
                info!("KNetworkServer admin add-learner succeeded: {}", msg);
                (StatusCode::OK, msg).into_response()
            }
            Err(err) => Self::raft_client_write_error_response("add-learner", err),
        }
    }

    async fn handle_change_membership_request(
        State(state): State<KNetworkServerState>,
        ConnectInfo(peer): ConnectInfo<SocketAddr>,
        Query(query): Query<ChangeMembershipQuery>,
    ) -> Response {
        if let Some(resp) = Self::reject_non_loopback_admin_access(
            state.admin_local_only,
            peer,
            "change-membership",
        ) {
            return resp;
        }

        let retain = query.retain.unwrap_or(true);
        let voter_ids = match parse_voter_ids_csv(&query.voters) {
            Ok(ids) => ids,
            Err(err) => {
                let msg = format!(
                    "KNetworkServer admin change-membership invalid voters '{}': {}",
                    query.voters, err
                );
                error!("{}", msg);
                return Self::error_response(StatusCode::BAD_REQUEST, msg);
            }
        };

        info!(
            "KNetworkServer admin change-membership request: voters={:?}, retain={}",
            voter_ids, retain
        );

        match state
            .raft
            .change_membership(voter_ids.clone(), retain)
            .await
        {
            Ok(resp) => {
                let msg = format!(
                    "change-membership committed: voters={:?}, log_id={}, membership={:?}",
                    voter_ids, resp.log_id, resp.membership
                );
                info!("KNetworkServer admin change-membership succeeded: {}", msg);
                (StatusCode::OK, msg).into_response()
            }
            Err(err) => Self::raft_client_write_error_response("change-membership", err),
        }
    }

    async fn handle_remove_learner_request(
        State(state): State<KNetworkServerState>,
        ConnectInfo(peer): ConnectInfo<SocketAddr>,
        Query(query): Query<RemoveLearnerQuery>,
    ) -> Response {
        if let Some(resp) =
            Self::reject_non_loopback_admin_access(state.admin_local_only, peer, "remove-learner")
        {
            return resp;
        }

        info!(
            "KNetworkServer admin remove-learner request: node_id={}",
            query.node_id
        );
        let mut remove_nodes = BTreeSet::new();
        remove_nodes.insert(query.node_id);

        match state
            .raft
            .change_membership(ChangeMembers::RemoveNodes(remove_nodes), true)
            .await
        {
            Ok(resp) => {
                let msg = format!(
                    "remove-learner committed: node_id={}, log_id={}, membership={:?}",
                    query.node_id, resp.log_id, resp.membership
                );
                info!("KNetworkServer admin remove-learner succeeded: {}", msg);
                (StatusCode::OK, msg).into_response()
            }
            Err(err) => Self::raft_client_write_error_response("remove-learner", err),
        }
    }

    async fn handle_cluster_state_request(
        State(state): State<KNetworkServerState>,
        ConnectInfo(peer): ConnectInfo<SocketAddr>,
    ) -> Response {
        if let Some(resp) =
            Self::reject_non_loopback_admin_access(state.admin_local_only, peer, "cluster-state")
        {
            return resp;
        }

        let metrics = state.raft.metrics();
        let metrics = metrics.borrow().clone();

        let membership = metrics.membership_config.membership();
        let voters = membership.voter_ids().collect::<Vec<_>>();
        let learners = membership.learner_ids().collect::<Vec<_>>();
        let nodes = metrics
            .membership_config
            .nodes()
            .map(|(id, node)| (*id, node.clone()))
            .collect();

        let body = KLogClusterStateResponse {
            node_id: metrics.id,
            cluster_name: state.cluster_name.clone(),
            cluster_id: state.cluster_id.clone(),
            server_state: format!("{:?}", metrics.state),
            current_leader: metrics.current_leader,
            voters,
            learners,
            nodes,
        };

        info!(
            "KNetworkServer admin cluster-state request: node_id={}, cluster_name={}, cluster_id={}, server_state={}, current_leader={:?}, voters={:?}, learners={:?}",
            body.node_id,
            body.cluster_name,
            body.cluster_id,
            body.server_state,
            body.current_leader,
            body.voters,
            body.learners
        );

        (StatusCode::OK, axum::Json(body)).into_response()
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

    fn raft_client_write_error_response(
        action: &str,
        err: RaftError<KNodeId, ClientWriteError<KNodeId, KNode>>,
    ) -> Response {
        if let Some(forward) = err.forward_to_leader::<KNode>() {
            let msg = format!(
                "KNetworkServer admin {} rejected on non-leader: leader_id={:?}, leader_node={:?}",
                action, forward.leader_id, forward.leader_node
            );
            warn!("{}", msg);
            return Self::error_response(StatusCode::CONFLICT, msg);
        }

        let msg = format!("KNetworkServer admin {} failed: {}", action, err);
        error!("{}", msg);
        Self::error_response(StatusCode::INTERNAL_SERVER_ERROR, msg)
    }

    fn reject_non_loopback_admin_access(
        admin_local_only: bool,
        peer: SocketAddr,
        action: &str,
    ) -> Option<Response> {
        if !admin_local_only || peer.ip().is_loopback() {
            return None;
        }

        let msg = format!(
            "KNetworkServer admin {} forbidden for non-loopback peer: {}",
            action, peer
        );
        warn!("{}", msg);
        Some(Self::error_response(StatusCode::FORBIDDEN, msg))
    }

    fn parse_forward_hops(headers: &HeaderMap) -> Result<u32, String> {
        let Some(raw) = headers.get(KLOG_FORWARD_HOPS_HEADER) else {
            return Ok(0);
        };
        let raw = raw.to_str().map_err(|e| {
            format!(
                "KNetworkServer data append invalid {} header utf8: {}",
                KLOG_FORWARD_HOPS_HEADER, e
            )
        })?;
        raw.parse::<u32>().map_err(|e| {
            format!(
                "KNetworkServer data append invalid {} header '{}': {}",
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

fn parse_voter_ids_csv(raw: &str) -> Result<Vec<KNodeId>, String> {
    let mut ids = BTreeSet::new();
    for token in raw.split(',') {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            continue;
        }
        let id = trimmed
            .parse::<KNodeId>()
            .map_err(|e| format!("invalid node id '{}': {}", trimmed, e))?;
        ids.insert(id);
    }

    if ids.is_empty() {
        return Err("empty voter set".to_string());
    }

    Ok(ids.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::{KNetworkServer, parse_voter_ids_csv};
    use crate::network::KLOG_FORWARD_HOPS_HEADER;
    use axum::http::HeaderMap;

    #[test]
    fn test_parse_voter_ids_csv_ok() {
        let ids = parse_voter_ids_csv("1, 2,3,2").expect("parse voter ids");
        assert_eq!(ids, vec![1, 2, 3]);
    }

    #[test]
    fn test_parse_voter_ids_csv_empty_rejected() {
        let err = parse_voter_ids_csv(" ,  ").expect_err("empty voters should fail");
        assert!(err.contains("empty voter set"));
    }

    #[test]
    fn test_parse_voter_ids_csv_invalid_rejected() {
        let err = parse_voter_ids_csv("1,a").expect_err("invalid voter should fail");
        assert!(err.contains("invalid node id"));
    }

    #[test]
    fn test_parse_forward_hops_default_zero() {
        let headers = HeaderMap::new();
        let hops = KNetworkServer::parse_forward_hops(&headers).expect("parse hops");
        assert_eq!(hops, 0);
    }

    #[test]
    fn test_parse_forward_hops_ok() {
        let mut headers = HeaderMap::new();
        headers.insert(KLOG_FORWARD_HOPS_HEADER, "2".parse().expect("header value"));
        let hops = KNetworkServer::parse_forward_hops(&headers).expect("parse hops");
        assert_eq!(hops, 2);
    }

    #[test]
    fn test_parse_forward_hops_invalid_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert(
            KLOG_FORWARD_HOPS_HEADER,
            "abc".parse().expect("header value"),
        );
        let err = KNetworkServer::parse_forward_hops(&headers).expect_err("invalid hops");
        assert!(err.contains("invalid"));
        assert!(err.contains(KLOG_FORWARD_HOPS_HEADER));
    }
}
