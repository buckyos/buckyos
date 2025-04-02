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
            Some(Value::Number(n)) => n.as_i64().unwrap(),
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
            Some(Value::Number(n)) => n.as_i64().unwrap(),
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
            Some(Value::Number(n)) => n.as_i64().unwrap(),
            Some(_) => {
                return self.error(req.seq, "'id' field is not a number".to_string());
            }
            None => {
                return self.error(req.seq, "Missing 'id' field in params".to_string());
            }
        };

        let completed_items = match params.get("completed_items") {
            Some(Value::Number(n)) => n.as_i64().unwrap(),
            Some(_) => {
                return self.error(req.seq, "'completed_items' field is not a number".to_string());
            }
            None => {
                return self.error(req.seq, "Missing 'completed_items' field in params".to_string());
            }
        };

        let total_items = match params.get("total_items") {
            Some(Value::Number(n)) => n.as_i64().unwrap(),
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
        let result = db_manager.update_task_progress(id, progress, completed_items as i32, total_items as i32).await;
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
            Some(Value::Number(n)) => n.as_i64().unwrap(),
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
            Some(Value::Number(n)) => n.as_i64().unwrap(),
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
            Some(Value::Number(n)) => n.as_i64().unwrap(),
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
impl InnerServiceHandler for TaskManagerServer {
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
    
    async fn handle_http_get(&self, req_path:&str,_ip_from:IpAddr) -> Result<String,RPCErrors> {
        return Err(RPCErrors::UnknownMethod(req_path.to_string()));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::{Task, TaskStatus};
    use crate::task_db::{init_db, TaskDb};
    use ::kRPC::*;
    use serde_json::json;
    use std::net::IpAddr;
    use std::str::FromStr;
    use tempfile::tempdir;
    use tokio::sync::Mutex;

    // 辅助函数：创建测试请求
    fn create_rpc_request(method: &str, params: Value) -> RPCRequest {
        RPCRequest {
            method: method.to_string(),
            params,
            seq: 1,
            token: Some("".to_string()),
            trace_id: Some("".to_string()),
        }
    }

    // 辅助函数：设置测试环境
    async fn setup_test_environment() -> (TaskManagerServer, tempfile::TempDir) {
        // 创建临时目录和数据库
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let db_path_str = db_path.to_str().unwrap();
        
        // 初始化数据库
        let mut db = TaskDb::new();
        db.connect(db_path_str).unwrap();
        db.init_db().await.unwrap();
        
        // 替换全局DB_MANAGER以便测试
        *crate::task_db::DB_MANAGER.lock().await = db;
        
        // 创建服务器实例
        let server = TaskManagerServer::new();
        
        (server, temp_dir)
    }

    #[tokio::test]
    async fn test_create_and_get_task() {
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();
        
        // 创建任务
        let create_params = json!({
            "name": "test_task",
            "title": "Test Task",
            "task_type": "test_type",
            "app_name": "test_app",
            "data": {"key": "value"}
        });
        
        let create_req = create_rpc_request("create_task", create_params);
        let create_resp = server.handle_rpc_call(create_req, ip).await.unwrap();
        
        // 验证创建成功
        if let RPCResult::Success(result) = create_resp.result {
            assert_eq!(result["code"], "0");
            let task_id = result["task_id"].as_i64().unwrap() as i32;
            assert!(task_id > 0);
            
            // 获取任务
            let get_params = json!({
                "id": task_id
            });
            
            let get_req = create_rpc_request("get_task", get_params);
            let get_resp = server.handle_rpc_call(get_req, ip).await.unwrap();
            
            // 验证获取成功
            if let RPCResult::Success(result) = get_resp.result {
                assert_eq!(result["code"], "0");
                assert_eq!(result["task"]["name"], "test_task");
                assert_eq!(result["task"]["title"], "Test Task");
                assert_eq!(result["task"]["task_type"], "test_type");
                assert_eq!(result["task"]["app_name"], "test_app");
            } else {
                panic!("Failed to get task");
            }
        } else {
            panic!("Failed to create task");
        }
    }

    #[tokio::test]
    async fn test_list_tasks() {
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();
        
        // 创建多个任务
        for i in 1..4 {
            let create_params = json!({
                "name": format!("test_task_{}", i),
                "title": format!("Test Task {}", i),
                "task_type": "test_type",
                "app_name": "test_app",
            });
            
            let create_req = create_rpc_request("create_task", create_params);
            let _ = server.handle_rpc_call(create_req, ip).await.unwrap();
        }
        
        // 列出所有任务
        let list_req = create_rpc_request("list_tasks", json!({}));
        let list_resp = server.handle_rpc_call(list_req, ip).await.unwrap();
        
        // 验证列表
        if let RPCResult::Success(result) = list_resp.result {
            assert_eq!(result["code"], "0");
            let tasks = result["tasks"].as_array().unwrap();
            assert_eq!(tasks.len(), 3);
        } else {
            panic!("Failed to list tasks");
        }
    }

    #[tokio::test]
    async fn test_list_tasks_by_app() {
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();
        
        // 创建不同app的任务
        let create_params1 = json!({
            "name": "app1_task",
            "title": "App1 Task",
            "task_type": "test_type",
            "app_name": "app1",
        });
        
        let create_params2 = json!({
            "name": "app2_task",
            "title": "App2 Task",
            "task_type": "test_type",
            "app_name": "app2",
        });
        
        let create_req1 = create_rpc_request("create_task", create_params1);
        let create_req2 = create_rpc_request("create_task", create_params2);
        let _ = server.handle_rpc_call(create_req1, ip).await.unwrap();
        let _ = server.handle_rpc_call(create_req2, ip).await.unwrap();
        
        // 按app筛选
        let list_params = json!({
            "app_name": "app1"
        });
        
        let list_req = create_rpc_request("list_tasks", list_params);
        let list_resp = server.handle_rpc_call(list_req, ip).await.unwrap();
        
        // 验证筛选结果
        if let RPCResult::Success(result) = list_resp.result {
            assert_eq!(result["code"], "0");
            let tasks = result["tasks"].as_array().unwrap();
            assert_eq!(tasks.len(), 1);
            assert_eq!(tasks[0]["app_name"], "app1");
        } else {
            panic!("Failed to list tasks by app");
        }
    }

    #[tokio::test]
    async fn test_update_task_status() {
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();
        
        // 创建任务
        let create_params = json!({
            "name": "status_test",
            "title": "Status Test",
            "task_type": "test_type",
            "app_name": "test_app",
        });
        
        let create_req = create_rpc_request("create_task", create_params);
        let create_resp = server.handle_rpc_call(create_req, ip).await.unwrap();
        
        let task_id = if let RPCResult::Success(result) = create_resp.result {
            result["task_id"].as_i64().unwrap() as i32
        } else {
            panic!("Failed to create task");
        };
        
        // 更新状态
        let update_params = json!({
            "id": task_id,
            "status": "Running"
        });
        
        let update_req = create_rpc_request("update_task_status", update_params);
        let update_resp = server.handle_rpc_call(update_req, ip).await.unwrap();
        
        // 验证更新成功
        if let RPCResult::Success(result) = update_resp.result {
            assert_eq!(result["code"], "0");
            
            // 获取任务验证状态
            let get_params = json!({
                "id": task_id
            });
            
            let get_req = create_rpc_request("get_task", get_params);
            let get_resp = server.handle_rpc_call(get_req, ip).await.unwrap();
            
            if let RPCResult::Success(result) = get_resp.result {
                assert_eq!(result["task"]["status"], "Running");
            } else {
                panic!("Failed to get task after status update");
            }
        } else {
            panic!("Failed to update task status");
        }
    }

    #[tokio::test]
    async fn test_update_task_progress() {
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();
        
        // 创建任务
        let create_params = json!({
            "name": "progress_test",
            "title": "Progress Test",
            "task_type": "test_type",
            "app_name": "test_app",
        });
        
        let create_req = create_rpc_request("create_task", create_params);
        let create_resp = server.handle_rpc_call(create_req, ip).await.unwrap();
        
        let task_id = if let RPCResult::Success(result) = create_resp.result {
            result["task_id"].as_i64().unwrap() as i32
        } else {
            panic!("Failed to create task");
        };
        
        // 更新进度
        let update_params = json!({
            "id": task_id,
            "completed_items": 5,
            "total_items": 10
        });
        
        let update_req = create_rpc_request("update_task_progress", update_params);
        let update_resp = server.handle_rpc_call(update_req, ip).await.unwrap();
        
        // 验证更新成功
        if let RPCResult::Success(result) = update_resp.result {
            assert_eq!(result["code"], "0");
            
            // 获取任务验证进度
            let get_params = json!({
                "id": task_id
            });
            
            let get_req = create_rpc_request("get_task", get_params);
            let get_resp = server.handle_rpc_call(get_req, ip).await.unwrap();
            
            if let RPCResult::Success(result) = get_resp.result {
                assert_eq!(result["task"]["completed_items"], 5);
                assert_eq!(result["task"]["total_items"], 10);
                assert_eq!(result["task"]["progress"], 50.0);
            } else {
                panic!("Failed to get task after progress update");
            }
        } else {
            panic!("Failed to update task progress");
        }
    }

    #[tokio::test]
    async fn test_update_task_error() {
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();
        
        // 创建任务
        let create_params = json!({
            "name": "error_test",
            "title": "Error Test",
            "task_type": "test_type",
            "app_name": "test_app",
        });
        
        let create_req = create_rpc_request("create_task", create_params);
        let create_resp = server.handle_rpc_call(create_req, ip).await.unwrap();
        
        let task_id = if let RPCResult::Success(result) = create_resp.result {
            result["task_id"].as_i64().unwrap() as i32
        } else {
            panic!("Failed to create task");
        };
        
        // 更新错误信息
        let update_params = json!({
            "id": task_id,
            "error_message": "Test error occurred"
        });
        
        let update_req = create_rpc_request("update_task_error", update_params);
        let update_resp = server.handle_rpc_call(update_req, ip).await.unwrap();
        
        // 验证更新成功
        if let RPCResult::Success(result) = update_resp.result {
            assert_eq!(result["code"], "0");
            
            // 获取任务验证错误信息
            let get_params = json!({
                "id": task_id
            });
            
            let get_req = create_rpc_request("get_task", get_params);
            let get_resp = server.handle_rpc_call(get_req, ip).await.unwrap();
            
            if let RPCResult::Success(result) = get_resp.result {
                assert_eq!(result["task"]["error_message"], "Test error occurred");
                assert_eq!(result["task"]["status"], "Failed");
            } else {
                panic!("Failed to get task after error update");
            }
        } else {
            panic!("Failed to update task error");
        }
    }

    #[tokio::test]
    async fn test_update_task_data() {
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();
        
        // 创建任务
        let create_params = json!({
            "name": "data_test",
            "title": "Data Test",
            "task_type": "test_type",
            "app_name": "test_app",
        });
        
        let create_req = create_rpc_request("create_task", create_params);
        let create_resp = server.handle_rpc_call(create_req, ip).await.unwrap();
        
        let task_id = if let RPCResult::Success(result) = create_resp.result {
            result["task_id"].as_i64().unwrap() as i32
        } else {
            panic!("Failed to create task");
        };
        
        // 更新数据
        let update_params = json!({
            "id": task_id,
            "data": {"updated": true, "value": "new data"}
        });
        
        let update_req = create_rpc_request("update_task_data", update_params);
        let update_resp = server.handle_rpc_call(update_req, ip).await.unwrap();
        
        // 验证更新成功
        if let RPCResult::Success(result) = update_resp.result {
            assert_eq!(result["code"], "0");
            
            // 获取任务验证数据
            let get_params = json!({
                "id": task_id
            });
            
            let get_req = create_rpc_request("get_task", get_params);
            let get_resp = server.handle_rpc_call(get_req, ip).await.unwrap();
            
            if let RPCResult::Success(result) = get_resp.result {
                // 解析数据字段
                let data_str = result["task"]["data"].as_str().unwrap();
                let data: Value = serde_json::from_str(data_str).unwrap();
                assert_eq!(data["updated"], true);
                assert_eq!(data["value"], "new data");
            } else {
                panic!("Failed to get task after data update");
            }
        } else {
            panic!("Failed to update task data");
        }
    }

    #[tokio::test]
    async fn test_delete_task() {
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();
        
        // 创建任务
        let create_params = json!({
            "name": "delete_test",
            "title": "Delete Test",
            "task_type": "test_type",
            "app_name": "test_app",
        });
        
        let create_req = create_rpc_request("create_task", create_params);
        let create_resp = server.handle_rpc_call(create_req, ip).await.unwrap();
        
        let task_id = if let RPCResult::Success(result) = create_resp.result {
            result["task_id"].as_i64().unwrap() as i32
        } else {
            panic!("Failed to create task");
        };
        
        // 删除任务
        let delete_params = json!({
            "id": task_id
        });
        
        let delete_req = create_rpc_request("delete_task", delete_params);
        let delete_resp = server.handle_rpc_call(delete_req, ip).await.unwrap();
        
        // 验证删除成功
        if let RPCResult::Success(result) = delete_resp.result {
            assert_eq!(result["code"], "0");
            
            // 尝试获取已删除的任务
            let get_params = json!({
                "id": task_id
            });
            
            let get_req = create_rpc_request("get_task", get_params);
            let get_resp = server.handle_rpc_call(get_req, ip).await.unwrap();
            
            if let RPCResult::Success(result) = get_resp.result {
                assert_eq!(result["code"], "1"); // 应该返回错误代码
            } else {
                panic!("Unexpected error response when getting deleted task");
            }
        } else {
            panic!("Failed to delete task");
        }
    }

    #[tokio::test]
    async fn test_invalid_method() {
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();
        
        // 调用不存在的方法
        let req = create_rpc_request("invalid_method", json!({}));
        let result = server.handle_rpc_call(req, ip).await;
        
        // 验证返回了未知方法错误
        assert!(matches!(result, Err(RPCErrors::UnknownMethod(_))));
    }

    #[tokio::test]
    async fn test_invalid_params() {
        let (server, _temp_dir) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();
        
        // 缺少必要参数
        let create_params = json!({
            // 缺少name字段
            "title": "Invalid Test",
            "task_type": "test_type",
            "app_name": "test_app",
        });
        
        let create_req = create_rpc_request("create_task", create_params);
        let create_resp = server.handle_rpc_call(create_req, ip).await.unwrap();
        
        // 验证返回了参数错误
        if let RPCResult::Success(result) = create_resp.result {
            assert_eq!(result["code"], "1"); // 错误代码
            assert!(result["msg"].as_str().unwrap().contains("name"));
        } else {
            panic!("Unexpected error response for invalid params");
        }
    }
}
