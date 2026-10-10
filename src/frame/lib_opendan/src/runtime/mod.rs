pub mod bin_overlay;
pub use agent_tool::runtime::tmux;
pub use agent_tool::runtime::{native_host_id, AgentRuntime, NativeRuntime, TmuxRuntime};
pub use bin_overlay::{BinPlan, ToolPlan};

use crate::error::{OpenDanError, Result};
use crate::protocol::*;
use crate::session::SessionDir;
use agent_tool::runtime::RuntimeOpenCtx;
use agent_tool::xllm::{LoopModel, ToolsConfig};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionEnvCtx {
    pub session_id: String,
    pub session_dir: PathBuf,
    pub agent_did: String,
    pub agent_root: Option<PathBuf>,
    pub input_queue: Option<String>,
    pub trace_id: String,
    pub extra_env: Vec<(String, String)>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEnv {
    pub runtime_id: String,
    pub kind: String,
    pub workdir: PathBuf,
    pub path_layers: Vec<PathBuf>,
    pub env: Vec<(String, String)>,
}

fn verify_workspace_access(directory: &Path, access: WorkspaceAccess) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(directory.as_os_str().as_bytes())
            .map_err(|_| OpenDanError::WorkspaceBindingInvalid("invalid workspace path".into()))?;
        let mut mode = libc::R_OK | libc::X_OK;
        if access == WorkspaceAccess::ReadWrite {
            mode |= libc::W_OK;
        }
        if unsafe { libc::faccessat(libc::AT_FDCWD, path.as_ptr(), mode, libc::AT_EACCESS) } != 0 {
            return Err(OpenDanError::WorkspaceBindingInvalid(format!(
                "workspace directory permission denied: {}",
                std::io::Error::last_os_error()
            )));
        }
    }
    #[cfg(not(unix))]
    {
        let metadata = std::fs::metadata(directory)
            .map_err(|e| OpenDanError::WorkspaceBindingInvalid(e.to_string()))?;
        if access == WorkspaceAccess::ReadWrite && metadata.permissions().readonly() {
            return Err(OpenDanError::WorkspaceBindingInvalid(
                "workspace directory is read-only".into(),
            ));
        }
    }
    Ok(())
}

pub async fn resolve_workspace(
    agent: &dyn crate::state::AgentStateClient,
    workspace: Option<&WorkspaceRef>,
) -> Result<Option<WorkspaceBinding>> {
    let Some(workspace) = workspace else {
        return Ok(None);
    };
    let record = agent.workspaces().check(&workspace.workspace_id).await?;
    if record.lifecycle != WorkspaceLifecycle::Active
        || record.availability != WorkspaceAvailability::Available
        || record.location.runtime_id != LOCAL_WORKSPACE_RUNTIME
        || record.runtime_host != native_host_id()
    {
        return Err(OpenDanError::WorkspaceBindingInvalid(format!(
            "workspace {} is not available for a local session: {:?} {:?}",
            record.workspace_id, record.lifecycle, record.availability
        )));
    }
    verify_workspace_access(&record.location.directory, workspace.access)?;
    Ok(Some(WorkspaceBinding {
        workspace_id: record.workspace_id,
        revision: record.revision,
        location_revision: record.location_revision,
        runtime_host: record.runtime_host,
        location: record.location,
        access: workspace.access,
    }))
}

pub async fn verify_workspace(
    agent: &dyn crate::state::AgentStateClient,
    cfg: &SessionConfig,
) -> Result<()> {
    let invalid = |message: String| OpenDanError::WorkspaceBindingInvalid(message);
    match (&cfg.workspace, &cfg.workspace_binding) {
        (None, None) => Ok(()),
        (Some(reference), Some(binding)) => {
            if reference.workspace_id != binding.workspace_id || reference.access != binding.access
            {
                return Err(invalid(
                    "the workspace reference differs from its frozen binding".into(),
                ));
            }
            let record = agent
                .workspaces()
                .check(&binding.workspace_id)
                .await
                .map_err(|e| match e {
                    OpenDanError::Busy { .. } | OpenDanError::LeaseLost(_) => e,
                    _ => invalid(e.to_string()),
                })?;
            if record.location != binding.location
                || record.runtime_host != binding.runtime_host
                || binding.runtime_host != native_host_id()
                || record.location_revision != binding.location_revision
                || record.lifecycle != WorkspaceLifecycle::Active
                || record.availability != WorkspaceAvailability::Available
                || record.location.runtime_id != LOCAL_WORKSPACE_RUNTIME
            {
                return Err(invalid(format!(
                    "workspace {} no longer matches location revision {} ({:?}, {:?})",
                    binding.workspace_id,
                    binding.location_revision,
                    record.lifecycle,
                    record.availability
                )));
            }
            verify_workspace_access(&binding.location.directory, binding.access)?;
            let actual = std::fs::canonicalize(&binding.location.directory)
                .map_err(|e| invalid(e.to_string()))?;
            if actual != binding.location.directory {
                return Err(invalid(
                    "the workspace directory changed its canonical location".into(),
                ));
            }
            Ok(())
        }
        _ => Err(invalid(
            "the session is missing its frozen workspace binding".into(),
        )),
    }
}

pub(crate) async fn workspace_waiting_children(
    agent: &dyn crate::state::AgentStateClient,
    cfg: &SessionConfig,
    state: &SessionState,
) -> Result<Vec<RegistryEntry>> {
    let Some(workspace) = &cfg.workspace else {
        return Ok(Vec::new());
    };
    let Some(waiting) = &state.waiting_for else {
        return Ok(Vec::new());
    };
    if state.run_state != RunState::Waiting
        || !matches!(waiting.kind, WaitingKind::Tool | WaitingKind::Children)
        || !state.watched_tasks.is_empty()
        || waiting.refs.is_empty()
    {
        return Ok(Vec::new());
    }
    let mut children = Vec::new();
    for reference in &waiting.refs {
        let sid = reference.strip_prefix("session:").unwrap_or(reference);
        let Some(child) = agent.sessions().lookup(sid).await? else {
            return Ok(Vec::new());
        };
        if child
            .origin
            .as_ref()
            .and_then(|origin| origin.parent_session.as_deref())
            != Some(cfg.session.session_id.as_str())
            || child
                .workspace
                .as_ref()
                .is_none_or(|child| child.workspace_id != workspace.workspace_id)
        {
            return Ok(Vec::new());
        }
        children.push(child);
    }
    Ok(children)
}

pub fn resolve_workdir(
    sd: &SessionDir,
    cfg: &SessionConfig,
    _agent_root: Option<&Path>,
) -> Result<PathBuf> {
    let p = match (&cfg.workspace, &cfg.workspace_binding) {
        (None, None) => sd.path().to_path_buf(),
        (Some(reference), Some(binding))
            if reference.workspace_id == binding.workspace_id
                && reference.access == binding.access =>
        {
            if binding.location.runtime_id != LOCAL_WORKSPACE_RUNTIME
                || binding.runtime_host != native_host_id()
            {
                return Err(OpenDanError::WorkspaceBindingInvalid(
                    "remote workspace runtimes are not supported".into(),
                ));
            }
            binding.location.directory.clone()
        }
        _ => {
            return Err(OpenDanError::WorkspaceBindingInvalid(
                "the workspace binding is inconsistent".into(),
            ))
        }
    };
    p.canonicalize()
        .map_err(|e| OpenDanError::WorkspaceBindingInvalid(format!("{}: {e}", p.display())))
}

pub fn validate_workspace_workdir(cfg: &SessionConfig) -> Result<()> {
    if let (Some(binding), Some(workdir)) = (
        &cfg.workspace_binding,
        cfg.prompt
            .llm_context
            .get("runtime")
            .and_then(|r| r.get("workdir"))
            .and_then(|v| v.as_str()),
    ) {
        let path = Path::new(workdir);
        if !path.is_absolute()
            || path.canonicalize().ok().as_ref() != Some(&binding.location.directory)
        {
            return Err(OpenDanError::WorkspaceBindingInvalid(
                "explicit runtime workdir conflicts with the selected workspace".into(),
            ));
        }
    }
    Ok(())
}

/// The runtime configuration a session declares
/// (`prompt.llm_context.runtime`; native when absent).
fn declared_runtime(
    sd: &SessionDir,
    cfg: &SessionConfig,
) -> Result<Option<agent_tool::runtime::RuntimeConfig>> {
    let Some(value) = cfg.prompt.llm_context.get("runtime").cloned() else {
        return Ok(None);
    };
    let file = agent_tool::xllm::parse_llm_context_file(
        &sd.path().join("session_config.json"),
        &serde_json::json!({ "runtime": value }).to_string(),
    )
    .map_err(|e| OpenDanError::Bind(format!("Config: {e}")))?;
    Ok(file.runtime)
}

/// Session rules laid over a runtime configuration (xAgent §5.2.1): the
/// working directory is the session's workspace, and a tmux runtime belongs
/// to the session — its id is the session id, its tmux session name is
/// derived from it, it is created on the first binding and attached to
/// afterwards. A declared id / name that differs is a configuration error.
fn settle_for_session(
    sd: &SessionDir,
    cfg: &SessionConfig,
    agent_root: Option<&Path>,
    config: &mut agent_tool::runtime::RuntimeConfig,
) -> Result<()> {
    let expected = resolve_workdir(sd, cfg, agent_root)?;
    if let Some(workdir) = &config.workdir {
        let path = Path::new(workdir);
        if !path.is_absolute() || path.canonicalize().ok().as_ref() != Some(&expected) {
            return Err(OpenDanError::WorkspaceBindingInvalid(
                "explicit runtime workdir conflicts with the session directory binding".into(),
            ));
        }
    }
    config.workdir = Some(expected.display().to_string());
    if config.kind() != "tmux" {
        return Ok(());
    }
    let sid = sd.sid();
    let conflict = |what: &str, got: &str, want: &str| {
        OpenDanError::Bind(format!(
            "Config: the session's tmux runtime {what} is `{want}` (derived from the session id), not `{got}`"
        ))
    };
    match &config.id {
        Some(id) if id != sid => return Err(conflict("id", id, sid)),
        _ => config.id = Some(sid.to_string()),
    }
    if let Some(req) = cfg.runtime.requirement.runtime_id.as_deref().filter(|r| *r != sid) {
        return Err(conflict("id (runtime.requirement.runtime_id)", req, sid));
    }
    let name = tmux::tmux_session_name(sid);
    let t = config.tmux.get_or_insert_with(Default::default);
    match &t.session {
        Some(n) if n != &name => return Err(conflict("session name", n, &name)),
        _ => t.session = Some(name),
    }
    // Not bound yet: create it or reuse an existing one. Bound: only the
    // target the binding names may be attached to, a lost target is never
    // re-created under the old identity.
    t.mode = Some(if sd.binding_opt()?.is_some() {
        agent_tool::runtime::TmuxMode::Attach
    } else {
        agent_tool::runtime::TmuxMode::CreateOrAttach
    });
    Ok(())
}

/// The runtime of a session, built from its configuration: what a host
/// puts into that session's `RunnerDeps.runtime`. Nothing is opened or
/// created here — that happens at the binding, under the session lease.
pub fn runtime_for_session(
    sd: &SessionDir,
    cfg: &SessionConfig,
    agent_root: Option<&Path>,
    runtime_id: Option<&str>,
) -> Result<std::sync::Arc<dyn AgentRuntime>> {
    let mut config = declared_runtime(sd, cfg)?.unwrap_or_default();
    // An explicit id is a binding identity requirement, not a way to choose
    // the executor: a session's tmux runtime only accepts its own id.
    if let Some(id) = runtime_id {
        config.id = Some(id.to_string());
    }
    if config.id.is_none() && config.kind() != "tmux" {
        config.id = cfg.runtime.requirement.runtime_id.clone();
    }
    settle_for_session(sd, cfg, agent_root, &mut config)?;
    agent_tool::runtime::RuntimeRegistry::from_config(&config)
        .map_err(|e| OpenDanError::Bind(e.to_string()))
}

/// The runtime a drive uses: the provided one, checked against what the
/// session declares and completed by the session rules. A provided runtime
/// that names another executor than the session's is refused.
pub fn session_runtime(
    sd: &SessionDir,
    cfg: &SessionConfig,
    provided: &std::sync::Arc<dyn AgentRuntime>,
    agent_root: Option<&Path>,
) -> Result<std::sync::Arc<dyn AgentRuntime>> {
    let original = provided.config();
    let mut config = original.clone();
    if let Some(r) = declared_runtime(sd, cfg)? {
        config.merge_over(&r);
    }
    settle_for_session(sd, cfg, agent_root, &mut config)?;
    let differs = original.kind() != config.kind()
        || original.id.as_ref().is_some_and(|id| config.id.as_ref() != Some(id))
        || original
            .workdir
            .as_ref()
            .is_some_and(|cwd| config.workdir.as_ref() != Some(cwd))
        || original
            .remote_ssh
            .as_ref()
            .is_some_and(|ssh| config.remote_ssh.as_ref() != Some(ssh))
        || original.env.iter().any(|(k, v)| config.env.get(k) != Some(v))
        || original.tmux.as_ref().is_some_and(|tmux| {
            config.tmux.as_ref().is_none_or(|new| {
                tmux.session
                    .as_ref()
                    .is_some_and(|v| new.session.as_ref() != Some(v))
                    || tmux.socket.as_ref().is_some_and(|v| new.socket.as_ref() != Some(v))
            })
        });
    if differs {
        return Err(OpenDanError::RuntimeMismatch {
            bound: "prompt.llm_context.runtime of the session".into(),
            provided: format!("{} {}", original.kind(), original.id.clone().unwrap_or_default()),
        });
    }
    if config == original {
        return Ok(provided.clone());
    }
    agent_tool::runtime::RuntimeRegistry::from_config(&config)
        .map_err(|e| OpenDanError::Bind(e.to_string()))
}

/// A tmux session belongs to one Agent Session: the owner is recorded on
/// the target (`@opendan_session`). Two session ids whose tmux names
/// collide never share a target.
fn claim_tmux_target(config: &agent_tool::runtime::RuntimeConfig, sid: &str) -> Result<()> {
    let Some(t) = &config.tmux else {
        return Ok(());
    };
    let Some(name) = &t.session else {
        return Ok(());
    };
    let tmux = |args: &[&str]| -> std::io::Result<std::process::Output> {
        let mut c = std::process::Command::new("tmux");
        if let Some(socket) = &t.socket {
            c.arg("-L").arg(socket);
        }
        c.args(args).output()
    };
    let target = format!("{name}:");
    let shown = tmux(&["show-options", "-t", &target, "-qv", "@opendan_session"])
        .map_err(|e| OpenDanError::Bind(format!("tmux: {e}")))?;
    let owner = String::from_utf8_lossy(&shown.stdout).trim().to_string();
    if owner.is_empty() {
        tmux(&["set-option", "-t", &target, "@opendan_session", sid])
            .map_err(|e| OpenDanError::Bind(format!("tmux: {e}")))?;
        return Ok(());
    }
    if owner != sid {
        return Err(OpenDanError::Bind(format!(
            "tmux session `{name}` belongs to Agent Session `{owner}`; the name of `{sid}` collides with it"
        )));
    }
    Ok(())
}

pub async fn bind_or_verify(
    sd: &SessionDir,
    lease: &crate::lock::Lease,
    rt: &dyn AgentRuntime,
    cfg: &SessionConfig,
    agent_root: Option<&Path>,
    app_tools: &[String],
    plan: &BinPlan,
    runner_id: &str,
) -> Result<Binding> {
    if rt.config().kind() == "remote_ssh" {
        return Err(OpenDanError::Bind("Capability: remote Session helpers are not deployed; use SSH runtime with xllm independently".into()));
    }
    let fallback = resolve_workdir(sd, cfg, agent_root)?;
    let mut open = RuntimeOpenCtx::new(&fallback, sd.sid(), LoopModel::FunctionCall);
    let _ = rt
        .open(
            &open,
            &ToolsConfig {
                enabled: Some(false),
                ..Default::default()
            },
            &agent_tool::xllm::XllmDeps::default(),
        )
        .await
        .map_err(|e| OpenDanError::Bind(e.to_string()))?;
    let desc = rt.descriptor();
    if desc.kind == "tmux" {
        claim_tmux_target(&rt.config(), sd.sid())?;
    }
    let existing = sd.binding_opt()?;
    let binding = Binding {
        schema: "opendan.binding/4".into(),
        runtime_id: desc.runtime_id.clone(),
        kind: desc.kind.clone(),
        target: desc.target.clone(),
        workdir: desc.workdir.clone(),
        workspace: cfg.workspace_binding.clone(),
        bound_at_ms: crate::now_ms(),
        bound_by: runner_id.into(),
    };
    if Path::new(&binding.workdir).canonicalize().ok().as_ref() != Some(&fallback) {
        return Err(OpenDanError::WorkspaceBindingInvalid(
            "actual runtime workdir differs from the frozen binding".into(),
        ));
    }
    if let Some(b) = &existing {
        if !b.same_binding(&binding) {
            return Err(OpenDanError::RuntimeMismatch {
                bound: format!("{} {} {} {}", b.runtime_id, b.kind, b.target, b.workdir),
                provided: format!(
                    "{} {} {} {}",
                    binding.runtime_id, binding.kind, binding.target, binding.workdir
                ),
            });
        }
    }
    if let Some(id) = &cfg.runtime.requirement.runtime_id {
        if id != &binding.runtime_id {
            return Err(OpenDanError::RuntimeMismatch {
                bound: id.clone(),
                provided: binding.runtime_id.clone(),
            });
        }
    }
    let missing_app = cfg
        .runtime
        .requirement
        .app_tools
        .iter()
        .filter(|n| !app_tools.contains(n))
        .collect::<Vec<_>>();
    if !missing_app.is_empty() {
        return Err(OpenDanError::Bind(format!(
            "runner lacks app tools: {missing_app:?}"
        )));
    }
    bin_overlay::prepare(&sd.runtime_bin_dir(), plan)?;
    bin_overlay::verify(&sd.runtime_bin_dir(), plan)?;
    open.path_prefix.push(sd.runtime_bin_dir());
    if let Some(root) = agent_root {
        if root.join("tools").is_dir() {
            open.path_prefix.push(root.join("tools"));
        }
    }
    let (_, sandbox) = rt
        .open(
            &open,
            &ToolsConfig {
                enabled: Some(false),
                ..Default::default()
            },
            &agent_tool::xllm::XllmDeps::default(),
        )
        .await
        .map_err(|e| OpenDanError::Bind(e.to_string()))?;
    for name in &cfg.runtime.requirement.tools {
        let result = agent_tool::runtime::Sandbox::exec(
            &sandbox,
            agent_tool::llm_bash::BashRunRequest {
                command: format!("command -v {}", agent_tool::runtime::shell_quote(name)),
                cwd: binding.workdir.clone().into(),
                timeout_ms: 5000,
                max_output_bytes: 4096,
                env: Vec::new(),
                target: agent_tool::llm_bash::BashTarget::Local,
                call_id: None,
            },
            &agent_tool::SessionRuntimeContext {
                trace_id: sd.sid().into(),
                agent_name: "libopendan".into(),
                behavior: "runtime_probe".into(),
                tool_call_index: 0,
                wakeup_id: String::new(),
                session_id: sd.sid().into(),
                read_token_limit: agent_tool::DEFAULT_READ_TOKEN_LIMIT,
            },
        )
        .await
        .map_err(|e| OpenDanError::Bind(e.to_string()))?;
        if result.exit_code != 0 {
            return Err(OpenDanError::Bind(format!(
                "runtime lacks required tool {name}"
            )));
        }
    }
    if existing.is_none() {
        let published = lease
            .fenced(|| crate::fsutil::publish_noreplace_json(&sd.file(BINDING_FILE), &binding))?;
        if !published {
            let current = sd
                .binding_opt()?
                .ok_or_else(|| OpenDanError::Bind("binding vanished".into()))?;
            if !current.same_binding(&binding) {
                return Err(OpenDanError::RuntimeMismatch {
                    bound: current.runtime_id,
                    provided: binding.runtime_id,
                });
            }
        }
    }
    Ok(existing.unwrap_or(binding))
}

pub async fn open_session_env(binding: &Binding, ctx: &SessionEnvCtx) -> Result<SessionEnv> {
    let mut layers = vec![ctx.session_dir.join(RUNTIME_DIR).join("bin")];
    if let Some(root) = &ctx.agent_root {
        if root.join("tools").is_dir() {
            layers.push(root.join("tools"));
        }
    }
    Ok(SessionEnv {
        runtime_id: binding.runtime_id.clone(),
        kind: binding.kind.clone(),
        workdir: binding.workdir.clone().into(),
        path_layers: layers,
        env: session_env_vars(ctx, &binding.runtime_id).await?,
    })
}

pub fn bin_plan_for(
    cfg: &SessionConfig,
    agent_root: Option<&Path>,
    session_cli: Option<PathBuf>,
    runtime_layers: &[PathBuf],
) -> Result<BinPlan> {
    let mut plan = BinPlan {
        session_cli,
        ..Default::default()
    };
    if let Some(root) = agent_root {
        plan.universe_dirs.push(root.join("tools"));
    }
    plan.universe_dirs.extend(runtime_layers.iter().cloned());
    if let (Some(name), Some(root)) = (&cfg.runtime.tool_plan, agent_root) {
        if let Some(tp) = ToolPlan::load(root, name)? {
            plan.tool_plan_name = Some(name.clone());
            plan.tool_plan = Some(tp);
        }
    }
    Ok(plan)
}

/// The §7.3 environment contract.
pub async fn session_env_vars(ctx: &SessionEnvCtx, runtime_id: &str) -> Result<Vec<(String, String)>> {
    let mut v = vec![
        ("OPENDAN_AGENT_DID".to_string(), ctx.agent_did.clone()),
        (
            "OPENDAN_SESSION_DIR".to_string(),
            ctx.session_dir.display().to_string(),
        ),
        ("OPENDAN_SESSION_ID".to_string(), ctx.session_id.clone()),
        ("OPENDAN_TRACE_ID".to_string(), ctx.trace_id.clone()),
        ("OPENDAN_RUNTIME_ID".to_string(), runtime_id.to_string()),
    ];
    if let Some(r) = &ctx.agent_root {
        v.push(("OPENDAN_AGENT_ROOT".to_string(), r.display().to_string()));
    }
    if let Some(q) = &ctx.input_queue {
        v.push(("OPENDAN_INPUT_QUEUE".to_string(), q.clone()));
    }
    if let Ok(tok) = std::env::var("BUCKYOS_APPCLIENT_SESSION_TOKEN") {
        v.push(("BUCKYOS_APPCLIENT_SESSION_TOKEN".to_string(), tok));
    }
    v.extend(ctx.extra_env.iter().cloned());
    agent_tool::runtime_context::refresh_appclient_session_env(&mut v)
        .await
        .map_err(|e| OpenDanError::Bind(e.to_string()))?;
    Ok(v)
}
