use super::request::{RaftRequest, RaftResponse};
use crate::{KNode, KNodeId, KTypeConfig};
use openraft::error::{
    InstallSnapshotError, NetworkError, PayloadTooLarge, RPCError, RaftError, RemoteError, Timeout,
    Unreachable,
};
use openraft::network::{RPCOption, RaftNetwork, RaftNetworkFactory};
use openraft::raft::{
    AppendEntriesRequest, AppendEntriesResponse, InstallSnapshotRequest, InstallSnapshotResponse,
    VoteRequest, VoteResponse,
};

pub struct KNetworkFactory {
    local: KNodeId,
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
            let msg = format!("Request to {} failed with status: {}", url, status);
            error!("{}", msg);
            if status == reqwest::StatusCode::PAYLOAD_TOO_LARGE {
                return Err(RPCError::PayloadTooLarge(req.payload_too_large()));
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
            Ok(other) => {
                unreachable!("Unexpected response type: {:?}", other);
            }
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
            Ok(other) => {
                unreachable!("Unexpected response type: {:?}", other);
            }
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
            Ok(other) => {
                unreachable!("Unexpected response type: {:?}", other);
            }
            Err(e) => Err(e),
        }
    }
}
