use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use buckyos_kit::buckyos_get_unix_timestamp;
use log::*;
use name_lib::{DeviceConfig, DeviceInfo};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::net::TcpStream;
use tokio::task::JoinSet;

pub const NETWORK_OBSERVATION_KEY: &str = "network_observation";
pub const DEFAULT_RTCP_PORT: u32 = 2980;

const DEFAULT_IPV6_PROBE_TARGET: &str = "[2606:4700:4700::1111]:443";
const DEFAULT_DIRECT_PROBE_TIMEOUT_MS: u64 = 1500;
const DEFAULT_IPV6_PROBE_TIMEOUT_MS: u64 = 1500;
const DEFAULT_DIRECT_PROBE_TTL_SECS: u64 = 600;
const DEFAULT_PROBE_SOURCE: &str = "node_daemon";

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct NetworkObservation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changed_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtcp_port: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ipv6: Option<Ipv6Observation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub endpoints: Vec<NetworkEndpoint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub direct_probe: Vec<ProbeInfo>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Ipv6Observation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_probe: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct NetworkEndpoint {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ProbeInfo {
    #[serde(
        default,
        alias = "target_node_id",
        skip_serializing_if = "Option::is_none"
    )]
    pub target_node: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtt_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_probe: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_success: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness_ttl_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Clone, Debug)]
pub struct NetworkObserverConfig {
    pub ipv6_probe_target: String,
    pub ipv6_probe_timeout: Duration,
    pub direct_probe_timeout: Duration,
    pub direct_probe_freshness_ttl_secs: u64,
    pub probe_source: String,
}

impl Default for NetworkObserverConfig {
    fn default() -> Self {
        Self {
            ipv6_probe_target: DEFAULT_IPV6_PROBE_TARGET.to_string(),
            ipv6_probe_timeout: Duration::from_millis(DEFAULT_IPV6_PROBE_TIMEOUT_MS),
            direct_probe_timeout: Duration::from_millis(DEFAULT_DIRECT_PROBE_TIMEOUT_MS),
            direct_probe_freshness_ttl_secs: DEFAULT_DIRECT_PROBE_TTL_SECS,
            probe_source: DEFAULT_PROBE_SOURCE.to_string(),
        }
    }
}

pub struct NetworkObserver {
    config: NetworkObserverConfig,
    last_observation_id: Option<String>,
    generation: u64,
    last_changed_at: u64,
}

impl NetworkObserver {
    pub fn new(config: NetworkObserverConfig) -> Self {
        Self {
            config,
            last_observation_id: None,
            generation: 0,
            last_changed_at: 0,
        }
    }

    pub async fn observe(
        &mut self,
        device_doc: &DeviceConfig,
        all_ip: &[IpAddr],
        peers: &HashMap<String, DeviceInfo>,
    ) -> NetworkObservation {
        let observed_at = buckyos_get_unix_timestamp();
        let endpoints = collect_endpoints(all_ip, &device_doc.ips, observed_at);
        let has_global_v6 = endpoints
            .iter()
            .any(|ep| ep.family.as_deref() == Some("ipv6") && ep.scope.as_deref() == Some("global"));
        let ipv6 = self.probe_ipv6_state(has_global_v6, observed_at).await;
        let direct_probe = self
            .probe_peers(device_doc.name.as_str(), peers, observed_at)
            .await;
        let rtcp_port = device_doc.rtcp_port.or(Some(DEFAULT_RTCP_PORT));

        let mut obs = NetworkObservation {
            generation: None,
            observation_id: None,
            changed_at: None,
            observed_at: Some(observed_at),
            rtcp_port,
            ipv6: Some(ipv6),
            endpoints,
            direct_probe,
        };

        let id = compute_observation_id(&obs);
        let changed = self.last_observation_id.as_deref() != Some(id.as_str());
        if changed {
            self.generation = self.generation.saturating_add(1);
            self.last_changed_at = observed_at;
            self.last_observation_id = Some(id.clone());
        }
        obs.generation = Some(self.generation);
        obs.observation_id = Some(id);
        obs.changed_at = Some(self.last_changed_at);
        obs
    }

    async fn probe_ipv6_state(&self, has_global_v6: bool, now: u64) -> Ipv6Observation {
        if !has_global_v6 {
            return Ipv6Observation {
                state: Some("unavailable".to_string()),
                probe_target: None,
                last_probe: Some(now),
                failure_reason: None,
            };
        }
        let target = self.config.ipv6_probe_target.clone();
        match tokio::time::timeout(
            self.config.ipv6_probe_timeout,
            TcpStream::connect(target.as_str()),
        )
        .await
        {
            Ok(Ok(_)) => Ipv6Observation {
                state: Some("egress_ok".to_string()),
                probe_target: Some(target),
                last_probe: Some(now),
                failure_reason: None,
            },
            Ok(Err(err)) => Ipv6Observation {
                state: Some("address_only".to_string()),
                probe_target: Some(target),
                last_probe: Some(now),
                failure_reason: Some(err.to_string()),
            },
            Err(_) => Ipv6Observation {
                state: Some("address_only".to_string()),
                probe_target: Some(target),
                last_probe: Some(now),
                failure_reason: Some("timeout".to_string()),
            },
        }
    }

    async fn probe_peers(
        &self,
        self_node_id: &str,
        peers: &HashMap<String, DeviceInfo>,
        now: u64,
    ) -> Vec<ProbeInfo> {
        let mut join_set: JoinSet<Option<ProbeInfo>> = JoinSet::new();
        for (peer_name, peer_info) in peers.iter() {
            if peer_name == self_node_id {
                continue;
            }
            let candidates = collect_probe_candidates(peer_info);
            if candidates.is_empty() {
                continue;
            }
            let port = peer_info
                .device_doc
                .rtcp_port
                .unwrap_or(DEFAULT_RTCP_PORT);
            let timeout = self.config.direct_probe_timeout;
            let source = self.config.probe_source.clone();
            let ttl = self.config.direct_probe_freshness_ttl_secs;
            let target = peer_name.clone();
            join_set.spawn(async move {
                probe_single_peer(target, port, candidates, timeout, source, ttl, now).await
            });
        }

        let mut results = Vec::new();
        while let Some(joined) = join_set.join_next().await {
            match joined {
                Ok(Some(probe)) => results.push(probe),
                Ok(None) => {}
                Err(err) => warn!("direct probe task join failed: {}", err),
            }
        }
        results.sort_by(|a, b| a.target_node.cmp(&b.target_node));
        results
    }
}

fn collect_endpoints(
    all_ip: &[IpAddr],
    declared_ips: &[IpAddr],
    observed_at: u64,
) -> Vec<NetworkEndpoint> {
    let mut seen: Vec<IpAddr> = Vec::new();
    let mut endpoints: Vec<NetworkEndpoint> = Vec::new();
    let mut push = |ip: IpAddr| {
        if seen.iter().any(|existing| existing == &ip) {
            return;
        }
        if let Some((family, scope)) = classify_ip(&ip) {
            seen.push(ip);
            endpoints.push(NetworkEndpoint {
                ip: Some(ip.to_string()),
                family: Some(family.to_string()),
                scope: Some(scope.to_string()),
                source: Some("system_interface".to_string()),
                observed_at: Some(observed_at),
            });
        }
    };
    for ip in all_ip {
        push(*ip);
    }
    for ip in declared_ips {
        push(*ip);
    }
    endpoints
}

fn classify_ip(ip: &IpAddr) -> Option<(&'static str, &'static str)> {
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_loopback()
                || v4.is_unspecified()
                || v4.is_multicast()
                || v4.is_broadcast()
                || v4.is_link_local()
                || v4.is_documentation()
            {
                return None;
            }
            let scope = if v4.is_private() { "lan" } else { "global" };
            Some(("ipv4", scope))
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() {
                return None;
            }
            let segments = v6.segments();
            // link-local fe80::/10
            if (segments[0] & 0xffc0) == 0xfe80 {
                return None;
            }
            // unique-local fc00::/7 — treat as lan-only would need a stable interpretation;
            // skip for now to match the upstream `should_collect_ipv6` filtering.
            if (segments[0] & 0xfe00) == 0xfc00 {
                return None;
            }
            // documentation 2001:db8::/32
            if segments[0] == 0x2001 && segments[1] == 0x0db8 {
                return None;
            }
            Some(("ipv6", "global"))
        }
    }
}

fn collect_probe_candidates(peer: &DeviceInfo) -> Vec<IpAddr> {
    let mut ips: Vec<IpAddr> = Vec::new();
    for ip in peer.device_doc.ips.iter() {
        if !ips.contains(ip) {
            ips.push(*ip);
        }
    }
    if let Some(value) = peer.device_doc.extra_info.get(NETWORK_OBSERVATION_KEY) {
        if let Ok(obs) = serde_json::from_value::<NetworkObservation>(value.clone()) {
            for ep in obs.endpoints {
                if let Some(ip_str) = ep.ip.as_deref() {
                    if let Ok(ip) = ip_str.parse::<IpAddr>() {
                        if !ips.contains(&ip) {
                            ips.push(ip);
                        }
                    }
                }
            }
        }
    }
    ips
}

async fn probe_single_peer(
    target_node: String,
    port: u32,
    candidates: Vec<IpAddr>,
    timeout: Duration,
    source: String,
    ttl_secs: u64,
    now: u64,
) -> Option<ProbeInfo> {
    for ip in candidates {
        let addr = format_socket_addr(&ip, port);
        let started = Instant::now();
        match tokio::time::timeout(timeout, TcpStream::connect(addr.as_str())).await {
            Ok(Ok(_stream)) => {
                let rtt_ms = started.elapsed().as_millis() as u64;
                return Some(ProbeInfo {
                    target_node: Some(target_node),
                    kind: Some("rtcp_direct".to_string()),
                    url: Some(format_rtcp_url(&ip, port)),
                    status: Some("reachable".to_string()),
                    rtt_ms: Some(rtt_ms),
                    last_probe: Some(now),
                    last_success: Some(now),
                    freshness_ttl_secs: Some(ttl_secs),
                    failure_reason: None,
                    source: Some(source),
                });
            }
            Ok(Err(err)) => debug!(
                "direct probe to {} via {} failed: {}",
                target_node, addr, err
            ),
            Err(_) => debug!("direct probe to {} via {} timed out", target_node, addr),
        }
    }
    None
}

fn format_socket_addr(ip: &IpAddr, port: u32) -> String {
    match ip {
        IpAddr::V4(v4) => format!("{}:{}", v4, port),
        IpAddr::V6(v6) => format!("[{}]:{}", v6, port),
    }
}

fn format_rtcp_url(ip: &IpAddr, port: u32) -> String {
    let host = match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => format!("[{}]", v6),
    };
    if port == DEFAULT_RTCP_PORT {
        format!("rtcp://{}/", host)
    } else {
        format!("rtcp://{}:{}/", host, port)
    }
}

fn compute_observation_id(obs: &NetworkObservation) -> String {
    let mut hasher = Sha256::new();
    if let Some(port) = obs.rtcp_port {
        hasher.update(b"port:");
        hasher.update(port.to_le_bytes());
    }
    if let Some(ipv6) = obs.ipv6.as_ref() {
        if let Some(state) = ipv6.state.as_deref() {
            hasher.update(b"v6_state:");
            hasher.update(state.as_bytes());
        }
        if let Some(target) = ipv6.probe_target.as_deref() {
            hasher.update(b"v6_target:");
            hasher.update(target.as_bytes());
        }
    }
    let mut endpoints: Vec<&NetworkEndpoint> = obs.endpoints.iter().collect();
    endpoints.sort_by(|a, b| {
        let key_a = (a.ip.as_deref(), a.family.as_deref(), a.scope.as_deref());
        let key_b = (b.ip.as_deref(), b.family.as_deref(), b.scope.as_deref());
        key_a.cmp(&key_b)
    });
    for ep in endpoints {
        hasher.update(b"|ep|");
        if let Some(ip) = ep.ip.as_deref() {
            hasher.update(ip.as_bytes());
        }
        hasher.update(b"|");
        if let Some(scope) = ep.scope.as_deref() {
            hasher.update(scope.as_bytes());
        }
        hasher.update(b"|");
        if let Some(family) = ep.family.as_deref() {
            hasher.update(family.as_bytes());
        }
    }
    let mut probes: Vec<&ProbeInfo> = obs.direct_probe.iter().collect();
    probes.sort_by(|a, b| a.target_node.cmp(&b.target_node));
    for p in probes {
        hasher.update(b"|pr|");
        if let Some(target) = p.target_node.as_deref() {
            hasher.update(target.as_bytes());
        }
        hasher.update(b"|");
        if let Some(status) = p.status.as_deref() {
            hasher.update(status.as_bytes());
        }
        hasher.update(b"|");
        if let Some(url) = p.url.as_deref() {
            hasher.update(url.as_bytes());
        }
    }
    let digest = hasher.finalize();
    let mut out = String::with_capacity(7 + digest.len() * 2);
    out.push_str("sha256:");
    for byte in digest.iter() {
        use std::fmt::Write as _;
        let _ = write!(out, "{:02x}", byte);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn classify_private_ipv4_as_lan() {
        let (family, scope) = classify_ip(&IpAddr::V4(Ipv4Addr::new(192, 168, 1, 23))).unwrap();
        assert_eq!(family, "ipv4");
        assert_eq!(scope, "lan");
    }

    #[test]
    fn classify_public_ipv4_as_global() {
        let (family, scope) = classify_ip(&IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))).unwrap();
        assert_eq!(family, "ipv4");
        assert_eq!(scope, "global");
    }

    #[test]
    fn classify_link_local_ipv4_skipped() {
        assert!(classify_ip(&IpAddr::V4(Ipv4Addr::new(169, 254, 1, 1))).is_none());
    }

    #[test]
    fn classify_global_ipv6() {
        let (family, scope) = classify_ip(&IpAddr::V6(
            "2001:db9::1".parse::<Ipv6Addr>().unwrap(),
        ))
        .unwrap();
        assert_eq!(family, "ipv6");
        assert_eq!(scope, "global");
    }

    #[test]
    fn classify_link_local_ipv6_skipped() {
        let v6: Ipv6Addr = "fe80::1".parse().unwrap();
        assert!(classify_ip(&IpAddr::V6(v6)).is_none());
    }

    #[test]
    fn collect_endpoints_dedupes_and_categorizes() {
        let lan = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 23));
        let global = IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8));
        let endpoints = collect_endpoints(&[lan, lan, global], &[lan], 100);
        assert_eq!(endpoints.len(), 2);
        assert_eq!(endpoints[0].ip.as_deref(), Some("192.168.1.23"));
        assert_eq!(endpoints[0].scope.as_deref(), Some("lan"));
        assert_eq!(endpoints[1].ip.as_deref(), Some("8.8.8.8"));
        assert_eq!(endpoints[1].scope.as_deref(), Some("global"));
    }

    #[test]
    fn observation_id_is_stable_under_volatile_fields() {
        let mut a = NetworkObservation::default();
        a.observed_at = Some(100);
        a.rtcp_port = Some(2980);
        a.endpoints.push(NetworkEndpoint {
            ip: Some("192.168.1.1".to_string()),
            family: Some("ipv4".to_string()),
            scope: Some("lan".to_string()),
            source: Some("system_interface".to_string()),
            observed_at: Some(100),
        });
        let mut b = a.clone();
        b.observed_at = Some(999);
        b.endpoints[0].observed_at = Some(999);
        assert_eq!(compute_observation_id(&a), compute_observation_id(&b));
    }

    #[test]
    fn observation_id_changes_when_content_changes() {
        let mut a = NetworkObservation::default();
        a.rtcp_port = Some(2980);
        let mut b = a.clone();
        b.rtcp_port = Some(2981);
        assert_ne!(compute_observation_id(&a), compute_observation_id(&b));
    }

    #[tokio::test]
    async fn observer_bumps_generation_only_when_changed() {
        let mut observer = NetworkObserver::new(NetworkObserverConfig {
            ipv6_probe_target: "127.0.0.1:1".to_string(),
            ipv6_probe_timeout: Duration::from_millis(50),
            direct_probe_timeout: Duration::from_millis(50),
            direct_probe_freshness_ttl_secs: 600,
            probe_source: "test".to_string(),
        });
        let mut device_doc = name_lib::DeviceConfig::new("ood1", "x".to_string());
        device_doc.rtcp_port = Some(2980);
        let peers = HashMap::new();
        let lan = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 23));
        let obs1 = observer.observe(&device_doc, &[lan], &peers).await;
        let obs2 = observer.observe(&device_doc, &[lan], &peers).await;
        assert_eq!(obs1.generation, obs2.generation);
        assert_eq!(obs1.observation_id, obs2.observation_id);
        let lan2 = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 24));
        let obs3 = observer.observe(&device_doc, &[lan, lan2], &peers).await;
        assert_eq!(obs3.generation, obs2.generation.map(|g| g + 1));
        assert_ne!(obs3.observation_id, obs2.observation_id);
    }
}
