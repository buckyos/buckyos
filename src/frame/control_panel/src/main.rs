mod agent_daemon;
mod agent_mgr;
mod agent_runtimes;
mod aiws_pkg;
mod app_install_deployer;
mod app_login;
mod app_install_driver;
mod app_install_engine;
mod app_install_planner;
mod app_install_resolver;
mod app_install_runner;
mod app_installer;
mod app_mutation;
mod app_package_namespace;
mod app_servcie_mgr;
mod app_staging;
mod buckets;
mod cluster_mgr;
mod dashboard;
mod desktop_folder;
mod diagnostic_mgr;
mod download_mgr;
mod exclusive_invite;
mod federation;
mod image_data_url;
mod ndn_download;
mod pikg;
mod pre_install_reconciler;
mod product;
mod redaction;
mod sso_entry;
mod sso_identity;
mod sso_settings;
mod sys_auth_backend;
mod sys_log_mgr;
mod task_ops;
mod ui_session_mgr;
mod user_group_mgr;
mod user_mgr;
mod zone_mgr;

use diagnostic_mgr::DiagnosticBundleEntry;
use sys_log_mgr::LogDownloadEntry;

use ::kRPC::*;
use anyhow::Result;
use async_trait::async_trait;
use buckyos_api::{
    get_buckyos_api_runtime, hide_child_console, init_buckyos_api_runtime, set_buckyos_api_runtime,
    BuckyOSRuntimeType, SystemConfigClient, UserType, CONTROL_PANEL_SERVICE_NAME,
    CONTROL_PANEL_SERVICE_PORT,
};
use buckyos_http_server::*;
use buckyos_kit::*;
use bytes::Bytes;
use http::header::{CACHE_CONTROL, CONTENT_TYPE};
use http::{Method, StatusCode, Version};
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use log::info;
use serde_json::*;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::OsStr;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::{net::IpAddr, time::Instant};
use sysinfo::{Disks, Networks};
use tokio::sync::{Mutex, RwLock};

// RPC docs live under doc/dashboard. UI endpoints use "ui.*" names.

pub(crate) fn bytes_to_gb(bytes: u64) -> f64 {
    (bytes as f64) / 1024.0 / 1024.0 / 1024.0
}

fn external_command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    hide_child_console(&mut command);
    command
}

fn docker_command() -> Command {
    external_command("docker")
}

const METRICS_DISK_REFRESH_INTERVAL_SECS: u64 = 5;
const NETWORK_TIMELINE_LIMIT: usize = 300;
const DOCKER_OVERVIEW_CACHE_TTL_SECS: u64 = 15;
const GATEWAY_CONFIG_FILES: [&str; 6] = [
    "cyfs_gateway.json",
    "node_gateway_params.json",
    "boot_gateway.yaml",
    "node_gateway.json",
    "user_gateway.yaml",
    "post_gateway.yaml",
];
const CONTROL_PANEL_LOCALE_KEY: &str = "services/control_panel/settings/locale";
/// Request facts the HTTP layer adds to kRPC params; callers cannot set them.
const REQ_HOST_PARAM: &str = "x_req_host";
const REQ_ORIGIN_PARAM: &str = "x_req_origin";

pub(crate) fn gateway_etc_dir() -> PathBuf {
    get_buckyos_system_etc_dir()
}

pub(crate) fn sn_self_cert_state_path() -> PathBuf {
    get_buckyos_root_dir()
        .join("data")
        .join("cyfs_gateway")
        .join("sn_dns")
        .join("self_cert_state.json")
}

#[derive(Clone, Debug)]
struct RpcAuthPrincipal {
    username: String,
    owner_user_id: String,
    authenticated_app_id: String,
    user_type: UserType,
    owner_did: String,
    is_user_session: bool,
    is_control_panel_session: bool,
    principal_kind: buckyos_api::TokenPrincipalKind,
    agent_permission_mode: Option<buckyos_api::AgentPermissionMode>,
    session: PrincipalSession,
}

/// What the verified session says beyond the account: its ceiling (restricted
/// Limited / External sessions), how it was established, and whether it is a
/// short sudo session.
#[derive(Clone, Debug, Default)]
struct PrincipalSession {
    ceiling: buckyos_api::SessionCeiling,
    auth_method: Option<buckyos_api::AuthMethod>,
    is_external: bool,
    is_sudo: bool,
}

impl RpcAuthPrincipal {
    fn is_restricted(&self) -> bool {
        self.session.ceiling.restricted || self.session.is_external
    }

    fn require_agent_permission(&self, method: &str) -> Result<(), RPCErrors> {
        if self.agent_permission_mode == Some(buckyos_api::AgentPermissionMode::ReadOnly)
            && !matches!(
                method,
                "executor.online.list"
                    | "executor.stream.open"
                    | "ui.locale.get"
                    | "ui.dashboard"
                    | "user.list"
                    | "user.get"
                    | "user.profile.get"
                    | "user.invite.get"
                    | "user_group.list"
                    | "agent.check_name"
                    | "agent.list_templates"
                    | "agent.create.status"
                    | "agent.list"
                    | "agent.get"
                    | "agent.profile.get"
                    | "agent.runtime_directory"
                    | "system.overview"
                    | "system.buckyos_info.get"
                    | "system.product.get"
                    | "system.dev_mode.get"
                    | "system.status"
                    | "system.metrics"
                    | "network.overview"
                    | "system.logs.list"
                    | "system.logs.query"
                    | "system.logs.tail"
                    | "system.logs.download"
                    | "diagnostic.export"
                    | "system.update.check"
                    | "apps.list"
                    | "apps.details"
                    | "apps.status"
                    | "apps.availability.get"
                    | "apps.availability.check"
                    | "apps.staging.status"
                    | "apps.install.status"
                    | "apps.upgrade.check"
                    | "zone.overview"
                    | "zone.devices.list"
                    | "zone.certificates.list"
                    | "zone.presentation.get"
                    | "gateway.overview"
                    | "gateway.file.get"
                    | "container.overview"
                    | "desktop.session.list"
                    | "desktop.state.get"
            )
        {
            return Err(RPCErrors::NoPermission(format!(
                "read-only Agent cannot call {method}"
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod agent_permission_tests {
    use super::*;
    use buckyos_api::AgentPermissionMode::{ReadOnly, ReadWrite};

    fn principal(mode: Option<buckyos_api::AgentPermissionMode>) -> RpcAuthPrincipal {
        RpcAuthPrincipal {
            username: "alice".to_string(),
            owner_user_id: "alice".to_string(),
            authenticated_app_id: "helper.example.com@alice".to_string(),
            user_type: UserType::Admin,
            owner_did: "did:web:alice.example.com".to_string(),
            is_user_session: false,
            is_control_panel_session: false,
            principal_kind: buckyos_api::TokenPrincipalKind::App,
            agent_permission_mode: mode,
            session: PrincipalSession::default(),
        }
    }

    #[test]
    fn agent_permission_blocks_admin_rpc_and_desktop_writes() {
        let read_only = principal(Some(ReadOnly));
        for method in [
            "agent.profile.set",
            "agent.update",
            "user.profile.set",
            "user.create",
            "apps.start",
            "apps.submit",
            "apps.uninstall",
            "container.action",
            "download.create",
            "diagnostic.collect",
            "ui.locale.set",
            "desktop.session.create",
            "desktop.session.rename",
            "desktop.session.delete",
            "desktop.state.set",
            "desktop.state.delete",
            "future.read",
        ] {
            assert!(
                matches!(
                    read_only.require_agent_permission(method),
                    Err(RPCErrors::NoPermission(_))
                ),
                "{method}"
            );
        }
    }

    #[test]
    fn agent_permission_keeps_reads_and_unrestricted_principals() {
        let read_only = principal(Some(ReadOnly));
        for method in [
            "agent.get",
            "agent.create.status",
            "apps.list",
            "apps.status",
            "system.logs.query",
            "gateway.file.get",
            "desktop.session.list",
            "desktop.state.get",
        ] {
            assert!(
                read_only.require_agent_permission(method).is_ok(),
                "{method}"
            );
        }
        for mode in [None, Some(ReadWrite)] {
            let caller = principal(mode);
            assert!(caller.require_agent_permission("apps.submit").is_ok());
            assert!(caller.require_agent_permission("desktop.state.set").is_ok());
        }
    }
}

#[cfg(test)]
mod rpc_wire_tests {
    use super::*;
    use buckyos_api::{AccessDenial, DenyReason};

    #[test]
    fn structured_denials_keep_their_code_on_the_wire() {
        let error = AccessDenial::new(DenyReason::SessionKindNotAllowed)
            .at("control-panel", "user.list")
            .into_rpc_error();
        let (response, detail) =
            ControlPanelServer::failed_rpc_response(error, 7, Some("trace".to_string()));
        let wire = response.to_wire_value(detail.as_ref()).unwrap();
        assert_eq!(wire["code"], "access_denied");
        assert_eq!(wire["data"]["reason"], "session_kind_not_allowed");
        assert_eq!(wire["data"]["adjust"]["party"], "product");
        assert_eq!(wire["data"]["method"], "user.list");
        assert_eq!(wire["sys"], json!([7, "trace"]));
        let parsed = RPCErrorDetail::from_response_value(&wire).unwrap();
        assert_eq!(
            AccessDenial::from_rpc_error(&RPCErrors::Detailed(parsed))
                .unwrap()
                .reason,
            DenyReason::SessionKindNotAllowed
        );

        let (plain, detail) = ControlPanelServer::failed_rpc_response(
            RPCErrors::ReasonError("boom".to_string()),
            8,
            None,
        );
        let wire = plain.to_wire_value(detail.as_ref()).unwrap();
        assert!(wire.get("code").is_none() && wire["error"].is_string());
    }

    #[test]
    fn client_address_comes_from_the_gateways_last_forwarded_hop() {
        let loopback: IpAddr = "127.0.0.1".parse().unwrap();
        let headers = |xff: Option<&str>, real_ip: Option<&str>| {
            let mut headers = http::HeaderMap::new();
            if let Some(xff) = xff {
                headers.insert("x-forwarded-for", xff.parse().unwrap());
            }
            if let Some(real_ip) = real_ip {
                headers.insert("x-real-ip", real_ip.parse().unwrap());
            }
            headers
        };
        assert_eq!(
            ControlPanelServer::forwarded_client_ip(
                &headers(Some("10.9.9.9, 203.0.113.7"), Some("198.51.100.1")),
                loopback
            ),
            "203.0.113.7".parse::<IpAddr>().unwrap()
        );
        assert_eq!(
            ControlPanelServer::forwarded_client_ip(&headers(None, Some("198.51.100.1")), loopback),
            loopback
        );
        assert_eq!(
            ControlPanelServer::forwarded_client_ip(&headers(Some("garbage"), None), loopback),
            loopback
        );
        let direct: IpAddr = "192.168.1.20".parse().unwrap();
        assert_eq!(
            ControlPanelServer::forwarded_client_ip(&headers(Some("203.0.113.7"), None), direct),
            direct
        );
    }

    #[test]
    fn typed_params_drop_transport_fields_and_refuse_others() {
        #[derive(serde::Deserialize, Debug)]
        #[serde(deny_unknown_fields)]
        struct Params {
            invite_id: String,
        }
        let req = RPCRequest::new(
            "auth.invite.request_resend",
            json!({"invite_id": "ei-1", "x_req_host": "a", "x_req_origin": "https://a", "session_token": "t"}),
        );
        let params: Params = ControlPanelServer::typed_params(&req).unwrap();
        assert_eq!(params.invite_id, "ei-1");
        let req = RPCRequest::new(
            "auth.invite.request_resend",
            json!({"invite_id": "ei-1", "user_id": "admin"}),
        );
        assert!(matches!(
            ControlPanelServer::typed_params::<Params>(&req),
            Err(RPCErrors::ParseRequestError(_))
        ));
    }
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
    agent_daemon: Arc<agent_daemon::AgentDaemonHub>,
    log_downloads: Arc<Mutex<HashMap<String, LogDownloadEntry>>>,
    diagnostic_bundles: Arc<Mutex<HashMap<String, DiagnosticBundleEntry>>>,
    running_diagnostic_tasks: Arc<Mutex<HashSet<String>>>,
    running_download_tasks: Arc<Mutex<HashSet<String>>>,
    metrics_snapshot: Arc<RwLock<SystemMetricsSnapshot>>,
    pending_sso_logins: Arc<Mutex<HashMap<u64, sys_auth_backend::PendingSsoLogin>>>,
    pending_did_logins: Arc<Mutex<HashMap<String, federation::PendingDidLogin>>>,
    external_profiles: Arc<std::sync::Mutex<HashMap<String, buckyos_api::FederationProfile>>>,
    registration_limiter: Arc<std::sync::Mutex<sso_entry::RegistrationLimiter>>,
    app_login_limiter: Arc<std::sync::Mutex<app_login::AppLoginLimiter>>,
    docker_overview_cache: Arc<Mutex<Option<DockerOverviewCacheEntry>>>,
    docker_overview_refresh_lock: Arc<Mutex<()>>,
    app_installer: app_installer::AppInstaller,
    install_engine: Arc<app_install_engine::InstallEngine>,
    install_runner: Arc<app_install_runner::InstallRunner>,
    running_update_batch_tasks: Arc<tokio::sync::Mutex<HashSet<String>>>,
    staging_store: Arc<app_staging::PikgStagingStore>,
    agent_drives: Arc<std::sync::Mutex<HashMap<String, bool>>>,
}

impl ControlPanelServer {
    pub fn new() -> Self {
        let metrics_snapshot = Arc::new(RwLock::new(SystemMetricsSnapshot::default()));
        Self::start_metrics_sampler(metrics_snapshot.clone());
        let staging_store = Arc::new(app_staging::PikgStagingStore::new());
        let staging_gc = staging_store.clone();
        tokio::spawn(async move {
            if let Err(error) = staging_gc.gc().await {
                log::warn!("pikg staging startup GC failed: {error}");
            }
        });
        let install_engine = Arc::new(app_install_engine::InstallEngine::new(
            Arc::new(app_install_engine::TaskMgrInstallStore),
            Arc::new(app_install_driver::ProductionInstallDriver::new(
                staging_store.clone(),
            )),
        ));
        let install_runner = app_install_runner::InstallRunner::new(install_engine.clone());
        ControlPanelServer {
            agent_daemon: Arc::new(agent_daemon::AgentDaemonHub::default()),
            log_downloads: Arc::new(Mutex::new(HashMap::new())),
            diagnostic_bundles: Arc::new(Mutex::new(HashMap::new())),
            running_diagnostic_tasks: Arc::new(Mutex::new(HashSet::new())),
            running_download_tasks: Arc::new(Mutex::new(HashSet::new())),
            metrics_snapshot,
            pending_sso_logins: Arc::new(Mutex::new(HashMap::new())),
            pending_did_logins: Arc::new(Mutex::new(HashMap::new())),
            external_profiles: Arc::new(std::sync::Mutex::new(HashMap::new())),
            registration_limiter: Arc::new(std::sync::Mutex::new(
                sso_entry::RegistrationLimiter::default(),
            )),
            app_login_limiter: Arc::new(std::sync::Mutex::new(
                app_login::AppLoginLimiter::default(),
            )),
            docker_overview_cache: Arc::new(Mutex::new(None)),
            docker_overview_refresh_lock: Arc::new(Mutex::new(())),
            app_installer: app_installer::AppInstaller::new(),
            install_engine,
            install_runner,
            running_update_batch_tasks: Arc::new(tokio::sync::Mutex::new(HashSet::new())),
            staging_store,
            agent_drives: Arc::new(std::sync::Mutex::new(HashMap::new())),
        }
    }

    /// The storage behind the mounted disks, each counted once: container layers and repeated
    /// mounts of one file system would otherwise multiply the capacity (name-lib `disk_space`).
    pub(crate) fn distinct_disks(disks: &Disks) -> Vec<name_lib::DiskSpaceSample> {
        name_lib::distinct_disk_samples(disks.list().iter().map(|disk| name_lib::DiskSpaceSample {
            device: disk.name().to_string_lossy().into_owned(),
            file_system: disk.file_system().to_string_lossy().into_owned(),
            mount_point: disk.mount_point().to_string_lossy().into_owned(),
            total: disk.total_space(),
            available: disk.available_space(),
        }))
    }

    fn sum_disks(disks: &Disks) -> (u64, u64, Vec<Value>) {
        let mut total_bytes: u64 = 0;
        let mut used_bytes: u64 = 0;
        let mut details: Vec<Value> = Vec::new();

        for disk in Self::distinct_disks(disks) {
            let total = disk.total;
            let used = disk.used();
            total_bytes = total_bytes.saturating_add(total);
            used_bytes = used_bytes.saturating_add(used);
            let usage_percent = if total > 0 {
                ((used as f64 / total as f64) * 100.0).round()
            } else {
                0.0
            };

            details.push(json!({
                "label": disk.device,
                "totalGb": bytes_to_gb(total),
                "usedGb": bytes_to_gb(used),
                "usagePercent": usage_percent,
                "fs": disk.file_system,
                "mount": disk.mount_point,
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

    fn param_str(req: &RPCRequest, key: &str) -> Option<String> {
        req.params
            .get(key)
            .and_then(|value| value.as_str())
            .map(|value| value.to_string())
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

    /// A text field patch: `None` keeps the stored value, `Some(None)` clears it (JSON null or a
    /// blank string), `Some(Some(text))` sets the trimmed text.
    fn param_text_patch(req: &RPCRequest, key: &str) -> Result<Option<Option<String>>, RPCErrors> {
        match req.params.get(key) {
            None => Ok(None),
            Some(Value::Null) => Ok(Some(None)),
            Some(Value::String(value)) => {
                let value = value.trim();
                Ok(Some((!value.is_empty()).then(|| value.to_string())))
            }
            Some(_) => Err(RPCErrors::ParseRequestError(format!(
                "{key} must be a string or null"
            ))),
        }
    }

    fn require_param_str(req: &RPCRequest, key: &str) -> Result<String, RPCErrors> {
        Self::param_str(req, key).ok_or(RPCErrors::ParseRequestError(format!("Missing {}", key)))
    }

    /// Params as a typed request, without the fields the transport adds; a
    /// struct with `deny_unknown_fields` refuses anything else the caller sends.
    fn typed_params<T: serde::de::DeserializeOwned>(req: &RPCRequest) -> Result<T, RPCErrors> {
        let mut params = req.params.clone();
        if let Some(object) = params.as_object_mut() {
            object.remove(REQ_HOST_PARAM);
            object.remove(REQ_ORIGIN_PARAM);
            object.remove("session_token");
        }
        serde_json::from_value(params).map_err(|error| {
            RPCErrors::ParseRequestError(format!("invalid {} params: {error}", req.method))
        })
    }

    /// The browser's address: the peer itself, or behind the node gateway (a loopback
    /// peer) the hop the gateway appended last to `X-Forwarded-For`. `X-Real-IP` is
    /// not used: the gateway keeps one a client sent. Loopback means "unknown".
    fn forwarded_client_ip(headers: &http::HeaderMap, peer: IpAddr) -> IpAddr {
        if !peer.is_loopback() {
            return peer;
        }
        headers
            .get_all("x-forwarded-for")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .last()
            .and_then(|value| value.rsplit(',').next())
            .and_then(|hop| hop.trim().parse::<IpAddr>().ok())
            .unwrap_or(peer)
    }

    /// A failed call as the browser receives it: a structured error (D-4) keeps its
    /// `code` / `data` next to the text.
    fn failed_rpc_response(
        error: RPCErrors,
        seq: u64,
        trace_id: Option<String>,
    ) -> (RPCResponse, Option<RPCErrorDetail>) {
        let detail = error.detail().cloned();
        let mut response = RPCResponse::new(RPCResult::Failed(error.to_string()), seq);
        response.trace_id = trace_id;
        (response, detail)
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
        let req_origin = get_buckyos_api_runtime().ok().and_then(|runtime| {
            Self::request_origin(&req, !runtime.force_https, runtime.node_gateway_port).ok()
        });
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
        let client_ip = Self::forwarded_client_ip(req.headers(), client_ip);

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

        if let Some(params) = rpc_request.params.as_object_mut() {
            if let Some(req_host) = req_host {
                params.insert(REQ_HOST_PARAM.to_string(), Value::String(req_host));
            }
            match req_origin {
                Some(origin) => {
                    params.insert(REQ_ORIGIN_PARAM.to_string(), Value::String(origin));
                }
                None => {
                    params.remove(REQ_ORIGIN_PARAM);
                }
            }
        }

        let req_seq = rpc_request.seq;
        let req_trace_id = rpc_request.trace_id.clone();
        let carries_credentials = rpc_request.method.starts_with("auth.");
        let (response, detail) = match self.handle_rpc_call(rpc_request, client_ip).await {
            Ok(response) => (response, None),
            Err(error) => Self::failed_rpc_response(error, req_seq, req_trace_id),
        };
        let wire = response.to_wire_value(detail.as_ref()).map_err(|error| {
            server_err!(
                ServerErrorCode::EncodeError,
                "Failed to encode rpc response: {}",
                error
            )
        })?;

        let body_json = serde_json::to_vec(&wire).map_err(|error| {
            server_err!(
                ServerErrorCode::EncodeError,
                "Failed to encode rpc response: {}",
                error
            )
        })?;

        let mut response = http::Response::builder().header(CONTENT_TYPE, "application/json");
        if carries_credentials {
            response = response.header(CACHE_CONTROL, "no-store");
        }
        Ok(response
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
        ip_from: IpAddr,
    ) -> Result<RPCResponse, RPCErrors> {
        if req.token.is_none() {
            req.token = Self::extract_rpc_session_token(&req);
        }

        let principal = match self.authenticate_rpc_request(&req).await {
            Ok(principal) => principal,
            Err(RPCErrors::InvalidToken(reason))
                if req.method == sys_auth_backend::PREPARE_APP_LAUNCH_METHOD =>
            {
                info!("prepare_app_launch source session rejected: {}", reason);
                return Ok(Self::app_launch_source_inactive_response(req.seq));
            }
            Err(error) => return Err(error),
        };
        if let Some(principal) = principal.as_ref() {
            principal.require_agent_permission(&req.method)?;
            Self::require_restricted_self_target(principal, &req)?;
            log::debug!(
                "control-panel rpc auth: method={}, user={}, type={:?}, restricted={}, auth_method={:?}",
                req.method,
                principal.username,
                principal.user_type,
                principal.is_restricted(),
                principal.session.auth_method
            );
        }

        let result = self
            .dispatch_rpc_call(req, ip_from, principal.as_ref())?
            .await;
        result
    }
}

impl ControlPanelServer {
    fn dispatch_rpc_call<'a>(
        &'a self,
        req: RPCRequest,
        ip_from: IpAddr,
        principal: Option<&'a RpcAuthPrincipal>,
    ) -> Result<futures::future::BoxFuture<'a, Result<RPCResponse, RPCErrors>>, RPCErrors> {
        Ok(match req.method.as_str() {
            method if method.starts_with("executor.") => Box::pin(self.handle_agent_daemon_rpc(req, principal)),
            // UI bootstrap
            "ui.locale.get" => Box::pin(self.handle_ui_locale_get(req)),
            "ui.locale.set" => Box::pin(self.handle_ui_locale_set(req)),
            // Auth
            "auth.login" => Box::pin(self.handle_auth_login(req)),
            "auth.describe_target" => Box::pin(self.handle_auth_describe_target(req)),
            sys_auth_backend::PREPARE_APP_LAUNCH_METHOD => {
                Box::pin(self.handle_auth_prepare_app_launch(req, principal))
            }
            "auth.current_identity" => Box::pin(self.handle_auth_current_identity(req, principal)),
            "auth.sso_config" => Box::pin(self.handle_auth_sso_config(req)),
            "auth.entry.describe" => Box::pin(self.handle_auth_entry_describe(req)),
            "auth.register" => Box::pin(self.handle_auth_register(req, ip_from)),
            "auth.invite.describe" => Box::pin(self.handle_auth_invite_describe(req)),
            "auth.invite.login" => Box::pin(self.handle_auth_invite_login(req)),
            "auth.invite.request_resend" => Box::pin(self.handle_auth_invite_request_resend(req)),
            "auth.app_login.exchange" => {
                Box::pin(self.handle_auth_app_login_exchange(req, ip_from))
            }
            "auth.invite.set_password" => {
                Box::pin(self.handle_auth_invite_set_password(req, principal))
            }
            "auth.did.begin" => Box::pin(self.handle_auth_did_begin(req)),
            "auth.federation.describe" => {
                Box::pin(self.handle_auth_federation_describe(req, principal))
            }
            "auth.federation.approve" => {
                Box::pin(self.handle_auth_federation_approve(req, principal))
            }
            "auth.federation.deny" => Box::pin(self.handle_auth_federation_deny(req, principal)),
            "sso.settings.get" => Box::pin(self.handle_sso_settings_get(req, principal)),
            "sso.settings.set" => Box::pin(self.handle_sso_settings_set(req, principal)),

            // User Mgr
            "user.list" => Box::pin(self.handle_user_list(req, principal)),
            "user.get" => Box::pin(self.handle_user_get(req, principal)),
            "user.create" => Box::pin(self.handle_user_create(req, principal)),
            "user.update" => Box::pin(self.handle_user_update(req, principal)),
            "user.update_contact" => Box::pin(self.handle_user_update_contact(req, principal)),
            "user.profile.get" => Box::pin(self.handle_user_profile_get(req, principal)),
            "user.profile.set" => Box::pin(self.handle_user_profile_set(req, principal)),
            "user.set_msg_tunnel" => Box::pin(self.handle_user_set_msg_tunnel(req, principal)),
            "user.remove_msg_tunnel" => {
                Box::pin(self.handle_user_remove_msg_tunnel(req, principal))
            }
            "user.invite.create" => Box::pin(self.handle_user_invite_create(req, principal)),
            "user.invite.get" => Box::pin(self.handle_user_invite_get(req, principal)),
            "user.invite.accept" => Box::pin(self.handle_user_invite_accept(req, principal)),
            "user.delete" => Box::pin(self.handle_user_delete(req, principal)),
            "user.exclusive_invite.create" => {
                Box::pin(self.handle_exclusive_invite_create(req, principal))
            }
            "user.exclusive_invite.list" => {
                Box::pin(self.handle_exclusive_invite_list(req, principal))
            }
            "user.exclusive_invite.get" => {
                Box::pin(self.handle_exclusive_invite_get(req, principal))
            }
            "user.exclusive_invite.revoke" => {
                Box::pin(self.handle_exclusive_invite_revoke(req, principal))
            }
            "user.exclusive_invite.rotate" => {
                Box::pin(self.handle_exclusive_invite_rotate(req, principal))
            }
            "user.exclusive_invite.retry_resource" => {
                Box::pin(self.handle_exclusive_invite_retry_resource(req, principal))
            }
            "user.app_login.issue" => Box::pin(self.handle_app_login_issue(req, principal)),
            "apps.app_login.get" => Box::pin(self.handle_app_login_get(req, principal)),
            "apps.app_login.set" => Box::pin(self.handle_app_login_set(req, principal)),

            "user.change_password" => Box::pin(self.handle_user_change_password(req, principal)),
            "user.change_state" => Box::pin(self.handle_user_change_state(req, principal)),
            "user.change_type" => Box::pin(self.handle_user_change_type(req, principal)),
            "user.set_account_options" => {
                Box::pin(self.handle_user_set_account_options(req, principal))
            }
            "user_group.list" => Box::pin(self.handle_user_group_list(req, principal)),
            "user_group.create" => Box::pin(self.handle_user_group_create(req, principal)),
            "user_group.update" => Box::pin(self.handle_user_group_update(req, principal)),
            "user_group.delete" => Box::pin(self.handle_user_group_delete(req, principal)),
            "user_group.set_members" => {
                Box::pin(self.handle_user_group_set_members(req, principal))
            }
            "user_group.set_projections" => {
                Box::pin(self.handle_user_group_set_projections(req, principal))
            }
            "user_group.reset_projections" => {
                Box::pin(self.handle_user_group_reset_projections(req, principal))
            }

            "agent.check_name" => Box::pin(self.handle_agent_check_name(req, principal)),
            "agent.list_templates" => Box::pin(self.handle_agent_list_templates(req, principal)),
            "agent.create" => Box::pin(self.handle_agent_create(req, principal)),
            "agent.create.status" => Box::pin(self.handle_agent_create_status(req, principal)),
            "agent.create.retry" => Box::pin(self.handle_agent_create_retry(req, principal)),
            "agent.create.cancel" => Box::pin(self.handle_agent_create_cancel(req, principal)),
            "agent.list" => Box::pin(self.handle_agent_list(req, principal)),
            "agent.get" => Box::pin(self.handle_agent_get(req, principal)),
            "agent.update" => Box::pin(self.handle_agent_update(req, principal)),
            "agent.delete" => Box::pin(self.handle_agent_delete(req, principal)),
            "agent.profile.get" => Box::pin(self.handle_agent_profile_get(req, principal)),
            "agent.runtime_directory" => Box::pin(self.handle_agent_runtime_directory(req, principal)),
            "agent.profile.set" => Box::pin(self.handle_agent_profile_set(req, principal)),
            "agent.set_msg_tunnel" => Box::pin(self.handle_agent_set_msg_tunnel(req, principal)),
            "agent.remove_msg_tunnel" => {
                Box::pin(self.handle_agent_remove_msg_tunnel(req, principal))
            }
            // System dashboard
            "ui.dashboard" => Box::pin(self.handle_dashboard(req)),
            "system.overview" => Box::pin(self.handle_system_overview(req)),
            "system.buckyos_info.get" => Box::pin(self.handle_buckyos_info_get(req)),
            "system.product.get" => Box::pin(self.handle_system_product_get(req)),
            "system.dev_mode.get" => Box::pin(self.handle_dev_mode_get(req)),
            "system.dev_mode.set" => Box::pin(self.handle_dev_mode_set(req, principal)),
            "system.status" => Box::pin(self.handle_system_status(req)),
            "system.metrics" => Box::pin(self.handle_system_metrics(req)),
            "network.overview" => Box::pin(self.handle_network_overview(req)),

            //SystemLogs
            "system.logs.list" => {
                Self::require_rpc_principal(principal)?;
                Box::pin(self.handle_system_logs_list(req))
            }
            "system.logs.query" => {
                Self::require_rpc_principal(principal)?;
                Box::pin(self.handle_system_logs_query(req))
            }
            "system.logs.tail" => {
                Self::require_rpc_principal(principal)?;
                Box::pin(self.handle_system_logs_tail(req))
            }
            "system.logs.download" => {
                Self::require_rpc_principal(principal)?;
                Box::pin(self.handle_system_logs_download(req))
            }
            "task.retry" => Box::pin(self.handle_task_retry(req, principal)),
            "diagnostic.collect" => Box::pin(self.handle_diagnostic_collect(req, principal)),
            "diagnostic.export" => Box::pin(self.handle_diagnostic_export(req, principal)),
            "download.create" => Box::pin(self.handle_download_create(req, principal)),
            "system.update.check" => Box::pin(self.handle_unimplemented(req, "Check updates")),
            "system.update.apply" => Box::pin(self.handle_unimplemented(req, "Apply update")),

            //AppMgr
            "apps.list" => Box::pin(self.handle_apps_list(req, principal)),
            "apps.details" => Box::pin(self.handle_app_detials(req, principal)),
            "apps.status" => Box::pin(self.handle_apps_status(req, principal)),
            "apps.availability.get" => Box::pin(self.handle_app_availability_get(req, principal)),
            "apps.availability.set" => Box::pin(self.handle_app_availability_set(req, principal)),
            "apps.availability.check" => {
                Box::pin(self.handle_app_availability_check(req, principal))
            }
            //"apps.version.list" => Box::pin(self.handle_apps_version_list(req)),
            //AppInstaller
            "apps.staging.finalize" => Box::pin(self.handle_apps_staging_finalize(req, principal)),
            "apps.staging.status" => Box::pin(self.handle_apps_staging_status(req, principal)),
            "apps.staging.release" => Box::pin(self.handle_apps_staging_release(req, principal)),
            "apps.inspect" => Box::pin(self.handle_apps_inspect(req, principal)),
            "apps.plan.recompute" => Box::pin(self.handle_apps_plan_recompute(req, principal)),
            "apps.submit" => Box::pin(self.handle_apps_submit(req, principal)),
            "apps.install.confirm" => Box::pin(self.handle_apps_install_confirm(req, principal)),
            "apps.install.retry" => Box::pin(self.handle_apps_install_retry(req, principal)),
            "apps.install.cancel" => Box::pin(self.handle_apps_install_cancel(req, principal)),
            "apps.install.status" => Box::pin(self.handle_apps_install_status(req, principal)),
            "apps.upgrade.check" => Box::pin(self.handle_apps_update_availability(req, principal)),
            "apps.upgrade" => Box::pin(self.handle_apps_upgrade_batch(req, principal)),
            "apps.uninstall" => Box::pin(self.handle_apps_uninstall(req, principal)),
            "apps.start" => Box::pin(self.handle_apps_start(req, principal)),
            "apps.stop" => Box::pin(self.handle_apps_stop(req, principal)),
            "apps.restart" => Box::pin(self.handle_apps_restart(req, principal)),
            "app.publish" => Box::pin(self.handle_app_publish(req, principal)),

            //ZoneMgr
            "zone.overview" => Box::pin(self.handle_zone_overview(req)),
            "zone.devices.list" => Box::pin(self.handle_zone_devices_list(req)),
            "zone.certificates.list" => Box::pin(self.handle_zone_certificates_list(req)),
            "zone.presentation.get" => Box::pin(self.handle_zone_presentation_get(req)),
            "zone.presentation.set" => Box::pin(self.handle_zone_presentation_set(req, principal)),
            "gateway.overview" => Box::pin(self.handle_gateway_overview(req)),
            "gateway.file.get" => Box::pin(self.handle_gateway_file_get(req)),
            "container.overview" => Box::pin(self.handle_container_overview(req)),
            "container.action" => Box::pin(self.handle_container_action(req)),

            _ => return Err(RPCErrors::UnknownMethod(req.method)),
        })
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

        if method == Method::GET && path.starts_with("/kapi/control-panel/executor/") {
            return self.serve_agent_daemon_ws(req).await;
        }

        if method == Method::GET && path == "/sso_callback" {
            return self.serve_sso_callback(req, info).await;
        }
        if method == Method::GET && path == federation::DID_CALLBACK_PATH {
            return self.serve_did_callback(req).await;
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
            if let Some(bundle_id) = path.strip_prefix("/kapi/control-panel/diagnostics/download/")
            {
                if !bundle_id.is_empty() {
                    return self.handle_diagnostic_download_http(bundle_id).await;
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
        .renew_token_from_verify_hub()
        .await
        .map_err(|error| {
            log::error!("exchange control-panel login assertion failed: {:?}", error);
            anyhow::anyhow!("exchange control-panel login assertion failed: {:?}", error)
        })?;
    runtime
        .set_main_service_port(CONTROL_PANEL_SERVICE_PORT)
        .await;
    set_buckyos_api_runtime(runtime)
        .map_err(|err| anyhow::anyhow!("register control-panel runtime failed: {}", err))?;

    let control_panel_server = ControlPanelServer::new();
    // Every product rule below (install admission, the Agent run slot,
    // `system.product.get`) reads `system/product`: publish it first.
    product::publish_installed_product().await.map_err(|error| {
        log::error!("publish product profile failed: {:?}", error);
        error
    })?;
    // Agent 运行时 App 的本地 AppDoc 授权只在内存中，恢复安装任务前先重新注册。
    agent_mgr::register_agent_runtime_authorities().await;
    // 启动安装任务恢复循环；正常路径由业务 RPC 直接执行，TaskManager
    // 启动扫描/低频 sweep 恢复重启前或异常遗漏的非终态安装事务。
    control_panel_server.install_runner.start();
    control_panel_server.app_installer.start_lifecycle_runner();
    control_panel_server.start_update_batch_runner();
    control_panel_server.start_download_recovery();

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
    let _ = runner.add_http_server(
        federation::DID_CALLBACK_PATH.to_string(),
        control_panel_server.clone(),
    );
    // Control-panel desktop API endpoints.
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
    pre_install_reconciler::PreInstallReconciler::new(control_panel_server.clone()).start();
    control_panel_server.start_agent_driver();
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

#[cfg(test)]
mod rpc_stack_tests {
    use super::*;

    #[test]
    fn public_login_and_invite_rejection_fit_small_worker_stack() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_stack_size(1024 * 1024)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            tokio::spawn(async {
                let server = ControlPanelServer::new();
                let ip = "127.0.0.1".parse().unwrap();
                let login = server
                    .handle_rpc_call(RPCRequest::new("auth.login", json!({})), ip)
                    .await;
                assert!(matches!(login, Err(RPCErrors::ParseRequestError(_))));
                let accept = server
                    .handle_rpc_call(RPCRequest::new("user.invite.accept", json!({})), ip)
                    .await;
                assert!(matches!(accept, Err(RPCErrors::InvalidToken(_))));
            })
            .await
            .unwrap();
        });
    }
}
