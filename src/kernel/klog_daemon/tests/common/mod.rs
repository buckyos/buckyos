#![allow(dead_code)]

use klog::KClusterTransportMode;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::process::{Child, Command};
use tokio::time::{Instant, sleep};

#[derive(Debug, Clone, Deserialize)]
pub struct ClusterState {
    pub node_id: u64,
    pub server_state: String,
    pub current_leader: Option<u64>,
    pub voters: Vec<u64>,
    pub learners: Vec<u64>,
}

#[derive(Debug, Serialize)]
pub struct AppendLogBody {
    pub message: String,
    pub timestamp: Option<u64>,
    pub node_name: Option<String>,
    pub request_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AppendLogResponse {
    pub id: u64,
}

#[derive(Debug, Deserialize)]
pub struct QueryLogEntry {
    pub id: u64,
    pub timestamp: u64,
    pub node_name: String,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct QueryLogResponse {
    pub items: Vec<QueryLogEntry>,
}

pub struct TestNode {
    pub node_id: u64,
    pub port: u16,
    pub inter_node_port: u16,
    pub admin_port: u16,
    pub rpc_port: u16,
    pub data_dir: PathBuf,
    pub config_path: PathBuf,
    pub child: Child,
}

#[derive(Debug, Clone)]
pub struct TestNodeSpawnOptions {
    pub advertise_addr: String,
    pub advertise_port: Option<u16>,
    pub advertise_inter_port: Option<u16>,
    pub advertise_admin_port: Option<u16>,
    pub rpc_advertise_port: Option<u16>,
    pub advertise_node_name: Option<String>,
    pub cluster_network_mode: KClusterTransportMode,
    pub cluster_gateway_addr: String,
    pub cluster_gateway_route_prefix: String,
}

impl Default for TestNodeSpawnOptions {
    fn default() -> Self {
        Self {
            advertise_addr: "127.0.0.1".to_string(),
            advertise_port: None,
            advertise_inter_port: None,
            advertise_admin_port: None,
            rpc_advertise_port: None,
            advertise_node_name: None,
            cluster_network_mode: KClusterTransportMode::Direct,
            cluster_gateway_addr: "127.0.0.1:3180".to_string(),
            cluster_gateway_route_prefix: "/.cluster/klog".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TestGatewayNodeTarget {
    pub raft_port: u16,
    pub inter_port: u16,
    pub admin_port: u16,
}

pub struct TestGatewayProxy {
    pub addr: String,
    handle: tokio::task::JoinHandle<()>,
}

impl Drop for TestGatewayProxy {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

impl TestNode {
    pub async fn stop(&mut self) {
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }
}

impl Drop for TestNode {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
        unregister_admin_port(self.port);
        let _ = std::fs::remove_file(&self.config_path);
        let _ = std::fs::remove_dir_all(&self.data_dir);
    }
}

static RAFT_TO_ADMIN_PORT_MAP: OnceLock<Mutex<HashMap<u16, u16>>> = OnceLock::new();

fn admin_port_map() -> &'static Mutex<HashMap<u16, u16>> {
    RAFT_TO_ADMIN_PORT_MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn derive_admin_port(raft_port: u16) -> u16 {
    const OFFSET: u16 = 1_000;
    if raft_port <= u16::MAX - OFFSET {
        raft_port + OFFSET
    } else {
        raft_port - OFFSET
    }
}

fn register_admin_port(raft_port: u16, admin_port: u16) {
    if let Ok(mut m) = admin_port_map().lock() {
        m.insert(raft_port, admin_port);
    }
}

fn unregister_admin_port(raft_port: u16) {
    if let Ok(mut m) = admin_port_map().lock() {
        m.remove(&raft_port);
    }
}

fn resolve_admin_port(port_or_admin: u16) -> u16 {
    if let Ok(m) = admin_port_map().lock() {
        if let Some(admin_port) = m.get(&port_or_admin) {
            return *admin_port;
        }
        if m.values().any(|v| *v == port_or_admin) {
            return port_or_admin;
        }
    }
    derive_admin_port(port_or_admin)
}

fn normalize_join_targets_as_admin_endpoints(targets: &[String]) -> Vec<String> {
    targets
        .iter()
        .map(|target| {
            let trimmed = target.trim();
            if let Some((host, port_str)) = trimmed.rsplit_once(':')
                && let Ok(port) = port_str.parse::<u16>()
            {
                return format!("{}:{}", host, resolve_admin_port(port));
            }
            trimmed.to_string()
        })
        .collect()
}

pub fn unique_tmp_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "buckyos_klog_daemon_it_{}_{}_{}",
        std::process::id(),
        nanos,
        tag
    ))
}

pub fn choose_free_port() -> std::io::Result<u16> {
    for _ in 0..200 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        drop(listener);

        let admin_port = derive_admin_port(port);
        if port == admin_port {
            continue;
        }
        if std::net::TcpListener::bind(("127.0.0.1", admin_port)).is_ok() {
            return Ok(port);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AddrNotAvailable,
        "failed to choose free port pair for raft/admin",
    ))
}

pub fn choose_unique_ports(count: usize) -> Result<Vec<u16>, String> {
    let mut ports = Vec::with_capacity(count);
    let mut guard = HashSet::new();
    let mut attempts = 0usize;
    while ports.len() < count && attempts < 200 {
        attempts += 1;
        let p = choose_free_port().map_err(|e| format!("choose free port failed: {}", e))?;
        let admin_port = derive_admin_port(p);
        if p == admin_port || guard.contains(&p) || guard.contains(&admin_port) {
            continue;
        }
        if std::net::TcpListener::bind(("127.0.0.1", admin_port)).is_err() {
            continue;
        }
        guard.insert(p);
        guard.insert(admin_port);
        ports.push(p);
    }
    if ports.len() != count {
        return Err(format!(
            "failed to choose {} unique ports after {} attempts; chosen={:?}",
            count, attempts, ports
        ));
    }
    Ok(ports)
}

pub fn can_bind_localhost() -> bool {
    std::net::TcpListener::bind("127.0.0.1:0").is_ok()
}

pub async fn spawn_gateway_proxy(
    route_prefix: &str,
    targets: HashMap<String, TestGatewayNodeTarget>,
) -> Result<TestGatewayProxy, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("failed to bind test gateway proxy listener: {}", e))?;
    let addr = listener
        .local_addr()
        .map_err(|e| format!("failed to read test gateway proxy local addr: {}", e))?;
    let route_prefix = normalize_route_prefix(route_prefix);
    let client = reqwest::Client::builder()
        .pool_idle_timeout(Duration::from_secs(1))
        .build()
        .map_err(|e| format!("failed to build test gateway proxy client: {}", e))?;

    let handle = tokio::spawn(async move {
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(v) => v,
                Err(_) => break,
            };
            let route_prefix = route_prefix.clone();
            let targets = targets.clone();
            let client = client.clone();
            tokio::spawn(async move {
                let _ =
                    handle_gateway_proxy_connection(stream, route_prefix, targets, client).await;
            });
        }
    });

    Ok(TestGatewayProxy {
        addr: addr.to_string(),
        handle,
    })
}

pub fn make_targets_toml(targets: &[String]) -> String {
    let quoted = targets
        .iter()
        .map(|x| format!("\"{}\"", x))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{}]", quoted)
}

fn normalize_route_prefix(prefix: &str) -> String {
    let trimmed = prefix.trim();
    if trimmed.is_empty() {
        return "/.cluster/klog".to_string();
    }

    let with_leading_slash = if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{}", trimmed)
    };

    let normalized = with_leading_slash.trim_end_matches('/').to_string();
    if normalized.is_empty() {
        "/".to_string()
    } else {
        normalized
    }
}

async fn handle_gateway_proxy_connection(
    mut stream: tokio::net::TcpStream,
    route_prefix: String,
    targets: HashMap<String, TestGatewayNodeTarget>,
    client: reqwest::Client,
) -> Result<(), String> {
    let request = read_http_request(&mut stream).await?;
    let (target_port, forward_path) =
        resolve_gateway_proxy_target(&route_prefix, &targets, &request.path_and_query)?;
    let url = format!("http://127.0.0.1:{}{}", target_port, forward_path);
    let method = reqwest::Method::from_bytes(request.method.as_bytes())
        .map_err(|e| format!("invalid proxy method {}: {}", request.method, e))?;
    let mut builder = client.request(method, &url);
    for (name, value) in &request.headers {
        let lower = name.to_ascii_lowercase();
        if lower == "host"
            || lower == "connection"
            || lower == "proxy-connection"
            || lower == "content-length"
        {
            continue;
        }
        builder = builder.header(name, value);
    }
    if !request.body.is_empty() {
        builder = builder.body(request.body);
    }

    let response = builder
        .send()
        .await
        .map_err(|e| format!("proxy forward {} failed: {}", url, e))?;
    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .bytes()
        .await
        .map_err(|e| format!("proxy read response body {} failed: {}", url, e))?;

    let reason = status.canonical_reason().unwrap_or("Unknown");
    let mut response_head = format!(
        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        status.as_u16(),
        reason,
        body.len()
    );
    for (name, value) in headers.iter() {
        let lower = name.as_str().to_ascii_lowercase();
        if lower == "content-length" || lower == "connection" || lower == "transfer-encoding" {
            continue;
        }
        if let Ok(value) = value.to_str() {
            response_head.push_str(&format!("{}: {}\r\n", name.as_str(), value));
        }
    }
    response_head.push_str("\r\n");

    stream
        .write_all(response_head.as_bytes())
        .await
        .map_err(|e| format!("proxy write response head failed: {}", e))?;
    stream
        .write_all(&body)
        .await
        .map_err(|e| format!("proxy write response body failed: {}", e))?;
    stream
        .shutdown()
        .await
        .map_err(|e| format!("proxy shutdown failed: {}", e))?;
    Ok(())
}

fn resolve_gateway_proxy_target(
    route_prefix: &str,
    targets: &HashMap<String, TestGatewayNodeTarget>,
    path_and_query: &str,
) -> Result<(u16, String), String> {
    let (path, query) = match path_and_query.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (path_and_query, None),
    };
    let path = if path.starts_with(route_prefix) {
        &path[route_prefix.len()..]
    } else {
        return Err(format!(
            "proxy path '{}' does not start with route_prefix '{}'",
            path, route_prefix
        ));
    };
    let trimmed = path.trim_start_matches('/');
    let mut parts = trimmed.splitn(3, '/');
    let node_name = parts
        .next()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| format!("proxy path '{}' missing target node", path_and_query))?;
    let plane = parts
        .next()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| format!("proxy path '{}' missing plane", path_and_query))?;
    let rest = parts
        .next()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| format!("proxy path '{}' missing forwarded suffix", path_and_query))?;
    let target = targets
        .get(node_name)
        .ok_or_else(|| format!("proxy target node '{}' not found", node_name))?;
    let port = match plane {
        "raft" => target.raft_port,
        "inter" => target.inter_port,
        "admin" => target.admin_port,
        _ => {
            return Err(format!(
                "proxy path '{}' uses unsupported plane '{}'",
                path_and_query, plane
            ));
        }
    };
    let mut forward_path = format!("{}/{}/{}", route_prefix, node_name, plane);
    if !rest.is_empty() {
        forward_path.push('/');
        forward_path.push_str(rest);
    }
    if let Some(query) = query {
        forward_path.push('?');
        forward_path.push_str(query);
    }
    Ok((port, forward_path))
}

struct SimpleHttpRequest {
    method: String,
    path_and_query: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

async fn read_http_request(
    stream: &mut tokio::net::TcpStream,
) -> Result<SimpleHttpRequest, String> {
    let mut buf = Vec::new();
    let header_end = loop {
        let mut chunk = [0_u8; 4096];
        let n = stream
            .read(&mut chunk)
            .await
            .map_err(|e| format!("proxy read request failed: {}", e))?;
        if n == 0 {
            return Err("proxy client closed before request headers".to_string());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = find_header_end(&buf) {
            break pos;
        }
        if buf.len() > 1024 * 1024 {
            return Err("proxy request headers too large".to_string());
        }
    };
    let head = std::str::from_utf8(&buf[..header_end])
        .map_err(|e| format!("proxy request header is not valid utf-8: {}", e))?;
    let mut lines = head.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| "proxy request missing request line".to_string())?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts
        .next()
        .ok_or_else(|| "proxy request line missing method".to_string())?
        .to_string();
    let path_and_query = request_parts
        .next()
        .ok_or_else(|| "proxy request line missing path".to_string())?
        .to_string();

    let mut headers = Vec::new();
    let mut content_length = 0usize;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| format!("proxy request header missing ':' separator: {}", line))?;
        let value = value.trim().to_string();
        if name.eq_ignore_ascii_case("content-length") {
            content_length = value
                .parse::<usize>()
                .map_err(|e| format!("invalid content-length '{}': {}", value, e))?;
        }
        headers.push((name.to_string(), value));
    }

    let body_start = header_end + 4;
    let mut body = buf[body_start..].to_vec();
    while body.len() < content_length {
        let mut chunk = vec![0_u8; content_length - body.len()];
        let n = stream
            .read(&mut chunk)
            .await
            .map_err(|e| format!("proxy read request body failed: {}", e))?;
        if n == 0 {
            return Err(format!(
                "proxy client closed before full body: got={}, expect={}",
                body.len(),
                content_length
            ));
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(content_length);

    Ok(SimpleHttpRequest {
        method,
        path_and_query,
        headers,
        body,
    })
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

pub fn write_config_file(
    path: &PathBuf,
    node_id: u64,
    port: u16,
    inter_node_port: u16,
    admin_port: u16,
    rpc_port: u16,
    data_dir: &PathBuf,
    cluster_name: &str,
    auto_bootstrap: bool,
    join_targets: &[String],
    target_role: &str,
) -> Result<(), String> {
    write_config_file_with_options(
        path,
        node_id,
        port,
        inter_node_port,
        admin_port,
        rpc_port,
        data_dir,
        cluster_name,
        auto_bootstrap,
        join_targets,
        target_role,
        &TestNodeSpawnOptions::default(),
    )
}

pub fn write_config_file_with_options(
    path: &PathBuf,
    node_id: u64,
    port: u16,
    inter_node_port: u16,
    admin_port: u16,
    rpc_port: u16,
    data_dir: &PathBuf,
    cluster_name: &str,
    auto_bootstrap: bool,
    join_targets: &[String],
    target_role: &str,
    options: &TestNodeSpawnOptions,
) -> Result<(), String> {
    let advertise_port = options.advertise_port.unwrap_or(port);
    let advertise_inter_port = options.advertise_inter_port.unwrap_or(inter_node_port);
    let advertise_admin_port = options.advertise_admin_port.unwrap_or(admin_port);
    let rpc_advertise_port = options.rpc_advertise_port.unwrap_or(rpc_port);
    let advertise_node_name = options
        .advertise_node_name
        .as_ref()
        .map(|v| format!("advertise_node_name = \"{}\"\n", v))
        .unwrap_or_default();
    let content = format!(
        r#"
node_id = {node_id}

[network]
listen_addr = "127.0.0.1:{port}"
inter_node_listen_addr = "127.0.0.1:{inter_node_port}"
admin_listen_addr = "127.0.0.1:{admin_port}"
rpc_listen_addr = "127.0.0.1:{rpc_port}"
advertise_addr = "{advertise_addr}"
advertise_port = {advertise_port}
advertise_inter_port = {advertise_inter_port}
advertise_admin_port = {advertise_admin_port}
rpc_advertise_port = {rpc_advertise_port}
{advertise_node_name}

[storage]
data_dir = "{data_dir}"
state_store_sync_write = true

[cluster]
name = "{cluster_name}"
id = "{cluster_name}"
auto_bootstrap = {auto_bootstrap}

[cluster_network]
mode = "{cluster_network_mode}"
gateway_addr = "{cluster_gateway_addr}"
gateway_route_prefix = "{cluster_gateway_route_prefix}"

[join]
targets = {join_targets}
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
"#,
        node_id = node_id,
        port = port,
        inter_node_port = inter_node_port,
        admin_port = admin_port,
        rpc_port = rpc_port,
        advertise_addr = options.advertise_addr,
        advertise_port = advertise_port,
        advertise_inter_port = advertise_inter_port,
        advertise_admin_port = advertise_admin_port,
        rpc_advertise_port = rpc_advertise_port,
        advertise_node_name = advertise_node_name,
        data_dir = data_dir.display(),
        cluster_name = cluster_name,
        auto_bootstrap = auto_bootstrap,
        cluster_network_mode = options.cluster_network_mode.as_str(),
        cluster_gateway_addr = options.cluster_gateway_addr,
        cluster_gateway_route_prefix = options.cluster_gateway_route_prefix,
        join_targets = make_targets_toml(join_targets),
        target_role = target_role,
    );

    std::fs::write(path, content)
        .map_err(|e| format!("failed to write config {}: {}", path.display(), e))
}

pub fn daemon_bin_filename() -> &'static str {
    if cfg!(windows) {
        "klog_daemon.exe"
    } else {
        "klog_daemon"
    }
}

pub fn daemon_bin_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(v) = std::env::var("KLOG_DAEMON_BIN") {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            candidates.push(PathBuf::from(trimmed));
        }
    }

    if let Ok(v) = std::env::var("CARGO_BIN_EXE_klog_daemon") {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            candidates.push(PathBuf::from(trimmed));
        }
    }

    if let Ok(current_exe) = std::env::current_exe()
        && let Some(debug_dir) = current_exe.parent().and_then(|p| p.parent())
    {
        candidates.push(debug_dir.join(daemon_bin_filename()));
    }

    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let base = PathBuf::from(manifest_dir);
        candidates.push(base.join("../../target/debug").join(daemon_bin_filename()));
        candidates.push(
            base.join("../../target/release")
                .join(daemon_bin_filename()),
        );
    }

    // PATH fallback.
    candidates.push(PathBuf::from(daemon_bin_filename()));

    let mut dedup = HashSet::new();
    let mut unique = Vec::new();
    for c in candidates {
        let key = c.to_string_lossy().to_string();
        if dedup.insert(key) {
            unique.push(c);
        }
    }
    unique
}

pub fn resolve_daemon_bin() -> Result<(PathBuf, Vec<PathBuf>), String> {
    let candidates = daemon_bin_candidates();

    for c in &candidates {
        if c.components().count() > 1 || c.is_absolute() {
            if c.exists() {
                return Ok((c.clone(), candidates));
            }
            continue;
        }
    }

    if let Some(last) = candidates.last() {
        return Ok((last.clone(), candidates));
    }

    Err("no daemon binary candidates available".to_string())
}

pub async fn spawn_node(
    node_id: u64,
    port: u16,
    cluster_name: &str,
    auto_bootstrap: bool,
    join_targets: &[String],
    target_role: &str,
) -> Result<TestNode, String> {
    spawn_node_with_options(
        node_id,
        port,
        cluster_name,
        auto_bootstrap,
        join_targets,
        target_role,
        &TestNodeSpawnOptions::default(),
    )
    .await
}

pub async fn spawn_node_with_options(
    node_id: u64,
    port: u16,
    cluster_name: &str,
    auto_bootstrap: bool,
    join_targets: &[String],
    target_role: &str,
    options: &TestNodeSpawnOptions,
) -> Result<TestNode, String> {
    let rpc_port = loop {
        let p = choose_free_port().map_err(|e| format!("choose rpc port failed: {}", e))?;
        if p != port {
            break p;
        }
    };
    let admin_port = derive_admin_port(port);
    let inter_node_port = loop {
        let p = choose_free_port().map_err(|e| format!("choose inter-node port failed: {}", e))?;
        if p != port && p != rpc_port && p != admin_port {
            break p;
        }
    };
    spawn_node_on_ports_with_options(
        node_id,
        port,
        inter_node_port,
        admin_port,
        rpc_port,
        cluster_name,
        auto_bootstrap,
        join_targets,
        target_role,
        options,
    )
    .await
}

pub async fn spawn_node_on_ports_with_options(
    node_id: u64,
    port: u16,
    inter_node_port: u16,
    admin_port: u16,
    rpc_port: u16,
    cluster_name: &str,
    auto_bootstrap: bool,
    join_targets: &[String],
    target_role: &str,
    options: &TestNodeSpawnOptions,
) -> Result<TestNode, String> {
    let data_dir = unique_tmp_path(&format!("node{}_data", node_id));
    let config_path = unique_tmp_path(&format!("node{}_config.toml", node_id));
    std::fs::create_dir_all(&data_dir)
        .map_err(|e| format!("failed to create data dir {}: {}", data_dir.display(), e))?;
    let normalized_join_targets = normalize_join_targets_as_admin_endpoints(join_targets);
    write_config_file_with_options(
        &config_path,
        node_id,
        port,
        inter_node_port,
        admin_port,
        rpc_port,
        &data_dir,
        cluster_name,
        auto_bootstrap,
        &normalized_join_targets,
        target_role,
        options,
    )?;

    let mut child = spawn_daemon_child(&config_path).await?;
    register_admin_port(port, admin_port);

    if let Err(e) = wait_node_http_ready_after_spawn(
        &mut child,
        node_id,
        port,
        inter_node_port,
        admin_port,
        rpc_port,
        Duration::from_secs(12),
    )
    .await
    {
        unregister_admin_port(port);
        return Err(e);
    }

    Ok(TestNode {
        node_id,
        port,
        inter_node_port,
        admin_port,
        rpc_port,
        data_dir,
        config_path,
        child,
    })
}

pub async fn restart_node(node: &mut TestNode) -> Result<(), String> {
    node.stop().await;
    let mut child = spawn_daemon_child(&node.config_path).await?;
    register_admin_port(node.port, node.admin_port);
    if let Err(e) = wait_node_http_ready_after_spawn(
        &mut child,
        node.node_id,
        node.port,
        node.inter_node_port,
        node.admin_port,
        node.rpc_port,
        Duration::from_secs(12),
    )
    .await
    {
        unregister_admin_port(node.port);
        return Err(e);
    }
    node.child = child;
    Ok(())
}

async fn spawn_daemon_child(config_path: &PathBuf) -> Result<Child, String> {
    let (daemon_bin, candidates) = resolve_daemon_bin()?;
    let mut cmd = Command::new(&daemon_bin);
    cmd.env("KLOG_CONFIG_FILE", config_path)
        .env("RUST_LOG", "warn")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    cmd.spawn().map_err(|e| {
        let candidate_strings = candidates
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "<unknown>".to_string());
        format!(
            "failed to spawn klog_daemon: bin={}, cwd={}, config={}, candidates=[{}], err={}. If running outside cargo, set KLOG_DAEMON_BIN or run `cargo build -p klog_daemon` first",
            daemon_bin.display(),
            cwd,
            config_path.display(),
            candidate_strings,
            e
        )
    })
}

pub async fn wait_node_http_ready_after_spawn(
    child: &mut Child,
    node_id: u64,
    port: u16,
    inter_node_port: u16,
    admin_port: u16,
    rpc_port: u16,
    timeout: Duration,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(500))
        .build()
        .map_err(|e| format!("failed to build readiness http client: {}", e))?;
    let deadline = Instant::now() + timeout;
    let mut last_err = String::new();

    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| format!("failed to poll node process status: {}", e))?
        {
            return Err(format!(
                "node process exited before HTTP ready: node_id={}, port={}, status={}, last_err={}",
                node_id, port, status, last_err
            ));
        }

        match fetch_cluster_state(&client, admin_port).await {
            Ok(_) => {
                match query_logs(&client, inter_node_port, None, None, Some(1), Some(false)).await {
                    Ok(_) => match query_logs(&client, rpc_port, None, None, Some(1), Some(false))
                        .await
                    {
                        Ok(_) => return Ok(()),
                        Err(e) => {
                            last_err = format!("raft/inter ready but rpc not ready: {}", e);
                        }
                    },
                    Err(e) => {
                        last_err = format!("cluster-state ok but inter-node data not ready: {}", e);
                    }
                }
            }
            Err(e) => {
                last_err = e;
            }
        }

        if Instant::now() > deadline {
            return Err(format!(
                "timeout waiting node HTTP ready: node_id={}, port={}, last_err={}",
                node_id, port, last_err
            ));
        }

        sleep(Duration::from_millis(120)).await;
    }
}

pub async fn fetch_cluster_state(
    client: &reqwest::Client,
    port: u16,
) -> Result<ClusterState, String> {
    let admin_port = resolve_admin_port(port);
    let url = format!("http://127.0.0.1:{}/klog/admin/cluster-state", admin_port);
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("request {} failed: {}", url, e))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_else(|_| String::new());
        return Err(format!("request {} returned {}: {}", url, status, body));
    }
    resp.json::<ClusterState>()
        .await
        .map_err(|e| format!("decode {} failed: {}", url, e))
}

pub async fn append_log(
    client: &reqwest::Client,
    port: u16,
    message: &str,
    timestamp: Option<u64>,
    node_id: Option<u64>,
) -> Result<AppendLogResponse, String> {
    append_log_with_request_id(client, port, message, timestamp, node_id, None).await
}

pub async fn append_log_with_request_id(
    client: &reqwest::Client,
    port: u16,
    message: &str,
    timestamp: Option<u64>,
    node_id: Option<u64>,
    request_id: Option<&str>,
) -> Result<AppendLogResponse, String> {
    let url = format!("http://127.0.0.1:{}/klog/data/append", port);
    let body = AppendLogBody {
        message: message.to_string(),
        timestamp,
        node_name: node_id.map(|v| format!("node-{}", v)),
        request_id: request_id.map(|v| v.to_string()),
    };

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("request {} failed: {}", url, e))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_else(|_| String::new());
        return Err(format!("request {} returned {}: {}", url, status, text));
    }

    resp.json::<AppendLogResponse>()
        .await
        .map_err(|e| format!("decode {} failed: {}", url, e))
}

pub async fn query_logs(
    client: &reqwest::Client,
    port: u16,
    start_id: Option<u64>,
    end_id: Option<u64>,
    limit: Option<usize>,
    desc: Option<bool>,
) -> Result<QueryLogResponse, String> {
    query_logs_with_strong_read(client, port, start_id, end_id, limit, desc, None).await
}

pub async fn query_logs_with_strong_read(
    client: &reqwest::Client,
    port: u16,
    start_id: Option<u64>,
    end_id: Option<u64>,
    limit: Option<usize>,
    desc: Option<bool>,
    strong_read: Option<bool>,
) -> Result<QueryLogResponse, String> {
    let mut url = reqwest::Url::parse(&format!("http://127.0.0.1:{}/klog/data/query", port))
        .map_err(|e| format!("invalid query url: {}", e))?;
    {
        let mut q = url.query_pairs_mut();
        if let Some(v) = start_id {
            q.append_pair("start_id", &v.to_string());
        }
        if let Some(v) = end_id {
            q.append_pair("end_id", &v.to_string());
        }
        if let Some(v) = limit {
            q.append_pair("limit", &v.to_string());
        }
        if let Some(v) = desc {
            q.append_pair("desc", if v { "true" } else { "false" });
        }
        if let Some(v) = strong_read {
            q.append_pair("strong_read", if v { "true" } else { "false" });
        }
    }

    let resp = client
        .get(url.clone())
        .send()
        .await
        .map_err(|e| format!("request {} failed: {}", url, e))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_else(|_| String::new());
        return Err(format!("request {} returned {}: {}", url, status, text));
    }

    resp.json::<QueryLogResponse>()
        .await
        .map_err(|e| format!("decode {} failed: {}", url, e))
}

pub async fn wait_single_node_leader(
    port: u16,
    node_id: u64,
    timeout: Duration,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(800))
        .build()
        .map_err(|e| format!("failed to build http client: {}", e))?;
    let deadline = Instant::now() + timeout;
    loop {
        if Instant::now() > deadline {
            return Err(format!(
                "timeout waiting for single-node leader: node_id={}, port={}",
                node_id, port
            ));
        }

        if let Ok(state) = fetch_cluster_state(&client, port).await {
            let voters = state.voters.iter().copied().collect::<BTreeSet<_>>();
            if state.current_leader == Some(node_id) && voters == BTreeSet::from([node_id]) {
                return Ok(());
            }
        }

        sleep(Duration::from_millis(200)).await;
    }
}

pub async fn wait_cluster_voters(
    ports: &[u16],
    expect_voters: &[u64],
    timeout: Duration,
) -> Result<Vec<ClusterState>, String> {
    wait_cluster_membership(ports, expect_voters, &[], timeout).await
}

pub async fn wait_cluster_membership(
    ports: &[u16],
    expect_voters: &[u64],
    expect_learners: &[u64],
    timeout: Duration,
) -> Result<Vec<ClusterState>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(800))
        .build()
        .map_err(|e| format!("failed to build http client: {}", e))?;
    let expected_voters = expect_voters.iter().copied().collect::<BTreeSet<_>>();
    let expected_learners = expect_learners.iter().copied().collect::<BTreeSet<_>>();
    let deadline = Instant::now() + timeout;

    loop {
        if Instant::now() > deadline {
            return Err(format!(
                "timeout waiting cluster membership voters={:?}, learners={:?} on ports={:?}",
                expect_voters, expect_learners, ports
            ));
        }

        let mut states = Vec::with_capacity(ports.len());
        let mut all_ok = true;
        for port in ports {
            match fetch_cluster_state(&client, *port).await {
                Ok(state) => {
                    let got_voters = state.voters.iter().copied().collect::<BTreeSet<_>>();
                    let got_learners = state.learners.iter().copied().collect::<BTreeSet<_>>();
                    if got_voters != expected_voters || got_learners != expected_learners {
                        all_ok = false;
                    }
                    states.push(state);
                }
                Err(_) => {
                    all_ok = false;
                    break;
                }
            }
        }

        if all_ok && states.len() == ports.len() {
            return Ok(states);
        }

        sleep(Duration::from_millis(250)).await;
    }
}

pub async fn send_remove_learner(port: u16, node_id: u64) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(1200))
        .build()
        .map_err(|e| format!("failed to build http client: {}", e))?;
    let admin_port = resolve_admin_port(port);
    let url = format!(
        "http://127.0.0.1:{}/klog/admin/remove-learner?node_id={}",
        admin_port, node_id
    );
    let resp = client
        .post(&url)
        .send()
        .await
        .map_err(|e| format!("request {} failed: {}", url, e))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_else(|_| String::new());
    if status.is_success() {
        Ok(body)
    } else {
        Err(format!("request {} returned {}: {}", url, status, body))
    }
}

pub async fn send_add_learner(
    port: u16,
    node_id: u64,
    addr: &str,
    node_port: u16,
    blocking: bool,
) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(1200))
        .build()
        .map_err(|e| format!("failed to build http client: {}", e))?;
    let admin_port = resolve_admin_port(port);
    let url = format!(
        "http://127.0.0.1:{}/klog/admin/add-learner?node_id={}&addr={}&port={}&blocking={}",
        admin_port,
        node_id,
        addr,
        node_port,
        if blocking { "true" } else { "false" }
    );
    let resp = client
        .post(&url)
        .send()
        .await
        .map_err(|e| format!("request {} failed: {}", url, e))?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_else(|_| String::new());
    if status.is_success() {
        Ok(body)
    } else {
        Err(format!("request {} returned {}: {}", url, status, body))
    }
}

pub async fn add_learner_with_retry(
    voter_ports: &[u16],
    node_id: u64,
    node_port: u16,
    timeout: Duration,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(800))
        .build()
        .map_err(|e| format!("failed to build http client: {}", e))?;
    let deadline = Instant::now() + timeout;
    let mut last_err = String::new();

    loop {
        if Instant::now() > deadline {
            return Err(format!(
                "timeout adding learner node_id={}, node_port={}, voter_ports={:?}, last_err={}",
                node_id, node_port, voter_ports, last_err
            ));
        }

        let mut states = Vec::with_capacity(voter_ports.len());
        let mut all_ok = true;
        for port in voter_ports {
            match fetch_cluster_state(&client, *port).await {
                Ok(state) => states.push(state),
                Err(e) => {
                    all_ok = false;
                    last_err = format!("fetch cluster state failed on port {}: {}", port, e);
                    break;
                }
            }
        }

        if all_ok && states.len() == voter_ports.len() {
            let all_have_learner = states
                .iter()
                .all(|s| s.learners.iter().any(|id| *id == node_id));
            if all_have_learner {
                return Ok(());
            }

            let mut leader_port = None;
            if let Some(leader_id) = states.iter().find_map(|s| s.current_leader)
                && let Some(idx) = states.iter().position(|s| s.node_id == leader_id)
            {
                leader_port = voter_ports.get(idx).copied();
            }

            let mut candidate_ports = Vec::new();
            if let Some(p) = leader_port {
                candidate_ports.push(p);
            }
            for p in voter_ports {
                if !candidate_ports.contains(p) {
                    candidate_ports.push(*p);
                }
            }

            let mut errs = Vec::new();
            for p in candidate_ports {
                match send_add_learner(p, node_id, "127.0.0.1", node_port, true).await {
                    Ok(_) => {
                        errs.clear();
                        break;
                    }
                    Err(e) => errs.push(format!("port={}, err={}", p, e)),
                }
            }
            if errs.is_empty() {
                last_err = format!(
                    "add-learner accepted for node_id={}, waiting membership propagation",
                    node_id
                );
            } else {
                last_err = format!(
                    "add-learner failed for node_id={}: {}",
                    node_id,
                    errs.join(" | ")
                );
            }
        }

        sleep(Duration::from_millis(250)).await;
    }
}

pub async fn remove_learners_with_retry(
    voter_ports: &[u16],
    final_voters: &[u64],
    learner_ids: &[u64],
    timeout: Duration,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(800))
        .build()
        .map_err(|e| format!("failed to build http client: {}", e))?;
    let final_voters_set = final_voters.iter().copied().collect::<BTreeSet<_>>();
    let expected_learners_set = learner_ids.iter().copied().collect::<BTreeSet<_>>();
    let deadline = Instant::now() + timeout;
    let mut last_err = String::new();

    loop {
        if Instant::now() > deadline {
            return Err(format!(
                "timeout removing learners with voters={:?}, ports={:?}, last_err={}",
                final_voters, voter_ports, last_err
            ));
        }

        let mut states = Vec::with_capacity(voter_ports.len());
        let mut all_state_ok = true;
        for port in voter_ports {
            match fetch_cluster_state(&client, *port).await {
                Ok(state) => states.push(state),
                Err(e) => {
                    all_state_ok = false;
                    last_err = format!("fetch cluster state failed on port {}: {}", port, e);
                    break;
                }
            }
        }

        if all_state_ok && states.len() == voter_ports.len() {
            let mut all_voters_match = true;
            let mut all_learners_empty = true;
            for state in &states {
                let voters = state.voters.iter().copied().collect::<BTreeSet<_>>();
                if voters != final_voters_set {
                    all_voters_match = false;
                }
                if !state.learners.is_empty() {
                    all_learners_empty = false;
                }
            }
            if all_voters_match && all_learners_empty {
                return Ok(());
            }

            let mut pending_learners = BTreeSet::new();
            for state in &states {
                for learner in &state.learners {
                    pending_learners.insert(*learner);
                }
            }
            if !expected_learners_set.is_empty() {
                pending_learners = pending_learners
                    .intersection(&expected_learners_set)
                    .copied()
                    .collect();
            }

            let mut leader_port = None;
            for state in &states {
                if state.current_leader == Some(state.node_id)
                    && let Some(idx) = states.iter().position(|s| s.node_id == state.node_id)
                {
                    leader_port = voter_ports.get(idx).copied();
                    break;
                }
            }
            if leader_port.is_none()
                && let Some(leader_id) = states.iter().find_map(|s| s.current_leader)
            {
                if let Some(idx) = states.iter().position(|s| s.node_id == leader_id) {
                    leader_port = voter_ports.get(idx).copied();
                }
            }

            if pending_learners.is_empty() {
                last_err = format!(
                    "learners are not empty yet but no pending learner id extracted from voter states; states={:?}",
                    states
                        .iter()
                        .map(|s| format!("node_id={}, learners={:?}", s.node_id, s.learners))
                        .collect::<Vec<_>>()
                );
            } else if let Some(port) = leader_port {
                let mut remove_errors = Vec::new();
                for learner_id in pending_learners {
                    if let Err(e) = send_remove_learner(port, learner_id).await {
                        remove_errors.push(format!("node_id={}, err={}", learner_id, e));
                    }
                }
                if remove_errors.is_empty() {
                    last_err = format!(
                        "remove-learner accepted on leader port {}; waiting learners to drain",
                        port
                    );
                } else {
                    last_err = format!(
                        "remove-learner failed on leader port {}: {}",
                        port,
                        remove_errors.join(" | ")
                    );
                }
            } else {
                last_err = format!(
                    "leader not discovered from states: {:?}",
                    states
                        .iter()
                        .map(|s| format!("node_id={}, leader={:?}", s.node_id, s.current_leader))
                        .collect::<Vec<_>>()
                );
            }
        }

        sleep(Duration::from_millis(250)).await;
    }
}

pub async fn ensure_learners_absent_for_duration(
    voter_ports: &[u16],
    absent_learner_ids: &[u64],
    duration: Duration,
) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(800))
        .build()
        .map_err(|e| format!("failed to build http client: {}", e))?;
    let absent = absent_learner_ids.iter().copied().collect::<BTreeSet<_>>();
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        for port in voter_ports {
            let state = fetch_cluster_state(&client, *port).await?;
            let got = state.learners.iter().copied().collect::<BTreeSet<_>>();
            let overlap = got.intersection(&absent).copied().collect::<Vec<_>>();
            if !overlap.is_empty() {
                return Err(format!(
                    "unexpected learner rejoin observed: port={}, overlap={:?}, learners={:?}",
                    port, overlap, state.learners
                ));
            }
        }
        sleep(Duration::from_millis(250)).await;
    }
    Ok(())
}

pub async fn spawn_three_voter_cluster(
    cluster_name: &str,
    port1: u16,
    port2: u16,
    port3: u16,
) -> Result<Vec<TestNode>, String> {
    let default_options = [
        TestNodeSpawnOptions::default(),
        TestNodeSpawnOptions::default(),
        TestNodeSpawnOptions::default(),
    ];
    spawn_three_voter_cluster_with_options(cluster_name, port1, port2, port3, &default_options)
        .await
}

pub async fn spawn_three_voter_cluster_with_options(
    cluster_name: &str,
    port1: u16,
    port2: u16,
    port3: u16,
    options: &[TestNodeSpawnOptions; 3],
) -> Result<Vec<TestNode>, String> {
    let join_seed = vec![format!("127.0.0.1:{}", port1)];
    let mut nodes = Vec::new();
    nodes.push(
        spawn_node_with_options(1, port1, cluster_name, true, &[], "voter", &options[0]).await?,
    );
    wait_single_node_leader(port1, 1, Duration::from_secs(20)).await?;

    nodes.push(
        spawn_node_with_options(
            2,
            port2,
            cluster_name,
            false,
            &join_seed,
            "voter",
            &options[1],
        )
        .await?,
    );
    let _ = wait_cluster_voters(&[port1, port2], &[1, 2], Duration::from_secs(40)).await?;

    nodes.push(
        spawn_node_with_options(
            3,
            port3,
            cluster_name,
            false,
            &join_seed,
            "voter",
            &options[2],
        )
        .await?,
    );
    let _ =
        wait_cluster_voters(&[port1, port2, port3], &[1, 2, 3], Duration::from_secs(50)).await?;
    let leader =
        wait_consistent_leader_on_ports(&[port1, port2, port3], Duration::from_secs(40)).await?;
    if ![1_u64, 2_u64, 3_u64].contains(&leader) {
        return Err(format!(
            "unexpected leader id after voter cluster bootstrap: {}",
            leader
        ));
    }

    Ok(nodes)
}

pub fn rpc_port_by_node_id(nodes: &[TestNode], node_id: u64) -> Result<u16, String> {
    nodes
        .iter()
        .find(|n| n.node_id == node_id)
        .map(|n| n.rpc_port)
        .ok_or_else(|| format!("rpc port not found for node_id={}", node_id))
}

pub async fn wait_new_leader_on_ports(
    ports: &[u16],
    old_leader: u64,
    timeout: Duration,
) -> Result<u64, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(800))
        .build()
        .map_err(|e| format!("failed to build http client: {}", e))?;
    let deadline = Instant::now() + timeout;
    let mut last_observation = String::new();

    loop {
        if Instant::now() > deadline {
            return Err(format!(
                "timeout waiting new leader after old_leader={} on ports={:?}; last_observation={}",
                old_leader, ports, last_observation
            ));
        }

        let mut leader_by_role = None;
        let mut leaders = BTreeSet::new();
        let mut all_ok = true;
        let mut observations = Vec::new();
        for port in ports {
            match fetch_cluster_state(&client, *port).await {
                Ok(state) => {
                    observations.push(format!(
                        "port={}, node_id={}, state={}, current_leader={:?}, voters={:?}",
                        port, state.node_id, state.server_state, state.current_leader, state.voters
                    ));

                    if state.server_state == "Leader" && state.node_id != old_leader {
                        leader_by_role = Some(state.node_id);
                    }

                    if let Some(leader) = state.current_leader {
                        if leader != old_leader {
                            leaders.insert(leader);
                        } else {
                            all_ok = false;
                        }
                    } else {
                        all_ok = false;
                    }
                }
                Err(_) => {
                    observations.push(format!("port={}, state_fetch=err", port));
                    all_ok = false;
                }
            }
        }
        last_observation = observations.join(" | ");

        if let Some(leader) = leader_by_role {
            return Ok(leader);
        }

        if all_ok && leaders.len() == 1 {
            return leaders
                .iter()
                .next()
                .copied()
                .ok_or_else(|| "leader set unexpectedly empty".to_string());
        }

        sleep(Duration::from_millis(250)).await;
    }
}

pub async fn wait_consistent_leader_on_ports(
    ports: &[u16],
    timeout: Duration,
) -> Result<u64, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(800))
        .build()
        .map_err(|e| format!("failed to build http client: {}", e))?;
    let deadline = Instant::now() + timeout;
    let mut last_observation = String::new();

    loop {
        if Instant::now() > deadline {
            return Err(format!(
                "timeout waiting consistent leader on ports={:?}; last_observation={}",
                ports, last_observation
            ));
        }

        let mut leaders = BTreeSet::new();
        let mut all_ok = true;
        let mut observations = Vec::new();
        for port in ports {
            match fetch_cluster_state(&client, *port).await {
                Ok(state) => {
                    observations.push(format!(
                        "port={}, node_id={}, state={}, current_leader={:?}, voters={:?}",
                        port, state.node_id, state.server_state, state.current_leader, state.voters
                    ));
                    if let Some(leader) = state.current_leader {
                        leaders.insert(leader);
                    } else {
                        all_ok = false;
                    }
                }
                Err(e) => {
                    observations.push(format!("port={}, state_fetch_err={}", port, e));
                    all_ok = false;
                }
            }
        }
        last_observation = observations.join(" | ");

        if all_ok && leaders.len() == 1 {
            return leaders
                .iter()
                .next()
                .copied()
                .ok_or_else(|| "leader set unexpectedly empty".to_string());
        }

        sleep(Duration::from_millis(250)).await;
    }
}
