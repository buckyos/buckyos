use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ::kRPC::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::str::FromStr;
use std::sync::Arc;
use std::result::Result;
use log::{debug, error, info};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TaskStatus {
    Pending,    // 任务已创建但尚未开始
    Running,    // 任务正在运行
    Paused,     // 任务已暂停
    Completed,  // 任务已完成
    Failed,     // 任务失败
    WaitingForApproval, // 任务完成但等待审核/批准
}

impl TaskStatus {
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "Pending" => Ok(TaskStatus::Pending),
            "Running" => Ok(TaskStatus::Running),
            "Paused" => Ok(TaskStatus::Paused),
            "Completed" => Ok(TaskStatus::Completed),
            "Failed" => Ok(TaskStatus::Failed),
            "WaitingForApproval" => Ok(TaskStatus::WaitingForApproval),
            _ => Err(format!("Invalid task status: {}", s)),
        }
    }
}

impl ToString for TaskStatus {
    fn to_string(&self) -> String {
        match self {
            TaskStatus::Pending => "Pending".to_string(),
            TaskStatus::Running => "Running".to_string(),
            TaskStatus::Paused => "Paused".to_string(),
            TaskStatus::Completed => "Completed".to_string(),
            TaskStatus::Failed => "Failed".to_string(),
            TaskStatus::WaitingForApproval => "WaitingForApproval".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: i32,
    pub name: String,
    pub task_type: String,     // 任务类型，如"publish", "backup", "restore"等
    pub app_name: String,      // 关联的应用名称
    pub status: TaskStatus,    // 任务状态
    pub progress: f32,         // 进度百分比 (0.0-100.0)
    pub total_items: i32,      // 总项目数（如总文件数、总chunk数等）
    pub completed_items: i32,  // 已完成项目数
    pub error_message: Option<String>, // 错误信息（如果有）
    pub data: Option<String>,  // 任务相关的JSON数据，可存储任何任务特定信息
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskFilter {
    pub app_name: Option<String>,
    pub task_type: Option<String>,
    pub status: Option<TaskStatus>,
}

pub struct TaskManager {
    rpc_client: Arc<kRPC>,
}

impl TaskManager {
    pub fn new(rpc_client: Arc<kRPC>) -> Self {
        Self { rpc_client }
    }

    pub async fn create_task(&self, name: &str, task_type: &str, app_name: &str, data: Option<Value>) -> Result<i32, String> {
        let params = json!({
            "name": name,
            "task_type": task_type,
            "app_name": app_name,
            "data": data
        });

        //let req = RPCRequest::new("create_task", params);
        match self.rpc_client.call("create_task", params).await {
            Ok(resp) => {
                match resp.get("task_id") {
                    Some(task_id) => Ok(task_id.as_i64().unwrap_or_default() as i32),
                    None => Err("Response missing task_id field".to_string()),
                }
            },
            Err(e) => Err(format!("RPC error: {:?}", e)),
        }
    }

    pub async fn get_task(&self, id: i32) -> Result<Task, String> {
        let params = json!({
            "id": id
        });

        //let req = RPCRequest::new("get_task", params);
        match self.rpc_client.call("get_task", params).await {
            Ok(resp) => {
                match  resp.get("task") {
                    Some(task_json) => {
                        match serde_json::from_value::<Task>(task_json.clone()) {
                            Ok(task) => Ok(task),
                            Err(e) => Err(format!("Failed to parse task: {}", e)),
                        }
                    },
                    None => Err("Response missing task field".to_string()),
                }
            },
            Err(e) => Err(format!("RPC error: {:?}", e)),
        }
    }

    pub async fn list_tasks(&self, filter: Option<TaskFilter>) -> Result<Vec<Task>, String> {
        let mut params = json!({});
        
        if let Some(filter) = filter {
            if let Some(app_name) = filter.app_name {
                params["app_name"] = json!(app_name);
            }
            if let Some(task_type) = filter.task_type {
                params["task_type"] = json!(task_type);
            }
            if let Some(status) = filter.status {
                params["status"] = json!(status.to_string());
            }
        }

        //let req = RPCRequest::new("list_tasks", params);
        match self.rpc_client.call("list_tasks", params).await {
            Ok(resp) => {
                match resp.get("tasks") {
                    Some(tasks_json) => {
                        match serde_json::from_value::<Vec<Task>>(tasks_json.clone()) {
                            Ok(tasks) => Ok(tasks),
                            Err(e) => Err(format!("Failed to parse tasks: {}", e)),
                        }
                    },
                    None => Err("Response missing tasks field".to_string()),
                }
            },
            Err(e) => Err(format!("RPC error: {:?}", e)),
        }
    }

    pub async fn update_task_status(&self, id: i32, status: TaskStatus) -> Result<(), String> {
        let params = json!({
            "id": id,
            "status": status.to_string()
        });

        //let req = RPCRequest::new("update_task_status", params);
        match self.rpc_client.call("update_task_status", params).await {
            Ok(resp) => {
                Ok(())
            },
            Err(e) => Err(format!("RPC error: {:?}", e)),
        }
    }

    pub async fn update_task_progress(&self, id: i32, completed_items: u64, total_items: u64) -> Result<(), String> {
        let progress = if total_items > 0 {
            (completed_items as f32 / total_items as f32) * 100.0
        } else {
            0.0
        };

        let params = json!({
            "id": id,
            "progress": progress,
            "completed_items": completed_items,
            "total_items": total_items
        });

        //let req = RPCRequest::new("update_task_progress", params);
        match self.rpc_client.call("update_task_progress", params).await {
            Ok(resp) => {
                Ok(())
            },
            Err(e) => Err(format!("RPC error: {:?}", e)),
        }
    }

    pub async fn update_task_error(&self, id: i32, error_message: &str) -> Result<(), String> {
        let params = json!({
            "id": id,
            "error_message": error_message
        });

        //let req = RPCRequest::new("update_task_error", params);
        match self.rpc_client.call("update_task_error", params).await {
            Ok(resp) => {
                Ok(())
            },
            Err(e) => Err(format!("RPC error: {:?}", e)),
        }
    }

    pub async fn update_task_data(&self, id: i32, data: Value) -> Result<(), String> {
        let params = json!({
            "id": id,
            "data": data
        });

        //let req = RPCRequest::new("update_task_data", params);
        match self.rpc_client.call("update_task_data", params).await {
            Ok(resp) => {
                Ok(())
            },
            Err(e) => Err(format!("RPC error: {:?}", e)),
        }
    }

    pub async fn delete_task(&self, id: i32) -> Result<(), String> {
        let params = json!({
            "id": id
        });

        //let req = RPCRequest::new("delete_task", params);
        match self.rpc_client.call("delete_task", params).await {
            Ok(resp) => {
                Ok(())
            },
            Err(e) => Err(format!("RPC error: {:?}", e)),
        }
    }

    // Convenience methods similar to the TypeScript client
    pub async fn pause_task(&self, id: i32) -> Result<(), String> {
        self.update_task_status(id, TaskStatus::Paused).await
    }

    pub async fn resume_task(&self, id: i32) -> Result<(), String> {
        self.update_task_status(id, TaskStatus::Running).await
    }

    pub async fn complete_task(&self, id: i32) -> Result<(), String> {
        self.update_task_status(id, TaskStatus::Completed).await
    }

    pub async fn mark_task_as_waiting_for_approval(&self, id: i32) -> Result<(), String> {
        self.update_task_status(id, TaskStatus::WaitingForApproval).await
    }

    pub async fn mark_task_as_failed(&self, id: i32, error_message: &str) -> Result<(), String> {
        // First update the error message
        self.update_task_error(id, error_message).await?;
        // Then update the status
        self.update_task_status(id, TaskStatus::Failed).await
    }

    pub async fn pause_all_running_tasks(&self) -> Result<(), String> {
        let filter = TaskFilter {
            app_name: None,
            task_type: None,
            status: Some(TaskStatus::Running),
        };
        
        let running_tasks = self.list_tasks(Some(filter)).await?;
        
        for task in running_tasks {
            self.pause_task(task.id).await?;
        }
        
        Ok(())
    }

    pub async fn resume_last_paused_task(&self) -> Result<(), String> {
        let filter = TaskFilter {
            app_name: None,
            task_type: None,
            status: Some(TaskStatus::Paused),
        };
        
        let paused_tasks = self.list_tasks(Some(filter)).await?;
        
        if let Some(last_paused) = paused_tasks.last() {
            self.resume_task(last_paused.id).await?;
            Ok(())
        } else {
            Err("No paused tasks found".to_string())
        }
    }
}
