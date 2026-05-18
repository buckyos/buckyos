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
use buckyos_api::{CreateTaskOptions, Task, TaskManagerClient, TaskStatus};
use log::warn;
use serde_json::Value;

/// Task type tag used for opendan-dispatched async tools. Lets task_mgr
/// (and any introspection UI on top of it) filter / group these alongside
/// other workloads. Stable string — keep in sync with whatever subscriber
/// downstream filters by.
pub const TASK_TYPE_OPENDAN_TOOL: &str = "opendan.async_tool";

/// Default app id we tag dispatched tasks with. Real deployments will
/// override this via [`TaskDispatch::with_app_id`] so multi-app installs
/// can route tasks correctly.
const DEFAULT_APP_ID: &str = "opendan";

/// Wrapper over `TaskManagerClient` carving out an opendan-specific
/// surface. Cloning is cheap (Arc) so handing a clone to each
/// AgentSession is fine.
#[derive(Clone)]
pub struct TaskDispatch {
    client: Arc<TaskManagerClient>,
    user_id: String,
    app_id: String,
}

impl TaskDispatch {
    pub fn new(client: Arc<TaskManagerClient>, user_id: impl Into<String>) -> Self {
        Self {
            client,
            user_id: user_id.into(),
            app_id: DEFAULT_APP_ID.to_string(),
        }
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
        let opts = CreateTaskOptions {
            // Parenting under the session's logical id lets future
            // worksession code group tasks by session for cancel / cleanup.
            parent_id: None,
            root_id: None,
            priority: None,
            permissions: None,
        };
        let task = self
            .client
            .create_task(
                &task_name,
                TASK_TYPE_OPENDAN_TOOL,
                Some(payload),
                &self.user_id,
                &self.app_id,
                Some(opts),
            )
            .await
            .map_err(|err| anyhow!("create_task `{task_name}` failed: {err}"))?;
        Ok(DispatchedTask {
            task_id: task.id,
            task,
        })
    }

    /// Mark a previously-dispatched task as completed (success or failure)
    /// — used when the session worker has already absorbed the tool
    /// result and wants to release the task_mgr slot. Errors are
    /// warn-logged but not propagated: by the time we reach this point
    /// the result is already in the LLM context, so we don't want a
    /// task_mgr glitch to fail the turn.
    pub async fn mark_task_completed(&self, task_id: i64, success: bool) {
        let status = if success {
            TaskStatus::Completed
        } else {
            TaskStatus::Failed
        };
        if let Err(err) = self.client.update_task_status(task_id, status).await {
            warn!("opendan.task_dispatch: update_task_status({task_id}, {status:?}) failed: {err}");
        }
    }
}

/// The handle returned by `dispatch_async_tool` — the session stores
/// `task_id` on its snapshot's pending list; `task` carries the freshly
/// minted record for logging.
pub struct DispatchedTask {
    pub task_id: i64,
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
