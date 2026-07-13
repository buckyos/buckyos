use std::io::ErrorKind;
use std::path::Path;

use buckyos_api::atomic_write;
use buckyos_kit::get_buckyos_system_etc_dir;
use cyfs_gateway_api::SnZoneInfoResp;

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

pub fn relay_node_for_keep_tunnel(
    net_id: Option<&str>,
    sn: Option<&str>,
    zone_info: Option<&SnZoneInfoResp>,
) -> Option<String> {
    sn?;
    if net_id.is_some_and(|net_id| net_id.starts_with("wan")) {
        return None;
    }
    zone_info?
        .relay_sn
        .as_deref()
        .map(str::trim)
        .filter(|relay_node| !relay_node.is_empty())
        .map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn relay_node_is_only_used_for_non_wan_sn_keep_tunnel() {
        let info = zone_info(Some(" relay.example.com "));
        assert_eq!(
            relay_node_for_keep_tunnel(Some("nat"), Some("sn.example.com"), Some(&info)),
            Some("relay.example.com".to_string())
        );
        assert_eq!(
            relay_node_for_keep_tunnel(Some("wan_dyn"), Some("sn.example.com"), Some(&info)),
            None
        );
        assert_eq!(
            relay_node_for_keep_tunnel(Some("nat"), None, Some(&info)),
            None
        );
        assert_eq!(
            relay_node_for_keep_tunnel(
                Some("nat"),
                Some("sn.example.com"),
                Some(&zone_info(Some(" "))),
            ),
            None
        );
    }
}
