//! Task bridge: the state of a task (TaskMgr or any other task manager,
//! seen through `RunningTaskResolver`) as a stable `AgentEvent`.
//!
//! The mapping lives here, in the adapter: a session never reads a
//! service's own status fields or event paths to infer what a tool call
//! depends on. A notification only speeds up the check — the task / state
//! API stays the authority, and a lost notification is recovered by asking
//! again.

use llm_context::tasks::TaskState;

use crate::protocol::*;

/// Dedup key of the event for revision `rev` of a task (`None`: its
/// terminal state).
pub fn task_event_key(task_id: &str, rev: Option<u64>) -> String {
    match rev {
        Some(r) => format!("task:{task_id}:revision:{r}"),
        None => format!("task:{task_id}:terminal"),
    }
}

fn one_line(text: &str, max: usize) -> String {
    let one = text.split_whitespace().collect::<Vec<_>>().join(" ");
    one.chars().take(max).collect()
}

/// The event describing `state` of `task_id`. `source = task:<task_id>`:
/// the same id the suspended call record and the task tools use, so a
/// waiting call and the event match by equality.
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
    }
}

/// Stable identity of one task dispatch: the idempotency key a tool hands
/// to an external task service, derived from the session / run / call. It
/// is persisted with the in-flight record before the tool runs, so a crash
/// between creating the task and recording its binding finds the same task
/// again instead of creating a second one. In-process tasks do not need it:
/// their `task_id` is in the call result.
pub fn dispatch_idempotency_key(run_id: &str, call_id: &str) -> String {
    crate::ids::h(&[run_id, call_id])
}
