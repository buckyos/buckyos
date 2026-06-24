use crate::error::{KLogErrorCode, KLogServiceError, normalize_trace_id};
use crate::network::{
    KDataClient, KLOG_FORWARD_HOPS_HEADER, KLOG_FORWARDED_BY_HEADER, KLOG_TRACE_ID_HEADER,
    KLogAppendRequest, KLogAppendResponse, KLogMetaChangesRequest, KLogMetaChangesResponse,
    KLogMetaDeleteRequest, KLogMetaDeleteResponse, KLogMetaPutRequest, KLogMetaPutResponse,
    KLogMetaQueryRequest, KLogMetaQueryResponse, KLogQueryRequest, KLogQueryResponse,
};
use crate::state_store::{
    KLogMetaChangeCursor, KLogMetaChangeQuery, KLogQuery, KLogQueryOrder, KLogStateStoreManagerRef,
};
use crate::{
    KClusterTransportConfig, KClusterTransportMode, KLogEntry, KLogLevel, KLogMetaEntry,
    KLogMetaTxAction, KLogMetaTxRequest, KLogMetaTxResponse, KLogRequest, KLogResponse, KNode,
    KRaftRef,
};
use axum::http::{HeaderMap, StatusCode};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::{Duration, Instant, sleep};

pub const DATA_QUERY_DEFAULT_LIMIT: usize = 200;
pub const DATA_QUERY_MAX_LIMIT: usize = 2_000;
pub const DATA_QUERY_MAX_FORWARD_HOPS: u32 = 2;
pub const DATA_APPEND_MAX_MESSAGE_BYTES: usize = 64 * 1024;
pub const DATA_APPEND_MAX_REQUEST_ID_BYTES: usize = 128;
pub const DATA_APPEND_MAX_FORWARD_HOPS: u32 = 2;
pub const META_KEY_MAX_BYTES: usize = 256;
pub const META_VALUE_MAX_BYTES: usize = 256 * 1024;
pub const META_QUERY_DEFAULT_LIMIT: usize = 200;
pub const META_QUERY_MAX_LIMIT: usize = 2_000;
pub const META_CHANGES_MAX_WAIT_MS: u64 = 2_000;
pub const META_CHANGES_POLL_INTERVAL_MS: u64 = 100;
pub const META_RW_MAX_FORWARD_HOPS: u32 = 2;
const DEFAULT_WRITE_QUORUM_ACK_MAX_AGE_MS: u64 = 1_000;
const DEFAULT_WRITE_QUORUM_ACK_WAIT_MS: u64 = 300;
const DEFAULT_WRITE_QUORUM_ACK_POLL_MS: u64 = 50;
pub type KServiceResult<T> = Result<T, KLogServiceError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KLogWriteQuorumPolicy {
    pub max_age_ms: u64,
    pub wait_ms: u64,
    pub poll_ms: u64,
}

impl Default for KLogWriteQuorumPolicy {
    fn default() -> Self {
        Self {
            max_age_ms: DEFAULT_WRITE_QUORUM_ACK_MAX_AGE_MS,
            wait_ms: DEFAULT_WRITE_QUORUM_ACK_WAIT_MS,
            poll_ms: DEFAULT_WRITE_QUORUM_ACK_POLL_MS,
        }
    }
}

fn with_forward_error_context(
    mut forward_err: KLogServiceError,
    context: String,
    leader_node: KNode,
) -> KLogServiceError {
    let upstream_message = forward_err.error.message.clone();
    error!("{}", context);
    forward_err.error.message = format!("{}; upstream={}", context, upstream_message);
    if forward_err.error.leader_hint.is_none() {
        forward_err.error.leader_hint = Some(leader_node);
    }
    forward_err
}

fn normalize_node_name(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
}

fn resolve_local_node_name(
    metrics: &openraft::RaftMetrics<crate::KNodeId, crate::KNode>,
) -> String {
    metrics
        .membership_config
        .nodes()
        .find_map(|(id, node)| {
            (*id == metrics.id)
                .then(|| normalize_node_name(node.node_name.as_deref()))
                .flatten()
        })
        .unwrap_or_else(|| format!("raft-node-{}", metrics.id))
}

#[derive(Clone)]
pub struct KLogWriteService {
    service_name: &'static str,
    raft: KRaftRef,
    state_store_manager: KLogStateStoreManagerRef,
    data_client: KDataClient,
    write_quorum_policy: KLogWriteQuorumPolicy,
}

impl KLogWriteService {
    pub fn new(
        service_name: &'static str,
        raft: KRaftRef,
        state_store_manager: KLogStateStoreManagerRef,
    ) -> Self {
        Self {
            service_name,
            raft,
            state_store_manager,
            data_client: KDataClient::new(),
            write_quorum_policy: KLogWriteQuorumPolicy::default(),
        }
    }

    pub fn with_transport_mode(mut self, transport_mode: KClusterTransportMode) -> Self {
        self.data_client = self.data_client.with_transport_mode(transport_mode);
        self
    }

    pub fn with_transport_config(mut self, transport: KClusterTransportConfig) -> Self {
        self.data_client = self.data_client.with_transport_config(transport);
        self
    }

    pub fn with_write_quorum_policy(mut self, policy: KLogWriteQuorumPolicy) -> Self {
        self.write_quorum_policy = policy;
        self
    }

    async fn ensure_fresh_quorum_for_local_leader(
        &self,
        operation: &str,
        context: &str,
        trace_id: &str,
    ) -> KServiceResult<()> {
        let policy = self.write_quorum_policy;
        let deadline = Instant::now() + Duration::from_millis(policy.wait_ms);
        loop {
            let metrics = self.raft.metrics().borrow().clone();
            if !metrics.state.is_leader() {
                return Ok(());
            }

            let voter_count = metrics.membership_config.voter_ids().count();
            if voter_count <= 1 {
                return Ok(());
            }

            if let Some(age_ms) = metrics.millis_since_quorum_ack
                && age_ms <= policy.max_age_ms
            {
                return Ok(());
            }

            if Instant::now() >= deadline {
                let msg = format!(
                    "{} {} rejected: local leader has no fresh quorum ack, {}, local_node_id={}, current_leader={:?}, voters={}, millis_since_quorum_ack={:?}, max_age_ms={}",
                    self.service_name,
                    operation,
                    context,
                    metrics.id,
                    metrics.current_leader,
                    voter_count,
                    metrics.millis_since_quorum_ack,
                    policy.max_age_ms
                );
                warn!("{}", msg);
                return Err(self.service_error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    KLogErrorCode::Unavailable,
                    msg,
                    trace_id,
                ));
            }

            sleep(Duration::from_millis(policy.poll_ms)).await;
        }
    }

    pub async fn append(
        &self,
        headers: &HeaderMap,
        req: KLogAppendRequest,
    ) -> KServiceResult<KLogAppendResponse> {
        let trace_id = self.resolve_trace_id(headers);
        if req.message.trim().is_empty() {
            let msg = format!("{} data append rejected: empty message", self.service_name);
            error!("{}", msg);
            return Err(self.service_error(
                StatusCode::BAD_REQUEST,
                KLogErrorCode::InvalidArgument,
                msg,
                &trace_id,
            ));
        }

        if req.message.len() > DATA_APPEND_MAX_MESSAGE_BYTES {
            let msg = format!(
                "{} data append rejected: message too large, bytes={}, max_bytes={}",
                self.service_name,
                req.message.len(),
                DATA_APPEND_MAX_MESSAGE_BYTES
            );
            error!("{}", msg);
            return Err(self.service_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                KLogErrorCode::PayloadTooLarge,
                msg,
                &trace_id,
            ));
        }

        let request_id = req
            .request_id
            .as_ref()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        if let Some(request_id) = request_id.as_ref()
            && request_id.len() > DATA_APPEND_MAX_REQUEST_ID_BYTES
        {
            let msg = format!(
                "{} data append rejected: request_id too large, bytes={}, max_bytes={}",
                self.service_name,
                request_id.len(),
                DATA_APPEND_MAX_REQUEST_ID_BYTES
            );
            error!("{}", msg);
            return Err(self.service_error(
                StatusCode::BAD_REQUEST,
                KLogErrorCode::InvalidArgument,
                msg,
                &trace_id,
            ));
        }

        if let Some(request_id) = request_id.as_ref()
            && let Some(existing_id) = self
                .state_store_manager
                .find_recent_request_id(request_id)
                .await
        {
            info!(
                "{} data append dedup hit before raft write: request_id={}, existing_id={}",
                self.service_name, request_id, existing_id
            );
            return Ok(KLogAppendResponse { id: existing_id });
        }

        let forward_hops = self
            .parse_forward_hops(headers, "data append")
            .map_err(|msg| {
                error!("{}", msg);
                self.service_error(
                    StatusCode::BAD_REQUEST,
                    KLogErrorCode::InvalidArgument,
                    msg,
                    &trace_id,
                )
            })?;
        let forwarded_by = headers
            .get(KLOG_FORWARDED_BY_HEADER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("-");

        if forward_hops > DATA_APPEND_MAX_FORWARD_HOPS {
            let msg = format!(
                "{} data append rejected: too many forward hops, hops={}, max_hops={}, forwarded_by={}",
                self.service_name, forward_hops, DATA_APPEND_MAX_FORWARD_HOPS, forwarded_by
            );
            error!("{}", msg);
            return Err(self.service_error(
                StatusCode::BAD_GATEWAY,
                KLogErrorCode::LeaderUnavailable,
                msg,
                &trace_id,
            ));
        }

        let metrics = self.raft.metrics().borrow().clone();
        let local_node_id = metrics.id;
        let local_node_name = resolve_local_node_name(&metrics);
        let level = req.level.unwrap_or(KLogLevel::Info);
        let source = req
            .source
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string());
        let attrs = req.attrs.unwrap_or_default();
        let origin_node_name = normalize_node_name(req.node_name.as_deref())
            .unwrap_or_else(|| local_node_name.clone());
        let req = KLogAppendRequest {
            message: req.message,
            timestamp: req.timestamp.or_else(|| Some(now_millis())),
            node_name: Some(origin_node_name.clone()),
            level: Some(level),
            source: source.clone(),
            attrs: Some(attrs.clone()),
            request_id: request_id.clone(),
        };

        let item = self.state_store_manager.prepare_append_entry(KLogEntry {
            id: 0,
            timestamp: req.timestamp.unwrap_or(0),
            node_name: origin_node_name.clone(),
            request_id,
            level,
            source,
            attrs,
            message: req.message.clone(),
        });
        let requested_id = item.id;

        info!(
            "{} data append request: trace_id={}, id={}, request_id={:?}, timestamp={}, node_name={}, level={:?}, source={:?}, attrs_len={}, msg_len={}, local_raft_node_id={}, local_node_name={}, current_leader={:?}, forward_hops={}, forwarded_by={}",
            self.service_name,
            trace_id,
            item.id,
            item.request_id.as_deref(),
            item.timestamp,
            item.node_name,
            item.level,
            item.source.as_deref(),
            item.attrs.len(),
            item.message.len(),
            local_node_id,
            local_node_name,
            metrics.current_leader,
            forward_hops,
            forwarded_by
        );

        let quorum_context = format!("requested_id={}", requested_id);
        self.ensure_fresh_quorum_for_local_leader("data append", &quorum_context, &trace_id)
            .await?;

        match self
            .raft
            .client_write(KLogRequest::AppendLog { item })
            .await
        {
            Ok(resp) => match resp.data {
                KLogResponse::AppendOk { id } => {
                    info!("{} data append committed: id={}", self.service_name, id);
                    Ok(KLogAppendResponse { id })
                }
                KLogResponse::Err(err_msg) => {
                    let msg = format!(
                        "{} data append failed in state machine: requested_id={}, err={}",
                        self.service_name, requested_id, err_msg
                    );
                    error!("{}", msg);
                    Err(self.service_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        KLogErrorCode::Internal,
                        msg,
                        &trace_id,
                    ))
                }
                other => {
                    let msg = format!(
                        "{} data append unexpected response: requested_id={}, response={:?}",
                        self.service_name, requested_id, other
                    );
                    error!("{}", msg);
                    Err(self.service_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        KLogErrorCode::Internal,
                        msg,
                        &trace_id,
                    ))
                }
            },
            Err(err) => {
                if let Some(forward) = err.forward_to_leader::<KNode>() {
                    if forward_hops >= DATA_APPEND_MAX_FORWARD_HOPS {
                        let msg = format!(
                            "{} data append forward aborted due to hop limit: local_node_id={}, requested_id={}, leader_id={:?}, leader_node={:?}, hops={}, max_hops={}",
                            self.service_name,
                            local_node_id,
                            requested_id,
                            forward.leader_id,
                            forward.leader_node,
                            forward_hops,
                            DATA_APPEND_MAX_FORWARD_HOPS
                        );
                        error!("{}", msg);
                        return Err(self.service_error(
                            StatusCode::BAD_GATEWAY,
                            KLogErrorCode::LeaderUnavailable,
                            msg,
                            &trace_id,
                        ));
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
                            "{} data append can not resolve leader node for forwarding: local_node_id={}, requested_id={}, leader_id={:?}",
                            self.service_name, local_node_id, requested_id, forward.leader_id
                        );
                        warn!("{}", msg);
                        return Err(self
                            .service_error(
                                StatusCode::SERVICE_UNAVAILABLE,
                                KLogErrorCode::LeaderUnavailable,
                                msg,
                                &trace_id,
                            )
                            .with_leader_hint(forward.leader_node.clone()));
                    };

                    let target_hops = forward_hops + 1;
                    warn!(
                        "{} data append forwarding to leader: local_node_id={}, requested_id={}, leader_id={}, leader_addr={}:{}, hops={} -> {}",
                        self.service_name,
                        local_node_id,
                        requested_id,
                        leader_node.id,
                        leader_node.addr,
                        leader_node.port,
                        forward_hops,
                        target_hops
                    );
                    match self
                        .data_client
                        .append_to_node(&leader_node, &req, target_hops, local_node_id, &trace_id)
                        .await
                    {
                        Ok(resp) => {
                            info!(
                                "{} data append forwarded and committed: trace_id={}, local_node_id={}, requested_id={}, committed_id={}, leader_id={}, hops={}",
                                self.service_name,
                                trace_id,
                                local_node_id,
                                requested_id,
                                resp.id,
                                leader_node.id,
                                target_hops
                            );
                            Ok(resp)
                        }
                        Err(forward_err) => {
                            let msg = format!(
                                "{} data append forward failed: local_node_id={}, requested_id={}, leader_id={}, err={}",
                                self.service_name,
                                local_node_id,
                                requested_id,
                                leader_node.id,
                                forward_err
                            );
                            Err(with_forward_error_context(
                                forward_err,
                                msg,
                                leader_node.clone(),
                            ))
                        }
                    }
                } else {
                    let msg = format!(
                        "{} data append raft client_write failed: requested_id={}, err={}",
                        self.service_name, requested_id, err
                    );
                    error!("{}", msg);
                    Err(self.service_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        KLogErrorCode::Internal,
                        msg,
                        &trace_id,
                    ))
                }
            }
        }
    }

    pub async fn put_meta(
        &self,
        headers: &HeaderMap,
        req: KLogMetaPutRequest,
    ) -> KServiceResult<KLogMetaPutResponse> {
        let trace_id = self.resolve_trace_id(headers);
        let key = req.key.trim().to_string();
        if key.is_empty() {
            let msg = format!("{} meta put rejected: empty key", self.service_name);
            error!("{}", msg);
            return Err(self.service_error(
                StatusCode::BAD_REQUEST,
                KLogErrorCode::InvalidArgument,
                msg,
                &trace_id,
            ));
        }
        if key.len() > META_KEY_MAX_BYTES {
            let msg = format!(
                "{} meta put rejected: key too large, bytes={}, max_bytes={}",
                self.service_name,
                key.len(),
                META_KEY_MAX_BYTES
            );
            error!("{}", msg);
            return Err(self.service_error(
                StatusCode::BAD_REQUEST,
                KLogErrorCode::InvalidArgument,
                msg,
                &trace_id,
            ));
        }
        if req.value.len() > META_VALUE_MAX_BYTES {
            let msg = format!(
                "{} meta put rejected: value too large, bytes={}, max_bytes={}",
                self.service_name,
                req.value.len(),
                META_VALUE_MAX_BYTES
            );
            error!("{}", msg);
            return Err(self.service_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                KLogErrorCode::PayloadTooLarge,
                msg,
                &trace_id,
            ));
        }

        let forward_hops = self
            .parse_forward_hops(headers, "meta put")
            .map_err(|msg| {
                error!("{}", msg);
                self.service_error(
                    StatusCode::BAD_REQUEST,
                    KLogErrorCode::InvalidArgument,
                    msg,
                    &trace_id,
                )
            })?;
        let forwarded_by = headers
            .get(KLOG_FORWARDED_BY_HEADER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("-");
        if forward_hops > META_RW_MAX_FORWARD_HOPS {
            let msg = format!(
                "{} meta put rejected: too many forward hops, hops={}, max_hops={}, forwarded_by={}",
                self.service_name, forward_hops, META_RW_MAX_FORWARD_HOPS, forwarded_by
            );
            error!("{}", msg);
            return Err(self.service_error(
                StatusCode::BAD_GATEWAY,
                KLogErrorCode::LeaderUnavailable,
                msg,
                &trace_id,
            ));
        }

        let metrics = self.raft.metrics().borrow().clone();
        let local_node_id = metrics.id;
        let local_node_name = resolve_local_node_name(&metrics);
        let expected_revision = req.expected_revision;
        let origin_node_name = normalize_node_name(req.node_name.as_deref())
            .unwrap_or_else(|| local_node_name.clone());
        let item = KLogMetaEntry {
            key: key.clone(),
            value: req.value.clone(),
            // Meta write audit fields are owned by the raft service, not client input.
            updated_at: now_millis(),
            updated_by_node_name: origin_node_name.clone(),
            ..KLogMetaEntry::default()
        };
        info!(
            "{} meta put request: trace_id={}, key={}, value_len={}, updated_at={}, updated_by_node_name={}, expected_revision={:?}, local_raft_node_id={}, local_node_name={}, current_leader={:?}, forward_hops={}, forwarded_by={}",
            self.service_name,
            trace_id,
            item.key,
            item.value.len(),
            item.updated_at,
            item.updated_by_node_name,
            expected_revision,
            local_node_id,
            local_node_name,
            metrics.current_leader,
            forward_hops,
            forwarded_by
        );

        let quorum_context = format!("key={}", item.key);
        self.ensure_fresh_quorum_for_local_leader("meta put", &quorum_context, &trace_id)
            .await?;

        match self
            .raft
            .client_write(KLogRequest::PutMeta {
                item,
                expected_revision,
            })
            .await
        {
            Ok(resp) => match resp.data {
                KLogResponse::MetaPutOk { item } => {
                    info!(
                        "{} meta put committed: key={}, mod_revision={}, create_revision={}, version={}",
                        self.service_name,
                        item.key,
                        item.effective_mod_revision(),
                        item.effective_create_revision(),
                        item.effective_version()
                    );
                    Ok(KLogMetaPutResponse::from_entry(&item))
                }
                KLogResponse::MetaPutConflict {
                    key,
                    expected_revision,
                    current_revision,
                } => {
                    let msg = format!(
                        "{} meta put version conflict: key={}, expected_revision={}, current_revision={:?}",
                        self.service_name, key, expected_revision, current_revision
                    );
                    warn!("{}", msg);
                    Err(self.service_error(
                        StatusCode::CONFLICT,
                        KLogErrorCode::VersionConflict,
                        msg,
                        &trace_id,
                    ))
                }
                KLogResponse::Err(err_msg) => {
                    let msg = format!(
                        "{} meta put failed in state machine: key={}, err={}",
                        self.service_name, key, err_msg
                    );
                    error!("{}", msg);
                    Err(self.service_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        KLogErrorCode::Internal,
                        msg,
                        &trace_id,
                    ))
                }
                other => {
                    let msg = format!(
                        "{} meta put unexpected response: key={}, response={:?}",
                        self.service_name, key, other
                    );
                    error!("{}", msg);
                    Err(self.service_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        KLogErrorCode::Internal,
                        msg,
                        &trace_id,
                    ))
                }
            },
            Err(err) => {
                if let Some(forward) = err.forward_to_leader::<KNode>() {
                    if forward_hops >= META_RW_MAX_FORWARD_HOPS {
                        let msg = format!(
                            "{} meta put forward aborted due to hop limit: local_node_id={}, key={}, leader_id={:?}, leader_node={:?}, hops={}, max_hops={}",
                            self.service_name,
                            local_node_id,
                            key,
                            forward.leader_id,
                            forward.leader_node,
                            forward_hops,
                            META_RW_MAX_FORWARD_HOPS
                        );
                        error!("{}", msg);
                        return Err(self.service_error(
                            StatusCode::BAD_GATEWAY,
                            KLogErrorCode::LeaderUnavailable,
                            msg,
                            &trace_id,
                        ));
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
                            "{} meta put can not resolve leader node for forwarding: local_node_id={}, key={}, leader_id={:?}",
                            self.service_name, local_node_id, key, forward.leader_id
                        );
                        warn!("{}", msg);
                        return Err(self
                            .service_error(
                                StatusCode::SERVICE_UNAVAILABLE,
                                KLogErrorCode::LeaderUnavailable,
                                msg,
                                &trace_id,
                            )
                            .with_leader_hint(forward.leader_node.clone()));
                    };

                    let target_hops = forward_hops + 1;
                    warn!(
                        "{} meta put forwarding to leader: local_node_id={}, key={}, leader_id={}, leader_addr={}:{}, hops={} -> {}",
                        self.service_name,
                        local_node_id,
                        key,
                        leader_node.id,
                        leader_node.addr,
                        leader_node.port,
                        forward_hops,
                        target_hops
                    );
                    match self
                        .data_client
                        .put_meta_to_node(
                            &leader_node,
                            &KLogMetaPutRequest {
                                key: key.clone(),
                                value: req.value,
                                node_name: Some(origin_node_name.clone()),
                                expected_revision: req.expected_revision,
                            },
                            target_hops,
                            local_node_id,
                            &trace_id,
                        )
                        .await
                    {
                        Ok(resp) => {
                            info!(
                                "{} meta put forwarded and committed: trace_id={}, local_node_id={}, key={}, leader_id={}, hops={}",
                                self.service_name,
                                trace_id,
                                local_node_id,
                                resp.key,
                                leader_node.id,
                                target_hops
                            );
                            Ok(resp)
                        }
                        Err(forward_err) => {
                            let msg = format!(
                                "{} meta put forward failed: local_node_id={}, key={}, leader_id={}, err={}",
                                self.service_name, local_node_id, key, leader_node.id, forward_err
                            );
                            Err(with_forward_error_context(
                                forward_err,
                                msg,
                                leader_node.clone(),
                            ))
                        }
                    }
                } else {
                    let msg = format!(
                        "{} meta put raft client_write failed: key={}, err={}",
                        self.service_name, key, err
                    );
                    error!("{}", msg);
                    Err(self.service_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        KLogErrorCode::Internal,
                        msg,
                        &trace_id,
                    ))
                }
            }
        }
    }

    pub async fn exec_meta_tx(
        &self,
        headers: &HeaderMap,
        req: KLogMetaTxRequest,
    ) -> KServiceResult<KLogMetaTxResponse> {
        let trace_id = self.resolve_trace_id(headers);
        if req.actions.is_empty() {
            let msg = format!("{} meta tx rejected: empty actions", self.service_name);
            error!("{}", msg);
            return Err(self.service_error(
                StatusCode::BAD_REQUEST,
                KLogErrorCode::InvalidArgument,
                msg,
                &trace_id,
            ));
        }

        for (action_key, action) in req.actions.iter() {
            if action_key.trim().is_empty() || action.key().trim().is_empty() {
                let msg = format!("{} meta tx rejected: empty key", self.service_name);
                error!("{}", msg);
                return Err(self.service_error(
                    StatusCode::BAD_REQUEST,
                    KLogErrorCode::InvalidArgument,
                    msg,
                    &trace_id,
                ));
            }
            if action_key != action.key() {
                let msg = format!(
                    "{} meta tx rejected: action key mismatch, map_key={}, action_key={}",
                    self.service_name,
                    action_key,
                    action.key()
                );
                error!("{}", msg);
                return Err(self.service_error(
                    StatusCode::BAD_REQUEST,
                    KLogErrorCode::InvalidArgument,
                    msg,
                    &trace_id,
                ));
            }
            if action_key.len() > META_KEY_MAX_BYTES {
                let msg = format!(
                    "{} meta tx rejected: key too large, key={}, bytes={}, max_bytes={}",
                    self.service_name,
                    action_key,
                    action_key.len(),
                    META_KEY_MAX_BYTES
                );
                error!("{}", msg);
                return Err(self.service_error(
                    StatusCode::BAD_REQUEST,
                    KLogErrorCode::InvalidArgument,
                    msg,
                    &trace_id,
                ));
            }
            if let KLogMetaTxAction::Put { item, .. } = action
                && item.value.len() > META_VALUE_MAX_BYTES
            {
                let msg = format!(
                    "{} meta tx rejected: value too large, key={}, bytes={}, max_bytes={}",
                    self.service_name,
                    action_key,
                    item.value.len(),
                    META_VALUE_MAX_BYTES
                );
                error!("{}", msg);
                return Err(self.service_error(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    KLogErrorCode::PayloadTooLarge,
                    msg,
                    &trace_id,
                ));
            }
        }

        if let Some(guard) = req.guard.as_ref() {
            if guard.key.trim().is_empty() {
                let msg = format!("{} meta tx rejected: empty guard key", self.service_name);
                error!("{}", msg);
                return Err(self.service_error(
                    StatusCode::BAD_REQUEST,
                    KLogErrorCode::InvalidArgument,
                    msg,
                    &trace_id,
                ));
            }
            if guard.key.len() > META_KEY_MAX_BYTES {
                let msg = format!(
                    "{} meta tx rejected: guard key too large, key={}, bytes={}, max_bytes={}",
                    self.service_name,
                    guard.key,
                    guard.key.len(),
                    META_KEY_MAX_BYTES
                );
                error!("{}", msg);
                return Err(self.service_error(
                    StatusCode::BAD_REQUEST,
                    KLogErrorCode::InvalidArgument,
                    msg,
                    &trace_id,
                ));
            }
        }

        let forward_hops = self.parse_forward_hops(headers, "meta tx").map_err(|msg| {
            error!("{}", msg);
            self.service_error(
                StatusCode::BAD_REQUEST,
                KLogErrorCode::InvalidArgument,
                msg,
                &trace_id,
            )
        })?;
        let forwarded_by = headers
            .get(KLOG_FORWARDED_BY_HEADER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("-");
        if forward_hops > META_RW_MAX_FORWARD_HOPS {
            let msg = format!(
                "{} meta tx rejected: too many forward hops, hops={}, max_hops={}, forwarded_by={}",
                self.service_name, forward_hops, META_RW_MAX_FORWARD_HOPS, forwarded_by
            );
            error!("{}", msg);
            return Err(self.service_error(
                StatusCode::BAD_GATEWAY,
                KLogErrorCode::LeaderUnavailable,
                msg,
                &trace_id,
            ));
        }

        let metrics = self.raft.metrics().borrow().clone();
        let local_node_id = metrics.id;
        info!(
            "{} meta tx request: trace_id={}, actions={}, guard={:?}, local_raft_node_id={}, current_leader={:?}, forward_hops={}, forwarded_by={}",
            self.service_name,
            trace_id,
            req.actions.len(),
            req.guard,
            local_node_id,
            metrics.current_leader,
            forward_hops,
            forwarded_by
        );

        let quorum_context = format!("actions={}", req.actions.len());
        self.ensure_fresh_quorum_for_local_leader("meta tx", &quorum_context, &trace_id)
            .await?;

        match self
            .raft
            .client_write(KLogRequest::ExecMetaTx { tx: req.clone() })
            .await
        {
            Ok(resp) => match resp.data {
                KLogResponse::MetaTxOk { response } => {
                    info!(
                        "{} meta tx committed: revisions={:?}, meta_versions={:?}",
                        self.service_name, response.revisions, response.meta_versions
                    );
                    Ok(response)
                }
                KLogResponse::MetaTxConflict {
                    key,
                    expected_revision,
                    current_revision,
                } => {
                    let msg = format!(
                        "{} meta tx version conflict: key={}, expected_revision={}, current_revision={:?}",
                        self.service_name, key, expected_revision, current_revision
                    );
                    warn!("{}", msg);
                    Err(self.service_error(
                        StatusCode::CONFLICT,
                        KLogErrorCode::VersionConflict,
                        msg,
                        &trace_id,
                    ))
                }
                KLogResponse::Err(err_msg) => {
                    let msg = format!(
                        "{} meta tx failed in state machine: actions={}, err={}",
                        self.service_name,
                        req.actions.len(),
                        err_msg
                    );
                    error!("{}", msg);
                    Err(self.service_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        KLogErrorCode::Internal,
                        msg,
                        &trace_id,
                    ))
                }
                other => {
                    let msg = format!(
                        "{} meta tx unexpected response: response={:?}",
                        self.service_name, other
                    );
                    error!("{}", msg);
                    Err(self.service_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        KLogErrorCode::Internal,
                        msg,
                        &trace_id,
                    ))
                }
            },
            Err(err) => {
                if let Some(forward) = err.forward_to_leader::<KNode>() {
                    if forward_hops >= META_RW_MAX_FORWARD_HOPS {
                        let msg = format!(
                            "{} meta tx forward aborted due to hop limit: local_node_id={}, leader_id={:?}, leader_node={:?}, hops={}, max_hops={}",
                            self.service_name,
                            local_node_id,
                            forward.leader_id,
                            forward.leader_node,
                            forward_hops,
                            META_RW_MAX_FORWARD_HOPS
                        );
                        error!("{}", msg);
                        return Err(self.service_error(
                            StatusCode::BAD_GATEWAY,
                            KLogErrorCode::LeaderUnavailable,
                            msg,
                            &trace_id,
                        ));
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
                            "{} meta tx can not resolve leader node for forwarding: local_node_id={}, leader_id={:?}",
                            self.service_name, local_node_id, forward.leader_id
                        );
                        warn!("{}", msg);
                        return Err(self
                            .service_error(
                                StatusCode::SERVICE_UNAVAILABLE,
                                KLogErrorCode::LeaderUnavailable,
                                msg,
                                &trace_id,
                            )
                            .with_leader_hint(forward.leader_node.clone()));
                    };

                    let target_hops = forward_hops + 1;
                    warn!(
                        "{} meta tx forwarding to leader: local_node_id={}, leader_id={}, leader_addr={}:{}, hops={} -> {}",
                        self.service_name,
                        local_node_id,
                        leader_node.id,
                        leader_node.addr,
                        leader_node.port,
                        forward_hops,
                        target_hops
                    );
                    return self
                        .data_client
                        .exec_meta_tx_to_node(
                            &leader_node,
                            &req,
                            target_hops,
                            local_node_id,
                            &trace_id,
                        )
                        .await
                        .map_err(|forward_err| {
                            let msg = format!(
                                "{} meta tx forward failed: local_node_id={}, leader_id={}, err={}",
                                self.service_name, local_node_id, leader_node.id, forward_err
                            );
                            with_forward_error_context(forward_err, msg, leader_node.clone())
                        });
                }

                let msg = format!(
                    "{} meta tx raft client_write failed: err={}",
                    self.service_name, err
                );
                error!("{}", msg);
                Err(self.service_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    KLogErrorCode::Internal,
                    msg,
                    &trace_id,
                ))
            }
        }
    }

    pub async fn delete_meta(
        &self,
        headers: &HeaderMap,
        req: KLogMetaDeleteRequest,
    ) -> KServiceResult<KLogMetaDeleteResponse> {
        let trace_id = self.resolve_trace_id(headers);
        let key = req.key.trim().to_string();
        if key.is_empty() {
            let msg = format!("{} meta delete rejected: empty key", self.service_name);
            error!("{}", msg);
            return Err(self.service_error(
                StatusCode::BAD_REQUEST,
                KLogErrorCode::InvalidArgument,
                msg,
                &trace_id,
            ));
        }

        let forward_hops = self
            .parse_forward_hops(headers, "meta delete")
            .map_err(|msg| {
                error!("{}", msg);
                self.service_error(
                    StatusCode::BAD_REQUEST,
                    KLogErrorCode::InvalidArgument,
                    msg,
                    &trace_id,
                )
            })?;
        let forwarded_by = headers
            .get(KLOG_FORWARDED_BY_HEADER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("-");
        if forward_hops > META_RW_MAX_FORWARD_HOPS {
            let msg = format!(
                "{} meta delete rejected: too many forward hops, hops={}, max_hops={}, forwarded_by={}",
                self.service_name, forward_hops, META_RW_MAX_FORWARD_HOPS, forwarded_by
            );
            error!("{}", msg);
            return Err(self.service_error(
                StatusCode::BAD_GATEWAY,
                KLogErrorCode::LeaderUnavailable,
                msg,
                &trace_id,
            ));
        }

        let metrics = self.raft.metrics().borrow().clone();
        let local_node_id = metrics.id;
        info!(
            "{} meta delete request: trace_id={}, key={}, local_node_id={}, current_leader={:?}, forward_hops={}, forwarded_by={}",
            self.service_name,
            trace_id,
            key,
            local_node_id,
            metrics.current_leader,
            forward_hops,
            forwarded_by
        );
        let quorum_context = format!("key={}", key);
        self.ensure_fresh_quorum_for_local_leader("meta delete", &quorum_context, &trace_id)
            .await?;

        match self
            .raft
            .client_write(KLogRequest::DeleteMeta { key: key.clone() })
            .await
        {
            Ok(resp) => match resp.data {
                KLogResponse::MetaDeleteOk {
                    key,
                    existed,
                    prev_meta,
                    meta_version,
                } => {
                    info!(
                        "{} meta delete committed: key={}, existed={}, prev_meta_revision={:?}, delete_meta_version={:?}",
                        self.service_name,
                        key,
                        existed,
                        prev_meta
                            .as_ref()
                            .map(KLogMetaEntry::effective_mod_revision),
                        meta_version
                    );
                    Ok(KLogMetaDeleteResponse {
                        key,
                        existed,
                        prev_meta,
                        meta_version,
                    })
                }
                KLogResponse::Err(err_msg) => {
                    let msg = format!(
                        "{} meta delete failed in state machine: key={}, err={}",
                        self.service_name, key, err_msg
                    );
                    error!("{}", msg);
                    Err(self.service_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        KLogErrorCode::Internal,
                        msg,
                        &trace_id,
                    ))
                }
                other => {
                    let msg = format!(
                        "{} meta delete unexpected response: key={}, response={:?}",
                        self.service_name, key, other
                    );
                    error!("{}", msg);
                    Err(self.service_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        KLogErrorCode::Internal,
                        msg,
                        &trace_id,
                    ))
                }
            },
            Err(err) => {
                if let Some(forward) = err.forward_to_leader::<KNode>() {
                    if forward_hops >= META_RW_MAX_FORWARD_HOPS {
                        let msg = format!(
                            "{} meta delete forward aborted due to hop limit: local_node_id={}, key={}, leader_id={:?}, leader_node={:?}, hops={}, max_hops={}",
                            self.service_name,
                            local_node_id,
                            key,
                            forward.leader_id,
                            forward.leader_node,
                            forward_hops,
                            META_RW_MAX_FORWARD_HOPS
                        );
                        error!("{}", msg);
                        return Err(self.service_error(
                            StatusCode::BAD_GATEWAY,
                            KLogErrorCode::LeaderUnavailable,
                            msg,
                            &trace_id,
                        ));
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
                            "{} meta delete can not resolve leader node for forwarding: local_node_id={}, key={}, leader_id={:?}",
                            self.service_name, local_node_id, key, forward.leader_id
                        );
                        warn!("{}", msg);
                        return Err(self
                            .service_error(
                                StatusCode::SERVICE_UNAVAILABLE,
                                KLogErrorCode::LeaderUnavailable,
                                msg,
                                &trace_id,
                            )
                            .with_leader_hint(forward.leader_node.clone()));
                    };

                    let target_hops = forward_hops + 1;
                    warn!(
                        "{} meta delete forwarding to leader: local_node_id={}, key={}, leader_id={}, leader_addr={}:{}, hops={} -> {}",
                        self.service_name,
                        local_node_id,
                        key,
                        leader_node.id,
                        leader_node.addr,
                        leader_node.port,
                        forward_hops,
                        target_hops
                    );
                    match self
                        .data_client
                        .delete_meta_to_node(
                            &leader_node,
                            &KLogMetaDeleteRequest { key: key.clone() },
                            target_hops,
                            local_node_id,
                            &trace_id,
                        )
                        .await
                    {
                        Ok(resp) => {
                            info!(
                                "{} meta delete forwarded and committed: trace_id={}, local_node_id={}, key={}, existed={}, leader_id={}, hops={}",
                                self.service_name,
                                trace_id,
                                local_node_id,
                                resp.key,
                                resp.existed,
                                leader_node.id,
                                target_hops
                            );
                            Ok(resp)
                        }
                        Err(forward_err) => {
                            let msg = format!(
                                "{} meta delete forward failed: local_node_id={}, key={}, leader_id={}, err={}",
                                self.service_name, local_node_id, key, leader_node.id, forward_err
                            );
                            Err(with_forward_error_context(
                                forward_err,
                                msg,
                                leader_node.clone(),
                            ))
                        }
                    }
                } else {
                    let msg = format!(
                        "{} meta delete raft client_write failed: key={}, err={}",
                        self.service_name, key, err
                    );
                    error!("{}", msg);
                    Err(self.service_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        KLogErrorCode::Internal,
                        msg,
                        &trace_id,
                    ))
                }
            }
        }
    }

    fn parse_forward_hops(&self, headers: &HeaderMap, op: &str) -> Result<u32, String> {
        let Some(raw) = headers.get(KLOG_FORWARD_HOPS_HEADER) else {
            return Ok(0);
        };
        let raw = raw.to_str().map_err(|e| {
            format!(
                "{} {} invalid {} header utf8: {}",
                self.service_name, op, KLOG_FORWARD_HOPS_HEADER, e
            )
        })?;
        raw.parse::<u32>().map_err(|e| {
            format!(
                "{} {} invalid {} header '{}': {}",
                self.service_name, op, KLOG_FORWARD_HOPS_HEADER, raw, e
            )
        })
    }

    fn resolve_trace_id(&self, headers: &HeaderMap) -> String {
        normalize_trace_id(
            headers
                .get(KLOG_TRACE_ID_HEADER)
                .and_then(|v| v.to_str().ok()),
        )
    }

    fn service_error(
        &self,
        status: StatusCode,
        code: KLogErrorCode,
        message: String,
        trace_id: &str,
    ) -> KLogServiceError {
        KLogServiceError::new(status.as_u16(), code, message, trace_id.to_string())
    }
}

#[derive(Clone)]
pub struct KLogQueryService {
    service_name: &'static str,
    raft: KRaftRef,
    state_store_manager: KLogStateStoreManagerRef,
    data_client: KDataClient,
}

impl KLogQueryService {
    pub fn new(
        service_name: &'static str,
        raft: KRaftRef,
        state_store_manager: KLogStateStoreManagerRef,
    ) -> Self {
        Self {
            service_name,
            raft,
            state_store_manager,
            data_client: KDataClient::new(),
        }
    }

    pub fn with_transport_mode(mut self, transport_mode: KClusterTransportMode) -> Self {
        self.data_client = self.data_client.with_transport_mode(transport_mode);
        self
    }

    pub fn with_transport_config(mut self, transport: KClusterTransportConfig) -> Self {
        self.data_client = self.data_client.with_transport_config(transport);
        self
    }

    pub async fn query(
        &self,
        headers: &HeaderMap,
        query: KLogQueryRequest,
    ) -> KServiceResult<KLogQueryResponse> {
        let trace_id = self.resolve_trace_id(headers);
        let strong_read = query.strong_read.unwrap_or(false);
        let forward_hops = self
            .parse_forward_hops(headers, "data query")
            .map_err(|msg| {
                error!("{}", msg);
                self.service_error(
                    StatusCode::BAD_REQUEST,
                    KLogErrorCode::InvalidArgument,
                    msg,
                    &trace_id,
                )
            })?;
        let forwarded_by = headers
            .get(KLOG_FORWARDED_BY_HEADER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("-");
        if strong_read {
            if forward_hops > DATA_QUERY_MAX_FORWARD_HOPS {
                let msg = format!(
                    "{} data query rejected: too many forward hops, hops={}, max_hops={}, forwarded_by={}",
                    self.service_name, forward_hops, DATA_QUERY_MAX_FORWARD_HOPS, forwarded_by
                );
                error!("{}", msg);
                return Err(self.service_error(
                    StatusCode::BAD_GATEWAY,
                    KLogErrorCode::LeaderUnavailable,
                    msg,
                    &trace_id,
                ));
            }

            let metrics = self.raft.metrics().borrow().clone();
            let local_node_id = metrics.id;
            match self.raft.ensure_linearizable().await {
                Ok(read_log_id) => {
                    info!(
                        "{} data query linearizable barrier passed: trace_id={}, read_log_id={:?}, local_node_id={}, forward_hops={}, forwarded_by={}",
                        self.service_name,
                        trace_id,
                        read_log_id,
                        local_node_id,
                        forward_hops,
                        forwarded_by
                    );
                }
                Err(err) => {
                    if let Some(forward) = err.forward_to_leader::<KNode>() {
                        if forward_hops >= DATA_QUERY_MAX_FORWARD_HOPS {
                            let msg = format!(
                                "{} data query forward aborted due to hop limit: local_node_id={}, leader_id={:?}, leader_node={:?}, hops={}, max_hops={}",
                                self.service_name,
                                local_node_id,
                                forward.leader_id,
                                forward.leader_node,
                                forward_hops,
                                DATA_QUERY_MAX_FORWARD_HOPS
                            );
                            error!("{}", msg);
                            return Err(self.service_error(
                                StatusCode::BAD_GATEWAY,
                                KLogErrorCode::LeaderUnavailable,
                                msg,
                                &trace_id,
                            ));
                        }

                        let leader_node = forward.leader_node.clone().or_else(|| {
                            forward.leader_id.and_then(|leader_id| {
                                metrics.membership_config.nodes().find_map(|(id, node)| {
                                    (*id == leader_id).then_some(node.clone())
                                })
                            })
                        });
                        let Some(leader_node) = leader_node else {
                            let msg = format!(
                                "{} data query can not resolve leader node for forwarding: local_node_id={}, leader_id={:?}",
                                self.service_name, local_node_id, forward.leader_id
                            );
                            warn!("{}", msg);
                            return Err(self
                                .service_error(
                                    StatusCode::SERVICE_UNAVAILABLE,
                                    KLogErrorCode::LeaderUnavailable,
                                    msg,
                                    &trace_id,
                                )
                                .with_leader_hint(forward.leader_node.clone()));
                        };

                        let target_hops = forward_hops + 1;
                        warn!(
                            "{} data query forwarding to leader: local_node_id={}, leader_id={}, leader_addr={}:{}, hops={} -> {}",
                            self.service_name,
                            local_node_id,
                            leader_node.id,
                            leader_node.addr,
                            leader_node.port,
                            forward_hops,
                            target_hops
                        );

                        return self
                            .data_client
                            .query_to_node(
                                &leader_node,
                                &query,
                                target_hops,
                                local_node_id,
                                &trace_id,
                            )
                            .await
                            .map_err(|forward_err| {
                                let msg = format!(
                                    "{} data query forward failed: local_node_id={}, leader_id={}, err={}",
                                    self.service_name, local_node_id, leader_node.id, forward_err
                                );
                                with_forward_error_context(forward_err, msg, leader_node.clone())
                            });
                    }

                    let msg = format!(
                        "{} data query strong_read failed to ensure linearizable read: {}",
                        self.service_name, err
                    );
                    error!("{}", msg);
                    return Err(self.service_error(
                        StatusCode::SERVICE_UNAVAILABLE,
                        KLogErrorCode::Unavailable,
                        msg,
                        &trace_id,
                    ));
                }
            }
        }

        if let (Some(start_id), Some(end_id)) = (query.start_id, query.end_id)
            && start_id > end_id
        {
            let msg = format!(
                "{} data query invalid range: start_id={} > end_id={}",
                self.service_name, start_id, end_id
            );
            error!("{}", msg);
            return Err(self.service_error(
                StatusCode::BAD_REQUEST,
                KLogErrorCode::InvalidArgument,
                msg,
                &trace_id,
            ));
        }

        let limit = query.limit.unwrap_or(DATA_QUERY_DEFAULT_LIMIT);
        if limit == 0 || limit > DATA_QUERY_MAX_LIMIT {
            let msg = format!(
                "{} data query invalid limit: limit={}, allowed=1..={}",
                self.service_name, limit, DATA_QUERY_MAX_LIMIT
            );
            error!("{}", msg);
            return Err(self.service_error(
                StatusCode::BAD_REQUEST,
                KLogErrorCode::InvalidArgument,
                msg,
                &trace_id,
            ));
        }

        let order = if query.desc.unwrap_or(false) {
            KLogQueryOrder::Desc
        } else {
            KLogQueryOrder::Asc
        };
        let source = query
            .source
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string());
        let attr_key = query
            .attr_key
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string());
        let attr_value = query
            .attr_value
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string());
        if attr_key.is_none() && attr_value.is_some() {
            let msg = format!(
                "{} data query invalid attrs filter: attr_value is set but attr_key is empty",
                self.service_name
            );
            error!("{}", msg);
            return Err(self.service_error(
                StatusCode::BAD_REQUEST,
                KLogErrorCode::InvalidArgument,
                msg,
                &trace_id,
            ));
        }
        info!(
            "{} data query request: trace_id={}, strong_read={}, start_id={:?}, end_id={:?}, limit={}, order={:?}, level={:?}, source={:?}, attr_key={:?}, attr_value={:?}, forward_hops={}, forwarded_by={}",
            self.service_name,
            trace_id,
            strong_read,
            query.start_id,
            query.end_id,
            limit,
            order,
            query.level,
            source.as_deref(),
            attr_key.as_deref(),
            attr_value.as_deref(),
            forward_hops,
            forwarded_by
        );

        let entries = self
            .state_store_manager
            .query_entries(KLogQuery {
                start_id: query.start_id,
                end_id: query.end_id,
                limit,
                order,
                level: query.level,
                source,
                attr_key,
                attr_value,
            })
            .await
            .map_err(|e| {
                let msg = format!("{} data query failed: {}", self.service_name, e);
                error!("{}", msg);
                self.service_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    KLogErrorCode::Internal,
                    msg,
                    &trace_id,
                )
            })?;

        info!(
            "{} data query response: items={}",
            self.service_name,
            entries.len()
        );
        Ok(KLogQueryResponse { items: entries })
    }

    pub async fn query_meta(
        &self,
        headers: &HeaderMap,
        query: KLogMetaQueryRequest,
    ) -> KServiceResult<KLogMetaQueryResponse> {
        let trace_id = self.resolve_trace_id(headers);
        let strong_read = query.strong_read.unwrap_or(false);
        let forward_hops = self
            .parse_forward_hops(headers, "meta query")
            .map_err(|msg| {
                error!("{}", msg);
                self.service_error(
                    StatusCode::BAD_REQUEST,
                    KLogErrorCode::InvalidArgument,
                    msg,
                    &trace_id,
                )
            })?;
        let forwarded_by = headers
            .get(KLOG_FORWARDED_BY_HEADER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("-");

        let key = query
            .key
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string());
        let prefix = query
            .prefix
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string());
        let cursor = query
            .cursor
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string());
        if key.is_some() && prefix.is_some() {
            let msg = format!(
                "{} meta query invalid params: key and prefix can not be set together",
                self.service_name
            );
            error!("{}", msg);
            return Err(self.service_error(
                StatusCode::BAD_REQUEST,
                KLogErrorCode::InvalidArgument,
                msg,
                &trace_id,
            ));
        }
        if key.is_some() && cursor.is_some() {
            let msg = format!(
                "{} meta query invalid params: cursor can not be used with key query",
                self.service_name
            );
            error!("{}", msg);
            return Err(self.service_error(
                StatusCode::BAD_REQUEST,
                KLogErrorCode::InvalidArgument,
                msg,
                &trace_id,
            ));
        }
        if let Some(key) = key.as_ref()
            && key.len() > META_KEY_MAX_BYTES
        {
            let msg = format!(
                "{} meta query invalid key length: key_bytes={}, max_bytes={}",
                self.service_name,
                key.len(),
                META_KEY_MAX_BYTES
            );
            error!("{}", msg);
            return Err(self.service_error(
                StatusCode::BAD_REQUEST,
                KLogErrorCode::InvalidArgument,
                msg,
                &trace_id,
            ));
        }
        if let Some(cursor) = cursor.as_ref() {
            if cursor.len() > META_KEY_MAX_BYTES {
                let msg = format!(
                    "{} meta query invalid cursor length: cursor_bytes={}, max_bytes={}",
                    self.service_name,
                    cursor.len(),
                    META_KEY_MAX_BYTES
                );
                error!("{}", msg);
                return Err(self.service_error(
                    StatusCode::BAD_REQUEST,
                    KLogErrorCode::InvalidArgument,
                    msg,
                    &trace_id,
                ));
            }
            if let Some(prefix) = prefix.as_ref()
                && !cursor.starts_with(prefix)
            {
                let msg = format!(
                    "{} meta query invalid cursor: cursor must start with prefix",
                    self.service_name
                );
                error!("{}", msg);
                return Err(self.service_error(
                    StatusCode::BAD_REQUEST,
                    KLogErrorCode::InvalidArgument,
                    msg,
                    &trace_id,
                ));
            }
        }
        if query.revision == Some(0) {
            let msg = format!(
                "{} meta query invalid revision: revision must be greater than 0",
                self.service_name
            );
            error!("{}", msg);
            return Err(self.service_error(
                StatusCode::BAD_REQUEST,
                KLogErrorCode::InvalidArgument,
                msg,
                &trace_id,
            ));
        }

        if strong_read {
            if forward_hops > META_RW_MAX_FORWARD_HOPS {
                let msg = format!(
                    "{} meta query rejected: too many forward hops, hops={}, max_hops={}, forwarded_by={}",
                    self.service_name, forward_hops, META_RW_MAX_FORWARD_HOPS, forwarded_by
                );
                error!("{}", msg);
                return Err(self.service_error(
                    StatusCode::BAD_GATEWAY,
                    KLogErrorCode::LeaderUnavailable,
                    msg,
                    &trace_id,
                ));
            }

            let metrics = self.raft.metrics().borrow().clone();
            let local_node_id = metrics.id;
            match self.raft.ensure_linearizable().await {
                Ok(read_log_id) => {
                    info!(
                        "{} meta query linearizable barrier passed: trace_id={}, read_log_id={:?}, local_node_id={}, forward_hops={}, forwarded_by={}",
                        self.service_name,
                        trace_id,
                        read_log_id,
                        local_node_id,
                        forward_hops,
                        forwarded_by
                    );
                }
                Err(err) => {
                    if let Some(forward) = err.forward_to_leader::<KNode>() {
                        if forward_hops >= META_RW_MAX_FORWARD_HOPS {
                            let msg = format!(
                                "{} meta query forward aborted due to hop limit: local_node_id={}, leader_id={:?}, leader_node={:?}, hops={}, max_hops={}",
                                self.service_name,
                                local_node_id,
                                forward.leader_id,
                                forward.leader_node,
                                forward_hops,
                                META_RW_MAX_FORWARD_HOPS
                            );
                            error!("{}", msg);
                            return Err(self.service_error(
                                StatusCode::BAD_GATEWAY,
                                KLogErrorCode::LeaderUnavailable,
                                msg,
                                &trace_id,
                            ));
                        }

                        let leader_node = forward.leader_node.clone().or_else(|| {
                            forward.leader_id.and_then(|leader_id| {
                                metrics.membership_config.nodes().find_map(|(id, node)| {
                                    (*id == leader_id).then_some(node.clone())
                                })
                            })
                        });
                        let Some(leader_node) = leader_node else {
                            let msg = format!(
                                "{} meta query can not resolve leader node for forwarding: local_node_id={}, leader_id={:?}",
                                self.service_name, local_node_id, forward.leader_id
                            );
                            warn!("{}", msg);
                            return Err(self
                                .service_error(
                                    StatusCode::SERVICE_UNAVAILABLE,
                                    KLogErrorCode::LeaderUnavailable,
                                    msg,
                                    &trace_id,
                                )
                                .with_leader_hint(forward.leader_node.clone()));
                        };

                        let target_hops = forward_hops + 1;
                        warn!(
                            "{} meta query forwarding to leader: local_node_id={}, leader_id={}, leader_addr={}:{}, hops={} -> {}",
                            self.service_name,
                            local_node_id,
                            leader_node.id,
                            leader_node.addr,
                            leader_node.port,
                            forward_hops,
                            target_hops
                        );
                        return self
                            .data_client
                            .query_meta_to_node(
                                &leader_node,
                                &KLogMetaQueryRequest {
                                    key: key.clone(),
                                    prefix: prefix.clone(),
                                    limit: query.limit,
                                    cursor: cursor.clone(),
                                    revision: query.revision,
                                    strong_read: query.strong_read,
                                },
                                target_hops,
                                local_node_id,
                                &trace_id,
                            )
                            .await
                            .map_err(|forward_err| {
                                let msg = format!(
                                    "{} meta query forward failed: local_node_id={}, leader_id={}, err={}",
                                    self.service_name, local_node_id, leader_node.id, forward_err
                                );
                                with_forward_error_context(forward_err, msg, leader_node.clone())
                            });
                    }
                    let msg = format!(
                        "{} meta query strong_read failed to ensure linearizable read: {}",
                        self.service_name, err
                    );
                    error!("{}", msg);
                    return Err(self.service_error(
                        StatusCode::SERVICE_UNAVAILABLE,
                        KLogErrorCode::Unavailable,
                        msg,
                        &trace_id,
                    ));
                }
            }
        }

        if let Some(revision) = query.revision {
            let compacted_revision = self
                .state_store_manager
                .meta_compacted_revision()
                .await
                .map_err(|e| {
                    let msg = format!(
                        "{} meta query read compacted revision failed: {}",
                        self.service_name, e
                    );
                    error!("{}", msg);
                    self.service_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        KLogErrorCode::Internal,
                        msg,
                        &trace_id,
                    )
                })?;
            if revision <= compacted_revision {
                let msg = format!(
                    "{} meta query rejected: revision={} has been compacted, compacted_revision={}",
                    self.service_name, revision, compacted_revision
                );
                warn!("{}", msg);
                return Err(self.service_error(
                    StatusCode::GONE,
                    KLogErrorCode::Compacted,
                    msg,
                    &trace_id,
                ));
            }
        }

        let limit = query.limit.unwrap_or(META_QUERY_DEFAULT_LIMIT);
        if limit == 0 || limit > META_QUERY_MAX_LIMIT {
            let msg = format!(
                "{} meta query invalid limit: limit={}, allowed=1..={}",
                self.service_name, limit, META_QUERY_MAX_LIMIT
            );
            error!("{}", msg);
            return Err(self.service_error(
                StatusCode::BAD_REQUEST,
                KLogErrorCode::InvalidArgument,
                msg,
                &trace_id,
            ));
        }

        info!(
            "{} meta query request: trace_id={}, strong_read={}, key={:?}, prefix={:?}, cursor={:?}, revision={:?}, limit={}, forward_hops={}, forwarded_by={}",
            self.service_name,
            trace_id,
            strong_read,
            key,
            prefix,
            cursor,
            query.revision,
            limit,
            forward_hops,
            forwarded_by
        );

        let mut has_more = false;
        let mut next_cursor = None;
        let items = if let Some(key) = key.as_deref() {
            let item = if let Some(revision) = query.revision {
                self.state_store_manager
                    .get_meta_entry_at_revision(key, revision)
                    .await
            } else {
                self.state_store_manager.get_meta_entry(key).await
            }
            .map_err(|e| {
                let msg = format!("{} meta query get failed: {}", self.service_name, e);
                error!("{}", msg);
                self.service_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    KLogErrorCode::Internal,
                    msg,
                    &trace_id,
                )
            })?;
            item.into_iter().collect::<Vec<_>>()
        } else {
            let mut items = if let Some(revision) = query.revision {
                self.state_store_manager
                    .list_meta_entries_at_revision(
                        prefix.as_deref(),
                        cursor.as_deref(),
                        limit.saturating_add(1),
                        revision,
                    )
                    .await
            } else {
                self.state_store_manager
                    .list_meta_entries(
                        prefix.as_deref(),
                        cursor.as_deref(),
                        limit.saturating_add(1),
                    )
                    .await
            }
            .map_err(|e| {
                let msg = format!("{} meta query list failed: {}", self.service_name, e);
                error!("{}", msg);
                self.service_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    KLogErrorCode::Internal,
                    msg,
                    &trace_id,
                )
            })?;
            if items.len() > limit {
                has_more = true;
                items.truncate(limit);
                next_cursor = items.last().map(|item| item.key.clone());
            }
            items
        };
        info!(
            "{} meta query response: items={}, has_more={}, next_cursor={:?}",
            self.service_name,
            items.len(),
            has_more,
            next_cursor
        );
        Ok(KLogMetaQueryResponse {
            items,
            next_cursor,
            has_more,
        })
    }

    pub async fn query_meta_changes(
        &self,
        headers: &HeaderMap,
        query: KLogMetaChangesRequest,
    ) -> KServiceResult<KLogMetaChangesResponse> {
        let trace_id = self.resolve_trace_id(headers);
        let strong_read = query.strong_read.unwrap_or(false);
        let forward_hops = self
            .parse_forward_hops(headers, "meta changes")
            .map_err(|msg| {
                error!("{}", msg);
                self.service_error(
                    StatusCode::BAD_REQUEST,
                    KLogErrorCode::InvalidArgument,
                    msg,
                    &trace_id,
                )
            })?;
        let forwarded_by = headers
            .get(KLOG_FORWARDED_BY_HEADER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("-");

        let start_revision = query.start_revision.unwrap_or(1);
        if start_revision == 0 {
            let msg = format!(
                "{} meta changes invalid start_revision: must be greater than 0",
                self.service_name
            );
            error!("{}", msg);
            return Err(self.service_error(
                StatusCode::BAD_REQUEST,
                KLogErrorCode::InvalidArgument,
                msg,
                &trace_id,
            ));
        }
        if let Some(end_revision) = query.end_revision
            && (end_revision == 0 || end_revision < start_revision)
        {
            let msg = format!(
                "{} meta changes invalid end_revision: start_revision={}, end_revision={}",
                self.service_name, start_revision, end_revision
            );
            error!("{}", msg);
            return Err(self.service_error(
                StatusCode::BAD_REQUEST,
                KLogErrorCode::InvalidArgument,
                msg,
                &trace_id,
            ));
        }

        let key = query
            .key
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string());
        let prefix = query
            .prefix
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string());
        if key.is_some() && prefix.is_some() {
            let msg = format!(
                "{} meta changes invalid params: key and prefix can not be set together",
                self.service_name
            );
            error!("{}", msg);
            return Err(self.service_error(
                StatusCode::BAD_REQUEST,
                KLogErrorCode::InvalidArgument,
                msg,
                &trace_id,
            ));
        }
        if let Some(key) = key.as_ref()
            && key.len() > META_KEY_MAX_BYTES
        {
            let msg = format!(
                "{} meta changes invalid key length: key_bytes={}, max_bytes={}",
                self.service_name,
                key.len(),
                META_KEY_MAX_BYTES
            );
            error!("{}", msg);
            return Err(self.service_error(
                StatusCode::BAD_REQUEST,
                KLogErrorCode::InvalidArgument,
                msg,
                &trace_id,
            ));
        }
        if let Some(prefix) = prefix.as_ref()
            && prefix.len() > META_KEY_MAX_BYTES
        {
            let msg = format!(
                "{} meta changes invalid prefix length: prefix_bytes={}, max_bytes={}",
                self.service_name,
                prefix.len(),
                META_KEY_MAX_BYTES
            );
            error!("{}", msg);
            return Err(self.service_error(
                StatusCode::BAD_REQUEST,
                KLogErrorCode::InvalidArgument,
                msg,
                &trace_id,
            ));
        }

        let cursor = match (query.cursor_revision, query.cursor_key.as_deref()) {
            (None, None) => None,
            (Some(revision), Some(key)) => {
                let key = key.trim();
                if revision == 0 || key.is_empty() || key.len() > META_KEY_MAX_BYTES {
                    let msg = format!(
                        "{} meta changes invalid cursor: revision={}, key_bytes={}, max_key_bytes={}",
                        self.service_name,
                        revision,
                        key.len(),
                        META_KEY_MAX_BYTES
                    );
                    error!("{}", msg);
                    return Err(self.service_error(
                        StatusCode::BAD_REQUEST,
                        KLogErrorCode::InvalidArgument,
                        msg,
                        &trace_id,
                    ));
                }
                Some(KLogMetaChangeCursor {
                    revision,
                    key: key.to_string(),
                })
            }
            _ => {
                let msg = format!(
                    "{} meta changes invalid cursor: cursor_revision and cursor_key must be set together",
                    self.service_name
                );
                error!("{}", msg);
                return Err(self.service_error(
                    StatusCode::BAD_REQUEST,
                    KLogErrorCode::InvalidArgument,
                    msg,
                    &trace_id,
                ));
            }
        };

        let limit = query.limit.unwrap_or(META_QUERY_DEFAULT_LIMIT);
        if limit == 0 || limit > META_QUERY_MAX_LIMIT {
            let msg = format!(
                "{} meta changes invalid limit: limit={}, allowed=1..={}",
                self.service_name, limit, META_QUERY_MAX_LIMIT
            );
            error!("{}", msg);
            return Err(self.service_error(
                StatusCode::BAD_REQUEST,
                KLogErrorCode::InvalidArgument,
                msg,
                &trace_id,
            ));
        }
        let wait_timeout_ms = query.wait_timeout_ms.unwrap_or(0);
        if wait_timeout_ms > META_CHANGES_MAX_WAIT_MS {
            let msg = format!(
                "{} meta changes invalid wait_timeout_ms: timeout_ms={}, max_timeout_ms={}",
                self.service_name, wait_timeout_ms, META_CHANGES_MAX_WAIT_MS
            );
            error!("{}", msg);
            return Err(self.service_error(
                StatusCode::BAD_REQUEST,
                KLogErrorCode::InvalidArgument,
                msg,
                &trace_id,
            ));
        }
        if wait_timeout_ms > 0 && query.end_revision.is_some() {
            let msg = format!(
                "{} meta changes invalid params: wait_timeout_ms can not be used with end_revision",
                self.service_name
            );
            error!("{}", msg);
            return Err(self.service_error(
                StatusCode::BAD_REQUEST,
                KLogErrorCode::InvalidArgument,
                msg,
                &trace_id,
            ));
        }
        let include_deleted = query.include_deleted.unwrap_or(true);

        if strong_read {
            if forward_hops > META_RW_MAX_FORWARD_HOPS {
                let msg = format!(
                    "{} meta changes rejected: too many forward hops, hops={}, max_hops={}, forwarded_by={}",
                    self.service_name, forward_hops, META_RW_MAX_FORWARD_HOPS, forwarded_by
                );
                error!("{}", msg);
                return Err(self.service_error(
                    StatusCode::BAD_GATEWAY,
                    KLogErrorCode::LeaderUnavailable,
                    msg,
                    &trace_id,
                ));
            }

            let metrics = self.raft.metrics().borrow().clone();
            let local_node_id = metrics.id;
            if let Err(err) = self.raft.ensure_linearizable().await {
                if let Some(forward) = err.forward_to_leader::<KNode>() {
                    if forward_hops >= META_RW_MAX_FORWARD_HOPS {
                        let msg = format!(
                            "{} meta changes forward aborted due to hop limit: local_node_id={}, leader_id={:?}, leader_node={:?}, hops={}, max_hops={}",
                            self.service_name,
                            local_node_id,
                            forward.leader_id,
                            forward.leader_node,
                            forward_hops,
                            META_RW_MAX_FORWARD_HOPS
                        );
                        error!("{}", msg);
                        return Err(self.service_error(
                            StatusCode::BAD_GATEWAY,
                            KLogErrorCode::LeaderUnavailable,
                            msg,
                            &trace_id,
                        ));
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
                            "{} meta changes can not resolve leader node for forwarding: local_node_id={}, leader_id={:?}",
                            self.service_name, local_node_id, forward.leader_id
                        );
                        warn!("{}", msg);
                        return Err(self
                            .service_error(
                                StatusCode::SERVICE_UNAVAILABLE,
                                KLogErrorCode::LeaderUnavailable,
                                msg,
                                &trace_id,
                            )
                            .with_leader_hint(forward.leader_node.clone()));
                    };

                    let target_hops = forward_hops + 1;
                    warn!(
                        "{} meta changes forwarding to leader: local_node_id={}, leader_id={}, leader_addr={}:{}, hops={} -> {}",
                        self.service_name,
                        local_node_id,
                        leader_node.id,
                        leader_node.addr,
                        leader_node.port,
                        forward_hops,
                        target_hops
                    );
                    return self
                        .data_client
                        .query_meta_changes_to_node(
                            &leader_node,
                            &query,
                            target_hops,
                            local_node_id,
                            &trace_id,
                        )
                        .await
                        .map_err(|forward_err| {
                            let msg = format!(
                                "{} meta changes forward failed: local_node_id={}, leader_id={}, err={}",
                                self.service_name, local_node_id, leader_node.id, forward_err
                            );
                            with_forward_error_context(forward_err, msg, leader_node.clone())
                        });
                }
                let msg = format!(
                    "{} meta changes strong_read failed to ensure linearizable read: {}",
                    self.service_name, err
                );
                error!("{}", msg);
                return Err(self.service_error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    KLogErrorCode::Unavailable,
                    msg,
                    &trace_id,
                ));
            }
        }

        info!(
            "{} meta changes request: trace_id={}, strong_read={}, start_revision={}, end_revision={:?}, key={:?}, prefix={:?}, cursor={:?}, limit={}, include_deleted={}, wait_timeout_ms={}, forward_hops={}, forwarded_by={}",
            self.service_name,
            trace_id,
            strong_read,
            start_revision,
            query.end_revision,
            key,
            prefix,
            cursor,
            limit,
            include_deleted,
            wait_timeout_ms,
            forward_hops,
            forwarded_by
        );

        let deadline =
            (wait_timeout_ms > 0).then(|| Instant::now() + Duration::from_millis(wait_timeout_ms));
        loop {
            let current_revision = self
                .state_store_manager
                .meta_revision()
                .await
                .map_err(|e| {
                    let msg = format!(
                        "{} meta changes read current revision failed: {}",
                        self.service_name, e
                    );
                    error!("{}", msg);
                    self.service_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        KLogErrorCode::Internal,
                        msg,
                        &trace_id,
                    )
                })?;
            let compacted_revision = self
                .state_store_manager
                .meta_compacted_revision()
                .await
                .map_err(|e| {
                    let msg = format!(
                        "{} meta changes read compacted revision failed: {}",
                        self.service_name, e
                    );
                    error!("{}", msg);
                    self.service_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        KLogErrorCode::Internal,
                        msg,
                        &trace_id,
                    )
                })?;
            let resume_revision = cursor
                .as_ref()
                .map(|cursor| cursor.revision)
                .unwrap_or(start_revision);
            if resume_revision <= compacted_revision {
                let msg = format!(
                    "{} meta changes rejected: resume_revision={} has been compacted, compacted_revision={}",
                    self.service_name, resume_revision, compacted_revision
                );
                warn!("{}", msg);
                return Err(self.service_error(
                    StatusCode::GONE,
                    KLogErrorCode::Compacted,
                    msg,
                    &trace_id,
                ));
            }

            let effective_end_revision = query
                .end_revision
                .map(|end_revision| end_revision.min(current_revision))
                .or(Some(current_revision))
                .filter(|end_revision| *end_revision >= start_revision);
            let mut items = if let Some(end_revision) = effective_end_revision {
                self.state_store_manager
                    .list_meta_changes(KLogMetaChangeQuery {
                        start_revision,
                        end_revision: Some(end_revision),
                        key: key.clone(),
                        prefix: prefix.clone(),
                        cursor: cursor.clone(),
                        limit: limit.saturating_add(1),
                        include_deleted,
                    })
                    .await
            } else {
                Ok(Vec::new())
            }
            .map_err(|e| {
                let msg = format!("{} meta changes list failed: {}", self.service_name, e);
                error!("{}", msg);
                self.service_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    KLogErrorCode::Internal,
                    msg,
                    &trace_id,
                )
            })?;

            let should_return = !items.is_empty()
                || wait_timeout_ms == 0
                || deadline.is_some_and(|deadline| Instant::now() >= deadline);
            if should_return {
                let mut has_more = false;
                let mut next_cursor = None;
                if items.len() > limit {
                    has_more = true;
                    items.truncate(limit);
                    next_cursor = items.last().map(|item| item.change_cursor());
                }
                let next_start_revision = if has_more {
                    start_revision
                } else if let Some(end_revision) = query.end_revision {
                    end_revision
                        .min(current_revision)
                        .saturating_add(1)
                        .max(start_revision)
                } else if current_revision >= start_revision {
                    current_revision.saturating_add(1)
                } else {
                    start_revision
                };

                info!(
                    "{} meta changes response: items={}, has_more={}, next_cursor={:?}, current_revision={}, next_start_revision={}",
                    self.service_name,
                    items.len(),
                    has_more,
                    next_cursor,
                    current_revision,
                    next_start_revision
                );
                return Ok(KLogMetaChangesResponse {
                    items,
                    next_cursor,
                    has_more,
                    current_revision,
                    next_start_revision,
                });
            }

            sleep(Duration::from_millis(META_CHANGES_POLL_INTERVAL_MS)).await;
        }
    }

    fn parse_forward_hops(&self, headers: &HeaderMap, op: &str) -> Result<u32, String> {
        let Some(raw) = headers.get(KLOG_FORWARD_HOPS_HEADER) else {
            return Ok(0);
        };
        let raw = raw.to_str().map_err(|e| {
            format!(
                "{} {} invalid {} header utf8: {}",
                self.service_name, op, KLOG_FORWARD_HOPS_HEADER, e
            )
        })?;
        raw.parse::<u32>().map_err(|e| {
            format!(
                "{} {} invalid {} header '{}': {}",
                self.service_name, op, KLOG_FORWARD_HOPS_HEADER, raw, e
            )
        })
    }

    fn resolve_trace_id(&self, headers: &HeaderMap) -> String {
        normalize_trace_id(
            headers
                .get(KLOG_TRACE_ID_HEADER)
                .and_then(|v| v.to_str().ok()),
        )
    }

    fn service_error(
        &self,
        status: StatusCode,
        code: KLogErrorCode,
        message: String,
        trace_id: &str,
    ) -> KLogServiceError {
        KLogServiceError::new(status.as_u16(), code, message, trace_id.to_string())
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
