/// Environment variable key: config file path.
pub const ENV_CONFIG_FILE: &str = "KLOG_CONFIG_FILE";

/// Environment variable key: raft node id.
pub const ENV_NODE_ID: &str = "KLOG_NODE_ID";

/// Environment variable key: raft server listen address.
pub const ENV_LISTEN_ADDR: &str = "KLOG_LISTEN_ADDR";

/// Environment variable key: client RPC listen address.
pub const ENV_RPC_LISTEN_ADDR: &str = "KLOG_RPC_LISTEN_ADDR";

/// Environment variable key: advertised host/IP for cluster peers.
pub const ENV_ADVERTISE_ADDR: &str = "KLOG_ADVERTISE_ADDR";

/// Environment variable key: advertised raft protocol port.
pub const ENV_ADVERTISE_PORT: &str = "KLOG_ADVERTISE_PORT";

/// Environment variable key: advertised client RPC port.
pub const ENV_RPC_ADVERTISE_PORT: &str = "KLOG_RPC_ADVERTISE_PORT";

/// Environment variable key: whether client RPC server is enabled.
pub const ENV_ENABLE_RPC_SERVER: &str = "KLOG_ENABLE_RPC_SERVER";

/// Environment variable key: storage data directory.
pub const ENV_DATA_DIR: &str = "KLOG_DATA_DIR";

/// Environment variable key: cluster name.
pub const ENV_CLUSTER_NAME: &str = "KLOG_CLUSTER_NAME";

/// Environment variable key: cluster id.
pub const ENV_CLUSTER_ID: &str = "KLOG_CLUSTER_ID";

/// Environment variable key: bootstrap a new single-node cluster.
pub const ENV_AUTO_BOOTSTRAP: &str = "KLOG_AUTO_BOOTSTRAP";

/// Environment variable key: fsync/sync-write for state store.
pub const ENV_STATE_STORE_SYNC_WRITE: &str = "KLOG_STATE_STORE_SYNC_WRITE";

/// Environment variable key: comma-separated join targets.
pub const ENV_JOIN_TARGETS: &str = "KLOG_JOIN_TARGETS";

/// Environment variable key: auto-join retry interval milliseconds.
pub const ENV_JOIN_RETRY_INTERVAL_MS: &str = "KLOG_JOIN_RETRY_INTERVAL_MS";

/// Environment variable key: max auto-join attempts.
pub const ENV_JOIN_MAX_ATTEMPTS: &str = "KLOG_JOIN_MAX_ATTEMPTS";

/// Environment variable key: auto-join add-learner blocking mode.
pub const ENV_JOIN_BLOCKING: &str = "KLOG_JOIN_BLOCKING";

/// Environment variable key: target role after join, learner/voter.
pub const ENV_JOIN_TARGET_ROLE: &str = "KLOG_JOIN_TARGET_ROLE";

/// Environment variable key: restrict admin APIs to loopback access.
pub const ENV_ADMIN_LOCAL_ONLY: &str = "KLOG_ADMIN_LOCAL_ONLY";

/// Default host for raft protocol listener.
pub const DEFAULT_LISTEN_HOST: &str = "0.0.0.0";

/// Default host for client RPC listener.
pub const DEFAULT_RPC_LISTEN_HOST: &str = "127.0.0.1";

/// Default host/IP advertised to peers.
pub const DEFAULT_ADVERTISE_ADDR: &str = "127.0.0.1";

/// Default raft protocol port (peer-to-peer replication).
pub const DEFAULT_RAFT_PORT: u16 = 21001;

/// Default client RPC port (local service client).
pub const DEFAULT_RPC_PORT: u16 = 21101;

/// Default switch: enable client RPC server.
pub const DEFAULT_ENABLE_RPC_SERVER: bool = true;

/// Default switch: do not auto bootstrap cluster.
pub const DEFAULT_AUTO_BOOTSTRAP: bool = false;

/// Default switch: enable state-store sync write for durability.
pub const DEFAULT_STATE_STORE_SYNC_WRITE: bool = true;

/// Default retry interval for auto join loop.
pub const DEFAULT_JOIN_RETRY_INTERVAL_MS: u64 = 3_000;

/// Default max attempts for auto join loop, 0 means retry forever.
pub const DEFAULT_JOIN_MAX_ATTEMPTS: u32 = 0;

/// Default add-learner request mode, non-blocking.
pub const DEFAULT_JOIN_BLOCKING: bool = false;

/// Default switch: admin APIs are loopback-only.
pub const DEFAULT_ADMIN_LOCAL_ONLY: bool = true;

/// Service name used to derive default data dir.
pub const KLOG_SERVICE_NAME: &str = "klog";
