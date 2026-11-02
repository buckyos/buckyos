//! The session's TodoList on the runner side (`doc/opendan/Agent TodoList.md`
//! §2): the work sessions' committed state written back and the ready Todos
//! dispatched at every batch boundary (the scheduling point of the host:
//! the plan never asks for a dispatch), the compact view and the end of a
//! plan.

use crate::error::Result;
use crate::protocol::*;
use crate::todos::{store_of, DispatchOptions, Scheduled, TodoList, TodoStore};

use super::shared::Shared;

pub(super) fn store(sh: &Shared) -> TodoStore {
    store_of(&sh.dir)
}

pub(super) fn load(sh: &Shared) -> TodoList {
    store(sh).load().unwrap_or_else(|e| {
        log::warn!("session {}: todo list unreadable: {e}", sh.dir.sid());
        TodoList::default()
    })
}

/// Bring the list up to its work sessions' committed state (see
/// [`crate::todos::sync`]), then dispatch what became ready: a prerequisite
/// that completed starts the Todos after it before the plan infers again,
/// a work session that ended makes room for one held back. `true`: the
/// list changed.
pub(super) async fn sync(sh: &Shared) -> Result<bool> {
    if load(sh).todos.is_empty() {
        return Ok(false);
    }
    let changed = crate::todos::sync(sh.agent(), &sh.dir).await?;
    let scheduled = schedule(sh).await?;
    Ok(changed || scheduled.iter().any(|s| !matches!(s, Scheduled::Held { .. })))
}

/// One scheduling point of the host (`crate::todos::schedule`).
pub(super) async fn schedule(sh: &Shared) -> Result<Vec<Scheduled>> {
    let out = crate::todos::schedule(
        sh.agent(),
        &sh.deps.who,
        sh.deps.inputs.as_ref(),
        &sh.dir,
        &DispatchOptions::from_env(),
    )
    .await?;
    for s in &out {
        match s {
            Scheduled::Dispatched(d) => log::info!(
                "session {}: todo {} attempt {} dispatched to {}",
                sh.dir.sid(),
                d.todo_id,
                d.attempt,
                d.child_sid
            ),
            Scheduled::Blocked { todo_id, reason } => log::info!(
                "session {}: todo {todo_id} blocked: {reason}",
                sh.dir.sid()
            ),
            Scheduled::Held { .. } => {}
        }
    }
    Ok(out)
}

/// The plan ended: what still runs of its list ends as stopped (the work
/// sessions are stopped with the plan).
pub(super) fn stop_open(sh: &Shared, outcome: Option<Outcome>) {
    let reason = match outcome {
        Some(Outcome::Stopped) => "the plan session was stopped",
        _ => "the plan session ended",
    };
    match crate::todos::stop_open(&sh.dir, reason) {
        Ok(ids) if !ids.is_empty() => log::info!(
            "session {}: todos {} stopped: {reason}",
            sh.dir.sid(),
            ids.join(", ")
        ),
        Ok(_) => {}
        Err(e) => log::warn!("session {}: todo list not settled at its end: {e}", sh.dir.sid()),
    }
}

/// The compact view for an input batch of the top-level context, when the
/// list changed since it was last shown: `(view, rev)`.
pub(super) fn view_for_batch(sh: &Shared, state: &SessionState) -> Option<(String, u64)> {
    if state.child_call().is_some() {
        return None;
    }
    let list = load(sh);
    if list.todos.is_empty() || list.rev == state.todo_view_rev {
        return None;
    }
    Some((list.render_view(), list.rev))
}

/// The Todo a work session executes, as the list binds it.
pub(super) struct BoundChild {
    pub child_sid: String,
    pub todo_id: String,
    pub attempt: u32,
    pub summary: Option<String>,
    /// The Todos that run after it and what became of them (`T02 running`,
    /// `T03 blocked`).
    pub dependents: Vec<String>,
}

/// `todo:<id>:<attempt>` work sessions the list binds.
pub(super) fn bound_children(sh: &Shared) -> Vec<BoundChild> {
    let list = load(sh);
    list.todos
        .iter()
        .flat_map(|t| {
            let dependents: Vec<String> = list
                .todos
                .iter()
                .filter(|d| d.after.contains(&t.todo_id))
                .map(|d| match d.latest().filter(|a| !a.ended()).and_then(|a| a.child_sid.clone()) {
                    Some(c) => format!("{} {} ({c})", d.todo_id, d.status.as_str()),
                    None => format!("{} {}", d.todo_id, d.status.as_str()),
                })
                .collect();
            t.attempts.iter().filter_map(move |a| {
                a.child_sid.clone().map(|c| BoundChild {
                    child_sid: c,
                    todo_id: t.todo_id.clone(),
                    attempt: a.attempt,
                    summary: a.summary.clone(),
                    dependents: dependents.clone(),
                })
            })
        })
        .collect()
}
