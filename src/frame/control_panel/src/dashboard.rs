use crate::{
    bytes_to_gb, ControlPanelServer, MetricsTimelinePoint, SystemMetricsSnapshot,
    METRICS_DISK_REFRESH_INTERVAL_SECS, NETWORK_TIMELINE_LIMIT,
};
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult};
use buckyos_api::get_buckyos_api_runtime;
use chrono::Utc;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};
use sysinfo::{DiskRefreshKind, Disks, Networks, System};
use tokio::sync::RwLock;

impl ControlPanelServer {
    pub(crate) fn start_metrics_sampler(metrics_snapshot: Arc<RwLock<SystemMetricsSnapshot>>) {
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
                let refresh_disks =
                    disk_refresh_counter.is_multiple_of(METRICS_DISK_REFRESH_INTERVAL_SECS);
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

    pub(crate) async fn handle_dashboard(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let mut system = System::new_all();
        system.refresh_memory();
        system.refresh_cpu_usage();
        // Wait a moment so CPU usage has a meaningful delta before the second refresh.
        tokio::time::sleep(Duration::from_millis(200)).await;
        system.refresh_cpu_usage();

        let cpu_usage = system.global_cpu_usage() as f64;
        let cpu_brand = system
            .cpus()
            .first()
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

    pub(crate) async fn handle_system_overview(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let mut system = System::new_all();
        system.refresh_all();

        let cpu_brand = system
            .cpus()
            .first()
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

    pub(crate) async fn handle_system_status(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
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

    pub(crate) async fn handle_system_metrics(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
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
}
