use crate::constants::{
    DEFAULT_ADMIN_LOCAL_ONLY, DEFAULT_ADMIN_PORT, DEFAULT_ADVERTISE_ADDR, DEFAULT_AUTO_BOOTSTRAP,
    DEFAULT_CLUSTER_GATEWAY_ADDR, DEFAULT_CLUSTER_GATEWAY_ROUTE_PREFIX,
    DEFAULT_CLUSTER_NETWORK_MODE, DEFAULT_ENABLE_RPC_SERVER, DEFAULT_INTER_NODE_PORT,
    DEFAULT_JOIN_BLOCKING, DEFAULT_JOIN_RETRY_CONFIG_CHANGE_CONFLICT_EXTRA_BACKOFF_MS,
    DEFAULT_JOIN_RETRY_INITIAL_INTERVAL_MS, DEFAULT_JOIN_RETRY_JITTER_RATIO,
    DEFAULT_JOIN_RETRY_MAX_ATTEMPTS, DEFAULT_JOIN_RETRY_MAX_INTERVAL_MS,
    DEFAULT_JOIN_RETRY_MULTIPLIER, DEFAULT_JOIN_RETRY_REQUEST_TIMEOUT_MS,
    DEFAULT_JOIN_RETRY_SHUFFLE_TARGETS, DEFAULT_JOIN_RETRY_STRATEGY, DEFAULT_LISTEN_HOST,
    DEFAULT_RAFT_ELECTION_TIMEOUT_MAX_MS, DEFAULT_RAFT_ELECTION_TIMEOUT_MIN_MS,
    DEFAULT_RAFT_HEARTBEAT_INTERVAL_MS, DEFAULT_RAFT_INSTALL_SNAPSHOT_TIMEOUT_MS,
    DEFAULT_RAFT_MAX_IN_SNAPSHOT_LOG_TO_KEEP, DEFAULT_RAFT_MAX_PAYLOAD_ENTRIES, DEFAULT_RAFT_PORT,
    DEFAULT_RAFT_PURGE_BATCH_SIZE, DEFAULT_RAFT_REPLICATION_LAG_THRESHOLD,
    DEFAULT_RAFT_SNAPSHOT_MAX_CHUNK_SIZE_BYTES, DEFAULT_RAFT_SNAPSHOT_POLICY,
    DEFAULT_RPC_BODY_LIMIT_BYTES, DEFAULT_RPC_CONCURRENCY_LIMIT, DEFAULT_RPC_LISTEN_HOST,
    DEFAULT_RPC_PORT, DEFAULT_RPC_TIMEOUT_MS, DEFAULT_STATE_STORE_SYNC_WRITE,
    ENV_ADMIN_ADVERTISE_PORT, ENV_ADMIN_LISTEN_ADDR, ENV_ADMIN_LOCAL_ONLY, ENV_ADVERTISE_ADDR,
    ENV_ADVERTISE_INTER_PORT, ENV_ADVERTISE_NODE_NAME, ENV_ADVERTISE_PORT, ENV_AUTO_BOOTSTRAP,
    ENV_CLUSTER_GATEWAY_ADDR, ENV_CLUSTER_GATEWAY_ROUTE_PREFIX, ENV_CLUSTER_ID, ENV_CLUSTER_NAME,
    ENV_CLUSTER_NETWORK_MODE, ENV_CONFIG_FILE, ENV_DATA_DIR, ENV_ENABLE_RPC_SERVER,
    ENV_INTER_NODE_LISTEN_ADDR, ENV_JOIN_BLOCKING,
    ENV_JOIN_RETRY_CONFIG_CHANGE_CONFLICT_EXTRA_BACKOFF_MS, ENV_JOIN_RETRY_INITIAL_INTERVAL_MS,
    ENV_JOIN_RETRY_JITTER_RATIO, ENV_JOIN_RETRY_MAX_ATTEMPTS, ENV_JOIN_RETRY_MAX_INTERVAL_MS,
    ENV_JOIN_RETRY_MULTIPLIER, ENV_JOIN_RETRY_REQUEST_TIMEOUT_MS, ENV_JOIN_RETRY_SHUFFLE_TARGETS,
    ENV_JOIN_RETRY_STRATEGY, ENV_JOIN_TARGET_ROLE, ENV_JOIN_TARGETS, ENV_LISTEN_ADDR, ENV_NODE_ID,
    ENV_RAFT_ELECTION_TIMEOUT_MAX_MS, ENV_RAFT_ELECTION_TIMEOUT_MIN_MS,
    ENV_RAFT_HEARTBEAT_INTERVAL_MS, ENV_RAFT_INSTALL_SNAPSHOT_TIMEOUT_MS,
    ENV_RAFT_MAX_IN_SNAPSHOT_LOG_TO_KEEP, ENV_RAFT_MAX_PAYLOAD_ENTRIES, ENV_RAFT_PURGE_BATCH_SIZE,
    ENV_RAFT_REPLICATION_LAG_THRESHOLD, ENV_RAFT_SNAPSHOT_MAX_CHUNK_SIZE_BYTES,
    ENV_RAFT_SNAPSHOT_POLICY, ENV_RPC_ADVERTISE_PORT, ENV_RPC_APPEND_BODY_LIMIT_BYTES,
    ENV_RPC_APPEND_CONCURRENCY, ENV_RPC_APPEND_TIMEOUT_MS, ENV_RPC_JSONRPC_BODY_LIMIT_BYTES,
    ENV_RPC_JSONRPC_CONCURRENCY, ENV_RPC_JSONRPC_TIMEOUT_MS, ENV_RPC_LISTEN_ADDR,
    ENV_RPC_QUERY_BODY_LIMIT_BYTES, ENV_RPC_QUERY_CONCURRENCY, ENV_RPC_QUERY_TIMEOUT_MS,
    ENV_STATE_STORE_SYNC_WRITE, KLOG_SERVICE_NAME,
};
use buckyos_kit::get_buckyos_service_data_dir;
use klog::rpc::{KRpcRoutePolicy, KRpcServerPolicy};
use klog::{KClusterTransportMode, KNodeId};
use log::error;
use openraft::{Config as OpenRaftConfig, SnapshotPolicy};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const DEFAULT_JOIN_TARGET_ROLE: KLogJoinTargetRole = KLogJoinTargetRole::Voter;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum KLogJoinTargetRole {
    Learner,
    Voter,
}

impl KLogJoinTargetRole {
    pub fn as_str(self) -> &'static str {
        match self {
            KLogJoinTargetRole::Learner => "learner",
            KLogJoinTargetRole::Voter => "voter",
        }
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "learner" => Ok(KLogJoinTargetRole::Learner),
            "voter" => Ok(KLogJoinTargetRole::Voter),
            _ => Err("expected learner or voter".to_string()),
        }
    }
}

impl std::fmt::Display for KLogJoinTargetRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum KLogJoinRetryStrategy {
    Fixed,
    Exponential,
}

impl KLogJoinRetryStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            KLogJoinRetryStrategy::Fixed => "fixed",
            KLogJoinRetryStrategy::Exponential => "exponential",
        }
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "fixed" => Ok(KLogJoinRetryStrategy::Fixed),
            "exponential" => Ok(KLogJoinRetryStrategy::Exponential),
            _ => Err("expected fixed or exponential".to_string()),
        }
    }
}

impl std::fmt::Display for KLogJoinRetryStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KLogRuntimeConfigSource {
    Env,
    File(PathBuf),
    Buckyos,
}

impl std::fmt::Display for KLogRuntimeConfigSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Env => write!(f, "env"),
            Self::File(path) => write!(f, "file({})", path.display()),
            Self::Buckyos => write!(f, "buckyos"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct KLogRpcRouteConfig {
    /// Request timeout in milliseconds.
    pub timeout_ms: u64,

    /// Maximum request body bytes.
    pub body_limit_bytes: usize,

    /// Maximum in-flight requests.
    pub concurrency: usize,
}

impl Default for KLogRpcRouteConfig {
    fn default() -> Self {
        Self {
            timeout_ms: DEFAULT_RPC_TIMEOUT_MS,
            body_limit_bytes: DEFAULT_RPC_BODY_LIMIT_BYTES,
            concurrency: DEFAULT_RPC_CONCURRENCY_LIMIT,
        }
    }
}

impl From<KLogRpcRouteConfig> for KRpcRoutePolicy {
    fn from(value: KLogRpcRouteConfig) -> Self {
        Self {
            timeout_ms: value.timeout_ms,
            body_limit_bytes: value.body_limit_bytes,
            concurrency: value.concurrency,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct KLogRpcConfig {
    /// Route policy for `/klog/data/append`.
    pub append: KLogRpcRouteConfig,

    /// Route policy for `/klog/data/query`.
    pub query: KLogRpcRouteConfig,

    /// Route policy shared by `/klog/rpc` and `/kapi/klog-service`.
    pub jsonrpc: KLogRpcRouteConfig,
}

impl From<KLogRpcConfig> for KRpcServerPolicy {
    fn from(value: KLogRpcConfig) -> Self {
        Self {
            append: value.append.into(),
            query: value.query.into(),
            jsonrpc: value.jsonrpc.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KLogRaftConfig {
    /// Election timeout lower bound in milliseconds.
    pub election_timeout_min_ms: u64,

    /// Election timeout upper bound in milliseconds.
    pub election_timeout_max_ms: u64,

    /// Heartbeat interval in milliseconds.
    pub heartbeat_interval_ms: u64,

    /// Install snapshot timeout in milliseconds.
    pub install_snapshot_timeout_ms: u64,

    /// Max number of log entries in one replication payload.
    pub max_payload_entries: u64,

    /// Replication lag threshold to switch to snapshot transfer.
    pub replication_lag_threshold: u64,

    /// Snapshot policy string, e.g. "since_last:5000" or "never".
    pub snapshot_policy: String,

    /// Snapshot transport max chunk size in bytes.
    pub snapshot_max_chunk_size_bytes: u64,

    /// Max number of logs already in snapshot to keep.
    pub max_in_snapshot_log_to_keep: u64,

    /// Purge batch size for applied logs.
    pub purge_batch_size: u64,
}

impl Default for KLogRaftConfig {
    fn default() -> Self {
        Self {
            election_timeout_min_ms: DEFAULT_RAFT_ELECTION_TIMEOUT_MIN_MS,
            election_timeout_max_ms: DEFAULT_RAFT_ELECTION_TIMEOUT_MAX_MS,
            heartbeat_interval_ms: DEFAULT_RAFT_HEARTBEAT_INTERVAL_MS,
            install_snapshot_timeout_ms: DEFAULT_RAFT_INSTALL_SNAPSHOT_TIMEOUT_MS,
            max_payload_entries: DEFAULT_RAFT_MAX_PAYLOAD_ENTRIES,
            replication_lag_threshold: DEFAULT_RAFT_REPLICATION_LAG_THRESHOLD,
            snapshot_policy: DEFAULT_RAFT_SNAPSHOT_POLICY.to_string(),
            snapshot_max_chunk_size_bytes: DEFAULT_RAFT_SNAPSHOT_MAX_CHUNK_SIZE_BYTES,
            max_in_snapshot_log_to_keep: DEFAULT_RAFT_MAX_IN_SNAPSHOT_LOG_TO_KEEP,
            purge_batch_size: DEFAULT_RAFT_PURGE_BATCH_SIZE,
        }
    }
}

impl KLogRaftConfig {
    pub fn to_openraft_config(&self, cluster_name: String) -> Result<OpenRaftConfig, String> {
        let snapshot_policy = parse_snapshot_policy(&self.snapshot_policy)?;
        let cfg = OpenRaftConfig {
            cluster_name,
            election_timeout_min: self.election_timeout_min_ms,
            election_timeout_max: self.election_timeout_max_ms,
            heartbeat_interval: self.heartbeat_interval_ms,
            install_snapshot_timeout: self.install_snapshot_timeout_ms,
            max_payload_entries: self.max_payload_entries,
            replication_lag_threshold: self.replication_lag_threshold,
            snapshot_policy,
            snapshot_max_chunk_size: self.snapshot_max_chunk_size_bytes,
            max_in_snapshot_log_to_keep: self.max_in_snapshot_log_to_keep,
            purge_batch_size: self.purge_batch_size,
            ..Default::default()
        };
        cfg.validate()
            .map_err(|e| format!("Invalid openraft config: {}", e))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KLogJoinRetryConfig {
    /// Retry strategy: fixed interval or exponential backoff.
    pub strategy: KLogJoinRetryStrategy,

    /// Initial retry interval in milliseconds.
    pub initial_interval_ms: u64,

    /// Max retry interval in milliseconds.
    pub max_interval_ms: u64,

    /// Exponential multiplier (only used by exponential strategy).
    pub multiplier: f64,

    /// Random jitter ratio in [0.0, 1.0].
    pub jitter_ratio: f64,

    /// Max retry attempts, 0 means retry forever.
    pub max_attempts: u32,

    /// HTTP timeout for join/admin requests in milliseconds.
    pub request_timeout_ms: u64,

    /// Whether to shuffle join targets every retry round.
    pub shuffle_targets_each_round: bool,

    /// Extra backoff in milliseconds for config-change conflict errors.
    pub config_change_conflict_extra_backoff_ms: u64,
}

impl Default for KLogJoinRetryConfig {
    fn default() -> Self {
        Self {
            strategy: KLogJoinRetryStrategy::parse(DEFAULT_JOIN_RETRY_STRATEGY)
                .expect("DEFAULT_JOIN_RETRY_STRATEGY must be valid"),
            initial_interval_ms: DEFAULT_JOIN_RETRY_INITIAL_INTERVAL_MS,
            max_interval_ms: DEFAULT_JOIN_RETRY_MAX_INTERVAL_MS,
            multiplier: DEFAULT_JOIN_RETRY_MULTIPLIER,
            jitter_ratio: DEFAULT_JOIN_RETRY_JITTER_RATIO,
            max_attempts: DEFAULT_JOIN_RETRY_MAX_ATTEMPTS,
            request_timeout_ms: DEFAULT_JOIN_RETRY_REQUEST_TIMEOUT_MS,
            shuffle_targets_each_round: DEFAULT_JOIN_RETRY_SHUFFLE_TARGETS,
            config_change_conflict_extra_backoff_ms:
                DEFAULT_JOIN_RETRY_CONFIG_CHANGE_CONFLICT_EXTRA_BACKOFF_MS,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KLogClusterNetworkConfig {
    /// Cluster internal transport mode.
    pub mode: KClusterTransportMode,

    /// Local gateway/proxy address used by proxy and hybrid modes.
    pub gateway_addr: String,

    /// Gateway route prefix used by proxy and hybrid modes.
    pub gateway_route_prefix: String,
}

impl Default for KLogClusterNetworkConfig {
    fn default() -> Self {
        Self {
            mode: KClusterTransportMode::parse(DEFAULT_CLUSTER_NETWORK_MODE)
                .expect("DEFAULT_CLUSTER_NETWORK_MODE must be valid"),
            gateway_addr: DEFAULT_CLUSTER_GATEWAY_ADDR.to_string(),
            gateway_route_prefix: DEFAULT_CLUSTER_GATEWAY_ROUTE_PREFIX.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KLogRuntimeConfig {
    /// Raft node id in current cluster, must be greater than 0.
    pub node_id: KNodeId,

    /// Raft protocol listen address, format "host:port".
    pub listen_addr: String,

    /// Inter-node data/meta listen address, format "host:port".
    pub inter_node_listen_addr: String,

    /// Admin API listen address, format "host:port".
    pub admin_listen_addr: String,

    /// Whether client-facing RPC server is enabled.
    pub enable_rpc_server: bool,

    /// Client-facing RPC listen address, format "host:port".
    pub rpc_listen_addr: String,

    /// Advertised host/IP for other raft nodes to connect.
    pub advertise_addr: String,

    /// Advertised raft protocol port for peer-to-peer traffic.
    pub advertise_port: u16,

    /// Advertised inter-node data/meta port.
    pub advertise_inter_port: u16,

    /// Advertised admin API port.
    pub advertise_admin_port: u16,

    /// Advertised client RPC port, set to 0 when RPC server is disabled.
    pub rpc_advertise_port: u16,

    /// Stable advertised node name used for gateway/proxy cluster routing.
    pub advertise_node_name: Option<String>,

    /// Root data directory for raft log, state store and snapshots.
    pub data_dir: PathBuf,

    /// Human-readable cluster name for operation and diagnostics.
    pub cluster_name: String,

    /// Stable cluster identity string for join validation.
    pub cluster_id: String,

    /// Whether this node should bootstrap a new single-node cluster.
    pub auto_bootstrap: bool,

    /// Whether state store writes use sync/fdatasync for durability.
    pub state_store_sync_write: bool,

    /// Seed admin targets for auto join, each item format "host:port".
    pub join_targets: Vec<String>,

    /// Whether add-learner uses blocking mode during auto join.
    pub join_blocking: bool,

    /// Target role after joining cluster: learner or voter.
    pub join_target_role: KLogJoinTargetRole,

    /// Retry/backoff policy for auto join workflow.
    pub join_retry: KLogJoinRetryConfig,

    /// OpenRaft core runtime settings.
    pub raft: KLogRaftConfig,

    /// Cluster internal transport mode and future routing settings.
    pub cluster_network: KLogClusterNetworkConfig,

    /// Restrict admin APIs to loopback clients only.
    pub admin_local_only: bool,

    /// Route-level RPC policies for append/query/jsonrpc.
    pub rpc: KLogRpcConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KLogNetworkConfigPatch {
    /// Optional override for raft listen address.
    pub listen_addr: Option<String>,

    /// Optional override for inter-node data/meta listen address.
    pub inter_node_listen_addr: Option<String>,

    /// Optional override for admin API listen address.
    pub admin_listen_addr: Option<String>,

    /// Optional override for client RPC enable flag.
    pub enable_rpc_server: Option<bool>,

    /// Optional override for client RPC listen address.
    pub rpc_listen_addr: Option<String>,

    /// Optional override for advertised host/IP.
    pub advertise_addr: Option<String>,

    /// Optional override for advertised raft port.
    pub advertise_port: Option<u16>,

    /// Optional override for advertised inter-node data/meta port.
    pub advertise_inter_port: Option<u16>,

    /// Optional override for advertised admin API port.
    pub advertise_admin_port: Option<u16>,

    /// Optional override for advertised client RPC port.
    pub rpc_advertise_port: Option<u16>,

    /// Optional override for advertised stable node name.
    pub advertise_node_name: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KLogStorageConfigPatch {
    /// Optional override for root data directory.
    pub data_dir: Option<PathBuf>,

    /// Optional override for state-store sync-write mode.
    pub state_store_sync_write: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KLogClusterConfigPatch {
    /// Optional override for cluster name.
    pub name: Option<String>,

    /// Optional override for cluster id.
    pub id: Option<String>,

    /// Optional override for auto bootstrap switch.
    pub auto_bootstrap: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KLogClusterNetworkConfigPatch {
    /// Optional cluster internal transport mode.
    pub mode: Option<KClusterTransportMode>,

    /// Optional local gateway/proxy address for cluster transport.
    pub gateway_addr: Option<String>,

    /// Optional gateway route prefix for cluster transport.
    pub gateway_route_prefix: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KLogJoinConfigPatch {
    /// Optional override for join seed admin targets.
    pub targets: Option<Vec<String>>,

    /// Optional override for add-learner blocking mode.
    pub blocking: Option<bool>,

    /// Optional override for target role after join.
    pub target_role: Option<KLogJoinTargetRole>,

    /// Optional override for join retry/backoff policy.
    pub retry: Option<KLogJoinRetryConfigPatch>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KLogJoinRetryConfigPatch {
    /// Optional retry strategy: fixed or exponential.
    pub strategy: Option<KLogJoinRetryStrategy>,

    /// Optional initial retry interval in milliseconds.
    pub initial_interval_ms: Option<u64>,

    /// Optional max retry interval in milliseconds.
    pub max_interval_ms: Option<u64>,

    /// Optional exponential multiplier.
    pub multiplier: Option<f64>,

    /// Optional jitter ratio in [0.0, 1.0].
    pub jitter_ratio: Option<f64>,

    /// Optional max retry attempts, 0 means retry forever.
    pub max_attempts: Option<u32>,

    /// Optional HTTP request timeout in milliseconds.
    pub request_timeout_ms: Option<u64>,

    /// Optional switch to shuffle targets every retry round.
    pub shuffle_targets_each_round: Option<bool>,

    /// Optional extra backoff for config-change conflict errors in milliseconds.
    pub config_change_conflict_extra_backoff_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KLogRaftConfigPatch {
    /// Optional election timeout lower bound in milliseconds.
    pub election_timeout_min_ms: Option<u64>,

    /// Optional election timeout upper bound in milliseconds.
    pub election_timeout_max_ms: Option<u64>,

    /// Optional heartbeat interval in milliseconds.
    pub heartbeat_interval_ms: Option<u64>,

    /// Optional install snapshot timeout in milliseconds.
    pub install_snapshot_timeout_ms: Option<u64>,

    /// Optional max payload entries per replication round.
    pub max_payload_entries: Option<u64>,

    /// Optional replication lag threshold before snapshot transfer.
    pub replication_lag_threshold: Option<u64>,

    /// Optional snapshot policy string.
    pub snapshot_policy: Option<String>,

    /// Optional snapshot max chunk size in bytes.
    pub snapshot_max_chunk_size_bytes: Option<u64>,

    /// Optional max in-snapshot logs to keep.
    pub max_in_snapshot_log_to_keep: Option<u64>,

    /// Optional purge batch size.
    pub purge_batch_size: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KLogAdminConfigPatch {
    /// Optional override for admin local-only access control.
    pub local_only: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KLogRpcRouteConfigPatch {
    /// Optional timeout in milliseconds.
    pub timeout_ms: Option<u64>,

    /// Optional request body limit in bytes.
    pub body_limit_bytes: Option<usize>,

    /// Optional concurrency limit.
    pub concurrency: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KLogRpcConfigPatch {
    /// Optional append route policy patch.
    pub append: Option<KLogRpcRouteConfigPatch>,

    /// Optional query route policy patch.
    pub query: Option<KLogRpcRouteConfigPatch>,

    /// Optional json-rpc route policy patch.
    pub jsonrpc: Option<KLogRpcRouteConfigPatch>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KLogRuntimeConfigPatch {
    /// Optional grouped network section.
    pub network: Option<KLogNetworkConfigPatch>,

    /// Optional grouped storage section.
    pub storage: Option<KLogStorageConfigPatch>,

    /// Optional grouped cluster identity section.
    pub cluster: Option<KLogClusterConfigPatch>,

    /// Optional grouped auto-join section.
    pub join: Option<KLogJoinConfigPatch>,

    /// Optional grouped raft runtime section.
    pub raft: Option<KLogRaftConfigPatch>,

    /// Optional grouped cluster internal transport section.
    pub cluster_network: Option<KLogClusterNetworkConfigPatch>,

    /// Optional grouped admin API section.
    pub admin: Option<KLogAdminConfigPatch>,

    /// Optional grouped rpc policy section.
    pub rpc: Option<KLogRpcConfigPatch>,

    /// Required node id; can also come from env.
    pub node_id: Option<KNodeId>,
}

// Placeholder type for future buckyos config integration.
// It intentionally mirrors daemon runtime fields to keep conversion simple.
pub type BuckyosKlogConfig = KLogRuntimeConfigPatch;

impl KLogRuntimeConfig {
    pub fn load() -> Result<(Self, KLogRuntimeConfigSource), String> {
        match std::env::var(ENV_CONFIG_FILE) {
            Ok(path_raw) => {
                let path = path_raw.trim();
                if path.is_empty() {
                    return Err(format!("{} is set but empty", ENV_CONFIG_FILE));
                }

                let path = PathBuf::from(path);
                let cfg = Self::from_file(&path)?;
                Ok((cfg, KLogRuntimeConfigSource::File(path)))
            }
            Err(std::env::VarError::NotPresent) => {
                let cfg = Self::from_env()?;
                Ok((cfg, KLogRuntimeConfigSource::Env))
            }
            Err(std::env::VarError::NotUnicode(_)) => {
                Err(format!("{} contains invalid unicode", ENV_CONFIG_FILE))
            }
        }
    }

    pub fn from_env() -> Result<Self, String> {
        let patch = KLogRuntimeConfigPatch {
            node_id: parse_env_u64(ENV_NODE_ID)?,
            network: Some(KLogNetworkConfigPatch {
                listen_addr: parse_env_string(ENV_LISTEN_ADDR)?,
                inter_node_listen_addr: parse_env_string(ENV_INTER_NODE_LISTEN_ADDR)?,
                admin_listen_addr: parse_env_string(ENV_ADMIN_LISTEN_ADDR)?,
                enable_rpc_server: parse_env_bool(ENV_ENABLE_RPC_SERVER)?,
                rpc_listen_addr: parse_env_string(ENV_RPC_LISTEN_ADDR)?,
                advertise_addr: parse_env_string(ENV_ADVERTISE_ADDR)?,
                advertise_port: parse_env_u16(ENV_ADVERTISE_PORT)?,
                advertise_inter_port: parse_env_u16(ENV_ADVERTISE_INTER_PORT)?,
                advertise_admin_port: parse_env_u16(ENV_ADMIN_ADVERTISE_PORT)?,
                rpc_advertise_port: parse_env_u16(ENV_RPC_ADVERTISE_PORT)?,
                advertise_node_name: parse_env_string(ENV_ADVERTISE_NODE_NAME)?,
            }),
            storage: Some(KLogStorageConfigPatch {
                data_dir: parse_env_pathbuf(ENV_DATA_DIR)?,
                state_store_sync_write: parse_env_bool(ENV_STATE_STORE_SYNC_WRITE)?,
            }),
            cluster: Some(KLogClusterConfigPatch {
                name: parse_env_string(ENV_CLUSTER_NAME)?,
                id: parse_env_string(ENV_CLUSTER_ID)?,
                auto_bootstrap: parse_env_bool(ENV_AUTO_BOOTSTRAP)?,
            }),
            cluster_network: Some(KLogClusterNetworkConfigPatch {
                mode: parse_env_cluster_transport_mode(ENV_CLUSTER_NETWORK_MODE)?,
                gateway_addr: parse_env_string(ENV_CLUSTER_GATEWAY_ADDR)?,
                gateway_route_prefix: parse_env_string(ENV_CLUSTER_GATEWAY_ROUTE_PREFIX)?,
            }),
            join: Some(KLogJoinConfigPatch {
                targets: parse_env_string_list(ENV_JOIN_TARGETS)?,
                blocking: parse_env_bool(ENV_JOIN_BLOCKING)?,
                target_role: parse_env_join_target_role(ENV_JOIN_TARGET_ROLE)?,
                retry: Some(KLogJoinRetryConfigPatch {
                    strategy: parse_env_join_retry_strategy(ENV_JOIN_RETRY_STRATEGY)?,
                    initial_interval_ms: parse_env_u64(ENV_JOIN_RETRY_INITIAL_INTERVAL_MS)?,
                    max_interval_ms: parse_env_u64(ENV_JOIN_RETRY_MAX_INTERVAL_MS)?,
                    multiplier: parse_env_f64(ENV_JOIN_RETRY_MULTIPLIER)?,
                    jitter_ratio: parse_env_f64(ENV_JOIN_RETRY_JITTER_RATIO)?,
                    max_attempts: parse_env_u32(ENV_JOIN_RETRY_MAX_ATTEMPTS)?,
                    request_timeout_ms: parse_env_u64(ENV_JOIN_RETRY_REQUEST_TIMEOUT_MS)?,
                    shuffle_targets_each_round: parse_env_bool(ENV_JOIN_RETRY_SHUFFLE_TARGETS)?,
                    config_change_conflict_extra_backoff_ms: parse_env_u64(
                        ENV_JOIN_RETRY_CONFIG_CHANGE_CONFLICT_EXTRA_BACKOFF_MS,
                    )?,
                }),
            }),
            raft: Some(KLogRaftConfigPatch {
                election_timeout_min_ms: parse_env_u64(ENV_RAFT_ELECTION_TIMEOUT_MIN_MS)?,
                election_timeout_max_ms: parse_env_u64(ENV_RAFT_ELECTION_TIMEOUT_MAX_MS)?,
                heartbeat_interval_ms: parse_env_u64(ENV_RAFT_HEARTBEAT_INTERVAL_MS)?,
                install_snapshot_timeout_ms: parse_env_u64(ENV_RAFT_INSTALL_SNAPSHOT_TIMEOUT_MS)?,
                max_payload_entries: parse_env_u64(ENV_RAFT_MAX_PAYLOAD_ENTRIES)?,
                replication_lag_threshold: parse_env_u64(ENV_RAFT_REPLICATION_LAG_THRESHOLD)?,
                snapshot_policy: parse_env_string(ENV_RAFT_SNAPSHOT_POLICY)?,
                snapshot_max_chunk_size_bytes: parse_env_u64(
                    ENV_RAFT_SNAPSHOT_MAX_CHUNK_SIZE_BYTES,
                )?,
                max_in_snapshot_log_to_keep: parse_env_u64(ENV_RAFT_MAX_IN_SNAPSHOT_LOG_TO_KEEP)?,
                purge_batch_size: parse_env_u64(ENV_RAFT_PURGE_BATCH_SIZE)?,
            }),
            admin: Some(KLogAdminConfigPatch {
                local_only: parse_env_bool(ENV_ADMIN_LOCAL_ONLY)?,
            }),
            rpc: Some(KLogRpcConfigPatch {
                append: Some(KLogRpcRouteConfigPatch {
                    timeout_ms: parse_env_u64(ENV_RPC_APPEND_TIMEOUT_MS)?,
                    body_limit_bytes: parse_env_usize(ENV_RPC_APPEND_BODY_LIMIT_BYTES)?,
                    concurrency: parse_env_usize(ENV_RPC_APPEND_CONCURRENCY)?,
                }),
                query: Some(KLogRpcRouteConfigPatch {
                    timeout_ms: parse_env_u64(ENV_RPC_QUERY_TIMEOUT_MS)?,
                    body_limit_bytes: parse_env_usize(ENV_RPC_QUERY_BODY_LIMIT_BYTES)?,
                    concurrency: parse_env_usize(ENV_RPC_QUERY_CONCURRENCY)?,
                }),
                jsonrpc: Some(KLogRpcRouteConfigPatch {
                    timeout_ms: parse_env_u64(ENV_RPC_JSONRPC_TIMEOUT_MS)?,
                    body_limit_bytes: parse_env_usize(ENV_RPC_JSONRPC_BODY_LIMIT_BYTES)?,
                    concurrency: parse_env_usize(ENV_RPC_JSONRPC_CONCURRENCY)?,
                }),
            }),
            ..Default::default()
        };

        Self::from_patch(patch)
    }

    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let path_ref = path.as_ref();
        let content = std::fs::read_to_string(path_ref).map_err(|e| {
            format!(
                "Failed to read klog config file {}: {}",
                path_ref.display(),
                e
            )
        })?;

        let patch: KLogRuntimeConfigPatch = toml::from_str(&content).map_err(|e| {
            format!(
                "Failed to parse klog config file {}: {}",
                path_ref.display(),
                e
            )
        })?;

        Self::from_patch(patch)
    }

    pub fn from_buckyos_config(cfg: &BuckyosKlogConfig) -> Result<Self, String> {
        Self::from_patch(cfg.clone())
    }

    pub fn load_from_buckyos(
        cfg: &BuckyosKlogConfig,
    ) -> Result<(Self, KLogRuntimeConfigSource), String> {
        let config = Self::from_buckyos_config(cfg)?;
        Ok((config, KLogRuntimeConfigSource::Buckyos))
    }

    fn from_patch(patch: KLogRuntimeConfigPatch) -> Result<Self, String> {
        let KLogRuntimeConfigPatch {
            network,
            storage,
            cluster,
            join,
            raft,
            cluster_network,
            admin,
            rpc,
            node_id,
        } = patch;

        let network = network.unwrap_or_default();
        let storage = storage.unwrap_or_default();
        let cluster = cluster.unwrap_or_default();
        let join = join.unwrap_or_default();
        let raft = raft.unwrap_or_default();
        let cluster_network = cluster_network.unwrap_or_default();
        let admin = admin.unwrap_or_default();
        let rpc = rpc.unwrap_or_default();

        let node_id = match node_id {
            Some(v) => v,
            None => {
                let msg = "Missing required field: node_id (or KLOG_NODE_ID)".to_string();
                error!("{}", msg);
                return Err(msg);
            }
        };
        if node_id == 0 {
            let msg = "Invalid node_id=0: node_id must be greater than 0".to_string();
            error!("{}", msg);
            return Err(msg);
        }

        let join_targets = join.targets.unwrap_or_default();
        let auto_bootstrap = cluster.auto_bootstrap.unwrap_or(DEFAULT_AUTO_BOOTSTRAP);

        let cluster_name = match cluster.name {
            Some(v) if !v.trim().is_empty() => v,
            _ => {
                let msg = "Missing required field: cluster.name (or KLOG_CLUSTER_NAME)".to_string();
                error!("{}", msg);
                return Err(msg);
            }
        };
        let cluster_id = match cluster.id {
            Some(v) if !v.trim().is_empty() => v,
            _ => {
                let msg = "Missing required field: cluster.id (or KLOG_CLUSTER_ID)".to_string();
                error!("{}", msg);
                return Err(msg);
            }
        };
        if cluster_id.trim().is_empty() {
            let msg = "Invalid cluster_id: cluster id must not be empty".to_string();
            error!("{}", msg);
            return Err(msg);
        }

        if auto_bootstrap && !join_targets.is_empty() {
            let msg = "Invalid config: auto_bootstrap=true must not be combined with non-empty join.targets".to_string();
            error!("{}", msg);
            return Err(msg);
        }

        let default_data_dir = default_data_dir();
        let rpc_cfg = merge_rpc_config(rpc)?;
        let join_retry_cfg = merge_join_retry_config(join.retry.unwrap_or_default())?;
        let raft_cfg = merge_raft_config(raft)?;
        let cluster_network_cfg = merge_cluster_network_config(cluster_network)?;
        let listen_addr = network.listen_addr.unwrap_or_else(default_listen_addr);
        let inter_node_listen_addr = network
            .inter_node_listen_addr
            .unwrap_or_else(default_inter_node_listen_addr);
        let admin_listen_addr = network
            .admin_listen_addr
            .unwrap_or_else(default_admin_listen_addr);
        if admin_listen_addr == listen_addr {
            let msg = format!(
                "Invalid config: network.admin_listen_addr ({}) must not equal network.listen_addr ({})",
                admin_listen_addr, listen_addr
            );
            error!("{}", msg);
            return Err(msg);
        }
        if admin_listen_addr == inter_node_listen_addr {
            let msg = format!(
                "Invalid config: network.admin_listen_addr ({}) must not equal network.inter_node_listen_addr ({})",
                admin_listen_addr, inter_node_listen_addr
            );
            error!("{}", msg);
            return Err(msg);
        }
        let advertise_port = network.advertise_port.unwrap_or(DEFAULT_RAFT_PORT);
        let advertise_inter_port = network
            .advertise_inter_port
            .unwrap_or(DEFAULT_INTER_NODE_PORT);
        let advertise_admin_port = network
            .advertise_admin_port
            .or_else(|| parse_port_from_addr(&admin_listen_addr))
            .unwrap_or(DEFAULT_ADMIN_PORT);
        let advertise_node_name = network
            .advertise_node_name
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        if advertise_admin_port == advertise_port {
            let msg = format!(
                "Invalid config: network.advertise_admin_port ({}) must not equal network.advertise_port ({})",
                advertise_admin_port, advertise_port
            );
            error!("{}", msg);
            return Err(msg);
        }
        if advertise_admin_port == advertise_inter_port {
            let msg = format!(
                "Invalid config: network.advertise_admin_port ({}) must not equal network.advertise_inter_port ({})",
                advertise_admin_port, advertise_inter_port
            );
            error!("{}", msg);
            return Err(msg);
        }
        if let Some(node_name) = advertise_node_name.as_ref()
            && node_name.contains('/')
        {
            let msg = format!(
                "Invalid config: network.advertise_node_name ({}) must not contain '/'",
                node_name
            );
            error!("{}", msg);
            return Err(msg);
        }
        if cluster_network_cfg.mode != KClusterTransportMode::Direct
            && advertise_node_name.is_none()
        {
            let msg = format!(
                "Missing required field: network.advertise_node_name for cluster_network.mode={}",
                cluster_network_cfg.mode
            );
            error!("{}", msg);
            return Err(msg);
        }

        Ok(Self {
            node_id,
            listen_addr,
            inter_node_listen_addr,
            admin_listen_addr,
            enable_rpc_server: network
                .enable_rpc_server
                .unwrap_or(DEFAULT_ENABLE_RPC_SERVER),
            rpc_listen_addr: network
                .rpc_listen_addr
                .unwrap_or_else(default_rpc_listen_addr),
            advertise_addr: network
                .advertise_addr
                .unwrap_or_else(|| DEFAULT_ADVERTISE_ADDR.to_string()),
            advertise_port,
            advertise_inter_port,
            advertise_admin_port,
            rpc_advertise_port: network.rpc_advertise_port.unwrap_or(DEFAULT_RPC_PORT),
            advertise_node_name,
            data_dir: storage.data_dir.unwrap_or(default_data_dir),
            cluster_name,
            cluster_id,
            auto_bootstrap,
            state_store_sync_write: storage
                .state_store_sync_write
                .unwrap_or(DEFAULT_STATE_STORE_SYNC_WRITE),
            join_targets,
            join_blocking: join.blocking.unwrap_or(DEFAULT_JOIN_BLOCKING),
            join_target_role: join.target_role.unwrap_or(DEFAULT_JOIN_TARGET_ROLE),
            join_retry: join_retry_cfg,
            raft: raft_cfg,
            cluster_network: cluster_network_cfg,
            admin_local_only: admin.local_only.unwrap_or(DEFAULT_ADMIN_LOCAL_ONLY),
            rpc: rpc_cfg,
        })
    }
}

fn default_data_dir() -> PathBuf {
    get_buckyos_service_data_dir(KLOG_SERVICE_NAME)
}

fn default_listen_addr() -> String {
    format!("{}:{}", DEFAULT_LISTEN_HOST, DEFAULT_RAFT_PORT)
}

fn default_inter_node_listen_addr() -> String {
    format!("{}:{}", DEFAULT_LISTEN_HOST, DEFAULT_INTER_NODE_PORT)
}

fn default_admin_listen_addr() -> String {
    format!("{}:{}", DEFAULT_RPC_LISTEN_HOST, DEFAULT_ADMIN_PORT)
}

fn default_rpc_listen_addr() -> String {
    format!("{}:{}", DEFAULT_RPC_LISTEN_HOST, DEFAULT_RPC_PORT)
}

fn merge_rpc_config(patch: KLogRpcConfigPatch) -> Result<KLogRpcConfig, String> {
    let append = merge_rpc_route_config("append", patch.append.unwrap_or_default())?;
    let query = merge_rpc_route_config("query", patch.query.unwrap_or_default())?;
    let jsonrpc = merge_rpc_route_config("jsonrpc", patch.jsonrpc.unwrap_or_default())?;
    Ok(KLogRpcConfig {
        append,
        query,
        jsonrpc,
    })
}

fn merge_cluster_network_config(
    patch: KLogClusterNetworkConfigPatch,
) -> Result<KLogClusterNetworkConfig, String> {
    let cfg = KLogClusterNetworkConfig {
        mode: patch.mode.unwrap_or(
            KClusterTransportMode::parse(DEFAULT_CLUSTER_NETWORK_MODE)
                .expect("DEFAULT_CLUSTER_NETWORK_MODE must be valid"),
        ),
        gateway_addr: patch
            .gateway_addr
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| DEFAULT_CLUSTER_GATEWAY_ADDR.to_string()),
        gateway_route_prefix: patch
            .gateway_route_prefix
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| DEFAULT_CLUSTER_GATEWAY_ROUTE_PREFIX.to_string()),
    };

    if cfg.gateway_addr.trim().is_empty() {
        let msg = format!("Invalid cluster_network.gateway_addr: gateway_addr must not be empty");
        error!("{}", msg);
        return Err(msg);
    }
    if cfg.gateway_route_prefix.trim().is_empty() {
        let msg =
            "Invalid cluster_network.gateway_route_prefix: gateway_route_prefix must not be empty"
                .to_string();
        error!("{}", msg);
        return Err(msg);
    }

    Ok(cfg)
}

fn merge_rpc_route_config(
    route_name: &str,
    patch: KLogRpcRouteConfigPatch,
) -> Result<KLogRpcRouteConfig, String> {
    let cfg = KLogRpcRouteConfig {
        timeout_ms: patch.timeout_ms.unwrap_or(DEFAULT_RPC_TIMEOUT_MS),
        body_limit_bytes: patch
            .body_limit_bytes
            .unwrap_or(DEFAULT_RPC_BODY_LIMIT_BYTES),
        concurrency: patch.concurrency.unwrap_or(DEFAULT_RPC_CONCURRENCY_LIMIT),
    };

    if cfg.timeout_ms == 0 {
        let msg = format!(
            "Invalid rpc.{} timeout_ms=0: timeout_ms must be greater than 0",
            route_name
        );
        error!("{}", msg);
        return Err(msg);
    }
    if cfg.body_limit_bytes == 0 {
        let msg = format!(
            "Invalid rpc.{} body_limit_bytes=0: body_limit_bytes must be greater than 0",
            route_name
        );
        error!("{}", msg);
        return Err(msg);
    }
    if cfg.concurrency == 0 {
        let msg = format!(
            "Invalid rpc.{} concurrency=0: concurrency must be greater than 0",
            route_name
        );
        error!("{}", msg);
        return Err(msg);
    }

    Ok(cfg)
}

fn merge_join_retry_config(patch: KLogJoinRetryConfigPatch) -> Result<KLogJoinRetryConfig, String> {
    let cfg = KLogJoinRetryConfig {
        strategy: patch
            .strategy
            .unwrap_or(KLogJoinRetryConfig::default().strategy),
        initial_interval_ms: patch
            .initial_interval_ms
            .unwrap_or(DEFAULT_JOIN_RETRY_INITIAL_INTERVAL_MS),
        max_interval_ms: patch
            .max_interval_ms
            .unwrap_or(DEFAULT_JOIN_RETRY_MAX_INTERVAL_MS),
        multiplier: patch.multiplier.unwrap_or(DEFAULT_JOIN_RETRY_MULTIPLIER),
        jitter_ratio: patch
            .jitter_ratio
            .unwrap_or(DEFAULT_JOIN_RETRY_JITTER_RATIO),
        max_attempts: patch
            .max_attempts
            .unwrap_or(DEFAULT_JOIN_RETRY_MAX_ATTEMPTS),
        request_timeout_ms: patch
            .request_timeout_ms
            .unwrap_or(DEFAULT_JOIN_RETRY_REQUEST_TIMEOUT_MS),
        shuffle_targets_each_round: patch
            .shuffle_targets_each_round
            .unwrap_or(DEFAULT_JOIN_RETRY_SHUFFLE_TARGETS),
        config_change_conflict_extra_backoff_ms: patch
            .config_change_conflict_extra_backoff_ms
            .unwrap_or(DEFAULT_JOIN_RETRY_CONFIG_CHANGE_CONFLICT_EXTRA_BACKOFF_MS),
    };

    if cfg.initial_interval_ms == 0 {
        let msg =
            "Invalid join.retry.initial_interval_ms=0: initial_interval_ms must be greater than 0"
                .to_string();
        error!("{}", msg);
        return Err(msg);
    }
    if cfg.max_interval_ms == 0 {
        let msg = "Invalid join.retry.max_interval_ms=0: max_interval_ms must be greater than 0"
            .to_string();
        error!("{}", msg);
        return Err(msg);
    }
    if cfg.max_interval_ms < cfg.initial_interval_ms {
        let msg = format!(
            "Invalid join.retry.max_interval_ms={}: max_interval_ms must be >= initial_interval_ms ({})",
            cfg.max_interval_ms, cfg.initial_interval_ms
        );
        error!("{}", msg);
        return Err(msg);
    }
    if cfg.multiplier < 1.0 {
        let msg = format!(
            "Invalid join.retry.multiplier={}: multiplier must be >= 1.0",
            cfg.multiplier
        );
        error!("{}", msg);
        return Err(msg);
    }
    if !(0.0..=1.0).contains(&cfg.jitter_ratio) {
        let msg = format!(
            "Invalid join.retry.jitter_ratio={}: jitter_ratio must be in [0.0, 1.0]",
            cfg.jitter_ratio
        );
        error!("{}", msg);
        return Err(msg);
    }
    if cfg.request_timeout_ms == 0 {
        let msg =
            "Invalid join.retry.request_timeout_ms=0: request_timeout_ms must be greater than 0"
                .to_string();
        error!("{}", msg);
        return Err(msg);
    }

    Ok(cfg)
}

fn merge_raft_config(patch: KLogRaftConfigPatch) -> Result<KLogRaftConfig, String> {
    let cfg = KLogRaftConfig {
        election_timeout_min_ms: patch
            .election_timeout_min_ms
            .unwrap_or(DEFAULT_RAFT_ELECTION_TIMEOUT_MIN_MS),
        election_timeout_max_ms: patch
            .election_timeout_max_ms
            .unwrap_or(DEFAULT_RAFT_ELECTION_TIMEOUT_MAX_MS),
        heartbeat_interval_ms: patch
            .heartbeat_interval_ms
            .unwrap_or(DEFAULT_RAFT_HEARTBEAT_INTERVAL_MS),
        install_snapshot_timeout_ms: patch
            .install_snapshot_timeout_ms
            .unwrap_or(DEFAULT_RAFT_INSTALL_SNAPSHOT_TIMEOUT_MS),
        max_payload_entries: patch
            .max_payload_entries
            .unwrap_or(DEFAULT_RAFT_MAX_PAYLOAD_ENTRIES),
        replication_lag_threshold: patch
            .replication_lag_threshold
            .unwrap_or(DEFAULT_RAFT_REPLICATION_LAG_THRESHOLD),
        snapshot_policy: patch
            .snapshot_policy
            .unwrap_or_else(|| DEFAULT_RAFT_SNAPSHOT_POLICY.to_string()),
        snapshot_max_chunk_size_bytes: patch
            .snapshot_max_chunk_size_bytes
            .unwrap_or(DEFAULT_RAFT_SNAPSHOT_MAX_CHUNK_SIZE_BYTES),
        max_in_snapshot_log_to_keep: patch
            .max_in_snapshot_log_to_keep
            .unwrap_or(DEFAULT_RAFT_MAX_IN_SNAPSHOT_LOG_TO_KEEP),
        purge_batch_size: patch
            .purge_batch_size
            .unwrap_or(DEFAULT_RAFT_PURGE_BATCH_SIZE),
    };

    if cfg.election_timeout_min_ms == 0 {
        let msg = "Invalid raft.election_timeout_min_ms=0: must be greater than 0".to_string();
        error!("{}", msg);
        return Err(msg);
    }
    if cfg.election_timeout_max_ms <= cfg.election_timeout_min_ms {
        let msg = format!(
            "Invalid raft election timeout range: min={} max={} (max must be greater than min)",
            cfg.election_timeout_min_ms, cfg.election_timeout_max_ms
        );
        error!("{}", msg);
        return Err(msg);
    }
    if cfg.heartbeat_interval_ms == 0 {
        let msg = "Invalid raft.heartbeat_interval_ms=0: must be greater than 0".to_string();
        error!("{}", msg);
        return Err(msg);
    }
    if cfg.election_timeout_min_ms <= cfg.heartbeat_interval_ms {
        let msg = format!(
            "Invalid raft timing: election_timeout_min_ms={} must be greater than heartbeat_interval_ms={}",
            cfg.election_timeout_min_ms, cfg.heartbeat_interval_ms
        );
        error!("{}", msg);
        return Err(msg);
    }
    if cfg.install_snapshot_timeout_ms == 0 {
        let msg = "Invalid raft.install_snapshot_timeout_ms=0: must be greater than 0".to_string();
        error!("{}", msg);
        return Err(msg);
    }
    if cfg.max_payload_entries == 0 {
        let msg = "Invalid raft.max_payload_entries=0: must be greater than 0".to_string();
        error!("{}", msg);
        return Err(msg);
    }
    if cfg.replication_lag_threshold == 0 {
        let msg = "Invalid raft.replication_lag_threshold=0: must be greater than 0".to_string();
        error!("{}", msg);
        return Err(msg);
    }
    if cfg.snapshot_max_chunk_size_bytes == 0 {
        let msg =
            "Invalid raft.snapshot_max_chunk_size_bytes=0: must be greater than 0".to_string();
        error!("{}", msg);
        return Err(msg);
    }
    if cfg.max_in_snapshot_log_to_keep == 0 {
        let msg = "Invalid raft.max_in_snapshot_log_to_keep=0: must be greater than 0".to_string();
        error!("{}", msg);
        return Err(msg);
    }
    if cfg.purge_batch_size == 0 {
        let msg = "Invalid raft.purge_batch_size=0: must be greater than 0".to_string();
        error!("{}", msg);
        return Err(msg);
    }
    parse_snapshot_policy(&cfg.snapshot_policy)?;

    Ok(cfg)
}

fn parse_snapshot_policy(input: &str) -> Result<SnapshotPolicy, String> {
    let v = input.trim();
    if v.eq_ignore_ascii_case("never") {
        return Ok(SnapshotPolicy::Never);
    }

    let Some((kind, value)) = v.split_once(':') else {
        let msg = format!(
            "Invalid raft.snapshot_policy='{}': expected 'never' or 'since_last:<u64>'",
            input
        );
        error!("{}", msg);
        return Err(msg);
    };

    if !kind.eq_ignore_ascii_case("since_last") {
        let msg = format!(
            "Invalid raft.snapshot_policy='{}': expected prefix 'since_last'",
            input
        );
        error!("{}", msg);
        return Err(msg);
    }

    let n = value.trim().parse::<u64>().map_err(|e| {
        let msg = format!(
            "Invalid raft.snapshot_policy='{}': invalid since_last value: {}",
            input, e
        );
        error!("{}", msg);
        msg
    })?;
    Ok(SnapshotPolicy::LogsSinceLast(n))
}

fn parse_port_from_addr(addr: &str) -> Option<u16> {
    let (_, port_str) = addr.rsplit_once(':')?;
    port_str.parse::<u16>().ok()
}

fn parse_env_string(key: &str) -> Result<Option<String>, String> {
    match std::env::var(key) {
        Ok(v) => {
            let trimmed = v.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(trimmed.to_string()))
            }
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{} contains invalid unicode", key)),
    }
}

fn parse_env_pathbuf(key: &str) -> Result<Option<PathBuf>, String> {
    let value = parse_env_string(key)?;
    Ok(value.map(PathBuf::from))
}

fn parse_env_string_list(key: &str) -> Result<Option<Vec<String>>, String> {
    match std::env::var(key) {
        Ok(v) => {
            let items = v
                .split(',')
                .map(|x| x.trim())
                .filter(|x| !x.is_empty())
                .map(|x| x.to_string())
                .collect::<Vec<_>>();
            if items.is_empty() {
                Ok(None)
            } else {
                Ok(Some(items))
            }
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{} contains invalid unicode", key)),
    }
}

fn parse_env_u64(key: &str) -> Result<Option<u64>, String> {
    match parse_env_string(key)? {
        Some(v) => v
            .parse::<u64>()
            .map(Some)
            .map_err(|e| format!("Invalid {}='{}': {}", key, v, e)),
        None => Ok(None),
    }
}

fn parse_env_usize(key: &str) -> Result<Option<usize>, String> {
    match parse_env_string(key)? {
        Some(v) => v
            .parse::<usize>()
            .map(Some)
            .map_err(|e| format!("Invalid {}='{}': {}", key, v, e)),
        None => Ok(None),
    }
}

fn parse_env_u16(key: &str) -> Result<Option<u16>, String> {
    match parse_env_string(key)? {
        Some(v) => v
            .parse::<u16>()
            .map(Some)
            .map_err(|e| format!("Invalid {}='{}': {}", key, v, e)),
        None => Ok(None),
    }
}

fn parse_env_u32(key: &str) -> Result<Option<u32>, String> {
    match parse_env_string(key)? {
        Some(v) => v
            .parse::<u32>()
            .map(Some)
            .map_err(|e| format!("Invalid {}='{}': {}", key, v, e)),
        None => Ok(None),
    }
}

fn parse_env_f64(key: &str) -> Result<Option<f64>, String> {
    match parse_env_string(key)? {
        Some(v) => v
            .parse::<f64>()
            .map(Some)
            .map_err(|e| format!("Invalid {}='{}': {}", key, v, e)),
        None => Ok(None),
    }
}

fn parse_env_bool(key: &str) -> Result<Option<bool>, String> {
    match parse_env_string(key)? {
        Some(v) => {
            let s = v.to_ascii_lowercase();
            let parsed = match s.as_str() {
                "1" | "true" | "yes" | "y" | "on" => true,
                "0" | "false" | "no" | "n" | "off" => false,
                _ => {
                    return Err(format!("Invalid {}='{}': expected true/false", key, v));
                }
            };
            Ok(Some(parsed))
        }
        None => Ok(None),
    }
}

fn parse_env_join_target_role(key: &str) -> Result<Option<KLogJoinTargetRole>, String> {
    match parse_env_string(key)? {
        Some(v) => KLogJoinTargetRole::parse(&v)
            .map(Some)
            .map_err(|e| format!("Invalid {}='{}': {}", key, v, e)),
        None => Ok(None),
    }
}

fn parse_env_join_retry_strategy(key: &str) -> Result<Option<KLogJoinRetryStrategy>, String> {
    match parse_env_string(key)? {
        Some(v) => KLogJoinRetryStrategy::parse(&v)
            .map(Some)
            .map_err(|e| format!("Invalid {}='{}': {}", key, v, e)),
        None => Ok(None),
    }
}

fn parse_env_cluster_transport_mode(key: &str) -> Result<Option<KClusterTransportMode>, String> {
    match parse_env_string(key)? {
        Some(v) => KClusterTransportMode::parse(&v)
            .map(Some)
            .map_err(|e| format!("Invalid {}='{}': {}", key, v, e)),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_test_file(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!(
            "buckyos_klog_daemon_cfg_{}_{}_{}.toml",
            std::process::id(),
            nanos,
            name
        ))
    }

    #[test]
    fn test_from_file_full_values() {
        let file = unique_test_file("full");
        let content = r#"
node_id = 9

[network]
listen_addr = "0.0.0.0:22001"
inter_node_listen_addr = "0.0.0.0:22002"
admin_listen_addr = "127.0.0.1:22003"
enable_rpc_server = false
rpc_listen_addr = "0.0.0.0:22101"
advertise_addr = "10.2.3.4"
advertise_node_name = "node-full"
advertise_port = 22001
advertise_inter_port = 22002
advertise_admin_port = 22003
rpc_advertise_port = 22101

[storage]
data_dir = "/tmp/klog_cfg_test_full"
state_store_sync_write = false

[cluster]
name = "cluster_a"
id = "cluster_a_id"
auto_bootstrap = false

[cluster_network]
mode = "direct"
gateway_addr = "127.0.0.1:4180"
gateway_route_prefix = "/.cluster/klog-test"

[join]
targets = ["127.0.0.1:21001", "127.0.0.1:21002"]
blocking = true
target_role = "learner"

[join.retry]
strategy = "fixed"
initial_interval_ms = 1500
max_interval_ms = 6000
multiplier = 2.0
jitter_ratio = 0.1
max_attempts = 9
request_timeout_ms = 1800
shuffle_targets_each_round = false
config_change_conflict_extra_backoff_ms = 900

[raft]
election_timeout_min_ms = 300
election_timeout_max_ms = 900
heartbeat_interval_ms = 120
install_snapshot_timeout_ms = 2300
max_payload_entries = 512
replication_lag_threshold = 15000
snapshot_policy = "since_last:8000"
snapshot_max_chunk_size_bytes = 4194304
max_in_snapshot_log_to_keep = 2000
purge_batch_size = 64

[admin]
local_only = false

[rpc.append]
timeout_ms = 3100
body_limit_bytes = 131072
concurrency = 64

[rpc.query]
timeout_ms = 3200
body_limit_bytes = 262144
concurrency = 96

[rpc.jsonrpc]
timeout_ms = 3300
body_limit_bytes = 1048576
concurrency = 128
"#;
        std::fs::write(&file, content).expect("write file");

        let cfg = KLogRuntimeConfig::from_file(&file).expect("parse file");
        assert_eq!(cfg.node_id, 9);
        assert_eq!(cfg.listen_addr, "0.0.0.0:22001");
        assert_eq!(cfg.inter_node_listen_addr, "0.0.0.0:22002");
        assert_eq!(cfg.admin_listen_addr, "127.0.0.1:22003");
        assert!(!cfg.enable_rpc_server);
        assert_eq!(cfg.rpc_listen_addr, "0.0.0.0:22101");
        assert_eq!(cfg.advertise_addr, "10.2.3.4");
        assert_eq!(cfg.advertise_node_name.as_deref(), Some("node-full"));
        assert_eq!(cfg.advertise_port, 22001);
        assert_eq!(cfg.advertise_inter_port, 22002);
        assert_eq!(cfg.advertise_admin_port, 22003);
        assert_eq!(cfg.rpc_advertise_port, 22101);
        assert_eq!(cfg.data_dir, PathBuf::from("/tmp/klog_cfg_test_full"));
        assert_eq!(cfg.cluster_name, "cluster_a");
        assert_eq!(cfg.cluster_id, "cluster_a_id");
        assert!(!cfg.auto_bootstrap);
        assert_eq!(cfg.cluster_network.mode, KClusterTransportMode::Direct);
        assert_eq!(cfg.cluster_network.gateway_addr, "127.0.0.1:4180");
        assert_eq!(
            cfg.cluster_network.gateway_route_prefix,
            "/.cluster/klog-test"
        );
        assert!(!cfg.state_store_sync_write);
        assert_eq!(
            cfg.join_targets,
            vec!["127.0.0.1:21001".to_string(), "127.0.0.1:21002".to_string()]
        );
        assert!(cfg.join_blocking);
        assert_eq!(cfg.join_target_role, KLogJoinTargetRole::Learner);
        assert_eq!(cfg.join_retry.strategy, KLogJoinRetryStrategy::Fixed);
        assert_eq!(cfg.join_retry.initial_interval_ms, 1500);
        assert_eq!(cfg.join_retry.max_interval_ms, 6000);
        assert_eq!(cfg.join_retry.multiplier, 2.0);
        assert_eq!(cfg.join_retry.jitter_ratio, 0.1);
        assert_eq!(cfg.join_retry.max_attempts, 9);
        assert_eq!(cfg.join_retry.request_timeout_ms, 1800);
        assert!(!cfg.join_retry.shuffle_targets_each_round);
        assert_eq!(cfg.join_retry.config_change_conflict_extra_backoff_ms, 900);
        assert_eq!(cfg.raft.election_timeout_min_ms, 300);
        assert_eq!(cfg.raft.election_timeout_max_ms, 900);
        assert_eq!(cfg.raft.heartbeat_interval_ms, 120);
        assert_eq!(cfg.raft.install_snapshot_timeout_ms, 2300);
        assert_eq!(cfg.raft.max_payload_entries, 512);
        assert_eq!(cfg.raft.replication_lag_threshold, 15000);
        assert_eq!(cfg.raft.snapshot_policy, "since_last:8000");
        assert_eq!(cfg.raft.snapshot_max_chunk_size_bytes, 4194304);
        assert_eq!(cfg.raft.max_in_snapshot_log_to_keep, 2000);
        assert_eq!(cfg.raft.purge_batch_size, 64);
        assert!(!cfg.admin_local_only);
        assert_eq!(cfg.rpc.append.timeout_ms, 3100);
        assert_eq!(cfg.rpc.append.body_limit_bytes, 131072);
        assert_eq!(cfg.rpc.append.concurrency, 64);
        assert_eq!(cfg.rpc.query.timeout_ms, 3200);
        assert_eq!(cfg.rpc.query.body_limit_bytes, 262144);
        assert_eq!(cfg.rpc.query.concurrency, 96);
        assert_eq!(cfg.rpc.jsonrpc.timeout_ms, 3300);
        assert_eq!(cfg.rpc.jsonrpc.body_limit_bytes, 1048576);
        assert_eq!(cfg.rpc.jsonrpc.concurrency, 128);

        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn test_from_file_legacy_flat_values_rejected() {
        let file = unique_test_file("legacy_flat_rejected");
        let content = r#"
node_id = 6
listen_addr = "0.0.0.0:26001"
advertise_addr = "10.0.0.6"
advertise_port = 26001
data_dir = "/tmp/klog_cfg_test_legacy"
cluster_name = "legacy_cluster"
auto_bootstrap = false
state_store_sync_write = false
join_targets = ["127.0.0.1:21001"]
join_retry_max_attempts = 5
join_blocking = true
join_target_role = "learner"
"#;
        std::fs::write(&file, content).expect("write file");

        let err = KLogRuntimeConfig::from_file(&file).expect_err("legacy flat fields must fail");
        assert!(err.contains("unknown field"));

        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn test_from_file_partial_values() {
        let file = unique_test_file("partial");
        let content = r#"
node_id = 7

[network]
advertise_addr = "192.168.2.7"

[cluster]
name = "cluster_partial"
id = "cluster_partial_id"
"#;
        std::fs::write(&file, content).expect("write file");

        let cfg = KLogRuntimeConfig::from_file(&file).expect("parse file");
        assert_eq!(cfg.node_id, 7);
        assert_eq!(cfg.listen_addr, default_listen_addr());
        assert_eq!(cfg.inter_node_listen_addr, default_inter_node_listen_addr());
        assert_eq!(cfg.admin_listen_addr, default_admin_listen_addr());
        assert_eq!(cfg.enable_rpc_server, DEFAULT_ENABLE_RPC_SERVER);
        assert_eq!(cfg.rpc_listen_addr, default_rpc_listen_addr());
        assert_eq!(cfg.advertise_addr, "192.168.2.7");
        assert_eq!(cfg.advertise_node_name, None);
        assert_eq!(cfg.advertise_port, DEFAULT_RAFT_PORT);
        assert_eq!(cfg.advertise_inter_port, DEFAULT_INTER_NODE_PORT);
        assert_eq!(cfg.advertise_admin_port, DEFAULT_ADMIN_PORT);
        assert_eq!(cfg.rpc_advertise_port, DEFAULT_RPC_PORT);
        assert_eq!(cfg.data_dir, default_data_dir());
        assert_eq!(cfg.cluster_name, "cluster_partial");
        assert_eq!(cfg.cluster_id, "cluster_partial_id");
        assert_eq!(cfg.auto_bootstrap, DEFAULT_AUTO_BOOTSTRAP);
        assert_eq!(cfg.cluster_network.mode, KClusterTransportMode::Direct);
        assert_eq!(
            cfg.cluster_network.gateway_addr,
            DEFAULT_CLUSTER_GATEWAY_ADDR
        );
        assert_eq!(
            cfg.cluster_network.gateway_route_prefix,
            DEFAULT_CLUSTER_GATEWAY_ROUTE_PREFIX
        );
        assert_eq!(cfg.state_store_sync_write, DEFAULT_STATE_STORE_SYNC_WRITE);
        assert!(cfg.join_targets.is_empty());
        assert_eq!(cfg.join_blocking, DEFAULT_JOIN_BLOCKING);
        assert_eq!(cfg.join_target_role, DEFAULT_JOIN_TARGET_ROLE);
        assert_eq!(
            cfg.join_retry.strategy.as_str(),
            DEFAULT_JOIN_RETRY_STRATEGY
        );
        assert_eq!(
            cfg.join_retry.initial_interval_ms,
            DEFAULT_JOIN_RETRY_INITIAL_INTERVAL_MS
        );
        assert_eq!(
            cfg.join_retry.max_interval_ms,
            DEFAULT_JOIN_RETRY_MAX_INTERVAL_MS
        );
        assert_eq!(cfg.join_retry.multiplier, DEFAULT_JOIN_RETRY_MULTIPLIER);
        assert_eq!(cfg.join_retry.jitter_ratio, DEFAULT_JOIN_RETRY_JITTER_RATIO);
        assert_eq!(cfg.join_retry.max_attempts, DEFAULT_JOIN_RETRY_MAX_ATTEMPTS);
        assert_eq!(
            cfg.join_retry.request_timeout_ms,
            DEFAULT_JOIN_RETRY_REQUEST_TIMEOUT_MS
        );
        assert_eq!(
            cfg.join_retry.shuffle_targets_each_round,
            DEFAULT_JOIN_RETRY_SHUFFLE_TARGETS
        );
        assert_eq!(
            cfg.join_retry.config_change_conflict_extra_backoff_ms,
            DEFAULT_JOIN_RETRY_CONFIG_CHANGE_CONFLICT_EXTRA_BACKOFF_MS
        );
        assert_eq!(
            cfg.raft.election_timeout_min_ms,
            DEFAULT_RAFT_ELECTION_TIMEOUT_MIN_MS
        );
        assert_eq!(
            cfg.raft.election_timeout_max_ms,
            DEFAULT_RAFT_ELECTION_TIMEOUT_MAX_MS
        );
        assert_eq!(
            cfg.raft.heartbeat_interval_ms,
            DEFAULT_RAFT_HEARTBEAT_INTERVAL_MS
        );
        assert_eq!(
            cfg.raft.install_snapshot_timeout_ms,
            DEFAULT_RAFT_INSTALL_SNAPSHOT_TIMEOUT_MS
        );
        assert_eq!(
            cfg.raft.max_payload_entries,
            DEFAULT_RAFT_MAX_PAYLOAD_ENTRIES
        );
        assert_eq!(
            cfg.raft.replication_lag_threshold,
            DEFAULT_RAFT_REPLICATION_LAG_THRESHOLD
        );
        assert_eq!(cfg.raft.snapshot_policy, DEFAULT_RAFT_SNAPSHOT_POLICY);
        assert_eq!(
            cfg.raft.snapshot_max_chunk_size_bytes,
            DEFAULT_RAFT_SNAPSHOT_MAX_CHUNK_SIZE_BYTES
        );
        assert_eq!(
            cfg.raft.max_in_snapshot_log_to_keep,
            DEFAULT_RAFT_MAX_IN_SNAPSHOT_LOG_TO_KEEP
        );
        assert_eq!(cfg.raft.purge_batch_size, DEFAULT_RAFT_PURGE_BATCH_SIZE);
        assert_eq!(cfg.admin_local_only, DEFAULT_ADMIN_LOCAL_ONLY);
        assert_eq!(cfg.rpc.append.timeout_ms, DEFAULT_RPC_TIMEOUT_MS);
        assert_eq!(
            cfg.rpc.append.body_limit_bytes,
            DEFAULT_RPC_BODY_LIMIT_BYTES
        );
        assert_eq!(cfg.rpc.append.concurrency, DEFAULT_RPC_CONCURRENCY_LIMIT);
        assert_eq!(cfg.rpc.query.timeout_ms, DEFAULT_RPC_TIMEOUT_MS);
        assert_eq!(cfg.rpc.query.body_limit_bytes, DEFAULT_RPC_BODY_LIMIT_BYTES);
        assert_eq!(cfg.rpc.query.concurrency, DEFAULT_RPC_CONCURRENCY_LIMIT);
        assert_eq!(cfg.rpc.jsonrpc.timeout_ms, DEFAULT_RPC_TIMEOUT_MS);
        assert_eq!(
            cfg.rpc.jsonrpc.body_limit_bytes,
            DEFAULT_RPC_BODY_LIMIT_BYTES
        );
        assert_eq!(cfg.rpc.jsonrpc.concurrency, DEFAULT_RPC_CONCURRENCY_LIMIT);

        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn test_from_file_missing_cluster_name_rejected() {
        let file = unique_test_file("missing_cluster_name");
        let content = r#"
node_id = 7

[cluster]
id = "cluster_id_only"
"#;
        std::fs::write(&file, content).expect("write file");

        let err = KLogRuntimeConfig::from_file(&file).expect_err("missing cluster.name must fail");
        assert!(err.contains("Missing required field: cluster.name"));

        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn test_from_file_missing_cluster_id_rejected() {
        let file = unique_test_file("missing_cluster_id");
        let content = r#"
node_id = 7

[cluster]
name = "cluster_name_only"
"#;
        std::fs::write(&file, content).expect("write file");

        let err = KLogRuntimeConfig::from_file(&file).expect_err("missing cluster.id must fail");
        assert!(err.contains("Missing required field: cluster.id"));

        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn test_from_file_missing_node_id_rejected() {
        let file = unique_test_file("missing_node_id");
        let content = r#"
[network]
listen_addr = "0.0.0.0:22001"
"#;
        std::fs::write(&file, content).expect("write file");

        let err = KLogRuntimeConfig::from_file(&file).expect_err("missing node_id must fail");
        assert!(err.contains("Missing required field: node_id"));

        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn test_from_file_auto_bootstrap_conflicts_with_join_targets() {
        let file = unique_test_file("bootstrap_join_conflict");
        let content = r#"
node_id = 9

[cluster]
name = "cluster_x"
id = "cluster_x_id"
auto_bootstrap = true

[join]
targets = ["127.0.0.1:21001"]
"#;
        std::fs::write(&file, content).expect("write file");

        let err =
            KLogRuntimeConfig::from_file(&file).expect_err("bootstrap+join.targets must fail");
        assert!(err.contains("auto_bootstrap=true"));

        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn test_from_file_cluster_network_gateway_proxy_accepted() {
        let file = unique_test_file("cluster_network_gateway_proxy");
        let content = r#"
node_id = 9

[network]
advertise_node_name = "node-gateway"

[cluster]
name = "cluster_transport"
id = "cluster_transport_id"

[cluster_network]
mode = "gateway_proxy"
gateway_addr = "127.0.0.1:4180"
gateway_route_prefix = "/.cluster/klog-proxy"
"#;
        std::fs::write(&file, content).expect("write file");

        let cfg = KLogRuntimeConfig::from_file(&file)
            .expect("cluster_network.mode=gateway_proxy should be accepted");
        assert_eq!(
            cfg.cluster_network.mode,
            KClusterTransportMode::GatewayProxy
        );
        assert_eq!(cfg.cluster_network.gateway_addr, "127.0.0.1:4180");
        assert_eq!(
            cfg.cluster_network.gateway_route_prefix,
            "/.cluster/klog-proxy"
        );
        assert_eq!(cfg.advertise_node_name.as_deref(), Some("node-gateway"));

        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn test_from_file_cluster_network_non_direct_requires_node_name() {
        let file = unique_test_file("cluster_network_non_direct_requires_node_name");
        let content = r#"
node_id = 9

[cluster]
name = "cluster_transport"
id = "cluster_transport_id"

[cluster_network]
mode = "hybrid"
"#;
        std::fs::write(&file, content).expect("write file");

        let err = KLogRuntimeConfig::from_file(&file)
            .expect_err("cluster_network.mode=hybrid without advertise_node_name must fail");
        assert!(err.contains("network.advertise_node_name"));

        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn test_from_file_admin_listener_must_be_distinct() {
        let file = unique_test_file("admin_listener_conflict");
        let content = r#"
node_id = 7

[network]
listen_addr = "0.0.0.0:21001"
inter_node_listen_addr = "0.0.0.0:21002"
admin_listen_addr = "0.0.0.0:21001"

[cluster]
name = "cluster_admin_conflict"
id = "cluster_admin_conflict_id"
"#;
        std::fs::write(&file, content).expect("write file");

        let err = KLogRuntimeConfig::from_file(&file).expect_err("admin listener must be distinct");
        assert!(err.contains("admin_listen_addr"));

        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn test_from_buckyos_patch() {
        let patch = BuckyosKlogConfig {
            network: Some(KLogNetworkConfigPatch {
                listen_addr: Some("0.0.0.0:23001".to_string()),
                inter_node_listen_addr: Some("0.0.0.0:23002".to_string()),
                admin_listen_addr: Some("127.0.0.1:23003".to_string()),
                enable_rpc_server: Some(false),
                rpc_listen_addr: Some("0.0.0.0:23101".to_string()),
                advertise_addr: Some("172.20.0.3".to_string()),
                advertise_node_name: Some("node-buckyos".to_string()),
                advertise_port: Some(23001),
                advertise_inter_port: Some(23002),
                advertise_admin_port: Some(23003),
                rpc_advertise_port: Some(23101),
            }),
            storage: Some(KLogStorageConfigPatch {
                data_dir: None,
                state_store_sync_write: Some(false),
            }),
            cluster: Some(KLogClusterConfigPatch {
                name: Some("bk".to_string()),
                id: Some("bk-id".to_string()),
                auto_bootstrap: Some(false),
            }),
            join: Some(KLogJoinConfigPatch {
                targets: Some(vec!["10.0.0.1:21001".to_string()]),
                blocking: Some(true),
                target_role: Some(KLogJoinTargetRole::Learner),
                retry: Some(KLogJoinRetryConfigPatch {
                    strategy: Some(KLogJoinRetryStrategy::Fixed),
                    initial_interval_ms: Some(6000),
                    max_interval_ms: Some(20000),
                    multiplier: Some(1.0),
                    jitter_ratio: Some(0.0),
                    max_attempts: Some(3),
                    request_timeout_ms: Some(2200),
                    shuffle_targets_each_round: Some(false),
                    config_change_conflict_extra_backoff_ms: Some(700),
                }),
            }),
            raft: Some(KLogRaftConfigPatch {
                election_timeout_min_ms: Some(400),
                election_timeout_max_ms: Some(1200),
                heartbeat_interval_ms: Some(150),
                install_snapshot_timeout_ms: Some(2800),
                max_payload_entries: Some(256),
                replication_lag_threshold: Some(12000),
                snapshot_policy: Some("since_last:6000".to_string()),
                snapshot_max_chunk_size_bytes: Some(2 * 1024 * 1024),
                max_in_snapshot_log_to_keep: Some(1500),
                purge_batch_size: Some(8),
            }),
            cluster_network: Some(KLogClusterNetworkConfigPatch {
                mode: Some(KClusterTransportMode::Direct),
                gateway_addr: Some("127.0.0.1:5180".to_string()),
                gateway_route_prefix: Some("/.cluster/klog-buckyos".to_string()),
            }),
            admin: Some(KLogAdminConfigPatch {
                local_only: Some(false),
            }),
            node_id: Some(3),
            ..Default::default()
        };

        let (cfg, source) =
            KLogRuntimeConfig::load_from_buckyos(&patch).expect("build from buckyos patch");
        assert_eq!(source, KLogRuntimeConfigSource::Buckyos);
        assert_eq!(cfg.node_id, 3);
        assert_eq!(cfg.listen_addr, "0.0.0.0:23001");
        assert_eq!(cfg.inter_node_listen_addr, "0.0.0.0:23002");
        assert_eq!(cfg.admin_listen_addr, "127.0.0.1:23003");
        assert!(!cfg.enable_rpc_server);
        assert_eq!(cfg.rpc_listen_addr, "0.0.0.0:23101");
        assert_eq!(cfg.advertise_addr, "172.20.0.3");
        assert_eq!(cfg.advertise_node_name.as_deref(), Some("node-buckyos"));
        assert_eq!(cfg.advertise_port, 23001);
        assert_eq!(cfg.advertise_inter_port, 23002);
        assert_eq!(cfg.advertise_admin_port, 23003);
        assert_eq!(cfg.rpc_advertise_port, 23101);
        assert_eq!(cfg.data_dir, default_data_dir());
        assert_eq!(cfg.cluster_name, "bk");
        assert_eq!(cfg.cluster_id, "bk-id");
        assert!(!cfg.auto_bootstrap);
        assert_eq!(cfg.cluster_network.mode, KClusterTransportMode::Direct);
        assert_eq!(cfg.cluster_network.gateway_addr, "127.0.0.1:5180");
        assert_eq!(
            cfg.cluster_network.gateway_route_prefix,
            "/.cluster/klog-buckyos"
        );
        assert!(!cfg.state_store_sync_write);
        assert_eq!(cfg.join_targets, vec!["10.0.0.1:21001".to_string()]);
        assert!(cfg.join_blocking);
        assert_eq!(cfg.join_target_role, KLogJoinTargetRole::Learner);
        assert_eq!(cfg.join_retry.strategy, KLogJoinRetryStrategy::Fixed);
        assert_eq!(cfg.join_retry.initial_interval_ms, 6000);
        assert_eq!(cfg.join_retry.max_interval_ms, 20000);
        assert_eq!(cfg.join_retry.multiplier, 1.0);
        assert_eq!(cfg.join_retry.jitter_ratio, 0.0);
        assert_eq!(cfg.join_retry.max_attempts, 3);
        assert_eq!(cfg.join_retry.request_timeout_ms, 2200);
        assert!(!cfg.join_retry.shuffle_targets_each_round);
        assert_eq!(cfg.join_retry.config_change_conflict_extra_backoff_ms, 700);
        assert_eq!(cfg.raft.election_timeout_min_ms, 400);
        assert_eq!(cfg.raft.election_timeout_max_ms, 1200);
        assert_eq!(cfg.raft.heartbeat_interval_ms, 150);
        assert_eq!(cfg.raft.install_snapshot_timeout_ms, 2800);
        assert_eq!(cfg.raft.max_payload_entries, 256);
        assert_eq!(cfg.raft.replication_lag_threshold, 12000);
        assert_eq!(cfg.raft.snapshot_policy, "since_last:6000");
        assert_eq!(cfg.raft.snapshot_max_chunk_size_bytes, 2 * 1024 * 1024);
        assert_eq!(cfg.raft.max_in_snapshot_log_to_keep, 1500);
        assert_eq!(cfg.raft.purge_batch_size, 8);
        assert!(!cfg.admin_local_only);
        assert_eq!(cfg.rpc.append.timeout_ms, DEFAULT_RPC_TIMEOUT_MS);
        assert_eq!(cfg.rpc.query.timeout_ms, DEFAULT_RPC_TIMEOUT_MS);
        assert_eq!(cfg.rpc.jsonrpc.timeout_ms, DEFAULT_RPC_TIMEOUT_MS);
    }

    #[test]
    fn test_from_file_rpc_invalid_zero_rejected() {
        let file = unique_test_file("rpc_invalid_zero");
        let content = r#"
node_id = 7

[cluster]
name = "cluster_rpc_invalid"
id = "cluster_rpc_invalid_id"

[rpc.append]
concurrency = 0
"#;
        std::fs::write(&file, content).expect("write file");

        let err =
            KLogRuntimeConfig::from_file(&file).expect_err("rpc.append.concurrency=0 must fail");
        assert!(err.contains("rpc.append concurrency=0"));

        let _ = std::fs::remove_file(&file);
    }
}
