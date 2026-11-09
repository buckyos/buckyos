//! Running tasks: work that outlives one tool call and is addressed by a
//! `task_id` (runtime spec, `notepads/done/llm-context-freeze-todo.md` §3.1).
//!
//! The waist knows nothing about tasks: every tool call returns a terminal
//! observation. A tool whose work is not done within the runtime's wait
//! budget returns "still running, task X" and the task is followed through
//! [`RunningTaskResolver`] — by the model (`wait_task` / `get_task_state`)
//! or by the session (task done becomes a new input).
//!
//! One render function turns a [`TaskState`] into the observation the LLM
//! sees, so the inline `wait_task` / `get_task_state` tools, the task CLI
//! bridge and a session's task result read the same.

use async_trait::async_trait;
use llm_context::deps::ToolCallCtx;
use llm_context::observation::Observation;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::result_view::{ToolResultStatusView, ToolResultView};

/// Longest a tool call may wait for a task inside the call: whatever is
/// waited for, the call returns to the LLM with the task's state by then.
pub const MAX_IN_TOOL_WAIT_MS: u64 = 30 * 60_000;

/// Default in-call wait of a tool call for its task (`RuntimeConfig.task_wait_ms`,
/// `wait_task`): a call that ends within it is synchronous, anything longer
/// returns "still running, task X".
pub const DEFAULT_TASK_WAIT_MS: u64 = 25_000;

/// Terminal result of a task, in the shape of a tool result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskResult {
    pub success: bool,
    /// Text the LLM sees (output or summary).
    pub output: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_result: Option<ToolResultView>,
}

impl TaskResult {
    /// The end of a task whose work produced `result`: the text is what
    /// the same result reads like when the call ends within its wait.
    pub fn from_tool_result(result: &crate::AgentToolResult) -> Self {
        Self {
            success: result.status != crate::AgentToolStatus::Error,
            output: result.observation_text(),
            tool_result: ToolResultView::from_host(&Some(result.to_host())),
        }
    }
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

/// One task a context follows.
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

    /// Tasks this context follows. A finished task stays listed until the
    /// LLM read its result once.
    async fn active(&self) -> Vec<TaskBrief>;

    /// How the model follows these tasks (the wording of hints).
    fn followup(&self) -> TaskFollowup {
        TaskFollowup::Tools
    }

    /// Whether the model was already given this task's end through this
    /// resolver (a task tool, a fill, or the task CLI bridge). A host that
    /// keeps following tasks after a run skips those.
    fn result_seen(&self, task_id: &str) -> bool {
        let _ = task_id;
        false
    }
}

/// How the model follows a task: with the task tools (`wait_task` /
/// `get_task_state` / `cancel_task`), or with a command run through
/// `shell` that reaches the same resolver (`<cli> wait|state|cancel <id>`),
/// or not at all: the host follows it outside the loop and brings its end
/// back as input (an Agent Session; `Some(cli)` checks it with the command,
/// `None` with the task tools).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum TaskFollowup {
    #[default]
    Tools,
    Command(String),
    Input(Option<String>),
}

impl TaskFollowup {
    /// Hint appended to a still-running task's result.
    pub fn hint(&self, task_id: &str, cancellable: bool) -> String {
        match self {
            TaskFollowup::Tools => {
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
            TaskFollowup::Command(cli) => {
                let mut s = format!(
                    "Run `{cli} wait {task_id} --wait-ms <ms>` to keep waiting (give the shell call a larger wait_ms than that), or `{cli} state {task_id}` to check later."
                );
                if cancellable {
                    s.push_str(&format!(" `{cli} cancel {task_id}` stops it."));
                }
                s
            }
            TaskFollowup::Input(cli) => {
                let mut s = String::from(
                    "Its end comes back to you as input: do not check on it again. Reply with where things stand (that ends your turn), or go on with other work meanwhile.",
                );
                match cli {
                    Some(cli) => {
                        if cancellable {
                            s.push_str(&format!(" `{cli} cancel {task_id}` stops it."));
                        }
                    }
                    None => {
                        if cancellable {
                            s.push_str(&format!(
                                " `cancel_task` with task_id=\"{task_id}\" stops it."
                            ));
                        }
                    }
                }
                s
            }
        }
    }

    /// How tasks are followed, for any task (`<task_id>` stands for its
    /// id): the hint a result shows once per session.
    pub fn general_hint(&self) -> String {
        match self {
            TaskFollowup::Tools => format!(
                "A task is followed with `wait_task` (task_id, wait_ms up to {} min) to keep waiting, or `get_task_state` to check it later; `cancel_task` stops a cancellable task.",
                MAX_IN_TOOL_WAIT_MS / 60_000
            ),
            TaskFollowup::Command(cli) => format!(
                "A task is followed with `{cli} wait <task_id> --wait-ms <ms>` to keep waiting (give the shell call a larger wait_ms than that), or `{cli} state <task_id>` to check it later; `{cli} cancel <task_id>` stops a cancellable task."
            ),
            TaskFollowup::Input(cli) => {
                let mut s = String::from(
                    "A task's end comes back to you as input with its full result, and its new output is shown to you before you answer: do not wait for it or check on it in a loop. When nothing else is to be done now, reply with where things stand — that ends your turn; the task's end starts the next one.",
                );
                match cli {
                    Some(cli) => s.push_str(&format!(
                        " `{cli} state <task_id>` shows a task's output once when you need it; `{cli} cancel <task_id>` stops a cancellable one."
                    )),
                    None => s.push_str(" `cancel_task` stops a cancellable one."),
                }
                s
            }
        }
    }
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
/// `call_id`. Used for the task tools and for a pending call waited for
/// within its budget alike.
pub fn task_state_observation(call_id: &str, task_id: &str, state: &TaskState) -> Observation {
    task_state_observation_via(call_id, task_id, state, &TaskFollowup::Tools)
}

/// [`task_state_observation`] with the hints worded for `followup`.
pub fn task_state_observation_via(
    call_id: &str,
    task_id: &str,
    state: &TaskState,
    followup: &TaskFollowup,
) -> Observation {
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
            text.push_str(&followup.hint(task_id, *cancellable));
            let bytes = text.len();
            Observation::Success {
                call_id: call_id.to_string(),
                content: Value::String(text),
                bytes,
                truncated: false,
                images: Vec::new(),
                host: Some(
                    ToolResultView {
                        status: ToolResultStatusView::Success,
                        task_id: Some(task_id.to_string()),
                        summary: format!("task {task_id} still running"),
                        partial_output: Some(output_tail.clone()).filter(|s| !s.is_empty()),
                        ..Default::default()
                    }
                    .into_host(),
                ),
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
                    host: Some(tool_result.into_host()),
                    images: Vec::new(),
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
                    host: Some(tool_result.into_host()),
                }
            }
        }
        TaskState::Unknown { reason } => Observation::Error {
            call_id: call_id.to_string(),
            message: format!(
                "task {task_id}: state unknown ({reason}). Its result was not recorded; check the actual state (files, processes, logs) before repeating the work."
            ),
            host: Some(
                ToolResultView {
                    status: ToolResultStatusView::Error,
                    task_id: Some(task_id.to_string()),
                    summary: format!("task {task_id} state unknown"),
                    ..Default::default()
                }
                .into_host(),
            ),
        },
    }
}

/// Epoch ms by which a wait inside a tool call must end: the run deadline
/// capped by [`MAX_IN_TOOL_WAIT_MS`] from now, further capped by `wait_ms`
/// when given.
pub fn wait_until_ms(ctx: &ToolCallCtx, wait_ms: Option<u64>) -> u64 {
    let now = llm_context::now_ms();
    let mut until = now.saturating_add(wait_ms.unwrap_or(MAX_IN_TOOL_WAIT_MS));
    until = until.min(now.saturating_add(MAX_IN_TOOL_WAIT_MS));
    if let Some(d) = ctx.deadline_ms {
        until = until.min(d);
    }
    until
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

}
