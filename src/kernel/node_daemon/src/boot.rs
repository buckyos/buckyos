// Boot 阶段的角色识别、LAN OOD 发现、boot route 生成与 system-config 入口选择。
//
// 设计参考 doc/arch/gateway/boot_gateway的配置生成.md `角色启动流程` 章节。
// 三个角色（OOD / 非 OOD ZoneGateway / 普通 Node）流程虽不同，但下面的工具
// 都可被复用，差异只在调用顺序与目标。

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use jsonwebtoken::{DecodingKey, EncodingKey};
use log::*;
use name_lib::{DeviceDocument, ZoneDocument};
use serde_json::{json, Value};

use crate::finder::{DiscoveredNode, NodeFinderClient};
use crate::sn_zone_info::device_requires_sn_keep_tunnel;

const DEFAULT_RTCP_PORT: u32 = 2980;
const SYSTEM_CONFIG_PORT: u16 = 3200;
pub const CROSS_ZONE_SOCKS_FILE: &str = "cross_zone_socks.json";
pub const CROSS_ZONE_SOCKS_PARAMS_FILE: &str = "cross_zone_socks_params.json";
const CROSS_ZONE_SOCKS_USERNAME: &str = "buckyos";
const FINDER_DISCOVERY_TIMEOUT_SECS: u64 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeRole {
    Ood,
    ZoneGateway,
    Node,
}

impl NodeRole {
    pub fn from_zone_document(zone_document: &ZoneDocument, device_name: &str) -> Self {
        let device_node_type = zone_document
            .oods
            .iter()
            .find(|ood| ood.name == device_name)
            .map(|ood| &ood.node_type);
        if device_node_type
            .map(|node_type| node_type.is_ood())
            .unwrap_or(false)
        {
            NodeRole::Ood
        } else if device_node_type
            .map(|node_type| node_type.is_gateway())
            .unwrap_or(false)
        {
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
    zone_document: ZoneDocument,
    owner_public_key: DecodingKey,
    role: NodeRole,
) -> HashMap<String, DiscoveredNode> {
    // OOD 自己作为 server 同时也是合法 client，使用强校验路径；其它角色用宽松版本
    // 以绕过"自身必须是 OOD"的检查。
    let client_result = match role {
        NodeRole::Ood => NodeFinderClient::new_for_zone(
            this_device_jwt,
            device_private_key,
            zone_document,
            owner_public_key,
        ),
        NodeRole::ZoneGateway | NodeRole::Node => NodeFinderClient::new_as_lan_client(
            this_device_jwt,
            device_private_key,
            zone_document,
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
// 目的：让 cyfs-gateway 在 scheduler 还没产出正式 routes 时也能用 RTCP 直连/relay
// 转发 `127.0.0.1:3180/kapi/system_config` 到 OOD。
//
// 写入字段：
//  - node_info.this_node_id / this_zone_host
//  - service_info.system_config 的 selector 指向所有 OOD
//  - node_route_map 提供 OOD/ZoneGateway 的 RTCP URL（兼容现有 boot_gateway.yaml）
//  - routes 为新格式（per doc 设计）的 direct + via-sn 候选
//  - app_info / trust_key 留空，等 scheduler 接管
pub fn build_boot_node_gateway_info(
    this_node_id: &str,
    zone_host: &str,
    zone_document: &ZoneDocument,
    discovered_oods: &HashMap<String, DiscoveredNode>,
    sn_host_name: Option<&str>,
) -> Value {
    let oods_in_zone: Vec<&str> = zone_document
        .oods
        .iter()
        .filter(|ood| ood.node_type.is_ood() && ood.name != this_node_id)
        .map(|ood| ood.name.as_str())
        .collect();

    let mut node_route_map: HashMap<String, String> = HashMap::new();
    let mut routes: HashMap<String, Vec<Value>> = HashMap::new();

    for ood_name in oods_in_zone.iter() {
        let port = discovered_oods
            .get(*ood_name)
            .map(|node| node.rtcp_port as u32)
            .unwrap_or(DEFAULT_RTCP_PORT);
        let direct_url = format_rtcp_did_url(ood_name, zone_host, port);
        node_route_map.insert((*ood_name).to_string(), direct_url.clone());

        let mut candidates = vec![build_route_candidate(
            "direct",
            "rtcp_direct",
            10,
            false,
            true,
            &direct_url,
            "zone_document",
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
                "zone_document",
                Some(sn),
                None,
            ));
        }

        routes.insert((*ood_name).to_string(), candidates);
    }

    // 非 OOD ZoneGateway 也是 boot 阶段需要 keep_tunnel 的目标之一
    for ood in zone_document.oods.iter() {
        if ood.name == this_node_id {
            continue;
        }
        if ood.node_type.is_ood() {
            continue;
        }
        if !ood.node_type.is_gateway() {
            continue;
        }
        let direct_url = format_rtcp_did_url(ood.name.as_str(), zone_host, DEFAULT_RTCP_PORT);
        node_route_map
            .entry(ood.name.clone())
            .or_insert_with(|| direct_url.clone());
        routes.entry(ood.name.clone()).or_insert_with(|| {
            vec![build_route_candidate(
                "direct",
                "rtcp_direct",
                10,
                false,
                true,
                &direct_url,
                "zone_document",
                None,
                None,
            )]
        });
    }

    // service_info.system_config 让 boot_gateway.yaml 的 forward_to_service 能命中
    // OOD 上的 system_config 服务。selector 指向所有 OOD；本节点是 OOD 时，
    // forward_to_service 会先检测 THIS_NODE_ID 命中，走本机 127.0.0.1。
    let mut sysconfig_selector = serde_json::Map::new();
    for ood in zone_document.oods.iter() {
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

    let cross_zone_relay = match zone_document.get_default_zone_gateway() {
        Some(gateway) if gateway != this_node_id => {
            let port = discovered_oods
                .get(gateway.as_str())
                .map(|node| node.rtcp_port as u32)
                .unwrap_or(DEFAULT_RTCP_PORT);
            format_rtcp_did_url(gateway.as_str(), zone_host, port)
        }
        _ => String::new(),
    };

    json!({
        "node_info": {
            "this_node_id": this_node_id,
            "this_zone_host": zone_host,
            "cross_zone_relay": cross_zone_relay,
        },
        "app_info": {},
        "service_info": service_info,
        "node_route_map": node_route_map,
        "routes": routes,
        "trust_key": {},
    })
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
    if port == DEFAULT_RTCP_PORT {
        format!("rtcp://{}.{}/", node_id, zone_host)
    } else {
        format!("rtcp://{}.{}:{}/", node_id, zone_host, port)
    }
}

fn format_relay_rtcp_url(
    sn_host: &str,
    target_node_id: &str,
    zone_host: &str,
    target_port: u32,
) -> String {
    let bootstrap_url = format!(
        "rtcp://{}/{}.{}:{}",
        sn_host, target_node_id, zone_host, target_port
    );
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

// 启动 cyfs-gateway 前写入 node_rtcp.keep_tunnel 的目标。
// SN：本机非 wan 系时，需要它 keep tunnel 解决"被动可达"。
// 其它 OOD：作为 RTCP direct 的 keep_tunnel 目标。
pub fn build_keep_tunnel_targets(
    role: NodeRole,
    device_doc: &DeviceDocument,
    zone_document: &ZoneDocument,
    zone_host: &str,
    sn_host_name: Option<&str>,
) -> Vec<String> {
    let mut targets: Vec<String> = Vec::new();

    if device_requires_sn_keep_tunnel(device_doc) {
        if let Some(sn) = sn_host_name {
            targets.push(sn.to_string());
        }
    }

    // boot 阶段对其它节点的 rtcp_port 没有可信来源，统一用默认 2980；
    // scheduler 接管后可生成包含真实端口的 routes。
    match role {
        NodeRole::Ood | NodeRole::ZoneGateway => {
            for ood in zone_document.oods.iter() {
                if ood.name == device_doc.name {
                    continue;
                }
                targets.push(format_rtcp_did_url(
                    ood.name.as_str(),
                    zone_host,
                    DEFAULT_RTCP_PORT,
                ));
            }
        }
        NodeRole::Node => {
            // 普通 Node：与至少 1 个、最多 2 个 OOD 维持 keep_tunnel，
            // 让"ZoneGateway 失效时也能走 OOD"。
            for ood in zone_document
                .oods
                .iter()
                .filter(|ood| ood.node_type.is_ood())
                .take(2)
            {
                targets.push(format_rtcp_did_url(
                    ood.name.as_str(),
                    zone_host,
                    DEFAULT_RTCP_PORT,
                ));
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
            targets.push(url.to_string());
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
        if target.trim().is_empty() {
            continue;
        }
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

/// 本节点访问自己的 Zone resolver 要经过的 RTCP 主机：本机 3180 的 `/1.0/identifiers`
/// 按 ROUTES 转发到跑 system_config 的节点，路由、NODE_ROUTE_MAP 与 keep-tunnel 目标里
/// 出现的目标主机和中继主机都在这条路上。cyfs-gateway 解析它们的身份时必须跳过 Zone
/// locker：否则解析要经 3180 → RTCP 转发，而转发又要先解析它们，形成回环（buckyos#710）。
pub fn zone_locker_bypass_hosts(gateway_info: Option<&Value>, keep_tunnels: &[String]) -> Vec<String> {
    let mut targets: Vec<&str> = Vec::new();
    if let Some(gateway_info) = gateway_info {
        if let Some(routes) = gateway_info.get("routes").and_then(Value::as_object) {
            for candidates in routes.values() {
                for candidate in candidates.as_array().into_iter().flatten() {
                    if let Some(url) = candidate.get("url").and_then(Value::as_str) {
                        targets.push(url);
                    }
                }
            }
        }
        if let Some(route_map) = gateway_info.get("node_route_map").and_then(Value::as_object) {
            targets.extend(route_map.values().filter_map(Value::as_str));
        }
    }
    targets.extend(keep_tunnels.iter().map(String::as_str));

    let mut hosts = Vec::new();
    for target in targets {
        collect_rtcp_target_hosts(target, &mut hosts);
    }
    // 固定顺序：名单进 node_gateway.json，顺序变化不应触发网关 reload。
    hosts.sort();
    hosts.dedup();
    hosts
}

/// `rtcp://[<编码后的引导 URL>@]<host>[:port]/...` 或裸主机名：收集目标主机，并递归展开
/// 引导 URL（经 SN / ZoneGateway 中继的路由）。
fn collect_rtcp_target_hosts(target: &str, hosts: &mut Vec<String>) {
    let target = target.trim();
    let authority = target
        .strip_prefix("rtcp://")
        .unwrap_or(target)
        .split('/')
        .next()
        .unwrap_or_default();
    let host_port = match authority.rsplit_once('@') {
        Some((bootstrap, host_port)) => {
            if let Some((bootstrap_url, _)) = url::form_urlencoded::parse(bootstrap.as_bytes()).next() {
                collect_rtcp_target_hosts(bootstrap_url.as_ref(), hosts);
            }
            host_port
        }
        None => authority,
    };
    let host = match host_port.rsplit_once(':') {
        Some((host, port)) if port.chars().all(|c| c.is_ascii_digit()) => host,
        _ => host_port,
    };
    if !host.is_empty() {
        hosts.push(host.to_string());
    }
}

/// 写入 cyfs-gateway 顶层 `name_client.zone_locker_bypass`。
pub fn merge_zone_locker_bypass_into_gateway_config(
    mut gateway_config: Value,
    hosts: &[String],
) -> Value {
    if !gateway_config.is_object() {
        gateway_config = json!({});
    }
    if gateway_config
        .get("name_client")
        .and_then(Value::as_object)
        .is_none()
    {
        gateway_config["name_client"] = json!({});
    }
    gateway_config["name_client"]["zone_locker_bypass"] = json!(hosts);
    gateway_config
}

pub fn write_boot_node_gateway_config(
    keep_tunnels: &[String],
    zone_locker_bypass: &[String],
) -> std::io::Result<()> {
    let path = buckyos_kit::get_buckyos_system_etc_dir().join("node_gateway.json");
    let existing_config = std::fs::read_to_string(&path)
        .ok()
        .and_then(|content| serde_json::from_str::<Value>(content.as_str()).ok())
        .unwrap_or_else(|| json!({}));
    let content = merge_keep_tunnel_into_gateway_config(existing_config, keep_tunnels);
    let content = merge_zone_locker_bypass_into_gateway_config(content, zone_locker_bypass);
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

/// Credentials of the loopback `cross_zone_entry` SOCKS5 exit: services read
/// `cross_zone_socks.json`, the gateway includes the same values as params.
/// Existing credentials are kept so running clients stay valid.
pub fn write_cross_zone_socks_credentials(etc_dir: &Path) -> std::io::Result<()> {
    let path = etc_dir.join(CROSS_ZONE_SOCKS_FILE);
    let existing = std::fs::read(&path)
        .ok()
        .and_then(|content| serde_json::from_slice::<Value>(&content).ok())
        .and_then(|value| {
            let username = value.get("username")?.as_str()?.to_string();
            let password = value.get("password")?.as_str()?.to_string();
            (!username.is_empty() && !password.is_empty()).then_some((username, password))
        });
    let (username, password) = existing.unwrap_or_else(|| {
        (
            CROSS_ZONE_SOCKS_USERNAME.to_string(),
            hex::encode(rand::random::<[u8; 32]>()),
        )
    });
    write_private_json(
        &path,
        &json!({ "username": username, "password": password }),
    )?;
    write_private_json(
        &etc_dir.join(CROSS_ZONE_SOCKS_PARAMS_FILE),
        &json!({
            "params": {
                "cross_zone_socks_username": username,
                "cross_zone_socks_password": password,
            }
        }),
    )
}

fn write_private_json(path: &Path, value: &Value) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address_test_device_and_zone() -> (DeviceDocument, ZoneDocument) {
        use name_lib::DIDDocumentTrait;

        let mut device = DeviceDocument::new("ood1", "test-key".to_string());
        device.net_id = Some("wan_dyn".to_string());
        let zone = ZoneDocument::new(
            name_lib::DID::new("bns", "test-zone"),
            name_lib::DID::new("bns", "test-owner"),
            device.get_default_key().unwrap(),
        );
        (device, zone)
    }

    fn encode_bootstrap(url: &str) -> String {
        url::form_urlencoded::byte_serialize(url.as_bytes()).collect()
    }

    #[test]
    fn zone_locker_bypass_covers_every_host_on_the_route_paths() {
        let via_sn = format!(
            "rtcp://{}@ood2.zone.example/",
            encode_bootstrap("rtcp://sn.example.com/")
        );
        // 经 ZoneGateway 的路由，ZoneGateway 自身又经 SN 中继。
        let via_gateway = format!(
            "rtcp://{}@ood3.zone.example:2981/",
            encode_bootstrap(
                format!(
                    "rtcp://{}@gw.zone.example/",
                    encode_bootstrap("rtcp://sn.example.com/")
                )
                .as_str()
            )
        );
        let gateway_info = json!({
            "routes": {
                "ood2": [
                    {"url": "rtcp://ood2.zone.example:2981/", "keep_tunnel": true},
                    {"url": via_sn, "keep_tunnel": false}
                ],
                "ood3": [{"url": via_gateway}]
            },
            "node_route_map": {"ood4": "rtcp://ood4.zone.example/"}
        });

        let hosts = zone_locker_bypass_hosts(
            Some(&gateway_info),
            &["relay.example.net".to_string()],
        );
        assert_eq!(
            hosts,
            vec![
                "gw.zone.example",
                "ood2.zone.example",
                "ood3.zone.example",
                "ood4.zone.example",
                "relay.example.net",
                "sn.example.com",
            ]
        );
        let config = merge_zone_locker_bypass_into_gateway_config(json!({}), &hosts);
        assert_eq!(config["name_client"]["zone_locker_bypass"], json!(hosts));
        assert_eq!(zone_locker_bypass_hosts(None, &[]), Vec::<String>::new());
    }

    #[test]
    fn boot_routes_feed_the_zone_locker_bypass() {
        let (_, mut zone) = address_test_device_and_zone();
        zone.oods = vec!["ood1".parse().unwrap(), "ood2".parse().unwrap()];
        let info = build_boot_node_gateway_info(
            "node1",
            "test-zone.example",
            &zone,
            &HashMap::new(),
            Some("sn.example.com"),
        );
        assert_eq!(
            zone_locker_bypass_hosts(Some(&info), &[]),
            vec![
                "ood1.test-zone.example",
                "ood2.test-zone.example",
                "sn.example.com",
            ]
        );
    }

    fn decode_bootstrap(route_url: &str) -> (String, String) {
        let authority = route_url
            .strip_prefix("rtcp://")
            .unwrap()
            .split('/')
            .next()
            .unwrap();
        let (bootstrap, remote) = authority.rsplit_once('@').unwrap();
        let (bootstrap, _) = url::form_urlencoded::parse(bootstrap.as_bytes())
            .next()
            .unwrap();
        (bootstrap.into_owned(), remote.to_string())
    }

    // P1-6: the relay hop names its target, otherwise the relay is asked to
    // open a stream with an empty destination.
    #[test]
    fn via_sn_route_carries_the_relay_target() {
        let (_, mut zone) = address_test_device_and_zone();
        zone.oods = vec!["ood1".parse().unwrap(), "ood2".parse().unwrap()];
        let info = build_boot_node_gateway_info(
            "ood1",
            "test-zone.example",
            &zone,
            &HashMap::new(),
            Some("sn.example.com"),
        );
        let via_sn = info["routes"]["ood2"]
            .as_array()
            .unwrap()
            .iter()
            .find(|candidate| candidate["id"] == "via-sn")
            .unwrap();
        let (bootstrap, remote) = decode_bootstrap(via_sn["url"].as_str().unwrap());
        assert_eq!(
            bootstrap,
            "rtcp://sn.example.com/ood2.test-zone.example:2980"
        );
        assert_eq!(remote, "ood2.test-zone.example");
        let bootstrap = url::Url::parse(&bootstrap).unwrap();
        assert_eq!(bootstrap.host_str(), Some("sn.example.com"));
        assert_eq!(bootstrap.path(), "/ood2.test-zone.example:2980");
    }

    // P1-9: nodes other than the default Zone gateway leave the Zone through it.
    #[test]
    fn cross_zone_relay_points_at_the_default_zone_gateway() {
        let (_, mut zone) = address_test_device_and_zone();
        zone.oods = vec!["ood1".parse().unwrap(), "ood2".parse().unwrap()];
        let relay = |node: &str| {
            build_boot_node_gateway_info(node, "test-zone.example", &zone, &HashMap::new(), None)
                ["node_info"]["cross_zone_relay"]
                .clone()
        };
        assert_eq!(relay("node1"), json!("rtcp://ood1.test-zone.example/"));
        assert_eq!(relay("ood2"), json!("rtcp://ood1.test-zone.example/"));
        assert_eq!(relay("ood1"), json!(""));
    }

    #[test]
    fn cross_zone_socks_credentials_are_private_and_stable() {
        let dir = tempfile::tempdir().unwrap();
        write_cross_zone_socks_credentials(dir.path()).unwrap();
        let read = |name: &str| -> Value {
            serde_json::from_slice(&std::fs::read(dir.path().join(name)).unwrap()).unwrap()
        };
        let credentials = read(CROSS_ZONE_SOCKS_FILE);
        let password = credentials["password"].as_str().unwrap().to_string();
        assert_eq!(credentials["username"], "buckyos");
        assert_eq!(password.len(), 64);
        assert_eq!(
            read(CROSS_ZONE_SOCKS_PARAMS_FILE),
            json!({"params": {
                "cross_zone_socks_username": "buckyos",
                "cross_zone_socks_password": password,
            }})
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for name in [CROSS_ZONE_SOCKS_FILE, CROSS_ZONE_SOCKS_PARAMS_FILE] {
                let mode = std::fs::metadata(dir.path().join(name))
                    .unwrap()
                    .permissions()
                    .mode();
                assert_eq!(mode & 0o777, 0o600, "{name}");
            }
        }

        write_cross_zone_socks_credentials(dir.path()).unwrap();
        assert_eq!(read(CROSS_ZONE_SOCKS_FILE)["password"], json!(password));

        std::fs::write(dir.path().join(CROSS_ZONE_SOCKS_FILE), b"{}").unwrap();
        write_cross_zone_socks_credentials(dir.path()).unwrap();
        let regenerated = read(CROSS_ZONE_SOCKS_FILE)["password"].clone();
        assert_ne!(regenerated, json!(password));
        assert_eq!(
            read(CROSS_ZONE_SOCKS_PARAMS_FILE)["params"]["cross_zone_socks_password"],
            regenerated
        );
    }

    #[test]
    fn addressless_wan_bootstrap_keeps_sn_for_every_node_role() {
        let (device, zone) = address_test_device_and_zone();
        for role in [NodeRole::Ood, NodeRole::ZoneGateway, NodeRole::Node] {
            let targets = build_keep_tunnel_targets(
                role,
                &device,
                &zone,
                "test-zone.example",
                Some("sn.example.com"),
            );
            assert_eq!(targets, vec!["sn.example.com".to_string()]);
            let config = merge_keep_tunnel_into_gateway_config(json!({}), &targets);
            assert_eq!(
                config["stacks"]["node_rtcp"]["keep_tunnel"],
                json!(["sn.example.com"])
            );
        }
    }

    #[test]
    fn fixed_wan_bootstrap_does_not_add_sn() {
        let (mut device, zone) = address_test_device_and_zone();
        device.ips = vec!["203.0.113.10".parse().unwrap()];
        for role in [NodeRole::Ood, NodeRole::ZoneGateway, NodeRole::Node] {
            assert!(build_keep_tunnel_targets(
                role,
                &device,
                &zone,
                "test-zone.example",
                Some("sn.example.com"),
            )
            .is_empty());
        }
    }

    #[test]
    fn addressless_wan_bootstrap_does_not_invent_an_sn() {
        let (device, zone) = address_test_device_and_zone();
        assert!(build_keep_tunnel_targets(
            NodeRole::Ood,
            &device,
            &zone,
            "test-zone.example",
            None,
        )
        .is_empty());
    }

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
            &[
                "rtcp://ood2.zone/".to_string(),
                "rtcp://ood3.zone/".to_string(),
            ],
        );

        assert_eq!(merged["acme"]["enabled"], true);
        assert_eq!(merged["stacks"]["zone_tls"]["protocol"], "tls");
        assert_eq!(
            merged["stacks"]["node_rtcp"]["keep_tunnel"],
            json!(["rtcp://ood2.zone/", "rtcp://ood3.zone/"])
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
            vec![
                "rtcp://ood2.zone/".to_string(),
                "rtcp://ood3.zone/".to_string()
            ]
        );
    }

    #[test]
    fn dedup_keep_tunnel_targets_keeps_order() {
        let mut targets = vec![
            "rtcp://ood2.zone/".to_string(),
            "".to_string(),
            "rtcp://ood2.zone/".to_string(),
            "rtcp://ood3.zone/".to_string(),
        ];

        dedup_keep_tunnel_targets(&mut targets);

        assert_eq!(
            targets,
            vec![
                "rtcp://ood2.zone/".to_string(),
                "rtcp://ood3.zone/".to_string()
            ]
        );
    }
}
