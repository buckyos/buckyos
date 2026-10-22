//! Operation manager: admission, the serial control gate, execution and
//! terminal publication (doc §7.3–7.5).

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use buckyos_api::{
    executor_content_hash, executor_request_digest, ExecutorChannel, ExecutorChunk, ExecutorError,
    ExecutorErrorCode, ExecutorFileResult, ExecutorLimits, ExecutorOpKind, ExecutorOpResult,
    ExecutorOpSpec, ExecutorOpState, ExecutorOpStatus, ExecutorOutputSummary, ExecutorProfile,
    ExecutorRunAs, EXECUTOR_MAX_QUERY_BYTES, EXECUTOR_MAX_QUERY_WAIT_MS,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{broadcast, watch};

use crate::now_ms;
use crate::spawn::{run_file_job, spawn_shell, FileJobOutcome, WorkerLauncher};
use crate::store::{Dedupe, OpBudget, OpIdentity, OpRequestRecord, Store, STORE_SCHEMA_VERSION};
use crate::worker::WorkerJob;

const DRAIN_GRACE: Duration = Duration::from_millis(500);
const PUSH_CHUNK: usize = 16 * 1024;
#[cfg(target_os = "macos")]
const DEFAULT_LANG: &str = "en_US.UTF-8";
#[cfg(all(unix, not(target_os = "macos")))]
const DEFAULT_LANG: &str = "C.UTF-8";
/// Host variables a Windows shell needs that a profile normally leaves out;
/// the profile's own values win. Identity and control-side variables are
/// never copied.
#[cfg(windows)]
const WINDOWS_HOST_ENV: [&str; 25] = [
    "PATH",
    "PATHEXT",
    "SystemRoot",
    "SystemDrive",
    "windir",
    "ComSpec",
    "TEMP",
    "TMP",
    "USERPROFILE",
    "USERNAME",
    "USERDOMAIN",
    "HOMEDRIVE",
    "HOMEPATH",
    "APPDATA",
    "LOCALAPPDATA",
    "ProgramData",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramW6432",
    "CommonProgramFiles",
    "CommonProgramFiles(x86)",
    "PUBLIC",
    "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE",
    "OS",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelReason {
    Client,
    Revoked,
    AuthExpired,
    Storage,
}

#[derive(Debug, Clone)]
pub enum OpEvent {
    Started {
        key: String,
        op_id: String,
        pid: u32,
    },
    Output {
        key: String,
        op_id: String,
        channel: ExecutorChannel,
        offset: u64,
        data: Arc<Vec<u8>>,
    },
    Finished {
        key: String,
        op_id: String,
        result: ExecutorOpResult,
    },
}

/// Everything an admitted operation needs, fixed at admission.
#[derive(Debug, Clone)]
pub struct OpContext {
    pub profile: ExecutorProfile,
    pub limits: ExecutorLimits,
    pub auth_generation: u64,
}

pub struct ActiveOp {
    pub key: String,
    pub identity: OpIdentity,
    pub kind: ExecutorOpKind,
    pub profile: ExecutorProfile,
    cancel: watch::Sender<Option<CancelReason>>,
    progress: watch::Sender<u64>,
    spawned: Mutex<bool>,
}

impl ActiveOp {
    fn bump(&self) {
        self.progress.send_modify(|v| *v += 1);
    }
}

pub struct OpsManager {
    store: Arc<Store>,
    launcher: WorkerLauncher,
    shell: PathBuf,
    server_epoch: String,
    active: Mutex<HashMap<String, Arc<ActiveOp>>>,
    gate: tokio::sync::Mutex<()>,
    path_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    events: broadcast::Sender<OpEvent>,
}

fn err(code: ExecutorErrorCode, msg: impl Into<String>) -> ExecutorError {
    ExecutorError::new(code, msg)
}

fn internal(e: impl std::fmt::Display) -> ExecutorError {
    err(ExecutorErrorCode::Internal, e.to_string())
}

fn validate_spec(spec: &ExecutorOpSpec, limits: &ExecutorLimits) -> Result<(), ExecutorError> {
    let abs = |p: &str| -> Result<(), ExecutorError> {
        if !crate::is_host_absolute(p) {
            return Err(err(
                ExecutorErrorCode::InvalidArgument,
                format!("path `{p}` must be absolute"),
            ));
        }
        Ok(())
    };
    match spec {
        ExecutorOpSpec::Exec {
            cmd,
            cwd,
            env_overrides,
            execution_timeout_ms,
        } => {
            if cmd.trim().is_empty() || cmd.contains('\0') {
                return Err(err(ExecutorErrorCode::InvalidArgument, "cmd is empty or contains NUL"));
            }
            abs(cwd)?;
            for (k, v) in env_overrides {
                if k.is_empty()
                    || !k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    || buckyos_api::is_forbidden_env_name(k)
                    || v.contains('\0')
                {
                    return Err(err(
                        ExecutorErrorCode::InvalidArgument,
                        format!("env override `{k}` is not allowed"),
                    ));
                }
            }
            if execution_timeout_ms == &Some(0) {
                return Err(err(ExecutorErrorCode::InvalidArgument, "execution_timeout_ms is 0"));
            }
        }
        ExecutorOpSpec::Read { path, length, .. } => {
            abs(path)?;
            if length.unwrap_or(0) > limits.max_file_bytes {
                return Err(err(ExecutorErrorCode::InvalidArgument, "length exceeds max_file_bytes"));
            }
        }
        ExecutorOpSpec::Write {
            path,
            content_length,
            ..
        } => {
            abs(path)?;
            if *content_length > limits.max_file_bytes {
                return Err(err(
                    ExecutorErrorCode::InvalidArgument,
                    "content_length exceeds max_file_bytes",
                ));
            }
        }
        ExecutorOpSpec::Edit { path, edits, .. } => {
            abs(path)?;
            if edits.is_empty() {
                return Err(err(ExecutorErrorCode::InvalidArgument, "edits is empty"));
            }
        }
    }
    Ok(())
}

pub struct SubmitOutcome {
    pub status: ExecutorOpStatus,
    pub duplicate: bool,
}

impl OpsManager {
    pub fn new(store: Arc<Store>, launcher: WorkerLauncher, shell: PathBuf, server_epoch: String) -> Self {
        let (events, _) = broadcast::channel(1024);
        Self {
            store,
            launcher,
            shell,
            server_epoch,
            active: Mutex::new(HashMap::new()),
            gate: tokio::sync::Mutex::new(()),
            path_locks: Mutex::new(HashMap::new()),
            events,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<OpEvent> {
        self.events.subscribe()
    }

    /// Validate, dedupe, reserve a slot and space, persist `starting`, and
    /// only then start the work.
    pub async fn submit(
        self: &Arc<Self>,
        ctx: OpContext,
        identity: OpIdentity,
        spec: ExecutorOpSpec,
        upload: Option<(PathBuf, u64)>,
    ) -> Result<SubmitOutcome, ExecutorError> {
        validate_spec(&spec, &ctx.limits)?;
        let store_id = self.store.store_id().to_string();
        let key = identity.key_hash(&store_id);
        let call = identity.call_hash(&store_id);
        let digest = executor_request_digest(
            &identity.runtime_id,
            &identity.run_id,
            &identity.call_id,
            &identity.op_id,
            &spec,
        );
        if let Dedupe::Existing(k) = self.store.dedupe(&key, &call, &digest)? {
            if let Some((path, bytes)) = &upload {
                let _ = std::fs::remove_file(path);
                self.store.release(*bytes);
            }
            return Ok(SubmitOutcome {
                status: self.store.status(&k)?,
                duplicate: true,
            });
        }
        let budget = OpBudget {
            execution_timeout_ms: match &spec {
                ExecutorOpSpec::Exec {
                    execution_timeout_ms,
                    ..
                } => execution_timeout_ms
                    .unwrap_or(ctx.limits.max_execution_ms)
                    .min(ctx.limits.max_execution_ms),
                _ => 0,
            },
            max_output_bytes: ctx.limits.max_output_bytes_per_stream,
            max_file_bytes: ctx.limits.max_file_bytes,
        };
        let record = OpRequestRecord {
            schema_version: STORE_SCHEMA_VERSION,
            store_id,
            key_hash: key.clone(),
            call_hash: call,
            identity: identity.clone(),
            digest,
            reservation_bytes: Store::reservation_for(&spec, &budget),
            spec,
            accepted_at_ms: now_ms(),
            server_epoch: self.server_epoch.clone(),
            auth_generation: ctx.auth_generation,
            budget,
        };
        let (cancel, _) = watch::channel(None);
        let (progress, _) = watch::channel(0u64);
        let op = Arc::new(ActiveOp {
            key: key.clone(),
            identity,
            kind: record.spec.kind(),
            profile: ctx.profile.clone(),
            cancel,
            progress,
            spawned: Mutex::new(false),
        });
        {
            let mut active = self.active.lock().unwrap();
            if active.len() >= ctx.limits.max_concurrency as usize {
                return Err(err(ExecutorErrorCode::Busy, "no free execution slot"));
            }
            if active.contains_key(&key) {
                return Err(err(ExecutorErrorCode::Busy, "operation is being admitted"));
            }
            active.insert(key.clone(), op.clone());
        }
        let store = self.store.clone();
        let limits = ctx.limits.clone();
        let rec = record.clone();
        let admitted = tokio::task::spawn_blocking(move || {
            store.admit(&rec, &limits, upload.as_ref().map(|(p, n)| (p.as_path(), *n)))
        })
        .await
        .map_err(internal)
        .and_then(|r| r);
        match admitted {
            Ok(Dedupe::New) => {}
            Ok(Dedupe::Existing(k)) => {
                self.active.lock().unwrap().remove(&key);
                return Ok(SubmitOutcome {
                    status: self.store.status(&k)?,
                    duplicate: true,
                });
            }
            Err(e) => {
                self.active.lock().unwrap().remove(&key);
                return Err(e);
            }
        }
        let status = self.store.status(&key)?;
        let me = self.clone();
        tokio::spawn(async move {
            me.run(op, record).await;
        });
        Ok(SubmitOutcome {
            status,
            duplicate: false,
        })
    }

    /// Record the request, then let the running work observe it. The final
    /// state is whatever the confirmed exit says.
    pub async fn cancel(&self, key: &str, reason: CancelReason) -> Result<ExecutorOpStatus, ExecutorError> {
        let status = self.store.status(key)?;
        if status.state.is_terminal() {
            return Ok(status);
        }
        let _gate = self.gate.lock().await;
        let op = self.active.lock().unwrap().get(key).cloned();
        if let Some(op) = op {
            let store = self.store.clone();
            let k = key.to_string();
            let _ = tokio::task::spawn_blocking(move || {
                store.update_state(&k, |s| s.cancel_requested = true)
            })
            .await;
            op.cancel.send_if_modified(|v| {
                if v.is_none() {
                    *v = Some(reason);
                    true
                } else {
                    false
                }
            });
            op.bump();
        }
        self.store.status(key)
    }

    /// Cancel every active operation the predicate selects.
    pub async fn cancel_where(&self, reason: CancelReason, pred: impl Fn(&ActiveOp) -> bool) -> usize {
        let keys: Vec<String> = self
            .active
            .lock()
            .unwrap()
            .values()
            .filter(|op| pred(op))
            .map(|op| op.key.clone())
            .collect();
        for key in &keys {
            let _ = self.cancel(key, reason).await;
        }
        keys.len()
    }

    pub async fn query(
        &self,
        key: &str,
        channel: Option<ExecutorChannel>,
        offset: u64,
        max_bytes: Option<u64>,
        wait_ms: Option<u64>,
    ) -> Result<(ExecutorOpStatus, Option<ExecutorChunk>), ExecutorError> {
        let mut status = self.store.status(key)?;
        let wait = wait_ms.unwrap_or(0).min(EXECUTOR_MAX_QUERY_WAIT_MS);
        if wait > 0 && !status.state.is_terminal() {
            let progress = self
                .active
                .lock()
                .unwrap()
                .get(key)
                .map(|op| op.progress.subscribe());
            if let Some(mut rx) = progress {
                let deadline = tokio::time::Instant::now() + Duration::from_millis(wait);
                loop {
                    let has_more = match channel {
                        Some(ch) => self.store.read_chunk(key, ch, offset, 1).map(|(d, _)| !d.is_empty()).unwrap_or(false),
                        None => false,
                    };
                    if has_more {
                        break;
                    }
                    tokio::select! {
                        changed = rx.changed() => if changed.is_err() { break; },
                        _ = tokio::time::sleep_until(deadline) => break,
                    }
                    status = self.store.status(key)?;
                    if status.state.is_terminal() {
                        break;
                    }
                }
                status = self.store.status(key)?;
            }
        }
        let Some(channel) = channel else {
            return Ok((status, None));
        };
        if status.expired {
            return Err(err(
                ExecutorErrorCode::ResultExpired,
                "the operation result was garbage collected",
            ));
        }
        let max = max_bytes
            .unwrap_or(EXECUTOR_MAX_QUERY_BYTES)
            .min(EXECUTOR_MAX_QUERY_BYTES);
        let (data, retained) = self
            .store
            .read_chunk(key, channel, offset, max)
            .map_err(internal)?;
        let summary = status.result.as_ref().and_then(|r| match channel {
            ExecutorChannel::Stdout => r.stdout.clone(),
            ExecutorChannel::Stderr => r.stderr.clone(),
            ExecutorChannel::Content => r.content.clone(),
            ExecutorChannel::Upload => None,
        });
        let next_offset = offset.max(0) + data.len() as u64;
        let truncated = summary.as_ref().map(|s| s.truncated).unwrap_or(false);
        let eof = status.state.is_terminal() && next_offset >= retained;
        use base64::Engine;
        Ok((
            status,
            Some(ExecutorChunk {
                channel,
                offset,
                next_offset,
                retained_from: 0,
                truncated,
                eof,
                data_b64: base64::engine::general_purpose::STANDARD.encode(&data),
            }),
        ))
    }

    async fn finish(&self, op: &ActiveOp, result: ExecutorOpResult) {
        let store = self.store.clone();
        let key = op.key.clone();
        let published = tokio::task::spawn_blocking(move || store.publish_result(&key, result)).await;
        let result = match published {
            Ok(Ok(r)) => Some(r),
            Ok(Err(e)) => {
                log::error!("executor: cannot publish result of {}: {e}", op.key);
                None
            }
            Err(e) => {
                log::error!("executor: publish task failed for {}: {e}", op.key);
                None
            }
        };
        self.active.lock().unwrap().remove(&op.key);
        op.bump();
        if let Some(result) = result {
            let _ = self.events.send(OpEvent::Finished {
                key: op.key.clone(),
                op_id: op.identity.op_id.clone(),
                result,
            });
        }
    }

    fn failed(error: ExecutorError, state: ExecutorOpState) -> ExecutorOpResult {
        ExecutorOpResult {
            state,
            exit_code: None,
            signal: None,
            error: Some(error),
            stdout: None,
            stderr: None,
            content: None,
            file: None,
            finished_at_ms: now_ms(),
        }
    }

    /// Under the gate: a cancel that arrived before the start makes the
    /// operation provably not executed.
    async fn cancelled_before_start(&self, op: &ActiveOp) -> bool {
        op.cancel.borrow().is_some()
    }

    async fn mark_running(&self, op: &ActiveOp, pid: Option<u32>) {
        *op.spawned.lock().unwrap() = true;
        let store = self.store.clone();
        let key = op.key.clone();
        let _ = tokio::task::spawn_blocking(move || {
            store.update_state(&key, |s| {
                s.state = ExecutorOpState::Running;
                s.pid = pid;
                s.started_at_ms = Some(now_ms());
            })
        })
        .await;
        op.bump();
        if let Some(pid) = pid {
            let _ = self.events.send(OpEvent::Started {
                key: op.key.clone(),
                op_id: op.identity.op_id.clone(),
                pid,
            });
        }
    }

    async fn run(self: Arc<Self>, op: Arc<ActiveOp>, record: OpRequestRecord) {
        let result = match &record.spec {
            ExecutorOpSpec::Exec { .. } => self.run_exec(&op, &record).await,
            _ => self.run_file(&op, &record).await,
        };
        self.finish(&op, result).await;
    }

    fn base_env(profile: &ExecutorProfile, overrides: &BTreeMap<String, String>) -> BTreeMap<String, String> {
        let mut env = profile.env.clone();
        for (k, v) in overrides {
            env.insert(k.clone(), v.clone());
        }
        #[cfg(unix)]
        {
            env.entry("PATH".into())
                .or_insert_with(|| "/usr/local/bin:/usr/bin:/bin".into());
            env.entry("LANG".into()).or_insert_with(|| DEFAULT_LANG.into());
            if let Some((home, name)) = profile.run_as.as_ref().and_then(|r| passwd_entry(r.uid)) {
                env.entry("HOME".into()).or_insert(home);
                env.entry("USER".into()).or_insert(name.clone());
                env.entry("LOGNAME".into()).or_insert(name);
            }
        }
        #[cfg(windows)]
        for name in WINDOWS_HOST_ENV {
            if env.keys().any(|k| k.eq_ignore_ascii_case(name)) {
                continue;
            }
            if let Ok(value) = std::env::var(name) {
                env.insert(name.to_string(), value);
            }
        }
        env
    }

    async fn run_exec(&self, op: &Arc<ActiveOp>, record: &OpRequestRecord) -> ExecutorOpResult {
        let ExecutorOpSpec::Exec {
            cmd,
            cwd,
            env_overrides,
            ..
        } = &record.spec
        else {
            unreachable!()
        };
        let run_as = op.profile.run_as.clone();
        let cwd = match self
            .worker_job(
                run_as.as_ref(),
                &WorkerJob::ResolveDir {
                    path: cwd.clone(),
                    roots: op.profile.workdir_roots.clone(),
                },
                &[],
                0,
            )
            .await
        {
            FileJobOutcome::Reply(reply, _) => match (reply.error, reply.path) {
                (None, Some(path)) => PathBuf::from(path),
                (Some(e), _) => return Self::failed(e, ExecutorOpState::Failed),
                _ => return Self::failed(internal("worker returned no cwd"), ExecutorOpState::Failed),
            },
            FileJobOutcome::SpawnFailed(e) | FileJobOutcome::Unknown(e) => {
                return Self::failed(e, ExecutorOpState::Failed)
            }
        };
        let env = Self::base_env(&op.profile, env_overrides);
        let gate = self.gate.lock().await;
        if self.cancelled_before_start(op).await {
            drop(gate);
            return Self::failed(
                err(ExecutorErrorCode::Unavailable, "cancelled before the command started"),
                ExecutorOpState::Cancelled,
            );
        }
        let mut shell = match spawn_shell(&self.shell, run_as.as_ref(), &cwd, &env) {
            Ok(c) => c,
            Err(e) => {
                drop(gate);
                return Self::failed(
                    err(ExecutorErrorCode::Internal, format!("cannot start shell: {e}")),
                    ExecutorOpState::Failed,
                );
            }
        };
        let pid = shell.pid();
        drop(gate);
        self.mark_running(op, Some(pid)).await;

        let mut stdin = shell.child.stdin.take().expect("piped stdin");
        let script = cmd.clone().into_bytes();
        tokio::spawn(async move {
            let _ = stdin.write_all(&script).await;
            let _ = stdin.shutdown().await;
        });
        let dir = self.store.op_dir(&op.key);
        let (stop_tx, stop_rx) = watch::channel(false);
        let limit = record.budget.max_output_bytes;
        let storage_failed = Arc::new(Mutex::new(false));
        let mut collectors = Vec::new();
        for (channel, pipe) in [
            (ExecutorChannel::Stdout, shell.child.stdout.take().map(Pipe::Out)),
            (ExecutorChannel::Stderr, shell.child.stderr.take().map(Pipe::Err)),
        ] {
            let Some(pipe) = pipe else { continue };
            collectors.push(tokio::spawn(collect(
                pipe,
                dir.join(channel.as_str()),
                limit,
                channel,
                op.clone(),
                self.events.clone(),
                stop_rx.clone(),
                storage_failed.clone(),
            )));
        }

        let timeout = Duration::from_millis(record.budget.execution_timeout_ms.max(1));
        let mut cancel_rx = op.cancel.subscribe();
        let mut timed_out = false;
        let mut cancelled = None;
        let sleep = tokio::time::sleep(timeout);
        tokio::pin!(sleep);
        let status = loop {
            tokio::select! {
                status = shell.child.wait() => break status,
                _ = &mut sleep, if !timed_out && cancelled.is_none() => {
                    timed_out = true;
                    shell.terminate().await;
                }
                changed = cancel_rx.changed(), if cancelled.is_none() && !timed_out => {
                    if changed.is_err() { continue; }
                    let reason = *cancel_rx.borrow();
                    if let Some(reason) = reason {
                        cancelled = Some(reason);
                        shell.terminate().await;
                    }
                }
            }
        };
        let drain_deadline = tokio::time::Instant::now() + DRAIN_GRACE;
        let mut summaries = Vec::new();
        for mut c in collectors {
            let joined = match tokio::time::timeout_at(drain_deadline, &mut c).await {
                Ok(r) => r,
                Err(_) => {
                    let _ = stop_tx.send(true);
                    c.await
                }
            };
            if let Ok(s) = joined {
                summaries.push(s);
            }
        }
        let stdout = summaries.iter().find(|s| s.0 == ExecutorChannel::Stdout).map(|s| s.1.clone());
        let stderr = summaries.iter().find(|s| s.0 == ExecutorChannel::Stderr).map(|s| s.1.clone());

        let (exit_code, signal) = match &status {
            Ok(s) => {
                #[cfg(unix)]
                let signal = {
                    use std::os::unix::process::ExitStatusExt;
                    s.signal()
                };
                #[cfg(not(unix))]
                let signal = None;
                (s.code(), signal)
            }
            Err(_) => (None, None),
        };
        let storage_failed = *storage_failed.lock().unwrap();
        let (state, error) = if storage_failed {
            (
                ExecutorOpState::Unknown,
                Some(err(ExecutorErrorCode::Internal, "output could not be persisted").unknown_effect()),
            )
        } else if let Err(e) = &status {
            (
                ExecutorOpState::Unknown,
                Some(err(ExecutorErrorCode::Internal, format!("wait failed: {e}")).unknown_effect()),
            )
        } else if timed_out {
            (
                ExecutorOpState::TimedOut,
                Some(err(ExecutorErrorCode::Unavailable, "execution timeout reached")),
            )
        } else if let Some(reason) = cancelled {
            (
                ExecutorOpState::Cancelled,
                Some(err(
                    ExecutorErrorCode::Unavailable,
                    format!("cancelled ({reason:?}); side effects already made are not rolled back"),
                )),
            )
        } else if exit_code == Some(0) {
            (ExecutorOpState::Succeeded, None)
        } else {
            (ExecutorOpState::Failed, None)
        };
        ExecutorOpResult {
            state,
            exit_code,
            signal,
            error,
            stdout,
            stderr,
            content: None,
            file: None,
            finished_at_ms: now_ms(),
        }
    }

    async fn worker_job(&self, run_as: Option<&ExecutorRunAs>, job: &WorkerJob, payload: &[u8], max_reply: u64) -> FileJobOutcome {
        run_file_job(&self.launcher, run_as, job, payload, max_reply).await
    }

    fn path_lock(&self, path: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.path_locks.lock().unwrap();
        locks.retain(|_, l| Arc::strong_count(l) > 1);
        locks
            .entry(path.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    async fn run_file(&self, op: &Arc<ActiveOp>, record: &OpRequestRecord) -> ExecutorOpResult {
        let run_as = op.profile.run_as.clone();
        let roots = op.profile.workdir_roots.clone();
        let max_file = record.budget.max_file_bytes;
        let dir = self.store.op_dir(&op.key);
        let mutating = !matches!(record.spec, ExecutorOpSpec::Read { .. });

        let mut _path_guard = None;
        if let ExecutorOpSpec::Write { path, .. } | ExecutorOpSpec::Edit { path, .. } = &record.spec {
            let target = match self
                .worker_job(
                    run_as.as_ref(),
                    &WorkerJob::ResolveTarget {
                        path: path.clone(),
                        roots: roots.clone(),
                    },
                    &[],
                    0,
                )
                .await
            {
                FileJobOutcome::Reply(reply, _) => match (reply.error, reply.path) {
                    (None, Some(p)) => p,
                    (Some(e), _) => return Self::failed(e, ExecutorOpState::Failed),
                    _ => return Self::failed(internal("worker returned no path"), ExecutorOpState::Failed),
                },
                FileJobOutcome::SpawnFailed(e) | FileJobOutcome::Unknown(e) => {
                    return Self::failed(e, ExecutorOpState::Failed)
                }
            };
            _path_guard = Some(self.path_lock(&target).lock_owned().await);
        }

        let (job, payload) = match &record.spec {
            ExecutorOpSpec::Read { path, offset, length } => (
                WorkerJob::Read {
                    path: path.clone(),
                    roots,
                    offset: *offset,
                    length: *length,
                    max_bytes: max_file,
                },
                Vec::new(),
            ),
            ExecutorOpSpec::Write {
                path,
                mode,
                expected_hash,
                content_length,
                ..
            } => {
                let content = match tokio::fs::read(dir.join("upload")).await {
                    Ok(c) => c,
                    Err(e) => return Self::failed(internal(format!("upload missing: {e}")), ExecutorOpState::Failed),
                };
                (
                    WorkerJob::Write {
                        path: path.clone(),
                        roots,
                        mode: *mode,
                        expected_hash: expected_hash.clone(),
                        content_length: *content_length,
                        max_bytes: max_file,
                    },
                    content,
                )
            }
            ExecutorOpSpec::Edit {
                path,
                edits,
                expected_hash,
            } => (
                WorkerJob::Edit {
                    path: path.clone(),
                    roots,
                    edits: edits.clone(),
                    expected_hash: expected_hash.clone(),
                    max_bytes: max_file,
                },
                Vec::new(),
            ),
            ExecutorOpSpec::Exec { .. } => unreachable!(),
        };

        let gate = self.gate.lock().await;
        if self.cancelled_before_start(op).await {
            drop(gate);
            return Self::failed(
                err(ExecutorErrorCode::Unavailable, "cancelled before the operation started"),
                ExecutorOpState::Cancelled,
            );
        }
        drop(gate);
        self.mark_running(op, None).await;
        let reply_payload = if matches!(record.spec, ExecutorOpSpec::Write { .. }) { 0 } else { max_file };
        match self.worker_job(run_as.as_ref(), &job, &payload, reply_payload).await {
            FileJobOutcome::SpawnFailed(e) => Self::failed(e, ExecutorOpState::Failed),
            FileJobOutcome::Unknown(e) => {
                if mutating {
                    Self::failed(e.unknown_effect(), ExecutorOpState::Unknown)
                } else {
                    Self::failed(e, ExecutorOpState::Failed)
                }
            }
            FileJobOutcome::Reply(reply, data) => {
                if let Some(e) = reply.error {
                    return Self::failed(e, ExecutorOpState::Failed);
                }
                let mut content = None;
                if !matches!(record.spec, ExecutorOpSpec::Write { .. }) {
                    let path = dir.join(ExecutorChannel::Content.as_str());
                    let write = async {
                        let mut f = tokio::fs::File::create(&path).await?;
                        f.write_all(&data).await?;
                        f.sync_all().await
                    };
                    if let Err(e) = write.await {
                        let state = if mutating { ExecutorOpState::Unknown } else { ExecutorOpState::Failed };
                        return Self::failed(
                            err(ExecutorErrorCode::Internal, format!("cannot persist content: {e}")).unknown_effect(),
                            state,
                        );
                    }
                    content = Some(ExecutorOutputSummary {
                        total_bytes: data.len() as u64,
                        retained_bytes: data.len() as u64,
                        truncated: false,
                        hash: Some(executor_content_hash(&data)),
                    });
                }
                let file: Option<ExecutorFileResult> = reply.file;
                ExecutorOpResult {
                    state: ExecutorOpState::Succeeded,
                    exit_code: None,
                    signal: None,
                    error: None,
                    stdout: None,
                    stderr: None,
                    content,
                    file,
                    finished_at_ms: now_ms(),
                }
            }
        }
    }
}

enum Pipe {
    Out(tokio::process::ChildStdout),
    Err(tokio::process::ChildStderr),
}

impl Pipe {
    async fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Pipe::Out(p) => p.read(buf).await,
            Pipe::Err(p) => p.read(buf).await,
        }
    }
}

/// Keep the first `limit` bytes, keep draining the rest, fsync at the end.
#[allow(clippy::too_many_arguments)]
async fn collect(
    mut pipe: Pipe,
    path: PathBuf,
    limit: u64,
    channel: ExecutorChannel,
    op: Arc<ActiveOp>,
    events: broadcast::Sender<OpEvent>,
    mut stop: watch::Receiver<bool>,
    storage_failed: Arc<Mutex<bool>>,
) -> (ExecutorChannel, ExecutorOutputSummary) {
    let mut file = match tokio::fs::File::create(&path).await {
        Ok(f) => Some(f),
        Err(_) => {
            *storage_failed.lock().unwrap() = true;
            None
        }
    };
    let mut total = 0u64;
    let mut retained = 0u64;
    let mut buf = vec![0u8; PUSH_CHUNK];
    loop {
        let n = tokio::select! {
            r = pipe.read(&mut buf) => match r { Ok(0) | Err(_) => break, Ok(n) => n },
            _ = stop.changed() => break,
        };
        total += n as u64;
        if retained < limit {
            let keep = ((limit - retained) as usize).min(n);
            let chunk = &buf[..keep];
            if let Some(f) = file.as_mut() {
                if f.write_all(chunk).await.is_err() {
                    *storage_failed.lock().unwrap() = true;
                    file = None;
                    let _ = op.cancel.send_if_modified(|v| {
                        if v.is_none() {
                            *v = Some(CancelReason::Storage);
                            true
                        } else {
                            false
                        }
                    });
                }
            }
            let _ = events.send(OpEvent::Output {
                key: op.key.clone(),
                op_id: op.identity.op_id.clone(),
                channel,
                offset: retained,
                data: Arc::new(chunk.to_vec()),
            });
            retained += keep as u64;
            op.bump();
        }
    }
    let mut hash = None;
    if let Some(f) = file.as_mut() {
        if f.flush().await.is_err() || f.sync_all().await.is_err() {
            *storage_failed.lock().unwrap() = true;
        } else if let Ok(data) = tokio::fs::read(&path).await {
            hash = Some(executor_content_hash(&data));
        }
    }
    (
        channel,
        ExecutorOutputSummary {
            total_bytes: total,
            retained_bytes: retained,
            truncated: total > retained,
            hash,
        },
    )
}

#[cfg(unix)]
fn passwd_entry(uid: u32) -> Option<(String, String)> {
    let mut buf = vec![0 as libc::c_char; 16 * 1024];
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut out: *mut libc::passwd = std::ptr::null_mut();
    let rc = unsafe { libc::getpwuid_r(uid, &mut pwd, buf.as_mut_ptr(), buf.len(), &mut out) };
    if rc != 0 || out.is_null() {
        return None;
    }
    let home = unsafe { std::ffi::CStr::from_ptr(pwd.pw_dir) }.to_string_lossy().to_string();
    let name = unsafe { std::ffi::CStr::from_ptr(pwd.pw_name) }.to_string_lossy().to_string();
    Some((home, name))
}
