//! Jobs of a Goal (`state/goals/<goal_id>/jobs/`): programs the host runs on
//! a timetable, as the agent, without an inference — the routine a Goal
//! would otherwise wake up for. A check script observes; a job does things.
//!
//! ```text
//! jobs/
//!   usage.jsonl            model tokens the jobs of the Goal used (UsageRecord, run_id = the job run's)
//!   <job_id>/
//!     job.json             definition                                  (GoalStore, under the goal's lock)
//!     versions/<n>/        the program as it was installed, never changed
//!     data/                the job's own directory, its working directory; an upgrade leaves it
//!     schedule.json        progress                                    (host)
//!     request.json         "run it now", taken by the host
//!     runs.jsonl           the runs                                    (host)
//!     logs/<run_id>.log    the end of what a run printed               (host)
//! ```
//!
//! A job lives with its Goal: a dormant or ended Goal runs none, one that
//! was running is ended. Nothing here infers or tells the user anything: the
//! check script reads the runs (`ctx.jobs`) and decides.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::AsyncReadExt;

use crate::error::{OpenDanError, Result};
use crate::fsutil;
use crate::goal_time::Timetable;
use crate::host::goals::Clock;
use crate::protocol::*;
use crate::skills::SkillFile;
use crate::state::AgentStateClient;

pub const GOAL_JOB_SCHEMA: &str = "opendan.goal_job/1";
/// Shortest timetable of a job.
pub const JOB_MIN_EVERY_SECS: u64 = 300;
/// How long a run may take: the default, and the most (a lent session token
/// holds no longer).
pub const JOB_TIMEOUT_SECS: u64 = 300;
pub const JOB_TIMEOUT_MAX_SECS: u64 = 600;
pub const JOB_PROGRAM_MAX: usize = 20 * 1024 * 1024;
pub const JOBS_PER_GOAL: usize = 64;
/// Jobs of one agent that run at the same time.
pub const JOBS_AT_ONCE: usize = 2;
pub const JOB_VERSIONS_KEEP: usize = 5;
pub const JOB_RUNS_KEEP: usize = 200;
pub const JOB_LOGS_KEEP: usize = 20;
pub const JOB_LOG_MAX: usize = 64 * 1024;
/// Pauses before a run that failed for a passing reason is tried again.
pub const JOB_RETRY_SECS: [u64; 2] = [30, 120];
/// A job's usage records are kept this long.
const USAGE_KEEP_MS: u64 = 35 * 86_400_000;
const USAGE_TRIM_BYTES: u64 = 2 * 1024 * 1024;

/// Environment of a run, besides the agent's.
pub const ENV_GOAL_ID: &str = "OPENDAN_GOAL_ID";
pub const ENV_JOB_ID: &str = "OPENDAN_JOB_ID";
pub const ENV_JOB_RUN_ID: &str = "OPENDAN_JOB_RUN_ID";
pub const ENV_JOB_DIR: &str = "OPENDAN_JOB_DIR";
/// Where a model client of the run records its tokens, and how many are left
/// (`0`: none) — read by `xllm`.
pub const ENV_USAGE_LOG: &str = agent_tool::xllm::ENV_USAGE_LOG;
pub const ENV_MODEL_BUDGET: &str = agent_tool::xllm::ENV_MODEL_BUDGET;

/// `job.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GoalJob {
    pub schema: String,
    pub goal_id: String,
    pub job_id: String,
    pub title: String,
    /// The installed program (`versions/<version>/`).
    pub version: u64,
    /// The program and its arguments; `{dir}` is the directory of the
    /// installed version.
    pub command: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cron: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    pub timeout_secs: u64,
    pub enabled: bool,
    pub updated_at_ms: u64,
    /// `owner | ui_session:<sid> | work_session:<sid> | goal_session:<sid>`.
    pub updated_by: String,
}

impl GoalJob {
    pub fn timetable(&self) -> Option<Timetable> {
        Timetable::new(self.every_secs, self.cron.as_deref(), self.timezone.as_deref())
    }
}

/// What installs a job, or a new version of one.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobPut {
    pub job_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub command: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cron: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    /// The program's files (relative paths).
    pub files: BTreeMap<String, SkillFile>,
    /// Those that are executable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub executable: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JobRunStatus {
    Ok,
    Failed,
    Timeout,
    /// The host ended it (its Goal went dormant or ended, the job was
    /// switched off, the host stopped or died).
    Killed,
}

impl JobRunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            JobRunStatus::Ok => "ok",
            JobRunStatus::Failed => "failed",
            JobRunStatus::Timeout => "timeout",
            JobRunStatus::Killed => "killed",
        }
    }
}

/// One line of `runs.jsonl`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct JobRun {
    pub run_id: String,
    pub version: u64,
    pub at_ms: u64,
    pub ended_at_ms: u64,
    pub status: JobRunStatus,
    /// Why it failed: `transient | auth | program | budget`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default)]
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metrics: Option<Value>,
    /// `schedule | install | manual`.
    pub trigger: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Attempts it took (a passing failure is tried again).
    #[serde(default = "one")]
    pub attempts: u32,
    /// Model tokens it used.
    #[serde(default)]
    pub tokens: u64,
    /// The request that asked for it (`manual`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

fn one() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct JobRunning {
    pub run_id: String,
    pub version: u64,
    pub at_ms: u64,
    pub trigger: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
}

/// `schedule.json` of a job.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct JobSchedule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timetable: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_slot: Option<u64>,
    /// The version that ran (or began to): a newer one runs at once.
    #[serde(default)]
    pub version_run: u64,
    /// Runs in a row that did not succeed.
    #[serde(default)]
    pub failures: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub running: Option<JobRunning>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobRequest {
    pub request_id: String,
    pub by: String,
    pub at_ms: u64,
}

/// A job as it is shown (`goals.jobs`, `ctx.jobs`, `<jobs>`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobView {
    #[serde(flatten)]
    pub job: GoalJob,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub running: Option<JobRunning>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run: Option<JobRun>,
    pub failures: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_at_ms: Option<u64>,
    /// Versions that can be gone back to.
    #[serde(default)]
    pub versions: Vec<u64>,
}

fn invalid(e: impl Into<String>) -> OpenDanError {
    OpenDanError::InvalidArgument(e.into())
}

fn check_job_id(id: &str) -> Result<()> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_'))
        && id.starts_with(|c: char| c.is_ascii_alphanumeric());
    if ok {
        Ok(())
    } else {
        Err(invalid(format!("bad job id `{id}`: lower case letters, digits, `-` and `_`, at most 64")))
    }
}

/// The jobs of one Goal on the AgentRoot.
#[derive(Debug, Clone)]
pub struct JobFiles {
    dir: PathBuf,
    goal_id: String,
}

impl JobFiles {
    /// `goal_dir`: `state/goals/<goal_id>`.
    pub fn new(goal_dir: &Path, goal_id: &str) -> Self {
        Self { dir: goal_dir.join("jobs"), goal_id: goal_id.to_string() }
    }

    pub fn of(agent_root: &Path, goal_id: &str) -> Self {
        Self::new(&crate::state::AgentLayout::new(agent_root).goals_dir().join(goal_id), goal_id)
    }

    fn job_dir(&self, job_id: &str) -> PathBuf {
        self.dir.join(job_id)
    }

    fn file(&self, job_id: &str, name: &str) -> PathBuf {
        self.job_dir(job_id).join(name)
    }

    pub fn version_dir(&self, job_id: &str, version: u64) -> PathBuf {
        self.job_dir(job_id).join("versions").join(version.to_string())
    }

    pub fn data_dir(&self, job_id: &str) -> PathBuf {
        self.job_dir(job_id).join("data")
    }

    pub fn usage_file(&self) -> PathBuf {
        self.dir.join("usage.jsonl")
    }

    pub fn ids(&self) -> Vec<String> {
        let mut out: Vec<String> = std::fs::read_dir(&self.dir)
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| e.file_name().to_str().map(str::to_string))
                    .filter(|n| check_job_id(n).is_ok() && self.file(n, "job.json").is_file())
                    .collect()
            })
            .unwrap_or_default();
        out.sort();
        out
    }

    pub fn get(&self, job_id: &str) -> Result<Option<GoalJob>> {
        check_job_id(job_id)?;
        fsutil::read_json_opt(&self.file(job_id, "job.json"))
    }

    pub fn must(&self, job_id: &str) -> Result<GoalJob> {
        self.get(job_id)?
            .ok_or_else(|| OpenDanError::NotFound(format!("job {job_id} of goal {}", self.goal_id)))
    }

    /// The jobs (one whose `job.json` is damaged is left out, with a
    /// warning).
    pub fn list(&self) -> Vec<GoalJob> {
        self.ids()
            .iter()
            .filter_map(|id| match self.get(id) {
                Ok(j) => j,
                Err(e) => {
                    log::warn!("goal {}: job {id}: {e}", self.goal_id);
                    None
                }
            })
            .collect()
    }

    pub fn schedule(&self, job_id: &str) -> Result<JobSchedule> {
        Ok(fsutil::read_json_opt(&self.file(job_id, "schedule.json"))?.unwrap_or_default())
    }

    fn write_schedule(&self, job_id: &str, s: &JobSchedule) -> Result<()> {
        fsutil::atomic_replace_json(&self.file(job_id, "schedule.json"), s)
    }

    fn versions(&self, job_id: &str) -> Vec<u64> {
        let mut v: Vec<u64> = std::fs::read_dir(self.job_dir(job_id).join("versions"))
            .map(|rd| rd.flatten().filter_map(|e| e.file_name().to_str()?.parse().ok()).collect())
            .unwrap_or_default();
        v.sort_unstable();
        v
    }

    /// The newest `limit` runs, oldest first (`0`: all that are kept).
    pub fn runs(&self, job_id: &str, limit: usize) -> Result<Vec<JobRun>> {
        check_job_id(job_id)?;
        crate::state::goals::last_lines(&self.file(job_id, "runs.jsonl"), limit)
    }

    /// The end of what a run printed (`None`: the newest one kept).
    pub fn log(&self, job_id: &str, run_id: Option<&str>) -> Result<Option<String>> {
        check_job_id(job_id)?;
        let run_id = match run_id {
            Some(r) => r.to_string(),
            None => match self.runs(job_id, 1)?.pop() {
                Some(r) => r.run_id,
                None => return Ok(None),
            },
        };
        crate::ids::validate_session_id(&run_id).map_err(|_| invalid(format!("bad run id `{run_id}`")))?;
        match std::fs::read(self.job_dir(job_id).join("logs").join(format!("{run_id}.log"))) {
            Ok(b) => Ok(Some(String::from_utf8_lossy(&b).into_owned())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(OpenDanError::io(&self.job_dir(job_id), e)),
        }
    }

    pub fn view(&self, job: GoalJob, now_ms: u64) -> Result<JobView> {
        let s = self.schedule(&job.job_id)?;
        let next_at_ms = job.timetable().filter(|_| job.enabled).and_then(|t| t.next_ms(now_ms));
        Ok(JobView {
            last_run: self.runs(&job.job_id, 1)?.pop(),
            running: s.running,
            failures: s.failures,
            next_at_ms,
            versions: self.versions(&job.job_id),
            job,
        })
    }

    pub fn views(&self, now_ms: u64) -> Result<Vec<JobView>> {
        self.list().into_iter().map(|j| self.view(j, now_ms)).collect()
    }

    /// Model tokens the jobs used since `since_ms`.
    pub fn usage_since(&self, since_ms: u64) -> Vec<UsageRecord> {
        let mut out: Vec<UsageRecord> = crate::state::goals::last_lines(&self.usage_file(), 0).unwrap_or_default();
        out.retain(|r| r.at_ms >= since_ms);
        out
    }

    fn run_tokens(&self, run_id: &str, since_ms: u64) -> u64 {
        self.usage_since(since_ms)
            .iter()
            .filter(|r| r.run_id.as_deref() == Some(run_id))
            .map(|r| r.total_tokens)
            .sum()
    }

    // ---- changes: the caller holds the goal's lock ----

    /// Install a job or a new version of one. The running program is not
    /// touched: the new one starts as soon as the job may run.
    pub fn put(&self, req: &JobPut, by: &str, now_ms: u64) -> Result<GoalJob> {
        check_job_id(&req.job_id)?;
        if req.command.first().is_none_or(|c| c.trim().is_empty()) {
            return Err(invalid("a job needs a command"));
        }
        if req.files.is_empty() {
            return Err(invalid("a job needs a program directory with at least one file"));
        }
        let timeout = req.timeout_secs.unwrap_or(JOB_TIMEOUT_SECS);
        if timeout == 0 || timeout > JOB_TIMEOUT_MAX_SECS {
            return Err(invalid(format!("a run takes 1–{JOB_TIMEOUT_MAX_SECS} seconds at most (timeout_secs)")));
        }
        match (req.every_secs, &req.cron) {
            (Some(e), None) if e < JOB_MIN_EVERY_SECS => {
                return Err(invalid(format!("every_secs is at least {JOB_MIN_EVERY_SECS}")))
            }
            (None, None) => return Err(invalid("a job needs a timetable: every_secs or cron")),
            _ => {}
        }
        crate::goal_time::validate_timetable(req.every_secs, req.cron.as_deref(), req.timezone.as_deref()).map_err(invalid)?;
        let old = self.get(&req.job_id)?;
        if old.is_none() && self.ids().len() >= JOBS_PER_GOAL {
            return Err(invalid(format!("a goal has at most {JOBS_PER_GOAL} jobs")));
        }
        let mut contents = Vec::new();
        let mut size = 0;
        for (path, f) in &req.files {
            let plain = !path.is_empty()
                && !path.starts_with('/')
                && !path.contains(['\\', ':'])
                && path.split('/').all(|p| !p.is_empty() && p != "." && p != "..");
            if !plain {
                return Err(invalid(format!("bad file path `{path}` in the program directory")));
            }
            let bytes = f.bytes()?;
            size += bytes.len();
            if size > JOB_PROGRAM_MAX {
                return Err(invalid(format!("the program directory is larger than {} MiB", JOB_PROGRAM_MAX / 1024 / 1024)));
            }
            contents.push((path, bytes));
        }
        let versions = self.versions(&req.job_id);
        let version = versions.last().copied().unwrap_or(0) + 1;
        let vdir = self.version_dir(&req.job_id, version);
        let tmp = self.job_dir(&req.job_id).join("versions").join(format!(".new-{version}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        for (path, bytes) in contents {
            let at = tmp.join(path);
            fsutil::atomic_replace(&at, &bytes)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = if req.executable.contains(path) { 0o755 } else { 0o644 };
                std::fs::set_permissions(&at, std::fs::Permissions::from_mode(mode)).map_err(|e| OpenDanError::io(&at, e))?;
            }
        }
        if !fsutil::publish_dir(&tmp, &vdir)? {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(invalid(format!("version {version} of job {} exists already", req.job_id)));
        }
        let data = self.data_dir(&req.job_id);
        std::fs::create_dir_all(&data).map_err(|e| OpenDanError::io(&data, e))?;
        let job = GoalJob {
            schema: GOAL_JOB_SCHEMA.to_string(),
            goal_id: self.goal_id.clone(),
            job_id: req.job_id.clone(),
            title: req
                .title
                .clone()
                .filter(|t| !t.trim().is_empty())
                .or_else(|| old.as_ref().map(|o| o.title.clone()))
                .unwrap_or_else(|| req.job_id.clone())
                .chars()
                .take(200)
                .collect(),
            version,
            command: req.command.clone(),
            every_secs: req.every_secs,
            cron: req.cron.clone(),
            timezone: req.timezone.clone(),
            timeout_secs: timeout,
            enabled: old.as_ref().map(|o| o.enabled).unwrap_or(true),
            updated_at_ms: now_ms,
            updated_by: by.to_string(),
        };
        fsutil::atomic_replace_json(&self.file(&job.job_id, "job.json"), &job)?;
        // The oldest versions go; the one that may be running stays.
        let running = self.schedule(&job.job_id)?.running.map(|r| r.version);
        let keep = versions.len().saturating_sub(JOB_VERSIONS_KEEP - 1);
        for v in versions.iter().take(keep).filter(|v| Some(**v) != running) {
            let _ = std::fs::remove_dir_all(self.version_dir(&job.job_id, *v));
        }
        Ok(job)
    }

    fn change(&self, job_id: &str, by: &str, now_ms: u64, f: impl FnOnce(&mut GoalJob) -> Result<()>) -> Result<GoalJob> {
        let mut job = self.must(job_id)?;
        f(&mut job)?;
        job.updated_at_ms = now_ms;
        job.updated_by = by.to_string();
        fsutil::atomic_replace_json(&self.file(job_id, "job.json"), &job)?;
        Ok(job)
    }

    pub fn set_enabled(&self, job_id: &str, enabled: bool, by: &str, now_ms: u64) -> Result<GoalJob> {
        self.change(job_id, by, now_ms, |j| {
            j.enabled = enabled;
            Ok(())
        })
    }

    /// Back to an earlier version (`None`: the one before).
    pub fn rollback(&self, job_id: &str, to: Option<u64>, by: &str, now_ms: u64) -> Result<GoalJob> {
        let versions = self.versions(job_id);
        self.change(job_id, by, now_ms, |j| {
            let to = match to {
                Some(v) => v,
                None => *versions
                    .iter()
                    .rfind(|v| **v < j.version)
                    .ok_or_else(|| invalid(format!("job {job_id} has no version before {}", j.version)))?,
            };
            if !versions.contains(&to) {
                return Err(invalid(format!("job {job_id} has no version {to} (kept: {versions:?})")));
            }
            j.version = to;
            Ok(())
        })
    }

    pub fn delete(&self, job_id: &str) -> Result<()> {
        self.must(job_id)?;
        let dir = self.job_dir(job_id);
        // Out of sight first: a run that ends writes nothing back.
        let gone = self.dir.join(format!(".gone-{job_id}-{}", crate::now_ms()));
        std::fs::rename(&dir, &gone).map_err(|e| OpenDanError::io(&dir, e))?;
        std::fs::remove_dir_all(&gone).map_err(|e| OpenDanError::io(&gone, e))
    }

    /// Ask the host to run it now.
    pub fn request(&self, job_id: &str, by: &str, now_ms: u64) -> Result<JobRequest> {
        let job = self.must(job_id)?;
        if !job.enabled {
            return Err(invalid(format!("job {job_id} is switched off (`goal job enable {job_id}`)")));
        }
        let req = JobRequest {
            request_id: uuid::Uuid::new_v4().simple().to_string(),
            by: by.to_string(),
            at_ms: now_ms,
        };
        fsutil::atomic_replace_json(&self.file(job_id, "request.json"), &req)?;
        Ok(req)
    }

    fn take_request(&self, job_id: &str) -> Option<JobRequest> {
        let path = self.file(job_id, "request.json");
        let req = fsutil::read_json_opt(&path).ok().flatten();
        let _ = std::fs::remove_file(&path);
        req
    }

    // ---- the host ----

    fn begin(&self, job: &GoalJob, running: &JobRunning, slot: Option<u64>) -> Result<()> {
        let mut s = self.schedule(&job.job_id)?;
        s.running = Some(running.clone());
        s.version_run = job.version;
        if let Some(t) = job.timetable() {
            s.timetable = Some(t.key());
            s.last_slot = slot.or(s.last_slot);
        }
        self.write_schedule(&job.job_id, &s)
    }

    /// Record the end of a run (`redo`: the job is due again at once).
    fn finish(&self, job_id: &str, run: &JobRun, log: Option<&[u8]>, redo: bool) -> Result<()> {
        if self.get(job_id)?.is_none() {
            return Ok(());
        }
        let path = self.file(job_id, "runs.jsonl");
        fsutil::append_batch(&path, &fsutil::to_json_lines(std::slice::from_ref(run))?)?;
        let runs: Vec<JobRun> = crate::state::goals::last_lines(&path, 0)?;
        if runs.len() > 2 * JOB_RUNS_KEEP {
            let kept = fsutil::to_json_lines(&runs[runs.len() - JOB_RUNS_KEEP..])?;
            fsutil::atomic_replace(&path, &kept)?;
        }
        if let Some(log) = log {
            let dir = self.job_dir(job_id).join("logs");
            fsutil::atomic_replace(&dir.join(format!("{}.log", run.run_id)), log)?;
            let mut logs: Vec<PathBuf> = std::fs::read_dir(&dir).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default();
            logs.sort();
            let extra = logs.len().saturating_sub(JOB_LOGS_KEEP);
            for old in &logs[..extra] {
                let _ = std::fs::remove_file(old);
            }
        }
        let mut s = self.schedule(job_id)?;
        s.running = None;
        match run.status {
            JobRunStatus::Ok => s.failures = 0,
            JobRunStatus::Failed | JobRunStatus::Timeout => s.failures = s.failures.saturating_add(1),
            JobRunStatus::Killed => {}
        }
        if redo {
            s.version_run = 0;
        }
        self.write_schedule(job_id, &s)?;
        self.trim_usage(run.ended_at_ms)
    }

    fn trim_usage(&self, now_ms: u64) -> Result<()> {
        let path = self.usage_file();
        if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) <= USAGE_TRIM_BYTES {
            return Ok(());
        }
        let kept = self.usage_since(now_ms.saturating_sub(USAGE_KEEP_MS));
        fsutil::atomic_replace(&path, &fsutil::to_json_lines(&kept)?)
    }

    /// Why `job` runs now, if it does, and the slot that takes it there.
    fn due(&self, job: &GoalJob, now_ms: u64) -> Result<Option<(&'static str, Option<u64>)>> {
        if !job.enabled {
            return Ok(None);
        }
        let mut s = self.schedule(&job.job_id)?;
        let Some(t) = job.timetable() else {
            return Ok(None);
        };
        let slot = t.slot(now_ms);
        if self.file(&job.job_id, "request.json").is_file() {
            return Ok(Some(("manual", slot)));
        }
        if s.version_run != job.version {
            return Ok(Some(("install", slot)));
        }
        if s.timetable.as_deref() != Some(&t.key()) {
            // A changed timetable counts from the current slot.
            s.timetable = Some(t.key());
            s.last_slot = slot;
            self.write_schedule(&job.job_id, &s)?;
            return Ok(None);
        }
        Ok(match (slot, s.last_slot) {
            (Some(slot), Some(last)) if slot > last => Some(("schedule", Some(slot))),
            (Some(slot), None) => Some(("schedule", Some(slot))),
            _ => None,
        })
    }
}

/// What a run of this host left of a run that was going on when it stopped.
fn end_orphan(running: &JobRunning) {
    #[cfg(target_os = "linux")]
    if let Some(pid) = running.pid {
        let mark = format!("{ENV_JOB_RUN_ID}={}", running.run_id);
        let mine = std::fs::read(format!("/proc/{pid}/environ"))
            .is_ok_and(|env| env.split(|b| *b == 0).any(|v| v == mark.as_bytes()));
        if mine {
            unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = running;
}

/// The end of everything a run prints, at most [`JOB_LOG_MAX`] bytes, and the
/// last line of its stdout.
#[derive(Default)]
struct Output {
    tail: Vec<u8>,
    line: Vec<u8>,
    last_line: Vec<u8>,
}

impl Output {
    fn push(&mut self, bytes: &[u8], stdout: bool) {
        self.tail.extend_from_slice(bytes);
        let extra = self.tail.len().saturating_sub(JOB_LOG_MAX);
        self.tail.drain(..extra);
        if !stdout {
            return;
        }
        for b in bytes {
            if *b == b'\n' {
                if !self.line.iter().all(u8::is_ascii_whitespace) {
                    self.last_line = std::mem::take(&mut self.line);
                }
                self.line.clear();
            } else if self.line.len() < JOB_LOG_MAX {
                self.line.push(*b);
            }
        }
    }

    fn last(&self) -> &[u8] {
        if self.line.iter().all(u8::is_ascii_whitespace) { &self.last_line } else { &self.line }
    }
}

async fn pump(mut from: impl tokio::io::AsyncRead + Unpin, out: Arc<Mutex<Output>>, stdout: bool) {
    let mut buf = vec![0u8; 8192];
    while let Ok(n) = from.read(&mut buf).await {
        if n == 0 {
            break;
        }
        out.lock().expect("job output").push(&buf[..n], stdout);
    }
}

/// What the last line of a run's stdout may say.
#[derive(Debug, Default, Deserialize)]
struct JobReport {
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    metrics: Option<Value>,
}

/// One attempt: how it ended.
struct Attempt {
    status: JobRunStatus,
    exit_code: Option<i32>,
    report: JobReport,
    error: Option<String>,
}

struct Live {
    stop: tokio::sync::watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

/// What a run is given.
#[derive(Debug, Clone)]
pub struct JobEnv {
    pub agent_did: String,
    pub agent_root: PathBuf,
    /// Model tokens the run may use (`None`: no limit).
    pub model_budget: Option<u64>,
}

/// Runs the jobs of an agent's Goals.
pub struct JobHost {
    clock: Arc<dyn Clock>,
    /// Pauses between the attempts of a run.
    retry: Vec<Duration>,
    live: Mutex<HashMap<(String, String), Live>>,
    /// Goals whose unfinished runs of an earlier host were looked at.
    recovered: Mutex<std::collections::BTreeSet<String>>,
}

impl JobHost {
    pub fn new(clock: Arc<dyn Clock>, retry: Vec<Duration>) -> Arc<Self> {
        Arc::new(Self {
            clock,
            retry,
            live: Mutex::new(HashMap::new()),
            recovered: Mutex::new(Default::default()),
        })
    }

    fn reap(&self) {
        self.live.lock().expect("live jobs").retain(|_, l| !l.task.is_finished());
    }

    fn is_live(&self, goal_id: &str, job_id: &str) -> bool {
        self.live.lock().expect("live jobs").contains_key(&(goal_id.to_string(), job_id.to_string()))
    }

    /// A run an earlier host did not see the end of.
    fn recover(&self, files: &JobFiles, now_ms: u64) {
        if !self.recovered.lock().expect("recovered").insert(files.goal_id.clone()) {
            return;
        }
        for job in files.list() {
            let Ok(Some(r)) = files.schedule(&job.job_id).map(|s| s.running) else {
                continue;
            };
            end_orphan(&r);
            let run = JobRun {
                run_id: r.run_id.clone(),
                version: r.version,
                at_ms: r.at_ms,
                ended_at_ms: now_ms,
                status: JobRunStatus::Killed,
                kind: None,
                summary: "the host stopped while it ran".into(),
                metrics: None,
                trigger: r.trigger.clone(),
                exit_code: None,
                attempts: 1,
                tokens: files.run_tokens(&r.run_id, r.at_ms),
                request_id: None,
            };
            if let Err(e) = files.finish(&job.job_id, &run, None, true) {
                log::warn!("goal {}: job {}: {e}", files.goal_id, job.job_id);
            }
        }
    }

    /// End the running jobs of a Goal (it sleeps, it ended).
    pub fn stop_goal(&self, goal_id: &str) {
        for ((g, _), l) in self.live.lock().expect("live jobs").iter() {
            if g == goal_id {
                let _ = l.stop.send(true);
            }
        }
    }

    fn stop_job(&self, goal_id: &str, job_id: &str) {
        if let Some(l) = self.live.lock().expect("live jobs").get(&(goal_id.to_string(), job_id.to_string())) {
            let _ = l.stop.send(true);
        }
    }

    /// The jobs of a live Goal that are due, when there is room for them.
    pub fn due(&self, files: &JobFiles) -> Vec<(GoalJob, &'static str, Option<u64>)> {
        self.reap();
        let now = self.clock.now_ms();
        self.recover(files, now);
        let mut out = Vec::new();
        let jobs = files.list();
        // A job that was switched off or removed while it ran is ended.
        let ids: Vec<String> = {
            let live = self.live.lock().expect("live jobs");
            live.keys().filter(|(g, _)| *g == files.goal_id).map(|(_, j)| j.clone()).collect()
        };
        for id in ids {
            if !jobs.iter().any(|j| j.job_id == id && j.enabled) {
                self.stop_job(&files.goal_id, &id);
            }
        }
        for job in jobs {
            if self.is_live(&files.goal_id, &job.job_id) {
                continue;
            }
            match files.due(&job, now) {
                Ok(Some((trigger, slot))) => out.push((job, trigger, slot)),
                Ok(None) => {}
                Err(e) => log::warn!("goal {}: job {}: {e}", files.goal_id, job.job_id),
            }
        }
        let room = JOBS_AT_ONCE.saturating_sub(self.live.lock().expect("live jobs").len());
        out.truncate(room);
        out
    }

    /// Start a run.
    pub fn start(self: &Arc<Self>, files: &JobFiles, job: GoalJob, trigger: &str, slot: Option<u64>, env: JobEnv) -> Result<String> {
        let now = self.clock.now_ms();
        let request = files.take_request(&job.job_id);
        let trigger = if request.is_some() { "manual" } else { trigger };
        let run_id = format!(
            "{}-{}",
            crate::runner::input_view::utc_rfc3339(now).chars().filter(|c| c.is_ascii_digit()).take(14).collect::<String>(),
            &uuid::Uuid::new_v4().simple().to_string()[..6]
        );
        let running = JobRunning { run_id: run_id.clone(), version: job.version, at_ms: now, trigger: trigger.to_string(), pid: None };
        files.begin(&job, &running, slot)?;
        let (stop, stopped) = tokio::sync::watch::channel(false);
        let (host, files, id) = (self.clone(), files.clone(), run_id.clone());
        let key = (files.goal_id.clone(), job.job_id.clone());
        let task = tokio::spawn(async move {
            host.run(files, job, running, request, env, stopped).await;
        });
        self.live.lock().expect("live jobs").insert(key, Live { stop, task });
        Ok(id)
    }

    async fn run(&self, files: JobFiles, job: GoalJob, mut running: JobRunning, request: Option<JobRequest>, env: JobEnv, mut stop: tokio::sync::watch::Receiver<bool>) {
        log::info!("goal {}: job {} v{} runs ({})", files.goal_id, job.job_id, job.version, running.trigger);
        let output = Arc::new(Mutex::new(Output::default()));
        let mut attempts = 0;
        let end = loop {
            attempts += 1;
            let end = self.attempt(&files, &job, &mut running, &env, &output, &mut stop).await;
            let again = end.status == JobRunStatus::Failed && end.report.kind.as_deref() == Some("transient");
            let Some(pause) = self.retry.get(attempts as usize - 1).copied().filter(|_| again) else {
                break end;
            };
            output.lock().expect("job output").push(format!("\n[tried again after {}s]\n", pause.as_secs()).as_bytes(), false);
            tokio::select! {
                _ = tokio::time::sleep(pause) => {}
                _ = stopped(&mut stop) => {
                    break Attempt { status: JobRunStatus::Killed, exit_code: None, report: JobReport::default(), error: None };
                }
            }
        };
        let tokens = files.run_tokens(&running.run_id, running.at_ms);
        let spent = env.model_budget.is_some_and(|b| tokens >= b);
        let kind = match end.status {
            JobRunStatus::Ok => None,
            _ => end.report.kind.clone().or_else(|| {
                (end.status == JobRunStatus::Failed).then(|| if spent { "budget".into() } else { "program".to_string() })
            }),
        };
        let summary = end.report.summary.clone().or(end.error).unwrap_or_else(|| match end.status {
            JobRunStatus::Ok => String::new(),
            JobRunStatus::Timeout => format!("did not finish within {}s", job.timeout_secs),
            JobRunStatus::Killed => "ended by the host".into(),
            JobRunStatus::Failed => format!("exit {}", end.exit_code.map(|c| c.to_string()).unwrap_or_else(|| "by a signal".into())),
        });
        let run = JobRun {
            run_id: running.run_id.clone(),
            version: job.version,
            at_ms: running.at_ms,
            ended_at_ms: self.clock.now_ms(),
            status: end.status,
            kind,
            summary: summary.chars().take(2000).collect(),
            metrics: end.report.metrics,
            trigger: running.trigger.clone(),
            exit_code: end.exit_code,
            attempts,
            tokens,
            request_id: request.map(|r| r.request_id),
        };
        let log = output.lock().expect("job output").tail.clone();
        // What was cut short because its Goal sleeps runs when it wakes.
        let redo = run.status == JobRunStatus::Killed;
        if let Err(e) = files.finish(&job.job_id, &run, Some(&log), redo) {
            log::warn!("goal {}: job {}: {e}", files.goal_id, job.job_id);
        }
        log::info!("goal {}: job {} {} ({})", files.goal_id, job.job_id, run.status.as_str(), run.summary);
    }

    async fn attempt(&self, files: &JobFiles, job: &GoalJob, running: &mut JobRunning, env: &JobEnv, output: &Arc<Mutex<Output>>, stop: &mut tokio::sync::watch::Receiver<bool>) -> Attempt {
        let failed = |kind: &str, error: String| Attempt {
            status: JobRunStatus::Failed,
            exit_code: None,
            report: JobReport { kind: Some(kind.into()), ..Default::default() },
            error: Some(error),
        };
        if *stop.borrow() {
            return Attempt { status: JobRunStatus::Killed, exit_code: None, report: JobReport::default(), error: None };
        }
        let dir = files.version_dir(&job.job_id, job.version);
        let data = files.data_dir(&job.job_id);
        let mut vars = vec![
            ("OPENDAN_AGENT_DID".to_string(), env.agent_did.clone()),
            ("OPENDAN_AGENT_ROOT".to_string(), env.agent_root.display().to_string()),
            ("OPENDAN_RUNTIME_ID".to_string(), crate::protocol::PRIVATE_RUNTIME.to_string()),
            (ENV_GOAL_ID.to_string(), files.goal_id.clone()),
            (ENV_JOB_ID.to_string(), job.job_id.clone()),
            (ENV_JOB_RUN_ID.to_string(), running.run_id.clone()),
            (ENV_JOB_DIR.to_string(), dir.display().to_string()),
            (ENV_USAGE_LOG.to_string(), files.usage_file().display().to_string()),
        ];
        if let Some(left) = env.model_budget {
            vars.push((ENV_MODEL_BUDGET.to_string(), left.to_string()));
        }
        if let Ok(token) = std::env::var(agent_tool::BUCKYOS_APPCLIENT_SESSION_TOKEN_ENV) {
            vars.push((agent_tool::BUCKYOS_APPCLIENT_SESSION_TOKEN_ENV.to_string(), token));
        }
        if let Err(e) = agent_tool::runtime_context::refresh_appclient_session_env(&mut vars).await {
            return failed("transient", format!("no session token for the run: {e}"));
        }
        let mut path: Vec<PathBuf> = vec![env.agent_root.join("tools")];
        path.extend(crate::baseline::managed_tool_dirs());
        path.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
        let word = |w: &str| w.replace("{dir}", &dir.display().to_string()).replace("$OPENDAN_JOB_DIR", &dir.display().to_string());
        let mut words = job.command.iter().map(|w| word(w));
        let program = words.next().unwrap_or_default();
        // A program of the installed version by its name, else one on PATH.
        let own = dir.join(&program);
        let program = if own.is_file() { own.into_os_string() } else { program.into() };
        let mut cmd = tokio::process::Command::new(&program);
        cmd.args(words)
            .current_dir(&data)
            .envs(vars)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        for gone in ["OPENDAN_SESSION_ID", "OPENDAN_SESSION_DIR", "OPENDAN_INPUT_QUEUE", "OPENDAN_TRACE_ID"] {
            cmd.env_remove(gone);
        }
        if let Ok(joined) = std::env::join_paths(path) {
            cmd.env("PATH", joined);
        }
        #[cfg(unix)]
        cmd.process_group(0);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return failed("program", format!("start {}: {e}", program.to_string_lossy())),
        };
        let pid = child.id();
        running.pid = pid;
        if let Ok(mut s) = files.schedule(&job.job_id) {
            s.running = Some(running.clone());
            let _ = files.write_schedule(&job.job_id, &s);
        }
        let pumps = [
            tokio::spawn(pump(child.stdout.take().expect("piped stdout"), output.clone(), true)),
            tokio::spawn(pump(child.stderr.take().expect("piped stderr"), output.clone(), false)),
        ];
        let ended = tokio::select! {
            r = child.wait() => Some(r),
            _ = tokio::time::sleep(Duration::from_secs(job.timeout_secs)) => None,
            _ = stopped(stop) => {
                kill_tree(pid, &mut child).await;
                return Attempt { status: JobRunStatus::Killed, exit_code: None, report: JobReport::default(), error: None };
            }
        };
        // What the program left behind ends with it.
        kill_tree(pid, &mut child).await;
        for p in pumps {
            let _ = tokio::time::timeout(Duration::from_secs(5), p).await;
        }
        let report: JobReport = {
            let out = output.lock().expect("job output");
            serde_json::from_slice(out.last()).unwrap_or_default()
        };
        match ended {
            None => Attempt { status: JobRunStatus::Timeout, exit_code: None, report, error: None },
            Some(Err(e)) => failed("program", format!("wait: {e}")),
            Some(Ok(status)) => Attempt {
                status: if status.success() { JobRunStatus::Ok } else { JobRunStatus::Failed },
                exit_code: status.code(),
                report,
                error: None,
            },
        }
    }

    /// End every run (the host stops); each is recorded and runs again.
    pub async fn shutdown(&self) {
        let live: Vec<Live> = self.live.lock().expect("live jobs").drain().map(|(_, l)| l).collect();
        for l in &live {
            let _ = l.stop.send(true);
        }
        for l in live {
            let _ = tokio::time::timeout(Duration::from_secs(10), l.task).await;
        }
    }

    /// Wait until nothing runs (tests).
    pub async fn settled(&self) {
        loop {
            self.reap();
            if self.live.lock().expect("live jobs").is_empty() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

async fn stopped(stop: &mut tokio::sync::watch::Receiver<bool>) {
    let _ = stop.wait_for(|s| *s).await;
}

async fn kill_tree(pid: Option<u32>, child: &mut tokio::process::Child) {
    #[cfg(unix)]
    if let Some(pid) = pid {
        unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
    }
    #[cfg(windows)]
    if let Some(pid) = pid {
        let _ = tokio::process::Command::new("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).output().await;
    }
    let _ = child.start_kill();
    let _ = child.wait().await;
}

/// A program directory as the files of a [`JobPut`].
pub fn read_program_dir(dir: &Path) -> Result<(BTreeMap<String, SkillFile>, Vec<String>)> {
    fn walk(base: &Path, dir: &Path, files: &mut BTreeMap<String, SkillFile>, exec: &mut Vec<String>, size: &mut u64) -> Result<()> {
        for e in std::fs::read_dir(dir).map_err(|e| OpenDanError::io(dir, e))?.flatten() {
            let path = e.path();
            let kind = e.file_type().map_err(|err| OpenDanError::io(&path, err))?;
            if kind.is_symlink() {
                return Err(invalid(format!("{}: a program directory holds plain files, not links", path.display())));
            }
            if kind.is_dir() {
                walk(base, &path, files, exec, size)?;
                continue;
            }
            let meta = e.metadata().map_err(|err| OpenDanError::io(&path, err))?;
            *size += meta.len();
            if *size > JOB_PROGRAM_MAX as u64 {
                return Err(invalid(format!("the program directory is larger than {} MiB", JOB_PROGRAM_MAX / 1024 / 1024)));
            }
            let rel = path
                .strip_prefix(base)
                .map_err(|e| invalid(e.to_string()))?
                .components()
                .map(|c| c.as_os_str().to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .join("/");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if meta.permissions().mode() & 0o111 != 0 {
                    exec.push(rel.clone());
                }
            }
            let bytes = std::fs::read(&path).map_err(|err| OpenDanError::io(&path, err))?;
            files.insert(rel, SkillFile::of(&bytes));
        }
        Ok(())
    }
    let (mut files, mut exec, mut size) = (BTreeMap::new(), Vec::new(), 0);
    walk(dir, dir, &mut files, &mut exec, &mut size)?;
    exec.sort();
    Ok((files, exec))
}

/// The words of a command line: quotes group, a backslash takes the next
/// character as it is; nothing is expanded.
pub fn split_command(line: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut word: Option<String> = None;
    let mut quote: Option<char> = None;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('\''), c) => word.get_or_insert_default().push(c),
            (_, '\\') => word.get_or_insert_default().push(chars.next().ok_or_else(|| invalid("the command ends with a backslash"))?),
            (Some(_), c) => word.get_or_insert_default().push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                word.get_or_insert_default();
            }
            (None, c) if c.is_whitespace() => out.extend(word.take()),
            (None, c) => word.get_or_insert_default().push(c),
        }
    }
    if quote.is_some() {
        return Err(invalid("a quote of the command is not closed"));
    }
    out.extend(word);
    Ok(out)
}

/// Model tokens a run started now may use: what the Goal's period and the
/// agent's total have left (`None`: neither has a limit).
pub async fn model_budget(agent: &dyn AgentStateClient, goal: &GoalDefinition, d: &crate::template::GoalDefaults, now_ms: u64) -> Result<Option<u64>> {
    let budget = crate::goal::effective_budget(goal, d);
    let state = agent.goals().state(&goal.goal_id).await?;
    let usage = crate::goal::period_usage(agent, &goal.goal_id, state.as_ref(), now_ms, crate::goal::window_secs(&budget)).await?;
    let total = crate::goal::agent_usage(agent, d, now_ms).await?;
    let own = budget.period.max_total_tokens.map(|m| m.saturating_sub(usage.total_tokens()));
    let agent_left = total.max.map(|m| m.saturating_sub(total.used));
    Ok(match (own, agent_left) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    })
}
