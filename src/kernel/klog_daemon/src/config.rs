use crate::constants::{
    DEFAULT_ADMIN_LOCAL_ONLY, DEFAULT_ADMIN_PORT, DEFAULT_ADVERTISE_ADDR, DEFAULT_AUTO_BOOTSTRAP,
    DEFAULT_ENABLE_RPC_SERVER, DEFAULT_INTER_NODE_PORT, DEFAULT_JOIN_BLOCKING,
    DEFAULT_JOIN_MAX_ATTEMPTS, DEFAULT_JOIN_RETRY_INTERVAL_MS, DEFAULT_LISTEN_HOST,
    DEFAULT_RAFT_PORT, DEFAULT_RPC_BODY_LIMIT_BYTES, DEFAULT_RPC_CONCURRENCY_LIMIT,
    DEFAULT_RPC_LISTEN_HOST, DEFAULT_RPC_PORT, DEFAULT_RPC_TIMEOUT_MS,
    DEFAULT_STATE_STORE_SYNC_WRITE, ENV_ADMIN_ADVERTISE_PORT, ENV_ADMIN_LISTEN_ADDR,
    ENV_ADMIN_LOCAL_ONLY, ENV_ADVERTISE_ADDR, ENV_ADVERTISE_INTER_PORT, ENV_ADVERTISE_PORT,
    ENV_AUTO_BOOTSTRAP, ENV_CLUSTER_ID, ENV_CLUSTER_NAME, ENV_CONFIG_FILE, ENV_DATA_DIR,
    ENV_ENABLE_RPC_SERVER, ENV_INTER_NODE_LISTEN_ADDR, ENV_JOIN_BLOCKING, ENV_JOIN_MAX_ATTEMPTS,
    ENV_JOIN_RETRY_INTERVAL_MS, ENV_JOIN_TARGET_ROLE, ENV_JOIN_TARGETS, ENV_LISTEN_ADDR,
    ENV_NODE_ID, ENV_RPC_ADVERTISE_PORT, ENV_RPC_APPEND_BODY_LIMIT_BYTES,
    ENV_RPC_APPEND_CONCURRENCY, ENV_RPC_APPEND_TIMEOUT_MS, ENV_RPC_JSONRPC_BODY_LIMIT_BYTES,
    ENV_RPC_JSONRPC_CONCURRENCY, ENV_RPC_JSONRPC_TIMEOUT_MS, ENV_RPC_LISTEN_ADDR,
    ENV_RPC_QUERY_BODY_LIMIT_BYTES, ENV_RPC_QUERY_CONCURRENCY, ENV_RPC_QUERY_TIMEOUT_MS,
    ENV_STATE_STORE_SYNC_WRITE, KLOG_SERVICE_NAME,
};
use buckyos_kit::get_buckyos_service_data_dir;
use klog::KNodeId;
use klog::rpc::{KRpcRoutePolicy, KRpcServerPolicy};
use log::error;
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

    /// Route policy for `/klog/rpc`.
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

    /// Retry interval for auto join loop, in milliseconds.
    pub join_retry_interval_ms: u64,

    /// Max retries for auto join loop, 0 means retry forever.
    pub join_max_attempts: u32,

    /// Whether add-learner uses blocking mode during auto join.
    pub join_blocking: bool,

    /// Target role after joining cluster: learner or voter.
    pub join_target_role: KLogJoinTargetRole,

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
pub struct KLogJoinConfigPatch {
    /// Optional override for join seed admin targets.
    pub targets: Option<Vec<String>>,

    /// Optional override for join retry interval milliseconds.
    pub retry_interval_ms: Option<u64>,

    /// Optional override for join max attempts, 0 means forever.
    pub max_attempts: Option<u32>,

    /// Optional override for add-learner blocking mode.
    pub blocking: Option<bool>,

    /// Optional override for target role after join.
    pub target_role: Option<KLogJoinTargetRole>,
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
            join: Some(KLogJoinConfigPatch {
                targets: parse_env_string_list(ENV_JOIN_TARGETS)?,
                retry_interval_ms: parse_env_u64(ENV_JOIN_RETRY_INTERVAL_MS)?,
                max_attempts: parse_env_u32(ENV_JOIN_MAX_ATTEMPTS)?,
                blocking: parse_env_bool(ENV_JOIN_BLOCKING)?,
                target_role: parse_env_join_target_role(ENV_JOIN_TARGET_ROLE)?,
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
            admin,
            rpc,
            node_id,
        } = patch;

        let network = network.unwrap_or_default();
        let storage = storage.unwrap_or_default();
        let cluster = cluster.unwrap_or_default();
        let join = join.unwrap_or_default();
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
            data_dir: storage.data_dir.unwrap_or(default_data_dir),
            cluster_name,
            cluster_id,
            auto_bootstrap,
            state_store_sync_write: storage
                .state_store_sync_write
                .unwrap_or(DEFAULT_STATE_STORE_SYNC_WRITE),
            join_targets,
            join_retry_interval_ms: join
                .retry_interval_ms
                .unwrap_or(DEFAULT_JOIN_RETRY_INTERVAL_MS),
            join_max_attempts: join.max_attempts.unwrap_or(DEFAULT_JOIN_MAX_ATTEMPTS),
            join_blocking: join.blocking.unwrap_or(DEFAULT_JOIN_BLOCKING),
            join_target_role: join.target_role.unwrap_or(DEFAULT_JOIN_TARGET_ROLE),
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

[join]
targets = ["127.0.0.1:21001", "127.0.0.1:21002"]
retry_interval_ms = 1500
max_attempts = 9
blocking = true
target_role = "learner"

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
        assert_eq!(cfg.advertise_port, 22001);
        assert_eq!(cfg.advertise_inter_port, 22002);
        assert_eq!(cfg.advertise_admin_port, 22003);
        assert_eq!(cfg.rpc_advertise_port, 22101);
        assert_eq!(cfg.data_dir, PathBuf::from("/tmp/klog_cfg_test_full"));
        assert_eq!(cfg.cluster_name, "cluster_a");
        assert_eq!(cfg.cluster_id, "cluster_a_id");
        assert!(!cfg.auto_bootstrap);
        assert!(!cfg.state_store_sync_write);
        assert_eq!(
            cfg.join_targets,
            vec!["127.0.0.1:21001".to_string(), "127.0.0.1:21002".to_string()]
        );
        assert_eq!(cfg.join_retry_interval_ms, 1500);
        assert_eq!(cfg.join_max_attempts, 9);
        assert!(cfg.join_blocking);
        assert_eq!(cfg.join_target_role, KLogJoinTargetRole::Learner);
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
join_retry_interval_ms = 2500
join_max_attempts = 5
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
        assert_eq!(cfg.advertise_port, DEFAULT_RAFT_PORT);
        assert_eq!(cfg.advertise_inter_port, DEFAULT_INTER_NODE_PORT);
        assert_eq!(cfg.advertise_admin_port, DEFAULT_ADMIN_PORT);
        assert_eq!(cfg.rpc_advertise_port, DEFAULT_RPC_PORT);
        assert_eq!(cfg.data_dir, default_data_dir());
        assert_eq!(cfg.cluster_name, "cluster_partial");
        assert_eq!(cfg.cluster_id, "cluster_partial_id");
        assert_eq!(cfg.auto_bootstrap, DEFAULT_AUTO_BOOTSTRAP);
        assert_eq!(cfg.state_store_sync_write, DEFAULT_STATE_STORE_SYNC_WRITE);
        assert!(cfg.join_targets.is_empty());
        assert_eq!(cfg.join_retry_interval_ms, DEFAULT_JOIN_RETRY_INTERVAL_MS);
        assert_eq!(cfg.join_max_attempts, DEFAULT_JOIN_MAX_ATTEMPTS);
        assert_eq!(cfg.join_blocking, DEFAULT_JOIN_BLOCKING);
        assert_eq!(cfg.join_target_role, DEFAULT_JOIN_TARGET_ROLE);
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
                retry_interval_ms: Some(6000),
                max_attempts: Some(3),
                blocking: Some(true),
                target_role: Some(KLogJoinTargetRole::Learner),
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
        assert_eq!(cfg.advertise_port, 23001);
        assert_eq!(cfg.advertise_inter_port, 23002);
        assert_eq!(cfg.advertise_admin_port, 23003);
        assert_eq!(cfg.rpc_advertise_port, 23101);
        assert_eq!(cfg.data_dir, default_data_dir());
        assert_eq!(cfg.cluster_name, "bk");
        assert_eq!(cfg.cluster_id, "bk-id");
        assert!(!cfg.auto_bootstrap);
        assert!(!cfg.state_store_sync_write);
        assert_eq!(cfg.join_targets, vec!["10.0.0.1:21001".to_string()]);
        assert_eq!(cfg.join_retry_interval_ms, 6000);
        assert_eq!(cfg.join_max_attempts, 3);
        assert!(cfg.join_blocking);
        assert_eq!(cfg.join_target_role, KLogJoinTargetRole::Learner);
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
