use reqwest::Url;
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
    voters: Vec<u64>,
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
        self.processes.push(LocalProcess {
            name: name.to_string(),
            child: Some(child),
        });
        Ok(())
    }

    fn stop(&mut self, name: &str) -> Result<(), String> {
        let process = self
            .processes
            .iter_mut()
            .find(|process| process.name == name)
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

fn reserve_port(host: &str, port: u16) -> bool {
    TcpListener::bind((host, port)).is_ok()
}

fn pick_common_port(hosts: &[&str], used: &mut BTreeSet<u16>) -> Result<u16, String> {
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

fn build_local_node_defs(ingress_port: u16) -> Result<Vec<LocalNodeDef>, String> {
    let mut used = BTreeSet::from([ingress_port]);
    let hosts = ["127.0.0.1", "127.0.0.2", "127.0.0.3"];
    let mut nodes = Vec::with_capacity(hosts.len());
    for (idx, host) in hosts.iter().enumerate() {
        nodes.push(LocalNodeDef {
            id: (idx + 1) as u64,
            name: format!("ood{}", idx + 1),
            gateway_host: (*host).to_string(),
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
    all_nodes: &[LocalNodeDef],
    ingress_port: u16,
    route_prefix: &str,
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
        format!("bind: {}:{}", node.gateway_host, ingress_port).as_str(),
    );
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

    let cluster_nodes = all_nodes
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
    let routes = all_nodes
        .iter()
        .filter(|target| target.name != node.name)
        .map(|target| {
            (
                target.name.clone(),
                json!({
                    "direct": {
                        "url": format!("tcp://{}/", target.gateway_host),
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
                "route_prefix": route_prefix,
                "ingress_port": ingress_port,
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
    seed: &LocalNodeDef,
    ingress_port: u16,
    route_prefix: &str,
    cluster_name: &str,
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
    let join_targets = if node.id == seed.id {
        String::new()
    } else {
        format!("\"127.0.0.1:{}\"", seed.ports.admin)
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
target_role = "voter"

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
        cluster_name = cluster_name,
        auto_bootstrap = if node.id == seed.id { "true" } else { "false" },
        gateway_addr = gateway_addr(node, ingress_port),
        route_prefix = route_prefix,
        join_targets = join_targets,
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
    let repo_root = repo_root()?;
    let buckyos_root = get_buckyos_root();
    let cyfs_gateway_bin = resolve_cyfs_gateway_bin(&repo_root, &buckyos_root)?;
    let klog_daemon_bin = resolve_klog_daemon_bin(&repo_root, &buckyos_root)?;
    let route_prefix = "/.cluster/klog-it-dv";
    let gateway_hosts = ["127.0.0.1", "127.0.0.2", "127.0.0.3"];
    let mut used_ports = BTreeSet::new();
    let ingress_port = pick_common_port(&gateway_hosts, &mut used_ports)?;
    let nodes = build_local_node_defs(ingress_port)?;
    let cluster_name = format!("klog_gateway_cluster_dv_{}", ingress_port);

    println!("[klog-cluster-dv] mode={}", MULTI_NODE_MODE);
    println!("[klog-cluster-dv] temp_root={}", harness.root.display());
    println!(
        "[klog-cluster-dv] cyfs_gateway_bin={}",
        cyfs_gateway_bin.display()
    );
    println!(
        "[klog-cluster-dv] klog_daemon_bin={}",
        klog_daemon_bin.display()
    );
    println!("[klog-cluster-dv] ingress_port={}", ingress_port);

    for node in &nodes {
        let gateway_config = write_gateway_runtime(
            harness,
            &repo_root,
            &buckyos_root,
            node,
            &nodes,
            ingress_port,
            route_prefix,
        )?;
        spawn_gateway(harness, &cyfs_gateway_bin, &gateway_config, node)?;
        wait_tcp(
            node.gateway_host.as_str(),
            ingress_port,
            Duration::from_secs(8),
        )
        .await?;
    }

    let seed = nodes
        .first()
        .ok_or_else(|| "missing seed node".to_string())?
        .clone();
    for node in &nodes {
        let config = write_klog_config(
            harness,
            node,
            &seed,
            ingress_port,
            route_prefix,
            cluster_name.as_str(),
        )?;
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
        other => Err(format!(
            "unsupported KLOG_CLUSTER_DV_MODE='{}'; supported values: '', '{}'",
            other, MULTI_NODE_MODE
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
