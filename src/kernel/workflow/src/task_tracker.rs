//! 把 Workflow Run / Step / Map shard / Thunk 的执行单元同步为
//! task_manager 的任务树（详见 doc/workflow/workflow service.md §6.3）。
//!
//! 设计原则：
//! - workflow 写"语义性"字段（哪个节点、第几 attempt、subject 引用、prompt、
//!   output_schema、stakeholders、waiting_human_since）和自己负责镜像的
//!   status（Run / Step / Map shard）。
//! - scheduler 在 Thunk task 上覆盖 status / progress / payload / error，
//!   workflow 只负责 Thunk task 的创建（落 ACL）。
//! - 用户写 TaskData 中的 `human_action`，由 orchestrator 的 apply_task_data
//!   解释；tracker 只负责在校验失败时把 `last_error` 回写到 TaskData。

use crate::error::{WorkflowError, WorkflowResult};
use crate::runtime::{NodeRunState, RunStatus, WorkflowRun};
use async_trait::async_trait;
use buckyos_api::{CreateTaskOptions, TaskManagerClient, TaskStatus};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// 一个 Step（含 control node）当前应该呈现给 task_manager 的视图。
/// 由 orchestrator 在每次状态机转换后构造，传给 tracker。
#[derive(Debug, Clone)]
pub struct StepTaskView {
    pub node_id: String,
    pub name: String,
    pub state: NodeRunState,
    pub attempt: u32,
    pub executor: Option<String>,
    /// 仅 human_confirm 等节点会带；其它节点为 None。
    pub subject: Option<Value>,
    pub subject_obj_id: Option<String>,
    pub prompt: Option<String>,
    pub output_schema: Option<Value>,
    pub stakeholders: Vec<String>,
    pub progress_message: Option<String>,
    pub error: Option<String>,
    pub output: Option<Value>,
    /// 进入 WaitingHuman 时的时间戳（UNIX 秒）。
    pub waiting_human_since: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct MapShardTaskView {
    pub for_each_id: String,
    pub shard_index: u32,
    pub state: NodeRunState,
    pub attempt: u32,
    pub item: Value,
    pub output: Option<Value>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ThunkTaskView {
    pub node_id: String,
    pub thunk_obj_id: String,
    pub attempt: u32,
    pub shard_index: Option<u32>,
}

#[async_trait]
pub trait WorkflowTaskTracker: Send + Sync {
    /// Run 整体状态同步到 root task。
    async fn sync_run(&self, run: &WorkflowRun) -> WorkflowResult<()>;

    /// Step / control-node 状态同步到 Run task 下的 step 子任务。
    async fn sync_step(&self, run: &WorkflowRun, step: &StepTaskView) -> WorkflowResult<()> {
        let _ = (run, step);
        Ok(())
    }

    /// for_each shard 状态同步到对应 Step task 下的 map_shard 子任务。
    async fn sync_map_shard(
        &self,
        run: &WorkflowRun,
        shard: &MapShardTaskView,
    ) -> WorkflowResult<()> {
        let _ = (run, shard);
        Ok(())
    }

    /// 投递给 scheduler 的 thunk 子任务（仅创建 + 写描述性字段，scheduler 后续
    /// 覆盖 status / progress / payload）。
    async fn sync_thunk(&self, run: &WorkflowRun, thunk: &ThunkTaskView) -> WorkflowResult<()> {
        let _ = (run, thunk);
        Ok(())
    }

    /// 校验失败时把原因写回 Step task 的 TaskData.last_error，让 TaskMgr UI 重新
    /// 提示用户修正。task 状态不变（仍是 WaitingForApproval）。
    async fn report_step_validation_error(
        &self,
        run: &WorkflowRun,
        node_id: &str,
        message: &str,
    ) -> WorkflowResult<()> {
        let _ = (run, node_id, message);
        Ok(())
    }
}

#[derive(Debug, Default, Clone)]
pub struct NoopTaskTracker;

#[async_trait]
impl WorkflowTaskTracker for NoopTaskTracker {
    async fn sync_run(&self, _run: &WorkflowRun) -> WorkflowResult<()> {
        Ok(())
    }
}

/// 测试用：在内存里保留所有 sync 调用，方便断言 task_manager 视图。
#[derive(Debug, Default)]
pub struct RecordingTaskTracker {
    inner: Mutex<RecordingState>,
}

#[derive(Debug, Default)]
struct RecordingState {
    pub runs: Vec<WorkflowRun>,
    pub steps: HashMap<(String, String), StepTaskView>,
    pub step_history: Vec<(String, StepTaskView)>,
    pub map_shards: HashMap<(String, String, u32), MapShardTaskView>,
    pub thunks: HashMap<(String, String), ThunkTaskView>,
    pub validation_errors: Vec<(String, String, String)>,
}

impl RecordingTaskTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn step(&self, run_id: &str, node_id: &str) -> Option<StepTaskView> {
        self.inner
            .lock()
            .await
            .steps
            .get(&(run_id.to_string(), node_id.to_string()))
            .cloned()
    }

    pub async fn step_history(&self, node_id: &str) -> Vec<StepTaskView> {
        self.inner
            .lock()
            .await
            .step_history
            .iter()
            .filter(|(id, _)| id == node_id)
            .map(|(_, view)| view.clone())
            .collect()
    }

    pub async fn map_shard(
        &self,
        run_id: &str,
        for_each_id: &str,
        shard_index: u32,
    ) -> Option<MapShardTaskView> {
        self.inner
            .lock()
            .await
            .map_shards
            .get(&(run_id.to_string(), for_each_id.to_string(), shard_index))
            .cloned()
    }

    pub async fn thunk(&self, run_id: &str, thunk_obj_id: &str) -> Option<ThunkTaskView> {
        self.inner
            .lock()
            .await
            .thunks
            .get(&(run_id.to_string(), thunk_obj_id.to_string()))
            .cloned()
    }

    pub async fn validation_errors(&self, node_id: &str) -> Vec<String> {
        self.inner
            .lock()
            .await
            .validation_errors
            .iter()
            .filter(|(_, n, _)| n == node_id)
            .map(|(_, _, msg)| msg.clone())
            .collect()
    }
}

#[async_trait]
impl WorkflowTaskTracker for RecordingTaskTracker {
    async fn sync_run(&self, run: &WorkflowRun) -> WorkflowResult<()> {
        let mut guard = self.inner.lock().await;
        guard.runs.push(run.clone());
        Ok(())
    }

    async fn sync_step(&self, run: &WorkflowRun, step: &StepTaskView) -> WorkflowResult<()> {
        let mut guard = self.inner.lock().await;
        guard.steps.insert(
            (run.run_id.clone(), step.node_id.clone()),
            step.clone(),
        );
        guard
            .step_history
            .push((step.node_id.clone(), step.clone()));
        Ok(())
    }

    async fn sync_map_shard(
        &self,
        run: &WorkflowRun,
        shard: &MapShardTaskView,
    ) -> WorkflowResult<()> {
        let mut guard = self.inner.lock().await;
        guard.map_shards.insert(
            (
                run.run_id.clone(),
                shard.for_each_id.clone(),
                shard.shard_index,
            ),
            shard.clone(),
        );
        Ok(())
    }

    async fn sync_thunk(&self, run: &WorkflowRun, thunk: &ThunkTaskView) -> WorkflowResult<()> {
        let mut guard = self.inner.lock().await;
        guard.thunks.insert(
            (run.run_id.clone(), thunk.thunk_obj_id.clone()),
            thunk.clone(),
        );
        Ok(())
    }

    async fn report_step_validation_error(
        &self,
        run: &WorkflowRun,
        node_id: &str,
        message: &str,
    ) -> WorkflowResult<()> {
        let mut guard = self.inner.lock().await;
        guard.validation_errors.push((
            run.run_id.clone(),
            node_id.to_string(),
            message.to_string(),
        ));
        Ok(())
    }
}

/// 把执行单元真实落到 buckyos task_manager 的实现。
pub struct TaskManagerTaskTracker {
    client: Arc<TaskManagerClient>,
    user_id: String,
    app_id: String,
    state: Mutex<TaskTrackerState>,
}

#[derive(Default)]
struct TaskTrackerState {
    /// run_id -> root task id
    run_tasks: HashMap<String, i64>,
    /// (run_id, node_id) -> step task id
    step_tasks: HashMap<(String, String), i64>,
    /// (run_id, for_each_id, shard_index) -> map_shard task id
    map_shard_tasks: HashMap<(String, String, u32), i64>,
    /// (run_id, thunk_obj_id) -> thunk task id
    thunk_tasks: HashMap<(String, String), i64>,
}

impl TaskManagerTaskTracker {
    pub fn new(
        client: Arc<TaskManagerClient>,
        user_id: impl Into<String>,
        app_id: impl Into<String>,
    ) -> Self {
        Self {
            client,
            user_id: user_id.into(),
            app_id: app_id.into(),
            state: Mutex::new(TaskTrackerState::default()),
        }
    }

    async fn ensure_run_task(&self, run: &WorkflowRun) -> WorkflowResult<i64> {
        if let Some(task_id) = self.state.lock().await.run_tasks.get(&run.run_id).copied() {
            return Ok(task_id);
        }

        // task table has UNIQUE(app_id, user_id, name); two runs of the same
        // workflow share workflow_name, so include run_id to disambiguate.
        let task_name = format!("{} [{}]", run.workflow_name, run.run_id);
        let task = self
            .client
            .create_task(
                task_name.as_str(),
                "workflow/run",
                Some(json!({
                    "workflow": {
                        "run_id": run.run_id,
                        "workflow_id": run.workflow_id,
                        "workflow_name": run.workflow_name,
                        "plan_version": run.plan_version,
                    },
                })),
                self.user_id.as_str(),
                self.app_id.as_str(),
                Some(CreateTaskOptions::with_root_id(run.run_id.clone())),
            )
            .await
            .map_err(|err| WorkflowError::TaskTracker(err.to_string()))?;
        self.state
            .lock()
            .await
            .run_tasks
            .insert(run.run_id.clone(), task.id);
        Ok(task.id)
    }

    async fn ensure_step_task(
        &self,
        run: &WorkflowRun,
        step: &StepTaskView,
    ) -> WorkflowResult<i64> {
        let key = (run.run_id.clone(), step.node_id.clone());
        if let Some(task_id) = self.state.lock().await.step_tasks.get(&key).copied() {
            return Ok(task_id);
        }
        let parent_id = self.ensure_run_task(run).await?;

        let task_name = format!("{} [{}/{}]", step.name, run.run_id, step.node_id);
        let task = self
            .client
            .create_task(
                task_name.as_str(),
                "workflow/step",
                Some(initial_step_task_data(run, step)),
                self.user_id.as_str(),
                self.app_id.as_str(),
                Some(CreateTaskOptions {
                    parent_id: Some(parent_id),
                    root_id: Some(run.run_id.clone()),
                    ..Default::default()
                }),
            )
            .await
            .map_err(|err| WorkflowError::TaskTracker(err.to_string()))?;
        self.state
            .lock()
            .await
            .step_tasks
            .insert(key, task.id);
        Ok(task.id)
    }

    async fn ensure_map_shard_task(
        &self,
        run: &WorkflowRun,
        shard: &MapShardTaskView,
    ) -> WorkflowResult<i64> {
        let key = (
            run.run_id.clone(),
            shard.for_each_id.clone(),
            shard.shard_index,
        );
        if let Some(task_id) = self.state.lock().await.map_shard_tasks.get(&key).copied() {
            return Ok(task_id);
        }
        let parent_id = self
            .state
            .lock()
            .await
            .step_tasks
            .get(&(run.run_id.clone(), shard.for_each_id.clone()))
            .copied();
        let parent_id = match parent_id {
            Some(id) => id,
            None => self.ensure_run_task(run).await?,
        };

        let task = self
            .client
            .create_task(
                &format!("{}[{}] [{}]", shard.for_each_id, shard.shard_index, run.run_id),
                "workflow/map_shard",
                Some(initial_map_shard_task_data(run, shard)),
                self.user_id.as_str(),
                self.app_id.as_str(),
                Some(CreateTaskOptions {
                    parent_id: Some(parent_id),
                    root_id: Some(run.run_id.clone()),
                    ..Default::default()
                }),
            )
            .await
            .map_err(|err| WorkflowError::TaskTracker(err.to_string()))?;
        self.state
            .lock()
            .await
            .map_shard_tasks
            .insert(key, task.id);
        Ok(task.id)
    }

    async fn ensure_thunk_task(
        &self,
        run: &WorkflowRun,
        thunk: &ThunkTaskView,
    ) -> WorkflowResult<i64> {
        let key = (run.run_id.clone(), thunk.thunk_obj_id.clone());
        if let Some(task_id) = self.state.lock().await.thunk_tasks.get(&key).copied() {
            return Ok(task_id);
        }

        // 父任务：shard 走 map_shard task，否则走 step task。
        let parent_id = if let Some(shard_index) = thunk.shard_index {
            self.state
                .lock()
                .await
                .map_shard_tasks
                .get(&(run.run_id.clone(), thunk.node_id.clone(), shard_index))
                .copied()
        } else {
            self.state
                .lock()
                .await
                .step_tasks
                .get(&(run.run_id.clone(), thunk.node_id.clone()))
                .copied()
        };
        let parent_id = match parent_id {
            Some(id) => id,
            None => self.ensure_run_task(run).await?,
        };

        let task = self
            .client
            .create_task(
                &format!("thunk:{} [{}]", thunk.thunk_obj_id, run.run_id),
                "workflow/thunk",
                Some(json!({
                    "workflow": {
                        "run_id": run.run_id,
                        "node_id": thunk.node_id,
                        "thunk_obj_id": thunk.thunk_obj_id,
                        "attempt": thunk.attempt,
                        "shard_index": thunk.shard_index,
                    }
                })),
                self.user_id.as_str(),
                self.app_id.as_str(),
                Some(CreateTaskOptions {
                    parent_id: Some(parent_id),
                    root_id: Some(run.run_id.clone()),
                    ..Default::default()
                }),
            )
            .await
            .map_err(|err| WorkflowError::TaskTracker(err.to_string()))?;
        self.state
            .lock()
            .await
            .thunk_tasks
            .insert(key, task.id);
        Ok(task.id)
    }
}

#[async_trait]
impl WorkflowTaskTracker for TaskManagerTaskTracker {
    async fn sync_run(&self, run: &WorkflowRun) -> WorkflowResult<()> {
        let task_id = self.ensure_run_task(run).await?;
        self.client
            .update_task(
                task_id,
                Some(map_run_status(run.status)),
                Some(run.progress_percent()),
                Some(run.status.to_string()),
                Some(json!({
                    "workflow": {
                        "run_id": run.run_id,
                        "workflow_id": run.workflow_id,
                        "workflow_name": run.workflow_name,
                        "plan_version": run.plan_version,
                        "status": run.status,
                        "summary": run.node_state_counts(),
                        "updated_at": run.updated_at,
                    }
                })),
            )
            .await
            .map_err(|err| WorkflowError::TaskTracker(err.to_string()))
    }

    async fn sync_step(&self, run: &WorkflowRun, step: &StepTaskView) -> WorkflowResult<()> {
        let task_id = self.ensure_step_task(run, step).await?;
        let message = step
            .progress_message
            .clone()
            .or_else(|| step.error.clone())
            .unwrap_or_else(|| node_state_label(step.state));
        self.client
            .update_task(
                task_id,
                Some(map_node_status(step.state)),
                None,
                Some(message),
                Some(step_task_data(run, step)),
            )
            .await
            .map_err(|err| WorkflowError::TaskTracker(err.to_string()))
    }

    async fn sync_map_shard(
        &self,
        run: &WorkflowRun,
        shard: &MapShardTaskView,
    ) -> WorkflowResult<()> {
        let task_id = self.ensure_map_shard_task(run, shard).await?;
        let message = shard
            .error
            .clone()
            .unwrap_or_else(|| node_state_label(shard.state));
        self.client
            .update_task(
                task_id,
                Some(map_node_status(shard.state)),
                None,
                Some(message),
                Some(map_shard_task_data(run, shard)),
            )
            .await
            .map_err(|err| WorkflowError::TaskTracker(err.to_string()))
    }

    async fn sync_thunk(&self, run: &WorkflowRun, thunk: &ThunkTaskView) -> WorkflowResult<()> {
        // Thunk 的 status / progress / payload 由 scheduler 写，workflow 只做创建 +
        // ACL 落地，不再覆盖 status。
        let _ = self.ensure_thunk_task(run, thunk).await?;
        Ok(())
    }

    async fn report_step_validation_error(
        &self,
        run: &WorkflowRun,
        node_id: &str,
        message: &str,
    ) -> WorkflowResult<()> {
        let task_id = {
            let guard = self.state.lock().await;
            guard
                .step_tasks
                .get(&(run.run_id.clone(), node_id.to_string()))
                .copied()
        };
        let Some(task_id) = task_id else {
            return Ok(());
        };
        self.client
            .update_task_data(
                task_id,
                json!({
                    "last_error": {
                        "message": message,
                        "ts": chrono::Utc::now().timestamp(),
                    }
                }),
            )
            .await
            .map_err(|err| WorkflowError::TaskTracker(err.to_string()))
    }
}

fn initial_step_task_data(run: &WorkflowRun, step: &StepTaskView) -> Value {
    json!({
        "workflow": workflow_descriptor(run, step),
    })
}

fn step_task_data(run: &WorkflowRun, step: &StepTaskView) -> Value {
    let mut data = json!({
        "workflow": workflow_descriptor(run, step),
    });
    if let Some(output) = step.output.as_ref() {
        data.as_object_mut()
            .unwrap()
            .insert("output".to_string(), output.clone());
    }
    if let Some(err) = step.error.as_ref() {
        data.as_object_mut().unwrap().insert(
            "last_error".to_string(),
            json!({
                "message": err,
                "ts": chrono::Utc::now().timestamp(),
            }),
        );
    } else {
        data.as_object_mut()
            .unwrap()
            .insert("last_error".to_string(), Value::Null);
    }
    data
}

fn workflow_descriptor(run: &WorkflowRun, step: &StepTaskView) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("run_id".to_string(), Value::String(run.run_id.clone()));
    obj.insert(
        "node_id".to_string(),
        Value::String(step.node_id.clone()),
    );
    obj.insert(
        "attempt".to_string(),
        Value::Number(step.attempt.into()),
    );
    if let Some(executor) = step.executor.as_ref() {
        obj.insert(
            "executor".to_string(),
            Value::String(executor.clone()),
        );
    }
    if let Some(prompt) = step.prompt.as_ref() {
        obj.insert("prompt".to_string(), Value::String(prompt.clone()));
    }
    if let Some(schema) = step.output_schema.as_ref() {
        obj.insert("output_schema".to_string(), schema.clone());
    }
    if let Some(subject) = step.subject.as_ref() {
        obj.insert("subject".to_string(), subject.clone());
    }
    if let Some(subject_obj_id) = step.subject_obj_id.as_ref() {
        obj.insert(
            "subject_obj_id".to_string(),
            Value::String(subject_obj_id.clone()),
        );
    }
    if !step.stakeholders.is_empty() {
        obj.insert(
            "stakeholders".to_string(),
            Value::Array(
                step.stakeholders
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect(),
            ),
        );
    }
    if let Some(ts) = step.waiting_human_since {
        obj.insert(
            "waiting_human_since".to_string(),
            Value::Number(ts.into()),
        );
    }
    Value::Object(obj)
}

fn initial_map_shard_task_data(run: &WorkflowRun, shard: &MapShardTaskView) -> Value {
    json!({
        "workflow": {
            "run_id": run.run_id,
            "node_id": shard.for_each_id,
            "shard_index": shard.shard_index,
            "attempt": shard.attempt,
            "item": shard.item,
        }
    })
}

fn map_shard_task_data(run: &WorkflowRun, shard: &MapShardTaskView) -> Value {
    let mut data = initial_map_shard_task_data(run, shard);
    if let Some(output) = shard.output.as_ref() {
        data.as_object_mut()
            .unwrap()
            .insert("output".to_string(), output.clone());
    }
    if let Some(err) = shard.error.as_ref() {
        data.as_object_mut().unwrap().insert(
            "last_error".to_string(),
            json!({
                "message": err,
                "ts": chrono::Utc::now().timestamp(),
            }),
        );
    }
    data
}

fn map_run_status(status: RunStatus) -> TaskStatus {
    match status {
        RunStatus::Created => TaskStatus::Pending,
        RunStatus::Running => TaskStatus::Running,
        RunStatus::WaitingHuman => TaskStatus::WaitingForApproval,
        RunStatus::Completed => TaskStatus::Completed,
        RunStatus::Failed | RunStatus::BudgetExhausted => TaskStatus::Failed,
        RunStatus::Paused => TaskStatus::Paused,
        RunStatus::Aborted => TaskStatus::Canceled,
    }
}

fn node_state_label(state: NodeRunState) -> String {
    match state {
        NodeRunState::Pending => "pending",
        NodeRunState::Ready => "ready",
        NodeRunState::Running => "running",
        NodeRunState::Completed => "completed",
        NodeRunState::Failed => "failed",
        NodeRunState::Retrying => "retrying",
        NodeRunState::WaitingHuman => "waiting_human",
        NodeRunState::Skipped => "skipped",
        NodeRunState::Aborted => "aborted",
        NodeRunState::Cancelled => "cancelled",
    }
    .to_string()
}

fn map_node_status(state: NodeRunState) -> TaskStatus {
    match state {
        NodeRunState::Pending | NodeRunState::Ready => TaskStatus::Pending,
        NodeRunState::Running => TaskStatus::Running,
        NodeRunState::Retrying => TaskStatus::Running,
        NodeRunState::Completed | NodeRunState::Skipped => TaskStatus::Completed,
        NodeRunState::Failed => TaskStatus::Failed,
        NodeRunState::WaitingHuman => TaskStatus::WaitingForApproval,
        NodeRunState::Aborted | NodeRunState::Cancelled => TaskStatus::Canceled,
    }
}
