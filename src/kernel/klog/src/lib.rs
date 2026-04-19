#![allow(dead_code)]

#[macro_use]
extern crate log;

use openraft::Raft;
use openraft::{LogId, declare_raft_types};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

pub mod error;
pub mod logs;
pub mod network;
pub mod rpc;
pub(crate) mod service;
pub mod state_machine;
pub mod state_store;
#[cfg(test)]
mod test;
pub(crate) mod util;

#[derive(Serialize, Deserialize, Debug, Clone, thiserror::Error)]
pub enum KLogError {
    #[error("Invalid format: {0}")]
    InvalidFormat(String),
}

pub type KResult<T> = Result<T, KLogError>;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum KLogLevel {
    Trace,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
    Fatal,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct KLogEntry {
    pub id: u64, // The unique ID of the log entry
    pub timestamp: u64,
    pub node_id: u64, // The ID of the node that created the log entry
    #[serde(default)]
    pub request_id: Option<String>, // Optional idempotency key for dedup.
    #[serde(default)]
    pub level: KLogLevel,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub attrs: BTreeMap<String, String>,
    pub message: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct KLogMetaEntry {
    pub key: String,
    pub value: String,
    pub updated_at: u64,
    pub updated_by: KNodeId,
    #[serde(default)]
    pub revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KLogRequest {
    AppendLog {
        item: KLogEntry,
    },
    PutMeta {
        item: KLogMetaEntry,
        expected_revision: Option<u64>,
    },
    DeleteMeta {
        key: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KLogResponse {
    Ok,
    Empty, // For empty and membership payloads
    AppendOk {
        id: u64,
    },
    MetaPutOk {
        key: String,
        revision: u64,
    },
    MetaPutConflict {
        key: String,
        expected_revision: u64,
        current_revision: Option<u64>,
    },
    MetaDeleteOk {
        key: String,
        existed: bool,
        prev_meta: Option<KLogMetaEntry>,
    },
    Err(String),
}

pub type KNodeId = u64;

/// Selects how cluster-internal traffic reaches a specific peer node.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum KClusterTransportMode {
    #[default]
    Direct,
    GatewayProxy,
    Hybrid,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct KNode {
    pub id: KNodeId,
    pub addr: String,
    /// Raft protocol port for append-entries/vote/install-snapshot.
    pub port: u16,
    /// Inter-node service port for data/meta forwarding.
    #[serde(default)]
    pub inter_port: u16,
    /// Admin service port for cluster membership/cluster-state APIs.
    #[serde(default)]
    pub admin_port: u16,
    /// Client-facing json-rpc port.
    #[serde(default)]
    pub rpc_port: u16,
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

pub type KRaft = Raft<KTypeConfig>;
pub type KRaftRef = Arc<KRaft>;
