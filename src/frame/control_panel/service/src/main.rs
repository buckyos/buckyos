use ::kRPC::*;
use anyhow::Result;
use async_trait::async_trait;
use buckyos_kit::*;
use bytes::Bytes;
use cyfs_gateway_lib::*;
use http::{Method, Version};
use http_body_util::combinators::BoxBody;
use serde_json::*;
use server_runner::*;
use std::sync::Arc;
use std::{net::IpAddr, time::Duration};
use sysinfo::{DiskRefreshKind, Disks, System};

fn bytes_to_gb(bytes: u64) -> f64 {
    (bytes as f64) / 1024.0 / 1024.0 / 1024.0
}

#[derive(Clone)]
struct ControlPanelServer {}

impl ControlPanelServer {
    pub fn new() -> Self {
        ControlPanelServer {}
    }

    async fn handle_main(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "test":"test",
            })),
            req.id,
        ))
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

        Ok(RPCResponse::new(RPCResult::Success(layout), req.id))
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

        Ok(RPCResponse::new(RPCResult::Success(dashboard), req.id))
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
            "main" => self.handle_main(req).await,
            "layout" => self.handle_layout(req).await,
            "dashboard" => self.handle_dashboard(req).await,
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
        return Err(server_err!(
            ServerErrorCode::BadRequest,
            "Method not allowed"
        ));
    }

    fn id(&self) -> String {
        "active-server".to_string()
    }

    fn http_version(&self) -> Version {
        Version::HTTP_11
    }

    fn http3_port(&self) -> Option<u16> {
        None
    }
}

pub async fn start_node_active_service() {
    let control_panel_server = ControlPanelServer::new();
    const PORT: u16 = 3180;
    let runner = Runner::new(PORT);
    // 添加 RPC 服务
    let _ = runner.add_http_server(
        "/kapi/control-panel".to_string(),
        Arc::new(control_panel_server),
    );

    let _ = runner.run().await;
}

async fn service_main() {
    init_logging("control_server", true);
    let _ = start_node_active_service().await;
    let _ = tokio::signal::ctrl_c().await;
}

fn main() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(service_main());
}
