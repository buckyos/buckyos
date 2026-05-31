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
const RESTART_RECOVERY_MODE: &str = "local-gateway-restart-recovery";
const SYSTEM_CONFIG_KV_MODE: &str = "local-gateway-system-config-kv";
const SYSTEM_CONFIG_SERVICE_MODE: &str = "local-gateway-system-config-service";
const SYSTEM_CONFIG_ROLLOUT_MODE: &str = "local-gateway-system-config-rollout";
const KLOG_JSON_RPC_SERVICE_PATH: &str = "/kapi/klog-service";
const SYSTEM_CONFIG_RPC_SERVICE_PATH: &str = "/kapi/system_config";
const KLOG_RPC_METHOD_LOG_APPEND: &str = "klog.log.append";
const KLOG_RPC_METHOD_LOG_QUERY: &str = "klog.log.query";
const KLOG_CLUSTER_DV_ROUTE_MODE_ENV: &str = "KLOG_CLUSTER_DV_ROUTE_MODE";
const ENV_SYSTEM_CONFIG_PORT: &str = "BUCKYOS_SYSTEM_CONFIG_PORT";
const ENV_SYSTEM_CONFIG_STORE: &str = "BUCKYOS_SYSTEM_CONFIG_STORE";
const ENV_SYSTEM_CONFIG_KLOG_ENDPOINT: &str = "BUCKYOS_SYSTEM_CONFIG_KLOG_ENDPOINT";
const ENV_SYSTEM_CONFIG_KLOG_NODE_NAME: &str = "BUCKYOS_SYSTEM_CONFIG_KLOG_NODE_NAME";
const ENV_SYSTEM_CONFIG_KLOG_BOOTSTRAP_FROM_SLED: &str =
    "BUCKYOS_SYSTEM_CONFIG_KLOG_BOOTSTRAP_FROM_SLED";
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
}

#[derive(Debug, Deserialize)]
struct MetaQueryResponse {
    items: Vec<MetaEntry>,
}

#[derive(Debug, Deserialize)]
struct MetaEntry {
    key: String,
    value: String,
    revision: u64,
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
    if status.is_success() {
        Ok(())
    } else {
        Err(format!(
            "add-learner returned {} from {}: {}",
            status, url, body
        ))
    }
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
    let data_dir = harness.root.join(format!("klog-data-{}", node.name));
    fs::create_dir_all(&data_dir).map_err(|err| {
        format!(
            "failed to create klog data dir {}: {}",
            data_dir.display(),
            err
        )
    })?;
    let config_path = harness.root.join(format!("klog-{}.toml", node.name));
    let join_targets = if node.id == options.seed.id || !options.auto_join_seed {
        String::new()
    } else {
        format!("\"127.0.0.1:{}\"", options.seed.ports.admin)
    };
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
install_snapshot_timeout_ms = 5000
"#,
        node_id = node.id,
        raft_port = node.ports.raft,
        inter_port = node.ports.inter,
        admin_port = node.ports.admin,
        rpc_port = node.ports.rpc,
        node_name = node.name,
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
    let mut command = Command::new(klog_daemon_bin);
    command
        .env("KLOG_CONFIG_FILE", config_path)
        .env("RUST_LOG", "warn");
    harness.spawn(format!("klog-{}", node.name).as_str(), &mut command)
}

fn spawn_system_config(
    harness: &mut LocalHarness,
    system_config_bin: &Path,
    service_port: u16,
    klog_endpoint: &str,
) -> Result<(), String> {
    let buckyos_root = harness.root.clone();
    spawn_system_config_with_options(
        harness,
        "system-config-klog",
        system_config_bin,
        buckyos_root.as_path(),
        service_port,
        Some(klog_endpoint),
        TEST_DEVICE_NAME,
        false,
    )
}

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
        RESTART_RECOVERY_MODE => run_local_gateway_restart_recovery().await,
        SYSTEM_CONFIG_KV_MODE => run_local_gateway_system_config_kv().await,
        SYSTEM_CONFIG_SERVICE_MODE => run_local_gateway_system_config_service().await,
        SYSTEM_CONFIG_ROLLOUT_MODE => run_local_gateway_system_config_rollout().await,
        other => Err(format!(
            "unsupported KLOG_CLUSTER_DV_MODE='{}'; supported values: '', '{}', '{}', '{}', '{}', '{}', '{}'",
            other,
            MULTI_NODE_MODE,
            MEMBERSHIP_MODE,
            RESTART_RECOVERY_MODE,
            SYSTEM_CONFIG_KV_MODE,
            SYSTEM_CONFIG_SERVICE_MODE,
            SYSTEM_CONFIG_ROLLOUT_MODE
        )),
    }
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("[klog-cluster-dv][error] {}", err);
        std::process::exit(1);
    }
}
