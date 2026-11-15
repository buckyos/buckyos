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
    /// page-render (headless Chromium): one at a time.
    Browser,
}

impl ExecutionLane {
    pub const ALL: [ExecutionLane; 6] = [
        Self::Light,
        Self::Image,
        Self::Pdf,
        Self::Video,
        Self::Office,
        Self::Browser,
    ];

    pub fn default_slots(self) -> usize {
        match self {
            Self::Light => 8,
            Self::Image => 2,
            Self::Pdf => 2,
            Self::Video => 1,
            Self::Office => 1,
            Self::Browser => 1,
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
    /// Digest of the installed font set, when this host can list its fonts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fonts: Option<String>,
    /// Language data tesseract has (`tesseract --list-langs`), sorted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ocr_langs: Vec<String>,
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
            "fonts": self.fonts,
            "ocr_langs": self.ocr_langs,
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
            "ocr_langs": self.ocr_langs,
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

    /// Where an external tool reads `content_id` without a temp copy, valid until
    /// the deadline; the caller releases it when the execution ends.
    async fn open_tool_input(&self, request: &ToolInputRequest<'_>) -> Result<ToolInput, ThunkExecutionError>;

    /// Ends what [`Self::open_tool_input`] set up; a crash is covered by its expiry.
    async fn release_tool_input(&self, lease: &ToolInputLease);

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

    /// Makes exactly these objects readable through NDM to `user_id` via
    /// `app_id` (TODO D3), the path Agents and AICC read by.
    async fn grant_holdings(&self, user_id: &str, app_id: &str, obj_ids: &[String]) -> Result<(), String>;
}

pub struct ToolInputRequest<'a> {
    pub content_id: &'a str,
    /// The run's input FileObject (`ThunkExecution::source_obj_id`), when it has one.
    pub source_obj_id: Option<&'a str>,
    pub execution_id: &'a str,
    /// Milliseconds; what the execution's deadline requires.
    pub expires_at_ms: i64,
    pub max_bytes: u64,
    /// A temp path for IO that has no URL to offer.
    pub temp_path: &'a Path,
}

/// What must be undone when an execution ends. Holds a capability: never logged.
pub struct ToolInputLease {
    pub key: String,
    pub token_id: String,
}

pub struct ToolInput {
    /// The path or URL handed to the tool. A URL carries a read token: it stays in
    /// process arguments and never reaches logs, errors, Task data or manifests.
    pub location: String,
    pub size: u64,
    /// `-protocol_whitelist` of this input.
    pub protocols: &'static str,
    /// Text to scrub from tool output.
    pub secrets: Vec<String>,
    pub lease: Option<ToolInputLease>,
}

/// Demuxers the media functions read. HLS, DASH and concat are deliberately out:
/// a file in those formats names other URLs, which an input URL would let it fetch.
pub const MEDIA_FORMAT_WHITELIST: &str = "mov,matroska,avi,mpegts,flv,ogg,mp3,aac,wav,flac,asf,mpeg,mpegvideo,h264,hevc";
pub const NDN_PROTOCOL_WHITELIST: &str = "http,https,tcp,tls";
pub const FILE_PROTOCOL_WHITELIST: &str = "file";

/// `bkt_...` read tokens anywhere in a text, however it got there.
pub fn scrub_tokens(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("bkt_") {
        out.push_str(&rest[..at]);
        out.push_str("bkt_***");
        let after = &rest[at + 4..];
        let end = after.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_')).unwrap_or(after.len());
        rest = &after[end..];
    }
    out.push_str(rest);
    out
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
    /// Tool inputs opened by this execution.
    pub tool_inputs: ToolInputs,
}

#[derive(Default)]
pub struct ToolInputs {
    leases: std::sync::Mutex<Vec<ToolInputLease>>,
    secrets: std::sync::Mutex<Vec<String>>,
}

impl NativeContext {
    /// `content_id` for ffmpeg and ffprobe: opened once per execution, released when it ends.
    pub async fn open_tool_input(&self, content_id: &str, temp_name: &str) -> Result<ToolInput, ThunkExecutionError> {
        self.check_cancelled()?;
        let now = chrono::Utc::now().timestamp();
        let expires_at = self.execution.deadline_at.unwrap_or(now + self.limits.max_exec_secs as i64) + 60;
        let request = ToolInputRequest {
            content_id,
            source_obj_id: self.execution.source_obj_id.as_deref(),
            execution_id: &self.execution.execution_id,
            expires_at_ms: expires_at.saturating_mul(1000),
            max_bytes: self.limits.max_input_bytes,
            temp_path: &self.temp_dir.join(temp_name),
        };
        let mut input = self.io.open_tool_input(&request).await?;
        self.tool_inputs.secrets.lock().unwrap().extend(input.secrets.iter().cloned());
        if let Some(lease) = input.lease.take() {
            self.tool_inputs.leases.lock().unwrap().push(lease);
        }
        Ok(input)
    }

    /// Revokes and unbinds whatever this execution opened. Runs however it ended.
    pub async fn release_tool_inputs(&self) {
        let leases = std::mem::take(&mut *self.tool_inputs.leases.lock().unwrap());
        for lease in &leases {
            self.io.release_tool_input(lease).await;
        }
    }

    fn scrub(&self, text: &str) -> String {
        let mut out = text.to_string();
        for secret in self.tool_inputs.secrets.lock().unwrap().iter() {
            out = out.replace(secret.as_str(), "<input>");
        }
        scrub_tokens(&out)
    }

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
        self.run_tool_env(program, args, &[], stdout_limit).await
    }

    /// [`Self::run_tool`] with extra environment variables.
    pub async fn run_tool_env(
        &self,
        program: &Path,
        args: &[String],
        envs: &[(&str, &str)],
        stdout_limit: usize,
    ) -> Result<ToolOutput, ThunkExecutionError> {
        self.check_cancelled()?;
        let mut command = tokio::process::Command::new(program);
        command.args(args).envs(envs.iter().copied());
        self.run_command(command, program, stdout_limit, false).await
    }

    /// [`Self::run_tool`] for a tool that renders third-party content: only the
    /// whitelisted environment (no `WORKFLOW_SESSION_TOKEN`), a process group of
    /// its own that never outlives the call, `nobody` instead of root on Linux,
    /// and a network namespace of its own unless `isolation.network`.
    pub async fn run_tool_isolated(
        &self,
        program: &Path,
        args: &[String],
        isolation: &ToolIsolation<'_>,
        stdout_limit: usize,
    ) -> Result<ToolOutput, ThunkExecutionError> {
        self.check_cancelled()?;
        let home = isolation.dir.join("home");
        tokio::fs::create_dir_all(&home)
            .await
            .map_err(|err| internal_error(format!("create {}: {err}", home.display())))?;
        let tmp = ToolTmp::create(isolation.dir)
            .map_err(|err| internal_error(format!("create the temp directory of the tool: {err}")))?;
        #[cfg(target_os = "linux")]
        let no_network = if isolation.network { confine::NoNetwork::Kept } else { confine::no_network().await };
        #[cfg(target_os = "linux")]
        let mut command = match no_network {
            confine::NoNetwork::Through([unshare, rest @ ..]) => {
                let mut command = tokio::process::Command::new(unshare);
                command.args(rest).arg(program);
                command
            }
            _ => tokio::process::Command::new(program),
        };
        #[cfg(not(target_os = "linux"))]
        let mut command = tokio::process::Command::new(program);
        command
            .args(args)
            .env_clear()
            .envs(isolated_env(&home, &tmp.0))
            .current_dir(isolation.dir);
        #[cfg(unix)]
        command.process_group(0);
        #[cfg(target_os = "linux")]
        if confine::is_root() {
            let account = confine::nobody()
                .ok_or_else(|| internal_error("this host has no `nobody` account to run the tool as"))?;
            let dirs = [isolation.dir.to_path_buf(), tmp.0.clone()];
            tokio::task::spawn_blocking(move || dirs.iter().try_for_each(|dir| confine::hand_over(dir, account)))
                .await
                .map_err(|err| internal_error(format!("hand over the tool directory: {err}")))?
                .map_err(|err| internal_error(format!("hand over {}: {err}", isolation.dir.display())))?;
            confine::drop_root(&mut command, account, matches!(no_network, confine::NoNetwork::Unshare));
        }
        self.run_command(command, program, stdout_limit, true).await
    }

    /// Spawns `command` and waits for it. With `own_tree` everything it started
    /// is killed when the call ends.
    async fn run_command(
        &self,
        mut command: tokio::process::Command,
        program: &Path,
        stdout_limit: usize,
        own_tree: bool,
    ) -> Result<ToolOutput, ThunkExecutionError> {
        command
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
        let mut tree = child.id().filter(|_| own_tree).map(ProcessTree::new);
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
        if status.is_none() {
            if let Some(tree) = &tree {
                tree.kill();
            }
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
        if let Some(tree) = tree.as_mut() {
            tree.leader_reaped();
        }
        let status = match status {
            Some(status) => status,
            None => {
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
        let mut stdout = stdout_task.await.unwrap_or_default();
        let stderr = stderr_task.await.unwrap_or_default();
        if !self.tool_inputs.secrets.lock().unwrap().is_empty() {
            stdout = self.scrub(&String::from_utf8_lossy(&stdout)).into_bytes();
        }
        let stderr_tail = self.scrub(&String::from_utf8_lossy(&stderr))
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

/// Where [`NativeContext::run_tool_isolated`] confines a tool.
pub struct ToolIsolation<'a> {
    /// The tool's working directory; its home is made in it. On Linux as root
    /// the whole directory is handed to `nobody` first.
    pub dir: &'a Path,
    /// Whether the tool may reach the network.
    pub network: bool,
}

/// What an isolated tool keeps of the service's environment.
const ISOLATED_ENV: &[&str] = if cfg!(windows) {
    &["PATH", "PATHEXT", "ComSpec", "SystemRoot", "SystemDrive", "windir", "ProgramData", "ProgramFiles", "ProgramFiles(x86)"]
} else {
    &["PATH", "LANG", "LC_ALL", "LC_CTYPE", "TZ"]
};

fn isolated_env(home: &Path, tmp: &Path) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    let mut env: Vec<(std::ffi::OsString, std::ffi::OsString)> = ISOLATED_ENV
        .iter()
        .filter_map(|name| std::env::var_os(name).map(|value| (name.into(), value)))
        .collect();
    let mut set = |name: &str, value: &Path| env.push((name.into(), value.into()));
    if cfg!(windows) {
        set("USERPROFILE", home);
        set("APPDATA", &home.join("AppData").join("Roaming"));
        set("LOCALAPPDATA", &home.join("AppData").join("Local"));
        set("TEMP", tmp);
        set("TMP", tmp);
    } else {
        set("HOME", home);
        set("TMPDIR", tmp);
        if !env.iter().any(|(name, _)| name == "LANG" || name == "LC_ALL") {
            env.push(("LANG".into(), "C.UTF-8".into()));
        }
    }
    env
}

/// The temp space of an isolated tool, removed when the call ends. Chromium
/// binds sockets under `TMPDIR` and refuses to start when their paths pass
/// about 100 bytes, which the execution temp dir of the service nearly is by
/// itself: on Unix it is a short private directory in `/tmp`.
struct ToolTmp(PathBuf);

impl ToolTmp {
    #[cfg(unix)]
    fn create(_dir: &Path) -> std::io::Result<Self> {
        use std::os::unix::fs::DirBuilderExt;
        let path = PathBuf::from(format!("/tmp/wf-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]));
        std::fs::DirBuilder::new().mode(0o700).create(&path)?;
        Ok(Self(path))
    }

    #[cfg(not(unix))]
    fn create(dir: &Path) -> std::io::Result<Self> {
        let path = dir.join("tmp");
        std::fs::create_dir_all(&path)?;
        Ok(Self(path))
    }
}

impl Drop for ToolTmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What an isolated tool started. None of it outlives the call, however the
/// call ends: returned, timed out, cancelled, or its future dropped.
struct ProcessTree {
    leader: u32,
    leader_reaped: bool,
}

impl ProcessTree {
    fn new(leader: u32) -> Self {
        Self { leader, leader_reaped: false }
    }

    fn kill(&self) {
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.leader as libc::pid_t), libc::SIGKILL);
        }
        #[cfg(windows)]
        {
            let mut taskkill = std::process::Command::new("taskkill");
            taskkill
                .args(["/F", "/T", "/PID", &self.leader.to_string()])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            buckyos_api::hide_child_console(&mut taskkill);
            let _ = taskkill.status();
        }
    }

    /// On Unix the leader's id still names the group it left behind; on Windows
    /// a waited-for pid names nothing.
    fn leader_reaped(&mut self) {
        #[cfg(unix)]
        self.kill();
        self.leader_reaped = true;
    }
}

impl Drop for ProcessTree {
    fn drop(&mut self) {
        if !self.leader_reaped {
            self.kill();
        }
    }
}

#[cfg(target_os = "linux")]
mod confine {
    use std::path::Path;
    use std::process::Stdio;

    /// uid and gid.
    pub type Account = (libc::uid_t, libc::gid_t);

    pub fn is_root() -> bool {
        unsafe { libc::geteuid() == 0 }
    }

    pub fn nobody() -> Option<Account> {
        static NOBODY: std::sync::OnceLock<Option<Account>> = std::sync::OnceLock::new();
        *NOBODY.get_or_init(|| unsafe {
            let account = libc::getpwnam(c"nobody".as_ptr());
            (!account.is_null()).then(|| ((*account).pw_uid, (*account).pw_gid))
        })
    }

    /// Gives `dir` and everything in it to `account`; links are not followed.
    pub fn hand_over(dir: &Path, account: Account) -> std::io::Result<()> {
        std::os::unix::fs::lchown(dir, Some(account.0), Some(account.1))?;
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                hand_over(&entry.path(), account)?;
            } else {
                std::os::unix::fs::lchown(entry.path(), Some(account.0), Some(account.1))?;
            }
        }
        Ok(())
    }

    /// The child leaves root for `account` before it runs the tool, after
    /// entering a network namespace of its own when `unshare_network`.
    pub fn drop_root(command: &mut tokio::process::Command, account: Account, unshare_network: bool) {
        let (uid, gid) = account;
        unsafe {
            command.pre_exec(move || {
                if unshare_network && libc::unshare(libc::CLONE_NEWNET) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::setgroups(1, &gid) != 0 || libc::setgid(gid) != 0 || libc::setuid(uid) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::getuid() != uid || libc::geteuid() != uid || libc::getgid() != gid || libc::getegid() != gid {
                    return Err(std::io::Error::from_raw_os_error(libc::EPERM));
                }
                Ok(())
            });
        }
    }

    /// How a tool is kept off the network on this host.
    #[derive(Clone, Copy)]
    pub enum NoNetwork {
        /// The service is root: `unshare(2)` in the child, before it drops root.
        Unshare,
        /// Run through this command.
        Through(&'static [&'static str]),
        /// It keeps the network: asked to, or this host cannot take it away.
        Kept,
    }

    /// Probed once. Not root, the ways are those of the nfs_server preview.
    pub async fn no_network() -> NoNetwork {
        static WAY: tokio::sync::OnceCell<NoNetwork> = tokio::sync::OnceCell::const_new();
        *WAY.get_or_init(|| async {
            let runs = |mut probe: tokio::process::Command| async move {
                probe.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
                matches!(probe.status().await, Ok(status) if status.success())
            };
            if is_root() {
                let mut probe = tokio::process::Command::new("true");
                unsafe {
                    probe.pre_exec(|| match libc::unshare(libc::CLONE_NEWNET) {
                        0 => Ok(()),
                        _ => Err(std::io::Error::last_os_error()),
                    });
                }
                if runs(probe).await {
                    log::info!("workflow: isolated tools run in a network namespace of their own");
                    return NoNetwork::Unshare;
                }
            } else {
                const WAYS: [&[&str]; 2] = [&["unshare", "--net", "--"], &["unshare", "--user", "--map-root-user", "--net", "--"]];
                for way in WAYS {
                    let mut probe = tokio::process::Command::new(way[0]);
                    probe.args(&way[1..]).arg("true");
                    if runs(probe).await {
                        log::info!("workflow: isolated tools run without network ({})", way.join(" "));
                        return NoNetwork::Through(way);
                    }
                }
            }
            log::warn!("workflow: no network namespace for isolated tools (unshare is missing or not permitted); they only have their own request blocking");
            NoNetwork::Kept
        })
        .await
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::io::MemoryNativeIo;
    use crate::object_store::InMemoryObjectStore;
    use std::os::unix::fs::PermissionsExt;

    /// The execution temp dir of a context; as in the service, other accounts
    /// can be let into it.
    fn temp_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        dir
    }

    fn context(temp_dir: &Path) -> NativeContext {
        NativeContext {
            execution: ThunkExecution {
                execution_id: "run-1/step/1".into(),
                thunk_obj_id: ndn_lib::ObjId::new("thunk:00").unwrap(),
                run_id: "run-1".into(),
                node_id: "step".into(),
                attempt: 1,
                execution_epoch: 1,
                source_obj_id: None,
                requested_by: None,
                deadline_at: None,
                registration: None,
                fingerprint: BTreeMap::new(),
                cache_key: None,
            },
            temp_dir: temp_dir.to_path_buf(),
            cancel: CancelToken::new(),
            deadline: None,
            env: Arc::new(ToolEnvironment::default()),
            io: Arc::new(MemoryNativeIo::new(Arc::new(InMemoryObjectStore::new()))),
            limits: ExecutionLimits::default(),
            artifact_prefix: "media/test".into(),
            tool_inputs: Default::default(),
        }
    }

    fn sh(script: &str) -> Vec<String> {
        vec!["-c".to_string(), script.to_string()]
    }

    /// Runs a shell script isolated in `<temp>/work`.
    async fn isolated(ctx: &NativeContext, script: &str, network: bool) -> Result<ToolOutput, ThunkExecutionError> {
        let dir = ctx.temp_dir.join("work");
        std::fs::create_dir_all(&dir).unwrap();
        ctx.run_tool_isolated(Path::new("/bin/sh"), &sh(script), &ToolIsolation { dir: &dir, network }, 64 * 1024)
            .await
    }

    #[tokio::test]
    async fn an_isolated_tool_keeps_only_the_whitelisted_environment() {
        std::env::set_var("WORKFLOW_SESSION_TOKEN", "secret-of-the-service");
        let tmp = temp_dir();
        // As deep as the execution temp dir of the service.
        let deep = tmp.path().join("opt/buckyos/data/cache/workflow/exec/t-0169ddaa93524c83a1c61bf302a0c1e0_page_to_pdf_1");
        std::fs::create_dir_all(&deep).unwrap();
        assert!(deep.as_os_str().len() > 100);
        let ctx = context(&deep);
        let plain = ctx.run_tool(Path::new("/bin/sh"), &sh("env"), 64 * 1024).await.unwrap();
        assert!(plain.stdout_string().contains("WORKFLOW_SESSION_TOKEN=secret-of-the-service"), "an ordinary tool inherits it");

        let out = isolated(&ctx, "env; ls -ld \"$TMPDIR\" | cut -c1-10; touch \"$TMPDIR/t\" && echo wrote; pwd -P", true).await.unwrap();
        assert!(out.success, "{}", out.stderr_tail);
        let text = out.stdout_string();
        assert!(!text.contains("WORKFLOW_SESSION_TOKEN") && !text.contains("secret-of-the-service"), "{text}");
        let names: Vec<&str> = text.lines().filter_map(|line| line.split_once('=')).map(|(name, _)| name).collect();
        for name in &names {
            assert!(
                ISOLATED_ENV.contains(name) || ["HOME", "TMPDIR", "PWD", "OLDPWD", "SHLVL", "_"].contains(name),
                "`{name}` reached the tool: {text}"
            );
        }
        assert!(names.contains(&"PATH"));
        let work = deep.join("work");
        assert!(text.contains(&format!("HOME={}\n", work.join("home").display())), "{text}");
        assert_eq!(text.lines().last().map(PathBuf::from), Some(work.canonicalize().unwrap()), "it runs in its directory");

        // Its temp space is short whatever the depth of its directory, private, and gone with the call.
        let tmpdir = text.lines().find_map(|line| line.strip_prefix("TMPDIR=")).unwrap();
        assert!(tmpdir.starts_with("/tmp/wf-") && tmpdir.len() == 20, "{tmpdir}");
        assert!(text.contains("\ndrwx------\nwrote\n"), "{text}");
        assert!(!Path::new(tmpdir).exists());
    }

    /// Whether `pid` still runs; a zombie waiting for its parent does not.
    fn runs(pid: i32) -> bool {
        #[cfg(target_os = "linux")]
        return std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .is_ok_and(|stat| stat.rsplit(')').next().is_some_and(|rest| !rest.trim_start().starts_with('Z')));
        #[cfg(not(target_os = "linux"))]
        return unsafe { libc::kill(pid, 0) == 0 };
    }

    /// A leader with two children of its own, all waiting; each writes its pid.
    const FAMILY: &str = "echo \"$TMPDIR\" > tmpdir; echo $$ > leader; sleep 300 & echo $! > first; sleep 300 & echo $! > second; wait";

    /// Whether the temp space the last tool in `work` had is still there.
    fn temp_space_left(work: &Path) -> bool {
        let tmpdir = std::fs::read_to_string(work.join("tmpdir")).unwrap();
        assert!(tmpdir.starts_with("/tmp/wf-"), "{tmpdir}");
        Path::new(tmpdir.trim()).exists()
    }

    async fn family(work: &Path) -> Vec<i32> {
        let read = |name: &str| std::fs::read_to_string(work.join(name)).ok().and_then(|pid| pid.trim().parse::<i32>().ok());
        for _ in 0..250 {
            if let (Some(leader), Some(first), Some(second)) = (read("leader"), read("first"), read("second")) {
                for name in ["leader", "first", "second"] {
                    std::fs::remove_file(work.join(name)).unwrap();
                }
                return vec![leader, first, second];
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("the tool did not start");
    }

    async fn all_gone(pids: &[i32]) -> bool {
        for _ in 0..250 {
            if pids.iter().all(|pid| !runs(*pid)) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        false
    }

    #[tokio::test]
    async fn nothing_an_isolated_tool_started_outlives_its_deadline() {
        let tmp = temp_dir();
        let work = tmp.path().join("work");
        let mut ctx = context(tmp.path());
        ctx.deadline = Some(Instant::now() + Duration::from_secs(2));
        let (result, pids) = tokio::join!(isolated(&ctx, FAMILY, true), async {
            let pids = family(&work).await;
            assert!(pids.iter().all(|pid| runs(*pid)), "{pids:?}");
            pids
        });
        assert_eq!(result.unwrap_err().code, error_codes::TIMEOUT);
        assert!(all_gone(&pids).await, "{pids:?}");
        assert!(!temp_space_left(&work));
    }

    #[tokio::test]
    async fn cancelling_or_dropping_the_call_kills_what_the_tool_started() {
        let tmp = temp_dir();
        let work = tmp.path().join("work");
        let ctx = context(tmp.path());
        let (result, pids) = tokio::join!(isolated(&ctx, FAMILY, true), async {
            let pids = family(&work).await;
            ctx.cancel.cancel();
            pids
        });
        assert_eq!(result.unwrap_err().code, error_codes::CANCELLED);
        assert!(all_gone(&pids).await, "{pids:?}");

        // The runner drops the call when its own timer wins.
        let ctx = context(tmp.path());
        let pids = tokio::select! {
            result = isolated(&ctx, FAMILY, true) => panic!("the tool ended: {result:?}"),
            pids = family(&work) => pids,
        };
        assert!(all_gone(&pids).await, "{pids:?}");
        assert!(!temp_space_left(&work));

        // A tool that ends on its own takes what it left in the background with it.
        let ctx = context(tmp.path());
        let out = tokio::time::timeout(Duration::from_secs(20), isolated(&ctx, "sleep 300 & echo $! > straggler; echo done", true))
            .await
            .expect("the call does not wait for the straggler")
            .unwrap();
        assert_eq!(out.stdout_string().trim(), "done");
        let straggler: i32 = std::fs::read_to_string(work.join("straggler")).unwrap().trim().parse().unwrap();
        assert!(all_gone(&[straggler]).await);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn without_network_an_isolated_tool_sees_only_loopback_where_the_host_can_do_that() {
        let tmp = temp_dir();
        let ctx = context(tmp.path());
        let out = isolated(&ctx, "cat /proc/net/dev", false).await.unwrap();
        assert!(out.success, "{}", out.stderr_tail);
        let interfaces: Vec<String> = out
            .stdout_string()
            .lines()
            .filter_map(|line| line.split_once(':'))
            .map(|(name, _)| name.trim().to_string())
            .collect();
        match confine::no_network().await {
            confine::NoNetwork::Kept => eprintln!("skipped: this host makes no network namespace for the tool ({interfaces:?})"),
            _ => assert_eq!(interfaces, vec!["lo"]),
        }
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn as_root_an_isolated_tool_runs_as_nobody_in_a_directory_handed_to_it() {
        use std::os::unix::fs::MetadataExt;
        if !confine::is_root() {
            eprintln!("skipped: the service account is not root");
            return;
        }
        std::env::set_var("WORKFLOW_SESSION_TOKEN", "secret-of-the-service");
        let tmp = temp_dir();
        let ctx = context(tmp.path());
        let work = tmp.path().join("work");
        std::fs::create_dir_all(work.join("site")).unwrap();
        std::fs::write(work.join("site/index.html"), "<p>page</p>").unwrap();
        let secret = tmp.path().join("secret");
        std::fs::write(&secret, "of root").unwrap();
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).unwrap();
        let (uid, gid) = confine::nobody().unwrap();
        assert_ne!(uid, 0);

        let script = format!(
            "id -u; id -g; id -G; cat site/index.html; echo; touch \"$HOME/h\" \"$TMPDIR/t\" out.txt && echo wrote; cat {} 2>/dev/null || echo unreadable; env | grep -c WORKFLOW_SESSION_TOKEN",
            secret.display()
        );
        for network in [true, false] {
            let out = isolated(&ctx, &script, network).await.unwrap();
            let text = out.stdout_string();
            let lines: Vec<&str> = text.lines().collect();
            assert_eq!(
                lines,
                vec![uid.to_string().as_str(), gid.to_string().as_str(), gid.to_string().as_str(), "<p>page</p>", "wrote", "unreadable", "0"],
                "network={network} stderr={}",
                out.stderr_tail
            );
            assert_eq!(std::fs::metadata(work.join("out.txt")).unwrap().uid(), uid);
        }
    }
}
