//! Task bridge: the state of a task (TaskMgr or any other task manager,
//! seen through `RunningTaskResolver`) as a stable `AgentEvent`.
//!
//! The mapping lives here, in the adapter: a session never reads a
//! service's own status fields or event paths to infer what a tool call
//! depends on. A notification only speeds up the check — the task / state
//! API stays the authority, and a lost notification is recovered by asking
//! again.

use agent_tool::task_model::TaskState;

use crate::protocol::*;

fn one_line(text: &str, max: usize) -> String {
    let one = text.split_whitespace().collect::<Vec<_>>().join(" ");
    one.chars().take(max).collect()
}

/// The event describing `state` of `task_id`. `source = task:<task_id>`:
/// the same id the task tools and the session's followed tasks use, so they
/// match by equality.
pub fn task_event(
    task_id: &str,
    state: &TaskState,
    rev: Option<u64>,
    subscription_id: Option<String>,
) -> AgentEvent {
    let (event, summary, terminal) = match state {
        TaskState::Running { brief, .. } => (
            "updated",
            format!("task {task_id} is running: {}", one_line(brief, 240)),
            false,
        ),
        TaskState::Finished(r) => (
            "finished",
            format!(
                "task {task_id} {}: {}",
                if r.success { "succeeded" } else { "failed" },
                one_line(&r.output, 240)
            ),
            true,
        ),
        TaskState::Unknown { reason } => (
            "unknown",
            format!("task {task_id}: state unknown ({})", one_line(reason, 240)),
            true,
        ),
    };
    AgentEvent {
        subscription_id,
        source: EventSource::task(task_id),
        event: event.to_string(),
        seq: rev,
        summary,
        data_ref: None,
        terminal,
        call_id: None,
    }
}

