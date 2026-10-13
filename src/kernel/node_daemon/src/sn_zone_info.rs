use std::io::ErrorKind;
use std::path::Path;

use buckyos_api::atomic_write;
use buckyos_kit::get_buckyos_system_etc_dir;
use cyfs_gateway_api::SnZoneInfoResp;
use name_lib::DeviceDocument;

const SN_ZONE_INFO_FILE_NAME: &str = "sn_zone_info.json";

pub fn load_sn_zone_info() -> Result<Option<SnZoneInfoResp>, String> {
    load_sn_zone_info_in(get_buckyos_system_etc_dir().as_path())
}

fn load_sn_zone_info_in(etc_dir: &Path) -> Result<Option<SnZoneInfoResp>, String> {
    let path = etc_dir.join(SN_ZONE_INFO_FILE_NAME);
    let content = match std::fs::read_to_string(path.as_path()) {
        Ok(content) => content,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("read {} failed: {}", path.display(), error)),
    };
    serde_json::from_str(&content)
        .map(Some)
        .map_err(|error| format!("parse {} failed: {}", path.display(), error))
}

pub fn save_sn_zone_info(zone_info: &SnZoneInfoResp) -> Result<(), String> {
    save_sn_zone_info_in(get_buckyos_system_etc_dir().as_path(), zone_info)
}

fn save_sn_zone_info_in(etc_dir: &Path, zone_info: &SnZoneInfoResp) -> Result<(), String> {
    let path = etc_dir.join(SN_ZONE_INFO_FILE_NAME);
    let content = serde_json::to_vec_pretty(zone_info)
        .map_err(|error| format!("serialize {} failed: {}", path.display(), error))?;
    atomic_write(path.as_path(), &content)
}

pub fn device_requires_sn_keep_tunnel(device_doc: &DeviceDocument) -> bool {
    let declared_wan = device_doc
        .net_id
        .as_deref()
        .is_some_and(|net_id| net_id.starts_with("wan"));
    !declared_wan || device_doc.ips.is_empty()
}

pub fn relay_node_for_keep_tunnel(
    device_doc: &DeviceDocument,
    sn: Option<&str>,
    zone_info: Option<&SnZoneInfoResp>,
) -> Option<String> {
    let sn = sn?.trim();
    if sn.is_empty() {
        return None;
    }
    if !device_requires_sn_keep_tunnel(device_doc) {
        return None;
    }
    let relay_node = zone_info
        .and_then(|zone_info| {
            zone_info
                .relay_sn
                .as_deref()
                .map(str::trim)
                .filter(|relay_node| !relay_node.is_empty())
        })
        .unwrap_or(sn);
    Some(relay_node.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(net_id: Option<&str>, ips: &[&str]) -> DeviceDocument {
        let mut device = DeviceDocument::new("ood1", "test-key".to_string());
        device.net_id = net_id.map(ToString::to_string);
        device.ips = ips.iter().map(|ip| ip.parse().unwrap()).collect();
        device
    }

    fn zone_info(relay_sn: Option<&str>) -> SnZoneInfoResp {
        SnZoneInfoResp {
            code: 0,
            zone: "alice".to_string(),
            bns_name: "alice".to_string(),
            relay_sn: relay_sn.map(ToString::to_string),
            self_cert: false,
            cert_checked_at: None,
            cert_expires_at: None,
            source_version: Some("v2".to_string()),
            updated_at: 1,
        }
    }

    #[test]
    fn zone_info_round_trip_preserves_relay_node() {
        let temp_dir = tempfile::tempdir().unwrap();
        let expected = zone_info(Some("relay.example.com"));
        save_sn_zone_info_in(temp_dir.path(), &expected).unwrap();
        let loaded = load_sn_zone_info_in(temp_dir.path()).unwrap().unwrap();
        assert_eq!(loaded.zone, expected.zone);
        assert_eq!(loaded.relay_sn, expected.relay_sn);
        assert_eq!(loaded.updated_at, expected.updated_at);
    }

    #[test]
    fn addressless_wan_devices_keep_an_explicit_sn_fallback() {
        for net_id in [Some("wan"), Some("wan_dyn"), Some("wan_custom")] {
            let addressless = device(net_id, &[]);
            assert!(device_requires_sn_keep_tunnel(&addressless));
            assert_eq!(
                relay_node_for_keep_tunnel(
                    &addressless,
                    Some("sn.example.com"),
                    Some(&zone_info(Some(" relay.example.com "))),
                ),
                Some("relay.example.com".to_string())
            );
        }
    }

    #[test]
    fn signed_fixed_wan_addresses_preserve_direct_only_bootstrap() {
        for net_id in [Some("wan"), Some("wan_dyn"), Some("wan_custom")] {
            for ip in ["203.0.113.10", "2001:db8::10"] {
                let fixed = device(net_id, &[ip]);
                assert!(!device_requires_sn_keep_tunnel(&fixed));
                assert_eq!(
                    relay_node_for_keep_tunnel(
                        &fixed,
                        Some("sn.example.com"),
                        Some(&zone_info(Some("relay.example.com"))),
                    ),
                    None
                );
            }
        }
    }

    #[test]
    fn non_wan_and_unknown_topologies_retain_sn_tunnels() {
        for net_id in [None, Some(""), Some("nat"), Some("lan0"), Some("portmap")] {
            for ips in [&[][..], &["192.168.1.10"][..]] {
                let local = device(net_id, ips);
                assert!(device_requires_sn_keep_tunnel(&local));
                assert_eq!(
                    relay_node_for_keep_tunnel(&local, Some("sn.example.com"), None),
                    Some("sn.example.com".to_string())
                );
            }
        }
    }

    #[test]
    fn relay_assignment_falls_back_to_declared_sn_without_inventing_one() {
        let dynamic = device(Some("wan_dyn"), &[]);
        assert_eq!(relay_node_for_keep_tunnel(&dynamic, None, None), None);
        assert_eq!(
            relay_node_for_keep_tunnel(
                &dynamic,
                Some(" "),
                Some(&zone_info(Some("relay.example.com")))
            ),
            None
        );
        for info in [None, Some(zone_info(None)), Some(zone_info(Some(" ")))] {
            assert_eq!(
                relay_node_for_keep_tunnel(&dynamic, Some(" sn.example.com "), info.as_ref()),
                Some("sn.example.com".to_string())
            );
        }
    }
}
