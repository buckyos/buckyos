use ::kRPC::*;
use async_trait::async_trait;
use buckyos_kit::*;
use cyfs_gateway_lib::*;
use cyfs_warp::*;
use log::*;
use serde_json::{json, Value};
use std::net::IpAddr;
use std::result::Result;

#[derive(Clone)]
struct TaskManagerServer {}

impl TaskManagerServer {
    pub fn new() -> Self {
        TaskManagerServer {}
    }

    async fn handle_create_task(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        return Ok(RPCResponse::new(RPCResult::Success(json!({})), req.seq));
    }
}

#[async_trait]
impl kRPCHandler for TaskManagerServer {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        _ip_from: IpAddr,
    ) -> Result<RPCResponse, RPCErrors> {
        match req.method.as_str() {
            "create" => self.handle_create_task(req).await,
            _ => Err(RPCErrors::UnknownMethod(req.method)),
        }
    }
}

pub async fn start_task_manager_service() {
    let server = TaskManagerServer::new();
    register_inner_service_builder("task_manager", move || Box::new(server.clone())).await;
    let _ = get_buckyos_system_bin_dir().join("task_manager");

    let active_server_config = json!({
      "tls_port":3343,
      "http_port":3380,
      "hosts": {
        "*": {
          "enable_cors":true,
          "routes": {

            "/kapi/task_manager" : {
                "inner_service":"task_manager"
            }
          }
        }
      }
    });
    let active_server_config: WarpServerConfig =
        serde_json::from_value(active_server_config).unwrap();
    info!("start node task manager service...");
    let _ = start_cyfs_warp_server(active_server_config).await;
}
