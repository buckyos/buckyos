use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DEFAULT_BUCKYOS_ROOT: &str = "/opt/buckyos";
const DEFAULT_CLUSTER_ROUTE_NAME: &str = "klog-service";
const DEFAULT_TIMEOUT_SECS: u64 = 15;
const MULTI_NODE_MODE: &str = "local-gateway-failover";
const MEMBERSHIP_MODE: &str = "local-gateway-membership";
const OOD_MEMBERSHIP_MODE: &str = "local-gateway-ood-membership";
const OOD_SNAPSHOT_MEMBERSHIP_MODE: &str = "local-gateway-ood-snapshot-membership";
const OOD_LEADER_FAILOVER_SHRINK_MODE: &str = "local-gateway-ood-leader-failover-shrink";
const OOD_SEED_UNAVAILABLE_JOIN_MODE: &str = "local-gateway-ood-seed-unavailable-join";
const OOD_SINGLE_TO_TWO_MODE: &str = "local-gateway-ood-single-to-two";
const OOD_TWO_VOTER_LOSS_MODE: &str = "local-gateway-ood-two-voter-loss";
const RESTART_RECOVERY_MODE: &str = "local-gateway-restart-recovery";
const MVCC_CLUSTER_MODE: &str = "local-gateway-mvcc-cluster";
const MVCC_CHANGE_FEED_MODE: &str = "local-gateway-mvcc-change-feed";
const MVCC_CHANGE_FEED_FAILOVER_MODE: &str = "local-gateway-mvcc-change-feed-failover";
const MVCC_CHANGE_FEED_STRESS_MODE: &str = "local-gateway-mvcc-change-feed-stress";
const MVCC_FAILOVER_MODE: &str = "local-gateway-mvcc-failover";
const MVCC_AUTO_COMPACT_FAILOVER_MODE: &str = "local-gateway-mvcc-auto-compact-failover";
const MVCC_COMPACTION_LEADER_SWITCH_MODE: &str = "local-gateway-mvcc-compaction-leader-switch";
const MVCC_CRASH_RECOVERY_MODE: &str = "local-gateway-mvcc-crash-recovery";
const MVCC_COMPACT_DURING_SNAPSHOT_MODE: &str = "local-gateway-mvcc-compact-during-snapshot";
const RAFT_OLD_LEADER_REJOIN_MODE: &str = "local-gateway-raft-old-leader-rejoin";
const RAFT_FOLLOWER_LAG_SNAPSHOT_INSTALL_MODE: &str =
    "local-gateway-raft-follower-lag-snapshot-install";
const RAFT_QUORUM_LOSS_RECOVERY_MODE: &str = "local-gateway-raft-quorum-loss-recovery";
const RAFT_MEMBERSHIP_CHANGE_REJOIN_MODE: &str = "local-gateway-raft-membership-change-rejoin";
const RAFT_CONCURRENT_MEMBERSHIP_MODE: &str = "local-gateway-raft-concurrent-membership";
const RAFT_JOIN_RETRY_IDEMPOTENCY_MODE: &str = "local-gateway-raft-join-retry-idempotency";
const RAFT_SNAPSHOT_INSTALL_CRASH_MODE: &str = "local-gateway-raft-snapshot-install-crash";
const NODE_ID_REUSE_MODE: &str = "local-gateway-node-id-reuse";
const MVCC_SNAPSHOT_MEMBERSHIP_MODE: &str = "local-gateway-mvcc-snapshot-membership";
const SYSTEM_CONFIG_KV_MODE: &str = "local-gateway-system-config-kv";
const SYSTEM_CONFIG_SERVICE_MODE: &str = "local-gateway-system-config-service";
const SYSTEM_CONFIG_LEADER_FAILOVER_MODE: &str = "local-gateway-system-config-leader-failover";
const GATEWAY_ABNORMAL_MODE: &str = "local-gateway-abnormal";
const SYSTEM_CONFIG_STALE_CONFIG_REJOIN_MODE: &str =
    "local-gateway-system-config-stale-config-rejoin";
const SYSTEM_CONFIG_ROLLOUT_MODE: &str = "local-gateway-system-config-rollout";
const SYSTEM_CONFIG_PAGINATION_MODE: &str = "local-gateway-system-config-pagination";
const SYSTEM_CONFIG_MVCC_MODE: &str = "local-gateway-system-config-mvcc";
const SYSTEM_CONFIG_MULTI_OOD_MVCC_MODE: &str = "local-gateway-system-config-multi-ood-mvcc";
const KLOG_JSON_RPC_SERVICE_PATH: &str = "/kapi/klog-service";
const SYSTEM_CONFIG_RPC_SERVICE_PATH: &str = "/kapi/system_config";
const KLOG_RPC_METHOD_LOG_APPEND: &str = "klog.log.append";
const KLOG_RPC_METHOD_LOG_QUERY: &str = "klog.log.query";
const KLOG_CLUSTER_DV_ROUTE_MODE_ENV: &str = "KLOG_CLUSTER_DV_ROUTE_MODE";
const ENV_SYSTEM_CONFIG_PORT: &str = "BUCKYOS_SYSTEM_CONFIG_PORT";
const ENV_SYSTEM_CONFIG_STORE: &str = "BUCKYOS_SYSTEM_CONFIG_STORE";
const ENV_SYSTEM_CONFIG_KLOG_ENDPOINT: &str = "BUCKYOS_SYSTEM_CONFIG_KLOG_ENDPOINT";
const ENV_SYSTEM_CONFIG_KLOG_NODE_NAME: &str = "BUCKYOS_SYSTEM_CONFIG_KLOG_NODE_NAME";
const ENV_SYSTEM_CONFIG_KLOG_META_QUERY_LIMIT: &str = "BUCKYOS_SYSTEM_CONFIG_KLOG_META_QUERY_LIMIT";
const ENV_SYSTEM_CONFIG_KLOG_BOOTSTRAP_FROM_SLED: &str =
    "BUCKYOS_SYSTEM_CONFIG_KLOG_BOOTSTRAP_FROM_SLED";
const ENV_OOD_SNAPSHOT_MEMBERSHIP_ITEMS: &str = "KLOG_OOD_SNAPSHOT_DV_ITEMS";
const ENV_OOD_SNAPSHOT_MEMBERSHIP_VALUE_BYTES: &str = "KLOG_OOD_SNAPSHOT_DV_VALUE_BYTES";
const ENV_MVCC_SNAPSHOT_MEMBERSHIP_KEYS: &str = "KLOG_MVCC_SNAPSHOT_DV_KEYS";
const ENV_MVCC_CHANGE_FEED_STRESS_KEYS: &str = "KLOG_MVCC_CHANGE_FEED_STRESS_KEYS";
const ENV_MVCC_CHANGE_FEED_STRESS_CONCURRENCY: &str = "KLOG_MVCC_CHANGE_FEED_STRESS_CONCURRENCY";
const ENV_MVCC_CHANGE_FEED_STRESS_ROUNDS: &str = "KLOG_MVCC_CHANGE_FEED_STRESS_ROUNDS";
const ENV_MVCC_CHANGE_FEED_STRESS_PAGE_LIMIT: &str = "KLOG_MVCC_CHANGE_FEED_STRESS_PAGE_LIMIT";
const ENV_MVCC_CHANGE_FEED_STRESS_ROUND_DELAY_MS: &str =
    "KLOG_MVCC_CHANGE_FEED_STRESS_ROUND_DELAY_MS";
const ENV_RAFT_SNAPSHOT_INSTALL_CRASH_ITEMS: &str = "KLOG_RAFT_SNAPSHOT_INSTALL_CRASH_ITEMS";
const ENV_RAFT_SNAPSHOT_INSTALL_CRASH_VALUE_BYTES: &str =
    "KLOG_RAFT_SNAPSHOT_INSTALL_CRASH_VALUE_BYTES";
const ENV_RAFT_SNAPSHOT_INSTALL_CRASH_CHUNK_BYTES: &str =
    "KLOG_RAFT_SNAPSHOT_INSTALL_CRASH_CHUNK_BYTES";
const ENV_MVCC_COMPACT_DURING_SNAPSHOT_KEYS: &str = "KLOG_MVCC_COMPACT_DURING_SNAPSHOT_KEYS";
const ENV_MVCC_COMPACT_DURING_SNAPSHOT_VALUE_BYTES: &str =
    "KLOG_MVCC_COMPACT_DURING_SNAPSHOT_VALUE_BYTES";
const ENV_MVCC_COMPACT_DURING_SNAPSHOT_CHUNK_BYTES: &str =
    "KLOG_MVCC_COMPACT_DURING_SNAPSHOT_CHUNK_BYTES";
const DEFAULT_OOD_SNAPSHOT_MEMBERSHIP_ITEMS: usize = 300;
const DEFAULT_OOD_SNAPSHOT_MEMBERSHIP_VALUE_BYTES: usize = 512;
const DEFAULT_RAFT_SNAPSHOT_INSTALL_CRASH_ITEMS: usize = 300;
const DEFAULT_RAFT_SNAPSHOT_INSTALL_CRASH_VALUE_BYTES: usize = 4096;
const DEFAULT_RAFT_SNAPSHOT_INSTALL_CRASH_CHUNK_BYTES: usize = 4096;
const DEFAULT_MVCC_COMPACT_DURING_SNAPSHOT_KEYS: usize = 80;
const DEFAULT_MVCC_COMPACT_DURING_SNAPSHOT_VALUE_BYTES: usize = 4096;
const DEFAULT_MVCC_COMPACT_DURING_SNAPSHOT_CHUNK_BYTES: usize = 4096;
const DEFAULT_MVCC_SNAPSHOT_MEMBERSHIP_KEYS: usize = 60;
const DEFAULT_MVCC_CHANGE_FEED_STRESS_KEYS: usize = 48;
const DEFAULT_MVCC_CHANGE_FEED_STRESS_CONCURRENCY: usize = 4;
const DEFAULT_MVCC_CHANGE_FEED_STRESS_ROUNDS: usize = 3;
const DEFAULT_MVCC_CHANGE_FEED_STRESS_PAGE_LIMIT: usize = 17;
const DEFAULT_MVCC_CHANGE_FEED_STRESS_ROUND_DELAY_MS: usize = 25;
const TEST_DEVICE_NAME: &str = "ood1";
const TEST_DEVICE_PUBLIC_KEY_X: &str = "vZ2kEJdazmmmmxTYIuVPCt0gGgMOnBP6mMrQmqminB0";
const TEST_DEVICE_PRIVATE_KEY_PEM: &str = r#"
-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIK45kLWIAx3CHmbEmyCST4YB3InSCA4XAV6udqHtRV5P
-----END PRIVATE KEY-----
"#;

#[derive(Debug, Deserialize)]
struct NodeGatewayInfo {
    #[serde(default)]
    cluster_route_map: BTreeMap<String, ClusterRouteEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct ClusterRouteEntry {
    route_prefix: String,
    ingress_port: u16,
    #[serde(default)]
    nodes: BTreeMap<String, ClusterRouteNode>,
}

#[derive(Debug, Clone, Deserialize)]
struct ClusterRouteNode {
    #[serde(default)]
    ports: BTreeMap<String, u16>,
}

#[derive(Debug, Serialize)]
struct AppendRequest {
    message: String,
    timestamp: Option<u64>,
    node_name: Option<String>,
    level: Option<String>,
    source: Option<String>,
    attrs: Option<BTreeMap<String, String>>,
    request_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AppendResponse {
    id: u64,
}

#[derive(Debug, Serialize)]
struct QueryRequest {
    start_id: Option<u64>,
    end_id: Option<u64>,
    limit: Option<usize>,
    desc: Option<bool>,
    level: Option<String>,
    source: Option<String>,
    attr_key: Option<String>,
    attr_value: Option<String>,
    strong_read: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct QueryResponse {
    items: Vec<LogEntry>,
}

#[derive(Debug, Serialize)]
struct MetaPutRequest {
    key: String,
    value: String,
    node_name: Option<String>,
    expected_revision: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct MetaPutResponse {
    key: String,
    revision: u64,
    #[serde(default)]
    create_revision: u64,
    #[serde(default)]
    mod_revision: u64,
    #[serde(default)]
    version: u64,
}

#[derive(Debug, Serialize)]
struct MetaDeleteRequest {
    key: String,
}

#[derive(Debug, Deserialize)]
struct MetaDeleteResponse {
    key: String,
    existed: bool,
    prev_meta: Option<MetaEntry>,
    #[serde(default)]
    meta_version: Option<MetaVersion>,
}

#[derive(Debug, Deserialize)]
struct MetaQueryResponse {
    items: Vec<MetaEntry>,
    #[serde(default)]
    next_cursor: Option<String>,
    #[serde(default)]
    has_more: bool,
}

#[derive(Debug, Deserialize)]
struct MetaEntry {
    key: String,
    value: String,
    revision: u64,
    #[serde(default)]
    create_revision: u64,
    #[serde(default)]
    mod_revision: u64,
    #[serde(default)]
    version: u64,
}

#[derive(Debug, Deserialize)]
struct MetaVersion {
    #[serde(default)]
    create_revision: u64,
    #[serde(default)]
    mod_revision: u64,
    #[serde(default)]
    version: u64,
    #[serde(default)]
    deleted: bool,
}

#[derive(Debug, Deserialize)]
struct MetaTxResponse {
    #[serde(default)]
    revisions: BTreeMap<String, Option<u64>>,
    #[serde(default)]
    meta_versions: BTreeMap<String, MetaVersion>,
}

#[derive(Debug, Clone, Deserialize)]
struct MetaHistoryRecord {
    key: String,
    value: String,
    create_revision: u64,
    mod_revision: u64,
    version: u64,
    deleted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct MetaChangeCursor {
    revision: u64,
    key: String,
}

#[derive(Debug, Deserialize)]
struct MetaChangesResponse {
    items: Vec<MetaHistoryRecord>,
    #[serde(default)]
    next_cursor: Option<MetaChangeCursor>,
    #[serde(default)]
    has_more: bool,
    current_revision: u64,
    next_start_revision: u64,
}

#[derive(Debug, Serialize)]
struct MetaCompactRequest {
    revision: u64,
}

#[derive(Debug, Deserialize)]
struct MetaCompactResponse {
    compacted_revision: u64,
    current_revision: u64,
}

#[derive(Debug, Deserialize)]
struct LogEntry {
    id: u64,
    source: Option<String>,
}

#[derive(Debug, Serialize)]
struct JsonRpcRequest<'a, T>
where
    T: Serialize,
{
    jsonrpc: &'static str,
    method: &'a str,
    params: &'a T,
    id: u64,
}

#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    result: Option<Value>,
    error: Option<Value>,
    id: u64,
}

#[derive(Debug, Serialize)]
struct KrpcRequest<'a> {
    method: &'a str,
    params: &'a Value,
    sys: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct KrpcResponse {
    result: Option<Value>,
    error: Option<String>,
}

fn get_buckyos_root() -> PathBuf {
    std::env::var("BUCKYOS_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_BUCKYOS_ROOT))
}

fn load_json<T>(path: &Path) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    let text = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
    serde_json::from_str(&text)
        .map_err(|err| format!("failed to decode {}: {}", path.display(), err))
}

fn load_local_node_name(buckyos_root: &Path) -> Result<String, String> {
    let path = buckyos_root.join("etc/node_device_config.json");
    let value: Value = load_json(&path)?;
    value
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("missing non-empty node name in {}", path.display()))
}

fn normalize_route_prefix(prefix: &str) -> String {
    let trimmed = prefix.trim();
    let with_leading = if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{}", trimmed)
    };
    let normalized = with_leading.trim_end_matches('/').to_string();
    if normalized.is_empty() {
        "/".to_string()
    } else {
        normalized
    }
}

fn unique_suffix(prefix: &str) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{}-{}", prefix, now)
}

fn parse_env_usize(name: &str, default_value: usize) -> Result<usize, String> {
    match std::env::var(name) {
        Ok(raw) => raw
            .trim()
            .parse::<usize>()
            .map_err(|err| format!("invalid {}={}: {}", name, raw, err)),
        Err(std::env::VarError::NotPresent) => Ok(default_value),
        Err(err) => Err(format!("failed to read {}: {}", name, err)),
    }
}

fn generate_request_id(node_name: &str) -> String {
    format!(
        "{}-{}-{}",
        node_name,
        std::process::id(),
        unique_suffix("req")
    )
}

fn load_klog_cluster_route(buckyos_root: &Path) -> Result<ClusterRouteEntry, String> {
    let route_name = std::env::var("KLOG_CLUSTER_ROUTE_NAME")
        .unwrap_or_else(|_| DEFAULT_CLUSTER_ROUTE_NAME.to_string());
    let path = buckyos_root.join("etc/node_gateway_info.json");
    let info: NodeGatewayInfo = load_json(&path)?;
    let route = info
        .cluster_route_map
        .get(route_name.as_str())
        .ok_or_else(|| {
            let available = info
                .cluster_route_map
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "missing cluster_route_map entry '{}' in {}; available=[{}]",
                route_name,
                path.display(),
                available
            )
        })?;

    if route.nodes.is_empty() {
        return Err(format!(
            "cluster_route_map entry '{}' has no nodes in {}",
            route_name,
            path.display()
        ));
    }

    Ok(ClusterRouteEntry {
        route_prefix: normalize_route_prefix(route.route_prefix.as_str()),
        ingress_port: route.ingress_port,
        nodes: route.nodes.clone(),
    })
}

fn require_node_route<'a>(
    route: &'a ClusterRouteEntry,
    node_name: &str,
) -> Result<&'a ClusterRouteNode, String> {
    let node = route.nodes.get(node_name).ok_or_else(|| {
        let available = route.nodes.keys().cloned().collect::<Vec<_>>().join(", ");
        format!(
            "cluster route does not contain local node '{}'; available=[{}]",
            node_name, available
        )
    })?;

    for plane in ["raft", "inter", "admin"] {
        let port = node.ports.get(plane).copied().unwrap_or_default();
        if port == 0 {
            return Err(format!(
                "cluster route for node '{}' missing non-zero '{}' port: {:?}",
                node_name, plane, node.ports
            ));
        }
    }

    Ok(node)
}

fn gateway_addr_from_route(route: &ClusterRouteEntry) -> String {
    std::env::var("KLOG_NODE_GATEWAY_ADDR")
        .unwrap_or_else(|_| format!("127.0.0.1:{}", route.ingress_port))
}

fn cluster_route_url(
    gateway_addr: &str,
    route_prefix: &str,
    node_name: &str,
    plane: &str,
    suffix: &str,
) -> String {
    let prefix = route_prefix.trim_end_matches('/');
    let path = if prefix.is_empty() {
        format!(
            "/{}/{}/{}",
            node_name,
            plane,
            suffix.trim_start_matches('/')
        )
    } else {
        format!(
            "{}/{}/{}/{}",
            prefix,
            node_name,
            plane,
            suffix.trim_start_matches('/')
        )
    };
    format!("http://{}{}", gateway_addr, path)
}

fn require_query_match(
    response: &QueryResponse,
    expected_id: u64,
    expected_source: &str,
) -> Result<(), String> {
    if response.items.len() != 1 {
        return Err(format!(
            "expected exactly one log item, got {}: {:?}",
            response.items.len(),
            response.items
        ));
    }

    let item = &response.items[0];
    if item.id != expected_id {
        return Err(format!(
            "query returned unexpected id: expected {}, got {}",
            expected_id, item.id
        ));
    }
    if item.source.as_deref() != Some(expected_source) {
        return Err(format!(
            "query returned unexpected source: expected {}, got {:?}",
            expected_source, item.source
        ));
    }

    Ok(())
}

async fn append_via_cluster_inter_route(
    client: &reqwest::Client,
    gateway_addr: &str,
    route_prefix: &str,
    node_name: &str,
    source: &str,
    message: &str,
) -> Result<AppendResponse, String> {
    let url = cluster_route_url(gateway_addr, route_prefix, node_name, "inter", "/append");
    let body = AppendRequest {
        message: message.to_string(),
        timestamp: None,
        node_name: Some(node_name.to_string()),
        level: Some("INFO".to_string()),
        source: Some(source.to_string()),
        attrs: None,
        request_id: Some(generate_request_id(node_name)),
    };

    let response = client
        .post(url.as_str())
        .json(&body)
        .send()
        .await
        .map_err(|err| {
            format!(
                "cluster inter append request failed: url={}, err={}",
                url, err
            )
        })?;
    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|err| format!("<failed to read body: {}>", err));
        return Err(format!(
            "cluster inter append returned non-success status {} from {}: {}",
            status, url, body
        ));
    }

    response.json::<AppendResponse>().await.map_err(|err| {
        format!(
            "failed to decode cluster inter append response from {}: {}",
            url, err
        )
    })
}

async fn query_via_cluster_inter_route(
    client: &reqwest::Client,
    gateway_addr: &str,
    route_prefix: &str,
    node_name: &str,
    log_id: u64,
    source: &str,
) -> Result<QueryResponse, String> {
    let mut url = Url::parse(
        cluster_route_url(gateway_addr, route_prefix, node_name, "inter", "/query").as_str(),
    )
    .map_err(|err| format!("failed to build cluster inter query url: {}", err))?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("start_id", log_id.to_string().as_str());
        query.append_pair("end_id", log_id.to_string().as_str());
        query.append_pair("limit", "4");
        query.append_pair("desc", "false");
        query.append_pair("level", "INFO");
        query.append_pair("source", source);
        query.append_pair("strong_read", "true");
    }

    let response = client.get(url.clone()).send().await.map_err(|err| {
        format!(
            "cluster inter query request failed: url={}, err={}",
            url, err
        )
    })?;
    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|err| format!("<failed to read body: {}>", err));
        return Err(format!(
            "cluster inter query returned non-success status {} from {}: {}",
            status, url, body
        ));
    }

    response.json::<QueryResponse>().await.map_err(|err| {
        format!(
            "failed to decode cluster inter query response from {}: {}",
            url, err
        )
    })
}

async fn put_meta_via_cluster_inter_route(
    client: &reqwest::Client,
    gateway_addr: &str,
    route_prefix: &str,
    node_name: &str,
    key: &str,
    value: &str,
    expected_revision: Option<u64>,
) -> Result<MetaPutResponse, String> {
    let url = cluster_route_url(gateway_addr, route_prefix, node_name, "inter", "/meta-put");
    let body = MetaPutRequest {
        key: key.to_string(),
        value: value.to_string(),
        node_name: Some(node_name.to_string()),
        expected_revision,
    };

    let response = client
        .post(url.as_str())
        .json(&body)
        .send()
        .await
        .map_err(|err| {
            format!(
                "cluster inter meta-put request failed: url={}, err={}",
                url, err
            )
        })?;
    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|err| format!("<failed to read body: {}>", err));
        return Err(format!(
            "cluster inter meta-put returned non-success status {} from {}: {}",
            status, url, body
        ));
    }

    response.json::<MetaPutResponse>().await.map_err(|err| {
        format!(
            "failed to decode cluster inter meta-put response from {}: {}",
            url, err
        )
    })
}

async fn expect_meta_put_error_via_cluster_inter_route(
    client: &reqwest::Client,
    gateway_addr: &str,
    route_prefix: &str,
    node_name: &str,
    key: &str,
    value: &str,
) -> Result<String, String> {
    match put_meta_via_cluster_inter_route(
        client,
        gateway_addr,
        route_prefix,
        node_name,
        key,
        value,
        None,
    )
    .await
    {
        Ok(response) => Err(format!(
            "cluster inter meta-put unexpectedly succeeded for key {}: {:?}",
            key, response
        )),
        Err(err) => Ok(err),
    }
}

async fn expect_meta_put_status_via_cluster_inter_route(
    client: &reqwest::Client,
    gateway_addr: &str,
    route_prefix: &str,
    node_name: &str,
    body: MetaPutRequest,
    expected_status: StatusCode,
) -> Result<(), String> {
    let url = cluster_route_url(gateway_addr, route_prefix, node_name, "inter", "/meta-put");

    let response = client
        .post(url.as_str())
        .json(&body)
        .send()
        .await
        .map_err(|err| {
            format!(
                "cluster inter meta-put request failed: url={}, err={}",
                url, err
            )
        })?;
    let status = response.status();
    if status != expected_status {
        let body = response
            .text()
            .await
            .unwrap_or_else(|err| format!("<failed to read body: {}>", err));
        return Err(format!(
            "cluster inter meta-put expected status {} but got {} from {}: {}",
            expected_status, status, url, body
        ));
    }

    Ok(())
}

async fn query_meta_via_cluster_inter_route(
    client: &reqwest::Client,
    gateway_addr: &str,
    route_prefix: &str,
    node_name: &str,
    key: &str,
) -> Result<MetaQueryResponse, String> {
    query_meta_at_revision_via_cluster_inter_route(
        client,
        gateway_addr,
        route_prefix,
        node_name,
        key,
        None,
    )
    .await
}

async fn query_meta_at_revision_via_cluster_inter_route(
    client: &reqwest::Client,
    gateway_addr: &str,
    route_prefix: &str,
    node_name: &str,
    key: &str,
    revision: Option<u64>,
) -> Result<MetaQueryResponse, String> {
    let mut url = Url::parse(
        cluster_route_url(
            gateway_addr,
            route_prefix,
            node_name,
            "inter",
            "/meta-query",
        )
        .as_str(),
    )
    .map_err(|err| format!("failed to build cluster inter meta-query url: {}", err))?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("key", key);
        query.append_pair("limit", "1");
        query.append_pair("strong_read", "true");
        if let Some(revision) = revision {
            query.append_pair("revision", revision.to_string().as_str());
        }
    }

    let response = client.get(url.clone()).send().await.map_err(|err| {
        format!(
            "cluster inter meta-query request failed: url={}, err={}",
            url, err
        )
    })?;
    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|err| format!("<failed to read body: {}>", err));
        return Err(format!(
            "cluster inter meta-query returned non-success status {} from {}: {}",
            status, url, body
        ));
    }

    response.json::<MetaQueryResponse>().await.map_err(|err| {
        format!(
            "failed to decode cluster inter meta-query response from {}: {}",
            url, err
        )
    })
}

async fn query_meta_prefix_via_cluster_inter_route(
    client: &reqwest::Client,
    gateway_addr: &str,
    route_prefix: &str,
    node_name: &str,
    prefix: &str,
    limit: usize,
) -> Result<MetaQueryResponse, String> {
    query_meta_prefix_page_via_cluster_inter_route(
        client,
        gateway_addr,
        route_prefix,
        node_name,
        prefix,
        limit,
        None,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn query_meta_prefix_page_via_cluster_inter_route(
    client: &reqwest::Client,
    gateway_addr: &str,
    route_prefix: &str,
    node_name: &str,
    prefix: &str,
    limit: usize,
    cursor: Option<&str>,
) -> Result<MetaQueryResponse, String> {
    query_meta_prefix_page_at_revision_via_cluster_inter_route(
        client,
        gateway_addr,
        route_prefix,
        node_name,
        prefix,
        limit,
        cursor,
        None,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn query_meta_prefix_page_at_revision_via_cluster_inter_route(
    client: &reqwest::Client,
    gateway_addr: &str,
    route_prefix: &str,
    node_name: &str,
    prefix: &str,
    limit: usize,
    cursor: Option<&str>,
    revision: Option<u64>,
) -> Result<MetaQueryResponse, String> {
    let mut url = Url::parse(
        cluster_route_url(
            gateway_addr,
            route_prefix,
            node_name,
            "inter",
            "/meta-query",
        )
        .as_str(),
    )
    .map_err(|err| format!("failed to build cluster inter meta-query url: {}", err))?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("prefix", prefix);
        query.append_pair("limit", limit.to_string().as_str());
        query.append_pair("strong_read", "true");
        if let Some(cursor) = cursor {
            query.append_pair("cursor", cursor);
        }
        if let Some(revision) = revision {
            query.append_pair("revision", revision.to_string().as_str());
        }
    }

    let response = client.get(url.clone()).send().await.map_err(|err| {
        format!(
            "cluster inter meta-query request failed: url={}, err={}",
            url, err
        )
    })?;
    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|err| format!("<failed to read body: {}>", err));
        return Err(format!(
            "cluster inter meta-query returned non-success status {} from {}: {}",
            status, url, body
        ));
    }

    response.json::<MetaQueryResponse>().await.map_err(|err| {
        format!(
            "failed to decode cluster inter meta-query response from {}: {}",
            url, err
        )
    })
}

async fn delete_meta_via_cluster_inter_route(
    client: &reqwest::Client,
    gateway_addr: &str,
    route_prefix: &str,
    node_name: &str,
    key: &str,
) -> Result<MetaDeleteResponse, String> {
    let url = cluster_route_url(
        gateway_addr,
        route_prefix,
        node_name,
        "inter",
        "/meta-delete",
    );
    let body = MetaDeleteRequest {
        key: key.to_string(),
    };

    let response = client
        .post(url.as_str())
        .json(&body)
        .send()
        .await
        .map_err(|err| {
            format!(
                "cluster inter meta-delete request failed: url={}, err={}",
                url, err
            )
        })?;
    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|err| format!("<failed to read body: {}>", err));
        return Err(format!(
            "cluster inter meta-delete returned non-success status {} from {}: {}",
            status, url, body
        ));
    }

    response.json::<MetaDeleteResponse>().await.map_err(|err| {
        format!(
            "failed to decode cluster inter meta-delete response from {}: {}",
            url, err
        )
    })
}

async fn exec_meta_tx_via_cluster_inter_route(
    client: &reqwest::Client,
    gateway_addr: &str,
    route_prefix: &str,
    node_name: &str,
    actions: BTreeMap<String, Value>,
) -> Result<MetaTxResponse, String> {
    let url = cluster_route_url(gateway_addr, route_prefix, node_name, "inter", "/meta-tx");
    let body = json!({ "actions": actions });

    let response = client
        .post(url.as_str())
        .json(&body)
        .send()
        .await
        .map_err(|err| {
            format!(
                "cluster inter meta-tx request failed: url={}, err={}",
                url, err
            )
        })?;
    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|err| format!("<failed to read body: {}>", err));
        return Err(format!(
            "cluster inter meta-tx returned non-success status {} from {}: {}",
            status, url, body
        ));
    }

    response.json::<MetaTxResponse>().await.map_err(|err| {
        format!(
            "failed to decode cluster inter meta-tx response from {}: {}",
            url, err
        )
    })
}

#[allow(clippy::too_many_arguments)]
async fn query_meta_changes_via_cluster_inter_route(
    client: &reqwest::Client,
    gateway_addr: &str,
    route_prefix: &str,
    node_name: &str,
    prefix: &str,
    start_revision: u64,
    limit: usize,
    cursor: Option<&MetaChangeCursor>,
) -> Result<MetaChangesResponse, String> {
    query_meta_changes_with_wait_via_cluster_inter_route(
        client,
        gateway_addr,
        route_prefix,
        node_name,
        prefix,
        start_revision,
        limit,
        cursor,
        None,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn query_meta_changes_with_wait_via_cluster_inter_route(
    client: &reqwest::Client,
    gateway_addr: &str,
    route_prefix: &str,
    node_name: &str,
    prefix: &str,
    start_revision: u64,
    limit: usize,
    cursor: Option<&MetaChangeCursor>,
    wait_timeout_ms: Option<u64>,
) -> Result<MetaChangesResponse, String> {
    let mut url = Url::parse(
        cluster_route_url(
            gateway_addr,
            route_prefix,
            node_name,
            "inter",
            "/meta-changes",
        )
        .as_str(),
    )
    .map_err(|err| format!("failed to build cluster inter meta-changes url: {}", err))?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("prefix", prefix);
        query.append_pair("start_revision", start_revision.to_string().as_str());
        query.append_pair("limit", limit.to_string().as_str());
        query.append_pair("include_deleted", "true");
        query.append_pair("strong_read", "true");
        if let Some(cursor) = cursor {
            query.append_pair("cursor_revision", cursor.revision.to_string().as_str());
            query.append_pair("cursor_key", cursor.key.as_str());
        }
        if let Some(wait_timeout_ms) = wait_timeout_ms {
            query.append_pair("wait_timeout_ms", wait_timeout_ms.to_string().as_str());
        }
    }

    let response = client.get(url.clone()).send().await.map_err(|err| {
        format!(
            "cluster inter meta-changes request failed: url={}, err={}",
            url, err
        )
    })?;
    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|err| format!("<failed to read body: {}>", err));
        return Err(format!(
            "cluster inter meta-changes returned non-success status {} from {}: {}",
            status, url, body
        ));
    }

    response.json::<MetaChangesResponse>().await.map_err(|err| {
        format!(
            "failed to decode cluster inter meta-changes response from {}: {}",
            url, err
        )
    })
}

#[allow(clippy::too_many_arguments)]
async fn expect_meta_query_status_via_cluster_inter_route(
    client: &reqwest::Client,
    gateway_addr: &str,
    route_prefix: &str,
    node_name: &str,
    key: Option<&str>,
    prefix: Option<&str>,
    revision: Option<u64>,
    expected_status: StatusCode,
    expected_error_code: Option<&str>,
) -> Result<(), String> {
    let mut url = Url::parse(
        cluster_route_url(
            gateway_addr,
            route_prefix,
            node_name,
            "inter",
            "/meta-query",
        )
        .as_str(),
    )
    .map_err(|err| format!("failed to build cluster inter meta-query url: {}", err))?;
    {
        let mut query = url.query_pairs_mut();
        if let Some(key) = key {
            query.append_pair("key", key);
        }
        if let Some(prefix) = prefix {
            query.append_pair("prefix", prefix);
        }
        if let Some(revision) = revision {
            query.append_pair("revision", revision.to_string().as_str());
        }
        query.append_pair("limit", "16");
        query.append_pair("strong_read", "true");
    }

    let response = client.get(url.clone()).send().await.map_err(|err| {
        format!(
            "cluster inter meta-query status request failed: url={}, err={}",
            url, err
        )
    })?;
    let status = response.status();
    let body = response
        .text()
        .await
        .unwrap_or_else(|err| format!("<failed to read body: {}>", err));
    if status != expected_status {
        return Err(format!(
            "cluster inter meta-query expected status {} but got {} from {}: {}",
            expected_status, status, url, body
        ));
    }
    if let Some(expected_error_code) = expected_error_code {
        require_error_code(body.as_str(), expected_error_code)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn wait_meta_query_compacted_via_cluster_inter_route(
    client: &reqwest::Client,
    gateway_addr: &str,
    route_prefix: &str,
    node_name: &str,
    key: Option<&str>,
    prefix: Option<&str>,
    revision: u64,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match expect_meta_query_status_via_cluster_inter_route(
            client,
            gateway_addr,
            route_prefix,
            node_name,
            key,
            prefix,
            Some(revision),
            StatusCode::GONE,
            Some("COMPACTED"),
        )
        .await
        {
            Ok(()) => return Ok(()),
            Err(err) => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(format!(
                        "timeout waiting meta query compacted at revision {}; last={}",
                        revision, err
                    ));
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn expect_meta_changes_status_via_cluster_inter_route(
    client: &reqwest::Client,
    gateway_addr: &str,
    route_prefix: &str,
    node_name: &str,
    prefix: &str,
    start_revision: u64,
    expected_status: StatusCode,
    expected_error_code: Option<&str>,
) -> Result<(), String> {
    expect_meta_changes_status_with_options_via_cluster_inter_route(
        client,
        gateway_addr,
        route_prefix,
        node_name,
        prefix,
        start_revision,
        None,
        None,
        expected_status,
        expected_error_code,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn expect_meta_changes_status_with_options_via_cluster_inter_route(
    client: &reqwest::Client,
    gateway_addr: &str,
    route_prefix: &str,
    node_name: &str,
    prefix: &str,
    start_revision: u64,
    cursor: Option<&MetaChangeCursor>,
    wait_timeout_ms: Option<u64>,
    expected_status: StatusCode,
    expected_error_code: Option<&str>,
) -> Result<(), String> {
    let mut url = Url::parse(
        cluster_route_url(
            gateway_addr,
            route_prefix,
            node_name,
            "inter",
            "/meta-changes",
        )
        .as_str(),
    )
    .map_err(|err| format!("failed to build cluster inter meta-changes url: {}", err))?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("prefix", prefix);
        query.append_pair("start_revision", start_revision.to_string().as_str());
        query.append_pair("limit", "16");
        query.append_pair("include_deleted", "true");
        query.append_pair("strong_read", "true");
        if let Some(cursor) = cursor {
            query.append_pair("cursor_revision", cursor.revision.to_string().as_str());
            query.append_pair("cursor_key", cursor.key.as_str());
        }
        if let Some(wait_timeout_ms) = wait_timeout_ms {
            query.append_pair("wait_timeout_ms", wait_timeout_ms.to_string().as_str());
        }
    }

    let response = client.get(url.clone()).send().await.map_err(|err| {
        format!(
            "cluster inter meta-changes status request failed: url={}, err={}",
            url, err
        )
    })?;
    let status = response.status();
    let body = response
        .text()
        .await
        .unwrap_or_else(|err| format!("<failed to read body: {}>", err));
    if status != expected_status {
        return Err(format!(
            "cluster inter meta-changes expected status {} but got {} from {}: {}",
            expected_status, status, url, body
        ));
    }
    if let Some(expected_error_code) = expected_error_code {
        require_error_code(body.as_str(), expected_error_code)?;
    }
    Ok(())
}

async fn post_meta_compact_via_admin_route(
    client: &reqwest::Client,
    gateway_addr: &str,
    route_prefix: &str,
    leader_node_name: &str,
    revision: u64,
) -> Result<MetaCompactResponse, String> {
    let url = cluster_route_url(
        gateway_addr,
        route_prefix,
        leader_node_name,
        "admin",
        "/meta-compact",
    );
    let response = client
        .post(url.as_str())
        .json(&MetaCompactRequest { revision })
        .send()
        .await
        .map_err(|err| format!("meta-compact request failed: url={}, err={}", url, err))?;
    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|err| format!("<failed to read body: {}>", err));
        return Err(format!(
            "meta-compact returned non-success status {} from {}: {}",
            status, url, body
        ));
    }

    response.json::<MetaCompactResponse>().await.map_err(|err| {
        format!(
            "failed to decode meta-compact response from {}: {}",
            url, err
        )
    })
}

fn require_error_code(body: &str, expected_error_code: &str) -> Result<(), String> {
    let value = serde_json::from_str::<Value>(body).map_err(|err| {
        format!(
            "failed to parse error body as json: body={}, err={}",
            body, err
        )
    })?;
    let code = value
        .get("error_code")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing error_code in error body: {}", body))?;
    if code != expected_error_code {
        return Err(format!(
            "unexpected error_code: expected={}, actual={}, body={}",
            expected_error_code, code, body
        ));
    }
    Ok(())
}

fn require_meta_value(
    response: &MetaQueryResponse,
    key: &str,
    value: &str,
    revision: u64,
) -> Result<(), String> {
    if response.items.len() != 1
        || response.items[0].key != key
        || response.items[0].value != value
        || response.items[0].revision != revision
    {
        return Err(format!(
            "unexpected meta query result for key {}: items={:?}",
            key, response.items
        ));
    }
    Ok(())
}

fn require_meta_keys(response: &MetaQueryResponse, expected_keys: &[&str]) -> Result<(), String> {
    for key in expected_keys {
        if !response.items.iter().any(|item| item.key == *key) {
            return Err(format!(
                "missing expected meta key {} in items={:?}",
                key, response.items
            ));
        }
    }
    Ok(())
}

fn require_meta_values(
    response: &MetaQueryResponse,
    expected: &[(&str, &str, u64, u64, u64)],
) -> Result<(), String> {
    if response.items.len() != expected.len() {
        return Err(format!(
            "unexpected meta item count: expected={}, actual={}, items={:?}",
            expected.len(),
            response.items.len(),
            response.items
        ));
    }

    for (
        item,
        (
            expected_key,
            expected_value,
            expected_create_revision,
            expected_mod_revision,
            expected_version,
        ),
    ) in response.items.iter().zip(expected.iter())
    {
        if item.key != *expected_key
            || item.value != *expected_value
            || item.revision != *expected_mod_revision
            || item.create_revision != *expected_create_revision
            || item.mod_revision != *expected_mod_revision
            || item.version != *expected_version
        {
            return Err(format!(
                "unexpected meta item: expected=({}, {}, create={}, mod={}, version={}), actual={:?}",
                expected_key,
                expected_value,
                expected_create_revision,
                expected_mod_revision,
                expected_version,
                item
            ));
        }
    }
    Ok(())
}

fn require_meta_selected_values(
    response: &MetaQueryResponse,
    expected: &[(&str, &str, u64, u64, u64)],
) -> Result<(), String> {
    for (
        expected_key,
        expected_value,
        expected_create_revision,
        expected_mod_revision,
        expected_version,
    ) in expected
    {
        let item = response
            .items
            .iter()
            .find(|item| item.key == *expected_key)
            .ok_or_else(|| {
                format!(
                    "missing expected meta key {} in items={:?}",
                    expected_key, response.items
                )
            })?;

        if item.value != *expected_value
            || item.revision != *expected_mod_revision
            || item.create_revision != *expected_create_revision
            || item.mod_revision != *expected_mod_revision
            || item.version != *expected_version
        {
            return Err(format!(
                "unexpected meta item: expected=({}, {}, create={}, mod={}, version={}), actual={:?}",
                expected_key,
                expected_value,
                expected_create_revision,
                expected_mod_revision,
                expected_version,
                item
            ));
        }
    }
    Ok(())
}

fn require_meta_version(
    version: Option<&MetaVersion>,
    create_revision: u64,
    mod_revision: u64,
    item_version: u64,
    deleted: bool,
) -> Result<(), String> {
    let version = version.ok_or_else(|| "missing meta version".to_string())?;
    if version.create_revision != create_revision
        || version.mod_revision != mod_revision
        || version.version != item_version
        || version.deleted != deleted
    {
        return Err(format!(
            "unexpected meta version: expected=({}, {}, {}, {}), actual={:?}",
            create_revision, mod_revision, item_version, deleted, version
        ));
    }
    Ok(())
}

fn require_meta_changes(
    response: &MetaChangesResponse,
    expected: &[(u64, &str, &str, bool, u64, u64)],
) -> Result<(), String> {
    if response.items.len() != expected.len() {
        return Err(format!(
            "unexpected meta changes count: expected={}, actual={}, items={:?}",
            expected.len(),
            response.items.len(),
            response.items
        ));
    }

    for (
        item,
        (
            expected_revision,
            expected_key,
            expected_value,
            expected_deleted,
            expected_create_revision,
            expected_version,
        ),
    ) in response.items.iter().zip(expected.iter())
    {
        if item.mod_revision != *expected_revision
            || item.key != *expected_key
            || item.value != *expected_value
            || item.deleted != *expected_deleted
            || item.create_revision != *expected_create_revision
            || item.version != *expected_version
        {
            return Err(format!(
                "unexpected meta change: expected=(mod={}, key={}, value={}, deleted={}, create={}, version={}), actual={:?}",
                expected_revision,
                expected_key,
                expected_value,
                expected_deleted,
                expected_create_revision,
                expected_version,
                item
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct ExpectedMetaChange {
    revision: u64,
    key: String,
    value: String,
    deleted: bool,
    create_revision: u64,
    version: u64,
}

fn require_expected_meta_changes(
    items: &[MetaHistoryRecord],
    expected: &[ExpectedMetaChange],
) -> Result<(), String> {
    if items.len() != expected.len() {
        return Err(format!(
            "unexpected stress meta changes count: expected={}, actual={}, items={:?}",
            expected.len(),
            items.len(),
            items
        ));
    }

    for (item, expected) in items.iter().zip(expected.iter()) {
        if item.mod_revision != expected.revision
            || item.key != expected.key
            || item.value != expected.value
            || item.deleted != expected.deleted
            || item.create_revision != expected.create_revision
            || item.version != expected.version
        {
            return Err(format!(
                "unexpected stress meta change: expected={:?}, actual={:?}",
                expected, item
            ));
        }
    }
    Ok(())
}

fn require_expected_current_meta_values(
    response: &MetaQueryResponse,
    expected: &[ExpectedMetaChange],
) -> Result<(), String> {
    if response.items.len() != expected.len() {
        return Err(format!(
            "unexpected current meta count: expected={}, actual={}, items={:?}",
            expected.len(),
            response.items.len(),
            response.items
        ));
    }

    for expected in expected {
        let item = response
            .items
            .iter()
            .find(|item| item.key == expected.key)
            .ok_or_else(|| {
                format!(
                    "missing expected current meta key {} in items={:?}",
                    expected.key, response.items
                )
            })?;
        if item.value != expected.value
            || item.revision != expected.revision
            || item.create_revision != expected.create_revision
            || item.mod_revision != expected.revision
            || item.version != expected.version
        {
            return Err(format!(
                "unexpected current meta item: expected={:?}, actual={:?}",
                expected, item
            ));
        }
    }
    Ok(())
}

fn meta_tx_put_action(
    key: &str,
    value: &str,
    node_name: &str,
    expected_revision: Option<u64>,
) -> Value {
    json!({
        "action": "put",
        "item": {
            "key": key,
            "value": value,
            "updated_at": 0_u64,
            "updated_by_node_name": node_name,
        },
        "expected_revision": expected_revision,
    })
}

async fn fetch_cluster_state_via_admin_route(
    client: &reqwest::Client,
    gateway_addr: &str,
    route_prefix: &str,
    node_name: &str,
) -> Result<Value, String> {
    let url = cluster_route_url(
        gateway_addr,
        route_prefix,
        node_name,
        "admin",
        "/cluster-state",
    );
    let response = client.get(url.as_str()).send().await.map_err(|err| {
        format!(
            "cluster admin-state request failed: url={}, err={}",
            url, err
        )
    })?;
    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|err| format!("<failed to read body: {}>", err));
        return Err(format!(
            "cluster admin-state returned non-success status {} from {}: {}",
            status, url, body
        ));
    }

    response.json::<Value>().await.map_err(|err| {
        format!(
            "failed to decode cluster admin-state response from {}: {}",
            url, err
        )
    })
}

async fn post_add_learner_via_admin_route(
    client: &reqwest::Client,
    gateway_addr: &str,
    route_prefix: &str,
    leader_node_name: &str,
    learner: &LocalNodeDef,
    blocking: bool,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let (status, body) = post_add_learner_via_admin_route_status(
            client,
            gateway_addr,
            route_prefix,
            leader_node_name,
            learner,
            blocking,
        )
        .await?;
        if status.is_success() {
            return Ok(());
        }
        if is_membership_change_in_progress(status, body.as_str())
            && tokio::time::Instant::now() < deadline
        {
            tokio::time::sleep(Duration::from_millis(250)).await;
            continue;
        }
        Err(format!(
            "add-learner returned {} from leader {}: {}",
            status, leader_node_name, body
        ))?;
    }
}

fn is_membership_change_in_progress(status: reqwest::StatusCode, body: &str) -> bool {
    status == StatusCode::CONFLICT && body.contains("membership change already in progress")
}

async fn post_add_learner_via_admin_route_status(
    client: &reqwest::Client,
    gateway_addr: &str,
    route_prefix: &str,
    leader_node_name: &str,
    learner: &LocalNodeDef,
    blocking: bool,
) -> Result<(reqwest::StatusCode, String), String> {
    let mut url = Url::parse(
        cluster_route_url(
            gateway_addr,
            route_prefix,
            leader_node_name,
            "admin",
            "/add-learner",
        )
        .as_str(),
    )
    .map_err(|err| format!("failed to build add-learner url: {}", err))?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("node_id", learner.id.to_string().as_str());
        query.append_pair("node_name", learner.name.as_str());
        query.append_pair("device_id", learner.device_id.as_str());
        query.append_pair("addr", "127.0.0.1");
        query.append_pair("port", learner.ports.raft.to_string().as_str());
        query.append_pair("inter_port", learner.ports.inter.to_string().as_str());
        query.append_pair("admin_port", learner.ports.admin.to_string().as_str());
        query.append_pair("rpc_port", learner.ports.rpc.to_string().as_str());
        query.append_pair("blocking", if blocking { "true" } else { "false" });
    }

    let response = client
        .post(url.clone())
        .send()
        .await
        .map_err(|err| format!("add-learner request failed: url={}, err={}", url, err))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .unwrap_or_else(|err| format!("<failed to read body: {}>", err));
    Ok((status, body))
}

async fn post_remove_learner_via_admin_route(
    client: &reqwest::Client,
    gateway_addr: &str,
    route_prefix: &str,
    leader_node_name: &str,
    node_id: u64,
) -> Result<(reqwest::StatusCode, String), String> {
    let mut url = Url::parse(
        cluster_route_url(
            gateway_addr,
            route_prefix,
            leader_node_name,
            "admin",
            "/remove-learner",
        )
        .as_str(),
    )
    .map_err(|err| format!("failed to build remove-learner url: {}", err))?;
    url.query_pairs_mut()
        .append_pair("node_id", node_id.to_string().as_str());

    let response = client
        .post(url.clone())
        .send()
        .await
        .map_err(|err| format!("remove-learner request failed: url={}, err={}", url, err))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .unwrap_or_else(|err| format!("<failed to read body: {}>", err));
    Ok((status, body))
}

async fn post_change_membership_via_admin_route(
    client: &reqwest::Client,
    gateway_addr: &str,
    route_prefix: &str,
    node_name: &str,
    voters: &[u64],
    retain: bool,
) -> Result<(reqwest::StatusCode, String), String> {
    let mut url = Url::parse(
        cluster_route_url(
            gateway_addr,
            route_prefix,
            node_name,
            "admin",
            "/change-membership",
        )
        .as_str(),
    )
    .map_err(|err| format!("failed to build change-membership url: {}", err))?;
    let voters_csv = voters
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(",");
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("voters", voters_csv.as_str());
        query.append_pair("retain", if retain { "true" } else { "false" });
    }

    let response = client
        .post(url.clone())
        .send()
        .await
        .map_err(|err| format!("change-membership request failed: url={}, err={}", url, err))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .unwrap_or_else(|err| format!("<failed to read body: {}>", err));
    Ok((status, body))
}

async fn call_service_json_rpc<Req, Resp>(
    client: &reqwest::Client,
    gateway_addr: &str,
    method: &str,
    params: &Req,
) -> Result<Resp, String>
where
    Req: Serialize,
    Resp: for<'de> Deserialize<'de>,
{
    let url = format!("http://{}{}", gateway_addr, KLOG_JSON_RPC_SERVICE_PATH);
    let request = JsonRpcRequest {
        jsonrpc: "2.0",
        method,
        params,
        id: 1,
    };
    let response = client
        .post(url.as_str())
        .json(&request)
        .send()
        .await
        .map_err(|err| {
            format!(
                "service json-rpc request failed: url={}, method={}, err={}",
                url, method, err
            )
        })?;
    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .unwrap_or_else(|err| format!("<failed to read body: {}>", err));
        return Err(format!(
            "service json-rpc returned non-success status {} from {} method={}: {}",
            status, url, method, body
        ));
    }

    let payload = response.json::<JsonRpcResponse>().await.map_err(|err| {
        format!(
            "failed to decode service json-rpc response from {} method={}: {}",
            url, method, err
        )
    })?;
    if let Some(error) = payload.error {
        return Err(format!(
            "service json-rpc returned error from {} method={} id={}: {}",
            url, method, payload.id, error
        ));
    }
    let result = payload.result.ok_or_else(|| {
        format!(
            "service json-rpc response missing result from {} method={} id={}",
            url, method, payload.id
        )
    })?;
    serde_json::from_value(result).map_err(|err| {
        format!(
            "failed to decode service json-rpc result from {} method={} id={}: {}",
            url, method, payload.id, err
        )
    })
}

async fn append_via_service_route(
    client: &reqwest::Client,
    gateway_addr: &str,
    node_name: &str,
    source: &str,
    message: &str,
) -> Result<AppendResponse, String> {
    let request = AppendRequest {
        message: message.to_string(),
        timestamp: None,
        node_name: Some(node_name.to_string()),
        level: Some("INFO".to_string()),
        source: Some(source.to_string()),
        attrs: None,
        request_id: Some(generate_request_id(node_name)),
    };
    call_service_json_rpc(client, gateway_addr, KLOG_RPC_METHOD_LOG_APPEND, &request).await
}

async fn query_via_service_route(
    client: &reqwest::Client,
    gateway_addr: &str,
    log_id: u64,
    source: &str,
) -> Result<QueryResponse, String> {
    let request = QueryRequest {
        start_id: Some(log_id),
        end_id: Some(log_id),
        limit: Some(4),
        desc: Some(false),
        level: Some("INFO".to_string()),
        source: Some(source.to_string()),
        attr_key: None,
        attr_value: None,
        strong_read: Some(true),
    };
    call_service_json_rpc(client, gateway_addr, KLOG_RPC_METHOD_LOG_QUERY, &request).await
}

fn require_cluster_state(payload: &Value, expected_node_name: &str) -> Result<(), String> {
    let server_state = payload
        .get("server_state")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing server_state in cluster-state payload: {}", payload))?;
    if server_state.trim().is_empty() {
        return Err(format!(
            "server_state is empty in cluster-state payload: {}",
            payload
        ));
    }

    let nodes = payload
        .get("nodes")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("missing nodes object in cluster-state payload: {}", payload))?;
    let route_nodes = nodes
        .values()
        .filter_map(|node| node.get("node_name").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    if !route_nodes.contains(expected_node_name) {
        return Err(format!(
            "cluster-state does not contain expected node_name {}; got={:?}; payload={}",
            expected_node_name, route_nodes, payload
        ));
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct LocalNodePorts {
    raft: u16,
    inter: u16,
    admin: u16,
    rpc: u16,
    rtcp: u16,
    zone_http: u16,
    control: u16,
}

#[derive(Debug, Clone)]
struct LocalNodeDef {
    id: u64,
    name: String,
    device_id: String,
    gateway_host: String,
    ports: LocalNodePorts,
}

#[derive(Debug, Deserialize)]
struct LocalClusterState {
    node_id: u64,
    current_leader: Option<u64>,
    #[serde(default)]
    voters: Vec<u64>,
    #[serde(default)]
    learners: Vec<u64>,
}

struct LocalProcess {
    name: String,
    child: Option<Child>,
}

struct LocalHarness {
    root: PathBuf,
    keep_temp: bool,
    processes: Vec<LocalProcess>,
}

impl LocalHarness {
    fn new() -> Result<Self, String> {
        let root = std::env::temp_dir().join(format!(
            "buckyos-klog-gateway-cluster-dv-{}-{}",
            std::process::id(),
            unique_suffix("run")
        ));
        fs::create_dir_all(root.join("logs"))
            .map_err(|err| format!("failed to create temp root {}: {}", root.display(), err))?;
        Ok(Self {
            root,
            keep_temp: false,
            processes: Vec::new(),
        })
    }

    fn spawn(&mut self, name: &str, command: &mut Command) -> Result<(), String> {
        let stdout = File::create(self.root.join("logs").join(format!("{}.out.log", name)))
            .map_err(|err| format!("failed to create stdout log for {}: {}", name, err))?;
        let stderr = File::create(self.root.join("logs").join(format!("{}.err.log", name)))
            .map_err(|err| format!("failed to create stderr log for {}: {}", name, err))?;
        let child = command
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
            .map_err(|err| format!("failed to spawn {}: {}", name, err))?;
        if let Some(process) = self
            .processes
            .iter_mut()
            .find(|process| process.name == name && process.child.is_none())
        {
            process.child = Some(child);
        } else {
            self.processes.push(LocalProcess {
                name: name.to_string(),
                child: Some(child),
            });
        }
        Ok(())
    }

    fn stop(&mut self, name: &str) -> Result<(), String> {
        let process = self
            .processes
            .iter_mut()
            .rev()
            .find(|process| process.name == name && process.child.is_some())
            .ok_or_else(|| format!("process {} not found", name))?;
        if let Some(mut child) = process.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        Ok(())
    }
}

impl Drop for LocalHarness {
    fn drop(&mut self) {
        for process in self.processes.iter_mut().rev() {
            if let Some(mut child) = process.child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        if !self.keep_temp {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

struct LocalGatewaySetup {
    klog_daemon_bin: PathBuf,
    route_prefix: String,
    ingress_port: u16,
    nodes: Vec<LocalNodeDef>,
    cluster_name: String,
}

struct KLogConfigOptions<'a> {
    seed: &'a LocalNodeDef,
    ingress_port: u16,
    route_prefix: &'a str,
    cluster_name: &'a str,
    auto_join_seed: bool,
    target_role: &'a str,
}

#[derive(Clone, Copy, Default)]
struct KLogRaftPatch<'a> {
    install_snapshot_timeout_ms: Option<u64>,
    max_payload_entries: Option<u64>,
    replication_lag_threshold: Option<u64>,
    snapshot_policy: Option<&'a str>,
    snapshot_max_chunk_size_bytes: Option<u64>,
    max_in_snapshot_log_to_keep: Option<u64>,
    purge_batch_size: Option<u64>,
}

#[derive(Clone, Copy)]
struct KLogMetaCompactionPatch {
    retention_revisions: u64,
    check_interval_ms: u64,
    min_compact_gap: u64,
}

#[derive(Clone, Copy)]
struct KLogJoinRetryPatch {
    initial_interval_ms: u64,
    max_interval_ms: u64,
    max_attempts: u64,
    request_timeout_ms: u64,
}

struct GatewayRuntimeOptions<'a> {
    all_nodes: &'a [LocalNodeDef],
    ingress_port: u16,
    route_prefix: &'a str,
    route_mode: LocalGatewayRouteMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LocalGatewayRouteMode {
    DirectPlane,
    TargetGateway,
}

impl LocalGatewayRouteMode {
    fn from_env() -> Result<Self, String> {
        match std::env::var(KLOG_CLUSTER_DV_ROUTE_MODE_ENV) {
            Ok(value) => match value.as_str() {
                "direct-plane" => Ok(Self::DirectPlane),
                "target-gateway" | "" => Ok(Self::TargetGateway),
                other => Err(format!(
                    "invalid {}={}, expected direct-plane or target-gateway",
                    KLOG_CLUSTER_DV_ROUTE_MODE_ENV, other
                )),
            },
            Err(std::env::VarError::NotPresent) => Ok(Self::TargetGateway),
            Err(err) => Err(format!(
                "failed to read {}: {}",
                KLOG_CLUSTER_DV_ROUTE_MODE_ENV, err
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::DirectPlane => "direct-plane",
            Self::TargetGateway => "target-gateway",
        }
    }
}

fn repo_root() -> Result<PathBuf, String> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            format!(
                "failed to resolve repo root from {}",
                manifest_dir.display()
            )
        })
}

fn first_existing_path(candidates: Vec<PathBuf>, label: &str) -> Result<PathBuf, String> {
    for candidate in candidates {
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(format!("failed to resolve {}", label))
}

fn resolve_cyfs_gateway_bin(repo_root: &Path, buckyos_root: &Path) -> Result<PathBuf, String> {
    if let Ok(raw) = std::env::var("CYFS_GATEWAY_BIN") {
        let path = PathBuf::from(raw.trim());
        if path.exists() {
            return Ok(path);
        }
    }

    first_existing_path(
        vec![
            buckyos_root
                .join("bin")
                .join("cyfs-gateway")
                .join("cyfs_gateway"),
            repo_root
                .join(".dev_buckyos")
                .join("bin")
                .join("cyfs-gateway")
                .join("cyfs_gateway"),
            repo_root
                .parent()
                .unwrap_or(repo_root)
                .join("cyfs-gateway")
                .join("src")
                .join("rootfs")
                .join("bin")
                .join("cyfs-gateway")
                .join("cyfs_gateway"),
        ],
        "cyfs_gateway binary",
    )
}

fn resolve_klog_daemon_bin(repo_root: &Path, buckyos_root: &Path) -> Result<PathBuf, String> {
    if let Ok(raw) = std::env::var("KLOG_DAEMON_BIN") {
        let path = PathBuf::from(raw.trim());
        if path.exists() {
            return Ok(path);
        }
    }

    first_existing_path(
        vec![
            buckyos_root
                .join("bin")
                .join("klog-service")
                .join("klog_daemon"),
            repo_root
                .join(".dev_buckyos")
                .join("bin")
                .join("klog-service")
                .join("klog_daemon"),
            repo_root
                .join("src")
                .join("target")
                .join("debug")
                .join("klog_daemon"),
        ],
        "klog_daemon binary",
    )
}

fn resolve_system_config_bin(repo_root: &Path, buckyos_root: &Path) -> Result<PathBuf, String> {
    if let Ok(raw) = std::env::var("SYSTEM_CONFIG_BIN") {
        let path = PathBuf::from(raw.trim());
        if path.exists() {
            return Ok(path);
        }
    }

    first_existing_path(
        vec![
            buckyos_root
                .join("bin")
                .join("system-config")
                .join("system_config"),
            repo_root
                .join(".dev_buckyos")
                .join("bin")
                .join("system-config")
                .join("system_config"),
            repo_root
                .join("src")
                .join("target")
                .join("debug")
                .join("system_config"),
        ],
        "system_config binary",
    )
}

fn reserve_port(host: &str, port: u16) -> bool {
    TcpListener::bind((host, port)).is_ok()
}

fn pick_common_port(hosts: &[String], used: &mut BTreeSet<u16>) -> Result<u16, String> {
    for _ in 0..200 {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .map_err(|err| format!("failed to bind ephemeral port: {}", err))?;
        let port = listener
            .local_addr()
            .map_err(|err| format!("failed to inspect ephemeral port: {}", err))?
            .port();
        drop(listener);
        if used.contains(&port) {
            continue;
        }
        if hosts.iter().all(|host| reserve_port(host, port)) {
            used.insert(port);
            return Ok(port);
        }
    }
    Err("failed to pick common free port".to_string())
}

fn pick_local_port(used: &mut BTreeSet<u16>) -> Result<u16, String> {
    for _ in 0..200 {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .map_err(|err| format!("failed to bind ephemeral port: {}", err))?;
        let port = listener
            .local_addr()
            .map_err(|err| format!("failed to inspect ephemeral port: {}", err))?
            .port();
        drop(listener);
        if used.insert(port) {
            return Ok(port);
        }
    }
    Err("failed to pick free local port".to_string())
}

fn local_gateway_hosts(count: usize) -> Vec<String> {
    (1..=count).map(|idx| format!("127.0.0.{}", idx)).collect()
}

fn build_local_node_defs(ingress_port: u16, count: usize) -> Result<Vec<LocalNodeDef>, String> {
    let mut used = BTreeSet::from([ingress_port]);
    let hosts = local_gateway_hosts(count);
    let mut nodes = Vec::with_capacity(hosts.len());
    for (idx, host) in hosts.iter().enumerate() {
        nodes.push(LocalNodeDef {
            id: (idx + 1) as u64,
            name: format!("ood{}", idx + 1),
            device_id: format!("did:dv:ood{}", idx + 1),
            gateway_host: host.to_string(),
            ports: LocalNodePorts {
                raft: pick_local_port(&mut used)?,
                inter: pick_local_port(&mut used)?,
                admin: pick_local_port(&mut used)?,
                rpc: pick_local_port(&mut used)?,
                rtcp: pick_local_port(&mut used)?,
                zone_http: pick_local_port(&mut used)?,
                control: pick_local_port(&mut used)?,
            },
        });
    }
    Ok(nodes)
}

fn gateway_addr(node: &LocalNodeDef, ingress_port: u16) -> String {
    format!("{}:{}", node.gateway_host, ingress_port)
}

fn gateway_info_path(harness: &LocalHarness, node: &LocalNodeDef) -> PathBuf {
    harness
        .root
        .join(format!("gateway-{}", node.name))
        .join("etc")
        .join("node_gateway_info.json")
}

fn patch_gateway_direct_route(
    harness: &LocalHarness,
    source_node: &LocalNodeDef,
    target_node_name: &str,
    direct_url: &str,
) -> Result<(), String> {
    let path = gateway_info_path(harness, source_node);
    let content = fs::read_to_string(path.as_path())
        .map_err(|err| format!("failed to read gateway info {}: {}", path.display(), err))?;
    let mut value = serde_json::from_str::<Value>(content.as_str()).map_err(|err| {
        format!(
            "failed to decode gateway info {} before route patch: {}",
            path.display(),
            err
        )
    })?;
    let route = value
        .get_mut("routes")
        .and_then(|routes| routes.get_mut(target_node_name))
        .and_then(|route| route.get_mut("direct"))
        .and_then(|direct| direct.get_mut("url"))
        .ok_or_else(|| {
            format!(
                "missing direct route for target {} in {}",
                target_node_name,
                path.display()
            )
        })?;
    *route = Value::String(direct_url.to_string());
    fs::write(
        path.as_path(),
        serde_json::to_string_pretty(&value).unwrap(),
    )
    .map_err(|err| {
        format!(
            "failed to write gateway info {} after route patch: {}",
            path.display(),
            err
        )
    })
}

fn gateway_admin_join_target(
    source_gateway: &LocalNodeDef,
    ingress_port: u16,
    route_prefix: &str,
    target_node: &LocalNodeDef,
) -> String {
    let gateway = gateway_addr(source_gateway, ingress_port);
    let route_prefix = route_prefix.trim_matches('/');
    if route_prefix.is_empty() {
        format!("http://{}/{}/admin", gateway, target_node.name)
    } else {
        format!(
            "http://{}/{}/{}/admin",
            gateway, route_prefix, target_node.name
        )
    }
}

fn write_gateway_runtime(
    harness: &LocalHarness,
    repo_root: &Path,
    buckyos_root: &Path,
    node: &LocalNodeDef,
    options: &GatewayRuntimeOptions<'_>,
) -> Result<PathBuf, String> {
    let root = harness.root.join(format!("gateway-{}", node.name));
    let etc = root.join("etc");
    fs::create_dir_all(&etc)
        .map_err(|err| format!("failed to create gateway etc {}: {}", etc.display(), err))?;
    fs::create_dir_all(root.join("data").join("srv").join("publish"))
        .map_err(|err| format!("failed to create gateway data dir: {}", err))?;

    let mut boot_gateway = fs::read_to_string(repo_root.join("src/rootfs/etc/boot_gateway.yaml"))
        .map_err(|err| format!("failed to read boot_gateway.yaml: {}", err))?;
    boot_gateway = boot_gateway.replacen(
        "stacks:\n",
        format!(
            "stacks:\n  __control_server__:\n    bind: 127.0.0.1:{}\n",
            node.ports.control
        )
        .as_str(),
        1,
    );
    boot_gateway = boot_gateway.replace(
        "bind: 0.0.0.0:2980",
        format!("bind: {}:{}", node.gateway_host, node.ports.rtcp).as_str(),
    );
    boot_gateway = boot_gateway.replace(
        "bind: 0.0.0.0:80",
        format!("bind: {}:{}", node.gateway_host, node.ports.zone_http).as_str(),
    );
    boot_gateway = boot_gateway.replace(
        "bind: 0.0.0.0:3180",
        format!("bind: {}:{}", node.gateway_host, options.ingress_port).as_str(),
    );
    if options.route_mode == LocalGatewayRouteMode::DirectPlane {
        boot_gateway = boot_gateway.replace(
            r#"local target_url="${route.url}:${KLOG_CLUSTER_INGRESS_PORT}";"#,
            r#"local target_url="${route.url}:${KLOG_CLUSTER_TARGET_PORT}";"#,
        );
    }
    fs::write(etc.join("boot_gateway.yaml"), boot_gateway)
        .map_err(|err| format!("failed to write temp boot_gateway.yaml: {}", err))?;

    fs::copy(
        buckyos_root.join("etc/node_private_key.pem"),
        etc.join("node_private_key.pem"),
    )
    .map_err(|err| format!("failed to copy node_private_key.pem: {}", err))?;
    fs::copy(
        buckyos_root.join("etc/node_device_config.json"),
        etc.join("node_device_config.json"),
    )
    .map_err(|err| format!("failed to copy node_device_config.json: {}", err))?;

    let cluster_nodes = options
        .all_nodes
        .iter()
        .map(|node| {
            (
                node.name.clone(),
                json!({
                    "ports": {
                        "raft": node.ports.raft,
                        "inter": node.ports.inter,
                        "admin": node.ports.admin
                    }
                }),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    let routes = options
        .all_nodes
        .iter()
        .filter(|target| target.name != node.name)
        .map(|target| {
            let route_url = match options.route_mode {
                LocalGatewayRouteMode::DirectPlane => "tcp:///127.0.0.1".to_string(),
                LocalGatewayRouteMode::TargetGateway => {
                    format!("tcp:///{}", target.gateway_host)
                }
            };
            (
                target.name.clone(),
                json!({
                    "direct": {
                        "url": route_url,
                        "backup": false
                    }
                }),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    let gateway_info = json!({
        "node_info": {
            "this_node_id": node.name,
            "this_zone_host": "test.buckyos.io"
        },
        "app_info": {},
        "service_info": {},
        "node_route_map": {},
        "routes": routes,
        "cluster_route_map": {
            "klog-service": {
                "route_prefix": options.route_prefix,
                "ingress_port": options.ingress_port,
                "nodes": cluster_nodes
            }
        },
        "trust_key": {}
    });
    fs::write(
        etc.join("node_gateway_info.json"),
        serde_json::to_string_pretty(&gateway_info).unwrap(),
    )
    .map_err(|err| format!("failed to write node_gateway_info.json: {}", err))?;

    Ok(etc.join("boot_gateway.yaml"))
}

fn write_klog_config(
    harness: &LocalHarness,
    node: &LocalNodeDef,
    options: &KLogConfigOptions<'_>,
) -> Result<PathBuf, String> {
    write_klog_config_with_raft_patch(harness, node, options, KLogRaftPatch::default())
}

fn write_klog_config_with_meta_compaction(
    harness: &LocalHarness,
    node: &LocalNodeDef,
    options: &KLogConfigOptions<'_>,
    meta_compaction: KLogMetaCompactionPatch,
) -> Result<PathBuf, String> {
    let config_path = write_klog_config(harness, node, options)?;
    let mut content = fs::read_to_string(&config_path).map_err(|err| {
        format!(
            "failed to read klog config {} before meta compaction patch: {}",
            config_path.display(),
            err
        )
    })?;
    content.push_str(
        format!(
            r#"
[meta_compaction]
enabled = true
policy = "revision_count"
retention_revisions = {}
check_interval_ms = {}
min_compact_gap = {}
"#,
            meta_compaction.retention_revisions,
            meta_compaction.check_interval_ms,
            meta_compaction.min_compact_gap
        )
        .as_str(),
    );
    fs::write(&config_path, content).map_err(|err| {
        format!(
            "failed to write klog config {} after meta compaction patch: {}",
            config_path.display(),
            err
        )
    })?;
    Ok(config_path)
}

fn write_klog_config_with_join_targets(
    harness: &LocalHarness,
    node: &LocalNodeDef,
    options: &KLogConfigOptions<'_>,
    join_targets: &[String],
) -> Result<PathBuf, String> {
    write_klog_config_inner(
        harness,
        node,
        options,
        KLogRaftPatch::default(),
        Some(join_targets),
    )
}

fn write_klog_config_with_join_targets_and_retry_patch(
    harness: &LocalHarness,
    node: &LocalNodeDef,
    options: &KLogConfigOptions<'_>,
    join_targets: &[String],
    retry_patch: KLogJoinRetryPatch,
    raft_patch: KLogRaftPatch<'_>,
) -> Result<PathBuf, String> {
    let config_path =
        write_klog_config_inner(harness, node, options, raft_patch, Some(join_targets))?;
    let mut content = fs::read_to_string(&config_path).map_err(|err| {
        format!(
            "failed to read klog config {} before join retry patch: {}",
            config_path.display(),
            err
        )
    })?;
    for (from, to) in [
        (
            "initial_interval_ms = 500",
            format!("initial_interval_ms = {}", retry_patch.initial_interval_ms),
        ),
        (
            "max_interval_ms = 500",
            format!("max_interval_ms = {}", retry_patch.max_interval_ms),
        ),
        (
            "max_attempts = 0",
            format!("max_attempts = {}", retry_patch.max_attempts),
        ),
        (
            "request_timeout_ms = 2000",
            format!("request_timeout_ms = {}", retry_patch.request_timeout_ms),
        ),
    ] {
        content = content.replace(from, to.as_str());
    }
    fs::write(&config_path, content).map_err(|err| {
        format!(
            "failed to write klog config {} after join retry patch: {}",
            config_path.display(),
            err
        )
    })?;
    Ok(config_path)
}

fn render_toml_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn render_toml_string_list(values: &[String]) -> String {
    values
        .iter()
        .map(|value| render_toml_string(value))
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_raft_patch(patch: KLogRaftPatch<'_>) -> String {
    let mut lines = String::new();
    if let Some(value) = patch.max_payload_entries {
        lines.push_str(format!("max_payload_entries = {}\n", value).as_str());
    }
    if let Some(value) = patch.replication_lag_threshold {
        lines.push_str(format!("replication_lag_threshold = {}\n", value).as_str());
    }
    if let Some(value) = patch.snapshot_policy {
        lines.push_str(format!("snapshot_policy = \"{}\"\n", value).as_str());
    }
    if let Some(value) = patch.snapshot_max_chunk_size_bytes {
        lines.push_str(format!("snapshot_max_chunk_size_bytes = {}\n", value).as_str());
    }
    if let Some(value) = patch.max_in_snapshot_log_to_keep {
        lines.push_str(format!("max_in_snapshot_log_to_keep = {}\n", value).as_str());
    }
    if let Some(value) = patch.purge_batch_size {
        lines.push_str(format!("purge_batch_size = {}\n", value).as_str());
    }
    lines
}

fn write_klog_config_with_raft_patch(
    harness: &LocalHarness,
    node: &LocalNodeDef,
    options: &KLogConfigOptions<'_>,
    raft_patch: KLogRaftPatch<'_>,
) -> Result<PathBuf, String> {
    write_klog_config_inner(harness, node, options, raft_patch, None)
}

fn write_klog_config_inner(
    harness: &LocalHarness,
    node: &LocalNodeDef,
    options: &KLogConfigOptions<'_>,
    raft_patch: KLogRaftPatch<'_>,
    explicit_join_targets: Option<&[String]>,
) -> Result<PathBuf, String> {
    let data_dir = harness.root.join(format!("klog-data-{}", node.name));
    fs::create_dir_all(&data_dir).map_err(|err| {
        format!(
            "failed to create klog data dir {}: {}",
            data_dir.display(),
            err
        )
    })?;
    let config_path = harness.root.join(format!("klog-{}.toml", node.name));
    let join_targets = if let Some(targets) = explicit_join_targets {
        targets.to_vec()
    } else if node.id == options.seed.id || !options.auto_join_seed {
        Vec::new()
    } else {
        vec![format!("127.0.0.1:{}", options.seed.ports.admin)]
    };
    let join_targets = render_toml_string_list(join_targets.as_slice());
    let install_snapshot_timeout_ms = raft_patch.install_snapshot_timeout_ms.unwrap_or(5000);
    let raft_patch = render_raft_patch(raft_patch);
    let content = format!(
        r#"
node_id = {node_id}

[network]
listen_addr = "127.0.0.1:{raft_port}"
inter_node_listen_addr = "127.0.0.1:{inter_port}"
admin_listen_addr = "127.0.0.1:{admin_port}"
rpc_listen_addr = "127.0.0.1:{rpc_port}"
advertise_addr = "127.0.0.1"
advertise_port = {raft_port}
advertise_inter_port = {inter_port}
advertise_admin_port = {admin_port}
rpc_advertise_port = {rpc_port}
advertise_node_name = "{node_name}"
advertise_device_id = "{device_id}"

[storage]
data_dir = "{data_dir}"
state_store_sync_write = true

[cluster]
name = "{cluster_name}"
id = "{cluster_name}"
auto_bootstrap = {auto_bootstrap}

[cluster_network]
mode = "gateway_proxy"
gateway_addr = "{gateway_addr}"
gateway_route_prefix = "{route_prefix}"

[join]
targets = [{join_targets}]
blocking = true
target_role = "{target_role}"

[join.retry]
strategy = "fixed"
initial_interval_ms = 500
max_interval_ms = 500
multiplier = 1.0
jitter_ratio = 0.0
max_attempts = 0
request_timeout_ms = 2000
shuffle_targets_each_round = false
config_change_conflict_extra_backoff_ms = 0

[raft]
election_timeout_min_ms = 600
election_timeout_max_ms = 1200
heartbeat_interval_ms = 100
install_snapshot_timeout_ms = {install_snapshot_timeout_ms}
{raft_patch}
"#,
        node_id = node.id,
        raft_port = node.ports.raft,
        inter_port = node.ports.inter,
        admin_port = node.ports.admin,
        rpc_port = node.ports.rpc,
        node_name = node.name,
        device_id = node.device_id,
        data_dir = data_dir.display(),
        cluster_name = options.cluster_name,
        auto_bootstrap = if node.id == options.seed.id {
            "true"
        } else {
            "false"
        },
        gateway_addr = gateway_addr(node, options.ingress_port),
        route_prefix = options.route_prefix,
        join_targets = join_targets,
        target_role = options.target_role,
        install_snapshot_timeout_ms = install_snapshot_timeout_ms,
        raft_patch = raft_patch,
    );
    fs::write(&config_path, content).map_err(|err| {
        format!(
            "failed to write klog config {}: {}",
            config_path.display(),
            err
        )
    })?;
    Ok(config_path)
}

fn spawn_gateway(
    harness: &mut LocalHarness,
    cyfs_gateway_bin: &Path,
    config_path: &Path,
    node: &LocalNodeDef,
) -> Result<(), String> {
    let mut command = Command::new(cyfs_gateway_bin);
    command.arg("--config_file").arg(config_path);
    harness.spawn(format!("gateway-{}", node.name).as_str(), &mut command)
}

fn spawn_klog(
    harness: &mut LocalHarness,
    klog_daemon_bin: &Path,
    config_path: &Path,
    node: &LocalNodeDef,
) -> Result<(), String> {
    spawn_klog_with_log_level(harness, klog_daemon_bin, config_path, node, "warn")
}

fn spawn_klog_with_log_level(
    harness: &mut LocalHarness,
    klog_daemon_bin: &Path,
    config_path: &Path,
    node: &LocalNodeDef,
    log_level: &str,
) -> Result<(), String> {
    let mut command = Command::new(klog_daemon_bin);
    command
        .env("KLOG_CONFIG_FILE", config_path)
        .env("RUST_LOG", log_level);
    harness.spawn(format!("klog-{}", node.name).as_str(), &mut command)
}

fn spawn_system_config(
    harness: &mut LocalHarness,
    system_config_bin: &Path,
    service_port: u16,
    klog_endpoint: &str,
) -> Result<(), String> {
    spawn_system_config_with_extra_env(harness, system_config_bin, service_port, klog_endpoint, &[])
}

fn spawn_system_config_with_extra_env(
    harness: &mut LocalHarness,
    system_config_bin: &Path,
    service_port: u16,
    klog_endpoint: &str,
    extra_env: &[(&str, &str)],
) -> Result<(), String> {
    let buckyos_root = harness.root.clone();
    spawn_system_config_with_options_and_extra_env(
        harness,
        "system-config-klog",
        system_config_bin,
        buckyos_root.as_path(),
        service_port,
        Some(klog_endpoint),
        TEST_DEVICE_NAME,
        false,
        extra_env,
    )
}

#[allow(clippy::too_many_arguments)]
fn spawn_system_config_with_options(
    harness: &mut LocalHarness,
    process_name: &str,
    system_config_bin: &Path,
    buckyos_root: &Path,
    service_port: u16,
    klog_endpoint: Option<&str>,
    device_name: &str,
    bootstrap_from_sled: bool,
) -> Result<(), String> {
    spawn_system_config_with_options_and_extra_env(
        harness,
        process_name,
        system_config_bin,
        buckyos_root,
        service_port,
        klog_endpoint,
        device_name,
        bootstrap_from_sled,
        &[],
    )
}

#[allow(clippy::too_many_arguments)]
fn spawn_system_config_with_options_and_extra_env(
    harness: &mut LocalHarness,
    process_name: &str,
    system_config_bin: &Path,
    buckyos_root: &Path,
    service_port: u16,
    klog_endpoint: Option<&str>,
    device_name: &str,
    bootstrap_from_sled: bool,
    extra_env: &[(&str, &str)],
) -> Result<(), String> {
    fs::create_dir_all(buckyos_root).map_err(|err| {
        format!(
            "failed to create system_config root {}: {}",
            buckyos_root.display(),
            err
        )
    })?;

    let mut command = Command::new(system_config_bin);
    command
        .env("BUCKYOS_ROOT", buckyos_root.as_os_str())
        .env(ENV_SYSTEM_CONFIG_PORT, service_port.to_string())
        .env("BUCKYOS_THIS_DEVICE", test_device_doc(device_name))
        .env("RUST_LOG", "warn");

    if let Some(klog_endpoint) = klog_endpoint {
        command
            .env(ENV_SYSTEM_CONFIG_STORE, "klog")
            .env(ENV_SYSTEM_CONFIG_KLOG_ENDPOINT, klog_endpoint)
            .env(ENV_SYSTEM_CONFIG_KLOG_NODE_NAME, device_name);
    }

    if bootstrap_from_sled {
        command.env(ENV_SYSTEM_CONFIG_KLOG_BOOTSTRAP_FROM_SLED, "true");
    }

    for (key, value) in extra_env {
        command.env(key, value);
    }

    harness.spawn(process_name, &mut command)
}

async fn wait_tcp(host: &str, port: u16, timeout: Duration) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;
    let addr = format!("{}:{}", host, port);
    loop {
        match tokio::net::TcpStream::connect(addr.as_str()).await {
            Ok(_) => return Ok(()),
            Err(err) if tokio::time::Instant::now() >= deadline => {
                return Err(format!("timeout waiting for tcp {}: {}", addr, err));
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0)
}

fn test_device_doc(device_name: &str) -> String {
    let did = format!("did:dev:{}", TEST_DEVICE_PUBLIC_KEY_X);
    json!({
        "id": did,
        "verificationMethod": [{
            "type": "Ed25519VerificationKey2020",
            "id": "#main_key",
            "controller": did,
            "publicKeyJwk": {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": TEST_DEVICE_PUBLIC_KEY_X
            }
        }],
        "authentication": ["#main_key"],
        "assertion_method": ["#main_key"],
        "exp": now_secs() + 3600,
        "iat": now_secs(),
        "owner": "did:undefined:undefined",
        "device_type": "ood",
        "name": device_name
    })
    .to_string()
}

fn system_config_jwt(issuer: &str, sub: &str, appid: &str) -> Result<String, String> {
    let private_key = EncodingKey::from_ed_pem(TEST_DEVICE_PRIVATE_KEY_PEM.as_bytes())
        .map_err(|err| format!("failed to load test device private key: {}", err))?;
    let mut header = Header::new(Algorithm::EdDSA);
    header.typ = None;
    jsonwebtoken::encode(
        &header,
        &json!({
            "iss": issuer,
            "sub": sub,
            "appid": appid,
            "exp": now_secs() + 900
        }),
        &private_key,
    )
    .map_err(|err| format!("failed to generate system_config JWT: {}", err))
}

async fn call_system_config_rpc(
    client: &reqwest::Client,
    endpoint: &str,
    token: &str,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let request = KrpcRequest {
        method,
        params: &params,
        sys: vec![json!(0), json!(token)],
    };
    let response = client
        .post(endpoint)
        .json(&request)
        .send()
        .await
        .map_err(|err| format!("system_config rpc {} send failed: {}", method, err))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|err| format!("system_config rpc {} read body failed: {}", method, err))?;
    if !status.is_success() {
        return Err(format!(
            "system_config rpc {} http status {} body={}",
            method, status, body
        ));
    }
    let response: KrpcResponse = serde_json::from_str(body.as_str())
        .map_err(|err| format!("system_config rpc {} decode failed: {}", method, err))?;
    if let Some(error) = response.error {
        return Err(format!("system_config rpc {} failed: {}", method, error));
    }
    Ok(response.result.unwrap_or(Value::Null))
}

async fn expect_system_config_rpc_error(
    client: &reqwest::Client,
    endpoint: &str,
    token: &str,
    method: &str,
    params: Value,
) -> Result<String, String> {
    match call_system_config_rpc(client, endpoint, token, method, params).await {
        Ok(value) => Err(format!(
            "system_config rpc {} unexpectedly succeeded: {}",
            method, value
        )),
        Err(err) => Ok(err),
    }
}

async fn wait_system_config_rpc_success(
    client: &reqwest::Client,
    endpoint: &str,
    token: &str,
    method: &str,
    params: Value,
    timeout: Duration,
) -> Result<Value, String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match call_system_config_rpc(client, endpoint, token, method, params.clone()).await {
            Ok(value) => return Ok(value),
            Err(err) => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(format!(
                        "timeout waiting system_config rpc {} success; last={}",
                        method, err
                    ));
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn require_system_config_klog_failover_error(err: &str) -> Result<(), String> {
    let lower = err.to_ascii_lowercase();
    let expected = [
        "klog",
        "unavailable",
        "timeout",
        "network",
        "leader",
        "connection",
        "failed",
    ];
    if expected.iter().any(|needle| lower.contains(needle)) {
        return Ok(());
    }
    Err(format!(
        "system_config failover error did not look like klog/rpc transient failure: {}",
        err
    ))
}

fn require_gateway_diagnostic_error(err: &str, context: &str) -> Result<(), String> {
    let lower = err.to_ascii_lowercase();
    let has_route_context = lower.contains("http://")
        || lower.contains("url=")
        || lower.contains("status")
        || lower.contains("gateway")
        || lower.contains("route");
    let has_failure_context = [
        "500",
        "502",
        "503",
        "timeout",
        "connect",
        "connection",
        "refused",
        "failed",
        "non-success",
        "tcp",
    ]
    .iter()
    .any(|needle| lower.contains(needle));
    if has_route_context && has_failure_context {
        return Ok(());
    }
    Err(format!(
        "gateway abnormal error for {} is not diagnostic enough: {}",
        context, err
    ))
}

fn require_node_id_reuse_error(err: &str, context: &str) -> Result<(), String> {
    let lower = err.to_ascii_lowercase();
    let has_node_context = lower.contains("node_id") || lower.contains("node id");
    let has_reuse_context = [
        "node identity mismatch",
        "already",
        "voter",
        "learner",
        "membership",
        "exists",
        "conflict",
    ]
    .iter()
    .any(|needle| lower.contains(needle));
    if has_node_context && has_reuse_context {
        return Ok(());
    }
    Err(format!(
        "node-id reuse error for {} is not diagnostic enough: {}",
        context, err
    ))
}

fn require_system_config_value(
    value: &Value,
    expected_value: &str,
    min_version: u64,
) -> Result<u64, String> {
    let actual_value = value
        .get("value")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("system_config get missing value: {}", value))?;
    if actual_value != expected_value {
        return Err(format!(
            "system_config value mismatch: expected={}, actual={}",
            expected_value, actual_value
        ));
    }
    let version = value
        .get("version")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("system_config get missing version: {}", value))?;
    if version < min_version {
        return Err(format!(
            "system_config version too small: expected>={}, actual={}",
            min_version, version
        ));
    }
    Ok(version)
}

fn system_config_value_and_version(value: &Value) -> Result<(String, u64), String> {
    let actual_value = value
        .get("value")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("system_config get missing value: {}", value))?;
    let version = value
        .get("version")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("system_config get missing version: {}", value))?;
    Ok((actual_value.to_string(), version))
}

#[derive(Clone)]
struct SystemConfigRpcEndpoint {
    node_name: String,
    endpoint: String,
    token: String,
}

fn require_system_config_null(value: &Value, context: &str) -> Result<(), String> {
    if !value.is_null() {
        return Err(format!(
            "system_config value should be null for {}: {}",
            context, value
        ));
    }
    Ok(())
}

fn require_meta_key_absent(response: &MetaQueryResponse, key: &str) -> Result<(), String> {
    if response.items.iter().any(|item| item.key == key) {
        return Err(format!(
            "meta key {} should be absent but query returned {:?}",
            key, response.items
        ));
    }
    Ok(())
}

async fn prepare_local_gateway_setup(
    harness: &mut LocalHarness,
    mode: &str,
    route_prefix: &str,
    node_count: usize,
) -> Result<LocalGatewaySetup, String> {
    let repo_root = repo_root()?;
    let buckyos_root = get_buckyos_root();
    let cyfs_gateway_bin = resolve_cyfs_gateway_bin(&repo_root, &buckyos_root)?;
    let klog_daemon_bin = resolve_klog_daemon_bin(&repo_root, &buckyos_root)?;
    let route_mode = LocalGatewayRouteMode::from_env()?;
    let gateway_hosts = local_gateway_hosts(node_count);
    let mut used_ports = BTreeSet::new();
    let ingress_port = pick_common_port(&gateway_hosts, &mut used_ports)?;
    let nodes = build_local_node_defs(ingress_port, node_count)?;
    let cluster_name = format!("klog_{}_{}", mode.replace('-', "_"), ingress_port);

    println!("[klog-cluster-dv] mode={}", mode);
    println!("[klog-cluster-dv] temp_root={}", harness.root.display());
    println!(
        "[klog-cluster-dv] cyfs_gateway_bin={}",
        cyfs_gateway_bin.display()
    );
    println!(
        "[klog-cluster-dv] klog_daemon_bin={}",
        klog_daemon_bin.display()
    );
    println!("[klog-cluster-dv] route_mode={}", route_mode.as_str());
    println!("[klog-cluster-dv] ingress_port={}", ingress_port);

    let gateway_options = GatewayRuntimeOptions {
        all_nodes: &nodes,
        ingress_port,
        route_prefix,
        route_mode,
    };
    for node in &nodes {
        let gateway_config =
            write_gateway_runtime(harness, &repo_root, &buckyos_root, node, &gateway_options)?;
        spawn_gateway(harness, &cyfs_gateway_bin, &gateway_config, node)?;
        wait_tcp(
            node.gateway_host.as_str(),
            ingress_port,
            Duration::from_secs(8),
        )
        .await?;
    }

    Ok(LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix: route_prefix.to_string(),
        ingress_port,
        nodes,
        cluster_name,
    })
}

async fn fetch_cluster_state_for_node(
    client: &reqwest::Client,
    node: &LocalNodeDef,
    ingress_port: u16,
    route_prefix: &str,
) -> Result<LocalClusterState, String> {
    let value = fetch_cluster_state_via_admin_route(
        client,
        gateway_addr(node, ingress_port).as_str(),
        route_prefix,
        node.name.as_str(),
    )
    .await?;
    serde_json::from_value(value).map_err(|err| {
        format!(
            "failed to decode cluster-state for node {} via gateway: {}",
            node.name, err
        )
    })
}

async fn wait_voters(
    client: &reqwest::Client,
    nodes: &[LocalNodeDef],
    ingress_port: u16,
    route_prefix: &str,
    expected_voters: &[u64],
    timeout: Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;
    let expected = expected_voters.iter().copied().collect::<BTreeSet<_>>();
    let mut last = String::new();
    loop {
        let mut ok = true;
        for node in nodes {
            match fetch_cluster_state_for_node(client, node, ingress_port, route_prefix).await {
                Ok(state) => {
                    let voters = state.voters.iter().copied().collect::<BTreeSet<_>>();
                    if voters != expected {
                        ok = false;
                        last = format!("node={} voters={:?}", node.name, voters);
                        break;
                    }
                }
                Err(err) => {
                    ok = false;
                    last = err;
                    break;
                }
            }
        }
        if ok {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "timeout waiting voters {:?}; last={}",
                expected, last
            ));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

async fn wait_membership(
    client: &reqwest::Client,
    nodes: &[LocalNodeDef],
    ingress_port: u16,
    route_prefix: &str,
    expected_voters: &[u64],
    expected_learners: &[u64],
    timeout: Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;
    let expected_voters = expected_voters.iter().copied().collect::<BTreeSet<_>>();
    let expected_learners = expected_learners.iter().copied().collect::<BTreeSet<_>>();
    let mut last = String::new();
    loop {
        let mut ok = true;
        for node in nodes {
            match fetch_cluster_state_for_node(client, node, ingress_port, route_prefix).await {
                Ok(state) => {
                    let voters = state.voters.iter().copied().collect::<BTreeSet<_>>();
                    let learners = state.learners.iter().copied().collect::<BTreeSet<_>>();
                    if voters != expected_voters || learners != expected_learners {
                        ok = false;
                        last = format!(
                            "node={} voters={:?} learners={:?}",
                            node.name, voters, learners
                        );
                        break;
                    }
                }
                Err(err) => {
                    ok = false;
                    last = err;
                    break;
                }
            }
        }
        if ok {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "timeout waiting voters {:?} learners {:?}; last={}",
                expected_voters, expected_learners, last
            ));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

async fn wait_consistent_leader(
    client: &reqwest::Client,
    nodes: &[LocalNodeDef],
    ingress_port: u16,
    route_prefix: &str,
    forbidden_leader: Option<u64>,
    timeout: Duration,
) -> Result<u64, String> {
    let deadline = tokio::time::Instant::now() + timeout;
    let alive_ids = nodes.iter().map(|node| node.id).collect::<BTreeSet<_>>();
    let mut last = String::new();
    loop {
        let mut leaders = BTreeSet::new();
        for node in nodes {
            match fetch_cluster_state_for_node(client, node, ingress_port, route_prefix).await {
                Ok(state) => {
                    if state.node_id != node.id {
                        last = format!(
                            "node {} returned unexpected node_id {}",
                            node.name, state.node_id
                        );
                        leaders.clear();
                        break;
                    }
                    if let Some(leader) = state.current_leader {
                        leaders.insert(leader);
                    }
                }
                Err(err) => {
                    leaders.clear();
                    last = err;
                    break;
                }
            }
        }
        if leaders.len() == 1 {
            let leader = *leaders.iter().next().unwrap();
            if alive_ids.contains(&leader) && Some(leader) != forbidden_leader {
                return Ok(leader);
            }
            last = format!(
                "leader {} not acceptable; alive={:?}, forbidden={:?}",
                leader, alive_ids, forbidden_leader
            );
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!("timeout waiting consistent leader; last={}", last));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

async fn wait_log_visible_on_nodes(
    client: &reqwest::Client,
    nodes: &[LocalNodeDef],
    ingress_port: u16,
    route_prefix: &str,
    log_id: u64,
    source: &str,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut last = String::new();
    loop {
        let mut ok = true;
        for node in nodes {
            match query_via_cluster_inter_route(
                client,
                gateway_addr(node, ingress_port).as_str(),
                route_prefix,
                node.name.as_str(),
                log_id,
                source,
            )
            .await
            .and_then(|response| require_query_match(&response, log_id, source))
            {
                Ok(()) => {}
                Err(err) => {
                    ok = false;
                    last = format!("node={} {}", node.name, err);
                    break;
                }
            }
        }
        if ok {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "timeout waiting log {} visible on nodes; last={}",
                log_id, last
            ));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn klog_snapshot_dir(harness: &LocalHarness, node: &LocalNodeDef) -> PathBuf {
    harness
        .root
        .join(format!("klog-data-{}", node.name))
        .join("snapshots")
}

fn klog_snapshot_temp_path(harness: &LocalHarness, node: &LocalNodeDef) -> PathBuf {
    klog_snapshot_dir(harness, node).join("snapshot.temp")
}

fn klog_out_log_path(harness: &LocalHarness, node: &LocalNodeDef) -> PathBuf {
    harness
        .root
        .join("logs")
        .join(format!("klog-{}.out.log", node.name))
}

async fn wait_klog_out_log_contains(
    harness: &LocalHarness,
    node: &LocalNodeDef,
    patterns: &[&str],
    timeout: Duration,
) -> Result<String, String> {
    let deadline = tokio::time::Instant::now() + timeout;
    let path = klog_out_log_path(harness, node);
    loop {
        let content = fs::read_to_string(&path).unwrap_or_default();
        if patterns.iter().all(|pattern| content.contains(pattern)) {
            return Ok(content);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "timeout waiting klog log {} to contain {:?}; content_len={}",
                path.display(),
                patterns,
                content.len()
            ));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn count_klog_out_log_occurrences(
    harness: &LocalHarness,
    node: &LocalNodeDef,
    pattern: &str,
) -> Result<usize, String> {
    let path = klog_out_log_path(harness, node);
    let content = fs::read_to_string(&path)
        .map_err(|err| format!("failed to read klog log {}: {}", path.display(), err))?;
    Ok(content.matches(pattern).count())
}

async fn wait_snapshot_temp_file_exists(
    harness: &LocalHarness,
    node: &LocalNodeDef,
    timeout: Duration,
) -> Result<u64, String> {
    let deadline = tokio::time::Instant::now() + timeout;
    let path = klog_snapshot_temp_path(harness, node);
    loop {
        if let Ok(metadata) = fs::metadata(&path)
            && metadata.is_file()
            && metadata.len() > 0
        {
            return Ok(metadata.len());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "timeout waiting snapshot temp file for {}: temp={}, snapshot_count={}, log_len={}",
                node.name,
                path.display(),
                snapshot_file_count(harness, node)?,
                fs::read_to_string(klog_out_log_path(harness, node))
                    .map(|content| content.len())
                    .unwrap_or(0)
            ));
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn snapshot_file_count(harness: &LocalHarness, node: &LocalNodeDef) -> Result<usize, String> {
    let snapshot_dir = klog_snapshot_dir(harness, node);
    if !snapshot_dir.exists() {
        return Ok(0);
    }

    let mut count = 0;
    for entry in fs::read_dir(&snapshot_dir).map_err(|err| {
        format!(
            "failed to read snapshot dir {}: {}",
            snapshot_dir.display(),
            err
        )
    })? {
        let entry = entry.map_err(|err| {
            format!(
                "failed to read snapshot dir entry {}: {}",
                snapshot_dir.display(),
                err
            )
        })?;
        if entry.file_name().to_string_lossy().starts_with("snapshot_") && entry.path().is_file() {
            count += 1;
        }
    }
    Ok(count)
}

async fn wait_snapshot_file_count(
    harness: &LocalHarness,
    node: &LocalNodeDef,
    min_count: usize,
    timeout: Duration,
) -> Result<usize, String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let last_count = snapshot_file_count(harness, node)?;
        if last_count >= min_count {
            return Ok(last_count);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "timeout waiting snapshot files for {}: expected>={}, actual={}, dir={}",
                node.name,
                min_count,
                last_count,
                klog_snapshot_dir(harness, node).display()
            ));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

async fn wait_meta_prefix_count_on_nodes(
    client: &reqwest::Client,
    nodes: &[LocalNodeDef],
    ingress_port: u16,
    route_prefix: &str,
    prefix: &str,
    expected_count: usize,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut last = String::new();
    loop {
        let mut ok = true;
        for node in nodes {
            let result = query_meta_prefix_via_cluster_inter_route(
                client,
                gateway_addr(node, ingress_port).as_str(),
                route_prefix,
                node.name.as_str(),
                prefix,
                expected_count + 8,
            )
            .await;
            match result {
                Ok(response) if response.items.len() == expected_count => {}
                Ok(response) => {
                    ok = false;
                    last = format!(
                        "node={} prefix={} expected_count={} actual_count={}",
                        node.name,
                        prefix,
                        expected_count,
                        response.items.len()
                    );
                    break;
                }
                Err(err) => {
                    ok = false;
                    last = format!("node={} {}", node.name, err);
                    break;
                }
            }
        }
        if ok {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "timeout waiting meta prefix count on nodes; last={}",
                last
            ));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[derive(Debug, Clone)]
struct LogMetaWitness {
    log_id: u64,
    log_source: String,
    meta_key: String,
    meta_value: String,
    meta_revision: u64,
}

#[allow(clippy::too_many_arguments)]
async fn wait_meta_value_on_nodes(
    client: &reqwest::Client,
    nodes: &[LocalNodeDef],
    ingress_port: u16,
    route_prefix: &str,
    key: &str,
    value: &str,
    revision: u64,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut last = String::new();
    loop {
        let mut ok = true;
        for node in nodes {
            match query_meta_via_cluster_inter_route(
                client,
                gateway_addr(node, ingress_port).as_str(),
                route_prefix,
                node.name.as_str(),
                key,
            )
            .await
            .and_then(|response| require_meta_value(&response, key, value, revision))
            {
                Ok(()) => {}
                Err(err) => {
                    ok = false;
                    last = format!("node={} {}", node.name, err);
                    break;
                }
            }
        }
        if ok {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "timeout waiting meta key {} visible on nodes; last={}",
                key, last
            ));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

async fn verify_log_and_meta_witness_on_nodes(
    client: &reqwest::Client,
    nodes: &[LocalNodeDef],
    ingress_port: u16,
    route_prefix: &str,
    witness: &LogMetaWitness,
    timeout: Duration,
) -> Result<(), String> {
    wait_log_visible_on_nodes(
        client,
        nodes,
        ingress_port,
        route_prefix,
        witness.log_id,
        witness.log_source.as_str(),
        timeout,
    )
    .await?;
    wait_meta_value_on_nodes(
        client,
        nodes,
        ingress_port,
        route_prefix,
        witness.meta_key.as_str(),
        witness.meta_value.as_str(),
        witness.meta_revision,
        timeout,
    )
    .await
}

async fn write_log_and_meta_witness(
    client: &reqwest::Client,
    ingress_port: u16,
    route_prefix: &str,
    source_gateway: &LocalNodeDef,
    target_node: &LocalNodeDef,
    visible_nodes: &[LocalNodeDef],
    scenario: &str,
) -> Result<LogMetaWitness, String> {
    let source = format!(
        "test/test_klog_cluster_dv-{}-{}",
        scenario,
        unique_suffix("source")
    );
    let append = append_via_cluster_inter_route(
        client,
        gateway_addr(source_gateway, ingress_port).as_str(),
        route_prefix,
        target_node.name.as_str(),
        source.as_str(),
        format!("cluster consistency write {}", scenario).as_str(),
    )
    .await?;
    wait_log_visible_on_nodes(
        client,
        visible_nodes,
        ingress_port,
        route_prefix,
        append.id,
        source.as_str(),
        Duration::from_secs(30),
    )
    .await?;

    let meta_key = format!(
        "test/cluster_consistency/{}/{}",
        scenario,
        unique_suffix("meta")
    );
    let meta_value = format!("meta-value-{}", scenario);
    let meta = put_meta_via_cluster_inter_route(
        client,
        gateway_addr(source_gateway, ingress_port).as_str(),
        route_prefix,
        target_node.name.as_str(),
        meta_key.as_str(),
        meta_value.as_str(),
        Some(0),
    )
    .await?;
    if meta.create_revision != meta.mod_revision || meta.version != 1 {
        return Err(format!(
            "unexpected meta version for {}: create_revision={}, mod_revision={}, version={}",
            scenario, meta.create_revision, meta.mod_revision, meta.version
        ));
    }

    let witness = LogMetaWitness {
        log_id: append.id,
        log_source: source,
        meta_key,
        meta_value,
        meta_revision: meta.mod_revision,
    };
    verify_log_and_meta_witness_on_nodes(
        client,
        visible_nodes,
        ingress_port,
        route_prefix,
        &witness,
        Duration::from_secs(30),
    )
    .await?;

    println!(
        "[klog-cluster-dv] log/meta witness ok: scenario={}, log_id={}, source_gateway={}, target_node={}, visible_nodes={}",
        scenario,
        witness.log_id,
        source_gateway.name,
        target_node.name,
        visible_nodes
            .iter()
            .map(|node| node.name.as_str())
            .collect::<Vec<_>>()
            .join(",")
    );
    Ok(witness)
}

async fn require_log_and_meta_roundtrip(
    client: &reqwest::Client,
    ingress_port: u16,
    route_prefix: &str,
    source_gateway: &LocalNodeDef,
    target_node: &LocalNodeDef,
    visible_nodes: &[LocalNodeDef],
    scenario: &str,
) -> Result<(), String> {
    let source = format!(
        "test/test_klog_ood_membership_dv-{}",
        unique_suffix(scenario)
    );
    let append = append_via_cluster_inter_route(
        client,
        gateway_addr(source_gateway, ingress_port).as_str(),
        route_prefix,
        target_node.name.as_str(),
        source.as_str(),
        format!("ood membership write {}", scenario).as_str(),
    )
    .await?;
    wait_log_visible_on_nodes(
        client,
        visible_nodes,
        ingress_port,
        route_prefix,
        append.id,
        source.as_str(),
        Duration::from_secs(30),
    )
    .await?;

    let meta_key = format!("test/ood_membership/{}/{}", scenario, unique_suffix("meta"));
    let meta_value = format!("meta-value-{}", scenario);
    let meta = put_meta_via_cluster_inter_route(
        client,
        gateway_addr(source_gateway, ingress_port).as_str(),
        route_prefix,
        target_node.name.as_str(),
        meta_key.as_str(),
        meta_value.as_str(),
        Some(0),
    )
    .await?;
    if meta.create_revision != meta.mod_revision || meta.version != 1 {
        return Err(format!(
            "unexpected meta version for {}: create_revision={}, mod_revision={}, version={}",
            scenario, meta.create_revision, meta.mod_revision, meta.version
        ));
    }
    let queried = query_meta_via_cluster_inter_route(
        client,
        gateway_addr(target_node, ingress_port).as_str(),
        route_prefix,
        source_gateway.name.as_str(),
        meta_key.as_str(),
    )
    .await?;
    require_meta_value(
        &queried,
        meta_key.as_str(),
        meta_value.as_str(),
        meta.mod_revision,
    )?;

    println!(
        "[klog-cluster-dv] ood membership roundtrip ok: scenario={}, log_id={}, source_gateway={}, target_node={}",
        scenario, append.id, source_gateway.name, target_node.name
    );
    Ok(())
}

struct SnapshotBulkWitness {
    source: String,
    meta_prefix: String,
    expected_meta_count: usize,
    log_checks: Vec<u64>,
    meta_checks: Vec<(String, String, u64)>,
}

fn fixed_payload(label: &str, index: usize, min_bytes: usize) -> String {
    let seed = format!("{}:{}:", label, index);
    let mut payload = String::with_capacity(min_bytes.max(seed.len()));
    while payload.len() < min_bytes {
        payload.push_str(seed.as_str());
    }
    payload.truncate(min_bytes);
    payload
}

fn snapshot_bulk_checkpoints(count: usize) -> BTreeSet<usize> {
    let mut checkpoints = BTreeSet::new();
    if count == 0 {
        return checkpoints;
    }
    checkpoints.insert(0);
    checkpoints.insert(count / 2);
    checkpoints.insert(count - 1);
    checkpoints
}

#[allow(clippy::too_many_arguments)]
async fn write_snapshot_bulk_data(
    client: &reqwest::Client,
    ingress_port: u16,
    route_prefix: &str,
    source_gateway: &LocalNodeDef,
    target_node: &LocalNodeDef,
    label: &str,
    count: usize,
    value_bytes: usize,
) -> Result<SnapshotBulkWitness, String> {
    if count == 0 {
        return Err("snapshot bulk item count must be greater than 0".to_string());
    }

    let run_id = unique_suffix(label);
    let source = format!(
        "test/test_klog_ood_snapshot_membership_dv/{}/{}",
        label, run_id
    );
    let meta_prefix = format!("test/ood_snapshot_membership/{}/{}/meta/", label, run_id);
    let checkpoints = snapshot_bulk_checkpoints(count);
    let mut log_checks = Vec::new();
    let mut meta_checks = Vec::new();

    for index in 0..count {
        let payload = fixed_payload(label, index, value_bytes);
        let append = append_via_cluster_inter_route(
            client,
            gateway_addr(source_gateway, ingress_port).as_str(),
            route_prefix,
            target_node.name.as_str(),
            source.as_str(),
            format!("{}:{}", index, payload).as_str(),
        )
        .await?;
        let meta_key = format!("{}{:06}", meta_prefix, index);
        let meta_value = format!("{}:{}:{}", label, index, payload);
        let meta = put_meta_via_cluster_inter_route(
            client,
            gateway_addr(source_gateway, ingress_port).as_str(),
            route_prefix,
            target_node.name.as_str(),
            meta_key.as_str(),
            meta_value.as_str(),
            Some(0),
        )
        .await?;
        if meta.create_revision != meta.mod_revision || meta.version != 1 {
            return Err(format!(
                "unexpected snapshot bulk meta version: key={}, create_revision={}, mod_revision={}, version={}",
                meta_key, meta.create_revision, meta.mod_revision, meta.version
            ));
        }

        if checkpoints.contains(&index) {
            log_checks.push(append.id);
            meta_checks.push((meta_key, meta_value, meta.mod_revision));
        }

        if index > 0 && index % 50 == 0 {
            println!(
                "[klog-cluster-dv] snapshot membership bulk progress: label={}, written={}/{}",
                label, index, count
            );
        }
    }

    Ok(SnapshotBulkWitness {
        source,
        meta_prefix,
        expected_meta_count: count,
        log_checks,
        meta_checks,
    })
}

async fn verify_snapshot_bulk_witness(
    client: &reqwest::Client,
    ingress_port: u16,
    route_prefix: &str,
    nodes: &[LocalNodeDef],
    witness: &SnapshotBulkWitness,
    timeout: Duration,
) -> Result<(), String> {
    for log_id in &witness.log_checks {
        wait_log_visible_on_nodes(
            client,
            nodes,
            ingress_port,
            route_prefix,
            *log_id,
            witness.source.as_str(),
            timeout,
        )
        .await?;
    }

    wait_meta_prefix_count_on_nodes(
        client,
        nodes,
        ingress_port,
        route_prefix,
        witness.meta_prefix.as_str(),
        witness.expected_meta_count,
        timeout,
    )
    .await?;

    for node in nodes {
        for (key, value, revision) in &witness.meta_checks {
            let response = query_meta_via_cluster_inter_route(
                client,
                gateway_addr(node, ingress_port).as_str(),
                route_prefix,
                node.name.as_str(),
                key.as_str(),
            )
            .await?;
            require_meta_value(&response, key.as_str(), value.as_str(), *revision)?;
        }
    }

    Ok(())
}

async fn change_voters_via_current_leader(
    client: &reqwest::Client,
    nodes: &[LocalNodeDef],
    ingress_port: u16,
    route_prefix: &str,
    voters: &[u64],
    retain: bool,
) -> Result<u64, String> {
    let leader_id = wait_consistent_leader(
        client,
        nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(50),
    )
    .await?;
    let leader = nodes
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| format!("leader node {} not found", leader_id))?;
    let (status, body) = post_change_membership_via_admin_route(
        client,
        gateway_addr(leader, ingress_port).as_str(),
        route_prefix,
        leader.name.as_str(),
        voters,
        retain,
    )
    .await?;
    if !status.is_success() {
        return Err(format!(
            "change-membership voters={:?} via leader {} returned status={}, body={}",
            voters, leader.name, status, body
        ));
    }
    Ok(leader_id)
}

async fn run_local_gateway_failover_smoke_inner(harness: &mut LocalHarness) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-dv";
    let setup = prepare_local_gateway_setup(harness, MULTI_NODE_MODE, route_prefix, 3).await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();

    let seed = nodes
        .first()
        .ok_or_else(|| "missing seed node".to_string())?
        .clone();
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };
    for node in &nodes {
        let config = write_klog_config(harness, node, &voter_config)?;
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_voters(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        Duration::from_secs(50),
    )
    .await?;
    let leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let follower = nodes
        .iter()
        .find(|node| node.id != leader_id)
        .ok_or_else(|| format!("failed to choose follower; leader_id={}", leader_id))?;
    let first_source = format!(
        "test/test_klog_cluster_dv-gateway-{}",
        unique_suffix("write")
    );
    let first_append = append_via_cluster_inter_route(
        &client,
        gateway_addr(follower, ingress_port).as_str(),
        route_prefix,
        follower.name.as_str(),
        first_source.as_str(),
        "gateway cluster transport write before failover",
    )
    .await?;
    wait_log_visible_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        first_append.id,
        first_source.as_str(),
        Duration::from_secs(20),
    )
    .await?;
    println!(
        "[klog-cluster-dv] gateway transport write replicated before failover: id={}, leader_id={}, follower={}",
        first_append.id, leader_id, follower.name
    );

    let leader = nodes
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| format!("leader node {} not found", leader_id))?;
    harness.stop(format!("klog-{}", leader.name).as_str())?;
    let alive_nodes = nodes
        .iter()
        .filter(|node| node.id != leader_id)
        .cloned()
        .collect::<Vec<_>>();
    let new_leader = wait_consistent_leader(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        Some(leader_id),
        Duration::from_secs(60),
    )
    .await?;
    let write_node = alive_nodes
        .iter()
        .find(|node| node.id != new_leader)
        .unwrap_or_else(|| alive_nodes.first().unwrap());
    let failover_source = format!(
        "test/test_klog_cluster_dv-failover-{}",
        unique_suffix("write")
    );
    let failover_append = append_via_cluster_inter_route(
        &client,
        gateway_addr(write_node, ingress_port).as_str(),
        route_prefix,
        write_node.name.as_str(),
        failover_source.as_str(),
        "gateway cluster transport write after failover",
    )
    .await?;
    wait_log_visible_on_nodes(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        failover_append.id,
        failover_source.as_str(),
        Duration::from_secs(25),
    )
    .await?;
    println!(
        "[klog-cluster-dv] failover write replicated: old_leader={}, new_leader={}, write_node={}, id={}",
        leader_id, new_leader, write_node.name, failover_append.id
    );
    println!("[klog-cluster-dv] local gateway failover smoke success");
    Ok(())
}

async fn run_local_gateway_failover_smoke() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_failover_smoke_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_membership_inner(harness: &mut LocalHarness) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-membership-dv";
    let setup = prepare_local_gateway_setup(harness, MEMBERSHIP_MODE, route_prefix, 4).await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let voter_nodes = nodes.iter().take(3).cloned().collect::<Vec<_>>();
    let learner = nodes
        .get(3)
        .cloned()
        .ok_or_else(|| "missing learner node".to_string())?;
    let seed = voter_nodes
        .first()
        .ok_or_else(|| "missing seed node".to_string())?
        .clone();
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };

    for node in &voter_nodes {
        let config = write_klog_config(harness, node, &voter_config)?;
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let learner_options = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: false,
        target_role: "learner",
    };
    let learner_config = write_klog_config(harness, &learner, &learner_options)?;
    spawn_klog(harness, &klog_daemon_bin, &learner_config, &learner)?;
    wait_tcp("127.0.0.1", learner.ports.admin, Duration::from_secs(12)).await?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &voter_nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(50),
    )
    .await?;

    let leader_id = wait_consistent_leader(
        &client,
        &voter_nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let leader = voter_nodes
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| format!("leader node {} not found", leader_id))?;
    post_add_learner_via_admin_route(
        &client,
        gateway_addr(leader, ingress_port).as_str(),
        route_prefix,
        leader.name.as_str(),
        &learner,
        true,
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[4],
        Duration::from_secs(60),
    )
    .await?;
    println!(
        "[klog-cluster-dv] gateway admin add-learner ok: leader={}, learner={}",
        leader.name, learner.name
    );

    let leader_id = wait_consistent_leader(
        &client,
        &voter_nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let leader = voter_nodes
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| format!("leader node {} not found after add-learner", leader_id))?;
    let follower = voter_nodes
        .iter()
        .find(|node| node.id != leader_id)
        .ok_or_else(|| "failed to choose follower for admin semantics check".to_string())?;
    let (status_change, body_change) = post_change_membership_via_admin_route(
        &client,
        gateway_addr(follower, ingress_port).as_str(),
        route_prefix,
        follower.name.as_str(),
        &[1, 2, 3],
        true,
    )
    .await?;
    if status_change != reqwest::StatusCode::CONFLICT {
        return Err(format!(
            "follower change-membership should return 409 via gateway, got status={}, body={}",
            status_change, body_change
        ));
    }

    let (status_remove, body_remove) = post_remove_learner_via_admin_route(
        &client,
        gateway_addr(leader, ingress_port).as_str(),
        route_prefix,
        leader.name.as_str(),
        learner.id,
    )
    .await?;
    if !status_remove.is_success() {
        return Err(format!(
            "remove-learner via gateway returned status={}, body={}",
            status_remove, body_remove
        ));
    }
    wait_membership(
        &client,
        &voter_nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;

    let (repeat_status, repeat_body) = post_remove_learner_via_admin_route(
        &client,
        gateway_addr(leader, ingress_port).as_str(),
        route_prefix,
        leader.name.as_str(),
        learner.id,
    )
    .await?;
    if repeat_status != reqwest::StatusCode::OK
        && repeat_status != reqwest::StatusCode::CONFLICT
        && repeat_status != reqwest::StatusCode::INTERNAL_SERVER_ERROR
    {
        return Err(format!(
            "unexpected repeated remove-learner status via gateway: status={}, body={}",
            repeat_status, repeat_body
        ));
    }
    wait_membership(
        &client,
        &voter_nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(30),
    )
    .await?;
    println!(
        "[klog-cluster-dv] gateway admin remove-learner semantics ok: repeat_status={}",
        repeat_status
    );
    println!("[klog-cluster-dv] local gateway membership DV success");
    Ok(())
}

async fn run_local_gateway_membership() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_membership_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_ood_membership_three_to_four(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-ood-membership-3-4-dv";
    let setup = prepare_local_gateway_setup(harness, OOD_MEMBERSHIP_MODE, route_prefix, 4).await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let base_voters = nodes.iter().take(3).cloned().collect::<Vec<_>>();
    let added_ood = nodes
        .get(3)
        .cloned()
        .ok_or_else(|| "missing fourth OOD node".to_string())?;
    let seed = base_voters
        .first()
        .ok_or_else(|| "missing seed node".to_string())?
        .clone();
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };

    for node in &base_voters {
        let config = write_klog_config(harness, node, &voter_config)?;
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        wait_tcp("127.0.0.1", node.ports.inter, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(50),
    )
    .await?;
    require_log_and_meta_roundtrip(
        &client,
        ingress_port,
        route_prefix,
        &base_voters[0],
        &base_voters[1],
        &base_voters,
        "three-voters-before-add",
    )
    .await?;

    let added_options = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: false,
        target_role: "learner",
    };
    let added_config = write_klog_config(harness, &added_ood, &added_options)?;
    spawn_klog(harness, &klog_daemon_bin, &added_config, &added_ood)?;
    wait_tcp("127.0.0.1", added_ood.ports.admin, Duration::from_secs(12)).await?;
    wait_tcp("127.0.0.1", added_ood.ports.inter, Duration::from_secs(12)).await?;

    let leader_id = wait_consistent_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let leader = base_voters
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| format!("leader node {} not found before add OOD", leader_id))?;
    post_add_learner_via_admin_route(
        &client,
        gateway_addr(leader, ingress_port).as_str(),
        route_prefix,
        leader.name.as_str(),
        &added_ood,
        true,
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[4],
        Duration::from_secs(60),
    )
    .await?;

    let promote_leader = change_voters_via_current_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        &[1, 2, 3, 4],
        true,
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3, 4],
        &[],
        Duration::from_secs(70),
    )
    .await?;
    require_log_and_meta_roundtrip(
        &client,
        ingress_port,
        route_prefix,
        &added_ood,
        &base_voters[0],
        &nodes,
        "four-voters-after-add",
    )
    .await?;

    let demote_leader = change_voters_via_current_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        true,
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[4],
        Duration::from_secs(70),
    )
    .await?;

    let leader_id = wait_consistent_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let leader = base_voters
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| format!("leader node {} not found before remove OOD", leader_id))?;
    let (status_remove, body_remove) = post_remove_learner_via_admin_route(
        &client,
        gateway_addr(leader, ingress_port).as_str(),
        route_prefix,
        leader.name.as_str(),
        added_ood.id,
    )
    .await?;
    if !status_remove.is_success() {
        return Err(format!(
            "remove fourth OOD learner returned status={}, body={}",
            status_remove, body_remove
        ));
    }
    wait_membership(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    harness.stop(format!("klog-{}", added_ood.name).as_str())?;
    require_log_and_meta_roundtrip(
        &client,
        ingress_port,
        route_prefix,
        &base_voters[2],
        &base_voters[0],
        &base_voters,
        "three-voters-after-remove",
    )
    .await?;

    println!(
        "[klog-cluster-dv] ood membership 3<->4 ok: promote_leader={}, demote_leader={}, removed_ood={}",
        promote_leader, demote_leader, added_ood.name
    );
    Ok(())
}

async fn run_local_gateway_ood_membership_one_to_two(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-ood-membership-1-2-dv";
    let setup = prepare_local_gateway_setup(harness, OOD_MEMBERSHIP_MODE, route_prefix, 2).await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let seed = nodes
        .first()
        .cloned()
        .ok_or_else(|| "missing single OOD seed node".to_string())?;
    let added_ood = nodes
        .get(1)
        .cloned()
        .ok_or_else(|| "missing second OOD node".to_string())?;
    let seed_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };
    let config = write_klog_config(harness, &seed, &seed_config)?;
    spawn_klog(harness, &klog_daemon_bin, &config, &seed)?;
    wait_tcp("127.0.0.1", seed.ports.admin, Duration::from_secs(12)).await?;
    wait_tcp("127.0.0.1", seed.ports.inter, Duration::from_secs(12)).await?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        std::slice::from_ref(&seed),
        ingress_port,
        route_prefix,
        &[1],
        &[],
        Duration::from_secs(30),
    )
    .await?;
    require_log_and_meta_roundtrip(
        &client,
        ingress_port,
        route_prefix,
        &seed,
        &seed,
        std::slice::from_ref(&seed),
        "one-voter-before-add",
    )
    .await?;

    let added_options = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: false,
        target_role: "learner",
    };
    let added_config = write_klog_config(harness, &added_ood, &added_options)?;
    spawn_klog(harness, &klog_daemon_bin, &added_config, &added_ood)?;
    wait_tcp("127.0.0.1", added_ood.ports.admin, Duration::from_secs(12)).await?;
    wait_tcp("127.0.0.1", added_ood.ports.inter, Duration::from_secs(12)).await?;
    post_add_learner_via_admin_route(
        &client,
        gateway_addr(&seed, ingress_port).as_str(),
        route_prefix,
        seed.name.as_str(),
        &added_ood,
        true,
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1],
        &[2],
        Duration::from_secs(60),
    )
    .await?;

    let promote_leader = change_voters_via_current_leader(
        &client,
        std::slice::from_ref(&seed),
        ingress_port,
        route_prefix,
        &[1, 2],
        true,
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2],
        &[],
        Duration::from_secs(70),
    )
    .await?;
    require_log_and_meta_roundtrip(
        &client,
        ingress_port,
        route_prefix,
        &added_ood,
        &seed,
        &nodes,
        "two-voters-after-add",
    )
    .await?;

    let demote_leader =
        change_voters_via_current_leader(&client, &nodes, ingress_port, route_prefix, &[1], true)
            .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1],
        &[2],
        Duration::from_secs(70),
    )
    .await?;
    let (status_remove, body_remove) = post_remove_learner_via_admin_route(
        &client,
        gateway_addr(&seed, ingress_port).as_str(),
        route_prefix,
        seed.name.as_str(),
        added_ood.id,
    )
    .await?;
    if !status_remove.is_success() {
        return Err(format!(
            "remove second OOD learner returned status={}, body={}",
            status_remove, body_remove
        ));
    }
    wait_membership(
        &client,
        std::slice::from_ref(&seed),
        ingress_port,
        route_prefix,
        &[1],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    harness.stop(format!("klog-{}", added_ood.name).as_str())?;
    require_log_and_meta_roundtrip(
        &client,
        ingress_port,
        route_prefix,
        &seed,
        &seed,
        std::slice::from_ref(&seed),
        "one-voter-after-remove",
    )
    .await?;

    println!(
        "[klog-cluster-dv] ood membership 1<->2 ok: promote_leader={}, demote_leader={}, removed_ood={}",
        promote_leader, demote_leader, added_ood.name
    );
    Ok(())
}

async fn run_local_gateway_ood_membership_two_to_three(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-ood-membership-2-3-dv";
    let setup = prepare_local_gateway_setup(harness, OOD_MEMBERSHIP_MODE, route_prefix, 3).await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let base_voters = nodes.iter().take(2).cloned().collect::<Vec<_>>();
    let added_ood = nodes
        .get(2)
        .cloned()
        .ok_or_else(|| "missing third OOD node".to_string())?;
    let seed = base_voters
        .first()
        .ok_or_else(|| "missing seed node".to_string())?
        .clone();
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };

    for node in &base_voters {
        let config = write_klog_config(harness, node, &voter_config)?;
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        wait_tcp("127.0.0.1", node.ports.inter, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        &[1, 2],
        &[],
        Duration::from_secs(50),
    )
    .await?;
    require_log_and_meta_roundtrip(
        &client,
        ingress_port,
        route_prefix,
        &base_voters[0],
        &base_voters[1],
        &base_voters,
        "two-voters-before-add-third",
    )
    .await?;

    let added_options = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: false,
        target_role: "learner",
    };
    let added_config = write_klog_config(harness, &added_ood, &added_options)?;
    spawn_klog(harness, &klog_daemon_bin, &added_config, &added_ood)?;
    wait_tcp("127.0.0.1", added_ood.ports.admin, Duration::from_secs(12)).await?;
    wait_tcp("127.0.0.1", added_ood.ports.inter, Duration::from_secs(12)).await?;

    let leader_id = wait_consistent_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let leader = base_voters
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| format!("leader node {} not found before add third OOD", leader_id))?;
    post_add_learner_via_admin_route(
        &client,
        gateway_addr(leader, ingress_port).as_str(),
        route_prefix,
        leader.name.as_str(),
        &added_ood,
        true,
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2],
        &[3],
        Duration::from_secs(60),
    )
    .await?;

    let promote_leader = change_voters_via_current_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        true,
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(70),
    )
    .await?;
    require_log_and_meta_roundtrip(
        &client,
        ingress_port,
        route_prefix,
        &added_ood,
        &base_voters[0],
        &nodes,
        "three-voters-after-add-third",
    )
    .await?;

    let demote_leader = change_voters_via_current_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2],
        true,
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2],
        &[3],
        Duration::from_secs(70),
    )
    .await?;

    let leader_id = wait_consistent_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let leader = base_voters
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| {
            format!(
                "leader node {} not found before remove third OOD",
                leader_id
            )
        })?;
    let (status_remove, body_remove) = post_remove_learner_via_admin_route(
        &client,
        gateway_addr(leader, ingress_port).as_str(),
        route_prefix,
        leader.name.as_str(),
        added_ood.id,
    )
    .await?;
    if !status_remove.is_success() {
        return Err(format!(
            "remove third OOD learner returned status={}, body={}",
            status_remove, body_remove
        ));
    }
    wait_membership(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        &[1, 2],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    harness.stop(format!("klog-{}", added_ood.name).as_str())?;
    require_log_and_meta_roundtrip(
        &client,
        ingress_port,
        route_prefix,
        &base_voters[1],
        &base_voters[0],
        &base_voters,
        "two-voters-after-remove-third",
    )
    .await?;

    println!(
        "[klog-cluster-dv] ood membership 2<->3 ok: promote_leader={}, demote_leader={}, removed_ood={}",
        promote_leader, demote_leader, added_ood.name
    );
    Ok(())
}

async fn run_local_gateway_ood_membership() -> Result<(), String> {
    {
        let mut harness = LocalHarness::new()?;
        let result = run_local_gateway_ood_membership_three_to_four(&mut harness).await;
        if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
            harness.keep_temp = true;
            eprintln!(
                "[klog-cluster-dv] keeping temp root for diagnostics: {}",
                harness.root.display()
            );
        }
        result?;
    }

    {
        let mut harness = LocalHarness::new()?;
        let result = run_local_gateway_ood_membership_two_to_three(&mut harness).await;
        if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
            harness.keep_temp = true;
            eprintln!(
                "[klog-cluster-dv] keeping temp root for diagnostics: {}",
                harness.root.display()
            );
        }
        result?;
    }

    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_ood_membership_one_to_two(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_ood_leader_failover_shrink_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-ood-leader-failover-shrink-dv";
    let setup =
        prepare_local_gateway_setup(harness, OOD_LEADER_FAILOVER_SHRINK_MODE, route_prefix, 3)
            .await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let seed = nodes
        .first()
        .cloned()
        .ok_or_else(|| "missing seed node".to_string())?;
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };

    for node in &nodes {
        let config = write_klog_config(harness, node, &voter_config)?;
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        wait_tcp("127.0.0.1", node.ports.inter, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let before_failover = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &nodes[0],
        &nodes[1],
        &nodes,
        "three-voters-before-leader-failover",
    )
    .await?;

    let old_leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let old_leader = nodes
        .iter()
        .find(|node| node.id == old_leader_id)
        .ok_or_else(|| format!("leader node {} not found", old_leader_id))?;
    harness.stop(format!("klog-{}", old_leader.name).as_str())?;
    let alive_nodes = nodes
        .iter()
        .filter(|node| node.id != old_leader_id)
        .cloned()
        .collect::<Vec<_>>();

    let new_leader_id = wait_consistent_leader(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        Some(old_leader_id),
        Duration::from_secs(70),
    )
    .await?;
    verify_log_and_meta_witness_on_nodes(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        &before_failover,
        Duration::from_secs(40),
    )
    .await?;

    let failover_writer = alive_nodes
        .iter()
        .find(|node| node.id != new_leader_id)
        .unwrap_or_else(|| alive_nodes.first().unwrap());
    let failover_target = alive_nodes
        .iter()
        .find(|node| node.id == new_leader_id)
        .unwrap_or(failover_writer);
    let after_failover = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        failover_writer,
        failover_target,
        &alive_nodes,
        "two-voters-after-leader-failover",
    )
    .await?;

    let alive_voters = alive_nodes.iter().map(|node| node.id).collect::<Vec<_>>();
    let shrink_leader_id = change_voters_via_current_leader(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        alive_voters.as_slice(),
        false,
    )
    .await?;
    wait_membership(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        alive_voters.as_slice(),
        &[],
        Duration::from_secs(80),
    )
    .await?;
    let stable_leader_id = wait_consistent_leader(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(50),
    )
    .await?;

    for witness in [&before_failover, &after_failover] {
        verify_log_and_meta_witness_on_nodes(
            &client,
            &alive_nodes,
            ingress_port,
            route_prefix,
            witness,
            Duration::from_secs(40),
        )
        .await?;
    }

    let post_shrink_writer = alive_nodes
        .iter()
        .find(|node| node.id != stable_leader_id)
        .unwrap_or_else(|| alive_nodes.first().unwrap());
    let post_shrink_target = alive_nodes
        .iter()
        .find(|node| node.id == stable_leader_id)
        .unwrap_or(post_shrink_writer);
    let after_shrink = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        post_shrink_writer,
        post_shrink_target,
        &alive_nodes,
        "two-voters-after-shrink",
    )
    .await?;

    for witness in [&before_failover, &after_failover, &after_shrink] {
        verify_log_and_meta_witness_on_nodes(
            &client,
            &alive_nodes,
            ingress_port,
            route_prefix,
            witness,
            Duration::from_secs(40),
        )
        .await?;
    }

    println!(
        "[klog-cluster-dv] ood leader failover shrink ok: old_leader={}, new_leader={}, shrink_leader={}, stable_leader={}, alive_voters={:?}, log_ids=[{},{},{}]",
        old_leader_id,
        new_leader_id,
        shrink_leader_id,
        stable_leader_id,
        alive_voters,
        before_failover.log_id,
        after_failover.log_id,
        after_shrink.log_id
    );
    Ok(())
}

async fn run_local_gateway_ood_leader_failover_shrink() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_ood_leader_failover_shrink_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_ood_seed_unavailable_join_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-ood-seed-unavailable-join-dv";
    let setup =
        prepare_local_gateway_setup(harness, OOD_SEED_UNAVAILABLE_JOIN_MODE, route_prefix, 4)
            .await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let base_voters = nodes.iter().take(3).cloned().collect::<Vec<_>>();
    let seed = base_voters
        .first()
        .cloned()
        .ok_or_else(|| "missing seed OOD node".to_string())?;
    let survivors = base_voters
        .iter()
        .filter(|node| node.id != seed.id)
        .cloned()
        .collect::<Vec<_>>();
    if survivors.len() != 2 {
        return Err(format!(
            "expected two survivor OOD nodes, got {}",
            survivors.len()
        ));
    }
    let added_ood = nodes
        .get(3)
        .cloned()
        .ok_or_else(|| "missing fourth OOD node".to_string())?;
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };

    for node in &base_voters {
        let config = write_klog_config(harness, node, &voter_config)?;
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        wait_tcp("127.0.0.1", node.ports.inter, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let before_seed_stop = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &seed,
        &survivors[0],
        &base_voters,
        "seed-unavailable-before-stop",
    )
    .await?;

    harness.stop(format!("klog-{}", seed.name).as_str())?;
    let survivor_leader = wait_consistent_leader(
        &client,
        &survivors,
        ingress_port,
        route_prefix,
        Some(seed.id),
        Duration::from_secs(70),
    )
    .await?;
    wait_membership(
        &client,
        &survivors,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(40),
    )
    .await?;
    verify_log_and_meta_witness_on_nodes(
        &client,
        &survivors,
        ingress_port,
        route_prefix,
        &before_seed_stop,
        Duration::from_secs(40),
    )
    .await?;
    let after_seed_stop = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &survivors[0],
        &survivors[1],
        &survivors,
        "seed-unavailable-after-stop",
    )
    .await?;

    let join_targets = base_voters
        .iter()
        .map(|target| gateway_admin_join_target(&added_ood, ingress_port, route_prefix, target))
        .collect::<Vec<_>>();
    println!(
        "[klog-cluster-dv] fourth OOD join_targets={}",
        join_targets.join(",")
    );
    let added_options = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: false,
        target_role: "voter",
    };
    let added_config =
        write_klog_config_with_join_targets(harness, &added_ood, &added_options, &join_targets)?;
    spawn_klog(harness, &klog_daemon_bin, &added_config, &added_ood)?;
    wait_tcp("127.0.0.1", added_ood.ports.admin, Duration::from_secs(12)).await?;
    wait_tcp("127.0.0.1", added_ood.ports.inter, Duration::from_secs(12)).await?;

    let online_after_join = survivors
        .iter()
        .cloned()
        .chain(std::iter::once(added_ood.clone()))
        .collect::<Vec<_>>();
    wait_membership(
        &client,
        &online_after_join,
        ingress_port,
        route_prefix,
        &[1, 2, 3, 4],
        &[],
        Duration::from_secs(90),
    )
    .await?;
    for witness in [&before_seed_stop, &after_seed_stop] {
        verify_log_and_meta_witness_on_nodes(
            &client,
            &online_after_join,
            ingress_port,
            route_prefix,
            witness,
            Duration::from_secs(50),
        )
        .await?;
    }
    let after_join = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &added_ood,
        &survivors[0],
        &online_after_join,
        "seed-unavailable-after-fourth-join",
    )
    .await?;

    println!(
        "[klog-cluster-dv] ood seed-unavailable auto-join ok: stopped_seed={}, survivor_leader={}, added_ood={}, log_ids=[{},{},{}]",
        seed.id,
        survivor_leader,
        added_ood.id,
        before_seed_stop.log_id,
        after_seed_stop.log_id,
        after_join.log_id
    );
    Ok(())
}

async fn run_local_gateway_ood_seed_unavailable_join() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_ood_seed_unavailable_join_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_ood_single_to_two_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-ood-single-to-two-dv";
    let setup =
        prepare_local_gateway_setup(harness, OOD_SINGLE_TO_TWO_MODE, route_prefix, 2).await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let seed = nodes
        .first()
        .cloned()
        .ok_or_else(|| "missing single OOD seed node".to_string())?;
    let added_ood = nodes
        .get(1)
        .cloned()
        .ok_or_else(|| "missing second OOD node".to_string())?;

    let seed_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };
    let config = write_klog_config(harness, &seed, &seed_config)?;
    spawn_klog(harness, &klog_daemon_bin, &config, &seed)?;
    wait_tcp("127.0.0.1", seed.ports.admin, Duration::from_secs(12)).await?;
    wait_tcp("127.0.0.1", seed.ports.inter, Duration::from_secs(12)).await?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        std::slice::from_ref(&seed),
        ingress_port,
        route_prefix,
        &[1],
        &[],
        Duration::from_secs(30),
    )
    .await?;
    let before_join = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &seed,
        &seed,
        std::slice::from_ref(&seed),
        "single-voter-before-learner-join",
    )
    .await?;

    let added_options = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: false,
        target_role: "learner",
    };
    let added_config = write_klog_config(harness, &added_ood, &added_options)?;
    spawn_klog(harness, &klog_daemon_bin, &added_config, &added_ood)?;
    wait_tcp("127.0.0.1", added_ood.ports.admin, Duration::from_secs(12)).await?;
    wait_tcp("127.0.0.1", added_ood.ports.inter, Duration::from_secs(12)).await?;
    post_add_learner_via_admin_route(
        &client,
        gateway_addr(&seed, ingress_port).as_str(),
        route_prefix,
        seed.name.as_str(),
        &added_ood,
        true,
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1],
        &[2],
        Duration::from_secs(60),
    )
    .await?;
    verify_log_and_meta_witness_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &before_join,
        Duration::from_secs(40),
    )
    .await?;

    let learner_phase = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &added_ood,
        &seed,
        &nodes,
        "single-voter-plus-learner",
    )
    .await?;

    let promote_leader = change_voters_via_current_leader(
        &client,
        std::slice::from_ref(&seed),
        ingress_port,
        route_prefix,
        &[1, 2],
        true,
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2],
        &[],
        Duration::from_secs(70),
    )
    .await?;
    for witness in [&before_join, &learner_phase] {
        verify_log_and_meta_witness_on_nodes(
            &client,
            &nodes,
            ingress_port,
            route_prefix,
            witness,
            Duration::from_secs(40),
        )
        .await?;
    }
    let post_promote = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &added_ood,
        &seed,
        &nodes,
        "two-voters-after-single-promote",
    )
    .await?;
    for witness in [&before_join, &learner_phase, &post_promote] {
        verify_log_and_meta_witness_on_nodes(
            &client,
            &nodes,
            ingress_port,
            route_prefix,
            witness,
            Duration::from_secs(40),
        )
        .await?;
    }

    println!(
        "[klog-cluster-dv] ood single-to-two ok: promote_leader={}, log_ids=[{},{},{}]",
        promote_leader, before_join.log_id, learner_phase.log_id, post_promote.log_id
    );
    Ok(())
}

async fn run_local_gateway_ood_single_to_two() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_ood_single_to_two_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_ood_two_voter_loss_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-ood-two-voter-loss-dv";
    let setup =
        prepare_local_gateway_setup(harness, OOD_TWO_VOTER_LOSS_MODE, route_prefix, 2).await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let seed = nodes
        .first()
        .cloned()
        .ok_or_else(|| "missing two-voter seed node".to_string())?;
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };

    for node in &nodes {
        let config = write_klog_config(harness, node, &voter_config)?;
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        wait_tcp("127.0.0.1", node.ports.inter, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let before_loss = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &nodes[0],
        &nodes[1],
        &nodes,
        "two-voters-before-loss",
    )
    .await?;

    let leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let leader = nodes
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| format!("leader node {} not found", leader_id))?;
    harness.stop(format!("klog-{}", leader.name).as_str())?;
    let survivor = nodes
        .iter()
        .find(|node| node.id != leader_id)
        .cloned()
        .ok_or_else(|| {
            format!(
                "survivor node not found after stopping leader {}",
                leader_id
            )
        })?;

    if wait_consistent_leader(
        &client,
        std::slice::from_ref(&survivor),
        ingress_port,
        route_prefix,
        Some(leader_id),
        Duration::from_secs(12),
    )
    .await
    .is_ok()
    {
        return Err(format!(
            "two-voter cluster unexpectedly elected a replacement leader after node {} stopped",
            leader_id
        ));
    }

    if append_via_cluster_inter_route(
        &client,
        gateway_addr(&survivor, ingress_port).as_str(),
        route_prefix,
        survivor.name.as_str(),
        "test/two-voter-loss-unavailable",
        "write should fail without two-voter quorum",
    )
    .await
    .is_ok()
    {
        return Err(format!(
            "surviving voter {} unexpectedly accepted append without quorum",
            survivor.id
        ));
    }

    if query_via_cluster_inter_route(
        &client,
        gateway_addr(&survivor, ingress_port).as_str(),
        route_prefix,
        survivor.name.as_str(),
        before_loss.log_id,
        before_loss.log_source.as_str(),
    )
    .await
    .is_ok()
    {
        return Err(format!(
            "surviving voter {} unexpectedly served strong log query without quorum",
            survivor.id
        ));
    }

    if put_meta_via_cluster_inter_route(
        &client,
        gateway_addr(&survivor, ingress_port).as_str(),
        route_prefix,
        survivor.name.as_str(),
        "test/two_voter_loss/unavailable_meta",
        "should-not-commit",
        Some(0),
    )
    .await
    .is_ok()
    {
        return Err(format!(
            "surviving voter {} unexpectedly accepted meta put without quorum",
            survivor.id
        ));
    }

    println!(
        "[klog-cluster-dv] ood two-voter loss ok: stopped_leader={}, survivor={}, pre_loss_log_id={}",
        leader_id, survivor.id, before_loss.log_id
    );
    Ok(())
}

async fn run_local_gateway_ood_two_voter_loss() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_ood_two_voter_loss_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

fn ood_snapshot_membership_raft_patch() -> KLogRaftPatch<'static> {
    KLogRaftPatch {
        install_snapshot_timeout_ms: Some(15_000),
        max_payload_entries: Some(16),
        replication_lag_threshold: Some(10),
        snapshot_policy: Some("since_last:25"),
        snapshot_max_chunk_size_bytes: Some(512 * 1024),
        max_in_snapshot_log_to_keep: Some(5),
        purge_batch_size: Some(50),
    }
}

fn raft_snapshot_install_crash_raft_patch(chunk_bytes: usize) -> KLogRaftPatch<'static> {
    KLogRaftPatch {
        install_snapshot_timeout_ms: Some(30_000),
        max_payload_entries: Some(8),
        replication_lag_threshold: Some(5),
        snapshot_policy: Some("since_last:20"),
        snapshot_max_chunk_size_bytes: Some(chunk_bytes as u64),
        max_in_snapshot_log_to_keep: Some(2),
        purge_batch_size: Some(20),
    }
}

async fn run_local_gateway_ood_snapshot_membership_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let item_count = parse_env_usize(
        ENV_OOD_SNAPSHOT_MEMBERSHIP_ITEMS,
        DEFAULT_OOD_SNAPSHOT_MEMBERSHIP_ITEMS,
    )?;
    let value_bytes = parse_env_usize(
        ENV_OOD_SNAPSHOT_MEMBERSHIP_VALUE_BYTES,
        DEFAULT_OOD_SNAPSHOT_MEMBERSHIP_VALUE_BYTES,
    )?;
    let route_prefix = "/.cluster/klog-it-ood-snapshot-membership-dv";
    let setup =
        prepare_local_gateway_setup(harness, OOD_SNAPSHOT_MEMBERSHIP_MODE, route_prefix, 3).await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let base_voters = nodes.iter().take(2).cloned().collect::<Vec<_>>();
    let seed = base_voters
        .first()
        .cloned()
        .ok_or_else(|| "missing snapshot membership seed node".to_string())?;
    let second = base_voters
        .get(1)
        .cloned()
        .ok_or_else(|| "missing second snapshot membership voter".to_string())?;
    let added_ood = nodes
        .get(2)
        .cloned()
        .ok_or_else(|| "missing snapshot membership third OOD".to_string())?;
    let raft_patch = ood_snapshot_membership_raft_patch();
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };

    for node in &base_voters {
        let config = write_klog_config_with_raft_patch(harness, node, &voter_config, raft_patch)?;
        spawn_klog_with_log_level(harness, &klog_daemon_bin, &config, node, "info")?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        wait_tcp("127.0.0.1", node.ports.inter, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        &[1, 2],
        &[],
        Duration::from_secs(50),
    )
    .await?;

    let pre_add_witness = write_snapshot_bulk_data(
        &client,
        ingress_port,
        route_prefix,
        &seed,
        &second,
        "pre-add",
        item_count,
        value_bytes,
    )
    .await?;
    verify_snapshot_bulk_witness(
        &client,
        ingress_port,
        route_prefix,
        &base_voters,
        &pre_add_witness,
        Duration::from_secs(40),
    )
    .await?;

    let leader_id = wait_consistent_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let snapshot_leader = base_voters
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| format!("snapshot leader {} not found", leader_id))?;
    let leader_snapshot_count =
        wait_snapshot_file_count(harness, snapshot_leader, 1, Duration::from_secs(70)).await?;

    let added_options = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: false,
        target_role: "learner",
    };
    let added_config =
        write_klog_config_with_raft_patch(harness, &added_ood, &added_options, raft_patch)?;
    spawn_klog_with_log_level(harness, &klog_daemon_bin, &added_config, &added_ood, "info")?;
    wait_tcp("127.0.0.1", added_ood.ports.admin, Duration::from_secs(12)).await?;
    wait_tcp("127.0.0.1", added_ood.ports.inter, Duration::from_secs(12)).await?;

    let leader = base_voters
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| format!("leader node {} not found before snapshot add", leader_id))?;
    post_add_learner_via_admin_route(
        &client,
        gateway_addr(leader, ingress_port).as_str(),
        route_prefix,
        leader.name.as_str(),
        &added_ood,
        false,
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2],
        &[3],
        Duration::from_secs(80),
    )
    .await?;
    let added_snapshot_count =
        wait_snapshot_file_count(harness, &added_ood, 1, Duration::from_secs(80)).await?;
    verify_snapshot_bulk_witness(
        &client,
        ingress_port,
        route_prefix,
        &nodes,
        &pre_add_witness,
        Duration::from_secs(60),
    )
    .await?;

    let promote_leader = change_voters_via_current_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        true,
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(80),
    )
    .await?;
    let post_promote_count = (item_count / 5).max(20);
    let post_promote_witness = write_snapshot_bulk_data(
        &client,
        ingress_port,
        route_prefix,
        &added_ood,
        &seed,
        "post-promote",
        post_promote_count,
        value_bytes,
    )
    .await?;
    verify_snapshot_bulk_witness(
        &client,
        ingress_port,
        route_prefix,
        &nodes,
        &post_promote_witness,
        Duration::from_secs(60),
    )
    .await?;

    let demote_leader = change_voters_via_current_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2],
        true,
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2],
        &[3],
        Duration::from_secs(80),
    )
    .await?;

    let remaining_voters = base_voters.clone();
    let leader_id = wait_consistent_leader(
        &client,
        &remaining_voters,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(50),
    )
    .await?;
    let leader = remaining_voters
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| format!("leader node {} not found before snapshot remove", leader_id))?;
    let (status_remove, body_remove) = post_remove_learner_via_admin_route(
        &client,
        gateway_addr(leader, ingress_port).as_str(),
        route_prefix,
        leader.name.as_str(),
        added_ood.id,
    )
    .await?;
    if !status_remove.is_success() {
        return Err(format!(
            "remove snapshot-added OOD learner returned status={}, body={}",
            status_remove, body_remove
        ));
    }
    wait_membership(
        &client,
        &remaining_voters,
        ingress_port,
        route_prefix,
        &[1, 2],
        &[],
        Duration::from_secs(70),
    )
    .await?;
    harness.stop(format!("klog-{}", added_ood.name).as_str())?;

    verify_snapshot_bulk_witness(
        &client,
        ingress_port,
        route_prefix,
        &remaining_voters,
        &pre_add_witness,
        Duration::from_secs(60),
    )
    .await?;
    verify_snapshot_bulk_witness(
        &client,
        ingress_port,
        route_prefix,
        &remaining_voters,
        &post_promote_witness,
        Duration::from_secs(60),
    )
    .await?;
    require_log_and_meta_roundtrip(
        &client,
        ingress_port,
        route_prefix,
        &seed,
        &second,
        &remaining_voters,
        "snapshot-two-voters-after-remove-added",
    )
    .await?;

    println!(
        "[klog-cluster-dv] ood snapshot membership ok: items={}, value_bytes={}, leader_snapshot_count={}, added_snapshot_count={}, promote_leader={}, demote_leader={}, removed_ood={}",
        item_count,
        value_bytes,
        leader_snapshot_count,
        added_snapshot_count,
        promote_leader,
        demote_leader,
        added_ood.name
    );
    Ok(())
}

async fn run_local_gateway_ood_snapshot_membership() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_ood_snapshot_membership_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

fn mvcc_snapshot_key(prefix: &str, index: usize) -> String {
    format!("{}key-{:04}", prefix, index)
}

fn mvcc_compact_snapshot_value(phase: &str, index: usize, value_bytes: usize) -> String {
    let label = format!("mvcc-compact-during-snapshot-{}", phase);
    fixed_payload(label.as_str(), index, value_bytes)
}

#[allow(clippy::too_many_arguments)]
async fn require_mvcc_snapshot_current_on_nodes(
    client: &reqwest::Client,
    nodes: &[LocalNodeDef],
    ingress_port: u16,
    route_prefix: &str,
    prefix: &str,
    expected_count: usize,
    expected_values: &[(&str, &str, u64, u64, u64)],
) -> Result<(), String> {
    for node in nodes {
        let response = query_meta_prefix_via_cluster_inter_route(
            client,
            gateway_addr(node, ingress_port).as_str(),
            route_prefix,
            node.name.as_str(),
            prefix,
            expected_count + 8,
        )
        .await?;
        if response.items.len() != expected_count {
            return Err(format!(
                "unexpected MVCC snapshot current count on {}: expected={}, actual={}, items={:?}",
                node.name,
                expected_count,
                response.items.len(),
                response.items
            ));
        }
        require_meta_selected_values(&response, expected_values)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn require_meta_at_revision_on_nodes(
    client: &reqwest::Client,
    nodes: &[LocalNodeDef],
    ingress_port: u16,
    route_prefix: &str,
    key: &str,
    revision: u64,
    expected_value: &str,
    expected_create_revision: u64,
    expected_version: u64,
) -> Result<(), String> {
    for node in nodes {
        let response = query_meta_at_revision_via_cluster_inter_route(
            client,
            gateway_addr(node, ingress_port).as_str(),
            route_prefix,
            node.name.as_str(),
            key,
            Some(revision),
        )
        .await?;
        require_meta_selected_values(
            &response,
            &[(
                key,
                expected_value,
                expected_create_revision,
                revision,
                expected_version,
            )],
        )?;
    }
    Ok(())
}

async fn require_mvcc_snapshot_key_absent_on_nodes(
    client: &reqwest::Client,
    nodes: &[LocalNodeDef],
    ingress_port: u16,
    route_prefix: &str,
    key: &str,
) -> Result<(), String> {
    for node in nodes {
        let response = query_meta_via_cluster_inter_route(
            client,
            gateway_addr(node, ingress_port).as_str(),
            route_prefix,
            node.name.as_str(),
            key,
        )
        .await?;
        if !response.items.is_empty() {
            return Err(format!(
                "deleted MVCC snapshot key visible on {}: key={}, items={:?}",
                node.name, key, response.items
            ));
        }
    }
    Ok(())
}

async fn run_local_gateway_mvcc_snapshot_membership_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let item_count = parse_env_usize(
        ENV_MVCC_SNAPSHOT_MEMBERSHIP_KEYS,
        DEFAULT_MVCC_SNAPSHOT_MEMBERSHIP_KEYS,
    )?;
    if item_count < 30 {
        return Err(format!(
            "{} must be at least 30 for MVCC snapshot membership coverage, got {}",
            ENV_MVCC_SNAPSHOT_MEMBERSHIP_KEYS, item_count
        ));
    }

    let route_prefix = "/.cluster/klog-it-mvcc-snapshot-membership-dv";
    let setup =
        prepare_local_gateway_setup(harness, MVCC_SNAPSHOT_MEMBERSHIP_MODE, route_prefix, 4)
            .await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let base_voters = nodes.iter().take(3).cloned().collect::<Vec<_>>();
    let seed = base_voters
        .first()
        .cloned()
        .ok_or_else(|| "missing MVCC snapshot seed node".to_string())?;
    let target = base_voters
        .get(1)
        .cloned()
        .ok_or_else(|| "missing MVCC snapshot target voter".to_string())?;
    let added_ood = nodes
        .get(3)
        .cloned()
        .ok_or_else(|| "missing MVCC snapshot added OOD".to_string())?;
    let raft_patch = ood_snapshot_membership_raft_patch();
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };

    for node in &base_voters {
        let config = write_klog_config_with_raft_patch(harness, node, &voter_config, raft_patch)?;
        spawn_klog_with_log_level(harness, &klog_daemon_bin, &config, node, "info")?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        wait_tcp("127.0.0.1", node.ports.inter, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let leader_id = wait_consistent_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(50),
    )
    .await?;
    let leader = base_voters
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| format!("MVCC snapshot leader {} not found", leader_id))?;
    let leader_gateway_addr = gateway_addr(leader, ingress_port);
    let seed_gateway_addr = gateway_addr(&seed, ingress_port);

    let run_id = unique_suffix("mvcc-snapshot-membership");
    let prefix = format!("test/klog_mvcc_snapshot_membership/{}/", run_id);
    let key0 = mvcc_snapshot_key(&prefix, 0);
    let key1 = mvcc_snapshot_key(&prefix, 1);
    let key2 = mvcc_snapshot_key(&prefix, 2);
    let key3 = mvcc_snapshot_key(&prefix, 3);
    let key4 = mvcc_snapshot_key(&prefix, 4);
    let key5 = mvcc_snapshot_key(&prefix, 5);
    let key10 = mvcc_snapshot_key(&prefix, 10);
    let key25 = mvcc_snapshot_key(&prefix, 25);
    let key_last = mvcc_snapshot_key(&prefix, item_count - 1);
    let key_last_value = format!("v1-{:04}", item_count - 1);

    let mut create_revisions = Vec::with_capacity(item_count);
    for index in 0..item_count {
        let key = mvcc_snapshot_key(&prefix, index);
        let value = format!("v1-{index:04}");
        let stored = put_meta_via_cluster_inter_route(
            &client,
            seed_gateway_addr.as_str(),
            route_prefix,
            target.name.as_str(),
            key.as_str(),
            value.as_str(),
            Some(0),
        )
        .await?;
        if stored.create_revision != stored.mod_revision || stored.version != 1 {
            return Err(format!(
                "unexpected initial MVCC put response: {:?}",
                stored
            ));
        }
        create_revisions.push(stored.mod_revision);
    }

    let mut update_revisions = BTreeMap::new();
    let update_count = (item_count / 3).max(12);
    for (index, create_revision) in create_revisions.iter().enumerate().take(update_count) {
        let key = mvcc_snapshot_key(&prefix, index);
        let value = format!("v2-{index:04}");
        let stored = put_meta_via_cluster_inter_route(
            &client,
            seed_gateway_addr.as_str(),
            route_prefix,
            target.name.as_str(),
            key.as_str(),
            value.as_str(),
            Some(*create_revision),
        )
        .await?;
        if stored.create_revision != *create_revision || stored.version != 2 {
            return Err(format!("unexpected MVCC update response: {:?}", stored));
        }
        update_revisions.insert(index, stored.mod_revision);
    }

    let delete_count = 10usize;
    let mut delete_revisions = BTreeMap::new();
    for (index, create_revision) in create_revisions.iter().enumerate().take(delete_count) {
        let key = mvcc_snapshot_key(&prefix, index);
        let deleted = delete_meta_via_cluster_inter_route(
            &client,
            seed_gateway_addr.as_str(),
            route_prefix,
            target.name.as_str(),
            key.as_str(),
        )
        .await?;
        let version = deleted
            .meta_version
            .as_ref()
            .ok_or_else(|| format!("missing delete meta_version: {:?}", deleted))?;
        if !version.deleted || version.version != 0 || version.create_revision != *create_revision {
            return Err(format!("unexpected MVCC delete response: {:?}", deleted));
        }
        delete_revisions.insert(index, version.mod_revision);
    }
    let compact_revision = *delete_revisions
        .get(&(delete_count - 1))
        .ok_or_else(|| "missing compact revision".to_string())?;

    let mut recreate_revisions = BTreeMap::new();
    for index in 0..5usize {
        let key = mvcc_snapshot_key(&prefix, index);
        let value = format!("v3-{index:04}");
        let stored = put_meta_via_cluster_inter_route(
            &client,
            seed_gateway_addr.as_str(),
            route_prefix,
            target.name.as_str(),
            key.as_str(),
            value.as_str(),
            Some(0),
        )
        .await?;
        if stored.create_revision != stored.mod_revision || stored.version != 1 {
            return Err(format!("unexpected MVCC recreate response: {:?}", stored));
        }
        recreate_revisions.insert(index, stored.mod_revision);
    }

    let current_revision = *recreate_revisions
        .get(&4)
        .ok_or_else(|| "missing recreated revision for key4".to_string())?;
    let compacted = post_meta_compact_via_admin_route(
        &client,
        leader_gateway_addr.as_str(),
        route_prefix,
        leader.name.as_str(),
        compact_revision,
    )
    .await?;
    if compacted.compacted_revision != compact_revision
        || compacted.current_revision < current_revision
    {
        return Err(format!(
            "unexpected MVCC snapshot compaction response: {:?}, expected_compacted={}, current>={}",
            compacted, compact_revision, current_revision
        ));
    }

    let leader_snapshot_count =
        wait_snapshot_file_count(harness, leader, 1, Duration::from_secs(80)).await?;
    let current_expected_count = item_count - 5;
    let key10_update_revision = *update_revisions
        .get(&10)
        .ok_or_else(|| "missing key10 update revision".to_string())?;

    require_mvcc_snapshot_current_on_nodes(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        prefix.as_str(),
        current_expected_count,
        &[
            (
                key0.as_str(),
                "v3-0000",
                *recreate_revisions.get(&0).unwrap(),
                *recreate_revisions.get(&0).unwrap(),
                1,
            ),
            (
                key10.as_str(),
                "v2-0010",
                create_revisions[10],
                key10_update_revision,
                2,
            ),
            (
                key25.as_str(),
                "v1-0025",
                create_revisions[25],
                create_revisions[25],
                1,
            ),
            (
                key_last.as_str(),
                key_last_value.as_str(),
                create_revisions[item_count - 1],
                create_revisions[item_count - 1],
                1,
            ),
        ],
    )
    .await?;
    require_mvcc_snapshot_key_absent_on_nodes(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        key5.as_str(),
    )
    .await?;

    let added_options = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: false,
        target_role: "learner",
    };
    let added_config =
        write_klog_config_with_raft_patch(harness, &added_ood, &added_options, raft_patch)?;
    spawn_klog_with_log_level(harness, &klog_daemon_bin, &added_config, &added_ood, "info")?;
    wait_tcp("127.0.0.1", added_ood.ports.admin, Duration::from_secs(12)).await?;
    wait_tcp("127.0.0.1", added_ood.ports.inter, Duration::from_secs(12)).await?;

    let leader_id = wait_consistent_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(50),
    )
    .await?;
    let leader = base_voters
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| {
            format!(
                "leader node {} not found before MVCC snapshot add",
                leader_id
            )
        })?;
    post_add_learner_via_admin_route(
        &client,
        gateway_addr(leader, ingress_port).as_str(),
        route_prefix,
        leader.name.as_str(),
        &added_ood,
        false,
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[4],
        Duration::from_secs(90),
    )
    .await?;
    let added_snapshot_count =
        wait_snapshot_file_count(harness, &added_ood, 1, Duration::from_secs(90)).await?;

    require_mvcc_snapshot_current_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        prefix.as_str(),
        current_expected_count,
        &[
            (
                key0.as_str(),
                "v3-0000",
                *recreate_revisions.get(&0).unwrap(),
                *recreate_revisions.get(&0).unwrap(),
                1,
            ),
            (
                key10.as_str(),
                "v2-0010",
                create_revisions[10],
                key10_update_revision,
                2,
            ),
            (
                key25.as_str(),
                "v1-0025",
                create_revisions[25],
                create_revisions[25],
                1,
            ),
        ],
    )
    .await?;
    require_mvcc_snapshot_key_absent_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        key5.as_str(),
    )
    .await?;
    expect_meta_query_status_via_cluster_inter_route(
        &client,
        gateway_addr(&added_ood, ingress_port).as_str(),
        route_prefix,
        added_ood.name.as_str(),
        Some(key10.as_str()),
        None,
        Some(create_revisions[0]),
        StatusCode::GONE,
        Some("COMPACTED"),
    )
    .await?;
    let added_changes = query_meta_changes_via_cluster_inter_route(
        &client,
        gateway_addr(&added_ood, ingress_port).as_str(),
        route_prefix,
        added_ood.name.as_str(),
        prefix.as_str(),
        compact_revision + 1,
        8,
        None,
    )
    .await?;
    require_meta_changes(
        &added_changes,
        &[
            (
                *recreate_revisions.get(&0).unwrap(),
                key0.as_str(),
                "v3-0000",
                false,
                *recreate_revisions.get(&0).unwrap(),
                1,
            ),
            (
                *recreate_revisions.get(&1).unwrap(),
                key1.as_str(),
                "v3-0001",
                false,
                *recreate_revisions.get(&1).unwrap(),
                1,
            ),
            (
                *recreate_revisions.get(&2).unwrap(),
                key2.as_str(),
                "v3-0002",
                false,
                *recreate_revisions.get(&2).unwrap(),
                1,
            ),
            (
                *recreate_revisions.get(&3).unwrap(),
                key3.as_str(),
                "v3-0003",
                false,
                *recreate_revisions.get(&3).unwrap(),
                1,
            ),
            (
                *recreate_revisions.get(&4).unwrap(),
                key4.as_str(),
                "v3-0004",
                false,
                *recreate_revisions.get(&4).unwrap(),
                1,
            ),
        ],
    )?;

    let promote_leader = change_voters_via_current_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        &[1, 2, 3, 4],
        true,
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3, 4],
        &[],
        Duration::from_secs(90),
    )
    .await?;

    let post_promote_key = format!("{}post-promote", prefix);
    let post_promote_tx = exec_meta_tx_via_cluster_inter_route(
        &client,
        gateway_addr(&added_ood, ingress_port).as_str(),
        route_prefix,
        seed.name.as_str(),
        BTreeMap::from([
            (
                key10.clone(),
                meta_tx_put_action(
                    key10.as_str(),
                    "v3-0010",
                    added_ood.name.as_str(),
                    Some(key10_update_revision),
                ),
            ),
            (
                post_promote_key.clone(),
                meta_tx_put_action(
                    post_promote_key.as_str(),
                    "post-promote-value",
                    added_ood.name.as_str(),
                    Some(0),
                ),
            ),
        ]),
    )
    .await?;
    let post_promote_revision = post_promote_tx
        .revisions
        .get(&key10)
        .and_then(|revision| *revision)
        .ok_or_else(|| format!("missing post-promote revision for {}", key10))?;
    if post_promote_tx
        .revisions
        .get(&post_promote_key)
        .and_then(|revision| *revision)
        != Some(post_promote_revision)
    {
        return Err(format!(
            "post-promote MVCC tx did not share revision: {:?}",
            post_promote_tx
        ));
    }
    require_mvcc_snapshot_current_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        prefix.as_str(),
        current_expected_count + 1,
        &[
            (
                key10.as_str(),
                "v3-0010",
                create_revisions[10],
                post_promote_revision,
                3,
            ),
            (
                post_promote_key.as_str(),
                "post-promote-value",
                post_promote_revision,
                post_promote_revision,
                1,
            ),
        ],
    )
    .await?;

    let demote_leader = change_voters_via_current_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        true,
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[4],
        Duration::from_secs(90),
    )
    .await?;
    let leader_id = wait_consistent_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(60),
    )
    .await?;
    let leader = base_voters
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| {
            format!(
                "leader node {} not found before MVCC snapshot remove",
                leader_id
            )
        })?;
    let (status_remove, body_remove) = post_remove_learner_via_admin_route(
        &client,
        gateway_addr(leader, ingress_port).as_str(),
        route_prefix,
        leader.name.as_str(),
        added_ood.id,
    )
    .await?;
    if !status_remove.is_success() {
        return Err(format!(
            "remove MVCC snapshot-added OOD learner returned status={}, body={}",
            status_remove, body_remove
        ));
    }
    wait_membership(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(80),
    )
    .await?;
    harness.stop(format!("klog-{}", added_ood.name).as_str())?;

    require_mvcc_snapshot_current_on_nodes(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        prefix.as_str(),
        current_expected_count + 1,
        &[
            (
                key10.as_str(),
                "v3-0010",
                create_revisions[10],
                post_promote_revision,
                3,
            ),
            (
                post_promote_key.as_str(),
                "post-promote-value",
                post_promote_revision,
                post_promote_revision,
                1,
            ),
        ],
    )
    .await?;
    expect_meta_changes_status_via_cluster_inter_route(
        &client,
        seed_gateway_addr.as_str(),
        route_prefix,
        seed.name.as_str(),
        prefix.as_str(),
        create_revisions[0],
        StatusCode::GONE,
        Some("COMPACTED"),
    )
    .await?;

    println!(
        "[klog-cluster-dv] MVCC snapshot membership ok: items={}, current_count={}, leader_snapshot_count={}, added_snapshot_count={}, promote_leader={}, demote_leader={}, removed_ood={}, compacted={}, post_promote_revision={}",
        item_count,
        current_expected_count + 1,
        leader_snapshot_count,
        added_snapshot_count,
        promote_leader,
        demote_leader,
        added_ood.name,
        compact_revision,
        post_promote_revision
    );
    Ok(())
}

async fn run_local_gateway_mvcc_snapshot_membership() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_mvcc_snapshot_membership_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_mvcc_compact_during_snapshot_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let item_count = parse_env_usize(
        ENV_MVCC_COMPACT_DURING_SNAPSHOT_KEYS,
        DEFAULT_MVCC_COMPACT_DURING_SNAPSHOT_KEYS,
    )?;
    let value_bytes = parse_env_usize(
        ENV_MVCC_COMPACT_DURING_SNAPSHOT_VALUE_BYTES,
        DEFAULT_MVCC_COMPACT_DURING_SNAPSHOT_VALUE_BYTES,
    )?;
    let chunk_bytes = parse_env_usize(
        ENV_MVCC_COMPACT_DURING_SNAPSHOT_CHUNK_BYTES,
        DEFAULT_MVCC_COMPACT_DURING_SNAPSHOT_CHUNK_BYTES,
    )?;
    if item_count < 30 {
        return Err(format!(
            "{} must be at least 30 for compact-during-snapshot coverage, got {}",
            ENV_MVCC_COMPACT_DURING_SNAPSHOT_KEYS, item_count
        ));
    }
    if value_bytes == 0 || chunk_bytes == 0 {
        return Err(format!(
            "{}={} and {}={} must both be greater than 0",
            ENV_MVCC_COMPACT_DURING_SNAPSHOT_VALUE_BYTES,
            value_bytes,
            ENV_MVCC_COMPACT_DURING_SNAPSHOT_CHUNK_BYTES,
            chunk_bytes
        ));
    }

    let route_prefix = "/.cluster/klog-it-mvcc-compact-during-snapshot-dv";
    let setup =
        prepare_local_gateway_setup(harness, MVCC_COMPACT_DURING_SNAPSHOT_MODE, route_prefix, 4)
            .await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let base_voters = nodes.iter().take(3).cloned().collect::<Vec<_>>();
    let seed = base_voters
        .first()
        .cloned()
        .ok_or_else(|| "missing MVCC compact snapshot seed node".to_string())?;
    let target = base_voters
        .get(1)
        .cloned()
        .ok_or_else(|| "missing MVCC compact snapshot target node".to_string())?;
    let learner = nodes
        .get(3)
        .cloned()
        .ok_or_else(|| "missing MVCC compact snapshot learner node".to_string())?;
    let raft_patch = raft_snapshot_install_crash_raft_patch(chunk_bytes);
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };
    for node in &base_voters {
        let config = write_klog_config_with_raft_patch(harness, node, &voter_config, raft_patch)?;
        spawn_klog_with_log_level(harness, &klog_daemon_bin, &config, node, "info")?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        wait_tcp("127.0.0.1", node.ports.inter, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let seed_gateway_addr = gateway_addr(&seed, ingress_port);
    let run_id = unique_suffix("mvcc-compact-during-snapshot");
    let prefix = format!("test/klog_mvcc_compact_during_snapshot/{}/", run_id);
    let key0 = mvcc_snapshot_key(&prefix, 0);
    let key1 = mvcc_snapshot_key(&prefix, 1);
    let key2 = mvcc_snapshot_key(&prefix, 2);
    let key3 = mvcc_snapshot_key(&prefix, 3);
    let key4 = mvcc_snapshot_key(&prefix, 4);
    let key5 = mvcc_snapshot_key(&prefix, 5);
    let key10 = mvcc_snapshot_key(&prefix, 10);
    let key_last = mvcc_snapshot_key(&prefix, item_count - 1);

    let mut create_revisions = Vec::with_capacity(item_count);
    for index in 0..item_count {
        let key = mvcc_snapshot_key(&prefix, index);
        let value = mvcc_compact_snapshot_value("v1", index, value_bytes);
        let stored = put_meta_via_cluster_inter_route(
            &client,
            seed_gateway_addr.as_str(),
            route_prefix,
            target.name.as_str(),
            key.as_str(),
            value.as_str(),
            Some(0),
        )
        .await?;
        if stored.create_revision != stored.mod_revision || stored.version != 1 {
            return Err(format!(
                "unexpected compact snapshot initial put response: {:?}",
                stored
            ));
        }
        create_revisions.push(stored.mod_revision);
    }

    let update_count = (item_count / 3).max(12);
    let mut update_revisions = BTreeMap::new();
    for (index, create_revision) in create_revisions.iter().enumerate().take(update_count) {
        let key = mvcc_snapshot_key(&prefix, index);
        let value = mvcc_compact_snapshot_value("v2", index, value_bytes);
        let stored = put_meta_via_cluster_inter_route(
            &client,
            seed_gateway_addr.as_str(),
            route_prefix,
            target.name.as_str(),
            key.as_str(),
            value.as_str(),
            Some(*create_revision),
        )
        .await?;
        if stored.create_revision != *create_revision || stored.version != 2 {
            return Err(format!(
                "unexpected compact snapshot update response: {:?}",
                stored
            ));
        }
        update_revisions.insert(index, stored.mod_revision);
    }

    let delete_count = 10usize;
    let mut delete_revisions = BTreeMap::new();
    for (index, create_revision) in create_revisions.iter().enumerate().take(delete_count) {
        let key = mvcc_snapshot_key(&prefix, index);
        let deleted = delete_meta_via_cluster_inter_route(
            &client,
            seed_gateway_addr.as_str(),
            route_prefix,
            target.name.as_str(),
            key.as_str(),
        )
        .await?;
        let version = deleted
            .meta_version
            .as_ref()
            .ok_or_else(|| format!("missing compact snapshot delete version: {:?}", deleted))?;
        if !version.deleted || version.version != 0 || version.create_revision != *create_revision {
            return Err(format!(
                "unexpected compact snapshot delete response: {:?}",
                deleted
            ));
        }
        delete_revisions.insert(index, version.mod_revision);
    }
    let compact_revision = *delete_revisions
        .get(&(delete_count - 1))
        .ok_or_else(|| "missing compact snapshot compact revision".to_string())?;

    let mut recreate_revisions = BTreeMap::new();
    for index in 0..5usize {
        let key = mvcc_snapshot_key(&prefix, index);
        let value = mvcc_compact_snapshot_value("v3", index, value_bytes);
        let stored = put_meta_via_cluster_inter_route(
            &client,
            seed_gateway_addr.as_str(),
            route_prefix,
            target.name.as_str(),
            key.as_str(),
            value.as_str(),
            Some(0),
        )
        .await?;
        if stored.create_revision != stored.mod_revision || stored.version != 1 {
            return Err(format!(
                "unexpected compact snapshot recreate response: {:?}",
                stored
            ));
        }
        recreate_revisions.insert(index, stored.mod_revision);
    }
    let current_revision = *recreate_revisions
        .get(&4)
        .ok_or_else(|| "missing compact snapshot current revision".to_string())?;

    let snapshot_leader_id = wait_consistent_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(50),
    )
    .await?;
    let snapshot_leader = base_voters
        .iter()
        .find(|node| node.id == snapshot_leader_id)
        .ok_or_else(|| format!("snapshot leader node {} not found", snapshot_leader_id))?;
    let leader_snapshot_count =
        wait_snapshot_file_count(harness, snapshot_leader, 1, Duration::from_secs(100)).await?;

    let learner_options = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: false,
        target_role: "learner",
    };
    let learner_config =
        write_klog_config_with_raft_patch(harness, &learner, &learner_options, raft_patch)?;
    spawn_klog_with_log_level(harness, &klog_daemon_bin, &learner_config, &learner, "info")?;
    wait_tcp("127.0.0.1", learner.ports.admin, Duration::from_secs(12)).await?;
    wait_tcp("127.0.0.1", learner.ports.inter, Duration::from_secs(12)).await?;

    let add_leader_id = wait_consistent_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(50),
    )
    .await?;
    let add_leader = base_voters
        .iter()
        .find(|node| node.id == add_leader_id)
        .ok_or_else(|| format!("add-learner leader node {} not found", add_leader_id))?;
    post_add_learner_via_admin_route(
        &client,
        gateway_addr(add_leader, ingress_port).as_str(),
        route_prefix,
        add_leader.name.as_str(),
        &learner,
        false,
    )
    .await?;
    wait_membership(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[4],
        Duration::from_secs(80),
    )
    .await?;
    let temp_bytes =
        wait_snapshot_temp_file_exists(harness, &learner, Duration::from_secs(120)).await?;
    let learner_snapshot_count_before_compact = snapshot_file_count(harness, &learner)?;

    let compact_leader_id = wait_consistent_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(50),
    )
    .await?;
    let compact_leader = base_voters
        .iter()
        .find(|node| node.id == compact_leader_id)
        .ok_or_else(|| format!("compact leader node {} not found", compact_leader_id))?;
    let compacted = post_meta_compact_via_admin_route(
        &client,
        gateway_addr(compact_leader, ingress_port).as_str(),
        route_prefix,
        compact_leader.name.as_str(),
        compact_revision,
    )
    .await?;
    if compacted.compacted_revision != compact_revision
        || compacted.current_revision < current_revision
    {
        return Err(format!(
            "unexpected compact-during-snapshot response: {:?}, expected_compacted={}, current>={}",
            compacted, compact_revision, current_revision
        ));
    }

    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[4],
        Duration::from_secs(160),
    )
    .await?;
    let learner_snapshot_count =
        wait_snapshot_file_count(harness, &learner, 1, Duration::from_secs(160)).await?;
    wait_meta_query_compacted_via_cluster_inter_route(
        &client,
        gateway_addr(&learner, ingress_port).as_str(),
        route_prefix,
        learner.name.as_str(),
        Some(key5.as_str()),
        None,
        create_revisions[5],
        Duration::from_secs(120),
    )
    .await?;

    let key0_v3 = mvcc_compact_snapshot_value("v3", 0, value_bytes);
    let key1_v3 = mvcc_compact_snapshot_value("v3", 1, value_bytes);
    let key2_v3 = mvcc_compact_snapshot_value("v3", 2, value_bytes);
    let key3_v3 = mvcc_compact_snapshot_value("v3", 3, value_bytes);
    let key4_v3 = mvcc_compact_snapshot_value("v3", 4, value_bytes);
    let key10_v2 = mvcc_compact_snapshot_value("v2", 10, value_bytes);
    let key_last_v1 = mvcc_compact_snapshot_value("v1", item_count - 1, value_bytes);
    let current_expected_count = item_count - 5;
    let key10_update_revision = *update_revisions
        .get(&10)
        .ok_or_else(|| "missing compact snapshot key10 update revision".to_string())?;
    require_mvcc_snapshot_current_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        prefix.as_str(),
        current_expected_count,
        &[
            (
                key0.as_str(),
                key0_v3.as_str(),
                *recreate_revisions.get(&0).unwrap(),
                *recreate_revisions.get(&0).unwrap(),
                1,
            ),
            (
                key10.as_str(),
                key10_v2.as_str(),
                create_revisions[10],
                key10_update_revision,
                2,
            ),
            (
                key_last.as_str(),
                key_last_v1.as_str(),
                create_revisions[item_count - 1],
                create_revisions[item_count - 1],
                1,
            ),
        ],
    )
    .await?;
    require_mvcc_snapshot_key_absent_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        key5.as_str(),
    )
    .await?;
    require_meta_at_revision_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        key0.as_str(),
        *recreate_revisions.get(&0).unwrap(),
        key0_v3.as_str(),
        *recreate_revisions.get(&0).unwrap(),
        1,
    )
    .await?;

    for node in &nodes {
        let node_gateway_addr = gateway_addr(node, ingress_port);
        expect_meta_query_status_via_cluster_inter_route(
            &client,
            node_gateway_addr.as_str(),
            route_prefix,
            node.name.as_str(),
            Some(key5.as_str()),
            None,
            Some(create_revisions[5]),
            StatusCode::GONE,
            Some("COMPACTED"),
        )
        .await?;
        expect_meta_changes_status_via_cluster_inter_route(
            &client,
            node_gateway_addr.as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            create_revisions[0],
            StatusCode::GONE,
            Some("COMPACTED"),
        )
        .await?;
        let changes = query_meta_changes_via_cluster_inter_route(
            &client,
            node_gateway_addr.as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            compact_revision + 1,
            8,
            None,
        )
        .await?;
        require_meta_changes(
            &changes,
            &[
                (
                    *recreate_revisions.get(&0).unwrap(),
                    key0.as_str(),
                    key0_v3.as_str(),
                    false,
                    *recreate_revisions.get(&0).unwrap(),
                    1,
                ),
                (
                    *recreate_revisions.get(&1).unwrap(),
                    key1.as_str(),
                    key1_v3.as_str(),
                    false,
                    *recreate_revisions.get(&1).unwrap(),
                    1,
                ),
                (
                    *recreate_revisions.get(&2).unwrap(),
                    key2.as_str(),
                    key2_v3.as_str(),
                    false,
                    *recreate_revisions.get(&2).unwrap(),
                    1,
                ),
                (
                    *recreate_revisions.get(&3).unwrap(),
                    key3.as_str(),
                    key3_v3.as_str(),
                    false,
                    *recreate_revisions.get(&3).unwrap(),
                    1,
                ),
                (
                    *recreate_revisions.get(&4).unwrap(),
                    key4.as_str(),
                    key4_v3.as_str(),
                    false,
                    *recreate_revisions.get(&4).unwrap(),
                    1,
                ),
            ],
        )?;
    }

    let post_key = format!("{}post-recovery", prefix);
    let post_value = mvcc_compact_snapshot_value("post", 0, value_bytes);
    let post_tx = exec_meta_tx_via_cluster_inter_route(
        &client,
        gateway_addr(&learner, ingress_port).as_str(),
        route_prefix,
        seed.name.as_str(),
        BTreeMap::from([
            (
                key10.clone(),
                meta_tx_put_action(
                    key10.as_str(),
                    key10_v2.as_str(),
                    learner.name.as_str(),
                    Some(key10_update_revision),
                ),
            ),
            (
                post_key.clone(),
                meta_tx_put_action(
                    post_key.as_str(),
                    post_value.as_str(),
                    learner.name.as_str(),
                    Some(0),
                ),
            ),
        ]),
    )
    .await?;
    let post_revision = post_tx
        .revisions
        .get(&post_key)
        .and_then(|revision| *revision)
        .ok_or_else(|| format!("missing compact snapshot post revision for {}", post_key))?;
    require_mvcc_snapshot_current_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        prefix.as_str(),
        current_expected_count + 1,
        &[
            (
                post_key.as_str(),
                post_value.as_str(),
                post_revision,
                post_revision,
                1,
            ),
            (
                key10.as_str(),
                key10_v2.as_str(),
                create_revisions[10],
                post_revision,
                3,
            ),
        ],
    )
    .await?;

    println!(
        "[klog-cluster-dv] MVCC compact during snapshot ok: items={}, value_bytes={}, chunk_bytes={}, add_leader={}, compact_leader={}, snapshot_leader={}, leader_snapshots={}, learner_snapshots_before_compact={}, learner_snapshots_after={}, temp_bytes_before_compact={}, compacted={}, current_revision={}, post_revision={}, prefix={}",
        item_count,
        value_bytes,
        chunk_bytes,
        add_leader_id,
        compact_leader_id,
        snapshot_leader_id,
        leader_snapshot_count,
        learner_snapshot_count_before_compact,
        learner_snapshot_count,
        temp_bytes,
        compact_revision,
        compacted.current_revision,
        post_revision,
        prefix
    );
    Ok(())
}

async fn run_local_gateway_mvcc_compact_during_snapshot() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_mvcc_compact_during_snapshot_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_restart_recovery_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-restart-dv";
    let setup =
        prepare_local_gateway_setup(harness, RESTART_RECOVERY_MODE, route_prefix, 3).await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let seed = nodes
        .first()
        .ok_or_else(|| "missing seed node".to_string())?
        .clone();
    let mut configs = BTreeMap::new();
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };

    for node in &nodes {
        let config = write_klog_config(harness, node, &voter_config)?;
        configs.insert(node.id, config.clone());
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(50),
    )
    .await?;
    let leader_before = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let leader_node = nodes
        .iter()
        .find(|node| node.id == leader_before)
        .ok_or_else(|| format!("leader node {} not found", leader_before))?;
    let leader_gateway_addr_before = gateway_addr(leader_node, ingress_port);
    let suffix = unique_suffix("restart");
    let source = format!("test/test_klog_restart_recovery_dv-{}", suffix);
    let first = append_via_cluster_inter_route(
        &client,
        leader_gateway_addr_before.as_str(),
        route_prefix,
        leader_node.name.as_str(),
        source.as_str(),
        "restart recovery write before full stop 1",
    )
    .await?;
    let second = append_via_cluster_inter_route(
        &client,
        leader_gateway_addr_before.as_str(),
        route_prefix,
        leader_node.name.as_str(),
        source.as_str(),
        "restart recovery write before full stop 2",
    )
    .await?;
    if second.id <= first.id {
        return Err(format!(
            "append id not increasing before restart: first_id={}, second_id={}",
            first.id, second.id
        ));
    }
    let meta_key = format!("test/test_klog_restart_recovery_dv/meta/{}", suffix);
    let meta_before = put_meta_via_cluster_inter_route(
        &client,
        leader_gateway_addr_before.as_str(),
        route_prefix,
        leader_node.name.as_str(),
        meta_key.as_str(),
        "before-restart",
        Some(0),
    )
    .await?;
    if meta_before.key != meta_key || meta_before.revision != 1 {
        return Err(format!(
            "unexpected meta before restart: key={}, revision={}",
            meta_before.key, meta_before.revision
        ));
    }

    for node in &nodes {
        harness.stop(format!("klog-{}", node.name).as_str())?;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;

    for node_id in [2_u64, 3_u64, 1_u64] {
        let node = nodes
            .iter()
            .find(|node| node.id == node_id)
            .ok_or_else(|| format!("restart node {} not found", node_id))?;
        let config = configs
            .get(&node_id)
            .ok_or_else(|| format!("restart config for node {} not found", node_id))?;
        spawn_klog(harness, &klog_daemon_bin, config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
    }

    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(90),
    )
    .await?;
    let leader_after = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(60),
    )
    .await?;
    let leader_node = nodes
        .iter()
        .find(|node| node.id == leader_after)
        .ok_or_else(|| format!("post-restart leader node {} not found", leader_after))?;
    let leader_gateway_addr_after = gateway_addr(leader_node, ingress_port);

    for log_id in [first.id, second.id] {
        let response = query_via_cluster_inter_route(
            &client,
            leader_gateway_addr_after.as_str(),
            route_prefix,
            leader_node.name.as_str(),
            log_id,
            source.as_str(),
        )
        .await?;
        require_query_match(&response, log_id, source.as_str())?;
    }

    let meta_after_restart = query_meta_via_cluster_inter_route(
        &client,
        leader_gateway_addr_after.as_str(),
        route_prefix,
        leader_node.name.as_str(),
        meta_key.as_str(),
    )
    .await?;
    if meta_after_restart.items.len() != 1
        || meta_after_restart.items[0].key != meta_key
        || meta_after_restart.items[0].value != "before-restart"
        || meta_after_restart.items[0].revision != 1
    {
        return Err(format!(
            "unexpected meta after restart: items={:?}",
            meta_after_restart
                .items
                .iter()
                .map(|item| format!(
                    "key={}, value={}, revision={}",
                    item.key, item.value, item.revision
                ))
                .collect::<Vec<_>>()
        ));
    }

    let after = append_via_cluster_inter_route(
        &client,
        leader_gateway_addr_after.as_str(),
        route_prefix,
        leader_node.name.as_str(),
        source.as_str(),
        "restart recovery write after full restart",
    )
    .await?;
    let response = query_via_cluster_inter_route(
        &client,
        leader_gateway_addr_after.as_str(),
        route_prefix,
        leader_node.name.as_str(),
        after.id,
        source.as_str(),
    )
    .await?;
    require_query_match(&response, after.id, source.as_str())?;

    let meta_after_update = put_meta_via_cluster_inter_route(
        &client,
        leader_gateway_addr_after.as_str(),
        route_prefix,
        leader_node.name.as_str(),
        meta_key.as_str(),
        "after-restart",
        Some(1),
    )
    .await?;
    if meta_after_update.revision != 2 {
        return Err(format!(
            "unexpected meta revision after restart update: expected=2, got={}",
            meta_after_update.revision
        ));
    }
    println!(
        "[klog-cluster-dv] restart recovery ok: leader_before={}, leader_after={}, log_ids=[{}, {}, {}], meta_revision={}",
        leader_before, leader_after, first.id, second.id, after.id, meta_after_update.revision
    );
    println!("[klog-cluster-dv] local gateway restart recovery DV success");
    Ok(())
}

async fn run_local_gateway_restart_recovery() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_restart_recovery_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_mvcc_cluster_inner(harness: &mut LocalHarness) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-mvcc-cluster-dv";
    let setup = prepare_local_gateway_setup(harness, MVCC_CLUSTER_MODE, route_prefix, 3).await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let seed = nodes
        .first()
        .ok_or_else(|| "missing seed node".to_string())?
        .clone();
    let mut configs = BTreeMap::new();
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };

    for node in &nodes {
        let config = write_klog_config(harness, node, &voter_config)?;
        configs.insert(node.id, config.clone());
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(50),
    )
    .await?;
    let leader_before = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;

    let source = nodes
        .first()
        .ok_or_else(|| "missing source node".to_string())?;
    let target = nodes
        .get(1)
        .ok_or_else(|| "missing target node".to_string())?;
    let observer = nodes
        .get(2)
        .ok_or_else(|| "missing observer node".to_string())?;
    let source_gateway_addr = gateway_addr(source, ingress_port);
    let target_gateway_addr = gateway_addr(target, ingress_port);
    let observer_gateway_addr = gateway_addr(observer, ingress_port);
    let suffix = unique_suffix("mvcc-cluster");
    let prefix = format!("test/klog_mvcc_cluster_dv/{}/", suffix);
    let key_a = format!("{}a", prefix);
    let key_b = format!("{}b", prefix);
    let key_c = format!("{}c", prefix);
    let key_d = format!("{}d", prefix);

    let tx1 = exec_meta_tx_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        BTreeMap::from([
            (
                key_a.clone(),
                meta_tx_put_action(&key_a, "a-v1", target.name.as_str(), Some(0)),
            ),
            (
                key_b.clone(),
                meta_tx_put_action(&key_b, "b-v1", target.name.as_str(), Some(0)),
            ),
        ]),
    )
    .await?;
    let r1 = tx1
        .revisions
        .get(&key_a)
        .and_then(|revision| *revision)
        .ok_or_else(|| format!("missing tx1 revision for {}", key_a))?;
    if tx1.revisions.get(&key_b).and_then(|revision| *revision) != Some(r1) {
        return Err(format!("tx1 keys did not share revision: {:?}", tx1));
    }
    require_meta_version(tx1.meta_versions.get(&key_a), r1, r1, 1, false)?;
    require_meta_version(tx1.meta_versions.get(&key_b), r1, r1, 1, false)?;

    let a_v2 = put_meta_via_cluster_inter_route(
        &client,
        observer_gateway_addr.as_str(),
        route_prefix,
        source.name.as_str(),
        key_a.as_str(),
        "a-v2",
        Some(r1),
    )
    .await?;
    let r2 = a_v2.mod_revision;
    if a_v2.create_revision != r1 || a_v2.version != 2 || r2 != r1 + 1 {
        return Err(format!("unexpected a_v2 MVCC response: {:?}", a_v2));
    }

    let deleted_b = delete_meta_via_cluster_inter_route(
        &client,
        target_gateway_addr.as_str(),
        route_prefix,
        observer.name.as_str(),
        key_b.as_str(),
    )
    .await?;
    if deleted_b.key != key_b
        || !deleted_b.existed
        || deleted_b.prev_meta.as_ref().map(|item| item.revision) != Some(r1)
    {
        return Err(format!("unexpected key_b delete response: {:?}", deleted_b));
    }
    let delete_version = deleted_b
        .meta_version
        .as_ref()
        .ok_or_else(|| format!("missing key_b delete meta_version: {:?}", deleted_b))?;
    require_meta_version(Some(delete_version), r1, r2 + 1, 0, true)?;
    let r3 = delete_version.mod_revision;

    expect_meta_put_status_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        MetaPutRequest {
            key: key_b.clone(),
            value: "stale-b".to_string(),
            node_name: Some(target.name.clone()),
            expected_revision: Some(r1),
        },
        StatusCode::CONFLICT,
    )
    .await?;

    let b_v2 = put_meta_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        observer.name.as_str(),
        key_b.as_str(),
        "b-v2",
        Some(0),
    )
    .await?;
    let r4 = b_v2.mod_revision;
    if b_v2.create_revision != r4 || b_v2.version != 1 || r4 != r3 + 1 {
        return Err(format!("unexpected b_v2 MVCC response: {:?}", b_v2));
    }

    let tx5 = exec_meta_tx_via_cluster_inter_route(
        &client,
        observer_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        BTreeMap::from([
            (
                key_a.clone(),
                meta_tx_put_action(&key_a, "a-v3", target.name.as_str(), Some(r2)),
            ),
            (
                key_c.clone(),
                meta_tx_put_action(&key_c, "c-v1", target.name.as_str(), Some(0)),
            ),
            (
                key_d.clone(),
                meta_tx_put_action(&key_d, "d-v1", target.name.as_str(), Some(0)),
            ),
        ]),
    )
    .await?;
    let r5 = tx5
        .revisions
        .get(&key_a)
        .and_then(|revision| *revision)
        .ok_or_else(|| format!("missing tx5 revision for {}", key_a))?;
    if r5 != r4 + 1 {
        return Err(format!("unexpected tx5 revision: r4={}, r5={}", r4, r5));
    }
    require_meta_version(tx5.meta_versions.get(&key_a), r1, r5, 3, false)?;
    require_meta_version(tx5.meta_versions.get(&key_c), r5, r5, 1, false)?;
    require_meta_version(tx5.meta_versions.get(&key_d), r5, r5, 1, false)?;

    for node in &nodes {
        let gateway = gateway_addr(node, ingress_port);
        let rev1 = query_meta_prefix_page_at_revision_via_cluster_inter_route(
            &client,
            gateway.as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            16,
            None,
            Some(r1),
        )
        .await?;
        require_meta_values(
            &rev1,
            &[(&key_a, "a-v1", r1, r1, 1), (&key_b, "b-v1", r1, r1, 1)],
        )?;

        let rev3 = query_meta_prefix_page_at_revision_via_cluster_inter_route(
            &client,
            gateway.as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            16,
            None,
            Some(r3),
        )
        .await?;
        require_meta_values(&rev3, &[(&key_a, "a-v2", r1, r2, 2)])?;

        let current = query_meta_prefix_via_cluster_inter_route(
            &client,
            gateway.as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            16,
        )
        .await?;
        require_meta_values(
            &current,
            &[
                (&key_a, "a-v3", r1, r5, 3),
                (&key_b, "b-v2", r4, r4, 1),
                (&key_c, "c-v1", r5, r5, 1),
                (&key_d, "d-v1", r5, r5, 1),
            ],
        )?;
    }

    let page1 = query_meta_changes_via_cluster_inter_route(
        &client,
        observer_gateway_addr.as_str(),
        route_prefix,
        source.name.as_str(),
        prefix.as_str(),
        r1,
        4,
        None,
    )
    .await?;
    if !page1.has_more || page1.next_cursor.is_none() || page1.current_revision < r5 {
        return Err(format!(
            "unexpected first changes page metadata: {:?}",
            page1
        ));
    }
    require_meta_changes(
        &page1,
        &[
            (r1, &key_a, "a-v1", false, r1, 1),
            (r1, &key_b, "b-v1", false, r1, 1),
            (r2, &key_a, "a-v2", false, r1, 2),
            (r3, &key_b, "b-v1", true, r1, 0),
        ],
    )?;

    let page2 = query_meta_changes_via_cluster_inter_route(
        &client,
        observer_gateway_addr.as_str(),
        route_prefix,
        source.name.as_str(),
        prefix.as_str(),
        r1,
        4,
        page1.next_cursor.as_ref(),
    )
    .await?;
    if page2.has_more || page2.next_start_revision <= r5 {
        return Err(format!(
            "unexpected second changes page metadata: {:?}",
            page2
        ));
    }
    require_meta_changes(
        &page2,
        &[
            (r4, &key_b, "b-v2", false, r4, 1),
            (r5, &key_a, "a-v3", false, r1, 3),
            (r5, &key_c, "c-v1", false, r5, 1),
            (r5, &key_d, "d-v1", false, r5, 1),
        ],
    )?;

    let leader_node = nodes
        .iter()
        .find(|node| node.id == leader_before)
        .ok_or_else(|| format!("leader node {} not found", leader_before))?;
    let leader_gateway_addr = gateway_addr(leader_node, ingress_port);
    let compacted = post_meta_compact_via_admin_route(
        &client,
        leader_gateway_addr.as_str(),
        route_prefix,
        leader_node.name.as_str(),
        r4,
    )
    .await?;
    if compacted.compacted_revision != r4 || compacted.current_revision != r5 {
        return Err(format!(
            "unexpected compaction response: {:?}, expected compacted={}, current={}",
            compacted, r4, r5
        ));
    }

    expect_meta_query_status_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        observer.name.as_str(),
        None,
        Some(prefix.as_str()),
        Some(r1),
        StatusCode::GONE,
        Some("COMPACTED"),
    )
    .await?;
    expect_meta_changes_status_via_cluster_inter_route(
        &client,
        target_gateway_addr.as_str(),
        route_prefix,
        source.name.as_str(),
        prefix.as_str(),
        r1,
        StatusCode::GONE,
        Some("COMPACTED"),
    )
    .await?;

    let post_compact_changes = query_meta_changes_via_cluster_inter_route(
        &client,
        target_gateway_addr.as_str(),
        route_prefix,
        observer.name.as_str(),
        prefix.as_str(),
        r4 + 1,
        8,
        None,
    )
    .await?;
    require_meta_changes(
        &post_compact_changes,
        &[
            (r5, &key_a, "a-v3", false, r1, 3),
            (r5, &key_c, "c-v1", false, r5, 1),
            (r5, &key_d, "d-v1", false, r5, 1),
        ],
    )?;

    for node in &nodes {
        harness.stop(format!("klog-{}", node.name).as_str())?;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;

    for node in &nodes {
        let config = configs
            .get(&node.id)
            .ok_or_else(|| format!("restart config for node {} not found", node.id))?;
        spawn_klog(harness, &klog_daemon_bin, config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
    }
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(90),
    )
    .await?;
    let leader_after = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(60),
    )
    .await?;

    for node in &nodes {
        let gateway = gateway_addr(node, ingress_port);
        let current = query_meta_prefix_via_cluster_inter_route(
            &client,
            gateway.as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            16,
        )
        .await?;
        require_meta_values(
            &current,
            &[
                (&key_a, "a-v3", r1, r5, 3),
                (&key_b, "b-v2", r4, r4, 1),
                (&key_c, "c-v1", r5, r5, 1),
                (&key_d, "d-v1", r5, r5, 1),
            ],
        )?;
    }

    let restarted_observer_gateway = gateway_addr(observer, ingress_port);
    expect_meta_query_status_via_cluster_inter_route(
        &client,
        restarted_observer_gateway.as_str(),
        route_prefix,
        observer.name.as_str(),
        Some(key_a.as_str()),
        None,
        Some(r4),
        StatusCode::GONE,
        Some("COMPACTED"),
    )
    .await?;
    let after_restart_changes = query_meta_changes_via_cluster_inter_route(
        &client,
        restarted_observer_gateway.as_str(),
        route_prefix,
        observer.name.as_str(),
        prefix.as_str(),
        r5,
        8,
        None,
    )
    .await?;
    require_meta_changes(
        &after_restart_changes,
        &[
            (r5, &key_a, "a-v3", false, r1, 3),
            (r5, &key_c, "c-v1", false, r5, 1),
            (r5, &key_d, "d-v1", false, r5, 1),
        ],
    )?;

    println!(
        "[klog-cluster-dv] mvcc cluster ok: leader_before={}, leader_after={}, revisions=[{},{},{},{},{}], prefix={}",
        leader_before, leader_after, r1, r2, r3, r4, r5, prefix
    );
    Ok(())
}

async fn run_local_gateway_mvcc_cluster() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_mvcc_cluster_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_mvcc_change_feed_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-mvcc-change-feed-dv";
    let setup =
        prepare_local_gateway_setup(harness, MVCC_CHANGE_FEED_MODE, route_prefix, 3).await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let seed = nodes
        .first()
        .cloned()
        .ok_or_else(|| "missing seed node".to_string())?;
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };

    for node in &nodes {
        let config = write_klog_config(harness, node, &voter_config)?;
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(50),
    )
    .await?;
    let leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let leader = nodes
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| format!("leader node {} not found", leader_id))?;
    let source = nodes
        .first()
        .ok_or_else(|| "missing source node".to_string())?;
    let target = nodes
        .iter()
        .find(|node| node.name != source.name)
        .ok_or_else(|| "missing target node".to_string())?;
    let observer = nodes
        .iter()
        .find(|node| node.name != source.name && node.name != target.name)
        .unwrap_or(source);
    let source_gateway_addr = gateway_addr(source, ingress_port);
    let target_gateway_addr = gateway_addr(target, ingress_port);
    let observer_gateway_addr = gateway_addr(observer, ingress_port);
    let suffix = unique_suffix("mvcc-change-feed");
    let prefix = format!("test/klog_mvcc_change_feed_dv/{}/", suffix);
    let key_a = format!("{}a", prefix);
    let key_b = format!("{}b", prefix);

    let empty_started = std::time::Instant::now();
    let empty = query_meta_changes_with_wait_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        prefix.as_str(),
        1,
        8,
        None,
        Some(350),
    )
    .await?;
    let empty_elapsed = empty_started.elapsed();
    if !empty.items.is_empty()
        || empty.has_more
        || empty.next_cursor.is_some()
        || empty.next_start_revision != 1
    {
        return Err(format!(
            "unexpected empty long-poll response: elapsed_ms={}, response={:?}",
            empty_elapsed.as_millis(),
            empty
        ));
    }
    if empty_elapsed < Duration::from_millis(200) {
        return Err(format!(
            "empty long-poll returned too early: elapsed_ms={}, response={:?}",
            empty_elapsed.as_millis(),
            empty
        ));
    }

    let waiter_client = client.clone();
    let waiter_gateway_addr = observer_gateway_addr.clone();
    let waiter_route_prefix = route_prefix.to_string();
    let waiter_node_name = observer.name.clone();
    let waiter_prefix = prefix.clone();
    let wait_started = std::time::Instant::now();
    let wait_task = tokio::spawn(async move {
        query_meta_changes_with_wait_via_cluster_inter_route(
            &waiter_client,
            waiter_gateway_addr.as_str(),
            waiter_route_prefix.as_str(),
            waiter_node_name.as_str(),
            waiter_prefix.as_str(),
            1,
            8,
            None,
            Some(1_500),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(250)).await;
    let a_v1 = put_meta_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        key_a.as_str(),
        "a-v1",
        Some(0),
    )
    .await?;
    let r1 = a_v1.mod_revision;
    let waited = wait_task
        .await
        .map_err(|err| format!("long-poll change-feed task join failed: {}", err))??;
    let wait_elapsed = wait_started.elapsed();
    if wait_elapsed >= Duration::from_millis(1_450) {
        return Err(format!(
            "long-poll did not return promptly after write: elapsed_ms={}, response={:?}",
            wait_elapsed.as_millis(),
            waited
        ));
    }
    require_meta_changes(&waited, &[(r1, &key_a, "a-v1", false, r1, 1)])?;
    if waited.next_start_revision != r1 + 1 || waited.current_revision < r1 {
        return Err(format!(
            "unexpected long-poll next revision after write: response={:?}",
            waited
        ));
    }

    let b_v1 = put_meta_via_cluster_inter_route(
        &client,
        target_gateway_addr.as_str(),
        route_prefix,
        observer.name.as_str(),
        key_b.as_str(),
        "b-v1",
        Some(0),
    )
    .await?;
    let r2 = b_v1.mod_revision;
    if r2 != r1 + 1 {
        return Err(format!("unexpected key_b revision: r1={}, r2={}", r1, r2));
    }
    let deleted_a = delete_meta_via_cluster_inter_route(
        &client,
        observer_gateway_addr.as_str(),
        route_prefix,
        source.name.as_str(),
        key_a.as_str(),
    )
    .await?;
    let delete_version = deleted_a
        .meta_version
        .as_ref()
        .ok_or_else(|| format!("missing key_a delete meta_version: {:?}", deleted_a))?;
    require_meta_version(Some(delete_version), r1, r2 + 1, 0, true)?;
    let r3 = delete_version.mod_revision;
    let a_v2 = put_meta_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        key_a.as_str(),
        "a-v2",
        Some(0),
    )
    .await?;
    let r4 = a_v2.mod_revision;
    if a_v2.create_revision != r4 || a_v2.version != 1 || r4 != r3 + 1 {
        return Err(format!("unexpected key_a recreate response: {:?}", a_v2));
    }

    let page1 = query_meta_changes_via_cluster_inter_route(
        &client,
        target_gateway_addr.as_str(),
        route_prefix,
        observer.name.as_str(),
        prefix.as_str(),
        r1,
        1,
        None,
    )
    .await?;
    if !page1.has_more || page1.next_cursor.is_none() {
        return Err(format!(
            "change-feed cursor page did not return cursor: {:?}",
            page1
        ));
    }
    require_meta_changes(&page1, &[(r1, &key_a, "a-v1", false, r1, 1)])?;
    let resume_cursor = page1
        .next_cursor
        .clone()
        .ok_or_else(|| "missing change-feed resume cursor".to_string())?;

    let compacted = post_meta_compact_via_admin_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        leader.name.as_str(),
        r1,
    )
    .await?;
    if compacted.compacted_revision != r1 || compacted.current_revision != r4 {
        return Err(format!(
            "unexpected change-feed compaction response: {:?}, expected compacted={}, current={}",
            compacted, r1, r4
        ));
    }

    expect_meta_changes_status_with_options_via_cluster_inter_route(
        &client,
        target_gateway_addr.as_str(),
        route_prefix,
        observer.name.as_str(),
        prefix.as_str(),
        r1,
        Some(&resume_cursor),
        Some(500),
        StatusCode::GONE,
        Some("COMPACTED"),
    )
    .await?;
    expect_meta_changes_status_with_options_via_cluster_inter_route(
        &client,
        observer_gateway_addr.as_str(),
        route_prefix,
        source.name.as_str(),
        prefix.as_str(),
        r1,
        None,
        Some(500),
        StatusCode::GONE,
        Some("COMPACTED"),
    )
    .await?;

    let post_compact_changes = query_meta_changes_with_wait_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        prefix.as_str(),
        r2,
        8,
        None,
        Some(500),
    )
    .await?;
    require_meta_changes(
        &post_compact_changes,
        &[
            (r2, &key_b, "b-v1", false, r2, 1),
            (r3, &key_a, "a-v1", true, r1, 0),
            (r4, &key_a, "a-v2", false, r4, 1),
        ],
    )?;
    if post_compact_changes.next_start_revision != r4 + 1 {
        return Err(format!(
            "unexpected post-compact next_start_revision: {:?}",
            post_compact_changes
        ));
    }

    let after_current_started = std::time::Instant::now();
    let after_current = query_meta_changes_with_wait_via_cluster_inter_route(
        &client,
        target_gateway_addr.as_str(),
        route_prefix,
        observer.name.as_str(),
        prefix.as_str(),
        r4 + 1,
        8,
        None,
        Some(350),
    )
    .await?;
    let after_current_elapsed = after_current_started.elapsed();
    if !after_current.items.is_empty()
        || after_current.next_start_revision != r4 + 1
        || after_current_elapsed < Duration::from_millis(200)
    {
        return Err(format!(
            "unexpected post-current empty long-poll: elapsed_ms={}, response={:?}",
            after_current_elapsed.as_millis(),
            after_current
        ));
    }

    for node in &nodes {
        let gateway = gateway_addr(node, ingress_port);
        let current = query_meta_prefix_via_cluster_inter_route(
            &client,
            gateway.as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            8,
        )
        .await?;
        require_meta_values(
            &current,
            &[(&key_a, "a-v2", r4, r4, 1), (&key_b, "b-v1", r2, r2, 1)],
        )?;
    }

    println!(
        "[klog-cluster-dv] MVCC change-feed long-poll ok: leader={}, empty_wait_ms={}, wake_wait_ms={}, revisions=[{},{},{},{}], prefix={}",
        leader_id,
        empty_elapsed.as_millis(),
        wait_elapsed.as_millis(),
        r1,
        r2,
        r3,
        r4,
        prefix
    );
    Ok(())
}

async fn run_local_gateway_mvcc_change_feed() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_mvcc_change_feed_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_mvcc_change_feed_failover_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-mvcc-change-feed-failover-dv";
    let setup =
        prepare_local_gateway_setup(harness, MVCC_CHANGE_FEED_FAILOVER_MODE, route_prefix, 3)
            .await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let seed = nodes
        .first()
        .cloned()
        .ok_or_else(|| "missing seed node".to_string())?;
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };
    let mut configs = BTreeMap::new();
    for node in &nodes {
        let config = write_klog_config(harness, node, &voter_config)?;
        configs.insert(node.id, config.clone());
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(50),
    )
    .await?;
    let old_leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let old_leader = nodes
        .iter()
        .find(|node| node.id == old_leader_id)
        .cloned()
        .ok_or_else(|| format!("old leader node {} not found", old_leader_id))?;
    let source = nodes
        .iter()
        .find(|node| node.id != old_leader_id)
        .ok_or_else(|| format!("missing non-leader source node: leader={}", old_leader_id))?;
    let source_gateway_addr = gateway_addr(source, ingress_port);
    let old_leader_gateway_addr = gateway_addr(&old_leader, ingress_port);
    let suffix = unique_suffix("mvcc-change-feed-failover");
    let prefix = format!("test/klog_mvcc_change_feed_failover_dv/{}/", suffix);
    let key_a = format!("{}a", prefix);
    let key_b = format!("{}b", prefix);
    let key_c = format!("{}c", prefix);

    let a_v1 = put_meta_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        old_leader.name.as_str(),
        key_a.as_str(),
        "a-v1",
        Some(0),
    )
    .await?;
    let r1 = a_v1.mod_revision;
    if a_v1.create_revision != r1 || a_v1.version != 1 {
        return Err(format!(
            "unexpected change-feed failover a_v1 response: {:?}",
            a_v1
        ));
    }

    let waiter_client = client.clone();
    let waiter_gateway_addr = old_leader_gateway_addr.clone();
    let waiter_route_prefix = route_prefix.to_string();
    let waiter_node_name = old_leader.name.clone();
    let waiter_prefix = prefix.clone();
    let wait_started = std::time::Instant::now();
    let wait_task = tokio::spawn(async move {
        query_meta_changes_with_wait_via_cluster_inter_route(
            &waiter_client,
            waiter_gateway_addr.as_str(),
            waiter_route_prefix.as_str(),
            waiter_node_name.as_str(),
            waiter_prefix.as_str(),
            r1 + 1,
            8,
            None,
            Some(1_800),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(250)).await;
    harness.stop(format!("klog-{}", old_leader.name).as_str())?;

    let alive_nodes = nodes
        .iter()
        .filter(|node| node.id != old_leader_id)
        .cloned()
        .collect::<Vec<_>>();
    wait_membership(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let new_leader_id = wait_consistent_leader(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        Some(old_leader_id),
        Duration::from_secs(70),
    )
    .await?;
    let new_leader = alive_nodes
        .iter()
        .find(|node| node.id == new_leader_id)
        .ok_or_else(|| format!("new leader node {} not found", new_leader_id))?;
    let failover_writer = alive_nodes
        .iter()
        .find(|node| node.id != new_leader_id)
        .unwrap_or(new_leader);
    let new_leader_gateway_addr = gateway_addr(new_leader, ingress_port);
    let failover_writer_gateway_addr = gateway_addr(failover_writer, ingress_port);

    let b_v1 = put_meta_via_cluster_inter_route(
        &client,
        failover_writer_gateway_addr.as_str(),
        route_prefix,
        new_leader.name.as_str(),
        key_b.as_str(),
        "b-v1",
        Some(0),
    )
    .await?;
    let r2 = b_v1.mod_revision;
    if b_v1.create_revision != r2 || b_v1.version != 1 || r2 != r1 + 1 {
        return Err(format!(
            "unexpected change-feed failover b_v1 response: {:?}",
            b_v1
        ));
    }

    let wait_outcome = match wait_task
        .await
        .map_err(|err| format!("change-feed failover long-poll task join failed: {}", err))?
    {
        Ok(waited) => {
            require_meta_changes(&waited, &[(r2, &key_b, "b-v1", false, r2, 1)])?;
            "continued"
        }
        Err(err) => {
            let retried = query_meta_changes_with_wait_via_cluster_inter_route(
                &client,
                new_leader_gateway_addr.as_str(),
                route_prefix,
                failover_writer.name.as_str(),
                prefix.as_str(),
                r1 + 1,
                8,
                None,
                Some(1_500),
            )
            .await
            .map_err(|retry_err| {
                format!(
                    "long-poll failed during leader switch and resume also failed: initial={}, retry={}",
                    err, retry_err
                )
            })?;
            require_meta_changes(&retried, &[(r2, &key_b, "b-v1", false, r2, 1)])?;
            "resumed"
        }
    };
    let wait_elapsed = wait_started.elapsed();

    let cursor_page = query_meta_changes_via_cluster_inter_route(
        &client,
        new_leader_gateway_addr.as_str(),
        route_prefix,
        new_leader.name.as_str(),
        prefix.as_str(),
        r1,
        1,
        None,
    )
    .await?;
    if !cursor_page.has_more || cursor_page.next_cursor.is_none() {
        return Err(format!(
            "change-feed failover cursor page did not produce cursor: {:?}",
            cursor_page
        ));
    }
    require_meta_changes(&cursor_page, &[(r1, &key_a, "a-v1", false, r1, 1)])?;
    let compacted_cursor = cursor_page
        .next_cursor
        .clone()
        .ok_or_else(|| "missing compacted change-feed cursor".to_string())?;

    let compacted = post_meta_compact_via_admin_route(
        &client,
        new_leader_gateway_addr.as_str(),
        route_prefix,
        new_leader.name.as_str(),
        r1,
    )
    .await?;
    if compacted.compacted_revision != r1 || compacted.current_revision != r2 {
        return Err(format!(
            "unexpected change-feed failover compaction response: {:?}, expected compacted={}, current={}",
            compacted, r1, r2
        ));
    }

    expect_meta_changes_status_with_options_via_cluster_inter_route(
        &client,
        failover_writer_gateway_addr.as_str(),
        route_prefix,
        failover_writer.name.as_str(),
        prefix.as_str(),
        r1,
        Some(&compacted_cursor),
        Some(600),
        StatusCode::GONE,
        Some("COMPACTED"),
    )
    .await?;
    expect_meta_changes_status_with_options_via_cluster_inter_route(
        &client,
        new_leader_gateway_addr.as_str(),
        route_prefix,
        new_leader.name.as_str(),
        prefix.as_str(),
        r1,
        None,
        Some(600),
        StatusCode::GONE,
        Some("COMPACTED"),
    )
    .await?;

    let post_compact_client = client.clone();
    let post_compact_gateway_addr = failover_writer_gateway_addr.clone();
    let post_compact_route_prefix = route_prefix.to_string();
    let post_compact_node_name = failover_writer.name.clone();
    let post_compact_prefix = prefix.clone();
    let post_compact_wait_started = std::time::Instant::now();
    let post_compact_wait_task = tokio::spawn(async move {
        query_meta_changes_with_wait_via_cluster_inter_route(
            &post_compact_client,
            post_compact_gateway_addr.as_str(),
            post_compact_route_prefix.as_str(),
            post_compact_node_name.as_str(),
            post_compact_prefix.as_str(),
            r2 + 1,
            8,
            None,
            Some(1_800),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(250)).await;
    let c_v1 = put_meta_via_cluster_inter_route(
        &client,
        new_leader_gateway_addr.as_str(),
        route_prefix,
        failover_writer.name.as_str(),
        key_c.as_str(),
        "c-v1",
        Some(0),
    )
    .await?;
    let r3 = c_v1.mod_revision;
    if c_v1.create_revision != r3 || c_v1.version != 1 || r3 != r2 + 1 {
        return Err(format!(
            "unexpected change-feed failover c_v1 response: {:?}",
            c_v1
        ));
    }
    let post_compact_waited = post_compact_wait_task
        .await
        .map_err(|err| format!("post-compact long-poll task join failed: {}", err))??;
    let post_compact_wait_elapsed = post_compact_wait_started.elapsed();
    if post_compact_wait_elapsed >= Duration::from_millis(1_600) {
        return Err(format!(
            "post-compact long-poll did not return promptly after write: elapsed_ms={}, response={:?}",
            post_compact_wait_elapsed.as_millis(),
            post_compact_waited
        ));
    }
    require_meta_changes(&post_compact_waited, &[(r3, &key_c, "c-v1", false, r3, 1)])?;

    for node in &alive_nodes {
        let gateway = gateway_addr(node, ingress_port);
        expect_meta_changes_status_via_cluster_inter_route(
            &client,
            gateway.as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            r1,
            StatusCode::GONE,
            Some("COMPACTED"),
        )
        .await?;
        let changes = query_meta_changes_via_cluster_inter_route(
            &client,
            gateway.as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            r2,
            8,
            None,
        )
        .await?;
        require_meta_changes(
            &changes,
            &[
                (r2, &key_b, "b-v1", false, r2, 1),
                (r3, &key_c, "c-v1", false, r3, 1),
            ],
        )?;
        let current = query_meta_prefix_via_cluster_inter_route(
            &client,
            gateway.as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            8,
        )
        .await?;
        require_meta_values(
            &current,
            &[
                (&key_a, "a-v1", r1, r1, 1),
                (&key_b, "b-v1", r2, r2, 1),
                (&key_c, "c-v1", r3, r3, 1),
            ],
        )?;
    }

    let old_leader_config = configs
        .get(&old_leader_id)
        .ok_or_else(|| format!("missing config for old leader {}", old_leader_id))?;
    spawn_klog(harness, &klog_daemon_bin, old_leader_config, &old_leader)?;
    wait_tcp("127.0.0.1", old_leader.ports.admin, Duration::from_secs(12)).await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(90),
    )
    .await?;
    wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(80),
    )
    .await?;
    for node in &nodes {
        let gateway = gateway_addr(node, ingress_port);
        expect_meta_changes_status_via_cluster_inter_route(
            &client,
            gateway.as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            r1,
            StatusCode::GONE,
            Some("COMPACTED"),
        )
        .await?;
        let current = query_meta_prefix_via_cluster_inter_route(
            &client,
            gateway.as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            8,
        )
        .await?;
        require_meta_values(
            &current,
            &[
                (&key_a, "a-v1", r1, r1, 1),
                (&key_b, "b-v1", r2, r2, 1),
                (&key_c, "c-v1", r3, r3, 1),
            ],
        )?;
    }

    println!(
        "[klog-cluster-dv] MVCC change-feed failover ok: old_leader={}, new_leader={}, wait_outcome={}, wait_ms={}, post_compact_wait_ms={}, compacted={}, revisions=[{},{},{}], prefix={}",
        old_leader_id,
        new_leader_id,
        wait_outcome,
        wait_elapsed.as_millis(),
        post_compact_wait_elapsed.as_millis(),
        compacted.compacted_revision,
        r1,
        r2,
        r3,
        prefix
    );
    Ok(())
}

async fn run_local_gateway_mvcc_change_feed_failover() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_mvcc_change_feed_failover_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

#[derive(Debug, Clone)]
struct StressKeyState {
    key: String,
    value: String,
    create_revision: u64,
    mod_revision: u64,
    version: u64,
}

async fn run_local_gateway_mvcc_change_feed_stress_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let key_count = parse_env_usize(
        ENV_MVCC_CHANGE_FEED_STRESS_KEYS,
        DEFAULT_MVCC_CHANGE_FEED_STRESS_KEYS,
    )?;
    let concurrency = parse_env_usize(
        ENV_MVCC_CHANGE_FEED_STRESS_CONCURRENCY,
        DEFAULT_MVCC_CHANGE_FEED_STRESS_CONCURRENCY,
    )?;
    let rounds = parse_env_usize(
        ENV_MVCC_CHANGE_FEED_STRESS_ROUNDS,
        DEFAULT_MVCC_CHANGE_FEED_STRESS_ROUNDS,
    )?;
    let page_limit = parse_env_usize(
        ENV_MVCC_CHANGE_FEED_STRESS_PAGE_LIMIT,
        DEFAULT_MVCC_CHANGE_FEED_STRESS_PAGE_LIMIT,
    )?;
    let round_delay_ms = parse_env_usize(
        ENV_MVCC_CHANGE_FEED_STRESS_ROUND_DELAY_MS,
        DEFAULT_MVCC_CHANGE_FEED_STRESS_ROUND_DELAY_MS,
    )?;
    if key_count < 8 {
        return Err(format!(
            "{} must be at least 8, got {}",
            ENV_MVCC_CHANGE_FEED_STRESS_KEYS, key_count
        ));
    }
    if concurrency == 0 {
        return Err(format!(
            "{} must be greater than 0",
            ENV_MVCC_CHANGE_FEED_STRESS_CONCURRENCY
        ));
    }
    if rounds == 0 {
        return Err(format!(
            "{} must be greater than 0",
            ENV_MVCC_CHANGE_FEED_STRESS_ROUNDS
        ));
    }
    if page_limit < 2 {
        return Err(format!(
            "{} must be at least 2, got {}",
            ENV_MVCC_CHANGE_FEED_STRESS_PAGE_LIMIT, page_limit
        ));
    }

    let route_prefix = "/.cluster/klog-it-mvcc-change-feed-stress-dv";
    let setup =
        prepare_local_gateway_setup(harness, MVCC_CHANGE_FEED_STRESS_MODE, route_prefix, 3).await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let seed = nodes
        .first()
        .cloned()
        .ok_or_else(|| "missing seed node".to_string())?;
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };

    for node in &nodes {
        let config = write_klog_config(harness, node, &voter_config)?;
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(50),
    )
    .await?;
    let leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let leader = nodes
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| format!("leader node {} not found", leader_id))?;
    let suffix = unique_suffix("mvcc-change-feed-stress");
    let prefix = format!("test/klog_mvcc_change_feed_stress_dv/{}/", suffix);
    let stress_started = std::time::Instant::now();

    let mut expected_changes = Vec::new();
    let mut states: Vec<Option<StressKeyState>> = vec![None; key_count];
    for batch_start in (0..key_count).step_by(concurrency) {
        let batch_end = (batch_start + concurrency).min(key_count);
        let mut tasks = Vec::new();
        for index in batch_start..batch_end {
            let client = client.clone();
            let source = nodes[index % nodes.len()].clone();
            let target = nodes[(index + 1) % nodes.len()].clone();
            let gateway_addr = gateway_addr(&source, ingress_port);
            let route_prefix = route_prefix.to_string();
            let key = format!("{}key-{:04}", prefix, index);
            let value = format!("create-{:04}", index);
            tasks.push(tokio::spawn(async move {
                let stored = put_meta_via_cluster_inter_route(
                    &client,
                    gateway_addr.as_str(),
                    route_prefix.as_str(),
                    target.name.as_str(),
                    key.as_str(),
                    value.as_str(),
                    Some(0),
                )
                .await?;
                Ok::<_, String>((index, key, value, stored))
            }));
        }

        for task in tasks {
            let (index, key, value, stored) = task
                .await
                .map_err(|err| format!("stress create task join failed: {}", err))??;
            if stored.create_revision != stored.mod_revision || stored.version != 1 {
                return Err(format!("unexpected stress create response: {:?}", stored));
            }
            expected_changes.push(ExpectedMetaChange {
                revision: stored.mod_revision,
                key: key.clone(),
                value: value.clone(),
                deleted: false,
                create_revision: stored.create_revision,
                version: stored.version,
            });
            states[index] = Some(StressKeyState {
                key,
                value,
                create_revision: stored.create_revision,
                mod_revision: stored.mod_revision,
                version: stored.version,
            });
        }
    }
    let mut states = states
        .into_iter()
        .map(|state| state.ok_or_else(|| "missing stress key state after create".to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let first_revision = expected_changes
        .iter()
        .map(|change| change.revision)
        .min()
        .ok_or_else(|| "missing first stress revision".to_string())?;

    for round in 1..=rounds {
        for batch_start in (0..key_count).step_by(concurrency) {
            let batch_end = (batch_start + concurrency).min(key_count);
            let mut tasks = Vec::new();
            for (index, state) in states
                .iter()
                .enumerate()
                .skip(batch_start)
                .take(batch_end - batch_start)
            {
                let client = client.clone();
                let source = nodes[(index + round) % nodes.len()].clone();
                let target = nodes[(index + round + 1) % nodes.len()].clone();
                let gateway_addr = gateway_addr(&source, ingress_port);
                let route_prefix = route_prefix.to_string();
                let key = state.key.clone();
                let value = format!("round-{:02}-key-{:04}", round, index);
                let expected_revision = state.mod_revision;
                tasks.push(tokio::spawn(async move {
                    let stored = put_meta_via_cluster_inter_route(
                        &client,
                        gateway_addr.as_str(),
                        route_prefix.as_str(),
                        target.name.as_str(),
                        key.as_str(),
                        value.as_str(),
                        Some(expected_revision),
                    )
                    .await?;
                    Ok::<_, String>((index, value, stored))
                }));
            }

            for task in tasks {
                let (index, value, stored) = task
                    .await
                    .map_err(|err| format!("stress update task join failed: {}", err))??;
                let state = states
                    .get_mut(index)
                    .ok_or_else(|| format!("missing stress state for update index {}", index))?;
                if stored.create_revision != state.create_revision
                    || stored.version != state.version + 1
                {
                    return Err(format!(
                        "unexpected stress update response: state={:?}, stored={:?}",
                        state, stored
                    ));
                }
                state.value = value.clone();
                state.mod_revision = stored.mod_revision;
                state.version = stored.version;
                expected_changes.push(ExpectedMetaChange {
                    revision: stored.mod_revision,
                    key: state.key.clone(),
                    value,
                    deleted: false,
                    create_revision: state.create_revision,
                    version: state.version,
                });
            }
        }

        if round_delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(round_delay_ms as u64)).await;
        }
    }

    let delete_count = (key_count / 4).max(2);
    for batch_start in (0..delete_count).step_by(concurrency) {
        let batch_end = (batch_start + concurrency).min(delete_count);
        let mut tasks = Vec::new();
        for (index, state) in states
            .iter()
            .enumerate()
            .take(delete_count)
            .skip(batch_start)
            .take(batch_end - batch_start)
        {
            let client = client.clone();
            let source = nodes[(index + rounds + 1) % nodes.len()].clone();
            let target = nodes[(index + rounds + 2) % nodes.len()].clone();
            let gateway_addr = gateway_addr(&source, ingress_port);
            let route_prefix = route_prefix.to_string();
            let key = state.key.clone();
            tasks.push(tokio::spawn(async move {
                let deleted = delete_meta_via_cluster_inter_route(
                    &client,
                    gateway_addr.as_str(),
                    route_prefix.as_str(),
                    target.name.as_str(),
                    key.as_str(),
                )
                .await?;
                Ok::<_, String>((index, deleted))
            }));
        }

        for task in tasks {
            let (index, deleted) = task
                .await
                .map_err(|err| format!("stress delete task join failed: {}", err))??;
            let state = states
                .get(index)
                .ok_or_else(|| format!("missing stress state for delete index {}", index))?;
            let version = deleted
                .meta_version
                .as_ref()
                .ok_or_else(|| format!("missing stress delete meta_version: {:?}", deleted))?;
            require_meta_version(
                Some(version),
                state.create_revision,
                version.mod_revision,
                0,
                true,
            )?;
            expected_changes.push(ExpectedMetaChange {
                revision: version.mod_revision,
                key: state.key.clone(),
                value: state.value.clone(),
                deleted: true,
                create_revision: state.create_revision,
                version: 0,
            });
        }
    }

    for batch_start in (0..delete_count).step_by(concurrency) {
        let batch_end = (batch_start + concurrency).min(delete_count);
        let mut tasks = Vec::new();
        for (index, state) in states
            .iter()
            .enumerate()
            .take(delete_count)
            .skip(batch_start)
            .take(batch_end - batch_start)
        {
            let client = client.clone();
            let source = nodes[(index + rounds + 2) % nodes.len()].clone();
            let target = nodes[(index + rounds) % nodes.len()].clone();
            let gateway_addr = gateway_addr(&source, ingress_port);
            let route_prefix = route_prefix.to_string();
            let key = state.key.clone();
            let value = format!("recreate-key-{:04}", index);
            tasks.push(tokio::spawn(async move {
                let stored = put_meta_via_cluster_inter_route(
                    &client,
                    gateway_addr.as_str(),
                    route_prefix.as_str(),
                    target.name.as_str(),
                    key.as_str(),
                    value.as_str(),
                    Some(0),
                )
                .await?;
                Ok::<_, String>((index, value, stored))
            }));
        }

        for task in tasks {
            let (index, value, stored) = task
                .await
                .map_err(|err| format!("stress recreate task join failed: {}", err))??;
            if stored.create_revision != stored.mod_revision || stored.version != 1 {
                return Err(format!("unexpected stress recreate response: {:?}", stored));
            }
            let state = states
                .get_mut(index)
                .ok_or_else(|| format!("missing stress state for recreate index {}", index))?;
            state.value = value.clone();
            state.create_revision = stored.create_revision;
            state.mod_revision = stored.mod_revision;
            state.version = stored.version;
            expected_changes.push(ExpectedMetaChange {
                revision: stored.mod_revision,
                key: state.key.clone(),
                value,
                deleted: false,
                create_revision: state.create_revision,
                version: state.version,
            });
        }
    }

    expected_changes.sort_by(|left, right| {
        left.revision
            .cmp(&right.revision)
            .then_with(|| left.key.cmp(&right.key))
    });

    let source = nodes
        .first()
        .ok_or_else(|| "missing stress source node".to_string())?;
    let target = nodes
        .get(1)
        .ok_or_else(|| "missing stress target node".to_string())?;
    let source_gateway_addr = gateway_addr(source, ingress_port);
    let target_gateway_addr = gateway_addr(target, ingress_port);
    let mut cursor = None;
    let mut page_sizes = Vec::new();
    let mut collected = Vec::with_capacity(expected_changes.len());
    loop {
        let page = query_meta_changes_via_cluster_inter_route(
            &client,
            source_gateway_addr.as_str(),
            route_prefix,
            target.name.as_str(),
            prefix.as_str(),
            first_revision,
            page_limit,
            cursor.as_ref(),
        )
        .await?;
        if page.items.is_empty() && page.has_more {
            return Err(format!(
                "stress change-feed returned empty page with has_more=true: {:?}",
                page
            ));
        }
        page_sizes.push(page.items.len());
        collected.extend(page.items.iter().cloned());
        if !page.has_more {
            if page.next_start_revision
                <= expected_changes
                    .last()
                    .ok_or_else(|| "missing last expected change".to_string())?
                    .revision
            {
                return Err(format!(
                    "stress change-feed next_start_revision did not advance: page={:?}",
                    page
                ));
            }
            break;
        }
        let next_cursor = page
            .next_cursor
            .ok_or_else(|| "stress change-feed missing next_cursor".to_string())?;
        if cursor.as_ref() == Some(&next_cursor) {
            return Err(format!(
                "stress change-feed cursor did not advance: {:?}",
                next_cursor
            ));
        }
        cursor = Some(next_cursor);
        if page_sizes.len() > expected_changes.len() + 2 {
            return Err(format!(
                "stress change-feed pagination exceeded expected pages: sizes={:?}",
                page_sizes
            ));
        }
    }
    require_expected_meta_changes(&collected, &expected_changes)?;

    let cursor_page = query_meta_changes_via_cluster_inter_route(
        &client,
        target_gateway_addr.as_str(),
        route_prefix,
        source.name.as_str(),
        prefix.as_str(),
        first_revision,
        page_limit,
        None,
    )
    .await?;
    if !cursor_page.has_more {
        return Err(format!(
            "stress change-feed first cursor page unexpectedly has no more pages: {:?}",
            cursor_page
        ));
    }
    let compact_cursor = cursor_page
        .next_cursor
        .clone()
        .ok_or_else(|| "stress change-feed first page missing cursor".to_string())?;
    let compact_revision = compact_cursor.revision;
    let compacted = post_meta_compact_via_admin_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        leader.name.as_str(),
        compact_revision,
    )
    .await?;
    let last_revision = expected_changes
        .last()
        .ok_or_else(|| "missing last stress revision".to_string())?
        .revision;
    if compacted.compacted_revision != compact_revision
        || compacted.current_revision < last_revision
    {
        return Err(format!(
            "unexpected stress compaction response: {:?}, compact_revision={}, last_revision={}",
            compacted, compact_revision, last_revision
        ));
    }
    expect_meta_changes_status_with_options_via_cluster_inter_route(
        &client,
        target_gateway_addr.as_str(),
        route_prefix,
        source.name.as_str(),
        prefix.as_str(),
        first_revision,
        Some(&compact_cursor),
        Some(500),
        StatusCode::GONE,
        Some("COMPACTED"),
    )
    .await?;

    let expected_after_compact = expected_changes
        .iter()
        .filter(|change| change.revision > compact_revision)
        .cloned()
        .collect::<Vec<_>>();
    let mut post_compact_cursor = None;
    let mut post_compact_collected = Vec::with_capacity(expected_after_compact.len());
    loop {
        let page = query_meta_changes_with_wait_via_cluster_inter_route(
            &client,
            source_gateway_addr.as_str(),
            route_prefix,
            target.name.as_str(),
            prefix.as_str(),
            compact_revision + 1,
            page_limit,
            post_compact_cursor.as_ref(),
            Some(500),
        )
        .await?;
        post_compact_collected.extend(page.items.iter().cloned());
        if !page.has_more {
            break;
        }
        post_compact_cursor = page.next_cursor;
        if post_compact_cursor.is_none() {
            return Err("stress post-compact page missing cursor".to_string());
        }
    }
    require_expected_meta_changes(&post_compact_collected, &expected_after_compact)?;

    for node in &nodes {
        let gateway = gateway_addr(node, ingress_port);
        let current = query_meta_prefix_via_cluster_inter_route(
            &client,
            gateway.as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            key_count + 8,
        )
        .await?;
        if current.items.len() != key_count {
            return Err(format!(
                "stress current key count mismatch on {}: expected={}, actual={}, items={:?}",
                node.name,
                key_count,
                current.items.len(),
                current.items
            ));
        }
        let samples = [
            0usize,
            delete_count.saturating_sub(1),
            delete_count,
            key_count / 2,
            key_count - 1,
        ];
        let mut expected_samples = Vec::new();
        for index in samples {
            let state = states
                .get(index)
                .ok_or_else(|| format!("missing stress sample state {}", index))?;
            expected_samples.push((
                state.key.as_str(),
                state.value.as_str(),
                state.create_revision,
                state.mod_revision,
                state.version,
            ));
        }
        require_meta_selected_values(&current, expected_samples.as_slice())?;
    }

    println!(
        "[klog-cluster-dv] MVCC change-feed stress ok: leader={}, keys={}, concurrency={}, rounds={}, delete_recreate={}, changes={}, pages={:?}, compact_revision={}, post_compact_changes={}, elapsed_ms={}, prefix={}",
        leader_id,
        key_count,
        concurrency,
        rounds,
        delete_count,
        expected_changes.len(),
        page_sizes,
        compact_revision,
        expected_after_compact.len(),
        stress_started.elapsed().as_millis(),
        prefix
    );
    Ok(())
}

async fn run_local_gateway_mvcc_change_feed_stress() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_mvcc_change_feed_stress_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_mvcc_failover_inner(harness: &mut LocalHarness) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-mvcc-failover-dv";
    let setup = prepare_local_gateway_setup(harness, MVCC_FAILOVER_MODE, route_prefix, 3).await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let seed = nodes
        .first()
        .cloned()
        .ok_or_else(|| "missing seed node".to_string())?;
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };

    for node in &nodes {
        let config = write_klog_config(harness, node, &voter_config)?;
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(50),
    )
    .await?;
    let old_leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;

    let source = nodes
        .iter()
        .find(|node| node.id != old_leader_id)
        .ok_or_else(|| format!("missing non-leader source node: leader={}", old_leader_id))?;
    let target = nodes
        .iter()
        .find(|node| node.id != old_leader_id && node.id != source.id)
        .or_else(|| nodes.iter().find(|node| node.id == old_leader_id))
        .ok_or_else(|| format!("missing target node: leader={}", old_leader_id))?;
    let observer = nodes
        .iter()
        .find(|node| node.id == old_leader_id)
        .unwrap_or(target);
    let source_gateway_addr = gateway_addr(source, ingress_port);
    let observer_gateway_addr = gateway_addr(observer, ingress_port);
    let suffix = unique_suffix("mvcc-failover");
    let prefix = format!("test/klog_mvcc_failover_dv/{}/", suffix);
    let key_a = format!("{}a", prefix);
    let key_b = format!("{}b", prefix);
    let key_c = format!("{}c", prefix);
    let key_d = format!("{}d", prefix);

    let tx1 = exec_meta_tx_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        BTreeMap::from([
            (
                key_a.clone(),
                meta_tx_put_action(&key_a, "a-v1", target.name.as_str(), Some(0)),
            ),
            (
                key_b.clone(),
                meta_tx_put_action(&key_b, "b-v1", target.name.as_str(), Some(0)),
            ),
        ]),
    )
    .await?;
    let r1 = tx1
        .revisions
        .get(&key_a)
        .and_then(|revision| *revision)
        .ok_or_else(|| format!("missing failover tx1 revision for {}", key_a))?;
    if tx1.revisions.get(&key_b).and_then(|revision| *revision) != Some(r1) {
        return Err(format!(
            "failover tx1 keys did not share revision: {:?}",
            tx1
        ));
    }
    require_meta_version(tx1.meta_versions.get(&key_a), r1, r1, 1, false)?;
    require_meta_version(tx1.meta_versions.get(&key_b), r1, r1, 1, false)?;

    let a_v2 = put_meta_via_cluster_inter_route(
        &client,
        observer_gateway_addr.as_str(),
        route_prefix,
        source.name.as_str(),
        key_a.as_str(),
        "a-v2",
        Some(r1),
    )
    .await?;
    let r2 = a_v2.mod_revision;
    if a_v2.create_revision != r1 || a_v2.version != 2 || r2 != r1 + 1 {
        return Err(format!("unexpected failover a_v2 response: {:?}", a_v2));
    }

    for node in &nodes {
        let gateway = gateway_addr(node, ingress_port);
        let rev1 = query_meta_prefix_page_at_revision_via_cluster_inter_route(
            &client,
            gateway.as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            16,
            None,
            Some(r1),
        )
        .await?;
        require_meta_values(
            &rev1,
            &[(&key_a, "a-v1", r1, r1, 1), (&key_b, "b-v1", r1, r1, 1)],
        )?;
    }

    let old_leader = nodes
        .iter()
        .find(|node| node.id == old_leader_id)
        .ok_or_else(|| format!("old leader node {} not found", old_leader_id))?;
    harness.stop(format!("klog-{}", old_leader.name).as_str())?;
    let alive_nodes = nodes
        .iter()
        .filter(|node| node.id != old_leader_id)
        .cloned()
        .collect::<Vec<_>>();
    wait_membership(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let new_leader_id = wait_consistent_leader(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        Some(old_leader_id),
        Duration::from_secs(70),
    )
    .await?;
    let new_leader = alive_nodes
        .iter()
        .find(|node| node.id == new_leader_id)
        .ok_or_else(|| format!("new leader node {} not found", new_leader_id))?;
    let failover_writer = alive_nodes
        .iter()
        .find(|node| node.id != new_leader_id)
        .unwrap_or(new_leader);
    let failover_writer_gateway = gateway_addr(failover_writer, ingress_port);
    let new_leader_gateway = gateway_addr(new_leader, ingress_port);

    let deleted_b = delete_meta_via_cluster_inter_route(
        &client,
        failover_writer_gateway.as_str(),
        route_prefix,
        new_leader.name.as_str(),
        key_b.as_str(),
    )
    .await?;
    let delete_version = deleted_b.meta_version.as_ref().ok_or_else(|| {
        format!(
            "missing failover key_b delete meta_version: {:?}",
            deleted_b
        )
    })?;
    require_meta_version(Some(delete_version), r1, r2 + 1, 0, true)?;
    let r3 = delete_version.mod_revision;

    expect_meta_put_status_via_cluster_inter_route(
        &client,
        failover_writer_gateway.as_str(),
        route_prefix,
        new_leader.name.as_str(),
        MetaPutRequest {
            key: key_b.clone(),
            value: "stale-b".to_string(),
            node_name: Some(new_leader.name.clone()),
            expected_revision: Some(r1),
        },
        StatusCode::CONFLICT,
    )
    .await?;

    let b_v2 = put_meta_via_cluster_inter_route(
        &client,
        new_leader_gateway.as_str(),
        route_prefix,
        failover_writer.name.as_str(),
        key_b.as_str(),
        "b-v2",
        Some(0),
    )
    .await?;
    let r4 = b_v2.mod_revision;
    if b_v2.create_revision != r4 || b_v2.version != 1 || r4 != r3 + 1 {
        return Err(format!("unexpected failover b_v2 response: {:?}", b_v2));
    }

    let tx5 = exec_meta_tx_via_cluster_inter_route(
        &client,
        failover_writer_gateway.as_str(),
        route_prefix,
        new_leader.name.as_str(),
        BTreeMap::from([
            (
                key_a.clone(),
                meta_tx_put_action(&key_a, "a-v3", failover_writer.name.as_str(), Some(r2)),
            ),
            (
                key_c.clone(),
                meta_tx_put_action(&key_c, "c-v1", failover_writer.name.as_str(), Some(0)),
            ),
            (
                key_d.clone(),
                meta_tx_put_action(&key_d, "d-v1", failover_writer.name.as_str(), Some(0)),
            ),
        ]),
    )
    .await?;
    let r5 = tx5
        .revisions
        .get(&key_a)
        .and_then(|revision| *revision)
        .ok_or_else(|| format!("missing failover tx5 revision for {}", key_a))?;
    if r5 != r4 + 1 {
        return Err(format!(
            "unexpected failover tx5 revision: r4={}, r5={}",
            r4, r5
        ));
    }
    require_meta_version(tx5.meta_versions.get(&key_a), r1, r5, 3, false)?;
    require_meta_version(tx5.meta_versions.get(&key_c), r5, r5, 1, false)?;
    require_meta_version(tx5.meta_versions.get(&key_d), r5, r5, 1, false)?;

    for node in &alive_nodes {
        let gateway = gateway_addr(node, ingress_port);
        let rev4 = query_meta_prefix_page_at_revision_via_cluster_inter_route(
            &client,
            gateway.as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            16,
            None,
            Some(r4),
        )
        .await?;
        require_meta_values(
            &rev4,
            &[(&key_a, "a-v2", r1, r2, 2), (&key_b, "b-v2", r4, r4, 1)],
        )?;

        let current = query_meta_prefix_via_cluster_inter_route(
            &client,
            gateway.as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            16,
        )
        .await?;
        require_meta_values(
            &current,
            &[
                (&key_a, "a-v3", r1, r5, 3),
                (&key_b, "b-v2", r4, r4, 1),
                (&key_c, "c-v1", r5, r5, 1),
                (&key_d, "d-v1", r5, r5, 1),
            ],
        )?;
    }

    let compacted = post_meta_compact_via_admin_route(
        &client,
        new_leader_gateway.as_str(),
        route_prefix,
        new_leader.name.as_str(),
        r3,
    )
    .await?;
    if compacted.compacted_revision != r3 || compacted.current_revision != r5 {
        return Err(format!(
            "unexpected failover compaction response: {:?}, expected compacted={}, current={}",
            compacted, r3, r5
        ));
    }

    expect_meta_query_status_via_cluster_inter_route(
        &client,
        failover_writer_gateway.as_str(),
        route_prefix,
        failover_writer.name.as_str(),
        None,
        Some(prefix.as_str()),
        Some(r1),
        StatusCode::GONE,
        Some("COMPACTED"),
    )
    .await?;
    expect_meta_changes_status_via_cluster_inter_route(
        &client,
        new_leader_gateway.as_str(),
        route_prefix,
        new_leader.name.as_str(),
        prefix.as_str(),
        r1,
        StatusCode::GONE,
        Some("COMPACTED"),
    )
    .await?;

    for node in &alive_nodes {
        let gateway = gateway_addr(node, ingress_port);
        let rev4 = query_meta_prefix_page_at_revision_via_cluster_inter_route(
            &client,
            gateway.as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            16,
            None,
            Some(r4),
        )
        .await?;
        require_meta_values(
            &rev4,
            &[(&key_a, "a-v2", r1, r2, 2), (&key_b, "b-v2", r4, r4, 1)],
        )?;
    }

    let post_compact_changes = query_meta_changes_via_cluster_inter_route(
        &client,
        failover_writer_gateway.as_str(),
        route_prefix,
        failover_writer.name.as_str(),
        prefix.as_str(),
        r4,
        8,
        None,
    )
    .await?;
    require_meta_changes(
        &post_compact_changes,
        &[
            (r4, &key_b, "b-v2", false, r4, 1),
            (r5, &key_a, "a-v3", false, r1, 3),
            (r5, &key_c, "c-v1", false, r5, 1),
            (r5, &key_d, "d-v1", false, r5, 1),
        ],
    )?;

    println!(
        "[klog-cluster-dv] MVCC failover ok: old_leader={}, new_leader={}, writer={}, revisions=[{},{},{},{},{}], prefix={}",
        old_leader_id, new_leader_id, failover_writer.name, r1, r2, r3, r4, r5, prefix
    );
    Ok(())
}

async fn run_local_gateway_mvcc_failover() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_mvcc_failover_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_mvcc_auto_compact_failover_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-mvcc-auto-compact-failover-dv";
    let setup =
        prepare_local_gateway_setup(harness, MVCC_AUTO_COMPACT_FAILOVER_MODE, route_prefix, 3)
            .await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let seed = nodes
        .first()
        .cloned()
        .ok_or_else(|| "missing seed node".to_string())?;
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };
    let meta_compaction = KLogMetaCompactionPatch {
        retention_revisions: 6,
        check_interval_ms: 200,
        min_compact_gap: 2,
    };

    for node in &nodes {
        let config =
            write_klog_config_with_meta_compaction(harness, node, &voter_config, meta_compaction)?;
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(50),
    )
    .await?;
    let initial_leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;

    let suffix = unique_suffix("mvcc-auto-compact-failover");
    let prefix = format!("test/klog_mvcc_auto_compact_failover_dv/{}/", suffix);
    let mut expected_current = Vec::new();
    let phase1_count = 14usize;
    for index in 0..phase1_count {
        let source = &nodes[index % nodes.len()];
        let target = &nodes[(index + 1) % nodes.len()];
        let key = format!("{}phase1-{:03}", prefix, index);
        let value = format!("phase1-value-{:03}", index);
        let stored = put_meta_via_cluster_inter_route(
            &client,
            gateway_addr(source, ingress_port).as_str(),
            route_prefix,
            target.name.as_str(),
            key.as_str(),
            value.as_str(),
            Some(0),
        )
        .await?;
        if stored.create_revision != stored.mod_revision || stored.version != 1 {
            return Err(format!(
                "unexpected auto-compact phase1 put response: {:?}",
                stored
            ));
        }
        expected_current.push(ExpectedMetaChange {
            revision: stored.mod_revision,
            key,
            value,
            deleted: false,
            create_revision: stored.create_revision,
            version: stored.version,
        });
    }
    let first_revision = expected_current
        .first()
        .ok_or_else(|| "missing auto-compact first revision".to_string())?
        .revision;
    let phase1_last = expected_current
        .last()
        .cloned()
        .ok_or_else(|| "missing auto-compact phase1 last revision".to_string())?;

    let observer = nodes
        .iter()
        .find(|node| node.id != initial_leader_id)
        .unwrap_or(&nodes[0]);
    wait_meta_query_compacted_via_cluster_inter_route(
        &client,
        gateway_addr(observer, ingress_port).as_str(),
        route_prefix,
        observer.name.as_str(),
        Some(expected_current[0].key.as_str()),
        None,
        first_revision,
        Duration::from_secs(40),
    )
    .await?;

    let old_leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(20),
    )
    .await?;
    let old_leader = nodes
        .iter()
        .find(|node| node.id == old_leader_id)
        .ok_or_else(|| format!("old leader node {} not found", old_leader_id))?;
    harness.stop(format!("klog-{}", old_leader.name).as_str())?;
    let alive_nodes = nodes
        .iter()
        .filter(|node| node.id != old_leader_id)
        .cloned()
        .collect::<Vec<_>>();
    wait_membership(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let new_leader_id = wait_consistent_leader(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        Some(old_leader_id),
        Duration::from_secs(70),
    )
    .await?;

    let phase2_count = 14usize;
    for index in 0..phase2_count {
        let source = &alive_nodes[index % alive_nodes.len()];
        let target = &alive_nodes[(index + 1) % alive_nodes.len()];
        let key = format!("{}phase2-{:03}", prefix, index);
        let value = format!("phase2-value-{:03}", index);
        let stored = put_meta_via_cluster_inter_route(
            &client,
            gateway_addr(source, ingress_port).as_str(),
            route_prefix,
            target.name.as_str(),
            key.as_str(),
            value.as_str(),
            Some(0),
        )
        .await?;
        if stored.create_revision != stored.mod_revision || stored.version != 1 {
            return Err(format!(
                "unexpected auto-compact phase2 put response: {:?}",
                stored
            ));
        }
        expected_current.push(ExpectedMetaChange {
            revision: stored.mod_revision,
            key,
            value,
            deleted: false,
            create_revision: stored.create_revision,
            version: stored.version,
        });
    }

    let alive_observer = alive_nodes
        .iter()
        .find(|node| node.id != new_leader_id)
        .unwrap_or(&alive_nodes[0]);
    wait_meta_query_compacted_via_cluster_inter_route(
        &client,
        gateway_addr(alive_observer, ingress_port).as_str(),
        route_prefix,
        alive_observer.name.as_str(),
        Some(phase1_last.key.as_str()),
        None,
        phase1_last.revision,
        Duration::from_secs(40),
    )
    .await?;
    expect_meta_changes_status_via_cluster_inter_route(
        &client,
        gateway_addr(alive_observer, ingress_port).as_str(),
        route_prefix,
        alive_observer.name.as_str(),
        prefix.as_str(),
        phase1_last.revision,
        StatusCode::GONE,
        Some("COMPACTED"),
    )
    .await?;

    for node in &alive_nodes {
        let current = query_meta_prefix_via_cluster_inter_route(
            &client,
            gateway_addr(node, ingress_port).as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            expected_current.len() + 8,
        )
        .await?;
        require_expected_current_meta_values(&current, expected_current.as_slice())?;
    }

    let latest = expected_current
        .last()
        .ok_or_else(|| "missing auto-compact latest revision".to_string())?;
    let post_compact_changes = query_meta_changes_via_cluster_inter_route(
        &client,
        gateway_addr(alive_observer, ingress_port).as_str(),
        route_prefix,
        alive_observer.name.as_str(),
        prefix.as_str(),
        latest.revision,
        8,
        None,
    )
    .await?;
    require_meta_changes(
        &post_compact_changes,
        &[(
            latest.revision,
            latest.key.as_str(),
            latest.value.as_str(),
            false,
            latest.create_revision,
            latest.version,
        )],
    )?;

    println!(
        "[klog-cluster-dv] MVCC auto-compact failover ok: initial_leader={}, stopped_leader={}, new_leader={}, first_revision={}, phase1_last_revision={}, latest_revision={}, keys={}, prefix={}",
        initial_leader_id,
        old_leader_id,
        new_leader_id,
        first_revision,
        phase1_last.revision,
        latest.revision,
        expected_current.len(),
        prefix
    );
    Ok(())
}

async fn run_local_gateway_mvcc_auto_compact_failover() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_mvcc_auto_compact_failover_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_mvcc_compaction_leader_switch_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-mvcc-compaction-leader-switch-dv";
    let setup =
        prepare_local_gateway_setup(harness, MVCC_COMPACTION_LEADER_SWITCH_MODE, route_prefix, 3)
            .await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let seed = nodes
        .first()
        .cloned()
        .ok_or_else(|| "missing seed node".to_string())?;
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };
    let meta_compaction = KLogMetaCompactionPatch {
        retention_revisions: 8,
        check_interval_ms: 1500,
        min_compact_gap: 1,
    };
    let mut configs = BTreeMap::new();
    for node in &nodes {
        let config =
            write_klog_config_with_meta_compaction(harness, node, &voter_config, meta_compaction)?;
        configs.insert(node.id, config.clone());
        spawn_klog_with_log_level(harness, &klog_daemon_bin, &config, node, "info")?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(50),
    )
    .await?;

    let suffix = unique_suffix("mvcc-compaction-leader-switch");
    let prefix = format!("test/klog_mvcc_compaction_leader_switch_dv/{}/", suffix);
    let mut expected_current = Vec::new();
    for index in 0..6usize {
        let source = &nodes[index % nodes.len()];
        let target = &nodes[(index + 1) % nodes.len()];
        let key = format!("{}manual-{:03}", prefix, index);
        let value = format!("manual-value-{:03}", index);
        let stored = put_meta_via_cluster_inter_route(
            &client,
            gateway_addr(source, ingress_port).as_str(),
            route_prefix,
            target.name.as_str(),
            key.as_str(),
            value.as_str(),
            Some(0),
        )
        .await?;
        if stored.create_revision != stored.mod_revision || stored.version != 1 {
            return Err(format!(
                "unexpected manual phase put response: {:?}",
                stored
            ));
        }
        expected_current.push(ExpectedMetaChange {
            revision: stored.mod_revision,
            key,
            value,
            deleted: false,
            create_revision: stored.create_revision,
            version: stored.version,
        });
    }
    let manual_compact_revision = expected_current
        .get(2)
        .ok_or_else(|| "missing manual compact target revision".to_string())?
        .revision;
    let manual_retained_revision = expected_current
        .get(3)
        .ok_or_else(|| "missing manual retained revision".to_string())?
        .revision;
    let manual_current_revision = expected_current
        .last()
        .ok_or_else(|| "missing manual current revision".to_string())?
        .revision;

    let manual_leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let manual_leader = nodes
        .iter()
        .find(|node| node.id == manual_leader_id)
        .cloned()
        .ok_or_else(|| format!("manual compact leader {} not found", manual_leader_id))?;
    let manual_url = cluster_route_url(
        gateway_addr(&manual_leader, ingress_port).as_str(),
        route_prefix,
        manual_leader.name.as_str(),
        "admin",
        "/meta-compact",
    );
    let manual_client = client.clone();
    let manual_task = tokio::spawn(async move {
        let response = manual_client
            .post(manual_url.as_str())
            .json(&MetaCompactRequest {
                revision: manual_compact_revision,
            })
            .send()
            .await
            .map_err(|err| format!("manual in-flight meta-compact request failed: {}", err))?;
        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|err| format!("<failed to read body: {}>", err));
            return Err(format!(
                "manual in-flight meta-compact returned {}: {}",
                status, body
            ));
        }
        response
            .json::<MetaCompactResponse>()
            .await
            .map_err(|err| format!("manual in-flight meta-compact decode failed: {}", err))
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    harness.stop(format!("klog-{}", manual_leader.name).as_str())?;
    let manual_result = manual_task
        .await
        .map_err(|err| format!("manual in-flight compact task join failed: {}", err));

    let alive_after_manual = nodes
        .iter()
        .filter(|node| node.id != manual_leader_id)
        .cloned()
        .collect::<Vec<_>>();
    wait_membership(
        &client,
        &alive_after_manual,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let manual_failover_leader_id = wait_consistent_leader(
        &client,
        &alive_after_manual,
        ingress_port,
        route_prefix,
        Some(manual_leader_id),
        Duration::from_secs(70),
    )
    .await?;
    let manual_observer = alive_after_manual
        .first()
        .ok_or_else(|| "missing manual alive observer".to_string())?;
    let manual_observer_gateway = gateway_addr(manual_observer, ingress_port);
    let manual_already_compacted = wait_meta_query_compacted_via_cluster_inter_route(
        &client,
        manual_observer_gateway.as_str(),
        route_prefix,
        manual_observer.name.as_str(),
        None,
        Some(prefix.as_str()),
        manual_compact_revision,
        Duration::from_secs(8),
    )
    .await
    .is_ok();
    if !manual_already_compacted {
        let new_leader = alive_after_manual
            .iter()
            .find(|node| node.id == manual_failover_leader_id)
            .ok_or_else(|| {
                format!(
                    "manual failover leader {} not found",
                    manual_failover_leader_id
                )
            })?;
        let compacted = post_meta_compact_via_admin_route(
            &client,
            gateway_addr(new_leader, ingress_port).as_str(),
            route_prefix,
            new_leader.name.as_str(),
            manual_compact_revision,
        )
        .await?;
        if compacted.compacted_revision != manual_compact_revision
            || compacted.current_revision < manual_current_revision
        {
            return Err(format!(
                "unexpected manual failover compaction response: {:?}, expected compacted={}, current>={}",
                compacted, manual_compact_revision, manual_current_revision
            ));
        }
    } else if let Ok(Ok(compacted)) = manual_result
        && (compacted.compacted_revision != manual_compact_revision
            || compacted.current_revision < manual_current_revision)
    {
        return Err(format!(
            "unexpected in-flight manual compaction response: {:?}, expected compacted={}, current>={}",
            compacted, manual_compact_revision, manual_current_revision
        ));
    }

    for node in &alive_after_manual {
        wait_meta_query_compacted_via_cluster_inter_route(
            &client,
            gateway_addr(node, ingress_port).as_str(),
            route_prefix,
            node.name.as_str(),
            None,
            Some(prefix.as_str()),
            manual_compact_revision,
            Duration::from_secs(20),
        )
        .await?;
        let retained = query_meta_at_revision_via_cluster_inter_route(
            &client,
            gateway_addr(node, ingress_port).as_str(),
            route_prefix,
            node.name.as_str(),
            expected_current[3].key.as_str(),
            Some(manual_retained_revision),
        )
        .await?;
        require_meta_values(
            &retained,
            &[(
                expected_current[3].key.as_str(),
                expected_current[3].value.as_str(),
                expected_current[3].create_revision,
                expected_current[3].revision,
                expected_current[3].version,
            )],
        )?;
    }

    let manual_commit_pattern = format!(
        "StateMachine meta-compact request committed: compacted_revision={},",
        manual_compact_revision
    );
    for node in &nodes {
        let count = count_klog_out_log_occurrences(harness, node, manual_commit_pattern.as_str())?;
        if count > 1 {
            return Err(format!(
                "manual compact target committed more than once on {}: target={}, count={}",
                node.name, manual_compact_revision, count
            ));
        }
    }

    let manual_leader_config = configs
        .get(&manual_leader_id)
        .ok_or_else(|| format!("missing config for manual leader {}", manual_leader_id))?;
    spawn_klog_with_log_level(
        harness,
        &klog_daemon_bin,
        manual_leader_config,
        &manual_leader,
        "info",
    )?;
    wait_tcp(
        "127.0.0.1",
        manual_leader.ports.admin,
        Duration::from_secs(12),
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(90),
    )
    .await?;
    wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(80),
    )
    .await?;
    for node in &nodes {
        wait_meta_query_compacted_via_cluster_inter_route(
            &client,
            gateway_addr(node, ingress_port).as_str(),
            route_prefix,
            node.name.as_str(),
            None,
            Some(prefix.as_str()),
            manual_compact_revision,
            Duration::from_secs(30),
        )
        .await?;
    }

    for index in 0..14usize {
        let source = &nodes[index % nodes.len()];
        let target = &nodes[(index + 1) % nodes.len()];
        let key = format!("{}auto-{:03}", prefix, index);
        let value = format!("auto-value-{:03}", index);
        let stored = put_meta_via_cluster_inter_route(
            &client,
            gateway_addr(source, ingress_port).as_str(),
            route_prefix,
            target.name.as_str(),
            key.as_str(),
            value.as_str(),
            Some(0),
        )
        .await?;
        if stored.create_revision != stored.mod_revision || stored.version != 1 {
            return Err(format!("unexpected auto phase put response: {:?}", stored));
        }
        expected_current.push(ExpectedMetaChange {
            revision: stored.mod_revision,
            key,
            value,
            deleted: false,
            create_revision: stored.create_revision,
            version: stored.version,
        });
    }
    let auto_compact_probe_revision = manual_retained_revision;
    let auto_latest = expected_current
        .last()
        .cloned()
        .ok_or_else(|| "missing auto phase latest revision".to_string())?;
    let auto_leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let auto_leader = nodes
        .iter()
        .find(|node| node.id == auto_leader_id)
        .cloned()
        .ok_or_else(|| format!("auto compact leader {} not found", auto_leader_id))?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    harness.stop(format!("klog-{}", auto_leader.name).as_str())?;

    let alive_after_auto = nodes
        .iter()
        .filter(|node| node.id != auto_leader_id)
        .cloned()
        .collect::<Vec<_>>();
    wait_membership(
        &client,
        &alive_after_auto,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let auto_failover_leader_id = wait_consistent_leader(
        &client,
        &alive_after_auto,
        ingress_port,
        route_prefix,
        Some(auto_leader_id),
        Duration::from_secs(70),
    )
    .await?;
    let auto_observer = alive_after_auto
        .first()
        .ok_or_else(|| "missing auto alive observer".to_string())?;
    wait_meta_query_compacted_via_cluster_inter_route(
        &client,
        gateway_addr(auto_observer, ingress_port).as_str(),
        route_prefix,
        auto_observer.name.as_str(),
        None,
        Some(prefix.as_str()),
        auto_compact_probe_revision,
        Duration::from_secs(70),
    )
    .await?;
    for node in &alive_after_auto {
        let current = query_meta_prefix_via_cluster_inter_route(
            &client,
            gateway_addr(node, ingress_port).as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            expected_current.len() + 8,
        )
        .await?;
        require_expected_current_meta_values(&current, expected_current.as_slice())?;
        let latest_page = query_meta_changes_via_cluster_inter_route(
            &client,
            gateway_addr(node, ingress_port).as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            auto_latest.revision,
            8,
            None,
        )
        .await?;
        require_meta_changes(
            &latest_page,
            &[(
                auto_latest.revision,
                auto_latest.key.as_str(),
                auto_latest.value.as_str(),
                false,
                auto_latest.create_revision,
                auto_latest.version,
            )],
        )?;
    }

    let auto_leader_config = configs
        .get(&auto_leader_id)
        .ok_or_else(|| format!("missing config for auto leader {}", auto_leader_id))?;
    spawn_klog_with_log_level(
        harness,
        &klog_daemon_bin,
        auto_leader_config,
        &auto_leader,
        "info",
    )?;
    wait_tcp(
        "127.0.0.1",
        auto_leader.ports.admin,
        Duration::from_secs(12),
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(90),
    )
    .await?;
    wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(80),
    )
    .await?;
    for node in &nodes {
        wait_meta_query_compacted_via_cluster_inter_route(
            &client,
            gateway_addr(node, ingress_port).as_str(),
            route_prefix,
            node.name.as_str(),
            None,
            Some(prefix.as_str()),
            auto_compact_probe_revision,
            Duration::from_secs(40),
        )
        .await?;
        let current = query_meta_prefix_via_cluster_inter_route(
            &client,
            gateway_addr(node, ingress_port).as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            expected_current.len() + 8,
        )
        .await?;
        require_expected_current_meta_values(&current, expected_current.as_slice())?;
    }

    println!(
        "[klog-cluster-dv] MVCC compaction leader switch ok: manual_leader={}, manual_failover_leader={}, auto_switch_leader={}, auto_failover_leader={}, manual_compacted={}, auto_probe_compacted={}, latest_revision={}, keys={}, prefix={}",
        manual_leader_id,
        manual_failover_leader_id,
        auto_leader_id,
        auto_failover_leader_id,
        manual_compact_revision,
        auto_compact_probe_revision,
        auto_latest.revision,
        expected_current.len(),
        prefix
    );
    Ok(())
}

async fn run_local_gateway_mvcc_compaction_leader_switch() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_mvcc_compaction_leader_switch_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_mvcc_crash_recovery_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-mvcc-crash-recovery-dv";
    let setup =
        prepare_local_gateway_setup(harness, MVCC_CRASH_RECOVERY_MODE, route_prefix, 3).await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let seed = nodes
        .first()
        .cloned()
        .ok_or_else(|| "missing seed node".to_string())?;
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };
    let mut configs = BTreeMap::new();
    for node in &nodes {
        let config = write_klog_config(harness, node, &voter_config)?;
        configs.insert(node.id, config.clone());
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(50),
    )
    .await?;
    let leader_before_crash = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;

    let suffix = unique_suffix("mvcc-crash-recovery");
    let prefix = format!("test/klog_mvcc_crash_recovery_dv/{}/", suffix);
    let key_a = format!("{}a", prefix);
    let key_b = format!("{}b", prefix);
    let key_c = format!("{}c", prefix);
    let key_d = format!("{}d", prefix);
    let key_e = format!("{}e", prefix);
    let source = nodes
        .iter()
        .find(|node| node.id != leader_before_crash)
        .unwrap_or(&nodes[0]);
    let target = nodes
        .iter()
        .find(|node| node.id != source.id)
        .unwrap_or(source);
    let source_gateway = gateway_addr(source, ingress_port);

    let a_v1 = put_meta_via_cluster_inter_route(
        &client,
        source_gateway.as_str(),
        route_prefix,
        target.name.as_str(),
        key_a.as_str(),
        "a-v1",
        Some(0),
    )
    .await?;
    let r1 = a_v1.mod_revision;
    if a_v1.create_revision != r1 || a_v1.version != 1 {
        return Err(format!("unexpected crash recovery a_v1: {:?}", a_v1));
    }

    let b_v1 = put_meta_via_cluster_inter_route(
        &client,
        source_gateway.as_str(),
        route_prefix,
        target.name.as_str(),
        key_b.as_str(),
        "b-v1",
        Some(0),
    )
    .await?;
    let r2 = b_v1.mod_revision;
    if b_v1.create_revision != r2 || b_v1.version != 1 || r2 != r1 + 1 {
        return Err(format!("unexpected crash recovery b_v1: {:?}", b_v1));
    }

    let a_v2 = put_meta_via_cluster_inter_route(
        &client,
        source_gateway.as_str(),
        route_prefix,
        target.name.as_str(),
        key_a.as_str(),
        "a-v2",
        Some(r1),
    )
    .await?;
    let r3 = a_v2.mod_revision;
    if a_v2.create_revision != r1 || a_v2.version != 2 || r3 != r2 + 1 {
        return Err(format!("unexpected crash recovery a_v2: {:?}", a_v2));
    }

    let b_deleted = delete_meta_via_cluster_inter_route(
        &client,
        source_gateway.as_str(),
        route_prefix,
        target.name.as_str(),
        key_b.as_str(),
    )
    .await?;
    let b_delete_version = b_deleted.meta_version.as_ref().ok_or_else(|| {
        format!(
            "missing crash recovery b delete meta_version: {:?}",
            b_deleted
        )
    })?;
    require_meta_version(Some(b_delete_version), r2, r3 + 1, 0, true)?;
    let r4 = b_delete_version.mod_revision;

    let b_v2 = put_meta_via_cluster_inter_route(
        &client,
        source_gateway.as_str(),
        route_prefix,
        target.name.as_str(),
        key_b.as_str(),
        "b-v2",
        Some(0),
    )
    .await?;
    let r5 = b_v2.mod_revision;
    if b_v2.create_revision != r5 || b_v2.version != 1 || r5 != r4 + 1 {
        return Err(format!("unexpected crash recovery b_v2: {:?}", b_v2));
    }

    let tx6 = exec_meta_tx_via_cluster_inter_route(
        &client,
        source_gateway.as_str(),
        route_prefix,
        target.name.as_str(),
        BTreeMap::from([
            (
                key_c.clone(),
                meta_tx_put_action(&key_c, "c-v1", target.name.as_str(), Some(0)),
            ),
            (
                key_d.clone(),
                meta_tx_put_action(&key_d, "d-v1", target.name.as_str(), Some(0)),
            ),
        ]),
    )
    .await?;
    let r6 = tx6
        .revisions
        .get(&key_c)
        .and_then(|revision| *revision)
        .ok_or_else(|| format!("missing crash recovery tx6 revision for {}", key_c))?;
    if tx6.revisions.get(&key_d).and_then(|revision| *revision) != Some(r6) || r6 != r5 + 1 {
        return Err(format!("unexpected crash recovery tx6 response: {:?}", tx6));
    }
    require_meta_version(tx6.meta_versions.get(&key_c), r6, r6, 1, false)?;
    require_meta_version(tx6.meta_versions.get(&key_d), r6, r6, 1, false)?;

    let leader = nodes
        .iter()
        .find(|node| node.id == leader_before_crash)
        .ok_or_else(|| format!("leader node {} not found", leader_before_crash))?;
    let leader_gateway = gateway_addr(leader, ingress_port);
    let compacted = post_meta_compact_via_admin_route(
        &client,
        leader_gateway.as_str(),
        route_prefix,
        leader.name.as_str(),
        r4,
    )
    .await?;
    if compacted.compacted_revision != r4 || compacted.current_revision != r6 {
        return Err(format!(
            "unexpected crash recovery compaction response: {:?}, expected compacted={}, current={}",
            compacted, r4, r6
        ));
    }
    for node in &nodes {
        expect_meta_query_status_via_cluster_inter_route(
            &client,
            gateway_addr(node, ingress_port).as_str(),
            route_prefix,
            node.name.as_str(),
            Some(key_a.as_str()),
            None,
            Some(r1),
            StatusCode::GONE,
            Some("COMPACTED"),
        )
        .await?;
    }

    harness.stop(format!("klog-{}", leader.name).as_str())?;
    let alive_nodes = nodes
        .iter()
        .filter(|node| node.id != leader_before_crash)
        .cloned()
        .collect::<Vec<_>>();
    wait_membership(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let new_leader_id = wait_consistent_leader(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        Some(leader_before_crash),
        Duration::from_secs(70),
    )
    .await?;
    let failover_writer = alive_nodes
        .iter()
        .find(|node| node.id != new_leader_id)
        .unwrap_or(&alive_nodes[0]);
    let new_leader = alive_nodes
        .iter()
        .find(|node| node.id == new_leader_id)
        .ok_or_else(|| format!("new leader node {} not found", new_leader_id))?;
    let failover_gateway = gateway_addr(failover_writer, ingress_port);

    let a_v3 = put_meta_via_cluster_inter_route(
        &client,
        failover_gateway.as_str(),
        route_prefix,
        new_leader.name.as_str(),
        key_a.as_str(),
        "a-v3",
        Some(r3),
    )
    .await?;
    let r7 = a_v3.mod_revision;
    if a_v3.create_revision != r1 || a_v3.version != 3 || r7 != r6 + 1 {
        return Err(format!("unexpected crash recovery a_v3: {:?}", a_v3));
    }

    let c_deleted = delete_meta_via_cluster_inter_route(
        &client,
        failover_gateway.as_str(),
        route_prefix,
        new_leader.name.as_str(),
        key_c.as_str(),
    )
    .await?;
    let c_delete_version = c_deleted.meta_version.as_ref().ok_or_else(|| {
        format!(
            "missing crash recovery c delete meta_version: {:?}",
            c_deleted
        )
    })?;
    require_meta_version(Some(c_delete_version), r6, r7 + 1, 0, true)?;
    let r8 = c_delete_version.mod_revision;

    let c_v2 = put_meta_via_cluster_inter_route(
        &client,
        failover_gateway.as_str(),
        route_prefix,
        new_leader.name.as_str(),
        key_c.as_str(),
        "c-v2",
        Some(0),
    )
    .await?;
    let r9 = c_v2.mod_revision;
    if c_v2.create_revision != r9 || c_v2.version != 1 || r9 != r8 + 1 {
        return Err(format!("unexpected crash recovery c_v2: {:?}", c_v2));
    }

    let e_v1 = put_meta_via_cluster_inter_route(
        &client,
        failover_gateway.as_str(),
        route_prefix,
        new_leader.name.as_str(),
        key_e.as_str(),
        "e-v1",
        Some(0),
    )
    .await?;
    let r10 = e_v1.mod_revision;
    if e_v1.create_revision != r10 || e_v1.version != 1 || r10 != r9 + 1 {
        return Err(format!("unexpected crash recovery e_v1: {:?}", e_v1));
    }

    let old_leader_config = configs
        .get(&leader_before_crash)
        .ok_or_else(|| format!("missing config for old leader {}", leader_before_crash))?;
    spawn_klog(harness, &klog_daemon_bin, old_leader_config, leader)?;
    wait_tcp("127.0.0.1", leader.ports.admin, Duration::from_secs(12)).await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(90),
    )
    .await?;
    let leader_after_recovery = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(80),
    )
    .await?;

    wait_meta_prefix_count_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        prefix.as_str(),
        5,
        Duration::from_secs(60),
    )
    .await?;
    let expected_current = vec![
        ExpectedMetaChange {
            revision: r7,
            key: key_a.clone(),
            value: "a-v3".to_string(),
            deleted: false,
            create_revision: r1,
            version: 3,
        },
        ExpectedMetaChange {
            revision: r5,
            key: key_b.clone(),
            value: "b-v2".to_string(),
            deleted: false,
            create_revision: r5,
            version: 1,
        },
        ExpectedMetaChange {
            revision: r9,
            key: key_c.clone(),
            value: "c-v2".to_string(),
            deleted: false,
            create_revision: r9,
            version: 1,
        },
        ExpectedMetaChange {
            revision: r6,
            key: key_d.clone(),
            value: "d-v1".to_string(),
            deleted: false,
            create_revision: r6,
            version: 1,
        },
        ExpectedMetaChange {
            revision: r10,
            key: key_e.clone(),
            value: "e-v1".to_string(),
            deleted: false,
            create_revision: r10,
            version: 1,
        },
    ];
    for node in &nodes {
        let current = query_meta_prefix_via_cluster_inter_route(
            &client,
            gateway_addr(node, ingress_port).as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            16,
        )
        .await?;
        require_expected_current_meta_values(&current, expected_current.as_slice())?;
        expect_meta_query_status_via_cluster_inter_route(
            &client,
            gateway_addr(node, ingress_port).as_str(),
            route_prefix,
            node.name.as_str(),
            Some(key_a.as_str()),
            None,
            Some(r4),
            StatusCode::GONE,
            Some("COMPACTED"),
        )
        .await?;
        expect_meta_changes_status_via_cluster_inter_route(
            &client,
            gateway_addr(node, ingress_port).as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            r1,
            StatusCode::GONE,
            Some("COMPACTED"),
        )
        .await?;
    }

    let changes = query_meta_changes_via_cluster_inter_route(
        &client,
        gateway_addr(leader, ingress_port).as_str(),
        route_prefix,
        leader.name.as_str(),
        prefix.as_str(),
        r5,
        16,
        None,
    )
    .await?;
    require_meta_changes(
        &changes,
        &[
            (r5, &key_b, "b-v2", false, r5, 1),
            (r6, &key_c, "c-v1", false, r6, 1),
            (r6, &key_d, "d-v1", false, r6, 1),
            (r7, &key_a, "a-v3", false, r1, 3),
            (r8, &key_c, "c-v1", true, r6, 0),
            (r9, &key_c, "c-v2", false, r9, 1),
            (r10, &key_e, "e-v1", false, r10, 1),
        ],
    )?;

    println!(
        "[klog-cluster-dv] MVCC crash recovery ok: crashed_leader={}, failover_leader={}, recovered_leader={}, compacted_revision={}, latest_revision={}, prefix={}",
        leader_before_crash, new_leader_id, leader_after_recovery, r4, r10, prefix
    );
    Ok(())
}

async fn run_local_gateway_mvcc_crash_recovery() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_mvcc_crash_recovery_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_raft_old_leader_rejoin_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-raft-old-leader-rejoin-dv";
    let setup =
        prepare_local_gateway_setup(harness, RAFT_OLD_LEADER_REJOIN_MODE, route_prefix, 3).await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let seed = nodes
        .first()
        .cloned()
        .ok_or_else(|| "missing seed node".to_string())?;
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };
    let mut configs = BTreeMap::new();
    for node in &nodes {
        let config = write_klog_config(harness, node, &voter_config)?;
        configs.insert(node.id, config.clone());
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(50),
    )
    .await?;
    let old_leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let old_leader = nodes
        .iter()
        .find(|node| node.id == old_leader_id)
        .ok_or_else(|| format!("old leader node {} not found", old_leader_id))?;
    let pre_writer = nodes
        .iter()
        .find(|node| node.id != old_leader_id)
        .unwrap_or(old_leader);
    let old_leader_gateway = gateway_addr(old_leader, ingress_port);
    let pre_writer_gateway = gateway_addr(pre_writer, ingress_port);
    let suffix = unique_suffix("raft-old-leader-rejoin");
    let log_source = format!("test/klog_raft_old_leader_rejoin_dv/log/{}", suffix);
    let prefix = format!("test/klog_raft_old_leader_rejoin_dv/{}/", suffix);
    let key_a = format!("{}a", prefix);
    let key_b = format!("{}b", prefix);

    let before_log = append_via_cluster_inter_route(
        &client,
        pre_writer_gateway.as_str(),
        route_prefix,
        old_leader.name.as_str(),
        log_source.as_str(),
        "old leader rejoin write before crash",
    )
    .await?;
    wait_log_visible_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        before_log.id,
        log_source.as_str(),
        Duration::from_secs(30),
    )
    .await?;

    let a_v1 = put_meta_via_cluster_inter_route(
        &client,
        pre_writer_gateway.as_str(),
        route_prefix,
        old_leader.name.as_str(),
        key_a.as_str(),
        "a-before-crash",
        Some(0),
    )
    .await?;
    let r1 = a_v1.mod_revision;
    if a_v1.create_revision != r1 || a_v1.version != 1 {
        return Err(format!("unexpected old-leader-rejoin a_v1: {:?}", a_v1));
    }

    harness.stop(format!("klog-{}", old_leader.name).as_str())?;
    let alive_nodes = nodes
        .iter()
        .filter(|node| node.id != old_leader_id)
        .cloned()
        .collect::<Vec<_>>();
    wait_membership(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let new_leader_id = wait_consistent_leader(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        Some(old_leader_id),
        Duration::from_secs(70),
    )
    .await?;
    let new_leader = alive_nodes
        .iter()
        .find(|node| node.id == new_leader_id)
        .ok_or_else(|| format!("new leader node {} not found", new_leader_id))?;
    let failover_writer = alive_nodes
        .iter()
        .find(|node| node.id != new_leader_id)
        .unwrap_or(new_leader);
    let failover_writer_gateway = gateway_addr(failover_writer, ingress_port);

    let after_log = append_via_cluster_inter_route(
        &client,
        failover_writer_gateway.as_str(),
        route_prefix,
        new_leader.name.as_str(),
        log_source.as_str(),
        "old leader rejoin write after crash",
    )
    .await?;
    if after_log.id <= before_log.id {
        return Err(format!(
            "log id did not advance after leader crash: before={}, after={}",
            before_log.id, after_log.id
        ));
    }
    wait_log_visible_on_nodes(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        after_log.id,
        log_source.as_str(),
        Duration::from_secs(30),
    )
    .await?;

    let a_v2 = put_meta_via_cluster_inter_route(
        &client,
        failover_writer_gateway.as_str(),
        route_prefix,
        new_leader.name.as_str(),
        key_a.as_str(),
        "a-after-crash",
        Some(r1),
    )
    .await?;
    let r2 = a_v2.mod_revision;
    if a_v2.create_revision != r1 || a_v2.version != 2 || r2 != r1 + 1 {
        return Err(format!("unexpected old-leader-rejoin a_v2: {:?}", a_v2));
    }
    let b_v1 = put_meta_via_cluster_inter_route(
        &client,
        failover_writer_gateway.as_str(),
        route_prefix,
        new_leader.name.as_str(),
        key_b.as_str(),
        "b-after-crash",
        Some(0),
    )
    .await?;
    let r3 = b_v1.mod_revision;
    if b_v1.create_revision != r3 || b_v1.version != 1 || r3 != r2 + 1 {
        return Err(format!("unexpected old-leader-rejoin b_v1: {:?}", b_v1));
    }

    let old_leader_config = configs
        .get(&old_leader_id)
        .ok_or_else(|| format!("missing config for old leader {}", old_leader_id))?;
    spawn_klog(harness, &klog_daemon_bin, old_leader_config, old_leader)?;
    wait_tcp("127.0.0.1", old_leader.ports.admin, Duration::from_secs(12)).await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(90),
    )
    .await?;
    let leader_after_rejoin = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(80),
    )
    .await?;

    wait_log_visible_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        before_log.id,
        log_source.as_str(),
        Duration::from_secs(45),
    )
    .await?;
    wait_log_visible_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        after_log.id,
        log_source.as_str(),
        Duration::from_secs(45),
    )
    .await?;

    for node in &nodes {
        let gateway = gateway_addr(node, ingress_port);
        let current = query_meta_prefix_via_cluster_inter_route(
            &client,
            gateway.as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            8,
        )
        .await?;
        require_meta_values(
            &current,
            &[
                (&key_a, "a-after-crash", r1, r2, 2),
                (&key_b, "b-after-crash", r3, r3, 1),
            ],
        )?;
    }

    expect_meta_put_status_via_cluster_inter_route(
        &client,
        old_leader_gateway.as_str(),
        route_prefix,
        old_leader.name.as_str(),
        MetaPutRequest {
            key: key_a.clone(),
            value: "stale-old-leader-write".to_string(),
            node_name: Some(old_leader.name.clone()),
            expected_revision: Some(r1),
        },
        StatusCode::CONFLICT,
    )
    .await?;

    let a_v3 = put_meta_via_cluster_inter_route(
        &client,
        old_leader_gateway.as_str(),
        route_prefix,
        old_leader.name.as_str(),
        key_a.as_str(),
        "a-after-rejoin",
        Some(r2),
    )
    .await?;
    let r4 = a_v3.mod_revision;
    if a_v3.create_revision != r1 || a_v3.version != 3 || r4 != r3 + 1 {
        return Err(format!("unexpected old-leader-rejoin a_v3: {:?}", a_v3));
    }
    wait_meta_value_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        key_a.as_str(),
        "a-after-rejoin",
        r4,
        Duration::from_secs(45),
    )
    .await?;

    let changes = query_meta_changes_via_cluster_inter_route(
        &client,
        old_leader_gateway.as_str(),
        route_prefix,
        old_leader.name.as_str(),
        prefix.as_str(),
        r1,
        8,
        None,
    )
    .await?;
    require_meta_changes(
        &changes,
        &[
            (r1, &key_a, "a-before-crash", false, r1, 1),
            (r2, &key_a, "a-after-crash", false, r1, 2),
            (r3, &key_b, "b-after-crash", false, r3, 1),
            (r4, &key_a, "a-after-rejoin", false, r1, 3),
        ],
    )?;

    println!(
        "[klog-cluster-dv] raft old leader rejoin ok: old_leader={}, new_leader={}, leader_after_rejoin={}, log_ids=[{},{}], revisions=[{},{},{},{}], prefix={}",
        old_leader_id,
        new_leader_id,
        leader_after_rejoin,
        before_log.id,
        after_log.id,
        r1,
        r2,
        r3,
        r4,
        prefix
    );
    Ok(())
}

async fn run_local_gateway_raft_old_leader_rejoin() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_raft_old_leader_rejoin_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_raft_follower_lag_snapshot_install_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-raft-follower-lag-snapshot-dv";
    let setup = prepare_local_gateway_setup(
        harness,
        RAFT_FOLLOWER_LAG_SNAPSHOT_INSTALL_MODE,
        route_prefix,
        3,
    )
    .await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let seed = nodes
        .first()
        .cloned()
        .ok_or_else(|| "missing seed node".to_string())?;
    let raft_patch = ood_snapshot_membership_raft_patch();
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };
    let mut configs = BTreeMap::new();
    for node in &nodes {
        let config = write_klog_config_with_raft_patch(harness, node, &voter_config, raft_patch)?;
        configs.insert(node.id, config.clone());
        spawn_klog_with_log_level(harness, &klog_daemon_bin, &config, node, "info")?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        wait_tcp("127.0.0.1", node.ports.inter, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let initial_leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let lagged_follower = nodes
        .iter()
        .find(|node| node.id != initial_leader_id && node.id != seed.id)
        .or_else(|| nodes.iter().find(|node| node.id != initial_leader_id))
        .cloned()
        .ok_or_else(|| {
            format!(
                "failed to pick lagged follower: leader={}",
                initial_leader_id
            )
        })?;
    let alive_nodes = nodes
        .iter()
        .filter(|node| node.id != lagged_follower.id)
        .cloned()
        .collect::<Vec<_>>();
    let writer = alive_nodes
        .first()
        .cloned()
        .ok_or_else(|| "missing writer node after picking lagged follower".to_string())?;
    let target = alive_nodes
        .iter()
        .find(|node| node.id != writer.id)
        .cloned()
        .unwrap_or_else(|| writer.clone());

    let baseline = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &writer,
        &target,
        &nodes,
        "raft-follower-lag-before-stop",
    )
    .await?;

    harness.stop(format!("klog-{}", lagged_follower.name).as_str())?;
    wait_membership(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let active_leader_id = wait_consistent_leader(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(50),
    )
    .await?;

    let bulk = write_snapshot_bulk_data(
        &client,
        ingress_port,
        route_prefix,
        &writer,
        &target,
        "raft-follower-lag-snapshot-install",
        DEFAULT_OOD_SNAPSHOT_MEMBERSHIP_ITEMS,
        DEFAULT_OOD_SNAPSHOT_MEMBERSHIP_VALUE_BYTES,
    )
    .await?;
    verify_snapshot_bulk_witness(
        &client,
        ingress_port,
        route_prefix,
        &alive_nodes,
        &bulk,
        Duration::from_secs(60),
    )
    .await?;
    let snapshot_leader_id = wait_consistent_leader(
        &client,
        &alive_nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(50),
    )
    .await?;
    let snapshot_leader = alive_nodes
        .iter()
        .find(|node| node.id == snapshot_leader_id)
        .ok_or_else(|| format!("snapshot leader node {} not found", snapshot_leader_id))?;
    let leader_snapshot_files =
        wait_snapshot_file_count(harness, snapshot_leader, 1, Duration::from_secs(90)).await?;

    let lagged_config = configs
        .get(&lagged_follower.id)
        .ok_or_else(|| format!("missing config for lagged follower {}", lagged_follower.id))?;
    spawn_klog_with_log_level(
        harness,
        &klog_daemon_bin,
        lagged_config,
        &lagged_follower,
        "info",
    )?;
    wait_tcp(
        "127.0.0.1",
        lagged_follower.ports.admin,
        Duration::from_secs(12),
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(120),
    )
    .await?;
    wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(80),
    )
    .await?;
    let follower_snapshot_files =
        wait_snapshot_file_count(harness, &lagged_follower, 1, Duration::from_secs(120)).await?;

    verify_log_and_meta_witness_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &baseline,
        Duration::from_secs(60),
    )
    .await?;
    verify_snapshot_bulk_witness(
        &client,
        ingress_port,
        route_prefix,
        &nodes,
        &bulk,
        Duration::from_secs(90),
    )
    .await?;

    let recovered_write = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &lagged_follower,
        snapshot_leader,
        &nodes,
        "raft-follower-lag-after-recovery",
    )
    .await?;

    println!(
        "[klog-cluster-dv] raft follower lag snapshot install ok: initial_leader={}, active_leader={}, snapshot_leader={}, lagged_follower={}, leader_snapshots={}, follower_snapshots={}, bulk_items={}, recovered_log_id={}, prefix={}",
        initial_leader_id,
        active_leader_id,
        snapshot_leader_id,
        lagged_follower.id,
        leader_snapshot_files,
        follower_snapshot_files,
        bulk.expected_meta_count,
        recovered_write.log_id,
        bulk.meta_prefix
    );
    Ok(())
}

async fn run_local_gateway_raft_follower_lag_snapshot_install() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_raft_follower_lag_snapshot_install_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_raft_snapshot_install_crash_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let item_count = parse_env_usize(
        ENV_RAFT_SNAPSHOT_INSTALL_CRASH_ITEMS,
        DEFAULT_RAFT_SNAPSHOT_INSTALL_CRASH_ITEMS,
    )?;
    let value_bytes = parse_env_usize(
        ENV_RAFT_SNAPSHOT_INSTALL_CRASH_VALUE_BYTES,
        DEFAULT_RAFT_SNAPSHOT_INSTALL_CRASH_VALUE_BYTES,
    )?;
    let chunk_bytes = parse_env_usize(
        ENV_RAFT_SNAPSHOT_INSTALL_CRASH_CHUNK_BYTES,
        DEFAULT_RAFT_SNAPSHOT_INSTALL_CRASH_CHUNK_BYTES,
    )?;
    if item_count == 0 || value_bytes == 0 || chunk_bytes == 0 {
        return Err(format!(
            "{}={}, {}={}, and {}={} must all be greater than 0",
            ENV_RAFT_SNAPSHOT_INSTALL_CRASH_ITEMS,
            item_count,
            ENV_RAFT_SNAPSHOT_INSTALL_CRASH_VALUE_BYTES,
            value_bytes,
            ENV_RAFT_SNAPSHOT_INSTALL_CRASH_CHUNK_BYTES,
            chunk_bytes
        ));
    }

    let route_prefix = "/.cluster/klog-it-raft-snapshot-install-crash-dv";
    let setup =
        prepare_local_gateway_setup(harness, RAFT_SNAPSHOT_INSTALL_CRASH_MODE, route_prefix, 4)
            .await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let base_voters = nodes.iter().take(3).cloned().collect::<Vec<_>>();
    let learner = nodes
        .get(3)
        .cloned()
        .ok_or_else(|| "missing snapshot install crash learner node".to_string())?;
    let seed = base_voters
        .first()
        .cloned()
        .ok_or_else(|| "missing snapshot install crash seed node".to_string())?;
    let raft_patch = raft_snapshot_install_crash_raft_patch(chunk_bytes);
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };
    for node in &base_voters {
        let config = write_klog_config_with_raft_patch(harness, node, &voter_config, raft_patch)?;
        spawn_klog_with_log_level(harness, &klog_daemon_bin, &config, node, "info")?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        wait_tcp("127.0.0.1", node.ports.inter, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let writer = base_voters
        .first()
        .cloned()
        .ok_or_else(|| "missing snapshot install crash writer".to_string())?;
    let target = base_voters
        .get(1)
        .cloned()
        .ok_or_else(|| "missing snapshot install crash target".to_string())?;
    let bulk = write_snapshot_bulk_data(
        &client,
        ingress_port,
        route_prefix,
        &writer,
        &target,
        "raft-snapshot-install-crash-prejoin",
        item_count,
        value_bytes,
    )
    .await?;
    verify_snapshot_bulk_witness(
        &client,
        ingress_port,
        route_prefix,
        &base_voters,
        &bulk,
        Duration::from_secs(80),
    )
    .await?;
    let snapshot_leader_id = wait_consistent_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(50),
    )
    .await?;
    let snapshot_leader = base_voters
        .iter()
        .find(|node| node.id == snapshot_leader_id)
        .ok_or_else(|| format!("snapshot leader node {} not found", snapshot_leader_id))?;
    let leader_snapshot_files =
        wait_snapshot_file_count(harness, snapshot_leader, 1, Duration::from_secs(100)).await?;

    let learner_options = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: false,
        target_role: "learner",
    };
    let learner_config =
        write_klog_config_with_raft_patch(harness, &learner, &learner_options, raft_patch)?;
    spawn_klog_with_log_level(harness, &klog_daemon_bin, &learner_config, &learner, "info")?;
    wait_tcp("127.0.0.1", learner.ports.admin, Duration::from_secs(12)).await?;
    wait_tcp("127.0.0.1", learner.ports.inter, Duration::from_secs(12)).await?;

    let add_leader_id = wait_consistent_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(50),
    )
    .await?;
    let add_leader = base_voters
        .iter()
        .find(|node| node.id == add_leader_id)
        .ok_or_else(|| format!("add-learner leader node {} not found", add_leader_id))?;
    post_add_learner_via_admin_route(
        &client,
        gateway_addr(add_leader, ingress_port).as_str(),
        route_prefix,
        add_leader.name.as_str(),
        &learner,
        false,
    )
    .await?;
    wait_membership(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[4],
        Duration::from_secs(80),
    )
    .await?;
    let temp_bytes =
        wait_snapshot_temp_file_exists(harness, &learner, Duration::from_secs(120)).await?;
    harness.stop(format!("klog-{}", learner.name).as_str())?;
    let snapshot_files_before_restart = snapshot_file_count(harness, &learner)?;

    spawn_klog_with_log_level(harness, &klog_daemon_bin, &learner_config, &learner, "info")?;
    wait_tcp("127.0.0.1", learner.ports.admin, Duration::from_secs(12)).await?;
    wait_tcp("127.0.0.1", learner.ports.inter, Duration::from_secs(12)).await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[4],
        Duration::from_secs(160),
    )
    .await?;
    wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(90),
    )
    .await?;
    let learner_snapshot_files =
        wait_snapshot_file_count(harness, &learner, 1, Duration::from_secs(160)).await?;
    verify_snapshot_bulk_witness(
        &client,
        ingress_port,
        route_prefix,
        &nodes,
        &bulk,
        Duration::from_secs(120),
    )
    .await?;

    let after_restart = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &learner,
        snapshot_leader,
        &nodes,
        "raft-snapshot-install-crash-after-restart",
    )
    .await?;

    println!(
        "[klog-cluster-dv] raft snapshot install crash ok: add_leader={}, snapshot_leader={}, learner={}, leader_snapshots={}, learner_snapshots_before_restart={}, learner_snapshots_after_restart={}, temp_bytes_before_kill={}, bulk_items={}, value_bytes={}, chunk_bytes={}, recovered_log_id={}, prefix={}",
        add_leader_id,
        snapshot_leader_id,
        learner.id,
        leader_snapshot_files,
        snapshot_files_before_restart,
        learner_snapshot_files,
        temp_bytes,
        bulk.expected_meta_count,
        value_bytes,
        chunk_bytes,
        after_restart.log_id,
        bulk.meta_prefix
    );
    Ok(())
}

async fn run_local_gateway_raft_snapshot_install_crash() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_raft_snapshot_install_crash_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_raft_quorum_loss_recovery_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-raft-quorum-loss-recovery-dv";
    let setup =
        prepare_local_gateway_setup(harness, RAFT_QUORUM_LOSS_RECOVERY_MODE, route_prefix, 3)
            .await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let seed = nodes
        .first()
        .cloned()
        .ok_or_else(|| "missing seed node".to_string())?;
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };
    let mut configs = BTreeMap::new();
    for node in &nodes {
        let config = write_klog_config(harness, node, &voter_config)?;
        configs.insert(node.id, config.clone());
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        wait_tcp("127.0.0.1", node.ports.inter, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let survivor = nodes
        .iter()
        .find(|node| node.id == leader_id)
        .cloned()
        .ok_or_else(|| format!("leader node {} not found", leader_id))?;
    let stopped_nodes = nodes
        .iter()
        .filter(|node| node.id != leader_id)
        .cloned()
        .collect::<Vec<_>>();
    if stopped_nodes.len() != 2 {
        return Err(format!(
            "expected two followers to stop, got {}",
            stopped_nodes.len()
        ));
    }
    let baseline = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &stopped_nodes[0],
        &survivor,
        &nodes,
        "raft-quorum-loss-before-loss",
    )
    .await?;

    for node in &stopped_nodes {
        harness.stop(format!("klog-{}", node.name).as_str())?;
    }
    tokio::time::sleep(Duration::from_secs(2)).await;

    let survivor_gateway = gateway_addr(&survivor, ingress_port);
    if append_via_cluster_inter_route(
        &client,
        survivor_gateway.as_str(),
        route_prefix,
        survivor.name.as_str(),
        "test/raft-quorum-loss/unavailable-log",
        "write should fail without quorum",
    )
    .await
    .is_ok()
    {
        return Err(format!(
            "single survivor {} unexpectedly accepted append without quorum",
            survivor.id
        ));
    }

    if query_via_cluster_inter_route(
        &client,
        survivor_gateway.as_str(),
        route_prefix,
        survivor.name.as_str(),
        baseline.log_id,
        baseline.log_source.as_str(),
    )
    .await
    .is_ok()
    {
        return Err(format!(
            "single survivor {} unexpectedly served strong read without quorum",
            survivor.id
        ));
    }

    let suffix = unique_suffix("raft-quorum-loss-recovery");
    let prefix = format!("test/klog_raft_quorum_loss_recovery_dv/{}/", suffix);
    let doomed_key = format!("{}doomed", prefix);
    if put_meta_via_cluster_inter_route(
        &client,
        survivor_gateway.as_str(),
        route_prefix,
        survivor.name.as_str(),
        doomed_key.as_str(),
        "should-not-commit-without-quorum",
        Some(0),
    )
    .await
    .is_ok()
    {
        return Err(format!(
            "single survivor {} unexpectedly accepted meta put without quorum",
            survivor.id
        ));
    }

    let first_restored = stopped_nodes[0].clone();
    let first_restored_config = configs
        .get(&first_restored.id)
        .ok_or_else(|| format!("missing config for restored node {}", first_restored.id))?;
    spawn_klog(
        harness,
        &klog_daemon_bin,
        first_restored_config,
        &first_restored,
    )?;
    wait_tcp(
        "127.0.0.1",
        first_restored.ports.admin,
        Duration::from_secs(12),
    )
    .await?;
    wait_tcp(
        "127.0.0.1",
        first_restored.ports.inter,
        Duration::from_secs(12),
    )
    .await?;
    let quorum_nodes = vec![survivor.clone(), first_restored.clone()];
    wait_membership(
        &client,
        &quorum_nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(80),
    )
    .await?;
    let quorum_leader_id = wait_consistent_leader(
        &client,
        &quorum_nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(80),
    )
    .await?;
    let quorum_leader = quorum_nodes
        .iter()
        .find(|node| node.id == quorum_leader_id)
        .cloned()
        .ok_or_else(|| format!("quorum leader node {} not found", quorum_leader_id))?;
    let quorum_writer = quorum_nodes
        .iter()
        .find(|node| node.id != quorum_leader_id)
        .cloned()
        .unwrap_or_else(|| quorum_leader.clone());
    tokio::time::sleep(Duration::from_secs(3)).await;

    let pre_recovery_query = query_meta_via_cluster_inter_route(
        &client,
        gateway_addr(&quorum_writer, ingress_port).as_str(),
        route_prefix,
        quorum_leader.name.as_str(),
        doomed_key.as_str(),
    )
    .await?;
    if !pre_recovery_query.items.is_empty() {
        return Err(format!(
            "no-quorum meta write was applied after quorum recovery: key={}, items={:?}",
            doomed_key, pre_recovery_query.items
        ));
    }

    let recovered_meta = put_meta_via_cluster_inter_route(
        &client,
        gateway_addr(&quorum_writer, ingress_port).as_str(),
        route_prefix,
        quorum_leader.name.as_str(),
        doomed_key.as_str(),
        "committed-after-quorum-recovery",
        Some(0),
    )
    .await?;
    if recovered_meta.create_revision != recovered_meta.mod_revision || recovered_meta.version != 1
    {
        return Err(format!(
            "unexpected recovered quorum meta version: {:?}",
            recovered_meta
        ));
    }
    wait_meta_value_on_nodes(
        &client,
        &quorum_nodes,
        ingress_port,
        route_prefix,
        doomed_key.as_str(),
        "committed-after-quorum-recovery",
        recovered_meta.mod_revision,
        Duration::from_secs(40),
    )
    .await?;

    verify_log_and_meta_witness_on_nodes(
        &client,
        &quorum_nodes,
        ingress_port,
        route_prefix,
        &baseline,
        Duration::from_secs(40),
    )
    .await?;
    let after_quorum = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &quorum_writer,
        &quorum_leader,
        &quorum_nodes,
        "raft-quorum-loss-after-one-restore",
    )
    .await?;

    let second_restored = stopped_nodes[1].clone();
    let second_restored_config = configs
        .get(&second_restored.id)
        .ok_or_else(|| format!("missing config for restored node {}", second_restored.id))?;
    spawn_klog(
        harness,
        &klog_daemon_bin,
        second_restored_config,
        &second_restored,
    )?;
    wait_tcp(
        "127.0.0.1",
        second_restored.ports.admin,
        Duration::from_secs(12),
    )
    .await?;
    wait_tcp(
        "127.0.0.1",
        second_restored.ports.inter,
        Duration::from_secs(12),
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(100),
    )
    .await?;
    let final_leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(80),
    )
    .await?;
    let final_leader = nodes
        .iter()
        .find(|node| node.id == final_leader_id)
        .cloned()
        .ok_or_else(|| format!("final leader node {} not found", final_leader_id))?;
    verify_log_and_meta_witness_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &baseline,
        Duration::from_secs(50),
    )
    .await?;
    verify_log_and_meta_witness_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &after_quorum,
        Duration::from_secs(50),
    )
    .await?;
    wait_meta_value_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        doomed_key.as_str(),
        "committed-after-quorum-recovery",
        recovered_meta.mod_revision,
        Duration::from_secs(50),
    )
    .await?;
    let final_write = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &second_restored,
        &final_leader,
        &nodes,
        "raft-quorum-loss-after-all-recovered",
    )
    .await?;

    println!(
        "[klog-cluster-dv] raft quorum loss recovery ok: survivor={}, stopped=[{},{}], quorum_leader={}, final_leader={}, baseline_log_id={}, recovered_log_id={}, final_log_id={}, recovered_revision={}, prefix={}",
        survivor.id,
        stopped_nodes[0].id,
        stopped_nodes[1].id,
        quorum_leader_id,
        final_leader_id,
        baseline.log_id,
        after_quorum.log_id,
        final_write.log_id,
        recovered_meta.mod_revision,
        prefix
    );
    Ok(())
}

async fn run_local_gateway_raft_quorum_loss_recovery() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_raft_quorum_loss_recovery_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_raft_membership_change_rejoin_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-raft-membership-change-rejoin-dv";
    let setup =
        prepare_local_gateway_setup(harness, RAFT_MEMBERSHIP_CHANGE_REJOIN_MODE, route_prefix, 3)
            .await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let seed = nodes
        .first()
        .cloned()
        .ok_or_else(|| "missing seed node".to_string())?;
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };
    let mut configs = BTreeMap::new();
    for node in &nodes {
        let config = write_klog_config(harness, node, &voter_config)?;
        configs.insert(node.id, config.clone());
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        wait_tcp("127.0.0.1", node.ports.inter, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let initial_leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let removed_node = nodes
        .iter()
        .find(|node| node.id != initial_leader_id)
        .cloned()
        .ok_or_else(|| {
            format!(
                "failed to choose non-leader voter, leader={}",
                initial_leader_id
            )
        })?;
    let active_nodes = nodes
        .iter()
        .filter(|node| node.id != removed_node.id)
        .cloned()
        .collect::<Vec<_>>();
    if active_nodes.len() != 2 {
        return Err(format!(
            "expected two active voters after picking removed node, got {}",
            active_nodes.len()
        ));
    }
    let active_voters = active_nodes.iter().map(|node| node.id).collect::<Vec<_>>();

    let before_remove = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &active_nodes[0],
        &removed_node,
        &nodes,
        "raft-membership-rejoin-before-remove",
    )
    .await?;

    harness.stop(format!("klog-{}", removed_node.name).as_str())?;
    let shrink_leader_id = change_voters_via_current_leader(
        &client,
        &active_nodes,
        ingress_port,
        route_prefix,
        active_voters.as_slice(),
        false,
    )
    .await?;
    wait_membership(
        &client,
        &active_nodes,
        ingress_port,
        route_prefix,
        active_voters.as_slice(),
        &[],
        Duration::from_secs(80),
    )
    .await?;
    let active_leader_id = wait_consistent_leader(
        &client,
        &active_nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(50),
    )
    .await?;
    let active_leader = active_nodes
        .iter()
        .find(|node| node.id == active_leader_id)
        .cloned()
        .ok_or_else(|| format!("active leader node {} not found", active_leader_id))?;
    let active_writer = active_nodes
        .iter()
        .find(|node| node.id != active_leader_id)
        .cloned()
        .unwrap_or_else(|| active_leader.clone());
    verify_log_and_meta_witness_on_nodes(
        &client,
        &active_nodes,
        ingress_port,
        route_prefix,
        &before_remove,
        Duration::from_secs(40),
    )
    .await?;
    let after_shrink = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &active_writer,
        &active_leader,
        &active_nodes,
        "raft-membership-rejoin-after-shrink",
    )
    .await?;

    let removed_config = configs
        .get(&removed_node.id)
        .ok_or_else(|| format!("missing config for removed node {}", removed_node.id))?;
    spawn_klog(harness, &klog_daemon_bin, removed_config, &removed_node)?;
    wait_tcp(
        "127.0.0.1",
        removed_node.ports.admin,
        Duration::from_secs(12),
    )
    .await?;
    wait_tcp(
        "127.0.0.1",
        removed_node.ports.inter,
        Duration::from_secs(12),
    )
    .await?;
    tokio::time::sleep(Duration::from_secs(3)).await;

    wait_membership(
        &client,
        &active_nodes,
        ingress_port,
        route_prefix,
        active_voters.as_slice(),
        &[],
        Duration::from_secs(30),
    )
    .await?;
    let (stale_status, stale_body) = post_change_membership_via_admin_route(
        &client,
        gateway_addr(&removed_node, ingress_port).as_str(),
        route_prefix,
        removed_node.name.as_str(),
        &[1, 2, 3],
        true,
    )
    .await?;
    tokio::time::sleep(Duration::from_secs(3)).await;
    wait_membership(
        &client,
        &active_nodes,
        ingress_port,
        route_prefix,
        active_voters.as_slice(),
        &[],
        Duration::from_secs(30),
    )
    .await?;
    let after_stale_restart = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &active_writer,
        &active_leader,
        &active_nodes,
        "raft-membership-rejoin-after-stale-restart",
    )
    .await?;

    post_add_learner_via_admin_route(
        &client,
        gateway_addr(&active_leader, ingress_port).as_str(),
        route_prefix,
        active_leader.name.as_str(),
        &removed_node,
        true,
    )
    .await?;
    let learner_ids = [removed_node.id];
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        active_voters.as_slice(),
        &learner_ids,
        Duration::from_secs(90),
    )
    .await?;
    for witness in [&before_remove, &after_shrink, &after_stale_restart] {
        verify_log_and_meta_witness_on_nodes(
            &client,
            &nodes,
            ingress_port,
            route_prefix,
            witness,
            Duration::from_secs(60),
        )
        .await?;
    }

    let mut promoted_voters = active_voters.clone();
    promoted_voters.push(removed_node.id);
    promoted_voters.sort_unstable();
    let promote_leader_id = change_voters_via_current_leader(
        &client,
        &active_nodes,
        ingress_port,
        route_prefix,
        promoted_voters.as_slice(),
        true,
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        promoted_voters.as_slice(),
        &[],
        Duration::from_secs(90),
    )
    .await?;
    let final_leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(70),
    )
    .await?;
    let final_leader = nodes
        .iter()
        .find(|node| node.id == final_leader_id)
        .cloned()
        .ok_or_else(|| format!("final leader node {} not found", final_leader_id))?;
    let final_write = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &removed_node,
        &final_leader,
        &nodes,
        "raft-membership-rejoin-after-promote",
    )
    .await?;
    for witness in [
        &before_remove,
        &after_shrink,
        &after_stale_restart,
        &final_write,
    ] {
        verify_log_and_meta_witness_on_nodes(
            &client,
            &nodes,
            ingress_port,
            route_prefix,
            witness,
            Duration::from_secs(60),
        )
        .await?;
    }

    println!(
        "[klog-cluster-dv] raft membership change rejoin ok: initial_leader={}, removed_node={}, shrink_leader={}, active_leader={}, stale_admin_status={}, stale_admin_body_len={}, promote_leader={}, final_leader={}, active_voters={:?}, promoted_voters={:?}, log_ids=[{},{},{},{}]",
        initial_leader_id,
        removed_node.id,
        shrink_leader_id,
        active_leader_id,
        stale_status,
        stale_body.len(),
        promote_leader_id,
        final_leader_id,
        active_voters,
        promoted_voters,
        before_remove.log_id,
        after_shrink.log_id,
        after_stale_restart.log_id,
        final_write.log_id
    );
    Ok(())
}

async fn run_local_gateway_raft_membership_change_rejoin() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_raft_membership_change_rejoin_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_node_id_reuse_inner(harness: &mut LocalHarness) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-node-id-reuse-dv";
    let setup = prepare_local_gateway_setup(harness, NODE_ID_REUSE_MODE, route_prefix, 3).await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let seed = nodes
        .first()
        .cloned()
        .ok_or_else(|| "missing node-id reuse seed node".to_string())?;
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };
    for node in &nodes {
        let config = write_klog_config(harness, node, &voter_config)?;
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        wait_tcp("127.0.0.1", node.ports.rpc, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(50),
    )
    .await?;
    let leader_node = nodes
        .iter()
        .find(|node| node.id == leader_id)
        .cloned()
        .ok_or_else(|| format!("node-id reuse leader {} not found", leader_id))?;
    let reused_node = nodes
        .iter()
        .find(|node| node.id != seed.id)
        .cloned()
        .ok_or_else(|| "missing non-seed node for node-id reuse".to_string())?;
    let mut used_ports = collect_used_ports(&nodes, ingress_port);
    let replacement = LocalNodeDef {
        id: reused_node.id,
        name: format!("{}-replacement", reused_node.name),
        device_id: format!("did:dv:{}-replacement", reused_node.name),
        gateway_host: "127.0.0.4".to_string(),
        ports: LocalNodePorts {
            raft: pick_local_port(&mut used_ports)?,
            inter: pick_local_port(&mut used_ports)?,
            admin: pick_local_port(&mut used_ports)?,
            rpc: pick_local_port(&mut used_ports)?,
            rtcp: pick_local_port(&mut used_ports)?,
            zone_http: pick_local_port(&mut used_ports)?,
            control: pick_local_port(&mut used_ports)?,
        },
    };

    let (duplicate_status, duplicate_body) = post_add_learner_via_admin_route_status(
        &client,
        gateway_addr(&leader_node, ingress_port).as_str(),
        route_prefix,
        leader_node.name.as_str(),
        &replacement,
        true,
    )
    .await?;
    if duplicate_status.is_success() {
        return Err(format!(
            "duplicate add-learner unexpectedly succeeded: reused_node_id={}, replacement={:?}, body={}",
            replacement.id, replacement, duplicate_body
        ));
    }
    require_node_id_reuse_error(
        format!(
            "duplicate add-learner node_id={} status={} body={}",
            replacement.id, duplicate_status, duplicate_body
        )
        .as_str(),
        "duplicate admin add-learner",
    )?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(30),
    )
    .await?;

    let join_targets = vec![format!("127.0.0.1:{}", leader_node.ports.admin)];
    let replacement_options = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: false,
        target_role: "voter",
    };
    let replacement_retry = KLogJoinRetryPatch {
        initial_interval_ms: 200,
        max_interval_ms: 200,
        max_attempts: 1,
        request_timeout_ms: 1000,
    };
    let replacement_config = write_klog_config_with_join_targets_and_retry_patch(
        harness,
        &replacement,
        &replacement_options,
        join_targets.as_slice(),
        replacement_retry,
        KLogRaftPatch::default(),
    )?;
    spawn_klog_with_log_level(
        harness,
        &klog_daemon_bin,
        &replacement_config,
        &replacement,
        "info",
    )?;
    wait_tcp(
        "127.0.0.1",
        replacement.ports.admin,
        Duration::from_secs(12),
    )
    .await?;
    wait_tcp("127.0.0.1", replacement.ports.rpc, Duration::from_secs(12)).await?;
    let node_id_pattern = format!("node_id={}", replacement.id);
    let replacement_name_pattern = format!("expected={} remote=", replacement.name);
    let replacement_device_pattern = format!("expected={} remote=", replacement.device_id);
    let join_log = wait_klog_out_log_contains(
        harness,
        &replacement,
        &[
            "Auto-join reached max attempts without success",
            "node identity mismatch",
            node_id_pattern.as_str(),
            "node_name",
            replacement_name_pattern.as_str(),
            "device_id",
            replacement_device_pattern.as_str(),
        ],
        Duration::from_secs(12),
    )
    .await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(30),
    )
    .await?;
    let witness = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &leader_node,
        &reused_node,
        &nodes,
        "node-id-reuse-after-rejected-replacement",
    )
    .await?;
    verify_log_and_meta_witness_on_nodes(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &witness,
        Duration::from_secs(40),
    )
    .await?;

    println!(
        "[klog-cluster-dv] node id reuse ok: leader={}, reused_node={}, replacement_name={}, duplicate_status={}, duplicate_body_len={}, join_log_len={}, log_id={}, meta_key={}",
        leader_id,
        reused_node.id,
        replacement.name,
        duplicate_status,
        duplicate_body.len(),
        join_log.len(),
        witness.log_id,
        witness.meta_key
    );
    Ok(())
}

async fn run_local_gateway_node_id_reuse() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_node_id_reuse_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_raft_concurrent_membership_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-raft-concurrent-membership-dv";
    let setup =
        prepare_local_gateway_setup(harness, RAFT_CONCURRENT_MEMBERSHIP_MODE, route_prefix, 5)
            .await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let base_voters = nodes.iter().take(3).cloned().collect::<Vec<_>>();
    let candidate_a = nodes
        .get(3)
        .cloned()
        .ok_or_else(|| "missing concurrent membership candidate A".to_string())?;
    let candidate_b = nodes
        .get(4)
        .cloned()
        .ok_or_else(|| "missing concurrent membership candidate B".to_string())?;
    let seed = base_voters
        .first()
        .cloned()
        .ok_or_else(|| "missing concurrent membership seed node".to_string())?;
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };
    for node in &base_voters {
        let config = write_klog_config(harness, node, &voter_config)?;
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        wait_tcp("127.0.0.1", node.ports.inter, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let candidate_config_options = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: false,
        target_role: "learner",
    };
    for candidate in [&candidate_a, &candidate_b] {
        let config = write_klog_config(harness, candidate, &candidate_config_options)?;
        spawn_klog(harness, &klog_daemon_bin, &config, candidate)?;
        wait_tcp("127.0.0.1", candidate.ports.admin, Duration::from_secs(12)).await?;
        wait_tcp("127.0.0.1", candidate.ports.inter, Duration::from_secs(12)).await?;
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let before_concurrent = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &base_voters[0],
        &base_voters[1],
        &base_voters,
        "raft-concurrent-membership-before",
    )
    .await?;

    let leader_id = wait_consistent_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let leader = base_voters
        .iter()
        .find(|node| node.id == leader_id)
        .cloned()
        .ok_or_else(|| format!("leader node {} not found", leader_id))?;
    let leader_gateway = gateway_addr(&leader, ingress_port);
    let add_a = post_add_learner_via_admin_route_status(
        &client,
        leader_gateway.as_str(),
        route_prefix,
        leader.name.as_str(),
        &candidate_a,
        true,
    );
    let add_b = post_add_learner_via_admin_route_status(
        &client,
        leader_gateway.as_str(),
        route_prefix,
        leader.name.as_str(),
        &candidate_b,
        true,
    );
    let ((status_a, body_a), (status_b, body_b)) = tokio::try_join!(add_a, add_b)?;
    let mut successes = Vec::new();
    let mut failures = Vec::new();
    for (candidate, status, body) in [
        (candidate_a.clone(), status_a, body_a),
        (candidate_b.clone(), status_b, body_b),
    ] {
        if status.is_success() {
            successes.push((candidate, body));
        } else {
            failures.push((candidate, status, body));
        }
    }
    if successes.len() != 1 || failures.len() != 1 {
        return Err(format!(
            "expected exactly one concurrent add-learner success and one failure, successes={}, failures={}",
            successes.len(),
            failures.len()
        ));
    }
    let (added_ood, add_success_body) = successes
        .pop()
        .ok_or_else(|| "missing successful concurrent add-learner result".to_string())?;
    let (rejected_ood, rejected_status, rejected_body) = failures
        .pop()
        .ok_or_else(|| "missing rejected concurrent add-learner result".to_string())?;
    if rejected_status != StatusCode::CONFLICT {
        return Err(format!(
            "concurrent add-learner for {} expected 409 Conflict, got status={}, body={}",
            rejected_ood.name, rejected_status, rejected_body
        ));
    }
    if !rejected_body.contains("membership change already in progress")
        && !rejected_body.contains("undergoing a configuration change")
    {
        return Err(format!(
            "concurrent add-learner rejection body missing conflict marker: node={}, body={}",
            rejected_ood.name, rejected_body
        ));
    }

    let mut member_nodes = base_voters.clone();
    member_nodes.push(added_ood.clone());
    let learner_ids = [added_ood.id];
    wait_membership(
        &client,
        &member_nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &learner_ids,
        Duration::from_secs(80),
    )
    .await?;
    tokio::time::sleep(Duration::from_secs(2)).await;
    wait_membership(
        &client,
        &member_nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &learner_ids,
        Duration::from_secs(20),
    )
    .await?;
    verify_log_and_meta_witness_on_nodes(
        &client,
        &member_nodes,
        ingress_port,
        route_prefix,
        &before_concurrent,
        Duration::from_secs(50),
    )
    .await?;

    let mut promoted_voters = vec![1, 2, 3, added_ood.id];
    promoted_voters.sort_unstable();
    let promote_leader_id = change_voters_via_current_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        promoted_voters.as_slice(),
        true,
    )
    .await?;
    wait_membership(
        &client,
        &member_nodes,
        ingress_port,
        route_prefix,
        promoted_voters.as_slice(),
        &[],
        Duration::from_secs(80),
    )
    .await?;
    let after_promote = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &added_ood,
        &base_voters[0],
        &member_nodes,
        "raft-concurrent-membership-after-promote",
    )
    .await?;
    for witness in [&before_concurrent, &after_promote] {
        verify_log_and_meta_witness_on_nodes(
            &client,
            &member_nodes,
            ingress_port,
            route_prefix,
            witness,
            Duration::from_secs(50),
        )
        .await?;
    }

    println!(
        "[klog-cluster-dv] raft concurrent membership ok: leader={}, added={}, rejected={}, rejected_status={}, promote_leader={}, voters={:?}, success_body_len={}, rejected_body_len={}, log_ids=[{},{}]",
        leader_id,
        added_ood.id,
        rejected_ood.id,
        rejected_status,
        promote_leader_id,
        promoted_voters,
        add_success_body.len(),
        rejected_body.len(),
        before_concurrent.log_id,
        after_promote.log_id
    );
    Ok(())
}

async fn run_local_gateway_raft_concurrent_membership() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_raft_concurrent_membership_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_raft_join_retry_idempotency_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-raft-join-retry-idempotency-dv";
    let setup =
        prepare_local_gateway_setup(harness, RAFT_JOIN_RETRY_IDEMPOTENCY_MODE, route_prefix, 4)
            .await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let base_voters = nodes.iter().take(3).cloned().collect::<Vec<_>>();
    let added_ood = nodes
        .get(3)
        .cloned()
        .ok_or_else(|| "missing join retry learner node".to_string())?;
    let seed = base_voters
        .first()
        .cloned()
        .ok_or_else(|| "missing join retry seed node".to_string())?;
    let raft_patch = ood_snapshot_membership_raft_patch();
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };
    for node in &base_voters {
        let config = write_klog_config_with_raft_patch(harness, node, &voter_config, raft_patch)?;
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        wait_tcp("127.0.0.1", node.ports.inter, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;

    let pre_join_witness = write_snapshot_bulk_data(
        &client,
        ingress_port,
        route_prefix,
        &base_voters[0],
        &base_voters[1],
        "join-retry-idempotency-prejoin",
        220,
        1024,
    )
    .await?;
    verify_snapshot_bulk_witness(
        &client,
        ingress_port,
        route_prefix,
        &base_voters,
        &pre_join_witness,
        Duration::from_secs(50),
    )
    .await?;

    let join_targets = base_voters
        .iter()
        .map(|target| gateway_admin_join_target(&added_ood, ingress_port, route_prefix, target))
        .collect::<Vec<_>>();
    let added_options = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: false,
        target_role: "learner",
    };
    let retry_patch = KLogJoinRetryPatch {
        initial_interval_ms: 100,
        max_interval_ms: 100,
        max_attempts: 80,
        request_timeout_ms: 20,
    };
    let added_config = write_klog_config_with_join_targets_and_retry_patch(
        harness,
        &added_ood,
        &added_options,
        &join_targets,
        retry_patch,
        raft_patch,
    )?;
    spawn_klog_with_log_level(harness, &klog_daemon_bin, &added_config, &added_ood, "info")?;
    wait_tcp("127.0.0.1", added_ood.ports.admin, Duration::from_secs(12)).await?;
    wait_tcp("127.0.0.1", added_ood.ports.inter, Duration::from_secs(12)).await?;

    let member_nodes = base_voters
        .iter()
        .cloned()
        .chain(std::iter::once(added_ood.clone()))
        .collect::<Vec<_>>();
    wait_membership(
        &client,
        &member_nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[4],
        Duration::from_secs(120),
    )
    .await?;
    let join_log = wait_klog_out_log_contains(
        harness,
        &added_ood,
        &[
            "add-learner request send failed",
            "Auto-join skip add-learner because node is already learner",
            "Auto-join succeeded",
        ],
        Duration::from_secs(20),
    )
    .await?;
    if join_log.contains("Auto-join promote learner to voter") {
        return Err("auto-join unexpectedly promoted learner target role to voter".to_string());
    }

    tokio::time::sleep(Duration::from_secs(3)).await;
    wait_membership(
        &client,
        &member_nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[4],
        Duration::from_secs(30),
    )
    .await?;
    verify_snapshot_bulk_witness(
        &client,
        ingress_port,
        route_prefix,
        &member_nodes,
        &pre_join_witness,
        Duration::from_secs(60),
    )
    .await?;

    let promote_leader_id = change_voters_via_current_leader(
        &client,
        &base_voters,
        ingress_port,
        route_prefix,
        &[1, 2, 3, 4],
        true,
    )
    .await?;
    wait_membership(
        &client,
        &member_nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3, 4],
        &[],
        Duration::from_secs(80),
    )
    .await?;
    let after_promote = write_log_and_meta_witness(
        &client,
        ingress_port,
        route_prefix,
        &added_ood,
        &base_voters[0],
        &member_nodes,
        "raft-join-retry-idempotency-after-promote",
    )
    .await?;

    println!(
        "[klog-cluster-dv] raft join retry idempotency ok: added={}, promote_leader={}, prejoin_meta_count={}, post_promote_log_id={}, timeout_ms={}, join_log_len={}",
        added_ood.id,
        promote_leader_id,
        pre_join_witness.expected_meta_count,
        after_promote.log_id,
        retry_patch.request_timeout_ms,
        join_log.len()
    );
    Ok(())
}

async fn run_local_gateway_raft_join_retry_idempotency() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_raft_join_retry_idempotency_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_system_config_kv_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-system-config-dv";
    let setup =
        prepare_local_gateway_setup(harness, SYSTEM_CONFIG_KV_MODE, route_prefix, 3).await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let seed = nodes
        .first()
        .ok_or_else(|| "missing seed node".to_string())?
        .clone();
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };

    for node in &nodes {
        let config = write_klog_config(harness, node, &voter_config)?;
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(50),
    )
    .await?;
    let leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let source = nodes
        .first()
        .ok_or_else(|| "missing source gateway node".to_string())?;
    let target = nodes
        .iter()
        .find(|node| node.name != source.name)
        .ok_or_else(|| "missing target gateway node".to_string())?;
    let source_gateway_addr = gateway_addr(source, ingress_port);
    let suffix = unique_suffix("syscfg");
    let key_prefix = format!("test/system_config_kv/{}/", suffix);
    let boot_key = format!("{}boot/config", key_prefix);
    let node_config_key = format!("{}nodes/ood1/config", key_prefix);
    let device_info_key = format!("{}devices/ood1/info", key_prefix);
    let deleted_key = format!("{}nodes/ood2/config", key_prefix);

    let boot_value_v1 = r#"{"oods":["ood1","ood2","ood3"],"revision":1}"#;
    let boot_created = put_meta_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        boot_key.as_str(),
        boot_value_v1,
        Some(0),
    )
    .await?;
    if boot_created.revision != 1 {
        return Err(format!(
            "system-config create expected revision 1, got {}",
            boot_created.revision
        ));
    }
    expect_meta_put_status_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        MetaPutRequest {
            key: boot_key.clone(),
            value: boot_value_v1.to_string(),
            node_name: Some(target.name.clone()),
            expected_revision: Some(0),
        },
        StatusCode::CONFLICT,
    )
    .await?;

    let boot_value_v2 = r#"{"oods":["ood1","ood2","ood3"],"revision":2}"#;
    let boot_updated = put_meta_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        boot_key.as_str(),
        boot_value_v2,
        Some(boot_created.revision),
    )
    .await?;
    if boot_updated.revision != 2 {
        return Err(format!(
            "system-config update expected revision 2, got {}",
            boot_updated.revision
        ));
    }
    expect_meta_put_status_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        MetaPutRequest {
            key: boot_key.clone(),
            value: r#"{"stale":true}"#.to_string(),
            node_name: Some(target.name.clone()),
            expected_revision: Some(boot_created.revision),
        },
        StatusCode::CONFLICT,
    )
    .await?;

    let fetched = query_meta_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        boot_key.as_str(),
    )
    .await?;
    require_meta_value(
        &fetched,
        boot_key.as_str(),
        boot_value_v2,
        boot_updated.revision,
    )?;

    let node_value = r#"{"kernel":{"scheduler":{},"verify-hub":{}}}"#;
    let device_value = r#"{"name":"ood1","device_type":"node"}"#;
    let deleted_value = r#"{"kernel":{"scheduler":{}}}"#;
    put_meta_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        node_config_key.as_str(),
        node_value,
        Some(0),
    )
    .await?;
    put_meta_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        device_info_key.as_str(),
        device_value,
        Some(0),
    )
    .await?;
    put_meta_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        deleted_key.as_str(),
        deleted_value,
        Some(0),
    )
    .await?;

    let listed = query_meta_prefix_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        key_prefix.as_str(),
        16,
    )
    .await?;
    require_meta_keys(
        &listed,
        &[
            boot_key.as_str(),
            node_config_key.as_str(),
            device_info_key.as_str(),
            deleted_key.as_str(),
        ],
    )?;

    let deleted = delete_meta_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        deleted_key.as_str(),
    )
    .await?;
    if deleted.key != deleted_key
        || !deleted.existed
        || deleted.prev_meta.as_ref().map(|item| item.value.as_str()) != Some(deleted_value)
    {
        return Err(format!("unexpected meta delete result: {:?}", deleted));
    }
    let deleted_query = query_meta_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        deleted_key.as_str(),
    )
    .await?;
    if !deleted_query.items.is_empty() {
        return Err(format!(
            "deleted system-config key still visible: items={:?}",
            deleted_query.items
        ));
    }

    println!(
        "[klog-cluster-dv] system-config kv semantics ok: leader={}, source_gateway={}, target_node={}, prefix={}",
        leader_id, source.name, target.name, key_prefix
    );
    Ok(())
}

async fn run_local_gateway_system_config_kv() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_system_config_kv_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

fn collect_used_ports(nodes: &[LocalNodeDef], ingress_port: u16) -> BTreeSet<u16> {
    let mut used_ports = BTreeSet::from([ingress_port]);
    for node in nodes {
        used_ports.insert(node.ports.raft);
        used_ports.insert(node.ports.inter);
        used_ports.insert(node.ports.admin);
        used_ports.insert(node.ports.rpc);
        used_ports.insert(node.ports.rtcp);
        used_ports.insert(node.ports.zone_http);
        used_ports.insert(node.ports.control);
    }
    used_ports
}

async fn run_local_gateway_system_config_service_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-system-config-service-dv";
    let setup =
        prepare_local_gateway_setup(harness, SYSTEM_CONFIG_SERVICE_MODE, route_prefix, 3).await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let seed = nodes
        .first()
        .ok_or_else(|| "missing seed node".to_string())?
        .clone();
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };

    for node in &nodes {
        let config = write_klog_config(harness, node, &voter_config)?;
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        wait_tcp("127.0.0.1", node.ports.rpc, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(50),
    )
    .await?;
    let leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let leader = nodes
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| format!("leader node {} not found", leader_id))?;
    let klog_endpoint = format!(
        "http://127.0.0.1:{}{}",
        leader.ports.rpc, KLOG_JSON_RPC_SERVICE_PATH
    );

    let repo_root = repo_root()?;
    let buckyos_root = get_buckyos_root();
    let system_config_bin = resolve_system_config_bin(&repo_root, &buckyos_root)?;
    let mut used_ports = collect_used_ports(&nodes, ingress_port);
    let system_config_port = pick_local_port(&mut used_ports)?;
    spawn_system_config(
        harness,
        &system_config_bin,
        system_config_port,
        klog_endpoint.as_str(),
    )?;
    wait_tcp("127.0.0.1", system_config_port, Duration::from_secs(15)).await?;

    let endpoint = format!(
        "http://127.0.0.1:{}{}",
        system_config_port, SYSTEM_CONFIG_RPC_SERVICE_PATH
    );
    let user_token = system_config_jwt(TEST_DEVICE_NAME, "root", "scheduler")?;
    let scheduler_token = system_config_jwt(TEST_DEVICE_NAME, "alice", "scheduler")?;
    let suffix = unique_suffix("syscfg-service");
    let base = format!("users/alice/klog_service_dv/{}", suffix);
    let profile_key = format!("{}/profile", base);
    let notes_key = format!("{}/notes", base);
    let tx_key1 = format!("{}/tx/key1", base);
    let tx_key2 = format!("{}/tx/key2", base);
    let stale_key = format!("{}/tx/stale", base);

    let profile_v1 = r#"{"name":"v1","flags":{"enabled":false}}"#;
    call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_create",
        json!({"key": profile_key, "value": profile_v1}),
    )
    .await?;
    let created = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_get",
        json!({"key": profile_key}),
    )
    .await?;
    require_system_config_value(&created, profile_v1, 1)?;
    expect_system_config_rpc_error(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_create",
        json!({"key": profile_key, "value": profile_v1}),
    )
    .await?;

    let profile_v2 = r#"{"name":"v2","flags":{"enabled":false}}"#;
    call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_set",
        json!({"key": profile_key, "value": profile_v2}),
    )
    .await?;
    let set = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_get",
        json!({"key": profile_key}),
    )
    .await?;
    require_system_config_value(&set, profile_v2, 1)?;

    call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_set_by_json_path",
        json!({"key": profile_key, "json_path": "/flags/enabled", "value": "true"}),
    )
    .await?;
    let path_updated = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_get",
        json!({"key": profile_key}),
    )
    .await?;
    let path_updated_version = path_updated
        .get("version")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("missing version after json path update: {}", path_updated))?;
    let path_updated_value: Value = serde_json::from_str(
        path_updated
            .get("value")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("missing value after json path update: {}", path_updated))?,
    )
    .map_err(|err| format!("failed to parse profile json value: {}", err))?;
    if path_updated_value
        .pointer("/flags/enabled")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return Err(format!(
            "json path update was not visible: {}",
            path_updated_value
        ));
    }

    call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_set",
        json!({"key": profile_key, "value": profile_v2}),
    )
    .await?;
    let mut stale_actions = serde_json::Map::new();
    stale_actions.insert(
        stale_key.clone(),
        json!({
            "action": "create",
            "value": "should-not-exist"
        }),
    );
    expect_system_config_rpc_error(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_exec_tx",
        json!({
            "main_key": format!("{}:{}", profile_key, path_updated_version),
            "actions": stale_actions
        }),
    )
    .await?;
    let stale = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_get",
        json!({"key": stale_key}),
    )
    .await?;
    if !stale.is_null() {
        return Err(format!(
            "stale guarded exec_tx left partial state: {}",
            stale
        ));
    }

    call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_create",
        json!({"key": notes_key, "value": "hello"}),
    )
    .await?;
    call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_append",
        json!({"key": notes_key, "append_value": " world"}),
    )
    .await?;
    let notes = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_get",
        json!({"key": notes_key}),
    )
    .await?;
    require_system_config_value(&notes, "hello world", 1)?;

    let mut tx_actions = serde_json::Map::new();
    tx_actions.insert(
        tx_key1.clone(),
        json!({
            "action": "create",
            "value": "tx-value-1"
        }),
    );
    tx_actions.insert(
        tx_key2.clone(),
        json!({
            "action": "create",
            "value": "tx-value-2"
        }),
    );
    call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_exec_tx",
        json!({"actions": tx_actions}),
    )
    .await?;
    let tx1 = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_get",
        json!({"key": tx_key1}),
    )
    .await?;
    require_system_config_value(&tx1, "tx-value-1", 1)?;
    let tx2 = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_get",
        json!({"key": tx_key2}),
    )
    .await?;
    require_system_config_value(&tx2, "tx-value-2", 1)?;

    let listed = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_list",
        json!({"key": base}),
    )
    .await?;
    let listed = listed
        .as_array()
        .ok_or_else(|| format!("system_config list result is not array: {}", listed))?;
    for expected_child in ["profile", "notes", "tx"] {
        if !listed
            .iter()
            .any(|value| value.as_str() == Some(expected_child))
        {
            return Err(format!(
                "system_config list missing child {}: {:?}",
                expected_child, listed
            ));
        }
    }

    call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_delete",
        json!({"key": notes_key}),
    )
    .await?;
    let deleted = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        user_token.as_str(),
        "sys_config_get",
        json!({"key": notes_key}),
    )
    .await?;
    if !deleted.is_null() {
        return Err(format!("deleted key is still visible: {}", deleted));
    }

    let scheduler_dump = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        scheduler_token.as_str(),
        "dump_configs_for_scheduler",
        json!({}),
    )
    .await?;
    if scheduler_dump.get(profile_key.as_str()).is_none()
        || scheduler_dump.get(tx_key1.as_str()).is_none()
    {
        return Err(format!(
            "scheduler dump missing klog-backed system_config keys: {}",
            scheduler_dump
        ));
    }

    println!(
        "[klog-cluster-dv] system_config service klog backend ok: leader={}, endpoint={}, prefix={}",
        leader_id, endpoint, base
    );
    Ok(())
}

async fn run_local_gateway_system_config_service() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_system_config_service_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_system_config_leader_failover_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-system-config-leader-failover-dv";
    let setup =
        prepare_local_gateway_setup(harness, SYSTEM_CONFIG_LEADER_FAILOVER_MODE, route_prefix, 3)
            .await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let seed = nodes
        .first()
        .cloned()
        .ok_or_else(|| "missing system_config failover seed node".to_string())?;
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };
    let mut configs = BTreeMap::new();
    for node in &nodes {
        let config = write_klog_config(harness, node, &voter_config)?;
        configs.insert(node.id, config.clone());
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        wait_tcp("127.0.0.1", node.ports.rpc, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let old_leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(50),
    )
    .await?;
    let old_leader = nodes
        .iter()
        .find(|node| node.id == old_leader_id)
        .cloned()
        .ok_or_else(|| format!("system_config failover leader {} not found", old_leader_id))?;
    let endpoint_node = nodes
        .iter()
        .find(|node| node.id != old_leader_id)
        .cloned()
        .ok_or_else(|| "missing non-leader klog RPC endpoint node".to_string())?;
    let survivors = nodes
        .iter()
        .filter(|node| node.id != old_leader_id)
        .cloned()
        .collect::<Vec<_>>();

    let klog_endpoint = format!(
        "http://127.0.0.1:{}{}",
        endpoint_node.ports.rpc, KLOG_JSON_RPC_SERVICE_PATH
    );
    let repo_root = repo_root()?;
    let buckyos_root = get_buckyos_root();
    let system_config_bin = resolve_system_config_bin(&repo_root, &buckyos_root)?;
    let mut used_ports = collect_used_ports(&nodes, ingress_port);
    let system_config_port = pick_local_port(&mut used_ports)?;
    spawn_system_config(
        harness,
        &system_config_bin,
        system_config_port,
        klog_endpoint.as_str(),
    )?;
    wait_tcp("127.0.0.1", system_config_port, Duration::from_secs(15)).await?;

    let endpoint = format!(
        "http://127.0.0.1:{}{}",
        system_config_port, SYSTEM_CONFIG_RPC_SERVICE_PATH
    );
    let token = system_config_jwt(TEST_DEVICE_NAME, "root", "scheduler")?;
    let suffix = unique_suffix("syscfg-leader-failover");
    let base = format!("users/alice/klog_leader_failover_dv/{}", suffix);
    let prefix = format!("{}/", base);
    let profile_key = format!("{}profile", prefix);
    let tx_key = format!("{}tx/key", prefix);
    let profile_v1 = "profile-before-failover-v1";
    let profile_v2 = "profile-before-failover-v2";
    let profile_during_failover = "profile-during-failover";
    let profile_v3 = "profile-after-failover-v3";
    let profile_v4 = "profile-after-rejoin-v4";

    call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_create",
        json!({"key": profile_key.as_str(), "value": profile_v1}),
    )
    .await?;
    let created = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": profile_key.as_str()}),
    )
    .await?;
    let (_, r1) = system_config_value_and_version(&created)?;
    require_system_config_value(&created, profile_v1, r1)?;

    call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_set",
        json!({"key": profile_key.as_str(), "value": profile_v2}),
    )
    .await?;
    let before_failover = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": profile_key.as_str()}),
    )
    .await?;
    let (_, r2) = system_config_value_and_version(&before_failover)?;
    if r2 <= r1 {
        return Err(format!(
            "system_config pre-failover set revision did not advance: r1={}, r2={}",
            r1, r2
        ));
    }
    require_system_config_value(&before_failover, profile_v2, r2)?;

    harness.stop(format!("klog-{}", old_leader.name).as_str())?;
    let failover_err = expect_system_config_rpc_error(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_set",
        json!({"key": profile_key.as_str(), "value": profile_during_failover}),
    )
    .await?;
    require_system_config_klog_failover_error(failover_err.as_str())?;

    let new_leader_id = wait_consistent_leader(
        &client,
        &survivors,
        ingress_port,
        route_prefix,
        Some(old_leader_id),
        Duration::from_secs(90),
    )
    .await?;
    wait_membership(
        &client,
        &survivors,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let after_failed_write = wait_system_config_rpc_success(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": profile_key.as_str()}),
        Duration::from_secs(40),
    )
    .await?;
    require_system_config_value(&after_failed_write, profile_v2, r2)?;

    wait_system_config_rpc_success(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_set",
        json!({"key": profile_key.as_str(), "value": profile_v3}),
        Duration::from_secs(40),
    )
    .await?;
    let after_retry = wait_system_config_rpc_success(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": profile_key.as_str()}),
        Duration::from_secs(40),
    )
    .await?;
    let (_, r3) = system_config_value_and_version(&after_retry)?;
    if r3 <= r2 {
        return Err(format!(
            "system_config post-failover retry revision did not advance: r2={}, r3={}",
            r2, r3
        ));
    }
    require_system_config_value(&after_retry, profile_v3, r3)?;

    let mut tx_actions = serde_json::Map::new();
    tx_actions.insert(
        profile_key.clone(),
        json!({
            "action": "update",
            "value": profile_v4
        }),
    );
    tx_actions.insert(
        tx_key.clone(),
        json!({
            "action": "create",
            "value": "tx-after-failover"
        }),
    );
    wait_system_config_rpc_success(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_exec_tx",
        json!({
            "main_key": format!("{}:{}", profile_key, r3),
            "actions": tx_actions
        }),
        Duration::from_secs(40),
    )
    .await?;
    let after_tx = wait_system_config_rpc_success(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": profile_key.as_str()}),
        Duration::from_secs(40),
    )
    .await?;
    let (_, r4) = system_config_value_and_version(&after_tx)?;
    if r4 <= r3 {
        return Err(format!(
            "system_config post-failover tx revision did not advance: r3={}, r4={}",
            r3, r4
        ));
    }
    require_system_config_value(&after_tx, profile_v4, r4)?;
    let tx_after_failover = wait_system_config_rpc_success(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": tx_key.as_str()}),
        Duration::from_secs(40),
    )
    .await?;
    require_system_config_value(&tx_after_failover, "tx-after-failover", r4)?;

    let old_leader_config = configs
        .get(&old_leader.id)
        .ok_or_else(|| format!("missing old leader config {}", old_leader.id))?;
    spawn_klog(harness, &klog_daemon_bin, old_leader_config, &old_leader)?;
    wait_tcp("127.0.0.1", old_leader.ports.admin, Duration::from_secs(12)).await?;
    wait_tcp("127.0.0.1", old_leader.ports.rpc, Duration::from_secs(12)).await?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(120),
    )
    .await?;
    let final_leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(80),
    )
    .await?;
    let after_rejoin = wait_system_config_rpc_success(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": profile_key.as_str()}),
        Duration::from_secs(40),
    )
    .await?;
    require_system_config_value(&after_rejoin, profile_v4, r4)?;

    for node in &nodes {
        let response = query_meta_prefix_via_cluster_inter_route(
            &client,
            gateway_addr(node, ingress_port).as_str(),
            route_prefix,
            node.name.as_str(),
            prefix.as_str(),
            8,
        )
        .await?;
        require_meta_selected_values(
            &response,
            &[
                (profile_key.as_str(), profile_v4, r1, r4, 4),
                (tx_key.as_str(), "tx-after-failover", r4, r4, 1),
            ],
        )?;
    }

    println!(
        "[klog-cluster-dv] system_config leader failover ok: old_leader={}, new_leader={}, final_leader={}, endpoint_node={}, endpoint={}, failover_error_len={}, revisions=[{},{},{},{}], prefix={}",
        old_leader_id,
        new_leader_id,
        final_leader_id,
        endpoint_node.id,
        endpoint,
        failover_err.len(),
        r1,
        r2,
        r3,
        r4,
        prefix
    );
    Ok(())
}

async fn run_local_gateway_system_config_leader_failover() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_system_config_leader_failover_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_abnormal_inner(harness: &mut LocalHarness) -> Result<(), String> {
    if LocalGatewayRouteMode::from_env()? != LocalGatewayRouteMode::TargetGateway {
        return Err(format!(
            "{} requires {}=target-gateway",
            GATEWAY_ABNORMAL_MODE, KLOG_CLUSTER_DV_ROUTE_MODE_ENV
        ));
    }

    let route_prefix = "/.cluster/klog-it-gateway-abnormal-dv";
    let setup =
        prepare_local_gateway_setup(harness, GATEWAY_ABNORMAL_MODE, route_prefix, 3).await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let seed = nodes
        .first()
        .cloned()
        .ok_or_else(|| "missing gateway abnormal seed node".to_string())?;
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };
    for node in &nodes {
        let config = write_klog_config(harness, node, &voter_config)?;
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        wait_tcp("127.0.0.1", node.ports.rpc, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(50),
    )
    .await?;
    let source = nodes
        .iter()
        .find(|node| node.id == leader_id)
        .cloned()
        .ok_or_else(|| format!("gateway abnormal leader {} not found", leader_id))?;
    let mut non_leaders = nodes
        .iter()
        .filter(|node| node.id != leader_id)
        .cloned()
        .collect::<Vec<_>>();
    if non_leaders.len() < 2 {
        return Err("gateway abnormal requires two non-leader nodes".to_string());
    }
    let stopped_victim = non_leaders.remove(0);
    let healthy_target = non_leaders.remove(0);
    let source_gateway_addr = gateway_addr(&source, ingress_port);
    let healthy_gateway_addr = gateway_addr(&healthy_target, ingress_port);
    let repo_root = repo_root()?;
    let buckyos_root = get_buckyos_root();
    let cyfs_gateway_bin = resolve_cyfs_gateway_bin(&repo_root, &buckyos_root)?;
    let mut used_ports = collect_used_ports(&nodes, ingress_port);
    let stale_source = LocalNodeDef {
        id: 99,
        name: "client".to_string(),
        device_id: "did:dv:client".to_string(),
        gateway_host: "127.0.0.4".to_string(),
        ports: LocalNodePorts {
            raft: pick_local_port(&mut used_ports)?,
            inter: pick_local_port(&mut used_ports)?,
            admin: pick_local_port(&mut used_ports)?,
            rpc: pick_local_port(&mut used_ports)?,
            rtcp: pick_local_port(&mut used_ports)?,
            zone_http: pick_local_port(&mut used_ports)?,
            control: pick_local_port(&mut used_ports)?,
        },
    };
    if !reserve_port(stale_source.gateway_host.as_str(), ingress_port) {
        return Err(format!(
            "gateway abnormal stale source ingress is not free: {}:{}",
            stale_source.gateway_host, ingress_port
        ));
    }
    let stale_source_gateway_addr = gateway_addr(&stale_source, ingress_port);

    let suffix = unique_suffix("gateway-abnormal");
    let base = format!("gateway_abnormal_dv/{}/", suffix);
    let baseline_key = format!("{}baseline", base);
    let stopped_key = format!("{}target-gateway-stopped", base);
    let stale_key = format!("{}stale-route", base);
    let stale_recovery_key = format!("{}stale-route-recovered", base);

    let baseline = put_meta_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        healthy_target.name.as_str(),
        baseline_key.as_str(),
        "baseline-v1",
        None,
    )
    .await?;
    let baseline_query = query_meta_via_cluster_inter_route(
        &client,
        healthy_gateway_addr.as_str(),
        route_prefix,
        healthy_target.name.as_str(),
        baseline_key.as_str(),
    )
    .await?;
    require_meta_value(
        &baseline_query,
        baseline_key.as_str(),
        "baseline-v1",
        baseline.revision,
    )?;

    harness.stop(format!("gateway-{}", stopped_victim.name).as_str())?;
    let stopped_err = expect_meta_put_error_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        stopped_victim.name.as_str(),
        stopped_key.as_str(),
        "must-not-write-while-target-gateway-stopped",
    )
    .await?;
    require_gateway_diagnostic_error(stopped_err.as_str(), "target gateway stopped data route")?;
    let stopped_query = query_meta_via_cluster_inter_route(
        &client,
        healthy_gateway_addr.as_str(),
        route_prefix,
        healthy_target.name.as_str(),
        stopped_key.as_str(),
    )
    .await?;
    require_meta_key_absent(&stopped_query, stopped_key.as_str())?;

    let stale_gateway_options = GatewayRuntimeOptions {
        all_nodes: &nodes,
        ingress_port,
        route_prefix,
        route_mode: LocalGatewayRouteMode::TargetGateway,
    };
    let stale_gateway_config = write_gateway_runtime(
        harness,
        &repo_root,
        &buckyos_root,
        &stale_source,
        &stale_gateway_options,
    )?;
    patch_gateway_direct_route(
        harness,
        &stale_source,
        healthy_target.name.as_str(),
        "tcp:///127.0.0.250",
    )?;
    spawn_gateway(
        harness,
        &cyfs_gateway_bin,
        stale_gateway_config.as_path(),
        &stale_source,
    )?;
    wait_tcp(
        stale_source.gateway_host.as_str(),
        ingress_port,
        Duration::from_secs(8),
    )
    .await?;

    let stale_err = expect_meta_put_error_via_cluster_inter_route(
        &client,
        stale_source_gateway_addr.as_str(),
        route_prefix,
        healthy_target.name.as_str(),
        stale_key.as_str(),
        "must-not-write-through-stale-route",
    )
    .await?;
    require_gateway_diagnostic_error(stale_err.as_str(), "stale route data route")?;
    let admin_err = match fetch_cluster_state_via_admin_route(
        &client,
        stale_source_gateway_addr.as_str(),
        route_prefix,
        healthy_target.name.as_str(),
    )
    .await
    {
        Ok(value) => Err(format!(
            "stale gateway admin route unexpectedly succeeded: {}",
            value
        )),
        Err(err) => Ok(err),
    }?;
    require_gateway_diagnostic_error(admin_err.as_str(), "stale route admin route")?;

    let stale_query = query_meta_via_cluster_inter_route(
        &client,
        healthy_gateway_addr.as_str(),
        route_prefix,
        healthy_target.name.as_str(),
        stale_key.as_str(),
    )
    .await?;
    require_meta_key_absent(&stale_query, stale_key.as_str())?;
    wait_consistent_leader(
        &client,
        &[source.clone(), healthy_target.clone()],
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(60),
    )
    .await?;
    let stale_recovery = put_meta_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        healthy_target.name.as_str(),
        stale_recovery_key.as_str(),
        "stale-route-recovered-v1",
        None,
    )
    .await?;
    let current = query_meta_prefix_via_cluster_inter_route(
        &client,
        healthy_gateway_addr.as_str(),
        route_prefix,
        healthy_target.name.as_str(),
        base.as_str(),
        16,
    )
    .await?;
    require_meta_key_absent(&current, stopped_key.as_str())?;
    require_meta_key_absent(&current, stale_key.as_str())?;
    require_meta_selected_values(
        &current,
        &[
            (
                baseline_key.as_str(),
                "baseline-v1",
                baseline.revision,
                baseline.revision,
                1,
            ),
            (
                stale_recovery_key.as_str(),
                "stale-route-recovered-v1",
                stale_recovery.revision,
                stale_recovery.revision,
                1,
            ),
        ],
    )?;

    println!(
        "[klog-cluster-dv] gateway abnormal ok: leader={}, source={}, stopped_victim={}, healthy_target={}, stale_source={}, stopped_error_len={}, stale_error_len={}, admin_error_len={}, prefix={}",
        leader_id,
        source.name,
        stopped_victim.name,
        healthy_target.name,
        stale_source.name,
        stopped_err.len(),
        stale_err.len(),
        admin_err.len(),
        base
    );
    Ok(())
}

async fn run_local_gateway_abnormal() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_abnormal_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_system_config_stale_config_rejoin_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-system-config-stale-config-rejoin-dv";
    let setup = prepare_local_gateway_setup(
        harness,
        SYSTEM_CONFIG_STALE_CONFIG_REJOIN_MODE,
        route_prefix,
        3,
    )
    .await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let seed = nodes
        .first()
        .cloned()
        .ok_or_else(|| "missing system_config stale-config seed node".to_string())?;
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };
    let mut configs = BTreeMap::new();
    for node in &nodes {
        let config = write_klog_config(harness, node, &voter_config)?;
        configs.insert(node.id, config.clone());
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        wait_tcp("127.0.0.1", node.ports.rpc, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(60),
    )
    .await?;
    let initial_leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(50),
    )
    .await?;
    let removed_node = nodes
        .iter()
        .find(|node| node.id != initial_leader_id)
        .cloned()
        .ok_or_else(|| {
            format!("failed to pick stale-config removed node, leader={initial_leader_id}")
        })?;
    let active_nodes = nodes
        .iter()
        .filter(|node| node.id != removed_node.id)
        .cloned()
        .collect::<Vec<_>>();
    if active_nodes.len() != 2 {
        return Err(format!(
            "expected two active nodes after removing {}, got {}",
            removed_node.id,
            active_nodes.len()
        ));
    }
    let active_voters = active_nodes.iter().map(|node| node.id).collect::<Vec<_>>();
    let active_system_node = active_nodes
        .first()
        .cloned()
        .ok_or_else(|| "missing active system_config node".to_string())?;

    let repo_root = repo_root()?;
    let buckyos_root = get_buckyos_root();
    let system_config_bin = resolve_system_config_bin(&repo_root, &buckyos_root)?;
    let mut used_ports = collect_used_ports(&nodes, ingress_port);
    let active_system_config_port = pick_local_port(&mut used_ports)?;
    let stale_system_config_port = pick_local_port(&mut used_ports)?;
    let active_root = harness.root.join("system-config-active-root");
    let stale_root = harness.root.join("system-config-stale-root");
    let active_klog_endpoint = format!(
        "http://127.0.0.1:{}{}",
        active_system_node.ports.rpc, KLOG_JSON_RPC_SERVICE_PATH
    );
    let stale_klog_endpoint = format!(
        "http://127.0.0.1:{}{}",
        removed_node.ports.rpc, KLOG_JSON_RPC_SERVICE_PATH
    );
    let active_token = system_config_jwt(active_system_node.name.as_str(), "root", "scheduler")?;
    let stale_token = system_config_jwt(removed_node.name.as_str(), "root", "scheduler")?;

    spawn_system_config_with_options(
        harness,
        "system-config-active-klog",
        &system_config_bin,
        active_root.as_path(),
        active_system_config_port,
        Some(active_klog_endpoint.as_str()),
        active_system_node.name.as_str(),
        false,
    )?;
    wait_tcp(
        "127.0.0.1",
        active_system_config_port,
        Duration::from_secs(15),
    )
    .await?;
    let active_endpoint = format!(
        "http://127.0.0.1:{}{}",
        active_system_config_port, SYSTEM_CONFIG_RPC_SERVICE_PATH
    );
    let suffix = unique_suffix("syscfg-stale-config");
    let base = format!("users/alice/klog_stale_config_dv/{}", suffix);
    let before_key = format!("{}/before_shrink", base);
    let after_shrink_key = format!("{}/after_shrink", base);
    let stale_key = format!("{}/stale_write", base);
    let active_after_stale_key = format!("{}/active_after_stale", base);

    call_system_config_rpc(
        &client,
        active_endpoint.as_str(),
        active_token.as_str(),
        "sys_config_create",
        json!({"key": before_key.as_str(), "value": "before-shrink"}),
    )
    .await?;
    let before_value = call_system_config_rpc(
        &client,
        active_endpoint.as_str(),
        active_token.as_str(),
        "sys_config_get",
        json!({"key": before_key.as_str()}),
    )
    .await?;
    let (_, before_revision) = system_config_value_and_version(&before_value)?;
    require_system_config_value(&before_value, "before-shrink", before_revision)?;

    harness.stop(format!("klog-{}", removed_node.name).as_str())?;
    let shrink_leader_id = change_voters_via_current_leader(
        &client,
        &active_nodes,
        ingress_port,
        route_prefix,
        active_voters.as_slice(),
        false,
    )
    .await?;
    wait_membership(
        &client,
        &active_nodes,
        ingress_port,
        route_prefix,
        active_voters.as_slice(),
        &[],
        Duration::from_secs(80),
    )
    .await?;
    call_system_config_rpc(
        &client,
        active_endpoint.as_str(),
        active_token.as_str(),
        "sys_config_create",
        json!({"key": after_shrink_key.as_str(), "value": "after-shrink"}),
    )
    .await?;

    let removed_config = configs
        .get(&removed_node.id)
        .ok_or_else(|| format!("missing stale removed node config {}", removed_node.id))?;
    spawn_klog(harness, &klog_daemon_bin, removed_config, &removed_node)?;
    wait_tcp(
        "127.0.0.1",
        removed_node.ports.admin,
        Duration::from_secs(12),
    )
    .await?;
    wait_tcp("127.0.0.1", removed_node.ports.rpc, Duration::from_secs(12)).await?;
    tokio::time::sleep(Duration::from_secs(3)).await;
    wait_membership(
        &client,
        &active_nodes,
        ingress_port,
        route_prefix,
        active_voters.as_slice(),
        &[],
        Duration::from_secs(30),
    )
    .await?;

    spawn_system_config_with_options(
        harness,
        "system-config-stale-klog",
        &system_config_bin,
        stale_root.as_path(),
        stale_system_config_port,
        Some(stale_klog_endpoint.as_str()),
        removed_node.name.as_str(),
        false,
    )?;
    wait_tcp(
        "127.0.0.1",
        stale_system_config_port,
        Duration::from_secs(15),
    )
    .await?;
    let stale_endpoint = format!(
        "http://127.0.0.1:{}{}",
        stale_system_config_port, SYSTEM_CONFIG_RPC_SERVICE_PATH
    );
    let stale_write_err = expect_system_config_rpc_error(
        &client,
        stale_endpoint.as_str(),
        stale_token.as_str(),
        "sys_config_create",
        json!({"key": stale_key.as_str(), "value": "must-not-land-from-stale-config"}),
    )
    .await?;
    require_system_config_klog_failover_error(stale_write_err.as_str())?;
    tokio::time::sleep(Duration::from_secs(2)).await;
    wait_membership(
        &client,
        &active_nodes,
        ingress_port,
        route_prefix,
        active_voters.as_slice(),
        &[],
        Duration::from_secs(30),
    )
    .await?;
    let stale_from_active = call_system_config_rpc(
        &client,
        active_endpoint.as_str(),
        active_token.as_str(),
        "sys_config_get",
        json!({"key": stale_key.as_str()}),
    )
    .await?;
    require_system_config_null(&stale_from_active, stale_key.as_str())?;

    call_system_config_rpc(
        &client,
        active_endpoint.as_str(),
        active_token.as_str(),
        "sys_config_create",
        json!({"key": active_after_stale_key.as_str(), "value": "active-after-stale"}),
    )
    .await?;
    let active_after_stale = call_system_config_rpc(
        &client,
        active_endpoint.as_str(),
        active_token.as_str(),
        "sys_config_get",
        json!({"key": active_after_stale_key.as_str()}),
    )
    .await?;
    let (_, active_after_stale_revision) = system_config_value_and_version(&active_after_stale)?;
    require_system_config_value(
        &active_after_stale,
        "active-after-stale",
        active_after_stale_revision,
    )?;

    let active_prefix = query_meta_prefix_via_cluster_inter_route(
        &client,
        gateway_addr(&active_system_node, ingress_port).as_str(),
        route_prefix,
        active_system_node.name.as_str(),
        format!("{}/", base).as_str(),
        16,
    )
    .await?;
    require_meta_key_absent(&active_prefix, stale_key.as_str())?;
    require_meta_keys(
        &active_prefix,
        &[
            before_key.as_str(),
            after_shrink_key.as_str(),
            active_after_stale_key.as_str(),
        ],
    )?;

    println!(
        "[klog-cluster-dv] system_config stale config rejoin ok: initial_leader={}, removed_node={}, shrink_leader={}, active_voters={:?}, active_endpoint={}, stale_endpoint={}, stale_error_len={}, prefix={}",
        initial_leader_id,
        removed_node.id,
        shrink_leader_id,
        active_voters,
        active_endpoint,
        stale_endpoint,
        stale_write_err.len(),
        base
    );
    Ok(())
}

async fn run_local_gateway_system_config_stale_config_rejoin() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_system_config_stale_config_rejoin_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_system_config_mvcc_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-system-config-mvcc-dv";
    let setup =
        prepare_local_gateway_setup(harness, SYSTEM_CONFIG_MVCC_MODE, route_prefix, 3).await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let seed = nodes
        .first()
        .ok_or_else(|| "missing seed node".to_string())?
        .clone();
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };

    for node in &nodes {
        let config = write_klog_config(harness, node, &voter_config)?;
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        wait_tcp("127.0.0.1", node.ports.rpc, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(50),
    )
    .await?;
    let leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let leader = nodes
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| format!("leader node {} not found", leader_id))?;
    let source = nodes
        .first()
        .ok_or_else(|| "missing source gateway node".to_string())?;
    let target = nodes
        .iter()
        .find(|node| node.name != source.name)
        .ok_or_else(|| "missing target gateway node".to_string())?;
    let source_gateway_addr = gateway_addr(source, ingress_port);

    let klog_endpoint = format!(
        "http://127.0.0.1:{}{}",
        leader.ports.rpc, KLOG_JSON_RPC_SERVICE_PATH
    );
    let repo_root = repo_root()?;
    let buckyos_root = get_buckyos_root();
    let system_config_bin = resolve_system_config_bin(&repo_root, &buckyos_root)?;
    let mut used_ports = collect_used_ports(&nodes, ingress_port);
    let system_config_port = pick_local_port(&mut used_ports)?;
    spawn_system_config(
        harness,
        &system_config_bin,
        system_config_port,
        klog_endpoint.as_str(),
    )?;
    wait_tcp("127.0.0.1", system_config_port, Duration::from_secs(15)).await?;

    let endpoint = format!(
        "http://127.0.0.1:{}{}",
        system_config_port, SYSTEM_CONFIG_RPC_SERVICE_PATH
    );
    let token = system_config_jwt(TEST_DEVICE_NAME, "root", "scheduler")?;
    let suffix = unique_suffix("syscfg-mvcc");
    let base = format!("users/alice/klog_mvcc_dv/{}", suffix);
    let prefix = format!("{}/", base);
    let profile_key = format!("{}profile", prefix);
    let tx_key1 = format!("{}tx/key1", prefix);
    let tx_key2 = format!("{}tx/key2", prefix);
    let stale_key = format!("{}tx/stale", prefix);

    let profile_v1 = r#"{"name":"v1","flags":{"enabled":false}}"#;
    call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_create",
        json!({"key": profile_key.as_str(), "value": profile_v1}),
    )
    .await?;
    let created = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": profile_key.as_str()}),
    )
    .await?;
    let (_, r1) = system_config_value_and_version(&created)?;
    require_system_config_value(&created, profile_v1, r1)?;

    let profile_v2 = r#"{"name":"v2","flags":{"enabled":false}}"#;
    call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_set",
        json!({"key": profile_key.as_str(), "value": profile_v2}),
    )
    .await?;
    let set = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": profile_key.as_str()}),
    )
    .await?;
    let (_, r2) = system_config_value_and_version(&set)?;
    if r2 <= r1 {
        return Err(format!(
            "system_config set revision did not advance: r1={r1}, r2={r2}"
        ));
    }
    require_system_config_value(&set, profile_v2, r2)?;

    call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_set_by_json_path",
        json!({"key": profile_key.as_str(), "json_path": "/flags/enabled", "value": "true"}),
    )
    .await?;
    let path_updated = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": profile_key.as_str()}),
    )
    .await?;
    let (profile_v3, r3) = system_config_value_and_version(&path_updated)?;
    if r3 <= r2 {
        return Err(format!(
            "system_config json-path revision did not advance: r2={r2}, r3={r3}"
        ));
    }
    let profile_v3_json: Value = serde_json::from_str(profile_v3.as_str())
        .map_err(|err| format!("failed to decode json-path profile value: {}", err))?;
    if profile_v3_json
        .pointer("/flags/enabled")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return Err(format!(
            "system_config json-path update not visible: {}",
            profile_v3_json
        ));
    }

    let mut stale_actions = serde_json::Map::new();
    stale_actions.insert(
        profile_key.clone(),
        json!({
            "action": "update",
            "value": "stale-profile-value"
        }),
    );
    stale_actions.insert(
        stale_key.clone(),
        json!({
            "action": "create",
            "value": "should-not-exist"
        }),
    );
    let stale_err = expect_system_config_rpc_error(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_exec_tx",
        json!({
            "main_key": format!("{}:{}", profile_key, r2),
            "actions": stale_actions
        }),
    )
    .await?;
    if !stale_err.contains("revision mismatch") {
        return Err(format!(
            "stale system_config exec_tx returned unexpected error: {}",
            stale_err
        ));
    }
    let stale = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": stale_key.as_str()}),
    )
    .await?;
    if !stale.is_null() {
        return Err(format!(
            "stale system_config exec_tx left partial create: {}",
            stale
        ));
    }
    let after_stale = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": profile_key.as_str()}),
    )
    .await?;
    let (after_stale_value, after_stale_revision) = system_config_value_and_version(&after_stale)?;
    if after_stale_value != profile_v3 || after_stale_revision != r3 {
        return Err(format!(
            "stale system_config exec_tx changed guarded key: before=({}, {}), after=({}, {})",
            profile_v3, r3, after_stale_value, after_stale_revision
        ));
    }

    let profile_v4 = "profile-v4";
    let mut good_actions = serde_json::Map::new();
    good_actions.insert(
        profile_key.clone(),
        json!({
            "action": "update",
            "value": profile_v4
        }),
    );
    good_actions.insert(
        tx_key1.clone(),
        json!({
            "action": "create",
            "value": "tx-value-1"
        }),
    );
    good_actions.insert(
        tx_key2.clone(),
        json!({
            "action": "create",
            "value": "tx-value-2"
        }),
    );
    call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_exec_tx",
        json!({
            "main_key": format!("{}:{}", profile_key, r3),
            "actions": good_actions
        }),
    )
    .await?;
    let profile_after_tx = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": profile_key.as_str()}),
    )
    .await?;
    let (_, r4) = system_config_value_and_version(&profile_after_tx)?;
    if r4 <= r3 {
        return Err(format!(
            "system_config tx revision did not advance: r3={r3}, r4={r4}"
        ));
    }
    require_system_config_value(&profile_after_tx, profile_v4, r4)?;
    let tx1 = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": tx_key1.as_str()}),
    )
    .await?;
    let (_, tx1_revision) = system_config_value_and_version(&tx1)?;
    let tx2 = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": tx_key2.as_str()}),
    )
    .await?;
    let (_, tx2_revision) = system_config_value_and_version(&tx2)?;
    if tx1_revision != r4 || tx2_revision != r4 {
        return Err(format!(
            "system_config exec_tx keys did not share one klog revision: profile={}, tx1={}, tx2={}",
            r4, tx1_revision, tx2_revision
        ));
    }

    call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_delete",
        json!({"key": tx_key1.as_str()}),
    )
    .await?;
    let deleted_tx1 = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": tx_key1.as_str()}),
    )
    .await?;
    if !deleted_tx1.is_null() {
        return Err(format!(
            "deleted system_config key still visible: {}",
            deleted_tx1
        ));
    }
    let delete_changes = query_meta_changes_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        prefix.as_str(),
        r4 + 1,
        8,
        None,
    )
    .await?;
    require_meta_changes(
        &delete_changes,
        &[(
            delete_changes
                .items
                .first()
                .ok_or_else(|| "missing delete change".to_string())?
                .mod_revision,
            &tx_key1,
            "tx-value-1",
            true,
            r4,
            0,
        )],
    )?;
    let r5 = delete_changes.items[0].mod_revision;

    call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_create",
        json!({"key": tx_key1.as_str(), "value": "tx-value-1-recreated"}),
    )
    .await?;
    let recreated_tx1 = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": tx_key1.as_str()}),
    )
    .await?;
    let (_, r6) = system_config_value_and_version(&recreated_tx1)?;
    if r6 <= r5 {
        return Err(format!(
            "system_config recreate revision did not advance: r5={r5}, r6={r6}"
        ));
    }
    require_system_config_value(&recreated_tx1, "tx-value-1-recreated", r6)?;

    let rev1 = query_meta_prefix_page_at_revision_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        prefix.as_str(),
        16,
        None,
        Some(r1),
    )
    .await?;
    require_meta_values(&rev1, &[(&profile_key, profile_v1, r1, r1, 1)])?;

    let rev3 = query_meta_prefix_page_at_revision_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        prefix.as_str(),
        16,
        None,
        Some(r3),
    )
    .await?;
    require_meta_values(&rev3, &[(&profile_key, profile_v3.as_str(), r1, r3, 3)])?;

    let rev5 = query_meta_prefix_page_at_revision_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        prefix.as_str(),
        16,
        None,
        Some(r5),
    )
    .await?;
    require_meta_values(
        &rev5,
        &[
            (&profile_key, profile_v4, r1, r4, 4),
            (&tx_key2, "tx-value-2", r4, r4, 1),
        ],
    )?;

    let current = query_meta_prefix_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        prefix.as_str(),
        16,
    )
    .await?;
    require_meta_values(
        &current,
        &[
            (&profile_key, profile_v4, r1, r4, 4),
            (&tx_key1, "tx-value-1-recreated", r6, r6, 1),
            (&tx_key2, "tx-value-2", r4, r4, 1),
        ],
    )?;

    let all_changes = query_meta_changes_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        prefix.as_str(),
        r1,
        16,
        None,
    )
    .await?;
    if all_changes.has_more {
        return Err(format!(
            "system_config MVCC change-feed unexpectedly paginated: {:?}",
            all_changes
        ));
    }
    require_meta_changes(
        &all_changes,
        &[
            (r1, &profile_key, profile_v1, false, r1, 1),
            (r2, &profile_key, profile_v2, false, r1, 2),
            (r3, &profile_key, profile_v3.as_str(), false, r1, 3),
            (r4, &profile_key, profile_v4, false, r1, 4),
            (r4, &tx_key1, "tx-value-1", false, r4, 1),
            (r4, &tx_key2, "tx-value-2", false, r4, 1),
            (r5, &tx_key1, "tx-value-1", true, r4, 0),
            (r6, &tx_key1, "tx-value-1-recreated", false, r6, 1),
        ],
    )?;

    let compacted = post_meta_compact_via_admin_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        leader.name.as_str(),
        r5,
    )
    .await?;
    if compacted.compacted_revision != r5 || compacted.current_revision < r6 {
        return Err(format!(
            "unexpected system_config MVCC compaction response: {:?}, expected compacted={}, current>={}",
            compacted, r5, r6
        ));
    }

    expect_meta_query_status_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        Some(profile_key.as_str()),
        None,
        Some(r1),
        StatusCode::GONE,
        Some("COMPACTED"),
    )
    .await?;
    expect_meta_changes_status_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        prefix.as_str(),
        r1,
        StatusCode::GONE,
        Some("COMPACTED"),
    )
    .await?;

    let profile_after_compact = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": profile_key.as_str()}),
    )
    .await?;
    require_system_config_value(&profile_after_compact, profile_v4, r4)?;
    let recreated_after_compact = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        token.as_str(),
        "sys_config_get",
        json!({"key": tx_key1.as_str()}),
    )
    .await?;
    require_system_config_value(&recreated_after_compact, "tx-value-1-recreated", r6)?;

    let post_compact_changes = query_meta_changes_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        prefix.as_str(),
        r6,
        8,
        None,
    )
    .await?;
    require_meta_changes(
        &post_compact_changes,
        &[(r6, &tx_key1, "tx-value-1-recreated", false, r6, 1)],
    )?;

    println!(
        "[klog-cluster-dv] system_config MVCC ok: leader={}, endpoint={}, revisions=[{},{},{},{},{},{}], prefix={}",
        leader_id, endpoint, r1, r2, r3, r4, r5, r6, prefix
    );
    Ok(())
}

async fn run_local_gateway_system_config_mvcc() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_system_config_mvcc_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_system_config_multi_ood_mvcc_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-system-config-multi-ood-mvcc-dv";
    let setup =
        prepare_local_gateway_setup(harness, SYSTEM_CONFIG_MULTI_OOD_MVCC_MODE, route_prefix, 3)
            .await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let seed = nodes
        .first()
        .ok_or_else(|| "missing seed node".to_string())?
        .clone();
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };

    for node in &nodes {
        let config = write_klog_config(harness, node, &voter_config)?;
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        wait_tcp("127.0.0.1", node.ports.rpc, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(50),
    )
    .await?;
    let leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let leader = nodes
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| format!("leader node {} not found", leader_id))?;
    let source = nodes
        .first()
        .ok_or_else(|| "missing source gateway node".to_string())?;
    let target = nodes
        .iter()
        .find(|node| node.name != source.name)
        .ok_or_else(|| "missing target gateway node".to_string())?;
    let source_gateway_addr = gateway_addr(source, ingress_port);

    let repo_root = repo_root()?;
    let buckyos_root = get_buckyos_root();
    let system_config_bin = resolve_system_config_bin(&repo_root, &buckyos_root)?;
    let mut used_ports = collect_used_ports(&nodes, ingress_port);
    let mut endpoints = Vec::new();
    for node in &nodes {
        let system_config_port = pick_local_port(&mut used_ports)?;
        let klog_endpoint = format!(
            "http://127.0.0.1:{}{}",
            node.ports.rpc, KLOG_JSON_RPC_SERVICE_PATH
        );
        let process_name = format!("system-config-{}", node.name);
        let system_config_root = harness.root.join(process_name.as_str());
        spawn_system_config_with_options(
            harness,
            process_name.as_str(),
            &system_config_bin,
            system_config_root.as_path(),
            system_config_port,
            Some(klog_endpoint.as_str()),
            node.name.as_str(),
            false,
        )?;
        wait_tcp("127.0.0.1", system_config_port, Duration::from_secs(15)).await?;
        endpoints.push(SystemConfigRpcEndpoint {
            node_name: node.name.clone(),
            endpoint: format!(
                "http://127.0.0.1:{}{}",
                system_config_port, SYSTEM_CONFIG_RPC_SERVICE_PATH
            ),
            token: system_config_jwt(node.name.as_str(), "root", "scheduler")?,
        });
    }

    let suffix = unique_suffix("syscfg-multi-ood-mvcc");
    let base = format!("users/alice/klog_multi_ood_mvcc_dv/{}", suffix);
    let prefix = format!("{}/", base);
    let items_per_ood = 8usize;
    let mut create_tasks = Vec::new();
    for endpoint in &endpoints {
        for index in 0..items_per_ood {
            let client = client.clone();
            let endpoint_url = endpoint.endpoint.clone();
            let token = endpoint.token.clone();
            let node_name = endpoint.node_name.clone();
            let key = format!("{}{}/item-{:02}", prefix, endpoint.node_name, index);
            let value = format!("value-{}-{:02}", endpoint.node_name, index);
            create_tasks.push(tokio::spawn(async move {
                call_system_config_rpc(
                    &client,
                    endpoint_url.as_str(),
                    token.as_str(),
                    "sys_config_create",
                    json!({"key": key.as_str(), "value": value.as_str()}),
                )
                .await?;
                let got = call_system_config_rpc(
                    &client,
                    endpoint_url.as_str(),
                    token.as_str(),
                    "sys_config_get",
                    json!({"key": key.as_str()}),
                )
                .await?;
                let (_, revision) = system_config_value_and_version(&got)?;
                Ok::<_, String>((node_name, key, value, revision))
            }));
        }
    }

    let mut created_records = Vec::new();
    for task in create_tasks {
        let record = task
            .await
            .map_err(|err| format!("system_config create task join failed: {}", err))??;
        created_records.push(record);
    }
    created_records.sort_by(|left, right| left.1.cmp(&right.1));

    for (_, key, value, revision) in &created_records {
        for endpoint in &endpoints {
            let got = call_system_config_rpc(
                &client,
                endpoint.endpoint.as_str(),
                endpoint.token.as_str(),
                "sys_config_get",
                json!({"key": key.as_str()}),
            )
            .await?;
            require_system_config_value(&got, value.as_str(), *revision)?;
        }
    }

    for endpoint in &endpoints {
        let node_base = format!("{}/{}", base, endpoint.node_name);
        let listed = call_system_config_rpc(
            &client,
            endpoint.endpoint.as_str(),
            endpoint.token.as_str(),
            "sys_config_list",
            json!({"key": node_base.as_str()}),
        )
        .await?;
        let listed = listed.as_array().ok_or_else(|| {
            format!(
                "system_config multi-OOD list result is not array for {}: {}",
                endpoint.node_name, listed
            )
        })?;
        if listed.len() != items_per_ood {
            return Err(format!(
                "system_config multi-OOD list length mismatch on {}: expected={}, actual={}, value={}",
                endpoint.node_name,
                items_per_ood,
                listed.len(),
                Value::Array(listed.clone())
            ));
        }
    }

    let initial_current = query_meta_prefix_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        prefix.as_str(),
        128,
    )
    .await?;
    if initial_current.items.len() != created_records.len() {
        return Err(format!(
            "system_config multi-OOD initial current count mismatch: expected={}, actual={}, items={:?}",
            created_records.len(),
            initial_current.items.len(),
            initial_current.items
        ));
    }

    let shared_key = format!("{}shared/profile", prefix);
    let shared_v1 = "shared-v1";
    call_system_config_rpc(
        &client,
        endpoints[0].endpoint.as_str(),
        endpoints[0].token.as_str(),
        "sys_config_create",
        json!({"key": shared_key.as_str(), "value": shared_v1}),
    )
    .await?;
    let shared_created = call_system_config_rpc(
        &client,
        endpoints[1].endpoint.as_str(),
        endpoints[1].token.as_str(),
        "sys_config_get",
        json!({"key": shared_key.as_str()}),
    )
    .await?;
    let (_, shared_r1) = system_config_value_and_version(&shared_created)?;

    let mut cas_tasks = Vec::new();
    for endpoint in &endpoints {
        let client = client.clone();
        let endpoint_url = endpoint.endpoint.clone();
        let token = endpoint.token.clone();
        let node_name = endpoint.node_name.clone();
        let shared_key_for_task = shared_key.clone();
        let attempt_key = format!("{}shared/attempt-{}", prefix, endpoint.node_name);
        let candidate_value = format!("shared-by-{}", endpoint.node_name);
        cas_tasks.push(tokio::spawn(async move {
            let mut actions = serde_json::Map::new();
            actions.insert(
                shared_key_for_task.clone(),
                json!({
                    "action": "update",
                    "value": candidate_value.as_str()
                }),
            );
            actions.insert(
                attempt_key.clone(),
                json!({
                    "action": "create",
                    "value": candidate_value.as_str()
                }),
            );
            let result = call_system_config_rpc(
                &client,
                endpoint_url.as_str(),
                token.as_str(),
                "sys_config_exec_tx",
                json!({
                    "main_key": format!("{}:{}", shared_key_for_task, shared_r1),
                    "actions": actions
                }),
            )
            .await;
            Ok::<_, String>((node_name, candidate_value, attempt_key, result))
        }));
    }

    let mut cas_results = Vec::new();
    for task in cas_tasks {
        let record = task
            .await
            .map_err(|err| format!("system_config CAS task join failed: {}", err))??;
        cas_results.push(record);
    }
    let winners = cas_results
        .iter()
        .filter(|(_, _, _, result)| result.is_ok())
        .collect::<Vec<_>>();
    if winners.len() != 1 {
        return Err(format!(
            "system_config multi-OOD CAS expected exactly one winner, got {}: {:?}",
            winners.len(),
            cas_results
        ));
    }
    for (node_name, _, _, result) in &cas_results {
        if let Err(err) = result
            && !err.contains("revision mismatch")
        {
            return Err(format!(
                "system_config CAS loser {} returned unexpected error: {}",
                node_name, err
            ));
        }
    }
    let winner_value = winners[0].1.as_str();
    let winner_attempt_key = winners[0].2.as_str();
    let final_shared = call_system_config_rpc(
        &client,
        endpoints[2].endpoint.as_str(),
        endpoints[2].token.as_str(),
        "sys_config_get",
        json!({"key": shared_key.as_str()}),
    )
    .await?;
    let (_, shared_r2) = system_config_value_and_version(&final_shared)?;
    if shared_r2 <= shared_r1 {
        return Err(format!(
            "system_config shared CAS revision did not advance: before={}, after={}",
            shared_r1, shared_r2
        ));
    }
    require_system_config_value(&final_shared, winner_value, shared_r2)?;
    for endpoint in &endpoints {
        let shared = call_system_config_rpc(
            &client,
            endpoint.endpoint.as_str(),
            endpoint.token.as_str(),
            "sys_config_get",
            json!({"key": shared_key.as_str()}),
        )
        .await?;
        require_system_config_value(&shared, winner_value, shared_r2)?;
    }
    for (_, candidate_value, attempt_key, result) in &cas_results {
        let got = call_system_config_rpc(
            &client,
            endpoints[0].endpoint.as_str(),
            endpoints[0].token.as_str(),
            "sys_config_get",
            json!({"key": attempt_key.as_str()}),
        )
        .await?;
        if result.is_ok() {
            require_system_config_value(&got, candidate_value.as_str(), shared_r2)?;
        } else {
            require_system_config_null(&got, attempt_key.as_str())?;
        }
    }

    let stale_key = format!("{}shared/stale-partial", prefix);
    let mut stale_actions = serde_json::Map::new();
    stale_actions.insert(
        shared_key.clone(),
        json!({
            "action": "update",
            "value": "stale-shared-value"
        }),
    );
    stale_actions.insert(
        stale_key.clone(),
        json!({
            "action": "create",
            "value": "should-not-exist"
        }),
    );
    let stale_error = expect_system_config_rpc_error(
        &client,
        endpoints[1].endpoint.as_str(),
        endpoints[1].token.as_str(),
        "sys_config_exec_tx",
        json!({
            "main_key": format!("{}:{}", shared_key, shared_r1),
            "actions": stale_actions
        }),
    )
    .await?;
    if !stale_error.contains("revision mismatch") {
        return Err(format!(
            "system_config stale multi-OOD tx returned unexpected error: {}",
            stale_error
        ));
    }
    let stale = call_system_config_rpc(
        &client,
        endpoints[2].endpoint.as_str(),
        endpoints[2].token.as_str(),
        "sys_config_get",
        json!({"key": stale_key.as_str()}),
    )
    .await?;
    require_system_config_null(&stale, stale_key.as_str())?;

    let delete_key = format!("{}delete-recreate/item", prefix);
    call_system_config_rpc(
        &client,
        endpoints[0].endpoint.as_str(),
        endpoints[0].token.as_str(),
        "sys_config_create",
        json!({"key": delete_key.as_str(), "value": "delete-v1"}),
    )
    .await?;
    let delete_created = call_system_config_rpc(
        &client,
        endpoints[1].endpoint.as_str(),
        endpoints[1].token.as_str(),
        "sys_config_get",
        json!({"key": delete_key.as_str()}),
    )
    .await?;
    let (_, delete_r1) = system_config_value_and_version(&delete_created)?;
    call_system_config_rpc(
        &client,
        endpoints[1].endpoint.as_str(),
        endpoints[1].token.as_str(),
        "sys_config_delete",
        json!({"key": delete_key.as_str()}),
    )
    .await?;
    for endpoint in &endpoints {
        let deleted = call_system_config_rpc(
            &client,
            endpoint.endpoint.as_str(),
            endpoint.token.as_str(),
            "sys_config_get",
            json!({"key": delete_key.as_str()}),
        )
        .await?;
        require_system_config_null(&deleted, delete_key.as_str())?;
    }
    call_system_config_rpc(
        &client,
        endpoints[2].endpoint.as_str(),
        endpoints[2].token.as_str(),
        "sys_config_create",
        json!({"key": delete_key.as_str(), "value": "delete-v2"}),
    )
    .await?;
    let delete_recreated = call_system_config_rpc(
        &client,
        endpoints[0].endpoint.as_str(),
        endpoints[0].token.as_str(),
        "sys_config_get",
        json!({"key": delete_key.as_str()}),
    )
    .await?;
    let (_, delete_r3) = system_config_value_and_version(&delete_recreated)?;
    if delete_r3 <= delete_r1 {
        return Err(format!(
            "system_config delete/recreate revision did not advance: before={}, after={}",
            delete_r1, delete_r3
        ));
    }
    require_system_config_value(&delete_recreated, "delete-v2", delete_r3)?;

    let delete_changes = query_meta_changes_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        delete_key.as_str(),
        delete_r1,
        8,
        None,
    )
    .await?;
    if delete_changes.items.len() != 3 {
        return Err(format!(
            "system_config delete/recreate changes mismatch: {:?}",
            delete_changes
        ));
    }
    let delete_tombstone_revision = delete_changes.items[1].mod_revision;
    require_meta_changes(
        &delete_changes,
        &[
            (delete_r1, &delete_key, "delete-v1", false, delete_r1, 1),
            (
                delete_tombstone_revision,
                &delete_key,
                "delete-v1",
                true,
                delete_r1,
                0,
            ),
            (delete_r3, &delete_key, "delete-v2", false, delete_r3, 1),
        ],
    )?;

    let compacted = post_meta_compact_via_admin_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        leader.name.as_str(),
        delete_tombstone_revision,
    )
    .await?;
    if compacted.compacted_revision != delete_tombstone_revision
        || compacted.current_revision < delete_r3
    {
        return Err(format!(
            "unexpected system_config multi-OOD compaction response: {:?}, expected compacted={}, current>={}",
            compacted, delete_tombstone_revision, delete_r3
        ));
    }
    expect_meta_query_status_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        Some(delete_key.as_str()),
        None,
        Some(delete_r1),
        StatusCode::GONE,
        Some("COMPACTED"),
    )
    .await?;
    for endpoint in &endpoints {
        let current = call_system_config_rpc(
            &client,
            endpoint.endpoint.as_str(),
            endpoint.token.as_str(),
            "sys_config_get",
            json!({"key": delete_key.as_str()}),
        )
        .await?;
        require_system_config_value(&current, "delete-v2", delete_r3)?;
    }

    let final_expected_count = created_records.len() + 3;
    let final_current = query_meta_prefix_via_cluster_inter_route(
        &client,
        source_gateway_addr.as_str(),
        route_prefix,
        target.name.as_str(),
        prefix.as_str(),
        128,
    )
    .await?;
    if final_current.items.len() != final_expected_count {
        return Err(format!(
            "system_config multi-OOD final current count mismatch: expected={}, actual={}, winner_attempt={}, items={:?}",
            final_expected_count,
            final_current.items.len(),
            winner_attempt_key,
            final_current.items
        ));
    }

    let scheduler_dump = call_system_config_rpc(
        &client,
        endpoints[0].endpoint.as_str(),
        endpoints[0].token.as_str(),
        "dump_configs_for_scheduler",
        json!({}),
    )
    .await?;
    for (_, key, value, _) in created_records.iter().step_by(items_per_ood) {
        if scheduler_dump.get(key.as_str()).and_then(Value::as_str) != Some(value.as_str()) {
            return Err(format!(
                "scheduler dump missing multi-OOD key {} in {}",
                key, scheduler_dump
            ));
        }
    }
    if scheduler_dump
        .get(stale_key.as_str())
        .and_then(Value::as_str)
        .is_some()
    {
        return Err(format!(
            "scheduler dump contains stale partial key {} in {}",
            stale_key, scheduler_dump
        ));
    }

    println!(
        "[klog-cluster-dv] system_config multi-OOD MVCC ok: leader={}, endpoints={}, created={}, shared_revisions=[{},{}], delete_revisions=[{},{},{}], prefix={}",
        leader_id,
        endpoints.len(),
        created_records.len(),
        shared_r1,
        shared_r2,
        delete_r1,
        delete_tombstone_revision,
        delete_r3,
        prefix
    );
    Ok(())
}

async fn run_local_gateway_system_config_multi_ood_mvcc() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_system_config_multi_ood_mvcc_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_system_config_pagination_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-system-config-pagination-dv";
    let setup =
        prepare_local_gateway_setup(harness, SYSTEM_CONFIG_PAGINATION_MODE, route_prefix, 3)
            .await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let seed = nodes
        .first()
        .ok_or_else(|| "missing seed node".to_string())?
        .clone();
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };

    for node in &nodes {
        let config = write_klog_config(harness, node, &voter_config)?;
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        wait_tcp("127.0.0.1", node.ports.rpc, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(50),
    )
    .await?;
    let leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;
    let leader = nodes
        .iter()
        .find(|node| node.id == leader_id)
        .ok_or_else(|| format!("leader node {} not found", leader_id))?;

    let item_count = 45usize;
    let page_limit = 17usize;
    let suffix = unique_suffix("syscfg-pagination");
    let base = format!("users/alice/klog_pagination_dv/{}", suffix);
    let prefix = format!("{}/", base);
    let source = nodes
        .first()
        .ok_or_else(|| "missing source gateway node".to_string())?;
    let target = nodes
        .iter()
        .find(|node| node.name != source.name)
        .ok_or_else(|| "missing target gateway node".to_string())?;
    let source_gateway_addr = gateway_addr(source, ingress_port);
    let expected_keys = (0..item_count)
        .map(|idx| format!("{}item-{:04}", prefix, idx))
        .collect::<Vec<_>>();

    for (idx, key) in expected_keys.iter().enumerate() {
        let value = format!("value-{:04}", idx);
        put_meta_via_cluster_inter_route(
            &client,
            source_gateway_addr.as_str(),
            route_prefix,
            target.name.as_str(),
            key.as_str(),
            value.as_str(),
            Some(0),
        )
        .await?;
    }

    let mut cursor = None;
    let mut page_sizes = Vec::new();
    let mut collected_keys = Vec::new();
    loop {
        let page = query_meta_prefix_page_via_cluster_inter_route(
            &client,
            source_gateway_addr.as_str(),
            route_prefix,
            target.name.as_str(),
            prefix.as_str(),
            page_limit,
            cursor.as_deref(),
        )
        .await?;
        if page.items.is_empty() && page.has_more {
            return Err("meta pagination returned empty page with has_more=true".to_string());
        }
        page_sizes.push(page.items.len());
        collected_keys.extend(page.items.iter().map(|item| item.key.clone()));
        if !page.has_more {
            break;
        }
        let Some(next_cursor) = page.next_cursor else {
            return Err("meta pagination missing next_cursor while has_more=true".to_string());
        };
        if cursor.as_ref() == Some(&next_cursor) {
            return Err(format!(
                "meta pagination cursor did not advance: {}",
                next_cursor
            ));
        }
        cursor = Some(next_cursor);
    }
    if collected_keys != expected_keys {
        return Err(format!(
            "meta pagination keys mismatch: expected_len={}, actual_len={}, page_sizes={:?}",
            expected_keys.len(),
            collected_keys.len(),
            page_sizes
        ));
    }
    if page_sizes != vec![17, 17, 11] {
        return Err(format!(
            "unexpected meta pagination page sizes: {:?}",
            page_sizes
        ));
    }

    let klog_endpoint = format!(
        "http://127.0.0.1:{}{}",
        leader.ports.rpc, KLOG_JSON_RPC_SERVICE_PATH
    );
    let repo_root = repo_root()?;
    let buckyos_root = get_buckyos_root();
    let system_config_bin = resolve_system_config_bin(&repo_root, &buckyos_root)?;
    let mut used_ports = collect_used_ports(&nodes, ingress_port);
    let system_config_port = pick_local_port(&mut used_ports)?;
    let page_limit_env = page_limit.to_string();
    spawn_system_config_with_extra_env(
        harness,
        &system_config_bin,
        system_config_port,
        klog_endpoint.as_str(),
        &[(
            ENV_SYSTEM_CONFIG_KLOG_META_QUERY_LIMIT,
            page_limit_env.as_str(),
        )],
    )?;
    wait_tcp("127.0.0.1", system_config_port, Duration::from_secs(15)).await?;

    let endpoint = format!(
        "http://127.0.0.1:{}{}",
        system_config_port, SYSTEM_CONFIG_RPC_SERVICE_PATH
    );
    let scheduler_token = system_config_jwt(TEST_DEVICE_NAME, "root", "scheduler")?;
    let listed = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        scheduler_token.as_str(),
        "sys_config_list",
        json!({"key": base}),
    )
    .await?;
    let listed = listed
        .as_array()
        .ok_or_else(|| format!("system_config paginated list is not array: {}", listed))?;
    if listed.len() != item_count {
        return Err(format!(
            "system_config paginated list length mismatch: expected={}, actual={}, value={}",
            item_count,
            listed.len(),
            Value::Array(listed.clone())
        ));
    }
    for idx in [0usize, 16, 17, 34, 44] {
        let expected_child = format!("item-{:04}", idx);
        if !listed
            .iter()
            .any(|value| value.as_str() == Some(expected_child.as_str()))
        {
            return Err(format!(
                "system_config paginated list missing child {}: {:?}",
                expected_child, listed
            ));
        }
    }

    let scheduler_dump = call_system_config_rpc(
        &client,
        endpoint.as_str(),
        scheduler_token.as_str(),
        "dump_configs_for_scheduler",
        json!({}),
    )
    .await?;
    for idx in [0usize, 17, 44] {
        let key = format!("{}item-{:04}", prefix, idx);
        if scheduler_dump.get(key.as_str()).and_then(Value::as_str)
            != Some(format!("value-{:04}", idx).as_str())
        {
            return Err(format!(
                "scheduler dump missing paginated key {} in {}",
                key, scheduler_dump
            ));
        }
    }

    println!(
        "[klog-cluster-dv] system_config pagination ok: leader={}, endpoint={}, prefix={}, items={}, page_sizes={:?}",
        leader_id, endpoint, prefix, item_count, page_sizes
    );
    Ok(())
}

async fn run_local_gateway_system_config_pagination() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_system_config_pagination_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_local_gateway_system_config_rollout_inner(
    harness: &mut LocalHarness,
) -> Result<(), String> {
    let route_prefix = "/.cluster/klog-it-system-config-rollout-dv";
    let setup =
        prepare_local_gateway_setup(harness, SYSTEM_CONFIG_ROLLOUT_MODE, route_prefix, 3).await?;
    let LocalGatewaySetup {
        klog_daemon_bin,
        route_prefix,
        ingress_port,
        nodes,
        cluster_name,
    } = setup;
    let route_prefix = route_prefix.as_str();
    let seed = nodes
        .first()
        .ok_or_else(|| "missing seed node".to_string())?
        .clone();
    let reader_node = nodes
        .get(1)
        .ok_or_else(|| "missing second OOD node".to_string())?
        .clone();
    let voter_config = KLogConfigOptions {
        seed: &seed,
        ingress_port,
        route_prefix,
        cluster_name: cluster_name.as_str(),
        auto_join_seed: true,
        target_role: "voter",
    };

    for node in &nodes {
        let config = write_klog_config(harness, node, &voter_config)?;
        spawn_klog(harness, &klog_daemon_bin, &config, node)?;
        wait_tcp("127.0.0.1", node.ports.admin, Duration::from_secs(12)).await?;
        wait_tcp("127.0.0.1", node.ports.rpc, Duration::from_secs(12)).await?;
        if node.id == seed.id {
            wait_voters(
                &reqwest::Client::new(),
                std::slice::from_ref(node),
                ingress_port,
                route_prefix,
                &[seed.id],
                Duration::from_secs(20),
            )
            .await?;
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;
    wait_membership(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        &[1, 2, 3],
        &[],
        Duration::from_secs(50),
    )
    .await?;
    let leader_id = wait_consistent_leader(
        &client,
        &nodes,
        ingress_port,
        route_prefix,
        None,
        Duration::from_secs(40),
    )
    .await?;

    let repo_root = repo_root()?;
    let buckyos_root = get_buckyos_root();
    let system_config_bin = resolve_system_config_bin(&repo_root, &buckyos_root)?;
    let mut used_ports = collect_used_ports(&nodes, ingress_port);
    let bootstrap_sled_port = pick_local_port(&mut used_ports)?;
    let reader_sled_port = pick_local_port(&mut used_ports)?;
    let bootstrap_klog_port = pick_local_port(&mut used_ports)?;
    let reader_klog_port = pick_local_port(&mut used_ports)?;
    let bootstrap_root = harness.root.join("system-config-ood1-root");
    let reader_root = harness.root.join("system-config-ood2-root");
    let bootstrap_token = system_config_jwt(seed.name.as_str(), "root", "scheduler")?;
    let reader_token = system_config_jwt(reader_node.name.as_str(), "root", "scheduler")?;
    let suffix = unique_suffix("syscfg-rollout");
    let base = format!("users/alice/klog_rollout_dv/{}", suffix);
    let migrated_key = format!("{}/migrated", base);
    let local_only_key = format!("{}/local_only", base);
    let reader_write_key = format!("{}/reader_write", base);

    spawn_system_config_with_options(
        harness,
        "system-config-ood1-sled",
        &system_config_bin,
        bootstrap_root.as_path(),
        bootstrap_sled_port,
        None,
        seed.name.as_str(),
        false,
    )?;
    wait_tcp("127.0.0.1", bootstrap_sled_port, Duration::from_secs(15)).await?;
    let bootstrap_sled_endpoint = format!(
        "http://127.0.0.1:{}{}",
        bootstrap_sled_port, SYSTEM_CONFIG_RPC_SERVICE_PATH
    );
    call_system_config_rpc(
        &client,
        bootstrap_sled_endpoint.as_str(),
        bootstrap_token.as_str(),
        "sys_config_create",
        json!({"key": migrated_key, "value": "from-ood1-sled"}),
    )
    .await?;
    harness.stop("system-config-ood1-sled")?;

    spawn_system_config_with_options(
        harness,
        "system-config-ood2-sled",
        &system_config_bin,
        reader_root.as_path(),
        reader_sled_port,
        None,
        reader_node.name.as_str(),
        false,
    )?;
    wait_tcp("127.0.0.1", reader_sled_port, Duration::from_secs(15)).await?;
    let reader_sled_endpoint = format!(
        "http://127.0.0.1:{}{}",
        reader_sled_port, SYSTEM_CONFIG_RPC_SERVICE_PATH
    );
    call_system_config_rpc(
        &client,
        reader_sled_endpoint.as_str(),
        reader_token.as_str(),
        "sys_config_create",
        json!({"key": local_only_key, "value": "from-ood2-local-sled"}),
    )
    .await?;
    harness.stop("system-config-ood2-sled")?;

    let bootstrap_klog_endpoint = format!(
        "http://127.0.0.1:{}{}",
        seed.ports.rpc, KLOG_JSON_RPC_SERVICE_PATH
    );
    spawn_system_config_with_options(
        harness,
        "system-config-ood1-klog-bootstrap",
        &system_config_bin,
        bootstrap_root.as_path(),
        bootstrap_klog_port,
        Some(bootstrap_klog_endpoint.as_str()),
        seed.name.as_str(),
        true,
    )?;
    wait_tcp("127.0.0.1", bootstrap_klog_port, Duration::from_secs(15)).await?;
    let bootstrap_klog_service_endpoint = format!(
        "http://127.0.0.1:{}{}",
        bootstrap_klog_port, SYSTEM_CONFIG_RPC_SERVICE_PATH
    );
    let migrated = call_system_config_rpc(
        &client,
        bootstrap_klog_service_endpoint.as_str(),
        bootstrap_token.as_str(),
        "sys_config_get",
        json!({"key": migrated_key}),
    )
    .await?;
    require_system_config_value(&migrated, "from-ood1-sled", 1)?;
    let local_only_after_bootstrap = call_system_config_rpc(
        &client,
        bootstrap_klog_service_endpoint.as_str(),
        bootstrap_token.as_str(),
        "sys_config_get",
        json!({"key": local_only_key}),
    )
    .await?;
    if !local_only_after_bootstrap.is_null() {
        return Err(format!(
            "non-bootstrap OOD local sled key was unexpectedly migrated before reader start: {}",
            local_only_after_bootstrap
        ));
    }

    let reader_klog_endpoint = format!(
        "http://127.0.0.1:{}{}",
        reader_node.ports.rpc, KLOG_JSON_RPC_SERVICE_PATH
    );
    spawn_system_config_with_options(
        harness,
        "system-config-ood2-klog-reader",
        &system_config_bin,
        reader_root.as_path(),
        reader_klog_port,
        Some(reader_klog_endpoint.as_str()),
        reader_node.name.as_str(),
        false,
    )?;
    wait_tcp("127.0.0.1", reader_klog_port, Duration::from_secs(15)).await?;
    let reader_klog_service_endpoint = format!(
        "http://127.0.0.1:{}{}",
        reader_klog_port, SYSTEM_CONFIG_RPC_SERVICE_PATH
    );
    let migrated_from_reader = call_system_config_rpc(
        &client,
        reader_klog_service_endpoint.as_str(),
        reader_token.as_str(),
        "sys_config_get",
        json!({"key": migrated_key}),
    )
    .await?;
    require_system_config_value(&migrated_from_reader, "from-ood1-sled", 1)?;
    let local_only_from_reader = call_system_config_rpc(
        &client,
        reader_klog_service_endpoint.as_str(),
        reader_token.as_str(),
        "sys_config_get",
        json!({"key": local_only_key}),
    )
    .await?;
    if !local_only_from_reader.is_null() {
        return Err(format!(
            "non-bootstrap OOD copied its local sled state without bootstrap flag: {}",
            local_only_from_reader
        ));
    }

    call_system_config_rpc(
        &client,
        reader_klog_service_endpoint.as_str(),
        reader_token.as_str(),
        "sys_config_create",
        json!({"key": reader_write_key, "value": "from-ood2-klog"}),
    )
    .await?;
    let reader_write_from_bootstrap = call_system_config_rpc(
        &client,
        bootstrap_klog_service_endpoint.as_str(),
        bootstrap_token.as_str(),
        "sys_config_get",
        json!({"key": reader_write_key}),
    )
    .await?;
    require_system_config_value(&reader_write_from_bootstrap, "from-ood2-klog", 1)?;

    println!(
        "[klog-cluster-dv] system_config rollout ok: leader={}, bootstrap_ood={}, reader_ood={}, prefix={}",
        leader_id, seed.name, reader_node.name, base
    );
    Ok(())
}

async fn run_local_gateway_system_config_rollout() -> Result<(), String> {
    let mut harness = LocalHarness::new()?;
    let result = run_local_gateway_system_config_rollout_inner(&mut harness).await;
    if result.is_err() || std::env::var_os("KLOG_CLUSTER_DV_KEEP_TEMP").is_some() {
        harness.keep_temp = true;
        eprintln!(
            "[klog-cluster-dv] keeping temp root for diagnostics: {}",
            harness.root.display()
        );
    }
    result
}

async fn run_installed_runtime_smoke() -> Result<(), String> {
    let buckyos_root = get_buckyos_root();
    let local_node_name = load_local_node_name(&buckyos_root)?;
    let cluster_route = load_klog_cluster_route(&buckyos_root)?;
    let local_route = require_node_route(&cluster_route, local_node_name.as_str())?;
    let gateway_addr = gateway_addr_from_route(&cluster_route);
    let timeout = Duration::from_secs(DEFAULT_TIMEOUT_SECS);
    let http_client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|err| format!("failed to build http client: {}", err))?;

    println!("[klog-cluster-dv] BUCKYOS_ROOT={}", buckyos_root.display());
    println!("[klog-cluster-dv] node_name={}", local_node_name);
    println!("[klog-cluster-dv] node_gateway_addr={}", gateway_addr);
    println!(
        "[klog-cluster-dv] route_prefix={}",
        cluster_route.route_prefix
    );
    println!(
        "[klog-cluster-dv] local_cluster_ports={:?}",
        local_route.ports
    );

    let suffix = unique_suffix("cluster-dv");

    let service_source = format!("test/test_klog_cluster_dv-service-{}", suffix);
    let service_append = append_via_service_route(
        &http_client,
        gateway_addr.as_str(),
        local_node_name.as_str(),
        service_source.as_str(),
        format!("cluster dv service append {}", suffix).as_str(),
    )
    .await?;
    let cluster_query = query_via_cluster_inter_route(
        &http_client,
        gateway_addr.as_str(),
        cluster_route.route_prefix.as_str(),
        local_node_name.as_str(),
        service_append.id,
        service_source.as_str(),
    )
    .await?;
    require_query_match(&cluster_query, service_append.id, service_source.as_str())?;
    println!(
        "[klog-cluster-dv] service append visible via cluster inter route: id={}",
        service_append.id
    );

    let cluster_source = format!("test/test_klog_cluster_dv-cluster-{}", suffix);
    let cluster_append = append_via_cluster_inter_route(
        &http_client,
        gateway_addr.as_str(),
        cluster_route.route_prefix.as_str(),
        local_node_name.as_str(),
        cluster_source.as_str(),
        format!("cluster dv inter append {}", suffix).as_str(),
    )
    .await?;
    let service_query = query_via_service_route(
        &http_client,
        gateway_addr.as_str(),
        cluster_append.id,
        cluster_source.as_str(),
    )
    .await?;
    require_query_match(&service_query, cluster_append.id, cluster_source.as_str())?;
    println!(
        "[klog-cluster-dv] cluster inter append visible via service route: id={}",
        cluster_append.id
    );

    let cluster_state = fetch_cluster_state_via_admin_route(
        &http_client,
        gateway_addr.as_str(),
        cluster_route.route_prefix.as_str(),
        local_node_name.as_str(),
    )
    .await?;
    require_cluster_state(&cluster_state, local_node_name.as_str())?;
    println!("[klog-cluster-dv] admin cluster-state ok via cluster route");

    println!("[klog-cluster-dv] smoke test success");
    Ok(())
}

async fn run() -> Result<(), String> {
    match std::env::var("KLOG_CLUSTER_DV_MODE")
        .unwrap_or_default()
        .trim()
    {
        "" => run_installed_runtime_smoke().await,
        MULTI_NODE_MODE => run_local_gateway_failover_smoke().await,
        MEMBERSHIP_MODE => run_local_gateway_membership().await,
        OOD_MEMBERSHIP_MODE => run_local_gateway_ood_membership().await,
        OOD_LEADER_FAILOVER_SHRINK_MODE => run_local_gateway_ood_leader_failover_shrink().await,
        OOD_SEED_UNAVAILABLE_JOIN_MODE => run_local_gateway_ood_seed_unavailable_join().await,
        OOD_SINGLE_TO_TWO_MODE => run_local_gateway_ood_single_to_two().await,
        OOD_TWO_VOTER_LOSS_MODE => run_local_gateway_ood_two_voter_loss().await,
        OOD_SNAPSHOT_MEMBERSHIP_MODE => run_local_gateway_ood_snapshot_membership().await,
        RESTART_RECOVERY_MODE => run_local_gateway_restart_recovery().await,
        MVCC_CLUSTER_MODE => run_local_gateway_mvcc_cluster().await,
        MVCC_CHANGE_FEED_MODE => run_local_gateway_mvcc_change_feed().await,
        MVCC_CHANGE_FEED_FAILOVER_MODE => run_local_gateway_mvcc_change_feed_failover().await,
        MVCC_CHANGE_FEED_STRESS_MODE => run_local_gateway_mvcc_change_feed_stress().await,
        MVCC_FAILOVER_MODE => run_local_gateway_mvcc_failover().await,
        MVCC_AUTO_COMPACT_FAILOVER_MODE => run_local_gateway_mvcc_auto_compact_failover().await,
        MVCC_COMPACTION_LEADER_SWITCH_MODE => {
            run_local_gateway_mvcc_compaction_leader_switch().await
        }
        MVCC_CRASH_RECOVERY_MODE => run_local_gateway_mvcc_crash_recovery().await,
        MVCC_COMPACT_DURING_SNAPSHOT_MODE => run_local_gateway_mvcc_compact_during_snapshot().await,
        RAFT_OLD_LEADER_REJOIN_MODE => run_local_gateway_raft_old_leader_rejoin().await,
        RAFT_FOLLOWER_LAG_SNAPSHOT_INSTALL_MODE => {
            run_local_gateway_raft_follower_lag_snapshot_install().await
        }
        RAFT_QUORUM_LOSS_RECOVERY_MODE => run_local_gateway_raft_quorum_loss_recovery().await,
        RAFT_MEMBERSHIP_CHANGE_REJOIN_MODE => {
            run_local_gateway_raft_membership_change_rejoin().await
        }
        RAFT_CONCURRENT_MEMBERSHIP_MODE => run_local_gateway_raft_concurrent_membership().await,
        RAFT_JOIN_RETRY_IDEMPOTENCY_MODE => run_local_gateway_raft_join_retry_idempotency().await,
        RAFT_SNAPSHOT_INSTALL_CRASH_MODE => run_local_gateway_raft_snapshot_install_crash().await,
        NODE_ID_REUSE_MODE => run_local_gateway_node_id_reuse().await,
        MVCC_SNAPSHOT_MEMBERSHIP_MODE => run_local_gateway_mvcc_snapshot_membership().await,
        SYSTEM_CONFIG_KV_MODE => run_local_gateway_system_config_kv().await,
        SYSTEM_CONFIG_SERVICE_MODE => run_local_gateway_system_config_service().await,
        SYSTEM_CONFIG_LEADER_FAILOVER_MODE => {
            run_local_gateway_system_config_leader_failover().await
        }
        GATEWAY_ABNORMAL_MODE => run_local_gateway_abnormal().await,
        SYSTEM_CONFIG_STALE_CONFIG_REJOIN_MODE => {
            run_local_gateway_system_config_stale_config_rejoin().await
        }
        SYSTEM_CONFIG_MVCC_MODE => run_local_gateway_system_config_mvcc().await,
        SYSTEM_CONFIG_MULTI_OOD_MVCC_MODE => run_local_gateway_system_config_multi_ood_mvcc().await,
        SYSTEM_CONFIG_PAGINATION_MODE => run_local_gateway_system_config_pagination().await,
        SYSTEM_CONFIG_ROLLOUT_MODE => run_local_gateway_system_config_rollout().await,
        other => {
            let supported = [
                "",
                MULTI_NODE_MODE,
                MEMBERSHIP_MODE,
                OOD_MEMBERSHIP_MODE,
                OOD_LEADER_FAILOVER_SHRINK_MODE,
                OOD_SEED_UNAVAILABLE_JOIN_MODE,
                OOD_SINGLE_TO_TWO_MODE,
                OOD_TWO_VOTER_LOSS_MODE,
                OOD_SNAPSHOT_MEMBERSHIP_MODE,
                RESTART_RECOVERY_MODE,
                MVCC_CLUSTER_MODE,
                MVCC_CHANGE_FEED_MODE,
                MVCC_CHANGE_FEED_FAILOVER_MODE,
                MVCC_CHANGE_FEED_STRESS_MODE,
                MVCC_FAILOVER_MODE,
                MVCC_AUTO_COMPACT_FAILOVER_MODE,
                MVCC_COMPACTION_LEADER_SWITCH_MODE,
                MVCC_CRASH_RECOVERY_MODE,
                MVCC_COMPACT_DURING_SNAPSHOT_MODE,
                RAFT_OLD_LEADER_REJOIN_MODE,
                RAFT_FOLLOWER_LAG_SNAPSHOT_INSTALL_MODE,
                RAFT_QUORUM_LOSS_RECOVERY_MODE,
                RAFT_MEMBERSHIP_CHANGE_REJOIN_MODE,
                RAFT_CONCURRENT_MEMBERSHIP_MODE,
                RAFT_JOIN_RETRY_IDEMPOTENCY_MODE,
                RAFT_SNAPSHOT_INSTALL_CRASH_MODE,
                NODE_ID_REUSE_MODE,
                MVCC_SNAPSHOT_MEMBERSHIP_MODE,
                SYSTEM_CONFIG_KV_MODE,
                SYSTEM_CONFIG_SERVICE_MODE,
                SYSTEM_CONFIG_LEADER_FAILOVER_MODE,
                GATEWAY_ABNORMAL_MODE,
                SYSTEM_CONFIG_STALE_CONFIG_REJOIN_MODE,
                SYSTEM_CONFIG_MVCC_MODE,
                SYSTEM_CONFIG_MULTI_OOD_MVCC_MODE,
                SYSTEM_CONFIG_PAGINATION_MODE,
                SYSTEM_CONFIG_ROLLOUT_MODE,
            ]
            .join("', '");
            Err(format!(
                "unsupported KLOG_CLUSTER_DV_MODE='{}'; supported values: '{}'",
                other, supported
            ))
        }
    }
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("[klog-cluster-dv][error] {}", err);
        std::process::exit(1);
    }
}
