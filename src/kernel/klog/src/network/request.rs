use crate::{KNode, KNodeId, KResult, KTypeConfig, KLogError};
use bincode::error;
use openraft::error::PayloadTooLarge;
use openraft::network::RPCTypes;
use openraft::raft::{
    AppendEntriesRequest, AppendEntriesResponse, InstallSnapshotRequest, InstallSnapshotResponse,
    VoteRequest, VoteResponse,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RaftRequest {
    AppendEntries(AppendEntriesRequest<KTypeConfig>),
    InstallSnapshot(InstallSnapshotRequest<KTypeConfig>),
    Vote(VoteRequest<KNodeId>),
}

impl RaftRequest {
    pub fn request_path(&self) -> &str {
        match self {
            RaftRequest::AppendEntries(_) => "append-entries",
            RaftRequest::InstallSnapshot(_) => "install-snapshot",
            RaftRequest::Vote(_) => "vote",
        }
    }

    pub fn rpc_type(&self) -> RPCTypes {
        match self {
            RaftRequest::AppendEntries(_) => RPCTypes::AppendEntries,
            RaftRequest::InstallSnapshot(_) => RPCTypes::InstallSnapshot,
            RaftRequest::Vote(_) => RPCTypes::Vote,
        }
    }

    pub fn payload_too_large(&self) -> PayloadTooLarge {
        match self {
            RaftRequest::AppendEntries(req) => {
                PayloadTooLarge::new_entries_hint(req.entries.len() as u64 / 2)
            }
            RaftRequest::InstallSnapshot(_) => {
                error!("InstallSnapshotRequest is too large to send");
                PayloadTooLarge::new_entries_hint(0)
            }
            RaftRequest::Vote(_) => {
                error!("VoteRequest is too large to send");
                PayloadTooLarge::new_entries_hint(0)
            }
        }
    }

    // Use bincode for compactness and speed.
    pub fn serialize(&self) -> KResult<Vec<u8>> {
        bincode::serde::encode_to_vec(self, bincode::config::legacy()).map_err(|e| {
            let msg = format!("Failed to serialize RaftRequest: {}", e);
            error!("{}", msg);
            KLogError::InvalidFormat(msg)
        })
    }

    pub fn deserialize(data: &[u8]) -> KResult<Self> {
        let (this, _) = bincode::serde::decode_from_slice(data, bincode::config::legacy())
            .map_err(|e| {
                let msg = format!("Failed to deserialize RaftRequest: {}", e);
                error!("{}", msg);
                KLogError::InvalidFormat(msg)
            })?;

        Ok(this)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub enum RaftResponse {
    AppendEntries(AppendEntriesResponse<KNodeId>),
    InstallSnapshot(InstallSnapshotResponse<KNodeId>),
    Vote(VoteResponse<KNodeId>),
}

impl RaftResponse {
    // Use bincode for compactness and speed.
    pub fn serialize(&self) -> KResult<Vec<u8>> {
        bincode::serde::encode_to_vec(self, bincode::config::legacy()).map_err(|e| {
            let msg = format!("Failed to serialize RaftResponse: {}", e);
            error!("{}", msg);
            KLogError::InvalidFormat(msg)
        })
    }

    pub fn deserialize(data: &[u8]) -> KResult<Self> {
        let (this, _) = bincode::serde::decode_from_slice(data, bincode::config::legacy())
            .map_err(|e| {
                let msg = format!("Failed to deserialize RaftResponse: {}", e);
                error!("{}", msg);
                KLogError::InvalidFormat(msg)
            })?;

        Ok(this)
    }
}
