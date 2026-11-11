//! The check of a Goal (architecture §12.1.1, TODO H3): a TypeScript
//! function the host runs on Deno before a scheduled run opportunity becomes
//! an inference. It observes and never acts: it may read files and the
//! network, never run a command or write a file; it spends no token and
//! leaves no Activation record.
//!
//! The script's default export is `check(ctx, state)` returning `{run,
//! reason?, state?}`: `reason` becomes the reason of the Activation it lets
//! through, `state` is kept by the host and given back to the next check.
//! `ctx.host.call(method, params)` reaches the read-only calls of
//! [`CheckCalls`] (`memory.status`).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::protocol::*;

/// How long one check may take.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
/// What a check may print (its log) and the size of its `state`.
pub const CHECK_OUTPUT_MAX: usize = 64 * 1024;
pub const CHECK_STATE_MAX: usize = 64 * 1024;
/// The largest script `set_check` accepts.
pub const CHECK_SCRIPT_MAX: usize = 256 * 1024;

const MARK: &str = "\u{1}opendan-check ";
const RUNNER_TS: &str = include_str!("goal_check_runner.ts");

/// What a check decided.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckVerdict {
    pub run: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// `None`: the script returned none (the stored one stays).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<Value>,
}

/// One check run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CheckRun {
    /// The verdict, or why there is none (thrown, timed out, a result that
    /// does not follow the convention, no Deno).
    pub verdict: std::result::Result<CheckVerdict, String>,
    /// What the script printed (bounded).
    pub log: String,
    pub elapsed_ms: u64,
}

impl CheckRun {
    pub fn outcome(&self) -> CheckOutcome {
        match &self.verdict {
            Ok(v) if v.run => CheckOutcome::True,
            Ok(_) => CheckOutcome::False,
            Err(_) => CheckOutcome::Error,
        }
    }

    fn failed(e: impl Into<String>, started: Instant) -> Self {
        Self {
            verdict: Err(e.into()),
            log: String::new(),
            elapsed_ms: started.elapsed().as_millis() as u64,
        }
    }
}

/// The read-only calls a check may make through `ctx.host.call`.
#[async_trait]
pub trait CheckCalls: Send + Sync {
    async fn call(&self, method: &str, params: &Value) -> std::result::Result<Value, String>;
}

/// No calls (a check that only reads files or the network).
pub struct NoCalls;

#[async_trait]
impl CheckCalls for NoCalls {
    async fn call(&self, method: &str, _params: &Value) -> std::result::Result<Value, String> {
        Err(format!("`{method}` is not available here"))
    }
}

/// The calls of an agent's own host: `memory.status {full?}` (Memory of the
/// AgentRoot, read only).
pub struct AgentCalls {
    pub agent_root: Option<PathBuf>,
}

#[async_trait]
impl CheckCalls for AgentCalls {
    async fn call(&self, method: &str, params: &Value) -> std::result::Result<Value, String> {
        match method {
            "memory.status" => {
                let root = self.agent_root.clone().ok_or("no AgentRoot here: memory.status is not available")?;
                let full = params.get("full").and_then(Value::as_bool).unwrap_or(false);
                let status = crate::host::maintenance::memory_status(&root, full).await.map_err(|e| e.to_string())?;
                serde_json::to_value(status).map_err(|e| e.to_string())
            }
            other => Err(format!("unknown call `{other}` (memory.status)")),
        }
    }
}

/// Where checks run.
#[async_trait]
pub trait CheckRunner: Send + Sync {
    async fn run(&self, script: &Path, ctx: &Value, state: &Value, calls: &dyn CheckCalls) -> CheckRun;
}

/// Deno with read, network and environment only (no `--allow-run`, no
/// `--allow-write`): the standard Tool's Deno and its `deno.json` (the
/// `buckyos` import of the websdk) when the host has them, else `deno` on
/// `PATH` (development).
pub struct DenoCheckRunner {
    pub deno: PathBuf,
    pub config: Option<PathBuf>,
    pub timeout: Duration,
    /// Where the runner script is written.
    pub work_dir: PathBuf,
}

fn on_path(name: &str) -> Option<PathBuf> {
    let exe = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(&exe))
        .find(|p| p.is_file())
}

impl DenoCheckRunner {
    /// The Deno of this host and the `deno.json` of the standard Tool, in
    /// this order: the agent image's (`$BUCKYOS_ROOT/bin/opendan/deno`, the
    /// one its `buckyos-agent` commands run on), a host install's
    /// (`$BUCKYOS_ROOT/libexec/buckyos-tool/runtime/deno`), one beside this
    /// program, else `deno` on `PATH` (development). `work_dir`: a directory
    /// the host may write.
    pub fn discover(work_dir: PathBuf) -> Option<Self> {
        let exe = if cfg!(windows) { "deno.exe" } else { "deno" };
        let mut candidates: Vec<(PathBuf, PathBuf)> = Vec::new();
        if let Some(root) = crate::baseline::buckyos_root() {
            let image = root.join("bin/opendan");
            candidates.push((image.join(exe), image.join("buckyos-agent/deno.json")));
            candidates.push((
                root.join("libexec/buckyos-tool/runtime").join(exe),
                root.join(crate::baseline::AGENT_TOOLS_LIBEXEC).join("deno.json"),
            ));
        }
        if let Some(dir) = crate::baseline::beside_current_exe() {
            candidates.push((dir.join(exe), dir.join("buckyos-agent/deno.json")));
        }
        for (deno, config) in candidates {
            if deno.is_file() {
                return Some(Self {
                    deno,
                    config: config.is_file().then_some(config),
                    timeout: CHECK_TIMEOUT,
                    work_dir,
                });
            }
        }
        let home_deno = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".deno/bin/deno"));
        let deno = on_path("deno").or(home_deno.filter(|p| p.is_file()))?;
        Some(Self {
            deno,
            config: None,
            timeout: CHECK_TIMEOUT,
            work_dir,
        })
    }

    fn runner_script(&self) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(&self.work_dir)?;
        let path = self.work_dir.join("goal_check_runner.ts");
        if std::fs::read_to_string(&path).ok().as_deref() != Some(RUNNER_TS) {
            crate::fsutil::atomic_replace(&path, RUNNER_TS.as_bytes())
                .map_err(|e| std::io::Error::other(e.to_string()))?;
        }
        Ok(path)
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Line {
    Call {
        id: u64,
        method: String,
        #[serde(default)]
        params: Value,
    },
    Result {
        value: CheckVerdict,
    },
    Error {
        message: String,
    },
}

fn push_log(log: &mut String, line: &str) {
    // Deno's own banner for the Zone's self-signed certificates is no log of
    // the check.
    if log.len() >= CHECK_OUTPUT_MAX || line.contains("TLS certificate validation is disabled") {
        return;
    }
    let room = CHECK_OUTPUT_MAX - log.len();
    let mut end = line.len().min(room);
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    log.push_str(&line[..end]);
    log.push('\n');
}

#[async_trait]
impl CheckRunner for DenoCheckRunner {
    async fn run(&self, script: &Path, ctx: &Value, state: &Value, calls: &dyn CheckCalls) -> CheckRun {
        let started = Instant::now();
        let runner = match self.runner_script() {
            Ok(p) => p,
            Err(e) => return CheckRun::failed(format!("cannot write the check runner: {e}"), started),
        };
        let mut cmd = tokio::process::Command::new(&self.deno);
        cmd.arg("run").arg("--quiet").arg("--no-prompt");
        if let Some(c) = &self.config {
            cmd.arg("--config").arg(c);
        } else {
            cmd.arg("--no-config");
        }
        cmd.args(["--allow-read", "--allow-net", "--allow-env", "--unsafely-ignore-certificate-errors"])
            .arg(&runner)
            .arg(script)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return CheckRun::failed(format!("start {}: {e}", self.deno.display()), started),
        };
        let mut stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let errors = tokio::spawn(async move {
            let mut log = String::new();
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                push_log(&mut log, &l);
            }
            log
        });
        let mut log = String::new();
        let talk = async {
            let first = json!({ "ctx": ctx, "state": state });
            stdin
                .write_all(format!("{first}\n").as_bytes())
                .await
                .map_err(|e| format!("write the check's input: {e}"))?;
            let mut lines = BufReader::new(stdout).lines();
            loop {
                let Some(l) = lines.next_line().await.map_err(|e| format!("read the check's output: {e}"))? else {
                    return Err("the check ended without a result".to_string());
                };
                let Some(body) = l.strip_prefix(MARK) else {
                    push_log(&mut log, &l);
                    continue;
                };
                match serde_json::from_str::<Line>(body) {
                    Ok(Line::Call { id, method, params }) => {
                        let answer = match calls.call(&method, &params).await {
                            Ok(v) => json!({ "id": id, "value": v }),
                            Err(e) => json!({ "id": id, "error": e }),
                        };
                        stdin
                            .write_all(format!("{answer}\n").as_bytes())
                            .await
                            .map_err(|e| format!("answer call {method}: {e}"))?;
                    }
                    Ok(Line::Result { value }) => return Ok(value),
                    Ok(Line::Error { message }) => return Err(message),
                    Err(e) => return Err(format!("a check returns {{run: boolean, reason?: string, state?: any}}: {e}")),
                }
            }
        };
        let verdict = match tokio::time::timeout(self.timeout, talk).await {
            Ok(v) => v,
            Err(_) => Err(format!("the check did not finish within {}s", self.timeout.as_secs())),
        };
        let _ = child.start_kill();
        let _ = child.wait().await;
        let stderr_log = errors.await.unwrap_or_default();
        if !stderr_log.is_empty() {
            push_log(&mut log, stderr_log.trim_end());
        }
        let verdict = verdict.and_then(|v| {
            let size = v.state.as_ref().map(|s| s.to_string().len()).unwrap_or(0);
            if size > CHECK_STATE_MAX {
                Err(format!("the check's state is {size} bytes, more than {CHECK_STATE_MAX}"))
            } else {
                Ok(v)
            }
        });
        CheckRun {
            verdict,
            log,
            elapsed_ms: started.elapsed().as_millis() as u64,
        }
    }
}

/// A runner that answers with a function (tests).
pub struct FnCheckRunner<F>(pub F);

#[async_trait]
impl<F> CheckRunner for FnCheckRunner<F>
where
    F: Fn(&Path, &Value, &Value) -> std::result::Result<CheckVerdict, String> + Send + Sync,
{
    async fn run(&self, script: &Path, ctx: &Value, state: &Value, _calls: &dyn CheckCalls) -> CheckRun {
        CheckRun {
            verdict: (self.0)(script, ctx, state),
            log: String::new(),
            elapsed_ms: 0,
        }
    }
}

/// No runner on this host: every check is an error (the hard guarantee
/// still wakes the Goal).
pub struct MissingRunner;

#[async_trait]
impl CheckRunner for MissingRunner {
    async fn run(&self, _script: &Path, _ctx: &Value, _state: &Value, _calls: &dyn CheckCalls) -> CheckRun {
        CheckRun {
            verdict: Err("no Deno on this host: checks cannot run".into()),
            log: String::new(),
            elapsed_ms: 0,
        }
    }
}

/// The `ctx` a check gets (without `host`, which the runner adds).
pub fn check_ctx(
    goal: &GoalDefinition,
    state: Option<&GoalState>,
    schedule: Option<&GoalSchedule>,
    agent_root: Option<&Path>,
    trigger: &str,
    now_ms: u64,
) -> Value {
    json!({
        "goal_id": goal.goal_id,
        "title": goal.title,
        "objective": goal.objective,
        "epoch": goal.session.epoch,
        "scope": goal.scope,
        "agent_root": agent_root.map(|p| p.display().to_string()),
        "trigger": trigger,
        "now_ms": now_ms,
        "last_inference_at_ms": crate::goal::last_inference_ms(state),
        "last_check_at_ms": schedule.and_then(|s| s.last_check.as_ref()).map(|c| c.at_ms),
        "last_check": schedule.and_then(|s| s.last_check.clone()),
        "pending": state.map(|s| s.pending.clone()).unwrap_or_default(),
        "active_works": state.map(|s| s.active_works.iter().map(|w| w.sid.clone()).collect::<Vec<_>>()).unwrap_or_default(),
    })
}

/// Run a Goal's check the way the host does: its current script (or
/// `script`, a draft), its `ctx` and the `state` the host keeps. Nothing is
/// recorded (the host records what it runs; `goal check run` is a dry run).
pub async fn run_for(
    agent: &dyn crate::state::AgentStateClient,
    runner: &dyn CheckRunner,
    goal: &GoalDefinition,
    script: Option<&Path>,
    trigger: &str,
    now_ms: u64,
) -> crate::error::Result<CheckRun> {
    let goals = agent.goals();
    let state = goals.state(&goal.goal_id).await?;
    let schedule = goals.schedule(&goal.goal_id).await?;
    let ctx = check_ctx(goal, state.as_ref(), schedule.as_ref(), agent.agent_root(), trigger, now_ms);
    let stored = schedule.as_ref().and_then(|s| s.check_state.clone()).unwrap_or(Value::Null);
    let path = match script {
        Some(p) => p.to_path_buf(),
        None => {
            let Some(c) = &goal.schedule.check else {
                return Ok(CheckRun {
                    verdict: Ok(CheckVerdict { run: true, reason: None, state: None }),
                    log: "no check script: every scheduled check passes".into(),
                    elapsed_ms: 0,
                });
            };
            match agent
                .agent_root()
                .map(|r| crate::state::goals::check_script_path(r, &goal.goal_id, c.version))
                .filter(|p| p.is_file())
            {
                Some(p) => p,
                None => {
                    let text = goals.check_script(&goal.goal_id, Some(c.version)).await?.ok_or_else(|| {
                        crate::error::OpenDanError::NotFound(format!("check script {} of goal {}", c.version, goal.goal_id))
                    })?;
                    let dir = std::env::temp_dir().join("opendan-goal-checks");
                    std::fs::create_dir_all(&dir).map_err(|e| crate::error::OpenDanError::io(&dir, e))?;
                    let p = dir.join(format!("{}-{}.ts", goal.goal_id, c.version));
                    crate::fsutil::atomic_replace(&p, text.as_bytes())?;
                    p
                }
            }
        }
    };
    let calls = AgentCalls {
        agent_root: agent.agent_root().map(Path::to_path_buf),
    };
    Ok(runner.run(&path, &ctx, &stored, &calls).await)
}
