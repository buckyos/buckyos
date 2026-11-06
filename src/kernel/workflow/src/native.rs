//! Native function contract: what a `FunctionType::Native` implementation sees
//! when the local runner executes a thunk (TODO §2.3).
//!
//! A native function receives canonical params and an execution context; it
//! reads inputs by content id, writes artifacts through the context, and
//! returns a result value plus the facts the orchestrator needs for caching.
//! It never sees run ids, credentials or temp paths in its params.

use crate::error::{WorkflowError, WorkflowResult};
use async_trait::async_trait;
use buckyos_api::{error_codes, ThunkExecution, ThunkExecutionError};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;
use tokio::sync::Notify;

/// Concurrency lane of a native function; each lane has its own slot count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionLane {
    /// Pure bookkeeping (capabilities, build_catalog, test helpers).
    Light,
    /// CPU image work on a blocking thread.
    Image,
    /// poppler commands.
    Pdf,
    /// ffmpeg / ffprobe.
    Video,
    /// soffice: one at a time.
    Office,
}

impl ExecutionLane {
    pub const ALL: [ExecutionLane; 5] = [
        Self::Light,
        Self::Image,
        Self::Pdf,
        Self::Video,
        Self::Office,
    ];

    pub fn default_slots(self) -> usize {
        match self {
            Self::Light => 8,
            Self::Image => 2,
            Self::Pdf => 2,
            Self::Video => 1,
            Self::Office => 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolInfo {
    pub path: PathBuf,
    pub version: String,
}

/// Observed tool environment of this runner process. `revision` is the hash of
/// every tool identity; a function's fingerprint picks only the tools it uses.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolEnvironment {
    pub tools: BTreeMap<String, ToolInfo>,
    pub missing: Vec<String>,
    pub searched: Vec<PathBuf>,
    pub revision: String,
    pub probed_at: i64,
}

impl ToolEnvironment {
    pub fn with_tools(tools: BTreeMap<String, ToolInfo>) -> Self {
        let mut env = Self {
            tools,
            ..Default::default()
        };
        env.revision = env.compute_revision();
        env.probed_at = chrono::Utc::now().timestamp();
        env
    }

    pub fn compute_revision(&self) -> String {
        let value = json!({
            "tools": self.tools.iter().map(|(name, info)| {
                (name.clone(), json!({"path": info.path, "version": info.version}))
            }).collect::<BTreeMap<_, _>>(),
        });
        let (id, _) = ndn_lib::build_named_object_by_json("toolenv", &value);
        hex::encode(&id.obj_hash)
    }

    pub fn tool(&self, name: &str) -> Option<&ToolInfo> {
        self.tools.get(name)
    }

    /// Fingerprint of the named tools: `cmd/<name>` -> version, or the
    /// `missing` marker when absent (so a cache entry never matches a node
    /// that lacks the tool).
    pub fn fingerprint_for(&self, tools: &[&str]) -> BTreeMap<String, String> {
        tools
            .iter()
            .map(|name| {
                let value = self
                    .tools
                    .get(*name)
                    .map(|t| t.version.clone())
                    .unwrap_or_else(|| "missing".to_string());
                (format!("cmd/{name}"), value)
            })
            .collect()
    }

    pub fn missing_from(&self, tools: &[&str]) -> Vec<String> {
        tools
            .iter()
            .filter(|name| !self.tools.contains_key(**name))
            .map(|name| name.to_string())
            .collect()
    }

    pub fn describe(&self) -> Value {
        json!({
            "revision": self.revision,
            "probed_at": self.probed_at,
            "tools": self.tools,
            "missing": self.missing,
            "searched": self.searched,
        })
    }
}

/// Source of the current environment snapshot (refreshable in production).
pub trait EnvironmentProvider: Send + Sync {
    fn current(&self) -> Arc<ToolEnvironment>;
}

pub struct StaticEnvironment(pub Arc<ToolEnvironment>);

impl EnvironmentProvider for StaticEnvironment {
    fn current(&self) -> Arc<ToolEnvironment> {
        self.0.clone()
    }
}

#[derive(Default)]
pub struct CancelToken {
    flag: AtomicBool,
    notify: Notify,
}

impl CancelToken {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    pub async fn cancelled(&self) {
        loop {
            if self.is_cancelled() {
                return;
            }
            self.notify.notified().await;
        }
    }
}

/// Service-level execution limits (TODO §3.2); functions tighten, never loosen.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionLimits {
    pub max_input_bytes: u64,
    pub max_pages: u32,
    pub max_frames: u32,
    pub max_page_pixels: u64,
    pub max_total_pixels: u64,
    pub max_output_bytes: u64,
    pub max_temp_bytes: u64,
    pub max_queue: usize,
    pub max_queue_wait_secs: u64,
    pub max_exec_secs: u64,
}

impl Default for ExecutionLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 512 * 1024 * 1024,
            max_pages: 200,
            max_frames: 64,
            max_page_pixels: 4096 * 4096,
            max_total_pixels: 200 * 2048 * 2048,
            max_output_bytes: 2 * 1024 * 1024 * 1024,
            max_temp_bytes: 4 * 1024 * 1024 * 1024,
            max_queue: 64,
            max_queue_wait_secs: 300,
            max_exec_secs: 900,
        }
    }
}

/// How the runner reads inputs and stores outputs on behalf of a function.
#[async_trait]
pub trait NativeIo: Send + Sync {
    /// Size of a chunk / chunklist content, when known.
    async fn content_size(&self, content_id: &str) -> Result<Option<u64>, ThunkExecutionError>;

    /// Whole content in memory (bounded by `max_bytes`).
    async fn read_content(
        &self,
        content_id: &str,
        max_bytes: u64,
    ) -> Result<Vec<u8>, ThunkExecutionError>;

    /// Streams content into `path`; returns the byte count.
    async fn materialize(
        &self,
        content_id: &str,
        path: &Path,
        max_bytes: u64,
    ) -> Result<u64, ThunkExecutionError>;

    /// Stores bytes as a deterministic artifact FileObject (fixed time fields,
    /// stable name) bound under `key`; returns the FileObject id.
    async fn put_artifact(
        &self,
        key: &str,
        name: &str,
        content_type: &str,
        bytes: Vec<u8>,
    ) -> Result<ArtifactRef, ThunkExecutionError>;

    /// Same as [`Self::put_artifact`] from a file on disk.
    async fn put_artifact_file(
        &self,
        key: &str,
        name: &str,
        content_type: &str,
        path: &Path,
    ) -> Result<ArtifactRef, ThunkExecutionError>;

    /// Stores an immutable JSON named object; returns its id.
    async fn put_object(&self, obj_type: &str, value: &Value) -> Result<String, ThunkExecutionError>;

    async fn get_object(&self, obj_id: &str) -> Result<Option<Value>, ThunkExecutionError>;

    /// Whether an artifact / object referenced by a cached result is still
    /// readable now (object present, bucket binding local).
    async fn artifact_available(&self, obj_id: &str) -> bool;

    /// Grants the caller read access to exactly these artifacts (TODO D3 /
    /// D15): stable object ids plus a separate, expiring access fact.
    async fn deliver(
        &self,
        principal: &str,
        obj_ids: &[String],
        expires_at: i64,
    ) -> Vec<ArtifactDelivery>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactDelivery {
    pub obj_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator: Option<String>,
    /// Bucket data-plane path with a read token, when the store issues one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    pub expires_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactRef {
    pub obj_id: String,
    pub content_id: String,
    pub key: String,
    pub size: u64,
}

pub struct NativeContext {
    pub execution: ThunkExecution,
    pub temp_dir: PathBuf,
    pub cancel: Arc<CancelToken>,
    pub deadline: Option<Instant>,
    pub env: Arc<ToolEnvironment>,
    pub io: Arc<dyn NativeIo>,
    pub limits: ExecutionLimits,
    /// `media/<cache_key>` when the step result is reusable, else
    /// `exec/<execution_id>`.
    pub artifact_prefix: String,
}

impl NativeContext {
    pub fn artifact_key(&self, name: &str) -> String {
        format!("{}/{}", self.artifact_prefix, name)
    }

    pub fn check_cancelled(&self) -> Result<(), ThunkExecutionError> {
        if self.cancel.is_cancelled() {
            return Err(ThunkExecutionError::new(error_codes::CANCELLED, "execution cancelled"));
        }
        if let Some(deadline) = self.deadline {
            if Instant::now() >= deadline {
                return Err(ThunkExecutionError::new(error_codes::TIMEOUT, "execution deadline passed"));
            }
        }
        Ok(())
    }

    pub fn remaining(&self) -> Option<Duration> {
        self.deadline
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
    }

    /// Fails with `tools_missing` listing every absent tool.
    pub fn require_tools(&self, tools: &[&str]) -> Result<(), ThunkExecutionError> {
        let missing = self.env.missing_from(tools);
        if missing.is_empty() {
            return Ok(());
        }
        Err(ThunkExecutionError::new(
            error_codes::TOOLS_MISSING,
            format!("required tools are missing on this node: {}", missing.join(", ")),
        )
        .with_details(json!({
            "tools_missing": missing,
            "searched": self.env.searched,
        })))
    }

    pub fn tool_path(&self, name: &str) -> Result<PathBuf, ThunkExecutionError> {
        self.require_tools(&[name])?;
        Ok(self.env.tools[name].path.clone())
    }

    /// Materializes `content_id` into the execution temp dir.
    pub async fn materialize_input(
        &self,
        content_id: &str,
        file_name: &str,
    ) -> Result<(PathBuf, u64), ThunkExecutionError> {
        self.check_cancelled()?;
        let path = self.temp_dir.join(file_name);
        let size = self
            .io
            .materialize(content_id, &path, self.limits.max_input_bytes)
            .await?;
        Ok((path, size))
    }

    /// Runs CPU-bound work on a blocking thread; the closure must poll
    /// `cancel` between units of work.
    pub async fn run_blocking<T: Send + 'static>(
        &self,
        f: impl FnOnce(Arc<CancelToken>) -> Result<T, ThunkExecutionError> + Send + 'static,
    ) -> Result<T, ThunkExecutionError> {
        self.check_cancelled()?;
        let cancel = self.cancel.clone();
        let handle = tokio::task::spawn_blocking(move || f(cancel));
        let timeout = self.remaining().unwrap_or(Duration::from_secs(u64::MAX / 4));
        match tokio::time::timeout(timeout, handle).await {
            Ok(Ok(result)) => result,
            Ok(Err(err)) => Err(ThunkExecutionError::new(
                error_codes::INTERNAL,
                format!("blocking task failed: {err}"),
            )),
            Err(_) => {
                self.cancel.cancel();
                Err(ThunkExecutionError::new(error_codes::TIMEOUT, "execution deadline passed"))
            }
        }
    }

    /// Runs an external command with stdin closed, bounded stdout / stderr
    /// capture, and process kill + reap on deadline or cancellation.
    pub async fn run_tool(
        &self,
        program: &Path,
        args: &[String],
        stdout_limit: usize,
    ) -> Result<ToolOutput, ThunkExecutionError> {
        self.check_cancelled()?;
        let mut command = tokio::process::Command::new(program);
        command
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        buckyos_api::hide_child_console_async(&mut command);
        let tool = program
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut child = command.spawn().map_err(|err| {
            ThunkExecutionError::new(error_codes::TOOL_FAILED, format!("start {tool} failed: {err}"))
                .with_details(json!({"tool": tool}))
        })?;
        let mut stdout = child.stdout.take().expect("piped stdout");
        let mut stderr = child.stderr.take().expect("piped stderr");
        let stdout_task = tokio::spawn(async move {
            let mut buf = Vec::new();
            let _ = (&mut stdout)
                .take(stdout_limit as u64)
                .read_to_end(&mut buf)
                .await;
            let _ = tokio::io::copy(&mut stdout, &mut tokio::io::sink()).await;
            buf
        });
        let stderr_task = tokio::spawn(async move {
            let mut buf = Vec::new();
            let _ = (&mut stderr).take(64 * 1024).read_to_end(&mut buf).await;
            let _ = tokio::io::copy(&mut stderr, &mut tokio::io::sink()).await;
            buf
        });
        let timeout = self.remaining().unwrap_or(Duration::from_secs(u64::MAX / 4));
        let status = tokio::select! {
            status = child.wait() => Some(status),
            _ = self.cancel.cancelled() => None,
            _ = tokio::time::sleep(timeout) => None,
        };
        let status = match status {
            Some(status) => status,
            None => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                let code = if self.cancel.is_cancelled() {
                    error_codes::CANCELLED
                } else {
                    error_codes::TIMEOUT
                };
                return Err(ThunkExecutionError::new(code, format!("{tool} stopped: {code}"))
                    .with_details(json!({"tool": tool})));
            }
        };
        let status = status.map_err(|err| {
            ThunkExecutionError::new(error_codes::TOOL_FAILED, format!("wait {tool} failed: {err}"))
        })?;
        let stdout = stdout_task.await.unwrap_or_default();
        let stderr = stderr_task.await.unwrap_or_default();
        let stderr_tail = String::from_utf8_lossy(&stderr)
            .chars()
            .rev()
            .take(2048)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<String>();
        Ok(ToolOutput {
            tool,
            success: status.success(),
            exit_code: status.code(),
            stdout,
            stderr_tail,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub tool: String,
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr_tail: String,
}

impl ToolOutput {
    pub fn ok(self) -> Result<Self, ThunkExecutionError> {
        if self.success {
            return Ok(self);
        }
        Err(ThunkExecutionError::new(
            error_codes::TOOL_FAILED,
            format!("{} exited with {:?}", self.tool, self.exit_code),
        )
        .with_details(json!({
            "tool": self.tool,
            "exit_code": self.exit_code,
            "stderr_tail": self.stderr_tail,
        })))
    }

    pub fn stdout_string(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
}

#[derive(Debug, Clone, Default)]
pub struct NativeOutcome {
    pub result: Value,
    /// `false` for partial / truncated results: deliverable, never cached.
    pub cacheable: bool,
    pub artifact_refs: Vec<String>,
    pub metrics: Value,
}

impl NativeOutcome {
    pub fn new(result: Value) -> Self {
        Self {
            result,
            cacheable: true,
            artifact_refs: Vec::new(),
            metrics: Value::Null,
        }
    }

    pub fn with_artifacts(mut self, refs: Vec<String>) -> Self {
        self.artifact_refs = refs;
        self
    }

    pub fn not_cacheable(mut self) -> Self {
        self.cacheable = false;
        self
    }
}

#[async_trait]
pub trait NativeFunction: Send + Sync {
    /// Tools this implementation shells out to; they form the execution
    /// fingerprint and the `tools_missing` diagnosis.
    fn tools(&self) -> &'static [&'static str] {
        &[]
    }

    fn lane(&self) -> ExecutionLane {
        ExecutionLane::Light
    }

    fn fingerprint(&self, env: &ToolEnvironment) -> BTreeMap<String, String> {
        env.fingerprint_for(self.tools())
    }

    async fn invoke(
        &self,
        ctx: &NativeContext,
        params: &Value,
    ) -> Result<NativeOutcome, ThunkExecutionError>;
}

/// Maps a thunk execution error into the workflow error space for RPC callers.
pub fn execution_error(err: &ThunkExecutionError) -> WorkflowError {
    WorkflowError::Execution(err.clone())
}

pub fn internal_error(message: impl Into<String>) -> ThunkExecutionError {
    ThunkExecutionError::new(error_codes::INTERNAL, message)
}

pub fn invalid_params(message: impl Into<String>) -> ThunkExecutionError {
    ThunkExecutionError::new(error_codes::INVALID_PARAMS, message)
}

/// Reads a required integer param.
pub fn param_u64(params: &Value, name: &str) -> Result<u64, ThunkExecutionError> {
    params
        .get(name)
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid_params(format!("param `{name}` must be a non-negative integer")))
}

pub fn param_str<'a>(params: &'a Value, name: &str) -> Result<&'a str, ThunkExecutionError> {
    params
        .get(name)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid_params(format!("param `{name}` must be a non-empty string")))
}

pub fn workflow_result<T>(result: Result<T, ThunkExecutionError>) -> WorkflowResult<T> {
    result.map_err(|err| execution_error(&err))
}
