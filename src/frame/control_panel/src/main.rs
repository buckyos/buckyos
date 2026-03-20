mod app_installer;
mod file_manager;

use ::kRPC::*;
use anyhow::Result;
use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use buckyos_api::{
    get_buckyos_api_runtime, init_buckyos_api_runtime, set_buckyos_api_runtime, AccessGroupLevel,
    AiMessage, AiPayload, AppDoc, AppServiceSpec, AppType, BoxKind, BuckyOSRuntimeType, Capability,
    CompleteRequest, Contact, ContactQuery, Event, KEventClient, ModelSpec, MsgCenterClient,
    LoginByPasswordResponse, MsgRecordWithObject, MsgState, RepoListFilter, RepoRecord, Requirements, SendContext,
    ServiceExposeConfig, ServiceInstallConfig, ServiceState, SystemConfigClient, UserType,
    CONTROL_PANEL_SERVICE_NAME, CONTROL_PANEL_SERVICE_PORT,
};
use buckyos_kit::*;
use bytes::Bytes;
use chrono::{DateTime, Datelike, NaiveDateTime, TimeZone, Utc};
use cyfs_gateway_lib::*;
use futures::{stream, TryStreamExt};
use http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE};
use http::{Method, StatusCode, Version};
use http_body_util::{combinators::BoxBody, BodyExt, Full, StreamBody};
use hyper::body::Frame;
use log::{info, warn};
use name_lib::{load_private_key, DID};
use ndn_lib::{MsgContent, MsgContentFormat, MsgObjKind, MsgObject};
use semver::Version as SemVer;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::*;
use server_runner::*;
use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};
use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, OnceLock};
use std::{
    net::IpAddr,
    time::{Duration, Instant},
};
use sysinfo::{DiskRefreshKind, Disks, Networks, System};
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::task;
use uuid::Uuid;
use zip::write::FileOptions;
use zip::CompressionMethod;

// RPC docs live under doc/dashboard. UI endpoints use "ui.*" as canonical names;
// "main/layout/dashboard" are kept as legacy aliases.

fn bytes_to_gb(bytes: u64) -> f64 {
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

const LOG_ROOT_DIR: &str = "/opt/buckyos/logs";
const LOG_DOWNLOAD_TTL_SECS: u64 = 600;
const DEFAULT_LOG_LIMIT: usize = 200;
const MAX_LOG_LIMIT: usize = 1000;
const METRICS_DISK_REFRESH_INTERVAL_SECS: u64 = 5;
const NETWORK_TIMELINE_LIMIT: usize = 300;
const SYS_CONFIG_TREE_MAX_DEPTH: u64 = 24;
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
const CONTROL_PANEL_AUTH_APPID: &str = "control-panel";
const CONTROL_PANEL_SSO_TOKEN_EXPIRE_SECONDS: u64 = 15 * 60;
const SN_SELF_CERT_STATE_PATH: &str = "/opt/buckyos/data/cyfs_gateway/sn_dns/self_cert_state.json";
const DEFAULT_CHAT_CONTACT_LIMIT: usize = 100;
const DEFAULT_CHAT_MESSAGE_LIMIT: usize = 60;
const MAX_CHAT_MESSAGE_LIMIT: usize = 100;
const MAX_CHAT_SCAN_LIMIT: usize = 240;
const DEFAULT_CHAT_STREAM_KEEPALIVE_MS: u64 = 15_000;
const MIN_CHAT_STREAM_KEEPALIVE_MS: u64 = 5_000;
const MAX_CHAT_STREAM_KEEPALIVE_MS: u64 = 60_000;
const AICC_SETTINGS_KEY: &str = "services/aicc/settings";
const CONTROL_PANEL_LOCALE_KEY: &str = "services/control_panel/settings/locale";
const AI_MODELS_POLICIES_KEY: &str = "services/control_panel/ai_models/policies";
const AI_MODELS_PROVIDER_OVERRIDES_KEY: &str =
    "services/control_panel/ai_models/provider_overrides";
const AI_MODELS_MODEL_CATALOG_KEY: &str = "services/control_panel/ai_models/model_catalog";
const AI_MODELS_PROVIDER_SECRETS_KEY: &str = "services/control_panel/ai_models/provider_secrets";

#[derive(Clone, Debug)]
struct RpcAuthPrincipal {
    username: String,
    user_type: UserType,
    owner_did: String,
}

#[derive(Clone, Serialize)]
struct ChatScopeInfo {
    username: String,
    owner_did: String,
    access_mode: &'static str,
}

#[derive(Clone, Serialize)]
struct ChatCapabilityInfo {
    contact_list: bool,
    message_list: bool,
    message_send: bool,
    thread_id_send: bool,
    realtime_events: bool,
    standalone_chat_app_link: bool,
    opendan_channel_ready: bool,
}

#[derive(Clone, Serialize)]
struct ChatBootstrapResponse {
    scope: ChatScopeInfo,
    capabilities: ChatCapabilityInfo,
    notes: Vec<String>,
}

#[derive(Serialize)]
struct AuthLoginResponse {
    #[serde(flatten)]
    login_result: LoginByPasswordResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    sso_token: Option<String>,
}

#[derive(Clone, Serialize)]
struct ChatBindingView {
    platform: String,
    account_id: String,
    display_id: String,
    tunnel_id: String,
    last_active_at: u64,
    meta: HashMap<String, String>,
}

#[derive(Clone, Serialize)]
struct ChatContactView {
    did: String,
    name: String,
    avatar: Option<String>,
    note: Option<String>,
    access_level: &'static str,
    is_verified: bool,
    groups: Vec<String>,
    tags: Vec<String>,
    created_at: u64,
    updated_at: u64,
    bindings: Vec<ChatBindingView>,
}

#[derive(Clone, Serialize)]
struct ChatContactListResponse {
    scope: ChatScopeInfo,
    items: Vec<ChatContactView>,
}

#[derive(Clone, Serialize)]
struct ChatMessageView {
    record_id: String,
    msg_id: String,
    direction: &'static str,
    peer_did: String,
    peer_name: Option<String>,
    state: &'static str,
    created_at_ms: u64,
    updated_at_ms: u64,
    sort_key: u64,
    thread_id: Option<String>,
    content: String,
    content_format: Option<String>,
}

#[derive(Clone, Serialize)]
struct ChatMessageListResponse {
    scope: ChatScopeInfo,
    peer_did: String,
    peer_name: Option<String>,
    items: Vec<ChatMessageView>,
}

#[derive(Clone, Serialize)]
struct ChatSendMessageResponse {
    scope: ChatScopeInfo,
    target_did: String,
    delivery_count: usize,
    message: ChatMessageView,
}

#[derive(Clone, Serialize)]
struct MessageHubThreadSummaryResponse {
    peer_did: String,
    peer_name: Option<String>,
    model_alias: String,
    summary: String,
    source_message_count: usize,
}

#[derive(Clone, Deserialize)]
struct ChatStreamHttpRequest {
    #[serde(default)]
    session_token: Option<String>,
    peer_did: String,
    #[serde(default)]
    thread_id: Option<String>,
    #[serde(default)]
    keepalive_ms: Option<u64>,
}

#[derive(Clone, Deserialize)]
struct MsgCenterBoxChangedEvent {
    #[serde(default)]
    operation: Option<String>,
    #[serde(default)]
    record_id: Option<String>,
}

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

struct RepoAppReleaseCandidate {
    record: RepoRecord,
    app_doc: AppDoc,
    parsed_version: Option<SemVer>,
}

#[derive(Clone)]
struct ControlPanelServer {
    log_downloads: Arc<Mutex<HashMap<String, LogDownloadEntry>>>,
    metrics_snapshot: Arc<RwLock<SystemMetricsSnapshot>>,
    file_manager: Arc<file_manager::BuckyFileServer>,
    app_installer: app_installer::AppInstaller,
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
            file_manager,
            app_installer: app_installer::AppInstaller::new(),
        }
    }

    async fn init_file_manager(&self) -> Result<(), RPCErrors> {
        self.file_manager.init_share_db().await
    }

    fn start_metrics_sampler(metrics_snapshot: Arc<RwLock<SystemMetricsSnapshot>>) {
        // Background sampler so metrics timelines are produced server-side.
        tokio::spawn(async move {
            let mut system = System::new_all();
            let mut networks = Networks::new_with_refreshed_list();
            let mut disks = Disks::new_with_refreshed_list_specifics(DiskRefreshKind::everything());

            system.refresh_memory();
            system.refresh_cpu_usage();
            networks.refresh(true);
            disks.refresh(true);

            let cpu_brand = system
                .cpus()
                .first()
                .map(|cpu| cpu.brand().to_string())
                .unwrap_or_else(|| "Unknown CPU".to_string());
            let cpu_cores = system.cpus().len() as u64;

            let (storage_capacity_bytes, storage_used_bytes, disks_detail) =
                ControlPanelServer::sum_disks(&disks);

            let mut proc_net_dev_stats = ControlPanelServer::read_proc_net_dev_stats();
            let mut network_totals =
                ControlPanelServer::collect_network_snapshot(&networks, &proc_net_dev_stats);
            let mut prev_interfaces = network_totals.per_interfaces.clone();
            let mut prev_rx = network_totals.rx_bytes;
            let mut prev_tx = network_totals.tx_bytes;
            let mut prev_error_total = network_totals
                .rx_errors
                .saturating_add(network_totals.tx_errors);
            let mut prev_drop_total = network_totals
                .rx_drops
                .saturating_add(network_totals.tx_drops);
            {
                let mut snapshot = metrics_snapshot.write().await;
                snapshot.cpu_brand = cpu_brand;
                snapshot.cpu_cores = cpu_cores;
                snapshot.memory_total_bytes = system.total_memory();
                snapshot.memory_used_bytes = system.used_memory();
                snapshot.swap_total_bytes = system.total_swap();
                snapshot.swap_used_bytes = system.used_swap();
                snapshot.load_one = System::load_average().one;
                snapshot.load_five = System::load_average().five;
                snapshot.load_fifteen = System::load_average().fifteen;
                snapshot.process_count = system.processes().len() as u64;
                snapshot.uptime_seconds = System::uptime();
                snapshot.storage_capacity_bytes = storage_capacity_bytes;
                snapshot.storage_used_bytes = storage_used_bytes;
                snapshot.disks_detail = disks_detail;
                snapshot.network.rx_bytes = network_totals.rx_bytes;
                snapshot.network.tx_bytes = network_totals.tx_bytes;
                snapshot.network.rx_per_sec = 0;
                snapshot.network.tx_per_sec = 0;
                snapshot.network.rx_errors = network_totals.rx_errors;
                snapshot.network.tx_errors = network_totals.tx_errors;
                snapshot.network.rx_drops = network_totals.rx_drops;
                snapshot.network.tx_drops = network_totals.tx_drops;
                snapshot.network.interface_count = network_totals.interface_count;
                snapshot.network.per_interfaces = ControlPanelServer::build_interface_rate_stats(
                    &network_totals.per_interfaces,
                    &prev_interfaces,
                    0.0,
                );
                snapshot.network.updated_at = Some(std::time::SystemTime::now());
                snapshot.updated_at = Some(std::time::SystemTime::now());
            }

            let mut last_at = Instant::now();
            let mut ticker = tokio::time::interval(Duration::from_secs(1));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut disk_refresh_counter: u64 = 0;

            loop {
                ticker.tick().await;

                system.refresh_memory();
                system.refresh_cpu_usage();
                networks.refresh(true);
                proc_net_dev_stats = ControlPanelServer::read_proc_net_dev_stats();

                disk_refresh_counter = disk_refresh_counter.saturating_add(1);
                let refresh_disks = disk_refresh_counter % METRICS_DISK_REFRESH_INTERVAL_SECS == 0;
                if refresh_disks {
                    disks.refresh(true);
                }

                network_totals =
                    ControlPanelServer::collect_network_snapshot(&networks, &proc_net_dev_stats);
                let dt = last_at.elapsed().as_secs_f64();
                last_at = Instant::now();

                let rx_delta = network_totals.rx_bytes.saturating_sub(prev_rx);
                let tx_delta = network_totals.tx_bytes.saturating_sub(prev_tx);
                prev_rx = network_totals.rx_bytes;
                prev_tx = network_totals.tx_bytes;

                let error_total = network_totals
                    .rx_errors
                    .saturating_add(network_totals.tx_errors);
                let drop_total = network_totals
                    .rx_drops
                    .saturating_add(network_totals.tx_drops);
                let error_delta = error_total.saturating_sub(prev_error_total);
                let drop_delta = drop_total.saturating_sub(prev_drop_total);
                prev_error_total = error_total;
                prev_drop_total = drop_total;

                let (rx_per_sec, tx_per_sec) = if dt > 0.0 {
                    (
                        ((rx_delta as f64) / dt).round() as u64,
                        ((tx_delta as f64) / dt).round() as u64,
                    )
                } else {
                    (0, 0)
                };

                let total_memory_bytes = system.total_memory();
                let used_memory_bytes = system.used_memory();
                let memory_percent = if total_memory_bytes > 0 {
                    ((used_memory_bytes as f64 / total_memory_bytes as f64) * 100.0).round() as u64
                } else {
                    0
                };
                let cpu_usage_percent = system.global_cpu_usage() as f64;
                let cpu_percent = cpu_usage_percent.round() as u64;

                let now = Utc::now();
                let time_label = now.format("%H:%M:%S").to_string();

                let mut snapshot = metrics_snapshot.write().await;
                snapshot.cpu_usage_percent = cpu_usage_percent;
                snapshot.memory_total_bytes = total_memory_bytes;
                snapshot.memory_used_bytes = used_memory_bytes;
                snapshot.swap_total_bytes = system.total_swap();
                snapshot.swap_used_bytes = system.used_swap();

                let load_avg = System::load_average();
                snapshot.load_one = load_avg.one;
                snapshot.load_five = load_avg.five;
                snapshot.load_fifteen = load_avg.fifteen;
                snapshot.process_count = system.processes().len() as u64;
                snapshot.uptime_seconds = System::uptime();

                if refresh_disks {
                    let (capacity, used, details) = ControlPanelServer::sum_disks(&disks);
                    snapshot.storage_capacity_bytes = capacity;
                    snapshot.storage_used_bytes = used;
                    snapshot.disks_detail = details;
                }

                snapshot.network.rx_bytes = network_totals.rx_bytes;
                snapshot.network.tx_bytes = network_totals.tx_bytes;
                snapshot.network.rx_per_sec = rx_per_sec;
                snapshot.network.tx_per_sec = tx_per_sec;
                snapshot.network.rx_errors = network_totals.rx_errors;
                snapshot.network.tx_errors = network_totals.tx_errors;
                snapshot.network.rx_drops = network_totals.rx_drops;
                snapshot.network.tx_drops = network_totals.tx_drops;
                snapshot.network.interface_count = network_totals.interface_count;
                snapshot.network.per_interfaces = ControlPanelServer::build_interface_rate_stats(
                    &network_totals.per_interfaces,
                    &prev_interfaces,
                    dt,
                );
                snapshot.network.updated_at = Some(std::time::SystemTime::now());
                prev_interfaces = network_totals.per_interfaces.clone();

                if snapshot.timeline.len() >= NETWORK_TIMELINE_LIMIT {
                    snapshot.timeline.pop_front();
                }
                snapshot.timeline.push_back(MetricsTimelinePoint {
                    time: time_label,
                    cpu: cpu_percent.min(100),
                    memory: memory_percent.min(100),
                    rx: rx_per_sec,
                    tx: tx_per_sec,
                    errors: if dt > 0.0 {
                        ((error_delta as f64) / dt).round() as u64
                    } else {
                        0
                    },
                    drops: if dt > 0.0 {
                        ((drop_delta as f64) / dt).round() as u64
                    } else {
                        0
                    },
                });
                snapshot.updated_at = Some(std::time::SystemTime::now());
            }
        });
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

    async fn handle_auth_login(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let username = Self::require_param_str(&req, "username")?;
        let password = Self::require_param_str(&req, "password")?;
        let redirect_url = Self::param_str(&req, "redirect_url");
        let appid = Self::param_str(&req, "appid")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| CONTROL_PANEL_AUTH_APPID.to_string());
        let login_nonce = req
            .params
            .get("login_nonce")
            .and_then(|value| value.as_u64())
            .or(Some(req.seq));

        let runtime = get_buckyos_api_runtime()?;
        let verify_hub_client = runtime.get_verify_hub_client().await?;
        let login_result = verify_hub_client
            .login_by_password(username, password, appid, login_nonce)
            .await?;
        let sso_token = Self::resolve_sso_target_appid(
            redirect_url.as_deref(),
            runtime.zone_id.to_host_name().as_str(),
        )?
        .map(|target_appid| {
            let issuer = Self::resolve_local_device_name(runtime)?;
            Self::issue_gateway_sso_token(
                issuer.as_str(),
                login_result.user_info.user_id.as_str(),
                target_appid.as_str(),
            )
        })
        .transpose()?;
        let response = AuthLoginResponse {
            login_result,
            sso_token,
        };

        Ok(RPCResponse::new(
            RPCResult::Success(json!(response)),
            req.seq,
        ))
    }

    async fn handle_auth_issue_sso_token(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let redirect_url = Self::require_param_str(&req, "redirect_url")?;
        let session_token = Self::extract_rpc_session_token(&req)
            .ok_or_else(|| RPCErrors::ParseRequestError("Missing session_token".to_string()))?;

        let runtime = get_buckyos_api_runtime()?;
        let verify_hub_client = runtime.get_verify_hub_client().await?;
        let verified = verify_hub_client
            .verify_token(session_token.as_str(), Some(CONTROL_PANEL_AUTH_APPID))
            .await?;
        if !verified {
            return Err(RPCErrors::InvalidToken(
                "Invalid control-panel session token".to_string(),
            ));
        }

        let target_appid = Self::resolve_sso_target_appid(
            Some(redirect_url.as_str()),
            runtime.zone_id.to_host_name().as_str(),
        )?
        .ok_or_else(|| RPCErrors::ParseRequestError("Missing redirect_url".to_string()))?;
        let session_token = RPCSessionToken::from_string(session_token.as_str())?;
        let user_id = session_token
            .sub
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| RPCErrors::InvalidToken("session token missing subject".to_string()))?;
        let issuer = Self::resolve_local_device_name(runtime)?;
        let sso_token =
            Self::issue_gateway_sso_token(issuer.as_str(), user_id.as_str(), target_appid.as_str())?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({ "sso_token": sso_token })),
            req.seq,
        ))
    }

    async fn handle_auth_refresh(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let refresh_token = Self::require_param_str(&req, "refresh_token")?;

        let runtime = get_buckyos_api_runtime()?;
        let verify_hub_client = runtime.get_verify_hub_client().await?;
        let token_pair = verify_hub_client
            .refresh_token(refresh_token.as_str())
            .await?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!(token_pair)),
            req.seq,
        ))
    }

    async fn handle_auth_verify(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let session_token = Self::extract_rpc_session_token(&req)
            .ok_or_else(|| RPCErrors::ParseRequestError("Missing session_token".to_string()))?;
        let appid = Self::param_str(&req, "appid");

        let runtime = get_buckyos_api_runtime()?;
        let verify_hub_client = runtime.get_verify_hub_client().await?;
        let verified = verify_hub_client
            .verify_token(session_token.as_str(), appid.as_deref())
            .await?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!(verified)),
            req.seq,
        ))
    }

    async fn handle_auth_logout(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        Ok(RPCResponse::new(
            RPCResult::Success(json!({ "ok": true })),
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

    fn normalize_session_token(token: Option<String>) -> Option<String> {
        token
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    fn require_param_str(req: &RPCRequest, key: &str) -> Result<String, RPCErrors> {
        Self::param_str(req, key).ok_or(RPCErrors::ParseRequestError(format!("Missing {}", key)))
    }

    fn is_public_rpc_method(method: &str) -> bool {
        matches!(
            method,
            "auth.login" | "auth.refresh" | "auth.verify" | "auth.logout" | "auth.issue_sso_token"
        )
    }

    fn resolve_local_device_name(
        runtime: &buckyos_api::BuckyOSRuntime,
    ) -> Result<String, RPCErrors> {
        runtime
            .device_config
            .as_ref()
            .map(|value| value.name.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| RPCErrors::ReasonError("missing local device name".to_string()))
    }

    fn resolve_sso_target_appid(
        redirect_url: Option<&str>,
        zone_host: &str,
    ) -> Result<Option<String>, RPCErrors> {
        let redirect_url = match redirect_url.map(|value| value.trim()) {
            Some(value) if !value.is_empty() => value,
            _ => return Ok(None),
        };

        let zone_host = zone_host.trim().trim_matches('.').to_ascii_lowercase();
        if zone_host.is_empty() {
            return Err(RPCErrors::ReasonError("missing zone host".to_string()));
        }

        let url = url::Url::parse(redirect_url).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Invalid redirect_url: {}", error))
        })?;
        let host = url
            .host_str()
            .map(|value| value.trim().trim_matches('.').to_ascii_lowercase())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| RPCErrors::ParseRequestError("redirect_url missing host".to_string()))?;

        let app_key = if host == zone_host {
            "_".to_string()
        } else {
            let suffix = format!(".{}", zone_host);
            let prefix = host.strip_suffix(suffix.as_str()).ok_or_else(|| {
                RPCErrors::ParseRequestError("redirect_url host is outside current zone".to_string())
            })?;
            prefix
                .split(['.', '-'])
                .next()
                .map(|value| value.trim().to_string())
                .filter(|value| {
                    !value.is_empty()
                        && value
                            .chars()
                            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
                })
                .ok_or_else(|| {
                    RPCErrors::ParseRequestError("redirect_url host does not resolve to an app".to_string())
                })?
        };

        Self::lookup_gateway_appid(app_key.as_str()).map(Some)
    }

    fn lookup_gateway_appid(app_key: &str) -> Result<String, RPCErrors> {
        let gateway_info_path = Path::new(GATEWAY_ETC_DIR).join("node_gateway_info.json");
        let content = std::fs::read_to_string(gateway_info_path.as_path()).map_err(|error| {
            RPCErrors::ReasonError(format!("read node_gateway_info.json failed: {}", error))
        })?;
        let value: Value = serde_json::from_str(content.as_str()).map_err(|error| {
            RPCErrors::ReasonError(format!("parse node_gateway_info.json failed: {}", error))
        })?;
        let app_info = value
            .get("app_info")
            .and_then(|value| value.get(app_key))
            .ok_or_else(|| {
                RPCErrors::ParseRequestError(format!(
                    "redirect_url app '{}' is not present in gateway info",
                    app_key
                ))
            })?;

        app_info
            .get("app_id")
            .and_then(Value::as_str)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                RPCErrors::ParseRequestError(format!(
                    "redirect_url app '{}' does not have a routable app_id",
                    app_key
                ))
            })
    }

    fn issue_gateway_sso_token(
        issuer: &str,
        user_id: &str,
        appid: &str,
    ) -> Result<String, RPCErrors> {
        let key_path = Path::new(GATEWAY_ETC_DIR).join("node_private_key.pem");
        let private_key = load_private_key(key_path.as_path()).map_err(|error| {
            RPCErrors::ReasonError(format!("load node private key failed: {}", error))
        })?;
        let session_token = RPCSessionToken {
            token_type: RPCSessionTokenType::JWT,
            token: None,
            aud: None,
            exp: Some(buckyos_get_unix_timestamp() + CONTROL_PANEL_SSO_TOKEN_EXPIRE_SECONDS),
            iss: Some(issuer.to_string()),
            jti: Some(Uuid::new_v4().to_string()),
            session: None,
            sub: Some(user_id.to_string()),
            appid: Some(appid.to_string()),
            extra: HashMap::new(),
        };

        session_token.generate_jwt(None, &private_key)
    }

    fn extract_rpc_session_token(req: &RPCRequest) -> Option<String> {
        Self::normalize_session_token(req.token.clone())
            .or_else(|| Self::normalize_session_token(Self::param_str(req, "session_token")))
    }

    fn extract_http_session_token(
        req: &http::Request<BoxBody<Bytes, ServerError>>,
    ) -> Option<String> {
        if let Some(value) = req.headers().get("X-Auth") {
            if let Ok(token) = value.to_str() {
                if let Some(token) = Self::normalize_session_token(Some(token.to_string())) {
                    return Some(token);
                }
            }
        }

        if let Some(value) = req.headers().get(http::header::AUTHORIZATION) {
            if let Ok(raw) = value.to_str() {
                if let Some(token) = raw.strip_prefix("Bearer ") {
                    if let Some(token) = Self::normalize_session_token(Some(token.to_string())) {
                        return Some(token);
                    }
                }
            }
        }

        if let Some(query) = req.uri().query() {
            for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
                if key == "auth" || key == "session_token" {
                    if let Some(token) = Self::normalize_session_token(Some(value.to_string())) {
                        return Some(token);
                    }
                }
            }
        }

        if let Some(cookie_header) = req.headers().get("Cookie") {
            if let Ok(raw_cookie) = cookie_header.to_str() {
                for piece in raw_cookie.split(';') {
                    let segment = piece.trim();
                    for key in ["auth=", "control-panel_token=", "control_panel_token="] {
                        if let Some(token) = segment.strip_prefix(key) {
                            if let Some(token) =
                                Self::normalize_session_token(Some(token.to_string()))
                            {
                                return Some(token);
                            }
                        }
                    }
                }
            }
        }

        None
    }

    async fn authenticate_session_token_for_method(
        &self,
        method: &str,
        token: Option<String>,
    ) -> Result<Option<RpcAuthPrincipal>, RPCErrors> {
        if Self::is_public_rpc_method(method) {
            return Ok(None);
        }

        let token = Self::normalize_session_token(token)
            .ok_or_else(|| RPCErrors::InvalidToken("missing session token".to_string()))?;

        let runtime = get_buckyos_api_runtime()?;
        let parsed = runtime.verify_trusted_session_token(&token).await?;
        let username = parsed
            .sub
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| RPCErrors::InvalidToken("session token missing subject".to_string()))?;
        let owner_did = DID::new("bns", &username).to_string();

        Ok(Some(RpcAuthPrincipal {
            username,
            user_type: UserType::Root,
            owner_did,
        }))
    }

    async fn authenticate_rpc_request(
        &self,
        req: &RPCRequest,
    ) -> Result<Option<RpcAuthPrincipal>, RPCErrors> {
        self.authenticate_session_token_for_method(
            req.method.as_str(),
            Self::extract_rpc_session_token(req),
        )
        .await
    }

    fn require_chat_principal<'a>(
        principal: Option<&'a RpcAuthPrincipal>,
    ) -> Result<&'a RpcAuthPrincipal, RPCErrors> {
        principal
            .ok_or_else(|| RPCErrors::InvalidToken("missing authenticated principal".to_string()))
    }

    fn require_rpc_principal<'a>(
        principal: Option<&'a RpcAuthPrincipal>,
    ) -> Result<&'a RpcAuthPrincipal, RPCErrors> {
        principal
            .ok_or_else(|| RPCErrors::InvalidToken("missing authenticated principal".to_string()))
    }

    fn resolve_target_user_id(req: &RPCRequest, principal: &RpcAuthPrincipal) -> String {
        Self::param_str(req, "user_id").unwrap_or_else(|| principal.username.clone())
    }

    fn parse_chat_owner_did(principal: &RpcAuthPrincipal) -> Result<DID, RPCErrors> {
        DID::from_str(principal.owner_did.as_str()).map_err(|error| {
            RPCErrors::ReasonError(format!(
                "invalid chat owner DID `{}`: {}",
                principal.owner_did, error
            ))
        })
    }

    fn chat_scope_info(principal: &RpcAuthPrincipal) -> ChatScopeInfo {
        ChatScopeInfo {
            username: principal.username.clone(),
            owner_did: principal.owner_did.clone(),
            access_mode: match principal.user_type {
                UserType::Root | UserType::Admin => "full_access",
                UserType::User | UserType::Limited | UserType::Guest => "read_only",
            },
        }
    }

    async fn get_msg_center_client(&self) -> Result<MsgCenterClient, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        runtime.get_msg_center_client().await.map_err(|error| {
            RPCErrors::ReasonError(format!("get msg-center client failed: {}", error))
        })
    }

    fn get_chat_kevent_client() -> KEventClient {
        static CHAT_KEVENT_CLIENT: OnceLock<KEventClient> = OnceLock::new();
        CHAT_KEVENT_CLIENT
            .get_or_init(|| KEventClient::new_full(CONTROL_PANEL_SERVICE_NAME, None))
            .clone()
    }

    fn normalize_chat_stream_keepalive_ms(keepalive_ms: Option<u64>) -> u64 {
        keepalive_ms
            .unwrap_or(DEFAULT_CHAT_STREAM_KEEPALIVE_MS)
            .clamp(MIN_CHAT_STREAM_KEEPALIVE_MS, MAX_CHAT_STREAM_KEEPALIVE_MS)
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

    fn build_chat_stream_response(
        receiver: mpsc::Receiver<std::result::Result<Bytes, ServerError>>,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let stream = stream::unfold(receiver, |mut receiver| async move {
            receiver.recv().await.map(|item| (item, receiver))
        });
        let body = StreamBody::new(stream.map_ok(Frame::data));

        http::Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "application/x-ndjson")
            .header(CACHE_CONTROL, "no-store")
            .header("X-Accel-Buffering", "no")
            .body(BodyExt::map_err(body, |error| error).boxed())
            .map_err(|error| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "Failed to build chat stream response: {}",
                    error
                )
            })
    }

    async fn send_chat_stream_json<T: Serialize>(
        sender: &mpsc::Sender<std::result::Result<Bytes, ServerError>>,
        payload: &T,
    ) -> bool {
        let mut body = match serde_json::to_vec(payload) {
            Ok(body) => body,
            Err(error) => {
                let _ = sender
                    .send(Err(server_err!(
                        ServerErrorCode::EncodeError,
                        "Failed to serialize chat stream payload: {}",
                        error
                    )))
                    .await;
                return false;
            }
        };
        body.push(b'\n');
        sender.send(Ok(Bytes::from(body))).await.is_ok()
    }

    async fn send_chat_stream_error(
        sender: &mpsc::Sender<std::result::Result<Bytes, ServerError>>,
        message: String,
    ) -> bool {
        Self::send_chat_stream_json(
            sender,
            &json!({
                "type": "error",
                "message": message,
                "at_ms": Self::current_time_ms(),
            }),
        )
        .await
    }

    fn chat_access_level_label(level: &AccessGroupLevel) -> &'static str {
        match level {
            AccessGroupLevel::Block => "block",
            AccessGroupLevel::Stranger => "stranger",
            AccessGroupLevel::Temporary => "temporary",
            AccessGroupLevel::Friend => "friend",
        }
    }

    fn chat_msg_state_label(state: &MsgState) -> &'static str {
        match state {
            MsgState::Unread => "unread",
            MsgState::Reading => "reading",
            MsgState::Readed => "readed",
            MsgState::Wait => "wait",
            MsgState::Sending => "sending",
            MsgState::Sent => "sent",
            MsgState::Failed => "failed",
            MsgState::Dead => "dead",
            MsgState::Deleted => "deleted",
            MsgState::Archived => "archived",
        }
    }

    fn normalize_chat_contact_limit(limit: Option<usize>) -> usize {
        limit
            .unwrap_or(DEFAULT_CHAT_CONTACT_LIMIT)
            .clamp(1, DEFAULT_CHAT_CONTACT_LIMIT)
    }

    fn normalize_chat_message_limit(limit: Option<usize>) -> usize {
        limit
            .unwrap_or(DEFAULT_CHAT_MESSAGE_LIMIT)
            .clamp(1, MAX_CHAT_MESSAGE_LIMIT)
    }

    fn chat_scan_limit(message_limit: usize) -> usize {
        message_limit
            .saturating_mul(4)
            .clamp(40, MAX_CHAT_SCAN_LIMIT)
    }

    fn current_time_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    fn map_chat_contact(contact: Contact) -> ChatContactView {
        ChatContactView {
            did: contact.did.to_string(),
            name: contact.name,
            avatar: contact.avatar,
            note: contact.note,
            access_level: Self::chat_access_level_label(&contact.access_level),
            is_verified: contact.is_verified,
            groups: contact.groups,
            tags: contact.tags,
            created_at: contact.created_at,
            updated_at: contact.updated_at,
            bindings: contact
                .bindings
                .into_iter()
                .map(|binding| ChatBindingView {
                    platform: binding.platform,
                    account_id: binding.account_id,
                    display_id: binding.display_id,
                    tunnel_id: binding.tunnel_id,
                    last_active_at: binding.last_active_at,
                    meta: binding.meta,
                })
                .collect(),
        }
    }

    fn chat_message_thread_id(record: &MsgRecordWithObject) -> Option<String> {
        record
            .record
            .ui_session_id
            .clone()
            .or_else(|| record.msg.as_ref().and_then(|msg| msg.thread.topic.clone()))
            .or_else(|| {
                record
                    .msg
                    .as_ref()
                    .and_then(|msg| msg.thread.correlation_id.clone())
            })
            .or_else(|| {
                record.msg.as_ref().and_then(|msg| {
                    msg.meta
                        .get("session_id")
                        .and_then(|value| value.as_str())
                        .map(|value| value.trim().to_string())
                        .filter(|value| !value.is_empty())
                })
            })
            .or_else(|| {
                record.msg.as_ref().and_then(|msg| {
                    msg.meta
                        .get("owner_session_id")
                        .and_then(|value| value.as_str())
                        .map(|value| value.trim().to_string())
                        .filter(|value| !value.is_empty())
                })
            })
    }

    fn chat_record_matches_peer(
        record: &MsgRecordWithObject,
        owner_did: &DID,
        peer_did: &DID,
    ) -> bool {
        if record.record.msg_kind != MsgObjKind::Chat {
            return false;
        }

        if record.record.from == *owner_did {
            record.record.to == *peer_did
        } else {
            record.record.from == *peer_did
        }
    }

    fn chat_record_matches_stream(
        record: &MsgRecordWithObject,
        owner_did: &DID,
        peer_did: &DID,
        thread_id: Option<&str>,
    ) -> bool {
        if !Self::chat_record_matches_peer(record, owner_did, peer_did) {
            return false;
        }

        match thread_id {
            Some(thread_id) => Self::chat_message_thread_id(record).as_deref() == Some(thread_id),
            None => true,
        }
    }

    fn chat_record_id_from_event(event: &Event) -> Option<String> {
        serde_json::from_value::<MsgCenterBoxChangedEvent>(event.data.clone())
            .ok()
            .and_then(|payload| payload.record_id)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    fn chat_event_operation(event: &Event) -> String {
        serde_json::from_value::<MsgCenterBoxChangedEvent>(event.data.clone())
            .ok()
            .and_then(|payload| payload.operation)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "changed".to_string())
    }

    fn map_chat_message_record(
        record: &MsgRecordWithObject,
        owner_did: &DID,
        peer_name: Option<String>,
    ) -> ChatMessageView {
        let direction = if record.record.from == *owner_did {
            "outbound"
        } else {
            "inbound"
        };
        let peer_name = peer_name.or_else(|| {
            if direction == "inbound" {
                record.record.from_name.clone()
            } else {
                None
            }
        });
        let peer_did = if direction == "outbound" {
            record.record.to.to_string()
        } else {
            record.record.from.to_string()
        };
        let (content, content_format) = match record.msg.as_ref() {
            Some(msg) => (
                msg.content.content.clone(),
                msg.content
                    .format
                    .as_ref()
                    .map(|format| format!("{:?}", format)),
            ),
            None => (String::new(), None),
        };

        ChatMessageView {
            record_id: record.record.record_id.clone(),
            msg_id: record.record.msg_id.to_string(),
            direction,
            peer_did,
            peer_name,
            state: Self::chat_msg_state_label(&record.record.state),
            created_at_ms: record.record.created_at_ms,
            updated_at_ms: record.record.updated_at_ms,
            sort_key: record.record.sort_key,
            thread_id: Self::chat_message_thread_id(record),
            content,
            content_format,
        }
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
        external_command("rg")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    fn rg_search_lines(path: &Path, keyword: &str) -> Result<Vec<(u64, String)>, RPCErrors> {
        let output = external_command("rg")
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
        let snapshot = { self.metrics_snapshot.read().await.clone() };

        let total_memory_bytes = snapshot.memory_total_bytes;
        let used_memory_bytes = snapshot.memory_used_bytes;
        let memory_percent = if total_memory_bytes > 0 {
            ((used_memory_bytes as f64 / total_memory_bytes as f64) * 100.0).round()
        } else {
            0.0
        };
        let total_swap_bytes = snapshot.swap_total_bytes;
        let used_swap_bytes = snapshot.swap_used_bytes;
        let swap_percent = if total_swap_bytes > 0 {
            ((used_swap_bytes as f64 / total_swap_bytes as f64) * 100.0).round()
        } else {
            0.0
        };
        let process_count = snapshot.process_count;
        let uptime_seconds = snapshot.uptime_seconds;

        let lite = req
            .params
            .get("lite")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let disks_detail = if lite {
            Vec::new()
        } else {
            snapshot.disks_detail.clone()
        };

        let storage_capacity_bytes = snapshot.storage_capacity_bytes;
        let storage_used_bytes = snapshot.storage_used_bytes;
        let disk_usage_percent = if storage_capacity_bytes > 0 {
            ((storage_used_bytes as f64 / storage_capacity_bytes as f64) * 100.0).round()
        } else {
            0.0
        };

        let resource_timeline: Vec<Value> = snapshot
            .timeline
            .iter()
            .map(|point| {
                json!({
                    "time": point.time,
                    "cpu": point.cpu,
                    "memory": point.memory,
                })
            })
            .collect();
        let network_timeline: Vec<Value> = snapshot
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
        let network_per_interface: Vec<Value> = snapshot
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

        let metrics = json!({
            "cpu": {
                "usagePercent": snapshot.cpu_usage_percent,
                "model": snapshot.cpu_brand,
                "cores": snapshot.cpu_cores,
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
                "rxBytes": snapshot.network.rx_bytes,
                "txBytes": snapshot.network.tx_bytes,
                "rxPerSec": snapshot.network.rx_per_sec,
                "txPerSec": snapshot.network.tx_per_sec,
                "rxErrors": snapshot.network.rx_errors,
                "txErrors": snapshot.network.tx_errors,
                "rxDrops": snapshot.network.rx_drops,
                "txDrops": snapshot.network.tx_drops,
                "interfaceCount": snapshot.network.interface_count,
                "perInterface": network_per_interface,
            },
            "swap": {
                "totalGb": bytes_to_gb(total_swap_bytes),
                "usedGb": bytes_to_gb(used_swap_bytes),
                "usagePercent": swap_percent,
            },
            "loadAverage": {
                "one": snapshot.load_one,
                "five": snapshot.load_five,
                "fifteen": snapshot.load_fifteen,
            },
            "processCount": process_count,
            "uptimeSeconds": uptime_seconds,
            "resourceTimeline": resource_timeline,
            "networkTimeline": network_timeline,
        });

        Ok(RPCResponse::new(RPCResult::Success(metrics), req.seq))
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
        let depth = depth.min(SYS_CONFIG_TREE_MAX_DEPTH);
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

    fn default_ai_policies_value() -> Value {
        json!({
            "items": [
                {
                    "id": "message_hub.reply",
                    "label": "Message Hub Reply",
                    "primaryModel": "gpt-fast",
                    "fallbackModels": ["gemini-ops"],
                    "objective": "Fast reply drafting with safe structured output when needed.",
                    "status": "active"
                },
                {
                    "id": "message_hub.summary",
                    "label": "Message Hub Summary",
                    "primaryModel": "gpt-fast",
                    "fallbackModels": ["gpt-plan"],
                    "objective": "Summarize cross-thread context into compact inbox cards and digest blocks.",
                    "status": "active"
                },
                {
                    "id": "message_hub.task_extract",
                    "label": "Task Extraction",
                    "primaryModel": "gemini-ops",
                    "fallbackModels": ["minimax-api", "gpt-fast"],
                    "objective": "Convert commitments and deadlines into follow-up objects connected to the source thread.",
                    "status": "review"
                },
                {
                    "id": "agent.plan",
                    "label": "Agent Plan",
                    "primaryModel": "minimax-code-plan",
                    "fallbackModels": ["gpt-plan"],
                    "objective": "Use MiniMax code-planning mode for task decomposition, implementation planning, and multi-step agent execution guidance.",
                    "status": "review"
                },
                {
                    "id": "agent.raw_explain",
                    "label": "Agent RAW Explain",
                    "primaryModel": "minimax-api",
                    "fallbackModels": ["gpt-plan", "gpt-fast"],
                    "objective": "Use MiniMax API mode for structured explanation of agent-to-agent raw records.",
                    "status": "planned"
                }
            ]
        })
    }

    fn default_ai_provider_overrides_value() -> Value {
        json!({
            "items": [
                {
                    "id": "openai-compatible",
                    "displayName": "OpenAI-Compatible Gateway",
                    "providerType": "Compatible",
                    "status": "needs_setup",
                    "endpoint": "http://127.0.0.1:11434/v1",
                    "authMode": "Optional token",
                    "capabilities": ["Local LLM", "Low-cost fallback"],
                    "defaultModel": "Not assigned",
                    "note": "Reserved for local or self-hosted models once the backend management flow is connected."
                },
                {
                    "id": "claude-planned",
                    "displayName": "Claude",
                    "providerType": "Anthropic",
                    "status": "planned",
                    "endpoint": "https://api.anthropic.com",
                    "authMode": "API key",
                    "capabilities": ["Long-form reasoning", "Tool calling"],
                    "defaultModel": "Planned",
                    "note": "Documented as a future provider family; not wired in this control-panel phase."
                }
            ]
        })
    }

    fn default_ai_model_catalog_value() -> Value {
        json!({
            "items": [
                {
                    "alias": "minimax-code-plan",
                    "providerId": "minimax-main",
                    "providerModel": "MiniMax-M2.5",
                    "capabilities": ["llm_router"],
                    "features": ["plan", "tool_calling", "code"],
                    "useCases": ["agent.plan", "message_hub.reply"]
                },
                {
                    "alias": "minimax-api",
                    "providerId": "minimax-main",
                    "providerModel": "MiniMax-M2.1-highspeed",
                    "capabilities": ["llm_router"],
                    "features": ["json_output", "tool_calling", "api"],
                    "useCases": ["message_hub.task_extract", "agent.raw_explain"]
                },
                {
                    "alias": "gpt-fast",
                    "providerId": "openai-main",
                    "providerModel": "gpt-4.1-mini",
                    "capabilities": ["llm_router"],
                    "features": ["json_output", "tool_calling"],
                    "useCases": ["message_hub.reply", "message_hub.summary"]
                },
                {
                    "alias": "gpt-plan",
                    "providerId": "openai-main",
                    "providerModel": "gpt-4.1",
                    "capabilities": ["llm_router"],
                    "features": ["plan", "tool_calling", "json_output"],
                    "useCases": ["agent.plan", "agent.raw_explain"]
                },
                {
                    "alias": "gemini-ops",
                    "providerId": "google-main",
                    "providerModel": "gemini-2.5-flash",
                    "capabilities": ["llm_router"],
                    "features": ["json_output", "vision"],
                    "useCases": ["message_hub.task_extract", "message_hub.priority_rank"]
                }
            ]
        })
    }

    fn default_ai_provider_secrets_value() -> Value {
        json!({ "items": [] })
    }

    async fn load_json_config_or_default(
        client: &SystemConfigClient,
        key: &str,
        default: Value,
    ) -> Value {
        match client.get(key).await {
            Ok(value) => serde_json::from_str::<Value>(&value.value).unwrap_or(default),
            Err(_) => default,
        }
    }

    async fn save_json_config(
        client: &SystemConfigClient,
        key: &str,
        value: &Value,
    ) -> Result<(), RPCErrors> {
        let serialized = serde_json::to_string_pretty(value)
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        client
            .set(key, &serialized)
            .await
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        Ok(())
    }

    fn upsert_item_by_id(items: &mut Vec<Value>, id: &str, next: Value) {
        if let Some(index) = items
            .iter()
            .position(|item| item.get("id").and_then(|value| value.as_str()) == Some(id))
        {
            items[index] = next;
        } else {
            items.push(next);
        }
    }

    fn upsert_item_by_alias(items: &mut Vec<Value>, alias: &str, next: Value) {
        if let Some(index) = items
            .iter()
            .position(|item| item.get("alias").and_then(|value| value.as_str()) == Some(alias))
        {
            items[index] = next;
        } else {
            items.push(next);
        }
    }

    fn merge_provider_overrides(base_items: Vec<Value>, overrides: &[Value]) -> Vec<Value> {
        let mut merged = base_items;
        for override_item in overrides.iter() {
            if let Some(id) = override_item.get("id").and_then(|value| value.as_str()) {
                if ["openai-main", "google-main", "minimax-main"].contains(&id) {
                    continue;
                }
                Self::upsert_item_by_id(&mut merged, id, override_item.clone());
            }
        }
        merged
    }

    fn provider_secret_configured(provider_id: &str, secret_doc: &Value) -> bool {
        secret_doc
            .get("items")
            .and_then(|value| value.as_array())
            .and_then(|items| {
                items.iter().find(|item| {
                    item.get("id").and_then(|value| value.as_str()) == Some(provider_id)
                })
            })
            .and_then(|item| item.get("apiKey").and_then(|value| value.as_str()))
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
    }

    fn mask_secret(value: &str) -> Option<String> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return None;
        }

        let chars = trimmed.chars().collect::<Vec<_>>();
        let len = chars.len();
        let prefix_len = len.min(4);
        let suffix_len = len.saturating_sub(prefix_len).min(4);
        let prefix = chars.iter().take(prefix_len).collect::<String>();
        let suffix = if suffix_len == 0 {
            String::new()
        } else {
            chars.iter().skip(len - suffix_len).collect::<String>()
        };

        Some(format!("{}***{}", prefix, suffix))
    }

    fn provider_masked_secret(provider_id: &str, secret_doc: &Value) -> Option<String> {
        secret_doc
            .get("items")
            .and_then(|value| value.as_array())
            .and_then(|items| {
                items.iter().find(|item| {
                    item.get("id").and_then(|value| value.as_str()) == Some(provider_id)
                })
            })
            .and_then(|item| item.get("apiKey").and_then(|value| value.as_str()))
            .and_then(Self::mask_secret)
    }

    fn ai_openai_provider_card(settings: &Value) -> Value {
        let openai = settings.get("openai").cloned().unwrap_or_else(|| json!({}));
        let enabled = openai
            .get("enabled")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let api_token = openai
            .get("api_token")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let instances = openai
            .get("instances")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        let first = instances.first().cloned().unwrap_or_else(|| json!({}));
        let endpoint = first
            .get("base_url")
            .and_then(|value| value.as_str())
            .unwrap_or("https://api.openai.com/v1");
        let default_model = first
            .get("default_model")
            .and_then(|value| value.as_str())
            .or_else(|| {
                first
                    .get("models")
                    .and_then(|value| value.as_array())
                    .and_then(|items| items.first())
                    .and_then(|value| value.as_str())
            })
            .unwrap_or("gpt-fast");
        let status = if enabled && !api_token.trim().is_empty() {
            "healthy"
        } else if enabled {
            "degraded"
        } else {
            "needs_setup"
        };

        json!({
            "id": "openai-main",
            "displayName": "OpenAI Main",
            "providerType": "OpenAI",
            "status": status,
            "endpoint": endpoint,
            "authMode": "Bearer token",
            "credentialConfigured": !api_token.trim().is_empty(),
            "maskedApiKey": Self::mask_secret(api_token),
            "capabilities": ["Reply", "Summary", "Tool calling"],
            "defaultModel": default_model,
            "note": "Primary cloud provider for Message Hub reply and summary flows."
        })
    }

    fn ai_google_provider_card(settings: &Value) -> Value {
        let google = settings
            .get("google")
            .or_else(|| settings.get("gimini"))
            .or_else(|| settings.get("gemini"))
            .cloned()
            .unwrap_or_else(|| json!({}));
        let enabled = google
            .get("enabled")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let api_token = google
            .get("api_token")
            .or_else(|| google.get("api_key"))
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let instances = google
            .get("instances")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        let first = instances.first().cloned().unwrap_or_else(|| json!({}));
        let endpoint = first
            .get("base_url")
            .and_then(|value| value.as_str())
            .unwrap_or("https://generativelanguage.googleapis.com/v1beta");
        let default_model = first
            .get("default_model")
            .and_then(|value| value.as_str())
            .or_else(|| {
                first
                    .get("models")
                    .and_then(|value| value.as_array())
                    .and_then(|items| items.first())
                    .and_then(|value| value.as_str())
            })
            .unwrap_or("gemini-ops");
        let status = if enabled && !api_token.trim().is_empty() {
            "healthy"
        } else if enabled {
            "degraded"
        } else {
            "needs_setup"
        };

        json!({
            "id": "google-main",
            "displayName": "Google Gemini",
            "providerType": "Google",
            "status": status,
            "endpoint": endpoint,
            "authMode": "API key",
            "credentialConfigured": !api_token.trim().is_empty(),
            "maskedApiKey": Self::mask_secret(api_token),
            "capabilities": ["Task extract", "Multimodal", "JSON output"],
            "defaultModel": default_model,
            "note": "Secondary provider used for extraction-heavy workflows and fallback coverage."
        })
    }

    fn ai_minimax_provider_card(settings: &Value, secret_doc: &Value) -> Value {
        let minimax = settings
            .get("minimax")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let enabled = minimax
            .get("enabled")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let api_token = minimax
            .get("api_token")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let instances = minimax
            .get("instances")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        let first = instances.first().cloned().unwrap_or_else(|| json!({}));
        let endpoint = first
            .get("base_url")
            .and_then(|value| value.as_str())
            .unwrap_or("https://api.minimaxi.com/anthropic/v1");
        let default_model = first
            .get("default_model")
            .and_then(|value| value.as_str())
            .or_else(|| {
                first
                    .get("models")
                    .and_then(|value| value.as_array())
                    .and_then(|items| items.first())
                    .and_then(|value| value.as_str())
            })
            .unwrap_or("MiniMax-M2.5");
        let credential_configured = !api_token.trim().is_empty()
            || Self::provider_secret_configured("minimax-main", secret_doc);
        let masked_api_key = Self::mask_secret(api_token)
            .or_else(|| Self::provider_masked_secret("minimax-main", secret_doc));
        let status = if enabled && credential_configured {
            "healthy"
        } else if enabled {
            "degraded"
        } else {
            "needs_setup"
        };

        json!({
            "id": "minimax-main",
            "displayName": "MiniMax",
            "providerType": "MiniMax",
            "status": status,
            "endpoint": endpoint,
            "authMode": "X-API-Key",
            "credentialConfigured": credential_configured,
            "maskedApiKey": masked_api_key,
            "availableModels": [
                "MiniMax-M2.5",
                "MiniMax-M2.5-highspeed",
                "MiniMax-M2.1",
                "MiniMax-M2.1-highspeed",
                "MiniMax-M2"
            ],
            "capabilities": ["Code plan", "API mode"],
            "defaultModel": default_model,
            "note": "Anthropic-compatible MiniMax runtime for code planning and API-oriented workflows."
        })
    }

    fn ai_provider_cards(settings: &Value, overrides: &[Value], secret_doc: &Value) -> Vec<Value> {
        let base_items = vec![
            Self::ai_openai_provider_card(settings),
            Self::ai_google_provider_card(settings),
            Self::ai_minimax_provider_card(settings, secret_doc),
        ];

        let mut merged = Self::merge_provider_overrides(base_items, overrides);
        for item in merged.iter_mut() {
            let provider_id = item
                .get("id")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string();
            if item.get("credentialConfigured").is_none() {
                item["credentialConfigured"] = Value::Bool(Self::provider_secret_configured(
                    provider_id.as_str(),
                    secret_doc,
                ));
            }
            if item.get("maskedApiKey").is_none() {
                if let Some(masked) = Self::provider_masked_secret(provider_id.as_str(), secret_doc)
                {
                    item["maskedApiKey"] = Value::String(masked);
                }
            }
        }
        merged
    }

    fn ai_model_catalog(settings: &Value, overrides: &[Value]) -> Vec<Value> {
        let openai = settings.get("openai").cloned().unwrap_or_else(|| json!({}));
        let google = settings
            .get("google")
            .or_else(|| settings.get("gimini"))
            .or_else(|| settings.get("gemini"))
            .cloned()
            .unwrap_or_else(|| json!({}));
        let openai_instance = openai
            .get("instances")
            .and_then(|value| value.as_array())
            .and_then(|items| items.first())
            .cloned()
            .unwrap_or_else(|| json!({}));
        let google_instance = google
            .get("instances")
            .and_then(|value| value.as_array())
            .and_then(|items| items.first())
            .cloned()
            .unwrap_or_else(|| json!({}));

        let openai_default = openai_instance
            .get("default_model")
            .and_then(|value| value.as_str())
            .or_else(|| {
                openai_instance
                    .get("models")
                    .and_then(|value| value.as_array())
                    .and_then(|items| items.first())
                    .and_then(|value| value.as_str())
            })
            .unwrap_or("gpt-4.1-mini");
        let google_default = google_instance
            .get("default_model")
            .and_then(|value| value.as_str())
            .or_else(|| {
                google_instance
                    .get("models")
                    .and_then(|value| value.as_array())
                    .and_then(|items| items.first())
                    .and_then(|value| value.as_str())
            })
            .unwrap_or("gemini-2.5-flash");

        let mut items = vec![
            json!({
                "alias": "gpt-fast",
                "providerId": "openai-main",
                "providerModel": openai_default,
                "capabilities": ["llm_router"],
                "features": ["json_output", "tool_calling"],
                "useCases": ["message_hub.reply", "message_hub.summary"]
            }),
            json!({
                "alias": "gpt-plan",
                "providerId": "openai-main",
                "providerModel": openai_default,
                "capabilities": ["llm_router"],
                "features": ["plan", "tool_calling", "json_output"],
                "useCases": ["agent.plan", "agent.raw_explain"]
            }),
            json!({
                "alias": "gemini-ops",
                "providerId": "google-main",
                "providerModel": google_default,
                "capabilities": ["llm_router"],
                "features": ["json_output", "vision"],
                "useCases": ["message_hub.task_extract", "message_hub.priority_rank"]
            }),
        ];

        for override_item in overrides.iter() {
            if let Some(alias) = override_item.get("alias").and_then(|value| value.as_str()) {
                Self::upsert_item_by_alias(&mut items, alias, override_item.clone());
            }
        }

        items
    }

    fn ai_overview(providers: &[Value], policies: &[Value]) -> Value {
        let providers_online = providers
            .iter()
            .filter(|provider| {
                provider.get("status").and_then(|value| value.as_str()) == Some("healthy")
            })
            .count();

        let primary_model = |id: &str, fallback: &str| -> String {
            policies
                .iter()
                .find(|policy| policy.get("id").and_then(|value| value.as_str()) == Some(id))
                .and_then(|policy| policy.get("primaryModel").and_then(|value| value.as_str()))
                .unwrap_or(fallback)
                .to_string()
        };

        json!({
            "providersOnline": providers_online,
            "providersTotal": providers.len(),
            "defaultReplyModel": primary_model("message_hub.reply", "gpt-fast"),
            "defaultSummaryModel": primary_model("message_hub.summary", "gpt-fast"),
            "defaultTaskExtractModel": primary_model("message_hub.task_extract", "gemini-ops"),
            "defaultAgentModel": primary_model("agent.plan", "minimax-code-plan"),
            "avgLatencyMs": 840,
            "estimatedDailyCostUsd": 2.37,
            "lastDiagnosticsAt": format!("Today {}", chrono::Local::now().format("%H:%M")),
        })
    }

    fn ai_policy_primary_model(policies: &[Value], policy_id: &str, fallback: &str) -> String {
        policies
            .iter()
            .find(|policy| policy.get("id").and_then(|value| value.as_str()) == Some(policy_id))
            .and_then(|policy| policy.get("primaryModel").and_then(|value| value.as_str()))
            .unwrap_or(fallback)
            .to_string()
    }

    fn build_message_hub_summary_prompt(
        peer_name: Option<&str>,
        peer_did: &str,
        messages: &[ChatMessageView],
    ) -> String {
        let mut transcript = String::new();
        for message in messages.iter().rev().take(20).rev() {
            let speaker = if message.direction == "outbound" {
                "Me"
            } else {
                peer_name.unwrap_or(peer_did)
            };
            let line = format!(
                "[{}] {}: {}\n",
                chrono::DateTime::<chrono::Utc>::from_timestamp_millis(
                    message.created_at_ms as i64
                )
                .map(|ts| ts.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| message.created_at_ms.to_string()),
                speaker,
                message.content.replace('\n', " ")
            );
            transcript.push_str(&line);
        }

        format!(
            "You summarize a direct communication thread for Message Hub. Return plain text with three short sections titled Summary, Decisions, and Follow-ups. Keep it concise and action-oriented.\n\nPeer: {}\nPeer DID: {}\n\nTranscript:\n{}",
            peer_name.unwrap_or(peer_did),
            peer_did,
            transcript
        )
    }

    async fn handle_ai_overview(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        let settings =
            Self::load_json_config_or_default(&client, AICC_SETTINGS_KEY, json!({})).await;
        let secret_doc = Self::load_json_config_or_default(
            &client,
            AI_MODELS_PROVIDER_SECRETS_KEY,
            Self::default_ai_provider_secrets_value(),
        )
        .await;
        let provider_overrides = Self::load_json_config_or_default(
            &client,
            AI_MODELS_PROVIDER_OVERRIDES_KEY,
            Self::default_ai_provider_overrides_value(),
        )
        .await;
        let policy_doc = Self::load_json_config_or_default(
            &client,
            AI_MODELS_POLICIES_KEY,
            Self::default_ai_policies_value(),
        )
        .await;
        let providers = Self::ai_provider_cards(
            &settings,
            provider_overrides
                .get("items")
                .and_then(|value| value.as_array())
                .map(|items| items.as_slice())
                .unwrap_or(&[]),
            &secret_doc,
        );
        let policies = policy_doc
            .get("items")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();

        Ok(RPCResponse::new(
            RPCResult::Success(Self::ai_overview(&providers, &policies)),
            req.seq,
        ))
    }

    async fn handle_ai_provider_list(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        let settings =
            Self::load_json_config_or_default(&client, AICC_SETTINGS_KEY, json!({})).await;
        let secret_doc = Self::load_json_config_or_default(
            &client,
            AI_MODELS_PROVIDER_SECRETS_KEY,
            Self::default_ai_provider_secrets_value(),
        )
        .await;
        let provider_overrides = Self::load_json_config_or_default(
            &client,
            AI_MODELS_PROVIDER_OVERRIDES_KEY,
            Self::default_ai_provider_overrides_value(),
        )
        .await;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "items": Self::ai_provider_cards(
                    &settings,
                    provider_overrides
                        .get("items")
                        .and_then(|value| value.as_array())
                        .map(|items| items.as_slice())
                        .unwrap_or(&[]),
                    &secret_doc,
                )
            })),
            req.seq,
        ))
    }

    async fn handle_ai_model_list(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        let settings =
            Self::load_json_config_or_default(&client, AICC_SETTINGS_KEY, json!({})).await;
        let model_catalog = Self::load_json_config_or_default(
            &client,
            AI_MODELS_MODEL_CATALOG_KEY,
            Self::default_ai_model_catalog_value(),
        )
        .await;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "items": Self::ai_model_catalog(
                    &settings,
                    model_catalog
                        .get("items")
                        .and_then(|value| value.as_array())
                        .map(|items| items.as_slice())
                        .unwrap_or(&[]),
                )
            })),
            req.seq,
        ))
    }

    async fn handle_ai_policy_list(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        let policy_doc = Self::load_json_config_or_default(
            &client,
            AI_MODELS_POLICIES_KEY,
            Self::default_ai_policies_value(),
        )
        .await;
        let items = policy_doc
            .get("items")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();

        Ok(RPCResponse::new(
            RPCResult::Success(json!({ "items": items })),
            req.seq,
        ))
    }

    async fn handle_ai_diagnostics_list(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "items": [
                    {
                        "id": "diag-openai",
                        "title": "OpenAI round-trip test",
                        "status": "pass",
                        "detail": "Use control_panel to trigger a light /kapi/aicc completion through the AI Models module.",
                        "actionLabel": "Run again"
                    },
                    {
                        "id": "diag-google",
                        "title": "Gemini extraction profile",
                        "status": "pass",
                        "detail": "Current policy defaults reserve Gemini for extraction-heavy Message Hub workflows.",
                        "actionLabel": "Run again"
                    },
                    {
                        "id": "diag-local",
                        "title": "Local LLM gateway",
                        "status": "pending",
                        "detail": "Reserved for a future OpenAI-compatible or local endpoint once provider configuration broadens.",
                        "actionLabel": "Review checklist"
                    }
                ]
            })),
            req.seq,
        ))
    }

    async fn handle_ai_provider_test(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let provider_id = Self::require_param_str(&req, "provider_id")?;
        let alias = match provider_id.as_str() {
            "openai-main" => "gpt-fast",
            "google-main" => "gemini-ops",
            "minimax-main" => "minimax-code-plan",
            _ => {
                return Ok(RPCResponse::new(
                    RPCResult::Success(json!({
                        "providerId": provider_id,
                        "ok": false,
                        "status": "pending",
                        "detail": "This provider family is not wired to /kapi/aicc in the current control_panel phase."
                    })),
                    req.seq,
                ))
            }
        };

        let runtime = get_buckyos_api_runtime()?;
        let aicc = runtime.get_aicc_client().await.map_err(|error| {
            RPCErrors::ReasonError(format!("init aicc client failed: {}", error))
        })?;

        let request = CompleteRequest::new(
            Capability::LlmRouter,
            ModelSpec::new(alias.to_string(), None),
            Requirements::default(),
            AiPayload::new(
                None,
                vec![AiMessage::new(
                    "user".to_string(),
                    "Return a compact JSON object that confirms provider connectivity.".to_string(),
                )],
                vec![],
                vec![],
                None,
                Some(json!({
                    "max_tokens": 64,
                    "temperature": 0.1,
                    "response_format": { "type": "json_object" }
                })),
            ),
            None,
        );

        match aicc.complete(request).await {
            Ok(result) => Ok(RPCResponse::new(
                RPCResult::Success(json!({
                    "providerId": provider_id,
                    "ok": true,
                    "status": "pass",
                    "taskId": result.task_id,
                    "detail": result
                        .result
                        .and_then(|summary| summary.text)
                        .unwrap_or_else(|| "Provider test completed successfully.".to_string())
                })),
                req.seq,
            )),
            Err(error) => Ok(RPCResponse::new(
                RPCResult::Success(json!({
                    "providerId": provider_id,
                    "ok": false,
                    "status": "warn",
                    "detail": error.to_string()
                })),
                req.seq,
            )),
        }
    }

    async fn handle_ai_message_hub_thread_summary(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_chat_principal(principal)?;
        let owner_did = Self::parse_chat_owner_did(principal)?;
        let peer_did_raw = Self::require_param_str(&req, "peer_did")?;
        let peer_did = DID::from_str(peer_did_raw.trim()).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Invalid peer_did `{}`: {}", peer_did_raw, error))
        })?;

        let msg_center = self.get_msg_center_client().await?;
        let peer_name = match msg_center
            .get_contact(peer_did.clone(), Some(owner_did.clone()))
            .await
        {
            Ok(Some(contact)) => Some(contact.name),
            _ => None,
        };

        let inbox = msg_center
            .list_box_by_time(
                owner_did.clone(),
                BoxKind::Inbox,
                None,
                Some(60),
                None,
                None,
                Some(true),
                Some(true),
            )
            .await?;
        let outbox = msg_center
            .list_box_by_time(
                owner_did.clone(),
                BoxKind::Outbox,
                None,
                Some(60),
                None,
                None,
                Some(true),
                Some(true),
            )
            .await?;

        let mut records = inbox
            .items
            .into_iter()
            .chain(outbox.items.into_iter())
            .filter(|record| Self::chat_record_matches_peer(record, &owner_did, &peer_did))
            .collect::<Vec<_>>();
        records.sort_by(|left, right| {
            left.record
                .sort_key
                .cmp(&right.record.sort_key)
                .then_with(|| left.record.updated_at_ms.cmp(&right.record.updated_at_ms))
        });

        let items = records
            .iter()
            .map(|record| Self::map_chat_message_record(record, &owner_did, peer_name.clone()))
            .collect::<Vec<_>>();
        if items.is_empty() {
            return Err(RPCErrors::ReasonError(
                "No thread messages available to summarize yet.".to_string(),
            ));
        }

        let runtime = get_buckyos_api_runtime()?;
        let config_client = runtime.get_system_config_client().await?;
        let policy_doc = Self::load_json_config_or_default(
            &config_client,
            AI_MODELS_POLICIES_KEY,
            Self::default_ai_policies_value(),
        )
        .await;
        let policies = policy_doc
            .get("items")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        let model_alias =
            Self::ai_policy_primary_model(&policies, "message_hub.summary", "gpt-fast");

        let aicc = runtime.get_aicc_client().await.map_err(|error| {
            RPCErrors::ReasonError(format!("init aicc client failed: {}", error))
        })?;
        let peer_did_string = peer_did.to_string();
        let request = CompleteRequest::new(
            Capability::LlmRouter,
            ModelSpec::new(model_alias.clone(), None),
            Requirements::default(),
            AiPayload::new(
                Some(Self::build_message_hub_summary_prompt(
                    peer_name.as_deref(),
                    peer_did_string.as_str(),
                    &items,
                )),
                vec![],
                vec![],
                vec![],
                None,
                Some(json!({
                    "max_tokens": 240,
                    "temperature": 0.2
                })),
            ),
            None,
        );

        let result = aicc
            .complete(request)
            .await
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        let summary = result
            .result
            .and_then(|summary| summary.text)
            .filter(|text| !text.trim().is_empty())
            .unwrap_or_else(|| "No summary text returned by the model.".to_string());

        Ok(RPCResponse::new(
            RPCResult::Success(json!(MessageHubThreadSummaryResponse {
                peer_did: peer_did_string,
                peer_name,
                model_alias,
                summary,
                source_message_count: items.len(),
            })),
            req.seq,
        ))
    }

    async fn handle_ai_reload(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        let krpc_client = runtime
            .get_zone_service_krpc_client("aicc")
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!("init aicc rpc client failed: {}", error))
            })?;

        let result = krpc_client
            .call("service.reload_settings", json!({}))
            .await
            .map_err(|error| {
                RPCErrors::ReasonError(format!("reload aicc settings failed: {}", error))
            })?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "ok": true,
                "result": result,
            })),
            req.seq,
        ))
    }

    async fn handle_ai_provider_set(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let provider = req
            .params
            .get("provider")
            .cloned()
            .ok_or_else(|| RPCErrors::ReasonError("missing provider payload".to_string()))?;
        let api_key = Self::param_str(&req, "api_key");
        let has_new_api_key = api_key
            .as_ref()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
        let provider_id = provider
            .get("id")
            .and_then(|value| value.as_str())
            .ok_or_else(|| RPCErrors::ReasonError("provider.id is required".to_string()))?;

        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;

        info!(
            "control_panel.ai.provider.set provider_id={} has_new_api_key={} requested_status={:?} default_model={:?} endpoint={:?}",
            provider_id,
            has_new_api_key,
            provider.get("status").and_then(|value| value.as_str()),
            provider.get("defaultModel").and_then(|value| value.as_str()),
            provider.get("endpoint").and_then(|value| value.as_str()),
        );

        if provider_id == "openai-main"
            || provider_id == "google-main"
            || provider_id == "minimax-main"
        {
            let mut settings =
                Self::load_json_config_or_default(&client, AICC_SETTINGS_KEY, json!({})).await;
            let key = if provider_id == "openai-main" {
                "openai"
            } else if provider_id == "google-main" {
                "google"
            } else {
                "minimax"
            };
            let endpoint = provider
                .get("endpoint")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            let default_model = provider
                .get("defaultModel")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            let enabled = provider
                .get("status")
                .and_then(|value| value.as_str())
                .map(|value| value == "healthy" || value == "degraded")
                .unwrap_or(false)
                || has_new_api_key
                || provider
                    .get("credentialConfigured")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);

            let mut section = settings.get(key).cloned().unwrap_or_else(|| json!({}));
            if !section.is_object() {
                section = json!({});
            }

            section["enabled"] = Value::Bool(enabled);

            let existing_api_token = section
                .get("api_token")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string();
            section["api_token"] = Value::String(
                api_key
                    .clone()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or(existing_api_token),
            );

            let mut instances = section
                .get("instances")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_else(|| vec![json!({})]);
            if instances.is_empty() {
                instances.push(json!({}));
            }

            let mut first = instances.first().cloned().unwrap_or_else(|| json!({}));
            if !first.is_object() {
                first = json!({});
            }
            first["base_url"] = Value::String(endpoint.to_string());
            first["default_model"] = Value::String(default_model.to_string());
            if provider_id == "minimax-main" {
                first["provider_type"] = Value::String("minimax".to_string());
            }
            if first.get("models").is_none() {
                first["models"] = json!([default_model]);
            }
            if let Some(models) = first
                .get_mut("models")
                .and_then(|value| value.as_array_mut())
            {
                if !models
                    .iter()
                    .any(|item| item.as_str() == Some(default_model))
                {
                    models.insert(0, Value::String(default_model.to_string()));
                }
            }
            let api_key_present = section
                .get("api_token")
                .and_then(|value| value.as_str())
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false);
            instances[0] = first;
            section["instances"] = Value::Array(instances);
            settings[key] = section;
            Self::save_json_config(&client, AICC_SETTINGS_KEY, &settings).await?;
            info!(
                "control_panel.ai.provider.set persisted_aicc provider_id={} settings_key={} enabled={} api_key_present={} default_model={} endpoint={}",
                provider_id,
                key,
                enabled,
                api_key_present,
                default_model,
                endpoint,
            );
        } else {
            let mut overrides = Self::load_json_config_or_default(
                &client,
                AI_MODELS_PROVIDER_OVERRIDES_KEY,
                Self::default_ai_provider_overrides_value(),
            )
            .await;
            let mut items = overrides
                .get("items")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default();
            Self::upsert_item_by_id(&mut items, provider_id, provider.clone());
            overrides["items"] = Value::Array(items);
            Self::save_json_config(&client, AI_MODELS_PROVIDER_OVERRIDES_KEY, &overrides).await?;

            if let Some(api_key) = api_key.filter(|value| !value.trim().is_empty()) {
                let mut secret_doc = Self::load_json_config_or_default(
                    &client,
                    AI_MODELS_PROVIDER_SECRETS_KEY,
                    Self::default_ai_provider_secrets_value(),
                )
                .await;
                let mut secret_items = secret_doc
                    .get("items")
                    .and_then(|value| value.as_array())
                    .cloned()
                    .unwrap_or_default();
                Self::upsert_item_by_id(
                    &mut secret_items,
                    provider_id,
                    json!({ "id": provider_id, "apiKey": api_key }),
                );
                secret_doc["items"] = Value::Array(secret_items);
                Self::save_json_config(&client, AI_MODELS_PROVIDER_SECRETS_KEY, &secret_doc)
                    .await?;
                info!(
                    "control_panel.ai.provider.set persisted_secret provider_id={} api_key_present=true",
                    provider_id,
                );
            }
        }

        let settings =
            Self::load_json_config_or_default(&client, AICC_SETTINGS_KEY, json!({})).await;
        let secret_doc = Self::load_json_config_or_default(
            &client,
            AI_MODELS_PROVIDER_SECRETS_KEY,
            Self::default_ai_provider_secrets_value(),
        )
        .await;
        let provider_overrides = Self::load_json_config_or_default(
            &client,
            AI_MODELS_PROVIDER_OVERRIDES_KEY,
            Self::default_ai_provider_overrides_value(),
        )
        .await;
        let provider_card = Self::ai_provider_cards(
            &settings,
            provider_overrides
                .get("items")
                .and_then(|value| value.as_array())
                .map(|items| items.as_slice())
                .unwrap_or(&[]),
            &secret_doc,
        )
        .into_iter()
        .find(|item| item.get("id").and_then(|value| value.as_str()) == Some(provider_id))
        .unwrap_or(provider.clone());

        info!(
            "control_panel.ai.provider.set result provider_id={} status={:?} credential_configured={:?} default_model={:?}",
            provider_id,
            provider_card.get("status").and_then(|value| value.as_str()),
            provider_card
                .get("credentialConfigured")
                .and_then(|value| value.as_bool()),
            provider_card
                .get("defaultModel")
                .and_then(|value| value.as_str()),
        );

        Ok(RPCResponse::new(
            RPCResult::Success(json!({ "ok": true, "provider": provider_card })),
            req.seq,
        ))
    }

    async fn handle_ai_model_set(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let model = req
            .params
            .get("model")
            .cloned()
            .ok_or_else(|| RPCErrors::ReasonError("missing model payload".to_string()))?;
        let alias = model
            .get("alias")
            .and_then(|value| value.as_str())
            .ok_or_else(|| RPCErrors::ReasonError("model.alias is required".to_string()))?;

        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        let mut catalog = Self::load_json_config_or_default(
            &client,
            AI_MODELS_MODEL_CATALOG_KEY,
            Self::default_ai_model_catalog_value(),
        )
        .await;
        let mut items = catalog
            .get("items")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        Self::upsert_item_by_alias(&mut items, alias, model.clone());
        catalog["items"] = Value::Array(items);
        Self::save_json_config(&client, AI_MODELS_MODEL_CATALOG_KEY, &catalog).await?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({ "ok": true, "model": model })),
            req.seq,
        ))
    }

    async fn handle_ai_policy_set(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let policy = req
            .params
            .get("policy")
            .cloned()
            .ok_or_else(|| RPCErrors::ReasonError("missing policy payload".to_string()))?;
        let policy_id = policy
            .get("id")
            .and_then(|value| value.as_str())
            .ok_or_else(|| RPCErrors::ReasonError("policy.id is required".to_string()))?;

        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        let mut policy_doc = Self::load_json_config_or_default(
            &client,
            AI_MODELS_POLICIES_KEY,
            Self::default_ai_policies_value(),
        )
        .await;
        let mut items = policy_doc
            .get("items")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        Self::upsert_item_by_id(&mut items, policy_id, policy.clone());
        policy_doc["items"] = Value::Array(items);
        Self::save_json_config(&client, AI_MODELS_POLICIES_KEY, &policy_doc).await?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({ "ok": true, "policy": policy })),
            req.seq,
        ))
    }

    fn repo_record_version(record: &RepoRecord) -> Option<String> {
        record
            .meta
            .get("version")
            .and_then(|value| value.as_str())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    fn repo_status_rank(status: &str) -> u8 {
        match status {
            "pinned" => 2,
            "collected" => 1,
            _ => 0,
        }
    }

    fn compare_repo_app_release(
        lhs: &RepoAppReleaseCandidate,
        rhs: &RepoAppReleaseCandidate,
    ) -> Ordering {
        match (&lhs.parsed_version, &rhs.parsed_version) {
            (Some(left), Some(right)) if left != right => return left.cmp(right),
            (Some(_), None) => return Ordering::Greater,
            (None, Some(_)) => return Ordering::Less,
            _ => {}
        }

        let lhs_status = Self::repo_status_rank(lhs.record.status.as_str());
        let rhs_status = Self::repo_status_rank(rhs.record.status.as_str());
        if lhs_status != rhs_status {
            return lhs_status.cmp(&rhs_status);
        }

        let lhs_updated = lhs
            .record
            .updated_at
            .or(lhs.record.pinned_at)
            .or(lhs.record.collected_at)
            .unwrap_or(0);
        let rhs_updated = rhs
            .record
            .updated_at
            .or(rhs.record.pinned_at)
            .or(rhs.record.collected_at)
            .unwrap_or(0);
        if lhs_updated != rhs_updated {
            return lhs_updated.cmp(&rhs_updated);
        }

        lhs.app_doc.version.cmp(&rhs.app_doc.version)
    }

    async fn resolve_repo_app_release(
        &self,
        app_id: &str,
        version: Option<&str>,
    ) -> Result<RepoAppReleaseCandidate, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        info!(
            "resolve repo app release app_id=`{}` version={:?}",
            app_id, version
        );
        let repo = runtime.get_repo_client().await.map_err(|error| {
            warn!(
                "init repo client failed while resolving app `{}`: {}",
                app_id, error
            );
            RPCErrors::ReasonError(format!("Init repo client failed: {}", error))
        })?;
        let requested_version = version
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());
        let records = repo
            .list(Some(RepoListFilter::new(
                None,
                None,
                Some(app_id.to_string()),
                None,
            )))
            .await
            .map_err(|error| {
                warn!(
                    "repo.list failed while resolving app `{}` version {:?}: {}",
                    app_id, requested_version, error
                );
                error
            })?;

        let mut candidates = Vec::new();
        for record in records {
            if record.content_name.as_deref() != Some(app_id) {
                continue;
            }

            let Some(record_version) = Self::repo_record_version(&record) else {
                warn!(
                    "skip repo record without version for app `{}` content `{}`",
                    app_id, record.content_id
                );
                continue;
            };
            if requested_version
                .as_deref()
                .map(|expected| expected != record_version)
                .unwrap_or(false)
            {
                continue;
            }

            let app_doc: AppDoc = match serde_json::from_value(record.meta.clone()) {
                Ok(value) => value,
                Err(error) => {
                    warn!(
                        "skip repo record `{}` for app `{}` because meta is not AppDoc: {}",
                        record.content_id, app_id, error
                    );
                    continue;
                }
            };
            if app_doc.name != app_id {
                continue;
            }

            candidates.push(RepoAppReleaseCandidate {
                parsed_version: SemVer::parse(app_doc.version.as_str()).ok(),
                record,
                app_doc,
            });
        }

        let release = candidates
            .into_iter()
            .max_by(|lhs, rhs| Self::compare_repo_app_release(lhs, rhs))
            .ok_or_else(|| {
                let detail = requested_version
                    .map(|value| format!(" version `{value}`"))
                    .unwrap_or_default();
                RPCErrors::ReasonError(format!("No repo app release found for `{app_id}`{detail}"))
            })?;
        info!(
            "resolved repo app release app_id=`{}` version=`{}` content=`{}` status=`{}`",
            release.app_doc.name,
            release.app_doc.version,
            release.record.content_id,
            release.record.status
        );
        Ok(release)
    }

    fn build_default_install_config(app_id: &str, app_doc: &AppDoc) -> ServiceInstallConfig {
        let mut install_config = ServiceInstallConfig::default();
        install_config.local_cache_mount_point =
            app_doc.install_config_tips.local_cache_mount_point.clone();
        install_config.container_param = app_doc.install_config_tips.container_param.clone();
        install_config.start_param = app_doc.install_config_tips.start_param.clone();

        for (service_name, service_port) in app_doc.install_config_tips.service_ports.iter() {
            let mut expose = ServiceExposeConfig::default();
            if service_name == "www" {
                expose.sub_hostname.push(app_id.to_string());
            } else {
                expose.expose_port = Some(*service_port);
            }
            install_config
                .expose_config
                .insert(service_name.clone(), expose);
        }

        if app_doc.get_app_type() == AppType::Web
            && !install_config.expose_config.contains_key("www")
        {
            install_config.expose_config.insert(
                "www".to_string(),
                ServiceExposeConfig {
                    sub_hostname: vec![app_id.to_string()],
                    ..Default::default()
                },
            );
        }

        install_config
    }

    fn build_install_spec_for_user(app_doc: AppDoc, user_id: String) -> AppServiceSpec {
        let app_id = app_doc.name.clone();
        AppServiceSpec {
            install_config: Self::build_default_install_config(app_id.as_str(), &app_doc),
            app_doc,
            app_index: 0,
            user_id,
            enable: true,
            expected_instance_count: 1,
            state: ServiceState::New,
        }
    }

    fn parse_app_type(raw: &str) -> Result<AppType, RPCErrors> {
        AppType::try_from(raw.trim()).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Invalid app_type `{}`: {}", raw, error))
        })
    }

    async fn handle_app_publish(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let _principal = Self::require_rpc_principal(principal)?;
        let local_dir = Self::require_param_str(&req, "local_dir")
            .or_else(|_| Self::require_param_str(&req, "path"))?;
        let app_doc_value = req
            .params
            .get("app_doc")
            .cloned()
            .or_else(|| req.params.get("app_doc_template").cloned())
            .ok_or_else(|| RPCErrors::ReasonError("missing app_doc payload".to_string()))?;
        let app_doc: AppDoc = serde_json::from_value(app_doc_value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Invalid app_doc payload: {}", error))
        })?;
        let app_type = Self::param_str(&req, "app_type")
            .map(|raw| Self::parse_app_type(raw.as_str()))
            .transpose()?
            .unwrap_or_else(|| app_doc.get_app_type());

        info!(
            "rpc app.publish app=`{}` version=`{}` type=`{}` local_dir=`{}`",
            app_doc.name,
            app_doc.version,
            app_type.to_string(),
            local_dir
        );
        let obj_id = self
            .app_installer
            .publish_app_to_repo(app_type, Path::new(local_dir.as_str()), &app_doc)
            .await
            .map_err(|error| {
                warn!(
                    "rpc app.publish failed for app `{}` version `{}` local_dir `{}`: {}",
                    app_doc.name, app_doc.version, local_dir, error
                );
                error
            })?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "ok": true,
                "obj_id": obj_id.to_string(),
            })),
            req.seq,
        ))
    }

    async fn handle_apps_install(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let app_id = Self::require_param_str(&req, "app_id")?;
        let user_id = Self::resolve_target_user_id(&req, principal);
        let version = Self::param_str(&req, "version");
        let release = self
            .resolve_repo_app_release(app_id.as_str(), version.as_deref())
            .await?;
        let spec = Self::build_install_spec_for_user(release.app_doc, user_id);
        let task_id = self.app_installer.install_app(&spec).await?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "task_id": task_id.to_string(),
            })),
            req.seq,
        ))
    }

    async fn handle_apps_update(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let app_id = Self::require_param_str(&req, "app_id")?;
        let version = Self::require_param_str(&req, "version")?;
        let user_id = Self::resolve_target_user_id(&req, principal);
        let current_spec = self
            .app_installer
            .get_app_service_spec(app_id.as_str(), Some(user_id.as_str()))
            .await?;
        let release = self
            .resolve_repo_app_release(app_id.as_str(), Some(version.as_str()))
            .await?;

        let next_spec = AppServiceSpec {
            app_doc: release.app_doc,
            app_index: current_spec.app_index,
            user_id: current_spec.user_id.clone(),
            enable: current_spec.enable,
            expected_instance_count: current_spec.expected_instance_count,
            state: current_spec.state,
            install_config: current_spec.install_config.clone(),
        };
        let task_id = self.app_installer.upgrade_app(&next_spec).await?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "task_id": task_id.to_string(),
            })),
            req.seq,
        ))
    }

    async fn handle_apps_uninstall(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let app_id = Self::require_param_str(&req, "app_id")?;
        let user_id = Self::resolve_target_user_id(&req, principal);
        let remove_data = Self::param_bool(&req, "remove_data")
            .or_else(|| Self::param_bool(&req, "is_remove_data"))
            .unwrap_or(false);
        let task_id = self
            .app_installer
            .uninstall_app(app_id.as_str(), Some(user_id.as_str()), remove_data)
            .await?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "task_id": task_id.to_string(),
            })),
            req.seq,
        ))
    }

    async fn handle_apps_start(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let app_id = Self::require_param_str(&req, "app_id")?;
        let user_id = Self::resolve_target_user_id(&req, principal);
        let task_id = self
            .app_installer
            .start_app(app_id.as_str(), Some(user_id.as_str()))
            .await?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "ok": true,
                "task_id": task_id.to_string(),
            })),
            req.seq,
        ))
    }

    async fn handle_apps_stop(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let app_id = Self::require_param_str(&req, "app_id")?;
        let user_id = Self::resolve_target_user_id(&req, principal);
        self.app_installer
            .stop_app(app_id.as_str(), Some(user_id.as_str()))
            .await?;

        Ok(RPCResponse::new(
            RPCResult::Success(json!({ "ok": true })),
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

    async fn handle_apps_version_list(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
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

    fn run_version_command_with_timeout(binary_path: &Path, timeout: Duration) -> Option<String> {
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
                    while let Some(code) = chars.next() {
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

    fn gateway_file_summary(path: &Path) -> Value {
        let metadata = std::fs::metadata(path).ok();
        let size_bytes = metadata.as_ref().map(|meta| meta.len()).unwrap_or(0);
        let modified_at = metadata
            .as_ref()
            .and_then(|meta| meta.modified().ok())
            .map(|time| DateTime::<Utc>::from(time).to_rfc3339())
            .unwrap_or_else(|| "".to_string());

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

    async fn handle_zone_overview(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
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

    async fn handle_gateway_overview(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
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

    async fn handle_gateway_file_get(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
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

    async fn handle_container_overview(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let mut notes: Vec<String> = Vec::new();

        let server_info = match docker_command()
            .args(["info", "--format", "{{json .}}"])
            .output()
        {
            Ok(output) => {
                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                    let message = if stderr.is_empty() {
                        "docker info returned non-zero exit code".to_string()
                    } else {
                        stderr
                    };
                    notes.push(format!("Docker daemon is unavailable: {}", message));
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
                        "notes": notes,
                    });
                    return Ok(RPCResponse::new(RPCResult::Success(response), req.seq));
                }

                let content = String::from_utf8_lossy(&output.stdout).trim().to_string();
                serde_json::from_str::<Value>(content.as_str()).unwrap_or_else(|_| json!({}))
            }
            Err(error) => {
                notes.push(format!("docker command not available: {}", error));
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
                    "notes": notes,
                });
                return Ok(RPCResponse::new(RPCResult::Success(response), req.seq));
            }
        };

        let ps_output = docker_command()
            .args(["ps", "--all", "--format", "{{json .}}"])
            .output()
            .map_err(|error| RPCErrors::ReasonError(format!("docker ps failed: {}", error)))?;

        if !ps_output.status.success() {
            let stderr = String::from_utf8_lossy(&ps_output.stderr)
                .trim()
                .to_string();
            let message = if stderr.is_empty() {
                "docker ps returned non-zero exit code".to_string()
            } else {
                stderr
            };
            return Err(RPCErrors::ReasonError(format!(
                "docker ps failed: {}",
                message
            )));
        }

        let mut containers: Vec<Value> = Vec::new();
        for line in String::from_utf8_lossy(&ps_output.stdout).lines() {
            let row = line.trim();
            if row.is_empty() {
                continue;
            }

            let item = match serde_json::from_str::<Value>(row) {
                Ok(value) => value,
                Err(_) => continue,
            };

            containers.push(json!({
                "id": item.get("ID").and_then(|v| v.as_str()).unwrap_or_default(),
                "name": item.get("Names").and_then(|v| v.as_str()).unwrap_or_default(),
                "image": item.get("Image").and_then(|v| v.as_str()).unwrap_or_default(),
                "state": item.get("State").and_then(|v| v.as_str()).unwrap_or_default(),
                "status": item.get("Status").and_then(|v| v.as_str()).unwrap_or_default(),
                "ports": item.get("Ports").and_then(|v| v.as_str()).unwrap_or_default(),
                "networks": item.get("Networks").and_then(|v| v.as_str()).unwrap_or_default(),
                "createdAt": item.get("CreatedAt").and_then(|v| v.as_str()).unwrap_or_default(),
                "runningFor": item.get("RunningFor").and_then(|v| v.as_str()).unwrap_or_default(),
                "command": item.get("Command").and_then(|v| v.as_str()).unwrap_or_default(),
            }));
        }

        let running = containers
            .iter()
            .filter(|item| {
                item.get("state")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .eq_ignore_ascii_case("running")
            })
            .count() as u64;
        let paused = containers
            .iter()
            .filter(|item| {
                item.get("state")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .eq_ignore_ascii_case("paused")
            })
            .count() as u64;
        let restarting = containers
            .iter()
            .filter(|item| {
                item.get("state")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .eq_ignore_ascii_case("restarting")
            })
            .count() as u64;
        let dead = containers
            .iter()
            .filter(|item| {
                item.get("state")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .eq_ignore_ascii_case("dead")
            })
            .count() as u64;
        let exited = containers
            .iter()
            .filter(|item| {
                let state = item
                    .get("state")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                state == "exited" || state == "created"
            })
            .count() as u64;

        let response = json!({
            "available": true,
            "daemonRunning": true,
            "server": {
                "name": server_info.get("Name").and_then(|v| v.as_str()).unwrap_or_default(),
                "version": server_info.get("ServerVersion").and_then(|v| v.as_str()).unwrap_or_default(),
                "apiVersion": server_info.get("APIVersion").and_then(|v| v.as_str()).unwrap_or_default(),
                "os": server_info.get("OperatingSystem").and_then(|v| v.as_str()).unwrap_or_default(),
                "kernel": server_info.get("KernelVersion").and_then(|v| v.as_str()).unwrap_or_default(),
                "driver": server_info.get("Driver").and_then(|v| v.as_str()).unwrap_or_default(),
                "cgroupDriver": server_info.get("CgroupDriver").and_then(|v| v.as_str()).unwrap_or_default(),
                "cpuCount": server_info.get("NCPU").and_then(|v| v.as_u64()).unwrap_or_default(),
                "memTotalBytes": server_info.get("MemTotal").and_then(|v| v.as_u64()).unwrap_or_default(),
            },
            "summary": {
                "total": containers.len() as u64,
                "running": running,
                "paused": paused,
                "exited": exited,
                "restarting": restarting,
                "dead": dead,
            },
            "containers": containers,
            "notes": notes,
        });

        Ok(RPCResponse::new(RPCResult::Success(response), req.seq))
    }

    async fn handle_container_action(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
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

    async fn handle_chat_bootstrap(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_chat_principal(principal)?;
        let can_send = matches!(principal.user_type, UserType::Root | UserType::Admin);
        let response = ChatBootstrapResponse {
            scope: Self::chat_scope_info(principal),
            capabilities: ChatCapabilityInfo {
                contact_list: true,
                message_list: true,
                message_send: can_send,
                thread_id_send: can_send,
                realtime_events: false,
                standalone_chat_app_link: true,
                opendan_channel_ready: false,
            },
            notes: vec![
                "Message Hub currently uses a browser-safe wrapper over msg-center."
                    .to_string(),
                "The current standalone route is /message-hub/chat while the backend adapter remains in transition."
                    .to_string(),
                "Future email, calendar, notification, TODO, and agent record views remain follow-up work."
                    .to_string(),
            ],
        };

        Ok(RPCResponse::new(
            RPCResult::Success(json!(response)),
            req.seq,
        ))
    }

    async fn handle_chat_contact_list(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_chat_principal(principal)?;
        let owner_did = Self::parse_chat_owner_did(principal)?;
        let msg_center = self.get_msg_center_client().await?;
        let limit = Self::normalize_chat_contact_limit(Self::param_usize(&req, "limit"));
        let keyword = Self::param_str(&req, "keyword")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let query = ContactQuery {
            keyword,
            limit: Some(limit),
            offset: Self::param_u64(&req, "offset"),
            ..Default::default()
        };

        let mut items = msg_center
            .list_contacts(query, Some(owner_did))
            .await?
            .into_iter()
            .map(Self::map_chat_contact)
            .collect::<Vec<_>>();
        items.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.name.cmp(&right.name))
        });

        Ok(RPCResponse::new(
            RPCResult::Success(json!(ChatContactListResponse {
                scope: Self::chat_scope_info(principal),
                items,
            })),
            req.seq,
        ))
    }

    async fn handle_chat_message_list(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_chat_principal(principal)?;
        let owner_did = Self::parse_chat_owner_did(principal)?;
        let peer_did_raw = Self::require_param_str(&req, "peer_did")?;
        let peer_did = DID::from_str(peer_did_raw.trim()).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Invalid peer_did `{}`: {}", peer_did_raw, error))
        })?;
        let limit = Self::normalize_chat_message_limit(Self::param_usize(&req, "limit"));
        let scan_limit = Self::chat_scan_limit(limit);
        let msg_center = self.get_msg_center_client().await?;

        let peer_name = match msg_center
            .get_contact(peer_did.clone(), Some(owner_did.clone()))
            .await
        {
            Ok(Some(contact)) => Some(contact.name),
            Ok(None) => None,
            Err(error) => {
                log::warn!(
                    "chat.message.list get_contact failed: peer={:?} owner={:?} err={}",
                    peer_did,
                    owner_did,
                    error
                );
                None
            }
        };

        let inbox = msg_center
            .list_box_by_time(
                owner_did.clone(),
                BoxKind::Inbox,
                None,
                Some(scan_limit),
                None,
                None,
                Some(true),
                Some(true),
            )
            .await?;
        let outbox = msg_center
            .list_box_by_time(
                owner_did.clone(),
                BoxKind::Outbox,
                None,
                Some(scan_limit),
                None,
                None,
                Some(true),
                Some(true),
            )
            .await?;

        let mut records = inbox
            .items
            .into_iter()
            .chain(outbox.items.into_iter())
            .filter(|record| Self::chat_record_matches_peer(record, &owner_did, &peer_did))
            .collect::<Vec<_>>();
        records.sort_by(|left, right| {
            right
                .record
                .sort_key
                .cmp(&left.record.sort_key)
                .then_with(|| right.record.updated_at_ms.cmp(&left.record.updated_at_ms))
                .then_with(|| right.record.record_id.cmp(&left.record.record_id))
        });
        records.truncate(limit);

        let items = records
            .iter()
            .map(|record| Self::map_chat_message_record(record, &owner_did, peer_name.clone()))
            .collect::<Vec<_>>();

        Ok(RPCResponse::new(
            RPCResult::Success(json!(ChatMessageListResponse {
                scope: Self::chat_scope_info(principal),
                peer_did: peer_did.to_string(),
                peer_name,
                items,
            })),
            req.seq,
        ))
    }

    async fn handle_chat_message_send(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_chat_principal(principal)?;
        let owner_did = Self::parse_chat_owner_did(principal)?;
        let target_did_raw = Self::require_param_str(&req, "target_did")?;
        let target_did = DID::from_str(target_did_raw.trim()).map_err(|error| {
            RPCErrors::ParseRequestError(format!(
                "Invalid target_did `{}`: {}",
                target_did_raw, error
            ))
        })?;
        let content = Self::require_param_str(&req, "content")?.trim().to_string();
        if content.is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "content cannot be empty".to_string(),
            ));
        }
        let thread_id = Self::param_str(&req, "thread_id")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let msg_center = self.get_msg_center_client().await?;

        let peer_name = match msg_center
            .get_contact(target_did.clone(), Some(owner_did.clone()))
            .await
        {
            Ok(Some(contact)) => Some(contact.name),
            Ok(None) => None,
            Err(error) => {
                log::warn!(
                    "chat.message.send get_contact failed: peer={:?} owner={:?} err={}",
                    target_did,
                    owner_did,
                    error
                );
                None
            }
        };

        let mut message = MsgObject {
            from: owner_did.clone(),
            to: vec![target_did.clone()],
            kind: MsgObjKind::Chat,
            created_at_ms: Self::current_time_ms(),
            content: MsgContent {
                format: Some(MsgContentFormat::TextPlain),
                content: content.clone(),
                ..Default::default()
            },
            ..Default::default()
        };
        if let Some(thread_id) = thread_id.as_ref() {
            message.thread.topic = Some(thread_id.clone());
            message.thread.correlation_id = Some(thread_id.clone());
            message
                .meta
                .insert("session_id".to_string(), Value::String(thread_id.clone()));
            message.meta.insert(
                "owner_session_id".to_string(),
                Value::String(thread_id.clone()),
            );
        }

        let result = msg_center
            .post_send(
                message.clone(),
                Some(SendContext {
                    contact_mgr_owner: Some(owner_did.clone()),
                    ..Default::default()
                }),
                None,
            )
            .await?;
        if !result.ok {
            return Err(RPCErrors::ReasonError(result.reason.unwrap_or_else(|| {
                "msg-center rejected the chat send request".to_string()
            })));
        }

        let first_delivery = result.deliveries.first().ok_or_else(|| {
            RPCErrors::ReasonError("msg-center returned no delivery record".to_string())
        })?;
        let stored_record = msg_center
            .get_record(first_delivery.record_id.clone(), Some(true))
            .await?;
        let mapped_message = if let Some(record) = stored_record.as_ref() {
            Self::map_chat_message_record(record, &owner_did, peer_name.clone())
        } else {
            ChatMessageView {
                record_id: first_delivery.record_id.clone(),
                msg_id: result.msg_id.to_string(),
                direction: "outbound",
                peer_did: target_did.to_string(),
                peer_name,
                state: "sent",
                created_at_ms: message.created_at_ms,
                updated_at_ms: message.created_at_ms,
                sort_key: message.created_at_ms,
                thread_id,
                content,
                content_format: Some("TextPlain".to_string()),
            }
        };

        Ok(RPCResponse::new(
            RPCResult::Success(json!(ChatSendMessageResponse {
                scope: Self::chat_scope_info(principal),
                target_did: target_did.to_string(),
                delivery_count: result.deliveries.len(),
                message: mapped_message,
            })),
            req.seq,
        ))
    }

    async fn handle_chat_stream_http(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let fallback_token = Self::extract_http_session_token(&req);
        let collected = req.into_body().collect().await.map_err(|error| {
            server_err!(
                ServerErrorCode::BadRequest,
                "Failed to read chat stream request body: {}",
                error
            )
        })?;
        let body = collected.to_bytes();
        let stream_req = match serde_json::from_slice::<ChatStreamHttpRequest>(&body) {
            Ok(value) => value,
            Err(error) => {
                return Self::build_http_json_response(
                    StatusCode::BAD_REQUEST,
                    json!({
                        "error": format!("Invalid chat stream request: {}", error),
                    }),
                );
            }
        };

        let peer_did_raw = stream_req.peer_did.trim().to_string();
        if peer_did_raw.is_empty() {
            return Self::build_http_json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "peer_did is required" }),
            );
        }
        let peer_did = match DID::from_str(peer_did_raw.as_str()) {
            Ok(value) => value,
            Err(error) => {
                return Self::build_http_json_response(
                    StatusCode::BAD_REQUEST,
                    json!({
                        "error": format!("Invalid peer_did `{}`: {}", peer_did_raw, error),
                    }),
                );
            }
        };

        let thread_id = stream_req
            .thread_id
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let keepalive_ms = Self::normalize_chat_stream_keepalive_ms(stream_req.keepalive_ms);
        let principal = match self
            .authenticate_session_token_for_method(
                "chat.stream",
                stream_req.session_token.or(fallback_token),
            )
            .await
        {
            Ok(Some(principal)) => principal,
            Ok(None) => {
                return Self::build_http_json_response(
                    StatusCode::UNAUTHORIZED,
                    json!({ "error": "chat stream requires an authenticated session" }),
                );
            }
            Err(error) => {
                let status = match error {
                    RPCErrors::InvalidToken(_) => StatusCode::UNAUTHORIZED,
                    RPCErrors::NoPermission(_) => StatusCode::FORBIDDEN,
                    _ => StatusCode::BAD_REQUEST,
                };
                return Self::build_http_json_response(
                    status,
                    json!({ "error": error.to_string() }),
                );
            }
        };
        let owner_did = match Self::parse_chat_owner_did(&principal) {
            Ok(value) => value,
            Err(error) => {
                return Self::build_http_json_response(
                    StatusCode::BAD_REQUEST,
                    json!({ "error": error.to_string() }),
                );
            }
        };

        let owner_token = owner_did.to_raw_host_name();
        let patterns = vec![
            format!("/msg_center/{}/box/in/**", owner_token),
            format!("/msg_center/{}/box/out/**", owner_token),
        ];
        let event_reader = match Self::get_chat_kevent_client()
            .create_event_reader(patterns)
            .await
        {
            Ok(reader) => reader,
            Err(error) => {
                return Self::build_http_json_response(
                    StatusCode::BAD_REQUEST,
                    json!({
                        "error": format!("Failed to create chat event reader: {}", error),
                    }),
                );
            }
        };

        let (sender, receiver) = mpsc::channel::<std::result::Result<Bytes, ServerError>>(32);
        let scope = Self::chat_scope_info(&principal);
        let peer_name = match self.get_msg_center_client().await {
            Ok(msg_center) => match msg_center
                .get_contact(peer_did.clone(), Some(owner_did.clone()))
                .await
            {
                Ok(Some(contact)) => Some(contact.name),
                Ok(None) => None,
                Err(error) => {
                    log::warn!(
                        "chat.stream get_contact failed: peer={:?} owner={:?} err={}",
                        peer_did,
                        owner_did,
                        error
                    );
                    None
                }
            },
            Err(error) => {
                return Self::build_http_json_response(
                    StatusCode::BAD_REQUEST,
                    json!({ "error": error.to_string() }),
                );
            }
        };

        if !Self::send_chat_stream_json(
            &sender,
            &json!({
                "type": "ack",
                "connection_id": Uuid::new_v4().to_string(),
                "scope": scope,
                "peer_did": peer_did.to_string(),
                "thread_id": thread_id.clone(),
                "keepalive_ms": keepalive_ms,
                "at_ms": Self::current_time_ms(),
            }),
        )
        .await
        {
            return Self::build_http_json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Failed to initialize chat stream" }),
            );
        }

        let server = self.clone();
        tokio::spawn(async move {
            let msg_center = match server.get_msg_center_client().await {
                Ok(client) => client,
                Err(error) => {
                    let _ = Self::send_chat_stream_error(&sender, error.to_string()).await;
                    return;
                }
            };

            loop {
                let event = match event_reader.pull_event(Some(keepalive_ms)).await {
                    Ok(Some(event)) => event,
                    Ok(None) => {
                        if !Self::send_chat_stream_json(
                            &sender,
                            &json!({
                                "type": "keepalive",
                                "at_ms": Self::current_time_ms(),
                            }),
                        )
                        .await
                        {
                            return;
                        }
                        continue;
                    }
                    Err(error) => {
                        let _ = Self::send_chat_stream_error(
                            &sender,
                            format!("chat event reader failed: {}", error),
                        )
                        .await;
                        return;
                    }
                };

                let record_id = match Self::chat_record_id_from_event(&event) {
                    Some(record_id) => record_id,
                    None => {
                        if !Self::send_chat_stream_json(
                            &sender,
                            &json!({
                                "type": "resync",
                                "reason": "missing_record_id",
                                "peer_did": peer_did.to_string(),
                                "thread_id": thread_id.clone(),
                                "at_ms": Self::current_time_ms(),
                            }),
                        )
                        .await
                        {
                            return;
                        }
                        continue;
                    }
                };

                let record = match msg_center.get_record(record_id.clone(), Some(true)).await {
                    Ok(Some(record)) => record,
                    Ok(None) => {
                        if !Self::send_chat_stream_json(
                            &sender,
                            &json!({
                                "type": "resync",
                                "reason": "record_not_found",
                                "record_id": record_id,
                                "peer_did": peer_did.to_string(),
                                "thread_id": thread_id.clone(),
                                "at_ms": Self::current_time_ms(),
                            }),
                        )
                        .await
                        {
                            return;
                        }
                        continue;
                    }
                    Err(error) => {
                        let _ = Self::send_chat_stream_error(
                            &sender,
                            format!("Failed to load chat record {}: {}", record_id, error),
                        )
                        .await;
                        return;
                    }
                };

                if !Self::chat_record_matches_stream(
                    &record,
                    &owner_did,
                    &peer_did,
                    thread_id.as_deref(),
                ) {
                    continue;
                }

                let message = Self::map_chat_message_record(&record, &owner_did, peer_name.clone());
                if !Self::send_chat_stream_json(
                    &sender,
                    &json!({
                        "type": "message",
                        "operation": Self::chat_event_operation(&event),
                        "record_id": record.record.record_id,
                        "message": message,
                        "at_ms": Self::current_time_ms(),
                    }),
                )
                .await
                {
                    return;
                }
            }
        });

        Self::build_chat_stream_response(receiver)
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
            "dashboard" | "ui.dashboard" => self.handle_dashboard(req).await,
            "ui.locale.get" => self.handle_ui_locale_get(req).await,
            "ui.locale.set" => self.handle_ui_locale_set(req).await,
            // Auth
            "auth.login" => self.handle_auth_login(req).await,
            "auth.issue_sso_token" => self.handle_auth_issue_sso_token(req).await,
            "auth.logout" => self.handle_auth_logout(req).await,
            "auth.refresh" => self.handle_auth_refresh(req).await,
            "auth.verify" => self.handle_auth_verify(req).await,
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
            "apps.version.list" => self.handle_apps_version_list(req).await,
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
            "apps.install" => self.handle_apps_install(req, principal.as_ref()).await,
            "apps.update" => self.handle_apps_update(req, principal.as_ref()).await,
            "apps.uninstall" => self.handle_apps_uninstall(req, principal.as_ref()).await,
            "apps.start" => self.handle_apps_start(req, principal.as_ref()).await,
            "apps.stop" => self.handle_apps_stop(req, principal.as_ref()).await,
            "app.publish" => self.handle_app_publish(req, principal.as_ref()).await,
            // Chat
            "chat.bootstrap" => self.handle_chat_bootstrap(req, principal.as_ref()).await,
            "chat.contact.list" => self.handle_chat_contact_list(req, principal.as_ref()).await,
            "chat.message.list" => self.handle_chat_message_list(req, principal.as_ref()).await,
            "chat.message.send" => self.handle_chat_message_send(req, principal.as_ref()).await,
            // Network
            "network.interfaces" => self.handle_unimplemented(req, "List interfaces").await,
            "network.interface.update" => {
                self.handle_unimplemented(req, "Update interface config")
                    .await
            }
            "network.overview" | "network.metrics" => self.handle_network_overview(req).await,
            "network.dns" => self.handle_unimplemented(req, "Get/set DNS").await,
            "network.ddns" => self.handle_unimplemented(req, "Get/set DDNS").await,
            "network.firewall.rules" => self.handle_unimplemented(req, "List firewall rules").await,
            "network.firewall.update" => {
                self.handle_unimplemented(req, "Update firewall rules")
                    .await
            }
            "zone.overview" | "zone.config" => self.handle_zone_overview(req).await,
            "gateway.overview" | "gateway.config" => self.handle_gateway_overview(req).await,
            "gateway.file.get" => self.handle_gateway_file_get(req).await,
            "container.overview" | "containers.overview" | "docker.overview" => {
                self.handle_container_overview(req).await
            }
            "container.action" | "containers.action" | "docker.action" => {
                self.handle_container_action(req).await
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
        let method = req.method().clone();
        let path = req.uri().path().to_string();

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
            return serve_http_by_rpc_handler(req, info, self).await;
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
    set_buckyos_api_runtime(runtime);

    let control_panel_server = ControlPanelServer::new();
    control_panel_server
        .init_file_manager()
        .await
        .map_err(|err| anyhow::anyhow!("init control-panel file manager failed: {}", err))?;
    let control_panel_server = Arc::new(control_panel_server);
    // Bind to the default control-panel service port.

    let runner = Runner::new(CONTROL_PANEL_SERVICE_PORT);
    // 添加 RPC 服务
    let _ = runner.add_http_server(
        "/kapi/control-panel".to_string(),
        control_panel_server.clone(),
    );
    // File manager API exposed by control-panel.
    let _ = runner.add_http_server("/api".to_string(), control_panel_server.clone());

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
