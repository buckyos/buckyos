//! The TodoList of a Work Session (`doc/opendan/Agent TodoList.md`): where it
//! is stored, how an exploration Todo becomes a sub session, how a sub
//! session's committed state maps to its attempt, and where an attempt's
//! report is read. The records and their rules are `agent_tool::todo_tools`;
//! the runner side (serial admission, write-back at batch boundaries, the
//! PLAN's view, the end check) is `runner::todos`.
//!
//! One writer per list: the session's host and the tool processes of its
//! runs (`agent-session todo …`), under the session's lease. A sub session
//! never writes its parent's list.

use std::path::PathBuf;

pub use agent_tool::todo_tools::{
    one_line, AttemptTarget, CallRef, ChildObservation, DispatchState, NewTodo, ReportRef,
    ReviewState, ReviewVerdict, SerialPick, TodoAttempt, TodoList, TodoMode, TodoRecord,
    TodoStatus, TodoStore, TODO_LIST_FILE, TODO_LIST_SCHEMA,
};
use serde::Serialize;
use serde_json::Value;

use crate::api::{create_sub_session, sub_session_id, SubSessionSpec, SubWorkspace};
use crate::channel::InputChannelFactory;
use crate::error::{OpenDanError, Result};
use crate::protocol::*;
use crate::session::SessionDir;
use crate::state::AgentStateClient;

/// Behavior an exploration sub session starts with, unless told otherwise.
pub const EXPLORE_BEHAVIOR: &str = "explore";
/// Behavior a serial Todo's own work session starts with (one bounded task
/// on another runtime or workspace), unless told otherwise.
pub const SERIAL_SESSION_BEHAVIOR: &str = "work_simple";
/// Sub sessions the host drives at the same time (`xagent --max-children`),
/// exported to the session's tool processes so a dispatch beyond it is
/// refused instead of silently queued.
pub const ENV_MAX_CHILDREN: &str = "LIBOPENDAN_MAX_CHILDREN";
/// Directory (in the state directory) of report submissions kept beyond
/// the run that made them: a serial attempt's report outlives its run.
pub const REPORTS_DIR: &str = "reports";

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

/// How an exploration is dispatched.
#[derive(Debug, Clone, Default)]
pub struct ExploreOptions {
    /// Entry behavior of the sub session (default [`EXPLORE_BEHAVIOR`]).
    pub behavior: Option<String>,
    /// Session class (default `work`).
    pub class: Option<String>,
    /// The host's limit of sub sessions driven at once (`None`: unknown).
    pub max_children: Option<usize>,
}

impl ExploreOptions {
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

/// The idempotency key of an exploration attempt's sub session:
/// `(parent, todo, attempt)` — two Todos of one shell batch never share a
/// sub session, a retried dispatch of the same attempt finds it again.
pub fn explore_key(todo_id: &str, attempt: u32) -> String {
    format!("todo:{todo_id}:{attempt}")
}

/// The workspace a Todo's sub session takes: what the Todo says, else none
/// for an exploration and the session's own for a serial Todo.
fn todo_workspace(t: &TodoRecord, mode: TodoMode) -> SubWorkspace {
    match t.workspace.as_deref() {
        Some("inherit") => SubWorkspace::Inherit,
        Some("none") => SubWorkspace::None,
        Some(id) => SubWorkspace::Id(id.to_string()),
        None if mode == TodoMode::Serial => SubWorkspace::Inherit,
        None => SubWorkspace::None,
    }
}

/// Whether a serial Todo executes in a work session of its own instead of
/// a sub context of this session: it names another runtime, or another
/// workspace than the session's (integration TODO §3.5).
pub fn needs_own_session(t: &TodoRecord, cfg: &SessionConfig) -> bool {
    if t.mode != TodoMode::Serial {
        return false;
    }
    let other_runtime = t
        .runtime_id
        .as_deref()
        .is_some_and(|r| canonical_runtime(r) != cfg.runtime.runtime_id());
    let parent_workspace = cfg.workspace.as_ref().map(|w| w.workspace_id.as_str());
    let other_workspace = match t.workspace.as_deref() {
        None | Some("inherit") => false,
        Some("none") => parent_workspace.is_some(),
        Some(id) => parent_workspace != Some(id),
    };
    other_runtime || other_workspace
}

fn child_spec(list: &TodoList, t: &TodoRecord, attempt: u32, opts: &ExploreOptions, mode: TodoMode) -> SubSessionSpec {
    let mut task = t.task_text(list, attempt);
    let (note, behavior) = match mode {
        TodoMode::Explore => (
            "\nYou are an exploration sub session of a parent work session: do exactly this Todo, within its scope. \
Leads outside the scope go into your report for the parent, not into more work. \
Finish with a final report (is_end) that the parent's PLAN can review.\n",
            EXPLORE_BEHAVIOR,
        ),
        TodoMode::Serial => (
            "\nYou are a work session executing exactly this serial Todo of a parent session's plan, in the execution environment the Todo names (your own runtime and working directory). \
Do only this Todo, within its scope; leads outside it go into your report. \
Finish with a final report (is_end) with the evidence for each acceptance criterion; the parent's PLAN reviews it before the next Todo runs.\n",
            SERIAL_SESSION_BEHAVIOR,
        ),
    };
    task.push_str(note);
    SubSessionSpec {
        objective: t.task_text(list, attempt),
        msgs: vec![task],
        class: opts.class.clone(),
        behavior: Some(opts.behavior.clone().unwrap_or_else(|| behavior.to_string())),
        // The Todo's cmd_skills; without any the parent Work's are inherited.
        cmd_skills: (!t.skills.is_empty()).then(|| t.skills.clone()),
        workspace: todo_workspace(t, mode),
        runtime_id: t.runtime_id.clone(),
        report: ReportMode::Final,
        key: Some(explore_key(&t.todo_id, attempt)),
        ..Default::default()
    }
}

/// Dispatch the pending Todo `todo_id` of `parent` as an exploration sub
/// session: record the attempt (`dispatching`) → create the sub session →
/// bind it (`running`). The list stays locked meanwhile, so the host never
/// sees a dispatch half done; after a crash the attempt is found again by
/// its sub session id (bound if the session exists, withdrawn if not). A
/// limit (`max_sub_sessions`, the host's `--max-children`) refuses the
/// dispatch: the Todo stays pending with the refusal recorded.
pub async fn dispatch(
    agent: &dyn AgentStateClient,
    who: &str,
    channels: &dyn InputChannelFactory,
    parent: &SessionDir,
    todo_id: &str,
    opts: &ExploreOptions,
) -> Result<Dispatched> {
    dispatch_as(agent, who, channels, parent, todo_id, opts, TodoMode::Explore).await
}

/// Dispatch a serial Todo that executes in a work session of its own (see
/// [`needs_own_session`]): the attempt stays serial, the session waits for
/// the sub session and reviews its report. A running attempt of the Todo
/// is returned as it is (a hand-over redone after a crash).
pub async fn dispatch_serial(
    agent: &dyn AgentStateClient,
    who: &str,
    channels: &dyn InputChannelFactory,
    parent: &SessionDir,
    todo_id: &str,
    opts: &ExploreOptions,
) -> Result<Dispatched> {
    let store = store_of(parent);
    let list = store.load().map_err(todo_error)?;
    if let Some(a) = list.get(todo_id).and_then(|t| t.latest()) {
        if let (Some(sid), false, Some(DispatchState::Dispatched)) = (&a.child_sid, a.ended(), a.dispatch) {
            return Ok(Dispatched {
                todo_id: todo_id.to_string(),
                attempt: a.attempt,
                child_sid: sid.clone(),
                path: PathBuf::new(),
            });
        }
    }
    dispatch_as(agent, who, channels, parent, todo_id, opts, TodoMode::Serial).await
}

async fn dispatch_as(
    agent: &dyn AgentStateClient,
    who: &str,
    channels: &dyn InputChannelFactory,
    parent: &SessionDir,
    todo_id: &str,
    opts: &ExploreOptions,
    mode: TodoMode,
) -> Result<Dispatched> {
    let parent_sid = parent.sid().to_string();
    let store = store_of(parent);
    let lock = store.lock().map_err(todo_error)?;
    let list = lock.load().map_err(todo_error)?;
    let t = list
        .get(todo_id)
        .ok_or_else(|| OpenDanError::NotFound(format!("todo {todo_id}")))?;
    let attempt = match t.latest() {
        Some(a) if a.dispatch == Some(DispatchState::Dispatching) => a.attempt,
        _ => t.next_attempt(),
    };
    let sub = child_spec(&list, t, attempt, opts, mode);
    let child_sid = sub_session_id(agent, who, &parent_sid, &sub)?;
    let now = crate::now_ms();
    let n = lock
        .update(|l| match mode {
            TodoMode::Explore => l.begin_explore(todo_id, &child_sid, now),
            TodoMode::Serial => l.begin_serial_session(todo_id, &child_sid, now),
        })
        .map_err(todo_error)?;
    crate::fault::point("todo:after_dispatch_record");
    let refuse = |reason: String| -> Result<Dispatched> {
        lock.update(|l| l.withdraw_dispatch(todo_id, n, &reason, crate::now_ms()))
            .map_err(todo_error)?;
        Err(OpenDanError::InvalidArgument(reason))
    };
    if agent.sessions().lookup(&child_sid).await?.is_none() {
        if let Some(max) = opts.max_children {
            let running = agent
                .sessions()
                .children_of(&[parent_sid.clone()])
                .await?
                .into_iter()
                .filter(|e| e.status.run_state != RunState::Finished)
                .count();
            if running >= max.max(1) {
                return refuse(format!(
                    "the host drives at most {max} sub sessions at once (--max-children) and {running} are running: \
end your step with a progress report to wait for one, then dispatch {todo_id} again (`agent-session todo dispatch {todo_id}`)"
                ));
            }
        }
    }
    let created = match create_sub_session(agent, who, &parent_sid, sub, channels).await {
        Ok(sd) => sd,
        Err(OpenDanError::InvalidArgument(reason)) => {
            return refuse(format!("{reason}; {todo_id} stays pending (`agent-session todo dispatch {todo_id}` again later)"))
        }
        Err(e) => {
            // Whether the session came to exist decides: bound if it did,
            // withdrawn if not (a retry creates it then).
            if agent.sessions().lookup(&child_sid).await.ok().flatten().is_none() {
                let _ = lock.update(|l| l.withdraw_dispatch(todo_id, n, &e.to_string(), crate::now_ms()));
            }
            return Err(e);
        }
    };
    crate::fault::point("todo:after_child_created");
    let target = created
        .config()
        .map(|c| AttemptTarget {
            runtime_id: Some(c.runtime.runtime_id().to_string()),
            workspace_id: c.workspace.map(|w| w.workspace_id),
        })
        .unwrap_or_default();
    lock.update(|l| l.bind_explore(todo_id, n, crate::now_ms(), target))
        .map_err(todo_error)?;
    crate::fault::point("todo:after_bind");
    Ok(Dispatched {
        todo_id: todo_id.to_string(),
        attempt: n,
        child_sid,
        path: created.path().to_path_buf(),
    })
}

/// What an exploration attempt is, from its sub session's committed state.
pub fn observe(entry: &RegistryEntry, state: Option<&SessionState>) -> ChildObservation {
    let st = &entry.status;
    let report = state.and_then(|s| s.final_report.as_ref());
    let error = state
        .and_then(|s| s.last_error.as_ref())
        .and_then(|e| e.get("message").and_then(Value::as_str).map(str::to_string));
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
            .or_else(|| Some(one_line(&st.one_line_status)).filter(|s| !s.is_empty()));
        return ChildObservation {
            status,
            summary,
            report: report.map(|r| ReportRef {
                session_id: entry.session_id.clone(),
                run_id: None,
                report_id: r.id.clone(),
            }),
            artifacts: report.map(artifact_refs).unwrap_or_default(),
            error: (status != TodoStatus::Completed).then_some(error).flatten(),
        };
    }
    let blocked = st.pending_decision.is_some()
        || (st.run_state == RunState::Waiting && st.waiting_for == Some(WaitingKind::Input));
    ChildObservation {
        status: if blocked { TodoStatus::Blocked } else { TodoStatus::Running },
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

/// Keep a report submission beyond its run (`<state>/reports/<id>/submission.json`).
pub fn save_submission(sd: &SessionDir, lease: &crate::lock::Lease, r: &ReportSubmission) -> Result<()> {
    let path = sd.state_dir().join(REPORTS_DIR).join(&r.id).join("submission.json");
    lease.fenced(|| crate::fsutil::atomic_replace_json(&path, r))
}

/// A report as the PLAN reads it (`todo show <id> --report`).
#[derive(Debug, Clone, Serialize)]
pub struct ReportView {
    pub report_ref: ReportRef,
    pub report: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ReportArtifact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Where the full sub session can be read (explorations).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_dir: Option<PathBuf>,
}

/// The report an attempt points to, read from where it is kept.
pub async fn read_report(
    agent: &dyn AgentStateClient,
    sd: &SessionDir,
    attempt: &TodoAttempt,
) -> Result<Option<ReportView>> {
    let Some(r) = attempt.report.clone() else {
        return Ok(None);
    };
    if r.session_id != sd.sid() {
        let Some(e) = agent.sessions().lookup(&r.session_id).await? else {
            return Ok(None);
        };
        let child = SessionDir::open(&e.location)?;
        let st = child.state()?;
        return Ok(st.final_report.filter(|s| s.id == r.report_id).map(|s| ReportView {
            report_ref: r,
            report: s.report,
            artifacts: s.artifacts,
            result: s.result,
            session_dir: Some(child.path().to_path_buf()),
        }));
    }
    let path = sd.state_dir().join(REPORTS_DIR).join(&r.report_id).join("submission.json");
    let Some(s) = crate::fsutil::read_json_opt::<ReportSubmission>(&path)? else {
        return Ok(None);
    };
    Ok(Some(ReportView {
        report_ref: r,
        report: s.report,
        artifacts: s.artifacts,
        result: s.result,
        session_dir: None,
    }))
}

/// Write what the list's executions committed back into it (TodoList §2.1):
/// an exploration's attempt follows its sub session's registry state — never
/// the sub session's own word —, a dispatch that died before its sub session
/// existed is withdrawn (the list lock is free, so no dispatch is in
/// progress), and a serial attempt whose call is no longer on the session's
/// `process_stack` (the session or Turn was stopped) ended stopped.
/// `state`: the session's committed state. `true`: the list changed.
pub async fn sync(agent: &dyn AgentStateClient, sd: &SessionDir, state: &SessionState) -> Result<bool> {
    let store = store_of(sd);
    let list = store.load().map_err(todo_error)?;
    let open: Vec<(String, u32, Option<String>, TodoMode)> = list
        .todos
        .iter()
        .filter_map(|t| {
            let a = t.latest().filter(|a| !a.ended())?;
            Some((t.todo_id.clone(), a.attempt, a.child_sid.clone(), a.mode))
        })
        .collect();
    if open.is_empty() {
        return Ok(false);
    }
    let bound: Vec<TodoBinding> = state
        .process_stack
        .iter()
        .filter_map(|f| f.call.as_ref().and_then(|c| c.todo.clone()))
        .collect();
    let mut seen = Vec::new();
    for (todo_id, attempt, child, mode) in &open {
        match (mode, child) {
            (_, Some(sid)) => {
                let entry = agent.sessions().lookup(sid).await?;
                let child_state = entry
                    .as_ref()
                    .and_then(|e| SessionDir::open(&e.location).ok())
                    .and_then(|d| d.state().ok());
                seen.push((todo_id.clone(), *attempt, entry.map(|e| observe(&e, child_state.as_ref()))));
            }
            _ if bound.iter().any(|b| &b.todo_id == todo_id && b.attempt == *attempt) => {}
            _ => seen.push((
                todo_id.clone(),
                *attempt,
                Some(ChildObservation {
                    status: TodoStatus::Stopped,
                    summary: None,
                    report: None,
                    artifacts: Vec::new(),
                    error: Some(
                        "the sub context ended without returning a result (the session or its Turn was stopped)".into(),
                    ),
                }),
            )),
        }
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
                    Some(o) if a.mode == TodoMode::Serial => {
                        changed |= l.end_serial(&todo_id, attempt, "", o, now)?;
                    }
                    Some(o) => changed |= l.observe_child(&todo_id, attempt, o, now)?,
                    None if a.dispatch == Some(DispatchState::Dispatching) => {
                        l.withdraw_dispatch(
                            &todo_id,
                            attempt,
                            "the dispatch was interrupted before its sub session existed",
                            now,
                        )?;
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
                                error: Some("its sub session is not registered".into()),
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
