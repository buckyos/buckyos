use ::kRPC::*;
use anyhow::Result;
use async_trait::async_trait;
use buckyos_kit::*;
use cyfs_gateway_lib::WarpServerConfig;
use cyfs_warp::*;
use log::*;
// use name_client::*;
use serde_json::*;
use std::net::IpAddr;

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
            "resourceTimeline": [
                { "time": "00:00", "cpu": 52, "memory": 68 },
                { "time": "00:05", "cpu": 62, "memory": 70 },
                { "time": "00:10", "cpu": 58, "memory": 72 },
                { "time": "00:15", "cpu": 54, "memory": 74 },
                { "time": "00:20", "cpu": 57, "memory": 75 },
                { "time": "00:25", "cpu": 60, "memory": 76 }
            ],
            "storageSlices": [
                { "label": "Apps", "value": 28, "color": "#1d4ed8" },
                { "label": "System", "value": 22, "color": "#6b7280" },
                { "label": "Photos", "value": 18, "color": "#22c55e" },
                { "label": "Documents", "value": 12, "color": "#facc15" },
                { "label": "Other", "value": 20, "color": "#38bdf8" }
            ],
            "storageCapacityGb": 4000,
            "storageUsedGb": 2400
        });

        Ok(RPCResponse::new(RPCResult::Success(dashboard), req.id))
    }
}

#[async_trait]
impl InnerServiceHandler for ControlPanelServer {
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

    async fn handle_http_get(&self, req_path: &str, _ip_from: IpAddr) -> Result<String, RPCErrors> {
        return Err(RPCErrors::UnknownMethod(req_path.to_string()));
    }
}

async fn service_main() {
    init_logging("control_server", true);

    let control_server = ControlPanelServer::new();
    // .map_err(|e| {
    //     error!("control_server init error! err:{}", e);
    //     anyhow::anyhow!("control_server init error! err:{}", e)
    // })?;

    // control_server.init().await?;
    info!("control_server init check OK.");

    register_inner_service_builder("control_server", move || Box::new(control_server.clone()))
        .await;

    let service_config = json!({
      "http_port":3180,
      "tls_port":0,
      "hosts": {
        "*": {
          "enable_cors":true,
          "routes": {
            "/kapi/control-panel" : {
                "inner_service":"control_server"
            }
          }
        }
      }
    });

    let service_config: WarpServerConfig = serde_json::from_value(service_config).unwrap();
    let _ = start_cyfs_warp_server(service_config).await;
    let _ = tokio::signal::ctrl_c().await;
}

fn main() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(service_main());
}
