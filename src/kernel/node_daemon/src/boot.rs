// Boot 阶段的角色识别、LAN OOD 发现、boot route 生成与 system-config 入口选择。
//
// 设计参考 doc/arch/gateway/boot_gateway的配置生成.md `角色启动流程` 章节。
// 三个角色（OOD / 非 OOD ZoneGateway / 普通 Node）流程虽不同，但下面的工具
// 都可被复用，差异只在调用顺序与目标。

use buckyos_api::{
    KLOG_CLUSTER_ADMIN_PORT, KLOG_CLUSTER_ADMIN_SERVICE_NAME, KLOG_CLUSTER_INTER_PORT,
    KLOG_CLUSTER_INTER_SERVICE_NAME, KLOG_CLUSTER_RAFT_PORT, KLOG_CLUSTER_RAFT_SERVICE_NAME,
    KLOG_SERVICE_UNIQUE_ID,
};
use jsonwebtoken::{DecodingKey, EncodingKey};
use log::*;
use name_lib::{DeviceConfig, ZoneBootConfig};
use serde_json::{json, Value};
use std::collections::HashMap;

use crate::finder::{DiscoveredNode, NodeFinderClient};

const DEFAULT_RTCP_PORT: u32 = 2980;
const DEFAULT_NODE_GATEWAY_HTTP_PORT: u16 = 3180;
const SYSTEM_CONFIG_PORT: u16 = 3200;
const KLOG_CLUSTER_ROUTE_PREFIX: &str = "/.cluster/klog";
const FINDER_DISCOVERY_TIMEOUT_SECS: u64 = 3;
// 仅用于 VM/dev 集成测试：VM 的 /etc/hosts 会写入同 Zone OOD 主机名，
// 此时可以让 boot 阶段生成 tcp_direct node-gateway 路由，绕过 RTCP tunnel 依赖。
// 默认不设置该变量，保持正式环境的 RTCP DID 路由和 keep_tunnel 行为。
const ENV_DEV_BOOT_LAN_ROUTE_KIND: &str = "BUCKYOS_DEV_BOOT_LAN_ROUTE_KIND";
const DEV_BOOT_LAN_ROUTE_TCP_DIRECT: &str = "tcp_direct";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeRole {
    Ood,
    ZoneGateway,
    Node,
}

impl NodeRole {
    pub fn from_zone_boot_config(zone_boot_config: &ZoneBootConfig, device_name: &str) -> Self {
        if zone_boot_config.device_is_ood(device_name) {
            NodeRole::Ood
        } else if zone_boot_config.device_is_gateway(device_name) {
            NodeRole::ZoneGateway
        } else {
            NodeRole::Node
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            NodeRole::Ood => "ood",
            NodeRole::ZoneGateway => "zone_gateway",
            NodeRole::Node => "node",
        }
    }
}

// 在局域网内查找其它 OOD。任何角色都可以调用：
//  - OOD 调用，找其它 OOD 以建立 keep_tunnel，满足 quorum；
//  - 非 OOD ZoneGateway / 普通 Node 调用，找 LAN 内的 OOD 用于 system-config 路由。
//
// `expected_oods` 控制需要找到几个 OOD 才能立即返回；为空时跑满 timeout。
// 失败时返回空 map（调用者决定降级策略），不会让 boot 卡死。
pub async fn discover_oods_in_lan(
    this_device_jwt: String,
    device_private_key: EncodingKey,
    zone_boot_config: ZoneBootConfig,
    owner_public_key: DecodingKey,
    role: NodeRole,
) -> HashMap<String, DiscoveredNode> {
    // OOD 自己作为 server 同时也是合法 client，使用强校验路径；其它角色用宽松版本
    // 以绕过"自身必须是 OOD"的检查。
    let client_result = match role {
        NodeRole::Ood => NodeFinderClient::new_for_zone(
            this_device_jwt,
            device_private_key,
            zone_boot_config,
            owner_public_key,
        ),
        NodeRole::ZoneGateway | NodeRole::Node => NodeFinderClient::new_as_lan_client(
            this_device_jwt,
            device_private_key,
            zone_boot_config,
            owner_public_key,
        ),
    };

    let client = match client_result {
        Ok(client) => client,
        Err(err) => {
            warn!("init NodeFinderClient for {:?} failed: {}", role, err);
            return HashMap::new();
        }
    };

    match client
        .looking_oods_by_udpv4(FINDER_DISCOVERY_TIMEOUT_SECS)
        .await
    {
        Ok(nodes) => {
            info!(
                "LAN OOD discovery done: role={}, found={}",
                role.as_str(),
                nodes.len()
            );
            nodes
        }
        Err(err) => {
            warn!(
                "LAN OOD discovery failed: role={}, err={}",
                role.as_str(),
                err
            );
            client.load_cached_oods().unwrap_or_default()
        }
    }
}

// Boot 阶段为本节点构造一份最小 `node_gateway_info.json` 内容。
// 目的：让 cyfs-gateway 在 scheduler 还没产出正式 routes 时，也能通过 boot routes
// 转发 `127.0.0.1:3180/kapi/system_config` 到 OOD。默认 route 仍是 RTCP；
// DEV VM 场景可用 `BUCKYOS_DEV_BOOT_LAN_ROUTE_KIND=tcp_direct` 切到 tcp_direct。
//
// 写入字段：
//  - node_info.this_node_id / this_zone_host
//  - service_info.system_config 的 selector 指向所有 OOD
//  - node_route_map 提供 OOD/ZoneGateway 的 boot URL（默认 RTCP，DEV 可为 tcp_direct）
//  - cluster_route_map.klog-service 提供 klog 集群启动期的 gateway proxy route
//  - routes 为新格式（per doc 设计）的 direct + via-sn 候选
//  - did_ip_hints 对齐 scheduler 生成的 gateway_info schema，boot 期写入 finder 发现的 RTCP IP
//  - app_info / trust_key 留空，等 scheduler 接管
pub fn build_boot_node_gateway_info(
    this_node_id: &str,
    zone_host: &str,
    zone_boot_config: &ZoneBootConfig,
    discovered_oods: &HashMap<String, DiscoveredNode>,
    sn_host_name: Option<&str>,
) -> Value {
    build_boot_node_gateway_info_inner(
        this_node_id,
        zone_host,
        zone_boot_config,
        discovered_oods,
        sn_host_name,
        dev_boot_lan_tcp_direct_enabled(),
    )
}

fn build_boot_node_gateway_info_inner(
    this_node_id: &str,
    zone_host: &str,
    zone_boot_config: &ZoneBootConfig,
    discovered_oods: &HashMap<String, DiscoveredNode>,
    sn_host_name: Option<&str>,
    prefer_tcp_direct: bool,
) -> Value {
    let oods_in_zone: Vec<&str> = zone_boot_config
        .oods
        .iter()
        .filter(|ood| ood.node_type.is_ood() && ood.name != this_node_id)
        .map(|ood| ood.name.as_str())
        .collect();

    let mut node_route_map: HashMap<String, String> = HashMap::new();
    let mut routes: HashMap<String, Vec<Value>> = HashMap::new();
    let mut did_ip_hints = serde_json::Map::new();

    for ood_name in oods_in_zone.iter() {
        let discovered = discovered_oods.get(*ood_name);
        let port = discovered
            .map(|node| node.rtcp_port as u32)
            .unwrap_or(DEFAULT_RTCP_PORT);
        let rtcp_host = discovered_ood_rtcp_host(discovered);
        if !prefer_tcp_direct {
            if let (Some(discovered), Some(rtcp_host)) = (discovered, rtcp_host.as_deref()) {
                did_ip_hints.insert(
                    rtcp_host.to_string(),
                    json!([{
                        "ip": discovered.addr.ip(),
                        "port": port,
                        "source": "lan_endpoint",
                        "confidence": "medium",
                        "last_observed_at": discovered.last_seen,
                    }]),
                );
            }
        }
        let direct_route = build_lan_direct_route(
            ood_name,
            zone_host,
            rtcp_host.as_deref(),
            port,
            prefer_tcp_direct,
        );
        let direct_url = direct_route.url;
        node_route_map.insert((*ood_name).to_string(), direct_url.clone());

        let mut candidates = vec![build_route_candidate(
            "direct",
            direct_route.kind,
            10,
            false,
            direct_route.keep_tunnel,
            &direct_url,
            "zone_boot_config",
            None,
            evidence_for_direct(discovered_oods.get(*ood_name)),
        )];

        if let Some(sn) = sn_host_name {
            let relay_url = format_relay_rtcp_url(sn, ood_name, zone_host, port);
            candidates.push(build_route_candidate(
                "via-sn",
                "rtcp_relay",
                30,
                true,
                true,
                &relay_url,
                "zone_boot_config",
                Some(sn),
                None,
            ));
        }

        routes.insert((*ood_name).to_string(), candidates);
    }

    // 非 OOD ZoneGateway 也是 boot 阶段需要 keep_tunnel 的目标之一
    for ood in zone_boot_config.oods.iter() {
        if ood.name == this_node_id {
            continue;
        }
        if ood.node_type.is_ood() {
            continue;
        }
        if !ood.node_type.is_gateway() {
            continue;
        }
        let direct_route = build_lan_direct_route(
            ood.name.as_str(),
            zone_host,
            None,
            DEFAULT_RTCP_PORT,
            prefer_tcp_direct,
        );
        let direct_url = direct_route.url;
        node_route_map
            .entry(ood.name.clone())
            .or_insert_with(|| direct_url.clone());
        routes.entry(ood.name.clone()).or_insert_with(|| {
            vec![build_route_candidate(
                "direct",
                direct_route.kind,
                10,
                false,
                direct_route.keep_tunnel,
                &direct_url,
                "zone_boot_config",
                None,
                None,
            )]
        });
    }

    // service_info.system_config 让 boot_gateway.yaml 的 forward_to_service 能命中
    // OOD 上的 system_config 服务。selector 指向所有 OOD；本节点是 OOD 时，
    // forward_to_service 会先检测 THIS_NODE_ID 命中，走本机 127.0.0.1。
    let mut sysconfig_selector = serde_json::Map::new();
    for ood in zone_boot_config.oods.iter() {
        if !ood.node_type.is_ood() {
            continue;
        }
        sysconfig_selector.insert(
            ood.name.clone(),
            json!({
                "port": SYSTEM_CONFIG_PORT,
                "weight": 100,
            }),
        );
    }

    let mut service_info = serde_json::Map::new();
    if !sysconfig_selector.is_empty() {
        service_info.insert(
            "system_config".to_string(),
            json!({ "selector": Value::Object(sysconfig_selector) }),
        );
    }

    let mut klog_cluster_nodes = serde_json::Map::new();
    for ood in zone_boot_config.oods.iter() {
        if !ood.node_type.is_ood() {
            continue;
        }
        let mut ports = serde_json::Map::new();
        ports.insert(
            KLOG_CLUSTER_RAFT_SERVICE_NAME.to_string(),
            json!(KLOG_CLUSTER_RAFT_PORT),
        );
        ports.insert(
            KLOG_CLUSTER_INTER_SERVICE_NAME.to_string(),
            json!(KLOG_CLUSTER_INTER_PORT),
        );
        ports.insert(
            KLOG_CLUSTER_ADMIN_SERVICE_NAME.to_string(),
            json!(KLOG_CLUSTER_ADMIN_PORT),
        );
        klog_cluster_nodes.insert(
            ood.name.clone(),
            json!({
                "ports": Value::Object(ports),
            }),
        );
    }

    let mut cluster_route_map = serde_json::Map::new();
    if !klog_cluster_nodes.is_empty() {
        cluster_route_map.insert(
            KLOG_SERVICE_UNIQUE_ID.to_string(),
            json!({
                "route_prefix": KLOG_CLUSTER_ROUTE_PREFIX,
                "ingress_port": DEFAULT_NODE_GATEWAY_HTTP_PORT,
                "nodes": Value::Object(klog_cluster_nodes),
            }),
        );
    }

    json!({
        "node_info": {
            "this_node_id": this_node_id,
            "this_zone_host": zone_host,
        },
        "app_info": {},
        "service_info": service_info,
        "node_route_map": node_route_map,
        "routes": routes,
        "did_ip_hints": Value::Object(did_ip_hints),
        "cluster_route_map": Value::Object(cluster_route_map),
        "trust_key": {},
    })
}

struct BootDirectRoute {
    kind: &'static str,
    url: String,
    keep_tunnel: bool,
}

fn build_lan_direct_route(
    node_id: &str,
    zone_host: &str,
    rtcp_host: Option<&str>,
    rtcp_port: u32,
    prefer_tcp_direct: bool,
) -> BootDirectRoute {
    if prefer_tcp_direct {
        // dev-only tcp_direct 指向对端 node-gateway 主机名；它不是直连 klog 端口。
        // 生产默认路径仍为下面的 rtcp_direct DID URL。
        return BootDirectRoute {
            kind: "tcp_direct",
            url: format_tcp_direct_node_url(node_id, zone_host),
            keep_tunnel: false,
        };
    }

    let rtcp_host = rtcp_host
        .filter(|host| !host.trim().is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("{}.{}", node_id, zone_host));
    BootDirectRoute {
        kind: "rtcp_direct",
        url: format_rtcp_host_url(rtcp_host.as_str(), rtcp_port),
        keep_tunnel: true,
    }
}

fn build_route_candidate(
    id: &str,
    kind: &str,
    priority: u32,
    backup: bool,
    keep_tunnel: bool,
    url: &str,
    source: &str,
    relay_node: Option<&str>,
    evidence: Option<Value>,
) -> Value {
    let mut entry = serde_json::Map::new();
    entry.insert("id".to_string(), json!(id));
    entry.insert("kind".to_string(), json!(kind));
    entry.insert("priority".to_string(), json!(priority));
    entry.insert("weight".to_string(), json!(100));
    entry.insert("backup".to_string(), json!(backup));
    entry.insert("keep_tunnel".to_string(), json!(keep_tunnel));
    entry.insert("url".to_string(), json!(url));
    entry.insert("source".to_string(), json!(source));
    if let Some(relay) = relay_node {
        entry.insert("relay_node".to_string(), json!(relay));
    }
    if let Some(ev) = evidence {
        entry.insert("evidence".to_string(), ev);
    }
    Value::Object(entry)
}

fn evidence_for_direct(node: Option<&DiscoveredNode>) -> Option<Value> {
    let node = node?;
    Some(json!({
        "type": "lan_discovery",
        "source_node": node.node_id,
        "last_observed_at": node.last_seen,
        "confidence": "medium",
        "applicability": "same_lan",
    }))
}

fn format_rtcp_did_url(node_id: &str, zone_host: &str, port: u32) -> String {
    format_rtcp_host_url(format!("{}.{}", node_id, zone_host).as_str(), port)
}

fn format_rtcp_host_url(host: &str, port: u32) -> String {
    if port == DEFAULT_RTCP_PORT {
        format!("rtcp://{}/", host)
    } else {
        format!("rtcp://{}:{}/", host, port)
    }
}

fn discovered_ood_rtcp_host(node: Option<&DiscoveredNode>) -> Option<String> {
    node.map(|node| node.device_doc.id.to_host_name())
        .filter(|host| !host.trim().is_empty())
}

fn format_tcp_direct_node_url(node_id: &str, zone_host: &str) -> String {
    // cyfs-gateway tunnel URL uses the path as "host:port"; boot_gateway.yaml appends the port.
    format!("tcp:///{}.{}", node_id, zone_host)
}

fn dev_boot_lan_tcp_direct_enabled() -> bool {
    std::env::var(ENV_DEV_BOOT_LAN_ROUTE_KIND)
        .map(|value| value.eq_ignore_ascii_case(DEV_BOOT_LAN_ROUTE_TCP_DIRECT))
        .unwrap_or(false)
}

fn format_relay_rtcp_url(
    sn_host: &str,
    target_node_id: &str,
    zone_host: &str,
    target_port: u32,
) -> String {
    let bootstrap_url = format!("rtcp://{}/", sn_host);
    let encoded: String = url::form_urlencoded::byte_serialize(bootstrap_url.as_bytes()).collect();
    if target_port == DEFAULT_RTCP_PORT {
        format!("rtcp://{}@{}.{}/", encoded, target_node_id, zone_host)
    } else {
        format!(
            "rtcp://{}@{}.{}:{}/",
            encoded, target_node_id, zone_host, target_port
        )
    }
}

fn rtcp_keep_tunnel_target_from_url(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if !raw.starts_with("rtcp://") {
        return Some(raw.to_string());
    }

    let url = match url::Url::parse(raw) {
        Ok(url) => url,
        Err(err) => {
            warn!("ignore invalid rtcp keep_tunnel url {}: {}", raw, err);
            return None;
        }
    };
    if url.scheme() != "rtcp" {
        return Some(raw.to_string());
    }

    let host = match url.host_str() {
        Some(host) if !host.is_empty() => host,
        _ => return None,
    };

    let mut target = String::new();
    if !url.username().is_empty() {
        target.push_str(url.username());
        if let Some(password) = url.password() {
            target.push(':');
            target.push_str(password);
        }
        target.push('@');
    }
    target.push_str(host);
    if let Some(port) = url.port() {
        target.push(':');
        target.push_str(port.to_string().as_str());
    }

    Some(target)
}

// 启动 cyfs-gateway 前写入 node_rtcp.keep_tunnel 的目标。
// SN：本机非 wan 系时，需要它 keep tunnel 解决"被动可达"。
// 其它 OOD：作为 RTCP direct 的 keep_tunnel 目标。
pub fn build_keep_tunnel_targets(
    role: NodeRole,
    device_doc: &DeviceConfig,
    zone_boot_config: &ZoneBootConfig,
    discovered_oods: &HashMap<String, DiscoveredNode>,
    zone_host: &str,
    sn_host_name: Option<&str>,
) -> Vec<String> {
    build_keep_tunnel_targets_inner(
        role,
        device_doc,
        zone_boot_config,
        discovered_oods,
        zone_host,
        sn_host_name,
        dev_boot_lan_tcp_direct_enabled(),
    )
}

fn build_keep_tunnel_targets_inner(
    role: NodeRole,
    device_doc: &DeviceConfig,
    zone_boot_config: &ZoneBootConfig,
    discovered_oods: &HashMap<String, DiscoveredNode>,
    zone_host: &str,
    sn_host_name: Option<&str>,
    prefer_tcp_direct: bool,
) -> Vec<String> {
    let mut targets: Vec<String> = Vec::new();

    let net_id_is_wan = device_doc
        .net_id
        .as_ref()
        .map(|n| n.starts_with("wan"))
        .unwrap_or(false);
    if !net_id_is_wan {
        if let Some(sn) = sn_host_name {
            targets.push(sn.to_string());
        }
    }

    // boot 阶段对其它节点的 rtcp_port 没有可信来源，统一用默认 2980；
    // scheduler 接管后可生成包含真实端口的 routes。
    // dev tcp_direct 模式下不会生成 OOD RTCP route，因此这里也不写入 OOD
    // keep_tunnel，避免 VM 测试环境被 RTCP 连通性影响。
    match role {
        NodeRole::Ood | NodeRole::ZoneGateway => {
            for ood in zone_boot_config.oods.iter() {
                if ood.name == device_doc.name {
                    continue;
                }
                if prefer_tcp_direct {
                    continue;
                }
                let rtcp_host = discovered_ood_rtcp_host(discovered_oods.get(ood.name.as_str()))
                    .unwrap_or_else(|| format!("{}.{}", ood.name, zone_host));
                if let Some(target) = rtcp_keep_tunnel_target_from_url(
                    format_rtcp_host_url(&rtcp_host, DEFAULT_RTCP_PORT).as_str(),
                ) {
                    targets.push(target);
                }
            }
        }
        NodeRole::Node => {
            // 普通 Node：与至少 1 个、最多 2 个 OOD 维持 keep_tunnel，
            // 让"ZoneGateway 失效时也能走 OOD"。
            for ood in zone_boot_config
                .oods
                .iter()
                .filter(|ood| ood.node_type.is_ood())
                .take(2)
            {
                if prefer_tcp_direct {
                    continue;
                }
                let rtcp_host = discovered_ood_rtcp_host(discovered_oods.get(ood.name.as_str()))
                    .unwrap_or_else(|| format!("{}.{}", ood.name, zone_host));
                if let Some(target) = rtcp_keep_tunnel_target_from_url(
                    format_rtcp_host_url(&rtcp_host, DEFAULT_RTCP_PORT).as_str(),
                ) {
                    targets.push(target);
                }
            }
        }
    }

    targets
}

pub fn extract_keep_tunnel_targets_from_gateway_info(gateway_info: &Value) -> Vec<String> {
    let mut targets = Vec::new();
    let Some(routes) = gateway_info.get("routes").and_then(Value::as_object) else {
        return targets;
    };

    for candidates in routes.values() {
        let Some(candidates) = candidates.as_array() else {
            continue;
        };
        for candidate in candidates {
            let keep_tunnel = candidate
                .get("keep_tunnel")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !keep_tunnel {
                continue;
            }
            let Some(url) = candidate.get("url").and_then(Value::as_str) else {
                continue;
            };
            if url.trim().is_empty() {
                continue;
            }
            if let Some(target) = rtcp_keep_tunnel_target_from_url(url) {
                targets.push(target);
            }
        }
    }

    dedup_keep_tunnel_targets(&mut targets);
    targets
}

pub fn extract_keep_tunnel_targets_from_gateway_config(gateway_config: &Value) -> Vec<String> {
    let mut targets = gateway_config
        .get("stacks")
        .and_then(|stacks| stacks.get("node_rtcp"))
        .and_then(|node_rtcp| node_rtcp.get("keep_tunnel"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    dedup_keep_tunnel_targets(&mut targets);
    targets
}

pub fn read_local_gateway_keep_tunnel_targets() -> Vec<String> {
    let path = buckyos_kit::get_buckyos_system_etc_dir().join("node_gateway.json");
    let Some(gateway_config) = std::fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str::<Value>(content.as_str()).ok())
    else {
        return Vec::new();
    };
    extract_keep_tunnel_targets_from_gateway_config(&gateway_config)
}

pub fn dedup_keep_tunnel_targets(targets: &mut Vec<String>) {
    let mut deduped = Vec::with_capacity(targets.len());
    for target in targets.drain(..) {
        let Some(target) = rtcp_keep_tunnel_target_from_url(target.as_str()) else {
            continue;
        };
        if !deduped.iter().any(|item| item == &target) {
            deduped.push(target);
        }
    }
    *targets = deduped;
}

pub fn merge_keep_tunnel_into_gateway_config(
    mut gateway_config: Value,
    targets: &[String],
) -> Value {
    if !gateway_config.is_object() {
        gateway_config = json!({});
    }
    if gateway_config
        .get("stacks")
        .and_then(Value::as_object)
        .is_none()
    {
        gateway_config["stacks"] = json!({});
    }
    if gateway_config["stacks"]
        .get("node_rtcp")
        .and_then(Value::as_object)
        .is_none()
    {
        gateway_config["stacks"]["node_rtcp"] = json!({});
    }

    gateway_config["stacks"]["node_rtcp"]["keep_tunnel"] = json!(targets);
    gateway_config
}

pub fn merge_missing_boot_klog_gateway_info(
    mut gateway_info: Value,
    boot_gateway_info: &Value,
) -> Value {
    // klog voter 集群启动时，scheduler 可能还没来得及产出完整 runtime
    // gateway_info。保留 boot 阶段的 klog cluster route 可以避免 system_config
    // 依赖 klog quorum、klog quorum 又依赖 gateway route 的启动环。
    // 这里只补缺失字段，不覆盖 scheduler 已经发布的正式 gateway_info。
    let Some(boot_klog_route) = boot_gateway_info
        .get("cluster_route_map")
        .and_then(Value::as_object)
        .and_then(|cluster_route_map| cluster_route_map.get(KLOG_SERVICE_UNIQUE_ID))
        .cloned()
    else {
        return gateway_info;
    };

    if !gateway_info.is_object() {
        return gateway_info;
    }

    let has_klog_route = gateway_info
        .get("cluster_route_map")
        .and_then(Value::as_object)
        .and_then(|cluster_route_map| cluster_route_map.get(KLOG_SERVICE_UNIQUE_ID))
        .is_some();
    if !has_klog_route {
        if gateway_info
            .get("cluster_route_map")
            .and_then(Value::as_object)
            .is_none()
        {
            gateway_info["cluster_route_map"] = json!({});
        }
        if let Some(cluster_route_map) = gateway_info
            .get_mut("cluster_route_map")
            .and_then(Value::as_object_mut)
        {
            cluster_route_map.insert(KLOG_SERVICE_UNIQUE_ID.to_string(), boot_klog_route);
        }
    }

    merge_missing_object_field(&mut gateway_info, boot_gateway_info, "node_info");
    merge_missing_object_field(&mut gateway_info, boot_gateway_info, "app_info");
    merge_missing_object_field(&mut gateway_info, boot_gateway_info, "service_info");
    merge_missing_object_field(&mut gateway_info, boot_gateway_info, "node_route_map");
    merge_missing_object_field(&mut gateway_info, boot_gateway_info, "routes");
    merge_missing_object_field(&mut gateway_info, boot_gateway_info, "did_ip_hints");
    merge_missing_object_field(&mut gateway_info, boot_gateway_info, "trust_key");

    gateway_info
}

fn merge_missing_object_field(target: &mut Value, source: &Value, field: &str) {
    let Some(source_map) = source.get(field).and_then(Value::as_object) else {
        return;
    };

    if target.get(field).and_then(Value::as_object).is_none() {
        target[field] = Value::Object(serde_json::Map::new());
    }
    let Some(target_map) = target.get_mut(field).and_then(Value::as_object_mut) else {
        return;
    };
    for (key, value) in source_map.iter() {
        target_map
            .entry(key.clone())
            .or_insert_with(|| value.clone());
    }
}

pub fn write_boot_node_gateway_config(keep_tunnels: &[String]) -> std::io::Result<()> {
    let path = buckyos_kit::get_buckyos_system_etc_dir().join("node_gateway.json");
    let existing_config = std::fs::read_to_string(&path)
        .ok()
        .and_then(|content| serde_json::from_str::<Value>(content.as_str()).ok())
        .unwrap_or_else(|| json!({}));
    let content = merge_keep_tunnel_into_gateway_config(existing_config, keep_tunnels);
    let body = serde_json::to_string_pretty(&content).unwrap_or_else(|_| "{}".to_string());
    std::fs::write(&path, body.as_bytes())?;
    info!("write boot node_gateway.json -> {}", path.display());
    Ok(())
}

// 把 boot 阶段构造的 gateway info 写入 `$BUCKYOS_ROOT/etc/node_gateway_info.json`。
// 必须在启动 cyfs-gateway 之前完成，否则它会读到空文件 / 旧文件。
pub fn write_boot_node_gateway_info(content: &Value) -> std::io::Result<()> {
    let path = buckyos_kit::get_buckyos_system_etc_dir().join("node_gateway_info.json");
    let body = serde_json::to_string_pretty(content).unwrap_or_else(|_| "{}".to_string());
    std::fs::write(&path, body.as_bytes())?;
    info!("write boot node_gateway_info.json -> {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_keep_tunnel_preserves_existing_gateway_config() {
        let config = json!({
            "acme": {"enabled": true},
            "stacks": {
                "zone_tls": {"protocol": "tls"},
                "node_rtcp": {
                    "keep_tunnel": ["old"],
                    "hook_point": {}
                }
            }
        });

        let merged = merge_keep_tunnel_into_gateway_config(
            config,
            &["ood2.zone".to_string(), "ood3.zone".to_string()],
        );

        assert_eq!(merged["acme"]["enabled"], true);
        assert_eq!(merged["stacks"]["zone_tls"]["protocol"], "tls");
        assert_eq!(
            merged["stacks"]["node_rtcp"]["keep_tunnel"],
            json!(["ood2.zone", "ood3.zone"])
        );
    }

    #[test]
    fn extract_keep_tunnel_targets_from_gateway_info_uses_route_flag() {
        let gateway_info = json!({
            "routes": {
                "ood2": [
                    {"url": "rtcp://ood2.zone/", "keep_tunnel": true},
                    {"url": "rtcp://backup@ood2.zone/", "keep_tunnel": false}
                ],
                "ood3": [
                    {"url": "rtcp://ood3.zone/", "keep_tunnel": true}
                ]
            }
        });

        assert_eq!(
            extract_keep_tunnel_targets_from_gateway_info(&gateway_info),
            vec!["ood2.zone".to_string(), "ood3.zone".to_string()]
        );
    }

    #[test]
    fn merge_missing_boot_klog_gateway_info_preserves_boot_routes() {
        let runtime_gateway_info = json!({
            "node_info": {
                "this_node_id": "ood1",
                "this_zone_host": "test.zone"
            },
            "app_info": {},
            "service_info": {},
            "node_route_map": {},
            "routes": {},
            "cluster_route_map": {},
            "trust_key": {},
        });
        let boot_gateway_info = json!({
            "cluster_route_map": {
                "klog-service": {
                    "route_prefix": KLOG_CLUSTER_ROUTE_PREFIX,
                    "ingress_port": DEFAULT_NODE_GATEWAY_HTTP_PORT,
                    "nodes": {
                        "ood1": {"ports": {"admin": KLOG_CLUSTER_ADMIN_PORT}},
                        "ood2": {"ports": {"admin": KLOG_CLUSTER_ADMIN_PORT}}
                    }
                }
            },
            "node_route_map": {
                "ood2": "tcp:///ood2.test.zone"
            },
            "routes": {
                "ood2": [
                    {
                        "id": "direct",
                        "kind": "tcp_direct",
                        "url": "tcp:///ood2.test.zone",
                        "priority": 10,
                        "weight": 100,
                        "backup": false,
                        "keep_tunnel": false,
                        "source": "zone_boot_config"
                    }
                ]
            }
        });

        let merged = merge_missing_boot_klog_gateway_info(runtime_gateway_info, &boot_gateway_info);

        assert_eq!(
            merged["cluster_route_map"][KLOG_SERVICE_UNIQUE_ID]["nodes"]["ood2"]["ports"]["admin"],
            KLOG_CLUSTER_ADMIN_PORT
        );
        assert_eq!(merged["node_route_map"]["ood2"], "tcp:///ood2.test.zone");
        assert_eq!(merged["routes"]["ood2"][0]["kind"], "tcp_direct");
    }

    #[test]
    fn merge_missing_boot_klog_gateway_info_preserves_boot_identity() {
        let runtime_gateway_info = json!({
            "cluster_route_map": {},
            "node_route_map": {},
            "routes": {},
        });
        let boot_gateway_info = json!({
            "node_info": {
                "this_node_id": "ood2",
                "this_zone_host": "test.zone"
            },
            "service_info": {
                "system_config": {
                    "selector": {
                        "ood1": {"port": SYSTEM_CONFIG_PORT, "weight": 100},
                        "ood2": {"port": SYSTEM_CONFIG_PORT, "weight": 100}
                    }
                }
            },
            "cluster_route_map": {
                "klog-service": {
                    "route_prefix": KLOG_CLUSTER_ROUTE_PREFIX,
                    "ingress_port": DEFAULT_NODE_GATEWAY_HTTP_PORT,
                    "nodes": {
                        "ood1": {"ports": {"inter": KLOG_CLUSTER_INTER_PORT}},
                        "ood2": {"ports": {"inter": KLOG_CLUSTER_INTER_PORT}}
                    }
                }
            }
        });

        let merged = merge_missing_boot_klog_gateway_info(runtime_gateway_info, &boot_gateway_info);

        assert_eq!(merged["node_info"]["this_node_id"], "ood2");
        assert_eq!(merged["node_info"]["this_zone_host"], "test.zone");
        assert_eq!(
            merged["service_info"]["system_config"]["selector"]["ood1"]["port"],
            SYSTEM_CONFIG_PORT
        );
    }

    #[test]
    fn dedup_keep_tunnel_targets_keeps_order() {
        let mut targets = vec![
            "rtcp://ood2.zone/".to_string(),
            "".to_string(),
            "ood2.zone".to_string(),
            "rtcp://ood3.zone:2981/".to_string(),
        ];

        dedup_keep_tunnel_targets(&mut targets);

        assert_eq!(
            targets,
            vec!["ood2.zone".to_string(), "ood3.zone:2981".to_string()]
        );
    }

    #[test]
    fn build_boot_node_gateway_info_adds_klog_cluster_route() {
        let zone_boot_config = ZoneBootConfig {
            id: None,
            oods: vec!["ood1".parse().unwrap(), "$ood2".parse().unwrap()],
            sn: None,
            exp: 0,
            owner: None,
            owner_key: None,
            extra_info: HashMap::new(),
        };

        let gateway_info = build_boot_node_gateway_info(
            "ood1",
            "test.zone",
            &zone_boot_config,
            &HashMap::new(),
            None,
        );
        let route = &gateway_info["cluster_route_map"][KLOG_SERVICE_UNIQUE_ID];

        assert_eq!(route["route_prefix"], KLOG_CLUSTER_ROUTE_PREFIX);
        assert_eq!(route["ingress_port"], DEFAULT_NODE_GATEWAY_HTTP_PORT);
        assert_eq!(
            route["nodes"]["ood1"]["ports"][KLOG_CLUSTER_ADMIN_SERVICE_NAME],
            KLOG_CLUSTER_ADMIN_PORT
        );
        assert_eq!(
            route["nodes"]["ood2"]["ports"][KLOG_CLUSTER_RAFT_SERVICE_NAME],
            KLOG_CLUSTER_RAFT_PORT
        );
    }

    #[test]
    fn build_boot_node_gateway_info_defaults_to_rtcp_direct_for_lan_ood() {
        let zone_boot_config = ZoneBootConfig {
            id: None,
            oods: vec!["ood1".parse().unwrap(), "$ood2".parse().unwrap()],
            sn: None,
            exp: 0,
            owner: None,
            owner_key: None,
            extra_info: HashMap::new(),
        };
        let mut discovered_oods = HashMap::new();
        discovered_oods.insert(
            "ood2".to_string(),
            DiscoveredNode {
                node_id: "ood2".to_string(),
                device_doc: DeviceConfig::new("ood2", "test_public_key".to_string()),
                device_doc_jwt: "test_device_doc_jwt".to_string(),
                addr: "192.168.64.22:2981".parse().unwrap(),
                rtcp_port: 2981,
                last_seen: 42,
                from_cache: false,
            },
        );

        let gateway_info = build_boot_node_gateway_info_inner(
            "ood1",
            "test.zone",
            &zone_boot_config,
            &discovered_oods,
            None,
            false,
        );
        let direct_route = &gateway_info["routes"]["ood2"][0];

        assert_eq!(
            gateway_info["node_route_map"]["ood2"],
            "rtcp://test_public_key.dev.did:2981/"
        );
        assert_eq!(direct_route["kind"], "rtcp_direct");
        assert_eq!(direct_route["url"], "rtcp://test_public_key.dev.did:2981/");
        assert_eq!(direct_route["keep_tunnel"], true);
        assert_eq!(
            gateway_info["did_ip_hints"]["test_public_key.dev.did"][0]["ip"],
            "192.168.64.22"
        );
        assert_eq!(
            gateway_info["did_ip_hints"]["test_public_key.dev.did"][0]["port"],
            2981
        );
    }

    #[test]
    fn build_boot_node_gateway_info_uses_tcp_direct_for_lan_ood() {
        let zone_boot_config = ZoneBootConfig {
            id: None,
            oods: vec!["ood1".parse().unwrap(), "$ood2".parse().unwrap()],
            sn: None,
            exp: 0,
            owner: None,
            owner_key: None,
            extra_info: HashMap::new(),
        };
        let mut discovered_oods = HashMap::new();
        discovered_oods.insert(
            "ood2".to_string(),
            DiscoveredNode {
                node_id: "ood2".to_string(),
                device_doc: DeviceConfig::new("ood2", "test_public_key".to_string()),
                device_doc_jwt: "test_device_doc_jwt".to_string(),
                addr: "192.168.64.22:2980".parse().unwrap(),
                rtcp_port: DEFAULT_RTCP_PORT as u16,
                last_seen: 42,
                from_cache: false,
            },
        );

        let gateway_info = build_boot_node_gateway_info_inner(
            "ood1",
            "test.zone",
            &zone_boot_config,
            &discovered_oods,
            None,
            true,
        );
        let direct_route = &gateway_info["routes"]["ood2"][0];

        assert_eq!(
            gateway_info["node_route_map"]["ood2"],
            "tcp:///ood2.test.zone"
        );
        assert_eq!(direct_route["kind"], "tcp_direct");
        assert_eq!(direct_route["url"], "tcp:///ood2.test.zone");
        assert_eq!(direct_route["keep_tunnel"], false);
    }

    #[test]
    fn build_keep_tunnel_targets_defaults_to_rtcp_and_dev_tcp_direct_skips_ood_rtcp() {
        let zone_boot_config = ZoneBootConfig {
            id: None,
            oods: vec!["ood1".parse().unwrap(), "$ood2".parse().unwrap()],
            sn: None,
            exp: 0,
            owner: None,
            owner_key: None,
            extra_info: HashMap::new(),
        };
        let device_doc = DeviceConfig::new("ood1", "test_public_key".to_string());

        let default_targets = build_keep_tunnel_targets_inner(
            NodeRole::Ood,
            &device_doc,
            &zone_boot_config,
            &HashMap::new(),
            "test.zone",
            None,
            false,
        );
        assert_eq!(default_targets, vec!["ood2.test.zone".to_string()]);

        let dev_tcp_targets = build_keep_tunnel_targets_inner(
            NodeRole::Ood,
            &device_doc,
            &zone_boot_config,
            &HashMap::new(),
            "test.zone",
            None,
            true,
        );
        assert!(dev_tcp_targets.is_empty());
    }
}
