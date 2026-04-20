use super::request::{
    KLOG_FORWARD_HOPS_HEADER, KLOG_FORWARDED_BY_HEADER, KLOG_TRACE_ID_HEADER, KLogAppendRequest,
    KLogAppendResponse, KLogDataRequestType, KLogMetaDeleteRequest, KLogMetaDeleteResponse,
    KLogMetaPutRequest, KLogMetaPutResponse, KLogMetaQueryRequest, KLogMetaQueryResponse,
    KLogQueryRequest, KLogQueryResponse, RaftRequest, RaftResponse,
};
use crate::error::{KLogErrorCode, KLogServiceError, parse_error_envelope_json};
use crate::{KClusterTransportMode, KNode, KNodeId, KTypeConfig};
use openraft::error::{
    InstallSnapshotError, NetworkError, RPCError, RaftError, RemoteError, Timeout, Unreachable,
};
use openraft::network::{RPCOption, RaftNetwork, RaftNetworkFactory};
use openraft::raft::{
    AppendEntriesRequest, AppendEntriesResponse, InstallSnapshotRequest, InstallSnapshotResponse,
    VoteRequest, VoteResponse,
};
use std::time::Duration;

const DEFAULT_DATA_RPC_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KClusterPlane {
    Raft,
    InterNode,
    Admin,
}

#[derive(Debug, Clone, Copy)]
struct KClusterEndpointBuilder<'a> {
    target: &'a KNode,
    mode: KClusterTransportMode,
}

impl<'a> KClusterEndpointBuilder<'a> {
    fn new(target: &'a KNode, mode: KClusterTransportMode) -> Self {
        Self { target, mode }
    }

    fn direct(target: &'a KNode) -> Self {
        Self::new(target, KClusterTransportMode::Direct)
    }

    fn build(self, plane: KClusterPlane, path: &str) -> Result<String, String> {
        match self.mode {
            KClusterTransportMode::Direct => Ok(format!(
                "http://{}:{}{}",
                self.target.addr,
                self.port_for_plane(plane),
                path
            )),
            KClusterTransportMode::GatewayProxy => Err(format!(
                "cluster transport mode gateway_proxy is not implemented yet: target_node_id={}, plane={}",
                self.target.id,
                plane.as_str()
            )),
            KClusterTransportMode::Hybrid => Err(format!(
                "cluster transport mode hybrid is not implemented yet: target_node_id={}, plane={}",
                self.target.id,
                plane.as_str()
            )),
        }
    }

    fn port_for_plane(self, plane: KClusterPlane) -> u16 {
        match plane {
            KClusterPlane::Raft => self.target.port,
            KClusterPlane::InterNode => {
                if self.target.inter_port > 0 {
                    self.target.inter_port
                } else {
                    self.target.port
                }
            }
            KClusterPlane::Admin => {
                if self.target.admin_port > 0 {
                    self.target.admin_port
                } else if self.target.inter_port > 0 {
                    self.target.inter_port
                } else {
                    self.target.port
                }
            }
        }
    }
}

impl KClusterPlane {
    fn as_str(self) -> &'static str {
        match self {
            KClusterPlane::Raft => "raft",
            KClusterPlane::InterNode => "inter",
            KClusterPlane::Admin => "admin",
        }
    }
}

pub struct KNetworkFactory {
    local: KNodeId,
    transport_mode: KClusterTransportMode,
}

impl KNetworkFactory {
    pub fn new(local: KNodeId, transport_mode: KClusterTransportMode) -> Self {
        Self {
            local,
            transport_mode,
        }
    }
}

impl RaftNetworkFactory<KTypeConfig> for KNetworkFactory {
    type Network = KNetworkClient;

    async fn new_client(&mut self, target: KNodeId, node: &KNode) -> Self::Network {
        KNetworkClient::new(self.local, target, node.clone(), self.transport_mode)
    }
}

pub struct KNetworkClient {
    client: reqwest::Client,
    local: KNodeId,
    target: KNodeId,
    node: KNode,
    transport_mode: KClusterTransportMode,
}

#[derive(Clone)]
pub struct KDataClient {
    client: reqwest::Client,
    timeout: Duration,
    transport_mode: KClusterTransportMode,
}

impl Default for KDataClient {
    fn default() -> Self {
        Self::new()
    }
}

impl KDataClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            timeout: DEFAULT_DATA_RPC_TIMEOUT,
            transport_mode: KClusterTransportMode::Direct,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_transport_mode(mut self, transport_mode: KClusterTransportMode) -> Self {
        self.transport_mode = transport_mode;
        self
    }

    pub async fn append_to_node(
        &self,
        target: &KNode,
        req: &KLogAppendRequest,
        forward_hops: u32,
        forwarded_by: KNodeId,
        trace_id: &str,
    ) -> Result<KLogAppendResponse, KLogServiceError> {
        let path = KLogDataRequestType::Append.klog_path();
        let endpoint_port = Self::inter_node_port(target);
        let url = self.build_data_url(target, &path, trace_id)?;
        let response = self
            .client
            .post(&url)
            .timeout(self.timeout)
            .header(KLOG_FORWARD_HOPS_HEADER, forward_hops.to_string())
            .header(KLOG_FORWARDED_BY_HEADER, forwarded_by.to_string())
            .header(KLOG_TRACE_ID_HEADER, trace_id)
            .json(req)
            .send()
            .await
            .map_err(|e| {
                let msg = format!(
                    "forward data append send failed: target={}({}:{}), url={}, err={}",
                    target.id, target.addr, endpoint_port, url, e
                );
                KLogServiceError::new(
                    reqwest::StatusCode::BAD_GATEWAY.as_u16(),
                    KLogErrorCode::Unavailable,
                    msg,
                    trace_id.to_string(),
                )
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|e| format!(r#"{{"message":"<failed to read body: {}>"}}"#, e));
            let fallback_msg = format!(
                "forward data append failed: target={}({}:{}), url={}, status={}, body={}",
                target.id, target.addr, endpoint_port, url, status, body
            );
            return Err(parse_error_envelope_json(&body)
                .map(|e| KLogServiceError {
                    http_status: status.as_u16(),
                    error: e,
                })
                .unwrap_or_else(|| {
                    KLogServiceError::from_http_status(
                        status.as_u16(),
                        fallback_msg,
                        trace_id.to_string(),
                    )
                }));
        }

        response.json::<KLogAppendResponse>().await.map_err(|e| {
            let msg = format!(
                "forward data append decode failed: target={}({}:{}), url={}, err={}",
                target.id, target.addr, endpoint_port, url, e
            );
            KLogServiceError::new(
                reqwest::StatusCode::BAD_GATEWAY.as_u16(),
                KLogErrorCode::Unavailable,
                msg,
                trace_id.to_string(),
            )
        })
    }

    pub async fn query_to_node(
        &self,
        target: &KNode,
        req: &KLogQueryRequest,
        forward_hops: u32,
        forwarded_by: KNodeId,
        trace_id: &str,
    ) -> Result<KLogQueryResponse, KLogServiceError> {
        let path = KLogDataRequestType::Query.klog_path();
        let endpoint_port = Self::inter_node_port(target);
        let url = self.build_data_url(target, &path, trace_id)?;
        let response = self
            .client
            .get(&url)
            .timeout(self.timeout)
            .header(KLOG_FORWARD_HOPS_HEADER, forward_hops.to_string())
            .header(KLOG_FORWARDED_BY_HEADER, forwarded_by.to_string())
            .header(KLOG_TRACE_ID_HEADER, trace_id)
            .query(req)
            .send()
            .await
            .map_err(|e| {
                let msg = format!(
                    "forward data query send failed: target={}({}:{}), url={}, err={}",
                    target.id, target.addr, endpoint_port, url, e
                );
                KLogServiceError::new(
                    reqwest::StatusCode::BAD_GATEWAY.as_u16(),
                    KLogErrorCode::Unavailable,
                    msg,
                    trace_id.to_string(),
                )
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|e| format!(r#"{{"message":"<failed to read body: {}>"}}"#, e));
            let fallback_msg = format!(
                "forward data query failed: target={}({}:{}), url={}, status={}, body={}",
                target.id, target.addr, endpoint_port, url, status, body
            );
            return Err(parse_error_envelope_json(&body)
                .map(|e| KLogServiceError {
                    http_status: status.as_u16(),
                    error: e,
                })
                .unwrap_or_else(|| {
                    KLogServiceError::from_http_status(
                        status.as_u16(),
                        fallback_msg,
                        trace_id.to_string(),
                    )
                }));
        }

        response.json::<KLogQueryResponse>().await.map_err(|e| {
            let msg = format!(
                "forward data query decode failed: target={}({}:{}), url={}, err={}",
                target.id, target.addr, endpoint_port, url, e
            );
            KLogServiceError::new(
                reqwest::StatusCode::BAD_GATEWAY.as_u16(),
                KLogErrorCode::Unavailable,
                msg,
                trace_id.to_string(),
            )
        })
    }

    pub async fn put_meta_to_node(
        &self,
        target: &KNode,
        req: &KLogMetaPutRequest,
        forward_hops: u32,
        forwarded_by: KNodeId,
        trace_id: &str,
    ) -> Result<KLogMetaPutResponse, KLogServiceError> {
        let path = KLogDataRequestType::MetaPut.klog_path();
        let endpoint_port = Self::inter_node_port(target);
        let url = self.build_data_url(target, &path, trace_id)?;
        let response = self
            .client
            .post(&url)
            .timeout(self.timeout)
            .header(KLOG_FORWARD_HOPS_HEADER, forward_hops.to_string())
            .header(KLOG_FORWARDED_BY_HEADER, forwarded_by.to_string())
            .header(KLOG_TRACE_ID_HEADER, trace_id)
            .json(req)
            .send()
            .await
            .map_err(|e| {
                let msg = format!(
                    "forward meta put send failed: target={}({}:{}), url={}, err={}",
                    target.id, target.addr, endpoint_port, url, e
                );
                KLogServiceError::new(
                    reqwest::StatusCode::BAD_GATEWAY.as_u16(),
                    KLogErrorCode::Unavailable,
                    msg,
                    trace_id.to_string(),
                )
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|e| format!(r#"{{"message":"<failed to read body: {}>"}}"#, e));
            let fallback_msg = format!(
                "forward meta put failed: target={}({}:{}), url={}, status={}, body={}",
                target.id, target.addr, endpoint_port, url, status, body
            );
            return Err(parse_error_envelope_json(&body)
                .map(|e| KLogServiceError {
                    http_status: status.as_u16(),
                    error: e,
                })
                .unwrap_or_else(|| {
                    KLogServiceError::from_http_status(
                        status.as_u16(),
                        fallback_msg,
                        trace_id.to_string(),
                    )
                }));
        }

        response.json::<KLogMetaPutResponse>().await.map_err(|e| {
            let msg = format!(
                "forward meta put decode failed: target={}({}:{}), url={}, err={}",
                target.id, target.addr, endpoint_port, url, e
            );
            KLogServiceError::new(
                reqwest::StatusCode::BAD_GATEWAY.as_u16(),
                KLogErrorCode::Unavailable,
                msg,
                trace_id.to_string(),
            )
        })
    }

    pub async fn delete_meta_to_node(
        &self,
        target: &KNode,
        req: &KLogMetaDeleteRequest,
        forward_hops: u32,
        forwarded_by: KNodeId,
        trace_id: &str,
    ) -> Result<KLogMetaDeleteResponse, KLogServiceError> {
        let path = KLogDataRequestType::MetaDelete.klog_path();
        let endpoint_port = Self::inter_node_port(target);
        let url = self.build_data_url(target, &path, trace_id)?;
        let response = self
            .client
            .post(&url)
            .timeout(self.timeout)
            .header(KLOG_FORWARD_HOPS_HEADER, forward_hops.to_string())
            .header(KLOG_FORWARDED_BY_HEADER, forwarded_by.to_string())
            .header(KLOG_TRACE_ID_HEADER, trace_id)
            .json(req)
            .send()
            .await
            .map_err(|e| {
                let msg = format!(
                    "forward meta delete send failed: target={}({}:{}), url={}, err={}",
                    target.id, target.addr, endpoint_port, url, e
                );
                KLogServiceError::new(
                    reqwest::StatusCode::BAD_GATEWAY.as_u16(),
                    KLogErrorCode::Unavailable,
                    msg,
                    trace_id.to_string(),
                )
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|e| format!(r#"{{"message":"<failed to read body: {}>"}}"#, e));
            let fallback_msg = format!(
                "forward meta delete failed: target={}({}:{}), url={}, status={}, body={}",
                target.id, target.addr, endpoint_port, url, status, body
            );
            return Err(parse_error_envelope_json(&body)
                .map(|e| KLogServiceError {
                    http_status: status.as_u16(),
                    error: e,
                })
                .unwrap_or_else(|| {
                    KLogServiceError::from_http_status(
                        status.as_u16(),
                        fallback_msg,
                        trace_id.to_string(),
                    )
                }));
        }

        response
            .json::<KLogMetaDeleteResponse>()
            .await
            .map_err(|e| {
                let msg = format!(
                    "forward meta delete decode failed: target={}({}:{}), url={}, err={}",
                    target.id, target.addr, endpoint_port, url, e
                );
                KLogServiceError::new(
                    reqwest::StatusCode::BAD_GATEWAY.as_u16(),
                    KLogErrorCode::Unavailable,
                    msg,
                    trace_id.to_string(),
                )
            })
    }

    pub async fn query_meta_to_node(
        &self,
        target: &KNode,
        req: &KLogMetaQueryRequest,
        forward_hops: u32,
        forwarded_by: KNodeId,
        trace_id: &str,
    ) -> Result<KLogMetaQueryResponse, KLogServiceError> {
        let path = KLogDataRequestType::MetaQuery.klog_path();
        let endpoint_port = Self::inter_node_port(target);
        let url = self.build_data_url(target, &path, trace_id)?;
        let response = self
            .client
            .get(&url)
            .timeout(self.timeout)
            .header(KLOG_FORWARD_HOPS_HEADER, forward_hops.to_string())
            .header(KLOG_FORWARDED_BY_HEADER, forwarded_by.to_string())
            .header(KLOG_TRACE_ID_HEADER, trace_id)
            .query(req)
            .send()
            .await
            .map_err(|e| {
                let msg = format!(
                    "forward meta query send failed: target={}({}:{}), url={}, err={}",
                    target.id, target.addr, endpoint_port, url, e
                );
                KLogServiceError::new(
                    reqwest::StatusCode::BAD_GATEWAY.as_u16(),
                    KLogErrorCode::Unavailable,
                    msg,
                    trace_id.to_string(),
                )
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|e| format!(r#"{{"message":"<failed to read body: {}>"}}"#, e));
            let fallback_msg = format!(
                "forward meta query failed: target={}({}:{}), url={}, status={}, body={}",
                target.id, target.addr, endpoint_port, url, status, body
            );
            return Err(parse_error_envelope_json(&body)
                .map(|e| KLogServiceError {
                    http_status: status.as_u16(),
                    error: e,
                })
                .unwrap_or_else(|| {
                    KLogServiceError::from_http_status(
                        status.as_u16(),
                        fallback_msg,
                        trace_id.to_string(),
                    )
                }));
        }

        response.json::<KLogMetaQueryResponse>().await.map_err(|e| {
            let msg = format!(
                "forward meta query decode failed: target={}({}:{}), url={}, err={}",
                target.id, target.addr, endpoint_port, url, e
            );
            KLogServiceError::new(
                reqwest::StatusCode::BAD_GATEWAY.as_u16(),
                KLogErrorCode::Unavailable,
                msg,
                trace_id.to_string(),
            )
        })
    }

    fn inter_node_port(target: &KNode) -> u16 {
        if target.inter_port > 0 {
            target.inter_port
        } else {
            target.port
        }
    }

    fn build_data_url(
        &self,
        target: &KNode,
        path: &str,
        trace_id: &str,
    ) -> Result<String, KLogServiceError> {
        KClusterEndpointBuilder::new(target, self.transport_mode)
            .build(KClusterPlane::InterNode, path)
            .map_err(|e| {
                let msg = format!(
                    "build inter-node endpoint failed: target={}({}), path={}, mode={}, err={}",
                    target.id, target.addr, path, self.transport_mode, e
                );
                error!("{}", msg);
                KLogServiceError::new(
                    reqwest::StatusCode::BAD_GATEWAY.as_u16(),
                    KLogErrorCode::Unavailable,
                    msg,
                    trace_id.to_string(),
                )
            })
    }
}

impl KNetworkClient {
    pub fn new(
        local: KNodeId,
        target: KNodeId,
        node: KNode,
        transport_mode: KClusterTransportMode,
    ) -> Self {
        Self {
            local,
            target,
            node,
            transport_mode,
            client: reqwest::Client::new(),
        }
    }

    pub async fn request<Err>(
        &self,
        req: RaftRequest,
        option: RPCOption,
    ) -> Result<RaftResponse, RPCError<KNodeId, KNode, Err>>
    where
        Err: std::error::Error + 'static + Clone,
    {
        let url = self.get_request_url(&req).map_err(|msg| {
            error!("{}", msg);
            RPCError::Unreachable(Unreachable::new(&std::io::Error::other(msg)))
        })?;

        let body = req.serialize().map_err(|e| {
            let msg = format!("Failed to serialize request for {}: {}", url, e);
            error!("{}", msg);
            RPCError::Unreachable(Unreachable::new(&e))
        })?;

        let resp = self
            .client
            .post(&url)
            .timeout(option.soft_ttl())
            .header("Content-Type", "application/octet-stream")
            .body(body)
            .send()
            .await
            .map_err(|e| {
                let msg = format!("Failed to send request to {}: {}", url, e);
                error!("{}", msg);
                if e.is_connect() {
                    RPCError::Network(NetworkError::new(&e))
                } else if e.is_timeout() {
                    let timeout = Timeout {
                        action: req.rpc_type(),
                        id: self.local,
                        target: self.target,
                        timeout: option.hard_ttl(),
                    };
                    RPCError::Timeout(timeout)
                } else {
                    RPCError::Unreachable(Unreachable::new(&e))
                }
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let msg = format!(
                "Request to {} failed with status: {} (rpc={:?})",
                url,
                status,
                req.rpc_type()
            );
            error!("{}", msg);
            if status == reqwest::StatusCode::PAYLOAD_TOO_LARGE {
                if let Some(payload_too_large) = req.payload_too_large() {
                    return Err(RPCError::PayloadTooLarge(payload_too_large));
                }

                return Err(RPCError::Network(NetworkError::new(
                    &std::io::Error::other(format!(
                        "Payload too large for unsupported rpc type: {}",
                        req.rpc_type()
                    )),
                )));
            } else if status.is_client_error() {
                return Err(RPCError::Unreachable(Unreachable::new(
                    &std::io::Error::other(msg),
                )));
            } else if status.is_server_error() {
                return Err(RPCError::Network(NetworkError::new(
                    &std::io::Error::other(msg),
                )));
            }

            return Err(RPCError::Unreachable(Unreachable::new(
                &std::io::Error::other(msg),
            )));
        }

        let res_body_bytes = resp.bytes().await.map_err(|e| {
            let msg = format!("Failed to read response body from {}: {}", url, e);
            error!("{}", msg);
            RPCError::Network(NetworkError::new(&e))
        })?;

        let raft_response = RaftResponse::deserialize(&res_body_bytes).map_err(|e| {
            let msg = format!("Failed to deserialize response from {}: {}", url, e);
            error!("{}", msg);

            RPCError::Unreachable(Unreachable::new(&e))
        })?;

        Ok(raft_response)
    }

    fn unexpected_response_type<Err>(
        &self,
        rpc: &'static str,
        response: &RaftResponse,
    ) -> RPCError<KNodeId, KNode, Err>
    where
        Err: std::error::Error + 'static + Clone,
    {
        let msg = format!(
            "Unexpected response type for {} from node {}: {:?}",
            rpc, self.target, response
        );
        error!("{}", msg);
        let io_err = std::io::Error::other(msg);
        RPCError::Network(NetworkError::new(&io_err))
    }

    fn get_request_url(&self, req: &RaftRequest) -> Result<String, String> {
        KClusterEndpointBuilder::new(&self.node, self.transport_mode)
            .build(KClusterPlane::Raft, &req.request_type().klog_path())
            .map_err(|e| {
                format!(
                    "build raft endpoint failed: target={}({}), rpc_type={:?}, mode={}, err={}",
                    self.target,
                    self.node.addr,
                    req.rpc_type(),
                    self.transport_mode,
                    e
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{KClusterEndpointBuilder, KClusterPlane};
    use crate::{KClusterTransportMode, KNode};

    fn test_node() -> KNode {
        KNode {
            id: 7,
            addr: "10.0.0.7".to_string(),
            port: 21001,
            inter_port: 21002,
            admin_port: 21003,
            rpc_port: 4070,
        }
    }

    #[test]
    fn test_cluster_endpoint_builder_direct_urls() {
        let node = test_node();
        let builder = KClusterEndpointBuilder::direct(&node);

        assert_eq!(
            builder.build(KClusterPlane::Raft, "/klog/vote").unwrap(),
            "http://10.0.0.7:21001/klog/vote"
        );
        assert_eq!(
            builder
                .build(KClusterPlane::InterNode, "/klog/data/query")
                .unwrap(),
            "http://10.0.0.7:21002/klog/data/query"
        );
        assert_eq!(
            builder
                .build(KClusterPlane::Admin, "/klog/admin/cluster-state")
                .unwrap(),
            "http://10.0.0.7:21003/klog/admin/cluster-state"
        );
    }

    #[test]
    fn test_cluster_endpoint_builder_direct_fallback_ports() {
        let node = KNode {
            id: 8,
            addr: "10.0.0.8".to_string(),
            port: 22001,
            inter_port: 0,
            admin_port: 0,
            rpc_port: 4070,
        };
        let builder = KClusterEndpointBuilder::direct(&node);

        assert_eq!(
            builder
                .build(KClusterPlane::InterNode, "/klog/data/query")
                .unwrap(),
            "http://10.0.0.8:22001/klog/data/query"
        );
        assert_eq!(
            builder
                .build(KClusterPlane::Admin, "/klog/admin/cluster-state")
                .unwrap(),
            "http://10.0.0.8:22001/klog/admin/cluster-state"
        );
    }

    #[test]
    fn test_cluster_endpoint_builder_rejects_unimplemented_modes() {
        let node = test_node();
        let gateway_proxy =
            KClusterEndpointBuilder::new(&node, KClusterTransportMode::GatewayProxy)
                .build(KClusterPlane::Raft, "/klog/vote")
                .expect_err("gateway proxy should not be implemented in phase 1");
        assert!(gateway_proxy.contains("gateway_proxy"));

        let hybrid = KClusterEndpointBuilder::new(&node, KClusterTransportMode::Hybrid)
            .build(KClusterPlane::Raft, "/klog/vote")
            .expect_err("hybrid should not be implemented in phase 1");
        assert!(hybrid.contains("hybrid"));
    }
}

// pub type RPCResult<T> = Result<T, openraft::network::RPCError<KNodeId>>;

impl RaftNetwork<KTypeConfig> for KNetworkClient {
    /// Send an AppendEntries RPC to the target.
    async fn append_entries(
        &mut self,
        rpc: AppendEntriesRequest<KTypeConfig>,
        option: RPCOption,
    ) -> Result<AppendEntriesResponse<KNodeId>, RPCError<KNodeId, KNode, RaftError<KNodeId>>> {
        match self
            .request::<RaftError<KNodeId>>(RaftRequest::AppendEntries(rpc), option)
            .await
        {
            Ok(RaftResponse::AppendEntries(resp)) => Ok(resp),
            Ok(RaftResponse::AppendEntriesError(err)) => Err(RPCError::RemoteError(
                RemoteError::new_with_node(self.target, self.node.clone(), err),
            )),
            Ok(other) => Err(self.unexpected_response_type("append_entries", &other)),
            Err(e) => Err(e),
        }
    }

    async fn install_snapshot(
        &mut self,
        rpc: InstallSnapshotRequest<KTypeConfig>,
        option: RPCOption,
    ) -> Result<
        InstallSnapshotResponse<KNodeId>,
        RPCError<KNodeId, KNode, RaftError<KNodeId, InstallSnapshotError>>,
    > {
        match self
            .request::<RaftError<KNodeId, InstallSnapshotError>>(
                RaftRequest::InstallSnapshot(rpc),
                option,
            )
            .await
        {
            Ok(RaftResponse::InstallSnapshot(resp)) => Ok(resp),
            Ok(RaftResponse::InstallSnapshotError(err)) => Err(RPCError::RemoteError(
                RemoteError::new_with_node(self.target, self.node.clone(), err),
            )),
            Ok(other) => Err(self.unexpected_response_type("install_snapshot", &other)),
            Err(e) => Err(e),
        }
    }

    /// Send a RequestVote RPC to the target.
    async fn vote(
        &mut self,
        rpc: VoteRequest<KNodeId>,
        option: RPCOption,
    ) -> Result<VoteResponse<KNodeId>, RPCError<KNodeId, KNode, RaftError<KNodeId>>> {
        match self
            .request::<RaftError<KNodeId>>(RaftRequest::Vote(rpc), option)
            .await
        {
            Ok(RaftResponse::Vote(resp)) => Ok(resp),
            Ok(RaftResponse::VoteError(err)) => Err(RPCError::RemoteError(
                RemoteError::new_with_node(self.target, self.node.clone(), err),
            )),
            Ok(other) => Err(self.unexpected_response_type("vote", &other)),
            Err(e) => Err(e),
        }
    }
}
