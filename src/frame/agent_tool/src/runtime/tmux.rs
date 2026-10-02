use super::{native_host_id, shell_quote, TmuxConfig, TmuxMode};
use crate::exec_tracking::{
    current_boot_id, new_execution_id, probe_execution, stop_execution, ExecutionProbe,
    ExecutionRecord, ExecutionRegistrar,
};
use crate::llm_bash::{BashRunOutput, BashRunRequest, BashRunner, BashTarget};
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
    pane: String,
    identity: Value,
    gate: tokio::sync::Mutex<()>,
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
        let o = Self::command(
            socket,
            &[
                "new-window",
                "-d",
                "-P",
                "-F",
                "#{pane_id}",
                "-t",
                &target,
                "-n",
                "llm_runtime",
                "-c",
                &cwd.display().to_string(),
                "bash --noprofile --norc",
            ],
        )
        .await?;
        if !o.status.success() {
            return Err(XllmError::Capability(format!(
                "cannot create dedicated tmux pane: {}",
                String::from_utf8_lossy(&o.stderr)
            )));
        }
        let pane = String::from_utf8_lossy(&o.stdout).trim().to_string();
        Ok(Self {
            socket: cfg.socket.clone(),
            pane,
            identity: json!({"session": session, "actual": actual}),
            gate: tokio::sync::Mutex::new(()),
        })
    }
    pub fn identity(&self) -> Value {
        self.identity.clone()
    }
    async fn send(&self, text: &str) -> Result<(), AgentToolError> {
        let out = Self::command(
            self.socket.as_deref(),
            &["send-keys", "-l", "-t", &self.pane, "--", text],
        )
        .await
        .map_err(|e| AgentToolError::Transport {
            message: e.to_string(),
            effect_unknown: true,
        })?;
        if !out.status.success() {
            return Err(AgentToolError::Transport {
                message: "tmux send-keys failed".into(),
                effect_unknown: true,
            });
        }
        let out = Self::command(
            self.socket.as_deref(),
            &["send-keys", "-t", &self.pane, "Enter"],
        )
        .await
        .map_err(|e| AgentToolError::Transport {
            message: e.to_string(),
            effect_unknown: true,
        })?;
        if !out.status.success() {
            return Err(AgentToolError::Transport {
                message: "tmux release failed".into(),
                effect_unknown: true,
            });
        }
        Ok(())
    }
}
impl Drop for TmuxTarget {
    fn drop(&mut self) {
        let mut c = std::process::Command::new("tmux");
        if let Some(s) = &self.socket {
            c.args(["-S", s]);
        }
        let _ = c
            .args(["kill-pane", "-t", &self.pane])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

pub struct TmuxBashRunner {
    target: Arc<TmuxTarget>,
    runtime_id: String,
    registrar: Arc<dyn ExecutionRegistrar>,
    env: BTreeMap<String, String>,
    pending: std::sync::Mutex<BTreeMap<String, ExecutionRecord>>,
}
impl TmuxBashRunner {
    pub fn new(
        target: Arc<TmuxTarget>,
        id: &str,
        registrar: Arc<dyn ExecutionRegistrar>,
        env: BTreeMap<String, String>,
    ) -> Self {
        Self {
            target,
            runtime_id: id.into(),
            registrar,
            env,
            pending: std::sync::Mutex::new(BTreeMap::new()),
        }
    }
}

struct ExecutionGuard {
    record: ExecutionRecord,
    dir: PathBuf,
    done: bool,
    cleanup: bool,
}
impl Drop for ExecutionGuard {
    fn drop(&mut self) {
        if self.done {
            if self.cleanup {
                let _ = std::fs::remove_dir_all(&self.dir);
            }
        } else {
            let rec = self.record.clone();
            let dir = self.dir.clone();
            if let Ok(h) = tokio::runtime::Handle::try_current() {
                h.spawn(async move {
                    if stop_execution(&rec, Some(&native_host_id()), Duration::from_secs(5))
                        .await
                        .is_ok()
                    {
                        let _ = tokio::fs::remove_dir_all(dir).await;
                    }
                });
            }
        }
    }
}

#[async_trait]
impl BashRunner for TmuxBashRunner {
    async fn cancel(&self) -> Result<(), AgentToolError> {
        let records = self
            .pending
            .lock()
            .expect("tmux pending")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for rec in records {
            stop_execution(&rec, Some(&native_host_id()), Duration::from_secs(5))
                .await
                .map_err(|message| AgentToolError::Transport {
                    message,
                    effect_unknown: true,
                })?;
            self.pending
                .lock()
                .expect("tmux pending")
                .remove(&rec.execution_id);
            self.registrar.completed(&rec.execution_id).await;
        }
        Ok(())
    }
    async fn run(
        &self,
        _ctx: &SessionRuntimeContext,
        req: BashRunRequest,
    ) -> Result<BashRunOutput, AgentToolError> {
        if let BashTarget::Unsupported(t) = &req.target {
            return Err(AgentToolError::InvalidArgs(format!(
                "unsupported target {t}"
            )));
        }
        let _serial = self.target.gate.lock().await;
        let pending = self
            .pending
            .lock()
            .expect("tmux pending")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for rec in pending {
            if matches!(
                probe_execution(&rec, Some(&native_host_id())),
                ExecutionProbe::Stopped
            ) {
                self.pending
                    .lock()
                    .expect("tmux pending")
                    .remove(&rec.execution_id);
                self.registrar.completed(&rec.execution_id).await;
            }
        }
        let started = Instant::now();
        let id = new_execution_id();
        let dir = std::env::temp_dir().join(format!("llm-tmux-{id}"));
        let mut builder = tokio::fs::DirBuilder::new();
        #[cfg(unix)]
        builder.mode(0o700);
        builder
            .create(&dir)
            .await
            .map_err(|e| AgentToolError::ExecFailed(e.to_string()))?;
        let mut rec = ExecutionRecord {
            execution_id: id.clone(),
            call_id: None,
            kind: "native".into(),
            runtime_id: Some(self.runtime_id.clone()),
            host: Some(native_host_id()),
            boot_id: current_boot_id(),
            pgid: None,
            leader_start_ticks: None,
            command: req.command.chars().take(512).collect(),
            started_at_ms: crate::exec_tracking::now_ms(),
        };
        let mut guard = ExecutionGuard {
            record: rec.clone(),
            dir: dir.clone(),
            done: false,
            cleanup: false,
        };
        let q = shell_quote(&dir.display().to_string());
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
        tokio::fs::write(dir.join("command"), command)
            .await
            .map_err(|e| AgentToolError::ExecFailed(e.to_string()))?;
        let wrapper = format!(
            r#"llm_pid=$BASHPID
IFS= read -r llm_stat < /proc/$llm_pid/stat
llm_rest=${{llm_stat##*) }}
read -ra llm_fields <<< "$llm_rest"
printf '%s\n' "$llm_pid" "${{llm_fields[19]}}" > {q}/identity
for ((i=0; i<300; i++)); do [ -f {q}/go ] && break; sleep .1; done
[ -f {q}/go ] || exit 125
bash --noprofile --norc {q}/command > >(tee {q}/stdout) 2> >(tee {q}/stderr >&2) </dev/null
llm_ec=$?
wait
printf '%s\n' "$llm_ec" > {q}/exit
"#
        );
        let invoke = format!(
            "env OPENDAN_EXECUTION_ID={} setsid bash -c {}",
            shell_quote(&id),
            shell_quote(&wrapper)
        );
        self.target.send(&invoke).await?;
        for _ in 0..200 {
            if dir.join("identity").exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let identity = tokio::fs::read_to_string(dir.join("identity"))
            .await
            .map_err(|e| AgentToolError::Transport {
                message: e.to_string(),
                effect_unknown: false,
            })?;
        let fields = identity.lines().collect::<Vec<_>>();
        rec.pgid = fields.first().and_then(|s| s.parse().ok());
        rec.leader_start_ticks = fields.get(1).and_then(|s| s.parse().ok());
        guard.record = rec.clone();
        self.registrar
            .register(&rec)
            .await
            .map_err(|e| AgentToolError::Transport {
                message: format!("registration failed; command not started: {e}"),
                effect_unknown: false,
            })?;
        self.pending
            .lock()
            .expect("tmux pending")
            .insert(id.clone(), rec.clone());
        tokio::fs::write(dir.join("go"), b"go")
            .await
            .map_err(|e| AgentToolError::Transport {
                message: e.to_string(),
                effect_unknown: true,
            })?;
        let mut exit = None;
        while started.elapsed() < Duration::from_millis(req.timeout_ms.max(1)) {
            if let Ok(s) = tokio::fs::read_to_string(dir.join("exit")).await {
                exit = s.trim().parse::<i32>().ok();
                break;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        let timed_out = exit.is_none();
        if timed_out {
            stop_execution(&rec, Some(&native_host_id()), Duration::from_secs(5))
                .await
                .map_err(|message| AgentToolError::Transport {
                    message,
                    effect_unknown: true,
                })?;
        }
        match probe_execution(&rec, Some(&native_host_id())) {
            ExecutionProbe::Stopped => {
                guard.done = true;
                guard.cleanup = true;
                self.pending.lock().expect("tmux pending").remove(&id);
                self.registrar.completed(&id).await;
            }
            ExecutionProbe::Alive { .. } => {
                guard.done = true;
            }
            ExecutionProbe::Unknown { reason } => {
                return Err(AgentToolError::Transport {
                    message: reason,
                    effect_unknown: true,
                })
            }
        }
        let stdout = tokio::fs::read(dir.join("stdout"))
            .await
            .unwrap_or_default();
        let stderr = tokio::fs::read(dir.join("stderr"))
            .await
            .unwrap_or_default();
        Ok(super::output(
            exit.unwrap_or(124),
            &stdout,
            &stderr,
            req.max_output_bytes,
            timed_out,
            started.elapsed(),
            "tmux",
            req.cwd,
        ))
    }
}
