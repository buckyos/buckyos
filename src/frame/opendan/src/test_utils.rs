use std::collections::{HashMap, VecDeque};
use std::ops::Range;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use buckyos_api::{
    AiccHandler, CancelResponse, CompleteRequest, CompleteResponse, CreateTaskOptions, Task,
    TaskFilter, TaskManagerHandler, TaskPermissions, TaskStatus,
};
use kRPC::{RPCContext, RPCErrors, Result as KRPCResult};
use serde_json::{json, Value as Json};

pub struct MockTaskMgrHandler {
    pub counter: Mutex<u64>,
    pub tasks: Arc<Mutex<HashMap<i64, Task>>>,
}

#[async_trait]
impl TaskManagerHandler for MockTaskMgrHandler {
    async fn handle_create_task(
        &self,
        name: &str,
        task_type: &str,
        data: Option<Json>,
        opts: CreateTaskOptions,
        user_id: &str,
        app_id: &str,
        _ctx: RPCContext,
    ) -> KRPCResult<Task> {
        let mut guard = self.counter.lock().expect("counter lock");
        *guard += 1;
        let now = now_ms();
        let task = Task {
            id: *guard as i64,
            user_id: user_id.to_string(),
            app_id: app_id.to_string(),
            parent_id: opts.parent_id,
            root_id: String::new(),
            name: name.to_string(),
            task_type: task_type.to_string(),
            status: TaskStatus::Pending,
            progress: 0.0,
            message: None,
            data: data.unwrap_or_else(|| json!({})),
            permissions: opts.permissions.unwrap_or(TaskPermissions::default()),
            created_at: now,
            updated_at: now,
        };
        self.tasks
            .lock()
            .expect("tasks lock")
            .insert(task.id, task.clone());
        Ok(task)
    }

    async fn handle_get_task(&self, id: i64, _ctx: RPCContext) -> KRPCResult<Task> {
        self.tasks
            .lock()
            .expect("tasks lock")
            .get(&id)
            .cloned()
            .ok_or_else(|| RPCErrors::ReasonError(format!("mock task {} not found", id)))
    }

    async fn handle_list_tasks(
        &self,
        _filter: TaskFilter,
        _source_user_id: Option<&str>,
        _source_app_id: Option<&str>,
        _ctx: RPCContext,
    ) -> KRPCResult<Vec<Task>> {
        let tasks = self
            .tasks
            .lock()
            .expect("tasks lock")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        Ok(tasks)
    }

    async fn handle_list_tasks_by_time_range(
        &self,
        _app_id: Option<&str>,
        _task_type: Option<&str>,
        _source_user_id: Option<&str>,
        _source_app_id: Option<&str>,
        _time_range: Range<u64>,
        _ctx: RPCContext,
    ) -> KRPCResult<Vec<Task>> {
        Ok(vec![])
    }

    async fn handle_get_subtasks(
        &self,
        _parent_id: i64,
        _ctx: RPCContext,
    ) -> KRPCResult<Vec<Task>> {
        Ok(vec![])
    }

    async fn handle_update_task(
        &self,
        id: i64,
        status: Option<TaskStatus>,
        progress: Option<f32>,
        message: Option<String>,
        data: Option<Json>,
        _ctx: RPCContext,
    ) -> KRPCResult<()> {
        if let Some(task) = self.tasks.lock().expect("tasks lock").get_mut(&id) {
            if let Some(s) = status {
                task.status = s;
            }
            if let Some(p) = progress {
                task.progress = p;
            }
            task.message = message;
            if let Some(patch) = data {
                task.data = patch;
            }
        }
        Ok(())
    }

    async fn handle_update_task_progress(
        &self,
        id: i64,
        completed_items: u64,
        total_items: u64,
        _ctx: RPCContext,
    ) -> KRPCResult<()> {
        if let Some(task) = self.tasks.lock().expect("tasks lock").get_mut(&id) {
            if total_items > 0 {
                task.progress = (completed_items as f32 / total_items as f32).clamp(0.0, 1.0);
            }
        }
        Ok(())
    }

    async fn handle_update_task_status(
        &self,
        id: i64,
        status: TaskStatus,
        _ctx: RPCContext,
    ) -> KRPCResult<()> {
        if let Some(task) = self.tasks.lock().expect("tasks lock").get_mut(&id) {
            task.status = status;
        }
        Ok(())
    }

    async fn handle_update_task_error(
        &self,
        id: i64,
        error_message: &str,
        _ctx: RPCContext,
    ) -> KRPCResult<()> {
        if let Some(task) = self.tasks.lock().expect("tasks lock").get_mut(&id) {
            task.status = TaskStatus::Failed;
            task.message = Some(error_message.to_string());
        }
        Ok(())
    }

    async fn handle_update_task_data(
        &self,
        id: i64,
        data: Json,
        _ctx: RPCContext,
    ) -> KRPCResult<()> {
        if let Some(task) = self.tasks.lock().expect("tasks lock").get_mut(&id) {
            task.data = data;
        }
        Ok(())
    }

    async fn handle_cancel_task(
        &self,
        _id: i64,
        _recursive: bool,
        _ctx: RPCContext,
    ) -> KRPCResult<()> {
        Ok(())
    }

    async fn handle_delete_task(&self, _id: i64, _ctx: RPCContext) -> KRPCResult<()> {
        Ok(())
    }
}

pub struct MockAicc {
    pub responses: Arc<Mutex<VecDeque<CompleteResponse>>>,
    pub requests: Arc<Mutex<Vec<CompleteRequest>>>,
}

#[async_trait]
impl AiccHandler for MockAicc {
    async fn handle_complete(
        &self,
        request: CompleteRequest,
        _ctx: RPCContext,
    ) -> KRPCResult<CompleteResponse> {
        self.requests.lock().expect("requests lock").push(request);
        self.responses
            .lock()
            .expect("responses lock")
            .pop_front()
            .ok_or_else(|| RPCErrors::ReasonError("no response queued".to_string()))
    }

    async fn handle_cancel(&self, task_id: &str, _ctx: RPCContext) -> KRPCResult<CancelResponse> {
        Ok(CancelResponse::new(task_id.to_string(), true))
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
