use super::request::{
    KLOG_FORWARD_HOPS_HEADER, KLOG_FORWARDED_BY_HEADER, KLogAppendRequest, KLogAppendResponse,
    KLogDataRequestType, KLogQueryRequest, KLogQueryResponse, RaftRequest, RaftResponse,
};
use crate::{KNode, KNodeId, KTypeConfig};
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

pub struct KNetworkFactory {
    local: KNodeId,
}

impl KNetworkFactory {
    pub fn new(local: KNodeId) -> Self {
        Self { local }
    }
}

impl RaftNetworkFactory<KTypeConfig> for KNetworkFactory {
    type Network = KNetworkClient;

    async fn new_client(&mut self, target: KNodeId, node: &KNode) -> Self::Network {
        KNetworkClient::new(self.local.clone(), target, node.clone())
    }
}

pub struct KNetworkClient {
    client: reqwest::Client,
    local: KNodeId,
    target: KNodeId,
    node: KNode,
}

#[derive(Clone)]
pub struct KDataClient {
    client: reqwest::Client,
    timeout: Duration,
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
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub async fn append_to_node(
        &self,
        target: &KNode,
        req: &KLogAppendRequest,
        forward_hops: u32,
        forwarded_by: KNodeId,
    ) -> Result<KLogAppendResponse, String> {
        let path = KLogDataRequestType::Append.klog_path();
        let url = format!("http://{}:{}{}", target.addr, target.port, path);
        let response = self
            .client
            .post(&url)
            .timeout(self.timeout)
            .header(KLOG_FORWARD_HOPS_HEADER, forward_hops.to_string())
            .header(KLOG_FORWARDED_BY_HEADER, forwarded_by.to_string())
            .json(req)
            .send()
            .await
            .map_err(|e| {
                format!(
                    "forward data append send failed: target={}({}:{}), url={}, err={}",
                    target.id, target.addr, target.port, url, e
                )
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|e| format!("<failed to read body: {}>", e));
            return Err(format!(
                "forward data append failed: target={}({}:{}), url={}, status={}, body={}",
                target.id, target.addr, target.port, url, status, body
            ));
        }

        response.json::<KLogAppendResponse>().await.map_err(|e| {
            format!(
                "forward data append decode failed: target={}({}:{}), url={}, err={}",
                target.id, target.addr, target.port, url, e
            )
        })
    }

    pub async fn query_to_node(
        &self,
        target: &KNode,
        req: &KLogQueryRequest,
        forward_hops: u32,
        forwarded_by: KNodeId,
    ) -> Result<KLogQueryResponse, String> {
        let path = KLogDataRequestType::Query.klog_path();
        let url = format!("http://{}:{}{}", target.addr, target.port, path);
        let response = self
            .client
            .get(&url)
            .timeout(self.timeout)
            .header(KLOG_FORWARD_HOPS_HEADER, forward_hops.to_string())
            .header(KLOG_FORWARDED_BY_HEADER, forwarded_by.to_string())
            .query(req)
            .send()
            .await
            .map_err(|e| {
                format!(
                    "forward data query send failed: target={}({}:{}), url={}, err={}",
                    target.id, target.addr, target.port, url, e
                )
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|e| format!("<failed to read body: {}>", e));
            return Err(format!(
                "forward data query failed: target={}({}:{}), url={}, status={}, body={}",
                target.id, target.addr, target.port, url, status, body
            ));
        }

        response.json::<KLogQueryResponse>().await.map_err(|e| {
            format!(
                "forward data query decode failed: target={}({}:{}), url={}, err={}",
                target.id, target.addr, target.port, url, e
            )
        })
    }
}

impl KNetworkClient {
    pub fn new(local: KNodeId, target: KNodeId, node: KNode) -> Self {
        Self {
            local,
            target,
            node,
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
        let url = self.get_request_url(&req);

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
                    &std::io::Error::new(std::io::ErrorKind::Other, msg),
                )));
            } else if status.is_server_error() {
                return Err(RPCError::Network(NetworkError::new(&std::io::Error::new(
                    std::io::ErrorKind::Other,
                    msg,
                ))));
            }

            return Err(RPCError::Unreachable(Unreachable::new(
                &std::io::Error::new(std::io::ErrorKind::Other, msg),
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

    fn get_request_url(&self, req: &RaftRequest) -> String {
        format!(
            "http://{}:{}/klog/{}",
            self.node.addr,
            self.node.port,
            req.request_path()
        )
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
