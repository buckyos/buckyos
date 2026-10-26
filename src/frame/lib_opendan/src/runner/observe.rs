//! What a context is shown after its tool results (`[input] background =
//! "after_tools"`, plan_simple TODO §4.4): the action → observation →
//! next decision of the behavior loop, for a function call loop. Before the
//! next inference the host brings the TodoList up to its sub sessions'
//! committed state and appends, as one `<<background_context>>` message,
//! the lines that changed since they were last shown: each Todo (its
//! compact view line) and the sub sessions no Todo binds. The session's
//! runtime and workspace are frozen for its life and told by its first
//! input. Never a sub session's transcript or report: those are read on
//! demand (`todo show --report`).
//!
//! What was shown is `HostMeta.observed` (line key → digest), committed with
//! the snapshot that carries the message: a crash before that commit shows
//! the lines again, one after it never twice. A batch whose results are not
//! all paired is never at this boundary (the waist only calls the hook
//! between batches), and the message after a final report is never needed
//! (the run finishes without another inference).

use std::collections::BTreeMap;

use buckyos_api::{AiMessage, AiRole};
use llm_context::deps::Injection;
use llm_context::state::LLMContextSnapshot;
use serde_json::Value;

use crate::error::Result;
use crate::protocol::*;
use crate::session::runs::RunHandle;

use super::background::{child_line, digest, render, BackgroundLine};
use super::receipts::{snapshot_host_meta, with_host_meta};
use super::shared::{commit_and_report, Shared};

/// run.json `host.extra` key: the run is shown background after its tool
/// results (`"after_tools"`), frozen with the run.
pub(super) const EXTRA_BACKGROUND: &str = "background";

const BLOCK_START: &str = "<<background_context>>";

pub(super) fn enabled(run: &RunHandle) -> bool {
    super::tools::run_flag(run, EXTRA_BACKGROUND)
        .as_ref()
        .and_then(Value::as_str)
        == Some("after_tools")
}

/// Whether the snapshot stands right after a batch of tool results (a
/// behavior step with action results).
fn after_tool_results(snapshot: &LLMContextSnapshot, behavior: bool) -> bool {
    let st = &snapshot.state;
    if behavior {
        return st
            .last_step
            .as_ref()
            .is_some_and(|s| !s.action_results.is_empty());
    }
    st.accumulated.last().is_some_and(|m| m.role == AiRole::Tool)
}

/// The candidate lines now, and the finished sub sessions with the key of
/// the line that tells their end.
struct Observed {
    lines: Vec<BackgroundLine>,
    ends: Vec<(String, String)>,
}

fn todo_key(todo_id: &str) -> String {
    format!("todo:{todo_id}")
}

async fn observe(sh: &Shared) -> Result<Observed> {
    let sid = sh.dir.sid().to_string();
    let list = super::todos::load(sh);
    let mut lines = Vec::new();
    for t in &list.todos {
        lines.push(BackgroundLine::new(todo_key(&t.todo_id), t.view_line()));
    }
    let bound: BTreeMap<String, (String, u32)> = list
        .todos
        .iter()
        .flat_map(|t| {
            t.attempts.iter().filter_map(|a| {
                a.child_sid
                    .clone()
                    .map(|c| (c, (t.todo_id.clone(), a.attempt)))
            })
        })
        .collect();
    let mut ends = Vec::new();
    for e in sh.agent().sessions().children_of(&[sid]).await? {
        let st = &e.status;
        let finished = st.run_state == RunState::Finished;
        match bound.get(&e.session_id) {
            // Its Todo's line tells it, once the attempt ended there.
            Some((todo, attempt)) => {
                if finished
                    && list
                        .get(todo)
                        .and_then(|t| t.attempt(*attempt))
                        .is_some_and(|a| a.ended())
                {
                    ends.push((e.session_id.clone(), todo_key(todo)));
                }
            }
            None => {
                let line = if finished
                    || st.pending_decision.is_some()
                    || st.waiting_for == Some(WaitingKind::Input)
                {
                    child_line(&e)
                } else {
                    // A running session's activity is not a change to
                    // decide on.
                    BackgroundLine::new(
                        format!("work:{}", e.session_id),
                        format!("work {}: {}", e.session_id, st.run_state.as_str()),
                    )
                };
                if finished {
                    ends.push((e.session_id.clone(), line.key.clone()));
                }
                lines.push(line);
            }
        }
    }
    Ok(Observed { lines, ends })
}

/// The lines to show and the `observed` map after showing them.
fn select(
    lines: &[BackgroundLine],
    observed: &BTreeMap<String, String>,
) -> (Vec<BackgroundLine>, BTreeMap<String, String>) {
    let mut shown = Vec::new();
    let mut next = observed.clone();
    for l in lines {
        let d = digest(&l.text);
        if observed.get(&l.key) == Some(&d) {
            continue;
        }
        next.insert(l.key.clone(), d);
        shown.push(l.clone());
    }
    (shown, next)
}

/// A finished sub session whose end the context was shown (the line in the
/// published snapshot is the current one) is heard from: its end is not
/// delivered again, and the session may finish without waiting for it.
async fn mark_delivered(
    sh: &Shared,
    now: &Observed,
    observed: &BTreeMap<String, String>,
) -> Result<()> {
    let current: BTreeMap<&str, String> = now
        .lines
        .iter()
        .map(|l| (l.key.as_str(), digest(&l.text)))
        .collect();
    let mut s = sh.session.lock().await;
    let mut changed = false;
    for (sid, key) in &now.ends {
        let shown = observed
            .get(key)
            .is_some_and(|d| current.get(key.as_str()) == Some(d));
        if shown && !super::children::end_delivered(&s.state, sid) {
            super::children::set_attention(&mut s.state, sid, Some(super::children::ATTENTION_FINISHED));
            changed = true;
        }
    }
    if changed {
        commit_and_report(sh, &mut s).await?;
    }
    Ok(())
}

/// The checkpoint hook's observation at an inference boundary (after the
/// snapshot was published): the TodoList is synchronized, ends already
/// shown are marked heard, and right after tool results the changed lines
/// are returned as the injection (with the `observed` map in the host
/// metadata; after a history rewrite an unchanged state is recorded anew
/// without a message).
pub(super) async fn at_boundary(
    sh: &Shared,
    snapshot: &LLMContextSnapshot,
    behavior: bool,
) -> Result<Option<Injection>> {
    if let Err(e) = super::todos::sync(sh).await {
        log::warn!("todo sync of {}: {e}", sh.dir.sid());
    }
    let mut meta = snapshot_host_meta(snapshot);
    let current_epoch = meta.observed_epoch == meta.history_epoch;
    let observed = if current_epoch {
        meta.observed.clone()
    } else {
        BTreeMap::new()
    };
    let now = observe(sh).await?;
    mark_delivered(sh, &now, &observed).await?;
    if !after_tool_results(snapshot, behavior) {
        if snapshot
            .state
            .accumulated
            .last()
            .is_some_and(|m| m.role == AiRole::User && m.text_content().starts_with(BLOCK_START))
        {
            // The injection is published with the snapshot.
            crate::fault::point("observe:after_inject_commit");
        }
        return Ok(None);
    }
    let (shown, next) = select(&now.lines, &observed);
    if shown.is_empty() && next == observed && current_epoch {
        return Ok(None);
    }
    meta.observed = next;
    meta.observed_epoch = meta.history_epoch;
    crate::fault::point("observe:before_inject");
    let messages = if shown.is_empty() {
        Vec::new()
    } else {
        vec![AiMessage::text(AiRole::User, render(&shown).trim_end().to_string())]
    };
    Ok(Some(Injection {
        messages,
        host: Some(with_host_meta(snapshot.state.host.as_ref(), &meta)),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(key: &str, text: &str) -> BackgroundLine {
        BackgroundLine::new(key, text)
    }

    #[test]
    fn changed_lines_only() {
        let lines = vec![
            line("todo:T01", "<todo id=\"T01\" status=\"running\">a</todo>"),
            line("work:w1", "work w1: running"),
        ];
        let (shown, map) = select(&lines, &BTreeMap::new());
        assert_eq!(shown.len(), 2);
        let (shown, again) = select(&lines, &map);
        assert!(shown.is_empty());
        assert_eq!(again, map);
        let changed = vec![
            line("todo:T01", "<todo id=\"T01\" status=\"completed\">a — done</todo>"),
            line("work:w1", "work w1: running"),
        ];
        let (shown, _) = select(&changed, &map);
        assert_eq!(shown.iter().map(|l| l.key.as_str()).collect::<Vec<_>>(), ["todo:T01"]);
    }
}
