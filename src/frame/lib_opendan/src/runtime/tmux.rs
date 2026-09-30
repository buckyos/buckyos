//! `TmuxRuntime` (ported from `opendan::agent_bash::TmuxBashRunner`): every
//! session gets a persistent `od_<sid>` tmux session an operator can attach
//! to (`tmux attach -t od_<sid>`); each command is sourced into its pane
//! through a wrapper script that tees stdout / stderr into the runtime's
//! instance directory (never the session directory) and writes an exit-code
//! file plus a pane marker.
//!
//! Execution tracking (§5.2): the execution identity is persisted through the
//! registrar *before* `send-keys` releases the command; the command's
//! subshell exports `OPENDAN_EXECUTION_ID`, which every child inherits, so a
//! later runner finds and stops leftovers by that marker (tmux runs are
//! never continued by xllm).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use agent_tool::exec_tracking::{
    current_boot_id, new_execution_id, probe_execution, stop_execution, ExecutionProbe,
    ExecutionRecord, ExecutionRegistrar, EXECUTION_ENV,
};
use agent_tool::llm_bash::{BashRunOutput, BashRunRequest, BashRunner, BashTarget};
use agent_tool::{AgentToolError, SessionRuntimeContext};
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::process::Command;

use crate::error::{OpenDanError, Result};
use crate::protocol::*;
use crate::session::SessionDir;

use super::native::{local_host_id, NativeRuntime};
use super::{AgentRuntime, BinPlan, SessionEnv, SessionEnvCtx};

pub const TMUX_SESSION_PREFIX: &str = "od_";
const POLL_MS: u64 = 50;
static RUN_SEQ: AtomicU64 = AtomicU64::new(0);

/// tmux session name of a session id (`[A-Za-z0-9_]` only).
pub fn tmux_session_name(sid: &str) -> String {
    let s: String = sid
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect();
    format!("{TMUX_SESSION_PREFIX}{s}")
}

fn shell_quote(raw: &str) -> String {
    if raw.is_empty() {
        return "''".to_string();
    }
    format!("'{}'", raw.replace('\'', "'\"'\"'"))
}

async fn tmux(args: &[&str]) -> std::result::Result<std::process::Output, AgentToolError> {
    Command::new("tmux")
        .args(args)
        .output()
        .await
        .map_err(|e| AgentToolError::ExecFailed(format!("tmux unavailable: {e}")))
}

async fn ensure_session(name: &str, cwd: &Path) -> std::result::Result<(), AgentToolError> {
    if tmux(&["has-session", "-t", name]).await?.status.success() {
        return Ok(());
    }
    let out = Command::new("tmux")
        .args(["new-session", "-d", "-s", name, "-c"])
        .arg(cwd)
        .arg("/bin/bash --noprofile --norc")
        .output()
        .await
        .map_err(|e| AgentToolError::ExecFailed(format!("tmux new-session: {e}")))?;
    if out.status.success() || tmux(&["has-session", "-t", name]).await?.status.success() {
        return Ok(());
    }
    Err(AgentToolError::ExecFailed(format!(
        "create tmux session `{name}` failed: {}",
        String::from_utf8_lossy(&out.stderr)
    )))
}

fn truncate_output(data: &[u8], max: usize) -> (String, bool) {
    if data.len() <= max {
        return (String::from_utf8_lossy(data).to_string(), false);
    }
    let head = max / 4;
    let tail = max - head;
    let omitted = data.len() - head - tail;
    (
        format!(
            "{}\n…[{omitted} bytes omitted]…\n{}",
            String::from_utf8_lossy(&data[..head]),
            String::from_utf8_lossy(&data[data.len() - tail..])
        ),
        true,
    )
}

fn build_script(
    rec: &ExecutionRecord,
    files: &RunFiles,
    cwd: &Path,
    command: &str,
    env: &[(String, String)],
) -> String {
    let mut l = Vec::new();
    l.push(format!("__od_out={}", shell_quote(&files.stdout.display().to_string())));
    l.push(format!("__od_err={}", shell_quote(&files.stderr.display().to_string())));
    l.push(format!("__od_ec_file={}", shell_quote(&files.exit.display().to_string())));
    l.push(": > \"$__od_out\"; : > \"$__od_err\"; rm -f \"$__od_ec_file\"".to_string());
    l.push("{".to_string());
    // The subshell carries the execution marker; the pane shell does not.
    l.push("  (".to_string());
    l.push(format!("    cd {} || exit 126", shell_quote(&cwd.display().to_string())));
    for (k, v) in env {
        l.push(format!("    export {k}={}", shell_quote(v)));
    }
    l.push(format!("    export {EXECUTION_ENV}={}", shell_quote(&rec.execution_id)));
    l.push(command.to_string());
    l.push("  )".to_string());
    l.push("} > >(tee \"$__od_out\") 2> >(tee \"$__od_err\" >&2) < /dev/null".to_string());
    l.push("__od_ec=$?".to_string());
    l.push("printf '%s\\n' \"$__od_ec\" > \"$__od_ec_file\"".to_string());
    l.push(format!(
        "printf '__OD_EXIT__%s:%s\\n' {} \"$__od_ec\"",
        shell_quote(&rec.execution_id)
    ));
    l.join("\n")
}

struct RunFiles {
    script: PathBuf,
    stdout: PathBuf,
    stderr: PathBuf,
    exit: PathBuf,
}

impl RunFiles {
    fn new(dir: &Path, id: &str) -> Self {
        Self {
            script: dir.join(format!("{id}.exec.sh")),
            stdout: dir.join(format!("{id}.stdout.log")),
            stderr: dir.join(format!("{id}.stderr.log")),
            exit: dir.join(format!("{id}.exit.code")),
        }
    }

    async fn cleanup(&self) {
        for p in [&self.script, &self.stdout, &self.stderr, &self.exit] {
            let _ = tokio::fs::remove_file(p).await;
        }
    }
}

/// `exec` runner sending commands into the session's tmux pane.
pub struct TmuxBashRunner {
    session: String,
    runtime_dir: PathBuf,
    registrar: Arc<dyn ExecutionRegistrar>,
    runtime_id: String,
    host: String,
    path_layers: Vec<PathBuf>,
    env: Vec<(String, String)>,
}

impl TmuxBashRunner {
    fn path_value(&self, user_env: &[(String, String)]) -> String {
        let base = user_env
            .iter()
            .rev()
            .find(|(k, _)| k == "PATH")
            .map(|(_, v)| v.clone())
            .or_else(|| std::env::var("PATH").ok())
            .unwrap_or_else(|| "/usr/local/bin:/usr/bin:/bin".into());
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
}

#[async_trait]
impl BashRunner for TmuxBashRunner {
    async fn run(
        &self,
        _ctx: &SessionRuntimeContext,
        req: BashRunRequest,
    ) -> std::result::Result<BashRunOutput, AgentToolError> {
        if let BashTarget::Unsupported(v) = &req.target {
            return Err(AgentToolError::InvalidArgs(format!(
                "unsupported exec target `{v}` (only local is supported)"
            )));
        }
        let started = Instant::now();
        ensure_session(&self.session, &req.cwd).await?;
        tokio::fs::create_dir_all(&self.runtime_dir)
            .await
            .map_err(|e| AgentToolError::ExecFailed(format!("runtime dir: {e}")))?;
        let rec = ExecutionRecord {
            execution_id: new_execution_id(),
            call_id: None,
            kind: "native".to_string(), // tracked by the /proc marker scan
            runtime_id: Some(self.runtime_id.clone()),
            host: Some(self.host.clone()),
            boot_id: current_boot_id(),
            pgid: None,
            leader_start_ticks: None,
            command: req.command.chars().take(512).collect(),
            started_at_ms: agent_tool::exec_tracking::now_ms(),
        };
        let files = RunFiles::new(
            &self.runtime_dir,
            &format!("{}-{}", rec.execution_id, RUN_SEQ.fetch_add(1, Ordering::Relaxed)),
        );
        let mut env = self.env.clone();
        env.extend(req.env.iter().cloned());
        env.retain(|(k, _)| k != "PATH");
        env.push(("PATH".into(), self.path_value(&req.env)));
        let script = build_script(&rec, &files, &req.cwd, &req.command, &env);
        tokio::fs::write(&files.script, script)
            .await
            .map_err(|e| AgentToolError::ExecFailed(format!("write exec script: {e}")))?;
        // Persist the identity before the command is released.
        if let Err(e) = self.registrar.register(&rec).await {
            files.cleanup().await;
            return Err(AgentToolError::ExecFailed(format!(
                "execution tracking could not be persisted; command not started: {e}"
            )));
        }
        let target = format!("{}:0.0", self.session);
        let banner = format!(
            "printf '\\n# exec[%s] %s\\n' {} {}",
            shell_quote(&rec.execution_id),
            shell_quote(&rec.command)
        );
        let _ = tmux(&["send-keys", "-t", &target, "--", &banner, "C-m"]).await;
        let invoke = format!(". {}", shell_quote(&files.script.display().to_string()));
        let out = tmux(&["send-keys", "-t", &target, "--", &invoke, "C-m"]).await?;
        if !out.status.success() {
            files.cleanup().await;
            self.registrar.completed(&rec.execution_id).await;
            return Err(AgentToolError::ExecFailed(format!(
                "tmux send-keys failed: {}",
                String::from_utf8_lossy(&out.stderr)
            )));
        }
        let deadline = Duration::from_millis(req.timeout_ms.max(1));
        let marker = format!("__OD_EXIT__{}:", rec.execution_id);
        let mut exit: Option<i32> = None;
        while started.elapsed() < deadline {
            if let Ok(s) = tokio::fs::read_to_string(&files.exit).await {
                if let Ok(c) = s.trim().parse::<i32>() {
                    exit = Some(c);
                    break;
                }
            }
            if let Ok(o) = tmux(&["capture-pane", "-p", "-J", "-t", &target]).await {
                let pane = String::from_utf8_lossy(&o.stdout);
                if let Some(line) = pane.lines().rev().find(|l| l.contains(&marker)) {
                    if let Some(c) = line
                        .split(&marker)
                        .nth(1)
                        .and_then(|t| t.trim().parse::<i32>().ok())
                    {
                        exit = Some(c);
                        break;
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(POLL_MS)).await;
        }
        let timed_out = exit.is_none();
        if timed_out {
            let _ = tmux(&["send-keys", "-t", &target, "C-c"]).await;
            let _ = stop_execution(&rec, Some(&self.host), Duration::from_secs(5)).await;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
        let stdout = tokio::fs::read(&files.stdout).await.unwrap_or_default();
        let stderr = tokio::fs::read(&files.stderr).await.unwrap_or_default();
        files.cleanup().await;
        if probe_execution(&rec, Some(&self.host)) == ExecutionProbe::Stopped {
            self.registrar.completed(&rec.execution_id).await;
        }
        let mut merged = stdout.clone();
        if !stderr.is_empty() {
            if !merged.is_empty() && !merged.ends_with(b"\n") {
                merged.push(b'\n');
            }
            merged.extend_from_slice(&stderr);
        }
        let (output, output_truncated) = truncate_output(&merged, req.max_output_bytes);
        Ok(BashRunOutput {
            exit_code: exit.unwrap_or(124),
            stdout: truncate_output(&stdout, req.max_output_bytes).0,
            stderr: truncate_output(&stderr, req.max_output_bytes).0,
            output,
            output_truncated,
            timed_out,
            duration_ms: started.elapsed().as_millis() as u64,
            engine: "tmux".to_string(),
            cwd: req.cwd,
        })
    }
}

/// tmux-backed runtime (the future default paios runtime, in a container).
pub struct TmuxRuntime {
    native: NativeRuntime,
    desc: RuntimeDescriptor,
    instance_dir: PathBuf,
}

impl TmuxRuntime {
    /// `instance_dir` holds exec scripts and output logs (runtime volume).
    pub fn new(runtime_id: &str, provider: &str, instance_dir: impl Into<PathBuf>) -> Self {
        let native = NativeRuntime::local(runtime_id, provider);
        let mut desc = native.descriptor().clone();
        desc.kind = "tmux".to_string();
        Self {
            native,
            desc,
            instance_dir: instance_dir.into(),
        }
    }

    pub fn available() -> bool {
        std::process::Command::new("tmux")
            .arg("-V")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

#[async_trait]
impl AgentRuntime for TmuxRuntime {
    fn descriptor(&self) -> &RuntimeDescriptor {
        &self.desc
    }

    fn host_id(&self) -> &str {
        self.native.host_id()
    }

    fn resolve_workdir(
        &self,
        sd: &SessionDir,
        workspace: Option<&WorkspaceRef>,
        agent_root: Option<&Path>,
    ) -> Result<PathBuf> {
        self.native.resolve_workdir(sd, workspace, agent_root)
    }

    fn can_access(&self, path: &Path) -> bool {
        self.native.can_access(path)
    }

    fn has_tool(&self, name: &str) -> bool {
        self.native.has_tool(name)
    }

    async fn prepare_session_bin(&self, sd: &SessionDir, plan: &BinPlan) -> Result<()> {
        self.native.prepare_session_bin(sd, plan).await
    }

    async fn verify_session_env(&self, sd: &SessionDir, binding: &Binding, plan: &BinPlan) -> Result<()> {
        if !Self::available() {
            return Err(OpenDanError::Bind("tmux is not available".into()));
        }
        self.native.verify_session_env(sd, binding, plan).await
    }

    async fn open_session_env(&self, binding: &Binding, ctx: &SessionEnvCtx) -> Result<SessionEnv> {
        let mut env = self.native.open_session_env(binding, ctx).await?;
        env.kind = "tmux".to_string();
        env.runtime_id = self.desc.runtime_id.clone();
        ensure_session(&tmux_session_name(&ctx.session_id), &env.workdir)
            .await
            .map_err(|e| OpenDanError::Bind(e.to_string()))?;
        env.env.push(("OPENDAN_TMUX_SESSION".into(), tmux_session_name(&ctx.session_id)));
        Ok(env)
    }

    fn bash_runner(&self, env: &SessionEnv, registrar: Arc<dyn ExecutionRegistrar>) -> Arc<dyn agent_tool::llm_bash::BashRunner> {
        let sid = env
            .env
            .iter()
            .find(|(k, _)| k == "OPENDAN_SESSION_ID")
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        Arc::new(TmuxBashRunner {
            session: tmux_session_name(&sid),
            runtime_dir: self.instance_dir.join(crate::ids::sanitize_segment(&sid)),
            registrar,
            runtime_id: self.desc.runtime_id.clone(),
            host: local_host_id(),
            path_layers: env.path_layers.clone(),
            env: env.env.clone(),
        })
    }

    async fn reconcile_execution(&self, rec: &ExecutionRecord) -> Result<()> {
        self.native.reconcile_execution(rec).await
    }

    async fn status(&self) -> Value {
        let sessions = tmux(&["list-sessions", "-F", "#{session_name}"])
            .await
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .filter(|l| l.starts_with(TMUX_SESSION_PREFIX))
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        json!({
            "runtime_id": self.desc.runtime_id,
            "kind": "tmux",
            "host": self.native.host_id(),
            "tmux_sessions": sessions,
        })
    }

    async fn close_session_env(&self, _env: SessionEnv) -> Result<()> {
        // The tmux session stays for audit (`tmux attach`); GC is separate.
        Ok(())
    }
}
