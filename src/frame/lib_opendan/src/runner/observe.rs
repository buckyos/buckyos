//! The tasks a session follows, as lines of its background (simple Work
//! Session TODO §3.2, Q8): a running task that printed something since its
//! output was last shown gets a line with only the new output, at most
//! [`TASK_OUTPUT_MAX`] bytes of it (its end is kept). The task's end is its
//! event (`<<task_result>>`), never a line; like every background line, the
//! output is only seen in an input that places it (`background.rs`).
//!
//! How far a task's output was shown is `background_shown["task:<id>"]`
//! (`<stdout bytes>,<stderr bytes>`). The call that became the task showed
//! the output so far: the boundary right after it records where the output
//! stands ([`note_started`]), without showing anything.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::Result;
use crate::protocol::*;
use crate::session::runs::RunHandle;

use super::background::BackgroundLine;
use super::shared::{commit_and_report, Shared};

/// Most new output of one task a line shows (its end is kept).
pub(super) const TASK_OUTPUT_MAX: usize = 2048;

const LOCAL_SHELL_PREFIX: &str = "local:shell:";

fn key(task_id: &str) -> String {
    format!("task:{task_id}")
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
fn read_from(path: &Path, from: u64, max: usize) -> (u64, String, bool) {
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

fn lengths(dir: &Path) -> String {
    let len = |f: &str| std::fs::metadata(dir.join(f)).map(|m| m.len()).unwrap_or(0);
    format!("{},{}", len("stdout"), len("stderr"))
}

fn parse(mark: &str) -> Option<[u64; 2]> {
    let (out, err) = mark.split_once(',')?;
    Some([out.parse().ok()?, err.parse().ok()?])
}

/// The execution directory of a `local:shell:<call>` task, under the run
/// whose call started it.
fn exec_dir(sh: &Shared, task_id: &str) -> Option<PathBuf> {
    let call = task_id.strip_prefix(LOCAL_SHELL_PREFIX)?;
    let runs = sh.dir.runs();
    runs.list()
        .ok()?
        .into_iter()
        .rev()
        .map(|run| runs.dir().join(run).join("exec").join(call))
        .find(|d| d.is_dir())
}

/// The tasks the session follows: the watched ones and those its live run
/// noted (a batch joining an open Turn comes before the Turn hands them
/// over).
fn followed(sh: &Shared, state: &SessionState) -> Vec<String> {
    let mut tasks = state.watched_tasks.clone();
    let noted: Vec<String> = state
        .live_run
        .as_ref()
        .and_then(|l| sh.dir.runs().record(&l.run_id).ok())
        .and_then(|r| r.host)
        .and_then(|h| serde_json::from_value(h.extra["tasks"].clone()).ok())
        .unwrap_or_default();
    for t in noted {
        if !tasks.contains(&t) {
            tasks.push(t);
        }
    }
    tasks.retain(|t| !t.starts_with(super::children::SESSION_TASK_PREFIX));
    tasks
}

/// The candidate lines of the running tasks the session follows: the new
/// output of each, or (nothing new) a record that keeps where it stands.
pub(super) async fn task_lines(sh: &Arc<Shared>, state: &SessionState) -> Vec<BackgroundLine> {
    let resolver = super::drive::task_resolver(sh);
    let mut lines = Vec::new();
    for task_id in followed(sh, state) {
        if !resolver.can_resolve(&task_id) {
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
        let key = key(&task_id);
        let before = state.background_shown.get(&key);
        let Some(dir) = exec_dir(sh, &task_id) else {
            // Another runtime: its tail, shown when it changed.
            let tail = output_tail.trim_end();
            if !tail.is_empty() {
                lines.push(BackgroundLine::new(
                    key,
                    format!("task {task_id} · {brief} · output so far (tail):\n{tail}"),
                ));
            } else if let Some(b) = before {
                lines.push(BackgroundLine::marked(key, "", b.clone()));
            }
            continue;
        };
        let Some([out_at, err_at]) = before.and_then(|b| parse(b)) else {
            // Not recorded at its call (an earlier host's run): from now.
            lines.push(BackgroundLine::marked(key, "", lengths(&dir)));
            continue;
        };
        let (out_len, out, out_cut) = read_from(&dir.join("stdout"), out_at, TASK_OUTPUT_MAX);
        let (err_len, err, err_cut) = read_from(&dir.join("stderr"), err_at, TASK_OUTPUT_MAX);
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
        if text.trim().is_empty() {
            lines.push(BackgroundLine::marked(key, "", format!("{out_at},{err_at}")));
            continue;
        }
        lines.push(BackgroundLine::marked(
            key,
            format!(
                "task {task_id} · {brief} · running {} · new output:\n{text}",
                fmt_elapsed(elapsed_ms)
            ),
            format!("{out_len},{err_len}"),
        ));
    }
    lines
}

/// At an inference boundary: the tasks the run's calls just started are
/// recorded where their output stands (their call's result showed it so
/// far). Nothing is shown here.
pub(super) async fn note_started(sh: &Shared, run: &RunHandle) -> Result<()> {
    let run_dir = sh.dir.runs().dir().join(run.run_id()).join("exec");
    let mut s = sh.session.lock().await;
    let mut changed = false;
    for task_id in run.noted_tasks() {
        let Some(call) = task_id.strip_prefix(LOCAL_SHELL_PREFIX) else {
            continue;
        };
        let key = key(&task_id);
        if s.state.background_shown.contains_key(&key) || !run_dir.join(call).is_dir() {
            continue;
        }
        s.state.background_shown.insert(key, lengths(&run_dir.join(call)));
        changed = true;
    }
    if changed {
        commit_and_report(sh, &mut s).await?;
    }
    Ok(())
}
