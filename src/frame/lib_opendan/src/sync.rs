//! Sync tasks: the programs that fill the AI Workspace's data sources from
//! outside (`kind: zone` sources read their output), run by the Agent's host
//! on their own timetable, independent of Goals.
//!
//! A task lives in `<agent_root>/sync/<task_id>/`: `task.json` (the
//! definition), `program.ts` (the current version), `state.json` (what the
//! program keeps between runs), `runs.jsonl` (the runs), `out.json` (the
//! `aiws.sync.v1` envelope the data source reads) and `schedule.json` (the
//! host's progress: the last slot, the failures in a row).
//!
//! The program is a Deno script: it reads `{task_id, inputs, secrets,
//! state}` on stdin and prints one JSON object `{status, error?,
//! error_kind?, business_time?, fields?, records, truncated?, state?}`. It
//! runs with network access to its declared hosts only, no file access, an
//! empty environment, 120 s and 32 MiB of output; the host writes the
//! envelope. The values of its credentials never appear in results, logs,
//! run records or errors.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use crate::error::{OpenDanError, Result};
use crate::goal_time::Timetable;
use crate::host::goals::{Clock, SystemClock};

pub const SYNC_DIR: &str = "sync";
/// The shortest interval of a task's timetable.
pub const MIN_EVERY_SECS: u64 = 900;
pub const RUN_TIMEOUT: Duration = Duration::from_secs(120);
/// What a program may print on stdout.
pub const OUTPUT_MAX: usize = 32 * 1024 * 1024;
pub const PROGRAM_MAX: usize = 256 * 1024;
pub const STATE_MAX: usize = 1024 * 1024;
pub const INPUTS_MAX: usize = 64 * 1024;
/// What is kept of a program's stderr.
pub const LOG_MAX: usize = 16 * 1024;
/// The longest a `sync.run` waits for its run.
pub const WAIT_MAX_MS: u64 = 120_000;
/// Program failures in a row before the owner is told.
pub const PROGRAM_FAILURES_TOLD: u32 = 3;
const RUNS_KEEP: usize = 200;
const SAMPLE: usize = 3;
const CONVENTION: &str = "a sync program prints one JSON object {status: \"ok\" | \"failed\", error?, error_kind?: \"transient\" | \"program\" | \"auth\", business_time?, fields?, records, truncated?, state?}";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// Retried after 30 s and 120 s (a timeout or a network error too).
    Transient,
    /// The program threw, or printed something that is not a result.
    Program,
    /// A credential is missing or refused: the owner is told at once.
    Auth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncSchedule {
    pub every_secs: u64,
}

/// `task.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskDef {
    pub task_id: String,
    pub title: String,
    pub version: u64,
    pub hosts: Vec<String>,
    pub schedule: SyncSchedule,
    #[serde(default)]
    pub inputs: Value,
    /// Names of the credentials it takes (their values are elsewhere).
    #[serde(default)]
    pub secrets: Vec<String>,
    pub enabled: bool,
    pub updated_at: String,
}

/// One run (`runs.jsonl`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub run_id: String,
    pub at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    pub status: RunStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<FailureKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub records: Option<usize>,
    /// `schedule` | `manual` | `aiworkspace` | `try`.
    pub trigger: String,
}

/// `schedule.json`: what the host keeps of a task.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Progress {
    #[serde(default)]
    every_secs: u64,
    #[serde(default)]
    last_slot: Option<u64>,
    /// Failed runs in a row.
    #[serde(default)]
    failures: u32,
    /// The owner was told about this streak of failures.
    #[serde(default)]
    told: bool,
    #[serde(default)]
    last_run: Option<Run>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SecretView {
    pub name: String,
    pub set: bool,
}

/// A task as `sync.get` shows it.
#[derive(Debug, Clone, Serialize)]
pub struct TaskView {
    pub task_id: String,
    pub title: String,
    pub version: u64,
    pub hosts: Vec<String>,
    pub schedule: SyncSchedule,
    pub inputs: Value,
    pub secrets: Vec<SecretView>,
    pub enabled: bool,
    /// `ok` | `failing` | `disabled`.
    pub state: &'static str,
    pub output: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run: Option<Run>,
    pub failures: u32,
}

/// `sync.put`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PutRequest {
    pub task_id: String,
    pub title: String,
    pub program: String,
    pub hosts: Vec<String>,
    pub schedule: SyncSchedule,
    #[serde(default)]
    pub inputs: Option<Value>,
    #[serde(default)]
    pub secrets: Option<Vec<String>>,
}

/// `sync.try`: the stored task, or a draft of it (what is given replaces
/// the stored part; a task not put yet needs `program`).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TryRequest {
    pub task_id: String,
    #[serde(default)]
    pub program: Option<String>,
    #[serde(default)]
    pub hosts: Option<Vec<String>>,
    #[serde(default)]
    pub inputs: Option<Value>,
    #[serde(default)]
    pub secrets: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncField {
    pub field_id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
}

/// What a program printed.
#[derive(Debug, Clone, Deserialize)]
pub struct ProgramResult {
    pub status: String,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub error_kind: Option<FailureKind>,
    #[serde(default)]
    pub business_time: Option<String>,
    #[serde(default)]
    pub fields: Option<Vec<SyncField>>,
    #[serde(default)]
    pub records: Option<Vec<Map<String, Value>>>,
    #[serde(default)]
    pub truncated: Option<bool>,
    #[serde(default)]
    pub state: Option<Value>,
}

/// One execution of a program.
#[derive(Debug, Clone)]
pub struct ProgramRun {
    /// What it printed on stdout, or why there is nothing to read.
    pub stdout: std::result::Result<Vec<u8>, (FailureKind, String)>,
    pub exited_ok: bool,
    /// Its stderr (bounded).
    pub log: String,
}

/// How one attempt ended.
#[derive(Debug, Clone)]
pub enum Attempt {
    Ok(ProgramResult),
    Failed { kind: FailureKind, error: String },
}

/// Where programs run.
#[async_trait]
pub trait SyncRunner: Send + Sync {
    async fn run(&self, program: &Path, hosts: &[String], input: &Value) -> ProgramRun;
}

/// Deno with network access to the declared hosts only: no file access, no
/// environment, no subprocesses.
pub struct DenoSyncRunner {
    pub deno: PathBuf,
    pub timeout: Duration,
}

impl DenoSyncRunner {
    /// The Deno the Goal checks run on.
    pub fn discover() -> Option<Self> {
        crate::goal_check::DenoCheckRunner::discover(std::env::temp_dir()).map(|d| Self {
            deno: d.deno,
            timeout: RUN_TIMEOUT,
        })
    }
}

fn failed(kind: FailureKind, e: impl Into<String>) -> ProgramRun {
    ProgramRun {
        stdout: Err((kind, e.into())),
        exited_ok: false,
        log: String::new(),
    }
}

fn push_log(log: &mut String, line: &str) {
    if log.len() >= LOG_MAX {
        return;
    }
    let mut end = line.len().min(LOG_MAX - log.len());
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    log.push_str(&line[..end]);
    log.push('\n');
}

#[async_trait]
impl SyncRunner for DenoSyncRunner {
    async fn run(&self, program: &Path, hosts: &[String], input: &Value) -> ProgramRun {
        let mut cmd = tokio::process::Command::new(&self.deno);
        cmd.env_clear().env("NO_COLOR", "1");
        for keep in ["DENO_DIR", "SYSTEMROOT"] {
            if let Some(v) = std::env::var_os(keep) {
                cmd.env(keep, v);
            }
        }
        cmd.args(["run", "--quiet", "--no-prompt", "--no-config"]);
        if !hosts.is_empty() {
            cmd.arg(format!("--allow-net={}", hosts.join(",")));
        }
        cmd.arg(program)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return failed(FailureKind::Program, format!("start {}: {e}", self.deno.display())),
        };
        let mut stdin = child.stdin.take().expect("piped stdin");
        let mut stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let input = serde_json::to_vec(input).unwrap_or_default();
        tokio::spawn(async move {
            let _ = stdin.write_all(&input).await;
        });
        let errors = tokio::spawn(async move {
            let mut log = String::new();
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                push_log(&mut log, &l);
            }
            log
        });
        let talk = async {
            let mut out = Vec::new();
            let mut chunk = vec![0u8; 64 * 1024];
            loop {
                let n = stdout.read(&mut chunk).await.map_err(|e| (FailureKind::Program, format!("read the program's output: {e}")))?;
                if n == 0 {
                    break;
                }
                if out.len() + n > OUTPUT_MAX {
                    return Err((FailureKind::Program, format!("the program printed more than {} MiB", OUTPUT_MAX >> 20)));
                }
                out.extend_from_slice(&chunk[..n]);
            }
            let status = child.wait().await.map_err(|e| (FailureKind::Program, format!("wait for the program: {e}")))?;
            Ok((out, status.success()))
        };
        let result = match tokio::time::timeout(self.timeout, talk).await {
            Ok(r) => r,
            Err(_) => Err((FailureKind::Transient, format!("the program did not finish within {} s", self.timeout.as_secs()))),
        };
        let _ = child.start_kill();
        let _ = child.wait().await;
        let log = tokio::time::timeout(Duration::from_secs(2), errors).await.ok().and_then(|r| r.ok()).unwrap_or_default();
        match result {
            Ok((out, ok)) => ProgramRun { stdout: Ok(out), exited_ok: ok, log },
            Err(e) => ProgramRun { stdout: Err(e), exited_ok: false, log },
        }
    }
}

/// No Deno on this host.
pub struct MissingSyncRunner;

#[async_trait]
impl SyncRunner for MissingSyncRunner {
    async fn run(&self, _program: &Path, _hosts: &[String], _input: &Value) -> ProgramRun {
        failed(FailureKind::Program, "no Deno on this host: sync programs cannot run")
    }
}

/// An error the network caused (the program did not catch it).
fn network_failure(log: &str) -> bool {
    ["error sending request", "dns error", "Connection refused", "connection reset", "Connection reset", "tcp connect error"]
        .iter()
        .any(|m| log.contains(m))
}

/// What a program's run comes to.
pub fn judge(run: &ProgramRun) -> Attempt {
    let stdout = match &run.stdout {
        Ok(s) => s,
        Err((kind, e)) => return Attempt::Failed { kind: *kind, error: e.clone() },
    };
    let text = String::from_utf8_lossy(stdout);
    let text = text.trim();
    let parsed = serde_json::from_str::<ProgramResult>(text).or_else(|e| {
        text.lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .filter(|l| l.len() < text.len())
            .map(|l| serde_json::from_str::<ProgramResult>(l.trim()))
            .unwrap_or(Err(e))
    });
    let result = match parsed {
        Ok(r) => r,
        Err(_) if !run.exited_ok => {
            let first = |p: &str| run.log.lines().map(str::trim).find(|l| l.starts_with(p));
            let error = match (first("error"), first("Caused by")) {
                (Some(e), Some(c)) => format!("{e} ({c})"),
                (Some(e), None) => e.to_string(),
                _ => run.log.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or("the program exited with an error").to_string(),
            };
            let kind = if network_failure(&run.log) { FailureKind::Transient } else { FailureKind::Program };
            return Attempt::Failed { kind, error: error.chars().take(2000).collect() };
        }
        Err(_) if text.is_empty() => return Attempt::Failed { kind: FailureKind::Program, error: format!("the program printed nothing: {CONVENTION}") },
        Err(e) => return Attempt::Failed { kind: FailureKind::Program, error: format!("{CONVENTION}: {e}") },
    };
    let bad = |e: String| Attempt::Failed { kind: FailureKind::Program, error: e };
    match result.status.as_str() {
        "failed" => Attempt::Failed {
            kind: result.error_kind.unwrap_or(FailureKind::Program),
            error: result
                .error
                .filter(|e| !e.trim().is_empty())
                .unwrap_or_else(|| "the program reported a failure without saying why".into())
                .chars()
                .take(2000)
                .collect(),
        },
        "ok" => {
            if result.records.is_none() {
                return bad(format!("`records` is missing: {CONVENTION}"));
            }
            if let Some(t) = &result.business_time {
                if chrono::DateTime::parse_from_rfc3339(t).is_err() {
                    return bad(format!("`business_time` {t} is not an RFC 3339 date-time"));
                }
            }
            let size = result.state.as_ref().map(|s| s.to_string().len()).unwrap_or(0);
            if size > STATE_MAX {
                return bad(format!("`state` is {size} bytes, more than {STATE_MAX}"));
            }
            Attempt::Ok(result)
        }
        other => bad(format!("status `{other}`: {CONVENTION}")),
    }
}

/// Where the values of credentials are kept (the Agent's settings in a
/// Zone).
#[async_trait]
pub trait SyncSecrets: Send + Sync {
    async fn values(&self, task_id: &str) -> Result<BTreeMap<String, String>>;
    async fn set(&self, task_id: &str, name: &str, value: Option<String>) -> Result<()>;
    async fn forget(&self, task_id: &str) -> Result<()>;
}

/// In memory (tests, development).
#[derive(Default)]
pub struct MemorySecrets(Mutex<BTreeMap<String, BTreeMap<String, String>>>);

#[async_trait]
impl SyncSecrets for MemorySecrets {
    async fn values(&self, task_id: &str) -> Result<BTreeMap<String, String>> {
        Ok(self.0.lock().expect("secrets").get(task_id).cloned().unwrap_or_default())
    }

    async fn set(&self, task_id: &str, name: &str, value: Option<String>) -> Result<()> {
        let mut all = self.0.lock().expect("secrets");
        let task = all.entry(task_id.to_string()).or_default();
        match value {
            Some(v) => task.insert(name.to_string(), v),
            None => task.remove(name),
        };
        Ok(())
    }

    async fn forget(&self, task_id: &str) -> Result<()> {
        self.0.lock().expect("secrets").remove(task_id);
        Ok(())
    }
}

/// `text` with every credential value replaced by its name.
pub fn mask(text: &str, secrets: &BTreeMap<String, String>) -> String {
    let mut out = text.to_string();
    for (name, value) in secrets {
        if !value.is_empty() {
            out = out.replace(value.as_str(), &format!("[secret {name}]"));
        }
    }
    out
}

fn mask_value(v: &mut Value, secrets: &BTreeMap<String, String>) {
    match v {
        Value::String(s) => *s = mask(s, secrets),
        Value::Array(items) => items.iter_mut().for_each(|i| mask_value(i, secrets)),
        Value::Object(m) => m.values_mut().for_each(|i| mask_value(i, secrets)),
        _ => {}
    }
}

fn invalid(e: impl Into<String>) -> OpenDanError {
    OpenDanError::InvalidArgument(e.into())
}

fn check_task_id(id: &str) -> Result<()> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id.bytes().next().is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_');
    ok.then_some(()).ok_or_else(|| invalid(format!("task_id `{id}`: 1–64 of a-z 0-9 - _, starting with a letter or digit")))
}

/// A host a program may reach: `name[:port]`, not this machine.
fn check_host(host: &str) -> Result<()> {
    let (name, port) = match host.rsplit_once(':') {
        Some((n, p)) => (n, Some(p)),
        None => (host, None),
    };
    let name_ok = !name.is_empty()
        && name.len() <= 253
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
        && !name.starts_with(['.', '-']);
    if !name_ok || port.is_some_and(|p| p.parse::<u16>().map_or(true, |p| p == 0)) {
        return Err(invalid(format!("host `{host}`: a lowercase host name or IPv4 address, optionally :port")));
    }
    if name == "localhost" || name.ends_with(".localhost") || name.starts_with("127.") || name.starts_with("0.") || name == "host.docker.internal" {
        return Err(invalid(format!("host `{host}` is this machine: a sync program reaches only outside hosts")));
    }
    Ok(())
}

fn check_secret_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name.bytes().next().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    ok.then_some(()).ok_or_else(|| invalid(format!("secret name `{name}`: letters, digits and _, not starting with a digit")))
}

fn check_parts(hosts: &[String], inputs: &Value, secrets: &[String]) -> Result<()> {
    if hosts.len() > 32 {
        return Err(invalid("at most 32 hosts"));
    }
    hosts.iter().try_for_each(|h| check_host(h))?;
    if !inputs.is_object() {
        return Err(invalid("`inputs` is a JSON object"));
    }
    if inputs.to_string().len() > INPUTS_MAX {
        return Err(invalid(format!("`inputs` is more than {INPUTS_MAX} bytes")));
    }
    if secrets.len() > 16 {
        return Err(invalid("at most 16 secrets"));
    }
    secrets.iter().try_for_each(|s| check_secret_name(s))
}

fn check_program(program: &str) -> Result<()> {
    if program.trim().is_empty() {
        return Err(invalid("`program` is empty"));
    }
    if program.len() > PROGRAM_MAX {
        return Err(invalid(format!("`program` is more than {PROGRAM_MAX} bytes")));
    }
    Ok(())
}

fn rfc3339(ms: u64) -> String {
    chrono::DateTime::from_timestamp_millis(ms as i64)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// The Zone path of an AgentRoot `<…>/home/<owner>/agents/<agent_id>`:
/// `/home/<owner>/agents/<agent_id>` (else the path as it is).
pub fn zone_root_of(agent_root: &Path) -> String {
    let parts: Vec<&str> = agent_root.iter().filter_map(|p| p.to_str()).collect();
    match parts.as_slice() {
        [.., "home", owner, "agents", agent] => format!("/home/{owner}/agents/{agent}"),
        _ => agent_root.display().to_string(),
    }
}

pub struct SyncHostOptions {
    pub agent_root: PathBuf,
    /// The Zone path of the AgentRoot (where a data source reads outputs).
    pub zone_root: String,
    pub runner: Arc<dyn SyncRunner>,
    pub secrets: Arc<dyn SyncSecrets>,
    /// Where the owner is told about a failing task (`None`: nobody is).
    pub notify: Option<Arc<dyn crate::notify::NotifyHost>>,
    pub clock: Arc<dyn Clock>,
    /// Waits before the retries of a transient failure.
    pub retry_delays: Vec<Duration>,
    /// How often the timetables are looked at.
    pub tick: Duration,
}

impl SyncHostOptions {
    pub fn new(agent_root: PathBuf, secrets: Arc<dyn SyncSecrets>, notify: Option<Arc<dyn crate::notify::NotifyHost>>) -> Self {
        let runner: Arc<dyn SyncRunner> = match DenoSyncRunner::discover() {
            Some(d) => Arc::new(d),
            None => Arc::new(MissingSyncRunner),
        };
        Self {
            zone_root: zone_root_of(&agent_root),
            agent_root,
            runner,
            secrets,
            notify,
            clock: Arc::new(SystemClock),
            retry_delays: vec![Duration::from_secs(30), Duration::from_secs(120)],
            tick: Duration::from_secs(15),
        }
    }
}

struct Inflight {
    run: Run,
    done: tokio::sync::watch::Receiver<Option<Run>>,
}

/// The sync tasks of one Agent.
pub struct SyncHost {
    o: SyncHostOptions,
    /// One writer of a task's files at a time.
    files: tokio::sync::Mutex<()>,
    inflight: Mutex<HashMap<String, Inflight>>,
}

impl SyncHost {
    pub fn new(options: SyncHostOptions) -> Arc<Self> {
        Arc::new(Self {
            o: options,
            files: tokio::sync::Mutex::new(()),
            inflight: Mutex::new(HashMap::new()),
        })
    }

    fn dir(&self, task_id: &str) -> PathBuf {
        self.o.agent_root.join(SYNC_DIR).join(task_id)
    }

    fn now(&self) -> u64 {
        self.o.clock.now_ms()
    }

    fn def_opt(&self, task_id: &str) -> Result<Option<TaskDef>> {
        check_task_id(task_id)?;
        crate::fsutil::read_json_opt(&self.dir(task_id).join("task.json"))
    }

    fn def(&self, task_id: &str) -> Result<TaskDef> {
        self.def_opt(task_id)?
            .ok_or_else(|| OpenDanError::NotFound(format!("sync task {task_id}")))
    }

    fn progress(&self, task_id: &str) -> Result<Progress> {
        Ok(crate::fsutil::read_json_opt(&self.dir(task_id).join("schedule.json"))?.unwrap_or_default())
    }

    fn write_progress(&self, task_id: &str, p: &Progress) -> Result<()> {
        crate::fsutil::atomic_replace_json(&self.dir(task_id).join("schedule.json"), p)
    }

    fn running(&self, task_id: &str) -> Option<Run> {
        self.inflight.lock().expect("inflight").get(task_id).map(|i| i.run.clone())
    }

    async fn view(&self, def: TaskDef) -> Result<TaskView> {
        let progress = self.progress(&def.task_id)?;
        let set = match self.o.secrets.values(&def.task_id).await {
            Ok(v) => v,
            Err(e) => {
                log::warn!("sync: {}: credentials not readable: {e}", def.task_id);
                BTreeMap::new()
            }
        };
        let state = if !def.enabled {
            "disabled"
        } else if progress.failures > 0 {
            "failing"
        } else {
            "ok"
        };
        Ok(TaskView {
            output: json!({ "path": format!("{}/{SYNC_DIR}/{}/out.json", self.o.zone_root, def.task_id) }),
            last_run: self.running(&def.task_id).or(progress.last_run),
            failures: progress.failures,
            secrets: def
                .secrets
                .iter()
                .map(|n| SecretView { name: n.clone(), set: set.get(n).is_some_and(|v| !v.is_empty()) })
                .collect(),
            state,
            task_id: def.task_id,
            title: def.title,
            version: def.version,
            hosts: def.hosts,
            schedule: def.schedule,
            inputs: def.inputs,
            enabled: def.enabled,
        })
    }

    fn defs(&self) -> Result<Vec<TaskDef>> {
        let root = self.o.agent_root.join(SYNC_DIR);
        let entries = match std::fs::read_dir(&root) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(OpenDanError::io(&root, e)),
        };
        let mut out = Vec::new();
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || check_task_id(&name).is_err() {
                continue;
            }
            match self.def_opt(&name) {
                Ok(Some(d)) => out.push(d),
                Ok(None) => {}
                Err(err) => log::warn!("sync: {name}: {err}"),
            }
        }
        out.sort_by(|a, b| a.task_id.cmp(&b.task_id));
        Ok(out)
    }

    pub async fn list(&self) -> Result<Vec<TaskView>> {
        let mut out = Vec::new();
        for d in self.defs()? {
            out.push(self.view(d).await?);
        }
        Ok(out)
    }

    pub async fn get(&self, task_id: &str) -> Result<TaskView> {
        self.view(self.def(task_id)?).await
    }

    /// Install or change a task: changing the program raises its version
    /// (and starts a new count of failures); a new task is enabled and runs
    /// on its timetable from the next slot.
    pub async fn put(&self, req: PutRequest) -> Result<TaskView> {
        check_task_id(&req.task_id)?;
        let title = req.title.trim().to_string();
        if title.is_empty() || title.chars().count() > 200 {
            return Err(invalid("`title`: 1–200 characters"));
        }
        check_program(&req.program)?;
        if req.schedule.every_secs < MIN_EVERY_SECS {
            return Err(invalid(format!("`schedule.every_secs` is at least {MIN_EVERY_SECS}")));
        }
        let old = self.def_opt(&req.task_id)?;
        let inputs = req.inputs.or_else(|| old.as_ref().map(|o| o.inputs.clone())).unwrap_or_else(|| json!({}));
        let secrets = req.secrets.or_else(|| old.as_ref().map(|o| o.secrets.clone())).unwrap_or_default();
        check_parts(&req.hosts, &inputs, &secrets)?;
        let dir = self.dir(&req.task_id);
        let _files = self.files.lock().await;
        let program_path = dir.join("program.ts");
        let changed = std::fs::read_to_string(&program_path).ok().as_deref() != Some(req.program.as_str());
        let version = match &old {
            None => 1,
            Some(o) if changed => o.version + 1,
            Some(o) => o.version,
        };
        let def = TaskDef {
            task_id: req.task_id.clone(),
            title,
            version,
            hosts: req.hosts,
            schedule: req.schedule,
            inputs,
            secrets,
            enabled: old.as_ref().map(|o| o.enabled).unwrap_or(true),
            updated_at: rfc3339(self.now()),
        };
        if changed {
            crate::fsutil::atomic_replace(&program_path, req.program.as_bytes())?;
        }
        crate::fsutil::atomic_replace_json(&dir.join("task.json"), &def)?;
        let mut p = self.progress(&def.task_id)?;
        if changed {
            p.failures = 0;
            p.told = false;
        }
        if p.every_secs != def.schedule.every_secs {
            p.last_slot = None;
        }
        self.write_progress(&def.task_id, &p)?;
        drop(_files);
        if let Some(o) = &old {
            for gone in o.secrets.iter().filter(|n| !def.secrets.contains(n)) {
                self.o.secrets.set(&def.task_id, gone, None).await?;
            }
        }
        log::info!("sync: {} put (version {}, every {} s, hosts {})", def.task_id, def.version, def.schedule.every_secs, def.hosts.join(","));
        self.view(def).await
    }

    pub async fn set_enabled(&self, task_id: &str, enabled: bool) -> Result<TaskView> {
        let _files = self.files.lock().await;
        let mut def = self.def(task_id)?;
        def.enabled = enabled;
        def.updated_at = rfc3339(self.now());
        crate::fsutil::atomic_replace_json(&self.dir(task_id).join("task.json"), &def)?;
        drop(_files);
        self.view(def).await
    }

    pub async fn delete(&self, task_id: &str) -> Result<()> {
        let _files = self.files.lock().await;
        self.def(task_id)?;
        if self.running(task_id).is_some() {
            return Err(OpenDanError::Busy {
                resource: format!("sync task {task_id}: a run is in progress, delete it when it ends"),
                holder: None,
            });
        }
        let dir = self.dir(task_id);
        std::fs::remove_dir_all(&dir).map_err(|e| OpenDanError::io(&dir, e))?;
        drop(_files);
        self.o.secrets.forget(task_id).await?;
        log::info!("sync: {task_id} deleted");
        Ok(())
    }

    /// Set (or clear, `None`) a credential the task declares.
    pub async fn set_secret(&self, task_id: &str, name: &str, value: Option<String>) -> Result<TaskView> {
        let def = self.def(task_id)?;
        if !def.secrets.iter().any(|n| n == name) {
            return Err(invalid(format!("sync task {task_id} declares no secret `{name}` (it declares: {})", def.secrets.join(", "))));
        }
        let value = value.filter(|v| !v.is_empty());
        if value.as_ref().is_some_and(|v| v.len() > 8192) {
            return Err(invalid("a secret is at most 8192 bytes"));
        }
        let set = value.is_some();
        self.o.secrets.set(task_id, name, value).await?;
        log::info!("sync: {task_id}: secret {name} {}", if set { "set" } else { "cleared" });
        self.view(def).await
    }

    /// The runs, newest first (the one in progress on top).
    pub async fn runs(&self, task_id: &str, limit: usize) -> Result<Vec<Run>> {
        self.def(task_id)?;
        let mut out: Vec<Run> = self.running(task_id).into_iter().collect();
        out.extend(self.recorded_runs(task_id)?.into_iter().rev());
        out.truncate(limit.clamp(1, RUNS_KEEP));
        Ok(out)
    }

    fn recorded_runs(&self, task_id: &str) -> Result<Vec<Run>> {
        let path = self.dir(task_id).join("runs.jsonl");
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(OpenDanError::io(&path, e)),
        };
        Ok(text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect())
    }

    fn record_run(&self, task_id: &str, run: &Run) -> Result<()> {
        let path = self.dir(task_id).join("runs.jsonl");
        let mut line = serde_json::to_vec(run).map_err(|e| OpenDanError::json(&path, e))?;
        line.push(b'\n');
        crate::fsutil::append_batch(&path, &line)?;
        let runs = self.recorded_runs(task_id)?;
        if runs.len() > 2 * RUNS_KEEP {
            let keep = crate::fsutil::to_json_lines(&runs[runs.len() - RUNS_KEEP..])?;
            crate::fsutil::atomic_replace(&path, &keep)?;
        }
        Ok(())
    }

    /// Run the task now (or join the run in progress) and wait up to
    /// `wait` for it to end.
    pub async fn run(self: &Arc<Self>, task_id: &str, trigger: &str, wait: Duration) -> Result<Run> {
        let def = self.def(task_id)?;
        if !def.enabled {
            return Err(invalid(format!("sync task {task_id} is disabled")));
        }
        let (run, mut done) = self.start(def, trigger);
        if wait.is_zero() {
            return Ok(run);
        }
        let end = match tokio::time::timeout(wait, done.wait_for(Option::is_some)).await {
            Ok(Ok(end)) => end.clone(),
            _ => None,
        };
        Ok(end.unwrap_or(run))
    }

    fn start(self: &Arc<Self>, def: TaskDef, trigger: &str) -> (Run, tokio::sync::watch::Receiver<Option<Run>>) {
        let mut inflight = self.inflight.lock().expect("inflight");
        if let Some(i) = inflight.get(&def.task_id) {
            return (i.run.clone(), i.done.clone());
        }
        let run = Run {
            run_id: format!("sr-{}", &uuid::Uuid::new_v4().simple().to_string()[..16]),
            at: rfc3339(self.now()),
            ended_at: None,
            status: RunStatus::Running,
            kind: None,
            error: None,
            records: None,
            trigger: trigger.to_string(),
        };
        let (tx, rx) = tokio::sync::watch::channel(None);
        inflight.insert(def.task_id.clone(), Inflight { run: run.clone(), done: rx.clone() });
        let host = self.clone();
        let started = run.clone();
        tokio::spawn(async move {
            let task_id = def.task_id.clone();
            let end = host.execute(def, started).await;
            host.inflight.lock().expect("inflight").remove(&task_id);
            let _ = tx.send(Some(end));
        });
        (run, rx)
    }

    fn input(&self, def: &TaskDef, secrets: &BTreeMap<String, String>) -> Value {
        let state: Value = crate::fsutil::read_json_opt(&self.dir(&def.task_id).join("state.json"))
            .ok()
            .flatten()
            .unwrap_or(Value::Null);
        json!({ "task_id": def.task_id, "inputs": def.inputs, "secrets": secrets, "state": state })
    }

    /// The credentials of `def`, or why the program cannot run.
    async fn credentials(&self, def: &TaskDef) -> std::result::Result<BTreeMap<String, String>, Attempt> {
        let all = self.o.secrets.values(&def.task_id).await.map_err(|e| Attempt::Failed {
            kind: FailureKind::Transient,
            error: format!("the credentials could not be read: {e}"),
        })?;
        if let Some(missing) = def.secrets.iter().find(|n| all.get(*n).is_none_or(|v| v.is_empty())) {
            return Err(Attempt::Failed {
                kind: FailureKind::Auth,
                error: format!("the credential `{missing}` is not set: the owner enters it in the data source's properties in the AI Workspace"),
            });
        }
        Ok(def.secrets.iter().filter_map(|n| all.get(n).map(|v| (n.clone(), v.clone()))).collect())
    }

    async fn attempt(&self, def: &TaskDef, program: &Path, secrets: &BTreeMap<String, String>) -> (Attempt, String) {
        let run = self.o.runner.run(program, &def.hosts, &self.input(def, secrets)).await;
        (judge(&run), mask(&run.log, secrets))
    }

    async fn execute(&self, def: TaskDef, run: Run) -> Run {
        let (attempt, secrets) = match self.credentials(&def).await {
            Err(a) => (a, BTreeMap::new()),
            Ok(secrets) => {
                let program = self.dir(&def.task_id).join("program.ts");
                let mut delays = self.o.retry_delays.iter();
                let attempt = loop {
                    let (a, log) = self.attempt(&def, &program, &secrets).await;
                    if !log.is_empty() {
                        log::debug!("sync: {} run {}: {log}", def.task_id, run.run_id);
                    }
                    match (&a, delays.next()) {
                        (Attempt::Failed { kind: FailureKind::Transient, error }, Some(d)) => {
                            log::info!("sync: {} run {}: {}; again in {} s", def.task_id, run.run_id, mask(error, &secrets), d.as_secs());
                            tokio::time::sleep(*d).await;
                        }
                        _ => break a,
                    }
                };
                (attempt, secrets)
            }
        };
        self.finish(&def, run, attempt, &secrets).await
    }

    /// Write the envelope (and the program's state), record the run, count
    /// the failure and tell the owner when it is time.
    async fn finish(&self, def: &TaskDef, mut run: Run, attempt: Attempt, secrets: &BTreeMap<String, String>) -> Run {
        let now = self.now();
        run.ended_at = Some(rfc3339(now));
        let dir = self.dir(&def.task_id);
        let mut envelope = json!({
            "format": "aiws.sync.v1",
            "program": { "name": def.task_id, "version": def.version.to_string() },
            "produced_at": rfc3339(now),
        });
        let mut state = None;
        match attempt {
            Attempt::Ok(r) => {
                run.status = RunStatus::Succeeded;
                run.records = r.records.as_ref().map(Vec::len);
                envelope["status"] = json!("ok");
                envelope["records"] = json!(r.records.unwrap_or_default());
                if let Some(t) = r.business_time {
                    envelope["business_time"] = json!(t);
                }
                if let Some(f) = r.fields {
                    envelope["fields"] = json!(f);
                }
                if let Some(t) = r.truncated {
                    envelope["truncated"] = json!(t);
                }
                state = r.state;
            }
            Attempt::Failed { kind, error } => {
                let error = mask(&error, secrets);
                run.status = RunStatus::Failed;
                run.kind = Some(kind);
                run.error = Some(error.clone());
                envelope["status"] = json!("failed");
                envelope["error"] = json!(error);
            }
        }
        let written = serde_json::to_vec(&envelope)
            .map_err(|e| OpenDanError::json(dir.join("out.json"), e))
            .and_then(|b| crate::fsutil::atomic_replace(&dir.join("out.json"), &b))
            .and_then(|_| match &state {
                Some(s) => crate::fsutil::atomic_replace_json(&dir.join("state.json"), s),
                None => Ok(()),
            });
        if let Err(e) = written {
            run.status = RunStatus::Failed;
            run.kind = Some(FailureKind::Transient);
            run.error = Some(format!("the output could not be written: {e}"));
        }
        let tell = {
            let _files = self.files.lock().await;
            if let Err(e) = self.record_run(&def.task_id, &run) {
                log::warn!("sync: {}: run {} not recorded: {e}", def.task_id, run.run_id);
            }
            let mut p = self.progress(&def.task_id).unwrap_or_default();
            if run.status == RunStatus::Succeeded {
                p.failures = 0;
                p.told = false;
            } else {
                p.failures = p.failures.saturating_add(1);
            }
            let tell = !p.told
                && match run.kind {
                    Some(FailureKind::Auth) => true,
                    Some(FailureKind::Program) => p.failures >= PROGRAM_FAILURES_TOLD,
                    _ => false,
                };
            p.last_run = Some(run.clone());
            if let Err(e) = self.write_progress(&def.task_id, &p) {
                log::warn!("sync: {}: progress not written: {e}", def.task_id);
            }
            tell.then_some(p.failures)
        };
        match run.status {
            RunStatus::Succeeded => log::info!("sync: {} run {} ({}) succeeded: {} records", def.task_id, run.run_id, run.trigger, run.records.unwrap_or(0)),
            _ => log::warn!("sync: {} run {} ({}) failed: {}", def.task_id, run.run_id, run.trigger, run.error.as_deref().unwrap_or("")),
        }
        if let Some(failures) = tell {
            if self.tell_owner(def, &run, failures).await {
                let _files = self.files.lock().await;
                if let Ok(mut p) = self.progress(&def.task_id) {
                    p.told = true;
                    let _ = self.write_progress(&def.task_id, &p);
                }
            }
        }
        run
    }

    /// Tell the owner a task needs them: a credential (they enter it), or
    /// a program that keeps failing (they hand it to the Agent to repair).
    async fn tell_owner(&self, def: &TaskDef, run: &Run, failures: u32) -> bool {
        let Some(notify) = &self.o.notify else {
            return false;
        };
        let error = run.error.clone().unwrap_or_default();
        let (title, message) = if run.kind == Some(FailureKind::Auth) {
            (
                format!("Sync “{}” needs a credential", def.title),
                format!("{error}\nOnly you can enter it: open the data source's properties in the AI Workspace. The data source keeps its last good data until then."),
            )
        } else {
            (
                format!("Sync “{}” keeps failing", def.title),
                format!("It failed {failures} times in a row: {error}\nAsk your Agent to repair sync task {} (manual aiworkspace_sync). The data source keeps its last good data.", def.task_id),
            )
        };
        let mut req = crate::notify::NotifySendReq::notice(String::new(), title.chars().take(200).collect::<String>(), message, format!("sync:{}:{}", def.task_id, run.run_id));
        req.source = Some(crate::notify::NotifySource {
            operation: Some(format!("sync task {}", def.task_id)),
            ..Default::default()
        });
        match notify.send(req).await {
            Ok(v) => {
                log::info!("sync: {}: owner told ({})", def.task_id, v.notify_id);
                true
            }
            Err(e) => {
                log::warn!("sync: {}: owner not told: {e}", def.task_id);
                false
            }
        }
    }

    /// Run the task (or a draft of it) once, without retries: nothing is
    /// written or recorded.
    pub async fn try_run(&self, req: TryRequest) -> Result<Value> {
        let base = self.def_opt(&req.task_id)?;
        let draft = TaskDef {
            task_id: req.task_id.clone(),
            title: base.as_ref().map(|b| b.title.clone()).unwrap_or_else(|| req.task_id.clone()),
            version: base.as_ref().map(|b| b.version).unwrap_or(0),
            hosts: req.hosts.clone().or_else(|| base.as_ref().map(|b| b.hosts.clone())).unwrap_or_default(),
            schedule: base.as_ref().map(|b| b.schedule.clone()).unwrap_or(SyncSchedule { every_secs: MIN_EVERY_SECS }),
            inputs: req.inputs.clone().or_else(|| base.as_ref().map(|b| b.inputs.clone())).unwrap_or_else(|| json!({})),
            secrets: req.secrets.clone().or_else(|| base.as_ref().map(|b| b.secrets.clone())).unwrap_or_default(),
            enabled: true,
            updated_at: rfc3339(self.now()),
        };
        check_parts(&draft.hosts, &draft.inputs, &draft.secrets)?;
        let run_id = format!("st-{}", &uuid::Uuid::new_v4().simple().to_string()[..16]);
        let (program, scratch) = match &req.program {
            Some(text) => {
                check_program(text)?;
                let p = self.o.agent_root.join(SYNC_DIR).join(".try").join(format!("{}-{run_id}.ts", req.task_id));
                crate::fsutil::atomic_replace(&p, text.as_bytes())?;
                (p.clone(), Some(p))
            }
            None if base.is_some() => (self.dir(&req.task_id).join("program.ts"), None),
            None => return Err(invalid(format!("sync task {} is not put yet: give the `program` to try", req.task_id))),
        };
        let at = rfc3339(self.now());
        let (attempt, log, secrets) = match self.credentials(&draft).await {
            Err(a) => (a, String::new(), BTreeMap::new()),
            Ok(secrets) => {
                let (a, log) = self.attempt(&draft, &program, &secrets).await;
                (a, log, secrets)
            }
        };
        if let Some(p) = scratch {
            let _ = std::fs::remove_file(p);
        }
        let mut run = Run {
            run_id,
            at,
            ended_at: Some(rfc3339(self.now())),
            status: RunStatus::Succeeded,
            kind: None,
            error: None,
            records: None,
            trigger: "try".into(),
        };
        let preview = match attempt {
            Attempt::Ok(r) => {
                let records = r.records.unwrap_or_default();
                run.records = Some(records.len());
                let mut sample = json!(records.iter().take(SAMPLE).collect::<Vec<_>>());
                mask_value(&mut sample, &secrets);
                json!({
                    "status": "ok",
                    "records": records.len(),
                    "sample": sample,
                    "fields": r.fields,
                    "business_time": r.business_time,
                    "truncated": r.truncated,
                    "log": log,
                })
            }
            Attempt::Failed { kind, error } => {
                let error = mask(&error, &secrets);
                run.status = RunStatus::Failed;
                run.kind = Some(kind);
                run.error = Some(error.clone());
                json!({ "status": "failed", "records": 0, "sample": [], "error": error, "kind": kind, "log": log })
            }
        };
        Ok(json!({ "run": run, "preview": preview }))
    }

    /// Start the runs whose slot came (the slots missed while the host was
    /// down are one run).
    pub async fn tick(self: &Arc<Self>) -> Result<()> {
        let now = self.now();
        for def in self.defs()? {
            if !def.enabled {
                continue;
            }
            let t = Timetable::Every(def.schedule.every_secs.max(MIN_EVERY_SECS));
            let Some(slot) = t.slot(now) else { continue };
            let fire = {
                let _files = self.files.lock().await;
                let mut p = self.progress(&def.task_id)?;
                let fire = match p.last_slot {
                    Some(last) if p.every_secs == def.schedule.every_secs => slot > last,
                    _ => false,
                };
                if p.last_slot != Some(slot) || p.every_secs != def.schedule.every_secs {
                    p.every_secs = def.schedule.every_secs;
                    p.last_slot = Some(slot);
                    self.write_progress(&def.task_id, &p)?;
                }
                fire
            };
            if fire && self.running(&def.task_id).is_none() {
                self.start(def, "schedule");
            }
        }
        Ok(())
    }

    /// Follow the timetables until the task is aborted.
    pub async fn serve(self: Arc<Self>) {
        loop {
            if let Err(e) = self.tick().await {
                log::warn!("sync: {e}");
            }
            tokio::time::sleep(self.o.tick).await;
        }
    }
}

fn arg<T: serde::de::DeserializeOwned>(params: &Value, name: &str) -> Result<T> {
    serde_json::from_value(params.get(name).cloned().unwrap_or(Value::Null))
        .map_err(|e| invalid(format!("param `{name}`: {e}")))
}

fn out<T: Serialize>(v: T) -> Result<Value> {
    serde_json::to_value(v).map_err(|e| OpenDanError::Other(format!("encode result: {e}")))
}

/// One `sync.*` call of the Agent Service (authorized by the service).
/// `trigger`: who runs a task with `sync.run` (`manual`, `aiworkspace`).
pub async fn call(host: &Arc<SyncHost>, method: &str, params: &Value, trigger: &str) -> Result<Value> {
    let task_id = || arg::<String>(params, "task_id");
    match method {
        "sync.list" => Ok(json!({ "tasks": host.list().await? })),
        "sync.get" => out(host.get(&task_id()?).await?),
        "sync.put" => out(host.put(serde_json::from_value(params.clone()).map_err(|e| invalid(format!("sync.put: {e}")))?).await?),
        "sync.set_enabled" => out(host.set_enabled(&task_id()?, arg(params, "enabled")?).await?),
        "sync.delete" => {
            host.delete(&task_id()?).await?;
            Ok(json!({}))
        }
        "sync.set_secret" => out(host.set_secret(&task_id()?, &arg::<String>(params, "name")?, arg(params, "value")?).await?),
        "sync.try" => host.try_run(serde_json::from_value(params.clone()).map_err(|e| invalid(format!("sync.try: {e}")))?).await,
        "sync.run" => {
            let wait = arg::<Option<u64>>(params, "wait_ms")?.unwrap_or(0).min(WAIT_MAX_MS);
            Ok(json!({ "run": host.run(&task_id()?, trigger, Duration::from_millis(wait)).await? }))
        }
        "sync.runs" => {
            let limit = arg::<Option<usize>>(params, "limit")?.unwrap_or(20);
            Ok(json!({ "runs": host.runs(&task_id()?, limit).await? }))
        }
        other => Err(invalid(format!("unknown method `{other}`"))),
    }
}
