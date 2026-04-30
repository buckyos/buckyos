// Boot 阶段的角色识别、LAN OOD 发现、boot route 生成与 system-config 入口选择。
//
// 设计参考 doc/arch/gateway/boot_gateway的配置生成.md `角色启动流程` 章节。
// 三个角色（OOD / 非 OOD ZoneGateway / 普通 Node）流程虽不同，但下面的工具
// 都可被复用，差异只在调用顺序与目标。

use std::collections::HashMap;

use jsonwebtoken::{DecodingKey, EncodingKey};
use log::*;
use name_lib::{DeviceConfig, ZoneBootConfig};
use serde_json::{json, Value};

use crate::finder::{DiscoveredNode, NodeFinderClient};

const DEFAULT_RTCP_PORT: u32 = 2980;
const SYSTEM_CONFIG_PORT: u16 = 3200;
const FINDER_DISCOVERY_TIMEOUT_SECS: u64 = 3;

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
    zone_boot_config: &ZoneBootConfig,
    discovered_oods: &HashMap<String, DiscoveredNode>,
    sn_host_name: Option<&str>,
) -> Value {
    let oods_in_zone: Vec<&str> = zone_boot_config
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

    json!({
        "node_info": {
            "this_node_id": this_node_id,
            "this_zone_host": zone_host,
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

// 启动 cyfs-gateway 前写入 node_rtcp.keep_tunnel 的目标。
// SN：本机非 wan 系时，需要它 keep tunnel 解决"被动可达"。
// 其它 OOD：作为 RTCP direct 的 keep_tunnel 目标。
pub fn build_keep_tunnel_targets(
    role: NodeRole,
    device_doc: &DeviceConfig,
    zone_boot_config: &ZoneBootConfig,
    zone_host: &str,
    sn_host_name: Option<&str>,
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
    match role {
        NodeRole::Ood | NodeRole::ZoneGateway => {
            for ood in zone_boot_config.oods.iter() {
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
            for ood in zone_boot_config
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
