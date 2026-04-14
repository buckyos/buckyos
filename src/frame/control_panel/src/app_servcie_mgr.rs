use crate::{external_command, ControlPanelServer};
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult};
use buckyos_api::{get_buckyos_api_runtime, SystemConfigClient};
use buckyos_kit::get_buckyos_root_dir;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::task;

impl ControlPanelServer {
    pub(crate) async fn handle_apps_list(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let key = Self::param_str(&req, "key").unwrap_or_else(|| "services".to_string());
        let base_key = key.trim_end_matches('/').to_string();
        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        let items = client
            .list(&key)
            .await
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;

        let mut apps: Vec<Value> = Vec::new();
        for name in items {
            let settings_key = format!("{}/{}/settings", base_key, name);
            let settings = match client.get(&settings_key).await {
                Ok(value) => serde_json::from_str::<Value>(&value.value)
                    .unwrap_or_else(|_| json!(value.value)),
                Err(_) => Value::Null,
            };
            apps.push(json!({
                "name": name,
                "icon": "package",
                "category": "Service",
                "status": "installed",
                "version": "0.0.0",
                "settings": settings,
            }));
        }

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "key": key,
                "items": apps,
            })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_apps_version_list(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let key = Self::param_str(&req, "key").unwrap_or_else(|| "services".to_string());
        let base_key = key.trim_end_matches('/').to_string();
        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;

        let requested_names = req
            .params
            .get("names")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str())
                    .map(|name| name.trim().to_string())
                    .filter(|name| !name.is_empty())
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default();

        let names = if requested_names.is_empty() {
            client
                .list(&key)
                .await
                .map_err(|error| RPCErrors::ReasonError(error.to_string()))?
        } else {
            requested_names
        };

        let mut deduped_names = Vec::new();
        for name in names {
            if deduped_names
                .iter()
                .any(|existing: &String| existing == &name)
            {
                continue;
            }
            deduped_names.push(name);
        }

        let mut versions: Vec<Value> = Vec::new();
        for name in deduped_names {
            let version = Self::resolve_app_version(&name, &base_key, &client).await;
            versions.push(json!({
                "name": name,
                "version": version,
            }));
        }

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "key": key,
                "items": versions,
            })),
            req.seq,
        ))
    }

    async fn resolve_app_version(
        service_name: &str,
        base_key: &str,
        client: &SystemConfigClient,
    ) -> String {
        if let Some(version) = Self::probe_service_binary_version(service_name).await {
            return version;
        }

        let spec_key = format!("{}/{}/spec", base_key, service_name);
        if let Ok(value) = client.get(&spec_key).await {
            if let Some(version) = Self::parse_service_spec_version(&value.value) {
                return version;
            }
        }

        "0.0.0".to_string()
    }

    async fn probe_service_binary_version(service_name: &str) -> Option<String> {
        let candidates = Self::service_binary_candidates(service_name);
        for binary_path in candidates {
            if !binary_path.exists() {
                continue;
            }

            let candidate = binary_path.clone();
            let version = task::spawn_blocking(move || {
                Self::run_version_command_with_timeout(&candidate, Duration::from_millis(500))
            })
            .await
            .ok()
            .flatten();

            if version.is_some() {
                return version;
            }
        }

        None
    }

    fn service_binary_candidates(service_name: &str) -> Vec<PathBuf> {
        let mut candidates = Vec::new();
        let mut push_candidate = |path: PathBuf| {
            if !candidates.iter().any(|existing| existing == &path) {
                candidates.push(path);
            }
        };

        let normalized = service_name
            .split('@')
            .next()
            .unwrap_or(service_name)
            .trim();
        if normalized.is_empty() {
            return candidates;
        }

        let bin_root = get_buckyos_root_dir().join("bin");
        if normalized == "gateway" {
            push_candidate(bin_root.join("cyfs-gateway").join("cyfs_gateway"));
        }

        let dir = bin_root.join(normalized);
        let snake_name = normalized.replace('-', "_");
        push_candidate(dir.join(&snake_name));
        if snake_name != normalized {
            push_candidate(dir.join(normalized));
        }

        let kebab_name = normalized.replace('_', "-");
        if kebab_name != normalized {
            let kebab_dir = bin_root.join(&kebab_name);
            push_candidate(kebab_dir.join(&snake_name));
            push_candidate(kebab_dir.join(&kebab_name));
        }

        candidates
    }

    fn run_version_command_with_timeout(binary_path: &std::path::Path, timeout: Duration) -> Option<String> {
        let mut child = external_command(binary_path)
            .arg("--version")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .ok()?;

        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => {
                    let output = child.wait_with_output().ok()?;
                    let merged = format!(
                        "{}\n{}",
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    );
                    return Self::extract_version_from_output(&merged);
                }
                Ok(None) => {
                    if started.elapsed() >= timeout {
                        let _ = child.kill();
                        if let Ok(output) = child.wait_with_output() {
                            let merged = format!(
                                "{}\n{}",
                                String::from_utf8_lossy(&output.stdout),
                                String::from_utf8_lossy(&output.stderr)
                            );
                            if let Some(version) = Self::extract_version_from_output(&merged) {
                                return Some(version);
                            }
                        }
                        return None;
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(_) => return None,
            }
        }
    }

    fn parse_service_spec_version(raw_spec: &str) -> Option<String> {
        let parsed = serde_json::from_str::<Value>(raw_spec).ok()?;
        parsed
            .get("service_doc")
            .and_then(|service_doc| service_doc.get("version"))
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
            .or_else(|| {
                parsed
                    .get("version")
                    .and_then(|value| value.as_str())
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_string())
            })
    }

    fn extract_version_from_output(output: &str) -> Option<String> {
        let cleaned = Self::strip_ansi_codes(output);
        for raw_line in cleaned.lines() {
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }

            if let Some(index) = line.find("buckyos version ") {
                let tail = &line[index + "buckyos version ".len()..];
                if let Some(token) = tail.split_whitespace().next() {
                    if Self::is_likely_version_token(token) {
                        return Some(token.to_string());
                    }
                }
            }

            if let Some(index) = line.find("CYFS Gateway Service ") {
                let tail = &line[index + "CYFS Gateway Service ".len()..];
                if let Some(token) = tail.split_whitespace().next() {
                    if Self::is_likely_version_token(token) {
                        return Some(token.to_string());
                    }
                }
            }

            if let Some(token) = line
                .split_whitespace()
                .find(|token| Self::is_likely_version_token(token))
            {
                return Some(token.to_string());
            }
        }

        None
    }

    fn strip_ansi_codes(input: &str) -> String {
        let mut result = String::with_capacity(input.len());
        let mut chars = input.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\u{1b}' {
                if matches!(chars.peek(), Some('[')) {
                    let _ = chars.next();
                    for code in chars.by_ref() {
                        if ('@'..='~').contains(&code) {
                            break;
                        }
                    }
                }
                continue;
            }
            result.push(ch);
        }
        result
    }

    fn is_likely_version_token(token: &str) -> bool {
        let trimmed =
            token.trim_matches(|ch: char| matches!(ch, ',' | ';' | '(' | ')' | '"' | '\''));
        if !trimmed.contains('.') || !trimmed.chars().any(|ch| ch.is_ascii_digit()) {
            return false;
        }
        if trimmed.contains(':') {
            return false;
        }
        trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '+' | '-' | '_'))
    }
}
