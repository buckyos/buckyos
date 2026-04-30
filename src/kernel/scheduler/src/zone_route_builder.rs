use std::collections::{HashMap, HashSet};

use name_lib::{DeviceInfo, ZoneConfig};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const DEFAULT_RTCP_PORT: u32 = 2980;
const DEFAULT_ROUTE_WEIGHT: u32 = 100;

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

fn direct_probe_targets(device_info: &DeviceInfo) -> HashSet<String> {
    let mut result = HashSet::new();

    if let Some(direct_probe) = device_info
        .device_doc
        .extra_info
        .get("network_observation")
        .and_then(|network_observation| network_observation.get("direct_probe"))
        .and_then(Value::as_array)
    {
        collect_reachable_probe_targets(direct_probe, &mut result);
    }

    if let Some(direct_probe) = device_info
        .device_doc
        .extra_info
        .get("direct_probe")
        .and_then(Value::as_array)
    {
        collect_reachable_probe_targets(direct_probe, &mut result);
    }

    result
}

fn collect_reachable_probe_targets(direct_probe: &[Value], result: &mut HashSet<String>) {
    for probe in direct_probe.iter() {
        let status = probe
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !status.eq_ignore_ascii_case("reachable") {
            continue;
        }

        let target_node = probe
            .get("target_node")
            .or_else(|| probe.get("target_node_id"))
            .and_then(Value::as_str);
        if let Some(target_node) = target_node {
            result.insert(target_node.to_string());
        }
    }
}

fn has_direct_probe_to(
    probe_targets: &HashMap<String, HashSet<String>>,
    source_node_id: &str,
    target_node_id: &str,
) -> bool {
    probe_targets
        .get(source_node_id)
        .map(|targets| targets.contains(target_node_id))
        .unwrap_or(false)
}

fn shared_ood_direct_probe(
    zone_config: &ZoneConfig,
    probe_targets: &HashMap<String, HashSet<String>>,
    source_node_id: &str,
    target_node_id: &str,
) -> Option<String> {
    zone_config
        .oods
        .iter()
        .filter(|ood| ood.node_type.is_ood())
        .map(|ood| ood.name.as_str())
        .find(|ood_name| {
            has_direct_probe_to(probe_targets, source_node_id, ood_name)
                && has_direct_probe_to(probe_targets, target_node_id, ood_name)
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
        confidence: confidence.to_string(),
        applicability: applicability.to_string(),
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
        let mut candidates = Vec::new();

        if has_signed_wan_ip(target_device) {
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
            candidates.sort_by(|left, right| {
                left.priority
                    .cmp(&right.priority)
                    .then(left.backup.cmp(&right.backup))
                    .then(left.id.cmp(&right.id))
            });
            routes.insert(target_node_id.clone(), candidates);
            continue;
        }

        if has_direct_probe_to(&probe_targets, this_node_id, target_node_id) {
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
                    evidence: Some(route_evidence(
                        "direct_probe",
                        Some(this_node_id),
                        Some(target_node_id),
                        None,
                        "high",
                        "source_node",
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
                    keep_tunnel: true,
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
    fn test_build_forward_plan_prefers_signed_wan_ip_and_stops_exploration() {
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

        assert_eq!(ood2_routes.len(), 1);
        assert_eq!(ood2_routes[0].id, "direct-signed-wan-ip");
        assert_eq!(ood2_routes[0].kind, "rtcp_direct");
        assert_eq!(ood2_routes[0].url, "rtcp://203.0.113.10/");
        assert!(!ood2_routes[0].backup);
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
