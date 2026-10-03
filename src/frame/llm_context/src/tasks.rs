//! Running tasks: work that outlives one tool call and is addressed by a
//! `task_id` (see `notepads/llm-context-long-tool-todo.md` §4).
//!
//! The waist makes exactly two mechanical decisions about a tool result:
//! return it to the LLM, or — when the tool answered `Observation::Pending
//! { task_id, .. }` and the host allows it — suspend the run so the host
//! waits for the task outside the process. Everything else (which task
//! manager owns a task, how it is waited for or cancelled) sits behind
//! [`RunningTaskResolver`], assembled by the host.
//!
//! One render function turns a [`TaskState`] into the observation the LLM
//! sees, so the inline `wait_task` / `get_task_state` tools and the fill of
//! a suspended call produce identical results.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::observation::{Observation, ToolResultStatusView, ToolResultView};

/// Longest a tool call may wait for a task inside the waist: whatever is
/// waited for, the call returns to the LLM with the task's state by then.
pub const MAX_IN_TOOL_WAIT_MS: u64 = 30 * 60_000;

/// Default `wait_ms` of `wait_task` / of the `shell` auto mode.
pub const DEFAULT_TASK_WAIT_MS: u64 = 30_000;

/// Terminal result of a task, in the shape of a tool result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskResult {
    pub success: bool,
    /// Text the LLM sees (output or summary).
    pub output: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_result: Option<ToolResultView>,
}

/// State of a task as reported by its task manager.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaskState {
    Running {
        /// One-line description (e.g. the command).
        brief: String,
        #[serde(default)]
        elapsed_ms: u64,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        output_tail: String,
        /// Whether `cancel` can stop it reliably.
        #[serde(default)]
        cancellable: bool,
    },
    Finished(TaskResult),
    /// The task manager cannot tell (restarted, unreachable, unknown id).
    Unknown {
        reason: String,
    },
}

/// One line of the background env.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskBrief {
    pub task_id: String,
    pub brief: String,
    /// `running | finished | unknown`.
    pub status: String,
    #[serde(default)]
    pub elapsed_ms: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub last_line: String,
    #[serde(default)]
    pub cancellable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelUnsupported {
    pub task_id: String,
}

impl std::fmt::Display for CancelUnsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "task `{}` does not support cancellation", self.task_id)
    }
}

/// Host-assembled view on the tasks this context may wait for. Task ids are
/// opaque to the waist; dispatching to the owning task manager is the
/// resolver's business.
#[async_trait]
pub trait RunningTaskResolver: Send + Sync {
    async fn state(&self, task_id: &str) -> TaskState;

    /// Wait until the task ends or `until_ms` (epoch ms) passes; the state
    /// at that moment is returned.
    async fn wait(&self, task_id: &str, until_ms: Option<u64>) -> TaskState;

    /// Only a task that declared itself cancellable can be cancelled.
    async fn cancel(&self, task_id: &str) -> Result<TaskState, CancelUnsupported>;

    /// Whether this resolver can answer for `task_id` at all (a resolver
    /// without access to the owning task manager says `false`; the executor
    /// then refuses to take the run over).
    fn can_resolve(&self, task_id: &str) -> bool {
        let _ = task_id;
        true
    }

    /// Register a task this context is interested in (a result carried its
    /// `task_id`). Default: no-op.
    fn watch(&self, task_id: &str) {
        let _ = task_id;
    }

    /// Tasks this context follows, for the background env. A finished task
    /// stays listed until the LLM read its result once.
    async fn active(&self) -> Vec<TaskBrief>;
}

/// Hint appended to a still-running task's result.
pub fn next_step_hint(task_id: &str, cancellable: bool) -> String {
    let mut s = format!(
        "Call `wait_task` with task_id=\"{task_id}\" to keep waiting (wait_ms up to {} min), or `get_task_state` to check later.",
        MAX_IN_TOOL_WAIT_MS / 60_000
    );
    if cancellable {
        s.push_str(&format!(
            " `cancel_task` with task_id=\"{task_id}\" stops it."
        ));
    }
    s
}

fn fmt_elapsed(ms: u64) -> String {
    let s = ms / 1000;
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m{:02}s", s / 60, s % 60)
    } else {
        format!("{}h{:02}m", s / 3600, (s % 3600) / 60)
    }
}

/// The observation the LLM sees for `state` of `task_id`, answering
/// `call_id`. Used for the inline task tools and for the fill of a
/// suspended call alike.
pub fn task_state_observation(call_id: &str, task_id: &str, state: &TaskState) -> Observation {
    match state {
        TaskState::Running {
            brief,
            elapsed_ms,
            output_tail,
            cancellable,
        } => {
            let mut text = format!(
                "task {task_id} is still running ({}): {brief}",
                fmt_elapsed(*elapsed_ms)
            );
            if !output_tail.trim().is_empty() {
                text.push_str("\n--- output so far (tail) ---\n");
                text.push_str(output_tail.trim_end());
            }
            text.push('\n');
            text.push_str(&next_step_hint(task_id, *cancellable));
            let bytes = text.len();
            Observation::Success {
                call_id: call_id.to_string(),
                content: Value::String(text),
                bytes,
                truncated: false,
                tool_result: Some(ToolResultView {
                    status: ToolResultStatusView::Success,
                    task_id: Some(task_id.to_string()),
                    summary: format!("task {task_id} still running"),
                    partial_output: Some(output_tail.clone()).filter(|s| !s.is_empty()),
                    ..Default::default()
                }),
            }
        }
        TaskState::Finished(result) => {
            let mut tool_result = result.tool_result.clone().unwrap_or_default();
            tool_result.task_id = Some(task_id.to_string());
            if result.success {
                let bytes = result.output.len();
                tool_result.status = ToolResultStatusView::Success;
                Observation::Success {
                    call_id: call_id.to_string(),
                    content: Value::String(result.output.clone()),
                    bytes,
                    truncated: false,
                    tool_result: Some(tool_result),
                }
            } else {
                tool_result.status = ToolResultStatusView::Error;
                Observation::Error {
                    call_id: call_id.to_string(),
                    message: if result.output.trim().is_empty() {
                        format!("task {task_id} failed")
                    } else {
                        result.output.clone()
                    },
                    tool_result: Some(tool_result),
                }
            }
        }
        TaskState::Unknown { reason } => Observation::Error {
            call_id: call_id.to_string(),
            message: format!(
                "task {task_id}: state unknown ({reason}). Its result was not recorded; check the actual state (files, processes, logs) before repeating the work."
            ),
            tool_result: Some(ToolResultView {
                status: ToolResultStatusView::Error,
                task_id: Some(task_id.to_string()),
                summary: format!("task {task_id} state unknown"),
                ..Default::default()
            }),
        },
    }
}

/// The background env block rendered before an inference, or `None` when
/// there is nothing to show. Appended to the request only; never part of
/// the history.
pub fn render_background_env(tasks: &[TaskBrief]) -> Option<String> {
    if tasks.is_empty() {
        return None;
    }
    let mut s = String::from(
        "<background_tasks>\nTasks started earlier in this context (status at this moment; use get_task_state / wait_task for full output):\n",
    );
    for t in tasks {
        s.push_str(&format!(
            "- {} [{}] {} ({})",
            t.task_id,
            t.status,
            t.brief,
            fmt_elapsed(t.elapsed_ms)
        ));
        if !t.last_line.trim().is_empty() {
            s.push_str(&format!(" | last: {}", t.last_line.trim()));
        }
        s.push('\n');
    }
    s.push_str("</background_tasks>");
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_state_renders_hint_and_cancel_only_when_cancellable() {
        let obs = task_state_observation(
            "c1",
            "local:1",
            &TaskState::Running {
                brief: "cargo build".into(),
                elapsed_ms: 65_000,
                output_tail: "Compiling x\n".into(),
                cancellable: false,
            },
        );
        let Observation::Success { content, .. } = obs else {
            panic!("{obs:?}")
        };
        let text = content.as_str().unwrap();
        assert!(text.contains("still running (1m05s)"));
        assert!(text.contains("wait_task"));
        assert!(!text.contains("cancel_task"));
        let obs = task_state_observation(
            "c1",
            "local:1",
            &TaskState::Running {
                brief: "x".into(),
                elapsed_ms: 0,
                output_tail: String::new(),
                cancellable: true,
            },
        );
        let Observation::Success { content, .. } = obs else {
            panic!("{obs:?}")
        };
        assert!(content.as_str().unwrap().contains("cancel_task"));
    }

    #[test]
    fn finished_and_unknown_render() {
        let ok = task_state_observation(
            "c",
            "t",
            &TaskState::Finished(TaskResult {
                success: true,
                output: "done".into(),
                tool_result: None,
            }),
        );
        assert!(matches!(ok, Observation::Success { .. }));
        let bad = task_state_observation(
            "c",
            "t",
            &TaskState::Finished(TaskResult {
                success: false,
                output: "exit=2".into(),
                tool_result: None,
            }),
        );
        assert!(matches!(bad, Observation::Error { ref message, .. } if message == "exit=2"));
        let unknown = task_state_observation(
            "c",
            "t",
            &TaskState::Unknown {
                reason: "restarted".into(),
            },
        );
        assert!(
            matches!(unknown, Observation::Error { ref message, .. } if message.contains("state unknown"))
        );
    }

    #[test]
    fn background_env_is_absent_without_tasks() {
        assert!(render_background_env(&[]).is_none());
        let env = render_background_env(&[TaskBrief {
            task_id: "local:1".into(),
            brief: "sleep 100".into(),
            status: "running".into(),
            elapsed_ms: 3000,
            last_line: "tick".into(),
            cancellable: true,
        }])
        .unwrap();
        assert!(env.contains("local:1 [running] sleep 100 (3s) | last: tick"));
    }
}
