//! tmux runtime: each `shell` command runs in its own window of the
//! configured session, writing `command` / `stdout` / `stderr` / `exit`
//! into the run's execution directory. tmux is the isolation: an executor
//! that is interrupted or dies leaves the command running; a `shell`
//! timeout or a task cancel kills the window (tmux's own means, no /proc,
//! no setsid).

use super::{shell_quote, TmuxConfig, TmuxMode};
use crate::llm_bash::{
    exec_dir_for, read_exit_file, read_file_tail, sanitize_call_id, BashRunOutput, BashRunRequest,
    BashRunner, BashTarget, CommandHandle, CommandProgress, RunBindingSlot, OUTPUT_TAIL_BYTES,
    TIMEOUT_EXIT_CODE,
};
use crate::xllm::XllmError;
use crate::{AgentToolError, SessionRuntimeContext};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::process::Command;

pub fn tmux_session_name(sid: &str) -> String {
    format!(
        "od_{}",
        sid.chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            })
            .collect::<String>()
    )
}

pub struct TmuxTarget {
    socket: Option<String>,
    session: String,
    identity: Value,
}

impl TmuxTarget {
    async fn command(
        socket: Option<&str>,
        args: &[&str],
    ) -> Result<std::process::Output, XllmError> {
        let mut c = Command::new("tmux");
        if let Some(s) = socket {
            c.args(["-S", s]);
        }
        c.args(args).stdin(Stdio::null()).kill_on_drop(true);
        tokio::time::timeout(Duration::from_secs(10), c.output())
            .await
            .map_err(|e| XllmError::Capability(e.to_string()))?
            .map_err(|e| XllmError::Capability(format!("tmux unavailable: {e}")))
    }

    pub async fn open(cfg: &TmuxConfig, cwd: &Path) -> Result<Self, XllmError> {
        let session = cfg.session.as_ref().unwrap();
        let target = format!("={session}");
        let socket = cfg.socket.as_deref();
        let exists = Self::command(socket, &["has-session", "-t", &target])
            .await?
            .status
            .success();
        match (cfg.mode.unwrap_or_default(), exists) {
            (TmuxMode::Create, true) => {
                return Err(XllmError::Capability(format!(
                    "tmux session {session} already exists"
                )))
            }
            (TmuxMode::Attach, false) => {
                return Err(XllmError::Capability(format!(
                    "tmux session {session} does not exist"
                )))
            }
            _ => {}
        }
        if !exists {
            let o = Self::command(
                socket,
                &[
                    "new-session",
                    "-d",
                    "-s",
                    session,
                    "-c",
                    &cwd.display().to_string(),
                    "bash --noprofile --norc",
                ],
            )
            .await?;
            if !o.status.success() {
                return Err(XllmError::Capability(
                    String::from_utf8_lossy(&o.stderr).to_string(),
                ));
            }
        }
        let o = Self::command(
            socket,
            &[
                "display-message",
                "-p",
                "-t",
                &target,
                "#{socket_path}|#{session_id}|#{pid}",
            ],
        )
        .await?;
        if !o.status.success() {
            return Err(XllmError::Capability("cannot identify tmux session".into()));
        }
        let actual = String::from_utf8_lossy(&o.stdout).trim().to_string();
        Ok(Self {
            socket: cfg.socket.clone(),
            session: session.clone(),
            identity: json!({"session": session, "actual": actual}),
        })
    }

    pub fn identity(&self) -> Value {
        self.identity.clone()
    }

    pub fn session(&self) -> &str {
        &self.session
    }

    async fn tmux(&self, args: &[&str]) -> Result<std::process::Output, AgentToolError> {
        Self::command(self.socket.as_deref(), args)
            .await
            .map_err(|e| AgentToolError::Transport {
                message: e.to_string(),
                effect_unknown: true,
            })
    }

    /// Start `script` in a new window named `window`; returns once the
    /// window exists.
    async fn new_window(
        &self,
        window: &str,
        cwd: &Path,
        script: &str,
    ) -> Result<(), AgentToolError> {
        let target = format!("={}", self.session);
        let out = self
            .tmux(&[
                "new-window",
                "-d",
                "-t",
                &target,
                "-n",
                window,
                "-c",
                &cwd.display().to_string(),
                script,
            ])
            .await?;
        if !out.status.success() {
            return Err(AgentToolError::Transport {
                message: format!(
                    "tmux new-window failed: {}",
                    String::from_utf8_lossy(&out.stderr)
                ),
                effect_unknown: false,
            });
        }
        Ok(())
    }

    async fn kill_window(&self, window: &str) -> Result<(), AgentToolError> {
        let target = format!("={}:={window}", self.session);
        let out = self.tmux(&["kill-window", "-t", &target]).await?;
        // A window that already closed (command ended) is not an error.
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            if !err.contains("can't find") {
                return Err(AgentToolError::Transport {
                    message: format!("tmux kill-window failed: {err}"),
                    effect_unknown: true,
                });
            }
        }
        Ok(())
    }
}

pub struct TmuxBashRunner {
    target: Arc<TmuxTarget>,
    env: BTreeMap<String, String>,
    run: RunBindingSlot,
}

impl TmuxBashRunner {
    pub fn new(
        target: Arc<TmuxTarget>,
        env: BTreeMap<String, String>,
        run: RunBindingSlot,
    ) -> Self {
        Self { target, env, run }
    }

    fn exec_dir(&self, call_id: Option<&str>) -> PathBuf {
        let bound = self
            .run
            .lock()
            .expect("run binding")
            .clone()
            .and_then(|b| exec_dir_for(b.run_dir.as_deref(), call_id));
        bound.unwrap_or_else(|| {
            std::env::temp_dir().join(format!(
                "llm-tmux-{}-{}",
                std::process::id(),
                crate::tasks::next_local_seq()
            ))
        })
    }
}

/// Wrapper run as the window's command: output to files, exit code to
/// `exit`; the window closes when it ends.
fn window_script(dir: &Path) -> String {
    let q = shell_quote(&dir.display().to_string());
    format!(
        "bash --noprofile --norc {q}/command </dev/null >{q}/stdout 2>{q}/stderr; __llm_ec=$?; printf '%s\\n' \"$__llm_ec\" > {q}/exit.tmp && mv -f {q}/exit.tmp {q}/exit"
    )
}

#[async_trait]
impl BashRunner for TmuxBashRunner {
    fn engine(&self) -> &str {
        "tmux"
    }

    async fn start(
        &self,
        _ctx: &SessionRuntimeContext,
        req: BashRunRequest,
    ) -> Result<Box<dyn CommandHandle>, AgentToolError> {
        if let BashTarget::Unsupported(t) = &req.target {
            return Err(AgentToolError::InvalidArgs(format!(
                "unsupported target {t}"
            )));
        }
        let dir = self.exec_dir(req.call_id.as_deref());
        let mut builder = tokio::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        builder.mode(0o700);
        builder
            .create(&dir)
            .await
            .map_err(|e| AgentToolError::ExecFailed(e.to_string()))?;
        for stale in ["exit", "exit.tmp"] {
            let _ = std::fs::remove_file(dir.join(stale));
        }
        let mut env = self.env.clone();
        env.extend(req.env.iter().cloned());
        let mut command = format!(
            "cd {} || exit 126\n",
            shell_quote(&req.cwd.display().to_string())
        );
        for (k, v) in &env {
            command.push_str(&format!("export {k}={}\n", shell_quote(v)));
        }
        command.push_str(&req.command);
        command.push('\n');
        tokio::fs::write(dir.join("command"), command)
            .await
            .map_err(|e| AgentToolError::ExecFailed(e.to_string()))?;
        let window = format!(
            "llm-{}",
            req.call_id
                .as_deref()
                .map(sanitize_call_id)
                .unwrap_or_else(|| crate::tasks::next_local_seq().to_string())
        );
        self.target
            .new_window(&window, &req.cwd, &window_script(&dir))
            .await?;
        Ok(Box::new(TmuxCommand {
            target: self.target.clone(),
            window,
            dir,
            started: Instant::now(),
            max_output: req.max_output_bytes,
            cwd: req.cwd,
            finished: None,
        }))
    }
}

pub struct TmuxCommand {
    target: Arc<TmuxTarget>,
    window: String,
    dir: PathBuf,
    started: Instant,
    max_output: usize,
    cwd: PathBuf,
    finished: Option<BashRunOutput>,
}

impl TmuxCommand {
    fn output(&self, exit_code: i32, timed_out: bool) -> BashRunOutput {
        crate::llm_bash::output_from_exec_dir(
            &self.dir,
            exit_code,
            timed_out,
            self.started.elapsed(),
            self.max_output,
            "tmux",
            self.cwd.clone(),
        )
    }
}

#[async_trait]
impl CommandHandle for TmuxCommand {
    async fn wait(&mut self, timeout: Duration) -> Result<Option<BashRunOutput>, AgentToolError> {
        if let Some(done) = &self.finished {
            return Ok(Some(done.clone()));
        }
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(code) = read_exit_file(&self.dir) {
                let out = self.output(code, false);
                self.finished = Some(out.clone());
                return Ok(Some(out));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
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
        self.target.kill_window(&self.window).await?;
        let code = read_exit_file(&self.dir).unwrap_or(TIMEOUT_EXIT_CODE);
        let out = self.output(code, true);
        self.finished = Some(out.clone());
        Ok(out)
    }

    fn detach(&mut self) {}

    fn cancellable(&self) -> bool {
        true
    }

    fn locator(&self) -> String {
        format!(
            "tmux session `{}` window `{}`; output and `exit` file in {}",
            self.target.session(),
            self.window,
            self.dir.display()
        )
    }
}
