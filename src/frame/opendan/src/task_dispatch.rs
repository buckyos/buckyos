//! §9.4 PendingTool dispatch — task_mgr-backed async tool execution.
//!
//! Skeleton for the §9.4 residual item:
//!
//! > `PendingTool` outcome 的 async tool dispatch（需 `task_mgr` 接入）
//!
//! When the `LLMContext::run()` returns `Outcome::PendingTool` the waist
//! has decided that one or more tool calls should NOT block the session
//! worker — typically because they're long-running (downloads, builds) or
//! they're scheduled work the LLM expects results from later. The agent
//! parks the corresponding tool ids on the snapshot and yields; once the
//! task completes the agent feeds the results back via
//! `ResumeFill::ToolResults` to continue inference.
//!
//! This module is the *seam* between that pending-tool list and BuckyOS's
//! task_mgr. The shape:
//!
//! ```text
//!   LLMContext::run → Outcome::PendingTool { pending, snapshot, .. }
//!         │
//!         ▼
//!   AgentSession::handle_outcome
//!         │  TaskDispatch::dispatch_async_tool(session_id, call) →
//!         │    task_mgr.create_task(name=tool_name, type="opendan/tool",
//!         │                         data=<args JSON>, parent=session.task_root)
//!         │
//!         ▼
//!   later: task_mgr emits a completion kevent / RPC →
//!         AgentSession sees Inbound::Event matching its subscription →
//!         worker feeds ResumeFill::ToolResults into the next turn.
//! ```
//!
//! The current iteration only ships the `dispatch_async_tool` entry-point
//! and a `mark_task_completed` helper used by the receive path. The
//! end-to-end loop (subscription / matching / ResumeFill assembly) lands
//! once the `PendingTool` outcome surface is exercised — this module
//! prevents the seam from being a no-op shim sprayed across the session
//! file.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use buckyos_api::{
    get_buckyos_api_runtime, CommitResultReq, CreateTaskExecutor, CreateTaskReq, FailTaskReq,
    OpenDanAsyncToolTaskData, RunnerWriteEnvelope, Task, TaskError, TaskManagerClient,
};
use log::warn;
use serde_json::Value;

/// Task type tag used for opendan-dispatched async tools. Lets task_mgr
/// (and any introspection UI on top of it) filter / group these alongside
/// other workloads. Stable string — keep in sync with whatever subscriber
/// downstream filters by.
pub const TASK_TYPE_OPENDAN_TOOL: &str = "opendan.async_tool";
/// Versioned task schema id for async tool tasks (2.0).
pub const OPENDAN_TOOL_SCHEMA_ID: &str = "opendan.async_tool/v1";

/// Default app id we tag dispatched tasks with. Real deployments will
/// override this via [`TaskDispatch::with_app_id`] so multi-app installs
/// can route tasks correctly.
const DEFAULT_APP_ID: &str = "opendan";

/// OpenDAN-specific TaskManager surface. Production instances acquire a
/// short-session client from the runtime for each operation; fixed clients
/// are reserved for tests or explicit token management.
#[derive(Clone)]
pub struct TaskDispatch {
    client: Option<Arc<TaskManagerClient>>,
    user_id: String,
    app_id: String,
}

impl TaskDispatch {
    pub fn new(client: Arc<TaskManagerClient>, user_id: impl Into<String>) -> Self {
        Self {
            client: Some(client),
            user_id: user_id.into(),
            app_id: DEFAULT_APP_ID.to_string(),
        }
    }

    pub fn from_runtime(
        client_override: Option<Arc<TaskManagerClient>>,
        user_id: impl Into<String>,
    ) -> Self {
        Self {
            client: client_override,
            user_id: user_id.into(),
            app_id: DEFAULT_APP_ID.to_string(),
        }
    }

    async fn client(&self) -> Result<Arc<TaskManagerClient>> {
        if let Some(client) = self.client.as_ref() {
            return Ok(client.clone());
        }
        let runtime = get_buckyos_api_runtime().map_err(|err| anyhow!(err.to_string()))?;
        runtime
            .get_task_mgr_client()
            .await
            .map(Arc::new)
            .map_err(|err| anyhow!(err.to_string()))
    }

    pub fn with_app_id(mut self, app_id: impl Into<String>) -> Self {
        self.app_id = app_id.into();
        self
    }

    /// Create a task representing an async tool dispatch. Returns the
    /// task's id — the session stores it on the snapshot's pending list so
    /// completion events can be reconciled.
    ///
    /// `tool_name` is the agent-side tool identifier (e.g. `"download"`,
    /// `"build_project"`); `payload` is whatever blob the tool's worker
    /// needs to do its job. The worker is *not* invoked here — we only
    /// create the task record. Whoever consumes the task_mgr backend is
    /// responsible for running it and writing the result back.
    pub async fn dispatch_async_tool(
        &self,
        session_id: &str,
        tool_name: &str,
        payload: Value,
    ) -> Result<DispatchedTask> {
        let task_name = format!("opendan/{session_id}/{tool_name}");
        let task = self
            .client()
            .await?
            .create_task(CreateTaskReq {
                name: task_name.clone(),
                schema_id: OPENDAN_TOOL_SCHEMA_ID.to_string(),
                schema_version: None,
                // The session id rides in the immutable input so future
                // worksession code can still group tasks by session.
                input: serde_json::to_value(OpenDanAsyncToolTaskData {
                    request: serde_json::json!({
                        "session_id": session_id,
                        "payload": payload,
                    }),
                    result: None,
                })?,
                executor: CreateTaskExecutor::SelfApp {
                    app_instance_id: None,
                },
                parent_id: None,
                child_control_policy: None,
                policy_preset: None,
                permission_boundary: false,
                idempotency_key: format!("opendan-tool-{}", uuid::Uuid::new_v4().simple()),
                retry_of: None,
                supersedes: None,
                message: None,
            })
            .await
            .map_err(|err| anyhow!("create_task `{task_name}` failed: {err}"))?;
        Ok(DispatchedTask {
            task_id: task.task_id.clone(),
            task,
        })
    }

    /// Mark a previously-dispatched task as completed (success or failure)
    /// — used when the session worker has already absorbed the tool
    /// result and wants to release the task_mgr slot. Errors are
    /// warn-logged but not propagated: by the time we reach this point
    /// the result is already in the LLM context, so we don't want a
    /// task_mgr glitch to fail the turn.
    pub async fn mark_task_completed(&self, task_id: &str, success: bool) {
        let client = match self.client().await {
            Ok(client) => client,
            Err(err) => {
                warn!(
                    "opendan.task_dispatch: get task-manager client for task {task_id} failed: {err}"
                );
                return;
            }
        };
        let task = match client.get_task(task_id).await {
            Ok(task) => task,
            Err(err) => {
                warn!("opendan.task_dispatch: get_task({task_id}) failed: {err}");
                return;
            }
        };
        if task.phase.is_terminal() {
            return;
        }
        let result = if success {
            client
                .commit_result(CommitResultReq {
                    task_id: task.task_id.clone(),
                    result: serde_json::json!({"absorbed": true}),
                    app_instance_id: None,
                    runner_epoch: Some(task.runner_epoch),
                    expected_revision: task.revision,
                })
                .await
                .map(|_| ())
        } else {
            client
                .fail_task(FailTaskReq {
                    envelope: RunnerWriteEnvelope {
                        task_id: task.task_id.clone(),
                        app_instance_id: None,
                        runner_epoch: task.runner_epoch,
                        expected_revision: task.revision,
                    },
                    error: TaskError::new("tool_failed", "async tool reported failure"),
                })
                .await
                .map(|_| ())
        };
        if let Err(err) = result {
            warn!("opendan.task_dispatch: close task {task_id} (success={success}) failed: {err}");
        }
    }
}

/// The handle returned by `dispatch_async_tool` — the session stores
/// `task_id` on its snapshot's pending list; `task` carries the freshly
/// minted record for logging.
pub struct DispatchedTask {
    pub task_id: String,
    pub task: Task,
}

#[cfg(test)]
mod tests {
    use super::*;

    // No-RPC unit tests — the dispatch flow needs a real (or mocked)
    // TaskManagerClient which is heavy to spin up for a skeleton test.
    // We assert the surface constants are stable here; integration tests
    // for the dispatch path land alongside the §9.4 PendingTool wiring.

    #[test]
    fn task_type_tag_is_stable() {
        assert_eq!(TASK_TYPE_OPENDAN_TOOL, "opendan.async_tool");
    }

    #[test]
    fn default_app_id_is_opendan() {
        assert_eq!(DEFAULT_APP_ID, "opendan");
    }
}
