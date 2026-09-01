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
use buckyos_api::{
    get_buckyos_api_runtime, BindAppExecutorReq, CreatePromisedTaskReq, CreateTaskExecutor,
    CreateTaskReq, StorageDomain, Task, TaskDataErrorInfo, TaskDataProgress, TaskError,
    TaskExecutor, TaskManagerClient, TaskPhase, TaskWaitReason, TaskWaitReasonKind, ThunkTaskData,
    ThunkTaskRequest, WorkflowMapShardTaskData, WorkflowMapShardTaskRequest, WorkflowRunTaskData,
    WorkflowRunTaskRequest, WorkflowRunTaskResult, WorkflowStepTaskData, WorkflowStepTaskRequest,
};
use serde::Serialize;
use serde_json::Value;
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
        guard
            .steps
            .insert((run.run_id.clone(), step.node_id.clone()), step.clone());
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
    client: Option<Arc<TaskManagerClient>>,
    app_id: String,
    state: Mutex<TaskTrackerState>,
}

#[derive(Default)]
struct TaskTrackerState {
    /// run_id -> root task id
    run_tasks: HashMap<String, String>,
    /// (run_id, node_id) -> step task id
    step_tasks: HashMap<(String, String), String>,
    /// (run_id, for_each_id, shard_index) -> map_shard task id
    map_shard_tasks: HashMap<(String, String, u32), String>,
    /// (run_id, thunk_obj_id) -> thunk task id
    thunk_tasks: HashMap<(String, String), String>,
}

impl TaskManagerTaskTracker {
    pub fn new(client: Arc<TaskManagerClient>, app_id: impl Into<String>) -> Self {
        Self {
            client: Some(client),
            app_id: app_id.into(),
            state: Mutex::new(TaskTrackerState::default()),
        }
    }

    pub fn from_runtime(app_id: impl Into<String>) -> Self {
        Self {
            client: None,
            app_id: app_id.into(),
            state: Mutex::new(TaskTrackerState::default()),
        }
    }

    async fn client(&self) -> WorkflowResult<Arc<TaskManagerClient>> {
        if let Some(client) = self.client.as_ref() {
            return Ok(client.clone());
        }
        let runtime =
            get_buckyos_api_runtime().map_err(|err| WorkflowError::TaskTracker(err.to_string()))?;
        Box::pin(runtime.get_task_mgr_client())
            .await
            .map(Arc::new)
            .map_err(|err| WorkflowError::TaskTracker(err.to_string()))
    }

    async fn create_tracked_task(
        &self,
        name: String,
        schema_id: String,
        input: Value,
        parent_id: Option<String>,
        idempotency_key: String,
    ) -> WorkflowResult<Task> {
        let client = self.client().await?;
        let Some(parent_id) = parent_id else {
            return client
                .create_task(CreateTaskReq {
                    name,
                    schema_id,
                    schema_version: None,
                    input,
                    executor: CreateTaskExecutor::SelfApp {
                        app_instance_id: None,
                    },
                    parent_id: None,
                    child_control_policy: None,
                    policy_preset: None,
                    permission_boundary: false,
                    storage_domain: Some(StorageDomain::System),
                    idempotency_key,
                    retry_of: None,
                    supersedes: None,
                    message: None,
                })
                .await
                .map_err(|err| WorkflowError::TaskTracker(err.to_string()));
        };

        let parent = client
            .get_task(&parent_id)
            .await
            .map_err(|err| WorkflowError::TaskTracker(err.to_string()))?;
        let task = client
            .create_promised_task(CreatePromisedTaskReq {
                name,
                schema_id,
                schema_version: None,
                input,
                creator: parent.creator,
                expected_input_digest: None,
                origin_ref: None,
                parent_id: Some(parent_id),
                child_control_policy: None,
                policy_preset: None,
                permission_boundary: false,
                storage_domain: None,
                idempotency_key: idempotency_key.clone(),
                wait_reason: None,
                message: None,
            })
            .await
            .map_err(|err| WorkflowError::TaskTracker(err.to_string()))?;
        match &task.executor {
            TaskExecutor::Unbound => client
                .bind_app_executor(BindAppExecutorReq {
                    task_id: task.task_id.clone(),
                    target_id: None,
                    app_id: self.app_id.clone(),
                    app_instance_id: self.app_id.clone(),
                    delivery_id: Some(idempotency_key),
                    expected_revision: task.revision,
                })
                .await
                .map_err(|err| WorkflowError::TaskTracker(err.to_string())),
            TaskExecutor::App {
                app_id,
                app_instance_id,
                ..
            } if app_id == &self.app_id && app_instance_id.as_deref() == Some(&self.app_id) => {
                Ok(task)
            }
            _ => Err(WorkflowError::TaskTracker(format!(
                "delegated task {} has an unexpected executor",
                task.task_id
            ))),
        }
    }

    async fn ensure_run_task(&self, run: &WorkflowRun) -> WorkflowResult<String> {
        if let Some(task_id) = self.state.lock().await.run_tasks.get(&run.run_id).cloned() {
            return Ok(task_id);
        }

        let task_name = format!("{} [{}]", run.workflow_name, run.run_id);
        let parent_id = run_task_parent(run);
        // The run id doubles as the idempotency key, so a restarted tracker
        // finds the same root task instead of minting a duplicate.
        let task = self
            .create_tracked_task(
                task_name,
                WORKFLOW_RUN_SCHEMA_ID.to_string(),
                run_request_data(run),
                parent_id,
                format!("wf-run-{}", run.run_id),
            )
            .await?;
        self.state
            .lock()
            .await
            .run_tasks
            .insert(run.run_id.clone(), task.task_id.clone());
        Ok(task.task_id)
    }

    async fn ensure_step_task(
        &self,
        run: &WorkflowRun,
        step: &StepTaskView,
    ) -> WorkflowResult<String> {
        let key = (run.run_id.clone(), step.node_id.clone());
        if let Some(task_id) = self.state.lock().await.step_tasks.get(&key).cloned() {
            return Ok(task_id);
        }
        let parent_id = self.ensure_run_task(run).await?;

        let task_name = format!("{} [{}/{}]", step.name, run.run_id, step.node_id);
        let task = self
            .create_tracked_task(
                task_name,
                WORKFLOW_STEP_SCHEMA_ID.to_string(),
                initial_step_task_data(run, step),
                Some(parent_id),
                format!("wf-step-{}-{}", run.run_id, step.node_id),
            )
            .await?;
        self.state
            .lock()
            .await
            .step_tasks
            .insert(key, task.task_id.clone());
        Ok(task.task_id)
    }

    async fn ensure_map_shard_task(
        &self,
        run: &WorkflowRun,
        shard: &MapShardTaskView,
    ) -> WorkflowResult<String> {
        let key = (
            run.run_id.clone(),
            shard.for_each_id.clone(),
            shard.shard_index,
        );
        if let Some(task_id) = self.state.lock().await.map_shard_tasks.get(&key).cloned() {
            return Ok(task_id);
        }
        let parent_id = self
            .state
            .lock()
            .await
            .step_tasks
            .get(&(run.run_id.clone(), shard.for_each_id.clone()))
            .cloned();
        let parent_id = match parent_id {
            Some(id) => id,
            None => self.ensure_run_task(run).await?,
        };

        let task = self
            .create_tracked_task(
                format!(
                    "{}[{}] [{}]",
                    shard.for_each_id, shard.shard_index, run.run_id
                ),
                WORKFLOW_MAP_SHARD_SCHEMA_ID.to_string(),
                initial_map_shard_task_data(run, shard),
                Some(parent_id),
                format!(
                    "wf-shard-{}-{}-{}",
                    run.run_id, shard.for_each_id, shard.shard_index
                ),
            )
            .await?;
        self.state
            .lock()
            .await
            .map_shard_tasks
            .insert(key, task.task_id.clone());
        Ok(task.task_id)
    }

    async fn ensure_thunk_task(
        &self,
        run: &WorkflowRun,
        thunk: &ThunkTaskView,
    ) -> WorkflowResult<String> {
        let key = (run.run_id.clone(), thunk.thunk_obj_id.clone());
        if let Some(task_id) = self.state.lock().await.thunk_tasks.get(&key).cloned() {
            return Ok(task_id);
        }

        // 父任务：shard 走 map_shard task，否则走 step task。
        let parent_id = if let Some(shard_index) = thunk.shard_index {
            self.state
                .lock()
                .await
                .map_shard_tasks
                .get(&(run.run_id.clone(), thunk.node_id.clone(), shard_index))
                .cloned()
        } else {
            self.state
                .lock()
                .await
                .step_tasks
                .get(&(run.run_id.clone(), thunk.node_id.clone()))
                .cloned()
        };
        let parent_id = match parent_id {
            Some(id) => id,
            None => self.ensure_run_task(run).await?,
        };

        let task = self
            .create_tracked_task(
                format!("thunk:{} [{}]", thunk.thunk_obj_id, run.run_id),
                WORKFLOW_THUNK_SCHEMA_ID.to_string(),
                thunk_task_data(run, thunk),
                Some(parent_id),
                format!("wf-thunk-{}-{}", run.run_id, thunk.thunk_obj_id),
            )
            .await?;
        self.state
            .lock()
            .await
            .thunk_tasks
            .insert(key, task.task_id.clone());
        Ok(task.task_id)
    }

    /// Mirror one execution unit's state onto its 2.0 task. Terminal tasks
    /// absorb: once the mirror committed/failed/canceled, later syncs no-op.
    async fn mirror_state(
        &self,
        task_id: &str,
        desired: MirrorState,
        progress_data: Value,
        message: String,
    ) -> WorkflowResult<()> {
        let client = self.client().await?;
        let task = client
            .get_task(task_id)
            .await
            .map_err(|err| WorkflowError::TaskTracker(err.to_string()))?;
        if task.phase.is_terminal() {
            return Ok(());
        }
        let task = self
            .write_progress(
                &client,
                task,
                Some(progress_data.clone()),
                Some(message.clone()),
            )
            .await?;
        let result = match desired {
            MirrorState::Pending => Ok(task),
            MirrorState::Running => self.drive_running(&client, task).await,
            MirrorState::Waiting(reason) => {
                if task.phase == TaskPhase::Waiting {
                    Ok(task)
                } else {
                    let task = self.drive_running(&client, task).await?;
                    client
                        .report_waiting(buckyos_api::ReportWaitingReq {
                            envelope: runner_envelope(&task),
                            reason,
                        })
                        .await
                        .map_err(|err| WorkflowError::TaskTracker(err.to_string()))
                }
            }
            MirrorState::Succeeded => client
                .commit_result(buckyos_api::CommitResultReq {
                    task_id: task.task_id.clone(),
                    result: progress_data,
                    app_instance_id: runner_envelope(&task).app_instance_id,
                    runner_epoch: Some(task.runner_epoch),
                    expected_revision: task.revision,
                })
                .await
                .map_err(|err| WorkflowError::TaskTracker(err.to_string())),
            MirrorState::Failed(code) => client
                .fail_task(buckyos_api::FailTaskReq {
                    envelope: runner_envelope(&task),
                    error: TaskError {
                        code,
                        message,
                        detail: Some(progress_data),
                    },
                })
                .await
                .map_err(|err| WorkflowError::TaskTracker(err.to_string())),
            MirrorState::Canceled => {
                let request_id = format!("wf-cancel-{}", task.task_id);
                let requested = client
                    .request_control(buckyos_api::RequestControlReq {
                        task_id: task.task_id.clone(),
                        action: buckyos_api::TaskControlAction::Cancel,
                        request_id: request_id.clone(),
                        recursive: false,
                        expected_revision: None,
                    })
                    .await;
                match requested {
                    Ok(buckyos_api::RequestControlResult::Task { task })
                        if !task.phase.is_terminal() =>
                    {
                        client
                            .ack_control(buckyos_api::AckControlReq {
                                envelope: runner_envelope(&task),
                                request_id,
                                applied: true,
                                reject_reason: None,
                            })
                            .await
                            .map_err(|err| WorkflowError::TaskTracker(err.to_string()))
                    }
                    Ok(buckyos_api::RequestControlResult::Task { task }) => Ok(task),
                    Ok(_) => client
                        .get_task(task_id)
                        .await
                        .map_err(|err| WorkflowError::TaskTracker(err.to_string())),
                    Err(err) => Err(WorkflowError::TaskTracker(err.to_string())),
                }
            }
        };
        result.map(|_| ())
    }

    async fn drive_running(&self, client: &TaskManagerClient, task: Task) -> WorkflowResult<Task> {
        match task.phase {
            TaskPhase::Accepted => client
                .report_started(buckyos_api::ReportStartedReq {
                    envelope: runner_envelope(&task),
                })
                .await
                .map_err(|err| WorkflowError::TaskTracker(err.to_string())),
            TaskPhase::Waiting => client
                .report_running(buckyos_api::ReportRunningReq {
                    envelope: runner_envelope(&task),
                })
                .await
                .map_err(|err| WorkflowError::TaskTracker(err.to_string())),
            _ => Ok(task),
        }
    }

    async fn write_progress(
        &self,
        client: &TaskManagerClient,
        task: Task,
        progress: Option<Value>,
        message: Option<String>,
    ) -> WorkflowResult<Task> {
        client
            .report_progress(buckyos_api::ReportProgressReq {
                envelope: runner_envelope(&task),
                progress,
                message,
            })
            .await
            .map_err(|err| WorkflowError::TaskTracker(err.to_string()))
    }
}

enum MirrorState {
    Pending,
    Running,
    Waiting(TaskWaitReason),
    Succeeded,
    Failed(String),
    Canceled,
}

fn runner_envelope(task: &Task) -> buckyos_api::RunnerWriteEnvelope {
    let app_instance_id = match &task.executor {
        TaskExecutor::App {
            app_instance_id, ..
        } => app_instance_id.clone(),
        _ => None,
    };
    buckyos_api::RunnerWriteEnvelope {
        task_id: task.task_id.clone(),
        app_instance_id,
        runner_epoch: task.runner_epoch,
        expected_revision: task.revision,
    }
}

#[async_trait]
impl WorkflowTaskTracker for TaskManagerTaskTracker {
    async fn sync_run(&self, run: &WorkflowRun) -> WorkflowResult<()> {
        let task_id = self.ensure_run_task(run).await?;
        self.mirror_state(
            &task_id,
            map_run_state(run.status),
            run_task_data(run),
            run.status.to_string(),
        )
        .await
    }

    async fn sync_step(&self, run: &WorkflowRun, step: &StepTaskView) -> WorkflowResult<()> {
        let task_id = self.ensure_step_task(run, step).await?;
        let message = step
            .progress_message
            .clone()
            .or_else(|| step.error.clone())
            .unwrap_or_else(|| node_state_label(step.state));
        self.mirror_state(
            &task_id,
            map_node_state(step.state),
            step_task_data(run, step),
            message,
        )
        .await
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
        self.mirror_state(
            &task_id,
            map_node_state(shard.state),
            map_shard_task_data(run, shard),
            message,
        )
        .await
    }

    async fn sync_thunk(&self, run: &WorkflowRun, thunk: &ThunkTaskView) -> WorkflowResult<()> {
        // Thunk 任务只做创建（ACL 落地 + 树结构）；执行状态由消费方驱动。
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
                .cloned()
        };
        let Some(task_id) = task_id else {
            return Ok(());
        };
        let client = self.client().await?;
        let task = client
            .get_task(&task_id)
            .await
            .map_err(|err| WorkflowError::TaskTracker(err.to_string()))?;
        if task.phase.is_terminal() {
            return Ok(());
        }
        self.write_progress(
            &client,
            task,
            Some(task_data_value(WorkflowStepTaskData {
                request: WorkflowStepTaskRequest {
                    run_id: run.run_id.clone(),
                    node_id: node_id.to_string(),
                    ..Default::default()
                },
                progress: None,
                result: None,
                human_action: None,
                last_error: Some(TaskDataErrorInfo {
                    message: Some(message.to_string()),
                    ts: Some(chrono::Utc::now().timestamp()),
                    ..Default::default()
                }),
            })),
            Some(message.to_string()),
        )
        .await
        .map(|_| ())
    }
}

/// Versioned task schema ids for the workflow mirror tree.
pub const WORKFLOW_RUN_SCHEMA_ID: &str = "workflow.run_tree/v1";
pub const WORKFLOW_STEP_SCHEMA_ID: &str = "workflow.step/v1";
pub const WORKFLOW_MAP_SHARD_SCHEMA_ID: &str = "workflow.map_shard/v1";
pub const WORKFLOW_THUNK_SCHEMA_ID: &str = "workflow.thunk/v1";

/// Roots derive from parents in 2.0: a scheduled run hangs under its
/// schedule task when that id is known, otherwise the run task is its own
/// root.
fn run_task_parent(run: &WorkflowRun) -> Option<String> {
    run.metrics
        .get("schedule_task")
        .and_then(|schedule_task| {
            schedule_task
                .get("task_id")
                .or_else(|| schedule_task.get("root_task_id"))
        })
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.is_empty())
}

/// Immutable request payload for the run task; the mutable mirror rides in
/// progress via `run_task_data`.
fn run_request_data(run: &WorkflowRun) -> Value {
    task_data_value(WorkflowRunTaskData {
        request: WorkflowRunTaskRequest {
            run_id: run.run_id.clone(),
            workflow_id: run.workflow_id.clone(),
            workflow_name: run.workflow_name.clone(),
            plan_version: run.plan_version,
        },
        progress: None,
        result: None,
        human_action: None,
        last_error: None,
    })
}

fn task_data_value<T: Serialize>(data: T) -> Value {
    serde_json::to_value(data).unwrap_or_else(|_| Value::Object(Default::default()))
}

fn run_task_data(run: &WorkflowRun) -> Value {
    let summary = run
        .node_state_counts()
        .into_iter()
        .map(|(key, value)| (key, value as u64))
        .collect::<std::collections::BTreeMap<_, _>>();
    let total = summary.values().copied().sum::<u64>();
    let completed = summary.get("Completed").copied().unwrap_or_default();
    task_data_value(WorkflowRunTaskData {
        request: WorkflowRunTaskRequest {
            run_id: run.run_id.clone(),
            workflow_id: run.workflow_id.clone(),
            workflow_name: run.workflow_name.clone(),
            plan_version: run.plan_version,
        },
        progress: (total > 0).then(|| TaskDataProgress::with_items(completed, Some(total))),
        result: Some(WorkflowRunTaskResult {
            status: run.status.to_string(),
            summary,
            updated_at: Some(run.updated_at),
        }),
        human_action: None,
        last_error: None,
    })
}

fn initial_step_task_data(run: &WorkflowRun, step: &StepTaskView) -> Value {
    task_data_value(WorkflowStepTaskData {
        request: workflow_step_request(run, step),
        progress: None,
        result: None,
        human_action: None,
        last_error: None,
    })
}

fn step_task_data(run: &WorkflowRun, step: &StepTaskView) -> Value {
    task_data_value(WorkflowStepTaskData {
        request: workflow_step_request(run, step),
        progress: None,
        result: step.output.clone(),
        human_action: None,
        last_error: step.error.as_ref().map(|err| TaskDataErrorInfo {
            message: Some(err.clone()),
            ts: Some(chrono::Utc::now().timestamp()),
            ..Default::default()
        }),
    })
}

fn workflow_step_request(run: &WorkflowRun, step: &StepTaskView) -> WorkflowStepTaskRequest {
    WorkflowStepTaskRequest {
        run_id: run.run_id.clone(),
        node_id: step.node_id.clone(),
        attempt: step.attempt,
        executor: step.executor.clone(),
        prompt: step.prompt.clone(),
        output_schema: step.output_schema.clone(),
        subject: step.subject.clone(),
        subject_obj_id: step.subject_obj_id.clone(),
        stakeholders: step.stakeholders.clone(),
        waiting_human_since: step.waiting_human_since,
    }
}

fn initial_map_shard_task_data(run: &WorkflowRun, shard: &MapShardTaskView) -> Value {
    task_data_value(WorkflowMapShardTaskData {
        request: workflow_map_shard_request(run, shard),
        progress: None,
        result: None,
        last_error: None,
    })
}

fn map_shard_task_data(run: &WorkflowRun, shard: &MapShardTaskView) -> Value {
    task_data_value(WorkflowMapShardTaskData {
        request: workflow_map_shard_request(run, shard),
        progress: None,
        result: shard.output.clone(),
        last_error: shard.error.as_ref().map(|err| TaskDataErrorInfo {
            message: Some(err.clone()),
            ts: Some(chrono::Utc::now().timestamp()),
            ..Default::default()
        }),
    })
}

fn workflow_map_shard_request(
    run: &WorkflowRun,
    shard: &MapShardTaskView,
) -> WorkflowMapShardTaskRequest {
    WorkflowMapShardTaskRequest {
        run_id: run.run_id.clone(),
        node_id: shard.for_each_id.clone(),
        shard_index: shard.shard_index,
        attempt: shard.attempt,
        item: Some(shard.item.clone()),
    }
}

fn thunk_task_data(run: &WorkflowRun, thunk: &ThunkTaskView) -> Value {
    let mut extra = std::collections::BTreeMap::new();
    extra.insert(
        "workflow_run_id".to_string(),
        Value::String(run.run_id.clone()),
    );
    extra.insert(
        "workflow_attempt".to_string(),
        Value::Number(thunk.attempt.into()),
    );
    if let Some(shard_index) = thunk.shard_index {
        extra.insert("workflow_shard_index".to_string(), Value::from(shard_index));
    }
    task_data_value(ThunkTaskData {
        request: ThunkTaskRequest {
            node_id: Some(thunk.node_id.clone()),
            thunk_obj_id: Some(thunk.thunk_obj_id.clone()),
            extra,
            ..Default::default()
        },
        progress: None,
        result: None,
        executor: None,
        extra: Default::default(),
    })
}

fn map_run_state(status: RunStatus) -> MirrorState {
    match status {
        RunStatus::Created => MirrorState::Pending,
        RunStatus::Running => MirrorState::Running,
        RunStatus::WaitingHuman => MirrorState::Waiting(TaskWaitReason::with_code(
            TaskWaitReasonKind::HumanInput,
            "waiting_human",
        )),
        RunStatus::Completed => MirrorState::Succeeded,
        RunStatus::Failed => MirrorState::Failed("workflow_failed".to_string()),
        RunStatus::BudgetExhausted => MirrorState::Failed("budget_exhausted".to_string()),
        // The workflow's own pause is a business wait, not the 2.0 pause
        // control handshake (the tracker mirrors state, it doesn't control).
        RunStatus::Paused => MirrorState::Waiting(TaskWaitReason::with_code(
            TaskWaitReasonKind::Other,
            "workflow_paused",
        )),
        RunStatus::Aborted => MirrorState::Canceled,
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

fn map_node_state(state: NodeRunState) -> MirrorState {
    match state {
        NodeRunState::Pending | NodeRunState::Ready => MirrorState::Pending,
        NodeRunState::Running | NodeRunState::Retrying => MirrorState::Running,
        NodeRunState::Completed | NodeRunState::Skipped => MirrorState::Succeeded,
        NodeRunState::Failed => MirrorState::Failed("step_failed".to_string()),
        NodeRunState::WaitingHuman => MirrorState::Waiting(TaskWaitReason::with_code(
            TaskWaitReasonKind::HumanInput,
            "waiting_human",
        )),
        NodeRunState::Aborted | NodeRunState::Cancelled => MirrorState::Canceled,
    }
}
