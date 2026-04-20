use super::request::{
    KLOG_FORWARD_HOPS_HEADER, KLOG_FORWARDED_BY_HEADER, KLOG_TRACE_ID_HEADER, KLogAppendRequest,
    KLogAppendResponse, KLogDataRequestType, KLogMetaDeleteRequest, KLogMetaDeleteResponse,
    KLogMetaPutRequest, KLogMetaPutResponse, KLogMetaQueryRequest, KLogMetaQueryResponse,
    KLogQueryRequest, KLogQueryResponse, RaftRequest, RaftResponse,
};
use crate::error::{KLogErrorCode, KLogServiceError, parse_error_envelope_json};
use crate::{KClusterTransportConfig, KClusterTransportMode, KNode, KNodeId, KTypeConfig};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KResolvedTransport {
    Direct,
    GatewayProxy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct KClusterEndpointCandidate {
    url: String,
    transport: KResolvedTransport,
}

#[derive(Debug, Clone)]
struct KClusterEndpointBuilder<'a> {
    target: &'a KNode,
    transport: &'a KClusterTransportConfig,
}

impl<'a> KClusterEndpointBuilder<'a> {
    fn new(target: &'a KNode, transport: &'a KClusterTransportConfig) -> Self {
        Self { target, transport }
    }

    fn build(
        &self,
        plane: KClusterPlane,
        path: &str,
    ) -> Result<Vec<KClusterEndpointCandidate>, String> {
        match self.transport.mode {
            KClusterTransportMode::Direct => Ok(vec![self.direct_candidate(plane, path)]),
            KClusterTransportMode::GatewayProxy => Ok(vec![self.gateway_candidate(plane, path)?]),
            KClusterTransportMode::Hybrid => Ok(vec![
                self.direct_candidate(plane, path),
                self.gateway_candidate(plane, path)?,
            ]),
        }
    }

    fn direct_candidate(&self, plane: KClusterPlane, path: &str) -> KClusterEndpointCandidate {
        KClusterEndpointCandidate {
            url: format!(
                "http://{}:{}{}",
                self.target.addr,
                self.port_for_plane(plane),
                path
            ),
            transport: KResolvedTransport::Direct,
        }
    }

    fn gateway_candidate(
        &self,
        plane: KClusterPlane,
        path: &str,
    ) -> Result<KClusterEndpointCandidate, String> {
        let gateway_addr = self.transport.gateway_addr.trim();
        if gateway_addr.is_empty() {
            return Err(format!(
                "cluster transport gateway_addr is empty: target_node_id={}, plane={}",
                self.target.id,
                plane.as_str()
            ));
        }

        let target_node_name = self
            .target
            .node_name
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| {
                format!(
                    "cluster transport requires target node_name for gateway/proxy mode: target_node_id={}, plane={}",
                    self.target.id,
                    plane.as_str()
                )
            })?;
        if target_node_name.contains('/') {
            return Err(format!(
                "cluster transport target node_name contains invalid '/': target_node_id={}, node_name={}, plane={}",
                self.target.id,
                target_node_name,
                plane.as_str()
            ));
        }

        let route_prefix = normalize_gateway_route_prefix(&self.transport.gateway_route_prefix)?;
        let route_base = if route_prefix == "/" {
            String::new()
        } else {
            route_prefix
        };
        let suffix = self.proxy_path_suffix(plane, path)?;
        Ok(KClusterEndpointCandidate {
            url: format!(
                "http://{}{}/{}/{}/{}",
                gateway_addr,
                route_base,
                target_node_name,
                plane.as_str(),
                suffix
            ),
            transport: KResolvedTransport::GatewayProxy,
        })
    }

    fn proxy_path_suffix(&self, plane: KClusterPlane, path: &str) -> Result<String, String> {
        let suffix = match plane {
            KClusterPlane::Raft => path.strip_prefix("/klog/"),
            KClusterPlane::InterNode => path.strip_prefix("/klog/data/"),
            KClusterPlane::Admin => path.strip_prefix("/klog/admin/"),
        };
        suffix.map(|v| v.to_string()).ok_or_else(|| {
            format!(
                "unexpected klog path for plane {}: {}",
                plane.as_str(),
                path
            )
        })
    }

    fn port_for_plane(&self, plane: KClusterPlane) -> u16 {
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

fn normalize_gateway_route_prefix(prefix: &str) -> Result<String, String> {
    let trimmed = prefix.trim();
    if trimmed.is_empty() {
        return Err("cluster transport gateway_route_prefix must not be empty".to_string());
    }

    let with_leading_slash = if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{}", trimmed)
    };

    let normalized = with_leading_slash.trim_end_matches('/').to_string();
    if normalized.is_empty() {
        return Ok("/".to_string());
    }
    Ok(normalized)
}

fn should_fallback_to_next_endpoint(err: &reqwest::Error) -> bool {
    err.is_connect() || err.is_timeout()
}

pub struct KNetworkFactory {
    local: KNodeId,
    transport: KClusterTransportConfig,
}

impl KNetworkFactory {
    pub fn new(local: KNodeId, transport: KClusterTransportConfig) -> Self {
        Self { local, transport }
    }
}

impl RaftNetworkFactory<KTypeConfig> for KNetworkFactory {
    type Network = KNetworkClient;

    async fn new_client(&mut self, target: KNodeId, node: &KNode) -> Self::Network {
        KNetworkClient::new(self.local, target, node.clone(), self.transport.clone())
    }
}

pub struct KNetworkClient {
    client: reqwest::Client,
    local: KNodeId,
    target: KNodeId,
    node: KNode,
    transport: KClusterTransportConfig,
}

#[derive(Clone)]
pub struct KDataClient {
    client: reqwest::Client,
    timeout: Duration,
    transport: KClusterTransportConfig,
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
            transport: KClusterTransportConfig::default(),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_transport_mode(mut self, transport_mode: KClusterTransportMode) -> Self {
        self.transport.mode = transport_mode;
        self
    }

    pub fn with_transport_config(mut self, transport: KClusterTransportConfig) -> Self {
        self.transport = transport;
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
        let endpoints = self.build_data_endpoints(target, &path, trace_id)?;
        let first_url = endpoints[0].url.clone();
        let response = self
            .send_with_fallback(&endpoints, |candidate_url| {
                self.client
                    .post(candidate_url)
                    .timeout(self.timeout)
                    .header(KLOG_FORWARD_HOPS_HEADER, forward_hops.to_string())
                    .header(KLOG_FORWARDED_BY_HEADER, forwarded_by.to_string())
                    .header(KLOG_TRACE_ID_HEADER, trace_id)
                    .json(req)
            })
            .await
            .map_err(|(final_url, e)| {
                let msg = format!(
                    "forward data append send failed: target={}({}:{}), url={}, err={}",
                    target.id, target.addr, endpoint_port, final_url, e
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
                target.id, target.addr, endpoint_port, first_url, status, body
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
                target.id, target.addr, endpoint_port, first_url, e
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
        let endpoints = self.build_data_endpoints(target, &path, trace_id)?;
        let first_url = endpoints[0].url.clone();
        let response = self
            .send_with_fallback(&endpoints, |candidate_url| {
                self.client
                    .get(candidate_url)
                    .timeout(self.timeout)
                    .header(KLOG_FORWARD_HOPS_HEADER, forward_hops.to_string())
                    .header(KLOG_FORWARDED_BY_HEADER, forwarded_by.to_string())
                    .header(KLOG_TRACE_ID_HEADER, trace_id)
                    .query(req)
            })
            .await
            .map_err(|(final_url, e)| {
                let msg = format!(
                    "forward data query send failed: target={}({}:{}), url={}, err={}",
                    target.id, target.addr, endpoint_port, final_url, e
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
                target.id, target.addr, endpoint_port, first_url, status, body
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
                target.id, target.addr, endpoint_port, first_url, e
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
        let endpoints = self.build_data_endpoints(target, &path, trace_id)?;
        let first_url = endpoints[0].url.clone();
        let response = self
            .send_with_fallback(&endpoints, |candidate_url| {
                self.client
                    .post(candidate_url)
                    .timeout(self.timeout)
                    .header(KLOG_FORWARD_HOPS_HEADER, forward_hops.to_string())
                    .header(KLOG_FORWARDED_BY_HEADER, forwarded_by.to_string())
                    .header(KLOG_TRACE_ID_HEADER, trace_id)
                    .json(req)
            })
            .await
            .map_err(|(final_url, e)| {
                let msg = format!(
                    "forward meta put send failed: target={}({}:{}), url={}, err={}",
                    target.id, target.addr, endpoint_port, final_url, e
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
                target.id, target.addr, endpoint_port, first_url, status, body
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
                target.id, target.addr, endpoint_port, first_url, e
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
        let endpoints = self.build_data_endpoints(target, &path, trace_id)?;
        let first_url = endpoints[0].url.clone();
        let response = self
            .send_with_fallback(&endpoints, |candidate_url| {
                self.client
                    .post(candidate_url)
                    .timeout(self.timeout)
                    .header(KLOG_FORWARD_HOPS_HEADER, forward_hops.to_string())
                    .header(KLOG_FORWARDED_BY_HEADER, forwarded_by.to_string())
                    .header(KLOG_TRACE_ID_HEADER, trace_id)
                    .json(req)
            })
            .await
            .map_err(|(final_url, e)| {
                let msg = format!(
                    "forward meta delete send failed: target={}({}:{}), url={}, err={}",
                    target.id, target.addr, endpoint_port, final_url, e
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
                target.id, target.addr, endpoint_port, first_url, status, body
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
                    target.id, target.addr, endpoint_port, first_url, e
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
        let endpoints = self.build_data_endpoints(target, &path, trace_id)?;
        let first_url = endpoints[0].url.clone();
        let response = self
            .send_with_fallback(&endpoints, |candidate_url| {
                self.client
                    .get(candidate_url)
                    .timeout(self.timeout)
                    .header(KLOG_FORWARD_HOPS_HEADER, forward_hops.to_string())
                    .header(KLOG_FORWARDED_BY_HEADER, forwarded_by.to_string())
                    .header(KLOG_TRACE_ID_HEADER, trace_id)
                    .query(req)
            })
            .await
            .map_err(|(final_url, e)| {
                let msg = format!(
                    "forward meta query send failed: target={}({}:{}), url={}, err={}",
                    target.id, target.addr, endpoint_port, final_url, e
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
                target.id, target.addr, endpoint_port, first_url, status, body
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
                target.id, target.addr, endpoint_port, first_url, e
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

    fn build_data_endpoints(
        &self,
        target: &KNode,
        path: &str,
        trace_id: &str,
    ) -> Result<Vec<KClusterEndpointCandidate>, KLogServiceError> {
        KClusterEndpointBuilder::new(target, &self.transport)
            .build(KClusterPlane::InterNode, path)
            .map_err(|e| {
                let msg = format!(
                    "build inter-node endpoint failed: target={}({}), path={}, mode={}, gateway_addr={}, route_prefix={}, err={}",
                    target.id,
                    target.addr,
                    path,
                    self.transport.mode,
                    self.transport.gateway_addr,
                    self.transport.gateway_route_prefix,
                    e
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

    async fn send_with_fallback<F>(
        &self,
        endpoints: &[KClusterEndpointCandidate],
        mut build_request: F,
    ) -> Result<reqwest::Response, (String, reqwest::Error)>
    where
        F: FnMut(&str) -> reqwest::RequestBuilder,
    {
        let mut last_error = None;
        for (idx, endpoint) in endpoints.iter().enumerate() {
            match build_request(&endpoint.url).send().await {
                Ok(response) => return Ok(response),
                Err(err) => {
                    let can_fallback =
                        idx + 1 < endpoints.len() && should_fallback_to_next_endpoint(&err);
                    if can_fallback {
                        warn!(
                            "cluster transport fallback: mode={:?}, url={}, err={}",
                            endpoint.transport, endpoint.url, err
                        );
                        last_error = Some((endpoint.url.clone(), err));
                        continue;
                    }
                    return Err((endpoint.url.clone(), err));
                }
            }
        }

        Err(last_error.expect("send_with_fallback requires at least one endpoint candidate"))
    }
}

impl KNetworkClient {
    pub fn new(
        local: KNodeId,
        target: KNodeId,
        node: KNode,
        transport: KClusterTransportConfig,
    ) -> Self {
        Self {
            local,
            target,
            node,
            transport,
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
        let endpoints = self.get_request_endpoints(&req).map_err(|msg| {
            error!("{}", msg);
            RPCError::Unreachable(Unreachable::new(&std::io::Error::other(msg)))
        })?;
        let first_url = endpoints[0].url.clone();

        let body = req.serialize().map_err(|e| {
            let msg = format!("Failed to serialize request for {}: {}", first_url, e);
            error!("{}", msg);
            RPCError::Unreachable(Unreachable::new(&e))
        })?;

        let (final_url, resp) = self
            .send_with_fallback::<Err>(&endpoints, &req, &option, &body)
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let msg = format!(
                "Request to {} failed with status: {} (rpc={:?})",
                final_url,
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
            let msg = format!("Failed to read response body from {}: {}", final_url, e);
            error!("{}", msg);
            RPCError::Network(NetworkError::new(&e))
        })?;

        let raft_response = RaftResponse::deserialize(&res_body_bytes).map_err(|e| {
            let msg = format!("Failed to deserialize response from {}: {}", final_url, e);
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

    fn get_request_endpoints(
        &self,
        req: &RaftRequest,
    ) -> Result<Vec<KClusterEndpointCandidate>, String> {
        KClusterEndpointBuilder::new(&self.node, &self.transport)
            .build(KClusterPlane::Raft, &req.request_type().klog_path())
            .map_err(|e| {
                format!(
                    "build raft endpoint failed: target={}({}), rpc_type={:?}, mode={}, gateway_addr={}, route_prefix={}, err={}",
                    self.target,
                    self.node.addr,
                    req.rpc_type(),
                    self.transport.mode,
                    self.transport.gateway_addr,
                    self.transport.gateway_route_prefix,
                    e
                )
            })
    }

    async fn send_with_fallback<Err>(
        &self,
        endpoints: &[KClusterEndpointCandidate],
        req: &RaftRequest,
        option: &RPCOption,
        body: &[u8],
    ) -> Result<(String, reqwest::Response), RPCError<KNodeId, KNode, Err>>
    where
        Err: std::error::Error + 'static + Clone,
    {
        for (idx, endpoint) in endpoints.iter().enumerate() {
            let response = self
                .client
                .post(&endpoint.url)
                .timeout(option.soft_ttl())
                .header("Content-Type", "application/octet-stream")
                .body(body.to_vec())
                .send()
                .await;

            match response {
                Ok(resp) => return Ok((endpoint.url.clone(), resp)),
                Err(err) => {
                    let can_fallback =
                        idx + 1 < endpoints.len() && should_fallback_to_next_endpoint(&err);
                    if can_fallback {
                        warn!(
                            "cluster transport fallback: local_node_id={}, target_node_id={}, rpc_type={:?}, mode={:?}, url={}, err={}",
                            self.local,
                            self.target,
                            req.rpc_type(),
                            endpoint.transport,
                            endpoint.url,
                            err
                        );
                        continue;
                    }
                    return Err(self.map_send_error(req, option, &endpoint.url, err));
                }
            }
        }

        let msg = format!(
            "cluster transport produced no request endpoints: target_node_id={}, rpc_type={:?}",
            self.target,
            req.rpc_type()
        );
        error!("{}", msg);
        Err(RPCError::Unreachable(Unreachable::new(
            &std::io::Error::other(msg),
        )))
    }

    fn map_send_error<Err>(
        &self,
        req: &RaftRequest,
        option: &RPCOption,
        url: &str,
        err: reqwest::Error,
    ) -> RPCError<KNodeId, KNode, Err>
    where
        Err: std::error::Error + 'static + Clone,
    {
        let msg = format!(
            "Failed to send request to {}: target_node_id={}, rpc_type={:?}, err={}",
            url,
            self.target,
            req.rpc_type(),
            err
        );
        error!("{}", msg);
        if err.is_connect() {
            RPCError::Network(NetworkError::new(&err))
        } else if err.is_timeout() {
            RPCError::Timeout(Timeout {
                action: req.rpc_type(),
                id: self.local,
                target: self.target,
                timeout: option.hard_ttl(),
            })
        } else {
            RPCError::Unreachable(Unreachable::new(&err))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        KClusterEndpointBuilder, KClusterEndpointCandidate, KClusterPlane, KResolvedTransport,
    };
    use crate::{KClusterTransportConfig, KClusterTransportMode, KNode};

    fn test_node() -> KNode {
        KNode {
            id: 7,
            addr: "10.0.0.7".to_string(),
            port: 21001,
            inter_port: 21002,
            admin_port: 21003,
            rpc_port: 4070,
            node_name: Some("node-7".to_string()),
        }
    }

    #[test]
    fn test_cluster_endpoint_builder_direct_urls() {
        let node = test_node();
        let transport = KClusterTransportConfig::default();
        let builder = KClusterEndpointBuilder::new(&node, &transport);

        assert_eq!(
            builder.build(KClusterPlane::Raft, "/klog/vote").unwrap(),
            vec![KClusterEndpointCandidate {
                url: "http://10.0.0.7:21001/klog/vote".to_string(),
                transport: KResolvedTransport::Direct,
            }]
        );
        assert_eq!(
            builder
                .build(KClusterPlane::InterNode, "/klog/data/query")
                .unwrap(),
            vec![KClusterEndpointCandidate {
                url: "http://10.0.0.7:21002/klog/data/query".to_string(),
                transport: KResolvedTransport::Direct,
            }]
        );
        assert_eq!(
            builder
                .build(KClusterPlane::Admin, "/klog/admin/cluster-state")
                .unwrap(),
            vec![KClusterEndpointCandidate {
                url: "http://10.0.0.7:21003/klog/admin/cluster-state".to_string(),
                transport: KResolvedTransport::Direct,
            }]
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
            node_name: None,
        };
        let transport = KClusterTransportConfig::default();
        let builder = KClusterEndpointBuilder::new(&node, &transport);

        assert_eq!(
            builder
                .build(KClusterPlane::InterNode, "/klog/data/query")
                .unwrap(),
            vec![KClusterEndpointCandidate {
                url: "http://10.0.0.8:22001/klog/data/query".to_string(),
                transport: KResolvedTransport::Direct,
            }]
        );
        assert_eq!(
            builder
                .build(KClusterPlane::Admin, "/klog/admin/cluster-state")
                .unwrap(),
            vec![KClusterEndpointCandidate {
                url: "http://10.0.0.8:22001/klog/admin/cluster-state".to_string(),
                transport: KResolvedTransport::Direct,
            }]
        );
    }

    #[test]
    fn test_cluster_endpoint_builder_gateway_proxy_url() {
        let node = test_node();
        let transport = KClusterTransportConfig {
            mode: KClusterTransportMode::GatewayProxy,
            gateway_addr: "127.0.0.1:3180".to_string(),
            gateway_route_prefix: "/.cluster/klog".to_string(),
        };
        let endpoints = KClusterEndpointBuilder::new(&node, &transport)
            .build(KClusterPlane::Raft, "/klog/vote")
            .expect("gateway proxy endpoint should be built");
        assert_eq!(
            endpoints,
            vec![KClusterEndpointCandidate {
                url: "http://127.0.0.1:3180/.cluster/klog/node-7/raft/vote".to_string(),
                transport: KResolvedTransport::GatewayProxy,
            }]
        );
    }

    #[test]
    fn test_cluster_endpoint_builder_hybrid_endpoints() {
        let node = test_node();
        let transport = KClusterTransportConfig {
            mode: KClusterTransportMode::Hybrid,
            gateway_addr: "127.0.0.1:3180".to_string(),
            gateway_route_prefix: "/.cluster/klog".to_string(),
        };
        let endpoints = KClusterEndpointBuilder::new(&node, &transport)
            .build(KClusterPlane::Raft, "/klog/vote")
            .expect("hybrid endpoints should be built");
        assert_eq!(
            endpoints,
            vec![
                KClusterEndpointCandidate {
                    url: "http://10.0.0.7:21001/klog/vote".to_string(),
                    transport: KResolvedTransport::Direct,
                },
                KClusterEndpointCandidate {
                    url: "http://127.0.0.1:3180/.cluster/klog/node-7/raft/vote".to_string(),
                    transport: KResolvedTransport::GatewayProxy,
                },
            ]
        );
    }

    #[test]
    fn test_cluster_endpoint_builder_gateway_proxy_requires_node_name() {
        let mut node = test_node();
        node.node_name = None;
        let transport = KClusterTransportConfig {
            mode: KClusterTransportMode::GatewayProxy,
            gateway_addr: "127.0.0.1:3180".to_string(),
            gateway_route_prefix: "/.cluster/klog".to_string(),
        };
        let err = KClusterEndpointBuilder::new(&node, &transport)
            .build(KClusterPlane::Raft, "/klog/vote")
            .expect_err("gateway proxy should require node_name");
        assert!(err.contains("node_name"));
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
