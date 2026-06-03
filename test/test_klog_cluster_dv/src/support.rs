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

