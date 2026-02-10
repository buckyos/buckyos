mod share_content_mgr;

use ::kRPC::*;
use anyhow::Result;
use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use buckyos_api::{
    get_buckyos_api_runtime, init_buckyos_api_runtime, set_buckyos_api_runtime, BuckyOSRuntimeType,
    SystemConfigClient, CONTROL_PANEL_SERVICE_NAME, CONTROL_PANEL_SERVICE_PORT,
};
use buckyos_kit::*;
use bytes::Bytes;
use chrono::{DateTime, Datelike, NaiveDateTime, TimeZone, Utc};
use cyfs_gateway_lib::*;
use http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE};
use http::{Method, Version};
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use log::info;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::*;
use server_runner::*;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::{
    net::IpAddr,
    time::{Duration, Instant},
};
use sysinfo::{DiskRefreshKind, Disks, Networks, System};
use tokio::sync::{Mutex, RwLock};
use tokio::task;
use uuid::Uuid;
use zip::write::FileOptions;
use zip::CompressionMethod;

// RPC docs live under doc/dashboard. UI endpoints use "ui.*" as canonical names;
// "main/layout/dashboard" are kept as legacy aliases.

fn bytes_to_gb(bytes: u64) -> f64 {
    (bytes as f64) / 1024.0 / 1024.0 / 1024.0
}

const LOG_ROOT_DIR: &str = "/opt/buckyos/logs";
const LOG_DOWNLOAD_TTL_SECS: u64 = 600;
const DEFAULT_LOG_LIMIT: usize = 200;
const MAX_LOG_LIMIT: usize = 1000;

#[derive(Clone, Serialize, Deserialize)]
struct LogQueryCursor {
    service: String,
    file: String,
    line_index: u64,
    direction: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct LogTailCursor {
    file: String,
    offset: u64,
}

struct LogDownloadEntry {
    path: PathBuf,
    filename: String,
    expires_at: std::time::SystemTime,
}

struct LogFileRef {
    service: String,
    name: String,
    path: PathBuf,
    modified: std::time::SystemTime,
}

#[derive(Clone, Debug, Default)]
struct NetworkStatsSnapshot {
    rx_bytes: u64,
    tx_bytes: u64,
    rx_per_sec: u64,
    tx_per_sec: u64,
    interface_count: usize,
    updated_at: Option<std::time::SystemTime>,
}

#[derive(Clone)]
struct ControlPanelServer {
    log_downloads: Arc<Mutex<HashMap<String, LogDownloadEntry>>>,
    network_stats: Arc<RwLock<NetworkStatsSnapshot>>,
}

impl ControlPanelServer {
    pub fn new() -> Self {
        let network_stats = Arc::new(RwLock::new(NetworkStatsSnapshot::default()));
        Self::start_network_sampler(network_stats.clone());
        ControlPanelServer {
            log_downloads: Arc::new(Mutex::new(HashMap::new())),
            network_stats,
        }
    }

    fn start_network_sampler(network_stats: Arc<RwLock<NetworkStatsSnapshot>>) {
        // Background sampler so network rates are stable and independent of UI polling.
        tokio::spawn(async move {
            let mut networks = Networks::new_with_refreshed_list();
            networks.refresh(true);

            let (mut prev_rx, mut prev_tx, iface_count) =
                ControlPanelServer::sum_network_totals(&networks);
            {
                let mut snapshot = network_stats.write().await;
                snapshot.rx_bytes = prev_rx;
                snapshot.tx_bytes = prev_tx;
                snapshot.rx_per_sec = 0;
                snapshot.tx_per_sec = 0;
                snapshot.interface_count = iface_count;
                snapshot.updated_at = Some(std::time::SystemTime::now());
            }

            let mut last_at = Instant::now();
            let mut ticker = tokio::time::interval(Duration::from_secs(1));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                ticker.tick().await;
                networks.refresh(true);

                let (rx_bytes, tx_bytes, iface_count) =
                    ControlPanelServer::sum_network_totals(&networks);
                let dt = last_at.elapsed().as_secs_f64();
                last_at = Instant::now();

                let rx_delta = rx_bytes.saturating_sub(prev_rx);
                let tx_delta = tx_bytes.saturating_sub(prev_tx);
                prev_rx = rx_bytes;
                prev_tx = tx_bytes;

                let (rx_per_sec, tx_per_sec) = if dt > 0.0 {
                    (
                        ((rx_delta as f64) / dt).round() as u64,
                        ((tx_delta as f64) / dt).round() as u64,
                    )
                } else {
                    (0, 0)
                };

                let mut snapshot = network_stats.write().await;
                snapshot.rx_bytes = rx_bytes;
                snapshot.tx_bytes = tx_bytes;
                snapshot.rx_per_sec = rx_per_sec;
                snapshot.tx_per_sec = tx_per_sec;
                snapshot.interface_count = iface_count;
                snapshot.updated_at = Some(std::time::SystemTime::now());
            }
        });
    }

    fn sum_network_totals(networks: &Networks) -> (u64, u64, usize) {
        // Default behavior: sum all non-loopback interfaces.
        let mut rx_bytes: u64 = 0;
        let mut tx_bytes: u64 = 0;
        let mut iface_count: usize = 0;
        for (name, data) in networks.iter() {
            let iface = name.as_str();
            if iface == "lo" || iface == "lo0" {
                continue;
            }
            iface_count = iface_count.saturating_add(1);
            rx_bytes = rx_bytes.saturating_add(data.total_received());
            tx_bytes = tx_bytes.saturating_add(data.total_transmitted());
        }
        (rx_bytes, tx_bytes, iface_count)
    }

    async fn handle_main(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "test":"test",
            })),
            req.seq,
        ))
    }

    fn param_str(req: &RPCRequest, key: &str) -> Option<String> {
        req.params
            .get(key)
            .and_then(|value| value.as_str())
            .map(|value| value.to_string())
    }

    fn require_param_str(req: &RPCRequest, key: &str) -> Result<String, RPCErrors> {
        Self::param_str(req, key).ok_or(RPCErrors::ParseRequestError(format!("Missing {}", key)))
    }

    fn encode_cursor<T: Serialize>(value: &T) -> String {
        let payload = serde_json::to_vec(value).unwrap_or_default();
        general_purpose::STANDARD.encode(payload)
    }

    fn decode_cursor<T: DeserializeOwned>(value: &str) -> Option<T> {
        let decoded = general_purpose::STANDARD.decode(value).ok()?;
        serde_json::from_slice(&decoded).ok()
    }

    fn normalize_log_level(value: &str) -> String {
        match value.to_uppercase().as_str() {
            "INFO" => "info".to_string(),
            "WARN" | "WARNING" => "warning".to_string(),
            "ERROR" => "error".to_string(),
            other => other.to_lowercase(),
        }
    }

    fn split_log_line(line: &str) -> (String, String, String) {
        let trimmed = line.trim_start().trim_end();
        if let Some(bracket_start) = trimmed.find('[') {
            if let Some(bracket_end) = trimmed[bracket_start + 1..].find(']') {
                let ts_candidate = trimmed[..bracket_start].trim_end();
                let level = trimmed[bracket_start + 1..bracket_start + 1 + bracket_end].trim();
                let message = trimmed[bracket_start + 1 + bracket_end + 1..].trim_start();
                if !ts_candidate.is_empty() {
                    return (
                        ts_candidate.to_string(),
                        Self::normalize_log_level(level),
                        message.to_string(),
                    );
                }
            }
        }
        ("".to_string(), "unknown".to_string(), trimmed.to_string())
    }

    fn extract_log_entry(
        raw: &str,
        context: Option<&(String, String)>,
    ) -> Option<(String, String, String, Option<(String, String)>)> {
        let trimmed = raw.trim_end();
        if trimmed.is_empty() {
            return None;
        }
        let (ts, level, message) = Self::split_log_line(trimmed);
        if !ts.is_empty() {
            let normalized_level = if level == "unknown" {
                "info".to_string()
            } else {
                level
            };
            let msg = if message.is_empty() {
                trimmed.to_string()
            } else {
                message
            };
            return Some((
                ts.clone(),
                normalized_level.clone(),
                msg,
                Some((ts, normalized_level)),
            ));
        }
        if let Some((ctx_ts, ctx_level)) = context {
            return Some((
                ctx_ts.clone(),
                ctx_level.clone(),
                trimmed.trim().to_string(),
                None,
            ));
        }
        None
    }

    fn parse_log_timestamp(value: &str) -> Option<DateTime<Utc>> {
        if value.is_empty() {
            return None;
        }
        let year = Utc::now().year();
        let with_year = format!("{}-{}", year, value);
        let parsed = NaiveDateTime::parse_from_str(&with_year, "%Y-%m-%d %H:%M:%S%.3f").ok()?;
        Some(Utc.from_utc_datetime(&parsed))
    }

    fn parse_filter_time(value: &str) -> Option<DateTime<Utc>> {
        if let Ok(parsed) = DateTime::parse_from_rfc3339(value) {
            return Some(parsed.with_timezone(&Utc));
        }
        if let Ok(parsed) = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.3f") {
            return Some(Utc.from_utc_datetime(&parsed));
        }
        if let Ok(parsed) = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S") {
            return Some(Utc.from_utc_datetime(&parsed));
        }
        if NaiveDateTime::parse_from_str(value, "%m-%d %H:%M:%S%.3f").is_ok() {
            let year = Utc::now().year();
            let with_year = format!("{}-{}", year, value);
            let parsed = NaiveDateTime::parse_from_str(&with_year, "%Y-%m-%d %H:%M:%S%.3f").ok()?;
            return Some(Utc.from_utc_datetime(&parsed));
        }
        None
    }

    fn format_log_filter_key(value: &DateTime<Utc>) -> String {
        value.format("%m-%d %H:%M:%S%.3f").to_string()
    }

    fn rg_available() -> bool {
        Command::new("rg")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    fn rg_search_lines(path: &Path, keyword: &str) -> Result<Vec<(u64, String)>, RPCErrors> {
        let output = Command::new("rg")
            .arg("--line-number")
            .arg("--fixed-strings")
            .arg("--no-heading")
            .arg("--no-filename")
            .arg("--color")
            .arg("never")
            .arg("-i")
            .arg(keyword)
            .arg(path)
            .output()
            .map_err(|err| RPCErrors::ReasonError(format!("Failed to run rg: {}", err)))?;

        if !output.status.success() {
            if output.status.code() == Some(1) {
                return Ok(Vec::new());
            }
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(RPCErrors::ReasonError(format!(
                "rg failed for {}: {}",
                path.display(),
                stderr
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut results = Vec::new();
        for line in stdout.lines() {
            let mut parts = line.splitn(2, ':');
            let line_no = parts.next().unwrap_or("");
            let content = parts.next().unwrap_or("").to_string();
            if let Ok(number) = line_no.parse::<u64>() {
                let line_index = number.saturating_sub(1);
                results.push((line_index, content));
            }
        }
        Ok(results)
    }

    fn list_log_service_ids(&self) -> Result<Vec<String>, RPCErrors> {
        let mut services = Vec::new();
        let entries = std::fs::read_dir(LOG_ROOT_DIR)
            .map_err(|err| RPCErrors::ReasonError(format!("Failed to read log root: {}", err)))?;
        for entry in entries.flatten() {
            if let Ok(file_type) = entry.file_type() {
                if file_type.is_dir() {
                    if let Some(name) = entry.file_name().to_str() {
                        services.push(name.to_string());
                    }
                }
            }
        }
        services.sort();
        Ok(services)
    }

    fn format_log_service_label(name: &str) -> String {
        name.split(|ch| ch == '_' || ch == '-')
            .filter(|part| !part.is_empty())
            .map(|part| {
                let mut chars = part.chars();
                match chars.next() {
                    Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                    None => "".to_string(),
                }
            })
            .collect::<Vec<String>>()
            .join(" ")
    }

    fn collect_log_files(
        &self,
        service: &str,
        file_filter: Option<&str>,
    ) -> Result<Vec<LogFileRef>, RPCErrors> {
        let mut files = Vec::new();
        let dir_path = Path::new(LOG_ROOT_DIR).join(service);
        let entries = std::fs::read_dir(&dir_path).map_err(|err| {
            RPCErrors::ReasonError(format!("Failed to read log dir {}: {}", service, err))
        })?;

        for entry in entries.flatten() {
            let path = entry.path();
            if let Ok(file_type) = entry.file_type() {
                if !file_type.is_file() {
                    continue;
                }
            }
            let name = match path.file_name().and_then(|value| value.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };
            if let Some(filter) = file_filter {
                if name != filter {
                    continue;
                }
            }
            let modified = std::fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .unwrap_or_else(|_| std::time::SystemTime::UNIX_EPOCH);
            files.push(LogFileRef {
                service: service.to_string(),
                name,
                path,
                modified,
            });
        }

        files.sort_by(|a, b| b.modified.cmp(&a.modified));
        Ok(files)
    }

    async fn cleanup_log_downloads(&self) {
        let mut downloads = self.log_downloads.lock().await;
        let now = std::time::SystemTime::now();
        let mut expired: Vec<PathBuf> = Vec::new();
        downloads.retain(|_, entry| {
            if entry.expires_at <= now {
                expired.push(entry.path.clone());
                false
            } else {
                true
            }
        });
        for path in expired {
            let _ = std::fs::remove_file(path);
        }
    }

    async fn handle_layout(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let layout = json!({
            "profile": {
                "name": "Admin User",
                "email": "admin@buckyos.io",
                "avatar": "https://i.pravatar.cc/64?img=12"
            },
            "systemStatus": {
                "label": "System Online",
                "state": "online",
                "networkPeers": 10,
                "activeSessions": 23
            }
        });

        Ok(RPCResponse::new(RPCResult::Success(layout), req.seq))
    }

    async fn handle_dashboard(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let mut system = System::new_all();
        system.refresh_memory();
        system.refresh_cpu_usage();
        // Wait a moment so CPU usage has a meaningful delta before the second refresh.
        tokio::time::sleep(Duration::from_millis(200)).await;
        system.refresh_cpu_usage();

        let cpu_usage = system.global_cpu_usage() as f64;
        let cpu_brand = system
            .cpus()
            .get(0)
            .map(|c| c.brand().to_string())
            .unwrap_or_else(|| "Unknown CPU".to_string());
        let cpu_cores = system.cpus().len() as u64;
        let total_memory_bytes = system.total_memory();
        let used_memory_bytes = system.used_memory();
        let memory_percent = if total_memory_bytes > 0 {
            ((used_memory_bytes as f64 / total_memory_bytes as f64) * 100.0).round()
        } else {
            0.0
        };

        let mut storage_slices: Vec<Value> = Vec::new();
        let mut disks_detail: Vec<Value> = Vec::new();
        let mut storage_capacity_bytes: u64 = 0;
        let mut storage_used_bytes: u64 = 0;
        let palette = [
            "#1d4ed8", "#6b7280", "#22c55e", "#facc15", "#38bdf8", "#a855f7",
        ];

        let mut disks = Disks::new_with_refreshed_list_specifics(DiskRefreshKind::everything());
        disks.refresh(true);

        for (idx, disk) in disks.list().iter().enumerate() {
            let total = disk.total_space();
            let available = disk.available_space();
            let used = total.saturating_sub(available);
            storage_capacity_bytes = storage_capacity_bytes.saturating_add(total);
            storage_used_bytes = storage_used_bytes.saturating_add(used);

            let used_percent = if total > 0 {
                ((used as f64 / total as f64) * 100.0).round()
            } else {
                0.0
            };

            storage_slices.push(json!({
                "label": disk.name().to_string_lossy(),
                "value": used_percent,
                "color": palette[idx % palette.len()],
            }));

            disks_detail.push(json!({
                "label": disk.name().to_string_lossy(),
                "totalGb": bytes_to_gb(total),
                "usedGb": bytes_to_gb(used),
                "usagePercent": used_percent,
                "fs": disk.file_system().to_string_lossy(),
                "mount": disk.mount_point().to_string_lossy(),
            }));
        }

        if storage_slices.is_empty() {
            storage_slices.push(json!({
                "label": "Storage",
                "value": 0,
                "color": "#6b7280",
            }));
        }

        let storage_capacity_gb = bytes_to_gb(storage_capacity_bytes);
        let storage_used_gb = bytes_to_gb(storage_used_bytes);
        let memory_total_gb = bytes_to_gb(total_memory_bytes);
        let memory_used_gb = bytes_to_gb(used_memory_bytes);

        let device_name = System::host_name().unwrap_or_else(|| "Local Node".to_string());
        let device_info = json!({
            "name": device_name,
            "role": "server",
            "status": "online",
            "uptimeHours": System::uptime() / 3600,
            "cpu": (cpu_usage.round() as u64).min(100),
            "memory": memory_percent as u64,
        });

        let base_cpu = cpu_usage.round() as i64;
        let timeline: Vec<Value> = (0..6)
            .map(|step| {
                let cpu_val = (base_cpu + step as i64 * 2 - 5).clamp(0, 100) as u64;
                json!({
                    "time": format!("{:02}:{:02}", (step * 5) / 60, (step * 5) % 60),
                    "cpu": cpu_val,
                    "memory": memory_percent as u64,
                })
            })
            .collect();

        let dashboard = json!({
            "recentEvents": [
                { "title": "System backup completed", "subtitle": "2 mins ago", "tone": "success" },
                { "title": "High memory usage detected", "subtitle": "15 mins ago", "tone": "warning" },
                { "title": "New device connected: iPhone 15", "subtitle": "1 hour ago", "tone": "info" },
                { "title": "dApp \"FileSync\" updated successfully", "subtitle": "2 hours ago", "tone": "success" },
                { "title": "New admin policy applied", "subtitle": "Yesterday", "tone": "info" }
            ],
            "dapps": [
                { "name": "FileSync", "icon": "🗂️", "status": "running" },
                { "name": "SecureChat", "icon": "💬", "status": "stopped" },
                { "name": "CloudBridge", "icon": "🌉", "status": "stopped" },
                { "name": "PhotoVault", "icon": "📷", "status": "running" },
                { "name": "DataAnalyzer", "icon": "📊", "status": "running" },
                { "name": "WebPortal", "icon": "🌐", "status": "running" }
            ],
            "resourceTimeline": timeline,
            "storageSlices": storage_slices,
            "storageCapacityGb": storage_capacity_gb,
            "storageUsedGb": storage_used_gb,
            "devices": [device_info],
            "memory": {
                "totalGb": memory_total_gb,
                "usedGb": memory_used_gb,
                "usagePercent": memory_percent,
            },
            "cpu": {
                "usagePercent": cpu_usage,
                "model": cpu_brand,
                "cores": cpu_cores,
            },
            "disks": disks_detail
        });

        Ok(RPCResponse::new(RPCResult::Success(dashboard), req.seq))
    }

    async fn handle_system_overview(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let mut system = System::new_all();
        system.refresh_all();

        let cpu_brand = system
            .cpus()
            .get(0)
            .map(|c| c.brand().to_string())
            .unwrap_or_else(|| "Unknown CPU".to_string());

        let overview = json!({
            "name": System::host_name().unwrap_or_else(|| "BuckyOS Node".to_string()),
            "model": cpu_brand,
            "os": System::name().unwrap_or_else(|| "Unknown OS".to_string()),
            "version": System::os_version().unwrap_or_else(|| "Unknown".to_string()),
            "uptime_seconds": System::uptime(),
        });

        Ok(RPCResponse::new(RPCResult::Success(overview), req.seq))
    }

    async fn handle_system_status(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let mut system = System::new_all();
        system.refresh_memory();
        system.refresh_cpu_usage();
        tokio::time::sleep(Duration::from_millis(200)).await;
        system.refresh_cpu_usage();

        let cpu_usage = system.global_cpu_usage() as f64;
        let total_memory_bytes = system.total_memory();
        let used_memory_bytes = system.used_memory();
        let memory_percent = if total_memory_bytes > 0 {
            ((used_memory_bytes as f64 / total_memory_bytes as f64) * 100.0).round()
        } else {
            0.0
        };
        let total_swap_bytes = system.total_swap();
        let used_swap_bytes = system.used_swap();
        let swap_percent = if total_swap_bytes > 0 {
            ((used_swap_bytes as f64 / total_swap_bytes as f64) * 100.0).round()
        } else {
            0.0
        };

        let mut disks = Disks::new_with_refreshed_list_specifics(DiskRefreshKind::everything());
        disks.refresh(true);
        let mut storage_capacity_bytes: u64 = 0;
        let mut storage_used_bytes: u64 = 0;
        for disk in disks.list().iter() {
            let total = disk.total_space();
            let available = disk.available_space();
            let used = total.saturating_sub(available);
            storage_capacity_bytes = storage_capacity_bytes.saturating_add(total);
            storage_used_bytes = storage_used_bytes.saturating_add(used);
        }
        let disk_usage_percent = if storage_capacity_bytes > 0 {
            ((storage_used_bytes as f64 / storage_capacity_bytes as f64) * 100.0).round()
        } else {
            0.0
        };

        let mut warnings: Vec<Value> = Vec::new();
        let mut status_level: u8 = 0;
        let mut push_warning = |label: &str,
                                message: String,
                                severity: &'static str,
                                value: f64,
                                unit: &'static str| {
            let level = if severity == "critical" { 2 } else { 1 };
            status_level = status_level.max(level);
            warnings.push(json!({
                "label": label,
                "message": message,
                "severity": severity,
                "value": value,
                "unit": unit,
            }));
        };

        let warn_threshold = 85.0;
        let critical_threshold = 95.0;

        if cpu_usage >= critical_threshold {
            push_warning(
                "CPU",
                format!("CPU usage above {:.0}%", critical_threshold),
                "critical",
                cpu_usage,
                "%",
            );
        } else if cpu_usage >= warn_threshold {
            push_warning(
                "CPU",
                format!("CPU usage above {:.0}%", warn_threshold),
                "warning",
                cpu_usage,
                "%",
            );
        }

        if memory_percent >= critical_threshold {
            push_warning(
                "Memory",
                format!("Memory usage above {:.0}%", critical_threshold),
                "critical",
                memory_percent,
                "%",
            );
        } else if memory_percent >= warn_threshold {
            push_warning(
                "Memory",
                format!("Memory usage above {:.0}%", warn_threshold),
                "warning",
                memory_percent,
                "%",
            );
        }

        if disk_usage_percent >= critical_threshold {
            push_warning(
                "Storage",
                format!("Disk usage above {:.0}%", critical_threshold),
                "critical",
                disk_usage_percent,
                "%",
            );
        } else if disk_usage_percent >= warn_threshold {
            push_warning(
                "Storage",
                format!("Disk usage above {:.0}%", warn_threshold),
                "warning",
                disk_usage_percent,
                "%",
            );
        }

        if swap_percent >= critical_threshold {
            push_warning(
                "Swap",
                format!("Swap usage above {:.0}%", critical_threshold),
                "critical",
                swap_percent,
                "%",
            );
        } else if swap_percent >= warn_threshold {
            push_warning(
                "Swap",
                format!("Swap usage above {:.0}%", warn_threshold),
                "warning",
                swap_percent,
                "%",
            );
        }

        let key = Self::param_str(&req, "key").unwrap_or_else(|| "services".to_string());
        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        let services: Vec<Value> = match client.list(&key).await {
            Ok(items) => items
                .into_iter()
                .map(|name| json!({ "name": name, "status": "unknown" }))
                .collect(),
            Err(error) => {
                push_warning(
                    "Services",
                    format!("Failed to list services: {}", error),
                    "warning",
                    0.0,
                    "",
                );
                Vec::new()
            }
        };

        let state = match status_level {
            2 => "critical",
            1 => "warning",
            _ => "online",
        };

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "state": state,
                "warnings": warnings,
                "services": services,
            })),
            req.seq,
        ))
    }

    async fn handle_system_logs_list(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let services = self.list_log_service_ids()?;
        let items: Vec<Value> = services
            .iter()
            .map(|service| {
                json!({
                    "id": service,
                    "label": Self::format_log_service_label(service),
                    "path": format!("{}/{}", LOG_ROOT_DIR, service),
                })
            })
            .collect();

        Ok(RPCResponse::new(
            RPCResult::Success(json!({ "services": items })),
            req.seq,
        ))
    }

    async fn handle_system_logs_query(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let mut services: Vec<String> = req
            .params
            .get("services")
            .and_then(|value| value.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|item| item.as_str().map(|value| value.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        if services.is_empty() {
            if let Some(service) = Self::param_str(&req, "service") {
                services.push(service);
            }
        }
        if services.is_empty() {
            return Err(RPCErrors::ParseRequestError("Missing service".to_string()));
        }

        let available = self.list_log_service_ids()?;
        for service in services.iter() {
            if !available.contains(service) {
                return Err(RPCErrors::ReasonError(format!(
                    "Unknown log service: {}",
                    service
                )));
            }
        }

        let file_filter = Self::param_str(&req, "file");
        let direction = Self::param_str(&req, "direction").unwrap_or_else(|| "forward".to_string());
        let direction = if direction == "backward" {
            "backward".to_string()
        } else {
            "forward".to_string()
        };
        let level_filter = Self::param_str(&req, "level").map(|value| value.to_lowercase());
        let keyword_raw = Self::param_str(&req, "keyword");
        let keyword_filter = keyword_raw.as_ref().map(|value| value.to_lowercase());
        let since_filter =
            Self::param_str(&req, "since").and_then(|value| Self::parse_filter_time(&value));
        let until_filter =
            Self::param_str(&req, "until").and_then(|value| Self::parse_filter_time(&value));
        let since_key = since_filter.as_ref().map(Self::format_log_filter_key);
        let until_key = until_filter.as_ref().map(Self::format_log_filter_key);
        let limit = req
            .params
            .get("limit")
            .and_then(|value| value.as_u64())
            .unwrap_or(DEFAULT_LOG_LIMIT as u64)
            .clamp(1, MAX_LOG_LIMIT as u64) as usize;
        let cursor = Self::param_str(&req, "cursor")
            .and_then(|value| Self::decode_cursor::<LogQueryCursor>(&value));

        let mut files: Vec<LogFileRef> = Vec::new();
        for service in services.iter() {
            files.extend(self.collect_log_files(service, file_filter.as_deref())?);
        }
        files.sort_by(|a, b| b.modified.cmp(&a.modified));

        let cursor = cursor.and_then(|value| {
            if value.direction != direction {
                return None;
            }
            if files
                .iter()
                .any(|file| file.service == value.service && file.name == value.file)
            {
                Some(value)
            } else {
                None
            }
        });

        if direction == "backward" {
            let mut collected: Vec<Value> = Vec::new();
            let mut has_more = false;
            let mut next_cursor: Option<LogQueryCursor> = None;
            let use_rg = keyword_raw.as_ref().is_some() && Self::rg_available();
            let cursor_index = cursor.as_ref().and_then(|value| {
                files
                    .iter()
                    .position(|file| file.service == value.service && file.name == value.file)
            });

            for (file_index, file) in files.iter().enumerate() {
                if let Some(cursor_index) = cursor_index {
                    if file_index < cursor_index {
                        continue;
                    }
                }

                let mut candidates: Vec<(u64, String, String, String, String)> = Vec::new();
                let mut rg_used = false;
                if use_rg {
                    let keyword = keyword_raw.as_ref().unwrap();
                    match Self::rg_search_lines(&file.path, keyword) {
                        Ok(matched_lines) => {
                            rg_used = true;
                            for (line_index, raw) in matched_lines.into_iter() {
                                let (ts, level, message) = Self::split_log_line(&raw);
                                if ts.is_empty() {
                                    continue;
                                }
                                if let Some(filter) = level_filter.as_ref() {
                                    if &level != filter {
                                        continue;
                                    }
                                }
                                if since_key.is_some() || until_key.is_some() {
                                    if ts.is_empty() {
                                        continue;
                                    }
                                    if let Some(since) = since_key.as_ref() {
                                        if ts < *since {
                                            continue;
                                        }
                                    }
                                    if let Some(until) = until_key.as_ref() {
                                        if ts > *until {
                                            continue;
                                        }
                                    }
                                }
                                candidates.push((line_index, ts, level, message, raw));
                            }
                        }
                        Err(err) => {
                            log::warn!("rg failed for {}: {}", file.name, err);
                        }
                    }
                }

                if !rg_used {
                    let mut last_context: Option<(String, String)> = None;
                    let file_handle = std::fs::File::open(&file.path).map_err(|err| {
                        RPCErrors::ReasonError(format!(
                            "Failed to open log file {}: {}",
                            file.name, err
                        ))
                    })?;
                    let reader = BufReader::new(file_handle);
                    for (index, line) in reader.lines().enumerate() {
                        let raw = match line {
                            Ok(value) => value,
                            Err(_) => continue,
                        };
                        let maybe_entry = Self::extract_log_entry(&raw, last_context.as_ref());
                        let (ts, level, message, raw_line) = match maybe_entry {
                            Some((ts, level, message, next_context)) => {
                                if let Some(context) = next_context {
                                    last_context = Some(context);
                                }
                                (ts, level, message, raw.trim_end().to_string())
                            }
                            None => continue,
                        };
                        if let Some(filter) = level_filter.as_ref() {
                            if &level != filter {
                                continue;
                            }
                        }
                        if let Some(filter) = keyword_filter.as_ref() {
                            if !raw_line.to_lowercase().contains(filter) {
                                continue;
                            }
                        }
                        if since_key.is_some() || until_key.is_some() {
                            if ts.is_empty() {
                                continue;
                            }
                            if let Some(since) = since_key.as_ref() {
                                if ts < *since {
                                    continue;
                                }
                            }
                            if let Some(until) = until_key.as_ref() {
                                if ts > *until {
                                    continue;
                                }
                            }
                        }
                        candidates.push((index as u64, ts, level, message, raw_line));
                    }
                }

                if let Some(cursor) = cursor.as_ref() {
                    if cursor.service == file.service && cursor.file == file.name {
                        candidates
                            .retain(|(line_index, _, _, _, _)| *line_index < cursor.line_index);
                    }
                }

                for (line_index, ts, level, message, raw) in candidates.into_iter().rev() {
                    collected.push(json!({
                        "timestamp": ts,
                        "level": level,
                        "message": message,
                        "raw": raw,
                        "service": file.service.clone(),
                        "file": file.name.clone(),
                        "line": line_index,
                    }));

                    if collected.len() >= limit {
                        has_more = true;
                        next_cursor = Some(LogQueryCursor {
                            service: file.service.clone(),
                            file: file.name.clone(),
                            line_index,
                            direction: direction.clone(),
                        });
                        break;
                    }
                }

                if has_more {
                    break;
                }
            }

            collected.reverse();
            Ok(RPCResponse::new(
                RPCResult::Success(json!({
                    "entries": collected,
                    "hasMore": has_more,
                    "nextCursor": next_cursor.map(|value| Self::encode_cursor(&value)),
                })),
                req.seq,
            ))
        } else {
            let mut entries: Vec<Value> = Vec::new();
            let mut has_more = false;
            let mut next_cursor: Option<LogQueryCursor> = None;
            let mut reached_cursor = cursor.is_none();
            let use_rg = keyword_raw.as_ref().is_some() && Self::rg_available();

            for file in files.iter() {
                let mut rg_used = false;
                if use_rg {
                    let keyword = keyword_raw.as_ref().unwrap();
                    match Self::rg_search_lines(&file.path, keyword) {
                        Ok(matched_lines) => {
                            rg_used = true;
                            for (line_index, raw) in matched_lines.into_iter() {
                                if !reached_cursor {
                                    if let Some(cursor) = cursor.as_ref() {
                                        if cursor.service == file.service
                                            && cursor.file == file.name
                                        {
                                            if line_index <= cursor.line_index {
                                                continue;
                                            }
                                            reached_cursor = true;
                                        } else {
                                            continue;
                                        }
                                    }
                                }

                                let (ts, level, message) = Self::split_log_line(&raw);
                                if ts.is_empty() {
                                    continue;
                                }
                                if let Some(filter) = level_filter.as_ref() {
                                    if &level != filter {
                                        continue;
                                    }
                                }
                                if since_key.is_some() || until_key.is_some() {
                                    if ts.is_empty() {
                                        continue;
                                    }
                                    if let Some(since) = since_key.as_ref() {
                                        if ts < *since {
                                            continue;
                                        }
                                    }
                                    if let Some(until) = until_key.as_ref() {
                                        if ts > *until {
                                            continue;
                                        }
                                    }
                                }

                                entries.push(json!({
                                    "timestamp": ts,
                                    "level": level,
                                    "message": message,
                                    "raw": raw,
                                    "service": file.service.clone(),
                                    "file": file.name.clone(),
                                    "line": line_index,
                                }));

                                if entries.len() >= limit {
                                    has_more = true;
                                    next_cursor = Some(LogQueryCursor {
                                        service: file.service.clone(),
                                        file: file.name.clone(),
                                        line_index,
                                        direction: direction.clone(),
                                    });
                                    break;
                                }
                            }
                        }
                        Err(err) => {
                            log::warn!("rg failed for {}: {}", file.name, err);
                        }
                    }
                }

                if !rg_used {
                    let mut last_context: Option<(String, String)> = None;
                    let file_handle = std::fs::File::open(&file.path).map_err(|err| {
                        RPCErrors::ReasonError(format!(
                            "Failed to open log file {}: {}",
                            file.name, err
                        ))
                    })?;
                    let reader = BufReader::new(file_handle);
                    for (index, line) in reader.lines().enumerate() {
                        let line_index = index as u64;
                        let raw = match line {
                            Ok(value) => value,
                            Err(_) => continue,
                        };
                        let maybe_entry = Self::extract_log_entry(&raw, last_context.as_ref());
                        let (ts, level, message, raw_line) = match maybe_entry {
                            Some((ts, level, message, next_context)) => {
                                if let Some(context) = next_context {
                                    last_context = Some(context);
                                }
                                (ts, level, message, raw.trim_end().to_string())
                            }
                            None => continue,
                        };

                        if !reached_cursor {
                            if let Some(cursor) = cursor.as_ref() {
                                if cursor.service == file.service && cursor.file == file.name {
                                    if line_index <= cursor.line_index {
                                        continue;
                                    }
                                    reached_cursor = true;
                                } else {
                                    continue;
                                }
                            }
                        }
                        if let Some(filter) = level_filter.as_ref() {
                            if &level != filter {
                                continue;
                            }
                        }
                        if let Some(filter) = keyword_filter.as_ref() {
                            if !raw_line.to_lowercase().contains(filter) {
                                continue;
                            }
                        }
                        if since_key.is_some() || until_key.is_some() {
                            if ts.is_empty() {
                                continue;
                            }
                            if let Some(since) = since_key.as_ref() {
                                if ts < *since {
                                    continue;
                                }
                            }
                            if let Some(until) = until_key.as_ref() {
                                if ts > *until {
                                    continue;
                                }
                            }
                        }

                        entries.push(json!({
                            "timestamp": ts,
                            "level": level,
                            "message": message,
                            "raw": raw_line,
                            "service": file.service.clone(),
                            "file": file.name.clone(),
                            "line": line_index,
                        }));

                        if entries.len() >= limit {
                            has_more = true;
                            next_cursor = Some(LogQueryCursor {
                                service: file.service.clone(),
                                file: file.name.clone(),
                                line_index,
                                direction: direction.clone(),
                            });
                            break;
                        }
                    }
                }

                if !reached_cursor {
                    if let Some(cursor) = cursor.as_ref() {
                        if cursor.service == file.service && cursor.file == file.name {
                            reached_cursor = true;
                        }
                    }
                }

                if has_more {
                    break;
                }
            }

            Ok(RPCResponse::new(
                RPCResult::Success(json!({
                    "entries": entries,
                    "hasMore": has_more,
                    "nextCursor": next_cursor.map(|value| Self::encode_cursor(&value)),
                })),
                req.seq,
            ))
        }
    }

    async fn handle_system_logs_tail(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let mut services: Vec<String> = req
            .params
            .get("services")
            .and_then(|value| value.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|item| item.as_str().map(|value| value.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        if services.is_empty() {
            if let Some(service) = Self::param_str(&req, "service") {
                services.push(service);
            }
        }
        if services.len() != 1 {
            return Err(RPCErrors::ReasonError(
                "Tail requires exactly one service".to_string(),
            ));
        }
        let service = services[0].clone();

        let available = self.list_log_service_ids()?;
        if !available.contains(&service) {
            return Err(RPCErrors::ReasonError(format!(
                "Unknown log service: {}",
                service
            )));
        }

        let file_param = Self::param_str(&req, "file");
        let level_filter = Self::param_str(&req, "level").map(|value| value.to_lowercase());
        let keyword_filter = Self::param_str(&req, "keyword").map(|value| value.to_lowercase());
        let limit = req
            .params
            .get("limit")
            .and_then(|value| value.as_u64())
            .unwrap_or(DEFAULT_LOG_LIMIT as u64)
            .clamp(1, MAX_LOG_LIMIT as u64) as usize;
        let from = Self::param_str(&req, "from").unwrap_or_else(|| "end".to_string());
        let cursor = Self::param_str(&req, "cursor")
            .and_then(|value| Self::decode_cursor::<LogTailCursor>(&value));

        let mut files = self.collect_log_files(&service, None)?;
        if let Some(file) = file_param.as_deref() {
            files.retain(|entry| entry.name == file);
        }
        let file = files
            .first()
            .ok_or_else(|| RPCErrors::ReasonError(format!("No log files found for {}", service)))?;

        let mut start_offset = 0u64;
        let mut read_from_end = false;
        if let Some(cursor) = cursor.as_ref() {
            if cursor.file == file.name {
                start_offset = cursor.offset;
            } else {
                read_from_end = from != "start";
            }
        } else if from != "start" {
            read_from_end = true;
        }

        let path = file.path.clone();
        let file_name = file.name.clone();
        let read_result = task::spawn_blocking(move || -> Result<(Vec<String>, u64), RPCErrors> {
            let mut file = std::fs::File::open(&path).map_err(|err| {
                RPCErrors::ReasonError(format!("Failed to open log file: {}", err))
            })?;
            let metadata = file.metadata().map_err(|err| {
                RPCErrors::ReasonError(format!("Failed to read log metadata: {}", err))
            })?;
            let file_len = metadata.len();
            if read_from_end {
                let mut buffer = String::new();
                file.read_to_string(&mut buffer).map_err(|err| {
                    RPCErrors::ReasonError(format!("Failed to read log file: {}", err))
                })?;
                let lines = buffer.lines().map(|line| line.to_string()).collect();
                return Ok((lines, file_len));
            }
            let offset = start_offset.min(file_len);
            file.seek(SeekFrom::Start(offset)).map_err(|err| {
                RPCErrors::ReasonError(format!("Failed to seek log file: {}", err))
            })?;
            let mut buffer = String::new();
            file.read_to_string(&mut buffer).map_err(|err| {
                RPCErrors::ReasonError(format!("Failed to read log file: {}", err))
            })?;
            let lines = buffer.lines().map(|line| line.to_string()).collect();
            Ok((lines, file_len))
        })
        .await
        .map_err(|err| RPCErrors::ReasonError(format!("Log tail task failed: {}", err)))??;

        let mut lines = read_result.0;
        let new_offset = read_result.1;
        if read_from_end && lines.len() > limit {
            lines = lines.split_off(lines.len() - limit);
        }

        let mut entries: Vec<Value> = Vec::new();
        let mut last_context: Option<(String, String)> = None;
        for raw in lines.into_iter() {
            let maybe_entry = Self::extract_log_entry(&raw, last_context.as_ref());
            let (ts, level, message, raw_line) = match maybe_entry {
                Some((ts, level, message, next_context)) => {
                    if let Some(context) = next_context {
                        last_context = Some(context);
                    }
                    (ts, level, message, raw.trim_end().to_string())
                }
                None => continue,
            };
            if let Some(filter) = level_filter.as_ref() {
                if &level != filter {
                    continue;
                }
            }
            if let Some(filter) = keyword_filter.as_ref() {
                if !raw_line.to_lowercase().contains(filter) {
                    continue;
                }
            }
            entries.push(json!({
                "timestamp": ts,
                "level": level,
                "message": message,
                "raw": raw_line,
                "service": service.clone(),
                "file": file_name.clone(),
            }));
        }

        let next_cursor = LogTailCursor {
            file: file_name,
            offset: new_offset,
        };

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "entries": entries,
                "nextCursor": Self::encode_cursor(&next_cursor),
            })),
            req.seq,
        ))
    }

    async fn handle_system_logs_download(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let mut services: Vec<String> = req
            .params
            .get("services")
            .and_then(|value| value.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|item| item.as_str().map(|value| value.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        if services.is_empty() {
            if let Some(service) = Self::param_str(&req, "service") {
                services.push(service);
            }
        }
        if services.is_empty() {
            return Err(RPCErrors::ParseRequestError("Missing service".to_string()));
        }

        let available = self.list_log_service_ids()?;
        for service in services.iter() {
            if !available.contains(service) {
                return Err(RPCErrors::ReasonError(format!(
                    "Unknown log service: {}",
                    service
                )));
            }
        }

        let mode = Self::param_str(&req, "mode").unwrap_or_else(|| "filtered".to_string());
        let level_filter = Self::param_str(&req, "level").map(|value| value.to_lowercase());
        let keyword_filter = Self::param_str(&req, "keyword").map(|value| value.to_lowercase());
        let since_filter =
            Self::param_str(&req, "since").and_then(|value| Self::parse_filter_time(&value));
        let until_filter =
            Self::param_str(&req, "until").and_then(|value| Self::parse_filter_time(&value));

        let token = Uuid::new_v4().to_string();
        let file_name = format!("buckyos-logs-{}.zip", token);
        let zip_path = std::env::temp_dir().join(&file_name);
        let zip_path_clone = zip_path.clone();

        let services_clone = services.clone();
        let mode_clone = mode.clone();
        let file_filter = Self::param_str(&req, "file");

        task::spawn_blocking(move || -> Result<(), RPCErrors> {
            let file = std::fs::File::create(&zip_path_clone)
                .map_err(|err| RPCErrors::ReasonError(format!("Failed to create zip: {}", err)))?;
            let mut zip = zip::ZipWriter::new(file);
            let options =
                FileOptions::<()>::default().compression_method(CompressionMethod::Deflated);

            for service in services_clone.iter() {
                let dir_path = Path::new(LOG_ROOT_DIR).join(service);
                if mode_clone == "full" {
                    let entries = std::fs::read_dir(&dir_path).map_err(|err| {
                        RPCErrors::ReasonError(format!(
                            "Failed to read log dir {}: {}",
                            service, err
                        ))
                    })?;
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if !path.is_file() {
                            continue;
                        }
                        let name = match path.file_name().and_then(|value| value.to_str()) {
                            Some(name) => name.to_string(),
                            None => continue,
                        };
                        if let Some(filter) = file_filter.as_deref() {
                            if name != filter {
                                continue;
                            }
                        }
                        let entry_name = format!("{}/{}", service, name);
                        zip.start_file(entry_name, options)
                            .map_err(|err| RPCErrors::ReasonError(format!("Zip error: {}", err)))?;
                        let mut file_reader = std::fs::File::open(&path).map_err(|err| {
                            RPCErrors::ReasonError(format!("Failed to read log file: {}", err))
                        })?;
                        let mut buffer = Vec::new();
                        file_reader.read_to_end(&mut buffer).map_err(|err| {
                            RPCErrors::ReasonError(format!("Failed to read log file: {}", err))
                        })?;
                        zip.write_all(&buffer).map_err(|err| {
                            RPCErrors::ReasonError(format!("Failed to write zip: {}", err))
                        })?;
                    }
                } else {
                    let mut filtered = String::new();
                    let entries = std::fs::read_dir(&dir_path).map_err(|err| {
                        RPCErrors::ReasonError(format!(
                            "Failed to read log dir {}: {}",
                            service, err
                        ))
                    })?;
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if !path.is_file() {
                            continue;
                        }
                        let name = match path.file_name().and_then(|value| value.to_str()) {
                            Some(name) => name.to_string(),
                            None => continue,
                        };
                        if let Some(filter) = file_filter.as_deref() {
                            if name != filter {
                                continue;
                            }
                        }
                        let file_handle = std::fs::File::open(&path).map_err(|err| {
                            RPCErrors::ReasonError(format!("Failed to read log file: {}", err))
                        })?;
                        let reader = BufReader::new(file_handle);
                        for line in reader.lines().flatten() {
                            let (ts, level, message) = ControlPanelServer::split_log_line(&line);
                            if let Some(filter) = level_filter.as_ref() {
                                if &level != filter {
                                    continue;
                                }
                            }
                            if let Some(filter) = keyword_filter.as_ref() {
                                if !line.to_lowercase().contains(filter) {
                                    continue;
                                }
                            }
                            if since_filter.is_some() || until_filter.is_some() {
                                let ts_value = match ControlPanelServer::parse_log_timestamp(&ts) {
                                    Some(value) => value,
                                    None => continue,
                                };
                                if let Some(since) = since_filter.as_ref() {
                                    if &ts_value < since {
                                        continue;
                                    }
                                }
                                if let Some(until) = until_filter.as_ref() {
                                    if &ts_value > until {
                                        continue;
                                    }
                                }
                            }
                            filtered.push_str(&format!(
                                "{} {} {}\n",
                                ts,
                                level.to_uppercase(),
                                message
                            ));
                        }
                    }
                    let entry_name = format!("{}/filtered.log", service);
                    zip.start_file(entry_name, options)
                        .map_err(|err| RPCErrors::ReasonError(format!("Zip error: {}", err)))?;
                    zip.write_all(filtered.as_bytes()).map_err(|err| {
                        RPCErrors::ReasonError(format!("Failed to write zip: {}", err))
                    })?;
                }
            }

            zip.finish()
                .map_err(|err| RPCErrors::ReasonError(format!("Failed to finish zip: {}", err)))?;
            Ok(())
        })
        .await
        .map_err(|err| RPCErrors::ReasonError(format!("Zip task failed: {}", err)))??;

        self.cleanup_log_downloads().await;
        let mut downloads = self.log_downloads.lock().await;
        downloads.insert(
            token.clone(),
            LogDownloadEntry {
                path: zip_path,
                filename: file_name.clone(),
                expires_at: std::time::SystemTime::now()
                    + std::time::Duration::from_secs(LOG_DOWNLOAD_TTL_SECS),
            },
        );

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "url": format!("/kapi/control-panel/logs/download/{}", token),
                "expiresInSec": LOG_DOWNLOAD_TTL_SECS,
                "filename": file_name,
            })),
            req.seq,
        ))
    }

    async fn handle_logs_download_http(
        &self,
        token: &str,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        self.cleanup_log_downloads().await;
        let (path, filename) = {
            let downloads = self.log_downloads.lock().await;
            match downloads.get(token) {
                Some(entry) => (entry.path.clone(), entry.filename.clone()),
                None => {
                    return Err(server_err!(
                        ServerErrorCode::BadRequest,
                        "Invalid download token"
                    ))
                }
            }
        };

        let content = tokio::fs::read(&path)
            .await
            .map_err(|err| server_err!(ServerErrorCode::InvalidData, "Read zip error: {}", err))?;
        let body = BoxBody::new(
            Full::new(Bytes::from(content))
                .map_err(|never: std::convert::Infallible| match never {})
                .boxed(),
        );

        http::Response::builder()
            .header(CONTENT_TYPE, "application/zip")
            .header(
                CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", filename),
            )
            .header(CACHE_CONTROL, "no-store")
            .body(body)
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "Failed to build download response: {}",
                    err
                )
            })
    }

    async fn handle_system_metrics(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let mut system = System::new_all();
        system.refresh_memory();
        system.refresh_cpu_usage();
        tokio::time::sleep(Duration::from_millis(200)).await;
        system.refresh_cpu_usage();

        let cpu_usage = system.global_cpu_usage() as f64;
        let cpu_brand = system
            .cpus()
            .get(0)
            .map(|c| c.brand().to_string())
            .unwrap_or_else(|| "Unknown CPU".to_string());
        let cpu_cores = system.cpus().len() as u64;

        let total_memory_bytes = system.total_memory();
        let used_memory_bytes = system.used_memory();
        let memory_percent = if total_memory_bytes > 0 {
            ((used_memory_bytes as f64 / total_memory_bytes as f64) * 100.0).round()
        } else {
            0.0
        };
        let total_swap_bytes = system.total_swap();
        let used_swap_bytes = system.used_swap();
        let swap_percent = if total_swap_bytes > 0 {
            ((used_swap_bytes as f64 / total_swap_bytes as f64) * 100.0).round()
        } else {
            0.0
        };
        let load_avg = System::load_average();
        let process_count = system.processes().len() as u64;
        let uptime_seconds = System::uptime();

        let lite = req
            .params
            .get("lite")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let mut disks_detail: Vec<Value> = Vec::new();
        let mut storage_capacity_bytes: u64 = 0;
        let mut storage_used_bytes: u64 = 0;

        let network_stats = { self.network_stats.read().await.clone() };

        if !lite {
            let mut disks = Disks::new_with_refreshed_list_specifics(DiskRefreshKind::everything());
            disks.refresh(true);

            for disk in disks.list().iter() {
                let total = disk.total_space();
                let available = disk.available_space();
                let used = total.saturating_sub(available);
                storage_capacity_bytes = storage_capacity_bytes.saturating_add(total);
                storage_used_bytes = storage_used_bytes.saturating_add(used);
                let usage_percent = if total > 0 {
                    ((used as f64 / total as f64) * 100.0).round()
                } else {
                    0.0
                };

                disks_detail.push(json!({
                    "label": disk.name().to_string_lossy(),
                    "totalGb": bytes_to_gb(total),
                    "usedGb": bytes_to_gb(used),
                    "usagePercent": usage_percent,
                    "fs": disk.file_system().to_string_lossy(),
                    "mount": disk.mount_point().to_string_lossy(),
                }));
            }
        }

        let disk_usage_percent = if storage_capacity_bytes > 0 {
            ((storage_used_bytes as f64 / storage_capacity_bytes as f64) * 100.0).round()
        } else {
            0.0
        };

        let metrics = json!({
            "cpu": {
                "usagePercent": cpu_usage,
                "model": cpu_brand,
                "cores": cpu_cores,
            },
            "memory": {
                "totalGb": bytes_to_gb(total_memory_bytes),
                "usedGb": bytes_to_gb(used_memory_bytes),
                "usagePercent": memory_percent,
            },
            "disk": {
                "totalGb": bytes_to_gb(storage_capacity_bytes),
                "usedGb": bytes_to_gb(storage_used_bytes),
                "usagePercent": disk_usage_percent,
                "disks": disks_detail,
            },
            "network": {
                "rxBytes": network_stats.rx_bytes,
                "txBytes": network_stats.tx_bytes,
                "rxPerSec": network_stats.rx_per_sec,
                "txPerSec": network_stats.tx_per_sec,
            },
            "swap": {
                "totalGb": bytes_to_gb(total_swap_bytes),
                "usedGb": bytes_to_gb(used_swap_bytes),
                "usagePercent": swap_percent,
            },
            "loadAverage": {
                "one": load_avg.one,
                "five": load_avg.five,
                "fifteen": load_avg.fifteen,
            },
            "processCount": process_count,
            "uptimeSeconds": uptime_seconds,
        });

        Ok(RPCResponse::new(RPCResult::Success(metrics), req.seq))
    }

    async fn handle_sys_config_get(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let key = Self::require_param_str(&req, "key")?;
        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        let value = client
            .get(&key)
            .await
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "key": key,
                "value": value.value,
                "version": value.version,
                "isChanged": value.is_changed,
            })),
            req.seq,
        ))
    }

    async fn handle_sys_config_set(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let key = Self::require_param_str(&req, "key")?;
        let value = Self::require_param_str(&req, "value")?;
        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        client
            .set(&key, &value)
            .await
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "ok": true,
                "key": key,
            })),
            req.seq,
        ))
    }

    async fn handle_sys_config_list(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let key = Self::param_str(&req, "key")
            .or_else(|| Self::param_str(&req, "prefix"))
            .unwrap_or_default();
        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        let items = client
            .list(&key)
            .await
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "key": key,
                "items": items,
            })),
            req.seq,
        ))
    }

    async fn build_sys_config_tree(
        &self,
        client: &SystemConfigClient,
        key: &str,
        depth: u64,
    ) -> Result<Value, RPCErrors> {
        if depth == 0 {
            return Ok(json!({}));
        }

        let mut queue: Vec<(String, u64)> = vec![(key.to_string(), depth)];
        let mut children_map: HashMap<String, Vec<String>> = HashMap::new();

        while let Some((current_key, current_depth)) = queue.pop() {
            if current_depth == 0 {
                continue;
            }
            let children = client
                .list(&current_key)
                .await
                .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
            children_map.insert(current_key.clone(), children.clone());
            if current_depth > 1 {
                for child in children {
                    let child_key = if current_key.is_empty() || child.starts_with(&current_key) {
                        child
                    } else {
                        format!("{}/{}", current_key, child)
                    };
                    queue.push((child_key, current_depth - 1));
                }
            }
        }

        fn build_tree_node(
            children_map: &HashMap<String, Vec<String>>,
            key: &str,
            depth: u64,
        ) -> Value {
            if depth == 0 {
                return json!({});
            }
            let mut map = Map::new();
            let children = children_map.get(key).cloned().unwrap_or_default();
            for child in children {
                let child_key = if key.is_empty() || child.starts_with(key) {
                    child.clone()
                } else {
                    format!("{}/{}", key, child)
                };
                let child_name = child
                    .split('/')
                    .last()
                    .unwrap_or(child.as_str())
                    .to_string();
                let subtree = if depth > 1 {
                    build_tree_node(children_map, &child_key, depth - 1)
                } else {
                    json!({})
                };
                map.insert(child_name, subtree);
            }
            Value::Object(map)
        }

        Ok(build_tree_node(&children_map, key, depth))
    }

    async fn handle_sys_config_tree(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let key = Self::param_str(&req, "key")
            .or_else(|| Self::param_str(&req, "prefix"))
            .unwrap_or_default();
        let depth = req
            .params
            .get("depth")
            .and_then(|value| value.as_u64())
            .unwrap_or(2);
        let depth = depth.min(6);
        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        let tree = self.build_sys_config_tree(&client, &key, depth).await?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "key": key,
                "depth": depth,
                "tree": tree,
            })),
            req.seq,
        ))
    }

    async fn handle_system_config_test(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let key = req
            .params
            .get("key")
            .and_then(|value| value.as_str())
            .unwrap_or("boot/config")
            .to_string();
        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        let value = client
            .get(&key)
            .await
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "key": key,
                "value": value.value,
                "version": value.version,
                "isChanged": value.is_changed,
            })),
            req.seq,
        ))
    }

    async fn handle_apps_list(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
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

    async fn handle_unimplemented(
        &self,
        req: RPCRequest,
        purpose: &'static str,
    ) -> Result<RPCResponse, RPCErrors> {
        Err(RPCErrors::ReasonError(format!(
            "Not implemented: {} ({})",
            req.method, purpose
        )))
    }
}

#[async_trait]
impl RPCHandler for ControlPanelServer {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        _ip_from: IpAddr,
    ) -> Result<RPCResponse, RPCErrors> {
        match req.method.as_str() {
            // Core / UI bootstrap
            "main" | "ui.main" => self.handle_main(req).await,
            "layout" | "ui.layout" => self.handle_layout(req).await,
            "dashboard" | "ui.dashboard" => self.handle_dashboard(req).await,
            // Auth
            "auth.login" => {
                self.handle_unimplemented(req, "Authenticate admin/user session")
                    .await
            }
            "auth.logout" => self.handle_unimplemented(req, "Terminate session").await,
            "auth.refresh" => {
                self.handle_unimplemented(req, "Refresh token/session")
                    .await
            }
            // User & Role
            "user.list" => self.handle_unimplemented(req, "List users").await,
            "user.get" => self.handle_unimplemented(req, "Get user detail").await,
            "user.create" => self.handle_unimplemented(req, "Create user").await,
            "user.update" => self.handle_unimplemented(req, "Update user").await,
            "user.delete" => self.handle_unimplemented(req, "Delete user").await,
            "user.role.list" => self.handle_unimplemented(req, "List roles/policies").await,
            "user.role.update" => self.handle_unimplemented(req, "Update role/policy").await,
            // System
            "system.overview" => self.handle_system_overview(req).await,
            "system.status" => self.handle_system_status(req).await,
            "system.metrics" => self.handle_system_metrics(req).await,
            "system.logs.list" => self.handle_system_logs_list(req).await,
            "system.logs.query" => self.handle_system_logs_query(req).await,
            "system.logs.tail" => self.handle_system_logs_tail(req).await,
            "system.logs.download" => self.handle_system_logs_download(req).await,
            "system.update.check" => self.handle_unimplemented(req, "Check updates").await,
            "system.update.apply" => self.handle_unimplemented(req, "Apply update").await,
            "system.config.test" => self.handle_system_config_test(req).await,
            // Storage
            "storage.volumes" => self.handle_unimplemented(req, "List volumes/arrays").await,
            "storage.volume.get" => self.handle_unimplemented(req, "Get volume detail").await,
            "storage.volume.create" => self.handle_unimplemented(req, "Create volume").await,
            "storage.volume.expand" => self.handle_unimplemented(req, "Expand volume").await,
            "storage.volume.delete" => self.handle_unimplemented(req, "Delete volume").await,
            "storage.disks" => self.handle_unimplemented(req, "List physical disks").await,
            "storage.smart" => self.handle_unimplemented(req, "Disk SMART info").await,
            "storage.raid.rebuild" => self.handle_unimplemented(req, "RAID rebuild").await,
            // Shares
            "share.list" => self.handle_unimplemented(req, "List shared folders").await,
            "share.get" => self.handle_unimplemented(req, "Get share detail").await,
            "share.create" => self.handle_unimplemented(req, "Create share").await,
            "share.update" => self.handle_unimplemented(req, "Update share").await,
            "share.delete" => self.handle_unimplemented(req, "Delete share").await,
            // Files
            "files.browse" => {
                self.handle_unimplemented(req, "List directory entries")
                    .await
            }
            "files.stat" => self.handle_unimplemented(req, "File metadata").await,
            "files.mkdir" => self.handle_unimplemented(req, "Create folder").await,
            "files.delete" => self.handle_unimplemented(req, "Delete file/folder").await,
            "files.move" => self.handle_unimplemented(req, "Move/rename").await,
            "files.copy" => self.handle_unimplemented(req, "Copy").await,
            "files.upload.init" => {
                self.handle_unimplemented(req, "Init multipart upload")
                    .await
            }
            "files.upload.part" => self.handle_unimplemented(req, "Upload part").await,
            "files.upload.complete" => self.handle_unimplemented(req, "Complete upload").await,
            "files.download" => self.handle_unimplemented(req, "Download file").await,
            // Backup
            "backup.jobs" => self.handle_unimplemented(req, "List backup jobs").await,
            "backup.job.create" => self.handle_unimplemented(req, "Create backup job").await,
            "backup.job.run" => self.handle_unimplemented(req, "Run backup job").await,
            "backup.job.stop" => self.handle_unimplemented(req, "Stop backup job").await,
            "backup.targets" => self.handle_unimplemented(req, "List backup targets").await,
            "backup.restore" => self.handle_unimplemented(req, "Restore backup").await,
            // Apps
            "apps.list" => self.handle_apps_list(req).await,
            "apps.install" => self.handle_unimplemented(req, "Install app").await,
            "apps.update" => self.handle_unimplemented(req, "Update app").await,
            "apps.uninstall" => self.handle_unimplemented(req, "Uninstall app").await,
            "apps.start" => self.handle_unimplemented(req, "Start app").await,
            "apps.stop" => self.handle_unimplemented(req, "Stop app").await,
            // Network
            "network.interfaces" => self.handle_unimplemented(req, "List interfaces").await,
            "network.interface.update" => {
                self.handle_unimplemented(req, "Update interface config")
                    .await
            }
            "network.dns" => self.handle_unimplemented(req, "Get/set DNS").await,
            "network.ddns" => self.handle_unimplemented(req, "Get/set DDNS").await,
            "network.firewall.rules" => self.handle_unimplemented(req, "List firewall rules").await,
            "network.firewall.update" => {
                self.handle_unimplemented(req, "Update firewall rules")
                    .await
            }
            // Device
            "device.list" => self.handle_unimplemented(req, "List devices/clients").await,
            "device.block" => self.handle_unimplemented(req, "Block device").await,
            "device.unblock" => self.handle_unimplemented(req, "Unblock device").await,
            // Notification
            "notification.list" => {
                self.handle_unimplemented(req, "List notifications/events")
                    .await
            }
            // Logs
            "log.system" => self.handle_unimplemented(req, "System logs").await,
            "log.access" => self.handle_unimplemented(req, "Access logs").await,
            // Security
            "security.2fa.enable" => self.handle_unimplemented(req, "Enable 2FA").await,
            "security.2fa.disable" => self.handle_unimplemented(req, "Disable 2FA").await,
            "security.keys" => self.handle_unimplemented(req, "List/revoke API keys").await,
            // File Services (SMB/NFS/FTP/WebDAV/...)
            "file_service.list" => self.handle_unimplemented(req, "List file services").await,
            "file_service.smb.get" => self.handle_unimplemented(req, "Get SMB config").await,
            "file_service.smb.update" => self.handle_unimplemented(req, "Update SMB config").await,
            "file_service.nfs.get" => self.handle_unimplemented(req, "Get NFS config").await,
            "file_service.nfs.update" => self.handle_unimplemented(req, "Update NFS config").await,
            "file_service.afp.get" => self.handle_unimplemented(req, "Get AFP config").await,
            "file_service.afp.update" => self.handle_unimplemented(req, "Update AFP config").await,
            "file_service.ftp.get" => self.handle_unimplemented(req, "Get FTP config").await,
            "file_service.ftp.update" => self.handle_unimplemented(req, "Update FTP config").await,
            "file_service.webdav.get" => self.handle_unimplemented(req, "Get WebDAV config").await,
            "file_service.webdav.update" => {
                self.handle_unimplemented(req, "Update WebDAV config").await
            }
            "file_service.rsync.get" => self.handle_unimplemented(req, "Get rsync config").await,
            "file_service.rsync.update" => {
                self.handle_unimplemented(req, "Update rsync config").await
            }
            "file_service.sftp.get" => self.handle_unimplemented(req, "Get SFTP config").await,
            "file_service.sftp.update" => {
                self.handle_unimplemented(req, "Update SFTP config").await
            }
            "file_service.ssh.get" => self.handle_unimplemented(req, "Get SSH config").await,
            "file_service.ssh.update" => self.handle_unimplemented(req, "Update SSH config").await,
            // iSCSI
            "iscsi.targets" => self.handle_unimplemented(req, "List iSCSI targets").await,
            "iscsi.target.create" => self.handle_unimplemented(req, "Create iSCSI target").await,
            "iscsi.target.update" => self.handle_unimplemented(req, "Update iSCSI target").await,
            "iscsi.target.delete" => self.handle_unimplemented(req, "Delete iSCSI target").await,
            "iscsi.luns" => self.handle_unimplemented(req, "List iSCSI LUNs").await,
            "iscsi.lun.create" => self.handle_unimplemented(req, "Create iSCSI LUN").await,
            "iscsi.lun.update" => self.handle_unimplemented(req, "Update iSCSI LUN").await,
            "iscsi.lun.delete" => self.handle_unimplemented(req, "Delete iSCSI LUN").await,
            "iscsi.sessions" => self.handle_unimplemented(req, "List iSCSI sessions").await,
            // Snapshot
            "snapshot.list" => self.handle_unimplemented(req, "List snapshots").await,
            "snapshot.create" => self.handle_unimplemented(req, "Create snapshot").await,
            "snapshot.delete" => self.handle_unimplemented(req, "Delete snapshot").await,
            "snapshot.restore" => self.handle_unimplemented(req, "Restore snapshot").await,
            "snapshot.schedule.list" => {
                self.handle_unimplemented(req, "List snapshot schedules")
                    .await
            }
            "snapshot.schedule.update" => {
                self.handle_unimplemented(req, "Update snapshot schedule")
                    .await
            }
            // Replication
            "replication.jobs" => {
                self.handle_unimplemented(req, "List replication jobs")
                    .await
            }
            "replication.job.create" => {
                self.handle_unimplemented(req, "Create replication job")
                    .await
            }
            "replication.job.run" => self.handle_unimplemented(req, "Run replication job").await,
            "replication.job.pause" => {
                self.handle_unimplemented(req, "Pause replication job")
                    .await
            }
            "replication.job.delete" => {
                self.handle_unimplemented(req, "Delete replication job")
                    .await
            }
            "replication.status" => self.handle_unimplemented(req, "Replication status").await,
            // Sync
            "sync.providers" => self.handle_unimplemented(req, "List sync providers").await,
            "sync.tasks" => self.handle_unimplemented(req, "List sync tasks").await,
            "sync.task.create" => self.handle_unimplemented(req, "Create sync task").await,
            "sync.task.run" => self.handle_unimplemented(req, "Run sync task").await,
            "sync.task.pause" => self.handle_unimplemented(req, "Pause sync task").await,
            "sync.task.resume" => self.handle_unimplemented(req, "Resume sync task").await,
            "sync.task.delete" => self.handle_unimplemented(req, "Delete sync task").await,
            // Quota
            "quota.get" => self.handle_unimplemented(req, "Get quotas").await,
            "quota.update" => self.handle_unimplemented(req, "Update quota").await,
            "quota.defaults" => self.handle_unimplemented(req, "Get quota defaults").await,
            // ACL / Permissions
            "acl.get" => self.handle_unimplemented(req, "Get ACL").await,
            "acl.update" => self.handle_unimplemented(req, "Update ACL").await,
            "acl.reset" => self.handle_unimplemented(req, "Reset ACL").await,
            // Recycle Bin
            "recycle_bin.get" => {
                self.handle_unimplemented(req, "Get recycle bin settings")
                    .await
            }
            "recycle_bin.update" => {
                self.handle_unimplemented(req, "Update recycle bin settings")
                    .await
            }
            "recycle_bin.list" => self.handle_unimplemented(req, "List recycled items").await,
            "recycle_bin.restore" => {
                self.handle_unimplemented(req, "Restore recycled item")
                    .await
            }
            "recycle_bin.delete" => self.handle_unimplemented(req, "Delete recycled item").await,
            // Index / Search
            "index.status" => self.handle_unimplemented(req, "Index status").await,
            "index.rebuild" => self.handle_unimplemented(req, "Rebuild index").await,
            "search.query" => self.handle_unimplemented(req, "Search query").await,
            // Media
            "media.library.scan" => self.handle_unimplemented(req, "Scan media library").await,
            "media.library.status" => self.handle_unimplemented(req, "Media library status").await,
            "media.dlna.get" => self.handle_unimplemented(req, "Get DLNA config").await,
            "media.dlna.update" => self.handle_unimplemented(req, "Update DLNA config").await,
            // Download
            "download.tasks" => self.handle_unimplemented(req, "List download tasks").await,
            "download.task.create" => self.handle_unimplemented(req, "Create download task").await,
            "download.task.pause" => self.handle_unimplemented(req, "Pause download task").await,
            "download.task.resume" => self.handle_unimplemented(req, "Resume download task").await,
            "download.task.delete" => self.handle_unimplemented(req, "Delete download task").await,
            // Container
            "container.list" => self.handle_unimplemented(req, "List containers").await,
            "container.create" => self.handle_unimplemented(req, "Create container").await,
            "container.start" => self.handle_unimplemented(req, "Start container").await,
            "container.stop" => self.handle_unimplemented(req, "Stop container").await,
            "container.update" => self.handle_unimplemented(req, "Update container").await,
            "container.delete" => self.handle_unimplemented(req, "Delete container").await,
            "container.images" => {
                self.handle_unimplemented(req, "List container images")
                    .await
            }
            "container.image.pull" => self.handle_unimplemented(req, "Pull container image").await,
            "container.image.remove" => {
                self.handle_unimplemented(req, "Remove container image")
                    .await
            }
            // VM
            "vm.list" => self.handle_unimplemented(req, "List VMs").await,
            "vm.create" => self.handle_unimplemented(req, "Create VM").await,
            "vm.start" => self.handle_unimplemented(req, "Start VM").await,
            "vm.stop" => self.handle_unimplemented(req, "Stop VM").await,
            "vm.delete" => self.handle_unimplemented(req, "Delete VM").await,
            "vm.snapshot.create" => self.handle_unimplemented(req, "Create VM snapshot").await,
            "vm.snapshot.restore" => self.handle_unimplemented(req, "Restore VM snapshot").await,
            // Certificate
            "cert.list" => self.handle_unimplemented(req, "List certificates").await,
            "cert.issue" => self.handle_unimplemented(req, "Issue certificate").await,
            "cert.import" => self.handle_unimplemented(req, "Import certificate").await,
            "cert.delete" => self.handle_unimplemented(req, "Delete certificate").await,
            "cert.renew" => self.handle_unimplemented(req, "Renew certificate").await,
            // Reverse Proxy
            "proxy.list" => self.handle_unimplemented(req, "List proxy rules").await,
            "proxy.create" => self.handle_unimplemented(req, "Create proxy rule").await,
            "proxy.update" => self.handle_unimplemented(req, "Update proxy rule").await,
            "proxy.delete" => self.handle_unimplemented(req, "Delete proxy rule").await,
            // VPN
            "vpn.profiles" => self.handle_unimplemented(req, "List VPN profiles").await,
            "vpn.profile.create" => self.handle_unimplemented(req, "Create VPN profile").await,
            "vpn.profile.update" => self.handle_unimplemented(req, "Update VPN profile").await,
            "vpn.profile.delete" => self.handle_unimplemented(req, "Delete VPN profile").await,
            "vpn.status" => self.handle_unimplemented(req, "VPN status").await,
            "vpn.connect" => self.handle_unimplemented(req, "Connect VPN").await,
            "vpn.disconnect" => self.handle_unimplemented(req, "Disconnect VPN").await,
            // Power
            "power.shutdown" => self.handle_unimplemented(req, "Shutdown").await,
            "power.reboot" => self.handle_unimplemented(req, "Reboot").await,
            "power.schedule.list" => self.handle_unimplemented(req, "List power schedules").await,
            "power.schedule.update" => {
                self.handle_unimplemented(req, "Update power schedule")
                    .await
            }
            "power.wol.send" => self.handle_unimplemented(req, "Send Wake-on-LAN").await,
            // Time
            "time.get" => self.handle_unimplemented(req, "Get system time").await,
            "time.update" => self.handle_unimplemented(req, "Update system time").await,
            "time.ntp.get" => self.handle_unimplemented(req, "Get NTP settings").await,
            "time.ntp.update" => self.handle_unimplemented(req, "Update NTP settings").await,
            // Hardware
            "hardware.sensors" => self.handle_unimplemented(req, "Hardware sensors").await,
            "hardware.fans" => self.handle_unimplemented(req, "Fan status").await,
            "hardware.led.update" => self.handle_unimplemented(req, "Update LED").await,
            "hardware.ups.get" => self.handle_unimplemented(req, "Get UPS settings").await,
            "hardware.ups.update" => self.handle_unimplemented(req, "Update UPS settings").await,
            // Audit
            "audit.events" => self.handle_unimplemented(req, "Audit events").await,
            "audit.export" => self.handle_unimplemented(req, "Export audit").await,
            // Antivirus
            "antivirus.status" => self.handle_unimplemented(req, "Antivirus status").await,
            "antivirus.scan" => self.handle_unimplemented(req, "Run antivirus scan").await,
            "antivirus.signatures.update" => {
                self.handle_unimplemented(req, "Update antivirus signatures")
                    .await
            }
            "antivirus.quarantine.list" => {
                self.handle_unimplemented(req, "List quarantined items")
                    .await
            }
            "antivirus.quarantine.delete" => {
                self.handle_unimplemented(req, "Delete quarantined item")
                    .await
            }
            // System Config
            "sys_config.get" => self.handle_sys_config_get(req).await,
            "sys_config.set" => self.handle_sys_config_set(req).await,
            "sys_config.list" => self.handle_sys_config_list(req).await,
            "sys_config.tree" => self.handle_sys_config_tree(req).await,
            "sys_config.history" => self.handle_unimplemented(req, "Config history").await,
            // Scheduler
            "scheduler.status" => self.handle_unimplemented(req, "Scheduler status").await,
            "scheduler.queue.list" => self.handle_unimplemented(req, "Scheduler queue").await,
            "scheduler.task.list" => self.handle_unimplemented(req, "Scheduler tasks").await,
            "scheduler.task.cancel" => {
                self.handle_unimplemented(req, "Cancel scheduler task")
                    .await
            }
            // Node / Daemon
            "node.list" => self.handle_unimplemented(req, "List nodes").await,
            "node.get" => self.handle_unimplemented(req, "Node detail").await,
            "node.services.list" => self.handle_unimplemented(req, "Node services").await,
            "node.restart" => self.handle_unimplemented(req, "Restart node").await,
            "node.shutdown" => self.handle_unimplemented(req, "Shutdown node").await,
            // Activation
            "node.activate" => self.handle_unimplemented(req, "Activate node").await,
            "node.activation.status" => self.handle_unimplemented(req, "Activation status").await,
            // Task Manager
            "task.list" => self.handle_unimplemented(req, "List tasks").await,
            "task.get" => self.handle_unimplemented(req, "Task detail").await,
            "task.cancel" => self.handle_unimplemented(req, "Cancel task").await,
            "task.retry" => self.handle_unimplemented(req, "Retry task").await,
            "task.logs" => self.handle_unimplemented(req, "Task logs").await,
            // Verify Hub
            "verify.status" => self.handle_unimplemented(req, "Verify hub status").await,
            "verify.sessions" => self.handle_unimplemented(req, "List sessions").await,
            "verify.session.revoke" => self.handle_unimplemented(req, "Revoke session").await,
            // Repo Service
            "repo.sources" => self.handle_unimplemented(req, "List repo sources").await,
            "repo.pkgs" => self.handle_unimplemented(req, "List repo packages").await,
            "repo.install" => self.handle_unimplemented(req, "Install package").await,
            "repo.publish" => self.handle_unimplemented(req, "Publish package").await,
            "repo.sync" => self.handle_unimplemented(req, "Sync repo").await,
            "repo.tasks" => self.handle_unimplemented(req, "Repo tasks").await,
            // Message Bus
            "msgbus.status" => self.handle_unimplemented(req, "Message bus status").await,
            "msgbus.topics" => self.handle_unimplemented(req, "List topics").await,
            "msgbus.publish" => self.handle_unimplemented(req, "Publish message").await,
            // Nginx / Web Gateway
            "nginx.status" => self.handle_unimplemented(req, "Nginx status").await,
            "nginx.sites" => self.handle_unimplemented(req, "List sites").await,
            "nginx.site.update" => self.handle_unimplemented(req, "Update site").await,
            "nginx.reload" => self.handle_unimplemented(req, "Reload nginx").await,
            // K8s Service
            "k8s.status" => self.handle_unimplemented(req, "K8s status").await,
            "k8s.nodes" => self.handle_unimplemented(req, "List k8s nodes").await,
            "k8s.deployments" => self.handle_unimplemented(req, "List deployments").await,
            "k8s.deployment.scale" => self.handle_unimplemented(req, "Scale deployment").await,
            // Slog Server
            "slog.status" => self.handle_unimplemented(req, "Slog status").await,
            "slog.streams" => self.handle_unimplemented(req, "List log streams").await,
            "slog.query" => self.handle_unimplemented(req, "Query logs").await,
            // Gateway / Zone
            "gateway.status" => self.handle_unimplemented(req, "Gateway status").await,
            "gateway.routes.list" => self.handle_unimplemented(req, "List routes").await,
            "gateway.routes.update" => self.handle_unimplemented(req, "Update routes").await,
            "zone.info" => self.handle_unimplemented(req, "Zone info").await,
            "zone.config.get" => self.handle_unimplemented(req, "Get zone config").await,
            "zone.config.update" => self.handle_unimplemented(req, "Update zone config").await,
            "zone.devices.list" => self.handle_unimplemented(req, "List zone devices").await,
            // RBAC / Permission
            "rbac.model.get" => self.handle_unimplemented(req, "Get RBAC model").await,
            "rbac.model.update" => self.handle_unimplemented(req, "Update RBAC model").await,
            "rbac.policy.get" => self.handle_unimplemented(req, "Get RBAC policy").await,
            "rbac.policy.update" => self.handle_unimplemented(req, "Update RBAC policy").await,
            // Runtime
            "runtime.info" => self.handle_unimplemented(req, "Runtime info").await,
            "runtime.reload" => {
                self.handle_unimplemented(req, "Reload runtime config")
                    .await
            }
            _ => Err(RPCErrors::UnknownMethod(req.method)),
        }
    }
}

#[async_trait]
impl HttpServer for ControlPanelServer {
    async fn serve_request(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        info: StreamInfo,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        if *req.method() == Method::POST {
            return serve_http_by_rpc_handler(req, info, self).await;
        }
        if *req.method() == Method::GET {
            let path = req.uri().path();
            if let Some(token) = path.strip_prefix("/kapi/control-panel/logs/download/") {
                if !token.is_empty() {
                    return self.handle_logs_download_http(token).await;
                }
            }
        }
        return Err(server_err!(
            ServerErrorCode::BadRequest,
            "Method not allowed"
        ));
    }

    fn id(&self) -> String {
        "control-panel".to_string()
    }

    fn http_version(&self) -> Version {
        Version::HTTP_11
    }

    fn http3_port(&self) -> Option<u16> {
        None
    }
}

pub async fn start_control_panel_service() -> anyhow::Result<()> {
    let mut runtime = init_buckyos_api_runtime(
        CONTROL_PANEL_SERVICE_NAME,
        None,
        BuckyOSRuntimeType::KernelService,
    )
    .await?;
    let login_result = runtime.login().await;
    if login_result.is_err() {
        log::error!(
            "control-panel service login to system failed! err:{:?}",
            login_result
        );
        return Err(anyhow::anyhow!(
            "control-panel service login to system failed! err:{:?}",
            login_result
        ));
    }
    runtime
        .set_main_service_port(CONTROL_PANEL_SERVICE_PORT)
        .await;
    set_buckyos_api_runtime(runtime);

    let control_panel_server = ControlPanelServer::new();
    // Bind to the default control-panel service port.

    let runner = Runner::new(CONTROL_PANEL_SERVICE_PORT);
    // 添加 RPC 服务
    let _ = runner.add_http_server(
        "/kapi/control-panel".to_string(),
        Arc::new(control_panel_server),
    );

    // 添加 web (best-effort, skip if path cannot be resolved)
    let web_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|parent| parent.join("web")));
    if let Some(web_dir) = web_dir {
        let _ = runner
            .add_dir_handler_with_options(
                "/".to_string(),
                web_dir,
                DirHandlerOptions {
                    fallback_file: Some("index.html".to_string()),
                    ..Default::default()
                },
            )
            .await;
    } else {
        log::warn!("control-panel web_dir not available; static web UI disabled");
    }

    let _ = runner.start();
    info!(
        "control-panel service started at port {}",
        CONTROL_PANEL_SERVICE_PORT
    );
    Ok(())
}

async fn service_main() {
    init_logging("control-panel", true);
    let start_result = start_control_panel_service().await;
    if start_result.is_err() {
        log::error!("control-panel service start failed! err:{:?}", start_result);
        return;
    }

    let _ = tokio::signal::ctrl_c().await;
}

fn main() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(service_main());
}
