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
    pub node_name: String, // The BuckyOS node name that created the log entry
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
    pub updated_by_node_name: String,
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

impl KClusterTransportMode {
    pub fn as_str(self) -> &'static str {
        match self {
            KClusterTransportMode::Direct => "direct",
            KClusterTransportMode::GatewayProxy => "gateway_proxy",
            KClusterTransportMode::Hybrid => "hybrid",
        }
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "direct" => Ok(KClusterTransportMode::Direct),
            "gateway_proxy" => Ok(KClusterTransportMode::GatewayProxy),
            "hybrid" => Ok(KClusterTransportMode::Hybrid),
            _ => Err("expected direct, gateway_proxy or hybrid".to_string()),
        }
    }
}

impl std::fmt::Display for KClusterTransportMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KClusterTransportConfig {
    pub mode: KClusterTransportMode,
    pub gateway_addr: String,
    pub gateway_route_prefix: String,
}

impl Default for KClusterTransportConfig {
    fn default() -> Self {
        Self {
            mode: KClusterTransportMode::Direct,
            gateway_addr: "127.0.0.1:3180".to_string(),
            gateway_route_prefix: "/.cluster/klog".to_string(),
        }
    }
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
    /// Stable node name for gateway/proxy routing.
    #[serde(default)]
    pub node_name: Option<String>,
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
