//! What a context is shown besides its inputs (`[input] background`):
//!
//! - `"after_tools"` (plan_simple TODO §4.4): after its tool results — the
//!   action → observation → next decision of the behavior loop, for a
//!   function call loop;
//! - `"before_inference"` (simple Work Session TODO §3.9, Q14): before
//!   every inference, the first of a Turn included.
//!
//! The host brings the TodoList up to its sub sessions' committed state and
//! appends, as one `<<background_context>>` message, the lines that changed
//! since they were last shown: each Todo (its compact view line), the sub
//! sessions no Todo binds and (`before_inference`) the tasks the run
//! follows that printed something new — only the new output, at most
//! [`TASK_OUTPUT_MAX`] bytes of it. The session's runtime and workspace are
//! frozen for its life and told by its first input. Never a sub session's
//! transcript or report: those are read on demand (`todo show --report`).
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

/// run.json `host.extra` key: what the run is shown besides its inputs
/// (`"after_tools"` | `"before_inference"`), frozen with the run.
pub(super) const EXTRA_BACKGROUND: &str = "background";

const BLOCK_START: &str = "<<background_context>>";

/// Most new output of one task a line shows (its end is kept).
pub(super) const TASK_OUTPUT_MAX: usize = 2048;

/// When the run is shown background lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Background {
    AfterTools,
    BeforeInference,
}

impl Background {
    /// The run.json value of `mode`, `None` for the default (inputs only).
    pub(super) fn extra(mode: InputBackground) -> Option<&'static str> {
        match mode {
            InputBackground::Batch => None,
            InputBackground::AfterTools => Some("after_tools"),
            InputBackground::BeforeInference => Some("before_inference"),
        }
    }
}

pub(super) fn enabled(run: &RunHandle) -> Option<Background> {
    match super::tools::run_flag(run, EXTRA_BACKGROUND)
        .as_ref()
        .and_then(Value::as_str)
    {
        Some("after_tools") => Some(Background::AfterTools),
        Some("before_inference") => Some(Background::BeforeInference),
        _ => None,
    }
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

/// Bytes `[from, len)` of a file, at most `max` of them (its end).
fn read_from(path: &std::path::Path, from: u64, max: usize) -> (u64, String, bool) {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else {
        return (from, String::new(), false);
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    if len <= from {
        return (len.min(from), String::new(), false);
    }
    let start = from.max(len.saturating_sub(max as u64));
    let mut buf = Vec::new();
    if f.seek(SeekFrom::Start(start)).is_err() || f.read_to_end(&mut buf).is_err() {
        return (from, String::new(), false);
    }
    (len, String::from_utf8_lossy(&buf).to_string(), start > from)
}

/// The tasks the run follows that printed something since they were last
/// shown: one line each with the new output, and how far each task's
/// output is shown after them. A task seen for the first time only records
/// where it stands (its call's result just showed the output so far).
async fn task_lines(
    sh: &Shared,
    run: &RunHandle,
    shown: &BTreeMap<String, [u64; 2]>,
) -> (Vec<BackgroundLine>, BTreeMap<String, [u64; 2]>) {
    let mut lines = Vec::new();
    let mut next = BTreeMap::new();
    let Some(resolver) = sh.tasks.lock().expect("tasks").clone() else {
        return (lines, next);
    };
    let run_dir = sh.dir.runs().dir().join(run.run_id());
    for task_id in run.noted_tasks() {
        if task_id.starts_with(super::children::SESSION_TASK_PREFIX) {
            continue;
        }
        let agent_tool::TaskState::Running {
            brief,
            elapsed_ms,
            output_tail,
            ..
        } = resolver.state(&task_id).await
        else {
            continue;
        };
        let before = shown.get(&task_id).copied();
        let dir = task_id
            .strip_prefix("local:shell:")
            .map(|call| run_dir.join("exec").join(call))
            .filter(|d| d.is_dir());
        let (new_output, at) = match &dir {
            Some(dir) => {
                let [out_at, err_at] = before.unwrap_or([u64::MAX, u64::MAX]);
                let (out_len, out, out_cut) = read_from(&dir.join("stdout"), out_at, TASK_OUTPUT_MAX);
                let (err_len, err, err_cut) = read_from(&dir.join("stderr"), err_at, TASK_OUTPUT_MAX);
                let at = [out_len, err_len];
                if before.is_none() {
                    // First seen: where it stands is what its call showed.
                    let at = [
                        std::fs::metadata(dir.join("stdout")).map(|m| m.len()).unwrap_or(0),
                        std::fs::metadata(dir.join("stderr")).map(|m| m.len()).unwrap_or(0),
                    ];
                    next.insert(task_id.clone(), at);
                    continue;
                }
                let mut text = String::new();
                if out_cut || err_cut {
                    text.push_str("[… earlier new output omitted]\n");
                }
                text.push_str(out.trim_end());
                if !err.trim().is_empty() {
                    if !text.trim().is_empty() {
                        text.push_str("\n--- stderr ---\n");
                    }
                    text.push_str(err.trim_end());
                }
                (text, at)
            }
            // Another runtime: its tail, shown when it changed.
            None => (output_tail.trim_end().to_string(), [0, 0]),
        };
        next.insert(task_id.clone(), at);
        if new_output.trim().is_empty() {
            continue;
        }
        let text = match &dir {
            Some(_) => format!(
                "task {task_id} · {brief} · running {} · new output:\n{new_output}",
                fmt_elapsed(elapsed_ms)
            ),
            None => format!("task {task_id} · {brief} · output so far (tail):\n{new_output}"),
        };
        lines.push(BackgroundLine::new(format!("task:{task_id}"), text));
    }
    (lines, next)
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
    run: &RunHandle,
    snapshot: &LLMContextSnapshot,
    behavior: bool,
    mode: Background,
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
    let mut now = observe(sh).await?;
    mark_delivered(sh, &now, &observed).await?;
    let shown_output = if current_epoch {
        meta.task_output.clone()
    } else {
        BTreeMap::new()
    };
    let (tasks, task_output) = if mode == Background::BeforeInference {
        task_lines(sh, run, &shown_output).await
    } else {
        (Vec::new(), BTreeMap::new())
    };
    now.lines.extend(tasks);
    if mode == Background::AfterTools && !after_tool_results(snapshot, behavior) {
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
    if snapshot
        .state
        .accumulated
        .last()
        .is_some_and(|m| m.role == AiRole::User && m.text_content().starts_with(BLOCK_START))
    {
        // Just injected at this boundary (the waist asks again after an
        // injection): published with the snapshot.
        crate::fault::point("observe:after_inject_commit");
        return Ok(None);
    }
    let (shown, next) = select(&now.lines, &observed);
    if shown.is_empty() && next == observed && task_output == meta.task_output && current_epoch {
        return Ok(None);
    }
    meta.observed = next;
    meta.task_output = task_output;
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
