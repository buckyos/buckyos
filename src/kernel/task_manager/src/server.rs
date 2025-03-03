use crate::task_db::DB_MANAGER;
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
        info!("params : {}", req.params);
        let params: Value = match req.params {
            Value::String(s) => serde_json::from_str(&s).map_err(|e| {
                error!("Failed to parse params: {}", e);
                RPCErrors::ReasonError(e.to_string())
            })?,
            Value::Object(_) => req.params,
            _ => {
                error!("Invalid params type");
                return self.error(req.seq, "Invalid params type".to_string());
            }
        };

        let name = match params.get("name") {
            Some(Value::String(s)) => s,
            Some(_) => {
                return self.error(req.seq, "'name' field is not a string".to_string());
            }
            None => {
                return self.error(req.seq, "Missing 'name' field in params".to_string());
            }
        };
        let title = match params.get("title") {
            Some(Value::String(s)) => s,
            Some(_) => {
                return self.error(req.seq, "'title' field is not a string".to_string());
            }
            None => {
                return self.error(req.seq, "Missing 'title' field in params".to_string());
            }
        };

        let app_name = match params.get("app_name") {
            Some(Value::String(s)) => s,
            Some(_) => {
                return self.error(req.seq, "'app_name' field is not a string".to_string());
            }
            None => {
                return self.error(req.seq, "Missing 'app_name' field in params".to_string());
            }
        };
        let task_type = match params.get("task_type") {
            Some(Value::String(s)) => s,
            Some(_) => {
                return self.error(req.seq, "'task_type' field is not a string".to_string());
            }
            None => {
                return self.error(req.seq, "Missing 'task_type' field in params".to_string());
            }
        };

        // 可选的数据字段
        let data = match params.get("data") {
            Some(Value::String(s)) => Some(s.to_string()),
            Some(Value::Object(o)) => Some(serde_json::to_string(o).unwrap()),
            Some(_) => None,
            None => None,
        };

        let task = Task::new(
            name.to_string(),
            title.to_string(),
            task_type.to_string(),
            app_name.to_string(),
            data,
        );

        let db_manager = DB_MANAGER.lock().await;
        let result = db_manager.create_task(&task).await;
        match result {
            Ok(task_id) => {
                return Ok(RPCResponse::new(
                    RPCResult::Success(json!({
                        "code": "0",
                        "task_id": task_id
                    })),
                    req.seq,
                ));
            }
            Err(e) => {
                let error_message = e.to_string();
                return self.error(req.seq, error_message);
            }
        }
    }

    async fn handle_get_task(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let params: Value = match req.params {
            Value::String(s) => serde_json::from_str(&s).map_err(|e| {
                error!("Failed to parse params: {}", e);
                RPCErrors::ReasonError(e.to_string())
            })?,
            Value::Object(_) => req.params,
            _ => {
                error!("Invalid params type");
                return self.error(req.seq, "Invalid params type".to_string());
            }
        };

        let id = match params.get("id") {
            Some(Value::Number(n)) => n.as_i64().unwrap() as i32,
            Some(_) => {
                return self.error(req.seq, "'id' field is not a number".to_string());
            }
            None => {
                return self.error(req.seq, "Missing 'id' field in params".to_string());
            }
        };

        let db_manager = DB_MANAGER.lock().await;
        let result = db_manager.get_task(id).await;
        match result {
            Ok(Some(task)) => {
                let task_json = serde_json::to_value(&task).unwrap();
                return Ok(RPCResponse::new(
                    RPCResult::Success(json!({
                        "code": "0",
                        "task": task_json
                    })),
                    req.seq,
                ));
            }
            Ok(None) => {
                return self.error(req.seq, format!("Task with id {} not found", id));
            }
            Err(e) => {
                let error_message = e.to_string();
                return self.error(req.seq, error_message);
            }
        }
    }

    async fn handle_list_tasks(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let params: Value = match req.params {
            Value::String(s) => serde_json::from_str(&s).map_err(|e| {
                error!("Failed to parse params: {}", e);
                RPCErrors::ReasonError(e.to_string())
            })?,
            Value::Object(_) => req.params,
            _ => {
                error!("Invalid params type");
                return self.error(req.seq, "Invalid params type".to_string());
            }
        };

        let db_manager = DB_MANAGER.lock().await;
        
        // 根据不同的过滤条件查询任务
        let tasks = if let Some(Value::String(app_name)) = params.get("app_name") {
            db_manager.list_tasks_by_app(app_name).await
        } else if let Some(Value::String(task_type)) = params.get("task_type") {
            db_manager.list_tasks_by_type(task_type).await
        } else if let Some(Value::String(status_str)) = params.get("status") {
            match TaskStatus::from_str(status_str) {
                Ok(status) => db_manager.list_tasks_by_status(status).await,
                Err(_) => {
                    return self.error(req.seq, format!("Invalid status: {}", status_str));
                }
            }
        } else {
            db_manager.list_tasks().await
        };

        match tasks {
            Ok(tasks) => {
                let tasks_json = serde_json::to_value(&tasks).unwrap();
                return Ok(RPCResponse::new(
                    RPCResult::Success(json!({
                        "code": "0",
                        "tasks": tasks_json
                    })),
                    req.seq,
                ));
            }
            Err(e) => {
                let error_message = e.to_string();
                return self.error(req.seq, error_message);
            }
        }
    }

    async fn handle_update_task_status(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let params: Value = match req.params {
            Value::String(s) => serde_json::from_str(&s).map_err(|e| {
                error!("Failed to parse params: {}", e);
                RPCErrors::ReasonError(e.to_string())
            })?,
            Value::Object(_) => req.params,
            _ => {
                error!("Invalid params type");
                return self.error(req.seq, "Invalid params type".to_string());
            }
        };

        let id = match params.get("id") {
            Some(Value::Number(n)) => n.as_i64().unwrap() as i32,
            Some(_) => {
                return self.error(req.seq, "'id' field is not a number".to_string());
            }
            None => {
                return self.error(req.seq, "Missing 'id' field in params".to_string());
            }
        };

        let status_str = match params.get("status") {
            Some(Value::String(s)) => s,
            Some(_) => {
                return self.error(req.seq, "'status' field is not a string".to_string());
            }
            None => {
                return self.error(req.seq, "Missing 'status' field in params".to_string());
            }
        };

        let status = match TaskStatus::from_str(status_str) {
            Ok(status) => status,
            Err(_) => {
                return self.error(req.seq, format!("Invalid status: {}", status_str));
            }
        };

        let db_manager = DB_MANAGER.lock().await;
        let result = db_manager.update_task_status(id, status).await;
        match result {
            Ok(_) => {
                return Ok(RPCResponse::new(
                    RPCResult::Success(json!({
                        "code": "0",
                        "message": "Task status updated successfully"
                    })),
                    req.seq,
                ));
            }
            Err(e) => {
                let error_message = e.to_string();
                return self.error(req.seq, error_message);
            }
        }
    }

    async fn handle_update_task_progress(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let params: Value = match req.params {
            Value::String(s) => serde_json::from_str(&s).map_err(|e| {
                error!("Failed to parse params: {}", e);
                RPCErrors::ReasonError(e.to_string())
            })?,
            Value::Object(_) => req.params,
            _ => {
                error!("Invalid params type");
                return self.error(req.seq, "Invalid params type".to_string());
            }
        };

        let id = match params.get("id") {
            Some(Value::Number(n)) => n.as_i64().unwrap() as i32,
            Some(_) => {
                return self.error(req.seq, "'id' field is not a number".to_string());
            }
            None => {
                return self.error(req.seq, "Missing 'id' field in params".to_string());
            }
        };

        let completed_items = match params.get("completed_items") {
            Some(Value::Number(n)) => n.as_i64().unwrap() as i32,
            Some(_) => {
                return self.error(req.seq, "'completed_items' field is not a number".to_string());
            }
            None => {
                return self.error(req.seq, "Missing 'completed_items' field in params".to_string());
            }
        };

        let total_items = match params.get("total_items") {
            Some(Value::Number(n)) => n.as_i64().unwrap() as i32,
            Some(_) => {
                return self.error(req.seq, "'total_items' field is not a number".to_string());
            }
            None => {
                return self.error(req.seq, "Missing 'total_items' field in params".to_string());
            }
        };

        let progress = if total_items > 0 {
            (completed_items as f32 / total_items as f32) * 100.0
        } else {
            0.0
        };

        let db_manager = DB_MANAGER.lock().await;
        let result = db_manager.update_task_progress(id, progress, completed_items, total_items).await;
        match result {
            Ok(_) => {
                return Ok(RPCResponse::new(
                    RPCResult::Success(json!({
                        "code": "0",
                        "message": "Task progress updated successfully"
                    })),
                    req.seq,
                ));
            }
            Err(e) => {
                let error_message = e.to_string();
                return self.error(req.seq, error_message);
            }
        }
    }

    async fn handle_update_task_error(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let params: Value = match req.params {
            Value::String(s) => serde_json::from_str(&s).map_err(|e| {
                error!("Failed to parse params: {}", e);
                RPCErrors::ReasonError(e.to_string())
            })?,
            Value::Object(_) => req.params,
            _ => {
                error!("Invalid params type");
                return self.error(req.seq, "Invalid params type".to_string());
            }
        };

        let id = match params.get("id") {
            Some(Value::Number(n)) => n.as_i64().unwrap() as i32,
            Some(_) => {
                return self.error(req.seq, "'id' field is not a number".to_string());
            }
            None => {
                return self.error(req.seq, "Missing 'id' field in params".to_string());
            }
        };

        let error_message = match params.get("error_message") {
            Some(Value::String(s)) => s,
            Some(_) => {
                return self.error(req.seq, "'error_message' field is not a string".to_string());
            }
            None => {
                return self.error(req.seq, "Missing 'error_message' field in params".to_string());
            }
        };

        let db_manager = DB_MANAGER.lock().await;
        let result = db_manager.update_task_error(id, error_message).await;
        match result {
            Ok(_) => {
                return Ok(RPCResponse::new(
                    RPCResult::Success(json!({
                        "code": "0",
                        "message": "Task error updated successfully"
                    })),
                    req.seq,
                ));
            }
            Err(e) => {
                let error_message = e.to_string();
                return self.error(req.seq, error_message);
            }
        }
    }

    async fn handle_update_task_data(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let params: Value = match req.params {
            Value::String(s) => serde_json::from_str(&s).map_err(|e| {
                error!("Failed to parse params: {}", e);
                RPCErrors::ReasonError(e.to_string())
            })?,
            Value::Object(_) => req.params,
            _ => {
                error!("Invalid params type");
                return self.error(req.seq, "Invalid params type".to_string());
            }
        };

        let id = match params.get("id") {
            Some(Value::Number(n)) => n.as_i64().unwrap() as i32,
            Some(_) => {
                return self.error(req.seq, "'id' field is not a number".to_string());
            }
            None => {
                return self.error(req.seq, "Missing 'id' field in params".to_string());
            }
        };

        let data = match params.get("data") {
            Some(Value::String(s)) => s.to_string(),
            Some(Value::Object(o)) => serde_json::to_string(o).unwrap(),
            Some(_) => {
                return self.error(req.seq, "'data' field is not a string or object".to_string());
            }
            None => {
                return self.error(req.seq, "Missing 'data' field in params".to_string());
            }
        };

        let db_manager = DB_MANAGER.lock().await;
        let result = db_manager.update_task_data(id, &data).await;
        match result {
            Ok(_) => {
                return Ok(RPCResponse::new(
                    RPCResult::Success(json!({
                        "code": "0",
                        "message": "Task data updated successfully"
                    })),
                    req.seq,
                ));
            }
            Err(e) => {
                let error_message = e.to_string();
                return self.error(req.seq, error_message);
            }
        }
    }

    async fn handle_delete_task(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let params: Value = match req.params {
            Value::String(s) => serde_json::from_str(&s).map_err(|e| {
                error!("Failed to parse params: {}", e);
                RPCErrors::ReasonError(e.to_string())
            })?,
            Value::Object(_) => req.params,
            _ => {
                error!("Invalid params type");
                return self.error(req.seq, "Invalid params type".to_string());
            }
        };

        let id = match params.get("id") {
            Some(Value::Number(n)) => n.as_i64().unwrap() as i32,
            Some(_) => {
                return self.error(req.seq, "'id' field is not a number".to_string());
            }
            None => {
                return self.error(req.seq, "Missing 'id' field in params".to_string());
            }
        };

        let db_manager = DB_MANAGER.lock().await;
        let result = db_manager.delete_task(id).await;
        match result {
            Ok(_) => {
                return Ok(RPCResponse::new(
                    RPCResult::Success(json!({
                        "code": "0",
                        "message": "Task deleted successfully"
                    })),
                    req.seq,
                ));
            }
            Err(e) => {
                let error_message = e.to_string();
                return self.error(req.seq, error_message);
            }
        }
    }

    fn error(&self, seq: u64, error_message: String) -> Result<RPCResponse, RPCErrors> {
        return Ok(RPCResponse::new(
            RPCResult::Success(json!({"code":"1", "msg": error_message})),
            seq,
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
            "get_task" => self.handle_get_task(req).await,
            "list_tasks" => self.handle_list_tasks(req).await,
            "update_task_status" => self.handle_update_task_status(req).await,
            "update_task_progress" => self.handle_update_task_progress(req).await,
            "update_task_error" => self.handle_update_task_error(req).await,
            "update_task_data" => self.handle_update_task_data(req).await,
            "delete_task" => self.handle_delete_task(req).await,
            _ => Err(RPCErrors::UnknownMethod(req.method)),
        }
    }
}

pub async fn start_task_manager_service() {
    let server = TaskManagerServer::new();
    register_inner_service_builder("task_manager", move || Box::new(server.clone())).await;
    let _ = get_buckyos_system_bin_dir().join("task_manager");

    let active_server_config = json!({
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
