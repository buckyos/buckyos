pub mod files;
pub mod ssh;
pub mod tmux;

use crate::exec_tracking::{unresolved_reason, InflightAction};
use crate::llm_bash::{
    exec_dir_for, new_run_binding_slot, read_exit_file, read_file_tail, BashRunOutput,
    BashRunRequest, BashRunner, BashTarget, LocalProcessBashRunner, RunBinding, RunBindingSlot,
    OUTPUT_TAIL_BYTES, TOOL_SHELL,
};
use crate::xllm::{EffectiveTools, LoopModel, ToolsConfig, XllmDeps, XllmError, XllmToolManager};
use crate::{AgentToolError, SessionRuntimeContext};
use async_trait::async_trait;
use files::{FileBackend, LocalFileBackend};
use llm_context::deps::{ToolCallCtx, ToolManager};
use llm_context::prompt_engine::{
    PromptExec, PromptExecOutput, PromptExecRequest, RenderError, ValueLoader,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

type Result<T> = std::result::Result<T, XllmError>;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workdir: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tmux: Option<TmuxConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_ssh: Option<SshConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TmuxConfig {
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socket: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<TmuxMode>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TmuxMode {
    Create,
    Attach,
    #[default]
    CreateOrAttach,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SshConfig {
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<String>,
}

impl RuntimeConfig {
    pub fn kind(&self) -> &str {
        self.kind.as_deref().unwrap_or("native")
    }
    pub fn validate(&self, complete: bool) -> Result<()> {
        let bad = |reason: String| XllmError::Config {
            file: ".llm_context".into(),
            field: "runtime".into(),
            reason,
        };
        match self.kind() {
            "native" | "tmux" | "remote_ssh" => {}
            "container" | "container_host" | "remote_node" | "http_proxy_runtime" => {
                return Err(XllmError::Capability(format!(
                    "runtime `{}` is planned but not implemented",
                    self.kind()
                )))
            }
            kind => return Err(bad(format!("unknown runtime kind `{kind}`"))),
        }
        if self.tmux.is_some() && self.kind() != "tmux" && self.kind.is_some() {
            return Err(bad("tmux connection block requires kind: tmux".into()));
        }
        if self.remote_ssh.is_some() && self.kind() != "remote_ssh" && self.kind.is_some() {
            return Err(bad(
                "remote_ssh connection block requires kind: remote_ssh".into()
            ));
        }
        for (k, v) in &self.env {
            if !valid_env_key(k) || v.contains('\0') {
                return Err(bad(format!("invalid env key or value `{k}`")));
            }
        }
        if self.id.as_ref().is_some_and(|id| id.trim().is_empty()) {
            return Err(bad("id cannot be empty".into()));
        }
        if complete {
            if self.tmux.is_some() && self.kind() != "tmux"
                || self.remote_ssh.is_some() && self.kind() != "remote_ssh"
            {
                return Err(bad("connection block does not match runtime kind".into()));
            }
            match self.kind() {
                "tmux" => {
                    let name = self
                        .tmux
                        .as_ref()
                        .and_then(|t| t.session.as_deref())
                        .unwrap_or("");
                    if name.is_empty()
                        || name.starts_with('-')
                        || name.chars().any(|c| c.is_control() || c == ':' || c == '.')
                    {
                        return Err(bad(
                            "tmux.session is required and must be a valid session name".into(),
                        ));
                    }
                }
                "remote_ssh" => {
                    let s = self
                        .remote_ssh
                        .as_ref()
                        .ok_or_else(|| bad("remote_ssh.host is required".into()))?;
                    for val in [s.host.as_deref(), s.user.as_deref()].into_iter().flatten() {
                        if val.is_empty()
                            || val.starts_with('-')
                            || val.chars().any(|c| c.is_whitespace() || c.is_control())
                        {
                            return Err(bad("invalid SSH host/user".into()));
                        }
                    }
                    if s.host.is_none() || s.port == Some(0) {
                        return Err(bad(
                            "remote_ssh.host is required; port must be nonzero".into()
                        ));
                    }
                    let cwd = self.workdir.as_deref().unwrap_or("");
                    if !cwd.starts_with('/') || cwd.contains('\0') {
                        return Err(bad(
                            "remote_ssh requires an explicit absolute remote workdir".into(),
                        ));
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
    pub fn resolve_paths(&mut self, source: &Path) {
        if self.kind.is_some() && self.kind() != "remote_ssh" {
            if let Some(p) = &mut self.workdir {
                *p = crate::xllm::resolve_config_path(p, source)
                    .display()
                    .to_string();
            }
        }
        if let Some(t) = &mut self.tmux {
            if let Some(p) = &mut t.socket {
                *p = crate::xllm::resolve_config_path(p, source)
                    .display()
                    .to_string();
            }
        }
        if let Some(s) = &mut self.remote_ssh {
            if let Some(p) = &mut s.identity_file {
                *p = crate::xllm::resolve_config_path(p, source)
                    .display()
                    .to_string();
            }
        }
    }
    pub fn merge_over(&mut self, other: &Self) {
        if other.kind.is_some() && other.kind() != self.kind() {
            *self = Self::default();
        }
        if other.kind.is_some() {
            self.kind = other.kind.clone();
        }
        if other.id.is_some() {
            self.id = other.id.clone();
        }
        if other.workdir.is_some() {
            self.workdir = other.workdir.clone();
        }
        self.env.extend(other.env.clone());
        if let Some(t) = &other.tmux {
            let b = self.tmux.get_or_insert_with(Default::default);
            if t.session.is_some() {
                b.session = t.session.clone();
            }
            if t.socket.is_some() {
                b.socket = t.socket.clone();
            }
            if t.mode.is_some() {
                b.mode = t.mode;
            }
        }
        if let Some(s) = &other.remote_ssh {
            let b = self.remote_ssh.get_or_insert_with(Default::default);
            if s.host.is_some() {
                b.host = s.host.clone();
            }
            if s.user.is_some() {
                b.user = s.user.clone();
            }
            if s.port.is_some() {
                b.port = s.port;
            }
            if s.identity_file.is_some() {
                b.identity_file = s.identity_file.clone();
            }
        }
    }
}

pub fn valid_env_key(k: &str) -> bool {
    let mut c = k.chars();
    c.next()
        .is_some_and(|x| x.is_ascii_alphabetic() || x == '_')
        && c.all(|x| x.is_ascii_alphanumeric() || x == '_')
}

pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Capabilities {
    pub tools: BTreeMap<String, String>,
    pub network: bool,
    pub os: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuntimeDescriptor {
    pub runtime_id: String,
    pub kind: String,
    pub host: Option<String>,
    pub target: Value,
    pub workdir: String,
    pub capabilities: Capabilities,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeInfo {
    pub id: String,
    pub kind: String,
    pub os: String,
    pub arch: String,
    pub hostname: String,
    pub shell: String,
    pub cwd: String,
    pub tools: Vec<String>,
    pub current_time: String,
    pub timezone: String,
    pub target: Value,
}

impl RuntimeInfo {
    pub fn template_values(&self) -> BTreeMap<String, String> {
        let v = serde_json::to_value(self).expect("runtime info");
        v.as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| v.to_string()),
                )
            })
            .collect()
    }
}

#[derive(Clone)]
pub struct RuntimeOpenCtx {
    pub workdir: PathBuf,
    pub run_id: String,
    pub loop_model: LoopModel,
    pub sources: BTreeMap<String, String>,
    pub env: BTreeMap<String, String>,
    pub path_prefix: Vec<PathBuf>,
    /// The run the opened tools belong to; bound once the run id and its
    /// directory are known (`XllmToolManager::bind_run`). Execution
    /// directories of `shell` commands derive from it.
    pub run: RunBindingSlot,
}

impl RuntimeOpenCtx {
    pub fn new(workdir: &Path, run_id: &str, loop_model: LoopModel) -> Self {
        Self {
            workdir: workdir.into(),
            run_id: run_id.into(),
            loop_model,
            sources: BTreeMap::new(),
            env: BTreeMap::new(),
            path_prefix: Vec::new(),
            run: new_run_binding_slot(),
        }
    }
}

#[async_trait]
pub trait AgentRuntime: Send + Sync {
    fn descriptor(&self) -> &RuntimeDescriptor;
    fn config(&self) -> RuntimeConfig;
    async fn info(&self) -> Result<RuntimeInfo>;
    async fn open(
        &self,
        ctx: &RuntimeOpenCtx,
        tools: &ToolsConfig,
        deps: &XllmDeps,
    ) -> Result<(EffectiveTools, XllmToolManager)>;

    /// The text a previous executor's in-flight `action` is answered with
    /// after a crash (long-tool TODO §3.2): no process is verified or
    /// stopped. For `shell` the runtime reads the execution directory of
    /// `(run, call_id)` — exit code and output tail when the command ended,
    /// "may still be running" and where to look otherwise.
    async fn describe_interrupted(&self, run: &RunBinding, action: &InflightAction) -> String;
}

#[async_trait]
pub trait Sandbox: ToolManager {
    fn workdir(&self) -> &str;
    fn env_check(&self) -> Value;
    async fn exec(
        &self,
        request: BashRunRequest,
        ctx: &SessionRuntimeContext,
    ) -> std::result::Result<BashRunOutput, AgentToolError>;
}

struct OpenedRuntime {
    config: RuntimeConfig,
    descriptor: RuntimeDescriptor,
    info: RuntimeInfo,
    files: Arc<dyn FileBackend>,
    ssh: Option<Arc<ssh::SshTransport>>,
    tmux: Option<Arc<tmux::TmuxTarget>>,
    base_path: String,
}

pub struct Runtime {
    config: RuntimeConfig,
    provisional: RuntimeDescriptor,
    opened: tokio::sync::OnceCell<OpenedRuntime>,
}
pub type NativeRuntime = Runtime;
pub type TmuxRuntime = Runtime;

/// What a finished `shell` command's execution directory says.
pub(crate) struct ExecDirFacts {
    pub exit_code: Option<i32>,
    pub stdout_tail: String,
    pub stderr_tail: String,
    pub exists: bool,
}

pub(crate) fn read_exec_dir(dir: &Path) -> ExecDirFacts {
    ExecDirFacts {
        exit_code: read_exit_file(dir),
        stdout_tail: read_file_tail(&dir.join("stdout"), OUTPUT_TAIL_BYTES),
        stderr_tail: read_file_tail(&dir.join("stderr"), OUTPUT_TAIL_BYTES),
        exists: dir.is_dir(),
    }
}

fn shell_command_of(action: &InflightAction) -> String {
    action
        .args
        .get("command")
        .and_then(Value::as_str)
        .map(|c| {
            let one = c.split_whitespace().collect::<Vec<_>>().join(" ");
            if one.chars().count() > 160 {
                format!("{}…", one.chars().take(160).collect::<String>())
            } else {
                one
            }
        })
        .unwrap_or_default()
}

fn tails(facts: &ExecDirFacts) -> String {
    let mut s = String::new();
    if !facts.stdout_tail.trim().is_empty() {
        s.push_str("\n--- stdout (tail) ---\n");
        s.push_str(facts.stdout_tail.trim_end());
    }
    if !facts.stderr_tail.trim().is_empty() {
        s.push_str("\n--- stderr (tail) ---\n");
        s.push_str(facts.stderr_tail.trim_end());
    }
    s
}

fn started_text(action: &InflightAction) -> String {
    let elapsed = crate::now_ms().saturating_sub(action.started_at_ms);
    format!(
        "started {} ago (at {} ms since epoch)",
        crate::llm_bash::fmt_elapsed(elapsed),
        action.started_at_ms
    )
}

/// Text for an interrupted `shell` call, by runtime kind (§3.2).
pub(crate) fn interrupted_shell_text(
    kind: &str,
    target: &str,
    action: &InflightAction,
    facts: Option<&ExecDirFacts>,
    location: &str,
) -> String {
    let command = shell_command_of(action);
    let started = started_text(action);
    if let Some(f) = facts {
        if let Some(code) = f.exit_code {
            return format!(
                "shell ({kind}): the previous executor exited while `{command}` was running ({started}). The command has since ended with exit code {code} (read from {location}); its result was never returned, so review it before deciding whether to repeat anything.{}",
                tails(f)
            );
        }
    }
    match kind {
        "tmux" => format!(
            "shell ({kind}): the previous executor exited while `{command}` was running in tmux session `{target}` ({started}). No exit was recorded, so the command may still be running there; its output and `exit` file are in {location}. Check (tmux, the files, `ps`) before repeating it.{}",
            facts.map(tails).unwrap_or_default()
        ),
        "remote_ssh" => match facts {
            Some(f) if f.exists => format!(
                "shell ({kind}): the previous executor exited while `{command}` was running on `{target}` ({started}). No exit was recorded, so the command may still be running there; its output and `exit` file are in {location}. Check before repeating it.{}",
                tails(f)
            ),
            _ => format!(
                "shell ({kind}): the previous executor exited while `{command}` was running on `{target}` ({started}). The remote execution directory {location} could not be read (host unreachable or directory missing); the command may have completed or may still be running. Check the host before repeating it."
            ),
        },
        _ => format!(
            "shell ({kind}): the previous executor exited while `{command}` was running ({started}). The command may have partly executed; it usually ended together with the executor, but not necessarily (e.g. after kill -9 it may still be running). Background processes it started are unaffected. Check the actual state before repeating it.{}",
            facts.map(tails).unwrap_or_default()
        ),
    }
}

pub fn native_host_id() -> String {
    let host = std::fs::read_to_string("/etc/hostname")
        .unwrap_or_else(|_| std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".into()));
    let machine = std::fs::read_to_string("/etc/machine-id").unwrap_or_default();
    format!("host:{}:{}", host.trim(), machine.trim())
}

impl Runtime {
    pub fn local(id: &str, _provider: &str) -> Self {
        Self::from_config(RuntimeConfig {
            id: Some(id.into()),
            ..Default::default()
        })
    }
    pub fn from_config(config: RuntimeConfig) -> Self {
        let provisional = RuntimeDescriptor {
            runtime_id: config.id.clone().unwrap_or_default(),
            kind: config.kind().into(),
            host: Some(native_host_id()),
            target: Value::Null,
            workdir: config.workdir.clone().unwrap_or_default(),
            capabilities: Capabilities::default(),
        };
        Self {
            config,
            provisional,
            opened: tokio::sync::OnceCell::new(),
        }
    }
    pub fn available() -> bool {
        std::process::Command::new("tmux")
            .arg("-V")
            .output()
            .is_ok_and(|o| o.status.success())
    }
    async fn initialize(&self, fallback: &Path) -> Result<&OpenedRuntime> {
        self.opened
            .get_or_try_init(|| async {
                self.config.validate(true)?;
                let mut config = self.config.clone();
                let kind = config.kind().to_string();
                config.kind = Some(kind.clone());
                let cwd = config
                    .workdir
                    .clone()
                    .unwrap_or_else(|| fallback.display().to_string());
                let (info, target, files, ssh, tmux, base_path) = if kind == "remote_ssh" {
                    let transport = Arc::new(
                        ssh::SshTransport::connect(config.remote_ssh.as_ref().unwrap()).await?,
                    );
                    let probe = transport.probe(&cwd, &config.env).await?;
                    let target = transport.target(&probe);
                    let info = probe.info.clone();
                    let base_path = probe.path.clone();
                    let transport = Arc::new(transport.with_identity(probe.identity.clone()));
                    transport.verify_sftp().await?;
                    (
                        info,
                        target,
                        transport.clone() as Arc<dyn FileBackend>,
                        Some(transport),
                        None,
                        base_path,
                    )
                } else {
                    let cwd = PathBuf::from(&cwd)
                        .canonicalize()
                        .map_err(|e| XllmError::Capability(format!("runtime cwd {cwd}: {e}")))?;
                    if !cwd.is_dir() {
                        return Err(XllmError::Capability(
                            "runtime workdir must be a directory".into(),
                        ));
                    }
                    let tmux = if kind == "tmux" {
                        let t = Arc::new(
                            tmux::TmuxTarget::open(config.tmux.as_ref().unwrap(), &cwd).await?,
                        );
                        Some(t)
                    } else {
                        None
                    };
                    let probe = if let Some(t) = &tmux {
                        t.probe(&cwd, &config.env).await?
                    } else {
                        ssh::local_probe(&cwd, &config.env).await?
                    };
                    let mut target = json!({ "host": native_host_id(), "uid": probe.identity.uid });
                    if let Some(t) = &tmux {
                        target["tmux"] = t.identity();
                    }
                    (
                        probe.info,
                        target,
                        Arc::new(LocalFileBackend) as Arc<dyn FileBackend>,
                        None,
                        tmux,
                        probe.path,
                    )
                };
                let id = config.id.clone().unwrap_or_else(|| {
                    format!(
                        "{kind}:{}",
                        &blake3::hash(target.to_string().as_bytes()).to_hex()[..24]
                    )
                });
                let mut info = info;
                info.id = id.clone();
                info.kind = kind.clone();
                info.target = target.clone();
                config.id = Some(id.clone());
                config.workdir = Some(info.cwd.clone());
                if let Some(t) = &mut config.tmux {
                    t.mode = Some(TmuxMode::Attach);
                }
                let descriptor = RuntimeDescriptor {
                    runtime_id: id,
                    kind,
                    host: target
                        .get("host")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    target,
                    workdir: info.cwd.clone(),
                    capabilities: Capabilities {
                        os: info.os.clone(),
                        network: true,
                        tools: info
                            .tools
                            .iter()
                            .map(|s| (s.clone(), "available".into()))
                            .collect(),
                    },
                };
                Ok(OpenedRuntime {
                    config,
                    descriptor,
                    info,
                    files,
                    ssh,
                    tmux,
                    base_path,
                })
            })
            .await
    }
}

#[async_trait]
impl AgentRuntime for Runtime {
    fn descriptor(&self) -> &RuntimeDescriptor {
        self.opened
            .get()
            .map(|s| &s.descriptor)
            .unwrap_or(&self.provisional)
    }
    fn config(&self) -> RuntimeConfig {
        self.opened
            .get()
            .map(|s| s.config.clone())
            .unwrap_or_else(|| self.config.clone())
    }
    async fn info(&self) -> Result<RuntimeInfo> {
        let state = self.initialize(&std::env::current_dir()?).await?;
        let probe = if let Some(ssh) = &state.ssh {
            ssh.probe(&state.info.cwd, &state.config.env).await?
        } else if let Some(tmux) = &state.tmux {
            tmux.probe(Path::new(&state.info.cwd), &state.config.env)
                .await?
        } else {
            ssh::local_probe(Path::new(&state.info.cwd), &state.config.env).await?
        };
        if let Some(ssh) = &state.ssh {
            ssh.verify_identity(&probe.identity)
                .map_err(|e| XllmError::RuntimeMismatch(e.to_string()))?;
        }
        let mut info = state.info.clone();
        info.current_time = probe.info.current_time;
        info.timezone = probe.info.timezone;
        Ok(info)
    }
    async fn open(
        &self,
        ctx: &RuntimeOpenCtx,
        tools: &ToolsConfig,
        deps: &XllmDeps,
    ) -> Result<(EffectiveTools, XllmToolManager)> {
        RuntimeConfig {
            env: ctx.env.clone(),
            ..Default::default()
        }
        .validate(false)?;
        let s = self.initialize(&ctx.workdir).await?;
        let mut env = s.config.env.clone();
        env.extend(ctx.env.clone());
        let mut path = ctx
            .path_prefix
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>();
        path.push(
            env.get("PATH")
                .cloned()
                .unwrap_or_else(|| s.base_path.clone()),
        );
        env.insert("PATH".into(), path.join(":"));
        let runner: Arc<dyn BashRunner> = if let Some(ssh) = &s.ssh {
            if !ctx.path_prefix.is_empty() {
                return Err(XllmError::Capability(
                    "remote Session helper deployment is not available".into(),
                ));
            }
            Arc::new(ssh::SshBashRunner::new(ssh.clone(), env, ctx.run.clone()))
        } else if let Some(tmux) = &s.tmux {
            Arc::new(tmux::TmuxBashRunner::new(tmux.clone(), env, ctx.run.clone()))
        } else {
            Arc::new(
                LocalProcessBashRunner::new()
                    .with_path_layers(ctx.path_prefix.clone())
                    .with_env(env.into_iter().collect())
                    .with_run_binding(ctx.run.clone()),
            )
        };
        let (effective, mut manager) = crate::xllm::build_runtime_toolset(
            tools,
            ctx.sources.clone(),
            ctx.loop_model,
            Path::new(&s.info.cwd),
            &ctx.run_id,
            deps,
            runner.clone(),
            s.files.clone(),
            &s.descriptor,
            ctx.run.clone(),
        )
        .await?;
        let mut info = self.info().await?;
        info.tools.extend(effective.all_names());
        info.tools.sort();
        info.tools.dedup();
        info.tools.truncate(32);
        manager.set_runtime(runner, s.descriptor.clone(), info);
        Ok((effective, manager))
    }
    async fn describe_interrupted(&self, run: &RunBinding, action: &InflightAction) -> String {
        if action.tool != TOOL_SHELL {
            return unresolved_reason(&action.tool);
        }
        let kind = self.descriptor().kind.clone();
        let opened = self.opened.get();
        if kind == "remote_ssh" {
            let target = self
                .config()
                .remote_ssh
                .as_ref()
                .and_then(|s| s.host.clone())
                .unwrap_or_default();
            let Some(ssh) = opened.and_then(|o| o.ssh.clone()) else {
                return interrupted_shell_text(&kind, &target, action, None, "(remote, not opened)");
            };
            let dir = ssh.remote_exec_dir(&run.run_id, &action.call_id);
            let facts = ssh.read_exec_dir(&dir).await;
            return interrupted_shell_text(&kind, &target, action, facts.as_ref(), &dir);
        }
        let target = self
            .config()
            .tmux
            .as_ref()
            .and_then(|t| t.session.clone())
            .unwrap_or_default();
        match exec_dir_for(run.run_dir.as_deref(), Some(&action.call_id)) {
            Some(dir) => {
                let facts = read_exec_dir(&dir);
                interrupted_shell_text(&kind, &target, action, Some(&facts), &dir.display().to_string())
            }
            None => interrupted_shell_text(&kind, &target, action, None, "(no execution directory)"),
        }
    }
}

pub struct RuntimeRegistry;
impl RuntimeRegistry {
    pub fn from_config(config: &RuntimeConfig) -> Result<Arc<dyn AgentRuntime>> {
        config.validate(true)?;
        Ok(Arc::new(Runtime::from_config(config.clone())))
    }
}

pub async fn open_runtime(
    config: &RuntimeConfig,
    ctx: &RuntimeOpenCtx,
    tools: &ToolsConfig,
    deps: &XllmDeps,
) -> Result<(Arc<dyn AgentRuntime>, EffectiveTools, XllmToolManager)> {
    config.validate(true)?;
    let runtime = match &deps.runtime {
        Some(r) => r.clone(),
        None => RuntimeRegistry::from_config(config)?,
    };
    if deps.runtime.is_some() {
        let provided = runtime.config();
        let mut expected = provided.clone();
        expected.merge_over(config);
        if expected != provided {
            return Err(XllmError::RuntimeMismatch(
                "injected runtime does not match effective configuration".into(),
            ));
        }
    }
    let (tools, manager) = runtime.open(ctx, tools, deps).await?;
    Ok((runtime, tools, manager))
}

pub struct SandboxPromptExec {
    pub sandbox: Arc<dyn Sandbox>,
    pub context: SessionRuntimeContext,
}
impl std::fmt::Debug for SandboxPromptExec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SandboxPromptExec")
            .field("workdir", &self.sandbox.workdir())
            .finish()
    }
}
#[async_trait]
impl PromptExec for SandboxPromptExec {
    async fn exec(&self, req: PromptExecRequest) -> std::result::Result<PromptExecOutput, String> {
        let command = req.command.clone();
        let out = self
            .sandbox
            .exec(
                BashRunRequest {
                    command: req.command,
                    cwd: self.sandbox.workdir().into(),
                    timeout_ms: req.timeout.as_millis().max(1) as u64,
                    max_output_bytes: req.max_output_bytes,
                    env: Vec::new(),
                    target: BashTarget::Local,
                    call_id: None,
                },
                &self.context,
            )
            .await
            .map_err(|e| e.to_string())?;
        log::info!(
            "template exec command={command:?} duration_ms={} exit_code={}",
            out.duration_ms,
            out.exit_code
        );
        Ok(PromptExecOutput {
            exit_code: out.exit_code,
            stdout: out.stdout,
            stderr: out.stderr,
            timed_out: out.timed_out,
        })
    }
}

pub struct RuntimeValueLoader(pub RuntimeInfo);
#[async_trait]
impl ValueLoader for RuntimeValueLoader {
    async fn load(&self, expr: &str) -> std::result::Result<Option<Value>, RenderError> {
        Ok(expr
            .trim_start_matches('$')
            .strip_prefix("runtime.")
            .and_then(|k| serde_json::to_value(&self.0).ok()?.get(k).cloned()))
    }
}

pub(crate) fn output(
    exit_code: i32,
    stdout: &[u8],
    stderr: &[u8],
    max: usize,
    timed_out: bool,
    elapsed: std::time::Duration,
    engine: &str,
    cwd: PathBuf,
) -> BashRunOutput {
    let mut combined = stdout.to_vec();
    if !stderr.is_empty() {
        if !combined.is_empty() && !combined.ends_with(b"\n") {
            combined.push(b'\n');
        }
        combined.extend_from_slice(stderr);
    }
    let text = |bytes: &[u8]| String::from_utf8_lossy(&bytes[..bytes.len().min(max)]).to_string();
    BashRunOutput {
        exit_code,
        stdout: text(stdout),
        stderr: text(stderr),
        output: text(&combined),
        output_truncated: combined.len() > max,
        timed_out,
        duration_ms: elapsed.as_millis() as u64,
        engine: engine.into(),
        cwd,
    }
}

tokio::task_local! {
    /// Call id of the tool call running in this task (names the `shell`
    /// execution directory, keys task ids).
    pub static CURRENT_TOOL_CALL: String;
    /// Interrupt / finish / deadline context of the tool call running in
    /// this task (`llm_context::ToolCallCtx`), set by the dispatcher.
    pub static CURRENT_TOOL_CTX: ToolCallCtx;
}

#[cfg(test)]
mod tests;
