//! The `shell` tool and the command runners behind it.
//!
//! `shell` runs a bash command in the runtime of the current run (native
//! process, tmux session or remote host — see `runtime`). Its lifecycle
//! follows standard parent / child process semantics
//! (`notepads/llm-context-long-tool-todo.md` §3.2): only the command in
//! progress is managed; whatever the command leaves behind (`&`, nohup,
//! setsid, daemons) is not tracked and never killed.
//!
//! Two execution modes, chosen by configuration, never by the model (§5):
//! - `wait`: run to completion or `timeout_ms` (the command is stopped);
//! - `auto` (default): wait `wait_ms`, then hand the still-running command
//!   to the in-process task manager and return "still running" with a
//!   `task_id` the model follows up with `wait_task` / `get_task_state`.
//!
//! Every runner writes `command`, `stdout`, `stderr` and `exit` into an
//! execution directory derived from `(run, call_id)`, so a command survives
//! the executor's exit without SIGPIPE and its result can be read back
//! after a crash.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use async_trait::async_trait;
use llm_context::deps::CancelCause;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map as JsonMap, Value as Json};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::time::{timeout as tokio_timeout, Duration};

use crate::path_utils::to_abs_path;
use crate::tasks::{InProcessTaskManager, ShellTask};
use crate::tool::CallingConventions;
use crate::{
    build_builtin_tool_result, AgentTool, AgentToolError, AgentToolResult, AgentToolStatus,
    SessionRuntimeContext, ToolSpec,
};

pub const TOOL_SHELL: &str = "shell";

/// `wait` mode: default `timeout_ms`.
pub const DEFAULT_TIMEOUT_MS: u64 = 30 * 60_000;
/// `wait` mode: default upper bound of `timeout_ms` (0 = unlimited).
pub const DEFAULT_MAX_TIMEOUT_MS: u64 = 60 * 60_000;
/// `auto` mode: default in-call wait before the command becomes a task.
pub const DEFAULT_AUTO_WAIT_MS: u64 = llm_context::tasks::DEFAULT_TASK_WAIT_MS;
/// `auto` mode: upper bound of `wait_ms`.
pub const MAX_AUTO_WAIT_MS: u64 = llm_context::tasks::MAX_IN_TOOL_WAIT_MS;
const DEFAULT_MAX_OUTPUT_BYTES: usize = 256 * 1024;
/// Bytes of output kept in a progress / cancel text.
pub const OUTPUT_TAIL_BYTES: usize = 2048;
pub(crate) const LOCAL_ENGINE: &str = "native";
pub(crate) const TIMEOUT_EXIT_CODE: i32 = 124;
const UNLIMITED_TIMEOUT: Duration = Duration::from_secs(365 * 24 * 3600);

/// How long `shell` waits inside the call.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ShellMode {
    /// Hard wait: to completion or `timeout_ms` (then the command is
    /// stopped). Not bound by the 30 minute in-tool wait rule.
    Wait,
    /// Wait `wait_ms`, then turn the command into a task and return.
    #[default]
    Auto,
}

/// The run a runner belongs to: where its execution directories live.
/// Bound late (`XllmToolManager::bind_run`): the runtime is opened before
/// the run id is known.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunBinding {
    pub run_id: String,
    /// The run directory (`runs/<run_id>`); `None` for an in-memory store.
    pub run_dir: Option<PathBuf>,
}

pub type RunBindingSlot = Arc<Mutex<Option<RunBinding>>>;

pub fn new_run_binding_slot() -> RunBindingSlot {
    Arc::new(Mutex::new(None))
}

/// `<run_dir>/exec/<call_id>`, the execution directory of one call (native
/// and tmux runtimes). `None` without a persistent run directory or call id.
pub fn exec_dir_for(run_dir: Option<&Path>, call_id: Option<&str>) -> Option<PathBuf> {
    let dir = run_dir?;
    let call_id = call_id.filter(|c| !c.is_empty())?;
    Some(dir.join("exec").join(sanitize_call_id(call_id)))
}

pub fn sanitize_call_id(call_id: &str) -> String {
    let s: String = call_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() || s.starts_with('.') {
        format!("c_{s}")
    } else {
        s
    }
}

/// Runtime facts rendered into the tool description (§3.3).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShellRuntimeNote {
    /// `native | tmux | remote_ssh`.
    pub kind: String,
    /// tmux session name / SSH host, when relevant.
    pub target: String,
}

impl ShellRuntimeNote {
    pub fn native() -> Self {
        Self {
            kind: "native".into(),
            target: String::new(),
        }
    }

    fn powershell(&self) -> bool {
        cfg!(windows) && self.kind == "native"
    }

    fn lifecycle_sentence(&self) -> String {
        match self.kind.as_str() {
            "tmux" => format!(
                "The command runs in tmux session `{}`; it keeps running if this executor is interrupted or exits.",
                self.target
            ),
            "remote_ssh" => format!(
                "The command runs on the remote host `{}`; it keeps running if this executor is interrupted or exits.",
                self.target
            ),
            _ => "The command is a child process of this executor: an interrupt or a timeout ends its process group; processes it leaves behind after returning are not managed.".into(),
        }
    }
}

/// User-facing target field. `None` / empty string means [`BashTarget::Local`];
/// other values are passed through [`BashTargetSpec::parse`] before reaching
/// the runner.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum BashTargetSpec {
    #[default]
    Local,
    Raw(String),
}

impl BashTargetSpec {
    pub fn parse(raw: Option<&str>) -> BashTarget {
        let Some(value) = raw else {
            return BashTarget::Local;
        };
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return BashTarget::Local;
        }
        match trimmed.to_ascii_lowercase().as_str() {
            "local" | "localhost" | "." => BashTarget::Local,
            _ => BashTarget::Unsupported(trimmed.to_string()),
        }
    }

    pub fn resolve(&self) -> BashTarget {
        match self {
            BashTargetSpec::Local => BashTarget::Local,
            BashTargetSpec::Raw(raw) => Self::parse(Some(raw.as_str())),
        }
    }
}

/// Internal, structured execution target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BashTarget {
    Local,
    Unsupported(String),
}

impl BashTarget {
    pub fn label(&self) -> &str {
        match self {
            BashTarget::Local => "local",
            BashTarget::Unsupported(value) => value.as_str(),
        }
    }
}

/// PATH overlay: an ordered list of bin directories prepended to `PATH`.
/// `layers[0]` has the highest precedence.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BinOverlayConfig {
    pub layers: Vec<PathBuf>,
    pub enabled: bool,
}

impl BinOverlayConfig {
    pub fn disabled() -> Self {
        Self {
            layers: Vec::new(),
            enabled: false,
        }
    }

    pub fn local(bin_dir: impl Into<PathBuf>) -> Self {
        Self {
            layers: vec![bin_dir.into()],
            enabled: true,
        }
    }

    pub fn layered<I, P>(layers: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<PathBuf>,
    {
        let layers: Vec<PathBuf> = layers.into_iter().map(Into::into).collect();
        Self {
            enabled: !layers.is_empty(),
            layers,
        }
    }

    fn active_layers(&self) -> &[PathBuf] {
        if !self.enabled {
            return &[];
        }
        &self.layers
    }
}

#[derive(Clone)]
pub struct LlmBashConfig {
    pub workspace: PathBuf,
    pub restrict_cwd: bool,
    /// `wait` mode: default `timeout_ms`.
    pub default_timeout_ms: u64,
    /// `wait` mode: upper bound of `timeout_ms`; 0 = unlimited.
    pub max_timeout_ms: u64,
    pub max_output_bytes: usize,
    pub allow_env: bool,
    pub target: BashTargetSpec,
    pub overlay: BinOverlayConfig,
    pub tool_name: String,
    pub mode: ShellMode,
    /// `auto` mode: default `wait_ms`.
    pub default_wait_ms: u64,
    pub runtime: ShellRuntimeNote,
    /// `auto` mode: where a command that outlives `wait_ms` goes. Without
    /// it the tool behaves as `wait`.
    pub tasks: Option<Arc<InProcessTaskManager>>,
}

impl std::fmt::Debug for LlmBashConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmBashConfig")
            .field("workspace", &self.workspace)
            .field("restrict_cwd", &self.restrict_cwd)
            .field("default_timeout_ms", &self.default_timeout_ms)
            .field("max_timeout_ms", &self.max_timeout_ms)
            .field("max_output_bytes", &self.max_output_bytes)
            .field("allow_env", &self.allow_env)
            .field("target", &self.target)
            .field("overlay", &self.overlay)
            .field("tool_name", &self.tool_name)
            .field("mode", &self.mode)
            .field("default_wait_ms", &self.default_wait_ms)
            .field("runtime", &self.runtime)
            .field("tasks", &self.tasks.is_some())
            .finish()
    }
}

impl LlmBashConfig {
    pub fn local_workspace(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
            restrict_cwd: true,
            default_timeout_ms: DEFAULT_TIMEOUT_MS,
            max_timeout_ms: DEFAULT_MAX_TIMEOUT_MS,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
            allow_env: true,
            target: BashTargetSpec::Local,
            overlay: BinOverlayConfig::disabled(),
            tool_name: TOOL_SHELL.to_string(),
            mode: ShellMode::Wait,
            default_wait_ms: DEFAULT_AUTO_WAIT_MS,
            runtime: ShellRuntimeNote::native(),
            tasks: None,
        }
    }

    pub fn with_tool_name(mut self, name: impl Into<String>) -> Self {
        self.tool_name = name.into();
        self
    }

    pub fn with_overlay(mut self, overlay: BinOverlayConfig) -> Self {
        self.overlay = overlay;
        self
    }

    pub fn with_default_timeout_ms(mut self, ms: u64) -> Self {
        self.default_timeout_ms = ms;
        self
    }

    /// 0 = unlimited.
    pub fn with_max_timeout_ms(mut self, ms: u64) -> Self {
        self.max_timeout_ms = ms;
        self
    }

    pub fn with_max_output_bytes(mut self, bytes: usize) -> Self {
        self.max_output_bytes = bytes;
        self
    }

    pub fn with_allow_env(mut self, allow: bool) -> Self {
        self.allow_env = allow;
        self
    }

    pub fn with_restrict_cwd(mut self, restrict: bool) -> Self {
        self.restrict_cwd = restrict;
        self
    }

    pub fn with_mode(mut self, mode: ShellMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn with_default_wait_ms(mut self, ms: u64) -> Self {
        self.default_wait_ms = ms.clamp(1, MAX_AUTO_WAIT_MS);
        self
    }

    pub fn with_runtime_note(mut self, note: ShellRuntimeNote) -> Self {
        self.runtime = note;
        self
    }

    pub fn with_tasks(mut self, tasks: Arc<InProcessTaskManager>) -> Self {
        self.tasks = Some(tasks);
        self
    }

    fn effective_mode(&self) -> ShellMode {
        match self.mode {
            ShellMode::Auto if self.tasks.is_some() => ShellMode::Auto,
            _ => ShellMode::Wait,
        }
    }
}

/// One command handed to a [`BashRunner`].
#[derive(Clone, Debug)]
pub struct BashRunRequest {
    pub command: String,
    pub cwd: PathBuf,
    /// `BashRunner::run` only: how long to wait before stopping the command.
    pub timeout_ms: u64,
    pub max_output_bytes: usize,
    pub env: Vec<(String, String)>,
    pub target: BashTarget,
    /// The tool call this command belongs to: names its execution directory.
    pub call_id: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct BashRunOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub output: String,
    pub output_truncated: bool,
    pub timed_out: bool,
    pub duration_ms: u64,
    pub engine: String,
    pub cwd: PathBuf,
}

/// Progress of a running command.
#[derive(Clone, Debug, Default)]
pub struct CommandProgress {
    pub elapsed_ms: u64,
    pub output_tail: String,
}

/// A started command. Dropping the handle without `detach` stops a native
/// command (process group guard); tmux / remote commands keep running.
#[async_trait]
pub trait CommandHandle: Send + Sync {
    /// Wait until the command exits or `timeout` passes (`Ok(None)`: still
    /// running).
    async fn wait(&mut self, timeout: Duration) -> Result<Option<BashRunOutput>, AgentToolError>;

    /// Elapsed time and the tail of the output so far.
    async fn progress(&self) -> CommandProgress;

    /// Stop the command and return the output so far (`timed_out = true`).
    async fn kill(&mut self) -> Result<BashRunOutput, AgentToolError>;

    /// Give the command up: it keeps running after this handle is dropped.
    fn detach(&mut self);

    /// Whether `kill` stops the command reliably.
    fn cancellable(&self) -> bool;

    /// Where the command lives / how to look at it, for texts shown to the
    /// model (e.g. `tmux window llm-c1 of session x; output in <dir>`).
    fn locator(&self) -> String;
}

/// A command that already ran to the end (runners implementing only
/// `BashRunner::run`).
pub struct FinishedCommand(pub Option<BashRunOutput>);

#[async_trait]
impl CommandHandle for FinishedCommand {
    async fn wait(&mut self, _timeout: Duration) -> Result<Option<BashRunOutput>, AgentToolError> {
        Ok(self.0.take())
    }
    async fn progress(&self) -> CommandProgress {
        CommandProgress::default()
    }
    async fn kill(&mut self) -> Result<BashRunOutput, AgentToolError> {
        Ok(self.0.take().unwrap_or_default())
    }
    fn detach(&mut self) {}
    fn cancellable(&self) -> bool {
        true
    }
    fn locator(&self) -> String {
        String::new()
    }
}

/// Command runner of one runtime. Implement `start` (preferred: enables the
/// `auto` mode, cancellation and progress) or `run`; the defaults derive one
/// from the other.
#[async_trait]
pub trait BashRunner: Send + Sync {
    async fn resolve_cwd(
        &self,
        root: &std::path::Path,
        raw: Option<&str>,
        restricted: bool,
    ) -> Result<PathBuf, AgentToolError> {
        use crate::runtime::files::{FileBackend, LocalFileBackend};
        let root = to_abs_path(root)?;
        let allowed = if restricted {
            vec![root.clone()]
        } else {
            Vec::new()
        };
        let path = LocalFileBackend
            .resolve(&root, raw.unwrap_or("."), &allowed)
            .await?;
        if !path.is_dir() {
            return Err(AgentToolError::InvalidArgs(format!(
                "cwd does not exist: {}",
                path.display()
            )));
        }
        Ok(path)
    }

    /// Start the command and return its handle.
    async fn start(
        &self,
        ctx: &SessionRuntimeContext,
        req: BashRunRequest,
    ) -> Result<Box<dyn CommandHandle>, AgentToolError> {
        let out = self.run(ctx, req).await?;
        Ok(Box::new(FinishedCommand(Some(out))))
    }

    /// Run to completion or `req.timeout_ms` (then the command is stopped).
    async fn run(
        &self,
        ctx: &SessionRuntimeContext,
        req: BashRunRequest,
    ) -> Result<BashRunOutput, AgentToolError> {
        let timeout = wait_duration(req.timeout_ms);
        let mut handle = self.start(ctx, req).await?;
        match handle.wait(timeout).await? {
            Some(out) => Ok(out),
            None => handle.kill().await,
        }
    }

    /// Engine label of this runner (`native | tmux | remote_ssh`).
    fn engine(&self) -> &str {
        LOCAL_ENGINE
    }
}

pub(crate) fn wait_duration(ms: u64) -> Duration {
    if ms == 0 {
        UNLIMITED_TIMEOUT
    } else {
        Duration::from_millis(ms)
    }
}

/// Build the env list applied to the spawned shell (overlay layers first).
pub fn prepare_overlay_env(
    overlay: &BinOverlayConfig,
    user_env: &[(String, String)],
) -> Vec<(String, String)> {
    prepare_shell_env(overlay, user_env, cfg!(windows))
}

fn prepare_shell_env(
    overlay: &BinOverlayConfig,
    user_env: &[(String, String)],
    powershell: bool,
) -> Vec<(String, String)> {
    let mut merged = BTreeMap::<String, String>::new();
    for (key, value) in user_env {
        merged.insert(key.clone(), value.clone());
    }

    let base_path = merged
        .get("PATH")
        .cloned()
        .or_else(|| std::env::var("PATH").ok())
        .unwrap_or_default();
    let mut path = ensure_system_path_entries(&base_path, powershell);

    let active = overlay.active_layers();
    if !active.is_empty() {
        for layer in active.iter().rev() {
            let entry = layer.display().to_string();
            path = prepend_path_entry(&entry, &path, powershell);
        }
    }
    merged.insert("PATH".to_string(), path);

    merged.into_iter().collect()
}

fn ensure_system_path_entries(base_path: &str, powershell: bool) -> String {
    const SYSTEM_PATH_ENTRIES: [&str; 6] = [
        "/usr/local/sbin",
        "/usr/local/bin",
        "/usr/sbin",
        "/usr/bin",
        "/sbin",
        "/bin",
    ];

    let mut path = base_path.trim().to_string();
    if powershell {
        let root =
            PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()));
        for entry in [
            root.join("System32"),
            root.clone(),
            root.join("System32/WindowsPowerShell/v1.0"),
        ] {
            path = append_path_entry(&entry.display().to_string(), &path, true);
        }
    } else {
        for entry in SYSTEM_PATH_ENTRIES {
            path = append_path_entry(entry, &path, false);
        }
    }
    path
}

fn append_path_entry(entry: &str, base_path: &str, powershell: bool) -> String {
    let entry = entry.trim();
    if entry.is_empty() {
        return base_path.to_string();
    }
    if base_path.is_empty() {
        return entry.to_string();
    }
    let separator = if powershell { ';' } else { ':' };
    if base_path.split(separator).any(|item| item == entry) {
        return base_path.to_string();
    }
    format!("{base_path}{separator}{entry}")
}

fn prepend_path_entry(entry: &str, base_path: &str, powershell: bool) -> String {
    let entry = entry.trim();
    if entry.is_empty() {
        return base_path.to_string();
    }
    if base_path.is_empty() {
        return entry.to_string();
    }
    let separator = if powershell { ';' } else { ':' };
    if base_path.split(separator).any(|item| item == entry) {
        return base_path.to_string();
    }
    format!("{entry}{separator}{base_path}")
}

/// Bounded output buffer keeping the head and the tail of a stream; the
/// tail gets the larger share because errors usually land at the end.
pub(crate) struct OutputCollector {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    head_cap: usize,
    tail_cap: usize,
    total: usize,
}

impl OutputCollector {
    pub(crate) fn new(max_bytes: usize) -> Self {
        let head_cap = max_bytes / 4;
        Self {
            head: Vec::new(),
            tail: VecDeque::new(),
            head_cap,
            tail_cap: max_bytes - head_cap,
            total: 0,
        }
    }

    pub(crate) fn push(&mut self, mut data: &[u8]) {
        self.total += data.len();
        if self.head.len() < self.head_cap {
            let n = (self.head_cap - self.head.len()).min(data.len());
            self.head.extend_from_slice(&data[..n]);
            data = &data[n..];
        }
        self.tail.extend(data);
        let overflow = self.tail.len().saturating_sub(self.tail_cap);
        self.tail.drain(..overflow);
    }

    pub(crate) fn is_truncated(&self) -> bool {
        self.total > self.head.len() + self.tail.len()
    }

    pub(crate) fn render(&self) -> String {
        let (a, b) = self.tail.as_slices();
        let mut tail = Vec::with_capacity(self.tail.len());
        tail.extend_from_slice(a);
        tail.extend_from_slice(b);
        let head = String::from_utf8_lossy(&self.head);
        let tail = String::from_utf8_lossy(&tail);
        if self.is_truncated() {
            let omitted = self.total - self.head.len() - self.tail.len();
            format!("{head}\n…[{omitted} bytes omitted]…\n{tail}")
        } else {
            format!("{head}{tail}")
        }
    }
}

pub(crate) async fn drain_pipe<R: AsyncRead + Unpin>(
    mut reader: R,
    collector: Arc<Mutex<OutputCollector>>,
) {
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => collector.lock().expect("bash output lock").push(&buf[..n]),
        }
    }
}

/// Head + tail of a file, bounded by `max` bytes, with the omitted count.
pub(crate) fn read_file_bounded(path: &Path, max: usize) -> (String, bool) {
    let Ok(bytes) = std::fs::read(path) else {
        return (String::new(), false);
    };
    let mut c = OutputCollector::new(max.max(16));
    c.push(&bytes);
    (c.render(), c.is_truncated())
}

/// Last `max` bytes of a file as text.
pub(crate) fn read_file_tail(path: &Path, max: usize) -> String {
    let Ok(bytes) = std::fs::read(path) else {
        return String::new();
    };
    let start = bytes.len().saturating_sub(max);
    String::from_utf8_lossy(&bytes[start..]).to_string()
}

/// Assemble a [`BashRunOutput`] from the files of an execution directory.
pub(crate) fn output_from_exec_dir(
    dir: &Path,
    exit_code: i32,
    timed_out: bool,
    elapsed: Duration,
    max: usize,
    engine: &str,
    cwd: PathBuf,
) -> BashRunOutput {
    let (stdout, out_trunc) = read_file_bounded(&dir.join("stdout"), max);
    let (stderr, err_trunc) = read_file_bounded(&dir.join("stderr"), max);
    let mut output = stdout.clone();
    if !stderr.is_empty() {
        if !output.is_empty() && !output.ends_with('\n') {
            output.push('\n');
        }
        output.push_str(&stderr);
    }
    BashRunOutput {
        exit_code,
        stdout,
        stderr,
        output,
        output_truncated: out_trunc || err_trunc,
        timed_out,
        duration_ms: elapsed.as_millis() as u64,
        engine: engine.into(),
        cwd,
    }
}

/// Exit code recorded by a wrapper in `<dir>/exit`, if the command ended.
pub fn read_exit_file(dir: &Path) -> Option<i32> {
    std::fs::read_to_string(dir.join("exit"))
        .ok()
        .and_then(|s| s.trim().parse::<i32>().ok())
}

fn live_process_groups() -> &'static Mutex<std::collections::BTreeSet<u32>> {
    static GROUPS: OnceLock<Mutex<std::collections::BTreeSet<u32>>> = OnceLock::new();
    GROUPS.get_or_init(|| Mutex::new(std::collections::BTreeSet::new()))
}

pub(crate) fn kill_process_group(pgid: u32) {
    let _ = std::process::Command::new("/bin/bash")
        .arg("-c")
        .arg(format!("kill -KILL -- -{pgid}"))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// SIGKILL every process group of a native `shell` command still owned by
/// this process. For hosts that are about to exit without unwinding (e.g. a
/// second Ctrl-C). Commands handed to the task manager are not owned any
/// more and are left alone.
pub fn kill_running_bash_process_groups() {
    #[cfg(windows)]
    for job in windows_process_groups()
        .lock()
        .expect("process group lock")
        .iter()
    {
        unsafe {
            TerminateJobObject(*job as _, TIMEOUT_EXIT_CODE as u32);
        }
    }
    let groups: Vec<u32> = live_process_groups()
        .lock()
        .expect("process group lock")
        .iter()
        .copied()
        .collect();
    for pgid in groups {
        kill_process_group(pgid);
    }
}

/// Kills the command's whole process group when the owning handle is
/// dropped (timeout, cancel, the caller dropping the future). Disarmed once
/// bash exits or the command is detached into a task.
pub(crate) struct ProcessGroupGuard {
    pgid: Option<u32>,
    #[cfg(windows)]
    job: Option<usize>,
    #[cfg(windows)]
    kill_on_drop: bool,
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn CreateJobObjectW(
        attributes: *const std::ffi::c_void,
        name: *const u16,
    ) -> *mut std::ffi::c_void;
    fn AssignProcessToJobObject(job: *mut std::ffi::c_void, process: *mut std::ffi::c_void) -> i32;
    fn TerminateJobObject(job: *mut std::ffi::c_void, exit_code: u32) -> i32;
    fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
}

#[cfg(windows)]
fn windows_process_groups() -> &'static Mutex<std::collections::BTreeSet<usize>> {
    static JOBS: OnceLock<Mutex<std::collections::BTreeSet<usize>>> = OnceLock::new();
    JOBS.get_or_init(|| Mutex::new(std::collections::BTreeSet::new()))
}

impl ProcessGroupGuard {
    pub(crate) fn new(pgid: Option<u32>) -> Self {
        if let Some(id) = pgid {
            live_process_groups()
                .lock()
                .expect("process group lock")
                .insert(id);
        }
        Self {
            pgid,
            #[cfg(windows)]
            job: None,
            #[cfg(windows)]
            kill_on_drop: true,
        }
    }

    #[cfg(windows)]
    fn attach(&mut self, child: &tokio::process::Child) -> std::io::Result<()> {
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let process = child
            .raw_handle()
            .ok_or_else(|| std::io::Error::other("shell exited before job assignment"));
        let result = process.and_then(|process| {
            if unsafe { AssignProcessToJobObject(job, process) } == 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
        if let Err(err) = result {
            unsafe {
                CloseHandle(job);
            }
            return Err(err);
        }
        self.job = Some(job as usize);
        windows_process_groups()
            .lock()
            .expect("process group lock")
            .insert(job as usize);
        Ok(())
    }

    pub(crate) fn kill(&mut self) {
        #[cfg(windows)]
        if let Some(job) = self.job {
            unsafe {
                TerminateJobObject(job as _, TIMEOUT_EXIT_CODE as u32);
            }
            self.disarm();
        }
        if let Some(id) = self.pgid.take() {
            kill_process_group(id);
            live_process_groups()
                .lock()
                .expect("process group lock")
                .remove(&id);
        }
    }

    pub(crate) fn disarm(&mut self) {
        #[cfg(windows)]
        if let Some(job) = self.job.take() {
            windows_process_groups()
                .lock()
                .expect("process group lock")
                .remove(&job);
            unsafe {
                CloseHandle(job as _);
            }
        }
        if let Some(id) = self.pgid.take() {
            live_process_groups()
                .lock()
                .expect("process group lock")
                .remove(&id);
        }
    }

    #[cfg(windows)]
    fn detach(&mut self) {
        self.kill_on_drop = false;
        if let Some(job) = self.job {
            windows_process_groups()
                .lock()
                .expect("process group lock")
                .remove(&job);
        }
    }
}

impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        if !self.kill_on_drop {
            self.disarm();
            return;
        }
        self.kill();
    }
}

fn is_valid_shell_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

/// Wrapper run as `bash -c WRAPPER <command> <exec_dir>`: the command's
/// output goes to files, its exit code to `exit`, so the command outlives
/// this executor without SIGPIPE and its result survives a crash.
#[cfg(not(windows))]
const NATIVE_WRAPPER: &str = r#"if [ "$#" -ge 2 ]; then export PATH="$2"; fi
printf '%s\n' "$0" > "$1/command"
"$BASH" -c "$0" </dev/null >"$1/stdout" 2>"$1/stderr"
__llm_ec=$?
printf '%s\n' "$__llm_ec" > "$1/exit.tmp" && command -p mv -f "$1/exit.tmp" "$1/exit"
exit $__llm_ec"#;

#[cfg(windows)]
const NATIVE_WRAPPER: &str = r#"$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$utf8 = [System.Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding = $utf8
$dir = $env:XLLM_EXEC_DIR
$stderr = Join-Path $dir 'stderr'
try {
    $script = Join-Path $dir 'script.ps1'
    $arguments = '-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -OutputFormat Text -File "' + $script + '"'
    $process = Start-Process -FilePath $env:XLLM_EXEC_SHELL -ArgumentList $arguments -WorkingDirectory $env:XLLM_EXEC_CWD -NoNewWindow -PassThru -RedirectStandardOutput (Join-Path $dir 'stdout') -RedirectStandardError $stderr
    $null = $process.Handle
    $process.WaitForExit()
    $code = $process.ExitCode
} catch {
    [System.IO.File]::WriteAllText($stderr, $_.ToString(), $utf8)
    $code = 1
}
$tmp = Join-Path $dir 'exit.tmp'
[System.IO.File]::WriteAllText($tmp, [string]$code, $utf8)
[System.IO.File]::Move($tmp, (Join-Path $dir 'exit'))
exit $code"#;

#[cfg(windows)]
const POWERSHELL_SCRIPT_PREFIX: &str = r#"$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$utf8 = [System.Text.UTF8Encoding]::new($false)
[Console]::InputEncoding = $utf8
[Console]::OutputEncoding = $utf8
$OutputEncoding = $utf8
$global:LASTEXITCODE = 0
try {
    & {
"#;

#[cfg(windows)]
const POWERSHELL_SCRIPT_SUFFIX: &str = r#"
    }
    exit $LASTEXITCODE
} catch {
    [Console]::Error.WriteLine($_.ToString())
    exit 1
}
"#;

pub(crate) fn native_shell_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    #[cfg(windows)]
    {
        let path = path.strip_prefix(r"\\?\").unwrap_or(&path);
        path.strip_prefix(r"UNC\")
            .map(|unc| format!(r"\\{unc}"))
            .unwrap_or_else(|| path.to_string())
    }
    #[cfg(not(windows))]
    path.into_owned()
}

pub(crate) fn native_shell_executable() -> PathBuf {
    #[cfg(windows)]
    if let Some(path) = std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join("pwsh.exe"))
            .find(|file| file.is_file())
    }) {
        return path;
    }
    #[cfg(windows)]
    {
        return PathBuf::from(
            std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()),
        )
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    }
    #[cfg(not(windows))]
    PathBuf::from("/bin/bash")
}

pub(crate) fn native_shell_command(script: &str) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(native_shell_executable());
    #[cfg(windows)]
    {
        use base64::Engine;
        let bytes: Vec<u8> = script
            .encode_utf16()
            .flat_map(|unit| unit.to_le_bytes())
            .collect();
        command
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-OutputFormat",
                "Text",
                "-EncodedCommand",
            ])
            .arg(base64::engine::general_purpose::STANDARD.encode(bytes))
            .creation_flags(0x08000000);
    }
    #[cfg(not(windows))]
    command.arg("-c").arg(script);
    command
}

/// Default [`BashRunner`]: spawns `/bin/bash` in its own process group (Unix)
/// with its output redirected into the execution directory. Only the
/// command in progress is managed; on timeout / cancel the whole group is
/// killed, after a normal return nothing is tracked.
#[derive(Clone, Debug, Default)]
pub struct LocalProcessBashRunner {
    /// PATH layers prepended in order (`layers[0]` first).
    path_layers: Vec<PathBuf>,
    extra_env: Vec<(String, String)>,
    run: Option<RunBindingSlot>,
}

impl LocalProcessBashRunner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_path_layers(mut self, layers: Vec<PathBuf>) -> Self {
        self.path_layers = layers;
        self
    }

    pub fn with_env(mut self, env: Vec<(String, String)>) -> Self {
        self.extra_env = env;
        self
    }

    pub fn with_run_binding(mut self, run: RunBindingSlot) -> Self {
        self.run = Some(run);
        self
    }

    fn build_path(&self, user_env: &[(String, String)]) -> Option<String> {
        if self.path_layers.is_empty() && self.extra_env.is_empty() {
            return None;
        }
        let base = user_env
            .iter()
            .rev()
            .find(|(k, _)| k == "PATH")
            .map(|(_, v)| v.clone())
            .or_else(|| {
                self.extra_env
                    .iter()
                    .rev()
                    .find(|(k, _)| k == "PATH")
                    .map(|(_, v)| v.clone())
            })
            .or_else(|| std::env::var("PATH").ok())
            .unwrap_or_else(|| "/usr/local/bin:/usr/bin:/bin".to_string());
        let mut parts: Vec<String> = self
            .path_layers
            .iter()
            .map(|p| native_shell_path(p))
            .collect();
        let separator = if cfg!(windows) { ';' } else { ':' };
        for p in base.split(separator) {
            if !p.is_empty() && !parts.iter().any(|x| x == p) {
                parts.push(p.to_string());
            }
        }
        Some(parts.join(&separator.to_string()))
    }

    fn exec_dir(&self, call_id: Option<&str>) -> (PathBuf, bool) {
        let bound = self
            .run
            .as_ref()
            .and_then(|slot| slot.lock().expect("run binding").clone())
            .and_then(|b| exec_dir_for(b.run_dir.as_deref(), call_id));
        match bound {
            Some(dir) => (dir, false),
            None => (
                std::env::temp_dir().join(format!(
                    "llm-shell-{}-{}",
                    std::process::id(),
                    crate::tasks::next_local_seq()
                )),
                true,
            ),
        }
    }
}

#[async_trait]
impl BashRunner for LocalProcessBashRunner {
    async fn start(
        &self,
        _ctx: &SessionRuntimeContext,
        req: BashRunRequest,
    ) -> Result<Box<dyn CommandHandle>, AgentToolError> {
        match req.target {
            BashTarget::Local => {}
            BashTarget::Unsupported(value) => {
                return Err(AgentToolError::InvalidArgs(format!(
                    "unsupported shell target `{value}` (only local is supported)"
                )));
            }
        }
        let (dir, temp) = self.exec_dir(req.call_id.as_deref());
        let mut builder = tokio::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        builder.mode(0o700);
        builder
            .create(&dir)
            .await
            .map_err(|e| AgentToolError::ExecFailed(format!("execution directory: {e}")))?;
        for stale in ["exit", "exit.tmp"] {
            let _ = std::fs::remove_file(dir.join(stale));
        }

        // `-c` rather than `-lc`: a login shell sources profile files which
        // would reorder PATH under the overlay.
        #[cfg(windows)]
        {
            std::fs::write(dir.join("command"), &req.command)
                .and_then(|_| {
                    std::fs::write(
                        dir.join("script.ps1"),
                        format!(
                            "\u{feff}{POWERSHELL_SCRIPT_PREFIX}{}{POWERSHELL_SCRIPT_SUFFIX}",
                            req.command
                        ),
                    )
                })
                .map_err(|err| {
                    AgentToolError::ExecFailed(format!("write shell script failed: {err}"))
                })?;
        }
        let mut cmd = native_shell_command(NATIVE_WRAPPER);
        #[cfg(not(windows))]
        cmd.arg(&req.command).arg(native_shell_path(&dir));
        cmd.current_dir(&req.cwd);
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());
        #[cfg(unix)]
        cmd.process_group(0);
        #[cfg(not(unix))]
        cmd.kill_on_drop(true);
        for (k, v) in &self.extra_env {
            cmd.env(k, v);
        }
        for (k, v) in &req.env {
            cmd.env(k, v);
        }
        let path = self.build_path(&req.env);
        if let Some(path) = &path {
            cmd.env("PATH", path);
        }
        #[cfg(windows)]
        {
            cmd.env("XLLM_EXEC_DIR", native_shell_path(&dir))
                .env("XLLM_EXEC_CWD", native_shell_path(&req.cwd))
                .env("XLLM_EXEC_SHELL", native_shell_executable());
        }

        let child = cmd
            .spawn()
            .map_err(|err| AgentToolError::ExecFailed(format!("spawn shell failed: {err}")))?;
        let pgid = if cfg!(unix) { child.id() } else { None };
        let guard = ProcessGroupGuard::new(pgid);
        #[cfg(windows)]
        let guard = {
            let mut guard = guard;
            guard.attach(&child).map_err(|err| {
                AgentToolError::ExecFailed(format!("assign shell process group failed: {err}"))
            })?;
            guard
        };
        Ok(Box::new(LocalCommand {
            child: Some(child),
            guard,
            dir,
            temp,
            started: Instant::now(),
            max_output: req.max_output_bytes,
            cwd: req.cwd,
            finished: None,
        }))
    }
}

/// A native command in progress.
pub struct LocalCommand {
    child: Option<tokio::process::Child>,
    guard: ProcessGroupGuard,
    dir: PathBuf,
    temp: bool,
    started: Instant,
    max_output: usize,
    cwd: PathBuf,
    finished: Option<BashRunOutput>,
}

impl LocalCommand {
    fn output(&self, exit_code: i32, timed_out: bool) -> BashRunOutput {
        output_from_exec_dir(
            &self.dir,
            exit_code,
            timed_out,
            self.started.elapsed(),
            self.max_output,
            LOCAL_ENGINE,
            self.cwd.clone(),
        )
    }

    fn cleanup(&self) {
        if self.temp {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn exit_code_of(status: std::process::ExitStatus) -> i32 {
        status.code().unwrap_or_else(|| {
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                status.signal().map(|sig| 128 + sig).unwrap_or(-1)
            }
            #[cfg(not(unix))]
            {
                -1
            }
        })
    }
}

#[async_trait]
impl CommandHandle for LocalCommand {
    async fn wait(&mut self, timeout: Duration) -> Result<Option<BashRunOutput>, AgentToolError> {
        if let Some(done) = &self.finished {
            return Ok(Some(done.clone()));
        }
        let Some(child) = self.child.as_mut() else {
            return Ok(None);
        };
        match tokio_timeout(timeout, child.wait()).await {
            Ok(Ok(status)) => {
                self.guard.disarm();
                let out = self.output(Self::exit_code_of(status), false);
                self.cleanup();
                self.finished = Some(out.clone());
                Ok(Some(out))
            }
            Ok(Err(err)) => Err(AgentToolError::ExecFailed(format!(
                "wait shell failed: {err}"
            ))),
            Err(_) => Ok(None),
        }
    }

    async fn progress(&self) -> CommandProgress {
        CommandProgress {
            elapsed_ms: self.started.elapsed().as_millis() as u64,
            output_tail: read_file_tail(&self.dir.join("stdout"), OUTPUT_TAIL_BYTES),
        }
    }

    async fn kill(&mut self) -> Result<BashRunOutput, AgentToolError> {
        if let Some(done) = &self.finished {
            return Ok(done.clone());
        }
        self.guard.kill();
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill().await;
            let _ = tokio_timeout(Duration::from_secs(5), child.wait()).await;
        }
        let out = self.output(TIMEOUT_EXIT_CODE, true);
        self.cleanup();
        self.finished = Some(out.clone());
        Ok(out)
    }

    fn detach(&mut self) {
        #[cfg(windows)]
        self.guard.detach();
        #[cfg(not(windows))]
        self.guard.disarm();
    }

    fn cancellable(&self) -> bool {
        true
    }

    fn locator(&self) -> String {
        format!("execution directory {}", self.dir.display())
    }
}

/// `AgentTool` implementation backed by a [`BashRunner`].
pub struct ShellTool {
    config: LlmBashConfig,
    runner: Arc<dyn BashRunner>,
}

/// Cause text for a cancelled command.
fn cause_text(cause: CancelCause) -> &'static str {
    match cause {
        CancelCause::Interrupted => "the run was interrupted",
        CancelCause::Finishing => "the run is finishing",
        CancelCause::Deadline => "the run's total time limit was reached",
    }
}

impl ShellTool {
    pub fn new(config: LlmBashConfig) -> Self {
        Self {
            config,
            runner: Arc::new(LocalProcessBashRunner::new()),
        }
    }

    pub fn with_runner(config: LlmBashConfig, runner: Arc<dyn BashRunner>) -> Self {
        Self { config, runner }
    }

    pub fn local_workspace(workspace: impl Into<PathBuf>) -> Self {
        Self::new(LlmBashConfig::local_workspace(workspace))
    }

    pub fn config(&self) -> &LlmBashConfig {
        &self.config
    }

    fn tool_name(&self) -> &str {
        self.config.tool_name.as_str()
    }

    async fn resolve_cwd(&self, raw: Option<&str>) -> Result<PathBuf, AgentToolError> {
        self.runner
            .resolve_cwd(&self.config.workspace, raw, self.config.restrict_cwd)
            .await
    }

    /// `wait` mode `timeout_ms`: default, clamped to the configured maximum
    /// (0 = unlimited).
    fn resolve_timeout(&self, raw: Option<u64>) -> u64 {
        let candidate = raw.unwrap_or(self.config.default_timeout_ms);
        if self.config.max_timeout_ms == 0 {
            candidate
        } else {
            candidate.clamp(1, self.config.max_timeout_ms)
        }
    }

    fn parse_ms(&self, raw: Option<&Json>, name: &str) -> Result<Option<u64>, AgentToolError> {
        let value = match raw {
            None | Some(Json::Null) => None,
            Some(Json::Number(value)) => value.as_u64(),
            Some(Json::String(value)) => value.trim().parse::<u64>().ok(),
            Some(_) => None,
        };
        if raw.is_some_and(|raw| !raw.is_null()) && value.is_none() {
            return Err(AgentToolError::InvalidArgs(format!(
                "`{name}` must be a non-negative integer"
            )));
        }
        Ok(value)
    }

    fn parse_timeout(&self, raw: Option<&Json>) -> Result<u64, AgentToolError> {
        let value = self.parse_ms(raw, "timeout_ms")?;
        if value == Some(0) && self.config.max_timeout_ms != 0 {
            return Err(AgentToolError::InvalidArgs(
                "`timeout_ms` must be a positive integer".to_string(),
            ));
        }
        Ok(self.resolve_timeout(value))
    }

    fn parse_wait(&self, raw: Option<&Json>) -> Result<u64, AgentToolError> {
        let value = self.parse_ms(raw, "wait_ms")?;
        Ok(value
            .unwrap_or(self.config.default_wait_ms)
            .clamp(1, MAX_AUTO_WAIT_MS))
    }

    fn parse_env(&self, raw: Option<&Json>) -> Result<Vec<(String, String)>, AgentToolError> {
        let Some(value) = raw else {
            return Ok(Vec::new());
        };
        if value.is_null() {
            return Ok(Vec::new());
        }
        let map = value.as_object().ok_or_else(|| {
            AgentToolError::InvalidArgs("`env` must be an object of string=>string".to_string())
        })?;
        if map.is_empty() {
            return Ok(Vec::new());
        }
        if !self.config.allow_env {
            return Err(AgentToolError::InvalidArgs(
                "env passing is disabled for this shell tool".to_string(),
            ));
        }
        let mut out = Vec::with_capacity(map.len());
        for (key, val) in map {
            if !is_valid_shell_env_key(key) {
                return Err(AgentToolError::InvalidArgs(format!(
                    "invalid env key `{key}` (must match [A-Za-z_][A-Za-z0-9_]*)"
                )));
            }
            let value_str = match val {
                Json::String(s) => s.clone(),
                Json::Number(n) => n.to_string(),
                Json::Bool(b) => b.to_string(),
                Json::Null => String::new(),
                other => {
                    return Err(AgentToolError::InvalidArgs(format!(
                        "env value for `{key}` must be string/number/bool, got {}",
                        other
                    )));
                }
            };
            out.push((key.clone(), value_str));
        }
        Ok(out)
    }

    fn build_details(&self, command: &str, output: &BashRunOutput) -> Json {
        json!({
            "command": command,
            "cwd": output.cwd.to_string_lossy().to_string(),
            "exit_code": output.exit_code,
            "stdout": output.stdout,
            "stderr": output.stderr,
            "output": output.output,
            "output_truncated": output.output_truncated,
            "timed_out": output.timed_out,
            "duration_ms": output.duration_ms,
            "engine": output.engine,
            "runtime": self.config.runtime.kind,
        })
    }

    fn try_forward_inner_agent_tool_result(
        &self,
        command: &str,
        output: &BashRunOutput,
    ) -> Option<AgentToolResult> {
        if !command_is_simple_for_protocol_forward(command) {
            return None;
        }

        let stdout = output.stdout.trim();
        if stdout.is_empty() {
            return None;
        }

        let mut result = match serde_json::from_str::<AgentToolResult>(stdout) {
            Ok(result) => result,
            Err(_) => return None,
        };

        if result.status == AgentToolStatus::Pending && result.task_id.is_none() {
            log::warn!(
                "shell inner AgentTool returned pending without task_id; falling back to the shell envelope command={}",
                command
            );
            return None;
        }

        if result.return_code.is_none() && output.exit_code != 0 {
            result.return_code = Some(output.exit_code);
        }

        Some(result)
    }

    fn finished_result(&self, command: &str, output: BashRunOutput, timeout_ms: u64) -> AgentToolResult {
        if !output.timed_out {
            if let Some(result) = self.try_forward_inner_agent_tool_result(command, &output) {
                return result;
            }
        }
        let summary = if output.timed_out {
            format!(
                "timed out after {}ms (timeout_ms={timeout_ms}); the command was stopped. Retry with a larger timeout_ms{}, or start a long-lived service with {} and return at once.",
                output.duration_ms,
                if self.config.max_timeout_ms == 0 {
                    String::new()
                } else {
                    format!(" (max {})", self.config.max_timeout_ms)
                },
                if self.config.runtime.powershell() { "Start-Process -WindowStyle Hidden without -Wait" } else { "nohup / setsid" }
            )
        } else if output.exit_code == 0 {
            format!("exit=0 in {}ms", output.duration_ms)
        } else {
            format!("exit={} in {}ms", output.exit_code, output.duration_ms)
        };
        let details = self.build_details(command, &output);
        let status = if output.exit_code == 0 && !output.timed_out {
            AgentToolStatus::Success
        } else {
            AgentToolStatus::Error
        };
        let fallback_cmd_line = format!("{} {}", self.tool_name(), command);
        let mut result = build_builtin_tool_result(details, fallback_cmd_line, summary)
            .with_tool(self.tool_name())
            .with_status(status)
            .with_return_code(output.exit_code)
            .refresh_default_title();
        if !output.output.is_empty() {
            result = result.with_output(output.output.clone());
        }
        result
    }

    /// The command was cancelled by the run (interrupt / finish / deadline):
    /// stop it when the runtime can, otherwise stop waiting; say what state
    /// the command is in. The error maps to `Observation::Cancelled`.
    async fn cancel_command(
        &self,
        mut handle: Box<dyn CommandHandle>,
        command: &str,
        cause: CancelCause,
    ) -> AgentToolError {
        let progress = handle.progress().await;
        let runtime = &self.config.runtime.kind;
        let tail = if progress.output_tail.trim().is_empty() {
            String::new()
        } else {
            format!(
                "\n--- output so far (tail) ---\n{}",
                progress.output_tail.trim_end()
            )
        };
        if runtime != "tmux" && handle.cancellable() {
            match handle.kill().await {
                Ok(out) => AgentToolError::Cancelled {
                    message: format!(
                        "shell ({runtime}): {} after {}ms because {}. It may have had partial side effects; check before repeating it.{tail}",
                        format!("`{command}` was stopped"),
                        out.duration_ms,
                        cause_text(cause)
                    ),
                    effect_unknown: false,
                },
                Err(e) => AgentToolError::Transport {
                    message: format!("cancelling `{command}` failed: {e}"),
                    effect_unknown: true,
                },
            }
        } else {
            let locator = handle.locator();
            handle.detach();
            AgentToolError::Cancelled {
                message: format!(
                    "shell ({runtime}): stopped waiting for `{command}` after {}ms because {}. The command is still running ({locator}); it may complete with side effects. Check its state before repeating it.{tail}",
                    progress.elapsed_ms,
                    cause_text(cause)
                ),
                effect_unknown: false,
            }
        }
    }

    /// `auto` mode: the command outlived `wait_ms`; it becomes a task.
    async fn detach_into_task(
        &self,
        handle: Box<dyn CommandHandle>,
        command: &str,
        call_id: Option<&str>,
        wait_ms: u64,
    ) -> AgentToolResult {
        let tasks = self.config.tasks.as_ref().expect("auto mode requires tasks");
        let progress = handle.progress().await;
        let cancellable = handle.cancellable();
        let locator = handle.locator();
        let task = ShellTask::new(command, handle, &self.config.runtime.kind);
        let task_id = tasks.register_shell(call_id, task);
        let mut text = format!(
            "`{command}` is still running after {wait_ms}ms ({} elapsed); it continues as task {task_id} ({locator}).",
            fmt_elapsed(progress.elapsed_ms)
        );
        if !progress.output_tail.trim().is_empty() {
            text.push_str("\n--- output so far (tail) ---\n");
            text.push_str(progress.output_tail.trim_end());
        }
        text.push('\n');
        text.push_str(&llm_context::tasks::next_step_hint(&task_id, cancellable));
        let details = json!({
            "command": command,
            "task_id": task_id,
            "elapsed_ms": progress.elapsed_ms,
            "runtime": self.config.runtime.kind,
            "still_running": true,
            "cancellable": cancellable,
        });
        let fallback_cmd_line = format!("{} {}", self.tool_name(), command);
        let mut result = build_builtin_tool_result(
            details,
            fallback_cmd_line,
            format!("still running as task {task_id}"),
        )
        .with_tool(self.tool_name())
        .with_status(AgentToolStatus::Success)
        .with_task_id(task_id)
        .refresh_default_title()
        .with_output(text);
        if !progress.output_tail.is_empty() {
            result = result.with_partial_output(progress.output_tail);
        }
        result
    }
}

pub(crate) fn fmt_elapsed(ms: u64) -> String {
    let s = ms / 1000;
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m{:02}s", s / 60, s % 60)
    } else {
        format!("{}h{:02}m", s / 3600, (s % 3600) / 60)
    }
}

fn command_is_simple_for_protocol_forward(command: &str) -> bool {
    if command_has_shell_operator(command) {
        return false;
    }
    crate::tokenize_bash_command_line(command)
        .map(|tokens| !tokens.is_empty())
        .unwrap_or(false)
}

fn command_has_shell_operator(command: &str) -> bool {
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    for ch in command.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if !in_single => {
                escaped = true;
            }
            '\'' if !in_double => {
                in_single = !in_single;
            }
            '"' if !in_single => {
                in_double = !in_double;
            }
            '|' | ';' | '&' | '>' | '<' | '\n' | '\r' if !in_single && !in_double => {
                return true;
            }
            _ => {}
        }
    }

    false
}

/// Pending cancellation of the current tool call, from the dispatcher's
/// task-local context (`None`: nothing ever cancels).
async fn current_cancel() -> CancelCause {
    match crate::runtime::CURRENT_TOOL_CTX.try_with(|c| c.clone()) {
        Ok(ctx) => ctx.cancelled().await,
        Err(_) => std::future::pending().await,
    }
}

#[async_trait]
impl AgentTool for ShellTool {
    fn spec(&self) -> ToolSpec {
        let mode = self.config.effective_mode();
        let default_timeout_ms = self.config.default_timeout_ms;
        let max_timeout_ms = self.config.max_timeout_ms;
        let default_wait_ms = self.config.default_wait_ms;
        let language = if self.config.runtime.powershell() {
            "PowerShell"
        } else {
            "bash"
        };
        let mut description = format!(
            "Run a command in this run's runtime ({}) with {} syntax. {} ",
            self.config.runtime.kind,
            language,
            self.config.runtime.lifecycle_sentence()
        );
        match mode {
            ShellMode::Wait => description.push_str(&format!(
                "Waits for the command to finish. Default timeout {}s{}; on timeout the command is stopped and the output so far is returned. ",
                default_timeout_ms / 1000,
                if max_timeout_ms == 0 {
                    ", no upper limit".to_string()
                } else {
                    format!(", max {}s", max_timeout_ms / 1000)
                }
            )),
            ShellMode::Auto => description.push_str(&format!(
                "Waits up to wait_ms (default {}s, max {} min) for the command to finish; a command still running then continues as a task and this call returns its task_id with the output so far — follow up with `wait_task` / `get_task_state`. Raise wait_ms for commands you expect to take long (builds, tests). ",
                default_wait_ms / 1000,
                MAX_AUTO_WAIT_MS / 60_000
            )),
        }
        if self.config.runtime.powershell() {
            description.push_str("Use Windows paths, $env:NAME for environment variables, Get-Content for reading files, and Start-Sleep for delays. Start a long-lived service with Start-Process -WindowStyle Hidden and return without -Wait. Long output keeps its beginning and end.");
        } else {
            description.push_str("Start a long-lived service with nohup / setsid and return at once; on Unix, processes started with `&` / nohup in the foreground command's process group also end when that command is interrupted or times out. Long output keeps its beginning and end.");
        }
        let mut properties = json!({
            "command": {
                "type": "string",
                "description": format!("{language} command to execute")
            },
            "cwd": {
                "type": "string",
                "description": if self.config.restrict_cwd {
                    "Working directory, relative to or within the configured workspace. Defaults to the workspace."
                } else {
                    "Working directory, absolute or relative to the default working directory. Defaults to that directory; outside paths are allowed subject to OS permissions."
                }
            }
        });
        let usage = match mode {
            ShellMode::Wait => {
                let mut t = json!({
                    "type": "integer",
                    "minimum": 1,
                    "description": format!("Timeout in milliseconds (default {default_timeout_ms}{}). Set it explicitly for long builds, tests or media commands.",
                        if max_timeout_ms == 0 { String::new() } else { format!(", max {max_timeout_ms}") })
                });
                if max_timeout_ms != 0 {
                    t["maximum"] = json!(max_timeout_ms);
                }
                properties["timeout_ms"] = t;
                format!(
                    "{name} command='<shell>' [timeout_ms={default_timeout_ms}]",
                    name = self.tool_name()
                )
            }
            ShellMode::Auto => {
                properties["wait_ms"] = json!({
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_AUTO_WAIT_MS,
                    "description": format!("How long this call waits in milliseconds before the command continues as a task (default {default_wait_ms}, max {MAX_AUTO_WAIT_MS}).")
                });
                format!(
                    "{name} command='<shell>' [wait_ms={default_wait_ms}]",
                    name = self.tool_name()
                )
            }
        };
        ToolSpec {
            name: self.tool_name().to_string(),
            description,
            args_schema: json!({
                "type": "object",
                "properties": properties,
                "required": ["command"]
            }),
            output_schema: json!({
                "type": "object",
                "properties": {
                    "exit_code": {"type": "integer"},
                    "output": {"type": "string"},
                    "task_id": {"type": "string"},
                }
            }),
            usage: Some(usage),
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
        let map = match args {
            Json::Object(map) => map,
            Json::Null => JsonMap::new(),
            other => {
                return Err(AgentToolError::InvalidArgs(format!(
                    "shell args must be a json object, got {}",
                    other
                )));
            }
        };

        let command = map
            .get("command")
            .and_then(Json::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                AgentToolError::InvalidArgs("`command` is required and must be non-empty".into())
            })?
            .to_string();

        let raw_cwd = map.get("cwd").and_then(Json::as_str);
        let cwd = self.resolve_cwd(raw_cwd).await?;
        let user_env = self.parse_env(map.get("env"))?;
        let target = self.config.target.resolve();
        if let BashTarget::Unsupported(value) = &target {
            return Err(AgentToolError::InvalidArgs(format!(
                "unsupported shell target `{value}` (only local is supported)"
            )));
        }
        let env = if self.config.runtime.kind != "native"
            && self.config.overlay.active_layers().is_empty()
            && !user_env.iter().any(|(key, _)| key == "PATH")
        {
            user_env
        } else {
            prepare_shell_env(
                &self.config.overlay,
                &user_env,
                self.config.runtime.powershell(),
            )
        };
        let mode = self.config.effective_mode();
        let timeout_ms = self.parse_timeout(map.get("timeout_ms"))?;
        let wait_ms = self.parse_wait(map.get("wait_ms"))?;
        let call_id = crate::runtime::CURRENT_TOOL_CALL
            .try_with(|c| c.clone())
            .ok();

        let request = BashRunRequest {
            command: command.clone(),
            cwd,
            timeout_ms,
            max_output_bytes: self.config.max_output_bytes,
            env,
            target,
            call_id: call_id.clone(),
        };

        let mut handle = self.runner.start(ctx, request).await?;
        let wait = match mode {
            ShellMode::Wait => wait_duration(timeout_ms),
            ShellMode::Auto => Duration::from_millis(wait_ms),
        };
        let waited = tokio::select! {
            r = handle.wait(wait) => r?,
            cause = current_cancel() => {
                return Err(self.cancel_command(handle, &command, cause).await);
            }
        };
        match waited {
            Some(output) => Ok(self.finished_result(&command, output, timeout_ms)),
            None => match mode {
                ShellMode::Wait => {
                    let output = handle.kill().await?;
                    Ok(self.finished_result(&command, output, timeout_ms))
                }
                ShellMode::Auto => Ok(self
                    .detach_into_task(handle, &command, call_id.as_deref(), wait_ms)
                    .await),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use crate::AgentToolPendingReason;
    use tempfile::tempdir;

    fn ctx() -> SessionRuntimeContext {
        SessionRuntimeContext {
            trace_id: "t".into(),
            agent_name: "a".into(),
            behavior: "b".into(),
            tool_call_index: 0,
            wakeup_id: "w".into(),
            session_id: "s".into(),
            read_token_limit: crate::DEFAULT_READ_TOKEN_LIMIT,
        }
    }

    fn ws() -> (tempfile::TempDir, PathBuf) {
        let dir = tempdir().expect("tempdir");
        let workspace = dir.path().join("workspace");
        fs::create_dir_all(&workspace).expect("mkdir workspace");
        (dir, workspace)
    }

    #[cfg(windows)]
    fn powershell_child(script: &str) -> String {
        use base64::Engine;
        let bytes: Vec<u8> = script
            .encode_utf16()
            .flat_map(|unit| unit.to_le_bytes())
            .collect();
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        format!("Start-Process -FilePath (Get-Process -Id $PID).Path -ArgumentList '-NoLogo -NoProfile -NonInteractive -EncodedCommand {encoded}' -WindowStyle Hidden -PassThru")
    }

    #[cfg(unix)]
    fn write_shim(path: &Path, stdout: &str, exit_code: i32) {
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' '{}'\nexit {}\n",
            stdout.replace('\'', "'\\''"),
            exit_code
        );
        fs::write(path, script).expect("write shim");
        let mut perms = fs::metadata(path).expect("meta").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).expect("chmod");
    }

    #[cfg(unix)]
    fn shell_with_overlay(workspace: PathBuf, bin_dir: PathBuf) -> ShellTool {
        let cfg = LlmBashConfig::local_workspace(workspace)
            .with_overlay(BinOverlayConfig::local(bin_dir));
        ShellTool::new(cfg)
    }

    #[tokio::test]
    async fn local_pwd_runs_inside_workspace() {
        let (_dir, workspace) = ws();
        let tool = ShellTool::local_workspace(workspace.clone());

        let result = tool
            .call(&ctx(), json!({ "command": if cfg!(windows) { "[Console]::Write((Get-Location).ProviderPath)" } else { "pwd" } }))
            .await
            .expect("call ok");

        assert_eq!(result.status, AgentToolStatus::Success);
        assert_eq!(result.details["exit_code"], 0);
        let stdout = result.details["stdout"].as_str().unwrap().trim();
        let canonical_ws = fs::canonicalize(&workspace).expect("canonicalize");
        let canonical_stdout = fs::canonicalize(Path::new(stdout)).expect("canonicalize stdout");
        assert_eq!(canonical_stdout, canonical_ws);
        assert_eq!(result.details["engine"], LOCAL_ENGINE);
        assert_eq!(result.details["runtime"], "native");
    }

    #[tokio::test]
    async fn bin_overlay_shadows_system_path() {
        let (dir, workspace) = ws();
        let bin_dir = dir.path().join("bin 中文 ' space");
        fs::create_dir_all(&bin_dir).expect("mkdir bin");

        let unique_path = bin_dir.join(if cfg!(windows) {
            "llm_bash_overlay_probe.ps1"
        } else {
            "llm_bash_overlay_probe"
        });
        fs::write(
            &unique_path,
            if cfg!(windows) {
                "Write-Output UNIQUE_HIT\n"
            } else {
                "#!/bin/sh\necho UNIQUE_HIT\n"
            },
        )
        .expect("write unique shim");
        #[cfg(unix)]
        {
            let mut perms = fs::metadata(&unique_path).expect("meta").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&unique_path, perms).expect("chmod");
        }

        let cat_shim = bin_dir.join(if cfg!(windows) { "whoami.ps1" } else { "cat" });
        fs::write(
            &cat_shim,
            if cfg!(windows) {
                "Write-Output SHIM_CAT_WINS\n"
            } else {
                "#!/bin/sh\necho SHIM_CAT_WINS\n"
            },
        )
        .expect("write cat shim");
        #[cfg(unix)]
        {
            let mut perms = fs::metadata(&cat_shim).expect("meta").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&cat_shim, perms).expect("chmod");
        }

        let cfg = LlmBashConfig::local_workspace(workspace.clone())
            .with_overlay(BinOverlayConfig::local(bin_dir.clone()));
        let tool = ShellTool::new(cfg);

        let unique_result = tool
            .call(&ctx(), json!({ "command": "llm_bash_overlay_probe" }))
            .await
            .expect("call ok");
        let unique_stdout = unique_result.details["stdout"].as_str().unwrap();
        assert!(
            unique_stdout.contains("UNIQUE_HIT"),
            "overlay shim not found, got: {unique_stdout}"
        );

        let shadow_result = tool
            .call(
                &ctx(),
                json!({ "command": if cfg!(windows) { "whoami" } else { "cat /dev/null" } }),
            )
            .await
            .expect("call ok");
        let shadow_stdout = shadow_result.details["stdout"].as_str().unwrap();
        assert!(
            shadow_stdout.contains("SHIM_CAT_WINS"),
            "overlay should win over system cat, got: {shadow_stdout}"
        );
        #[cfg(windows)]
        {
            let plain_tool = ShellTool::local_workspace(workspace);
            let result = plain_tool
                .call(&ctx(), json!({ "command": "llm_bash_overlay_probe", "env": { "PATH": bin_dir } }))
                .await
                .expect("call ok");
            assert!(result.details["stdout"].as_str().unwrap().contains("UNIQUE_HIT"), "{result:?}");
        }
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn forwards_inner_builtin_envelope() {
        let (dir, workspace) = ws();
        let bin_dir = dir.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("mkdir bin");
        write_shim(
            &bin_dir.join("fake_tool"),
            r#"{"agent_tool_protocol":"1","status":"success","cmd_name":"fake_tool","detail":{"x":1}}"#,
            0,
        );
        let tool = shell_with_overlay(workspace, bin_dir);

        let result = tool
            .call(&ctx(), json!({ "command": "fake_tool arg1" }))
            .await
            .expect("call ok");

        assert_eq!(result.status, AgentToolStatus::Success);
        assert_eq!(result.cmd_name.as_deref(), Some("fake_tool"));
        assert_eq!(result.details["x"], 1);
        assert_eq!(result.output, None);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn forwards_inner_pending_envelope() {
        let (dir, workspace) = ws();
        let bin_dir = dir.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("mkdir bin");
        write_shim(
            &bin_dir.join("fake_tool"),
            r#"{"agent_tool_protocol":"1","status":"pending","cmd_name":"fake_tool","task_id":"42","pending_reason":"long_running","check_after":3,"detail":{"queued":true}}"#,
            0,
        );
        let tool = shell_with_overlay(workspace, bin_dir);

        let result = tool
            .call(&ctx(), json!({ "command": "fake_tool start" }))
            .await
            .expect("call ok");

        assert_eq!(result.status, AgentToolStatus::Pending);
        assert_eq!(result.task_id.as_deref(), Some("42"));
        assert_eq!(
            result.pending_reason,
            Some(AgentToolPendingReason::LongRunning)
        );
        assert_eq!(result.check_after, Some(3));
        assert_eq!(result.details["queued"], true);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn plain_bash_protocol_stdout_is_forwarded() {
        let (_dir, workspace) = ws();
        let tool = ShellTool::local_workspace(workspace);

        let result = tool
            .call(
                &ctx(),
                json!({ "command": "echo '{\"agent_tool_protocol\":\"1\",\"status\":\"success\",\"cmd_name\":\"echo_protocol\",\"detail\":{\"x\":1}}'" }),
            )
            .await
            .expect("call ok");

        assert_eq!(result.status, AgentToolStatus::Success);
        assert_eq!(result.cmd_name.as_deref(), Some("echo_protocol"));
        assert_eq!(result.details["x"], 1);
        assert_eq!(result.output, None);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn plain_bash_non_protocol_json_is_not_forwarded() {
        let (_dir, workspace) = ws();
        let tool = ShellTool::local_workspace(workspace);

        let result = tool
            .call(
                &ctx(),
                json!({ "command": "echo '{\"status\":\"success\"}'" }),
            )
            .await
            .expect("call ok");

        assert_eq!(result.cmd_name.as_deref(), Some(TOOL_SHELL));
        assert_eq!(
            result.details["stdout"].as_str().unwrap().trim(),
            r#"{"status":"success"}"#
        );
        assert_eq!(
            result.output.as_deref().unwrap_or_default().trim(),
            r#"{"status":"success"}"#
        );
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn compound_command_is_not_forwarded() {
        let (dir, workspace) = ws();
        let bin_dir = dir.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("mkdir bin");
        write_shim(
            &bin_dir.join("fake_tool"),
            r#"{"agent_tool_protocol":"1","status":"success","cmd_name":"fake_tool","detail":{"x":1}}"#,
            0,
        );
        let tool = shell_with_overlay(workspace, bin_dir);

        let result = tool
            .call(&ctx(), json!({ "command": "fake_tool args | cat" }))
            .await
            .expect("call ok");

        assert_eq!(result.cmd_name.as_deref(), Some(TOOL_SHELL));
        assert_eq!(result.details["exit_code"], 0);
        assert!(result
            .output
            .as_deref()
            .unwrap_or_default()
            .contains("fake_tool"));
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn non_zero_exit_with_inner_envelope() {
        let (dir, workspace) = ws();
        let bin_dir = dir.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("mkdir bin");
        write_shim(
            &bin_dir.join("fake_tool"),
            r#"{"agent_tool_protocol":"1","status":"error","cmd_name":"fake_tool","detail":{"failed":true}}"#,
            1,
        );
        let tool = shell_with_overlay(workspace, bin_dir);

        let result = tool
            .call(&ctx(), json!({ "command": "fake_tool fail" }))
            .await
            .expect("call ok");

        assert_eq!(result.status, AgentToolStatus::Error);
        assert_eq!(result.return_code, Some(1));
        assert_eq!(result.cmd_name.as_deref(), Some("fake_tool"));
        assert_eq!(result.details["failed"], true);
    }

    #[tokio::test]
    async fn cwd_outside_workspace_rejected() {
        let (dir, workspace) = ws();
        let outside = dir.path().join("outside");
        fs::create_dir_all(&outside).expect("mkdir outside");
        let tool = ShellTool::local_workspace(workspace);

        let err = tool
            .call(
                &ctx(),
                json!({
                    "command": "pwd",
                    "cwd": outside.to_string_lossy().to_string()
                }),
            )
            .await
            .expect_err("should reject");
        assert!(matches!(err, AgentToolError::InvalidArgs(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn invalid_env_key_rejected() {
        let (_dir, workspace) = ws();
        let tool = ShellTool::local_workspace(workspace);

        let err = tool
            .call(
                &ctx(),
                json!({
                    "command": "true",
                    "env": { "1BAD": "value" }
                }),
            )
            .await
            .expect_err("should reject");
        assert!(matches!(err, AgentToolError::InvalidArgs(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn allow_env_false_rejects_env_arg() {
        let (_dir, workspace) = ws();
        let cfg = LlmBashConfig::local_workspace(workspace).with_allow_env(false);
        let tool = ShellTool::new(cfg);

        let err = tool
            .call(
                &ctx(),
                json!({
                    "command": "true",
                    "env": { "FOO": "bar" }
                }),
            )
            .await
            .expect_err("should reject");
        assert!(matches!(err, AgentToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn non_zero_exit_is_error_with_exit_code() {
        let (_dir, workspace) = ws();
        let tool = ShellTool::local_workspace(workspace);

        let commands = if cfg!(windows) {
            vec![
                "exit 7",
                "& powershell.exe -NoLogo -NoProfile -NonInteractive -Command 'exit 7'",
            ]
        } else {
            vec!["exit 7"]
        };
        for command in commands {
            let result = tool
                .call(&ctx(), json!({ "command": command }))
                .await
                .expect("call ok despite non-zero exit");
            assert_eq!(result.status, AgentToolStatus::Error);
            assert_eq!(result.details["exit_code"], 7);
            assert_eq!(result.return_code, Some(7));
            assert!(
                result.title.contains("=> error"),
                "failed command title must not say success: {}",
                result.title
            );
        }
    }

    #[tokio::test]
    async fn timeout_returns_error_with_partial_output() {
        let (_dir, workspace) = ws();
        let cfg = LlmBashConfig::local_workspace(workspace)
            .with_default_timeout_ms(if cfg!(windows) { 2500 } else { 300 })
            .with_max_timeout_ms(if cfg!(windows) { 3000 } else { 500 });
        let tool = ShellTool::new(cfg);

        let started = Instant::now();
        let result = tool
            .call(&ctx(), json!({ "command": "echo started; sleep 5" }))
            .await
            .expect("timeout is reported as a tool result");
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(result.status, AgentToolStatus::Error);
        assert_eq!(result.details["timed_out"], true);
        assert_eq!(result.return_code, Some(TIMEOUT_EXIT_CODE));
        assert!(result.summary.contains("timed out"), "{}", result.summary);
        assert!(result.output.as_deref().unwrap_or("").contains("started"));
    }

    #[tokio::test]
    async fn timeout_kills_child_processes() {
        let (_dir, workspace) = ws();
        let marker = workspace.join("survived");
        let cfg = LlmBashConfig::local_workspace(workspace.clone())
            .with_default_timeout_ms(if cfg!(windows) { 2500 } else { 300 })
            .with_max_timeout_ms(if cfg!(windows) { 3000 } else { 500 });
        let tool = ShellTool::new(cfg);

        #[cfg(not(windows))]
        let command = "(sleep 1; touch survived) & wait".to_string();
        #[cfg(windows)]
        let command = format!("$child = {}; $child.WaitForExit()", powershell_child("Start-Sleep -Seconds 4; [IO.File]::WriteAllText((Join-Path (Get-Location).ProviderPath 'survived'), 'yes')"));
        let result = tool
            .call(&ctx(), json!({ "command": command }))
            .await
            .expect("call ok");
        assert_eq!(result.details["timed_out"], true);
        tokio::time::sleep(Duration::from_millis(if cfg!(windows) {
            4500
        } else {
            1500
        }))
        .await;
        assert!(!marker.exists(), "child of timed-out command kept running");
    }

    #[tokio::test]
    async fn background_job_does_not_block_and_survives_the_call() {
        let (_dir, workspace) = ws();
        let marker = workspace.join("bg-done");
        let cfg = LlmBashConfig::local_workspace(workspace.clone()).with_default_timeout_ms(20_000);
        let tool = ShellTool::new(cfg);
        #[cfg(not(windows))]
        let command =
            "echo hi; nohup \"$BASH\" -c 'sleep 1; touch bg-done' >/dev/null 2>&1 &".to_string();
        #[cfg(windows)]
        let command = format!("$null = {}; Write-Output hi", powershell_child("Start-Sleep -Seconds 1; [IO.File]::WriteAllText((Join-Path (Get-Location).ProviderPath 'bg-done'), 'yes')"));

        let started = Instant::now();
        let result = tool
            .call(&ctx(), json!({ "command": command }))
            .await
            .expect("call ok");
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(result.status, AgentToolStatus::Success);
        assert_eq!(result.details["timed_out"], false);
        assert!(result.output.as_deref().unwrap_or("").contains("hi"));
        tokio::time::sleep(Duration::from_millis(if cfg!(windows) {
            3000
        } else {
            1800
        }))
        .await;
        assert!(
            marker.exists(),
            "a process the command left behind is not killed"
        );
    }

    #[test]
    fn xml_string_timeout_is_honored() {
        let (_dir, workspace) = ws();
        let cfg = LlmBashConfig::local_workspace(workspace)
            .with_default_timeout_ms(5_000)
            .with_max_timeout_ms(200);
        let tool = ShellTool::new(cfg);

        assert_eq!(
            tool.parse_timeout(Some(&Json::String("120".to_string())))
                .expect("XML string timeout must be accepted"),
            120
        );
    }

    #[test]
    fn unlimited_wait_mode_accepts_zero() {
        let (_dir, workspace) = ws();
        let cfg = LlmBashConfig::local_workspace(workspace).with_max_timeout_ms(0);
        let tool = ShellTool::new(cfg);
        assert_eq!(tool.parse_timeout(Some(&json!(0))).unwrap(), 0);
        assert_eq!(
            tool.parse_timeout(Some(&json!(10_000_000))).unwrap(),
            10_000_000
        );
        assert!(tool.spec().args_schema["properties"]["timeout_ms"]["maximum"].is_null());
    }

    #[test]
    fn spec_follows_mode_and_runtime() {
        let (_dir, workspace) = ws();
        let spec = ShellTool::local_workspace(workspace.clone()).spec();
        assert_eq!(spec.name, TOOL_SHELL);
        assert_eq!(
            spec.args_schema["properties"]["timeout_ms"]["maximum"],
            DEFAULT_MAX_TIMEOUT_MS
        );
        assert!(spec.args_schema["properties"]["wait_ms"].is_null());
        assert!(spec.description.contains("child process of this executor"));
        if cfg!(windows) {
            assert!(spec.description.contains("PowerShell"));
            assert!(spec.description.contains("Start-Process"));
            assert!(!spec.description.contains("nohup"));
        }

        let cfg = LlmBashConfig::local_workspace(workspace)
            .with_default_timeout_ms(90_000)
            .with_max_timeout_ms(120_000)
            .with_runtime_note(ShellRuntimeNote {
                kind: "tmux".into(),
                target: "od_x".into(),
            });
        let spec = ShellTool::new(cfg).spec();
        assert!(spec.description.contains("Default timeout 90s, max 120s"));
        assert!(spec.description.contains("tmux session `od_x`"));
        assert!(spec.description.contains("bash"));
        assert!(spec.usage.unwrap_or_default().contains("timeout_ms=90000"));
    }

    #[tokio::test]
    async fn output_truncated_flag_is_set() {
        let (_dir, workspace) = ws();
        let cfg = LlmBashConfig::local_workspace(workspace).with_max_output_bytes(32);
        let tool = ShellTool::new(cfg);

        let result = tool
            .call(
                &ctx(),
                json!({ "command": if cfg!(windows) { "[Console]::Write(('abcdefghij' * 200))" } else { "for i in $(seq 1 200); do echo -n abcdefghij; done" } }),
            )
            .await
            .expect("call ok");
        assert_eq!(result.details["output_truncated"], true);
        let output = result.details["output"].as_str().unwrap();
        assert!(output.contains("bytes omitted"), "{output}");
        assert!(
            output.len() <= 32 + 64,
            "output should be truncated, got {} bytes",
            output.len()
        );
    }

    #[tokio::test]
    async fn truncation_keeps_head_and_tail() {
        let (_dir, workspace) = ws();
        let cfg = LlmBashConfig::local_workspace(workspace).with_max_output_bytes(64);
        let tool = ShellTool::new(cfg);

        let result = tool
            .call(
                &ctx(),
                json!({ "command": if cfg!(windows) { "Write-Output BEGIN; 1..2000; Write-Output FINAL_LINE" } else { "echo BEGIN; seq 1 2000; echo FINAL_LINE" } }),
            )
            .await
            .expect("call ok");
        let output = result.output.as_deref().unwrap_or("");
        assert!(output.starts_with("BEGIN"), "{output}");
        assert!(output.trim_end().ends_with("FINAL_LINE"), "{output}");
        assert_eq!(result.details["output_truncated"], true);
    }

    #[test]
    fn parse_target_spec() {
        assert_eq!(BashTargetSpec::parse(None), BashTarget::Local);
        assert_eq!(BashTargetSpec::parse(Some("")), BashTarget::Local);
        assert_eq!(BashTargetSpec::parse(Some("local")), BashTarget::Local);
        assert_eq!(BashTargetSpec::parse(Some("LocalHost")), BashTarget::Local);
        assert_eq!(BashTargetSpec::parse(Some(".")), BashTarget::Local);
        assert!(matches!(
            BashTargetSpec::parse(Some("node-1")),
            BashTarget::Unsupported(_)
        ));
    }

    #[test]
    fn overlay_env_prepends_bin_dir() {
        let overlay = BinOverlayConfig::local("/tmp/llm_bash_overlay");
        let env = prepare_shell_env(
            &overlay,
            &[("PATH".to_string(), "/usr/bin:/bin".to_string())],
            false,
        );
        let path = env
            .iter()
            .find(|(k, _)| k == "PATH")
            .map(|(_, v)| v.as_str())
            .unwrap_or_default();
        assert!(
            path.starts_with("/tmp/llm_bash_overlay:"),
            "got PATH={path}"
        );

        let env_disabled = prepare_shell_env(
            &BinOverlayConfig::disabled(),
            &[("PATH".into(), "/p".into())],
            false,
        );
        let path2 = env_disabled
            .iter()
            .find(|(k, _)| k == "PATH")
            .map(|(_, v)| v.clone())
            .unwrap();
        assert_eq!(
            path2,
            "/p:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
        );
    }

    #[test]
    fn overlay_env_stacks_multiple_layers_in_priority_order() {
        let overlay =
            BinOverlayConfig::layered(["/a/session", "/a/agent", "/a/runtime", "/a/system"]);
        let env = prepare_shell_env(
            &overlay,
            &[("PATH".to_string(), "/usr/bin:/bin".to_string())],
            false,
        );
        let path = env
            .iter()
            .find(|(k, _)| k == "PATH")
            .map(|(_, v)| v.clone())
            .unwrap();
        assert_eq!(
            path,
            "/a/session:/a/agent:/a/runtime:/a/system:/usr/bin:/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/sbin"
        );
    }

    #[test]
    fn overlay_env_adds_system_bins_when_path_missing() {
        let env = prepare_overlay_env(&BinOverlayConfig::disabled(), &[]);
        let path = env
            .iter()
            .find(|(k, _)| k == "PATH")
            .map(|(_, v)| v.as_str())
            .unwrap_or_default();
        if cfg!(windows) {
            let root = PathBuf::from(std::env::var_os("SystemRoot").unwrap());
            assert!(
                path.split(';')
                    .any(|entry| Path::new(entry) == root.join("System32")),
                "got PATH={path}"
            );
        } else {
            assert!(
                path.split(':').any(|entry| entry == "/usr/bin"),
                "got PATH={path}"
            );
        }
    }

    #[tokio::test]
    async fn exec_dir_under_the_run_keeps_command_output_and_exit() {
        let (dir, workspace) = ws();
        let run_dir = dir.path().join("runs 中文 ' space").join("r1");
        let slot = new_run_binding_slot();
        *slot.lock().unwrap() = Some(RunBinding {
            run_id: "r1".into(),
            run_dir: Some(run_dir.clone()),
        });
        let runner = Arc::new(LocalProcessBashRunner::new().with_run_binding(slot));
        let tool = ShellTool::with_runner(LlmBashConfig::local_workspace(workspace), runner);
        let command = if cfg!(windows) {
            "[Console]::WriteLine('out'); [Console]::Error.WriteLine('err'); exit 3"
        } else {
            "echo out; echo err >&2; exit 3"
        };
        let result = crate::runtime::CURRENT_TOOL_CALL
            .scope(
                "call-7".into(),
                tool.call(&ctx(), json!({ "command": command })),
            )
            .await
            .unwrap();
        assert_eq!(result.details["exit_code"], 3);
        let exec = run_dir.join("exec").join("call-7");
        assert_eq!(fs::read_to_string(exec.join("exit")).unwrap().trim(), "3");
        assert_eq!(
            fs::read_to_string(exec.join("stdout"))
                .unwrap()
                .replace("\r\n", "\n"),
            "out\n"
        );
        assert_eq!(
            fs::read_to_string(exec.join("stderr"))
                .unwrap()
                .replace("\r\n", "\n"),
            "err\n"
        );
        assert_eq!(
            fs::read_to_string(exec.join("command")).unwrap().trim(),
            command
        );
        assert_eq!(read_exit_file(&exec), Some(3));
    }
}
