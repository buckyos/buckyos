use std::collections::HashMap;
use std::net::IpAddr;

use buckyos_api::network_observation::{
    NetworkObservation, ProbeInfo, DEFAULT_RTCP_PORT, NETWORK_OBSERVATION_KEY,
};
use name_lib::{DeviceInfo, ZoneConfig};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const DEFAULT_ROUTE_WEIGHT: u32 = 100;
const DIRECT_PRIORITY: u32 = 10;
const SN_PRIORITY: u32 = 30;
const ZONE_GATEWAY_PRIORITY: u32 = 40;

fn probe_target_node_id(probe: &ProbeInfo) -> Option<&str> {
    probe.target_node.as_deref()
}

fn probe_is_reachable(probe: &ProbeInfo) -> bool {
    probe
        .status
        .as_deref()
        .map(|status| status.eq_ignore_ascii_case("reachable"))
        .unwrap_or(false)
}

fn parse_network_observation(device_info: &DeviceInfo) -> Option<NetworkObservation> {
    let value = device_info
        .device_doc
        .extra_info
        .get(NETWORK_OBSERVATION_KEY)?;
    serde_json::from_value::<NetworkObservation>(value.clone()).ok()
}

fn global_ipv6_endpoint(observation: &NetworkObservation) -> Option<IpAddr> {
    observation
        .endpoints
        .iter()
        .filter(|ep| ep.scope.as_deref() == Some("global"))
        .filter(|ep| ep.family.as_deref() == Some("ipv6"))
        .filter_map(|ep| ep.ip.as_deref().and_then(|ip| ip.parse::<IpAddr>().ok()))
        .next()
}

fn collect_lan_ips(observation: &NetworkObservation) -> Vec<IpAddr> {
    observation
        .endpoints
        .iter()
        .filter(|ep| ep.scope.as_deref() == Some("lan"))
        .filter_map(|ep| ep.ip.as_deref().and_then(|ip| ip.parse::<IpAddr>().ok()))
        .collect()
}

fn ip_subnet_matches(a: &IpAddr, b: &IpAddr) -> bool {
    match (a, b) {
        (IpAddr::V4(a4), IpAddr::V4(b4)) => {
            // 家用 LAN 默认按 /24 判等：高于 /24 的子网划分在家用场景里不常见，
            // 即使误判也只是把"应当能直连"放过，后续业务建链失败会自然降级到 relay。
            let a_oct = a4.octets();
            let b_oct = b4.octets();
            a_oct[0] == b_oct[0] && a_oct[1] == b_oct[1] && a_oct[2] == b_oct[2]
        }
        (IpAddr::V6(a6), IpAddr::V6(b6)) => {
            // IPv6 LAN 默认按 /64 前缀匹配。
            a6.segments()[..4] == b6.segments()[..4]
        }
        _ => false,
    }
}

fn endpoints_refute_same_lan(
    source_obs: Option<&NetworkObservation>,
    target_obs: Option<&NetworkObservation>,
) -> bool {
    let (Some(src), Some(tgt)) = (source_obs, target_obs) else {
        return false;
    };
    let src_lans = collect_lan_ips(src);
    let tgt_lans = collect_lan_ips(tgt);
    if src_lans.is_empty() || tgt_lans.is_empty() {
        return false;
    }
    !src_lans
        .iter()
        .any(|s| tgt_lans.iter().any(|t| ip_subnet_matches(s, t)))
}

fn probe_is_fresh(probe: &ProbeInfo, observed_at: Option<u64>) -> bool {
    let Some(observed_at) = observed_at else {
        return true;
    };
    let Some(last_success) = probe.last_success else {
        return true;
    };
    let Some(ttl) = probe.freshness_ttl_secs else {
        return true;
    };
    observed_at.saturating_sub(last_success) <= ttl
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

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ForwardPlan {
    pub(crate) routes: HashMap<String, Vec<NodeGatewayRouteCandidate>>,
}

// scheduler 内部用来描述"为什么 direct 走得通"，最终被翻译成 candidate.evidence。
// IP 选哪个由 cyfs-gateway 的 DID/name 解析和历史 RTT 决定。
enum DirectEvidence {
    SignedWanIp,
    DirectProbe(ProbeInfo),
    Ipv6GlobalEndpoint { confidence: &'static str },
    WanTarget,
    SameNetId,
    SharedOodInference { witness: String },
}

fn device_rtcp_port(device_info: &DeviceInfo) -> u32 {
    device_info
        .device_doc
        .rtcp_port
        .unwrap_or(DEFAULT_RTCP_PORT)
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

fn device_rtcp_host(device_info: &DeviceInfo, fallback_node_id: &str, zone_host: &str) -> String {
    let host = device_info.device_doc.id.to_host_name();
    if host.trim().is_empty() {
        format!("{}.{}", fallback_node_id, zone_host)
    } else {
        host
    }
}

fn format_tcp_direct_node_url(node_id: &str, zone_host: &str) -> String {
    // cyfs-gateway tunnel URL uses the path as "host:port"; boot_gateway.yaml appends the port.
    format!("tcp:///{}.{}", node_id, zone_host)
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

fn same_trusted_lan(
    source: &DeviceInfo,
    target: &DeviceInfo,
    source_obs: Option<&NetworkObservation>,
    target_obs: Option<&NetworkObservation>,
) -> bool {
    if is_wan_net_id(source.device_doc.net_id.as_ref())
        || is_wan_net_id(target.device_doc.net_id.as_ref())
    {
        return false;
    }

    if net_id_for_planning(source) != net_id_for_planning(target) {
        return false;
    }

    // net_id 字面相同（含都为 unknown_lan）只是必要条件；如果双方都上报了 LAN endpoint
    // 但子网完全对不上，说明只是标签巧合，不能据此推断同 LAN，避免家用 VPN/桥接误标。
    !endpoints_refute_same_lan(source_obs, target_obs)
}

fn direct_probe_targets(device_info: &DeviceInfo) -> HashMap<String, ProbeInfo> {
    let mut result = HashMap::new();

    if let Some(network_observation) = device_info
        .device_doc
        .extra_info
        .get(NETWORK_OBSERVATION_KEY)
    {
        if let Ok(parsed) =
            serde_json::from_value::<NetworkObservation>(network_observation.clone())
        {
            collect_reachable_probe_targets(&parsed.direct_probe, parsed.observed_at, &mut result);
        } else if let Some(direct_probe) = network_observation
            .get("direct_probe")
            .and_then(Value::as_array)
        {
            let observed_at = network_observation
                .get("observed_at")
                .and_then(Value::as_u64);
            collect_reachable_probe_target_values(direct_probe, observed_at, &mut result);
        }
    }

    if let Some(direct_probe) = device_info
        .device_doc
        .extra_info
        .get("direct_probe")
        .and_then(Value::as_array)
    {
        // 顶层 `direct_probe` 字段没有 observation 上下文，无从判断 freshness；
        // 仍按 reachable 直接保留，由其它新鲜度通道（network_observation）覆盖。
        collect_reachable_probe_target_values(direct_probe, None, &mut result);
    }

    result
}

fn collect_reachable_probe_target_values(
    direct_probe: &[Value],
    observed_at: Option<u64>,
    result: &mut HashMap<String, ProbeInfo>,
) {
    for probe in direct_probe.iter() {
        let Ok(probe_info) = serde_json::from_value::<ProbeInfo>(probe.clone()) else {
            continue;
        };
        collect_reachable_probe_target(probe_info, observed_at, result);
    }
}

fn collect_reachable_probe_targets(
    direct_probe: &[ProbeInfo],
    observed_at: Option<u64>,
    result: &mut HashMap<String, ProbeInfo>,
) {
    for probe in direct_probe.iter().cloned() {
        collect_reachable_probe_target(probe, observed_at, result);
    }
}

fn collect_reachable_probe_target(
    probe: ProbeInfo,
    observed_at: Option<u64>,
    result: &mut HashMap<String, ProbeInfo>,
) {
    if !probe_is_reachable(&probe) {
        return;
    }
    if !probe_is_fresh(&probe, observed_at) {
        return;
    }

    if let Some(target_node) = probe_target_node_id(&probe).map(str::to_string) {
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
    source_obs: Option<&NetworkObservation>,
    target_obs: Option<&NetworkObservation>,
) -> Option<String> {
    // 共同 OOD 推断仅在 witness OOD 不在公网时成立：能 direct 连上同一非公网 OOD 的两个
    // device 大概率位于同一 LAN；如果 witness 自己是 wan/wan_dyn/portmap，两端只是都有
    // 公网出口，不能据此推断同 LAN。
    // 同时如果双方都上报了 LAN endpoint 且子网不一致，OOD 可能是 dual-homed，仅凭它
    // 直连成功无法证明 device 之间也在同一 LAN，按反证排除。
    if endpoints_refute_same_lan(source_obs, target_obs) {
        return None;
    }
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

fn ipv6_direct_confidence(
    source_obs: Option<&NetworkObservation>,
    target_obs: Option<&NetworkObservation>,
) -> Option<&'static str> {
    let source_obs = source_obs?;
    let target_obs = target_obs?;

    // source 必须至少能 IPv6 出站；address_only / unknown / unavailable 都不足以发起 direct。
    let source_state = source_obs.ipv6.as_ref()?.state.as_deref()?;
    if !matches!(source_state, "egress_ok" | "rtcp_direct_ok") {
        return None;
    }
    // target 显式声明 IPv6 不可用时，没必要把它当成 direct 路径。
    if let Some(target_state) = target_obs.ipv6.as_ref().and_then(|i| i.state.as_deref()) {
        if target_state == "unavailable" {
            return None;
        }
    }
    global_ipv6_endpoint(target_obs)?;

    // 双方都 rtcp_direct_ok 时，IPv6 直连基本等价于已验证；只 source 侧 egress_ok 时降一档。
    Some(if source_state == "rtcp_direct_ok" {
        "high"
    } else {
        "medium"
    })
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

fn direct_evidence_for(
    source_device: &DeviceInfo,
    target_device: &DeviceInfo,
    this_node_id: &str,
    target_node_id: &str,
    zone_config: &ZoneConfig,
    probe_targets: &HashMap<String, HashMap<String, ProbeInfo>>,
    source_obs: Option<&NetworkObservation>,
    target_obs: Option<&NetworkObservation>,
) -> Option<DirectEvidence> {
    if has_signed_wan_ip(target_device) {
        return Some(DirectEvidence::SignedWanIp);
    }
    if let Some(probe_info) = direct_probe_to(probe_targets, this_node_id, target_node_id) {
        return Some(DirectEvidence::DirectProbe(probe_info.clone()));
    }
    if let Some(confidence) = ipv6_direct_confidence(source_obs, target_obs) {
        return Some(DirectEvidence::Ipv6GlobalEndpoint { confidence });
    }
    if is_publicly_reachable_net_id(target_device.device_doc.net_id.as_ref()) {
        return Some(DirectEvidence::WanTarget);
    }
    if same_trusted_lan(source_device, target_device, source_obs, target_obs) {
        return Some(DirectEvidence::SameNetId);
    }
    if let Some(witness) = shared_ood_direct_probe(
        zone_config,
        probe_targets,
        this_node_id,
        target_node_id,
        source_obs,
        target_obs,
    ) {
        return Some(DirectEvidence::SharedOodInference { witness });
    }
    None
}

fn build_direct_candidate(
    this_node_id: &str,
    target_node_id: &str,
    target_device: &DeviceInfo,
    target_port: u32,
    zone_host: &str,
    evidence: DirectEvidence,
    prefer_tcp_direct: bool,
) -> NodeGatewayRouteCandidate {
    let rtcp_host = device_rtcp_host(target_device, target_node_id, zone_host);
    let (kind, url, keep_tunnel) = if prefer_tcp_direct {
        (
            "tcp_direct".to_string(),
            format_tcp_direct_node_url(target_node_id, zone_host),
            false,
        )
    } else {
        (
            "rtcp_direct".to_string(),
            format_rtcp_host_url(&rtcp_host, target_port),
            true,
        )
    };
    let route_evidence = match evidence {
        DirectEvidence::SignedWanIp => route_evidence(
            "signed_wan_ip",
            Some(this_node_id),
            Some(target_node_id),
            None,
            "high",
            "zone_wide",
        ),
        DirectEvidence::DirectProbe(probe_info) => {
            route_evidence_from_probe(this_node_id, target_node_id, &probe_info)
        }
        DirectEvidence::Ipv6GlobalEndpoint { confidence } => route_evidence(
            "ipv6_global_endpoint",
            Some(this_node_id),
            Some(target_node_id),
            None,
            confidence,
            "zone_wide",
        ),
        DirectEvidence::WanTarget => route_evidence(
            "wan_target_net_id",
            Some(this_node_id),
            Some(target_node_id),
            None,
            "medium",
            "zone_wide",
        ),
        DirectEvidence::SameNetId => route_evidence(
            "same_net_id",
            Some(this_node_id),
            Some(target_node_id),
            None,
            "medium",
            "same_lan",
        ),
        DirectEvidence::SharedOodInference { witness } => route_evidence(
            "shared_ood_direct_probe",
            Some(this_node_id),
            Some(target_node_id),
            Some(witness.as_str()),
            "medium",
            "same_lan",
        ),
    };

    NodeGatewayRouteCandidate {
        id: "direct".to_string(),
        kind,
        priority: DIRECT_PRIORITY,
        weight: DEFAULT_ROUTE_WEIGHT,
        backup: false,
        keep_tunnel,
        url,
        source: "system_config".to_string(),
        relay_node: None,
        evidence: Some(route_evidence),
    }
}

pub(crate) fn build_forward_plan(
    this_node_id: &str,
    zone_config: &ZoneConfig,
    zone_host: &str,
    device_list: &HashMap<String, DeviceInfo>,
) -> ForwardPlan {
    build_forward_plan_inner(this_node_id, zone_config, zone_host, device_list, false)
}

pub(crate) fn build_forward_plan_with_dev_tcp_direct(
    this_node_id: &str,
    zone_config: &ZoneConfig,
    zone_host: &str,
    device_list: &HashMap<String, DeviceInfo>,
) -> ForwardPlan {
    build_forward_plan_inner(this_node_id, zone_config, zone_host, device_list, true)
}

fn build_forward_plan_inner(
    this_node_id: &str,
    zone_config: &ZoneConfig,
    zone_host: &str,
    device_list: &HashMap<String, DeviceInfo>,
    prefer_tcp_direct: bool,
) -> ForwardPlan {
    let mut plan = ForwardPlan::default();
    let Some(source_device) = device_list.get(this_node_id) else {
        return plan;
    };
    let probe_targets = device_list
        .iter()
        .map(|(node_id, device_info)| (node_id.clone(), direct_probe_targets(device_info)))
        .collect::<HashMap<_, _>>();
    let network_observations: HashMap<String, NetworkObservation> = device_list
        .iter()
        .filter_map(|(node_id, device_info)| {
            parse_network_observation(device_info).map(|obs| (node_id.clone(), obs))
        })
        .collect();
    let source_obs = network_observations.get(this_node_id);
    let zone_gateway_node = zone_config.get_default_zone_gateway();

    for (target_node_id, target_device) in device_list.iter() {
        if target_node_id == this_node_id {
            continue;
        }

        let target_port = device_rtcp_port(target_device);
        let target_obs = network_observations.get(target_node_id);
        let mut candidates = Vec::new();

        if let Some(evidence) = direct_evidence_for(
            source_device,
            target_device,
            this_node_id,
            target_node_id,
            zone_config,
            &probe_targets,
            source_obs,
            target_obs,
        ) {
            candidates.push(build_direct_candidate(
                this_node_id,
                target_node_id,
                target_device,
                target_port,
                zone_host,
                evidence,
                prefer_tcp_direct,
            ));
        }

        // DEV tcp_direct 用于 VM 同 LAN 集成测试：一旦 direct 可用，routes 中只保留
        // tcp_direct，避免 cyfs-gateway 当前 round-robin 逻辑随机选中 backup relay。
        let add_relay_candidates = !(prefer_tcp_direct && !candidates.is_empty());
        if add_relay_candidates {
            if let Some(sn_host) = zone_config.sn.as_ref() {
                let relay_is_backup = !candidates.is_empty();
                candidates.push(NodeGatewayRouteCandidate {
                    id: "via-sn".to_string(),
                    kind: "rtcp_relay".to_string(),
                    priority: SN_PRIORITY,
                    weight: DEFAULT_ROUTE_WEIGHT,
                    backup: relay_is_backup,
                    // 签名 wan IP 的 target 是稳定 WAN，不需要 SN 维持 keep_tunnel；只在异常时回退。
                    keep_tunnel: !has_signed_wan_ip(target_device),
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
                });
            }

            if let Some(gateway_node) = zone_gateway_node.as_ref() {
                if gateway_node != this_node_id && gateway_node != target_node_id {
                    let relay_is_backup = !candidates.is_empty();
                    candidates.push(NodeGatewayRouteCandidate {
                        id: format!("via-zone-gateway-{}", gateway_node),
                        kind: "rtcp_relay".to_string(),
                        priority: ZONE_GATEWAY_PRIORITY,
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
                    });
                }
            }
        }

        candidates.sort_by(|left, right| {
            left.priority
                .cmp(&right.priority)
                .then(left.backup.cmp(&right.backup))
                .then(left.id.cmp(&right.id))
        });

        if !candidates.is_empty() {
            plan.routes.insert(target_node_id.clone(), candidates);
        }
    }

    plan
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

    fn expected_rtcp_url(device_info: &DeviceInfo) -> String {
        format!("rtcp://{}/", device_info.device_doc.id.to_host_name())
    }

    fn expected_rtcp_host(device_info: &DeviceInfo) -> String {
        device_info.device_doc.id.to_host_name()
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

    fn add_lan_endpoint(device_info: &mut DeviceInfo, ip: &str) {
        device_info.device_doc.extra_info.insert(
            "network_observation".to_string(),
            json!({
                "observed_at": 1710000030_u64,
                "endpoints": [
                    {
                        "ip": ip,
                        "family": "ipv4",
                        "scope": "lan",
                        "source": "system_interface",
                        "observed_at": 1710000030_u64
                    }
                ]
            }),
        );
    }

    fn add_ipv6_observation(device_info: &mut DeviceInfo, state: &str, global_ipv6: Option<&str>) {
        let mut endpoints = Vec::new();
        if let Some(ip) = global_ipv6 {
            endpoints.push(json!({
                "ip": ip,
                "family": "ipv6",
                "scope": "global",
                "source": "system_interface",
                "observed_at": 1710000030_u64
            }));
        }
        device_info.device_doc.extra_info.insert(
            "network_observation".to_string(),
            json!({
                "observed_at": 1710000030_u64,
                "ipv6": { "state": state },
                "endpoints": endpoints
            }),
        );
    }

    #[test]
    fn test_build_forward_plan_signed_wan_ip_yields_did_url() {
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

        let plan = build_forward_plan("ood1", &zone_config, &zone_host, &device_list);
        let ood2_routes = plan.routes.get("ood2").unwrap();

        // routes 收敛到 direct + via-sn 两条；URL 始终是 DID hostname 形态。
        assert_eq!(ood2_routes.len(), 2);
        assert_eq!(ood2_routes[0].id, "direct");
        assert_eq!(ood2_routes[0].kind, "rtcp_direct");
        assert_eq!(ood2_routes[0].priority, DIRECT_PRIORITY);
        assert_eq!(
            ood2_routes[0].url,
            expected_rtcp_url(device_list.get("ood2").unwrap())
        );
        assert!(!ood2_routes[0].backup);
        assert_eq!(
            ood2_routes[0].evidence.as_ref().unwrap().evidence_type,
            "signed_wan_ip"
        );

        let via_sn = ood2_routes
            .iter()
            .find(|c| c.id == "via-sn")
            .expect("signed wan IP target should still get SN relay backup");
        assert!(via_sn.backup);
        assert!(
            !via_sn.keep_tunnel,
            "stable wan target should not need SN keep_tunnel"
        );
    }

    #[test]
    fn test_build_forward_plan_dev_tcp_direct_rewrites_direct_candidate() {
        let zone_config = create_test_zone_config();
        let zone_host = zone_config.id.to_host_name();
        let device_ood1 = create_test_device_info_with_net_id("ood1", Some("lan1"));
        let device_ood2 = create_test_device_info_with_net_id("ood2", Some("lan1"));

        let device_list = HashMap::from([
            ("ood1".to_string(), device_ood1),
            ("ood2".to_string(), device_ood2),
        ]);

        let plan =
            build_forward_plan_with_dev_tcp_direct("ood1", &zone_config, &zone_host, &device_list);
        let ood2_routes = plan.routes.get("ood2").unwrap();
        let direct = ood2_routes
            .iter()
            .find(|candidate| candidate.id == "direct")
            .unwrap();

        assert_eq!(direct.kind, "tcp_direct");
        assert_eq!(direct.url, format!("tcp:///ood2.{}", zone_host));
        assert!(!direct.keep_tunnel);
        assert!(!direct.backup);
        assert_eq!(ood2_routes.len(), 1);
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

        let plan = build_forward_plan("ood1", &zone_config, &zone_host, &device_list);
        let ood2_routes = plan.routes.get("ood2").unwrap();

        let direct = ood2_routes
            .iter()
            .find(|c| c.id == "direct")
            .expect("wan_dyn target should yield a direct candidate even from NAT source");
        assert_eq!(direct.kind, "rtcp_direct");
        assert!(!direct.backup);
        assert_eq!(
            direct.url,
            expected_rtcp_url(device_list.get("ood2").unwrap())
        );
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

        let plan = build_forward_plan("ood1", &zone_config, &zone_host, &device_list);
        let ood2_routes = plan.routes.get("ood2").unwrap();

        let direct = ood2_routes
            .iter()
            .find(|c| c.id == "direct")
            .expect("portmap target should yield a direct candidate");
        assert_eq!(
            direct.evidence.as_ref().unwrap().evidence_type,
            "wan_target_net_id"
        );
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

        let plan = build_forward_plan("ood1", &zone_config, &zone_host, &device_list);
        let ood2_routes = plan.routes.get("ood2").unwrap();
        let direct = ood2_routes
            .iter()
            .find(|c| c.id == "direct")
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

        let plan = build_forward_plan("node1", &zone_config, &zone_host, &device_list);

        if let Some(node2_routes) = plan.routes.get("node2") {
            let has_direct = node2_routes.iter().any(|c| c.id == "direct");
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

        let plan = build_forward_plan("ood1", &zone_config, &zone_host, &device_list);
        let ood2_routes = plan.routes.get("ood2").unwrap();

        assert_eq!(ood2_routes.len(), 2);
        assert_eq!(ood2_routes[0].id, "direct");
        assert_eq!(
            ood2_routes[0].url,
            expected_rtcp_url(device_list.get("ood2").unwrap())
        );
        assert!(!ood2_routes[0].backup);
        assert_eq!(
            ood2_routes[0].evidence.as_ref().unwrap().evidence_type,
            "same_net_id"
        );
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

        let plan = build_forward_plan("ood1", &zone_config, &zone_host, &device_list);
        let ood2_routes = plan.routes.get("ood2").unwrap();

        assert_eq!(ood2_routes[0].id, "direct");
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

        let plan = build_forward_plan("node1", &zone_config, &zone_host, &device_list);
        let node2_routes = plan.routes.get("node2").unwrap();

        assert_eq!(node2_routes[0].id, "direct");
        let evidence = node2_routes[0].evidence.as_ref().unwrap();
        assert_eq!(evidence.evidence_type, "shared_ood_direct_probe");
        assert_eq!(evidence.witness_node.as_deref(), Some("ood1"));
    }

    #[test]
    fn test_build_forward_plan_skips_stale_direct_probe() {
        // probe 上报时已经超过 freshness_ttl_secs：last_success 比 observed_at 老 1000s，
        // ttl 60s。direct evidence 应回退到 net_id 推断而非 direct_probe。
        let mut zone_config = create_test_zone_config();
        zone_config.sn = Some("sn.test.buckyos.io".to_string());
        let zone_host = zone_config.id.to_host_name();
        let mut device_ood1 = create_test_device_info_with_net_id("ood1", Some("lan1"));
        let device_ood2 = create_test_device_info_with_net_id("ood2", Some("lan1"));
        device_ood1.device_doc.extra_info.insert(
            "network_observation".to_string(),
            json!({
                "observed_at": 1710001000_u64,
                "direct_probe": [
                    {
                        "target_node": "ood2",
                        "status": "reachable",
                        "last_success": 1710000000_u64,
                        "freshness_ttl_secs": 60_u64
                    }
                ]
            }),
        );

        let device_list = HashMap::from([
            ("ood1".to_string(), device_ood1),
            ("ood2".to_string(), device_ood2),
        ]);

        let plan = build_forward_plan("ood1", &zone_config, &zone_host, &device_list);
        let ood2_routes = plan.routes.get("ood2").unwrap();

        let direct = ood2_routes
            .iter()
            .find(|c| c.id == "direct")
            .expect("net_id inference should still produce direct candidate");
        assert_eq!(
            direct.evidence.as_ref().unwrap().evidence_type,
            "same_net_id",
            "stale probe must not produce direct_probe evidence"
        );
    }

    #[test]
    fn test_build_forward_plan_keeps_lan_endpoint_out_of_routes() {
        // LAN endpoint 不再写成独立 candidate；IP 解析走 DID info cache / resolve_ips。
        let zone_config = create_test_zone_config();
        let zone_host = zone_config.id.to_host_name();
        let device_ood1 = create_test_device_info_with_net_id("ood1", Some("lan1"));
        let mut device_ood2 = create_test_device_info_with_net_id("ood2", Some("lan1"));
        add_lan_endpoint(&mut device_ood2, "192.168.1.23");

        let device_list = HashMap::from([
            ("ood1".to_string(), device_ood1),
            ("ood2".to_string(), device_ood2),
        ]);

        let plan = build_forward_plan("ood1", &zone_config, &zone_host, &device_list);
        let ood2_routes = plan.routes.get("ood2").unwrap();

        // routes 只剩 DID hostname 的 direct，没有 IP 形态 candidate。
        assert!(ood2_routes.iter().all(|c| !c.url.contains("192.168.1.23")));
        assert_eq!(ood2_routes[0].id, "direct");
    }

    #[test]
    fn test_build_forward_plan_prefers_direct_probe_evidence_over_raw_lan_endpoints() {
        let zone_config = create_test_zone_config();
        let zone_host = zone_config.id.to_host_name();
        let mut device_ood1 = create_test_device_info_with_net_id("ood1", Some("lan1"));
        let mut device_ood2 = create_test_device_info_with_net_id("ood2", Some("lan1"));

        device_ood1.device_doc.extra_info.insert(
            "network_observation".to_string(),
            json!({
                "observed_at": 1710000030_u64,
                "direct_probe": [
                    {
                        "target_node": "ood2",
                        "kind": "rtcp_direct",
                        "url": "rtcp://10.178.80.14/",
                        "status": "reachable",
                        "rtt_ms": 12_u64,
                        "last_probe": 1710000030_u64,
                        "last_success": 1710000030_u64,
                        "freshness_ttl_secs": 600_u64,
                        "source": "node_daemon"
                    }
                ]
            }),
        );
        device_ood2.device_doc.extra_info.insert(
            "network_observation".to_string(),
            json!({
                "observed_at": 1710000030_u64,
                "endpoints": [
                    {
                        "ip": "172.17.0.1",
                        "family": "ipv4",
                        "scope": "lan",
                        "source": "system_interface",
                        "observed_at": 1710000030_u64
                    },
                    {
                        "ip": "10.178.80.14",
                        "family": "ipv4",
                        "scope": "lan",
                        "source": "system_interface",
                        "observed_at": 1710000030_u64
                    }
                ]
            }),
        );

        let device_list = HashMap::from([
            ("ood1".to_string(), device_ood1),
            ("ood2".to_string(), device_ood2),
        ]);

        let plan = build_forward_plan("ood1", &zone_config, &zone_host, &device_list);
        let ood2_routes = plan.routes.get("ood2").unwrap();
        let direct = ood2_routes
            .iter()
            .find(|c| c.id == "direct")
            .expect("fresh probe should produce direct candidate");
        assert_eq!(
            direct.evidence.as_ref().unwrap().evidence_type,
            "direct_probe"
        );
        assert_eq!(
            direct.url,
            expected_rtcp_url(device_list.get("ood2").unwrap())
        );
        assert!(ood2_routes.iter().all(|c| !c.url.contains("172.17.0.1")));
    }

    #[test]
    fn test_build_forward_plan_emits_ipv6_direct_for_dual_lan_with_global_v6() {
        // 双 LAN（lan1 / lan2）—— same_trusted_lan 不成立，没有共同 OOD，没有 wan target；
        // 但双方都有 IPv6，target 上报 global v6 endpoint：仍可生成 direct candidate（DID URL）。
        let mut zone_config = create_test_zone_config();
        zone_config.sn = Some("sn.test.buckyos.io".to_string());
        let zone_host = zone_config.id.to_host_name();
        let mut device_ood1 = create_test_device_info_with_net_id("ood1", Some("lan1"));
        let mut device_ood2 = create_test_device_info_with_net_id("ood2", Some("lan2"));
        add_ipv6_observation(&mut device_ood1, "egress_ok", None);
        add_ipv6_observation(&mut device_ood2, "address_only", Some("2001:db9::23"));

        let device_list = HashMap::from([
            ("ood1".to_string(), device_ood1),
            ("ood2".to_string(), device_ood2),
        ]);

        let plan = build_forward_plan("ood1", &zone_config, &zone_host, &device_list);
        let ood2_routes = plan.routes.get("ood2").unwrap();

        let direct = ood2_routes
            .iter()
            .find(|c| c.id == "direct")
            .expect("dual-LAN with global v6 endpoint should yield IPv6 direct candidate");
        assert_eq!(
            direct.url,
            expected_rtcp_url(device_list.get("ood2").unwrap())
        );
        let evidence = direct.evidence.as_ref().unwrap();
        assert_eq!(evidence.evidence_type, "ipv6_global_endpoint");
        assert_eq!(evidence.confidence, "medium");
    }

    #[test]
    fn test_build_forward_plan_skips_ipv6_when_target_v6_unavailable() {
        // target 显式声明 IPv6 unavailable：v6 evidence 不触发。
        let zone_config = create_test_zone_config();
        let zone_host = zone_config.id.to_host_name();
        let mut device_ood1 = create_test_device_info_with_net_id("ood1", Some("lan1"));
        let mut device_ood2 = create_test_device_info_with_net_id("ood2", Some("lan2"));
        add_ipv6_observation(&mut device_ood1, "rtcp_direct_ok", None);
        add_ipv6_observation(&mut device_ood2, "unavailable", Some("2001:db9::23"));

        let device_list = HashMap::from([
            ("ood1".to_string(), device_ood1),
            ("ood2".to_string(), device_ood2),
        ]);

        let plan = build_forward_plan("ood1", &zone_config, &zone_host, &device_list);
        let ood2_routes = plan.routes.get("ood2").cloned().unwrap_or_default();
        assert!(ood2_routes
            .iter()
            .all(|c| c.evidence.as_ref().map(|e| e.evidence_type.as_str())
                != Some("ipv6_global_endpoint")));
    }

    #[test]
    fn test_build_forward_plan_subnet_mismatch_blocks_same_lan() {
        // 双方都标 lan1（可能因为家用 VPN/桥接误标），但实际 LAN endpoint 子网完全不同；
        // endpoints 子网反证应阻止 same_trusted_lan 推断，落到 relay-only。
        let mut zone_config = create_test_zone_config();
        zone_config.sn = Some("sn.test.buckyos.io".to_string());
        let zone_host = zone_config.id.to_host_name();
        let mut device_ood1 = create_test_device_info_with_net_id("ood1", Some("lan1"));
        let mut device_ood2 = create_test_device_info_with_net_id("ood2", Some("lan1"));
        add_lan_endpoint(&mut device_ood1, "192.168.1.10");
        add_lan_endpoint(&mut device_ood2, "10.0.0.20");

        let device_list = HashMap::from([
            ("ood1".to_string(), device_ood1),
            ("ood2".to_string(), device_ood2),
        ]);

        let plan = build_forward_plan("ood1", &zone_config, &zone_host, &device_list);
        let ood2_routes = plan.routes.get("ood2").unwrap();

        assert!(
            !ood2_routes.iter().any(|c| c.id == "direct"),
            "subnet mismatch must refute same-LAN direct candidate"
        );
        assert!(ood2_routes.iter().any(|c| c.id == "via-sn"));
    }
}
