//! The TodoList of a plan session (`doc/opendan/Agent TodoList.md`): where it
//! is stored, how a ready Todo becomes a work session of its own (the host
//! dispatches every Todo whose prerequisites completed, nobody asks for it),
//! how a work session's committed state maps to its attempt, how an update
//! or a control reaches the work session of a running Todo, and where an
//! attempt's report is read. The records and their rules are
//! `agent_tool::todo_tools`; the runner side (write-back and scheduling at
//! batch boundaries, the end) is `runner::todos`.
//!
//! One writer per list: the session's host and the tool processes of its
//! runs (`agent-session todo …`), under the session's lease. A sub session
//! never writes its parent's list.

use std::collections::BTreeSet;
use std::path::PathBuf;

pub use agent_tool::todo_tools::{
    one_line, AttemptTarget, ChildObservation, DispatchState, NewTodo, ReportRef, TodoAttempt,
    TodoList, TodoMode, TodoPatch, TodoRecord, TodoStatus, TodoStore, TodoSummary,
    UpdateOutcome, TODO_LIST_FILE, TODO_LIST_SCHEMA,
};
use serde::Serialize;
use serde_json::Value;

use crate::api::{create_sub_session, sub_session_id, SubSessionSpec, SubWorkspace};
use crate::channel::InputChannelFactory;
use crate::error::{OpenDanError, Result};
use crate::protocol::*;
use crate::session::SessionDir;
use crate::state::AgentStateClient;

/// Behavior an exploration's work session starts with, unless told otherwise.
pub const EXPLORE_BEHAVIOR: &str = "explore";
/// Behavior a serial Todo's work session starts with, unless told otherwise.
pub const WORK_BEHAVIOR: &str = "work_simple";
/// Sub sessions the host drives at the same time (`xagent --max-children`),
/// exported to the session's tool processes: a ready Todo beyond it waits
/// for the next scheduling point.
pub const ENV_MAX_CHILDREN: &str = "LIBOPENDAN_MAX_CHILDREN";
/// `source.kind` of the events a plan sends the work session of a running
/// Todo (`todo:<todo_id>`).
pub const TODO_EVENT_SOURCE: &str = "todo";
/// The Todo the work session executes was updated.
pub const TODO_UPDATED: &str = "updated";

pub fn store_of(sd: &SessionDir) -> TodoStore {
    TodoStore::in_dir(sd.state_dir())
}

/// A todo store / rule error as a session error.
pub fn todo_error(e: agent_tool::AgentToolError) -> OpenDanError {
    match e {
        agent_tool::AgentToolError::InvalidArgs(m) => OpenDanError::InvalidArgument(m),
        agent_tool::AgentToolError::NotFound(m) => OpenDanError::NotFound(m),
        other => OpenDanError::Other(other.to_string()),
    }
}

/// How Todos are dispatched.
#[derive(Debug, Clone, Default)]
pub struct DispatchOptions {
    /// Session class of the work sessions (default `work`).
    pub class: Option<String>,
    /// The host's limit of sub sessions driven at once (`None`: unknown).
    pub max_children: Option<usize>,
}

impl DispatchOptions {
    /// The host's limit from the tool process's environment.
    pub fn from_env() -> Self {
        Self {
            max_children: std::env::var(ENV_MAX_CHILDREN)
                .ok()
                .and_then(|v| v.parse().ok()),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Dispatched {
    pub todo_id: String,
    pub attempt: u32,
    pub child_sid: String,
    pub path: PathBuf,
}

/// What scheduling did with a ready Todo.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Scheduled {
    /// Its work session runs.
    Dispatched(Dispatched),
    /// Held back by the limit of sub sessions running at once: it stays
    /// pending and starts at a later scheduling point.
    Held { todo_id: String, reason: String },
    /// Its work session could not be created as the Todo asks: blocked
    /// until the plan changes it.
    Blocked { todo_id: String, reason: String },
}

/// The idempotency key of an attempt's work session: `(parent, todo,
/// attempt)` — two Todos never share one, a dispatch redone for the same
/// attempt finds it again.
pub fn attempt_key(todo_id: &str, attempt: u32) -> String {
    format!("todo:{todo_id}:{attempt}")
}

/// The workspace a Todo's work session takes: what the Todo says, else
/// none for an exploration and the plan's own for a serial Todo.
fn todo_workspace(t: &TodoRecord) -> SubWorkspace {
    match t.workspace.as_deref() {
        Some("inherit") => SubWorkspace::Inherit,
        Some("none") => SubWorkspace::None,
        Some(id) => SubWorkspace::Id(id.to_string()),
        None if t.mode == TodoMode::Serial => SubWorkspace::Inherit,
        None => SubWorkspace::None,
    }
}

/// The behavior a Todo's work session starts with: the Todo's own, else
/// the mode's default.
fn todo_behavior(t: &TodoRecord) -> String {
    t.behavior.clone().unwrap_or_else(|| match t.mode {
        TodoMode::Explore => EXPLORE_BEHAVIOR.to_string(),
        TodoMode::Serial => WORK_BEHAVIOR.to_string(),
    })
}

fn child_spec(list: &TodoList, t: &TodoRecord, attempt: u32, opts: &DispatchOptions) -> SubSessionSpec {
    let mut task = t.task_text(list, attempt);
    let note = match t.mode {
        TodoMode::Explore => {
            "\nYou are an exploration of a plan session: do exactly this Todo, within its scope. \
Leads outside the scope go into your report for the plan, not into more work. \
Your Todo may be updated while you work: the update comes to you as input. \
Finish with a final report (is_end) the plan can decide on.\n"
        }
        TodoMode::Serial => {
            "\nYou are the work session of exactly this Todo of a plan session, on your own session-runtime and in your own working directory. \
When you have a workspace, the project files are there: change them there and say what you changed; other results go into your report's artifacts. \
Do only this Todo, within its scope; leads outside it go into your report. \
Your Todo may be updated while you work: the update comes to you as input. \
Finish with a final report (is_end) with the evidence for each acceptance criterion; the plan decides on it.\n"
        }
    };
    task.push_str(note);
    SubSessionSpec {
        objective: t.task_text(list, attempt),
        title: Some(t.title.clone()),
        msgs: vec![task],
        class: opts.class.clone(),
        behavior: Some(todo_behavior(t)),
        // The Todo's cmd_skills; without any the plan's are inherited.
        cmd_skills: (!t.skills.is_empty()).then(|| t.skills.clone()),
        workspace: todo_workspace(t),
        runtime_id: t.runtime_id.clone(),
        report: ReportMode::Final,
        queued: true,
        key: Some(attempt_key(&t.todo_id, attempt)),
        ..Default::default()
    }
}

/// Sub sessions `parent` may still start: the host's limit and the plan's
/// `max_sub_sessions`, less those running.
async fn room(agent: &dyn AgentStateClient, parent: &SessionDir, opts: &DispatchOptions) -> Result<Option<String>> {
    let policy = parent.config()?.session.policy;
    let max = opts
        .max_children
        .unwrap_or(usize::MAX)
        .min(policy.max_sub_sessions as usize)
        .max(1);
    let running = agent
        .sessions()
        .children_of(&[parent.sid().to_string()])
        .await?
        .into_iter()
        .filter(|e| e.status.run_state != RunState::Finished)
        .count();
    Ok((running >= max).then(|| {
        format!("{running} work sessions run already, at most {max} at once; it starts when one ends")
    }))
}

/// Dispatch the pending Todo `todo_id` of `parent` as a work session of its
/// own: record the attempt (`dispatching`) → create the work session → bind
/// it (`running`). The list stays locked meanwhile, so nobody sees a
/// dispatch half done; after a crash the attempt is found again by its work
/// session's id (bound if the session exists, withdrawn if not). The limit
/// of sub sessions holds it back without a record; a work session that
/// cannot be created as the Todo asks blocks it.
pub async fn dispatch(
    agent: &dyn AgentStateClient,
    who: &str,
    channels: &dyn InputChannelFactory,
    parent: &SessionDir,
    todo_id: &str,
    opts: &DispatchOptions,
) -> Result<Scheduled> {
    let parent_sid = parent.sid().to_string();
    let store = store_of(parent);
    let lock = store.lock().map_err(todo_error)?;
    let list = lock.load().map_err(todo_error)?;
    let t = list
        .get(todo_id)
        .ok_or_else(|| OpenDanError::NotFound(format!("todo {todo_id}")))?;
    let attempt = match t.latest() {
        Some(a) if a.dispatch == Some(DispatchState::Dispatching) => a.attempt,
        _ => {
            list.admit(todo_id).map_err(todo_error)?;
            t.next_attempt()
        }
    };
    let behavior = todo_behavior(t);
    let sub = child_spec(&list, t, attempt, opts);
    let child_sid = sub_session_id(agent, who, &parent_sid, &sub).await?;
    if agent.sessions().lookup(&child_sid).await?.is_none() {
        if let Some(reason) = room(agent, parent, opts).await? {
            return Ok(Scheduled::Held {
                todo_id: todo_id.to_string(),
                reason,
            });
        }
    }
    let now = crate::now_ms();
    let n = lock
        .update(|l| l.begin_dispatch(todo_id, &child_sid, now))
        .map_err(todo_error)?;
    crate::fault::point("todo:after_dispatch_record");
    let created = match create_sub_session(agent, who, &parent_sid, sub, channels).await {
        Ok(sd) => sd,
        // What the Todo asks for cannot be had as it is (a bad parameter; a
        // runtime that is off, expired, retired or read-only for work that
        // writes): it waits with the reason until the plan changes it.
        Err(OpenDanError::InvalidArgument(reason)) | Err(OpenDanError::Bind(reason)) => {
            lock.update(|l| l.withdraw_dispatch(todo_id, n, Some(&reason), crate::now_ms()))
                .map_err(todo_error)?;
            return Ok(Scheduled::Blocked {
                todo_id: todo_id.to_string(),
                reason,
            });
        }
        Err(e) => {
            // Whether the session came to exist decides: bound if it did,
            // withdrawn if not (the next scheduling point creates it then).
            if agent.sessions().lookup(&child_sid).await.ok().flatten().is_none() {
                let _ = lock.update(|l| l.withdraw_dispatch(todo_id, n, None, crate::now_ms()));
            }
            return Err(e);
        }
    };
    crate::fault::point("todo:after_child_created");
    let target = created
        .config()
        .map(|c| AttemptTarget {
            runtime_id: Some(c.runtime.runtime_id().to_string()),
            baseline: c.runtime.frozen.as_ref().map(|f| f.baseline().as_str().to_string()),
            workspace_id: c.workspace.map(|w| w.workspace_id),
            behavior: Some(behavior.clone()),
        })
        .unwrap_or_default();
    lock.update(|l| l.bind_dispatch(todo_id, n, crate::now_ms(), target))
        .map_err(todo_error)?;
    crate::fault::point("todo:after_bind");
    Ok(Scheduled::Dispatched(Dispatched {
        todo_id: todo_id.to_string(),
        attempt: n,
        child_sid,
        path: created.path().to_path_buf(),
    }))
}

/// One scheduling point: every ready Todo of `parent` is dispatched, in
/// creation order, until the limit of sub sessions holds one back. Nothing
/// is dispatched for a plan that is paused, stopping or finished.
pub async fn schedule(
    agent: &dyn AgentStateClient,
    who: &str,
    channels: &dyn InputChannelFactory,
    parent: &SessionDir,
    opts: &DispatchOptions,
) -> Result<Vec<Scheduled>> {
    let state = parent.state()?;
    if state.is_finished() || state.paused.is_some() || state.stop_requested {
        return Ok(Vec::new());
    }
    let store = store_of(parent);
    let mut out = Vec::new();
    let mut tried = BTreeSet::new();
    loop {
        let list = store.load().map_err(todo_error)?;
        let Some(id) = list.ready().into_iter().find(|id| !tried.contains(id)) else {
            break;
        };
        tried.insert(id.clone());
        let s = match dispatch(agent, who, channels, parent, &id, opts).await {
            Ok(s) => s,
            // Changed meanwhile (another writer dispatched it): next one.
            Err(OpenDanError::InvalidArgument(_)) => continue,
            Err(e) => return Err(e),
        };
        let held = matches!(s, Scheduled::Held { .. });
        out.push(s);
        if held {
            break;
        }
    }
    Ok(out)
}

/// What an attempt is, from its work session's committed state.
pub fn observe(entry: &RegistryEntry, state: Option<&SessionState>) -> ChildObservation {
    let st = &entry.status;
    let report = state.and_then(|s| s.final_report.as_ref());
    // Retries used up: the plan learns the provider was unavailable, not
    // only the last error's text.
    let exhausted = state
        .and_then(|s| s.last_error.as_ref())
        .and_then(crate::runner::exhausted_line);
    let error = exhausted.clone().or_else(|| {
        state
            .and_then(|s| s.last_error.as_ref())
            .and_then(|e| e.get("message").and_then(Value::as_str).map(str::to_string))
    });
    if st.run_state == RunState::Finished {
        let status = match st.outcome {
            Some(Outcome::Succeeded) => TodoStatus::Completed,
            Some(Outcome::Stopped) => TodoStatus::Stopped,
            _ if state
                .and_then(|s| s.last_error.as_ref())
                .is_some_and(|e| e["kind"] == "budget_exhausted") =>
            {
                TodoStatus::Timeout
            }
            _ => TodoStatus::Failed,
        };
        let summary = report
            .map(|r| one_line(&r.report))
            .filter(|s| !s.is_empty())
            .or_else(|| exhausted.as_deref().map(one_line))
            .or_else(|| Some(one_line(&st.one_line_status)).filter(|s| !s.is_empty()));
        return ChildObservation {
            status,
            summary,
            report: report.map(|r| ReportRef {
                session_id: entry.session_id.clone(),
                report_id: r.id.clone(),
            }),
            artifacts: report.map(artifact_refs).unwrap_or_default(),
            error: (status != TodoStatus::Completed).then_some(error).flatten(),
        };
    }
    ChildObservation {
        status: TodoStatus::Running,
        summary: None,
        report: None,
        artifacts: Vec::new(),
        error: None,
    }
}

/// Artifacts of a report as references: object ids where there are, the
/// stable copies otherwise.
pub fn artifact_refs(r: &ReportSubmission) -> Vec<String> {
    r.artifacts
        .iter()
        .map(|a| a.obj_id.clone().unwrap_or_else(|| format!("{} ({})", a.path, a.reference)))
        .collect()
}

/// A report as the plan reads it (`todo show <id> --report`).
#[derive(Debug, Clone, Serialize)]
pub struct ReportView {
    pub report_ref: ReportRef,
    pub report: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ReportArtifact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Where the whole work session can be read.
    pub session_dir: PathBuf,
}

/// The report an attempt points to: the final report of its work session.
pub async fn read_report(agent: &dyn AgentStateClient, attempt: &TodoAttempt) -> Result<Option<ReportView>> {
    let Some(r) = attempt.report.clone() else {
        return Ok(None);
    };
    let Some(e) = agent.sessions().lookup(&r.session_id).await? else {
        return Ok(None);
    };
    let child = SessionDir::open(&e.location)?;
    let st = child.state()?;
    Ok(st.final_report.filter(|s| s.id == r.report_id).map(|s| ReportView {
        report_ref: r,
        report: s.report,
        artifacts: s.artifacts,
        result: s.result,
        session_dir: child.path().to_path_buf(),
    }))
}

/// Write what the work sessions committed back into the list (TodoList
/// §2.1): an attempt follows its work session's registry state — never the
/// session's own word —, and a dispatch that died before its work session
/// existed is withdrawn (the list lock is free, so no dispatch is in
/// progress; the Todo is pending again). `true`: the list changed.
pub async fn sync(agent: &dyn AgentStateClient, sd: &SessionDir) -> Result<bool> {
    let store = store_of(sd);
    let list = store.load().map_err(todo_error)?;
    let open = list.in_flight();
    if open.is_empty() {
        return Ok(false);
    }
    let mut seen = Vec::new();
    for (todo_id, attempt, sid) in &open {
        let entry = agent.sessions().lookup(sid).await?;
        let child_state = entry
            .as_ref()
            .and_then(|e| SessionDir::open(&e.location).ok())
            .and_then(|d| d.state().ok());
        seen.push((todo_id.clone(), *attempt, entry.map(|e| observe(&e, child_state.as_ref()))));
    }
    let now = crate::now_ms();
    store
        .update(|l| {
            let mut changed = false;
            for (todo_id, attempt, obs) in seen {
                let Some(a) = l.get(&todo_id).and_then(|t| t.attempt(attempt)).cloned() else {
                    continue;
                };
                match obs {
                    Some(o) => changed |= l.observe_child(&todo_id, attempt, o, now)?,
                    None if a.dispatch == Some(DispatchState::Dispatching) => {
                        l.withdraw_dispatch(&todo_id, attempt, None, now)?;
                        changed = true;
                    }
                    None => {
                        changed |= l.observe_child(
                            &todo_id,
                            attempt,
                            ChildObservation {
                                status: TodoStatus::Failed,
                                summary: None,
                                report: None,
                                artifacts: Vec::new(),
                                error: Some("its work session is not registered".into()),
                            },
                            now,
                        )?;
                    }
                }
            }
            Ok(changed)
        })
        .map_err(todo_error)
}

/// The plan ended: attempts still running end as stopped (their work
/// sessions are stopped with it).
pub fn stop_open(sd: &SessionDir, reason: &str) -> Result<Vec<String>> {
    let now = crate::now_ms();
    store_of(sd)
        .update(|l| {
            let mut stopped = Vec::new();
            for (todo_id, attempt, _) in l.in_flight() {
                l.observe_child(
                    &todo_id,
                    attempt,
                    ChildObservation {
                        status: TodoStatus::Stopped,
                        summary: None,
                        report: None,
                        artifacts: Vec::new(),
                        error: Some(reason.to_string()),
                    },
                    now,
                )?;
                stopped.push(todo_id);
            }
            Ok(stopped)
        })
        .map_err(todo_error)
}

/// Tell the work session of a running Todo that the Todo was updated: an
/// Input event in its queue (`todo:<id>`, `updated`) that opens its next
/// Turn. The summary names the change; the work session's host puts the
/// Todo's whole new task next to it ([`expand_update`]). Idempotent per
/// revision of the Todo.
pub async fn post_update(
    agent: &dyn AgentStateClient,
    who: &str,
    parent: &SessionDir,
    t: &TodoRecord,
    child_sid: &str,
    attempt: u32,
    out: &UpdateOutcome,
    reason: &str,
) -> Result<u64> {
    let mut summary = format!(
        "Todo {} was updated ({}): {}",
        t.todo_id,
        out.changed.join(", "),
        reason.trim()
    );
    if summary.len() > MAX_EVENT_SUMMARY_BYTES {
        let mut end = MAX_EVENT_SUMMARY_BYTES - 3;
        while !summary.is_char_boundary(end) {
            end -= 1;
        }
        summary.truncate(end);
        summary.push('…');
    }
    let ev = AgentEvent {
        subscription_id: None,
        source: EventSource::new(TODO_EVENT_SOURCE, t.todo_id.clone()),
        event: TODO_UPDATED.into(),
        seq: Some(t.rev),
        summary,
        data_ref: Some(format!("{}#{}@{attempt}", parent.sid(), t.todo_id)),
        terminal: false,
        call_id: None,
    };
    let key = format!("{}:{}:{attempt}#updated@{}", parent.sid(), t.todo_id, t.rev);
    agent
        .sessions()
        .post_input(child_sid, &PostedInput::event(who, key, ev))
        .await
}

/// The update event a work session takes in, with the whole task of its
/// Todo as the plan's list holds it now (`data_ref` =
/// `<plan sid>#<todo id>@<attempt>`). Unchanged when the list cannot be
/// read.
pub async fn expand_update(agent: &dyn AgentStateClient, ev: &AgentEvent) -> AgentEvent {
    let mut out = ev.clone();
    if ev.source.kind != TODO_EVENT_SOURCE {
        return out;
    }
    let Some((plan, attempt)) = ev
        .data_ref
        .as_deref()
        .and_then(|r| r.split_once('#'))
        .and_then(|(plan, rest)| Some((plan, rest.split_once('@')?.1.parse::<u32>().ok()?)))
    else {
        return out;
    };
    let Ok(Some(e)) = agent.sessions().lookup(plan).await else {
        return out;
    };
    let Ok(sd) = SessionDir::open(&e.location) else {
        return out;
    };
    let Ok(list) = store_of(&sd).load() else {
        return out;
    };
    if let Some(t) = list.get(&ev.source.id) {
        out.summary = format!("{}\n\nYour task now:\n{}", ev.summary, t.task_text(&list, attempt));
    }
    out
}

/// Send `pause` / `resume` / `stop` to the work session of a running Todo.
pub async fn control_child(
    agent: &dyn AgentStateClient,
    who: &str,
    child_sid: &str,
    key: String,
    command: ControlCommand,
) -> Result<u64> {
    agent
        .sessions()
        .post_input(child_sid, &PostedInput::control(who, key, command))
        .await
}
