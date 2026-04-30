use std::collections::HashMap;

use name_lib::{DeviceInfo, ZoneConfig};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const DEFAULT_RTCP_PORT: u32 = 2980;
const DEFAULT_ROUTE_WEIGHT: u32 = 100;

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub(crate) struct NetworkObservation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) observation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) changed_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) observed_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) rtcp_port: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) ipv6: Option<Ipv6Observation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) endpoints: Vec<NetworkEndpoint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) direct_probe: Vec<ProbeInfo>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub(crate) struct Ipv6Observation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) probe_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) last_probe: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) failure_reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub(crate) struct NetworkEndpoint {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) ip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) observed_at: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub(crate) struct ProbeInfo {
    #[serde(
        default,
        alias = "target_node_id",
        skip_serializing_if = "Option::is_none"
    )]
    pub(crate) target_node: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) rtt_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) last_probe: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) last_success: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) freshness_ttl_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) failure_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) source: Option<String>,
}

impl ProbeInfo {
    fn target_node_id(&self) -> Option<&str> {
        self.target_node.as_deref()
    }

    fn is_reachable(&self) -> bool {
        self.status
            .as_deref()
            .map(|status| status.eq_ignore_ascii_case("reachable"))
            .unwrap_or(false)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct NodeGatewayRouteEvidence {
    #[serde(rename = "type")]
    pub(crate) evidence_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) source_node: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) target_node: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) witness_node: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_probe: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_success: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) rtt_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) freshness_ttl_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) failure_reason: Option<String>,
    pub(crate) confidence: String,
    pub(crate) applicability: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct NodeGatewayRouteCandidate {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) priority: u32,
    pub(crate) weight: u32,
    pub(crate) backup: bool,
    pub(crate) keep_tunnel: bool,
    pub(crate) url: String,
    pub(crate) source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) relay_node: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) evidence: Option<NodeGatewayRouteEvidence>,
}

fn device_rtcp_port(device_info: &DeviceInfo) -> u32 {
    device_info
        .device_doc
        .rtcp_port
        .unwrap_or(DEFAULT_RTCP_PORT)
}

fn format_rtcp_did_url(node_id: &str, zone_host: &str, port: u32) -> String {
    if port == DEFAULT_RTCP_PORT {
        format!("rtcp://{}.{}/", node_id, zone_host)
    } else {
        format!("rtcp://{}.{}:{}/", node_id, zone_host, port)
    }
}

fn format_rtcp_ip_url(ip: &std::net::IpAddr, port: u32) -> String {
    let host = match ip {
        std::net::IpAddr::V4(ipv4) => ipv4.to_string(),
        std::net::IpAddr::V6(ipv6) => format!("[{}]", ipv6),
    };

    if port == DEFAULT_RTCP_PORT {
        format!("rtcp://{}/", host)
    } else {
        format!("rtcp://{}:{}/", host, port)
    }
}

fn format_relay_rtcp_url(
    relay_url: &str,
    target_node_id: &str,
    zone_host: &str,
    port: u32,
) -> String {
    let encoded_relay_url: String =
        url::form_urlencoded::byte_serialize(relay_url.as_bytes()).collect();
    if port == DEFAULT_RTCP_PORT {
        format!(
            "rtcp://{}@{}.{}/",
            encoded_relay_url, target_node_id, zone_host
        )
    } else {
        format!(
            "rtcp://{}@{}.{}:{}/",
            encoded_relay_url, target_node_id, zone_host, port
        )
    }
}

fn is_wan_net_id(net_id: Option<&String>) -> bool {
    net_id
        .map(|net_id| net_id.starts_with("wan"))
        .unwrap_or(false)
}

fn is_publicly_reachable_net_id(net_id: Option<&String>) -> bool {
    let Some(net_id) = net_id else {
        return false;
    };
    net_id.starts_with("wan") || net_id == "portmap"
}

fn net_id_for_planning(device_info: &DeviceInfo) -> &str {
    device_info
        .device_doc
        .net_id
        .as_deref()
        .unwrap_or("unknown_lan")
}

fn has_signed_wan_ip(device_info: &DeviceInfo) -> bool {
    is_wan_net_id(device_info.device_doc.net_id.as_ref()) && !device_info.device_doc.ips.is_empty()
}

fn same_trusted_lan(source: &DeviceInfo, target: &DeviceInfo) -> bool {
    if is_wan_net_id(source.device_doc.net_id.as_ref())
        || is_wan_net_id(target.device_doc.net_id.as_ref())
    {
        return false;
    }

    net_id_for_planning(source) == net_id_for_planning(target)
}

fn direct_probe_targets(device_info: &DeviceInfo) -> HashMap<String, ProbeInfo> {
    let mut result = HashMap::new();

    if let Some(network_observation) = device_info.device_doc.extra_info.get("network_observation")
    {
        if let Ok(network_observation) =
            serde_json::from_value::<NetworkObservation>(network_observation.clone())
        {
            collect_reachable_probe_targets(&network_observation.direct_probe, &mut result);
        } else if let Some(direct_probe) = network_observation
            .get("direct_probe")
            .and_then(Value::as_array)
        {
            collect_reachable_probe_target_values(direct_probe, &mut result);
        }
    }

    if let Some(direct_probe) = device_info
        .device_doc
        .extra_info
        .get("direct_probe")
        .and_then(Value::as_array)
    {
        collect_reachable_probe_target_values(direct_probe, &mut result);
    }

    result
}

fn collect_reachable_probe_target_values(
    direct_probe: &[Value],
    result: &mut HashMap<String, ProbeInfo>,
) {
    for probe in direct_probe.iter() {
        let Ok(probe_info) = serde_json::from_value::<ProbeInfo>(probe.clone()) else {
            continue;
        };
        collect_reachable_probe_target(probe_info, result);
    }
}

fn collect_reachable_probe_targets(
    direct_probe: &[ProbeInfo],
    result: &mut HashMap<String, ProbeInfo>,
) {
    for probe in direct_probe.iter().cloned() {
        collect_reachable_probe_target(probe, result);
    }
}

fn collect_reachable_probe_target(probe: ProbeInfo, result: &mut HashMap<String, ProbeInfo>) {
    if !probe.is_reachable() {
        return;
    }

    if let Some(target_node) = probe.target_node_id().map(str::to_string) {
        result.insert(target_node, probe);
    }
}

fn direct_probe_to<'a>(
    probe_targets: &'a HashMap<String, HashMap<String, ProbeInfo>>,
    source_node_id: &str,
    target_node_id: &str,
) -> Option<&'a ProbeInfo> {
    probe_targets
        .get(source_node_id)
        .and_then(|targets| targets.get(target_node_id))
}

fn shared_ood_direct_probe(
    zone_config: &ZoneConfig,
    probe_targets: &HashMap<String, HashMap<String, ProbeInfo>>,
    source_node_id: &str,
    target_node_id: &str,
) -> Option<String> {
    // 共同 OOD 推断仅在 witness OOD 不在公网时成立：能 direct 连上同一非公网 OOD 的两个
    // device 大概率位于同一 LAN；如果 witness 自己是 wan/wan_dyn/portmap，两端只是都有
    // 公网出口，不能据此推断同 LAN。
    zone_config
        .oods
        .iter()
        .filter(|ood| ood.node_type.is_ood() && !is_publicly_reachable_net_id(ood.net_id.as_ref()))
        .map(|ood| ood.name.as_str())
        .find(|ood_name| {
            direct_probe_to(probe_targets, source_node_id, ood_name).is_some()
                && direct_probe_to(probe_targets, target_node_id, ood_name).is_some()
        })
        .map(str::to_string)
}

fn route_evidence(
    evidence_type: &str,
    source_node: Option<&str>,
    target_node: Option<&str>,
    witness_node: Option<&str>,
    confidence: &str,
    applicability: &str,
) -> NodeGatewayRouteEvidence {
    NodeGatewayRouteEvidence {
        evidence_type: evidence_type.to_string(),
        source_node: source_node.map(str::to_string),
        target_node: target_node.map(str::to_string),
        witness_node: witness_node.map(str::to_string),
        last_probe: None,
        last_success: None,
        rtt_ms: None,
        freshness_ttl_secs: None,
        failure_reason: None,
        confidence: confidence.to_string(),
        applicability: applicability.to_string(),
    }
}

fn route_evidence_from_probe(
    source_node: &str,
    target_node: &str,
    probe_info: &ProbeInfo,
) -> NodeGatewayRouteEvidence {
    NodeGatewayRouteEvidence {
        evidence_type: "direct_probe".to_string(),
        source_node: Some(source_node.to_string()),
        target_node: Some(target_node.to_string()),
        witness_node: None,
        last_probe: probe_info.last_probe,
        last_success: probe_info.last_success,
        rtt_ms: probe_info.rtt_ms,
        freshness_ttl_secs: probe_info.freshness_ttl_secs,
        failure_reason: probe_info.failure_reason.clone(),
        confidence: "high".to_string(),
        applicability: "source_node".to_string(),
    }
}

fn push_route_candidate(
    candidates: &mut Vec<NodeGatewayRouteCandidate>,
    candidate: NodeGatewayRouteCandidate,
) {
    if candidates
        .iter()
        .any(|existing| existing.id == candidate.id || existing.url == candidate.url)
    {
        return;
    }
    candidates.push(candidate);
}

pub(crate) fn build_forward_plan(
    this_node_id: &str,
    zone_config: &ZoneConfig,
    zone_host: &str,
    device_list: &HashMap<String, DeviceInfo>,
) -> HashMap<String, Vec<NodeGatewayRouteCandidate>> {
    let mut routes = HashMap::new();
    let Some(source_device) = device_list.get(this_node_id) else {
        return routes;
    };
    let probe_targets = device_list
        .iter()
        .map(|(node_id, device_info)| (node_id.clone(), direct_probe_targets(device_info)))
        .collect::<HashMap<_, _>>();
    let zone_gateway_node = zone_config.get_default_zone_gateway();

    for (target_node_id, target_device) in device_list.iter() {
        if target_node_id == this_node_id {
            continue;
        }

        let target_port = device_rtcp_port(target_device);
        let target_has_signed_wan_ip = has_signed_wan_ip(target_device);
        let mut candidates = Vec::new();

        if target_has_signed_wan_ip {
            let signed_ip = &target_device.device_doc.ips[0];
            push_route_candidate(
                &mut candidates,
                NodeGatewayRouteCandidate {
                    id: "direct-signed-wan-ip".to_string(),
                    kind: "rtcp_direct".to_string(),
                    priority: 1,
                    weight: DEFAULT_ROUTE_WEIGHT,
                    backup: false,
                    keep_tunnel: false,
                    url: format_rtcp_ip_url(signed_ip, target_port),
                    source: "signed_device_doc".to_string(),
                    relay_node: None,
                    evidence: Some(route_evidence(
                        "signed_wan_ip",
                        Some(this_node_id),
                        Some(target_node_id),
                        None,
                        "high",
                        "zone_wide",
                    )),
                },
            );
            // 不再短路：签名 IP 仅停止 IP 级探索（不再尝试 DNS / Finder 推断的 IP），
            // 但显式 relay candidate 仍应作为 backup 写入，覆盖签名 IP 暂时不可达的场景。
        } else if let Some(probe_info) =
            direct_probe_to(&probe_targets, this_node_id, target_node_id)
        {
            push_route_candidate(
                &mut candidates,
                NodeGatewayRouteCandidate {
                    id: "direct-probed".to_string(),
                    kind: "rtcp_direct".to_string(),
                    priority: 10,
                    weight: DEFAULT_ROUTE_WEIGHT,
                    backup: false,
                    keep_tunnel: true,
                    url: format_rtcp_did_url(target_node_id, zone_host, target_port),
                    source: "system_config".to_string(),
                    relay_node: None,
                    evidence: Some(route_evidence_from_probe(
                        this_node_id,
                        target_node_id,
                        probe_info,
                    )),
                },
            );
        } else if same_trusted_lan(source_device, target_device) {
            push_route_candidate(
                &mut candidates,
                NodeGatewayRouteCandidate {
                    id: "direct-net-id".to_string(),
                    kind: "rtcp_direct".to_string(),
                    priority: 20,
                    weight: DEFAULT_ROUTE_WEIGHT,
                    backup: false,
                    keep_tunnel: true,
                    url: format_rtcp_did_url(target_node_id, zone_host, target_port),
                    source: "system_config".to_string(),
                    relay_node: None,
                    evidence: Some(route_evidence(
                        "same_net_id",
                        Some(this_node_id),
                        Some(target_node_id),
                        None,
                        "medium",
                        "same_lan",
                    )),
                },
            );
        } else if is_publicly_reachable_net_id(target_device.device_doc.net_id.as_ref()) {
            // target 自身声明公网可达（wan / wan_dyn / portmap），任意 source 都可以尝试 direct。
            // URL 用 DID hostname，由 name-client 在解析阶段处理 DDNS 等动态 IP。
            push_route_candidate(
                &mut candidates,
                NodeGatewayRouteCandidate {
                    id: "direct-wan-target".to_string(),
                    kind: "rtcp_direct".to_string(),
                    priority: 18,
                    weight: DEFAULT_ROUTE_WEIGHT,
                    backup: false,
                    keep_tunnel: true,
                    url: format_rtcp_did_url(target_node_id, zone_host, target_port),
                    source: "system_config".to_string(),
                    relay_node: None,
                    evidence: Some(route_evidence(
                        "wan_target_net_id",
                        Some(this_node_id),
                        Some(target_node_id),
                        None,
                        "medium",
                        "zone_wide",
                    )),
                },
            );
        } else if let Some(witness_node) =
            shared_ood_direct_probe(zone_config, &probe_targets, this_node_id, target_node_id)
        {
            push_route_candidate(
                &mut candidates,
                NodeGatewayRouteCandidate {
                    id: format!("direct-shared-ood-{}", witness_node),
                    kind: "rtcp_direct".to_string(),
                    priority: 25,
                    weight: DEFAULT_ROUTE_WEIGHT,
                    backup: false,
                    keep_tunnel: true,
                    url: format_rtcp_did_url(target_node_id, zone_host, target_port),
                    source: "system_config".to_string(),
                    relay_node: None,
                    evidence: Some(route_evidence(
                        "shared_ood_direct_probe",
                        Some(this_node_id),
                        Some(target_node_id),
                        Some(witness_node.as_str()),
                        "medium",
                        "same_lan",
                    )),
                },
            );
        }

        let relay_is_backup = !candidates.is_empty();
        if let Some(sn_host) = zone_config.sn.as_ref() {
            push_route_candidate(
                &mut candidates,
                NodeGatewayRouteCandidate {
                    id: "via-sn".to_string(),
                    kind: "rtcp_relay".to_string(),
                    priority: 30,
                    weight: DEFAULT_ROUTE_WEIGHT,
                    backup: relay_is_backup,
                    // 签名 IP 的 target 不需要为它单独维持 SN keep_tunnel：target 是稳定 WAN，
                    // SN relay 只是异常时的 backup 触点。
                    keep_tunnel: !target_has_signed_wan_ip,
                    url: format_relay_rtcp_url(
                        format!("rtcp://{}/", sn_host).as_str(),
                        target_node_id,
                        zone_host,
                        target_port,
                    ),
                    source: "zone_config".to_string(),
                    relay_node: Some(sn_host.clone()),
                    evidence: Some(route_evidence(
                        "sn_relay",
                        Some(this_node_id),
                        Some(target_node_id),
                        None,
                        "low",
                        "zone_wide",
                    )),
                },
            );
        }

        if let Some(gateway_node) = zone_gateway_node.as_ref() {
            if gateway_node != this_node_id && gateway_node != target_node_id {
                let relay_is_backup = !candidates.is_empty();
                push_route_candidate(
                    &mut candidates,
                    NodeGatewayRouteCandidate {
                        id: format!("via-zone-gateway-{}", gateway_node),
                        kind: "rtcp_relay".to_string(),
                        priority: 40,
                        weight: DEFAULT_ROUTE_WEIGHT,
                        backup: relay_is_backup,
                        keep_tunnel: false,
                        url: format_relay_rtcp_url(
                            format_rtcp_did_url(gateway_node, zone_host, DEFAULT_RTCP_PORT)
                                .as_str(),
                            target_node_id,
                            zone_host,
                            target_port,
                        ),
                        source: "zone_config".to_string(),
                        relay_node: Some(gateway_node.clone()),
                        evidence: Some(route_evidence(
                            "zone_gateway_relay",
                            Some(this_node_id),
                            Some(target_node_id),
                            Some(gateway_node.as_str()),
                            "low",
                            "zone_wide",
                        )),
                    },
                );
            }
        }

        candidates.sort_by(|left, right| {
            left.priority
                .cmp(&right.priority)
                .then(left.backup.cmp(&right.backup))
                .then(left.id.cmp(&right.id))
        });

        if !candidates.is_empty() {
            routes.insert(target_node_id.clone(), candidates);
        }
    }

    routes
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::jwk::Jwk;
    use name_lib::{
        generate_ed25519_key_pair, get_x_from_jwk, DeviceConfig, DeviceNodeType,
        OODDescriptionString, VerifyHubInfo, DID,
    };
    use serde_json::json;

    fn create_test_device_info(name: &str) -> DeviceInfo {
        let (_, public_key_jwk) = generate_ed25519_key_pair();
        let public_key_jwk: Jwk = serde_json::from_value(public_key_jwk).unwrap();
        let pkx = get_x_from_jwk(&public_key_jwk).unwrap();
        let mut device = DeviceConfig::new(name, pkx);
        device.owner = DID::new("bns", "owner");
        DeviceInfo::from_device_doc(&device)
    }

    fn create_test_device_info_with_net_id(name: &str, net_id: Option<&str>) -> DeviceInfo {
        let mut device_info = create_test_device_info(name);
        device_info.device_doc.net_id = net_id.map(str::to_string);
        device_info
    }

    fn add_direct_probe(device_info: &mut DeviceInfo, target_node: &str) {
        device_info.device_doc.extra_info.insert(
            "direct_probe".to_string(),
            json!([
                {
                    "target_node": target_node,
                    "status": "reachable"
                }
            ]),
        );
    }

    fn add_network_observation_direct_probe(device_info: &mut DeviceInfo, target_node: &str) {
        device_info.device_doc.extra_info.insert(
            "network_observation".to_string(),
            json!({
                "generation": 12,
                "observation_id": "sha256:test",
                "changed_at": 1710000000_u64,
                "observed_at": 1710000030_u64,
                "rtcp_port": 2980,
                "ipv6": {
                    "state": "egress_ok",
                    "probe_target": "ipv6.test.example",
                    "last_probe": 1710000030_u64,
                    "failure_reason": null
                },
                "endpoints": [
                    {
                        "ip": "192.168.1.23",
                        "family": "ipv4",
                        "scope": "lan",
                        "source": "system_interface",
                        "observed_at": 1710000030_u64
                    }
                ],
                "direct_probe": [
                    {
                        "target_node": target_node,
                        "kind": "rtcp_direct",
                        "url": format!("rtcp://{}.test.buckyos.io/", target_node),
                        "status": "reachable",
                        "rtt_ms": 12_u64,
                        "last_probe": 1710000030_u64,
                        "last_success": 1710000030_u64,
                        "freshness_ttl_secs": 600_u64,
                        "failure_reason": null,
                        "source": "tunnel_mgr"
                    }
                ]
            }),
        );
    }

    fn create_test_zone_config() -> ZoneConfig {
        let (_, owner_key_jwk) = generate_ed25519_key_pair();
        let owner_key_jwk: Jwk = serde_json::from_value(owner_key_jwk).unwrap();
        let (_, verify_hub_key_jwk) = generate_ed25519_key_pair();
        let verify_hub_key_jwk: Jwk = serde_json::from_value(verify_hub_key_jwk).unwrap();

        let mut zone_config = ZoneConfig::new(
            DID::new("web", "test.buckyos.io"),
            DID::new("bns", "owner"),
            owner_key_jwk,
        );
        zone_config.oods = vec![
            OODDescriptionString::new("ood1".to_string(), DeviceNodeType::OOD, None, None),
            OODDescriptionString::new("ood2".to_string(), DeviceNodeType::OOD, None, None),
        ];
        zone_config.verify_hub_info = Some(VerifyHubInfo {
            public_key: verify_hub_key_jwk,
        });
        zone_config
    }

    #[test]
    fn test_build_forward_plan_signed_wan_ip_is_primary_with_explicit_relay_backup() {
        let mut zone_config = create_test_zone_config();
        zone_config.sn = Some("sn.test.buckyos.io".to_string());
        let zone_host = zone_config.id.to_host_name();
        let device_ood1 = create_test_device_info_with_net_id("ood1", None);
        let mut device_ood2 = create_test_device_info_with_net_id("ood2", Some("wan"));
        device_ood2
            .device_doc
            .ips
            .push("203.0.113.10".parse().unwrap());

        let device_list = HashMap::from([
            ("ood1".to_string(), device_ood1),
            ("ood2".to_string(), device_ood2),
        ]);

        let routes = build_forward_plan("ood1", &zone_config, &zone_host, &device_list);
        let ood2_routes = routes.get("ood2").unwrap();

        // signed wan IP 仅停止 IP 级探索，仍应保留显式 relay backup 用于签名 IP 暂时不可达
        // 时的兜底。SN backup 不需要 keep_tunnel —— 稳定 WAN target 不该消耗 SN 资源。
        assert_eq!(ood2_routes.len(), 2);
        assert_eq!(ood2_routes[0].id, "direct-signed-wan-ip");
        assert_eq!(ood2_routes[0].kind, "rtcp_direct");
        assert_eq!(ood2_routes[0].url, "rtcp://203.0.113.10/");
        assert!(!ood2_routes[0].backup);

        let via_sn = ood2_routes
            .iter()
            .find(|candidate| candidate.id == "via-sn")
            .expect("signed wan IP target should still get SN relay backup");
        assert_eq!(via_sn.kind, "rtcp_relay");
        assert!(via_sn.backup);
        assert!(
            !via_sn.keep_tunnel,
            "signed wan IP target should not need keep_tunnel via SN"
        );
    }

    #[test]
    fn test_build_forward_plan_generates_direct_for_wan_dyn_target_without_signed_ip() {
        let mut zone_config = create_test_zone_config();
        zone_config.sn = Some("sn.test.buckyos.io".to_string());
        let zone_host = zone_config.id.to_host_name();
        // source 在 NAT 后，target 是 wan_dyn（动态公网，device_doc 中没有静态 IP）。
        let device_ood1 = create_test_device_info_with_net_id("ood1", Some("nat"));
        let device_ood2 = create_test_device_info_with_net_id("ood2", Some("wan_dyn"));

        let device_list = HashMap::from([
            ("ood1".to_string(), device_ood1),
            ("ood2".to_string(), device_ood2),
        ]);

        let routes = build_forward_plan("ood1", &zone_config, &zone_host, &device_list);
        let ood2_routes = routes.get("ood2").unwrap();

        // target 公网可达 → 任意 source（包括 NAT 后）都应生成 direct candidate；
        // URL 用 DID hostname，由 DDNS / name-client 处理动态 IP。
        let direct = ood2_routes
            .iter()
            .find(|candidate| candidate.kind == "rtcp_direct")
            .expect("wan_dyn target should yield a direct candidate even from NAT source");
        assert_eq!(direct.id, "direct-wan-target");
        assert!(!direct.backup);
        assert_eq!(direct.url, "rtcp://ood2.test.buckyos.io/");
        let evidence = direct.evidence.as_ref().unwrap();
        assert_eq!(evidence.evidence_type, "wan_target_net_id");
        assert_eq!(evidence.applicability, "zone_wide");
        assert_eq!(evidence.confidence, "medium");
    }

    #[test]
    fn test_build_forward_plan_generates_direct_for_portmap_target() {
        let zone_config = create_test_zone_config();
        let zone_host = zone_config.id.to_host_name();
        let device_ood1 = create_test_device_info_with_net_id("ood1", Some("lan1"));
        let device_ood2 = create_test_device_info_with_net_id("ood2", Some("portmap"));

        let device_list = HashMap::from([
            ("ood1".to_string(), device_ood1),
            ("ood2".to_string(), device_ood2),
        ]);

        let routes = build_forward_plan("ood1", &zone_config, &zone_host, &device_list);
        let ood2_routes = routes.get("ood2").unwrap();

        let direct = ood2_routes
            .iter()
            .find(|candidate| candidate.kind == "rtcp_direct")
            .expect("portmap target should yield a direct candidate");
        assert_eq!(direct.id, "direct-wan-target");
    }

    #[test]
    fn test_build_forward_plan_preserves_direct_probe_evidence_fields() {
        let zone_config = create_test_zone_config();
        let zone_host = zone_config.id.to_host_name();
        let mut device_ood1 = create_test_device_info_with_net_id("ood1", Some("nat"));
        let device_ood2 = create_test_device_info_with_net_id("ood2", Some("lan2"));
        add_network_observation_direct_probe(&mut device_ood1, "ood2");

        let device_list = HashMap::from([
            ("ood1".to_string(), device_ood1),
            ("ood2".to_string(), device_ood2),
        ]);

        let routes = build_forward_plan("ood1", &zone_config, &zone_host, &device_list);
        let ood2_routes = routes.get("ood2").unwrap();
        let direct = ood2_routes
            .iter()
            .find(|candidate| candidate.id == "direct-probed")
            .expect("reachable direct probe should produce a direct candidate");
        let evidence = direct.evidence.as_ref().unwrap();
        assert_eq!(evidence.evidence_type, "direct_probe");
        assert_eq!(evidence.rtt_ms, Some(12));
        assert_eq!(evidence.last_probe, Some(1710000030));
        assert_eq!(evidence.last_success, Some(1710000030));
        assert_eq!(evidence.freshness_ttl_secs, Some(600));
    }

    #[test]
    fn test_build_forward_plan_skips_shared_ood_when_witness_is_wan() {
        // 两个 LAN 后的 device 都能 direct 到一个 wan OOD：这只能说明它们都有公网出口，
        // 不能据此推断它们位于同一 LAN。共同 OOD 推断必须排除 wan witness。
        let mut zone_config = create_test_zone_config();
        zone_config.oods = vec![OODDescriptionString::new(
            "ood1".to_string(),
            DeviceNodeType::OOD,
            Some("wan".to_string()),
            None,
        )];
        let zone_host = zone_config.id.to_host_name();
        let device_ood1 = create_test_device_info_with_net_id("ood1", Some("wan"));
        let mut device_node1 = create_test_device_info_with_net_id("node1", Some("lan1"));
        let mut device_node2 = create_test_device_info_with_net_id("node2", Some("lan2"));
        add_direct_probe(&mut device_node1, "ood1");
        add_direct_probe(&mut device_node2, "ood1");

        let device_list = HashMap::from([
            ("ood1".to_string(), device_ood1),
            ("node1".to_string(), device_node1),
            ("node2".to_string(), device_node2),
        ]);

        let routes = build_forward_plan("node1", &zone_config, &zone_host, &device_list);

        // node1 -> node2: same_trusted_lan(lan1, lan2) = false；target 不是 wan-class；
        // witness ood1 是 wan，必须被过滤；没有 SN/ZG 配置 → 整个 target 没有任何 candidate。
        if let Some(node2_routes) = routes.get("node2") {
            let has_direct = node2_routes
                .iter()
                .any(|candidate| candidate.kind == "rtcp_direct");
            assert!(
                !has_direct,
                "wan witness OOD must not produce same-LAN direct inference"
            );
        }
    }

    #[test]
    fn test_build_forward_plan_uses_net_id_for_direct_primary_and_sn_backup() {
        let mut zone_config = create_test_zone_config();
        zone_config.sn = Some("sn.test.buckyos.io".to_string());
        let zone_host = zone_config.id.to_host_name();
        let device_ood1 = create_test_device_info_with_net_id("ood1", Some("lan1"));
        let device_ood2 = create_test_device_info_with_net_id("ood2", Some("lan1"));
        let device_list = HashMap::from([
            ("ood1".to_string(), device_ood1),
            ("ood2".to_string(), device_ood2),
        ]);

        let routes = build_forward_plan("ood1", &zone_config, &zone_host, &device_list);
        let ood2_routes = routes.get("ood2").unwrap();

        assert_eq!(ood2_routes[0].id, "direct-net-id");
        assert_eq!(ood2_routes[0].url, "rtcp://ood2.test.buckyos.io/");
        assert!(!ood2_routes[0].backup);
        assert_eq!(ood2_routes[1].id, "via-sn");
        assert!(ood2_routes[1].backup);
    }

    #[test]
    fn test_build_forward_plan_treats_missing_net_id_as_unknown_lan() {
        let zone_config = create_test_zone_config();
        let zone_host = zone_config.id.to_host_name();
        let device_ood1 = create_test_device_info_with_net_id("ood1", None);
        let device_ood2 = create_test_device_info_with_net_id("ood2", None);
        let device_list = HashMap::from([
            ("ood1".to_string(), device_ood1),
            ("ood2".to_string(), device_ood2),
        ]);

        let routes = build_forward_plan("ood1", &zone_config, &zone_host, &device_list);
        let ood2_routes = routes.get("ood2").unwrap();

        assert_eq!(ood2_routes[0].id, "direct-net-id");
        assert_eq!(
            ood2_routes[0].evidence.as_ref().unwrap().evidence_type,
            "same_net_id"
        );
    }

    #[test]
    fn test_build_forward_plan_uses_shared_ood_probe_as_direct_evidence() {
        let zone_config = create_test_zone_config();
        let zone_host = zone_config.id.to_host_name();
        let device_ood1 = create_test_device_info_with_net_id("ood1", Some("lan-ood"));
        let mut device_node1 = create_test_device_info_with_net_id("node1", Some("lan1"));
        let mut device_node2 = create_test_device_info_with_net_id("node2", Some("lan2"));
        add_direct_probe(&mut device_node1, "ood1");
        add_direct_probe(&mut device_node2, "ood1");

        let device_list = HashMap::from([
            ("ood1".to_string(), device_ood1),
            ("node1".to_string(), device_node1),
            ("node2".to_string(), device_node2),
        ]);

        let routes = build_forward_plan("node1", &zone_config, &zone_host, &device_list);
        let node2_routes = routes.get("node2").unwrap();

        assert_eq!(node2_routes[0].id, "direct-shared-ood-ood1");
        let evidence = node2_routes[0].evidence.as_ref().unwrap();
        assert_eq!(evidence.evidence_type, "shared_ood_direct_probe");
        assert_eq!(evidence.witness_node.as_deref(), Some("ood1"));
    }
}
