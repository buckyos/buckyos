use crate::error::{WorkflowError, WorkflowResult};
use crate::runtime::{RunStatus, WorkflowRun};
use async_trait::async_trait;
use buckyos_api::{CreateTaskOptions, TaskManagerClient, TaskStatus};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

#[async_trait]
pub trait WorkflowTaskTracker: Send + Sync {
    async fn sync_run(&self, run: &WorkflowRun) -> WorkflowResult<()>;
}

#[derive(Debug, Default, Clone)]
pub struct NoopTaskTracker;

#[async_trait]
impl WorkflowTaskTracker for NoopTaskTracker {
    async fn sync_run(&self, _run: &WorkflowRun) -> WorkflowResult<()> {
        Ok(())
    }
}

pub struct TaskManagerTaskTracker {
    client: Arc<TaskManagerClient>,
    user_id: String,
    app_id: String,
    run_tasks: Mutex<HashMap<String, i64>>,
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
            run_tasks: Mutex::new(HashMap::new()),
        }
    }

    async fn ensure_run_task(&self, run: &WorkflowRun) -> WorkflowResult<i64> {
        if let Some(task_id) = self.run_tasks.lock().await.get(&run.run_id).copied() {
            return Ok(task_id);
        }

        let task = self
            .client
            .create_task(
                run.workflow_name.as_str(),
                "workflow/run",
                Some(json!({
                    "workflow_id": run.workflow_id,
                    "run_id": run.run_id,
                    "plan_version": run.plan_version,
                })),
                self.user_id.as_str(),
                self.app_id.as_str(),
                Some(CreateTaskOptions::with_root_id(run.run_id.clone())),
            )
            .await
            .map_err(|err| WorkflowError::TaskTracker(err.to_string()))?;
        self.run_tasks
            .lock()
            .await
            .insert(run.run_id.clone(), task.id);
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
                    "workflow_id": run.workflow_id,
                    "run_id": run.run_id,
                    "status": run.status,
                    "plan_version": run.plan_version,
                    "summary": run.node_state_counts(),
                    "updated_at": run.updated_at,
                })),
            )
            .await
            .map_err(|err| WorkflowError::TaskTracker(err.to_string()))
    }
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
