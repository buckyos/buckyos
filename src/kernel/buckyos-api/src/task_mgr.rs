use ::kRPC::*;
use async_trait::async_trait;
use name_lib::DID;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fmt;
use std::net::IpAddr;
use std::ops::Range;
use std::str::FromStr;

use crate::{AppDoc, AppType, SelectorType};

pub const TASK_MANAGER_SERVICE_UNIQUE_ID: &str = "task-manager";
pub const TASK_MANAGER_SERVICE_NAME: &str = "task-manager";
pub const TASK_MANAGER_SERVICE_PORT: u16 = 3380;

pub fn generate_task_manager_service_doc() -> AppDoc {
    const VERSION: &str = env!("CARGO_PKG_VERSION");
    let owner_did = DID::from_str("did:bns:buckyos").unwrap();
    AppDoc::builder(
        AppType::Service,
        TASK_MANAGER_SERVICE_UNIQUE_ID,
        VERSION,
        "did:bns:buckyos",
        &owner_did,
    )
    .show_name("Task Manager")
    .selector_type(SelectorType::Single)
    .build()
    .unwrap()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    Pending,
    Running,
    Paused,
    Completed,
    Failed,
    Canceled,
    WaitingForApproval,
}

impl TaskStatus {
    pub fn from_str(s: &str) -> Result<Self> {
        match s {
            "Pending" => Ok(TaskStatus::Pending),
            "Running" => Ok(TaskStatus::Running),
            "Paused" => Ok(TaskStatus::Paused),
            "Completed" => Ok(TaskStatus::Completed),
            "Failed" => Ok(TaskStatus::Failed),
            "Canceled" => Ok(TaskStatus::Canceled),
            "WaitingForApproval" => Ok(TaskStatus::WaitingForApproval),
            _ => Err(RPCErrors::ReasonError(format!(
                "Invalid task status: {}",
                s
            ))),
        }
    }
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl FromStr for TaskStatus {
    type Err = RPCErrors;

    fn from_str(s: &str) -> Result<Self> {
        TaskStatus::from_str(s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Serialize, Deserialize)]
pub enum TaskScope {
    Private,
    User,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPermissions {
    pub read: TaskScope,
    pub write: TaskScope,
}

impl Default for TaskPermissions {
    fn default() -> Self {
        Self {
            read: TaskScope::User,
            write: TaskScope::Private,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: i64,
    pub user_id: String,
    pub app_id: String,
    pub parent_id: Option<i64>,
    pub root_id: Option<i64>,
    pub name: String,
    pub task_type: String,
    pub status: TaskStatus,
    pub progress: f32,
    pub message: Option<String>,
    pub data: Value,
    pub permissions: TaskPermissions,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct CreateTaskOptions {
    pub permissions: Option<TaskPermissions>,
    pub parent_id: Option<i64>,
    pub priority: Option<u8>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TaskFilter {
    pub app_id: Option<String>,
    pub task_type: Option<String>,
    pub status: Option<TaskStatus>,
    pub parent_id: Option<i64>,
    pub root_id: Option<i64>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TaskUpdatePayload {
    pub id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<TaskStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTaskResult {
    pub task_id: i64,
    pub task: Task,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTaskResult {
    pub task: Task,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListTasksResult {
    pub tasks: Vec<Task>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskManagerCreateTaskReq {
    pub name: String,
    pub task_type: String,
    #[serde(default)]
    pub data: Option<Value>,
    #[serde(default)]
    pub permissions: Option<TaskPermissions>,
    #[serde(default)]
    pub parent_id: Option<i64>,
    #[serde(default)]
    pub priority: Option<u8>,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub app_id: String,
    #[serde(default)]
    pub app_name: Option<String>,
}

impl TaskManagerCreateTaskReq {
    pub fn new(
        name: String,
        task_type: String,
        data: Option<Value>,
        permissions: Option<TaskPermissions>,
        parent_id: Option<i64>,
        priority: Option<u8>,
        user_id: String,
        app_id: String,
    ) -> Self {
        let app_name = if app_id.is_empty() {
            None
        } else {
            Some(app_id.clone())
        };
        Self {
            name,
            task_type,
            data,
            permissions,
            parent_id,
            priority,
            user_id,
            app_id,
            app_name,
        }
    }

    pub fn from_json(value: Value) -> Result<Self> {
        serde_json::from_value(value).map_err(|e| {
            RPCErrors::ParseRequestError(format!("Failed to parse TaskManagerCreateTaskReq: {}", e))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskManagerGetTaskReq {
    pub id: i64,
}

impl TaskManagerGetTaskReq {
    pub fn new(id: i64) -> Self {
        Self { id }
    }

    pub fn from_json(value: Value) -> Result<Self> {
        serde_json::from_value(value).map_err(|e| {
            RPCErrors::ParseRequestError(format!("Failed to parse TaskManagerGetTaskReq: {}", e))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskManagerListTasksReq {
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub task_type: Option<String>,
    #[serde(default)]
    pub status: Option<TaskStatus>,
    #[serde(default)]
    pub parent_id: Option<i64>,
    #[serde(default)]
    pub root_id: Option<i64>,
    #[serde(default)]
    pub source_user_id: Option<String>,
    #[serde(default)]
    pub source_app_id: Option<String>,
}

impl TaskManagerListTasksReq {
    pub fn new(
        filter: TaskFilter,
        source_user_id: Option<String>,
        source_app_id: Option<String>,
    ) -> Self {
        Self {
            app_id: filter.app_id,
            task_type: filter.task_type,
            status: filter.status,
            parent_id: filter.parent_id,
            root_id: filter.root_id,
            source_user_id,
            source_app_id,
        }
    }

    pub fn from_json(value: Value) -> Result<Self> {
        serde_json::from_value(value).map_err(|e| {
            RPCErrors::ParseRequestError(format!("Failed to parse TaskManagerListTasksReq: {}", e))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskManagerListTasksByTimeRangeReq {
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub task_type: Option<String>,
    #[serde(default)]
    pub source_user_id: Option<String>,
    #[serde(default)]
    pub source_app_id: Option<String>,
    pub start_time: u64,
    pub end_time: u64,
}

impl TaskManagerListTasksByTimeRangeReq {
    pub fn new(
        app_id: Option<String>,
        task_type: Option<String>,
        source_user_id: Option<String>,
        source_app_id: Option<String>,
        time_range: Range<u64>,
    ) -> Self {
        Self {
            app_id,
            task_type,
            source_user_id,
            source_app_id,
            start_time: time_range.start,
            end_time: time_range.end,
        }
    }

    pub fn from_json(value: Value) -> Result<Self> {
        serde_json::from_value(value).map_err(|e| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse TaskManagerListTasksByTimeRangeReq: {}",
                e
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskManagerUpdateTaskReq {
    pub id: i64,
    #[serde(default)]
    pub status: Option<TaskStatus>,
    #[serde(default)]
    pub progress: Option<f32>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub data: Option<Value>,
}

impl TaskManagerUpdateTaskReq {
    pub fn new(payload: TaskUpdatePayload) -> Self {
        Self {
            id: payload.id,
            status: payload.status,
            progress: payload.progress,
            message: payload.message,
            data: payload.data,
        }
    }

    pub fn from_json(value: Value) -> Result<Self> {
        serde_json::from_value(value).map_err(|e| {
            RPCErrors::ParseRequestError(format!("Failed to parse TaskManagerUpdateTaskReq: {}", e))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskManagerCancelTaskReq {
    pub id: i64,
    #[serde(default)]
    pub recursive: bool,
}

impl TaskManagerCancelTaskReq {
    pub fn new(id: i64, recursive: bool) -> Self {
        Self { id, recursive }
    }

    pub fn from_json(value: Value) -> Result<Self> {
        serde_json::from_value(value).map_err(|e| {
            RPCErrors::ParseRequestError(format!("Failed to parse TaskManagerCancelTaskReq: {}", e))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskManagerGetSubtasksReq {
    pub parent_id: i64,
}

impl TaskManagerGetSubtasksReq {
    pub fn new(parent_id: i64) -> Self {
        Self { parent_id }
    }

    pub fn from_json(value: Value) -> Result<Self> {
        serde_json::from_value(value).map_err(|e| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse TaskManagerGetSubtasksReq: {}",
                e
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskManagerUpdateTaskStatusReq {
    pub id: i64,
    pub status: TaskStatus,
}

impl TaskManagerUpdateTaskStatusReq {
    pub fn new(id: i64, status: TaskStatus) -> Self {
        Self { id, status }
    }

    pub fn from_json(value: Value) -> Result<Self> {
        serde_json::from_value(value).map_err(|e| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse TaskManagerUpdateTaskStatusReq: {}",
                e
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskManagerUpdateTaskProgressReq {
    pub id: i64,
    pub completed_items: u64,
    pub total_items: u64,
}

impl TaskManagerUpdateTaskProgressReq {
    pub fn new(id: i64, completed_items: u64, total_items: u64) -> Self {
        Self {
            id,
            completed_items,
            total_items,
        }
    }

    pub fn from_json(value: Value) -> Result<Self> {
        serde_json::from_value(value).map_err(|e| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse TaskManagerUpdateTaskProgressReq: {}",
                e
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskManagerUpdateTaskErrorReq {
    pub id: i64,
    pub error_message: String,
}

impl TaskManagerUpdateTaskErrorReq {
    pub fn new(id: i64, error_message: String) -> Self {
        Self { id, error_message }
    }

    pub fn from_json(value: Value) -> Result<Self> {
        serde_json::from_value(value).map_err(|e| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse TaskManagerUpdateTaskErrorReq: {}",
                e
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskManagerUpdateTaskDataReq {
    pub id: i64,
    pub data: Value,
}

impl TaskManagerUpdateTaskDataReq {
    pub fn new(id: i64, data: Value) -> Self {
        Self { id, data }
    }

    pub fn from_json(value: Value) -> Result<Self> {
        serde_json::from_value(value).map_err(|e| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse TaskManagerUpdateTaskDataReq: {}",
                e
            ))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskManagerDeleteTaskReq {
    pub id: i64,
}

impl TaskManagerDeleteTaskReq {
    pub fn new(id: i64) -> Self {
        Self { id }
    }

    pub fn from_json(value: Value) -> Result<Self> {
        serde_json::from_value(value).map_err(|e| {
            RPCErrors::ParseRequestError(format!("Failed to parse TaskManagerDeleteTaskReq: {}", e))
        })
    }
}

pub enum TaskManagerClient {
    InProcess(Box<dyn TaskManagerHandler>),
    KRPC(Box<kRPC>),
}

impl TaskManagerClient {
    pub fn new(client: kRPC) -> Self {
        Self::KRPC(Box::new(client))
    }

    pub fn new_in_process(handler: Box<dyn TaskManagerHandler>) -> Self {
        Self::InProcess(handler)
    }

    pub fn new_krpc(client: Box<kRPC>) -> Self {
        Self::KRPC(client)
    }

    pub async fn set_context(&self, context: RPCContext) {
        match self {
            Self::InProcess(_) => {}
            Self::KRPC(client) => client.set_context(context).await,
        }
    }

    //reivew:
    pub async fn create_task(
        &self,
        name: &str,
        task_type: &str,
        data: Option<Value>,
        user_id: &str,
        app_id: &str,
        opts: Option<CreateTaskOptions>,
    ) -> Result<Task> {
        let opts = opts.unwrap_or_default();
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                let result = handler
                    .handle_create_task(name, task_type, data, opts, user_id, app_id, ctx)
                    .await?;
                Ok(result)
            }
            Self::KRPC(client) => {
                let req = TaskManagerCreateTaskReq::new(
                    name.to_string(),
                    task_type.to_string(),
                    data,
                    opts.permissions,
                    opts.parent_id,
                    opts.priority,
                    user_id.to_string(),
                    app_id.to_string(),
                );
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to serialize request: {}", e))
                })?;
                let result = client.call("create_task", req_json).await?;

                if let Ok(response) = serde_json::from_value::<CreateTaskResult>(result.clone()) {
                    return Ok(response.task);
                }

                if let Some(task_id) = result.get("task_id").and_then(|v| v.as_i64()) {
                    let task = self.get_task(task_id).await?;
                    return Ok(task);
                }

                Err(RPCErrors::ParserResponseError(
                    "Expected CreateTaskResult response".to_string(),
                ))
            }
        }
    }

    pub async fn get_task(&self, id: i64) -> Result<Task> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                let result = handler.handle_get_task(id, ctx).await?;
                Ok(result)
            }
            Self::KRPC(client) => {
                let req = TaskManagerGetTaskReq::new(id);
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to serialize request: {}", e))
                })?;
                let result = client.call("get_task", req_json).await?;

                if let Ok(response) = serde_json::from_value::<GetTaskResult>(result.clone()) {
                    return Ok(response.task);
                }

                result
                    .get("task")
                    .and_then(|value| serde_json::from_value::<Task>(value.clone()).ok())
                    .ok_or_else(|| {
                        RPCErrors::ParserResponseError("Expected task in response".to_string())
                    })
            }
        }
    }

    pub async fn list_tasks(
        &self,
        filter: Option<TaskFilter>,
        source_user_id: Option<&str>,
        source_app_id: Option<&str>,
    ) -> Result<Vec<Task>> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                let result = handler
                    .handle_list_tasks(
                        filter.unwrap_or_default(),
                        source_user_id,
                        source_app_id,
                        ctx,
                    )
                    .await?;
                Ok(result)
            }
            Self::KRPC(client) => {
                let req = TaskManagerListTasksReq::new(
                    filter.unwrap_or_default(),
                    source_user_id.map(|value| value.to_string()),
                    source_app_id.map(|value| value.to_string()),
                );
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to serialize request: {}", e))
                })?;
                let result = client.call("list_tasks", req_json).await?;

                if let Ok(response) = serde_json::from_value::<ListTasksResult>(result.clone()) {
                    return Ok(response.tasks);
                }

                if let Some(tasks_value) = result.get("tasks") {
                    return serde_json::from_value::<Vec<Task>>(tasks_value.clone()).map_err(|e| {
                        RPCErrors::ParserResponseError(format!("Failed to parse tasks: {}", e))
                    });
                }

                Err(RPCErrors::ParserResponseError(
                    "Expected tasks in response".to_string(),
                ))
            }
        }
    }

    pub async fn list_tasks_by_time_range(
        &self,
        app_id: Option<&str>,
        task_type: Option<&str>,
        source_user_id: Option<&str>,
        source_app_id: Option<&str>,
        time_range: Range<u64>,
    ) -> Result<Vec<Task>> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                let result = handler
                    .handle_list_tasks_by_time_range(
                        app_id,
                        task_type,
                        source_user_id,
                        source_app_id,
                        time_range,
                        ctx,
                    )
                    .await?;
                Ok(result)
            }
            Self::KRPC(client) => {
                let req = TaskManagerListTasksByTimeRangeReq::new(
                    app_id.map(|value| value.to_string()),
                    task_type.map(|value| value.to_string()),
                    source_user_id.map(|value| value.to_string()),
                    source_app_id.map(|value| value.to_string()),
                    time_range,
                );
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to serialize request: {}", e))
                })?;
                let result = client.call("list_tasks_by_time_range", req_json).await?;

                if let Ok(response) = serde_json::from_value::<ListTasksResult>(result.clone()) {
                    return Ok(response.tasks);
                }

                if let Some(tasks_value) = result.get("tasks") {
                    return serde_json::from_value::<Vec<Task>>(tasks_value.clone()).map_err(|e| {
                        RPCErrors::ParserResponseError(format!("Failed to parse tasks: {}", e))
                    });
                }

                Err(RPCErrors::ParserResponseError(
                    "Expected tasks in response".to_string(),
                ))
            }
        }
    }

    pub async fn update_task(
        &self,
        id: i64,
        status: Option<TaskStatus>,
        progress: Option<f32>,
        message: Option<String>,
        data_patch: Option<Value>,
    ) -> Result<()> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler
                    .handle_update_task(id, status, progress, message, data_patch, ctx)
                    .await
            }
            Self::KRPC(client) => {
                let payload = TaskUpdatePayload {
                    id,
                    status,
                    progress,
                    message,
                    data: data_patch,
                };
                let req = TaskManagerUpdateTaskReq::new(payload);
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to serialize request: {}", e))
                })?;
                client.call("update_task", req_json).await?;
                Ok(())
            }
        }
    }

    pub async fn cancel_task(&self, id: i64, recursive: bool) -> Result<()> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_cancel_task(id, recursive, ctx).await
            }
            Self::KRPC(client) => {
                let req = TaskManagerCancelTaskReq::new(id, recursive);
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to serialize request: {}", e))
                })?;
                client.call("cancel_task", req_json).await?;
                Ok(())
            }
        }
    }

    pub async fn get_subtasks(&self, parent_id: i64) -> Result<Vec<Task>> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                let result = handler.handle_get_subtasks(parent_id, ctx).await?;
                Ok(result)
            }
            Self::KRPC(client) => {
                let req = TaskManagerGetSubtasksReq::new(parent_id);
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to serialize request: {}", e))
                })?;
                let result = client.call("get_subtasks", req_json).await?;
                if let Ok(response) = serde_json::from_value::<ListTasksResult>(result.clone()) {
                    return Ok(response.tasks);
                }
                if let Some(tasks_value) = result.get("tasks") {
                    return serde_json::from_value::<Vec<Task>>(tasks_value.clone()).map_err(|e| {
                        RPCErrors::ParserResponseError(format!("Failed to parse tasks: {}", e))
                    });
                }
                Err(RPCErrors::ParserResponseError(
                    "Expected tasks in response".to_string(),
                ))
            }
        }
    }

    pub async fn update_task_status(&self, id: i64, status: TaskStatus) -> Result<()> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_update_task_status(id, status, ctx).await
            }
            Self::KRPC(client) => {
                let req = TaskManagerUpdateTaskStatusReq::new(id, status);
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to serialize request: {}", e))
                })?;
                client.call("update_task_status", req_json).await?;
                Ok(())
            }
        }
    }

    pub async fn update_task_progress(
        &self,
        id: i64,
        completed_items: u64,
        total_items: u64,
    ) -> Result<()> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler
                    .handle_update_task_progress(id, completed_items, total_items, ctx)
                    .await
            }
            Self::KRPC(client) => {
                let req = TaskManagerUpdateTaskProgressReq::new(id, completed_items, total_items);
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to serialize request: {}", e))
                })?;
                client.call("update_task_progress", req_json).await?;
                Ok(())
            }
        }
    }

    pub async fn update_task_error(&self, id: i64, error_message: &str) -> Result<()> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler
                    .handle_update_task_error(id, error_message, ctx)
                    .await
            }
            Self::KRPC(client) => {
                let req = TaskManagerUpdateTaskErrorReq::new(id, error_message.to_string());
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to serialize request: {}", e))
                })?;
                client.call("update_task_error", req_json).await?;
                Ok(())
            }
        }
    }

    pub async fn update_task_data(&self, id: i64, data: Value) -> Result<()> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_update_task_data(id, data, ctx).await
            }
            Self::KRPC(client) => {
                let req = TaskManagerUpdateTaskDataReq::new(id, data);
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to serialize request: {}", e))
                })?;
                client.call("update_task_data", req_json).await?;
                Ok(())
            }
        }
    }

    pub async fn delete_task(&self, id: i64) -> Result<()> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_delete_task(id, ctx).await
            }
            Self::KRPC(client) => {
                let req = TaskManagerDeleteTaskReq::new(id);
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!("Failed to serialize request: {}", e))
                })?;
                client.call("delete_task", req_json).await?;
                Ok(())
            }
        }
    }

    pub async fn pause_task(&self, id: i64) -> Result<()> {
        self.update_task_status(id, TaskStatus::Paused).await
    }

    pub async fn resume_task(&self, id: i64) -> Result<()> {
        self.update_task_status(id, TaskStatus::Running).await
    }

    pub async fn complete_task(&self, id: i64) -> Result<()> {
        self.update_task_status(id, TaskStatus::Completed).await
    }

    pub async fn mark_task_as_waiting_for_approval(&self, id: i64) -> Result<()> {
        self.update_task_status(id, TaskStatus::WaitingForApproval)
            .await
    }

    pub async fn mark_task_as_failed(&self, id: i64, error_message: &str) -> Result<()> {
        self.update_task_error(id, error_message).await?;
        self.update_task_status(id, TaskStatus::Failed).await
    }

    pub async fn pause_all_running_tasks(
        &self,
        source_user_id: Option<&str>,
        source_app_id: Option<&str>,
    ) -> Result<()> {
        let filter = TaskFilter {
            status: Some(TaskStatus::Running),
            ..Default::default()
        };

        let running_tasks = self
            .list_tasks(Some(filter), source_user_id, source_app_id)
            .await?;

        for task in running_tasks {
            self.pause_task(task.id).await?;
        }

        Ok(())
    }

    pub async fn resume_last_paused_task(
        &self,
        source_user_id: Option<&str>,
        source_app_id: Option<&str>,
    ) -> Result<()> {
        let filter = TaskFilter {
            status: Some(TaskStatus::Paused),
            ..Default::default()
        };

        let paused_tasks = self
            .list_tasks(Some(filter), source_user_id, source_app_id)
            .await?;

        if let Some(last_paused) = paused_tasks.last() {
            self.resume_task(last_paused.id).await?;
            Ok(())
        } else {
            Err(RPCErrors::ReasonError("No paused tasks found".to_string()))
        }
    }
}

#[async_trait]
pub trait TaskManagerHandler: Send + Sync {
    async fn handle_create_task(
        &self,
        name: &str,
        task_type: &str,
        data: Option<Value>,
        opts: CreateTaskOptions,
        user_id: &str,
        app_id: &str,
        ctx: RPCContext,
    ) -> Result<Task>;

    async fn handle_get_task(&self, id: i64, ctx: RPCContext) -> Result<Task>;

    async fn handle_list_tasks(
        &self,
        filter: TaskFilter,
        source_user_id: Option<&str>,
        source_app_id: Option<&str>,
        ctx: RPCContext,
    ) -> Result<Vec<Task>>;

    //得到从一个时间范围内的所有任务
    async fn handle_list_tasks_by_time_range(
        &self,
        app_id: Option<&str>,
        task_type: Option<&str>,
        source_user_id: Option<&str>,
        source_app_id: Option<&str>,
        time_range: Range<u64>,
        ctx: RPCContext,
    ) -> Result<Vec<Task>>;

    async fn handle_get_subtasks(&self, parent_id: i64, ctx: RPCContext) -> Result<Vec<Task>>;

    async fn handle_update_task(
        &self,
        id: i64,
        status: Option<TaskStatus>,
        progress: Option<f32>,
        message: Option<String>,
        data: Option<Value>,
        ctx: RPCContext,
    ) -> Result<()>;

    async fn handle_update_task_progress(
        &self,
        id: i64,
        completed_items: u64,
        total_items: u64,
        ctx: RPCContext,
    ) -> Result<()>;

    async fn handle_update_task_status(
        &self,
        id: i64,
        status: TaskStatus,
        ctx: RPCContext,
    ) -> Result<()>;

    async fn handle_update_task_error(
        &self,
        id: i64,
        error_message: &str,
        ctx: RPCContext,
    ) -> Result<()>;

    async fn handle_update_task_data(&self, id: i64, data: Value, ctx: RPCContext) -> Result<()>;

    async fn handle_cancel_task(&self, id: i64, recursive: bool, ctx: RPCContext) -> Result<()>;

    async fn handle_delete_task(&self, id: i64, ctx: RPCContext) -> Result<()>;
}

pub struct TaskManagerServerHandler<T: TaskManagerHandler>(pub T);

impl<T: TaskManagerHandler> TaskManagerServerHandler<T> {
    pub fn new(handler: T) -> Self {
        Self(handler)
    }
}

#[async_trait]
impl<T: TaskManagerHandler> RPCHandler for TaskManagerServerHandler<T> {
    async fn handle_rpc_call(&self, req: RPCRequest, ip_from: IpAddr) -> Result<RPCResponse> {
        let seq = req.seq;
        let trace_id = req.trace_id.clone();
        let ctx = RPCContext::from_request(&req, ip_from);

        let result = match req.method.as_str() {
            "create_task" => {
                let create_req = TaskManagerCreateTaskReq::from_json(req.params)?;
                let TaskManagerCreateTaskReq {
                    name,
                    task_type,
                    data,
                    permissions,
                    parent_id,
                    priority,
                    user_id,
                    app_id,
                    ..
                } = create_req;
                let opts = CreateTaskOptions {
                    permissions,
                    parent_id,
                    priority,
                };
                let task = self
                    .0
                    .handle_create_task(
                        &name,
                        &task_type,
                        data,
                        opts,
                        user_id.as_str(),
                        app_id.as_str(),
                        ctx,
                    )
                    .await?;
                RPCResult::Success(json!(CreateTaskResult {
                    task_id: task.id,
                    task
                }))
            }
            "get_task" => {
                let get_req = TaskManagerGetTaskReq::from_json(req.params)?;
                let task = self.0.handle_get_task(get_req.id, ctx).await?;
                RPCResult::Success(json!(GetTaskResult { task }))
            }
            "list_tasks" => {
                let list_req = TaskManagerListTasksReq::from_json(req.params)?;
                let TaskManagerListTasksReq {
                    app_id,
                    task_type,
                    status,
                    parent_id,
                    root_id,
                    source_user_id,
                    source_app_id,
                } = list_req;
                let filter = TaskFilter {
                    app_id,
                    task_type,
                    status,
                    parent_id,
                    root_id,
                };
                let tasks = self
                    .0
                    .handle_list_tasks(
                        filter,
                        source_user_id.as_deref(),
                        source_app_id.as_deref(),
                        ctx,
                    )
                    .await?;
                RPCResult::Success(json!(ListTasksResult { tasks }))
            }
            "list_tasks_by_time_range" => {
                let list_req = TaskManagerListTasksByTimeRangeReq::from_json(req.params)?;
                let TaskManagerListTasksByTimeRangeReq {
                    app_id,
                    task_type,
                    source_user_id,
                    source_app_id,
                    start_time,
                    end_time,
                } = list_req;
                let time_range = start_time..end_time;
                let tasks = self
                    .0
                    .handle_list_tasks_by_time_range(
                        app_id.as_deref(),
                        task_type.as_deref(),
                        source_user_id.as_deref(),
                        source_app_id.as_deref(),
                        time_range,
                        ctx,
                    )
                    .await?;
                RPCResult::Success(json!(ListTasksResult { tasks }))
            }
            "update_task" => {
                let update_req = TaskManagerUpdateTaskReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_update_task(
                        update_req.id,
                        update_req.status,
                        update_req.progress,
                        update_req.message,
                        update_req.data,
                        ctx,
                    )
                    .await?;
                RPCResult::Success(json!(result))
            }
            "cancel_task" => {
                let cancel_req = TaskManagerCancelTaskReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_cancel_task(cancel_req.id, cancel_req.recursive, ctx)
                    .await?;
                RPCResult::Success(json!(result))
            }
            "get_subtasks" => {
                let sub_req = TaskManagerGetSubtasksReq::from_json(req.params)?;
                let tasks = self.0.handle_get_subtasks(sub_req.parent_id, ctx).await?;
                RPCResult::Success(json!(ListTasksResult { tasks }))
            }
            "update_task_status" => {
                let update_req = TaskManagerUpdateTaskStatusReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_update_task_status(update_req.id, update_req.status, ctx)
                    .await?;
                RPCResult::Success(json!(result))
            }
            "update_task_progress" => {
                let update_req = TaskManagerUpdateTaskProgressReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_update_task_progress(
                        update_req.id,
                        update_req.completed_items,
                        update_req.total_items,
                        ctx,
                    )
                    .await?;
                RPCResult::Success(json!(result))
            }
            "update_task_error" => {
                let update_req = TaskManagerUpdateTaskErrorReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_update_task_error(update_req.id, update_req.error_message.as_str(), ctx)
                    .await?;
                RPCResult::Success(json!(result))
            }
            "update_task_data" => {
                let update_req = TaskManagerUpdateTaskDataReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_update_task_data(update_req.id, update_req.data, ctx)
                    .await?;
                RPCResult::Success(json!(result))
            }
            "delete_task" => {
                let delete_req = TaskManagerDeleteTaskReq::from_json(req.params)?;
                let result = self.0.handle_delete_task(delete_req.id, ctx).await?;
                RPCResult::Success(json!(result))
            }
            _ => return Err(RPCErrors::UnknownMethod(req.method.clone())),
        };

        Ok(RPCResponse {
            result,
            seq,
            trace_id,
        })
    }
}
