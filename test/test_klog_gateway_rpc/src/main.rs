use buckyos_api::{KLogAppendRequest, KLogClient, KLogLevel, KLogQueryRequest};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DEFAULT_BUCKYOS_ROOT: &str = "/opt/buckyos";
const DEFAULT_NODE_GATEWAY_ADDR: &str = "127.0.0.1:3180";
const DEFAULT_CLUSTER_GATEWAY_ROUTE_PREFIX: &str = "/.cluster/klog";
const DEFAULT_TIMEOUT_SECS: u64 = 15;

fn get_buckyos_root() -> PathBuf {
    std::env::var("BUCKYOS_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_BUCKYOS_ROOT))
}

fn load_local_node_name(buckyos_root: &Path) -> Result<String, String> {
    let path = buckyos_root.join("etc/node_device_config.json");
    let text = fs::read_to_string(&path)
        .map_err(|err| format!("failed to read {}: {}", path.display(), err))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|err| format!("failed to decode {}: {}", path.display(), err))?;
    value
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("missing non-empty node name in {}", path.display()))
}

fn normalize_cluster_gateway_route_prefix(prefix: &str) -> String {
    let trimmed = prefix.trim();
    if trimmed.is_empty() {
        return DEFAULT_CLUSTER_GATEWAY_ROUTE_PREFIX.to_string();
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

fn cluster_gateway_route_prefix() -> String {
    std::env::var("KLOG_CLUSTER_GATEWAY_ROUTE_PREFIX")
        .ok()
        .map(|prefix| normalize_cluster_gateway_route_prefix(prefix.as_str()))
        .unwrap_or_else(|| DEFAULT_CLUSTER_GATEWAY_ROUTE_PREFIX.to_string())
}

fn unique_suffix(prefix: &str) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{}-{}", prefix, now)
}

fn require_query_match(
    payload: &Value,
    expected_id: u64,
    expected_source: &str,
) -> Result<(), String> {
    let items = payload
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("missing items array in query response: {}", payload))?;

    if items.len() != 1 {
        return Err(format!(
            "expected exactly one log item, got {}: {}",
            items.len(),
            payload
        ));
    }

    let item = &items[0];
    let id = item
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("missing id in query item: {}", item))?;
    if id != expected_id {
        return Err(format!(
            "query returned unexpected id: expected {}, got {}",
            expected_id, id
        ));
    }

    let source = item
        .get("source")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing source in query item: {}", item))?;
    if source != expected_source {
        return Err(format!(
            "query returned unexpected source: expected {}, got {}",
            expected_source, source
        ));
    }

    Ok(())
}

fn require_cluster_state(payload: &Value, expected_node_name: &str) -> Result<(), String> {
    let cluster_name = payload
        .get("cluster_name")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing cluster_name in cluster-state payload: {}", payload))?;
    if cluster_name.trim().is_empty() {
        return Err(format!("cluster_name is empty in payload: {}", payload));
    }

    let server_state = payload
        .get("server_state")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing server_state in cluster-state payload: {}", payload))?;
    if server_state.trim().is_empty() {
        return Err(format!("server_state is empty in payload: {}", payload));
    }

    let nodes = payload
        .get("nodes")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("missing nodes object in cluster-state payload: {}", payload))?;
    if nodes.is_empty() {
        return Err(format!("cluster-state nodes object is empty: {}", payload));
    }

    let found = nodes.values().any(|node| {
        node.get("node_name")
            .and_then(Value::as_str)
            .map(|name| name == expected_node_name)
            .unwrap_or(false)
    });
    if !found {
        return Err(format!(
            "cluster-state does not contain expected node_name {}: {}",
            expected_node_name, payload
        ));
    }

    Ok(())
}

async fn run() -> Result<(), String> {
    let buckyos_root = get_buckyos_root();
    let request_node_name = load_local_node_name(&buckyos_root)?;
    let route_prefix = cluster_gateway_route_prefix();
    let node_gateway_addr = std::env::var("KLOG_NODE_GATEWAY_ADDR")
        .unwrap_or_else(|_| DEFAULT_NODE_GATEWAY_ADDR.to_string());
    let timeout = Duration::from_secs(DEFAULT_TIMEOUT_SECS);

    println!(
        "[klog-gateway-smoke] BUCKYOS_ROOT={}",
        buckyos_root.display()
    );
    println!("[klog-gateway-smoke] node_name={}", request_node_name);
    println!(
        "[klog-gateway-smoke] node_gateway_addr={}",
        node_gateway_addr
    );
    println!("[klog-gateway-smoke] route_prefix={}", route_prefix);

    let client = KLogClient::from_buckyos_service_addr(
        node_gateway_addr.as_str(),
        request_node_name.clone(),
    )
    .with_timeout(timeout);

    let suffix = unique_suffix("gateway-smoke");
    let source = format!("test/test_klog_gateway_rpc-{}", suffix);
    let request_id = KLogClient::generate_request_id(request_node_name.as_str());
    let append = client
        .append_log(KLogAppendRequest {
            message: format!("gateway smoke append {}", suffix),
            timestamp: None,
            node_name: Some(request_node_name.clone()),
            level: Some(KLogLevel::Info),
            source: Some(source.clone()),
            attrs: None,
            request_id: Some(request_id.clone()),
        })
        .await
        .map_err(|err| format!("append_log via gateway failed: {}", err))?;
    println!(
        "[klog-gateway-smoke] append ok: id={}, request_id={}",
        append.id, request_id
    );

    let query = client
        .query_log(KLogQueryRequest {
            start_id: Some(append.id),
            end_id: Some(append.id),
            limit: Some(4),
            desc: Some(false),
            level: Some(KLogLevel::Info),
            source: Some(source.clone()),
            attr_key: None,
            attr_value: None,
            strong_read: Some(true),
        })
        .await
        .map_err(|err| format!("query_log via gateway failed: {}", err))?;
    let query_value = serde_json::to_value(&query)
        .map_err(|err| format!("failed to encode query response: {}", err))?;
    require_query_match(&query_value, append.id, source.as_str())?;
    println!(
        "[klog-gateway-smoke] query ok: matched log id={}",
        append.id
    );

    let cluster_state_url = format!(
        "http://{}{}{}/admin/cluster-state",
        node_gateway_addr,
        route_prefix,
        format!("/{}", request_node_name)
    );
    let cluster_state = reqwest::Client::new()
        .get(cluster_state_url.as_str())
        .timeout(timeout)
        .send()
        .await
        .map_err(|err| format!("cluster-state request failed: {}", err))?;
    let status = cluster_state.status();
    if !status.is_success() {
        let body = cluster_state
            .text()
            .await
            .unwrap_or_else(|err| format!("<failed to read body: {}>", err));
        return Err(format!(
            "cluster-state returned non-success status {}: {}",
            status, body
        ));
    }
    let cluster_state_value = cluster_state
        .json::<Value>()
        .await
        .map_err(|err| format!("failed to decode cluster-state response: {}", err))?;
    require_cluster_state(&cluster_state_value, request_node_name.as_str())?;
    println!("[klog-gateway-smoke] cluster-state ok via node gateway");

    println!("[klog-gateway-smoke] smoke test success");
    Ok(())
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("[klog-gateway-smoke][error] {}", err);
        std::process::exit(1);
    }
}
