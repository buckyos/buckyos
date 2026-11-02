//! Tasks of the zone's TaskMgr (`t-` ids) as a [`RunningTaskResolver`]:
//! the end of work a tool handed to a krpc service that answers with a task
//! (`ask_user`'s `human.input/v1`, AICC, …). This process only reads them
//! (state, wait, cancel request); it never runs them.
//!
//! `shell` commands are never handed to TaskMgr: they stay tasks of the
//! process that runs them (`local:shell:<call_id>`, [`crate::tasks`];
//! simple Work Session TODO Q1).
//!
//! A terminal task's result is rendered by one rule (freeze TODO F7): a
//! result that is an AgentToolResult envelope is that envelope, anything
//! else is its JSON as the text; a `tool.shell/v1` task (another runner's)
//! in its own terms.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value as Json};

use crate::llm_bash::TOOL_SHELL;
use crate::result_view::ToolResultView;
use crate::task_model::{
    CancelUnsupported, RunningTaskResolver, TaskBrief, TaskResult, TaskState, MAX_IN_TOOL_WAIT_MS,
};
use crate::tasks::compact;

/// Prefix of every TaskMgr task id (`t-<32 hex>`).
pub const ZONE_TASK_PREFIX: &str = "t-";
/// How often a wait reads a task again.
const WAIT_POLL: Duration = Duration::from_millis(1_000);

/// A TaskMgr task as this module reads it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ZoneTaskRecord {
    pub task_id: String,
    pub schema_id: Option<String>,
    pub name: String,
    pub input: Json,
    pub terminal: bool,
    /// `succeeded` | `failed` | `canceled` once terminal.
    pub outcome: Option<String>,
    pub result: Option<Json>,
    /// `{code, message, detail}` of a failed task.
    pub error: Option<Json>,
    pub message: Option<String>,
    pub progress: Option<Json>,
    /// A cancel waits for the runner: its request id.
    pub cancel_requested: Option<String>,
    pub parent_id: Option<String>,
}

/// TaskMgr, as far as reading tasks needs it.
#[async_trait]
pub trait ZoneTaskBackend: Send + Sync {
    /// `None`: no such task.
    async fn get(&self, task_id: &str) -> Result<Option<ZoneTaskRecord>, String>;
    /// Ask for a cancel (the task's runner applies it).
    async fn request_cancel(&self, task_id: &str) -> Result<(), String>;
}

/// The resolver of `t-` tasks.
pub struct ZoneTaskResolver {
    backend: Arc<dyn ZoneTaskBackend>,
    /// Terminal tasks whose result was read once.
    read: Mutex<BTreeSet<String>>,
}

impl ZoneTaskResolver {
    pub fn new(backend: Arc<dyn ZoneTaskBackend>) -> Arc<Self> {
        Arc::new(Self {
            backend,
            read: Mutex::new(BTreeSet::new()),
        })
    }

    pub fn is_zone_task(task_id: &str) -> bool {
        task_id.starts_with(ZONE_TASK_PREFIX)
    }

    fn mark_read(&self, task_id: &str) {
        self.read
            .lock()
            .expect("zone tasks")
            .insert(task_id.to_string());
    }

    async fn state_of(&self, task_id: &str) -> TaskState {
        match self.backend.get(task_id).await {
            Ok(Some(rec)) if rec.terminal => {
                self.mark_read(task_id);
                TaskState::Finished(result_of(&rec))
            }
            Ok(Some(rec)) => {
                let started = rec
                    .input
                    .get("started_at_ms")
                    .and_then(Json::as_u64)
                    .unwrap_or_else(crate::now_ms);
                let brief = if rec.is_shell() {
                    format!("shell: {}", compact(command_of(&rec), 80))
                } else {
                    compact(rec.message.as_deref().unwrap_or(&rec.name), 80)
                };
                TaskState::Running {
                    brief,
                    elapsed_ms: crate::now_ms().saturating_sub(started),
                    output_tail: rec
                        .progress
                        .as_ref()
                        .and_then(|p| p.get("output_tail"))
                        .and_then(Json::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    cancellable: rec.cancel_requested.is_none(),
                }
            }
            Ok(None) => TaskState::Unknown {
                reason: format!("task {task_id} is not known to TaskMgr"),
            },
            Err(e) => TaskState::Unknown {
                reason: format!("TaskMgr is not reachable: {e}"),
            },
        }
    }
}

#[async_trait]
impl RunningTaskResolver for ZoneTaskResolver {
    async fn state(&self, task_id: &str) -> TaskState {
        self.state_of(task_id).await
    }

    async fn wait(&self, task_id: &str, until_ms: Option<u64>) -> TaskState {
        let until = until_ms.unwrap_or_else(|| crate::now_ms() + MAX_IN_TOOL_WAIT_MS);
        loop {
            let st = self.state_of(task_id).await;
            if !matches!(st, TaskState::Running { .. }) || crate::now_ms() >= until {
                return st;
            }
            tokio::time::sleep(WAIT_POLL.min(Duration::from_millis(
                until.saturating_sub(crate::now_ms()).max(1),
            )))
            .await;
        }
    }

    async fn cancel(&self, task_id: &str) -> Result<TaskState, CancelUnsupported> {
        self.backend
            .request_cancel(task_id)
            .await
            .map_err(|_| CancelUnsupported {
                task_id: task_id.to_string(),
            })?;
        Ok(self.state_of(task_id).await)
    }

    fn can_resolve(&self, task_id: &str) -> bool {
        Self::is_zone_task(task_id)
    }

    /// Nothing of its own: the run's resolver lists the `t-` tasks it
    /// follows.
    async fn active(&self) -> Vec<TaskBrief> {
        Vec::new()
    }

    fn result_seen(&self, task_id: &str) -> bool {
        self.read.lock().expect("zone tasks").contains(task_id)
    }
}

fn command_of(rec: &ZoneTaskRecord) -> &str {
    rec.input
        .get("command")
        .and_then(Json::as_str)
        .unwrap_or(rec.name.as_str())
}

impl ZoneTaskRecord {
    fn is_shell(&self) -> bool {
        self.schema_id
            .as_deref()
            .map_or(true, |s| s == buckyos_api::TOOL_SHELL_TASK_SCHEMA_ID)
    }
}

/// The result of a terminal task, from what TaskMgr holds: a `shell` task
/// in its own terms; any other task by one rule (freeze TODO F7) — a result
/// that is an AgentToolResult envelope is that envelope, anything else is
/// its JSON as the text.
fn result_of(rec: &ZoneTaskRecord) -> TaskResult {
    if !rec.is_shell() {
        return generic_result_of(rec);
    }
    let command = command_of(rec).to_string();
    let detail = rec
        .result
        .clone()
        .or_else(|| rec.error.as_ref().and_then(|e| e.get("detail").cloned()))
        .unwrap_or(Json::Null);
    let exit_code = detail.get("exit_code").and_then(Json::as_i64).map(|c| c as i32);
    let duration_ms = detail.get("duration_ms").and_then(Json::as_u64).unwrap_or(0);
    let output = detail.get("output").and_then(Json::as_str).unwrap_or("");
    let outcome = rec.outcome.as_deref().unwrap_or("failed");
    let mut text = match (outcome, exit_code) {
        ("canceled", _) => format!("`{command}` was cancelled after {duration_ms}ms."),
        (_, Some(code)) => format!("`{command}` exited with code {code} after {duration_ms}ms."),
        ("succeeded", None) => format!("`{command}` completed."),
        (_, None) => format!(
            "`{command}` failed: {}",
            rec.error
                .as_ref()
                .and_then(|e| e.get("message"))
                .and_then(Json::as_str)
                .unwrap_or("no detail")
        ),
    };
    if !output.trim().is_empty() {
        text.push_str("\n--- output ---\n");
        text.push_str(output.trim_end());
    }
    let summary = match (outcome, exit_code) {
        ("canceled", _) => "cancelled".to_string(),
        (_, Some(code)) => format!("exit={code} in {duration_ms}ms"),
        _ => outcome.to_string(),
    };
    TaskResult {
        success: outcome == "succeeded",
        output: text,
        tool_result: Some(ToolResultView {
            cmd_name: Some(TOOL_SHELL.into()),
            cmd_args: Some(compact(&command, 80)),
            return_code: exit_code,
            summary,
            detail: Some(detail),
            ..Default::default()
        }),
    }
}

fn generic_result_of(rec: &ZoneTaskRecord) -> TaskResult {
    let outcome = rec.outcome.as_deref().unwrap_or("failed");
    let success = outcome == "succeeded";
    let payload = rec
        .result
        .clone()
        .or_else(|| rec.error.clone())
        .unwrap_or(Json::Null);
    if let Some(envelope) = payload
        .get("agent_tool_protocol")
        .and_then(|_| ToolResultView::from_host(&Some(payload.clone())))
    {
        let output = envelope
            .output
            .clone()
            .filter(|o| !o.trim().is_empty())
            .or_else(|| envelope.detail.as_ref().map(|d| {
                serde_json::to_string_pretty(d).unwrap_or_else(|_| d.to_string())
            }))
            .unwrap_or_else(|| envelope.summary.clone());
        return TaskResult {
            success: success && envelope.status != crate::result_view::ToolResultStatusView::Error,
            output,
            tool_result: Some(envelope),
        };
    }
    let body = match &payload {
        Json::Null => String::new(),
        Json::String(s) => s.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
    };
    let head = match outcome {
        "canceled" => format!("task {} ({}) was cancelled.", rec.task_id, rec.name),
        "succeeded" => format!("task {} ({}) completed.", rec.task_id, rec.name),
        _ => format!("task {} ({}) failed.", rec.task_id, rec.name),
    };
    TaskResult {
        success,
        output: if body.trim().is_empty() {
            head
        } else {
            format!("{head}\n{body}")
        },
        tool_result: Some(ToolResultView {
            summary: outcome.to_string(),
            detail: (!payload.is_null()).then_some(payload),
            ..Default::default()
        }),
    }
}

// ---------------------------------------------------------------------------
// TaskMgr of the zone
// ---------------------------------------------------------------------------

/// TaskMgr of the zone this process runs in: the client is taken from the
/// runtime for every call (its session token is renewed).
pub struct ZoneTaskMgrBackend;

#[cfg(not(target_arch = "wasm32"))]
mod zone {
    use super::*;
    use buckyos_api::{Task, TaskControlAction, TaskManagerClient, TaskOutcome};

    async fn client() -> Result<TaskManagerClient, String> {
        buckyos_api::get_buckyos_api_runtime()
            .map_err(|e| e.to_string())?
            .get_task_mgr_client()
            .await
            .map_err(|e| e.to_string())
    }

    fn record(task: &Task) -> ZoneTaskRecord {
        ZoneTaskRecord {
            task_id: task.task_id.clone(),
            schema_id: Some(task.schema_id.clone()),
            name: task.name.clone(),
            input: task.input.clone(),
            terminal: task.phase.is_terminal(),
            outcome: task.outcome.map(|o| match o {
                TaskOutcome::Succeeded => "succeeded".to_string(),
                TaskOutcome::Failed => "failed".to_string(),
                TaskOutcome::Canceled => "canceled".to_string(),
            }),
            result: task.result.clone(),
            error: task
                .error
                .as_ref()
                .map(|e| json!({ "code": e.code, "message": e.message, "detail": e.detail })),
            message: task.message.clone(),
            progress: task.progress.clone(),
            cancel_requested: task
                .pending_control
                .as_ref()
                .filter(|c| c.action == TaskControlAction::Cancel)
                .map(|c| c.request_id.clone()),
            parent_id: task.parent_id.clone(),
        }
    }

    #[async_trait]
    impl ZoneTaskBackend for ZoneTaskMgrBackend {
        async fn get(&self, task_id: &str) -> Result<Option<ZoneTaskRecord>, String> {
            match client().await?.get_task(task_id).await {
                Ok(task) => Ok(Some(record(&task))),
                Err(e)
                    if buckyos_api::task_mgr_error_code(&e)
                        == Some(buckyos_api::TASK_ERR_NOT_FOUND) =>
                {
                    Ok(None)
                }
                Err(e) => Err(e.to_string()),
            }
        }

        async fn request_cancel(&self, task_id: &str) -> Result<(), String> {
            client()
                .await?
                .cancel_task(task_id, false)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// In-memory backend (tests, hosts without a zone)
// ---------------------------------------------------------------------------

/// A TaskMgr in memory: tasks are put there and ended by the test (or the
/// host) playing their runner.
#[derive(Default)]
pub struct MemoryTaskBackend {
    inner: Mutex<BTreeMap<String, ZoneTaskRecord>>,
}

impl MemoryTaskBackend {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn record(&self, task_id: &str) -> Option<ZoneTaskRecord> {
        self.inner.lock().expect("memory tasks").get(task_id).cloned()
    }

    pub fn records(&self) -> Vec<ZoneTaskRecord> {
        self.inner.lock().expect("memory tasks").values().cloned().collect()
    }

    /// Put a task there (running unless `terminal`).
    pub fn seed(&self, record: ZoneTaskRecord) {
        self.inner
            .lock()
            .expect("memory tasks")
            .insert(record.task_id.clone(), record);
    }

    fn update(&self, task_id: &str, f: impl FnOnce(&mut ZoneTaskRecord)) -> Result<(), String> {
        let mut s = self.inner.lock().expect("memory tasks");
        let rec = s
            .get_mut(task_id)
            .ok_or_else(|| format!("task {task_id} not found"))?;
        if !rec.terminal {
            f(rec);
        }
        Ok(())
    }

    /// The runner completes the task with `result`.
    pub fn complete(&self, task_id: &str, result: Json) -> Result<(), String> {
        self.update(task_id, |r| {
            r.terminal = true;
            r.outcome = Some("succeeded".into());
            r.result = Some(result);
        })
    }

    /// The runner fails the task.
    pub fn fail(&self, task_id: &str, code: &str, message: &str) -> Result<(), String> {
        self.update(task_id, |r| {
            r.terminal = true;
            r.outcome = Some("failed".into());
            r.error = Some(json!({ "code": code, "message": message }));
        })
    }
}

#[async_trait]
impl ZoneTaskBackend for MemoryTaskBackend {
    async fn get(&self, task_id: &str) -> Result<Option<ZoneTaskRecord>, String> {
        Ok(self.record(task_id))
    }

    async fn request_cancel(&self, task_id: &str) -> Result<(), String> {
        self.update(task_id, |r| {
            r.terminal = true;
            r.outcome = Some("canceled".into());
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn zone_tasks_are_read_until_they_end() {
        let backend = MemoryTaskBackend::new();
        backend.seed(ZoneTaskRecord {
            task_id: "t-1".into(),
            schema_id: Some("human.input/v1".into()),
            name: "ask".into(),
            message: Some("which one?".into()),
            ..Default::default()
        });
        let resolver = ZoneTaskResolver::new(backend.clone());
        assert!(resolver.can_resolve("t-1"));
        assert!(!resolver.can_resolve("local:shell:c1"));
        assert!(matches!(
            resolver.state("t-1").await,
            TaskState::Running { ref brief, .. } if brief == "which one?"
        ));
        assert!(!resolver.result_seen("t-1"));
        let waiting = {
            let r = resolver.clone();
            tokio::spawn(async move { r.wait("t-1", Some(crate::now_ms() + 10_000)).await })
        };
        backend
            .complete("t-1", json!({"action": "submit", "values": {"x": "blue"}}))
            .unwrap();
        let TaskState::Finished(r) = waiting.await.unwrap() else {
            panic!()
        };
        assert!(r.success);
        assert!(r.output.contains("blue"), "{}", r.output);
        assert!(resolver.result_seen("t-1"));
        assert!(matches!(
            resolver.state("t-nope").await,
            TaskState::Unknown { .. }
        ));
    }

    /// A task of another service (F7): its result is an AgentToolResult
    /// envelope, or JSON shown as the text — never read as a shell task.
    #[test]
    fn other_tasks_render_by_the_generic_rule() {
        let envelope = ZoneTaskRecord {
            task_id: "t-1".into(),
            schema_id: Some("human.input/v1".into()),
            name: "ask".into(),
            terminal: true,
            outcome: Some("succeeded".into()),
            result: Some(json!({
                "agent_tool_protocol": "1",
                "status": "success",
                "summary": "answered",
                "output": "use the blue one",
                "detail": {}
            })),
            ..Default::default()
        };
        let r = result_of(&envelope);
        assert!(r.success);
        assert_eq!(r.output, "use the blue one");
        assert_eq!(r.tool_result.unwrap().summary, "answered");
        let plain = ZoneTaskRecord {
            result: Some(json!({"obj_id": "cyfile:1"})),
            ..envelope
        };
        let r = result_of(&plain);
        assert!(r.success);
        assert!(r.output.contains("completed") && r.output.contains("cyfile:1"), "{}", r.output);
    }
}