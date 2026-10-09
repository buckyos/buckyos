//! Turns as tasks of the host's task service: every Turn a session opens
//! gets one task (created when the Turn opens, whatever its length), the
//! session reports its state to it and closes it with the Turn. The runner
//! is the only writer; nothing is read back from the task service here.
//!
//! libopendan does not know the task service: a host provides a
//! [`TurnTaskSink`]. Without one no task is bound and nothing changes.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::protocol::*;
use crate::session::Session;

use super::shared::Shared;

/// The Turn a task is created for.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnTaskOpen {
    pub session_id: String,
    pub agent_did: String,
    pub kind: SessionKind,
    pub turn: u64,
    /// Task of the Turn that created this session (`None`: a root task).
    pub parent_task: Option<String>,
    /// What the Turn is about, for a list of tasks.
    pub title: String,
    /// The Turn's logical input (`<source>#<index>`) of its opening batch.
    pub inputs: Vec<String>,
}

/// What the session does right now.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnTaskStatus {
    pub run_state: RunState,
    pub waiting_for: Option<WaitingFor>,
    /// One line: the current activity.
    pub message: String,
    pub behavior: Option<String>,
    pub tool: Option<String>,
}

/// How the Turn ended.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnTaskEnd {
    pub status: TurnStatus,
    pub summary: Option<String>,
    pub error: Option<Value>,
}

/// The host's task service, as far as Turns need it.
#[async_trait]
pub trait TurnTaskSink: Send + Sync {
    /// Create the task of a Turn and return its id. Idempotent for the same
    /// `(session_id, turn)`: asked again after a crash it returns the task
    /// created before.
    async fn open(&self, turn: &TurnTaskOpen) -> std::result::Result<String, String>;
    async fn update(&self, task_id: &str, status: &TurnTaskStatus)
        -> std::result::Result<(), String>;
    /// Report the terminal state. Asked again until it succeeds.
    async fn close(&self, task_id: &str, end: &TurnTaskEnd) -> std::result::Result<(), String>;
}

fn cut(text: &str, max: usize) -> String {
    let one = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= max {
        return one;
    }
    let mut out: String = one.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Bind the open Turn to a task when it has none. A task that cannot be
/// created leaves the Turn without one (tried again by the next drive).
pub(super) async fn ensure_turn_task(sh: &Arc<Shared>, title: Option<String>) {
    let Some(sink) = sh.deps.turn_tasks.clone() else {
        return;
    };
    let open = {
        let s = sh.session.lock().await;
        let Some(t) = s.state.open_turn.as_ref() else {
            return;
        };
        if s.state.turn_tasks.iter().any(|b| b.turn == t.index) {
            return;
        }
        let title = title
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| s.config.session.objective.clone());
        (
            TurnTaskOpen {
                session_id: s.sid().to_string(),
                agent_did: s.config.session.agent_did.clone(),
                kind: s.config.session.kind,
                turn: t.index,
                parent_task: s
                    .config
                    .session
                    .origin
                    .as_ref()
                    .and_then(|o| o.parent_task.clone()),
                title: cut(&title, 120),
                inputs: t.inputs.clone(),
            },
            t.has_msg,
            t.at_ms,
        )
    };
    let (open, has_msg, at_ms) = open;
    let task_id = match sink.open(&open).await {
        Ok(id) => id,
        Err(e) => {
            log::warn!(
                "session {}: no task for turn {}: {e}",
                open.session_id,
                open.turn
            );
            return;
        }
    };
    crate::fault::point("turn_task:after_open");
    {
        let mut s = sh.session.lock().await;
        if s.state.turn_tasks.iter().any(|b| b.turn == open.turn) {
            return;
        }
        let still_open = s.state.open_turn.as_ref().map(|t| t.index) == Some(open.turn);
        s.state.turn_tasks.push(TurnTask {
            turn: open.turn,
            task_id,
            has_msg,
            opened_at_ms: at_ms,
            placeholder: None,
            closed: (!still_open).then_some(TurnStatus::Completed),
            summary: None,
            error: None,
            reported: false,
        });
        if let Err(e) = s.commit_state(&sh.lease) {
            log::warn!("session {}: turn task not committed: {e}", open.session_id);
            return;
        }
    }
    if has_msg {
        arm_placeholder(sh);
    }
    sync_turn_task(sh).await;
}

/// The placeholder of a Turn still open after `placeholder_delay`. The
/// timer only holds a weak reference: a drive that ended is not kept alive
/// (the next drive checks the delay again).
fn arm_placeholder(sh: &Arc<Shared>) {
    let delay = sh.deps.options.placeholder_delay;
    let weak = Arc::downgrade(sh);
    tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        if let Some(sh) = weak.upgrade() {
            super::outbound::maybe_placeholder(&sh, false).await;
        }
    });
}

fn status_of(s: &Session, tool: Option<String>) -> Option<(String, TurnTaskStatus)> {
    let binding = s.state.open_turn_task()?;
    let st = &s.state;
    let message = match (&st.waiting_for, st.run_state) {
        (Some(w), RunState::Waiting) => match w.kind {
            WaitingKind::Children => format!("waiting for {} sub session(s)", w.refs.len().max(1)),
            WaitingKind::Tool => "waiting for a tool".to_string(),
            WaitingKind::Event => "waiting for an event".to_string(),
            WaitingKind::Input => "waiting for input".to_string(),
        },
        _ => match (&tool, &st.current_behavior) {
            (Some(t), _) => format!("running {t}"),
            (None, Some(b)) => format!("working ({b})"),
            (None, None) => "working".to_string(),
        },
    };
    Some((
        binding.task_id.clone(),
        TurnTaskStatus {
            run_state: st.run_state,
            waiting_for: st.waiting_for.clone(),
            message,
            behavior: st.current_behavior.clone(),
            tool,
        },
    ))
}

/// Report the state of the open Turn to its task when it changed since the
/// last report of this drive (best effort: the next change reports again).
pub(super) async fn sync_turn_task(sh: &Shared) {
    let Some(sink) = sh.deps.turn_tasks.clone() else {
        return;
    };
    let tool = sh.current_tool.lock().expect("current tool").clone();
    let Some((task_id, status)) = status_of(&*sh.session.lock().await, tool) else {
        return;
    };
    {
        let mut last = sh.task_status.lock().expect("task status");
        if last.as_ref() == Some(&(task_id.clone(), status.clone())) {
            return;
        }
        *last = Some((task_id.clone(), status.clone()));
    }
    if let Err(e) = sink.update(&task_id, &status).await {
        log::warn!("session {}: task {task_id} not updated: {e}", sh.dir.sid());
        *sh.task_status.lock().expect("task status") = None;
    }
}

/// Inside the commit that closes a Turn: its binding takes the result.
pub(super) fn close_turn_task(
    s: &mut Session,
    turn: u64,
    status: TurnStatus,
    answer: Option<&str>,
    error: Option<&Value>,
) {
    if let Some(b) = s
        .state
        .turn_tasks
        .iter_mut()
        .find(|b| b.turn == turn && b.closed.is_none())
    {
        b.closed = Some(status);
        b.summary = answer.map(|a| cut(a, 400)).filter(|a| !a.is_empty());
        b.error = error.cloned();
    }
}

fn trim(tasks: &mut Vec<TurnTask>) {
    let settled = tasks.iter().filter(|b| b.reported).count();
    let mut drop = settled.saturating_sub(TURN_TASKS_KEEP_SETTLED);
    tasks.retain(|b| {
        if drop > 0 && b.reported {
            drop -= 1;
            return false;
        }
        true
    });
}

/// Report closed Turns to their tasks. A task service that is not reached
/// leaves the binding unreported: a later drive tries again.
pub(super) async fn flush_turn_tasks(sh: &Arc<Shared>) {
    let Some(sink) = sh.deps.turn_tasks.clone() else {
        return;
    };
    let pending: Vec<TurnTask> = {
        let s = sh.session.lock().await;
        s.state
            .turn_tasks
            .iter()
            .filter(|b| b.closed.is_some() && !b.reported)
            .cloned()
            .collect()
    };
    if pending.is_empty() {
        return;
    }
    let mut done = Vec::new();
    for b in pending {
        let end = TurnTaskEnd {
            status: b.closed.expect("closed"),
            summary: b.summary.clone(),
            error: b.error.clone(),
        };
        match sink.close(&b.task_id, &end).await {
            Ok(()) => done.push(b.turn),
            Err(e) => log::warn!(
                "session {}: task {} of turn {} not closed, will retry: {e}",
                sh.dir.sid(),
                b.task_id,
                b.turn
            ),
        }
    }
    if done.is_empty() {
        return;
    }
    crate::fault::point("turn_task:after_close");
    let mut s = sh.session.lock().await;
    for b in s.state.turn_tasks.iter_mut() {
        if done.contains(&b.turn) {
            b.reported = true;
        }
    }
    trim(&mut s.state.turn_tasks);
    if let Err(e) = s.commit_state(&sh.lease) {
        log::warn!("session {}: turn task state not committed: {e}", sh.dir.sid());
    }
}

/// Whether the session still has Turn results to report to their tasks.
pub fn has_pending_turn_tasks(state: &SessionState) -> bool {
    state
        .turn_tasks
        .iter()
        .any(|b| b.closed.is_some() && !b.reported)
}
