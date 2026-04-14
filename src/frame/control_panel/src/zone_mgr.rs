use crate::{
    docker_command, external_command, ControlPanelServer, DockerOverviewCacheEntry,
    GATEWAY_CONFIG_FILES, GATEWAY_ETC_DIR, SN_SELF_CERT_STATE_PATH, ZONE_CONFIG_FILES,
};
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Instant;

impl ControlPanelServer {
    fn gateway_file_summary(path: &Path) -> Value {
        let metadata = std::fs::metadata(path).ok();
        let size_bytes = metadata.as_ref().map(|meta| meta.len()).unwrap_or(0);
        let modified_at = metadata
            .as_ref()
            .and_then(|meta| meta.modified().ok())
            .map(|time| DateTime::<Utc>::from(time).to_rfc3339())
            .unwrap_or_default();

        json!({
            "name": path.file_name().and_then(|value| value.to_str()).unwrap_or(""),
            "path": path.display().to_string(),
            "exists": path.exists(),
            "sizeBytes": size_bytes,
            "modifiedAt": modified_at,
        })
    }

    fn gateway_config_file_path(name: &str) -> Option<PathBuf> {
        if !GATEWAY_CONFIG_FILES.contains(&name) {
            return None;
        }
        Some(Path::new(GATEWAY_ETC_DIR).join(name))
    }

    fn zone_config_file_path(name: &str) -> Option<PathBuf> {
        if !ZONE_CONFIG_FILES.contains(&name) {
            return None;
        }
        Some(Path::new(GATEWAY_ETC_DIR).join(name))
    }

    fn extract_first_quoted_after(value: &str, marker: &str) -> Option<String> {
        let marker_index = value.find(marker)?;
        let tail = &value[marker_index + marker.len()..];
        let quote_start = tail.find('"')?;
        let quoted_tail = &tail[quote_start + 1..];
        let quote_end = quoted_tail.find('"')?;
        Some(quoted_tail[..quote_end].to_string())
    }

    fn parse_gateway_route_rules(block: &str) -> Vec<Value> {
        let mut rules: Vec<Value> = Vec::new();

        for raw_line in block.lines() {
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }

            let kind = if line.contains("match ${REQ.path}") {
                "path"
            } else if line.contains("match ${REQ.host}") {
                "host"
            } else if line.starts_with("return ") {
                "fallback"
            } else {
                "logic"
            };

            let matcher = if kind == "path" || kind == "host" {
                Self::extract_first_quoted_after(line, "match ").unwrap_or_default()
            } else {
                "".to_string()
            };

            let action = Self::extract_first_quoted_after(line, "return ").unwrap_or_default();

            rules.push(json!({
                "kind": kind,
                "matcher": matcher,
                "action": action,
                "raw": line,
            }));
        }

        rules
    }

    fn parse_boot_gateway_stacks(yaml: &str) -> Vec<Value> {
        let mut stacks: Vec<Value> = Vec::new();
        let mut current_name: Option<String> = None;
        let mut current_id = String::new();
        let mut current_protocol = String::new();
        let mut current_bind = String::new();

        let flush_current = |stacks: &mut Vec<Value>,
                             current_name: &mut Option<String>,
                             current_id: &mut String,
                             current_protocol: &mut String,
                             current_bind: &mut String| {
            if let Some(name) = current_name.take() {
                stacks.push(json!({
                    "name": name,
                    "id": current_id.clone(),
                    "protocol": current_protocol.clone(),
                    "bind": current_bind.clone(),
                }));
            }
            current_id.clear();
            current_protocol.clear();
            current_bind.clear();
        };

        let mut in_stacks = false;
        for raw_line in yaml.lines() {
            let line = raw_line.trim_end();
            if line.trim().is_empty() || line.trim_start().starts_with('#') {
                continue;
            }

            let indent = raw_line
                .chars()
                .take_while(|ch| ch.is_ascii_whitespace())
                .count();
            let trimmed = line.trim();

            if trimmed == "stacks:" {
                in_stacks = true;
                continue;
            }

            if !in_stacks {
                continue;
            }

            if indent == 0 || trimmed == "global_process_chains:" {
                break;
            }

            if indent == 2 && trimmed.ends_with(':') {
                flush_current(
                    &mut stacks,
                    &mut current_name,
                    &mut current_id,
                    &mut current_protocol,
                    &mut current_bind,
                );
                current_name = Some(trimmed.trim_end_matches(':').to_string());
                continue;
            }

            if current_name.is_none() {
                continue;
            }

            if indent == 4 {
                if let Some(value) = trimmed.strip_prefix("id:") {
                    current_id = value.trim().to_string();
                    continue;
                }
                if let Some(value) = trimmed.strip_prefix("protocol:") {
                    current_protocol = value.trim().to_string();
                    continue;
                }
                if let Some(value) = trimmed.strip_prefix("bind:") {
                    current_bind = value.trim().to_string();
                    continue;
                }
            }
        }

        flush_current(
            &mut stacks,
            &mut current_name,
            &mut current_id,
            &mut current_protocol,
            &mut current_bind,
        );

        stacks
    }

    fn extract_host_from_url(raw: &str) -> Option<String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }

        let normalized = if trimmed.contains("://") {
            trimmed.to_string()
        } else {
            format!("https://{}", trimmed)
        };

        url::Url::parse(normalized.as_str())
            .ok()
            .and_then(|value| value.host_str().map(|host| host.to_string()))
            .map(|value| value.trim().trim_matches('.').to_string())
            .filter(|value| !value.is_empty())
    }

    fn query_dig_short_records(
        server: Option<&str>,
        record_name: &str,
        record_type: &str,
    ) -> Result<Vec<String>, String> {
        let mut cmd = external_command("dig");
        cmd.arg("+short");

        if let Some(server) = server
            .map(|item| item.trim())
            .filter(|item| !item.is_empty())
        {
            cmd.arg(format!("@{}", server));
        }

        let output = cmd
            .arg(record_name)
            .arg(record_type)
            .output()
            .map_err(|err| {
                if err.kind() == std::io::ErrorKind::NotFound {
                    "dig command not found. Please install dnsutils/bind-tools.".to_string()
                } else {
                    format!("failed to execute dig: {}", err)
                }
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if stderr.is_empty() {
                return Err(format!(
                    "dig {} {} failed with status {}",
                    record_name, record_type, output.status
                ));
            }
            return Err(stderr);
        }

        let records = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|line| line.trim().to_string())
            .filter(|line| !line.is_empty())
            .collect::<Vec<String>>();

        Ok(records)
    }

    fn self_cert_state_matches_domain(cert_domain: &str, zone_domain: &str) -> bool {
        let cert_domain = cert_domain.trim().trim_matches('.').to_lowercase();
        let zone_domain = zone_domain.trim().trim_matches('.').to_lowercase();
        if cert_domain.is_empty() || zone_domain.is_empty() {
            return false;
        }

        if cert_domain == zone_domain {
            return true;
        }

        cert_domain
            .strip_prefix("*.")
            .map(|suffix| {
                zone_domain == suffix || zone_domain.ends_with(format!(".{}", suffix).as_str())
            })
            .unwrap_or(false)
    }

    fn read_self_cert_state(zone_domain: &str) -> Result<Option<bool>, String> {
        let content = std::fs::read_to_string(SN_SELF_CERT_STATE_PATH)
            .map_err(|err| format!("read self cert state failed: {}", err))?;
        let parsed = serde_json::from_str::<Value>(content.as_str())
            .map_err(|err| format!("parse self cert state failed: {}", err))?;
        let items = parsed
            .as_array()
            .ok_or_else(|| "self cert state is not an array".to_string())?;

        let mut wildcard_state: Option<bool> = None;
        for item in items {
            let domain = item
                .get("domain")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            let state = item.get("state").and_then(|value| value.as_bool());
            let Some(state) = state else {
                continue;
            };

            if domain.trim().eq_ignore_ascii_case(zone_domain.trim()) {
                return Ok(Some(state));
            }

            if wildcard_state.is_none() && Self::self_cert_state_matches_domain(domain, zone_domain)
            {
                wildcard_state = Some(state);
            }
        }

        Ok(wildcard_state)
    }

    pub(crate) async fn handle_zone_overview(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let start_config_path = Self::zone_config_file_path("start_config.json")
            .unwrap_or_else(|| Path::new(GATEWAY_ETC_DIR).join("start_config.json"));
        let node_device_config_path = Self::zone_config_file_path("node_device_config.json")
            .unwrap_or_else(|| Path::new(GATEWAY_ETC_DIR).join("node_device_config.json"));
        let node_identity_path = Self::zone_config_file_path("node_identity.json")
            .unwrap_or_else(|| Path::new(GATEWAY_ETC_DIR).join("node_identity.json"));

        let files = vec![
            Self::gateway_file_summary(&start_config_path),
            Self::gateway_file_summary(&node_device_config_path),
            Self::gateway_file_summary(&node_identity_path),
        ];

        let mut zone_name = String::new();
        let mut zone_domain = String::new();
        let mut zone_did = String::new();
        let mut owner_did = String::new();
        let mut user_name = String::new();
        let mut device_name = String::new();
        let mut device_did = String::new();
        let mut device_type = String::new();
        let mut net_id = String::new();
        let mut sn_url = String::new();
        let mut sn_username = String::new();
        let mut sn_ip = String::new();
        let mut sn_dns_a_records: Vec<String> = Vec::new();
        let mut sn_dns_txt_records: Vec<String> = Vec::new();
        let mut sn_dig_error = String::new();
        let mut self_cert_state = false;
        let self_cert_state_source = SN_SELF_CERT_STATE_PATH.to_string();
        let mut zone_iat: i64 = 0;
        let mut notes: Vec<String> = Vec::new();

        if let Ok(content) = std::fs::read_to_string(&start_config_path) {
            if let Ok(value) = serde_json::from_str::<Value>(content.as_str()) {
                zone_domain = value
                    .get("zone_name")
                    .and_then(|item| item.as_str())
                    .unwrap_or_default()
                    .to_string();
                user_name = value
                    .get("user_name")
                    .and_then(|item| item.as_str())
                    .unwrap_or_default()
                    .to_string();
                sn_url = value
                    .get("sn_url")
                    .and_then(|item| item.as_str())
                    .unwrap_or_default()
                    .to_string();
                sn_username = value
                    .get("sn_username")
                    .and_then(|item| item.as_str())
                    .unwrap_or_default()
                    .to_string();
                if net_id.is_empty() {
                    net_id = value
                        .get("net_id")
                        .and_then(|item| item.as_str())
                        .unwrap_or_default()
                        .to_string();
                }
            }
        }

        if let Ok(content) = std::fs::read_to_string(&node_device_config_path) {
            if let Ok(value) = serde_json::from_str::<Value>(content.as_str()) {
                device_did = value
                    .get("id")
                    .and_then(|item| item.as_str())
                    .unwrap_or_default()
                    .to_string();
                device_name = value
                    .get("name")
                    .and_then(|item| item.as_str())
                    .unwrap_or_default()
                    .to_string();
                device_type = value
                    .get("device_type")
                    .and_then(|item| item.as_str())
                    .unwrap_or_default()
                    .to_string();
                if zone_did.is_empty() {
                    zone_did = value
                        .get("zone_did")
                        .and_then(|item| item.as_str())
                        .unwrap_or_default()
                        .to_string();
                }
                if owner_did.is_empty() {
                    owner_did = value
                        .get("owner")
                        .and_then(|item| item.as_str())
                        .unwrap_or_default()
                        .to_string();
                }
                if net_id.is_empty() {
                    net_id = value
                        .get("net_id")
                        .and_then(|item| item.as_str())
                        .unwrap_or_default()
                        .to_string();
                }
            }
        }

        if let Ok(content) = std::fs::read_to_string(&node_identity_path) {
            if let Ok(value) = serde_json::from_str::<Value>(content.as_str()) {
                if zone_did.is_empty() {
                    zone_did = value
                        .get("zone_did")
                        .and_then(|item| item.as_str())
                        .unwrap_or_default()
                        .to_string();
                }
                if owner_did.is_empty() {
                    owner_did = value
                        .get("owner_did")
                        .and_then(|item| item.as_str())
                        .unwrap_or_default()
                        .to_string();
                }
                zone_iat = value
                    .get("zone_iat")
                    .and_then(|item| item.as_i64())
                    .unwrap_or(0);
            }
        }

        if zone_name.is_empty() {
            zone_name = Self::parse_zone_name_from_did(zone_did.as_str()).unwrap_or_default();
        }

        if zone_domain.is_empty() && !zone_name.is_empty() {
            zone_domain = format!("{}.web3.buckyos.ai", zone_name);
        }

        if zone_name.is_empty() {
            notes.push("zone name not found in start_config.json or zone_did".to_string());
        }
        if zone_did.is_empty() {
            notes.push(
                "zone_did not found in node_device_config.json/node_identity.json".to_string(),
            );
        }
        if device_name.is_empty() {
            notes.push("device name not found in node_device_config.json".to_string());
        }

        let mut dig_errors: Vec<String> = Vec::new();
        let mut dig_available = true;
        let sn_host = Self::extract_host_from_url(sn_url.as_str()).unwrap_or_default();

        if sn_host.is_empty() {
            notes.push("SN host cannot be parsed from sn.url".to_string());
        } else {
            match Self::query_dig_short_records(None, sn_host.as_str(), "A") {
                Ok(records) => {
                    if let Some(first_ip) = records.first() {
                        sn_ip = first_ip.to_string();
                    }
                }
                Err(err) => {
                    dig_available = !err.contains("dig command not found");
                    dig_errors.push(format!("resolve SN host A failed: {}", err));
                }
            }
        }

        if !zone_domain.is_empty() && dig_available {
            let dns_server = if !sn_ip.is_empty() {
                Some(sn_ip.as_str())
            } else if !sn_host.is_empty() {
                Some(sn_host.as_str())
            } else {
                None
            };

            if let Some(server) = dns_server {
                match Self::query_dig_short_records(Some(server), zone_domain.as_str(), "A") {
                    Ok(records) => {
                        sn_dns_a_records = records;
                    }
                    Err(err) => {
                        dig_available = !err.contains("dig command not found");
                        dig_errors.push(format!("query zone A via SN failed: {}", err));
                    }
                }

                if dig_available {
                    match Self::query_dig_short_records(Some(server), zone_domain.as_str(), "TXT") {
                        Ok(records) => {
                            sn_dns_txt_records = records;
                        }
                        Err(err) => {
                            dig_errors.push(format!("query zone TXT via SN failed: {}", err));
                        }
                    }
                }
            } else {
                dig_errors.push("SN DNS server is unavailable for dig query".to_string());
            }
        }

        if !dig_errors.is_empty() {
            sn_dig_error = dig_errors.join("; ");
            notes.push(format!("SN dig diagnostics: {}", sn_dig_error));
        }

        if !zone_domain.is_empty() {
            match Self::read_self_cert_state(zone_domain.as_str()) {
                Ok(Some(state)) => {
                    self_cert_state = state;
                }
                Ok(None) => {
                    notes.push(
                        "Self cert state entry not found for current zone domain".to_string(),
                    );
                }
                Err(err) => {
                    notes.push(format!("Self cert state read failed: {}", err));
                }
            }
        }

        let response = json!({
            "etcDir": GATEWAY_ETC_DIR,
            "zone": {
                "name": zone_name,
                "domain": zone_domain,
                "did": zone_did,
                "ownerDid": owner_did,
                "userName": user_name,
                "zoneIat": zone_iat,
            },
            "device": {
                "name": device_name,
                "did": device_did,
                "type": device_type,
                "netId": net_id,
            },
            "sn": {
                "url": sn_url,
                "username": sn_username,
                "host": sn_host,
                "ip": sn_ip,
                "dnsARecords": sn_dns_a_records,
                "dnsTxtRecords": sn_dns_txt_records,
                "digError": sn_dig_error,
                "selfCertState": self_cert_state,
                "selfCertStateSource": self_cert_state_source,
            },
            "files": files,
            "notes": notes,
        });

        Ok(RPCResponse::new(RPCResult::Success(response), req.seq))
    }

    pub(crate) async fn handle_gateway_overview(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let etc_dir = Path::new(GATEWAY_ETC_DIR);
        let cyfs_gateway_path = etc_dir.join("cyfs_gateway.json");
        let boot_gateway_path = etc_dir.join("boot_gateway.yaml");
        let node_gateway_path = etc_dir.join("node_gateway.json");
        let user_gateway_path = etc_dir.join("user_gateway.yaml");
        let post_gateway_path = etc_dir.join("post_gateway.yaml");

        let files = vec![
            Self::gateway_file_summary(&cyfs_gateway_path),
            Self::gateway_file_summary(&boot_gateway_path),
            Self::gateway_file_summary(&node_gateway_path),
            Self::gateway_file_summary(&user_gateway_path),
            Self::gateway_file_summary(&post_gateway_path),
        ];

        let mut includes: Vec<String> = Vec::new();
        let mut route_rules: Vec<Value> = Vec::new();
        let mut route_preview = String::new();
        let mut tls_domains: Vec<String> = Vec::new();
        let mut stacks: Vec<Value> = Vec::new();
        let mut custom_overrides: Vec<Value> = Vec::new();
        let mut notes: Vec<String> = Vec::new();

        if let Ok(content) = std::fs::read_to_string(&cyfs_gateway_path) {
            if let Ok(value) = serde_json::from_str::<Value>(content.as_str()) {
                if let Some(items) = value.get("includes").and_then(|item| item.as_array()) {
                    includes = items
                        .iter()
                        .filter_map(|item| item.get("path").and_then(|value| value.as_str()))
                        .map(|value| value.to_string())
                        .collect();
                }
            }
        }

        if let Ok(content) = std::fs::read_to_string(&node_gateway_path) {
            if let Ok(value) = serde_json::from_str::<Value>(content.as_str()) {
                if let Some(block) = value
                    .get("servers")
                    .and_then(|item| item.get("node_gateway"))
                    .and_then(|item| item.get("hook_point"))
                    .and_then(|item| item.get("main"))
                    .and_then(|item| item.get("blocks"))
                    .and_then(|item| item.get("default"))
                    .and_then(|item| item.get("block"))
                    .and_then(|item| item.as_str())
                {
                    route_rules = Self::parse_gateway_route_rules(block);
                    route_preview = block
                        .lines()
                        .take(8)
                        .map(|line| line.trim())
                        .filter(|line| !line.is_empty())
                        .collect::<Vec<&str>>()
                        .join("\n");
                }

                if let Some(certs) = value
                    .get("stacks")
                    .and_then(|item| item.get("zone_tls"))
                    .and_then(|item| item.get("certs"))
                    .and_then(|item| item.as_array())
                {
                    tls_domains = certs
                        .iter()
                        .filter_map(|item| item.get("domain").and_then(|value| value.as_str()))
                        .map(|value| value.to_string())
                        .collect();
                }
            }
        }

        if let Ok(content) = std::fs::read_to_string(&boot_gateway_path) {
            stacks = Self::parse_boot_gateway_stacks(content.as_str());
        }

        for path in [&user_gateway_path, &post_gateway_path] {
            if let Ok(content) = std::fs::read_to_string(path) {
                let normalized = content
                    .lines()
                    .map(|line| line.trim())
                    .filter(|line| !line.is_empty() && !line.starts_with('#'))
                    .collect::<Vec<&str>>()
                    .join(" ");

                if normalized != "--- {}" && normalized != "{}" {
                    custom_overrides.push(json!({
                        "name": path.file_name().and_then(|value| value.to_str()).unwrap_or(""),
                        "preview": content.lines().take(6).collect::<Vec<&str>>().join("\n"),
                    }));
                }
            }
        }

        let mode = if tls_domains
            .iter()
            .any(|domain| domain.contains("web3.buckyos.ai"))
        {
            "sn"
        } else {
            "direct"
        };

        notes.push("Gateway config loaded from /opt/buckyos/etc.".to_string());
        if custom_overrides.is_empty() {
            notes.push(
                "No user override rules detected in user_gateway.yaml/post_gateway.yaml."
                    .to_string(),
            );
        } else {
            notes.push(
                "User override rules detected; they may overwrite generated gateway blocks."
                    .to_string(),
            );
        }

        let response = json!({
            "mode": mode,
            "etcDir": GATEWAY_ETC_DIR,
            "files": files,
            "includes": includes,
            "stacks": stacks,
            "tlsDomains": tls_domains,
            "routes": route_rules,
            "routePreview": route_preview,
            "customOverrides": custom_overrides,
            "notes": notes,
        });

        Ok(RPCResponse::new(RPCResult::Success(response), req.seq))
    }

    pub(crate) async fn handle_gateway_file_get(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let name = Self::require_param_str(&req, "name")?;
        let path = Self::gateway_config_file_path(name.as_str()).ok_or_else(|| {
            RPCErrors::ParseRequestError(format!("Unsupported gateway config file: {}", name))
        })?;

        if !path.exists() {
            return Err(RPCErrors::ReasonError(format!(
                "Gateway config file not found: {}",
                path.display()
            )));
        }

        let bytes = std::fs::read(&path).map_err(|err| {
            RPCErrors::ReasonError(format!("Failed to read {}: {}", path.display(), err))
        })?;

        if bytes.len() > 2 * 1024 * 1024 {
            return Err(RPCErrors::ReasonError(format!(
                "Gateway config file too large ({} bytes)",
                bytes.len()
            )));
        }

        let content = String::from_utf8_lossy(&bytes).to_string();
        let metadata = std::fs::metadata(&path).ok();
        let modified_at = metadata
            .as_ref()
            .and_then(|meta| meta.modified().ok())
            .map(|time| DateTime::<Utc>::from(time).to_rfc3339())
            .unwrap_or_else(|| "".to_string());

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "name": name,
                "path": path.display().to_string(),
                "sizeBytes": bytes.len(),
                "modifiedAt": modified_at,
                "content": content,
            })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_container_overview(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let response = json!({
            "available": false,
            "daemonRunning": false,
            "server": {},
            "summary": {
                "total": 0,
                "running": 0,
                "paused": 0,
                "exited": 0,
                "restarting": 0,
                "dead": 0,
            },
            "containers": [],
            "notes": ["container overview disabled"],
        });

        let mut cache = self.docker_overview_cache.lock().await;
        *cache = Some(DockerOverviewCacheEntry {
            captured_at: Instant::now(),
            response: response.clone(),
        });

        Ok(RPCResponse::new(RPCResult::Success(response), req.seq))
    }

    pub(crate) async fn handle_container_action(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let id = Self::require_param_str(&req, "id")?;
        let action = Self::require_param_str(&req, "action")?;

        let docker_action = match action.as_str() {
            "start" => "start",
            "stop" => "stop",
            "restart" => "restart",
            _ => {
                return Err(RPCErrors::ParseRequestError(format!(
                    "Unsupported container action: {}",
                    action
                )));
            }
        };

        let output = docker_command()
            .arg(docker_action)
            .arg(id.as_str())
            .output()
            .map_err(|error| {
                RPCErrors::ReasonError(format!("docker {} failed: {}", docker_action, error))
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

        if !output.status.success() {
            let reason = if stderr.is_empty() {
                format!("docker {} returned non-zero exit code", docker_action)
            } else {
                stderr
            };
            return Err(RPCErrors::ReasonError(reason));
        }

        let mut cache = self.docker_overview_cache.lock().await;
        *cache = None;
        drop(cache);

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "id": id,
                "action": docker_action,
                "ok": true,
                "stdout": stdout,
            })),
            req.seq,
        ))
    }
}
