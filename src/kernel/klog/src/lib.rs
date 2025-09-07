use openraft::{declare_raft_types, LogId};
use serde::{Deserialize, Serialize};

mod logs;
mod state_machine;
mod storage;
mod network;
mod test;

#[macro_use]
extern crate log;



#[derive(Serialize, Deserialize, Debug, Clone, thiserror::Error)]
pub enum KLogError {
    #[error("Invalid format: {0}")]
    InvalidFormat(String),
}


pub type KResult<T> = Result<T, KLogError>;


#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct KLogEntry {
    pub id: u64, // The unique ID of the log entry
    pub timestamp: u64,
    pub node_id: u64, // The ID of the node that created the log entry
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KLogRequest {
    AppendLog { item: KLogEntry },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KLogResponse {
    Ok,
    Empty, // For empty and membership payloads
    AppendOk { id: u64 },
    Err(String),
}

pub type KNodeId = u64;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct KNode {
    pub id: KNodeId,
    pub addr: String,
    pub port: u16,
}

declare_raft_types!(
   pub KTypeConfig:
       D            = KLogRequest,
       R            = KLogResponse,
       Node = KNode,
       SnapshotData = tokio::fs::File,
);

pub type StorageResult<T> = Result<T, openraft::StorageError<KNodeId>>;
pub type KLogId = LogId<KNodeId>;