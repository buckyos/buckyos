//! Turns in TaskMgr: every Turn of a hosted session is a task the agent
//! runs itself (`opendan.agent_turn/v1`), sub sessions hang under the Turn
//! that created them, and a cancel requested in TaskMgr becomes a `stop` of
//! the session.
//!
//! The session is the only writer of a task's state; TaskMgr is never asked
//! what a session does. The stop itself always goes through the session
//! control protocol: the task tree only says which sessions it reaches.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use buckyos_api::{
    AckControlReq, CreateTaskExecutor, CreateTaskReq, GrantTaskAccessReq, RequestControlReq,
    RunnerWriteEnvelope, Task, TaskAclGrantSpec, TaskAction, TaskControlAction, TaskDataScope,
    TaskExecutor, TaskGrantScope, TaskGrantSubject, TaskManagerClient, TaskOutcome, TaskPhase,
    TaskWaitReason, TaskWaitReasonKind, OPENDAN_AGENT_TURN_TASK_SCHEMA_ID,
};
use libopendan::host::Supervisor;
use libopendan::protocol::*;
use libopendan::runner::{TurnTaskEnd, TurnTaskOpen, TurnTaskSink, TurnTaskStatus};
use libopendan::state::AgentStateClient;
use libopendan::{OpenDanError, SessionDir};
use serde_json::{json, Value};

/// A task as far as the Loader looks at it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskView {
    pub task_id: String,
    pub terminal: bool,
    pub canceled: bool,
    /// A cancel was requested and waits for the runner.
    pub cancel_requested: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewTask {
    pub name: String,
    pub input: Value,
    pub parent_id: Option<String>,
    pub idempotency_key: String,
    /// User granted read and control on the task and its subtree (the
    /// agent's owner).
    pub grant_user: Option<String>,
}

/// What the Loader needs from TaskMgr.
#[async_trait]
pub trait TaskService: Send + Sync {
    /// Idempotent for the same `idempotency_key`.
    async fn create(&self, task: NewTask) -> Result<TaskView, String>;
    async fn get(&self, task_id: &str) -> Result<TaskView, String>;
    /// Running, with `message` / `progress` as the display snapshot.
    async fn running(&self, task_id: &str, message: &str, progress: Value) -> Result<(), String>;
    async fn waiting(&self, task_id: &str, reason: TaskWaitReason) -> Result<(), String>;
    async fn complete(&self, task_id: &str, result: Value) -> Result<(), String>;
    async fn fail(&self, task_id: &str, code: &str, message: &str, detail: Option<Value>)
        -> Result<(), String>;
    /// Close as canceled: acknowledge the pending cancel, or request one
    /// first when the stop did not come from TaskMgr.
    async fn canceled(&self, task_id: &str) -> Result<(), String>;
}

/// TaskMgr of the zone this process runs in. The client is taken from the
/// runtime for every call: the process's session token is renewed.
pub struct ZoneTaskService;

impl ZoneTaskService {
    async fn client() -> Result<TaskManagerClient, String> {
        buckyos_api::get_buckyos_api_runtime()
            .map_err(|e| e.to_string())?
            .get_task_mgr_client()
            .await
            .map_err(|e| e.to_string())
    }
}

/// The display message a task ends with: the activity reported last
/// ("running shell") says nothing about a finished task.
async fn last_message(client: &TaskManagerClient, task_id: &str, message: &str) {
    if let Err(e) = client
        .runner_progress(task_id, None, Some(message.to_string()))
        .await
    {
        log::debug!("task {task_id}: closing message not written: {e}");
    }
}

fn view(task: &Task) -> TaskView {
    TaskView {
        task_id: task.task_id.clone(),
        terminal: task.phase.is_terminal(),
        canceled: task.outcome == Some(TaskOutcome::Canceled),
        cancel_requested: task
            .pending_control
            .as_ref()
            .is_some_and(|c| c.action == TaskControlAction::Cancel),
    }
}

fn envelope(task: &Task) -> RunnerWriteEnvelope {
    RunnerWriteEnvelope {
        task_id: task.task_id.clone(),
        app_instance_id: match &task.executor {
            TaskExecutor::App {
                app_instance_id, ..
            } => app_instance_id.clone(),
            _ => None,
        },
        runner_epoch: task.runner_epoch,
        expected_revision: task.revision,
    }
}

#[async_trait]
impl TaskService for ZoneTaskService {
    async fn create(&self, new: NewTask) -> Result<TaskView, String> {
        let client = Self::client().await?;
        let task = client
            .create_task(CreateTaskReq {
                name: new.name,
                schema_id: OPENDAN_AGENT_TURN_TASK_SCHEMA_ID.to_string(),
                schema_version: None,
                input: new.input,
                executor: CreateTaskExecutor::SelfApp {
                    app_instance_id: None,
                },
                parent_id: new.parent_id.clone(),
                child_control_policy: None,
                policy_preset: None,
                permission_boundary: false,
                storage_domain: None,
                idempotency_key: new.idempotency_key,
                retry_of: None,
                supersedes: None,
                message: None,
            })
            .await
            .map_err(|e| e.to_string())?;
        // The owner reads and controls the whole tree through the grant on
        // its root. A task found again (already started) has it.
        if let (Some(user_id), None, TaskPhase::Accepted) =
            (new.grant_user, &new.parent_id, task.phase)
        {
            let granted = client
                .grant_task_access(GrantTaskAccessReq {
                    task_id: task.task_id.clone(),
                    grant: TaskAclGrantSpec {
                        subject: TaskGrantSubject::User { user_id },
                        actions: vec![
                            TaskAction::ReadMeta,
                            TaskAction::ReadInput,
                            TaskAction::ReadResult,
                            TaskAction::Control,
                        ],
                        scope: TaskGrantScope::Subtree,
                        data_scope: TaskDataScope::Full,
                    },
                    expected_revision: task.revision,
                })
                .await;
            if let Err(e) = granted {
                log::warn!("task {}: owner grant failed: {e}", task.task_id);
            }
        }
        Ok(view(&task))
    }

    async fn get(&self, task_id: &str) -> Result<TaskView, String> {
        let task = Self::client()
            .await?
            .get_task(task_id)
            .await
            .map_err(|e| e.to_string())?;
        Ok(view(&task))
    }

    async fn running(&self, task_id: &str, message: &str, progress: Value) -> Result<(), String> {
        let client = Self::client().await?;
        client.runner_start(task_id).await.map_err(|e| e.to_string())?;
        client
            .runner_progress(task_id, Some(progress), Some(message.to_string()))
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn waiting(&self, task_id: &str, reason: TaskWaitReason) -> Result<(), String> {
        let client = Self::client().await?;
        if let Some(message) = reason.message.clone() {
            client
                .runner_progress(task_id, None, Some(message))
                .await
                .map_err(|e| e.to_string())?;
        }
        client
            .runner_wait(task_id, reason)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn complete(&self, task_id: &str, result: Value) -> Result<(), String> {
        let client = Self::client().await?;
        client.runner_start(task_id).await.map_err(|e| e.to_string())?;
        last_message(&client, task_id, "").await;
        client
            .runner_complete(task_id, result)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn fail(
        &self,
        task_id: &str,
        code: &str,
        message: &str,
        detail: Option<Value>,
    ) -> Result<(), String> {
        let client = Self::client().await?;
        last_message(&client, task_id, message).await;
        client
            .runner_fail(task_id, code, message, detail)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn canceled(&self, task_id: &str) -> Result<(), String> {
        let client = Self::client().await?;
        let mut task = client.get_task(task_id).await.map_err(|e| e.to_string())?;
        if task.phase.is_terminal() {
            return Ok(());
        }
        last_message(&client, task_id, "").await;
        task = client.get_task(task_id).await.map_err(|e| e.to_string())?;
        if task.pending_control.is_none() {
            client
                .request_control(RequestControlReq {
                    task_id: task_id.to_string(),
                    action: TaskControlAction::Cancel,
                    request_id: format!("session-stop:{task_id}"),
                    recursive: false,
                    expected_revision: None,
                })
                .await
                .map_err(|e| e.to_string())?;
            task = client.get_task(task_id).await.map_err(|e| e.to_string())?;
        }
        let Some(pending) = task.pending_control.clone() else {
            return Err(format!("task {task_id} has no pending cancel to acknowledge"));
        };
        if pending.action != TaskControlAction::Cancel {
            return Err(format!(
                "task {task_id} waits for {} while its session stopped",
                pending.action
            ));
        }
        client
            .ack_control(AckControlReq {
                envelope: envelope(&task),
                request_id: pending.request_id,
                applied: true,
                reject_reason: None,
            })
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// User part of an app principal (`app:<app>@<user>`).
pub fn owner_of(who: &str) -> Option<String> {
    who.rsplit_once('@')
        .map(|(_, user)| user.to_string())
        .filter(|u| !u.is_empty())
}

/// Turns of hosted sessions → TaskMgr.
pub struct TurnTasks {
    pub tasks: Arc<dyn TaskService>,
    /// Granted read and control on every root task.
    pub owner: Option<String>,
}

fn wait_reason(w: &WaitingFor, message: &str) -> TaskWaitReason {
    let (kind, code) = match w.kind {
        WaitingKind::Children => (TaskWaitReasonKind::ChildTask, "sub_sessions"),
        WaitingKind::Tool => (TaskWaitReasonKind::Dependency, "tool"),
        WaitingKind::Event => (TaskWaitReasonKind::External, "event"),
        WaitingKind::Input => (TaskWaitReasonKind::Other, "input"),
    };
    TaskWaitReason {
        kind,
        code: Some(code.to_string()),
        related_task_id: None,
        message: Some(message.to_string()),
    }
}

#[async_trait]
impl TurnTaskSink for TurnTasks {
    async fn open(&self, turn: &TurnTaskOpen) -> Result<String, String> {
        let task = self
            .tasks
            .create(NewTask {
                name: turn.title.clone(),
                // Nothing here may change between two calls for the same
                // Turn: a replay must match the frozen input.
                input: json!({
                    "agent_did": turn.agent_did,
                    "session_id": turn.session_id,
                    "session_kind": turn.kind,
                    "turn": turn.turn,
                    "inputs": turn.inputs,
                }),
                parent_id: turn.parent_task.clone(),
                idempotency_key: format!("agent_turn:{}:{}", turn.session_id, turn.turn),
                grant_user: self.owner.clone(),
            })
            .await?;
        Ok(task.task_id)
    }

    async fn update(&self, task_id: &str, status: &TurnTaskStatus) -> Result<(), String> {
        match (&status.waiting_for, status.run_state) {
            (Some(w), RunState::Waiting) => {
                self.tasks
                    .waiting(task_id, wait_reason(w, &status.message))
                    .await
            }
            _ => {
                self.tasks
                    .running(
                        task_id,
                        &status.message,
                        json!({ "behavior": status.behavior, "tool": status.tool }),
                    )
                    .await
            }
        }
    }

    async fn close(&self, task_id: &str, end: &TurnTaskEnd) -> Result<(), String> {
        match end.status {
            TurnStatus::Completed => {
                self.tasks
                    .complete(task_id, json!({ "summary": end.summary }))
                    .await
            }
            TurnStatus::Stopped => self.tasks.canceled(task_id).await,
            TurnStatus::Failed | TurnStatus::BudgetExhausted => {
                let code = if end.status == TurnStatus::Failed {
                    "turn_failed"
                } else {
                    "budget_exhausted"
                };
                let message = end
                    .error
                    .as_ref()
                    .and_then(|e| e.get("message"))
                    .and_then(Value::as_str)
                    .or(end.summary.as_deref())
                    .unwrap_or(code);
                self.tasks
                    .fail(task_id, code, message, end.error.clone())
                    .await
            }
        }
    }
}

/// A cancel requested in TaskMgr for the task of an open Turn stops the
/// session that runs it. Polling carries the work; the sessions' own stop
/// propagation reaches their sub sessions.
pub struct CancelBridge {
    pub tasks: Arc<dyn TaskService>,
    pub agent: Arc<dyn AgentStateClient>,
    pub supervisor: Arc<Supervisor>,
    pub who: String,
}

impl CancelBridge {
    /// One look at every unfinished session this process drives.
    pub async fn scan(&self) -> Result<(), String> {
        let entries = self
            .agent
            .sessions()
            .query(&RegistryQuery {
                driver: Some(self.who.clone()),
                not_finished_or_pending: Some(true),
                ..Default::default()
            })
            .await
            .map_err(|e| e.to_string())?;
        for e in entries {
            if e.status.run_state == RunState::Finished {
                continue;
            }
            let Some(task_id) = SessionDir::open(&e.location)
                .and_then(|sd| sd.state())
                .ok()
                .filter(|s| !s.stop_requested)
                .and_then(|s| s.open_turn_task().map(|t| t.task_id.clone()))
            else {
                continue;
            };
            let task = match self.tasks.get(&task_id).await {
                Ok(t) => t,
                Err(err) => {
                    log::debug!("task bridge: task {task_id} of {}: {err}", e.session_id);
                    continue;
                }
            };
            if !(task.cancel_requested || task.canceled) {
                continue;
            }
            let input = PostedInput::control(
                &self.who,
                format!("stop:task:{task_id}"),
                ControlCommand::Stop {
                    reason: Some(format!("task {task_id} was canceled")),
                },
            );
            match self.agent.sessions().post_input(&e.session_id, &input).await {
                Ok(_) => {
                    log::info!("task bridge: task {task_id} canceled, stopping {}", e.session_id);
                    let _ = self.supervisor.ensure_task(&e.session_id, "task canceled").await;
                }
                // Recreated by its driver; the next scan posts again.
                Err(err @ OpenDanError::QueueMissing { .. }) => {
                    log::warn!("task bridge: cannot stop {} yet: {err}", e.session_id);
                    let _ = self.supervisor.ensure_task(&e.session_id, "input queue missing").await;
                }
                // A session without a queue is stopped with its parent.
                Err(err) => log::warn!("task bridge: cannot stop {}: {err}", e.session_id),
            }
        }
        Ok(())
    }

    pub async fn run(self: Arc<Self>, poll: Duration) {
        loop {
            if let Err(e) = self.scan().await {
                log::warn!("task bridge: {e}");
            }
            tokio::time::sleep(poll).await;
        }
    }
}
