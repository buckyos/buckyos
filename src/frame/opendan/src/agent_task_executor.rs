use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use buckyos_api::{
    get_buckyos_api_runtime, parse_typed_task_data, AgentDelegateProgress, AgentDelegateTaskData,
    AiMessage, AiRole, CreateTaskOptions, HumanInputTaskData, HumanInputTaskRequest, Task,
    TaskFilter, TaskStatus, TypedTaskData,
};
use log::{error, info, warn};
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::agent::{AIAgent, CreateWorkSessionParams};
use crate::session_model::{InterruptMode, PendingInput, SessionKind, SessionMeta, SessionStatus};

pub const TASK_TYPE_AGENT_DELEGATE: &str = "agent.delegate";
pub const TASK_TYPE_HUMAN_INPUT: &str = "human.input";
const TASK_ROUTE_BEHAVIOR: &str = "task_route";

impl AIAgent {
    pub fn task_executor_runner_id(&self) -> Result<String> {
        let configured = self.config.toml.runtime.task_executor.runner_id.trim();
        if configured.is_empty() {
            let runtime = get_buckyos_api_runtime().map_err(|err| {
                anyhow!(
                    "task executor runner_id is unset and BuckyOS runtime is unavailable: {err}"
                )
            })?;
            let runner = runtime.get_full_appid().trim().to_string();
            if runner.is_empty() {
                Err(anyhow!(
                    "task executor runner_id resolved to empty full_appid"
                ))
            } else {
                Ok(runner)
            }
        } else {
            Ok(configured.to_string())
        }
    }

    pub fn spawn_task_inbox(self: Arc<Self>) -> Option<tokio::task::JoinHandle<()>> {
        if !self.config.toml.runtime.task_executor.enabled {
            return None;
        }
        if self.runtime.task_mgr.is_none() {
            return None;
        }
        let runner = match self.task_executor_runner_id() {
            Ok(runner) => runner,
            Err(err) => {
                error!(
                    "opendan.task_inbox[{}]: cannot resolve task executor runner: {err:#}",
                    self.agent_name
                );
                std::process::exit(1);
            }
        };
        Some(tokio::spawn(async move {
            self.run_task_inbox(runner).await;
        }))
    }

    async fn run_task_inbox(self: Arc<Self>, runner: String) {
        let poll_ms = self
            .config
            .toml
            .runtime
            .task_executor
            .poll_interval_ms
            .max(1_000);
        let (wake_tx, mut wake_rx) = mpsc::channel::<()>(16);

        if let Some(kevent) = self.runtime.kevent_client.clone() {
            let event_ids = task_executor_event_ids(&runner);
            match kevent.create_event_reader(event_ids.clone()).await {
                Ok(reader) => {
                    let wake_tx = wake_tx.clone();
                    let shutdown = self.pump_shutdown.clone();
                    tokio::spawn(async move {
                        loop {
                            tokio::select! {
                                _ = shutdown.notified() => break,
                                event = reader.pull_event(Some(poll_ms)) => {
                                    match event {
                                        Ok(Some(_)) => {
                                            let _ = wake_tx.send(()).await;
                                        }
                                        Ok(None) => {}
                                        Err(err) => {
                                            warn!("opendan.task_inbox: task event reader failed: {err}");
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    });
                    info!(
                        "opendan.task_inbox[{}]: subscribed {:?}",
                        self.agent_name, event_ids
                    );
                }
                Err(err) => {
                    warn!(
                        "opendan.task_inbox[{}]: subscribe task events failed: {err}",
                        self.agent_name
                    );
                }
            }
        }

        self.clone().sweep_agent_delegate_tasks(&runner).await;
        let mut interval = tokio::time::interval(Duration::from_millis(poll_ms));
        loop {
            tokio::select! {
                _ = self.pump_shutdown.notified() => break,
                _ = interval.tick() => {
                    self.clone().sweep_agent_delegate_tasks(&runner).await;
                }
                wake = wake_rx.recv() => {
                    if wake.is_none() {
                        break;
                    }
                    self.clone().sweep_agent_delegate_tasks(&runner).await;
                }
            }
        }
    }

    /// Sweep this agent's own `agent.delegate` tasks. TaskMgr no longer has a
    /// runner column: the executing agent is part of the task payload
    /// (`progress.execution.runner`), so we list by task_type and filter the
    /// target agent locally.
    async fn sweep_agent_delegate_tasks(self: Arc<Self>, runner: &str) {
        for status in [
            TaskStatus::Pending,
            TaskStatus::WaitingForApproval,
            TaskStatus::Running,
            TaskStatus::Paused,
            TaskStatus::Canceled,
        ] {
            let Some(task_mgr) = self.runtime.task_mgr.as_ref().cloned() else {
                return;
            };
            let filter = TaskFilter {
                task_type: Some(TASK_TYPE_AGENT_DELEGATE.to_string()),
                status: Some(status),
                ..Default::default()
            };
            let tasks = match task_mgr.list_tasks(Some(filter)).await {
                Ok(tasks) => tasks,
                Err(err) => {
                    warn!(
                        "opendan.task_inbox[{}]: list {:?} delegate tasks failed: {err}",
                        self.agent_name, status
                    );
                    continue;
                }
            };
            for task in tasks {
                if !task_targets_runner(&task, runner) {
                    continue;
                }
                if let Err(err) = self.clone().process_agent_delegate_task(task, runner).await {
                    warn!(
                        "opendan.task_executor[{}]: process delegate task failed: {err:#}",
                        self.agent_name
                    );
                }
            }
        }
    }

    async fn process_agent_delegate_task(
        self: Arc<Self>,
        mut task: Task,
        runner: &str,
    ) -> Result<()> {
        if task.status == TaskStatus::Canceled {
            return self
                .reflect_task_control_to_session(task, runner, "canceled", InterruptMode::Discard)
                .await;
        }
        if task.status.is_terminal() {
            return Ok(());
        }
        if task.status == TaskStatus::Paused {
            return self
                .reflect_task_control_to_session(task, runner, "paused", InterruptMode::Discard)
                .await;
        }
        if task.status == TaskStatus::WaitingForApproval
            && !self.clone().resume_waiting_delegate_task(&task).await?
        {
            return Ok(());
        }
        if task.status == TaskStatus::WaitingForApproval {
            let Some(task_mgr) = self.runtime.task_mgr.as_ref().cloned() else {
                return Err(anyhow!("task manager unavailable"));
            };
            task = task_mgr.get_task(task.id).await?;
            if task.status.is_terminal() {
                return Ok(());
            }
        }

        let delegate_data = agent_delegate_task_data(&task)?;
        if let Some(session_id) = execution_session_id(&delegate_data) {
            let session = self.clone().ensure_session(&session_id).await?;
            session.wake().await;
            return Ok(());
        }
        if self
            .clone()
            .recover_existing_bound_session(&task, runner)
            .await?
        {
            return Ok(());
        }
        if let Some(session_id) = route_session_id(&delegate_data) {
            if !self.config.toml.runtime.task_route.enabled {
                self.clone()
                    .fail_task_route_disabled(task, Some(session_id))
                    .await?;
                return Ok(());
            }
            let session = self.clone().ensure_session(&session_id).await?;
            session.wake().await;
            return Ok(());
        }

        if task_data_supports_direct_worksession(&delegate_data) {
            self.clone()
                .create_worksession_by_task_id(task, delegate_data)
                .await?;
            return Ok(());
        }

        if !self.config.toml.runtime.task_route.enabled {
            self.clone().fail_task_route_disabled(task, None).await?;
            return Ok(());
        }
        self.clone().route_task_via_task_route(task).await?;
        Ok(())
    }

    async fn recover_existing_bound_session(
        self: Arc<Self>,
        task: &Task,
        runner: &str,
    ) -> Result<bool> {
        let Some(bound) = find_bound_worksession(&self.config.layout.sessions_dir, task.id) else {
            return Ok(false);
        };
        let Some(task_mgr) = self.runtime.task_mgr.as_ref().cloned() else {
            return Err(anyhow!("task manager unavailable"));
        };

        if bound.ended {
            task_mgr
                .update_task(
                    task.id,
                    Some(TaskStatus::Failed),
                    Some(task.progress),
                    Some(
                        "Existing bound agent session already ended before task recovery"
                            .to_string(),
                    ),
                    Some(agent_delegate_update_value(task, |data| {
                        set_agent_delegate_execution(
                            data,
                            json!({
                                "session_id": bound.session_id,
                                "workspace_id": bound.workspace_id,
                                "behavior": bound.behavior,
                                "runner": runner,
                                "status": "ended",
                                "recovered_at_ms": now_ms()
                            }),
                        );
                    })?),
                )
                .await?;
            return Ok(true);
        }

        task_mgr
            .update_task(
                task.id,
                Some(TaskStatus::Running),
                Some(task.progress.max(10.0)),
                Some("Recovered existing agent session binding".to_string()),
                Some(agent_delegate_update_value(task, |data| {
                    set_agent_delegate_execution(
                        data,
                        json!({
                            "session_id": bound.session_id,
                            "workspace_id": bound.workspace_id,
                            "behavior": bound.behavior,
                            "runner": runner,
                            "status": "running",
                            "recovered_at_ms": now_ms()
                        }),
                    );
                })?),
            )
            .await?;

        let session = self.clone().ensure_session(&bound.session_id).await?;
        session.wake().await;
        Ok(true)
    }

    async fn create_worksession_by_task_id(
        self: Arc<Self>,
        task: Task,
        data: AgentDelegateTaskData,
    ) -> Result<()> {
        let Some(task_mgr) = self.runtime.task_mgr.as_ref().cloned() else {
            return Err(anyhow!("task manager unavailable"));
        };
        task_mgr
            .update_task(
                task.id,
                Some(TaskStatus::Running),
                Some(5.0),
                Some("Creating agent session from task data".to_string()),
                Some(agent_delegate_update_value(&task, |data| {
                    data.route = Some(json!({
                        "status": "direct",
                        "strategy": "create_worksession_by_taskid"
                    }));
                })?),
            )
            .await?;

        self.clone()
            .create_work_session(CreateWorkSessionParams {
                title: String::new(),
                objective: String::new(),
                workspace_id: direct_task_workspace_id(&data),
                behavior: data
                    .progress
                    .as_ref()
                    .and_then(|progress| progress.execution.as_ref())
                    .and_then(|execution| execution.get("behavior"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                created_by_session_id: data
                    .request
                    .owner_session_id
                    .as_deref()
                    .map(str::to_string)
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| format!("task-{}", task.id)),
                reason_messages: vec![format!(
                    "agent.delegate task {} used direct task_id worksession creation",
                    task.id
                )],
                task_binding: None,
                task_id: Some(task.id),
                auto_start: true,
                bind_task: true,
            })
            .await?;
        Ok(())
    }

    async fn route_task_via_task_route(self: Arc<Self>, task: Task) -> Result<()> {
        let Some(task_mgr) = self.runtime.task_mgr.as_ref().cloned() else {
            return Err(anyhow!("task manager unavailable"));
        };
        let session_id = format!("task-route-{}", task.id);
        let session_dir = self.config.layout.session_dir(&session_id);
        let existing_meta = load_session_meta(&session_dir);
        let session = if let Some(meta) = existing_meta {
            self.clone()
                .ensure_session_inner(
                    session_id.clone(),
                    meta.kind,
                    meta.owner.clone(),
                    Some(meta.current_behavior.clone()),
                    Some(meta),
                )
                .await?
        } else {
            let data = agent_delegate_task_data(&task)?;
            let created_by_session_id = data
                .request
                .owner_session_id
                .as_deref()
                .map(str::to_string)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| format!("task-{}", task.id));
            let mut meta = SessionMeta::new(
                session_id.clone(),
                SessionKind::Work,
                TASK_ROUTE_BEHAVIOR.to_string(),
                created_by_session_id,
            );
            meta.title = format!("Route task {}", task.id);
            meta.objective = format!("Create a WorkSession for task {}", task.id);
            meta.workspace_id = Some(session_id.clone());
            meta.bootstrap_done = true;
            self.clone()
                .ensure_session_inner(
                    session_id.clone(),
                    SessionKind::Work,
                    meta.owner.clone(),
                    Some(TASK_ROUTE_BEHAVIOR.to_string()),
                    Some(meta),
                )
                .await?
        };

        let input_text = task_route_input_text(&task);
        session
            .enqueue_pending(PendingInput::Msg {
                record_id: format!("task-route-{}", task.id),
                from: format!("task:{}", task.id),
                from_did: None,
                from_name: Some("TaskManager".to_string()),
                tunnel_did: None,
                text: input_text.clone(),
                ai_message: AiMessage::text(AiRole::User, input_text),
            })
            .await?;
        let update_result = task_mgr
            .update_task(
                task.id,
                Some(TaskStatus::Running),
                Some(task.progress.max(1.0)),
                Some("Routing agent task via task_route".to_string()),
                Some(agent_delegate_update_value(&task, |data| {
                    data.route = Some(json!({
                        "status": "routed",
                        "strategy": "task_route_behavior",
                        "session_id": session_id,
                        "routed_at_ms": now_ms()
                    }));
                })?),
            )
            .await;
        session.wake().await;
        update_result?;
        Ok(())
    }

    async fn fail_task_route_disabled(
        self: Arc<Self>,
        task: Task,
        route_session_id: Option<String>,
    ) -> Result<()> {
        let Some(task_mgr) = self.runtime.task_mgr.as_ref().cloned() else {
            return Err(anyhow!("task manager unavailable"));
        };
        let reason =
            "agent.delegate task requires task_route, but runtime.task_route.enabled is false";
        task_mgr
            .update_task(
                task.id,
                Some(TaskStatus::Failed),
                Some(task.progress),
                Some(reason.to_string()),
                Some(agent_delegate_update_value(&task, |data| {
                    data.route = Some(json!({
                        "status": "failed",
                        "strategy": "task_route_disabled",
                        "session_id": route_session_id,
                        "reason": reason,
                        "failed_at_ms": now_ms()
                    }));
                })?),
            )
            .await?;
        Ok(())
    }

    async fn reflect_task_control_to_session(
        self: Arc<Self>,
        task: Task,
        runner: &str,
        status: &'static str,
        mode: InterruptMode,
    ) -> Result<()> {
        if !task_targets_runner(&task, runner) {
            return Ok(());
        }
        let delegate_data = agent_delegate_task_data(&task)?;
        if task_control_already_reflected(&delegate_data, status) {
            return Ok(());
        }
        let Some(task_mgr) = self.runtime.task_mgr.as_ref().cloned() else {
            return Err(anyhow!("task manager unavailable"));
        };
        let Some(session_id) = execution_session_id(&delegate_data) else {
            task_mgr
                .update_task(
                    task.id,
                    Some(task.status),
                    Some(task.progress),
                    Some(format!("Agent task {status} before session start")),
                    Some(agent_delegate_update_value(&task, |data| {
                        set_agent_delegate_execution(
                            data,
                            json!({
                                "status": status,
                                "control_observed_at_ms": now_ms(),
                            }),
                        );
                    })?),
                )
                .await?;
            return Ok(());
        };
        if let Ok(session) = self.clone().ensure_session(&session_id).await {
            if let Err(err) = session.interrupt(mode).await {
                warn!(
                    "opendan.task_executor[{}]: interrupt session {} for task {} {} failed: {err:#}",
                    self.agent_name, session_id, task.id, status
                );
            }
        }
        task_mgr
            .update_task(
                task.id,
                Some(task.status),
                Some(task.progress),
                Some(format!("Agent session {status} by task manager")),
                Some(agent_delegate_update_value(&task, |data| {
                    set_agent_delegate_execution(
                        data,
                        json!({
                            "session_id": session_id,
                            "status": status,
                            "control": {
                                "status": status,
                                "observed_at_ms": now_ms(),
                            }
                        }),
                    );
                })?),
            )
            .await?;
        Ok(())
    }

    async fn resume_waiting_delegate_task(self: Arc<Self>, task: &Task) -> Result<bool> {
        let Some(task_mgr) = self.runtime.task_mgr.as_ref().cloned() else {
            return Ok(false);
        };
        let subtasks = task_mgr.get_subtasks(task.id).await?;
        let completed = subtasks
            .iter()
            .filter(|child| child.task_type == TASK_TYPE_HUMAN_INPUT)
            .find(|child| child.status == TaskStatus::Completed);
        let Some(child) = completed else {
            return Ok(false);
        };
        let child_data = human_input_task_data(child)?;
        let response_text = human_input_response_text(&child_data)
            .unwrap_or_else(|| "Human input task was completed.".to_string());
        let Some(session_id) = execution_session_id(&agent_delegate_task_data(task)?) else {
            let response = child_data
                .result
                .as_ref()
                .and_then(|result| result.response.clone())
                .unwrap_or(Value::Null);
            task_mgr
                .update_task(
                    task.id,
                    Some(TaskStatus::Pending),
                    Some(task.progress),
                    Some("Human input received; routing task".to_string()),
                    Some(agent_delegate_update_value(task, |data| {
                        data.human_input = Some(json!({
                            "task_id": child.id,
                            "response": response,
                        }));
                    })?),
                )
                .await?;
            return Ok(true);
        };
        let session = self.clone().ensure_session(&session_id).await?;
        let record_id = format!("task-human-input-{}-{}", task.id, child.id);
        session
            .enqueue_pending(PendingInput::Msg {
                record_id,
                from: format!("task:{}", child.id),
                from_did: None,
                from_name: Some("TaskCenter".to_string()),
                tunnel_did: None,
                text: response_text.clone(),
                ai_message: AiMessage::text(AiRole::User, response_text),
            })
            .await?;
        task_mgr
            .update_task(
                task.id,
                Some(TaskStatus::Running),
                Some(task.progress.max(10.0)),
                Some("Human input received; resuming agent session".to_string()),
                Some(agent_delegate_update_value(task, |data| {
                    data.human_input = Some(json!({
                        "task_id": child.id,
                        "response": child_data
                            .result
                            .as_ref()
                            .and_then(|result| result.response.clone())
                            .unwrap_or(Value::Null),
                    }));
                })?),
            )
            .await?;
        Ok(true)
    }

    pub async fn create_human_input_task(
        self: Arc<Self>,
        parent: &Task,
        question: &str,
        kind: &str,
        candidates: Vec<Value>,
    ) -> Result<Task> {
        let Some(task_mgr) = self.runtime.task_mgr.as_ref().cloned() else {
            return Err(anyhow!("task manager unavailable"));
        };
        let existing = task_mgr.get_subtasks(parent.id).await.unwrap_or_default();
        if let Some(open) = existing.iter().find(|child| {
            child.task_type == TASK_TYPE_HUMAN_INPUT
                && child.status == TaskStatus::WaitingForApproval
        }) {
            task_mgr
                .update_task(
                    parent.id,
                    Some(TaskStatus::WaitingForApproval),
                    Some(parent.progress),
                    Some("Waiting for human input".to_string()),
                    None,
                )
                .await?;
            return Ok(open.clone());
        }
        let child = task_mgr
            .create_task(
                &format!("human-input/{}", parent.id),
                TASK_TYPE_HUMAN_INPUT,
                Some(serde_json::to_value(HumanInputTaskData {
                    request: HumanInputTaskRequest {
                        version: 1,
                        kind: kind.to_string(),
                        question: Some(question.to_string()),
                        required_by: Some(json!({
                            "task_id": parent.id,
                            "executor": self.task_executor_runner_id()?,
                        })),
                        candidates,
                        response_schema: Some(json!({ "type": "object" })),
                    },
                    result: None,
                })?),
                &parent.user_id,
                &parent.app_id,
                Some(CreateTaskOptions {
                    parent_id: Some(parent.id),
                    root_id: Some(parent.root_id.clone()),
                    session_id: Some(parent.session_id.clone()),
                    priority: None,
                    permissions: Some(parent.permissions.clone()),
                }),
            )
            .await?;
        task_mgr
            .update_task(
                child.id,
                Some(TaskStatus::WaitingForApproval),
                None,
                Some(question.to_string()),
                None,
            )
            .await?;
        task_mgr
            .update_task(
                parent.id,
                Some(TaskStatus::WaitingForApproval),
                Some(parent.progress),
                Some("Waiting for human input".to_string()),
                Some(agent_delegate_update_value(parent, |data| {
                    data.blocker = Some(json!({
                        "task_id": child.id,
                        "task_type": TASK_TYPE_HUMAN_INPUT,
                        "kind": kind,
                    }));
                })?),
            )
            .await?;
        Ok(child)
    }
}

fn workspace_id_from_hint(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_string)
        .or_else(|| {
            value
                .get("workspace_id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| value.get("id").and_then(Value::as_str).map(str::to_string))
}

fn agent_delegate_task_data(task: &Task) -> Result<AgentDelegateTaskData> {
    match parse_typed_task_data(task.task_type.as_str(), task.data.clone()) {
        Ok(TypedTaskData::AgentDelegate(data)) => Ok(data),
        Ok(other) => Err(anyhow!(
            "expected agent.delegate task data, got {:?}",
            other.task_data_type()
        )),
        Err(err) => Err(anyhow!("invalid agent.delegate task data: {}", err)),
    }
}

fn human_input_task_data(task: &Task) -> Result<HumanInputTaskData> {
    match parse_typed_task_data(task.task_type.as_str(), task.data.clone()) {
        Ok(TypedTaskData::HumanInput(data)) => Ok(data),
        Ok(other) => Err(anyhow!(
            "expected human.input task data, got {:?}",
            other.task_data_type()
        )),
        Err(err) => Err(anyhow!("invalid human.input task data: {}", err)),
    }
}

fn agent_delegate_data_value(data: AgentDelegateTaskData) -> Result<Value> {
    serde_json::to_value(data).map_err(|err| anyhow!("serialize agent.delegate task data: {}", err))
}

fn agent_delegate_update_value(
    task: &Task,
    mutate: impl FnOnce(&mut AgentDelegateTaskData),
) -> Result<Value> {
    let mut data = agent_delegate_task_data(task)?;
    mutate(&mut data);
    agent_delegate_data_value(data)
}

fn set_agent_delegate_execution(data: &mut AgentDelegateTaskData, execution: Value) {
    let progress = data
        .progress
        .get_or_insert_with(AgentDelegateProgress::default);
    progress.execution = Some(execution);
    progress.updated_at_ms = Some(now_ms() as i64);
}

fn execution_session_id(data: &AgentDelegateTaskData) -> Option<String> {
    data.progress
        .as_ref()
        .and_then(|progress| progress.execution.as_ref())
        .and_then(|execution| execution.get("session_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn delegate_execution_runner(data: &AgentDelegateTaskData) -> Option<String> {
    data.progress
        .as_ref()
        .and_then(|progress| progress.execution.as_ref())
        .and_then(|execution| execution.get("runner"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// Ownership check for delegate tasks. The executing agent lives in the task
/// payload (`progress.execution.runner`; schedule templates land there via
/// the legacy `agent_delegate.execution` mapping). A task without a target
/// agent belongs to no executor — same contract as the removed top-level
/// runner column, where every producer set the field explicitly.
fn task_targets_runner(task: &Task, runner: &str) -> bool {
    agent_delegate_task_data(task)
        .ok()
        .as_ref()
        .and_then(delegate_execution_runner)
        .map(|target| target == runner)
        .unwrap_or(false)
}

fn route_session_id(data: &AgentDelegateTaskData) -> Option<String> {
    data.route
        .as_ref()
        .and_then(|route| route.get("session_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn task_data_supports_direct_worksession(data: &AgentDelegateTaskData) -> bool {
    let objective = data
        .request
        .purpose
        .as_deref()
        .or_else(|| {
            data.request
                .input
                .as_ref()
                .and_then(|input| input.get("text"))
                .and_then(Value::as_str)
        })
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if objective.is_none() {
        return false;
    }
    if data.request.workspace_hints.len() > 1 {
        return false;
    }
    true
}

fn direct_task_workspace_id(data: &AgentDelegateTaskData) -> Option<String> {
    data.route
        .as_ref()
        .and_then(|route| route.get("workspace_id"))
        .and_then(Value::as_str)
        .or_else(|| {
            data.progress
                .as_ref()
                .and_then(|progress| progress.execution.as_ref())
                .and_then(|execution| execution.get("workspace_id"))
                .and_then(Value::as_str)
        })
        .or_else(|| {
            data.request
                .input
                .as_ref()
                .and_then(|input| input.get("workspace_id"))
                .and_then(Value::as_str)
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            if data.request.workspace_hints.len() == 1 {
                data.request
                    .workspace_hints
                    .first()
                    .and_then(workspace_id_from_hint)
            } else {
                None
            }
        })
}

fn task_control_already_reflected(data: &AgentDelegateTaskData, status: &str) -> bool {
    data.progress
        .as_ref()
        .and_then(|progress| progress.execution.as_ref())
        .and_then(|execution| execution.get("status"))
        .and_then(Value::as_str)
        .map(|value| value == status)
        .unwrap_or(false)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn human_input_response_text(data: &HumanInputTaskData) -> Option<String> {
    let response = data.result.as_ref()?.response.as_ref()?;
    response
        .as_str()
        .map(str::to_string)
        .or_else(|| {
            response
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| {
            response
                .get("text")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| serde_json::to_string_pretty(response).ok())
}

fn task_route_input_text(task: &Task) -> String {
    serde_json::to_string_pretty(&json!({
        "task_id": task.id,
        "root_task_id": task.root_id.parse::<i64>().unwrap_or(task.id),
        "root_id": task.root_id.as_str(),
        "task_type": task.task_type.as_str(),
        "task_name": task.name.as_str(),
        "user_id": task.user_id.as_str(),
        "app_id": task.app_id.as_str(),
        "parent_id": task.parent_id,
        "data": &task.data,
    }))
    .unwrap_or_else(|_| format!(r#"{{"task_id":{}}}"#, task.id))
}

fn load_session_meta(session_dir: &std::path::Path) -> Option<SessionMeta> {
    let bytes = std::fs::read(session_dir.join(".meta").join("session.json")).ok()?;
    serde_json::from_slice(&bytes).ok()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BoundWorkSession {
    session_id: String,
    workspace_id: Option<String>,
    behavior: String,
    ended: bool,
}

fn find_bound_worksession(
    sessions_dir: &std::path::Path,
    task_id: i64,
) -> Option<BoundWorkSession> {
    let entries = std::fs::read_dir(sessions_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let meta_path = path.join(".meta").join("session.json");
        let Ok(bytes) = std::fs::read(meta_path) else {
            continue;
        };
        let Ok(meta) = serde_json::from_slice::<SessionMeta>(&bytes) else {
            continue;
        };
        if !meta.kind.is_work_family() {
            continue;
        }
        let Some(binding) = meta.task_binding.as_ref() else {
            continue;
        };
        if binding.task_id != task_id {
            continue;
        }
        return Some(BoundWorkSession {
            session_id: meta.session_id,
            workspace_id: meta.workspace_id,
            behavior: meta.current_behavior,
            ended: meta.status == SessionStatus::Ended,
        });
    }
    None
}

/// Wake-up topics for the task inbox. The runner-specific task_ready inbox is
/// gone (beta2.2); task-changed events under `/task_mgr/**` remain the
/// acceleration signal, with the periodic sweep as the authoritative path.
fn task_executor_event_ids(_runner: &str) -> Vec<String> {
    vec!["/task_mgr/**".to_string()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::{TaskPermissions, TaskScope};

    fn task(data: Value) -> Task {
        Task {
            id: 7,
            user_id: "user".to_string(),
            app_id: "opendan".to_string(),
            session_id: String::new(),
            parent_id: None,
            root_id: "7".to_string(),
            name: "delegate".to_string(),
            task_type: TASK_TYPE_AGENT_DELEGATE.to_string(),
            status: TaskStatus::Pending,
            progress: 0.0,
            message: None,
            data,
            permissions: TaskPermissions {
                read: TaskScope::User,
                write: TaskScope::Private,
            },
            created_at: 1,
            updated_at: 1,
        }
    }

    #[test]
    fn direct_schema_uses_single_workspace_hint() {
        let task = task(json!({
            "agent_delegate": {
                "purpose": "Do the task",
                "workspace_hints": [{"workspace_id": "buckyos"}]
            }
        }));
        let data = agent_delegate_task_data(&task).expect("agent.delegate task data");
        assert!(task_data_supports_direct_worksession(&data));
        assert_eq!(direct_task_workspace_id(&data).as_deref(), Some("buckyos"));
    }

    #[test]
    fn ambiguous_workspace_hints_are_not_direct() {
        let task = task(json!({
            "agent_delegate": {
                "purpose": "Do the task",
                "workspace_hints": ["a", "b"]
            }
        }));
        let data = agent_delegate_task_data(&task).expect("agent.delegate task data");
        assert!(!task_data_supports_direct_worksession(&data));
    }

    #[test]
    fn unrecognized_task_data_is_not_direct() {
        let task = task(json!({
            "input": "free-form task"
        }));
        assert!(agent_delegate_task_data(&task).is_err());
    }

    #[test]
    fn task_executor_subscribes_task_changes_only() {
        assert_eq!(
            task_executor_event_ids("agent"),
            vec!["/task_mgr/**".to_string()]
        );
    }

    #[test]
    fn delegate_ownership_comes_from_execution_runner() {
        // Target agent rides in the payload; a task without one belongs to
        // no executor (same contract as the removed runner column).
        let mine = task(json!({
            "agent_delegate": {
                "purpose": "Do the task",
                "execution": {"runner": "agent"}
            }
        }));
        assert!(task_targets_runner(&mine, "agent"));
        assert!(!task_targets_runner(&mine, "other-agent"));

        let unassigned = task(json!({
            "agent_delegate": {
                "purpose": "Do the task"
            }
        }));
        assert!(!task_targets_runner(&unassigned, "agent"));
    }

    #[test]
    fn finds_existing_bound_worksession_by_task_id() {
        let dir = tempfile::tempdir().expect("tempdir");
        let session_dir = dir.path().join("ws-bound").join(".meta");
        std::fs::create_dir_all(&session_dir).expect("mkdir meta");
        let mut meta = SessionMeta::new(
            "ws-bound".to_string(),
            crate::session_model::SessionKind::Work,
            "work_default".to_string(),
            "owner".to_string(),
        );
        meta.workspace_id = Some("workspace-1".to_string());
        meta.task_binding = Some(crate::session_model::AgentTaskBinding {
            task_id: 7,
            root_task_id: 7,
            root_id: "7".to_string(),
            task_type: TASK_TYPE_AGENT_DELEGATE.to_string(),
            runner: "agent".to_string(),
            task_name: "delegate".to_string(),
            user_id: "user".to_string(),
            app_id: "opendan".to_string(),
            parent_id: None,
        });
        std::fs::write(
            session_dir.join("session.json"),
            serde_json::to_vec_pretty(&meta).expect("serialize meta"),
        )
        .expect("write meta");

        assert_eq!(
            find_bound_worksession(dir.path(), 7),
            Some(BoundWorkSession {
                session_id: "ws-bound".to_string(),
                workspace_id: Some("workspace-1".to_string()),
                behavior: "work_default".to_string(),
                ended: false,
            })
        );
    }
}
