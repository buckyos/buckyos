use crate::network::{
    KDataClient, KLOG_FORWARD_HOPS_HEADER, KLOG_FORWARDED_BY_HEADER, KLogAppendRequest,
    KLogAppendResponse, KLogQueryRequest, KLogQueryResponse,
};
use crate::state_store::{KLogQuery, KLogQueryOrder, KLogStateStoreManagerRef};
use crate::{KLogEntry, KLogRequest, KLogResponse, KNode, KRaftRef};
use axum::http::{HeaderMap, StatusCode};
use std::time::{SystemTime, UNIX_EPOCH};

pub const DATA_QUERY_DEFAULT_LIMIT: usize = 200;
pub const DATA_QUERY_MAX_LIMIT: usize = 2_000;
pub const DATA_QUERY_MAX_FORWARD_HOPS: u32 = 2;
pub const DATA_APPEND_MAX_MESSAGE_BYTES: usize = 64 * 1024;
pub const DATA_APPEND_MAX_REQUEST_ID_BYTES: usize = 128;
pub const DATA_APPEND_MAX_FORWARD_HOPS: u32 = 2;

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
    ) -> Result<KLogAppendResponse, (StatusCode, String)> {
        if req.message.trim().is_empty() {
            let msg = format!("{} data append rejected: empty message", self.service_name);
            error!("{}", msg);
            return Err((StatusCode::BAD_REQUEST, msg));
        }

        if req.message.len() > DATA_APPEND_MAX_MESSAGE_BYTES {
            let msg = format!(
                "{} data append rejected: message too large, bytes={}, max_bytes={}",
                self.service_name,
                req.message.len(),
                DATA_APPEND_MAX_MESSAGE_BYTES
            );
            error!("{}", msg);
            return Err((StatusCode::PAYLOAD_TOO_LARGE, msg));
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
            return Err((StatusCode::BAD_REQUEST, msg));
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

        let forward_hops = self.parse_forward_hops(headers).map_err(|msg| {
            error!("{}", msg);
            (StatusCode::BAD_REQUEST, msg)
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
            return Err((StatusCode::BAD_GATEWAY, msg));
        }

        let metrics = self.raft.metrics().borrow().clone();
        let local_node_id = metrics.id;
        let req = KLogAppendRequest {
            message: req.message,
            timestamp: req.timestamp.or_else(|| Some(now_millis())),
            node_id: req.node_id.or(Some(local_node_id)),
            request_id: request_id.clone(),
        };

        let item = self.state_store_manager.prepare_append_entry(KLogEntry {
            id: 0,
            timestamp: req.timestamp.unwrap_or(0),
            node_id: req.node_id.unwrap_or(local_node_id),
            request_id,
            message: req.message.clone(),
        });
        let requested_id = item.id;

        info!(
            "{} data append request: id={}, request_id={:?}, timestamp={}, node_id={}, msg_len={}, local_node_id={}, current_leader={:?}, forward_hops={}, forwarded_by={}",
            self.service_name,
            item.id,
            item.request_id.as_deref(),
            item.timestamp,
            item.node_id,
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
                    Err((StatusCode::INTERNAL_SERVER_ERROR, msg))
                }
                other => {
                    let msg = format!(
                        "{} data append unexpected response: requested_id={}, response={:?}",
                        self.service_name, requested_id, other
                    );
                    error!("{}", msg);
                    Err((StatusCode::INTERNAL_SERVER_ERROR, msg))
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
                            "{} data append can not resolve leader node for forwarding: local_node_id={}, requested_id={}, leader_id={:?}",
                            self.service_name, local_node_id, requested_id, forward.leader_id
                        );
                        warn!("{}", msg);
                        return Err((StatusCode::SERVICE_UNAVAILABLE, msg));
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
                        .append_to_node(&leader_node, &req, target_hops, local_node_id)
                        .await
                    {
                        Ok(resp) => {
                            info!(
                                "{} data append forwarded and committed: local_node_id={}, requested_id={}, committed_id={}, leader_id={}, hops={}",
                                self.service_name,
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
                            error!("{}", msg);
                            Err((StatusCode::BAD_GATEWAY, msg))
                        }
                    }
                } else {
                    let msg = format!(
                        "{} data append raft client_write failed: requested_id={}, err={}",
                        self.service_name, requested_id, err
                    );
                    error!("{}", msg);
                    Err((StatusCode::INTERNAL_SERVER_ERROR, msg))
                }
            }
        }
    }

    fn parse_forward_hops(&self, headers: &HeaderMap) -> Result<u32, String> {
        let Some(raw) = headers.get(KLOG_FORWARD_HOPS_HEADER) else {
            return Ok(0);
        };
        let raw = raw.to_str().map_err(|e| {
            format!(
                "{} data append invalid {} header utf8: {}",
                self.service_name, KLOG_FORWARD_HOPS_HEADER, e
            )
        })?;
        raw.parse::<u32>().map_err(|e| {
            format!(
                "{} data append invalid {} header '{}': {}",
                self.service_name, KLOG_FORWARD_HOPS_HEADER, raw, e
            )
        })
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
    ) -> Result<KLogQueryResponse, (StatusCode, String)> {
        let strong_read = query.strong_read.unwrap_or(false);
        let forward_hops = self.parse_forward_hops(headers).map_err(|msg| {
            error!("{}", msg);
            (StatusCode::BAD_REQUEST, msg)
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
                return Err((StatusCode::BAD_GATEWAY, msg));
            }

            let metrics = self.raft.metrics().borrow().clone();
            let local_node_id = metrics.id;
            match self.raft.ensure_linearizable().await {
                Ok(read_log_id) => {
                    info!(
                        "{} data query linearizable barrier passed: read_log_id={:?}, local_node_id={}, forward_hops={}, forwarded_by={}",
                        self.service_name, read_log_id, local_node_id, forward_hops, forwarded_by
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
                            return Err((StatusCode::BAD_GATEWAY, msg));
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
                            return Err((StatusCode::SERVICE_UNAVAILABLE, msg));
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
                            .query_to_node(&leader_node, &query, target_hops, local_node_id)
                            .await
                            .map_err(|forward_err| {
                                let msg = format!(
                                    "{} data query forward failed: local_node_id={}, leader_id={}, err={}",
                                    self.service_name, local_node_id, leader_node.id, forward_err
                                );
                                error!("{}", msg);
                                (StatusCode::BAD_GATEWAY, msg)
                            });
                    }

                    let msg = format!(
                        "{} data query strong_read failed to ensure linearizable read: {}",
                        self.service_name, err
                    );
                    error!("{}", msg);
                    return Err((StatusCode::SERVICE_UNAVAILABLE, msg));
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
            return Err((StatusCode::BAD_REQUEST, msg));
        }

        let limit = query.limit.unwrap_or(DATA_QUERY_DEFAULT_LIMIT);
        if limit == 0 || limit > DATA_QUERY_MAX_LIMIT {
            let msg = format!(
                "{} data query invalid limit: limit={}, allowed=1..={}",
                self.service_name, limit, DATA_QUERY_MAX_LIMIT
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
            "{} data query request: strong_read={}, start_id={:?}, end_id={:?}, limit={}, order={:?}, forward_hops={}, forwarded_by={}",
            self.service_name,
            strong_read,
            query.start_id,
            query.end_id,
            limit,
            order,
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
            })
            .await
            .map_err(|e| {
                let msg = format!("{} data query failed: {}", self.service_name, e);
                error!("{}", msg);
                (StatusCode::INTERNAL_SERVER_ERROR, msg)
            })?;

        info!(
            "{} data query response: items={}",
            self.service_name,
            entries.len()
        );
        Ok(KLogQueryResponse { items: entries })
    }

    fn parse_forward_hops(&self, headers: &HeaderMap) -> Result<u32, String> {
        let Some(raw) = headers.get(KLOG_FORWARD_HOPS_HEADER) else {
            return Ok(0);
        };
        let raw = raw.to_str().map_err(|e| {
            format!(
                "{} data query invalid {} header utf8: {}",
                self.service_name, KLOG_FORWARD_HOPS_HEADER, e
            )
        })?;
        raw.parse::<u32>().map_err(|e| {
            format!(
                "{} data query invalid {} header '{}': {}",
                self.service_name, KLOG_FORWARD_HOPS_HEADER, raw, e
            )
        })
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
