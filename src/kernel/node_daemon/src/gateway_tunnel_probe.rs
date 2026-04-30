use std::collections::HashMap;
use std::time::Duration;

use buckyos_api::{ProbeInfo, DEFAULT_RTCP_PORT};
use buckyos_kit::{buckyos_get_unix_timestamp, get_buckyos_service_data_dir};
use cyfs_gateway_lib::{
    GatewayControlClient, TunnelProbeOptions, TunnelUrlSortPolicy, TunnelUrlStatus, CONTROL_SERVER,
};
use kRPC::RPCSessionToken;
use log::*;
use name_lib::{load_private_key, DeviceInfo};

const GATEWAY_PROBE_RPC_TIMEOUT_SECS: u64 = 3;
const GATEWAY_PROBE_TIMEOUT_MS: u64 = 2_000;
// Accept history up to 30s old; matches the default reachable_ttl in
// cyfs-gateway tunnel_mgr so we usually piggy-back on keep_tunnel/business
// signals instead of triggering fresh probes from the observation loop.
const GATEWAY_PROBE_MAX_AGE_MS: u64 = 30_000;

fn build_local_gateway_token() -> Option<String> {
    // Mirrors `read_login_token(CONTROL_SERVER)` in cyfs-gateway: sign a
    // short-lived JWT with the gateway's local private key. We replicate
    // the call instead of depending on the gateway crate to keep the dep
    // tree small and avoid a circular workspace edge.
    let private_key_path = get_buckyos_service_data_dir("cyfs_gateway")
        .join("token_key")
        .join("private_key.pem");
    let encoding_key = match load_private_key(private_key_path.as_path()) {
        Ok(key) => key,
        Err(err) => {
            debug!(
                "load gateway private key for tunnel probe failed ({}): {}",
                private_key_path.display(),
                err
            );
            return None;
        }
    };
    match RPCSessionToken::generate_jwt_token("root", "cyfs-gateway", None, &encoding_key) {
        Ok((token, _)) => Some(token),
        Err(err) => {
            debug!(
                "generate gateway control jwt for tunnel probe failed: {}",
                err
            );
            None
        }
    }
}

fn format_rtcp_did_url(peer_node_id: &str, zone_host: &str, port: u32) -> String {
    if port == DEFAULT_RTCP_PORT {
        format!("rtcp://{}.{}/", peer_node_id, zone_host)
    } else {
        format!("rtcp://{}.{}:{}/", peer_node_id, zone_host, port)
    }
}

fn ms_to_secs(ms: u64) -> u64 {
    ms / 1_000
}

/// Query cyfs-gateway tunnel_mgr for the live status of every key tunnel
/// (one DID-form RTCP URL per zone peer). Best-effort: if cyfs-gateway is
/// not yet running (boot phase) or we cannot mint a control token, returns
/// an empty vector and the caller falls back to the existing TCP probes.
pub async fn probe_key_tunnels_via_gateway(
    self_node_id: &str,
    zone_host: Option<&str>,
    peers: &HashMap<String, DeviceInfo>,
) -> Vec<ProbeInfo> {
    let zone_host = match zone_host {
        Some(h) if !h.is_empty() => h,
        _ => {
            debug!("zone_host unavailable, skip cyfs-gateway tunnel_mgr probe");
            return Vec::new();
        }
    };

    let mut url_to_peer: Vec<(String, String)> = Vec::new();
    for (name, peer) in peers.iter() {
        if name == self_node_id {
            continue;
        }
        let port = peer.device_doc.rtcp_port.unwrap_or(DEFAULT_RTCP_PORT);
        let url = format_rtcp_did_url(name.as_str(), zone_host, port);
        url_to_peer.push((url, name.clone()));
    }
    if url_to_peer.is_empty() {
        return Vec::new();
    }

    let token = match build_local_gateway_token() {
        Some(t) => t,
        None => {
            debug!("skip cyfs-gateway tunnel_mgr probe: no gateway control token");
            return Vec::new();
        }
    };

    let urls: Vec<String> = url_to_peer.iter().map(|(u, _)| u.clone()).collect();
    let options = TunnelProbeOptions {
        force_probe: false,
        max_age_ms: Some(GATEWAY_PROBE_MAX_AGE_MS),
        timeout_ms: Some(GATEWAY_PROBE_TIMEOUT_MS),
        sort: TunnelUrlSortPolicy::None,
        include_unsupported: true,
        caller_priorities: None,
    };

    let client = GatewayControlClient::new(CONTROL_SERVER, Some(token));
    // GatewayControlClient::query_tunnel_url_statuses uses a kRPC client
    // with no built-in RPC timeout, so guard the call here. The probe
    // itself honors `options.timeout_ms`; this outer bound just ensures
    // a hung gateway can never stall the observation loop.
    let resp = match tokio::time::timeout(
        Duration::from_secs(GATEWAY_PROBE_RPC_TIMEOUT_SECS),
        client.query_tunnel_url_statuses(&urls, options),
    )
    .await
    {
        Ok(Ok(v)) => v,
        Ok(Err(err)) => {
            debug!(
                "cyfs-gateway tunnel_mgr probe rpc failed (gateway may not be ready): {:?}",
                err
            );
            return Vec::new();
        }
        Err(_) => {
            debug!(
                "cyfs-gateway tunnel_mgr probe rpc timed out after {}s",
                GATEWAY_PROBE_RPC_TIMEOUT_SECS
            );
            return Vec::new();
        }
    };

    let statuses: Vec<TunnelUrlStatus> = match resp.get("statuses") {
        Some(arr) => match serde_json::from_value(arr.clone()) {
            Ok(v) => v,
            Err(err) => {
                warn!(
                    "cyfs-gateway tunnel_mgr probe response 'statuses' decode failed: {}",
                    err
                );
                return Vec::new();
            }
        },
        None => {
            warn!("cyfs-gateway tunnel_mgr probe response missing 'statuses' array");
            return Vec::new();
        }
    };

    let now_secs = buckyos_get_unix_timestamp();
    let mut results: Vec<ProbeInfo> = Vec::with_capacity(statuses.len());
    for status in statuses {
        let target_node = url_to_peer
            .iter()
            .find(|(u, _)| u == &status.url || u == &status.normalized_url)
            .map(|(_, n)| n.clone());

        let last_probe = if status.observed_at_ms > 0 {
            Some(ms_to_secs(status.observed_at_ms))
        } else {
            Some(now_secs)
        };

        results.push(ProbeInfo {
            target_node,
            kind: Some("rtcp_tunnel".to_string()),
            url: Some(status.url),
            status: Some(status.state.as_str().to_string()),
            rtt_ms: status.rtt_ms,
            last_probe,
            last_success: status.last_success_at_ms.map(ms_to_secs),
            freshness_ttl_secs: None,
            failure_reason: status.failure_reason,
            source: Some(format!("gateway_tunnel_mgr:{}", status.source.as_str())),
        });
    }
    results.sort_by(|a, b| a.target_node.cmp(&b.target_node));
    results
}
