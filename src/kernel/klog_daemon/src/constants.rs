use buckyos_api::{KLOG_SERVICE_PORT, KLOG_SERVICE_UNIQUE_ID};

/// Environment variable key: config file path.
pub const ENV_CONFIG_FILE: &str = "KLOG_CONFIG_FILE";

/// Environment variable key: raft node id.
pub const ENV_NODE_ID: &str = "KLOG_NODE_ID";

/// Environment variable key: raft server listen address.
pub const ENV_LISTEN_ADDR: &str = "KLOG_LISTEN_ADDR";

/// Environment variable key: client RPC listen address.
pub const ENV_RPC_LISTEN_ADDR: &str = "KLOG_RPC_LISTEN_ADDR";

/// Environment variable key: inter-node data/meta listen address.
pub const ENV_INTER_NODE_LISTEN_ADDR: &str = "KLOG_INTER_NODE_LISTEN_ADDR";

/// Environment variable key: advertised host/IP for cluster peers.
pub const ENV_ADVERTISE_ADDR: &str = "KLOG_ADVERTISE_ADDR";

/// Environment variable key: advertised stable node name for gateway/proxy routing.
pub const ENV_ADVERTISE_NODE_NAME: &str = "KLOG_ADVERTISE_NODE_NAME";

/// Environment variable key: advertised raft protocol port.
pub const ENV_ADVERTISE_PORT: &str = "KLOG_ADVERTISE_PORT";

/// Environment variable key: advertised client RPC port.
pub const ENV_RPC_ADVERTISE_PORT: &str = "KLOG_RPC_ADVERTISE_PORT";

/// Environment variable key: advertised inter-node data/meta port.
pub const ENV_ADVERTISE_INTER_PORT: &str = "KLOG_ADVERTISE_INTER_PORT";

/// Environment variable key: admin API listen address.
pub const ENV_ADMIN_LISTEN_ADDR: &str = "KLOG_ADMIN_LISTEN_ADDR";

/// Environment variable key: advertised admin API port.
pub const ENV_ADMIN_ADVERTISE_PORT: &str = "KLOG_ADMIN_ADVERTISE_PORT";

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

/// Environment variable key: cluster internal transport mode.
pub const ENV_CLUSTER_NETWORK_MODE: &str = "KLOG_CLUSTER_NETWORK_MODE";

/// Environment variable key: node-gateway or local proxy address for cluster transport.
pub const ENV_CLUSTER_GATEWAY_ADDR: &str = "KLOG_CLUSTER_GATEWAY_ADDR";

/// Environment variable key: gateway route prefix for cluster transport proxying.
pub const ENV_CLUSTER_GATEWAY_ROUTE_PREFIX: &str = "KLOG_CLUSTER_GATEWAY_ROUTE_PREFIX";

/// Environment variable key: comma-separated join targets (admin endpoint host:port).
pub const ENV_JOIN_TARGETS: &str = "KLOG_JOIN_TARGETS";

/// Environment variable key: auto-join add-learner blocking mode.
pub const ENV_JOIN_BLOCKING: &str = "KLOG_JOIN_BLOCKING";

/// Environment variable key: target role after join, learner/voter.
pub const ENV_JOIN_TARGET_ROLE: &str = "KLOG_JOIN_TARGET_ROLE";

/// Environment variable key: auto-join retry strategy, fixed/exponential.
pub const ENV_JOIN_RETRY_STRATEGY: &str = "KLOG_JOIN_RETRY_STRATEGY";

/// Environment variable key: auto-join initial retry interval in milliseconds.
pub const ENV_JOIN_RETRY_INITIAL_INTERVAL_MS: &str = "KLOG_JOIN_RETRY_INITIAL_INTERVAL_MS";

/// Environment variable key: auto-join maximum retry interval in milliseconds.
pub const ENV_JOIN_RETRY_MAX_INTERVAL_MS: &str = "KLOG_JOIN_RETRY_MAX_INTERVAL_MS";

/// Environment variable key: auto-join exponential multiplier.
pub const ENV_JOIN_RETRY_MULTIPLIER: &str = "KLOG_JOIN_RETRY_MULTIPLIER";

/// Environment variable key: auto-join jitter ratio in [0.0, 1.0].
pub const ENV_JOIN_RETRY_JITTER_RATIO: &str = "KLOG_JOIN_RETRY_JITTER_RATIO";

/// Environment variable key: max auto-join attempts, 0 means retry forever.
pub const ENV_JOIN_RETRY_MAX_ATTEMPTS: &str = "KLOG_JOIN_RETRY_MAX_ATTEMPTS";

/// Environment variable key: auto-join HTTP request timeout in milliseconds.
pub const ENV_JOIN_RETRY_REQUEST_TIMEOUT_MS: &str = "KLOG_JOIN_RETRY_REQUEST_TIMEOUT_MS";

/// Environment variable key: shuffle join targets every retry round.
pub const ENV_JOIN_RETRY_SHUFFLE_TARGETS: &str = "KLOG_JOIN_RETRY_SHUFFLE_TARGETS";

/// Environment variable key: extra backoff when config-change conflict is detected.
pub const ENV_JOIN_RETRY_CONFIG_CHANGE_CONFLICT_EXTRA_BACKOFF_MS: &str =
    "KLOG_JOIN_RETRY_CONFIG_CHANGE_CONFLICT_EXTRA_BACKOFF_MS";

/// Environment variable key: restrict admin APIs to loopback access.
pub const ENV_ADMIN_LOCAL_ONLY: &str = "KLOG_ADMIN_LOCAL_ONLY";

/// Environment variable key: append API timeout in milliseconds.
pub const ENV_RPC_APPEND_TIMEOUT_MS: &str = "KLOG_RPC_APPEND_TIMEOUT_MS";

/// Environment variable key: append API body size limit in bytes.
pub const ENV_RPC_APPEND_BODY_LIMIT_BYTES: &str = "KLOG_RPC_APPEND_BODY_LIMIT_BYTES";

/// Environment variable key: append API concurrency limit.
pub const ENV_RPC_APPEND_CONCURRENCY: &str = "KLOG_RPC_APPEND_CONCURRENCY";

/// Environment variable key: query API timeout in milliseconds.
pub const ENV_RPC_QUERY_TIMEOUT_MS: &str = "KLOG_RPC_QUERY_TIMEOUT_MS";

/// Environment variable key: query API body size limit in bytes.
pub const ENV_RPC_QUERY_BODY_LIMIT_BYTES: &str = "KLOG_RPC_QUERY_BODY_LIMIT_BYTES";

/// Environment variable key: query API concurrency limit.
pub const ENV_RPC_QUERY_CONCURRENCY: &str = "KLOG_RPC_QUERY_CONCURRENCY";

/// Environment variable key: json-rpc timeout in milliseconds.
pub const ENV_RPC_JSONRPC_TIMEOUT_MS: &str = "KLOG_RPC_JSONRPC_TIMEOUT_MS";

/// Environment variable key: json-rpc body size limit in bytes.
pub const ENV_RPC_JSONRPC_BODY_LIMIT_BYTES: &str = "KLOG_RPC_JSONRPC_BODY_LIMIT_BYTES";

/// Environment variable key: json-rpc concurrency limit.
pub const ENV_RPC_JSONRPC_CONCURRENCY: &str = "KLOG_RPC_JSONRPC_CONCURRENCY";

/// Environment variable key: raft election timeout lower bound in milliseconds.
pub const ENV_RAFT_ELECTION_TIMEOUT_MIN_MS: &str = "KLOG_RAFT_ELECTION_TIMEOUT_MIN_MS";

/// Environment variable key: raft election timeout upper bound in milliseconds.
pub const ENV_RAFT_ELECTION_TIMEOUT_MAX_MS: &str = "KLOG_RAFT_ELECTION_TIMEOUT_MAX_MS";

/// Environment variable key: raft heartbeat interval in milliseconds.
pub const ENV_RAFT_HEARTBEAT_INTERVAL_MS: &str = "KLOG_RAFT_HEARTBEAT_INTERVAL_MS";

/// Environment variable key: raft install snapshot timeout in milliseconds.
pub const ENV_RAFT_INSTALL_SNAPSHOT_TIMEOUT_MS: &str = "KLOG_RAFT_INSTALL_SNAPSHOT_TIMEOUT_MS";

/// Environment variable key: raft max payload entries for replication.
pub const ENV_RAFT_MAX_PAYLOAD_ENTRIES: &str = "KLOG_RAFT_MAX_PAYLOAD_ENTRIES";

/// Environment variable key: raft replication lag threshold.
pub const ENV_RAFT_REPLICATION_LAG_THRESHOLD: &str = "KLOG_RAFT_REPLICATION_LAG_THRESHOLD";

/// Environment variable key: raft snapshot policy, e.g. since_last:5000 or never.
pub const ENV_RAFT_SNAPSHOT_POLICY: &str = "KLOG_RAFT_SNAPSHOT_POLICY";

/// Environment variable key: raft snapshot max chunk size in bytes.
pub const ENV_RAFT_SNAPSHOT_MAX_CHUNK_SIZE_BYTES: &str = "KLOG_RAFT_SNAPSHOT_MAX_CHUNK_SIZE_BYTES";

/// Environment variable key: raft max in-snapshot logs to keep.
pub const ENV_RAFT_MAX_IN_SNAPSHOT_LOG_TO_KEEP: &str = "KLOG_RAFT_MAX_IN_SNAPSHOT_LOG_TO_KEEP";

/// Environment variable key: raft purge batch size.
pub const ENV_RAFT_PURGE_BATCH_SIZE: &str = "KLOG_RAFT_PURGE_BATCH_SIZE";

/// Default host for raft protocol listener.
pub const DEFAULT_LISTEN_HOST: &str = "0.0.0.0";

/// Default host for client RPC listener.
pub const DEFAULT_RPC_LISTEN_HOST: &str = "127.0.0.1";

/// Default host/IP advertised to peers.
pub const DEFAULT_ADVERTISE_ADDR: &str = "127.0.0.1";

/// Default raft protocol port (peer-to-peer replication).
pub const DEFAULT_RAFT_PORT: u16 = 21001;

/// Default inter-node service port (data/meta forwarding).
pub const DEFAULT_INTER_NODE_PORT: u16 = 21002;

/// Default admin service port (cluster membership/state APIs).
pub const DEFAULT_ADMIN_PORT: u16 = 21003;

/// Default client RPC port (local service client).
pub const DEFAULT_RPC_PORT: u16 = KLOG_SERVICE_PORT;

/// Default timeout for append/query/json-rpc handlers in milliseconds.
pub const DEFAULT_RPC_TIMEOUT_MS: u64 = 3_000;

/// Default request body size limit for append/query/json-rpc handlers.
pub const DEFAULT_RPC_BODY_LIMIT_BYTES: usize = 1 * 1024 * 1024;

/// Default in-flight request limit for append/query/json-rpc handlers.
pub const DEFAULT_RPC_CONCURRENCY_LIMIT: usize = 128;

/// Default switch: enable client RPC server.
pub const DEFAULT_ENABLE_RPC_SERVER: bool = true;

/// Default switch: do not auto bootstrap cluster.
pub const DEFAULT_AUTO_BOOTSTRAP: bool = false;

/// Default switch: enable state-store sync write for durability.
pub const DEFAULT_STATE_STORE_SYNC_WRITE: bool = true;

/// Default cluster internal transport mode.
pub const DEFAULT_CLUSTER_NETWORK_MODE: &str = "direct";

/// Default local gateway address for proxy/hybrid cluster transport.
pub const DEFAULT_CLUSTER_GATEWAY_ADDR: &str = "127.0.0.1:3180";

/// Default route prefix used by cluster transport proxy mode.
pub const DEFAULT_CLUSTER_GATEWAY_ROUTE_PREFIX: &str = "/.cluster/klog";

/// Default add-learner request mode, non-blocking.
pub const DEFAULT_JOIN_BLOCKING: bool = false;

/// Default retry strategy for auto-join loop.
pub const DEFAULT_JOIN_RETRY_STRATEGY: &str = "exponential";

/// Default initial retry interval for auto-join loop.
pub const DEFAULT_JOIN_RETRY_INITIAL_INTERVAL_MS: u64 = 3_000;

/// Default upper bound for auto-join retry interval.
pub const DEFAULT_JOIN_RETRY_MAX_INTERVAL_MS: u64 = 30_000;

/// Default multiplier for exponential auto-join backoff.
pub const DEFAULT_JOIN_RETRY_MULTIPLIER: f64 = 1.8;

/// Default random jitter ratio for auto-join interval in [0.0, 1.0].
pub const DEFAULT_JOIN_RETRY_JITTER_RATIO: f64 = 0.2;

/// Default max attempts for auto-join loop, 0 means retry forever.
pub const DEFAULT_JOIN_RETRY_MAX_ATTEMPTS: u32 = 0;

/// Default HTTP timeout for auto-join admin requests.
pub const DEFAULT_JOIN_RETRY_REQUEST_TIMEOUT_MS: u64 = 3_000;

/// Default switch: shuffle join targets every retry round.
pub const DEFAULT_JOIN_RETRY_SHUFFLE_TARGETS: bool = true;

/// Default extra backoff for config-change conflict failures.
pub const DEFAULT_JOIN_RETRY_CONFIG_CHANGE_CONFLICT_EXTRA_BACKOFF_MS: u64 = 1_500;

/// Default switch: admin APIs are loopback-only.
pub const DEFAULT_ADMIN_LOCAL_ONLY: bool = true;

/// Default raft election timeout lower bound in milliseconds.
pub const DEFAULT_RAFT_ELECTION_TIMEOUT_MIN_MS: u64 = 150;

/// Default raft election timeout upper bound in milliseconds.
pub const DEFAULT_RAFT_ELECTION_TIMEOUT_MAX_MS: u64 = 300;

/// Default raft heartbeat interval in milliseconds.
pub const DEFAULT_RAFT_HEARTBEAT_INTERVAL_MS: u64 = 50;

/// Default raft install snapshot timeout in milliseconds.
pub const DEFAULT_RAFT_INSTALL_SNAPSHOT_TIMEOUT_MS: u64 = 200;

/// Default raft max payload entries for replication.
pub const DEFAULT_RAFT_MAX_PAYLOAD_ENTRIES: u64 = 300;

/// Default raft replication lag threshold.
pub const DEFAULT_RAFT_REPLICATION_LAG_THRESHOLD: u64 = 5_000;

/// Default raft snapshot policy string.
pub const DEFAULT_RAFT_SNAPSHOT_POLICY: &str = "since_last:5000";

/// Default raft snapshot max chunk size in bytes (3MiB).
pub const DEFAULT_RAFT_SNAPSHOT_MAX_CHUNK_SIZE_BYTES: u64 = 3 * 1024 * 1024;

/// Default raft max in-snapshot logs to keep.
pub const DEFAULT_RAFT_MAX_IN_SNAPSHOT_LOG_TO_KEEP: u64 = 1_000;

/// Default raft purge batch size.
pub const DEFAULT_RAFT_PURGE_BATCH_SIZE: u64 = 1;

/// Service name used to derive default data dir.
pub const KLOG_SERVICE_NAME: &str = KLOG_SERVICE_UNIQUE_ID;
