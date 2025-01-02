use crate::database_manager::DB_MANAGER;
use crate::task::{Task, TaskStatus};
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
        let task = Task {
            id: "123".to_string(),
            name: "task1".to_string(),
            status: TaskStatus::Running,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let db_manager = DB_MANAGER.lock().await;
        let result = db_manager.create_task(&task).await;
        if let Err(e) = result {
            let error_message = e.to_string();
            return Ok(RPCResponse::new(
                RPCResult::Success(json!({"code":"1", "msg": error_message})),
                req.seq,
            ));
        }

        return Ok(RPCResponse::new(RPCResult::Success(json!({})), req.seq));
    }

    async fn handle_list_task(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let db_manager = DB_MANAGER.lock().await;
        let result = db_manager.list_tasks().await;
        if let Err(e) = result {
            let error_message = e.to_string();
            return Ok(RPCResponse::new(
                RPCResult::Success(json!({"code":"1", "msg": error_message})),
                req.seq,
            ));
        }
        let tasks = result.unwrap();
        info!("len {}", tasks.len());

        let result = serde_json::to_string(&tasks).unwrap();
        return Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "code": "0",
                "data": result,
            })),
            req.seq,
        ));
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
            "create_task" => self.handle_create_task(req).await,
            "list_task" => self.handle_list_task(req).await,
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
