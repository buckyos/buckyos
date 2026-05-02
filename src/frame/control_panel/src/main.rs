mod aicc_settings;
mod app_installer;
mod app_servcie_mgr;
mod dashboard;
mod file_manager;
mod message_hub;
mod sys_auth_backend;
mod sys_log_mgr;
mod sys_settings;
mod ui_session_mgr;
mod user_mgr;
mod zone_mgr;

use sys_log_mgr::LogDownloadEntry;

pub(crate) use message_hub::ChatMessageView;

use ::kRPC::*;
use anyhow::Result;
use async_trait::async_trait;
use buckyos_api::{
    get_buckyos_api_runtime, init_buckyos_api_runtime, set_buckyos_api_runtime, BuckyOSRuntimeType,
    SystemConfigClient, UserType, CONTROL_PANEL_SERVICE_NAME, CONTROL_PANEL_SERVICE_PORT,
};
use buckyos_http_server::*;
use buckyos_kit::*;
use bytes::Bytes;
use http::header::{CACHE_CONTROL, CONTENT_TYPE};
use http::{Method, StatusCode, Version};
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use log::info;
use named_store::{NamedDataMgrZoneGateway, NdmZoneGatewayConfig};
use serde_json::*;
use server_runner::*;
use std::collections::{HashMap, VecDeque};
use std::ffi::OsStr;
use std::net::SocketAddr;
use std::process::Command;
use std::sync::Arc;
use std::{net::IpAddr, time::Instant};
use sysinfo::{Disks, Networks};
use tokio::sync::{Mutex, RwLock};

// RPC docs live under doc/dashboard. UI endpoints use "ui.*" as canonical names;
// "main/layout/dashboard" are kept as legacy aliases.

pub(crate) fn bytes_to_gb(bytes: u64) -> f64 {
    (bytes as f64) / 1024.0 / 1024.0 / 1024.0
}

#[cfg(not(target_os = "windows"))]
fn external_command(program: impl AsRef<OsStr>) -> Command {
    Command::new(program)
}

#[cfg(target_os = "windows")]
fn external_command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    use std::os::windows::process::CommandExt;
    command.creation_flags(windows_hidden_process_creation_flags());
    command
}

fn docker_command() -> Command {
    external_command("docker")
}

#[cfg(target_os = "windows")]
fn windows_hidden_process_creation_flags() -> u32 {
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW
}

const METRICS_DISK_REFRESH_INTERVAL_SECS: u64 = 5;
const NETWORK_TIMELINE_LIMIT: usize = 300;
const DOCKER_OVERVIEW_CACHE_TTL_SECS: u64 = 15;
const GATEWAY_ETC_DIR: &str = "/opt/buckyos/etc";
const GATEWAY_CONFIG_FILES: [&str; 5] = [
    "cyfs_gateway.json",
    "boot_gateway.yaml",
    "node_gateway.json",
    "user_gateway.yaml",
    "post_gateway.yaml",
];
const ZONE_CONFIG_FILES: [&str; 3] = [
    "start_config.json",
    "node_device_config.json",
    "node_identity.json",
];
const SN_SELF_CERT_STATE_PATH: &str = "/opt/buckyos/data/cyfs_gateway/sn_dns/self_cert_state.json";
const CONTROL_PANEL_LOCALE_KEY: &str = "services/control_panel/settings/locale";

#[derive(Clone, Debug)]
struct RpcAuthPrincipal {
    username: String,
    user_type: UserType,
    owner_did: String,
}

#[derive(Clone, Debug, Default)]
struct NetworkStatsSnapshot {
    rx_bytes: u64,
    tx_bytes: u64,
    rx_per_sec: u64,
    tx_per_sec: u64,
    rx_errors: u64,
    tx_errors: u64,
    rx_drops: u64,
    tx_drops: u64,
    interface_count: usize,
    per_interfaces: Vec<InterfaceNetworkStats>,
    updated_at: Option<std::time::SystemTime>,
}

#[derive(Clone, Debug, Default)]
struct InterfaceNetworkStats {
    name: String,
    rx_bytes: u64,
    tx_bytes: u64,
    rx_per_sec: u64,
    tx_per_sec: u64,
    rx_errors: u64,
    tx_errors: u64,
    rx_drops: u64,
    tx_drops: u64,
}

#[derive(Clone, Debug)]
struct MetricsTimelinePoint {
    time: String,
    cpu: u64,
    memory: u64,
    rx: u64,
    tx: u64,
    errors: u64,
    drops: u64,
}

#[derive(Clone, Debug, Default)]
struct NetworkInterfaceTotals {
    rx_bytes: u64,
    tx_bytes: u64,
    rx_errors: u64,
    tx_errors: u64,
    rx_drops: u64,
    tx_drops: u64,
}

#[derive(Clone, Debug, Default)]
struct NetworkCollectionSnapshot {
    rx_bytes: u64,
    tx_bytes: u64,
    rx_errors: u64,
    tx_errors: u64,
    rx_drops: u64,
    tx_drops: u64,
    interface_count: usize,
    per_interfaces: HashMap<String, NetworkInterfaceTotals>,
}

#[derive(Clone, Debug, Default)]
struct ProcNetDevStats {
    rx_errors: u64,
    rx_drops: u64,
    tx_errors: u64,
    tx_drops: u64,
}

#[derive(Clone, Debug, Default)]
struct SystemMetricsSnapshot {
    cpu_usage_percent: f64,
    cpu_brand: String,
    cpu_cores: u64,
    memory_total_bytes: u64,
    memory_used_bytes: u64,
    swap_total_bytes: u64,
    swap_used_bytes: u64,
    load_one: f64,
    load_five: f64,
    load_fifteen: f64,
    process_count: u64,
    uptime_seconds: u64,
    storage_capacity_bytes: u64,
    storage_used_bytes: u64,
    disks_detail: Vec<Value>,
    network: NetworkStatsSnapshot,
    timeline: VecDeque<MetricsTimelinePoint>,
    updated_at: Option<std::time::SystemTime>,
}

#[derive(Clone)]
struct DockerOverviewCacheEntry {
    captured_at: Instant,
    response: Value,
}

#[derive(Clone)]
struct ControlPanelServer {
    log_downloads: Arc<Mutex<HashMap<String, LogDownloadEntry>>>,
    metrics_snapshot: Arc<RwLock<SystemMetricsSnapshot>>,
    pending_sso_logins: Arc<Mutex<HashMap<u64, sys_auth_backend::PendingSsoLogin>>>,
    docker_overview_cache: Arc<Mutex<Option<DockerOverviewCacheEntry>>>,
    docker_overview_refresh_lock: Arc<Mutex<()>>,
    file_manager: Arc<file_manager::BuckyFileServer>,
    app_installer: app_installer::AppInstaller,
    ndm_gateway: Option<Arc<NamedDataMgrZoneGateway>>,
}

impl ControlPanelServer {
    pub fn new() -> Self {
        let metrics_snapshot = Arc::new(RwLock::new(SystemMetricsSnapshot::default()));
        Self::start_metrics_sampler(metrics_snapshot.clone());
        let file_manager_data_dir = get_buckyos_root_dir()
            .join("data")
            .join("control-panel")
            .join("file-manager");
        if let Err(err) = std::fs::create_dir_all(&file_manager_data_dir) {
            log::warn!(
                "failed to create file-manager data dir {}: {}",
                file_manager_data_dir.display(),
                err
            );
        }
        let file_manager = Arc::new(file_manager::BuckyFileServer::new(
            file_manager_data_dir,
            false,
        ));
        ControlPanelServer {
            log_downloads: Arc::new(Mutex::new(HashMap::new())),
            metrics_snapshot,
            pending_sso_logins: Arc::new(Mutex::new(HashMap::new())),
            docker_overview_cache: Arc::new(Mutex::new(None)),
            docker_overview_refresh_lock: Arc::new(Mutex::new(())),
            file_manager,
            app_installer: app_installer::AppInstaller::new(),
            ndm_gateway: None,
        }
    }

    async fn init_file_manager(&self) -> Result<(), RPCErrors> {
        self.file_manager.init_share_db().await
    }

    fn sum_disks(disks: &Disks) -> (u64, u64, Vec<Value>) {
        let mut total_bytes: u64 = 0;
        let mut used_bytes: u64 = 0;
        let mut details: Vec<Value> = Vec::new();

        for disk in disks.list().iter() {
            let total = disk.total_space();
            let available = disk.available_space();
            let used = total.saturating_sub(available);
            total_bytes = total_bytes.saturating_add(total);
            used_bytes = used_bytes.saturating_add(used);
            let usage_percent = if total > 0 {
                ((used as f64 / total as f64) * 100.0).round()
            } else {
                0.0
            };

            details.push(json!({
                "label": disk.name().to_string_lossy(),
                "totalGb": bytes_to_gb(total),
                "usedGb": bytes_to_gb(used),
                "usagePercent": usage_percent,
                "fs": disk.file_system().to_string_lossy(),
                "mount": disk.mount_point().to_string_lossy(),
            }));
        }

        (total_bytes, used_bytes, details)
    }

    fn read_proc_net_dev_stats() -> HashMap<String, ProcNetDevStats> {
        let mut map: HashMap<String, ProcNetDevStats> = HashMap::new();
        let content = match std::fs::read_to_string("/proc/net/dev") {
            Ok(value) => value,
            Err(_) => return map,
        };

        for line in content.lines().skip(2) {
            let (iface_raw, metrics_raw) = match line.split_once(':') {
                Some(value) => value,
                None => continue,
            };
            let iface = iface_raw.trim().to_string();
            if iface.is_empty() || iface == "lo" || iface == "lo0" {
                continue;
            }

            let values: Vec<&str> = metrics_raw.split_whitespace().collect();
            if values.len() < 12 {
                continue;
            }

            let rx_errors = values
                .get(2)
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);
            let rx_drops = values
                .get(3)
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);
            let tx_errors = values
                .get(10)
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);
            let tx_drops = values
                .get(11)
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);

            map.insert(
                iface,
                ProcNetDevStats {
                    rx_errors,
                    rx_drops,
                    tx_errors,
                    tx_drops,
                },
            );
        }

        map
    }

    fn collect_network_snapshot(
        networks: &Networks,
        proc_net_dev_stats: &HashMap<String, ProcNetDevStats>,
    ) -> NetworkCollectionSnapshot {
        let mut snapshot = NetworkCollectionSnapshot::default();

        for (name, data) in networks.iter() {
            let iface = name.as_str();
            if iface == "lo" || iface == "lo0" {
                continue;
            }

            let mut totals = NetworkInterfaceTotals {
                rx_bytes: data.total_received(),
                tx_bytes: data.total_transmitted(),
                rx_errors: data.total_errors_on_received(),
                tx_errors: data.total_errors_on_transmitted(),
                rx_drops: 0,
                tx_drops: 0,
            };

            if let Some(proc_stats) = proc_net_dev_stats.get(iface) {
                totals.rx_errors = proc_stats.rx_errors;
                totals.tx_errors = proc_stats.tx_errors;
                totals.rx_drops = proc_stats.rx_drops;
                totals.tx_drops = proc_stats.tx_drops;
            }

            snapshot.interface_count = snapshot.interface_count.saturating_add(1);
            snapshot.rx_bytes = snapshot.rx_bytes.saturating_add(totals.rx_bytes);
            snapshot.tx_bytes = snapshot.tx_bytes.saturating_add(totals.tx_bytes);
            snapshot.rx_errors = snapshot.rx_errors.saturating_add(totals.rx_errors);
            snapshot.tx_errors = snapshot.tx_errors.saturating_add(totals.tx_errors);
            snapshot.rx_drops = snapshot.rx_drops.saturating_add(totals.rx_drops);
            snapshot.tx_drops = snapshot.tx_drops.saturating_add(totals.tx_drops);
            snapshot.per_interfaces.insert(iface.to_string(), totals);
        }

        snapshot
    }

    fn build_interface_rate_stats(
        current: &HashMap<String, NetworkInterfaceTotals>,
        prev: &HashMap<String, NetworkInterfaceTotals>,
        dt: f64,
    ) -> Vec<InterfaceNetworkStats> {
        let mut names: Vec<&String> = current.keys().collect();
        names.sort();

        names
            .into_iter()
            .filter_map(|name| {
                let now = current.get(name)?;
                let old = prev.get(name);

                let rx_delta = old
                    .map(|value| now.rx_bytes.saturating_sub(value.rx_bytes))
                    .unwrap_or(0);
                let tx_delta = old
                    .map(|value| now.tx_bytes.saturating_sub(value.tx_bytes))
                    .unwrap_or(0);

                let (rx_per_sec, tx_per_sec) = if dt > 0.0 {
                    (
                        ((rx_delta as f64) / dt).round() as u64,
                        ((tx_delta as f64) / dt).round() as u64,
                    )
                } else {
                    (0, 0)
                };

                Some(InterfaceNetworkStats {
                    name: name.to_string(),
                    rx_bytes: now.rx_bytes,
                    tx_bytes: now.tx_bytes,
                    rx_per_sec,
                    tx_per_sec,
                    rx_errors: now.rx_errors,
                    tx_errors: now.tx_errors,
                    rx_drops: now.rx_drops,
                    tx_drops: now.tx_drops,
                })
            })
            .collect()
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

    fn param_usize(req: &RPCRequest, key: &str) -> Option<usize> {
        match req.params.get(key) {
            Some(Value::Number(value)) => {
                value.as_u64().and_then(|value| usize::try_from(value).ok())
            }
            Some(Value::String(value)) => value.trim().parse::<usize>().ok(),
            _ => None,
        }
    }

    fn param_u64(req: &RPCRequest, key: &str) -> Option<u64> {
        match req.params.get(key) {
            Some(Value::Number(value)) => value.as_u64(),
            Some(Value::String(value)) => value.trim().parse::<u64>().ok(),
            _ => None,
        }
    }

    fn param_bool(req: &RPCRequest, key: &str) -> Option<bool> {
        match req.params.get(key) {
            Some(Value::Bool(value)) => Some(*value),
            Some(Value::String(value)) => match value.trim().to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "on" => Some(true),
                "false" | "0" | "no" | "off" => Some(false),
                _ => None,
            },
            _ => None,
        }
    }

    fn require_param_str(req: &RPCRequest, key: &str) -> Result<String, RPCErrors> {
        Self::param_str(req, key).ok_or(RPCErrors::ParseRequestError(format!("Missing {}", key)))
    }

    fn require_rpc_principal(
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<&RpcAuthPrincipal, RPCErrors> {
        principal
            .ok_or_else(|| RPCErrors::InvalidToken("missing authenticated principal".to_string()))
    }

    fn boxed_http_body(bytes: Vec<u8>) -> BoxBody<Bytes, ServerError> {
        Full::new(Bytes::from(bytes))
            .map_err(|never: std::convert::Infallible| match never {})
            .boxed()
    }

    fn build_http_json_response(
        status: StatusCode,
        payload: Value,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let body = serde_json::to_vec(&payload).map_err(|error| {
            server_err!(
                ServerErrorCode::EncodeError,
                "Failed to serialize JSON response: {}",
                error
            )
        })?;
        http::Response::builder()
            .status(status)
            .header(CONTENT_TYPE, "application/json")
            .header(CACHE_CONTROL, "no-store")
            .body(Self::boxed_http_body(body))
            .map_err(|error| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "Failed to build JSON response: {}",
                    error
                )
            })
    }

    fn parse_zone_name_from_did(did: &str) -> Option<String> {
        did.strip_prefix("did:bns:")
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
    }

    async fn resolve_profile_name_from_req(req: &RPCRequest) -> Option<String> {
        if let Ok(runtime) = get_buckyos_api_runtime() {
            let zone_did = runtime.zone_id.to_string();
            if let Some(zone_name) = Self::parse_zone_name_from_did(zone_did.as_str()) {
                return Some(zone_name);
            }

            if let Ok(client) = runtime.get_system_config_client().await {
                if let Ok(boot_config_str) = client.get("boot/config").await {
                    if let Ok(boot_config) =
                        serde_json::from_str::<Value>(boot_config_str.value.as_str())
                    {
                        if let Some(zone_id) =
                            boot_config.get("id").and_then(|value| value.as_str())
                        {
                            if let Some(zone_name) = Self::parse_zone_name_from_did(zone_id) {
                                return Some(zone_name);
                            }
                        }
                    }
                }
            }
        }

        let token_str = req
            .token
            .as_ref()
            .cloned()
            .or_else(|| Self::param_str(req, "session_token"));

        if let Some(token_str) = token_str {
            if let Ok(token) = RPCSessionToken::from_string(token_str.as_str()) {
                if let Some(subject) = token.sub {
                    let subject = subject.trim();
                    if !subject.is_empty() {
                        return Some(subject.to_string());
                    }
                }
            }
        }

        get_buckyos_api_runtime()
            .ok()
            .and_then(|runtime| {
                runtime
                    .user_config
                    .as_ref()
                    .map(|cfg| cfg.name.clone())
                    .or_else(|| runtime.user_id.clone())
                    .or_else(|| runtime.get_owner_user_id())
            })
            .and_then(|name| {
                let name = name.trim().to_string();
                if name.is_empty() {
                    None
                } else {
                    Some(name)
                }
            })
    }

    fn resolve_device_name_from_req(req: &RPCRequest) -> Option<String> {
        if let Ok(runtime) = get_buckyos_api_runtime() {
            if let Some(device_name) = runtime
                .device_config
                .as_ref()
                .map(|device| device.name.clone())
                .or_else(|| runtime.user_id.clone())
            {
                let trimmed = device_name.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }

        let token_str = req
            .token
            .as_ref()
            .cloned()
            .or_else(|| Self::param_str(req, "session_token"));

        if let Some(token_str) = token_str {
            if let Ok(token) = RPCSessionToken::from_string(token_str.as_str()) {
                if let Some(subject) = token.sub {
                    let subject = subject.trim();
                    if !subject.is_empty() {
                        return Some(subject.to_string());
                    }
                }
            }
        }

        None
    }

    async fn handle_layout(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let profile_name = Self::resolve_profile_name_from_req(&req)
            .await
            .unwrap_or_else(|| "Admin User".to_string());
        let profile_email = if let Some(device_name) = Self::resolve_device_name_from_req(&req) {
            format!("{} @ {}", profile_name, device_name)
        } else if profile_name.contains('@') {
            profile_name.clone()
        } else {
            "admin@buckyos.io".to_string()
        };

        let layout = json!({
            "profile": {
                "name": profile_name,
                "email": profile_email,
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

    fn normalize_control_panel_locale(value: Option<&str>) -> String {
        let normalized = value.unwrap_or("en").trim().to_ascii_lowercase();
        match normalized.as_str() {
            "zh" | "zh-cn" => "zh-CN".to_string(),
            "en" | "en-us" | "en-gb" => "en".to_string(),
            _ => "en".to_string(),
        }
    }

    async fn load_control_panel_locale(client: &SystemConfigClient) -> Result<String, RPCErrors> {
        match client.get(CONTROL_PANEL_LOCALE_KEY).await {
            Ok(value) => Ok(Self::normalize_control_panel_locale(Some(&value.value))),
            Err(_) => Ok("en".to_string()),
        }
    }

    async fn handle_ui_locale_get(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        let locale = Self::load_control_panel_locale(&client).await?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "key": CONTROL_PANEL_LOCALE_KEY,
                "locale": locale,
            })),
            req.seq,
        ))
    }

    async fn handle_ui_locale_set(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let requested = Self::param_str(&req, "locale")
            .or_else(|| Self::param_str(&req, "value"))
            .unwrap_or_else(|| "en".to_string());
        let trimmed = requested.trim();
        let locale = Self::normalize_control_panel_locale(Some(trimmed));

        if !trimmed.is_empty() {
            let normalized_requested = trimmed.to_ascii_lowercase();
            let is_supported = matches!(
                normalized_requested.as_str(),
                "en" | "en-us" | "en-gb" | "zh" | "zh-cn"
            );
            if !is_supported {
                return Err(RPCErrors::ReasonError(format!(
                    "unsupported control panel locale: {}",
                    requested
                )));
            }
        }

        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        client
            .set(CONTROL_PANEL_LOCALE_KEY, &locale)
            .await
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "ok": true,
                "key": CONTROL_PANEL_LOCALE_KEY,
                "locale": locale,
            })),
            req.seq,
        ))
    }

    async fn handle_network_overview(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let snapshot = { self.metrics_snapshot.read().await.clone() };

        let timeline: Vec<Value> = snapshot
            .timeline
            .iter()
            .map(|point| {
                json!({
                    "time": point.time,
                    "rx": point.rx,
                    "tx": point.tx,
                    "errors": point.errors,
                    "drops": point.drops,
                })
            })
            .collect();

        let per_interface: Vec<Value> = snapshot
            .network
            .per_interfaces
            .iter()
            .map(|iface| {
                json!({
                    "name": iface.name,
                    "rxBytes": iface.rx_bytes,
                    "txBytes": iface.tx_bytes,
                    "rxPerSec": iface.rx_per_sec,
                    "txPerSec": iface.tx_per_sec,
                    "rxErrors": iface.rx_errors,
                    "txErrors": iface.tx_errors,
                    "rxDrops": iface.rx_drops,
                    "txDrops": iface.tx_drops,
                })
            })
            .collect();

        let response = json!({
            "summary": {
                "rxBytes": snapshot.network.rx_bytes,
                "txBytes": snapshot.network.tx_bytes,
                "rxPerSec": snapshot.network.rx_per_sec,
                "txPerSec": snapshot.network.tx_per_sec,
                "rxErrors": snapshot.network.rx_errors,
                "txErrors": snapshot.network.tx_errors,
                "rxDrops": snapshot.network.rx_drops,
                "txDrops": snapshot.network.tx_drops,
                "interfaceCount": snapshot.network.interface_count,
            },
            "timeline": timeline,
            "perInterface": per_interface,
        });

        Ok(RPCResponse::new(RPCResult::Success(response), req.seq))
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

    fn request_host_from_http_request(
        req: &http::Request<BoxBody<Bytes, ServerError>>,
    ) -> Option<String> {
        req.headers()
            .get(http::header::HOST)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.trim().to_ascii_lowercase())
            .and_then(|value| {
                let host = value.rsplit_once(':').map(|(host, port)| {
                    if port.chars().all(|ch| ch.is_ascii_digit()) {
                        host.to_string()
                    } else {
                        value.clone()
                    }
                });
                host.or(Some(value))
            })
            .and_then(|value| {
                let trimmed = value
                    .trim()
                    .trim_matches('.')
                    .trim_matches('[')
                    .trim_matches(']');
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            })
    }

    async fn serve_rpc_with_req_host(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        info: StreamInfo,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        if req.method() != Method::POST {
            return Ok(http::Response::builder()
                .status(StatusCode::METHOD_NOT_ALLOWED)
                .body(Self::boxed_http_body(b"Method Not Allowed".to_vec()))
                .map_err(|error| {
                    server_err!(
                        ServerErrorCode::BadRequest,
                        "Failed to build response: {}",
                        error
                    )
                })?);
        }

        let req_host = Self::request_host_from_http_request(&req);
        let client_ip = match info.src_addr.as_ref() {
            Some(addr) => match addr.parse::<SocketAddr>() {
                Ok(socket_addr) => socket_addr.ip(),
                Err(error) => {
                    return Ok(http::Response::builder()
                        .status(StatusCode::BAD_REQUEST)
                        .body(Self::boxed_http_body(
                            format!("Bad Request: invalid client ip: {}", error).into_bytes(),
                        ))
                        .map_err(|build_error| {
                            server_err!(
                                ServerErrorCode::BadRequest,
                                "Failed to build response: {}",
                                build_error
                            )
                        })?);
                }
            },
            None => {
                return Ok(http::Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Self::boxed_http_body(b"Bad Request".to_vec()))
                    .map_err(|error| {
                        server_err!(
                            ServerErrorCode::BadRequest,
                            "Failed to build response: {}",
                            error
                        )
                    })?);
            }
        };

        let body_bytes = match req.collect().await {
            Ok(data) => data.to_bytes(),
            Err(error) => {
                return Ok(http::Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Self::boxed_http_body(
                        format!("Failed to read body: {:?}", error).into_bytes(),
                    ))
                    .map_err(|build_error| {
                        server_err!(
                            ServerErrorCode::BadRequest,
                            "Failed to build response: {}",
                            build_error
                        )
                    })?);
            }
        };

        let body_str = match String::from_utf8(body_bytes.to_vec()) {
            Ok(body) => body,
            Err(error) => {
                return Ok(http::Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Self::boxed_http_body(
                        format!("Failed to convert body to string: {}", error).into_bytes(),
                    ))
                    .map_err(|build_error| {
                        server_err!(
                            ServerErrorCode::BadRequest,
                            "Failed to build response: {}",
                            build_error
                        )
                    })?);
            }
        };

        let mut rpc_request: RPCRequest = match serde_json::from_str(body_str.as_str()) {
            Ok(rpc_request) => rpc_request,
            Err(error) => {
                return Ok(http::Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Self::boxed_http_body(
                        format!("Failed to parse request body to RPCRequest: {}", error)
                            .into_bytes(),
                    ))
                    .map_err(|build_error| {
                        server_err!(
                            ServerErrorCode::BadRequest,
                            "Failed to build response: {}",
                            build_error
                        )
                    })?);
            }
        };

        if let Some(req_host) = req_host {
            if let Some(params) = rpc_request.params.as_object_mut() {
                params.insert("x_req_host".to_string(), Value::String(req_host));
            }
        }

        let req_seq = rpc_request.seq;
        let req_trace_id = rpc_request.trace_id.clone();
        let response = match self.handle_rpc_call(rpc_request, client_ip).await {
            Ok(response) => response,
            Err(error) => {
                let mut err_resp = RPCResponse::new(RPCResult::Failed(error.to_string()), req_seq);
                err_resp.trace_id = req_trace_id;
                err_resp
            }
        };

        let body_json = serde_json::to_vec(&response).map_err(|error| {
            server_err!(
                ServerErrorCode::EncodeError,
                "Failed to encode rpc response: {}",
                error
            )
        })?;

        Ok(http::Response::builder()
            .header(CONTENT_TYPE, "application/json")
            .body(Self::boxed_http_body(body_json))
            .map_err(|error| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "Failed to build response: {}",
                    error
                )
            })?)
    }
}

#[async_trait]
impl RPCHandler for ControlPanelServer {
    async fn handle_rpc_call(
        &self,
        mut req: RPCRequest,
        _ip_from: IpAddr,
    ) -> Result<RPCResponse, RPCErrors> {
        if req.token.is_none() {
            req.token = Self::extract_rpc_session_token(&req);
        }

        let principal = self.authenticate_rpc_request(&req).await?;
        if let Some(principal) = principal.as_ref() {
            log::debug!(
                "control-panel rpc auth: method={}, user={}, type={:?}",
                req.method,
                principal.username,
                principal.user_type
            );
        }

        match req.method.as_str() {
            // Core / UI bootstrap
            "main" | "ui.main" => self.handle_main(req).await,
            "layout" | "ui.layout" => self.handle_layout(req).await,

            "ui.locale.get" => self.handle_ui_locale_get(req).await,
            "ui.locale.set" => self.handle_ui_locale_set(req).await,
            // Auth
            "auth.login" => self.handle_auth_login(req).await,

            // User Mgr
            "user.list" => self.handle_user_list(req, principal.as_ref()).await,
            "user.get" => self.handle_user_get(req, principal.as_ref()).await,
            "user.create" => self.handle_user_create(req, principal.as_ref()).await,
            "user.update" => self.handle_user_update(req, principal.as_ref()).await,
            "user.update_contact" => {
                self.handle_user_update_contact(req, principal.as_ref())
                    .await
            }
            "user.delete" => self.handle_user_delete(req, principal.as_ref()).await,

            "user.change_password" => {
                self.handle_user_change_password(req, principal.as_ref())
                    .await
            }
            "user.change_state" => self.handle_user_change_state(req, principal.as_ref()).await,
            "user.change_type" => self.handle_user_change_type(req, principal.as_ref()).await,

            "agent.list" => self.handle_agent_list(req, principal.as_ref()).await,
            "agent.get" => self.handle_agent_get(req, principal.as_ref()).await,
            "agent.set_msg_tunnel" => {
                self.handle_agent_set_msg_tunnel(req, principal.as_ref())
                    .await
            }
            "agent.remove_msg_tunnel" => {
                self.handle_agent_remove_msg_tunnel(req, principal.as_ref())
                    .await
            }
            // System dashboard
            "dashboard" | "ui.dashboard" => self.handle_dashboard(req).await,
            "system.overview" => self.handle_system_overview(req).await,
            "system.status" => self.handle_system_status(req).await,
            "system.metrics" => self.handle_system_metrics(req).await,

            //SystemLogs
            "system.logs.list" => self.handle_system_logs_list(req).await,
            "system.logs.query" => self.handle_system_logs_query(req).await,
            "system.logs.tail" => self.handle_system_logs_tail(req).await,
            "system.logs.download" => self.handle_system_logs_download(req).await,
            "system.update.check" => self.handle_unimplemented(req, "Check updates").await,
            "system.update.apply" => self.handle_unimplemented(req, "Apply update").await,

            // AICC
            "ai.overview" => self.handle_ai_overview(req).await,
            "ai.provider.list" => self.handle_ai_provider_list(req).await,
            "ai.provider.set" => self.handle_ai_provider_set(req).await,
            "ai.provider.test" => self.handle_ai_provider_test(req).await,
            "ai.message_hub.thread_summary" => {
                self.handle_ai_message_hub_thread_summary(req, principal.as_ref())
                    .await
            }
            "ai.reload" => self.handle_ai_reload(req).await,
            "ai.model.list" => self.handle_ai_model_list(req).await,
            "ai.model.set" => self.handle_ai_model_set(req).await,
            "ai.policy.list" => self.handle_ai_policy_list(req).await,
            "ai.policy.set" => self.handle_ai_policy_set(req).await,
            "ai.diagnostics.list" => self.handle_ai_diagnostics_list(req).await,

            //AppMgr
            "apps.list" => self.handle_apps_list(req, principal.as_ref()).await,
            "apps.details" | "app.details" => {
                self.handle_app_detials(req, principal.as_ref()).await
            }
            //"apps.version.list" => self.handle_apps_version_list(req).await,
            //AppInstaller
            "apps.install" => self.handle_apps_install(req, principal.as_ref()).await,
            "apps.update" => self.handle_apps_update(req, principal.as_ref()).await,
            "apps.uninstall" => self.handle_apps_uninstall(req, principal.as_ref()).await,
            "apps.start" => self.handle_apps_start(req, principal.as_ref()).await,
            "apps.stop" => self.handle_apps_stop(req, principal.as_ref()).await,
            "app.publish" => self.handle_app_publish(req, principal.as_ref()).await,

            // MessageHub
            "chat.bootstrap" => self.handle_chat_bootstrap(req, principal.as_ref()).await,
            "chat.contact.list" => self.handle_chat_contact_list(req, principal.as_ref()).await,
            "chat.message.list" => self.handle_chat_message_list(req, principal.as_ref()).await,
            "chat.message.send" => self.handle_chat_message_send(req, principal.as_ref()).await,

            //ZoneMgr
            "zone.overview" | "zone.config" => self.handle_zone_overview(req).await,
            "gateway.overview" | "gateway.config" => self.handle_gateway_overview(req).await,
            "gateway.file.get" => self.handle_gateway_file_get(req).await,
            "container.overview" | "containers.overview" | "docker.overview" => {
                self.handle_container_overview(req).await
            }
            "container.action" | "containers.action" | "docker.action" => {
                self.handle_container_action(req).await
            }

            // System Config
            "system.config.test" => self.handle_system_config_test(req).await,
            "sys_config.get" => self.handle_sys_config_get(req).await,
            "sys_config.set" => self.handle_sys_config_set(req).await,
            "sys_config.list" => self.handle_sys_config_list(req).await,
            "sys_config.tree" => self.handle_sys_config_tree(req).await,
            "sys_config.history" => self.handle_unimplemented(req, "Config history").await,

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
        let method = req.method().clone();
        let path = req.uri().path().to_string();

        if method == Method::GET && path == "/sso_callback" {
            return self.serve_sso_callback(req, info).await;
        }
        if method == Method::POST && path == "/sso_refresh" {
            return self.serve_sso_refresh(req, info).await;
        }
        if method == Method::POST && path == "/sso_logout" {
            return self.serve_sso_logout(req, info).await;
        }

        if path == "/api/desktop" || path.starts_with("/api/desktop/") {
            return self.handle_desktop_api(req).await;
        }

        if path == "/api" || path.starts_with("/api/") {
            return self.file_manager.serve_request(req, info).await;
        }

        if method == Method::POST
            && (path == "/kapi/control-panel/chat/stream"
                || path == "/kapi/message-hub/chat/stream")
        {
            return self.handle_chat_stream_http(req).await;
        }

        if method == Method::POST
            && (path.starts_with("/kapi/control-panel") || path.starts_with("/kapi/message-hub"))
        {
            return self.serve_rpc_with_req_host(req, info).await;
        }
        if method == Method::GET {
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
    set_buckyos_api_runtime(runtime)
        .map_err(|err| anyhow::anyhow!("register control-panel runtime failed: {}", err))?;

    let mut control_panel_server = ControlPanelServer::new();
    control_panel_server
        .init_file_manager()
        .await
        .map_err(|err| anyhow::anyhow!("init control-panel file manager failed: {}", err))?;

    // 初始化 NDM Zone Gateway（best-effort，named store 不可用时跳过）
    let runtime = get_buckyos_api_runtime()
        .map_err(|err| anyhow::anyhow!("get runtime for ndm gateway failed: {}", err))?;
    match runtime.get_named_store().await {
        Ok(store_mgr) => {
            let ndm_cache_dir = runtime
                .get_cache_folder()
                .unwrap_or_else(|_| get_buckyos_root_dir().join("cache").join("control-panel"))
                .join("ndm_upload_cache");
            let ndm_config = NdmZoneGatewayConfig {
                cache_dir: ndm_cache_dir,
                ..Default::default()
            };
            let ndm_gw = Arc::new(NamedDataMgrZoneGateway::new(
                Arc::new(store_mgr),
                ndm_config,
            ));
            control_panel_server.ndm_gateway = Some(ndm_gw);
            info!("NDM zone gateway initialized");
        }
        Err(e) => {
            log::warn!(
                "NDM zone gateway not available (named store not ready: {}), ndm upload disabled",
                e
            );
        }
    }

    let control_panel_server = Arc::new(control_panel_server);
    // Bind to the default control-panel service port.

    let runner = Runner::new(CONTROL_PANEL_SERVICE_PORT);
    // 添加 RPC 服务
    let _ = runner.add_http_server(
        "/kapi/control-panel".to_string(),
        control_panel_server.clone(),
    );
    let _ = runner.add_http_server("/sso_callback".to_string(), control_panel_server.clone());
    let _ = runner.add_http_server("/sso_refresh".to_string(), control_panel_server.clone());
    let _ = runner.add_http_server("/sso_logout".to_string(), control_panel_server.clone());
    // File manager API exposed by control-panel.
    let _ = runner.add_http_server("/api".to_string(), control_panel_server.clone());

    // NDM zone gateway: 注册 /ndm 路径，供系统 App 使用 NDM 上传协议
    if let Some(ref ndm_gw) = control_panel_server.ndm_gateway {
        let _ = runner.add_http_server("/ndm".to_string(), ndm_gw.clone());
        info!("NDM zone gateway registered at /ndm");
    }

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
