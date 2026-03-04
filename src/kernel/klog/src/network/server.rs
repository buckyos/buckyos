use super::request::{
    KLogAdminRequestType, KLogAppendRequest, KLogClusterStateResponse, KLogDataRequestType,
    KLogMetaDeleteRequest, KLogMetaPutRequest, KLogMetaQueryRequest, KLogQueryRequest, RaftRequest,
    RaftRequestType, RaftResponse,
};
use crate::error::{KLogErrorEnvelope, KLogServiceError, generate_trace_id};
use crate::service::{KLogQueryService, KLogWriteService};
use crate::state_store::KLogStateStoreManagerRef;
use crate::{KNode, KNodeId, KRaftRef};
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
    write_service: Option<KLogWriteService>,
    query_service: Option<KLogQueryService>,
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
            write_service: self.state_store_manager.clone().map(|state_store_manager| {
                KLogWriteService::new("KNetworkServer", self.raft.clone(), state_store_manager)
            }),
            query_service: self.state_store_manager.clone().map(|state_store_manager| {
                KLogQueryService::new("KNetworkServer", self.raft.clone(), state_store_manager)
            }),
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
        let data_query_path = KLogDataRequestType::Query.klog_path();
        let data_meta_put_path = KLogDataRequestType::MetaPut.klog_path();
        let data_meta_delete_path = KLogDataRequestType::MetaDelete.klog_path();
        let data_meta_query_path = KLogDataRequestType::MetaQuery.klog_path();
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
            .route(&data_query_path, get(Self::handle_data_query_request))
            .route(&data_meta_put_path, post(Self::handle_meta_put_request))
            .route(
                &data_meta_delete_path,
                post(Self::handle_meta_delete_request),
            )
            .route(&data_meta_query_path, get(Self::handle_meta_query_request))
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
            "KNetworkServer start listening at {}, cluster_name={}, cluster_id={}, control_limit_bytes={}, snapshot_limit_bytes={}, control_concurrency={}, snapshot_concurrency={}, control_timeout_ms={}, snapshot_timeout_ms={}, admin_local_only={}, data_append_path={}, data_query_path={}, data_meta_put_path={}, data_meta_delete_path={}, data_meta_query_path={}, admin_add_learner_path={}, admin_remove_learner_path={}, admin_change_membership_path={}, admin_cluster_state_path={}",
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
            data_query_path,
            data_meta_put_path,
            data_meta_delete_path,
            data_meta_query_path,
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
        let Some(write_service) = state.write_service.as_ref() else {
            let msg = "KNetworkServer data append rejected: state store manager is not configured"
                .to_string();
            error!("{}", msg);
            return Self::error_response(StatusCode::INTERNAL_SERVER_ERROR, msg);
        };

        match write_service.append(&headers, req).await {
            Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
            Err(err) => Self::service_error_response(err),
        }
    }

    async fn handle_data_query_request(
        State(state): State<KNetworkServerState>,
        headers: HeaderMap,
        Query(query): Query<KLogQueryRequest>,
    ) -> Response {
        let Some(query_service) = state.query_service.as_ref() else {
            let msg = "KNetworkServer data query rejected: state store manager is not configured"
                .to_string();
            error!("{}", msg);
            return Self::error_response(StatusCode::INTERNAL_SERVER_ERROR, msg);
        };

        match query_service.query(&headers, query).await {
            Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
            Err(err) => Self::service_error_response(err),
        }
    }

    async fn handle_meta_put_request(
        State(state): State<KNetworkServerState>,
        headers: HeaderMap,
        Json(req): Json<KLogMetaPutRequest>,
    ) -> Response {
        let Some(write_service) = state.write_service.as_ref() else {
            let msg = "KNetworkServer meta put rejected: state store manager is not configured"
                .to_string();
            error!("{}", msg);
            return Self::error_response(StatusCode::INTERNAL_SERVER_ERROR, msg);
        };

        match write_service.put_meta(&headers, req).await {
            Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
            Err(err) => Self::service_error_response(err),
        }
    }

    async fn handle_meta_delete_request(
        State(state): State<KNetworkServerState>,
        headers: HeaderMap,
        Json(req): Json<KLogMetaDeleteRequest>,
    ) -> Response {
        let Some(write_service) = state.write_service.as_ref() else {
            let msg = "KNetworkServer meta delete rejected: state store manager is not configured"
                .to_string();
            error!("{}", msg);
            return Self::error_response(StatusCode::INTERNAL_SERVER_ERROR, msg);
        };

        match write_service.delete_meta(&headers, req).await {
            Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
            Err(err) => Self::service_error_response(err),
        }
    }

    async fn handle_meta_query_request(
        State(state): State<KNetworkServerState>,
        headers: HeaderMap,
        Query(query): Query<KLogMetaQueryRequest>,
    ) -> Response {
        let Some(query_service) = state.query_service.as_ref() else {
            let msg = "KNetworkServer meta query rejected: state store manager is not configured"
                .to_string();
            error!("{}", msg);
            return Self::error_response(StatusCode::INTERNAL_SERVER_ERROR, msg);
        };

        match query_service.query_meta(&headers, query).await {
            Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
            Err(err) => Self::service_error_response(err),
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

    fn error_response(status: StatusCode, msg: String) -> Response {
        let trace_id = generate_trace_id();
        let envelope = KLogErrorEnvelope::from_http_status(status.as_u16(), msg, trace_id);
        (status, Json(envelope)).into_response()
    }

    fn service_error_response(err: KLogServiceError) -> Response {
        let status =
            StatusCode::from_u16(err.http_status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (status, Json(err.error)).into_response()
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
    use super::parse_voter_ids_csv;

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
}
