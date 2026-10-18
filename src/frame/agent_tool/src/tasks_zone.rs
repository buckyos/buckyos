//! `shell` tasks in the zone's TaskMgr (`tool.shell/v1`): the task manager a
//! hosted agent (OpenDAN's Loader) hands long commands to instead of the
//! process's own one (`notepads/llm-context-long-tool-todo.md` §4).
//!
//! The command still runs in this process (its handle stays here); TaskMgr
//! holds the task's identity, state and result. So any process of the host
//! can answer for the task (a later run, a later drive, a restarted
//! Loader), the owner sees it in the task tree under the Turn it belongs
//! to, and a cancel requested there reaches the command. A watcher per
//! task reports progress, applies cancels and commits the end; a process
//! that restarts takes the tasks it left running back from their execution
//! directories ([`ZoneShellTasks::recover`]).
//!
//! When TaskMgr cannot be reached at handover the command is not lost: it
//! goes to the process's own task manager (`local:` id).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use async_trait::async_trait;
use llm_context::observation::ToolResultView;
use llm_context::tasks::{
    CancelUnsupported, RunningTaskResolver, TaskBrief, TaskResult, TaskState,
    MAX_IN_TOOL_WAIT_MS,
};
use serde_json::{json, Value as Json};

use crate::llm_bash::{exec_dir_for, TOOL_SHELL};
use crate::tasks::{
    compact, exec_dir_state, InProcessTaskManager, LocalTask, ShellTask, ShellTaskContext,
    ShellTaskManager,
};

/// Prefix of every TaskMgr task id (`t-<32 hex>`).
pub const ZONE_TASK_PREFIX: &str = "t-";
/// How often a running command's progress is written to TaskMgr (and its
/// pending cancel read).
const PROGRESS_EVERY_MS: u64 = 5_000;
/// How long one observation of a running command waits.
const POLL_MS: u64 = 2_000;
/// Output kept in the task's result.
const RESULT_TAIL_BYTES: usize = 8 * 1024;
/// A recovered task is observed through its execution directory this often.
const RECOVER_POLL: Duration = Duration::from_secs(5);
const WRITE_RETRIES: usize = 3;

/// A `tool.shell` task as this module reads it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShellTaskRecord {
    pub task_id: String,
    pub name: String,
    pub input: Json,
    pub terminal: bool,
    /// `succeeded` | `failed` | `canceled` once terminal.
    pub outcome: Option<String>,
    pub result: Option<Json>,
    /// `{code, message, detail}` of a failed task.
    pub error: Option<Json>,
    pub message: Option<String>,
    pub progress: Option<Json>,
    /// A cancel waits for the runner: its request id.
    pub cancel_requested: Option<String>,
    pub parent_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ShellTaskCreate {
    pub name: String,
    pub input: Json,
    pub parent_id: Option<String>,
    pub idempotency_key: String,
    /// User granted read and control on a root task (the agent's owner).
    pub grant_user: Option<String>,
}

/// TaskMgr, as far as `shell` tasks need it.
#[async_trait]
pub trait ShellTaskBackend: Send + Sync {
    /// Idempotent for the same `idempotency_key`.
    async fn create(&self, req: ShellTaskCreate) -> Result<ShellTaskRecord, String>;
    /// `None`: no such task.
    async fn get(&self, task_id: &str) -> Result<Option<ShellTaskRecord>, String>;
    /// Running, with `message` / `progress` as the display snapshot.
    async fn progress(&self, task_id: &str, message: &str, progress: Json) -> Result<(), String>;
    async fn complete(&self, task_id: &str, result: Json) -> Result<(), String>;
    async fn fail(
        &self,
        task_id: &str,
        code: &str,
        message: &str,
        detail: Option<Json>,
    ) -> Result<(), String>;
    /// Ask for a cancel (the runner applies it).
    async fn request_cancel(&self, task_id: &str) -> Result<(), String>;
    /// Close as canceled: acknowledge the pending cancel, requesting one
    /// first when the cancel did not come from TaskMgr.
    async fn canceled(&self, task_id: &str) -> Result<(), String>;
    /// Every `tool.shell` task of the zone that is not terminal.
    async fn list_open(&self) -> Result<Vec<ShellTaskRecord>, String>;
}

struct Running {
    task: Arc<ShellTask>,
    session_id: Option<String>,
    brief: String,
    started_at_ms: u64,
}

struct Finished {
    result: TaskResult,
    session_id: Option<String>,
    brief: String,
    started_at_ms: u64,
}

#[derive(Default)]
struct State {
    running: BTreeMap<String, Arc<Running>>,
    /// Ended in this process: the result the model reads (TaskMgr holds the
    /// same).
    finished: BTreeMap<String, Finished>,
    /// Finished tasks whose result the model read once: no longer shown in
    /// the background env, not delivered again after the run.
    read: BTreeSet<String>,
}

/// The `shell` task manager of a host whose tasks live in TaskMgr.
pub struct ZoneShellTasks {
    backend: Arc<dyn ShellTaskBackend>,
    /// Whose tasks these are (`input.agent`): what `recover` takes back.
    agent: String,
    owner_user: Option<String>,
    fallback: Arc<InProcessTaskManager>,
    inner: Mutex<State>,
    me: Weak<Self>,
}

impl ZoneShellTasks {
    /// `agent`: the agent the tasks belong to (its DID); `owner_user`: the
    /// user granted read and control on root tasks; `fallback`: where a
    /// command goes when TaskMgr cannot be reached at handover.
    pub fn new(
        backend: Arc<dyn ShellTaskBackend>,
        agent: &str,
        owner_user: Option<String>,
        fallback: Arc<InProcessTaskManager>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            backend,
            agent: agent.to_string(),
            owner_user,
            fallback,
            inner: Mutex::new(State::default()),
            me: me.clone(),
        })
    }

    pub fn is_zone_task(task_id: &str) -> bool {
        task_id.starts_with(ZONE_TASK_PREFIX)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.inner.lock().expect("zone shell tasks")
    }

    fn mark_read(&self, task_id: &str) {
        self.lock().read.insert(task_id.to_string());
    }

    /// Take over `task` for `ctx`; the TaskMgr task id, or the `local:` id
    /// of the fallback.
    async fn hand_over(
        self: &Arc<Self>,
        ctx: &ShellTaskContext,
        call_id: Option<&str>,
        task: ShellTask,
    ) -> String {
        let binding = ctx.run_binding();
        let exec_dir = exec_dir_for(
            binding.as_ref().and_then(|b| b.run_dir.as_deref()),
            call_id,
        );
        let started_at_ms = task.started_at_ms();
        let brief = task.brief();
        let input = json!({
            "command": task.command(),
            "runtime": task.runtime(),
            "call_id": call_id,
            "run_id": binding.as_ref().map(|b| b.run_id.clone()),
            "session_id": ctx.session_id,
            "agent": self.agent,
            "exec_dir": exec_dir.as_ref().map(|p| p.display().to_string()),
            "locator": task.locator(),
            "started_at_ms": started_at_ms,
        });
        let idempotency_key = format!(
            "shell:{}:{}",
            binding.as_ref().map(|b| b.run_id.as_str()).unwrap_or("anon"),
            call_id
                .filter(|c| !c.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("anon-{}", crate::tasks::next_local_seq()))
        );
        let parent_task = ctx.parent_task();
        let created = self
            .backend
            .create(ShellTaskCreate {
                name: compact(task.command(), 120),
                input,
                grant_user: if parent_task.is_none() {
                    self.owner_user.clone()
                } else {
                    None
                },
                parent_id: parent_task,
                idempotency_key,
            })
            .await;
        let record = match created {
            Ok(r) => r,
            Err(e) => {
                log::warn!(
                    "TaskMgr did not take `{}`: {e}; it stays in this process",
                    compact(task.command(), 80)
                );
                return self.fallback.register_shell(call_id, task);
            }
        };
        let running = Arc::new(Running {
            task: Arc::new(task),
            session_id: ctx.session_id.clone(),
            brief,
            started_at_ms,
        });
        {
            let mut s = self.lock();
            s.read.remove(&record.task_id);
            s.finished.remove(&record.task_id);
            s.running.insert(record.task_id.clone(), running.clone());
        }
        tokio::spawn(self.clone().watch(record.task_id.clone(), running));
        record.task_id
    }

    /// Follow a command this process runs: progress and pending cancels
    /// every `PROGRESS_EVERY_MS`, the end when it comes.
    async fn watch(self: Arc<Self>, task_id: String, run: Arc<Running>) {
        let mut last_report: u64 = 0;
        loop {
            let state = run.task.wait(Some(crate::now_ms() + POLL_MS)).await;
            match state {
                TaskState::Running {
                    output_tail,
                    elapsed_ms,
                    ..
                } => {
                    let now = crate::now_ms();
                    if now.saturating_sub(last_report) < PROGRESS_EVERY_MS {
                        continue;
                    }
                    last_report = now;
                    match self.backend.get(&task_id).await {
                        Ok(Some(rec)) if rec.cancel_requested.is_some() || rec.terminal => {
                            log::info!(
                                "task {task_id}: {} in TaskMgr, stopping `{}`",
                                if rec.terminal { "closed" } else { "cancel requested" },
                                compact(run.task.command(), 80)
                            );
                            let ended = match run.task.cancel().await {
                                Ok(TaskState::Finished(r)) => r,
                                Ok(other) => {
                                    log::warn!("task {task_id}: cancel left it {other:?}");
                                    continue;
                                }
                                Err(_) => {
                                    // Not cancellable: TaskMgr is told, the
                                    // command runs on.
                                    continue;
                                }
                            };
                            self.settle(&task_id, &run, ended, true).await;
                            return;
                        }
                        Ok(None) => {
                            log::warn!("task {task_id} vanished from TaskMgr; `{}` runs on in this process", compact(run.task.command(), 80));
                        }
                        Ok(Some(_)) => {}
                        Err(e) => log::debug!("task {task_id}: TaskMgr not read: {e}"),
                    }
                    let line = last_line(&output_tail);
                    if let Err(e) = self
                        .backend
                        .progress(
                            &task_id,
                            &line,
                            json!({ "elapsed_ms": elapsed_ms, "output_tail": output_tail }),
                        )
                        .await
                    {
                        log::debug!("task {task_id}: progress not written: {e}");
                    }
                }
                TaskState::Finished(result) => {
                    let cancelled = was_cancelled(&result);
                    self.settle(&task_id, &run, result, cancelled).await;
                    return;
                }
                TaskState::Unknown { reason } => {
                    let result = TaskResult {
                        success: false,
                        output: format!("`{}` cannot be observed any more: {reason}", run.task.command()),
                        tool_result: None,
                    };
                    self.settle(&task_id, &run, result, false).await;
                    return;
                }
            }
        }
    }

    /// The command ended: remembered here first (the model reads it even
    /// when TaskMgr is unreachable), then written to TaskMgr.
    async fn settle(&self, task_id: &str, run: &Running, result: TaskResult, cancelled: bool) {
        {
            let mut s = self.lock();
            s.running.remove(task_id);
            s.finished.insert(
                task_id.to_string(),
                Finished {
                    result: result.clone(),
                    session_id: run.session_id.clone(),
                    brief: run.brief.clone(),
                    started_at_ms: run.started_at_ms,
                },
            );
        }
        let exit_code = result.tool_result.as_ref().and_then(|t| t.return_code);
        let duration_ms = result
            .tool_result
            .as_ref()
            .and_then(|t| t.detail.as_ref())
            .and_then(|d| d.get("duration_ms"))
            .and_then(Json::as_u64)
            .unwrap_or_else(|| crate::now_ms().saturating_sub(run.started_at_ms));
        let payload = json!({
            "success": result.success,
            "exit_code": exit_code,
            "duration_ms": duration_ms,
            "cancelled": cancelled,
            "command": run.task.command(),
            "output": tail(&result.output, RESULT_TAIL_BYTES),
        });
        let mut last_err = None;
        for attempt in 0..WRITE_RETRIES {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_secs(2 << attempt)).await;
            }
            let written = if cancelled {
                self.backend.canceled(task_id).await
            } else if result.success {
                self.backend.complete(task_id, payload.clone()).await
            } else {
                let code = match exit_code {
                    Some(c) => format!("exit_{c}"),
                    None => "failed".to_string(),
                };
                let message = match exit_code {
                    Some(c) => format!(
                        "`{}` exited with code {c}",
                        compact(run.task.command(), 80)
                    ),
                    None => first_line(&result.output),
                };
                self.backend
                    .fail(task_id, &code, &message, Some(payload.clone()))
                    .await
            };
            match written {
                Ok(()) => return,
                Err(e) => last_err = Some(e),
            }
        }
        log::warn!(
            "task {task_id}: end not written to TaskMgr: {}",
            last_err.unwrap_or_default()
        );
    }

    /// The tasks a previous process of this host left open: those whose
    /// execution directory still exists are observed through it until the
    /// command's `exit` is recorded; the others are failed.
    pub async fn recover(&self) {
        let open = match self.backend.list_open().await {
            Ok(v) => v,
            Err(e) => {
                log::warn!("shell tasks not recovered: TaskMgr not listed: {e}");
                return;
            }
        };
        let Some(me) = self.me.upgrade() else { return };
        for rec in open {
            if rec.input.get("agent").and_then(Json::as_str) != Some(self.agent.as_str()) {
                continue;
            }
            if self.lock().running.contains_key(&rec.task_id) {
                continue;
            }
            let dir = rec
                .input
                .get("exec_dir")
                .and_then(Json::as_str)
                .map(PathBuf::from);
            match dir {
                Some(dir) if dir.is_dir() => {
                    log::info!(
                        "task {}: `{}` left by a previous process, observed through {}",
                        rec.task_id,
                        compact(command_of(&rec), 80),
                        dir.display()
                    );
                    tokio::spawn(me.clone().recover_watch(rec, dir));
                }
                _ => {
                    log::info!(
                        "task {}: `{}` left by a previous process has no execution directory; failed",
                        rec.task_id,
                        compact(command_of(&rec), 80)
                    );
                    let _ = self
                        .backend
                        .fail(
                            &rec.task_id,
                            "executor_restarted",
                            "the process that ran this command restarted and has no record of it; it may have completed with side effects",
                            None,
                        )
                        .await;
                }
            }
        }
    }

    async fn recover_watch(self: Arc<Self>, rec: ShellTaskRecord, dir: PathBuf) {
        loop {
            match exec_dir_state(&rec.task_id, &dir) {
                TaskState::Finished(result) => {
                    let run = Running {
                        task: Arc::new(ShellTask::new(
                            command_of(&rec),
                            Box::new(crate::llm_bash::FinishedCommand(None)),
                            rec.input
                                .get("runtime")
                                .and_then(Json::as_str)
                                .unwrap_or("native"),
                        )),
                        session_id: rec
                            .input
                            .get("session_id")
                            .and_then(Json::as_str)
                            .map(str::to_string),
                        brief: format!("shell: {}", compact(command_of(&rec), 80)),
                        started_at_ms: rec
                            .input
                            .get("started_at_ms")
                            .and_then(Json::as_u64)
                            .unwrap_or_else(crate::now_ms),
                    };
                    self.settle(&rec.task_id, &run, result, false).await;
                    return;
                }
                TaskState::Unknown { .. } if dir.is_dir() => {
                    match self.backend.get(&rec.task_id).await {
                        Ok(Some(t)) if t.terminal => return,
                        Ok(None) => return,
                        _ => {}
                    }
                    tokio::time::sleep(RECOVER_POLL).await;
                }
                _ => {
                    let _ = self
                        .backend
                        .fail(
                            &rec.task_id,
                            "record_lost",
                            "the execution directory of this command disappeared before its exit was recorded",
                            None,
                        )
                        .await;
                    return;
                }
            }
        }
    }

    /// The state of a task of TaskMgr, from this process when it runs or
    /// ended here, else from TaskMgr. A finished result is marked read.
    async fn state_of(&self, task_id: &str) -> TaskState {
        let (running, finished) = {
            let s = self.lock();
            (
                s.running.get(task_id).cloned(),
                s.finished.get(task_id).map(|f| f.result.clone()),
            )
        };
        if let Some(run) = running {
            let st = run.task.state().await;
            if matches!(st, TaskState::Finished(_)) {
                self.mark_read(task_id);
            }
            return st;
        }
        if let Some(result) = finished {
            self.mark_read(task_id);
            return TaskState::Finished(result);
        }
        self.state_from_backend(task_id).await
    }

    async fn state_from_backend(&self, task_id: &str) -> TaskState {
        match self.backend.get(task_id).await {
            Ok(Some(rec)) if rec.terminal => {
                self.mark_read(task_id);
                TaskState::Finished(result_of(&rec))
            }
            Ok(Some(rec)) => {
                let started = rec
                    .input
                    .get("started_at_ms")
                    .and_then(Json::as_u64)
                    .unwrap_or_else(crate::now_ms);
                TaskState::Running {
                    brief: format!("shell: {}", compact(command_of(&rec), 80)),
                    elapsed_ms: crate::now_ms().saturating_sub(started),
                    output_tail: rec
                        .progress
                        .as_ref()
                        .and_then(|p| p.get("output_tail"))
                        .and_then(Json::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    cancellable: rec.cancel_requested.is_none(),
                }
            }
            Ok(None) => TaskState::Unknown {
                reason: format!("task {task_id} is not known to TaskMgr"),
            },
            Err(e) => TaskState::Unknown {
                reason: format!("TaskMgr is not reachable: {e}"),
            },
        }
    }

    async fn briefs(&self, session_id: Option<&str>) -> Vec<TaskBrief> {
        let (running, finished): (Vec<(String, Arc<Running>)>, Vec<(String, String, u64, TaskResult)>) = {
            let s = self.lock();
            let keep = |sid: &Option<String>| match session_id {
                Some(want) => sid.as_deref() == Some(want),
                None => true,
            };
            (
                s.running
                    .iter()
                    .filter(|(id, r)| !s.read.contains(*id) && keep(&r.session_id))
                    .map(|(id, r)| (id.clone(), r.clone()))
                    .collect(),
                s.finished
                    .iter()
                    .filter(|(id, f)| !s.read.contains(*id) && keep(&f.session_id))
                    .map(|(id, f)| (id.clone(), f.brief.clone(), f.started_at_ms, f.result.clone()))
                    .collect(),
            )
        };
        let mut out = Vec::new();
        for (id, run) in running {
            let (status, elapsed_ms, last) = match run.task.state().await {
                TaskState::Running {
                    elapsed_ms,
                    output_tail,
                    ..
                } => ("running", elapsed_ms, last_line(&output_tail)),
                TaskState::Finished(r) => (
                    if r.success { "finished" } else { "failed" },
                    crate::now_ms().saturating_sub(run.started_at_ms),
                    r.tool_result.as_ref().map(|t| t.summary.clone()).unwrap_or_default(),
                ),
                TaskState::Unknown { reason } => ("unknown", 0, reason),
            };
            out.push(TaskBrief {
                task_id: id,
                brief: run.brief.clone(),
                status: status.into(),
                elapsed_ms,
                last_line: compact(&last, 120),
                cancellable: run.task.cancellable(),
            });
        }
        for (id, brief, started_at_ms, r) in finished {
            out.push(TaskBrief {
                task_id: id,
                brief,
                status: if r.success { "finished" } else { "failed" }.into(),
                elapsed_ms: crate::now_ms().saturating_sub(started_at_ms),
                last_line: compact(
                    &r.tool_result.as_ref().map(|t| t.summary.clone()).unwrap_or_default(),
                    120,
                ),
                cancellable: false,
            });
        }
        out
    }
}

#[async_trait]
impl ShellTaskManager for ZoneShellTasks {
    async fn take_over(
        &self,
        ctx: &ShellTaskContext,
        call_id: Option<&str>,
        task: ShellTask,
    ) -> String {
        match self.me.upgrade() {
            Some(me) => me.hand_over(ctx, call_id, task).await,
            None => self.fallback.register_shell(call_id, task),
        }
    }

    fn resolver(&self, session_id: Option<&str>) -> Option<Arc<dyn RunningTaskResolver>> {
        let me = self.me.upgrade()?;
        Some(match session_id {
            Some(sid) => Arc::new(SessionScoped {
                inner: me,
                session_id: sid.to_string(),
            }),
            None => me,
        })
    }

    async fn recover(&self) {
        ZoneShellTasks::recover(self).await
    }
}

#[async_trait]
impl RunningTaskResolver for ZoneShellTasks {
    async fn state(&self, task_id: &str) -> TaskState {
        self.state_of(task_id).await
    }

    async fn wait(&self, task_id: &str, until_ms: Option<u64>) -> TaskState {
        let until = until_ms.unwrap_or_else(|| crate::now_ms() + MAX_IN_TOOL_WAIT_MS);
        let running = self.lock().running.get(task_id).cloned();
        if let Some(run) = running {
            let st = run.task.wait(Some(until)).await;
            if matches!(st, TaskState::Finished(_)) {
                self.mark_read(task_id);
            }
            return st;
        }
        loop {
            let st = self.state_of(task_id).await;
            if !matches!(st, TaskState::Running { .. }) || crate::now_ms() >= until {
                return st;
            }
            tokio::time::sleep(Duration::from_millis(1_000)).await;
        }
    }

    async fn cancel(&self, task_id: &str) -> Result<TaskState, CancelUnsupported> {
        let running = self.lock().running.get(task_id).cloned();
        if let Some(run) = running {
            // The watcher writes the end to TaskMgr.
            return run.task.cancel().await.map_err(|_| CancelUnsupported {
                task_id: task_id.to_string(),
            });
        }
        if self.lock().finished.contains_key(task_id) {
            return Ok(self.state_of(task_id).await);
        }
        // Run by another process of the host: it applies the cancel.
        self.backend
            .request_cancel(task_id)
            .await
            .map_err(|_| CancelUnsupported {
                task_id: task_id.to_string(),
            })?;
        Ok(self.state_from_backend(task_id).await)
    }

    fn can_resolve(&self, task_id: &str) -> bool {
        Self::is_zone_task(task_id)
    }

    async fn active(&self) -> Vec<TaskBrief> {
        self.briefs(None).await
    }

    fn result_seen(&self, task_id: &str) -> bool {
        self.lock().read.contains(task_id)
    }
}

/// The resolver of one session: `active()` lists its tasks only.
struct SessionScoped {
    inner: Arc<ZoneShellTasks>,
    session_id: String,
}

#[async_trait]
impl RunningTaskResolver for SessionScoped {
    async fn state(&self, task_id: &str) -> TaskState {
        self.inner.state(task_id).await
    }
    async fn wait(&self, task_id: &str, until_ms: Option<u64>) -> TaskState {
        self.inner.wait(task_id, until_ms).await
    }
    async fn cancel(&self, task_id: &str) -> Result<TaskState, CancelUnsupported> {
        self.inner.cancel(task_id).await
    }
    fn can_resolve(&self, task_id: &str) -> bool {
        self.inner.can_resolve(task_id)
    }
    async fn active(&self) -> Vec<TaskBrief> {
        self.inner.briefs(Some(&self.session_id)).await
    }
    fn result_seen(&self, task_id: &str) -> bool {
        self.inner.result_seen(task_id)
    }
}

fn command_of(rec: &ShellTaskRecord) -> &str {
    rec.input
        .get("command")
        .and_then(Json::as_str)
        .unwrap_or(rec.name.as_str())
}

fn was_cancelled(result: &TaskResult) -> bool {
    result
        .tool_result
        .as_ref()
        .and_then(|t| t.detail.as_ref())
        .and_then(|d| d.get("cancelled"))
        .and_then(Json::as_bool)
        .unwrap_or(false)
}

fn last_line(tail: &str) -> String {
    tail.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .to_string()
}

fn first_line(text: &str) -> String {
    compact(text.lines().next().unwrap_or(""), 160)
}

fn tail(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut start = text.len() - max;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &text[start..])
}

/// The result of a terminal task, from what TaskMgr holds.
fn result_of(rec: &ShellTaskRecord) -> TaskResult {
    let command = command_of(rec).to_string();
    let detail = rec
        .result
        .clone()
        .or_else(|| rec.error.as_ref().and_then(|e| e.get("detail").cloned()))
        .unwrap_or(Json::Null);
    let exit_code = detail.get("exit_code").and_then(Json::as_i64).map(|c| c as i32);
    let duration_ms = detail.get("duration_ms").and_then(Json::as_u64).unwrap_or(0);
    let output = detail.get("output").and_then(Json::as_str).unwrap_or("");
    let outcome = rec.outcome.as_deref().unwrap_or("failed");
    let mut text = match (outcome, exit_code) {
        ("canceled", _) => format!("`{command}` was cancelled after {duration_ms}ms."),
        (_, Some(code)) => format!("`{command}` exited with code {code} after {duration_ms}ms."),
        ("succeeded", None) => format!("`{command}` completed."),
        (_, None) => format!(
            "`{command}` failed: {}",
            rec.error
                .as_ref()
                .and_then(|e| e.get("message"))
                .and_then(Json::as_str)
                .unwrap_or("no detail")
        ),
    };
    if !output.trim().is_empty() {
        text.push_str("\n--- output ---\n");
        text.push_str(output.trim_end());
    }
    let summary = match (outcome, exit_code) {
        ("canceled", _) => "cancelled".to_string(),
        (_, Some(code)) => format!("exit={code} in {duration_ms}ms"),
        _ => outcome.to_string(),
    };
    TaskResult {
        success: outcome == "succeeded",
        output: text,
        tool_result: Some(ToolResultView {
            cmd_name: Some(TOOL_SHELL.into()),
            cmd_args: Some(compact(&command, 80)),
            return_code: exit_code,
            summary,
            detail: Some(detail),
            ..Default::default()
        }),
    }
}

// ---------------------------------------------------------------------------
// TaskMgr of the zone
// ---------------------------------------------------------------------------

/// TaskMgr of the zone this process runs in: the client is taken from the
/// runtime for every call (its session token is renewed).
pub struct ZoneShellTaskBackend;

#[cfg(not(target_arch = "wasm32"))]
mod zone {
    use super::*;
    use buckyos_api::{
        AckControlReq, CreateTaskExecutor, CreateTaskReq, GrantTaskAccessReq, ListTasksReq,
        RequestControlReq, RunnerWriteEnvelope, Task, TaskAclGrantSpec, TaskAction,
        TaskControlAction, TaskDataScope, TaskExecutor, TaskGrantScope, TaskGrantSubject,
        TaskManagerClient, TaskOutcome, TaskPhase, TOOL_SHELL_TASK_SCHEMA_ID,
    };

    async fn client() -> Result<TaskManagerClient, String> {
        buckyos_api::get_buckyos_api_runtime()
            .map_err(|e| e.to_string())?
            .get_task_mgr_client()
            .await
            .map_err(|e| e.to_string())
    }

    fn record(task: &Task) -> ShellTaskRecord {
        ShellTaskRecord {
            task_id: task.task_id.clone(),
            name: task.name.clone(),
            input: task.input.clone(),
            terminal: task.phase.is_terminal(),
            outcome: task.outcome.map(|o| match o {
                TaskOutcome::Succeeded => "succeeded".to_string(),
                TaskOutcome::Failed => "failed".to_string(),
                TaskOutcome::Canceled => "canceled".to_string(),
            }),
            result: task.result.clone(),
            error: task
                .error
                .as_ref()
                .map(|e| json!({ "code": e.code, "message": e.message, "detail": e.detail })),
            message: task.message.clone(),
            progress: task.progress.clone(),
            cancel_requested: task
                .pending_control
                .as_ref()
                .filter(|c| c.action == TaskControlAction::Cancel)
                .map(|c| c.request_id.clone()),
            parent_id: task.parent_id.clone(),
        }
    }

    fn envelope(task: &Task) -> RunnerWriteEnvelope {
        RunnerWriteEnvelope {
            task_id: task.task_id.clone(),
            app_instance_id: match &task.executor {
                TaskExecutor::App {
                    app_instance_id, ..
                } => app_instance_id.clone(),
                _ => None,
            },
            runner_epoch: task.runner_epoch,
            expected_revision: task.revision,
        }
    }

    #[async_trait]
    impl ShellTaskBackend for ZoneShellTaskBackend {
        async fn create(&self, req: ShellTaskCreate) -> Result<ShellTaskRecord, String> {
            let client = client().await?;
            let task = client
                .create_task(CreateTaskReq {
                    name: req.name,
                    schema_id: TOOL_SHELL_TASK_SCHEMA_ID.to_string(),
                    schema_version: None,
                    input: req.input,
                    executor: CreateTaskExecutor::SelfApp {
                        app_instance_id: None,
                    },
                    parent_id: req.parent_id.clone(),
                    child_control_policy: None,
                    policy_preset: None,
                    permission_boundary: false,
                    storage_domain: None,
                    idempotency_key: req.idempotency_key,
                    retry_of: None,
                    supersedes: None,
                    message: None,
                })
                .await
                .map_err(|e| e.to_string())?;
            if let (Some(user_id), None, TaskPhase::Accepted) =
                (req.grant_user, &req.parent_id, task.phase)
            {
                let granted = client
                    .grant_task_access(GrantTaskAccessReq {
                        task_id: task.task_id.clone(),
                        grant: TaskAclGrantSpec {
                            subject: TaskGrantSubject::User { user_id },
                            actions: vec![
                                TaskAction::ReadMeta,
                                TaskAction::ReadInput,
                                TaskAction::ReadResult,
                                TaskAction::Control,
                            ],
                            scope: TaskGrantScope::Subtree,
                            data_scope: TaskDataScope::Full,
                        },
                        expected_revision: task.revision,
                    })
                    .await;
                if let Err(e) = granted {
                    log::warn!("task {}: owner grant failed: {e}", task.task_id);
                }
            }
            // Accepted → Running right away: the command already runs.
            let task = client.runner_start(&task.task_id).await.unwrap_or(task);
            Ok(record(&task))
        }

        async fn get(&self, task_id: &str) -> Result<Option<ShellTaskRecord>, String> {
            match client().await?.get_task(task_id).await {
                Ok(task) => Ok(Some(record(&task))),
                Err(e)
                    if buckyos_api::task_mgr_error_code(&e)
                        == Some(buckyos_api::TASK_ERR_NOT_FOUND) =>
                {
                    Ok(None)
                }
                Err(e) => Err(e.to_string()),
            }
        }

        async fn progress(&self, task_id: &str, message: &str, progress: Json) -> Result<(), String> {
            let client = client().await?;
            client.runner_start(task_id).await.map_err(|e| e.to_string())?;
            client
                .runner_progress(task_id, Some(progress), Some(message.to_string()))
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
        }

        async fn complete(&self, task_id: &str, result: Json) -> Result<(), String> {
            let client = client().await?;
            client.runner_start(task_id).await.map_err(|e| e.to_string())?;
            client
                .runner_complete(task_id, result)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
        }

        async fn fail(
            &self,
            task_id: &str,
            code: &str,
            message: &str,
            detail: Option<Json>,
        ) -> Result<(), String> {
            let client = client().await?;
            let _ = client.runner_start(task_id).await;
            client
                .runner_fail(task_id, code, message, detail)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
        }

        async fn request_cancel(&self, task_id: &str) -> Result<(), String> {
            client()
                .await?
                .cancel_task(task_id, false)
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
        }

        async fn canceled(&self, task_id: &str) -> Result<(), String> {
            let client = client().await?;
            let mut task = client.get_task(task_id).await.map_err(|e| e.to_string())?;
            if task.phase.is_terminal() {
                return Ok(());
            }
            if task.pending_control.is_none() {
                client
                    .request_control(RequestControlReq {
                        task_id: task_id.to_string(),
                        action: TaskControlAction::Cancel,
                        request_id: format!("shell-cancel:{task_id}"),
                        recursive: false,
                        expected_revision: None,
                    })
                    .await
                    .map_err(|e| e.to_string())?;
                task = client.get_task(task_id).await.map_err(|e| e.to_string())?;
            }
            let Some(pending) = task.pending_control.clone() else {
                return Err(format!("task {task_id} has no pending cancel to acknowledge"));
            };
            if pending.action != TaskControlAction::Cancel {
                return Err(format!(
                    "task {task_id} waits for {} while its command was cancelled",
                    pending.action
                ));
            }
            client
                .ack_control(AckControlReq {
                    envelope: envelope(&task),
                    request_id: pending.request_id,
                    applied: true,
                    reject_reason: None,
                })
                .await
                .map(|_| ())
                .map_err(|e| e.to_string())
        }

        async fn list_open(&self) -> Result<Vec<ShellTaskRecord>, String> {
            let client = client().await?;
            let mut out = Vec::new();
            for phase in [TaskPhase::Accepted, TaskPhase::Running, TaskPhase::Waiting] {
                let mut cursor = None;
                loop {
                    let page = client
                        .list_tasks(ListTasksReq {
                            creator_user_id: None,
                            creator_app_id: None,
                            idempotency_key: None,
                            schema_id: Some(TOOL_SHELL_TASK_SCHEMA_ID.to_string()),
                            phase: Some(phase),
                            root_id: None,
                            executor_kind: None,
                            storage_domain: None,
                            runner_app_id: None,
                            runner_target_id: None,
                            created_after: None,
                            created_before: None,
                            include_archived: false,
                            cursor: cursor.take(),
                            limit: Some(200),
                        })
                        .await
                        .map_err(|e| e.to_string())?;
                    for summary in page.tasks {
                        match client.get_task(&summary.task_id).await {
                            Ok(task) => out.push(record(&task)),
                            Err(e) => log::debug!("task {}: not read: {e}", summary.task_id),
                        }
                    }
                    cursor = page.next_cursor;
                    if cursor.is_none() {
                        break;
                    }
                }
            }
            Ok(out)
        }
    }
}

// ---------------------------------------------------------------------------
// In-memory backend (tests, hosts without a zone)
// ---------------------------------------------------------------------------

/// A TaskMgr in memory: what the zone backend does, without the zone.
#[derive(Default)]
pub struct MemoryShellTaskBackend {
    inner: Mutex<MemoryState>,
}

#[derive(Default)]
struct MemoryState {
    tasks: BTreeMap<String, ShellTaskRecord>,
    by_key: BTreeMap<String, String>,
    seq: u64,
    /// Fail every `create` (TaskMgr unreachable).
    refuse_create: bool,
}

impl MemoryShellTaskBackend {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn set_refuse_create(&self, refuse: bool) {
        self.inner.lock().expect("memory tasks").refuse_create = refuse;
    }

    pub fn record(&self, task_id: &str) -> Option<ShellTaskRecord> {
        self.inner.lock().expect("memory tasks").tasks.get(task_id).cloned()
    }

    pub fn records(&self) -> Vec<ShellTaskRecord> {
        self.inner.lock().expect("memory tasks").tasks.values().cloned().collect()
    }

    /// Put a task there as a previous process would have left it.
    pub fn seed(&self, record: ShellTaskRecord) {
        self.inner
            .lock()
            .expect("memory tasks")
            .tasks
            .insert(record.task_id.clone(), record);
    }

    fn update(
        &self,
        task_id: &str,
        f: impl FnOnce(&mut ShellTaskRecord) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut s = self.inner.lock().expect("memory tasks");
        let rec = s
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| format!("task {task_id} not found"))?;
        f(rec)
    }
}

#[async_trait]
impl ShellTaskBackend for MemoryShellTaskBackend {
    async fn create(&self, req: ShellTaskCreate) -> Result<ShellTaskRecord, String> {
        let mut s = self.inner.lock().expect("memory tasks");
        if s.refuse_create {
            return Err("task manager unreachable".into());
        }
        if let Some(id) = s.by_key.get(&req.idempotency_key) {
            return Ok(s.tasks[id].clone());
        }
        s.seq += 1;
        let task_id = format!("{ZONE_TASK_PREFIX}{:032x}", s.seq);
        let rec = ShellTaskRecord {
            task_id: task_id.clone(),
            name: req.name,
            input: req.input,
            parent_id: req.parent_id,
            ..Default::default()
        };
        s.by_key.insert(req.idempotency_key, task_id.clone());
        s.tasks.insert(task_id, rec.clone());
        Ok(rec)
    }

    async fn get(&self, task_id: &str) -> Result<Option<ShellTaskRecord>, String> {
        Ok(self.record(task_id))
    }

    async fn progress(&self, task_id: &str, message: &str, progress: Json) -> Result<(), String> {
        self.update(task_id, |r| {
            if !r.terminal {
                r.message = Some(message.to_string());
                r.progress = Some(progress);
            }
            Ok(())
        })
    }

    async fn complete(&self, task_id: &str, result: Json) -> Result<(), String> {
        self.update(task_id, |r| {
            if !r.terminal {
                r.terminal = true;
                r.outcome = Some("succeeded".into());
                r.result = Some(result);
            }
            Ok(())
        })
    }

    async fn fail(
        &self,
        task_id: &str,
        code: &str,
        message: &str,
        detail: Option<Json>,
    ) -> Result<(), String> {
        self.update(task_id, |r| {
            if !r.terminal {
                r.terminal = true;
                r.outcome = Some("failed".into());
                r.error = Some(json!({ "code": code, "message": message, "detail": detail }));
            }
            Ok(())
        })
    }

    async fn request_cancel(&self, task_id: &str) -> Result<(), String> {
        self.update(task_id, |r| {
            if !r.terminal && r.cancel_requested.is_none() {
                r.cancel_requested = Some(format!("ctl-{task_id}"));
            }
            Ok(())
        })
    }

    async fn canceled(&self, task_id: &str) -> Result<(), String> {
        self.update(task_id, |r| {
            if !r.terminal {
                r.terminal = true;
                r.outcome = Some("canceled".into());
                r.cancel_requested = None;
            }
            Ok(())
        })
    }

    async fn list_open(&self) -> Result<Vec<ShellTaskRecord>, String> {
        Ok(self.records().into_iter().filter(|r| !r.terminal).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm_bash::{
        new_run_binding_slot, BashRunRequest, BashRunner, BashTarget, LocalProcessBashRunner,
        RunBinding,
    };
    use crate::SessionRuntimeContext;
    use std::path::Path;

    fn sctx() -> SessionRuntimeContext {
        SessionRuntimeContext {
            trace_id: "t".into(),
            agent_name: "a".into(),
            behavior: "b".into(),
            tool_call_index: 0,
            wakeup_id: String::new(),
            session_id: "s".into(),
            read_token_limit: crate::DEFAULT_READ_TOKEN_LIMIT,
        }
    }

    fn req(command: &str, cwd: &Path, call_id: &str) -> BashRunRequest {
        BashRunRequest {
            command: command.into(),
            cwd: cwd.into(),
            timeout_ms: 10_000,
            max_output_bytes: 64 * 1024,
            env: Vec::new(),
            target: BashTarget::Local,
            call_id: Some(call_id.into()),
        }
    }

    struct Harness {
        dir: tempfile::TempDir,
        slot: crate::llm_bash::RunBindingSlot,
        backend: Arc<MemoryShellTaskBackend>,
        tasks: Arc<ZoneShellTasks>,
    }

    fn harness() -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let slot = new_run_binding_slot();
        *slot.lock().unwrap() = Some(RunBinding {
            run_id: "r1".into(),
            run_dir: Some(dir.path().join("run")),
        });
        let backend = MemoryShellTaskBackend::new();
        let tasks = ZoneShellTasks::new(
            backend.clone(),
            "did:test:agent",
            Some("bob".into()),
            InProcessTaskManager::new(),
        );
        Harness {
            dir,
            slot,
            backend,
            tasks,
        }
    }

    async fn start(h: &Harness, command: &str, call_id: &str) -> ShellTask {
        let runner = LocalProcessBashRunner::new().with_run_binding(h.slot.clone());
        let mut handle = runner
            .start(&sctx(), req(command, h.dir.path(), call_id))
            .await
            .unwrap();
        assert!(handle.wait(Duration::from_millis(50)).await.unwrap().is_none());
        handle.detach();
        ShellTask::new(command, handle, "native")
    }

    fn ctx(h: &Harness) -> ShellTaskContext {
        ShellTaskContext {
            session_id: Some("ui-1".into()),
            parent_task: Some(Arc::new(|| Some("t-parent".to_string()))),
            run: Some(h.slot.clone()),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn handed_over_command_is_a_zone_task_until_read() {
        let h = harness();
        let task = start(&h, "sleep 0.4; echo finished", "c1").await;
        let id = h.tasks.take_over(&ctx(&h), Some("c1"), task).await;
        assert!(ZoneShellTasks::is_zone_task(&id), "{id}");
        let rec = h.backend.record(&id).unwrap();
        assert_eq!(rec.input["session_id"], "ui-1");
        assert_eq!(rec.input["agent"], "did:test:agent");
        assert_eq!(rec.input["call_id"], "c1");
        assert!(rec.input["exec_dir"].as_str().unwrap().ends_with("c1"));
        assert_eq!(rec.parent_id.as_deref(), Some("t-parent"));
        let resolver = h.tasks.resolver(Some("ui-1")).unwrap();
        assert!(matches!(
            resolver.state(&id).await,
            TaskState::Running {
                cancellable: true,
                ..
            }
        ));
        assert_eq!(resolver.active().await.len(), 1);
        // Another session sees nothing of it.
        assert!(h.tasks.resolver(Some("ui-2")).unwrap().active().await.is_empty());
        let TaskState::Finished(result) = resolver.wait(&id, Some(crate::now_ms() + 5_000)).await
        else {
            panic!()
        };
        assert!(result.success);
        assert!(result.output.contains("finished"));
        assert!(resolver.result_seen(&id));
        assert!(resolver.active().await.is_empty(), "read once: gone from the env");
        // The watcher wrote the end to TaskMgr.
        for _ in 0..50 {
            if h.backend.record(&id).unwrap().terminal {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let rec = h.backend.record(&id).unwrap();
        assert_eq!(rec.outcome.as_deref(), Some("succeeded"));
        assert_eq!(rec.result.as_ref().unwrap()["exit_code"], 0);
        assert!(rec.result.as_ref().unwrap()["output"]
            .as_str()
            .unwrap()
            .contains("finished"));
        // A process that did not run it reads the same from TaskMgr.
        let other = ZoneShellTasks::new(
            h.backend.clone(),
            "did:test:agent",
            None,
            InProcessTaskManager::new(),
        );
        let TaskState::Finished(seen) = other.state(&id).await else {
            panic!()
        };
        assert!(seen.success);
        assert!(seen.output.contains("finished"));
        assert_eq!(seen.tool_result.unwrap().return_code, Some(0));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failing_command_fails_the_task() {
        let h = harness();
        let task = start(&h, "sleep 0.3; echo oops >&2; exit 3", "c2").await;
        let id = h.tasks.take_over(&ctx(&h), Some("c2"), task).await;
        let TaskState::Finished(result) = h.tasks.wait(&id, Some(crate::now_ms() + 5_000)).await
        else {
            panic!()
        };
        assert!(!result.success);
        for _ in 0..50 {
            if h.backend.record(&id).unwrap().terminal {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let rec = h.backend.record(&id).unwrap();
        assert_eq!(rec.outcome.as_deref(), Some("failed"));
        assert_eq!(rec.error.as_ref().unwrap()["code"], "exit_3");
        let TaskState::Finished(seen) = ZoneShellTasks::new(
            h.backend.clone(),
            "did:test:agent",
            None,
            InProcessTaskManager::new(),
        )
        .state(&id)
        .await
        else {
            panic!()
        };
        assert!(!seen.success);
        assert!(seen.output.contains("code 3"), "{}", seen.output);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancel_from_the_model_ends_the_task_as_canceled() {
        let h = harness();
        let task = start(&h, "sleep 30", "c3").await;
        let id = h.tasks.take_over(&ctx(&h), Some("c3"), task).await;
        let st = h.tasks.cancel(&id).await.unwrap();
        assert!(matches!(st, TaskState::Finished(ref r) if !r.success), "{st:?}");
        for _ in 0..50 {
            if h.backend.record(&id).unwrap().terminal {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let rec = h.backend.record(&id).unwrap();
        assert_eq!(rec.outcome.as_deref(), Some("canceled"), "{rec:?}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unreachable_task_manager_keeps_the_command_in_process() {
        let h = harness();
        h.backend.set_refuse_create(true);
        let task = start(&h, "sleep 0.2; echo local", "c4").await;
        let id = h.tasks.take_over(&ctx(&h), Some("c4"), task).await;
        assert_eq!(id, "local:shell:c4");
        assert!(h.backend.records().is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn recover_reads_the_exit_of_a_command_left_by_a_previous_process() {
        let h = harness();
        // A command of a previous process: its execution directory has the exit.
        let runner = LocalProcessBashRunner::new().with_run_binding(h.slot.clone());
        let mut handle = runner
            .start(&sctx(), req("echo earlier; exit 0", h.dir.path(), "c5"))
            .await
            .unwrap();
        assert!(handle.wait(Duration::from_secs(5)).await.unwrap().is_some());
        let exec_dir = h.dir.path().join("run").join("exec").join("c5");
        assert!(exec_dir.join("exit").exists());
        h.backend.seed(ShellTaskRecord {
            task_id: "t-aaaa".into(),
            name: "echo earlier".into(),
            input: json!({
                "command": "echo earlier; exit 0",
                "agent": "did:test:agent",
                "exec_dir": exec_dir.display().to_string(),
                "session_id": "ui-1",
                "started_at_ms": crate::now_ms(),
            }),
            ..Default::default()
        });
        // One without a directory, and one of another agent.
        h.backend.seed(ShellTaskRecord {
            task_id: "t-bbbb".into(),
            name: "lost".into(),
            input: json!({ "command": "lost", "agent": "did:test:agent", "exec_dir": h.dir.path().join("nope").display().to_string() }),
            ..Default::default()
        });
        h.backend.seed(ShellTaskRecord {
            task_id: "t-cccc".into(),
            name: "other".into(),
            input: json!({ "command": "other", "agent": "did:test:other" }),
            ..Default::default()
        });
        h.tasks.recover().await;
        for _ in 0..50 {
            if h.backend.record("t-aaaa").unwrap().terminal {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let a = h.backend.record("t-aaaa").unwrap();
        assert_eq!(a.outcome.as_deref(), Some("succeeded"), "{a:?}");
        assert!(a.result.unwrap()["output"].as_str().unwrap().contains("earlier"));
        let b = h.backend.record("t-bbbb").unwrap();
        assert_eq!(b.outcome.as_deref(), Some("failed"));
        assert_eq!(b.error.unwrap()["code"], "executor_restarted");
        assert!(!h.backend.record("t-cccc").unwrap().terminal);
        // The recovered result is read from this process too.
        let TaskState::Finished(r) = h.tasks.state("t-aaaa").await else {
            panic!()
        };
        assert!(r.output.contains("earlier"));
    }
}
