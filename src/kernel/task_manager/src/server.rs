use crate::task::{Task, TaskPermissions, TaskScope, TaskStatus};
use crate::task_db::DB_MANAGER;
use buckyos_api::*;
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
use std::ops::Range;
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
            RPCErrors::ParseRequestError(format!(
                "Failed to parse TaskManagerListTasksReq: {}",
                e
            ))
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

fn request_context_from_source(user_id: Option<&str>, app_id: Option<&str>) -> RequestContext {
    RequestContext {
        user_id: user_id.unwrap_or_default().to_string(),
        app_id: app_id.unwrap_or_default().to_string(),
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

    async fn handle_list_tasks_by_time_range(
        &self,
        app_id: Option<&str>,
        task_type: Option<&str>,
        source_user_id: Option<&str>,
        source_app_id: Option<&str>,
        time_range: Range<u64>,
        ctx: RPCContext,
    ) -> Result<Vec<Task>>;

    async fn handle_update_task(
        &self,
        id: i64,
        status: Option<TaskStatus>,
        progress: Option<f32>,
        message: Option<String>,
        data: Option<Value>,
        ctx: RPCContext,
    ) -> Result<()>;

    async fn handle_cancel_task(
        &self,
        id: i64,
        recursive: bool,
        ctx: RPCContext,
    ) -> Result<()>;

    async fn handle_get_subtasks(&self, parent_id: i64, ctx: RPCContext) -> Result<Vec<Task>>;

    async fn handle_update_task_status(
        &self,
        id: i64,
        status: TaskStatus,
        ctx: RPCContext,
    ) -> Result<()>;

    async fn handle_update_task_progress(
        &self,
        id: i64,
        completed_items: u64,
        total_items: u64,
        ctx: RPCContext,
    ) -> Result<()>;

    async fn handle_update_task_error(
        &self,
        id: i64,
        error_message: &str,
        ctx: RPCContext,
    ) -> Result<()>;

    async fn handle_update_task_data(
        &self,
        id: i64,
        data: Value,
        ctx: RPCContext,
    ) -> Result<()>;

    async fn handle_delete_task(&self, id: i64, ctx: RPCContext) -> Result<()>;
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
        name: &str,
        task_type: &str,
        data: Option<Value>,
        opts: CreateTaskOptions,
        user_id: &str,
        app_id: &str,
        _ctx: RPCContext,
    ) -> Result<Task> {
        let request_ctx = request_context_from_source(Some(user_id), Some(app_id));
        let permissions = opts.permissions.unwrap_or_default();
        let data = data.unwrap_or_else(|| json!({}));

        let mut task = Task::new(
            name.to_string(),
            task_type.to_string(),
            request_ctx.user_id.clone(),
            request_ctx.app_id.clone(),
            opts.parent_id,
            permissions,
            data,
        );

        if let Some(parent_id) = task.parent_id {
            let parent = self.load_task(parent_id).await?;
            if !self.can_write_task(&request_ctx, &parent) {
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
        Ok(task)
    }

    async fn handle_get_task(&self, id: i64, _ctx: RPCContext) -> Result<Task> {
        let request_ctx = request_context_from_source(None, None);
        let task = self.load_task(id).await?;
        if !self.can_read_task(&request_ctx, &task) {
            return Err(RPCErrors::NoPermission(
                "No permission to read task".to_string(),
            ));
        }

        Ok(task)
    }

    async fn handle_list_tasks(
        &self,
        filter: TaskFilter,
        source_user_id: Option<&str>,
        source_app_id: Option<&str>,
        _ctx: RPCContext,
    ) -> Result<Vec<Task>> {
        let request_ctx = request_context_from_source(source_user_id, source_app_id);
        let db_manager = DB_MANAGER.lock().await;
        let tasks = db_manager
            .list_tasks_filtered(
                filter.app_id.as_deref(),
                filter.task_type.as_deref(),
                filter.status,
                filter.parent_id,
                filter.root_id,
                None,
            )
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;

        let filtered = tasks
            .into_iter()
            .filter(|task| self.can_read_task(&request_ctx, task))
            .collect();

        Ok(filtered)
    }

    async fn handle_list_tasks_by_time_range(
        &self,
        app_id: Option<&str>,
        task_type: Option<&str>,
        source_user_id: Option<&str>,
        source_app_id: Option<&str>,
        time_range: Range<u64>,
        _ctx: RPCContext,
    ) -> Result<Vec<Task>> {
        let request_ctx = request_context_from_source(source_user_id, source_app_id);
        let db_manager = DB_MANAGER.lock().await;
        let tasks = db_manager
            .list_tasks_filtered(app_id, task_type, None, None, None, None)
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;

        let filtered = tasks
            .into_iter()
            .filter(|task| {
                task.created_at >= time_range.start
                    && task.created_at < time_range.end
                    && self.can_read_task(&request_ctx, task)
            })
            .collect();

        Ok(filtered)
    }

    async fn handle_update_task(
        &self,
        id: i64,
        status: Option<TaskStatus>,
        progress: Option<f32>,
        message: Option<String>,
        data: Option<Value>,
        _ctx: RPCContext,
    ) -> Result<()> {
        let request_ctx = request_context_from_source(None, None);
        let task = self.load_task(id).await?;
        if !self.can_write_task(&request_ctx, &task) {
            return Err(RPCErrors::NoPermission(
                "No permission to update task".to_string(),
            ));
        }

        let db_manager = DB_MANAGER.lock().await;
        db_manager
            .update_task(id, status, progress, message, data)
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        Ok(())
    }

    async fn handle_cancel_task(
        &self,
        id: i64,
        recursive: bool,
        _ctx: RPCContext,
    ) -> Result<()> {
        let request_ctx = request_context_from_source(None, None);
        let task = self.load_task(id).await?;
        if !self.can_write_task(&request_ctx, &task) {
            return Err(RPCErrors::NoPermission(
                "No permission to cancel task".to_string(),
            ));
        }

        let db_manager = DB_MANAGER.lock().await;
        if recursive {
            let root_id = task.root_id.unwrap_or(task.id);
            db_manager
                .update_task_status_by_root_id(root_id, TaskStatus::Canceled)
                .await
                .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        } else {
            db_manager
                .update_task_status(id, TaskStatus::Canceled)
                .await
                .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        }
        Ok(())
    }

    async fn handle_get_subtasks(
        &self,
        parent_id: i64,
        _ctx: RPCContext,
    ) -> Result<Vec<Task>> {
        let request_ctx = request_context_from_source(None, None);
        let db_manager = DB_MANAGER.lock().await;
        let tasks = db_manager
            .list_tasks_filtered(None, None, None, Some(parent_id), None, None)
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;

        let filtered = tasks
            .into_iter()
            .filter(|task| self.can_read_task(&request_ctx, task))
            .collect();
        Ok(filtered)
    }

    async fn handle_update_task_status(
        &self,
        id: i64,
        status: TaskStatus,
        _ctx: RPCContext,
    ) -> Result<()> {
        let request_ctx = request_context_from_source(None, None);
        let task = self.load_task(id).await?;
        if !self.can_write_task(&request_ctx, &task) {
            return Err(RPCErrors::NoPermission(
                "No permission to update task".to_string(),
            ));
        }

        let db_manager = DB_MANAGER.lock().await;
        db_manager
            .update_task_status(id, status)
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        Ok(())
    }

    async fn handle_update_task_progress(
        &self,
        id: i64,
        completed_items: u64,
        total_items: u64,
        _ctx: RPCContext,
    ) -> Result<()> {
        let request_ctx = request_context_from_source(None, None);
        let task = self.load_task(id).await?;
        if !self.can_write_task(&request_ctx, &task) {
            return Err(RPCErrors::NoPermission(
                "No permission to update task".to_string(),
            ));
        }

        let progress = if total_items > 0 {
            (completed_items as f32 / total_items as f32) * 100.0
        } else {
            0.0
        };

        let db_manager = DB_MANAGER.lock().await;
        db_manager
            .update_task_progress(
                id,
                progress,
                completed_items as i32,
                total_items as i32,
            )
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;

        let data_patch = json!({
            "completed_items": completed_items,
            "total_items": total_items
        });
        db_manager
            .update_task(id, None, Some(progress), None, Some(data_patch))
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;

        Ok(())
    }

    async fn handle_update_task_error(
        &self,
        id: i64,
        error_message: &str,
        _ctx: RPCContext,
    ) -> Result<()> {
        let request_ctx = request_context_from_source(None, None);
        let task = self.load_task(id).await?;
        if !self.can_write_task(&request_ctx, &task) {
            return Err(RPCErrors::NoPermission(
                "No permission to update task".to_string(),
            ));
        }

        let db_manager = DB_MANAGER.lock().await;
        db_manager
            .update_task_error(id, error_message)
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        Ok(())
    }

    async fn handle_update_task_data(
        &self,
        id: i64,
        data: Value,
        _ctx: RPCContext,
    ) -> Result<()> {
        let request_ctx = request_context_from_source(None, None);
        let task = self.load_task(id).await?;
        if !self.can_write_task(&request_ctx, &task) {
            return Err(RPCErrors::NoPermission(
                "No permission to update task".to_string(),
            ));
        }

        let data_str = serde_json::to_string(&data)
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        let db_manager = DB_MANAGER.lock().await;
        db_manager
            .update_task_data(id, data_str.as_str())
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        Ok(())
    }

    async fn handle_delete_task(
        &self,
        id: i64,
        _ctx: RPCContext,
    ) -> Result<()> {
        let request_ctx = request_context_from_source(None, None);
        let task = self.load_task(id).await?;
        if !self.can_write_task(&request_ctx, &task) {
            return Err(RPCErrors::NoPermission(
                "No permission to delete task".to_string(),
            ));
        }

        let db_manager = DB_MANAGER.lock().await;
        db_manager
            .delete_task(id)
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
        ip_from: IpAddr,
    ) -> Result<RPCResponse> {
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
                    app_name,
                } = create_req;
                let resolved_app_id = if app_id.is_empty() {
                    app_name.unwrap_or_default()
                } else {
                    app_id
                };
                let opts = CreateTaskOptions {
                    permissions,
                    parent_id,
                    priority,
                };
                let result = self
                    .0
                    .handle_create_task(
                        &name,
                        &task_type,
                        data,
                        opts,
                        user_id.as_str(),
                        resolved_app_id.as_str(),
                        ctx,
                    )
                    .await
                    .map(|task| CreateTaskResult {
                        task_id: task.id,
                        task,
                    });
                Self::to_rpc_result(result)
            }
            "get_task" => {
                let get_req = TaskManagerGetTaskReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_get_task(get_req.id, ctx)
                    .await
                    .map(|task| GetTaskResult { task });
                Self::to_rpc_result(result)
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
                let result = self
                    .0
                    .handle_list_tasks(
                        filter,
                        source_user_id.as_deref(),
                        source_app_id.as_deref(),
                        ctx,
                    )
                    .await
                    .map(|tasks| ListTasksResult { tasks });
                Self::to_rpc_result(result)
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
                let result = self
                    .0
                    .handle_list_tasks_by_time_range(
                        app_id.as_deref(),
                        task_type.as_deref(),
                        source_user_id.as_deref(),
                        source_app_id.as_deref(),
                        time_range,
                        ctx,
                    )
                    .await
                    .map(|tasks| ListTasksResult { tasks });
                Self::to_rpc_result(result)
            }
            "update_task" => {
                let update_req = TaskManagerUpdateTaskReq::from_json(req.params)?;
                Self::to_rpc_result(
                    self.0
                        .handle_update_task(
                            update_req.id,
                            update_req.status,
                            update_req.progress,
                            update_req.message,
                            update_req.data,
                            ctx,
                        )
                        .await,
                )
            }
            "cancel_task" => {
                let cancel_req = TaskManagerCancelTaskReq::from_json(req.params)?;
                Self::to_rpc_result(
                    self.0
                        .handle_cancel_task(cancel_req.id, cancel_req.recursive, ctx)
                        .await,
                )
            }
            "get_subtasks" => {
                let sub_req = TaskManagerGetSubtasksReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_get_subtasks(sub_req.parent_id, ctx)
                    .await
                    .map(|tasks| ListTasksResult { tasks });
                Self::to_rpc_result(result)
            }
            "update_task_status" => {
                let update_req = TaskManagerUpdateTaskStatusReq::from_json(req.params)?;
                Self::to_rpc_result(
                    self.0
                        .handle_update_task_status(update_req.id, update_req.status, ctx)
                        .await,
                )
            }
            "update_task_progress" => {
                let update_req = TaskManagerUpdateTaskProgressReq::from_json(req.params)?;
                Self::to_rpc_result(
                    self.0
                        .handle_update_task_progress(
                            update_req.id,
                            update_req.completed_items,
                            update_req.total_items,
                            ctx,
                        )
                        .await,
                )
            }
            "update_task_error" => {
                let update_req = TaskManagerUpdateTaskErrorReq::from_json(req.params)?;
                Self::to_rpc_result(
                    self.0
                        .handle_update_task_error(update_req.id, update_req.error_message.as_str(), ctx)
                        .await,
                )
            }
            "update_task_data" => {
                let update_req = TaskManagerUpdateTaskDataReq::from_json(req.params)?;
                Self::to_rpc_result(
                    self.0
                        .handle_update_task_data(update_req.id, update_req.data, ctx)
                        .await,
                )
            }
            "delete_task" => {
                let delete_req = TaskManagerDeleteTaskReq::from_json(req.params)?;
                Self::to_rpc_result(
                    self.0.handle_delete_task(delete_req.id, ctx).await,
                )
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

pub async fn start_task_manager_service() -> Result<()> {
    let mut runtime = init_buckyos_api_runtime(
        TASK_MANAGER_SERVICE_NAME,
        None,
        BuckyOSRuntimeType::KernelService,
    )
    .await?;
    if let Err(err) = runtime.login().await {
        error!(
            "task manager service login to system failed! err:{:?}",
            err
        );
        return Err(RPCErrors::ReasonError(format!(
            "task manager login to system failed! err:{:?}",
            err
        )));
    }
    runtime.set_main_service_port(TASK_MANAGER_SERVICE_MAIN_PORT).await;
    set_buckyos_api_runtime(runtime);

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
    Ok(())
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
