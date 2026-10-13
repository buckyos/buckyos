//! Read-only Zone topology for the Settings app: the devices (nodes) of the Zone and the TLS
//! certificates the gateway on this node serves.

use crate::ControlPanelServer;
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult};
use buckyos_api::{
    get_buckyos_api_runtime, zone_document_hostname, NetworkObservation, NETWORK_OBSERVATION_KEY,
};
use buckyos_kit::buckyos_get_unix_timestamp;
use chrono::{DateTime, Utc};
use name_client::{IdentityMaterial, IdentityRoots, IdentityStatus, IdentityUsage, X509Metadata};
use name_lib::{DeviceInfo, DeviceNodeType, OODDescriptionString};
use serde_json::{json, Value};

/// node_daemon rewrites `devices/{name}/info` about every 30 seconds; a node that has been
/// silent for longer than this is reported offline.
const DEVICE_ONLINE_WINDOW_SECS: u64 = 120;

fn ood_role(node_type: &DeviceNodeType) -> &'static str {
    match node_type {
        DeviceNodeType::OOD | DeviceNodeType::OODOnly => "ood",
        DeviceNodeType::Gateway => "gateway",
        _ => "device",
    }
}

fn non_empty(value: Option<&String>) -> Option<String> {
    value
        .map(|value| value.trim())
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("none"))
        .map(ToOwned::to_owned)
}

/// One `zone.devices.list` entry. `info` is `devices/{name}/info` as reported by the node's
/// node_daemon; it is absent for an OOD listed in the ZoneDocument that never reported.
fn device_entry(
    name: &str,
    info: Result<&DeviceInfo, &str>,
    ood: Option<&OODDescriptionString>,
    is_local: bool,
    now: u64,
) -> Value {
    let role = ood.map(|ood| ood_role(&ood.node_type));
    let Ok(info) = info else {
        return json!({
            "name": name,
            "role": role.unwrap_or("device"),
            "is_ood": ood.is_some(),
            "is_local": is_local,
            "online": false,
            "net_id": ood.and_then(|ood| ood.net_id.clone()),
            "ips": ood.and_then(|ood| ood.ip).map(|ip| vec![ip.to_string()]).unwrap_or_default(),
            "info_error": info.err(),
        });
    };

    let doc = &info.device_doc;
    let online = now.saturating_sub(info.update_time) <= DEVICE_ONLINE_WINDOW_SECS;
    let observation = doc
        .extra_info
        .get(NETWORK_OBSERVATION_KEY)
        .and_then(|value| serde_json::from_value::<NetworkObservation>(value.clone()).ok());
    let cpu_usage_percent = match (info.cpu_usage, info.cpu_num) {
        // `cpu_usage` is the sum of per-core percentages (0..=cores*100).
        (Some(usage), Some(cores)) if cores > 0 => Some((usage / cores as f32).clamp(0.0, 100.0)),
        _ => None,
    };
    let gpu = non_empty(info.gpu_info.as_ref()).map(|model| {
        json!({
            "model": model,
            "total_mem_bytes": info.gpu_total_mem,
            "used_mem_bytes": info.gpu_used_mem,
            "load_percent": info.gpu_load,
            "tflops": info.gpu_tflops,
        })
    });

    json!({
        "name": name,
        "did": doc.id.to_string(),
        "device_type": doc.device_type,
        "role": role.unwrap_or(if doc.device_type.is_empty() { "device" } else { doc.device_type.as_str() }),
        "is_ood": ood.is_some(),
        "is_local": is_local,
        "online": online,
        "state": info.state,
        "last_seen": info.update_time,
        "hostname": non_empty(info.sys_hostname.as_ref()),
        "arch": info.arch,
        "os": info.os,
        "os_version": non_empty(info.base_os_info.as_ref()),
        "net_id": doc.net_id.clone().or_else(|| ood.and_then(|ood| ood.net_id.clone())),
        "ips": info.merged_ips().iter().map(ToString::to_string).collect::<Vec<_>>(),
        "endpoints": observation.as_ref().map(|item| item.endpoints.clone()).unwrap_or_default(),
        "ipv6_state": observation.as_ref().and_then(|item| item.ipv6.as_ref()).and_then(|ipv6| ipv6.state.clone()),
        "rtcp_port": doc.rtcp_port,
        "cpu": {
            "model": non_empty(info.cpu_info.as_ref()),
            "cores": info.cpu_num,
            "mhz": info.cpu_mhz,
            "usage_percent": cpu_usage_percent,
        },
        "memory": { "total_bytes": info.total_mem, "used_bytes": info.mem_usage },
        "storage": { "total_bytes": info.total_space, "used_bytes": info.disk_usage },
        "gpu": gpu,
    })
}

/// OODs first in ZoneDocument order, then the remaining devices by name.
fn ordered_device_names(oods: &[OODDescriptionString], reported: &[String]) -> Vec<String> {
    let mut names: Vec<String> = oods.iter().map(|ood| ood.name.clone()).collect();
    let mut others: Vec<String> = reported
        .iter()
        .filter(|name| !names.contains(name))
        .cloned()
        .collect();
    others.sort();
    others.dedup();
    names.extend(others);
    names
}

fn certificate_entry(
    host: &str,
    metadata: &X509Metadata,
    status: Option<&IdentityStatus>,
    cert_pem: Option<String>,
    now: DateTime<Utc>,
) -> Value {
    let certificate = metadata.certificate.clone().unwrap_or_default();
    let not_after = DateTime::parse_from_rfc3339(&certificate.not_after)
        .ok()
        .map(|time| time.with_timezone(&Utc));
    let not_before = certificate
        .not_before
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|time| time.with_timezone(&Utc));
    let state = match (not_before, not_after) {
        (_, Some(not_after)) if now > not_after => "expired",
        (Some(not_before), _) if now < not_before => "not_yet_valid",
        (_, None) => "unknown",
        _ if status.is_some_and(|status| !status.locally_usable) => "unusable",
        _ => "valid",
    };
    let renewal = metadata.renewal.as_ref();
    json!({
        "host": host,
        "match_type": status.and_then(|status| status.match_type.clone()),
        "source": renewal
            .and_then(|value| value.get("manager"))
            .and_then(Value::as_str)
            .unwrap_or("custom"),
        "issuer": certificate.issuer,
        "subject": certificate.subject,
        "serial_number": certificate.serial_number,
        "not_before": certificate.not_before,
        "not_after": certificate.not_after,
        "days_remaining": not_after.map(|time| (time - now).num_days()),
        "dns_names": metadata.san.as_ref().map(|san| san.dns.clone()).unwrap_or_default(),
        "fingerprint_sha256": certificate.fingerprint_sha256,
        "last_renewed_at": renewal.and_then(|value| value.get("last_renewed_at")).cloned(),
        "state": state,
        "cert_pem": cert_pem,
    })
}

impl ControlPanelServer {
    /// `zone.devices.list`: every device registered under `devices/` plus the OODs named by the
    /// ZoneDocument, with the runtime facts node_daemon reports for each.
    pub(crate) async fn handle_zone_devices_list(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        let zone_document = runtime
            .get_zone_config()
            .and_then(|config| config.zone_document().ok());
        let oods = zone_document
            .as_ref()
            .map(|document| document.oods.clone())
            .unwrap_or_default();
        let local_name = runtime
            .device_config
            .as_ref()
            .map(|doc| doc.name.clone())
            .unwrap_or_default();
        let reported = client
            .list("devices")
            .await
            .map_err(|error| RPCErrors::ReasonError(format!("list devices failed: {error}")))?;

        let now = buckyos_get_unix_timestamp();
        let mut devices = Vec::new();
        for name in ordered_device_names(&oods, &reported) {
            let ood = oods.iter().find(|ood| ood.name == name);
            let info = match client.get(format!("devices/{name}/info").as_str()).await {
                Ok(stored) => serde_json::from_str::<DeviceInfo>(&stored.value)
                    .map_err(|error| format!("invalid device info: {error}")),
                Err(error) => Err(format!("device info unavailable: {error}")),
            };
            devices.push(device_entry(
                &name,
                info.as_ref().map_err(String::as_str),
                ood,
                name == local_name,
                now,
            ));
        }

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "zone_did": runtime.zone_id.to_string(),
                "hostname": zone_document.as_ref().map(zone_document_hostname),
                "owner_did": zone_document.as_ref().map(|document| document.owner.to_string()),
                "sn": zone_document.as_ref().and_then(|document| document.sn.clone()),
                "local_device": local_name,
                "online_window_secs": DEVICE_ONLINE_WINDOW_SECS,
                "devices": devices,
            })),
            req.seq,
        ))
    }

    /// `zone.certificates.list`: the server certificates installed on this node for the Zone
    /// hostname and its wildcard, read from the identity store the gateway's ACME manager writes.
    pub(crate) async fn handle_zone_certificates_list(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        let hostname = runtime
            .get_zone_config()
            .and_then(|config| config.zone_document().ok())
            .map(|document| zone_document_hostname(&document))
            .unwrap_or_else(|| runtime.zone_id.to_host_name());
        let roots = IdentityRoots::from_env_or_buckyos_root()
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;

        let now = Utc::now();
        let mut certificates = Vec::new();
        for host in [hostname.clone(), format!("*.{hostname}")] {
            let Ok(metadata) = roots.parse_x509_metadata(&host, IdentityUsage::Server) else {
                continue;
            };
            let status = roots
                .check_x509_local_status(&host, IdentityUsage::Server)
                .ok();
            let cert_pem = [IdentityMaterial::Cert, IdentityMaterial::Fullchain]
                .into_iter()
                .find_map(|material| {
                    roots
                        .find_public_file(&host, IdentityUsage::Server, material)
                        .ok()
                        .and_then(|found| std::fs::read_to_string(found.path).ok())
                });
            certificates.push(certificate_entry(
                &host,
                &metadata,
                status.as_ref(),
                cert_pem,
                now,
            ));
        }

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "hostname": hostname,
                "node": runtime.device_config.as_ref().map(|doc| doc.name.clone()),
                "certificates": certificates,
            })),
            req.seq,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use name_client::{X509CertificateMetadata, X509SanMetadata};

    fn sample_info(update_time: u64) -> DeviceInfo {
        let mut doc = name_lib::DeviceDocument::new(
            "ood1",
            "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8".to_string(),
        );
        doc.device_type = "ood".to_string();
        doc.extra_info.insert(
            NETWORK_OBSERVATION_KEY.to_string(),
            json!({
                "ipv6": { "state": "available" },
                "endpoints": [{ "ip": "203.0.113.7", "scope": "global" }],
            }),
        );
        let mut info = DeviceInfo::from_device_doc(&doc);
        info.update_time = update_time;
        info.cpu_num = Some(4);
        info.cpu_usage = Some(200.0);
        info.total_mem = Some(8 << 30);
        info.gpu_info = Some(String::new());
        info
    }

    #[test]
    fn device_entry_reports_online_usage_and_network() {
        let info = sample_info(1_000);
        let ood = OODDescriptionString::new("ood1".into(), DeviceNodeType::OOD, None, None);
        let entry = device_entry("ood1", Ok(&info), Some(&ood), true, 1_060);
        assert_eq!(entry["online"], true);
        assert_eq!(entry["role"], "ood");
        assert_eq!(entry["is_local"], true);
        assert_eq!(entry["cpu"]["usage_percent"], 50.0);
        assert_eq!(entry["memory"]["total_bytes"], 8u64 << 30);
        assert_eq!(entry["gpu"], Value::Null);
        assert_eq!(entry["ipv6_state"], "available");
        assert_eq!(entry["endpoints"][0]["scope"], "global");

        let stale = device_entry("ood1", Ok(&info), Some(&ood), true, 1_000 + 121);
        assert_eq!(stale["online"], false);
    }

    #[test]
    fn device_entry_without_info_is_offline() {
        let ood = OODDescriptionString::new(
            "ood2".into(),
            DeviceNodeType::Gateway,
            Some("wan".into()),
            None,
        );
        let entry = device_entry(
            "ood2",
            Err("device info unavailable"),
            Some(&ood),
            false,
            10,
        );
        assert_eq!(entry["online"], false);
        assert_eq!(entry["role"], "gateway");
        assert_eq!(entry["net_id"], "wan");
        assert_eq!(entry["info_error"], "device info unavailable");
    }

    #[test]
    fn device_names_put_oods_first() {
        let oods = vec![
            OODDescriptionString::new("ood2".into(), DeviceNodeType::OOD, None, None),
            OODDescriptionString::new("ood1".into(), DeviceNodeType::OOD, None, None),
        ];
        let reported = vec![
            "phone".to_string(),
            "ood1".to_string(),
            "laptop".to_string(),
        ];
        assert_eq!(
            ordered_device_names(&oods, &reported),
            vec!["ood2", "ood1", "laptop", "phone"]
        );
    }

    #[test]
    fn certificate_entry_classifies_validity() {
        let metadata = X509Metadata {
            schema: None,
            did: "did:web:alice.example.com".into(),
            raw_host_uri: "alice.example.com".into(),
            dir_name: "alice.example.com".into(),
            usage: IdentityUsage::Server,
            match_info: None,
            certificate: Some(X509CertificateMetadata {
                issuer: Some("CN=R11".into()),
                not_before: Some("2026-01-01T00:00:00Z".into()),
                not_after: "2026-04-01T00:00:00Z".into(),
                ..Default::default()
            }),
            san: Some(X509SanMetadata {
                dns: vec!["alice.example.com".into()],
                uri: vec![],
            }),
            paths: None,
            did_binding: None,
            renewal: Some(json!({ "manager": "acme", "last_renewed_at": "2026-01-01T00:00:00Z" })),
            updated_at: None,
            generation: None,
        };
        let at = |value: &str| {
            DateTime::parse_from_rfc3339(value)
                .unwrap()
                .with_timezone(&Utc)
        };
        let valid = certificate_entry(
            "alice.example.com",
            &metadata,
            None,
            None,
            at("2026-03-01T00:00:00Z"),
        );
        assert_eq!(valid["state"], "valid");
        assert_eq!(valid["source"], "acme");
        assert_eq!(valid["days_remaining"], 31);
        let expired = certificate_entry(
            "alice.example.com",
            &metadata,
            None,
            None,
            at("2026-05-01T00:00:00Z"),
        );
        assert_eq!(expired["state"], "expired");
    }
}
