use ::kRPC::*;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fmt;
use std::net::IpAddr;
use std::str::FromStr;

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
    ) -> Self {
        Self {
            name,
            task_type,
            data,
            permissions,
            parent_id,
            priority,
            app_name: None,
        }
    }

    pub fn from_json(value: Value) -> Result<Self> {
        serde_json::from_value(value).map_err(|e| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse TaskManagerCreateTaskReq: {}",
                e
            ))
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
            RPCErrors::ParseRequestError(format!(
                "Failed to parse TaskManagerGetTaskReq: {}",
                e
            ))
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
}

impl TaskManagerListTasksReq {
    pub fn new(filter: TaskFilter) -> Self {
        Self {
            app_id: filter.app_id,
            task_type: filter.task_type,
            status: filter.status,
            parent_id: filter.parent_id,
            root_id: filter.root_id,
        }
    }

    pub fn from_json(value: Value) -> Result<Self> {
        serde_json::from_value(value).map_err(|e| {
            RPCErrors::ParseRequestError(format!(
                "Failed to parse TaskManagerListTasksReq: {}",
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
            RPCErrors::ParseRequestError(format!(
                "Failed to parse TaskManagerUpdateTaskReq: {}",
                e
            ))
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
            RPCErrors::ParseRequestError(format!(
                "Failed to parse TaskManagerCancelTaskReq: {}",
                e
            ))
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
            RPCErrors::ParseRequestError(format!(
                "Failed to parse TaskManagerDeleteTaskReq: {}",
                e
            ))
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

    pub async fn create_task(
        &self,
        name: &str,
        task_type: &str,
        data: Option<Value>,
        opts: Option<CreateTaskOptions>,
    ) -> Result<Task> {
        let opts = opts.unwrap_or_default();
        match self {
            Self::InProcess(handler) => {
                let req = TaskManagerCreateTaskReq::new(
                    name.to_string(),
                    task_type.to_string(),
                    data,
                    opts.permissions,
                    opts.parent_id,
                    opts.priority,
                );
                let result = handler.handle_create_task(req).await?;
                Ok(result.task)
            }
            Self::KRPC(client) => {
                let req = TaskManagerCreateTaskReq::new(
                    name.to_string(),
                    task_type.to_string(),
                    data,
                    opts.permissions,
                    opts.parent_id,
                    opts.priority,
                );
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize request: {}",
                        e
                    ))
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
                let result = handler.handle_get_task(id).await?;
                Ok(result.task)
            }
            Self::KRPC(client) => {
                let req = TaskManagerGetTaskReq::new(id);
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize request: {}",
                        e
                    ))
                })?;
                let result = client.call("get_task", req_json).await?;

                if let Ok(response) = serde_json::from_value::<GetTaskResult>(result.clone()) {
                    return Ok(response.task);
                }

                result
                    .get("task")
                    .and_then(|value| serde_json::from_value::<Task>(value.clone()).ok())
                    .ok_or_else(|| {
                        RPCErrors::ParserResponseError(
                            "Expected task in response".to_string(),
                        )
                    })
            }
        }
    }

    pub async fn list_tasks(&self, filter: Option<TaskFilter>) -> Result<Vec<Task>> {
        match self {
            Self::InProcess(handler) => {
                let req = TaskManagerListTasksReq::new(filter.unwrap_or_default());
                let result = handler.handle_list_tasks(req).await?;
                Ok(result.tasks)
            }
            Self::KRPC(client) => {
                let req = TaskManagerListTasksReq::new(filter.unwrap_or_default());
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize request: {}",
                        e
                    ))
                })?;
                let result = client.call("list_tasks", req_json).await?;

                if let Ok(response) = serde_json::from_value::<ListTasksResult>(result.clone()) {
                    return Ok(response.tasks);
                }

                if let Some(tasks_value) = result.get("tasks") {
                    return serde_json::from_value::<Vec<Task>>(tasks_value.clone()).map_err(
                        |e| {
                            RPCErrors::ParserResponseError(format!(
                                "Failed to parse tasks: {}",
                                e
                            ))
                        },
                    );
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
                let payload = TaskUpdatePayload {
                    id,
                    status,
                    progress,
                    message,
                    data: data_patch,
                };
                let req = TaskManagerUpdateTaskReq::new(payload);
                handler.handle_update_task(req).await
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
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize request: {}",
                        e
                    ))
                })?;
                client.call("update_task", req_json).await?;
                Ok(())
            }
        }
    }

    pub async fn cancel_task(&self, id: i64, recursive: bool) -> Result<()> {
        match self {
            Self::InProcess(handler) => {
                let req = TaskManagerCancelTaskReq::new(id, recursive);
                handler.handle_cancel_task(req).await
            }
            Self::KRPC(client) => {
                let req = TaskManagerCancelTaskReq::new(id, recursive);
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize request: {}",
                        e
                    ))
                })?;
                client.call("cancel_task", req_json).await?;
                Ok(())
            }
        }
    }

    pub async fn get_subtasks(&self, parent_id: i64) -> Result<Vec<Task>> {
        match self {
            Self::InProcess(handler) => {
                let req = TaskManagerGetSubtasksReq::new(parent_id);
                let result = handler.handle_get_subtasks(req).await?;
                Ok(result.tasks)
            }
            Self::KRPC(client) => {
                let req = TaskManagerGetSubtasksReq::new(parent_id);
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize request: {}",
                        e
                    ))
                })?;
                let result = client.call("get_subtasks", req_json).await?;
                if let Ok(response) = serde_json::from_value::<ListTasksResult>(result.clone()) {
                    return Ok(response.tasks);
                }
                if let Some(tasks_value) = result.get("tasks") {
                    return serde_json::from_value::<Vec<Task>>(tasks_value.clone()).map_err(
                        |e| {
                            RPCErrors::ParserResponseError(format!(
                                "Failed to parse tasks: {}",
                                e
                            ))
                        },
                    );
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
                let req = TaskManagerUpdateTaskStatusReq::new(id, status);
                handler.handle_update_task_status(req).await
            }
            Self::KRPC(client) => {
                let req = TaskManagerUpdateTaskStatusReq::new(id, status);
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize request: {}",
                        e
                    ))
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
                let req = TaskManagerUpdateTaskProgressReq::new(id, completed_items, total_items);
                handler.handle_update_task_progress(req).await
            }
            Self::KRPC(client) => {
                let req = TaskManagerUpdateTaskProgressReq::new(id, completed_items, total_items);
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize request: {}",
                        e
                    ))
                })?;
                client.call("update_task_progress", req_json).await?;
                Ok(())
            }
        }
    }

    pub async fn update_task_error(&self, id: i64, error_message: &str) -> Result<()> {
        match self {
            Self::InProcess(handler) => {
                let req = TaskManagerUpdateTaskErrorReq::new(id, error_message.to_string());
                handler.handle_update_task_error(req).await
            }
            Self::KRPC(client) => {
                let req = TaskManagerUpdateTaskErrorReq::new(id, error_message.to_string());
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize request: {}",
                        e
                    ))
                })?;
                client.call("update_task_error", req_json).await?;
                Ok(())
            }
        }
    }

    pub async fn update_task_data(&self, id: i64, data: Value) -> Result<()> {
        match self {
            Self::InProcess(handler) => {
                let req = TaskManagerUpdateTaskDataReq::new(id, data);
                handler.handle_update_task_data(req).await
            }
            Self::KRPC(client) => {
                let req = TaskManagerUpdateTaskDataReq::new(id, data);
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize request: {}",
                        e
                    ))
                })?;
                client.call("update_task_data", req_json).await?;
                Ok(())
            }
        }
    }

    pub async fn delete_task(&self, id: i64) -> Result<()> {
        match self {
            Self::InProcess(handler) => {
                let req = TaskManagerDeleteTaskReq::new(id);
                handler.handle_delete_task(req).await
            }
            Self::KRPC(client) => {
                let req = TaskManagerDeleteTaskReq::new(id);
                let req_json = serde_json::to_value(&req).map_err(|e| {
                    RPCErrors::ReasonError(format!(
                        "Failed to serialize request: {}",
                        e
                    ))
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

    pub async fn pause_all_running_tasks(&self) -> Result<()> {
        let filter = TaskFilter {
            status: Some(TaskStatus::Running),
            ..Default::default()
        };

        let running_tasks = self.list_tasks(Some(filter)).await?;

        for task in running_tasks {
            self.pause_task(task.id).await?;
        }

        Ok(())
    }

    pub async fn resume_last_paused_task(&self) -> Result<()> {
        let filter = TaskFilter {
            status: Some(TaskStatus::Paused),
            ..Default::default()
        };

        let paused_tasks = self.list_tasks(Some(filter)).await?;

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
        req: TaskManagerCreateTaskReq,
    ) -> Result<CreateTaskResult>;

    async fn handle_get_task(&self, id: i64) -> Result<GetTaskResult>;

    async fn handle_list_tasks(&self, req: TaskManagerListTasksReq) -> Result<ListTasksResult>;

    async fn handle_update_task(&self, req: TaskManagerUpdateTaskReq) -> Result<()>;

    async fn handle_cancel_task(&self, req: TaskManagerCancelTaskReq) -> Result<()>;

    async fn handle_get_subtasks(&self, req: TaskManagerGetSubtasksReq) -> Result<ListTasksResult>;

    async fn handle_update_task_status(&self, req: TaskManagerUpdateTaskStatusReq) -> Result<()>;

    async fn handle_update_task_progress(&self, req: TaskManagerUpdateTaskProgressReq)
        -> Result<()>;

    async fn handle_update_task_error(&self, req: TaskManagerUpdateTaskErrorReq) -> Result<()>;

    async fn handle_update_task_data(&self, req: TaskManagerUpdateTaskDataReq) -> Result<()>;

    async fn handle_delete_task(&self, req: TaskManagerDeleteTaskReq) -> Result<()>;
}

pub struct TaskManagerServerHandler<T: TaskManagerHandler>(pub T);

impl<T: TaskManagerHandler> TaskManagerServerHandler<T> {
    pub fn new(handler: T) -> Self {
        Self(handler)
    }
}

#[async_trait]
impl<T: TaskManagerHandler> RPCHandler for TaskManagerServerHandler<T> {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        _ip_from: IpAddr,
    ) -> Result<RPCResponse> {
        let seq = req.seq;
        let trace_id = req.trace_id.clone();

        let result = match req.method.as_str() {
            "create_task" => {
                let create_req = TaskManagerCreateTaskReq::from_json(req.params)?;
                let result = self.0.handle_create_task(create_req).await?;
                RPCResult::Success(json!(result))
            }
            "get_task" => {
                let get_req = TaskManagerGetTaskReq::from_json(req.params)?;
                let result = self.0.handle_get_task(get_req.id).await?;
                RPCResult::Success(json!(result))
            }
            "list_tasks" => {
                let list_req = TaskManagerListTasksReq::from_json(req.params)?;
                let result = self.0.handle_list_tasks(list_req).await?;
                RPCResult::Success(json!(result))
            }
            "update_task" => {
                let update_req = TaskManagerUpdateTaskReq::from_json(req.params)?;
                let result = self.0.handle_update_task(update_req).await?;
                RPCResult::Success(json!(result))
            }
            "cancel_task" => {
                let cancel_req = TaskManagerCancelTaskReq::from_json(req.params)?;
                let result = self.0.handle_cancel_task(cancel_req).await?;
                RPCResult::Success(json!(result))
            }
            "get_subtasks" => {
                let sub_req = TaskManagerGetSubtasksReq::from_json(req.params)?;
                let result = self.0.handle_get_subtasks(sub_req).await?;
                RPCResult::Success(json!(result))
            }
            "update_task_status" => {
                let update_req = TaskManagerUpdateTaskStatusReq::from_json(req.params)?;
                let result = self.0.handle_update_task_status(update_req).await?;
                RPCResult::Success(json!(result))
            }
            "update_task_progress" => {
                let update_req = TaskManagerUpdateTaskProgressReq::from_json(req.params)?;
                let result = self.0.handle_update_task_progress(update_req).await?;
                RPCResult::Success(json!(result))
            }
            "update_task_error" => {
                let update_req = TaskManagerUpdateTaskErrorReq::from_json(req.params)?;
                let result = self.0.handle_update_task_error(update_req).await?;
                RPCResult::Success(json!(result))
            }
            "update_task_data" => {
                let update_req = TaskManagerUpdateTaskDataReq::from_json(req.params)?;
                let result = self.0.handle_update_task_data(update_req).await?;
                RPCResult::Success(json!(result))
            }
            "delete_task" => {
                let delete_req = TaskManagerDeleteTaskReq::from_json(req.params)?;
                let result = self.0.handle_delete_task(delete_req).await?;
                RPCResult::Success(json!(result))
            }
            _ => return Err(RPCErrors::UnknownMethod(req.method.clone())),
        };

        Ok(RPCResponse { result, seq, trace_id })
    }
}
