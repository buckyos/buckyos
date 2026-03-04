use crate::error::{KLogErrorCode, KLogServiceError, normalize_trace_id};
use crate::network::{
    KDataClient, KLOG_FORWARD_HOPS_HEADER, KLOG_FORWARDED_BY_HEADER, KLOG_TRACE_ID_HEADER,
    KLogAppendRequest, KLogAppendResponse, KLogMetaDeleteRequest, KLogMetaDeleteResponse,
    KLogMetaPutRequest, KLogMetaPutResponse, KLogMetaQueryRequest, KLogMetaQueryResponse,
    KLogQueryRequest, KLogQueryResponse,
};
use crate::state_store::{KLogQuery, KLogQueryOrder, KLogStateStoreManagerRef};
use crate::{KLogEntry, KLogLevel, KLogMetaEntry, KLogRequest, KLogResponse, KNode, KRaftRef};
use axum::http::{HeaderMap, StatusCode};
use std::time::{SystemTime, UNIX_EPOCH};

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
pub const META_RW_MAX_FORWARD_HOPS: u32 = 2;
pub type KServiceResult<T> = Result<T, KLogServiceError>;

#[derive(Clone)]
pub struct KLogWriteService {
    service_name: &'static str,
    raft: KRaftRef,
    state_store_manager: KLogStateStoreManagerRef,
    data_client: KDataClient,
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
        let level = req.level.unwrap_or(KLogLevel::Info);
        let source = req
            .source
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string());
        let attrs = req.attrs.unwrap_or_default();
        let req = KLogAppendRequest {
            message: req.message,
            timestamp: req.timestamp.or_else(|| Some(now_millis())),
            node_id: req.node_id.or(Some(local_node_id)),
            level: Some(level),
            source: source.clone(),
            attrs: Some(attrs.clone()),
            request_id: request_id.clone(),
        };

        let item = self.state_store_manager.prepare_append_entry(KLogEntry {
            id: 0,
            timestamp: req.timestamp.unwrap_or(0),
            node_id: req.node_id.unwrap_or(local_node_id),
            request_id,
            level,
            source,
            attrs,
            message: req.message.clone(),
        });
        let requested_id = item.id;

        info!(
            "{} data append request: trace_id={}, id={}, request_id={:?}, timestamp={}, node_id={}, level={:?}, source={:?}, attrs_len={}, msg_len={}, local_node_id={}, current_leader={:?}, forward_hops={}, forwarded_by={}",
            self.service_name,
            trace_id,
            item.id,
            item.request_id.as_deref(),
            item.timestamp,
            item.node_id,
            item.level,
            item.source.as_deref(),
            item.attrs.len(),
            item.message.len(),
            local_node_id,
            metrics.current_leader,
            forward_hops,
            forwarded_by
        );

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
                        Err(mut forward_err) => {
                            let msg = format!(
                                "{} data append forward failed: local_node_id={}, requested_id={}, leader_id={}, err={}",
                                self.service_name,
                                local_node_id,
                                requested_id,
                                leader_node.id,
                                forward_err
                            );
                            error!("{}", msg);
                            forward_err.http_status = StatusCode::BAD_GATEWAY.as_u16();
                            forward_err.error.message = msg;
                            if forward_err.error.leader_hint.is_none() {
                                forward_err.error.leader_hint = Some(leader_node);
                            }
                            Err(forward_err)
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
        let expected_revision = req.expected_revision;
        let item = KLogMetaEntry {
            key: key.clone(),
            value: req.value.clone(),
            // Meta write audit fields are owned by the raft service, not client input.
            updated_at: now_millis(),
            updated_by: local_node_id,
            revision: 0,
        };
        info!(
            "{} meta put request: trace_id={}, key={}, value_len={}, updated_at={}, updated_by={}, expected_revision={:?}, local_node_id={}, current_leader={:?}, forward_hops={}, forwarded_by={}",
            self.service_name,
            trace_id,
            item.key,
            item.value.len(),
            item.updated_at,
            item.updated_by,
            expected_revision,
            local_node_id,
            metrics.current_leader,
            forward_hops,
            forwarded_by
        );

        match self
            .raft
            .client_write(KLogRequest::PutMeta {
                item,
                expected_revision,
            })
            .await
        {
            Ok(resp) => match resp.data {
                KLogResponse::MetaPutOk { key, revision } => {
                    info!(
                        "{} meta put committed: key={}, revision={}",
                        self.service_name, key, revision
                    );
                    Ok(KLogMetaPutResponse { key, revision })
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
                        Err(mut forward_err) => {
                            let msg = format!(
                                "{} meta put forward failed: local_node_id={}, key={}, leader_id={}, err={}",
                                self.service_name, local_node_id, key, leader_node.id, forward_err
                            );
                            error!("{}", msg);
                            forward_err.http_status = StatusCode::BAD_GATEWAY.as_u16();
                            forward_err.error.message = msg;
                            if forward_err.error.leader_hint.is_none() {
                                forward_err.error.leader_hint = Some(leader_node);
                            }
                            Err(forward_err)
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
                } => {
                    info!(
                        "{} meta delete committed: key={}, existed={}, prev_meta_revision={:?}",
                        self.service_name,
                        key,
                        existed,
                        prev_meta.as_ref().map(|v| v.revision)
                    );
                    Ok(KLogMetaDeleteResponse {
                        key,
                        existed,
                        prev_meta,
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
                        Err(mut forward_err) => {
                            let msg = format!(
                                "{} meta delete forward failed: local_node_id={}, key={}, leader_id={}, err={}",
                                self.service_name, local_node_id, key, leader_node.id, forward_err
                            );
                            error!("{}", msg);
                            forward_err.http_status = StatusCode::BAD_GATEWAY.as_u16();
                            forward_err.error.message = msg;
                            if forward_err.error.leader_hint.is_none() {
                                forward_err.error.leader_hint = Some(leader_node);
                            }
                            Err(forward_err)
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
                                error!("{}", msg);
                                self.service_error(
                                    StatusCode::BAD_GATEWAY,
                                    KLogErrorCode::LeaderUnavailable,
                                    msg,
                                    &trace_id,
                                )
                                .with_leader_hint(Some(leader_node.clone()))
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
                                error!("{}", msg);
                                self.service_error(
                                    StatusCode::BAD_GATEWAY,
                                    KLogErrorCode::LeaderUnavailable,
                                    msg,
                                    &trace_id,
                                )
                                .with_leader_hint(Some(leader_node.clone()))
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
            "{} meta query request: trace_id={}, strong_read={}, key={:?}, prefix={:?}, limit={}, forward_hops={}, forwarded_by={}",
            self.service_name,
            trace_id,
            strong_read,
            key,
            prefix,
            limit,
            forward_hops,
            forwarded_by
        );

        let items = if let Some(key) = key.as_deref() {
            let item = self
                .state_store_manager
                .get_meta_entry(key)
                .await
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
            self.state_store_manager
                .list_meta_entries(prefix.as_deref(), limit)
                .await
                .map_err(|e| {
                    let msg = format!("{} meta query list failed: {}", self.service_name, e);
                    error!("{}", msg);
                    self.service_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        KLogErrorCode::Internal,
                        msg,
                        &trace_id,
                    )
                })?
        };
        info!(
            "{} meta query response: items={}",
            self.service_name,
            items.len()
        );
        Ok(KLogMetaQueryResponse { items })
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
