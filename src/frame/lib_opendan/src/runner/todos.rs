//! The session's TodoList on the runner side (`doc/opendan/Agent TodoList.md`
//! §2): which Todo `next_behavior = <todo executor>` executes, waiting
//! instead of deciding, the serial attempt's result, the explorations'
//! state written back at batch boundaries, the PLAN's compact view and the
//! end check.

use llm_context::state::LLMContextSnapshot;
use llm_context::DecisionVerdict;
use serde_json::{json, Value};

use crate::error::Result;
use crate::protocol::*;
use crate::session::Session;
use crate::todos::{
    one_line, save_submission, store_of, todo_error, CallRef, ChildObservation, SerialPick,
    TodoList, TodoStatus, TodoStore,
};

use super::outcome::{refuse_handover, CallSite, FinishKind, Next};
use super::shared::Shared;

/// Answer of a sub context that ran out of budget (`outcome.rs`): a serial
/// attempt ending with it timed out.
pub(super) const SUB_CONTEXT_BUDGET: &str = "the sub context ran out of budget";

pub(super) fn store(sh: &Shared) -> TodoStore {
    store_of(&sh.dir)
}

fn load(sh: &Shared) -> TodoList {
    store(sh).load().unwrap_or_else(|e| {
        log::warn!("session {}: todo list unreadable: {e}", sh.dir.sid());
        TodoList::default()
    })
}

/// The step of a behavior run whose decision is being carried out.
pub(super) fn decision_step(snapshot: &LLMContextSnapshot) -> Option<u32> {
    let st = &snapshot.state;
    st.last_step
        .as_ref()
        .or(st.steps.last())
        .map(|s| s.meta.step_index)
}

fn caller_of(run_id: &str, step: Option<u32>) -> CallRef {
    CallRef {
        run_id: run_id.to_string(),
        call_id: format!("step:{}", step.unwrap_or(0)),
    }
}

async fn is_todo_executor(sh: &Shared, behavior: &str) -> bool {
    let cfg = sh.session.lock().await.config.clone();
    sh.deps
        .assembler
        .behavior_entry(&cfg, behavior)
        .is_ok_and(|e| e.todo)
}

/// What the session is waiting for besides a decision: sub sessions it
/// must hear from.
async fn waiting_children(sh: &Shared) -> Vec<String> {
    super::children::unsettled_children(sh)
        .await
        .unwrap_or_default()
}

fn with_view(message: String, list: &TodoList) -> String {
    if list.todos.is_empty() {
        message
    } else {
        format!("{message}\n{}", list.render_view())
    }
}

fn not_ready(target: &str, pick: &SerialPick, list: &TodoList) -> String {
    let detail = match pick {
        SerialPick::Waiting(w) => format!(
            " Pending serial todos wait for results that are not accepted yet: {}.",
            w.iter()
                .map(|(id, deps)| format!("{id} after {}", deps.join(",")))
                .collect::<Vec<_>>()
                .join("; ")
        ),
        _ => String::new(),
    };
    with_view(
        format!(
            "Decision refused: `{target}` executes the next serial Todo, and none is ready.{detail} \
Add one with `agent-session todo add \"<title>\" --content <what, scope, evidence> [--accept <criterion>]...`, \
dispatch explorations with `agent-session todo explore …`, or finish with <report end=\"true\">."
        ),
        list,
    )
}

fn review_first(ids: &[String], list: &TodoList) -> String {
    with_view(
        format!(
            "Decision refused: results came back and are not reviewed yet: {}. \
Read each with `agent-session todo show <id> --report`, then record your judgement with \
`agent-session todo review <id> --accept|--reject|--need-evidence --reason <why>` before the next Todo runs.",
            ids.join(", ")
        ),
        list,
    )
}

/// The checkpoint hook's verdict on a hand-over to a Todo executor (top
/// level only): a ready serial Todo or explorations to wait for accept it;
/// unreviewed results or nothing to execute refuse it, correctably.
pub(super) async fn review_handover(sh: &Shared, run_id: &str, target: &str) -> Option<DecisionVerdict> {
    if super::outcome::call_site_of(sh, run_id).await.0 || !is_todo_executor(sh, target).await {
        return None;
    }
    let list = match store(sh).load() {
        Ok(l) => l,
        Err(e) => return Some(DecisionVerdict::Refuse(format!("the todo list cannot be read: {e}"))),
    };
    match list.serial_pick() {
        SerialPick::Ready(_) => Some(DecisionVerdict::Accept),
        SerialPick::Unreviewed(ids) => Some(DecisionVerdict::Refuse(review_first(&ids, &list))),
        pick => {
            if waiting_children(sh).await.is_empty() {
                Some(DecisionVerdict::Refuse(not_ready(target, &pick, &list)))
            } else {
                // Nothing to execute while explorations run: waiting.
                Some(DecisionVerdict::Accept)
            }
        }
    }
}

/// Report-only steps in a row at the end of the run (corrections between
/// them do not break the row).
fn trailing_yields(snapshot: &LLMContextSnapshot) -> usize {
    let st = &snapshot.state;
    st.steps
        .iter()
        .chain(st.last_step.iter())
        .rev()
        .filter(|s| !s.is_correction())
        .take_while(|s| {
            llm_context::LLMBehaviorResult::from_step(s).decision()
                == Some(llm_context::StepDecision::Yield)
        })
        .count()
}

/// Report-only steps accepted in a row while nothing runs, before the next
/// one is refused (a context that only reports would re-infer forever).
const MAX_IDLE_YIELDS: usize = 2;

/// The checkpoint hook's verdict on a yield (a progress report and nothing
/// else): it ends the run while the top-level context waits for sub
/// sessions the session must hear from ("no decision" is waiting). With
/// nothing to wait for no verdict is given — the report stays a progress
/// report and the context continues, as it always did — until it happened
/// [`MAX_IDLE_YIELDS`] times in a row: then it is refused with the reason.
pub(super) async fn review_yield(
    sh: &Shared,
    run_id: &str,
    snapshot: &LLMContextSnapshot,
) -> Option<DecisionVerdict> {
    let child = super::outcome::call_site_of(sh, run_id).await.0;
    if !child && !waiting_children(sh).await.is_empty() {
        return Some(DecisionVerdict::Accept);
    }
    if trailing_yields(snapshot) < MAX_IDLE_YIELDS {
        return None;
    }
    let list = load(sh);
    Some(DecisionVerdict::Refuse(with_view(
        "Decision refused: this is another reply with only a progress report, and nothing this session started is running, so there is nothing to wait for. \
Act with actions, hand over with next_behavior, or finish with <report end=\"true\">."
            .into(),
        &list,
    )))
}

fn wait_for(next: &mut Next, children: Vec<String>) {
    next.kind = FinishKind::Wait;
    next.run_ended = true;
    next.suspended = false;
    next.finished = false;
    next.outcome = None;
    next.waiting = true;
    next.turn_end = None;
    next.next_behavior = None;
    next.call = None;
    next.error = None;
    next.wait_children = children;
}

/// Session-side meaning of a decision after `classify_done` (also when a
/// decision is redone after a crash): a yield waits for the sub sessions
/// the session must hear from; a hand-over to a Todo executor executes the
/// ready serial Todo (bound in [`bind_serial`]) or, with none ready while
/// explorations run, waits too. What the checkpoint hook would have refused
/// and still arrives here (an executor without the hook) ends the Turn as
/// failed.
pub(super) async fn settle(
    sh: &Shared,
    next: &mut Next,
    site: &CallSite<'_>,
    yielded: bool,
    run_id: &str,
    snapshot: &LLMContextSnapshot,
) -> Result<()> {
    if site.child {
        return Ok(());
    }
    if yielded {
        let children = waiting_children(sh).await;
        if !children.is_empty() {
            wait_for(next, children);
        } else if next.finished && next.outcome == Some(Outcome::Succeeded) {
            next.kind = FinishKind::Error;
            next.outcome = Some(Outcome::Failed);
            next.turn_end = Some(TurnStatus::Failed);
            next.error = Some(json!({
                "kind": "missing_end_report",
                "message": "a progress report alone does not finish the session; nothing it started is running",
                "recoverable": false,
            }));
        }
        return Ok(());
    }
    let Some(call) = next.call.as_ref().filter(|c| c.trigger == CallTrigger::Behavior && c.todo.is_none()) else {
        return Ok(());
    };
    if next.kind != FinishKind::Switch || !is_todo_executor(sh, &call.behavior).await {
        return Ok(());
    }
    let target = call.behavior.clone();
    let list = load(sh);
    // A hand-over redone after a crash finds the attempt it bound.
    let caller = caller_of(run_id, decision_step(snapshot));
    if list
        .todos
        .iter()
        .any(|t| t.attempts.iter().any(|a| a.caller.as_ref() == Some(&caller)))
    {
        return Ok(());
    }
    match list.serial_pick() {
        SerialPick::Ready(_) => {}
        SerialPick::Unreviewed(ids) => refuse_handover(next, site, review_first(&ids, &list)),
        pick => {
            let children = waiting_children(sh).await;
            if children.is_empty() {
                refuse_handover(next, site, not_ready(&target, &pick, &list));
            } else {
                wait_for(next, children);
            }
        }
    }
    Ok(())
}

/// Bind the hand-over to a Todo executor to the ready serial Todo (before
/// the suspension commits): its attempt starts, its body becomes the sub
/// task. Idempotent for the same caller step, so a hand-over redone after
/// a crash gets the same attempt.
pub(super) async fn bind_serial(
    sh: &Shared,
    next: &mut Next,
    run_id: &str,
    snapshot: &LLMContextSnapshot,
) -> Result<()> {
    let Some(call) = next.call.as_ref() else {
        return Ok(());
    };
    if call.trigger != CallTrigger::Behavior || call.todo.is_some() || !is_todo_executor(sh, &call.behavior).await {
        return Ok(());
    }
    let caller = caller_of(run_id, decision_step(snapshot));
    let now = crate::now_ms();
    let (todo_id, attempt, task) = store(sh)
        .update(|l| {
            let existing = l.todos.iter().find_map(|t| {
                t.attempts
                    .iter()
                    .find(|a| a.caller.as_ref() == Some(&caller))
                    .map(|a| (t.todo_id.clone(), a.attempt))
            });
            let (id, n) = match existing {
                Some(found) => found,
                None => match l.serial_pick() {
                    SerialPick::Ready(id) => {
                        let n = l.begin_serial(&id, caller.clone(), now)?;
                        (id, n)
                    }
                    _ => {
                        return Err(agent_tool::AgentToolError::InvalidArgs(
                            "no serial todo is ready to bind".into(),
                        ))
                    }
                },
            };
            let task = l.get(&id).map(|t| t.task_text(l, n)).unwrap_or_default();
            Ok((id, n, task))
        })
        .map_err(todo_error)?;
    crate::fault::point("todo:after_serial_bind");
    if let Some(call) = next.call.as_mut() {
        call.task = Some(task);
        call.todo = Some(TodoBinding { todo_id, attempt });
    }
    Ok(())
}

/// The serial attempt a returning sub context executed, written back before
/// the return commits (idempotent). Returns the brief the caller gets in
/// place of the full report (it reads that with `todo show --report`).
pub(super) fn serial_returned(
    sh: &Shared,
    s: &Session,
    binding: &TodoBinding,
    run_id: &str,
    child_status: Option<&str>,
    report: Option<&ReportSubmission>,
    answer: Option<&str>,
) -> Result<String> {
    let status = match child_status.unwrap_or("ok") {
        "ok" => TodoStatus::Completed,
        _ if answer.is_some_and(|a| a.starts_with(SUB_CONTEXT_BUDGET)) => TodoStatus::Timeout,
        _ => TodoStatus::Failed,
    };
    if let Some(r) = report {
        save_submission(&sh.dir, &sh.lease, r)?;
    }
    let summary = report
        .map(|r| one_line(&r.report))
        .or_else(|| answer.map(one_line))
        .filter(|t| !t.is_empty());
    let error = match (status, child_status) {
        (TodoStatus::Completed, _) => None,
        (_, Some("needs_user_input")) => Some(format!(
            "the executor needs user input: {}",
            answer.unwrap_or_default()
        )),
        _ => answer.map(str::to_string),
    };
    let seen = ChildObservation {
        status,
        summary: summary.clone(),
        report: report.map(|r| crate::todos::ReportRef {
            session_id: s.sid().to_string(),
            run_id: Some(run_id.to_string()),
            report_id: r.id.clone(),
        }),
        artifacts: report.map(crate::todos::artifact_refs).unwrap_or_default(),
        error,
    };
    store(sh)
        .update(|l| l.end_serial(&binding.todo_id, binding.attempt, run_id, seen, crate::now_ms()))
        .map_err(todo_error)?;
    crate::fault::point("todo:after_serial_writeback");
    Ok(format!(
        "Todo {} attempt {} {}: {}\nRead the full report with `agent-session todo show {} --report`, then review it.",
        binding.todo_id,
        binding.attempt,
        status.as_str(),
        summary.unwrap_or_else(|| "(no report)".into()),
        binding.todo_id
    ))
}

/// Write the explorations' committed state back into the list at a batch
/// boundary (and at the drive's start, after recovery). See
/// [`crate::todos::sync`].
pub(super) async fn sync(sh: &Shared) -> Result<bool> {
    let state = sh.session.lock().await.state.clone();
    crate::todos::sync(sh.agent(), &sh.dir, &state).await
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

/// The serial Todo a hand-over would execute now, for templates
/// (`session.current_todo`).
pub(super) fn current(sh: &Shared) -> Value {
    let list = load(sh);
    match list.serial_pick() {
        SerialPick::Ready(id) => list
            .get(&id)
            .and_then(|t| serde_json::to_value(t).ok())
            .unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

/// Why the session may not finish yet (TodoList §2.4).
pub(super) fn unconverged(sh: &Shared) -> Vec<String> {
    load(sh).unconverged()
}

/// `todo:<id>:<attempt>` sub sessions the list binds: child sid → (todo, attempt).
pub(super) fn bound_children(sh: &Shared) -> Vec<(String, String, u32, Option<String>)> {
    load(sh)
        .todos
        .iter()
        .flat_map(|t| {
            t.attempts.iter().filter_map(|a| {
                a.child_sid
                    .clone()
                    .map(|c| (c, t.todo_id.clone(), a.attempt, a.summary.clone()))
            })
        })
        .collect()
}
