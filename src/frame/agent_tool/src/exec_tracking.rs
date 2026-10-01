//! Tool execution tracking shared by xllm and libOpenDAN (Agent Session SDK
//! plan §5.2 / §8.7 X6).
//!
//! A run lock only protects the run directory. A runner that is killed
//! (`kill -9`) releases its locks immediately, but the tool processes it
//! started may still be running. Before anyone continues such a run, the old
//! executions must be proven stopped. This module provides:
//!
//! - the persisted records ([`ExecutionRecord`], [`InflightAction`],
//!   [`HostRunInfo`]) carried by xllm's `RunRecord`;
//! - [`probe_execution`] / [`stop_execution`]: find the processes of an
//!   execution by an environment marker (`OPENDAN_EXECUTION_ID`) instead of a
//!   possibly reused PID / PGID, terminate them and wait until they are gone;
//!   anything that cannot be verified is reported as `Unknown` (the caller
//!   blocks recovery);
//! - [`TrackedBashRunner`]: a [`BashRunner`] with a launch handshake — the
//!   execution identity is persisted through an [`ExecutionRegistrar`]
//!   *before* the user command is allowed to start.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::time::timeout as tokio_timeout;

use crate::llm_bash::{
    drain_pipe, BashRunOutput, BashRunRequest, BashRunner, BashTarget, OutputCollector,
    ProcessGroupGuard, RunCollectors, LOCAL_ENGINE, PIPE_DRAIN_GRACE, TIMEOUT_EXIT_CODE,
};
use crate::{AgentToolError, SessionRuntimeContext};

/// Environment variable carrying the execution id into every process of a
/// tracked execution (inherited by background children).
pub const EXECUTION_ENV: &str = "OPENDAN_EXECUTION_ID";

/// Exit code of the launch wrapper when it was never released.
pub const NOT_RELEASED_EXIT_CODE: i32 = 125;

/// A tool action that was dispatched but whose result is not persisted yet.
/// Written (fsync) before the tool starts; cleared only after a checkpoint
/// that contains its result (or an explicit `Unresolved`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InflightAction {
    pub call_id: String,
    pub tool: String,
    #[serde(default)]
    pub args: Value,
    /// `read_only | idempotent | side_effect | unknown`.
    #[serde(default = "unknown_effect")]
    pub effect: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub execution_ids: Vec<String>,
    /// Behavior step index the action belongs to (behavior loop only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_index: Option<u32>,
    pub started_at_ms: u64,
}

fn unknown_effect() -> String {
    "unknown".to_string()
}

/// A managed process execution (native runtime). Kept until every process of
/// the execution is confirmed stopped — independent of the in-flight marker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutionRecord {
    pub execution_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    /// `native` (tracked through /proc) — other kinds are reported Unknown.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pgid: Option<u32>,
    /// Start time (clock ticks since boot) of the process group leader.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leader_start_ticks: Option<u64>,
    #[serde(default)]
    pub command: String,
    pub started_at_ms: u64,
}

/// Marks a run assembled by a host (e.g. libOpenDAN) rather than by xllm's
/// own `.llm_context` preparation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostRunInfo {
    /// `libopendan` …
    pub assembled_by: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// `native | tmux` — xllm only takes over native runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_id: Option<String>,
    /// Environment facts xllm must verify before continuing (PATH layers,
    /// required tools …).
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub env_check: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub extra: Value,
}

/// Result of probing an execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionProbe {
    /// No process of the execution exists any more.
    Stopped,
    /// These verified processes still run.
    Alive { pids: Vec<u32> },
    /// Cannot prove the execution stopped (other host, no /proc, unreadable
    /// process of the same group …). Recovery must block.
    Unknown { reason: String },
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

static EXEC_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn new_execution_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let n = EXEC_COUNTER.fetch_add(1, Ordering::SeqCst);
    let seed = format!("{nanos}:{}:{n}", std::process::id());
    let hash = blake3::hash(seed.as_bytes()).to_hex();
    format!("ex-{}", &hash.as_str()[..24])
}

/// Kernel boot id (Linux). Processes cannot survive a reboot.
pub fn current_boot_id() -> Option<String> {
    std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

struct ProcStat {
    state: char,
    pgrp: u32,
    start_ticks: u64,
}

fn read_stat(pid: u32) -> Option<ProcStat> {
    let raw = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // comm may contain spaces / parens: split after the last ')'.
    let close = raw.rfind(')')?;
    let rest: Vec<&str> = raw[close + 1..].split_whitespace().collect();
    // rest[0] = state (field 3), rest[2] = pgrp (field 5), rest[19] = starttime (field 22)
    Some(ProcStat {
        state: rest.first()?.chars().next()?,
        pgrp: rest.get(2)?.parse().ok()?,
        start_ticks: rest.get(19)?.parse().ok()?,
    })
}

/// Start ticks of a live process (used to pin the group leader identity).
pub fn process_start_ticks(pid: u32) -> Option<u64> {
    read_stat(pid).map(|s| s.start_ticks)
}

enum EnvCheck {
    Marker,
    NoMarker,
    Unreadable,
}

fn env_has_marker(pid: u32, marker: &[u8]) -> EnvCheck {
    match std::fs::read(format!("/proc/{pid}/environ")) {
        Ok(bytes) => {
            if bytes.split(|b| *b == 0).any(|entry| entry == marker) {
                EnvCheck::Marker
            } else {
                EnvCheck::NoMarker
            }
        }
        Err(_) => EnvCheck::Unreadable,
    }
}

/// Probe an execution. `local_host` is this machine's host id as used in
/// `ExecutionRecord.host` (when both are set and differ → Unknown).
pub fn probe_execution(rec: &ExecutionRecord, local_host: Option<&str>) -> ExecutionProbe {
    if rec.kind != "native" {
        return ExecutionProbe::Unknown {
            reason: format!("execution kind `{}` cannot be verified here", rec.kind),
        };
    }
    if let (Some(h), Some(local)) = (rec.host.as_deref(), local_host) {
        if h != local {
            return ExecutionProbe::Unknown {
                reason: format!("execution ran on host {h}, this is {local}"),
            };
        }
    }
    if !std::path::Path::new("/proc/self/stat").exists() {
        return ExecutionProbe::Unknown {
            reason: "execution tracking requires /proc".into(),
        };
    }
    match (rec.boot_id.as_deref(), current_boot_id()) {
        (Some(recorded), Some(now)) if recorded != now => return ExecutionProbe::Stopped,
        (None, _) | (_, None) => {
            return ExecutionProbe::Unknown {
                reason: "no boot id recorded for the execution".into(),
            }
        }
        _ => {}
    }
    let marker = format!("{EXECUTION_ENV}={}", rec.execution_id).into_bytes();
    let me = std::process::id();
    let mut alive = Vec::new();
    let mut unknown = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return ExecutionProbe::Unknown {
            reason: "cannot list /proc".into(),
        };
    };
    // Leader identity: a leader with a different start time means the PGID was
    // reused by an unrelated group.
    let leader_reused = match (rec.pgid, rec.leader_start_ticks) {
        (Some(pgid), Some(start)) => read_stat(pgid)
            .map(|s| s.start_ticks != start)
            .unwrap_or(false),
        _ => false,
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        if pid == me {
            continue;
        }
        let Some(stat) = read_stat(pid) else {
            continue;
        };
        if stat.state == 'Z' || stat.state == 'X' {
            continue;
        }
        match env_has_marker(pid, &marker) {
            EnvCheck::Marker => alive.push(pid),
            EnvCheck::NoMarker => {
                if Some(stat.pgrp) == rec.pgid && !leader_reused {
                    // Same group without the marker (env cleared?): only a
                    // process that started after our leader can be ours.
                    let after_leader = rec
                        .leader_start_ticks
                        .map(|s| stat.start_ticks >= s)
                        .unwrap_or(true);
                    if after_leader {
                        unknown.push(pid);
                    }
                }
            }
            EnvCheck::Unreadable => {
                if Some(stat.pgrp) == rec.pgid && !leader_reused {
                    unknown.push(pid);
                }
            }
        }
    }
    if !unknown.is_empty() {
        return ExecutionProbe::Unknown {
            reason: format!(
                "processes {:?} share the execution's process group but cannot be verified",
                unknown
            ),
        };
    }
    if alive.is_empty() {
        ExecutionProbe::Stopped
    } else {
        alive.sort_unstable();
        ExecutionProbe::Alive { pids: alive }
    }
}

fn kill_pid(pid: u32) {
    let _ = std::process::Command::new("kill")
        .arg("-KILL")
        .arg(pid.to_string())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// Terminate every verified process of the execution and wait until none is
/// left. `Err` when the execution cannot be proven stopped within `wait`.
pub async fn stop_execution(
    rec: &ExecutionRecord,
    local_host: Option<&str>,
    wait: Duration,
) -> Result<(), String> {
    let started = Instant::now();
    loop {
        match probe_execution(rec, local_host) {
            ExecutionProbe::Stopped => return Ok(()),
            ExecutionProbe::Unknown { reason } => return Err(reason),
            ExecutionProbe::Alive { pids } => {
                if started.elapsed() > wait {
                    return Err(format!(
                        "processes {pids:?} of execution {} did not exit",
                        rec.execution_id
                    ));
                }
                for pid in pids {
                    kill_pid(pid);
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
}

/// Persists execution identities for a [`TrackedBashRunner`].
#[async_trait]
pub trait ExecutionRegistrar: Send + Sync {
    /// Persist the identity (fsync). The command only starts after `Ok`.
    async fn register(&self, rec: &ExecutionRecord) -> Result<(), String>;
    /// Every process of the execution is gone; the record may be removed.
    async fn completed(&self, execution_id: &str);
}

/// Bash runner with a launch handshake and execution tracking.
///
/// The spawned wrapper waits for a `go` line on stdin before `exec`ing the
/// user command. If the registrar fails, or the runner dies before releasing
/// it, the pipe closes and the command never runs.
pub struct TrackedBashRunner {
    registrar: Arc<dyn ExecutionRegistrar>,
    runtime_id: Option<String>,
    host: Option<String>,
    /// PATH layers prepended in order (`layers[0]` first).
    path_layers: Vec<PathBuf>,
    extra_env: Vec<(String, String)>,
    /// Remaining processes after the shell returns stay tracked; this map
    /// remembers them for `completed` notifications from later probes.
    pending: Mutex<HashMap<String, ExecutionRecord>>,
}

impl TrackedBashRunner {
    pub fn new(registrar: Arc<dyn ExecutionRegistrar>) -> Self {
        Self {
            registrar,
            runtime_id: None,
            host: None,
            path_layers: Vec::new(),
            extra_env: Vec::new(),
            pending: Mutex::new(HashMap::new()),
        }
    }

    pub fn with_runtime(mut self, runtime_id: Option<String>, host: Option<String>) -> Self {
        self.runtime_id = runtime_id;
        self.host = host;
        self
    }

    pub fn with_path_layers(mut self, layers: Vec<PathBuf>) -> Self {
        self.path_layers = layers;
        self
    }

    pub fn with_env(mut self, env: Vec<(String, String)>) -> Self {
        self.extra_env = env;
        self
    }

    fn build_path(&self, user_env: &[(String, String)]) -> String {
        let base = user_env
            .iter()
            .rev()
            .find(|(k, _)| k == "PATH")
            .map(|(_, v)| v.clone())
            .or_else(|| std::env::var("PATH").ok())
            .unwrap_or_else(|| "/usr/local/bin:/usr/bin:/bin".to_string());
        let mut parts: Vec<String> = self
            .path_layers
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        for p in base.split(':') {
            if !p.is_empty() && !parts.iter().any(|x| x == p) {
                parts.push(p.to_string());
            }
        }
        parts.join(":")
    }

    /// Re-probe executions whose shell returned while children kept running.
    pub async fn sweep(&self) {
        let recs: Vec<ExecutionRecord> = self
            .pending
            .lock()
            .expect("pending lock")
            .values()
            .cloned()
            .collect();
        for rec in recs {
            if probe_execution(&rec, self.host.as_deref()) == ExecutionProbe::Stopped {
                self.pending
                    .lock()
                    .expect("pending lock")
                    .remove(&rec.execution_id);
                self.registrar.completed(&rec.execution_id).await;
            }
        }
    }
}

const LAUNCH_WRAPPER: &str = "IFS= read -r __od_go || exit 125; [ \"$__od_go\" = go ] || exit 125; unset __od_go; exec /bin/bash -c \"$0\"";

#[async_trait]
impl BashRunner for TrackedBashRunner {
    async fn run(
        &self,
        _ctx: &SessionRuntimeContext,
        req: BashRunRequest,
    ) -> Result<BashRunOutput, AgentToolError> {
        match req.target {
            BashTarget::Local => {}
            BashTarget::Unsupported(value) => {
                return Err(AgentToolError::InvalidArgs(format!(
                    "unsupported exec target `{value}` (only local is supported)"
                )));
            }
        }
        let execution_id = new_execution_id();
        let mut cmd = tokio::process::Command::new("/bin/bash");
        cmd.arg("-c").arg(LAUNCH_WRAPPER).arg(&req.command);
        cmd.current_dir(&req.cwd);
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        cmd.kill_on_drop(true);
        #[cfg(unix)]
        cmd.process_group(0);
        for (k, v) in &self.extra_env {
            cmd.env(k, v);
        }
        for (k, v) in &req.env {
            cmd.env(k, v);
        }
        cmd.env("PATH", self.build_path(&req.env));
        cmd.env(EXECUTION_ENV, &execution_id);

        let mut child = cmd
            .spawn()
            .map_err(|err| AgentToolError::ExecFailed(format!("spawn bash failed: {err}")))?;
        let pgid = child.id();
        let mut guard = ProcessGroupGuard::new(pgid);
        let rec = ExecutionRecord {
            execution_id: execution_id.clone(),
            call_id: None,
            kind: "native".to_string(),
            runtime_id: self.runtime_id.clone(),
            host: self.host.clone(),
            boot_id: current_boot_id(),
            pgid,
            leader_start_ticks: pgid.and_then(process_start_ticks),
            command: req.command.chars().take(512).collect(),
            started_at_ms: now_ms(),
        };
        if let Err(e) = self.registrar.register(&rec).await {
            // Never released: the wrapper exits without running the command.
            drop(child.stdin.take());
            guard.kill();
            let _ = child.kill().await;
            return Err(AgentToolError::ExecFailed(format!(
                "execution tracking could not be persisted; command not started: {e}"
            )));
        }
        // Release the command.
        if let Some(mut stdin) = child.stdin.take() {
            if let Err(e) = stdin.write_all(b"go\n").await {
                guard.kill();
                let _ = child.kill().await;
                return Err(AgentToolError::ExecFailed(format!(
                    "failed to release command: {e}"
                )));
            }
            let _ = stdin.flush().await;
            drop(stdin);
        }

        let started = Instant::now();
        let collectors = Arc::new(Mutex::new(RunCollectors {
            stdout: OutputCollector::new(req.max_output_bytes),
            stderr: OutputCollector::new(req.max_output_bytes),
            combined: OutputCollector::new(req.max_output_bytes),
        }));
        let mut drains = Vec::new();
        if let Some(out) = child.stdout.take() {
            drains.push(tokio::spawn(drain_pipe(out, collectors.clone(), false)));
        }
        if let Some(err) = child.stderr.take() {
            drains.push(tokio::spawn(drain_pipe(err, collectors.clone(), true)));
        }
        let status = match tokio_timeout(Duration::from_millis(req.timeout_ms), child.wait()).await
        {
            Ok(Ok(status)) => {
                guard.disarm();
                Some(status)
            }
            Ok(Err(err)) => {
                return Err(AgentToolError::ExecFailed(format!("wait bash failed: {err}")));
            }
            Err(_) => {
                guard.kill();
                let _ = child.kill().await;
                None
            }
        };
        let elapsed = started.elapsed();
        let _ = tokio_timeout(PIPE_DRAIN_GRACE, async {
            for handle in drains {
                let _ = handle.await;
            }
        })
        .await;
        let (stdout, stderr, output, output_truncated) = {
            let c = collectors.lock().expect("bash output lock");
            (
                c.stdout.render(),
                c.stderr.render(),
                c.combined.render(),
                c.combined.is_truncated(),
            )
        };
        // The shell returned; background children may keep the execution
        // alive. Only a verified stop releases the record.
        match probe_execution(&rec, self.host.as_deref()) {
            ExecutionProbe::Stopped => self.registrar.completed(&execution_id).await,
            _ => {
                self.pending
                    .lock()
                    .expect("pending lock")
                    .insert(execution_id.clone(), rec);
            }
        }
        let exit_code = match status {
            None => TIMEOUT_EXIT_CODE,
            Some(status) => status.code().unwrap_or_else(|| {
                #[cfg(unix)]
                {
                    use std::os::unix::process::ExitStatusExt;
                    status.signal().map(|sig| 128 + sig).unwrap_or(-1)
                }
                #[cfg(not(unix))]
                {
                    -1
                }
            }),
        };
        Ok(BashRunOutput {
            exit_code,
            stdout,
            stderr,
            output,
            output_truncated,
            timed_out: status.is_none(),
            duration_ms: elapsed.as_millis() as u64,
            engine: LOCAL_ENGINE.to_string(),
            cwd: req.cwd,
        })
    }
}

/// Minimal registrar that keeps records in memory (tests, non-persistent
/// hosts).
#[derive(Default)]
pub struct MemoryRegistrar {
    pub records: Mutex<Vec<ExecutionRecord>>,
    pub completed: Mutex<Vec<String>>,
    pub fail: std::sync::atomic::AtomicBool,
}

#[async_trait]
impl ExecutionRegistrar for MemoryRegistrar {
    async fn register(&self, rec: &ExecutionRecord) -> Result<(), String> {
        if self.fail.load(Ordering::SeqCst) {
            return Err("registration disabled".into());
        }
        self.records.lock().expect("lock").push(rec.clone());
        Ok(())
    }

    async fn completed(&self, execution_id: &str) {
        self.completed
            .lock()
            .expect("lock")
            .push(execution_id.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(cmd: &str, dir: &std::path::Path) -> BashRunRequest {
        BashRunRequest {
            command: cmd.to_string(),
            cwd: dir.to_path_buf(),
            timeout_ms: 10_000,
            max_output_bytes: 64 * 1024,
            env: Vec::new(),
            target: BashTarget::Local,
        }
    }

    fn ctx() -> SessionRuntimeContext {
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

    #[tokio::test]
    async fn tracked_runner_runs_and_completes() {
        let dir = tempfile::tempdir().unwrap();
        let reg = Arc::new(MemoryRegistrar::default());
        let runner = TrackedBashRunner::new(reg.clone());
        let out = runner
            .run(&ctx(), req("echo hi; echo $OPENDAN_EXECUTION_ID", dir.path()))
            .await
            .unwrap();
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.starts_with("hi\nex-"));
        let recs = reg.records.lock().unwrap().clone();
        assert_eq!(recs.len(), 1);
        assert!(out.stdout.contains(&recs[0].execution_id));
        assert_eq!(reg.completed.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn failed_registration_never_runs_command() {
        let dir = tempfile::tempdir().unwrap();
        let reg = Arc::new(MemoryRegistrar::default());
        reg.fail.store(true, Ordering::SeqCst);
        let runner = TrackedBashRunner::new(reg.clone());
        let marker = dir.path().join("ran");
        let cmd = format!("touch {}", marker.display());
        let err = runner.run(&ctx(), req(&cmd, dir.path())).await;
        assert!(err.is_err());
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!marker.exists(), "command must not run without registration");
    }

    #[tokio::test]
    async fn background_child_stays_tracked_until_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let reg = Arc::new(MemoryRegistrar::default());
        let runner = TrackedBashRunner::new(reg.clone());
        let out = runner
            .run(
                &ctx(),
                req("(sleep 30 >/dev/null 2>&1 &) ; echo started", dir.path()),
            )
            .await
            .unwrap();
        assert_eq!(out.exit_code, 0);
        let rec = reg.records.lock().unwrap()[0].clone();
        assert!(reg.completed.lock().unwrap().is_empty());
        match probe_execution(&rec, None) {
            ExecutionProbe::Alive { pids } => assert!(!pids.is_empty()),
            other => panic!("expected alive background child, got {other:?}"),
        }
        stop_execution(&rec, None, Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(probe_execution(&rec, None), ExecutionProbe::Stopped);
        runner.sweep().await;
        assert_eq!(reg.completed.lock().unwrap().len(), 1);
    }

    #[test]
    fn other_host_is_unknown() {
        let rec = ExecutionRecord {
            execution_id: "ex-x".into(),
            call_id: None,
            kind: "native".into(),
            runtime_id: None,
            host: Some("did:dev:a".into()),
            boot_id: current_boot_id(),
            pgid: None,
            leader_start_ticks: None,
            command: String::new(),
            started_at_ms: 0,
        };
        assert!(matches!(
            probe_execution(&rec, Some("did:dev:b")),
            ExecutionProbe::Unknown { .. }
        ));
        assert_eq!(probe_execution(&rec, Some("did:dev:a")), ExecutionProbe::Stopped);
    }
}

// -------------------------------------------------------------------------
// Unresolved in-flight actions
// -------------------------------------------------------------------------

use buckyos_api::{AiContent, AiMessage, AiRole, AiToolCall};
use llm_context::behavior_loop::{StepMeta, StepRecord};
use llm_context::observation::Observation;
use llm_context::state::LLMContextSnapshot;

/// Text shown to the model for an action whose effect is unknown.
pub fn unresolved_reason(tool: &str) -> String {
    format!(
        "the `{tool}` call was dispatched before the previous runner stopped; its effect is unknown. Do not assume it succeeded or failed — check the actual state before repeating it."
    )
}

fn args_map(v: &Value) -> HashMap<String, Value> {
    match v {
        Value::Object(m) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        _ => HashMap::new(),
    }
}

fn fc_has_result(snapshot: &LLMContextSnapshot, call_id: &str) -> bool {
    snapshot.state.accumulated.iter().any(|m| {
        m.content.iter().any(|c| {
            matches!(c, AiContent::ToolResult { call_id: id, .. } if id == call_id)
        })
    })
}

fn fc_has_call(snapshot: &LLMContextSnapshot, call_id: &str) -> bool {
    snapshot.state.accumulated.iter().any(|m| {
        m.content
            .iter()
            .any(|c| matches!(c, AiContent::ToolUse { call_id: id, .. } if id == call_id))
    })
}

/// Inject an explicit "result unknown" for every in-flight action that has
/// no persisted outcome in `snapshot`. Actions that already have a result are
/// left alone (never marked unknown). Returns the call ids injected.
///
/// Function call mode: appends the assistant tool call (if missing) and a
/// tool result. Behavior mode: completes a partial step, or appends a
/// synthetic step carrying the action and an `Unresolved` observation.
pub fn materialize_unresolved(
    snapshot: &mut LLMContextSnapshot,
    inflight: &[InflightAction],
    behavior: bool,
) -> Vec<String> {
    let mut injected = Vec::new();
    for a in inflight {
        let reason = unresolved_reason(&a.tool);
        if !behavior {
            if fc_has_result(snapshot, &a.call_id) {
                continue;
            }
            if !fc_has_call(snapshot, &a.call_id) {
                snapshot.state.accumulated.push(AiMessage::new(
                    AiRole::Assistant,
                    vec![AiContent::tool_use(
                        a.call_id.clone(),
                        a.tool.clone(),
                        args_map(&a.args),
                    )],
                ));
            }
            snapshot.state.accumulated.push(AiMessage::new(
                AiRole::Tool,
                vec![AiContent::tool_result_text(
                    a.call_id.clone(),
                    format!("[unresolved] {reason}"),
                    true,
                )],
            ));
            injected.push(a.call_id.clone());
            continue;
        }
        // Behavior mode.
        let state = &mut snapshot.state;
        let mut handled = false;
        for step in state.steps.iter_mut().chain(state.last_step.iter_mut()) {
            if let Some(pos) = step.actions.iter().position(|x| x.call_id == a.call_id) {
                if step.action_results.len() > pos {
                    handled = true; // already has an outcome
                } else {
                    while step.action_results.len() < pos {
                        let missing = step.actions[step.action_results.len()].call_id.clone();
                        step.action_results.push(Observation::Unresolved {
                            call_id: missing,
                            reason: "not executed".into(),
                            effect_unknown: false,
                        });
                    }
                    step.action_results.push(Observation::Unresolved {
                        call_id: a.call_id.clone(),
                        reason: reason.clone(),
                        effect_unknown: true,
                    });
                    injected.push(a.call_id.clone());
                    handled = true;
                }
                break;
            }
        }
        if handled {
            continue;
        }
        let step_index = state.next_step_index;
        state.next_step_index = state.next_step_index.saturating_add(1);
        if let Ok(n) = a.call_id.parse::<u32>() {
            state.next_action_id = state.next_action_id.max(n);
        }
        let step = StepRecord {
            meta: StepMeta {
                behavior_name: snapshot.request.behavior_name.clone(),
                step_index,
                started_at_ms: a.started_at_ms,
                ended_at_ms: Some(a.started_at_ms),
                compression_level: Default::default(),
            },
            assistant_text: format!(
                "(recovered) dispatched `{}` before the previous runner stopped",
                a.tool
            ),
            actions: vec![AiToolCall {
                name: a.tool.clone(),
                args: args_map(&a.args),
                call_id: a.call_id.clone(),
            }],
            action_results: vec![Observation::Unresolved {
                call_id: a.call_id.clone(),
                reason,
                effect_unknown: true,
            }],
            ..Default::default()
        };
        if let Some(prev) = state.last_step.replace(step) {
            state.steps.push(prev);
        }
        injected.push(a.call_id.clone());
    }
    injected
}

/// Call ids that have a persisted outcome in `snapshot`.
pub fn persisted_outcome_ids(snapshot: &LLMContextSnapshot) -> Vec<String> {
    let mut out = Vec::new();
    for m in &snapshot.state.accumulated {
        for c in &m.content {
            if let AiContent::ToolResult { call_id, .. } = c {
                out.push(call_id.clone());
            }
        }
    }
    let st = &snapshot.state;
    for step in st.steps.iter().chain(st.last_step.iter()) {
        for (i, a) in step.actions.iter().enumerate() {
            if i < step.action_results.len() {
                out.push(a.call_id.clone());
            }
        }
    }
    out
}
