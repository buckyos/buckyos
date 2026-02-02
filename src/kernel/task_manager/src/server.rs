use crate::task::{Task, TaskPermissions, TaskScope, TaskStatus};
use crate::task_db::DB_MANAGER;
use ::kRPC::*;
use async_trait::async_trait;
use bytes::Bytes;
use cyfs_gateway_lib::{
    serve_http_by_rpc_handler, server_err, HttpServer, ServerError, ServerErrorCode, ServerResult,
    StreamInfo,
};
use http::{Method, Version};
use http_body_util::combinators::BoxBody;
use log::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use server_runner::*;
use std::net::IpAddr;
use std::sync::Arc;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateTaskOptions {
    pub permissions: Option<TaskPermissions>,
    pub parent_id: Option<i64>,
    pub priority: Option<u8>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskFilter {
    pub app_id: Option<String>,
    pub task_type: Option<String>,
    pub status: Option<TaskStatus>,
    pub parent_id: Option<i64>,
    pub root_id: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

#[derive(Debug, Clone)]
pub struct RequestContext {
    pub user_id: String,
    pub app_id: String,
}

impl RequestContext {
    pub fn empty() -> Self {
        Self {
            user_id: "".to_string(),
            app_id: "".to_string(),
        }
    }
}

fn resolve_context(req: &RPCRequest) -> RequestContext {
    if let Some(token) = req.token.as_deref() {
        if let Ok(session_token) = RPCSessionToken::from_string(token) {
            return RequestContext {
                user_id: session_token.sub.unwrap_or_default(),
                app_id: session_token.appid.unwrap_or_default(),
            };
        }
    }
    RequestContext::empty()
}

#[async_trait]
pub trait TaskManagerHandler: Send + Sync {
    async fn handle_create_task(
        &self,
        ctx: &RequestContext,
        req: TaskManagerCreateTaskReq,
    ) -> Result<CreateTaskResult>;

    async fn handle_get_task(
        &self,
        ctx: &RequestContext,
        id: i64,
    ) -> Result<GetTaskResult>;

    async fn handle_list_tasks(
        &self,
        ctx: &RequestContext,
        req: TaskManagerListTasksReq,
    ) -> Result<ListTasksResult>;

    async fn handle_update_task(
        &self,
        ctx: &RequestContext,
        req: TaskManagerUpdateTaskReq,
    ) -> Result<()>;

    async fn handle_cancel_task(
        &self,
        ctx: &RequestContext,
        req: TaskManagerCancelTaskReq,
    ) -> Result<()>;

    async fn handle_get_subtasks(
        &self,
        ctx: &RequestContext,
        req: TaskManagerGetSubtasksReq,
    ) -> Result<ListTasksResult>;

    async fn handle_update_task_status(
        &self,
        ctx: &RequestContext,
        req: TaskManagerUpdateTaskStatusReq,
    ) -> Result<()>;

    async fn handle_update_task_progress(
        &self,
        ctx: &RequestContext,
        req: TaskManagerUpdateTaskProgressReq,
    ) -> Result<()>;

    async fn handle_update_task_error(
        &self,
        ctx: &RequestContext,
        req: TaskManagerUpdateTaskErrorReq,
    ) -> Result<()>;

    async fn handle_update_task_data(
        &self,
        ctx: &RequestContext,
        req: TaskManagerUpdateTaskDataReq,
    ) -> Result<()>;

    async fn handle_delete_task(
        &self,
        ctx: &RequestContext,
        req: TaskManagerDeleteTaskReq,
    ) -> Result<()>;
}

#[derive(Clone)]
struct TaskManagerService {}

impl TaskManagerService {
    pub fn new() -> Self {
        TaskManagerService {}
    }

    fn is_system_app(app_id: &str) -> bool {
        app_id == "kernel" || app_id == "system"
    }

    fn can_read_task(&self, ctx: &RequestContext, task: &Task) -> bool {
        if ctx.user_id.is_empty() && ctx.app_id.is_empty() {
            return true;
        }
        if task.user_id.is_empty() {
            return task.app_id.is_empty() || task.app_id == ctx.app_id;
        }

        match task.permissions.read {
            TaskScope::Private => task.user_id == ctx.user_id && task.app_id == ctx.app_id,
            TaskScope::User => task.user_id == ctx.user_id,
            TaskScope::System => Self::is_system_app(ctx.app_id.as_str()),
        }
    }

    fn can_write_task(&self, ctx: &RequestContext, task: &Task) -> bool {
        if ctx.user_id.is_empty() && ctx.app_id.is_empty() {
            return true;
        }
        if task.user_id.is_empty() {
            return task.app_id.is_empty() || task.app_id == ctx.app_id;
        }

        match task.permissions.write {
            TaskScope::Private => task.user_id == ctx.user_id && task.app_id == ctx.app_id,
            TaskScope::User => task.user_id == ctx.user_id,
            TaskScope::System => Self::is_system_app(ctx.app_id.as_str()),
        }
    }

    async fn load_task(&self, id: i64) -> Result<Task> {
        let db_manager = DB_MANAGER.lock().await;
        let task = db_manager
            .get_task(id)
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        task.ok_or_else(|| RPCErrors::ReasonError(format!("Task {} not found", id)))
    }
}

#[async_trait]
impl TaskManagerHandler for TaskManagerService {
    async fn handle_create_task(
        &self,
        ctx: &RequestContext,
        req: TaskManagerCreateTaskReq,
    ) -> Result<CreateTaskResult> {
        let user_id = ctx.user_id.clone();
        let app_id = if ctx.app_id.is_empty() {
            req.app_name.clone().unwrap_or_default()
        } else {
            ctx.app_id.clone()
        };

        let permissions = req.permissions.unwrap_or_default();
        let data = req.data.unwrap_or_else(|| json!({}));

        let mut task = Task::new(
            req.name,
            req.task_type,
            user_id,
            app_id,
            req.parent_id,
            permissions,
            data,
        );

        if let Some(parent_id) = task.parent_id {
            let parent = self.load_task(parent_id).await?;
            if !self.can_write_task(ctx, &parent) {
                return Err(RPCErrors::NoPermission(
                    "No permission to create subtasks".to_string(),
                ));
            }
            task.root_id = parent.root_id.or(Some(parent.id));
        }

        let db_manager = DB_MANAGER.lock().await;
        let task_id = db_manager
            .create_task(&task)
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;

        if task.root_id.is_none() {
            db_manager
                .set_root_id(task_id, task_id)
                .await
                .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
            task.root_id = Some(task_id);
        }

        task.id = task_id;
        Ok(CreateTaskResult { task_id, task })
    }

    async fn handle_get_task(
        &self,
        ctx: &RequestContext,
        id: i64,
    ) -> Result<GetTaskResult> {
        let task = self.load_task(id).await?;
        if !self.can_read_task(ctx, &task) {
            return Err(RPCErrors::NoPermission(
                "No permission to read task".to_string(),
            ));
        }

        Ok(GetTaskResult { task })
    }

    async fn handle_list_tasks(
        &self,
        ctx: &RequestContext,
        req: TaskManagerListTasksReq,
    ) -> Result<ListTasksResult> {
        let db_manager = DB_MANAGER.lock().await;
        let tasks = db_manager
            .list_tasks_filtered(
                req.app_id.as_deref(),
                req.task_type.as_deref(),
                req.status,
                req.parent_id,
                req.root_id,
                None,
            )
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;

        let filtered = tasks
            .into_iter()
            .filter(|task| self.can_read_task(ctx, task))
            .collect();

        Ok(ListTasksResult { tasks: filtered })
    }

    async fn handle_update_task(
        &self,
        ctx: &RequestContext,
        req: TaskManagerUpdateTaskReq,
    ) -> Result<()> {
        let task = self.load_task(req.id).await?;
        if !self.can_write_task(ctx, &task) {
            return Err(RPCErrors::NoPermission(
                "No permission to update task".to_string(),
            ));
        }

        let db_manager = DB_MANAGER.lock().await;
        db_manager
            .update_task(req.id, req.status, req.progress, req.message, req.data)
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        Ok(())
    }

    async fn handle_cancel_task(
        &self,
        ctx: &RequestContext,
        req: TaskManagerCancelTaskReq,
    ) -> Result<()> {
        let task = self.load_task(req.id).await?;
        if !self.can_write_task(ctx, &task) {
            return Err(RPCErrors::NoPermission(
                "No permission to cancel task".to_string(),
            ));
        }

        let db_manager = DB_MANAGER.lock().await;
        if req.recursive {
            let root_id = task.root_id.unwrap_or(task.id);
            db_manager
                .update_task_status_by_root_id(root_id, TaskStatus::Canceled)
                .await
                .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        } else {
            db_manager
                .update_task_status(req.id, TaskStatus::Canceled)
                .await
                .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        }
        Ok(())
    }

    async fn handle_get_subtasks(
        &self,
        ctx: &RequestContext,
        req: TaskManagerGetSubtasksReq,
    ) -> Result<ListTasksResult> {
        let db_manager = DB_MANAGER.lock().await;
        let tasks = db_manager
            .list_tasks_filtered(None, None, None, Some(req.parent_id), None, None)
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;

        let filtered = tasks
            .into_iter()
            .filter(|task| self.can_read_task(ctx, task))
            .collect();
        Ok(ListTasksResult { tasks: filtered })
    }

    async fn handle_update_task_status(
        &self,
        ctx: &RequestContext,
        req: TaskManagerUpdateTaskStatusReq,
    ) -> Result<()> {
        let task = self.load_task(req.id).await?;
        if !self.can_write_task(ctx, &task) {
            return Err(RPCErrors::NoPermission(
                "No permission to update task".to_string(),
            ));
        }

        let db_manager = DB_MANAGER.lock().await;
        db_manager
            .update_task_status(req.id, req.status)
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        Ok(())
    }

    async fn handle_update_task_progress(
        &self,
        ctx: &RequestContext,
        req: TaskManagerUpdateTaskProgressReq,
    ) -> Result<()> {
        let task = self.load_task(req.id).await?;
        if !self.can_write_task(ctx, &task) {
            return Err(RPCErrors::NoPermission(
                "No permission to update task".to_string(),
            ));
        }

        let progress = if req.total_items > 0 {
            (req.completed_items as f32 / req.total_items as f32) * 100.0
        } else {
            0.0
        };

        let db_manager = DB_MANAGER.lock().await;
        db_manager
            .update_task_progress(
                req.id,
                progress,
                req.completed_items as i32,
                req.total_items as i32,
            )
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;

        let data_patch = json!({
            "completed_items": req.completed_items,
            "total_items": req.total_items
        });
        db_manager
            .update_task(req.id, None, Some(progress), None, Some(data_patch))
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;

        Ok(())
    }

    async fn handle_update_task_error(
        &self,
        ctx: &RequestContext,
        req: TaskManagerUpdateTaskErrorReq,
    ) -> Result<()> {
        let task = self.load_task(req.id).await?;
        if !self.can_write_task(ctx, &task) {
            return Err(RPCErrors::NoPermission(
                "No permission to update task".to_string(),
            ));
        }

        let db_manager = DB_MANAGER.lock().await;
        db_manager
            .update_task_error(req.id, req.error_message.as_str())
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        Ok(())
    }

    async fn handle_update_task_data(
        &self,
        ctx: &RequestContext,
        req: TaskManagerUpdateTaskDataReq,
    ) -> Result<()> {
        let task = self.load_task(req.id).await?;
        if !self.can_write_task(ctx, &task) {
            return Err(RPCErrors::NoPermission(
                "No permission to update task".to_string(),
            ));
        }

        let data_str = serde_json::to_string(&req.data)
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        let db_manager = DB_MANAGER.lock().await;
        db_manager
            .update_task_data(req.id, data_str.as_str())
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        Ok(())
    }

    async fn handle_delete_task(
        &self,
        ctx: &RequestContext,
        req: TaskManagerDeleteTaskReq,
    ) -> Result<()> {
        let task = self.load_task(req.id).await?;
        if !self.can_write_task(ctx, &task) {
            return Err(RPCErrors::NoPermission(
                "No permission to delete task".to_string(),
            ));
        }

        let db_manager = DB_MANAGER.lock().await;
        db_manager
            .delete_task(req.id)
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        Ok(())
    }
}

pub struct TaskManagerServerHandler<T: TaskManagerHandler>(pub T);

impl<T: TaskManagerHandler> TaskManagerServerHandler<T> {
    pub fn new(handler: T) -> Self {
        Self(handler)
    }

    fn to_rpc_result<R: Serialize>(res: Result<R>) -> RPCResult {
        match res {
            Ok(value) => RPCResult::Success(json!(value)),
            Err(err) => RPCResult::Failed(err.to_string()),
        }
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
        let ctx = resolve_context(&req);

        let result = match req.method.as_str() {
            "create_task" => {
                let create_req = TaskManagerCreateTaskReq::from_json(req.params)?;
                Self::to_rpc_result(self.0.handle_create_task(&ctx, create_req).await)
            }
            "get_task" => {
                let get_req = TaskManagerGetTaskReq::from_json(req.params)?;
                Self::to_rpc_result(self.0.handle_get_task(&ctx, get_req.id).await)
            }
            "list_tasks" => {
                let list_req = TaskManagerListTasksReq::from_json(req.params)?;
                Self::to_rpc_result(self.0.handle_list_tasks(&ctx, list_req).await)
            }
            "update_task" => {
                let update_req = TaskManagerUpdateTaskReq::from_json(req.params)?;
                Self::to_rpc_result(self.0.handle_update_task(&ctx, update_req).await)
            }
            "cancel_task" => {
                let cancel_req = TaskManagerCancelTaskReq::from_json(req.params)?;
                Self::to_rpc_result(self.0.handle_cancel_task(&ctx, cancel_req).await)
            }
            "get_subtasks" => {
                let sub_req = TaskManagerGetSubtasksReq::from_json(req.params)?;
                Self::to_rpc_result(self.0.handle_get_subtasks(&ctx, sub_req).await)
            }
            "update_task_status" => {
                let update_req = TaskManagerUpdateTaskStatusReq::from_json(req.params)?;
                Self::to_rpc_result(self.0.handle_update_task_status(&ctx, update_req).await)
            }
            "update_task_progress" => {
                let update_req = TaskManagerUpdateTaskProgressReq::from_json(req.params)?;
                Self::to_rpc_result(self.0.handle_update_task_progress(&ctx, update_req).await)
            }
            "update_task_error" => {
                let update_req = TaskManagerUpdateTaskErrorReq::from_json(req.params)?;
                Self::to_rpc_result(self.0.handle_update_task_error(&ctx, update_req).await)
            }
            "update_task_data" => {
                let update_req = TaskManagerUpdateTaskDataReq::from_json(req.params)?;
                Self::to_rpc_result(self.0.handle_update_task_data(&ctx, update_req).await)
            }
            "delete_task" => {
                let delete_req = TaskManagerDeleteTaskReq::from_json(req.params)?;
                Self::to_rpc_result(self.0.handle_delete_task(&ctx, delete_req).await)
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

#[async_trait]
impl<T: TaskManagerHandler + 'static> HttpServer for TaskManagerServerHandler<T> {
    async fn serve_request(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        info: StreamInfo,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        if *req.method() == Method::POST {
            return serve_http_by_rpc_handler(req, info, self).await;
        }
        Err(server_err!(ServerErrorCode::BadRequest, "Method not allowed"))
    }

    fn id(&self) -> String {
        "task-manager-server".to_string()
    }

    fn http_version(&self) -> Version {
        Version::HTTP_11
    }

    fn http3_port(&self) -> Option<u16> {
        None
    }
}

pub async fn start_task_manager_service() {
    let handler = TaskManagerService::new();
    let server = TaskManagerServerHandler::new(handler);

    info!("start node task manager service...");
    const TASK_MANAGER_SERVICE_MAIN_PORT: u16 = 3380;
    let runner = Runner::new(TASK_MANAGER_SERVICE_MAIN_PORT);
    if let Err(err) = runner.add_http_server("/kapi/task-manager".to_string(), Arc::new(server)) {
        error!("failed to add task manager http server: {:?}", err);
    }
    if let Err(err) = runner.run().await {
        error!("task manager runner exited with error: {:?}", err);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_db::TaskDb;
    use serde_json::json;
    use std::net::IpAddr;
    use std::str::FromStr;
    use std::sync::Once;
    use tempfile::tempdir;
    use tokio::sync::{Mutex as AsyncMutex, MutexGuard};

    lazy_static::lazy_static! {
        static ref TEST_MUTEX: AsyncMutex<()> = AsyncMutex::new(());
    }
    static INIT_LOGGING: Once = Once::new();

    fn create_rpc_request(method: &str, params: Value) -> RPCRequest {
        RPCRequest {
            method: method.to_string(),
            params,
            seq: 1,
            token: Some("".to_string()),
            trace_id: Some("".to_string()),
        }
    }

    async fn setup_test_environment(
    ) -> (
        TaskManagerServerHandler<TaskManagerService>,
        tempfile::TempDir,
        MutexGuard<'static, ()>,
    ) {
        let guard = TEST_MUTEX.lock().await;
        INIT_LOGGING.call_once(|| {
            buckyos_kit::init_logging("test_task_manager", false);
        });
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let db_path_str = db_path.to_str().unwrap();

        let mut db = TaskDb::new();
        db.connect(db_path_str).unwrap();
        db.init_db().await.unwrap();
        *crate::task_db::DB_MANAGER.lock().await = db;

        let server = TaskManagerServerHandler::new(TaskManagerService::new());
        (server, temp_dir, guard)
    }

    async fn clean_test_environment(_temp_dir: tempfile::TempDir) {}

    #[tokio::test(flavor = "current_thread")]
    async fn test_create_and_get_task() {
        let (server, _temp_dir, _guard) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();

        let create_params = json!({
            "name": "test_task",
            "task_type": "test_type",
            "app_name": "test_app",
            "data": {"key": "value"}
        });

        let create_req = create_rpc_request("create_task", create_params);
        let create_resp = server.handle_rpc_call(create_req, ip).await.unwrap();

        if let RPCResult::Success(result) = create_resp.result {
            let task_id = result["task_id"].as_i64().unwrap();
            assert!(task_id > 0);

            let get_params = json!({
                "id": task_id
            });

            let get_req = create_rpc_request("get_task", get_params);
            let get_resp = server.handle_rpc_call(get_req, ip).await.unwrap();

            if let RPCResult::Success(result) = get_resp.result {
                assert_eq!(result["task"]["name"], "test_task");
                assert_eq!(result["task"]["task_type"], "test_type");
                assert_eq!(result["task"]["app_id"], "test_app");
            } else {
                panic!("Failed to get task");
            }
        } else {
            panic!("Failed to create task");
        }
        clean_test_environment(_temp_dir).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_list_tasks() {
        let (server, _temp_dir, _guard) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();

        for i in 1..4 {
            let create_params = json!({
                "name": format!("test_task_{}", i),
                "task_type": "test_type",
                "app_name": "test_app"
            });

            let create_req = create_rpc_request("create_task", create_params);
            let _ = server.handle_rpc_call(create_req, ip).await.unwrap();
        }

        let list_req = create_rpc_request("list_tasks", json!({}));
        let list_resp = server.handle_rpc_call(list_req, ip).await.unwrap();

        if let RPCResult::Success(result) = list_resp.result {
            let tasks = result["tasks"].as_array().unwrap();
            assert_eq!(tasks.len(), 3);
        } else {
            panic!("Failed to list tasks");
        }
        clean_test_environment(_temp_dir).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_list_tasks_by_app() {
        let (server, _temp_dir, _guard) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();

        let create_params1 = json!({
            "name": "app1_task",
            "task_type": "test_type",
            "app_name": "app1"
        });

        let create_params2 = json!({
            "name": "app2_task",
            "task_type": "test_type",
            "app_name": "app2"
        });

        let create_req1 = create_rpc_request("create_task", create_params1);
        let create_req2 = create_rpc_request("create_task", create_params2);
        let _ = server.handle_rpc_call(create_req1, ip).await.unwrap();
        let _ = server.handle_rpc_call(create_req2, ip).await.unwrap();

        let list_params = json!({
            "app_id": "app1"
        });

        let list_req = create_rpc_request("list_tasks", list_params);
        let list_resp = server.handle_rpc_call(list_req, ip).await.unwrap();

        if let RPCResult::Success(result) = list_resp.result {
            let tasks = result["tasks"].as_array().unwrap();
            assert_eq!(tasks.len(), 1);
            assert_eq!(tasks[0]["app_id"], "app1");
        } else {
            panic!("Failed to list tasks by app");
        }
        clean_test_environment(_temp_dir).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_update_task_status() {
        let (server, _temp_dir, _guard) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();

        let create_params = json!({
            "name": "status_test",
            "task_type": "test_type",
            "app_name": "test_app"
        });

        let create_req = create_rpc_request("create_task", create_params);
        let create_resp = server.handle_rpc_call(create_req, ip).await.unwrap();

        let task_id = if let RPCResult::Success(result) = create_resp.result {
            result["task_id"].as_i64().unwrap()
        } else {
            panic!("Failed to create task");
        };

        let update_params = json!({
            "id": task_id,
            "status": "Running"
        });

        let update_req = create_rpc_request("update_task_status", update_params);
        let update_resp = server.handle_rpc_call(update_req, ip).await.unwrap();

        if let RPCResult::Success(_) = update_resp.result {
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
        clean_test_environment(_temp_dir).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_update_task_progress() {
        let (server, _temp_dir, _guard) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();

        let create_params = json!({
            "name": "progress_test",
            "task_type": "test_type",
            "app_name": "test_app"
        });

        let create_req = create_rpc_request("create_task", create_params);
        let create_resp = server.handle_rpc_call(create_req, ip).await.unwrap();

        let task_id = if let RPCResult::Success(result) = create_resp.result {
            result["task_id"].as_i64().unwrap()
        } else {
            panic!("Failed to create task");
        };

        let update_params = json!({
            "id": task_id,
            "completed_items": 5,
            "total_items": 10
        });

        let update_req = create_rpc_request("update_task_progress", update_params);
        let update_resp = server.handle_rpc_call(update_req, ip).await.unwrap();

        if let RPCResult::Success(_) = update_resp.result {
            let get_params = json!({
                "id": task_id
            });

            let get_req = create_rpc_request("get_task", get_params);
            let get_resp = server.handle_rpc_call(get_req, ip).await.unwrap();

            if let RPCResult::Success(result) = get_resp.result {
                assert_eq!(result["task"]["progress"], 50.0);
                assert_eq!(result["task"]["data"]["completed_items"], 5);
                assert_eq!(result["task"]["data"]["total_items"], 10);
            } else {
                panic!("Failed to get task after progress update");
            }
        } else {
            panic!("Failed to update task progress");
        }
        clean_test_environment(_temp_dir).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_update_task_error() {
        let (server, _temp_dir, _guard) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();

        let create_params = json!({
            "name": "error_test",
            "task_type": "test_type",
            "app_name": "test_app"
        });

        let create_req = create_rpc_request("create_task", create_params);
        let create_resp = server.handle_rpc_call(create_req, ip).await.unwrap();

        let task_id = if let RPCResult::Success(result) = create_resp.result {
            result["task_id"].as_i64().unwrap()
        } else {
            panic!("Failed to create task");
        };

        let update_params = json!({
            "id": task_id,
            "error_message": "Test error occurred"
        });

        let update_req = create_rpc_request("update_task_error", update_params);
        let update_resp = server.handle_rpc_call(update_req, ip).await.unwrap();

        if let RPCResult::Success(_) = update_resp.result {
            let get_params = json!({
                "id": task_id
            });

            let get_req = create_rpc_request("get_task", get_params);
            let get_resp = server.handle_rpc_call(get_req, ip).await.unwrap();

            if let RPCResult::Success(result) = get_resp.result {
                assert_eq!(result["task"]["message"], "Test error occurred");
                assert_eq!(result["task"]["status"], "Failed");
            } else {
                panic!("Failed to get task after error update");
            }
        } else {
            panic!("Failed to update task error");
        }
        clean_test_environment(_temp_dir).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_update_task_data() {
        let (server, _temp_dir, _guard) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();

        let create_params = json!({
            "name": "data_test",
            "task_type": "test_type",
            "app_name": "test_app"
        });

        let create_req = create_rpc_request("create_task", create_params);
        let create_resp = server.handle_rpc_call(create_req, ip).await.unwrap();

        let task_id = if let RPCResult::Success(result) = create_resp.result {
            result["task_id"].as_i64().unwrap()
        } else {
            panic!("Failed to create task");
        };

        let update_params = json!({
            "id": task_id,
            "data": {"updated": true, "value": "new data"}
        });

        let update_req = create_rpc_request("update_task_data", update_params);
        let update_resp = server.handle_rpc_call(update_req, ip).await.unwrap();

        if let RPCResult::Success(_) = update_resp.result {
            let get_params = json!({
                "id": task_id
            });

            let get_req = create_rpc_request("get_task", get_params);
            let get_resp = server.handle_rpc_call(get_req, ip).await.unwrap();

            if let RPCResult::Success(result) = get_resp.result {
                assert_eq!(result["task"]["data"]["updated"], true);
                assert_eq!(result["task"]["data"]["value"], "new data");
            } else {
                panic!("Failed to get task after data update");
            }
        } else {
            panic!("Failed to update task data");
        }
        clean_test_environment(_temp_dir).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_delete_task() {
        let (server, _temp_dir, _guard) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();

        let create_params = json!({
            "name": "delete_test",
            "task_type": "test_type",
            "app_name": "test_app"
        });

        let create_req = create_rpc_request("create_task", create_params);
        let create_resp = server.handle_rpc_call(create_req, ip).await.unwrap();

        let task_id = if let RPCResult::Success(result) = create_resp.result {
            result["task_id"].as_i64().unwrap()
        } else {
            panic!("Failed to create task");
        };

        let delete_params = json!({
            "id": task_id
        });

        let delete_req = create_rpc_request("delete_task", delete_params);
        let delete_resp = server.handle_rpc_call(delete_req, ip).await.unwrap();

        if let RPCResult::Success(_) = delete_resp.result {
            let get_params = json!({
                "id": task_id
            });

            let get_req = create_rpc_request("get_task", get_params);
            let get_resp = server.handle_rpc_call(get_req, ip).await.unwrap();

            if let RPCResult::Success(_) = get_resp.result {
                panic!("Unexpected success when getting deleted task");
            }
        } else {
            panic!("Failed to delete task");
        }
        clean_test_environment(_temp_dir).await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_invalid_method() {
        let (server, _temp_dir, _guard) = setup_test_environment().await;
        let ip = IpAddr::from_str("127.0.0.1").unwrap();

        let req = create_rpc_request("invalid_method", json!({}));
        let result = server.handle_rpc_call(req, ip).await;

        assert!(matches!(result, Err(RPCErrors::UnknownMethod(_))));
        clean_test_environment(_temp_dir).await;
    }
}
