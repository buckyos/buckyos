#![allow(dead_code, clippy::result_large_err)]

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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct KLogMetaEntry {
    pub key: String,
    pub value: String,
    pub updated_at: u64,
    pub updated_by_node_name: String,
    /// Backward-compatible alias for `mod_revision`.
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub create_revision: u64,
    #[serde(default)]
    pub mod_revision: u64,
    #[serde(default)]
    pub version: u64,
}

impl KLogMetaEntry {
    pub fn set_mvcc_revision(&mut self, create_revision: u64, mod_revision: u64, version: u64) {
        self.revision = mod_revision;
        self.create_revision = create_revision;
        self.mod_revision = mod_revision;
        self.version = version;
    }

    pub fn effective_mod_revision(&self) -> u64 {
        if self.mod_revision != 0 {
            self.mod_revision
        } else {
            self.revision
        }
    }

    pub fn effective_create_revision(&self) -> u64 {
        if self.create_revision != 0 {
            self.create_revision
        } else {
            self.effective_mod_revision()
        }
    }

    pub fn effective_version(&self) -> u64 {
        if self.version != 0 {
            self.version
        } else {
            self.effective_mod_revision().max(1)
        }
    }

    pub fn normalize_mvcc_revision(&mut self) {
        let mod_revision = self.effective_mod_revision();
        let create_revision = self.effective_create_revision();
        let version = self.effective_version();
        self.set_mvcc_revision(create_revision, mod_revision, version);
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct KLogMetaVersion {
    /// Backward-compatible alias for `mod_revision`.
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub create_revision: u64,
    #[serde(default)]
    pub mod_revision: u64,
    #[serde(default)]
    pub version: u64,
    #[serde(default)]
    pub deleted: bool,
}

impl KLogMetaVersion {
    pub fn new(create_revision: u64, mod_revision: u64, version: u64, deleted: bool) -> Self {
        Self {
            revision: mod_revision,
            create_revision,
            mod_revision,
            version,
            deleted,
        }
    }

    pub fn from_entry(item: &KLogMetaEntry) -> Self {
        Self::new(
            item.effective_create_revision(),
            item.effective_mod_revision(),
            item.effective_version(),
            false,
        )
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct KLogMetaTxGuard {
    pub key: String,
    pub expected_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KLogMetaTxAction {
    Put {
        item: KLogMetaEntry,
        expected_revision: Option<u64>,
    },
    Delete {
        key: String,
        expected_revision: Option<u64>,
    },
}

impl Serialize for KLogMetaTxAction {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;

        if serializer.is_human_readable() {
            match self {
                Self::Put {
                    item,
                    expected_revision,
                } => {
                    let field_count = if expected_revision.is_some() { 3 } else { 2 };
                    let mut state = serializer.serialize_struct("KLogMetaTxAction", field_count)?;
                    state.serialize_field("action", "put")?;
                    state.serialize_field("item", item)?;
                    if expected_revision.is_some() {
                        state.serialize_field("expected_revision", expected_revision)?;
                    }
                    return state.end();
                }
                Self::Delete {
                    key,
                    expected_revision,
                } => {
                    let field_count = if expected_revision.is_some() { 3 } else { 2 };
                    let mut state = serializer.serialize_struct("KLogMetaTxAction", field_count)?;
                    state.serialize_field("action", "delete")?;
                    state.serialize_field("key", key)?;
                    if expected_revision.is_some() {
                        state.serialize_field("expected_revision", expected_revision)?;
                    }
                    return state.end();
                }
            }
        }

        let mut state = serializer.serialize_struct("KLogMetaTxAction", 4)?;
        match self {
            Self::Put {
                item,
                expected_revision,
            } => {
                state.serialize_field("action", "put")?;
                state.serialize_field("item", &Some(item))?;
                state.serialize_field("key", &Option::<String>::None)?;
                state.serialize_field("expected_revision", expected_revision)?;
            }
            Self::Delete {
                key,
                expected_revision,
            } => {
                state.serialize_field("action", "delete")?;
                state.serialize_field("item", &Option::<KLogMetaEntry>::None)?;
                state.serialize_field("key", &Some(key))?;
                state.serialize_field("expected_revision", expected_revision)?;
            }
        }
        state.end()
    }
}

impl<'de> Deserialize<'de> for KLogMetaTxAction {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct WireAction {
            action: String,
            #[serde(default)]
            item: Option<KLogMetaEntry>,
            #[serde(default)]
            key: Option<String>,
            #[serde(default)]
            expected_revision: Option<u64>,
        }

        let wire = WireAction::deserialize(deserializer)?;
        match wire.action.as_str() {
            "put" => {
                let item = wire
                    .item
                    .ok_or_else(|| serde::de::Error::missing_field("item"))?;
                Ok(Self::Put {
                    item,
                    expected_revision: wire.expected_revision,
                })
            }
            "delete" => {
                let key = wire
                    .key
                    .ok_or_else(|| serde::de::Error::missing_field("key"))?;
                Ok(Self::Delete {
                    key,
                    expected_revision: wire.expected_revision,
                })
            }
            action => Err(serde::de::Error::unknown_variant(
                action,
                &["put", "delete"],
            )),
        }
    }
}

impl KLogMetaTxAction {
    pub fn key(&self) -> &str {
        match self {
            KLogMetaTxAction::Put { item, .. } => item.key.as_str(),
            KLogMetaTxAction::Delete { key, .. } => key.as_str(),
        }
    }

    pub fn expected_revision(&self) -> Option<u64> {
        match self {
            KLogMetaTxAction::Put {
                expected_revision, ..
            } => *expected_revision,
            KLogMetaTxAction::Delete {
                expected_revision, ..
            } => *expected_revision,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct KLogMetaTxRequest {
    pub actions: BTreeMap<String, KLogMetaTxAction>,
    #[serde(default)]
    pub guard: Option<KLogMetaTxGuard>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct KLogMetaTxResponse {
    /// Backward-compatible map: live put/update returns `Some(mod_revision)`, delete returns `None`.
    pub revisions: BTreeMap<String, Option<u64>>,
    #[serde(default)]
    pub meta_versions: BTreeMap<String, KLogMetaVersion>,
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
    ExecMetaTx {
        tx: KLogMetaTxRequest,
    },
    CompactMeta {
        revision: u64,
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
        item: KLogMetaEntry,
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
        meta_version: Option<KLogMetaVersion>,
    },
    MetaTxOk {
        response: KLogMetaTxResponse,
    },
    MetaTxConflict {
        key: String,
        expected_revision: u64,
        current_revision: Option<u64>,
    },
    MetaCompactOk {
        compacted_revision: u64,
        current_revision: u64,
    },
    MetaCompactRejected {
        revision: u64,
        current_revision: u64,
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
    /// Stable BuckyOS node name used by gateway/proxy cluster routing.
    #[serde(default)]
    pub node_name: Option<String>,
    /// Optional external device identity for diagnosing node-id reuse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
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

#[cfg(test)]
mod meta_tx_action_tests {
    use super::*;

    #[test]
    fn meta_tx_request_roundtrips_with_json_and_bincode() {
        let action = KLogMetaTxAction::Put {
            item: KLogMetaEntry {
                key: "boot/config".to_string(),
                value: "{}".to_string(),
                updated_at: 1,
                updated_by_node_name: "ood1".to_string(),
                ..KLogMetaEntry::default()
            },
            expected_revision: Some(0),
        };

        let json = serde_json::to_value(&action).expect("serialize action to json");
        assert_eq!(
            json.get("action").and_then(|value| value.as_str()),
            Some("put")
        );
        assert!(json.get("key").is_none());
        let from_json: KLogMetaTxAction =
            serde_json::from_value(json).expect("deserialize action from json");
        assert_eq!(from_json, action);

        let encoded = bincode::serde::encode_to_vec(&action, bincode::config::legacy())
            .expect("serialize action to bincode");
        let (from_bincode, _): (KLogMetaTxAction, usize) =
            bincode::serde::decode_from_slice(&encoded, bincode::config::legacy())
                .expect("deserialize action from bincode");
        assert_eq!(from_bincode, action);

        let mut actions = BTreeMap::new();
        actions.insert("boot/config".to_string(), action);
        let request = KLogRequest::ExecMetaTx {
            tx: KLogMetaTxRequest {
                actions,
                guard: None,
            },
        };
        let encoded = bincode::serde::encode_to_vec(&request, bincode::config::legacy())
            .expect("serialize request to bincode");
        let (from_bincode, _): (KLogRequest, usize) =
            bincode::serde::decode_from_slice(&encoded, bincode::config::legacy())
                .expect("deserialize request from bincode");
        let KLogRequest::ExecMetaTx { tx } = from_bincode else {
            panic!("unexpected request variant");
        };
        assert!(tx.guard.is_none());
        assert_eq!(tx.actions.len(), 1);
    }
}
