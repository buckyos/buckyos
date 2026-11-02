//! Session TodoList — the records and their rules (`doc/opendan/Agent TodoList.md`).
//!
//! The TodoList of a plan session is its persistent decision basis: what is
//! to be done, in which order (`after`), how each attempt ran in a work
//! session of its own and what it reported. This module only holds the
//! records, their state rules and the file store. When they change is
//! decided by the session host (libopendan): it dispatches every Todo that
//! is ready, writes the attempts' ends back and runs the plan's
//! `agent-session todo` commands; there is no prompt-visible tool here.
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
    /// Not dispatched and not dispatchable until the plan decides: a Todo
    /// it runs after ended without success (`blocked_by`), or its dispatch
    /// failed (`block_reason`).
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

/// How an attempt runs. Both run in a work session of their own.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TodoMode {
    /// Work (behavior `work_simple`, the plan's workspace), in the order of
    /// its `after` chain.
    #[default]
    Serial,
    /// A read-only investigation (behavior `explore`, no workspace).
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

/// Dispatch of an attempt (crash recovery: a `dispatching` attempt is
/// reconciled against the registry by its sub session id).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DispatchState {
    Dispatching,
    Dispatched,
}

/// Where the report of an attempt is: the final report of its sub session.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ReportRef {
    pub session_id: String,
    pub report_id: String,
}

impl ReportRef {
    pub fn display(&self) -> String {
        format!("{}#{}", self.session_id, self.report_id)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct TodoAttempt {
    pub attempt: u32,
    pub mode: TodoMode,
    /// The work session executing the attempt.
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
    /// The declared baseline of that runtime (`standard | extended`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<String>,
    /// The behavior its sub session started with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behavior: Option<String>,
    /// `running | completed | failed | timeout | stopped`.
    pub status: TodoStatus,
    pub started_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at_ms: Option<u64>,
    /// The plan paused its work session (it infers no more until resumed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resumed_at_ms: Option<u64>,
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

    /// Paused by the plan and not resumed since.
    pub fn paused(&self) -> bool {
        !self.ended()
            && self
                .paused_at_ms
                .is_some_and(|p| self.resumed_at_ms.is_none_or(|r| r < p))
    }
}

/// One change of a Todo after it was added (`todo update`).
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct TodoUpdate {
    pub at_ms: u64,
    pub reason: String,
    /// The fields it changed.
    pub fields: Vec<String>,
    /// The attempt that was running and was told (`None`: none ran).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
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
    /// Todos that must have completed before this one runs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub after: Vec<String>,
    /// Todos of `after` that ended without success: this one waits for the
    /// plan (retry them, change `after`, or cancel it).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_by: Vec<String>,
    /// Why the last dispatch failed (cleared by `update` / `retry`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_reason: Option<String>,
    /// Dispatch batch (explorations sent out together).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch: Option<String>,
    /// Where the Todo must execute: a session-runtime id (`None`: the
    /// session's own), and `inherit | none | <workspace id>` (`None`: a
    /// serial Todo inherits the session's workspace, an exploration takes
    /// none). The two are chosen independently: a workspace on another
    /// runtime than the work's is reached by copying files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// The behavior a sub session executing the Todo starts with (`None`:
    /// the mode's default); set when it is first dispatched, so a retry
    /// runs the same way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behavior: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attempts: Vec<TodoAttempt>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub updates: Vec<TodoUpdate>,
    /// Pending retry: the reason for the next attempt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel_reason: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
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

    /// An attempt is executing, or being dispatched.
    pub fn in_flight(&self) -> bool {
        self.latest().is_some_and(|a| !a.ended())
    }

    /// The attempt executing now, if any.
    pub fn running(&self) -> Option<&TodoAttempt> {
        self.latest().filter(|a| !a.ended())
    }

    /// Waiting for a (new) attempt: none was made yet, or a retry was asked.
    fn awaits_attempt(&self) -> bool {
        self.cancel_reason.is_none()
            && !self.in_flight()
            && (self.attempts.is_empty() || self.retry.is_some())
    }

    /// The status follows the records: cancelled; an attempt in flight →
    /// its status; waiting for an attempt → blocked (dependencies that
    /// failed, a failed dispatch) or pending; otherwise the latest
    /// attempt's status.
    fn refresh(&mut self) {
        self.status = if self.cancel_reason.is_some() {
            TodoStatus::Cancelled
        } else if let Some(a) = self.running() {
            a.status
        } else if self.awaits_attempt() {
            if self.blocked_by.is_empty() && self.block_reason.is_none() {
                TodoStatus::Pending
            } else {
                TodoStatus::Blocked
            }
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

    /// The complete task of the Todo as its work session receives it: body,
    /// acceptance criteria, scope, evidence, skills and the results of the
    /// Todos it runs after.
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
            s.push_str("\nBuilds on the results of:\n");
            for d in deps {
                let a = d.latest();
                s.push_str(&format!(
                    "- {} {}: {}{}\n",
                    d.todo_id,
                    d.title,
                    a.and_then(|a| a.summary.clone()).unwrap_or_default(),
                    a.and_then(|a| a.report.as_ref())
                        .map(|r| format!(
                            " (report {}; read it with `agent-session read-session {} --report`)",
                            r.display(),
                            r.session_id
                        ))
                        .unwrap_or_default()
                ));
                if let Some(a) = a.filter(|a| !a.artifacts.is_empty()) {
                    s.push_str(&format!("  artifacts: {}\n", a.artifacts.join(", ")));
                }
            }
        }
        if let Some(reason) = self.attempt(attempt).and_then(|a| a.reason.clone()) {
            s.push_str(&format!("\nWhy this attempt: {reason}\n"));
        }
        s
    }

    /// The Todo as one element of the compact view: its state, order,
    /// where its latest attempt runs and its result line — no execution
    /// detail.
    pub fn view_line(&self, list: &TodoList) -> String {
        let a = self.latest();
        let mut attrs = format!(
            "id=\"{}\" mode=\"{}\" status=\"{}\"",
            self.todo_id,
            self.mode.as_str(),
            self.status.as_str()
        );
        if !self.after.is_empty() {
            attrs.push_str(&format!(" after=\"{}\"", self.after.join(",")));
        }
        if !self.blocked_by.is_empty() {
            let why: Vec<String> = self
                .blocked_by
                .iter()
                .map(|d| match list.get(d) {
                    Some(dep) => format!("{d}:{}", dep.status.as_str()),
                    None => d.clone(),
                })
                .collect();
            attrs.push_str(&format!(" blocked_by=\"{}\"", why.join(",")));
        }
        if let Some(a) = a {
            attrs.push_str(&format!(" attempt=\"{}\"", a.attempt));
            if let Some(c) = &a.child_sid {
                attrs.push_str(&format!(" child_sid=\"{}\"", crate::escape_xml_attr(c)));
            }
            attrs.push_str(&format!(" started=\"{}\"", utc(a.started_at_ms)));
            if a.paused() {
                attrs.push_str(" paused=\"true\"");
            }
            if let Some(r) = &a.report {
                attrs.push_str(&format!(
                    " report_ref=\"{}\"",
                    crate::escape_xml_attr(&r.display())
                ));
            }
        }
        if let Some(r) = a.and_then(|a| a.runtime_id.as_deref()).or(self.runtime_id.as_deref()) {
            attrs.push_str(&format!(" runtime=\"{}\"", crate::escape_xml_attr(r)));
        }
        if let Some(b) = a.and_then(|a| a.baseline.as_deref()) {
            attrs.push_str(&format!(" baseline=\"{}\"", crate::escape_xml_attr(b)));
        }
        if let Some(w) = a.and_then(|a| a.workspace_id.as_deref()).or(self.workspace.as_deref()) {
            attrs.push_str(&format!(" workspace=\"{}\"", crate::escape_xml_attr(w)));
        }
        if self.retry.is_some() {
            attrs.push_str(" retry=\"pending\"");
        }
        if !self.updates.is_empty() {
            attrs.push_str(&format!(" updates=\"{}\"", self.updates.len()));
        }
        let mut body = crate::escape_xml_text(&self.title);
        if let Some(why) = &self.block_reason {
            body.push_str(" — dispatch failed: ");
            body.push_str(&crate::escape_xml_text(&one_line(why)));
        } else if let Some(summary) = a.and_then(|a| a.summary.as_deref()).filter(|s| !s.is_empty()) {
            body.push_str(" — ");
            body.push_str(&crate::escape_xml_text(summary));
        } else if let Some(err) = a.and_then(|a| a.error.as_deref()) {
            body.push_str(" — error: ");
            body.push_str(&crate::escape_xml_text(&one_line(err)));
        }
        if let Some(why) = &self.cancel_reason {
            body.push_str(" — cancelled: ");
            body.push_str(&crate::escape_xml_text(&one_line(why)));
        }
        format!("<todo {attrs}>{body}</todo>")
    }
}

/// `2026-11-02T08:15:03Z`.
fn utc(ms: u64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms as i64)
        .map(|t| t.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_default()
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
    /// `None`: after the latest Todo that is not cancelled (a chain);
    /// `Some(vec![])`: no prerequisite.
    pub after: Option<Vec<String>>,
    pub batch: Option<String>,
    pub runtime_id: Option<String>,
    pub workspace: Option<String>,
    pub behavior: Option<String>,
}

/// Changes of a Todo (`todo update`); `None` leaves a field as it is.
#[derive(Clone, Debug, Default)]
pub struct TodoPatch {
    pub title: Option<String>,
    pub content: Option<String>,
    /// Criteria to add (after `clear_accept`).
    pub accept: Vec<String>,
    pub clear_accept: bool,
    pub scope: Option<String>,
    /// Evidence to add.
    pub evidence: Vec<String>,
    /// Replaces `after` (`Some(vec![])`: no prerequisite).
    pub after: Option<Vec<String>>,
    pub runtime_id: Option<String>,
    pub workspace: Option<String>,
    /// Replaces the cmd_skills.
    pub skills: Option<Vec<String>>,
}

/// What `update` changed, and the attempt that runs it now (its work
/// session is told).
#[derive(Clone, Debug, PartialEq)]
pub struct UpdateOutcome {
    pub changed: Vec<String>,
    /// `(attempt, child_sid)` of the attempt in flight.
    pub running: Option<(u32, String)>,
}

/// Where an attempt was resolved to execute.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AttemptTarget {
    pub runtime_id: Option<String>,
    pub workspace_id: Option<String>,
    pub baseline: Option<String>,
    pub behavior: Option<String>,
}

/// What the session host observed of an attempt's sub session.
#[derive(Clone, Debug, PartialEq)]
pub struct ChildObservation {
    pub status: TodoStatus,
    pub summary: Option<String>,
    pub report: Option<ReportRef>,
    pub artifacts: Vec<String>,
    pub error: Option<String>,
}

/// The list at a glance (the plan's final report and the worklog, never a
/// gate): how many Todos are in each state, and which are open.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct TodoSummary {
    pub total: usize,
    pub counts: std::collections::BTreeMap<String, usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub running: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub paused: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pending: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub blocked: Vec<String>,
}

impl TodoSummary {
    /// `5 todos: 3 completed, 1 failed, 1 running (T05)`.
    pub fn line(&self) -> String {
        if self.total == 0 {
            return "no todos".into();
        }
        let parts: Vec<String> = self
            .counts
            .iter()
            .map(|(status, n)| {
                let ids = match status.as_str() {
                    "running" => &self.running,
                    "pending" => &self.pending,
                    "blocked" => &self.blocked,
                    _ => return format!("{n} {status}"),
                };
                format!("{n} {status} ({})", ids.join(", "))
            })
            .collect();
        let mut s = format!("{} todos: {}", self.total, parts.join(", "));
        if !self.paused.is_empty() {
            s.push_str(&format!("; paused: {}", self.paused.join(", ")));
        }
        s
    }
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

    /// A change of the list: the dependents' blocks follow it, the
    /// revision moves.
    fn changed(&mut self, now_ms: u64) {
        self.settle_dependents(now_ms);
        self.rev += 1;
    }

    /// The Todos `after` may name for `todo_id` (`None`: a new one): known
    /// ids, never itself, never a chain that leads back to it.
    fn check_after(&self, todo_id: Option<&str>, after: &[String]) -> Result<(), AgentToolError> {
        for dep in after {
            if self.get(dep).is_none() {
                return Err(invalid(format!("--after {dep}: no such todo")));
            }
            let Some(me) = todo_id else {
                continue;
            };
            let mut stack = vec![dep.clone()];
            let mut seen = std::collections::BTreeSet::new();
            while let Some(id) = stack.pop() {
                if id == me {
                    return Err(invalid(format!(
                        "--after {dep}: {me} would run after itself (through {dep})"
                    )));
                }
                if seen.insert(id.clone()) {
                    if let Some(t) = self.get(&id) {
                        stack.extend(t.after.iter().cloned());
                    }
                }
            }
        }
        Ok(())
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
        let after = match new.after {
            Some(after) => clean(after),
            // A chain: after the latest Todo that is not cancelled.
            None => self
                .todos
                .iter()
                .filter(|t| t.status != TodoStatus::Cancelled)
                .max_by_key(|t| t.seq)
                .map(|t| vec![t.todo_id.clone()])
                .unwrap_or_default(),
        };
        self.check_after(None, &after)?;
        let runtime_id = runtime_arg(new.runtime_id);
        let workspace = workspace_arg(new.workspace)?;
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
            after,
            blocked_by: Vec::new(),
            block_reason: None,
            batch: new.batch,
            runtime_id,
            workspace,
            behavior: new.behavior.map(|b| b.trim().to_string()).filter(|b| !b.is_empty()),
            attempts: Vec::new(),
            updates: Vec::new(),
            retry: None,
            cancel_reason: None,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        };
        record.touch(now_ms);
        self.todos.push(record);
        self.changed(now_ms);
        Ok(todo_id)
    }

    /// Whether every Todo `t` runs after completed (`Ok`), is still on its
    /// way (`Err(None)`), or ended without success (`Err(Some(ids))`).
    fn deps_of(&self, t: &TodoRecord) -> Result<(), Option<Vec<String>>> {
        let mut failed = Vec::new();
        let mut waiting = false;
        for dep in &t.after {
            match self.get(dep).map(|d| d.status) {
                Some(TodoStatus::Completed) => {}
                Some(
                    TodoStatus::Failed
                    | TodoStatus::Timeout
                    | TodoStatus::Stopped
                    | TodoStatus::Cancelled,
                )
                | None => failed.push(dep.clone()),
                Some(_) => waiting = true,
            }
        }
        if !failed.is_empty() {
            Err(Some(failed))
        } else if waiting {
            Err(None)
        } else {
            Ok(())
        }
    }

    /// A Todo waiting for an attempt whose prerequisite ended without
    /// success is blocked by it; once that is retried, changed or the
    /// prerequisite completes, it is pending again.
    pub fn settle_dependents(&mut self, now_ms: u64) {
        let blocks: Vec<(usize, Vec<String>)> = self
            .todos
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let by = if t.awaits_attempt() {
                    match self.deps_of(t) {
                        Err(Some(failed)) => failed,
                        _ => Vec::new(),
                    }
                } else {
                    Vec::new()
                };
                (i, by)
            })
            .collect();
        for (i, by) in blocks {
            let t = &mut self.todos[i];
            if t.blocked_by != by {
                t.blocked_by = by;
                t.touch(now_ms);
            }
        }
    }

    /// Todos that start now: pending, every Todo they run after completed;
    /// in creation order.
    pub fn ready(&self) -> Vec<String> {
        let mut out: Vec<&TodoRecord> = self
            .todos
            .iter()
            .filter(|t| t.status == TodoStatus::Pending && self.deps_of(t).is_ok())
            .collect();
        out.sort_by_key(|t| t.seq);
        out.into_iter().map(|t| t.todo_id.clone()).collect()
    }

    /// Whether `todo_id` may start now: pending and every Todo it runs
    /// after completed. `Err`: why not.
    pub fn admit(&self, todo_id: &str) -> Result<(), AgentToolError> {
        let t = self
            .get(todo_id)
            .ok_or_else(|| AgentToolError::NotFound(format!("todo `{todo_id}` not found")))?;
        if t.status != TodoStatus::Pending {
            return Err(invalid(format!(
                "todo {todo_id} is {}; only a pending todo is dispatched",
                t.status.as_str()
            )));
        }
        match self.deps_of(t) {
            Ok(()) => Ok(()),
            Err(Some(ids)) => Err(invalid(format!(
                "todo {todo_id} runs after {}, which ended without success",
                ids.join(", ")
            ))),
            Err(None) => Err(invalid(format!(
                "todo {todo_id} runs after {}, which did not complete yet",
                t.after.join(", ")
            ))),
        }
    }

    /// Record the dispatch of an attempt before its sub session is created.
    /// A `dispatching` attempt of the same sub session is reused (a
    /// dispatch redone after a crash).
    pub fn begin_dispatch(
        &mut self,
        todo_id: &str,
        child_sid: &str,
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
                "todo {todo_id} is {}; only a pending todo is dispatched",
                t.status.as_str()
            )));
        }
        let attempt = t.next_attempt();
        let mode = t.mode;
        t.attempts.push(TodoAttempt {
            attempt,
            mode,
            child_sid: Some(child_sid.to_string()),
            dispatch: Some(DispatchState::Dispatching),
            runtime_id: t.runtime_id.clone(),
            workspace_id: t.workspace.clone().filter(|w| w.starts_with("ws-")),
            baseline: None,
            behavior: None,
            status: TodoStatus::Running,
            started_at_ms: now_ms,
            ended_at_ms: None,
            paused_at_ms: None,
            resumed_at_ms: None,
            error: None,
            summary: None,
            report: None,
            artifacts: Vec::new(),
            reason: t.retry.take(),
        });
        t.touch(now_ms);
        self.changed(now_ms);
        Ok(attempt)
    }

    /// The sub session of a dispatched attempt exists: bound, with where it
    /// was resolved to execute.
    pub fn bind_dispatch(
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
        if target.baseline.is_some() {
            a.baseline = target.baseline;
        }
        if target.behavior.is_some() {
            a.behavior = target.behavior.clone();
            t.behavior = target.behavior;
        }
        t.touch(now_ms);
        self.changed(now_ms);
        Ok(true)
    }

    /// A dispatch that did not create its sub session: the attempt is
    /// withdrawn. `failure`: why it cannot be dispatched as it is — the
    /// Todo is blocked until the plan changes it; `None` (held back by a
    /// limit, interrupted): pending, the next scheduling point tries again.
    pub fn withdraw_dispatch(
        &mut self,
        todo_id: &str,
        attempt: u32,
        failure: Option<&str>,
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
        if let Some(why) = failure {
            t.block_reason = Some(why.to_string());
        }
        t.touch(now_ms);
        self.changed(now_ms);
        Ok(())
    }

    /// What the host observed of an attempt's sub session (idempotent; an
    /// ended attempt keeps its result, and only its own attempt changes — a
    /// late result never reaches a newer attempt).
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
        self.changed(now_ms);
        Ok(true)
    }

    /// Change a Todo (`todo update`). Its body, criteria, scope and evidence
    /// change in any state but cancelled — a running attempt is told
    /// (`UpdateOutcome.running`); where and in which order it runs
    /// (`after`, runtime, workspace, cmd_skills) only while no attempt
    /// runs. The change and its reason are kept in `updates`.
    pub fn update(
        &mut self,
        todo_id: &str,
        patch: TodoPatch,
        reason: &str,
        by: Option<String>,
        now_ms: u64,
    ) -> Result<UpdateOutcome, AgentToolError> {
        if reason.trim().is_empty() {
            return Err(invalid("update needs --reason: why the todo changes"));
        }
        if let Some(after) = &patch.after {
            self.check_after(Some(todo_id), after)?;
        }
        let workspace = workspace_arg(patch.workspace.clone())?;
        let t = self.get_mut(todo_id)?;
        if t.status == TodoStatus::Cancelled {
            return Err(invalid(format!("todo {todo_id} is cancelled; add a new one")));
        }
        let running = t
            .running()
            .and_then(|a| a.child_sid.clone().map(|c| (a.attempt, c)));
        let placement = [
            ("after", patch.after.is_some()),
            ("runtime", patch.runtime_id.is_some()),
            ("workspace", workspace.is_some()),
            ("skills", patch.skills.is_some()),
        ];
        let fixed: Vec<&str> = placement.iter().filter(|(_, set)| *set).map(|(f, _)| *f).collect();
        if t.in_flight() && !fixed.is_empty() {
            return Err(invalid(format!(
                "todo {todo_id} is running (attempt {}): its {} cannot change now; change only its body (--content, --accept, --scope, --evidence), or stop it and update it before the retry",
                t.latest().map(|a| a.attempt).unwrap_or(0),
                fixed.join(", ")
            )));
        }
        if let Some(skills) = &patch.skills {
            if skills.len() > MAX_SKILLS_PER_TODO {
                return Err(invalid(format!(
                    "at most {MAX_SKILLS_PER_TODO} cmd_skills per todo (got {})",
                    skills.len()
                )));
            }
        }
        let mut changed = Vec::new();
        if let Some(title) = patch.title.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
            if title != t.title {
                t.title = title;
                changed.push("title");
            }
        }
        if let Some(content) = patch.content.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
            if content != t.content {
                t.content = content;
                changed.push("content");
            }
        }
        if patch.clear_accept || !patch.accept.is_empty() {
            let mut accept = if patch.clear_accept { Vec::new() } else { t.accept.clone() };
            accept.extend(clean(patch.accept));
            if accept != t.accept {
                t.accept = accept;
                changed.push("accept");
            }
        }
        if let Some(scope) = patch.scope {
            let scope = Some(scope.trim().to_string()).filter(|s| !s.is_empty());
            if scope != t.scope {
                t.scope = scope;
                changed.push("scope");
            }
        }
        let evidence = clean(patch.evidence);
        if !evidence.is_empty() {
            t.evidence.extend(evidence);
            changed.push("evidence");
        }
        if let Some(after) = patch.after {
            let after = clean(after);
            if after != t.after {
                t.after = after;
                changed.push("after");
            }
        }
        if let Some(runtime) = patch.runtime_id {
            let runtime = runtime_arg(Some(runtime));
            if runtime != t.runtime_id {
                t.runtime_id = runtime;
                changed.push("runtime");
            }
        }
        if workspace.is_some() && workspace != t.workspace {
            t.workspace = workspace;
            changed.push("workspace");
        }
        if let Some(skills) = patch.skills {
            let skills = clean(skills);
            if skills != t.skills {
                t.skills = skills;
                changed.push("skills");
            }
        }
        if changed.is_empty() {
            return Err(invalid(format!("todo {todo_id}: nothing to change")));
        }
        // A failed dispatch is tried again with the changed Todo.
        t.block_reason = None;
        t.updates.push(TodoUpdate {
            at_ms: now_ms,
            reason: reason.trim().to_string(),
            fields: changed.iter().map(|f| f.to_string()).collect(),
            attempt: running.as_ref().map(|(n, _)| *n),
            by,
        });
        t.touch(now_ms);
        self.changed(now_ms);
        Ok(UpdateOutcome {
            changed: changed.into_iter().map(str::to_string).collect(),
            running,
        })
    }

    /// The attempt of `todo_id` the plan pauses / resumes: in flight, with
    /// its work session. `Ok(None)`: already in that state.
    pub fn mark_paused(
        &mut self,
        todo_id: &str,
        paused: bool,
        now_ms: u64,
    ) -> Result<Option<(u32, String)>, AgentToolError> {
        let t = self.get_mut(todo_id)?;
        let Some(a) = t.attempts.last_mut().filter(|a| !a.ended()) else {
            return Err(invalid(format!(
                "todo {todo_id} is {}: only a running todo is {}",
                t.status.as_str(),
                if paused { "paused" } else { "resumed" }
            )));
        };
        let Some(child) = a.child_sid.clone() else {
            return Err(invalid(format!("todo {todo_id} has no work session yet")));
        };
        if a.paused() == paused {
            return Ok(None);
        }
        if paused {
            a.paused_at_ms = Some(now_ms);
        } else {
            a.resumed_at_ms = Some(now_ms);
        }
        let n = a.attempt;
        t.touch(now_ms);
        self.changed(now_ms);
        Ok(Some((n, child)))
    }

    /// Cancel a Todo that is not running and did not complete; its history
    /// stays.
    pub fn cancel(&mut self, todo_id: &str, reason: &str, now_ms: u64) -> Result<(), AgentToolError> {
        if reason.trim().is_empty() {
            return Err(invalid("cancel needs --reason"));
        }
        let t = self.get_mut(todo_id)?;
        if t.in_flight() || matches!(t.status, TodoStatus::Completed | TodoStatus::Cancelled) {
            return Err(invalid(format!(
                "todo {todo_id} is {}: only a todo that is not running and did not complete can be cancelled{}",
                t.status.as_str(),
                if t.in_flight() { " (stop it first: `todo stop`)" } else { "" }
            )));
        }
        t.cancel_reason = Some(reason.trim().to_string());
        t.retry = None;
        t.touch(now_ms);
        self.changed(now_ms);
        Ok(())
    }

    /// Cancel every Todo that waits for an attempt (nothing is deleted, no
    /// id is reused).
    pub fn clean(&mut self, reason: &str, now_ms: u64) -> Result<Vec<String>, AgentToolError> {
        let ids: Vec<String> = self
            .todos
            .iter()
            .filter(|t| matches!(t.status, TodoStatus::Pending | TodoStatus::Blocked))
            .map(|t| t.todo_id.clone())
            .collect();
        for id in &ids {
            self.cancel(id, reason, now_ms)?;
        }
        Ok(ids)
    }

    /// Ask for a new attempt of a Todo whose latest attempt ended, or whose
    /// dispatch failed; the old attempts and their reports stay.
    pub fn retry(&mut self, todo_id: &str, reason: &str, now_ms: u64) -> Result<(), AgentToolError> {
        if reason.trim().is_empty() {
            return Err(invalid("retry needs --reason: what the new attempt must do differently"));
        }
        let t = self.get_mut(todo_id)?;
        if t.block_reason.is_some() && !t.in_flight() {
            t.block_reason = None;
            if !t.attempts.is_empty() {
                t.retry = Some(reason.trim().to_string());
            }
            t.touch(now_ms);
            self.changed(now_ms);
            return Ok(());
        }
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
        self.changed(now_ms);
        Ok(())
    }

    /// Attempts in flight with their work sessions.
    pub fn in_flight(&self) -> Vec<(String, u32, String)> {
        self.todos
            .iter()
            .filter_map(|t| {
                let a = t.running()?;
                Some((t.todo_id.clone(), a.attempt, a.child_sid.clone().unwrap_or_default()))
            })
            .collect()
    }

    pub fn summary(&self) -> TodoSummary {
        let mut s = TodoSummary {
            total: self.todos.len(),
            ..Default::default()
        };
        for t in &self.todos {
            *s.counts.entry(t.status.as_str().to_string()).or_default() += 1;
            let id = t.todo_id.clone();
            match t.status {
                TodoStatus::Running => s.running.push(id.clone()),
                TodoStatus::Pending => s.pending.push(id.clone()),
                TodoStatus::Blocked => s.blocked.push(id.clone()),
                _ => {}
            }
            if t.latest().is_some_and(TodoAttempt::paused) {
                s.paused.push(id);
            }
        }
        s
    }

    /// The compact view the plan decides on: no execution detail.
    pub fn render_view(&self) -> String {
        let mut s = format!("<todo_list rev=\"{}\">\n", self.rev);
        for t in &self.todos {
            s.push_str(&t.view_line(self));
            s.push('\n');
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

fn runtime_arg(runtime: Option<String>) -> Option<String> {
    runtime
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty() && r != "inherit")
}

fn workspace_arg(workspace: Option<String>) -> Result<Option<String>, AgentToolError> {
    let workspace = workspace.map(|w| w.trim().to_string()).filter(|w| !w.is_empty());
    if let Some(w) = &workspace {
        if w != "inherit" && w != "none" && !w.starts_with("ws-") {
            return Err(invalid(format!(
                "--workspace {w}: takes inherit | none | <workspace id>"
            )));
        }
    }
    Ok(workspace)
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

    fn free(title: &str) -> NewTodo {
        NewTodo {
            after: Some(Vec::new()),
            ..todo(title)
        }
    }

    fn done(status: TodoStatus, summary: &str) -> ChildObservation {
        ChildObservation {
            status,
            summary: Some(summary.into()),
            report: Some(ReportRef {
                session_id: "work-s".into(),
                report_id: format!("r-{summary}"),
            }),
            artifacts: Vec::new(),
            error: None,
        }
    }

    /// Dispatch `id` as `child` and bind it.
    fn start(l: &mut TodoList, id: &str, child: &str, now: u64) -> u32 {
        let n = l.begin_dispatch(id, child, now).unwrap();
        l.bind_dispatch(id, n, now, AttemptTarget::default()).unwrap();
        n
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
        assert!(list.todos[2].after.is_empty(), "cancelled todos are no default prerequisite");
        assert_eq!(list.next_seq, 4);
    }

    /// Without `--after` a Todo runs after the latest one; `--after none`
    /// runs at once; what else runs never holds a ready Todo back.
    #[test]
    fn a_chain_by_default_and_ready_todos() {
        let mut l = TodoList::default();
        let a = l.add(todo("a"), 1).unwrap();
        let b = l.add(todo("b"), 1).unwrap();
        let c = l.add(free("c"), 1).unwrap();
        let d = l.add(todo("d"), 1).unwrap();
        let e = l
            .add(NewTodo { after: Some(vec![a.clone(), c.clone()]), mode: TodoMode::Explore, ..todo("e") }, 1)
            .unwrap();
        assert!(l.get(&a).unwrap().after.is_empty());
        assert_eq!(l.get(&b).unwrap().after, vec![a.clone()]);
        assert!(l.get(&c).unwrap().after.is_empty());
        assert_eq!(l.get(&d).unwrap().after, vec![c.clone()]);
        assert_eq!(l.ready(), vec![a.clone(), c.clone()]);
        let n = start(&mut l, &a, "work-a", 2);
        assert_eq!(l.ready(), vec![c.clone()], "a running todo is not ready again; c is not held back");
        assert!(l.admit(&b).unwrap_err().to_string().contains("did not complete yet"));
        l.observe_child(&a, n, done(TodoStatus::Completed, "a done"), 3).unwrap();
        assert_eq!(l.ready(), vec![b.clone(), c.clone()], "completed is enough: no review");
        start(&mut l, &c, "work-c", 4);
        l.observe_child(&c, 1, done(TodoStatus::Completed, "c done"), 5).unwrap();
        assert_eq!(l.ready(), vec![b.clone(), d.clone(), e.clone()]);
        let text = l.get(&e).unwrap().task_text(&l, 1);
        assert!(
            text.contains("- T01 a: a done (report work-s#r-a done; read it with `agent-session read-session work-s --report`)"),
            "{text}"
        );
        assert!(l.add(NewTodo { after: Some(vec!["T99".into()]), ..todo("x") }, 6).is_err());
    }

    /// A prerequisite that ends without success blocks the Todos after it
    /// until the plan retries it; its success unblocks them.
    #[test]
    fn a_failed_prerequisite_blocks_until_it_is_retried() {
        let mut l = TodoList::default();
        let a = l.add(todo("a"), 1).unwrap();
        let b = l.add(todo("b"), 1).unwrap();
        let c = l.add(todo("c"), 1).unwrap();
        let first = start(&mut l, &a, "work-a1", 2);
        l.observe_child(&a, first, done(TodoStatus::Failed, "no evidence"), 3).unwrap();
        assert!(!l.observe_child(&a, first, done(TodoStatus::Completed, "late"), 3).unwrap());
        let tb = l.get(&b).unwrap();
        assert_eq!(tb.status, TodoStatus::Blocked);
        assert_eq!(tb.blocked_by, vec![a.clone()]);
        assert!(tb.view_line(&l).contains("blocked_by=\"T01:failed\""), "{}", tb.view_line(&l));
        assert_eq!(l.get(&c).unwrap().status, TodoStatus::Pending, "only the direct dependents are blocked");
        assert!(l.ready().is_empty());
        assert!(l.retry(&a, "", 3).is_err());
        l.retry(&a, "use the public entry", 4).unwrap();
        assert_eq!(l.get(&a).unwrap().status, TodoStatus::Pending);
        assert_eq!(l.get(&b).unwrap().status, TodoStatus::Pending, "unblocked by the retry");
        assert_eq!(l.ready(), vec![a.clone()]);
        let second = start(&mut l, &a, "work-a2", 5);
        assert_eq!(second, 2);
        let t = l.get(&a).unwrap();
        assert_eq!(t.attempt(2).unwrap().reason.as_deref(), Some("use the public entry"));
        assert_eq!(t.attempt(1).unwrap().summary.as_deref(), Some("no evidence"));
        assert!(l.cancel(&a, "running", 6).unwrap_err().to_string().contains("todo stop"));
        l.observe_child(&a, second, done(TodoStatus::Completed, "verified"), 7).unwrap();
        assert_eq!(l.ready(), vec![b.clone()]);
        // A cancelled prerequisite blocks too; changing `after` frees it.
        let d = l.add(todo("d"), 8).unwrap();
        assert_eq!(l.get(&d).unwrap().after, vec![c.clone()]);
        l.cancel(&c, "not needed", 9).unwrap();
        assert_eq!(l.get(&d).unwrap().status, TodoStatus::Blocked);
        assert_eq!(l.get(&d).unwrap().blocked_by, vec![c.clone()]);
        l.update(&d, TodoPatch { after: Some(vec![]), ..Default::default() }, "c is gone", None, 11)
            .unwrap();
        assert!(l.ready().contains(&d));
    }

    /// A dispatch held back by a limit leaves the Todo pending; one that
    /// fails blocks it with the reason until it is changed or retried.
    #[test]
    fn withdrawn_dispatches() {
        let mut l = TodoList::default();
        let a = l.add(NewTodo { mode: TodoMode::Explore, ..free("a") }, 1).unwrap();
        let n = l.begin_dispatch(&a, "work-a1", 2).unwrap();
        l.withdraw_dispatch(&a, n, None, 3).unwrap();
        let t = l.get(&a).unwrap();
        assert_eq!(t.status, TodoStatus::Pending);
        assert!(t.attempts.is_empty());
        assert_eq!(l.begin_dispatch(&a, "work-a1", 4).unwrap(), 1);
        l.withdraw_dispatch(&a, 1, Some("runtime r9 is not registered"), 5).unwrap();
        let t = l.get(&a).unwrap();
        assert_eq!(t.status, TodoStatus::Blocked);
        assert!(t.view_line(&l).contains("dispatch failed: runtime r9 is not registered"));
        assert!(l.ready().is_empty());
        l.update(&a, TodoPatch { runtime_id: Some("inherit".into()), content: Some("look again".into()), ..Default::default() }, "the session's runtime", None, 6)
            .unwrap();
        assert_eq!(l.get(&a).unwrap().status, TodoStatus::Pending);
        assert_eq!(l.ready(), vec![a.clone()]);
        let n = l.begin_dispatch(&a, "work-a2", 7).unwrap();
        l.withdraw_dispatch(&a, n, Some("no such workspace"), 8).unwrap();
        l.retry(&a, "workspace registered now", 9).unwrap();
        assert_eq!(l.get(&a).unwrap().status, TodoStatus::Pending);
        let view = {
            start(&mut l, &a, "work-a3", 10);
            l.render_view()
        };
        assert!(view.contains("id=\"T01\" mode=\"explore\" status=\"running\" attempt=\"1\" child_sid=\"work-a3\""), "{view}");
    }

    /// The body of a running Todo changes (its work session is told);
    /// where and in which order it runs does not; every change is kept.
    #[test]
    fn updates_and_their_history() {
        let mut l = TodoList::default();
        let a = l.add(todo("a"), 1).unwrap();
        let b = l.add(todo("b"), 1).unwrap();
        start(&mut l, &a, "work-a", 2);
        let err = l
            .update(&a, TodoPatch { after: Some(vec![]), ..Default::default() }, "x", None, 3)
            .unwrap_err()
            .to_string();
        assert!(err.contains("is running") && err.contains("after"), "{err}");
        assert!(l.update(&a, TodoPatch { content: Some("x".into()), ..Default::default() }, " ", None, 3).is_err());
        let out = l
            .update(
                &a,
                TodoPatch {
                    content: Some("do a, in Rust".into()),
                    accept: vec!["tests pass".into()],
                    ..Default::default()
                },
                "the user wants Rust",
                Some("did:user".into()),
                4,
            )
            .unwrap();
        assert_eq!(out.changed, vec!["content", "accept"]);
        assert_eq!(out.running, Some((1, "work-a".to_string())));
        let t = l.get(&a).unwrap();
        assert_eq!(t.updates.len(), 1);
        assert_eq!(t.updates[0].attempt, Some(1));
        assert!(t.task_text(&l, 1).contains("do a, in Rust\n\nAcceptance criteria:\n- tests pass"));
        let out = l
            .update(&a, TodoPatch { accept: vec!["documented".into()], clear_accept: true, ..Default::default() }, "only docs", None, 5)
            .unwrap();
        assert_eq!(out.changed, vec!["accept"]);
        assert_eq!(l.get(&a).unwrap().accept, vec!["documented"]);
        assert!(l.update(&a, TodoPatch { title: Some("a".into()), ..Default::default() }, "same", None, 6).is_err(), "nothing changed");
        // A cycle through `after` is refused.
        let err = l
            .update(&a, TodoPatch { content: Some("y".into()), ..Default::default() }, "y", None, 7)
            .and_then(|_| l.update(&b, TodoPatch { after: Some(vec![b.clone()]), ..Default::default() }, "z", None, 8))
            .unwrap_err()
            .to_string();
        assert!(err.contains("after itself"), "{err}");
        let pending = l
            .update(&b, TodoPatch { workspace: Some("none".into()), ..Default::default() }, "read only", None, 9)
            .unwrap();
        assert_eq!(pending.running, None);
        assert_eq!(l.get(&b).unwrap().workspace.as_deref(), Some("none"));
        assert!(l.update(&b, TodoPatch { workspace: Some("bad".into()), ..Default::default() }, "x", None, 9).is_err());
    }

    /// Pause / resume mark the running attempt; a Todo without one is
    /// refused; the summary counts states without judging them.
    #[test]
    fn pause_marks_and_summary() {
        let mut l = TodoList::default();
        let a = l.add(todo("a"), 1).unwrap();
        let b = l.add(todo("b"), 1).unwrap();
        let c = l.add(free("c"), 1).unwrap();
        assert!(l.mark_paused(&b, true, 2).is_err());
        start(&mut l, &a, "work-a", 2);
        assert_eq!(l.mark_paused(&a, true, 3).unwrap(), Some((1, "work-a".to_string())));
        assert_eq!(l.mark_paused(&a, true, 4).unwrap(), None, "already paused");
        assert!(l.get(&a).unwrap().view_line(&l).contains("paused=\"true\""));
        let s = l.summary();
        assert_eq!(s.paused, vec![a.clone()]);
        assert_eq!(s.line(), "3 todos: 2 pending (T02, T03), 1 running (T01); paused: T01");
        assert_eq!(l.mark_paused(&a, false, 5).unwrap(), Some((1, "work-a".to_string())));
        assert!(!l.get(&a).unwrap().latest().unwrap().paused());
        l.observe_child(&a, 1, done(TodoStatus::Stopped, "stopped"), 6).unwrap();
        l.cancel(&c, "dropped", 7).unwrap();
        let s = l.summary();
        assert_eq!(s.blocked, vec![b.clone()]);
        assert_eq!(s.line(), "3 todos: 1 blocked (T02), 1 cancelled, 1 stopped");
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
