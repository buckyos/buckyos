//! What a context is shown besides its inputs (`[input] background =
//! "before_inference"`, simple Work Session TODO §3.9, Q14): before every
//! inference, the first of a Turn included.
//!
//! The host brings the TodoList up to its sub sessions' committed state
//! (and dispatches what became ready) and shows, as one
//! `<<background_context>>` block, the lines that changed since they were
//! last shown: the sub sessions no Todo binds and the tasks the run follows
//! that printed something new — only the new output, at most
//! [`TASK_OUTPUT_MAX`] bytes of it. A Todo has no line: it is the helper
//! that creates a work session, whose end or need comes as that session's
//! event, and what the plan's own commands change their output says. The
//! session's runtime and workspace are frozen for its life and told by its
//! first input. Never a sub session's transcript or report: those are read
//! on demand (`todo show --report`).
//!
//! An input batch carries the block in front of its own message
//! ([`for_batch`], as a UI session's chat format does); only an inference
//! after tool results gets it as a message of its own ([`at_boundary`]).
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
use super::receipts::{host_meta_of, snapshot_host_meta, with_host_meta};
use super::shared::{commit_and_report, Shared};

/// run.json `host.extra` key: what the run is shown besides its inputs
/// (`"before_inference"`), frozen with the run.
pub(super) const EXTRA_BACKGROUND: &str = "background";

const BLOCK_START: &str = "<<background_context>>";

/// Most new output of one task a line shows (its end is kept).
pub(super) const TASK_OUTPUT_MAX: usize = 2048;

/// The run.json value of `mode`, `None` for the default (inputs only).
pub(super) fn extra(mode: InputBackground) -> Option<&'static str> {
    match mode {
        InputBackground::Batch => None,
        InputBackground::BeforeInference => Some("before_inference"),
    }
}

/// Whether the run is shown background lines before its inferences.
pub(super) fn enabled(run: &RunHandle) -> bool {
    super::tools::run_flag(run, EXTRA_BACKGROUND)
        .as_ref()
        .and_then(Value::as_str)
        == Some("before_inference")
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

async fn observe(sh: &Shared) -> Result<Observed> {
    let sid = sh.dir.sid().to_string();
    // A Todo's work session is told by its own events (`of todo T01`).
    let bound: std::collections::BTreeSet<String> = super::todos::bound_children(sh)
        .into_iter()
        .map(|b| b.child_sid)
        .collect();
    let mut lines = Vec::new();
    let mut ends = Vec::new();
    for e in sh.agent().sessions().children_of(&[sid]).await? {
        if bound.contains(&e.session_id) {
            continue;
        }
        let st = &e.status;
        let finished = st.run_state == RunState::Finished;
        let line = if finished || st.pending_decision.is_some() || st.needs_input {
            child_line(&e)
        } else {
            // A running session's activity is not a change to decide on.
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

/// The lines that changed since `meta` recorded them as shown, and `meta`
/// after showing them; `None` when there is nothing to record. Ends already
/// shown are marked heard first.
async fn changes(
    sh: &Shared,
    run: &RunHandle,
    mut meta: HostMeta,
) -> Result<Option<(Vec<BackgroundLine>, HostMeta)>> {
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
    let (tasks, task_output) = task_lines(sh, run, &shown_output).await;
    now.lines.extend(tasks);
    let (shown, next) = select(&now.lines, &observed);
    if shown.is_empty() && next == observed && task_output == meta.task_output && current_epoch {
        return Ok(None);
    }
    meta.observed = next;
    meta.task_output = task_output;
    meta.observed_epoch = meta.history_epoch;
    Ok(Some((shown, meta)))
}

/// The background an input batch carries in front of its message.
pub(super) struct BatchBackground {
    /// The `<<background_context>>` block (empty: nothing to show, only
    /// the record moves).
    pub block: String,
    meta: HostMeta,
}

impl BatchBackground {
    /// Record the lines as shown in the host metadata committed with the
    /// batch.
    pub(super) fn record(&self, meta: &mut HostMeta) {
        meta.observed = self.meta.observed.clone();
        meta.task_output = self.meta.task_output.clone();
        meta.observed_epoch = self.meta.observed_epoch;
    }
}

/// The background of an input batch of a run that is shown one: computed
/// right before the batch is committed, its record committed with it, so
/// the inference that answers the batch gets no message of its own.
pub(super) async fn for_batch(
    sh: &Shared,
    run: &RunHandle,
    host: Option<&Value>,
) -> Result<Option<BatchBackground>> {
    if !enabled(run) {
        return Ok(None);
    }
    Ok(changes(sh, run, host_meta_of(host))
        .await?
        .map(|(shown, meta)| BatchBackground {
            block: render(&shown),
            meta,
        }))
}

/// The checkpoint hook's observation at an inference boundary (after the
/// snapshot was published): the TodoList is synchronized (and what became
/// ready dispatched), ends already shown are marked heard, and the changed
/// lines are returned as the injection (with the `observed` map in the host
/// metadata; after a history rewrite an unchanged state is recorded anew
/// without a message). Nothing right after an input batch: the batch
/// showed the background in its own message.
pub(super) async fn at_boundary(
    sh: &Shared,
    run: &RunHandle,
    snapshot: &LLMContextSnapshot,
) -> Result<Option<Injection>> {
    if let Err(e) = super::todos::sync(sh).await {
        log::warn!("todo sync of {}: {e}", sh.dir.sid());
    }
    let meta = snapshot_host_meta(snapshot);
    if let Some(last) = snapshot
        .state
        .accumulated
        .last()
        .filter(|m| m.role == AiRole::User)
        .map(|m| m.text_content())
    {
        let batch = meta
            .input_receipts
            .last()
            .and_then(|r| r.parts.iter().rfind(|p| p.part == PART_INPUT))
            .is_some_and(|p| p.text == last);
        if batch {
            return Ok(None);
        }
        if last.starts_with(BLOCK_START) {
            // Just injected at this boundary (the waist asks again after an
            // injection): published with the snapshot.
            crate::fault::point("observe:after_inject_commit");
            return Ok(None);
        }
    }
    let Some((shown, meta)) = changes(sh, run, meta).await? else {
        return Ok(None);
    };
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
