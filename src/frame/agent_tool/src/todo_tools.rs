//! Session TodoList — the records and their rules (`doc/opendan/Agent TodoList.md`).
//!
//! The TodoList of a Work Session is the PLAN's persistent decision basis:
//! what is to be done, how each attempt ran — serially, as a sub context of
//! the session (`serial`), or as an exploration sub session (`explore`) —
//! what it reported and how the PLAN reviewed it. This module only holds the
//! records, their state rules and the file store. When they change is
//! decided by the session host (libopendan) and its `agent-session todo`
//! commands; there is no prompt-visible tool here.
//!
//! One writer: the session's host and the tool processes of its runs, under
//! the session's lease. The file lock only orders their writes; every
//! mutation is a read-modify-write of the whole list, published by atomic
//! replace.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::AgentToolError;

pub const TODO_LIST_SCHEMA: &str = "opendan.todo_list/1";
/// File name of a session's list (in its state directory).
pub const TODO_LIST_FILE: &str = "todos.json";
const TODO_LOCK_FILE: &str = "todos.lock";
/// cmd_skills a Todo names for the sub session that executes it (the
/// session limit of OpenDAN).
pub const MAX_SKILLS_PER_TODO: usize = 8;
/// Characters of the one-line summary of an attempt.
pub const SUMMARY_CHARS: usize = 160;

// ---------------------------------------------------------------------------
// Records
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    Running,
    /// Not terminal, but never picked as the serial current Todo: the PLAN
    /// handles it explicitly (an exploration that waits for an answer).
    Blocked,
    Completed,
    Failed,
    Timeout,
    Stopped,
    Cancelled,
}

impl TodoStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            TodoStatus::Completed
                | TodoStatus::Failed
                | TodoStatus::Timeout
                | TodoStatus::Stopped
                | TodoStatus::Cancelled
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            TodoStatus::Pending => "pending",
            TodoStatus::Running => "running",
            TodoStatus::Blocked => "blocked",
            TodoStatus::Completed => "completed",
            TodoStatus::Failed => "failed",
            TodoStatus::Timeout => "timeout",
            TodoStatus::Stopped => "stopped",
            TodoStatus::Cancelled => "cancelled",
        }
    }
}

/// How an attempt runs.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TodoMode {
    /// A sub context of the session (`next_behavior = do`): the caller's run
    /// is suspended until it returns.
    #[default]
    Serial,
    /// An independent sub session; several run in parallel.
    Explore,
}

impl TodoMode {
    pub fn as_str(self) -> &'static str {
        match self {
            TodoMode::Serial => "serial",
            TodoMode::Explore => "explore",
        }
    }
}

/// Dispatch of an exploration attempt (crash recovery: a `dispatching`
/// attempt is reconciled against the registry by its sub session id).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DispatchState {
    Dispatching,
    Dispatched,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewVerdict {
    Accept,
    Reject,
    NeedEvidence,
}

impl ReviewVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            ReviewVerdict::Accept => "accepted",
            ReviewVerdict::Reject => "rejected",
            ReviewVerdict::NeedEvidence => "need_evidence",
        }
    }
}

/// The run and call a serial attempt was admitted for (the hand-over step
/// of the caller): its idempotency key.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct CallRef {
    pub run_id: String,
    pub call_id: String,
}

/// Where the report of an attempt is: an exploration's final report in its
/// sub session, a serial attempt's report in the session's report store.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ReportRef {
    /// The session holding the report (the sub session of an exploration,
    /// the session itself for a serial attempt).
    pub session_id: String,
    /// The run that submitted it (serial attempts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    pub report_id: String,
}

impl ReportRef {
    pub fn display(&self) -> String {
        match &self.run_id {
            Some(run) => format!("{}/{run}#{}", self.session_id, self.report_id),
            None => format!("{}#{}", self.session_id, self.report_id),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct TodoAttempt {
    pub attempt: u32,
    pub mode: TodoMode,
    /// Serial: the caller's hand-over the attempt was admitted for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller: Option<CallRef>,
    /// Serial: the sub context's run (known once it returned).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// Explore, or a serial Todo on another runtime / workspace: the sub
    /// session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_sid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatch: Option<DispatchState>,
    /// Where the attempt executes, as resolved at its start (the Todo keeps
    /// the intent; a retry resolves again).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    /// `running | blocked | completed | failed | timeout | stopped`.
    pub status: TodoStatus,
    pub started_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// One line about the result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<ReportRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<String>,
    /// Why this attempt was made (a retry's reason).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl TodoAttempt {
    pub fn ended(&self) -> bool {
        self.status.is_terminal()
    }
}

/// The PLAN's judgement of one attempt's result. Execution success and
/// acceptance are recorded apart.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct TodoReview {
    pub attempt: u32,
    /// The report version that was read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_id: Option<String>,
    pub verdict: ReviewVerdict,
    pub reason: String,
    /// Todos created because of this review.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub followups: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    pub at_ms: u64,
}

/// A dispatch the concurrency limits refused (observable; the Todo stays
/// pending and can be dispatched again).
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct DispatchRefusal {
    pub at_ms: u64,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct TodoRecord {
    /// Stable, never reused (`T01`, `T02`, … from the list's counter).
    pub todo_id: String,
    /// Creation order.
    pub seq: u64,
    /// Revision of this record (+1 on every change).
    pub rev: u64,
    pub status: TodoStatus,
    pub mode: TodoMode,
    pub title: String,
    pub content: String,
    /// Acceptance criteria.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accept: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// Evidence already known.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
    /// Todos whose result must be accepted before this one may run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub after: Vec<String>,
    /// Dispatch batch (explorations sent out together).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch: Option<String>,
    /// Where the Todo must execute: a runtime id (`None`: the session's
    /// own), and `inherit | none | <workspace id>` (`None`: a serial Todo
    /// inherits the session's workspace, an exploration takes none). A
    /// workspace decides the runtime; both named and in conflict is refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attempts: Vec<TodoAttempt>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reviews: Vec<TodoReview>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refusals: Vec<DispatchRefusal>,
    /// Pending retry: the reason for the next attempt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel_reason: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

/// Where the PLAN's judgement of a Todo stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    /// No attempt ended yet.
    None,
    /// The latest attempt ended and was not reviewed.
    Pending,
    Accepted,
    Rejected,
    NeedEvidence,
}

impl ReviewState {
    pub fn as_str(self) -> &'static str {
        match self {
            ReviewState::None => "none",
            ReviewState::Pending => "pending",
            ReviewState::Accepted => "accepted",
            ReviewState::Rejected => "rejected",
            ReviewState::NeedEvidence => "need_evidence",
        }
    }
}

impl TodoRecord {
    pub fn latest(&self) -> Option<&TodoAttempt> {
        self.attempts.last()
    }

    pub fn attempt(&self, n: u32) -> Option<&TodoAttempt> {
        self.attempts.iter().find(|a| a.attempt == n)
    }

    fn attempt_mut(&mut self, n: u32) -> Option<&mut TodoAttempt> {
        self.attempts.iter_mut().find(|a| a.attempt == n)
    }

    pub fn next_attempt(&self) -> u32 {
        self.attempts.iter().map(|a| a.attempt).max().unwrap_or(0) + 1
    }

    pub fn review_of(&self, attempt: u32) -> Option<&TodoReview> {
        self.reviews.iter().rev().find(|r| r.attempt == attempt)
    }

    pub fn review_state(&self) -> ReviewState {
        match self.latest().filter(|a| a.ended()) {
            None => ReviewState::None,
            Some(a) => match self.review_of(a.attempt).map(|r| r.verdict) {
                None => ReviewState::Pending,
                Some(ReviewVerdict::Accept) => ReviewState::Accepted,
                Some(ReviewVerdict::Reject) => ReviewState::Rejected,
                Some(ReviewVerdict::NeedEvidence) => ReviewState::NeedEvidence,
            },
        }
    }

    /// The result of the latest attempt was accepted (the basis for the
    /// work that depends on it).
    pub fn accepted(&self) -> bool {
        self.retry.is_none() && self.review_state() == ReviewState::Accepted
    }

    /// An attempt is executing, or being dispatched.
    pub fn in_flight(&self) -> bool {
        self.latest().is_some_and(|a| !a.ended())
    }

    /// The status follows the records: cancelled, a pending retry or no
    /// attempt yet → pending, otherwise the latest attempt's status.
    fn refresh(&mut self) {
        self.status = if self.cancel_reason.is_some() {
            TodoStatus::Cancelled
        } else if self.retry.is_some() && !self.in_flight() {
            TodoStatus::Pending
        } else {
            match self.latest() {
                None => TodoStatus::Pending,
                Some(a) => a.status,
            }
        };
    }

    fn touch(&mut self, now_ms: u64) {
        self.refresh();
        self.rev += 1;
        self.updated_at_ms = now_ms;
    }

    /// Whether the Todo still needs the PLAN before the session may end:
    /// `None` when it converged (cancelled, or its latest result was
    /// reviewed), else why not.
    pub fn unconverged(&self) -> Option<String> {
        if self.status == TodoStatus::Cancelled {
            return None;
        }
        if self.in_flight() {
            return Some(format!("{} is {}", self.todo_id, self.status.as_str()));
        }
        if self.retry.is_some() || self.attempts.is_empty() {
            return Some(format!(
                "{} is pending (execute it, or cancel it with a reason)",
                self.todo_id
            ));
        }
        match self.review_state() {
            ReviewState::Pending => Some(format!(
                "the result of {} (attempt {}) is not reviewed",
                self.todo_id,
                self.latest().map(|a| a.attempt).unwrap_or(0)
            )),
            ReviewState::NeedEvidence => Some(format!(
                "{} needs more evidence (retry it, or review it again after a follow-up)",
                self.todo_id
            )),
            _ => None,
        }
    }

    /// The complete task of the Todo as an executor receives it: body,
    /// acceptance criteria, scope, evidence, skills and the accepted results
    /// it builds on.
    pub fn task_text(&self, list: &TodoList, attempt: u32) -> String {
        let mut s = format!(
            "Todo {} (attempt {attempt}, {}): {}\n\n{}\n",
            self.todo_id,
            self.mode.as_str(),
            self.title,
            self.content.trim()
        );
        if !self.accept.is_empty() {
            s.push_str("\nAcceptance criteria:\n");
            for a in &self.accept {
                s.push_str(&format!("- {}\n", a.trim()));
            }
        }
        if let Some(scope) = self.scope.as_deref().filter(|t| !t.trim().is_empty()) {
            s.push_str(&format!("\nScope: {}\n", scope.trim()));
        }
        if !self.evidence.is_empty() {
            s.push_str("\nKnown evidence:\n");
            for e in &self.evidence {
                s.push_str(&format!("- {}\n", e.trim()));
            }
        }
        if !self.skills.is_empty() {
            s.push_str(&format!("\ncmd_skills: {}\n", self.skills.join(", ")));
        }
        if self.runtime_id.is_some() || self.workspace.is_some() {
            s.push_str(&format!(
                "\nExecution environment: runtime {}, workspace {}\n",
                self.runtime_id.as_deref().unwrap_or("(the session's)"),
                self.workspace.as_deref().unwrap_or("(the default for this mode)")
            ));
        }
        let deps: Vec<&TodoRecord> = self.after.iter().filter_map(|id| list.get(id)).collect();
        if !deps.is_empty() {
            s.push_str("\nBuilds on these reviewed results:\n");
            for d in deps {
                let a = d.latest();
                s.push_str(&format!(
                    "- {} {}: {}{}\n",
                    d.todo_id,
                    d.title,
                    a.and_then(|a| a.summary.clone()).unwrap_or_default(),
                    a.and_then(|a| a.report.as_ref())
                        .map(|r| format!(" (report {})", r.display()))
                        .unwrap_or_default()
                ));
            }
        }
        if let Some(reason) = self.attempt(attempt).and_then(|a| a.reason.clone()) {
            s.push_str(&format!("\nWhy this attempt: {reason}\n"));
        }
        s
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct TodoList {
    pub schema: String,
    /// Counter of todo ids: never reused, also after cancellations.
    pub next_seq: u64,
    /// Revision of the list (+1 on every change).
    pub rev: u64,
    #[serde(default)]
    pub todos: Vec<TodoRecord>,
}

impl Default for TodoList {
    fn default() -> Self {
        Self {
            schema: TODO_LIST_SCHEMA.to_string(),
            next_seq: 1,
            rev: 0,
            todos: Vec::new(),
        }
    }
}

/// A Todo to add.
#[derive(Clone, Debug, Default)]
pub struct NewTodo {
    pub title: String,
    pub content: String,
    pub mode: TodoMode,
    pub accept: Vec<String>,
    pub scope: Option<String>,
    pub evidence: Vec<String>,
    pub skills: Vec<String>,
    pub after: Vec<String>,
    pub batch: Option<String>,
    pub runtime_id: Option<String>,
    pub workspace: Option<String>,
}

/// Where an attempt was resolved to execute.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AttemptTarget {
    pub runtime_id: Option<String>,
    pub workspace_id: Option<String>,
}

/// Which Todo `next_behavior = do` would execute now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SerialPick {
    /// The first pending serial Todo whose prerequisites are accepted.
    Ready(String),
    /// Results that came back and were not reviewed: review them first.
    Unreviewed(Vec<String>),
    /// Pending serial Todos wait for prerequisites not accepted yet.
    Waiting(Vec<(String, Vec<String>)>),
    /// Nothing serial to execute.
    None,
}

/// What the session host observed of an exploration's sub session.
#[derive(Clone, Debug, PartialEq)]
pub struct ChildObservation {
    pub status: TodoStatus,
    pub summary: Option<String>,
    pub report: Option<ReportRef>,
    pub artifacts: Vec<String>,
    pub error: Option<String>,
}

fn invalid(msg: impl Into<String>) -> AgentToolError {
    AgentToolError::InvalidArgs(msg.into())
}

/// First line of `text`, at most [`SUMMARY_CHARS`] characters.
pub fn one_line(text: &str) -> String {
    let first = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .unwrap_or("")
        .trim_start_matches(['-', '*', ' ']);
    let mut out: String = first.chars().take(SUMMARY_CHARS).collect();
    if first.chars().count() > SUMMARY_CHARS {
        out.push('…');
    }
    out
}

impl TodoList {
    pub fn get(&self, id: &str) -> Option<&TodoRecord> {
        self.todos.iter().find(|t| t.todo_id == id)
    }

    fn get_mut(&mut self, id: &str) -> Result<&mut TodoRecord, AgentToolError> {
        self.todos
            .iter_mut()
            .find(|t| t.todo_id == id)
            .ok_or_else(|| AgentToolError::NotFound(format!("todo `{id}` not found")))
    }

    fn bump(&mut self) {
        self.rev += 1;
    }

    pub fn add(&mut self, new: NewTodo, now_ms: u64) -> Result<String, AgentToolError> {
        let title = new.title.trim().to_string();
        if title.is_empty() {
            return Err(invalid("a todo needs a non-empty title"));
        }
        let content = new.content.trim().to_string();
        if content.is_empty() {
            return Err(invalid(
                "a todo needs --content: what to do, in which scope, with what evidence",
            ));
        }
        if new.skills.len() > MAX_SKILLS_PER_TODO {
            return Err(invalid(format!(
                "at most {MAX_SKILLS_PER_TODO} cmd_skills per todo (got {})",
                new.skills.len()
            )));
        }
        for dep in &new.after {
            if self.get(dep).is_none() {
                return Err(invalid(format!("--after {dep}: no such todo")));
            }
        }
        let runtime_id = new
            .runtime_id
            .map(|r| r.trim().to_string())
            .filter(|r| !r.is_empty() && r != "inherit");
        let workspace = new.workspace.map(|w| w.trim().to_string()).filter(|w| !w.is_empty());
        if let Some(w) = &workspace {
            if w != "inherit" && w != "none" && !w.starts_with("ws-") {
                return Err(invalid(format!(
                    "--workspace {w}: takes inherit | none | <workspace id>"
                )));
            }
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        let todo_id = format!("T{seq:02}");
        let mut record = TodoRecord {
            todo_id: todo_id.clone(),
            seq,
            rev: 0,
            status: TodoStatus::Pending,
            mode: new.mode,
            title,
            content,
            accept: clean(new.accept),
            scope: new.scope.filter(|s| !s.trim().is_empty()),
            evidence: clean(new.evidence),
            skills: clean(new.skills),
            after: new.after,
            batch: new.batch,
            runtime_id,
            workspace,
            attempts: Vec::new(),
            reviews: Vec::new(),
            refusals: Vec::new(),
            retry: None,
            cancel_reason: None,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        };
        record.touch(now_ms);
        self.todos.push(record);
        self.bump();
        Ok(todo_id)
    }

    /// Ended attempts nobody reviewed yet (the latest attempt of each Todo;
    /// a pending retry is a judgement of its own).
    pub fn unreviewed(&self) -> Vec<String> {
        self.todos
            .iter()
            .filter(|t| {
                t.status != TodoStatus::Cancelled
                    && t.retry.is_none()
                    && t.review_state() == ReviewState::Pending
            })
            .map(|t| t.todo_id.clone())
            .collect()
    }

    /// The serial Todo `next_behavior = do` executes: unreviewed results
    /// come first; then the first pending serial Todo (never one bound to
    /// an exploration, never a blocked one) whose prerequisites are accepted.
    pub fn serial_pick(&self) -> SerialPick {
        let unreviewed = self.unreviewed();
        if !unreviewed.is_empty() {
            return SerialPick::Unreviewed(unreviewed);
        }
        let mut waiting = Vec::new();
        for t in &self.todos {
            if t.mode != TodoMode::Serial || t.status != TodoStatus::Pending {
                continue;
            }
            let missing: Vec<String> = t
                .after
                .iter()
                .filter(|d| !self.get(d).is_some_and(TodoRecord::accepted))
                .cloned()
                .collect();
            if missing.is_empty() {
                return SerialPick::Ready(t.todo_id.clone());
            }
            waiting.push((t.todo_id.clone(), missing));
        }
        if waiting.is_empty() {
            SerialPick::None
        } else {
            SerialPick::Waiting(waiting)
        }
    }

    /// Exploration attempts in flight (dispatched or being dispatched).
    pub fn explorations_in_flight(&self) -> Vec<(String, String)> {
        self.todos
            .iter()
            .filter_map(|t| {
                let a = t.latest()?;
                (a.mode == TodoMode::Explore && !a.ended())
                    .then(|| (t.todo_id.clone(), a.child_sid.clone().unwrap_or_default()))
            })
            .collect()
    }

    /// Admit `todo_id` as the serial attempt of `caller` (idempotent: the
    /// same caller gets the same attempt). Returns the attempt number.
    pub fn begin_serial(
        &mut self,
        todo_id: &str,
        caller: CallRef,
        now_ms: u64,
    ) -> Result<u32, AgentToolError> {
        let t = self.get_mut(todo_id)?;
        if let Some(a) = t.attempts.iter().find(|a| a.caller.as_ref() == Some(&caller)) {
            return Ok(a.attempt);
        }
        if t.status != TodoStatus::Pending || t.mode != TodoMode::Serial {
            return Err(invalid(format!(
                "todo {todo_id} is {} ({}); only a pending serial todo is executed",
                t.status.as_str(),
                t.mode.as_str()
            )));
        }
        let attempt = t.next_attempt();
        t.attempts.push(TodoAttempt {
            attempt,
            mode: TodoMode::Serial,
            caller: Some(caller),
            run_id: None,
            child_sid: None,
            dispatch: None,
            runtime_id: None,
            workspace_id: None,
            status: TodoStatus::Running,
            started_at_ms: now_ms,
            ended_at_ms: None,
            error: None,
            summary: None,
            report: None,
            artifacts: Vec::new(),
            reason: t.retry.take(),
        });
        t.touch(now_ms);
        self.bump();
        Ok(attempt)
    }

    /// The end of a serial attempt (idempotent; an attempt that ended keeps
    /// its first result).
    pub fn end_serial(
        &mut self,
        todo_id: &str,
        attempt: u32,
        run_id: &str,
        result: ChildObservation,
        now_ms: u64,
    ) -> Result<bool, AgentToolError> {
        let t = self.get_mut(todo_id)?;
        let Some(a) = t.attempt_mut(attempt) else {
            return Err(AgentToolError::NotFound(format!(
                "todo {todo_id} has no attempt {attempt}"
            )));
        };
        if a.ended() {
            return Ok(false);
        }
        if !run_id.is_empty() {
            a.run_id = Some(run_id.to_string());
        }
        apply_observation(a, result, now_ms);
        t.touch(now_ms);
        self.bump();
        Ok(true)
    }

    /// Record the dispatch of an exploration attempt before its sub session
    /// is created. A `dispatching` attempt of the same sub session is reused
    /// (a retried dispatch after a crash).
    pub fn begin_explore(
        &mut self,
        todo_id: &str,
        child_sid: &str,
        now_ms: u64,
    ) -> Result<u32, AgentToolError> {
        self.begin_child(todo_id, child_sid, TodoMode::Explore, now_ms)
    }

    /// Record the dispatch of a serial Todo that executes in its own work
    /// session (another runtime or workspace than the session's): the Todo
    /// stays serial — the session waits for it and reviews it like a sub
    /// context's result.
    pub fn begin_serial_session(
        &mut self,
        todo_id: &str,
        child_sid: &str,
        now_ms: u64,
    ) -> Result<u32, AgentToolError> {
        self.begin_child(todo_id, child_sid, TodoMode::Serial, now_ms)
    }

    fn begin_child(
        &mut self,
        todo_id: &str,
        child_sid: &str,
        mode: TodoMode,
        now_ms: u64,
    ) -> Result<u32, AgentToolError> {
        let t = self.get_mut(todo_id)?;
        if let Some(a) = t
            .attempts
            .iter()
            .find(|a| a.child_sid.as_deref() == Some(child_sid))
        {
            return Ok(a.attempt);
        }
        if t.status != TodoStatus::Pending {
            return Err(invalid(format!(
                "todo {todo_id} is {}; only a pending todo is dispatched (retry it first)",
                t.status.as_str()
            )));
        }
        if mode == TodoMode::Serial && t.mode != TodoMode::Serial {
            return Err(invalid(format!(
                "todo {todo_id} is an exploration; dispatch it with `todo dispatch`"
            )));
        }
        t.mode = mode;
        let attempt = t.next_attempt();
        t.attempts.push(TodoAttempt {
            attempt,
            mode,
            caller: None,
            run_id: None,
            child_sid: Some(child_sid.to_string()),
            dispatch: Some(DispatchState::Dispatching),
            runtime_id: t.runtime_id.clone(),
            workspace_id: t.workspace.clone().filter(|w| w.starts_with("ws-")),
            status: TodoStatus::Running,
            started_at_ms: now_ms,
            ended_at_ms: None,
            error: None,
            summary: None,
            report: None,
            artifacts: Vec::new(),
            reason: t.retry.take(),
        });
        t.touch(now_ms);
        self.bump();
        Ok(attempt)
    }

    /// The sub session of a dispatched attempt exists: bound, with where it
    /// was resolved to execute.
    pub fn bind_explore(
        &mut self,
        todo_id: &str,
        attempt: u32,
        now_ms: u64,
        target: AttemptTarget,
    ) -> Result<bool, AgentToolError> {
        let t = self.get_mut(todo_id)?;
        let a = t
            .attempt_mut(attempt)
            .ok_or_else(|| AgentToolError::NotFound(format!("todo {todo_id} has no attempt {attempt}")))?;
        if a.dispatch == Some(DispatchState::Dispatched) {
            return Ok(false);
        }
        a.dispatch = Some(DispatchState::Dispatched);
        if target.runtime_id.is_some() {
            a.runtime_id = target.runtime_id;
        }
        if target.workspace_id.is_some() {
            a.workspace_id = target.workspace_id;
        }
        t.touch(now_ms);
        self.bump();
        Ok(true)
    }

    /// A dispatch that did not create its sub session (refused by a limit,
    /// or interrupted before the creation): the attempt is withdrawn, the
    /// Todo is pending again with the refusal recorded.
    pub fn withdraw_dispatch(
        &mut self,
        todo_id: &str,
        attempt: u32,
        reason: &str,
        now_ms: u64,
    ) -> Result<(), AgentToolError> {
        let t = self.get_mut(todo_id)?;
        let pos = t
            .attempts
            .iter()
            .position(|a| a.attempt == attempt && a.dispatch == Some(DispatchState::Dispatching));
        if let Some(pos) = pos {
            let a = t.attempts.remove(pos);
            if a.reason.is_some() {
                t.retry = a.reason;
            }
        }
        t.refusals.push(DispatchRefusal {
            at_ms: now_ms,
            reason: reason.to_string(),
        });
        t.touch(now_ms);
        self.bump();
        Ok(())
    }

    /// What the host observed of an exploration's sub session (idempotent;
    /// an ended attempt keeps its result, and only its own attempt changes —
    /// a late result never reaches a newer attempt).
    pub fn observe_child(
        &mut self,
        todo_id: &str,
        attempt: u32,
        seen: ChildObservation,
        now_ms: u64,
    ) -> Result<bool, AgentToolError> {
        let t = self.get_mut(todo_id)?;
        let Some(a) = t.attempt_mut(attempt) else {
            return Ok(false);
        };
        if a.ended() {
            return Ok(false);
        }
        if a.dispatch == Some(DispatchState::Dispatching) {
            a.dispatch = Some(DispatchState::Dispatched);
        }
        if !seen.status.is_terminal() {
            if a.status == seen.status {
                return Ok(false);
            }
            a.status = seen.status;
        } else {
            apply_observation(a, seen, now_ms);
        }
        t.touch(now_ms);
        self.bump();
        Ok(true)
    }

    /// The PLAN's review of the latest attempt. Replaying the same review is
    /// a no-op; a different one for an attempt already reviewed is refused.
    pub fn review(
        &mut self,
        todo_id: &str,
        verdict: ReviewVerdict,
        reason: &str,
        followups: Vec<String>,
        by: Option<String>,
        now_ms: u64,
    ) -> Result<(u32, bool), AgentToolError> {
        if reason.trim().is_empty() {
            return Err(invalid("a review needs --reason: what the report shows and why"));
        }
        for f in &followups {
            if self.get(f).is_none() {
                return Err(invalid(format!("--followup {f}: no such todo")));
            }
        }
        let t = self.get_mut(todo_id)?;
        let Some(a) = t.latest().filter(|a| a.ended()).cloned() else {
            return Err(invalid(format!(
                "todo {todo_id} has no result to review yet ({})",
                t.status.as_str()
            )));
        };
        if let Some(old) = t.review_of(a.attempt) {
            if old.verdict == verdict && old.reason == reason.trim() {
                return Ok((a.attempt, false));
            }
            return Err(invalid(format!(
                "attempt {} of {todo_id} was already reviewed ({}); retry it or add a follow-up todo",
                a.attempt,
                old.verdict.as_str()
            )));
        }
        t.reviews.push(TodoReview {
            attempt: a.attempt,
            report_id: a.report.as_ref().map(|r| r.report_id.clone()),
            verdict,
            reason: reason.trim().to_string(),
            followups,
            by,
            at_ms: now_ms,
        });
        t.touch(now_ms);
        self.bump();
        Ok((a.attempt, true))
    }

    /// Cancel a Todo that is not executing; its history stays.
    pub fn cancel(&mut self, todo_id: &str, reason: &str, now_ms: u64) -> Result<(), AgentToolError> {
        if reason.trim().is_empty() {
            return Err(invalid("cancel needs --reason"));
        }
        let t = self.get_mut(todo_id)?;
        if t.status != TodoStatus::Pending {
            return Err(invalid(format!(
                "todo {todo_id} is {}: only a todo that is not dispatched can be cancelled",
                t.status.as_str()
            )));
        }
        t.cancel_reason = Some(reason.trim().to_string());
        t.retry = None;
        t.touch(now_ms);
        self.bump();
        Ok(())
    }

    /// Cancel every pending Todo (nothing is deleted, no id is reused).
    pub fn clean(&mut self, reason: &str, now_ms: u64) -> Result<Vec<String>, AgentToolError> {
        let ids: Vec<String> = self
            .todos
            .iter()
            .filter(|t| t.status == TodoStatus::Pending)
            .map(|t| t.todo_id.clone())
            .collect();
        for id in &ids {
            self.cancel(id, reason, now_ms)?;
        }
        Ok(ids)
    }

    /// Ask for a new attempt of a Todo whose latest attempt ended; the old
    /// attempts and their reports stay.
    pub fn retry(&mut self, todo_id: &str, reason: &str, now_ms: u64) -> Result<(), AgentToolError> {
        if reason.trim().is_empty() {
            return Err(invalid("retry needs --reason: what the new attempt must do differently"));
        }
        let t = self.get_mut(todo_id)?;
        if t.attempts.is_empty() || t.in_flight() || t.status == TodoStatus::Cancelled {
            return Err(invalid(format!(
                "todo {todo_id} is {}: only a todo whose attempt ended can be retried",
                t.status.as_str()
            )));
        }
        if t.retry.is_some() {
            return Ok(());
        }
        t.retry = Some(reason.trim().to_string());
        t.touch(now_ms);
        self.bump();
        Ok(())
    }

    /// Why the session may not finish yet (empty: every Todo converged).
    pub fn unconverged(&self) -> Vec<String> {
        self.todos.iter().filter_map(TodoRecord::unconverged).collect()
    }

    /// The compact view the PLAN decides on (§2.0): no execution detail.
    pub fn render_view(&self) -> String {
        let mut s = format!("<todo_list rev=\"{}\">\n", self.rev);
        for t in &self.todos {
            let a = t.latest();
            let mut attrs = format!(
                "id=\"{}\" mode=\"{}\" status=\"{}\"",
                t.todo_id,
                t.mode.as_str(),
                t.status.as_str()
            );
            if let Some(a) = a {
                attrs.push_str(&format!(" attempt=\"{}\"", a.attempt));
                if let Some(c) = &a.child_sid {
                    attrs.push_str(&format!(" child_sid=\"{}\"", llm_context::escape_xml_attr(c)));
                }
                if let Some(r) = &a.report {
                    attrs.push_str(&format!(
                        " report_ref=\"{}\"",
                        llm_context::escape_xml_attr(&r.display())
                    ));
                }
            }
            attrs.push_str(&format!(" review=\"{}\"", t.review_state().as_str()));
            if let Some(r) = a.and_then(|a| a.runtime_id.as_deref()).or(t.runtime_id.as_deref()) {
                attrs.push_str(&format!(" runtime=\"{}\"", llm_context::escape_xml_attr(r)));
            }
            if let Some(w) = a.and_then(|a| a.workspace_id.as_deref()).or(t.workspace.as_deref()) {
                attrs.push_str(&format!(" workspace=\"{}\"", llm_context::escape_xml_attr(w)));
            }
            if !t.after.is_empty() {
                attrs.push_str(&format!(" after=\"{}\"", t.after.join(",")));
            }
            if t.retry.is_some() {
                attrs.push_str(" retry=\"pending\"");
            }
            let mut body = llm_context::escape_xml_text(&t.title);
            if let Some(summary) = a.and_then(|a| a.summary.as_deref()).filter(|s| !s.is_empty()) {
                body.push_str(" — ");
                body.push_str(&llm_context::escape_xml_text(summary));
            } else if let Some(err) = a.and_then(|a| a.error.as_deref()) {
                body.push_str(" — error: ");
                body.push_str(&llm_context::escape_xml_text(&one_line(err)));
            }
            if let Some(why) = &t.cancel_reason {
                body.push_str(" — cancelled: ");
                body.push_str(&llm_context::escape_xml_text(&one_line(why)));
            }
            s.push_str(&format!("<todo {attrs}>{body}</todo>\n"));
        }
        s.push_str("</todo_list>");
        s
    }
}

fn clean(items: Vec<String>) -> Vec<String> {
    items
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn apply_observation(a: &mut TodoAttempt, seen: ChildObservation, now_ms: u64) {
    a.status = seen.status;
    if seen.status.is_terminal() {
        a.ended_at_ms = Some(now_ms);
    }
    if seen.summary.is_some() {
        a.summary = seen.summary;
    }
    if seen.report.is_some() {
        a.report = seen.report;
    }
    if !seen.artifacts.is_empty() {
        a.artifacts = seen.artifacts;
    }
    if seen.error.is_some() {
        a.error = seen.error;
    }
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/// The list file of one session (`<state_dir>/todos.json`) and its lock.
#[derive(Clone, Debug)]
pub struct TodoStore {
    path: PathBuf,
    lock_path: PathBuf,
}

impl TodoStore {
    /// The store in a session's state directory.
    pub fn in_dir(state_dir: impl AsRef<Path>) -> Self {
        let dir = state_dir.as_ref();
        Self {
            path: dir.join(TODO_LIST_FILE),
            lock_path: dir.join(TODO_LOCK_FILE),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The committed list (an empty one when there is none).
    pub fn load(&self) -> Result<TodoList, AgentToolError> {
        read_list(&self.path)
    }

    /// Read-modify-write under the store lock; the list is published by
    /// atomic replace only when `mutate` succeeds and changed it.
    pub fn update<R>(
        &self,
        mutate: impl FnOnce(&mut TodoList) -> Result<R, AgentToolError>,
    ) -> Result<R, AgentToolError> {
        let _guard = self.lock()?;
        let mut list = read_list(&self.path)?;
        let before = list.rev;
        let out = mutate(&mut list)?;
        if list.rev != before {
            write_list(&self.path, &list)?;
        }
        Ok(out)
    }

    /// Hold the store lock (e.g. across a dispatch that creates a sub
    /// session between two writes). Released on drop and with the process.
    pub fn lock(&self) -> Result<StoreLock, AgentToolError> {
        if let Some(parent) = self.lock_path.parent() {
            std::fs::create_dir_all(parent).map_err(io(parent))?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&self.lock_path)
            .map_err(io(&self.lock_path))?;
        FileExt::lock_exclusive(&file).map_err(io(&self.lock_path))?;
        Ok(StoreLock {
            file,
            path: self.path.clone(),
        })
    }
}

/// An exclusive hold on a session's list.
pub struct StoreLock {
    file: File,
    path: PathBuf,
}

impl StoreLock {
    pub fn load(&self) -> Result<TodoList, AgentToolError> {
        read_list(&self.path)
    }

    pub fn update<R>(
        &self,
        mutate: impl FnOnce(&mut TodoList) -> Result<R, AgentToolError>,
    ) -> Result<R, AgentToolError> {
        let mut list = read_list(&self.path)?;
        let before = list.rev;
        let out = mutate(&mut list)?;
        if list.rev != before {
            write_list(&self.path, &list)?;
        }
        Ok(out)
    }
}

impl Drop for StoreLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> AgentToolError + '_ {
    move |e| AgentToolError::ExecFailed(format!("{}: {e}", path.display()))
}

fn read_list(path: &Path) -> Result<TodoList, AgentToolError> {
    let mut buf = String::new();
    match File::open(path) {
        Ok(mut f) => {
            f.read_to_string(&mut buf).map_err(io(path))?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(TodoList::default()),
        Err(e) => return Err(io(path)(e)),
    }
    if buf.trim().is_empty() {
        return Ok(TodoList::default());
    }
    let list: TodoList = serde_json::from_str(&buf).map_err(|e| {
        AgentToolError::ExecFailed(format!("{}: not a todo list ({TODO_LIST_SCHEMA}): {e}", path.display()))
    })?;
    if list.schema != TODO_LIST_SCHEMA {
        return Err(AgentToolError::ExecFailed(format!(
            "{}: schema `{}` is not {TODO_LIST_SCHEMA}",
            path.display(),
            list.schema
        )));
    }
    Ok(list)
}

fn write_list(path: &Path, list: &TodoList) -> Result<(), AgentToolError> {
    let dir = path
        .parent()
        .ok_or_else(|| AgentToolError::ExecFailed(format!("{} has no directory", path.display())))?;
    std::fs::create_dir_all(dir).map_err(io(dir))?;
    let data = serde_json::to_vec_pretty(list)
        .map_err(|e| AgentToolError::ExecFailed(format!("serialize todo list: {e}")))?;
    let tmp = dir.join(format!(".{TODO_LIST_FILE}.{}.tmp", std::process::id()));
    {
        let mut f = File::create(&tmp).map_err(io(&tmp))?;
        f.write_all(&data).map_err(io(&tmp))?;
        f.sync_all().map_err(io(&tmp))?;
    }
    std::fs::rename(&tmp, path).map_err(io(path))?;
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn todo(title: &str) -> NewTodo {
        NewTodo {
            title: title.into(),
            content: format!("do {title}"),
            ..Default::default()
        }
    }

    fn caller(n: u32) -> CallRef {
        CallRef {
            run_id: "run-1".into(),
            call_id: format!("step:{n}"),
        }
    }

    fn done(status: TodoStatus, summary: &str) -> ChildObservation {
        ChildObservation {
            status,
            summary: Some(summary.into()),
            report: Some(ReportRef {
                session_id: "work-s".into(),
                run_id: None,
                report_id: format!("r-{summary}"),
            }),
            artifacts: Vec::new(),
            error: None,
        }
    }

    /// Ids come from a persisted counter: cancelling never frees one.
    #[test]
    fn ids_are_monotonic_and_never_reused() {
        let dir = tempfile::tempdir().unwrap();
        let store = TodoStore::in_dir(dir.path());
        let a = store.update(|l| l.add(todo("a"), 1)).unwrap();
        let b = store.update(|l| l.add(todo("b"), 2)).unwrap();
        assert_eq!((a.as_str(), b.as_str()), ("T01", "T02"));
        let cancelled = store.update(|l| l.clean("replan", 3)).unwrap();
        assert_eq!(cancelled, vec!["T01", "T02"]);
        let c = store.update(|l| l.add(todo("c"), 4)).unwrap();
        assert_eq!(c, "T03");
        let list = store.load().unwrap();
        assert_eq!(list.todos.len(), 3, "cancel keeps the records");
        assert_eq!(list.todos[0].status, TodoStatus::Cancelled);
        assert_eq!(list.todos[0].cancel_reason.as_deref(), Some("replan"));
        assert_eq!(list.next_seq, 4);
    }

    /// The serial pick skips explorations and blocked Todos, waits for
    /// prerequisites and asks for reviews first.
    #[test]
    fn serial_pick_rules() {
        let mut l = TodoList::default();
        let a = l.add(NewTodo { mode: TodoMode::Explore, ..todo("explore a") }, 1).unwrap();
        let b = l.add(NewTodo { mode: TodoMode::Explore, ..todo("explore b") }, 1).unwrap();
        let c = l
            .add(NewTodo { after: vec![a.clone(), b.clone()], ..todo("verify") }, 1)
            .unwrap();
        assert_eq!(l.serial_pick(), SerialPick::Waiting(vec![(c.clone(), vec![a.clone(), b.clone()])]));
        let n = l.begin_explore(&a, "work-a", 2).unwrap();
        l.bind_explore(&a, n, 2, AttemptTarget::default()).unwrap();
        let m = l.begin_explore(&b, "work-b", 2).unwrap();
        assert_eq!(l.explorations_in_flight().len(), 2);
        l.observe_child(&a, n, ChildObservation { status: TodoStatus::Blocked, summary: None, report: None, artifacts: vec![], error: None }, 3).unwrap();
        assert_eq!(l.get(&a).unwrap().status, TodoStatus::Blocked);
        l.observe_child(&a, n, done(TodoStatus::Completed, "a found it"), 4).unwrap();
        assert_eq!(l.serial_pick(), SerialPick::Unreviewed(vec![a.clone()]));
        l.review(&a, ReviewVerdict::Accept, "repro holds", vec![], None, 5).unwrap();
        l.observe_child(&b, m, done(TodoStatus::Completed, "b found it"), 6).unwrap();
        l.review(&b, ReviewVerdict::Accept, "repro holds", vec![], None, 7).unwrap();
        assert_eq!(l.serial_pick(), SerialPick::Ready(c.clone()));
        let k = l.begin_serial(&c, caller(9), 8).unwrap();
        assert_eq!(k, 1);
        assert_eq!(l.begin_serial(&c, caller(9), 8).unwrap(), 1, "same caller, same attempt");
        assert!(l.begin_serial(&c, caller(10), 8).is_err(), "a running todo is not admitted again");
        assert_eq!(l.serial_pick(), SerialPick::None);
        let text = l.get(&c).unwrap().task_text(&l, k);
        assert!(text.contains("T01 explore a: a found it (report work-s#r-a found it)"), "{text}");
    }

    /// A late result of an old attempt never reaches the new one; a retry
    /// keeps the old attempt and its report.
    #[test]
    fn retries_keep_history_and_late_results_stay_with_their_attempt() {
        let mut l = TodoList::default();
        let a = l.add(todo("a"), 1).unwrap();
        let first = l.begin_serial(&a, caller(1), 1).unwrap();
        assert!(l.end_serial(&a, first, "run-do-1", done(TodoStatus::Failed, "no evidence"), 2).unwrap());
        assert!(!l.end_serial(&a, first, "run-do-1", done(TodoStatus::Completed, "late"), 3).unwrap());
        assert!(l.retry(&a, "", 3).is_err());
        assert_eq!(l.unconverged(), vec!["the result of T01 (attempt 1) is not reviewed".to_string()]);
        l.review(&a, ReviewVerdict::Reject, "no repro", vec![], None, 3).unwrap();
        assert!(l.review(&a, ReviewVerdict::Accept, "changed my mind", vec![], None, 3).is_err());
        let (_, again) = l.review(&a, ReviewVerdict::Reject, "no repro", vec![], None, 3).unwrap();
        assert!(!again, "the same review again is a no-op");
        l.retry(&a, "use the public entry", 4).unwrap();
        assert_eq!(l.get(&a).unwrap().status, TodoStatus::Pending);
        assert_eq!(l.serial_pick(), SerialPick::Ready(a.clone()));
        let second = l.begin_serial(&a, caller(5), 5).unwrap();
        assert_eq!(second, 2);
        let t = l.get(&a).unwrap();
        assert_eq!(t.attempt(2).unwrap().reason.as_deref(), Some("use the public entry"));
        assert_eq!(t.attempt(1).unwrap().summary.as_deref(), Some("no evidence"));
        assert!(l.cancel(&a, "running", 6).is_err(), "cancel only applies before dispatch");
        l.end_serial(&a, second, "run-do-2", done(TodoStatus::Completed, "verified"), 7).unwrap();
        assert_eq!(l.get(&a).unwrap().review_state(), ReviewState::Pending);
        l.review(&a, ReviewVerdict::Accept, "verified", vec![], None, 8).unwrap();
        assert!(l.unconverged().is_empty());
    }

    /// A withdrawn dispatch leaves the Todo pending with the refusal.
    #[test]
    fn a_refused_dispatch_is_observable_and_retriable() {
        let mut l = TodoList::default();
        let a = l.add(NewTodo { mode: TodoMode::Explore, ..todo("a") }, 1).unwrap();
        let n = l.begin_explore(&a, "work-a1", 2).unwrap();
        l.withdraw_dispatch(&a, n, "max_sub_sessions", 3).unwrap();
        let t = l.get(&a).unwrap();
        assert_eq!(t.status, TodoStatus::Pending);
        assert!(t.attempts.is_empty());
        assert_eq!(t.refusals[0].reason, "max_sub_sessions");
        let again = l.begin_explore(&a, "work-a1", 4).unwrap();
        assert_eq!(again, 1);
        let view = l.render_view();
        assert!(view.contains("id=\"T01\" mode=\"explore\" status=\"running\" attempt=\"1\" child_sid=\"work-a1\""), "{view}");
    }

    #[test]
    fn concurrent_updates_are_serialized_by_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let store = TodoStore::in_dir(dir.path());
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let s = store.clone();
                std::thread::spawn(move || s.update(|l| l.add(todo(&format!("t{i}")), 1)).unwrap())
            })
            .collect();
        let mut ids: Vec<String> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        ids.sort();
        assert_eq!(ids.len(), 8);
        ids.dedup();
        assert_eq!(ids.len(), 8);
        assert_eq!(store.load().unwrap().todos.len(), 8);
    }
}
