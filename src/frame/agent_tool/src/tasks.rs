//! Tasks: work that outlives one tool call (`notepads/llm-context-long-tool-todo.md`
//! §4 / §5).
//!
//! - [`InProcessTaskManager`]: the task manager of this executor process.
//!   Holds the commands the `shell` tool handed over in `auto` mode
//!   (`local:shell:<call_id>`) and any other process-local task
//!   (`local:<kind>:<n>`). Nothing here is persisted: a restarted executor
//!   answers `Unknown` for a task it did not start, except a `shell` task
//!   whose execution directory recorded an `exit`.
//! - [`CompositeTaskResolver`]: the `RunningTaskResolver` handed to the
//!   waist. Dispatches by task id: `local:` ids to the in-process manager,
//!   anything else to the optional buckyos task-mgr resolver the host may
//!   supply.
//! - `wait_task` / `get_task_state` / `cancel_task`: the tools the model
//!   uses to follow a task. Their result text is the waist's
//!   `task_state_observation`, so an inline wait and the fill of a
//!   suspended call read the same.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use llm_context::deps::ToolCallCtx;
use llm_context::observation::Observation;
use llm_context::tasks::{
    task_state_observation, CancelUnsupported, RunningTaskResolver, TaskBrief, TaskResult,
    TaskState, DEFAULT_TASK_WAIT_MS, MAX_IN_TOOL_WAIT_MS,
};
use serde_json::{json, Value as Json};

use crate::llm_bash::{
    exec_dir_for, read_exit_file, read_file_tail, CommandHandle, RunBinding, RunBindingSlot,
    OUTPUT_TAIL_BYTES,
};
use crate::tool::CallingConventions;
use crate::{
    build_builtin_tool_result, AgentTool, AgentToolError, AgentToolResult, AgentToolStatus,
    SessionRuntimeContext, ToolSpec,
};

pub const TOOL_WAIT_TASK: &str = "wait_task";
pub const TOOL_GET_TASK_STATE: &str = "get_task_state";
pub const TOOL_CANCEL_TASK: &str = "cancel_task";

/// Prefix of every task id owned by an in-process task manager.
pub const LOCAL_TASK_PREFIX: &str = "local:";

static LOCAL_SEQ: AtomicU64 = AtomicU64::new(1);

pub(crate) fn next_local_seq() -> u64 {
    LOCAL_SEQ.fetch_add(1, Ordering::SeqCst)
}

/// A task of this process.
#[async_trait]
pub trait LocalTask: Send + Sync {
    fn brief(&self) -> String;
    fn started_at_ms(&self) -> u64;
    fn cancellable(&self) -> bool;
    /// Current state without waiting.
    async fn state(&self) -> TaskState;
    /// Wait until the task ends or `until_ms` (epoch ms) passes.
    async fn wait(&self, until_ms: Option<u64>) -> TaskState;
    async fn cancel(&self) -> Result<TaskState, CancelUnsupported>;
}

/// A `shell` command that outlived its call's `wait_ms`.
pub struct ShellTask {
    command: String,
    runtime: String,
    handle: tokio::sync::Mutex<Box<dyn CommandHandle>>,
    started_at_ms: u64,
    cancellable: bool,
    locator: String,
    finished: Mutex<Option<TaskResult>>,
}

impl ShellTask {
    pub fn new(command: &str, handle: Box<dyn CommandHandle>, runtime: &str) -> Self {
        Self {
            command: command.to_string(),
            runtime: runtime.to_string(),
            cancellable: handle.cancellable(),
            locator: handle.locator(),
            handle: tokio::sync::Mutex::new(handle),
            started_at_ms: crate::now_ms(),
            finished: Mutex::new(None),
        }
    }

    fn finish(&self, out: crate::llm_bash::BashRunOutput, cancelled: bool) -> TaskState {
        let mut text = if cancelled {
            format!(
                "`{}` was cancelled after {}ms (runtime {}).",
                self.command, out.duration_ms, self.runtime
            )
        } else {
            format!(
                "`{}` exited with code {} after {}ms (runtime {}).",
                self.command, out.exit_code, out.duration_ms, self.runtime
            )
        };
        if !out.output.trim().is_empty() {
            text.push_str("\n--- output ---\n");
            text.push_str(out.output.trim_end());
        }
        let result = TaskResult {
            success: out.exit_code == 0 && !out.timed_out && !cancelled,
            output: text,
            tool_result: Some(llm_context::observation::ToolResultView {
                cmd_name: Some(crate::llm_bash::TOOL_SHELL.into()),
                cmd_args: Some(self.command.clone()),
                summary: if cancelled {
                    "cancelled".into()
                } else {
                    format!("exit={} in {}ms", out.exit_code, out.duration_ms)
                },
                return_code: Some(out.exit_code),
                detail: Some(json!({
                    "exit_code": out.exit_code,
                    "duration_ms": out.duration_ms,
                    "output_truncated": out.output_truncated,
                    "runtime": self.runtime,
                    "cancelled": cancelled,
                })),
                ..Default::default()
            }),
        };
        *self.finished.lock().expect("shell task") = Some(result.clone());
        TaskState::Finished(result)
    }

    async fn poll(&self, timeout: Duration) -> TaskState {
        if let Some(done) = self.finished.lock().expect("shell task").clone() {
            return TaskState::Finished(done);
        }
        let mut handle = self.handle.lock().await;
        match handle.wait(timeout).await {
            Ok(Some(out)) => self.finish(out, false),
            Ok(None) => {
                let progress = handle.progress().await;
                TaskState::Running {
                    brief: self.brief(),
                    elapsed_ms: progress.elapsed_ms,
                    output_tail: progress.output_tail,
                    cancellable: self.cancellable,
                }
            }
            Err(e) => TaskState::Unknown {
                reason: format!("cannot observe `{}`: {e}", self.command),
            },
        }
    }
}

#[async_trait]
impl LocalTask for ShellTask {
    fn brief(&self) -> String {
        format!("shell: {}", compact(&self.command, 80))
    }
    fn started_at_ms(&self) -> u64 {
        self.started_at_ms
    }
    fn cancellable(&self) -> bool {
        self.cancellable
    }
    async fn state(&self) -> TaskState {
        self.poll(Duration::from_millis(1)).await
    }
    async fn wait(&self, until_ms: Option<u64>) -> TaskState {
        let now = crate::now_ms();
        let budget = until_ms
            .map(|u| u.saturating_sub(now))
            .unwrap_or(MAX_IN_TOOL_WAIT_MS);
        self.poll(Duration::from_millis(budget.max(1))).await
    }
    async fn cancel(&self) -> Result<TaskState, CancelUnsupported> {
        if !self.cancellable {
            return Err(CancelUnsupported {
                task_id: String::new(),
            });
        }
        if let Some(done) = self.finished.lock().expect("shell task").clone() {
            return Ok(TaskState::Finished(done));
        }
        let mut handle = self.handle.lock().await;
        match handle.kill().await {
            Ok(out) => Ok(self.finish(out, true)),
            Err(e) => Ok(TaskState::Unknown {
                reason: format!("cancel failed: {e}"),
            }),
        }
    }
}

fn compact(s: &str, max: usize) -> String {
    let one_line = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= max {
        one_line
    } else {
        format!("{}…", one_line.chars().take(max).collect::<String>())
    }
}

#[derive(Default)]
struct Registry {
    tasks: BTreeMap<String, Arc<dyn LocalTask>>,
    /// Finished tasks whose result the model already read once: no longer
    /// shown in the background env.
    read: BTreeSet<String>,
}

/// Task manager of this executor process.
#[derive(Default)]
pub struct InProcessTaskManager {
    inner: Mutex<Registry>,
}

impl std::fmt::Debug for InProcessTaskManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.inner.lock().expect("tasks").tasks.len();
        f.debug_struct("InProcessTaskManager")
            .field("tasks", &n)
            .finish()
    }
}

impl InProcessTaskManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Register a `shell` command handed over by call `call_id`; the id
    /// `local:shell:<call_id>` lets a later executor find its execution
    /// directory.
    pub fn register_shell(&self, call_id: Option<&str>, task: ShellTask) -> String {
        let id = match call_id.filter(|c| !c.is_empty()) {
            Some(c) => format!(
                "{LOCAL_TASK_PREFIX}shell:{}",
                crate::llm_bash::sanitize_call_id(c)
            ),
            None => format!("{LOCAL_TASK_PREFIX}shell:anon-{}", next_local_seq()),
        };
        self.insert(id.clone(), Arc::new(task));
        id
    }

    /// Register any other process-local task under `local:<kind>:<n>`.
    pub fn register(&self, kind: &str, task: Arc<dyn LocalTask>) -> String {
        let id = format!("{LOCAL_TASK_PREFIX}{kind}:{}", next_local_seq());
        self.insert(id.clone(), task);
        id
    }

    fn insert(&self, id: String, task: Arc<dyn LocalTask>) {
        let mut r = self.inner.lock().expect("tasks");
        r.read.remove(&id);
        r.tasks.insert(id, task);
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn LocalTask>> {
        self.inner.lock().expect("tasks").tasks.get(id).cloned()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.lock().expect("tasks").tasks.is_empty()
    }

    fn mark_read(&self, id: &str) {
        self.inner
            .lock()
            .expect("tasks")
            .read
            .insert(id.to_string());
    }

    async fn briefs(&self) -> Vec<TaskBrief> {
        let entries: Vec<(String, Arc<dyn LocalTask>)> = {
            let r = self.inner.lock().expect("tasks");
            r.tasks
                .iter()
                .filter(|(id, _)| !r.read.contains(*id))
                .map(|(id, t)| (id.clone(), t.clone()))
                .collect()
        };
        let mut out = Vec::new();
        for (id, task) in entries {
            let state = task.state().await;
            let (status, elapsed_ms, last_line) = match &state {
                TaskState::Running {
                    elapsed_ms,
                    output_tail,
                    ..
                } => (
                    "running",
                    *elapsed_ms,
                    output_tail
                        .lines()
                        .rev()
                        .find(|l| !l.trim().is_empty())
                        .unwrap_or("")
                        .to_string(),
                ),
                TaskState::Finished(r) => (
                    if r.success { "finished" } else { "failed" },
                    crate::now_ms().saturating_sub(task.started_at_ms()),
                    r.tool_result
                        .as_ref()
                        .map(|t| t.summary.clone())
                        .unwrap_or_default(),
                ),
                TaskState::Unknown { reason } => ("unknown", 0, reason.clone()),
            };
            out.push(TaskBrief {
                task_id: id,
                brief: task.brief(),
                status: status.into(),
                elapsed_ms,
                last_line: compact(&last_line, 120),
                cancellable: task.cancellable(),
            });
        }
        out
    }
}

/// `RunningTaskResolver` of one run: the in-process task manager, the
/// run's execution directories (for `shell` tasks of a previous executor)
/// and, when the host has one, the buckyos task-mgr.
pub struct CompositeTaskResolver {
    local: Arc<InProcessTaskManager>,
    run: RunBindingSlot,
    buckyos: Option<Arc<dyn RunningTaskResolver>>,
    /// Non-local tasks this context follows (ids seen in results).
    watched: Mutex<BTreeSet<String>>,
}

impl CompositeTaskResolver {
    pub fn new(local: Arc<InProcessTaskManager>, run: RunBindingSlot) -> Self {
        Self {
            local,
            run,
            buckyos: None,
            watched: Mutex::new(BTreeSet::new()),
        }
    }

    pub fn with_buckyos(mut self, resolver: Arc<dyn RunningTaskResolver>) -> Self {
        self.buckyos = Some(resolver);
        self
    }

    fn run_binding(&self) -> Option<RunBinding> {
        self.run.lock().expect("run binding").clone()
    }

    /// A `shell` task this process did not start: its execution directory
    /// may hold the result.
    fn shell_exec_dir(&self, task_id: &str) -> Option<PathBuf> {
        let call_id = task_id.strip_prefix(&format!("{LOCAL_TASK_PREFIX}shell:"))?;
        let binding = self.run_binding()?;
        exec_dir_for(binding.run_dir.as_deref(), Some(call_id))
    }

    fn state_from_exec_dir(&self, task_id: &str, dir: &Path) -> TaskState {
        let command = std::fs::read_to_string(dir.join("command")).unwrap_or_default();
        let command = compact(command.trim(), 80);
        match read_exit_file(dir) {
            Some(code) => {
                let stdout = read_file_tail(&dir.join("stdout"), OUTPUT_TAIL_BYTES * 4);
                let stderr = read_file_tail(&dir.join("stderr"), OUTPUT_TAIL_BYTES * 4);
                let mut text = format!(
                    "`{command}` (task {task_id}) exited with code {code}; started by a previous executor, read from {}.",
                    dir.display()
                );
                if !stdout.trim().is_empty() {
                    text.push_str("\n--- stdout (tail) ---\n");
                    text.push_str(stdout.trim_end());
                }
                if !stderr.trim().is_empty() {
                    text.push_str("\n--- stderr (tail) ---\n");
                    text.push_str(stderr.trim_end());
                }
                TaskState::Finished(TaskResult {
                    success: code == 0,
                    output: text,
                    tool_result: Some(llm_context::observation::ToolResultView {
                        cmd_name: Some(crate::llm_bash::TOOL_SHELL.into()),
                        cmd_args: Some(command),
                        return_code: Some(code),
                        summary: format!("exit={code}"),
                        ..Default::default()
                    }),
                })
            }
            None if dir.is_dir() => TaskState::Unknown {
                reason: format!(
                    "`{command}` was started by a previous executor and has not recorded an exit; it may still be running. Output files: {}",
                    dir.display()
                ),
            },
            None => TaskState::Unknown {
                reason: "this executor did not start the task and has no record of it".into(),
            },
        }
    }

    async fn local_state(&self, task_id: &str) -> TaskState {
        match self.local.get(task_id) {
            Some(task) => {
                let state = task.state().await;
                if matches!(state, TaskState::Finished(_)) {
                    self.local.mark_read(task_id);
                }
                state
            }
            None => match self.shell_exec_dir(task_id) {
                Some(dir) => self.state_from_exec_dir(task_id, &dir),
                None => TaskState::Unknown {
                    reason: "this executor did not start the task and has no record of it".into(),
                },
            },
        }
    }
}

#[async_trait]
impl RunningTaskResolver for CompositeTaskResolver {
    async fn state(&self, task_id: &str) -> TaskState {
        if task_id.starts_with(LOCAL_TASK_PREFIX) {
            return self.local_state(task_id).await;
        }
        match &self.buckyos {
            Some(b) => b.state(task_id).await,
            None => TaskState::Unknown {
                reason: "the buckyos task manager is not reachable from this executor".into(),
            },
        }
    }

    async fn wait(&self, task_id: &str, until_ms: Option<u64>) -> TaskState {
        if task_id.starts_with(LOCAL_TASK_PREFIX) {
            return match self.local.get(task_id) {
                Some(task) => {
                    let state = task.wait(until_ms).await;
                    if matches!(state, TaskState::Finished(_)) {
                        self.local.mark_read(task_id);
                    }
                    state
                }
                None => self.local_state(task_id).await,
            };
        }
        match &self.buckyos {
            Some(b) => b.wait(task_id, until_ms).await,
            None => self.state(task_id).await,
        }
    }

    async fn cancel(&self, task_id: &str) -> Result<TaskState, CancelUnsupported> {
        if task_id.starts_with(LOCAL_TASK_PREFIX) {
            let Some(task) = self.local.get(task_id) else {
                return Err(CancelUnsupported {
                    task_id: task_id.into(),
                });
            };
            return task.cancel().await.map_err(|_| CancelUnsupported {
                task_id: task_id.into(),
            });
        }
        match &self.buckyos {
            Some(b) => b.cancel(task_id).await,
            None => Err(CancelUnsupported {
                task_id: task_id.into(),
            }),
        }
    }

    fn can_resolve(&self, task_id: &str) -> bool {
        task_id.starts_with(LOCAL_TASK_PREFIX) || self.buckyos.is_some()
    }

    fn watch(&self, task_id: &str) {
        if !task_id.starts_with(LOCAL_TASK_PREFIX) {
            self.watched
                .lock()
                .expect("watched")
                .insert(task_id.to_string());
        }
    }

    async fn active(&self) -> Vec<TaskBrief> {
        let mut out = self.local.briefs().await;
        for id in self.watched.lock().expect("watched").iter() {
            out.push(TaskBrief {
                task_id: id.clone(),
                brief: "task".into(),
                status: "running".into(),
                elapsed_ms: 0,
                last_line: String::new(),
                cancellable: false,
            });
        }
        out
    }
}

/// `AgentToolResult` carrying exactly the text of `task_state_observation`.
fn result_from_observation(tool: &str, task_id: &str, obs: Observation) -> AgentToolResult {
    let (status, text, view) = match obs {
        Observation::Success {
            content,
            tool_result,
            ..
        } => (
            AgentToolStatus::Success,
            content.as_str().unwrap_or_default().to_string(),
            tool_result,
        ),
        Observation::Error {
            message,
            tool_result,
            ..
        } => (AgentToolStatus::Error, message, tool_result),
        other => (AgentToolStatus::Error, format!("{other:?}"), None),
    };
    let summary = view
        .as_ref()
        .map(|v| v.summary.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("task {task_id}"));
    let details = view
        .as_ref()
        .and_then(|v| v.detail.clone())
        .unwrap_or_else(|| json!({ "task_id": task_id }));
    let mut result = build_builtin_tool_result(details, format!("{tool} {task_id}"), summary)
        .with_tool(tool)
        .with_status(status)
        .with_task_id(task_id)
        .refresh_default_title();
    if let Some(code) = view.as_ref().and_then(|v| v.return_code) {
        result = result.with_return_code(code);
    }
    if status == AgentToolStatus::Success {
        result = result.with_output(text);
    } else {
        result.summary = text;
    }
    result
}

fn require_task_id(args: &Json) -> Result<String, AgentToolError> {
    args.get("task_id")
        .and_then(Json::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| AgentToolError::InvalidArgs("`task_id` is required".into()))
}

fn task_id_schema(extra: Json) -> Json {
    let mut props = json!({
        "task_id": { "type": "string", "description": "task id from an earlier result" }
    });
    if let Some(m) = extra.as_object() {
        for (k, v) in m {
            props[k] = v.clone();
        }
    }
    json!({ "type": "object", "properties": props, "required": ["task_id"] })
}

fn current_ctx() -> Option<ToolCallCtx> {
    crate::runtime::CURRENT_TOOL_CTX
        .try_with(|c| c.clone())
        .ok()
}

/// `wait_task(task_id, wait_ms)`.
pub struct WaitTaskTool {
    resolver: Arc<dyn RunningTaskResolver>,
}

impl WaitTaskTool {
    pub fn new(resolver: Arc<dyn RunningTaskResolver>) -> Self {
        Self { resolver }
    }
}

#[async_trait]
impl AgentTool for WaitTaskTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_WAIT_TASK.into(),
            description: format!(
                "Wait for a task to finish and return its result, or its current state and new output when wait_ms (default {}s) passes first. A wait longer than {} minutes is only possible when the session can suspend; otherwise it is capped. Waiting never affects the task.",
                DEFAULT_TASK_WAIT_MS / 1000,
                MAX_IN_TOOL_WAIT_MS / 60_000
            ),
            args_schema: task_id_schema(json!({
                "wait_ms": { "type": "integer", "minimum": 1, "description": format!("How long to wait in milliseconds (default {DEFAULT_TASK_WAIT_MS}).") }
            })),
            output_schema: json!({ "type": "object" }),
            usage: Some(format!("{TOOL_WAIT_TASK} task_id=<id> [wait_ms={DEFAULT_TASK_WAIT_MS}]")),
        }
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::ALL
    }

    fn cancellable(&self) -> bool {
        true
    }

    async fn call(
        &self,
        ctx: &SessionRuntimeContext,
        args: Json,
    ) -> Result<AgentToolResult, AgentToolError> {
        let task_id = require_task_id(&args)?;
        let wait_ms = match args.get("wait_ms") {
            None | Some(Json::Null) => DEFAULT_TASK_WAIT_MS,
            Some(v) => v
                .as_u64()
                .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
                .filter(|n| *n > 0)
                .ok_or_else(|| {
                    AgentToolError::InvalidArgs("`wait_ms` must be a positive integer".into())
                })?,
        };
        let call_ctx = current_ctx();
        let now = crate::now_ms();
        if wait_ms > MAX_IN_TOOL_WAIT_MS && call_ctx.as_ref().is_some_and(|c| c.allow_deferred) {
            // Longer than the in-tool limit: the host waits outside the
            // process (§4, second decision).
            return Ok(build_builtin_tool_result(
                json!({ "task_id": task_id, "until_ms": now + wait_ms }),
                format!("{TOOL_WAIT_TASK} {task_id}"),
                format!("waiting for task {task_id} outside the run"),
            )
            .with_tool(TOOL_WAIT_TASK)
            .with_status(AgentToolStatus::Pending)
            .with_task_id(task_id.clone())
            .with_check_after(wait_ms / 1000)
            .refresh_default_title());
        }
        let until = match &call_ctx {
            Some(c) => c.wait_until_ms(Some(wait_ms)),
            None => now + wait_ms.min(MAX_IN_TOOL_WAIT_MS),
        };
        let state = tokio::select! {
            s = self.resolver.wait(&task_id, Some(until)) => s,
            cause = async {
                match &call_ctx {
                    Some(c) => c.cancelled().await,
                    None => std::future::pending().await,
                }
            } => {
                return Err(AgentToolError::Cancelled {
                    message: format!("stopped waiting for task {task_id} ({cause:?}); the task itself continues unless cancelled with cancel_task"),
                    effect_unknown: false,
                });
            }
        };
        let _ = ctx;
        Ok(result_from_observation(
            TOOL_WAIT_TASK,
            &task_id,
            task_state_observation("", &task_id, &state),
        ))
    }
}

/// `get_task_state(task_id)`.
pub struct GetTaskStateTool {
    resolver: Arc<dyn RunningTaskResolver>,
}

impl GetTaskStateTool {
    pub fn new(resolver: Arc<dyn RunningTaskResolver>) -> Self {
        Self { resolver }
    }
}

#[async_trait]
impl AgentTool for GetTaskStateTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_GET_TASK_STATE.into(),
            description: "Return a task's current state at once: its result when finished, otherwise how long it has run and the tail of its output.".into(),
            args_schema: task_id_schema(json!({})),
            output_schema: json!({ "type": "object" }),
            usage: Some(format!("{TOOL_GET_TASK_STATE} task_id=<id>")),
        }
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::ALL
    }

    async fn call(
        &self,
        _ctx: &SessionRuntimeContext,
        args: Json,
    ) -> Result<AgentToolResult, AgentToolError> {
        let task_id = require_task_id(&args)?;
        let state = self.resolver.state(&task_id).await;
        Ok(result_from_observation(
            TOOL_GET_TASK_STATE,
            &task_id,
            task_state_observation("", &task_id, &state),
        ))
    }
}

/// `cancel_task(task_id)`.
pub struct CancelTaskTool {
    resolver: Arc<dyn RunningTaskResolver>,
}

impl CancelTaskTool {
    pub fn new(resolver: Arc<dyn RunningTaskResolver>) -> Self {
        Self { resolver }
    }
}

#[async_trait]
impl AgentTool for CancelTaskTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_CANCEL_TASK.into(),
            description: "Stop a running task. Only tasks that declared themselves cancellable can be stopped; others report that cancellation is unsupported and keep running.".into(),
            args_schema: task_id_schema(json!({})),
            output_schema: json!({ "type": "object" }),
            usage: Some(format!("{TOOL_CANCEL_TASK} task_id=<id>")),
        }
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::ALL
    }

    async fn call(
        &self,
        _ctx: &SessionRuntimeContext,
        args: Json,
    ) -> Result<AgentToolResult, AgentToolError> {
        let task_id = require_task_id(&args)?;
        match self.resolver.cancel(&task_id).await {
            Ok(state) => Ok(result_from_observation(
                TOOL_CANCEL_TASK,
                &task_id,
                task_state_observation("", &task_id, &state),
            )),
            Err(_) => Ok(build_builtin_tool_result(
                json!({ "task_id": task_id, "cancellable": false }),
                format!("{TOOL_CANCEL_TASK} {task_id}"),
                format!("task {task_id} does not support cancellation; it keeps running"),
            )
            .with_tool(TOOL_CANCEL_TASK)
            .with_status(AgentToolStatus::Error)
            .with_task_id(task_id.clone())
            .refresh_default_title()),
        }
    }
}

/// The three task tools over one resolver.
pub fn task_tools(resolver: Arc<dyn RunningTaskResolver>) -> Vec<Arc<dyn AgentTool>> {
    vec![
        Arc::new(WaitTaskTool::new(resolver.clone())),
        Arc::new(GetTaskStateTool::new(resolver.clone())),
        Arc::new(CancelTaskTool::new(resolver)),
    ]
}

/// A task that is already finished (tests, hosts that only have a result).
pub struct FinishedTask {
    pub brief: String,
    pub result: TaskResult,
    pub started_at_ms: u64,
    read: AtomicBool,
}

impl FinishedTask {
    pub fn new(brief: &str, result: TaskResult) -> Self {
        Self {
            brief: brief.into(),
            result,
            started_at_ms: crate::now_ms(),
            read: AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl LocalTask for FinishedTask {
    fn brief(&self) -> String {
        self.brief.clone()
    }
    fn started_at_ms(&self) -> u64 {
        self.started_at_ms
    }
    fn cancellable(&self) -> bool {
        false
    }
    async fn state(&self) -> TaskState {
        self.read.store(true, Ordering::SeqCst);
        TaskState::Finished(self.result.clone())
    }
    async fn wait(&self, _until_ms: Option<u64>) -> TaskState {
        self.state().await
    }
    async fn cancel(&self) -> Result<TaskState, CancelUnsupported> {
        Err(CancelUnsupported {
            task_id: String::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm_bash::{
        new_run_binding_slot, BashRunRequest, BashRunner, BashTarget, LocalProcessBashRunner,
    };

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

    #[tokio::test]
    async fn shell_task_runs_to_completion_and_is_read_once() {
        let dir = tempfile::tempdir().unwrap();
        let slot = new_run_binding_slot();
        *slot.lock().unwrap() = Some(RunBinding {
            run_id: "r".into(),
            run_dir: Some(dir.path().join("run")),
        });
        let runner = LocalProcessBashRunner::new().with_run_binding(slot.clone());
        let mut handle = runner
            .start(&sctx(), req("sleep 0.4; echo finished", dir.path(), "c1"))
            .await
            .unwrap();
        assert!(handle
            .wait(Duration::from_millis(50))
            .await
            .unwrap()
            .is_none());
        handle.detach();
        let tasks = InProcessTaskManager::new();
        let id = tasks.register_shell(
            Some("c1"),
            ShellTask::new("sleep 0.4; echo finished", handle, "native"),
        );
        assert_eq!(id, "local:shell:c1");
        let resolver = CompositeTaskResolver::new(tasks.clone(), slot);
        assert!(matches!(
            resolver.state(&id).await,
            TaskState::Running {
                cancellable: true,
                ..
            }
        ));
        assert_eq!(resolver.active().await.len(), 1);
        let state = resolver.wait(&id, Some(crate::now_ms() + 5_000)).await;
        let TaskState::Finished(result) = state else {
            panic!("{state:?}")
        };
        assert!(result.success);
        assert!(result.output.contains("finished"));
        assert!(
            resolver.active().await.is_empty(),
            "read once: gone from the env"
        );
        // The execution directory keeps the result for a later executor.
        let fresh = CompositeTaskResolver::new(InProcessTaskManager::new(), resolver.run.clone());
        let TaskState::Finished(again) = fresh.state(&id).await else {
            panic!("exit file not read")
        };
        assert!(again.output.contains("previous executor"));
    }

    #[tokio::test]
    async fn shell_task_can_be_cancelled_and_unknown_ids_are_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let runner = LocalProcessBashRunner::new();
        let mut handle = runner
            .start(&sctx(), req("sleep 30", dir.path(), "c2"))
            .await
            .unwrap();
        assert!(handle
            .wait(Duration::from_millis(20))
            .await
            .unwrap()
            .is_none());
        handle.detach();
        let tasks = InProcessTaskManager::new();
        let id = tasks.register_shell(Some("c2"), ShellTask::new("sleep 30", handle, "native"));
        let resolver = CompositeTaskResolver::new(tasks, new_run_binding_slot());
        let TaskState::Finished(r) = resolver.cancel(&id).await.unwrap() else {
            panic!("cancel")
        };
        assert!(!r.success);
        assert!(matches!(
            resolver.state("local:shell:nope").await,
            TaskState::Unknown { .. }
        ));
        assert!(matches!(
            resolver.state("bucky:123").await,
            TaskState::Unknown { .. }
        ));
        assert!(!resolver.can_resolve("bucky:123"));
        assert!(resolver.can_resolve("local:x:1"));
        assert!(resolver.cancel("bucky:123").await.is_err());
    }

    #[tokio::test]
    async fn task_tools_render_the_resolver_state() {
        let tasks = InProcessTaskManager::new();
        let id = tasks.register(
            "test",
            Arc::new(FinishedTask::new(
                "unit",
                TaskResult {
                    success: true,
                    output: "all good".into(),
                    tool_result: None,
                },
            )),
        );
        let resolver: Arc<dyn RunningTaskResolver> =
            Arc::new(CompositeTaskResolver::new(tasks, new_run_binding_slot()));
        let tools = task_tools(resolver);
        let get = tools[1]
            .call(&sctx(), json!({ "task_id": id }))
            .await
            .unwrap();
        assert_eq!(get.status, AgentToolStatus::Success);
        assert_eq!(get.output.as_deref(), Some("all good"));
        assert_eq!(get.task_id.as_deref(), Some(id.as_str()));
        let wait = tools[0]
            .call(&sctx(), json!({ "task_id": id, "wait_ms": 10 }))
            .await
            .unwrap();
        assert_eq!(
            wait.output, get.output,
            "inline wait and state read the same"
        );
        let cancel = tools[2]
            .call(&sctx(), json!({ "task_id": id }))
            .await
            .unwrap();
        assert_eq!(cancel.status, AgentToolStatus::Error);
        assert!(cancel.summary.contains("does not support cancellation"));
        assert!(tools[0].call(&sctx(), json!({})).await.is_err());
    }
}
