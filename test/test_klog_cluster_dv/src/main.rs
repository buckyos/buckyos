use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DEFAULT_BUCKYOS_ROOT: &str = "/opt/buckyos";
const DEFAULT_CLUSTER_ROUTE_NAME: &str = "klog-service";
const DEFAULT_TIMEOUT_SECS: u64 = 15;
const KLOG_JSON_RPC_SERVICE_PATH: &str = "/kapi/klog-service";
const KLOG_RPC_METHOD_LOG_APPEND: &str = "klog.log.append";
const KLOG_RPC_METHOD_LOG_QUERY: &str = "klog.log.query";

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

async fn run() -> Result<(), String> {
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

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("[klog-cluster-dv][error] {}", err);
        std::process::exit(1);
    }
}
